#!/usr/bin/env bash
# Local self-tests for the e2e Docker base-image helper.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/build-e2e-docker-base-images.sh"

fail() {
  printf 'e2e Docker base image helper self-test failed: %s\n' "$*" >&2
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

assert_eq() {
  local got="$1"
  local want="$2"
  local label="$3"
  [[ "$got" == "$want" ]] || fail "$label: got '$got', want '$want'"
}

test_dry_run_prints_builds_and_env() {
  local out
  out="$(
    NVPN_E2E_BASE_DRY_RUN=1 \
    NVPN_E2E_BASE_BUILDER_FROM=fixture-rust:latest \
    NVPN_E2E_BASE_RUNTIME_FROM=fixture-debian:latest \
    NVPN_E2E_BUILDER_IMAGE=localhost/fixture-builder:local \
    NVPN_E2E_RUNTIME_IMAGE=localhost/fixture-runtime:local \
    "$SCRIPT"
  )"

  assert_contains "$out" "== build builder e2e base image ==" "builder step"
  assert_contains "$out" "== build runtime e2e base image ==" "runtime step"
  assert_contains "$out" "docker build -t localhost/fixture-builder:local --build-arg BASE_IMAGE=fixture-rust:latest" "builder docker build"
  assert_contains "$out" "docker build -t localhost/fixture-runtime:local --build-arg BASE_IMAGE=fixture-debian:latest" "runtime docker build"
  assert_contains "$out" "libclang-dev libdbus-1-dev pkg-config" "builder packages"
  assert_contains "$out" "iperf3 iproute2 iptables iputils-ping" "runtime packages"
  assert_contains "$out" "NVPN_E2E_BUILDER_APT_INSTALL=0" "builder apt skip env"
  assert_contains "$out" "NVPN_E2E_RUNTIME_APT_INSTALL=0" "runtime apt skip env"
}

test_dry_run_can_request_pull() {
  local out
  out="$(
    NVPN_E2E_BASE_DRY_RUN=1 \
    NVPN_E2E_BASE_PULL=1 \
    "$SCRIPT"
  )"
  assert_contains "$out" "--pull" "pull flag"
}

test_default_tags_are_localhost_qualified() {
  local builder runtime
  builder="$(
    bash -c 'source "$1"; printf "%s\n" "$BUILDER_TAG"' bash "$SCRIPT"
  )"
  runtime="$(
    bash -c 'source "$1"; printf "%s\n" "$RUNTIME_TAG"' bash "$SCRIPT"
  )"
  assert_eq "$builder" "localhost/nostr-vpn-e2e-builder:local" "default builder tag"
  assert_eq "$runtime" "localhost/nostr-vpn-e2e-runtime:local" "default runtime tag"
}

test_help_does_not_build() {
  local out
  out="$("$SCRIPT" --help)"
  assert_contains "$out" "NVPN_E2E_BASE_BUILDER_FROM" "help env"
  assert_not_contains "$out" "== build builder" "help build"
}

test_dry_run_prints_builds_and_env
test_dry_run_can_request_pull
test_default_tags_are_localhost_qualified
test_help_does_not_build

printf 'e2e Docker base image helper self-test passed\n'
