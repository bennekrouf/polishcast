//! What to do when ffmpeg is missing and couldn't be downloaded (offline, or a company network
//! that blocks the download): the one command that installs it on this system, to copy, and the
//! page to get it by hand. Export and the sound measurements find it there on the next try.

use dioxus::prelude::*;

/// Whether `e` is ffmpeg missing, rather than an export that failed for another reason.
pub fn is_missing(e: &anyhow::Error) -> bool {
    e.is::<polishcast_export::FfmpegMissing>()
}

/// The command that installs ffmpeg here, and what it needs.
fn install_command() -> (&'static str, &'static str) {
    if cfg!(windows) {
        (
            "winget install --id Gyan.FFmpeg -e",
            "Paste it in PowerShell or Git Bash. If winget is blocked on this computer, \
             ask IT to install ffmpeg, or download it below and unzip it.",
        )
    } else if cfg!(target_os = "macos") {
        ("brew install ffmpeg", "Paste it in Terminal. It needs Homebrew (brew.sh).")
    } else {
        ("sudo apt install ffmpeg", "Paste it in a terminal (dnf or pacman on other distributions).")
    }
}

const DOWNLOAD_PAGE: &str =
    if cfg!(windows) { "https://www.gyan.dev/ffmpeg/builds/" } else { "https://ffmpeg.org/download.html" };

/// Shown under the error, above the caller's Try again.
#[component]
pub fn FfmpegHelp() -> Element {
    let (command, note) = install_command();
    rsx! {
        div { class: "ffmpeg-help",
            p {
                "PolishCast needs ffmpeg to read and export videos, and couldn't download it. \
                 Install it yourself, then try again:"
            }
            crate::mcp::Snippet { text: command.to_string() }
            p { class: "hint", "{note}" }
            p { class: "hint",
                "Or "
                a {
                    href: "#",
                    onclick: move |e| {
                        e.prevent_default();
                        crate::licence::open_url(DOWNLOAD_PAGE);
                    },
                    "download ffmpeg"
                }
                if cfg!(windows) {
                    " and put ffmpeg.exe in "
                } else {
                    " and put ffmpeg in "
                }
                code { "{polishcast_export::tools::dir().display()}" }
                "."
            }
        }
    }
}
