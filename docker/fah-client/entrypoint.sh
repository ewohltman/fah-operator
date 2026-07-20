#!/usr/bin/env bash
#
# Render a Folding@Home v8 config.xml from environment variables and launch the
# client. Injected by the operator's DaemonSet; see src/resources.rs for the env.
#
#   USER          donor name (default: Anonymous)
#   TEAM          team number (default: 0)
#   POWER         light|medium|full (default: full) -> mapped to the v8 `cpus`
#                 count; v8 has no `power` setting (that was v7)
#   PASSKEY       optional passkey
#   ACCOUNT_TOKEN optional v8 account token linking this machine to an account
#   CAUSE         optional research cause preference
#   MACHINE_NAME  name used to identify this machine in the account (default: hostname)
#   ENABLE_GPU    "true" to allow GPU folding (informational; GPU visibility is
#                 controlled by the pod's resource requests)
#   DATA_DIR      where config/logs/data live (default: /fah)
set -euo pipefail

DATA_DIR="${DATA_DIR:-/fah}"
mkdir -p "${DATA_DIR}"
cd "${DATA_DIR}"

FAH_USER="${USER:-Anonymous}"
FAH_TEAM="${TEAM:-0}"
FAH_MACHINE="${MACHINE_NAME:-$(hostname)}"

# v8 has no `power` knob; folding intensity is the CPU count. `nproc` is
# cgroup-aware, so this honors the pod's CPU limit instead of grabbing every
# core on the node. Map the operator's power level onto that count.
CORES="$(nproc 2>/dev/null || true)"
# Guard against empty or non-numeric output so the arithmetic below can't crash
# the entrypoint under `set -e`.
case "${CORES}" in
  '' | *[!0-9]*) CORES=1 ;;
esac
[ "${CORES}" -lt 1 ] && CORES=1
case "${POWER:-full}" in
  light)  FAH_CPUS=1 ;;
  medium) FAH_CPUS=$(( CORES / 2 )); [ "${FAH_CPUS}" -lt 1 ] && FAH_CPUS=1 ;;
  *)      FAH_CPUS="${CORES}" ;;   # full, or any unrecognized value: all cores
esac

# Escape XML metacharacters in a value.
xml_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g'
}

{
  echo '<config>'
  echo "  <user v=\"$(xml_escape "${FAH_USER}")\"/>"
  echo "  <team v=\"$(xml_escape "${FAH_TEAM}")\"/>"
  echo "  <machine-name v=\"$(xml_escape "${FAH_MACHINE}")\"/>"
  # Fold continuously on a dedicated node, not only when it is idle, using the
  # power-derived core count.
  echo "  <on-idle v=\"false\"/>"
  echo "  <cpus v=\"$(xml_escape "${FAH_CPUS}")\"/>"
  [ -n "${PASSKEY:-}" ]       && echo "  <passkey v=\"$(xml_escape "${PASSKEY}")\"/>"
  [ -n "${CAUSE:-}" ]         && echo "  <cause v=\"$(xml_escape "${CAUSE}")\"/>"
  [ -n "${ACCOUNT_TOKEN:-}" ] && echo "  <account-token v=\"$(xml_escape "${ACCOUNT_TOKEN}")\"/>"
  echo '</config>'
} > "${DATA_DIR}/config.xml"

echo "fah-operator: wrote ${DATA_DIR}/config.xml for machine '${FAH_MACHINE}' (user='${FAH_USER}', team='${FAH_TEAM}', cpus='${FAH_CPUS}', gpu='${ENABLE_GPU:-false}')"

# A v8 client linked to an account comes up *paused* and stays paused until it is
# told to fold. No config.xml knob changes this (v8 dropped the v7 `<paused>`
# setting); the only control surface is the client's local WebSocket API. Once it
# is reachable, send a single idempotent "state:fold" command (fold.pl). Runs in
# the background so fah-client below stays PID 1 and handles signals itself.
(
  for _ in $(seq 1 60); do
    if out="$(perl /usr/local/bin/fold.pl 2>&1)"; then
      echo "fah-operator: ${out}"
      exit 0
    fi
    sleep 2
  done
  echo "fah-operator: WARNING: could not start folding via local API; client may stay paused" >&2
) &

# fah-client reads config.xml from and writes its data/logs to the working dir.
exec fah-client
