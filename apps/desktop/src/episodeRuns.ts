import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { api, errorText, type AssemblyProgress, type ChapterManifest, type RenderProgress } from "./api";
import { buildPart, chapterTitle } from "./episode";

/** The episode job running now: its parts rendering on Kaggle one after another, or the join on this PC. */
export interface EpisodeRun {
  episode: string;
  kind: "parts" | "assemble";
  /** The chapter whose part is rendering. */
  chapter?: string;
  /** Parts done and to do in this run. */
  done: number;
  total: number;
  progress: number;
  message: string;
  machines: string[];
}

interface EpisodeRunsState {
  run?: EpisodeRun;
  notice?: string;
  error?: string;
}

/**
 * Like runs.ts for chapters: an episode job lives here, not in the Episodes screen, so leaving the screen during a
 * three-hour render neither stops it nor hides its progress.
 */
let state: EpisodeRunsState = {};
const subscribers = new Set<() => void>();

function set(next: (s: EpisodeRunsState) => EpisodeRunsState) {
  state = next(state);
  subscribers.forEach((f) => f());
}

const patch = (p: Partial<EpisodeRun>) => set((s) => (s.run ? { ...s, run: { ...s.run, ...p } } : s));

void listen<RenderProgress>("render://progress", (e) => {
  if (state.run?.kind === "parts" && e.payload.chapter === state.run.chapter) patch({ progress: e.payload.progress, message: e.payload.message, machines: e.payload.machines });
});
void listen<AssemblyProgress>("assembly://progress", (e) => {
  if (state.run?.kind === "assemble" && e.payload.episode === state.run.episode) patch({ progress: e.payload.progress, message: e.payload.message });
});

export function useEpisodeRuns(): EpisodeRunsState {
  return useSyncExternalStore(
    (f) => {
      subscribers.add(f);
      return () => subscribers.delete(f);
    },
    () => state,
  );
}

export const episodeRunning = () => state.run !== undefined;

/** Every chapter's part spec as the app would render it now, keyed by chapter id. */
export async function partSpecs(chapters: ChapterManifest[]): Promise<Record<string, unknown>> {
  const specs: Record<string, unknown> = {};
  const { signature_s } = await api.brandSettings();
  for (const [i, c] of chapters.entries()) {
    specs[c.id] = buildPart(c.id, await api.chapterPreview(c.id), chapterTitle(c), { first: i === 0, last: i === chapters.length - 1 }, signature_s);
  }
  return specs;
}

/** Renders the parts of `todo` (chapters of `episode`, in order) one after another; the first failure stops the run. */
export async function renderParts(episode: string, chapters: ChapterManifest[], todo: string[], onDone: () => void) {
  set(() => ({ run: { episode, kind: "parts", done: 0, total: todo.length, progress: 0, message: "building the parts", machines: [] } }));
  try {
    const specs = await partSpecs(chapters);
    for (const [k, chapter] of todo.entries()) {
      patch({ chapter, done: k, progress: 0, message: "starting", machines: [] });
      await api.renderPart(episode, chapter, specs[chapter]);
    }
    set(() => ({ notice: `${episode}: ${todo.length} part${todo.length === 1 ? "" : "s"} rendered. Assemble the video next.` }));
  } catch (e) {
    set((s) => ({ error: `${episode}${s.run?.chapter ? ` (${s.run.chapter})` : ""}: ${errorText(e)}` }));
  } finally {
    onDone();
  }
}

/** Joins the episode's parts into its video on this PC. */
export async function assemble(episode: string, chapters: ChapterManifest[], onDone: () => void) {
  set(() => ({ run: { episode, kind: "assemble", done: 0, total: 1, progress: 0, message: "starting", machines: [] } }));
  try {
    const video = await api.assembleEpisode(episode, await partSpecs(chapters));
    set(() => ({ notice: `${episode}: the video is ready (${video}).` }));
  } catch (e) {
    set(() => ({ error: `${episode}: ${errorText(e)}` }));
  } finally {
    onDone();
  }
}

export function clearEpisodeOutcome() {
  set((s) => ({ run: s.run }));
}
