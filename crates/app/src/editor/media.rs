//! What the editor serves to its web view and keeps it in step with: the background picture,
//! the songs and where they play, the music lowered under the voice, and the frame track.

use super::*;

/// The background picture, if any, served next to the video: its file name and URL. Also
/// serves the songs, measures the recording's loudness when needed, and hands the preview
/// where the music plays.
pub(super) fn use_served_media(
    dir: &std::path::Path,
    doc: Doc,
    loudness: Signal<Measure>,
    in_app: Signal<bool>,
) -> Signal<Option<(String, String)>> {
    let mut picture = use_signal(|| Option::<(String, String)>::None);
    use_effect({
        let dir = dir.to_path_buf();
        move || {
            let name = match &doc.project.read().style.background {
                Background::Image(name) => Some(name.clone()),
                _ => None,
            };
            let current = picture.peek().clone();
            if current.as_ref().map(|c| &c.0) != name.as_ref() {
                if let Some((_, url)) = current {
                    media_server::unpublish(&url);
                }
                picture.set(name.and_then(|n| media_server::share(&dir.join(&n)).map(|url| (n, url))));
            }
        }
    });
    // The songs, served next to the video by file name, and where each piece plays.
    let mut songs = use_signal(HashMap::<String, String>::new);
    use_effect({
        let dir = dir.to_path_buf();
        move || {
            let files: Vec<String> = doc.project.read().music.iter().map(|m| m.file.clone()).collect();
            for file in files {
                if !songs.peek().contains_key(&file) {
                    if let Some(url) = media_server::share(&dir.join(&file)) {
                        songs.write().insert(file, url);
                    }
                }
            }
        }
    });
    // Lowering the music under the voice needs the recording's loudness, and so does knowing
    // whether there's any sound to take noise out of: measured once, and again when Try again
    // puts it back to Unknown.
    use_effect({
        let dir = dir.to_path_buf();
        move || {
            let wanted = {
                let p = doc.project.read();
                p.duck.is_some() || p.denoise.is_some()
            };
            if wanted && *loudness.read() == Measure::Unknown {
                let (dir, mut loudness) = (dir.clone(), loudness);
                loudness.set(Measure::Measuring);
                spawn(async move {
                    loudness.set(match tokio::task::spawn_blocking(move || measure(&dir)).await {
                        Ok(Ok(Some(l))) => Measure::Ready(Rc::new(l)),
                        Ok(Ok(None)) => Measure::Silent,
                        Ok(Err(e)) => Measure::failed(&e),
                        Err(e) => Measure::Failed(e.to_string()),
                    });
                });
            }
        }
    });
    use_effect(move || {
        let project = doc.project.read();
        let urls = songs.read();
        let list: Vec<serde_json::Value> = project
            .placed_music()
            .into_iter()
            .filter_map(|m| {
                let url = urls.get(&project.music[m.index].file)?;
                let url = if in_app() { media_server::in_app_path(url) } else { url.clone() };
                let mut v = serde_json::to_value(m).ok()?;
                v["url"] = url.into();
                Some(v)
            })
            .collect();
        let duck = match (project.duck, &*loudness.read()) {
            (Some(level), Measure::Ready(l)) => serde_json::json!({
                "level": level,
                "ramp": music::DUCK_RAMP,
                "spans": project.speaking(l),
            }),
            _ => serde_json::Value::Null,
        };
        document::eval(&format!("window.sv && window.sv.setMusic({}, {duck})", serde_json::Value::from(list)));
    });
    use_asset_handler("media", move |request, responder| {
        std::thread::spawn(move || responder.respond(media_server::in_app_response(&request)));
    });
    picture
}

/// The frame track, recomputed whenever the project changes, and the preview script it goes to.
pub(super) fn use_preview_track(doc: Doc, events: Rc<EventLog>) {
    let track = use_memo({
        let events = events.clone();
        move || {
            let scene = Scene::new(doc.project.read().clone(), &events);
            let (w, h) = scene.output_size(TRACK_LONG_SIDE);
            serde_json::to_string(&scene.track(TRACK_FPS, w, h)).unwrap_or_default()
        }
    });
    use_hook(|| {
        document::eval(PREVIEW_JS);
    });
    use_effect(move || {
        document::eval(&format!("window.sv && window.sv.setTrack({})", track()));
    });
}
