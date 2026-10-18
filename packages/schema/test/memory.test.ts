import { describe, expect, it } from "vitest";
import { MemoryDelta } from "../src";

const receipt = { chapter_id: "ch_0003", panel_id: "p_0012", quote: "Main ab kamzor nahi hoon." };

const delta = (ops: unknown[]) => ({ schema_version: 1, chapter_id: "ch_0003", base_ledger_offset: 41, ops });

describe("MemoryDelta", () => {
  it("accepts each op kind", () => {
    const parsed = MemoryDelta.parse(
      delta([
        { op: "ADD", entity_type: "character", entity_id: "character_hae_in", data: { name: "Hae-In" }, receipts: [receipt], confidence: 0.9 },
        { op: "UPDATE", entity_type: "character", entity_id: "character_jinwoo", field: "rank", value: "S", receipts: [receipt], confidence: 0.8 },
        { op: "DELETE", entity_type: "plot_thread", entity_id: "plot_thread_missing_father", reason: "resolved", receipts: [receipt], confidence: 0.7 },
        { op: "NOOP" },
      ]),
    );
    expect(parsed.ops).toHaveLength(4);
  });

  it("rejects ops without receipts", () => {
    const op = { op: "UPDATE", entity_type: "character", entity_id: "character_jinwoo", field: "rank", value: "S", receipts: [], confidence: 0.8 };
    expect(MemoryDelta.safeParse(delta([op])).success).toBe(false);
  });

  it("rejects ids that do not match their entity type", () => {
    const op = { op: "ADD", entity_type: "faction", entity_id: "character_hae_in", data: {}, receipts: [receipt], confidence: 1 };
    expect(MemoryDelta.safeParse(delta([op])).error?.issues[0]?.message).toBe("character_hae_in does not match type faction");
  });

  it("rejects receipts from chapters after the delta's chapter", () => {
    const op = { op: "ADD", entity_type: "item", entity_id: "item_dagger", data: {}, receipts: [{ ...receipt, chapter_id: "ch_0004" }], confidence: 1 };
    expect(MemoryDelta.safeParse(delta([op])).error?.issues[0]?.message).toBe("receipt cites future chapter ch_0004");
  });

  it("rejects adding the same entity twice in one delta", () => {
    const op = { op: "ADD", entity_type: "item", entity_id: "item_dagger", data: {}, receipts: [receipt], confidence: 1 };
    expect(MemoryDelta.safeParse(delta([op, op])).success).toBe(false);
  });
});
