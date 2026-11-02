//! Exporting a take, whole or one clip.

use super::*;

pub(super) fn export_clip(args: &Value) -> Result<Value, String> {
    let t = Take::from(args).or_else(|_| Take::of_video(args))?;
    let (start, end) = match (args["start"].as_f64(), args["end"].as_f64()) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => return Err("start and end are seconds of the video, end after start".into()),
    };
    let p = t.project()?;
    let mut clip = p
        .clipped(start, end)
        .ok_or(format!("{start}–{end} s isn't in the video, which is {:.1} s long", p.video_duration()))?;
    clip.style.aspect = aspect_of(args["aspect"].as_str().unwrap_or("9:16"))?;
    let long_side = args["size"].as_u64().map_or(1920, |n| n as u32);
    if !RESOLUTIONS.iter().any(|(px, _)| *px == long_side) {
        let allowed: Vec<u32> = RESOLUTIONS.iter().map(|(px, _)| *px).collect();
        return Err(format!("size is one of {allowed:?}"));
    }
    let transcript = transcribing::load(&t.dir).is_some();
    let captions = match args["captions"].as_str() {
        None if transcript => Captions::Burned,
        None => Captions::Off,
        Some(key) => Captions::from_key(key).ok_or(format!("captions is off, files or burned, not {key}"))?,
    };
    if captions != Captions::Off && !transcript {
        return Err("the take has no transcript yet: call transcribe first, or captions off".into());
    }
    let name = t.name();
    if !licence::allows_export(&name) {
        return Err(format!(
            "The free version of PolishCast exports {} different takes, and they are used. PolishCast Pro exports \
             every take: {}",
            polishcast_core::license::FREE_EXPORTS,
            licence::buy_url()
        ));
    }
    let settings = Settings { format: Format::Mp4, long_side, fps: 30, mute: false, captions };
    let file_name = match args["name"].as_str() {
        Some(typed) => clean_export_name(typed, Format::Mp4).ok_or("name is empty")?,
        None => {
            let now = chrono::Local::now().naive_local();
            let full = export_name(&name, settings, clip.style.aspect, clip.speed, clip.blanks.is_some(), now);
            format!("{name} clip {start:.1}s-{end:.1}s{}", &full[name.len()..])
        }
    };
    let out = export_path(&t.dir, &file_name, Format::Mp4);
    let started = std::time::Instant::now();
    polishcast_export::export_project(&t.dir, clip, &out, settings, &Progress::default(), &AtomicBool::new(false))
        .map_err(|e| format!("{e:#}"))?;
    licence::count_export(&name);
    let mut v = json!({
        "file": out,
        "take": name,
        "seconds": ((end - start) * 10.0).round() / 10.0,
        "size_mb": std::fs::metadata(&out).map_or(0.0, |m| (m.len() as f64 / 1e5).round() / 10.0),
        "took_secs": started.elapsed().as_secs(),
    });
    if captions != Captions::Off {
        let written: Vec<PathBuf> =
            polishcast_export::caption_files(&out).into_iter().filter(|f| f.is_file()).collect();
        v["captions"] = json!(written);
    }
    Ok(v)
}

pub(super) fn export(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    let (format, sizes, rates, defaults) = match args["format"].as_str() {
        Some("mp4") => (Format::Mp4, RESOLUTIONS, FRAME_RATES, Settings::default()),
        Some("gif") => (
            Format::Gif,
            GIF_SIZES,
            GIF_FRAME_RATES,
            Settings { format: Format::Gif, long_side: 960, fps: 15, mute: false, ..Settings::default() },
        ),
        _ => return Err("format is mp4 or gif".into()),
    };
    let long_side = args["size"].as_u64().map_or(defaults.long_side, |n| n as u32);
    if !sizes.iter().any(|(px, _)| *px == long_side) {
        let allowed: Vec<u32> = sizes.iter().map(|(px, _)| *px).collect();
        return Err(format!("size is one of {allowed:?} for this format"));
    }
    let fps = args["fps"].as_u64().map_or(defaults.fps, |n| n as u32);
    if !rates.contains(&fps) {
        return Err(format!("fps is one of {rates:?} for this format"));
    }
    let name = t.name();
    if !licence::allows_export(&name) {
        return Err(format!(
            "The free version of PolishCast exports {} different takes, and they are used. Exporting one of them \
             again is free. PolishCast Pro exports every take: {}",
            polishcast_core::license::FREE_EXPORTS,
            licence::buy_url()
        ));
    }
    let mute = args["mute"].as_bool().unwrap_or(false);
    if mute && format == Format::Gif {
        return Err("mute is for MP4: a GIF never has sound".into());
    }
    let captions = match args["captions"].as_str() {
        None => Captions::Off,
        Some(key) => Captions::from_key(key).ok_or(format!("captions is off, files or burned, not {key}"))?,
    };
    if captions != Captions::Off && format == Format::Gif {
        return Err("captions are for MP4".into());
    }
    if captions != Captions::Off && transcribing::load(&t.dir).is_none() {
        return Err("the take has no transcript yet: call transcribe first".into());
    }
    let settings = Settings { format, long_side, fps, mute, captions };
    let file_name = match args["name"].as_str() {
        Some(typed) => clean_export_name(typed, format).ok_or("name is empty")?,
        None => {
            let p = t.project()?;
            let now = chrono::Local::now().naive_local();
            export_name(&name, settings, p.style.aspect, p.speed, p.blanks.is_some(), now)
        }
    };
    let out = export_path(&t.dir, &file_name, format);
    let started = std::time::Instant::now();
    polishcast_export::export(&t.dir, &out, settings, &Progress::default(), &AtomicBool::new(false))
        .map_err(|e| format!("{e:#}"))?;
    licence::count_export(&name);
    let mut v = json!({
        "file": out,
        "size_mb": std::fs::metadata(&out).map_or(0.0, |m| (m.len() as f64 / 1e5).round() / 10.0),
        "took_secs": started.elapsed().as_secs(),
    });
    if captions != Captions::Off {
        let written: Vec<PathBuf> =
            polishcast_export::caption_files(&out).into_iter().filter(|f| f.is_file()).collect();
        v["captions"] = json!(written);
    }
    if let Some(left) = licence::free_exports_left() {
        v["free_exports_left"] = json!(left);
    }
    Ok(v)
}
