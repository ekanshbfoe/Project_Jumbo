import type { EpisodePlan } from "./hooks";
import { PanelsDoc, type Panel } from "./panels";
import { PresetSpec } from "./presets";
import { ScriptDoc, type Emotion } from "./script";

/**
 * The episode wrapper (ADR 0008): the planned cold open, bridge and recap before the first chapter (the intro part)
 * and the tease after the last (the outro part), as one script voiced in one job and the panels it plays, taken from
 * whichever chapters they show. The narrator's voice follows each line's emotion: the cold open and the bridge are
 * mysterious (the serious voice), the recap and the tease calm (the bright one).
 */
export type WrapperPart = "intro" | "outro";

export interface WrapperInput {
  plan: EpisodePlan;
  /** The episode's first chapter: the wrapper's script and panels are filed under its id. */
  first: string;
  /**
   * Panels by chapter id: the chapters of the cold open and the tease, the first chapter, and the chapter before the
   * episode when there is a recap.
   */
  panels: Record<string, PanelsDoc | null | undefined>;
  /** The chapter right before the episode: its last panels play under the recap. */
  previous: string | null;
}

export interface Wrapper {
  script: ScriptDoc;
  panels: PanelsDoc;
  /** Each part's line ids, in order; a part with no lines is left out of the episode. */
  parts: Record<WrapperPart, string[]>;
}

/** Panels of the chapter before the episode shown under the recap: the moment the story stopped. */
const RECAP_PANELS = 3;

interface Draft {
  part: WrapperPart;
  panel_ids: string[];
  narration: string;
  emotion: Emotion;
  tone: string;
}

const inOrder = (doc: PanelsDoc) => [...doc.panels].sort((a, b) => a.order - b.order);

/** The wrapper's script and panels, or null when the plan has nothing to say before or after the chapters. */
export function buildWrapper({ plan, first, panels, previous }: WrapperInput): Wrapper | null {
  const used: Panel[] = [];
  const renamed = new Map<string, string>();
  // Panel ids repeat across chapters, so each panel the wrapper shows gets a new id of its own.
  const take = (chapter: string, id: string) => {
    const key = `${chapter}/${id}`;
    const known = renamed.get(key);
    if (known) return known;
    const panel = panels[chapter]?.panels.find((p) => p.id === id);
    if (!panel) return null;
    const mine = `p_${String(used.length + 1).padStart(4, "0")}`;
    // Character boxes are left out: their ids are only unique within their own chapter, and no card shows here.
    used.push({ ...panel, id: mine, order: used.length, characters: [] });
    renamed.set(key, mine);
    return mine;
  };
  const drafts: Draft[] = [];
  const add = (part: WrapperPart, chapter: string, ids: string[], narration: string, emotion: Emotion, tone = "neutral") => {
    const panel_ids = ids.map((id) => take(chapter, id)).filter((id): id is string => id !== null);
    if (panel_ids.length && narration.trim()) drafts.push({ part, panel_ids, narration: narration.trim(), emotion, tone });
  };

  if (plan.cold) {
    const { chapter, hook } = plan.cold;
    add("intro", chapter, hook.panel_ids, hook.cold, "mysterious");
    // The bridge stays on the cold open's last panel, in the same voice, on the way back to the start.
    if (plan.bridge) add("intro", chapter, hook.panel_ids.slice(-1), plan.bridge, "mysterious");
  }
  if (plan.recap) {
    const before = previous ? panels[previous] : null;
    if (previous && before) add("intro", previous, inOrder(before).slice(-RECAP_PANELS).map((p) => p.id), plan.recap, "calm");
    else if (panels[first]) add("intro", first, inOrder(panels[first]!).slice(0, 1).map((p) => p.id), plan.recap, "calm");
  }
  if (plan.tease) add("outro", plan.tease.chapter, plan.tease.hook.panel_ids, plan.tease.hook.tease, "calm", "soft, warm");
  if (!drafts.length) return null;

  const lines = drafts.map((d, i) => ({
    id: `l_${String(i + 1).padStart(4, "0")}`,
    panel_ids: d.panel_ids,
    narration: d.narration,
    emotion: d.emotion,
    voice: { speaker: "narrator", tone: d.tone, pace: "normal", intensity: 0.5 },
    preset: PresetSpec.parse({ name: d.panel_ids.length > 1 ? "split_screen" : d.emotion === "mysterious" && i === 0 ? "zoom" : "hold", params: {} }),
    // Each part opens on a cut; the lines inside it fade into each other like a chapter's.
    transition_in: { name: i === 0 || drafts[i - 1]!.part !== d.part ? "cut" : "fade", duration: 0.3 },
  }));
  const parts: Record<WrapperPart, string[]> = { intro: [], outro: [] };
  drafts.forEach((d, i) => parts[d.part].push(lines[i]!.id));
  return {
    script: ScriptDoc.parse({ schema_version: 1, chapter_id: first, lines }),
    panels: PanelsDoc.parse({ schema_version: 1, chapter_id: first, layout: panels[first]?.layout ?? "strip", panels: used }),
    parts,
  };
}

/** The script of one part of the wrapper: only its own lines, in order. */
export function wrapperPart(wrapper: Wrapper, part: WrapperPart): ScriptDoc | null {
  const ids = new Set(wrapper.parts[part]);
  const lines = wrapper.script.lines.filter((l) => ids.has(l.id));
  return lines.length ? { ...wrapper.script, lines } : null;
}
