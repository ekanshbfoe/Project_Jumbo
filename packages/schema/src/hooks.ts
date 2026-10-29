import { z } from "zod";
import { ChapterId, PanelId, SchemaVersion } from "./common";

/**
 * Hooks (ADR 0008): Gemini writes one chapter at a time, so each chapter offers its own hook material and the
 * episode picks from it locally, with no model involved. A hook is a moment of the chapter: the panels that show it
 * without giving the reveal away, a 10-15 s cold open told over them, and a one-sentence tease for the end of the
 * episode before. Low-spoiler hooks may open an episode; high-spoiler ones are only ever teased.
 */
export const HookKind = z.enum(["danger", "reveal", "irony", "question", "romance"]);

export const Hook = z
  .object({
    /** H1, H2 … in the order Gemini wrote them. */
    id: z.string().regex(/^H\d+$/),
    panel_ids: z.array(PanelId).min(1).max(3),
    /** How strongly the moment pulls a viewer in, 1 to 5. */
    strength: z.int().min(1).max(5),
    kind: HookKind,
    /** low: safe to show at the start of an episode; high: only for a tease. */
    spoiler: z.enum(["low", "high"]),
    /** The cold open: two or three Hindi sentences, present tense, holding back one answer. */
    cold: z.string().min(1).max(600),
    /** One Hindi sentence in the future tense, said at the end of the episode before. */
    tease: z.string().max(300).default(""),
  })
  .meta({ id: "Hook" });

/** A chapter's hook material, saved next to its script (script/hooks.json). */
export const ChapterHooks = z
  .object({
    schema_version: SchemaVersion,
    chapter_id: ChapterId,
    hooks: z.array(Hook).max(3),
    /** Two short Hindi sentences recapping the story before this chapter, for an episode that starts here. */
    recap_in: z.string().max(600).default(""),
  })
  .meta({ id: "ChapterHooks" });

export type HookKind = z.infer<typeof HookKind>;
export type Hook = z.infer<typeof Hook>;
export type ChapterHooks = z.infer<typeof ChapterHooks>;

/** The narrator's closing line on every video and channel, word for word: an audio logo, never translated. */
export const SIGNATURE = "I am your dost or host, Mayu, signing off.";

/**
 * Fixed lines from the cold open back to the start of the story, in Mayu's voice. Fixed text fits any chapter and
 * cannot go wrong; episodes take them in turn so back-to-back episodes do not repeat one.
 */
export const BRIDGES = [
  "पर यह सब यहाँ से शुरू नहीं हुआ था। शुरुआत से चलते हैं।",
  "यहाँ तक बात कैसे पहुँची, वह मैं आपको शुरू से बताती हूँ।",
  "पर इस पल तक पहुँचने से पहले, थोड़ा पीछे चलते हैं।",
  "यह कैसे हुआ, उसके लिए हमें थोड़ा पीछे जाना होगा।",
  "पर कहानी इससे पहले शुरू होती है। चलिए, वहीं से।",
] as const;

export interface PlannedHook {
  chapter: string;
  hook: Hook;
}

/** How an episode opens and closes, before anything is voiced. */
export interface EpisodePlan {
  /** The cold open, or null when no chapter of the episode offers a low-spoiler hook. */
  cold: PlannedHook | null;
  /** Said after the cold open, back to the start of the story. */
  bridge: string | null;
  /** The recap for viewers starting here: the first chapter's RECAP_IN, when the story did not start with it. */
  recap: string | null;
  /** The tease of the chapter after the episode, said after its last line. */
  tease: PlannedHook | null;
  signature: string;
  /** Why something is missing, for the Episodes screen. */
  notes: string[];
}

export interface PlanInput {
  /** The episode's chapters in story order, each with its hooks (null when its answer had none). */
  chapters: { id: string; hooks: ChapterHooks | null }[];
  /** The episode's place among the project's episodes, from 0: picks the bridge line. */
  index: number;
  /** True when the episode's first chapter is the first of the whole story (no recap needed). */
  startsStory: boolean;
  /** The chapter right after the episode's last one, if it has a script yet. */
  next: { id: string; hooks: ChapterHooks | null } | null;
}

const strongest = (list: PlannedHook[]) =>
  // Ties go to the later chapter, so the payoff comes as late in the hour as possible.
  list.reduce<PlannedHook | null>((best, h) => (!best || h.hook.strength >= best.hook.strength ? h : best), null);

/**
 * Picks the episode's opening and ending (ADR 0008). The cold open is the strongest low-spoiler hook of a chapter
 * after the first, so it pays off inside the episode; a one-chapter episode may open on its own hook. The ending
 * teases the strongest hook of the next chapter, whatever its spoiler level, since a tease never shows the reveal.
 */
export function planEpisode({ chapters, index, startsStory, next }: PlanInput): EpisodePlan {
  const notes: string[] = [];
  const pool = chapters.length > 1 ? chapters.slice(1) : chapters;
  const low = pool.flatMap((c) => (c.hooks?.hooks ?? []).filter((h) => h.spoiler === "low").map((hook) => ({ chapter: c.id, hook })));
  const cold = strongest(low);
  if (!cold) {
    const without = pool.filter((c) => !c.hooks?.hooks.length).map((c) => c.id);
    notes.push(
      without.length
        ? `No cold open: ${without.join(", ")} ${without.length === 1 ? "has" : "have"} no hooks (paste an answer with a HOOKS section)`
        : "No cold open: every hook here gives too much away (spoiler high)",
    );
  }
  const first = chapters[0];
  const recap = !startsStory && first?.hooks?.recap_in ? first.hooks.recap_in : null;
  if (!startsStory && first && !recap) notes.push(`No recap: ${first.id} has no RECAP_IN, so viewers start mid-story`);
  const teases = (next?.hooks?.hooks ?? []).filter((h) => h.tease).map((hook) => ({ chapter: next!.id, hook }));
  const tease = strongest(teases);
  if (!next) notes.push("No tease at the end: the chapter after this episode has no script yet");
  else if (!tease) notes.push(`No tease at the end: ${next.id} has no hook with a TEASE`);
  return {
    cold,
    bridge: cold ? BRIDGES[((index % BRIDGES.length) + BRIDGES.length) % BRIDGES.length]! : null,
    recap,
    tease,
    signature: SIGNATURE,
    notes,
  };
}
