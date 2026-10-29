//! The editor: preview, transport, timeline of zooms, and the style or zoom inspector.
//!
//! The preview is drawn by `assets/preview.js` from the frame track `render` computes; this
//! component owns the project, its undo history and saving.

use crate::ffmpeg_help::{self, FfmpegHelp};
use crate::{media_server, platform};
use dioxus::desktop::use_asset_handler;
use dioxus::prelude::*;
use polishcast_core::blanks::redetect;
use polishcast_core::clips;
use polishcast_core::history::History;
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
mod export;
pub(crate) use export::*;

const PREVIEW_JS: &str = include_str!("../assets/preview.js");
/// Frames per second of the preview's track; it interpolates in between.
const TRACK_FPS: f64 = 30.0;
/// Size the preview's track is laid out at (scaled to the window by CSS).
const TRACK_LONG_SIDE: u32 = 1920;
/// A zoom added by hand lasts this long, unless it runs into the next one.
const NEW_ZOOM_SECS: f64 = 2.5;

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
    Tick { t: f64, playing: bool },
    Key { key: String, cmd: bool, shift: bool },
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

fn modified(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

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
    let mut time = use_signal(|| 0.0f64);
    let loudness = use_signal({
        let dir = dir.clone();
        move || match std::fs::read(dir.join(take::LOUDNESS)).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(l) => Measure::Ready(Rc::new(l)),
            None => Measure::Unknown,
        }
    });
    let mut playing = use_signal(|| false);
    let mut selected = use_signal(|| Option::<usize>::None);
    // A clip picked on the timeline (start and end, in recording time); only one of a zoom and
    // a clip is selected at a time.
    let mut selected_clip = use_signal(|| Option::<(f64, f64)>::None);
    // A piece of music picked on the timeline; likewise exclusive.
    let mut selected_music = use_signal(|| Option::<usize>::None);

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
    // The background picture, if any, served next to the video: its file name and URL.
    let mut picture = use_signal(|| Option::<(String, String)>::None);
    use_effect({
        let dir = dir.clone();
        move || {
            let name = match &doc.project.read().style.background {
                Background::Image(name) => Some(name.clone()),
                _ => None,
            };
            let current = picture.peek().clone();
            if current.as_ref().map(|c| &c.0) != name.as_ref() {
                if let Some((_, url)) = current {
                    media_server::unpublish(&url);
                }
                picture.set(name.and_then(|n| media_server::share(&dir.join(&n)).map(|url| (n, url))));
            }
        }
    });
    // The songs, served next to the video by file name, and where each piece plays.
    let mut songs = use_signal(HashMap::<String, String>::new);
    use_effect({
        let dir = dir.clone();
        move || {
            let files: Vec<String> = doc.project.read().music.iter().map(|m| m.file.clone()).collect();
            for file in files {
                if !songs.peek().contains_key(&file) {
                    if let Some(url) = media_server::share(&dir.join(&file)) {
                        songs.write().insert(file, url);
                    }
                }
            }
        }
    });
    // Lowering the music under the voice needs the recording's loudness, and so does knowing
    // whether there's any sound to take noise out of: measured once, and again when Try again
    // puts it back to Unknown.
    use_effect({
        let dir = dir.clone();
        move || {
            let wanted = {
                let p = doc.project.read();
                p.duck.is_some() || p.denoise.is_some()
            };
            if wanted && *loudness.read() == Measure::Unknown {
                let (dir, mut loudness) = (dir.clone(), loudness);
                loudness.set(Measure::Measuring);
                spawn(async move {
                    loudness.set(match tokio::task::spawn_blocking(move || measure(&dir)).await {
                        Ok(Ok(Some(l))) => Measure::Ready(Rc::new(l)),
                        Ok(Ok(None)) => Measure::Silent,
                        Ok(Err(e)) => Measure::failed(&e),
                        Err(e) => Measure::Failed(e.to_string()),
                    });
                });
            }
        }
    });
    use_effect(move || {
        let project = doc.project.read();
        let urls = songs.read();
        let list: Vec<serde_json::Value> = project
            .placed_music()
            .into_iter()
            .filter_map(|m| {
                let url = urls.get(&project.music[m.index].file)?;
                let url = if in_app() { media_server::in_app_path(url) } else { url.clone() };
                let mut v = serde_json::to_value(m).ok()?;
                v["url"] = url.into();
                Some(v)
            })
            .collect();
        let duck = match (project.duck, &*loudness.read()) {
            (Some(level), Measure::Ready(l)) => serde_json::json!({
                "level": level,
                "ramp": music::DUCK_RAMP,
                "spans": project.speaking(l),
            }),
            _ => serde_json::Value::Null,
        };
        document::eval(&format!("window.sv && window.sv.setMusic({}, {duck})", serde_json::Value::from(list)));
    });
    use_asset_handler("media", move |request, responder| {
        std::thread::spawn(move || responder.respond(media_server::in_app_response(&request)));
    });

    // The frame track, recomputed whenever the project changes.
    let track = use_memo({
        let events = events.clone();
        move || {
            let scene = Scene::new(doc.project.read().clone(), &events);
            let (w, h) = scene.output_size(TRACK_LONG_SIDE);
            serde_json::to_string(&scene.track(TRACK_FPS, w, h)).unwrap_or_default()
        }
    });
    use_hook(|| {
        document::eval(PREVIEW_JS);
    });
    use_effect(move || {
        document::eval(&format!("window.sv && window.sv.setTrack({})", track()));
    });

    let add_zoom = use_callback({
        let events = events.clone();
        move |()| {
            let t = time();
            let (x, y) = events.cursor_at(t).unwrap_or((0.5, 0.5));
            let duration = doc.project.peek().duration;
            let z = Zoom { start: t, end: (t + NEW_ZOOM_SECS).min(duration), scale: 1.8, x, y };
            let mut added = None;
            doc.change(|p| added = zoom::insert(&mut p.zooms, z));
            match added {
                Some(i) => selected.set(Some(i)),
                None => {
                    let mut error = doc.error;
                    error.set(Some("There's no room for a zoom here: move the playhead to a gap.".into()));
                }
            }
        }
    });

    // Splits the recording at the playhead.
    let split_here = use_callback(move |()| {
        let t = time();
        let mut added = false;
        doc.change(|p| added = clips::split(&mut p.splits, p.duration, t));
        if !added {
            let mut error = doc.error;
            error.set(Some("Too close to the start, the end or another split to split here.".into()));
        }
    });
    // Leaves out everything before the playhead (`before`), or everything after it.
    let trim = use_callback(move |before: bool| {
        let t = time();
        doc.change(|p| {
            clips::split(&mut p.splits, p.duration, t);
            let stretch = if before { (0.0, t) } else { (t, p.duration) };
            if stretch.1 - stretch.0 > 1e-6 {
                clips::remove(&mut p.removed, stretch);
            }
        });
        selected_clip.set(None);
    });

    // A song chosen: it starts at the playhead (a piece playing there stops).
    let add_music = use_callback({
        let dir = dir.clone();
        move |from: PathBuf| {
            let mut error = doc.error;
            let file = match adopt_file(&dir, &from, "music") {
                Ok(file) => file,
                Err(e) => return error.set(Some(format!("Can't use that song: {e}"))),
            };
            let name = from.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| file.clone());
            let t = time();
            let m =
                Music { file, name, start: t, end: f64::INFINITY, from: 0.0, volume: music::VOLUME, fade: music::FADE };
            let mut added = None;
            doc.change(|p| added = music::insert(&mut p.music, m, p.duration));
            match added {
                Some(i) => {
                    selected.set(None);
                    selected_clip.set(None);
                    selected_music.set(Some(i));
                }
                None => error.set(Some("There's no room for music here: move the playhead.".into())),
            }
        }
    });
    // M: the music stops at the playhead if it's playing there; otherwise the last song before
    // it carries on from where it stopped.
    let toggle_music = use_callback(move |()| {
        let t = time();
        let (playing, last) = {
            let p = doc.project.peek();
            let last =
                p.music.iter().enumerate().rev().find(|(_, m)| m.end <= t).or(p.music.iter().enumerate().next_back());
            let placed = p.placed_music();
            let last = last.map(|(i, m)| {
                let played = placed.iter().find(|pl| pl.index == i).map_or(0.0, |pl| pl.until - pl.at);
                let from = if m.end <= t { m.from + played } else { 0.0 };
                Music { start: t, end: f64::INFINITY, from, ..m.clone() }
            });
            (music::at(&p.music, t), last)
        };
        let mut error = doc.error;
        if let Some(i) = playing {
            doc.change(|p| {
                let start = p.music[i].start;
                if t - start < music::MIN_SECS {
                    p.music.remove(i);
                } else {
                    music::set_bounds(&mut p.music, i, start, t, p.duration);
                }
            });
            selected_music.set(None);
        } else if let Some(m) = last {
            let mut added = None;
            doc.change(|p| added = music::insert(&mut p.music, m, p.duration));
            if added.is_none() {
                error.set(Some("There's no room for music here: move the playhead.".into()));
            }
        } else {
            error.set(Some("Choose a song with + Music first.".into()));
        }
    });

    // Ticks and keys from the preview script.
    use_future(move || async move {
        let mut channel = document::eval("window.sv.send = (m) => dioxus.send(m); await new Promise(() => {});");
        while let Ok(msg) = channel.recv::<Msg>().await {
            match msg {
                Msg::Tick { t, playing: p } => {
                    time.set(t);
                    playing.set(p);
                }
                Msg::Key { key, cmd, shift } => match (key.as_str(), cmd, shift) {
                    ("z", true, false) => doc.undo(),
                    ("z" | "Z", true, true) => doc.redo(),
                    ("z", false, false) => add_zoom(()),
                    ("s", false, false) => split_here(()),
                    ("m", false, false) => toggle_music(()),
                    ("[", false, _) => trim(true),
                    ("]", false, _) => trim(false),
                    ("Backspace" | "Delete", false, _) => {
                        if let Some(i) = selected.take() {
                            doc.change(|p| {
                                if i < p.zooms.len() {
                                    p.zooms.remove(i);
                                }
                            });
                        } else if let Some(i) = selected_music.take() {
                            doc.change(|p| {
                                if i < p.music.len() {
                                    p.music.remove(i);
                                }
                            });
                        } else if let Some(clip) = *selected_clip.peek() {
                            doc.change(|p| toggle_clip(p, clip));
                        }
                    }
                    ("Escape", false, _) => {
                        selected.set(None);
                        selected_clip.set(None);
                        selected_music.set(None);
                    }
                    _ => {}
                },
            }
        }
    });

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
                    // A song starts at the playhead; M then stops and restarts it as the video plays.
                    label { class: "button", title: "A song from the playhead. M stops it or starts it again there",
                        "+ Music"
                        input {
                            r#type: "file",
                            accept: "audio/*,.mp3,.m4a,.wav,.aac,.flac",
                            hidden: true,
                            onchange: move |e| {
                                if let Some(file) = e.files().into_iter().next() {
                                    add_music(file.path());
                                }
                            },
                        }
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
                                let again = *selected_music.peek() == Some(i);
                                selected_music.set(if again { None } else { Some(i) });
                            },
                            "♪ {m.name}"
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
                if let Some(i) = selection {
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
                ExportPanel { doc, dir: dir.clone() }
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
