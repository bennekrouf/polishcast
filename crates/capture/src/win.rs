//! Windows.Graphics.Capture session (Windows 10 2004 or later): the monitor the pointer is on,
//! without the cursor, encoded by Media Foundation (hardware where the GPU has it) at a bitrate
//! we choose, so unlike on macOS the file needs no shrinking afterwards. The microphone comes
//! through cpal and is muxed into the same file.

use crate::pcm::ToStereo16;
use crate::pointer::{self, Logger};
use crate::{Codec, Event, Options, Take};
use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use polishcast_core::take;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows_capture::capture::{CaptureControl, Context as CaptureContext, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder, VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Default)]
struct Shared {
    /// When the first frame arrived: time zero for the pointer log.
    started: Mutex<Option<Instant>>,
}

/// What the capture thread's handler needs, handed over through the capture settings.
#[derive(Clone)]
pub struct Setup {
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    codec: Codec,
    /// Sample rate and 16-bit stereo buffers from the microphone.
    audio: Option<(u32, Receiver<Vec<u8>>)>,
    shared: Arc<Shared>,
    events: Sender<Event>,
}

/// About 0.04 bit per pixel: 5 Mbit/s at 1080p60, 20 at 4K60. Screen content is mostly still,
/// so this keeps text sharp at roughly the size macOS takes reach after compacting.
fn bitrate(width: u32, height: u32, fps: u32) -> u32 {
    (width as u64 * height as u64 * fps as u64 / 25).clamp(2_000_000, 40_000_000) as u32
}

pub struct Handler {
    encoder: Option<VideoEncoder>,
    audio: Option<Receiver<Vec<u8>>>,
    shared: Arc<Shared>,
    events: Sender<Event>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Setup;
    type Error = BoxError;

    fn new(ctx: CaptureContext<Setup>) -> Result<Self, BoxError> {
        let s = ctx.flags;
        let video = VideoSettingsBuilder::new(s.width, s.height)
            .sub_type(match s.codec {
                Codec::H264 => VideoSettingsSubType::H264,
                Codec::Hevc => VideoSettingsSubType::HEVC,
            })
            .bitrate(bitrate(s.width, s.height, s.fps))
            .frame_rate(s.fps);
        let audio = match &s.audio {
            Some((rate, _)) => AudioSettingsBuilder::new().sample_rate(*rate).channel_count(2).bit_per_sample(16),
            None => AudioSettingsBuilder::new().disabled(true),
        };
        let encoder = VideoEncoder::new(video, audio, ContainerSettingsBuilder::new(), &s.path)?;
        Ok(Self { encoder: Some(encoder), audio: s.audio.map(|(_, rx)| rx), shared: s.shared, events: s.events })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _: InternalCaptureControl) -> Result<(), BoxError> {
        let first = {
            let mut started = self.shared.started.lock().unwrap_or_else(PoisonError::into_inner);
            let first = started.is_none();
            started.get_or_insert_with(Instant::now);
            first
        };
        let Some(encoder) = self.encoder.as_mut() else {
            return Ok(());
        };
        if let Some(audio) = &self.audio {
            for buf in audio.try_iter() {
                // Sound from before the first frame would play ahead of the picture.
                if !first {
                    encoder.send_audio_buffer(&buf, 0)?;
                }
            }
        }
        encoder.send_frame(frame)?;
        if first {
            let _ = self.events.send(Event::Started);
        }
        Ok(())
    }
}

pub struct Active {
    control: CaptureControl<Handler, BoxError>,
    /// Kept alive while recording; dropping it stops the microphone.
    mic: Option<cpal::Stream>,
    pointer: Logger,
    shared: Arc<Shared>,
    dir: PathBuf,
    /// The monitor's area in physical desktop pixels, to normalize pointer positions.
    origin: (f64, f64),
    size: (f64, f64),
    width: u32,
    height: u32,
    clicked: bool,
}

impl Active {
    pub fn start(dir: PathBuf, options: &Options, events: Sender<Event>) -> Result<Self> {
        if options.source != crate::Source::Display {
            bail!("Recording a window or an area of the screen is only on macOS for now.");
        }
        pointer::prepare_thread();
        let at = pointer::location().unwrap_or_default();
        // SAFETY: plain Win32 queries; `info` outlives the call that fills it.
        let (hmon, rect) = unsafe {
            let hmon = MonitorFromPoint(POINT { x: at.0 as i32, y: at.1 as i32 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(hmon, &mut info).as_bool() {
                bail!("Can't read the monitor's size");
            }
            (hmon, info.rcMonitor)
        };
        let (w, h) = ((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32);
        let (width, height) = (w / 2 * 2, h / 2 * 2);

        // Without a microphone (none, or none allowed), record the picture alone.
        let (mic, audio) = match options.microphone.then(microphone) {
            Some(Ok((stream, rate, rx))) => (Some(stream), Some((rate, rx))),
            _ => (None, None),
        };

        let shared = Arc::new(Shared::default());
        let setup = Setup {
            path: dir.join(take::SCREEN),
            width,
            height,
            fps: options.fps.max(1),
            codec: options.codec,
            audio,
            shared: shared.clone(),
            events,
        };
        let settings = |border| {
            Settings::new(
                Monitor::from_raw_hmonitor(hmon.0),
                CursorCaptureSettings::WithoutCursor,
                border,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / setup.fps as f64)),
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                setup.clone(),
            )
        };
        let pointer = Logger::start();
        // Hiding the yellow capture border needs Windows 11; older versions keep it.
        let control = Handler::start_free_threaded(settings(DrawBorderSettings::WithoutBorder))
            .or_else(|_| Handler::start_free_threaded(settings(DrawBorderSettings::Default)))
            .map_err(|e| anyhow!("Starting the capture: {e}"))?;
        if let Some(m) = &mic {
            m.play().context("Starting the microphone")?;
        }
        Ok(Self {
            control,
            mic,
            pointer,
            shared,
            dir,
            origin: (rect.left as f64, rect.top as f64),
            size: (w as f64, h as f64),
            width,
            height,
            clicked: options.clicked,
        })
    }

    pub fn finish(self, clicked: bool) -> Result<Take> {
        let stopped_at = Instant::now();
        drop(self.mic);
        let handler = self.control.callback();
        let stopped = self.control.stop();
        let raw = self.pointer.finish();
        stopped.map_err(|e| anyhow!("Stopping the capture: {e}"))?;

        let mut handler = handler.lock();
        let mut encoder = handler.encoder.take().context("The recording never started")?;
        if let Some(audio) = &handler.audio {
            for buf in audio.try_iter() {
                encoder.send_audio_buffer(&buf, 0).map_err(|e| anyhow!("Writing the sound: {e}"))?;
            }
        }
        encoder.finish().map_err(|e| anyhow!("Finishing the recording: {e}"))?;

        let started = self
            .shared
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .context("The recording never started (no frame arrived)")?;
        if !self.dir.join(take::SCREEN).is_file() {
            bail!("The recording wasn't saved.");
        }
        let log = pointer::event_log(&raw, started, self.origin, self.size);
        std::fs::write(self.dir.join(take::EVENTS), serde_json::to_vec(&log)?)?;
        let duration = stopped_at.saturating_duration_since(started).as_secs_f64();
        let kept = pointer::kept(&raw, started, stopped_at, self.clicked, clicked);
        Ok(Take { dir: self.dir, duration, width: self.width, height: self.height, compact: false, sound: None, kept })
    }
}

/// The default microphone, delivering 16-bit stereo buffers at the returned sample rate.
fn microphone() -> Result<(cpal::Stream, u32, Receiver<Vec<u8>>)> {
    let device = cpal::default_host().default_input_device().context("No microphone")?;
    let config = device.default_input_config()?;
    let mut convert = ToStereo16::new(config.sample_rate().0, config.channels());
    let rate = convert.out_rate;
    let (tx, rx) = crossbeam_channel::unbounded();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _: &_| {
                let _ = tx.send(convert.convert(data.iter().copied()));
            },
            |_| {},
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.into(),
            move |data: &[i16], _: &_| {
                let _ = tx.send(convert.convert(data.iter().map(|&s| s as f32 / 32_768.0)));
            },
            |_| {},
            None,
        )?,
        other => bail!("Unsupported microphone format {other:?}"),
    };
    Ok((stream, rate, rx))
}
