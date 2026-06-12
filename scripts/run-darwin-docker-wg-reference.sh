#!/usr/bin/env bash
# Run the userspace WireGuard reference harness from a Darwin host against a
# throwaway Linux Docker remote on this machine.
#
# The Darwin side is reached over SSH and must already have the narrow local
# helper plus trusted wg/backend binaries installed. The Linux side is created
# as a temporary container with SSH, /dev/net/tun, NET_ADMIN, wg, iperf3, and a
# wireguard-go binary built from a local source checkout.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

LOCAL_SSH="${NVPN_WG_DOCKER_REFERENCE_LOCAL_SSH:-}"
LOCAL_SSH_PORT="${NVPN_WG_DOCKER_REFERENCE_LOCAL_SSH_PORT:-}"
LOCAL_SSH_CONNECT_TIMEOUT="${NVPN_WG_DOCKER_REFERENCE_LOCAL_SSH_CONNECT_TIMEOUT:-10}"
LOCAL_UNDERLAY_IP="${NVPN_WG_DOCKER_REFERENCE_LOCAL_UNDERLAY_IP:-}"
REMOTE_UNDERLAY_IP="${NVPN_WG_DOCKER_REFERENCE_REMOTE_UNDERLAY_IP:-}"
REMOTE_SSH_HOST="${NVPN_WG_DOCKER_REFERENCE_REMOTE_SSH_HOST:-$REMOTE_UNDERLAY_IP}"
RUN_ID="${NVPN_WG_DOCKER_REFERENCE_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
OUTPUT_DIR="${NVPN_WG_DOCKER_REFERENCE_OUTPUT_DIR:-$ROOT_DIR/artifacts/userspace-wg-docker-reference/$RUN_ID}"
LOCAL_WORK_DIR="${NVPN_WG_DOCKER_REFERENCE_LOCAL_WORK_DIR:-/tmp/nvpn-wg-docker-reference-$RUN_ID}"
DOCKER_IMAGE="${NVPN_WG_DOCKER_REFERENCE_IMAGE:-localhost/nostr-vpn-e2e-runtime:local}"
DOCKER_BIN="${NVPN_WG_DOCKER_REFERENCE_DOCKER_BIN:-docker}"
CONTAINER_NAME="${NVPN_WG_DOCKER_REFERENCE_CONTAINER:-nvpn-wg-ref-remote-$RUN_ID}"
PUBLISH_ADDR="${NVPN_WG_DOCKER_REFERENCE_PUBLISH_ADDR:-0.0.0.0}"
REMOTE_SSH_PORT="${NVPN_WG_DOCKER_REFERENCE_REMOTE_SSH_PORT:-2222}"
REMOTE_LISTEN_PORT="${NVPN_WG_DOCKER_REFERENCE_REMOTE_LISTEN_PORT:-51871}"
BACKEND="${NVPN_WG_DOCKER_REFERENCE_BACKEND:-wireguard-go}"
LOCAL_BACKEND_BIN="${NVPN_WG_DOCKER_REFERENCE_LOCAL_BACKEND_BIN:-/opt/nvpn/bin/wireguard-go}"
REMOTE_BACKEND_BIN="${NVPN_WG_DOCKER_REFERENCE_REMOTE_BACKEND_BIN:-/usr/local/bin/wireguard-go}"
LOCAL_WG_BIN="${NVPN_WG_DOCKER_REFERENCE_LOCAL_WG_BIN:-/opt/nvpn/bin/wg}"
REMOTE_WG_BIN="${NVPN_WG_DOCKER_REFERENCE_REMOTE_WG_BIN:-/usr/bin/wg}"
WIREGUARD_GO_SRC="${NVPN_WG_DOCKER_REFERENCE_WIREGUARD_GO_SRC:-$HOME/src/wireguard-go}"
PING_COUNT="${NVPN_WG_DOCKER_REFERENCE_PING_COUNT:-20}"
IPERF_DURATION_SECS="${NVPN_WG_DOCKER_REFERENCE_IPERF_DURATION_SECS:-5}"
WG_MTU="${NVPN_WG_DOCKER_REFERENCE_MTU:-1060}"
RUN_PREFLIGHT="${NVPN_WG_DOCKER_REFERENCE_PREFLIGHT:-1}"
PREFLIGHT_ONLY="${NVPN_WG_DOCKER_REFERENCE_PREFLIGHT_ONLY:-0}"
KEEP="${NVPN_WG_DOCKER_REFERENCE_KEEP:-0}"
DRY_RUN="${NVPN_WG_DOCKER_REFERENCE_DRY_RUN:-0}"

LOCAL_SSH_OPTS=(-o BatchMode=yes -o "ConnectTimeout=$LOCAL_SSH_CONNECT_TIMEOUT" -o StrictHostKeyChecking=accept-new)
LOCAL_SCP_OPTS=(-o BatchMode=yes -o "ConnectTimeout=$LOCAL_SSH_CONNECT_TIMEOUT" -o StrictHostKeyChecking=accept-new)
if [[ -n "$LOCAL_SSH_PORT" ]]; then
  LOCAL_SSH_OPTS=(-p "$LOCAL_SSH_PORT" "${LOCAL_SSH_OPTS[@]}")
  LOCAL_SCP_OPTS=(-P "$LOCAL_SSH_PORT" "${LOCAL_SCP_OPTS[@]}")
fi

KEY_DIR=""
BUILD_DIR=""
REMOTE_KEY_PATH="$LOCAL_WORK_DIR/key/id_ed25519"
REMOTE_KNOWN_HOSTS_PATH="$LOCAL_WORK_DIR/key/known_hosts"
REMOTE_ARTIFACT_DIR="$LOCAL_WORK_DIR/artifacts/userspace-wg-host-pair/$RUN_ID"

die() {
  printf 'darwin/docker WG reference failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
usage: NVPN_WG_DOCKER_REFERENCE_LOCAL_SSH=user@darwin-host \
       NVPN_WG_DOCKER_REFERENCE_LOCAL_UNDERLAY_IP=<darwin-ip> \
       NVPN_WG_DOCKER_REFERENCE_REMOTE_UNDERLAY_IP=<docker-host-ip> \
       scripts/run-darwin-docker-wg-reference.sh

This creates a temporary Linux Docker remote, stages the userspace WireGuard
host-pair harness on the Darwin host, runs a short reference row, copies
non-secret artifacts back, then removes the container and temporary SSH key.

Common optional env:
  NVPN_WG_DOCKER_REFERENCE_IMAGE              Docker runtime image
  NVPN_WG_DOCKER_REFERENCE_WIREGUARD_GO_SRC   local wireguard-go checkout
  NVPN_WG_DOCKER_REFERENCE_REMOTE_SSH_HOST    host/IP the Darwin side uses for Docker SSH
  NVPN_WG_DOCKER_REFERENCE_REMOTE_SSH_PORT    published Docker SSH port (default 2222)
  NVPN_WG_DOCKER_REFERENCE_REMOTE_LISTEN_PORT published WireGuard UDP port (default 51871)
  NVPN_WG_DOCKER_REFERENCE_OUTPUT_DIR         local artifact output directory
  NVPN_WG_DOCKER_REFERENCE_PREFLIGHT_ONLY     set 1 to stop after preflight
  NVPN_WG_DOCKER_REFERENCE_KEEP               keep container and Darwin work dir
  NVPN_WG_DOCKER_REFERENCE_DRY_RUN            print planned commands only
EOF
}

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

q() {
  printf '%q' "$1"
}

quote_command() {
  local arg sep=""
  for arg in "$@"; do
    printf '%s%q' "$sep" "$arg"
    sep=" "
  done
}

run() {
  printf '== %s ==\n' "$*"
  "$@"
}

remote_sh() {
  local cmd="$1"
  ssh "${LOCAL_SSH_OPTS[@]}" "$LOCAL_SSH" "bash -lc $(q "$cmd")"
}

cleanup() {
  local rc=$?
  set +e
  if ! is_true "$KEEP"; then
    if [[ -n "$CONTAINER_NAME" ]]; then
      "$DOCKER_BIN" rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
    fi
    if [[ -n "$LOCAL_SSH" && -n "$LOCAL_WORK_DIR" ]]; then
      remote_sh "rm -rf $(q "$LOCAL_WORK_DIR")" >/dev/null 2>&1 || true
    fi
    [[ -n "$KEY_DIR" ]] && rm -rf "$KEY_DIR"
    [[ -n "$BUILD_DIR" ]] && rm -rf "$BUILD_DIR"
  else
    printf 'kept Docker container %s and Darwin work dir %s\n' "$CONTAINER_NAME" "$LOCAL_WORK_DIR" >&2
  fi
  exit "$rc"
}

validate_config() {
  [[ -n "$LOCAL_SSH" ]] || {
    usage
    die "set NVPN_WG_DOCKER_REFERENCE_LOCAL_SSH"
  }
  [[ -n "$LOCAL_UNDERLAY_IP" ]] || die "set NVPN_WG_DOCKER_REFERENCE_LOCAL_UNDERLAY_IP"
  [[ -n "$REMOTE_UNDERLAY_IP" ]] || die "set NVPN_WG_DOCKER_REFERENCE_REMOTE_UNDERLAY_IP"
  [[ "$BACKEND" == "wireguard-go" ]] || die "only wireguard-go is supported by this Docker reference runner for now"
}

print_dry_run() {
  validate_config
  cat <<EOF
Docker image: $DOCKER_IMAGE
Container: $CONTAINER_NAME
Publish: $PUBLISH_ADDR:$REMOTE_SSH_PORT->22/tcp $PUBLISH_ADDR:$REMOTE_LISTEN_PORT->$REMOTE_LISTEN_PORT/udp
Darwin SSH: $LOCAL_SSH
Darwin work dir: $LOCAL_WORK_DIR
Output dir: $OUTPUT_DIR
wireguard-go source: $WIREGUARD_GO_SRC
Benchmark env:
  NVPN_WG_HOST_PAIR_SSH=root@$REMOTE_SSH_HOST
  NVPN_WG_HOST_PAIR_SSH_PORT=$REMOTE_SSH_PORT
  NVPN_WG_HOST_PAIR_SSH_IDENTITY_FILE=$REMOTE_KEY_PATH
  NVPN_WG_HOST_PAIR_SSH_KNOWN_HOSTS_FILE=$REMOTE_KNOWN_HOSTS_PATH
  NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP=$LOCAL_UNDERLAY_IP
  NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP=$REMOTE_UNDERLAY_IP
  NVPN_WG_HOST_PAIR_LOCAL_BACKEND_BIN=$LOCAL_BACKEND_BIN
  NVPN_WG_HOST_PAIR_REMOTE_BACKEND_BIN=$REMOTE_BACKEND_BIN
  NVPN_WG_HOST_PAIR_REMOTE_WG_BIN=$REMOTE_WG_BIN
  NVPN_WG_HOST_PAIR_PING_COUNT=$PING_COUNT
  NVPN_WG_HOST_PAIR_IPERF_DURATION_SECS=$IPERF_DURATION_SECS
EOF
}

container_goarch() {
  local arch
  arch="$("$DOCKER_BIN" exec "$CONTAINER_NAME" uname -m)"
  case "$arch" in
    aarch64|arm64) printf 'arm64\n' ;;
    x86_64|amd64) printf 'amd64\n' ;;
    armv7l) printf 'arm\n' ;;
    *) die "unsupported container architecture for wireguard-go build: $arch" ;;
  esac
}

start_container() {
  "$DOCKER_BIN" rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
  run "$DOCKER_BIN" run -d \
    --name "$CONTAINER_NAME" \
    --cap-add NET_ADMIN \
    --device /dev/net/tun \
    -p "$PUBLISH_ADDR:$REMOTE_SSH_PORT:22/tcp" \
    -p "$PUBLISH_ADDR:$REMOTE_LISTEN_PORT:$REMOTE_LISTEN_PORT/udp" \
    "$DOCKER_IMAGE" sleep infinity >/dev/null

  run "$DOCKER_BIN" exec "$CONTAINER_NAME" sh -lc \
    'set -e; apt-get update >/dev/null; DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server sudo ca-certificates >/dev/null; mkdir -p /run/sshd /root/.ssh; chmod 700 /root/.ssh; printf "PermitRootLogin prohibit-password\nPasswordAuthentication no\nPubkeyAuthentication yes\n" >/etc/ssh/sshd_config.d/nvpn-ref.conf'
}

install_container_key() {
  KEY_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nvpn-wg-ref-key.XXXXXX")"
  ssh-keygen -q -t ed25519 -N '' -f "$KEY_DIR/id_ed25519"
  "$DOCKER_BIN" cp "$KEY_DIR/id_ed25519.pub" "$CONTAINER_NAME":/root/.ssh/authorized_keys
  run "$DOCKER_BIN" exec "$CONTAINER_NAME" sh -lc \
    'set -e; chown root:root /root/.ssh/authorized_keys; chmod 600 /root/.ssh/authorized_keys'
}

install_wireguard_go() {
  local goarch out
  [[ -d "$WIREGUARD_GO_SRC" ]] || die "wireguard-go source not found: $WIREGUARD_GO_SRC"
  command -v go >/dev/null 2>&1 || die "local Go toolchain is required to build Linux wireguard-go"
  BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nvpn-wg-ref-build.XXXXXX")"
  goarch="$(container_goarch)"
  out="$BUILD_DIR/wireguard-go"
  run bash -lc "cd $(q "$WIREGUARD_GO_SRC") && GOOS=linux GOARCH=$(q "$goarch") CGO_ENABLED=0 go build -o $(q "$out") ."
  "$DOCKER_BIN" cp "$out" "$CONTAINER_NAME":"$REMOTE_BACKEND_BIN"
  run "$DOCKER_BIN" exec "$CONTAINER_NAME" sh -lc \
    "set -e; chmod 0755 $(q "$REMOTE_BACKEND_BIN"); command -v $(q "$REMOTE_WG_BIN") >/dev/null; command -v iperf3 >/dev/null; command -v ip >/dev/null; test -e /dev/net/tun"
}

start_sshd() {
  run "$DOCKER_BIN" exec "$CONTAINER_NAME" sh -lc '/usr/sbin/sshd || true; pgrep -a sshd >/dev/null'
}

stage_darwin_runner() {
  remote_sh "rm -rf $(q "$LOCAL_WORK_DIR"); mkdir -p $(q "$LOCAL_WORK_DIR/scripts") $(q "$LOCAL_WORK_DIR/key")"
  scp "${LOCAL_SCP_OPTS[@]}" "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh" "$LOCAL_SSH:$LOCAL_WORK_DIR/scripts/" >/dev/null
  scp "${LOCAL_SCP_OPTS[@]}" "$KEY_DIR/id_ed25519" "$LOCAL_SSH:$REMOTE_KEY_PATH" >/dev/null
  remote_sh "chmod 600 $(q "$REMOTE_KEY_PATH")"
  remote_sh ": >$(q "$REMOTE_KNOWN_HOSTS_PATH"); chmod 600 $(q "$REMOTE_KNOWN_HOSTS_PATH")"
}

bench_env_command() {
  local preflight="$1"
  local -a envs=(
    "NVPN_WG_HOST_PAIR_SSH=root@$REMOTE_SSH_HOST"
    "NVPN_WG_HOST_PAIR_SSH_PORT=$REMOTE_SSH_PORT"
    "NVPN_WG_HOST_PAIR_SSH_IDENTITY_FILE=$REMOTE_KEY_PATH"
    "NVPN_WG_HOST_PAIR_SSH_KNOWN_HOSTS_FILE=$REMOTE_KNOWN_HOSTS_PATH"
    "NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP=$LOCAL_UNDERLAY_IP"
    "NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP=$REMOTE_UNDERLAY_IP"
    "NVPN_WG_HOST_PAIR_BACKEND=$BACKEND"
    "NVPN_WG_HOST_PAIR_LOCAL_BACKEND_BIN=$LOCAL_BACKEND_BIN"
    "NVPN_WG_HOST_PAIR_REMOTE_BACKEND_BIN=$REMOTE_BACKEND_BIN"
    "NVPN_WG_HOST_PAIR_REMOTE_WG_BIN=$REMOTE_WG_BIN"
    "NVPN_WG_HOST_PAIR_REMOTE_LISTEN_PORT=$REMOTE_LISTEN_PORT"
    "NVPN_WG_HOST_PAIR_MTU=$WG_MTU"
    "NVPN_WG_HOST_PAIR_PING_COUNT=$PING_COUNT"
    "NVPN_WG_HOST_PAIR_IPERF_DURATION_SECS=$IPERF_DURATION_SECS"
    "NVPN_WG_HOST_PAIR_OUTPUT_DIR=$REMOTE_ARTIFACT_DIR"
  )
  if is_true "$preflight"; then
    envs+=("NVPN_WG_HOST_PAIR_PREFLIGHT=1")
  fi
  printf 'cd %q && export PATH=/opt/nvpn/bin:/opt/homebrew/bin:$PATH && env %s scripts/bench-userspace-wg-host-pair.sh' \
    "$LOCAL_WORK_DIR" \
    "$(quote_command "${envs[@]}")"
}

run_remote_bench() {
  if is_true "$RUN_PREFLIGHT"; then
    run remote_sh "$(bench_env_command 1)"
  fi
  if is_true "$PREFLIGHT_ONLY"; then
    return 0
  fi
  run remote_sh "$(bench_env_command 0)"
}

copy_artifacts() {
  mkdir -p "$OUTPUT_DIR"
  ssh "${LOCAL_SSH_OPTS[@]}" "$LOCAL_SSH" \
    "bash -lc $(q "cd $(q "$REMOTE_ARTIFACT_DIR") && tar --exclude='*.key' -cf - .")" \
    | tar -C "$OUTPUT_DIR" -xf -
  printf 'darwin/docker WG reference artifacts: %s\n' "$OUTPUT_DIR"
}

main() {
  if is_true "$DRY_RUN"; then
    print_dry_run
    return 0
  fi

  validate_config
  trap cleanup EXIT
  start_container
  install_container_key
  install_wireguard_go
  start_sshd
  stage_darwin_runner
  run_remote_bench
  if ! is_true "$PREFLIGHT_ONLY"; then
    copy_artifacts
  fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
