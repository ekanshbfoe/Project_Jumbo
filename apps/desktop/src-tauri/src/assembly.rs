//! Episodes (ADR 0007): several voiced chapters joined into one long video. Each chapter is rendered on Kaggle as
//! its own part (`render.rs`), with a fingerprint of everything the part was made from, so an unchanged chapter is
//! never rendered twice and a changed one is never joined stale. The join runs on this PC with a pinned ffmpeg:
//! the parts' video is copied as it is, and their soundtracks are joined, brought to YouTube's loudness and laid
//! under the brand music and sound effects once (`mix.rs`).
//!
//! An episode lives in `episodes/<ep_NNNN>/episode.json`; its video is `episodes/<ep_NNNN>/<ep_NNNN>.mp4`. A chapter
//! belongs to at most one episode, so its part is simply `chapters/<id>/render/part.{mp4,wav,json}`. The episode's
//! wrapper (ADR 0008) adds up to two parts of its own around the chapters, `episodes/<ep_NNNN>/{intro,outro}/`, from
//! the script and clips voiced into `episodes/<ep_NNNN>/wrapper/`.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jumbo_bridge::render;
use jumbo_core::chapter;
use jumbo_core::{ChapterId, Project, fsx};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};

use crate::commands::{AppState, CmdResult, Lane, begin_job, config_dir, msg, with_project};
use crate::mix::{self, Effect, SfxLibrary, TARGET_LUFS};

/// File stem of a chapter's part inside its `render` folder.
pub(crate) const PART: &str = "part";
const EPISODES: &str = "episodes";
const EPISODE_FILE: &str = "episode.json";
/// The wrapper's parts: the intro plays before the episode's first chapter, the outro after its last.
pub(crate) const WRAPPER_PARTS: [&str; 2] = ["intro", "outro"];
/// The folder of an episode's voiced wrapper: its script (`script.json`) and clips (`audio/`).
const WRAPPER: &str = "wrapper";

/// The ffmpeg the join uses: a static build pinned by version and SHA-256, downloaded once into the app's folder.
const FFMPEG_RELEASE: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1";
const FFMPEG_BUILDS: &[(&str, &str, &str)] = &[
    ("linux-x86_64", "ffmpeg-linux-x64", "e7e7fb30477f717e6f55f9180a70386c62677ef8a4d4d1a5d948f4098aa3eb99"),
    ("linux-aarch64", "ffmpeg-linux-arm64", "6bb182d0d75d23028db82e9e4f723ca69b853d055698486e6984ddb2c06fb8ce"),
    ("windows-x86_64", "ffmpeg-win32-x64", "04e1307997530f9cf2fe35cba2ca7e8875ca91da02f89d6c7243df819c94ad00"),
    ("macos-x86_64", "ffmpeg-darwin-x64", "ebdddc936f61e14049a2d4b549a412b8a40deeff6540e58a9f2a2da9e6b18894"),
    ("macos-aarch64", "ffmpeg-darwin-arm64", "a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584"),
];

/// Seconds the brand music fades in at the start and out at the end of an episode.
const MUSIC_FADE_IN: f64 = 2.0;
const MUSIC_FADE_OUT: f64 = 3.0;
/// The brand music under the voice: the 808 sub held down, dips where the voice lives (body and clarity), and the
/// sides narrowed a little so phones playing in mono keep its middle.
const MUSIC_SHAPE: &str = "highpass=f=40,lowshelf=f=90:g=-6,equalizer=f=1200:t=o:w=2:g=-4,\
                           equalizer=f=3000:t=o:w=1:g=-2.5,stereotools=slev=0.7";
/// The music dips gently and steadily under the voice instead of slamming down on every word and surging in every
/// pause: a threshold at the voice's own level, 2:1, and a slow release that rides through short gaps.
const DUCK: &str = "sidechaincompress=threshold=0.1:ratio=2:attack=80:release=1500:knee=6";
/// The sound effects and their library, inside the brand folder.
const SFX_DIR: &str = "sfx";
/// The last stage of every episode: peaks held about 2 dB under full scale, so neither the AAC encode nor
/// YouTube's own re-encode overshoots -1 dBTP. `latency` keeps the sound in sync with the picture.
const LIMITER: &str = "alimiter=limit=0.79:attack=5:release=80:level=0:latency=1";
/// The narration before it is measured: rumble and DC removed, a lift where speech is clearest, and a slow
/// leveller (about 20 s) so one quiet stretch of lines does not sit a few dB under the rest of the hour.
const VOICE_SHAPE: &str = "highpass=f=70,equalizer=f=3500:t=q:w=1.2:g=2.5,\
                           dynaudnorm=f=1000:g=21:p=0.95:m=6:r=0.08:t=0.002";
/// The narration, once raised to the episode's loudness: its sharpest peaks evened out 3:1, so the limiters only
/// catch what is left instead of flattening every hard consonant.
const VOICE_COMPRESSOR: &str = "acompressor=threshold=0.25:ratio=3:attack=5:release=120:knee=4";
/// The narration's own peak limiter, so its peaks never reach the episode's limiter.
const VOICE_LIMITER: &str = "alimiter=limit=0.89:attack=1.5:release=40:level=0:latency=1";
/// The sound effects reach the music's ducking 6 dB down: a big hit dips the music, without a hole after it.
const FX_KEY_DB: f64 = -6.0;
/// Headroom kept free beyond the parts' own size while the episode is written.
const SPACE_MARGIN: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeFile {
    pub id: String,
    pub title: String,
    /// In story order, with no chapter left out between the first and the last.
    pub chapters: Vec<ChapterId>,
    pub created_at: String,
}

/// What a chapter's rendered part says about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartInfo {
    pub episode: String,
    pub fingerprint: String,
    pub width: u64,
    pub height: u64,
    pub fps: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeView {
    #[serde(flatten)]
    pub file: EpisodeFile,
    /// Project-relative path of the assembled video, once there is one.
    pub video: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartState {
    /// Never rendered for this episode.
    Missing,
    /// Rendered from an older script, voice, panels or renderer: rendering again is needed.
    Outdated,
    Ready,
}

/// The app-wide brand settings: one music track for every project and channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrandSettings {
    /// File name of the track inside `<app config>/brand`, if one is set.
    pub music: Option<String>,
    /// Level of the music under the narration, in dB below the voice (it dips further while the narrator speaks).
    pub music_db: f64,
    /// Level of the sound effects in dB, beyond each one's tier.
    #[serde(default)]
    pub sfx_db: f64,
    /// File name of the narrator's signature clip inside `<app config>/brand`: the same recording closes every
    /// episode of every channel (ADR 0008).
    #[serde(default)]
    pub signature: Option<String>,
    /// The clip's length in seconds: the last part holds its final panel long enough for it.
    #[serde(default)]
    pub signature_s: f64,
}

impl Default for BrandSettings {
    fn default() -> Self {
        Self { music: None, music_db: -17.0, sfx_db: 0.0, signature: None, signature_s: 0.0 }
    }
}

/// The longest signature clip: a closing line, not a speech.
const SIGNATURE_MAX_S: f64 = 8.0;
/// Seconds of silence after the signature before the episode ends.
const SIGNATURE_TAIL_S: f64 = 1.0;

#[derive(Clone, Serialize)]
struct Progress<'a> {
    episode: &'a str,
    progress: f64,
    message: String,
}

fn episodes_dir(project: &Project) -> PathBuf {
    project.paths().root().join(EPISODES)
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix("ep_").is_some_and(|d| d.len() == 4 && d.bytes().all(|b| b.is_ascii_digit()))
}

fn episode_dir(project: &Project, id: &str) -> CmdResult<PathBuf> {
    if !valid_id(id) {
        return Err(format!("{id} is not an episode id"));
    }
    Ok(episodes_dir(project).join(id))
}

pub(crate) fn load_episode(project: &Project, id: &str) -> CmdResult<EpisodeFile> {
    let file = episode_dir(project, id)?.join(EPISODE_FILE);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
}

pub(crate) fn list(project: &Project) -> CmdResult<Vec<EpisodeFile>> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(episodes_dir(project)) else { return Ok(out) };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if valid_id(&name) && entry.path().join(EPISODE_FILE).is_file() {
            out.push(load_episode(project, &name)?);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Checks a new episode's chapters: at least one, all in the project, in story order with none skipped, and none
/// already in another episode.
fn check_chapters(project: &Project, chapters: &[ChapterId], existing: &[EpisodeFile]) -> CmdResult<()> {
    let first = chapters.first().ok_or("an episode needs at least one chapter")?;
    let mut all: Vec<ChapterId> = chapter::list_chapters(project).map_err(msg)?.into_iter().map(|c| c.id).collect();
    all.sort();
    let start = all.iter().position(|c| c == first).ok_or_else(|| format!("{first} is not in this project"))?;
    for (k, id) in chapters.iter().enumerate() {
        if all.get(start + k) != Some(id) {
            return Err(format!(
                "an episode's chapters follow each other in story order; {id} does not come right after {}",
                chapters[k.saturating_sub(1)]
            ));
        }
        if let Some(other) = existing.iter().find(|e| e.chapters.contains(id)) {
            return Err(format!("{id} is already in {} ({})", other.id, other.title));
        }
    }
    Ok(())
}

pub(crate) fn create(project: &Project, title: &str, chapters: Vec<ChapterId>, now: String) -> CmdResult<EpisodeFile> {
    let title = title.trim();
    if title.is_empty() {
        return Err("an episode needs a title".into());
    }
    let existing = list(project)?;
    check_chapters(project, &chapters, &existing)?;
    let next = existing.iter().filter_map(|e| e.id[3..].parse::<u32>().ok()).max().unwrap_or(0) + 1;
    let file = EpisodeFile { id: format!("ep_{next:04}"), title: title.to_owned(), chapters, created_at: now };
    fsx::atomic_write_json(&episode_dir(project, &file.id)?.join(EPISODE_FILE), &file).map_err(msg)?;
    Ok(file)
}

/// Refuses a part render for a chapter that is not in `episode`.
pub(crate) fn check_member(project: &Project, episode: &str, id: ChapterId) -> CmdResult<()> {
    let file = load_episode(project, episode)?;
    if file.chapters.contains(&id) { Ok(()) } else { Err(format!("{id} is not in {episode}")) }
}

/// The folder of an episode's voiced wrapper.
pub(crate) fn wrapper_dir(project: &Project, episode: &str) -> CmdResult<PathBuf> {
    Ok(episode_dir(project, episode)?.join(WRAPPER))
}

/// Where the part `key` of `episode` lives: `intro` and `outro` in the episode's folder, a chapter's in its `render`.
pub(crate) fn part_dir(project: &Project, episode: &str, key: &str) -> CmdResult<PathBuf> {
    if WRAPPER_PARTS.contains(&key) {
        return episode_dir(project, episode).map(|d| d.join(key));
    }
    let id: ChapterId = key.parse().map_err(msg)?;
    Ok(chapter::chapter_dir(project, id).join("render"))
}

/// The episode's parts in playing order: the intro when the app has a spec for one, the chapters, then the outro.
fn part_order(file: &EpisodeFile, specs: &serde_json::Map<String, Value>) -> Vec<String> {
    let [intro, outro] = WRAPPER_PARTS;
    let mut keys: Vec<String> = specs.contains_key(intro).then(|| intro.to_owned()).into_iter().collect();
    keys.extend(file.chapters.iter().map(ToString::to_string));
    keys.extend(specs.contains_key(outro).then(|| outro.to_owned()));
    keys
}

/// A part's fingerprint: its EpisodeSpec, every file it plays (panels, backdrops, narration) and the renderer
/// itself. The same fingerprint means the same video, so a part is rendered again only when one of them changed.
/// Sound effects are left out: they are mixed in at assembly, so changing one never renders a part again.
pub(crate) fn fingerprint(root: &Path, spec: &Value) -> CmdResult<String> {
    let mut hash = blake3::Hasher::new();
    hash.update(b"jumbo-part-1\0");
    hash.update(&serde_json::to_vec(&mix::without_effects(spec)).map_err(msg)?);
    hash.update(render::render_kit().as_bytes());
    hash.update(render::render_scripts().as_bytes());
    for rel in crate::render::assets(spec) {
        let path = root.join(&rel);
        let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
        hash.update(rel.as_bytes());
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(hash.finalize().to_hex().to_string())
}

pub(crate) fn write_part(render_dir: &Path, episode: &str, fingerprint: &str, spec: &Value) -> CmdResult<()> {
    let video = &spec["video"];
    let number = |k: &str| video[k].as_u64().ok_or_else(|| format!("the part's video.{k} is missing"));
    let info = PartInfo {
        episode: episode.to_owned(),
        fingerprint: fingerprint.to_owned(),
        width: number("width")?,
        height: number("height")?,
        fps: number("fps")?,
    };
    fsx::atomic_write_json(&render_dir.join(format!("{PART}.json")), &info).map_err(msg)
}

fn read_part(render_dir: &Path) -> Option<PartInfo> {
    let text = std::fs::read_to_string(render_dir.join(format!("{PART}.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether the part `key` (a chapter id, `intro` or `outro`) of `episode` is current, given the part `spec` the app
/// would render now.
pub(crate) fn part_state(project: &Project, episode: &str, key: &str, spec: &Value) -> CmdResult<PartState> {
    let render_dir = part_dir(project, episode, key)?;
    let Some(info) = read_part(&render_dir) else { return Ok(PartState::Missing) };
    let files = [format!("{PART}.mp4"), format!("{PART}.wav")];
    if info.episode != episode || files.iter().any(|f| !render_dir.join(f).is_file()) {
        return Ok(PartState::Missing);
    }
    Ok(if info.fingerprint == fingerprint(project.paths().root(), spec)? {
        PartState::Ready
    } else {
        PartState::Outdated
    })
}

/// Length in seconds of a PCM WAV file, from its header.
fn wav_seconds(path: &Path) -> CmdResult<f64> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(1 << 16).read_to_end(&mut bytes))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let bad = || format!("{} is not a PCM WAV file", path.display());
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(bad());
    }
    let (mut at, mut byte_rate) = (12usize, None);
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().map_err(|_| bad())?) as u64;
        match &bytes[at..at + 4] {
            b"fmt " => byte_rate = bytes.get(at + 16..at + 20).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes),
            b"data" => {
                let rate = byte_rate.filter(|r| *r > 0).ok_or_else(bad)?;
                // A writer that could not seek leaves the size unset; the file's length is then the data's.
                let size = if size == 0 || size == u64::from(u32::MAX) {
                    std::fs::metadata(path).map_err(msg)?.len().saturating_sub(at as u64 + 8)
                } else {
                    size
                };
                return Ok(size as f64 / f64::from(rate));
            }
            _ => {}
        }
        at += 8 + size as usize + (size as usize & 1);
    }
    Err(bad())
}

/// A line of an ffmpeg concat list. Backslashes become slashes (Windows paths) and quotes are escaped.
fn concat_line(path: &Path) -> String {
    format!("file '{}'\n", path.to_string_lossy().replace('\\', "/").replace('\'', "'\\''"))
}

/// The music under the narration: its loop and its gain in dB.
struct Music<'a> {
    path: &'a Path,
    db: f64,
}

/// The signature clip, where it starts and its gain to the episode's loudness.
struct Signature<'a> {
    path: &'a Path,
    at: f64,
    db: f64,
}

/// The sound effects of an episode: the library folder, and each effect where it plays.
struct Effects<'a> {
    dir: &'a Path,
    list: &'a [Effect],
}

/// The narration's gain in dB before its compressor (to the episode's loudness) and after it (making up what the
/// compressor took), both measured from the soundtracks.
#[derive(Debug, Clone, Copy)]
struct VoiceGain {
    before: f64,
    after: f64,
}

/// The parts' soundtracks shaped (`VOICE_SHAPE`): what the first loudness measurement reads.
fn voice_shaped(first: usize, n: usize) -> String {
    voice_chain(first, n) + "," + VOICE_SHAPE
}

/// The shaped narration raised by `before` and compressed: what the second loudness measurement reads.
fn voice_compressed(first: usize, n: usize, before: f64) -> String {
    voice_shaped(first, n) + &format!(",volume={before:.2}dB,{VOICE_COMPRESSOR}")
}

/// The parts' soundtracks levelled: shaped, raised, compressed, made up to the episode's loudness and limited.
fn voice_levelled(first: usize, n: usize, gain: VoiceGain) -> String {
    voice_compressed(first, n, gain.before) + &format!(",volume={:.2}dB,{VOICE_LIMITER}", gain.after)
}

/// The parts' `n` soundtracks (inputs `first` onwards) resampled and joined, before any gain.
fn voice_chain(first: usize, n: usize) -> String {
    let format = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
    let mut graph: String = (first..first + n).map(|i| format!("[{i}:a]{format}[s{i}];")).collect();
    graph += &(first..first + n).map(|i| format!("[s{i}]")).collect::<String>();
    graph + &format!("concat=n={n}:v=0:a=1")
}

/// ffmpeg's arguments and filter graph for an episode: the parts' video copied as it is from the concat list
/// `video_list`, their soundtracks joined and levelled by `voice` (to the episode's loudness), the brand music
/// (looped, faded in and out) dipping under the narration and the big hits, the sound effects on their lines, the
/// signature in the final hold, a limiter, and the result cut to `seconds` and written to `out`. The graph is read
/// from `graph_file`.
#[allow(clippy::too_many_arguments)]
fn join_args(
    video_list: &Path,
    soundtracks: &[PathBuf],
    voice: VoiceGain,
    signature: Option<Signature>,
    music: Option<Music>,
    effects: Effects,
    seconds: f64,
    graph_file: &Path,
    out: &Path,
) -> (Vec<String>, String) {
    let mut args: Vec<String> =
        ["-y", "-v", "error", "-nostats", "-progress", "pipe:1", "-f", "concat", "-safe", "0", "-i"]
            .map(String::from)
            .into();
    args.push(video_list.display().to_string());
    for s in soundtracks {
        args.extend(["-i".into(), s.display().to_string()]);
    }
    let format = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
    let mut next = soundtracks.len() + 1;
    let mut graph = voice_levelled(1, soundtracks.len(), voice);
    // The signature joins the narration, so the music dips under it like under any other line.
    match signature {
        Some(sig) => {
            args.extend(["-i".into(), sig.path.display().to_string()]);
            graph += &format!(
                "[said];[{next}:a]{format},volume={:.2}dB,adelay=delays={}:all=1[sig];\
                 [said][sig]amix=inputs=2:duration=first:normalize=0[voice];",
                sig.db,
                (sig.at * 1000.0).round() as u64
            );
            next += 1;
        }
        None => graph += "[voice];",
    }

    // Each sound file is read once and split for every line it plays on.
    let mut files: Vec<(&str, Vec<&Effect>)> = Vec::new();
    for e in effects.list {
        match files.iter_mut().find(|(f, _)| *f == e.file) {
            Some((_, uses)) => uses.push(e),
            None => files.push((&e.file, vec![e])),
        }
    }
    let mut placed = Vec::new();
    for (file, uses) in &files {
        args.extend(["-i".into(), effects.dir.join(file).display().to_string()]);
        let labels: String = (0..uses.len()).map(|k| format!("[x{next}_{k}]")).collect();
        graph += &format!("[{next}:a]{format},asplit={}{labels};", uses.len());
        for (k, e) in uses.iter().enumerate() {
            let filters: String = e.filters.iter().map(|f| format!("{f},")).collect();
            graph += &format!(
                "[x{next}_{k}]{filters}volume={:.1}dB,adelay=delays={}:all=1[d{next}_{k}];",
                e.db,
                (e.at * 1000.0).round() as u64
            );
            placed.push(format!("[d{next}_{k}]"));
        }
        next += 1;
    }
    let fx = !placed.is_empty();
    if fx {
        graph += &format!("{}amix=inputs={}:duration=longest:normalize=0[fx];", placed.concat(), placed.len());
    }

    let mix = match music {
        Some(m) => {
            args.extend(["-stream_loop", "-1", "-i"].map(String::from));
            args.push(m.path.display().to_string());
            let fade_out = (seconds - MUSIC_FADE_OUT).max(0.0);
            graph += &format!(
                "[{next}:a]{format},{MUSIC_SHAPE},volume={:.2}dB,afade=t=in:d={MUSIC_FADE_IN},afade=t=out:st={fade_out:.3}:d={MUSIC_FADE_OUT}[bed];",
                m.db
            );
            // The music dips under the voice and under the sound effects alike.
            if fx {
                graph += &format!(
                    "[voice]asplit=2[say][vkey];[fx]asplit=2[fxmix][fxk];[fxk]volume={FX_KEY_DB}dB[fxkey];\
                     [vkey][fxkey]amix=inputs=2:duration=first:normalize=0[key];"
                );
            } else {
                graph += "[voice]asplit=2[say][key];";
            }
            graph += &format!("[bed][key]{DUCK}[duck];");
            if fx { "[say][duck][fxmix]amix=inputs=3" } else { "[say][duck]amix=inputs=2" }
        }
        None if fx => "[voice][fx]amix=inputs=2",
        None => "[voice]anull",
    };
    graph += &format!("{mix}{},{LIMITER}[a]", if mix.contains("amix") { ":duration=first:normalize=0" } else { "" });

    args.extend(["-filter_complex_script".into(), graph_file.display().to_string()]);
    args.extend(["-map", "0:v", "-map", "[a]", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-t"].map(String::from));
    args.push(format!("{seconds:.3}"));
    args.extend(["-movflags", "+faststart"].map(String::from));
    args.push(out.display().to_string());
    (args, graph)
}

fn brand_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    Ok(config_dir(app)?.join("brand"))
}

fn load_brand(app: &AppHandle) -> CmdResult<BrandSettings> {
    match std::fs::read_to_string(brand_dir(app)?.join("settings.json")) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("brand settings: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BrandSettings::default()),
        Err(e) => Err(format!("brand settings: {e}")),
    }
}

fn save_brand(app: &AppHandle, settings: &BrandSettings) -> CmdResult<()> {
    fsx::atomic_write_json(&brand_dir(app)?.join("settings.json"), settings).map_err(msg)
}

/// The pinned ffmpeg for this platform, downloaded and checked on first use.
async fn ffmpeg(app: &AppHandle, report: impl Fn(f64, String)) -> CmdResult<PathBuf> {
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let (_, asset, sha) = FFMPEG_BUILDS
        .iter()
        .find(|(p, _, _)| *p == platform)
        .ok_or_else(|| format!("no ffmpeg build is pinned for {platform}"))?;
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let path = config_dir(app)?.join("tools").join(format!("{asset}-6.1.1{exe}"));
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().ok_or("bad tools path")?).map_err(msg)?;
    let mut response = jumbo_bridge::http::upload_client()
        .get(format!("{FFMPEG_RELEASE}/{asset}"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("downloading ffmpeg: {e}"))?;
    let total = response.content_length().unwrap_or(0);
    let mut bytes = Vec::with_capacity(total as usize);
    while let Some(chunk) = response.chunk().await.map_err(|e| format!("downloading ffmpeg: {e}"))? {
        bytes.extend_from_slice(&chunk);
        if total > 0 {
            report(
                0.1 * bytes.len() as f64 / total as f64,
                format!("downloading ffmpeg ({} of {} MB)", bytes.len() >> 20, total >> 20),
            );
        }
    }
    let got = hex_string(&Sha256::digest(&bytes));
    if got != *sha {
        return Err(format!("the downloaded ffmpeg does not match its pinned checksum ({got})"));
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, &bytes).map_err(msg)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755)).map_err(msg)?;
    }
    std::fs::rename(&partial, &path).map_err(msg)?;
    Ok(path)
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `mix::loudness` off the async runtime.
async fn measure(ffmpeg: &Path, inputs: &[PathBuf], graph: String) -> CmdResult<Option<f64>> {
    let (ffmpeg, inputs) = (ffmpeg.to_path_buf(), inputs.to_vec());
    tokio::task::spawn_blocking(move || mix::loudness(&ffmpeg, &inputs, &graph)).await.map_err(msg)?
}

/// Removes the assembly's scratch folder however the join ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tauri::command]
pub fn list_episodes(state: State<AppState>) -> CmdResult<Vec<EpisodeView>> {
    with_project(&state, |p| {
        list(p)?
            .into_iter()
            .map(|file| {
                let video = episode_dir(p, &file.id)?.join(format!("{}.mp4", file.id));
                let video = video.is_file().then(|| format!("{EPISODES}/{}/{}.mp4", file.id, file.id));
                Ok(EpisodeView { file, video })
            })
            .collect()
    })
}

#[tauri::command]
pub fn create_episode(state: State<AppState>, title: String, chapters: Vec<String>) -> CmdResult<EpisodeFile> {
    let chapters = chapters.iter().map(|c| c.parse::<ChapterId>().map_err(msg)).collect::<CmdResult<Vec<_>>>()?;
    let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).map_err(msg)?;
    with_project(&state, |p| create(p, &title, chapters, now))
}

/// Deletes an episode and its assembled video. Its chapters and their parts stay.
#[tauri::command]
pub fn delete_episode(state: State<AppState>, episode: String) -> CmdResult<()> {
    crate::commands::ensure_idle(&state)?;
    with_project(&state, |p| {
        load_episode(p, &episode)?;
        std::fs::remove_dir_all(episode_dir(p, &episode)?).map_err(msg)
    })
}

/// Each part's state in playing order, given the part specs the app would render now (chapter id, `intro` or
/// `outro` -> EpisodeSpec).
#[tauri::command]
pub fn episode_parts(
    state: State<AppState>,
    episode: String,
    specs: serde_json::Map<String, Value>,
) -> CmdResult<Vec<(String, PartState)>> {
    with_project(&state, |p| {
        let file = load_episode(p, &episode)?;
        part_order(&file, &specs)
            .into_iter()
            .map(|key| {
                let spec = specs.get(&key).ok_or_else(|| format!("no part spec for {key}"))?;
                let state = part_state(p, &episode, &key, spec)?;
                Ok((key, state))
            })
            .collect()
    })
}

#[tauri::command]
pub fn brand_settings(app: AppHandle) -> CmdResult<BrandSettings> {
    load_brand(&app)
}

/// Makes `path` (an audio file) the brand music of every project, replacing the old track. The track is made into
/// a loop once, here: its intro and fade-out left out and its end blended into its start on the beat (`mix.rs`).
#[tauri::command]
pub async fn set_brand_music(app: AppHandle, path: String) -> CmdResult<BrandSettings> {
    let source = PathBuf::from(&path);
    let ext = source.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
    if !["mp3", "wav", "flac", "m4a", "aac", "ogg", "opus"].contains(&ext.as_str()) {
        return Err(format!("{path} is not an audio file this app can use (mp3, wav, flac, m4a, aac, ogg, opus)"));
    }
    let ffmpeg = ffmpeg(&app, |_, _| {}).await?;
    let dir = brand_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(msg)?;
    let name = "music.wav".to_owned();
    let partial = dir.join("music.partial.wav");
    let made = partial.clone();
    let made = tokio::task::spawn_blocking(move || mix::prepare_music(&ffmpeg, &source, &made)).await.map_err(msg)?;
    if let Err(e) = made {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("{path}: {e}"));
    }
    let mut settings = load_brand(&app)?;
    if let Some(old) = settings.music.take().filter(|old| *old != name) {
        let _ = std::fs::remove_file(dir.join(old));
    }
    std::fs::rename(&partial, dir.join(&name)).map_err(msg)?;
    settings.music = Some(name);
    save_brand(&app, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub fn set_brand_volume(app: AppHandle, music_db: f64) -> CmdResult<BrandSettings> {
    if !(-40.0..=0.0).contains(&music_db) {
        return Err("the music level is between -40 and 0 dB".into());
    }
    let mut settings = load_brand(&app)?;
    settings.music_db = music_db;
    save_brand(&app, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub fn set_sfx_volume(app: AppHandle, sfx_db: f64) -> CmdResult<BrandSettings> {
    if !(-20.0..=10.0).contains(&sfx_db) {
        return Err("the sound effects' level is between -20 and +10 dB".into());
    }
    let mut settings = load_brand(&app)?;
    settings.sfx_db = sfx_db;
    save_brand(&app, &settings)?;
    Ok(settings)
}

fn load_sfx(app: &AppHandle) -> CmdResult<(PathBuf, SfxLibrary)> {
    let dir = brand_dir(app)?.join(SFX_DIR);
    let file = dir.join("library.json");
    let text = std::fs::read_to_string(&file)
        .map_err(|e| format!("the sound effect library ({}) cannot be read: {e}", file.display()))?;
    let library = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok((dir, library))
}

/// Makes `path` (an audio file) the narrator's signature clip: converted once to a 48 kHz WAV in the brand folder,
/// so every episode closes on the identical recording.
#[tauri::command]
pub async fn set_brand_signature(app: AppHandle, path: String) -> CmdResult<BrandSettings> {
    let ffmpeg = ffmpeg(&app, |_, _| {}).await?;
    let dir = brand_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(msg)?;
    let partial = dir.join("signature.partial.wav");
    let (source, out) = (path.clone(), partial.clone());
    let made = tokio::task::spawn_blocking(move || {
        Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-i", &source, "-vn", "-ar", "48000", "-ac", "2", "-c:a", "pcm_s16le"])
            .arg(&out)
            .output()
    })
    .await
    .map_err(msg)?
    .map_err(|e| format!("ffmpeg did not start: {e}"))?;
    let seconds = if made.status.success() {
        wav_seconds(&partial)
    } else {
        Err(String::from_utf8_lossy(&made.stderr).trim().to_owned())
    };
    let seconds = match seconds {
        Ok(s) if s > 0.3 && s <= SIGNATURE_MAX_S => s,
        Ok(s) => {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("{path} is {s:.1} s long; a signature clip is up to {SIGNATURE_MAX_S:.0} s"));
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("{path}: {e}"));
        }
    };
    let name = "signature.wav".to_owned();
    std::fs::rename(&partial, dir.join(&name)).map_err(msg)?;
    let mut settings = load_brand(&app)?;
    settings.signature = Some(name);
    settings.signature_s = seconds;
    save_brand(&app, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub fn clear_brand_signature(app: AppHandle) -> CmdResult<BrandSettings> {
    let mut settings = load_brand(&app)?;
    if let Some(old) = settings.signature.take() {
        let _ = std::fs::remove_file(brand_dir(&app)?.join(old));
    }
    settings.signature_s = 0.0;
    save_brand(&app, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub fn clear_brand_music(app: AppHandle) -> CmdResult<BrandSettings> {
    let mut settings = load_brand(&app)?;
    if let Some(old) = settings.music.take() {
        let _ = std::fs::remove_file(brand_dir(&app)?.join(old));
    }
    save_brand(&app, &settings)?;
    Ok(settings)
}

/// Joins an episode's parts into `episodes/<id>/<id>.mp4` on this PC. `specs` are the part specs the app would render
/// now (chapter id, `intro` or `outro` -> EpisodeSpec): every part must match its own, so a stale part is never
/// joined, and the wrapper's parts are joined when there are specs for them. Progress arrives as
/// `assembly://progress` events.
#[tauri::command]
pub async fn assemble_episode(
    app: AppHandle,
    state: State<'_, AppState>,
    episode: String,
    specs: serde_json::Map<String, Value>,
) -> CmdResult<String> {
    let _job = begin_job(&state, Lane::Assembly, format!("{episode} assembly"))?;
    let report = |progress: f64, message: String| {
        let _ = app.emit("assembly://progress", Progress { episode: &episode, progress, message });
    };
    report(0.0, "checking the parts".into());
    let (dir, parts) = with_project(&state, |p| {
        let file = load_episode(p, &episode)?;
        let mut parts = Vec::new();
        for id in part_order(&file, &specs) {
            let spec = specs.get(&id).ok_or_else(|| format!("no part spec for {id}"))?;
            match part_state(p, &episode, &id, spec)? {
                PartState::Ready => {}
                PartState::Missing => {
                    return Err(format!("{id} has no part for {episode} yet; render the missing parts first"));
                }
                PartState::Outdated => {
                    return Err(format!("{id} changed since its part was rendered; render it again first"));
                }
            }
            let render_dir = part_dir(p, &episode, &id)?;
            let info = read_part(&render_dir).ok_or_else(|| format!("{id}: its part.json is unreadable"))?;
            parts.push((id, render_dir, info));
        }
        Ok((episode_dir(p, &episode)?, parts))
    })?;
    let first = &parts.first().ok_or("the episode has no chapters")?.2;
    if let Some((id, _, info)) =
        parts.iter().find(|(_, _, i)| (i.width, i.height, i.fps) != (first.width, first.height, first.fps))
    {
        return Err(format!(
            "{id}'s part is {}x{} at {} fps, the others {}x{} at {} fps; render it again first",
            info.width, info.height, info.fps, first.width, first.height, first.fps
        ));
    }
    let videos: Vec<PathBuf> = parts.iter().map(|(_, d, _)| d.join(format!("{PART}.mp4"))).collect();
    let soundtracks: Vec<PathBuf> = parts.iter().map(|(_, d, _)| d.join(format!("{PART}.wav"))).collect();
    let mut seconds = 0.0;
    for s in &soundtracks {
        seconds += wav_seconds(s)?;
    }
    let need = videos.iter().map(|v| std::fs::metadata(v).map(|m| m.len()).unwrap_or(0)).sum::<u64>() + SPACE_MARGIN;
    std::fs::create_dir_all(&dir).map_err(msg)?;
    let free = fs4::available_space(&dir).map_err(msg)?;
    if free < need {
        return Err(format!("the disk needs {} MB free for {episode}, and has {} MB", need >> 20, free >> 20));
    }

    let brand = load_brand(&app)?;
    let mut effects = Vec::new();
    let mut offset = 0.0;
    let mut library = None;
    for ((id, _, _), soundtrack) in parts.iter().zip(&soundtracks) {
        let spec = specs.get(id).ok_or_else(|| format!("no part spec for {id}"))?;
        let used = spec["beats"].as_array().is_some_and(|b| b.iter().any(|b| b["sfx"].is_string()));
        if used && library.is_none() {
            library = Some(load_sfx(&app)?);
        }
        if let Some((_, lib)) = &library {
            effects.extend(mix::part_effects(spec, offset, lib, brand.sfx_db).map_err(|e| format!("{id}: {e}"))?);
        }
        offset += wav_seconds(soundtrack)?;
    }
    let sfx_dir = library.map(|(dir, _)| dir).unwrap_or_default();

    let ffmpeg = ffmpeg(&app, &report).await?;
    let music_path =
        brand.music.as_ref().map(|m| brand_dir(&app).map(|d| d.join(m))).transpose()?.filter(|p| p.is_file());
    // Twice: the raw narration sets the gain into the compressor, the compressed one the make-up after it.
    report(0.1, "measuring the narration's loudness".into());
    let n = soundtracks.len();
    let mut voice = VoiceGain { before: 0.0, after: 0.0 };
    if let Some(lufs) = measure(&ffmpeg, &soundtracks, voice_shaped(0, n)).await? {
        voice.before = TARGET_LUFS - lufs;
        let compressed = voice_compressed(0, n, voice.before);
        voice.after = measure(&ffmpeg, &soundtracks, compressed).await?.map_or(0.0, |l| TARGET_LUFS - l);
    }
    // The music is measured as it plays, after its EQ, so `music_db` is what the viewer hears under the voice.
    let music_db = match &music_path {
        Some(path) => {
            let shaped = format!("[0:a]{MUSIC_SHAPE}");
            let lufs = measure(&ffmpeg, std::slice::from_ref(path), shaped).await?.unwrap_or(TARGET_LUFS);
            TARGET_LUFS + brand.music_db - lufs
        }
        None => 0.0,
    };
    // The signature ends a second before the episode, in the final hold, at the narration's loudness.
    let signature_path =
        brand.signature.as_ref().map(|s| brand_dir(&app).map(|d| d.join(s))).transpose()?.filter(|p| p.is_file());
    let signature = match signature_path {
        Some(path) => {
            let lufs = measure(&ffmpeg, std::slice::from_ref(&path), "[0:a]anull".into()).await?.unwrap_or(TARGET_LUFS);
            let at = (seconds - brand.signature_s - SIGNATURE_TAIL_S).max(0.0);
            Some((path, at, TARGET_LUFS - lufs))
        }
        None => None,
    };
    let scratch = Scratch(dir.join(".assembling"));
    let _ = std::fs::remove_dir_all(&scratch.0);
    std::fs::create_dir_all(&scratch.0).map_err(msg)?;
    let list = scratch.0.join("video.txt");
    std::fs::write(&list, videos.iter().map(|v| concat_line(v)).collect::<String>()).map_err(msg)?;
    let out = scratch.0.join(format!("{episode}.mp4"));
    let graph_file = scratch.0.join("graph.txt");
    let (args, graph) = join_args(
        &list,
        &soundtracks,
        voice,
        signature.as_ref().map(|(path, at, db)| Signature { path, at: *at, db: *db }),
        music_path.as_deref().map(|path| Music { path, db: music_db }),
        Effects { dir: &sfx_dir, list: &effects },
        seconds,
        &graph_file,
        &out,
    );
    std::fs::write(&graph_file, graph).map_err(msg)?;
    report(0.15, format!("joining {} parts ({:.0} min, {} sound effects)", parts.len(), seconds / 60.0, effects.len()));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<f64>();
    let run = tokio::task::spawn_blocking(move || -> CmdResult<()> {
        let mut child = Command::new(&ffmpeg)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ffmpeg did not start: {e}"))?;
        if let Some(stdout) = child.stdout.take() {
            // `-progress` writes key=value lines; out_time_us is how far the output has got.
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                    let _ = tx.send(us / 1e6);
                }
            }
        }
        let mut errors = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut errors);
        }
        let status = child.wait().map_err(msg)?;
        if status.success() { Ok(()) } else { Err(format!("ffmpeg failed ({status}): {}", errors.trim())) }
    });
    while let Some(done) = rx.recv().await {
        report(
            0.15 + 0.83 * (done / seconds.max(1.0)).clamp(0.0, 1.0),
            format!("joining: {:.0} of {:.0} min", done / 60.0, seconds / 60.0),
        );
    }
    run.await.map_err(msg)??;
    let video = dir.join(format!("{episode}.mp4"));
    std::fs::rename(&out, &video).map_err(msg)?;
    drop(scratch);
    report(1.0, format!("{EPISODES}/{episode}/{episode}.mp4"));
    Ok(format!("{EPISODES}/{episode}/{episode}.mp4"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with(chapters: u32) -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = Project::create(dir.path().join("p"), "Test").expect("project");
        for n in 1..=chapters {
            let pdf = dir.path().join(format!("{n}.pdf"));
            std::fs::write(&pdf, format!("%PDF-1.4 chapter {n}")).expect("pdf");
            chapter::import_chapter(&project, &pdf, None).expect("import");
        }
        (dir, project)
    }

    fn ids(list: &[u32]) -> Vec<ChapterId> {
        list.iter().map(|n| ChapterId::new(*n).expect("id")).collect()
    }

    #[test]
    fn an_episode_takes_chapters_in_story_order_with_none_skipped() {
        let (_dir, project) = project_with(5);
        assert!(
            create(&project, "Arc one", ids(&[1, 3]), "t".into())
                .expect_err("refused")
                .contains("ch_0003 does not come right after ch_0001")
        );
        assert!(create(&project, "Arc one", ids(&[2, 1]), "t".into()).is_err());
        assert!(
            create(&project, "Arc one", ids(&[7]), "t".into()).expect_err("refused").contains("not in this project")
        );
        assert!(create(&project, " ", ids(&[1]), "t".into()).is_err());
        let first = create(&project, "Arc one", ids(&[1, 2, 3]), "t".into()).expect("episode");
        assert_eq!(first.id, "ep_0001");
        assert!(
            create(&project, "Arc two", ids(&[3, 4]), "t".into())
                .expect_err("refused")
                .contains("ch_0003 is already in ep_0001")
        );
        let second = create(&project, "Arc two", ids(&[4, 5]), "t".into()).expect("episode");
        assert_eq!(second.id, "ep_0002");
        assert_eq!(list(&project).expect("list"), [first, second]);
    }

    #[test]
    fn a_part_is_current_only_while_its_spec_and_files_are_unchanged() {
        let (_dir, project) = project_with(2);
        let episode = create(&project, "Arc", ids(&[1, 2]), "t".into()).expect("episode");
        let id = ChapterId::new(1).expect("id");
        let root = project.paths().root();
        std::fs::create_dir_all(root.join("chapters/ch_0001/audio")).expect("dir");
        std::fs::write(root.join("chapters/ch_0001/audio/l_0001.wav"), b"one").expect("clip");
        let spec = serde_json::json!({
            "video": {"width": 1920, "height": 1080, "fps": 30},
            "panels": [],
            "beats": [{"audio": {"src": "chapters/ch_0001/audio/l_0001.wav"}}],
        });
        assert_eq!(part_state(&project, &episode.id, &id.to_string(), &spec).expect("state"), PartState::Missing);

        let render_dir = chapter::chapter_dir(&project, id).join("render");
        std::fs::create_dir_all(&render_dir).expect("dir");
        for f in ["part.mp4", "part.wav"] {
            std::fs::write(render_dir.join(f), b"x").expect("part");
        }
        write_part(&render_dir, &episode.id, &fingerprint(root, &spec).expect("fp"), &spec).expect("write");
        assert_eq!(part_state(&project, &episode.id, &id.to_string(), &spec).expect("state"), PartState::Ready);
        assert_eq!(part_state(&project, "ep_0009", &id.to_string(), &spec).expect("state"), PartState::Missing);

        // A new recording of the same line, or a different opening card, makes the part outdated.
        std::fs::write(root.join("chapters/ch_0001/audio/l_0001.wav"), b"two").expect("clip");
        assert_eq!(part_state(&project, &episode.id, &id.to_string(), &spec).expect("state"), PartState::Outdated);
        std::fs::write(root.join("chapters/ch_0001/audio/l_0001.wav"), b"one").expect("clip");
        let mut opened = spec.clone();
        opened["opening"] = serde_json::json!({"title": "Chapter 1"});
        assert_eq!(part_state(&project, &episode.id, &id.to_string(), &opened).expect("state"), PartState::Outdated);
    }

    #[test]
    fn the_wrapper_plays_around_the_chapters_and_keeps_its_parts_in_the_episode() {
        let (_dir, project) = project_with(2);
        let episode = create(&project, "Arc", ids(&[1, 2]), "t".into()).expect("episode");
        let spec = serde_json::json!({"video": {"width": 1920, "height": 1080, "fps": 30}, "panels": [], "beats": []});
        let mut specs = serde_json::Map::new();
        for key in ["outro", "ch_0002", "ch_0001"] {
            specs.insert(key.into(), spec.clone());
        }
        assert_eq!(part_order(&episode, &specs), ["ch_0001", "ch_0002", "outro"]);
        specs.insert("intro".into(), spec.clone());
        assert_eq!(part_order(&episode, &specs), ["intro", "ch_0001", "ch_0002", "outro"]);

        let intro = part_dir(&project, &episode.id, "intro").expect("dir");
        assert_eq!(intro, project.paths().root().join("episodes/ep_0001/intro"));
        assert_eq!(part_state(&project, &episode.id, "intro", &spec).expect("state"), PartState::Missing);
        std::fs::create_dir_all(&intro).expect("dir");
        for f in ["part.mp4", "part.wav"] {
            std::fs::write(intro.join(f), b"x").expect("part");
        }
        write_part(&intro, &episode.id, &fingerprint(project.paths().root(), &spec).expect("fp"), &spec)
            .expect("write");
        assert_eq!(part_state(&project, &episode.id, "intro", &spec).expect("state"), PartState::Ready);
        assert!(part_dir(&project, &episode.id, "nope").is_err());
    }

    #[test]
    fn wav_length_comes_from_the_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.wav");
        let data = vec![0u8; 48_000 * 2 * 3];
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&96_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(&path, wav).expect("wav");
        assert!((wav_seconds(&path).expect("seconds") - 3.0).abs() < 1e-9);
        std::fs::write(&path, b"not a wav").expect("junk");
        assert!(wav_seconds(&path).is_err());
    }

    #[test]
    fn concat_lines_quote_windows_paths_and_apostrophes() {
        assert_eq!(concat_line(Path::new(r"C:\Videos\Kim's\part.mp4")), "file 'C:/Videos/Kim'\\''s/part.mp4'\n");
    }

    const GAIN: VoiceGain = VoiceGain { before: 7.5, after: 0.8 };

    #[test]
    fn the_join_copies_video_and_ducks_the_music_under_the_narration() {
        let parts = [PathBuf::from("a.wav"), PathBuf::from("b.wav")];
        let none = Effects { dir: Path::new("sfx"), list: &[] };
        let (plain, graph) = join_args(
            Path::new("v.txt"),
            &parts,
            GAIN,
            None,
            None,
            none,
            600.0,
            Path::new("g.txt"),
            Path::new("out.mp4"),
        );
        // Shaped, raised into the compressor, made up after it and limited.
        let voice = format!(
            "[s1][s2]concat=n=2:v=0:a=1,{VOICE_SHAPE},volume=7.50dB,{VOICE_COMPRESSOR},volume=0.80dB,{VOICE_LIMITER}[voice]"
        );
        assert!(graph.contains(&voice), "{graph}");
        assert!(graph.ends_with(&format!("[voice]anull,{LIMITER}[a]")), "{graph}");
        assert!(plain.windows(2).any(|w| w == ["-c:v", "copy"]));
        assert!(plain.windows(2).any(|w| w == ["-t", "600.000"]));
        assert!(plain.windows(2).any(|w| w == ["-filter_complex_script", "g.txt"]));
        assert!(!plain.contains(&"-stream_loop".to_string()));

        let effects = [
            Effect { file: "02_punch.wav".into(), at: 12.5, db: -3.0, filters: vec![] },
            Effect {
                file: "01_whoosh.wav".into(),
                at: 30.0,
                db: -6.0,
                filters: vec!["apad=pad_dur=0.3", "aecho=1:1:40:0.3"],
            },
            Effect { file: "02_punch.wav".into(), at: 61.25, db: -3.0, filters: vec![] },
        ];
        let (music, graph) = join_args(
            Path::new("v.txt"),
            &parts,
            GAIN,
            None,
            Some(Music { path: Path::new("m.wav"), db: -12.4 }),
            Effects { dir: Path::new("sfx"), list: &effects },
            600.0,
            Path::new("g.txt"),
            Path::new("out.mp4"),
        );
        for part in [
            // Each sound file is read once and placed on every line it plays on.
            "[3:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asplit=2[x3_0][x3_1]",
            "[x3_1]volume=-3.0dB,adelay=delays=61250:all=1[d3_1]",
            "[x4_0]apad=pad_dur=0.3,aecho=1:1:40:0.3,volume=-6.0dB,adelay=delays=30000:all=1[d4_0]",
            "amix=inputs=3:duration=longest:normalize=0[fx]",
            // The music is the next input, EQ'd, levelled and ducked by the voice and the effects.
            "[5:a]aresample",
            MUSIC_SHAPE,
            "volume=-12.40dB",
            "afade=t=out:st=597.000:d=3",
            // The effects key the ducking 6 dB down.
            "[fxk]volume=-6dB[fxkey]",
            "[vkey][fxkey]amix=inputs=2:duration=first:normalize=0[key]",
            &format!("[bed][key]{DUCK}[duck]"),
            &format!("[say][duck][fxmix]amix=inputs=3:duration=first:normalize=0,{LIMITER}[a]"),
        ] {
            assert!(graph.contains(part), "{part} missing from {graph}");
        }
        let inputs: Vec<&String> = music.iter().skip(1).step_by(1).collect();
        assert!(inputs.windows(2).any(|w| w[0] == "-i" && w[1] == "sfx/02_punch.wav"));
        assert!(music.windows(4).any(|w| w == ["-stream_loop", "-1", "-i", "m.wav"]));
        assert_eq!(music.last().map(String::as_str), Some("out.mp4"));
    }

    #[test]
    fn the_signature_joins_the_narration_in_the_final_hold() {
        let parts = [PathBuf::from("a.wav")];
        let none = Effects { dir: Path::new("sfx"), list: &[] };
        let sig = Signature { path: Path::new("signature.wav"), at: 595.5, db: -2.25 };
        let (args, graph) = join_args(
            Path::new("v.txt"),
            &parts,
            GAIN,
            Some(sig),
            Some(Music { path: Path::new("m.wav"), db: -12.0 }),
            none,
            600.0,
            Path::new("g.txt"),
            Path::new("out.mp4"),
        );
        assert!(args.windows(2).any(|w| w == ["-i", "signature.wav"]));
        for part in [
            &format!("{VOICE_LIMITER}[said];[2:a]"),
            "volume=-2.25dB,adelay=delays=595500:all=1[sig]",
            "[said][sig]amix=inputs=2:duration=first:normalize=0[voice]",
            // The music comes after it and still dips under the voice, the signature included.
            "[3:a]aresample",
            "[voice]asplit=2[say][key]",
        ] {
            assert!(graph.contains(part), "{part} missing from {graph}");
        }
    }

    #[test]
    fn every_pinned_ffmpeg_has_a_checksum() {
        for (platform, asset, sha) in FFMPEG_BUILDS {
            assert!(
                asset.starts_with("ffmpeg-") && sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
                "{platform}"
            );
        }
    }
}
