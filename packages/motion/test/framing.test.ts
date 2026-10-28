import { describe, expect, it } from "vitest";
import { IDENTITY_CAMERA } from "../src/components/Camera";
import { capUpscale, keepInFrame, MAX_UPSCALE, OVERSCAN, scrollShot, SCROLL_WIDTH } from "../src/framing";
import { fitContain } from "../src/geometry";

const frame = { width: 1920, height: 1080 };
const big = { width: 3000, height: 1600 };
/** The panel's box on screen after a camera move (no rotation), to check it against the frame. */
function box(t: typeof IDENTITY_CAMERA, r: { left: number; top: number; width: number; height: number }) {
  const ox = r.left + (r.width * t.originX) / 100;
  const oy = r.top + (r.height * t.originY) / 100;
  const left = ox + (r.left - ox) * t.scale + t.x;
  const top = oy + (r.top - oy) * t.scale + t.y;
  return { left, top, right: left + r.width * t.scale, bottom: top + r.height * t.scale };
}

describe("capUpscale", () => {
  it("shows a small scan at most twice its own pixels, centred where it was", () => {
    const fit = fitContain({ width: 424, height: 286 }, frame, 0.06);
    const capped = capUpscale(fit, { width: 424, height: 286 });
    expect(capped.width).toBeCloseTo(424 * MAX_UPSCALE, 5);
    expect(capped.left + capped.width / 2).toBeCloseTo(fit.left + fit.width / 2, 5);
    expect(capUpscale(fitContain(big, frame, 0.06), big)).toEqual(fitContain(big, frame, 0.06));
  });
});

describe("keepInFrame", () => {
  const rect = fitContain(big, frame, 0.06);
  it("never lets a zoom push the panel past the frame's edge (plus overscan)", () => {
    for (const scale of [1.1, 1.4, 2.0, 3.0]) {
      for (const [ox, oy] of [[50, 50], [10, 90], [95, 5]]) {
        const b = box(keepInFrame({ ...IDENTITY_CAMERA, scale, originX: ox!, originY: oy! }, rect, frame, big), rect);
        expect(b.left).toBeGreaterThanOrEqual(-frame.width * OVERSCAN - 0.5);
        expect(b.right).toBeLessThanOrEqual(frame.width * (1 + OVERSCAN) + 0.5);
        expect(b.top).toBeGreaterThanOrEqual(-frame.height * OVERSCAN - 0.5);
        expect(b.bottom).toBeLessThanOrEqual(frame.height * (1 + OVERSCAN) + 0.5);
      }
    }
  });
  it("eases into the limit instead of stopping dead, and leaves small moves almost as they were", () => {
    const at = (scale: number) => keepInFrame({ ...IDENTITY_CAMERA, scale }, rect, frame, big).scale;
    expect(at(1.01)).toBeCloseTo(1.01, 3);
    expect(at(1.2)).toBeGreaterThan(at(1.1));
    expect(at(3.0)).toBeGreaterThan(at(1.5));
  });
  it("holds a small scan to twice its pixels however far a preset zooms", () => {
    const small = { width: 424, height: 286 };
    const r = capUpscale(fitContain(small, frame, 0.06), small);
    const t = keepInFrame({ ...IDENTITY_CAMERA, scale: 1.5 }, r, frame, small);
    expect((r.width * t.scale) / small.width).toBeLessThanOrEqual(MAX_UPSCALE + 1e-9);
  });
  it("keeps a pan's travel inside the room the frame leaves", () => {
    const tall = fitContain({ width: 800, height: 2400 }, frame, 0.06);
    const b = box(keepInFrame({ ...IDENTITY_CAMERA, y: 400, scale: 1.04 }, tall, frame, { width: 800, height: 2400 }), tall);
    expect(b.bottom).toBeLessThanOrEqual(frame.height * (1 + OVERSCAN) + 0.5);
  });
});

describe("scrollShot", () => {
  const frame = { width: 1920, height: 1080 };

  it("scrolls a tall panel from its top to its bottom at a readable width", () => {
    const tall = { width: 1080, height: 2700 };
    const start = scrollShot(tall, frame, 0, "down")!;
    const end = scrollShot(tall, frame, 1, "down")!;
    expect(start.rect.width).toBeCloseTo(1920 * SCROLL_WIDTH, 6);
    // Top edge just inside the frame at the start, bottom edge just inside it at the end.
    expect(start.transform.y).toBeGreaterThan(0);
    expect(end.transform.y + end.rect.height).toBeLessThan(1080);
    expect(end.transform.y + end.rect.height).toBeGreaterThan(1000);
    expect(scrollShot(tall, frame, 0, "up")!.transform.y).toBeCloseTo(end.transform.y, 6);
  });

  it("leaves a panel that fits the frame alone, and never magnifies past the cap", () => {
    expect(scrollShot({ width: 1080, height: 900 }, frame, 0.5, "down")).toBeNull();
    const small = scrollShot({ width: 300, height: 1500 }, frame, 0, "down")!;
    expect(small.rect.width).toBe(300 * MAX_UPSCALE);
  });
});
