use crate::cli::{resolve_dimensions, Fit, Sort};
use crate::{collect, decode, encode, ffmpeg};
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub fps: f32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub recursive: bool,
    pub sort: Sort,
    pub reverse: bool,
    pub every: usize,
    pub limit: Option<usize>,
    pub crf: u32,
    pub preset: String,
    pub codec: String,
    pub fit: Fit,
    pub threads: Option<usize>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output: PathBuf::from("timelapse.mp4"),
            fps: 30.0,
            width: None,
            height: None,
            recursive: false,
            sort: Sort::Name,
            reverse: false,
            every: 1,
            limit: None,
            crf: 18,
            preset: "medium".into(),
            codec: "libx264".into(),
            fit: Fit::Cover,
            threads: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Progress {
    Started {
        total: usize,
        width: u32,
        height: u32,
        ffmpeg: PathBuf,
    },
    Advanced {
        done: usize,
        total: usize,
    },
    Skipped {
        path: PathBuf,
        error: String,
    },
    Finished {
        encoded: usize,
        skipped: usize,
        elapsed: f64,
        output: PathBuf,
    },
    Cancelled {
        encoded: usize,
    },
}

pub fn run(opts: &BuildOptions, cancel: &AtomicBool, mut on: impl FnMut(Progress)) -> Result<()> {
    let started = Instant::now();

    let ffmpeg_bin = ffmpeg::locate().context(
        "could not find an ffmpeg binary. Place `ffmpeg` next to this executable, \
         put it on your PATH, or install it (Debian/Ubuntu: `sudo apt install ffmpeg`)",
    )?;

    let mut files =
        collect::gather(&opts.inputs, opts.recursive).context("failed to collect input files")?;
    collect::order(&mut files, opts.sort, opts.reverse);

    if opts.every > 1 {
        files = files.into_iter().step_by(opts.every).collect();
    }
    if let Some(limit) = opts.limit {
        files.truncate(limit);
    }
    if files.is_empty() {
        bail!("no supported image files found (png, jpg, jpeg, webp, or raw)");
    }

    let probe = decode::load_rgb(&files[0])
        .with_context(|| format!("failed to decode first frame: {}", files[0].display()))?;
    let (tw, th) = resolve_dimensions(opts.width, opts.height, probe.width(), probe.height());

    on(Progress::Started {
        total: files.len(),
        width: tw,
        height: th,
        ffmpeg: ffmpeg_bin.clone(),
    });

    let mut enc = encode::Encoder::start(&ffmpeg_bin, opts, tw, th)
        .context("failed to start ffmpeg encoder")?;

    let concurrency = opts
        .threads
        .unwrap_or_else(rayon::current_num_threads)
        .max(1);
    let frame_bytes = (tw as usize) * (th as usize) * 3;
    let mem_cap = (512 * 1024 * 1024 / frame_bytes.max(1)).max(4);
    let batch = concurrency.clamp(4, mem_cap);
    let fit = opts.fit;
    let total = files.len();
    let mut done = 0usize;
    let mut skipped = 0usize;

    let pool = opts
        .threads
        .map(|n| rayon::ThreadPoolBuilder::new().num_threads(n.max(1)).build())
        .transpose()?;

    'outer: for chunk in files.chunks(batch) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let decode_chunk = || -> Vec<Option<Result<Vec<u8>>>> {
            chunk
                .par_iter()
                .map(|p| {
                    if cancel.load(Ordering::Relaxed) {
                        None
                    } else {
                        Some(decode::load_frame(p, tw, th, fit))
                    }
                })
                .collect()
        };
        let frames = match &pool {
            Some(p) => p.install(decode_chunk),
            None => decode_chunk(),
        };

        for (path, frame) in chunk.iter().zip(frames) {
            let frame = match frame {
                Some(f) => f,
                None => break 'outer,
            };
            match frame {
                Ok(buf) => enc.write_frame(&buf).context(
                    "ffmpeg closed the pipe early (encoder failed); check the codec/output settings",
                )?,
                Err(e) => {
                    skipped += 1;
                    on(Progress::Skipped {
                        path: path.clone(),
                        error: format!("{e:#}"),
                    });
                }
            }
            done += 1;
            on(Progress::Advanced { done, total });
        }
    }

    if cancel.load(Ordering::Relaxed) {
        enc.kill();
        on(Progress::Cancelled {
            encoded: done - skipped,
        });
        return Ok(());
    }

    enc.finish().context("ffmpeg encoding failed")?;

    on(Progress::Finished {
        encoded: total - skipped,
        skipped,
        elapsed: started.elapsed().as_secs_f64(),
        output: opts.output.clone(),
    });

    Ok(())
}
