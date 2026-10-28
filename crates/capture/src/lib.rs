//! Screen recording.
//!
//! A dedicated thread owns the capture session, like TrackCut's playback engine: the UI sends
//! it commands and polls for events, and never blocks on ScreenCaptureKit (asking for content
//! can wait on a permission prompt; stopping waits for the file to be finalized).
//!
//! A take is a folder holding the screen recording (cursor hidden, microphone included) and a
//! log of the pointer's movements and clicks, so the cursor can be redrawn later.
//!
//! Backends: ScreenCaptureKit on macOS (`mac`), Windows.Graphics.Capture on Windows (`win`).
//! Elsewhere starting a recording fails with a message.

use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac::Active;
#[cfg_attr(not(windows), allow(dead_code))]
mod pcm;
#[cfg(any(target_os = "macos", windows))]
mod pointer;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win::Active;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    /// About half the size of H.264 for screen content, hardware encoded on Apple silicon.
    Hevc,
}

/// What a take records.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Source {
    /// The display the pointer is on.
    #[default]
    Display,
    /// One window, by its id from [`windows`], even when other windows cover it.
    Window(u32),
    /// A rectangle of the screen, in global points (origin at the main display's top-left).
    Area(Area),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A window that can be recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub id: u32,
    /// The app it belongs to.
    pub app: String,
    pub title: String,
    /// On the current Space; otherwise on another one, full screen, or minimized.
    pub on_screen: bool,
}

/// Where the pointer is, in global points (origin at the main display's top-left).
pub fn pointer_location() -> Option<(f64, f64)> {
    #[cfg(any(target_os = "macos", windows))]
    return pointer::location();
    #[cfg(not(any(target_os = "macos", windows)))]
    None
}

/// Whether this platform can record the sound apps play, not only the microphone.
pub const RECORDS_SYSTEM_AUDIO: bool = cfg!(target_os = "macos");

/// Whether this platform can record a window or an area, not only a whole display.
pub const RECORDS_WINDOWS_AND_AREAS: bool = cfg!(target_os = "macos");

/// The windows on screen that can be recorded, by app. Blocks: the first call may wait on the
/// Screen Recording permission prompt. Empty where windows can't be recorded.
pub fn windows() -> Vec<WindowInfo> {
    #[cfg(target_os = "macos")]
    return mac::windows();
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

#[derive(Clone, Debug)]
pub struct Options {
    pub source: Source,
    pub fps: u32,
    pub codec: Codec,
    /// Record at most this many pixels wide (the display's full resolution if smaller).
    pub max_width: Option<u32>,
    pub microphone: bool,
    /// Sound played by apps on the Mac.
    pub system_audio: bool,
    /// Started with a click (a button, the menu bar, an area dragged) rather than the shortcut:
    /// the pointer's way off it is left out of the take's project.
    pub clicked: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            source: Source::Display,
            fps: 60,
            codec: Codec::H264,
            max_width: None,
            microphone: true,
            system_audio: false,
            clicked: false,
        }
    }
}

/// A finished recording.
#[derive(Clone, Debug)]
pub struct Take {
    pub dir: PathBuf,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    /// The file was written at a fixed, high bitrate and is worth re-encoding smaller.
    pub compact: bool,
    /// The sound apps played was recorded apart (`take::SOUND`), starting this many seconds
    /// into the screen recording (before it, if negative), and is still to be mixed in.
    pub sound: Option<f64>,
    /// The stretch worth keeping, in seconds: without the pointer's way off the Record button
    /// and to the Stop click.
    pub kept: (f64, f64),
}

#[derive(Debug)]
pub enum Event {
    /// Frames are being written.
    Started,
    Finished(Take),
    Failed(String),
}

enum Cmd {
    Start { dir: PathBuf, options: Options },
    Stop { clicked: bool },
}

#[derive(Clone)]
pub struct Recorder {
    tx: Sender<Cmd>,
    events: Receiver<Event>,
}

impl Recorder {
    pub fn spawn() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (etx, events) = crossbeam_channel::unbounded();
        // The operating system refusing a thread leaves nothing to fall back to.
        #[allow(clippy::expect_used)]
        std::thread::Builder::new()
            .name("polishcast-capture".into())
            .spawn(move || run(rx, etx))
            .expect("spawn capture thread");
        Self { tx, events }
    }

    /// Records into `dir`, which must exist. Ignored while a recording is running.
    pub fn start(&self, dir: PathBuf, options: Options) {
        let _ = self.tx.send(Cmd::Start { dir, options });
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop { clicked: false });
    }

    /// Stops after a click on Stop, whose way there is left out of the take's project.
    pub fn stop_clicked(&self) {
        let _ = self.tx.send(Cmd::Stop { clicked: true });
    }

    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

#[cfg(any(target_os = "macos", windows))]
fn run(rx: Receiver<Cmd>, events: Sender<Event>) {
    let mut active = None;
    for cmd in rx {
        match cmd {
            Cmd::Start { dir, options } if active.is_none() => match Active::start(dir, &options, events.clone()) {
                Ok(a) => active = Some(a),
                Err(e) => {
                    let _ = events.send(Event::Failed(format!("{e:#}")));
                }
            },
            Cmd::Start { .. } => {}
            Cmd::Stop { clicked } => {
                if let Some(a) = active.take() {
                    let _ = events.send(match a.finish(clicked) {
                        Ok(take) => Event::Finished(take),
                        Err(e) => Event::Failed(format!("{e:#}")),
                    });
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn run(rx: Receiver<Cmd>, events: Sender<Event>) {
    for cmd in rx {
        if let Cmd::Start { .. } = cmd {
            let _ = events.send(Event::Failed("Recording is only available on macOS and Windows for now.".into()));
        }
    }
}
