//! PolishCast Pro: the free version's exports, and what the Pro window says about them.
//! The licence itself (the number, api0's answer, the daily check, the shared file) is
//! `desktop_kit::licence`, and so are the window and the badge; what api0's answer unlocks
//! is `polishcast_core::license`.

use dioxus::prelude::*;
use polishcast_core::license::{Exported, FREE_EXPORTS};
use std::path::PathBuf;

pub use desktop_kit::licence::{buy_url, current, title_badge, Status};

/// Shared by the sidebar's Pro button and the export panel.
#[derive(Clone, Copy)]
pub struct Pro {
    pub status: Signal<Status>,
    pub exported: Signal<Exported>,
    /// The Pro window is open.
    pub open: Signal<bool>,
    /// The take whose export opened the window, when it was refused.
    pub refused: Signal<Option<String>>,
}

impl Pro {
    pub fn new() -> Self {
        Self {
            status: Signal::new(current()),
            exported: Signal::new(load_exported()),
            open: Signal::new(false),
            refused: Signal::new(None),
        }
    }

    /// The window's state, for desktop-kit's window and badge.
    pub fn kit(self) -> desktop_kit::ui::Pro {
        desktop_kit::ui::Pro { open: self.open, status: self.status }
    }

    /// Opens the Pro window, saying `take` was refused when one was.
    pub fn ask(mut self, take: Option<String>) {
        self.refused.set(take);
        self.open.set(true);
    }

    /// Whether `take` may be exported now. Re-read from disk: another window may have exported.
    pub fn may_export(mut self, take: &str) -> bool {
        if self.status.peek().is_pro() {
            return true;
        }
        let exported = load_exported();
        let ok = exported.allows(take);
        self.exported.set(exported);
        ok
    }

    /// Counts a finished export (nothing with Pro).
    pub fn exported(mut self, take: &str) {
        if self.status.peek().is_pro() {
            return;
        }
        let mut exported = load_exported();
        if exported.record(take) {
            save_exported(&exported);
        }
        self.exported.set(exported);
    }
}

/// Whether `take` may be exported, by the same rule and from the same file as the window's
/// export panel: for `mcp`, which has no `Pro` signals.
pub fn allows_export(take: &str) -> bool {
    current().is_pro() || load_exported().allows(take)
}

/// Counts a finished export of `take` against the free version's, as the window does.
pub fn count_export(take: &str) {
    if current().is_pro() {
        return;
    }
    let mut exported = load_exported();
    if exported.record(take) {
        save_exported(&exported);
    }
}

/// Free exports not used yet; `None` when exports aren't limited.
pub fn free_exports_left() -> Option<usize> {
    (!current().is_pro()).then(|| load_exported().left())
}

fn data_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("Small Video"))
}

fn load_exported() -> Exported {
    data_dir()
        .and_then(|d| std::fs::read(d.join("exported.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_exported(exported: &Exported) {
    if let (Some(dir), Ok(json)) = (data_dir(), serde_json::to_vec_pretty(exported)) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("exported.json"), json);
    }
}

/// The badge's tooltip while PolishCast isn't Pro.
pub fn hint() -> String {
    format!("The free version exports {FREE_EXPORTS} different takes; Pro exports every take")
}

/// What the free version does and what Pro adds, in the Pro window: first that `refused`'s
/// export was, when it was.
pub fn pitch(refused: Option<String>) -> Element {
    rsx! {
        if refused.is_some() {
            p { class: "kit-pro-error", "You've used the free version's {FREE_EXPORTS} exports." }
        }
        p {
            "The free version exports {FREE_EXPORTS} different takes; exporting one of them again "
            "is free. Recording and editing are never limited. PolishCast Pro exports every take; "
            "it comes with a monthly plan on mayorana.ch."
        }
    }
}

/// The free exports left, while PolishCast isn't Pro.
pub fn details(pro: Pro) -> Option<Element> {
    if pro.status.read().is_pro() {
        return None;
    }
    let left = pro.exported.read().left();
    Some(rsx! {
        p { "Free exports left: {left} of {FREE_EXPORTS}" }
    })
}
