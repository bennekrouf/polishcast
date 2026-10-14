//! ScreenCaptureKit session: the display the pointer is on, one window, or an area of a
//! display, written straight to a file by `SCRecordingOutput` (hardware encoded, macOS 15+).

use crate::pointer::{self, Logger};
use crate::{Area, Codec, Event, Options, Source, Take, WindowInfo};
use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::Sender;
use polishcast_core::take;
use screencapturekit::prelude::*;
use screencapturekit::recording_output::{
    RecordingCallbacks, SCRecordingOutput, SCRecordingOutputCodec, SCRecordingOutputConfiguration,
    SCRecordingOutputFileType,
};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Shared {
    /// When the first frame was written: time zero for the pointer log.
    started: Mutex<Option<Instant>>,
    failed: Mutex<Option<String>>,
    /// Set when the file is complete or the recording failed.
    done: Mutex<bool>,
    done_cv: Condvar,
}

impl Shared {
    fn set_done(&self) {
        *self.done.lock().unwrap() = true;
        self.done_cv.notify_all();
    }

    /// Whether the recording ended within `timeout`.
    fn wait_done(&self, timeout: Duration) -> bool {
        let done = self.done.lock().unwrap();
        *self.done_cv.wait_timeout_while(done, timeout, |d| !*d).unwrap().0
    }
}

pub struct Active {
    stream: SCStream,
    output: SCRecordingOutput,
    pointer: Logger,
    shared: Arc<Shared>,
    dir: PathBuf,
    /// What's recorded, in global points, to normalize pointer positions: the display, the
    /// window where it was when recording started, or the area.
    frame: CGRect,
    width: u32,
    height: u32,
}

fn sc<E: std::fmt::Debug>(what: &'static str) -> impl FnOnce(E) -> anyhow::Error {
    move |e| anyhow!("{what}: {e:?}")
}

const PERMISSION: &str =
    "Can't list the screens. Allow Polishcast in System Settings › Privacy & Security › Screen Recording";

/// Windows worth offering: on screen, normal (not a menu or the Dock), big enough, not ours.
pub fn windows() -> Vec<WindowInfo> {
    let Ok(content) = SCShareableContent::get() else {
        return Vec::new();
    };
    let me = std::process::id() as i32;
    let mut out: Vec<WindowInfo> = content
        .windows()
        .iter()
        .filter(|w| w.is_on_screen() && w.window_layer() == 0)
        .filter(|w| w.frame().size.width >= 80.0 && w.frame().size.height >= 80.0)
        .filter_map(|w| {
            let app = w.owning_application()?;
            (app.process_id() != me).then(|| WindowInfo {
                id: w.window_id(),
                app: app.application_name(),
                title: w.title().unwrap_or_default(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.app.to_lowercase().cmp(&b.app.to_lowercase()).then(a.title.cmp(&b.title)));
    out
}

/// How long ScreenCaptureKit gets to finish writing the file after the recording stops.
const FINALIZING: Duration = Duration::from_secs(60);

/// Whether a QuickTime file has its index (`moov`), which is written when the file is
/// finished. Only the top-level box headers are read.
fn has_index(file: &std::path::Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(file) else {
        return false;
    };
    let len = f.metadata().map_or(0, |m| m.len());
    let mut at = 0u64;
    let mut header = [0u8; 16];
    while at + 8 <= len {
        if f.seek(SeekFrom::Start(at)).is_err() || f.read_exact(&mut header[..8]).is_err() {
            return false;
        }
        if &header[4..8] == b"moov" {
            return true;
        }
        let size = match u32::from_be_bytes(header[..4].try_into().unwrap()) as u64 {
            1 if f.read_exact(&mut header[8..]).is_ok() => u64::from_be_bytes(header[8..].try_into().unwrap()),
            // Zero runs to the end of the file: a box still being written.
            0 | 1 => return false,
            n => n,
        };
        if size < 8 {
            return false;
        }
        at += size;
    }
    false
}

fn contains(f: CGRect, (x, y): (f64, f64)) -> bool {
    x >= f.origin.x && x < f.origin.x + f.size.width && y >= f.origin.y && y < f.origin.y + f.size.height
}

impl Active {
    pub fn start(dir: PathBuf, options: &Options, events: Sender<Event>) -> Result<Self> {
        pointer::prepare_thread();
        let content = SCShareableContent::get().map_err(sc(PERMISSION))?;
        let displays = content.displays();
        // Leave Polishcast's own windows out of a display's recording.
        let me = std::process::id() as i32;
        let own: Vec<_> = content.applications().into_iter().filter(|a| a.process_id() == me).collect();
        let own: Vec<&_> = own.iter().collect();
        let display_at = |point: Option<(f64, f64)>| {
            displays
                .iter()
                .find(|d| point.is_some_and(|p| contains(d.frame(), p)))
                .or(displays.first())
                .context("No display to record")
        };
        let display_filter = |display: &SCDisplay| {
            SCContentFilter::create().with_display(display).with_excluding_applications(&own, &[]).build()
        };

        // The filter, what it shows in global points, and the part of the display to keep.
        let (filter, frame, crop) = match options.source {
            Source::Display => {
                let display = display_at(pointer::location())?;
                (display_filter(display).map_err(sc("Content filter"))?, display.frame(), None)
            }
            Source::Window(id) => {
                let windows = content.windows();
                let window = windows
                    .iter()
                    .find(|w| w.window_id() == id)
                    .context("That window has closed. Choose another one to record.")?;
                let filter = SCContentFilter::create().with_window(window).build().map_err(sc("Content filter"))?;
                (filter, window.frame(), None)
            }
            Source::Area(Area { x, y, w, h }) => {
                let display = display_at(Some((x + w / 2.0, y + h / 2.0)))?;
                let d = display.frame();
                // Kept within the display it's mostly on.
                let left = x.max(d.origin.x);
                let top = y.max(d.origin.y);
                let right = (x + w).min(d.origin.x + d.size.width);
                let bottom = (y + h).min(d.origin.y + d.size.height);
                if right - left < 16.0 || bottom - top < 16.0 {
                    bail!("The area is too small to record.");
                }
                let area = CGRect::new(left, top, right - left, bottom - top);
                let local = CGRect::new(left - d.origin.x, top - d.origin.y, right - left, bottom - top);
                (display_filter(display).map_err(sc("Content filter"))?, area, Some(local))
            }
        };

        let scale = filter.point_pixel_scale().max(1.0);
        let full = frame.size.width * scale as f64;
        let px = options.max_width.map_or(full, |m| full.min(m as f64)) / frame.size.width;
        let width = (frame.size.width * px).round() as u32 / 2 * 2;
        let height = (frame.size.height * px).round() as u32 / 2 * 2;
        let mut config = SCStreamConfiguration::new().with_width(width).with_height(height);
        if let Some(rect) = crop {
            config = config.with_source_rect(rect);
        }
        let config = config
            .with_fps(options.fps)
            .with_shows_cursor(false)
            .with_captures_audio(options.system_audio)
            .with_captures_microphone(options.microphone)
            .map_err(sc("Microphone"))?;

        let shared = Arc::new(Shared::default());
        let callbacks = {
            let (on_start, on_fail, on_finish) = (shared.clone(), shared.clone(), shared.clone());
            RecordingCallbacks::new()
                .on_start(move || {
                    *on_start.started.lock().unwrap() = Some(Instant::now());
                    let _ = events.send(Event::Started);
                })
                .on_fail(move |e| {
                    *on_fail.failed.lock().unwrap() = Some(e);
                    on_fail.set_done();
                })
                .on_finish(move || on_finish.set_done())
        };
        let out_config = SCRecordingOutputConfiguration::new()
            .map_err(sc("Recording output"))?
            .with_output_url(&dir.join(take::SCREEN))?
            .with_video_codec(match options.codec {
                Codec::H264 => SCRecordingOutputCodec::H264,
                Codec::Hevc => SCRecordingOutputCodec::HEVC,
            })
            .with_output_file_type(SCRecordingOutputFileType::MOV);
        let output = SCRecordingOutput::new_with_delegate(&out_config, callbacks)
            .context("Recording to a file needs macOS 15 or later")?;

        let stream = SCStream::new(&filter, &config).map_err(sc("Stream"))?;
        stream.add_recording_output(&output).map_err(sc("Recording output"))?;
        let pointer = Logger::start();
        stream.start_capture().map_err(sc("Starting the capture"))?;
        Ok(Self { stream, output, pointer, shared, dir, frame, width, height })
    }

    pub fn finish(self) -> Result<Take> {
        // Removing the only output stops the capture and waits until the file is complete.
        let removed = self.stream.remove_recording_output(&self.output).map_err(sc("Stopping"));
        let _ = self.stream.stop_capture();
        let raw = self.pointer.finish();
        removed?;
        // The file is finalized after the stream stops; its delegate says when. A long take
        // can take a while, and its index is only written at the very end.
        let ended = self.shared.wait_done(FINALIZING);
        if let Some(e) = self.shared.failed.lock().unwrap().take() {
            bail!("Recording failed: {e}");
        }
        let file = self.dir.join(take::SCREEN);
        if !file.is_file() {
            bail!(
                "The recording wasn't saved ({}).",
                if ended { "ScreenCaptureKit wrote no file" } else { "it never finished" }
            );
        }
        let d = self.output.recorded_duration();
        let duration = if d.timescale > 0 { d.value as f64 / d.timescale as f64 } else { 0.0 };
        // Opening or compacting a file without its index fails, so wait for it, with the
        // session released in case that's what lets it be written.
        drop(self.stream);
        drop(self.output);
        let deadline = Instant::now() + FINALIZING;
        while !has_index(&file) {
            if Instant::now() >= deadline {
                bail!("The recording wasn't saved: ScreenCaptureKit never finished writing it.");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let started = self.shared.started.lock().unwrap().context("The recording never started")?;

        let f = self.frame;
        let log = pointer::event_log(&raw, started, (f.origin.x, f.origin.y), (f.size.width, f.size.height));
        std::fs::write(self.dir.join(take::EVENTS), serde_json::to_vec(&log)?)?;
        // ScreenCaptureKit's file output has no bitrate setting: worth shrinking afterwards.
        Ok(Take { dir: self.dir, duration, width: self.width, height: self.height, compact: true })
    }
}

#[cfg(test)]
mod tests {
    use super::has_index;

    fn r#box(kind: &[u8; 4], body: usize) -> Vec<u8> {
        let mut b = ((8 + body) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.resize(8 + body, 0);
        b
    }

    #[test]
    fn index_is_found_only_once_written() {
        let dir = std::env::temp_dir().join(format!("polishcast-index-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("screen.mov");
        let mut bytes = [r#box(b"ftyp", 12), r#box(b"wide", 0), r#box(b"mdat", 1000)].concat();
        // Still recording: the media box is cut short.
        std::fs::write(&file, &bytes[..500]).unwrap();
        assert!(!has_index(&file));
        // Media complete, index not written yet.
        std::fs::write(&file, &bytes).unwrap();
        assert!(!has_index(&file));
        bytes.extend(r#box(b"moov", 100));
        std::fs::write(&file, &bytes).unwrap();
        assert!(has_index(&file));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
