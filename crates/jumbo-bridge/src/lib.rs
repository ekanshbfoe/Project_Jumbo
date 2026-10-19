//! Desktop side of the Kaggle bridge (docs/bridge-protocol.md).

pub mod crypto;
pub mod error;
pub mod http;
pub mod kaggle;
pub mod launcher;
pub mod protocol;
pub mod rendezvous;
pub mod worker;

pub use error::{BridgeError, Result};
