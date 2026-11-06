//! The tools `polishcast mcp` offers, as `tools/list` gives them.

use super::*;

pub(super) fn tools() -> Value {
    // What a take argument is is said once, in the instructions, not in every tool.
    let take_arg = json!({ "type": "string" });
    let indices = json!({ "type": "array", "items": { "type": "integer", "minimum": 1 } });
    let zoom = json!({
        "type": "object",
        "properties": {
            "start": { "type": "number" },
            "end": { "type": "number" },
            "scale": { "type": "number", "minimum": 1, "maximum": 4, "description": "2 shows half the screen's width. Default 1.8." },
            "x": { "type": "number", "minimum": 0, "maximum": 1, "description": "What to center on, from the screen's left (0) to right (1)." },
            "y": { "type": "number", "minimum": 0, "maximum": 1, "description": "From top (0) to bottom (1)." },
        },
    });
    let mp4_sizes: Vec<u32> = RESOLUTIONS.iter().map(|(px, _)| *px).collect();
    let gif_sizes: Vec<u32> = GIF_SIZES.iter().map(|(px, _)| *px).collect();
    json!([
        {
            "name": "list_takes",
            "description": "Every take in the library, newest first: its length, the video's length after cuts \
                            and speed, its zooms, its size on disk, and the videos exported from it.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "import_video",
            "description": "Makes a take of a video made elsewhere (MP4, MOV or M4V, by its absolute path): \
                            it's copied into the library, with no cursor or clicks, so it takes a background, \
                            frame shape, cuts, speed, music and zooms set by hand, then export and \
                            send_to_publishkit as any take. Returns the new take as get_take does.",
            "inputSchema": { "type": "object", "properties": { "file": { "type": "string" } }, "required": ["file"] },
        },
        {
            "name": "get_take",
            "description": "One take in full: its zooms, its clips (and which are left out), its blank cutting, \
                            its style, speed and frame shape, its noise reduction, and how many clicks were recorded.",
            "inputSchema": { "type": "object", "properties": { "take": take_arg }, "required": ["take"] },
        },
        {
            "name": "set_zooms",
            "description": "Edits the zooms. Zooms are numbered from 1 as get_take lists them before this call; \
                            updates are applied first, then removals, then additions (which never overlap \
                            another zoom: one starting inside one is refused). suggest replaces every zoom \
                            with the ones suggested from the clicks, before anything else.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "suggest": { "type": "boolean" },
                    "update": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "zoom": { "type": "integer", "minimum": 1 },
                                "start": { "type": "number" }, "end": { "type": "number" },
                                "scale": { "type": "number" }, "x": { "type": "number" }, "y": { "type": "number" },
                            },
                            "required": ["zoom"],
                        },
                    },
                    "remove": indices,
                    "add": { "type": "array", "items": zoom },
                },
                "required": ["take"],
            },
        },
        {
            "name": "set_clips",
            "description": "Splits the recording into clips and leaves clips out of the video, or puts them back. \
                            Splits come first; clips are then numbered from 1 as they are after the splits. \
                            keep_from leaves out everything before that time, keep_until everything after.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "split_at": { "type": "array", "items": { "type": "number" } },
                    "remove": indices,
                    "restore": indices,
                    "keep_from": { "type": "number" },
                    "keep_until": { "type": "number" },
                },
                "required": ["take"],
            },
        },
        {
            "name": "set_blanks",
            "description": "Cuts the pauses where nobody speaks, or stops cutting them. The first time, the \
                            recording's sound is measured with ffmpeg (a few seconds).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "on": { "type": "boolean" },
                    "threshold_db": { "type": "number", "description": "Quieter than this is silence. Default -45." },
                    "min_pause_secs": { "type": "number", "description": "Shorter pauses stay. Default 1." },
                },
                "required": ["take", "on"],
            },
        },
        {
            "name": "set_style",
            "description": "Changes the look, the playback speed and whether background noise is taken out \
                            of the voice. Only the fields given change.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "background_color": { "type": "string", "description": "A CSS colour, e.g. #18181b." },
                    "background_gradient": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 2, "description": "Top and bottom CSS colours." },
                    "padding": { "type": "number", "minimum": 0, "maximum": 0.3, "description": "Space around the recording, as a fraction of the frame's shorter side." },
                    "radius": { "type": "number", "minimum": 0, "maximum": 0.1, "description": "Corner radius, same unit." },
                    "shadow": { "type": "boolean" },
                    "aspect": { "type": "string", "enum": ["screen", "16:9", "9:16", "1:1"] },
                    "cursor_scale": { "type": "number", "minimum": 0.5, "maximum": 4 },
                    "cursor_smoothing": { "type": "number", "minimum": 0, "maximum": 0.5, "description": "Seconds the pointer's path is averaged over." },
                    "click_effect": { "type": "boolean" },
                    "hide_idle_cursor": { "type": "boolean" },
                    "speed": { "type": "number", "enum": SPEEDS },
                    "reduce_noise": { "type": "boolean", "description": "Takes steady background noise (a fan, hum, room hiss) out of the voice at export, never out of the music." },
                    "noise_strength": { "type": "number", "minimum": 0, "maximum": 1, "description": "How strongly, from 0 (lightly) to 1. Default 0.5; giving it turns reduce_noise on." },
                },
                "required": ["take"],
            },
        },
        {
            "name": "export",
            "description": format!(
                "Writes the video next to the take's folder, as an MP4 (with sound, unless mute) or a GIF \
                 (silent, looping), named after the take, its size, frame rate, shape, speed, whether the \
                 blanks are cut, whether it is muted and the time of the export, so it never replaces an earlier one; name sets another name. Takes \
                 about as long as the video on a recent Mac; ffmpeg is downloaded first if none is installed. Sizes are pixels along the longer side: {mp4_sizes:?} for MP4, \
                 {gif_sizes:?} for GIF. The free version exports {} different takes.",
                polishcast_core::license::FREE_EXPORTS
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "format": { "type": "string", "enum": ["mp4", "gif"] },
                    "size": { "type": "integer", "description": "Default 1920 for MP4, 960 for GIF." },
                    "fps": { "type": "integer", "description": format!("{FRAME_RATES:?} for MP4 (default 30), {GIF_FRAME_RATES:?} for GIF (default 15).") },
                    "name": { "type": "string", "description": "The file's name, without extension. A file of that name is replaced." },
                    "mute": { "type": "boolean", "description": "For MP4: leaves out the recording's sound and the music. Default false." },
                    "captions": { "type": "string", "enum": ["off", "files", "burned"], "description": "For MP4, from the take's transcript (transcribe first): files writes .srt, .vtt and .chapters.txt beside the video; burned also draws the captions into the picture. Default off." },
                },
                "required": ["take", "format"],
            },
        },
        {
            "name": "export_clip",
            "description": "Writes a short clip of a take as an MP4 next to its folder, for TikTok, Reels or Shorts: \
                            start and end are seconds of the take's exported video (after its cuts and speed), such as \
                            the clip times PublishKit's Repurpose suggests. The take's own edit is left as it is; zooms, \
                            cuts and style inside the clip are kept. Vertical (9:16) with the captions drawn in by \
                            default. Give the take, or the exported video the times are from (PublishKit's \
                            repurposed_from). Attach the file to its draft with PublishKit's update_post (media).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "video": { "type": "string", "description": "An exported MP4 of the take, as an absolute path: its take is found from it." },
                    "start": { "type": "number", "description": "Seconds into the exported video." },
                    "end": { "type": "number", "description": "Seconds into the exported video." },
                    "aspect": { "type": "string", "enum": ["9:16", "1:1", "16:9", "screen"], "description": "Default 9:16." },
                    "size": { "type": "integer", "description": format!("Pixels along the longer side, {mp4_sizes:?}. Default 1920.") },
                    "captions": { "type": "string", "enum": ["off", "files", "burned"], "description": "Default burned when the take has a transcript, off otherwise." },
                    "name": { "type": "string", "description": "The file's name, without extension." },
                },
                "required": ["start", "end"],
            },
        },
        {
            "name": "transcribe",
            "description": "Transcribes the take with api0 on the Mayorana licence's AI credits (about a credit per few \
                            minutes of sound), finds its chapters, and keeps both with the take, replacing an earlier \
                            transcript. With the microphone and the computer's sound on separate tracks, each line says \
                            who speaks (You or Guest). Takes a few seconds per minute of recording.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "language": { "type": "string", "description": "A language code like en or fr, when known; default: detected." },
                },
                "required": ["take"],
            },
        },
        {
            "name": "get_transcript",
            "description": "The take's transcript: as prose (text, the default), as SRT or WebVTT captions in the \
                            exported video's time (after cuts and speed), or its chapters as a YouTube description lists them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "format": { "type": "string", "enum": ["text", "srt", "vtt", "chapters"] },
                },
                "required": ["take"],
            },
        },
        {
            "name": "set_chapters",
            "description": "Replaces the take's chapters (times in seconds of the recording, the first one at its start), \
                            or finds them again from what is said (detect), or has api0 title them (name_with_ai, on the \
                            licence's credits). YouTube needs at least 3, each 10 seconds or longer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "chapters": {
                        "type": "array",
                        "items": { "type": "object", "properties": { "start": { "type": "number" }, "title": { "type": "string" } }, "required": ["start", "title"] },
                    },
                    "detect": { "type": "boolean" },
                    "name_with_ai": { "type": "boolean" },
                },
                "required": ["take"],
            },
        },
        {
            "name": "send_to_publishkit",
            "description": "Opens PublishKit's Repurpose window with an exported video and its captions filled in, for a \
                            blog draft or outline, clips and social posts in the site's voice. The video must have been \
                            exported with captions. Nothing is written in PublishKit until the person confirms there.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "take": take_arg,
                    "file": { "type": "string", "description": "The exported MP4's path; default the take's newest export with captions." },
                },
                "required": ["take"],
            },
        },
        {
            "name": "list_sources",
            "description": "What can be recorded: the displays (main first), the windows by app, and the \
                            microphones, with the ids start_recording takes. Whether this computer can record \
                            a window or a region, and the computer's sound.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "start_recording",
            "description": format!(
                "Starts recording, as the person would from PolishCast's menu bar: the menu bar counts down, \
                 then shows the recording with Stop Recording. Answers once the first frame is written, with \
                 the recording_id, which is also the take's name. PolishCast's own windows are left out of a \
                 display's recording. Fails if a recording is already in progress, if the person turned off \
                 recording from Claude or Codex in PolishCast, or without the Screen Recording permission \
                 (the error says how to grant it); the first recording with the microphone asks for that \
                 permission. Stops by itself after max_duration_secs (default {}).",
                remote::MAX_DURATION
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "object",
                        "description": "What to record. Default: the whole display the pointer is on.",
                        "properties": {
                            "kind": { "type": "string", "enum": ["display", "window", "region"] },
                            "display_id": { "type": "integer", "description": "For a display, from list_sources. Default: the one the pointer is on." },
                            "window_id": { "type": "integer", "description": "For a window, from list_sources." },
                            "app": { "type": "string", "description": "For a window: the app's name, instead of window_id. Its window on screen is recorded." },
                            "x": { "type": "number", "description": "For a region, in points from the main display's top-left, as list_sources gives the displays." },
                            "y": { "type": "number" },
                            "width": { "type": "number" },
                            "height": { "type": "number" },
                        },
                        "required": ["kind"],
                    },
                    "microphone": { "type": "boolean", "description": "Default true." },
                    "microphone_device": { "type": "string", "description": "A microphone's device or name from list_sources. Default: the system's." },
                    "system_audio": { "type": "boolean", "description": "Also the sound apps play. Default: as set in PolishCast." },
                    "countdown_secs": { "type": "integer", "minimum": 0, "maximum": 10, "description": format!("Default {}.", remote::COUNTDOWN) },
                    "max_duration_secs": { "type": "integer", "minimum": 1, "maximum": crate::limits::MAX_LENGTH.as_secs(), "description": format!("Default {}.", remote::MAX_DURATION) },
                    "name": { "type": "string", "description": "The take's folder name. Default: the date and time it starts." },
                },
            },
        },
        {
            "name": "stop_recording",
            "description": "Stops the recording and saves it as a take, ready for set_zooms (whose suggested zooms \
                            are already in) and export. Answers once saved, which can take a few seconds: the \
                            take's name, its length, the clicks recorded and why it stopped. A recording that \
                            already stopped (the person stopped it, or max_duration_secs) gives its take. Only \
                            recordings started with start_recording can be stopped here.",
            "inputSchema": {
                "type": "object",
                "properties": { "recording_id": { "type": "string", "description": "From start_recording. Default: the one in progress." } },
            },
        },
        {
            "name": "recording_status",
            "description": "Whether PolishCast is idle, counting down, recording or saving; what, since when, who \
                            started it and its recording_id; whether recording from here is turned on; and how \
                            the last recording started here ended.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "cancel_recording",
            "description": "Stops the recording started with start_recording and moves it to the Trash, so no take \
                            is left. During the countdown, nothing was recorded.",
            "inputSchema": {
                "type": "object",
                "properties": { "recording_id": { "type": "string", "description": "Default: the one in progress." } },
            },
        },
    ])
}
