import { describe, expect, it } from "vitest";
import { ScriptDoc } from "../src";

const line = (overrides: Record<string, unknown> = {}) => ({
  id: "l_0001",
  panel_ids: ["p_0001"],
  narration: "Aur tabhi, hero ne pehli baar system ki awaaz suni.",
  emotion: "mysterious",
  preset: { name: "zoom" },
  ...overrides,
});

const doc = (lines: unknown[]) => ({ schema_version: 1, chapter_id: "ch_0001", lines });

describe("ScriptDoc", () => {
  it("accepts a valid script and fills voice and transition defaults", () => {
    const parsed = ScriptDoc.parse(doc([line()]));
    expect(parsed.lines[0]?.voice).toEqual({ tone: "neutral", pace: "normal", intensity: 0.5 });
    expect(parsed.lines[0]?.transition_in).toEqual({ name: "cut", duration: 0.4 });
  });

  it("enforces panel counts per preset", () => {
    expect(ScriptDoc.safeParse(doc([line({ panel_ids: ["p_0001", "p_0002"] })])).success).toBe(false);
    expect(ScriptDoc.safeParse(doc([line({ preset: { name: "split_screen" } })])).success).toBe(false);
    expect(
      ScriptDoc.safeParse(doc([line({ preset: { name: "split_screen" }, panel_ids: ["p_0001", "p_0002"] })])).success,
    ).toBe(true);
  });

  it("rejects duplicate line ids", () => {
    expect(ScriptDoc.safeParse(doc([line(), line()])).success).toBe(false);
  });
});
