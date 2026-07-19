//! Builders that turn a [`FoldingAtHome`] into the Kubernetes objects the
//! operator manages: a [`ServiceAccount`] and a [`DaemonSet`].

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{DaemonSet, DaemonSetSpec};
use k8s_openapi::api::core::v1::{
    Container, ContainerPort, EnvVar, EnvVarSource, ObjectFieldSelector, PodSpec, PodTemplateSpec,
    ResourceRequirements, SecurityContext, ServiceAccount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, OwnerReference};
use kube::Resource;
use kube::api::ObjectMeta;

use crate::crd::{FoldingAtHome, SecretOrValue};
use crate::error::{Error, Result};

/// Default Folding@Home client image used when the spec omits one.
pub const DEFAULT_IMAGE: &str = "ghcr.io/ewohltman/fah-client:latest";

/// Container port the Folding@Home web control listens on.
const WEB_PORT: i32 = 7396;

/// Resource key GPU folding requests.
const GPU_RESOURCE: &str = "nvidia.com/gpu";

/// Standard labels applied to every managed object.
///
/// `app.kubernetes.io/instance` ties children to a specific `FoldingAtHome`.
pub fn labels(instance: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "app.kubernetes.io/name".to_string(),
            "folding-at-home".to_string(),
        ),
        (
            "app.kubernetes.io/instance".to_string(),
            instance.to_string(),
        ),
        (
            "app.kubernetes.io/component".to_string(),
            "client".to_string(),
        ),
        (
            "app.kubernetes.io/managed-by".to_string(),
            crate::MANAGER.to_string(),
        ),
    ])
}

/// Name shared by the managed DaemonSet and ServiceAccount for `cr`.
pub fn child_name(cr: &FoldingAtHome) -> Result<String> {
    cr.meta()
        .name
        .clone()
        .ok_or(Error::MissingObjectKey("metadata.name"))
}

/// An [`OwnerReference`] pointing back at `cr` so children are garbage-collected
/// when the `FoldingAtHome` is deleted.
fn owner_reference(cr: &FoldingAtHome) -> Result<OwnerReference> {
    let name = child_name(cr)?;
    let uid = cr
        .meta()
        .uid
        .clone()
        .ok_or(Error::MissingObjectKey("metadata.uid"))?;
    Ok(OwnerReference {
        api_version: FoldingAtHome::api_version(&()).into_owned(),
        kind: FoldingAtHome::kind(&()).into_owned(),
        name,
        uid,
        controller: Some(true),
        block_owner_deletion: Some(true),
    })
}

/// Build the ServiceAccount the client pods run as.
pub fn service_account(cr: &FoldingAtHome) -> Result<ServiceAccount> {
    let name = child_name(cr)?;
    Ok(ServiceAccount {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: cr.meta().namespace.clone(),
            labels: Some(labels(&name)),
            owner_references: Some(vec![owner_reference(cr)?]),
            ..Default::default()
        },
        ..Default::default()
    })
}

/// Turn a [`SecretOrValue`] into an [`EnvVar`] named `name`, if present.
fn secret_env(name: &str, source: Option<&SecretOrValue>) -> Option<EnvVar> {
    let source = source?;
    if let Some(secret_key_ref) = &source.secret_key_ref {
        Some(EnvVar {
            name: name.to_string(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(secret_key_ref.clone()),
                ..Default::default()
            }),
            ..Default::default()
        })
    } else {
        source.value.as_ref().map(|value| EnvVar {
            name: name.to_string(),
            value: Some(value.clone()),
            ..Default::default()
        })
    }
}

/// Environment variables consumed by the client image's entrypoint.
fn env_vars(cr: &FoldingAtHome) -> Vec<EnvVar> {
    let spec = &cr.spec;
    let mut env = vec![
        EnvVar {
            name: "USER".to_string(),
            value: Some(spec.user.clone()),
            ..Default::default()
        },
        EnvVar {
            name: "TEAM".to_string(),
            value: Some(spec.team.to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "POWER".to_string(),
            value: Some(spec.power.as_str().to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "ENABLE_GPU".to_string(),
            value: Some(spec.enable_gpu.to_string()),
            ..Default::default()
        },
        // Name each node's client after the node it runs on.
        EnvVar {
            name: "MACHINE_NAME".to_string(),
            value_from: Some(EnvVarSource {
                field_ref: Some(ObjectFieldSelector {
                    field_path: "spec.nodeName".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
    ];

    if let Some(cause) = &spec.cause {
        env.push(EnvVar {
            name: "CAUSE".to_string(),
            value: Some(cause.clone()),
            ..Default::default()
        });
    }
    if let Some(passkey) = secret_env("PASSKEY", spec.passkey.as_ref()) {
        env.push(passkey);
    }
    if let Some(token) = secret_env("ACCOUNT_TOKEN", spec.account_token.as_ref()) {
        env.push(token);
    }
    env
}

/// Compute resource requirements, adding a GPU request when GPU folding is enabled.
fn resource_requirements(cr: &FoldingAtHome) -> Option<ResourceRequirements> {
    let mut resources = cr.spec.resources.clone();
    if cr.spec.enable_gpu {
        let resources = resources.get_or_insert_with(Default::default);
        resources
            .limits
            .get_or_insert_with(Default::default)
            .insert(GPU_RESOURCE.to_string(), Quantity("1".to_string()));
    }
    resources
}

/// Build the DaemonSet that runs one client pod per node.
pub fn daemon_set(cr: &FoldingAtHome) -> Result<DaemonSet> {
    let name = child_name(cr)?;
    let labels = labels(&name);
    let spec = &cr.spec;

    let container = Container {
        name: "fah-client".to_string(),
        image: Some(
            spec.image
                .clone()
                .unwrap_or_else(|| DEFAULT_IMAGE.to_string()),
        ),
        env: Some(env_vars(cr)),
        ports: Some(vec![ContainerPort {
            name: Some("web".to_string()),
            container_port: WEB_PORT,
            ..Default::default()
        }]),
        resources: resource_requirements(cr),
        security_context: Some(SecurityContext {
            allow_privilege_escalation: Some(false),
            run_as_non_root: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    let pod_spec = PodSpec {
        service_account_name: Some(name.clone()),
        containers: vec![container],
        node_selector: (!spec.node_selector.is_empty()).then(|| spec.node_selector.clone()),
        tolerations: (!spec.tolerations.is_empty()).then(|| spec.tolerations.clone()),
        affinity: spec.affinity.clone(),
        ..Default::default()
    };

    Ok(DaemonSet {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: cr.meta().namespace.clone(),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(cr)?]),
            ..Default::default()
        },
        spec: Some(DaemonSetSpec {
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(pod_spec),
            },
            ..Default::default()
        }),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{FoldingAtHomeSpec, PowerLevel};

    fn sample(spec: FoldingAtHomeSpec) -> FoldingAtHome {
        let mut cr = FoldingAtHome::new("sample", spec);
        cr.metadata.namespace = Some("fah".to_string());
        cr.metadata.uid = Some("uid-123".to_string());
        cr
    }

    #[test]
    fn daemon_set_has_owner_reference_and_labels() {
        let ds = daemon_set(&sample(FoldingAtHomeSpec::default())).unwrap();
        let owners = ds.metadata.owner_references.unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].name, "sample");
        assert_eq!(owners[0].uid, "uid-123");
        assert_eq!(owners[0].controller, Some(true));

        let labels = ds.metadata.labels.unwrap();
        assert_eq!(
            labels.get("app.kubernetes.io/instance"),
            Some(&"sample".to_string())
        );
        assert_eq!(
            labels.get("app.kubernetes.io/managed-by"),
            Some(&crate::MANAGER.to_string())
        );
    }

    #[test]
    fn env_reflects_spec() {
        let spec = FoldingAtHomeSpec {
            user: "folder".to_string(),
            team: 42,
            power: PowerLevel::Light,
            ..Default::default()
        };
        let ds = daemon_set(&sample(spec)).unwrap();
        let env = ds.spec.unwrap().template.spec.unwrap().containers[0]
            .env
            .clone()
            .unwrap();
        let get = |k: &str| {
            env.iter()
                .find(|e| e.name == k)
                .and_then(|e| e.value.clone())
        };
        assert_eq!(get("USER"), Some("folder".to_string()));
        assert_eq!(get("TEAM"), Some("42".to_string()));
        assert_eq!(get("POWER"), Some("light".to_string()));
        assert_eq!(get("ENABLE_GPU"), Some("false".to_string()));
    }

    #[test]
    fn gpu_disabled_has_no_gpu_request() {
        let ds = daemon_set(&sample(FoldingAtHomeSpec::default())).unwrap();
        let resources = ds.spec.unwrap().template.spec.unwrap().containers[0]
            .resources
            .clone();
        assert!(resources.is_none());
    }

    #[test]
    fn gpu_enabled_requests_gpu_limit() {
        let spec = FoldingAtHomeSpec {
            enable_gpu: true,
            ..Default::default()
        };
        let ds = daemon_set(&sample(spec)).unwrap();
        let limits = ds.spec.unwrap().template.spec.unwrap().containers[0]
            .resources
            .clone()
            .unwrap()
            .limits
            .unwrap();
        assert_eq!(limits.get(GPU_RESOURCE), Some(&Quantity("1".to_string())));

        let env = daemon_set(&sample(FoldingAtHomeSpec {
            enable_gpu: true,
            ..Default::default()
        }))
        .unwrap();
        let env = env.spec.unwrap().template.spec.unwrap().containers[0]
            .env
            .clone()
            .unwrap();
        assert_eq!(
            env.iter().find(|e| e.name == "ENABLE_GPU").unwrap().value,
            Some("true".to_string())
        );
    }

    #[test]
    fn secret_ref_becomes_env_value_from() {
        use k8s_openapi::api::core::v1::SecretKeySelector;
        let spec = FoldingAtHomeSpec {
            passkey: Some(SecretOrValue {
                secret_key_ref: Some(SecretKeySelector {
                    name: "fah-secrets".to_string(),
                    key: "passkey".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ds = daemon_set(&sample(spec)).unwrap();
        let env = ds.spec.unwrap().template.spec.unwrap().containers[0]
            .env
            .clone()
            .unwrap();
        let passkey = env.iter().find(|e| e.name == "PASSKEY").unwrap();
        assert!(passkey.value.is_none());
        let key_ref = passkey
            .value_from
            .as_ref()
            .unwrap()
            .secret_key_ref
            .as_ref()
            .unwrap();
        assert_eq!(key_ref.name, "fah-secrets");
        assert_eq!(key_ref.key, "passkey");
    }
}
