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
use crate::{licence, remote, takes, transcribing};
use desktop_kit::mcp::Server;
use dioxus::prelude::*;
use polishcast_core::mask::{Cover, Mask};
use polishcast_core::project::{Aspect, Background, DENOISE_STRENGTH, SPEEDS};
use polishcast_core::zoom::{self, auto_zoom, AutoZoom, Zoom};
use polishcast_core::{clips, take, EventLog, Loudness, Project};
use polishcast_export::{Captions, Format, Progress, Settings};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;

mod dialog;
mod edits;
mod export;
mod one_take;
mod tools;
mod transcript;
pub use dialog::*;
use edits::*;
use export::*;
use one_take::*;
use tools::*;
use transcript::*;

const INSTRUCTIONS: &str = "PolishCast turns screen recordings into polished videos: the camera zooms in where \
the person clicked, the cursor is redrawn smooth, and the recording sits on a styled background. Each recording \
is a take: a take argument is its name as list_takes gives it (its folder's name) or its folder's absolute path. \
Times are seconds of the recording. A demo: start_recording, drive the app being shown, stop_recording, \
set_zooms with suggest, export. \
Before calling these tools, read the polishcast skill if one is available: it has the confirmation \
rules, defaults and report layouts that these instructions leave out.";

/// The client's name, from `initialize`, shown in the menu bar while it records.
static CLIENT: OnceLock<String> = OnceLock::new();

/// What the client is told: the plumbing is desktop-kit's, the tools are below.
const SERVER: Server =
    Server { name: "polishcast", version: env!("CARGO_PKG_VERSION"), instructions: INSTRUCTIONS, tools, library: true };

pub fn serve() {
    SERVER.serve(call);
}

fn call(name: &str, args: &Value) -> Result<Value, String> {
    let result = run(name, args);
    // A new export makes the take ready in the library the other apps show.
    if result.is_ok() && matches!(name, "export" | "export_clip") {
        if let Ok(t) = Take::from(args).or_else(|_| Take::of_video(args)) {
            crate::library::sync_take(&t.dir);
        }
    }
    result
}

fn run(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "list_takes" => Ok(list()),
        "import_video" => import_video(args),
        "get_take" => Take::from(args).map(|t| t.describe()),
        "set_zooms" => set_zooms(args),
        "set_masks" => set_masks(args),
        "set_clips" => set_clips(args),
        "set_blanks" => set_blanks(args),
        "set_style" => set_style(args),
        "export" => export(args),
        "export_clip" => export_clip(args),
        "transcribe" => transcribe(args),
        "get_transcript" => get_transcript(args),
        "set_chapters" => set_chapters(args),
        "send_to_publishkit" => send_to_publishkit(args),
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

#[cfg(test)]
mod tests;

/// Makes a take of an MP4 or MOV made elsewhere.
fn import_video(args: &Value) -> Result<Value, String> {
    let file = args["file"].as_str().ok_or("file is required: the video's absolute path")?;
    let path = std::path::Path::new(file);
    if !path.is_absolute() || !path.is_file() {
        return Err(format!("no file at {file}: give the video's absolute path"));
    }
    let dir = crate::takes::import(path)?;
    Take::from(&json!({ "take": dir.to_string_lossy() })).map(|t| t.describe())
}
