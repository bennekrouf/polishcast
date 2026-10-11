//! The window and the menu bar item.

use crate::editor::Editor;
use crate::{platform, takes};
use dioxus::desktop::tao::event::Event as TaoEvent;
use dioxus::desktop::trayicon::menu::{Menu, MenuItem, PredefinedMenuItem};
use dioxus::desktop::trayicon::{Icon, TrayIcon, TrayIconBuilder};
use dioxus::desktop::{use_global_shortcut, use_muda_event_handler, use_wry_event_handler, window};
use dioxus::prelude::*;
use small_video_capture::{Event, Options, Recorder};
use small_video_core::take;
use small_video_export::compact::compact;
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
        let show = MenuItem::new("Show Small Video", true, None);
        let menu = Menu::new();
        let _ = menu.append_items(&[&record, &show, &PredefinedMenuItem::separator(), &PredefinedMenuItem::quit(None)]);
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_tooltip("Small Video")
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
    let (compacted_tx, compacted_rx) = use_hook(|| {
        let (tx, rx) = mpsc::channel::<(PathBuf, Result<u64, String>)>();
        (tx, Rc::new(rx))
    });

    let toggle = {
        let recorder = recorder.clone();
        use_callback(move |()| match status() {
            Status::Idle => match takes::new_dir() {
                Ok(dir) => {
                    error.set(None);
                    recorder.start(dir, Options::default());
                    status.set(Status::Starting);
                }
                Err(e) => error.set(Some(format!("Can't create the take's folder: {e}"))),
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
            async move {
                loop {
                    while let Some(event) = recorder.try_event() {
                        match event {
                            // Unless Stop was already pressed while it was starting.
                            Event::Started if *status.peek() == Status::Starting => {
                                status.set(Status::Recording(Instant::now()))
                            }
                            Event::Started => {}
                            Event::Finished(t) => {
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
                            }
                            Event::Failed(e) => {
                                error.set(Some(e));
                                status.set(Status::Idle);
                            }
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
        if let Some(info) = update() {
            div { class: "update-banner",
                span { class: "update-banner-text",
                    "Small Video "
                    strong { "{info.latest_version}" }
                    " is available (you have {env!(\"CARGO_PKG_VERSION\")})."
                }
                a { class: "update-banner-link", href: "{info.download_url}", target: "_blank", "Download" }
                button { class: "update-banner-dismiss", title: "Dismiss", onclick: move |_| update.set(None), "×" }
            }
        }
        div { class: "app",
            nav { class: "sidebar",
                header {
                    h1 {
                        "Small Video"
                        // Which build is running, for bug reports and support.
                        span { class: "app-version", {concat!("v", env!("CARGO_PKG_VERSION"))} }
                    }
                    button {
                        class: if recording { "record on" } else { "record" },
                        disabled: status() == Status::Saving,
                        onclick: move |_| toggle(()),
                        "{label}"
                    }
                }
                {
                    let mut open = pro.open;
                    let (label, title) = match *pro.status.read() {
                        crate::licence::Status::Pro(_) => ("Pro \u{2713}", "Small Video Pro is active on this computer"),
                        crate::licence::Status::Renew(_) => ("Renew Pro\u{2026}", "Your Pro updates ended before this version"),
                        crate::licence::Status::Unavailable => ("Pro", "This build exports every take"),
                        crate::licence::Status::Free => ("Get Pro\u{2026}", "Buy Small Video Pro, or paste your licence key"),
                    };
                    rsx! {
                        button { class: "pro-button", title: "{title}", onclick: move |_| open.set(Some(None)), "{label}" }
                    }
                }
                p { class: "hint",
                    if shortcut.is_ok() {
                        "{platform::SHORTCUT_LABEL} or Small Video's icon in the {TRAY_PLACE} records from any app. Its windows are left out."
                    } else {
                        "Small Video's icon in the {TRAY_PLACE} records from any app. Its windows are left out."
                    }
                }
                if let Some(e) = error() {
                    p { class: "error", "{e}" }
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
            crate::licence::ProDialog {}
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

/// Asks before moving a take to the Trash: Enter confirms, Esc cancels (as in Splitter).
#[component]
fn DeleteDialog(dir: PathBuf, on_close: EventHandler<()>, on_confirm: EventHandler<PathBuf>) -> Element {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let exported = dir.with_file_name(format!("{name}.mp4")).is_file();
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
                    if exported { " The video you exported is not touched." }
                }
                div { class: "modal-actions",
                    button { onclick: move |_| on_close.call(()), "Cancel" kbd { "Esc" } }
                    button { class: "danger", onclick: move |_| confirm(), "Move to Trash" kbd { "Enter" } }
                }
            }
        }
    }
}
