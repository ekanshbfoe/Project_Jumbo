import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { api, errorText, type Detector, type Method, type VisionProgress } from "./api";
import { makeVideo, ReviewNeeded, type MakeVideoStage } from "./makeVideo";

/** A long chapter job started from the Chapters screen. */
export interface Run {
  chapter: string;
  kind: "make" | "analyze";
  /** Make video's current stage. */
  stage?: MakeVideoStage;
  progress: number;
  message: string;
}

interface RunsState {
  runs: Record<string, Run>;
  notice?: string;
  error?: string;
}

/**
 * Chapter jobs live here, not in a screen: a screen that is left and opened again shows the job still running,
 * with its latest progress, and the outcome is kept until the next job starts.
 */
let state: RunsState = { runs: {} };
const subscribers = new Set<() => void>();

function set(next: (s: RunsState) => RunsState) {
  state = next(state);
  subscribers.forEach((f) => f());
}

function update(chapter: string, patch: Partial<Run>) {
  set((s) => (s.runs[chapter] ? { ...s, runs: { ...s.runs, [chapter]: { ...s.runs[chapter]!, ...patch } } } : s));
}

function finish(chapter: string, outcome: { notice?: string; error?: string }) {
  set(({ runs: { [chapter]: _, ...rest } }) => ({ runs: rest, ...outcome }));
}

// Every stage reports the same {chapter, progress, message} shape.
for (const stage of ["vision", "script", "voice", "render"]) {
  void listen<VisionProgress>(`${stage}://progress`, (e) => update(e.payload.chapter, { progress: e.payload.progress, message: e.payload.message }));
}

export function useRuns(): RunsState {
  return useSyncExternalStore(
    (f) => {
      subscribers.add(f);
      return () => subscribers.delete(f);
    },
    () => state,
  );
}

/** Whether any chapter job is running (switching project waits for it). */
export const anyRun = () => Object.keys(state.runs).length > 0;

export function clearOutcome() {
  set((s) => ({ runs: s.runs }));
}

export async function startAnalyze(chapter: string, detector: Detector, onDone: () => void) {
  set((s) => ({ runs: { ...s.runs, [chapter]: { chapter, kind: "analyze", progress: 0, message: "starting" } } }));
  try {
    await api.analyzeChapter(chapter, detector);
    finish(chapter, {});
  } catch (e) {
    finish(chapter, { error: `${chapter}: ${errorText(e)}` });
  } finally {
    onDone();
  }
}

export async function startMakeVideo(chapter: string, method: Method, onStage: () => void) {
  set((s) => ({ runs: { ...s.runs, [chapter]: { chapter, kind: "make", progress: 0, message: "" } } }));
  try {
    await makeVideo(chapter, method, (stage) => {
      update(chapter, { stage, progress: 0, message: "" });
      onStage();
    });
    finish(chapter, { notice: `${chapter}: the video is ready on the Render screen.` });
  } catch (e) {
    finish(chapter, e instanceof ReviewNeeded ? { notice: `${chapter}: ${errorText(e)}` } : { error: `${chapter}: ${errorText(e)}` });
  } finally {
    onStage();
  }
}
