//! Choosing an area of the screen to record: a see-through window over the display the pointer
//! is on. Drag across the part to record; Esc cancels. The answer goes back over a channel,
//! since the picker is a window of its own with its own virtual DOM.

use dioxus::desktop::tao::monitor::MonitorHandle;
use dioxus::desktop::{window, Config, LogicalPosition, LogicalSize, WindowBuilder, WindowCloseBehaviour};
use dioxus::prelude::*;
use polishcast_capture::Area;
use std::sync::mpsc::Sender;

const CSS: &str = r#"
html, body { margin: 0; height: 100%; background: transparent; overflow: hidden; user-select: none; }
.picker {
  position: fixed; inset: 0; cursor: crosshair; outline: none;
  font: 13px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; color: #fff;
}
.picker.empty { background: rgba(0, 0, 0, 0.3); }
.picker .hint {
  position: absolute; top: 40%; left: 50%; transform: translate(-50%, -50%);
  background: rgba(24, 24, 27, 0.85); padding: 8px 14px; border-radius: 8px; pointer-events: none;
}
.selection {
  position: absolute; border: 1px solid #fff; pointer-events: none;
  box-shadow: 0 0 0 100vmax rgba(0, 0, 0, 0.3);
}
.selection .size {
  position: absolute; right: 0; bottom: -24px; background: rgba(24, 24, 27, 0.85);
  padding: 1px 6px; border-radius: 4px; font-size: 11px; font-variant-numeric: tabular-nums;
}
"#;

/// Smaller than this, a drag is taken for a stray click.
const MIN_SIDE: f64 = 16.0;

/// The picker's reply channel. Props must compare; a channel is always the same one.
#[derive(Clone)]
pub struct Reply(pub Sender<Option<Area>>);

impl PartialEq for Reply {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Opens the picker over the display the pointer is on. `reply` gets the area chosen, in
/// global points, or `None` when it was cancelled.
pub fn open(reply: Sender<Option<Area>>) {
    let (x, y, w, h) = display_under_pointer();
    let dom = VirtualDom::new_with_props(Picker, PickerProps { reply: Reply(reply) });
    let cfg = Config::new()
        .with_window(
            WindowBuilder::new()
                .with_title("Choose the area to record")
                .with_decorations(false)
                .with_transparent(true)
                .with_always_on_top(true)
                .with_resizable(false)
                .with_position(LogicalPosition::new(x, y))
                .with_inner_size(LogicalSize::new(w, h)),
        )
        .with_background_color((0, 0, 0, 0))
        .with_close_behaviour(WindowCloseBehaviour::WindowCloses);
    window().new_window(dom, cfg);
}

/// The display under the pointer, in global points: x, y, width, height.
fn display_under_pointer() -> (f64, f64, f64, f64) {
    let main = window();
    let rect = |m: &MonitorHandle| {
        let k = m.scale_factor();
        let (p, s) = (m.position().to_logical::<f64>(k), m.size().to_logical::<f64>(k));
        (p.x, p.y, s.width, s.height)
    };
    let at = polishcast_capture::pointer_location();
    main.window
        .available_monitors()
        .map(|m| rect(&m))
        .find(|&(x, y, w, h)| at.is_some_and(|(px, py)| px >= x && px < x + w && py >= y && py < y + h))
        .or_else(|| main.window.primary_monitor().map(|m| rect(&m)))
        .unwrap_or((0.0, 0.0, 1440.0, 900.0))
}

/// The rectangle between two corners, as left, top, width, height.
fn span((ax, ay): (f64, f64), (bx, by): (f64, f64)) -> (f64, f64, f64, f64) {
    (ax.min(bx), ay.min(by), (ax - bx).abs(), (ay - by).abs())
}

#[component]
fn Picker(reply: Reply) -> Element {
    let mut from = use_signal(|| Option::<(f64, f64)>::None);
    let mut to = use_signal(|| (0.0f64, 0.0f64));
    let finish = move |area: Option<Area>| {
        let _ = reply.0.send(area);
        window().close();
    };
    let finish_key = finish.clone();
    let rect = from().map(|a| span(a, to()));

    rsx! {
        style { {CSS} }
        div {
            class: if rect.is_some() { "picker" } else { "picker empty" },
            tabindex: "0",
            onmounted: move |e| async move {
                window().set_focus();
                let _ = e.set_focus(true).await;
            },
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    finish_key(None);
                }
            },
            onmousedown: move |e: MouseEvent| {
                let p = e.client_coordinates();
                from.set(Some((p.x, p.y)));
                to.set((p.x, p.y));
            },
            onmousemove: move |e: MouseEvent| {
                if from.peek().is_some() {
                    let p = e.client_coordinates();
                    to.set((p.x, p.y));
                }
            },
            onmouseup: move |e: MouseEvent| {
                let Some(a) = from.take() else { return };
                let p = e.client_coordinates();
                let (x, y, w, h) = span(a, (p.x, p.y));
                if w < MIN_SIDE || h < MIN_SIDE {
                    return;
                }
                // The page's points are the window's; the window may not sit exactly where it
                // was asked to (macOS keeps windows clear of the menu bar).
                let win = window();
                let origin = win.window.inner_position().map(|o| o.to_logical::<f64>(win.window.scale_factor()));
                let (ox, oy) = origin.map_or((0.0, 0.0), |o| (o.x, o.y));
                finish(Some(Area { x: ox + x, y: oy + y, w, h }));
            },
            if let Some((x, y, w, h)) = rect {
                div {
                    class: "selection",
                    left: "{x}px",
                    top: "{y}px",
                    width: "{w}px",
                    height: "{h}px",
                    span { class: "size", "{w.round()} × {h.round()}" }
                }
            } else {
                div { class: "hint", "Drag across the part of the screen to record · Esc cancels" }
            }
        }
    }
}
