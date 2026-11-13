//! A take named by a tool call, and how it is described to the client.

use super::*;

/// A take named by a tool call.
pub(super) struct Take {
    pub(super) dir: PathBuf,
}

impl Take {
    pub(super) fn from(args: &Value) -> Result<Self, String> {
        let name = args["take"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("take is required")?;
        let given = Path::new(name);
        let dir = if given.is_absolute() { given.to_path_buf() } else { takes::library().join(name) };
        if !dir.join(take::SCREEN).is_file() {
            return Err(format!("no take {name} (list_takes gives them)"));
        }
        if !dir.join(take::PROJECT).is_file() {
            return Err(format!("{name} has no project yet: open it once in PolishCast"));
        }
        Ok(Self { dir })
    }

    /// The take an exported video (`video`) was made from: exports are written in their take's
    /// folder (earlier versions wrote them beside it, named after the take).
    pub(super) fn of_video(args: &Value) -> Result<Self, String> {
        let video = args["video"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("give take or video")?;
        let video = Path::new(video);
        let stem = video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let folder = video.parent().ok_or("video is an absolute path")?;
        if folder.join(take::SCREEN).is_file() {
            return Take::from(&json!({ "take": folder }));
        }
        let dir = std::fs::read_dir(folder)
            .map_err(|e| format!("{}: {e}", folder.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|d| d.join(take::SCREEN).is_file())
            .filter(|d| {
                let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                stem == name || stem.strip_prefix(&name).is_some_and(|rest| rest.starts_with(' '))
            })
            .max_by_key(|d| d.as_os_str().len())
            .ok_or(format!("no take beside {} that it was exported from", video.display()))?;
        Take::from(&json!({ "take": dir }))
    }

    pub(super) fn name(&self) -> String {
        self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub(super) fn project(&self) -> Result<Project, String> {
        Project::load(&self.dir.join(take::PROJECT)).map_err(|e| format!("can't read the take's project: {e}"))
    }

    /// Applies `f` to the project as on disk now and saves it.
    pub(super) fn change<R>(&self, f: impl FnOnce(&mut Project) -> Result<R, String>) -> Result<R, String> {
        let mut p = self.project()?;
        let r = f(&mut p)?;
        p.save(&self.dir.join(take::PROJECT)).map_err(|e| format!("can't save the take's project: {e}"))?;
        Ok(r)
    }

    pub(super) fn events(&self) -> EventLog {
        std::fs::read(self.dir.join(take::EVENTS))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub(super) fn describe(&self) -> Value {
        match self.project() {
            Ok(p) => describe(self, &p),
            Err(e) => json!({ "take": self.name(), "error": e }),
        }
    }
}

pub(super) fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The model keeps sizes and positions as `f32`, which read back as 1.7999999523.
pub(super) fn round32(v: f32) -> f64 {
    round(f64::from(v))
}

pub(super) fn aspect_of(name: &str) -> Result<Aspect, String> {
    match name {
        "screen" => Ok(Aspect::Source),
        "16:9" => Ok(Aspect::Landscape16x9),
        "9:16" => Ok(Aspect::Portrait9x16),
        "1:1" => Ok(Aspect::Square),
        other => Err(format!("aspect is screen, 16:9, 9:16 or 1:1, not {other}")),
    }
}

pub(super) fn aspect_name(a: Aspect) -> &'static str {
    match a {
        Aspect::Source => "screen",
        Aspect::Landscape16x9 => "16:9",
        Aspect::Portrait9x16 => "9:16",
        Aspect::Square => "1:1",
    }
}

pub(super) fn describe(t: &Take, p: &Project) -> Value {
    let zooms: Vec<Value> = p
        .zooms
        .iter()
        .enumerate()
        .map(|(i, z)| {
            json!({
                "zoom": i + 1,
                "start": round(z.start),
                "end": round(z.end),
                "scale": round32(z.scale),
                "x": round32(z.x),
                "y": round32(z.y),
            })
        })
        .collect();
    let masks: Vec<Value> = p
        .masks
        .iter()
        .enumerate()
        .map(|(i, m)| {
            json!({
                "mask": i + 1,
                "start": round(m.start),
                "end": round(m.end),
                "x": round32(m.x),
                "y": round32(m.y),
                "w": round32(m.w),
                "h": round32(m.h),
                "cover": if m.cover == Cover::Blur { "blur" } else { "solid" },
            })
        })
        .collect();
    let clip_list: Vec<Value> = clips::clips(&p.splits, p.duration)
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            json!({ "clip": i + 1, "start": round(c.0), "end": round(c.1), "left_out": clips::is_removed(&p.removed, c) })
        })
        .collect();
    let s = &p.style;
    let background = match &s.background {
        Background::Color(c) => json!({ "color": c }),
        Background::Gradient(a, b) => json!({ "gradient": [a, b] }),
        Background::Image(name) => json!({ "picture": name }),
    };
    let blanks = p.blanks.map(|b| {
        let cut = p.cuts.iter().filter(|c| !c.keep).count();
        json!({ "threshold_db": b.threshold_db, "min_pause_secs": b.min_pause_secs, "pauses_cut": cut, "pauses_kept": p.cuts.len() - cut })
    });
    json!({
        "take": t.name(),
        "folder": t.dir,
        "recording_secs": round(p.duration),
        "video_secs": round(p.video_duration()),
        "size_px": [p.width, p.height],
        "clicks": t.events().clicks.len(),
        "zooms": zooms,
        "masks": masks,
        "clips": clip_list,
        "blank_cutting": blanks,
        "speed": p.speed(),
        "noise_reduction": p.denoise.map(round32),
        "music_pieces": p.music.len(),
        "style": {
            "background": background,
            "padding": round32(s.padding),
            "radius": round32(s.radius),
            "shadow": s.shadow,
            "aspect": aspect_name(s.aspect),
            "cursor_scale": round32(s.cursor_scale),
            "cursor_smoothing": s.cursor_smoothing,
            "click_effect": s.click_effect,
            "hide_idle_cursor": s.hide_idle_cursor,
        },
        "exported": exports_of(&t.dir).iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    })
}

pub(super) fn list() -> Value {
    let rows: Vec<Value> = takes::list()
        .iter()
        .map(|e| {
            let mut v = json!({
                "take": e.name,
                "size_mb": (e.size as f64 / 1e6).round(),
                "exported": exports_of(&e.dir).iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            });
            if let Some(p) = &e.project {
                v["recording_secs"] = json!(round(p.duration));
                v["video_secs"] = json!(round(p.video_duration()));
                v["zooms"] = json!(p.zooms.len());
            }
            v
        })
        .collect();
    let mut v = json!({ "library": takes::library(), "takes": rows });
    if let Some(left) = licence::free_exports_left() {
        v["free_exports_left"] = json!(left);
    }
    v
}

pub(super) fn numbers(args: &Value, key: &str) -> Vec<u64> {
    args[key].as_array().into_iter().flatten().filter_map(Value::as_u64).collect()
}
