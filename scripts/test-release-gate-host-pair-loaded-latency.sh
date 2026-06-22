#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/release-gate-host-pair-loaded-latency.sh"

fail() {
  printf 'release-gate host-pair loaded-latency harness failed: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle'"
}

test_auto_without_target_skips() {
  local out
  out="$(env -i PATH="$PATH" HOME="$HOME" bash "$SCRIPT")"
  assert_contains "$out" "Skipping host-pair loaded latency gate" "auto skip"
}

test_explicit_without_target_fails() {
  local out status
  set +e
  out="$(env -i PATH="$PATH" HOME="$HOME" NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY=1 bash "$SCRIPT" 2>&1)"
  status=$?
  set -e
  [[ "$status" -eq 1 ]] || fail "explicit mode without target should exit 1, got $status"
  assert_contains "$out" "requires an SSH target" "explicit target error"
}

test_explicit_dry_run_sets_loaded_defaults() {
  local out
  out="$(
    env -i PATH="$PATH" HOME="$HOME" \
      NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY=1 \
      NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY_DRY_RUN=1 \
      NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY_SSH=bench-host \
      bash "$SCRIPT"
  )"
  assert_contains "$out" "NVPN_HOST_PAIR_SSH=bench-host" "dry-run ssh"
  assert_contains "$out" "NVPN_RELEASE_GATE_HOST_PAIR_LOADED_DURATION_SECS=180" "dry-run duration"
  assert_contains "$out" "NVPN_RELEASE_GATE_HOST_PAIR_LOADED_IPERF_DURATION_SECS=10" "dry-run iperf duration"
  assert_contains "$out" "NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MIN_INTERVAL_MBPS=10" "dry-run stall threshold"
  assert_contains "$out" "NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_STALL_INTERVALS=1" "dry-run stall budget"
  assert_contains "$out" "NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_LOSS_PERCENT=2" "dry-run loss threshold"
  assert_contains "$out" "NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_GT1000=0" "dry-run high-latency threshold"
  assert_contains "$out" "release-gate-host-pair-loaded-latency.sh" "dry-run command"
}

test_host_pair_env_selects_target() {
  local out
  out="$(
    env -i PATH="$PATH" HOME="$HOME" \
      NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY=auto \
      NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY_DRY_RUN=1 \
      NVPN_HOST_PAIR_SSH=bench-host \
      bash "$SCRIPT"
  )"
  assert_contains "$out" "NVPN_HOST_PAIR_SSH=bench-host" "host-pair target"
}

test_auto_without_target_skips
test_explicit_without_target_fails
test_explicit_dry_run_sets_loaded_defaults
test_host_pair_env_selects_target

printf 'release-gate host-pair loaded-latency harness passed\n'
