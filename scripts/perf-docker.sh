#!/usr/bin/env bash
# Throughput / latency benchmark over a 2-node nvpn+FIPS overlay mesh inside docker.
#
# Spins up node-a + node-b on a private bridge subnet (10.203.0.0/24) with
# static peer endpoints, brings the mesh up, then runs iperf3 in both
# directions over the mesh tunnel addresses. Tears down on exit.
#
# Optional contention mode:
#   NVPN_DOCKER_CPU_STRESS=1
#   NVPN_DOCKER_CPU_STRESS_SIDES=local|remote|both
#   NVPN_DOCKER_CPU_STRESS_{LOCAL,REMOTE}_WORKERS=N
#
# Optional dataplane guardrails:
#   NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP=1
#   NVPN_DOCKER_REQUIRE_NO_FSP_AEAD_HELPERS=1
#   NVPN_DOCKER_NODE_{A,B}_NOSTR_{SECRET_KEY,PUBLIC_KEY}=...
#     overrides the public Docker-only benchmark identities installed after
#     `nvpn init`; values must be single-token nsec/npub or hex strings.
#   NVPN_DOCKER_NODE_{A,B}_ID=...
#     overrides the public Docker-only benchmark node IDs. FSP owner placement
#     can still vary with runtime transport/session assignment; keep placement
#     guards on.
#   NVPN_DOCKER_PLACEMENT_PREFLIGHT_MODE=tcp|ping
#     defaults to a short TCP preflight when placement guards are enabled. TCP
#     produces a real bulk stream, so placement assertions do not depend on
#     low-rate ping counters appearing in a trace interval.
#
# Dataplane profiles:
#   NVPN_DOCKER_DATAPLANE_PROFILE=linux-vnet-lan
#     expands to the LAN MTU daemon env used for peak Linux Docker dataplane
#     measurements. Linux vnet TUN is the canonical FIPS TUN path.
#   NVPN_DOCKER_PERF_PHASES=tcp-8
#     records a host `perf record` sample for named phases into raw/perf/.
#     Requires passwordless sudo for perf on hosts with restrictive
#     perf_event_paranoid settings. Perf sampling can perturb throughput, so
#     keep profiled rows out of scorecard comparisons.
#   NVPN_DOCKER_LOADED_PING_PHASES=tcp-8
#     records ping latency while selected iperf phases are running, writing
#     raw/nvpn-loaded-ping-*.txt and raw/nvpn-loaded-ping-summary.tsv.
#   NVPN_DOCKER_SETUP_PING_ATTEMPTS=N
#   NVPN_DOCKER_SETUP_PING_WAIT_SECS=N
#     retries the initial tunnel ping readiness check while recording every
#     attempt in raw/nvpn-setup-ping-attempts.log.
#   NVPN_DOCKER_PLACEMENT_PROFILE=worker-open
#     pins the protocol-neutral direct-peer same-owner FSP local-open expectation
#     by defaulting NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT=worker-open and
#     exclusive placement guards. The topology is the current FIPS default, so
#     this profile no longer injects retired FIPS placement envs. Set
#     NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE=0 for diagnostics.
#   NVPN_DOCKER_EXTRA_ENV="NAME=value ..." still passes explicit env into
#     `nvpn connect`; setting connect-only env outside those knobs is rejected.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"
docker_bench_apply_local_fips_patch_default
PROJECT_NAME="${PROJECT_NAME:-nvpn-perf}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-perf"
DURATION="${DURATION:-10}"
IPERF_INTERVAL_SECS="${NVPN_DOCKER_IPERF_INTERVAL_SECS:-0}"
IPERF_TIMEOUT_SECS="${NVPN_DOCKER_IPERF_TIMEOUT_SECS:-$((DURATION + 30))}"
NVPN_DOCKER_IPERF_TIMEOUT_SECS="$IPERF_TIMEOUT_SECS"
IPERF_SOCKET_BUFFER="${NVPN_DOCKER_IPERF_SOCKET_BUFFER:-}"
UDP1000_PARALLEL="${NVPN_DOCKER_UDP1000_PARALLEL:-}"
UDP1000_BANDWIDTH="${NVPN_DOCKER_UDP1000_BANDWIDTH:-1G}"
SKIP_BUILD="${NVPN_DOCKER_SKIP_BUILD:-0}"
OUTPUT_DIR="${NVPN_DOCKER_OUTPUT_DIR:-$ROOT_DIR/artifacts/nvpn-docker/$(date -u +%Y%m%dT%H%M%SZ)}"
RAW_DIR="$OUTPUT_DIR/raw"
SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
PIPELINE_PHASE_RANGES="$RAW_DIR/nvpn-pipeline-phase-ranges.tsv"
PIPELINE_PHASE_SUMMARY="$RAW_DIR/nvpn-pipeline-phase-summary.tsv"
DAEMON_CPU_PHASES="$RAW_DIR/nvpn-daemon-cpu-phases.tsv"
LOADED_PING_SUMMARY="$RAW_DIR/nvpn-loaded-ping-summary.tsv"
PIPELINE_TRACE="${NVPN_DOCKER_PIPELINE_TRACE:-0}"
PIPELINE_INTERVAL_SECS="${NVPN_DOCKER_PIPELINE_INTERVAL_SECS:-5}"
PERF_PHASES="${NVPN_DOCKER_PERF_PHASES:-}"
PERF_FREQ="${NVPN_DOCKER_PERF_FREQ:-19}"
LOADED_PING_PHASES="${NVPN_DOCKER_LOADED_PING_PHASES:-}"
LOADED_PING_INTERVAL_SECS="${NVPN_DOCKER_LOADED_PING_INTERVAL_SECS:-0.01}"
SETUP_PING_ATTEMPTS="${NVPN_DOCKER_SETUP_PING_ATTEMPTS:-8}"
SETUP_PING_WAIT_SECS="${NVPN_DOCKER_SETUP_PING_WAIT_SECS:-1}"
EXTRA_CONNECT_ENV=""
REQUIRE_NO_DIRECT_FMP="${NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP:-0}"
REQUIRE_NO_FSP_AEAD_HELPERS="${NVPN_DOCKER_REQUIRE_NO_FSP_AEAD_HELPERS:-0}"
EXPECT_FSP_OWNER_PLACEMENT="${NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT:-}"
EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE="${NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE:-0}"
EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE_PROVIDED=0
if [[ -n "${NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE+x}" ]]; then
  EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE_PROVIDED=1
fi
MAX_FSP_OWNER_PLACEMENT_OTHER_PATH_RATE="${NVPN_DOCKER_MAX_FSP_OWNER_PLACEMENT_OTHER_PATH_RATE:-0}"
PLACEMENT_PROFILE="${NVPN_DOCKER_PLACEMENT_PROFILE:-}"
PLACEMENT_PREFLIGHT="${NVPN_DOCKER_PLACEMENT_PREFLIGHT:-}"
PLACEMENT_PREFLIGHT_MODE="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_MODE:-tcp}"
PLACEMENT_PREFLIGHT_DURATION="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_DURATION:-3}"
PLACEMENT_PREFLIGHT_STREAMS="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_STREAMS:-4}"
PLACEMENT_PREFLIGHT_PING_COUNT="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_PING_COUNT:-120}"
PLACEMENT_PREFLIGHT_PING_SIZE="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_PING_SIZE:-1200}"
PLACEMENT_PREFLIGHT_WAIT_SECS="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_WAIT_SECS:-$((PIPELINE_INTERVAL_SECS + 1))}"
DEFAULT_NODE_A_NOSTR_SECRET_KEY="f55a70f3b3c2dd82bc7004118a246602cafa581cffced63e74676f0730c49478"
DEFAULT_NODE_A_NOSTR_PUBLIC_KEY="60661f4fd74e2274ee536d3ef0ba2261dcd2448e9b0de5fcb0b27fecb76cc3ed"
DEFAULT_NODE_B_NOSTR_SECRET_KEY="8e7a15bb8430bbd511b66340483f99177d6ea5e9a7bcf9d8f0d934747838d2d6"
DEFAULT_NODE_B_NOSTR_PUBLIC_KEY="1cd7d884c2b66c224eb13bcb83f17e0dab90adc28092df28b90c3073df5794ed"
DEFAULT_NODE_A_ID="11111111-1111-4111-8111-111111111111"
DEFAULT_NODE_B_ID="22222222-2222-4222-8222-222222222222"
NOSTR_IDENTITY_SOURCE="docker-default"
NODE_ID_SOURCE="docker-default"
if [[ -n "${NVPN_DOCKER_NODE_A_NOSTR_SECRET_KEY:-}${NVPN_DOCKER_NODE_A_NOSTR_PUBLIC_KEY:-}${NVPN_DOCKER_NODE_B_NOSTR_SECRET_KEY:-}${NVPN_DOCKER_NODE_B_NOSTR_PUBLIC_KEY:-}" ]]; then
  NOSTR_IDENTITY_SOURCE="custom"
fi
if [[ -n "${NVPN_DOCKER_NODE_A_ID:-}${NVPN_DOCKER_NODE_B_ID:-}" ]]; then
  NODE_ID_SOURCE="custom"
fi
NODE_A_NOSTR_SECRET_KEY="${NVPN_DOCKER_NODE_A_NOSTR_SECRET_KEY:-$DEFAULT_NODE_A_NOSTR_SECRET_KEY}"
NODE_A_NOSTR_PUBLIC_KEY="${NVPN_DOCKER_NODE_A_NOSTR_PUBLIC_KEY:-$DEFAULT_NODE_A_NOSTR_PUBLIC_KEY}"
NODE_B_NOSTR_SECRET_KEY="${NVPN_DOCKER_NODE_B_NOSTR_SECRET_KEY:-$DEFAULT_NODE_B_NOSTR_SECRET_KEY}"
NODE_B_NOSTR_PUBLIC_KEY="${NVPN_DOCKER_NODE_B_NOSTR_PUBLIC_KEY:-$DEFAULT_NODE_B_NOSTR_PUBLIC_KEY}"
NODE_A_ID="${NVPN_DOCKER_NODE_A_ID:-$DEFAULT_NODE_A_ID}"
NODE_B_ID="${NVPN_DOCKER_NODE_B_ID:-$DEFAULT_NODE_B_ID}"
NVPN_DOCKER_NOSTR_IDENTITY_SOURCE="$NOSTR_IDENTITY_SOURCE"
NVPN_DOCKER_NODE_ID_SOURCE="$NODE_ID_SOURCE"
NVPN_DOCKER_NODE_A_NOSTR_PUBLIC_KEY_EFFECTIVE="$NODE_A_NOSTR_PUBLIC_KEY"
NVPN_DOCKER_NODE_B_NOSTR_PUBLIC_KEY_EFFECTIVE="$NODE_B_NOSTR_PUBLIC_KEY"
NVPN_DOCKER_NODE_A_ID_EFFECTIVE="$NODE_A_ID"
NVPN_DOCKER_NODE_B_ID_EFFECTIVE="$NODE_B_ID"
NVPN_DOCKER_PIPELINE_TRACE="$PIPELINE_TRACE"
NVPN_DOCKER_PIPELINE_INTERVAL_SECS="$PIPELINE_INTERVAL_SECS"
DIAGNOSTICS_READY=0
DIAGNOSTICS_CAPTURED=0
PIPELINE_START_NODE_A=0
PIPELINE_START_NODE_B=0
COMPOSE_TOUCHED=0

if [[ -n "$IPERF_SOCKET_BUFFER" && ! "$IPERF_SOCKET_BUFFER" =~ ^[0-9]+([KMG])?$ ]]; then
  echo "perf: invalid NVPN_DOCKER_IPERF_SOCKET_BUFFER=$IPERF_SOCKET_BUFFER (expected bytes or K/M/G suffix)" >&2
  exit 2
fi
if [[ -n "$UDP1000_PARALLEL" && ! "$UDP1000_PARALLEL" =~ ^[1-9][0-9]*$ ]]; then
  echo "perf: invalid NVPN_DOCKER_UDP1000_PARALLEL=$UDP1000_PARALLEL (expected positive integer)" >&2
  exit 2
fi
case "$PLACEMENT_PREFLIGHT_MODE" in
  tcp | ping) ;;
  *)
    echo "perf: invalid NVPN_DOCKER_PLACEMENT_PREFLIGHT_MODE=$PLACEMENT_PREFLIGHT_MODE (expected tcp or ping)" >&2
    exit 2
    ;;
esac
if [[ ! "$PLACEMENT_PREFLIGHT_DURATION" =~ ^[1-9][0-9]*$ ]]; then
  echo "perf: invalid NVPN_DOCKER_PLACEMENT_PREFLIGHT_DURATION=$PLACEMENT_PREFLIGHT_DURATION (expected positive integer seconds)" >&2
  exit 2
fi
if [[ ! "$PLACEMENT_PREFLIGHT_STREAMS" =~ ^[1-9][0-9]*$ ]]; then
  echo "perf: invalid NVPN_DOCKER_PLACEMENT_PREFLIGHT_STREAMS=$PLACEMENT_PREFLIGHT_STREAMS (expected positive integer)" >&2
  exit 2
fi
if [[ ! "$MAX_FSP_OWNER_PLACEMENT_OTHER_PATH_RATE" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  echo "perf: invalid NVPN_DOCKER_MAX_FSP_OWNER_PLACEMENT_OTHER_PATH_RATE=$MAX_FSP_OWNER_PLACEMENT_OTHER_PATH_RATE (expected non-negative number)" >&2
  exit 2
fi
if [[ ! "$UDP1000_BANDWIDTH" =~ ^[0-9]+([KMG])?$ ]]; then
  echo "perf: invalid NVPN_DOCKER_UDP1000_BANDWIDTH=$UDP1000_BANDWIDTH (expected bits/sec or K/M/G suffix)" >&2
  exit 2
fi
if [[ -n "$LOADED_PING_PHASES" ]]; then
  if [[ ! "$LOADED_PING_INTERVAL_SECS" =~ ^[0-9]+([.][0-9]+)?$ ]] \
    || ! awk -v interval="$LOADED_PING_INTERVAL_SECS" 'BEGIN { exit(interval > 0 ? 0 : 1) }'; then
    echo "perf: invalid NVPN_DOCKER_LOADED_PING_INTERVAL_SECS=$LOADED_PING_INTERVAL_SECS (expected positive seconds)" >&2
    exit 2
  fi
fi
if [[ ! "$SETUP_PING_ATTEMPTS" =~ ^[1-9][0-9]*$ ]]; then
  echo "perf: invalid NVPN_DOCKER_SETUP_PING_ATTEMPTS=$SETUP_PING_ATTEMPTS (expected positive integer)" >&2
  exit 2
fi
if [[ ! "$SETUP_PING_WAIT_SECS" =~ ^[0-9]+([.][0-9]+)?$ ]] \
  || ! awk -v wait_secs="$SETUP_PING_WAIT_SECS" 'BEGIN { exit(wait_secs >= 0 ? 0 : 1) }'; then
  echo "perf: invalid NVPN_DOCKER_SETUP_PING_WAIT_SECS=$SETUP_PING_WAIT_SECS (expected non-negative seconds)" >&2
  exit 2
fi
UDP1000_PER_STREAM_BANDWIDTH="$(docker_bench_udp1000_per_stream_bandwidth)"
IPERF_SOCKET_BUFFER_ARGS=()
if [[ -n "$IPERF_SOCKET_BUFFER" ]]; then
  IPERF_SOCKET_BUFFER_ARGS=(-w "$IPERF_SOCKET_BUFFER")
fi
UDP1000_PARALLEL_ARGS=()
if [[ -n "$UDP1000_PARALLEL" && "$UDP1000_PARALLEL" != "1" ]]; then
  UDP1000_PARALLEL_ARGS=(-P "$UDP1000_PARALLEL")
fi

cleanup() {
  local status=$?
  if declare -F capture_nvpn_diagnostics >/dev/null; then
    capture_nvpn_diagnostics "$status" || true
  fi
  if [[ "$COMPOSE_TOUCHED" == "1" ]] && ! is_true "${KEEP:-0}"; then
    docker_bench_stop_cpu_stress
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  fi
}

trap cleanup EXIT

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

identity_value_is_safe_token() {
  [[ "$1" =~ ^[A-Za-z0-9]+$ ]]
}

node_id_value_is_safe_token() {
  [[ "$1" =~ ^[A-Za-z0-9-]+$ ]]
}

validate_nostr_identity_env() {
  local provided=0
  local missing=0
  local name value
  for name in \
    NVPN_DOCKER_NODE_A_NOSTR_SECRET_KEY \
    NVPN_DOCKER_NODE_A_NOSTR_PUBLIC_KEY \
    NVPN_DOCKER_NODE_B_NOSTR_SECRET_KEY \
    NVPN_DOCKER_NODE_B_NOSTR_PUBLIC_KEY; do
    value="${!name:-}"
    if [[ -n "$value" ]]; then
      provided=1
      if ! identity_value_is_safe_token "$value"; then
        printf 'perf: %s must be a single alphanumeric nsec/npub or hex token\n' "$name" >&2
        return 2
      fi
    else
      missing=1
    fi
  done

  if [[ "$provided" == "1" && "$missing" == "1" ]]; then
    echo "perf: Docker Nostr identity env is all-or-none; set secret/public for both node-a and node-b" >&2
    return 2
  fi

  for value in \
    "$NODE_A_NOSTR_SECRET_KEY" \
    "$NODE_A_NOSTR_PUBLIC_KEY" \
    "$NODE_B_NOSTR_SECRET_KEY" \
    "$NODE_B_NOSTR_PUBLIC_KEY"; do
    if ! identity_value_is_safe_token "$value"; then
      echo "perf: effective Docker Nostr identity values must be single alphanumeric nsec/npub or hex tokens" >&2
      return 2
    fi
  done
}

validate_node_id_env() {
  local provided=0
  local missing=0
  local name value
  for name in NVPN_DOCKER_NODE_A_ID NVPN_DOCKER_NODE_B_ID; do
    value="${!name:-}"
    if [[ -n "$value" ]]; then
      provided=1
      if ! node_id_value_is_safe_token "$value"; then
        printf 'perf: %s must be a single alphanumeric/dash node-id token\n' "$name" >&2
        return 2
      fi
    else
      missing=1
    fi
  done

  if [[ "$provided" == "1" && "$missing" == "1" ]]; then
    echo "perf: Docker node ID env is all-or-none; set node IDs for both node-a and node-b" >&2
    return 2
  fi

  for value in "$NODE_A_ID" "$NODE_B_ID"; do
    if ! node_id_value_is_safe_token "$value"; then
      echo "perf: effective Docker node IDs must be single alphanumeric/dash tokens" >&2
      return 2
    fi
  done
}

EXTRA_CONNECT_ENV="$(docker_bench_effective_extra_env)"
if ! docker_bench_validate_connect_env_scope "$EXTRA_CONNECT_ENV"; then
  exit 2
fi

if ! validate_nostr_identity_env; then
  exit 2
fi

if ! validate_node_id_env; then
  exit 2
fi

case "$PLACEMENT_PROFILE" in
  "" | plain | default)
    ;;
  worker-open)
    case "$EXPECT_FSP_OWNER_PLACEMENT" in
      "")
        EXPECT_FSP_OWNER_PLACEMENT=worker-open
        export NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT="$EXPECT_FSP_OWNER_PLACEMENT"
        ;;
      worker-open)
        ;;
      *)
        echo "perf: NVPN_DOCKER_PLACEMENT_PROFILE=worker-open requires NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT=worker-open or unset" >&2
        exit 2
        ;;
    esac
    if [[ "$EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE_PROVIDED" != "1" ]]; then
      EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE=1
      export NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE="$EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE"
    fi
    ;;
  *)
    echo "perf: unknown NVPN_DOCKER_PLACEMENT_PROFILE=$PLACEMENT_PROFILE (known: plain, worker-open)" >&2
    exit 2
    ;;
esac

if is_true "$REQUIRE_NO_DIRECT_FMP" && docker_bench_direct_fmp_forced_enabled; then
  echo "perf: NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP=1 rejects FIPS_DIRECT_ENDPOINT_FMP_ONLY in daemon connect env" >&2
  exit 2
fi

assert_no_direct_fmp_runtime_artifacts() {
  is_true "$REQUIRE_NO_DIRECT_FMP" || return 0
  [[ -d "$RAW_DIR" ]] || return 0

  local first_hit
  first_hit="$(
    grep -R -n -E \
      'decrypt_direct_fmp_endpoint_wait|endpoint_direct_fmp_receive_dropped|DirectEndpointData' \
      "$RAW_DIR" 2>/dev/null | head -1 || true
  )"
  if [[ -n "$first_hit" ]]; then
    echo "perf: NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP=1 saw direct-FMP runtime evidence: $first_hit" >&2
    return 2
  fi
}

assert_no_fsp_aead_helper_runtime_artifacts() {
  is_true "$REQUIRE_NO_FSP_AEAD_HELPERS" || return 0
  [[ -d "$RAW_DIR" ]] || return 0

  local first_hit
  first_hit="$(
    grep -R -n -E \
      'decrypt_fsp_path_helper' \
      "$RAW_DIR" 2>/dev/null | head -1 || true
  )"
  if [[ -n "$first_hit" ]]; then
    echo "perf: NVPN_DOCKER_REQUIRE_NO_FSP_AEAD_HELPERS=1 saw FSP AEAD helper path evidence: $first_hit" >&2
    return 2
  fi
}

if ! docker_bench_validate_extra_env_assignments "$EXTRA_CONNECT_ENV"; then
  exit 2
fi

case "$EXPECT_FSP_OWNER_PLACEMENT" in
  "" | any | same | owner-same | mismatch | owner-mismatch | local | handoff | worker-open) ;;
  *)
    echo "perf: unknown NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT=$EXPECT_FSP_OWNER_PLACEMENT (known: any, same, owner-same, mismatch, owner-mismatch, local, handoff, worker-open)" >&2
    exit 2
    ;;
esac

if [[ -z "$PLACEMENT_PREFLIGHT" ]]; then
  case "$EXPECT_FSP_OWNER_PLACEMENT" in
    "" | any) PLACEMENT_PREFLIGHT=0 ;;
    *) PLACEMENT_PREFLIGHT=1 ;;
  esac
fi
export NVPN_DOCKER_PLACEMENT_PREFLIGHT="$PLACEMENT_PREFLIGHT"

start_compose_services() {
  COMPOSE_TOUCHED=1
  if is_true "$SKIP_BUILD"; then
    "${COMPOSE[@]}" up -d --no-build node-a node-b >/dev/null
  else
    BUILDKIT_PROGRESS=plain "${COMPOSE[@]}" build node-a node-b
    "${COMPOSE[@]}" up -d node-a node-b >/dev/null
  fi
}

wait_for_service() {
  local service="$1"
  for _ in $(seq 1 30); do
    cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$cid" ]] && [[ "$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "perf: service '$service' did not start" >&2
  exit 1
}

phase_list_contains() {
  local list="$1"
  local phase="$2"
  [[ -n "$list" ]] || return 1
  local token
  for token in ${list//,/ }; do
    [[ "$token" == "all" || "$token" == "$phase" ]] && return 0
  done
  return 1
}

perf_phase_enabled() {
  phase_list_contains "$PERF_PHASES" "$1"
}

loaded_ping_phase_enabled() {
  phase_list_contains "$LOADED_PING_PHASES" "$1"
}

loaded_ping_count_for_duration() {
  awk -v duration="$DURATION" -v interval="$LOADED_PING_INTERVAL_SECS" '
    BEGIN {
      if (interval <= 0) interval = 1;
      count = int((duration / interval) + 0.999);
      if (count < 1) count = 1;
      print count;
    }'
}

write_loaded_ping_summary_header() {
  [[ -n "$LOADED_PING_PHASES" ]] || return 0
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    phase \
    samples \
    loss_pct \
    avg_ms \
    mdev_ms \
    p95_ms \
    p99_ms \
    max_ms \
    gt1ms \
    gt2ms \
    gt10ms >"$LOADED_PING_SUMMARY"
}

append_loaded_ping_summary() {
  local phase="$1"
  local ping_output="$2"
  [[ -s "$ping_output" ]] || return 0

  local ping_loss ping_avg ping_mdev ping_p95 ping_p99 ping_max ping_samples ping_gt1 ping_gt2 ping_gt10 ping_tail_stats
  read -r ping_loss ping_avg <<<"$(docker_bench_parse_ping_loss_avg "$ping_output")"
  ping_tail_stats="$(docker_bench_parse_ping_tail_stats "$ping_output")"
  IFS=$'\t' read -r ping_mdev ping_p95 ping_p99 ping_max ping_samples ping_gt1 ping_gt2 ping_gt10 \
    <<<"$ping_tail_stats"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$phase" \
    "$ping_samples" \
    "${ping_loss:-100}" \
    "${ping_avg:-null}" \
    "$ping_mdev" \
    "$ping_p95" \
    "$ping_p99" \
    "$ping_max" \
    "$ping_gt1" \
    "$ping_gt2" \
    "$ping_gt10" >>"$LOADED_PING_SUMMARY"
}

start_loaded_phase_ping() {
  LOADED_PHASE_PING_PID=""
  local phase="$1"
  local ping_output="$2"
  loaded_ping_phase_enabled "$phase" || return 0

  local ping_count timeout_secs
  ping_count="$(loaded_ping_count_for_duration)"
  timeout_secs="$((DURATION + 5))"
  local -a timeout_cmd=()
  if command -v timeout >/dev/null 2>&1; then
    timeout_cmd=(timeout --kill-after=2s "$timeout_secs")
  elif command -v gtimeout >/dev/null 2>&1; then
    timeout_cmd=(gtimeout --kill-after=2s "$timeout_secs")
  fi

  if ((${#timeout_cmd[@]} > 0)); then
    "${timeout_cmd[@]}" "${COMPOSE[@]}" exec -T node-a \
      ping -c "$ping_count" -i "$LOADED_PING_INTERVAL_SECS" "$BOB_TUNNEL_IP" \
      >"$ping_output" 2>&1 &
  else
    "${COMPOSE[@]}" exec -T node-a \
      ping -c "$ping_count" -i "$LOADED_PING_INTERVAL_SECS" "$BOB_TUNNEL_IP" \
      >"$ping_output" 2>&1 &
  fi
  LOADED_PHASE_PING_PID="$!"
}

finish_loaded_phase_ping() {
  local phase="$1"
  local ping_pid="$2"
  local ping_output="$3"
  [[ -n "$ping_pid" ]] || return 0
  wait "$ping_pid" || true
  append_loaded_ping_summary "$phase" "$ping_output"
}

host_pids_for_service_process() {
  local service="$1"
  local process_name="$2"
  local cid
  cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
  [[ -n "$cid" ]] || return 0
  docker top "$cid" -eo pid,comm 2>/dev/null \
    | awk -v name="$process_name" 'NR > 1 && $2 == name { print $1 }'
}

start_phase_perf() {
  PHASE_PERF_PID=""
  local phase="$1"
  perf_phase_enabled "$phase" || return 0
  mkdir -p "$RAW_DIR/perf"

  local pids=()
  local service pid
  for service in node-a node-b; do
    while IFS= read -r pid; do
      [[ -n "$pid" ]] && pids+=("$pid")
    done < <(host_pids_for_service_process "$service" nvpn)
  done
  if [[ "${#pids[@]}" == "0" ]]; then
    printf 'perf: no nvpn host PIDs found for phase %s\n' "$phase" \
      >"$RAW_DIR/perf/nvpn-$phase.log"
    return 0
  fi

  local pid_csv data_path log_path
  pid_csv="$(IFS=,; printf '%s' "${pids[*]}")"
  data_path="$RAW_DIR/perf/nvpn-$phase.data"
  log_path="$RAW_DIR/perf/nvpn-$phase.log"
  printf 'phase=%s\npids=%s\nfreq=%s\n' "$phase" "$pid_csv" "$PERF_FREQ" >"$log_path"
  sudo -n perf record -F "$PERF_FREQ" -g -p "$pid_csv" -o "$data_path" \
    -- sleep "$((DURATION + 1))" >>"$log_path" 2>&1 &
  PHASE_PERF_PID="$!"
}

finish_phase_perf() {
  local phase="$1"
  local perf_pid="$2"
  [[ -n "$perf_pid" ]] || return 0
  wait "$perf_pid" || true

  local data_path="$RAW_DIR/perf/nvpn-$phase.data"
  local report_path="$RAW_DIR/perf/nvpn-$phase-report.txt"
  local log_path="$RAW_DIR/perf/nvpn-$phase.log"
  [[ -s "$data_path" ]] || return 0
  sudo -n perf report --stdio --no-children --sort comm,dso,symbol \
    -i "$data_path" >"$report_path" 2>>"$log_path" || true
}

nostr_pubkey_from_config() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]\$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' /root/.config/nvpn/config.toml
  " | tr -d '\r"'
}

install_nostr_identity_for_service() {
  local service="$1"
  local secret_key="$2"
  local public_key="$3"
  [[ -n "$secret_key" && -n "$public_key" ]] || return 0

  local cid tmp_dir
  cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
  if [[ -z "$cid" ]]; then
    echo "perf: unable to resolve container for $service identity install" >&2
    exit 1
  fi

  tmp_dir="$(mktemp -d)"
  printf '%s' "$secret_key" >"$tmp_dir/nostr-secret-key.secret"
  printf '%s' "$public_key" >"$tmp_dir/nostr-public-key.txt"
  docker cp "$tmp_dir/nostr-secret-key.secret" "$cid:/tmp/nvpn-nostr-secret-key.secret" >/dev/null
  docker cp "$tmp_dir/nostr-public-key.txt" "$cid:/tmp/nvpn-nostr-public-key.txt" >/dev/null
  rm -rf "$tmp_dir"

  "${COMPOSE[@]}" exec -T "$service" sh -lc '
    set -eu
    config=/root/.config/nvpn/config.toml
    config_dir=/root/.config/nvpn
    secret_path="$config_dir/.config.toml.nostr-secret-key.secret"
    public_key="$(cat /tmp/nvpn-nostr-public-key.txt)"

    mkdir -p "$config_dir"
    cp /tmp/nvpn-nostr-secret-key.secret "$secret_path"
    chmod 600 "$secret_path"
    sed -i \
      -e "0,/^admins[[:space:]]*=.*/s//admins = [\"$public_key\"]/" \
      -e "/^\[nostr\]$/,/^\[/{s/^secret_key[[:space:]]*=.*/secret_key = \"stored-in-private-secret-file\"/;}" \
      -e "/^\[nostr\]$/,/^\[/{s/^public_key[[:space:]]*=.*/public_key = \"$public_key\"/;}" \
      "$config"
    grep -Fqx "admins = [\"$public_key\"]" "$config"
    grep -Fqx "secret_key = \"stored-in-private-secret-file\"" "$config"
    grep -Fqx "public_key = \"$public_key\"" "$config"
    rm -f /tmp/nvpn-nostr-secret-key.secret /tmp/nvpn-nostr-public-key.txt
  '
}

install_configured_nostr_identities() {
  install_nostr_identity_for_service node-a "$NODE_A_NOSTR_SECRET_KEY" "$NODE_A_NOSTR_PUBLIC_KEY"
  install_nostr_identity_for_service node-b "$NODE_B_NOSTR_SECRET_KEY" "$NODE_B_NOSTR_PUBLIC_KEY"
}

install_configured_node_ids() {
  [[ -n "$NODE_A_ID" && -n "$NODE_B_ID" ]] || return 0
  "${COMPOSE[@]}" exec -T node-a nvpn set --node-id "$NODE_A_ID" >/dev/null
  "${COMPOSE[@]}" exec -T node-b nvpn set --node-id "$NODE_B_ID" >/dev/null
}

write_runtime_identity_artifact() {
  jq -n \
    --arg nostr_identity_source "$NOSTR_IDENTITY_SOURCE" \
    --arg node_id_source "$NODE_ID_SOURCE" \
    --arg node_a_public_key "$ALICE_NPUB" \
    --arg node_b_public_key "$BOB_NPUB" \
    --arg node_a_node_id "$NODE_A_ID" \
    --arg node_b_node_id "$NODE_B_ID" \
    --arg node_a_tunnel_ip "$ALICE_TUNNEL_IP" \
    --arg node_b_tunnel_ip "$BOB_TUNNEL_IP" \
    '{
      nostr_identity_source: $nostr_identity_source,
      node_id_source: $node_id_source,
      node_a: {
        public_key: $node_a_public_key,
        node_id: $node_a_node_id,
        tunnel_ip: $node_a_tunnel_ip
      },
      node_b: {
        public_key: $node_b_public_key,
        node_id: $node_b_node_id,
        tunnel_ip: $node_b_tunnel_ip
      }
    }' >"$RAW_DIR/nvpn-runtime-identities.json"
}

pipeline_line_count() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "if [ -r /tmp/connect.log ]; then grep -Ec '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log || true; else printf '0\\n'; fi" \
    | tr -d '\r' \
    | awk 'NR == 1 { print $1 + 0; found = 1 } END { if (!found) print 0 }'
}

write_pipeline_phase_range_header() {
  local path="$1"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    phase \
    node_a_start \
    node_a_end \
    node_b_start \
    node_b_end >"$path"
}

write_pipeline_phase_summary_header() {
  local path="$1"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    phase \
    service \
    pipeline_line_start \
    pipeline_line_end \
    pipeline_line_count \
    load_top_queue_wait \
    peak_top_queue_wait \
    fmp_worker_batch \
    fmp_worker_dispatch_spread \
    decrypt_worker_batch \
    decrypt_worker_spread \
    decrypt_worker_turn_mix \
    fsp_worker_open_wait \
    udp_send_batch \
    nvpn_tun_read_batch \
    nvpn_mesh_send_batch \
    nvpn_mesh_recv_batch \
    nvpn_tun_write \
    nvpn_direct_endpoint \
    hard_events \
    selected_load_pipeline >"$path"
}

write_pipeline_summary_header() {
  local path="$1"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    service \
    pipeline_line_count \
    benchmark_pipeline_line_count \
    load_top_queue_wait \
    peak_top_queue_wait \
    fmp_worker_batch \
    fmp_worker_dispatch_spread \
    decrypt_worker_batch \
    decrypt_worker_spread \
    decrypt_worker_turn_mix \
    fsp_worker_open_wait \
    udp_send_batch \
    nvpn_tun_read_batch \
    nvpn_mesh_send_batch \
    nvpn_mesh_recv_batch \
    nvpn_tun_write \
    nvpn_direct_endpoint \
    hard_events \
    selected_load_pipeline >"$path"
}

capture_pipeline_for_service() {
  local service="$1"
  local start_line="$2"
  local log_path="$3"
  local summary_path="$4"
  local prefix="$RAW_DIR/nvpn-$service"
  local all_lines_path="$prefix-pipeline-lines.txt"
  local bench_lines_path="$prefix-pipeline-benchmark-lines.txt"
  local load_path="$prefix-pipeline-load-selected.txt"
  local peak_path="$prefix-pipeline-peak-wait-selected.txt"
  local all_count bench_count load_line peak_line
  local load_top peak_top fmp_batch fmp_spread decrypt_batch decrypt_spread decrypt_turn_mix fsp_worker_open_wait udp_send_batch
  local nvpn_tun_read_batch nvpn_mesh_send_batch nvpn_mesh_recv_batch nvpn_tun_write nvpn_direct_endpoint hard_events

  grep -E '^\[(pipe|nvpn-pipe) ' "$log_path" >"$all_lines_path" 2>/dev/null || true
  docker_bench_pipeline_lines_after_start_from_stdin "$start_line" <"$all_lines_path" >"$bench_lines_path"
  load_line="$(docker_bench_load_pipeline_line_from_stdin <"$bench_lines_path")"
  peak_line="$(docker_bench_peak_wait_pipeline_line_from_stdin <"$bench_lines_path")"
  printf '%s\n' "$load_line" >"$load_path"
  printf '%s\n' "$peak_line" >"$peak_path"

  all_count="$(wc -l <"$all_lines_path" | tr -d ' ')"
  bench_count="$(wc -l <"$bench_lines_path" | tr -d ' ')"
  load_top="$(docker_bench_pipeline_queue_wait_top_summary "$load_line")"
  peak_top="$(docker_bench_pipeline_queue_wait_top_summary "$peak_line")"
  fmp_batch="$(docker_bench_pipeline_fmp_worker_batch_summary "$load_line")"
  fmp_spread="$(docker_bench_pipeline_fmp_worker_dispatch_spread_summary "$load_line")"
  decrypt_batch="$(docker_bench_pipeline_decrypt_worker_batch_summary "$load_line")"
  decrypt_spread="$(docker_bench_pipeline_decrypt_worker_spread_summary "$load_line")"
  decrypt_turn_mix="$(docker_bench_pipeline_decrypt_worker_turn_mix_summary "$load_line")"
  fsp_worker_open_wait="$(docker_bench_pipeline_fsp_worker_open_wait_summary "$load_line")"
  udp_send_batch="$(docker_bench_pipeline_udp_send_batch_summary "$load_line")"
  nvpn_tun_read_batch="$(docker_bench_pipeline_nvpn_tun_read_batch_summary "$load_line")"
  nvpn_mesh_send_batch="$(docker_bench_pipeline_nvpn_mesh_send_batch_summary "$load_line")"
  nvpn_mesh_recv_batch="$(docker_bench_pipeline_nvpn_mesh_recv_batch_summary "$load_line")"
  nvpn_tun_write="$(docker_bench_pipeline_nvpn_tun_write_summary_from_stdin <"$bench_lines_path")"
  nvpn_direct_endpoint="$(docker_bench_pipeline_nvpn_direct_endpoint_summary "$load_line")"
  hard_events="$(docker_bench_pipeline_hard_event_summary_from_stdin "$start_line" <"$all_lines_path")"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$service" \
    "$all_count" \
    "$bench_count" \
    "$(docker_bench_tsv_field "$load_top")" \
    "$(docker_bench_tsv_field "$peak_top")" \
    "$(docker_bench_tsv_field "$fmp_batch")" \
    "$(docker_bench_tsv_field "$fmp_spread")" \
    "$(docker_bench_tsv_field "$decrypt_batch")" \
    "$(docker_bench_tsv_field "$decrypt_spread")" \
    "$(docker_bench_tsv_field "$decrypt_turn_mix")" \
    "$(docker_bench_tsv_field "$fsp_worker_open_wait")" \
    "$(docker_bench_tsv_field "$udp_send_batch")" \
    "$(docker_bench_tsv_field "$nvpn_tun_read_batch")" \
    "$(docker_bench_tsv_field "$nvpn_mesh_send_batch")" \
    "$(docker_bench_tsv_field "$nvpn_mesh_recv_batch")" \
    "$(docker_bench_tsv_field "$nvpn_tun_write")" \
    "$(docker_bench_tsv_field "$nvpn_direct_endpoint")" \
    "$(docker_bench_tsv_field "$hard_events")" \
    "$(docker_bench_tsv_field "$load_line")" >>"$summary_path"
}

append_pipeline_phase_range() {
  local phase="$1"
  local node_a_start="$2"
  local node_a_end="$3"
  local node_b_start="$4"
  local node_b_end="$5"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$phase" \
    "$node_a_start" \
    "$node_a_end" \
    "$node_b_start" \
    "$node_b_end" >>"$PIPELINE_PHASE_RANGES"
}

capture_pipeline_phase_for_service() {
  local phase="$1"
  local service="$2"
  local start_line="$3"
  local end_line="$4"
  local all_lines_path="$5"
  local summary_path="$6"
  local phase_lines_path="$RAW_DIR/nvpn-$service-pipeline-$phase-lines.txt"
  local phase_count load_line peak_line
  local load_top peak_top fmp_batch fmp_spread decrypt_batch decrypt_spread decrypt_turn_mix fsp_worker_open_wait udp_send_batch
  local nvpn_tun_read_batch nvpn_mesh_send_batch nvpn_mesh_recv_batch nvpn_tun_write nvpn_direct_endpoint hard_events

  docker_bench_pipeline_lines_in_range_from_stdin "$start_line" "$end_line" \
    <"$all_lines_path" >"$phase_lines_path"
  phase_count="$(wc -l <"$phase_lines_path" | tr -d ' ')"
  load_line="$(docker_bench_load_pipeline_line_from_stdin <"$phase_lines_path")"
  peak_line="$(docker_bench_peak_wait_pipeline_line_from_stdin <"$phase_lines_path")"

  load_top="$(docker_bench_pipeline_queue_wait_top_summary "$load_line")"
  peak_top="$(docker_bench_pipeline_queue_wait_top_summary "$peak_line")"
  fmp_batch="$(docker_bench_pipeline_fmp_worker_batch_summary "$load_line")"
  fmp_spread="$(docker_bench_pipeline_fmp_worker_dispatch_spread_summary "$load_line")"
  decrypt_batch="$(docker_bench_pipeline_decrypt_worker_batch_summary "$load_line")"
  decrypt_spread="$(docker_bench_pipeline_decrypt_worker_spread_summary "$load_line")"
  decrypt_turn_mix="$(docker_bench_pipeline_decrypt_worker_turn_mix_summary "$load_line")"
  fsp_worker_open_wait="$(docker_bench_pipeline_fsp_worker_open_wait_summary "$load_line")"
  udp_send_batch="$(docker_bench_pipeline_udp_send_batch_summary "$load_line")"
  nvpn_tun_read_batch="$(docker_bench_pipeline_nvpn_tun_read_batch_summary "$load_line")"
  nvpn_mesh_send_batch="$(docker_bench_pipeline_nvpn_mesh_send_batch_summary "$load_line")"
  nvpn_mesh_recv_batch="$(docker_bench_pipeline_nvpn_mesh_recv_batch_summary "$load_line")"
  nvpn_tun_write="$(docker_bench_pipeline_nvpn_tun_write_summary_from_stdin <"$phase_lines_path")"
  nvpn_direct_endpoint="$(docker_bench_pipeline_nvpn_direct_endpoint_summary "$load_line")"
  hard_events="$(docker_bench_pipeline_hard_event_summary_from_stdin "$start_line" "$end_line" <"$all_lines_path")"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$phase" \
    "$service" \
    "$start_line" \
    "$end_line" \
    "$phase_count" \
    "$(docker_bench_tsv_field "$load_top")" \
    "$(docker_bench_tsv_field "$peak_top")" \
    "$(docker_bench_tsv_field "$fmp_batch")" \
    "$(docker_bench_tsv_field "$fmp_spread")" \
    "$(docker_bench_tsv_field "$decrypt_batch")" \
    "$(docker_bench_tsv_field "$decrypt_spread")" \
    "$(docker_bench_tsv_field "$decrypt_turn_mix")" \
    "$(docker_bench_tsv_field "$fsp_worker_open_wait")" \
    "$(docker_bench_tsv_field "$udp_send_batch")" \
    "$(docker_bench_tsv_field "$nvpn_tun_read_batch")" \
    "$(docker_bench_tsv_field "$nvpn_mesh_send_batch")" \
    "$(docker_bench_tsv_field "$nvpn_mesh_recv_batch")" \
    "$(docker_bench_tsv_field "$nvpn_tun_write")" \
    "$(docker_bench_tsv_field "$nvpn_direct_endpoint")" \
    "$(docker_bench_tsv_field "$hard_events")" \
    "$(docker_bench_tsv_field "$load_line")" >>"$summary_path"
}

write_pipeline_phase_summary() {
  local ranges_path="$1"
  local summary_path="$2"
  [[ -s "$ranges_path" ]] || return 0
  write_pipeline_phase_summary_header "$summary_path"
  local phase node_a_start node_a_end node_b_start node_b_end
  while IFS=$'\t' read -r phase node_a_start node_a_end node_b_start node_b_end; do
    [[ "$phase" != "phase" ]] || continue
    capture_pipeline_phase_for_service \
      "$phase" node-a "$node_a_start" "$node_a_end" \
      "$RAW_DIR/nvpn-node-a-pipeline-lines.txt" "$summary_path"
    capture_pipeline_phase_for_service \
      "$phase" node-b "$node_b_start" "$node_b_end" \
      "$RAW_DIR/nvpn-node-b-pipeline-lines.txt" "$summary_path"
  done <"$ranges_path"
}

write_daemon_cpu_phase_header() {
  docker_bench_write_cpu_phase_header "$DAEMON_CPU_PHASES"
}

daemon_cpu_sample() {
  docker_bench_process_cpu_sample "$1" nvpn
}

append_daemon_cpu_phase_rows() {
  docker_bench_append_cpu_phase_rows "$DAEMON_CPU_PHASES" "$@"
}

capture_nvpn_diagnostics() {
  local exit_status="${1:-0}"
  [[ "$DIAGNOSTICS_READY" == "1" ]] || return 0
  [[ "$DIAGNOSTICS_CAPTURED" == "0" ]] || return 0
  DIAGNOSTICS_CAPTURED=1

  mkdir -p "$RAW_DIR"
  jq -n \
    --arg captured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg exit_status "$exit_status" \
    --arg node_a_pipeline_start "$PIPELINE_START_NODE_A" \
    --arg node_b_pipeline_start "$PIPELINE_START_NODE_B" \
    '{
      captured_at: $captured_at,
      benchmark_exit_status: ($exit_status | tonumber),
      pipeline_start_lines: {
        "node-a": ($node_a_pipeline_start | tonumber),
        "node-b": ($node_b_pipeline_start | tonumber)
      }
    }' >"$RAW_DIR/nvpn-diagnostics.json" 2>/dev/null || true

  local pipeline_summary="$RAW_DIR/nvpn-pipeline-summary.tsv"
  local placement_summary="$RAW_DIR/nvpn-fsp-owner-placement.tsv"
  write_pipeline_summary_header "$pipeline_summary"
  printf '%s\t%s\t%s\n' service placement_kind fsp_owner_placement >"$placement_summary"

  local service prefix log_path status_path stderr_path
  for service in node-a node-b; do
    prefix="$RAW_DIR/nvpn-$service"
    log_path="$prefix-connect.log"
    status_path="$prefix-status.json"
    stderr_path="$prefix-status.stderr"
    "${COMPOSE[@]}" exec -T "$service" sh -lc 'cat /tmp/connect.log 2>/dev/null || true' \
      >"$log_path" 2>/dev/null || true
    "${COMPOSE[@]}" exec -T "$service" nvpn status --json --discover-secs 0 \
      >"$status_path" 2>"$stderr_path" || true
    [[ -s "$stderr_path" ]] || rm -f "$stderr_path"
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      'ps -eo pid,ppid,stat,pcpu,pmem,comm,args 2>/dev/null | head -80 || true' \
      >"$prefix-ps.txt" 2>/dev/null || true
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      'pid="$(pgrep -x nvpn | head -1 || true)"; if [ -n "$pid" ]; then ps -L -p "$pid" -o pid,tid,psr,stat,pcpu,pmem,comm,args 2>/dev/null | awk '\''NR == 1 { print; next } $2 != $1'\'' | { IFS= read -r header && printf "%s\n" "$header"; sort -k5 -nr; } || true; fi' \
      >"$prefix-threads.txt" 2>/dev/null || true
    "${COMPOSE[@]}" exec -T "$service" sh -lc 'cat /tmp/iperf3-server.log 2>/dev/null || true' \
      >"$prefix-iperf3-server.log" 2>/dev/null || true
    [[ -s "$prefix-iperf3-server.log" ]] || rm -f "$prefix-iperf3-server.log"
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      'ip -s link show 2>/dev/null || true; printf "\n## ip -s addr\n"; ip -s addr show 2>/dev/null || true; printf "\n## routes\n"; ip route show table all 2>/dev/null || true' \
      >"$prefix-netdev.txt" 2>/dev/null || true
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      'netstat -su 2>/dev/null || true; printf "\n## ss -u\n"; ss -u -a -n -i 2>/dev/null || true; printf "\n## nstat\n"; nstat -az 2>/dev/null || true' \
      >"$prefix-udp-stats.txt" 2>/dev/null || true
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      'for key in net.core.rmem_default net.core.rmem_max net.core.wmem_default net.core.wmem_max net.ipv4.udp_mem net.ipv4.udp_rmem_min net.ipv4.udp_wmem_min; do printf "%s\t" "$key"; sysctl -n "$key" 2>/dev/null || printf "unavailable\n"; done' \
      >"$prefix-udp-receiver-limits.tsv" 2>/dev/null || true
    case "$service" in
      node-a) capture_pipeline_for_service "$service" "$PIPELINE_START_NODE_A" "$log_path" "$pipeline_summary" ;;
      node-b) capture_pipeline_for_service "$service" "$PIPELINE_START_NODE_B" "$log_path" "$pipeline_summary" ;;
    esac
    local load_line placement_kind placement_detail
    load_line="$(cat "$prefix-pipeline-load-selected.txt" 2>/dev/null || true)"
    placement_kind="$(docker_bench_pipeline_fsp_owner_placement_kind "$load_line")"
    placement_detail="$(docker_bench_pipeline_fsp_owner_placement_summary "$load_line")"
    if [[ -n "$placement_kind" || -n "$placement_detail" ]]; then
      printf '%s\t%s\t%s\n' \
        "$service" \
        "$(docker_bench_tsv_field "$placement_kind")" \
        "$(docker_bench_tsv_field "$placement_detail")" >>"$placement_summary"
    fi
  done
  write_pipeline_phase_summary "$PIPELINE_PHASE_RANGES" "$PIPELINE_PHASE_SUMMARY"
  docker_bench_write_pipeline_hard_event_totals \
    "$PIPELINE_PHASE_SUMMARY" \
    "$RAW_DIR/nvpn-pipeline-hard-event-totals.tsv"
  write_connected_udp_socket_buffer_summary
  write_linux_tun_netdev_summary
}

latest_mesh_line() {
  awk '
    /mesh: [0-9]+\/[0-9]+ peers connected/ { latest = $0 }
    END {
      gsub(/\r/, "", latest)
      print latest
    }
  '
}

wait_for_mesh_ready() {
  local attempts="$1"
  local log_path="$RAW_DIR/nvpn-setup-mesh-readiness.tsv"
  local attempt node_a_log node_b_log node_a_latest node_b_latest

  mkdir -p "$RAW_DIR"
  printf 'attempt\tnode_a_latest\tnode_b_latest\n' >"$log_path"
  for attempt in $(seq 1 "$attempts"); do
    node_a_log="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
    node_b_log="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
    node_a_latest="$(latest_mesh_line <<<"$node_a_log")"
    node_b_latest="$(latest_mesh_line <<<"$node_b_log")"
    printf '%s\t%s\t%s\n' \
      "$attempt" \
      "$(docker_bench_tsv_field "$node_a_latest")" \
      "$(docker_bench_tsv_field "$node_b_latest")" >>"$log_path"
    if [[ "$node_a_latest" == "mesh: 1/1 peers connected" \
      && "$node_b_latest" == "mesh: 1/1 peers connected" ]]; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_setup_ping() {
  local attempts="$1"
  local wait_secs="$2"
  local target_ip="$3"
  local log_path="$RAW_DIR/nvpn-setup-ping-attempts.log"
  local summary_path="$RAW_DIR/nvpn-setup-ping-summary.tsv"
  local attempt output status

  mkdir -p "$RAW_DIR"
  printf 'attempt\tstatus\n' >"$summary_path"
  : >"$log_path"

  for attempt in $(seq 1 "$attempts"); do
    {
      printf '## attempt %s %s\n' "$attempt" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
      set +e
      output="$("${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$target_ip" 2>&1)"
      status=$?
      set -e
      printf '%s\n' "$output"
      printf '\n'
    } >>"$log_path"
    printf '%s\t%s\n' "$attempt" "$status" >>"$summary_path"
    if [[ "$status" == "0" ]]; then
      return 0
    fi
    if [[ "$attempt" != "$attempts" ]] && awk -v wait_secs="$wait_secs" 'BEGIN { exit(wait_secs > 0 ? 0 : 1) }'; then
      sleep "$wait_secs"
    fi
  done

  return 1
}

write_iperf_socket_buffer_summary() {
  local output_path="$RAW_DIR/nvpn-iperf-socket-buffers.tsv"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    phase protocol streams requested_sock_bufsize actual_recv_buf actual_send_buf \
    >"$output_path"
  local phase json_path
  for phase in tcp-single tcp-4 tcp-8 udp-200 udp-1000; do
    case "$phase" in
      tcp-single) json_path="$tcp_single_json" ;;
      tcp-4) json_path="$tcp_4_json" ;;
      tcp-8) json_path="$tcp_8_json" ;;
      udp-200) json_path="$udp_200_json" ;;
      udp-1000) json_path="$udp_1000_json" ;;
    esac
    [[ -s "$json_path" ]] || continue
    jq -r --arg phase "$phase" '
      [
        $phase,
        (.start.test_start.protocol // ""),
        (.start.test_start.num_streams // ""),
        (.start.sock_bufsize // ""),
        (.start.rcvbuf_actual // ""),
        (.start.sndbuf_actual // "")
      ] | @tsv
    ' "$json_path" >>"$output_path" 2>/dev/null || true
  done
}

write_connected_udp_socket_buffer_summary() {
  local output_path="$RAW_DIR/nvpn-connected-udp-socket-buffers.tsv"
  local service log_path
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    service peer_addr requested_recv_buf actual_recv_buf requested_send_buf actual_send_buf \
    >"$output_path"
  for service in node-a node-b; do
    log_path="$RAW_DIR/nvpn-$service-connect.log"
    [[ -s "$log_path" ]] || continue
    awk -v service="$service" '
      function field(name,     pattern, value) {
        pattern = name "=[^ ]+"
        if (match($0, pattern)) {
          value = substr($0, RSTART + length(name) + 1, RLENGTH - length(name) - 1)
          gsub(/^"|"$/, "", value)
          return value
        }
        return ""
      }
      {
        gsub(/\033\[[0-9;]*m/, "")
      }
      /connected UDP socket installed/ {
        printf "%s\t%s\t%s\t%s\t%s\t%s\n",
          service,
          field("peer_addr"),
          field("requested_recv_buf"),
          field("actual_recv_buf"),
          field("requested_send_buf"),
          field("actual_send_buf")
      }
    ' "$log_path" >>"$output_path"
  done
}

write_linux_tun_netdev_summary() {
  local output_path="$RAW_DIR/nvpn-linux-tun-netdev.tsv"
  local service tunnel_ip json_path
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    service iface tx_queue_len rx_packets rx_dropped tx_packets tx_dropped \
    >"$output_path"
  for service in node-a node-b; do
    case "$service" in
      node-a) tunnel_ip="${ALICE_TUNNEL_IP:-}" ;;
      node-b) tunnel_ip="${BOB_TUNNEL_IP:-}" ;;
      *) tunnel_ip="" ;;
    esac
    [[ -n "$tunnel_ip" ]] || continue
    json_path="$RAW_DIR/nvpn-$service-netdev.json"
    "${COMPOSE[@]}" exec -T "$service" sh -lc 'ip -j -s addr show 2>/dev/null || true' \
      >"$json_path" 2>/dev/null || true
    [[ -s "$json_path" ]] || continue
    jq -r --arg service "$service" --arg tunnel_ip "$tunnel_ip" '
      .[]
      | select(any(.addr_info[]?; .local == $tunnel_ip))
      | [
          $service,
          (.ifname // ""),
          (.txqlen // ""),
          (.stats64.rx.packets // .stats.rx.packets // 0),
          (.stats64.rx.dropped // .stats.rx.dropped // 0),
          (.stats64.tx.packets // .stats.tx.packets // 0),
          (.stats64.tx.dropped // .stats.tx.dropped // 0)
        ]
      | @tsv
    ' "$json_path" >>"$output_path" 2>/dev/null || true
  done
}

node_b_fsp_owner_placement_load_line() {
  "${COMPOSE[@]}" exec -T node-b sh -lc \
    "grep -E '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log 2>/dev/null || true" \
    | docker_bench_fsp_owner_placement_line_from_stdin
}

assert_expected_fsp_owner_placement_line() {
  local expected="$1"
  local load_line="$2"
  local context="${3:-node-b FSP owner placement}"
  [[ -n "$expected" && "$expected" != "any" ]] || return 0
  if ! is_true "$PIPELINE_TRACE"; then
    echo "perf: NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT requires NVPN_DOCKER_PIPELINE_TRACE=1" >&2
    return 2
  fi

  local summary kind ok=0
  summary="$(docker_bench_pipeline_fsp_owner_placement_summary "$load_line")"
  kind="$(docker_bench_pipeline_fsp_owner_placement_kind "$load_line")"
  case "$expected" in
    same | owner-same)
      [[ "$summary" == owner=same,* ]] && ok=1
      ;;
    mismatch | owner-mismatch)
      [[ "$summary" == owner=mismatch,* ]] && ok=1
      ;;
    local | handoff | worker-open)
      [[ "$kind" == "$expected" ]] && ok=1
      ;;
  esac

  if [[ "$ok" != "1" ]]; then
    printf 'perf: expected %s %s, got kind=%s summary=%s\n' \
      "$context" "$expected" "${kind:-unknown}" "${summary:-none}" >&2
    return 2
  fi

  case "$expected" in
    local | handoff | worker-open)
      if is_true "$EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE"; then
        local other_path other_rate other max_rate
        other="$(docker_bench_pipeline_fsp_owner_placement_other_path_max "$load_line" "$expected")"
        IFS=$'\t' read -r other_path other_rate <<<"$other"
        max_rate="$MAX_FSP_OWNER_PLACEMENT_OTHER_PATH_RATE"
        if ! awk -v rate="${other_rate:-0}" -v max="$max_rate" 'BEGIN { exit(rate <= max ? 0 : 1) }'; then
          printf 'perf: expected exclusive %s %s, but alternate path %s ran at %s/s > %s/s; summary=%s\n' \
            "$context" "$expected" "${other_path:-unknown}" "${other_rate:-0}" "$max_rate" "${summary:-none}" >&2
          return 2
        fi
      fi
      ;;
  esac
}

assert_expected_fsp_owner_placement() {
  local expected="$EXPECT_FSP_OWNER_PLACEMENT"
  local load_path="$RAW_DIR/nvpn-node-b-pipeline-load-selected.txt"
  local load_line
  load_line="$(cat "$load_path" 2>/dev/null || true)"
  assert_expected_fsp_owner_placement_line "$expected" "$load_line" "node-b FSP owner placement"
}

run_placement_preflight() {
  is_true "$PLACEMENT_PREFLIGHT" || return 0
  [[ -n "$EXPECT_FSP_OWNER_PLACEMENT" && "$EXPECT_FSP_OWNER_PLACEMENT" != "any" ]] || return 0
  if ! is_true "$PIPELINE_TRACE"; then
    echo "perf: NVPN_DOCKER_PLACEMENT_PREFLIGHT requires NVPN_DOCKER_PIPELINE_TRACE=1" >&2
    return 2
  fi

  mkdir -p "$RAW_DIR"
  case "$PLACEMENT_PREFLIGHT_MODE" in
    ping)
      printf '## placement preflight (%s ping packets, %s-byte payload)\n' \
        "$PLACEMENT_PREFLIGHT_PING_COUNT" "$PLACEMENT_PREFLIGHT_PING_SIZE"
      "${COMPOSE[@]}" exec -T node-a ping -c "$PLACEMENT_PREFLIGHT_PING_COUNT" -s "$PLACEMENT_PREFLIGHT_PING_SIZE" -i 0.01 -q "$BOB_TUNNEL_IP" \
        >"$RAW_DIR/nvpn-placement-preflight-ping.txt" 2>&1
      ;;
    tcp)
      printf '## placement preflight (TCP %ss, %s streams)\n' \
        "$PLACEMENT_PREFLIGHT_DURATION" "$PLACEMENT_PREFLIGHT_STREAMS"
      start_iperf_server
      local preflight_json="$RAW_DIR/nvpn-placement-preflight-tcp.json"
      local preflight_stderr="$RAW_DIR/nvpn-placement-preflight-tcp.stderr"
      if ! "${COMPOSE[@]}" exec -T node-a \
        timeout --kill-after=5s "$((PLACEMENT_PREFLIGHT_DURATION + 15))" \
        iperf3 -c "$BOB_TUNNEL_IP" -t "$PLACEMENT_PREFLIGHT_DURATION" -i 0 -f m \
          --connect-timeout 3000 --json -P "$PLACEMENT_PREFLIGHT_STREAMS" \
        >"$preflight_json" 2>"$preflight_stderr"; then
        cat "$preflight_stderr" >&2
        cat "$preflight_json" >&2
        return 1
      fi
      if jq -e 'has("error")' "$preflight_json" >/dev/null; then
        cat "$preflight_stderr" >&2
        cat "$preflight_json" >&2
        return 1
      fi
      rm -f "$preflight_stderr"
      ;;
  esac
  sleep "$PLACEMENT_PREFLIGHT_WAIT_SECS"

  local load_line
  load_line="$(node_b_fsp_owner_placement_load_line)"
  assert_expected_fsp_owner_placement_line \
    "$EXPECT_FSP_OWNER_PLACEMENT" \
    "$load_line" \
    "node-b FSP owner placement preflight"
  printf '  placement: %s\n\n' \
    "$(docker_bench_pipeline_fsp_owner_placement_summary "$load_line")"
}

start_iperf_server() {
  "${COMPOSE[@]}" exec -T node-b sh -lc 'pkill -9 iperf3 >/dev/null 2>&1 || true; rm -f /tmp/iperf3-server.log /tmp/iperf3-server.out'
  "${COMPOSE[@]}" exec -T node-b sh -lc 'nohup iperf3 -s --logfile /tmp/iperf3-server.log >/tmp/iperf3-server.out 2>&1 &'
  sleep 1
  if ! "${COMPOSE[@]}" exec -T node-b sh -lc 'ss -ltn sport = :5201 | grep -q LISTEN'; then
    echo "perf: iperf3 server failed to start" >&2
    "${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/iperf3-server.out 2>/dev/null || true' >&2 || true
    "${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/iperf3-server.log 2>/dev/null || true' >&2 || true
    exit 1
  fi
}

cleanup
docker_bench_init_summary
write_pipeline_phase_range_header "$PIPELINE_PHASE_RANGES"
write_daemon_cpu_phase_header
write_loaded_ping_summary_header
docker_bench_write_metadata nvpn "$DURATION" "a_to_b,b_to_a"
docker_bench_validate_host_quiet perf
start_compose_services
for service in node-a node-b; do
  wait_for_service "$service"
done
docker_bench_configure_iperf_socket_buffer_limits perf "$IPERF_SOCKET_BUFFER"

"${COMPOSE[@]}" exec -T node-a nvpn init --force \
  --device "$NODE_A_NOSTR_PUBLIC_KEY" --device "$NODE_B_NOSTR_PUBLIC_KEY" >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force \
  --device "$NODE_A_NOSTR_PUBLIC_KEY" --device "$NODE_B_NOSTR_PUBLIC_KEY" >/dev/null
install_configured_nostr_identities
install_configured_node_ids
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"
if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "perf: unable to resolve node npubs from config" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" >/dev/null

ALICE_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"
NVPN_DOCKER_NODE_A_RUNTIME_PUBLIC_KEY="$ALICE_NPUB"
NVPN_DOCKER_NODE_B_RUNTIME_PUBLIC_KEY="$BOB_NPUB"
NVPN_DOCKER_NODE_A_RUNTIME_NODE_ID="$NODE_A_ID"
NVPN_DOCKER_NODE_B_RUNTIME_NODE_ID="$NODE_B_ID"
NVPN_DOCKER_NODE_A_RUNTIME_TUNNEL_IP="$ALICE_TUNNEL_IP"
NVPN_DOCKER_NODE_B_RUNTIME_TUNNEL_IP="$BOB_TUNNEL_IP"
write_runtime_identity_artifact
docker_bench_write_metadata nvpn "$DURATION" "a_to_b,b_to_a"

connect_env=""
if is_true "$PIPELINE_TRACE"; then
  connect_env="NVPN_PIPELINE_TRACE=1 NVPN_PIPELINE_INTERVAL_SECS='$PIPELINE_INTERVAL_SECS' FIPS_PERF_INTERVAL_SECS='$PIPELINE_INTERVAL_SECS'"
fi

"${COMPOSE[@]}" exec -d node-a sh -lc "$connect_env $EXTRA_CONNECT_ENV nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "$connect_env $EXTRA_CONNECT_ENV nvpn connect > /tmp/connect.log 2>&1"
DIAGNOSTICS_READY=1

if ! wait_for_mesh_ready 30; then
  echo "perf: mesh did not converge to latest 1/1 state" >&2
  exit 1
fi

if ! wait_for_setup_ping "$SETUP_PING_ATTEMPTS" "$SETUP_PING_WAIT_SECS" "$BOB_TUNNEL_IP"; then
  echo "perf: ping a->b over mesh failed" >&2
  exit 1
fi
docker_bench_assert_native_processes perf nvpn node-a node-b

echo "alice tunnel ip: $ALICE_TUNNEL_IP"
echo "bob   tunnel ip: $BOB_TUNNEL_IP"
echo
run_placement_preflight
PIPELINE_START_NODE_A="$(pipeline_line_count node-a)"
PIPELINE_START_NODE_B="$(pipeline_line_count node-b)"
docker_bench_start_cpu_stress

run_test_json() {
  local phase="$1"
  local label="$2"
  local json_path="$3"
  shift 3
  local phase_start_node_a phase_start_node_b phase_end_node_a phase_end_node_b
  local cpu_start_node_a cpu_start_node_b cpu_end_node_a cpu_end_node_b transfer_bytes
  local loaded_ping_pid="" loaded_ping_output="$RAW_DIR/nvpn-loaded-ping-$phase.txt"
  local is_udp=0
  [[ "${1:-}" == "-u" ]] && is_udp=1
  printf '## %s\n' "$label"
  start_iperf_server
  # --connect-timeout caps the 3WHS so a broken path bails out fast
  # instead of hanging on tcp_synack_retries.
  local err_path="$json_path.stderr"
  local iperf_cmd=(
    timeout --kill-after=5s "$IPERF_TIMEOUT_SECS"
    iperf3 -c "$BOB_TUNNEL_IP" -t "$DURATION" -i "$IPERF_INTERVAL_SECS" -f m
    --connect-timeout 3000 --json
  )
  if (( is_udp )) && [[ -n "$IPERF_SOCKET_BUFFER" ]]; then
    iperf_cmd+=("${IPERF_SOCKET_BUFFER_ARGS[@]}")
  fi
  iperf_cmd+=("$@")
  phase_start_node_a="$(pipeline_line_count node-a)"
  phase_start_node_b="$(pipeline_line_count node-b)"
  cpu_start_node_a="$(daemon_cpu_sample node-a)"
  cpu_start_node_b="$(daemon_cpu_sample node-b)"
  local phase_perf_pid
  start_phase_perf "$phase"
  phase_perf_pid="$PHASE_PERF_PID"
  start_loaded_phase_ping "$phase" "$loaded_ping_output"
  loaded_ping_pid="$LOADED_PHASE_PING_PID"
  if ! "${COMPOSE[@]}" exec -T node-a "${iperf_cmd[@]}" >"$json_path" 2>"$err_path"; then
    finish_phase_perf "$phase" "$phase_perf_pid"
    finish_loaded_phase_ping "$phase" "$loaded_ping_pid" "$loaded_ping_output"
    cat "$err_path" >&2
    cat "$json_path" >&2
    return 1
  fi
  finish_phase_perf "$phase" "$phase_perf_pid"
  finish_loaded_phase_ping "$phase" "$loaded_ping_pid" "$loaded_ping_output"
  cpu_end_node_a="$(daemon_cpu_sample node-a)"
  cpu_end_node_b="$(daemon_cpu_sample node-b)"
  if jq -e 'has("error")' "$json_path" >/dev/null; then
    cat "$err_path" >&2
    cat "$json_path" >&2
    return 1
  fi
  transfer_bytes="$(docker_bench_iperf_transfer_bytes "$json_path")"
  append_daemon_cpu_phase_rows \
    "$phase" \
    "$transfer_bytes" \
    "$cpu_start_node_a" \
    "$cpu_start_node_b" \
    "$cpu_end_node_a" \
    "$cpu_end_node_b"
  phase_end_node_a="$(pipeline_line_count node-a)"
  phase_end_node_b="$(pipeline_line_count node-b)"
  append_pipeline_phase_range \
    "$phase" \
    "$phase_start_node_a" \
    "$phase_end_node_a" \
    "$phase_start_node_b" \
    "$phase_end_node_b"
  rm -f "$err_path"
  printf '  receiver: %s Mbps' "$(docker_bench_iperf_mbps "$json_path")"
  if (( is_udp )); then
    printf ', loss: %s%%' "$(docker_bench_iperf_loss_pct "$json_path")"
  else
    printf ', retrans: %s' "$(docker_bench_iperf_retrans "$json_path")"
  fi
  printf '\n\n'
}

tcp_single_json="$RAW_DIR/nvpn-tcp-single.json"
tcp_4_json="$RAW_DIR/nvpn-tcp-4.json"
tcp_8_json="$RAW_DIR/nvpn-tcp-8.json"
udp_200_json="$RAW_DIR/nvpn-udp-200m.json"
udp_1000_json="$RAW_DIR/nvpn-udp-1000m.json"
tcp_single_reverse_json="$RAW_DIR/nvpn-tcp-single-b-to-a.json"
tcp_4_reverse_json="$RAW_DIR/nvpn-tcp-4-b-to-a.json"
tcp_8_reverse_json="$RAW_DIR/nvpn-tcp-8-b-to-a.json"
udp_200_reverse_json="$RAW_DIR/nvpn-udp-200m-b-to-a.json"
udp_1000_reverse_json="$RAW_DIR/nvpn-udp-1000m-b-to-a.json"
ping_output="$RAW_DIR/nvpn-ping.txt"

run_test_json tcp-single "TCP single stream (A -> B)" "$tcp_single_json"
run_test_json tcp-single-b-to-a "TCP single stream (B -> A)" "$tcp_single_reverse_json" -R
run_test_json tcp-4 "TCP 4 streams (A -> B)" "$tcp_4_json" -P 4
run_test_json tcp-4-b-to-a "TCP 4 streams (B -> A)" "$tcp_4_reverse_json" -P 4 -R
run_test_json tcp-8 "TCP 8 streams (A -> B)" "$tcp_8_json" -P 8
run_test_json tcp-8-b-to-a "TCP 8 streams (B -> A)" "$tcp_8_reverse_json" -P 8 -R
run_test_json udp-200 "UDP 200 Mbit target (A -> B)" "$udp_200_json" -u -b 200M
run_test_json udp-200-b-to-a "UDP 200 Mbit target (B -> A)" "$udp_200_reverse_json" -u -b 200M -R
if [[ ${#UDP1000_PARALLEL_ARGS[@]} -gt 0 ]]; then
  udp1000_args=(-u -b "$UDP1000_PER_STREAM_BANDWIDTH" "${UDP1000_PARALLEL_ARGS[@]}")
else
  udp1000_args=(-u -b "$UDP1000_BANDWIDTH")
fi
run_test_json udp-1000 "UDP 1000 Mbit target (A -> B)" "$udp_1000_json" "${udp1000_args[@]}"
run_test_json udp-1000-b-to-a "UDP 1000 Mbit target (B -> A)" "$udp_1000_reverse_json" "${udp1000_args[@]}" -R
write_iperf_socket_buffer_summary

printf '## ping (300 packets, 10ms apart) over mesh\n'
ping_start_node_a="$(pipeline_line_count node-a)"
ping_start_node_b="$(pipeline_line_count node-b)"
ping_cpu_start_node_a="$(daemon_cpu_sample node-a)"
ping_cpu_start_node_b="$(daemon_cpu_sample node-b)"
"${COMPOSE[@]}" exec -T node-a ping -c 300 -i 0.01 "$BOB_TUNNEL_IP" >"$ping_output" 2>&1
ping_cpu_end_node_a="$(daemon_cpu_sample node-a)"
ping_cpu_end_node_b="$(daemon_cpu_sample node-b)"
ping_end_node_a="$(pipeline_line_count node-a)"
ping_end_node_b="$(pipeline_line_count node-b)"
append_pipeline_phase_range ping "$ping_start_node_a" "$ping_end_node_a" "$ping_start_node_b" "$ping_end_node_b"
append_daemon_cpu_phase_rows \
  ping \
  "" \
  "$ping_cpu_start_node_a" \
  "$ping_cpu_start_node_b" \
  "$ping_cpu_end_node_a" \
  "$ping_cpu_end_node_b"
tail -3 "$ping_output"

docker_bench_append_summary_row \
  nvpn \
  "" \
  "$DURATION" \
  "$RAW_DIR" \
  "$tcp_single_json" \
  "$tcp_4_json" \
  "$tcp_8_json" \
  "$udp_200_json" \
  "$udp_1000_json" \
  "$ping_output" \
  "$tcp_single_reverse_json" \
  "$tcp_4_reverse_json" \
  "$tcp_8_reverse_json" \
  "$udp_200_reverse_json" \
  "$udp_1000_reverse_json"

capture_nvpn_diagnostics 0
guard_status=0
set +e
docker_bench_assert_summary_guards "$SUMMARY_TSV"
summary_guard_status=$?
docker_bench_assert_pipeline_hard_event_guards "$PIPELINE_PHASE_SUMMARY"
hard_event_guard_status=$?
docker_bench_assert_tun_drop_guards "$RAW_DIR/nvpn-linux-tun-netdev.tsv"
tun_drop_guard_status=$?
set -e
if (( summary_guard_status != 0 || hard_event_guard_status != 0 || tun_drop_guard_status != 0 )); then
  guard_status=1
fi
if (( guard_status != 0 )); then
  exit "$guard_status"
fi
assert_no_direct_fmp_runtime_artifacts
assert_no_fsp_aead_helper_runtime_artifacts
assert_expected_fsp_owner_placement
printf 'nvpn docker bench passed: wrote summary to %s\n' "$SUMMARY_TSV"
