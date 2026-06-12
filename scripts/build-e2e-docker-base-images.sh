#!/usr/bin/env bash
# Build reusable local base images for Docker e2e/perf runs.
#
# The e2e Dockerfile normally builds directly from Rust/Debian bases. This
# helper pre-bakes the apt dependencies into local images so repeated perf runs
# can use NVPN_E2E_* overrides and avoid Docker Hub/frontend metadata fetches in
# the hot benchmark loop.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BUILDER_FROM="${NVPN_E2E_BASE_BUILDER_FROM:-rust:1.93-bookworm}"
RUNTIME_FROM="${NVPN_E2E_BASE_RUNTIME_FROM:-debian:bookworm-slim}"
BUILDER_TAG="${NVPN_E2E_BUILDER_IMAGE:-localhost/nostr-vpn-e2e-builder:local}"
RUNTIME_TAG="${NVPN_E2E_RUNTIME_IMAGE:-localhost/nostr-vpn-e2e-runtime:local}"
DRY_RUN="${NVPN_E2E_BASE_DRY_RUN:-0}"
PULL="${NVPN_E2E_BASE_PULL:-0}"

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

print_command() {
  local first=1 arg
  for arg in "$@"; do
    if (( first )); then
      first=0
    else
      printf ' '
    fi
    printf '%q' "$arg"
  done
  printf '\n'
}

usage() {
  cat <<EOF
usage: scripts/build-e2e-docker-base-images.sh

Builds local base images for Docker e2e/perf runs and prints the env needed to
use them with Dockerfile.e2e.

Environment:
  NVPN_E2E_BASE_BUILDER_FROM    source builder image (default: $BUILDER_FROM)
  NVPN_E2E_BASE_RUNTIME_FROM    source runtime image (default: $RUNTIME_FROM)
  NVPN_E2E_BUILDER_IMAGE        local builder tag (default: $BUILDER_TAG)
  NVPN_E2E_RUNTIME_IMAGE        local runtime tag (default: $RUNTIME_TAG)
  NVPN_E2E_BASE_PULL            set 1 to pass --pull to docker build
  NVPN_E2E_BASE_DRY_RUN         set 1 to print commands only

After a successful build:
  NVPN_E2E_BUILDER_IMAGE=$BUILDER_TAG \\
  NVPN_E2E_RUNTIME_IMAGE=$RUNTIME_TAG \\
  NVPN_E2E_BUILDER_APT_INSTALL=0 \\
  NVPN_E2E_RUNTIME_APT_INSTALL=0 \\
  scripts/e2e-fips-perf-regression-docker.sh
EOF
}

dockerfile_for() {
  case "$1" in
    builder)
      cat <<'EOF'
ARG BASE_IMAGE=rust:1.93-bookworm
FROM ${BASE_IMAGE}
RUN apt-get update \
    && apt-get install -y --no-install-recommends libclang-dev libdbus-1-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
EOF
      ;;
    runtime)
      cat <<'EOF'
ARG BASE_IMAGE=debian:bookworm-slim
FROM ${BASE_IMAGE}
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates dnsutils iperf3 iproute2 iptables iputils-ping libdbus-1-3 netcat-openbsd nftables procps tcpdump wireguard-tools \
    && rm -rf /var/lib/apt/lists/*
EOF
      ;;
    *)
      printf 'unknown e2e base image kind: %s\n' "$1" >&2
      return 2
      ;;
  esac
}

build_image() {
  local kind="$1"
  local base="$2"
  local tag="$3"
  local dockerfile
  local -a cmd

  dockerfile="$(mktemp)"
  dockerfile_for "$kind" >"$dockerfile"

  cmd=(docker build -t "$tag" --build-arg "BASE_IMAGE=$base")
  if is_true "$PULL"; then
    cmd+=(--pull)
  fi
  cmd+=(-f "$dockerfile" "$ROOT_DIR")

  printf '== build %s e2e base image ==\n' "$kind"
  if is_true "$DRY_RUN"; then
    print_command "${cmd[@]}"
    printf '%s\n' "--- ${kind} Dockerfile ---"
    sed -n '1,120p' "$dockerfile"
  else
    "${cmd[@]}"
  fi
  rm -f "$dockerfile"
}

print_env() {
  cat <<EOF
e2e base images ready:
  builder: $BUILDER_TAG
  runtime: $RUNTIME_TAG

Use with perf/e2e runs:
  NVPN_E2E_BUILDER_IMAGE=$BUILDER_TAG \\
  NVPN_E2E_RUNTIME_IMAGE=$RUNTIME_TAG \\
  NVPN_E2E_BUILDER_APT_INSTALL=0 \\
  NVPN_E2E_RUNTIME_APT_INSTALL=0 \\
  scripts/e2e-fips-perf-regression-docker.sh
EOF
}

main() {
  case "${1:-}" in
    -h|--help)
      usage
      return 0
      ;;
    "")
      ;;
    *)
      usage >&2
      printf 'unknown argument: %s\n' "$1" >&2
      return 2
      ;;
  esac

  build_image builder "$BUILDER_FROM" "$BUILDER_TAG"
  build_image runtime "$RUNTIME_FROM" "$RUNTIME_TAG"
  print_env
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
