//! The window and the menu bar item.

use crate::editor::Editor;
use crate::settings::{Settings, SourceKind};
use crate::{platform, takes};
use dioxus::desktop::tao::event::Event as TaoEvent;
use dioxus::desktop::trayicon::menu::{Menu, MenuItem, PredefinedMenuItem};
use dioxus::desktop::trayicon::{Icon, TrayIcon, TrayIconBuilder};
use dioxus::desktop::{use_global_shortcut, use_muda_event_handler, use_wry_event_handler, window};
use dioxus::prelude::*;
use polishcast_capture::{Area, Event, Options, Recorder, Source, Take, WindowInfo};
use polishcast_core::take;
use polishcast_export::compact::compact;
use polishcast_export::sound;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const CSS: &str = include_str!("../assets/style.css");

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Idle,
    /// Waiting for the first frame (and, the first time, for the permission prompt).
    Starting,
    Recording(Instant),
    /// Finishing the file.
    Saving,
}

/// The app's mark, black on transparent: macOS tints it to suit the menu bar. Windows and
/// Linux show icons as they are, so they get the colour app icon.
#[cfg(target_os = "macos")]
const TRAY_PNG: &[u8] = include_bytes!("../assets/tray.png");
#[cfg(not(target_os = "macos"))]
const TRAY_PNG: &[u8] = include_bytes!("../assets/icon.png");
const TRAY_PLACE: &str = if cfg!(target_os = "macos") { "menu bar" } else { "notification area" };

/// The menu bar item: its title shows the state, its menu starts and stops.
struct Tray {
    icon: TrayIcon,
    record: MenuItem,
    show: MenuItem,
}

impl Tray {
    fn new() -> Self {
        let record = MenuItem::new("Start Recording", true, None);
        let show = MenuItem::new("Show PolishCast", true, None);
        let menu = Menu::new();
        let _ = menu.append_items(&[&record, &show, &PredefinedMenuItem::separator(), &PredefinedMenuItem::quit(None)]);
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_tooltip("PolishCast")
            .with_icon_as_template(cfg!(target_os = "macos"));
        if let Some(icon) = crate::decode_png(TRAY_PNG, 44).and_then(|(rgba, w, h)| Icon::from_rgba(rgba, w, h).ok()) {
            builder = builder.with_icon(icon);
        }
        let icon = builder.build().expect("menu bar item");
        let tray = Self { icon, record, show };
        tray.update(Status::Idle);
        tray
    }

    fn update(&self, status: Status) {
        let (title, item, enabled) = match status {
            Status::Idle => (None, "Start Recording", true),
            Status::Starting => (Some("…".to_string()), "Stop Recording", true),
            Status::Recording(since) => (Some(format!("● {}", clock(since.elapsed()))), "Stop Recording", true),
            Status::Saving => (Some("saving".to_string()), "Saving…", false),
        };
        self.icon.set_title(title);
        self.record.set_text(item);
        self.record.set_enabled(enabled);
    }
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

#[component]
pub fn App() -> Element {
    let recorder = use_hook(Recorder::spawn);
    let pro = use_context_provider(crate::licence::Pro::new);
    let mut mcp_open = use_context_provider(|| crate::mcp::McpOpen(Signal::new(false))).0;
    // The licence is checked with api0 at start, then once a day.
    use_future(move || async move {
        let mut status = pro.status;
        status.set(crate::licence::refresh_async(true).await);
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            status.set(crate::licence::refresh_async(false).await);
        }
    });
    // Delayed and best-effort: a release check is never worth slowing a cold start, and a
    // failed one is not worth mentioning. Dismissed for this session only.
    let mut update = use_signal(|| Option::<crate::update_check::UpdateInfo>::None);
    use_future(move || async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Some(info) = crate::update_check::check().await {
            update.set(Some(info));
        }
    });
    let tray = use_hook(|| std::rc::Rc::new(Tray::new()));
    // Small Video.app, left in Applications by the update that renamed the app.
    let mut old_app = use_signal(|| Option::<PathBuf>::None);
    let mut old_app_error = use_signal(|| Option::<String>::None);
    use_hook(|| platform::exclude_from_capture(&window()));
    let mut status = use_signal(|| Status::Idle);
    let mut error = use_signal(|| Option::<String>::None);
    let mut library = use_signal(takes::list);
    let mut selected = use_signal(|| library.peek().first().map(|e| e.dir.clone()));
    // Takes whose recording is being shrunk in the background, and why the last one wasn't.
    let mut compacting = use_signal(HashSet::<PathBuf>::new);
    let mut notice = use_signal(|| Option::<String>::None);
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
    let mut settings = use_signal(Settings::load);
    use_hook(move || {
        if !settings.peek().keep_old_app {
            old_app.set(crate::old_app::find());
        }
    });
    // The windows that can be recorded, and the one chosen (this session only: ids don't last).
    let mut windows = use_signal(Vec::<WindowInfo>::new);
    let mut chosen_window = use_signal(|| Option::<u32>::None);
    let refresh_windows = move || {
        spawn(async move {
            if let Ok(list) = tokio::task::spawn_blocking(polishcast_capture::windows).await {
                if chosen_window.peek().is_some_and(|id| !list.iter().any(|w| w.id == id)) {
                    chosen_window.set(None);
                }
                windows.set(list);
            }
        });
    };
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

    let start = {
        let recorder = recorder.clone();
        use_callback(move |source: Source| match takes::new_dir() {
            Ok(dir) => {
                error.set(None);
                let options = Options { source, system_audio: settings.peek().system_audio, ..Options::default() };
                recording_dir.set(Some(dir.clone()));
                auto_stopped.set(None);
                recorder.start(dir, options);
                status.set(Status::Starting);
            }
            Err(e) => error.set(Some(format!("Can't create the take's folder: {e}"))),
        })
    };

    let toggle = {
        let recorder = recorder.clone();
        let area_tx = area_tx.clone();
        use_callback(move |()| match status() {
            Status::Idle if picking() => {}
            Status::Idle => match settings.peek().source {
                SourceKind::Screen => start(Source::Display),
                SourceKind::Window => match chosen_window() {
                    Some(id) => start(Source::Window(id)),
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
            Status::Starting | Status::Recording(_) => {
                recorder.stop();
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
            toggle(());
        } else if e.id == show_id {
            window().set_visible(true);
            window().set_focus();
        }
    });
    // Clicking the app's icon (Dock, Finder, Launchpad) while it runs with the window closed:
    // macOS asks the app to reopen, and Dioxus leaves that unanswered.
    use_wry_event_handler(|event, _| {
        if let TaoEvent::Reopen { .. } = event {
            window().set_visible(true);
            window().set_focus();
        }
    });
    let shortcut = use_global_shortcut(platform::SHORTCUT, move |state| {
        if state == dioxus::desktop::HotKeyState::Pressed {
            toggle(());
        }
    });

    // Recorder events, and the elapsed time in the menu bar. 10 Hz is plenty for both.
    use_future({
        let (recorder, tray) = (recorder.clone(), tray.clone());
        move || {
            let (recorder, tray) = (recorder.clone(), tray.clone());
            let (compacted_tx, compacted_rx) = (compacted_tx.clone(), compacted_rx.clone());
            let area_rx = area_rx.clone();
            async move {
                let (mixed_tx, mixed_rx) = mpsc::channel::<(Take, Option<String>)>();
                // A finished take: its project, then shrinking it in the background.
                let mut saved = move |t: Take| {
                    if let Err(e) = takes::create_project(&t) {
                        error.set(Some(format!("Can't save the project: {e}")));
                    }
                    status.set(Status::Idle);
                    library.set(takes::list());
                    if t.compact {
                        compacting.write().insert(t.dir.clone());
                        let (tx, dir) = (compacted_tx.clone(), t.dir.clone());
                        std::thread::spawn(move || {
                            let result = compact(&dir.join(take::SCREEN)).map_err(|e| format!("{e:#}"));
                            let _ = tx.send((dir, result));
                        });
                    }
                    selected.set(Some(t.dir));
                    window().set_visible(true);
                    window().set_focus();
                };
                let mut tick = 0u64;
                loop {
                    tick += 1;
                    // A forgotten recording stops before it fills the disk (checked every 2 s).
                    let current = *status.peek();
                    if let (Status::Recording(since), true) = (current, tick.is_multiple_of(20)) {
                        let dir = recording_dir.peek().clone();
                        let size =
                            dir.as_ref().and_then(|d| std::fs::metadata(d.join(take::SCREEN)).ok()).map(|m| m.len());
                        let free = dir.as_deref().and_then(crate::limits::free_space);
                        if let Some(reason) = crate::limits::reason_to_stop(since.elapsed(), size, free) {
                            auto_stopped.set(Some(reason));
                            recorder.stop();
                            status.set(Status::Saving);
                        }
                    }
                    while let Ok(area) = area_rx.try_recv() {
                        picking.set(false);
                        if let Some(area) = area {
                            start(Source::Area(area));
                        }
                    }
                    while let Some(event) = recorder.try_event() {
                        match event {
                            // Unless Stop was already pressed while it was starting.
                            Event::Started if *status.peek() == Status::Starting => {
                                status.set(Status::Recording(Instant::now()))
                            }
                            Event::Started => {}
                            // The sound apps played, recorded apart from one window, goes in
                            // before the take opens: its pauses are measured once, from the file.
                            Event::Finished(t) if t.sound.is_some() => {
                                let tx = mixed_tx.clone();
                                std::thread::spawn(move || {
                                    let offset = t.sound.unwrap_or_default();
                                    let file = t.dir.join(take::SCREEN);
                                    let result = sound::mix_in(&file, &t.dir.join(take::SOUND), offset);
                                    let _ = tx.send((t, result.err().map(|e| format!("{e:#}"))));
                                });
                            }
                            Event::Finished(t) => saved(t),
                            Event::Failed(e) => {
                                error.set(Some(e));
                                status.set(Status::Idle);
                            }
                        }
                    }
                    while let Ok((t, failed)) = mixed_rx.try_recv() {
                        saved(t);
                        if failed.is_some() {
                            notice.set(failed);
                        }
                    }
                    while let Ok((dir, result)) = compacted_rx.try_recv() {
                        compacting.write().remove(&dir);
                        notice.set(result.err());
                        library.set(takes::list());
                    }
                    tray.update(*status.peek());
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

    let label = match status() {
        Status::Idle => "Record",
        Status::Starting | Status::Recording(_) => "Stop",
        Status::Saving => "Saving…",
    };
    let recording = matches!(status(), Status::Starting | Status::Recording(_));

    rsx! {
        style { {CSS} }
        crate::theme::ThemeToggle {}
        if let Some(info) = update() {
            div { class: "update-banner",
                span { class: "update-banner-text",
                    "PolishCast "
                    strong { "{info.latest_version}" }
                    " is available (you have {env!(\"CARGO_PKG_VERSION\")})."
                }
                a { class: "update-banner-link", href: "{info.download_url}", target: "_blank", "Download" }
                button { class: "update-banner-dismiss", title: "Dismiss", onclick: move |_| update.set(None), "×" }
            }
        } else if let Some(app) = old_app() {
            div { class: "update-banner",
                span { class: "update-banner-text",
                    if let Some(e) = old_app_error() {
                        "Couldn't move Small Video to the Trash ({e}). Drag it there from {platform::FILE_MANAGER}."
                    } else {
                        "Small Video, this app under its old name, is still in Applications. "
                        "Opening it starts the old version."
                    }
                }
                if old_app_error().is_some() {
                    button {
                        class: "update-banner-link",
                        onclick: {
                            let app = app.clone();
                            move |_| platform::reveal(&app)
                        },
                        "{platform::SHOW_IN_FOLDER}"
                    }
                } else {
                    button {
                        class: "update-banner-link",
                        onclick: {
                            let app = app.clone();
                            move |_| match crate::old_app::remove(&app) {
                                Ok(()) => old_app.set(None),
                                Err(e) => old_app_error.set(Some(e)),
                            }
                        },
                        "Move to Trash"
                    }
                    button {
                        class: "update-banner-link",
                        onclick: move |_| {
                            settings.write().keep_old_app = true;
                            settings.peek().save();
                            old_app.set(None);
                        },
                        "Keep"
                    }
                }
                button { class: "update-banner-dismiss", title: "Dismiss", onclick: move |_| old_app.set(None), "×" }
            }
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
                    button {
                        class: if recording { "record compact on" } else { "record compact" },
                        title: "{label}",
                        disabled: status() == Status::Saving,
                        onclick: move |_| toggle(()),
                        if recording { "■" } else { "●" }
                    }
                }
            }
            nav {
                class: "sidebar",
                hidden: !sidebar_open(),
                width: "{settings.read().sidebar_width()}px",
                header {
                    h1 {
                        "PolishCast"
                        // Which build is running, for bug reports and support.
                        span { class: "app-version", {concat!("v", env!("CARGO_PKG_VERSION"))} }
                    }
                    div { class: "header-actions",
                        button {
                            class: if recording { "record on" } else { "record" },
                            disabled: status() == Status::Saving,
                            onclick: move |_| toggle(()),
                            "{label}"
                        }
                        button {
                            class: "panel-toggle",
                            title: "Hide recordings",
                            onclick: move |_| sidebar_open.set(false),
                            "‹"
                        }
                    }
                }
                {
                    let mut open = pro.open;
                    let (label, title) = match *pro.status.read() {
                        crate::licence::Status::Pro(_) => ("Pro \u{2713}", "PolishCast Pro is active on this computer"),
                        crate::licence::Status::Free => ("Get Pro\u{2026}", "Subscribe to PolishCast Pro, or paste your licence number"),
                        crate::licence::Status::NotPro(ref a) if a.is_free() => ("Get Pro\u{2026}", "Free AI credits, no Pro: subscribe to PolishCast Pro to export every take"),
                        _ => ("Renew Pro\u{2026}", "Your licence doesn't make this PolishCast Pro right now"),
                    };
                    rsx! {
                        div { class: "sidebar-buttons",
                            button { class: "pro-button", title: "{title}", onclick: move |_| open.set(Some(None)), "{label}" }
                            button {
                                class: "mcp-button",
                                title: "Use PolishCast from Claude Desktop or Claude Code",
                                onclick: move |_| mcp_open.set(true),
                                "MCP"
                            }
                            // A link, not a button: the webview hands it to the browser.
                            a {
                                class: "button mcp-button",
                                href: "{crate::feedback::contact_url()}",
                                target: "_blank",
                                title: "Ideas, bugs, questions: send them to the person who makes PolishCast",
                                "Feedback"
                            }
                        }
                    }
                }
                RecordSettings {
                    settings,
                    windows,
                    chosen_window,
                    busy: status() != Status::Idle || picking(),
                    on_refresh: move |_| refresh_windows(),
                }
                p { class: "hint",
                    if shortcut.is_ok() {
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
                                move |_| selected.set(Some(dir.clone()))
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
            crate::licence::ProDialog {}
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
                if let Some(dir) = selected() {
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
                h2 { "Move to the Trash?" }
                p { class: "delete-name", "{name}" }
                p { class: "hint",
                    "The recording and its edits go to the system Trash, so you can still restore them from there."
                    if exported { " The videos you exported are not touched." }
                }
                div { class: "modal-actions",
                    button { onclick: move |_| on_close.call(()), "Cancel" kbd { "Esc" } }
                    button { class: "danger", onclick: move |_| confirm(), "Move to Trash" kbd { "Enter" } }
                }
            }
        }
    }
}
