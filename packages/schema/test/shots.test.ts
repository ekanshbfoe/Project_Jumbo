import { describe, expect, it } from "vitest";
import { breakTimes, focusPoints, planShots } from "../src";

const NARRATION =
  "सब हाथ मिलाकर उसका welcome करते हैं। उसी दिन Tendo को यकीन हो जाता है कि company बढ़िया है, और माहौल healthy है। " +
  "वो बड़े brands के ads बनाने का सपना देखता है। क्या पता किसी star से भी मुलाक़ात हो जाए। बेचारा।";

describe("breakTimes", () => {
  it("places sentence and clause ends where their text falls in the line", () => {
    const { sentence, clause } = breakTimes(NARRATION, 16);
    expect(sentence).toHaveLength(4);
    expect(sentence.every((t, i) => i === 0 || t > sentence[i - 1]!)).toBe(true);
    expect(clause.length).toBeGreaterThan(0);
  });
});

describe("focusPoints", () => {
  const panel = { x: 0.1, y: 0.2, w: 0.8, h: 0.4 };
  it("aims at characters' heads, largest first, inside the panel", () => {
    const pts = focusPoints(panel, [
      { cluster: "c_1", bbox: { x: 0.12, y: 0.22, w: 0.1, h: 0.1 } },
      { cluster: "c_2", bbox: { x: 0.5, y: 0.25, w: 0.3, h: 0.3 } },
    ]);
    expect(pts[0]![0]).toBeCloseTo((0.65 - 0.1) / 0.8, 5);
    expect(pts[0]![1]).toBeCloseTo((0.25 + 0.075 - 0.2) / 0.4, 5);
    // The small one near the corner is pulled in to 15% so the framing never sits on the edge.
    expect(pts[1]).toEqual([0.15, 0.15]);
  });
  it("falls back to the panel's thirds when no one is in it", () => {
    expect(focusPoints(panel, [])).toEqual([[0.3, 0.5], [0.7, 0.5]]);
    expect(focusPoints({ x: 0, y: 0, w: 0.3, h: 0.9 }, [])).toEqual([[0.5, 0.3], [0.5, 0.7]]);
  });
});

describe("planShots", () => {
  const focus: [number, number][] = [[0.4, 0.3], [0.7, 0.6]];
  it("cuts every few seconds, near sentence breaks, and comes back wide every third cut", () => {
    const shots = planShots(NARRATION, 16, 16.5, focus);
    expect(shots.length).toBeGreaterThanOrEqual(2);
    const starts = [0, ...shots.map((s) => s.at)];
    starts.forEach((t, i) => i && expect(t - starts[i - 1]!).toBeGreaterThanOrEqual(2.5));
    expect(16.5 - starts.at(-1)!).toBeGreaterThanOrEqual(2.5);
    const sentences = breakTimes(NARRATION, 16).sentence;
    expect(shots.some((s) => sentences.includes(s.at) || sentences.some((t) => Math.abs(t - s.at) < 0.01))).toBe(true);
    expect(shots[0]).toMatchObject({ focus: [0.4, 0.3], zoom: 1.45 });
    if (shots[2]) expect(shots[2]).toMatchObject({ focus: [0.5, 0.5], zoom: 1.08 });
  });
  it("leaves short beats on one framing", () => {
    expect(planShots("एक छोटी line।", 3, 4.5, focus)).toEqual([]);
  });
});
