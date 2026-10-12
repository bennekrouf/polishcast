//! The drawn cursor: smoothing (the logged path is jittery and steps at the sampling rate; the
//! drawn cursor should glide), fading out while the pointer rests, and the ring at each click.

use crate::events::{Click, CursorSample};

/// Seconds the pointer has to rest before the cursor fades out.
pub const IDLE_SECS: f64 = 1.5;
/// Seconds the cursor takes to fade out, and to fade back in before the pointer moves again.
pub const FADE_SECS: f64 = 0.3;
/// Seconds a click's ring takes to spread and fade.
pub const CLICK_SECS: f64 = 0.5;
/// A move shorter than this (a fraction of the screen) is the hand trembling, not moving.
const STILL: f32 = 0.002;

/// When the pointer was busy: each time it moved away from where it rested, and each click.
/// Sorted. The start of the take counts, so the cursor shows at first.
pub fn activity(samples: &[CursorSample], clicks: &[Click]) -> Vec<f64> {
    let mut out = vec![0.0];
    let mut rest = samples.first().map(|s| (s.x, s.y));
    for s in samples {
        if let Some((x, y)) = rest {
            if (s.x - x).hypot(s.y - y) > STILL {
                out.push(s.t);
                rest = Some((s.x, s.y));
            }
        }
    }
    out.extend(clicks.iter().map(|c| c.t));
    out.sort_by(f64::total_cmp);
    out
}

/// How visible the cursor is at `t` (0 to 1), given `activity`: shown while the pointer is
/// busy, faded out once it has rested `IDLE_SECS`, and faded back in just before it moves
/// again, so it never pops up already moving.
pub fn idle_opacity(activity: &[f64], t: f64) -> f32 {
    let i = activity.partition_point(|&a| a <= t);
    let last = i.checked_sub(1).map_or(0.0, |p| activity[p]);
    let fading_out = 1.0 - ((t - last - IDLE_SECS) / FADE_SECS).clamp(0.0, 1.0);
    let fading_in = activity.get(i).map_or(0.0, |&next| 1.0 - ((next - t) / FADE_SECS).clamp(0.0, 1.0));
    smoothstep(fading_out.max(fading_in)) as f32
}

/// The most recent click whose ring is still showing at `t`, and how far along it is (0 to 1).
pub fn click_at(clicks: &[Click], t: f64) -> Option<(&Click, f64)> {
    let i = clicks.partition_point(|c| c.t <= t);
    let c = clicks.get(i.checked_sub(1)?)?;
    let k = (t - c.t) / CLICK_SECS;
    (k < 1.0).then_some((c, k))
}

fn smoothstep(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

/// The path averaged over `±window` seconds around each sample. Centered, so the smoothed
/// cursor doesn't lag behind the real one; the first and last samples stay where they are.
pub fn smooth(samples: &[CursorSample], window: f64) -> Vec<CursorSample> {
    let (mut lo, mut hi) = (0, 0);
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if i == 0 || i == samples.len() - 1 {
                return *s;
            }
            while hi < samples.len() && samples[hi].t <= s.t + window {
                sx += samples[hi].x as f64;
                sy += samples[hi].y as f64;
                hi += 1;
            }
            while samples[lo].t < s.t - window {
                sx -= samples[lo].x as f64;
                sy -= samples[lo].y as f64;
                lo += 1;
            }
            let n = (hi - lo) as f64;
            CursorSample { t: s.t, x: (sx / n) as f32, y: (sy / n) as f32 }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Button;

    fn still_then_move() -> Vec<CursorSample> {
        // Resting at the middle for 5 s, then moving right.
        (0..=420)
            .map(|i| {
                let t = i as f64 / 60.0;
                let x = if t < 5.0 { 0.5 } else { 0.5 + (t - 5.0) as f32 * 0.1 };
                CursorSample { t, x, y: 0.5 }
            })
            .collect()
    }

    #[test]
    fn the_cursor_fades_out_at_rest_and_back_in_before_it_moves() {
        let a = activity(&still_then_move(), &[]);
        assert_eq!(idle_opacity(&a, 0.5), 1.0, "shown at the start");
        assert_eq!(idle_opacity(&a, IDLE_SECS + FADE_SECS + 0.1), 0.0, "faded out at rest");
        let before = idle_opacity(&a, 5.0 - FADE_SECS / 2.0);
        assert!(before > 0.0 && before < 1.0, "fading in ahead of the move: {before}");
        assert_eq!(idle_opacity(&a, 5.5), 1.0, "shown while moving");
    }

    #[test]
    fn a_click_keeps_the_cursor_shown_and_rings() {
        let click = Click { t: 3.0, x: 0.5, y: 0.5, button: Button::Left };
        let a = activity(&still_then_move(), &[click]);
        assert_eq!(idle_opacity(&a, 3.0), 1.0);
        let clicks = [click];
        assert!(click_at(&clicks, 2.9).is_none());
        let (_, k) = click_at(&clicks, 3.0 + CLICK_SECS / 2.0).unwrap();
        assert!((k - 0.5).abs() < 1e-9);
        assert!(click_at(&clicks, 3.0 + CLICK_SECS).is_none());
    }

    #[test]
    fn trembling_is_not_moving() {
        let path: Vec<_> =
            (0..600).map(|i| CursorSample { t: i as f64 / 60.0, x: 0.5 + (i % 2) as f32 * 0.001, y: 0.5 }).collect();
        assert_eq!(activity(&path, &[]), vec![0.0]);
    }

    #[test]
    fn a_still_cursor_stays_put() {
        let path: Vec<_> = (0..50).map(|i| CursorSample { t: i as f64 / 60.0, x: 0.3, y: 0.7 }).collect();
        for s in smooth(&path, 0.05) {
            assert!((s.x - 0.3).abs() < 1e-6 && (s.y - 0.7).abs() < 1e-6);
        }
    }

    #[test]
    fn jitter_is_reduced() {
        let path: Vec<_> = (0..120)
            .map(|i| CursorSample { t: i as f64 / 60.0, x: if i % 2 == 0 { 0.49 } else { 0.51 }, y: 0.5 })
            .collect();
        let out = smooth(&path, 0.05);
        for s in &out[10..110] {
            assert!((s.x - 0.5).abs() < 0.004, "{}", s.x);
        }
    }
}
