//! End to end: a generated take through ffmpeg, the compositor and back. Skipped (passes) when
//! ffmpeg isn't installed.

use small_video_core::{take, Button, Click, CursorSample, EventLog, Project};
use small_video_export::{export, ffmpeg, Progress, Settings};
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
    let ffprobe = ffmpeg.with_file_name("ffprobe");
    let dir = std::env::temp_dir().join(format!("small-video-export-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    make_take(&dir, &ffmpeg);

    let out = dir.join("out.mp4");
    let progress = Progress::default();
    export(&dir, &out, Settings { long_side: 640, fps: 30 }, &progress, &AtomicBool::new(false)).unwrap();
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
    let dir = std::env::temp_dir().join(format!("small-video-cancel-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    make_take(&dir, &ffmpeg);
    let out = dir.join("out.mp4");
    let err = export(&dir, &out, Settings::default(), &Progress::default(), &AtomicBool::new(true)).unwrap_err();
    assert!(err.is::<small_video_export::Cancelled>());
    assert!(!out.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compacting_shrinks_and_keeps_sound_and_length() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let ffprobe = ffmpeg.with_file_name("ffprobe");
    let dir = std::env::temp_dir().join(format!("small-video-compact-test-{}", std::process::id()));
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

    let after = small_video_export::compact::compact(&file).unwrap();
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
    let ffprobe = ffmpeg.with_file_name("ffprobe");
    let dir = std::env::temp_dir().join(format!("small-video-blanks-test-{}", std::process::id()));
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

    let loudness = small_video_export::sound::loudness(&ffmpeg, &dir.join(take::SCREEN)).unwrap().unwrap();
    assert!((loudness.db.len() as f64 * loudness.window - 9.0).abs() < 0.2, "{} windows", loudness.db.len());
    let params = small_video_core::BlankParams::default();
    let cuts = small_video_core::blanks::detect(&loudness, 9.0, &params);
    assert_eq!(cuts.len(), 1, "{cuts:?}");
    // The 3 s silence less the 0.25 s margins (within a window of the AAC edges).
    assert!((cuts[0].start - 3.25).abs() < 0.1 && (cuts[0].end - 5.75).abs() < 0.1, "{cuts:?}");

    let mut project = Project::new(9.0, 320, 200, &EventLog::default());
    project.blanks = Some(params);
    project.cuts = cuts;
    project.save(&dir.join(take::PROJECT)).unwrap();
    let expected = project.timeline().duration();

    let out = dir.join("out.mp4");
    export(&dir, &out, Settings { long_side: 320, fps: 30 }, &Progress::default(), &AtomicBool::new(false)).unwrap();
    if ffprobe.is_file() {
        let durations = probe(&ffprobe, &out, "stream=codec_type,duration");
        for line in durations.lines() {
            let secs: f64 = line.rsplit(',').next().unwrap().parse().unwrap();
            assert!((secs - expected).abs() < 0.1, "{line}: expected {expected:.2} s");
        }
        assert_eq!(durations.lines().count(), 2, "{durations}");
    }
    // What's left is the tone, with no pause long enough to cut.
    let after = small_video_export::sound::loudness(&ffmpeg, &out).unwrap().unwrap();
    assert!(small_video_core::blanks::detect(&after, expected, &params).is_empty(), "a silence is left");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_faster_speed_shortens_picture_and_sound() {
    let Some(ffmpeg) = ffmpeg() else {
        return;
    };
    let ffprobe = ffmpeg.with_file_name("ffprobe");
    let dir = std::env::temp_dir().join(format!("small-video-speed-test-{}", std::process::id()));
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
            project.cuts = vec![small_video_core::Cut { start: 3.25, end: 5.75, keep: false }];
        }
        project.save(&dir.join(take::PROJECT)).unwrap();
        let expected = project.video_duration();

        let out = dir.join(format!("out-{speed}.mp4"));
        let progress = Progress::default();
        export(&dir, &out, Settings { long_side: 320, fps: 30 }, &progress, &AtomicBool::new(false)).unwrap();
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
        let after = small_video_export::sound::loudness(&ffmpeg, &out).unwrap().unwrap();
        let loud = after.db.iter().take((1.0 / speed / after.window) as usize).all(|d| *d > -20.0);
        assert!(loud, "{speed}×: the tone's start is missing");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
