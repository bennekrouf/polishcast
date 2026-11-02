//! Answering an MCP client: its start, stop and cancel requests, and what a recording it
//! asked for came to.

use super::*;

pub(super) fn folder_name(dir: &std::path::Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// start_recording's answer, once the first frame is written.
pub(super) fn started(a: &Asked) -> Value {
    json!({
        "recording_id": a.id,
        "take": a.id,
        "started_at": a.started_at.map(|t| t.to_rfc3339()),
        "source": a.source_label,
        "max_duration_secs": a.max.as_secs(),
    })
}

/// stop_recording's answer, once the take is saved.
pub(super) fn finished(a: &Asked, t: &Take, project: Option<&polishcast_core::Project>) -> Value {
    let events: EventLog =
        std::fs::read(t.dir.join(take::EVENTS)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    json!({
        "recording_id": a.id,
        "take": a.id,
        "folder": t.dir,
        "recording_secs": (t.duration * 100.0).round() / 100.0,
        "clicks": events.clicks.len(),
        "zooms_suggested": project.map_or(0, |p| p.zooms.len()),
        "stopped_by": a.stopped_by.unwrap_or("the client"),
    })
}

/// Answers a request from an MCP client (`remote`): starts, stops or cancels its recording, or
/// says what's being recorded.
pub(super) fn answer(
    request: remote::Request,
    mut status: Signal<Status>,
    picking: bool,
    mut remote: Signal<Remote>,
    settings: Signal<Settings>,
    recording_dir: Signal<Option<PathBuf>>,
    recorder: &Recorder,
) {
    let remote::Request { op, client, reply } = request;
    let current = *status.peek();
    let by_person = remote.peek().current.is_none() && (current != Status::Idle || picking);
    let in_progress = {
        let id = remote.peek().current.as_ref().map(|a| a.id.clone());
        match id {
            Some(id) => {
                format!("A recording is already in progress ({id}). stop_recording or cancel_recording ends it.")
            }
            None => "The person is recording, or choosing what to record, from PolishCast. They stop it from the \
                     menu bar icon; it can't be stopped from here."
                .into(),
        }
    };
    let cancel = matches!(op, Op::Cancel { .. });
    match op {
        Op::Status => {
            let state = match current {
                Status::Idle if picking => "choosing_area",
                Status::Idle => "idle",
                Status::Countdown(_) => "counting_down",
                Status::Starting => "starting",
                Status::Recording(_) => "recording",
                Status::Saving => "saving",
            };
            let mut v = json!({ "state": state, "recording_allowed": Settings::load().allow_mcp_recording });
            if let Some(a) = remote.peek().current.as_ref() {
                v["recording_id"] = json!(a.id);
                v["started_by"] = json!(a.client);
                v["source"] = a.source_label.clone();
                v["max_duration_secs"] = json!(a.max.as_secs());
                v["started_at"] = json!(a.started_at.map(|t| t.to_rfc3339()));
            } else if by_person {
                v["recording_id"] = json!(recording_dir.peek().as_deref().map(folder_name));
                v["started_by"] = json!("the person");
                v["source"] = json!({ "kind": match settings.peek().source {
                    SourceKind::Screen => "display",
                    SourceKind::Window => "window",
                    SourceKind::Area => "region",
                } });
            }
            match current {
                Status::Countdown(until) => {
                    v["countdown_left_secs"] =
                        json!(until.saturating_duration_since(Instant::now()).as_secs_f64().ceil())
                }
                Status::Recording(since) => v["elapsed_secs"] = json!(since.elapsed().as_secs()),
                _ => {}
            }
            if let Some((id, result)) = &remote.peek().last {
                v["last_recording"] = match result {
                    Ok(done) => done.clone(),
                    Err(e) => json!({ "recording_id": id, "failed": e }),
                };
            }
            let _ = reply.send(Ok(v));
        }
        Op::Start(_) if current != Status::Idle || picking => {
            let _ = reply.send(Err(in_progress));
        }
        Op::Start(s) => {
            let dir = match &s.name {
                Some(name) => takes::new_named_dir(name),
                None => takes::new_dir(),
            };
            let dir = match dir {
                Ok(dir) => dir,
                Err(e) => {
                    let _ = reply.send(Err(format!("Can't create the take's folder: {e}")));
                    return;
                }
            };
            let system_audio = s.system_audio.unwrap_or(settings.peek().system_audio);
            let options = Options {
                source: s.source,
                microphone: s.microphone,
                microphone_device: s.microphone_device,
                system_audio,
                ..Options::default()
            };
            remote.write().current = Some(Asked {
                id: folder_name(&dir),
                dir,
                client: remote::client_label(&client),
                source_label: s.source_label,
                max: s.max,
                options: Some(options),
                started_at: None,
                on_started: Some(reply),
                on_saved: Vec::new(),
                stopped_by: None,
                cancel: false,
            });
            // The loop starts it once the countdown is over, the next tick if there is none.
            status.set(Status::Countdown(Instant::now() + s.countdown));
        }
        Op::Stop { id } | Op::Cancel { id } => {
            let mut r = remote.write();
            let Some(a) = r.current.as_mut().filter(|a| a.is(&id)) else {
                let answer = match (&r.current, &r.last) {
                    (Some(a), _) => {
                        Err(format!("no recording {}: the one in progress is {}", id.unwrap_or_default(), a.id))
                    }
                    (None, _) if by_person => Err(in_progress.clone()),
                    (None, Some((last, result))) if !cancel && id.as_ref().is_none_or(|i| i == last) => result.clone(),
                    (None, Some((last, _))) if cancel && id.as_ref().is_none_or(|i| i == last) => {
                        Err(format!("{last} is already saved: it can't be cancelled any more"))
                    }
                    _ => Err("No recording is in progress.".into()),
                };
                let _ = reply.send(answer);
                return;
            };
            match current {
                // Nothing recorded yet: stopping is cancelling.
                Status::Countdown(_) | Status::Idle => {
                    if let Some(mut a) = r.current.take() {
                        a.reply_started(Err("The recording was cancelled during the countdown.".into()));
                        let _ = std::fs::remove_dir(&a.dir);
                        let _ = reply.send(Ok(json!({
                            "cancelled": true,
                            "recording_id": a.id,
                            "note": "Stopped during the countdown: nothing was recorded.",
                        })));
                    }
                    status.set(Status::Idle);
                }
                Status::Starting | Status::Recording(_) => {
                    a.cancel |= cancel;
                    a.stopped_by.get_or_insert(if cancel { "cancel_recording" } else { "stop_recording" });
                    a.on_saved.push(reply);
                    recorder.stop();
                    status.set(Status::Saving);
                }
                Status::Saving => {
                    a.cancel |= cancel;
                    a.on_saved.push(reply);
                }
            }
        }
    }
}
