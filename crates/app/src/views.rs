//! The window and the menu bar item.

use crate::editor::Editor;
use crate::remote::{self, Asked, Op, Remote};
use crate::settings::{Settings, SourceKind};
use crate::{platform, takes};
use desktop_kit::ui::{use_theme, Banners, Header, ProButton, ProWindow};
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

/// The app's mark, black on transparent: macOS tints it to suit the menu bar. Windows and
/// Linux show icons as they are, so they get the colour app icon.
#[cfg(target_os = "macos")]
const TRAY_PNG: &[u8] = include_bytes!("../assets/tray.png");
#[cfg(not(target_os = "macos"))]
const TRAY_PNG: &[u8] = include_bytes!("../assets/icon.png");
const TRAY_PLACE: &str = if cfg!(target_os = "macos") { "menu bar" } else { "notification area" };

/// The menu bar item: its title shows the state, its menu starts and stops.
struct Tray {
    /// `None` when the system would not show one; the window still has every control.
    icon: Option<TrayIcon>,
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
        let icon = builder.build().ok();
        let tray = Self { icon, record, show };
        tray.update(Status::Idle, None);
        tray
    }

    /// `by`: the MCP client that started the recording, named next to the time.
    fn update(&self, status: Status, by: Option<&str>) {
        let after = by.map(|b| format!(" {b}")).unwrap_or_default();
        let (title, item, enabled) = match status {
            Status::Idle => (None, "Start Recording", true),
            Status::Countdown(until) => {
                let left = until.saturating_duration_since(Instant::now()).as_secs_f64().ceil();
                (Some(format!("● in {left}{after}")), "Cancel Recording", true)
            }
            Status::Starting => (Some(format!("…{after}")), "Stop Recording", true),
            Status::Recording(since) => (Some(format!("● {}{after}", clock(since.elapsed()))), "Stop Recording", true),
            Status::Saving => (Some("saving".to_string()), "Saving…", false),
        };
        let tip = match (status, by) {
            (Status::Idle, _) | (_, None) => "PolishCast".to_string(),
            (_, Some(b)) => format!("PolishCast: recording, started from {b}. Stop Recording stops it."),
        };
        if let Some(icon) = &self.icon {
            icon.set_title(title);
            let _ = icon.set_tooltip(Some(tip));
        }
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
    let theme = use_theme();
    // The title bar follows the licence, however it changes.
    use_effect(move || {
        dioxus::desktop::window().set_title(&crate::window_title(&pro.status.read()));
    });
    let mut mcp_open = use_context_provider(|| crate::mcp::McpOpen(Signal::new(false))).0;
    // The licence is checked with api0 at start, then once a day.
    use_future(move || async move {
        let mut status = pro.status;
        status.set(desktop_kit::licence::refresh(true).await);
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            status.set(desktop_kit::licence::refresh(false).await);
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
    let mut view = use_signal(|| View::Takes);
    // The library every Mayorana app shares, and a take opened from it, here or from
    // PublishKit or TrackCut.
    let shared = desktop_kit::ui::use_library();
    let mut open_take = move |target: String| {
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
    // The takes in the shared library, kept as they are on disk: read again whenever a take or
    // an export comes, goes or changes.
    use_future(move || async move {
        let mut last = None;
        loop {
            let stamp = tokio::task::spawn_blocking(crate::library::stamp).await.ok();
            if stamp != last {
                let _ = tokio::task::spawn_blocking(crate::library::sync_all).await;
                last = stamp;
            }
            if let Some(note) =
                tokio::task::spawn_blocking(|| desktop_kit::handoff::take("polishcast", "open")).await.ok().flatten()
            {
                if let Some(target) = note["open"].as_str() {
                    open_take(target.to_string());
                }
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    });
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
    let mut windows = use_signal(Vec::<WindowInfo>::new);
    let mut chosen_window = use_signal(|| Option::<u32>::None);
    let refresh_windows = move || {
        spawn(async move {
            if let Ok(list) = tokio::task::spawn_blocking(polishcast_capture::windows).await {
                if chosen_window.peek().is_some_and(|id| !list.iter().any(|w| w.id == id)) {
                    chosen_window.set(None);
                }
                // Unchanged, the menu is left alone: rebuilding it would close it while open.
                if *windows.peek() != list {
                    windows.set(list);
                }
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
    // Launched again while running (Start menu, a shortcut): the other launch handed over and
    // quit, and this window comes up instead.
    use_future(|| async {
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if crate::instance::take_show_request() {
                window().set_visible(true);
                window().set_minimized(false);
                window().set_focus();
            }
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
            toggle(false);
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
                    let project = match takes::create_project(&t) {
                        Ok(p) => Some(p),
                        Err(e) => {
                            error.set(Some(format!("Can't save the project: {e}")));
                            None
                        }
                    };
                    status.set(Status::Idle);
                    // A client's recording: its answer, or the Trash if it was cancelled.
                    let asked = remote.write().current.take_if(|a| a.dir == t.dir);
                    if let Some(mut a) = asked {
                        if a.cancel {
                            let _ = takes::move_to_trash(&t.dir);
                            a.reply_saved(&Ok(json!({ "cancelled": true, "recording_id": a.id })));
                            library.set(takes::list());
                            return;
                        }
                        let result = Ok(finished(&a, &t, project.as_ref()));
                        a.reply_saved(&result);
                        remote.write().last = Some((a.id, result));
                    }
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
                    // Windows come and go: the list follows while it's on show (every 2 s).
                    if tick.is_multiple_of(20)
                        && *status.peek() == Status::Idle
                        && settings.peek().source == SourceKind::Window
                        && window().is_visible()
                    {
                        refresh_windows();
                    }
                    // A forgotten recording stops before it fills the disk (checked every 2 s).
                    let current = *status.peek();
                    if let (Status::Recording(since), true) = (current, tick.is_multiple_of(20)) {
                        let dir = recording_dir.peek().clone();
                        let size =
                            dir.as_ref().and_then(|d| std::fs::metadata(d.join(take::SCREEN)).ok()).map(|m| m.len());
                        let free = dir.as_deref().and_then(crate::limits::free_space);
                        if let Some(reason) = crate::limits::reason_to_stop(since.elapsed(), size, free) {
                            auto_stopped.set(Some(reason));
                            if let Some(a) = remote.write().current.as_mut() {
                                a.stopped_by.get_or_insert("PolishCast's own limit on length, size or free disk space");
                            }
                            recorder.stop();
                            status.set(Status::Saving);
                        }
                    }
                    // A client's recording: past its countdown, it starts; past its length, it stops.
                    if let Status::Countdown(until) = current {
                        if Instant::now() >= until {
                            let next =
                                remote.write().current.as_mut().and_then(|a| Some((a.dir.clone(), a.options.take()?)));
                            match next {
                                Some(next) => begin(next),
                                None => status.set(Status::Idle),
                            }
                        }
                    }
                    if let Status::Recording(since) = current {
                        let max = remote.peek().current.as_ref().map(|a| (a.max, a.client.clone()));
                        if let Some((max, client)) = max.filter(|(max, _)| since.elapsed() >= *max) {
                            auto_stopped.set(Some(format!(
                                "The recording was stopped after {} s, the length {client} asked for.",
                                max.as_secs()
                            )));
                            if let Some(a) = remote.write().current.as_mut() {
                                a.stopped_by.get_or_insert("max_duration_secs");
                            }
                            recorder.stop();
                            status.set(Status::Saving);
                        }
                    }
                    for request in remote::take_requests() {
                        answer(request, status, *picking.peek(), remote, settings, recording_dir, &recorder);
                    }
                    while let Ok(area) = area_rx.try_recv() {
                        picking.set(false);
                        // The area was dragged: the pointer starts at its corner.
                        if let Some(area) = area {
                            start((Source::Area(area), true));
                        }
                    }
                    while let Some(event) = recorder.try_event() {
                        match event {
                            // Unless Stop was already pressed while it was starting.
                            Event::Started => {
                                if *status.peek() == Status::Starting {
                                    status.set(Status::Recording(Instant::now()));
                                }
                                if let Some(a) = remote.write().current.as_mut() {
                                    a.started_at = Some(chrono::Local::now());
                                    let started = started(a);
                                    a.reply_started(Ok(started));
                                }
                            }
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
                                let asked = remote.write().current.take();
                                if let Some(mut a) = asked {
                                    let failed = Err(e.clone());
                                    a.reply_started(failed.clone());
                                    a.reply_saved(&failed);
                                    let _ = std::fs::remove_dir(&a.dir);
                                    remote.write().last = Some((a.id, failed));
                                }
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
                    let by = remote.peek().current.as_ref().map(|a| a.client.clone());
                    tray.update(*status.peek(), by.as_deref());
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

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
            theme,
            on_mcp: move |_| mcp_open.set(true),
            actions: rsx! {
                button {
                    class: if recording { "kit-btn kit-record kit-on" } else { "kit-btn kit-record" },
                    disabled: status() == Status::Saving,
                    onclick: move |_| toggle(true),
                    span { class: "kit-record-dot" }
                    "{label}"
                }
            },
            pro: rsx! { ProButton { pro: pro.kit(), hint: crate::licence::hint() } },
            div { class: "kit-tabs",
                for v in View::ALL {
                    button {
                        class: if view() == v { "kit-tab kit-on" } else { "kit-tab" },
                        onclick: move |_| view.set(v),
                        "{v.label()}"
                    }
                }
            }
        }
        Banners {
            if let Some(app) = old_app() {
                div { class: "kit-banner",
                    span { class: "kit-banner-text",
                        if let Some(e) = old_app_error() {
                            "Couldn't move Small Video to the Trash ({e}). Drag it there from {platform::FILE_MANAGER}."
                        } else {
                            "Small Video, this app under its old name, is still in Applications. "
                            "Opening it starts the old version."
                        }
                    }
                    if old_app_error().is_some() {
                        button {
                            class: "kit-banner-link",
                            onclick: {
                                let app = app.clone();
                                move |_| platform::reveal(&app)
                            },
                            "{platform::SHOW_IN_FOLDER}"
                        }
                    } else {
                        button {
                            class: "kit-banner-link",
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
                            class: "kit-banner-link",
                            onclick: move |_| {
                                settings.write().keep_old_app = true;
                                settings.peek().save();
                                old_app.set(None);
                            },
                            "Keep"
                        }
                    }
                    button { class: "kit-banner-close", title: "Dismiss", onclick: move |_| old_app.set(None), "×" }
                }
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
                }
            }
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

fn folder_name(dir: &std::path::Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// start_recording's answer, once the first frame is written.
fn started(a: &Asked) -> Value {
    json!({
        "recording_id": a.id,
        "take": a.id,
        "started_at": a.started_at.map(|t| t.to_rfc3339()),
        "source": a.source_label,
        "max_duration_secs": a.max.as_secs(),
    })
}

/// stop_recording's answer, once the take is saved.
fn finished(a: &Asked, t: &Take, project: Option<&polishcast_core::Project>) -> Value {
    let events: EventLog =
        std::fs::read(t.dir.join(take::EVENTS)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    json!({
        "recording_id": a.id,
        "take": a.id,
        "folder": t.dir,
        "recording_secs": (t.duration * 100.0).round() / 100.0,
        "clicks": events.clicks.len(),
        "zooms_suggested": project.map_or(0, |p| p.zooms.len()),
        "stopped_by": a.stopped_by.unwrap_or("the client"),
    })
}

/// Answers a request from an MCP client (`remote`): starts, stops or cancels its recording, or
/// says what's being recorded.
fn answer(
    request: remote::Request,
    mut status: Signal<Status>,
    picking: bool,
    mut remote: Signal<Remote>,
    settings: Signal<Settings>,
    recording_dir: Signal<Option<PathBuf>>,
    recorder: &Recorder,
) {
    let remote::Request { op, client, reply } = request;
    let current = *status.peek();
    let by_person = remote.peek().current.is_none() && (current != Status::Idle || picking);
    let in_progress = {
        let id = remote.peek().current.as_ref().map(|a| a.id.clone());
        match id {
            Some(id) => {
                format!("A recording is already in progress ({id}). stop_recording or cancel_recording ends it.")
            }
            None => "The person is recording, or choosing what to record, from PolishCast. They stop it from the \
                     menu bar icon; it can't be stopped from here."
                .into(),
        }
    };
    let cancel = matches!(op, Op::Cancel { .. });
    match op {
        Op::Status => {
            let state = match current {
                Status::Idle if picking => "choosing_area",
                Status::Idle => "idle",
                Status::Countdown(_) => "counting_down",
                Status::Starting => "starting",
                Status::Recording(_) => "recording",
                Status::Saving => "saving",
            };
            let mut v = json!({ "state": state, "recording_allowed": Settings::load().allow_mcp_recording });
            if let Some(a) = remote.peek().current.as_ref() {
                v["recording_id"] = json!(a.id);
                v["started_by"] = json!(a.client);
                v["source"] = a.source_label.clone();
                v["max_duration_secs"] = json!(a.max.as_secs());
                v["started_at"] = json!(a.started_at.map(|t| t.to_rfc3339()));
            } else if by_person {
                v["recording_id"] = json!(recording_dir.peek().as_deref().map(folder_name));
                v["started_by"] = json!("the person");
                v["source"] = json!({ "kind": match settings.peek().source {
                    SourceKind::Screen => "display",
                    SourceKind::Window => "window",
                    SourceKind::Area => "region",
                } });
            }
            match current {
                Status::Countdown(until) => {
                    v["countdown_left_secs"] =
                        json!(until.saturating_duration_since(Instant::now()).as_secs_f64().ceil())
                }
                Status::Recording(since) => v["elapsed_secs"] = json!(since.elapsed().as_secs()),
                _ => {}
            }
            if let Some((id, result)) = &remote.peek().last {
                v["last_recording"] = match result {
                    Ok(done) => done.clone(),
                    Err(e) => json!({ "recording_id": id, "failed": e }),
                };
            }
            let _ = reply.send(Ok(v));
        }
        Op::Start(_) if current != Status::Idle || picking => {
            let _ = reply.send(Err(in_progress));
        }
        Op::Start(s) => {
            let dir = match &s.name {
                Some(name) => takes::new_named_dir(name),
                None => takes::new_dir(),
            };
            let dir = match dir {
                Ok(dir) => dir,
                Err(e) => {
                    let _ = reply.send(Err(format!("Can't create the take's folder: {e}")));
                    return;
                }
            };
            let system_audio = s.system_audio.unwrap_or(settings.peek().system_audio);
            let options = Options {
                source: s.source,
                microphone: s.microphone,
                microphone_device: s.microphone_device,
                system_audio,
                ..Options::default()
            };
            remote.write().current = Some(Asked {
                id: folder_name(&dir),
                dir,
                client: remote::client_label(&client),
                source_label: s.source_label,
                max: s.max,
                options: Some(options),
                started_at: None,
                on_started: Some(reply),
                on_saved: Vec::new(),
                stopped_by: None,
                cancel: false,
            });
            // The loop starts it once the countdown is over, the next tick if there is none.
            status.set(Status::Countdown(Instant::now() + s.countdown));
        }
        Op::Stop { id } | Op::Cancel { id } => {
            let mut r = remote.write();
            let Some(a) = r.current.as_mut().filter(|a| a.is(&id)) else {
                let answer = match (&r.current, &r.last) {
                    (Some(a), _) => {
                        Err(format!("no recording {}: the one in progress is {}", id.unwrap_or_default(), a.id))
                    }
                    (None, _) if by_person => Err(in_progress.clone()),
                    (None, Some((last, result))) if !cancel && id.as_ref().is_none_or(|i| i == last) => result.clone(),
                    (None, Some((last, _))) if cancel && id.as_ref().is_none_or(|i| i == last) => {
                        Err(format!("{last} is already saved: it can't be cancelled any more"))
                    }
                    _ => Err("No recording is in progress.".into()),
                };
                let _ = reply.send(answer);
                return;
            };
            match current {
                // Nothing recorded yet: stopping is cancelling.
                Status::Countdown(_) | Status::Idle => {
                    if let Some(mut a) = r.current.take() {
                        a.reply_started(Err("The recording was cancelled during the countdown.".into()));
                        let _ = std::fs::remove_dir(&a.dir);
                        let _ = reply.send(Ok(json!({
                            "cancelled": true,
                            "recording_id": a.id,
                            "note": "Stopped during the countdown: nothing was recorded.",
                        })));
                    }
                    status.set(Status::Idle);
                }
                Status::Starting | Status::Recording(_) => {
                    a.cancel |= cancel;
                    a.stopped_by.get_or_insert(if cancel { "cancel_recording" } else { "stop_recording" });
                    a.on_saved.push(reply);
                    recorder.stop();
                    status.set(Status::Saving);
                }
                Status::Saving => {
                    a.cancel |= cancel;
                    a.on_saved.push(reply);
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
}
