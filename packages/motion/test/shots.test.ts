import { describe, expect, it } from "vitest";
import { framingAt, framingStyle, SHOT_MOVE_SECONDS, WIDE } from "../src/shots";

const shots = [
  { at: 4, focus: [0.3, 0.4] as [number, number], zoom: 1.4 },
  { at: 8, focus: [0.5, 0.5] as [number, number], zoom: 1.08 },
];

describe("framingAt", () => {
  it("holds the preset's framing until the first cut, then snaps to each shot", () => {
    expect(framingAt(shots, 0, 30)).toEqual(WIDE);
    expect(framingAt(shots, 4 * 30 - 1, 30)).toEqual(WIDE);
    const mid = framingAt(shots, 4 * 30 + 2, 30);
    expect(mid.zoom).toBeGreaterThan(1);
    expect(mid.zoom).toBeLessThan(1.4);
    const settled = framingAt(shots, Math.ceil((4 + SHOT_MOVE_SECONDS) * 30), 30);
    expect(settled.fx).toBeCloseTo(0.3, 5);
    expect(settled.zoom).toBeGreaterThanOrEqual(1.4);
    expect(framingAt(shots, 9 * 30, 30).fx).toBeCloseTo(0.5, 5);
  });
});

describe("framingStyle", () => {
  it("is empty at the wide framing", () => {
    expect(framingStyle(WIDE)).toEqual({});
  });
  it("moves the focus toward the centre but never past the panel's own edge", () => {
    // Focus at the left edge, 1.4x: the shift is held to (1.4 - 1) / 2 = 20% of the panel.
    expect(framingStyle({ zoom: 1.4, fx: 0, fy: 0.5 }).transform).toBe("translate(20.000%, 0.000%) scale(1.4000)");
    expect(framingStyle({ zoom: 1.4, fx: 0.55, fy: 0.5 }).transform).toBe("translate(-7.000%, 0.000%) scale(1.4000)");
  });
});
