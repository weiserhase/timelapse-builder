use crate::collect::is_raw;
use anyhow::{anyhow, Context, Result};
use image::{imageops, imageops::FilterType, RgbImage};
use std::path::Path;

/// Concrete decode strategy for a RAW file, after `cli::Source::Auto` has been
/// resolved against the output size. Non-RAW inputs ignore this.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RawMode {
    /// Demosaic the sensor data through rawler's full develop pipeline.
    Develop,
    /// Decode the camera's embedded JPEG preview (no demosaic).
    Preview,
}

pub fn load_rgb(path: &Path, mode: RawMode) -> Result<RgbImage> {
    if is_raw(path) {
        match mode {
            RawMode::Develop => develop_raw(path),
            RawMode::Preview => preview_raw(path),
        }
    } else {
        let img = image::open(path).with_context(|| format!("decode {}", path.display()))?;
        Ok(img.to_rgb8())
    }
}

fn develop_raw(path: &Path) -> Result<RgbImage> {
    use rawler::imgop::develop::RawDevelop;

    let raw = rawler::decode_file(path)
        .map_err(|e| anyhow!("raw decode {}: {e:?}", path.display()))?;
    let developed = RawDevelop::default()
        .develop_intermediate(&raw)
        .map_err(|e| anyhow!("raw develop {}: {e:?}", path.display()))?;
    let img = developed
        .to_dynamic_image()
        .ok_or_else(|| anyhow!("raw {} produced no image", path.display()))?;
    Ok(img.to_rgb8())
}

fn preview_raw(path: &Path) -> Result<RgbImage> {
    use rawler::decoders::RawDecodeParams;

    let img = rawler::analyze::extract_preview_pixels(path, &RawDecodeParams::default())
        .map_err(|e| anyhow!("raw preview {}: {e:?}", path.display()))?;
    Ok(img.to_rgb8())
}

/// Pixel dimensions of the embedded preview, or `None` if there isn't one.
pub fn preview_dims(path: &Path) -> Option<(u32, u32)> {
    use rawler::decoders::RawDecodeParams;
    rawler::analyze::extract_preview_pixels(path, &RawDecodeParams::default())
        .ok()
        .map(|img| (img.width(), img.height()))
}

/// Full (demosaiced) dimensions of a frame. For RAW this reads the sensor size
/// without running the expensive develop; for other formats it reads the header.
pub fn source_dims(path: &Path) -> Result<(u32, u32)> {
    if is_raw(path) {
        let raw = rawler::decode_file(path)
            .map_err(|e| anyhow!("raw decode {}: {e:?}", path.display()))?;
        Ok((raw.width as u32, raw.height as u32))
    } else {
        image::image_dimensions(path).with_context(|| format!("read dimensions {}", path.display()))
    }
}

pub fn load_frame(path: &Path, tw: u32, th: u32, fit: crate::cli::Fit, mode: RawMode) -> Result<Vec<u8>> {
    let img = load_rgb(path, mode)?;
    let framed = fit_to(img, tw, th, fit);
    Ok(framed.into_raw())
}

fn fit_to(img: RgbImage, tw: u32, th: u32, fit: crate::cli::Fit) -> RgbImage {
    use crate::cli::Fit;
    let (w, h) = (img.width(), img.height());
    if w == tw && h == th {
        return img;
    }
    match fit {
        Fit::Stretch => imageops::resize(&img, tw, th, FilterType::Triangle),
        Fit::Cover => {
            let scale = (tw as f64 / w as f64).max(th as f64 / h as f64);
            let nw = ((w as f64 * scale).ceil() as u32).max(tw);
            let nh = ((h as f64 * scale).ceil() as u32).max(th);
            let resized = imageops::resize(&img, nw, nh, FilterType::Triangle);
            let x = (nw - tw) / 2;
            let y = (nh - th) / 2;
            imageops::crop_imm(&resized, x, y, tw, th).to_image()
        }
        Fit::Contain => {
            let scale = (tw as f64 / w as f64).min(th as f64 / h as f64);
            let nw = ((w as f64 * scale).round() as u32).clamp(1, tw);
            let nh = ((h as f64 * scale).round() as u32).clamp(1, th);
            let resized = imageops::resize(&img, nw, nh, FilterType::Triangle);
            let mut canvas = RgbImage::new(tw, th);
            let x = ((tw - nw) / 2) as i64;
            let y = ((th - nh) / 2) as i64;
            imageops::overlay(&mut canvas, &resized, x, y);
            canvas
        }
    }
}
