import { describe, expect, it } from "vitest";
import { BRIDGES, buildWrapper, SIGNATURE, wrapperPart, type EpisodePlan, type Hook, type PanelsDoc } from "../src";

const panels = (chapter: string, n: number): PanelsDoc => ({
  schema_version: 1,
  chapter_id: chapter,
  layout: "strip",
  panels: Array.from({ length: n }, (_, i) => {
    const id = `p_${String(i + 1).padStart(4, "0")}`;
    return {
      id,
      order: i,
      page: `chapters/${chapter}/pages/page_001.webp`,
      page_size: { width: 1080, height: 11378 },
      bbox: { x: 0, y: i / n, w: 1, h: 1 / n },
      polygon: [
        [0, i / n],
        [1, i / n],
        [1, (i + 1) / n],
        [0, (i + 1) / n],
      ],
      crop: `chapters/${chapter}/panels/crops/${id}.webp`,
      bubbles: [],
      characters: [{ id: "k_0001", cluster: "c_1", bbox: { x: 0, y: 0, w: 0.1, h: 0.1 } }],
    };
  }),
});
const hook = (panel_ids: string[], cold: string, tease = ""): Hook => ({ id: "H1", panel_ids, strength: 5, kind: "danger", spoiler: "low", cold, tease });
const plan = (p: Partial<EpisodePlan>): EpisodePlan => ({ cold: null, bridge: null, recap: null, tease: null, signature: SIGNATURE, notes: [], ...p });

describe("the episode wrapper", () => {
  it("voices the cold open, bridge, recap and tease over their own chapters' panels", () => {
    const wrapper = buildWrapper({
      plan: plan({
        cold: { chapter: "ch_0006", hook: hook(["p_0004", "p_0005"], "बारिश की रात है।") },
        bridge: BRIDGES[0],
        recap: "पहले की कहानी।",
        tease: { chapter: "ch_0007", hook: hook(["p_0002"], "", "अगली बार सब बदलेगा।") },
      }),
      first: "ch_0005",
      previous: "ch_0004",
      panels: { ch_0004: panels("ch_0004", 8), ch_0005: panels("ch_0005", 4), ch_0006: panels("ch_0006", 6), ch_0007: panels("ch_0007", 3) },
    })!;
    const lines = wrapper.script.lines;
    expect(wrapper.script.chapter_id).toBe("ch_0005");
    expect(lines.map((l) => [l.narration, l.emotion, l.panel_ids])).toEqual([
      ["बारिश की रात है।", "mysterious", ["p_0001", "p_0002"]],
      // The bridge holds the cold open's last panel, already in the wrapper's panels.
      [BRIDGES[0], "mysterious", ["p_0002"]],
      ["पहले की कहानी।", "calm", ["p_0003", "p_0004", "p_0005"]],
      ["अगली बार सब बदलेगा।", "calm", ["p_0006"]],
    ]);
    // Renumbered panels keep their own chapter's images, in the order the wrapper shows them.
    expect(wrapper.panels.panels.map((p) => p.crop)).toEqual([
      "chapters/ch_0006/panels/crops/p_0004.webp",
      "chapters/ch_0006/panels/crops/p_0005.webp",
      "chapters/ch_0004/panels/crops/p_0006.webp",
      "chapters/ch_0004/panels/crops/p_0007.webp",
      "chapters/ch_0004/panels/crops/p_0008.webp",
      "chapters/ch_0007/panels/crops/p_0002.webp",
    ]);
    expect(wrapper.panels.panels.every((p) => p.characters.length === 0)).toBe(true);
    expect(wrapper.parts).toEqual({ intro: ["l_0001", "l_0002", "l_0003"], outro: ["l_0004"] });
    expect(lines.map((l) => l.transition_in.name)).toEqual(["cut", "fade", "fade", "cut"]);
    expect(lines[3]!.voice.tone).toBe("soft, warm");
    expect(wrapperPart(wrapper, "outro")!.lines.map((l) => l.id)).toEqual(["l_0004"]);
  });

  it("recaps over the first chapter's opening panel when the chapter before is not in the project", () => {
    const wrapper = buildWrapper({ plan: plan({ recap: "पहले की कहानी।" }), first: "ch_0005", previous: null, panels: { ch_0005: panels("ch_0005", 4) } })!;
    expect(wrapper.script.lines.map((l) => l.panel_ids)).toEqual([["p_0001"]]);
    expect(wrapper.panels.panels[0]!.crop).toBe("chapters/ch_0005/panels/crops/p_0001.webp");
    expect(wrapperPart(wrapper, "outro")).toBeNull();
  });

  it("has nothing to voice when the plan has no opening or tease, or its panels are gone", () => {
    expect(buildWrapper({ plan: plan({}), first: "ch_0001", previous: null, panels: {} })).toBeNull();
    const missing = plan({ cold: { chapter: "ch_0002", hook: hook(["p_0009"], "ठंडी रात।") }, bridge: BRIDGES[0] });
    expect(buildWrapper({ plan: missing, first: "ch_0001", previous: null, panels: { ch_0002: panels("ch_0002", 3) } })).toBeNull();
  });
});
