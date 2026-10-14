//! Records the screen for a few seconds, without the app: `cargo run -p polishcast-capture
//! --example record -- <dir> [seconds]`. Prints every event, for debugging capture.
//!
//! `SV_FPS`, `SV_CODEC` (h264 or hevc) and `SV_MAX_WIDTH` override the default options.

use polishcast_capture::{Codec, Event, Options, Recorder};
use std::time::Duration;

fn next(recorder: &Recorder) -> Event {
    loop {
        if let Some(e) = recorder.try_event() {
            println!("{e:?}");
            if matches!(e, Event::Failed(_)) {
                std::process::exit(1);
            }
            return e;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("usage: record <dir> [seconds]"));
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(3);
    std::fs::create_dir_all(&dir).expect("create the folder");

    let recorder = Recorder::spawn();
    let env = |name: &str| std::env::var(name).ok();
    let mut options = Options::default();
    if let Some(fps) = env("SV_FPS").and_then(|v| v.parse().ok()) {
        options.fps = fps;
    }
    if env("SV_CODEC").as_deref() == Some("hevc") {
        options.codec = Codec::Hevc;
    }
    options.max_width = env("SV_MAX_WIDTH").and_then(|v| v.parse().ok());
    recorder.start(dir, options);
    next(&recorder); // Started
    std::thread::sleep(Duration::from_secs(secs));
    recorder.stop();
    next(&recorder); // Finished
}
