//! Entrypoint for the `fah-operator` binary.
//!
//! Sets up logging and a Kubernetes client, then runs the reconcile loop behind
//! Lease-based leader election so multiple replicas can run for high availability.

use std::env;

use fah_operator::error::Result;
use fah_operator::leader;
use kube::Client;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let client = Client::try_default().await?;

    // Identify this replica and the namespace it runs in via the downward API.
    // Falls back to sane defaults for local `cargo run` against a kubeconfig.
    let holder_id = env::var("POD_NAME")
        .unwrap_or_else(|_| hostname().unwrap_or_else(|| "fah-operator-local".to_string()));
    let namespace = env::var("POD_NAMESPACE")
        .or_else(|_| env::var("OPERATOR_NAMESPACE"))
        .unwrap_or_else(|_| "default".to_string());

    info!(version = env!("CARGO_PKG_VERSION"), %holder_id, %namespace, "starting fah-operator");

    // Returns only on SIGTERM/SIGINT, after releasing the leadership lease.
    leader::run(client, namespace, holder_id).await?;
    info!("fah-operator shut down gracefully");
    Ok(())
}

/// Best-effort hostname for use as a leader-election holder id outside a pod.
fn hostname() -> Option<String> {
    env::var("HOSTNAME").ok()
}
