#!/usr/bin/env bash
# Local dry-run self-test for scripts/run-darwin-docker-wg-reference.sh.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

fail() {
  printf 'darwin/docker WG reference runner self-test failed: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle'"
}

assert_not_exists() {
  local path="$1"
  local label="$2"
  [[ ! -e "$path" ]] || fail "$label: unexpectedly exists at $path"
}

test_dry_run_maps_container_and_bench_env() {
  local dir out
  dir="$(mktemp -d)"
  out="$(
    NVPN_WG_DOCKER_REFERENCE_DRY_RUN=1 \
    NVPN_WG_DOCKER_REFERENCE_RUN_ID=test-run \
    NVPN_WG_DOCKER_REFERENCE_LOCAL_SSH=darwin-host \
    NVPN_WG_DOCKER_REFERENCE_LOCAL_UNDERLAY_IP=192.0.2.10 \
    NVPN_WG_DOCKER_REFERENCE_REMOTE_UNDERLAY_IP=192.0.2.20 \
    NVPN_WG_DOCKER_REFERENCE_REMOTE_SSH_PORT=22022 \
    NVPN_WG_DOCKER_REFERENCE_REMOTE_LISTEN_PORT=51899 \
    NVPN_WG_DOCKER_REFERENCE_OUTPUT_DIR="$dir/out" \
    NVPN_WG_DOCKER_REFERENCE_WIREGUARD_GO_SRC="$dir/wireguard-go" \
    "$ROOT_DIR/scripts/run-darwin-docker-wg-reference.sh"
  )"

  assert_contains "$out" "Docker image:" "dry-run image line"
  assert_contains "$out" "Container: nvpn-wg-ref-remote-test-run" "dry-run container name"
  assert_contains "$out" "Publish: 0.0.0.0:22022->22/tcp 0.0.0.0:51899->51899/udp" "dry-run published ports"
  assert_contains "$out" "Darwin SSH: darwin-host" "dry-run Darwin SSH"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_SSH=root@192.0.2.20" "bench remote SSH"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_SSH_PORT=22022" "bench remote SSH port"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_SSH_IDENTITY_FILE=/tmp/nvpn-wg-docker-reference-test-run/key/id_ed25519" "bench identity file"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_SSH_KNOWN_HOSTS_FILE=/tmp/nvpn-wg-docker-reference-test-run/key/known_hosts" "bench known_hosts file"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP=192.0.2.10" "bench local underlay"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP=192.0.2.20" "bench remote underlay"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_LOCAL_BACKEND_BIN=/opt/nvpn/bin/wireguard-go" "bench local backend"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_REMOTE_BACKEND_BIN=/usr/local/bin/wireguard-go" "bench remote backend"
  assert_contains "$out" "NVPN_WG_HOST_PAIR_REMOTE_WG_BIN=/usr/bin/wg" "bench remote wg"
  assert_not_exists "$dir/out" "dry-run output directory"
  rm -rf "$dir"
}

test_dry_run_requires_inputs() {
  local err
  err="$(mktemp)"
  if NVPN_WG_DOCKER_REFERENCE_DRY_RUN=1 "$ROOT_DIR/scripts/run-darwin-docker-wg-reference.sh" 2>"$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "dry-run without required env unexpectedly passed"
  fi
  grep -Fq 'set NVPN_WG_DOCKER_REFERENCE_LOCAL_SSH' "$err" \
    || fail "missing-env error did not mention local SSH"
  rm -f "$err"
}

test_dry_run_maps_container_and_bench_env
test_dry_run_requires_inputs

printf 'darwin/docker WG reference runner self-test passed\n'
