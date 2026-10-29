//! Recording started and stopped from an MCP client.
//!
//! `polishcast mcp` runs as its own process, with no window and no menu bar item, so it doesn't
//! record itself: it asks the running app to, over the local port the single-instance lock
//! already listens on (`instance`). The app then records exactly as when started by hand, with
//! the menu bar showing it and its Stop Recording item stopping it, whoever started it.
//!
//! A request is one line of JSON with the token the app wrote beside its port when it started
//! (readable by this user only), and the answer one line back: `{"ok": …}` or `{"error": …}`.
//! Nothing is recorded unless the person turned on "Let Claude or Codex record" in the MCP window.
//!
//! What can be checked without the window (the setting, the arguments, the displays, windows
//! and microphones, the Screen Recording permission) is checked on the connection's thread;
//! what changes the recording goes to the window's loop (`take_requests`), which answers it.

use crate::settings::Settings;
use polishcast_capture::{Area, DisplayInfo, Microphone, Source, WindowInfo};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Seconds counted down before recording when not asked otherwise, and the most allowed.
pub const COUNTDOWN: u64 = 3;
const MAX_COUNTDOWN: u64 = 10;
/// How long a recording started from a client lasts at most when not asked otherwise.
pub const MAX_DURATION: u64 = 10 * 60;

pub type Reply = mpsc::Sender<Result<Value, String>>;

/// A request for the window's loop.
pub struct Request {
    pub op: Op,
    /// The client's name, as shown in the menu bar ("Claude", "Codex").
    pub client: String,
    pub reply: Reply,
}

pub enum Op {
    Start(Start),
    Stop { id: Option<String> },
    Cancel { id: Option<String> },
    Status,
}

/// A recording to start, checked.
#[derive(Debug)]
pub struct Start {
    pub source: Source,
    /// Said back in `recording_status`.
    pub source_label: Value,
    pub microphone: bool,
    pub microphone_device: Option<String>,
    pub system_audio: Option<bool>,
    pub countdown: Duration,
    pub max: Duration,
    pub name: Option<String>,
}

static QUEUE: Mutex<Vec<Request>> = Mutex::new(Vec::new());

/// The requests waiting for the window's loop.
pub fn take_requests() -> Vec<Request> {
    std::mem::take(&mut *QUEUE.lock().unwrap_or_else(PoisonError::into_inner))
}

fn token_path(dir: &Path) -> PathBuf {
    dir.join("instance.token")
}

/// A new token, written where `polishcast mcp` finds it, readable by this user only.
pub fn write_token(dir: &Path) -> Option<String> {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        // RandomState's keys come from the operating system's random source.
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        h.write_u32(std::process::id());
        h.finish()
    };
    let token = format!("{:016x}{:016x}", part(), part());
    let path = token_path(dir);
    let _ = std::fs::remove_file(&path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&path).ok()?.write_all(token.as_bytes()).ok()?;
    Some(token)
}

// ── In the app ───────────────────────────────────────────────────────────────────────────

/// Answers one request line from `polishcast mcp`; `token` is the one this instance wrote.
pub fn answer(line: &str, token: &str) -> Value {
    let reply = match serde_json::from_str::<Value>(line) {
        Ok(request) if request["token"].as_str() == Some(token) => handle(&request),
        Ok(_) => Err("not allowed".into()),
        Err(_) => Err("not JSON".into()),
    };
    match reply {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e }),
    }
}

fn off() -> String {
    "Recording from Claude or Codex is turned off in PolishCast. Ask the person to turn it on: in PolishCast's \
     window, the MCP button at the top › \u{201c}Let Claude or Codex record the screen\u{201d}. Or they can record \
     from PolishCast's menu bar icon themselves."
        .into()
}

fn handle(request: &Value) -> Result<Value, String> {
    let args = &request["args"];
    let client = request["client"].as_str().unwrap_or("MCP").to_string();
    let allowed = Settings::load().allow_mcp_recording;
    let (op, wait) = match request["op"].as_str().unwrap_or("") {
        "status" => (Op::Status, Duration::from_secs(5)),
        _ if !allowed => return Err(off()),
        "sources" => return Ok(sources()),
        "start" => {
            let start = parse_start(args, &Lists::fetch(args))?;
            // The first frame can wait on the permission prompt, after the countdown.
            let wait = start.countdown + Duration::from_secs(90);
            (Op::Start(start), wait)
        }
        "stop" => (Op::Stop { id: id(args) }, Duration::from_secs(10 * 60)),
        "cancel" => (Op::Cancel { id: id(args) }, Duration::from_secs(10 * 60)),
        other => return Err(format!("no request {other}")),
    };
    let (reply, answer) = mpsc::channel();
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner).push(Request { op, client, reply });
    answer.recv_timeout(wait).unwrap_or_else(|_| Err("PolishCast didn't answer in time.".into()))
}

fn id(args: &Value) -> Option<String> {
    args["recording_id"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn sources() -> Value {
    let displays: Vec<Value> = polishcast_capture::displays()
        .iter()
        .map(|d| json!({ "display_id": d.id, "main": d.main, "x": d.x, "y": d.y, "width": d.w, "height": d.h }))
        .collect();
    let windows: Vec<Value> = polishcast_capture::windows()
        .iter()
        .map(|w| json!({ "window_id": w.id, "app": w.app, "title": w.title, "on_screen": w.on_screen }))
        .collect();
    let microphones: Vec<Value> = polishcast_capture::microphones()
        .iter()
        .map(|m| json!({ "device": m.id, "name": m.name, "default": m.default }))
        .collect();
    let mut v = json!({
        "displays": displays,
        "windows": windows,
        "microphones": microphones,
        "records_windows_and_regions": polishcast_capture::RECORDS_WINDOWS_AND_AREAS,
        "records_system_audio": polishcast_capture::RECORDS_SYSTEM_AUDIO,
    });
    if polishcast_capture::screen_recording_allowed() == Some(false) {
        v["screen_recording_permission"] = json!(no_permission());
    }
    v
}

fn no_permission() -> String {
    "PolishCast isn't allowed to record the screen yet. Ask the person to open System Settings › Privacy & \
     Security › Screen & System Audio Recording, turn PolishCast on, and quit and reopen PolishCast when asked. \
     Starting one recording from PolishCast's menu bar icon shows the same request."
        .into()
}

/// What a start request is checked against, read only when it needs them (they block).
#[derive(Default)]
struct Lists {
    displays: Vec<DisplayInfo>,
    windows: Vec<WindowInfo>,
    microphones: Vec<Microphone>,
    permission: Option<bool>,
}

impl Lists {
    fn fetch(args: &Value) -> Self {
        let kind = args["source"]["kind"].as_str();
        Self {
            displays: if kind == Some("display") { polishcast_capture::displays() } else { Vec::new() },
            windows: if kind == Some("window") { polishcast_capture::windows() } else { Vec::new() },
            microphones: if args["microphone_device"].is_string() {
                polishcast_capture::microphones()
            } else {
                Vec::new()
            },
            permission: polishcast_capture::screen_recording_allowed(),
        }
    }
}

fn parse_start(args: &Value, lists: &Lists) -> Result<Start, String> {
    if lists.permission == Some(false) {
        return Err(no_permission());
    }
    let s = &args["source"];
    let number = |key: &str| s[key].as_f64();
    let (source, source_label) = match s["kind"].as_str() {
        None => (Source::Display, json!({ "kind": "display", "display": "the one the pointer is on" })),
        Some("display") => match s["display_id"].as_u64() {
            None => (Source::Display, json!({ "kind": "display", "display": "the one the pointer is on" })),
            Some(id) => {
                if !lists.displays.iter().any(|d| u64::from(d.id) == id) {
                    return Err(format!("no display {id}: list_sources gives them"));
                }
                (Source::Screen(id as u32), json!({ "kind": "display", "display_id": id }))
            }
        },
        Some("window") | Some("region") if !polishcast_capture::RECORDS_WINDOWS_AND_AREAS => {
            return Err("only a whole display can be recorded on this computer".into())
        }
        Some("window") => {
            let w = if let Some(id) = s["window_id"].as_u64() {
                lists
                    .windows
                    .iter()
                    .find(|w| u64::from(w.id) == id)
                    .ok_or(format!("no window {id}: list_sources gives them"))?
            } else if let Some(app) = s["app"].as_str() {
                let of_app = |w: &&WindowInfo| w.app.eq_ignore_ascii_case(app.trim());
                lists
                    .windows
                    .iter()
                    .filter(of_app)
                    .find(|w| w.on_screen)
                    .or_else(|| lists.windows.iter().find(of_app))
                    .ok_or(format!("no window of {app}: list_sources gives the windows"))?
            } else {
                return Err("a window is chosen by window_id or app".into());
            };
            (Source::Window(w.id), json!({ "kind": "window", "window_id": w.id, "app": w.app, "title": w.title }))
        }
        Some("region") => {
            let (Some(x), Some(y), Some(w), Some(h)) = (number("x"), number("y"), number("width"), number("height"))
            else {
                return Err("a region needs x, y, width and height".into());
            };
            if w < 16.0 || h < 16.0 {
                return Err("a region is at least 16 points wide and high".into());
            }
            (Source::Area(Area { x, y, w, h }), json!({ "kind": "region", "x": x, "y": y, "width": w, "height": h }))
        }
        Some(other) => return Err(format!("source kind is display, window or region, not {other}")),
    };
    let microphone = args["microphone"].as_bool().unwrap_or(true);
    let microphone_device = match args["microphone_device"].as_str() {
        None => None,
        Some(d) => {
            let m = lists
                .microphones
                .iter()
                .find(|m| m.id == d || m.name.eq_ignore_ascii_case(d.trim()))
                .ok_or(format!("no microphone {d}: list_sources gives them"))?;
            Some(m.id.clone())
        }
    };
    let system_audio = args["system_audio"].as_bool();
    if system_audio == Some(true) && !polishcast_capture::RECORDS_SYSTEM_AUDIO {
        return Err("the computer's sound can't be recorded on this platform, only the microphone".into());
    }
    let countdown = args["countdown_secs"].as_u64().unwrap_or(COUNTDOWN);
    if countdown > MAX_COUNTDOWN {
        return Err(format!("countdown_secs is at most {MAX_COUNTDOWN}"));
    }
    let longest = crate::limits::MAX_LENGTH.as_secs();
    let max = args["max_duration_secs"].as_u64().unwrap_or(MAX_DURATION);
    if !(1..=longest).contains(&max) {
        return Err(format!("max_duration_secs is from 1 to {longest}"));
    }
    let name = match args["name"].as_str() {
        None => None,
        Some(n) => Some(crate::takes::check_name(n)?),
    };
    Ok(Start {
        source,
        source_label,
        microphone,
        microphone_device,
        system_audio,
        countdown: Duration::from_secs(countdown),
        max: Duration::from_secs(max),
        name,
    })
}

/// A recording started from a client, from its request until it's saved.
pub struct Asked {
    pub id: String,
    pub dir: PathBuf,
    pub client: String,
    pub source_label: Value,
    pub max: Duration,
    /// What to record, until the countdown is over.
    pub options: Option<polishcast_capture::Options>,
    /// When its first frame was written (wall clock, for the client).
    pub started_at: Option<chrono::DateTime<chrono::Local>>,
    /// start_recording's answer, until the first frame.
    pub on_started: Option<Reply>,
    /// stop_recording's and cancel_recording's answers, once saved.
    pub on_saved: Vec<Reply>,
    /// Why it stopped, once it did.
    pub stopped_by: Option<&'static str>,
    pub cancel: bool,
}

impl Asked {
    pub fn reply_started(&mut self, r: Result<Value, String>) {
        if let Some(tx) = self.on_started.take() {
            let _ = tx.send(r);
        }
    }

    pub fn reply_saved(&mut self, r: &Result<Value, String>) {
        for tx in self.on_saved.drain(..) {
            let _ = tx.send(r.clone());
        }
    }

    /// Whether a request naming `id` (or none) is about this recording.
    pub fn is(&self, id: &Option<String>) -> bool {
        id.as_ref().is_none_or(|i| *i == self.id)
    }
}

/// The client's recordings: the one under way, and what the last one saved came to.
#[derive(Default)]
pub struct Remote {
    pub current: Option<Asked>,
    pub last: Option<(String, Result<Value, String>)>,
}

/// The menu bar's name for a client, from the name it gave when it connected.
pub fn client_label(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("claude") {
        "Claude".into()
    } else if lower.contains("codex") {
        "Codex".into()
    } else {
        "MCP".into()
    }
}

// ── In `polishcast mcp` ──────────────────────────────────────────────────────────────────

fn connect(dir: &Path) -> Option<(TcpStream, String)> {
    let port = std::fs::read_to_string(dir.join("instance.port")).ok()?.trim().parse::<u16>().ok()?;
    let token = std::fs::read_to_string(token_path(dir)).ok()?.trim().to_string();
    let stream = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(2)).ok()?;
    Some((stream, token))
}

/// Opens PolishCast. On macOS through Launch Services, so it runs as itself rather than as a
/// child of the MCP client (whose permissions macOS would check instead of PolishCast's).
fn launch() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find PolishCast: {e}"))?;
    let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"));
    let mut command = match bundle {
        Some(app) if cfg!(target_os = "macos") => {
            let mut c = std::process::Command::new("open");
            c.arg("-g").arg("-a").arg(app);
            c
        }
        _ => std::process::Command::new(&exe),
    };
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: it outlives this server.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    command.spawn().map_err(|e| format!("can't open PolishCast: {e}"))?;
    Ok(())
}

/// Asks the running app, opening it first if it isn't running.
pub fn ask(op: &str, args: &Value, client: &str, wait: Duration) -> Result<Value, String> {
    let dir = crate::webview_data_dir();
    let (mut stream, token) = match connect(&dir) {
        Some(c) => c,
        None => {
            launch()?;
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                std::thread::sleep(Duration::from_millis(300));
                if let Some(c) = connect(&dir) {
                    break c;
                }
                if Instant::now() >= deadline {
                    return Err("PolishCast didn't open. Ask the person to open it, then try again.".into());
                }
            }
        }
    };
    let request = json!({ "token": token, "op": op, "args": args, "client": client });
    let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(|e| format!("can't reach PolishCast: {e}"))?;
    let _ = stream.set_read_timeout(Some(wait + Duration::from_secs(10)));
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).map_err(|e| format!("PolishCast didn't answer: {e}"))?;
    let answer: Value = serde_json::from_str(&answer).map_err(|_| "PolishCast's answer wasn't readable".to_string())?;
    match (answer.get("ok"), answer["error"].as_str()) {
        (Some(v), _) => Ok(v.clone()),
        (None, Some(e)) => Err(e.to_string()),
        (None, None) => Err("PolishCast's answer wasn't readable".into()),
    }
}

/// How long `ask` waits for each request, as the app does plus a margin.
pub fn wait_for(op: &str, args: &Value) -> Duration {
    match op {
        "start" => Duration::from_secs(args["countdown_secs"].as_u64().unwrap_or(COUNTDOWN).min(MAX_COUNTDOWN) + 120),
        "stop" | "cancel" => Duration::from_secs(10 * 60 + 30),
        _ => Duration::from_secs(60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lists() -> Lists {
        Lists {
            displays: vec![DisplayInfo { id: 7, x: 0.0, y: 0.0, w: 1512.0, h: 982.0, main: true }],
            windows: vec![
                WindowInfo { id: 1, app: "Safari".into(), title: "Old".into(), on_screen: false },
                WindowInfo { id: 2, app: "Safari".into(), title: "Docs".into(), on_screen: true },
            ],
            microphones: vec![Microphone { id: "mic-1".into(), name: "MacBook Microphone".into(), default: true }],
            permission: Some(true),
        }
    }

    #[test]
    fn a_start_with_nothing_given_records_the_pointer_display_after_3_seconds() {
        let s = parse_start(&json!({}), &lists()).unwrap();
        assert_eq!(s.source, Source::Display);
        assert!(s.microphone && s.microphone_device.is_none() && s.system_audio.is_none());
        assert_eq!((s.countdown.as_secs(), s.max.as_secs()), (COUNTDOWN, MAX_DURATION));
    }

    #[test]
    fn sources_are_checked_against_what_is_there() {
        let l = lists();
        let s = parse_start(&json!({ "source": { "kind": "display", "display_id": 7 } }), &l).unwrap();
        assert_eq!(s.source, Source::Screen(7));
        assert!(parse_start(&json!({ "source": { "kind": "display", "display_id": 8 } }), &l).is_err());
        let m = parse_start(&json!({ "microphone_device": "macbook microphone" }), &l).unwrap();
        assert_eq!(m.microphone_device.as_deref(), Some("mic-1"));
        assert!(parse_start(&json!({ "microphone_device": "USB" }), &l).is_err());
        assert!(parse_start(&json!({ "max_duration_secs": 0 }), &l).is_err());
        assert!(parse_start(&json!({ "countdown_secs": 60 }), &l).is_err());
        if polishcast_capture::RECORDS_WINDOWS_AND_AREAS {
            let w = parse_start(&json!({ "source": { "kind": "window", "app": "safari" } }), &l).unwrap();
            assert_eq!(w.source, Source::Window(2), "the window on screen first");
            assert!(parse_start(
                &json!({ "source": { "kind": "region", "x": 0, "y": 0, "width": 4, "height": 4 } }),
                &l
            )
            .is_err());
        }
    }

    #[test]
    fn nothing_starts_without_the_screen_recording_permission() {
        let l = Lists { permission: Some(false), ..lists() };
        assert!(parse_start(&json!({}), &l).unwrap_err().contains("Screen & System Audio Recording"));
    }

    #[test]
    fn a_request_without_the_token_is_refused() {
        assert_eq!(answer(r#"{"op":"status","token":"wrong"}"#, "right")["error"], "not allowed");
        assert_eq!(answer("show me", "right")["error"], "not JSON");
    }

    #[test]
    fn clients_are_named_in_the_menu_bar() {
        assert_eq!(client_label("claude-ai"), "Claude");
        assert_eq!(client_label("codex-mcp-client"), "Codex");
        assert_eq!(client_label("cursor"), "MCP");
    }
}
