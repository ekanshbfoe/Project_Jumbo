//! Generates one module of Rust types per contract in packages/schema/json, so the Rust side can
//! never drift from the zod source. Run `npm run codegen` after changing the zod schemas.

use std::path::PathBuf;

use serde_json::{Map, Value};

/// Contract name -> module name, as in worker/scripts/codegen.py.
const CONTRACTS: &[(&str, &str)] = &[
    ("PanelsDoc", "panels_doc"),
    ("ScriptDoc", "script_doc"),
    ("AudioManifest", "audio_manifest"),
    ("EpisodeSpec", "episode_spec"),
    ("MemoryDelta", "memory_delta"),
    ("LedgerEvent", "ledger_event"),
    ("SessionTicket", "session_ticket"),
];

/// zod writes JSON Schema 2020-12; typify reads draft-07, so the 2020-12 spellings are translated:
/// `$defs` → `definitions`, tuple `prefixItems` → `items: [..]`, and `const` → a one-value `enum`
/// (typify ignores numeric `const`, which would let `proto: 2` through).
///
/// Formats are dropped so timestamps stay exact strings: the ledger hashes the text it wrote,
/// nanoseconds included. `{}` defaults (zod `prefault`) are dropped too, because typify cannot build
/// them for objects whose own fields have defaults; those fields become `Option`s, fine for reading.
fn to_draft7(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            let tuple = map.contains_key("prefixItems");
            for (key, value) in map {
                match key.as_str() {
                    "format" | "$schema" | "$id" => {}
                    "default" if value.as_object().is_some_and(Map::is_empty) => {}
                    "$defs" => {
                        out.insert("definitions".to_owned(), to_draft7(value));
                    }
                    "$ref" => {
                        let target = value.as_str().unwrap_or_default().replacen("#/$defs/", "#/definitions/", 1);
                        out.insert(key, Value::String(target));
                    }
                    "prefixItems" => {
                        out.insert("items".to_owned(), to_draft7(value));
                        out.insert("additionalItems".to_owned(), Value::Bool(false));
                    }
                    "items" if tuple => {}
                    "const" => {
                        out.insert("enum".to_owned(), Value::Array(vec![value]));
                    }
                    _ => {
                        out.insert(key, to_draft7(value));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(to_draft7).collect()),
        other => other,
    }
}

fn main() {
    let schemas =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("../../packages/schema/json");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir"));
    for (contract, module) in CONTRACTS {
        let path = schemas.join(format!("{contract}.schema.json"));
        println!("cargo:rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let json = to_draft7(serde_json::from_str(&text).expect("valid JSON"));
        let schema: schemars::schema::RootSchema =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("{contract}: not a JSON Schema: {e}"));
        let mut space = typify::TypeSpace::new(
            typify::TypeSpaceSettings::default().with_struct_builder(false).with_derive("PartialEq".to_owned()),
        );
        space.add_root_schema(schema).unwrap_or_else(|e| panic!("{contract}: {e}"));
        std::fs::write(out.join(format!("{module}.rs")), space.to_stream().to_string()).expect("write");
    }
}
