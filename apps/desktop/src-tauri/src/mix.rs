//! The episode's sound (ADR 0007): the brand music made into a seamless loop, the brand sound effects placed on
//! their lines, and the loudness measured so every episode comes out at the same level. The join itself is in
//! `assembly.rs`; this is the part that needs no app state, so it can be tested on its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

use crate::commands::{CmdResult, msg};

/// Integrated loudness every episode is brought to (LUFS): YouTube's own playback level.
pub(crate) const TARGET_LUFS: f64 = -14.0;
/// Seconds the end of the brand music is blended into its start, each time it loops.
const LOOP_FADE: f64 = 3.0;
/// The loop finder reads the track at this rate, in 10 ms frames.
const SCAN_RATE: u32 = 8000;
const FRAME: usize = (SCAN_RATE / 100) as usize;
/// Music quieter than its usual level by more than this (dB) is an intro or a fade-out, left out of the loop.
const EDGE_DB: f64 = 4.5;

/// Where the brand music loops: it plays from `start` to `end` (seconds into the track), and the `fade` seconds
/// after `end` are blended into the `fade` seconds after `start`, so the loop never stops or skips a beat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LoopPoints {
    pub start: f64,
    pub end: f64,
    pub fade: f64,
}

/// Finds the loop in a track (`samples` mono at `SCAN_RATE`): the steady middle, without a soft intro or the
/// fade-out, ending where the beat lines up best with the start.
pub(crate) fn loop_points(samples: &[f32]) -> CmdResult<LoopPoints> {
    let level: Vec<f64> = samples
        .as_chunks::<FRAME>()
        .0
        .iter()
        .map(|f| 10.0 * (f.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / FRAME as f64 + 1e-12).log10())
        .collect();
    // Half a second around each frame, so a quiet beat between two hits is not an edge.
    let half = 25;
    let smooth: Vec<f64> = (0..level.len())
        .map(|i| {
            let window = &level[i.saturating_sub(half)..(i + half).min(level.len())];
            10.0 * (window.iter().map(|db| 10f64.powf(db / 10.0)).sum::<f64>() / window.len() as f64).log10()
        })
        .collect();
    let mut loud: Vec<f64> = smooth.iter().copied().filter(|db| *db > -70.0).collect();
    if loud.is_empty() {
        return Err("the brand music is silent".into());
    }
    loud.sort_by(f64::total_cmp);
    let usual = loud[loud.len() / 2];
    let steady = |i: &usize| smooth[*i] >= usual - EDGE_DB;
    let first = (0..smooth.len()).find(steady).unwrap_or(0);
    let last = (0..smooth.len()).rev().find(steady).unwrap_or(0) + 1;
    let fade = (LOOP_FADE * 100.0) as usize;
    if last.saturating_sub(first) < 2 * fade + 1000 {
        return Err("the brand music is too short to loop: it needs at least 16 s of steady music".into());
    }
    // Rises in level are the hits: the loop ends where the hits after it fall like the hits after the start.
    let onset: Vec<f64> = std::iter::once(0.0).chain(level.windows(2).map(|w| (w[1] - w[0]).max(0.0))).collect();
    let head = &onset[first..first + fade];
    let centre = |xs: &[f64]| {
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        xs.iter().map(|x| x - mean).collect::<Vec<_>>()
    };
    let head = centre(head);
    let head_norm = head.iter().map(|x| x * x).sum::<f64>().sqrt();
    let latest = last - fade;
    let earliest = (first + fade + 1000).max(latest.saturating_sub(1500));
    let score = |end: usize| {
        let tail = centre(&onset[end..end + fade]);
        let norm = tail.iter().map(|x| x * x).sum::<f64>().sqrt() * head_norm;
        if norm < 1e-9 { 0.0 } else { tail.iter().zip(&head).map(|(a, b)| a * b).sum::<f64>() / norm }
    };
    let end = (earliest..=latest).rev().max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap_or(latest);
    Ok(LoopPoints { start: first as f64 / 100.0, end: end as f64 / 100.0, fade: LOOP_FADE })
}

/// ffmpeg's filter graph that turns a track (input 0) into its loop, at 48 kHz stereo.
pub(crate) fn loop_graph(p: LoopPoints) -> String {
    let LoopPoints { start, end, fade } = p;
    format!(
        "[0:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asplit=3[a][b][c];\
         [a]atrim=start={end:.3}:duration={fade},asetpts=PTS-STARTPTS,afade=t=out:d={fade}:curve=qsin[tail];\
         [b]atrim=start={start:.3}:duration={fade},asetpts=PTS-STARTPTS,afade=t=in:d={fade}:curve=qsin[head];\
         [tail][head]amix=inputs=2:normalize=0[blend];\
         [c]atrim=start={:.3}:end={end:.3},asetpts=PTS-STARTPTS[body];[blend][body]concat=n=2:v=0:a=1[out]",
        start + fade
    )
}

/// Makes `source` into the loop the episodes play (`out`, a 48 kHz float WAV).
pub(crate) fn prepare_music(ffmpeg: &Path, source: &Path, out: &Path) -> CmdResult<()> {
    let scan = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(source)
        .args(["-t", "900", "-ac", "1", "-ar", &SCAN_RATE.to_string(), "-f", "f32le", "-"])
        .output()
        .map_err(|e| format!("ffmpeg did not start: {e}"))?;
    if !scan.status.success() {
        return Err(format!("the track could not be read: {}", String::from_utf8_lossy(&scan.stderr).trim()));
    }
    let samples: Vec<f32> = scan.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    let points = loop_points(&samples)?;
    let made = Command::new(ffmpeg)
        .args(["-y", "-v", "error", "-i"])
        .arg(source)
        .args(["-filter_complex", &loop_graph(points), "-map", "[out]", "-c:a", "pcm_f32le"])
        .arg(out)
        .output()
        .map_err(|e| format!("ffmpeg did not start: {e}"))?;
    if !made.status.success() {
        return Err(format!("the loop could not be made: {}", String::from_utf8_lossy(&made.stderr).trim()));
    }
    Ok(())
}

/// Integrated loudness (LUFS) of `graph` (a filter chain over `inputs`, without its output label), or None if
/// it is silent.
pub(crate) fn loudness(ffmpeg: &Path, inputs: &[PathBuf], graph: &str) -> CmdResult<Option<f64>> {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-nostats"]);
    for input in inputs {
        command.arg("-i").arg(input);
    }
    let output = command
        .args(["-filter_complex", &format!("{graph},loudnorm=print_format=json[m]"), "-map", "[m]", "-f", "null", "-"])
        .output()
        .map_err(|e| format!("ffmpeg did not start: {e}"))?;
    let text = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!("measuring the loudness failed: {}", text.lines().last().unwrap_or("").trim()));
    }
    let report = text.rfind('{').and_then(|a| text[a..].find('}').map(|b| &text[a..=a + b]));
    let value: Value = serde_json::from_str(report.ok_or("ffmpeg printed no loudness")?).map_err(msg)?;
    let i = value["input_i"].as_str().ok_or("ffmpeg printed no loudness")?;
    Ok(i.parse::<f64>().ok().filter(|db| db.is_finite() && *db > -70.0))
}

/// The brand sound effects' `library.json` (ADR 0007): what each tag plays and how loud.
#[derive(Debug, Deserialize)]
pub(crate) struct SfxLibrary {
    /// Gain of each tier in dB ("soft", "medium", "loud"); every sound is mastered to the same loudness.
    pub tiers_db: BTreeMap<String, f64>,
    pub sounds: Vec<Sound>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Sound {
    pub tag: String,
    pub file: String,
    pub tier: String,
    /// Seconds the sound starts before its panel appears, so a swell peaks on the cut.
    #[serde(default)]
    pub lead_s: f64,
    /// Adds a copy of a very deep sound (heartbeat) moved up to the 200 Hz-2 kHz band, so phone speakers that cannot
    /// play its low end still play its rhythm.
    #[serde(default)]
    pub phone_layer: bool,
    /// Adds a short echo tail to a sound that stops at its loudest (whoosh, riser).
    #[serde(default)]
    pub tail: bool,
}

/// A deep sound plus itself ring-modulated at 220 Hz: the same thump, also heard around 200-300 Hz (+10 dB there for
/// the heartbeat), at about the same overall level. (ffmpeg's aexciter only works above 2 kHz.)
const PHONE_LAYER: &str = "aeval=val(0)+0.8*val(0)*sin(2*PI*220*t)|val(1)+0.8*val(1)*sin(2*PI*220*t):c=same";
/// A short echo after a sound that ends at its peak (padded first, so the echo has room to ring out).
const TAIL: &str = "apad=pad_dur=0.3,aecho=1:1:40|90:0.3|0.2";
/// A sound over the narration steps back where the voice is clearest.
const UNDER_VOICE: &str = "equalizer=f=2500:t=o:w=1.5:g=-4";

/// A sound effect in the episode: `file` (inside the library) played from `at` seconds at `db`, through `filters`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Effect {
    pub file: String,
    pub at: f64,
    pub db: f64,
    pub filters: Vec<&'static str>,
}

/// When each beat of a part starts, in seconds, as the Episode composition plays it: a transition overlaps the
/// previous beat (packages/motion/src/timeline.ts, and the render's join.py).
pub(crate) fn beat_starts(spec: &Value) -> Vec<f64> {
    let fps = spec["video"]["fps"].as_f64().unwrap_or(30.0);
    let (mut cursor, mut prev, mut starts) = (0i64, None::<i64>, Vec::new());
    for (i, beat) in spec["beats"].as_array().into_iter().flatten().enumerate() {
        let dur = ((beat["duration"].as_f64().unwrap_or(0.0) * fps).round() as i64).max(1);
        let cut = i == 0 || beat["transition_in"]["name"].as_str() == Some("cut");
        let want =
            if cut { 0 } else { (beat["transition_in"]["duration"].as_f64().unwrap_or(0.0) * fps).round() as i64 };
        let overlap = prev.map_or(0, |p| want.min(p - 1).min(dur - 1));
        let start = cursor - overlap;
        starts.push(start as f64 / fps);
        cursor = start + dur;
        prev = Some(dur);
    }
    starts
}

/// The sound effects of a part that starts `offset` seconds into the episode, at `extra_db` beyond their tier.
pub(crate) fn part_effects(spec: &Value, offset: f64, library: &SfxLibrary, extra_db: f64) -> CmdResult<Vec<Effect>> {
    let starts = beat_starts(spec);
    let mut effects = Vec::new();
    for (beat, start) in spec["beats"].as_array().into_iter().flatten().zip(starts) {
        let Some(tag) = beat["sfx"].as_str() else { continue };
        let sound = library
            .sounds
            .iter()
            .find(|s| s.tag == tag)
            .ok_or_else(|| format!("the sound effect library has no {tag}"))?;
        let tier =
            library.tiers_db.get(&sound.tier).ok_or_else(|| format!("{tag}'s tier {} is unknown", sound.tier))?;
        // A beat with narration means the sound lands on the narrator's voice.
        let over_voice = !beat["audio"].is_null();
        let filters = [(sound.phone_layer, PHONE_LAYER), (sound.tail, TAIL), (over_voice, UNDER_VOICE)]
            .into_iter()
            .filter_map(|(on, f)| on.then_some(f))
            .collect();
        effects.push(Effect {
            file: sound.file.clone(),
            at: (offset + start - sound.lead_s).max(0.0),
            db: tier + extra_db,
            filters,
        });
    }
    Ok(effects)
}

/// The part's spec without its sound effects: they are mixed in at assembly, so they never make a part outdated.
pub(crate) fn without_effects(spec: &Value) -> Value {
    let mut spec = spec.clone();
    for beat in spec["beats"].as_array_mut().into_iter().flatten() {
        if let Some(beat) = beat.as_object_mut() {
            beat.remove("sfx");
        }
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A beat every half second (120 BPM) for `seconds`, then a 3 s fade to silence and 2 s of nothing.
    fn track(seconds: f64) -> Vec<f32> {
        let rate = SCAN_RATE as f64;
        let body = (seconds * rate) as usize;
        let fade = (3.0 * rate) as usize;
        (0..body + fade + (2.0 * rate) as usize)
            .map(|i| {
                let t = i as f64 / rate;
                let hit = (-((t % 0.5) * 40.0)).exp();
                let bed = 0.05 * (t * 220.0 * std::f64::consts::TAU).sin();
                let gain = if i < body {
                    1.0
                } else if i < body + fade {
                    1.0 - (i - body) as f64 / fade as f64
                } else {
                    0.0
                };
                ((hit * (t * 90.0 * std::f64::consts::TAU).sin() + bed) * gain) as f32
            })
            .collect()
    }

    #[test]
    fn the_loop_leaves_out_the_fade_and_lands_on_the_beat() {
        let p = loop_points(&track(80.0)).expect("loop");
        assert!(p.start < 0.5, "{p:?}");
        // The blended 3 s after the end are still steady music, not the fade-out.
        assert!(p.end + p.fade <= 80.5 && p.end > 60.0, "{p:?}");
        let beats = (p.end - p.start) / 0.5;
        assert!((beats - beats.round()).abs() < 0.05, "the loop is {beats} beats: {p:?}");
    }

    #[test]
    fn a_short_or_silent_track_cannot_loop() {
        assert!(loop_points(&track(8.0)).expect_err("short").contains("too short"));
        assert!(loop_points(&vec![0.0; 80_000]).expect_err("silent").contains("silent"));
    }

    #[test]
    fn the_loop_graph_blends_the_end_into_the_start() {
        let graph = loop_graph(LoopPoints { start: 1.5, end: 80.0, fade: 3.0 });
        for part in [
            "atrim=start=80.000:duration=3",
            "atrim=start=1.500:duration=3",
            "atrim=start=4.500:end=80.000",
            "curve=qsin",
        ] {
            assert!(graph.contains(part), "{part} missing from {graph}");
        }
    }

    fn library() -> SfxLibrary {
        serde_json::from_value(json!({
            "tiers_db": {"soft": -6, "medium": -3, "loud": 0},
            "sounds": [
                {"tag": "punch", "file": "02_punch.wav", "tier": "medium", "phone_layer": true},
                {"tag": "whoosh", "file": "01_whoosh.wav", "tier": "soft", "lead_s": 0.72, "tail": true}
            ]
        }))
        .expect("library")
    }

    #[test]
    fn a_sound_effect_plays_as_its_beat_starts() {
        let spec = json!({"video": {"fps": 30}, "beats": [
            {"duration": 2.0, "transition_in": {"name": "cut", "duration": 0.3}, "sfx": "whoosh", "audio": null},
            {"duration": 3.0, "transition_in": {"name": "fade", "duration": 0.3}, "sfx": "punch", "audio": {"src": "a.wav"}},
            {"duration": 1.0, "transition_in": {"name": "cut", "duration": 0.3}, "sfx": null},
        ]});
        // The fade starts the second beat 9 frames early; the third follows on a cut.
        assert_eq!(beat_starts(&spec), vec![0.0, 1.7, 4.7]);
        let effects = part_effects(&spec, 100.0, &library(), 2.0).expect("effects");
        assert_eq!(
            effects,
            vec![
                Effect { file: "01_whoosh.wav".into(), at: 100.0 - 0.72, db: -4.0, filters: vec![TAIL] },
                // On the narration: it steps back where the voice is clearest.
                Effect { file: "02_punch.wav".into(), at: 101.7, db: -1.0, filters: vec![PHONE_LAYER, UNDER_VOICE] },
            ]
        );
        assert!(part_effects(&json!({"beats": [{"duration": 1, "sfx": "laser"}]}), 0.0, &library(), 0.0).is_err());
    }

    #[test]
    fn sound_effects_never_make_a_part_outdated() {
        let with = json!({"beats": [{"duration": 1, "sfx": "punch"}, {"duration": 2}]});
        assert_eq!(without_effects(&with), json!({"beats": [{"duration": 1}, {"duration": 2}]}));
    }
}
