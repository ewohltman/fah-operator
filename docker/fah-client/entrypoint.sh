#!/usr/bin/env bash
#
# Render a Folding@Home v8 config.xml from environment variables and launch the
# client. Injected by the operator's DaemonSet; see src/resources.rs for the env.
#
#   USER          donor name (default: Anonymous)
#   TEAM          team number (default: 0)
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

# Escape XML metacharacters in a value.
xml_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g'
}

{
  echo '<config>'
  echo "  <user v=\"$(xml_escape "${FAH_USER}")\"/>"
  echo "  <team v=\"$(xml_escape "${FAH_TEAM}")\"/>"
  echo "  <machine-name v=\"$(xml_escape "${FAH_MACHINE}")\"/>"
  [ -n "${PASSKEY:-}" ]       && echo "  <passkey v=\"$(xml_escape "${PASSKEY}")\"/>"
  [ -n "${CAUSE:-}" ]         && echo "  <cause v=\"$(xml_escape "${CAUSE}")\"/>"
  [ -n "${ACCOUNT_TOKEN:-}" ] && echo "  <account-token v=\"$(xml_escape "${ACCOUNT_TOKEN}")\"/>"
  echo '</config>'
} > "${DATA_DIR}/config.xml"

echo "fah-operator: wrote ${DATA_DIR}/config.xml for machine '${FAH_MACHINE}' (user='${FAH_USER}', team='${FAH_TEAM}', gpu='${ENABLE_GPU:-false}')"

# fah-client reads config.xml from and writes its data/logs to the working dir.
exec fah-client
