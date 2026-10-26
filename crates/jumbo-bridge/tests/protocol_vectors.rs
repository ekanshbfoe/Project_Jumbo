//! The Python worker generated protocol/bridge-v1.vectors.json; the Rust bridge must reproduce its
//! session key and signatures exactly and open the sealed record Python produced.

use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use jumbo_bridge::crypto::{AppKeys, sign_request};
use serde_json::Value;

fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/bridge-v1.vectors.json");
    serde_json::from_str(&std::fs::read_to_string(&path).expect("read vectors")).expect("parse vectors")
}

fn app_keys(v: &Value) -> AppKeys {
    let bytes: [u8; 32] = B64.decode(v["app_sk"].as_str().expect("app_sk")).expect("b64").try_into().expect("32 bytes");
    AppKeys::from_secret_bytes(bytes)
}

#[test]
fn derives_the_same_session_key_as_python() {
    let v = vectors();
    let keys = app_keys(&v);
    assert_eq!(keys.public_b64(), v["app_pk"].as_str().expect("app_pk"));
    let key =
        keys.session_key(v["worker_pk"].as_str().expect("pk"), v["session_id"].as_str().expect("sid")).expect("key");
    assert_eq!(hex::encode(key), v["session_key_hex"].as_str().expect("hex"));
}

#[test]
fn produces_the_same_signatures_as_python() {
    let v = vectors();
    let key: [u8; 32] = hex::decode(v["session_key_hex"].as_str().expect("hex")).expect("hex").try_into().expect("32");
    for s in v["signatures"].as_array().expect("signatures") {
        let got = sign_request(
            &key,
            s["method"].as_str().expect("method"),
            s["path"].as_str().expect("path"),
            s["ts"].as_u64().expect("ts"),
            s["body"].as_str().expect("body").as_bytes(),
        );
        assert_eq!(got, s["sig"].as_str().expect("sig"));
    }
}

#[test]
fn opens_the_sealed_record_python_produced() {
    let v = vectors();
    let opened = app_keys(&v).unseal(v["sealed"]["box"].as_str().expect("box")).expect("unseal");
    assert_eq!(String::from_utf8(opened).expect("utf8"), v["sealed"]["message"].as_str().expect("message"));
    let stranger = AppKeys::generate().expect("keys");
    assert!(stranger.unseal(v["sealed"]["box"].as_str().expect("box")).is_err());
}

/// packages/schema validates the same ticket with zod, so the launcher's ticket matches the contract.
#[test]
fn parses_the_shared_session_ticket_fixture() {
    use jumbo_bridge::protocol::{RendezvousSpec, SessionTicket};
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/schema/fixtures/session-ticket.sample.json");
    let ticket: SessionTicket = serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("parse");
    assert_eq!(ticket.proto, jumbo_bridge::protocol::PROTO);
    assert_eq!(ticket.rendezvous, RendezvousSpec::Gist { gist_id: "f9b3e91ef19772d3d607cfbaf38f5b53".into() });
    let again: SessionTicket = serde_json::from_value(serde_json::to_value(&ticket).expect("ser")).expect("parse");
    assert_eq!(ticket, again);
}

/// Tickets the launcher really creates satisfy the shared schema.
#[test]
fn launcher_tickets_match_the_generated_contract() {
    use jumbo_bridge::crypto::AppKeys;
    use jumbo_bridge::protocol::{RendezvousSpec, SessionTicket};
    let ticket = SessionTicket::new(&AppKeys::generate().expect("keys"), RendezvousSpec::Gist { gist_id: "g".into() })
        .expect("ticket");
    let json = serde_json::to_string(&ticket).expect("ser");
    let generated: jumbo_contracts::session_ticket::SessionTicket =
        serde_json::from_str(&json).expect("schema accepts");
    assert_eq!(generated.session_id.as_str(), ticket.session_id);
}
