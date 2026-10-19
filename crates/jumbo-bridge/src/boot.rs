//! The headless boot sequence: push the kernel, follow it through Kaggle's queue, wait for the
//! worker's rendezvous record, open it and connect. Reports every state change so the UI can show
//! exactly where a boot is (or why it failed).

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::time::Instant;

use crate::crypto::AppKeys;
use crate::error::{BridgeError, Result};
use crate::kaggle::KaggleClient;
use crate::launcher::{LaunchConfig, kernel_push};
use crate::protocol::SessionTicket;
use crate::rendezvous::RendezvousReader;
use crate::worker::WorkerClient;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BootState {
    Pushing,
    Queued,
    Running,
    AwaitingRendezvous,
    Connecting { epoch: u64 },
    Ready { epoch: u64, boot: Value },
    Reconnecting { from_epoch: u64 },
}

#[derive(Debug, Clone)]
pub struct BootTimings {
    /// Give up if Kaggle has not started the kernel within this long (TPU queues can be slow).
    pub queue: Duration,
    /// Give up if the kernel is running but no reachable rendezvous appears within this long.
    pub rendezvous: Duration,
    /// How long to wait for a dropped worker to come back before declaring the session lost.
    pub reconnect: Duration,
    pub poll_status: Duration,
    pub poll_rendezvous: Duration,
}

impl Default for BootTimings {
    fn default() -> Self {
        Self {
            queue: Duration::from_secs(20 * 60),
            rendezvous: Duration::from_secs(10 * 60),
            reconnect: Duration::from_secs(3 * 60),
            poll_status: Duration::from_secs(30),
            poll_rendezvous: Duration::from_secs(3),
        }
    }
}

/// Where the worker comes from: a Kaggle kernel we push, or one already running (local development).
pub enum Launch<'a> {
    Kaggle { client: &'a KaggleClient, config: &'a LaunchConfig },
    External,
}

/// Opens the newest record above `min_epoch` and checks the worker answers through it.
async fn try_connect(
    http: &reqwest::Client,
    reader: &mut RendezvousReader,
    keys: &AppKeys,
    ticket: &SessionTicket,
    min_epoch: u64,
    on_state: &mut impl FnMut(BootState),
) -> Result<Option<WorkerClient>> {
    let Some(record) = reader.fetch(&ticket.session_id).await? else { return Ok(None) };
    if record.epoch < min_epoch {
        return Ok(None);
    }
    let endpoint = record.open(keys, &ticket.session_id)?;
    on_state(BootState::Connecting { epoch: endpoint.epoch });
    let client = WorkerClient::new(http.clone(), endpoint);
    if !client.health().await {
        return Ok(None);
    }
    client.session().await?;
    Ok(Some(client))
}

/// Boots (or attaches to) the worker for `ticket` and returns a connected client.
pub async fn boot(
    http: &reqwest::Client,
    launch: Launch<'_>,
    reader: &mut RendezvousReader,
    keys: &AppKeys,
    ticket: &SessionTicket,
    timings: &BootTimings,
    mut on_state: impl FnMut(BootState),
) -> Result<WorkerClient> {
    let kaggle = match launch {
        Launch::Kaggle { client, config } => {
            on_state(BootState::Pushing);
            client.push(&kernel_push(config, ticket)?).await?;
            on_state(BootState::Queued);
            Some((client, config.kernel_slug.as_str()))
        }
        Launch::External => {
            on_state(BootState::AwaitingRendezvous);
            None
        }
    };

    let started = Instant::now();
    let mut running_since = kaggle.is_none().then(Instant::now);
    let mut next_status = Instant::now();
    loop {
        if let Some((client, slug)) = kaggle
            && Instant::now() >= next_status
        {
            next_status = Instant::now() + timings.poll_status;
            let status = client.status(slug).await?;
            if status.is_finished() {
                let why = status.failure_message.filter(|m| !m.is_empty()).unwrap_or(status.status);
                return Err(BridgeError::Protocol(format!("Kaggle kernel stopped before the worker came up: {why}")));
            }
            if status.status == "RUNNING" && running_since.is_none() {
                running_since = Some(Instant::now());
                on_state(BootState::Running);
                on_state(BootState::AwaitingRendezvous);
            }
        }

        if running_since.is_some()
            && let Some(client) = try_connect(http, reader, keys, ticket, 1, &mut on_state).await?
        {
            let endpoint = client.endpoint();
            on_state(BootState::Ready { epoch: endpoint.epoch, boot: endpoint.boot.clone() });
            return Ok(client);
        }

        match running_since {
            None if started.elapsed() > timings.queue => {
                return Err(BridgeError::Timeout(format!(
                    "Kaggle did not start the kernel within {:?}",
                    timings.queue
                )));
            }
            Some(since) if since.elapsed() > timings.rendezvous => {
                return Err(BridgeError::Timeout(format!(
                    "the worker did not publish a reachable rendezvous within {:?}",
                    timings.rendezvous
                )));
            }
            _ => {}
        }
        tokio::time::sleep(timings.poll_rendezvous).await;
    }
}

/// After the connection to `from_epoch` drops: waits for the worker to come back, either on a new
/// tunnel epoch or on the same one once it answers again.
pub async fn reconnect(
    http: &reqwest::Client,
    reader: &mut RendezvousReader,
    keys: &AppKeys,
    ticket: &SessionTicket,
    from_epoch: u64,
    timings: &BootTimings,
    mut on_state: impl FnMut(BootState),
) -> Result<WorkerClient> {
    on_state(BootState::Reconnecting { from_epoch });
    let deadline = Instant::now() + timings.reconnect;
    while Instant::now() < deadline {
        if let Some(client) = try_connect(http, reader, keys, ticket, from_epoch, &mut on_state).await? {
            let endpoint = client.endpoint();
            on_state(BootState::Ready { epoch: endpoint.epoch, boot: endpoint.boot.clone() });
            return Ok(client);
        }
        tokio::time::sleep(timings.poll_rendezvous).await;
    }
    Err(BridgeError::Timeout(format!("the worker did not come back within {:?}", timings.reconnect)))
}
