//! Frame layout: where everything goes in one output frame.
//!
//! `Scene::frame` is the single description of a frame. The editor's preview draws it in the
//! web view (from `Scene::track`) and the exporter draws it with `compose::Compositor`, so
//! both show the same picture.

pub mod compose;

use serde::Serialize;
use small_video_core::camera::viewport_at;
use small_video_core::cursor::smooth;
use small_video_core::events::position_at;
use small_video_core::{CursorSample, EventLog, Project, Viewport};

/// A rectangle in output pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Cursor {
    /// Tip of the pointer, in output pixels.
    pub x: f32,
    pub y: f32,
    /// Height of the pointer image, in output pixels.
    pub size: f32,
}

/// The recording's drop shadow, in output pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Shadow {
    /// Downward offset.
    pub dy: f32,
    /// Gaussian standard deviation (a CSS blur radius is twice this).
    pub sigma: f32,
    pub alpha: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Where the recording is drawn, over the background.
    pub content: Rect,
    /// The part of the recording that fills `content`.
    pub source: Viewport,
    pub radius: f32,
    pub shadow: Option<Shadow>,
    /// `None` when the pointer is outside the part on view.
    pub cursor: Option<Cursor>,
}

/// Pointer height at 1× zoom and `cursor_scale` 1, as a fraction of the content's width.
const CURSOR_SIZE: f32 = 0.012;

/// A project ready to render: the cursor path is smoothed once, not per frame.
pub struct Scene {
    pub project: Project,
    cursor: Vec<CursorSample>,
}

impl Scene {
    pub fn new(project: Project, events: &EventLog) -> Self {
        let cursor = smooth(&events.cursor, project.style.cursor_smoothing);
        Self { project, cursor }
    }

    /// Output size for the project's aspect, with the longer side `long_side` pixels (even
    /// numbers, as H.264 needs).
    pub fn output_size(&self, long_side: u32) -> (u32, u32) {
        let ratio = self.project.style.aspect.ratio(self.project.width as f32 / self.project.height as f32);
        let even = |v: f32| (v.round() as u32 / 2 * 2).max(2);
        if ratio >= 1.0 {
            (even(long_side as f32), even(long_side as f32 / ratio))
        } else {
            (even(long_side as f32 * ratio), even(long_side as f32))
        }
    }

    pub fn frame(&self, t: f64, width: u32, height: u32) -> Frame {
        let style = &self.project.style;
        let short = width.min(height) as f32;
        let pad = style.padding * short;
        let content = fit(
            self.project.width as f32 / self.project.height as f32,
            Rect { x: pad, y: pad, w: width as f32 - 2.0 * pad, h: height as f32 - 2.0 * pad },
        );
        let source = viewport_at(&self.project.zooms, t);
        let cursor = position_at(&self.cursor, t).and_then(|(x, y)| {
            let (u, v) = ((x - source.x) / source.w, (y - source.y) / source.h);
            ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then(|| Cursor {
                x: content.x + u * content.w,
                y: content.y + v * content.h,
                size: CURSOR_SIZE * style.cursor_scale * content.w / source.w,
            })
        });
        let shadow = style.shadow.then_some(Shadow { dy: 0.012 * short, sigma: 0.02 * short, alpha: 0.45 });
        Frame { width, height, content, source, radius: style.radius * short, shadow, cursor }
    }

    /// Every frame at `fps`, for the editor's preview to look up as the video plays.
    pub fn track(&self, fps: f64, width: u32, height: u32) -> Track {
        let first = self.frame(0.0, width, height);
        let count = (self.project.duration * fps).ceil() as usize + 1;
        let mut frames = Vec::with_capacity(count * TRACK_STRIDE);
        for i in 0..count {
            let f = self.frame(i as f64 / fps, width, height);
            let (cx, cy, cs) = f.cursor.map_or((0.0, 0.0, 0.0), |c| (c.x, c.y, c.size));
            frames.extend([f.source.x, f.source.y, f.source.w, cx, cy, cs]);
        }
        let timeline = self.project.timeline();
        Track {
            fps,
            width,
            height,
            content: first.content,
            radius: first.radius,
            shadow: first.shadow,
            skips: timeline.skips(self.project.duration),
            speed: self.project.speed(),
            duration: self.project.video_duration(),
            frames,
        }
    }
}

/// Values per frame in `Track::frames`.
pub const TRACK_STRIDE: usize = 6;

/// The preview's copy of the scene: what changes per frame, sampled, and what doesn't, once.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Track {
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub content: Rect,
    pub radius: f32,
    pub shadow: Option<Shadow>,
    /// Stretches of the recording left out of the video (blanks cut), which playback jumps over.
    pub skips: Vec<(f64, f64)>,
    /// How much faster than recorded the preview plays.
    pub speed: f64,
    /// Length of the video, without them and at that speed.
    pub duration: f64,
    /// Per frame: source viewport x, y, size; cursor x, y, size in output pixels (size 0 when
    /// the cursor is out of view).
    pub frames: Vec<f32>,
}

/// The largest rectangle of aspect `ratio` (width / height) centered in `area`.
fn fit(ratio: f32, area: Rect) -> Rect {
    let (w, h) = if area.w / area.h > ratio { (area.h * ratio, area.h) } else { (area.w, area.w / ratio) };
    Rect { x: area.x + (area.w - w) / 2.0, y: area.y + (area.h - h) / 2.0, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;
    use small_video_core::project::Aspect;

    fn scene(aspect: Aspect) -> Scene {
        let mut p = Project::new(10.0, 1600, 1000, &EventLog::default());
        p.style.aspect = aspect;
        p.style.padding = 0.1;
        let events = EventLog { cursor: vec![CursorSample { t: 0.0, x: 0.5, y: 0.5 }], clicks: vec![] };
        Scene::new(p, &events)
    }

    #[test]
    fn output_sizes_follow_the_aspect() {
        assert_eq!(scene(Aspect::Source).output_size(1920), (1920, 1200));
        assert_eq!(scene(Aspect::Portrait9x16).output_size(1920), (1080, 1920));
    }

    #[test]
    fn content_is_padded_and_centered() {
        let f = scene(Aspect::Landscape16x9).frame(0.0, 1920, 1080);
        // 108 px padding; the 16:10 recording is limited by the height left: 864 px.
        let r = f.content;
        for (got, want) in [(r.x, 268.8), (r.y, 108.0), (r.w, 1382.4), (r.h, 864.0)] {
            assert!((got - want).abs() < 0.01, "{r:?}");
        }
        let c = f.cursor.unwrap();
        assert!((c.x - 960.0).abs() < 0.01 && (c.y - 540.0).abs() < 0.01, "{c:?}");
    }

    #[test]
    fn the_track_samples_every_frame_to_the_end() {
        let s = scene(Aspect::Source);
        let t = s.track(30.0, 1920, 1200);
        assert_eq!(t.frames.len(), 301 * TRACK_STRIDE);
        let f = s.frame(5.0, 1920, 1200);
        let i = 150 * TRACK_STRIDE;
        assert_eq!(&t.frames[i..i + 3], &[f.source.x, f.source.y, f.source.w]);
        assert_eq!(t.content, f.content);
    }
}
