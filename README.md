# timelapse-builder

Fast Rust CLI that turns a pile of stills into a timelapse video. Decodes PNG,
JPG/JPEG, WebP and camera RAW (CR2/CR3, NEF, ARW, DNG, RAF, RW2, ORF, …) in
parallel and streams frames straight into ffmpeg for H.264/H.265/VP9 output.

## Usage

```sh
timelapse ./photos
timelapse ./photos -r --sort time --width 3840 --fps 24 -o sunset.mp4
timelapse ./photos --every 5 --limit 600
timelapse a.CR2 b.jpg ./more_frames -o mix.mp4
timelapse ./photos --codec libx265 --crf 24 --preset slow
# keep only matching files, then order by a number pulled from the name
timelapse ./photos --filter 'DSC0\d+\.ARW' --sort-key 'DSC0(\d+)'
```

Run `timelapse --help` for all flags. Common ones: `-o/--output`, `--fps`,
`--width`/`--height`, `--fit` (cover/contain/stretch), `--sort` (name/time/none),
`--filter REGEX` (keep only files whose name matches), `--sort-key REGEX` (order
by the first capture group, natural-sorted; overrides `--sort`), `-r`,
`--every N`, `--limit N`, `--reverse`, `--codec`, `--crf`, `--preset`,
`--threads`.

## ffmpeg

The encoder is an ffmpeg binary, resolved in order: `TIMELAPSE_FFMPEG`, embedded
(`--features embed-ffmpeg`), next to the executable, then PATH.

## Build & package

```sh
cargo build --release        # dev build
cargo test
scripts/package.sh           # self-contained dist/ folder + tarball
scripts/package.sh --embed   # single executable with ffmpeg embedded
```
