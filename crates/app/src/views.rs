//! The window and the menu bar item.

use crate::editor::Editor;
use crate::remote::{self, Asked, Op, Remote};
use crate::settings::{AiRegion, Settings, SourceKind};
use crate::{platform, takes};
use desktop_kit::ui::{use_theme, Banners, Header, ModelBadge, ProButton, ProWindow, SettingsWindow, Toolbar};
use dioxus::desktop::tao::event::Event as TaoEvent;
use dioxus::desktop::trayicon::menu::{Menu, MenuItem, PredefinedMenuItem};
use dioxus::desktop::trayicon::{Icon, TrayIcon, TrayIconBuilder};
use dioxus::desktop::{use_global_shortcut, use_muda_event_handler, use_wry_event_handler, window};
use dioxus::prelude::*;
use polishcast_capture::{Area, Event, Options, Recorder, Source, Take, WindowInfo};
use polishcast_core::{take, EventLog};
use polishcast_export::compact::compact;
use polishcast_export::sound;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

mod answer;
mod background;
mod banner;
mod recording;
mod sidebar;
mod tray;
use answer::*;
use tray::Tray;

const CSS: &str = include_str!("../assets/style.css");

/// What the main area shows: the selected take, or the library and calendar every Mayorana
/// app shares.
#[derive(Clone, Copy, PartialEq)]
enum View {
    Takes,
    Library,
    Calendar,
}

impl View {
    const ALL: [View; 3] = [View::Takes, View::Library, View::Calendar];

    fn label(self) -> &'static str {
        match self {
            View::Takes => "Takes",
            View::Library => "Library",
            View::Calendar => "Calendar",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Idle,
    /// Asked for by an MCP client: counting down until then.
    Countdown(Instant),
    /// Waiting for the first frame (and, the first time, for the permission prompt).
    Starting,
    Recording(Instant),
    /// Finishing the file.
    Saving,
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

#[component]
pub fn App() -> Element {
    let recorder = use_hook(Recorder::spawn);
    let pro = use_context_provider(crate::licence::Pro::new);
    let theme = use_theme();
    // The title bar follows the licence, however it changes.
    use_effect(move || {
        dioxus::desktop::window().set_title(&crate::window_title(&pro.status.read()));
    });
    let mut mcp_open = use_context_provider(|| crate::mcp::McpOpen(Signal::new(false))).0;
    let mut settings_open = use_signal(|| false);
    let closed_regions = desktop_kit::ui::use_closed_regions();
    background::use_licence_check(pro);
    let tray = use_hook(|| std::rc::Rc::new(Tray::new()));
    // Small Video.app, left in Applications by the update that renamed the app.
    let mut old_app = use_signal(|| Option::<PathBuf>::None);
    let old_app_error = use_signal(|| Option::<String>::None);
    use_hook(|| platform::exclude_from_capture(&window()));
    let mut status = use_signal(|| Status::Idle);
    let mut error = use_signal(|| Option::<String>::None);
    let mut library = use_signal(takes::list);
    let mut selected = use_signal(|| library.peek().first().map(|e| e.dir.clone()));
    let mut view = use_signal(|| View::Takes);
    // The library every Mayorana app shares, and a take opened from it, here or from
    // PublishKit or TrackCut.
    let shared = desktop_kit::ui::use_library();
    let open_take = move |target: String| {
        let dir = PathBuf::from(target);
        if dir.is_dir() {
            selected.set(Some(dir));
            view.set(View::Takes);
            window().set_visible(true);
            window().set_focus();
        } else {
            error.set(Some(format!("{} isn't there any more.", dir.display())));
        }
    };
    background::use_library_sync(open_take);
    // Takes whose recording is being shrunk in the background, and why the last one wasn't.
    let compacting = use_signal(HashSet::<PathBuf>::new);
    let notice = use_signal(|| Option::<String>::None);
    // A take waiting for the user to confirm moving it to the Trash.
    let mut confirm_delete = use_signal(|| Option::<PathBuf>::None);
    // The recordings panel on the left; closed, a thin rail keeps Record and a way back.
    let mut sidebar_open = use_signal(|| true);
    // Its edge being dragged: where the pointer and the edge were when the drag began.
    let mut resizing = use_signal(|| Option::<(f64, f64)>::None);
    let (compacted_tx, compacted_rx) = use_hook(|| {
        let (tx, rx) = mpsc::channel::<(PathBuf, Result<u64, String>)>();
        (tx, Rc::new(rx))
    });
    // What to record, and with which sound; kept for next time.
    // Shared with the MCP window, which turns recording from a client on and off.
    let mut settings = use_context_provider(|| Signal::new(Settings::load()));
    // A recording an MCP client asked for, and how the last one ended.
    let mut remote = use_signal(Remote::default);
    use_hook(move || {
        if !settings.peek().keep_old_app {
            old_app.set(crate::old_app::find());
        }
    });
    // The windows that can be recorded, and the one chosen (this session only: ids don't last).
    let windows = use_signal(Vec::<WindowInfo>::new);
    let chosen_window = use_signal(|| Option::<u32>::None);
    let refresh_windows = move || recording::refresh_windows(windows, chosen_window);
    use_hook(move || {
        if settings.peek().source == SourceKind::Window {
            refresh_windows();
        }
    });
    // The area picker's answer, and whether it's open (Record is ignored meanwhile).
    let (area_tx, area_rx) = use_hook(|| {
        let (tx, rx) = mpsc::channel::<Option<Area>>();
        (tx, Rc::new(rx))
    });
    let mut picking = use_signal(|| false);
    // The folder of the take being recorded, to keep an eye on its size.
    let mut recording_dir = use_signal(|| Option::<PathBuf>::None);
    // Why the last recording was stopped for the user, until the next one starts.
    let mut auto_stopped = use_signal(|| Option::<String>::None);
    let rec = recording::Rec {
        status,
        error,
        library,
        selected,
        compacting,
        notice,
        remote,
        settings,
        recording_dir,
        auto_stopped,
        picking,
        windows,
        chosen_window,
    };

    let begin = {
        let recorder = recorder.clone();
        use_callback(move |(dir, options): (PathBuf, Options)| {
            error.set(None);
            recording_dir.set(Some(dir.clone()));
            auto_stopped.set(None);
            crate::library::recording(&dir);
            recorder.start(dir, options);
            status.set(Status::Starting);
        })
    };
    let start = use_callback(move |(source, clicked): (Source, bool)| match takes::new_dir() {
        Ok(dir) => {
            let system_audio = settings.peek().system_audio;
            begin((dir, Options { source, system_audio, clicked, ..Options::default() }));
        }
        Err(e) => error.set(Some(format!("Can't create the take's folder: {e}"))),
    });

    let toggle = {
        let recorder = recorder.clone();
        let area_tx = area_tx.clone();
        // `clicked`: from a button or the menu bar rather than the shortcut.
        use_callback(move |clicked: bool| match status() {
            Status::Idle if picking() => {}
            Status::Idle => match settings.peek().source {
                SourceKind::Screen => start((Source::Display, clicked)),
                SourceKind::Window => match chosen_window() {
                    Some(id) => start((Source::Window(id), clicked)),
                    None => {
                        error.set(Some("Choose the window to record first.".into()));
                        window().set_visible(true);
                        window().set_focus();
                    }
                },
                SourceKind::Area => {
                    picking.set(true);
                    crate::area_picker::open(area_tx.clone());
                }
            },
            // Asked for by a client and not started yet: nothing was recorded.
            Status::Countdown(_) => {
                if let Some(mut a) = remote.write().current.take() {
                    a.reply_started(Err("The person cancelled the recording during the countdown.".into()));
                    let _ = std::fs::remove_dir(&a.dir);
                }
                status.set(Status::Idle);
            }
            Status::Starting | Status::Recording(_) => {
                if let Some(a) = remote.write().current.as_mut() {
                    a.stopped_by.get_or_insert("the person, from PolishCast");
                }
                if clicked {
                    recorder.stop_clicked();
                } else {
                    recorder.stop();
                }
                status.set(Status::Saving);
            }
            Status::Saving => {}
        })
    };

    let (record_id, show_id) = (tray.record.id().clone(), tray.show.id().clone());
    // Menu bar item clicks arrive as *window* menu events, not tray menu events: the menu
    // library keeps a single global handler that only the first registration sets, and Dioxus
    // 0.7 registers the window-menu one first. `use_tray_menu_event_handler` never fires.
    use_muda_event_handler(move |e| {
        if e.id == record_id {
            toggle(true);
        } else if e.id == show_id {
            window().set_visible(true);
            window().set_focus();
        }
    });
    background::use_show_requests();
    let shortcut = use_global_shortcut(platform::SHORTCUT, move |state| {
        if state == dioxus::desktop::HotKeyState::Pressed {
            toggle(false);
        }
    });

    recording::use_recording(rec, recorder.clone(), tray.clone(), compacted_tx, compacted_rx, area_rx, begin, start);

    let label = match status() {
        Status::Idle => "Record",
        Status::Countdown(_) => "Cancel",
        Status::Starting | Status::Recording(_) => "Stop",
        Status::Saving => "Saving…",
    };
    let recording = matches!(status(), Status::Countdown(_) | Status::Starting | Status::Recording(_));

    rsx! {
        style { {CSS} }
        Header {
            on_settings: move |_| settings_open.set(true),
            on_mcp: move |_| mcp_open.set(true),
            model: rsx! {
                ModelBadge {
                    label: desktop_kit::region::find(settings().ai_region.id()).map_or("api0", |r| r.label).to_string(),
                    region: Some(settings().ai_region.id()),
                    title: "where api0 transcribes and names chapters \u{2014} click to change it",
                    onclick: move |_| settings_open.set(true),
                }
            },
            pro: rsx! { ProButton { pro: pro.kit(), hint: crate::licence::hint() } },
        }
        Toolbar {
            div { class: "kit-tabs",
                for v in View::ALL {
                    button {
                        class: if view() == v { "kit-tab kit-on" } else { "kit-tab" },
                        onclick: move |_| view.set(v),
                        "{v.label()}"
                    }
                }
            }
            span { class: "kit-spacer" }
            button {
                class: if recording { "kit-btn kit-record kit-on" } else { "kit-btn kit-record" },
                disabled: status() == Status::Saving,
                onclick: move |_| toggle(true),
                span { class: "kit-record-dot" }
                "{label}"
            }
        }
        if settings_open() {
            SettingsWindow { theme, on_close: move |_| settings_open.set(false),
                div { class: "kit-set-general",
                    div { class: "kit-set-row",
                        span { class: "kit-set-label", "AI region" }
                        div { class: "kit-set-seg",
                            for r in AiRegion::ALL {
                                button {
                                    class: if settings().ai_region == r { "kit-on" } else { "" },
                                    disabled: closed_regions.read().iter().any(|c| c == r.id()),
                                    title: if closed_regions.read().iter().any(|c| c == r.id()) { desktop_kit::region::COMING_SOON } else { "Where api0 transcribes and names chapters, on the licence's AI credits" },
                                    onclick: move |_| {
                                        settings.write().ai_region = r;
                                        settings.peek().save();
                                    },
                                    {desktop_kit::region::find(r.id()).map_or(r.id(), |x| x.label)}
                                }
                            }
                        }
                    }
                }
            }
        }
        Banners {
            {banner::old_app_banner(old_app, old_app_error, settings)}
        }
        div { class: "app",
            if !sidebar_open() {
                nav { class: "rail",
                    button {
                        class: "panel-toggle",
                        title: "Show recordings",
                        onclick: move |_| sidebar_open.set(true),
                        "›"
                    }
                }
            }
            {sidebar::sidebar(rec, sidebar_open, view, confirm_delete, shortcut.is_ok())}
            if sidebar_open() {
                div {
                    class: "sidebar-resizer",
                    title: "Drag to resize · double-click for the default width",
                    onmousedown: move |e: MouseEvent| {
                        e.prevent_default();
                        resizing.set(Some((e.client_coordinates().x, settings.peek().sidebar_width())));
                    },
                    ondoubleclick: move |_| {
                        settings.write().sidebar_width = None;
                        settings.peek().save();
                    },
                }
            }
            // While the edge is dragged, a sheet over the whole window follows the pointer, so
            // the drag keeps going over the video and the timeline.
            if let Some((from_x, from_width)) = resizing() {
                div {
                    class: "resize-sheet",
                    onmousemove: move |e: MouseEvent| {
                        let w = from_width + e.client_coordinates().x - from_x;
                        settings.write().sidebar_width = Some(w.clamp(crate::settings::SIDEBAR_MIN, crate::settings::SIDEBAR_MAX));
                    },
                    onmouseup: move |_| {
                        resizing.set(None);
                        settings.peek().save();
                    },
                }
            }
            ProWindow {
                pro: pro.kit(),
                pitch: crate::licence::pitch(pro.refused.read().clone()),
                details: crate::licence::details(pro),
                on_close: move |_| {
                    let mut refused = pro.refused;
                    refused.set(None);
                },
            }
            crate::mcp::McpDialog {}
            if let Some(dir) = confirm_delete() {
                DeleteDialog {
                    dir,
                    on_close: move |_| confirm_delete.set(None),
                    on_confirm: move |dir: PathBuf| {
                        confirm_delete.set(None);
                        match takes::move_to_trash(&dir) {
                            Ok(()) => {
                                if selected.peek().as_ref() == Some(&dir) {
                                    selected.set(None);
                                }
                                library.set(takes::list());
                                if selected.peek().is_none() {
                                    selected.set(library.peek().first().map(|e| e.dir.clone()));
                                }
                            }
                            Err(e) => error.set(Some(format!("Could not move the take to the Trash: {e}"))),
                        }
                    },
                }
            }
            main { class: "main",
                if view() == View::Library {
                    div { class: "shared-pane",
                        desktop_kit::ui::LibraryView { here: "polishcast", items: shared, on_open: open_take }
                    }
                } else if view() == View::Calendar {
                    div { class: "shared-pane",
                        desktop_kit::ui::PipelineCalendar { here: "polishcast", items: shared, on_open: open_take }
                    }
                } else if let Some(dir) = selected() {
                    // Reloaded when its recording is replaced by the compacted one.
                    Editor { key: "{dir.display()}-{compacting.read().contains(&dir)}", dir }
                } else {
                    p { class: "empty pad", "Record something to start editing." }
                }
            }
        }
    }
}

/// What the next recording takes in: the screen, a window or an area, and whether the Mac's
/// sound goes in with the microphone. Locked while recording.
#[component]
fn RecordSettings(
    settings: Signal<Settings>,
    windows: Signal<Vec<WindowInfo>>,
    chosen_window: Signal<Option<u32>>,
    busy: bool,
    on_refresh: EventHandler<()>,
) -> Element {
    let current = settings();
    let mut set = move |change: &dyn Fn(&mut Settings)| {
        change(&mut settings.write());
        settings.peek().save();
    };
    let sources: &[(SourceKind, &str, &str)] = &[
        (SourceKind::Screen, "Screen", "The whole screen the pointer is on"),
        (SourceKind::Window, "Window", "One window, even when others cover it"),
        (SourceKind::Area, "Area", "Drag across the part to record when you start"),
    ];
    rsx! {
        div { class: "record-settings",
            if polishcast_capture::RECORDS_WINDOWS_AND_AREAS {
                div { class: "segmented",
                    for &(kind, name, tip) in sources {
                        button {
                            class: if current.source == kind { "on" } else { "" },
                            title: "{tip}",
                            disabled: busy,
                            onclick: move |_| {
                                set(&|s: &mut Settings| s.source = kind);
                                if kind == SourceKind::Window {
                                    on_refresh.call(());
                                }
                            },
                            "{name}"
                        }
                    }
                }
                if current.source == SourceKind::Window {
                    div { class: "window-choice",
                        select {
                            disabled: busy,
                            onchange: move |e| chosen_window.set(e.value().parse().ok()),
                            option { value: "", selected: chosen_window().is_none(), "Choose a window…" }
                            for w in windows() {
                                option {
                                    key: "{w.id}",
                                    value: "{w.id}",
                                    selected: chosen_window() == Some(w.id),
                                    if w.title.is_empty() { "{w.app}" } else { "{w.app} — {w.title}" }
                                    if !w.on_screen { " (another Space or minimized)" }
                                }
                            }
                        }
                        button { title: "Look for windows again", disabled: busy, onclick: move |_| on_refresh.call(()), "↻" }
                    }
                }
            }
            if polishcast_capture::RECORDS_SYSTEM_AUDIO {
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: current.system_audio,
                        disabled: busy,
                        onchange: move |e| set(&|s: &mut Settings| s.system_audio = e.checked()),
                    }
                    "Record the Mac's sound too"
                }
            }
        }
    }
}

/// Asks before moving a take to the Trash: Enter confirms, Esc cancels (as in TrackCut).
#[component]
fn DeleteDialog(dir: PathBuf, on_close: EventHandler<()>, on_confirm: EventHandler<PathBuf>) -> Element {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let exported = !crate::editor::exports_of(&dir).is_empty();
    let target = dir.clone();
    let confirm = move || on_confirm.call(target.clone());
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| on_close.call(()),
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
                            Key::Escape => on_close.call(()),
                            _ => {}
                        }
                    }
                },
                div { class: "modal-head",
                    h2 { "Move to the Trash?" }
                    button { class: "modal-close", title: "Close", onclick: move |_| on_close.call(()), "×" }
                }
                div { class: "modal-body",
                    p { class: "delete-name", "{name}" }
                    p { class: "hint",
                        if exported {
                            "The recording, its edits and the videos exported from it go to the system Trash, so you can still restore them from there."
                        } else {
                            "The recording and its edits go to the system Trash, so you can still restore them from there."
                        }
                    }
                    div { class: "modal-actions",
                        button { onclick: move |_| on_close.call(()), "Cancel" kbd { "Esc" } }
                        button { class: "danger", onclick: move |_| confirm(), "Move to Trash" kbd { "Enter" } }
                    }
                }
            }
        }
    }
}
