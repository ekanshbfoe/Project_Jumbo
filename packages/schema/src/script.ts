import { z } from "zod";
import { ChapterId, LineId, PanelId, SchemaVersion, VoiceId } from "./common";
import { MULTI_PANEL_PRESETS, PresetSpec, TransitionSpec } from "./presets";

export const Emotion = z.enum([
  "calm",
  "tense",
  "action",
  "shock",
  "sad",
  "comedic",
  "romantic",
  "mysterious",
  "triumphant",
]);

/** Delivery hints for the voice engine. */
export const VoiceDirection = z.object({
  /** Who speaks the line: the narrator, or a character's voice from the project's voice library. */
  speaker: VoiceId.default("narrator"),
  /** Free-form tone tag, e.g. "whispered", "excited", "menacing". */
  tone: z.string().min(1).max(40).default("neutral"),
  pace: z.enum(["slow", "normal", "fast"]).default("normal"),
  intensity: z.number().min(0).max(1).default(0.5),
}).meta({ id: "VoiceDirection" });

/**
 * A character's name card, shown once in the whole series: on the line where the character first appears (ADR 0007).
 * Gemini marks the line; the app keeps the card only when no earlier chapter's story memory knows the character.
 */
export const CharacterCard = z.object({
  name: z.string().min(1).max(60),
  /** Short English role from the CHARACTERS list, e.g. "the transfer student". */
  role: z.string().max(80).default(""),
}).meta({ id: "CharacterCard" });

/**
 * The brand sound effects (ADR 0007): the 15 sounds of the locked library, one per line at most. A line's sound plays
 * as its first panel appears; it is mixed in when the episode is assembled, so changing it never renders again.
 */
export const SoundEffect = z.enum([
  "whoosh",
  "punch",
  "impact",
  "slash",
  "clash",
  "explosion",
  "boom",
  "riser",
  "heartbeat",
  "shatter",
  "power",
  "system",
  "footsteps",
  "slam",
  "comedic",
]);

export const ScriptLine = z
  .object({
    id: LineId,
    panel_ids: z.array(PanelId).min(1).max(3),
    /** Narration spoken over the beat. Scripts for voicing are Devanagari or Mixed Hinglish (scriptProblems). */
    narration: z.string().min(1).max(2000),
    emotion: Emotion,
    voice: VoiceDirection.prefault({}),
    preset: PresetSpec,
    transition_in: TransitionSpec.prefault({}),
    card: CharacterCard.nullable().default(null),
    sfx: SoundEffect.nullable().default(null),
  })
  .superRefine((line, ctx) => {
    const multi = MULTI_PANEL_PRESETS.has(line.preset.name);
    if (multi && line.panel_ids.length < 2) {
      ctx.addIssue({ code: "custom", path: ["panel_ids"], message: `${line.preset.name} needs 2-3 panels` });
    }
    if (!multi && line.panel_ids.length !== 1) {
      ctx.addIssue({ code: "custom", path: ["panel_ids"], message: `${line.preset.name} takes exactly 1 panel` });
    }
  })
  .meta({ id: "ScriptLine" });

export const ScriptDoc = z
  .object({
    schema_version: SchemaVersion,
    chapter_id: ChapterId,
    title: z.string().max(200).optional(),
    lines: z.array(ScriptLine).min(1),
  })
  .superRefine((doc, ctx) => {
    const ids = new Set<string>();
    doc.lines.forEach((line, i) => {
      if (ids.has(line.id)) ctx.addIssue({ code: "custom", path: ["lines", i, "id"], message: `duplicate line id ${line.id}` });
      ids.add(line.id);
    });
  });

export type Emotion = z.infer<typeof Emotion>;
export type CharacterCard = z.infer<typeof CharacterCard>;
export type SoundEffect = z.infer<typeof SoundEffect>;
export type VoiceDirection = z.infer<typeof VoiceDirection>;
export type ScriptLine = z.infer<typeof ScriptLine>;
export type ScriptDoc = z.infer<typeof ScriptDoc>;
