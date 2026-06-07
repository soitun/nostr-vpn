#!/usr/bin/env bash
# Native host WG-exit dataplane smoke for macOS/Linux-style hosts.
#
# This complements the Linux Docker e2e. It uses `nvpn wg-upstream-test
# --self-test` so no external VPN account is needed: nvpn starts an in-process
# WireGuard responder, creates a real host tun/utun, installs a scoped route to
# the responder's tunnel IP, pings through the WG tunnel, then removes the route.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="${NVPN_WG_EXIT_HOST_PROFILE:-debug}"
BUILD_ARGS=(--locked -p nvpn --bin nvpn)
OUTPUT_ARGS=(nvpn)
case "$PROFILE" in
  release)
    BUILD_ARGS+=(--release)
    OUTPUT_ARGS+=(--release)
    ;;
  debug)
    ;;
  *)
    echo "Unsupported NVPN_WG_EXIT_HOST_PROFILE=$PROFILE; expected debug or release" >&2
    exit 2
    ;;
esac

cargo build "${BUILD_ARGS[@]}"
NVPN_BIN="$(./scripts/build-output-path --raw "${OUTPUT_ARGS[@]}")"
if [[ ! -x "$NVPN_BIN" ]]; then
  echo "Built nvpn binary not found at $NVPN_BIN" >&2
  exit 1
fi

TEST_ARGS=(
  wg-upstream-test
  --self-test
  --timeout-secs "${NVPN_WG_EXIT_HOST_TIMEOUT_SECS:-15}"
  --scoped-host "${NVPN_WG_EXIT_HOST_SCOPED_HOST:-10.99.99.1}"
  --ping-count "${NVPN_WG_EXIT_HOST_PING_COUNT:-3}"
)

case "$(uname -s)" in
  Darwin|Linux)
    if [[ "$(id -u)" == "0" ]]; then
      exec "$NVPN_BIN" "${TEST_ARGS[@]}"
    fi
    if sudo -n true 2>/dev/null; then
      exec sudo -n "$NVPN_BIN" "${TEST_ARGS[@]}"
    fi
    if [[ "${NVPN_WG_EXIT_HOST_INTERACTIVE_SUDO:-0}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]; then
      exec sudo "$NVPN_BIN" "${TEST_ARGS[@]}"
    fi
    echo "WireGuard exit host e2e needs passwordless sudo for tun/route changes." >&2
    echo "Set NVPN_WG_EXIT_HOST_INTERACTIVE_SUDO=1 for an interactive local run." >&2
    exit 1
    ;;
  *)
    echo "WireGuard exit host e2e supports Darwin/Linux here; use scripts/windows-vm-wireguard-exit-e2e.sh for Windows." >&2
    exit 2
    ;;
esac
