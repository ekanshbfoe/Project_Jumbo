import { PanelsDoc, PRESET_CATALOG } from "@jumbo/schema";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { galleryEpisode, GALLERY_SHAPES, sampleEpisode } from "../src/fixtures/episodes";
import { FIXTURE_PAGES, pageSvg, panelsDoc } from "../src/fixtures/pages";
import { episodeTimeline } from "../src/timeline";

describe("fixtures", () => {
  it("produce a valid PanelsDoc", () => {
    expect(PanelsDoc.safeParse(panelsDoc()).success).toBe(true);
  });

  it("committed SVGs match their definitions (run npm run fixtures)", () => {
    for (const page of FIXTURE_PAGES) {
      expect(readFileSync(join(__dirname, "..", "public", page.file), "utf8")).toBe(pageSvg(page));
    }
  });

  it("gallery covers every preset on every shape", () => {
    const spec = galleryEpisode();
    expect(spec.beats).toHaveLength(PRESET_CATALOG.length * GALLERY_SHAPES.length);
    expect(new Set(spec.beats.map((b) => b.preset.name))).toEqual(new Set(PRESET_CATALOG.map((p) => p.name)));
  });

  it("sample episode mixes voiced and unvoiced beats", () => {
    const spec = sampleEpisode();
    expect(spec.beats.map((b) => b.audio !== null)).toEqual([true, false, true, false, false]);
    // 5 beats with three overlapping transitions (fade 12, wipe 9, slide 12 frames).
    const total = spec.beats.reduce((sum, b) => sum + Math.round(b.duration * 30), 0);
    expect(episodeTimeline(spec).durationInFrames).toBe(total - 33);
  });
});
