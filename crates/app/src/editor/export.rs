//! Exporting a take.

use super::*;

/// The name suggested for an export: the take's, then what tells this export apart from the
/// take's others (size, frame rate, shape, speed, whether the blanks are cut), then when it was
/// made, so a new export never replaces an earlier one.
pub(crate) fn export_name(
    take: &str,
    settings: Settings,
    aspect: Aspect,
    speed: f64,
    blanks_cut: bool,
    at: chrono::NaiveDateTime,
) -> String {
    let size = match settings.format {
        Format::Mp4 => RESOLUTIONS
            .iter()
            .find(|(px, _)| *px == settings.long_side)
            .map_or_else(|| format!("{}px", settings.long_side), |(_, name)| name.to_string()),
        Format::Gif => format!("{}px", settings.long_side),
    };
    let mut parts = vec![take.to_string(), size, format!("{}fps", settings.fps)];
    // ":" can't be in a file name on Windows, and shows as "/" in Finder.
    match aspect {
        Aspect::Source => {}
        Aspect::Landscape16x9 => parts.push("16x9".into()),
        Aspect::Portrait9x16 => parts.push("9x16".into()),
        Aspect::Square => parts.push("1x1".into()),
    }
    parts.push(format!("{speed}x"));
    parts.push(if blanks_cut { "blanks cut" } else { "with blanks" }.into());
    if settings.mute {
        parts.push("muted".into());
    }
    format!("{} (exported {})", parts.join(" "), at.format("%Y-%m-%d %H.%M.%S"))
}

/// The videos exported from the take in `dir` that still carry its name: the one named exactly
/// after it (as exports were before they were dated) and those named as `export_name` suggests.
pub(crate) fn exports_of(dir: &std::path::Path) -> Vec<PathBuf> {
    let (Some(take), Some(parent)) = (dir.file_name().map(|n| n.to_string_lossy().into_owned()), dir.parent()) else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(parent) else { return Vec::new() };
    let mut found: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let ext = p.extension().and_then(|e| e.to_str());
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            matches!(ext, Some("mp4" | "gif"))
                && p.is_file()
                && (stem == take || stem.strip_prefix(&take).is_some_and(|rest| rest.starts_with(' ')))
        })
        .collect();
    found.sort();
    found
}

#[component]
pub(super) fn ExportPanel(doc: Doc, dir: PathBuf) -> Element {
    let mut settings = use_signal(Settings::default);
    let mut state = use_signal(|| Export::Idle);
    // The name being chosen for the next export, while its dialog is open.
    let mut naming = use_signal(|| None::<String>);
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
                        Some((Err(e), _)) if ffmpeg_help::is_missing(&e) => {
                            state.set(Export::NoFfmpeg(format!("{e:#}")))
                        }
                        Some((Err(e), _)) => state.set(Export::Failed(format!("{e:#}"))),
                        None => {}
                    }
                }
            }
        }
    });

    // Asks for the file's name, suggesting one.
    let start = {
        let take = take.clone();
        move |_| {
            // Without Pro, a take not exported before needs a free export left.
            if !pro.may_export(&take) {
                pro.ask(Some(take.clone()));
                return;
            }
            let (aspect, speed, blanks_cut) = {
                let p = doc.project.peek();
                (p.style.aspect, p.speed, p.blanks.is_some())
            };
            let now = chrono::Local::now().naive_local();
            naming.set(Some(export_name(&take, settings(), aspect, speed, blanks_cut, now)));
        }
    };

    let export = {
        let dir = dir.clone();
        move |name: String| {
            naming.set(None);
            let (progress, cancel) = (Arc::new(Progress::default()), Arc::new(AtomicBool::new(false)));
            let (tx, done) = mpsc::channel();
            let settings = settings();
            let (dir, out) = (dir.clone(), export_path(&dir, &name, settings.format));
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
        if let Some(suggested) = naming() {
            ExportDialog {
                dir: dir.clone(),
                suggested,
                format: current.format,
                on_close: move |_| naming.set(None),
                on_confirm: export,
            }
        }
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
                                    Format::Gif => Settings { format, long_side: 960, fps: 15, mute: false },
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
                Export::NoFfmpeg(e) => rsx! {
                    p { class: "error", "{e}" }
                    FfmpegHelp {}
                    button { class: "primary", onclick: start.clone(), "Try again" }
                },
                Export::Idle => {
                    let limited = !pro.status.read().is_pro();
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
                                    "Free exports used up \u{2014} PolishCast Pro exports every take"
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

/// Asks what to call the export, next to the take's folder: Enter exports, Esc cancels.
#[component]
pub(super) fn ExportDialog(
    dir: PathBuf,
    suggested: String,
    format: Format,
    on_close: EventHandler<()>,
    on_confirm: EventHandler<String>,
) -> Element {
    let mut name = use_signal(|| suggested.clone());
    // Export was pressed on a name already taken: asks before replacing that file.
    let mut asking = use_signal(|| false);
    let cleaned = clean_export_name(&name(), format);
    let replaces = cleaned.as_deref().is_some_and(|n| export_path(&dir, n, format).exists());
    let ext = format.extension();
    let confirm = {
        let cleaned = cleaned.clone();
        move || {
            if let Some(n) = cleaned.clone() {
                if replaces && !asking() {
                    let mut asking = asking;
                    asking.set(true);
                } else {
                    on_confirm.call(n);
                }
            }
        }
    };
    let file = cleaned.as_deref().map(|n| format!("{n}.{ext}")).unwrap_or_default();
    if asking() {
        return rsx! {
            div { class: "modal-backdrop", onclick: move |_| asking.set(false),
                div {
                    class: "modal",
                    tabindex: "0",
                    onclick: move |e| e.stop_propagation(),
                    onmounted: move |e| async move {
                        let _ = e.set_focus(true).await;
                    },
                    onkeydown: {
                        let confirm = confirm.clone();
                        move |e: KeyboardEvent| {
                            e.stop_propagation();
                            match e.key() {
                                Key::Enter => confirm(),
                                Key::Escape => asking.set(false),
                                _ => {}
                            }
                        }
                    },
                    div { class: "modal-head",
                        h2 { "Replace the existing video?" }
                        button { class: "modal-close", title: "Close", onclick: move |_| asking.set(false), "×" }
                    }
                    div { class: "modal-body",
                        p { class: "delete-name", "{file}" }
                        p { class: "hint", "A video with this name is already next to the take's folder. Replacing it can't be undone." }
                        div { class: "modal-actions",
                            button { onclick: move |_| asking.set(false), "Rename" kbd { "Esc" } }
                            button { class: "danger", onclick: move |_| confirm(), "Replace" kbd { "Enter" } }
                        }
                    }
                }
            }
        };
    }
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| on_close.call(()),
            div { class: "modal", onclick: move |e| e.stop_propagation(),
                div { class: "modal-head",
                    h2 { "Export as" }
                    button { class: "modal-close", title: "Close", onclick: move |_| on_close.call(()), "×" }
                }
                div { class: "modal-body",
                    div { class: "export-name",
                        input {
                            r#type: "text",
                            value: "{name}",
                            spellcheck: "false",
                            oninput: move |e| name.set(e.value()),
                            onmounted: move |e| async move {
                                let _ = e.set_focus(true).await;
                            },
                            onkeydown: {
                                let confirm = confirm.clone();
                                move |e: KeyboardEvent| {
                                    e.stop_propagation();
                                    match e.key() {
                                        Key::Enter => confirm(),
                                        Key::Escape => on_close.call(()),
                                        _ => {}
                                    }
                                }
                            },
                        }
                        span { ".{ext}" }
                    }
                    if replaces {
                        p { class: "error", "A video with this name is already there. Export asks before replacing it." }
                    } else {
                        p { class: "hint", "Saved next to the take's folder." }
                    }
                    div { class: "modal-actions",
                        button { onclick: move |_| on_close.call(()), "Cancel" kbd { "Esc" } }
                        button { class: "primary", disabled: cleaned.is_none(), onclick: move |_| confirm(), "Export" kbd { "Enter" } }
                    }
                }
            }
        }
    }
}
