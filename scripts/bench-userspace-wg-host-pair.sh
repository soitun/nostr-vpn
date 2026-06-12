#!/usr/bin/env bash
# Host/VM pair userspace WireGuard baseline using boringtun-cli or wireguard-go.
#
# This is intentionally environment-driven so local hostnames, usernames, IPs,
# keys, and interface choices stay out of committed files. It creates a
# temporary two-peer userspace WireGuard tunnel, records ping/iperf/wg/process
# artifacts, then tears the tunnel down.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

REMOTE_SSH="${NVPN_WG_HOST_PAIR_SSH:-${1:-}}"
REMOTE_SSH_PORT="${NVPN_WG_HOST_PAIR_SSH_PORT:-}"
REMOTE_SSH_CONNECT_TIMEOUT="${NVPN_WG_HOST_PAIR_SSH_CONNECT_TIMEOUT:-10}"
REMOTE_SSH_IDENTITY_FILE="${NVPN_WG_HOST_PAIR_SSH_IDENTITY_FILE:-}"
REMOTE_SSH_KNOWN_HOSTS_FILE="${NVPN_WG_HOST_PAIR_SSH_KNOWN_HOSTS_FILE:-}"
REMOTE_SSH_STRICT_HOST_KEY_CHECKING="${NVPN_WG_HOST_PAIR_SSH_STRICT_HOST_KEY_CHECKING:-accept-new}"
REMOTE_OS="${NVPN_WG_HOST_PAIR_REMOTE_OS:-auto}"
BACKEND="${NVPN_WG_HOST_PAIR_BACKEND:-boringtun}"
case "$BACKEND" in
  boringtun) DEFAULT_BACKEND_BIN="boringtun-cli" ;;
  wireguard-go) DEFAULT_BACKEND_BIN="wireguard-go" ;;
  *) DEFAULT_BACKEND_BIN="$BACKEND" ;;
esac
LOCAL_BACKEND_BIN="${NVPN_WG_HOST_PAIR_LOCAL_BACKEND_BIN:-$DEFAULT_BACKEND_BIN}"
REMOTE_BACKEND_BIN="${NVPN_WG_HOST_PAIR_REMOTE_BACKEND_BIN:-$DEFAULT_BACKEND_BIN}"
REMOTE_WG_BIN="${NVPN_WG_HOST_PAIR_REMOTE_WG_BIN:-wg}"
LOCAL_IFACE_REQUEST="${NVPN_WG_HOST_PAIR_LOCAL_IFACE:-utun}"
REMOTE_IFACE_REQUEST="${NVPN_WG_HOST_PAIR_REMOTE_IFACE:-}"
LOCAL_UNDERLAY_IP="${NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP:-}"
REMOTE_UNDERLAY_IP="${NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP:-}"
LOCAL_LISTEN_PORT="${NVPN_WG_HOST_PAIR_LOCAL_LISTEN_PORT:-51871}"
REMOTE_LISTEN_PORT="${NVPN_WG_HOST_PAIR_REMOTE_LISTEN_PORT:-51871}"
LOCAL_TUNNEL_IP="${NVPN_WG_HOST_PAIR_LOCAL_TUNNEL_IP:-10.44.77.1}"
REMOTE_TUNNEL_IP="${NVPN_WG_HOST_PAIR_REMOTE_TUNNEL_IP:-10.44.77.2}"
WG_MTU="${NVPN_WG_HOST_PAIR_MTU:-1420}"
WG_THREADS="${NVPN_WG_HOST_PAIR_THREADS:-1}"
PING_COUNT="${NVPN_WG_HOST_PAIR_PING_COUNT:-100}"
PING_INTERVAL="${NVPN_WG_HOST_PAIR_PING_INTERVAL:-0.01}"
IPERF_DURATION="${NVPN_WG_HOST_PAIR_IPERF_DURATION_SECS:-10}"
OUTPUT_DIR="${NVPN_WG_HOST_PAIR_OUTPUT_DIR:-$ROOT_DIR/artifacts/userspace-wg-host-pair/$(date -u +%Y%m%dT%H%M%SZ)}"
REMOTE_WORK_DIR="${NVPN_WG_HOST_PAIR_REMOTE_WORK_DIR:-/tmp/nvpn-userspace-wg-host-pair}"
LOCAL_INTERACTIVE_SUDO="${NVPN_WG_HOST_PAIR_INTERACTIVE_SUDO:-0}"
DEFAULT_LOCAL_PRIV_HELPER="${NVPN_WG_HOST_PAIR_DEFAULT_LOCAL_PRIV_HELPER:-/opt/nvpn/bin/nvpn-wg-host-pair-priv-helper}"
REMOTE_PRIV_HELPER="${NVPN_WG_HOST_PAIR_REMOTE_PRIV_HELPER:-/opt/nvpn/bin/nvpn-wg-host-pair-priv-helper}"
LOCAL_PRIV_HELPER="${NVPN_WG_HOST_PAIR_LOCAL_PRIV_HELPER:-}"
if [[ -z "$LOCAL_PRIV_HELPER" && -x "$DEFAULT_LOCAL_PRIV_HELPER" ]]; then
  LOCAL_PRIV_HELPER="$DEFAULT_LOCAL_PRIV_HELPER"
fi
ASSUME_LOCAL_BACKEND_TUN="${NVPN_WG_HOST_PAIR_ASSUME_LOCAL_BACKEND_TUN:-0}"
KEEP="${NVPN_WG_HOST_PAIR_KEEP:-0}"
PREFLIGHT="${NVPN_WG_HOST_PAIR_PREFLIGHT:-0}"
CPU_STRESS="${NVPN_WG_HOST_PAIR_CPU_STRESS:-0}"
CPU_STRESS_SIDES="${NVPN_WG_HOST_PAIR_CPU_STRESS_SIDES:-remote}"
CPU_STRESS_WORKERS="${NVPN_WG_HOST_PAIR_CPU_STRESS_WORKERS:-auto}"
CPU_STRESS_SETTLE_SECS="${NVPN_WG_HOST_PAIR_CPU_STRESS_SETTLE_SECS:-2}"

SSH_OPTS=(-o BatchMode=yes -o "ConnectTimeout=$REMOTE_SSH_CONNECT_TIMEOUT" -o "StrictHostKeyChecking=$REMOTE_SSH_STRICT_HOST_KEY_CHECKING")
if [[ -n "$REMOTE_SSH_PORT" ]]; then
  SSH_OPTS=(-p "$REMOTE_SSH_PORT" "${SSH_OPTS[@]}")
fi
if [[ -n "$REMOTE_SSH_IDENTITY_FILE" ]]; then
  SSH_OPTS=(-i "$REMOTE_SSH_IDENTITY_FILE" "${SSH_OPTS[@]}")
fi
if [[ -n "$REMOTE_SSH_KNOWN_HOSTS_FILE" ]]; then
  SSH_OPTS=(-o "UserKnownHostsFile=$REMOTE_SSH_KNOWN_HOSTS_FILE" "${SSH_OPTS[@]}")
fi

LOCAL_IFACE=""
REMOTE_IFACE=""
REMOTE_OS_RESOLVED=""
LOCAL_PID_FILE=""
REMOTE_PID_FILE="$REMOTE_WORK_DIR/backend.pid"
LOCAL_PRIV_FILE=""
LOCAL_TUN_NAME_FILE=""
REMOTE_TUN_NAME_FILE="$REMOTE_WORK_DIR/tun-name"
LOCAL_BACKEND_STATE_DIR=""
LOCAL_BACKEND_ARTIFACT_LOG=""
LOCAL_BACKEND_ARTIFACT_PID=""
LOCAL_BACKEND_ARTIFACT_TUN_NAME=""
LOCAL_CPU_STRESS_PID_FILE=""
REMOTE_CPU_STRESS_PID_FILE="$REMOTE_WORK_DIR/cpu-stress.pids"
LOCAL_CPU_STRESS_WORKERS_STARTED="0"
REMOTE_CPU_STRESS_WORKERS_STARTED="0"
REMOTE_BACKEND_STATE_DIR=""
SUMMARY=""
LOCAL_WG_BIN=""
PREFLIGHT_ROWS=()

die() {
  printf 'userspace WG host-pair bench failed: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

q() {
  printf '%q' "$1"
}

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

csv_has_token() {
  local csv="$1"
  local wanted="$2"
  local raw token
  local -a tokens
  IFS=',' read -r -a tokens <<<"$csv"
  for raw in "${tokens[@]}"; do
    token="${raw#"${raw%%[![:space:]]*}"}"
    token="${token%"${token##*[![:space:]]}"}"
    if [[ "$token" == "$wanted" ]]; then
      return 0
    fi
  done
  return 1
}

remote_sh() {
  local cmd="$1"
  ssh "${SSH_OPTS[@]}" "$REMOTE_SSH" "bash -lc $(q "$cmd")"
}

remote_priv_sh() {
  local cmd="$1"
  remote_sh "sudo -n bash -lc $(q "$cmd")"
}

remote_cmd_available() {
  local cmd="$1"
  remote_sh "case $(q "$cmd") in */*) test -x $(q "$cmd") ;; *) command -v $(q "$cmd") >/dev/null 2>&1 ;; esac" >/dev/null 2>&1
}

detect_remote_os() {
  if [[ "$REMOTE_OS" != "auto" ]]; then
    printf '%s\n' "$REMOTE_OS"
    return
  fi
  remote_sh "uname -s" | tr -d '\r'
}

remote_os() {
  [[ -n "$REMOTE_OS_RESOLVED" ]] || REMOTE_OS_RESOLVED="$(detect_remote_os)"
  printf '%s\n' "$REMOTE_OS_RESOLVED"
}

resolve_remote_iface_request() {
  [[ -n "$REMOTE_IFACE_REQUEST" ]] && return 0
  case "$(remote_os)" in
    Darwin) REMOTE_IFACE_REQUEST="utun" ;;
    *) REMOTE_IFACE_REQUEST="wgbench0" ;;
  esac
}

quote_command() {
  local arg sep=""
  for arg in "$@"; do
    printf '%s%q' "$sep" "$arg"
    sep=" "
  done
}

remote_priv_helper_cmd() {
  quote_command "$REMOTE_PRIV_HELPER" "$@"
}

run_remote_priv_helper() {
  [[ -n "$REMOTE_PRIV_HELPER" ]] || return 1
  remote_sh "sudo -n $(remote_priv_helper_cmd "$@")"
}

run_remote_priv_helper_with_stdin() {
  local input="$1"
  shift
  [[ -n "$REMOTE_PRIV_HELPER" ]] || return 1
  remote_sh "printf '%s\n' $(q "$input") | sudo -n $(remote_priv_helper_cmd "$@")"
}

remote_priv_helper_available() {
  [[ -n "$REMOTE_PRIV_HELPER" ]] || return 1
  remote_sh "test -x $(q "$REMOTE_PRIV_HELPER") && sudo -n $(q "$REMOTE_PRIV_HELPER") check" >/dev/null 2>&1
}

remote_backend_helper_available() {
  [[ -n "$REMOTE_PRIV_HELPER" ]] || return 1
  remote_sh "test -x $(q "$REMOTE_PRIV_HELPER") && sudo -n $(q "$REMOTE_PRIV_HELPER") check-backend $(q "$BACKEND") $(q "$REMOTE_BACKEND_BIN")" >/dev/null 2>&1
}

remote_backend_binary_available() {
  remote_cmd_available "$REMOTE_BACKEND_BIN" || remote_backend_helper_available
}

remote_wg_set_with_stdin() {
  local private_key="$1"
  local cmd
  shift
  cmd="$(quote_command "$REMOTE_WG_BIN" set "$@")"
  remote_sh "printf '%s\n' $(q "$private_key") | sudo -n bash -lc $(q "$cmd")"
}

run_local_priv_helper() {
  [[ -n "$LOCAL_PRIV_HELPER" ]] || return 1
  if [[ "$(id -u)" == "0" ]]; then
    "$LOCAL_PRIV_HELPER" "$@"
  else
    sudo -n "$LOCAL_PRIV_HELPER" "$@"
  fi
}

local_priv_helper_available() {
  [[ -n "$LOCAL_PRIV_HELPER" && -x "$LOCAL_PRIV_HELPER" ]] || return 1
  run_local_priv_helper check >/dev/null 2>&1
}

local_backend_helper_available() {
  [[ -n "$LOCAL_PRIV_HELPER" && -x "$LOCAL_PRIV_HELPER" ]] || return 1
  run_local_priv_helper check-backend "$BACKEND" "$LOCAL_BACKEND_BIN" >/dev/null 2>&1
}

local_backend_binary_available() {
  command -v "$LOCAL_BACKEND_BIN" >/dev/null 2>&1 || local_backend_helper_available
}

local_priv_sh() {
  local cmd="$1"
  if [[ "$(id -u)" == "0" ]]; then
    bash -lc "$cmd"
  elif sudo -n true 2>/dev/null; then
    sudo -n bash -lc "$cmd"
  elif is_true "$LOCAL_INTERACTIVE_SUDO"; then
    sudo bash -lc "$cmd"
  else
    die "local tunnel setup needs sudo for address/route changes; set NVPN_WG_HOST_PAIR_INTERACTIVE_SUDO=1 for an operator-local run"
  fi
}

have_local_privilege() {
  [[ "$(id -u)" == "0" ]] \
    || local_priv_helper_available \
    || sudo -n true 2>/dev/null \
    || is_true "$LOCAL_INTERACTIVE_SUDO"
}

local_backend_tun_available() {
  [[ "$(id -u)" == "0" ]] && return 0
  is_true "$ASSUME_LOCAL_BACKEND_TUN" && return 0
  local_backend_helper_available && return 0

  case "$(uname -s)" in
    Darwin)
      [[ "$LOCAL_IFACE_REQUEST" != utun* ]]
      ;;
    Linux)
      return 1
      ;;
    *)
      return 0
      ;;
  esac
}

require_local_backend_tun_available() {
  local_backend_tun_available && return 0
  die "local userspace WG backend needs permission to create the TUN/utun interface; install a trusted backend for the local helper, run as root, or set" \
    "NVPN_WG_HOST_PAIR_ASSUME_LOCAL_BACKEND_TUN=1 only on hosts where the backend is known to create TUNs unprivileged"
}

cleanup_local_priv_sh() {
  local cmd="$1"
  if [[ "$(id -u)" == "0" ]]; then
    bash -lc "$cmd"
  elif sudo -n true 2>/dev/null; then
    sudo -n bash -lc "$cmd"
  elif is_true "$LOCAL_INTERACTIVE_SUDO"; then
    sudo bash -lc "$cmd"
  else
    return 0
  fi
}

backend_is_supported() {
  case "$BACKEND" in
    boringtun|wireguard-go) return 0 ;;
    *) return 1 ;;
  esac
}

validate_backend() {
  backend_is_supported || die "unsupported NVPN_WG_HOST_PAIR_BACKEND=$BACKEND; expected boringtun or wireguard-go"
}

preflight_result=0

preflight_ok() {
  printf '[ok] %s\n' "$1"
  PREFLIGHT_ROWS+=("ok"$'\t'"$1")
}

preflight_missing() {
  printf '[missing] %s\n' "$1"
  PREFLIGHT_ROWS+=("missing"$'\t'"$1")
  preflight_result=1
}

preflight_cmd() {
  local label="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    preflight_ok "$label"
  else
    preflight_missing "$label"
  fi
}

run_remote_preflight() {
  local labels=(
    "remote SSH is reachable"
    "remote OS is supported"
    "remote backend binary is available"
    "remote wg is available"
    "remote iperf3 is available"
    "remote sudo is passwordless"
    "remote tunnel setup is available"
  )

  if [[ -z "$REMOTE_SSH" ]]; then
    local label
    for label in "${labels[@]}"; do
      preflight_missing "$label"
    done
    return
  fi

  local cmd out status label
  cmd="
remote_os=\"\$(uname -s)\"
cmd_available() {
  case \"\$1\" in
    */*) test -x \"\$1\" ;;
    *) command -v \"\$1\" >/dev/null 2>&1 ;;
  esac
}
backend_available() {
  cmd_available $(q "$REMOTE_BACKEND_BIN") && return 0
  [ \"\$remote_os\" = Darwin ] || return 1
  test -x $(q "$REMOTE_PRIV_HELPER") \
    && sudo -n $(q "$REMOTE_PRIV_HELPER") check-backend $(q "$BACKEND") $(q "$REMOTE_BACKEND_BIN") >/dev/null 2>&1
}
check() {
  label=\"\$1\"
  shift
  if \"\$@\" >/dev/null 2>&1; then
    printf 'ok\t%s\n' \"\$label\"
  else
    printf 'missing\t%s\n' \"\$label\"
  fi
}
check 'remote SSH is reachable' true
case \"\$remote_os\" in
  Linux|Darwin) printf 'ok\tremote OS is supported\n' ;;
  *) printf 'missing\tremote OS is supported\n' ;;
esac
check 'remote backend binary is available' backend_available
check 'remote wg is available' cmd_available $(q "$REMOTE_WG_BIN")
check 'remote iperf3 is available' command -v iperf3
check 'remote sudo is passwordless' sudo -n true
case \"\$remote_os\" in
  Linux)
    check 'remote tunnel setup is available' sh -c 'command -v ip >/dev/null 2>&1 && test -e /dev/net/tun'
    ;;
  Darwin)
    check 'remote tunnel setup is available' sh -c 'command -v ifconfig >/dev/null 2>&1 && command -v route >/dev/null 2>&1 && test -x $(q "$REMOTE_PRIV_HELPER") && sudo -n $(q "$REMOTE_PRIV_HELPER") check >/dev/null 2>&1 && sudo -n $(q "$REMOTE_PRIV_HELPER") check-backend $(q "$BACKEND") $(q "$REMOTE_BACKEND_BIN") >/dev/null 2>&1'
    ;;
  *)
    printf 'missing\tremote tunnel setup is available\n'
    ;;
esac
"

  if ! out="$(remote_sh "$cmd" 2>/dev/null)"; then
    for label in "${labels[@]}"; do
      preflight_missing "$label"
    done
    return
  fi

  while IFS=$'\t' read -r status label; do
    case "$status" in
      ok) preflight_ok "$label" ;;
      missing) preflight_missing "$label" ;;
    esac
  done <<<"$out"
}

run_preflight() {
  preflight_result=0
  PREFLIGHT_ROWS=()
  printf 'userspace WG host-pair preflight (%s)\n' "$BACKEND"

  if backend_is_supported; then
    preflight_ok "backend is supported"
  else
    preflight_missing "backend is supported"
  fi

  [[ -n "$REMOTE_SSH" ]] && preflight_ok "remote SSH target is configured" || preflight_missing "remote SSH target is configured"
  [[ -n "$LOCAL_UNDERLAY_IP" ]] && preflight_ok "local underlay IP env is configured" || preflight_missing "local underlay IP env is configured"
  [[ -n "$REMOTE_UNDERLAY_IP" ]] && preflight_ok "remote underlay IP env is configured" || preflight_missing "remote underlay IP env is configured"

  preflight_cmd "local wg is available" command -v wg
  preflight_cmd "local jq is available" command -v jq
  preflight_cmd "local ssh is available" command -v ssh
  preflight_cmd "local iperf3 is available" command -v iperf3
  if local_backend_binary_available; then
    preflight_ok "local backend binary is available"
  else
    preflight_missing "local backend binary is available"
  fi
  if [[ -n "$LOCAL_PRIV_HELPER" ]]; then
    if local_backend_helper_available; then
      preflight_ok "local privileged helper can start trusted backend"
    else
      preflight_missing "local privileged helper can start trusted backend"
    fi
  fi
  if local_backend_tun_available; then
    preflight_ok "local backend can create TUN interface"
  else
    preflight_missing "local backend can create TUN interface"
  fi

  if [[ -n "$LOCAL_PRIV_HELPER" ]]; then
    if local_priv_helper_available; then
      preflight_ok "local privileged helper is available"
    else
      preflight_missing "local privileged helper is available"
    fi
  fi

  if have_local_privilege; then
    preflight_ok "local sudo/root/helper is available for address and route setup"
  else
    preflight_missing "local sudo/root/helper is available for address and route setup"
  fi

  run_remote_preflight

  if (( preflight_result == 0 )); then
    printf 'userspace WG host-pair preflight passed\n'
  else
    printf 'userspace WG host-pair preflight found blockers\n'
  fi
  write_preflight_artifacts
  return "$preflight_result"
}

write_preflight_artifacts() {
  local preflight_path
  mkdir -p "$OUTPUT_DIR"
  preflight_path="$OUTPUT_DIR/preflight.tsv"
  printf 'status\tcheck\n' >"$preflight_path"
  printf '%s\n' "${PREFLIGHT_ROWS[@]}" >>"$preflight_path"
  printf 'userspace WG host-pair preflight wrote %s\n' "$preflight_path"
}

backend_start_command() {
  local backend="$1"
  local bin="$2"
  local iface="$3"
  local tun_name_file="$4"
  local log_path="$5"
  local pid_path="$6"
  case "$backend" in
    boringtun)
      printf 'nohup env WG_TUN_NAME_FILE=%q WG_THREADS=%q %q --foreground --disable-drop-privileges %q >%q 2>&1 & echo $! >%q' \
        "$tun_name_file" "$WG_THREADS" "$bin" "$iface" "$log_path" "$pid_path"
      ;;
    wireguard-go)
      printf 'nohup env WG_TUN_NAME_FILE=%q %q --foreground %q >%q 2>&1 & echo $! >%q' \
        "$tun_name_file" "$bin" "$iface" "$log_path" "$pid_path"
      ;;
  esac
}

sync_local_backend_helper_artifacts() {
  [[ -n "${LOCAL_BACKEND_STATE_DIR:-}" ]] || return 0
  [[ -d "$LOCAL_BACKEND_STATE_DIR" ]] || return 0
  mkdir -p "$OUTPUT_DIR"
  cp -f "$LOCAL_BACKEND_STATE_DIR/backend.log" "$LOCAL_BACKEND_ARTIFACT_LOG" 2>/dev/null || true
  cp -f "$LOCAL_BACKEND_STATE_DIR/backend.pid" "$LOCAL_BACKEND_ARTIFACT_PID" 2>/dev/null || true
  cp -f "$LOCAL_BACKEND_STATE_DIR/tun-name" "$LOCAL_BACKEND_ARTIFACT_TUN_NAME" 2>/dev/null || true
}

start_local_backend() {
  LOCAL_BACKEND_ARTIFACT_LOG="$OUTPUT_DIR/local-backend.log"
  LOCAL_BACKEND_ARTIFACT_PID="$OUTPUT_DIR/local-backend.pid"
  LOCAL_BACKEND_ARTIFACT_TUN_NAME="$OUTPUT_DIR/local-tun-name"

  if local_backend_helper_available; then
    LOCAL_BACKEND_STATE_DIR="$(run_local_priv_helper start-backend "$BACKEND" "$LOCAL_BACKEND_BIN" "$LOCAL_IFACE_REQUEST" "$WG_THREADS" | tr -d '\r')"
    [[ -n "$LOCAL_BACKEND_STATE_DIR" ]] || die "local privileged helper did not report a backend state directory"
    LOCAL_PID_FILE="$LOCAL_BACKEND_STATE_DIR/backend.pid"
    LOCAL_TUN_NAME_FILE="$LOCAL_BACKEND_STATE_DIR/tun-name"
    sync_local_backend_helper_artifacts
    return
  fi

  LOCAL_PID_FILE="$LOCAL_BACKEND_ARTIFACT_PID"
  LOCAL_TUN_NAME_FILE="$LOCAL_BACKEND_ARTIFACT_TUN_NAME"
  local local_cmd
  local_cmd="$(backend_start_command "$BACKEND" "$LOCAL_BACKEND_BIN" "$LOCAL_IFACE_REQUEST" "$LOCAL_TUN_NAME_FILE" "$LOCAL_BACKEND_ARTIFACT_LOG" "$LOCAL_PID_FILE")"
  bash -lc "$local_cmd"
}

stop_local_backend() {
  sync_local_backend_helper_artifacts
  if [[ -n "${LOCAL_BACKEND_STATE_DIR:-}" ]]; then
    run_local_priv_helper stop-backend "$LOCAL_BACKEND_STATE_DIR" >/dev/null 2>&1 || true
    LOCAL_BACKEND_STATE_DIR=""
    return
  fi
  if [[ -n "${LOCAL_PID_FILE:-}" && -f "$LOCAL_PID_FILE" ]]; then
    kill "$(cat "$LOCAL_PID_FILE")" >/dev/null 2>&1 || true
  fi
}

ping_command() {
  local target="$1"
  printf 'ping -c %q -i %q %q' "$PING_COUNT" "$PING_INTERVAL" "$target"
}

percentile_from_sorted() {
  local pct="$1"
  awk -v pct="$pct" '
    NF { values[++n] = $1 + 0 }
    END {
      if (n == 0) {
        print "null";
        exit;
      }
      idx = int((n * pct + 99) / 100);
      if (idx < 1) idx = 1;
      if (idx > n) idx = n;
      printf "%.3f", values[idx];
    }'
}

parse_ping_stats() {
  local path="$1"
  local loss avg max times p95 p99
  loss="$(awk -F',' '
    /packet loss/ {
      for (i = 1; i <= NF; i++) {
        if ($i ~ /packet loss/) {
          gsub(/[^0-9.]/, "", $i);
          print $i;
        }
      }
    }' "$path" | tail -n1)"
  avg="$(awk -F'= ' '/min\/avg\/max|round-trip/ { split($2, a, "/"); print a[2] }' "$path" | tail -n1)"
  max="$(awk -F'= ' '/min\/avg\/max|round-trip/ { split($2, a, "/"); print a[3] }' "$path" | tail -n1)"
  times="$(sed -nE 's/.*time[=<][[:space:]]*([0-9.]+).*/\1/p' "$path" | sort -n)"
  p95="$(printf '%s\n' "$times" | percentile_from_sorted 95)"
  p99="$(printf '%s\n' "$times" | percentile_from_sorted 99)"
  printf '%s %s %s %s %s\n' "${loss:-100}" "${avg:-null}" "${p95:-null}" "${p99:-null}" "${max:-null}"
}

iperf_mbps() {
  jq -er '
    [
      .end.sum_received.bits_per_second?,
      .end.sum.bits_per_second?,
      .end.sum_sent.bits_per_second?
    ]
    | map(select(type == "number"))
    | if length == 0 then empty
      elif (.[0] // 0) > 0 then .[0]
      else max
      end
    | . / 1000000
  ' "$1"
}

iperf_retransmits() {
  jq -r '(.end.sum_sent.retransmits // .end.sum.retransmits // 0)' "$1"
}

wait_for_local_iface() {
  local requested="$1"
  local name_file="$2"
  local actual=""
  for _ in $(seq 1 80); do
    if [[ -s "$name_file" ]]; then
      actual="$(cat "$name_file")"
    elif [[ "$requested" != "utun" ]]; then
      actual="$requested"
    fi
    if [[ -n "$actual" ]]; then
      case "$(uname -s)" in
        Darwin)
          if ifconfig "$actual" >/dev/null 2>&1; then
            printf '%s\n' "$actual"
            return 0
          fi
          ;;
        Linux)
          if ip link show dev "$actual" >/dev/null 2>&1; then
            printf '%s\n' "$actual"
            return 0
          fi
          ;;
      esac
    fi
    sleep 0.25
  done
  die "local userspace WG interface did not appear"
}

wait_for_remote_iface() {
  local requested="$1"
  local name_file="$2"
  local actual=""
  for _ in $(seq 1 80); do
    actual="$(remote_sh "if [ -s $(q "$name_file") ]; then cat $(q "$name_file"); else printf %s $(q "$requested"); fi" | tr -d '\r')"
    if [[ -n "$actual" ]]; then
      case "$(remote_os)" in
        Darwin)
          if remote_sh "ifconfig $(q "$actual") >/dev/null 2>&1"; then
            printf '%s\n' "$actual"
            return 0
          fi
          ;;
        Linux)
          if remote_priv_sh "ip link show dev $(q "$actual") >/dev/null 2>&1"; then
            printf '%s\n' "$actual"
            return 0
          fi
          ;;
      esac
    fi
    sleep 0.25
  done
  die "remote userspace WG interface did not appear"
}

configure_local_iface() {
  if local_priv_helper_available; then
    run_local_priv_helper configure-iface "$LOCAL_IFACE" "$LOCAL_TUNNEL_IP" "$REMOTE_TUNNEL_IP" "$WG_MTU"
    return
  fi

  case "$(uname -s)" in
    Darwin)
      local_priv_sh "ifconfig $(q "$LOCAL_IFACE") $(q "$LOCAL_TUNNEL_IP") $(q "$LOCAL_TUNNEL_IP") alias; ifconfig $(q "$LOCAL_IFACE") mtu $(q "$WG_MTU"); ifconfig $(q "$LOCAL_IFACE") up; route -q -n add -host $(q "$REMOTE_TUNNEL_IP") -interface $(q "$LOCAL_IFACE") 2>/dev/null || route -q -n change -host $(q "$REMOTE_TUNNEL_IP") -interface $(q "$LOCAL_IFACE")"
      ;;
    Linux)
      local_priv_sh "ip address replace $(q "$LOCAL_TUNNEL_IP/32") dev $(q "$LOCAL_IFACE"); ip link set mtu $(q "$WG_MTU") up dev $(q "$LOCAL_IFACE"); ip route replace $(q "$REMOTE_TUNNEL_IP/32") dev $(q "$LOCAL_IFACE")"
      ;;
    *)
      die "unsupported local OS for host-pair WG baseline: $(uname -s)"
      ;;
  esac
}

configure_remote_iface() {
  case "$(remote_os)" in
    Darwin)
      run_remote_priv_helper configure-iface "$REMOTE_IFACE" "$REMOTE_TUNNEL_IP" "$LOCAL_TUNNEL_IP" "$WG_MTU"
      ;;
    Linux)
      remote_priv_sh "ip address replace $(q "$REMOTE_TUNNEL_IP/32") dev $(q "$REMOTE_IFACE"); ip link set mtu $(q "$WG_MTU") up dev $(q "$REMOTE_IFACE"); ip route replace $(q "$LOCAL_TUNNEL_IP/32") dev $(q "$REMOTE_IFACE")"
      ;;
    *)
      die "unsupported remote OS for host-pair WG baseline: $(remote_os)"
      ;;
  esac
}

configure_local_wg_peer() {
  local remote_pub="$1"

  if local_priv_helper_available; then
    run_local_priv_helper wg-set "$LOCAL_IFACE" "$LOCAL_LISTEN_PORT" "$remote_pub" "$REMOTE_TUNNEL_IP/32" "$REMOTE_UNDERLAY_IP:$REMOTE_LISTEN_PORT" <"$LOCAL_PRIV_FILE"
  else
    local_priv_sh "$(q "$LOCAL_WG_BIN") set $(q "$LOCAL_IFACE") private-key $(q "$LOCAL_PRIV_FILE") listen-port $(q "$LOCAL_LISTEN_PORT") peer $(q "$remote_pub") allowed-ips $(q "$REMOTE_TUNNEL_IP/32") endpoint $(q "$REMOTE_UNDERLAY_IP:$REMOTE_LISTEN_PORT") persistent-keepalive 25"
  fi
}

configure_wg() {
  local local_priv="$1"
  local remote_priv="$2"
  local local_pub="$3"
  local remote_pub="$4"
  printf '%s\n' "$local_priv" >"$LOCAL_PRIV_FILE"

  configure_local_wg_peer "$remote_pub"

  case "$(remote_os)" in
    Darwin)
      run_remote_priv_helper_with_stdin "$remote_priv" wg-set "$REMOTE_IFACE" "$REMOTE_LISTEN_PORT" "$local_pub" "$LOCAL_TUNNEL_IP/32" "$LOCAL_UNDERLAY_IP:$LOCAL_LISTEN_PORT"
      ;;
    Linux)
      remote_wg_set_with_stdin "$remote_priv" "$REMOTE_IFACE" private-key /dev/stdin listen-port "$REMOTE_LISTEN_PORT" peer "$local_pub" allowed-ips "$LOCAL_TUNNEL_IP/32" endpoint "$LOCAL_UNDERLAY_IP:$LOCAL_LISTEN_PORT" persistent-keepalive 25
      ;;
    *)
      die "unsupported remote OS for host-pair WG baseline: $(remote_os)"
      ;;
  esac
}

start_backends() {
  local remote_cmd
  start_local_backend

  remote_sh "rm -rf $(q "$REMOTE_WORK_DIR"); mkdir -p $(q "$REMOTE_WORK_DIR")"
  case "$(remote_os)" in
    Darwin)
      REMOTE_BACKEND_STATE_DIR="$(run_remote_priv_helper start-backend "$BACKEND" "$REMOTE_BACKEND_BIN" "$REMOTE_IFACE_REQUEST" "$WG_THREADS" | tr -d '\r')"
      [[ -n "$REMOTE_BACKEND_STATE_DIR" ]] || die "remote privileged helper did not report a backend state directory"
      REMOTE_PID_FILE="$REMOTE_BACKEND_STATE_DIR/backend.pid"
      REMOTE_TUN_NAME_FILE="$REMOTE_BACKEND_STATE_DIR/tun-name"
      ;;
    Linux)
      remote_cmd="$(backend_start_command "$BACKEND" "$REMOTE_BACKEND_BIN" "$REMOTE_IFACE_REQUEST" "$REMOTE_TUN_NAME_FILE" "$REMOTE_WORK_DIR/remote-backend.log" "$REMOTE_PID_FILE")"
      remote_priv_sh "$remote_cmd"
      ;;
    *)
      die "unsupported remote OS for host-pair WG baseline: $(remote_os)"
      ;;
  esac

  LOCAL_IFACE="$(wait_for_local_iface "$LOCAL_IFACE_REQUEST" "$LOCAL_TUN_NAME_FILE")"
  REMOTE_IFACE="$(wait_for_remote_iface "$REMOTE_IFACE_REQUEST" "$REMOTE_TUN_NAME_FILE")"
}

cleanup_remote_side() {
  [[ -n "${REMOTE_SSH:-}" ]] || return 0
  remote_sh "pkill -9 iperf3 >/dev/null 2>&1 || true" >/dev/null 2>&1 || true
  case "${REMOTE_OS_RESOLVED:-}" in
    Darwin)
      if [[ -n "${REMOTE_IFACE:-}" ]]; then
        run_remote_priv_helper cleanup-iface "$REMOTE_IFACE" "$LOCAL_TUNNEL_IP" >/dev/null 2>&1 || true
      fi
      if [[ -n "${REMOTE_BACKEND_STATE_DIR:-}" ]]; then
        run_remote_priv_helper stop-backend "$REMOTE_BACKEND_STATE_DIR" >/dev/null 2>&1 || true
        REMOTE_BACKEND_STATE_DIR=""
      fi
      ;;
    *)
      remote_sh "if [ -f $(q "$REMOTE_PID_FILE") ]; then sudo -n kill \$(cat $(q "$REMOTE_PID_FILE")) >/dev/null 2>&1 || true; fi; sudo -n ip link del $(q "${REMOTE_IFACE:-$REMOTE_IFACE_REQUEST}") >/dev/null 2>&1 || true" >/dev/null 2>&1 || true
      ;;
  esac
}

cleanup() {
  local rc=$?
  set +e
  stop_cpu_stress
  sync_local_backend_helper_artifacts
  if [[ "$KEEP" != "1" ]]; then
    cleanup_remote_side
    stop_local_backend
    if [[ -n "${LOCAL_IFACE:-}" ]]; then
      cleanup_local_iface >/dev/null 2>&1 || true
    fi
  fi
  exit "$rc"
}

run_ping_probe() {
  local side="$1"
  local target="$2"
  local log_path="$3"
  local cmd
  cmd="$(ping_command "$target")"
  if [[ "$side" == "local" ]]; then
    bash -lc "$cmd" >"$log_path" 2>&1
  else
    remote_sh "$cmd" >"$log_path" 2>&1
  fi
  parse_ping_stats "$log_path"
}

run_iperf() {
  local label="$1"
  local json_path="$2"
  shift 2
  iperf3 -J -c "$REMOTE_TUNNEL_IP" -t "$IPERF_DURATION" -O 1 --connect-timeout 3000 "$@" \
    >"$json_path" 2>"$json_path.err"
  printf '%s %s\n' "$(iperf_mbps "$json_path")" "$(iperf_retransmits "$json_path")"
}

process_cpu() {
  local side="$1"
  local pid_path="$2"
  if [[ "$side" == "local" ]]; then
    [[ -f "$pid_path" ]] || {
      printf 'null\n'
      return
    }
    ps -o pcpu= -p "$(cat "$pid_path")" | awk 'NF { print $1 + 0; found=1 } END { if (!found) print "null" }'
  else
    remote_sh "if [ -f $(q "$pid_path") ]; then ps -o pcpu= -p \$(cat $(q "$pid_path")) | awk 'NF { print \$1 + 0; found=1 } END { if (!found) print \"null\" }'; else printf 'null\n'; fi" | tr -d '\r'
  fi
}

cpu_stress_start_cmd() {
  local workers="$1"
  local pid_path="$2"
  cat <<EOF
rm -f $(q "$pid_path")
: >$(q "$pid_path")
i=0
while [ "\$i" -lt $(q "$workers") ]; do
  (while :; do :; done) >/dev/null 2>&1 &
  echo \$! >>$(q "$pid_path")
  i=\$((i + 1))
done
EOF
}

cpu_stress_stop_cmd() {
  local pid_path="$1"
  cat <<EOF
if [ -f $(q "$pid_path") ]; then
  while IFS= read -r pid; do
    [ -n "\$pid" ] && kill "\$pid" >/dev/null 2>&1 || true
  done <$(q "$pid_path")
  rm -f $(q "$pid_path")
fi
EOF
}

bounded_cpu_count_cmd() {
  cat <<'EOF'
n="$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || printf 1)"
case "$n" in ''|*[!0-9]*) n=1 ;; esac
[ "$n" -lt 1 ] && n=1
[ "$n" -gt 4 ] && n=4
printf '%s\n' "$n"
EOF
}

cpu_stress_worker_count() {
  local side="$1"
  case "$CPU_STRESS_WORKERS" in
    auto)
      if [[ "$side" == "local" ]]; then
        bash -lc "$(bounded_cpu_count_cmd)"
      else
        remote_sh "$(bounded_cpu_count_cmd)" | tr -d '\r'
      fi
      ;;
    ''|*[!0-9]*)
      die "NVPN_WG_HOST_PAIR_CPU_STRESS_WORKERS must be a non-negative integer or auto"
      ;;
    *)
      printf '%s\n' "$CPU_STRESS_WORKERS"
      ;;
  esac
}

start_cpu_stress_side() {
  local side="$1"
  local workers pid_path
  workers="$(cpu_stress_worker_count "$side")"
  case "$workers" in ''|*[!0-9]*) die "resolved CPU stress worker count for $side is not numeric: $workers" ;; esac
  if (( workers == 0 )); then
    return
  fi

  if [[ "$side" == "local" ]]; then
    pid_path="$LOCAL_CPU_STRESS_PID_FILE"
    bash -lc "$(cpu_stress_stop_cmd "$pid_path")"
    bash -lc "$(cpu_stress_start_cmd "$workers" "$pid_path")"
    LOCAL_CPU_STRESS_WORKERS_STARTED="$workers"
  else
    pid_path="$REMOTE_CPU_STRESS_PID_FILE"
    remote_sh "$(cpu_stress_stop_cmd "$pid_path")"
    remote_sh "$(cpu_stress_start_cmd "$workers" "$pid_path")"
    REMOTE_CPU_STRESS_WORKERS_STARTED="$workers"
  fi
  printf 'started %s CPU stress worker(s) on %s\n' "$workers" "$side"
}

stop_cpu_stress() {
  if [[ -n "${LOCAL_CPU_STRESS_PID_FILE:-}" ]]; then
    bash -lc "$(cpu_stress_stop_cmd "$LOCAL_CPU_STRESS_PID_FILE")" >/dev/null 2>&1 || true
  fi
  if [[ -n "${REMOTE_SSH:-}" ]]; then
    remote_sh "$(cpu_stress_stop_cmd "$REMOTE_CPU_STRESS_PID_FILE")" >/dev/null 2>&1 || true
  fi
}

validate_cpu_stress_sides() {
  local raw side
  local selected=0
  local -a sides
  IFS=',' read -r -a sides <<<"$CPU_STRESS_SIDES"
  for raw in "${sides[@]}"; do
    side="${raw#"${raw%%[![:space:]]*}"}"
    side="${side%"${side##*[![:space:]]}"}"
    [[ -z "$side" ]] && continue
    case "$side" in
      local|remote|both) selected=$((selected + 1)) ;;
      *) die "NVPN_WG_HOST_PAIR_CPU_STRESS_SIDES must contain local, remote, or both" ;;
    esac
  done
  if (( selected == 0 )); then
    die "NVPN_WG_HOST_PAIR_CPU_STRESS_SIDES must contain local, remote, or both"
  fi
}

start_cpu_stress_if_enabled() {
  is_true "$CPU_STRESS" || return 0
  validate_cpu_stress_sides
  printf 'userspace WG host-pair CPU stress enabled: sides=%s workers=%s settle=%ss\n' \
    "$CPU_STRESS_SIDES" "$CPU_STRESS_WORKERS" "$CPU_STRESS_SETTLE_SECS"
  if csv_has_token "$CPU_STRESS_SIDES" "both" || csv_has_token "$CPU_STRESS_SIDES" "local"; then
    start_cpu_stress_side local
  fi
  if csv_has_token "$CPU_STRESS_SIDES" "both" || csv_has_token "$CPU_STRESS_SIDES" "remote"; then
    start_cpu_stress_side remote
  fi
  sleep "$CPU_STRESS_SETTLE_SECS"
}

cleanup_local_iface() {
  if local_priv_helper_available; then
    run_local_priv_helper cleanup-iface "$LOCAL_IFACE" "$REMOTE_TUNNEL_IP"
    return
  fi

  case "$(uname -s)" in
    Darwin)
      cleanup_local_priv_sh "route -q -n delete -host $(q "$REMOTE_TUNNEL_IP") >/dev/null 2>&1 || true; ifconfig $(q "$LOCAL_IFACE") down >/dev/null 2>&1 || true"
      ;;
    Linux)
      cleanup_local_priv_sh "ip link del $(q "$LOCAL_IFACE") >/dev/null 2>&1 || true"
      ;;
  esac
}

local_wg_show() {
  if local_priv_helper_available; then
    run_local_priv_helper wg-show "$LOCAL_IFACE"
  else
    local_priv_sh "$(q "$LOCAL_WG_BIN") show $(q "$LOCAL_IFACE")"
  fi
}

remote_wg_show() {
  case "$(remote_os)" in
    Darwin)
      run_remote_priv_helper wg-show "$REMOTE_IFACE"
      ;;
    Linux)
      remote_priv_sh "$(q "$REMOTE_WG_BIN") show $(q "$REMOTE_IFACE")"
      ;;
    *)
      die "unsupported remote OS for host-pair WG baseline: $(remote_os)"
      ;;
  esac
}

write_summary_header() {
  printf '%s\n' 'backend	threads	cpu_stress_enabled	cpu_stress_sides	local_cpu_stress_workers	remote_cpu_stress_workers	local_iface	remote_iface	ping_forward_loss_percent	ping_forward_avg_ms	ping_forward_p95_ms	ping_forward_p99_ms	ping_forward_max_ms	ping_reverse_loss_percent	ping_reverse_avg_ms	ping_reverse_p95_ms	ping_reverse_p99_ms	ping_reverse_max_ms	tcp_forward_mbps	tcp_forward_retrans	tcp_reverse_mbps	tcp_reverse_retrans	local_backend_cpu_percent	remote_backend_cpu_percent' >"$SUMMARY"
}

write_summary_row() {
  local ping_f_loss="$1"
  local ping_f_avg="$2"
  local ping_f_p95="$3"
  local ping_f_p99="$4"
  local ping_f_max="$5"
  local ping_r_loss="$6"
  local ping_r_avg="$7"
  local ping_r_p95="$8"
  local ping_r_p99="$9"
  local ping_r_max="${10}"
  local tcp_f_mbps="${11}"
  local tcp_f_retrans="${12}"
  local tcp_r_mbps="${13}"
  local tcp_r_retrans="${14}"
  local local_cpu="${15}"
  local remote_cpu="${16}"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$BACKEND" "$WG_THREADS" "$(is_true "$CPU_STRESS" && printf true || printf false)" \
    "$CPU_STRESS_SIDES" "$LOCAL_CPU_STRESS_WORKERS_STARTED" "$REMOTE_CPU_STRESS_WORKERS_STARTED" \
    "$LOCAL_IFACE" "$REMOTE_IFACE" \
    "$ping_f_loss" "$ping_f_avg" "$ping_f_p95" "$ping_f_p99" "$ping_f_max" \
    "$ping_r_loss" "$ping_r_avg" "$ping_r_p95" "$ping_r_p99" "$ping_r_max" \
    "$tcp_f_mbps" "$tcp_f_retrans" "$tcp_r_mbps" "$tcp_r_retrans" \
    "$local_cpu" "$remote_cpu" >>"$SUMMARY"
}

write_metadata() {
  jq -nc \
    --arg started_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg backend "$BACKEND" \
    --arg threads "$WG_THREADS" \
    --arg cpu_stress_enabled "$(is_true "$CPU_STRESS" && printf true || printf false)" \
    --arg cpu_stress_sides "$CPU_STRESS_SIDES" \
    --arg local_cpu_stress_workers "$LOCAL_CPU_STRESS_WORKERS_STARTED" \
    --arg remote_cpu_stress_workers "$REMOTE_CPU_STRESS_WORKERS_STARTED" \
    --arg local_iface "$LOCAL_IFACE" \
    --arg remote_iface "$REMOTE_IFACE" \
    --arg local_tunnel_ip "$LOCAL_TUNNEL_IP" \
    --arg remote_tunnel_ip "$REMOTE_TUNNEL_IP" \
    --arg summary "$SUMMARY" \
    '{
      started_at: $started_at,
      backend: $backend,
      threads: ($threads | tonumber? // $threads),
      cpu_stress: {
        enabled: ($cpu_stress_enabled == "true"),
        sides: $cpu_stress_sides,
        local_workers: ($local_cpu_stress_workers | tonumber),
        remote_workers: ($remote_cpu_stress_workers | tonumber)
      },
      local_iface: $local_iface,
      remote_iface: $remote_iface,
      local_tunnel_ip: $local_tunnel_ip,
      remote_tunnel_ip: $remote_tunnel_ip,
      summary: $summary
    }' >"$OUTPUT_DIR/metadata.json"
}

usage() {
  cat >&2 <<'EOF'
usage: NVPN_WG_HOST_PAIR_SSH=user@host \
       NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP=<local-ip> \
       NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP=<remote-ip> \
       scripts/bench-userspace-wg-host-pair.sh

Preflight only:
  NVPN_WG_HOST_PAIR_PREFLIGHT=1 scripts/bench-userspace-wg-host-pair.sh

Common optional env:
  NVPN_WG_HOST_PAIR_BACKEND             boringtun or wireguard-go (default: boringtun)
  NVPN_WG_HOST_PAIR_LOCAL_BACKEND_BIN   local backend binary/path
                                      helper-started local backends must resolve to a root-owned trusted boringtun-cli/wireguard-go
  NVPN_WG_HOST_PAIR_REMOTE_BACKEND_BIN  remote backend binary/path
  NVPN_WG_HOST_PAIR_REMOTE_WG_BIN       remote wg binary/path (default: wg)
  NVPN_WG_HOST_PAIR_REMOTE_OS           auto, Linux, or Darwin (default: auto)
  NVPN_WG_HOST_PAIR_SSH_IDENTITY_FILE   optional remote SSH identity file
  NVPN_WG_HOST_PAIR_SSH_KNOWN_HOSTS_FILE
                                      optional remote SSH known_hosts file
  NVPN_WG_HOST_PAIR_SSH_STRICT_HOST_KEY_CHECKING
                                      default accept-new
  NVPN_WG_HOST_PAIR_LOCAL_PRIV_HELPER   optional root-owned local helper path; defaults to /opt/nvpn/bin/nvpn-wg-host-pair-priv-helper when installed
  NVPN_WG_HOST_PAIR_REMOTE_PRIV_HELPER  remote Darwin helper path (default: /opt/nvpn/bin/nvpn-wg-host-pair-priv-helper)
  NVPN_WG_HOST_PAIR_LOCAL_IFACE         default utun on macOS
  NVPN_WG_HOST_PAIR_REMOTE_IFACE        default wgbench0 on Linux remote, utun on Darwin remote
  NVPN_WG_HOST_PAIR_THREADS             WG_THREADS for boringtun (default 1)
  NVPN_WG_HOST_PAIR_OUTPUT_DIR          artifact directory
  NVPN_WG_HOST_PAIR_INTERACTIVE_SUDO    set 1 for operator-local sudo prompt
  NVPN_WG_HOST_PAIR_ASSUME_LOCAL_BACKEND_TUN
                                      set 1 only when the local backend is known to create TUN/utun unprivileged
  NVPN_WG_HOST_PAIR_KEEP                set 1 to leave interfaces/processes up
  NVPN_WG_HOST_PAIR_PREFLIGHT           set 1 to check prerequisites only
  NVPN_WG_HOST_PAIR_CPU_STRESS          set 1 to run CPU stress during measured probes
  NVPN_WG_HOST_PAIR_CPU_STRESS_SIDES    remote, local, or both (default: remote)
  NVPN_WG_HOST_PAIR_CPU_STRESS_WORKERS  auto caps at 4, or explicit count
EOF
}

main() {
  if is_true "$PREFLIGHT"; then
    run_preflight
    exit "$?"
  fi

  [[ -n "$REMOTE_SSH" ]] || {
    usage
    exit 2
  }
  [[ -n "$LOCAL_UNDERLAY_IP" ]] || die "set NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP"
  [[ -n "$REMOTE_UNDERLAY_IP" ]] || die "set NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP"
  validate_backend
  need_cmd wg
  LOCAL_WG_BIN="$(command -v wg)"
  need_cmd jq
  need_cmd ssh
  need_cmd iperf3
  local_backend_binary_available || die "missing local backend binary or trusted helper backend: $LOCAL_BACKEND_BIN"
  require_local_backend_tun_available
  REMOTE_OS_RESOLVED="$(detect_remote_os)" || die "could not detect remote OS"
  case "$REMOTE_OS_RESOLVED" in
    Linux|Darwin) ;;
    *) die "unsupported remote OS for host-pair WG baseline: $REMOTE_OS_RESOLVED" ;;
  esac
  resolve_remote_iface_request
  remote_backend_binary_available || die "missing remote backend binary or trusted helper backend: $REMOTE_BACKEND_BIN"
  remote_cmd_available "$REMOTE_WG_BIN" || die "missing remote wg binary: $REMOTE_WG_BIN"
  remote_sh "command -v iperf3 >/dev/null 2>&1" || die "remote host needs iperf3"
  remote_sh "sudo -n true" || die "remote host needs passwordless sudo for tunnel setup"
  case "$REMOTE_OS_RESOLVED" in
    Linux)
      remote_sh "command -v ip >/dev/null 2>&1 && test -e /dev/net/tun" || die "remote Linux host needs ip and /dev/net/tun"
      ;;
    Darwin)
      remote_priv_helper_available || die "remote Darwin host needs the trusted helper for tunnel setup: $REMOTE_PRIV_HELPER"
      remote_backend_helper_available || die "remote Darwin helper cannot start trusted backend: $REMOTE_BACKEND_BIN"
      ;;
  esac
  have_local_privilege || die "local tunnel setup needs sudo for address/route changes; set NVPN_WG_HOST_PAIR_INTERACTIVE_SUDO=1 for an operator-local run"

  mkdir -p "$OUTPUT_DIR"
  SUMMARY="$OUTPUT_DIR/summary.tsv"
  LOCAL_PRIV_FILE="$OUTPUT_DIR/local.key"
  LOCAL_CPU_STRESS_PID_FILE="$OUTPUT_DIR/local-cpu-stress.pids"
  trap cleanup EXIT

  local local_priv local_pub remote_priv remote_pub
  local_priv="$(wg genkey)"
  local_pub="$(printf '%s\n' "$local_priv" | wg pubkey)"
  remote_priv="$(remote_sh "$(q "$REMOTE_WG_BIN") genkey" | tr -d '\r\n')"
  remote_pub="$(printf '%s\n' "$remote_priv" | wg pubkey)"

  start_backends
  configure_local_iface
  configure_remote_iface
  configure_wg "$local_priv" "$remote_priv" "$local_pub" "$remote_pub"

  for _ in $(seq 1 40); do
    if ping -c 1 -W 1 "$REMOTE_TUNNEL_IP" >/dev/null 2>&1; then
      break
    fi
    sleep 0.5
  done
  ping -c 1 -W 2 "$REMOTE_TUNNEL_IP" >/dev/null

  remote_sh "pkill -9 iperf3 >/dev/null 2>&1 || true; iperf3 -s -D --logfile $(q "$REMOTE_WORK_DIR/iperf3-server.log")"
  sleep 1
  start_cpu_stress_if_enabled

  local ping_f_loss ping_f_avg ping_f_p95 ping_f_p99 ping_f_max
  local ping_r_loss ping_r_avg ping_r_p95 ping_r_p99 ping_r_max
  local tcp_f_mbps tcp_f_retrans tcp_r_mbps tcp_r_retrans
  read -r ping_f_loss ping_f_avg ping_f_p95 ping_f_p99 ping_f_max \
    <<<"$(run_ping_probe local "$REMOTE_TUNNEL_IP" "$OUTPUT_DIR/ping-local-to-remote.txt")"
  read -r ping_r_loss ping_r_avg ping_r_p95 ping_r_p99 ping_r_max \
    <<<"$(run_ping_probe remote "$LOCAL_TUNNEL_IP" "$OUTPUT_DIR/ping-remote-to-local.txt")"
  read -r tcp_f_mbps tcp_f_retrans \
    <<<"$(run_iperf forward "$OUTPUT_DIR/iperf-forward.json")"
  read -r tcp_r_mbps tcp_r_retrans \
    <<<"$(run_iperf reverse "$OUTPUT_DIR/iperf-reverse.json" -R)"

  local_wg_show >"$OUTPUT_DIR/wg-local.txt"
  remote_wg_show >"$OUTPUT_DIR/wg-remote.txt"

  local local_cpu remote_cpu
  local_cpu="$(process_cpu local "$LOCAL_PID_FILE")"
  remote_cpu="$(process_cpu remote "$REMOTE_PID_FILE")"

  write_summary_header
  write_summary_row \
    "$ping_f_loss" "$ping_f_avg" "$ping_f_p95" "$ping_f_p99" "$ping_f_max" \
    "$ping_r_loss" "$ping_r_avg" "$ping_r_p95" "$ping_r_p99" "$ping_r_max" \
    "$tcp_f_mbps" "$tcp_f_retrans" "$tcp_r_mbps" "$tcp_r_retrans" \
    "$local_cpu" "$remote_cpu"
  write_metadata

  printf 'userspace WG host-pair bench passed: wrote summary to %s\n' "$SUMMARY"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
