#!/usr/bin/env bash
# Local self-test for the host-pair comparison run orchestrator dry-run mapping.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

fail() {
  printf 'host-pair comparison runner self-test failed: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle'"
}

assert_not_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  [[ "$haystack" != *"$needle"* ]] || fail "$label: unexpectedly contained '$needle'"
}

assert_occurrences() {
  local haystack="$1"
  local needle="$2"
  local want="$3"
  local label="$4"
  local got
  got="$(grep -F -o "$needle" <<<"$haystack" | wc -l | tr -d '[:space:]')"
  [[ "$got" == "$want" ]] || fail "$label: got $got occurrences of '$needle', want $want"
}

assert_fails_with() {
  local label="$1"
  local pattern="$2"
  shift 2
  local err
  err="$(mktemp)"
  if "$@" 2>"$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "$label: command unexpectedly passed"
  fi
  if ! grep -Fq "$pattern" "$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "$label: expected stderr to contain '$pattern'"
  fi
  rm -f "$err"
}

test_preflight_summary_aggregates_child_rows() {
  local dir out summary status blockers
  dir="$(mktemp -d)"
  mkdir -p "$dir/nvpn" "$dir/reference-boringtun"
  cat >"$dir/nvpn/preflight.tsv" <<'EOF'
status	label	detail
ok	local jq is available	command=jq
missing	local peer selector resolves one daemon peer	selector=auto expected_ip=198.51.100.20 peer_count=2
EOF
  cat >"$dir/reference-boringtun/preflight.tsv" <<'EOF'
status	check
ok	backend is supported
missing	local underlay IP env is configured
EOF

  out="$(
    bash -c '
      source "$1"
      OUTPUT_DIR="$2"
      preflight_sources=()
      record_preflight_source nvpn clean "" "$2/nvpn/preflight.tsv"
      record_preflight_source reference clean boringtun "$2/reference-boringtun/preflight.tsv"
      record_preflight_source reference stress boringtun "$2/reference-boringtun/preflight.tsv"
      write_preflight_summary
    ' bash "$ROOT_DIR/scripts/run-host-pair-comparison.sh" "$dir"
  )"
  summary="$dir/preflight-summary.tsv"
  status="$dir/preflight-status.tsv"
  blockers="$dir/preflight-blockers.tsv"

  assert_contains "$out" "host-pair comparison preflight summary: $summary" "preflight summary output"
  assert_contains "$out" "host-pair comparison preflight status: $status" "preflight status output"
  assert_contains "$out" "host-pair comparison preflight blockers: $blockers" "preflight blockers output"
  [[ -f "$summary" ]] || fail "preflight summary was not written"
  [[ -f "$status" ]] || fail "preflight status was not written"
  [[ -f "$blockers" ]] || fail "preflight blockers were not written"
  [[ "$(sed -n '1p' "$summary")" == $'scope\tmode\tbackend\tstatus\tcheck\tdetail\tsource' ]] \
    || fail "preflight summary header was wrong"
  [[ "$(sed -n '1p' "$status")" == $'status\tcount' ]] \
    || fail "preflight status header was wrong"
  [[ "$(sed -n '1p' "$blockers")" == $'scope\tbackend\tcheck\tdetails\toccurrences\tmodes' ]] \
    || fail "preflight blockers header was wrong"
  grep -Fq $'nvpn\tclean\t\tmissing\tlocal peer selector resolves one daemon peer\tselector=auto expected_ip=198.51.100.20 peer_count=2\t' "$summary" \
    || fail "preflight summary did not include nvpn blocker"
  grep -Fq $'reference\tclean\tboringtun\tmissing\tlocal underlay IP env is configured\t\t' "$summary" \
    || fail "preflight summary did not include reference blocker"
  grep -Fxq $'ok\t3' "$status" \
    || fail "preflight status did not count ok rows"
  grep -Fxq $'missing\t3' "$status" \
    || fail "preflight status did not count missing rows"
  grep -Fxq $'reference\tboringtun\tlocal underlay IP env is configured\t-\t2\tclean,stress' "$blockers" \
    || fail "preflight blockers did not dedupe repeated reference blocker"
  grep -Fxq $'nvpn\t\tlocal peer selector resolves one daemon peer\tselector=auto expected_ip=198.51.100.20 peer_count=2\t1\tclean' "$blockers" \
    || fail "preflight blockers did not include nvpn blocker"

  rm -rf "$dir"
}

test_dry_run_maps_shared_knobs() {
  local dir out
  dir="$(mktemp -d)"
  out="$(
    NVPN_HOST_PAIR_COMPARISON_DRY_RUN=1 \
    NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR="$dir/bundle" \
    NVPN_HOST_PAIR_COMPARISON_RUN_ID=test-run \
    NVPN_HOST_PAIR_COMPARISON_SSH=bench-host \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_UNDERLAY_IP=192.0.2.10 \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_UNDERLAY_IP=192.0.2.20 \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_PEER=remote-peer-selector \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_PEER=local-peer-selector \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_NVPN=/tmp/local-nvpn \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_NVPN=/opt/nvpn/bin/remote-nvpn \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_NVPN_COMMAND=local-status-wrapper \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_NVPN_COMMAND=remote-status-wrapper \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_CONFIG=/tmp/local.toml \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_CONFIG=/etc/nvpn/remote.toml \
    NVPN_HOST_PAIR_COMPARISON_BACKEND=wireguard-go \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS=1 \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_SIDES=both \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_WORKERS=2 \
    NVPN_HOST_PAIR_COMPARISON_DURATION_SECS=120 \
    NVPN_HOST_PAIR_COMPARISON_INTERVAL_SECS=30 \
    NVPN_HOST_PAIR_COMPARISON_PING_COUNT=10 \
    NVPN_HOST_PAIR_COMPARISON_IPERF_DURATION_SECS=3 \
    "$ROOT_DIR/scripts/run-host-pair-comparison.sh"
  )"

  assert_contains "$out" "== nvpn/FIPS host-pair preflight ==" "nvpn preflight step"
  assert_contains "$out" "== userspace WireGuard preflight ==" "WG preflight step"
  assert_contains "$out" "== nvpn/FIPS host-pair row ==" "nvpn step"
  assert_contains "$out" "== userspace WireGuard reference row ==" "reference step"
  assert_contains "$out" "== normalize comparison artifacts ==" "comparison step"
  assert_contains "$out" "NVPN_HOST_PAIR_SSH=bench-host" "nvpn SSH mapping"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_SSH=bench-host" "WG SSH mapping"
  assert_contains "$out" "NVPN_HOST_PAIR_EXPECTED_LOCAL_UNDERLAY_IP=192.0.2.10" "nvpn local underlay"
  assert_contains "$out" "NVPN_HOST_PAIR_EXPECTED_REMOTE_UNDERLAY_IP=192.0.2.20" "nvpn remote underlay"
  assert_contains "$out" "NVPN_HOST_PAIR_LOCAL_PEER=remote-peer-selector" "nvpn local peer selector"
  assert_contains "$out" "NVPN_HOST_PAIR_REMOTE_PEER=local-peer-selector" "nvpn remote peer selector"
  assert_contains "$out" "NVPN_HOST_PAIR_LOCAL_NVPN=/tmp/local-nvpn" "nvpn local binary"
  assert_contains "$out" "NVPN_HOST_PAIR_REMOTE_NVPN=/opt/nvpn/bin/remote-nvpn" "nvpn remote binary"
  assert_contains "$out" "NVPN_HOST_PAIR_LOCAL_NVPN_COMMAND=local-status-wrapper" "nvpn local command"
  assert_contains "$out" "NVPN_HOST_PAIR_REMOTE_NVPN_COMMAND=remote-status-wrapper" "nvpn remote command"
  assert_contains "$out" "NVPN_HOST_PAIR_LOCAL_CONFIG=/tmp/local.toml" "nvpn local config"
  assert_contains "$out" "NVPN_HOST_PAIR_REMOTE_CONFIG=/etc/nvpn/remote.toml" "nvpn remote config"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP=192.0.2.10" "WG local underlay"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP=192.0.2.20" "WG remote underlay"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_BACKEND=wireguard-go" "reference backend"
  assert_contains "$out" "NVPN_HOST_PAIR_CPU_STRESS=1" "nvpn CPU stress"
  assert_contains "$out" "NVPN_HOST_PAIR_PREFLIGHT=1" "nvpn preflight flag"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_CPU_STRESS=1" "WG CPU stress"
  assert_contains "$out" "NVPN_HOST_PAIR_DURATION_SECS=120" "nvpn duration"
  assert_contains "$out" "NVPN_HOST_PAIR_INTERVAL_SECS=30" "nvpn interval"
  assert_contains "$out" "NVPN_HOST_PAIR_PING_COUNT=10" "nvpn ping count"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_PING_COUNT=10" "WG ping count"
  assert_contains "$out" "NVPN_HOST_PAIR_IPERF_DURATION_SECS=3" "nvpn iperf duration"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_IPERF_DURATION_SECS=3" "WG iperf duration"
  assert_contains "$out" "$dir/bundle/nvpn" "nvpn output dir"
  assert_contains "$out" "$dir/bundle/reference" "reference output dir"
  assert_contains "$out" "$dir/bundle/comparison" "comparison output dir"

  [[ ! -e "$dir/bundle" ]] || fail "dry-run created output bundle"
  rm -rf "$dir"
}

test_dry_run_maps_multiple_backends() {
  local dir out
  dir="$(mktemp -d)"
  out="$(
    NVPN_HOST_PAIR_COMPARISON_DRY_RUN=1 \
    NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR="$dir/bundle" \
    NVPN_HOST_PAIR_COMPARISON_SSH=bench-host \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_UNDERLAY_IP=192.0.2.10 \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_UNDERLAY_IP=192.0.2.20 \
    NVPN_HOST_PAIR_COMPARISON_BACKENDS=boringtun,wireguard-go \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS=1 \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_SIDES=remote \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_WORKERS=auto \
    "$ROOT_DIR/scripts/run-host-pair-comparison.sh"
  )"

  assert_contains "$out" "== nvpn/FIPS host-pair preflight ==" "nvpn preflight step"
  assert_contains "$out" "== userspace WireGuard preflight (boringtun) ==" "boringtun preflight step"
  assert_contains "$out" "== userspace WireGuard preflight (wireguard-go) ==" "wireguard-go preflight step"
  assert_occurrences "$out" "== nvpn/FIPS host-pair row ==" 1 "nvpn row count"
  assert_contains "$out" "== userspace WireGuard reference row (boringtun) ==" "boringtun reference step"
  assert_contains "$out" "== userspace WireGuard reference row (wireguard-go) ==" "wireguard-go reference step"
  assert_contains "$out" "== normalize comparison artifacts (boringtun) ==" "boringtun comparison step"
  assert_contains "$out" "== normalize comparison artifacts (wireguard-go) ==" "wireguard-go comparison step"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_BACKEND=boringtun" "boringtun backend"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_BACKEND=wireguard-go" "wireguard-go backend"
  assert_contains "$out" "$dir/bundle/nvpn" "shared nvpn output dir"
  assert_contains "$out" "$dir/bundle/reference-boringtun" "boringtun reference output dir"
  assert_contains "$out" "$dir/bundle/reference-wireguard-go" "wireguard-go reference output dir"
  assert_contains "$out" "$dir/bundle/comparison-boringtun" "boringtun comparison output dir"
  assert_contains "$out" "$dir/bundle/comparison-wireguard-go" "wireguard-go comparison output dir"

  [[ ! -e "$dir/bundle" ]] || fail "multi-backend dry-run created output bundle"
  rm -rf "$dir"
}

test_dry_run_maps_clean_and_stress_sweep() {
  local dir out
  dir="$(mktemp -d)"
  out="$(
    NVPN_HOST_PAIR_COMPARISON_DRY_RUN=1 \
    NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR="$dir/bundle" \
    NVPN_HOST_PAIR_COMPARISON_SSH=bench-host \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_UNDERLAY_IP=192.0.2.10 \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_UNDERLAY_IP=192.0.2.20 \
    NVPN_HOST_PAIR_COMPARISON_BACKENDS=boringtun,wireguard-go \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_MODES=clean,stress \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_SIDES=both \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_WORKERS=2 \
    "$ROOT_DIR/scripts/run-host-pair-comparison.sh"
  )"

  assert_contains "$out" "== nvpn/FIPS host-pair preflight (clean) ==" "clean nvpn preflight"
  assert_contains "$out" "== nvpn/FIPS host-pair preflight (stress) ==" "stress nvpn preflight"
  assert_contains "$out" "== userspace WireGuard preflight (clean/boringtun) ==" "clean boringtun preflight"
  assert_contains "$out" "== userspace WireGuard preflight (stress/wireguard-go) ==" "stress wireguard-go preflight"
  assert_contains "$out" "== nvpn/FIPS host-pair row (clean) ==" "clean nvpn row"
  assert_contains "$out" "== nvpn/FIPS host-pair row (stress) ==" "stress nvpn row"
  assert_occurrences "$out" "== nvpn/FIPS host-pair row (" 2 "sweep nvpn row count"
  assert_contains "$out" "== userspace WireGuard reference row (clean/boringtun) ==" "clean boringtun reference"
  assert_contains "$out" "== userspace WireGuard reference row (stress/wireguard-go) ==" "stress wireguard-go reference"
  assert_contains "$out" "== normalize comparison artifacts (clean/wireguard-go) ==" "clean wireguard-go comparison"
  assert_contains "$out" "== normalize comparison artifacts (stress/boringtun) ==" "stress boringtun comparison"
  assert_contains "$out" "NVPN_HOST_PAIR_CPU_STRESS=0" "clean nvpn CPU stress"
  assert_contains "$out" "NVPN_HOST_PAIR_CPU_STRESS=1" "stress nvpn CPU stress"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_CPU_STRESS=0" "clean WG CPU stress"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_CPU_STRESS=1" "stress WG CPU stress"
  assert_contains "$out" "$dir/bundle/clean/nvpn" "clean nvpn output dir"
  assert_contains "$out" "$dir/bundle/stress/nvpn" "stress nvpn output dir"
  assert_contains "$out" "$dir/bundle/clean/reference-boringtun" "clean boringtun reference dir"
  assert_contains "$out" "$dir/bundle/stress/reference-wireguard-go" "stress wireguard-go reference dir"
  assert_contains "$out" "$dir/bundle/clean/comparison-wireguard-go" "clean wireguard-go comparison dir"
  assert_contains "$out" "$dir/bundle/stress/comparison-boringtun" "stress boringtun comparison dir"

  [[ ! -e "$dir/bundle" ]] || fail "sweep dry-run created output bundle"
  rm -rf "$dir"
}

test_dry_run_can_skip_wg_preflight() {
  local dir out
  dir="$(mktemp -d)"
  out="$(
    NVPN_HOST_PAIR_COMPARISON_DRY_RUN=1 \
    NVPN_HOST_PAIR_COMPARISON_WG_PREFLIGHT=0 \
    NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR="$dir/bundle" \
    NVPN_HOST_PAIR_COMPARISON_SSH=bench-host \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_UNDERLAY_IP=192.0.2.10 \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_UNDERLAY_IP=192.0.2.20 \
    "$ROOT_DIR/scripts/run-host-pair-comparison.sh"
  )"
  assert_not_contains "$out" "== userspace WireGuard preflight ==" "preflight skip"
  assert_contains "$out" "== nvpn/FIPS host-pair preflight ==" "nvpn preflight still runs"
  assert_contains "$out" "== nvpn/FIPS host-pair row ==" "nvpn still runs"
  rm -rf "$dir"
}

test_dry_run_can_skip_nvpn_preflight() {
  local dir out
  dir="$(mktemp -d)"
  out="$(
    NVPN_HOST_PAIR_COMPARISON_DRY_RUN=1 \
    NVPN_HOST_PAIR_COMPARISON_NVPN_PREFLIGHT=0 \
    NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR="$dir/bundle" \
    NVPN_HOST_PAIR_COMPARISON_SSH=bench-host \
    NVPN_HOST_PAIR_COMPARISON_LOCAL_UNDERLAY_IP=192.0.2.10 \
    NVPN_HOST_PAIR_COMPARISON_REMOTE_UNDERLAY_IP=192.0.2.20 \
    "$ROOT_DIR/scripts/run-host-pair-comparison.sh"
  )"
  assert_not_contains "$out" "== nvpn/FIPS host-pair preflight ==" "nvpn preflight skip"
  assert_contains "$out" "== userspace WireGuard preflight ==" "WG preflight still runs"
  assert_contains "$out" "== nvpn/FIPS host-pair row ==" "nvpn still runs"
  rm -rf "$dir"
}

test_dry_run_preflight_only_allows_missing_underlay_env() {
  local dir out
  dir="$(mktemp -d)"
  out="$(
    NVPN_HOST_PAIR_COMPARISON_DRY_RUN=1 \
    NVPN_HOST_PAIR_COMPARISON_PREFLIGHT_ONLY=1 \
    NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR="$dir/bundle" \
    NVPN_HOST_PAIR_COMPARISON_SSH=bench-host \
    NVPN_HOST_PAIR_COMPARISON_BACKENDS=boringtun,wireguard-go \
    NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_MODES=clean,stress \
    "$ROOT_DIR/scripts/run-host-pair-comparison.sh"
  )"

  assert_contains "$out" "== nvpn/FIPS host-pair preflight (clean) ==" "clean nvpn preflight"
  assert_contains "$out" "== userspace WireGuard preflight (stress/wireguard-go) ==" "stress wireguard-go preflight"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP=" "WG missing local underlay is passed through"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP=" "WG missing remote underlay is passed through"
  assert_contains "$out" "host-pair comparison preflight bundle: $dir/bundle" "preflight bundle output"
  assert_not_contains "$out" "== nvpn/FIPS host-pair row" "preflight-only skips nvpn rows"
  assert_not_contains "$out" "== userspace WireGuard reference row" "preflight-only skips reference rows"
  assert_not_contains "$out" "== normalize comparison artifacts" "preflight-only skips comparisons"
  assert_not_contains "$out" "== summarize comparison matrix" "preflight-only skips summary"
  [[ ! -e "$dir/bundle" ]] || fail "preflight-only dry-run created output bundle"
  rm -rf "$dir"
}

test_non_preflight_run_requires_underlay_env() {
  assert_fails_with \
    "missing underlay env" \
    "set local and remote underlay IPs" \
    env \
      NVPN_HOST_PAIR_COMPARISON_DRY_RUN=1 \
      NVPN_HOST_PAIR_COMPARISON_SSH=bench-host \
      "$ROOT_DIR/scripts/run-host-pair-comparison.sh"
}

test_preflight_summary_aggregates_child_rows
test_dry_run_maps_shared_knobs
test_dry_run_maps_multiple_backends
test_dry_run_maps_clean_and_stress_sweep
test_dry_run_can_skip_wg_preflight
test_dry_run_can_skip_nvpn_preflight
test_dry_run_preflight_only_allows_missing_underlay_env
test_non_preflight_run_requires_underlay_env

printf 'host-pair comparison runner self-test passed\n'
