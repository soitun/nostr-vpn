#!/usr/bin/env bash
# Full regression lane for adding a phone-class device from native desktop UI:
# shared mobile approval, every available desktop shell, and reciprocal FIPS
# reachability after GUI acceptance.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo test -p nostr-vpn-app-core compact_join_bootstrap_is_added_by_admin_without_request_event
cargo test -p nostr-vpn-app-core mobile_join_request_sends_and_records_over_real_fips_endpoint

case "$(uname -s)" in
  Darwin)
    "$ROOT/scripts/e2e-desktop-roster-join.sh"
    "$ROOT/tools/run-linux" /workspace/nostr-vpn/scripts/e2e-desktop-roster-join.sh
    if ssh -o BatchMode=yes -o ConnectTimeout=5 "${NVPN_WINDOWS_SSH_HOST:-win11-dev}" hostname >/dev/null 2>&1; then
      "$ROOT/scripts/windows-vm-roster-e2e.sh"
    else
      echo "Skipping Windows roster GUI because its test VM is unavailable."
    fi
    ;;
  Linux)
    "$ROOT/tools/run-linux" /workspace/nostr-vpn/scripts/e2e-desktop-roster-join.sh
    ;;
  *)
    echo "Run scripts/e2e-desktop-roster-join.ps1 directly on Windows." >&2
    exit 2
    ;;
esac

if docker info >/dev/null 2>&1; then
  "$ROOT/scripts/e2e-bootstrap-discovery-docker.sh"
else
  echo "Skipping reciprocal tunnel test because Docker is unavailable."
fi

echo "DEVICE_ROSTER_E2E_OK"
