use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use timelapse_builder::{run, BuildOptions, Progress};

fn ffmpeg() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TIMELAPSE_FFMPEG") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let bundled = PathBuf::from("dist/timelapse-builder-linux-x86_64/ffmpeg");
    bundled.is_file().then_some(bundled)
}

#[test]
#[ignore = "slow; needs ffmpeg + testdata"]
fn stop_cancels_promptly() {
    let Some(ff) = ffmpeg() else {
        eprintln!("skipping: no ffmpeg available");
        return;
    };
    let testdata = PathBuf::from("testdata");
    if !testdata.is_dir() {
        eprintln!("skipping: no testdata directory");
        return;
    }
    std::env::set_var("TIMELAPSE_FFMPEG", &ff);

    let opts = BuildOptions {
        inputs: vec![testdata],
        output: std::env::temp_dir().join("tl-cancel-test.mp4"),
        width: Some(640),
        limit: Some(96),
        ..Default::default()
    };
    let total_limit = opts.limit.unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let worker_cancel = cancel.clone();
    let worker = thread::spawn(move || run(&opts, &worker_cancel, move |p| { let _ = tx.send(p); }));

    let mut saw_progress = false;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Progress::Advanced { .. }) => {
                saw_progress = true;
                break;
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    assert!(saw_progress, "build never produced a frame");

    cancel.store(true, Ordering::Relaxed);
    let asked = Instant::now();

    let mut cancelled_at = None;
    while let Ok(p) = rx.recv_timeout(Duration::from_secs(60)) {
        if let Progress::Cancelled { encoded } = p {
            cancelled_at = Some(encoded);
            break;
        }
    }

    let result = worker.join().expect("worker panicked");
    assert!(result.is_ok(), "run returned error: {:?}", result.err());
    let encoded = cancelled_at.expect("no Cancelled progress emitted");
    assert!(
        encoded < total_limit,
        "cancel did not stop work early: encoded {encoded} of {total_limit}"
    );
    assert!(
        asked.elapsed() < Duration::from_secs(30),
        "cancel took too long: {:?}",
        asked.elapsed()
    );
}
