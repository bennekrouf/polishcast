//! Changing a take's edits: zooms, clips, blanks and style.

use super::*;

pub(super) fn set_zooms(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    let clicks = t.events().clicks;
    t.change(|p| {
        if args["suggest"].as_bool().unwrap_or(false) {
            p.zooms = auto_zoom(&clicks, p.duration, &AutoZoom::default());
        }
        let count = p.zooms.len();
        let index = |n: u64| -> Result<usize, String> {
            (n as usize).checked_sub(1).filter(|i| *i < count).ok_or(format!("no zoom {n}: the take has {count}"))
        };
        for u in args["update"].as_array().into_iter().flatten() {
            let i = index(u["zoom"].as_u64().ok_or("each update needs its zoom number")?)?;
            let z = p.zooms[i];
            let (start, end) = (u["start"].as_f64().unwrap_or(z.start), u["end"].as_f64().unwrap_or(z.end));
            if start != z.start || end != z.end {
                zoom::set_bounds(&mut p.zooms, i, start, end, p.duration);
            }
            let z = &mut p.zooms[i];
            if let Some(v) = u["scale"].as_f64() {
                z.scale = (v as f32).clamp(1.0, 4.0);
            }
            if let Some(v) = u["x"].as_f64() {
                z.x = (v as f32).clamp(0.0, 1.0);
            }
            if let Some(v) = u["y"].as_f64() {
                z.y = (v as f32).clamp(0.0, 1.0);
            }
        }
        let mut gone: Vec<usize> = numbers(args, "remove").into_iter().map(index).collect::<Result<_, _>>()?;
        gone.sort_unstable();
        gone.dedup();
        for i in gone.into_iter().rev() {
            p.zooms.remove(i);
        }
        let mut refused = vec![];
        for a in args["add"].as_array().into_iter().flatten() {
            let start = a["start"].as_f64().ok_or("each zoom added needs a start")?;
            let end = a["end"].as_f64().unwrap_or(start + 2.5);
            let z = Zoom {
                start: start.clamp(0.0, p.duration),
                end: end.clamp(0.0, p.duration),
                scale: a["scale"].as_f64().map_or(1.8, |v| (v as f32).clamp(1.0, 4.0)),
                x: a["x"].as_f64().map_or(0.5, |v| (v as f32).clamp(0.0, 1.0)),
                y: a["y"].as_f64().map_or(0.5, |v| (v as f32).clamp(0.0, 1.0)),
            };
            if zoom::insert(&mut p.zooms, z).is_none() {
                refused.push(format!("{start} s: starts inside another zoom, or too little room before the next"));
            }
        }
        if refused.is_empty() {
            Ok(())
        } else {
            Err(format!("Nothing was saved. Zooms not added: {}", refused.join("; ")))
        }
    })?;
    Ok(t.describe())
}

pub(super) fn set_clips(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    t.change(|p| {
        for at in args["split_at"].as_array().into_iter().flatten().filter_map(Value::as_f64) {
            clips::split(&mut p.splits, p.duration, at);
        }
        if let Some(from) = args["keep_from"].as_f64() {
            clips::split(&mut p.splits, p.duration, from);
            for c in clips::clips(&p.splits, p.duration).into_iter().filter(|c| c.1 <= from + 1e-6) {
                clips::remove(&mut p.removed, c);
            }
        }
        if let Some(until) = args["keep_until"].as_f64() {
            clips::split(&mut p.splits, p.duration, until);
            for c in clips::clips(&p.splits, p.duration).into_iter().filter(|c| c.0 >= until - 1e-6) {
                clips::remove(&mut p.removed, c);
            }
        }
        let list = clips::clips(&p.splits, p.duration);
        let clip = |n: u64| -> Result<(f64, f64), String> {
            (n as usize)
                .checked_sub(1)
                .and_then(|i| list.get(i).copied())
                .ok_or(format!("no clip {n}: the take has {}", list.len()))
        };
        for n in numbers(args, "remove") {
            let c = clip(n)?;
            if !clips::is_removed(&p.removed, c) {
                toggle_clip(p, c);
            }
        }
        for n in numbers(args, "restore") {
            let c = clip(n)?;
            if clips::is_removed(&p.removed, c) {
                toggle_clip(p, c);
            }
        }
        if p.timeline().duration() <= 0.0 {
            return Err("every clip would be left out: nothing would remain of the video. Nothing was saved.".into());
        }
        Ok(())
    })?;
    Ok(t.describe())
}

pub(super) fn set_blanks(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    let on = args["on"].as_bool().ok_or("on is required")?;
    if !on {
        t.change(|p| {
            p.blanks = None;
            Ok(())
        })?;
        return Ok(t.describe());
    }
    let kept: Option<Loudness> =
        std::fs::read(t.dir.join(take::LOUDNESS)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let loudness = match kept {
        Some(l) => l,
        None => measure(&t.dir)
            .map_err(|e| format!("can't measure the sound: {e:#}"))?
            .ok_or("this recording has no sound, so there are no pauses to find")?,
    };
    t.change(|p| {
        let mut params = p.blanks.unwrap_or_default();
        if let Some(v) = args["threshold_db"].as_f64() {
            params.threshold_db = (v as f32).clamp(-80.0, -10.0);
        }
        if let Some(v) = args["min_pause_secs"].as_f64() {
            params.min_pause_secs = v.clamp(0.2, 10.0);
        }
        cut_blanks(p, &loudness, params);
        Ok(())
    })?;
    Ok(t.describe())
}

pub(super) fn set_style(args: &Value) -> Result<Value, String> {
    let t = Take::from(args)?;
    t.change(|p| {
        let s = &mut p.style;
        if let Some(c) = args["background_color"].as_str() {
            s.background = Background::Color(c.trim().to_string());
        }
        if let Some(g) = args["background_gradient"].as_array() {
            let colors: Vec<String> = g.iter().filter_map(|v| v.as_str().map(|c| c.trim().to_string())).collect();
            let [top, bottom] = <[String; 2]>::try_from(colors).map_err(|_| "background_gradient takes two colours")?;
            s.background = Background::Gradient(top, bottom);
        }
        let number = |key: &str| args[key].as_f64();
        if let Some(v) = number("padding") {
            s.padding = (v as f32).clamp(0.0, 0.3);
        }
        if let Some(v) = number("radius") {
            s.radius = (v as f32).clamp(0.0, 0.1);
        }
        if let Some(v) = args["shadow"].as_bool() {
            s.shadow = v;
        }
        if let Some(a) = args["aspect"].as_str() {
            s.aspect = aspect_of(a)?;
        }
        if let Some(v) = number("cursor_scale") {
            s.cursor_scale = (v as f32).clamp(0.5, 4.0);
        }
        if let Some(v) = number("cursor_smoothing") {
            s.cursor_smoothing = v.clamp(0.0, 0.5);
        }
        if let Some(v) = args["click_effect"].as_bool() {
            s.click_effect = v;
        }
        if let Some(v) = args["hide_idle_cursor"].as_bool() {
            s.hide_idle_cursor = v;
        }
        if let Some(v) = number("speed") {
            if !SPEEDS.contains(&v) {
                return Err(format!("speed is one of {SPEEDS:?}"));
            }
            p.speed = v;
        }
        if let Some(on) = args["reduce_noise"].as_bool() {
            p.denoise = on.then(|| p.denoise.unwrap_or(DENOISE_STRENGTH));
        }
        if let Some(v) = number("noise_strength") {
            p.denoise = Some((v as f32).clamp(0.0, 1.0));
        }
        Ok(())
    })?;
    Ok(t.describe())
}
