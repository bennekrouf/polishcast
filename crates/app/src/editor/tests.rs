use super::*;

fn at() -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_opt(15, 32, 10).unwrap()
}

#[test]
fn the_suggested_name_tells_exports_apart_and_dates_them() {
    let take = "2026-10-03 10.00.12";
    assert_eq!(
        export_name(take, Settings::default(), Aspect::Source, 1.0, false, at()),
        "2026-10-03 10.00.12 1080p 30fps 1x with blanks (exported 2026-10-22 15.32.10)"
    );
    let gif = Settings { format: Format::Gif, long_side: 960, fps: 15, mute: false };
    assert_eq!(
        export_name(take, gif, Aspect::Landscape16x9, 1.5, true, at()),
        "2026-10-03 10.00.12 960px 15fps 16x9 1.5x blanks cut (exported 2026-10-22 15.32.10)"
    );
    let muted = Settings { mute: true, ..Settings::default() };
    assert_eq!(
        export_name(take, muted, Aspect::Source, 1.0, false, at()),
        "2026-10-03 10.00.12 1080p 30fps 1x with blanks muted (exported 2026-10-22 15.32.10)"
    );
}

#[test]
fn a_typed_name_becomes_a_file_name() {
    assert_eq!(clean_export_name("  demo.mp4 ", Format::Mp4).as_deref(), Some("demo"));
    assert_eq!(clean_export_name("a/b: c?", Format::Gif).as_deref(), Some("a-b- c-"));
    assert_eq!(clean_export_name("trailing. ", Format::Mp4).as_deref(), Some("trailing"));
    assert_eq!(clean_export_name(" .mp4", Format::Mp4), None);
}

#[test]
fn a_takes_exports_are_found_by_its_name() {
    let base = std::env::temp_dir().join(format!("polishcast-exports-{}", std::process::id()));
    let dir = base.join("2026-10-03 10.00.12");
    std::fs::create_dir_all(&dir).unwrap();
    for f in [
        "2026-10-03 10.00.12.mp4",
        "2026-10-03 10.00.12 1080p 60fps (exported 2026-10-22 15.32.10).mp4",
        "2026-10-03 10.00.12 960px 15fps (exported 2026-10-22 15.33.00).gif",
        "2026-10-03 10.00.120.mp4",
        "2026-10-03 10.00.12 notes.txt",
    ] {
        std::fs::write(base.join(f), b"").unwrap();
    }
    let found: Vec<String> =
        exports_of(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    std::fs::remove_dir_all(&base).unwrap();
    assert_eq!(found.len(), 3, "{found:?}");
}
