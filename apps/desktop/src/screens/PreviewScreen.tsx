import { AssetResolverProvider, Episode, episodeTimeline, type AssetResolver } from "@jumbo/motion";
import { AudioManifest, buildEpisodeSpec, EpisodeSpec, PanelsDoc, ScriptDoc } from "@jumbo/schema";
import { Player } from "@remotion/player";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { z } from "zod";
import { api, type PreviewSources } from "../api";
import type { ScreenProps } from "../hub/modules";
import { useAsync } from "../useAsync";

type Built = { spec: EpisodeSpec; from: string; silent: boolean } | { error: string };

/** Validates one source file, naming it in the error so a broken file is easy to find. */
function parseFile<T>(schema: z.ZodType<T>, file: string, data: unknown): T {
  const result = schema.safeParse(data);
  if (!result.success) throw new Error(`${file}\n${z.prettifyError(result.error)}`);
  return result.data;
}

/** Prefers the built `render/episode.json`; otherwise joins script + panels (+ audio) like the render step will. */
function buildSpec(chapter: string, src: PreviewSources): Built {
  const dir = `chapters/${chapter}`;
  try {
    if (src.episode != null) {
      return { spec: parseFile(EpisodeSpec, `${dir}/render/episode.json`, src.episode), from: "render/episode.json", silent: false };
    }
    if (src.script == null || src.panels == null) {
      return { error: "Nothing to preview yet: this chapter needs a script and panels.json, or a built render/episode.json." };
    }
    const script = parseFile(ScriptDoc, `${dir}/script/${src.script_file}`, src.script);
    const panels = parseFile(PanelsDoc, `${dir}/panels/panels.json`, src.panels);
    const audio = src.audio == null ? undefined : parseFile(AudioManifest, `${dir}/audio/audio.manifest.json`, src.audio);
    const from = [`script/${src.script_file}`, "panels.json", audio ? "audio manifest" : "no audio (estimated timing)"].join(" + ");
    return { spec: buildEpisodeSpec(script, panels, audio), from, silent: !audio };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

const isUrl = (path: string) => /^(https?:|data:|blob:)/.test(path);

export function PreviewScreen({ project }: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const [chapter, setChapter] = useState<string>();
  const sources = useAsync(() => (chapter ? api.chapterPreview(chapter) : Promise.resolve(null)), [chapter]);

  useEffect(() => {
    if (!chapter && chapters.data?.length) setChapter(chapters.data[0]!.id);
  }, [chapters.data, chapter]);

  const built = useMemo(() => (chapter && sources.data ? buildSpec(chapter, sources.data) : undefined), [chapter, sources.data]);
  // Spec paths are relative to the project root; the asset protocol serves the open project only.
  const resolver = useMemo<AssetResolver>(() => (path) => (isUrl(path) ? path : convertFileSrc(`${project.root}/${path}`)), [project.root]);

  return (
    <div>
      <div className="page-header">
        <h1>Preview</h1>
        <div className="row">
          <select value={chapter ?? ""} onChange={(e) => setChapter(e.target.value)} disabled={!chapters.data?.length}>
            {chapters.data?.map((c) => (
              <option key={c.id} value={c.id}>
                {c.id} · {c.title ?? c.original_name}
              </option>
            ))}
          </select>
          <button onClick={sources.reload} disabled={!chapter}>
            Reload
          </button>
        </div>
      </div>
      {chapters.error || sources.error ? <p className="error">{chapters.error ?? sources.error}</p> : null}
      {!chapters.loading && !chapters.data?.length ? <div className="card empty">No chapters yet. Import chapter PDFs to begin.</div> : null}
      {built && "error" in built ? <pre className="card error preview-error">{built.error}</pre> : null}
      {built && "spec" in built ? (
        <div className="stack">
          {built.silent ? (
            <div className="card warn">
              <strong>Draft preview: no voice yet.</strong> Timings are estimated from the word count, so its length is not the final video's. Press Make video on the
              Chapters screen to record the voice and render; the finished MP4 is on the Render screen.
            </div>
          ) : null}
          <AssetResolverProvider value={resolver}>
            <Player
              className="preview-player"
              component={Episode}
              inputProps={built.spec}
              durationInFrames={episodeTimeline(built.spec).durationInFrames}
              fps={built.spec.video.fps}
              compositionWidth={built.spec.video.width}
              compositionHeight={built.spec.video.height}
              style={{ width: "100%", aspectRatio: `${built.spec.video.width} / ${built.spec.video.height}` }}
              controls
              acknowledgeRemotionLicense
            />
          </AssetResolverProvider>
          <p className="muted">
            {built.spec.beats.length} beats · {built.spec.video.width}×{built.spec.video.height} @ {built.spec.video.fps} fps · from {built.from}
          </p>
        </div>
      ) : null}
    </div>
  );
}
