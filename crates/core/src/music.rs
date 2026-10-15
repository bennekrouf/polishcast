//! Music under the video. A piece starts and stops at moments of the recording, so it moves
//! with them when pauses are cut, clips are removed or the speed changes; in between it plays
//! straight through at its own speed, never cut or sped up.

use crate::blanks::Timeline;
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
    fn bounds_stay_between_neighbours() {
        let mut music = vec![song(2.0, 4.0), song(6.0, 8.0)];
        set_bounds(&mut music, 1, 3.0, 30.0, 20.0);
        assert_eq!((music[1].start, music[1].end), (4.0, 20.0));
    }
}
