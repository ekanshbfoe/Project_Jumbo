import { describe, expect, it } from "vitest";
import { bokehLights } from "../src/ambient/cityBokeh";
import { dustMotes } from "../src/ambient/dust";
import { smokePuffs } from "../src/ambient/smoke";
import type { AmbientClock } from "../src/ambient/types";

const W = 1920;
const H = 1080;
const clock = (frame: number): AmbientClock => ({ frame, fps: 30, width: W, height: H });

interface Particle {
  x: number;
  y: number;
  opacity: number;
}

const offscreen = (p: Particle, margin: number) =>
  p.x < -margin || p.x > W + margin || p.y < -margin || p.y > H + margin;

/**
 * Over `frames` consecutive frames, every particle either moves smoothly or jumps only while it is
 * invisible (off screen or fully faded), so loops never pop on screen and never reset at beat cuts.
 */
function expectSeamless(sample: (frame: number) => Particle[], frames: number, maxStep: number, margin = 0) {
  for (let f = 0; f < frames; f++) {
    const a = sample(f);
    const b = sample(f + 1);
    a.forEach((p, i) => {
      const q = b[i]!;
      const step = Math.hypot(q.x - p.x, q.y - p.y);
      if (step > maxStep) {
        const hidden = (offscreen(p, margin) && offscreen(q, margin)) || (p.opacity < 0.01 && q.opacity < 0.01);
        expect(hidden, `particle ${i} jumped ${step.toFixed(1)}px on screen at frame ${f}`).toBe(true);
      }
    });
  }
}

describe("dust", () => {
  it("is deterministic per episode frame", () => {
    expect(dustMotes(clock(1234))).toEqual(dustMotes(clock(1234)));
  });

  it("flows seamlessly over a long stretch", () => {
    expectSeamless((f) => dustMotes(clock(f * 7)), 600, 30);
  });

  it("stays faint", () => {
    expect(Math.max(...dustMotes(clock(99)).map((m) => m.opacity))).toBeLessThanOrEqual(0.7);
  });
});

describe("smoke", () => {
  it("is deterministic per episode frame", () => {
    expect(smokePuffs(clock(4321))).toEqual(smokePuffs(clock(4321)));
  });

  it("restarts each puff only while it is invisible", () => {
    // Steps of 10 frames over ~3.3 minutes cover every puff's 16-30 s cycle several times.
    expectSeamless((f) => smokePuffs(clock(f * 10)), 600, 120);
  });

  it("stays translucent and rises", () => {
    const puffs = smokePuffs(clock(300));
    expect(Math.max(...puffs.map((p) => p.opacity))).toBeLessThan(0.2);
    const later = smokePuffs(clock(330));
    const rising = puffs.filter((p, i) => later[i]!.y < p.y).length;
    expect(rising).toBeGreaterThan(puffs.length * 0.8);
  });
});

describe("city bokeh", () => {
  it("is deterministic per episode frame", () => {
    expect(bokehLights(clock(777))).toEqual(bokehLights(clock(777)));
  });

  it("wraps lights around only off screen", () => {
    // A disc's centre is off screen by at least its radius when it wraps; allow for the largest disc.
    expectSeamless((f) => bokehLights(clock(f * 5)), 900, 40, 0);
  });

  it("drifts both ways like traffic and keeps lights soft", () => {
    const a = bokehLights(clock(0));
    const b = bokehLights(clock(15));
    const moves = a.map((l, i) => b[i]!.x - l.x).filter((d) => Math.abs(d) < 200);
    expect(moves.some((d) => d > 0) && moves.some((d) => d < 0)).toBe(true);
    expect(Math.max(...a.map((l) => l.opacity))).toBeLessThanOrEqual(0.35);
  });
});
