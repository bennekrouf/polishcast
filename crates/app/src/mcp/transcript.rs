//! A take's transcript and chapters, and sending it on to PublishKit.

use super::*;

pub(super) fn transcribe(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    let started = std::time::Instant::now();
    let tr = transcribing::transcribe(
        &t.dir,
        args["language"].as_str(),
        &polishcast_transcribe::Progress::default(),
        &AtomicBool::new(false),
    )?;
    Ok(json!({
        "take": t.name(),
        "language": tr.language,
        "lines": tr.segments.len(),
        "words": tr.text().split_whitespace().count(),
        "speakers": tr.has_speakers(),
        "chapters": chapters_json(&tr.chapters),
        "took_secs": started.elapsed().as_secs(),
    }))
}

pub(super) fn chapters_json(chapters: &[polishcast_core::transcript::Chapter]) -> Value {
    json!(chapters.iter().map(|c| json!({ "start": round(c.start), "title": c.title })).collect::<Vec<_>>())
}

pub(super) fn get_transcript(args: &Value) -> Result<Value, String> {
    use polishcast_core::transcript;
    let t = Take::from(args)?;
    let tr = transcribing::load(&t.dir).ok_or("the take has no transcript yet: call transcribe first")?;
    let p = t.project()?;
    let (timeline, speed) = (p.timeline(), p.speed());
    let text = match args["format"].as_str().unwrap_or("text") {
        "text" => tr.text(),
        "srt" => transcript::srt(&tr.in_video(&timeline, speed)),
        "vtt" => transcript::vtt(&tr.in_video(&timeline, speed)),
        "chapters" => transcript::youtube_chapters(&tr.chapters_in_video(&timeline, speed)),
        other => return Err(format!("format is text, srt, vtt or chapters, not {other}")),
    };
    Ok(json!({ "take": t.name(), "language": tr.language, "transcript": text }))
}

pub(super) fn set_chapters(args: &Value) -> Result<Value, String> {
    use polishcast_core::transcript::{self, Chapter};
    let t = Take::from(args)?;
    let mut tr = transcribing::load(&t.dir).ok_or("the take has no transcript yet: call transcribe first")?;
    if let Some(given) = args["chapters"].as_array() {
        let mut chapters = Vec::with_capacity(given.len());
        for c in given {
            let start = c["start"].as_f64().ok_or("each chapter has a start, in seconds")?;
            let title =
                c["title"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("each chapter has a title")?;
            chapters.push(Chapter { start: start.max(0.0), title: title.to_string() });
        }
        chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
        if let Some(first) = chapters.first_mut() {
            first.start = 0.0;
        }
        tr.chapters = chapters;
    } else if args["detect"].as_bool().unwrap_or(false) {
        tr.chapters = transcript::detect_chapters(&tr.segments, t.project()?.duration);
    }
    tr.save(&transcribing::path(&t.dir)).map_err(|e| format!("can't save the transcript: {e}"))?;
    if args["name_with_ai"].as_bool().unwrap_or(false) {
        tr.chapters = transcribing::name_chapters(&t.dir)?;
    }
    Ok(json!({ "take": t.name(), "chapters": chapters_json(&tr.chapters) }))
}

pub(super) fn send_to_publishkit(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    let video = match args["file"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(f) => PathBuf::from(f),
        None => exports_of(&t.dir)
            .into_iter()
            .filter(|e| e.extension().is_some_and(|x| x == "mp4"))
            .find(|e| polishcast_export::caption_files(e)[0].is_file())
            .ok_or("no export of this take has captions: export it with captions files or burned first")?,
    };
    transcribing::send_to_publishkit(&video, &t.name())?;
    Ok(json!({ "sent": video, "note": "PublishKit's Repurpose window is open with the video and its transcript" }))
}
