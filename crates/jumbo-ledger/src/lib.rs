//! The Receipt Book: Project Jumbo's master story memory.
//!
//! `receipt_book/ledger.jsonl` is an append-only, hash-chained log of memory ops. Everything else
//! (entity views, snapshots) is derived from it. Kaggle only proposes deltas; this crate validates
//! them against the current state and is the only code that writes the ledger.

pub mod canonical;
pub mod log;
pub mod ops;
pub mod state;
pub mod validate;
pub mod views;

pub use log::{Ledger, LedgerError, LedgerEvent, Repair};
pub use ops::{EntityType, MemoryDelta, MemoryOp, Receipt};
pub use state::{ApplyError, BookState, Entity};
pub use validate::{Finding, PanelIndex, ProjectPanelIndex, Severity, ValidationReport, validate};
