//! The inspector's Transcript section: transcribe the take with api0, read what was said, and
//! keep or rename its chapters before they go into an export's captions.

use crate::settings::{AiRegion, Settings as AppSettings};
use crate::transcribing;
use dioxus::prelude::*;
use polishcast_core::transcript::{clock, Transcript};
use polishcast_transcribe::Progress;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

enum State {
    Idle,
    Running {
        progress: Arc<Progress>,
        cancel: Arc<AtomicBool>,
        done: mpsc::Receiver<Result<Transcript, String>>,
    },
    /// Naming the chapters with api0.
    Naming(mpsc::Receiver<Result<(), String>>),
    Failed(String),
}

#[component]
pub(super) fn TranscriptPanel(dir: PathBuf) -> Element {
    let mut transcript = use_signal({
        let dir = dir.clone();
        move || transcribing::load(&dir)
    });
    let mut state = use_signal(|| State::Idle);
    let mut fraction = use_signal(|| 0.0f32);
    let mut region = use_signal(|| AppSettings::load().ai_region);
    let mut copied = use_signal(|| false);

    use_future({
        let dir = dir.clone();
        move || {
            let dir = dir.clone();
            async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    let next = match &*state.peek() {
                        State::Running { progress, done, .. } => {
                            fraction.set(progress.fraction());
                            done.try_recv().ok().map(|r| match r {
                                Ok(t) => {
                                    transcript.set(Some(t));
                                    State::Idle
                                }
                                Err(e) if e == "Cancelled" => State::Idle,
                                Err(e) => State::Failed(e),
                            })
                        }
                        State::Naming(done) => done.try_recv().ok().map(|r| match r {
                            Ok(()) => {
                                transcript.set(transcribing::load(&dir));
                                State::Idle
                            }
                            Err(e) => State::Failed(e),
                        }),
                        _ => None,
                    };
                    if let Some(next) = next {
                        state.set(next);
                    }
                }
            }
        }
    });

    let start = {
        let dir = dir.clone();
        move |_| {
            let (progress, cancel) = (Arc::new(Progress::default()), Arc::new(AtomicBool::new(false)));
            let (tx, done) = mpsc::channel();
            let (dir, p, c) = (dir.clone(), progress.clone(), cancel.clone());
            std::thread::spawn(move || {
                let _ = tx.send(transcribing::transcribe(&dir, None, &p, &c));
            });
            fraction.set(0.0);
            copied.set(false);
            state.set(State::Running { progress, cancel, done });
        }
    };
    let name_with_ai = {
        let dir = dir.clone();
        move |_| {
            let (tx, done) = mpsc::channel();
            let dir = dir.clone();
            std::thread::spawn(move || {
                let _ = tx.send(transcribing::name_chapters(&dir).map(|_| ()));
            });
            state.set(State::Naming(done));
        }
    };
    let rename = {
        let dir = dir.clone();
        move |(i, title): (usize, String)| {
            let mut t = transcript.write();
            if let Some(c) = t.as_mut().and_then(|t| t.chapters.get_mut(i)) {
                c.title = title.trim().to_string();
            }
            if let Some(t) = t.as_ref() {
                let _ = t.save(&transcribing::path(&dir));
            }
        }
    };
    let remove = {
        let dir = dir.clone();
        move |i: usize| {
            let mut t = transcript.write();
            if let Some(t) = t.as_mut() {
                if i < t.chapters.len() {
                    t.chapters.remove(i);
                }
                if let Some(first) = t.chapters.first_mut() {
                    first.start = 0.0;
                }
                let _ = t.save(&transcribing::path(&dir));
            }
        }
    };

    let busy = matches!(*state.read(), State::Running { .. } | State::Naming(_));
    let current = transcript.read().clone();
    rsx! {
        div { class: "export transcript",
            h2 { "Transcript" }
            match &*state.read() {
                State::Running { cancel, .. } => {
                    let cancel = cancel.clone();
                    rsx! {
                        div { class: "progress",
                            div { class: "bar", width: "{fraction() * 100.0}%" }
                        }
                        div { class: "buttons",
                            span { class: "hint", "Transcribing with api0… {fraction() * 100.0:.0}%" }
                            button { onclick: move |_| cancel.store(true, Ordering::Relaxed), "Cancel" }
                        }
                    }
                }
                State::Naming(_) => rsx! { p { class: "hint", "Naming the chapters…" } },
                State::Failed(e) => rsx! { p { class: "error", "{e}" } },
                State::Idle => rsx! {},
            }
            if let Some(t) = current {
                {
                    let words = t.text().split_whitespace().count();
                    let who = if t.has_speakers() { " · You and a guest" } else { "" };
                    let lang = if t.language.is_empty() { String::new() } else { format!(" · {}", t.language.to_uppercase()) };
                    rsx! { p { class: "hint", "{words} words{lang}{who}" } }
                }
                div { class: "transcript-text", "{t.text()}" }
                div { class: "buttons",
                    button {
                        disabled: busy,
                        onclick: {
                            let text = t.text();
                            move |_| {
                                copied.set(arboard::Clipboard::new().and_then(|mut c| c.set_text(text.clone())).is_ok());
                            }
                        },
                        if copied() { "Copied ✓" } else { "Copy text" }
                    }
                    button { disabled: busy, onclick: start.clone(), "Transcribe again" }
                }
                h3 { "Chapters" }
                if t.chapters.is_empty() {
                    p { class: "hint", "None: the take is too short, or says too little, for chapters." }
                } else {
                    ol { class: "chapters",
                        for (i, c) in t.chapters.iter().enumerate() {
                            li { key: "{i}-{c.start}",
                                span { class: "chapter-at", "{clock(c.start)}" }
                                input {
                                    r#type: "text",
                                    value: "{c.title}",
                                    disabled: busy,
                                    onchange: {
                                        let mut rename = rename.clone();
                                        move |e: Event<FormData>| rename((i, e.value()))
                                    },
                                }
                                if i > 0 {
                                    button {
                                        class: "chapter-remove",
                                        title: "Join it to the chapter before",
                                        disabled: busy,
                                        onclick: {
                                            let mut remove = remove.clone();
                                            move |_| remove(i)
                                        },
                                        "×"
                                    }
                                }
                            }
                        }
                    }
                    div { class: "buttons",
                        button { disabled: busy, onclick: name_with_ai.clone(), "Name with AI" }
                    }
                    p { class: "hint", "Times are in the recording; exports move them to match cuts and speed." }
                }
            } else if !busy {
                p { class: "hint",
                    "Captions, chapters and a transcript for PublishKit. Uses the licence's AI credits; the sound is sent to api0 in {region().shown()}."
                }
                div { class: "segmented",
                    for r in AiRegion::ALL {
                        button {
                            class: if region() == r { "on" } else { "" },
                            onclick: move |_| {
                                region.set(r);
                                let mut s = AppSettings::load();
                                s.ai_region = r;
                                s.save();
                            },
                            "{r.shown()}"
                        }
                    }
                }
                button { class: "primary wide", onclick: start, "Transcribe" }
            }
        }
    }
}
