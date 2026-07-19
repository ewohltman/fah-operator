//! The `FoldingAtHome` custom resource definition.
//!
//! A `FoldingAtHome` object describes a fleet of Folding@Home clients that the
//! operator materializes as a single [`DaemonSet`](k8s_openapi::api::apps::v1::DaemonSet)
//! so that one client pod runs on every schedulable node in the cluster.

use k8s_openapi::api::core::v1::{Affinity, ResourceRequirements, SecretKeySelector, Toleration};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Desired state for a fleet of Folding@Home clients.
#[derive(CustomResource, Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "fah.ewohltman.github.io",
    version = "v1alpha1",
    kind = "FoldingAtHome",
    plural = "foldingathomes",
    singular = "foldingathome",
    shortname = "fah",
    namespaced,
    status = "FoldingAtHomeStatus",
    derive = "Default",
    printcolumn = r#"{"name":"DaemonSet","type":"string","jsonPath":".status.daemonSetName"}"#,
    printcolumn = r#"{"name":"Desired","type":"integer","jsonPath":".status.desiredNodes"}"#,
    printcolumn = r#"{"name":"Ready","type":"integer","jsonPath":".status.readyNodes"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct FoldingAtHomeSpec {
    /// Container image for the Folding@Home client. Defaults to the operator's
    /// bundled `fah-client` image when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    /// Folding@Home account/donor name reported with completed work.
    #[serde(default = "default_user")]
    pub user: String,

    /// Folding@Home team number to fold for.
    #[serde(default)]
    pub team: i64,

    /// Optional passkey that bonuses completed work to the configured user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passkey: Option<SecretOrValue>,

    /// Optional Folding@Home v8 account token linking the client to an account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_token: Option<SecretOrValue>,

    /// How aggressively the client should fold.
    #[serde(default)]
    pub power: PowerLevel,

    /// Whether to request GPU resources and enable GPU folding.
    ///
    /// Serialized as `enableGPU` (not the camelCase-default `enableGpu`) so the
    /// acronym reads correctly in manifests. The `enableGpu` alias keeps
    /// pre-rename resources deserializing correctly instead of silently
    /// defaulting GPU folding back off.
    #[serde(default, rename = "enableGPU", alias = "enableGpu")]
    pub enable_gpu: bool,

    /// Optional research cause preference (e.g. `cancer`, `alzheimers`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,

    /// Compute resource requests/limits applied to each client pod.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceRequirements>,

    /// Node-local host path used to persist each client's data directory
    /// (`/fah`), so its Folding@Home identity (RSA key / F@H ID) survives pod
    /// restarts instead of regenerating and registering a new machine every
    /// time. When omitted, the data directory is ephemeral.
    ///
    /// A `hostPath` (not a PVC) is used deliberately: this workload is a
    /// DaemonSet with one pod per node, so node-local storage is the correct
    /// fit — a DaemonSet has no `volumeClaimTemplates`, and a ReadWriteOnce
    /// volume cannot follow a pod across nodes. An init container fixes up
    /// ownership so the non-root client can write to the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_host_path: Option<String>,

    /// Node selector constraining which nodes run a client pod.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_selector: BTreeMap<String, String>,

    /// Tolerations added to the client pod so it can schedule onto tainted nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerations: Vec<Toleration>,

    /// Affinity rules applied to the client pod.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affinity: Option<Affinity>,
}

/// A value provided either inline or sourced from a Secret.
///
/// Prefer [`secret_key_ref`](Self::secret_key_ref) for sensitive values such as
/// passkeys and account tokens so they are not stored in plaintext on the
/// generated DaemonSet.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretOrValue {
    /// A literal, inline value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// A reference to a key within a Secret in the same namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key_ref: Option<SecretKeySelector>,
}

/// How aggressively the Folding@Home client should consume resources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PowerLevel {
    /// Minimal resource usage.
    Light,
    /// Balanced resource usage.
    Medium,
    /// Maximum resource usage.
    #[default]
    Full,
}

impl PowerLevel {
    /// The value the client expects for its `power`/`POWER` setting.
    pub fn as_str(&self) -> &'static str {
        match self {
            PowerLevel::Light => "light",
            PowerLevel::Medium => "medium",
            PowerLevel::Full => "full",
        }
    }
}

/// Observed state of a [`FoldingAtHome`].
///
/// Derives `PartialEq` so the controller can compare the computed status with
/// the observed one and skip the write entirely when nothing changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FoldingAtHomeStatus {
    /// The `.metadata.generation` the operator last reconciled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,

    /// Name of the DaemonSet managed by this resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_set_name: Option<String>,

    /// Number of nodes that should be running a client pod.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_nodes: Option<i32>,

    /// Number of nodes with a ready client pod.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_nodes: Option<i32>,

    /// Standard Kubernetes conditions (notably `Ready`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
}

/// Default donor name when none is provided.
fn default_user() -> String {
    "Anonymous".to_string()
}
