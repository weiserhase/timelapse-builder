#!/usr/bin/env bash
set -euo pipefail

os="${1:?usage: fetch-ffmpeg.sh <linux|windows|macos> [dest]}"
dest="${2:-ffmpeg-dl}"
mkdir -p "$dest"
dest="$(cd "$dest" && pwd)"

log() { echo ">> $*" >&2; }

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
    tar -xf "$dest/ffmpeg.zip" -C "$dest"
    src="$(find "$dest" -type f -name ffmpeg.exe | head -n1)"
    out="$dest/ffmpeg.exe"
    ;;
  macos)
    url="https://evermeet.cx/ffmpeg/getrelease/zip"
    log "downloading $url"
    curl -fL "$url" -o "$dest/ffmpeg.zip"
    tar -xf "$dest/ffmpeg.zip" -C "$dest"
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
