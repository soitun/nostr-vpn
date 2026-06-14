#!/usr/bin/env bash
# Optional release-gate latency check for an already configured FIPS host pair.
#
# The wrapper intentionally takes the peer target from environment variables so
# operator-local hostnames, users, and IPs stay out of committed scripts.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

MODE="${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY:-auto}"
TARGET="${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_SSH:-${NVPN_HOST_PAIR_SSH:-}}"
DRY_RUN="${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_DRY_RUN:-0}"
CONNECT_TIMEOUT="${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_CONNECT_TIMEOUT:-5}"

is_disabled() {
  case "$1" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off) return 0 ;;
    *) return 1 ;;
  esac
}

is_enabled() {
  case "$1" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) return 0 ;;
    *) return 1 ;;
  esac
}

is_auto() {
  case "$1" in
    auto|AUTO|Auto|"") return 0 ;;
    *) return 1 ;;
  esac
}

skip() {
  printf 'Skipping host-pair latency gate: %s\n' "$*"
}

require_target() {
  if [[ -z "$TARGET" ]]; then
    printf 'NVPN_RELEASE_GATE_HOST_PAIR_LATENCY=1 requires NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_SSH or NVPN_HOST_PAIR_SSH\n' >&2
    exit 2
  fi
}

ssh_reachable() {
  ssh -o BatchMode=yes -o ConnectTimeout="$CONNECT_TIMEOUT" "$TARGET" true >/dev/null 2>&1
}

print_dry_run_env() {
  printf 'NVPN_HOST_PAIR_SSH=%q\n' "$NVPN_HOST_PAIR_SSH"
  printf 'NVPN_HOST_PAIR_DURATION_SECS=%q\n' "$NVPN_HOST_PAIR_DURATION_SECS"
  printf 'NVPN_HOST_PAIR_INTERVAL_SECS=%q\n' "$NVPN_HOST_PAIR_INTERVAL_SECS"
  printf 'NVPN_HOST_PAIR_PING_COUNT=%q\n' "$NVPN_HOST_PAIR_PING_COUNT"
  printf 'NVPN_HOST_PAIR_PING_INTERVAL=%q\n' "$NVPN_HOST_PAIR_PING_INTERVAL"
  printf 'NVPN_HOST_PAIR_REQUIRE_IPERF=%q\n' "$NVPN_HOST_PAIR_REQUIRE_IPERF"
  printf 'NVPN_HOST_PAIR_MAX_PING_LOSS_PERCENT=%q\n' "$NVPN_HOST_PAIR_MAX_PING_LOSS_PERCENT"
  printf 'NVPN_HOST_PAIR_MAX_PING_AVG_MS=%q\n' "$NVPN_HOST_PAIR_MAX_PING_AVG_MS"
  printf 'NVPN_HOST_PAIR_MAX_PING_P95_MS=%q\n' "$NVPN_HOST_PAIR_MAX_PING_P95_MS"
  printf 'NVPN_HOST_PAIR_MAX_PING_P99_MS=%q\n' "$NVPN_HOST_PAIR_MAX_PING_P99_MS"
  printf 'NVPN_HOST_PAIR_MAX_PING_MAX_MS=%q\n' "$NVPN_HOST_PAIR_MAX_PING_MAX_MS"
}

configure_host_pair_defaults() {
  export NVPN_HOST_PAIR_SSH="$TARGET"
  export NVPN_HOST_PAIR_DURATION_SECS="${NVPN_HOST_PAIR_DURATION_SECS:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_DURATION_SECS:-30}}"
  export NVPN_HOST_PAIR_INTERVAL_SECS="${NVPN_HOST_PAIR_INTERVAL_SECS:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_INTERVAL_SECS:-30}}"
  export NVPN_HOST_PAIR_PING_COUNT="${NVPN_HOST_PAIR_PING_COUNT:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_PING_COUNT:-300}}"
  export NVPN_HOST_PAIR_PING_INTERVAL="${NVPN_HOST_PAIR_PING_INTERVAL:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_PING_INTERVAL:-0.1}}"
  export NVPN_HOST_PAIR_REQUIRE_IPERF="${NVPN_HOST_PAIR_REQUIRE_IPERF:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_REQUIRE_IPERF:-0}}"

  export NVPN_HOST_PAIR_MAX_PING_LOSS_PERCENT="${NVPN_HOST_PAIR_MAX_PING_LOSS_PERCENT:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_MAX_LOSS_PERCENT:-2}}"
  export NVPN_HOST_PAIR_MAX_PING_AVG_MS="${NVPN_HOST_PAIR_MAX_PING_AVG_MS:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_MAX_AVG_MS:-150}}"
  export NVPN_HOST_PAIR_MAX_PING_P95_MS="${NVPN_HOST_PAIR_MAX_PING_P95_MS:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_MAX_P95_MS:-350}}"
  export NVPN_HOST_PAIR_MAX_PING_P99_MS="${NVPN_HOST_PAIR_MAX_PING_P99_MS:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_MAX_P99_MS:-750}}"
  export NVPN_HOST_PAIR_MAX_PING_MAX_MS="${NVPN_HOST_PAIR_MAX_PING_MAX_MS:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_MAX_MAX_MS:-1000}}"
}

run_preflight() {
  local dir log_path
  dir="$(mktemp -d)"
  log_path="$dir/preflight.log"
  if NVPN_HOST_PAIR_OUTPUT_DIR="$dir" \
    NVPN_HOST_PAIR_PREFLIGHT=1 \
    "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" >"$log_path" 2>&1; then
    rm -rf "$dir"
    return 0
  fi
  printf 'Host-pair latency preflight did not pass; details:\n' >&2
  sed -n '1,160p' "$log_path" >&2 || true
  rm -rf "$dir"
  return 1
}

run_latency_gate() {
  configure_host_pair_defaults
  if [[ "$DRY_RUN" == "1" ]]; then
    print_dry_run_env
    printf '%q\n' "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
    return 0
  fi

  printf 'Running host-pair latency gate against configured SSH target.\n'
  "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
}

main() {
  if is_disabled "$MODE"; then
    skip "NVPN_RELEASE_GATE_HOST_PAIR_LATENCY=$MODE"
    return 0
  fi

  if is_enabled "$MODE"; then
    require_target
    run_latency_gate
    return 0
  fi

  if ! is_auto "$MODE"; then
    printf 'Unsupported NVPN_RELEASE_GATE_HOST_PAIR_LATENCY=%s\n' "$MODE" >&2
    exit 2
  fi

  if [[ -z "$TARGET" ]]; then
    skip "set NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_SSH or NVPN_HOST_PAIR_SSH to enable it"
    return 0
  fi
  if [[ "$DRY_RUN" != "1" ]] && ! ssh_reachable; then
    skip "ssh target is unreachable"
    return 0
  fi

  configure_host_pair_defaults
  if [[ "$DRY_RUN" == "1" ]]; then
    print_dry_run_env
    printf '%q\n' "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
    return 0
  fi
  if ! run_preflight; then
    skip "preflight failed in auto mode; set NVPN_RELEASE_GATE_HOST_PAIR_LATENCY=1 to make this fatal"
    return 0
  fi
  "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
}

main "$@"
