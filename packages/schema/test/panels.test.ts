import { describe, expect, it } from "vitest";
import { PanelsDoc } from "../src";

const panel = (id: string, order: number) => ({
  id,
  order,
  page: "chapters/ch_0001/pages/page_001.webp",
  page_size: { width: 1000, height: 1400 },
  bbox: { x: 0.05, y: 0.05, w: 0.9, h: 0.4 },
  polygon: [
    [0.05, 0.05],
    [0.95, 0.05],
    [0.95, 0.45],
    [0.05, 0.4],
  ],
});

const doc = (panels: unknown[]) => ({ schema_version: 1, chapter_id: "ch_0001", layout: "page", panels });

describe("PanelsDoc", () => {
  it("accepts a valid document and fills defaults", () => {
    const parsed = PanelsDoc.parse(doc([panel("p_0001", 0), panel("p_0002", 1)]));
    expect(parsed.panels[0]?.bubbles).toEqual([]);
    expect(parsed.panels[0]?.characters).toEqual([]);
  });

  it("rejects duplicate panel ids and reading orders", () => {
    const result = PanelsDoc.safeParse(doc([panel("p_0001", 0), panel("p_0001", 0)]));
    expect(result.success).toBe(false);
    expect(result.error?.issues.map((i) => i.message)).toEqual([
      "duplicate panel id p_0001",
      "duplicate reading order 0",
    ]);
  });

  it("rejects out-of-range coordinates and degenerate polygons", () => {
    const bad = { ...panel("p_0001", 0), polygon: [[0, 0], [1.2, 0]] };
    expect(PanelsDoc.safeParse(doc([bad])).success).toBe(false);
  });

  it("rejects a bbox that runs past the image", () => {
    const bad = { ...panel("p_0001", 0), bbox: { x: 0.5, y: 0, w: 0.6, h: 0.5 } };
    expect(PanelsDoc.safeParse(doc([bad])).success).toBe(false);
  });

  it("rejects Windows or absolute asset paths", () => {
    expect(PanelsDoc.safeParse(doc([{ ...panel("p_0001", 0), page: "C:\pages\a.webp" }])).success).toBe(false);
    expect(PanelsDoc.safeParse(doc([{ ...panel("p_0001", 0), page: "/abs/a.webp" }])).success).toBe(false);
  });

  it("ties every character box of a fingerprinted chapter to a row", () => {
    const fingerprints = { path: "chapters/ch_0001/panels/fingerprints.f16", dtype: "float16", dim: 768, count: 2, model: "magi" };
    const box = (id?: string) => ({ ...(id ? { id } : {}), cluster: "i001.c_0", bbox: { x: 0.1, y: 0.1, w: 0.2, h: 0.2 } });
    const withBoxes = (...boxes: unknown[]) => ({ ...doc([{ ...panel("p_0001", 0), characters: boxes }]), fingerprints });
    expect(PanelsDoc.safeParse(withBoxes(box("k_0001"), box("k_0002"))).success).toBe(true);
    const bad = PanelsDoc.safeParse(withBoxes(box("k_0001"), box("k_0001"), box(), box("k_0003")));
    expect(bad.error?.issues.map((i) => i.message)).toEqual([
      "duplicate character box id k_0001",
      "a chapter with fingerprints needs an id on every character box",
      "k_0003 has no fingerprint (the file has 2 rows)",
    ]);
  });
});
