use anyhow::Result;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::AtomicBool;
use timelapse_builder::cli::Args;
use timelapse_builder::{run, BuildOptions, Progress};

fn main() {
    if let Err(e) = real_main() {
        eprintln!("\x1b[31merror:\x1b[0m {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let args = Args::parse();
    let opts = BuildOptions {
        inputs: args.inputs,
        output: args.output,
        fps: args.fps,
        width: args.width,
        height: args.height,
        recursive: args.recursive,
        sort: args.sort,
        reverse: args.reverse,
        every: args.every,
        limit: args.limit,
        crf: args.crf,
        preset: args.preset,
        codec: args.codec,
        fit: args.fit,
        threads: args.threads,
        source: args.source,
    };

    let mut pb: Option<ProgressBar> = None;
    let cancel = AtomicBool::new(false);

    run(&opts, &cancel, |progress| match progress {
        Progress::Started {
            total,
            width,
            height,
            ffmpeg,
        } => {
            eprintln!(
                "timelapse-builder: {total} frames \u{2192} {width}x{height} @ {} fps  (ffmpeg: {})",
                opts.fps,
                ffmpeg.display(),
            );
            let bar = ProgressBar::new(total as u64);
            bar.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
                )
                .unwrap()
                .progress_chars("=>-"),
            );
            pb = Some(bar);
        }
        Progress::Advanced { done, .. } => {
            if let Some(bar) = &pb {
                bar.set_position(done as u64);
            }
        }
        Progress::Skipped { path, error } => {
            let line = format!("\x1b[33mskip\x1b[0m {}: {error}", path.display());
            match &pb {
                Some(bar) => bar.suspend(|| eprintln!("{line}")),
                None => eprintln!("{line}"),
            }
        }
        Progress::Finished {
            encoded,
            skipped,
            elapsed,
            output,
        } => {
            if let Some(bar) = pb.take() {
                bar.finish_and_clear();
            }
            let extra = if skipped > 0 {
                format!(", {skipped} skipped")
            } else {
                String::new()
            };
            eprintln!(
                "\x1b[32mdone\x1b[0m {} ({encoded} frames in {elapsed:.1}s{extra})",
                output.display(),
            );
        }
        Progress::Cancelled { encoded } => {
            if let Some(bar) = pb.take() {
                bar.finish_and_clear();
            }
            eprintln!("\x1b[33mcancelled\x1b[0m after {encoded} frames");
        }
    })
}
