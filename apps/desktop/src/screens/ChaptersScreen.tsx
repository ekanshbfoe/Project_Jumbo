import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { api, bridgeApi, CHAPTER_STATES, errorText, type ChapterManifest, type Detector, type VisionProgress } from "../api";
import type { ScreenProps } from "../hub/modules";
import { makeVideo, ReviewNeeded, type MakeVideoStage } from "../makeVideo";
import { useAsync } from "../useAsync";

function Pipeline({ chapter }: { chapter: ChapterManifest }) {
  const reached = CHAPTER_STATES.indexOf(chapter.state);
  return (
    <div className="pipeline" title={`Stage ${reached + 1} of ${CHAPTER_STATES.length}: ${chapter.state}`}>
      {CHAPTER_STATES.map((s, i) => (
        <span key={s} className={i <= reached ? "done" : ""} />
      ))}
    </div>
  );
}

const STAGE_LABEL: Record<MakeVideoStage, string> = {
  session: "starting the T4 session",
  panels: "finding panels",
  script: "writing the script",
  voice: "recording the voice",
  render: "rendering on Kaggle CPUs",
};

const DETECTORS: { id: Detector; label: string }[] = [
  { id: "magi", label: "Magi v2 (Kaggle)" },
  { id: "opencv", label: "OpenCV (local plumbing test only)" },
];

export function ChaptersScreen({ project }: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const session = useAsync(bridgeApi.view);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [detector, setDetector] = useState<Detector>("magi");
  const [analyzing, setAnalyzing] = useState<Record<string, VisionProgress>>({});
  const [making, setMaking] = useState<Record<string, MakeVideoStage>>({});
  const [notice, setNotice] = useState<string>();
  const connected = session.data?.phase === "ready";

  useEffect(() => {
    const follow = (e: { payload: VisionProgress }) => setAnalyzing((a) => (e.payload.chapter in a ? { ...a, [e.payload.chapter]: e.payload } : a));
    const offs = [
      // Every stage reports the same {chapter, progress, message} shape.
      ...["vision", "script", "voice", "render"].map((stage) => listen<VisionProgress>(`${stage}://progress`, follow)),
      // Analyze needs a connected worker; follow the session as it starts and stops.
      listen("bridge://state", () => session.reload()),
    ];
    return () => {
      offs.forEach((off) => off.then((f) => f()));
    };
  }, [session.reload]);

  const analyze = async (chapter: string) => {
    setError(undefined);
    setAnalyzing((a) => ({ ...a, [chapter]: { chapter, progress: 0, message: "starting" } }));
    try {
      await api.analyzeChapter(chapter, detector);
    } catch (e) {
      setError(`${chapter}: ${errorText(e)}`);
    } finally {
      setAnalyzing(({ [chapter]: _, ...rest }) => rest);
      chapters.reload();
    }
  };

  const make = async (chapter: string) => {
    setError(undefined);
    setNotice(undefined);
    setAnalyzing((a) => ({ ...a, [chapter]: { chapter, progress: 0, message: "" } }));
    try {
      await makeVideo(chapter, project.method, (stage) => {
        setMaking((m) => ({ ...m, [chapter]: stage }));
        setAnalyzing((a) => ({ ...a, [chapter]: { chapter, progress: 0, message: "" } }));
        chapters.reload();
      });
      setNotice(`${chapter}: the video is ready on the Render screen.`);
    } catch (e) {
      (e instanceof ReviewNeeded ? setNotice : setError)(`${chapter}: ${errorText(e)}`);
    } finally {
      setMaking(({ [chapter]: _, ...rest }) => rest);
      setAnalyzing(({ [chapter]: _, ...rest }) => rest);
      chapters.reload();
    }
  };

  const importPdfs = async () => {
    const picked = await open({
      multiple: true,
      title: "Import chapter PDFs",
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    if (paths.length === 0) return;
    setBusy(true);
    setError(undefined);
    try {
      // Import in natural filename order so "Chapter 2" comes before "Chapter 10".
      paths.sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
      await api.importChapters(paths);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      chapters.reload();
    }
  };

  return (
    <div>
      <div className="page-header">
        <h1>Chapters</h1>
        <div className="row">
          <select value={detector} onChange={(e) => setDetector(e.target.value as Detector)} title="Panel detector for Analyze">
            {DETECTORS.map((d) => (
              <option key={d.id} value={d.id}>
                {d.label}
              </option>
            ))}
          </select>
          <button className="primary" disabled={busy} onClick={importPdfs}>
            {busy ? "Importing…" : "Import PDFs…"}
          </button>
        </div>
      </div>
      {error || chapters.error ? <p className="error">{error ?? chapters.error}</p> : null}
      {notice ? <p className="card">{notice}</p> : null}
      <div className="card">
        {chapters.data?.length ? (
          <table>
            <thead>
              <tr>
                <th>Chapter</th>
                <th>Source</th>
                <th>Stage</th>
                <th>Progress</th>
                <th>Updated</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {chapters.data.map((c) => (
                <tr key={c.id}>
                  <td className="mono">{c.id}</td>
                  <td>{c.title ?? c.original_name}</td>
                  <td>
                    <span className="badge">{c.state}</span>{" "}
                    {c.stale ? <span className="badge warn">stale</span> : null}
                  </td>
                  <td>
                    <Pipeline chapter={c} />
                  </td>
                  <td className="muted">{new Date(c.updated_at).toLocaleString()}</td>
                  <td>
                    {making[c.id] ? (
                      <span className="muted">
                        {STAGE_LABEL[making[c.id]!]}
                        {analyzing[c.id]?.message ? ` · ${Math.round(analyzing[c.id]!.progress * 100)}% ${analyzing[c.id]!.message}` : "…"}
                      </span>
                    ) : analyzing[c.id] ? (
                      <span className="muted">
                        {Math.round(analyzing[c.id]!.progress * 100)}% {analyzing[c.id]!.message}
                      </span>
                    ) : (
                      <button
                        disabled={!connected || CHAPTER_STATES.indexOf(c.state) > CHAPTER_STATES.indexOf("analyzed")}
                        title={connected ? "Find panels with the selected detector" : "Launch a worker on the Session screen first"}
                        onClick={() => analyze(c.id)}
                      >
                        {c.state === "analyzed" ? "Re-analyze" : "Analyze"}
                      </button>
                    )}{" "}
                    {making[c.id] || analyzing[c.id] ? null : (
                      <button
                        className="primary"
                        disabled={Object.keys(making).length > 0 || c.state === "rendered"}
                        title="Panels, script and voice in one T4 session that ends when the voice is done, then the render on Kaggle CPUs"
                        onClick={() => make(c.id)}
                      >
                        Make video
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <div className="empty">{chapters.loading ? "Loading…" : "No chapters yet. Import chapter PDFs to begin."}</div>
        )}
      </div>
    </div>
  );
}
