//! Export: a take, as the editor shows it, to an MP4 or an animated GIF.
//!
//! ffmpeg decodes the recording into raw frames (resampled to the export's frame rate, and
//! scaled to the resolution the deepest zoom needs), `Compositor` draws each output frame from
//! the same `Scene::frame` the preview uses, and a second ffmpeg encodes them (hardware H.264 on
//! macOS) with the recording's sound copied over. ffmpeg is downloaded on first use (`tools`).

pub mod compact;
pub mod http;
pub mod sound;
pub mod tools;

use anyhow::{anyhow, bail, Context, Result};
use polishcast_core::project::Background;
use polishcast_core::{take, EventLog, Project, Timeline};
use polishcast_render::compose::{load_image, Compositor};
use polishcast_render::Scene;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use tiny_skia::{Pixmap, PixmapRef};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// H.264 with the sound.
    #[default]
    Mp4,
    /// Silent and looping, for pasting into a page, a ticket or a chat.
    Gif,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Gif => "gif",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub format: Format,
    /// Pixels along the output's longer side.
    pub long_side: u32,
    pub fps: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { format: Format::Mp4, long_side: 1920, fps: 30 }
    }
}

/// Frames written so far, out of the total; shared with the UI while an export runs. While
/// ffmpeg is being downloaded first, the download's progress instead.
#[derive(Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    downloading: AtomicBool,
}

impl Progress {
    pub fn downloading(&self) -> bool {
        self.downloading.load(Ordering::Relaxed)
    }

    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            (self.done.load(Ordering::Relaxed) as f32 / total as f32).min(1.0)
        }
    }
}

/// The error an export returns when `cancel` was set.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Export cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// An installed ffmpeg: ours, then the PATH, then where Homebrew, winget, Scoop, Chocolatey and
/// TrackCut put it. Apps opened from the Finder don't get the shell's PATH, and an app already
/// open doesn't see an install made since, hence the fixed places.
pub fn ffmpeg() -> Option<PathBuf> {
    let name = format!("ffmpeg{}", std::env::consts::EXE_SUFFIX);
    let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let local = dirs::data_local_dir().unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    let mut fixed = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/snap/bin"),
        local.join("Splitter").join("tools"),
    ];
    if cfg!(windows) {
        fixed.push(local.join("Microsoft").join("WinGet").join("Links"));
        fixed.extend(winget_bins(&local.join("Microsoft").join("WinGet").join("Packages")));
        fixed.push(home.join("scoop").join("shims"));
        fixed.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin"));
    }
    std::iter::once(tools::dir()).chain(path).chain(fixed).map(|dir| dir.join(&name)).find(|p| p.is_file())
}

/// The `bin` folders of the ffmpeg packages winget unpacked into `packages`
/// (`Gyan.FFmpeg_…\ffmpeg-7.1-full_build\bin`), for when it made no link in `Links`.
fn winget_bins(packages: &Path) -> Vec<PathBuf> {
    let dirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
    };
    dirs(packages)
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains("ffmpeg")))
        .flat_map(|package| dirs(&package))
        .map(|build| build.join("bin"))
        .collect()
}

/// ffmpeg isn't installed and couldn't be downloaded (offline, or blocked by a company
/// network): the editor then says how to install it by hand.
#[derive(Debug)]
pub struct FfmpegMissing(pub String);

impl std::fmt::Display for FfmpegMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Getting ffmpeg: {}", self.0)
    }
}

impl std::error::Error for FfmpegMissing {}

/// ffmpeg, downloaded into the app's tools folder if none is installed. `progress` gets the
/// fraction downloaded; `cancelled` is polled between chunks.
pub fn ensure_ffmpeg(progress: &mut dyn FnMut(f32), cancelled: &dyn Fn() -> bool) -> Result<PathBuf> {
    match ffmpeg() {
        Some(path) => Ok(path),
        None => tools::ensure_ffmpeg(&tools::dir(), progress, cancelled).map_err(|e| FfmpegMissing(e).into()),
    }
}

/// Exports the take in `dir` to `out`. Removes `out` again if it fails or is cancelled.
pub fn export(dir: &Path, out: &Path, settings: Settings, progress: &Progress, cancel: &AtomicBool) -> Result<()> {
    let result = run(dir, out, settings, progress, cancel);
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

fn run(dir: &Path, out: &Path, settings: Settings, progress: &Progress, cancel: &AtomicBool) -> Result<()> {
    let ffmpeg = if let Some(path) = ffmpeg() {
        path
    } else {
        progress.downloading.store(true, Ordering::Relaxed);
        progress.total.store(1000, Ordering::Relaxed);
        let got = ensure_ffmpeg(&mut |f| progress.done.store((f * 1000.0) as u64, Ordering::Relaxed), &|| {
            cancel.load(Ordering::Relaxed)
        });
        progress.downloading.store(false, Ordering::Relaxed);
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        got?
    };
    let project = Project::load(&dir.join(take::PROJECT)).context("Reading the project")?;
    let events: EventLog =
        std::fs::read(dir.join(take::EVENTS)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let screen = dir.join(take::SCREEN);
    let fps = settings.fps.max(1);

    let image = match &project.style.background {
        Background::Image(name) => {
            Some(load_image(&dir.join(name)).map_err(|e| anyhow!("Reading the background picture {name}: {e}"))?)
        }
        _ => None,
    };
    let scene = Scene::new(project, &events);
    let (width, height) = scene.output_size(settings.long_side);
    let compositor =
        Compositor::with_image(&scene, width, height, image.as_ref().map(|i| i.as_ref())).map_err(|e| anyhow!(e))?;
    let (sw, sh) = source_size(&scene, width, height);
    // Blanks cut: the picture skips them frame by frame, the sound through a filter graph.
    // Faster than recorded: the recording is read at `fps ÷ speed` (fewer frames per second
    // of recording) and every frame kept is written at `fps`, and the sound is sped up in the
    // same graph.
    let timeline = scene.project.timeline();
    let speed = scene.project.speed();
    let read_fps = fps as f64 / speed;
    let cutting = timeline.parts.len() != 1 || timeline.duration() < scene.project.duration;
    let gif = settings.format == Format::Gif;
    let tracks = if gif { 0 } else { sound::audio_tracks(&ffmpeg, &screen)? };
    let voice = match sound::filter(&timeline, cutting, speed, tracks) {
        Some(graph) if tracks > 0 => Some(graph),
        _ => None,
    };
    // Music, placed in video time: inputs after the recording.
    let music = if gif { Vec::new() } else { scene.project.placed_music() };
    let songs: Vec<PathBuf> = music.iter().map(|m| dir.join(&scene.project.music[m.index].file)).collect();
    if let Some(missing) = songs.iter().find(|s| !s.is_file()) {
        bail!("The music file {} is missing from the take's folder.", missing.display());
    }
    // Lowered where the voice speaks, found from the recording's loudness (kept by the editor).
    let speaking = match scene.project.duck {
        Some(_) if !music.is_empty() && tracks > 0 => {
            let kept = std::fs::read(dir.join(take::LOUDNESS)).ok().and_then(|b| serde_json::from_slice(&b).ok());
            let loudness = match kept {
                Some(l) => Some(l),
                None => sound::loudness(&ffmpeg, &screen)?,
            };
            loudness.map(|l| scene.project.speaking(&l))
        }
        _ => None,
    };
    let duck = scene.project.duck.zip(speaking.as_deref());
    let sound_filter = sound::with_music(voice, tracks, &music, duck, timeline.duration() / speed);
    let total = (timeline.duration() * fps as f64 / speed).ceil() as u64;
    progress.total.store(total.max(1), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);

    let mut decoder = tools::command(&ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&screen)
        .args(["-an", "-vf", &format!("fps={read_fps:.6},scale={sw}:{sh}:flags=bicubic")])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Starting ffmpeg")?;
    let decoder_errors = collect(decoder.stderr.take());

    let mut encoder = tools::command(&ffmpeg);
    encoder.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba"]).args([
        "-s",
        &format!("{width}x{height}"),
        "-r",
        &fps.to_string(),
        "-i",
        "-",
    ]);
    if gif {
        // One palette for the whole video, built from what changes between frames, and only the
        // changed rectangle of each frame stored: small files that keep text readable.
        encoder.args(["-filter_complex", GIF_FILTER, "-loop", "0"]);
    } else {
        encoder.arg("-i").arg(&screen);
        for song in &songs {
            encoder.arg("-i").arg(song);
        }
        encoder
            .args(if let Some(graph) = &sound_filter {
                vec!["-filter_complex".to_string(), graph.clone(), "-map".into(), "0:v:0".into()]
                    .into_iter()
                    .chain(["-map".into(), "[aout]".into()])
                    .collect::<Vec<String>>()
            } else {
                ["-map", "0:v:0", "-map", "1:a:0?"].map(String::from).to_vec()
            })
            .args(["-vf", "scale=out_color_matrix=bt709:out_range=tv", "-pix_fmt", "yuv420p"])
            .args(["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709"])
            .args(video_codec(width, height, fps))
            .args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"]);
    }
    let mut encoder =
        encoder.arg(out).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().context("Starting ffmpeg")?;
    let encoder_errors = collect(encoder.stderr.take());

    let pumped = pump(&mut decoder, &mut encoder, &scene, &timeline, &compositor, (sw, sh), read_fps, progress, cancel);
    if pumped.is_err() {
        let _ = decoder.kill();
        let _ = encoder.kill();
    }
    drop(encoder.stdin.take());
    let decoded = decoder.wait()?;
    let encoded = encoder.wait()?;
    let errors = |h: JoinHandle<String>| h.join().unwrap_or_default().trim().to_string();
    let (decoder_errors, encoder_errors) = (errors(decoder_errors), errors(encoder_errors));
    match pumped {
        Err(e) if e.is::<Cancelled>() => return Err(e),
        Err(e) if !encoder_errors.is_empty() => bail!("{e}: {encoder_errors}"),
        Err(e) => return Err(e),
        Ok(()) => {}
    }
    if !decoded.success() {
        bail!("Reading the recording failed: {decoder_errors}");
    }
    if !encoded.success() {
        bail!("Encoding failed: {encoder_errors}");
    }
    Ok(())
}

/// Reads decoded frames, draws each one the timeline shows, and hands it to the encoder.
#[allow(clippy::too_many_arguments)]
fn pump(
    decoder: &mut Child,
    encoder: &mut Child,
    scene: &Scene,
    timeline: &Timeline,
    compositor: &Compositor,
    (sw, sh): (u32, u32),
    // The rate the recording is read at: the export's, divided by the playback speed.
    read_fps: f64,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut input = decoder.stdout.take().context("decoder output")?;
    let mut output = encoder.stdin.take().context("encoder input")?;
    let mut source = vec![0u8; sw as usize * sh as usize * 4];
    let (width, height) = compositor.size();
    let mut frame_out = Pixmap::new(width, height).context("frame size")?;
    // `j` counts recording frames, `i` the frames written (the cut ones are skipped).
    let (mut i, mut j) = (0u64, 0u64);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        match input.read_exact(&mut source) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let t = j as f64 / read_fps;
        j += 1;
        if !timeline.shows(t) {
            continue;
        }
        let src = PixmapRef::from_bytes(&source, sw, sh).context("source frame")?;
        let frame = scene.frame(t, width, height);
        compositor.draw(&frame, src, &mut frame_out);
        output.write_all(frame_out.data()).map_err(|e| anyhow!("Writing to the encoder: {e}"))?;
        i += 1;
        progress.done.store(i, Ordering::Relaxed);
    }
    if i == 0 {
        bail!("The video has no frames: the recording is empty, or everything in it is cut");
    }
    Ok(())
}

/// Size to decode the recording at: enough for the deepest zoom to stay sharp, never more than
/// the recording itself (and even, for the scaler).
fn source_size(scene: &Scene, width: u32, height: u32) -> (u32, u32) {
    let p = &scene.project;
    let content = scene.frame(0.0, width, height).content;
    let deepest = p.zooms.iter().map(|z| z.scale).fold(1.0f32, f32::max);
    let w = (content.w * deepest).ceil().min(p.width as f32).max(2.0);
    let h = w * p.height as f32 / p.width as f32;
    let even = |v: f32| (v.round() as u32 / 2 * 2).max(2);
    (even(w), even(h))
}

/// Encodes a GIF from the frames on input 0.
const GIF_FILTER: &str =
    "[0:v]split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle";

fn video_codec(width: u32, height: u32, fps: u32) -> Vec<String> {
    // About 0.1 bit per pixel: screen content compresses well, text stays crisp.
    let bitrate = (width as u64 * height as u64 * fps as u64 / 10).max(2_000_000);
    let args: Vec<String> = if cfg!(target_os = "macos") {
        vec![
            "-c:v".into(),
            "h264_videotoolbox".into(),
            "-allow_sw".into(),
            "1".into(),
            "-profile:v".into(),
            "high".into(),
        ]
    } else {
        vec!["-c:v".into(), "libx264".into(), "-preset".into(), "medium".into()]
    };
    args.into_iter().chain(["-b:v".into(), bitrate.to_string()]).collect()
}

/// Everything a child process writes to stderr, collected on a thread so it never blocks.
fn collect(stderr: Option<ChildStderr>) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut s) = stderr {
            let _ = s.read_to_string(&mut text);
        }
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_that_failed_is_told_apart_from_other_errors() {
        let e: anyhow::Error = FfmpegMissing("Could not reach github.com".into()).into();
        assert!(e.is::<FfmpegMissing>());
        assert_eq!(e.to_string(), "Getting ffmpeg: Could not reach github.com");
        assert!(!anyhow::anyhow!("Reading the project").is::<FfmpegMissing>());
    }

    #[test]
    fn winget_packages_are_searched_for_their_bin_folder() {
        let root = std::env::temp_dir().join(format!("polishcast-winget-{}", std::process::id()));
        let bin =
            root.join("Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe").join("ffmpeg-7.1-full_build").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(root.join("Git.Git_x").join("cmd")).unwrap();
        assert_eq!(winget_bins(&root), vec![bin]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
