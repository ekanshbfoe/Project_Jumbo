//! Types generated from packages/schema at build time (see build.rs), one module per contract. The
//! hand-written types in jumbo-ledger and jumbo-bridge are tested against these, so they cannot
//! silently drift from the shared contract.
//!
//! Cross-field rules (unique ids, panel counts, receipt chapters) are not part of JSON Schema; the
//! ledger validator and zod enforce them.

macro_rules! contract {
    ($module:ident, $doc:literal) => {
        #[doc = $doc]
        #[allow(
            clippy::all,
            clippy::pedantic,
            clippy::unwrap_used,
            dead_code,
            missing_docs,
            unused_imports,
            irrefutable_let_patterns
        )]
        pub mod $module {
            include!(concat!(env!("OUT_DIR"), "/", stringify!($module), ".rs"));
        }
    };
}

contract!(panels_doc, "`PanelsDoc`: detected panels of one chapter.");
contract!(script_doc, "`ScriptDoc`: the Director's narration lines and presets.");
contract!(audio_manifest, "`AudioManifest`: voiced clips per line.");
contract!(episode_spec, "`EpisodeSpec`: everything Remotion needs to render a chapter.");
contract!(memory_delta, "`MemoryDelta`: memory ops the Librarian proposes.");
contract!(ledger_event, "`LedgerEvent`: one line of the Receipt Book ledger.");
contract!(session_ticket, "`SessionTicket`: what the launcher embeds in the kernel.");
