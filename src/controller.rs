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

use crate::crd::{FoldingAtHome, FoldingAtHomeStatus};
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

    // `any_semantic` lets relists be served from the apiserver watch cache
    // instead of forcing quorum reads from etcd; staleness is bounded by the
    // watch and the periodic resync.
    let crs = Config::default().any_semantic();
    // Scope the owned watches to objects this operator created (every child is
    // stamped with the managed-by label by `resources::labels`). Without the
    // selector the controller watches and caches every DaemonSet and
    // ServiceAccount in the cluster — including one `default` ServiceAccount
    // per namespace — for no benefit.
    let managed = Config::default()
        .labels(&format!("app.kubernetes.io/managed-by={MANAGER}"))
        .any_semantic();

    Controller::new(foldingathomes, crs)
        .owns(daemon_sets, managed.clone())
        .owns(service_accounts, managed)
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

/// Compute the `.status` a `FoldingAtHome` should have given its DaemonSet.
///
/// Pure so it can be unit-tested and diffed against the observed status. Per
/// Kubernetes API conventions, `lastTransitionTime` is carried over from the
/// existing `Ready` condition unless the condition's status actually flipped —
/// a fresh timestamp on every reconcile would make each write change content,
/// and every content change bumps `resourceVersion`, re-triggering reconcile
/// in a self-sustaining write loop.
fn desired_status(cr: &FoldingAtHome, daemon_set: &DaemonSet) -> Result<FoldingAtHomeStatus> {
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

    let last_transition_time = cr
        .status
        .as_ref()
        .and_then(|s| s.conditions.iter().find(|c| c.type_ == "Ready"))
        .filter(|c| c.status == status)
        .map(|c| c.last_transition_time.clone())
        .unwrap_or_else(|| Time(k8s_openapi::jiff::Timestamp::now()));

    let condition = Condition {
        type_: "Ready".to_string(),
        status: status.to_string(),
        reason: reason.to_string(),
        message,
        observed_generation: cr.meta().generation,
        last_transition_time,
    };

    Ok(FoldingAtHomeStatus {
        observed_generation: cr.meta().generation,
        daemon_set_name: Some(name),
        desired_nodes: Some(desired),
        ready_nodes: Some(ready),
        conditions: vec![condition],
    })
}

/// Reflect the managed DaemonSet's rollout into the CR's `.status`, skipping
/// the API write when the observed status already matches.
async fn update_status(
    ctx: &Context,
    cr: &FoldingAtHome,
    namespace: &str,
    daemon_set: &DaemonSet,
) -> Result<()> {
    let status = desired_status(cr, daemon_set)?;
    if cr.status.as_ref() == Some(&status) {
        return Ok(());
    }

    let name = resources::child_name(cr)?;
    let api: Api<FoldingAtHome> = Api::namespaced(ctx.client.clone(), namespace);
    api.patch_status(
        &name,
        &PatchParams::default(),
        &Patch::Merge(&json!({ "status": status })),
    )
    .await?;
    Ok(())
}

/// Decide what to do when a reconcile fails: log and requeue with backoff.
fn error_policy(cr: Arc<FoldingAtHome>, error: &Error, _ctx: Arc<Context>) -> Action {
    error!(error = %error, name = %cr.name_any(), "reconcile failed; requeueing");
    Action::requeue(ERROR_REQUEUE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::FoldingAtHomeSpec;
    use k8s_openapi::api::apps::v1::DaemonSetStatus;
    use k8s_openapi::jiff::Timestamp;

    fn sample_cr() -> FoldingAtHome {
        let mut cr = FoldingAtHome::new("sample", FoldingAtHomeSpec::default());
        cr.metadata.namespace = Some("fah".to_string());
        cr.metadata.generation = Some(3);
        cr
    }

    fn sample_ds(desired: i32, ready: i32) -> DaemonSet {
        DaemonSet {
            status: Some(DaemonSetStatus {
                desired_number_scheduled: desired,
                number_ready: ready,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn status_reflects_daemon_set_counts() {
        let status = desired_status(&sample_cr(), &sample_ds(4, 4)).unwrap();
        assert_eq!(status.observed_generation, Some(3));
        assert_eq!(status.daemon_set_name, Some("sample".to_string()));
        assert_eq!(status.desired_nodes, Some(4));
        assert_eq!(status.ready_nodes, Some(4));
        assert_eq!(status.conditions.len(), 1);
        assert_eq!(status.conditions[0].type_, "Ready");
        assert_eq!(status.conditions[0].status, "True");
        assert_eq!(status.conditions[0].reason, "AllReady");
    }

    #[test]
    fn status_progressing_and_no_nodes() {
        let progressing = desired_status(&sample_cr(), &sample_ds(4, 1)).unwrap();
        assert_eq!(progressing.conditions[0].status, "False");
        assert_eq!(progressing.conditions[0].reason, "Progressing");

        let no_nodes = desired_status(&sample_cr(), &sample_ds(0, 0)).unwrap();
        assert_eq!(no_nodes.conditions[0].status, "Unknown");
        assert_eq!(no_nodes.conditions[0].reason, "NoNodes");
    }

    #[test]
    fn unchanged_condition_keeps_transition_time_and_status_is_stable() {
        // Reconcile once, store the result as the observed status, and
        // reconcile again: the recomputed status must compare equal so the
        // controller skips the write instead of re-triggering itself forever.
        let mut cr = sample_cr();
        let epoch = Time(Timestamp::UNIX_EPOCH);
        let mut first = desired_status(&cr, &sample_ds(4, 4)).unwrap();
        first.conditions[0].last_transition_time = epoch.clone();
        cr.status = Some(first.clone());

        let second = desired_status(&cr, &sample_ds(4, 4)).unwrap();
        assert_eq!(second.conditions[0].last_transition_time, epoch);
        assert_eq!(cr.status.as_ref(), Some(&second));
    }

    #[test]
    fn flipped_condition_gets_new_transition_time() {
        let mut cr = sample_cr();
        let epoch = Time(Timestamp::UNIX_EPOCH);
        let mut ready = desired_status(&cr, &sample_ds(4, 4)).unwrap();
        ready.conditions[0].last_transition_time = epoch.clone();
        cr.status = Some(ready);

        let progressing = desired_status(&cr, &sample_ds(4, 2)).unwrap();
        assert_eq!(progressing.conditions[0].status, "False");
        assert_ne!(progressing.conditions[0].last_transition_time, epoch);
        assert_ne!(cr.status.as_ref(), Some(&progressing));
    }
}
