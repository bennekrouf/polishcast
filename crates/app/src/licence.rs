//! The PolishCast Pro licence on this computer, the free version's exports, and the Pro
//! window. The licence number is kept with api0's last answer about it; what that answer
//! unlocks is `polishcast_core::license`.
//!
//! ```text
//! GET  https://gateway.api0.ai/v1/licence          the plan, its apps, credits left
//! POST https://gateway.api0.ai/v1/licence/portal   { url } to change or cancel it
//! Authorization: Bearer <licence>
//! X-Client-App: small-video
//! ```

use dioxus::prelude::*;
pub use polishcast_core::license::Status;
use polishcast_core::license::{self, Answer, Exported, FreeError, Saved, Shared, FREE_EXPORTS, PRODUCT};
use polishcast_export::http;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where "Buy PolishCast Pro" leads.
pub const BUY_URL: &str = "https://mayorana.ch/en/apps/polishcast";

const GATEWAY: &str = "https://gateway.api0.ai";

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

/// Whether `take` may be exported, by the same rule and from the same file as the window's
/// export panel: for `mcp`, which has no `Pro` signals.
pub fn allows_export(take: &str) -> bool {
    current().unlimited() || load_exported().allows(take)
}

/// Counts a finished export of `take` against the free version's, as the window does.
pub fn count_export(take: &str) {
    if current().unlimited() {
        return;
    }
    let mut exported = load_exported();
    if exported.record(take) {
        save_exported(&exported);
    }
}

/// Free exports not used yet; `None` when exports aren't limited.
pub fn free_exports_left() -> Option<usize> {
    (!current().unlimited()).then(|| load_exported().left())
}

fn data_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("Small Video"))
}

fn licence_file() -> Option<PathBuf> {
    Some(data_dir()?.join("licence.json"))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn load_own() -> Option<Saved> {
    serde_json::from_str(&std::fs::read_to_string(licence_file()?).ok()?).ok()
}

/// The licence file every Mayorana app shares: `~/Library/Application Support/mayorana/licence`
/// on a Mac, `%APPDATA%\mayorana\licence` on Windows, `~/.config/mayorana/licence` on Linux.
fn shared_file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("mayorana").join("licence"))
}

fn load_shared() -> Shared {
    match shared_file().map(std::fs::read_to_string) {
        Some(Ok(text)) => license::shared_from(&text),
        _ => Shared::Missing,
    }
}

/// Writes `text` as the shared licence: a number, or nothing once removed.
fn save_shared(text: &str) {
    let Some(f) = shared_file() else { return };
    // Named after this app, so two apps writing at once don't share it.
    let tmp = f.with_extension(format!("{PRODUCT}.tmp"));
    let _ = std::fs::create_dir_all(f.parent().unwrap()).and_then(|_| std::fs::write(&tmp, text)).and_then(|_| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &f)
    });
}

/// This app's licence, or the one typed in another Mayorana app.
fn load() -> Option<Saved> {
    let shared = load_shared();
    if shared == Shared::Removed {
        if let Some(f) = licence_file() {
            let _ = std::fs::remove_file(f);
        }
    }
    let (saved, share) = license::reconcile(shared, load_own());
    if let (Some(saved), true) = (&saved, share) {
        save_shared(&saved.licence);
    }
    saved
}

fn save(saved: &Saved) -> Result<(), String> {
    let f = licence_file().ok_or("No settings folder to keep the licence in.")?;
    let json = serde_json::to_vec_pretty(saved).map_err(|e| e.to_string())?;
    // Written beside the target and renamed over it, so a crash can't leave half a file.
    let tmp = f.with_extension("tmp");
    std::fs::create_dir_all(f.parent().unwrap())
        .and_then(|_| std::fs::write(&tmp, json))
        .and_then(|_| std::fs::rename(&tmp, &f))
        .map_err(|e| format!("The licence couldn't be saved: {e}"))
}

/// The licence as of the last check, without asking api0.
pub fn current() -> Status {
    license::status_at(load().as_ref(), now())
}

/// Why a check didn't give an answer.
enum CheckError {
    /// api0 doesn't know this licence number.
    Unknown,
    /// Offline, api0 down, or an answer that made no sense.
    Unreachable(String),
    /// The connection came with a certificate this computer doesn't trust: a company proxy's,
    /// usually.
    Untrusted(String),
}

fn unreachable(e: ureq::Error) -> CheckError {
    if http::untrusted(&e) {
        CheckError::Untrusted(e.to_string())
    } else {
        CheckError::Unreachable(e.to_string())
    }
}

fn agent() -> ureq::Agent {
    http::config().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build().into()
}

fn ask(licence: &str) -> Result<Answer, CheckError> {
    let mut resp = agent()
        .get(format!("{GATEWAY}/v1/licence"))
        .header("Authorization", format!("Bearer {licence}"))
        .header("X-Client-App", PRODUCT)
        .call()
        .map_err(unreachable)?;
    match resp.status().as_u16() {
        200 => {
            let body = resp.body_mut().read_to_string().map_err(|e| CheckError::Unreachable(e.to_string()))?;
            serde_json::from_str(&body).map_err(|e| CheckError::Unreachable(e.to_string()))
        }
        401 => Err(CheckError::Unknown),
        code => Err(CheckError::Unreachable(format!("api0 answered {code}"))),
    }
}

/// Checks `typed` with api0 and, if api0 knows it, saves it. Blocking.
fn activate(typed: &str) -> Result<Status, String> {
    let licence = license::normalise(typed)
        .ok_or("This isn't a licence number. It has 20 letters and digits, like ABCDE-FGHJK-LMNPQ-RSTUV.")?;
    let answer = match ask(&licence) {
        Ok(a) => a,
        Err(CheckError::Unknown) => return Err("This licence number isn't known. Check it against the email.".into()),
        Err(CheckError::Unreachable(e)) => {
            return Err(format!("The licence couldn't be checked ({e}). Check the connection and try again."))
        }
        Err(CheckError::Untrusted(e)) => {
            return Err(format!("The licence couldn't be checked ({e}). {}", http::UNTRUSTED_HINT))
        }
    };
    let saved = Saved { licence, answer: Some(answer), checked: now() };
    save(&saved)?;
    save_shared(&saved.licence);
    Ok(license::status_at(Some(&saved), now()))
}

/// Checks the saved licence with api0 again, when `force`d or a day after the last check,
/// and keeps the answer. Offline, the last answer stands. Blocking.
fn refresh(force: bool) -> Status {
    let Some(mut saved) = load() else { return Status::Free };
    if !force && !license::check_due(&saved, now()) {
        return license::status_at(Some(&saved), now());
    }
    match ask(&saved.licence) {
        Ok(answer) => {
            saved.answer = Some(answer);
            saved.checked = now();
            let _ = save(&saved);
        }
        Err(CheckError::Unknown) => {
            saved.answer = None;
            let _ = save(&saved);
        }
        Err(CheckError::Unreachable(_) | CheckError::Untrusted(_)) => {}
    }
    license::status_at(Some(&saved), now())
}

/// The address of the page where the subscription is changed or cancelled. Blocking.
fn portal_url(licence: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Portal {
        url: String,
    }
    let failed = |e: String| format!("The subscription page couldn't be opened ({e}).");
    let mut resp = agent()
        .post(format!("{GATEWAY}/v1/licence/portal"))
        .header("Authorization", format!("Bearer {licence}"))
        .header("X-Client-App", PRODUCT)
        .send_empty()
        .map_err(|e| failed(http::describe(&e)))?;
    if resp.status().as_u16() != 200 {
        return Err(failed(format!("api0 answered {}", resp.status().as_u16())));
    }
    let body = resp.body_mut().read_to_string().map_err(|e| failed(e.to_string()))?;
    serde_json::from_str::<Portal>(&body).map(|p| p.url).map_err(|e| failed(e.to_string()))
}

/// `refresh`, off the UI thread.
pub async fn refresh_async(force: bool) -> Status {
    tokio::task::spawn_blocking(move || refresh(force)).await.unwrap_or_else(|_| current())
}

async fn activate_async(typed: String) -> Result<Status, String> {
    tokio::task::spawn_blocking(move || activate(&typed)).await.map_err(|e| e.to_string())?
}

async fn portal_url_async(licence: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || portal_url(&licence)).await.map_err(|e| e.to_string())?
}

/// Asks api0 to email `email` a licence for the free allowance of AI credits. The licence
/// number itself only goes by email. Blocking.
pub fn request_free_credits(email: &str) -> Result<String, FreeError> {
    if !license::plausible_email(email) {
        return Err(FreeError::Refused("Enter the email the licence should go to.".into()));
    }
    let failed = |e: String| FreeError::Refused(format!("The free AI credits couldn't be requested ({e})."));
    let mut resp = agent()
        .post(format!("{GATEWAY}/api/plans/free"))
        .header("X-Client-App", PRODUCT)
        .header("Content-Type", "application/json")
        .send(serde_json::json!({ "email": email.trim() }).to_string())
        .map_err(|e| failed(http::describe(&e)))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    license::free_reply(status, &body)
}

/// `request_free_credits`, off the UI thread.
pub async fn request_free_credits_async(email: String) -> Result<String, FreeError> {
    tokio::task::spawn_blocking(move || request_free_credits(&email))
        .await
        .map_err(|e| FreeError::Refused(e.to_string()))?
}

/// Opens `url` in the default browser.
pub(crate) fn open_url(url: &str) {
    use std::process::Command;
    let _ = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else if cfg!(windows) {
        Command::new("cmd").args(["/c", "start", "", url]).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
}

/// Removes the licence from this computer (to move it to another one), for every Mayorana
/// app on it.
pub fn deactivate() {
    if let Some(f) = licence_file() {
        let _ = std::fs::remove_file(f);
    }
    save_shared("");
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

/// The Pro window: what the free version allows, the plan, and the way to subscribe.
#[component]
pub fn ProDialog() -> Element {
    let pro = use_context::<Pro>();
    let mut status = pro.status;
    let open = pro.open;
    let mut key = use_signal(String::new);
    let mut problem = use_signal(|| Option::<String>::None);
    let busy = use_signal(|| false);
    let free_email = use_signal(String::new);
    let free_note = use_signal(|| Option::<String>::None);
    let free_hidden = use_signal(|| false);
    let Some(refused) = open() else { return rsx! {} };
    let current = status();
    let asks_number = matches!(current, Status::Free | Status::Unknown(_)) || current.free_credits().is_some();
    let offers_free = matches!(current, Status::Free | Status::Unknown(_)) && !free_hidden();
    let left = pro.exported.read().left();
    let close = move || {
        let (mut open, mut problem) = (open, problem);
        open.set(None);
        problem.set(None);
    };
    let activate = move || {
        let pasted = key.peek().clone();
        if pasted.trim().is_empty() || *busy.peek() {
            return;
        }
        let mut busy = busy;
        busy.set(true);
        spawn(async move {
            let (mut status, mut key, mut problem) = (status, key, problem);
            match activate_async(pasted).await {
                Ok(s) => {
                    status.set(s);
                    key.set(String::new());
                    problem.set(None);
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let check_again = move |_| {
        let mut busy = busy;
        busy.set(true);
        spawn(async move {
            status.set(refresh_async(true).await);
            busy.set(false);
        });
    };
    let ask_free = move || {
        let email = free_email.peek().clone();
        if email.trim().is_empty() || *busy.peek() {
            return;
        }
        let mut busy = busy;
        busy.set(true);
        spawn(async move {
            let (mut problem, mut free_note, mut free_hidden) = (problem, free_note, free_hidden);
            match request_free_credits_async(email).await {
                Ok(message) => {
                    free_note.set(Some(format!("{message} Paste the licence number above when it arrives.")));
                    problem.set(None);
                }
                Err(polishcast_core::license::FreeError::Unavailable) => free_hidden.set(true),
                Err(polishcast_core::license::FreeError::Refused(e)) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let manage = move |number: String| {
        let mut busy = busy;
        busy.set(true);
        spawn(async move {
            match portal_url_async(number).await {
                Ok(url) => open_url(&url),
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| close(),
            div { class: "modal", onclick: move |e| e.stop_propagation(),
                h2 { "PolishCast Pro" }
                match &current {
                    Status::Pro(a) => rsx! {
                        p { strong { "{a.plan.name}" } " plan for " strong { "{a.email}" } ". Every take can be exported." }
                        p { class: "hint", "{a.credits.left} AI credits left this month. {a.renewal()}." }
                    },
                    Status::NotPro(a) if a.is_free() => rsx! {
                        p { "Free — " strong { "{a.credits.left} AI credits" } " left this month, for " strong { "{a.email}" } ". {a.renewal()}." }
                        p { class: "hint",
                            "The free version exports {FREE_EXPORTS} different takes. PolishCast Pro exports every take; it "
                            "comes with a monthly plan on mayorana.ch."
                        }
                    },
                    Status::NotPro(a) if !a.active => rsx! {
                        p { "The subscription for " strong { "{a.email}" } " isn't active." }
                        p { class: "error", "PolishCast runs as the free version. Renew it to have Pro again." }
                    },
                    Status::NotPro(a) => rsx! {
                        p { "The " strong { "{a.plan.name}" } " plan for " strong { "{a.email}" } " doesn't include PolishCast." }
                        p { class: "error", "PolishCast runs as the free version. A plan with PolishCast makes it Pro." }
                    },
                    Status::Offline(a) => rsx! {
                        p { strong { "{a.plan.name}" } " plan for " strong { "{a.email}" } "." }
                        p { class: "error",
                            "PolishCast hasn't been able to check the licence for a week, so it runs as the free "
                            "version until it can. Check the connection, then check again."
                        }
                    },
                    Status::Unknown(_) => rsx! {
                        p { class: "error", "The licence number on this computer isn't known. Check it against the email, or paste it again." }
                    },
                    Status::Free => rsx! {
                        if refused.is_some() {
                            p { class: "error", "You've used the free version's {FREE_EXPORTS} exports." }
                        }
                        p {
                            "The free version exports {FREE_EXPORTS} different takes; exporting one of them again "
                            "is free. Recording and editing are never limited. PolishCast Pro exports every take; "
                            "it comes with a monthly plan on mayorana.ch."
                        }
                        p { class: "hint", "Free exports left: {left} of {FREE_EXPORTS}" }
                    },
                }
                if asks_number {
                    input {
                        class: "licence-key",
                        spellcheck: "false",
                        placeholder: "ABCDE-FGHJK-LMNPQ-RSTUV",
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
                    p { class: "hint",
                        "Paste the licence number from your plan's email. It's checked with api0 now and once a "
                        "day; Pro keeps working for a week without a connection."
                    }
                }
                if offers_free {
                    div { class: "free-credits",
                        p { class: "hint", "No plan? Get free AI credits every month, without Pro. The licence number comes by email." }
                        div { class: "free-row",
                            input {
                                class: "licence-key",
                                r#type: "email",
                                spellcheck: "false",
                                placeholder: "you@example.com",
                                value: "{free_email}",
                                oninput: move |e| {
                                    let mut free_email = free_email;
                                    free_email.set(e.value());
                                },
                                onkeydown: move |e: KeyboardEvent| {
                                    e.stop_propagation();
                                    match e.key() {
                                        Key::Escape => close(),
                                        Key::Enter => {
                                            e.prevent_default();
                                            ask_free();
                                        }
                                        _ => {}
                                    }
                                },
                            }
                            button { disabled: free_email.read().trim().is_empty() || busy(), onclick: move |_| ask_free(), "Get free AI credits" }
                        }
                        if let Some(n) = free_note() {
                            p { class: "hint", "{n}" }
                        }
                    }
                }
                if let Some(p) = problem() {
                    p { class: "error", "{p}" }
                }
                div { class: "modal-actions",
                    if current.licence().is_some() {
                        button {
                            class: "left",
                            title: "Removes the licence from every Mayorana app on this computer, e.g. to use it on another one",
                            onclick: move |_| {
                                deactivate();
                                status.set(Status::Free);
                            },
                            "Remove from this computer"
                        }
                    }
                    if let (Some(number), false) = (current.licence().map(str::to_owned), asks_number) {
                        button { disabled: busy(), title: "Change or cancel the plan on mayorana.ch", onclick: move |_| manage(number.clone()),
                            "Manage subscription…"
                        }
                        button { disabled: busy(), onclick: check_again, "Check again" }
                    }
                    if !matches!(current, Status::Pro(_)) {
                        a { class: "button-link", href: "{BUY_URL}", target: "_blank",
                            if current.licence().is_some() && current.free_credits().is_none() { "Renew…" } else { "Buy PolishCast Pro…" }
                        }
                    }
                    button { onclick: move |_| close(), if asks_number { "Cancel" } else { "Close" } }
                    if asks_number {
                        button { class: "primary", disabled: key.read().trim().is_empty() || busy(), onclick: move |_| activate(), "Activate" }
                    }
                }
            }
        }
    }
}
