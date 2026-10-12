//! Draws output frames on the CPU (tiny-skia): background and shadow, the recording seen through
//! the camera's viewport with rounded corners, and the cursor on top.
//!
//! What doesn't change from frame to frame (background, shadow, the corner mask) is drawn once.

use crate::{Frame, Scene};
use small_video_core::project::Background;
use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, IntSize, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap,
    PixmapPaint, PixmapRef, Point, SpreadMode, Stroke, Transform,
};

/// A background picture, decoded for drawing. PNG or JPEG.
pub fn load_image(path: &std::path::Path) -> Result<Pixmap, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.into_rgba8();
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    // tiny-skia wants premultiplied alpha.
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u16;
        for v in &mut px[..3] {
            *v = ((*v as u16 * a + 127) / 255) as u8;
        }
    }
    let size = IntSize::from_wh(w, h).ok_or("The picture is empty")?;
    Pixmap::from_vec(data, size).ok_or_else(|| "The picture can't be drawn".to_string())
}

/// The pointer's outline, tip near (1, 1), in a 20 × 28 box. The preview draws the same shape.
pub const CURSOR_POINTS: [(f32, f32); 7] =
    [(1.0, 1.0), (1.0, 22.0), (6.5, 16.8), (10.2, 25.6), (13.6, 24.1), (9.9, 15.4), (17.0, 15.4)];
/// Height of the box `CURSOR_POINTS` is drawn in; a cursor's `size` maps to it.
pub const CURSOR_BOX: f32 = 28.0;
pub const CURSOR_STROKE: f32 = 1.6;
/// A click ring's line width, as a fraction of its radius. The preview draws the same.
pub const RIPPLE_STROKE: f32 = 0.14;

/// The cursor outline as an SVG path, for the preview.
pub fn cursor_svg_path() -> String {
    let mut d = String::new();
    for (i, (x, y)) in CURSOR_POINTS.iter().enumerate() {
        d += &format!("{}{x} {y} ", if i == 0 { "M" } else { "L" });
    }
    d + "Z"
}

pub struct Compositor {
    backdrop: Pixmap,
    /// Where the recording may draw: the content rectangle with rounded corners.
    clip: Mask,
    cursor: Path,
}

impl Compositor {
    /// For frames of `width` × `height`. Layout that doesn't change over time is taken from the
    /// scene's first frame.
    pub fn new(scene: &Scene, width: u32, height: u32) -> Self {
        Self::with_image(scene, width, height, None)
    }

    /// Like `new`, with the picture a `Background::Image` names (from `load_image`). Without
    /// one, such a background is drawn dark.
    pub fn with_image(scene: &Scene, width: u32, height: u32, image: Option<PixmapRef>) -> Self {
        let first = scene.frame(0.0, width, height);
        let c = first.content;
        let content_path = rounded_rect(c.x, c.y, c.w, c.h, first.radius);

        let mut backdrop = Pixmap::new(width, height).expect("frame size");
        let mut paint = Paint { anti_alias: true, ..Default::default() };
        paint.shader = match &scene.project.style.background {
            Background::Color(c) => tiny_skia::Shader::SolidColor(parse_color(c)),
            Background::Gradient(a, b) => LinearGradient::new(
                Point::from_xy(0.0, 0.0),
                Point::from_xy(0.0, height as f32),
                vec![GradientStop::new(0.0, parse_color(a)), GradientStop::new(1.0, parse_color(b))],
                SpreadMode::Pad,
                Transform::identity(),
            )
            .expect("two stops"),
            Background::Image(_) => tiny_skia::Shader::SolidColor(parse_color("#18181b")),
        };
        backdrop.fill_rect(
            tiny_skia::Rect::from_xywh(0.0, 0.0, width as f32, height as f32).expect("frame size"),
            &paint,
            Transform::identity(),
            None,
        );
        if let (Background::Image(_), Some(img)) = (&scene.project.style.background, image) {
            // Cover the frame, centred: scaled until both sides fill it, the overflow cut off.
            let (iw, ih) = (img.width() as f32, img.height() as f32);
            let k = (width as f32 / iw).max(height as f32 / ih);
            let at = Transform::from_row(k, 0.0, 0.0, k, (width as f32 - iw * k) / 2.0, (height as f32 - ih * k) / 2.0);
            let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..Default::default() };
            backdrop.draw_pixmap(0, 0, img, &paint, at, None);
        }

        let mut clip = Mask::new(width, height).expect("frame size");
        clip.fill_path(&content_path, FillRule::Winding, true, Transform::identity());

        if let Some(s) = first.shadow {
            let mut shade = Mask::new(width, height).expect("frame size");
            shade.fill_path(&content_path, FillRule::Winding, true, Transform::from_translate(0.0, s.dy));
            let alpha = blur(shade.data(), width as usize, height as usize, s.sigma.round().max(1.0) as usize);
            darken(backdrop.data_mut(), &alpha, s.alpha);
        }

        let mut pb = PathBuilder::new();
        for (i, &(x, y)) in CURSOR_POINTS.iter().enumerate() {
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        pb.close();
        Self { backdrop, clip, cursor: pb.finish().expect("cursor path") }
    }

    /// Width and height of the frames it draws.
    pub fn size(&self) -> (u32, u32) {
        (self.backdrop.width(), self.backdrop.height())
    }

    /// Draws `frame` into `out` (same size as the compositor), with `source` the recording's
    /// frame at any resolution.
    pub fn draw(&self, frame: &Frame, source: PixmapRef, out: &mut Pixmap) {
        out.data_mut().copy_from_slice(self.backdrop.data());

        let (c, v) = (frame.content, frame.source);
        let (sw, sh) = (source.width() as f32, source.height() as f32);
        let (kx, ky) = (c.w / (v.w * sw), c.h / (v.h * sh));
        let to_content = Transform::from_row(kx, 0.0, 0.0, ky, c.x - v.x * sw * kx, c.y - v.y * sh * ky);
        let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
        out.draw_pixmap(0, 0, source, &paint, to_content, Some(&self.clip));

        // The click's ring goes under the cursor: white, edged in dark so it reads on any screen.
        if let Some(r) = frame.ripple {
            if let Some(ring) = PathBuilder::from_circle(r.x, r.y, r.radius.max(0.5)) {
                let width = (r.radius * RIPPLE_STROKE).max(1.0);
                let mut paint = Paint { anti_alias: true, ..Default::default() };
                paint.set_color(Color::from_rgba(0.0, 0.0, 0.0, 0.35 * r.alpha).unwrap_or(Color::BLACK));
                out.stroke_path(
                    &ring,
                    &paint,
                    &Stroke { width: width + 2.0, ..Default::default() },
                    Transform::identity(),
                    None,
                );
                paint.set_color(Color::from_rgba(1.0, 1.0, 1.0, 0.9 * r.alpha).unwrap_or(Color::WHITE));
                out.stroke_path(&ring, &paint, &Stroke { width, ..Default::default() }, Transform::identity(), None);
            }
        }

        if let Some(cur) = frame.cursor {
            let k = cur.size / CURSOR_BOX;
            let at = Transform::from_row(k, 0.0, 0.0, k, cur.x, cur.y);
            let alpha = cur.opacity.clamp(0.0, 1.0);
            let mut paint = Paint { anti_alias: true, ..Default::default() };
            paint.set_color(Color::from_rgba(0.0, 0.0, 0.0, alpha).unwrap_or(Color::BLACK));
            out.fill_path(&self.cursor, &paint, FillRule::Winding, at, None);
            paint.set_color(Color::from_rgba(1.0, 1.0, 1.0, alpha).unwrap_or(Color::WHITE));
            let stroke = Stroke { width: CURSOR_STROKE, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
            out.stroke_path(&self.cursor, &paint, &stroke, at, None);
        }
    }
}

/// A rectangle with circular corners of radius `r`.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r == 0.0 {
        return PathBuilder::from_rect(tiny_skia::Rect::from_xywh(x, y, w, h).expect("content size"));
    }
    // Control-point distance that makes a cubic Bézier a close quarter circle.
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish().expect("content path")
}

/// `#rgb` or `#rrggbb`; anything else is black.
fn parse_color(css: &str) -> Color {
    let hex = css.trim_start_matches('#');
    let digit = |i: usize, n: usize| u8::from_str_radix(hex.get(i..i + n)?, 16).ok();
    let rgb = match hex.len() {
        3 => (digit(0, 1), digit(1, 1), digit(2, 1)),
        6 => (digit(0, 2), digit(2, 2), digit(4, 2)),
        _ => (None, None, None),
    };
    match rgb {
        (Some(r), Some(g), Some(b)) if hex.len() == 3 => Color::from_rgba8(r * 17, g * 17, b * 17, 255),
        (Some(r), Some(g), Some(b)) => Color::from_rgba8(r, g, b, 255),
        _ => Color::BLACK,
    }
}

/// Three box blurs of radius `r` (close to a Gaussian with σ ≈ r), as coverage in 0..=1.
fn blur(mask: &[u8], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut a: Vec<f32> = mask.iter().map(|&v| v as f32 / 255.0).collect();
    let mut b = vec![0.0; a.len()];
    for _ in 0..3 {
        box_pass(&a, &mut b, w, h, r, true);
        box_pass(&b, &mut a, w, h, r, false);
    }
    a
}

/// One box blur along rows (`horizontal`) or columns, edges clamped.
fn box_pass(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    let n = (2 * r + 1) as f32;
    for line in 0..lines {
        let get = |i: isize| src[at(line, i.clamp(0, len as isize - 1) as usize)];
        let mut sum: f32 = (-(r as isize)..=r as isize).map(get).sum();
        for i in 0..len {
            dst[at(line, i)] = sum / n;
            sum += get(i as isize + r as isize + 1) - get(i as isize - r as isize);
        }
    }
}

/// Darkens premultiplied RGBA by `coverage × alpha` (black drawn over it).
fn darken(rgba: &mut [u8], coverage: &[f32], alpha: f32) {
    for (px, &c) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(coverage) {
        let keep = 1.0 - c * alpha;
        for v in &mut px[..3] {
            *v = (*v as f32 * keep).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use small_video_core::{CursorSample, EventLog, Project};

    fn scene(shadow: bool) -> Scene {
        let mut p = Project::new(1.0, 100, 50, &EventLog::default());
        p.style.padding = 0.1;
        p.style.radius = 0.1;
        p.style.shadow = shadow;
        p.style.background = Background::Color("#ff0000".into());
        p.style.cursor_scale = 10.0;
        let events = EventLog { cursor: vec![CursorSample { t: 0.0, x: 0.5, y: 0.5 }], clicks: vec![] };
        Scene::new(p, &events)
    }

    fn pixel(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let c = p.pixel(x, y).unwrap();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    #[test]
    fn draws_the_recording_inside_the_background() {
        let s = scene(false);
        let comp = Compositor::new(&s, 200, 120);
        let mut source = Pixmap::new(100, 50).unwrap();
        source.fill(Color::from_rgba8(0, 0, 255, 255));
        let mut out = Pixmap::new(200, 120).unwrap();
        let mut frame = s.frame(0.0, 200, 120);
        frame.cursor = None;
        comp.draw(&frame, source.as_ref(), &mut out);
        // 12 px padding; the 2:1 recording fills 176 × 88 at (12, 16).
        assert_eq!(pixel(&out, 2, 2), [255, 0, 0, 255], "background");
        assert_eq!(pixel(&out, 100, 60), [0, 0, 255, 255], "recording");
        assert_eq!(pixel(&out, 13, 17), [255, 0, 0, 255], "rounded corner shows the background");
    }

    #[test]
    fn the_cursor_is_drawn_at_its_tip() {
        let s = scene(false);
        let comp = Compositor::new(&s, 200, 120);
        let mut source = Pixmap::new(100, 50).unwrap();
        source.fill(Color::from_rgba8(0, 0, 255, 255));
        let mut out = Pixmap::new(200, 120).unwrap();
        let frame = s.frame(0.0, 200, 120);
        let c = frame.cursor.unwrap();
        comp.draw(&frame, source.as_ref(), &mut out);
        // Inside the arrow, just below and right of the tip: black.
        let k = c.size / CURSOR_BOX;
        let [r, g, b, _] = pixel(&out, (c.x + 4.0 * k) as u32, (c.y + 12.0 * k) as u32);
        assert!(r < 40 && g < 40 && b < 40, "{r} {g} {b}");
    }

    #[test]
    fn a_click_draws_a_ring() {
        let mut p = Project::new(1.0, 100, 50, &EventLog::default());
        p.style.cursor_scale = 10.0;
        p.style.background = Background::Color("#000000".into());
        let events = EventLog {
            cursor: vec![CursorSample { t: 0.0, x: 0.5, y: 0.5 }],
            clicks: vec![small_video_core::Click { t: 0.0, x: 0.5, y: 0.5, button: small_video_core::Button::Left }],
        };
        let s = Scene::new(p, &events);
        let comp = Compositor::new(&s, 200, 100);
        let source = Pixmap::new(100, 50).unwrap();
        let mut out = Pixmap::new(200, 100).unwrap();
        let mut frame = s.frame(0.1, 200, 100);
        frame.cursor = None;
        let r = frame.ripple.unwrap();
        comp.draw(&frame, source.as_ref(), &mut out);
        // Left of the centre, on the ring: light on the black recording.
        assert!(pixel(&out, (r.x - r.radius) as u32, r.y as u32)[0] > 100);
    }

    #[test]
    fn the_shadow_darkens_below_the_recording() {
        let (plain, shaded) = (scene(false), scene(true));
        let mut a = Pixmap::new(200, 120).unwrap();
        let mut b = Pixmap::new(200, 120).unwrap();
        let source = Pixmap::new(100, 50).unwrap();
        Compositor::new(&plain, 200, 120).draw(&plain.frame(0.0, 200, 120), source.as_ref(), &mut a);
        Compositor::new(&shaded, 200, 120).draw(&shaded.frame(0.0, 200, 120), source.as_ref(), &mut b);
        assert!(pixel(&b, 100, 106)[0] < pixel(&a, 100, 106)[0]);
        assert_eq!(pixel(&b, 2, 2), pixel(&a, 2, 2), "the far corner stays clear");
    }

    #[test]
    fn a_picture_covers_the_frame_without_stretching() {
        let mut p = Project::new(1.0, 100, 50, &EventLog::default());
        p.style.padding = 0.2;
        p.style.background = Background::Image("bg.png".into());
        let s = Scene::new(p, &EventLog::default());
        // A square picture, left half green, right half blue, over a 2:1 frame: scaled to the
        // width, so its middle band shows, still split down the centre.
        let mut img = Pixmap::new(10, 10).unwrap();
        img.fill(Color::from_rgba8(0, 0, 255, 255));
        img.fill_rect(
            tiny_skia::Rect::from_xywh(0.0, 0.0, 5.0, 10.0).unwrap(),
            &solid(0, 255, 0),
            Transform::identity(),
            None,
        );
        let comp = Compositor::with_image(&s, 200, 100, Some(img.as_ref()));
        let mut out = Pixmap::new(200, 100).unwrap();
        comp.draw(&s.frame(0.0, 200, 100), Pixmap::new(100, 50).unwrap().as_ref(), &mut out);
        assert_eq!(pixel(&out, 3, 3)[1], 255, "green on the left");
        assert_eq!(pixel(&out, 196, 96)[2], 255, "blue on the right");
    }

    fn solid(r: u8, g: u8, b: u8) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(r, g, b, 255));
        paint
    }

    #[test]
    fn colors_parse() {
        assert_eq!(parse_color("#ff8000"), Color::from_rgba8(255, 128, 0, 255));
        assert_eq!(parse_color("#fff"), Color::WHITE);
        assert_eq!(parse_color("teal"), Color::BLACK);
    }
}
