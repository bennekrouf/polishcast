//! The editor: preview, transport, timeline of zooms, and the style or zoom inspector.
//!
//! The preview is drawn by `assets/preview.js` from the frame track `render` computes; this
//! component owns the project, its undo history and saving.

use crate::{media_server, platform};
use dioxus::desktop::use_asset_handler;
use dioxus::prelude::*;
use polishcast_core::blanks::redetect;
use polishcast_core::clips;
use polishcast_core::history::History;
use polishcast_core::project::{Aspect, Background, SPEEDS};
use polishcast_core::zoom::{self, Zoom};
use polishcast_core::{take, BlankParams, EventLog, Loudness, Project};
use polishcast_export::{Format, Progress, Settings};
use polishcast_render::{compose, Scene};
use serde::Deserialize;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

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
    };
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
                    ("[", false, _) => trim(true),
                    ("]", false, _) => trim(false),
                    ("Backspace" | "Delete", false, _) => {
                        if let Some(i) = selected.take() {
                            doc.change(|p| {
                                if i < p.zooms.len() {
                                    p.zooms.remove(i);
                                }
                            });
                        } else if let Some(clip) = *selected_clip.peek() {
                            doc.change(|p| toggle_clip(p, clip));
                        }
                    }
                    ("Escape", false, _) => {
                        selected.set(None);
                        selected_clip.set(None);
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
                                let again = *selected.peek() == Some(i);
                                selected.set(if again { None } else { Some(i) });
                            },
                            "{z.scale:.1}×"
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
                } else {
                    StylePanel { doc, dir: dir.clone(), picture_url: picture_url.clone() }
                }
                BlanksPanel { doc, dir: dir.clone(), loudness }
                ExportPanel { dir: dir.clone() }
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

/// Copies a picture chosen as the background into the take's folder, under a new name each
/// time (so undo brings back the picture that was there), and returns that name.
fn adopt_picture(dir: &std::path::Path, from: &std::path::Path) -> std::io::Result<String> {
    let ext = from.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let name = format!("background-{stamp}.{ext}");
    std::fs::copy(from, dir.join(&name))?;
    Ok(name)
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A labelled slider. `oninput` changes the project live; releasing it settles the edit into
/// one undo step and saves.
#[component]
fn Slider(
    edits: Doc,
    label: String,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    show: String,
    oninput: EventHandler<f64>,
) -> Element {
    rsx! {
        label { class: "field",
            span { class: "row",
                span { "{label}" }
                span { class: "value", "{show}" }
            }
            input {
                r#type: "range",
                min: "{min}",
                max: "{max}",
                step: "{step}",
                value: "{value}",
                oninput: move |e| {
                    if let Ok(v) = e.value().parse() {
                        oninput.call(v);
                    }
                },
                onchange: move |_| edits.settle(),
            }
        }
    }
}

#[component]
fn StylePanel(doc: Doc, dir: PathBuf, picture_url: Option<String>) -> Element {
    let p = doc.project.read();
    let s = &p.style;
    let has_picture = matches!(s.background, Background::Image(_));
    rsx! {
        h2 { "Style" }
        div { class: "field",
            span { "Frame" }
            div { class: "segmented",
                for &(aspect, name) in ASPECTS {
                    button {
                        class: if s.aspect == aspect { "on" } else { "" },
                        onclick: move |_| doc.change(|p| p.style.aspect = aspect),
                        "{name}"
                    }
                }
            }
        }
        div { class: "field",
            span { "Background" }
            div { class: "swatches",
                for &(a, b) in BACKGROUNDS {
                    button {
                        class: if s.background == Background::Gradient(a.into(), b.into()) { "swatch on" } else { "swatch" },
                        background: "linear-gradient(to bottom, {a}, {b})",
                        onclick: move |_| doc.change(|p| p.style.background = Background::Gradient(a.into(), b.into())),
                    }
                }
                for &c in COLORS {
                    button {
                        class: if s.background == Background::Color(c.into()) { "swatch on" } else { "swatch" },
                        background: "{c}",
                        onclick: move |_| doc.change(|p| p.style.background = Background::Color(c.into())),
                    }
                }
                // A picture of your own: the swatch shows it once chosen.
                label {
                    class: if has_picture { "swatch picture on" } else { "swatch picture" },
                    title: "A picture of your own (PNG or JPEG)",
                    background: if has_picture { css_background(&s.background, picture_url.as_deref()) } else { String::new() },
                    if !has_picture { "+" }
                    input {
                        r#type: "file",
                        accept: "image/png,image/jpeg",
                        onchange: move |e| {
                            let Some(file) = e.files().into_iter().next() else { return };
                            match adopt_picture(&dir, &file.path()) {
                                Ok(name) => doc.change(|p| p.style.background = Background::Image(name)),
                                Err(err) => {
                                    let mut error = doc.error;
                                    error.set(Some(format!("Can't use that picture: {err}")));
                                }
                            }
                        },
                    }
                }
            }
        }
        div {
            Slider {
                edits: doc,
                label: "Padding",
                value: s.padding as f64,
                min: 0.0,
                max: 0.2,
                step: 0.005,
                show: format!("{:.0}%", s.padding * 100.0),
                oninput: move |v: f64| doc.live(|p| p.style.padding = v as f32),
            }
            Slider {
                edits: doc,
                label: "Corners",
                value: s.radius as f64,
                min: 0.0,
                max: 0.06,
                step: 0.002,
                show: format!("{:.1}%", s.radius * 100.0),
                oninput: move |v: f64| doc.live(|p| p.style.radius = v as f32),
            }
            Slider {
                edits: doc,
                label: "Cursor size",
                value: s.cursor_scale as f64,
                min: 0.5,
                max: 3.0,
                step: 0.1,
                show: format!("{:.1}×", s.cursor_scale),
                oninput: move |v: f64| doc.live(|p| p.style.cursor_scale = v as f32),
            }
            Slider {
                edits: doc,
                label: "Cursor smoothing",
                value: s.cursor_smoothing,
                min: 0.0,
                max: 0.3,
                step: 0.01,
                show: format!("{:.2} s", s.cursor_smoothing),
                oninput: move |v: f64| doc.live(|p| p.style.cursor_smoothing = v),
            }
        }
        label { class: "check",
            input {
                r#type: "checkbox",
                checked: s.shadow,
                onchange: move |e| doc.change(|p| p.style.shadow = e.checked()),
            }
            "Shadow"
        }
        label { class: "check",
            input {
                r#type: "checkbox",
                checked: s.click_effect,
                onchange: move |e| doc.change(|p| p.style.click_effect = e.checked()),
            }
            "Ring on clicks"
        }
        label { class: "check",
            input {
                r#type: "checkbox",
                checked: s.hide_idle_cursor,
                onchange: move |e| doc.change(|p| p.style.hide_idle_cursor = e.checked()),
            }
            "Hide the cursor when it rests"
        }
        p { class: "hint",
            "Click a zoom on the timeline to edit it. Z adds one at the playhead, S splits there and [ or ] trims before or after it; Space plays; ←/→ step."
        }
    }
}

#[component]
fn ZoomPanel(doc: Doc, index: usize, selected: Signal<Option<usize>>) -> Element {
    let p = doc.project.read();
    let z = p.zooms[index];
    let duration = p.duration;
    let bounds = move |start: f64, end: f64| doc.live(|p| zoom::set_bounds(&mut p.zooms, index, start, end, duration));
    rsx! {
        h2 { "Zoom {index + 1}" }
        div {
            Slider {
                edits: doc,
                label: "Start",
                value: z.start,
                min: 0.0,
                max: duration,
                step: 0.05,
                show: format!("{:.2} s", z.start),
                oninput: move |v| bounds(v, z.end),
            }
            Slider {
                edits: doc,
                label: "End",
                value: z.end,
                min: 0.0,
                max: duration,
                step: 0.05,
                show: format!("{:.2} s", z.end),
                oninput: move |v| bounds(z.start, v),
            }
            Slider {
                edits: doc,
                label: "Zoom",
                value: z.scale as f64,
                min: 1.1,
                max: 4.0,
                step: 0.1,
                show: format!("{:.1}×", z.scale),
                oninput: move |v: f64| doc.live(|p| p.zooms[index].scale = v as f32),
            }
            Slider {
                edits: doc,
                label: "Focus left–right",
                value: z.x as f64,
                min: 0.0,
                max: 1.0,
                step: 0.01,
                show: format!("{:.0}%", z.x * 100.0),
                oninput: move |v: f64| doc.live(|p| p.zooms[index].x = v as f32),
            }
            Slider {
                edits: doc,
                label: "Focus top–bottom",
                value: z.y as f64,
                min: 0.0,
                max: 1.0,
                step: 0.01,
                show: format!("{:.0}%", z.y * 100.0),
                oninput: move |v: f64| doc.live(|p| p.zooms[index].y = v as f32),
            }
        }
        div { class: "buttons",
            button {
                onclick: move |_| {
                    document::eval(&format!("window.sv.seek({})", z.start));
                },
                "Go to start"
            }
            button {
                class: "danger",
                onclick: move |_| {
                    selected.set(None);
                    doc.change(|p| {
                        p.zooms.remove(index);
                    });
                },
                "Delete"
            }
            button { onclick: move |_| selected.set(None), "Done" }
        }
    }
}

/// Removes a clip from the video, or puts it back.
fn toggle_clip(p: &mut Project, clip: (f64, f64)) {
    if clips::is_removed(&p.removed, clip) {
        clips::restore(&mut p.removed, clip);
    } else {
        clips::remove(&mut p.removed, clip);
    }
}

#[component]
fn ClipPanel(doc: Doc, clip: (f64, f64), selected_clip: Signal<Option<(f64, f64)>>) -> Element {
    let p = doc.project.read();
    let (start, end) = clip;
    let removed = clips::is_removed(&p.removed, clip);
    let number = clips::clips(&p.splits, p.duration).iter().position(|&c| c == clip).map_or(1, |i| i + 1);
    rsx! {
        h2 { "Clip {number}" }
        p { class: "hint",
            "{clock(start)} – {clock(end)} · {end - start:.1} s"
            if removed { " · left out of the video" }
        }
        div { class: "buttons",
            button {
                class: if removed { "primary" } else { "danger" },
                title: "⌫",
                onclick: move |_| doc.change(|p| toggle_clip(p, clip)),
                if removed { "Put back" } else { "Remove" }
            }
            button {
                onclick: move |_| {
                    document::eval(&format!("window.sv.seek({start})"));
                },
                "Go to start"
            }
            button { onclick: move |_| selected_clip.set(None), "Done" }
        }
        p { class: "hint",
            "S splits at the playhead. [ leaves out everything before it, ] everything after. Undo puts things back."
        }
    }
}

const RESOLUTIONS: &[(u32, &str)] = &[(1920, "1080p"), (2560, "1440p"), (3840, "4K")];
const FRAME_RATES: &[u32] = &[30, 60];
/// GIFs grow fast with size and frame rate; these keep them small enough to paste anywhere.
const GIF_SIZES: &[(u32, &str)] = &[(640, "640 px"), (960, "960 px"), (1280, "1280 px")];
const GIF_FRAME_RATES: &[u32] = &[10, 15, 24];

enum Export {
    Idle,
    Running { progress: Arc<Progress>, cancel: Arc<AtomicBool>, done: mpsc::Receiver<anyhow::Result<()>>, out: PathBuf },
    Done(PathBuf),
    Failed(String),
}

/// Where a take's export goes: next to its folder, named after it.
fn export_path(dir: &std::path::Path, format: Format) -> PathBuf {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Polishcast".into());
    dir.with_file_name(format!("{name}.{}", format.extension()))
}

#[component]
fn ExportPanel(dir: PathBuf) -> Element {
    let mut settings = use_signal(Settings::default);
    let mut state = use_signal(|| Export::Idle);
    let mut fraction = use_signal(|| 0.0f32);
    // ffmpeg is being fetched before the first export.
    let mut downloading = use_signal(|| false);
    // The take's name: what the free version's exports are counted by.
    let take = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let pro = use_context::<crate::licence::Pro>();

    // Progress and the result, while an export runs.
    use_future({
        let take = take.clone();
        move || {
            let take = take.clone();
            async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let finished = match &*state.peek() {
                        Export::Running { progress, done, out, .. } => {
                            fraction.set(progress.fraction());
                            downloading.set(progress.downloading());
                            done.try_recv().ok().map(|r| (r, out.clone()))
                        }
                        _ => None,
                    };
                    match finished {
                        Some((Ok(()), out)) => {
                            pro.exported(&take);
                            state.set(Export::Done(out));
                        }
                        Some((Err(e), _)) if e.is::<polishcast_export::Cancelled>() => state.set(Export::Idle),
                        Some((Err(e), _)) => state.set(Export::Failed(format!("{e:#}"))),
                        None => {}
                    }
                }
            }
        }
    });

    let start = {
        let (dir, take) = (dir.clone(), take.clone());
        move |_| {
            // Without Pro, a take not exported before needs a free export left.
            if !pro.may_export(&take) {
                let mut open = pro.open;
                open.set(Some(Some(take.clone())));
                return;
            }
            let (progress, cancel) = (Arc::new(Progress::default()), Arc::new(AtomicBool::new(false)));
            let (tx, done) = mpsc::channel();
            let settings = settings();
            let (dir, out) = (dir.clone(), export_path(&dir, settings.format));
            let (p, c, target) = (progress.clone(), cancel.clone(), out.clone());
            std::thread::spawn(move || {
                let _ = tx.send(polishcast_export::export(&dir, &target, settings, &p, &c));
            });
            fraction.set(0.0);
            state.set(Export::Running { progress, cancel, done, out });
        }
    };

    let current = settings();
    let running = matches!(*state.read(), Export::Running { .. });
    let gif = current.format == Format::Gif;
    let (sizes, rates) = if gif { (GIF_SIZES, GIF_FRAME_RATES) } else { (RESOLUTIONS, FRAME_RATES) };
    let kind = if gif { "GIF" } else { "MP4" };
    rsx! {
        div { class: "export",
            h2 { "Export" }
            div { class: "segmented",
                for (format, name) in [(Format::Mp4, "MP4"), (Format::Gif, "GIF")] {
                    button {
                        class: if current.format == format { "on" } else { "" },
                        disabled: running,
                        onclick: move |_| {
                            if settings.peek().format != format {
                                // Each format starts from its own sensible size and rate.
                                settings.set(match format {
                                    Format::Mp4 => Settings::default(),
                                    Format::Gif => Settings { format, long_side: 960, fps: 15 },
                                });
                                state.set(Export::Idle);
                            }
                        },
                        "{name}"
                    }
                }
            }
            div { class: "segmented",
                for &(long_side, name) in sizes {
                    button {
                        class: if current.long_side == long_side { "on" } else { "" },
                        disabled: running,
                        onclick: move |_| settings.write().long_side = long_side,
                        "{name}"
                    }
                }
            }
            div { class: "segmented",
                for &fps in rates {
                    button {
                        class: if current.fps == fps { "on" } else { "" },
                        disabled: running,
                        onclick: move |_| settings.write().fps = fps,
                        "{fps} fps"
                    }
                }
            }
            match &*state.read() {
                Export::Running { cancel, .. } => {
                    let cancel = cancel.clone();
                    rsx! {
                        div { class: "progress",
                            div { class: "bar", width: "{fraction() * 100.0}%" }
                        }
                        div { class: "buttons",
                            span { class: "hint",
                                if downloading() {
                                    "Getting ffmpeg (first export only)… {fraction() * 100.0:.0}%"
                                } else {
                                    "{fraction() * 100.0:.0}%"
                                }
                            }
                            button { onclick: move |_| cancel.store(true, Ordering::Relaxed), "Cancel" }
                        }
                    }
                }
                Export::Done(path) => {
                    let path = path.clone();
                    rsx! {
                        p { class: "hint", "Saved {path.file_name().unwrap_or_default().to_string_lossy()}" }
                        div { class: "buttons",
                            button { onclick: move |_| platform::reveal(&path), {platform::SHOW_IN_FOLDER} }
                            button { class: "primary", onclick: start.clone(), "Export again" }
                        }
                    }
                }
                Export::Failed(e) => rsx! {
                    p { class: "error", "{e}" }
                    button { class: "primary", onclick: start.clone(), "Try again" }
                },
                Export::Idle => {
                    let limited = !pro.status.read().unlimited();
                    let exported = pro.exported.read();
                    let allowed = !limited || exported.allows(&take);
                    let left = exported.left();
                    rsx! {
                        button { class: "primary wide", onclick: start.clone(),
                            if allowed { "Export {kind}" } else { "Export {kind} \u{1f512}" }
                        }
                        if gif {
                            p { class: "hint", "No sound, loops forever. Best for short clips." }
                        }
                        if limited {
                            p { class: "hint",
                                if !allowed {
                                    "Free exports used up \u{2014} Polishcast Pro exports every take"
                                } else if exported.takes.contains(&take) {
                                    "Exporting this take again is free"
                                } else {
                                    "{left} free export(s) left"
                                }
                            }
                        }
                    }
                },
            }
        }
    }
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
}

fn measure(dir: &std::path::Path) -> anyhow::Result<Option<Loudness>> {
    let ffmpeg = polishcast_export::ensure_ffmpeg(&mut |_| {}, &|| false)?;
    let loudness = polishcast_export::sound::loudness(&ffmpeg, &dir.join(take::SCREEN))?;
    if let Some(l) = &loudness {
        std::fs::write(dir.join(take::LOUDNESS), serde_json::to_vec(l)?)?;
    }
    Ok(loudness)
}

/// Turns blank cutting on with `params`, finding the cuts again (cuts the user kept stay kept).
fn cut_blanks(p: &mut Project, loudness: &Loudness, params: BlankParams) {
    p.cuts = redetect(&p.cuts, loudness, p.duration, &params);
    p.blanks = Some(params);
}

#[component]
fn BlanksPanel(doc: Doc, dir: PathBuf, loudness: Signal<Measure>) -> Element {
    let p = doc.project.read();
    let params = p.blanks;
    let measure_state = loudness();

    // Measures the loudness in the background; then, if `enable`, or if cutting is on but no
    // cuts were found yet, finds the cuts.
    let mut start_measuring = move |dir: PathBuf, enable: bool| {
        loudness.set(Measure::Measuring);
        spawn(async move {
            match tokio::task::spawn_blocking(move || measure(&dir)).await {
                Ok(Ok(Some(l))) => {
                    let l = Rc::new(l);
                    loudness.set(Measure::Ready(l.clone()));
                    let (on, empty) = {
                        let p = doc.project.peek();
                        (p.blanks.is_some(), p.cuts.is_empty())
                    };
                    if enable || (on && empty) {
                        doc.change(|p| cut_blanks(p, &l, p.blanks.unwrap_or_default()));
                    }
                }
                Ok(Ok(None)) => loudness.set(Measure::Silent),
                Ok(Err(e)) => loudness.set(Measure::Failed(format!("{e:#}"))),
                Err(e) => loudness.set(Measure::Failed(e.to_string())),
            }
        });
    };
    // Cutting is on but the loudness wasn't kept (or was deleted): measure it again.
    use_hook({
        let dir = dir.clone();
        let mut start = start_measuring;
        move || {
            if params.is_some() && *loudness.peek() == Measure::Unknown {
                start(dir, false);
            }
        }
    });
    let mut turn_on = move |dir: PathBuf| match loudness() {
        Measure::Ready(l) => doc.change(|p| cut_blanks(p, &l, p.blanks.unwrap_or_default())),
        Measure::Measuring | Measure::Silent => {}
        Measure::Unknown | Measure::Failed(_) => start_measuring(dir, true),
    };

    // A slider moved: find the cuts again with the new settings (one undo step per drag).
    let tune = move |change: &dyn Fn(&mut BlankParams)| {
        if let Measure::Ready(l) = loudness() {
            doc.live(|p| {
                let mut params = p.blanks.unwrap_or_default();
                change(&mut params);
                cut_blanks(p, &l, params);
            });
        }
    };
    let cuts = p.cuts.iter().filter(|c| !c.keep).count();
    let before = clock(p.duration);
    let after = clock(p.timeline().duration());

    rsx! {
        div { class: "blanks",
            h2 { "Blanks" }
            label { class: "check",
                input {
                    r#type: "checkbox",
                    checked: params.is_some(),
                    disabled: matches!(measure_state, Measure::Measuring | Measure::Silent),
                    onchange: move |e| {
                        if e.checked() {
                            turn_on(dir.clone());
                        } else {
                            doc.change(|p| p.blanks = None);
                        }
                    },
                }
                "Cut the pauses"
            }
            match &measure_state {
                Measure::Measuring => rsx! { p { class: "hint", "Listening to the recording…" } },
                Measure::Silent => rsx! { p { class: "hint", "This recording has no sound, so there are no pauses to find." } },
                Measure::Failed(e) => rsx! { p { class: "error", "{e}" } },
                _ => rsx! {},
            }
            if let Some(b) = params {
                Slider {
                    edits: doc,
                    label: "Silence below",
                    value: b.threshold_db as f64,
                    min: -60.0,
                    max: -25.0,
                    step: 1.0,
                    show: format!("{:.0} dB", b.threshold_db),
                    oninput: move |v: f64| tune(&|b: &mut BlankParams| b.threshold_db = v as f32),
                }
                Slider {
                    edits: doc,
                    label: "Pauses longer than",
                    value: b.min_pause_secs,
                    min: 0.3,
                    max: 3.0,
                    step: 0.1,
                    show: format!("{:.1} s", b.min_pause_secs),
                    oninput: move |v: f64| tune(&|b: &mut BlankParams| b.min_pause_secs = v),
                }
                p { class: "hint",
                    if cuts == 1 { "1 cut" } else { "{cuts} cuts" }
                    " · {before} → {after}. The hatched stretches on the timeline are cut; click one to keep it."
                }
            }
        }
    }
}
