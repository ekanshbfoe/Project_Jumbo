import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { LedgerEvent, MemoryDelta, SessionTicket } from "../src";

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
});
