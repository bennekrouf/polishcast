//! The banner offering to remove Small Video, this app under its old name.

use super::*;

pub(super) fn old_app_banner(
    mut old_app: Signal<Option<PathBuf>>,
    mut old_app_error: Signal<Option<String>>,
    mut settings: Signal<Settings>,
) -> Element {
    rsx! {
        if let Some(app) = old_app() {
            div { class: "kit-banner",
                span { class: "kit-banner-text",
                    if let Some(e) = old_app_error() {
                        "Couldn't move Small Video to the Trash ({e}). Drag it there from {platform::FILE_MANAGER}."
                    } else {
                        "Small Video, this app under its old name, is still in Applications. "
                        "Opening it starts the old version."
                    }
                }
                if old_app_error().is_some() {
                    button {
                        class: "kit-banner-link",
                        onclick: {
                            let app = app.clone();
                            move |_| platform::reveal(&app)
                        },
                        "{platform::SHOW_IN_FOLDER}"
                    }
                } else {
                    button {
                        class: "kit-banner-link",
                        onclick: {
                            let app = app.clone();
                            move |_| match crate::old_app::remove(&app) {
                                Ok(()) => old_app.set(None),
                                Err(e) => old_app_error.set(Some(e)),
                            }
                        },
                        "Move to Trash"
                    }
                    button {
                        class: "kit-banner-link",
                        onclick: move |_| {
                            settings.write().keep_old_app = true;
                            settings.peek().save();
                            old_app.set(None);
                        },
                        "Keep"
                    }
                }
                button { class: "kit-banner-close", title: "Dismiss", onclick: move |_| old_app.set(None), "×" }
            }
        }
    }
}
