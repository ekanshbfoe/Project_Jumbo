//! Wire types shared with the worker (docs/bridge-protocol.md).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::crypto::{AppKeys, CryptoError};
use crate::error::{BridgeError, Result};

pub const PROTO: u32 = 1;

/// What the launcher embeds in the kernel it pushes. Contains no secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTicket {
    pub proto: u32,
    pub session_id: String,
    pub app_pk: String,
    pub deadline_s: u64,
    pub idle_s: u64,
    pub rendezvous: RendezvousSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RendezvousSpec {
    Gist { gist_id: String },
}

impl SessionTicket {
    /// A fresh ticket for `keys` with a random session id. 8.5 h deadline, 30 min idle limit.
    pub fn new(keys: &AppKeys, rendezvous: RendezvousSpec) -> Result<Self> {
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|e| CryptoError::Random(e.to_string()))?;
        Ok(Self {
            proto: PROTO,
            session_id: hex::encode(id),
            app_pk: keys.public_b64(),
            deadline_s: 30_600,
            idle_s: 1_800,
            rendezvous,
        })
    }
}

/// What the worker publishes for each tunnel epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RendezvousRecord {
    pub proto: u32,
    pub session_id: String,
    pub epoch: u64,
    pub worker_pk: String,
    pub sealed: String,
    pub ts: u64,
}

/// The sealed part of a record, readable only with the session's app key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedPayload {
    pub url: String,
    #[serde(default)]
    pub boot: Value,
}

/// Everything needed to talk to one worker epoch.
#[derive(Debug, Clone)]
pub struct WorkerEndpoint {
    pub session_id: String,
    pub epoch: u64,
    pub url: String,
    pub key: [u8; 32],
    pub boot: Value,
}

impl RendezvousRecord {
    /// Checks the record belongs to this session, opens the sealed URL and derives the session key.
    pub fn open(&self, keys: &AppKeys, session_id: &str) -> Result<WorkerEndpoint> {
        if self.proto != PROTO {
            return Err(BridgeError::Protocol(format!("worker speaks bridge protocol {}", self.proto)));
        }
        if self.session_id != session_id {
            return Err(BridgeError::Protocol("rendezvous record is for another session".into()));
        }
        let payload: SealedPayload = serde_json::from_slice(&keys.unseal(&self.sealed)?)?;
        let url = payload.url.trim_end_matches('/').to_owned();
        if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost"))
        {
            return Err(BridgeError::Protocol(format!("refusing non-HTTPS worker URL {url}")));
        }
        Ok(WorkerEndpoint {
            session_id: self.session_id.clone(),
            epoch: self.epoch,
            url,
            key: keys.session_key(&self.worker_pk, session_id)?,
            boot: payload.boot,
        })
    }
}

pub fn record_name(session_id: &str) -> String {
    format!("session-{session_id}.json")
}
