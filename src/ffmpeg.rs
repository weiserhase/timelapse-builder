use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

const BIN: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };

pub fn locate() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("TIMELAPSE_FFMPEG") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
    }

    #[cfg(feature = "embed-ffmpeg")]
    if let Some(p) = embedded::extract()? {
        return Ok(p);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for cand in [dir.join(BIN), dir.join("bin").join(BIN)] {
                if cand.is_file() {
                    return Ok(cand);
                }
            }
        }
    }

    if let Some(p) = which(BIN) {
        return Ok(p);
    }

    Err(anyhow!("ffmpeg not found"))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if is_executable(&cand) {
            return Some(cand);
        }
    }
    None
}

fn is_executable(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(feature = "embed-ffmpeg")]
mod embedded {
    use anyhow::{Context, Result};
    use std::path::PathBuf;

    static FFMPEG_BYTES: &[u8] = include_bytes!(env!("TIMELAPSE_FFMPEG_EMBED"));

    pub fn extract() -> Result<Option<PathBuf>> {
        let mut dir = dirs_cache();
        dir.push("timelapse-builder");
        std::fs::create_dir_all(&dir).context("create cache dir for embedded ffmpeg")?;

        let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
        let out = dir.join(format!("ffmpeg-{}", FFMPEG_BYTES.len()));
        let out = out.with_file_name(format!(
            "{}-{}",
            name.trim_end_matches(".exe"),
            FFMPEG_BYTES.len()
        ));

        let needs_write = std::fs::metadata(&out)
            .map(|m| m.len() != FFMPEG_BYTES.len() as u64)
            .unwrap_or(true);
        if needs_write {
            std::fs::write(&out, FFMPEG_BYTES).context("write embedded ffmpeg")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(Some(out))
    }

    fn dirs_cache() -> PathBuf {
        if let Some(x) = std::env::var_os("XDG_CACHE_HOME") {
            return PathBuf::from(x);
        }
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h).join(".cache");
        }
        std::env::temp_dir()
    }
}
