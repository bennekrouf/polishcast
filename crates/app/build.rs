//! Build script: links the Swift runtime on macOS, and embeds the app icon into the .exe on
//! Windows (as TrackCut does).
//!
//! The Windows icon is `assets/icon.ico`, generated from `assets/icon.png` (release CI runs
//! `magick icon.png -define icon:auto-resize=16,24,32,48,64,128,256 icon.ico`). If it's
//! missing, the build still succeeds, icon-less, with a `cargo:warning`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // What a Pro licence's `updates_until` is compared with: POLISHCAST_RELEASE_DATE if set,
    // else the date of the commit being built (the release commit, in CI), else empty
    // (unknown: every licence covers it). The public key is read by the app with
    // `option_env!`; listed here so changing it rebuilds.
    println!("cargo:rerun-if-env-changed=POLISHCAST_RELEASE_DATE");
    println!("cargo:rerun-if-env-changed=POLISHCAST_LICENSE_PUBLIC_KEY");
    let date = std::env::var("POLISHCAST_RELEASE_DATE").ok().filter(|d| !d.is_empty()).or_else(commit_date);
    println!("cargo:rustc-env=POLISHCAST_RELEASE_DATE={}", date.unwrap_or_default());

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

/// The committer date of HEAD, `YYYY-MM-DD`.
fn commit_date() -> Option<String> {
    let out = std::process::Command::new("git").args(["log", "-1", "--format=%cs"]).output().ok()?;
    let date = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && date.len() == 10).then_some(date)
}
