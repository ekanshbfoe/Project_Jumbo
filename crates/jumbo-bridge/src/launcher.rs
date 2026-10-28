//! Turns a session ticket into the Kaggle kernel that boots the worker.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

use crate::error::Result;
use crate::kaggle::KernelPush;
use crate::protocol::SessionTicket;

/// The worker's Python source and pyproject, bundled at build time by build.rs.
const WORKER_BUNDLE: &str = include_str!(concat!(env!("OUT_DIR"), "/worker_bundle.json"));
const BOOTSTRAP: &str = include_str!("bootstrap.py");

/// Where and how to run the worker on Kaggle.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchConfig {
    /// `owner/kernel-slug`. Create it once in Kaggle and attach the JUMBO_GIST_TOKEN secret to it.
    pub kernel_slug: String,
    /// `TpuV5E8` for production; `None` runs on a CPU machine, which is enough for bridge tests.
    pub machine_shape: Option<String>,
    pub model_sources: Vec<String>,
    pub dataset_sources: Vec<String>,
}

pub fn worker_bundle() -> &'static str {
    WORKER_BUNDLE
}

/// The self-contained kernel script: it unpacks the bundled worker, installs its pinned dependencies,
/// fetches cloudflared and runs the worker with this session's ticket.
pub fn bootstrap_script(ticket: &SessionTicket) -> Result<String> {
    Ok(BOOTSTRAP
        .replace("__TICKET__", &B64.encode(serde_json::to_vec(ticket)?))
        .replace("__BUNDLE__", &B64.encode(WORKER_BUNDLE)))
}

pub fn kernel_push(config: &LaunchConfig, ticket: &SessionTicket) -> Result<KernelPush> {
    let title = config.kernel_slug.split_once('/').map_or(config.kernel_slug.as_str(), |(_, slug)| slug).to_owned();
    Ok(KernelPush {
        slug: config.kernel_slug.clone(),
        title,
        source: bootstrap_script(ticket)?,
        machine_shape: config.machine_shape.clone(),
        dataset_sources: config.dataset_sources.clone(),
        model_sources: config.model_sources.clone(),
        kernel_sources: vec![],
    })
}
