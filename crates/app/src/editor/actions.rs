//! What the editor does at the playhead (zoom, split, trim, music), and the keys and ticks
//! from the preview script that ask for it.

use super::*;

/// The playhead and what is selected on the timeline: one of a zoom, a clip, a piece of music,
/// or a hidden area.
#[derive(Clone, Copy)]
pub(super) struct Sel {
    pub time: Signal<f64>,
    pub selected: Signal<Option<usize>>,
    pub selected_clip: Signal<Option<(f64, f64)>>,
    pub selected_music: Signal<Option<usize>>,
    pub selected_mask: Signal<Option<usize>>,
}

#[derive(Clone, Copy)]
pub(super) struct Actions {
    pub add_zoom: Callback<()>,
    pub split_here: Callback<()>,
    pub trim: Callback<bool>,
    pub add_music: Callback<PathBuf>,
    pub toggle_music: Callback<()>,
    pub add_mask: Callback<()>,
}

pub(super) fn use_actions(dir: &std::path::Path, doc: Doc, events: Rc<EventLog>, sel: Sel) -> Actions {
    let Sel { time, mut selected, mut selected_clip, mut selected_music, mut selected_mask } = sel;
    let add_zoom = use_callback({
        let events = events.clone();
        move |()| {
            let t = time();
            let (x, y) = events.cursor_at(t).unwrap_or((0.5, 0.5));
            let duration = doc.project.peek().duration;
            let z = Zoom { start: t, end: (t + NEW_ZOOM_SECS).min(duration), scale: 1.8, x, y };
            let mut added = None;
            doc.change(|p| added = zoom::insert(&mut p.zooms, z));
            match added {
                Some(i) => {
                    selected_mask.set(None);
                    selected.set(Some(i));
                }
                None => {
                    let mut error = doc.error;
                    error.set(Some("There's no room for a zoom here: move the playhead to a gap.".into()));
                }
            }
        }
    });

    // Splits the recording at the playhead.
    let split_here = use_callback(move |()| {
        let t = time();
        let mut added = false;
        doc.change(|p| added = clips::split(&mut p.splits, p.duration, t));
        if !added {
            let mut error = doc.error;
            error.set(Some("Too close to the start, the end or another split to split here.".into()));
        }
    });
    // Leaves out everything before the playhead (`before`), or everything after it.
    let trim = use_callback(move |before: bool| {
        let t = time();
        doc.change(|p| {
            clips::split(&mut p.splits, p.duration, t);
            let stretch = if before { (0.0, t) } else { (t, p.duration) };
            if stretch.1 - stretch.0 > 1e-6 {
                clips::remove(&mut p.removed, stretch);
            }
        });
        selected_clip.set(None);
    });

    // A song chosen: it starts at the playhead (a piece playing there stops).
    let add_music = use_callback({
        let dir = dir.to_path_buf();
        move |from: PathBuf| {
            let mut error = doc.error;
            let file = match adopt_file(&dir, &from, "music") {
                Ok(file) => file,
                Err(e) => return error.set(Some(format!("Can't use that song: {e}"))),
            };
            let name = from.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| file.clone());
            let t = time();
            let m =
                Music { file, name, start: t, end: f64::INFINITY, from: 0.0, volume: music::VOLUME, fade: music::FADE };
            let mut added = None;
            doc.change(|p| added = music::insert(&mut p.music, m, p.duration));
            match added {
                Some(i) => {
                    selected.set(None);
                    selected_clip.set(None);
                    selected_mask.set(None);
                    selected_music.set(Some(i));
                }
                None => error.set(Some("There's no room for music here: move the playhead.".into())),
            }
        }
    });
    // M: the music stops at the playhead if it's playing there; otherwise the last song before
    // it carries on from where it stopped.
    let toggle_music = use_callback(move |()| {
        let t = time();
        let (playing, last) = {
            let p = doc.project.peek();
            let last =
                p.music.iter().enumerate().rev().find(|(_, m)| m.end <= t).or(p.music.iter().enumerate().next_back());
            let placed = p.placed_music();
            let last = last.map(|(i, m)| {
                let played = placed.iter().find(|pl| pl.index == i).map_or(0.0, |pl| pl.until - pl.at);
                let from = if m.end <= t { m.from + played } else { 0.0 };
                Music { start: t, end: f64::INFINITY, from, ..m.clone() }
            });
            (music::at(&p.music, t), last)
        };
        let mut error = doc.error;
        if let Some(i) = playing {
            doc.change(|p| {
                let start = p.music[i].start;
                if t - start < music::MIN_SECS {
                    p.music.remove(i);
                } else {
                    music::set_bounds(&mut p.music, i, start, t, p.duration);
                }
            });
            selected_music.set(None);
        } else if let Some(m) = last {
            let mut added = None;
            doc.change(|p| added = music::insert(&mut p.music, m, p.duration));
            if added.is_none() {
                error.set(Some("There's no room for music here: move the playhead.".into()));
            }
        } else {
            error.set(Some("Choose a song with + Music first.".into()));
        }
    });
    // Covers a box in the middle of what's on view at the playhead, to drag over what to hide.
    let add_mask = use_callback(move |()| {
        let t = time();
        let mut added = 0;
        doc.change(|p| {
            let view = viewport_at(&p.zooms, t);
            let (w, h) = (view.w * NEW_MASK_SIZE.0, view.h * NEW_MASK_SIZE.1);
            let rect = (view.x + (view.w - w) / 2.0, view.y + (view.h - h) / 2.0, w, h);
            p.masks.push(Mask::new(t, t + NEW_MASK_SECS, rect, Cover::default(), p.duration));
            added = p.masks.len() - 1;
        });
        selected.set(None);
        selected_clip.set(None);
        selected_music.set(None);
        selected_mask.set(Some(added));
    });
    Actions { add_zoom, split_here, trim, add_music, toggle_music, add_mask }
}

/// Ticks and keys from the preview script.
pub(super) fn use_preview_messages(doc: Doc, sel: Sel, mut playing: Signal<bool>, actions: Actions) {
    let Sel { mut time, mut selected, mut selected_clip, mut selected_music, mut selected_mask } = sel;
    let Actions { add_zoom, split_here, trim, toggle_music, add_mask, .. } = actions;
    use_future(move || async move {
        let mut channel = document::eval("window.sv.send = (m) => dioxus.send(m); await new Promise(() => {});");
        while let Ok(msg) = channel.recv::<Msg>().await {
            match msg {
                Msg::Tick { t, playing: p } => {
                    time.set(t);
                    playing.set(p);
                }
                Msg::Pick { index } => {
                    selected.set(None);
                    selected_clip.set(None);
                    selected_music.set(None);
                    selected_mask.set(Some(index));
                }
                Msg::Mask { index, x, y, w, h } => doc.change(|p| {
                    if let Some(m) = p.masks.get_mut(index) {
                        m.set_rect(x, y, w, h);
                    }
                }),
                Msg::Key { key, cmd, shift } => match (key.as_str(), cmd, shift) {
                    ("z", true, false) => doc.undo(),
                    ("z" | "Z", true, true) => doc.redo(),
                    ("z", false, false) => add_zoom(()),
                    ("s", false, false) => split_here(()),
                    ("m", false, false) => toggle_music(()),
                    ("h", false, false) => add_mask(()),
                    ("[", false, _) => trim(true),
                    ("]", false, _) => trim(false),
                    ("Backspace" | "Delete", false, _) => {
                        if let Some(i) = selected_mask.take() {
                            doc.change(|p| {
                                if i < p.masks.len() {
                                    p.masks.remove(i);
                                }
                            });
                        } else if let Some(i) = selected.take() {
                            doc.change(|p| {
                                if i < p.zooms.len() {
                                    p.zooms.remove(i);
                                }
                            });
                        } else if let Some(i) = selected_music.take() {
                            doc.change(|p| {
                                if i < p.music.len() {
                                    p.music.remove(i);
                                }
                            });
                        } else if let Some(clip) = *selected_clip.peek() {
                            doc.change(|p| toggle_clip(p, clip));
                        }
                    }
                    ("Escape", false, _) => {
                        selected.set(None);
                        selected_clip.set(None);
                        selected_music.set(None);
                        selected_mask.set(None);
                    }
                    _ => {}
                },
            }
        }
    });
}
