use crate::cli::Sort;
use anyhow::{bail, Result};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

pub const STD_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];
pub const RAW_EXTS: &[&str] = &[
    "raw", "dng", "cr2", "cr3", "nef", "arw", "raf", "rw2", "orf", "srw", "pef", "mrw", "dcr",
    "kdc", "3fr", "mef", "mos", "nrw", "x3f", "iiq", "rwl", "erf",
];

pub fn is_supported(path: &Path) -> bool {
    match ext_lower(path) {
        Some(e) => STD_EXTS.contains(&e.as_str()) || RAW_EXTS.contains(&e.as_str()),
        None => false,
    }
}

pub fn is_raw(path: &Path) -> bool {
    matches!(ext_lower(path), Some(e) if RAW_EXTS.contains(&e.as_str()))
}

fn ext_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

pub fn gather(inputs: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for input in inputs {
        if !input.exists() {
            bail!("input does not exist: {}", input.display());
        }
        if input.is_dir() {
            let max_depth = if recursive { usize::MAX } else { 1 };
            for entry in WalkDir::new(input)
                .max_depth(max_depth)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let p = entry.path();
                if p.is_file() && is_supported(p) {
                    out.push(p.to_path_buf());
                }
            }
        } else if is_supported(input) {
            out.push(input.clone());
        } else {
            bail!("unsupported file type: {}", input.display());
        }
    }
    Ok(out)
}

pub fn order(files: &mut [PathBuf], sort: Sort, reverse: bool) {
    match sort {
        Sort::Name => files.sort_by(|a, b| natural_cmp(&key(a), &key(b))),
        Sort::Time => files.sort_by_key(|p| mtime(p)),
        Sort::None => {}
    }
    if reverse {
        files.reverse();
    }
}

/// Keep only files whose name matches `re`.
pub fn filter(files: &mut Vec<PathBuf>, re: &Regex) {
    files.retain(|p| re.is_match(file_name(p)));
}

/// Order by a key extracted from each file name: the first capture group if the
/// regex has one, otherwise the whole match, otherwise empty. Keys are
/// natural-sorted so embedded numbers order numerically.
pub fn order_by_key(files: &mut [PathBuf], re: &Regex, reverse: bool) {
    files.sort_by(|a, b| natural_cmp(&extract_key(a, re), &extract_key(b, re)));
    if reverse {
        files.reverse();
    }
}

fn extract_key(p: &Path, re: &Regex) -> String {
    match re.captures(file_name(p)) {
        Some(c) => c
            .get(1)
            .or_else(|| c.get(0))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn file_name(p: &Path) -> &str {
    p.file_name().and_then(|s| s.to_str()).unwrap_or("")
}

fn key(p: &Path) -> String {
    p.to_string_lossy().to_ascii_lowercase()
}

fn mtime(p: &Path) -> SystemTime {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let na = take_number(&mut ai);
                    let nb = take_number(&mut bi);
                    match na.cmp(&nb) {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                } else {
                    match ca.cmp(&cb) {
                        Ordering::Equal => {
                            ai.next();
                            bi.next();
                        }
                        other => return other,
                    }
                }
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars>) -> u128 {
    let mut n: u128 = 0;
    while let Some(c) = it.peek().copied() {
        if let Some(d) = c.to_digit(10) {
            n = n.saturating_mul(10).saturating_add(d as u128);
            it.next();
        } else {
            break;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn natural_orders_numerically() {
        let mut v: Vec<PathBuf> = ["img10.png", "img2.png", "img1.png", "img21.png"]
            .iter().map(PathBuf::from).collect();
        order(&mut v, Sort::Name, false);
        let got: Vec<_> = v.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(got, ["img1.png", "img2.png", "img10.png", "img21.png"]);
    }

    #[test]
    fn filter_keeps_matching_names() {
        let mut v: Vec<PathBuf> = ["a_keep_1.png", "b_skip_2.png", "c_keep_3.png"]
            .iter().map(PathBuf::from).collect();
        filter(&mut v, &Regex::new("keep").unwrap());
        let got: Vec<_> = v.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(got, ["a_keep_1.png", "c_keep_3.png"]);
    }

    #[test]
    fn order_by_capture_group() {
        // Sort by the number after "seq", ignoring the leading shuffled prefix.
        let mut v: Vec<PathBuf> = ["z_seq10.png", "a_seq2.png", "m_seq1.png"]
            .iter().map(PathBuf::from).collect();
        order_by_key(&mut v, &Regex::new(r"seq(\d+)").unwrap(), false);
        let got: Vec<_> = v.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(got, ["m_seq1.png", "a_seq2.png", "z_seq10.png"]);
    }
}
