//! Canonical JSON and hashing. Hashes must not depend on serde_json's map ordering feature (another
//! crate in the build can enable `preserve_order`), so object keys are always sorted explicitly.

use serde::Serialize;
use serde_json::Value;

/// Compact JSON with object keys sorted recursively.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, &mut out);
    out
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_value(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// blake3 of the canonical JSON of `value`, as lowercase hex.
pub fn hash_of<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    Ok(blake3::hash(canonical_json(&value).as_bytes()).to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_the_output() {
        let a = json!({"b": 1, "a": {"y": [1, {"d": 2, "c": 3}], "x": "s"}});
        let b = json!({"a": {"x": "s", "y": [1, {"c": 3, "d": 2}]}, "b": 1});
        assert_eq!(canonical_json(&a), r#"{"a":{"x":"s","y":[1,{"c":3,"d":2}]},"b":1}"#);
        assert_eq!(hash_of(&a).expect("hash"), hash_of(&b).expect("hash"));
    }

    #[test]
    fn escapes_strings() {
        assert_eq!(canonical_json(&json!({"q\"k": "line\nbreak"})), r#"{"q\"k":"line\nbreak"}"#);
    }
}
