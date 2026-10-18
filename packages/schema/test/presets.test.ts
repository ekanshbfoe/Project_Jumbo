import { describe, expect, it } from "vitest";
import { PRESET_CATALOG, PresetSpec } from "../src";

describe("PresetSpec", () => {
  it("fills preset params from defaults when omitted", () => {
    expect(PresetSpec.parse({ name: "zoom" })).toEqual({ name: "zoom", params: { from: 1, to: 1.15 } });
    expect(PresetSpec.parse({ name: "shake", params: { intensity: 30 } })).toEqual({
      name: "shake",
      params: { intensity: 30, frequency: 12 },
    });
  });

  it("rejects unknown presets and out-of-range params", () => {
    expect(PresetSpec.safeParse({ name: "spin" }).success).toBe(false);
    expect(PresetSpec.safeParse({ name: "action_tilt", params: { angle: 45 } }).success).toBe(false);
  });

  it("requires text for hud_popup", () => {
    expect(PresetSpec.safeParse({ name: "hud_popup" }).success).toBe(false);
    expect(PresetSpec.safeParse({ name: "hud_popup", params: { text: "LEVEL UP" } }).success).toBe(true);
  });

  it("exposes a described catalog for the Director prompt", () => {
    expect(PRESET_CATALOG.map((p) => p.name)).toContain("kinetic_jitter");
    expect(PRESET_CATALOG.every((p) => p.description.length > 10)).toBe(true);
  });
});
