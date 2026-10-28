import { api, bridgeApi, CHAPTER_STATES, type ChapterState, type Method } from "./api";
import { buildEpisode } from "./episode";

export type MakeVideoStage = "session" | "panels" | "script" | "voice" | "render";

/** Raised when the Director's script needs a person before it is voiced: the GPU session is stopped first. */
export class ReviewNeeded extends Error {}

const at = (state: ChapterState, stage: ChapterState) => CHAPTER_STATES.indexOf(state) >= CHAPTER_STATES.indexOf(stage);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function waitForWorker() {
  for (;;) {
    const view = await bridgeApi.view();
    if (view.phase === "ready") return;
    if (view.phase === "failed") throw new Error(`the T4 session failed: ${view.error}`);
    if (view.phase === "stopped" || view.phase === "idle") throw new Error("the T4 session stopped before it was ready");
    await sleep(3000);
  }
}

/**
 * One chapter from wherever it stands to a finished video, as ADR 0005 places the stages: Magi, the Director and
 * Drama 3 in one T4 session that ends as soon as the voice is done, then the render on Kaggle CPU machines.
 * A script with failed scenes or rule problems stops the run for review; pressing Make Video again continues.
 *
 * Method 2 (ADR 0006) never runs the Director and never approves on its own: its T4 session only finds the panels,
 * then the run stops until the person has pasted Gemini's answer and approved the script.
 */
export async function makeVideo(chapter: string, method: Method, onStage: (stage: MakeVideoStage) => void): Promise<void> {
  const state = async () => {
    const found = (await api.listChapters()).find((c) => c.id === chapter);
    if (!found) throw new Error(`${chapter} is not in this project`);
    return found;
  };
  let current = await state();
  if (method === "method2" && at(current.state, "analyzed") && !at(current.state, "approved")) {
    // Nothing to do on Kaggle until the person has the script from Gemini and approves it.
    throw new ReviewNeeded(
      at(current.state, "directed")
        ? "Approve the Gemini script on the Script screen, then press Make video again."
        : "Give Gemini the numbered PDF, paste its answer on the Gemini script screen and approve it, then press Make video again.",
    );
  }
  if (!at(current.state, "voiced")) {
    onStage("session");
    const view = await bridgeApi.view();
    if ((view.phase === "ready" || view.phase === "booting") && view.mode !== "kaggle_gpu") {
      throw new Error("A session that is not on the T4s is running: stop it on the Session screen, then press Make video again.");
    }
    if (view.phase !== "ready" && view.phase !== "booting") await bridgeApi.start("kaggle_gpu");
    try {
      await waitForWorker();
      if (!at(current.state, "analyzed")) {
        onStage("panels");
        await api.analyzeChapter(chapter, "magi");
        current = await state();
        if (method === "method2") {
          throw new ReviewNeeded("Panels found. Give Gemini the numbered PDF, paste its answer on the Gemini script screen and approve it, then press Make video again.");
        }
      }
      if (!at(current.state, "directed") || (current.state === "directed" && !(await api.chapterScript(chapter)).script)) {
        onStage("script");
        await api.writeScript(chapter);
        current = await state();
      }
      if (current.state === "directed" || current.state === "librarian") {
        const view = await api.chapterScript(chapter);
        const failed = view.report?.failed ?? [];
        const problems = view.report?.problems ?? [];
        if (failed.length || problems.length) {
          throw new ReviewNeeded(
            `The script needs a look before it is voiced (${failed.length ? `scenes ${failed.join(", ")} failed` : problems.join("; ")}). Fix it on the Script screen, approve it, then press Make video again.`,
          );
        }
        await api.approveScript(chapter, view.script);
        current = await state();
      }
      onStage("voice");
      await api.recordVoice(chapter);
      current = await state();
    } finally {
      // The T4 hours are the scarce quota: the session ends the moment its stages are done or fail.
      await bridgeApi.stop().catch(() => undefined);
    }
  }
  onStage("render");
  const episode = buildEpisode(chapter, await api.chapterPreview(chapter), current.title ?? current.original_name.replace(/\.pdf$/i, ""));
  await api.saveEpisode(chapter, episode);
  await api.renderChapter(chapter);
}
