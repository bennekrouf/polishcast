//! The window that says how to connect Claude or Codex to PolishCast.

use super::*;

/// Whether the "Use PolishCast from Claude or Codex" window is open. Provided once, by `App`.
#[derive(Clone, Copy)]
pub struct McpOpen(pub Signal<bool>);

#[component]
pub fn McpDialog() -> Element {
    let mut open = use_context::<McpOpen>().0;
    let mut settings = use_context::<Signal<crate::settings::Settings>>();
    if !open() {
        return rsx! {};
    }
    rsx! {
        desktop_kit::ui::ConnectWindow {
            lead: "Ask Claude to polish your takes: add or tighten zooms, cut the pauses, trim the start and \
                   end, change the background or the frame's shape, and export an MP4 or a GIF. An open take \
                   shows its changes within a couple of seconds, and \u{2318}Z takes them back.",
            try_it: "\u{201c}Cut the pauses in my last take, make it 16:9 and export a GIF.\u{201d} With recording \
                     on: \u{201c}Record a 30-second demo of the export dialog in Safari and make it a GIF.\u{201d}",
            extra: rsx! {
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: settings.read().allow_mcp_recording,
                        onchange: move |e| {
                            settings.write().allow_mcp_recording = e.checked();
                            settings.peek().save();
                        },
                    }
                    "Let Claude or Codex record the screen"
                }
                p { class: "kit-pro-hint",
                    "So it can make a whole demo by itself: start recording, use the app, stop and polish. The menu \
                     bar counts down first, shows who is recording, and Stop Recording there stops it at any time. \
                     A recording started this way stops by itself after 10 minutes unless asked for longer."
                }
            },
            on_close: move |_| open.set(false),
        }
    }
}

/// A block of text to paste somewhere else, with its own Copy button.
#[component]
pub(crate) fn Snippet(text: String) -> Element {
    let mut copied = use_signal(|| false);
    let to_copy = text.clone();
    rsx! {
        div { class: "mcp-snippet-wrap",
            pre { class: "mcp-snippet", "{text}" }
            button {
                class: "mcp-copy",
                onclick: move |_| {
                    // The native clipboard: the web view's needs a secure origin and a user
                    // gesture check that differ per platform.
                    let ok = arboard::Clipboard::new().and_then(|mut c| c.set_text(to_copy.clone())).is_ok();
                    copied.set(ok);
                },
                if copied() { "Copied ✓" } else { "Copy" }
            }
        }
    }
}
