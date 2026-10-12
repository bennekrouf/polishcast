# Small Video

Record your screen, a window or an area from the menu bar, get a polished video: smooth
cursor with a ring on clicks, automatic zoom on clicks, background (colour, gradient or your
own picture), padding and rounded corners. Export as MP4 or GIF. Rust + Dioxus desktop, macOS 15+ and
Windows 10 2004+ (Windows not yet run on a real machine; see Platforms).

Built on the same layout as Splitter: a pure model crate, a platform crate behind a command
thread, and a thin UI.

## Layout

```
crates/core      project model, no I/O: events, auto-zoom, camera, cursor smoothing, undo
crates/capture   recording on its own thread: ScreenCaptureKit (macOS), Windows.Graphics.Capture
                 (Windows), and a pointer logger shared by both
crates/render    frame layout shared by preview and export (Scene::frame, Scene::track),
                 and the CPU compositor that draws export frames (compose::Compositor)
crates/export    ffmpeg decode → compositor → ffmpeg encode (hardware H.264), with sound
crates/app       Dioxus desktop: menu bar ⏺/⏹, global shortcut, takes, editor
```

Dependencies point one way: `app → capture/export → render → core`.

## Run

```bash
cargo run -p small-video
```

Click Small Video's icon in the menu bar and **Start Recording** (or press **⌘⇧2**, or
**Record** in the window); the menu bar shows **● 0:12** while recording. Same again to stop.

The app icon is `crates/app/assets/icon.svg` (rendered to `icon.png` at 1024 px with
`rsvg-convert -w 1024 -h 1024 icon.svg -o icon.png`): the Splitter / GitAgent / ais-* family
look in rose. `tray.svg` is its mark alone, rendered at 44 px high to `tray.png`, which macOS
uses as a template image in the menu bar.

Each take is a folder in `~/Movies/Small Video/<date time>/`:

| File | What |
|---|---|
| `screen.mov` | The display the pointer was on (or the window, or the area chosen), H.264, **cursor hidden**, with the microphone (and the Mac's sound, if chosen: a second track, mixed at export) |
| `events.json` | Pointer positions and clicks, normalized to the display, seconds from the first frame |
| `project.json` | Edits: zooms (suggested from clicks), splits and removed clips, style, aspect |
| `background-*.png/jpg` | A background picture, copied in when chosen (a new name each time, so undo works) |

Right after a take, `screen.mov` is **compacted** in the background: ScreenCaptureKit writes
at a fixed, high bitrate (100–500 MB a minute on a Retina display), so the app re-encodes it
with the hardware HEVC encoder at constant quality 50, which is 3–5× smaller with no visible
loss on text (40 already smears it). The file is only replaced if the result is smaller. The
take is usable meanwhile, and its size shows in the list. On Windows the encoder takes a
bitrate, so recordings are written small and not compacted.

The cursor is left out of the recording on purpose: it's redrawn from `events.json`, smoothed
and scaled, and the camera follows it. A ring spreads from it at each click, and it fades out
after 1.5 s at rest and back in just before it moves (`core/src/cursor.rs`; both are style
settings). Small Video's own windows are excluded from the capture.

### What to record

**Screen**, **Window** or **Area** under Record (macOS; remembered in `settings.json` in the
app's data folder, with the system sound setting). A window is recorded with ScreenCaptureKit's
single-window filter, so covering it doesn't matter; pointer positions are normalized to where
it was when recording started, so moving it while recording puts the redrawn cursor off. An area
is picked in a see-through window over the display the pointer is on (`area_picker.rs`) and
recorded as that display with a source rectangle.

### Editor

Select a take to edit it. The preview plays the recording with the camera moves, the redrawn
cursor, background, padding, corners and shadow. Rust computes every frame
(`Scene::track`, 30 per second) and `assets/preview.js` only looks them up and interpolates as
the video plays, so the preview has no layout math of its own and matches the export.

- **Space** plays/pauses, **←/→** step 1 s (**⇧** 5 s), click or drag the timeline to seek
- **Z** or **+ Zoom** adds a zoom at the playhead, centered on the cursor
- **S** or **Split** splits at the playhead; click a clip in the strip under the zooms to remove
  it or put it back (**⌫**). **[** / **]** leave out everything before / after the playhead.
  Removed clips go on the same timeline as cut blanks, so preview, export and sound follow.
- Click a zoom to edit its start, end, zoom and focus; **⌫** deletes it, **Esc** deselects
- **⌘Z / ⌘⇧Z** undo and redo; every change is saved to `project.json`

The video is served to the web view by a small HTTP server on 127.0.0.1 (from Splitter), under
a random token, with the app's own protocol as a fallback.

### Blanks

**Cut the pauses** in the editor's Blanks panel leaves the silences out of the video. The
detection is Splitter's (`core/src/blanks.rs`, from `splitter-core`'s `detect.rs`): the sound's
RMS level per 50 ms window (measured once by ffmpeg, kept as `loudness.json`), runs quieter
than the threshold (−45 dB) and longer than the minimum pause (1 s) are cut, less 0.25 s on each
side next to speech. Cuts stay in recording time; zooms and the cursor aren't touched. The
preview jumps over them; the export drops their frames and trims the sound to match, with a
10 ms fade at each join. Clicking a cut on the timeline keeps it, and kept cuts survive changing
the settings.

### Export

**Export MP4** in the editor writes `<take name>.mp4` next to the take's folder, at 1080p,
1440p or 4K (along the longer side, in the chosen frame), 30 or 60 fps. **GIF** writes
`<take name>.gif` at 640–1280 px and 10–24 fps, without sound, looping: one palette for the
whole video (`palettegen` on what changes between frames) and only each frame's changed
rectangle stored. ffmpeg decodes the
recording, each output frame is drawn from the same `Scene::frame` as the preview, and ffmpeg
encodes them with VideoToolbox (hardware H.264, ~0.1 bit per pixel) and the recording's sound
as AAC. 12 s of 1080p60 takes about 10 s on an M-series Mac (release build).

Export needs ffmpeg. The app uses its own copy (downloaded on first use into its data folder's
`tools/`, checked against its published SHA-256, as in Splitter), or else one on the PATH, in
Homebrew's folders or in Splitter's tools folder.

### Permissions

The first recording asks for **Screen Recording** and **Microphone**. When run with
`cargo run`, macOS grants them to your terminal app; you may need to restart it after allowing.
Pointer position and clicks are polled, which needs no Accessibility permission.

## Platforms

| | macOS 15+ | Windows 10 2004+ | Linux |
|---|---|---|---|
| Recording | ScreenCaptureKit, compacted after | Windows.Graphics.Capture + Media Foundation at a set bitrate | not yet |
| Cursor hidden, pointer logged | ✓ | ✓ (`GetCursorPos`, physical pixels) | — |
| App's windows left out | capture filter | `WDA_EXCLUDEFROMCAPTURE` | — |
| Microphone | in the capture | cpal, muxed by the encoder | — |
| Mac's sound | in the capture (setting) | — | — |
| Window, area | ✓ | — (screen only) | — |
| Shortcut | ⌘⇧2 | Ctrl+Shift+2 | Ctrl+Shift+2 |
| Tray icon | template image | colour icon | colour icon |
| Export, ffmpeg | VideoToolbox; ffmpeg fetched on first use | libx264; ffmpeg fetched on first use | same |

The Windows backend follows Splitter's patterns (ffmpeg installer, no console windows, the
icon embedded by `build.rs`, a blocking `check-windows` CI job). It type-checks and passes
clippy for `x86_64-pc-windows-msvc`, but hasn't been run on Windows yet. Things to verify on
a first run: the yellow capture border is hidden (Windows 11 only), the microphone is in sync,
pointer positions line up on scaled displays, and the window really is left out.

Linux isn't supported: under Wayland, capture goes through the portal's screen picker and
other apps' pointer positions can't be read, which the redrawn cursor depends on.

## Development

- `SMALL_VIDEO_LIBRARY=/some/folder cargo run -p small-video` uses another folder of takes.
- `cargo run -p small-video-capture --example record -- /tmp/take 3` records 3 s without the
  app and prints every capture event.
- `cargo run --release -p small-video-export --example export -- <take dir> out.mp4 1920 60`
  exports without the app; `--example compact -- <screen.mov>` compacts a recording.
- The `record` example takes `SV_FPS`, `SV_CODEC` (h264/hevc) and `SV_MAX_WIDTH`.
- Screenshots of the window with `screencapture -l` show the video black (it's on a hardware
  layer); capture the window's screen region with `-R` instead.

## Releasing

Same pipeline as Splitter and the ais-* apps:

- **Release notes**: every user-visible change gets a bullet under `## [Unreleased]` in
  `CHANGELOG.md` in the same PR (rules in `CLAUDE.md`); `release-notes.yml` fails a PR that
  changes the app without one, unless it has the `no-notes` label.
- **Cutting a release**: `./scripts/release.sh` (`--dry-run` first; `0.1.0`, `--minor`,
  `--major` to choose the version) bumps the workspace version, stamps `[Unreleased]` with it,
  tags and pushes. Or Actions → Release → Run workflow.
- **`release.yml`** (on `v*` tags) builds a signed, notarized `.dmg` (Apple Silicon, macOS 15+)
  and a signed Inno Setup installer (`installer/installer.iss`), publishes them with
  `latest.json` and `releases.json` to mayorana.ch/downloads/small-video, and creates the
  GitHub Release from the changelog. No Linux build: recording isn't supported there.
- **Secrets** (Settings → Secrets and variables → Actions), each group all-or-nothing:
  - macOS signing: `MACOS_SIGNING_IDENTITY`, `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PWD`,
    `KEYCHAIN_PASSWORD`, `APP_STORE_CONNECT_KEY`, `APP_STORE_CONNECT_KEY_ID`,
    `APP_STORE_CONNECT_ISSUER_ID`. Without them the macOS build is skipped.
  - Windows signing: `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`,
    `TRUSTED_SIGNING_ENDPOINT`, `TRUSTED_SIGNING_ACCOUNT`, `TRUSTED_SIGNING_CERT_PROFILE`.
    Without them the installer ships unsigned.
  - Publishing: `DIST_SSH_KEY`, `DIST_SSH_HOST`, `DIST_SSH_USER`, `DIST_SSH_KNOWN_HOSTS`, and
    the `DIST_DEST_ROOT` variable. Required on the main repository; the server needs a
    `small-video/` folder under the root.
- The macOS bundle identifier `ch.mayorana.small-video` must never change: macOS ties the
  Screen Recording and Microphone permissions to it.
- `.githooks/pre-commit` formats staged Rust files; enable it once per clone with
  `git config core.hooksPath .githooks`.

## Tests

```bash
cargo test --workspace
```

`core` and `render` are pure and unit-tested (zoom suggestion, camera easing, cursor smoothing,
layout, compositing pixel checks). `export`'s tests run a generated take through ffmpeg end to
end; they pass without checking anything when ffmpeg isn't installed.

## Roadmap

1. ~~Record: menu bar Record/Stop, events log, take folder~~
2. ~~Editor: preview, timeline, zoom editing, style panel, undo, split and remove clips,
   picture backgrounds, click rings, idle cursor~~; next: drag zoom edges on the timeline,
   click the preview to set a zoom's focus
3. ~~Export: compositor, VideoToolbox encode, 1080p/1440p/4K, 30/60 fps, GIF~~; next: faster
   (compose frames in parallel, or on the GPU)
4. ~~System audio toggle, window/area capture (macOS)~~; next: webcam bubble, captions,
   keystroke overlay, window/area capture on Windows

## Licence

Source-available under the [PolyForm Noncommercial License 1.0.0](LICENSE), like Splitter.

- **Free** for personal use, learning, research and hobby projects, and for
  charities, schools, universities and government institutions.
- **Commercial use requires a licence** — including a solo consultant using it
  on client work, and an employee using it at their job. Get in touch at
  <https://mayorana.ch/en/contact>.

This is deliberately not an OSI-approved open source licence: the source is
public and readable, but companies using it for work need a commercial licence.

The name, logo and icons are trademarks and are not covered by that licence —
fork it and rebrand it. See [TRADEMARK.md](TRADEMARK.md).
