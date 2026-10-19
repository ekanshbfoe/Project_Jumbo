import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { api, CHAPTER_STATES, errorText, type ChapterManifest } from "../api";
import type { ScreenProps } from "../hub/modules";
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

export function ChaptersScreen(_: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

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
        <button className="primary" disabled={busy} onClick={importPdfs}>
          {busy ? "Importing…" : "Import PDFs…"}
        </button>
      </div>
      {error || chapters.error ? <p className="error">{error ?? chapters.error}</p> : null}
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
