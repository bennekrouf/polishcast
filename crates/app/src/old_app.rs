//! The app was called Small Video until 0.1.8. On macOS the update installs PolishCast.app next
//! to Small Video.app instead of replacing it, and the old app, still in the Dock or Spotlight,
//! keeps offering an update the user has already installed. PolishCast offers to remove it.

use std::path::{Path, PathBuf};

/// Small Video.app in Applications, unless it's what's running.
pub fn find() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let home = dirs::home_dir().map(|h| h.join("Applications"));
    let found = [Some(PathBuf::from("/Applications")), home]
        .into_iter()
        .flatten()
        .map(|dir| dir.join("Small Video.app"))
        .find(|app| is_small_video(app))?;
    let running_it = std::env::current_exe().is_ok_and(|exe| exe.starts_with(&found));
    (!running_it).then_some(found)
}

/// Whether `app` is the old build: its executable kept the project's first name.
fn is_small_video(app: &Path) -> bool {
    app.join("Contents/MacOS/small-video").is_file()
}

/// Quits the old app if it's running (it holds the same shortcut and menu bar spot), then
/// moves it to the Trash.
pub fn remove(app: &Path) -> Result<(), String> {
    quit(app);
    crate::takes::move_to_trash(app).map_err(|e| format!("{e}"))
}

#[cfg(target_os = "macos")]
fn quit(app: &Path) {
    let exe = app.join("Contents/MacOS/small-video");
    let Ok(out) = std::process::Command::new("ps").args(["-axo", "pid=,comm="]).output() else {
        return;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((pid, comm)) = line.trim_start().split_once(' ') else { continue };
        if Path::new(comm.trim()) == exe {
            if let Ok(pid) = pid.parse::<libc::pid_t>() {
                // SAFETY: plain signal to a process found by its executable's path.
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn quit(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_old_build_counts() {
        let base = std::env::temp_dir().join(format!("polishcast-old-app-{}", std::process::id()));
        let app = base.join("Small Video.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        assert!(!is_small_video(&app));
        std::fs::write(app.join("Contents/MacOS/small-video"), b"").unwrap();
        assert!(is_small_video(&app));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
