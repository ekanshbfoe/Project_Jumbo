import { AudioManifest, buildWrapper, PanelsDoc, planEpisode, type EpisodePlan, type Wrapper, type WrapperPart } from "@jumbo/schema";
import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { api, errorText, type AssemblyProgress, type ChapterManifest, type RenderProgress, type VoiceProgress } from "./api";
import { buildPart, buildWrapperPart, chapterTitle } from "./episode";
import { onT4 } from "./makeVideo";

/**
 * The episode job running now: its parts rendering on Kaggle one after another, its wrapper being voiced on the
 * T4s, or the join on this PC.
 */
export interface EpisodeRun {
  episode: string;
  kind: "parts" | "voice" | "assemble";
  /** The part rendering: a chapter id, or `intro` / `outro`. */
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

// A wrapper part renders under the label `<episode>-<part>`.
const rendering = (label: string) => state.run?.kind === "parts" && (label === state.run.chapter || label === `${state.run.episode}-${state.run.chapter}`);
void listen<RenderProgress>("render://progress", (e) => {
  if (rendering(e.payload.chapter)) patch({ progress: e.payload.progress, message: e.payload.message, machines: e.payload.machines });
});
void listen<VoiceProgress>("voice://progress", (e) => {
  if (state.run?.kind === "voice" && e.payload.chapter === state.run.episode) patch({ progress: e.payload.progress, message: e.payload.message });
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

/** An episode and the project's chapters, all a spec or a wrapper is built from. */
export interface EpisodeSources {
  episode: { id: string; chapters: string[] };
  /** Every chapter of the project, in story order. */
  all: ChapterManifest[];
  /** The episode's place among the project's episodes, from 0 (picks the bridge line). */
  index: number;
}

/** The episode's wrapper now (ADR 0008): its plan, the wrapper built from it, and the one last voiced. */
export interface WrapperState {
  plan: EpisodePlan;
  /** Null when the plan has nothing to say before or after the chapters. */
  wrapper: Wrapper | null;
  /** The clips last voiced, or null before the first time. */
  audio: AudioManifest | null;
  /** True when the voiced wrapper says exactly what the plan says now, line for line. */
  current: boolean;
}

const spoken = (lines: { id: string; narration: string; emotion: string; voice?: unknown }[]) => JSON.stringify(lines.map((l) => [l.id, l.narration, l.emotion, l.voice]));

export async function episodeWrapper({ episode, all, index }: EpisodeSources): Promise<WrapperState> {
  const at = (id: string) => all.findIndex((c) => c.id === id);
  const first = episode.chapters[0]!;
  const next = all[at(episode.chapters[episode.chapters.length - 1]!) + 1];
  const previous = at(first) > 0 ? all[at(first) - 1]!.id : null;
  const hooks = await api.chapterHooks([...episode.chapters, ...(next ? [next.id] : [])]);
  const plan = planEpisode({
    chapters: episode.chapters.map((id) => ({ id, hooks: hooks[id] ?? null })),
    index,
    startsStory: all[0]?.id === first,
    next: next && hooks[next.id] ? { id: next.id, hooks: hooks[next.id]! } : null,
  });
  const wanted = [first, plan.cold?.chapter, plan.tease?.chapter, plan.recap ? previous : null].filter((c): c is string => !!c);
  const panels: Record<string, PanelsDoc | null> = {};
  for (const id of new Set(wanted)) panels[id] = PanelsDoc.safeParse((await api.chapterPreview(id)).panels).data ?? null;
  const wrapper = buildWrapper({ plan, first, panels, previous });
  const voiced = await api.episodeWrapper(episode.id);
  const audio = AudioManifest.safeParse(voiced.audio).data ?? null;
  const saved = (voiced.script as { lines?: Parameters<typeof spoken>[0] } | null)?.lines;
  const current = !!wrapper && !!audio && !!saved && spoken(saved) === spoken(wrapper.script.lines) && wrapper.script.lines.every((l) => audio.clips.some((c) => c.line_id === l.id));
  return { plan, wrapper, audio, current };
}

/**
 * Every part's spec as the app would render it now: chapter id, and `intro` / `outro` once the wrapper is voiced as
 * the plan stands. Until then the episode is made without them, as before the wrapper existed.
 */
export async function partSpecs(src: EpisodeSources, wrap?: WrapperState): Promise<Record<string, unknown>> {
  const state = wrap ?? (await episodeWrapper(src));
  const chapters = src.episode.chapters.map((id) => src.all.find((c) => c.id === id)).filter((c): c is ChapterManifest => !!c);
  const { signature_s } = await api.brandSettings();
  const voiced = state.current && state.wrapper && state.audio ? { wrapper: state.wrapper, audio: state.audio } : null;
  const part = (p: WrapperPart, title: string) => (voiced ? buildWrapperPart(voiced.wrapper, voiced.audio, p, title, signature_s) : null);
  const intro = chapters.length ? part("intro", chapterTitle(chapters[0]!)) : null;
  const outro = chapters.length ? part("outro", chapterTitle(chapters[chapters.length - 1]!)) : null;
  const specs: Record<string, unknown> = intro ? { intro } : {};
  for (const [i, c] of chapters.entries()) {
    specs[c.id] = buildPart(c.id, await api.chapterPreview(c.id), chapterTitle(c), { first: i === 0, last: i === chapters.length - 1 && !outro }, signature_s);
  }
  if (outro) specs.outro = outro;
  return specs;
}

/** Voices the episode's wrapper on the T4s with Drama 3: a session is started for it and stopped right after. */
export async function voiceWrapper(src: EpisodeSources, onDone: () => void) {
  const episode = src.episode.id;
  set(() => ({ run: { episode, kind: "voice", done: 0, total: 1, progress: 0, message: "starting the T4 session", machines: [] } }));
  try {
    const { wrapper } = await episodeWrapper(src);
    if (!wrapper) throw new Error("The episode has no cold open, recap or tease to voice: paste chapter answers with a HOOKS section first.");
    await onT4(() => api.recordWrapper(episode, wrapper.script));
    set(() => ({ notice: `${episode}: the opening and ending are voiced. Render the parts next.` }));
  } catch (e) {
    set(() => ({ error: `${episode}: ${errorText(e)}` }));
  } finally {
    onDone();
  }
}

/**
 * Renders the parts of `todo` (part keys of the episode, in playing order) one after another; the first failure
 * stops the run.
 */
export async function renderParts(src: EpisodeSources, todo: string[], onDone: () => void) {
  const episode = src.episode.id;
  set(() => ({ run: { episode, kind: "parts", done: 0, total: todo.length, progress: 0, message: "building the parts", machines: [] } }));
  try {
    const specs = await partSpecs(src);
    for (const [k, chapter] of todo.entries()) {
      patch({ chapter, done: k, progress: 0, message: "starting", machines: [] });
      if (chapter === "intro" || chapter === "outro") await api.renderWrapper(episode, chapter, specs[chapter]);
      else await api.renderPart(episode, chapter, specs[chapter]);
    }
    set(() => ({ notice: `${episode}: ${todo.length} part${todo.length === 1 ? "" : "s"} rendered. Assemble the video next.` }));
  } catch (e) {
    set((s) => ({ error: `${episode}${s.run?.chapter ? ` (${s.run.chapter})` : ""}: ${errorText(e)}` }));
  } finally {
    onDone();
  }
}

/** Joins the episode's parts into its video on this PC. */
export async function assemble(src: EpisodeSources, onDone: () => void) {
  const episode = src.episode.id;
  set(() => ({ run: { episode, kind: "assemble", done: 0, total: 1, progress: 0, message: "starting", machines: [] } }));
  try {
    const video = await api.assembleEpisode(episode, await partSpecs(src));
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
