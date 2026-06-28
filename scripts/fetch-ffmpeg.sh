#!/usr/bin/env bash
set -euo pipefail

os="${1:?usage: fetch-ffmpeg.sh <linux|windows|macos> [dest]}"
dest="${2:-ffmpeg-dl}"
mkdir -p "$dest"
dest="$(cd "$dest" && pwd)"

log() { echo ">> $*" >&2; }

# Extract a .zip into a directory. The `tar` on a runner's bash can be MSYS2
# GNU tar (no zip support), so try several extractors and use whichever exists.
extract_zip() {
  local zip="$1" out="$2"
  if command -v unzip >/dev/null 2>&1; then
    unzip -qo "$zip" -d "$out"
  elif command -v 7z >/dev/null 2>&1; then
    7z x -y -o"$out" "$zip" >/dev/null
  elif [[ -x /c/Windows/System32/tar.exe ]]; then
    /c/Windows/System32/tar.exe -xf "$zip" -C "$out"   # Windows bsdtar (libarchive)
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import sys,zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$zip" "$out"
  else
    tar -xf "$zip" -C "$out"   # last resort (bsdtar on macOS handles zip)
  fi
}

btbn="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest"

case "$os" in
  linux)
    url="$btbn/ffmpeg-master-latest-linux64-gpl.tar.xz"
    log "downloading $url"
    curl -fL "$url" -o "$dest/ffmpeg.tar.xz"
    tar -xJf "$dest/ffmpeg.tar.xz" -C "$dest"
    src="$(find "$dest" -type f -name ffmpeg | head -n1)"
    out="$dest/ffmpeg"
    ;;
  windows)
    url="$btbn/ffmpeg-master-latest-win64-gpl.zip"
    log "downloading $url"
    curl -fL "$url" -o "$dest/ffmpeg.zip"
    extract_zip "$dest/ffmpeg.zip" "$dest"
    src="$(find "$dest" -type f -name ffmpeg.exe | head -n1)"
    out="$dest/ffmpeg.exe"
    ;;
  macos)
    url="https://evermeet.cx/ffmpeg/getrelease/zip"
    log "downloading $url"
    curl -fL "$url" -o "$dest/ffmpeg.zip"
    extract_zip "$dest/ffmpeg.zip" "$dest"
    src="$(find "$dest" -type f -name ffmpeg | head -n1)"
    out="$dest/ffmpeg"
    ;;
  *)
    echo "unknown platform: $os (expected linux|windows|macos)" >&2
    exit 1
    ;;
esac

if [[ -z "${src:-}" || ! -f "$src" ]]; then
  echo "!! ffmpeg binary not found in archive" >&2
  exit 1
fi
cp "$src" "$out"
chmod +x "$out" 2>/dev/null || true
log "ffmpeg ready: $out"
echo "$out"
