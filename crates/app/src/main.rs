// Release builds on Windows are GUI apps: no console window next to the main one.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod area_picker;
mod editor;
mod feedback;
mod licence;
mod limits;
mod mcp;
mod media_server;
mod old_app;
mod platform;
mod settings;
mod takes;
mod theme;
mod update_check;
mod views;

use dioxus::desktop::{Config, LogicalSize, WindowBuilder, WindowCloseBehaviour};

fn main() {
    // `polishcast mcp`: the takes over stdio for an MCP client, and no window.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return mcp::serve();
    }
    let cfg = Config::new()
        .with_data_directory(webview_data_dir())
        .with_window(
            WindowBuilder::new()
                .with_title(concat!("PolishCast ", env!("CARGO_PKG_VERSION")))
                .with_inner_size(LogicalSize::new(1280.0, 820.0))
                .with_window_icon(window_icon()),
        )
        .with_background_color((24, 24, 27, 255))
        // Closing the window keeps the app in the menu bar, ready to record.
        .with_close_behaviour(WindowCloseBehaviour::WindowHides)
        // Clicking the menu bar item only opens its menu: starting a recording shouldn't
        // bring the window up. "Show PolishCast" does, and so does a finished take.
        .with_tray_icon_show_window_on_click(false);
    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(views::App);
}

/// Where the web view keeps its data. The default is next to the executable, which isn't
/// writable when installed for all users (WebView2 then fails to start on Windows).
fn webview_data_dir() -> std::path::PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("Small Video")
}

/// The window icon (title bar, alt-tab and taskbar on Windows and Linux; macOS takes the
/// Dock icon from the app bundle instead). Downscaled to 64 px: the platform gets this one
/// bitmap for every size, and a 1024 px source squeezed into a 16 px title bar looks muddy.
fn window_icon() -> Option<dioxus::desktop::tao::window::Icon> {
    const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
    const SIZE: u32 = 64;
    let img = image::load_from_memory(ICON_PNG).ok()?.resize_exact(SIZE, SIZE, image::imageops::FilterType::Lanczos3);
    dioxus::desktop::tao::window::Icon::from_rgba(img.into_rgba8().into_raw(), SIZE, SIZE).ok()
}

/// RGBA pixels, width and height of an embedded PNG, scaled to `height` pixels high if taller.
fn decode_png(png: &[u8], height: u32) -> Option<(Vec<u8>, u32, u32)> {
    let mut img = image::load_from_memory(png).ok()?;
    if img.height() > height {
        img = img.resize(u32::MAX, height, image::imageops::FilterType::Lanczos3);
    }
    let img = img.into_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}
