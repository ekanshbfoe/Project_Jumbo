//! The Receipt Book: Project Jumbo's master story memory.
//!
//! `receipt_book/ledger.jsonl` is an append-only, hash-chained log of memory ops. Everything else
//! (entity views, snapshots) is derived from it. Kaggle only proposes deltas; this crate validates
//! them against the current state and is the only code that writes the ledger.

pub mod canonical;
pub mod ops;

pub use ops::{EntityType, MemoryDelta, MemoryOp, Receipt};
