use crate::build::BuildOptions;
use anyhow::{bail, Result};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

pub struct Encoder {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
}

/// Translate an x264-style preset name to NVENC's p1..p7 scale. A value the
/// user already gave as a p-preset (or anything else) is passed through.
fn nvenc_preset(preset: &str) -> &str {
    match preset {
        "ultrafast" | "superfast" | "veryfast" => "p1",
        "faster" | "fast" => "p2",
        "medium" => "p4",
        "slow" => "p5",
        "slower" => "p6",
        "veryslow" => "p7",
        other => other,
    }
}

impl Encoder {
    pub fn start(ffmpeg: &Path, args: &BuildOptions, w: u32, h: u32) -> Result<Self> {
        let is_nvenc = args.codec.contains("nvenc");

        // NVIDIA's H.264 encoder maxes out at 4096x4096; full-res RAW frames
        // (e.g. 6000x4000) silently fail to open the encoder. Fail early and
        // loud, pointing at the codecs that can handle larger frames.
        if is_nvenc && args.codec.contains("264") && (w > 4096 || h > 4096) {
            bail!(
                "h264_nvenc cannot encode {w}x{h}: NVIDIA's H.264 encoder is limited to \
                 4096x4096. Use --codec hevc_nvenc or av1_nvenc (up to 8192), or downscale \
                 with --width/--height (e.g. --width 3840 for 4K)."
            );
        }

        let mut cmd = Command::new(ffmpeg);
        cmd.arg("-hide_banner").args(["-loglevel", "error", "-stats"]);

        // NVENC's automatic device selection fails under WSL2 ("No capable
        // devices found"); pinning the CUDA device makes it deterministic.
        if is_nvenc {
            cmd.args(["-init_hw_device", "cuda=cu:0"]);
        }

        cmd.arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pixel_format", "rgb24"])
            .args(["-video_size", &format!("{w}x{h}")])
            .args(["-framerate", &format!("{}", args.fps)])
            .args(["-i", "-"])
            .args(["-c:v", &args.codec])
            .args(["-pix_fmt", "yuv420p"])
            .args(["-r", &format!("{}", args.fps)]);

        if is_nvenc {
            // NVENC silently ignores -crf; constant-quality VBR is its CRF
            // equivalent. -preset takes p1 (fastest) .. p7 (best).
            cmd.args(["-rc", "vbr"])
                .args(["-cq", &args.crf.to_string()])
                .args(["-b:v", "0"])
                .args(["-preset", nvenc_preset(&args.preset)]);
        } else {
            cmd.args(["-crf", &args.crf.to_string()]);
            if args.codec.contains("264")
                || args.codec.contains("265")
                || args.codec.contains("hevc")
            {
                cmd.args(["-preset", &args.preset]);
            }
        }
        cmd.args(["-movflags", "+faststart"]).arg(&args.output);

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());

        // A GUI process has no console of its own, so Windows would otherwise pop
        // up a console window for the ffmpeg child. CREATE_NO_WINDOW suppresses it.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("could not launch ffmpeg ({}): {e}", ffmpeg.display()))?;
        let stdin = child
            .stdin
            .take()
            .map(BufWriter::new)
            .expect("stdin was requested as piped");

        Ok(Self {
            child,
            stdin: Some(stdin),
        })
    }

    #[inline]
    pub fn write_frame(&mut self, rgb: &[u8]) -> Result<()> {
        let w = self.stdin.as_mut().expect("encoder still open");
        w.write_all(rgb)?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        if let Some(mut w) = self.stdin.take() {
            w.flush()?;
        }
        let status = self.child.wait()?;
        if !status.success() {
            bail!("ffmpeg exited with {status}");
        }
        Ok(())
    }

    pub fn kill(mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
