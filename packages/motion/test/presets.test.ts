import { PRESET_CATALOG, PresetSpec } from "@jumbo/schema";
import { describe, expect, it } from "vitest";
import { IDENTITY_CAMERA } from "../src/components/Camera";
import { floatingLayout } from "../src/presets/floating";
import { evaluateCamera, PRESETS } from "../src/presets/registry";
import type { PresetContext } from "../src/presets/types";

const ctx = (frame: number, overrides: Partial<PresetContext> = {}): PresetContext => ({
  frame,
  durationInFrames: 90,
  fps: 30,
  frameSize: { width: 1920, height: 1080 },
  seed: "l_0001",
  index: 0,
  count: 1,
  ...overrides,
});

const examples = PRESET_CATALOG.map(({ name }) =>
  PresetSpec.parse(name === "hud_popup" ? { name, params: { text: "LEVEL UP" } } : { name }),
);

describe("preset registry", () => {
  it("implements every preset in the schema catalog", () => {
    expect(Object.keys(PRESETS).sort()).toEqual(PRESET_CATALOG.map((p) => p.name).sort());
  });

  it.each(examples)("$name is deterministic and finite on every frame", (spec) => {
    for (let f = 0; f < 90; f++) {
      const a = evaluateCamera(spec, ctx(f));
      expect(evaluateCamera(spec, ctx(f))).toEqual(a);
      for (const v of Object.values(a)) expect(Number.isFinite(v)).toBe(true);
      expect(a.opacity).toBeGreaterThanOrEqual(0);
      expect(a.opacity).toBeLessThanOrEqual(1);
    }
  });
});

describe("preset behaviour", () => {
  it("hold never moves", () => {
    expect(evaluateCamera(PresetSpec.parse({ name: "hold" }), ctx(45))).toEqual(IDENTITY_CAMERA);
  });

  it("zoom runs from `from` to `to` around the focus point", () => {
    const spec = PresetSpec.parse({ name: "zoom", params: { from: 1, to: 1.5, focus: [0.25, 0.75] } });
    expect(evaluateCamera(spec, ctx(0)).scale).toBeCloseTo(1);
    expect(evaluateCamera(spec, ctx(89)).scale).toBeCloseTo(1.5);
    expect(evaluateCamera(spec, ctx(0))).toMatchObject({ originX: 25, originY: 75 });
  });

  it("pan to the right moves the content left", () => {
    const spec = PresetSpec.parse({ name: "pan", params: { direction: "right", distance: 0.1 } });
    expect(evaluateCamera(spec, ctx(0)).x).toBeCloseTo(96);
    expect(evaluateCamera(spec, ctx(89)).x).toBeCloseTo(-96);
  });

  it("kinetic jitter decays", () => {
    const spec = PresetSpec.parse({ name: "kinetic_jitter", params: { intensity: 40, decay: 0.1 } });
    const peak = (from: number, to: number) =>
      Math.max(...Array.from({ length: to - from }, (_, i) => Math.abs(evaluateCamera(spec, ctx(from + i)).x)));
    expect(peak(60, 90)).toBeLessThan(peak(0, 30) / 5);
  });

  it("panoramic fade starts and ends transparent", () => {
    const spec = PresetSpec.parse({ name: "panoramic_fade" });
    expect(evaluateCamera(spec, ctx(0)).opacity).toBe(0);
    expect(evaluateCamera(spec, ctx(45)).opacity).toBe(1);
    expect(evaluateCamera(spec, ctx(89)).opacity).toBe(0);
  });

  it("split screen staggers each panel's entrance", () => {
    const spec = PresetSpec.parse({ name: "split_screen", params: { stagger: 0.5 } });
    const at = (frame: number, index: number) => evaluateCamera(spec, ctx(frame, { index, count: 2 }));
    expect(at(10, 0).opacity).toBe(1);
    expect(at(10, 1).opacity).toBe(0);
    expect(at(80, 1).opacity).toBe(1);
  });

  it("scales shake intensity with output resolution", () => {
    const spec = PresetSpec.parse({ name: "shake", params: { intensity: 20 } });
    const hd = evaluateCamera(spec, ctx(7)).x;
    const uhd = evaluateCamera(spec, ctx(7, { frameSize: { width: 3840, height: 2160 } })).x;
    expect(uhd).toBeCloseTo(hd * 2);
  });

  it("only hud_popup draws an overlay", () => {
    expect(Object.values(PRESETS).filter((p) => "Overlay" in p).map((p) => p.name)).toEqual(["hud_popup"]);
  });
});

describe("floating panels", () => {
  const spec = PresetSpec.parse({ name: "floating_panels" });

  it("fans tall panels across the frame with the centre panel in front", () => {
    const slots = floatingLayout(
      [
        { width: 400, height: 700 },
        { width: 400, height: 700 },
        { width: 400, height: 700 },
      ],
      { width: 1920, height: 1080 },
    );
    expect(slots.map((s) => s.zIndex)).toEqual([0, 3, 0]);
    expect(slots[0]!.left).toBeLessThan(slots[1]!.left);
    expect(slots.every((s) => s.height <= 1080 * 0.76 + 1e-6)).toBe(true);
  });

  it("turns outer panels toward the centre in 3D and settles in staggered", () => {
    const at = (frame: number, index: number) => evaluateCamera(spec, ctx(frame, { index, count: 3 }));
    expect(at(120, 0).rotateY).toBeGreaterThan(at(120, 2).rotateY);
    expect(at(3, 2).opacity).toBeLessThan(at(3, 0).opacity);
    expect(at(80, 2).opacity).toBe(1);
  });

  it("casts the heavy cinematic shadow at any resolution", () => {
    expect(PRESETS.floating_panels.panelShadow?.(2)).toBe("drop-shadow(0 40px 100px rgba(0, 0, 0, 0.8))");
  });
});
