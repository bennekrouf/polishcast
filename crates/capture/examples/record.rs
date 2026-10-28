//! Records the screen for a few seconds, without the app: `cargo run -p polishcast-capture
//! --example record -- <dir> [seconds]`. Prints every event, for debugging capture.
//!
//! `SV_FPS`, `SV_CODEC` (h264 or hevc) and `SV_MAX_WIDTH` override the default options,
//! `SV_SYSTEM_AUDIO` records the sound apps play and `SV_NO_MIC` leaves the microphone out;
//! `SV_WINDOW` records the window with that id instead of the display (`list` prints them).
// A test or an example may panic: that is a failed run, not a crash anyone sees.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use polishcast_capture::{Codec, Event, Options, Recorder, Source};
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

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGMainDisplayID() -> u32;
}

fn main() {
    // Connects to the window server as the app does; window filters assert otherwise.
    #[cfg(target_os = "macos")]
    // SAFETY: takes no arguments and only reads which display is the main one.
    unsafe {
        CGMainDisplayID();
    }
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
    options.system_audio = env("SV_SYSTEM_AUDIO").is_some();
    options.microphone = env("SV_NO_MIC").is_none();
    options.max_width = env("SV_MAX_WIDTH").and_then(|v| v.parse().ok());
    if env("SV_WINDOW").as_deref() == Some("list") {
        for w in polishcast_capture::windows() {
            println!("{}\t{}\t{}", w.id, w.app, w.title);
        }
        return;
    }
    if let Some(id) = env("SV_WINDOW").and_then(|v| v.parse().ok()) {
        options.source = Source::Window(id);
    }
    recorder.start(dir, options);
    next(&recorder); // Started
    std::thread::sleep(Duration::from_secs(secs));
    recorder.stop();
    next(&recorder); // Finished
}
