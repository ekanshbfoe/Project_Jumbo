import { PanelsDoc } from "@jumbo/schema";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { z } from "zod";
import { api, CHAPTER_STATES, type VisionReport } from "../api";
import type { ScreenProps } from "../hub/modules";
import { useAsync } from "../useAsync";

const KIND_LABEL = { vertical: "joined top-to-bottom", horizontal: "joined as a spread", none: "kept apart" } as const;

const count = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

function Report({ report }: { report: VisionReport }) {
  const joined = report.joins.filter((j) => j.kind !== "none");
  return (
    <div className="card stack">
      <strong>Page breaks</strong>
      <span className="muted">
        {count(report.pages, "PDF page")} → {count(report.canvases.length, "canvas", "canvases")} ({count(report.canvases.filter((c) => c.kind === "spread").length, "spread")},{" "}
        {count(report.canvases.filter((c) => c.kind === "strip").length, "strip run")}) → {count(report.images, "image")} → {count(report.panels, "panel")} · detector{" "}
        <span className={report.detector === "magi" ? "badge ok" : "badge warn"}>{report.detector}</span>
      </span>
      <table>
        <thead>
          <tr>
            <th>Break</th>
            <th>Decision</th>
            <th>Why</th>
            <th title="mean grey difference across the cut / inside each page / gradient correlation">cross · inner · corr</th>
          </tr>
        </thead>
        <tbody>
          {report.joins.map((j) => (
            <tr key={j.after_page}>
              <td className="mono">
                {j.after_page + 1} | {j.after_page + 2}
              </td>
              <td>
                <span className={`badge ${j.kind === "none" ? "" : "ok"}`}>{KIND_LABEL[j.kind]}</span>
              </td>
              <td className="muted">{j.reason}</td>
              <td className="mono muted">
                {j.cross.toFixed(1)} · {j.inner.toFixed(1)} · {j.correlation.toFixed(2)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {joined.length === 0 ? <span className="muted">No page break was judged to cut through a drawing.</span> : null}
    </div>
  );
}

/** Review of the vision stage: every page or slice with its detected panel outlines and reading order. */
export function PanelsScreen({ project }: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const [chapter, setChapter] = useState<string>();
  const data = useAsync(() => (chapter ? api.chapterPanels(chapter) : Promise.resolve(null)), [chapter]);

  useEffect(() => {
    // Start on the first chapter that has panels to review.
    const analyzed = chapters.data?.find((c) => CHAPTER_STATES.indexOf(c.state) >= CHAPTER_STATES.indexOf("analyzed"));
    if (!chapter && chapters.data?.length) setChapter((analyzed ?? chapters.data[0]!).id);
  }, [chapters.data, chapter]);

  const parsed = useMemo(() => {
    if (!data.data?.panels) return undefined;
    const result = PanelsDoc.safeParse(data.data.panels);
    return result.success ? { doc: result.data, error: undefined } : { doc: undefined, error: z.prettifyError(result.error) };
  }, [data.data]);

  const pages = useMemo(() => {
    if (!parsed?.doc) return [];
    const byPage = new Map<string, PanelsDoc["panels"]>();
    for (const panel of parsed.doc.panels) byPage.set(panel.page, [...(byPage.get(panel.page) ?? []), panel]);
    return [...byPage.entries()];
  }, [parsed]);

  return (
    <div>
      <div className="page-header">
        <h1>Panels</h1>
        <div className="row">
          <select value={chapter ?? ""} onChange={(e) => setChapter(e.target.value)} disabled={!chapters.data?.length}>
            {chapters.data?.map((c) => (
              <option key={c.id} value={c.id}>
                {c.id} · {c.title ?? c.original_name} ({c.state})
              </option>
            ))}
          </select>
          <button onClick={data.reload} disabled={!chapter}>
            Reload
          </button>
        </div>
      </div>
      {data.error ? <p className="error">{data.error}</p> : null}
      {data.data && !data.data.panels ? <div className="card empty">Not analyzed yet. Run Analyze on the Chapters screen.</div> : null}
      {parsed?.error ? <pre className="card error preview-error">{parsed.error}</pre> : null}
      <div className="stack">
        {data.data?.report ? <Report report={data.data.report} /> : null}
        <div className="panel-pages">
          {pages.map(([page, panels]) => (
            <figure key={page} className="panel-page">
              <div className="panel-page-image">
                <img src={convertFileSrc(`${project.root}/${page}`)} alt={page} />
                <svg viewBox="0 0 1 1" preserveAspectRatio="none">
                  {panels.map((p) => (
                    <polygon key={p.id} points={p.polygon.map(([x, y]) => `${x},${y}`).join(" ")} />
                  ))}
                </svg>
                {panels.map((p) => (
                  <span key={p.id} className="panel-label" style={{ left: `${p.bbox.x * 100}%`, top: `${p.bbox.y * 100}%` }} title={p.id}>
                    {p.order + 1}
                  </span>
                ))}
              </div>
              <figcaption className="mono muted">
                {page.split("/").pop()} · {panels.length} panel{panels.length === 1 ? "" : "s"}
              </figcaption>
            </figure>
          ))}
        </div>
      </div>
    </div>
  );
}
