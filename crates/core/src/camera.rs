//! The virtual camera: which part of the screen is on view at a given time.
//!
//! A pure function of `t`, so the preview can scrub anywhere and the export renders exactly
//! the frames the preview showed.

use crate::zoom::Zoom;
use serde::{Deserialize, Serialize};

/// The visible part of the screen, normalized: `(x, y)` is its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Viewport {
    pub const FULL: Viewport = Viewport { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
}

/// Seconds the camera takes to move in or out.
pub const EASE_SECS: f64 = 0.6;

/// What the camera shows at `t`. Zooms are expected in time order and not overlapping.
pub fn viewport_at(zooms: &[Zoom], t: f64) -> Viewport {
    let Some(z) = zooms.iter().find(|z| t >= z.start && t < z.end) else {
        return Viewport::FULL;
    };
    // In over the first EASE_SECS, out over the last; a short zoom never fully arrives.
    let ease = EASE_SECS.min((z.end - z.start) / 2.0);
    let k = smootherstep(((t - z.start) / ease).min((z.end - t) / ease).clamp(0.0, 1.0)) as f32;
    let scale = 1.0 + (z.scale - 1.0) * k;
    let size = 1.0 / scale;
    let center = |focus: f32| (0.5 + (focus - 0.5) * k).clamp(size / 2.0, 1.0 - size / 2.0);
    Viewport { x: center(z.x) - size / 2.0, y: center(z.y) - size / 2.0, w: size, h: size }
}

/// 0 → 1 with zero velocity and acceleration at both ends: no visible jolt when the camera
/// starts or stops.
pub fn smootherstep(x: f64) -> f64 {
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const Z: Zoom = Zoom { start: 10.0, end: 20.0, scale: 2.0, x: 0.9, y: 0.5 };

    #[test]
    fn full_screen_outside_zooms() {
        assert_eq!(viewport_at(&[Z], 5.0), Viewport::FULL);
        assert_eq!(viewport_at(&[Z], 20.0), Viewport::FULL);
    }

    #[test]
    fn fully_zoomed_in_the_middle_and_kept_on_screen() {
        let v = viewport_at(&[Z], 15.0);
        assert_eq!((v.w, v.h), (0.5, 0.5));
        // Centering on x = 0.9 would show past the right edge; the camera stops at it.
        assert_eq!(v.x + v.w, 1.0);
        assert_eq!(v.y, 0.25);
    }

    #[test]
    fn eases_in() {
        let early = viewport_at(&[Z], 10.1).w;
        let later = viewport_at(&[Z], 10.4).w;
        assert!(1.0 > early && early > later && later > 0.5);
    }
}
