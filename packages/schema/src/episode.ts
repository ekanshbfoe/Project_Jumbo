import { z } from "zod";
import { AssetPath, ChapterId, HexColor, LineId, PanelId, SchemaVersion } from "./common";
import { Panel, type PanelsDoc } from "./panels";
import { MULTI_PANEL_PRESETS, PresetSpec, TransitionSpec } from "./presets";
import { Emotion, type ScriptDoc } from "./script";

export const AudioClip = z.object({
  line_id: LineId,
  src: AssetPath,
  /** Clip length in seconds. */
  duration: z.number().positive(),
});

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

export const VideoFormat = z.object({
  width: z.number().int().positive().default(1920),
  height: z.number().int().positive().default(1080),
  fps: z.number().int().min(12).max(60).default(30),
  background: HexColor.default("#0b0b10"),
});

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
});

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
});

/** Everything Remotion needs to render one chapter. Consumed directly as composition input props. */
export const EpisodeSpec = z
  .object({
    schema_version: SchemaVersion,
    chapter_id: ChapterId,
    title: z.string().default(""),
    video: VideoFormat.prefault({}),
    captions: z.boolean().default(true),
    /** Mask out speech bubbles when no inpainted `clean` image exists. */
    hide_bubbles: z.boolean().default(false),
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
    });
  });

export type AudioClip = z.infer<typeof AudioClip>;
export type AudioManifest = z.infer<typeof AudioManifest>;
export type VideoFormat = z.infer<typeof VideoFormat>;
export type EpisodePanel = z.infer<typeof EpisodePanel>;
export type Beat = z.infer<typeof Beat>;
export type EpisodeSpec = z.infer<typeof EpisodeSpec>;

export interface BuildEpisodeOptions {
  title?: string;
  video?: Partial<VideoFormat>;
  /** Silence after each narration clip, in seconds. */
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
  const used = new Set(script.lines.flatMap((l) => l.panel_ids));

  return EpisodeSpec.parse({
    schema_version: 1,
    chapter_id: script.chapter_id,
    title: options.title ?? script.title ?? "",
    video: options.video ?? {},
    hide_bubbles: options.hideBubbles ?? false,
    panels: panelsDoc.panels.filter((p) => used.has(p.id)),
    beats: script.lines.map((line) => {
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
        duration: Math.max(minBeat, spoken + padding),
      };
    }),
  });
}
