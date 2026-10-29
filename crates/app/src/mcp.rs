//! `polishcast mcp` — the takes, served to an MCP client over stdio, and the window that says
//! how to connect one.
//!
//! No window, no capture: the same project model and export the editor uses, driven from tool
//! calls instead. A take is a folder in the library; its edits are its `project.json`, which the
//! editor may have open too. So every change here reads the file, applies the change and writes
//! it back at once, and an open editor takes it in as one undo step (`Doc::reload_if_changed`).
//!
//! Recording is the running app's: start_recording and the other recording tools ask it to
//! (`remote`), opening it if needed, so the menu bar shows the recording and can stop it, and
//! the take is made as when recorded by hand. Only if the person turned it on in this window.
//!
//! Requests are handled one at a time, so an export holds the next call until it is done.

use crate::editor::{
    clean_export_name, cut_blanks, export_name, export_path, exports_of, measure, toggle_clip, FRAME_RATES,
    GIF_FRAME_RATES, GIF_SIZES, RESOLUTIONS,
};
use crate::{licence, remote, takes};
use dioxus::prelude::*;
use polishcast_core::project::{Aspect, Background, SPEEDS};
use polishcast_core::zoom::{self, auto_zoom, AutoZoom, Zoom};
use polishcast_core::{clips, take, EventLog, Loudness, Project};
use polishcast_export::{Format, Progress, Settings};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;

/// The newest protocol revision this server speaks. A client asking for an older one gets its
/// own back; everything used here exists in all of them.
const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "PolishCast turns screen recordings into polished videos: the camera zooms in where \
the person clicked, the cursor is redrawn smooth, and the recording sits on a background with padding, rounded \
corners and a shadow. Each recording is a take, named by its folder; list_takes gives them, newest first. \
get_take shows a take's zooms, clips and cuts, style and length. set_zooms edits the zooms (or suggests them \
again from the clicks), set_clips splits the recording and leaves parts out, set_blanks cuts the pauses where \
nobody speaks, set_style changes the look, the frame's shape and the playback speed. Times are seconds of the \
recording. export writes an MP4 or a GIF next to the take's folder; the free version exports 10 different \
takes. Recording: start_recording records the screen, a display, a window or a region (list_sources gives \
their ids), after a countdown, until stop_recording, which saves it as a take with zooms suggested from its \
clicks; recording_status says what is being recorded, cancel_recording throws it away. The menu bar shows the \
recording, and the person can stop it there at any time. It must be turned on by the person first, in \
PolishCast's MCP window; until then the recording tools say how. A demo: start_recording, drive the app being \
shown, stop_recording, set_zooms with suggest, export.";

/// The client's name, from `initialize`, shown in the menu bar while it records.
static CLIENT: OnceLock<String> = OnceLock::new();

pub fn serve() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            write(&mut stdout, &error(Value::Null, -32700, "not JSON"));
            continue;
        };
        // A notification (no id) asks for nothing back.
        let Some(id) = message.get("id").cloned() else { continue };
        write(&mut stdout, &handle(id, &message));
    }
}

fn handle(id: Value, message: &Value) -> Value {
    let method = message["method"].as_str().unwrap_or("");
    let params = &message["params"];
    match method {
        "initialize" => {
            if let Some(name) = params["clientInfo"]["name"].as_str() {
                let _ = CLIENT.set(name.to_string());
            }
            let asked = params["protocolVersion"].as_str().unwrap_or(PROTOCOL);
            let version = if asked <= PROTOCOL { asked } else { PROTOCOL };
            result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "polishcast", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let (text, is_error) = match call(name, &params["arguments"]) {
                Ok(value) => (pretty(&value), false),
                Err(why) => (why, true),
            };
            result(id, json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
        }
        _ => error(id, -32601, &format!("no method {method}")),
    }
}

fn write(stdout: &mut std::io::Stdout, reply: &Value) {
    // One message per line, and nothing else ever on stdout: the client reads it as the
    // protocol, so a stray print is a corrupt message.
    let mut bytes = serde_json::to_vec(reply).unwrap_or_default();
    bytes.push(b'\n');
    let _ = stdout.write_all(&bytes);
    let _ = stdout.flush();
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

// ── Tool definitions ─────────────────────────────────────────────────────────────────────

fn tools() -> Value {
    let take_arg = json!({ "type": "string", "description": "The take's name, as list_takes gives it (its folder's name), or its folder's absolute path." });
    let indices = json!({ "type": "array", "items": { "type": "integer", "minimum": 1 } });
    let zoom = json!({
        "type": "object",
        "properties": {
            "start": { "type": "number" },
            "end": { "type": "number" },
            "scale": { "type": "number", "minimum": 1, "maximum": 4, "description": "2 shows half the screen's width. Default 1.8." },
            "x": { "type": "number", "minimum": 0, "maximum": 1, "description": "What to center on, from the screen's left (0) to right (1)." },
            "y": { "type": "number", "minimum": 0, "maximum": 1, "description": "From top (0) to bottom (1)." },
        },
    });
    let mp4_sizes: Vec<u32> = RESOLUTIONS.iter().map(|(px, _)| *px).collect();
    let gif_sizes: Vec<u32> = GIF_SIZES.iter().map(|(px, _)| *px).collect();
    json!([
        {
            "name": "list_takes",
            "description": "Every take in the library, newest first: its length, the video's length after cuts \
                            and speed, its zooms, its size on disk, and the videos exported from it.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "get_take",
            "description": "One take in full: its zooms, its clips (and which are left out), its blank cutting, \
                            its style, speed and frame shape, and how many clicks were recorded.",
            "inputSchema": { "type": "object", "properties": { "take": take_arg }, "required": ["take"] },
        },
        {
            "name": "set_zooms",
            "description": "Edits the zooms. Zooms are numbered from 1 as get_take lists them before this call; \
                            updates are applied first, then removals, then additions (which never overlap \
                            another zoom: one starting inside one is refused). suggest replaces every zoom \
                            with the ones suggested from the clicks, before anything else.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "suggest": { "type": "boolean" },
                    "update": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "zoom": { "type": "integer", "minimum": 1 },
                                "start": { "type": "number" }, "end": { "type": "number" },
                                "scale": { "type": "number" }, "x": { "type": "number" }, "y": { "type": "number" },
                            },
                            "required": ["zoom"],
                        },
                    },
                    "remove": indices,
                    "add": { "type": "array", "items": zoom },
                },
                "required": ["take"],
            },
        },
        {
            "name": "set_clips",
            "description": "Splits the recording into clips and leaves clips out of the video, or puts them back. \
                            Splits come first; clips are then numbered from 1 as they are after the splits. \
                            keep_from leaves out everything before that time, keep_until everything after.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "split_at": { "type": "array", "items": { "type": "number" } },
                    "remove": indices,
                    "restore": indices,
                    "keep_from": { "type": "number" },
                    "keep_until": { "type": "number" },
                },
                "required": ["take"],
            },
        },
        {
            "name": "set_blanks",
            "description": "Cuts the pauses where nobody speaks, or stops cutting them. The first time, the \
                            recording's sound is measured with ffmpeg (a few seconds).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "on": { "type": "boolean" },
                    "threshold_db": { "type": "number", "description": "Quieter than this is silence. Default -45." },
                    "min_pause_secs": { "type": "number", "description": "Shorter pauses stay. Default 1." },
                },
                "required": ["take", "on"],
            },
        },
        {
            "name": "set_style",
            "description": "Changes the look. Only the fields given change.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "background_color": { "type": "string", "description": "A CSS colour, e.g. #18181b." },
                    "background_gradient": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 2, "description": "Top and bottom CSS colours." },
                    "padding": { "type": "number", "minimum": 0, "maximum": 0.3, "description": "Space around the recording, as a fraction of the frame's shorter side." },
                    "radius": { "type": "number", "minimum": 0, "maximum": 0.1, "description": "Corner radius, same unit." },
                    "shadow": { "type": "boolean" },
                    "aspect": { "type": "string", "enum": ["screen", "16:9", "9:16", "1:1"] },
                    "cursor_scale": { "type": "number", "minimum": 0.5, "maximum": 4 },
                    "cursor_smoothing": { "type": "number", "minimum": 0, "maximum": 0.5, "description": "Seconds the pointer's path is averaged over." },
                    "click_effect": { "type": "boolean" },
                    "hide_idle_cursor": { "type": "boolean" },
                    "speed": { "type": "number", "enum": SPEEDS },
                },
                "required": ["take"],
            },
        },
        {
            "name": "export",
            "description": format!(
                "Writes the video next to the take's folder, as an MP4 (with sound) or a GIF (silent, looping), \
                 named after the take, its size, frame rate, shape, speed, whether the blanks are cut and \
                 the time of the export, so it never replaces an earlier one; name sets another name. Takes \
                 about as long as the video on a recent Mac; ffmpeg is downloaded first if none is installed. Sizes are pixels along the longer side: {mp4_sizes:?} for MP4, \
                 {gif_sizes:?} for GIF. The free version exports {} different takes.",
                polishcast_core::license::FREE_EXPORTS
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "format": { "type": "string", "enum": ["mp4", "gif"] },
                    "size": { "type": "integer", "description": "Default 1920 for MP4, 960 for GIF." },
                    "fps": { "type": "integer", "description": format!("{FRAME_RATES:?} for MP4 (default 30), {GIF_FRAME_RATES:?} for GIF (default 15).") },
                    "name": { "type": "string", "description": "The file's name, without extension. A file of that name is replaced." },
                },
                "required": ["take", "format"],
            },
        },
        {
            "name": "list_sources",
            "description": "What can be recorded: the displays (main first), the windows by app, and the \
                            microphones, with the ids start_recording takes. Whether this computer can record \
                            a window or a region, and the computer's sound.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "start_recording",
            "description": format!(
                "Starts recording, as the person would from PolishCast's menu bar: the menu bar counts down, \
                 then shows the recording with Stop Recording. Answers once the first frame is written, with \
                 the recording_id, which is also the take's name. PolishCast's own windows are left out of a \
                 display's recording. Fails if a recording is already in progress, if the person hasn't turned \
                 on recording from Claude or Codex in PolishCast, or without the Screen Recording permission \
                 (the error says how to grant it); the first recording with the microphone asks for that \
                 permission. Stops by itself after max_duration_secs (default {}).",
                remote::MAX_DURATION
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "object",
                        "description": "What to record. Default: the whole display the pointer is on.",
                        "properties": {
                            "kind": { "type": "string", "enum": ["display", "window", "region"] },
                            "display_id": { "type": "integer", "description": "For a display, from list_sources. Default: the one the pointer is on." },
                            "window_id": { "type": "integer", "description": "For a window, from list_sources." },
                            "app": { "type": "string", "description": "For a window: the app's name, instead of window_id. Its window on screen is recorded." },
                            "x": { "type": "number", "description": "For a region, in points from the main display's top-left, as list_sources gives the displays." },
                            "y": { "type": "number" },
                            "width": { "type": "number" },
                            "height": { "type": "number" },
                        },
                        "required": ["kind"],
                    },
                    "microphone": { "type": "boolean", "description": "Default true." },
                    "microphone_device": { "type": "string", "description": "A microphone's device or name from list_sources. Default: the system's." },
                    "system_audio": { "type": "boolean", "description": "Also the sound apps play. Default: as set in PolishCast." },
                    "countdown_secs": { "type": "integer", "minimum": 0, "maximum": 10, "description": format!("Default {}.", remote::COUNTDOWN) },
                    "max_duration_secs": { "type": "integer", "minimum": 1, "maximum": crate::limits::MAX_LENGTH.as_secs(), "description": format!("Default {}.", remote::MAX_DURATION) },
                    "name": { "type": "string", "description": "The take's folder name. Default: the date and time it starts." },
                },
            },
        },
        {
            "name": "stop_recording",
            "description": "Stops the recording and saves it as a take, ready for set_zooms (whose suggested zooms \
                            are already in) and export. Answers once saved, which can take a few seconds: the \
                            take's name, its length, the clicks recorded and why it stopped. A recording that \
                            already stopped (the person stopped it, or max_duration_secs) gives its take. Only \
                            recordings started with start_recording can be stopped here.",
            "inputSchema": {
                "type": "object",
                "properties": { "recording_id": { "type": "string", "description": "From start_recording. Default: the one in progress." } },
            },
        },
        {
            "name": "recording_status",
            "description": "Whether PolishCast is idle, counting down, recording or saving; what, since when, who \
                            started it and its recording_id; whether recording from here is turned on; and how \
                            the last recording started here ended.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "cancel_recording",
            "description": "Stops the recording started with start_recording and moves it to the Trash, so no take \
                            is left. During the countdown, nothing was recorded.",
            "inputSchema": {
                "type": "object",
                "properties": { "recording_id": { "type": "string", "description": "Default: the one in progress." } },
            },
        },
    ])
}

fn call(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "list_takes" => Ok(list()),
        "get_take" => Take::from(args).map(|t| t.describe()),
        "set_zooms" => set_zooms(args),
        "set_clips" => set_clips(args),
        "set_blanks" => set_blanks(args),
        "set_style" => set_style(args),
        "export" => export(args),
        "list_sources" => ask("sources", args),
        "start_recording" => ask("start", args),
        "stop_recording" => ask("stop", args),
        "recording_status" => ask("status", args),
        "cancel_recording" => ask("cancel", args),
        _ => Err(format!("no tool {name}")),
    }
}

/// Asks the running app (`remote`).
fn ask(op: &str, args: &Value) -> Result<Value, String> {
    let client = CLIENT.get().map_or("MCP", String::as_str);
    remote::ask(op, args, client, remote::wait_for(op, args))
}

// ── One take ─────────────────────────────────────────────────────────────────────────────

/// A take named by a tool call.
struct Take {
    dir: PathBuf,
}

impl Take {
    fn from(args: &Value) -> Result<Self, String> {
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

    fn name(&self) -> String {
        self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn project(&self) -> Result<Project, String> {
        Project::load(&self.dir.join(take::PROJECT)).map_err(|e| format!("can't read the take's project: {e}"))
    }

    /// Applies `f` to the project as on disk now and saves it.
    fn change<R>(&self, f: impl FnOnce(&mut Project) -> Result<R, String>) -> Result<R, String> {
        let mut p = self.project()?;
        let r = f(&mut p)?;
        p.save(&self.dir.join(take::PROJECT)).map_err(|e| format!("can't save the take's project: {e}"))?;
        Ok(r)
    }

    fn events(&self) -> EventLog {
        std::fs::read(self.dir.join(take::EVENTS))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn describe(&self) -> Value {
        match self.project() {
            Ok(p) => describe(self, &p),
            Err(e) => json!({ "take": self.name(), "error": e }),
        }
    }
}

fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The model keeps sizes and positions as `f32`, which read back as 1.7999999523.
fn round32(v: f32) -> f64 {
    round(f64::from(v))
}

fn aspect_name(a: Aspect) -> &'static str {
    match a {
        Aspect::Source => "screen",
        Aspect::Landscape16x9 => "16:9",
        Aspect::Portrait9x16 => "9:16",
        Aspect::Square => "1:1",
    }
}

fn describe(t: &Take, p: &Project) -> Value {
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
        "clips": clip_list,
        "blank_cutting": blanks,
        "speed": p.speed(),
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

// ── Tools ────────────────────────────────────────────────────────────────────────────────

fn list() -> Value {
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

fn numbers(args: &Value, key: &str) -> Vec<u64> {
    args[key].as_array().into_iter().flatten().filter_map(Value::as_u64).collect()
}

fn set_zooms(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    let clicks = t.events().clicks;
    t.change(|p| {
        if args["suggest"].as_bool().unwrap_or(false) {
            p.zooms = auto_zoom(&clicks, p.duration, &AutoZoom::default());
        }
        let count = p.zooms.len();
        let index = |n: u64| -> Result<usize, String> {
            (n as usize).checked_sub(1).filter(|i| *i < count).ok_or(format!("no zoom {n}: the take has {count}"))
        };
        for u in args["update"].as_array().into_iter().flatten() {
            let i = index(u["zoom"].as_u64().ok_or("each update needs its zoom number")?)?;
            let z = p.zooms[i];
            let (start, end) = (u["start"].as_f64().unwrap_or(z.start), u["end"].as_f64().unwrap_or(z.end));
            if start != z.start || end != z.end {
                zoom::set_bounds(&mut p.zooms, i, start, end, p.duration);
            }
            let z = &mut p.zooms[i];
            if let Some(v) = u["scale"].as_f64() {
                z.scale = (v as f32).clamp(1.0, 4.0);
            }
            if let Some(v) = u["x"].as_f64() {
                z.x = (v as f32).clamp(0.0, 1.0);
            }
            if let Some(v) = u["y"].as_f64() {
                z.y = (v as f32).clamp(0.0, 1.0);
            }
        }
        let mut gone: Vec<usize> = numbers(args, "remove").into_iter().map(index).collect::<Result<_, _>>()?;
        gone.sort_unstable();
        gone.dedup();
        for i in gone.into_iter().rev() {
            p.zooms.remove(i);
        }
        let mut refused = vec![];
        for a in args["add"].as_array().into_iter().flatten() {
            let start = a["start"].as_f64().ok_or("each zoom added needs a start")?;
            let end = a["end"].as_f64().unwrap_or(start + 2.5);
            let z = Zoom {
                start: start.clamp(0.0, p.duration),
                end: end.clamp(0.0, p.duration),
                scale: a["scale"].as_f64().map_or(1.8, |v| (v as f32).clamp(1.0, 4.0)),
                x: a["x"].as_f64().map_or(0.5, |v| (v as f32).clamp(0.0, 1.0)),
                y: a["y"].as_f64().map_or(0.5, |v| (v as f32).clamp(0.0, 1.0)),
            };
            if zoom::insert(&mut p.zooms, z).is_none() {
                refused.push(format!("{start} s: starts inside another zoom, or too little room before the next"));
            }
        }
        if refused.is_empty() {
            Ok(())
        } else {
            Err(format!("Nothing was saved. Zooms not added: {}", refused.join("; ")))
        }
    })?;
    Ok(t.describe())
}

fn set_clips(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    t.change(|p| {
        for at in args["split_at"].as_array().into_iter().flatten().filter_map(Value::as_f64) {
            clips::split(&mut p.splits, p.duration, at);
        }
        if let Some(from) = args["keep_from"].as_f64() {
            clips::split(&mut p.splits, p.duration, from);
            for c in clips::clips(&p.splits, p.duration).into_iter().filter(|c| c.1 <= from + 1e-6) {
                clips::remove(&mut p.removed, c);
            }
        }
        if let Some(until) = args["keep_until"].as_f64() {
            clips::split(&mut p.splits, p.duration, until);
            for c in clips::clips(&p.splits, p.duration).into_iter().filter(|c| c.0 >= until - 1e-6) {
                clips::remove(&mut p.removed, c);
            }
        }
        let list = clips::clips(&p.splits, p.duration);
        let clip = |n: u64| -> Result<(f64, f64), String> {
            (n as usize)
                .checked_sub(1)
                .and_then(|i| list.get(i).copied())
                .ok_or(format!("no clip {n}: the take has {}", list.len()))
        };
        for n in numbers(args, "remove") {
            let c = clip(n)?;
            if !clips::is_removed(&p.removed, c) {
                toggle_clip(p, c);
            }
        }
        for n in numbers(args, "restore") {
            let c = clip(n)?;
            if clips::is_removed(&p.removed, c) {
                toggle_clip(p, c);
            }
        }
        if p.timeline().duration() <= 0.0 {
            return Err("every clip would be left out: nothing would remain of the video. Nothing was saved.".into());
        }
        Ok(())
    })?;
    Ok(t.describe())
}

fn set_blanks(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    let on = args["on"].as_bool().ok_or("on is required")?;
    if !on {
        t.change(|p| {
            p.blanks = None;
            Ok(())
        })?;
        return Ok(t.describe());
    }
    let kept: Option<Loudness> =
        std::fs::read(t.dir.join(take::LOUDNESS)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let loudness = match kept {
        Some(l) => l,
        None => measure(&t.dir)
            .map_err(|e| format!("can't measure the sound: {e:#}"))?
            .ok_or("this recording has no sound, so there are no pauses to find")?,
    };
    t.change(|p| {
        let mut params = p.blanks.unwrap_or_default();
        if let Some(v) = args["threshold_db"].as_f64() {
            params.threshold_db = (v as f32).clamp(-80.0, -10.0);
        }
        if let Some(v) = args["min_pause_secs"].as_f64() {
            params.min_pause_secs = v.clamp(0.2, 10.0);
        }
        cut_blanks(p, &loudness, params);
        Ok(())
    })?;
    Ok(t.describe())
}

fn set_style(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    t.change(|p| {
        let s = &mut p.style;
        if let Some(c) = args["background_color"].as_str() {
            s.background = Background::Color(c.trim().to_string());
        }
        if let Some(g) = args["background_gradient"].as_array() {
            let colors: Vec<String> = g.iter().filter_map(|v| v.as_str().map(|c| c.trim().to_string())).collect();
            let [top, bottom] = <[String; 2]>::try_from(colors).map_err(|_| "background_gradient takes two colours")?;
            s.background = Background::Gradient(top, bottom);
        }
        let number = |key: &str| args[key].as_f64();
        if let Some(v) = number("padding") {
            s.padding = (v as f32).clamp(0.0, 0.3);
        }
        if let Some(v) = number("radius") {
            s.radius = (v as f32).clamp(0.0, 0.1);
        }
        if let Some(v) = args["shadow"].as_bool() {
            s.shadow = v;
        }
        if let Some(a) = args["aspect"].as_str() {
            s.aspect = match a {
                "screen" => Aspect::Source,
                "16:9" => Aspect::Landscape16x9,
                "9:16" => Aspect::Portrait9x16,
                "1:1" => Aspect::Square,
                other => return Err(format!("aspect is screen, 16:9, 9:16 or 1:1, not {other}")),
            };
        }
        if let Some(v) = number("cursor_scale") {
            s.cursor_scale = (v as f32).clamp(0.5, 4.0);
        }
        if let Some(v) = number("cursor_smoothing") {
            s.cursor_smoothing = v.clamp(0.0, 0.5);
        }
        if let Some(v) = args["click_effect"].as_bool() {
            s.click_effect = v;
        }
        if let Some(v) = args["hide_idle_cursor"].as_bool() {
            s.hide_idle_cursor = v;
        }
        if let Some(v) = number("speed") {
            if !SPEEDS.contains(&v) {
                return Err(format!("speed is one of {SPEEDS:?}"));
            }
            p.speed = v;
        }
        Ok(())
    })?;
    Ok(t.describe())
}

fn export(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    let (format, sizes, rates, defaults) = match args["format"].as_str() {
        Some("mp4") => (Format::Mp4, RESOLUTIONS, FRAME_RATES, Settings::default()),
        Some("gif") => {
            (Format::Gif, GIF_SIZES, GIF_FRAME_RATES, Settings { format: Format::Gif, long_side: 960, fps: 15 })
        }
        _ => return Err("format is mp4 or gif".into()),
    };
    let long_side = args["size"].as_u64().map_or(defaults.long_side, |n| n as u32);
    if !sizes.iter().any(|(px, _)| *px == long_side) {
        let allowed: Vec<u32> = sizes.iter().map(|(px, _)| *px).collect();
        return Err(format!("size is one of {allowed:?} for this format"));
    }
    let fps = args["fps"].as_u64().map_or(defaults.fps, |n| n as u32);
    if !rates.contains(&fps) {
        return Err(format!("fps is one of {rates:?} for this format"));
    }
    let name = t.name();
    if !licence::allows_export(&name) {
        return Err(format!(
            "The free version of PolishCast exports {} different takes, and they are used. Exporting one of them \
             again is free. PolishCast Pro exports every take: {}",
            polishcast_core::license::FREE_EXPORTS,
            licence::buy_url()
        ));
    }
    let settings = Settings { format, long_side, fps };
    let file_name = match args["name"].as_str() {
        Some(typed) => clean_export_name(typed, format).ok_or("name is empty")?,
        None => {
            let p = t.project()?;
            let now = chrono::Local::now().naive_local();
            export_name(&name, settings, p.style.aspect, p.speed, p.blanks.is_some(), now)
        }
    };
    let out = export_path(&t.dir, &file_name, format);
    let started = std::time::Instant::now();
    polishcast_export::export(&t.dir, &out, settings, &Progress::default(), &AtomicBool::new(false))
        .map_err(|e| format!("{e:#}"))?;
    licence::count_export(&name);
    let mut v = json!({
        "file": out,
        "size_mb": std::fs::metadata(&out).map_or(0.0, |m| (m.len() as f64 / 1e5).round() / 10.0),
        "took_secs": started.elapsed().as_secs(),
    });
    if let Some(left) = licence::free_exports_left() {
        v["free_exports_left"] = json!(left);
    }
    Ok(v)
}

// ── The window ───────────────────────────────────────────────────────────────────────────

/// Whether the "Use PolishCast from Claude or Codex" window is open. Provided once, by `App`.
#[derive(Clone, Copy)]
pub struct McpOpen(pub Signal<bool>);

/// This executable, as a client should start it.
fn command() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "polishcast".into())
}

/// The `mcpServers` entry for Claude Desktop's config file. Built with serde so a Windows
/// path's backslashes arrive escaped.
pub fn desktop_config(command: &str) -> String {
    let config = json!({ "mcpServers": { "polishcast": { "command": command, "args": ["mcp"] } } });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

/// The one line that adds it to Claude Code, for every project.
pub fn code_command(command: &str) -> String {
    format!("claude mcp add polishcast --scope user -- \"{command}\" mcp")
}

/// The one line that adds it to Codex. Its CLI, IDE extension and app share
/// one list of servers, each one there for every project.
pub fn codex_command(command: &str) -> String {
    format!("codex mcp add polishcast -- \"{command}\" mcp")
}

fn desktop_config_path() -> &'static str {
    if cfg!(target_os = "windows") {
        "%APPDATA%\\Claude\\claude_desktop_config.json"
    } else if cfg!(target_os = "macos") {
        "~/Library/Application Support/Claude/claude_desktop_config.json"
    } else {
        "~/.config/Claude/claude_desktop_config.json"
    }
}

#[component]
pub fn McpDialog() -> Element {
    let mut open = use_context::<McpOpen>().0;
    let mut settings = use_context::<Signal<crate::settings::Settings>>();
    if !open() {
        return rsx! {};
    }
    let command = command();
    let desktop = desktop_config(&command);
    let code = code_command(&command);
    let codex = codex_command(&command);
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| open.set(false),
            div { class: "modal mcp-modal", onclick: move |e| e.stop_propagation(),
                div { class: "modal-head",
                    h2 { "Use PolishCast from Claude or Codex" }
                    button { class: "modal-close", title: "Close", onclick: move |_| open.set(false), "×" }
                }
                div { class: "modal-body",
                    p { class: "hint",
                        "Ask Claude to polish your takes: add or tighten zooms, cut the pauses, trim the start and end, \
                         change the background or the frame's shape, and export an MP4 or a GIF. An open take shows \
                         its changes within a couple of seconds, and \u{2318}Z takes them back."
                    }
                    label { class: "check",
                        input {
                            r#type: "checkbox",
                            checked: settings.read().allow_mcp_recording,
                            onchange: move |e| {
                                settings.write().allow_mcp_recording = e.checked();
                                settings.peek().save();
                            },
                        }
                        "Let Claude or Codex record the screen"
                    }
                    p { class: "hint",
                        "So it can make a whole demo by itself: start recording, use the app, stop and polish. The menu \
                         bar counts down first, shows who is recording, and Stop Recording there stops it at any time. \
                         A recording started this way stops by itself after 10 minutes unless asked for longer."
                    }
                    h3 { "1 · Claude Desktop" }
                    p { class: "hint",
                        "Settings \u{2192} Developer \u{2192} Edit Config, paste this in and save, then quit Claude and open it \
                         again. If the file already has an mcpServers section, add just the polishcast entry to it."
                    }
                    Snippet { text: desktop.clone() }
                    p { class: "hint mcp-path", "The file: {desktop_config_path()}" }
                    h3 { "2 · Claude Code" }
                    p { class: "hint", "Run this once in a terminal:" }
                    Snippet { text: code.clone() }
                    h3 { "3 · Codex" }
                    p { class: "hint", "Run this once in a terminal:" }
                    Snippet { text: codex.clone() }
                    h3 { "4 · Try it" }
                    p { class: "hint",
                        "Ask: \u{201c}Cut the pauses in my last take, make it 16:9 and export a GIF.\u{201d} With \
                         recording on: \u{201c}Record a 30-second demo of the export dialog in Safari and make it a GIF.\u{201d}"
                    }
                    p { class: "hint mcp-note",
                        "Claude in the browser (claude.ai) can\u{2019}t use this: it only connects to servers on the internet, \
                         and PolishCast runs on your computer. Any other app that speaks MCP can, with the same command: "
                        code { "{command} mcp" }
                    }
                    div { class: "modal-actions",
                        button { onclick: move |_| open.set(false), "Close" }
                    }
                }
            }
        }
    }
}

/// A block of text to paste somewhere else, with its own Copy button.
#[component]
pub(crate) fn Snippet(text: String) -> Element {
    let mut copied = use_signal(|| false);
    let to_copy = text.clone();
    rsx! {
        div { class: "mcp-snippet-wrap",
            pre { class: "mcp-snippet", "{text}" }
            button {
                class: "mcp-copy",
                onclick: move |_| {
                    // The native clipboard: the web view's needs a secure origin and a user
                    // gesture check that differ per platform.
                    let ok = arboard::Clipboard::new().and_then(|mut c| c.set_text(to_copy.clone())).is_ok();
                    copied.set(ok);
                },
                if copied() { "Copied ✓" } else { "Copy" }
            }
        }
    }
}

#[cfg(test)]
mod tests;
