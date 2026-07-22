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
if grep -Eq '(windows_platform_lane_requested|docker_release_gates_enabled) \|\| return$' "$release_gate"; then
  fail "a disabled optional lane returns failure under set -e"
fi

printf 'release gate parallel harness passed\n'
