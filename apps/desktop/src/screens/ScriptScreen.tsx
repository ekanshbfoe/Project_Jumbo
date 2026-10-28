import { PanelsDoc, ScriptDoc, scriptProblems } from "@jumbo/schema";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";
import { z } from "zod";
import { api, bridgeApi, CHAPTER_STATES, errorText, type DirectorReport, type ScriptProgress } from "../api";
import type { ScreenProps } from "../hub/modules";
import { useAsync } from "../useAsync";

function Report({ report }: { report: DirectorReport }) {
  const words = report.scenes.reduce((n, s) => n + (s.summary?.words ?? 0), 0);
  const minutes = report.scenes.reduce((n, s) => n + (s.wall_s ?? 0), 0) / 60;
  return (
    <div className="card stack">
      <strong>Director</strong>
      <span className="muted">
        {report.scenes.length} scenes · {words} words · {minutes.toFixed(0)} min on the worker
      </span>
      {report.failed.length ? (
        <span className="error">
          Scenes {report.failed.join(", ")} failed twice and are missing from the script: write it again before approving.
        </span>
      ) : null}
      {report.problems.length ? <span className="error">{report.problems.join(" · ")}</span> : null}
    </div>
  );
}

/** The Director's script for a chapter: write it on the worker, read and edit each line beside its panels, approve. */
export function ScriptScreen({ project }: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const session = useAsync(bridgeApi.view);
  const [chapter, setChapter] = useState<string>();
  const data = useAsync(() => (chapter ? api.chapterScript(chapter) : Promise.resolve(null)), [chapter]);
  const [draft, setDraft] = useState<ScriptDoc>();
  const [progress, setProgress] = useState<ScriptProgress>();
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const connected = session.data?.phase === "ready";
  const current = chapters.data?.find((c) => c.id === chapter);

  useEffect(() => {
    const directed = chapters.data?.find((c) => c.state === "directed");
    if (!chapter && chapters.data?.length) setChapter((directed ?? chapters.data[0]!).id);
  }, [chapters.data, chapter]);

  useEffect(() => {
    const offs = [
      listen<ScriptProgress>("script://progress", (e) => setProgress((p) => (p && e.payload.chapter === p.chapter ? e.payload : p))),
      listen("bridge://state", () => session.reload()),
    ];
    return () => {
      offs.forEach((off) => off.then((f) => f()));
    };
  }, [session.reload]);

  const loaded = useMemo(() => {
    if (!data.data?.script) return undefined;
    const result = ScriptDoc.safeParse(data.data.script);
    return result.success ? { doc: result.data, error: undefined } : { doc: undefined, error: z.prettifyError(result.error) };
  }, [data.data]);

  useEffect(() => {
    setDraft(loaded?.doc);
    setSaved(false);
  }, [loaded]);

  const crops = useMemo(() => {
    const parsed = data.data?.panels ? PanelsDoc.safeParse(data.data.panels) : undefined;
    return new Map(parsed?.success ? parsed.data.panels.map((p) => [p.id, p.crop]) : []);
  }, [data.data]);

  const problems = useMemo(() => {
    if (!draft) return [];
    const shape = ScriptDoc.safeParse(draft);
    return shape.success ? scriptProblems(draft).map((p) => `${p.path.join(".")}: ${p.message}`) : [z.prettifyError(shape.error)];
  }, [draft]);

  const write = async () => {
    if (!chapter) return;
    setError(undefined);
    setProgress({ chapter, progress: 0, message: "starting" });
    try {
      await api.writeScript(chapter);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setProgress(undefined);
      chapters.reload();
      data.reload();
    }
  };

  const approve = async () => {
    if (!chapter || !draft) return;
    setError(undefined);
    try {
      await api.approveScript(chapter, draft);
      setSaved(true);
      chapters.reload();
      data.reload();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const edit = (i: number, narration: string) => {
    setSaved(false);
    setDraft((d) => d && { ...d, lines: d.lines.map((l, j) => (j === i ? { ...l, narration } : l)) });
  };

  const stage = current ? CHAPTER_STATES.indexOf(current.state) : -1;
  const canWrite = connected && (current?.state === "analyzed" || current?.state === "directed") && !progress;
  const canApprove = !!draft && problems.length === 0 && !data.data?.report?.failed.length && stage >= CHAPTER_STATES.indexOf("directed") && stage <= CHAPTER_STATES.indexOf("approved");

  return (
    <div>
      <div className="page-header">
        <h1>Script</h1>
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
            <button disabled={!canWrite} onClick={write} title={connected ? "Run the Director on the worker" : "Launch a worker on the Session screen first"}>
              {data.data?.script ? "Write again" : "Write script"}
            </button>
          )}
          <button className="primary" disabled={!canApprove} onClick={approve}>
            {saved ? "Approved" : "Approve"}
          </button>
        </div>
      </div>
      {error || data.error ? <p className="error">{error ?? data.error}</p> : null}
      {data.data && !data.data.script ? <div className="card empty">No script yet. Analyze the chapter, then press Write script.</div> : null}
      {loaded?.error ? <pre className="card error preview-error">{loaded.error}</pre> : null}
      <div className="stack">
        {data.data?.report ? <Report report={data.data.report} /> : null}
        {data.data?.script_file ? <span className="muted mono">{data.data.script_file}</span> : null}
        {problems.length ? <p className="error">{problems.join(" · ")}</p> : null}
        {draft?.lines.map((line, i) => (
          <div key={line.id} className="card script-line">
            <div className="script-panels">
              {line.panel_ids.map((pid) => {
                const crop = crops.get(pid);
                return crop ? <img key={pid} src={convertFileSrc(`${project.root}/${crop}`)} alt={pid} title={pid} /> : <span key={pid} className="mono muted">{pid}</span>;
              })}
            </div>
            <div className="stack">
              <div className="row">
                <span className="mono muted">{line.id}</span>
                <span className="badge">{line.emotion}</span>
                <span className="badge">{line.preset.name}</span>
                <span className="muted">
                  {line.voice?.tone ?? "neutral"} · {line.voice?.pace ?? "normal"}
                </span>
              </div>
              <textarea value={line.narration} rows={Math.max(3, Math.ceil(line.narration.length / 90))} onChange={(e) => edit(i, e.target.value)} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
