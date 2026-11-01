//! PolishCast's takes in the media library every Mayorana app shares: each take with where it
//! stands (recording, polishing, ready once exported) and its files, so PublishKit and TrackCut
//! show it too. All of it read from the takes' folders, so it is right however a take changed.

use crate::takes;
use desktop_kit::library::{self as shared, Item, Kind, Role, Status};
use polishcast_core::take;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn id_of(dir: &Path) -> String {
    format!("polishcast:{}", dir.display())
}

/// The take in `dir` as the library shows it, from what is in its folder and beside it.
pub fn item_of(dir: &Path) -> Option<Item> {
    if !dir.is_dir() {
        return None;
    }
    let title = dir.file_name()?.to_string_lossy().into_owned();
    let exports = crate::editor::exports_of(dir);
    let project = dir.join(take::PROJECT);
    let screen = dir.join(take::SCREEN);
    let (status, dated) = if !project.is_file() {
        (Status::Recording, screen.clone())
    } else if exports.is_empty() {
        (Status::Polishing, project)
    } else {
        let newest = exports.iter().max_by_key(|e| modified(e)).cloned().unwrap_or_default();
        (Status::Ready, newest)
    };
    let mut item = Item::new(id_of(dir), "polishcast", Kind::Video, title, status);
    if screen.is_file() {
        item = item.with_file(screen, Role::Source);
    }
    for export in &exports {
        let role = if export.extension().is_some_and(|e| e == "gif") { Role::Clip } else { Role::Video };
        item = item.with_file(export, role);
        for captions in polishcast_export::caption_files(export).into_iter().filter(|c| c.is_file()) {
            item = item.with_file(captions, Role::Captions);
        }
    }
    if let Some(at) = modified(&dated) {
        item.date = shared::day_of(at);
    }
    item.open = dir.to_string_lossy().into_owned();
    Some(item)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Writes the take in `dir` to the library as it is now; takes it out when the folder is gone.
pub fn sync_take(dir: &Path) {
    match item_of(dir) {
        Some(item) => {
            let _ = shared::upsert(item);
        }
        None => {
            let _ = shared::remove(&id_of(dir));
        }
    }
}

/// A recording starting in `dir`: it shows as recording until it stops.
pub fn recording(dir: &Path) {
    let title = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut item = Item::new(id_of(dir), "polishcast", Kind::Video, title, Status::Recording);
    item.open = dir.to_string_lossy().into_owned();
    let _ = shared::upsert(item);
}

/// Every take in the library, and none that is gone. A folder being recorded into, with no
/// recording in it yet, is left as it is.
pub fn sync_all() {
    let mut keep = HashSet::new();
    let dirs = std::fs::read_dir(takes::library())
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect::<Vec<PathBuf>>())
        .unwrap_or_default();
    for dir in dirs {
        let id = id_of(&dir);
        if dir.join(take::SCREEN).is_file() {
            sync_take(&dir);
            keep.insert(id);
        } else if shared::get(&id).is_some() {
            keep.insert(id);
        }
    }
    let _ = shared::retain("polishcast", "polishcast:", &keep);
}

/// Changes whenever a take or an export is added, removed or changes beside the others.
pub fn stamp() -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut stamp: Vec<(PathBuf, Option<SystemTime>)> = std::fs::read_dir(takes::library())
        .map(|r| r.filter_map(|e| e.ok()).map(|e| (e.path(), e.metadata().and_then(|m| m.modified()).ok())).collect())
        .unwrap_or_default();
    stamp.sort();
    stamp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_take_is_polishing_until_it_is_exported() {
        let root = std::env::temp_dir().join(format!("polishcast-library-{}", std::process::id()));
        let dir = root.join("Demo");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(take::SCREEN), b"movie").unwrap();
        assert_eq!(item_of(&dir).unwrap().status, Status::Recording);

        std::fs::write(dir.join(take::PROJECT), b"{}").unwrap();
        assert_eq!(item_of(&dir).unwrap().status, Status::Polishing);

        std::fs::write(root.join("Demo 1080p.mp4"), b"video").unwrap();
        std::fs::write(root.join("Demo 1080p.srt"), b"1").unwrap();
        let item = item_of(&dir).unwrap();
        assert_eq!(item.status, Status::Ready);
        let roles: Vec<Role> = item.files.iter().map(|f| f.role).collect();
        assert_eq!(roles, [Role::Source, Role::Video, Role::Captions]);
        assert_eq!(item.title, "Demo");
        let _ = std::fs::remove_dir_all(&root);
        assert!(item_of(&dir).is_none());
    }
}
