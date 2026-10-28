//! The headless boot sequence: push the kernel, follow it through Kaggle's queue, wait for the
//! worker's rendezvous record, open it, authenticate and wait for warm-up. Reports every state change
//! so the UI can show exactly where a boot is (or why it failed).

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::time::Instant;

use crate::crypto::AppKeys;
use crate::error::{BridgeError, Result};
use crate::kaggle::{KaggleClient, KernelStatus, log_tail};
use crate::launcher::{LaunchConfig, kernel_push};
use crate::protocol::SessionTicket;
use crate::rendezvous::RendezvousReader;
use crate::worker::WorkerClient;

/// Lines of Kaggle output shown with a failure.
const LOG_TAIL_LINES: usize = 40;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BootState {
    Pushing,
    Queued,
    Running,
    AwaitingRendezvous,
    Connecting {
        epoch: u64,
    },
    /// The tunnel answers; checking the worker accepts this session's signed requests.
    Authenticating {
        epoch: u64,
    },
    /// Authenticated; the worker is still loading models.
    Warming {
        epoch: u64,
    },
    Ready {
        epoch: u64,
        boot: Value,
    },
    Reconnecting {
        from_epoch: u64,
    },
    /// The kernel crashed during boot; it is pushed again once.
    Retrying {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct BootTimings {
    /// Give up if Kaggle has not started the kernel within this long (TPU queues can be slow).
    pub queue: Duration,
    /// Give up if the kernel is running but no reachable rendezvous appears within this long.
    pub rendezvous: Duration,
    /// Give up if an authenticated worker is still warming up after this long.
    pub warm: Duration,
    /// How long to wait for a dropped worker to come back before declaring the session lost.
    pub reconnect: Duration,
    /// Pause before pushing again after the kernel crashed during boot.
    pub retry_backoff: Duration,
    pub poll_status: Duration,
    pub poll_rendezvous: Duration,
}

impl Default for BootTimings {
    fn default() -> Self {
        Self {
            queue: Duration::from_secs(20 * 60),
            rendezvous: Duration::from_secs(10 * 60),
            warm: Duration::from_secs(30 * 60),
            reconnect: Duration::from_secs(3 * 60),
            retry_backoff: Duration::from_secs(30),
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

/// The tail of the kernel's output, or a note saying why it could not be fetched.
async fn kernel_log(client: &KaggleClient, slug: &str) -> String {
    match client.output_log(slug).await {
        Ok(log) => log_tail(&log, LOG_TAIL_LINES),
        Err(e) => format!("(could not fetch the Kaggle log: {e})"),
    }
}

fn why(status: KernelStatus) -> String {
    match status.failure_message.filter(|m| !m.is_empty()) {
        Some(reason) => format!("{} ({reason})", status.status),
        None => status.status,
    }
}

/// A poll's answer, with a transient failure (a dropped request, a 5xx) read as "nothing yet": the boot and
/// reconnect loops try again on their next round, and their own timeouts still bound the wait.
fn soft<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.is_transient() => Ok(None),
        Err(e) => Err(e),
    }
}

/// Opens the newest record at or above `min_epoch` and checks the worker answers through it.
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
    on_state(BootState::Authenticating { epoch: client.endpoint().epoch });
    client.session().await?;
    Ok(Some(client))
}

/// Waits until an authenticated worker reports it has finished warming up.
async fn wait_warm(client: &WorkerClient, timings: &BootTimings, on_state: &mut impl FnMut(BootState)) -> Result<()> {
    let deadline = Instant::now() + timings.warm;
    let mut announced = false;
    loop {
        let warming = soft(client.session().await)?.is_none_or(|info| info["warming"].as_bool().unwrap_or(false));
        if !warming {
            return Ok(());
        }
        if !announced {
            on_state(BootState::Warming { epoch: client.endpoint().epoch });
            announced = true;
        }
        if Instant::now() >= deadline {
            return Err(BridgeError::Timeout(format!("the worker was still warming up after {:?}", timings.warm)));
        }
        tokio::time::sleep(timings.poll_rendezvous).await;
    }
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
            Some((client, config))
        }
        Launch::External => {
            on_state(BootState::AwaitingRendezvous);
            None
        }
    };

    let mut retried = false;
    let mut started = Instant::now();
    let mut running_since = kaggle.is_none().then(Instant::now);
    let mut next_status = Instant::now();
    loop {
        if let Some((client, config)) = kaggle
            && Instant::now() >= next_status
        {
            let slug = config.kernel_slug.as_str();
            next_status = Instant::now() + timings.poll_status;
            let status = soft(client.status(slug).await)?;
            if let Some(status) = status.as_ref().filter(|s| s.is_finished()) {
                let crashed = status.status == "ERROR";
                let message = format!("Kaggle kernel stopped before the worker came up: {}", why(status.clone()));
                if crashed && !retried {
                    retried = true;
                    on_state(BootState::Retrying { reason: message });
                    tokio::time::sleep(timings.retry_backoff).await;
                    on_state(BootState::Pushing);
                    client.push(&kernel_push(config, ticket)?).await?;
                    on_state(BootState::Queued);
                    started = Instant::now();
                    running_since = None;
                    next_status = Instant::now() + timings.poll_status;
                    continue;
                }
                return Err(BridgeError::KernelFailed { message, log: kernel_log(client, slug).await });
            }
            if status.is_some_and(|s| s.status == "RUNNING") && running_since.is_none() {
                running_since = Some(Instant::now());
                on_state(BootState::Running);
                on_state(BootState::AwaitingRendezvous);
            }
        }

        if running_since.is_some()
            && let Some(client) = soft(try_connect(http, reader, keys, ticket, 1, &mut on_state).await)?.flatten()
        {
            wait_warm(&client, timings, &mut on_state).await?;
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
                let message =
                    format!("the worker did not publish a reachable rendezvous within {:?}", timings.rendezvous);
                return Err(match kaggle {
                    // The log usually says why (a missing secret, a failed install).
                    Some((client, config)) => {
                        BridgeError::KernelFailed { message, log: kernel_log(client, &config.kernel_slug).await }
                    }
                    None => BridgeError::Timeout(message),
                });
            }
            _ => {}
        }
        tokio::time::sleep(timings.poll_rendezvous).await;
    }
}

/// After the connection to `from_epoch` drops: waits for the worker to come back, either on a new
/// tunnel epoch or on the same one once it answers again. With `kernel`, a kernel that has finished
/// ends the wait at once as a lost session.
#[allow(clippy::too_many_arguments)]
pub async fn reconnect(
    http: &reqwest::Client,
    reader: &mut RendezvousReader,
    keys: &AppKeys,
    ticket: &SessionTicket,
    from_epoch: u64,
    kernel: Option<(&KaggleClient, &str)>,
    timings: &BootTimings,
    mut on_state: impl FnMut(BootState),
) -> Result<WorkerClient> {
    on_state(BootState::Reconnecting { from_epoch });
    let deadline = Instant::now() + timings.reconnect;
    let mut next_status = Instant::now();
    while Instant::now() < deadline {
        if let Some((client, slug)) = kernel
            && Instant::now() >= next_status
        {
            next_status = Instant::now() + timings.poll_status;
            if let Some(status) = soft(client.status(slug).await)?
                && status.is_finished()
            {
                let log = kernel_log(client, slug).await;
                return Err(BridgeError::SessionLost(format!("the Kaggle kernel ended: {}\n{log}", why(status))));
            }
        }
        if let Some(client) = soft(try_connect(http, reader, keys, ticket, from_epoch, &mut on_state).await)?.flatten()
        {
            wait_warm(&client, timings, &mut on_state).await?;
            let endpoint = client.endpoint();
            on_state(BootState::Ready { epoch: endpoint.epoch, boot: endpoint.boot.clone() });
            return Ok(client);
        }
        tokio::time::sleep(timings.poll_rendezvous).await;
    }
    Err(BridgeError::SessionLost(format!("the worker did not come back within {:?}", timings.reconnect)))
}
