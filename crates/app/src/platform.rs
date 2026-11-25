//! What differs between macOS, Windows and Linux in the app itself.

use std::path::Path;

/// Starts and stops a recording from any app.
#[cfg(target_os = "macos")]
pub const SHORTCUT: &str = "cmd+shift+2";
#[cfg(not(target_os = "macos"))]
pub const SHORTCUT: &str = "ctrl+shift+2";

#[cfg(target_os = "macos")]
pub const SHORTCUT_LABEL: &str = "⌘⇧2";
#[cfg(not(target_os = "macos"))]
pub const SHORTCUT_LABEL: &str = "Ctrl+Shift+2";

/// The command key, as written in button tips ("⌘Z", "Ctrl+Z").
#[cfg(target_os = "macos")]
pub const MOD: &str = "⌘";
#[cfg(not(target_os = "macos"))]
pub const MOD: &str = "Ctrl+";

#[cfg(target_os = "macos")]
pub const SHOW_IN_FOLDER: &str = "Show in Finder";
#[cfg(windows)]
pub const SHOW_IN_FOLDER: &str = "Show in Explorer";
#[cfg(not(any(target_os = "macos", windows)))]
pub const SHOW_IN_FOLDER: &str = "Show in folder";

/// The file manager, as named in short link labels.
#[cfg(target_os = "macos")]
pub const FILE_MANAGER: &str = "Finder";
#[cfg(windows)]
pub const FILE_MANAGER: &str = "Explorer";
#[cfg(not(any(target_os = "macos", windows)))]
pub const FILE_MANAGER: &str = "Folder";

/// Opens the file manager with `path` selected (Linux: its folder).
pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer parses its own command line: the path must be quoted inside the switch.
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}

/// Opens `path` in the app the system uses for its kind of file (a video plays in the default
/// player).
pub fn open(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `start` takes its first quoted argument as the window's title, hence the empty one.
        let _ = std::process::Command::new("cmd").raw_arg(format!("/C start \"\" \"{}\"", path.display())).spawn();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// Leaves the app's window out of screen recordings. On macOS the capture filter already
/// excludes the app; on Windows each window has to opt out (Windows 10 2004 or later).
pub fn exclude_from_capture(window: &dioxus::desktop::DesktopContext) {
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowExtWindows;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};
        let hwnd = HWND(window.window.hwnd() as *mut std::ffi::c_void);
        // SAFETY: the window handle is valid for as long as the window, which outlives this call.
        let _ = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) };
    }
    #[cfg(not(windows))]
    let _ = window;
}
