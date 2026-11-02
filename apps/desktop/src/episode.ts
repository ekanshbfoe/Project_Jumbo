import { AudioManifest, buildEpisodeSpec, PanelsDoc, ScriptDoc, wrapperPart, type BuildEpisodeOptions, type Wrapper, type WrapperPart } from "@jumbo/schema";
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
 * A chapter's part of an episode (ADR 0007): the follow card shows only at the end of the last part, so a long
 * episode shows it once. Parts join straight, with no title card between them.
 * The last part also holds its final panel after the narration (END_HOLD_S). `at.last` is false for the last
 * chapter when the episode's wrapper ends on its outro.
 */
export function buildPart(chapter: string, src: PreviewSources, title: string, at: { first: boolean; last: boolean }, signatureS = 0) {
  // The last part holds its final panel after the narration, so the episode does not stop on the last word, and
  // long enough for the signature clip with a breath on either side.
  const promo = at.last ? { early: false } : { early: false, outro_s: 0 };
  return buildEpisode(chapter, src, title, { promo, endHold: at.last ? endHold(signatureS) : 0 });
}

/**
 * A part of the episode's wrapper (ADR 0008), or null when it has no lines: the intro opens the episode with no
 * follow card; the outro ends it, with the follow card and the final hold for the signature.
 */
export function buildWrapperPart(wrapper: Wrapper, audio: AudioManifest, part: WrapperPart, title: string, signatureS = 0) {
  const script = wrapperPart(wrapper, part);
  if (!script) return null;
  const promo = part === "outro" ? { early: false } : { early: false, outro_s: 0 };
  return buildEpisodeSpec(script, wrapper.panels, audio, { title, promo, endHold: part === "outro" ? endHold(signatureS) : 0 });
}

/** Seconds the episode's last panel holds after the narration while the brand music fades out. */
export const END_HOLD_S = 3;

/** The final hold for a signature clip of `signatureS` seconds: a second before it and one after (assembly.rs). */
export const endHold = (signatureS: number) => (signatureS > 0 ? Math.max(END_HOLD_S, Math.ceil((signatureS + 2) * 10) / 10) : END_HOLD_S);

/** The title a chapter is shown under: its own, or its PDF's name. */
export const chapterTitle = (c: { title?: string; original_name: string }) => c.title ?? c.original_name.replace(/\.pdf$/i, "");
