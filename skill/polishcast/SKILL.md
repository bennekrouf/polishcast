---
name: polishcast
description: How to make polished screen-recording videos with PolishCast, the desktop app that adds automatic zoom on clicks, a smooth cursor, a background, padding and rounded corners, and exports MP4 or GIF. Read this skill before calling any polishcast tool (list_takes, get_take, set_zooms, set_clips, set_blanks, set_style, export, transcribe, get_transcript, set_chapters, send_to_publishkit, list_sources, start_recording, stop_recording), because it has the recording safety rules, good style defaults for demos and social clips, and how to report a take. Use it whenever the user mentions PolishCast or a take, or asks to record a demo, walkthrough or bug repro of an app on screen, change a recording's zooms, trim it, cut its pauses, change its background, aspect ratio, cursor or speed, remove background noise, transcribe it, add captions or chapters, export it as MP4 or GIF, or send it to PublishKit for a blog post and social posts.
---

# PolishCast

PolishCast is a desktop app (macOS 15+, Windows 10 2004+) that records the
screen, a window or an area, then makes the recording presentable: the camera
zooms in where the person clicked, the cursor is redrawn smooth with a ring on
clicks, and the recording sits on a background with padding, rounded corners
and a shadow. Each recording is a **take**, named by its folder.

The person you are helping wants a short, clean video of software: a product
demo, a feature for release notes, a tutorial, a bug report.

## Before you start

The tools come from PolishCast's own MCP server (`polishcast mcp`, over
stdio). If no tool named `list_takes` from a `polishcast` server is available,
the app is not connected:

- Tell the user PolishCast is a separate desktop app, from
  <https://mayorana.ch/en/apps/polishcast>, and that this skill only works with
  it installed. Do not download or install it for them.
- Once installed, the **MCP** button next to *Get Pro…* shows the exact
  config for Claude Desktop and the command for Claude Code and Codex.
  `references/connect.md` has the steps per client.

## Recording

- `list_sources` gives the ids of the screens, displays and windows.
- `start_recording` records after a countdown (3 s by default) until
  `stop_recording`, or until `max_duration_secs` (10 minutes by default).
  The menu bar shows the recording, and the person can stop it there at any
  time. `stop_recording` saves the take with zooms suggested from its clicks.
- `recording_status` says what is being recorded; `cancel_recording` throws
  the recording away (it goes to the Trash).
- Recording is the user's screen: always say what you are about to record
  (which window or screen, roughly how long) before starting, and warn them
  that anything on it (notifications, other windows, personal data) ends up in
  the video.
- If the user turned off *Let Claude or Codex record the screen* in the MCP
  window, the recording tools say so; tell them where to turn it back on and
  don't try to get around it.

**A demo, end to end:** `start_recording` on the app's window, drive the app
through the steps being shown (with whatever browser or computer-use tools you
have), `stop_recording`, `set_zooms` with `suggest: true`, check the take with
`get_take`, then `export`. Move the pointer and click deliberately: zooms
follow clicks, so a click on the thing being shown makes a good zoom.

## Editing a take

`list_takes` lists takes, newest first; `get_take` shows a take's zooms,
clips, cuts, style and length. Times are seconds of the recording.

- **Zooms**: `set_zooms` updates, removes or adds zooms (numbered from 1 as
  `get_take` listed them), or suggests them again from the clicks.
- **Cutting**: `set_clips` splits the recording and leaves parts out
  (`keep_from`, `keep_until` for trimming the ends). `set_blanks` cuts the
  pauses where nobody speaks (`threshold_db`, `min_pause_secs`).
- **Look**: `set_style` sets the background (colour or two-colour gradient),
  padding, corner radius, shadow, aspect (`screen`, `16:9`, `9:16`, `1:1`),
  cursor size and smoothing, click ring, hiding an idle cursor, playback
  speed, and background-noise reduction on the voice.

Sensible defaults for a product demo: 16:9, a dark gradient, padding around
0.06, radius around 0.02, shadow on, click ring on, blanks cut. For a social
clip, 9:16 or 1:1.

Changes land in the take at once, and an open editor shows them as one undo
step, so the user can always undo in the window.

## Exporting

`export` writes an MP4 or a GIF next to the take's folder (`size`, `fps`,
`name`, `mute`). A file of the same name is replaced, so ask before reusing a
name. The free version exports 10 different takes; after that, the user needs
PolishCast Pro. Export uses ffmpeg, which the app fetches itself on first use;
the first export can take a little longer.

## Transcript, captions and chapters

`transcribe` writes the take's transcript with api0 on the Mayorana licence's
AI credits: say so before the first one, and that the sound is sent to api0 in
the region set in PolishCast. It finds chapters too. With the microphone and
the computer's sound on separate tracks, lines say who speaks (You or Guest).
`get_transcript` reads it as text, SRT, VTT or YouTube chapters (those three
in the exported video's time, after cuts and speed). `set_chapters` replaces
them, finds them again (`detect`) or has api0 title them (`name_with_ai`,
also on credits). `export` with `captions: files` writes `.srt`, `.vtt` and
`.chapters.txt` beside the MP4; `burned` also draws the captions into the
picture. `send_to_publishkit` opens PublishKit's Repurpose with an export that
has captions; nothing is written there until the user confirms in PublishKit.
`export_clip` writes one clip of a take as an MP4, vertical with burned
captions by default, without changing the take: `start` and `end` are seconds
of an exported video (pass that export as `video`, as PublishKit's Repurpose
drafts name it in `repurposed_from`). The media library tools (`list_media`,
`calendar`, `plan_media`, `fulfil_plan`, `set_media_*`) are the same as in
PublishKit and TrackCut.

## How to report back

Before recording, one line: what will be recorded (window or screen), the
maximum length, and that the countdown starts now.

For a take, give its name and length, then:

- **Zooms**: how many, each as `start–end ×scale` in m:ss.
- **Cuts**: the clips left out and the pauses cut, with the time they save.
- **Look**: aspect, background, padding, speed, noise reduction.
- **Length after cuts**, at the chosen speed.

After an export, give the file's full path, its format, size in pixels, frame
rate and length, and how many free exports remain if the tool says so.
