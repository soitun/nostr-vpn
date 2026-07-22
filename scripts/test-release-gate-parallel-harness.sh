#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/lib-release-gate-parallel.sh"

fail() {
  printf 'release gate parallel harness failed: %s\n' "$*" >&2
  exit 1
}

tmp="$(mktemp -d "${TMPDIR:-/tmp}/nvpn-release-gate-parallel.XXXXXX")"
trap 'release_gate_parallel_cancel_all; rm -rf "$tmp"' EXIT
release_gate_parallel_init "$tmp/logs"

lane_waits_for_peer() {
  local peer_marker="$1"
  local own_marker="$2"
  local end=$(( $(date +%s) + 5 ))
  : >"$own_marker"
  while [[ ! -f "$peer_marker" && "$(date +%s)" -le "$end" ]]; do
    sleep 0.05
  done
  [[ -f "$peer_marker" ]]
  printf 'saw peer lane\n'
}

release_gate_parallel_start "first lane" lane_waits_for_peer "$tmp/second" "$tmp/first"
first="$RELEASE_GATE_PARALLEL_LAST_INDEX"
release_gate_parallel_start "second lane" lane_waits_for_peer "$tmp/first" "$tmp/second"
second="$RELEASE_GATE_PARALLEL_LAST_INDEX"
release_gate_parallel_wait "$first" >/dev/null
release_gate_parallel_wait "$second" >/dev/null
grep -Fq 'saw peer lane' "$tmp/logs/first-lane-0.log" \
  || fail "first lane did not run concurrently"
grep -Fq 'saw peer lane' "$tmp/logs/second-lane-1.log" \
  || fail "second lane did not run concurrently"

lane_fails() {
  printf 'intentional lane failure\n'
  return 7
}

release_gate_parallel_start "failing lane" lane_fails
failing="$RELEASE_GATE_PARALLEL_LAST_INDEX"
set +e
release_gate_parallel_wait "$failing" >/dev/null 2>&1
status=$?
set -e
[[ "$status" == "7" ]] || fail "failing lane returned $status instead of 7"
grep -Fq 'intentional lane failure' "$tmp/logs/failing-lane-2.log" \
  || fail "failing lane log was not preserved"

release_gate="$ROOT_DIR/scripts/release-gate.sh"
grep -Fq 'release_gate_parallel_start "Windows platform"' "$release_gate" \
  || fail "release gate does not dispatch the remote Windows lane"
grep -Fq 'release_gate_parallel_start "Docker node image build"' "$release_gate" \
  || fail "release gate does not overlap the reusable Docker build with host validation"
grep -Fq 'export NVPN_E2E_SKIP_NODE_BUILD=1' "$release_gate" \
  || fail "release gate does not reuse its prebuilt Docker node image"
grep -Fq 'cache_to:' "$ROOT_DIR/docker-compose.e2e.yml" \
  || fail "shared Docker node image does not persist reusable cache metadata"
grep -Fq 'type=local,src=${NVPN_E2E_BUILDX_CACHE_DIR:-.buildx-cache/e2e-node}' "$ROOT_DIR/docker-compose.e2e.yml" \
  || fail "shared Docker node image does not import its local BuildKit cache"
grep -Fq 'type=local,dest=${NVPN_E2E_BUILDX_CACHE_DIR:-.buildx-cache/e2e-node},mode=max' "$ROOT_DIR/docker-compose.e2e.yml" \
  || fail "shared Docker node image does not export a complete local BuildKit cache"
grep -Fq 'cache_to:' "$ROOT_DIR/linux/docker-compose.yml" \
  || fail "Linux GUI image does not persist reusable cache metadata"
grep -Fq 'type=local,src=${NVPN_LINUX_BUILDX_CACHE_DIR:-../.buildx-cache/linux-gui}' "$ROOT_DIR/linux/docker-compose.yml" \
  || fail "Linux GUI image does not import its local BuildKit cache"
grep -Fq 'type=local,dest=${NVPN_LINUX_BUILDX_CACHE_DIR:-../.buildx-cache/linux-gui},mode=max' "$ROOT_DIR/linux/docker-compose.yml" \
  || fail "Linux GUI image does not export a complete local BuildKit cache"

docker_wait_line="$(grep -n 'release_gate_parallel_wait "$docker_build_lane"' "$release_gate" | cut -d: -f1)"
signal_line="$(grep -n '^  run_docker_signal_gates$' "$release_gate" | cut -d: -f1)"
functional_line="$(grep -n '^  run_docker_isolated_functional_gates$' "$release_gate" | cut -d: -f1)"
perf_line="$(grep -n '^  run_docker_perf_gate$' "$release_gate" | cut -d: -f1)"
[[ -n "$docker_wait_line" && -n "$signal_line" && -n "$functional_line" && -n "$perf_line" ]] \
  || fail "release gate serial phase markers are incomplete"
(( docker_wait_line < signal_line && signal_line < functional_line && functional_line < perf_line )) \
  || fail "release gate does not join builds and functional lanes before performance phases"
grep -Fq 'release_gate_parallel_start "Docker NAT-safe MTU"' "$release_gate" \
  || fail "release gate does not dispatch the isolated NAT functional lane"
grep -Fq 'release_gate_parallel_start "Docker kernel WireGuard exit"' "$release_gate" \
  || fail "release gate does not dispatch the isolated kernel WireGuard lane"
grep -Fq 'release_gate_parallel_start "Docker userspace WireGuard exit"' "$release_gate" \
  || fail "release gate does not dispatch the isolated userspace WireGuard lane"
grep -Fq 'NVPN_WG_EXIT_USERSPACE_INTERNET_SUBNET' "$release_gate" \
  || fail "parallel userspace WireGuard fixture has no isolated subnet"
grep -Fq 'Release gate test selector matched no passing test' "$release_gate" \
  || fail "focused release-gate tests can pass with an empty selector"
grep -Fq -- '--skip websocket_seed_router_delivers_join_roster_to_guest_without_preconfigured_admin' "$release_gate" \
  || fail "the strict QR-join latency gate still runs during the cold Docker build"
grep -Fq 'desktop_mobile_manual_join_desktop_admin_to_mobile_joiner' "$release_gate" \
  || fail "release gate does not prove desktop-admin to mobile-joiner delivery"
grep -Fq 'desktop_mobile_manual_join_mobile_admin_to_desktop_joiner' "$release_gate" \
  || fail "release gate does not prove mobile-admin to desktop-joiner delivery"
grep -Fq 'NVPN_RELEASE_GATE_QR_JOIN_LATENCY' "$release_gate" \
  || fail "the strict QR-join latency gate cannot be scoped to calibrated hosts"
grep -Fq 'NVPN_RELEASE_GATE_TARGET_SECS:-1800' "$release_gate" \
  || fail "release gate has no explicit 30-minute wall-clock target"
grep -Fq 'NVPN_E2E_DIRECT_RECOVERY_SECS:-20' "$ROOT_DIR/scripts/e2e-fips-roaming-docker.sh" \
  || fail "FIPS direct recovery can wait longer than the verified 20-second gate"
grep -Fq 'NVPN_DESKTOP_ROSTER_E2E_TIMEOUT_SECS:-30' "$ROOT_DIR/scripts/e2e-desktop-roster-join.sh" \
  || fail "desktop roster acceptance failures wait longer than 30 seconds"
grep -Fq 'NVPN_MOBILE_WG_EXIT_INSTALL_IOS="$((1 - MOBILE_IOS_APP_READY))"' "$release_gate" \
  || fail "release gate rebuilds the same physical iOS app for the exit lane"
grep -Fq 'NVPN_MOBILE_JOIN_E2E_INSTALL_IOS="$((1 - MOBILE_IOS_APP_READY))"' "$release_gate" \
  || fail "release gate rebuilds the same physical iOS app for the join lane"
if grep -Eq '(windows_platform_lane_requested|docker_release_gates_enabled) \|\| return$' "$release_gate"; then
  fail "a disabled optional lane returns failure under set -e"
fi

printf 'release gate parallel harness passed\n'
