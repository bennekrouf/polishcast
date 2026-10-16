//! Takes on disk: one folder per recording under ~/Movies/PolishCast.

use polishcast_capture::Take;
use polishcast_core::{take, EventLog, Project};
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

/// Writes the take's project, with zooms suggested from its clicks.
pub fn create_project(t: &Take) -> std::io::Result<Project> {
    let events: EventLog =
        std::fs::read(t.dir.join(take::EVENTS)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let project = Project::new(t.duration, t.width, t.height, &events);
    project.save(&t.dir.join(take::PROJECT))?;
    Ok(project)
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
