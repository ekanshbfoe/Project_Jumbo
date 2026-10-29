import { AudioManifest, buildEpisodeSpec, PanelsDoc, ScriptDoc, type BuildEpisodeOptions } from "@jumbo/schema";
import { z } from "zod";
import type { PreviewSources } from "./api";

/** The episode a render plays: the approved script, the panels and the voiced clips, joined by buildEpisodeSpec. */
export function buildEpisode(chapter: string, src: PreviewSources, title: string, options: BuildEpisodeOptions = {}) {
  const parse = <T,>(schema: z.ZodType<T>, file: string, data: unknown) => {
    const result = schema.safeParse(data);
    if (!result.success) throw new Error(`chapters/${chapter}/${file}\n${z.prettifyError(result.error)}`);
    return result.data;
  };
  if (src.script_file !== "approved.json" || src.audio == null || src.panels == null) throw new Error("This chapter needs an approved script and its voice first.");
  return buildEpisodeSpec(parse(ScriptDoc, "script/approved.json", src.script), parse(PanelsDoc, "panels/panels.json", src.panels), parse(AudioManifest, "audio/audio.manifest.json", src.audio), { title, ...options });
}

/**
 * A chapter's part of an episode (ADR 0007): the follow card shows early only in the first part and at the end only
 * in the last, so a long episode shows it once at each end. Parts join straight, with no title card between them.
 * The last part also holds its final panel after the narration (END_HOLD_S).
 */
export function buildPart(chapter: string, src: PreviewSources, title: string, at: { first: boolean; last: boolean }, signatureS = 0) {
  // The last part holds its final panel after the narration, so the episode does not stop on the last word, and
  // long enough for the signature clip with a breath on either side.
  const promo = at.last ? { early: at.first } : { early: at.first, outro_s: 0 };
  return buildEpisode(chapter, src, title, { promo, endHold: at.last ? endHold(signatureS) : 0 });
}

/** Seconds the episode's last panel holds after the narration while the brand music fades out. */
export const END_HOLD_S = 3;

/** The final hold for a signature clip of `signatureS` seconds: a second before it and one after (assembly.rs). */
export const endHold = (signatureS: number) => (signatureS > 0 ? Math.max(END_HOLD_S, Math.ceil((signatureS + 2) * 10) / 10) : END_HOLD_S);

/** The title a chapter is shown under: its own, or its PDF's name. */
export const chapterTitle = (c: { title?: string; original_name: string }) => c.title ?? c.original_name.replace(/\.pdf$/i, "");
