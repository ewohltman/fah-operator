//! The reconcile loop: given a [`FoldingAtHome`], make the cluster match it.

use std::sync::Arc;
use std::time::Duration;

use futures::{Future, StreamExt};
use k8s_openapi::api::apps::v1::DaemonSet;
use k8s_openapi::api::core::v1::ServiceAccount;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, Time};
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::watcher::Config;
use kube::{Client, Resource, ResourceExt};
use serde_json::json;
use tracing::{error, info, warn};

use crate::crd::FoldingAtHome;
use crate::error::{Error, Result};
use crate::{MANAGER, resources};

/// Shared state handed to every reconcile invocation.
pub struct Context {
    /// Client used to read and apply resources.
    pub client: Client,
}

/// How often a healthy resource is re-reconciled even without changes.
const RESYNC: Duration = Duration::from_secs(300);

/// Backoff applied after a failed reconcile.
const ERROR_REQUEUE: Duration = Duration::from_secs(30);

/// Run the controller until `shutdown` resolves (used for leadership loss).
pub async fn run<S>(client: Client, shutdown: S)
where
    S: Future<Output = ()> + Send + Sync + 'static,
{
    let foldingathomes: Api<FoldingAtHome> = Api::all(client.clone());
    let daemon_sets: Api<DaemonSet> = Api::all(client.clone());
    let service_accounts: Api<ServiceAccount> = Api::all(client.clone());
    let context = Arc::new(Context { client });

    Controller::new(foldingathomes, Config::default())
        .owns(daemon_sets, Config::default())
        .owns(service_accounts, Config::default())
        .graceful_shutdown_on(shutdown)
        .run(reconcile, error_policy, context)
        .for_each(|result| async move {
            match result {
                Ok((obj, _action)) => info!(name = %obj.name, "reconciled"),
                Err(err) => warn!(error = %err, "reconcile queue error"),
            }
        })
        .await;
}

/// Bring the cluster in line with a single `FoldingAtHome`.
async fn reconcile(cr: Arc<FoldingAtHome>, ctx: Arc<Context>) -> Result<Action> {
    let namespace = cr
        .namespace()
        .ok_or(Error::MissingObjectKey("metadata.namespace"))?;
    let name = resources::child_name(&cr)?;
    info!(%namespace, %name, "reconciling FoldingAtHome");

    let service_account = resources::service_account(&cr)?;
    let daemon_set = resources::daemon_set(&cr)?;

    let sa_api: Api<ServiceAccount> = Api::namespaced(ctx.client.clone(), &namespace);
    let ds_api: Api<DaemonSet> = Api::namespaced(ctx.client.clone(), &namespace);

    let apply = PatchParams::apply(MANAGER).force();
    sa_api
        .patch(&name, &apply, &Patch::Apply(&service_account))
        .await?;
    let applied = ds_api
        .patch(&name, &apply, &Patch::Apply(&daemon_set))
        .await?;

    update_status(&ctx, &cr, &namespace, &applied).await?;

    Ok(Action::requeue(RESYNC))
}

/// Reflect the managed DaemonSet's rollout into the CR's `.status`.
async fn update_status(
    ctx: &Context,
    cr: &FoldingAtHome,
    namespace: &str,
    daemon_set: &DaemonSet,
) -> Result<()> {
    let name = resources::child_name(cr)?;
    let ds_status = daemon_set.status.as_ref();
    let desired = ds_status.map(|s| s.desired_number_scheduled).unwrap_or(0);
    let ready = ds_status.map(|s| s.number_ready).unwrap_or(0);

    let (status, reason, message) = if desired == 0 {
        (
            "Unknown",
            "NoNodes",
            "No nodes are scheduled to run the client".to_string(),
        )
    } else if ready >= desired {
        (
            "True",
            "AllReady",
            format!("{ready}/{desired} client pods ready"),
        )
    } else {
        (
            "False",
            "Progressing",
            format!("{ready}/{desired} client pods ready"),
        )
    };

    let condition = Condition {
        type_: "Ready".to_string(),
        status: status.to_string(),
        reason: reason.to_string(),
        message,
        observed_generation: cr.meta().generation,
        last_transition_time: Time(k8s_openapi::jiff::Timestamp::now()),
    };

    let patch = json!({
        "status": {
            "observedGeneration": cr.meta().generation,
            "daemonSetName": name,
            "desiredNodes": desired,
            "readyNodes": ready,
            "conditions": [condition],
        }
    });

    let api: Api<FoldingAtHome> = Api::namespaced(ctx.client.clone(), namespace);
    api.patch_status(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

/// Decide what to do when a reconcile fails: log and requeue with backoff.
fn error_policy(cr: Arc<FoldingAtHome>, error: &Error, _ctx: Arc<Context>) -> Action {
    error!(error = %error, name = %cr.name_any(), "reconcile failed; requeueing");
    Action::requeue(ERROR_REQUEUE)
}
