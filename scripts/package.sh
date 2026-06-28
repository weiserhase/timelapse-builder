#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

EMBED=0
[[ "${1:-}" == "--embed" ]] && EMBED=1

NAME="timelapse-builder"
DIST="dist/$NAME"
rm -rf "$DIST" && mkdir -p "$DIST"

find_ffmpeg() {
  if [[ -n "${FFMPEG:-}" && -x "${FFMPEG}" ]]; then echo "$FFMPEG"; return; fi
  if command -v ffmpeg >/dev/null 2>&1; then command -v ffmpeg; return; fi
  echo ""
}

FFMPEG_BIN="$(find_ffmpeg)"
if [[ -z "$FFMPEG_BIN" ]]; then
  if command -v curl >/dev/null 2>&1 && [[ "$(uname -s)-$(uname -m)" == "Linux-x86_64" ]]; then
    echo ">> no ffmpeg found; downloading a static Linux build..."
    tmp="$(mktemp -d)"
    url="https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz"
    curl -fL "$url" -o "$tmp/ff.tar.xz"
    tar -xJf "$tmp/ff.tar.xz" -C "$tmp"
    FFMPEG_BIN="$(find "$tmp" -name ffmpeg -type f | head -n1)"
  fi
fi
if [[ -z "$FFMPEG_BIN" || ! -x "$FFMPEG_BIN" ]]; then
  echo "!! could not obtain an ffmpeg binary."
  echo "   Set FFMPEG=/path/to/ffmpeg and re-run, or install ffmpeg first."
  exit 1
fi
echo ">> bundling ffmpeg: $FFMPEG_BIN"

if [[ "$EMBED" == "1" ]]; then
  echo ">> building single self-contained executable (embed-ffmpeg)..."
  TIMELAPSE_FFMPEG_EMBED="$FFMPEG_BIN" cargo build --release --features embed-ffmpeg
  cp target/release/timelapse "$DIST/"
else
  echo ">> building release binary..."
  cargo build --release
  cp target/release/timelapse "$DIST/"
  cp "$FFMPEG_BIN" "$DIST/ffmpeg"
  chmod +x "$DIST/ffmpeg"
fi

cp README.md "$DIST/" 2>/dev/null || true
chmod +x "$DIST/timelapse"

tarball="dist/${NAME}-$(uname -s)-$(uname -m).tar.gz"
tar -czf "$tarball" -C dist "$NAME"
echo ">> bundle ready: $DIST"
echo ">> archive:      $tarball"
ls -lh "$DIST"
