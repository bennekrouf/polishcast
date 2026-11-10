//! The editor: preview, transport, timeline of zooms, and the style or zoom inspector.
//!
//! The preview is drawn by `assets/preview.js` from the frame track `render` computes; this
//! component owns the project, its undo history and saving.

use crate::ffmpeg_help::{self, FfmpegHelp};
use crate::{media_server, platform};
use dioxus::desktop::use_asset_handler;
use dioxus::prelude::*;
use polishcast_core::blanks::redetect;
use polishcast_core::camera::viewport_at;
use polishcast_core::clips;
use polishcast_core::history::History;
use polishcast_core::mask::{Cover, Mask};
use polishcast_core::music::{self, Music};
use polishcast_core::project::{Aspect, Background, DENOISE_STRENGTH, SPEEDS};
use polishcast_core::zoom::{self, Zoom};
use polishcast_core::{take, BlankParams, EventLog, Loudness, Project};
use polishcast_export::{Format, Progress, Settings};
use polishcast_render::{compose, Scene};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
mod panels;
use panels::*;
mod actions;
mod export;
mod media;
mod transcript;
pub(crate) use export::*;
use transcript::TranscriptPanel;

const PREVIEW_JS: &str = include_str!("../assets/preview.js");
/// Frames per second of the preview's track; it interpolates in between.
const TRACK_FPS: f64 = 30.0;
/// Size the preview's track is laid out at (scaled to the window by CSS).
const TRACK_LONG_SIDE: u32 = 1920;
/// A zoom added by hand lasts this long, unless it runs into the next one.
const NEW_ZOOM_SECS: f64 = 2.5;
/// A hidden area added by hand lasts this long.
const NEW_MASK_SECS: f64 = 3.0;
/// And takes up this much of what's on view.
const NEW_MASK_SIZE: (f32, f32) = (0.25, 0.08);
const COVERS: &[(Cover, &str)] = &[(Cover::Solid, "Solid box"), (Cover::Blur, "Blur")];

const BACKGROUNDS: &[(&str, &str)] =
    &[("#4f46e5", "#db2777"), ("#0ea5e9", "#22c55e"), ("#f59e0b", "#ef4444"), ("#1e293b", "#475569")];
const COLORS: &[&str] = &["#f4f4f5", "#18181b"];
const ASPECTS: &[(Aspect, &str)] = &[
    (Aspect::Source, "Screen"),
    (Aspect::Landscape16x9, "16:9"),
    (Aspect::Portrait9x16, "9:16"),
    (Aspect::Square, "1:1"),
];

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Msg {
    Tick {
        t: f64,
        playing: bool,
    },
    Key {
        key: String,
        cmd: bool,
        shift: bool,
    },
    /// A hidden area was clicked on the preview.
    Pick {
        index: usize,
    },
    /// A hidden area was dragged on the preview to here.
    Mask {
        index: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
}

/// The project being edited, with undo and saving. Signals are `Copy`, so this is too.
#[derive(Clone, Copy, PartialEq)]
struct Doc {
    project: Signal<Project>,
    history: Signal<History<Project>>,
    /// The project as it was when a slider or field started changing: one undo step per gesture.
    before: Signal<Option<Project>>,
    path: Signal<PathBuf>,
    error: Signal<Option<String>>,
    /// project.json's modification time as this window last read or wrote it: a different
    /// one means `polishcast mcp`, driven by Claude, changed the take.
    seen: Signal<Option<std::time::SystemTime>>,
}

use desktop_kit::library::modified;

impl Doc {
    /// One undoable edit.
    fn change(self, f: impl FnOnce(&mut Project)) {
        self.live(f);
        self.settle();
    }

    /// Part of a gesture still going on (a slider being dragged).
    fn live(mut self, f: impl FnOnce(&mut Project)) {
        if self.before.peek().is_none() {
            self.before.set(Some(self.project.peek().clone()));
        }
        f(&mut self.project.write());
    }

    /// The gesture ended: record it and save.
    fn settle(mut self) {
        if let Some(before) = self.before.take() {
            if before != *self.project.peek() {
                self.history.write().record(before);
                self.save();
            }
        }
    }

    fn undo(mut self) {
        self.settle();
        let current = self.project.peek().clone();
        let prev = self.history.write().undo(current);
        if let Some(prev) = prev {
            self.project.set(prev);
            self.save();
        }
    }

    fn redo(mut self) {
        self.settle();
        let current = self.project.peek().clone();
        let next = self.history.write().redo(current);
        if let Some(next) = next {
            self.project.set(next);
            self.save();
        }
    }

    fn save(mut self) {
        if let Err(e) = self.project.peek().save(&self.path.peek()) {
            self.error.set(Some(format!("Can't save the project: {e}")));
        }
        let seen = modified(&self.path.peek());
        self.seen.set(seen);
    }

    /// Takes in a change another process saved, unless a gesture is under way here (it is the
    /// newer, and is saved over it). The change is one undo step, so ⌘Z takes it back.
    fn reload_if_changed(mut self) {
        if self.before.peek().is_some() {
            return;
        }
        let path = self.path.peek().clone();
        let now = modified(&path);
        if now == *self.seen.peek() {
            return;
        }
        self.seen.set(now);
        let Ok(changed) = Project::load(&path) else { return };
        let current = self.project.peek().clone();
        if changed != current {
            self.history.write().record(current);
            self.project.set(changed);
        }
    }
}

#[component]
pub fn Editor(dir: PathBuf) -> Element {
    let loaded = use_hook(|| Project::load(&dir.join(take::PROJECT)).map_err(|e| e.to_string()));
    let events = use_hook(|| {
        Rc::new(
            std::fs::read(dir.join(take::EVENTS))
                .ok()
                .and_then(|b| serde_json::from_slice::<EventLog>(&b).ok())
                .unwrap_or_default(),
        )
    });
    match loaded {
        Ok(project) => rsx! { Loaded { dir, project, events } },
        Err(e) => rsx! { p { class: "error pad", "Can't open this take's project: {e}" } },
    }
}

#[component]
fn Loaded(dir: PathBuf, project: Project, events: Rc<EventLog>) -> Element {
    let doc = Doc {
        project: use_signal(|| project),
        history: use_signal(History::default),
        before: use_signal(|| None),
        path: use_signal(|| dir.join(take::PROJECT)),
        error: use_signal(|| None),
        seen: use_signal(|| modified(&dir.join(take::PROJECT))),
    };
    // What Claude changes through `polishcast mcp` shows up here.
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            doc.reload_if_changed();
        }
    });
    let time = use_signal(|| 0.0f64);
    let loudness = use_signal({
        let dir = dir.clone();
        move || match std::fs::read(dir.join(take::LOUDNESS)).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(l) => Measure::Ready(Rc::new(l)),
            None => Measure::Unknown,
        }
    });
    let playing = use_signal(|| false);
    let mut selected = use_signal(|| Option::<usize>::None);
    // A clip picked on the timeline (start and end, in recording time); only one of a zoom and
    // a clip is selected at a time.
    let mut selected_clip = use_signal(|| Option::<(f64, f64)>::None);
    // A piece of music picked on the timeline; likewise exclusive.
    let mut selected_music = use_signal(|| Option::<usize>::None);
    // A hidden area picked on the timeline or the preview; likewise exclusive.
    let mut selected_mask = use_signal(|| Option::<usize>::None);

    // Serve the recording to the web view (127.0.0.1, or the app's own protocol as a fallback).
    let url = use_hook(|| media_server::publish(&dir.join(take::SCREEN)));
    let served = url.clone();
    use_drop(move || {
        if let Some(url) = &served {
            media_server::unpublish(url);
        }
    });
    let mut in_app = use_signal(|| false);
    let mut failed = use_signal(|| false);
    let picture = media::use_served_media(&dir, doc, loudness, in_app);

    media::use_preview_track(doc, events.clone());

    let sel = actions::Sel { time, selected, selected_clip, selected_music, selected_mask };
    let acts = actions::use_actions(&dir, doc, events.clone(), sel);
    let actions::Actions { add_zoom, split_here, add_music, add_mask, .. } = acts;
    // The music picker, while it is open.
    let mut picking_music = use_signal(|| false);
    let take_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    actions::use_preview_messages(doc, sel, playing, acts);

    let project = doc.project.read();
    let duration = project.duration.max(0.001);
    let video_duration = project.video_duration();
    let cutting = project.blanks.is_some();
    let picture_url = picture().map(|(_, url)| if in_app() { media_server::in_app_path(&url) } else { url });
    let background = css_background(&project.style.background, picture_url.as_deref());
    // A zoom removed by undo may leave the selection pointing past the end.
    let selection = selected().filter(|&i| i < project.zooms.len());
    let all_clips = clips::clips(&project.splits, project.duration);
    // Likewise a clip whose split was undone.
    let clip_selection = selected_clip().filter(|c| all_clips.contains(c));
    let music_selection = selected_music().filter(|&i| i < project.music.len());
    let mask_selection = selected_mask().filter(|&i| i < project.masks.len());
    let src = match &url {
        Some(url) if in_app() => media_server::in_app_path(url),
        Some(url) => url.clone(),
        None => String::new(),
    };

    rsx! {
        div { class: "editor",
            div { class: "canvas",
                div { class: "stage-wrap",
                    div { id: "sv-stage", class: "stage",
                        div { class: "background", background: "{background}" }
                        div {
                            id: "sv-content",
                            class: "content",
                            video {
                                id: "sv-video",
                                src: "{src}",
                                preload: "auto",
                                playsinline: true,
                                onclick: move |_| {
                                    document::eval("window.sv.toggle()");
                                },
                                onloadedmetadata: move |_| failed.set(false),
                                onerror: move |_| if in_app() { failed.set(true) } else { in_app.set(true) },
                            }
                            // Moved with the video by the preview script, which also shows each
                            // area while it is on.
                            div { id: "sv-masks", class: "masks",
                                for (i, m) in project.masks.iter().enumerate() {
                                    div {
                                        key: "{i}",
                                        class: {
                                            let cover = if m.cover == Cover::Blur { "blur" } else { "solid" };
                                            let on = if mask_selection == Some(i) { " selected" } else { "" };
                                            format!("mask {cover}{on}")
                                        },
                                        "data-index": "{i}",
                                        "data-from": "{m.from()}",
                                        "data-until": "{m.until()}",
                                        left: "{m.x * 100.0}%",
                                        top: "{m.y * 100.0}%",
                                        width: "{m.w * 100.0}%",
                                        height: "{m.h * 100.0}%",
                                        for grip in ["nw", "ne", "sw", "se"] {
                                            span { key: "{grip}", class: "grip {grip}", "data-grip": "{grip}" }
                                        }
                                    }
                                }
                            }
                        }
                        // Under the cursor, as the export draws it.
                        div { id: "sv-ripple", class: "ripple" }
                        svg {
                            id: "sv-cursor",
                            class: "cursor",
                            view_box: "0 0 20 {compose::CURSOR_BOX}",
                            path {
                                d: compose::cursor_svg_path(),
                                fill: "#000",
                                stroke: "#fff",
                                stroke_width: "{compose::CURSOR_STROKE}",
                                stroke_linejoin: "round",
                            }
                        }
                    }
                }
                if failed() {
                    p { class: "error", "The recording can't be played here." }
                }
                div { class: "transport",
                    button {
                        class: "play",
                        onclick: move |_| {
                            document::eval("window.sv.toggle()");
                        },
                        if playing() { "❚❚" } else { "▶" }
                    }
                    span { class: "time",
                        span { id: "sv-time", "0:00" }
                        " / {clock(video_duration)}"
                    }
                    label { class: "speed", title: "How fast the video plays: the preview and the export",
                        "Speed "
                        select {
                            onchange: move |e| {
                                if let Ok(speed) = e.value().parse::<f64>() {
                                    doc.change(|p| p.speed = speed);
                                }
                            },
                            for &s in SPEEDS {
                                option { key: "{s}", value: "{s}", selected: project.speed() == s, "{s}×" }
                            }
                        }
                    }
                    span { class: "spacer" }
                    button { onclick: move |_| split_here(()), title: "S — then click a clip to remove it", "Split" }
                    button { onclick: move |_| add_zoom(()), title: "Z", "+ Zoom" }
                    button {
                        title: "H — cover something confidential from the playhead",
                        onclick: move |_| add_mask(()),
                        "+ Hide"
                    }
                    // A song starts at the playhead; M then stops and restarts it as the video plays.
                    // From the library first (TrackCut's tracks, songs added before); the disk
                    // is a discreet link inside the picker.
                    button {
                        title: "A song from the playhead. M stops it or starts it again there",
                        onclick: move |_| picking_music.set(true),
                        "+ Music"
                    }
                    button { onclick: move |_| doc.undo(), title: "{platform::MOD}Z", "Undo" }
                    button { onclick: move |_| doc.redo(), title: "{platform::MOD}Shift+Z", "Redo" }
                }
                div { id: "sv-timeline", class: "timeline",
                    if cutting {
                        for (i, c) in project.cuts.iter().enumerate() {
                            div {
                                key: "cut-{i}-{c.start}",
                                class: if c.keep { "cut kept" } else { "cut" },
                                left: "{c.start / duration * 100.0}%",
                                width: "{(c.end - c.start) / duration * 100.0}%",
                                title: if c.keep { "Kept — click to cut it" } else { "Cut — click to keep it" },
                                onclick: move |_| {
                                    doc.change(|p| {
                                        if let Some(c) = p.cuts.get_mut(i) {
                                            c.keep = !c.keep;
                                        }
                                    })
                                },
                            }
                        }
                    }
                    for (i, z) in project.zooms.iter().enumerate() {
                        div {
                            key: "{i}-{z.start}",
                            class: if selection == Some(i) { "zoom selected" } else { "zoom" },
                            left: "{z.start / duration * 100.0}%",
                            width: "{(z.end - z.start) / duration * 100.0}%",
                            // A second click on the selected zoom deselects it.
                            onclick: move |_| {
                                selected_clip.set(None);
                                selected_music.set(None);
                                selected_mask.set(None);
                                let again = *selected.peek() == Some(i);
                                selected.set(if again { None } else { Some(i) });
                            },
                            "{z.scale:.1}×"
                        }
                    }
                    for (i, m) in project.music.iter().enumerate() {
                        div {
                            key: "music-{i}-{m.start}",
                            class: if music_selection == Some(i) { "music selected" } else { "music" },
                            left: "{m.start / duration * 100.0}%",
                            width: "{(m.end.min(duration) - m.start) / duration * 100.0}%",
                            title: "{m.name}",
                            onclick: move |_| {
                                selected.set(None);
                                selected_clip.set(None);
                                selected_mask.set(None);
                                let again = *selected_music.peek() == Some(i);
                                selected_music.set(if again { None } else { Some(i) });
                            },
                            "♪ {m.name}"
                        }
                    }
                    for (i, m) in project.masks.iter().enumerate() {
                        div {
                            key: "mask-{i}-{m.start}",
                            class: if mask_selection == Some(i) { "mask-span selected" } else { "mask-span" },
                            left: "{m.start / duration * 100.0}%",
                            width: "{(m.end - m.start) / duration * 100.0}%",
                            title: "Hidden from the video here",
                            onclick: move |_| {
                                selected.set(None);
                                selected_clip.set(None);
                                selected_music.set(None);
                                let again = *selected_mask.peek() == Some(i);
                                selected_mask.set(if again { None } else { Some(i) });
                            },
                            if m.cover == Cover::Blur { "Blur" } else { "Hidden" }
                        }
                    }
                    div { class: "clips",
                        for &(start, end) in all_clips.iter() {
                            div {
                                key: "clip-{start}-{end}",
                                class: {
                                    let removed = if clips::is_removed(&project.removed, (start, end)) { " removed" } else { "" };
                                    let on = if clip_selection == Some((start, end)) { " selected" } else { "" };
                                    format!("clip{removed}{on}")
                                },
                                left: "{start / duration * 100.0}%",
                                width: "{(end - start) / duration * 100.0}%",
                                title: "Click to select this clip; S splits at the playhead",
                                onclick: move |_| {
                                    selected.set(None);
                                    selected_music.set(None);
                                    selected_mask.set(None);
                                    selected_clip.set(Some((start, end)));
                                },
                            }
                        }
                    }
                    div { id: "sv-playhead", class: "playhead" }
                }
                if let Some(e) = (doc.error)() {
                    p { class: "error", "{e}" }
                }
            }
            aside { class: "inspector",
                if let Some(i) = mask_selection {
                    MaskPanel { doc, index: i, selected_mask }
                } else if let Some(i) = selection {
                    ZoomPanel { doc, index: i, selected }
                } else if let Some(clip) = clip_selection {
                    ClipPanel { doc, clip, selected_clip }
                } else if let Some(i) = music_selection {
                    MusicPanel { doc, dir: dir.clone(), index: i, selected_music, loudness }
                } else {
                    StylePanel { doc, dir: dir.clone(), picture_url: picture_url.clone() }
                }
                BlanksPanel { doc, dir: dir.clone(), loudness }
                NoisePanel { doc, loudness }
                TranscriptPanel { dir: dir.clone() }
                ExportPanel { doc, dir: dir.clone() }
            }
            if picking_music() {
                desktop_kit::ui::FilePicker {
                    want: desktop_kit::library::pick::Want::new("polishcast", "Music for this video")
                        .roles(&[desktop_kit::library::Role::Audio, desktop_kit::library::Role::Source])
                        .extensions(&["mp3", "m4a", "wav", "aac", "flac"])
                        .about(&take_name),
                    on_pick: move |files: Vec<PathBuf>| {
                        picking_music.set(false);
                        if let Some(file) = files.into_iter().next() {
                            add_music(file);
                        }
                    },
                    on_close: move |_| picking_music.set(false),
                }
            }
        }
    }
}

/// A background as CSS. A picture covers the frame, centred, as the export draws it.
fn css_background(b: &Background, picture_url: Option<&str>) -> String {
    match b {
        Background::Color(c) => c.clone(),
        Background::Gradient(a, b) => format!("linear-gradient(to bottom, {a}, {b})"),
        Background::Image(_) => match picture_url {
            Some(url) => format!("url(\"{url}\") center / cover no-repeat, #18181b"),
            None => "#18181b".into(),
        },
    }
}

/// Copies a file chosen for the take (a background picture, a song) into its folder, under a
/// new name each time (so undo brings back the one that was there), and returns that name.
fn adopt_file(dir: &std::path::Path, from: &std::path::Path, kind: &str) -> std::io::Result<String> {
    let ext = from.extension().and_then(|e| e.to_str()).unwrap_or("bin").to_ascii_lowercase();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let name = format!("{kind}-{stamp}.{ext}");
    std::fs::copy(from, dir.join(&name))?;
    Ok(name)
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Removes a clip from the video, or puts it back.
pub(crate) fn toggle_clip(p: &mut Project, clip: (f64, f64)) {
    if clips::is_removed(&p.removed, clip) {
        clips::restore(&mut p.removed, clip);
    } else {
        clips::remove(&mut p.removed, clip);
    }
}

pub(crate) const RESOLUTIONS: &[(u32, &str)] = &[(1920, "1080p"), (2560, "1440p"), (3840, "4K")];
pub(crate) const FRAME_RATES: &[u32] = &[30, 60];
/// GIFs grow fast with size and frame rate; these keep them small enough to paste anywhere.
pub(crate) const GIF_SIZES: &[(u32, &str)] = &[(640, "640 px"), (960, "960 px"), (1280, "1280 px")];
pub(crate) const GIF_FRAME_RATES: &[u32] = &[10, 15, 24];

enum Export {
    Idle,
    Running {
        progress: Arc<Progress>,
        cancel: Arc<AtomicBool>,
        done: mpsc::Receiver<anyhow::Result<()>>,
        out: PathBuf,
    },
    Done(PathBuf),
    Failed(String),
    /// ffmpeg isn't installed and couldn't be downloaded: say how to install it.
    NoFfmpeg(String),
}

/// `name` as typed, made into a file name that works on macOS and Windows: no path separators or
/// reserved characters, no extension typed by habit. `None` when nothing is left.
pub(crate) fn clean_export_name(name: &str, format: Format) -> Option<String> {
    let name = name.trim();
    let name = name.strip_suffix(&format!(".{}", format.extension())).unwrap_or(name);
    let name: String =
        name.chars().map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '-' } else { c }).collect();
    let name = name.trim_end_matches(['.', ' ']).trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Where an export named `name` goes: next to the take's folder.
pub(crate) fn export_path(dir: &std::path::Path, name: &str, format: Format) -> PathBuf {
    dir.with_file_name(format!("{name}.{}", format.extension()))
}

/// The take's loudness, needed to find blanks: measured once (ffmpeg reads the sound) and kept
/// in the take's folder.
#[derive(Clone, PartialEq)]
enum Measure {
    Unknown,
    Measuring,
    Ready(Rc<Loudness>),
    /// The recording has no sound: nothing to go by.
    Silent,
    Failed(String),
    /// ffmpeg, which reads the sound, isn't installed and couldn't be downloaded.
    NoFfmpeg(String),
}

impl Measure {
    fn failed(e: &anyhow::Error) -> Self {
        if ffmpeg_help::is_missing(e) {
            Measure::NoFfmpeg(format!("{e:#}"))
        } else {
            Measure::Failed(format!("{e:#}"))
        }
    }
}

pub(crate) fn measure(dir: &std::path::Path) -> anyhow::Result<Option<Loudness>> {
    let ffmpeg = polishcast_export::ensure_ffmpeg(&mut |_| {}, &|| false)?;
    let loudness = polishcast_export::sound::loudness(&ffmpeg, &dir.join(take::SCREEN))?;
    if let Some(l) = &loudness {
        std::fs::write(dir.join(take::LOUDNESS), serde_json::to_vec(l)?)?;
    }
    Ok(loudness)
}

/// Turns blank cutting on with `params`, finding the cuts again (cuts the user kept stay kept).
pub(crate) fn cut_blanks(p: &mut Project, loudness: &Loudness, params: BlankParams) {
    p.cuts = redetect(&p.cuts, loudness, p.duration, &params);
    p.blanks = Some(params);
}

#[cfg(test)]
mod tests;
