//! The window that says how to connect Claude or Codex to PolishCast.

use super::*;

/// Whether the "Use PolishCast from Claude or Codex" window is open. Provided once, by `App`.
#[derive(Clone, Copy)]
pub struct McpOpen(pub Signal<bool>);

/// This executable, as a client should start it.
pub(super) fn command() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "polishcast".into())
}

/// The `mcpServers` entry for Claude Desktop's config file. Built with serde so a Windows
/// path's backslashes arrive escaped.
pub fn desktop_config(command: &str) -> String {
    let config = json!({ "mcpServers": { "polishcast": { "command": command, "args": ["mcp"] } } });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

/// The one line that adds it to Claude Code, for every project.
pub fn code_command(command: &str) -> String {
    format!("claude mcp add polishcast --scope user -- \"{command}\" mcp")
}

/// The one line that adds it to Codex. Its CLI, IDE extension and app share
/// one list of servers, each one there for every project.
pub fn codex_command(command: &str) -> String {
    format!("codex mcp add polishcast -- \"{command}\" mcp")
}

pub(super) fn desktop_config_path() -> &'static str {
    if cfg!(target_os = "windows") {
        "%APPDATA%\\Claude\\claude_desktop_config.json"
    } else if cfg!(target_os = "macos") {
        "~/Library/Application Support/Claude/claude_desktop_config.json"
    } else {
        "~/.config/Claude/claude_desktop_config.json"
    }
}

#[component]
pub fn McpDialog() -> Element {
    let mut open = use_context::<McpOpen>().0;
    let mut settings = use_context::<Signal<crate::settings::Settings>>();
    if !open() {
        return rsx! {};
    }
    let command = command();
    let desktop = desktop_config(&command);
    let code = code_command(&command);
    let codex = codex_command(&command);
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| open.set(false),
            div { class: "modal mcp-modal", onclick: move |e| e.stop_propagation(),
                div { class: "modal-head",
                    h2 { "Use PolishCast from Claude or Codex" }
                    button { class: "modal-close", title: "Close", onclick: move |_| open.set(false), "×" }
                }
                div { class: "modal-body",
                    p { class: "hint",
                        "Ask Claude to polish your takes: add or tighten zooms, cut the pauses, trim the start and end, \
                         change the background or the frame's shape, and export an MP4 or a GIF. An open take shows \
                         its changes within a couple of seconds, and \u{2318}Z takes them back."
                    }
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
                    p { class: "hint",
                        "So it can make a whole demo by itself: start recording, use the app, stop and polish. The menu \
                         bar counts down first, shows who is recording, and Stop Recording there stops it at any time. \
                         A recording started this way stops by itself after 10 minutes unless asked for longer."
                    }
                    h3 { "1 · Claude Desktop" }
                    p { class: "hint",
                        "Settings \u{2192} Developer \u{2192} Edit Config, paste this in and save, then quit Claude and open it \
                         again. If the file already has an mcpServers section, add just the polishcast entry to it."
                    }
                    Snippet { text: desktop.clone() }
                    p { class: "hint mcp-path", "The file: {desktop_config_path()}" }
                    h3 { "2 · Claude Code" }
                    p { class: "hint", "Run this once in a terminal:" }
                    Snippet { text: code.clone() }
                    h3 { "3 · Codex" }
                    p { class: "hint", "Run this once in a terminal:" }
                    Snippet { text: codex.clone() }
                    h3 { "4 · Try it" }
                    p { class: "hint",
                        "Ask: \u{201c}Cut the pauses in my last take, make it 16:9 and export a GIF.\u{201d} With \
                         recording on: \u{201c}Record a 30-second demo of the export dialog in Safari and make it a GIF.\u{201d}"
                    }
                    p { class: "hint mcp-note",
                        "Claude in the browser (claude.ai) can\u{2019}t use this: it only connects to servers on the internet, \
                         and PolishCast runs on your computer. Any other app that speaks MCP can, with the same command: "
                        code { "{command} mcp" }
                    }
                    div { class: "modal-actions",
                        button { onclick: move |_| open.set(false), "Close" }
                    }
                }
            }
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
