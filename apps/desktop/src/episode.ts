import { AudioManifest, buildEpisodeSpec, PanelsDoc, ScriptDoc } from "@jumbo/schema";
import { z } from "zod";
import type { PreviewSources } from "./api";

/** The episode a render plays: the approved script, the panels and the voiced clips, joined by buildEpisodeSpec. */
export function buildEpisode(chapter: string, src: PreviewSources, title: string) {
  const parse = <T,>(schema: z.ZodType<T>, file: string, data: unknown) => {
    const result = schema.safeParse(data);
    if (!result.success) throw new Error(`chapters/${chapter}/${file}\n${z.prettifyError(result.error)}`);
    return result.data;
  };
  if (src.script_file !== "approved.json" || src.audio == null || src.panels == null) throw new Error("This chapter needs an approved script and its voice first.");
  return buildEpisodeSpec(parse(ScriptDoc, "script/approved.json", src.script), parse(PanelsDoc, "panels/panels.json", src.panels), parse(AudioManifest, "audio/audio.manifest.json", src.audio), { title });
}
