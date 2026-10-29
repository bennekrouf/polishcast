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
        moved_into_place(&videos.join("Small Video"), videos.join("PolishCast"))
    })
    .clone()
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
            dir,
        })
        .collect();
    entries.sort_by(|a, b| b.name.cmp(&a.name));
    entries
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
