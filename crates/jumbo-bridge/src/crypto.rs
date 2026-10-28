//! Session keys, request signing and sealed rendezvous payloads (docs/bridge-protocol.md).
//! Must agree byte-for-byte with worker/jumbo_worker/crypto.py; protocol/bridge-v1.vectors.json
//! is the shared test fixture.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

const SALT: &[u8] = b"jumbo-bridge-v1";

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid base64: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("expected a 32-byte key")]
    KeyLength,
    #[error("sealed payload could not be opened with this session's key")]
    Unseal,
    #[error("no system randomness available: {0}")]
    Random(String),
}

/// The app's per-session X25519 keypair. Only the public half ever leaves the PC.
pub struct AppKeys {
    secret: StaticSecret,
}

impl AppKeys {
    pub fn generate() -> Result<Self, CryptoError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| CryptoError::Random(e.to_string()))?;
        Ok(Self { secret: StaticSecret::from(bytes) })
    }

    pub fn from_secret_bytes(bytes: [u8; 32]) -> Self {
        Self { secret: StaticSecret::from(bytes) }
    }

    /// The secret key, for keeping a running session across an app restart (stored like the other secrets).
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    pub fn public_b64(&self) -> String {
        B64.encode(PublicKey::from(&self.secret).as_bytes())
    }

    /// 32-byte session key shared with the worker whose public key is `worker_pk_b64`.
    pub fn session_key(&self, worker_pk_b64: &str, session_id: &str) -> Result<[u8; 32], CryptoError> {
        let peer: [u8; 32] = B64.decode(worker_pk_b64)?.try_into().map_err(|_| CryptoError::KeyLength)?;
        let shared = self.secret.diffie_hellman(&PublicKey::from(peer));
        let mut key = [0u8; 32];
        hkdf::Hkdf::<Sha256>::new(Some(SALT), shared.as_bytes())
            .expand(session_id.as_bytes(), &mut key)
            .map_err(|_| CryptoError::KeyLength)?;
        Ok(key)
    }

    /// Opens a libsodium sealed box addressed to this app (the rendezvous record's `sealed` field).
    pub fn unseal(&self, sealed_b64: &str) -> Result<Vec<u8>, CryptoError> {
        let sealed = B64.decode(sealed_b64)?;
        crypto_box::SecretKey::from(self.secret.to_bytes()).unseal(&sealed).map_err(|_| CryptoError::Unseal)
    }
}

/// `hex(HMAC-SHA256(K, "v1\n" METHOD "\n" PATH_AND_QUERY "\n" ts "\n" hex(blake3(body))))`.
pub fn sign_request(key: &[u8; 32], method: &str, path_and_query: &str, ts: u64, body: &[u8]) -> String {
    let message =
        format!("v1\n{}\n{path_and_query}\n{ts}\n{}", method.to_ascii_uppercase(), blake3::hash(body).to_hex());
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// The full `Authorization` header value for one request.
pub fn auth_header(
    session_id: &str,
    key: &[u8; 32],
    method: &str,
    path_and_query: &str,
    ts: u64,
    body: &[u8],
) -> String {
    format!("Jumbo {session_id}.{ts}.{}", sign_request(key, method, path_and_query, ts, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_cover_method_path_time_and_body() {
        let key = [7u8; 32];
        let base = sign_request(&key, "post", "/v1/jobs", 10, b"{}");
        assert_eq!(base, sign_request(&key, "POST", "/v1/jobs", 10, b"{}"));
        assert_ne!(base, sign_request(&key, "POST", "/v1/jobs?x", 10, b"{}"));
        assert_ne!(base, sign_request(&key, "POST", "/v1/jobs", 11, b"{}"));
        assert_ne!(base, sign_request(&key, "POST", "/v1/jobs", 10, b"{ }"));
        assert_eq!(base.len(), 64);
    }

    #[test]
    fn generated_keys_are_distinct() {
        let a = AppKeys::generate().expect("keys");
        let b = AppKeys::generate().expect("keys");
        assert_ne!(a.public_b64(), b.public_b64());
    }
}
