//! Music under the video. A piece starts and stops at moments of the recording, so it moves
//! with them when pauses are cut, clips are removed or the speed changes; in between it plays
//! straight through at its own speed, never cut or sped up.

use crate::blanks::{Loudness, Timeline};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Music {
    /// The song: a file in the take's folder, copied there when it was chosen so the take
    /// keeps working on its own.
    pub file: String,
    /// What the song was called when it was chosen, to show.
    pub name: String,
    /// Where it starts and stops, in recording time.
    pub start: f64,
    pub end: f64,
    /// Where in the song it starts playing, in seconds.
    #[serde(default)]
    pub from: f64,
    /// 1 is the song as it is.
    pub volume: f32,
    /// Seconds it fades in at the start and out at the end.
    pub fade: f64,
}

/// A new piece's volume: under the voice rather than over it.
pub const VOLUME: f32 = 0.3;
/// A new piece's fades, in seconds.
pub const FADE: f64 = 1.0;
/// Shortest piece worth keeping, in seconds of recording.
pub const MIN_SECS: f64 = 0.5;

/// The music's volume while the voice speaks, when it's lowered: a fraction of its own.
pub const DUCK_LEVEL: f32 = 0.3;
/// Seconds the music takes to go down before the voice and back up after it.
pub const DUCK_RAMP: f64 = 0.3;
/// Silences shorter than this, between words, don't bring the music back up.
const DUCK_HOLD: f64 = 0.8;
/// Sounds shorter than this (a click, a cough) don't lower it.
const DUCK_MIN: f64 = 0.15;

/// Where the voice speaks, in video time: louder than `threshold_db`, short gaps bridged, and
/// apart enough that the music has time to come back up in between.
pub fn speaking(loudness: &Loudness, threshold_db: f32, timeline: &Timeline, speed: f64) -> Vec<(f64, f64)> {
    let w = loudness.window;
    let mut runs: Vec<(f64, f64)> = Vec::new();
    for (i, &db) in loudness.db.iter().enumerate() {
        if db < threshold_db {
            continue;
        }
        let (start, end) = (i as f64 * w, (i + 1) as f64 * w);
        match runs.last_mut() {
            Some(last) if start - last.1 < DUCK_HOLD => last.1 = end,
            _ => runs.push((start, end)),
        }
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (start, end) in runs.into_iter().filter(|(s, e)| e - s >= DUCK_MIN) {
        let (start, end) = (timeline.to_output(start) / speed, timeline.to_output(end) / speed);
        if end <= start {
            continue;
        }
        match out.last_mut() {
            Some(last) if start - last.1 < 2.0 * DUCK_RAMP + DUCK_HOLD / speed => last.1 = end,
            _ => out.push((start, end)),
        }
    }
    out
}

/// How far down the music is at video time `t`, from 0 (not at all) to 1 (all the way): it
/// ramps down before each stretch of `speaking` and back up after it.
pub fn ducked(speaking: &[(f64, f64)], t: f64) -> f64 {
    speaking
        .iter()
        .map(|&(s, e)| (((t - s) / DUCK_RAMP + 1.0).min((e - t) / DUCK_RAMP + 1.0)).clamp(0.0, 1.0))
        .fold(0.0, f64::max)
}

/// Where a piece plays in the video, in video seconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Placed {
    /// Which piece of the project's music.
    pub index: usize,
    /// When it starts and stops in the video.
    pub at: f64,
    pub until: f64,
    pub from: f64,
    pub volume: f32,
    /// Fades, shortened to fit when the piece is short.
    pub fade_in: f64,
    pub fade_out: f64,
}

/// Where each piece plays in the video, given its timeline and speed. A piece whose whole
/// stretch is cut doesn't play.
pub fn place(music: &[Music], timeline: &Timeline, speed: f64) -> Vec<Placed> {
    music
        .iter()
        .enumerate()
        .filter_map(|(index, m)| {
            let at = timeline.to_output(m.start) / speed;
            let until = timeline.to_output(m.end) / speed;
            let len = until - at;
            let fade = m.fade.max(0.0).min(len / 2.0);
            (len > 0.05).then_some(Placed {
                index,
                at,
                until,
                from: m.from.max(0.0),
                volume: m.volume.max(0.0),
                fade_in: fade,
                fade_out: fade,
            })
        })
        .collect()
}

/// The piece playing at recording time `t`.
pub fn at(music: &[Music], t: f64) -> Option<usize> {
    music.iter().position(|m| m.start <= t && t < m.end)
}

/// Starts `m` at its start, running until the next piece (or the end of the recording) if it
/// would overlap it; a piece playing there stops where `m` starts. Returns its index, or `None`
/// when there's no room.
pub fn insert(music: &mut Vec<Music>, mut m: Music, duration: f64) -> Option<usize> {
    if let Some(i) = at(music, m.start) {
        if m.start - music[i].start < MIN_SECS {
            return None;
        }
        music[i].end = m.start;
    }
    let i = music.partition_point(|o| o.start <= m.start);
    m.end = m.end.min(music.get(i).map_or(duration, |next| next.start));
    if m.end - m.start < MIN_SECS {
        return None;
    }
    music.insert(i, m);
    Some(i)
}

/// Moves piece `i`'s ends, kept between its neighbours and at least `MIN_SECS` long.
pub fn set_bounds(music: &mut [Music], i: usize, start: f64, end: f64, duration: f64) {
    let lo = if i > 0 { music[i - 1].end } else { 0.0 };
    let hi = music.get(i + 1).map_or(duration, |n| n.start);
    let start = start.clamp(lo, (hi - MIN_SECS).max(lo));
    music[i].start = start;
    music[i].end = end.clamp((start + MIN_SECS).min(hi), hi);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blanks::Cut;

    fn song(start: f64, end: f64) -> Music {
        Music { file: "a.mp3".into(), name: "A".into(), start, end, from: 0.0, volume: 0.5, fade: 1.0 }
    }

    #[test]
    fn pieces_follow_their_moments_through_cuts_and_speed() {
        let timeline = Timeline::new(&[Cut { start: 2.0, end: 6.0, keep: false }], 20.0);
        let placed = place(&[song(8.0, 12.0)], &timeline, 2.0);
        // 8 s of recording is 4 s of video once 4 s are cut, 2 s at twice the speed.
        assert_eq!((placed[0].at, placed[0].until), (2.0, 4.0));
        // A piece inside a cut doesn't play.
        assert!(place(&[song(3.0, 5.0)], &timeline, 1.0).is_empty());
    }

    #[test]
    fn short_pieces_get_short_fades() {
        let timeline = Timeline::new(&[], 10.0);
        let placed = place(&[song(0.0, 1.0)], &timeline, 1.0);
        assert_eq!((placed[0].fade_in, placed[0].fade_out), (0.5, 0.5));
    }

    #[test]
    fn a_new_piece_ends_the_one_playing_and_stops_before_the_next() {
        let mut music = vec![song(2.0, 8.0), song(10.0, 14.0)];
        assert_eq!(insert(&mut music, song(5.0, 20.0), 20.0), Some(1));
        assert_eq!(music[0].end, 5.0, "the piece playing stops there");
        assert_eq!((music[1].start, music[1].end), (5.0, 10.0), "runs up to the next one");
        assert_eq!(insert(&mut music, song(5.2, 20.0), 20.0), None, "too close to its start");
        assert_eq!(music.len(), 3);
    }

    #[test]
    fn the_music_goes_down_while_the_voice_speaks() {
        // Speech 1–2 s and 2.5–3 s (a short pause), then again at 8–9 s.
        let mut db = vec![-70.0f32; 200];
        for i in (20..40).chain(50..60).chain(160..180) {
            db[i] = -20.0;
        }
        // A click: too short to count.
        db[100] = -10.0;
        let loudness = Loudness { window: 0.05, db };
        let timeline = Timeline::new(&[], 10.0);
        let spans = speaking(&loudness, -45.0, &timeline, 1.0);
        assert_eq!(spans, vec![(1.0, 3.0), (8.0, 9.0)]);
        assert_eq!(ducked(&spans, 2.0), 1.0);
        assert!((ducked(&spans, 0.85) - 0.5).abs() < 1e-9, "half way down before the voice");
        assert_eq!(ducked(&spans, 5.0), 0.0, "back up in between");
        // At twice the speed, in video time.
        assert_eq!(speaking(&loudness, -45.0, &timeline, 2.0), vec![(0.5, 1.5), (4.0, 4.5)]);
    }

    #[test]
    fn bounds_stay_between_neighbours() {
        let mut music = vec![song(2.0, 4.0), song(6.0, 8.0)];
        set_bounds(&mut music, 1, 3.0, 30.0, 20.0);
        assert_eq!((music[1].start, music[1].end), (4.0, 20.0));
    }
}
