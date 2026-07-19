//! `fah-operator` — a Kubernetes operator that runs a Folding@Home client as a
//! DaemonSet on every node in the cluster.
//!
//! The crate is split into small modules:
//!
//! - [`crd`] — the `FoldingAtHome` custom resource definition.
//! - [`resources`] — builders that turn a `FoldingAtHome` into Kubernetes objects.
//! - [`controller`] — the reconcile loop and error policy.
//! - [`leader`] — Lease-based leader election for high availability.
//! - [`error`] — shared error types.

pub mod controller;
pub mod crd;
pub mod error;
pub mod leader;
pub mod resources;

/// Field manager name used for server-side apply patches.
pub const MANAGER: &str = "fah-operator";
