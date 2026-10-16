//! The PolishCast Pro licence on this computer, the free version's exports, and the Pro
//! window. The check itself is offline, in `polishcast_core::license`.

use base64::Engine;
use dioxus::prelude::*;
use polishcast_core::license::{self, Exported, License, FREE_EXPORTS};
use std::path::PathBuf;

/// Where "Buy PolishCast Pro" leads.
pub const BUY_URL: &str = "https://mayorana.ch/en/apps/polishcast";

/// The public half of mayorana.ch's licence signing key (32 bytes, standard base64), built in
/// by the release workflow from the `POLISHCAST_LICENSE_PUBLIC_KEY` variable.
const PUBLIC_KEY: Option<&str> = option_env!("POLISHCAST_LICENSE_PUBLIC_KEY");

/// This build's release date (`YYYY-MM-DD`), set by build.rs. Empty when unknown, which every
/// licence covers.
const RELEASE_DATE: &str = env!("POLISHCAST_RELEASE_DATE");

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// No licence on this computer.
    Free,
    /// Licensed, and this build is covered.
    Pro(License),
    /// Licensed, but this build was released after the licence's updates ended.
    Renew(License),
    /// A build without the public key (a local build): licences can't be checked, and
    /// nothing is limited.
    Unavailable,
}

impl Status {
    /// Whether every take can be exported. A build that can't check licences isn't limited
    /// either: that is a build from source, or a release missing its key, and neither should
    /// lock out someone who paid.
    pub fn unlimited(&self) -> bool {
        matches!(self, Status::Pro(_) | Status::Unavailable)
    }
}

/// Shared by the sidebar's Pro button and the export panel.
#[derive(Clone, Copy)]
pub struct Pro {
    pub status: Signal<Status>,
    pub exported: Signal<Exported>,
    /// The Pro window is open; `Some(take)` when it opened because that take's export was
    /// refused.
    pub open: Signal<Option<Option<String>>>,
}

impl Pro {
    pub fn new() -> Self {
        Self { status: Signal::new(current()), exported: Signal::new(load_exported()), open: Signal::new(None) }
    }

    /// Whether `take` may be exported now. Re-read from disk: another window may have exported.
    pub fn may_export(mut self, take: &str) -> bool {
        if self.status.peek().unlimited() {
            return true;
        }
        let exported = load_exported();
        let ok = exported.allows(take);
        self.exported.set(exported);
        ok
    }

    /// Counts a finished export (nothing with Pro).
    pub fn exported(mut self, take: &str) {
        if self.status.peek().unlimited() {
            return;
        }
        let mut exported = load_exported();
        if exported.record(take) {
            save_exported(&exported);
        }
        self.exported.set(exported);
    }
}

fn data_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("Small Video"))
}

fn public_key() -> Option<[u8; 32]> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(PUBLIC_KEY?.trim()).ok()?;
    bytes.try_into().ok()
}

fn status_of(license: License) -> Status {
    if license.covers(RELEASE_DATE) {
        Status::Pro(license)
    } else {
        Status::Renew(license)
    }
}

/// The licence saved on this computer, checked again at every start: a key that no longer
/// verifies counts as none.
pub fn current() -> Status {
    let Some(public) = public_key() else { return Status::Unavailable };
    let Some(key) = data_dir().and_then(|d| std::fs::read_to_string(d.join("licence")).ok()) else {
        return Status::Free;
    };
    match license::verify(&key, &public) {
        Ok(l) => status_of(l),
        Err(_) => Status::Free,
    }
}

/// Checks `key` and, if it is a PolishCast licence, saves it for the next starts.
pub fn activate(key: &str) -> Result<Status, String> {
    let public = public_key().ok_or("This build of PolishCast can't check licences. Download it from mayorana.ch.")?;
    let license = license::verify(key, &public).map_err(|e| e.to_string())?;
    let dir = data_dir().ok_or("No settings folder to keep the licence in.")?;
    let key: String = key.chars().filter(|c| !c.is_whitespace()).collect();
    std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(dir.join("licence"), key))
        .map_err(|e| format!("The licence couldn't be saved: {e}"))?;
    Ok(status_of(license))
}

/// Removes the licence from this computer (to move it to another one).
pub fn deactivate() {
    if let Some(dir) = data_dir() {
        let _ = std::fs::remove_file(dir.join("licence"));
    }
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

/// The Pro window: what the free version allows, the licence key, and the way to buy one.
#[component]
pub fn ProDialog() -> Element {
    let pro = use_context::<Pro>();
    let mut status = pro.status;
    let open = pro.open;
    let mut key = use_signal(String::new);
    let mut problem = use_signal(|| Option::<String>::None);
    let Some(refused) = open() else { return rsx! {} };
    let current = status();
    let left = pro.exported.read().left();
    let close = move || {
        let (mut open, mut problem) = (open, problem);
        open.set(None);
        problem.set(None);
    };
    let activate = move || {
        let (mut status, mut key, mut problem) = (status, key, problem);
        let pasted = key.peek().clone();
        match activate(&pasted) {
            Ok(s) => {
                status.set(s);
                key.set(String::new());
                problem.set(None);
            }
            Err(e) => problem.set(Some(e)),
        }
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| close(),
            div { class: "modal", onclick: move |e| e.stop_propagation(),
                h2 { "PolishCast Pro" }
                match &current {
                    Status::Pro(l) => rsx! {
                        p { "Licensed to " strong { "{l.email}" } ". Every take can be exported." }
                        p { class: "hint", "Includes every update released until {l.updates_until}." }
                    },
                    Status::Renew(l) => rsx! {
                        p { "Licensed to " strong { "{l.email}" } "." }
                        p { class: "error",
                            "This version was released on {RELEASE_DATE}, after your updates ended on {l.updates_until}. "
                            "Renew to use it, or keep using a version released before that day."
                        }
                    },
                    Status::Unavailable => rsx! {
                        p { class: "hint",
                            "This build of PolishCast can't check licences, and exports every take. "
                            "Download PolishCast from mayorana.ch to use a licence."
                        }
                    },
                    Status::Free => rsx! {
                        if refused.is_some() {
                            p { class: "error", "You've used the free version's {FREE_EXPORTS} exports." }
                        }
                        p {
                            "The free version exports {FREE_EXPORTS} different takes; exporting one of them again "
                            "is free. Recording and editing are never limited. PolishCast Pro exports every take."
                        }
                        p { class: "hint", "Free exports left: {left} of {FREE_EXPORTS}" }
                    },
                }
                if matches!(current, Status::Free | Status::Renew(_)) {
                    textarea {
                        class: "licence-key",
                        rows: "4",
                        spellcheck: "false",
                        placeholder: "Paste the licence key from your purchase email",
                        value: "{key}",
                        onmounted: move |e| async move {
                            let _ = e.set_focus(true).await;
                        },
                        oninput: move |e| {
                            problem.set(None);
                            key.set(e.value());
                        },
                        onkeydown: move |e: KeyboardEvent| {
                            e.stop_propagation();
                            match e.key() {
                                Key::Escape => close(),
                                Key::Enter => {
                                    e.prevent_default();
                                    activate();
                                }
                                _ => {}
                            }
                        },
                    }
                    if let Some(p) = problem() {
                        p { class: "error", "{p}" }
                    }
                }
                div { class: "modal-actions",
                    if matches!(current, Status::Pro(_) | Status::Renew(_)) {
                        button {
                            class: "left",
                            title: "Remove the licence from this computer, e.g. to use it on another one",
                            onclick: move |_| {
                                deactivate();
                                status.set(Status::Free);
                            },
                            "Remove from this computer"
                        }
                    }
                    if !matches!(current, Status::Pro(_)) {
                        a { class: "button-link", href: "{BUY_URL}", target: "_blank",
                            if matches!(current, Status::Renew(_)) { "Renew…" } else { "Buy PolishCast Pro…" }
                        }
                    }
                    button { onclick: move |_| close(), if matches!(current, Status::Pro(_)) { "Close" } else { "Cancel" } }
                    if matches!(current, Status::Free | Status::Renew(_)) {
                        button { class: "primary", disabled: key.read().trim().is_empty(), onclick: move |_| activate(), "Activate" }
                    }
                }
            }
        }
    }
}
