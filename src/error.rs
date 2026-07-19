//! Error types shared across the operator.

/// Errors that can occur while reconciling a [`crate::crd::FoldingAtHome`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An error returned by the Kubernetes API server.
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),

    /// A resource was missing a field the operator requires (e.g. `metadata.name`).
    #[error("missing object key: {0}")]
    MissingObjectKey(&'static str),

    /// Serialization/deserialization failure while building or patching a resource.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// A leader-election failure.
    #[error("leader election error: {0}")]
    LeaderElection(String),
}

/// Convenience alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
