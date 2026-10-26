import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { AudioManifest, buildEpisodeSpec, EpisodeSpec, LedgerEvent, MemoryDelta, PanelsDoc, ScriptDoc, SessionTicket } from "../src";

const fixture = (name: string) => JSON.parse(readFileSync(join(__dirname, "..", "fixtures", name), "utf8"));

// Shared with the Rust crates (crates/jumbo-ledger/tests/schema_conformance.rs): both sides must accept it.
describe("cross-language fixtures", () => {
  it("memory-delta.sample.json is a valid MemoryDelta", () => {
    const json = JSON.parse(readFileSync(join(__dirname, "..", "fixtures", "memory-delta.sample.json"), "utf8"));
    expect(MemoryDelta.safeParse(json).success).toBe(true);
  });

  it("every line of ledger.sample.jsonl (written by jumbo-ledger) is a valid LedgerEvent", () => {
    const lines = readFileSync(join(__dirname, "..", "fixtures", "ledger.sample.jsonl"), "utf8").trim().split("\n");
    expect(lines).toHaveLength(3);
    for (const line of lines) expect(LedgerEvent.parse(JSON.parse(line)).prev).toMatch(/^[0-9a-f]{64}$/);
  });

  it("session-ticket.sample.json is a valid SessionTicket", () => {
    const json = JSON.parse(readFileSync(join(__dirname, "..", "fixtures", "session-ticket.sample.json"), "utf8"));
    expect(SessionTicket.parse(json).rendezvous.kind).toBe("gist");
  });

  it("episode-spec.sample.json is what buildEpisodeSpec makes from the panels, script and audio samples", () => {
    const panels = PanelsDoc.parse(fixture("panels-doc.sample.json"));
    const script = ScriptDoc.parse(fixture("script-doc.sample.json"));
    const audio = AudioManifest.parse(fixture("audio-manifest.sample.json"));
    const expected = EpisodeSpec.parse(fixture("episode-spec.sample.json"));
    expect(buildEpisodeSpec(script, panels, audio)).toEqual(expected);
    // One beat has no clip yet, so its length comes from the narration estimate.
    expect(expected.beats.map((b) => b.audio === null)).toEqual([false, false, true]);
  });
});
