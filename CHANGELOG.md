# Changelog

What changed in each release of **PolishCast**, the screen recorder that turns
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

## [0.1.29] - 2026-10-27

### Changed

- Build and packaging only — no user-visible change.

## [0.1.28] - 2026-10-26

### Fixed

- On Windows and Linux, opening PolishCast while it already runs in the
  notification area brings its window back, instead of starting another,
  hidden copy each time. Only one PolishCast runs at a time, so its recording
  shortcut and notification-area icon always belong to the one you see.

## [0.1.27] - 2026-10-25

### Fixed

- Installing silently, as IT does for an unattended update with `/VERYSILENT`,
  no longer hangs when PolishCast is already installed: setup waited on a
  question nobody could see. It now replaces the installed version without
  asking.

## [0.1.26] - 2026-10-25

### Changed

- Activating a licence now shows a small spinner while it is checked, then
  a window that says you're Pro, with the AI credits left and the renewal
  date in small type below. Removing the licence and checking it again no
  longer sit beside it; Manage subscription is a small link. Press Esc or
  the × to close it.
- Text in buttons, fields and the rest of the window is now set in the same
  monospaced typeface as GitAgent and the other Mayorana apps.
- The new-version banner and the usage-statistics notice now look the same
  as in GitAgent: a blue bar across the top, or the bottom for the notice,
  with a Download link and a × to hide it.
- Every action button is now 30px tall, borders included, like the
  regular buttons beside it.

## [0.1.25] - 2026-10-23

### Changed

- Dialogs now have a title bar with a × to close them, and text fields have
  the same size and typeface as in GitAgent.
- An open dialog now dims the light/dark switch in the corner along with the
  rest of the window, instead of leaving it lit on top.
- The buttons along the top of the window now all have the same height,
  borders included, and the light/dark switch matches them.

## [0.1.24] - 2026-10-23

### Changed

- Buttons now have the same size, rounded corners and typeface as in
  GitAgent, so they look alike across the Mayorana apps.

## [0.1.23] - 2026-10-23

### Fixed

- On company networks that inspect secure connections (Zscaler and similar
  proxies), activating PolishCast Pro failed with "invalid peer certificate:
  UnknownIssuer". PolishCast now trusts the certificates your computer trusts,
  as your browser does, so the licence check, update check and ffmpeg download
  work there too. If the proxy's certificate isn't installed on the computer,
  the message now says so and suggests asking IT to install it.
- When ffmpeg can't be downloaded (offline, or blocked by a company network),
  exporting and the sound tools no longer stop at an error. PolishCast shows
  the one command that installs ffmpeg on your computer, ready to copy
  (winget on Windows, Homebrew on a Mac, apt on Linux), and a link to download
  it by hand, then Try again finds it. On Windows it now also finds an ffmpeg
  installed with winget, Scoop or Chocolatey without restarting PolishCast.

## [0.1.22] - 2026-10-22

### Changed

- A new take no longer starts with the pointer leaving the Record button, or
  ends with it heading to Stop and the click on it. Those few seconds are set
  aside as removed clips, so the take opens ready to use; put them back in
  the timeline if you want them. Recordings started and stopped with the
  keyboard shortcut are kept whole.
- Export suggests 30 fps for MP4 instead of 60: smaller files that look the
  same for most screen recordings. 60 fps is still one click away.
- Exporting under a name that is already taken now stops and asks whether to
  replace that video, instead of replacing it straight away.

### Fixed

- The window list under Window keeps itself up to date while PolishCast is
  open: windows you open or close show up or go away within a couple of
  seconds, without pressing ↻. It also lists windows on another Space and
  those of apps in full screen, which were missing.

## [0.1.21] - 2026-10-22

### Changed

- Export now asks what to call the video before it starts, and suggests a name
  that says what it is: the take, its size, frame rate, shape and speed,
  whether the pauses are cut, and when it was exported. Exporting the same
  take again no longer replaces the video you exported before; if you pick a
  name that is already taken, the window says so before anything is
  overwritten.

- One licence number for every Mayorana app on this computer. Paste it in any
  of them and the others are Pro too the next time they start, with nothing to
  paste again; a number already pasted in one app is shared the same way.
  Removing the licence in the Pro window removes it from all of them.

## [0.1.20] - 2026-10-21

### Added

- Free AI credits every month, without Pro. Enter your email under Get free
  AI credits in the Pro window and the licence number arrives by email; paste
  it in the same window. The window then shows how many credits are left this
  month. The free version's limits stay as they are.

## [0.1.19] - 2026-10-21

### Changed

- PolishCast Pro now comes with a monthly plan on mayorana.ch instead of a
  one-time licence key. Paste the licence number from the plan's email (like
  ABCDE-FGHJK-LMNPQ-RSTUV) in the Pro window. PolishCast checks it when it
  starts and once a day, and Pro keeps working for a week without a
  connection.
- The Pro window shows your plan, the AI credits left this month and when the
  plan renews. Manage subscription opens the page where you change or cancel
  it.

## [0.1.18] - 2026-10-20

### Added

- A Feedback button next to MCP in the sidebar opens the contact form with
  PolishCast and your version already filled in, so an idea or a problem
  reaches us in two clicks.

## [0.1.17] - 2026-10-19

### Changed

- Build and packaging only — no user-visible change.

## [0.1.16] - 2026-10-18

### Added

- Use PolishCast from Claude. The new **MCP** button next to Get Pro… shows
  what to paste into Claude Desktop, or the one line to run for Claude Code,
  already pointing at where PolishCast is installed. Claude can then list your
  takes, add, move and remove zooms (or suggest them again from your clicks),
  cut the pauses, trim and leave out clips, change the background, the frame's
  shape and the speed, and export an MP4 or a GIF. A take open in the editor
  shows Claude's changes within a couple of seconds, and ⌘Z takes them back.
  Recording stays yours, from the menu bar.

## [0.1.15] - 2026-10-16

### Added

- Light mode. PolishCast now matches your computer's light or dark setting
  and follows it when it changes. The sun/moon button at the top right switches
  between the two. The video preview stays black in both.

### Changed

- The name is now written PolishCast. Your takes folder follows: `Movies/Polishcast`
  is renamed `Movies/PolishCast` the first time it starts, with everything in it.
  On macOS the update installs as PolishCast.app; once it runs, you can delete
  Polishcast.app from Applications.

## [0.1.14] - 2026-10-15

### Added

- The recordings panel on the left can be made wider or narrower: drag its
  right edge. The width is remembered; double-click the edge to go back to the
  usual width.
- Music under your video. **+ Music** starts a song at the playhead; while the
  preview plays, press **M** to stop it and **M** again to bring it back, right
  where you want it. Each song starts and stops at moments of the recording, so
  it stays with them when you cut pauses, remove clips or change the speed, and
  it always plays at its own speed. Click a song on the timeline to set its
  volume, fades and where in the song it begins, or to swap the song. Turn on
  **Lower the music while I speak** and it dips under your voice, in the preview
  as in the export.

### Fixed

- Exporting at a faster speed made the picture twice as slow instead of faster
  while the sound was sped up, so the two drifted apart. The picture now plays
  at the speed you chose.

## [0.1.13] - 2026-10-15

### Fixed

- Recording one window with the Mac's sound on now includes the sound of every
  app, not only the recorded window's. Music playing in a browser behind the
  window used to be missing, and only came through faintly via the microphone.

## [0.1.12] - 2026-10-14

### Added

- On a Mac that still has Small Video.app, the app under its old name,
  PolishCast offers to quit it and move it to the Trash. Opening it by mistake
  from the Dock or Spotlight started the old version, which kept saying an
  update was available even after you had installed it.

## [0.1.11] - 2026-10-14

### Fixed

- A take that macOS was slow to finish saving, such as a long window
  recording, no longer opens half-written with a "Compacting failed … moov atom
  not found" message. PolishCast now waits until the file is complete before
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

- Small Video is now called PolishCast. Your takes move from `Movies/Small Video`
  to `Movies/PolishCast` (`Videos\PolishCast` on Windows) the first time it
  starts; your Pro licence and settings carry over, and macOS keeps the Screen
  Recording permission you gave it. On macOS the update installs as
  PolishCast.app; once it runs, you can delete Small Video.app from
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

- When you click the PolishCast icon in the Dock, Finder, or Launchpad while
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

- The version you are running now shows next to the PolishCast name at the
  top of the sidebar and in the window title, so it is at hand when you report
  a problem or check whether an update installed.
- When a newer version of PolishCast is out, a banner at the top of the
  window says so and **Download** fetches it for your system. **×** hides it
  until the next start.

## [0.1.2] - 2026-10-09

### Added

- PolishCast Pro. The free version exports 10 different takes; exporting one
  of them again, after changing a zoom for instance, is free, and recording
  and editing are never limited. The export panel shows how many free exports
  are left, and **Get Pro…** in the sidebar is where you buy PolishCast Pro
  or paste your licence key to export every take. The key is checked on your
  computer, with no account and nothing sent anywhere.

## [0.1.1] - 2026-10-09

### Added

- Record your screen from the menu bar (macOS) or the notification area
  (Windows), or with ⌘⇧2 / Ctrl+Shift+2 from any app. The display your pointer
  is on is recorded with your microphone, and PolishCast's own windows are
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
