//! End to end: a generated take through ffmpeg, the compositor and back. Skipped (passes) when
//! ffmpeg isn't installed.

use polishcast_core::{take, Button, Click, CursorSample, EventLog, Project};
use polishcast_export::{export, ffmpeg, Format, Progress, Settings};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

fn make_take(dir: &Path, ffmpeg: &Path) {
    let status = Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x200:rate=30:duration=2"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(dir.join(take::SCREEN))
        .status()
        .unwrap();
    assert!(status.success());
    let events = EventLog {
        cursor: (0..60).map(|i| CursorSample { t: i as f64 / 30.0, x: 0.2 + i as f32 / 100.0, y: 0.5 }).collect(),
        clicks: vec![Click { t: 1.0, x: 0.5, y: 0.5, button: Button::Left }],
    };
    std::fs::write(dir.join(take::EVENTS), serde_json::to_vec(&events).unwrap()).unwrap();
    Project::new(2.0, 320, 200, &events).save(&dir.join(take::PROJECT)).unwrap();
}

/// ffprobe next to ffmpeg, or on the PATH: the ffmpeg the app downloads comes without it, and
/// the checks that need it would quietly pass.
fn ffprobe(ffmpeg: &Path) -> std::path::PathBuf {
    let name = ffmpeg.with_file_name("ffprobe").file_name().unwrap().to_owned();
    let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    std::iter::once(ffmpeg.with_file_name(&name))
        .chain(path.into_iter().map(|dir| dir.join(&name)))
        .find(|p| p.is_file())
        .unwrap_or_else(|| ffmpeg.with_file_name(&name))
}

fn probe(ffprobe: &Path, file: &Path, entries: &str) -> String {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-count_frames", "-show_entries", entries, "-of", "csv=p=0"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn exports_video_with_sound() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("skipped: no ffmpeg");
        return;
    };
    let ffprobe = ffprobe(&ffmpeg);
    let dir = std::env::temp_dir().join(format!("polishcast-export-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    make_take(&dir, &ffmpeg);

    let out = dir.join("out.mp4");
    let progress = Progress::default();
    export(&dir, &out, Settings { format: Format::Mp4, long_side: 640, fps: 30 }, &progress, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(progress.fraction(), 1.0);

    if ffprobe.is_file() {
        let video = probe(&ffprobe, &out, "stream=codec_name,width,height,nb_read_frames");
        assert!(video.starts_with("h264,640,400,"), "{video}");
        let frames: u32 = video.lines().next().unwrap().rsplit(',').next().unwrap().parse().unwrap();
        assert!((59..=61).contains(&frames), "{frames} frames");
        assert!(video.contains("aac"), "{video}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cancelling_leaves_no_file() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("polishcast-cancel-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    make_take(&dir, &ffmpeg);
    let out = dir.join("out.mp4");
    let err = export(&dir, &out, Settings::default(), &Progress::default(), &AtomicBool::new(true)).unwrap_err();
    assert!(err.is::<polishcast_export::Cancelled>());
    assert!(!out.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compacting_shrinks_and_keeps_sound_and_length() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let ffprobe = ffprobe(&ffmpeg);
    let dir = std::env::temp_dir().join(format!("polishcast-compact-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A wasteful recording, like ScreenCaptureKit's: a near-still picture at a high bitrate.
    let file = dir.join(take::SCREEN);
    let status = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x400:rate=30:duration=3"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3"])
        .args(["-c:v", "libx264", "-b:v", "20M", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success());
    let before = std::fs::metadata(&file).unwrap().len();

    let after = polishcast_export::compact::compact(&file).unwrap();
    assert!(after < before / 2, "{before} → {after}");
    assert_eq!(std::fs::metadata(&file).unwrap().len(), after);
    if ffprobe.is_file() {
        let streams = probe(&ffprobe, &file, "stream=codec_name");
        assert_eq!(streams.lines().collect::<Vec<_>>(), ["hevc", "aac"]);
        let secs: f64 = probe(&ffprobe, &file, "format=duration").parse().unwrap();
        assert!((secs - 3.0).abs() < 0.1, "{secs}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn blanks_are_cut_from_picture_and_sound() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let ffprobe = ffprobe(&ffmpeg);
    let dir = std::env::temp_dir().join(format!("polishcast-blanks-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 9 s: a tone, 3 s of silence (3–6 s), the tone again.
    let status = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x200:rate=30:duration=9"])
        .args(["-f", "lavfi", "-i", "aevalsrc='if(between(t,3,6),0,0.5*sin(2*PI*440*t))':s=48000:d=9"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(dir.join(take::SCREEN))
        .status()
        .unwrap();
    assert!(status.success());

    let loudness = polishcast_export::sound::loudness(&ffmpeg, &dir.join(take::SCREEN)).unwrap().unwrap();
    assert!((loudness.db.len() as f64 * loudness.window - 9.0).abs() < 0.2, "{} windows", loudness.db.len());
    let params = polishcast_core::BlankParams::default();
    let cuts = polishcast_core::blanks::detect(&loudness, 9.0, &params);
    assert_eq!(cuts.len(), 1, "{cuts:?}");
    // The 3 s silence less the 0.25 s margins (within a window of the AAC edges).
    assert!((cuts[0].start - 3.25).abs() < 0.1 && (cuts[0].end - 5.75).abs() < 0.1, "{cuts:?}");

    let mut project = Project::new(9.0, 320, 200, &EventLog::default());
    project.blanks = Some(params);
    project.cuts = cuts;
    project.save(&dir.join(take::PROJECT)).unwrap();
    let expected = project.timeline().duration();

    let out = dir.join("out.mp4");
    export(
        &dir,
        &out,
        Settings { format: Format::Mp4, long_side: 320, fps: 30 },
        &Progress::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    if ffprobe.is_file() {
        let durations = probe(&ffprobe, &out, "stream=codec_type,duration");
        for line in durations.lines() {
            let secs: f64 = line.rsplit(',').next().unwrap().parse().unwrap();
            assert!((secs - expected).abs() < 0.1, "{line}: expected {expected:.2} s");
        }
        assert_eq!(durations.lines().count(), 2, "{durations}");
    }
    // What's left is the tone, with no pause long enough to cut.
    let after = polishcast_export::sound::loudness(&ffmpeg, &out).unwrap().unwrap();
    assert!(polishcast_core::blanks::detect(&after, expected, &params).is_empty(), "a silence is left");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_faster_speed_shortens_picture_and_sound() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let ffprobe = ffprobe(&ffmpeg);
    let dir = std::env::temp_dir().join(format!("polishcast-speed-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 6 s of a 440 Hz tone, with 3 s of silence (3–6 s) to cut as well.
    let status = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x200:rate=30:duration=9"])
        .args(["-f", "lavfi", "-i", "aevalsrc='if(between(t,3,6),0,0.5*sin(2*PI*440*t))':s=48000:d=9"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(dir.join(take::SCREEN))
        .status()
        .unwrap();
    assert!(status.success());

    for (speed, cut) in [(2.0, false), (1.5, true), (3.0, false)] {
        let mut project = Project::new(9.0, 320, 200, &EventLog::default());
        project.speed = speed;
        if cut {
            project.blanks = Some(Default::default());
            project.cuts = vec![polishcast_core::Cut { start: 3.25, end: 5.75, keep: false }];
        }
        project.save(&dir.join(take::PROJECT)).unwrap();
        let expected = project.video_duration();

        let out = dir.join(format!("out-{speed}.mp4"));
        let progress = Progress::default();
        export(
            &dir,
            &out,
            Settings { format: Format::Mp4, long_side: 320, fps: 30 },
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(progress.fraction(), 1.0, "{speed}×");
        if ffprobe.is_file() {
            let durations = probe(&ffprobe, &out, "stream=codec_type,duration");
            assert_eq!(durations.lines().count(), 2, "{durations}");
            for line in durations.lines() {
                let secs: f64 = line.rsplit(',').next().unwrap().parse().unwrap();
                assert!((secs - expected).abs() < 0.15, "{speed}× {line}: expected {expected:.2} s");
            }
            let frames: f64 = probe(&ffprobe, &out, "stream=nb_read_frames").lines().next().unwrap().parse().unwrap();
            assert!((frames - expected * 30.0).abs() <= 2.0, "{speed}×: {frames} frames for {expected:.2} s");
        }
        // The sound is still there from the start (atempo keeps its pitch; this doesn't measure it).
        let after = polishcast_export::sound::loudness(&ffmpeg, &out).unwrap().unwrap();
        let loud = after.db.iter().take((1.0 / speed / after.window) as usize).all(|d| *d > -20.0);
        assert!(loud, "{speed}×: the tone's start is missing");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exports_a_looping_gif() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let ffprobe = ffprobe(&ffmpeg);
    let dir = std::env::temp_dir().join(format!("polishcast-gif-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    make_take(&dir, &ffmpeg);
    let out = dir.join("out.gif");
    let settings = Settings { format: Format::Gif, long_side: 320, fps: 10 };
    export(&dir, &out, settings, &Progress::default(), &AtomicBool::new(false)).unwrap();
    if ffprobe.is_file() {
        let video = probe(&ffprobe, &out, "stream=codec_name,width,height,nb_read_frames");
        assert!(video.starts_with("gif,320,200,"), "{video}");
        let frames: u32 = video.rsplit(',').next().unwrap().parse().unwrap();
        assert!((19..=21).contains(&frames), "{frames} frames");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_picture_background_and_a_removed_clip_export() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let ffprobe = ffprobe(&ffmpeg);
    let dir = std::env::temp_dir().join(format!("polishcast-picture-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    make_take(&dir, &ffmpeg);
    let status = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=orange:size=64x48", "-frames:v", "1"])
        .arg(dir.join("background.png"))
        .status()
        .unwrap();
    assert!(status.success());
    let mut p = Project::load(&dir.join(take::PROJECT)).unwrap();
    p.style.background = polishcast_core::project::Background::Image("background.png".into());
    p.splits = vec![1.0];
    p.removed = vec![(1.0, 2.0)];
    p.save(&dir.join(take::PROJECT)).unwrap();

    let out = dir.join("out.mp4");
    let settings = Settings { format: Format::Mp4, long_side: 640, fps: 30 };
    export(&dir, &out, settings, &Progress::default(), &AtomicBool::new(false)).unwrap();
    if ffprobe.is_file() {
        let duration: f64 = probe(&ffprobe, &out, "format=duration").parse().unwrap();
        assert!((duration - 1.0).abs() < 0.1, "the second half is left out: {duration} s");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// RMS level in dB of `file`'s sound around `freq` Hz, between `start` and `end` seconds.
fn band_db(ffmpeg: &Path, file: &Path, freq: u32, (start, end): (f64, f64)) -> f64 {
    let filter = format!("atrim={start}:{end},bandpass=f={freq}:width_type=q:w=20,astats=measure_perchannel=none");
    let out = Command::new(ffmpeg)
        .args(["-nostdin", "-i"])
        .arg(file)
        .args(["-af", &filter, "-f", "null", "-"])
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stderr);
    let line = log.lines().find(|l| l.contains("RMS level dB")).unwrap_or_else(|| panic!("{log}"));
    line.rsplit(' ').next().unwrap().parse().unwrap_or(-200.0)
}

#[test]
fn music_plays_at_its_moments_under_the_voice() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("polishcast-music-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    make_take(&dir, &ffmpeg);
    let status = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=1000:duration=10"])
        .arg(dir.join("music-1.wav"))
        .status()
        .unwrap();
    assert!(status.success());
    // From 0.5 s to 1.5 s of the recording, at twice the speed: 0.25 s to 0.75 s of the video,
    // where the music plays at its own speed and pitch.
    let path = dir.join(take::PROJECT);
    let mut project = Project::load(&path).unwrap();
    project.speed = 2.0;
    project.music = vec![polishcast_core::music::Music {
        file: "music-1.wav".into(),
        name: "Tone".into(),
        start: 0.5,
        end: 1.5,
        from: 0.0,
        volume: 0.5,
        fade: 0.0,
    }];
    project.save(&path).unwrap();

    let out = dir.join("out.mp4");
    export(
        &dir,
        &out,
        Settings { format: Format::Mp4, long_side: 320, fps: 30 },
        &Progress::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let music = |span| band_db(&ffmpeg, &out, 1000, span);
    assert!(music((0.35, 0.65)) > -30.0, "plays: {}", music((0.35, 0.65)));
    assert!(music((0.0, 0.15)) < -50.0, "not before: {}", music((0.0, 0.15)));
    assert!(music((0.85, 1.0)) < -50.0, "nor after: {}", music((0.85, 1.0)));
    assert!(band_db(&ffmpeg, &out, 440, (0.0, 1.0)) > -40.0, "the voice is still there");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn music_is_lowered_while_the_voice_speaks() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("polishcast-duck-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A voice (440 Hz) for the first second of three, then silence; music (1 kHz) throughout.
    let status = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x200:rate=30:duration=3"])
        .args(["-f", "lavfi", "-i", "aevalsrc='if(lt(t,1),0.5*sin(2*PI*440*t),0)':s=48000:d=3"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(dir.join(take::SCREEN))
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=1000:duration=10"])
        .arg(dir.join("music-1.wav"))
        .status()
        .unwrap();
    assert!(status.success());
    let mut project = Project::new(3.0, 320, 200, &EventLog::default());
    project.music = vec![polishcast_core::music::Music {
        file: "music-1.wav".into(),
        name: "Tone".into(),
        start: 0.0,
        end: 3.0,
        from: 0.0,
        volume: 1.0,
        fade: 0.0,
    }];
    project.duck = Some(0.25);
    project.save(&dir.join(take::PROJECT)).unwrap();

    let out = dir.join("out.mp4");
    export(
        &dir,
        &out,
        Settings { format: Format::Mp4, long_side: 320, fps: 30 },
        &Progress::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let (under, alone) = (band_db(&ffmpeg, &out, 1000, (0.2, 0.8)), band_db(&ffmpeg, &out, 1000, (2.0, 2.8)));
    // A quarter of the volume is 12 dB down.
    assert!((alone - under - 12.0).abs() < 2.0, "under the voice {under:.1} dB, alone {alone:.1} dB");
    std::fs::remove_dir_all(&dir).unwrap();
}
