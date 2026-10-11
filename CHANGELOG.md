# Changelog

What changed in each release of **Small Video**, the screen recorder that turns
a take into a polished video: smooth cursor, zoom on clicks, background and
rounded corners.

The public version of this page — with the download for each release — lives at
<https://mayorana.ch/en/apps/small-video/releases>. It is generated from this
file by `scripts/changelog_to_json.py`, so this file is the only place a release
note is written.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each heading is dated on the day its tag was pushed. Releases that carried only
build or packaging work say so rather than being hidden: the version numbers a
user sees in the update prompt should all be accounted for.

## [Unreleased]

### Added

- A speed for each video: **Speed** next to the play button speeds it up to
  1.25×, 1.5×, 2× or 3×. The preview plays at that speed so you can check it
  reads well, the time shown is the exported video's, and the export comes out
  at that speed. The voice is sped up with the picture and keeps its pitch;
  zooms and the cursor stay in step.

## [0.1.3] - 2026-10-09

### Added

- The version you are running now shows next to the Small Video name at the
  top of the sidebar and in the window title, so it is at hand when you report
  a problem or check whether an update installed.
- When a newer version of Small Video is out, a banner at the top of the
  window says so and **Download** fetches it for your system. **×** hides it
  until the next start.

## [0.1.2] - 2026-10-09

### Added

- Small Video Pro. The free version exports 10 different takes; exporting one
  of them again, after changing a zoom for instance, is free, and recording
  and editing are never limited. The export panel shows how many free exports
  are left, and **Get Pro…** in the sidebar is where you buy Small Video Pro
  or paste your licence key to export every take. The key is checked on your
  computer, with no account and nothing sent anywhere.

## [0.1.1] - 2026-10-09

### Added

- Record your screen from the menu bar (macOS) or the notification area
  (Windows), or with ⌘⇧2 / Ctrl+Shift+2 from any app. The display your pointer
  is on is recorded with your microphone, and Small Video's own windows are
  left out.
- The cursor is redrawn instead of recorded, so it glides smoothly and can be
  made bigger. Clicks become zooms that ease in on where you clicked.
- An editor to fine-tune the result: pick a zoom on the timeline to change when
  it starts and ends, how deep it goes and where it looks, or delete it; add
  one at the playhead with Z; and choose the frame (screen, 16:9, 9:16 or
  square), background, padding, corners, shadow, cursor size and smoothing.
  Every change can be undone and is saved as you go.
- Cut the pauses: turn it on and the silences where nothing is said are left
  out of the video, picture and sound together, with a moment kept around
  your words so nothing is clipped. Two sliders set how quiet a silence is and
  how long a pause has to be; the cuts show on the timeline, and a click puts
  one back.
- Export to MP4 at 1080p, 1440p or 4K, 30 or 60 fps, with your voice. What you
  export is exactly what the preview shows.
- Recordings take little space: on macOS each take is shrunk to about a fifth
  of its size right after you stop, with no visible loss on text.
- The tool that exports videos is downloaded and checked on first use, so there
  is nothing to install by hand.
- Remove a recording you don't need from the list with the 🗑 that appears when
  you point at it. After you confirm (Enter, or Esc to cancel) it goes to the
  system Trash, so you can still get it back; a video you exported from it is
  kept.
