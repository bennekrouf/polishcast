//! A take's edits and look, saved as `project.json` next to the recording.

use crate::blanks::{BlankParams, Cut, Timeline};
use crate::events::EventLog;
use crate::music::{self, Music, Placed};
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
    /// Where the recording is split into clips by hand, in recording time, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub splits: Vec<f64>,
    /// Stretches removed by hand (whole clips), in recording time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<(f64, f64)>,
    /// How much faster than recorded the video plays: 1.5 shows a minute of recording in 40
    /// seconds. The sound is sped up with it, at the same pitch.
    #[serde(default = "normal_speed", skip_serializing_if = "is_normal_speed")]
    pub speed: f64,
    /// Music under the video, in order, never overlapping.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub music: Vec<Music>,
    /// While the voice speaks, the music plays at this fraction of its volume; `None` leaves
    /// it as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck: Option<f32>,
    /// Steady background noise (a fan, hum, room hiss) is taken out of the voice at export,
    /// this strongly, from 0 (lightly) to 1; `None` leaves the sound as recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denoise: Option<f32>,
}

/// How strongly background noise is reduced when it's turned on.
pub const DENOISE_STRENGTH: f32 = 0.5;

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
    /// A picture filling the frame (cropped to it, never stretched): the name of a file in the
    /// take's folder, copied there when it was chosen so the take keeps working on its own.
    Image(String),
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
    /// A ring spreads from the cursor at each click. Off in projects from before it existed.
    #[serde(default)]
    pub click_effect: bool,
    /// The cursor fades out while the pointer rests. Off in projects from before it existed.
    #[serde(default)]
    pub hide_idle_cursor: bool,
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
            click_effect: true,
            hide_idle_cursor: true,
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
            splits: Vec::new(),
            removed: Vec::new(),
            speed: 1.0,
            music: Vec::new(),
            duck: None,
            denoise: None,
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

    /// The video's timeline: the recording less the clips removed, and the blanks cut if blank
    /// cutting is on.
    pub fn timeline(&self) -> Timeline {
        let blanks: &[Cut] = if self.blanks.is_some() { &self.cuts } else { &[] };
        let removed = self.removed.iter().map(|&(start, end)| Cut { start, end, keep: false });
        let cuts: Vec<Cut> = blanks.iter().copied().chain(removed).collect();
        Timeline::new(&cuts, self.duration)
    }

    /// Where each piece of music plays in the video.
    pub fn placed_music(&self) -> Vec<Placed> {
        music::place(&self.music, &self.timeline(), self.speed())
    }

    /// Where the voice speaks in the video, from the recording's loudness, at the level blank
    /// cutting takes for silence.
    pub fn speaking(&self, loudness: &crate::Loudness) -> Vec<(f64, f64)> {
        let threshold = self.blanks.unwrap_or_default().threshold_db;
        music::speaking(loudness, threshold, &self.timeline(), self.speed())
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
    fn removed_clips_and_blanks_both_leave_the_video() {
        let mut p = Project::new(20.0, 2880, 1800, &EventLog::default());
        p.removed = vec![(0.0, 2.0)];
        assert_eq!(p.video_duration(), 18.0, "removed clips count with blank cutting off");
        p.cuts = vec![Cut { start: 1.0, end: 5.0, keep: false }, Cut { start: 10.0, end: 11.0, keep: true }];
        p.blanks = Some(BlankParams::default());
        assert_eq!(p.video_duration(), 15.0, "overlaps count once, kept cuts not at all");
    }

    #[test]
    fn round_trips_through_json() {
        let p = Project::new(12.5, 2880, 1800, &EventLog::default());
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), p);
    }
}
