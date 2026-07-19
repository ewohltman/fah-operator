//! Lease-based leader election for running the operator as multiple replicas.
//!
//! Every replica contends for a single [`Lease`](k8s_openapi::api::coordination::v1::Lease)
//! in `coordination.k8s.io`. Only the holder runs the controller; the rest stand
//! by. If the leader dies, its lease expires and a standby replica takes over,
//! so reconciliation continues without operator intervention.

use std::time::Duration;

use kube::Client;
use kube_leader_election::{LeaseLock, LeaseLockParams, LeaseLockResult};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{oneshot, watch};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::controller;
use crate::error::{Error, Result};

/// Name of the Lease all replicas contend for.
const LEASE_NAME: &str = "fah-operator-leader";

/// How long a lease is valid before a standby replica may claim it.
const LEASE_TTL: Duration = Duration::from_secs(15);

/// How often the leader renews the lease (well under [`LEASE_TTL`]).
const RENEW_INTERVAL: Duration = Duration::from_secs(5);

/// Consecutive renewal errors tolerated before stepping down. At
/// [`RENEW_INTERVAL`] apart this stays within [`LEASE_TTL`], so a transient API
/// blip does not cause leadership thrashing while still releasing the lease
/// before it can expire and be taken by another replica.
const MAX_CONSECUTIVE_RENEW_FAILURES: u32 = 2;

fn params(holder_id: &str) -> LeaseLockParams {
    LeaseLockParams {
        holder_id: holder_id.to_string(),
        lease_name: LEASE_NAME.to_string(),
        lease_ttl: LEASE_TTL,
    }
}

/// Attempt to acquire or renew the lease once, returning whether we hold it.
async fn try_acquire(client: &Client, namespace: &str, holder_id: &str) -> Result<bool> {
    let lock = LeaseLock::new(client.clone(), namespace, params(holder_id));
    let result = lock
        .try_acquire_or_renew()
        .await
        .map_err(|e| Error::LeaderElection(e.to_string()))?;
    Ok(matches!(result, LeaseLockResult::Acquired(_)))
}

/// Renew the lease on an interval; signal `lost` the first time renewal fails
/// or the lease is taken by another replica.
async fn renew_loop(
    client: Client,
    namespace: String,
    holder_id: String,
    lost: oneshot::Sender<()>,
) {
    let mut consecutive_failures = 0u32;
    loop {
        sleep(RENEW_INTERVAL).await;
        match try_acquire(&client, &namespace, &holder_id).await {
            Ok(true) => {
                consecutive_failures = 0;
                debug!("renewed leadership lease");
            }
            Ok(false) => {
                warn!("leadership lease was taken by another replica");
                break;
            }
            Err(err) => {
                consecutive_failures += 1;
                warn!(
                    error = %err,
                    consecutive_failures,
                    "failed to renew leadership lease"
                );
                if consecutive_failures >= MAX_CONSECUTIVE_RENEW_FAILURES {
                    warn!("giving up leadership after repeated renewal failures");
                    break;
                }
            }
        }
    }
    let _ = lost.send(());
}

/// Watch for SIGTERM/SIGINT; the returned channel flips to `true` on receipt.
///
/// Installing a handler turns pod termination from an abrupt kill into a
/// graceful shutdown: the leader gets a chance to release its lease so a
/// standby can take over immediately instead of waiting out [`LEASE_TTL`].
fn termination_signal() -> watch::Receiver<bool> {
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        let (Ok(mut sigterm), Ok(mut sigint)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) else {
            warn!("failed to install signal handlers; skipping graceful shutdown");
            return;
        };
        tokio::select! {
            _ = sigterm.recv() => info!("received SIGTERM"),
            _ = sigint.recv() => info!("received SIGINT"),
        }
        let _ = tx.send(true);
        // Keep the sender alive so receivers observe `true` instead of a
        // closed channel while shutdown completes.
        std::future::pending::<()>().await;
    });
    rx
}

/// Resolve once `term` signals termination; never resolves if the signal task
/// died without signalling (so a select arm using this cannot fire spuriously).
async fn terminated(term: &mut watch::Receiver<bool>) {
    if term.wait_for(|stop| *stop).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Best-effort lease release so a standby takes over in one renew interval
/// instead of waiting out [`LEASE_TTL`].
async fn step_down(client: &Client, namespace: &str, holder_id: &str) {
    let lock = LeaseLock::new(client.clone(), namespace, params(holder_id));
    match lock.step_down().await {
        Ok(()) => info!("released leadership lease"),
        Err(err) => {
            warn!(error = %err, "failed to release leadership lease; standbys must wait for expiry");
        }
    }
}

/// Run the operator with leader election: contend for the lease, run the
/// controller while leading, and re-contend if leadership is ever lost.
/// Returns only when a termination signal is received, releasing the lease
/// first if this replica is the leader.
pub async fn run(client: Client, namespace: String, holder_id: String) -> Result<()> {
    info!(%namespace, %holder_id, lease = LEASE_NAME, "starting leader election");
    let mut term = termination_signal();
    loop {
        match try_acquire(&client, &namespace, &holder_id).await {
            Ok(true) => {
                info!("acquired leadership; starting controller");
                let (lost_tx, lost_rx) = oneshot::channel();
                let renew = tokio::spawn(renew_loop(
                    client.clone(),
                    namespace.clone(),
                    holder_id.clone(),
                    lost_tx,
                ));

                // Stop the controller on leadership loss or termination.
                let (shutdown_tx, shutdown_rx) = oneshot::channel();
                let mut term_trigger = term.clone();
                let trigger = tokio::spawn(async move {
                    tokio::select! {
                        _ = lost_rx => {}
                        () = terminated(&mut term_trigger) => {}
                    }
                    let _ = shutdown_tx.send(());
                });

                controller::run(client.clone(), async move {
                    let _ = shutdown_rx.await;
                })
                .await;

                renew.abort();
                trigger.abort();

                if *term.borrow() {
                    info!("controller stopped for termination; stepping down");
                    step_down(&client, &namespace, &holder_id).await;
                    return Ok(());
                }
                info!("controller stopped; re-contending for leadership");
            }
            Ok(false) => debug!("another replica is leader; standing by"),
            // Never propagate: a standby replica must survive transient API
            // errors and keep contending rather than crashing the process.
            Err(err) => warn!(error = %err, "failed to check leadership lease; will retry"),
        }
        tokio::select! {
            _ = sleep(RENEW_INTERVAL) => {}
            () = terminated(&mut term) => {
                info!("terminating; exiting leader election");
                return Ok(());
            }
        }
    }
}
