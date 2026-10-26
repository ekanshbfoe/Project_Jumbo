import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { CATALOG_FILE, CONTRACTS, OUT_DIR, renderCatalog, renderContract } from "../scripts/export-json-schema";

describe("exported JSON Schema", () => {
  for (const name of Object.keys(CONTRACTS) as (keyof typeof CONTRACTS)[]) {
    it(`${name} is up to date (run npm run schema:export)`, () => {
      const committed = readFileSync(join(OUT_DIR, `${name}.schema.json`), "utf8");
      expect(committed).toBe(renderContract(name));
    });
  }

  it(`${CATALOG_FILE} is up to date (run npm run schema:export)`, () => {
    expect(readFileSync(join(OUT_DIR, CATALOG_FILE), "utf8")).toBe(renderCatalog());
    const catalog = JSON.parse(renderCatalog());
    expect(catalog.multi_panel_presets).toEqual(["floating_panels", "horizontal_slices", "split_screen"]);
    expect(catalog.presets).toHaveLength(11);
  });

  it("names shared types so code generators reuse them", () => {
    const delta = JSON.parse(renderContract("MemoryDelta"));
    expect(Object.keys(delta.$defs)).toEqual(expect.arrayContaining(["AddOp", "UpdateOp", "DeleteOp", "NoopOp", "Receipt"]));
    const episode = JSON.parse(renderContract("EpisodeSpec"));
    expect(Object.keys(episode.$defs)).toEqual(expect.arrayContaining(["Beat", "EpisodePanel", "ActionTiltPreset", "HudPopupPreset"]));
    expect(episode.$defs.HoldPreset.description).toMatch(/^Static panel/);
  });

  it("marks defaulted fields optional for producers", () => {
    const script = JSON.parse(renderContract("ScriptDoc"));
    expect(script.properties.lines.items).toEqual({ $ref: "#/$defs/ScriptLine" });
    const line = script.$defs.ScriptLine;
    expect(line.required).toEqual(expect.arrayContaining(["id", "panel_ids", "narration", "emotion", "preset"]));
    expect(line.required).not.toContain("voice");
  });
});
