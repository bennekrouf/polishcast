//! Build script: links the Swift runtime on macOS, and embeds the app icon into the .exe on
//! Windows (as TrackCut does).
//!
//! The Windows icon is `assets/icon.ico`, generated from `assets/icon.png` (release CI runs
//! `magick icon.png -define icon:auto-resize=16,24,32,48,64,128,256 icon.ico`). If it's
//! missing, the build still succeeds, icon-less, with a `cargo:warning`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // ScreenCaptureKit's bindings are Swift; the binary must find the Swift runtime that ships
    // with macOS. The bindings' own build script adds this rpath only to its own targets.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,/usr/lib/swift");
    }

    #[cfg(target_os = "windows")]
    {
        let icon_path = std::path::Path::new("assets/icon.ico");
        if !icon_path.exists() {
            println!(
                "cargo:warning=assets/icon.ico not found — the .exe will ship without an embedded icon. \
                 Generate it from assets/icon.png (see this file's header)."
            );
            return;
        }
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("FileDescription", "PolishCast");
        res.set("ProductName", "PolishCast");
        res.set("CompanyName", "Mayorana");
        res.set("LegalCopyright", "© 2026 Mayorana");
        if let Err(e) = res.compile() {
            // rc.exe / windres isn't on every Windows machine; warn and ship icon-less rather
            // than failing the build.
            println!("cargo:warning=Failed to embed Windows icon resource: {e} (the build continues without it)");
        }
    }
}
