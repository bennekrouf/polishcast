//! A take's edits and look, saved as `project.json` next to the recording.

use crate::blanks::{BlankParams, Cut, Timeline};
use crate::events::EventLog;
use crate::zoom::{auto_zoom, AutoZoom, Zoom};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Length of the recording in seconds.
    pub duration: f64,
    /// Size of the recording in pixels.
    pub width: u32,
    pub height: u32,
    pub zooms: Vec<Zoom>,
    #[serde(default)]
    pub style: Style,
    /// Blank cutting, when turned on: its settings, and the cuts they found (some may be kept).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blanks: Option<BlankParams>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cuts: Vec<Cut>,
    /// How much faster than recorded the video plays: 1.5 shows a minute of recording in 40
    /// seconds. The sound is sped up with it, at the same pitch.
    #[serde(default = "normal_speed", skip_serializing_if = "is_normal_speed")]
    pub speed: f64,
}

/// The speeds the editor offers.
pub const SPEEDS: &[f64] = &[1.0, 1.25, 1.5, 2.0, 3.0];

fn normal_speed() -> f64 {
    1.0
}

fn is_normal_speed(speed: &f64) -> bool {
    *speed == 1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Background {
    Color(String),
    /// Top to bottom, CSS colors.
    Gradient(String, String),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Aspect {
    /// The recording's own shape.
    Source,
    Landscape16x9,
    Portrait9x16,
    Square,
}

impl Aspect {
    /// Width / height of the output, given the recording's.
    pub fn ratio(self, source: f32) -> f32 {
        match self {
            Aspect::Source => source,
            Aspect::Landscape16x9 => 16.0 / 9.0,
            Aspect::Portrait9x16 => 9.0 / 16.0,
            Aspect::Square => 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub background: Background,
    /// Space around the recording, as a fraction of the output's shorter side.
    pub padding: f32,
    /// Corner radius, as a fraction of the output's shorter side.
    pub radius: f32,
    pub shadow: bool,
    pub aspect: Aspect,
    /// Cursor size relative to the system's.
    pub cursor_scale: f32,
    /// Seconds the cursor path is averaged over (0 = drawn as recorded).
    pub cursor_smoothing: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: Background::Gradient("#4f46e5".into(), "#db2777".into()),
            padding: 0.06,
            radius: 0.015,
            shadow: true,
            aspect: Aspect::Source,
            cursor_scale: 1.5,
            cursor_smoothing: 0.08,
        }
    }
}

impl Project {
    /// A fresh project for a new take, with zooms suggested from its clicks.
    pub fn new(duration: f64, width: u32, height: u32, events: &EventLog) -> Self {
        Self {
            duration,
            width,
            height,
            zooms: auto_zoom(&events.clicks, duration, &AutoZoom::default()),
            style: Style::default(),
            blanks: None,
            cuts: Vec::new(),
            speed: 1.0,
        }
    }

    /// The playback speed, kept to what playback and export can do.
    pub fn speed(&self) -> f64 {
        if self.speed.is_finite() {
            self.speed.clamp(0.25, 4.0)
        } else {
            1.0
        }
    }

    /// Length of the exported video: the timeline, at the playback speed.
    pub fn video_duration(&self) -> f64 {
        self.timeline().duration() / self.speed()
    }

    /// The video's timeline: the recording less the blanks cut, if blank cutting is on.
    pub fn timeline(&self) -> Timeline {
        let cuts: &[Cut] = if self.blanks.is_some() { &self.cuts } else { &[] };
        Timeline::new(cuts, self.duration)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_saved_before_speed_existed_play_at_normal_speed() {
        let mut p = Project::new(60.0, 2880, 1800, &EventLog::default());
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("speed"), "1x isn't written");
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap().speed, 1.0);
        p.speed = 1.5;
        assert_eq!(p.video_duration(), 40.0);
        p.speed = f64::NAN;
        assert_eq!(p.speed(), 1.0);
    }

    #[test]
    fn round_trips_through_json() {
        let p = Project::new(12.5, 2880, 1800, &EventLog::default());
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), p);
    }
}
