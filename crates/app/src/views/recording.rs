//! Driving a recording: the recorder's events, a client's countdown and length, the limits on
//! a forgotten recording, picking an area, and the elapsed time in the menu bar.

use super::tray::Tray;
use super::*;

/// The window's state a recording reads and changes.
#[derive(Clone, Copy)]
pub(super) struct Rec {
    pub status: Signal<Status>,
    pub error: Signal<Option<String>>,
    pub library: Signal<Vec<takes::Entry>>,
    pub selected: Signal<Option<PathBuf>>,
    /// Takes whose recording is being shrunk in the background.
    pub compacting: Signal<HashSet<PathBuf>>,
    /// Why the last compaction or sound mix didn't work.
    pub notice: Signal<Option<String>>,
    pub remote: Signal<Remote>,
    pub settings: Signal<Settings>,
    pub recording_dir: Signal<Option<PathBuf>>,
    pub auto_stopped: Signal<Option<String>>,
    pub picking: Signal<bool>,
    pub windows: Signal<Vec<WindowInfo>>,
    pub chosen_window: Signal<Option<u32>>,
}

/// Reads the windows that can be recorded again, dropping the chosen one if it closed.
pub(super) fn refresh_windows(mut windows: Signal<Vec<WindowInfo>>, mut chosen_window: Signal<Option<u32>>) {
    spawn(async move {
        if let Ok(list) = tokio::task::spawn_blocking(polishcast_capture::windows).await {
            if chosen_window.peek().is_some_and(|id| !list.iter().any(|w| w.id == id)) {
                chosen_window.set(None);
            }
            // Unchanged, the menu is left alone: rebuilding it would close it while open.
            if *windows.peek() != list {
                windows.set(list);
            }
        }
    });
}

/// Recorder events, and the elapsed time in the menu bar. 10 Hz is plenty for both.
#[allow(clippy::too_many_arguments)]
pub(super) fn use_recording(
    rec: Rec,
    recorder: Recorder,
    tray: Rc<Tray>,
    compacted_tx: mpsc::Sender<(PathBuf, Result<u64, String>)>,
    compacted_rx: Rc<mpsc::Receiver<(PathBuf, Result<u64, String>)>>,
    area_rx: Rc<mpsc::Receiver<Option<Area>>>,
    begin: Callback<(PathBuf, Options)>,
    start: Callback<(Source, bool)>,
) {
    let Rec {
        mut status,
        mut error,
        mut library,
        mut selected,
        mut compacting,
        mut notice,
        mut remote,
        settings,
        recording_dir,
        mut auto_stopped,
        mut picking,
        windows,
        chosen_window,
    } = rec;
    let refresh_windows = move || refresh_windows(windows, chosen_window);
    use_future({
        let (recorder, tray) = (recorder.clone(), tray.clone());
        move || {
            let (recorder, tray) = (recorder.clone(), tray.clone());
            let (compacted_tx, compacted_rx) = (compacted_tx.clone(), compacted_rx.clone());
            let area_rx = area_rx.clone();
            async move {
                let (mixed_tx, mixed_rx) = mpsc::channel::<(Take, Option<String>)>();
                // A finished take: its project, then shrinking it in the background.
                let mut saved = move |t: Take| {
                    let project = match takes::create_project(&t) {
                        Ok(p) => Some(p),
                        Err(e) => {
                            error.set(Some(format!("Can't save the project: {e}")));
                            None
                        }
                    };
                    status.set(Status::Idle);
                    // A client's recording: its answer, or the Trash if it was cancelled.
                    let asked = remote.write().current.take_if(|a| a.dir == t.dir);
                    if let Some(mut a) = asked {
                        if a.cancel {
                            let _ = takes::move_to_trash(&t.dir);
                            a.reply_saved(&Ok(json!({ "cancelled": true, "recording_id": a.id })));
                            library.set(takes::list());
                            return;
                        }
                        let result = Ok(finished(&a, &t, project.as_ref()));
                        a.reply_saved(&result);
                        remote.write().last = Some((a.id, result));
                    }
                    library.set(takes::list());
                    if t.compact {
                        compacting.write().insert(t.dir.clone());
                        let (tx, dir) = (compacted_tx.clone(), t.dir.clone());
                        std::thread::spawn(move || {
                            let result = compact(&dir.join(take::SCREEN)).map_err(|e| format!("{e:#}"));
                            let _ = tx.send((dir, result));
                        });
                    }
                    selected.set(Some(t.dir));
                    window().set_visible(true);
                    window().set_focus();
                };
                let mut tick = 0u64;
                loop {
                    tick += 1;
                    // Windows come and go: the list follows while it's on show (every 2 s).
                    if tick.is_multiple_of(20)
                        && *status.peek() == Status::Idle
                        && settings.peek().source == SourceKind::Window
                        && window().is_visible()
                    {
                        refresh_windows();
                    }
                    // A forgotten recording stops before it fills the disk (checked every 2 s).
                    let current = *status.peek();
                    if let (Status::Recording(since), true) = (current, tick.is_multiple_of(20)) {
                        let dir = recording_dir.peek().clone();
                        let size =
                            dir.as_ref().and_then(|d| std::fs::metadata(d.join(take::SCREEN)).ok()).map(|m| m.len());
                        let free = dir.as_deref().and_then(crate::limits::free_space);
                        if let Some(reason) = crate::limits::reason_to_stop(since.elapsed(), size, free) {
                            auto_stopped.set(Some(reason));
                            if let Some(a) = remote.write().current.as_mut() {
                                a.stopped_by.get_or_insert("PolishCast's own limit on length, size or free disk space");
                            }
                            recorder.stop();
                            status.set(Status::Saving);
                        }
                    }
                    // A client's recording: past its countdown, it starts; past its length, it stops.
                    if let Status::Countdown(until) = current {
                        if Instant::now() >= until {
                            let next =
                                remote.write().current.as_mut().and_then(|a| Some((a.dir.clone(), a.options.take()?)));
                            match next {
                                Some(next) => begin(next),
                                None => status.set(Status::Idle),
                            }
                        }
                    }
                    if let Status::Recording(since) = current {
                        let max = remote.peek().current.as_ref().map(|a| (a.max, a.client.clone()));
                        if let Some((max, client)) = max.filter(|(max, _)| since.elapsed() >= *max) {
                            auto_stopped.set(Some(format!(
                                "The recording was stopped after {} s, the length {client} asked for.",
                                max.as_secs()
                            )));
                            if let Some(a) = remote.write().current.as_mut() {
                                a.stopped_by.get_or_insert("max_duration_secs");
                            }
                            recorder.stop();
                            status.set(Status::Saving);
                        }
                    }
                    for request in remote::take_requests() {
                        answer(request, status, *picking.peek(), remote, settings, recording_dir, &recorder);
                    }
                    while let Ok(area) = area_rx.try_recv() {
                        picking.set(false);
                        // The area was dragged: the pointer starts at its corner.
                        if let Some(area) = area {
                            start((Source::Area(area), true));
                        }
                    }
                    while let Some(event) = recorder.try_event() {
                        match event {
                            // Unless Stop was already pressed while it was starting.
                            Event::Started => {
                                if *status.peek() == Status::Starting {
                                    status.set(Status::Recording(Instant::now()));
                                }
                                if let Some(a) = remote.write().current.as_mut() {
                                    a.started_at = Some(chrono::Local::now());
                                    let started = started(a);
                                    a.reply_started(Ok(started));
                                }
                            }
                            // The sound apps played, recorded apart from one window, goes in
                            // before the take opens: its pauses are measured once, from the file.
                            Event::Finished(t) if t.sound.is_some() => {
                                let tx = mixed_tx.clone();
                                std::thread::spawn(move || {
                                    let offset = t.sound.unwrap_or_default();
                                    let file = t.dir.join(take::SCREEN);
                                    let result = sound::mix_in(&file, &t.dir.join(take::SOUND), offset);
                                    let _ = tx.send((t, result.err().map(|e| format!("{e:#}"))));
                                });
                            }
                            Event::Finished(t) => saved(t),
                            Event::Failed(e) => {
                                let asked = remote.write().current.take();
                                if let Some(mut a) = asked {
                                    let failed = Err(e.clone());
                                    a.reply_started(failed.clone());
                                    a.reply_saved(&failed);
                                    let _ = std::fs::remove_dir(&a.dir);
                                    remote.write().last = Some((a.id, failed));
                                }
                                error.set(Some(e));
                                status.set(Status::Idle);
                            }
                        }
                    }
                    while let Ok((t, failed)) = mixed_rx.try_recv() {
                        saved(t);
                        if failed.is_some() {
                            notice.set(failed);
                        }
                    }
                    while let Ok((dir, result)) = compacted_rx.try_recv() {
                        compacting.write().remove(&dir);
                        notice.set(result.err());
                        library.set(takes::list());
                    }
                    let by = remote.peek().current.as_ref().map(|a| a.client.clone());
                    tray.update(*status.peek(), by.as_deref());
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });
}
