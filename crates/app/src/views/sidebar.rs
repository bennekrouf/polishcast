//! The recordings panel: what to record, the hints and errors, and the takes.

use super::tray::TRAY_PLACE;
use super::*;

pub(super) fn sidebar(
    rec: recording::Rec,
    mut sidebar_open: Signal<bool>,
    mut view: Signal<View>,
    mut confirm_delete: Signal<Option<PathBuf>>,
    shortcut_ok: bool,
) -> Element {
    let recording::Rec {
        status,
        error,
        library,
        mut selected,
        compacting,
        notice,
        settings,
        auto_stopped,
        picking,
        windows,
        chosen_window,
        ..
    } = rec;
    rsx! {
        nav {
            class: "sidebar",
            hidden: !sidebar_open(),
            width: "{settings.read().sidebar_width()}px",
            header {
                h2 { "Recordings" }
                div { class: "header-actions",
                    button {
                        class: "panel-toggle",
                        title: "Hide recordings",
                        onclick: move |_| sidebar_open.set(false),
                        "‹"
                    }
                }
            }
            RecordSettings {
                settings,
                windows,
                chosen_window,
                busy: status() != Status::Idle || picking(),
                on_refresh: move |_| recording::refresh_windows(windows, chosen_window),
            }
            p { class: "hint",
                if shortcut_ok {
                    "{platform::SHORTCUT_LABEL} or PolishCast's icon in the {TRAY_PLACE} records from any app. Its windows are left out."
                } else {
                    "PolishCast's icon in the {TRAY_PLACE} records from any app. Its windows are left out."
                }
            }
            if let Some(e) = error() {
                p { class: "error", "{e}" }
            }
            if let Some(reason) = auto_stopped() {
                p { class: "error", "{reason}" }
            }
            if let Some(n) = notice() {
                p { class: "hint", "{n}" }
            }
            if library.read().is_empty() {
                p { class: "empty", "No recordings yet." }
            }
            ul { class: "takes",
                for entry in library() {
                    li {
                        key: "{entry.name}",
                        class: if selected().as_ref() == Some(&entry.dir) { "on" } else { "" },
                        onclick: {
                            let dir = entry.dir.clone();
                            move |_| {
                                selected.set(Some(dir.clone()));
                                view.set(View::Takes);
                            }
                        },
                        span { class: "name", "{entry.name}" }
                        span { class: "meta",
                            if let Some(p) = &entry.project {
                                "{clock(Duration::from_secs_f64(p.duration))} · {p.width}×{p.height} · "
                            }
                            if compacting.read().contains(&entry.dir) {
                                "compacting…"
                            } else {
                                "{entry.size / 1_000_000} MB"
                            }
                            button {
                                class: "link",
                                title: platform::SHOW_IN_FOLDER,
                                onclick: {
                                    let path = entry.dir.join(take::SCREEN);
                                    move |e: MouseEvent| {
                                        e.stop_propagation();
                                        platform::reveal(&path);
                                    }
                                },
                                {platform::FILE_MANAGER}
                            }
                        }
                        if !compacting.read().contains(&entry.dir) {
                            button {
                                class: "item-trash",
                                title: "Move to the Trash…",
                                onclick: {
                                    let dir = entry.dir.clone();
                                    move |e: MouseEvent| {
                                        e.stop_propagation();
                                        confirm_delete.set(Some(dir.clone()));
                                    }
                                },
                                "🗑"
                            }
                        }
                    }
                }
            }
        }
    }
}
