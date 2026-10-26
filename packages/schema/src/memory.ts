import { z } from "zod";
import { ChapterId, PanelId, SchemaVersion } from "./common";

export const EntityType = z.enum(["character", "faction", "location", "item", "relationship", "plot_thread"]);

/** Stable ledger key, e.g. "character_sung_jinwoo" or "plot_thread_double_dungeon". */
export const EntityId = z.string().regex(/^[a-z]+(?:_[a-z0-9]+)+$/, "expected <type>_<slug> in snake_case");

/** Evidence for a memory op. Ops without receipts are rejected, which is the main defence against hallucinated facts. */
export const Receipt = z.object({
  chapter_id: ChapterId,
  panel_id: PanelId,
  /** Verbatim dialogue or a short description of what the panel shows. */
  quote: z.string().max(500).optional(),
}).meta({ id: "Receipt" });

const Receipts = z.array(Receipt).min(1);
const Confidence = z.number().min(0).max(1);

const entityRef = {
  entity_type: EntityType,
  entity_id: EntityId,
};

export const AddOp = z.object({
  op: z.literal("ADD"),
  ...entityRef,
  data: z.record(z.string(), z.unknown()),
  receipts: Receipts,
  confidence: Confidence,
}).meta({ id: "AddOp" });

export const UpdateOp = z.object({
  op: z.literal("UPDATE"),
  ...entityRef,
  /** Dot path into the entity, e.g. "status" or "abilities.shadow_extraction". */
  field: z.string().regex(/^[a-z0-9_]+(?:\.[a-z0-9_]+)*$/),
  value: z.unknown(),
  receipts: Receipts,
  confidence: Confidence,
}).meta({ id: "UpdateOp" });

export const DeleteOp = z.object({
  op: z.literal("DELETE"),
  ...entityRef,
  reason: z.string().min(1).max(300),
  receipts: Receipts,
  confidence: Confidence,
}).meta({ id: "DeleteOp" });

export const NoopOp = z.object({
  op: z.literal("NOOP"),
  reason: z.string().max(300).optional(),
}).meta({ id: "NoopOp" });

export const MemoryOp = z.discriminatedUnion("op", [AddOp, UpdateOp, DeleteOp, NoopOp]);

/** Ops the Librarian proposes for one chapter. The desktop app validates them against the ledger before committing. */
export const MemoryDelta = z
  .object({
    schema_version: SchemaVersion,
    chapter_id: ChapterId,
    /** Ledger length the ops were generated against, used to detect stale deltas. */
    base_ledger_offset: z.number().int().nonnegative(),
    ops: z.array(MemoryOp),
  })
  .superRefine((delta, ctx) => {
    const added = new Set<string>();
    delta.ops.forEach((op, i) => {
      if (op.op === "NOOP") return;
      if (op.op === "ADD") {
        if (added.has(op.entity_id)) ctx.addIssue({ code: "custom", path: ["ops", i], message: `${op.entity_id} added twice` });
        added.add(op.entity_id);
      }
      if (!op.entity_id.startsWith(`${op.entity_type}_`)) {
        ctx.addIssue({ code: "custom", path: ["ops", i, "entity_id"], message: `${op.entity_id} does not match type ${op.entity_type}` });
      }
      op.receipts.forEach((r, j) => {
        if (r.chapter_id > delta.chapter_id) {
          ctx.addIssue({ code: "custom", path: ["ops", i, "receipts", j], message: `receipt cites future chapter ${r.chapter_id}` });
        }
      });
    });
  });

export type EntityType = z.infer<typeof EntityType>;
export type Receipt = z.infer<typeof Receipt>;
export type MemoryOp = z.infer<typeof MemoryOp>;
export type MemoryDelta = z.infer<typeof MemoryDelta>;
