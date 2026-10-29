//! Blank cutting: stretches where nothing is said, left out of the video.
//!
//! Detection is TrackCut's (`trackcut-core/src/detect.rs`): the loudness track is the RMS level
//! in dBFS per fixed window, a run of windows below the threshold that lasts long enough is
//! a silence, and a margin is kept next to the sound so words aren't clipped. Here a silence
//! becomes a cut instead of a split between tracks.
//!
//! Cuts are in recording time. `Timeline` maps the video's (output) time to recording time
//! and back; zooms, the cursor and everything else stay in recording time.

use serde::{Deserialize, Serialize};

/// The recording's loudness: RMS in dBFS over consecutive windows of `window` seconds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Loudness {
    pub window: f64,
    pub db: Vec<f32>,
}

impl Loudness {
    /// The level of the steady background noise (a fan, hum, room hiss): what the quietest
    /// tenth of the recording sits at, between words and in the pauses. Digital silence (the
    /// gap before the microphone starts) doesn't count. `None` when all of it is silent.
    pub fn noise_floor(&self) -> Option<f32> {
        let mut heard: Vec<f32> = self.db.iter().copied().filter(|d| *d > -100.0).collect();
        if heard.is_empty() {
            return None;
        }
        heard.sort_by(f32::total_cmp);
        Some(heard[heard.len() / 10])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlankParams {
    /// Quieter than this counts as silence (dBFS, RMS over a window).
    pub threshold_db: f32,
    /// Pauses shorter than this are part of speaking and stay.
    pub min_pause_secs: f64,
    /// Silence kept next to the sound on each side of a cut.
    pub margin_secs: f64,
}

impl Default for BlankParams {
    fn default() -> Self {
        // TrackCut's threshold; a shorter pause than its 1.5 s, since this is speech.
        Self { threshold_db: -45.0, min_pause_secs: 1.0, margin_secs: 0.25 }
    }
}

/// A stretch of the recording, `[start, end)` in seconds, left out of the video unless `keep`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cut {
    pub start: f64,
    pub end: f64,
    /// The user put this one back.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
}

/// The silences in `loudness` as cuts within `[0, duration]`. Lead-in and tail silence are cut
/// right to the start and end; elsewhere `margin_secs` of silence stays next to the sound.
pub fn detect(loudness: &Loudness, duration: f64, p: &BlankParams) -> Vec<Cut> {
    let w = loudness.window;
    if w <= 0.0 {
        return Vec::new();
    }
    let min_windows = (p.min_pause_secs / w).ceil().max(1.0) as usize;
    let n = loudness.db.len();
    let mut cuts = Vec::new();
    let mut run_start = None;
    for (i, &d) in loudness.db.iter().chain(std::iter::once(&f32::INFINITY)).enumerate() {
        match (d < p.threshold_db, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(s)) => {
                if i - s >= min_windows {
                    let start = if s == 0 { 0.0 } else { s as f64 * w + p.margin_secs };
                    let end = if i >= n { duration } else { (i as f64 * w - p.margin_secs).min(duration) };
                    if end > start {
                        cuts.push(Cut { start, end, keep: false });
                    }
                }
                run_start = None;
            }
            _ => {}
        }
    }
    cuts
}

/// Fresh cuts for new settings, keeping the user's choices: a new cut whose middle falls in a
/// cut they kept stays kept.
pub fn redetect(old: &[Cut], loudness: &Loudness, duration: f64, p: &BlankParams) -> Vec<Cut> {
    let mut cuts = detect(loudness, duration, p);
    for c in &mut cuts {
        let mid = (c.start + c.end) / 2.0;
        c.keep = old.iter().any(|o| o.keep && o.start <= mid && mid < o.end);
    }
    cuts
}

/// The video's timeline: the recording minus the cuts that aren't kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    /// Kept stretches of the recording, in order: `(start, end)` in recording time.
    pub parts: Vec<(f64, f64)>,
}

impl Timeline {
    pub fn new(cuts: &[Cut], duration: f64) -> Self {
        let mut active: Vec<(f64, f64)> = cuts
            .iter()
            .filter(|c| !c.keep)
            .map(|c| (c.start.max(0.0), c.end.min(duration)))
            .filter(|(s, e)| e > s)
            .collect();
        active.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut parts = Vec::new();
        let mut at = 0.0;
        for (s, e) in active {
            if s > at {
                parts.push((at, s));
            }
            at = at.max(e);
        }
        if duration > at {
            parts.push((at, duration));
        }
        Self { parts }
    }

    /// Length of the video.
    pub fn duration(&self) -> f64 {
        self.parts.iter().map(|(s, e)| e - s).sum()
    }

    /// Recording time shown at video time `t`.
    pub fn to_source(&self, t: f64) -> f64 {
        let mut left = t.max(0.0);
        for &(s, e) in &self.parts {
            if left < e - s {
                return s + left;
            }
            left -= e - s;
        }
        self.parts.last().map_or(t, |p| p.1)
    }

    /// Video time of recording time `t`; inside a cut, where the video resumes.
    pub fn to_output(&self, t: f64) -> f64 {
        let mut out = 0.0;
        for &(s, e) in &self.parts {
            if t < s {
                return out;
            }
            if t < e {
                return out + (t - s);
            }
            out += e - s;
        }
        out
    }

    /// The stretches left out, in recording time: the gaps between the parts.
    pub fn skips(&self, duration: f64) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let mut at = 0.0;
        for &(s, e) in &self.parts {
            if s > at {
                out.push((at, s));
            }
            at = e;
        }
        if duration > at {
            out.push((at, duration));
        }
        out
    }

    /// Whether recording time `t` is shown.
    pub fn shows(&self, t: f64) -> bool {
        self.parts.iter().any(|&(s, e)| s <= t && t < e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pattern` of (seconds, dB) segments, at 20 windows per second.
    fn loudness(pattern: &[(f64, f32)]) -> Loudness {
        let db = pattern.iter().flat_map(|&(s, d)| std::iter::repeat_n(d, (s * 20.0).round() as usize)).collect();
        Loudness { window: 0.05, db }
    }

    fn bounds(cuts: &[Cut]) -> Vec<(f64, f64)> {
        cuts.iter().map(|c| ((c.start * 1000.0).round() / 1000.0, (c.end * 1000.0).round() / 1000.0)).collect()
    }

    #[test]
    fn the_noise_floor_is_the_quiet_between_words() {
        // Digital silence first, then speech with short pauses at the room's -62 dB.
        let l = loudness(&[(1.0, -200.0), (3.0, -20.0), (0.5, -62.0), (3.0, -25.0), (0.5, -62.0)]);
        assert_eq!(l.noise_floor(), Some(-62.0));
        assert_eq!(loudness(&[(2.0, -200.0)]).noise_floor(), None);
    }

    #[test]
    fn pauses_become_cuts_with_margins() {
        let l = loudness(&[
            (2.0, -80.0),
            (5.0, -20.0),
            (2.0, -70.0),
            (5.0, -20.0),
            (0.5, -70.0),
            (3.0, -20.0),
            (2.0, -80.0),
        ]);
        let cuts = detect(&l, 19.5, &BlankParams::default());
        // Lead-in cut to the very start, the 2 s pause less 0.25 s each side, the 0.5 s pause
        // kept (too short), the tail cut to the very end.
        assert_eq!(bounds(&cuts), [(0.0, 1.75), (7.25, 8.75), (17.75, 19.5)]);
    }

    #[test]
    fn kept_cuts_survive_new_settings() {
        let l = loudness(&[(5.0, -20.0), (3.0, -70.0), (5.0, -20.0)]);
        let mut cuts = detect(&l, 13.0, &BlankParams::default());
        cuts[0].keep = true;
        let p = BlankParams { margin_secs: 0.1, ..BlankParams::default() };
        let again = redetect(&cuts, &l, 13.0, &p);
        assert_eq!(bounds(&again), [(5.1, 7.9)]);
        assert!(again[0].keep);
    }

    #[test]
    fn the_timeline_skips_cuts() {
        let cuts = [
            Cut { start: 0.0, end: 1.0, keep: false },
            Cut { start: 4.0, end: 6.0, keep: false },
            Cut { start: 7.0, end: 8.0, keep: true },
        ];
        let tl = Timeline::new(&cuts, 10.0);
        assert_eq!(tl.parts, [(1.0, 4.0), (6.0, 10.0)]);
        assert_eq!(tl.duration(), 7.0);
        assert_eq!(tl.to_source(0.0), 1.0);
        assert_eq!(tl.to_source(3.5), 6.5);
        assert_eq!(tl.to_output(6.5), 3.5);
        assert_eq!(tl.to_output(5.0), 3.0, "inside a cut: where the video resumes");
        assert!(tl.shows(7.5) && !tl.shows(5.0));
        assert_eq!(tl.skips(10.0), [(0.0, 1.0), (4.0, 6.0)]);
        assert_eq!(Timeline::new(&[], 10.0).parts, [(0.0, 10.0)]);
    }
}
