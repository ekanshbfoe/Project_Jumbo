import { describe, expect, it } from "vitest";
import { BRIDGES, planEpisode, SIGNATURE, type ChapterHooks, type Hook } from "../src";

const hook = (id: string, strength: number, spoiler: "low" | "high", tease = ""): Hook => ({
  id,
  panel_ids: ["p_0001"],
  strength,
  kind: "reveal",
  spoiler,
  cold: `cold ${id}`,
  tease,
});
const chapter = (id: string, hooks: Hook[], recap_in = ""): { id: string; hooks: ChapterHooks } => ({
  id,
  hooks: { schema_version: 1, chapter_id: id, hooks, recap_in },
});

describe("planning an episode's opening and ending", () => {
  it("opens on the strongest low-spoiler hook after the first chapter, ties going to the later chapter", () => {
    const plan = planEpisode({
      chapters: [chapter("ch_0004", [hook("H1", 5, "low")], "पहले की कहानी।"), chapter("ch_0005", [hook("H1", 4, "low"), hook("H2", 5, "high")]), chapter("ch_0006", [hook("H1", 4, "low")])],
      index: 6,
      startsStory: false,
      next: chapter("ch_0007", [hook("H1", 2, "low", "अगली बार A।"), hook("H2", 5, "high", "अगली बार B।")]),
    });
    expect(plan.cold).toEqual({ chapter: "ch_0006", hook: hook("H1", 4, "low") });
    expect(plan.bridge).toBe(BRIDGES[1]);
    expect(plan.recap).toBe("पहले की कहानी।");
    // A tease never shows the reveal, so a high-spoiler hook may be teased.
    expect(plan.tease?.hook.tease).toBe("अगली बार B।");
    expect(plan.signature).toBe(SIGNATURE);
    expect(plan.notes).toEqual([]);
  });

  it("lets a one-chapter episode open on its own hook, with no recap at the start of the story", () => {
    const plan = planEpisode({ chapters: [chapter("ch_0001", [hook("H1", 5, "low")])], index: 0, startsStory: true, next: null });
    expect(plan.cold?.chapter).toBe("ch_0001");
    expect(plan.bridge).toBe(BRIDGES[0]);
    expect(plan.recap).toBeNull();
    expect(plan.notes).toEqual(["No tease at the end: the chapter after this episode has no script yet"]);
  });

  it("says why there is no cold open, recap or tease", () => {
    const plan = planEpisode({
      chapters: [chapter("ch_0002", []), { id: "ch_0003", hooks: null }, chapter("ch_0004", [hook("H1", 5, "high")])],
      index: 0,
      startsStory: false,
      next: chapter("ch_0005", [hook("H1", 5, "low")]),
    });
    expect(plan.cold).toBeNull();
    expect(plan.bridge).toBeNull();
    expect(plan.notes).toEqual([
      "No cold open: ch_0003 has no hooks (paste an answer with a HOOKS section)",
      "No recap: ch_0002 has no RECAP_IN, so viewers start mid-story",
      "No tease at the end: ch_0005 has no hook with a TEASE",
    ]);
  });
});
