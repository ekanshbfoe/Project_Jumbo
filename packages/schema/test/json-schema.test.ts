import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { CONTRACTS, OUT_DIR, renderContract } from "../scripts/export-json-schema";

describe("exported JSON Schema", () => {
  for (const name of Object.keys(CONTRACTS) as (keyof typeof CONTRACTS)[]) {
    it(`${name} is up to date (run npm run schema:export)`, () => {
      const committed = readFileSync(join(OUT_DIR, `${name}.schema.json`), "utf8");
      expect(committed).toBe(renderContract(name));
    });
  }

  it("marks defaulted fields optional for producers", () => {
    const script = JSON.parse(renderContract("ScriptDoc"));
    const line = script.properties.lines.items;
    expect(line.required).toEqual(expect.arrayContaining(["id", "panel_ids", "narration", "emotion", "preset"]));
    expect(line.required).not.toContain("voice");
  });
});
