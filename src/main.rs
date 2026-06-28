mod cli;
mod collect;
mod decode;
mod encode;
mod ffmpeg;

use anyhow::{bail, Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::io::Write;
use std::time::Instant;

use cli::Args;

fn main() {
    if let Err(e) = run() {
        eprintln!("\x1b[31merror:\x1b[0m {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let started = Instant::now();

    if let Some(n) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n.max(1))
            .build_global()
            .ok();
    }

    let ffmpeg_bin = ffmpeg::locate().context(
        "could not find an ffmpeg binary. Place `ffmpeg` next to this executable, \
         put it on your PATH, or install it (Debian/Ubuntu: `sudo apt install ffmpeg`)",
    )?;

    let mut files = collect::gather(&args.inputs, args.recursive)
        .context("failed to collect input files")?;
    collect::order(&mut files, args.sort, args.reverse);

    if args.every > 1 {
        files = files.into_iter().step_by(args.every).collect();
    }
    if let Some(limit) = args.limit {
        files.truncate(limit);
    }

    if files.is_empty() {
        bail!("no supported image files found (png, jpg, jpeg, webp, or raw)");
    }

    let probe = decode::load_rgb(&files[0])
        .with_context(|| format!("failed to decode first frame: {}", files[0].display()))?;
    let (tw, th) = cli::target_dimensions(&args, probe.width(), probe.height());

    eprintln!(
        "timelapse-builder: {} frames \u{2192} {}x{} @ {} fps  ({} threads, ffmpeg: {})",
        files.len(),
        tw,
        th,
        args.fps,
        rayon::current_num_threads(),
        ffmpeg_bin.display(),
    );

    let mut enc = encode::Encoder::start(&ffmpeg_bin, &args, tw, th)
        .context("failed to start ffmpeg encoder")?;

    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap()
        .progress_chars("=>-"),
    );

    let frame_bytes = (tw as usize) * (th as usize) * 3;
    let batch = (512 * 1024 * 1024 / frame_bytes.max(1)).clamp(4, 256);
    let fit = args.fit;
    let mut skipped = 0usize;

    for chunk in files.chunks(batch) {
        let frames: Vec<Result<Vec<u8>>> = chunk
            .par_iter()
            .map(|path| decode::load_frame(path, tw, th, fit))
            .collect();

        for (path, frame) in chunk.iter().zip(frames) {
            match frame {
                Ok(buf) => {
                    debug_assert_eq!(buf.len(), frame_bytes);
                    enc.write_frame(&buf).with_context(|| {
                        "ffmpeg closed the pipe early (encoder failed); run with a visible \
                         terminal to see its diagnostics"
                    })?;
                }
                Err(e) => {
                    skipped += 1;
                    pb.suspend(|| {
                        eprintln!("\x1b[33mskip\x1b[0m {}: {e:#}", path.display());
                    });
                }
            }
            pb.inc(1);
        }
        let _ = std::io::stderr().flush();
    }

    pb.finish_and_clear();

    enc.finish().context("ffmpeg encoding failed")?;

    let encoded = files.len() - skipped;
    eprintln!(
        "\x1b[32mdone\x1b[0m {} ({} frames in {:.1}s{})",
        args.output.display(),
        encoded,
        started.elapsed().as_secs_f64(),
        if skipped > 0 {
            format!(", {skipped} skipped")
        } else {
            String::new()
        },
    );

    Ok(())
}
