import { z } from "zod";
import { AssetPath, ChapterId, HexColor, LineId, PanelId, SchemaVersion } from "./common";
import { Panel, type PanelsDoc } from "./panels";
import { MULTI_PANEL_PRESETS, PresetSpec, TransitionSpec } from "./presets";
import { SocialPromoSettings } from "./promo";
import { Emotion, type ScriptDoc } from "./script";

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
  bubbles: true,
}).meta({ id: "EpisodePanel" });

export const Beat = z.object({
  line_id: LineId,
  panel_ids: z.array(PanelId).min(1).max(3),
  narration: z.string().min(1),
  emotion: Emotion,
  preset: PresetSpec,
  transition_in: TransitionSpec.prefault({}),
  audio: AudioClip.omit({ line_id: true }).nullable(),
  /** Time on screen in seconds. Never shorter than the narration audio. */
  duration: z.number().positive(),
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
  /** Shortest a beat may be, in seconds. */
  minBeat?: number;
  /** Speaking rate used to size beats that have no audio yet (preview before voicing). */
  wordsPerSecond?: number;
  hideBubbles?: boolean;
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
  const { padding = 0.35, minBeat = 1.5, wordsPerSecond = 2.6 } = options;
  const clips = new Map(audio?.clips.map((c) => [c.line_id, c]));
  const { fps } = VideoFormat.parse(options.video ?? {});
  const used = new Set(script.lines.flatMap((l) => l.panel_ids));

  return EpisodeSpec.parse({
    schema_version: 1,
    chapter_id: script.chapter_id,
    title: options.title ?? script.title ?? "",
    video: options.video ?? {},
    hide_bubbles: options.hideBubbles ?? false,
    panels: panelsDoc.panels.filter((p) => used.has(p.id)),
    beats: script.lines.map((line, i) => {
      const clip = clips.get(line.id);
      const spoken = clip ? clip.duration : line.narration.split(/\s+/).length / wordsPerSecond;
      return {
        line_id: line.id,
        panel_ids: line.panel_ids,
        narration: line.narration,
        emotion: line.emotion,
        preset: line.preset,
        transition_in: line.transition_in,
        audio: clip ? { src: clip.src, duration: clip.duration } : null,
        duration: beatLength(Math.max(minBeat, spoken + padding + overlapOf(script.lines[i + 1])), fps),
      };
    }),
  });
}

/** How far a line's transition reaches back into the beat before it, in seconds. */
function overlapOf(line?: ScriptDoc["lines"][number]): number {
  return !line || line.transition_in.name === "cut" ? 0 : line.transition_in.duration;
}

/** Seconds rounded up to a whole frame, so the timeline's frame rounding never cuts the end of a clip. */
function beatLength(seconds: number, fps: number): number {
  return Math.ceil(seconds * fps - 1e-6) / fps;
}
