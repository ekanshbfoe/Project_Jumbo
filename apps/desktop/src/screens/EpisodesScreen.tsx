import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useState } from "react";
import { api, CHAPTER_STATES, errorText, type ChapterManifest, type EpisodeView, type PartState } from "../api";
import { chapterTitle } from "../episode";
import { assemble, clearEpisodeOutcome, partSpecs, renderParts, useEpisodeRuns } from "../episodeRuns";
import type { ScreenProps } from "../hub/modules";
import { useRuns } from "../runs";
import { useAsync } from "../useAsync";

const MACHINE_BADGE: Record<string, string> = { COMPLETE: "ok", ERROR: "danger", STALE_JOB: "warn" };
const PART_BADGE: Record<PartState, { label: string; badge: string }> = {
  ready: { label: "part ready", badge: "ok" },
  outdated: { label: "changed: render again", badge: "warn" },
  missing: { label: "no part yet", badge: "" },
};
const voiced = (c: ChapterManifest) => CHAPTER_STATES.indexOf(c.state) >= CHAPTER_STATES.indexOf("voiced");

/** The app-wide brand music: one track under every episode of every project. */
function BrandMusic() {
  const brand = useAsync(api.brandSettings);
  const [error, setError] = useState<string>();
  const act = async (f: () => Promise<unknown>) => {
    setError(undefined);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      brand.reload();
    }
  };
  const choose = () =>
    act(async () => {
      const path = await open({ title: "Choose the brand music", filters: [{ name: "Audio", extensions: ["mp3", "wav", "flac", "m4a", "aac", "ogg", "opus"] }] });
      if (typeof path === "string") await api.setBrandMusic(path);
    });
  const music = brand.data?.music;
  return (
    <div className="card stack">
      <strong>Brand music</strong>
      <span className="muted">
        One track for every project and channel. It loops seamlessly under the whole episode, fades in and out, and dips while the narrator speaks and under
        the big sound effects. Every episode comes out at YouTube's loudness (-14 LUFS).
      </span>
      {error || brand.error ? <p className="error">{error ?? brand.error}</p> : null}
      <div className="row">
        <span>{music ? <span className="mono">{music}</span> : <span className="muted">No track yet: episodes are made with the narration only.</span>}</span>
        <button onClick={choose}>{music ? "Replace track…" : "Choose track…"}</button>
        {music ? (
          <>
            <label className="row">
              <span className="muted">Level under the voice</span>
              <select value={brand.data?.music_db ?? -12} onChange={(e) => act(() => api.setBrandVolume(Number(e.target.value)))}>
                {[-20, -17, -15, -12, -10, -8].map((db) => (
                  <option key={db} value={db}>
                    {db} dB{db === -12 ? " (default)" : ""}
                  </option>
                ))}
              </select>
            </label>
            <button onClick={() => window.confirm("Remove the brand music from every project?") && act(api.clearBrandMusic)}>Remove</button>
          </>
        ) : null}
      </div>
      <label className="row" title="The 15 brand sound effects play at their own tier (soft, medium, loud); this moves them all together">
        <span className="muted">Sound effects</span>
        <select value={brand.data?.sfx_db ?? 0} onChange={(e) => act(() => api.setSfxVolume(Number(e.target.value)))}>
          {[-9, -6, -3, 0, 3, 6].map((db) => (
            <option key={db} value={db}>
              {db > 0 ? `+${db}` : db} dB{db === 0 ? " (default)" : ""}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}

/** One episode: its chapters with their parts, the render and assembly buttons, and the video. */
function EpisodeCard({ episode, chapters, root, busy, reload }: { episode: EpisodeView; chapters: ChapterManifest[]; root: string; busy: boolean; reload: () => void }) {
  const members = useMemo(() => episode.chapters.map((id) => chapters.find((c) => c.id === id)).filter((c): c is ChapterManifest => !!c), [episode, chapters]);
  const unvoiced = members.filter((c) => !voiced(c)).map((c) => c.id);
  const ready = unvoiced.length === 0 && members.length === episode.chapters.length;
  const parts = useAsync(
    async () => (ready ? new Map(await api.episodeParts(episode.id, await partSpecs(members))) : null),
    [ready, members, busy],
  );
  const [error, setError] = useState<string>();
  const todo = members.filter((c) => parts.data?.get(c.id) !== "ready").map((c) => c.id);

  const remove = async () => {
    if (!window.confirm(`Delete ${episode.id} (${episode.title}) and its assembled video? Its chapters and their parts stay.`)) return;
    setError(undefined);
    try {
      await api.deleteEpisode(episode.id);
    } catch (e) {
      setError(errorText(e));
    } finally {
      reload();
    }
  };

  return (
    <div className="card stack">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <strong>
          {episode.title} <span className="muted mono">{episode.id}</span>
        </strong>
        <div className="row">
          <button
            disabled={busy || !ready || !parts.data || todo.length === 0}
            title="Each chapter renders on Kaggle CPUs as its own part, one after another; parts already current are kept"
            onClick={() => void renderParts(episode.id, members, todo, reload)}
          >
            {todo.length && parts.data ? `Render ${todo.length} part${todo.length === 1 ? "" : "s"}` : "Parts rendered"}
          </button>
          <button
            className="primary"
            disabled={busy || !ready || !parts.data || todo.length > 0}
            title="Joins the parts on this PC: the video is copied as it is, and the sound is mixed once with the brand music"
            onClick={() => void assemble(episode.id, members, reload)}
          >
            {episode.video ? "Assemble again" : "Assemble video"}
          </button>
          <button disabled={busy} onClick={remove}>
            Delete
          </button>
        </div>
      </div>
      {error || parts.error ? <p className="error">{error ?? parts.error}</p> : null}
      {unvoiced.length ? <span className="muted">Voice {unvoiced.join(", ")} first (Make video on the Chapters screen).</span> : null}
      <table>
        <thead>
          <tr>
            <th>Chapter</th>
            <th>Title</th>
            <th>Stage</th>
            <th>Part</th>
          </tr>
        </thead>
        <tbody>
          {members.map((c) => {
            const part = parts.data?.get(c.id);
            return (
              <tr key={c.id}>
                <td className="mono">{c.id}</td>
                <td>{chapterTitle(c)}</td>
                <td>
                  <span className="badge">{c.state}</span> {c.stale ? <span className="badge warn">stale</span> : null}
                </td>
                <td>{part ? <span className={`badge ${PART_BADGE[part].badge}`}>{PART_BADGE[part].label}</span> : <span className="muted">{parts.loading && ready ? "checking…" : "—"}</span>}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {episode.video ? (
        <div className="stack">
          <span className="mono muted">{episode.video}</span>
          <video className="render-video" controls src={convertFileSrc(`${root}/${episode.video}`)} />
        </div>
      ) : null}
    </div>
  );
}

/**
 * Episodes (ADR 0007): voiced chapters, in story order, made into one long video. Each chapter renders on Kaggle as
 * its own part, and the parts are joined on this PC with the brand music.
 */
export function EpisodesScreen({ project }: ScreenProps) {
  const chapters = useAsync(api.listChapters);
  const episodes = useAsync(api.listEpisodes);
  const { run, notice, error: runError } = useEpisodeRuns();
  const { runs } = useRuns();
  const [title, setTitle] = useState("");
  const [from, setFrom] = useState<string>();
  const [to, setTo] = useState<string>();
  const [error, setError] = useState<string>();

  const taken = new Set(episodes.data?.flatMap((e) => e.chapters));
  const free = chapters.data?.filter((c) => !taken.has(c.id)) ?? [];
  const fromIndex = free.findIndex((c) => c.id === from);
  const range = fromIndex < 0 ? [] : free.slice(fromIndex, free.findIndex((c) => c.id === to) + 1);

  useEffect(() => {
    if (!free.some((c) => c.id === from)) setFrom(free[0]?.id);
    if (!free.some((c) => c.id === to)) setTo(free[0]?.id);
  }, [free.map((c) => c.id).join(), from, to]);

  const reload = () => {
    chapters.reload();
    episodes.reload();
  };

  const create = async () => {
    setError(undefined);
    try {
      await api.createEpisode(title, range.map((c) => c.id));
      setTitle("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      episodes.reload();
    }
  };

  // Rendering parts uses the same Kaggle render machines as the Chapters and Render screens.
  const busy = run !== undefined || Object.keys(runs).length > 0;

  return (
    <div>
      <div className="page-header">
        <h1>Episodes</h1>
      </div>
      {error || runError || chapters.error || episodes.error ? <p className="error">{error ?? runError ?? chapters.error ?? episodes.error}</p> : null}
      {notice ? (
        <p className="card">
          {notice} <button onClick={clearEpisodeOutcome}>OK</button>
        </p>
      ) : null}
      <div className="stack">
        {run ? (
          <div className="card stack">
            <span>
              {run.episode} ·{" "}
              {run.kind === "parts" ? `part ${run.done + 1} of ${run.total}${run.chapter ? ` (${run.chapter})` : ""}` : "assembling on this PC"} ·{" "}
              {Math.round(run.progress * 100)}% {run.message}
            </span>
            {run.machines.length ? (
              <div className="row">
                {run.machines.map((m, i) => (
                  <span key={i} className={`badge ${MACHINE_BADGE[m] ?? ""}`}>
                    {i < run.machines.length - 1 ? `chunk ${i + 1}` : "join"}: {m.toLowerCase()}
                  </span>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}

        <div className="card stack">
          <strong>New episode</strong>
          <span className="muted">Chapters follow each other in story order, and each chapter can be in one episode only.</span>
          <div className="row">
            <input placeholder="Episode title" value={title} onChange={(e) => setTitle(e.target.value)} />
            <label className="row">
              <span className="muted">from</span>
              <select value={from ?? ""} onChange={(e) => setFrom(e.target.value)} disabled={!free.length}>
                {free.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.id} · {chapterTitle(c)}
                  </option>
                ))}
              </select>
            </label>
            <label className="row">
              <span className="muted">to</span>
              <select value={to ?? ""} onChange={(e) => setTo(e.target.value)} disabled={!free.length}>
                {free.slice(Math.max(0, fromIndex)).map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.id} · {chapterTitle(c)}
                  </option>
                ))}
              </select>
            </label>
            <button className="primary" disabled={!title.trim() || !range.length} onClick={create}>
              Create ({range.length} chapter{range.length === 1 ? "" : "s"})
            </button>
          </div>
        </div>

        {episodes.data?.map((e) => (
          <EpisodeCard key={e.id} episode={e} chapters={chapters.data ?? []} root={project.root} busy={busy} reload={reload} />
        ))}
        {episodes.data && !episodes.data.length ? <div className="card empty">No episodes yet. Voice some chapters, then make an episode of them here.</div> : null}

        <BrandMusic />
      </div>
    </div>
  );
}
