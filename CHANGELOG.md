# Changelog

What changed in each release of **Polishcast**, the screen recorder that turns
a take into a polished video: smooth cursor, zoom on clicks, background and
rounded corners.

The public version of this page — with the download for each release — lives at
<https://mayorana.ch/en/apps/polishcast/releases>. It is generated from this
file by `scripts/changelog_to_json.py`, so this file is the only place a release
note is written.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each heading is dated on the day its tag was pushed. Releases that carried only
build or packaging work say so rather than being hidden: the version numbers a
user sees in the update prompt should all be accounted for.

## [Unreleased]

### Fixed

- A take that macOS was slow to finish saving, such as a long window
  recording, no longer opens half-written with a "Compacting failed … moov atom
  not found" message. Polishcast now waits until the file is complete before
  opening and shrinking it.
## [0.1.10] - 2026-10-14

### Added

- Added support for recording system audio and specific windows on macOS. This
  allows users to capture audio from their system and record specific windows,
  enhancing the flexibility and utility of the application.

## [0.1.9] - 2026-10-14

### Changed

- Clicking a selected zoom on the timeline again deselects it, so the panel
  goes back to the style settings without having to press **Done**.

## [0.1.8] - 2026-10-14

### Changed

- Small Video is now called Polishcast. Your takes move from `Movies/Small Video`
  to `Movies/Polishcast` (`Videos\Polishcast` on Windows) the first time it
  starts; your Pro licence and settings carry over, and macOS keeps the Screen
  Recording permission you gave it. On macOS the update installs as
  Polishcast.app; once it runs, you can delete Small Video.app from
  Applications.

## [0.1.7] - 2026-10-13

### Added

- A recording you forget to stop now stops by itself after 2 hours, at 20 GB, or
  when less than 2 GB is left on the disk, whichever comes first, instead of
  filling the disk. The take is kept, and the sidebar says why it stopped.

## [0.1.6] - 2026-10-12

### Added

- Record one window or an area of the screen, not only the whole screen:
  choose **Screen**, **Window** or **Area** under Record. A window is recorded
  on its own even when others cover it; for an area, drag across the part to
  record when you start (Esc cancels). macOS only for now.
- **Record the Mac's sound too**, under Record, puts the sound apps play into
  the take along with your voice. The setting and what to record are
  remembered for next time.
- A ring spreads from the cursor at each click, and the cursor fades out while
  you leave the mouse alone, then back in just before it moves. Both are on for
  new takes and can be turned off under **Style** (**Ring on clicks**, **Hide
  the cursor when it rests**).
- Cut by hand: **Split** (or S) splits the video at the playhead, and clicking a
  clip in the strip under the zooms lets you remove it or put it back.
  **[** leaves out everything before the playhead and **]** everything after,
  to trim the start and end in one key. Undo brings everything back.
- Your own picture as the background: the **+** swatch under **Style** takes a
  PNG or JPEG. It fills the frame without being stretched, and is copied into
  the take so moving or deleting the original doesn't affect it.
- Export as an animated GIF: choose **GIF** in the export panel, at 640, 960 or
  1280 px and 10, 15 or 24 fps. It has no sound and loops, for pasting into a
  page, a ticket or a chat.
- A recording you forget to stop now stops by itself after 2 hours, at 20 GB,
  or when less than 2 GB is left on the disk, whichever comes first, instead of
  filling the disk. The take is kept, and the sidebar says why it stopped.
- The recordings panel on the left can be closed with **‹** to give the editor
  the whole window; a thin strip keeps Record and **›** to open it again.

## [0.1.5] - 2026-10-11

### Added

- When you click the Polishcast icon in the Dock, Finder, or Launchpad while
  the window is closed, the app now opens and focuses the window. This makes it
  easier to access the app quickly.

## [0.1.4] - 2026-10-11

### Added

- A speed for each video: **Speed** next to the play button speeds it up to
  1.25×, 1.5×, 2× or 3×. The preview plays at that speed so you can check it
  reads well, the time shown is the exported video's, and the export comes out
  at that speed. The voice is sped up with the picture and keeps its pitch;
  zooms and the cursor stay in step.

## [0.1.3] - 2026-10-09

### Added

- The version you are running now shows next to the Polishcast name at the
  top of the sidebar and in the window title, so it is at hand when you report
  a problem or check whether an update installed.
- When a newer version of Polishcast is out, a banner at the top of the
  window says so and **Download** fetches it for your system. **×** hides it
  until the next start.

## [0.1.2] - 2026-10-09

### Added

- Polishcast Pro. The free version exports 10 different takes; exporting one
  of them again, after changing a zoom for instance, is free, and recording
  and editing are never limited. The export panel shows how many free exports
  are left, and **Get Pro…** in the sidebar is where you buy Polishcast Pro
  or paste your licence key to export every take. The key is checked on your
  computer, with no account and nothing sent anywhere.

## [0.1.1] - 2026-10-09

### Added

- Record your screen from the menu bar (macOS) or the notification area
  (Windows), or with ⌘⇧2 / Ctrl+Shift+2 from any app. The display your pointer
  is on is recorded with your microphone, and Polishcast's own windows are
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
