import { z } from "zod";
import { ChapterId } from "./common";
import { MemoryOp } from "./memory";

/** Lowercase-hex blake3 digest. */
export const Hash = z.string().regex(/^[0-9a-f]{64}$/, "expected a 64-char lowercase hex hash");

/**
 * One line of `receipt_book/ledger.jsonl`. Written only by the desktop app (crates/jumbo-ledger);
 * `hash` covers every other field plus `prev`, so the lines form a tamper-evident chain.
 */
export const LedgerEvent = z
  .object({
    /** Position in the ledger, starting at 0. */
    seq: z.number().int().nonnegative(),
    /** Ops committed together share a commit number. */
    commit: z.number().int().nonnegative(),
    part: z.number().int().nonnegative(),
    parts: z.number().int().positive(),
    ts: z.iso.datetime({ offset: true }),
    chapter_id: ChapterId,
    session: z.string().min(1).optional(),
    op: MemoryOp,
    prev: Hash,
    hash: Hash,
  })
  .refine((e) => e.part < e.parts, { message: "part must be below parts", path: ["part"] });

export type LedgerEvent = z.infer<typeof LedgerEvent>;
