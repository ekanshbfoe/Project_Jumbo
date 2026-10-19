import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { MemoryDelta } from "../src";

// Shared with the Rust crates (crates/jumbo-ledger/tests/schema_conformance.rs): both sides must accept it.
describe("cross-language fixtures", () => {
  it("memory-delta.sample.json is a valid MemoryDelta", () => {
    const json = JSON.parse(readFileSync(join(__dirname, "..", "fixtures", "memory-delta.sample.json"), "utf8"));
    expect(MemoryDelta.safeParse(json).success).toBe(true);
  });
});
