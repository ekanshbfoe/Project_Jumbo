import { z } from "zod";
import { AssetPath, ChapterId, HexColor, LineId, PanelId, SchemaVersion } from "./common";
import { Panel, type PanelsDoc } from "./panels";
import { MULTI_PANEL_PRESETS, PresetSpec, TransitionSpec } from "./presets";
import { SocialPromoSettings } from "./promo";
import { CharacterCard, Emotion, SoundEffect, type ScriptDoc } from "./script";

export const AudioClip = z.object({
  line_id: LineId,
  src: AssetPath,
  /** Clip length in seconds. */
  duration: z.number().positive(),
}).meta({ id: "AudioClip" });

export const AudioManifest = z
  .object({
    schema_version: SchemaVersion,
    chapter_id: ChapterId,
    sample_rate: z.number().int().positive(),
    clips: z.array(AudioClip),
  })
  .superRefine((m, ctx) => {
    const ids = new Set<string>();
    m.clips.forEach((clip, i) => {
      if (ids.has(clip.line_id)) ctx.addIssue({ code: "custom", path: ["clips", i, "line_id"], message: `duplicate clip for ${clip.line_id}` });
      ids.add(clip.line_id);
    });
  });

/** Permanent channel mark in the bottom-right corner. */
export const WatermarkSettings = z.object({
  enabled: z.boolean().default(true),
  text: z.string().min(1).max(40).default("Project Jumbo"),
  opacity: z.number().min(0.05).max(1).default(0.55),
}).meta({ id: "WatermarkSettings" });

/** Motion layered over every beat's blurred backdrop. */
export const AmbientEffect = z.enum(["none", "dust", "smoke", "city_bokeh"]);

export const VideoFormat = z.object({
  width: z.number().int().positive().default(1920),
  height: z.number().int().positive().default(1080),
  fps: z.number().int().min(12).max(60).default(30),
  background: HexColor.default("#0b0b10"),
}).meta({ id: "VideoFormat" });

/** The parts of a detected panel the renderer needs. */
export const EpisodePanel = Panel.pick({
  id: true,
  page: true,
  page_size: true,
  bbox: true,
  polygon: true,
  crop: true,
  clean: true,
  backdrop: true,
  bubbles: true,
}).meta({ id: "EpisodePanel" });

export const Beat = z.object({
  line_id: LineId,
  panel_ids: z.array(PanelId).min(1).max(3),
  narration: z.string().min(1),
  emotion: Emotion,
  preset: PresetSpec,
  transition_in: TransitionSpec.prefault({}),
  /** The narration played during this beat: `duration` seconds of the clip, starting `from` seconds into it (a line
   *  whose panels are shown one after another plays one clip across several beats). */
  audio: AudioClip.omit({ line_id: true })
    .extend({ from: z.number().min(0).default(0) })
    .nullable(),
  /** Time on screen in seconds. Never shorter than the narration audio. */
  duration: z.number().positive(),
  /** A character's first-appearance name card, shown as the beat starts. */
  card: CharacterCard.nullable().default(null),
  /** The line's sound effect, played as the beat starts (mixed in at assembly, not in the render). */
  sfx: SoundEffect.nullable().default(null),
}).meta({ id: "Beat" });

/** Everything Remotion needs to render one chapter. Consumed directly as composition input props. */
export const EpisodeSpec = z
  .object({
    schema_version: SchemaVersion,
    chapter_id: ChapterId,
    title: z.string().default(""),
    video: VideoFormat.prefault({}),
    /** Mask out speech bubbles when no inpainted `clean` image exists. */
    hide_bubbles: z.boolean().default(false),
    ambient_effect: AmbientEffect.default("dust"),
    watermark: WatermarkSettings.prefault({}),
    social_promo: SocialPromoSettings.prefault({}),
    panels: z.array(EpisodePanel).min(1),
    beats: z.array(Beat).min(1),
  })
  .superRefine((spec, ctx) => {
    const panelIds = new Set(spec.panels.map((p) => p.id));
    spec.beats.forEach((beat, i) => {
      for (const id of beat.panel_ids) {
        if (!panelIds.has(id)) ctx.addIssue({ code: "custom", path: ["beats", i, "panel_ids"], message: `unknown panel ${id}` });
      }
      const multi = MULTI_PANEL_PRESETS.has(beat.preset.name);
      if (multi !== beat.panel_ids.length > 1) {
        ctx.addIssue({ code: "custom", path: ["beats", i, "panel_ids"], message: `wrong panel count for ${beat.preset.name}` });
      }
      if (beat.audio && beat.duration < beat.audio.duration) {
        ctx.addIssue({ code: "custom", path: ["beats", i, "duration"], message: "beat is shorter than its narration audio" });
      }
      const prev = spec.beats[i - 1];
      const overlap = beat.transition_in.name === "cut" ? 0 : beat.transition_in.duration;
      if (prev && overlap >= Math.min(prev.duration, beat.duration)) {
        ctx.addIssue({ code: "custom", path: ["beats", i, "transition_in"], message: "transition is longer than the beats it joins" });
      }
      // The next beat's audio starts when its transition does, so two voices would overlap.
      if (prev?.audio && prev.duration - overlap < prev.audio.duration) {
        ctx.addIssue({ code: "custom", path: ["beats", i, "transition_in"], message: "transition starts before the previous narration ends" });
      }
    });
  });

export type AudioClip = z.infer<typeof AudioClip>;
export type AudioManifest = z.infer<typeof AudioManifest>;
export type AmbientEffect = z.infer<typeof AmbientEffect>;
export type WatermarkSettings = z.infer<typeof WatermarkSettings>;
export type VideoFormat = z.infer<typeof VideoFormat>;
export type EpisodePanel = z.infer<typeof EpisodePanel>;
export type Beat = z.infer<typeof Beat>;
export type EpisodeSpec = z.infer<typeof EpisodeSpec>;

export interface BuildEpisodeOptions {
  title?: string;
  video?: Partial<VideoFormat>;
  /** Silence after each narration clip, in seconds, before the next beat's transition begins. */
  padding?: number;
  /** Longest a transition between beats may be, in seconds: quick cuts keep a recap moving. */
  maxTransition?: number;
  /** Shortest a beat may be, in seconds. */
  minBeat?: number;
  /** Speaking rate used to size beats that have no audio yet (preview before voicing). */
  wordsPerSecond?: number;
  hideBubbles?: boolean;
  /** Show the panels of a multi-panel line one after another, each filling the frame (default true). */
  sequential?: boolean;
  /** Overrides for the follow card (an episode part shows it early only in the first part, at the end only in the last). */
  promo?: Partial<z.input<typeof SocialPromoSettings>>;
  /** Seconds the last panel stays on screen after the narration ends (an episode's last part), so the music can fade out. */
  endHold?: number;
}

/** A panel this many times taller than wide is "tall": two of them may share the frame side by side. */
const TALL = 1.3;
/** Height over width of a panel as shown. */
const aspect = (p: EpisodePanel) => (p.bbox.h * p.page_size.height) / Math.max(1e-6, p.bbox.w * p.page_size.width);
/** Fade between the panels of one line, in seconds. */
const STEP_FADE = 0.25;
/** A line too short to give each of its panels this long stays one beat with its panels together. */
const MIN_STEP = 1.2;

/** A camera move that suits a panel's shape: tall panels scroll down, wide ones pan across, the rest push in. */
function moveFor(panel: EpisodePanel, n: number): PresetSpec {
  const a = aspect(panel);
  if (a >= 1.5) return PresetSpec.parse({ name: "pan", params: { direction: "down", distance: 0.2 } });
  if (a <= 0.55) return PresetSpec.parse({ name: "pan", params: { direction: "right", distance: 0.12 } });
  return PresetSpec.parse(n % 2 === 0 ? { name: "zoom", params: { from: 1, to: 1.12 } } : { name: "pan", params: { direction: "right", distance: 0.06 } });
}

/**
 * One beat as shown: a multi-panel beat becomes one beat per panel, one after another, sharing the line's clip (each
 * plays the next part of it) and its screen time, with a short fade between them. Two tall panels stay side by side,
 * and so does a line too short to step through.
 */
function stepThrough(beat: z.input<typeof Beat>, panels: ReadonlyMap<string, EpisodePanel>, fps: number): z.input<typeof Beat>[] {
  const shown = beat.panel_ids.map((id) => panels.get(id)).filter((p): p is EpisodePanel => p !== undefined);
  const n = shown.length;
  const together = n === 2 && shown.every((p) => aspect(p) >= TALL);
  const sideways = beat.preset.name === "pan" && (beat.preset.params?.direction ?? "right") !== "up" && beat.preset.params?.direction !== "down";
  if (n === 1 && aspect(shown[0]!) >= 1.5 && (beat.preset.name === "hold" || beat.preset.name === "zoom" || sideways)) {
    // A still, a push or a sideways pan on a tall panel shows it whole and small: read it top to bottom instead.
    // ({pan} with no direction is a pan to the right.)
    return [{ ...beat, preset: moveFor(shown[0]!, 0) }];
  }
  if (n < 2 || together || beat.duration / n < MIN_STEP) return [beat];
  // The steps overlap by the fade, so they add up to the line's own length on the timeline.
  const total = beat.duration + (n - 1) * STEP_FADE;
  const clip = beat.audio;
  let used = 0;
  let heard = 0;
  return shown.map((panel, k) => {
    const last = k === n - 1;
    const duration = last ? beatLength(total - used, fps) : beatLength(total / n, fps);
    used += duration;
    const room = last ? duration : duration - STEP_FADE;
    const length = clip ? Math.min(room - 1e-6, clip.duration - heard) : 0;
    const audio = clip && length > 0.01 ? { src: clip.src, duration: length, from: heard } : null;
    heard += Math.max(0, length);
    return {
      ...beat,
      panel_ids: [panel.id],
      preset: moveFor(panel, k),
      transition_in: k === 0 ? beat.transition_in : { name: "fade", duration: STEP_FADE },
      // The card belongs to the line: it shows once, with its first panel.
      card: k === 0 ? beat.card : null,
      sfx: k === 0 ? beat.sfx : null,
      audio,
      duration,
    };
  });
}

/**
 * Joins an approved script, its panels and (optionally) the voiced audio into a render-ready EpisodeSpec.
 * Without audio, beat lengths are estimated from the narration so the motion can be previewed early.
 */
export function buildEpisodeSpec(
  script: ScriptDoc,
  panelsDoc: PanelsDoc,
  audio?: AudioManifest,
  options: BuildEpisodeOptions = {},
): EpisodeSpec {
  const { padding = 0.12, maxTransition = 0.3, minBeat = 1.5, wordsPerSecond = 2.6, sequential = true } = options;
  const transition = (line?: ScriptDoc["lines"][number]) =>
    line && { ...line.transition_in, duration: Math.min(line.transition_in.duration, maxTransition) };
  const clips = new Map(audio?.clips.map((c) => [c.line_id, c]));
  const { fps } = VideoFormat.parse(options.video ?? {});
  const used = new Set(script.lines.flatMap((l) => l.panel_ids));
  const byId = new Map(panelsDoc.panels.map((p) => [p.id, EpisodePanel.parse(p)]));

  const beats = script.lines.flatMap((line, i) => {
    const clip = clips.get(line.id);
    const spoken = clip ? clip.duration : line.narration.split(/\s+/).length / wordsPerSecond;
    const beat = {
      line_id: line.id,
      panel_ids: line.panel_ids,
      narration: line.narration,
      emotion: line.emotion,
      preset: line.preset,
      transition_in: transition(line),
      audio: clip ? { src: clip.src, duration: clip.duration, from: 0 } : null,
      card: line.card,
      sfx: line.sfx,
      duration: beatLength(Math.max(minBeat, spoken + padding + overlapOf(transition(script.lines[i + 1]))), fps),
    };
    return sequential ? stepThrough(beat, byId, fps) : [beat];
  });
  const last = beats.at(-1);
  if (last && options.endHold) last.duration = beatLength(last.duration + options.endHold, fps);

  return EpisodeSpec.parse({
    schema_version: 1,
    chapter_id: script.chapter_id,
    title: options.title ?? script.title ?? "",
    video: options.video ?? {},
    hide_bubbles: options.hideBubbles ?? false,
    social_promo: options.promo ?? {},
    panels: panelsDoc.panels.filter((p) => used.has(p.id)),
    beats,
  });
}

/** How far a transition reaches back into the beat before it, in seconds. */
function overlapOf(t?: { name: string; duration: number }): number {
  return !t || t.name === "cut" ? 0 : t.duration;
}

/** Seconds rounded up to a whole frame, so the timeline's frame rounding never cuts the end of a clip. */
function beatLength(seconds: number, fps: number): number {
  return Math.ceil(seconds * fps - 1e-6) / fps;
}
