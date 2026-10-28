import { AudioManifest } from "@jumbo/schema";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";
import { api, bridgeApi, errorText, type VoiceProgress } from "../api";
import type { ScreenProps } from "../hub/modules";
import { useAsync } from "../useAsync";

/** The voice stage: the project's voice files, voicing an approved chapter on the worker, and every clip to listen to. */
export function VoiceScreen({ project }: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const session = useAsync(bridgeApi.view);
  const library = useAsync(api.voiceLibrary);
  const [chapter, setChapter] = useState<string>();
  const data = useAsync(() => (chapter ? api.chapterAudio(chapter) : Promise.resolve(null)), [chapter]);
  const [progress, setProgress] = useState<VoiceProgress>();
  const [error, setError] = useState<string>();
  const connected = session.data?.phase === "ready";
  const current = chapters.data?.find((c) => c.id === chapter);

  useEffect(() => {
    const approved = chapters.data?.find((c) => c.state === "approved");
    if (!chapter && chapters.data?.length) setChapter((approved ?? chapters.data[0]!).id);
  }, [chapters.data, chapter]);

  useEffect(() => {
    const offs = [
      listen<VoiceProgress>("voice://progress", (e) => setProgress((p) => (p && e.payload.chapter === p.chapter ? e.payload : p))),
      listen("bridge://state", () => session.reload()),
    ];
    return () => {
      offs.forEach((off) => off.then((f) => f()));
    };
  }, [session.reload]);

  const clips = useMemo(() => {
    const parsed = data.data?.audio ? AudioManifest.safeParse(data.data.audio) : undefined;
    return parsed?.success ? parsed.data.clips : [];
  }, [data.data]);
  const report = useMemo(() => new Map((data.data?.report?.lines ?? []).map((l) => [l.line_id, l])), [data.data]);

  const importVoice = async () => {
    const picked = await open({ title: "Narrator voice file", filters: [{ name: "Voice", extensions: ["gguf"] }] });
    if (typeof picked !== "string") return;
    setError(undefined);
    try {
      await api.importVoice("narrator", "neutral", picked);
      library.reload();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const record = async () => {
    if (!chapter) return;
    setError(undefined);
    setProgress({ chapter, progress: 0, message: "starting" });
    try {
      await api.recordVoice(chapter);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setProgress(undefined);
      chapters.reload();
      data.reload();
      library.reload();
    }
  };

  const canRecord = connected && (current?.state === "approved" || current?.state === "voiced") && !progress;
  const voices = Object.entries(library.data ?? {});

  return (
    <div>
      <div className="page-header">
        <h1>Voice</h1>
        <div className="row">
          <select value={chapter ?? ""} onChange={(e) => setChapter(e.target.value)} disabled={!chapters.data?.length}>
            {chapters.data?.map((c) => (
              <option key={c.id} value={c.id}>
                {c.id} · {c.title ?? c.original_name} ({c.state})
              </option>
            ))}
          </select>
          {progress ? (
            <span className="muted">
              {Math.round(progress.progress * 100)}% {progress.message}
            </span>
          ) : (
            <button className="primary" disabled={!canRecord} onClick={record} title={connected ? "Voice the approved script on the worker" : "Launch a worker on the Session screen first"}>
              {current?.state === "voiced" ? "Record again" : "Record voice"}
            </button>
          )}
        </div>
      </div>
      {error || data.error || library.error ? <p className="error">{error ?? data.error ?? library.error}</p> : null}
      <div className="stack">
        <div className="card stack">
          <div className="row">
            <strong>Voices</strong>
            <button onClick={importVoice}>Import narrator voice…</button>
          </div>
          {voices.length ? (
            voices.map(([speaker, emotions]) => (
              <span key={speaker}>
                {speaker}{" "}
                {emotions.map((e) => (
                  <span key={e} className="badge ok">
                    {e}
                  </span>
                ))}
              </span>
            ))
          ) : (
            <span className="muted">No voice files yet: the worker invents the narrator from its description and keeps that voice here.</span>
          )}
        </div>
        {data.data?.report ? (
          <span className="muted">
            {clips.length} clips · {(data.data.report.total_s / 60).toFixed(1)} min of narration
          </span>
        ) : current?.state === "approved" ? (
          <div className="card empty">Not voiced yet. Press Record voice.</div>
        ) : null}
        {clips.length ? (
          <div className="card">
            <table>
              <thead>
                <tr>
                  <th>Line</th>
                  <th>Clip</th>
                  <th>Length</th>
                  <th>Takes</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {clips.map((clip) => {
                  const line = report.get(clip.line_id);
                  return (
                    <tr key={clip.line_id}>
                      <td className="mono">{clip.line_id}</td>
                      <td>
                        <audio controls preload="none" src={convertFileSrc(`${project.root}/${clip.src}`)} />
                      </td>
                      <td className="mono">{clip.duration.toFixed(1)} s</td>
                      <td className="mono">{line?.attempts ?? "?"}</td>
                      <td>
                        {line?.plain ? <span className="badge warn">spoken plain</span> : null} {line?.problem ? <span className="badge danger">{line.problem}</span> : null}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : null}
      </div>
    </div>
  );
}
