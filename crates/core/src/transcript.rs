//! What is said in a take: its transcript, the chapters found in it, and the caption files made
//! from both for an export.
//!
//! The transcript is kept in recording time, like the zooms, so it survives every edit; an
//! export maps it onto the video's own time ([`Transcript::in_video`]), leaving out what was cut
//! and following the playback speed.

use crate::blanks::Timeline;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Who speaks, told apart by the sound track it was heard on: the microphone is the person
/// recording, the sound apps played (a call, a video) someone else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Speaker {
    You,
    Guest,
}

impl Speaker {
    pub fn label(self) -> &'static str {
        match self {
            Speaker::You => "You",
            Speaker::Guest => "Guest",
        }
    }
}

/// A caption-sized stretch of speech, `[start, end)` in seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Speaker>,
}

/// One word and when it is said, as a speech model times it.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Timed words grouped into caption lines of at most `max_chars`: a new line after a sentence
/// ends, at a pause, or when the line is full or has been on screen for seven seconds.
pub fn captions(words: &[Word], max_chars: usize) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut line: Option<Segment> = None;
    for w in words {
        let text = w.text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(l) = &line {
            let full = l.text.chars().count() + 1 + text.chars().count() > max_chars;
            let paused = w.start - l.end > 0.8;
            let ended = l.text.ends_with(['.', '?', '!']) && l.text.chars().count() >= 15;
            let long = w.end - l.start > 7.0;
            if full || paused || ended || long {
                out.extend(line.take());
            }
        }
        match &mut line {
            Some(l) => {
                l.text.push(' ');
                l.text.push_str(text);
                l.end = w.end.max(l.end);
            }
            None => line = Some(Segment { start: w.start, end: w.end, text: text.to_string(), speaker: None }),
        }
    }
    out.extend(line);
    out
}

/// Where a chapter starts, and what it's called.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub start: f64,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// The language heard, as whisper names it: `en`, `fr`.
    pub language: String,
    /// The model that wrote it.
    pub model: String,
    pub segments: Vec<Segment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<Chapter>,
}

impl Transcript {
    pub fn load(path: &Path) -> Option<Transcript> {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Whether more than one person is heard.
    pub fn has_speakers(&self) -> bool {
        let mut seen = self.segments.iter().filter_map(|s| s.speaker);
        seen.next().is_some_and(|first| seen.any(|s| s != first))
    }

    /// The segments as the video shows them: in video time, those inside a cut left out, those
    /// across one shortened, all at the playback `speed`.
    pub fn in_video(&self, timeline: &Timeline, speed: f64) -> Vec<Segment> {
        let speed = if speed > 0.0 { speed } else { 1.0 };
        self.segments
            .iter()
            .filter_map(|s| {
                let (start, end) = (timeline.to_output(s.start), timeline.to_output(s.end));
                // Under a fifth of a second left: it was cut.
                (end - start >= 0.2).then(|| Segment {
                    start: start / speed,
                    end: end / speed,
                    text: s.text.clone(),
                    speaker: s.speaker,
                })
            })
            .collect()
    }

    /// The chapters in video time: the first at 0:00, none closer than YouTube allows to the
    /// one before.
    pub fn chapters_in_video(&self, timeline: &Timeline, speed: f64) -> Vec<Chapter> {
        let speed = if speed > 0.0 { speed } else { 1.0 };
        let mut out: Vec<Chapter> = Vec::new();
        for c in &self.chapters {
            let start = if out.is_empty() { 0.0 } else { timeline.to_output(c.start) / speed };
            if out.last().is_some_and(|last| start - last.start < MIN_CHAPTER_SECS) {
                continue;
            }
            out.push(Chapter { start, title: c.title.clone() });
        }
        out
    }

    /// The words as prose, for reading or drafting from: a paragraph per speaker turn or long
    /// pause, each named after its speaker when more than one is heard.
    pub fn text(&self) -> String {
        paragraphs(&self.segments, self.has_speakers())
    }
}

/// YouTube refuses chapters shorter than this.
pub const MIN_CHAPTER_SECS: f64 = 10.0;

fn paragraphs(segments: &[Segment], named: bool) -> String {
    let mut out = String::new();
    let mut last: Option<&Segment> = None;
    for s in segments {
        let text = s.text.trim();
        if text.is_empty() {
            continue;
        }
        let turn = last.is_none_or(|l| l.speaker != s.speaker || s.start - l.end > 2.0);
        if turn {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            if let (true, Some(who)) = (named, s.speaker) {
                out.push_str(who.label());
                out.push_str(": ");
            }
        } else {
            out.push(' ');
        }
        out.push_str(text);
        last = Some(s);
    }
    out
}

/// `1:02:03,450` in SRT, `1:02:03.450` in WebVTT.
fn stamp(secs: f64, sep: char) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    let (h, m, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    format!("{h:02}:{m:02}:{s:02}{sep}{ms:03}")
}

/// The caption's text, its speaker named when more than one is heard: `[Guest] …`.
fn caption(s: &Segment, named: bool) -> String {
    let text = s.text.trim();
    match (named, s.speaker) {
        (true, Some(who)) => format!("[{}] {text}", who.label()),
        _ => text.to_string(),
    }
}

fn named(segments: &[Segment]) -> bool {
    let mut seen = segments.iter().filter_map(|s| s.speaker);
    seen.next().is_some_and(|first| seen.any(|s| s != first))
}

/// SubRip captions, as YouTube, LinkedIn and most players take them.
pub fn srt(segments: &[Segment]) -> String {
    let named = named(segments);
    let mut out = String::new();
    for (i, s) in segments.iter().filter(|s| !s.text.trim().is_empty()).enumerate() {
        out += &format!("{}\n{} --> {}\n{}\n\n", i + 1, stamp(s.start, ','), stamp(s.end, ','), caption(s, named));
    }
    out
}

/// WebVTT captions, for the web and TikTok.
pub fn vtt(segments: &[Segment]) -> String {
    let named = named(segments);
    let mut out = String::from("WEBVTT\n\n");
    for s in segments.iter().filter(|s| !s.text.trim().is_empty()) {
        out += &format!("{} --> {}\n{}\n\n", stamp(s.start, '.'), stamp(s.end, '.'), caption(s, named));
    }
    out
}

/// Chapters as a YouTube description lists them: `0:00 Title`, one per line.
pub fn youtube_chapters(chapters: &[Chapter]) -> String {
    chapters.iter().map(|c| format!("{} {}\n", clock(c.start), c.title)).collect()
}

/// `1:02`, or `1:02:03` past the hour.
pub fn clock(secs: f64) -> String {
    let t = secs.max(0.0).floor() as u64;
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Chapters found in what is said, on this computer: a new one where a long pause and a change
/// of words meet, about one every two minutes, none shorter than a minute; each titled from its
/// first sentence. A recording under two minutes has none.
pub fn detect_chapters(segments: &[Segment], duration: f64) -> Vec<Chapter> {
    let spoken: Vec<&Segment> = segments.iter().filter(|s| !s.text.trim().is_empty()).collect();
    if duration < 120.0 || spoken.len() < 4 {
        return Vec::new();
    }
    let wanted = ((duration / 120.0).round() as usize).clamp(3, 12);
    let min_len = (duration / (wanted as f64 * 2.0)).max(60.0);
    // Each gap between two segments, scored by its pause and how different the words are on
    // either side of it within a minute.
    let mut candidates: Vec<(f64, f64)> = (1..spoken.len())
        .filter_map(|i| {
            let at = spoken[i].start;
            if at < min_len || duration - at < min_len {
                return None;
            }
            let pause = (spoken[i].start - spoken[i - 1].end).clamp(0.0, 3.0) / 3.0;
            let before = words_between(&spoken, at - 60.0, at);
            let after = words_between(&spoken, at, at + 60.0);
            let shift = 1.0 - cosine(&before, &after);
            let turn = if spoken[i].speaker != spoken[i - 1].speaker { 0.1 } else { 0.0 };
            Some((at, 0.4 * pause + 0.6 * shift + turn))
        })
        .collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut starts = vec![0.0];
    for (at, _) in candidates {
        if starts.len() == wanted {
            break;
        }
        if starts.iter().all(|s: &f64| (s - at).abs() >= min_len) {
            starts.push(at);
        }
    }
    if starts.len() < 3 {
        return Vec::new();
    }
    starts.sort_by(f64::total_cmp);
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(duration);
            let text: Vec<&str> =
                spoken.iter().filter(|s| s.start >= start && s.start < end).map(|s| s.text.trim()).collect();
            Chapter { start, title: title_of(&text.join(" ")) }
        })
        .collect()
}

/// A title from a chapter's first sentence: its filler words left out, at most six words.
pub fn title_of(text: &str) -> String {
    const FILLER: &[&str] = &[
        "so", "ok", "okay", "um", "uh", "well", "right", "now", "and", "but", "alright", "yeah", "alors", "donc",
        "euh", "bon", "voilà", "et", "mais",
    ];
    let sentence = text.split(['.', '?', '!']).map(str::trim).find(|s| !s.is_empty()).unwrap_or_default();
    let words: Vec<&str> = sentence
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| c == ',' || c == ';' || c == ':'))
        .skip_while(|w| FILLER.contains(&w.to_lowercase().as_str()))
        .filter(|w| !w.is_empty())
        .take(6)
        .collect();
    let mut title = words.join(" ");
    if let Some(first) = title.chars().next() {
        title.replace_range(..first.len_utf8(), &first.to_uppercase().to_string());
    }
    if title.is_empty() {
        "Chapter".into()
    } else {
        title
    }
}

/// The words said between `from` and `to`, lowercased, short ones left out.
fn words_between(spoken: &[&Segment], from: f64, to: f64) -> Vec<String> {
    spoken
        .iter()
        .filter(|s| s.end > from && s.start < to)
        .flat_map(|s| s.text.split(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.chars().count() >= 4)
        .map(str::to_lowercase)
        .collect()
}

/// How alike two bags of words are, from 0 (nothing shared) to 1.
fn cosine(a: &[String], b: &[String]) -> f64 {
    use std::collections::HashMap;
    fn count(words: &[String]) -> HashMap<&str, f64> {
        let mut m: HashMap<&str, f64> = HashMap::new();
        for w in words {
            *m.entry(w.as_str()).or_default() += 1.0;
        }
        m
    }
    let (ca, cb) = (count(a), count(b));
    let dot: f64 = ca.iter().filter_map(|(w, x)| cb.get(w).map(|y| x * y)).sum();
    let norm = |m: &HashMap<&str, f64>| m.values().map(|v| v * v).sum::<f64>().sqrt();
    let n = norm(&ca) * norm(&cb);
    if n == 0.0 {
        0.0
    } else {
        dot / n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> Segment {
        Segment { start, end, text: text.into(), speaker: None }
    }

    #[test]
    fn captions_are_written_in_both_formats() {
        let segs = vec![seg(1.0, 3.5, "Hello."), seg(3661.25, 3662.0, " Later. ")];
        assert_eq!(
            srt(&segs),
            "1\n00:00:01,000 --> 00:00:03,500\nHello.\n\n2\n01:01:01,250 --> 01:01:02,000\nLater.\n\n"
        );
        assert_eq!(
            vtt(&segs),
            "WEBVTT\n\n00:00:01.000 --> 00:00:03.500\nHello.\n\n01:01:01.250 --> 01:01:02.000\nLater.\n\n"
        );
    }

    #[test]
    fn speakers_are_named_only_when_there_are_two() {
        let mut segs = vec![seg(0.0, 1.0, "Hi."), seg(1.0, 2.0, "Hey.")];
        segs[0].speaker = Some(Speaker::You);
        segs[1].speaker = Some(Speaker::You);
        assert!(srt(&segs).contains("\nHi.\n"));
        segs[1].speaker = Some(Speaker::Guest);
        assert!(srt(&segs).contains("\n[Guest] Hey.\n"));
        let t = Transcript { segments: segs, ..Transcript::default() };
        assert_eq!(t.text(), "You: Hi.\n\nGuest: Hey.");
    }

    #[test]
    fn the_video_leaves_out_what_was_cut_and_follows_the_speed() {
        let t = Transcript {
            segments: vec![seg(0.0, 4.0, "kept"), seg(5.0, 7.0, "cut"), seg(9.0, 12.0, "after")],
            chapters: vec![
                Chapter { start: 0.5, title: "A".into() },
                Chapter { start: 9.0, title: "Too close".into() },
                Chapter { start: 40.0, title: "B".into() },
            ],
            ..Transcript::default()
        };
        // 4 to 8 is cut.
        let tl = Timeline { parts: vec![(0.0, 4.0), (8.0, 60.0)] };
        let v = t.in_video(&tl, 2.0);
        assert_eq!(v.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(), vec![(0.0, 2.0), (2.5, 4.0)]);
        let ch = t.chapters_in_video(&tl, 1.0);
        assert_eq!(ch.iter().map(|c| (c.start, c.title.as_str())).collect::<Vec<_>>(), vec![(0.0, "A"), (36.0, "B")]);
        assert_eq!(youtube_chapters(&ch), "0:00 A\n0:36 B\n");
    }

    #[test]
    fn chapters_break_where_the_subject_changes() {
        // Six minutes: two minutes each on cats, invoices and rockets, with a pause before each.
        let mut segs = Vec::new();
        for (i, topic) in
            ["cats purring whiskers", "invoices payment accounting", "rockets launch orbit"].iter().enumerate()
        {
            let base = i as f64 * 120.0;
            for k in 0..12 {
                let at = base + 2.5 + k as f64 * 9.8;
                let text = if k == 0 { format!("So, {topic} matter. More words.") } else { format!("{topic} again") };
                segs.push(seg(at, at + 8.0, &text));
            }
        }
        let ch = detect_chapters(&segs, 360.0);
        assert_eq!(ch.len(), 3, "{ch:?}");
        assert_eq!(ch[0].start, 0.0);
        assert!((ch[1].start - 122.5).abs() < 0.01 && (ch[2].start - 242.5).abs() < 0.01, "{ch:?}");
        assert_eq!(ch[1].title, "Invoices payment accounting matter");
        assert!(detect_chapters(&segs[..6], 60.0).is_empty());
    }

    #[test]
    fn words_are_grouped_into_caption_lines() {
        let w = |start: f64, text: &str| Word { start, end: start + 0.3, text: text.into() };
        let words = vec![
            w(0.0, "Hello"),
            w(0.4, "everyone,"),
            w(0.8, "welcome"),
            w(1.2, "back."),
            w(1.6, "Today"),
            w(2.0, "we"),
            w(4.0, "ship."),
        ];
        let lines: Vec<String> = captions(&words, 42).into_iter().map(|s| s.text).collect();
        assert_eq!(lines, ["Hello everyone, welcome back.", "Today we", "ship."]);
        let narrow: Vec<String> = captions(&words[..4], 15).into_iter().map(|s| s.text).collect();
        assert_eq!(narrow, ["Hello everyone,", "welcome back."]);
    }

    #[test]
    fn a_title_skips_the_filler() {
        assert_eq!(
            title_of("okay, so now let's look at the billing page in detail. Then"),
            "Let's look at the billing page"
        );
        assert_eq!(title_of("..."), "Chapter");
        assert_eq!(clock(3725.9), "1:02:05");
    }
}
