//! How to record, remembered between launches: what to record and whether the Mac's sound goes
//! in with the microphone.

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub source: SourceKind,
    /// Sound played by apps, recorded along with the microphone.
    #[serde(default)]
    pub system_audio: bool,
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
        if !small_video_capture::RECORDS_WINDOWS_AND_AREAS {
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
    fn missing_fields_take_the_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
        let s: Settings = serde_json::from_str(r#"{"source":"Area","system_audio":true}"#).unwrap();
        assert_eq!((s.source, s.system_audio), (SourceKind::Area, true));
    }
}
