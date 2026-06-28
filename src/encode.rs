use crate::cli::Args;
use anyhow::{bail, Result};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

pub struct Encoder {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
}

impl Encoder {
    pub fn start(ffmpeg: &Path, args: &Args, w: u32, h: u32) -> Result<Self> {
        let mut cmd = Command::new(ffmpeg);
        cmd.arg("-hide_banner")
            .args(["-loglevel", "error", "-stats"])
            .arg("-y")
            .args(["-f", "rawvideo"])
            .args(["-pixel_format", "rgb24"])
            .args(["-video_size", &format!("{w}x{h}")])
            .args(["-framerate", &format!("{}", args.fps)])
            .args(["-i", "-"])
            .args(["-c:v", &args.codec])
            .args(["-pix_fmt", "yuv420p"])
            .args(["-r", &format!("{}", args.fps)]);

        cmd.args(["-crf", &args.crf.to_string()]);
        if args.codec.contains("264") || args.codec.contains("265") || args.codec.contains("hevc") {
            cmd.args(["-preset", &args.preset]);
        }
        cmd.args(["-movflags", "+faststart"]).arg(&args.output);

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());

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
}
