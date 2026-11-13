//! Takes on disk: one folder per recording under ~/Movies/PolishCast.

use polishcast_capture::Take;
use polishcast_core::{clips, take, EventLog, Project};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `POLISHCAST_LIBRARY` overrides it (for development and tests).
pub fn library() -> PathBuf {
    if let Some(dir) = std::env::var_os("POLISHCAST_LIBRARY") {
        return dir.into();
    }
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let videos = dirs::video_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Movies"));
        recased(&videos, "Polishcast", "PolishCast");
        let dir = moved_into_place(&videos.join("Small Video"), videos.join("PolishCast"));
        gather_exports(&dir);
        dir
    })
    .clone()
}

/// Moves the videos exported beside the take folders in `library` into the folder of the take
/// each is named after, with their captions files. Exports were written beside the folders until
/// they went in them, which left the library's top level a mix of folders and videos. A video
/// named after no take, or one that can't be moved, stays where it is.
fn gather_exports(library: &Path) {
    let Ok(read) = std::fs::read_dir(library) else { return };
    let (dirs, files): (Vec<PathBuf>, Vec<PathBuf>) =
        read.filter_map(|e| e.ok().map(|e| e.path())).partition(|p| p.is_dir());
    let takes: Vec<(PathBuf, String)> = dirs
        .into_iter()
        .filter(|d| d.join(take::SCREEN).is_file())
        .filter_map(|d| {
            let name = d.file_name()?.to_string_lossy().into_owned();
            Some((d, name))
        })
        .collect();
    for video in files.iter().filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("mp4" | "gif"))) {
        // The longest name wins: "Demo 2 1080p.mp4" is Demo 2's, not Demo's.
        let Some((dir, _)) =
            takes.iter().filter(|(_, name)| crate::editor::named_after(video, name)).max_by_key(|(_, name)| name.len())
        else {
            continue;
        };
        let mut moving = vec![video.clone()];
        moving.extend(polishcast_export::caption_files(video).into_iter().filter(|c| c.is_file()));
        for file in moving {
            let Some(name) = file.file_name() else { continue };
            let to = dir.join(name);
            if !to.exists() {
                let _ = std::fs::rename(&file, to);
            }
        }
    }
}

/// `dir`, after moving `legacy` there if only `legacy` exists. The app was called Small Video, and
/// its takes were kept under that name. If the move fails the old folder stays in use: takes the
/// app cannot find would look deleted.
fn moved_into_place(legacy: &Path, dir: PathBuf) -> PathBuf {
    if dir.exists() || !legacy.exists() {
        return dir;
    }
    match std::fs::rename(legacy, &dir) {
        Ok(()) => dir,
        Err(_) => legacy.to_path_buf(),
    }
}

/// Renames `parent/old` to `parent/new` when they differ only in case: the app wrote `Polishcast`
/// for a few releases before the name settled on PolishCast. macOS and Windows match folder names
/// regardless of case, so the folder is found either way; this only fixes how it reads in Finder.
/// The names are compared as listed, since `exists()` cannot tell the two apart.
fn recased(parent: &Path, old: &str, new: &str) {
    let Ok(read) = std::fs::read_dir(parent) else { return };
    let names: Vec<_> = read.filter_map(|e| e.ok().map(|e| e.file_name())).collect();
    if names.iter().any(|n| n == old) && !names.iter().any(|n| n == new) {
        let _ = std::fs::rename(parent.join(old), parent.join(new));
    }
}

/// A new, empty folder for the next take, named after the time it starts.
pub fn new_dir() -> std::io::Result<PathBuf> {
    let dir = library().join(chrono::Local::now().format("%Y-%m-%d %H.%M.%S").to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A new, empty folder for the next take, named `name` (checked with [`check_name`]).
pub fn new_named_dir(name: &str) -> std::io::Result<PathBuf> {
    let dir = library().join(name);
    std::fs::create_dir_all(library())?;
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

/// `name`, trimmed, if it can be a take's folder name: not empty or hidden, no path separators
/// or characters Windows refuses, and no take of that name yet.
pub fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') || name.chars().count() > 100 {
        return Err("the name is 1 to 100 characters and doesn't start with a dot".into());
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) {
        return Err("the name can't have / \\ : * ? \" < > | in it".into());
    }
    if library().join(name).exists() {
        return Err(format!("there is already a take named {name}"));
    }
    Ok(name.to_string())
}

/// Makes a take of a video made elsewhere (an MP4 or MOV): copies it into a new folder named
/// after the file, and writes a project for it. With no pointer recorded there is no cursor to
/// redraw and no zoom to suggest, but the background, frame, cuts, speed, music and export all
/// work as for a recording. Returns the new take's folder.
pub fn import(src: &Path) -> Result<PathBuf, String> {
    let ffmpeg = polishcast_export::ensure_ffmpeg(&mut |_| {}, &|| false).map_err(|e| e.to_string())?;
    let (duration, width, height) = probe(&ffmpeg, src)?;
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
    let stem: String = stem
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .collect();
    let stem = stem.trim().trim_start_matches('.').chars().take(90).collect::<String>();
    let stem = if stem.is_empty() { "Imported".to_string() } else { stem };
    let name = (1..)
        .map(|n| if n == 1 { stem.clone() } else { format!("{stem} {n}") })
        .find(|n| check_name(n).is_ok())
        .ok_or_else(|| format!("No free name for a take called \u{201c}{stem}\u{201d}"))?;
    let dir = new_named_dir(&name).map_err(|e| format!("Making the take's folder: {e}"))?;
    let written =
        std::fs::copy(src, dir.join(take::SCREEN)).map_err(|e| format!("Copying the video: {e}")).and_then(|_| {
            let mut project = Project::new(duration, width, height, &EventLog::default());
            project.style.click_effect = false;
            project.save(&dir.join(take::PROJECT)).map_err(|e| format!("Writing the project: {e}"))
        });
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
}

/// A video's length in seconds and its picture's size, from what ffmpeg says of it.
fn probe(ffmpeg: &Path, file: &Path) -> Result<(f64, u32, u32), String> {
    let out = polishcast_export::tools::command(ffmpeg)
        .arg("-hide_banner")
        .arg("-i")
        .arg(file)
        .output()
        .map_err(|e| format!("Running ffmpeg: {e}"))?;
    parse_probe(&String::from_utf8_lossy(&out.stderr)).ok_or_else(|| "That file isn't a video ffmpeg can read.".into())
}

fn parse_probe(text: &str) -> Option<(f64, u32, u32)> {
    let d = text.split("Duration: ").nth(1)?.split(',').next()?.trim();
    let mut hms = d.split(':').map(|p| p.parse::<f64>().ok());
    let duration = hms.next()?? * 3600.0 + hms.next()?? * 60.0 + hms.next()??;
    let video = text.lines().find(|l| l.contains("Stream #") && l.contains("Video:"))?;
    let (width, height) = video.split([',', ' ']).find_map(|w| {
        let (a, b) = w.split_once('x')?;
        Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
    })?;
    // A phone video recorded upright is stored sideways, with a rotation to apply.
    let turned = text.contains("rotation of -90")
        || text.contains("rotation of 90")
        || text.contains("rotate          : 90")
        || text.contains("rotate          : 270");
    let (width, height) = if turned { (height, width) } else { (width, height) };
    (duration > 0.0 && width > 0 && height > 0).then_some((duration, width, height))
}

/// Writes the take's project, with zooms suggested from its clicks.
pub fn create_project(t: &Take) -> std::io::Result<Project> {
    let events: EventLog =
        std::fs::read(t.dir.join(take::EVENTS)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let project = new_project(t, events);
    project.save(&t.dir.join(take::PROJECT))?;
    Ok(project)
}

/// Shorter than this, a take keeps everything: what's left would be too little to work with.
const LEAST_KEPT: f64 = 1.0;

/// The project for a take: the pointer's way off Record and to Stop are removed clips (they
/// can be put back), and their clicks suggest no zoom.
fn new_project(t: &Take, mut events: EventLog) -> Project {
    let (from, to) = t.kept;
    let trims = to - from >= LEAST_KEPT;
    if trims {
        events.clicks.retain(|c| c.t >= from && c.t < to);
    }
    let mut project = Project::new(t.duration, t.width, t.height, &events);
    if trims {
        if clips::split(&mut project.splits, t.duration, from) {
            clips::remove(&mut project.removed, (0.0, from));
        }
        if clips::split(&mut project.splits, t.duration, to) {
            clips::remove(&mut project.removed, (to, t.duration));
        }
    }
    project
}

#[derive(Clone, PartialEq)]
pub struct Entry {
    pub dir: PathBuf,
    pub name: String,
    pub project: Option<Project>,
    /// Size of the recording in bytes.
    pub size: u64,
    /// When the take was made: its folder's creation, else when its recording was last written.
    pub made: Option<std::time::SystemTime>,
}

/// When the take in `dir` was made. Named takes can't be ordered by name: a take called
/// "snwatch-demo" would sort above one called "2026-11-12 20.03.39" recorded after it.
fn made(dir: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(dir)
        .and_then(|m| m.created())
        .or_else(|_| std::fs::metadata(dir.join(take::SCREEN)).and_then(|m| m.modified()))
        .ok()
}

/// Every take in the library, newest first.
pub fn list() -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(library()) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join(take::SCREEN).is_file())
        .map(|dir| Entry {
            name: dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            project: Project::load(&dir.join(take::PROJECT)).ok(),
            size: std::fs::metadata(dir.join(take::SCREEN)).map_or(0, |m| m.len()),
            made: made(&dir),
            dir,
        })
        .collect();
    newest_first(&mut entries);
    entries
}

/// Newest first by when each take was made, then by name for takes made at the same time.
fn newest_first(entries: &mut [Entry]) {
    entries.sort_by(|a, b| b.made.cmp(&a.made).then_with(|| b.name.cmp(&a.name)));
}

/// Moves a take's folder to the system Trash. On macOS through the file manager API rather
/// than by scripting Finder, which would ask for permission to control Finder (the folder can
/// still be dragged back out of the Trash, just not "Put Back").
pub fn move_to_trash(dir: &std::path::Path) -> Result<(), trash::Error> {
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx.delete(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moves a real (empty-ish) folder to the system Trash.
    #[test]
    fn a_take_folder_goes_to_the_trash() {
        let dir = std::env::temp_dir().join(format!("polishcast-trash-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(take::SCREEN), b"not really a movie").unwrap();
        move_to_trash(&dir).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn exports_beside_the_takes_move_into_their_folders() {
        let base = std::env::temp_dir().join(format!("polishcast-gather-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for take in ["Demo", "Demo 2"] {
            std::fs::create_dir_all(base.join(take)).unwrap();
            std::fs::write(base.join(take).join(take::SCREEN), b"").unwrap();
        }
        for f in ["Demo.mp4", "Demo 1080p.mp4", "Demo 1080p.srt", "Demo 2 1080p.mp4", "Other.mp4", "Demo.jpg"] {
            std::fs::write(base.join(f), b"").unwrap();
        }
        gather_exports(&base);
        let names = |d: &Path| {
            let mut n: Vec<String> =
                std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            n.sort();
            n
        };
        assert_eq!(names(&base), ["Demo", "Demo 2", "Demo.jpg", "Other.mp4"]);
        assert_eq!(names(&base.join("Demo")), ["Demo 1080p.mp4", "Demo 1080p.srt", "Demo.mp4", "screen.mov"]);
        assert_eq!(names(&base.join("Demo 2")), ["Demo 2 1080p.mp4", "screen.mov"]);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn the_newest_take_comes_first_whatever_its_name() {
        use std::time::{Duration, UNIX_EPOCH};
        let entry = |name: &str, secs| Entry {
            dir: PathBuf::from(name),
            name: name.into(),
            project: None,
            size: 0,
            made: Some(UNIX_EPOCH + Duration::from_secs(secs)),
        };
        let mut entries = vec![entry("snwatch-demo", 100), entry("2026-11-12 20.03.39", 300), entry("api0-demo", 200)];
        newest_first(&mut entries);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["2026-11-12 20.03.39", "api0-demo", "snwatch-demo"]);
    }

    #[test]
    fn a_take_name_is_a_plain_folder_name() {
        assert_eq!(check_name("  Demo of the export  ").unwrap(), "Demo of the export");
        for bad in ["", "  ", ".hidden", "a/b", "a\\b", "a:b", "x".repeat(101).as_str()] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_start_and_stop_gestures_are_removed_clips() {
        let take = |kept| Take {
            dir: PathBuf::new(),
            duration: 10.0,
            width: 100,
            height: 100,
            compact: false,
            sound: None,
            kept,
        };
        let click = |t| polishcast_core::Click { t, x: 0.5, y: 0.5, button: polishcast_core::Button::Left };
        let events = EventLog { cursor: Vec::new(), clicks: vec![click(5.0), click(9.5)] };

        let p = new_project(&take((0.6, 9.0)), events.clone());
        assert_eq!(p.splits, [0.6, 9.0]);
        assert_eq!(p.removed, [(0.0, 0.6), (9.0, 10.0)]);
        assert!(p.zooms.iter().all(|z| z.start < 9.0), "the Stop click suggests no zoom");

        // Nothing to leave out, or almost nothing left: the take stays whole.
        for kept in [(0.0, 10.0), (4.0, 4.5)] {
            let p = new_project(&take(kept), events.clone());
            assert!(p.splits.is_empty() && p.removed.is_empty());
        }
    }

    #[test]
    fn takes_saved_under_the_old_name_move_to_the_new_one() {
        let base = std::env::temp_dir().join(format!("polishcast-library-test-{}", std::process::id()));
        let (legacy, dir) = (base.join("Small Video"), base.join("PolishCast"));
        std::fs::create_dir_all(legacy.join("take")).unwrap();

        assert_eq!(moved_into_place(&legacy, dir.clone()), dir);
        assert!(dir.join("take").is_dir());
        assert!(!legacy.exists());

        // Once moved, a folder left with the old name is not touched again.
        std::fs::create_dir_all(&legacy).unwrap();
        assert_eq!(moved_into_place(&legacy, dir.clone()), dir);
        assert!(legacy.exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_library_written_polishcast_becomes_polishcast_with_a_capital_c() {
        let base = std::env::temp_dir().join(format!("polishcast-recase-test-{}", std::process::id()));
        std::fs::create_dir_all(base.join("Polishcast").join("take")).unwrap();

        recased(&base, "Polishcast", "PolishCast");
        let names: Vec<_> = std::fs::read_dir(&base).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["PolishCast"]);
        assert!(base.join("PolishCast").join("take").is_dir());

        // Already right: nothing to do.
        recased(&base, "Polishcast", "PolishCast");
        assert!(base.join("PolishCast").join("take").is_dir());
        std::fs::remove_dir_all(base).unwrap();
    }
}

#[cfg(test)]
mod import_tests {
    #[test]
    fn ffmpeg_says_how_long_and_how_big() {
        let text = "  Duration: 00:01:02.50, start: 0.000000, bitrate: 900 kb/s\n  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 30 fps\n";
        assert_eq!(super::parse_probe(text), Some((62.5, 1920, 1080)));
        let upright = format!("{text}      displaymatrix: rotation of -90.00 degrees\n");
        assert_eq!(super::parse_probe(&upright), Some((62.5, 1080, 1920)));
        assert_eq!(super::parse_probe("not a video"), None);
    }
}
