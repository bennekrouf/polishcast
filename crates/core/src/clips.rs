//! Cutting by hand: the recording is split into clips at chosen moments, and clips can be
//! removed from the video (and put back).
//!
//! Splits and removed stretches are in recording time, like blank cuts, and end up on the same
//! `Timeline`: a removed clip is left out exactly as a cut pause is.

/// Splits closer than this to each other, or to either end, would make clips too short to see.
pub const MIN_CLIP_SECS: f64 = 0.1;

/// The clips between `splits` (sorted), from 0 to `duration`.
pub fn clips(splits: &[f64], duration: f64) -> Vec<(f64, f64)> {
    let mut at = 0.0;
    let mut out = Vec::with_capacity(splits.len() + 1);
    for &s in splits.iter().filter(|&&s| s > 0.0 && s < duration) {
        if s > at {
            out.push((at, s));
            at = s;
        }
    }
    out.push((at, duration));
    out
}

/// The clip that holds recording time `t`.
pub fn clip_at(splits: &[f64], duration: f64, t: f64) -> (f64, f64) {
    let all = clips(splits, duration);
    all.iter().copied().find(|&(s, e)| t >= s && t < e).unwrap_or_else(|| *all.last().expect("at least one clip"))
}

/// Adds a split at `t`, unless it would leave a clip shorter than `MIN_CLIP_SECS`. Whether it
/// was added.
pub fn split(splits: &mut Vec<f64>, duration: f64, t: f64) -> bool {
    let too_close =
        t < MIN_CLIP_SECS || t > duration - MIN_CLIP_SECS || splits.iter().any(|&s| (s - t).abs() < MIN_CLIP_SECS);
    if too_close {
        return false;
    }
    splits.push(t);
    splits.sort_by(f64::total_cmp);
    true
}

/// Whether `clip` is left out of the video.
pub fn is_removed(removed: &[(f64, f64)], clip: (f64, f64)) -> bool {
    let mid = (clip.0 + clip.1) / 2.0;
    removed.iter().any(|&(s, e)| s <= mid && mid < e)
}

/// Leaves `clip` out of the video.
pub fn remove(removed: &mut Vec<(f64, f64)>, clip: (f64, f64)) {
    removed.push(clip);
    removed.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Touching or overlapping stretches become one.
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(removed.len());
    for &(s, e) in removed.iter() {
        match merged.last_mut() {
            Some(last) if s <= last.1 + 1e-9 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    *removed = merged;
}

/// Puts `clip` back into the video.
pub fn restore(removed: &mut Vec<(f64, f64)>, clip: (f64, f64)) {
    let (cs, ce) = clip;
    *removed =
        removed.iter().flat_map(|&(s, e)| [(s, e.min(cs)), (s.max(ce), e)]).filter(|(s, e)| e - s > 1e-9).collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_make_clips() {
        assert_eq!(clips(&[], 10.0), vec![(0.0, 10.0)]);
        assert_eq!(clips(&[3.0, 7.0], 10.0), vec![(0.0, 3.0), (3.0, 7.0), (7.0, 10.0)]);
        assert_eq!(clip_at(&[3.0, 7.0], 10.0, 5.0), (3.0, 7.0));
        assert_eq!(clip_at(&[3.0, 7.0], 10.0, 10.0), (7.0, 10.0), "the very end is in the last clip");
    }

    #[test]
    fn a_split_too_close_to_another_or_an_end_is_refused() {
        let mut s = vec![3.0];
        assert!(!split(&mut s, 10.0, 3.05));
        assert!(!split(&mut s, 10.0, 0.05));
        assert!(!split(&mut s, 10.0, 9.95));
        assert!(split(&mut s, 10.0, 1.0));
        assert_eq!(s, vec![1.0, 3.0]);
    }

    #[test]
    fn clips_are_removed_and_put_back() {
        let mut r = Vec::new();
        remove(&mut r, (3.0, 7.0));
        remove(&mut r, (7.0, 10.0));
        assert_eq!(r, vec![(3.0, 10.0)], "neighbours merge");
        assert!(is_removed(&r, (7.0, 10.0)));
        restore(&mut r, (5.0, 7.0));
        assert_eq!(r, vec![(3.0, 5.0), (7.0, 10.0)]);
        assert!(!is_removed(&r, (5.0, 7.0)));
    }
}
