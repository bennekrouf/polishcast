//! ffmpeg, installed into the app's data folder on first use so nothing has to be installed by
//! hand (export and compacting need it). Taken from TrackCut's installer: a static GPL build
//! with x264, yt-dlp's own builds on Windows and Martin Riedl's on macOS and Linux (FFmpeg
//! publishes no binaries itself).
//!
//! The download is pinned in `tools.pins`: one build per PolishCast release, with the SHA-256
//! it must hash to. Nothing a server sends can change what runs: a file that doesn't match is
//! refused, and a newer PolishCast with a newer pin replaces the ffmpeg it finds.
//! `scripts/pin-tools.sh` moves the pin to the latest release.

use crate::http;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// `tool os arch sha256 url`, one line per platform, written by `scripts/pin-tools.sh`.
const PINS: &str = include_str!("tools.pins");

/// One pinned download.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pin {
    sha256: &'static str,
    url: &'static str,
}

/// The pin for `tool` on this platform.
fn pin(tool: &str) -> Option<Pin> {
    pin_in(PINS, tool, std::env::consts::OS, std::env::consts::ARCH)
}

fn pin_in(pins: &'static str, tool: &str, os: &str, arch: &str) -> Option<Pin> {
    pins.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).find_map(|l| {
        let mut f = l.split_whitespace();
        let (t, o, a, sha256, url) = (f.next()?, f.next()?, f.next()?, f.next()?, f.next()?);
        (t == tool && o == os && a == arch).then_some(Pin { sha256, url })
    })
}

/// Holds the pinned hash `path` was installed from.
fn stamp(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".pin");
    path.with_file_name(name)
}

/// `path` is there and came from `pin`. A tool from before pinning, or from an older pin, isn't.
fn installed(path: &Path, pin: &Pin) -> bool {
    path.is_file() && std::fs::read_to_string(stamp(path)).is_ok_and(|s| s.trim() == pin.sha256)
}

/// Download `pin`, check it, and install it as `dst`: the file itself, or `entry` from inside
/// it when it is a zip.
fn install_pinned(
    pin: &Pin,
    entry: Option<&str>,
    dst: &Path,
    progress: &mut dyn FnMut(f32),
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let dir = dst.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let download = fetch_verified(pin.url, pin.sha256, dir, progress, cancelled)?;
    let tmp = match entry {
        None => download,
        Some(entry) => {
            let tmp = dir.join(format!("{entry}.unzipped"));
            let unzipped = unzip_one(&download, entry, &tmp);
            let _ = std::fs::remove_file(&download);
            unzipped?;
            tmp
        }
    };
    install(&tmp, dst)?;
    std::fs::write(stamp(dst), pin.sha256).map_err(|e| format!("Could not install {}: {e}", dst.display()))
}

/// %LOCALAPPDATA%\Small Video\tools, ~/Library/Application Support/Small Video/tools,
/// ~/.local/share/Small Video/tools (named after the app before it was PolishCast).
pub fn dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("Small Video").join("tools")
}

/// The app's own folder, where the Windows installers put the ffmpeg they ship.
pub fn bundled_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// A command that doesn't open a console window next to the app on Windows.
pub fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Install ffmpeg into `dir` if it isn't there yet, or is pinned differently, and return its path.
pub fn ensure_ffmpeg(
    dir: &Path,
    progress: &mut dyn FnMut(f32),
    cancelled: &dyn Fn() -> bool,
) -> Result<PathBuf, String> {
    let exe = std::env::consts::EXE_SUFFIX;
    let ffmpeg = dir.join(format!("ffmpeg{exe}"));
    let pin = pin("ffmpeg").ok_or("Video export isn't supported on this platform")?;
    if !installed(&ffmpeg, &pin) {
        install_pinned(&pin, Some(&format!("ffmpeg{exe}")), &ffmpeg, progress, cancelled)?;
    }
    Ok(ffmpeg)
}

/// Download `url` into `dir` and check its SHA-256. Returns the temporary file.
fn fetch_verified(
    url: &str,
    sha256: &str,
    dir: &Path,
    progress: &mut dyn FnMut(f32),
    cancelled: &dyn Fn() -> bool,
) -> Result<PathBuf, String> {
    let err = |e: &dyn std::fmt::Display| format!("Could not download {url}: {e}");
    let mut resp = http::agent().get(url).call().map_err(|e| err(&http::describe(&e)))?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    let name = url.rsplit('/').next().unwrap_or("download");
    let tmp = dir.join(format!("{name}.part"));
    let result = (|| {
        let mut out = File::create(&tmp).map_err(|e| err(&e))?;
        let mut body = resp.body_mut().as_reader();
        let mut hasher = Sha256::new();
        let (mut buf, mut got) = (vec![0u8; 1 << 16], 0u64);
        loop {
            if cancelled() {
                return Err("cancelled".to_string());
            }
            let n = body.read(&mut buf).map_err(|e| err(&e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| err(&e))?;
            got += n as u64;
            if let Some(t) = total.filter(|&t| t > 0) {
                progress((got as f64 / t as f64).min(1.0) as f32);
            }
        }
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if actual != sha256.to_lowercase() {
            return Err(format!("{name} isn't the file this version of PolishCast expects, so it wasn't installed"));
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Extract the file named `entry` (at the top, or in any folder) from `zip` to `dst`.
fn unzip_one(zip: &Path, entry: &str, dst: &Path) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| format!("Could not unpack {}: {e}", zip.display());
    let mut archive = zip::ZipArchive::new(File::open(zip).map_err(|e| err(&e))?).map_err(|e| err(&e))?;
    let name = archive
        .file_names()
        .find(|n| *n == entry || n.ends_with(&format!("/{entry}")))
        .map(str::to_owned)
        .ok_or_else(|| err(&format!("no {entry} inside")))?;
    let mut file = archive.by_name(&name).map_err(|e| err(&e))?;
    let mut out = File::create(dst).map_err(|e| err(&e))?;
    std::io::copy(&mut file, &mut out).map_err(|e| err(&e))?;
    Ok(())
}

/// Make `tmp` executable and move it into place.
fn install(tmp: &Path, dst: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(tmp, dst).map_err(|e| format!("Could not install {}: {e}", dst.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLATFORMS: [(&str, &str); 6] = [
        ("macos", "x86_64"),
        ("macos", "aarch64"),
        ("windows", "x86_64"),
        ("windows", "aarch64"),
        ("linux", "x86_64"),
        ("linux", "aarch64"),
    ];

    #[test]
    fn every_tool_is_pinned_for_every_platform() {
        for tool in ["ffmpeg"] {
            for (os, arch) in PLATFORMS {
                let pin = pin_in(PINS, tool, os, arch).unwrap_or_else(|| panic!("{tool} {os} {arch}"));
                assert!(
                    pin.sha256.len() == 64 && pin.sha256.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
                    "{tool} {os} {arch}: {}",
                    pin.sha256
                );
                assert!(pin.url.starts_with("https://"), "{tool} {os} {arch}: {}", pin.url);
                // A moving name like `latest` would download whatever is newest, not the pin.
                assert!(!pin.url.contains("/latest/"), "{tool} {os} {arch}: {}", pin.url);
            }
        }
        assert!(pin("ffmpeg").is_some());
    }

    #[test]
    fn pins_are_matched_on_tool_os_and_arch() {
        let pins = "# comment\nffmpeg linux x86_64 aa https://a\nffmpeg linux aarch64 bb https://b\n";
        assert_eq!(pin_in(pins, "ffmpeg", "linux", "aarch64"), Some(Pin { sha256: "bb", url: "https://b" }));
        assert_eq!(pin_in(pins, "ffmpeg", "macos", "aarch64"), None);
        assert_eq!(pin_in(pins, "deno", "linux", "x86_64"), None);
    }

    #[test]
    fn a_tool_counts_as_installed_only_from_the_same_pin() {
        let dir = std::env::temp_dir().join(format!("polishcast-pins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("ffmpeg");
        let pin = Pin { sha256: "aa", url: "https://a" };
        assert!(!installed(&tool, &pin));
        std::fs::write(&tool, b"").unwrap();
        // Downloaded before pinning: no stamp.
        assert!(!installed(&tool, &pin));
        std::fs::write(stamp(&tool), "bb").unwrap();
        assert!(!installed(&tool, &pin));
        std::fs::write(stamp(&tool), "aa").unwrap();
        assert!(installed(&tool, &pin));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Serves `body` once over plain HTTP on localhost, and returns its URL.
    fn serve_once(body: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/ffmpeg.zip", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = conn.read(&mut request);
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = conn.write_all(head.as_bytes());
            let _ = conn.write_all(body);
        });
        url
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn a_download_is_kept_only_if_it_hashes_to_its_pin() {
        let dir = std::env::temp_dir().join(format!("polishcast-pin-fetch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body: &[u8] = b"a build someone swapped in";

        let err = fetch_verified(&serve_once(body), &"0".repeat(64), &dir, &mut |_| {}, &|| false).unwrap_err();
        assert!(err.contains("isn't the file this version of PolishCast expects"), "{err}");
        assert!(!dir.join("ffmpeg.zip.part").exists(), "a refused download is left behind");

        let file = fetch_verified(&serve_once(body), &sha256_hex(body), &dir, &mut |_| {}, &|| false).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), body);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Needs the network; installs the pinned ffmpeg into the app's real tools folder.
    /// `cargo test -p polishcast-export -- --ignored installs_ffmpeg`
    #[test]
    #[ignore]
    fn installs_ffmpeg_with_x264() {
        let ffmpeg = ensure_ffmpeg(&dir(), &mut |_| {}, &|| false).unwrap();
        let out = command(&ffmpeg).args(["-hide_banner", "-encoders"]).output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("libx264"));
    }

    /// Needs the network: every pinned file, for every platform, hashes to its pin.
    /// `cargo test -p polishcast-export -- --ignored every_pin`
    #[test]
    #[ignore]
    fn every_pin_matches_what_its_server_sends() {
        let dir = std::env::temp_dir().join(format!("polishcast-pin-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut seen = std::collections::HashSet::new();
        for tool in ["ffmpeg"] {
            for (os, arch) in PLATFORMS {
                let pin = pin_in(PINS, tool, os, arch).unwrap();
                if seen.insert(pin.url) {
                    let file = fetch_verified(pin.url, pin.sha256, &dir, &mut |_| {}, &|| false)
                        .unwrap_or_else(|e| panic!("{tool} {os} {arch}: {e}"));
                    let _ = std::fs::remove_file(file);
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
