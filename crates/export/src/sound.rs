//! The recording's sound: its loudness over time (for blank cutting), and the ffmpeg filter
//! that leaves the cut stretches out of it at export.

use crate::tools;
use anyhow::{bail, Context, Result};
use polishcast_core::music::{self, Placed};
use polishcast_core::{Loudness, Timeline};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

/// TrackCut's loudness window: 50 ms.
const WINDOW_SECS: f64 = 0.05;
const RATE: u32 = 16_000;

/// The sound, timed from the start of the recording: the microphone may start a moment after
/// the picture, and the gap is filled with silence so times line up with the video.
const ALIGN: &str = "aresample=async=1:first_pts=0";

/// How many sound tracks `file` has: none, one (the microphone, or the Mac's sound), or more
/// when both were recorded.
pub fn audio_tracks(ffmpeg: &Path, file: &Path) -> Result<usize> {
    let out = tools::command(ffmpeg).args(["-hide_banner", "-nostdin", "-i"]).arg(file).output()?;
    // ffmpeg describes the input on stderr (and complains that no output was given).
    Ok(String::from_utf8_lossy(&out.stderr).lines().filter(|l| l.contains("Stream #") && l.contains("Audio:")).count())
}

/// Whether `file` has a sound track.
pub fn has_audio(ffmpeg: &Path, file: &Path) -> Result<bool> {
    Ok(audio_tracks(ffmpeg, file)? > 0)
}

/// The start of a filter chain: input `input`'s sound, its `tracks` tracks aligned and mixed
/// into one. Further filters follow after a comma.
fn source(input: u8, tracks: usize) -> String {
    if tracks <= 1 {
        return format!("[{input}:a:0]{ALIGN}");
    }
    let mut graph = String::new();
    for i in 0..tracks {
        graph += &format!("[{input}:a:{i}]{ALIGN}[m{i}];");
    }
    for i in 0..tracks {
        graph += &format!("[m{i}]");
    }
    // normalize=0: each track at its own level, not each halved.
    graph + &format!("amix=inputs={tracks}:duration=longest:normalize=0")
}

/// Steady background noise (a fan, hum, room hiss) taken out of the voice: `strength` from 0
/// to 1, and `floor`, the recording's noise level in dB ([`Loudness::noise_floor`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Denoise {
    pub strength: f32,
    pub floor: f32,
}

impl Denoise {
    /// The filters, to follow a chain after a comma: the hum and rumble below 80 Hz cut, then
    /// ffmpeg's `afftdn` lowering what sits at the noise's level by 6 to 30 dB. `afftdn` only
    /// lowers it fully when told a level above the noise as RMS measures it (by about 20 dB):
    /// told the level itself, it takes out a few dB whatever the strength, and told much more,
    /// it starts to eat quiet words.
    fn filters(self) -> String {
        let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
        let reduce = 6.0 + 24.0 * finite(self.strength, 0.0).clamp(0.0, 1.0);
        let floor = finite(self.floor, -60.0);
        format!("highpass=f=80,afftdn=nr={reduce:.1}:nf={:.1}", (floor + 22.0).clamp(-80.0, -20.0))
    }
}

/// [`source`], with the background noise taken out when `denoise` says so.
fn voice(input: u8, tracks: usize, denoise: Option<Denoise>) -> String {
    match denoise {
        Some(d) => format!("{},{}", source(input, tracks), d.filters()),
        None => source(input, tracks),
    }
}

/// The graph mixing `sound` (input 1), which starts `offset` seconds into the recording, into
/// the recording's `tracks` sound tracks (input 0), producing `[a]`.
fn mix_graph(tracks: usize, offset: f64) -> String {
    let shift = if offset > 0.0 {
        format!(",adelay=delays={}:all=1", (offset * 1000.0).round() as u64)
    } else if offset < 0.0 {
        format!(",atrim=start={:.6},asetpts=PTS-STARTPTS", -offset)
    } else {
        String::new()
    };
    let sound = format!("[1:a:0]{ALIGN}{shift}");
    match tracks {
        // Padded: the picture sets the length.
        0 => format!("{sound},apad[a]"),
        _ => format!("{}[m];{sound}[s];[m][s]amix=inputs=2:duration=first:normalize=0[a]", source(0, tracks)),
    }
}

/// Mixes `sound`, the sound apps played recorded apart and starting `offset` seconds into the
/// recording, into `file`'s sound, in place; the picture is copied as is. `sound` is deleted
/// once it's in.
pub fn mix_in(file: &Path, sound: &Path, offset: f64) -> Result<()> {
    let ffmpeg = crate::ensure_ffmpeg(&mut |_| {}, &|| false)?;
    let tracks = audio_tracks(&ffmpeg, file)?;
    let tmp = file.with_file_name(".mixing.mov");
    let out = tools::command(&ffmpeg)
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(file)
        .arg("-i")
        .arg(sound)
        .args(["-filter_complex", &mix_graph(tracks, offset), "-map", "0:v:0", "-map", "[a]"])
        .args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest"])
        .arg(&tmp)
        .output()
        .context("Starting ffmpeg")?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("Adding the Mac's sound failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    std::fs::rename(&tmp, file)?;
    let _ = std::fs::remove_file(sound);
    Ok(())
}

/// The loudness of `file`'s sound in 50 ms windows; `None` if it has no sound.
pub fn loudness(ffmpeg: &Path, file: &Path) -> Result<Option<Loudness>> {
    let tracks = audio_tracks(ffmpeg, file)?;
    if tracks == 0 {
        return Ok(None);
    }
    let graph = format!("{}[out]", source(0, tracks));
    let mut child = tools::command(ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(["-filter_complex", &graph, "-map", "[out]"])
        .args(["-ac", "1", "-ar", &RATE.to_string(), "-f", "f32le", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Starting ffmpeg")?;
    let mut bytes = Vec::new();
    child.stdout.take().context("ffmpeg output")?.read_to_end(&mut bytes)?;
    let mut errors = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut errors);
    }
    if !child.wait()?.success() {
        bail!("Reading the sound failed: {}", errors.trim());
    }
    let per_window = (RATE as f64 * WINDOW_SECS) as usize;
    let samples: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    let db = samples
        .chunks(per_window)
        .map(|w| {
            let rms = (w.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / w.len() as f64).sqrt();
            (20.0 * rms.max(1e-9).log10()) as f32
        })
        .collect();
    Ok(Some(Loudness { window: WINDOW_SECS, db }))
}

/// ffmpeg's `atempo` speeds sound up without raising its pitch, by at most 2× per filter, so a
/// faster speed is a chain: 3× is 2× then 1.5×.
pub fn atempo(speed: f64) -> String {
    let mut left = speed;
    let mut steps = Vec::new();
    while left > 2.0 + 1e-9 {
        steps.push("atempo=2".to_string());
        left /= 2.0;
    }
    while left < 0.5 - 1e-9 {
        steps.push("atempo=0.5".to_string());
        left /= 0.5;
    }
    steps.push(format!("atempo={left:.6}"));
    steps.join(",")
}

/// The filter graph for the sound, taking input 1's `tracks` tracks and producing `[aout]`:
/// mixed into one, cleaned of background noise with `denoise`, the timeline's parts when
/// anything is cut, sped up when `speed` isn't 1. `None` when the one track is used as is.
pub fn filter(
    timeline: &Timeline,
    cutting: bool,
    speed: f64,
    tracks: usize,
    denoise: Option<Denoise>,
) -> Option<String> {
    let fast = (speed - 1.0).abs() > 1e-9;
    match (cutting, fast) {
        (false, false) if tracks <= 1 && denoise.is_none() => None,
        (false, false) => Some(format!("{}[aout]", voice(1, tracks, denoise))),
        (true, false) => Some(cut_filter(timeline, tracks, denoise)),
        (false, true) => Some(format!("{},{}[aout]", voice(1, tracks, denoise), atempo(speed))),
        (true, true) => {
            let cut = cut_filter(timeline, tracks, denoise);
            let cut = cut.strip_suffix("[aout]").unwrap_or(&cut);
            Some(format!("{cut}[acut];[acut]{}[aout]", atempo(speed)))
        }
    }
}

/// The music's volume over video time `t`, as an ffmpeg expression: `level` of it while the
/// voice speaks (`speaking`), ramping as [`music::ducked`] does. Only the stretches inside
/// `music` count.
fn duck_expr(level: f32, speaking: &[(f64, f64)], music: &[Placed]) -> String {
    let r = music::DUCK_RAMP;
    let terms: Vec<String> = speaking
        .iter()
        .filter(|&&(s, e)| music.iter().any(|m| s - r < m.until && e + r > m.at))
        .map(|&(s, e)| format!("clip(min((t-{:.4})/{r},({:.4}-t)/{r}),0,1)", s - r, e + r))
        .collect();
    if terms.is_empty() {
        return "1".into();
    }
    // The stretches are apart enough that their ramps never overlap: the sum is at most 1.
    format!("1-{:.4}*({})", 1.0 - level, terms.join("+"))
}

/// Adds music under the sound: `voice` is the graph [`filter`] made (`None` when it's used
/// as is), `tracks` the recording's sound tracks (input 1), and piece `k` of `music` is input
/// `2 + k`. With `duck`, the music is lowered to that fraction where the voice speaks.
/// Produces `[aout]`, `duration` seconds long; `None` without music.
pub fn with_music(
    voice: Option<String>,
    tracks: usize,
    music: &[Placed],
    duck: Option<(f32, &[(f64, f64)])>,
    duration: f64,
) -> Option<String> {
    if music.is_empty() {
        return voice;
    }
    let mut graph = match voice {
        Some(g) => format!("{}[voice];", g.strip_suffix("[aout]").unwrap_or(&g)),
        None if tracks > 0 => format!("{}[voice];", source(1, tracks)),
        None => String::new(),
    };
    for (k, m) in music.iter().enumerate() {
        let len = m.until - m.at;
        // A fade of 0 s would be ffmpeg's default length, not none.
        let fade_in = if m.fade_in > 0.0 { format!(",afade=t=in:d={:.6}", m.fade_in) } else { String::new() };
        let fade_out = if m.fade_out > 0.0 {
            format!(",afade=t=out:st={:.6}:d={:.6}", (len - m.fade_out).max(0.0), m.fade_out)
        } else {
            String::new()
        };
        graph += &format!(
            "[{}:a:0]atrim=start={:.6}:duration={len:.6},asetpts=PTS-STARTPTS{fade_in}{fade_out},\
             volume={:.4},adelay=delays={}:all=1[m{k}];",
            2 + k,
            m.from,
            m.volume,
            (m.at * 1000.0).round() as u64,
        );
    }
    for k in 0..music.len() {
        graph += &format!("[m{k}]");
    }
    graph += &format!("amix=inputs={}:duration=longest:normalize=0", music.len());
    if let Some((level, speaking)) = duck {
        graph += &format!(",volume='{}':eval=frame", duck_expr(level, speaking, music));
    }
    // The video sets the length: the voice is as long, music alone is padded or cut to it.
    let end = format!("apad,atrim=end={duration:.6}[aout]");
    Some(if tracks > 0 {
        graph + &format!("[music];[voice][music]amix=inputs=2:duration=first:normalize=0,{end}")
    } else {
        graph + &format!(",{end}")
    })
}

/// Seconds faded out and in at each cut, so a cut never clicks.
const FADE_SECS: f64 = 0.01;

/// An ffmpeg filter graph taking input 1's sound (its `tracks` tracks mixed, cleaned with
/// `denoise`) and producing `[aout]`: only the timeline's parts, joined, each faded in and out
/// over a few milliseconds. The noise is taken out before cutting, so each part starts with
/// the noise already measured.
pub fn cut_filter(timeline: &Timeline, tracks: usize, denoise: Option<Denoise>) -> String {
    let n = timeline.parts.len();
    let mut graph = format!("{},asplit={n}", voice(1, tracks, denoise));
    for i in 0..n {
        graph += &format!("[s{i}]");
    }
    for (i, &(start, end)) in timeline.parts.iter().enumerate() {
        let fade = FADE_SECS.min((end - start) / 2.0);
        graph += &format!(
            ";[s{i}]atrim=start={start:.6}:end={end:.6},asetpts=PTS-STARTPTS,\
             afade=t=in:d={fade:.6},afade=t=out:st={:.6}:d={fade:.6}[a{i}]",
            (end - start - fade).max(0.0)
        );
    }
    graph += ";";
    for i in 0..n {
        graph += &format!("[a{i}]");
    }
    graph + &format!("concat=n={n}:v=0:a=1[aout]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeding_up_keeps_the_pitch_in_steps_of_at_most_two() {
        assert_eq!(atempo(1.5), "atempo=1.500000");
        assert_eq!(atempo(2.0), "atempo=2.000000");
        assert_eq!(atempo(3.0), "atempo=2,atempo=1.500000");
        let tl = Timeline { parts: vec![(0.0, 10.0)] };
        assert_eq!(filter(&tl, false, 1.0, 1, None), None);
        assert_eq!(
            filter(&tl, false, 2.0, 1, None).unwrap(),
            "[1:a:0]aresample=async=1:first_pts=0,atempo=2.000000[aout]"
        );
        let both = filter(&Timeline { parts: vec![(1.0, 4.0), (6.0, 10.0)] }, true, 1.25, 1, None).unwrap();
        assert!(both.contains("concat=n=2:v=0:a=1[acut];[acut]atempo=1.250000[aout]"), "{both}");
    }

    #[test]
    fn the_microphone_and_the_macs_sound_are_mixed() {
        let tl = Timeline { parts: vec![(0.0, 10.0)] };
        let mixed = "[1:a:0]aresample=async=1:first_pts=0[m0];[1:a:1]aresample=async=1:first_pts=0[m1];\
                     [m0][m1]amix=inputs=2:duration=longest:normalize=0";
        assert_eq!(filter(&tl, false, 1.0, 2, None).unwrap(), format!("{mixed}[aout]"));
        let cut = cut_filter(&Timeline { parts: vec![(1.0, 4.0), (6.0, 10.0)] }, 2, None);
        assert!(cut.starts_with(&format!("{mixed},asplit=2[s0][s1];")), "{cut}");
    }

    #[test]
    fn music_goes_under_the_voice_where_it_plays() {
        let m = Placed { index: 0, at: 2.0, until: 6.0, from: 10.0, volume: 0.3, fade_in: 1.0, fade_out: 1.0 };
        let g = with_music(None, 1, &[m], None, 8.0).unwrap();
        assert!(
            g.starts_with("[1:a:0]aresample=async=1:first_pts=0[voice];[2:a:0]atrim=start=10.000000:duration=4.000000"),
            "{g}"
        );
        assert!(g.contains("afade=t=out:st=3.000000:d=1.000000,volume=0.3000,adelay=delays=2000:all=1[m0];"), "{g}");
        assert!(g.ends_with("[m0]amix=inputs=1:duration=longest:normalize=0[music];[voice][music]amix=inputs=2:duration=first:normalize=0,apad,atrim=end=8.000000[aout]"), "{g}");
        // After the voice's own graph, and without a voice.
        let cut = with_music(Some("[1:a:0]x,y[aout]".into()), 1, &[m], None, 8.0).unwrap();
        assert!(cut.starts_with("[1:a:0]x,y[voice];"), "{cut}");
        let alone = with_music(None, 0, &[m], None, 8.0).unwrap();
        assert!(
            alone.ends_with("[m0]amix=inputs=1:duration=longest:normalize=0,apad,atrim=end=8.000000[aout]"),
            "{alone}"
        );
        assert_eq!(with_music(None, 1, &[], None, 8.0), None);
        let sudden = Placed { fade_in: 0.0, fade_out: 0.0, ..m };
        assert!(!with_music(None, 1, &[sudden], None, 8.0).unwrap().contains("afade"));
    }

    #[test]
    fn the_music_is_lowered_where_the_voice_speaks() {
        let m = Placed { index: 0, at: 2.0, until: 6.0, from: 0.0, volume: 1.0, fade_in: 0.0, fade_out: 0.0 };
        // The second stretch is outside the music: left out of the expression.
        let speaking = [(3.0, 4.0), (10.0, 11.0)];
        let g = with_music(None, 1, &[m], Some((0.25, &speaking)), 12.0).unwrap();
        assert!(
            g.contains(",volume='1-0.7500*(clip(min((t-2.7000)/0.3,(4.3000-t)/0.3),0,1))':eval=frame[music]"),
            "{g}"
        );
        assert!(with_music(None, 1, &[m], Some((0.25, &[])), 12.0).unwrap().contains("volume='1'"));
    }

    #[test]
    fn the_sound_recorded_apart_is_lined_up_and_mixed_in() {
        let sound = "[1:a:0]aresample=async=1:first_pts=0";
        assert_eq!(
            mix_graph(1, 0.12),
            format!(
                "[0:a:0]aresample=async=1:first_pts=0[m];{sound},adelay=delays=120:all=1[s];\
                     [m][s]amix=inputs=2:duration=first:normalize=0[a]"
            )
        );
        assert!(mix_graph(1, -0.5).contains(&format!("{sound},atrim=start=0.500000,asetpts=PTS-STARTPTS[s]")));
        // No microphone: the sound alone.
        assert_eq!(mix_graph(0, 0.0), format!("{sound},apad[a]"));
    }

    #[test]
    fn the_noise_is_taken_out_of_the_voice_before_cutting_and_speeding_up() {
        const DENOISE: &str = "highpass=f=80,afftdn=nr=18.0:nf=-40.0";
        let d = Some(Denoise { strength: 0.5, floor: -62.0 });
        let whole = Timeline { parts: vec![(0.0, 10.0)] };
        let parts = Timeline { parts: vec![(1.0, 4.0), (6.0, 10.0)] };
        let one = "[1:a:0]aresample=async=1:first_pts=0";

        // The one track as is: denoising alone needs a graph.
        assert_eq!(filter(&whole, false, 1.0, 1, None), None);
        assert_eq!(filter(&whole, false, 1.0, 1, d).unwrap(), format!("{one},{DENOISE}[aout]"));
        assert_eq!(filter(&whole, false, 2.0, 1, d).unwrap(), format!("{one},{DENOISE},atempo=2.000000[aout]"));
        let cut = filter(&parts, true, 1.0, 1, d).unwrap();
        assert!(cut.starts_with(&format!("{one},{DENOISE},asplit=2[s0][s1];")), "{cut}");
        let both = filter(&parts, true, 1.5, 1, d).unwrap();
        assert!(both.starts_with(&format!("{one},{DENOISE},asplit=2")), "{both}");
        assert!(both.ends_with("concat=n=2:v=0:a=1[acut];[acut]atempo=1.500000[aout]"), "{both}");
        assert_eq!(both.matches("afftdn").count(), 1, "{both}");

        // Two tracks: denoised once, after they're mixed.
        let mixed = filter(&parts, true, 1.0, 2, d).unwrap();
        assert!(mixed.contains(&format!("amix=inputs=2:duration=longest:normalize=0,{DENOISE},asplit=2")), "{mixed}");

        // Off, everything is as it was.
        assert_eq!(filter(&parts, true, 1.0, 2, None).unwrap(), cut_filter(&parts, 2, None));
        assert!(!filter(&parts, true, 1.5, 1, None).unwrap().contains("afftdn"));
    }

    #[test]
    fn the_strength_and_the_noise_floor_set_afftdn() {
        let f = |strength, floor| Denoise { strength, floor }.filters();
        assert_eq!(f(0.0, -70.0), "highpass=f=80,afftdn=nr=6.0:nf=-48.0");
        assert_eq!(f(1.0, -50.0), "highpass=f=80,afftdn=nr=30.0:nf=-28.0");
        // Kept to what afftdn accepts.
        assert_eq!(f(2.0, -30.0), "highpass=f=80,afftdn=nr=30.0:nf=-20.0");
        assert_eq!(f(-1.0, -120.0), "highpass=f=80,afftdn=nr=6.0:nf=-80.0");
        assert_eq!(f(f32::NAN, f32::NAN), "highpass=f=80,afftdn=nr=6.0:nf=-38.0");
    }

    #[test]
    fn music_is_never_denoised() {
        let m = Placed { index: 0, at: 0.0, until: 4.0, from: 0.0, volume: 0.5, fade_in: 0.0, fade_out: 0.0 };
        let tl = Timeline { parts: vec![(0.0, 10.0)] };
        let voice = filter(&tl, false, 1.0, 1, Some(Denoise { strength: 0.5, floor: -60.0 }));
        let g = with_music(voice, 1, &[m], None, 10.0).unwrap();
        let (voice, music) = g.split_once("[voice];").unwrap();
        assert!(voice.contains("afftdn") && !music.contains("afftdn"), "{g}");
    }

    #[test]
    fn the_filter_keeps_each_part() {
        let tl = Timeline { parts: vec![(1.0, 4.0), (6.0, 10.0)] };
        let f = cut_filter(&tl, 1, None);
        assert!(f.starts_with("[1:a:0]aresample=async=1:first_pts=0,asplit=2[s0][s1];"));
        assert!(f.contains("[s0]atrim=start=1.000000:end=4.000000,asetpts=PTS-STARTPTS"));
        assert!(f.contains("afade=t=out:st=2.990000:d=0.010000[a0]"));
        assert!(f.ends_with("[a0][a1]concat=n=2:v=0:a=1[aout]"));
    }
}
