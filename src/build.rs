use crate::cli::{resolve_dimensions, Fit, Sort, Source};
use crate::decode::RawMode;
use crate::{collect, decode, encode, ffmpeg};
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::thread;
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
    pub source: Source,
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
            source: Source::Auto,
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

    // Resolve where RAW pixels come from, sizing the output against whatever we
    // will actually decode so `preview` never upscales the embedded JPEG.
    let first = &files[0];
    let (sw, sh, raw_mode) = match opts.source {
        Source::Raw => {
            let (w, h) = decode::source_dims(first)
                .with_context(|| format!("failed to read first frame: {}", first.display()))?;
            (w, h, RawMode::Develop)
        }
        Source::Preview => {
            let (w, h) = match decode::preview_dims(first) {
                Some(d) => d,
                None => decode::source_dims(first).with_context(|| {
                    format!("failed to read first frame: {}", first.display())
                })?,
            };
            (w, h, RawMode::Preview)
        }
        Source::Auto => {
            let (w, h) = decode::source_dims(first)
                .with_context(|| format!("failed to read first frame: {}", first.display()))?;
            let (tw, th) = resolve_dimensions(opts.width, opts.height, w, h);
            // Use the preview only when it can cover the output without upscaling.
            let mode = match decode::preview_dims(first) {
                Some((pw, ph)) if pw >= tw && ph >= th => RawMode::Preview,
                _ => RawMode::Develop,
            };
            (w, h, mode)
        }
    };
    let (tw, th) = resolve_dimensions(opts.width, opts.height, sw, sh);

    on(Progress::Started {
        total: files.len(),
        width: tw,
        height: th,
        ffmpeg: ffmpeg_bin.clone(),
    });

    let enc = encode::Encoder::start(&ffmpeg_bin, opts, tw, th)
        .context("failed to start ffmpeg encoder")?;

    let concurrency = opts
        .threads
        .unwrap_or_else(rayon::current_num_threads)
        .max(1);
    let frame_bytes = (tw as usize) * (th as usize) * 3;
    // Keep memory bounded: one chunk being decoded plus a chunk's worth queued
    // for the encoder is roughly two batches in flight (~512 MB target).
    let mem_cap = (256 * 1024 * 1024 / frame_bytes.max(1)).max(2);
    let batch = concurrency.clamp(2, mem_cap);
    let queue_cap = batch.max(2);
    let fit = opts.fit;
    let total = files.len();

    let pool = opts
        .threads
        .map(|n| rayon::ThreadPoolBuilder::new().num_threads(n.max(1)).build())
        .transpose()?;

    // Decode (CPU, parallel) and encode (ffmpeg/GPU) run concurrently: a writer
    // thread owns the encoder and drains a bounded channel while this thread
    // decodes the next batch and feeds frames into it in order. The bound
    // applies backpressure so decoding never races far ahead of encoding.
    let (done, skipped) = thread::scope(|s| -> Result<(usize, usize)> {
        let (tx, rx) = sync_channel::<Vec<u8>>(queue_cap);

        let writer = s.spawn(move || -> Result<()> {
            let mut enc = enc;
            while let Ok(buf) = rx.recv() {
                enc.write_frame(&buf).context(
                    "ffmpeg closed the pipe early (encoder failed); check the codec/output settings",
                )?;
            }
            if cancel.load(Ordering::Relaxed) {
                enc.kill();
                Ok(())
            } else {
                enc.finish().context("ffmpeg encoding failed")
            }
        });

        let mut done = 0usize;
        let mut skipped = 0usize;
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
                            Some(decode::load_frame(p, tw, th, fit, raw_mode))
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
                    // A send error means the writer thread exited early (ffmpeg
                    // failed); stop producing and let join() surface the cause.
                    Ok(buf) => {
                        if tx.send(buf).is_err() {
                            break 'outer;
                        }
                    }
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
        drop(tx);

        match writer.join() {
            Ok(res) => res?,
            Err(_) => bail!("encoder writer thread panicked"),
        }
        Ok((done, skipped))
    })?;

    if cancel.load(Ordering::Relaxed) {
        on(Progress::Cancelled {
            encoded: done - skipped,
        });
        return Ok(());
    }

    on(Progress::Finished {
        encoded: total - skipped,
        skipped,
        elapsed: started.elapsed().as_secs_f64(),
        output: opts.output.clone(),
    });

    Ok(())
}
