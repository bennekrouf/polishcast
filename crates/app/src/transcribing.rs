//! A take's transcript, shared by the editor and the MCP tools: transcribing it with api0,
//! naming its chapters, and handing an exported video and its captions to PublishKit.

use crate::settings::Settings;
use polishcast_core::transcript::{self, Chapter, Transcript};
use polishcast_core::{take, Project};
use polishcast_export::http;
use polishcast_transcribe::{Options, Progress};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Where a take's transcript is kept.
pub fn path(dir: &Path) -> PathBuf {
    dir.join(take::TRANSCRIPT)
}

pub fn load(dir: &Path) -> Option<Transcript> {
    Transcript::load(&path(dir))
}

/// The licence's number, or what to do without one.
pub fn licence() -> Result<String, String> {
    polishcast_transcribe::licence().ok_or_else(|| {
        "Transcribing uses the Mayorana licence's AI credits: add a licence in PolishCast → Licence (any Mayorana \
         app's licence works)."
            .to_string()
    })
}

/// Transcribes the take, finds its chapters and saves both, replacing an earlier transcript.
/// Blocks for as long as api0 takes: run it off the UI thread.
pub fn transcribe(
    dir: &Path,
    language: Option<&str>,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<Transcript, String> {
    let options = Options {
        licence: licence()?,
        region: Settings::load().ai_region.id().to_string(),
        language: language.map(str::trim).filter(|l| !l.is_empty()).map(str::to_lowercase),
    };
    let duration = Project::load(&dir.join(take::PROJECT)).map(|p| p.duration).unwrap_or_default();
    let mut t = polishcast_transcribe::transcribe(&dir.join(take::SCREEN), &options, progress, cancel)
        .map_err(|e| format!("{e:#}"))?;
    let duration = if duration > 0.0 { duration } else { t.segments.last().map_or(0.0, |s| s.end) };
    t.chapters = transcript::detect_chapters(&t.segments, duration);
    t.save(&path(dir)).map_err(|e| format!("Couldn't save the transcript: {e}"))?;
    Ok(t)
}

/// Better titles for the chapters, written by api0 from what is said in each, in the
/// transcript's language. The chapters' starts are kept.
pub fn name_chapters(dir: &Path) -> Result<Vec<Chapter>, String> {
    let mut t = load(dir).ok_or("Transcribe the take first.")?;
    if t.chapters.is_empty() {
        return Err("This take has no chapters to name.".into());
    }
    let excerpts: Vec<String> = t
        .chapters
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let end = t.chapters.get(i + 1).map_or(f64::MAX, |n| n.start);
            let said: String = t
                .segments
                .iter()
                .filter(|s| s.start >= c.start && s.start < end)
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let excerpt: String = said.chars().take(1200).collect();
            format!("Chapter {}: {excerpt}", i + 1)
        })
        .collect();
    let system = "You title the chapters of a video for its YouTube description. Each title is 2 to 6 words, \
                  specific to what that chapter covers, in the language the chapter is spoken in, without \
                  numbering, quotes or a final period.";
    let user = format!(
        "Reply with ONLY a JSON array of {} strings, one title per chapter, in order.\n\n{}",
        t.chapters.len(),
        excerpts.join("\n\n")
    );
    let reply = chat(system, &user)?;
    let titles = titles_of(&reply, t.chapters.len())?;
    for (c, title) in t.chapters.iter_mut().zip(titles) {
        c.title = title;
    }
    t.save(&path(dir)).map_err(|e| format!("Couldn't save the transcript: {e}"))?;
    Ok(t.chapters)
}

/// The titles in the model's reply: a JSON array of `n` strings, fences or chatter around it ignored.
fn titles_of(reply: &str, n: usize) -> Result<Vec<String>, String> {
    let (Some(from), Some(to)) = (reply.find('['), reply.rfind(']')) else {
        return Err("api0's answer had no titles in it".into());
    };
    let titles: Vec<String> = serde_json::from_str::<Vec<String>>(&reply[from..=to.max(from)])
        .map_err(|_| "api0's answer wasn't the list of titles asked for".to_string())?
        .into_iter()
        .map(|t| t.trim().trim_end_matches('.').to_string())
        .collect();
    if titles.len() != n || titles.iter().any(String::is_empty) {
        return Err(format!("api0 gave {} titles for {n} chapters", titles.len()));
    }
    Ok(titles)
}

/// One answer from api0's chat model in the region set in PolishCast.
fn chat(system: &str, user: &str) -> Result<String, String> {
    let licence = licence()?;
    let body = json!({
        "model": Settings::load().ai_region.id(),
        "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }],
        "temperature": 0.4,
    });
    let agent: ureq::Agent = http::config().http_status_as_error(false).build().into();
    let mut resp = agent
        .post(&format!("{}/v1/chat/completions", desktop_kit::licence::GATEWAY))
        .header("Authorization", &format!("Bearer {licence}"))
        .header("X-Client-App", polishcast_core::license::PRODUCT)
        .header("X-Task", "text")
        .header("Content-Type", "application/json")
        .send(body.to_string().as_bytes())
        .map_err(|e| format!("api0 is unreachable: {}", http::describe(&e)))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    match status {
        200..=299 => {
            v["choices"][0]["message"]["content"].as_str().map(str::to_string).ok_or("api0's answer was empty".into())
        }
        402 => Err("No AI credits left on the licence this month.".into()),
        401 | 403 => Err("api0 didn't accept the licence: check it in PolishCast → Licence.".into()),
        _ => Err(format!("api0 returned {status}: {}", v["error"]["message"].as_str().unwrap_or_default())),
    }
}

/// Hands an exported video, with the captions written beside it, to PublishKit's Repurpose:
/// a note in the folder every Mayorana app shares, which PublishKit opens with the video and
/// its transcript filled in. PublishKit is opened if it isn't running.
pub fn send_to_publishkit(video: &Path, title: &str) -> Result<PathBuf, String> {
    let [srt, ..] = polishcast_export::caption_files(video);
    if !video.is_file() {
        return Err(format!("{} isn't there any more: export the take again.", video.display()));
    }
    if !srt.is_file() {
        return Err("Export the video with captions first: PublishKit drafts from its transcript.".into());
    }
    let note = handoff_path().ok_or("This computer has no settings folder to hand the video over through")?;
    if let Some(dir) = note.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let body = json!({
        "media": video,
        "transcript": srt,
        "title": title,
        "from": "polishcast",
        "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
    });
    std::fs::write(&note, serde_json::to_vec_pretty(&body).unwrap_or_default())
        .map_err(|e| format!("Couldn't write {}: {e}", note.display()))?;
    open_publishkit()?;
    Ok(note)
}

/// `…/mayorana/handoff/publishkit-repurpose.json`, beside the shared licence.
fn handoff_path() -> Option<PathBuf> {
    Some(desktop_kit::licence::shared_path()?.parent()?.join("handoff").join("publishkit-repurpose.json"))
}

fn open_publishkit() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let ok = std::process::Command::new("open").args(["-a", "PublishKit"]).status().is_ok_and(|s| s.success());
        if !ok {
            return Err(
                "PublishKit isn't installed: get it at https://mayorana.ch/en/apps/publishkit, then send again.".into(),
            );
        }
    }
    #[cfg(windows)]
    {
        // Where its installer puts it: {autopf}\PublishKit, per user or for everyone.
        let installed =
            [dirs::data_local_dir().map(|d| d.join("Programs")), std::env::var_os("ProgramFiles").map(PathBuf::from)];
        let exe =
            installed.into_iter().flatten().map(|d| d.join("PublishKit").join("publishkit.exe")).find(|e| e.is_file());
        match exe {
            Some(exe) => {
                std::process::Command::new(exe).spawn().map_err(|e| format!("Couldn't open PublishKit: {e}"))?;
            }
            None => return Err("Open PublishKit: it picks the video up when it opens.".into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_read_from_the_answer() {
        assert_eq!(titles_of("```json\n[\"Setup.\", \" Billing \"]\n```", 2).unwrap(), ["Setup", "Billing"]);
        assert!(titles_of("[\"Only one\"]", 2).is_err());
        assert!(titles_of("no list", 1).is_err());
    }
}
