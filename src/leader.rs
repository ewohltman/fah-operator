//! Lease-based leader election for running the operator as multiple replicas.
//!
//! Every replica contends for a single [`Lease`](k8s_openapi::api::coordination::v1::Lease)
//! in `coordination.k8s.io`. Only the holder runs the controller; the rest stand
//! by. If the leader dies, its lease expires and a standby replica takes over,
//! so reconciliation continues without operator intervention.

use std::time::Duration;

use kube::Client;
use kube_leader_election::{LeaseLock, LeaseLockParams, LeaseLockResult};
use tokio::sync::oneshot;
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

/// Run the operator with leader election: contend for the lease, run the
/// controller while leading, and re-contend if leadership is ever lost. Never
/// returns under normal operation.
pub async fn run(client: Client, namespace: String, holder_id: String) -> Result<()> {
    info!(%namespace, %holder_id, lease = LEASE_NAME, "starting leader election");
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

                controller::run(client.clone(), async move {
                    let _ = lost_rx.await;
                })
                .await;

                renew.abort();
                info!("controller stopped; re-contending for leadership");
            }
            Ok(false) => debug!("another replica is leader; standing by"),
            // Never propagate: a standby replica must survive transient API
            // errors and keep contending rather than crashing the process.
            Err(err) => warn!(error = %err, "failed to check leadership lease; will retry"),
        }
        sleep(RENEW_INTERVAL).await;
    }
}
