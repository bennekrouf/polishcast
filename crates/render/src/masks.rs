//! Covers hidden areas on a frame of the recording, before anything else is done with it.
//!
//! Doing it to the frame itself, rather than over the finished picture, is what makes it hold:
//! the camera, the dissolve after a cut and every format are then drawn from pixels that no
//! longer have the area in them.

use polishcast_core::mask::{Cover, Mask};

/// The solid cover: the dark the app's backgrounds use.
const SOLID: [u8; 4] = [24, 24, 27, 255];
/// How many cells of frosted glass fit in the frame's height. About two lines of text to a cell
/// on a usual screen: the blur keeps one colour per cell and nothing else of what's under it.
const CELLS_TALL: usize = 24;
const MIN_CELL: usize = 8;

/// Covers on `rgba`, a frame of `width` × `height` of the recording at time `t`, every mask on
/// at that moment.
pub fn paint(rgba: &mut [u8], width: u32, height: u32, masks: &[Mask], t: f64) {
    let (w, h) = (width as usize, height as usize);
    if rgba.len() != w * h * 4 {
        return;
    }
    for m in masks.iter().filter(|m| m.covers(t)) {
        let area = Area::of(m, w, h);
        match m.cover {
            Cover::Solid => solid(rgba, w, &area),
            Cover::Blur => frost(rgba, w, h, &area),
        }
    }
}

/// A mask in the frame's pixels, one pixel larger each way so that what the frame's scaling
/// smeared past the edge is covered too.
struct Area {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Area {
    fn of(m: &Mask, w: usize, h: usize) -> Self {
        let low = |v: f32, size: usize| ((v * size as f32).floor() as usize).saturating_sub(1).min(size);
        let high = |v: f32, size: usize| ((v * size as f32).ceil() as usize + 1).min(size);
        Self { x0: low(m.x, w), y0: low(m.y, h), x1: high(m.x + m.w, w), y1: high(m.y + m.h, h) }
    }
}

fn solid(rgba: &mut [u8], w: usize, a: &Area) {
    for row in rgba.chunks_exact_mut(w * 4).take(a.y1).skip(a.y0) {
        row[a.x0 * 4..a.x1 * 4].as_chunks_mut::<4>().0.fill(SOLID);
    }
}

/// Replaces the area by a few averaged colours, blended smoothly into one another. What is
/// drawn depends only on the cells' averages, so nothing finer than a cell survives.
fn frost(rgba: &mut [u8], w: usize, h: usize, a: &Area) {
    let cell = (h / CELLS_TALL).max(MIN_CELL);
    let (cols, rows) = ((a.x1 - a.x0).div_ceil(cell), (a.y1 - a.y0).div_ceil(cell));
    if cols == 0 || rows == 0 {
        return;
    }
    let mut sums = vec![[0u32; 4]; cols * rows];
    for y in a.y0..a.y1 {
        for x in a.x0..a.x1 {
            let s = &mut sums[(y - a.y0) / cell * cols + (x - a.x0) / cell];
            let px = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 3];
            for (sum, &v) in s.iter_mut().zip(px) {
                *sum += v as u32;
            }
            s[3] += 1;
        }
    }
    let colors: Vec<[f32; 3]> = sums.iter().map(|s| [0, 1, 2].map(|c| s[c] as f32 / s[3].max(1) as f32)).collect();
    // Between the centres of the cells around it.
    let around = |at: usize, count: usize| {
        let f = ((at as f32 + 0.5) / cell as f32 - 0.5).clamp(0.0, (count - 1) as f32);
        let low = f as usize;
        (low, (low + 1).min(count - 1), f - low as f32)
    };
    for y in a.y0..a.y1 {
        let (r0, r1, v) = around(y - a.y0, rows);
        for x in a.x0..a.x1 {
            let (c0, c1, u) = around(x - a.x0, cols);
            let px = &mut rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
            for c in 0..3 {
                let top = colors[r0 * cols + c0][c] * (1.0 - u) + colors[r0 * cols + c1][c] * u;
                let bottom = colors[r1 * cols + c0][c] * (1.0 - u) + colors[r1 * cols + c1][c] * u;
                px[c] = (top * (1.0 - v) + bottom * v).round() as u8;
            }
            px[3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 200;
    const H: usize = 192;

    fn frame(color: u8) -> Vec<u8> {
        vec![color; W * H * 4]
    }

    fn at(rgba: &[u8], x: usize, y: usize) -> [u8; 3] {
        let i = (y * W + x) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }

    fn mask(cover: Cover, x: f32, y: f32, w: f32, h: f32) -> Mask {
        Mask { start: 1.0, end: 2.0, x, y, w, h, cover }
    }

    #[test]
    fn a_solid_mask_replaces_only_its_area_while_it_is_on() {
        let masks = [mask(Cover::Solid, 0.25, 0.25, 0.5, 0.5)];
        let mut f = frame(200);
        paint(&mut f, W as u32, H as u32, &masks, 1.5);
        assert_eq!(at(&f, 100, 96), [24, 24, 27]);
        assert_eq!(at(&f, 10, 10), [200, 200, 200], "outside");
        let mut later = frame(200);
        paint(&mut later, W as u32, H as u32, &masks, 3.0);
        assert_eq!(at(&later, 100, 96), [200, 200, 200], "after it ends");
    }

    #[test]
    fn its_edges_are_rounded_outward() {
        let mut f = frame(200);
        paint(&mut f, W as u32, H as u32, &[mask(Cover::Solid, 0.5, 0.5, 0.1, 0.1)], 1.5);
        assert_eq!(at(&f, 99, 95), [24, 24, 27], "the pixel before the edge");
        assert_eq!(at(&f, 98, 94), [200, 200, 200]);
    }

    #[test]
    fn a_mask_at_the_frames_edge_stays_in_it() {
        let mut f = frame(200);
        paint(&mut f, W as u32, H as u32, &[mask(Cover::Blur, 0.0, 0.0, 1.0, 1.0)], 1.5);
        paint(&mut f, W as u32, H as u32, &[mask(Cover::Solid, 0.9, 0.9, 0.1, 0.1)], 1.5);
        assert_eq!(at(&f, W - 1, H - 1), [24, 24, 27]);
    }

    #[test]
    fn blur_keeps_nothing_finer_than_a_cell() {
        // Two different patterns inside the cells (8 px at this height), the same average:
        // the result can't tell them apart.
        let masks = [mask(Cover::Blur, 0.0, 0.0, 1.0, 1.0)];
        let (mut a, mut b) = (frame(0), frame(0));
        for y in 0..H {
            for x in 0..W {
                let i = (y * W + x) * 4;
                a[i..i + 3].fill(if x % 2 == 0 { 255 } else { 0 });
                b[i..i + 3].fill(if y % 2 == 0 { 255 } else { 0 });
            }
        }
        assert_ne!(a, b);
        paint(&mut a, W as u32, H as u32, &masks, 1.5);
        paint(&mut b, W as u32, H as u32, &masks, 1.5);
        assert_eq!(a, b);
    }

    #[test]
    fn a_frame_of_the_wrong_size_is_left_alone() {
        let mut f = vec![200; 16];
        paint(&mut f, W as u32, H as u32, &[mask(Cover::Solid, 0.0, 0.0, 1.0, 1.0)], 1.5);
        assert_eq!(f, vec![200; 16]);
    }
}
