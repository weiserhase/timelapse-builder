use clap::{Parser, ValueEnum};
use std::path::PathBuf;

/// Blazingly fast timelapse builder. Combines PNG/JPG/WebP/RAW stills into an
/// MP4 (or other) video, decoding frames in parallel across all cores.
#[derive(Parser, Debug)]
#[command(name = "timelapse", version, about, long_about = None)]
pub struct Args {
    /// Input image files and/or directories.
    #[arg(required = true, value_name = "PATH")]
    pub inputs: Vec<PathBuf>,

    /// Output video file.
    #[arg(short, long, default_value = "timelapse.mp4", value_name = "FILE")]
    pub output: PathBuf,

    /// Playback frame rate. Higher = faster timelapse.
    #[arg(long, default_value_t = 30.0, value_name = "FPS")]
    pub fps: f32,

    /// Target width in pixels (height auto-derived to keep aspect if omitted).
    #[arg(long, value_name = "PX")]
    pub width: Option<u32>,

    /// Target height in pixels (width auto-derived to keep aspect if omitted).
    #[arg(long, value_name = "PX")]
    pub height: Option<u32>,

    /// Recurse into subdirectories when an input is a directory.
    #[arg(short, long)]
    pub recursive: bool,

    /// Keep only files whose name matches this regular expression.
    #[arg(long, value_name = "REGEX")]
    pub filter: Option<String>,

    /// Frame ordering.
    #[arg(long, value_enum, default_value_t = Sort::Name)]
    pub sort: Sort,

    /// Order by a key pulled from each file name with this regex (first capture
    /// group, else the whole match), natural-sorted. Overrides --sort.
    #[arg(long, value_name = "REGEX")]
    pub sort_key: Option<String>,

    /// Reverse the final frame order.
    #[arg(long)]
    pub reverse: bool,

    /// Use only every Nth frame (e.g. 4 = 4x faster / fewer frames).
    #[arg(long, default_value_t = 1, value_name = "N")]
    pub every: usize,

    /// Cap the number of frames used (after sampling).
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Quality: H.264/H.265 CRF (lower = better, 18 is visually lossless).
    #[arg(long, default_value_t = 18, value_name = "CRF")]
    pub crf: u32,

    /// x264/x265 preset (ultrafast..veryslow) trading speed for compression.
    #[arg(long, default_value = "medium")]
    pub preset: String,

    /// Video codec passed to ffmpeg. CPU: libx264, libx265, libvpx-vp9.
    /// GPU (NVIDIA): h264_nvenc, hevc_nvenc, av1_nvenc.
    #[arg(long, default_value = "libx264")]
    pub codec: String,

    /// How each frame is mapped onto the output dimensions.
    #[arg(long, value_enum, default_value_t = Fit::Cover)]
    pub fit: Fit,

    /// Where RAW pixels come from. `preview` extracts the camera's embedded
    /// JPEG (much faster, camera color); `raw` always demosaics; `auto` uses
    /// the preview when it is large enough for the output, else demosaics.
    #[arg(long, value_enum, default_value_t = Source::Auto)]
    pub source: Source,

    /// Number of decode threads (default: all available cores).
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Sort {
    /// Natural sort by file name (img2 before img10).
    Name,
    /// Sort by file modification time.
    Time,
    /// Keep the order given on the command line.
    None,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Source {
    /// Use the embedded preview when it covers the output size, else demosaic.
    Auto,
    /// Always demosaic the RAW sensor data (best quality, slowest).
    Raw,
    /// Always use the camera's embedded JPEG preview (fastest).
    Preview,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Fit {
    /// Scale to fill the frame, center-cropping overflow (no bars).
    Cover,
    /// Scale to fit inside the frame, letterboxing with black.
    Contain,
    /// Stretch to exactly the target size (may distort).
    Stretch,
}

pub fn resolve_dimensions(
    width: Option<u32>,
    height: Option<u32>,
    src_w: u32,
    src_h: u32,
) -> (u32, u32) {
    let (w, h) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => {
            let h = (w as f64 * src_h as f64 / src_w as f64).round() as u32;
            (w, h)
        }
        (None, Some(h)) => {
            let w = (h as f64 * src_w as f64 / src_h as f64).round() as u32;
            (w, h)
        }
        (None, None) => (src_w, src_h),
    };
    (even(w.max(2)), even(h.max(2)))
}

fn even(v: u32) -> u32 {
    v & !1
}
