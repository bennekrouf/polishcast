//! Hidden areas: a rectangle of the screen covered between two moments, so something
//! confidential on screen doesn't reach the video.
//!
//! A mask is kept in the recording's coordinates and time, like a zoom, so it stays on what it
//! covers while the camera moves and whatever is cut around it.

use serde::{Deserialize, Serialize};

/// Seconds a mask stays on before its start and after its end. Better covered a moment too long
/// than a frame too short: an export reads the recording at its own frame rate, so the frame it
/// takes for a moment can be up to half a frame period away, 0.2 s at the slowest (a 10 fps GIF
/// at the highest speed).
pub const MARGIN_SECS: f64 = 0.2;
/// Shortest time a mask lasts.
pub const MIN_SECS: f64 = 0.1;
/// Smallest side of a mask, as a fraction of the screen.
pub const MIN_SIZE: f32 = 0.005;

/// What is drawn over the area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Cover {
    /// An opaque box: nothing of what's under it is left in the video.
    #[default]
    Solid,
    /// A coarse frosted glass, only a few colours across: softer to look at.
    Blur,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub start: f64,
    pub end: f64,
    /// Top-left corner, normalized to the screen.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub cover: Cover,
}

impl Mask {
    /// A mask kept inside the take and the screen.
    pub fn new(start: f64, end: f64, (x, y, w, h): (f32, f32, f32, f32), cover: Cover, duration: f64) -> Self {
        let mut m = Self { start, end, x, y, w, h, cover };
        m.set_rect(x, y, w, h);
        m.set_bounds(start, end, duration);
        m
    }

    /// First moment it's drawn at, margin included.
    pub fn from(&self) -> f64 {
        self.start - MARGIN_SECS
    }

    /// Last moment it's drawn at, margin included.
    pub fn until(&self) -> f64 {
        self.end + MARGIN_SECS
    }

    pub fn covers(&self, t: f64) -> bool {
        t >= self.from() && t <= self.until()
    }

    /// Moves its edges in time, inside the take and at least `MIN_SECS` long.
    pub fn set_bounds(&mut self, start: f64, end: f64, duration: f64) {
        self.start = start.clamp(0.0, (duration - MIN_SECS).max(0.0));
        self.end = end.clamp((self.start + MIN_SECS).min(duration), duration);
    }

    /// Moves it on the screen: on the screen whole, and not too small to see.
    pub fn set_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.w = w.clamp(MIN_SIZE, 1.0);
        self.h = h.clamp(MIN_SIZE, 1.0);
        self.x = x.clamp(0.0, 1.0 - self.w);
        self.y = y.clamp(0.0, 1.0 - self.h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(start: f64, end: f64) -> Mask {
        Mask::new(start, end, (0.1, 0.1, 0.2, 0.2), Cover::Solid, 10.0)
    }

    #[test]
    fn it_stays_on_a_moment_past_both_ends() {
        let m = mask(2.0, 4.0);
        assert!(m.covers(2.0 - MARGIN_SECS) && m.covers(4.0 + MARGIN_SECS));
        assert!(!m.covers(1.7) && !m.covers(4.3));
    }

    #[test]
    fn times_stay_inside_the_take_and_in_order() {
        assert_eq!((mask(-1.0, 20.0).start, mask(-1.0, 20.0).end), (0.0, 10.0));
        let backwards = mask(5.0, 3.0);
        assert!(backwards.end - backwards.start >= MIN_SECS - 1e-9);
        let at_the_end = mask(10.0, 12.0);
        assert!(at_the_end.start < at_the_end.end && at_the_end.end == 10.0);
    }

    #[test]
    fn the_rectangle_stays_on_the_screen() {
        let m = Mask::new(0.0, 1.0, (0.9, -0.5, 0.5, 3.0), Cover::Blur, 10.0);
        assert_eq!((m.y, m.h), (0.0, 1.0));
        assert!((m.x + m.w - 1.0).abs() < 1e-6, "pushed back on: {m:?}");
        let tiny = Mask::new(0.0, 1.0, (0.5, 0.5, 0.0, 0.0), Cover::Solid, 10.0);
        assert_eq!((tiny.w, tiny.h), (MIN_SIZE, MIN_SIZE));
    }
}
