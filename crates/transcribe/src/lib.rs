//! Transcribing a take with api0, on the Mayorana licence's AI credits: no model on this
//! computer and no account of its own.
//!
//! ffmpeg decodes the recording's sound to 16 kHz mono, the rate speech models work at. It is
//! sent ten minutes at a time as a small MP3, each piece cut at its quietest moment near the
//! boundary so no word is split, to api0's OpenAI-compatible `/v1/audio/transcriptions` in the
//! chosen region. Timed words come back and are grouped into caption-sized lines here. When the
//! microphone and the sound apps played are on separate tracks, each line is given to whoever
//! was louder on it — measured here, from the recording itself.

use anyhow::{anyhow, bail, Context, Result};
use polishcast_core::transcript::{self, Segment, Speaker, Transcript, Word};
use polishcast_export::{http, sound, tools};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The api0 path, OpenAI's.
const ENDPOINT: &str = "/v1/audio/transcriptions";
/// The kind of work, for api0's `X-Task`: a region runs a speech model for it.
const TASK: &str = "transcribe";

const RATE: usize = 16_000;
/// Ten minutes per piece: about 2.4 MB at 32 kb/s, far under what speech APIs take.
const CHUNK: usize = 600 * RATE;
/// Where in the last stretch of a piece it is cut: at its quietest fifth of a second.
const CUT_SEARCH: usize = 20 * RATE;
const CUT_WINDOW: usize = RATE / 5;
/// The track the microphone is on when a take has two: ScreenCaptureKit writes the sound apps
/// play first and the microphone after it.
const MIC_TRACK: usize = 1;
/// Louder on the microphone by this much (dB) and a line is the person recording's.
const MIC_MARGIN_DB: f32 = 3.0;

/// What a transcription is doing, shared with the window or the MCP client while it runs.
#[derive(Default)]
pub struct Progress {
    /// Thousandths: of ffmpeg downloaded while `downloading`, else of the sound transcribed.
    pub done: AtomicU64,
    pub downloading: AtomicBool,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        self.done.load(Ordering::Relaxed) as f32 / 1000.0
    }

    fn set(&self, fraction: f32) {
        self.done.store((fraction.clamp(0.0, 1.0) * 1000.0) as u64, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// What to transcribe with.
#[derive(Clone, Debug)]
pub struct Options {
    /// The licence number api0 is paid with.
    pub licence: String,
    /// The api0 region (`europe`, `china`, `usa`): where the speech is processed.
    pub region: String,
    /// A language code (`en`, `fr`) when the user knows it; `None` lets the model tell.
    pub language: Option<String>,
}

/// The licence saved on this computer by any Mayorana app, or `API0_API_KEY`.
pub fn licence() -> Option<String> {
    if let Some(v) = std::env::var("API0_API_KEY").ok().filter(|v| !v.trim().is_empty()) {
        return Some(v.trim().to_string());
    }
    match desktop_kit::licence::load_shared() {
        desktop_kit::licence::Shared::Licence(l) if !l.trim().is_empty() => Some(l.trim().to_string()),
        _ => None,
    }
}

/// The transcript of `recording` (a take's screen recording, or any video or sound file), in
/// its own time.
pub fn transcribe(recording: &Path, options: &Options, progress: &Progress, cancel: &AtomicBool) -> Result<Transcript> {
    let cancelled = || cancel.load(Ordering::Relaxed);
    let ffmpeg = if let Some(path) = polishcast_export::ffmpeg() {
        path
    } else {
        progress.downloading.store(true, Ordering::Relaxed);
        let got = polishcast_export::ensure_ffmpeg(&mut |f| progress.set(f), &cancelled);
        progress.downloading.store(false, Ordering::Relaxed);
        got?
    };
    progress.set(0.0);
    let tracks = sound::audio_tracks(&ffmpeg, recording)?;
    if tracks == 0 {
        bail!("{} has no sound to transcribe.", recording.display());
    }
    let mixed = decode(&ffmpeg, recording, &sound::source(0, tracks))?;
    if mixed.is_empty() {
        bail!("{} has no sound to transcribe.", recording.display());
    }

    let mut words = Vec::new();
    let mut language = options.language.clone();
    let total = mixed.len();
    let mut from = 0;
    while from < total {
        if cancelled() {
            bail!(Cancelled);
        }
        let to = piece_end(&mixed, from);
        let offset = from as f64 / RATE as f64;
        let mp3 = encode(&ffmpeg, &mixed[from..to])?;
        let reply = request(&mp3, options, language.as_deref())?;
        if language.is_none() {
            // The rest in the language heard first, so a stray word doesn't switch it.
            language = reply["language"].as_str().map(language_code).filter(|l| !l.is_empty());
        }
        words.extend(words_of(&reply, offset));
        from = to;
        progress.set(from as f32 / total as f32);
    }
    let mut segments = transcript::captions(&words, 42);
    if tracks == 2 {
        label_speakers(&ffmpeg, recording, &mut segments)?;
    }
    Ok(Transcript {
        language: language.unwrap_or_default(),
        model: format!("api0 {}", options.region),
        segments,
        chapters: Vec::new(),
    })
}

/// api0's answer for one piece: OpenAI's `verbose_json`.
fn request(mp3: &[u8], options: &Options, language: Option<&str>) -> Result<Value> {
    let boundary = format!("polishcast{:016x}", std::process::id() as u64 ^ mp3.len() as u64);
    let mut fields = vec![
        ("model", options.region.as_str()),
        ("response_format", "verbose_json"),
        ("timestamp_granularities[]", "word"),
        ("timestamp_granularities[]", "segment"),
    ];
    if let Some(l) = language {
        fields.push(("language", l));
    }
    let body = multipart(&boundary, &fields, "piece.mp3", "audio/mpeg", mp3);
    let url = format!("{}{ENDPOINT}", desktop_kit::licence::GATEWAY);
    let agent: ureq::Agent = http::config().http_status_as_error(false).build().into();
    let mut resp = agent
        .post(&url)
        .header("Authorization", &format!("Bearer {}", options.licence))
        .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .header("X-Client-App", polishcast_core::license::PRODUCT)
        .header("X-Task", TASK)
        .send(&body[..])
        .map_err(|e| anyhow!("api0 is unreachable: {}", http::describe(&e)))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(32 << 20).read_to_string().unwrap_or_default();
    if !(200..300).contains(&status) {
        bail!("{}", refusal(status, &text));
    }
    serde_json::from_str(&text).context("api0's transcript wasn't the JSON expected")
}

/// What to tell the user when api0 says no.
fn refusal(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_default();
    match status {
        401 | 403 => "api0 didn't accept the licence: check it in PolishCast → Licence.".into(),
        402 => format!("No AI credits left on the licence this month. {message}").trim().to_string(),
        404 | 405 => "api0 doesn't transcribe yet in this region. Try again after the next update.".into(),
        413 => "A piece of the recording was too large for api0.".into(),
        _ if message.is_empty() => format!("api0 returned {status}."),
        _ => format!("api0 returned {status}: {message}"),
    }
}

/// A `multipart/form-data` body: the text fields, then the file.
fn multipart(boundary: &str, fields: &[(&str, &str)], name: &str, mime: &str, file: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(file.len() + 1024);
    for (key, value) in fields {
        out.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{key}\"\r\n\r\n{value}\r\n").as_bytes(),
        );
    }
    out.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(file);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

/// The timed words in a reply, moved by `offset` seconds. A reply with segments but no words
/// gives each segment as one word, so its timing is kept.
fn words_of(reply: &Value, offset: f64) -> Vec<Word> {
    let timed = |v: &Value, key: &str| -> Option<Word> {
        let text = v[key].as_str()?.trim();
        let (start, end) = (v["start"].as_f64()?, v["end"].as_f64()?);
        (!text.is_empty() && end >= start).then(|| Word { start: offset + start, end: offset + end, text: text.into() })
    };
    let mut words: Vec<Word> =
        reply["words"].as_array().into_iter().flatten().filter_map(|w| timed(w, "word")).collect();
    if !words.is_empty() {
        punctuate(&mut words, reply);
        return words;
    }
    reply["segments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["no_speech_prob"].as_f64().unwrap_or(0.0) < 0.8)
        .filter_map(|s| timed(s, "text"))
        .collect()
}

/// Words as the segments spell them: OpenAI's timed words come without their punctuation or
/// capitals, which the segments' text has. Each word takes the next matching one of the
/// segments' words, looking a few ahead in case one was dropped.
fn punctuate(words: &mut [Word], reply: &Value) {
    let spelled: Vec<&str> = reply["segments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["text"].as_str())
        .flat_map(str::split_whitespace)
        .collect();
    let bare = |w: &str| w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let mut next = 0;
    for w in words.iter_mut() {
        let want = bare(&w.text);
        let found = (next..spelled.len().min(next + 4)).find(|&i| bare(spelled[i]) == want);
        if let Some(i) = found {
            w.text = spelled[i].to_string();
            next = i + 1;
        }
    }
}

/// `en` for `en` or `english`: OpenAI's verbose_json names the language in full.
fn language_code(name: &str) -> String {
    const NAMES: &[(&str, &str)] = &[
        ("english", "en"),
        ("french", "fr"),
        ("german", "de"),
        ("spanish", "es"),
        ("italian", "it"),
        ("portuguese", "pt"),
        ("dutch", "nl"),
        ("arabic", "ar"),
        ("chinese", "zh"),
        ("japanese", "ja"),
        ("korean", "ko"),
        ("russian", "ru"),
        ("turkish", "tr"),
        ("polish", "pl"),
    ];
    let name = name.trim().to_lowercase();
    NAMES.iter().find(|(full, _)| *full == name).map_or(name, |(_, code)| code.to_string())
}

/// `file`'s sound through `graph` (whose output is its last filter), as 16 kHz mono samples.
fn decode(ffmpeg: &Path, file: &Path, graph: &str) -> Result<Vec<f32>> {
    let mut child = tools::command(ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args([
            "-filter_complex",
            &format!("{graph},aresample={RATE},aformat=sample_fmts=flt:channel_layouts=mono[out]"),
        ])
        .args(["-map", "[out]", "-f", "f32le", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Starting ffmpeg")?;
    let mut bytes = Vec::new();
    child.stdout.take().context("ffmpeg output")?.read_to_end(&mut bytes)?;
    let mut errors = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut errors);
    }
    if !child.wait()?.success() {
        bail!("ffmpeg couldn't read the sound: {}", errors.trim());
    }
    Ok(bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect())
}

/// 16 kHz mono samples as a 32 kb/s MP3: plenty for speech, small to send.
fn encode(ffmpeg: &Path, samples: &[f32]) -> Result<Vec<u8>> {
    let mut child = tools::command(ffmpeg)
        .args(["-v", "error", "-nostdin", "-f", "f32le", "-ar", &RATE.to_string(), "-ac", "1", "-i", "-"])
        .args(["-c:a", "libmp3lame", "-b:a", "32k", "-f", "mp3", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Starting ffmpeg")?;
    let mut stdin = child.stdin.take().context("ffmpeg input")?;
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    // Written from another thread: ffmpeg's output must be read while it's fed.
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let mut mp3 = Vec::new();
    child.stdout.take().context("ffmpeg output")?.read_to_end(&mut mp3)?;
    writer.join().map_err(|_| anyhow!("Feeding ffmpeg failed"))??;
    if !child.wait()?.success() || mp3.is_empty() {
        bail!("ffmpeg couldn't encode the sound to send");
    }
    Ok(mp3)
}

/// Where the piece starting at `from` ends: ten minutes on, moved back to the quietest
/// moment in its last twenty seconds so no word is cut in two.
fn piece_end(samples: &[f32], from: usize) -> usize {
    let end = from + CHUNK;
    if end + CUT_SEARCH >= samples.len() {
        return samples.len();
    }
    (end - CUT_SEARCH..end)
        .step_by(CUT_WINDOW / 2)
        .min_by(|&a, &b| energy(&samples[a..a + CUT_WINDOW]).total_cmp(&energy(&samples[b..b + CUT_WINDOW])))
        .map_or(end, |at| at + CUT_WINDOW / 2)
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32
}

fn db(samples: &[f32]) -> f32 {
    10.0 * energy(samples).max(1e-10).log10()
}

/// Each line is the person recording's when the microphone is clearly the louder track on it,
/// else the other person's.
fn label_speakers(ffmpeg: &Path, recording: &Path, segments: &mut [Segment]) -> Result<()> {
    let guest = decode(ffmpeg, recording, &format!("[0:a:{}]{}", 1 - MIC_TRACK, sound::ALIGN))?;
    let mic = decode(ffmpeg, recording, &format!("[0:a:{MIC_TRACK}]{}", sound::ALIGN))?;
    for s in segments.iter_mut() {
        let level = |track: &[f32]| {
            let a = ((s.start * RATE as f64) as usize).min(track.len());
            let b = ((s.end * RATE as f64) as usize).min(track.len()).max(a);
            db(&track[a..b])
        };
        s.speaker = Some(if level(&mic) >= level(&guest) - MIC_MARGIN_DB { Speaker::You } else { Speaker::Guest });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piece_is_cut_where_it_is_quiet() {
        let mut samples = vec![0.5f32; CHUNK + 60 * RATE];
        let quiet = CHUNK - 7 * RATE;
        samples[quiet..quiet + RATE].fill(0.0);
        let end = piece_end(&samples, 0);
        assert!((quiet..quiet + RATE).contains(&end), "{end} {quiet}");
        // The last piece takes the rest.
        assert_eq!(piece_end(&samples, end), samples.len());
    }

    #[test]
    fn words_come_from_the_reply_in_recording_time() {
        let reply = serde_json::json!({
            "language": "french",
            "words": [{ "word": " Bonjour", "start": 0.5, "end": 0.9 }, { "word": "", "start": 1.0, "end": 1.1 }],
            "segments": [{ "text": "ignored when there are words", "start": 0.0, "end": 2.0 }],
        });
        let words = words_of(&reply, 600.0);
        assert_eq!(words, vec![Word { start: 600.5, end: 600.9, text: "Bonjour".into() }]);
        assert_eq!(language_code("French"), "fr");
        assert_eq!(language_code("fr"), "fr");

        let bare = serde_json::json!({
            "words": [{ "word": "hello", "start": 0.0, "end": 0.4 }, { "word": "its", "start": 0.5, "end": 0.7 }, { "word": "me", "start": 0.7, "end": 0.9 }],
            "segments": [{ "text": " Hello, it's me.", "start": 0.0, "end": 1.0 }],
        });
        let spelled: Vec<String> = words_of(&bare, 0.0).into_iter().map(|w| w.text).collect();
        assert_eq!(spelled, ["Hello,", "it's", "me."]);

        let segments_only = serde_json::json!({ "segments": [
            { "text": " Hello there.", "start": 1.0, "end": 2.0 },
            { "text": "noise", "start": 2.0, "end": 3.0, "no_speech_prob": 0.95 },
        ]});
        assert_eq!(words_of(&segments_only, 0.0), vec![Word { start: 1.0, end: 2.0, text: "Hello there.".into() }]);
    }

    #[test]
    fn the_request_is_a_multipart_form() {
        let body = multipart("B", &[("model", "europe")], "piece.mp3", "audio/mpeg", b"MP3");
        let text = String::from_utf8(body).unwrap();
        assert!(text.starts_with("--B\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\neurope\r\n"));
        assert!(
            text.contains("name=\"file\"; filename=\"piece.mp3\"\r\nContent-Type: audio/mpeg\r\n\r\nMP3\r\n--B--\r\n")
        );
    }

    #[test]
    fn refusals_say_what_to_do() {
        assert!(
            refusal(402, r#"{"error":{"message":"Credits reset on the 1st."}}"#).ends_with("Credits reset on the 1st.")
        );
        assert!(refusal(404, "").contains("doesn't transcribe yet"));
        assert_eq!(refusal(500, "oops"), "api0 returned 500.");
    }

    /// Calls api0 for real, on the licence's credits: transcribes `POLISHCAST_TRANSCRIBE`.
    /// `POLISHCAST_TRANSCRIBE=talk.mp4 cargo test -p polishcast-transcribe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn transcribes_a_file() {
        let Some(file) = std::env::var_os("POLISHCAST_TRANSCRIBE") else { return };
        let options = Options { licence: licence().unwrap(), region: "europe".into(), language: None };
        let t = transcribe(Path::new(&file), &options, &Progress::default(), &AtomicBool::new(false)).unwrap();
        println!("{} ({} lines)\n{}", t.language, t.segments.len(), transcript::srt(&t.segments));
        assert!(!t.segments.is_empty());
    }
}
