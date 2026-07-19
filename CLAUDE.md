# CLAUDE.md

Guidance for Claude (and other AI assistants) working in this repository.

## What this is

`fah-operator` is a Kubernetes operator written in Rust with
[kube-rs](https://github.com/kube-rs/kube). When a `FoldingAtHome` custom
resource is created, the operator reconciles it into a **DaemonSet** that runs a
[Folding@Home](https://foldingathome.org/) client pod on every node in the
cluster. The operator is designed to run as **multiple replicas** using
**Lease-based leader election**, so it stays available if a replica dies.

## Common commands

```bash
cargo build                 # build the operator + crdgen binaries
cargo test                  # run unit tests (resource builders)
cargo clippy --all-targets  # lint
cargo run --bin fah-operator            # run against the current kubeconfig context
cargo run --bin crdgen > deploy/crd.yaml # regenerate the CRD manifest
```

Regenerate `deploy/crd.yaml` with `crdgen` whenever `src/crd.rs` changes — it is
a generated file, do not hand-edit it.

## Layout

| Path | Purpose |
|------|---------|
| `src/crd.rs` | `FoldingAtHome` custom resource (spec + status). |
| `src/resources.rs` | Builders: `FoldingAtHome` → `DaemonSet` + `ServiceAccount`. |
| `src/controller.rs` | Reconcile loop, status updates, error policy. |
| `src/leader.rs` | Lease-based leader election. |
| `src/error.rs` | Shared `Error`/`Result` types. |
| `src/main.rs` | Binary entrypoint: client, logging, leader election. |
| `src/bin/crdgen.rs` | Prints the CRD YAML. |
| `deploy/` | Raw manifests: CRD, RBAC, operator Deployment, example CR. |
| `docker/operator/` | Multi-stage Dockerfile for the operator image. |
| `docker/fah-client/` | Dockerfile + entrypoint for the bundled FAH client image. |

## Key design points

- **Ownership, not finalizers.** Managed objects carry an `ownerReference` back
  to the `FoldingAtHome`, so Kubernetes garbage-collects them on delete. There is
  no finalizer to manage.
- **Server-side apply.** Reconcile applies the `ServiceAccount` and `DaemonSet`
  with `PatchParams::apply(MANAGER).force()` (field manager `fah-operator`), which
  is idempotent create-or-update. `MANAGER` is defined in `src/lib.rs`.
- **Status** is written to the `.status` subresource via a merge patch and mirrors
  the DaemonSet's `desiredNumberScheduled` / `numberReady` plus a `Ready` condition.
- **Leader election** uses `kube-leader-election` (a `Lease` in `coordination.k8s.io`
  named `fah-operator-leader`). Only the lease holder runs the controller; on lease
  loss the controller shuts down gracefully and the replica re-contends. Holder id
  and namespace come from the `POD_NAME` / `POD_NAMESPACE` downward-API env vars.
- **GPU** is opt-in (`spec.enableGPU`): it adds an `nvidia.com/gpu` limit and sets
  `ENABLE_GPU=true`. CPU folding is the default.
- **Secrets.** `passkey` and `accountToken` accept either an inline `value` or a
  `secretKeyRef`; the latter becomes an env `valueFrom` so secrets are not stored
  in the DaemonSet spec.

## Version notes

- `kube` 4.x, `k8s-openapi` 0.28 with the `latest` + `schemars` features. Note that
  k8s-openapi 0.28 uses **jiff** (not chrono) for `Time`, and its `schemars` feature
  enables schemars **v1** (matching kube-derive).
- Rust edition 2024.

## Conventions

- Keep `deploy/crd.yaml` in sync with `src/crd.rs` via `crdgen`.
- Add unit tests for new builder logic in `src/resources.rs`.
- Run `cargo build`, `cargo test`, and `cargo clippy --all-targets` before committing.
