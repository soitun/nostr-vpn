#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/release-gate-host-pair-latency.sh"

fail() {
  printf 'release-gate host-pair latency harness failed: %s\n' "$*" >&2
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
  assert_contains "$out" "Skipping host-pair latency gate" "auto skip"
}

test_explicit_without_target_fails() {
  local out status
  set +e
  out="$(env -i PATH="$PATH" HOME="$HOME" NVPN_RELEASE_GATE_HOST_PAIR_LATENCY=1 bash "$SCRIPT" 2>&1)"
  status=$?
  set -e
  [[ "$status" -eq 2 ]] || fail "explicit mode without target should exit 2, got $status"
  assert_contains "$out" "requires NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_SSH" "explicit target error"
}

test_explicit_dry_run_sets_reasonable_defaults() {
  local out
  out="$(
    env -i PATH="$PATH" HOME="$HOME" \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY=1 \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_DRY_RUN=1 \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_SSH=bench-host \
      bash "$SCRIPT"
  )"
  assert_contains "$out" "NVPN_HOST_PAIR_SSH=bench-host" "dry-run ssh"
  assert_contains "$out" "NVPN_HOST_PAIR_DURATION_SECS=30" "dry-run duration"
  assert_contains "$out" "NVPN_HOST_PAIR_PING_COUNT=300" "dry-run ping count"
  assert_contains "$out" "NVPN_HOST_PAIR_REQUIRE_IPERF=0" "dry-run ping-only default"
  assert_contains "$out" "NVPN_HOST_PAIR_MAX_PING_AVG_MS=150" "dry-run avg threshold"
  assert_contains "$out" "NVPN_HOST_PAIR_MAX_PING_LOSS_PERCENT=2" "dry-run loss threshold"
  assert_contains "$out" "soak-fips-dataplane-host-pair.sh" "dry-run command"
}

test_host_pair_env_overrides_release_defaults() {
  local out
  out="$(
    env -i PATH="$PATH" HOME="$HOME" \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY=1 \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_DRY_RUN=1 \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_SSH=bench-host \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_PING_COUNT=40 \
      NVPN_HOST_PAIR_PING_COUNT=7 \
      NVPN_HOST_PAIR_MAX_PING_AVG_MS=42 \
      bash "$SCRIPT"
  )"
  assert_contains "$out" "NVPN_HOST_PAIR_PING_COUNT=7" "host-pair ping override"
  assert_contains "$out" "NVPN_HOST_PAIR_MAX_PING_AVG_MS=42" "host-pair threshold override"
}

test_auto_dry_run_uses_configured_target() {
  local out
  out="$(
    env -i PATH="$PATH" HOME="$HOME" \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY=auto \
      NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_DRY_RUN=1 \
      NVPN_HOST_PAIR_SSH=bench-host \
      bash "$SCRIPT"
  )"
  assert_contains "$out" "NVPN_HOST_PAIR_SSH=bench-host" "auto dry-run ssh"
}

test_auto_without_target_skips
test_explicit_without_target_fails
test_explicit_dry_run_sets_reasonable_defaults
test_host_pair_env_overrides_release_defaults
test_auto_dry_run_uses_configured_target

printf 'release-gate host-pair latency harness passed\n'
