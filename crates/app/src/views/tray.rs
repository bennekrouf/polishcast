//! The menu bar item, or notification area icon.

use super::*;

/// The app's mark, black on transparent: macOS tints it to suit the menu bar. Windows and
/// Linux show icons as they are, so they get the colour app icon.
#[cfg(target_os = "macos")]
const TRAY_PNG: &[u8] = include_bytes!("../../assets/tray.png");
#[cfg(not(target_os = "macos"))]
const TRAY_PNG: &[u8] = include_bytes!("../../assets/icon.png");
pub(super) const TRAY_PLACE: &str = if cfg!(target_os = "macos") { "menu bar" } else { "notification area" };

/// The menu bar item: its title shows the state, its menu starts and stops.
pub(super) struct Tray {
    /// `None` when the system would not show one; the window still has every control.
    icon: Option<TrayIcon>,
    pub record: MenuItem,
    pub show: MenuItem,
}

impl Tray {
    pub fn new() -> Self {
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
    pub fn update(&self, status: Status, by: Option<&str>) {
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
