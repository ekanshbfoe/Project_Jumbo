import { AudioManifest, buildEpisodeSpec, PanelsDoc, ScriptDoc } from "@jumbo/schema";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { z } from "zod";
import { api, errorText, type PreviewSources, type RenderProgress } from "../api";
import type { ScreenProps } from "../hub/modules";
import { useAsync } from "../useAsync";

/** The episode the render plays: the approved script, the panels and the voiced clips, joined by buildEpisodeSpec. */
function buildEpisode(chapter: string, src: PreviewSources, title: string) {
  const parse = <T,>(schema: z.ZodType<T>, file: string, data: unknown) => {
    const result = schema.safeParse(data);
    if (!result.success) throw new Error(`chapters/${chapter}/${file}\n${z.prettifyError(result.error)}`);
    return result.data;
  };
  if (src.script_file !== "approved.json" || src.audio == null || src.panels == null) throw new Error("This chapter needs an approved script and its voice first.");
  return buildEpisodeSpec(parse(ScriptDoc, "script/approved.json", src.script), parse(PanelsDoc, "panels/panels.json", src.panels), parse(AudioManifest, "audio/audio.manifest.json", src.audio), { title });
}

const MACHINE_BADGE: Record<string, string> = { COMPLETE: "ok", ERROR: "danger", STALE_JOB: "warn" };

/** Final step: build the episode from the voiced chapter, render it on five Kaggle CPU machines and watch the video. */
export function RenderScreen({ project }: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const [chapter, setChapter] = useState<string>();
  const result = useAsync(() => (chapter ? api.chapterRender(chapter) : Promise.resolve(null)), [chapter]);
  const [progress, setProgress] = useState<RenderProgress>();
  const [error, setError] = useState<string>();
  const current = chapters.data?.find((c) => c.id === chapter);

  useEffect(() => {
    const voiced = chapters.data?.find((c) => c.state === "voiced");
    if (!chapter && chapters.data?.length) setChapter((voiced ?? chapters.data[0]!).id);
  }, [chapters.data, chapter]);

  useEffect(() => {
    const off = listen<RenderProgress>("render://progress", (e) => setProgress((p) => (p && e.payload.chapter === p.chapter ? e.payload : p)));
    return () => {
      off.then((f) => f());
    };
  }, []);

  const render = async () => {
    if (!chapter || !current) return;
    setError(undefined);
    setProgress({ chapter, progress: 0, message: "building the episode", machines: [] });
    try {
      const episode = buildEpisode(chapter, await api.chapterPreview(chapter), current.title ?? current.original_name.replace(/\.pdf$/i, ""));
      await api.saveEpisode(chapter, episode);
      await api.renderChapter(chapter);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setProgress(undefined);
      chapters.reload();
      result.reload();
    }
  };

  const canRender = (current?.state === "voiced" || current?.state === "rendered") && !progress;
  const video = result.data?.video;

  return (
    <div>
      <div className="page-header">
        <h1>Render</h1>
        <div className="row">
          <select value={chapter ?? ""} onChange={(e) => setChapter(e.target.value)} disabled={!chapters.data?.length}>
            {chapters.data?.map((c) => (
              <option key={c.id} value={c.id}>
                {c.id} · {c.title ?? c.original_name} ({c.state})
              </option>
            ))}
          </select>
          <button className="primary" disabled={!canRender} onClick={render} title="Five Kaggle CPU machines render the chapter, a sixth joins it">
            {video ? "Render again" : "Render video"}
          </button>
        </div>
      </div>
      {error || result.error ? <pre className="card error preview-error">{error ?? result.error}</pre> : null}
      <div className="stack">
        {progress ? (
          <div className="card stack">
            <span>
              {Math.round(progress.progress * 100)}% {progress.message}
            </span>
            {progress.machines.length ? (
              <div className="row">
                {progress.machines.map((m, i) => (
                  <span key={i} className={`badge ${MACHINE_BADGE[m] ?? ""}`} title={i < progress.machines.length - 1 ? `chunk ${i + 1}` : "join"}>
                    {i < progress.machines.length - 1 ? `chunk ${i + 1}` : "join"}: {m.toLowerCase()}
                  </span>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}
        {video ? (
          <div className="card stack">
            <span className="mono muted">{video}</span>
            <video className="render-video" controls src={convertFileSrc(`${project.root}/${video}`)} />
          </div>
        ) : !progress ? (
          <div className="card empty">{current?.state === "voiced" ? "Ready to render." : "Voice the chapter first, then render it here."}</div>
        ) : null}
      </div>
    </div>
  );
}
