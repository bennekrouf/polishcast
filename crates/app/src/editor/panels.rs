//! The editor's side panels: style, zoom, music, clips and blanks.

use super::*;

/// A labelled slider. `oninput` changes the project live; releasing it settles the edit into
/// one undo step and saves.
#[component]
pub(super) fn Slider(
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
pub(super) fn StylePanel(doc: Doc, dir: PathBuf, picture_url: Option<String>) -> Element {
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
                            match adopt_file(&dir, &file.path(), "background") {
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
            "Click a zoom or a song on the timeline to edit it. Z adds a zoom at the playhead, S splits there and [ or ] trims before or after it, M stops or starts the music there; Space plays; ←/→ step."
        }
    }
}

#[component]
pub(super) fn ZoomPanel(doc: Doc, index: usize, selected: Signal<Option<usize>>) -> Element {
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

#[component]
pub(super) fn MusicPanel(
    doc: Doc,
    dir: PathBuf,
    index: usize,
    selected_music: Signal<Option<usize>>,
    loudness: Signal<Measure>,
) -> Element {
    let p = doc.project.read();
    let duck = p.duck;
    let m = p.music[index].clone();
    let duration = p.duration;
    let plays = p.placed_music().into_iter().find(|pl| pl.index == index).map(|pl| pl.until - pl.at);
    let bounds = move |start: f64, end: f64| doc.live(|p| music::set_bounds(&mut p.music, index, start, end, duration));
    rsx! {
        h2 { "Music {index + 1}" }
        p { class: "hint",
            "♪ {m.name} · "
            match plays {
                Some(secs) => rsx! { "plays for {secs:.1} s of the video" },
                None => rsx! { "all of it is cut from the video" },
            }
        }
        div {
            Slider {
                edits: doc,
                label: "Starts at",
                value: m.start,
                min: 0.0,
                max: duration,
                step: 0.05,
                show: clock(m.start),
                oninput: move |v| bounds(v, m.end),
            }
            Slider {
                edits: doc,
                label: "Stops at",
                value: m.end,
                min: 0.0,
                max: duration,
                step: 0.05,
                show: clock(m.end),
                oninput: move |v| bounds(m.start, v),
            }
            Slider {
                edits: doc,
                label: "From the song's",
                value: m.from,
                min: 0.0,
                max: 600.0_f64.max(m.from),
                step: 0.5,
                show: clock(m.from),
                oninput: move |v: f64| doc.live(|p| p.music[index].from = v),
            }
            Slider {
                edits: doc,
                label: "Volume",
                value: m.volume as f64,
                min: 0.0,
                max: 1.0,
                step: 0.01,
                show: format!("{:.0}%", m.volume * 100.0),
                oninput: move |v: f64| doc.live(|p| p.music[index].volume = v as f32),
            }
            Slider {
                edits: doc,
                label: "Fade in and out",
                value: m.fade,
                min: 0.0,
                max: 5.0,
                step: 0.1,
                show: format!("{:.1} s", m.fade),
                oninput: move |v: f64| doc.live(|p| p.music[index].fade = v),
            }
        }
        label { class: "check", title: "For all the music in this video",
            input {
                r#type: "checkbox",
                checked: duck.is_some(),
                onchange: move |e| doc.change(|p| p.duck = e.checked().then_some(music::DUCK_LEVEL)),
            }
            "Lower the music while I speak"
        }
        if let Some(level) = duck {
            Slider {
                edits: doc,
                label: "Under the voice",
                value: level as f64,
                min: 0.05,
                max: 0.8,
                step: 0.05,
                show: format!("{:.0}%", level * 100.0),
                oninput: move |v: f64| doc.live(|p| p.duck = Some(v as f32)),
            }
            match loudness() {
                Measure::Measuring => rsx! { p { class: "hint", "Listening for your voice…" } },
                Measure::Silent => rsx! { p { class: "hint", "This recording has no sound, so there's no voice to make room for." } },
                Measure::Failed(e) => rsx! { p { class: "error", "{e}" } },
                Measure::NoFfmpeg(e) => rsx! {
                    p { class: "error", "{e}" }
                    FfmpegHelp {}
                    button { onclick: move |_| loudness.set(Measure::Unknown), "Try again" }
                },
                _ => rsx! {},
            }
        }
        div { class: "buttons",
            button {
                onclick: move |_| {
                    document::eval(&format!("window.sv.seek({})", m.start));
                },
                "Go to start"
            }
            label { class: "button", title: "Another song, playing at the same moments",
                "Change song"
                input {
                    r#type: "file",
                    accept: "audio/*,.mp3,.m4a,.wav,.aac,.flac",
                    hidden: true,
                    onchange: move |e| {
                        let Some(file) = e.files().into_iter().next() else { return };
                        let from = file.path();
                        match adopt_file(&dir, &from, "music") {
                            Ok(name) => doc.change(|p| {
                                let m = &mut p.music[index];
                                m.file = name;
                                m.name = from.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                m.from = 0.0;
                            }),
                            Err(err) => {
                                let mut error = doc.error;
                                error.set(Some(format!("Can't use that song: {err}")));
                            }
                        }
                    },
                }
            }
            button {
                class: "danger",
                title: "⌫",
                onclick: move |_| {
                    selected_music.set(None);
                    doc.change(|p| {
                        p.music.remove(index);
                    });
                },
                "Delete"
            }
            button { onclick: move |_| selected_music.set(None), "Done" }
        }
        p { class: "hint",
            "The music starts and stops at these moments of the recording, wherever cuts and the speed put them in the video, and plays at its own speed in between."
        }
    }
}

#[component]
pub(super) fn ClipPanel(doc: Doc, clip: (f64, f64), selected_clip: Signal<Option<(f64, f64)>>) -> Element {
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

#[component]
pub(super) fn BlanksPanel(doc: Doc, dir: PathBuf, loudness: Signal<Measure>) -> Element {
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
                Ok(Err(e)) => loudness.set(Measure::failed(&e)),
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
        Measure::Unknown | Measure::Failed(_) | Measure::NoFfmpeg(_) => start_measuring(dir, true),
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
                Measure::NoFfmpeg(e) => {
                    let dir = dir.clone();
                    rsx! {
                        p { class: "error", "{e}" }
                        FfmpegHelp {}
                        button { onclick: move |_| start_measuring(dir.clone(), true), "Try again" }
                    }
                }
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
