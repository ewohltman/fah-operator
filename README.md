# fah-operator

[![CI](https://github.com/ewohltman/fah-operator/actions/workflows/ci.yml/badge.svg)](https://github.com/ewohltman/fah-operator/actions/workflows/ci.yml)
[![Publish images](https://github.com/ewohltman/fah-operator/actions/workflows/images.yml/badge.svg)](https://github.com/ewohltman/fah-operator/actions/workflows/images.yml)

A Kubernetes operator, written in Rust, that runs [Folding@Home](https://foldingathome.org/)
across your cluster. Create a single `FoldingAtHome` custom resource and the
operator deploys a Folding@Home client to **every node** as a DaemonSet.

## What is Folding@Home?

[Folding@Home](https://foldingathome.org/) is a distributed-computing project for
disease research. Volunteers donate spare CPU/GPU cycles to simulate protein
folding, contributing to research into diseases such as cancer, Alzheimer's, and
infectious diseases. This operator lets you donate the idle capacity of a
Kubernetes cluster with a single manifest.

## How it works

```mermaid
flowchart TD
    admin(["kubectl apply<br/>FoldingAtHome CR"])

    subgraph cluster["Kubernetes cluster"]
        cr["FoldingAtHome (CR)"]
        operator["fah-operator<br/>Deployment · 3 replicas<br/>leader-elected"]
        ds["DaemonSet"]
        pod1["FAH client<br/>node 1"]
        pod2["FAH client<br/>node 2"]
        pod3["FAH client<br/>node N"]

        cr -. watched by .-> operator
        operator -- creates / owns --> ds
        ds --> pod1
        ds --> pod2
        ds --> pod3
    end

    admin -- creates --> cr
```

- The operator runs as a **Deployment with multiple replicas**. They coordinate
  through a Kubernetes **Lease** (leader election); exactly one replica reconciles
  at a time, and if it dies another takes over automatically.
- Reconciling a `FoldingAtHome` produces an owner-referenced **DaemonSet** (plus a
  ServiceAccount), so a client pod runs on every schedulable node. Deleting the CR
  garbage-collects everything it created.

## Quick start

1. **Install the CRD, RBAC, and operator:**

   ```bash
   kubectl apply -f deploy/crd.yaml
   kubectl apply -f deploy/rbac.yaml
   kubectl apply -f deploy/operator.yaml
   ```

2. **Create a `FoldingAtHome` resource.** The bundled example creates a `fah`
   namespace and a `FoldingAtHome` in it:

   ```bash
   kubectl apply -f example/example-foldingathome.yaml
   ```

   Or inline (set `user`/`team` to your own donor name and team number):

   ```yaml
   apiVersion: fah.ewohltman.github.io/v1alpha1
   kind: FoldingAtHome
   metadata:
     name: cluster-fold
     namespace: default
   spec:
     user: my-donor-name
     team: 0
     power: full
   ```

3. **Check status** (in the CR's namespace — `fah` for the bundled example):

   ```bash
   kubectl get foldingathome -n fah   # DESIRED / READY node counts
   kubectl get daemonset -n fah       # the managed client DaemonSet
   kubectl get pods -n fah -l app.kubernetes.io/name=folding-at-home
   ```

## `FoldingAtHome` spec reference

| Field          | Type                      | Default                    | Description                                         |
|----------------|---------------------------|----------------------------|-----------------------------------------------------|
| `image`        | string                    | bundled `fah-client` image | Folding@Home client image.                          |
| `user`         | string                    | `Anonymous`                | Donor name credited with completed work.            |
| `team`         | integer                   | `0`                        | Team number to fold for.                            |
| `power`        | `light`\|`medium`\|`full` | `full`                     | How hard the client folds.                          |
| `enableGPU`    | bool                      | `false`                    | Request an `nvidia.com/gpu` and enable GPU folding. |
| `cause`        | string                    | —                          | Research cause preference (e.g. `cancer`).          |
| `passkey`      | value or secretKeyRef     | —                          | Passkey that bonuses work to your account.          |
| `accountToken` | value or secretKeyRef     | —                          | Folding@Home v8 account token.                      |
| `resources`    | ResourceRequirements      | —                          | CPU/memory requests & limits per pod.               |
| `nodeSelector` | map                       | —                          | Restrict which nodes run a client pod.              |
| `tolerations`  | []Toleration              | —                          | Allow scheduling onto tainted nodes.                |
| `affinity`     | Affinity                  | —                          | Pod affinity/anti-affinity rules.                   |

### Sensitive values

Prefer `secretKeyRef` for `passkey` and `accountToken` so they are not stored in
plaintext on the DaemonSet:

```yaml
spec:
  passkey:
    secretKeyRef:
      name: fah-secrets
      key: passkey
  accountToken:
    secretKeyRef:
      name: fah-secrets
      key: account-token
```

## High availability

`deploy/operator.yaml` runs **3 replicas** with pod anti-affinity to spread them
across nodes. They elect a leader via the `fah-operator-leader` Lease in the
operator's namespace; standby replicas take over within the lease TTL (~15s) if the
leader fails. On graceful shutdown (SIGTERM/SIGINT — e.g. a rolling update) the
leader releases the Lease before exiting, so a standby takes over within one
renew interval (~5s) instead of waiting out the TTL. You can inspect the current
leader with:

```bash
kubectl get lease fah-operator-leader -o yaml
```

## Building

```bash
cargo build --release                       # build binaries
cargo test                                  # unit tests
cargo run --bin crdgen > deploy/crd.yaml    # regenerate the CRD

# Container images
docker build -t ghcr.io/ewohltman/fah-operator:latest -f docker/operator/Dockerfile .
docker build -t ghcr.io/ewohltman/fah-client:latest   docker/fah-client
```

## Continuous integration

GitHub Actions workflows live in [`.github/workflows/`](./.github/workflows):

- **CI** (`ci.yml`) — on every push/PR to `main` and `develop`: `cargo fmt`
  check, Clippy (warnings as errors), tests, release build, a check that
  `deploy/crd.yaml` is in sync with `src/crd.rs`, and a no-push Docker build of
  both images.
- **Publish images** (`images.yml`) — on push to `main` / `v*` tags: builds and
  pushes the operator and client images to GHCR.
- **Claude Code Review** (`claude-code-review.yml`) — automatically reviews every
  pull request.
- **Claude Code** (`claude.yml`) — responds to `@claude` mentions in issues and
  pull requests.

The two Claude workflows authenticate through the Claude GitHub App
(`CLAUDE_CODE_OAUTH_TOKEN` secret), installed with `/install-github-app` in
Claude Code.

## Development

See [CLAUDE.md](./CLAUDE.md) for repository layout and design details.

## License

MIT
