import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { narrationProblem, ScriptDoc, scriptProblems } from "../src";

const line = (overrides: Record<string, unknown> = {}) => ({
  id: "l_0001",
  panel_ids: ["p_0001"],
  narration: "और तभी, hero ने पहली बार system की आवाज़ सुनी।",
  emotion: "mysterious",
  preset: { name: "zoom" },
  ...overrides,
});

const doc = (lines: unknown[]) => ({ schema_version: 1, chapter_id: "ch_0001", lines });

describe("ScriptDoc", () => {
  it("accepts a valid script and fills voice and transition defaults", () => {
    const parsed = ScriptDoc.parse(doc([line()]));
    expect(parsed.lines[0]?.voice).toEqual({ speaker: "narrator", tone: "neutral", pace: "normal", intensity: 0.5 });
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

describe("narration writing rule", () => {
  const cases = JSON.parse(readFileSync(new URL("../fixtures/hinglish-cases.json", import.meta.url), "utf8"));

  it("accepts Devanagari and Mixed Hinglish, cues included", () => {
    expect(cases.ok.map(narrationProblem)).toEqual(cases.ok.map(() => null));
  });

  it("refuses Hindi in Latin letters and lines that are mostly Latin", () => {
    for (const text of cases.romanized) expect(narrationProblem(text)).toMatch(/Latin letters/);
    for (const text of cases.too_little_devanagari) expect(narrationProblem(text)).toMatch(/at least half/);
    expect(narrationProblem(cases.romanized[0])).toContain("(ki, thi, lekin, kuch, tha)");
    const script = ScriptDoc.parse(doc([line(), line({ id: "l_0002", narration: cases.romanized[0] })]));
    expect(scriptProblems(script).map((p) => p.path)).toEqual([["lines", 1, "narration"]]);
  });
});
