//! What the app remembers between launches: what to record, whether the Mac's sound goes in
//! with the microphone, how wide the recordings panel is, and whether an MCP client may record.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What a recording takes in. Which window, or which area, is chosen each time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    #[default]
    Screen,
    Window,
    Area,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub source: SourceKind,
    /// Sound played by apps, recorded along with the microphone.
    #[serde(default)]
    pub system_audio: bool,
    /// Asked not to be offered to remove Small Video.app, the app under its old name.
    #[serde(default)]
    pub keep_old_app: bool,
    /// Width of the recordings panel in points, as last dragged. `None`: the default.
    #[serde(default)]
    pub sidebar_width: Option<f64>,
    /// Claude, Codex or another MCP client may start and stop recordings. On unless the person
    /// turned it off in the MCP window.
    #[serde(default = "on")]
    pub allow_mcp_recording: bool,
}

fn on() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source: SourceKind::default(),
            system_audio: false,
            keep_old_app: false,
            sidebar_width: None,
            allow_mcp_recording: true,
        }
    }
}

/// The recordings panel's width when it hasn't been dragged, and the range it can be dragged in.
pub const SIDEBAR_WIDTH: f64 = 260.0;
pub const SIDEBAR_MIN: f64 = 200.0;
pub const SIDEBAR_MAX: f64 = 520.0;

impl Settings {
    /// The recordings panel's width, kept within the range it may be dragged to.
    pub fn sidebar_width(&self) -> f64 {
        self.sidebar_width.filter(|w| w.is_finite()).unwrap_or(SIDEBAR_WIDTH).clamp(SIDEBAR_MIN, SIDEBAR_MAX)
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("Small Video").join("settings.json"))
}

impl Settings {
    /// The saved settings; the defaults if there are none or they can't be read.
    pub fn load() -> Self {
        let mut s: Self = path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Settings carried over from a Mac to a platform that can't record a window or an area.
        if !polishcast_capture::RECORDS_WINDOWS_AND_AREAS {
            s.source = SourceKind::Screen;
        }
        s
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_width_stays_in_range() {
        let mut s = Settings::default();
        assert_eq!(s.sidebar_width(), SIDEBAR_WIDTH);
        s.sidebar_width = Some(5000.0);
        assert_eq!(s.sidebar_width(), SIDEBAR_MAX);
        s.sidebar_width = Some(f64::NAN);
        assert_eq!(s.sidebar_width(), SIDEBAR_WIDTH);
    }

    #[test]
    fn missing_fields_take_the_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
        assert!(s.allow_mcp_recording, "an MCP client may record until turned off");
        let s: Settings = serde_json::from_str(r#"{"source":"Area","system_audio":true}"#).unwrap();
        assert_eq!((s.source, s.system_audio), (SourceKind::Area, true));
    }
}
