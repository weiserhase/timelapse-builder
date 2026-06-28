//! GPU-accelerated RAW demosaic via `wgpu` (D3D12 on Windows, Vulkan on Linux,
//! Metal on macOS). The expensive per-pixel demosaic plus the white-balance,
//! camera-to-sRGB matrix and gamma run on the GPU; the cheap black/white
//! scaling and matrix derivation are done on the CPU with rawler's own public
//! helpers, so the colour output matches the CPU developer. Only the demosaic
//! *algorithm* differs (bilinear here vs rawler's PPG).
//!
//! Every entry point returns `Result`; callers fall back to the CPU developer
//! on any error (no GPU, unsupported sensor, device loss, …). A software
//! adapter (e.g. lavapipe) is rejected unless `TIMELAPSE_ALLOW_SOFTWARE_GPU`
//! is set, so we never masquerade a slow CPU rasteriser as acceleration.

use anyhow::{anyhow, bail, Result};
use image::RgbImage;
use std::borrow::Cow;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use wgpu::util::DeviceExt;

use rawler::imgop::matrix::{multiply, normalize, pseudo_inverse};
use rawler::imgop::xyz::{Illuminant, SRGB_TO_XYZ_D65};
use rawler::rawimage::{RawImage, RawImageData, RawPhotometricInterpretation};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    dims0: [u32; 4], // full_w, full_h, active_x, active_y
    dims1: [u32; 4], // out_w, out_h, _, _
    cfa: [u32; 4],   // colour index (R=0,G=1,B=2) per 2x2 phase [00,01,10,11]
    wb: [f32; 4],    // white-balance multipliers r,g,b,_
    m0: [f32; 4],    // cam2rgb rows (.xyz used)
    m1: [f32; 4],
    m2: [f32; 4],
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// Serialises GPU work so concurrent decode threads don't exhaust VRAM.
    lock: Mutex<()>,
    info: String,
}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

const SHADER: &str = include_str!("demosaic.wgsl");

fn gpu() -> Option<&'static Gpu> {
    GPU.get_or_init(init).as_ref()
}

/// A human-readable description of the demosaic GPU, or `None` if unavailable.
pub fn describe() -> Option<String> {
    gpu().map(|g| g.info.clone())
}

fn init() -> Option<Gpu> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))?;
    let ai = adapter.get_info();
    let allow_sw = std::env::var_os("TIMELAPSE_ALLOW_SOFTWARE_GPU").is_some();
    if ai.device_type == wgpu::DeviceType::Cpu && !allow_sw {
        return None;
    }

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("timelapse-demosaic"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .ok()?;

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("demosaic"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
    });

    let storage = |read_only: bool| wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("demosaic-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: storage(true),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: storage(false),
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("demosaic-pl"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("demosaic"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: "main",
        compilation_options: Default::default(),
        cache: None,
    });

    Some(Gpu {
        device,
        queue,
        pipeline,
        layout,
        lock: Mutex::new(()),
        info: format!("{} [{:?} / {:?}]", ai.name, ai.device_type, ai.backend),
    })
}

/// Develop a RAW file to sRGB on the GPU. Returns `Err` (so the caller can fall
/// back to the CPU developer) for any unsupported sensor or GPU failure.
pub fn develop(path: &Path) -> Result<RgbImage> {
    let g = gpu().ok_or_else(|| anyhow!("no compatible GPU"))?;
    let mut raw =
        rawler::decode_file(path).map_err(|e| anyhow!("raw decode {}: {e:?}", path.display()))?;

    // Only plain RGB Bayer (3 colours, one sample per pixel) is handled here.
    let cfa = match &raw.photometric {
        RawPhotometricInterpretation::Cfa(cfg) => cfg.cfa.clone(),
        _ => bail!("non-CFA raw"),
    };
    if !cfa.is_rgb() || cfa.unique_colors() != 3 || raw.cpp != 1 {
        bail!("unsupported CFA ({})", cfa.name);
    }

    // Black/white normalisation to [0,1] — exactly rawler's Rescale step.
    raw.apply_scaling()
        .map_err(|e| anyhow!("apply_scaling: {e:?}"))?;
    let mosaic = match &raw.data {
        RawImageData::Float(v) => v,
        RawImageData::Integer(_) => bail!("raw data not normalised"),
    };
    let (full_w, full_h) = (raw.width, raw.height);
    if mosaic.len() != full_w * full_h {
        bail!("mosaic size mismatch");
    }

    // Region of interest: the active sensor area, else the whole frame.
    let (ax, ay, ow, oh) = match raw.active_area {
        Some(r) => (r.p.x, r.p.y, r.d.w, r.d.h),
        None => (0, 0, full_w, full_h),
    };
    if ow == 0 || oh == 0 {
        bail!("empty active area");
    }

    // 2x2 CFA colour phase (R=0,G=1,B=2). color_at returns the plane index,
    // which for an RGB CFA is the colour itself.
    let cfa = [
        cfa.color_at(0, 0) as u32,
        cfa.color_at(0, 1) as u32,
        cfa.color_at(1, 0) as u32,
        cfa.color_at(1, 1) as u32,
    ];

    let wb = if raw.wb_coeffs[0].is_nan() {
        [1.0, 1.0, 1.0, 1.0]
    } else {
        raw.wb_coeffs
    };

    // cam2rgb, derived identically to rawler's `map_3ch_to_rgb`.
    let xyz2cam = pick_xyz2cam(&raw)?;
    let rgb2cam = normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65));
    let cam2rgb = pseudo_inverse(rgb2cam); // [[f32; 4]; 3]

    let params = Params {
        dims0: [full_w as u32, full_h as u32, ax as u32, ay as u32],
        dims1: [ow as u32, oh as u32, 0, 0],
        cfa,
        wb: [wb[0], wb[1], wb[2], 1.0],
        m0: [cam2rgb[0][0], cam2rgb[0][1], cam2rgb[0][2], 0.0],
        m1: [cam2rgb[1][0], cam2rgb[1][1], cam2rgb[1][2], 0.0],
        m2: [cam2rgb[2][0], cam2rgb[2][1], cam2rgb[2][2], 0.0],
    };

    let packed = run_compute(g, &params, mosaic, ow, oh)?;
    let mut rgb = Vec::with_capacity(ow * oh * 3);
    for px in packed {
        rgb.push((px & 0xff) as u8);
        rgb.push(((px >> 8) & 0xff) as u8);
        rgb.push(((px >> 16) & 0xff) as u8);
    }
    RgbImage::from_raw(ow as u32, oh as u32, rgb).ok_or_else(|| anyhow!("image assembly failed"))
}

fn pick_xyz2cam(raw: &RawImage) -> Result<[[f32; 3]; 4]> {
    // Prefer the D65 matrix (matching rawler's Calibrate), else any available.
    let (_, cm) = raw
        .color_matrix
        .iter()
        .find(|(ill, _)| **ill == Illuminant::D65)
        .or_else(|| raw.color_matrix.iter().next())
        .ok_or_else(|| anyhow!("no colour matrix"))?;
    if cm.is_empty() || cm.len() % 3 != 0 {
        bail!("invalid colour matrix length {}", cm.len());
    }
    let mut xyz2cam = [[0.0f32; 3]; 4];
    for i in 0..(cm.len() / 3).min(4) {
        for j in 0..3 {
            xyz2cam[i][j] = cm[i * 3 + j];
        }
    }
    Ok(xyz2cam)
}

fn run_compute(g: &Gpu, params: &Params, mosaic: &[f32], ow: usize, oh: usize) -> Result<Vec<u32>> {
    let _guard = g.lock.lock().unwrap();
    let (device, queue) = (&g.device, &g.queue);
    let out_bytes = (ow * oh * 4) as u64;

    let param_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("params"),
        contents: bytemuck::bytes_of(params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let in_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mosaic"),
        contents: bytemuck::cast_slice(mosaic),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rgb-out"),
        size: out_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: out_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("demosaic-bg"),
        layout: &g.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: param_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: in_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: out_buf.as_entire_binding(),
            },
        ],
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("demosaic") });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("demosaic"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&g.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        let gx = (ow as u32).div_ceil(8);
        let gy = (oh as u32).div_ceil(8);
        pass.dispatch_workgroups(gx, gy, 1);
    }
    encoder.copy_buffer_to_buffer(&out_buf, 0, &staging, 0, out_bytes);
    queue.submit([encoder.finish()]);

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|_| anyhow!("gpu readback dropped"))?
        .map_err(|e| anyhow!("gpu readback failed: {e:?}"))?;

    let data = slice.get_mapped_range();
    let result = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
    drop(data);
    staging.unmap();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the compute pipeline directly with a synthetic RGGB mosaic of a
    /// flat mid-grey scene and confirm the output is a plausible uniform grey.
    /// Validates the wgpu plumbing + shader on whatever adapter is available
    /// (set TIMELAPSE_ALLOW_SOFTWARE_GPU=1 to permit lavapipe in CI/WSL).
    #[test]
    fn synthetic_grey_mosaic() {
        let Some(g) = gpu() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let (w, h) = (64usize, 64usize);
        let mosaic = vec![0.5f32; w * h]; // every photosite at 0.5 after scaling
        let params = Params {
            dims0: [w as u32, h as u32, 0, 0],
            dims1: [w as u32, h as u32, 0, 0],
            cfa: [0, 1, 1, 2], // RGGB
            wb: [1.0, 1.0, 1.0, 1.0],
            // identity cam2rgb so output == demosaiced+gamma grey
            m0: [1.0, 0.0, 0.0, 0.0],
            m1: [0.0, 1.0, 0.0, 0.0],
            m2: [0.0, 0.0, 1.0, 0.0],
        };
        let out = run_compute(g, &params, &mosaic, w, h).expect("compute");
        assert_eq!(out.len(), w * h);
        // sRGB(0.5) ≈ 0.735 → ~188. Interior pixels should be near-grey.
        let mid = out[(h / 2) * w + w / 2];
        let (r, gg, b) = (mid & 0xff, (mid >> 8) & 0xff, (mid >> 16) & 0xff);
        eprintln!("center pixel rgb = {r},{gg},{b}  adapter = {}", g.info);
        for c in [r, gg, b] {
            assert!((170..=205).contains(&c), "channel {c} not grey-ish");
        }
        assert!(r.abs_diff(gg) <= 6 && gg.abs_diff(b) <= 6, "not neutral");
    }
}
