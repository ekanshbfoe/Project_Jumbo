import { describe, expect, it } from "vitest";
import { dustMotes } from "../src/ambient/dust";
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
