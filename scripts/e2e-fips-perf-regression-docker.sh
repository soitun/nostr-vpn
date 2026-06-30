#!/usr/bin/env bash
# Release-gate nvpn+FIPS dataplane regression check.
#
# This is not a benchmark leaderboard. It is a conservative pass/fail guard for
# the failure modes that hurt interactive traffic: collapsed TCP throughput,
# packet loss, and ICMP/liveness packets sitting behind a saturated TCP flow for
# seconds after the direct path itself is healthy.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"
PROJECT_NAME="${PROJECT_NAME:-nostr-vpn-e2e-fips-perf}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="${NVPN_PERF_NETWORK_ID:-docker-fips-perf}"
CONFIG_PATH="/root/.config/nvpn/config.toml"
KNOWN_PERF_PHASES="unimpaired-underlay,constrained-underlay,worker-queue-pressure,rx-maintenance-fault"
PERF_PHASE_ALIASES="clean-underlay=unimpaired-underlay"
DURATION="${NVPN_PERF_DURATION_SECS:-8}"
LOAD_DURATION="${NVPN_PERF_LOAD_DURATION_SECS:-12}"
IPERF_INTERVAL_SECS="${NVPN_PERF_IPERF_INTERVAL_SECS:-1}"
if (( LOAD_DURATION > DURATION )); then
  IPERF_TIMEOUT_BASE_SECS="$LOAD_DURATION"
else
  IPERF_TIMEOUT_BASE_SECS="$DURATION"
fi
IPERF_TIMEOUT_SECS="${NVPN_DOCKER_IPERF_TIMEOUT_SECS:-$((IPERF_TIMEOUT_BASE_SECS + 30))}"
NVPN_DOCKER_IPERF_TIMEOUT_SECS="$IPERF_TIMEOUT_SECS"
NVPN_DOCKER_IPERF_INTERVAL_SECS="$IPERF_INTERVAL_SECS"
MIN_TCP_MBIT="${NVPN_PERF_MIN_TCP_MBIT:-100}"
MIN_REVERSE_TCP_MBIT="${NVPN_PERF_MIN_REVERSE_TCP_MBIT:-100}"
MAX_TCP_RETRANS="${NVPN_PERF_MAX_TCP_RETRANS:-}"
MIN_IPERF_INTERVAL_MBIT="${NVPN_PERF_MIN_IPERF_INTERVAL_MBIT:-1}"
MAX_IPERF_STALL_INTERVALS="${NVPN_PERF_MAX_IPERF_STALL_INTERVALS:-0}"
MAX_PING_LOSS_PERCENT="${NVPN_PERF_MAX_PING_LOSS_PERCENT:-2}"
MAX_PING_AVG_MS="${NVPN_PERF_MAX_PING_AVG_MS:-250}"
MAX_PING_MAX_MS="${NVPN_PERF_MAX_PING_MAX_MS:-1000}"
MAX_PING_P95_MS="${NVPN_PERF_MAX_PING_P95_MS:-$MAX_PING_MAX_MS}"
MAX_PING_P99_MS="${NVPN_PERF_MAX_PING_P99_MS:-$MAX_PING_MAX_MS}"
PING_COUNT="${NVPN_PERF_PING_COUNT:-60}"
PING_INTERVAL="${NVPN_PERF_PING_INTERVAL:-0.1}"
PERF_PHASES="${NVPN_PERF_PHASES:-$KNOWN_PERF_PHASES}"
SKIP_BUILD="${NVPN_PERF_SKIP_BUILD:-0}"
FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}"
CONSTRAINED_RATE_MBIT="${NVPN_PERF_CONSTRAINED_RATE_MBIT:-250}"
CONSTRAINED_BURST_KB="${NVPN_PERF_CONSTRAINED_BURST_KB:-32}"
CONSTRAINED_LATENCY_MS="${NVPN_PERF_CONSTRAINED_LATENCY_MS:-100}"
CONSTRAINED_MIN_TCP_MBIT="${NVPN_PERF_CONSTRAINED_MIN_TCP_MBIT:-100}"
CONSTRAINED_MIN_REVERSE_TCP_MBIT="${NVPN_PERF_CONSTRAINED_MIN_REVERSE_TCP_MBIT:-100}"
CONSTRAINED_MAX_PING_LOSS_PERCENT="${NVPN_PERF_CONSTRAINED_MAX_PING_LOSS_PERCENT:-2}"
CONSTRAINED_MAX_PING_AVG_MS="${NVPN_PERF_CONSTRAINED_MAX_PING_AVG_MS:-250}"
CONSTRAINED_MAX_PING_MAX_MS="${NVPN_PERF_CONSTRAINED_MAX_PING_MAX_MS:-1000}"
CONSTRAINED_MAX_PING_P95_MS="${NVPN_PERF_CONSTRAINED_MAX_PING_P95_MS:-$CONSTRAINED_MAX_PING_MAX_MS}"
CONSTRAINED_MAX_PING_P99_MS="${NVPN_PERF_CONSTRAINED_MAX_PING_P99_MS:-$CONSTRAINED_MAX_PING_MAX_MS}"
RX_MAINT_FAULT_MS="${NVPN_PERF_RX_MAINT_FAULT_MS:-250}"
RX_MAINT_MIN_TCP_MBIT="${NVPN_PERF_RX_MAINT_MIN_TCP_MBIT:-100}"
RX_MAINT_MIN_REVERSE_TCP_MBIT="${NVPN_PERF_RX_MAINT_MIN_REVERSE_TCP_MBIT:-100}"
RX_MAINT_MAX_PING_LOSS_PERCENT="${NVPN_PERF_RX_MAINT_MAX_PING_LOSS_PERCENT:-0}"
RX_MAINT_MAX_PING_AVG_MS="${NVPN_PERF_RX_MAINT_MAX_PING_AVG_MS:-50}"
RX_MAINT_MAX_PING_MAX_MS="${NVPN_PERF_RX_MAINT_MAX_PING_MAX_MS:-80}"
RX_MAINT_MAX_PING_P95_MS="${NVPN_PERF_RX_MAINT_MAX_PING_P95_MS:-$RX_MAINT_MAX_PING_MAX_MS}"
RX_MAINT_MAX_PING_P99_MS="${NVPN_PERF_RX_MAINT_MAX_PING_P99_MS:-$RX_MAINT_MAX_PING_MAX_MS}"
RX_MAINT_POST_MAX_PING_AVG_MS="${NVPN_PERF_RX_MAINT_POST_MAX_PING_AVG_MS:-100}"
RX_MAINT_POST_MAX_PING_MAX_MS="${NVPN_PERF_RX_MAINT_POST_MAX_PING_MAX_MS:-150}"
RX_MAINT_POST_MAX_PING_P95_MS="${NVPN_PERF_RX_MAINT_POST_MAX_PING_P95_MS:-$RX_MAINT_POST_MAX_PING_MAX_MS}"
RX_MAINT_POST_MAX_PING_P99_MS="${NVPN_PERF_RX_MAINT_POST_MAX_PING_P99_MS:-$RX_MAINT_POST_MAX_PING_MAX_MS}"
WORKER_QUEUE_PRESSURE_CAP="${NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP:-8}"
WORKER_QUEUE_PRESSURE_DECRYPT_CAP="${NVPN_PERF_DECRYPT_WORKER_QUEUE_PRESSURE_CAP:-$WORKER_QUEUE_PRESSURE_CAP}"
WORKER_QUEUE_PRESSURE_MIN_TCP_MBIT="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MIN_TCP_MBIT:-20}"
WORKER_QUEUE_PRESSURE_MIN_REVERSE_TCP_MBIT="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MIN_REVERSE_TCP_MBIT:-20}"
# This phase deliberately shrinks the encrypt-worker channel while saturating
# TCP. Hosted CI can drop short ICMP bursts under that synthetic pressure, so
# keep the during-load budget broad and rely on the tighter post-load check to
# catch lingering interactive starvation.
WORKER_QUEUE_PRESSURE_MAX_PING_LOSS_PERCENT="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MAX_PING_LOSS_PERCENT:-20}"
WORKER_QUEUE_PRESSURE_MAX_PING_AVG_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MAX_PING_AVG_MS:-100}"
WORKER_QUEUE_PRESSURE_MAX_PING_MAX_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MAX_PING_MAX_MS:-200}"
WORKER_QUEUE_PRESSURE_MAX_PING_P95_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MAX_PING_P95_MS:-$WORKER_QUEUE_PRESSURE_MAX_PING_MAX_MS}"
WORKER_QUEUE_PRESSURE_MAX_PING_P99_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MAX_PING_P99_MS:-$WORKER_QUEUE_PRESSURE_MAX_PING_MAX_MS}"
WORKER_QUEUE_PRESSURE_POST_MAX_PING_LOSS_PERCENT="${NVPN_PERF_WORKER_QUEUE_PRESSURE_POST_MAX_PING_LOSS_PERCENT:-5}"
WORKER_QUEUE_PRESSURE_POST_MAX_PING_AVG_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_POST_MAX_PING_AVG_MS:-100}"
WORKER_QUEUE_PRESSURE_POST_MAX_PING_MAX_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_POST_MAX_PING_MAX_MS:-150}"
WORKER_QUEUE_PRESSURE_POST_MAX_PING_P95_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_POST_MAX_PING_P95_MS:-$WORKER_QUEUE_PRESSURE_POST_MAX_PING_MAX_MS}"
WORKER_QUEUE_PRESSURE_POST_MAX_PING_P99_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_POST_MAX_PING_P99_MS:-$WORKER_QUEUE_PRESSURE_POST_MAX_PING_MAX_MS}"
PIPELINE_TRACE="${NVPN_PERF_PIPELINE_TRACE:-1}"
PIPELINE_INTERVAL_SECS="${NVPN_PERF_PIPELINE_INTERVAL_SECS:-5}"
FAIL_ON_PRIORITY_HARD_EVENTS="${NVPN_PERF_FAIL_ON_PRIORITY_HARD_EVENTS:-1}"
MAX_PRIORITY_QUEUE_WAIT_MS="${NVPN_PERF_MAX_PRIORITY_QUEUE_WAIT_MS:-150}"
MIN_PRIORITY_QUEUE_WAIT_RATE_PER_SEC="${NVPN_PERF_MIN_PRIORITY_QUEUE_WAIT_RATE_PER_SEC:-10}"
if [[ -n "${NVPN_PERF_RX_MAINT_MAX_PRIORITY_QUEUE_WAIT_MS+x}" ]]; then
  RX_MAINT_MAX_PRIORITY_QUEUE_WAIT_MS="$NVPN_PERF_RX_MAINT_MAX_PRIORITY_QUEUE_WAIT_MS"
elif awk -v threshold_ms="$MAX_PRIORITY_QUEUE_WAIT_MS" 'BEGIN { exit !((threshold_ms + 0) > 0) }'; then
  RX_MAINT_MAX_PRIORITY_QUEUE_WAIT_MS=150
else
  RX_MAINT_MAX_PRIORITY_QUEUE_WAIT_MS="$MAX_PRIORITY_QUEUE_WAIT_MS"
fi
EXTRA_ENV="${NVPN_PERF_EXTRA_ENV:-}"
DIRECT_COUNTER_COMMENT="nvpn-direct-underlay"
PERF_OUTPUT_DIR="${NVPN_PERF_OUTPUT_DIR:-}"
PHASE_SUMMARY=""
PHASE_NOTES=""
FAILURE_SUMMARY=""
CURRENT_PHASE=""
CURRENT_STEP=""
CURRENT_FORWARD_MBIT=""
CURRENT_FORWARD_RETRANS=""
CURRENT_REVERSE_MBIT=""
CURRENT_REVERSE_RETRANS=""
CURRENT_PROBE_MBIT=""
CURRENT_PROBE_RETRANS=""
CURRENT_PING_LOSS=""
CURRENT_PING_AVG=""
CURRENT_PING_P95=""
CURRENT_PING_P99=""
CURRENT_PING_MAX=""
CURRENT_DIRECT_BEFORE_A=""
CURRENT_DIRECT_BEFORE_B=""
CURRENT_DIRECT_DELTA_A=""
CURRENT_DIRECT_DELTA_B=""
CURRENT_PIPELINE_A=""
CURRENT_PIPELINE_B=""
CURRENT_PIPELINE_START_A="0"
CURRENT_PIPELINE_START_B="0"

# Keep the release gate aligned with CI by default: Docker builds use the
# published FIPS crates pinned by Cargo.lock unless local patching is explicit.
# Set NVPN_PATCH_LOCAL_FIPS=1 and NVPN_FIPS_REPO_PATH=../fips while developing
# cross-repo FIPS changes.

usage() {
  cat <<EOF
Usage: $(basename "$0") [--phase NAME ...] [--phases CSV]

Runs the Docker nvpn+FIPS dataplane perf regression. Use a targeted phase while
iterating on a metric, then run the default full matrix before committing.

Known phases:
  $KNOWN_PERF_PHASES
Aliases:
  $PERF_PHASE_ALIASES

Examples:
  $(basename "$0") --phase rx-maintenance-fault
  $(basename "$0") --phases unimpaired-underlay,rx-maintenance-fault
  NVPN_PERF_PHASES=worker-queue-pressure $(basename "$0")
  NVPN_PERF_SKIP_BUILD=1 $(basename "$0")
  NVPN_E2E_BUILDER_IMAGE=local-rust NVPN_E2E_RUNTIME_IMAGE=local-runtime $(basename "$0")
  NVPN_PERF_PIPELINE_INTERVAL_SECS=1 $(basename "$0") --phase unimpaired-underlay
  NVPN_PERF_MAX_TCP_RETRANS=1000 $(basename "$0") --phase unimpaired-underlay
  NVPN_PERF_MIN_IPERF_INTERVAL_MBIT=1 NVPN_PERF_MAX_IPERF_STALL_INTERVALS=0 $(basename "$0") --phase unimpaired-underlay
  NVPN_DOCKER_IPERF_TIMEOUT_SECS=20 $(basename "$0") --phase unimpaired-underlay
  NVPN_DOCKER_CPU_STRESS=1 NVPN_DOCKER_CPU_STRESS_SIDES=remote $(basename "$0") --phase unimpaired-underlay
  NVPN_PERF_FAIL_ON_PRIORITY_HARD_EVENTS=0 $(basename "$0") --phase worker-queue-pressure
  NVPN_PERF_MAX_PRIORITY_QUEUE_WAIT_MS=0 $(basename "$0") --phase unimpaired-underlay
  NVPN_PERF_MIN_PRIORITY_QUEUE_WAIT_RATE_PER_SEC=0 $(basename "$0") --phase unimpaired-underlay
  NVPN_PERF_RX_MAINT_MAX_PRIORITY_QUEUE_WAIT_MS=200 $(basename "$0") --phase rx-maintenance-fault
EOF
}

parse_args() {
  local selected=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --phase)
        if [[ $# -lt 2 || "$2" == --* ]]; then
          echo "--phase requires a phase name" >&2
          exit 2
        fi
        if [[ -n "$selected" ]]; then
          selected+=","
        fi
        selected+="$2"
        shift 2
        ;;
      --phases)
        if [[ $# -lt 2 || "$2" == --* ]]; then
          echo "--phases requires a comma-separated phase list" >&2
          exit 2
        fi
        PERF_PHASES="$2"
        selected=""
        shift 2
        ;;
      --list-phases)
        printf '%s\n' "$KNOWN_PERF_PHASES" | tr ',' '\n'
        printf 'alias: %s\n' "$PERF_PHASE_ALIASES"
        exit 0
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        echo "unknown argument: $1" >&2
        usage >&2
        exit 2
        ;;
    esac
  done
  if [[ -n "$selected" ]]; then
    PERF_PHASES="$selected"
  fi
}

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

cleanup() {
  docker_bench_stop_cpu_stress
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
}

dump_debug() {
  set +e
  echo "nvpn+FIPS perf regression e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a node-b; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 120 "$service" || true
    echo "--- $service connect.log ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "tail -n 240 /tmp/connect.log 2>/dev/null || true" || true
    echo "--- $service status ---"
    "${COMPOSE[@]}" exec -T "$service" nvpn status --json --discover-secs 0 || true
    echo "--- $service daemon.log ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "tail -n 240 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
    echo "--- $service direct underlay counters ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      "iptables-save -c 2>/dev/null | grep '$DIRECT_COUNTER_COMMENT' || true" || true
  done
}

on_exit() {
  local exit_code=$?
  if [[ $exit_code -ne 0 ]]; then
    write_host_snapshot "failure-exit"
    dump_debug
  else
    write_host_snapshot "end"
  fi
  cleanup
  exit "$exit_code"
}
wait_for_service() {
  local service="$1"
  local container_id=""
  for _ in $(seq 1 30); do
    container_id="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$container_id" ]] \
      && [[ "$(docker inspect -f '{{.State.Running}}' "$container_id" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done

  echo "nvpn+FIPS perf regression e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

nostr_pubkey_from_config() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' '$CONFIG_PATH'
  " | tr -d '\r\"'
}

wait_for_mesh() {
  for _ in $(seq 1 45); do
    local a b
    a="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
    b="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
    if grep -q "mesh: 1/1 peers connected" <<<"$a" \
      && grep -q "mesh: 1/1 peers connected" <<<"$b"; then
      return 0
    fi
    sleep 1
  done

  echo "nvpn+FIPS perf regression e2e failed: mesh did not converge to 1/1" >&2
  return 1
}

install_direct_underlay_counter() {
  local service="$1"
  local peer_ip="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- "$peer_ip" "$DIRECT_COUNTER_COMMENT" <<'SH'
set -eu
peer_ip="$1"
comment="$2"
while iptables -D OUTPUT -p udp -d "$peer_ip" --dport 51820 -m comment --comment "$comment" 2>/dev/null; do :; done
iptables -I OUTPUT 1 -p udp -d "$peer_ip" --dport 51820 -m comment --comment "$comment"
SH
}

direct_underlay_bytes() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- "$DIRECT_COUNTER_COMMENT" <<'SH' | tr -d '\r'
set -eu
comment="$1"
iptables-save -c 2>/dev/null | awk -v comment="$comment" '
  index($0, comment) {
    gsub(/^\[/, "", $1)
    gsub(/\]$/, "", $1)
    split($1, counters, ":")
    print counters[2]
    found = 1
    exit
  }
  END {
    if (!found) {
      print 0
    }
  }
'
SH
}

assert_direct_counter_advanced() {
  local service="$1"
  local before="$2"
  local label="$3"
  local after delta
  after="$(direct_underlay_bytes "$service")"
  delta=$((after - before))
  case "$service" in
    node-a)
      CURRENT_DIRECT_DELTA_A="$delta"
      ;;
    node-b)
      CURRENT_DIRECT_DELTA_B="$delta"
      ;;
  esac
  if (( delta <= 0 )); then
    append_failure_summary "$label direct UDP underlay bytes delta" ">" "$delta" "0"
    echo "nvpn+FIPS perf regression e2e failed: $label did not use the configured direct UDP underlay path (before=$before after=$after)" >&2
    exit 1
  fi
  LAST_DIRECT_DELTA="$delta"
  LAST_DIRECT_TOTAL="$after"
  printf '%s direct UDP underlay bytes: +%s (total=%s)\n' "$label" "$delta" "$after"
}

assert_float_at_least() {
  local actual="$1"
  local min="$2"
  local label="$3"
  if ! awk -v actual="$actual" -v min="$min" \
    'BEGIN { exit !((actual + 0) >= (min + 0)) }'; then
    append_failure_summary "$label" ">=" "$actual" "$min"
    printf 'nvpn+FIPS perf regression e2e failed: %s %.1f below minimum %.1f\n' \
      "$label" "$actual" "$min" >&2
    exit 1
  fi
}

assert_float_at_most() {
  local actual="$1"
  local max="$2"
  local label="$3"
  if ! awk -v actual="$actual" -v max="$max" \
    'BEGIN { exit !((actual + 0) <= (max + 0)) }'; then
    append_failure_summary "$label" "<=" "$actual" "$max"
    printf 'nvpn+FIPS perf regression e2e failed: %s %.1f above maximum %.1f\n' \
      "$label" "$actual" "$max" >&2
    exit 1
  fi
}

assert_int_at_most_if_set() {
  local actual="$1"
  local max="$2"
  local label="$3"
  [[ -z "$max" ]] && return 0
  if ! awk -v actual="$actual" -v max="$max" \
    'BEGIN { exit !((actual + 0) <= (max + 0)) }'; then
    append_failure_summary "$label" "<=" "$actual" "$max"
    printf 'nvpn+FIPS perf regression e2e failed: %s %s above maximum %s\n' \
      "$label" "$actual" "$max" >&2
    exit 1
  fi
}

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

canonical_perf_phase() {
  case "$1" in
    clean-underlay)
      printf '%s' "unimpaired-underlay"
      ;;
    *)
      printf '%s' "$1"
      ;;
  esac
}

phase_enabled() {
  local wanted="$1"
  local raw phase
  local -a raw_phases
  IFS=',' read -r -a raw_phases <<<"$PERF_PHASES"
  for raw in "${raw_phases[@]}"; do
    phase="$(canonical_perf_phase "$(trim "$raw")")"
    if [[ "$phase" == "$wanted" ]]; then
      return 0
    fi
  done
  return 1
}

validate_perf_phases() {
  local raw phase
  local selected=0
  local -a raw_phases
  IFS=',' read -r -a raw_phases <<<"$PERF_PHASES"
  for raw in "${raw_phases[@]}"; do
    phase="$(canonical_perf_phase "$(trim "$raw")")"
    [[ -z "$phase" ]] && continue
    case "$phase" in
      unimpaired-underlay|constrained-underlay|worker-queue-pressure|rx-maintenance-fault)
        selected=$((selected + 1))
        ;;
      *)
        echo "NVPN_PERF_PHASES contains unknown phase: $phase" >&2
        echo "known phases: $KNOWN_PERF_PHASES (alias: $PERF_PHASE_ALIASES)" >&2
        exit 2
        ;;
    esac
  done
  if (( selected == 0 )); then
    echo "NVPN_PERF_PHASES must include at least one known phase" >&2
    echo "known phases: $KNOWN_PERF_PHASES (alias: $PERF_PHASE_ALIASES)" >&2
    exit 2
  fi
}

iperf_mbps() {
  jq -er '((.end.sum_received.bits_per_second // .end.sum.bits_per_second // .end.sum_sent.bits_per_second) | select(type == "number")) / 1000000'
}

iperf_retransmits() {
  jq -r '(.end.sum_sent.retransmits // .end.sum.retransmits // 0)'
}

iperf_stall_interval_count() {
  local min_mbit="${1:-$MIN_IPERF_INTERVAL_MBIT}"
  jq -r --argjson min_mbit "$min_mbit" '
    [
      (.intervals // [])[]
      | select((.sum.omitted // false) | not)
      | ((.sum.bits_per_second // 0) | if type == "number" then . else 0 end) as $bps
      | select(($bps / 1000000) < $min_mbit)
    ] | length
  '
}

assert_iperf_progress_ok() {
  local label="$1"
  local json="$2"
  local stall_count
  stall_count="$(printf '%s\n' "$json" | iperf_stall_interval_count "$MIN_IPERF_INTERVAL_MBIT")"
  if ! [[ "$stall_count" =~ ^[0-9]+$ ]]; then
    echo "nvpn+FIPS perf regression e2e failed: unable to count iperf stalled intervals for $label" >&2
    exit 1
  fi
  if (( stall_count > MAX_IPERF_STALL_INTERVALS )); then
    append_failure_summary \
      "$label iperf stalled intervals below ${MIN_IPERF_INTERVAL_MBIT} Mbps" \
      "<=" \
      "$stall_count" \
      "$MAX_IPERF_STALL_INTERVALS"
    echo "nvpn+FIPS perf regression e2e failed: $label had $stall_count iperf interval(s) below ${MIN_IPERF_INTERVAL_MBIT} Mbps" >&2
    exit 1
  fi
}

assert_iperf_progress_file_ok() {
  local label="$1"
  local json_path="$2"
  assert_iperf_progress_ok "$label" "$(cat "$json_path")"
}

iperf_interval_summary() {
  jq -r '
    def numbers: map(select(type == "number"));
    def first_number: numbers | .[0] // "";
    def sum_numbers: numbers | if length == 0 then "" else add end;
    def mbps($bps):
      if ($bps | type) == "number" then
        (((($bps / 1000000) * 10) | round) / 10)
      else
        ""
      end;
    [
      "interval",
      "omitted",
      "start_sec",
      "end_sec",
      "mbps",
      "retransmits",
      "snd_cwnd_bytes",
      "rtt_us",
      "rttvar_us"
    ],
    ((.intervals // []) | to_entries[] | .key as $idx | .value as $interval |
      ($interval.sum // {}) as $sum |
      ($interval.streams // []) as $streams |
      [
        $idx,
        (($sum.omitted // false) | tostring),
        ($sum.start // ""),
        ($sum.end // ""),
        mbps($sum.bits_per_second),
        ($sum.retransmits // ([$streams[].retransmits?] | sum_numbers)),
        ([$streams[].snd_cwnd?] | first_number),
        ([$streams[].rtt?] | first_number),
        ([$streams[].rttvar?] | first_number)
      ]
    )
    | @tsv
  '
}

iperf_server_json() {
  jq -er '.server_output_json // empty | select(type == "object")'
}

iperf_sender_json() {
  jq -e '
    def sender_interval_fields:
      [
        (.intervals // [])[]
        | (.streams // [])[]
        | (.snd_cwnd?, .rtt?, .rttvar?, .retransmits?)
        | select(type == "number")
      ] | length;
    if ((.server_output_json? | type) == "object" and .server_output_json.end.sum_sent.sender == true) then
      .server_output_json
    elif (.end.sum_sent.sender == true) then
      .
    elif ((.server_output_json? | type) == "object" and ((sender_interval_fields) == 0) and ((.server_output_json | sender_interval_fields) > 0)) then
      .server_output_json
    else
      .
    end
  '
}

iperf_sender_interval_summary() {
  jq -r '
    def numbers: map(select(type == "number"));
    def sum_numbers: numbers | if length == 0 then "" else add end;
    def min_number: numbers | if length == 0 then "" else min end;
    def max_number: numbers | if length == 0 then "" else max end;
    def mbps($bps):
      if ($bps | type) == "number" then
        (((($bps / 1000000) * 10) | round) / 10)
      else
        ""
      end;
    def interval_retrans($interval):
      ($interval.sum.retransmits // ([$interval.streams[]?.retransmits?] | numbers | if length == 0 then null else add end));
    def stream_numbers($intervals; $field):
      [$intervals[] | (.streams // [])[] | .[$field]?] | numbers;
    [
      "intervals",
      "mbps_min",
      "mbps_max",
      "retransmits_total",
      "retransmits_max_interval",
      "snd_cwnd_min_bytes",
      "rtt_max_us",
      "rttvar_max_us"
    ],
    (
      [(.intervals // [])[] | select((.sum.omitted // false) | not)] as $intervals |
      [
        ($intervals | length),
        ([$intervals[].sum.bits_per_second? | select(type == "number") | mbps(.)] | min_number),
        ([$intervals[].sum.bits_per_second? | select(type == "number") | mbps(.)] | max_number),
        ([$intervals[] | interval_retrans(.)] | sum_numbers),
        ([$intervals[] | interval_retrans(.)] | max_number),
        (stream_numbers($intervals; "snd_cwnd") | min_number),
        (stream_numbers($intervals; "rtt") | max_number),
        (stream_numbers($intervals; "rttvar") | max_number)
      ]
    )
    | @tsv
  '
}

run_iperf_json() {
  local label="$1"
  shift
  local output code
  for attempt in 1 2 3; do
    if output="$("${COMPOSE[@]}" exec -T node-a timeout --kill-after=5s "$IPERF_TIMEOUT_SECS" iperf3 \
        -J --get-server-output -c "$BOB_TUNNEL_IP" -t "$DURATION" -i "$IPERF_INTERVAL_SECS" -O 1 --connect-timeout 3000 "$@" 2>&1)"; then
      code=0
      if printf '%s\n' "$output" | iperf_mbps >/dev/null 2>&1; then
        printf '%s\n' "$output"
        return 0
      fi
    else
      code=$?
    fi

    if [[ "$attempt" -lt 3 ]]; then
      echo "nvpn+FIPS perf regression e2e: retrying iperf $label after attempt $attempt produced no throughput result" >&2
      start_iperf_server
      continue
    fi

    if [[ "$code" -eq 124 || "$code" -eq 137 ]]; then
      echo "nvpn+FIPS perf regression e2e failed: iperf $label timed out after ${IPERF_TIMEOUT_SECS}s" >&2
    elif [[ "$code" -ne 0 ]]; then
      echo "nvpn+FIPS perf regression e2e failed: iperf $label failed with exit $code" >&2
    else
      echo "nvpn+FIPS perf regression e2e failed: iperf $label returned no throughput result" >&2
    fi
    printf '%s\n' "$output" >&2
    exit 1
  done
}

parse_ping_stats() {
  awk '
    /time=/ {
      sample = $0
      sub(/^.*time=/, "", sample)
      sub(/[[:space:]].*$/, "", sample)
      if (sample != "") {
        times[++count] = sample + 0
      }
    }
    /packets transmitted/ {
      loss = $0
      sub(/^.*received, /, "", loss)
      sub(/% packet loss.*$/, "", loss)
    }
    /^rtt / || /^round-trip / {
      split($0, parts, "=")
      split(parts[2], values, "/")
      avg = values[2]
      max = values[3]
      sub(/^ /, "", avg)
      sub(/^ /, "", max)
    }
    END {
      if (loss == "" || avg == "" || max == "" || count == 0) {
        exit 1
      }
      for (i = 1; i <= count; i++) {
        for (j = i + 1; j <= count; j++) {
          if (times[j] < times[i]) {
            tmp = times[i]
            times[i] = times[j]
            times[j] = tmp
          }
        }
      }
      p95_idx = int((count * 95 + 99) / 100)
      p99_idx = int((count * 99 + 99) / 100)
      if (p95_idx < 1) p95_idx = 1
      if (p99_idx < 1) p99_idx = 1
      if (p95_idx > count) p95_idx = count
      if (p99_idx > count) p99_idx = count
      printf "%s %s %s %s %s\n", loss, avg, times[p95_idx], times[p99_idx], max
    }
  '
}

assert_ping_ok() {
  local label="$1"
  local output="$2"
  local max_loss="${3:-$MAX_PING_LOSS_PERCENT}"
  local max_avg="${4:-$MAX_PING_AVG_MS}"
  local max_max="${5:-$MAX_PING_MAX_MS}"
  local max_p95="${6:-$max_max}"
  local max_p99="${7:-$max_max}"
  local stats loss avg p95 p99 max
  if ! stats="$(printf '%s\n' "$output" | parse_ping_stats)"; then
    echo "nvpn+FIPS perf regression e2e failed: could not parse ping stats for $label" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  read -r loss avg p95 p99 max <<<"$stats"
  LAST_PING_LOSS="$loss"
  LAST_PING_AVG="$avg"
  LAST_PING_P95="$p95"
  LAST_PING_P99="$p99"
  LAST_PING_MAX="$max"
  CURRENT_PING_LOSS="$loss"
  CURRENT_PING_AVG="$avg"
  CURRENT_PING_P95="$p95"
  CURRENT_PING_P99="$p99"
  CURRENT_PING_MAX="$max"
  printf '%s ping: loss=%s%% avg=%sms p95=%sms p99=%sms max=%sms\n' \
    "$label" "$loss" "$avg" "$p95" "$p99" "$max"
  assert_float_at_most "$loss" "$max_loss" "$label ping loss %"
  assert_float_at_most "$avg" "$max_avg" "$label ping avg ms"
  assert_float_at_most "$p95" "$max_p95" "$label ping p95 ms"
  assert_float_at_most "$p99" "$max_p99" "$label ping p99 ms"
  assert_float_at_most "$max" "$max_max" "$label ping max ms"
}

peak_wait_pipeline_lines_from_stdin() {
  awk '
    function duration_us(value, number) {
      number = value + 0
      if (value ~ /ns$/) {
        return number / 1000
      }
      if (value ~ /ms$/) {
        return number * 1000
      }
      if (value ~ /s$/ && value !~ /us$/ && value !~ /ms$/ && value !~ /ns$/) {
        return number * 1000000
      }
      return number
    }
    function metric_avg_us(line, metric, start, rest, avg_start, parts) {
      start = index(line, metric "=")
      if (start == 0) {
        return -1
      }
      rest = substr(line, start)
      avg_start = index(rest, " avg=")
      if (avg_start == 0) {
        return -1
      }
      rest = substr(rest, avg_start + 5)
      split(rest, parts, " ")
      return duration_us(parts[1])
    }
    function fips_score(line, score, value) {
      score = -1
      value = metric_avg_us(line, "endpoint_event_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_priority_event_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_bulk_event_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "transport_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "transport_priority_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "transport_bulk_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_worker_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_worker_priority_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_worker_bulk_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_linux_bulk_container_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_linux_bulk_container_ready_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_linux_bulk_container_first_slot_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_linux_bulk_container_all_slots_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "decrypt_fallback_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "decrypt_fallback_priority_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "decrypt_fallback_bulk_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fsp_aead_worker_open_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fsp_aead_worker_open_completion_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_command_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_priority_command_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_bulk_command_wait")
      if (value > score) score = value
      return score
    }
    function nvpn_score(line, score, value) {
      score = -1
      value = metric_avg_us(line, "nvpn_tun_to_mesh_queue_wait")
      if (value > score) score = value
      return score
    }
    /^\[pipe / {
      score = fips_score($0)
      if (fips == "" || score > fips_score_best || score == fips_score_best) {
        fips = $0
        fips_score_best = score
      }
    }
    /^\[nvpn-pipe / {
      score = nvpn_score($0)
      if (nvpn == "" || score > nvpn_score_best || score == nvpn_score_best) {
        nvpn = $0
        nvpn_score_best = score
      }
    }
    END {
      if (fips != "" && nvpn != "") {
        print fips " | " nvpn
      } else if (fips != "") {
        print fips
      } else if (nvpn != "") {
        print nvpn
      }
    }
  '
}

latest_pipeline_lines_from_stdin() {
  peak_wait_pipeline_lines_from_stdin
}

pipeline_lines_after_start_from_stdin() {
  local start_line="${1:-0}"
  awk -v start_line="$start_line" 'NR > start_line'
}

load_pipeline_lines_from_stdin() {
  awk '
    function metric_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return -1
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    function fips_load_score(line, score, value) {
      score = -1
      value = metric_rate(line, "fmp_worker_batch_packets")
      if (value > score) score = value
      value = metric_rate(line, "udp_send_connected")
      if (value > score) score = value
      value = metric_rate(line, "connected_udp_direct_decrypt")
      if (value > score) score = value
      value = metric_rate(line, "endpoint_send")
      if (value > score) score = value
      value = metric_rate(line, "fmp_decrypt")
      if (value > score) score = value
      return score
    }
    function nvpn_load_score(line, score, value) {
      score = -1
      value = metric_rate(line, "nvpn_tun_read")
      if (value > score) score = value
      value = metric_rate(line, "nvpn_tun_write")
      if (value > score) score = value
      value = metric_rate(line, "nvpn_tun_to_mesh_queue_wait")
      if (value > score) score = value
      value = metric_rate(line, "nvpn_mesh_send")
      if (value > score) score = value
      return score
    }
    /^\[pipe / {
      score = fips_load_score($0)
      if (fips == "" || score > fips_score_best || score == fips_score_best) {
        fips = $0
        fips_score_best = score
      }
    }
    /^\[nvpn-pipe / {
      score = nvpn_load_score($0)
      if (nvpn == "" || score > nvpn_score_best || score == nvpn_score_best) {
        nvpn = $0
        nvpn_score_best = score
      }
    }
    END {
      if (fips != "" && nvpn != "") {
        print fips " | " nvpn
      } else if (fips != "") {
        print fips
      } else if (nvpn != "") {
        print nvpn
      }
    }
  '
}

pipeline_queue_wait_top_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function duration_ms(value, number) {
      number = value + 0
      if (value ~ /ns$/) {
        return number / 1000000
      }
      if (value ~ /us$/) {
        return number / 1000
      }
      if (value ~ /ms$/) {
        return number
      }
      if (value ~ /s$/) {
        return number * 1000
      }
      return number
    }
    function parse_wait(line, metric, start, rest, parts, rate_raw, p95_raw, p99_raw, max_raw, allmax_raw) {
      start = index(line, metric "=")
      if (start == 0) {
        return 0
      }
      rest = substr(line, start)
      split(rest, parts, " ")
      if (parts[4] !~ /^p95<=/ || parts[5] !~ /^p99<=/ || parts[6] !~ /^max<=/ || parts[7] !~ /^allmax=/) {
        return 0
      }
      rate_raw = parts[1]
      p95_raw = parts[4]
      p99_raw = parts[5]
      max_raw = parts[6]
      allmax_raw = parts[7]
      sub(/^[^=]+=/, "", rate_raw)
      sub(/\/s$/, "", rate_raw)
      sub(/^p95<=/, "", p95_raw)
      sub(/^p99<=/, "", p99_raw)
      sub(/^max<=/, "", max_raw)
      sub(/^allmax=/, "", allmax_raw)
      metric_rate = rate_raw + 0
      metric_p95 = duration_ms(p95_raw)
      metric_p99 = duration_ms(p99_raw)
      metric_max = duration_ms(max_raw)
      metric_allmax = duration_ms(allmax_raw)
      return 1
    }
    function metric_specificity(metric) {
      return metric ~ /_(priority|bulk)_/ ? 1 : 0
    }
    function metric_is_better(metric, specificity) {
      specificity = metric_specificity(metric)
      if (best_name == "") {
        return 1
      }
      if (metric_p99 != best_p99) {
        return metric_p99 > best_p99
      }
      if (metric_p95 != best_p95) {
        return metric_p95 > best_p95
      }
      if (metric_max != best_max) {
        return metric_max > best_max
      }
      if (specificity != best_specificity) {
        return specificity > best_specificity
      }
      return 0
    }
    BEGIN {
      metrics = "endpoint_command_wait endpoint_priority_command_wait endpoint_bulk_command_wait endpoint_event_wait endpoint_priority_event_wait endpoint_bulk_event_wait connected_udp_drain_recv connected_udp_fast_path_dispatch fmp_worker_queue_wait fmp_worker_priority_queue_wait fmp_worker_bulk_queue_wait fmp_linux_bulk_container_queue_wait fmp_linux_bulk_container_ready_wait fmp_linux_bulk_container_first_slot_wait fmp_linux_bulk_container_all_slots_wait decrypt_worker_queue_wait decrypt_worker_priority_queue_wait decrypt_worker_bulk_queue_wait decrypt_fallback_wait decrypt_fallback_priority_wait decrypt_fallback_bulk_wait fsp_aead_worker_open_queue_wait fsp_aead_worker_open_completion_wait decrypt_authenticated_session_wait decrypt_authenticated_session_priority_wait decrypt_authenticated_session_bulk_wait decrypt_direct_session_commit_wait decrypt_direct_session_data_wait decrypt_fsp_worker_queue_wait decrypt_fsp_worker_priority_queue_wait decrypt_fsp_worker_bulk_queue_wait transport_queue_wait transport_priority_queue_wait transport_bulk_queue_wait transport_channel_wait transport_priority_channel_wait transport_bulk_channel_wait transport_rx_loop_wait transport_priority_rx_loop_wait transport_bulk_rx_loop_wait nvpn_tun_to_mesh_queue_wait"
      metric_count = split(metrics, names, " ")
      best_p99 = -1
      best_p95 = -1
      best_max = -1
      best_allmax = -1
      best_specificity = -1
    }
    {
      line = $0
      for (i = 1; i <= metric_count; i++) {
        name = names[i]
        if (!parse_wait(line, name)) {
          continue
        }
        if (metric_is_better(name)) {
          best_name = name
          best_p95 = metric_p95
          best_p99 = metric_p99
          best_max = metric_max
          best_allmax = metric_allmax
          best_rate = metric_rate
          best_specificity = metric_specificity(name)
        }
      }
    }
    END {
      if (best_name != "") {
        printf "%s:rate_per_sec=%g,p95_ms=%g,p99_ms=%g,max_ms=%g,allmax_ms=%g\n", best_name, best_rate, best_p95, best_p99, best_max, best_allmax
      }
    }
  '
}

pipeline_worker_batch_summary() {
  local line="$1"
  local prefix="$2"
  printf '%s\n' "$line" | awk -v prefix="$prefix" '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    {
      flush = parse_rate($0, prefix "_flush")
      if (flush <= 0) {
        next
      }
      packets = parse_rate($0, prefix "_packets")
      full = parse_rate($0, prefix "_full")
      single = parse_rate($0, prefix "_single")
      priority_packets = parse_rate($0, prefix "_priority_packets")
      bulk_packets = parse_rate($0, prefix "_bulk_packets")
      send_groups = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group") : 0
      send_group_packets = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_packets") : 0
      send_group_single = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_single") : 0
      split_target = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_split_target") : 0
      split_lane = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_split_lane") : 0
      split_backpressure = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_split_backpressure") : 0
      split_packet_cap = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_split_packet_cap") : 0
      split_total = split_target + split_lane + split_backpressure + split_packet_cap
      committed_dispatch = prefix == "fmp_worker_batch" ? parse_rate($0, "endpoint_committed_bulk_dispatch_batch") : 0
      committed_packets = prefix == "fmp_worker_batch" ? parse_rate($0, "endpoint_committed_bulk_dispatch_packets") : 0
      committed_merged_batches = prefix == "fmp_worker_batch" ? parse_rate($0, "endpoint_committed_bulk_dispatch_merged_batch") : 0
      committed_merged_packets = prefix == "fmp_worker_batch" ? parse_rate($0, "endpoint_committed_bulk_dispatch_merged_packets") : 0
      committed_avg = committed_dispatch > 0 ? committed_packets / committed_dispatch : 0
      select_priority = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_select_priority") : 0
      select_fmp_completion = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_select_fmp_completion") : 0
      select_fsp_completion_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_select_fsp_completion_packets") : 0
      select_bulk_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_select_bulk_packets") : 0
      drain_priority = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_drain_priority") : 0
      drain_completion_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_drain_aead_completion_packets") : 0
      drain_bulk_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_drain_bulk_packets") : 0
      interleave_completion_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_bulk_interleave_aead_completion_packets") : 0
      interleave_budget_exhausted = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_bulk_interleave_budget_exhausted") : 0
      scheduler_total = select_priority + select_fmp_completion + select_fsp_completion_packets + select_bulk_packets + drain_priority + drain_completion_packets + drain_bulk_packets + interleave_completion_packets + interleave_budget_exhausted
      avg = packets / flush
      full_pct = (full / flush) * 100
      single_pct = (single / flush) * 100
      send_groups_per_flush = send_groups > 0 ? send_groups / flush : 0
      send_group_avg = send_groups > 0 ? send_group_packets / send_groups : 0
      send_group_single_pct = send_groups > 0 ? (send_group_single / send_groups) * 100 : 0
      lane_packets = priority_packets + bulk_packets
      if (lane_packets > 0) {
        priority_pct = (priority_packets / lane_packets) * 100
        bulk_pct = (bulk_packets / lane_packets) * 100
        summary = sprintf("avg_packets=%.1f,full_pct=%.1f,single_pct=%.1f,priority_pct=%.1f,bulk_pct=%.1f,flush_per_sec=%g,packets_per_sec=%g,priority_packets_per_sec=%g,bulk_packets_per_sec=%g", avg, full_pct, single_pct, priority_pct, bulk_pct, flush, packets, priority_packets, bulk_packets)
      } else {
        summary = sprintf("avg_packets=%.1f,full_pct=%.1f,single_pct=%.1f,flush_per_sec=%g,packets_per_sec=%g", avg, full_pct, single_pct, flush, packets)
      }
      if (send_groups > 0) {
        summary = summary sprintf(",send_groups_per_flush=%.1f,send_group_avg_packets=%.1f,send_group_single_pct=%.1f,send_groups_per_sec=%g,send_group_packets_per_sec=%g", send_groups_per_flush, send_group_avg, send_group_single_pct, send_groups, send_group_packets)
      }
      if (split_total > 0) {
        summary = summary sprintf(",send_group_split_total_per_sec=%g,send_group_split_target_per_sec=%g,send_group_split_lane_per_sec=%g,send_group_split_backpressure_per_sec=%g,send_group_split_packet_cap_per_sec=%g", split_total, split_target, split_lane, split_backpressure, split_packet_cap)
      }
      if (committed_dispatch > 0) {
        summary = summary sprintf(",committed_bulk_dispatch_avg_packets=%.1f,committed_bulk_dispatch_per_sec=%g,committed_bulk_dispatch_packets_per_sec=%g,committed_bulk_merged_batches_per_sec=%g,committed_bulk_merged_packets_per_sec=%g", committed_avg, committed_dispatch, committed_packets, committed_merged_batches, committed_merged_packets)
      }
      if (scheduler_total > 0) {
        summary = summary sprintf(",select_priority_per_sec=%g,select_fmp_completion_per_sec=%g,select_fsp_completion_packets_per_sec=%g,select_bulk_packets_per_sec=%g,drain_priority_per_sec=%g,drain_completion_packets_per_sec=%g,drain_bulk_packets_per_sec=%g,bulk_interleave_completion_packets_per_sec=%g,bulk_interleave_budget_exhausted_per_sec=%g", select_priority, select_fmp_completion, select_fsp_completion_packets, select_bulk_packets, drain_priority, drain_completion_packets, drain_bulk_packets, interleave_completion_packets, interleave_budget_exhausted)
      }
      print summary
      exit
    }
  '
}

pipeline_fmp_worker_batch_summary() {
  pipeline_worker_batch_summary "$1" "fmp_worker_batch"
}

pipeline_fmp_worker_dispatch_spread_summary() {
  docker_bench_pipeline_fmp_worker_dispatch_spread_summary "$1"
}

pipeline_fsp_owner_placement_summary() {
  docker_bench_pipeline_fsp_owner_placement_summary "$1"
}

pipeline_fsp_owner_placement_kind() {
  docker_bench_pipeline_fsp_owner_placement_kind "$1"
}

pipeline_decrypt_worker_batch_summary() {
  pipeline_worker_batch_summary "$1" "decrypt_worker_batch"
}

pipeline_decrypt_worker_spread_summary() {
  docker_bench_pipeline_decrypt_worker_spread_summary "$1"
}

pipeline_decrypt_worker_turn_mix_summary() {
  docker_bench_pipeline_decrypt_worker_turn_mix_summary "$1"
}

pipeline_linux_bulk_container_summary() {
  docker_bench_pipeline_linux_bulk_container_summary "$1"
}

pipeline_udp_send_batch_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    {
      gso_batches = parse_rate($0, "udp_send_gso_batch")
      gso_packets = parse_rate($0, "udp_send_gso_packets")
      gso_ge32 = parse_rate($0, "udp_send_gso_batch_ge32")
      gso_ge48 = parse_rate($0, "udp_send_gso_batch_ge48")
      gso_eq64 = parse_rate($0, "udp_send_gso_batch_eq64")
      sendmmsg_batches = parse_rate($0, "udp_send_sendmmsg_batch")
      sendmmsg_packets = parse_rate($0, "udp_send_sendmmsg_packets")
      sendmmsg_ge32 = parse_rate($0, "udp_send_sendmmsg_batch_ge32")
      sendmmsg_ge48 = parse_rate($0, "udp_send_sendmmsg_batch_ge48")
      sendmmsg_eq64 = parse_rate($0, "udp_send_sendmmsg_batch_eq64")
      total_batches = gso_batches + sendmmsg_batches
      total_packets = gso_packets + sendmmsg_packets
      if (total_batches <= 0 && total_packets <= 0) {
        next
      }
      gso_pct = total_packets > 0 ? (gso_packets / total_packets) * 100 : 0
      sendmmsg_pct = total_packets > 0 ? (sendmmsg_packets / total_packets) * 100 : 0
      gso_avg = gso_batches > 0 ? gso_packets / gso_batches : 0
      sendmmsg_avg = sendmmsg_batches > 0 ? sendmmsg_packets / sendmmsg_batches : 0
      gso_ge32_pct = gso_batches > 0 ? (gso_ge32 / gso_batches) * 100 : 0
      gso_ge48_pct = gso_batches > 0 ? (gso_ge48 / gso_batches) * 100 : 0
      gso_eq64_pct = gso_batches > 0 ? (gso_eq64 / gso_batches) * 100 : 0
      sendmmsg_ge32_pct = sendmmsg_batches > 0 ? (sendmmsg_ge32 / sendmmsg_batches) * 100 : 0
      sendmmsg_ge48_pct = sendmmsg_batches > 0 ? (sendmmsg_ge48 / sendmmsg_batches) * 100 : 0
      sendmmsg_eq64_pct = sendmmsg_batches > 0 ? (sendmmsg_eq64 / sendmmsg_batches) * 100 : 0
      avg = total_batches > 0 ? total_packets / total_batches : 0
      printf "gso_packet_pct=%.1f,sendmmsg_packet_pct=%.1f,avg_packets=%.1f,gso_avg_packets=%.1f,sendmmsg_avg_packets=%.1f,gso_ge32_pct=%.1f,gso_ge48_pct=%.1f,gso_eq64_pct=%.1f,sendmmsg_ge32_pct=%.1f,sendmmsg_ge48_pct=%.1f,sendmmsg_eq64_pct=%.1f,gso_batch_per_sec=%g,gso_packets_per_sec=%g,sendmmsg_batch_per_sec=%g,sendmmsg_packets_per_sec=%g,total_packets_per_sec=%g\n", gso_pct, sendmmsg_pct, avg, gso_avg, sendmmsg_avg, gso_ge32_pct, gso_ge48_pct, gso_eq64_pct, sendmmsg_ge32_pct, sendmmsg_ge48_pct, sendmmsg_eq64_pct, gso_batches, gso_packets, sendmmsg_batches, sendmmsg_packets, total_packets
      exit
    }
  '
}

pipeline_hard_event_summary_from_stdin() {
  local start_line="${1:-0}"
  awk -v start_line="$start_line" '
    function parse_event(line, name, in_phase, start, rest, parts, rate_raw, rate, total_raw, total, i) {
      start = index(line, name "=")
      if (start == 0) {
        return
      }
      rest = substr(line, start + length(name) + 1)
      split(rest, parts, " ")
      rate_raw = parts[1]
      sub(/\/s$/, "", rate_raw)
      rate = rate_raw + 0
      if (in_phase && (!(name in max_rate) || rate > max_rate[name])) {
        max_rate[name] = rate
      }
      for (i = 2; i <= 6; i++) {
        if (!(i in parts)) {
          break
        }
        if (parts[i] ~ /^total=/) {
          total_raw = parts[i]
          sub(/^total=/, "", total_raw)
          total = total_raw + 0
          if (in_phase && (!(name in max_total) || total > max_total[name])) {
            max_total[name] = total
          } else if (!in_phase && (!(name in base_total) || total > base_total[name])) {
            base_total[name] = total
          }
          break
        }
      }
    }
    BEGIN {
      events = "udp_send_backpressure udp_send_backpressure_sleep udp_send_bulk_dropped connected_udp_activation_failed connected_udp_peer_cap_skipped connected_udp_fd_budget_skipped connected_udp_kernel_dropped connected_udp_peer_kernel_dropped connected_udp_drain_bulk_dropped connected_udp_direct_decrypt_bulk_shed encrypt_worker_queue_full encrypt_worker_priority_queue_full encrypt_worker_bulk_queue_full encrypt_worker_bulk_dropped fmp_linux_bulk_container_queue_full fmp_linux_bulk_container_queue_full_packets endpoint_bulk_fast_path_prepare_failed endpoint_bulk_fast_path_stage_full endpoint_bulk_fast_path_feedback_full decrypt_worker_queue_full decrypt_worker_bulk_dropped decrypt_worker_register_full decrypt_worker_priority_dropped decrypt_fallback_backlog_high rx_loop_slow_maintenance_timeout rx_loop_slow_maintenance_skipped decrypt_fallback_bulk_dropped decrypt_fallback_priority_dropped decrypt_fallback_pressure_drain decrypt_fallback_priority_gated decrypt_fsp_priority_queue_full_fallback decrypt_fsp_bulk_queue_full_fallback decrypt_fsp_owner_handoff_dropped decrypt_fsp_open_pool_queue_full_fallback decrypt_fsp_open_worker_completion_backlog_fallback decrypt_fsp_worker_replay_dropped decrypt_fsp_worker_replay_dropped_duplicate decrypt_fsp_worker_replay_dropped_too_old decrypt_fsp_worker_replay_dropped_too_old_lag_ge_2x_window decrypt_fsp_worker_replay_dropped_too_old_lag_ge_4x_window decrypt_fsp_worker_replay_dropped_too_old_lag_ge_16x_window decrypt_fsp_worker_replay_dropped_too_old_lag_ge_64x_window fmp_aead_completion_aead_failed fsp_aead_completion_aead_failed fsp_aead_completion_epoch_mismatch fsp_aead_completion_replay_dropped_helper fsp_aead_completion_replay_dropped_helper_returned fsp_aead_completion_replay_dropped_worker_open fsp_aead_completion_replay_dropped_worker_open_returned fsp_aead_completion_replay_dropped_duplicate fsp_aead_completion_replay_dropped_too_old fsp_aead_completion_replay_dropped_too_old_lag_ge_2x_window fsp_aead_completion_replay_dropped_too_old_lag_ge_4x_window fsp_aead_completion_replay_dropped_too_old_lag_ge_16x_window fsp_aead_completion_replay_dropped_too_old_lag_ge_64x_window decrypt_authenticated_session_priority_dropped decrypt_authenticated_session_bulk_dropped pending_tun_destination_dropped pending_tun_packet_dropped pending_endpoint_destination_dropped pending_endpoint_packet_dropped endpoint_event_backlog_high endpoint_command_bulk_dropped endpoint_event_bulk_dropped transport_channel_backlog_high transport_bulk_dropped nvpn_tun_to_mesh_bulk_dropped nvpn_tun_to_mesh_bulk_dropped_batches nvpn_tun_to_mesh_bulk_dropped_packet_cap nvpn_tun_to_mesh_bulk_dropped_channel_full"
      event_count = split(events, names, " ")
    }
    {
      in_phase = (NR > start_line)
      for (i = 1; i <= event_count; i++) {
        parse_event($0, names[i], in_phase)
      }
    }
    END {
      first = 1
      for (i = 1; i <= event_count; i++) {
        name = names[i]
        rate = (name in max_rate) ? max_rate[name] : 0
        base = (name in base_total) ? base_total[name] : 0
        total = (name in max_total) ? max_total[name] - base : 0
        if (total < 0) {
          total = 0
        }
        if (rate <= 0 && total <= 0) {
          continue
        }
        if (!first) {
          printf ";"
        }
        first = 0
        printf "%s:max_rate_per_sec=%g,total=%g", name, rate, total
      }
      if (!first) {
        printf "\n"
      }
    }
  '
}

pipeline_line_count() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "if [ -r /tmp/connect.log ]; then grep -Ec '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log || true; else printf '0\\n'; fi" \
    | tr -d '\r' \
    | awk 'NR == 1 { print $1 + 0; found = 1 } END { if (!found) print 0 }'
}

pipeline_hard_event_summary() {
  local service="$1"
  local start_line="${2:-0}"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "grep -E '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log 2>/dev/null || true" \
    | tr -d '\r' \
    | pipeline_hard_event_summary_from_stdin "$start_line"
}

priority_hard_event_violations_from_summary() {
  awk -F';' '
    BEGIN {
      events = "encrypt_worker_priority_queue_full decrypt_worker_priority_dropped decrypt_fallback_priority_dropped decrypt_fsp_priority_queue_full_fallback decrypt_authenticated_session_priority_dropped"
      event_count = split(events, names, " ")
      for (i = 1; i <= event_count; i++) {
        watched[names[i]] = 1
      }
      first = 1
    }
    {
      for (i = 1; i <= NF; i++) {
        name = $i
        sub(/:.*/, "", name)
        if (!(name in watched)) {
          continue
        }
        rate = 0
        total = 0
        rest = $i
        sub(/^[^:]*:/, "", rest)
        part_count = split(rest, parts, ",")
        for (j = 1; j <= part_count; j++) {
          if (parts[j] ~ /^max_rate_per_sec=/) {
            raw = parts[j]
            sub(/^max_rate_per_sec=/, "", raw)
            rate = raw + 0
          } else if (parts[j] ~ /^total=/) {
            raw = parts[j]
            sub(/^total=/, "", raw)
            total = raw + 0
          }
        }
        if (total > 0 || rate > 0) {
          if (!first) {
            printf ";"
          }
          first = 0
          printf "%s(total=%g,max_rate_per_sec=%g)", name, total, rate
        }
      }
    }
    END {
      if (!first) {
        printf "\n"
      }
    }
  '
}

assert_no_priority_hard_events() {
  is_true "$FAIL_ON_PRIORITY_HARD_EVENTS" || return 0
  local label="$1"
  local summary="$2"
  local violations
  violations="$(printf '%s\n' "$summary" | priority_hard_event_violations_from_summary)"
  if [[ -n "$violations" ]]; then
    append_failure_summary "$label priority/control hard events" "==" "$violations" "none"
    echo "nvpn+FIPS perf regression e2e failed: $label priority/control hard events observed: $violations" >&2
    exit 1
  fi
}

pipeline_priority_queue_wait_violations_from_stdin() {
  local threshold_ms="$1"
  local min_rate_per_sec="${2:-0}"
  awk -v threshold_ms="$threshold_ms" -v min_rate_per_sec="$min_rate_per_sec" '
    function duration_ms(value, number) {
      number = value + 0
      if (value ~ /ns$/) {
        return number / 1000000
      }
      if (value ~ /us$/) {
        return number / 1000
      }
      if (value ~ /ms$/) {
        return number
      }
      if (value ~ /s$/) {
        return number * 1000
      }
      return number
    }
    function parse_wait(line, metric, start, rest, parts, rate_raw, p99_raw, max_raw) {
      start = index(line, metric "=")
      if (start == 0) {
        return 0
      }
      rest = substr(line, start)
      split(rest, parts, " ")
      if (parts[1] !~ "^" metric "=" || parts[5] !~ /^p99<=/ || parts[6] !~ /^max<=/) {
        return 0
      }
      rate_raw = parts[1]
      p99_raw = parts[5]
      max_raw = parts[6]
      sub(/^[^=]+=/, "", rate_raw)
      sub(/\/s$/, "", rate_raw)
      sub(/^p99<=/, "", p99_raw)
      sub(/^max<=/, "", max_raw)
      metric_rate = rate_raw + 0
      metric_p99 = duration_ms(p99_raw)
      metric_max = duration_ms(max_raw)
      return 1
    }
    BEGIN {
      metrics = "endpoint_priority_command_wait endpoint_priority_event_wait fmp_worker_priority_queue_wait decrypt_worker_priority_queue_wait decrypt_fallback_priority_wait decrypt_authenticated_session_priority_wait decrypt_fsp_worker_priority_queue_wait transport_priority_queue_wait transport_priority_channel_wait transport_priority_rx_loop_wait"
      metric_count = split(metrics, names, " ")
      first = 1
    }
    {
      for (i = 1; i <= metric_count; i++) {
        name = names[i]
        if (!parse_wait($0, name)) {
          continue
        }
        if (metric_rate < min_rate_per_sec || metric_max <= threshold_ms) {
          continue
        }
        if (!(name in max_seen) || metric_max > max_seen[name]) {
          max_seen[name] = metric_max
          p99_seen[name] = metric_p99
          rate_seen[name] = metric_rate
        }
      }
    }
    END {
      for (i = 1; i <= metric_count; i++) {
        name = names[i]
        if (!(name in max_seen)) {
          continue
        }
        if (!first) {
          printf ";"
        }
        first = 0
        printf "%s(max_ms=%g,p99_ms=%g,rate_per_sec=%g)", name, max_seen[name], p99_seen[name], rate_seen[name]
      }
      if (!first) {
        printf "\n"
      }
    }
  '
}

pipeline_priority_queue_wait_violations() {
  local service="$1"
  local start_line="${2:-0}"
  local threshold_ms="$3"
  local min_rate_per_sec="${4:-0}"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "grep -E '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log 2>/dev/null || true" \
    | tr -d '\r' \
    | pipeline_lines_after_start_from_stdin "$start_line" \
    | pipeline_priority_queue_wait_violations_from_stdin "$threshold_ms" "$min_rate_per_sec"
}

assert_priority_queue_wait_ok() {
  local threshold_ms="${MAX_PRIORITY_QUEUE_WAIT_MS:-0}"
  if ! awk -v threshold_ms="$threshold_ms" 'BEGIN { exit !((threshold_ms + 0) > 0) }'; then
    return 0
  fi
  local min_rate_per_sec="${MIN_PRIORITY_QUEUE_WAIT_RATE_PER_SEC:-0}"
  local label="$1"
  local service="$2"
  local start_line="${3:-0}"
  local violations
  violations="$(pipeline_priority_queue_wait_violations "$service" "$start_line" "$threshold_ms" "$min_rate_per_sec")"
  if [[ -n "$violations" ]]; then
    append_failure_summary "$label priority queue wait max ms (rate >= ${min_rate_per_sec}/s)" "<=" "$violations" "$threshold_ms"
    echo "nvpn+FIPS perf regression e2e failed: $label priority queue waits exceeded ${threshold_ms}ms at >=${min_rate_per_sec}/s: $violations" >&2
    exit 1
  fi
}

peak_wait_pipeline_line() {
  local service="$1"
  local start_line="${2:-0}"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "grep -E '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log 2>/dev/null || true" \
    | tr -d '\r' \
    | pipeline_lines_after_start_from_stdin "$start_line" \
    | peak_wait_pipeline_lines_from_stdin
}

load_pipeline_line() {
  local service="$1"
  local start_line="${2:-0}"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "grep -E '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log 2>/dev/null || true" \
    | tr -d '\r' \
    | pipeline_lines_after_start_from_stdin "$start_line" \
    | load_pipeline_lines_from_stdin
}

linux_bulk_container_pipeline_summary() {
  local service="$1"
  local start_line="${2:-0}"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "grep -E '^\\[pipe ' /tmp/connect.log 2>/dev/null || true" \
    | tr -d '\r' \
    | pipeline_lines_after_start_from_stdin "$start_line" \
    | docker_bench_pipeline_linux_bulk_container_summary
}

latest_pipeline_line() {
  peak_wait_pipeline_line "$1"
}

print_pipeline_summary() {
  local phase="$1"
  local service output
  for service in node-a node-b; do
    output="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      "grep -E '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log 2>/dev/null | tail -n 6 || true" \
      || true)"
    if [[ -n "$output" ]]; then
      write_perf_artifact "$phase" "$service pipeline" "txt" "$output"
      printf '%s\n' "$output" | sed "s/^/$phase $service pipeline: /"
    fi
  done
}

tsv_field() {
  local value="$1"
  value="${value//$'\t'/ }"
  value="${value//$'\n'/ }"
  printf '%s' "$value"
}

artifact_slug() {
  local value="$1"
  value="$(printf '%s' "$value" | tr '[:upper:]' '[:lower:]')"
  value="$(printf '%s' "$value" | sed -E 's/[^a-z0-9._=-]+/-/g; s/^-+//; s/-+$//; s/-+/-/g')"
  printf '%s' "${value:-artifact}"
}

perf_artifact_path() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 1
  local phase_slug step_slug ext
  phase_slug="$(artifact_slug "$1")"
  step_slug="$(artifact_slug "$2")"
  ext="${3#.}"
  mkdir -p "$PERF_OUTPUT_DIR/raw"
  printf '%s/raw/%s-%s.%s\n' "$PERF_OUTPUT_DIR" "$phase_slug" "$step_slug" "$ext"
}

write_perf_artifact() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local path
  path="$(perf_artifact_path "$1" "$2" "$3")"
  printf '%s\n' "${4:-}" >"$path"
}

write_iperf_interval_artifact() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local phase="$1"
  local label="$2"
  local json="$3"
  local summary
  summary="$(printf '%s\n' "$json" | iperf_interval_summary)"
  write_perf_artifact "$phase" "$label intervals" "tsv" "$summary"
}

write_iperf_interval_artifact_from_file() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local phase="$1"
  local label="$2"
  local json_path="$3"
  local summary
  summary="$(iperf_interval_summary <"$json_path")"
  write_perf_artifact "$phase" "$label intervals" "tsv" "$summary"
}

write_iperf_server_artifacts() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local phase="$1"
  local label="$2"
  local json="$3"
  local server_json summary
  server_json="$(printf '%s\n' "$json" | iperf_server_json 2>/dev/null)" || return 0
  write_perf_artifact "$phase" "$label server" "iperf.json" "$server_json"
  summary="$(printf '%s\n' "$server_json" | iperf_interval_summary)"
  write_perf_artifact "$phase" "$label server intervals" "tsv" "$summary"
}

write_iperf_server_artifacts_from_file() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local phase="$1"
  local label="$2"
  local json_path="$3"
  local server_json summary
  server_json="$(iperf_server_json <"$json_path" 2>/dev/null)" || return 0
  write_perf_artifact "$phase" "$label server" "iperf.json" "$server_json"
  summary="$(printf '%s\n' "$server_json" | iperf_interval_summary)"
  write_perf_artifact "$phase" "$label server intervals" "tsv" "$summary"
}

write_iperf_sender_artifacts() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local phase="$1"
  local label="$2"
  local json="$3"
  local sender_json intervals summary
  sender_json="$(printf '%s\n' "$json" | iperf_sender_json)"
  write_perf_artifact "$phase" "$label sender" "iperf.json" "$sender_json"
  intervals="$(printf '%s\n' "$sender_json" | iperf_interval_summary)"
  write_perf_artifact "$phase" "$label sender intervals" "tsv" "$intervals"
  summary="$(printf '%s\n' "$sender_json" | iperf_sender_interval_summary)"
  write_perf_artifact "$phase" "$label sender summary" "tsv" "$summary"
}

write_iperf_sender_artifacts_from_file() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local phase="$1"
  local label="$2"
  local json_path="$3"
  local sender_json intervals summary
  sender_json="$(iperf_sender_json <"$json_path")"
  write_perf_artifact "$phase" "$label sender" "iperf.json" "$sender_json"
  intervals="$(printf '%s\n' "$sender_json" | iperf_interval_summary)"
  write_perf_artifact "$phase" "$label sender intervals" "tsv" "$intervals"
  summary="$(printf '%s\n' "$sender_json" | iperf_sender_interval_summary)"
  write_perf_artifact "$phase" "$label sender summary" "tsv" "$summary"
}

write_selected_pipeline_summary() {
  local phase="$1"
  local service="$2"
  local line="$3"
  [[ -z "$line" ]] && return 0
  write_perf_artifact "$phase" "$service selected pipeline" "txt" "$line"
}

copy_perf_artifact() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local path
  path="$(perf_artifact_path "$1" "$2" "$3")"
  cp "$4" "$path" 2>/dev/null || true
}

host_snapshot() {
  local ts cpu_count kernel loadavg
  ts="$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date 2>/dev/null || true)"
  printf 'timestamp_utc=%s\n' "$ts"

  kernel="$(uname -srm 2>/dev/null || true)"
  if [[ -n "$kernel" ]]; then
    printf 'kernel=%s\n' "$kernel"
  fi

  cpu_count=""
  if command -v nproc >/dev/null 2>&1; then
    cpu_count="$(nproc 2>/dev/null || true)"
  elif command -v sysctl >/dev/null 2>&1; then
    cpu_count="$(sysctl -n hw.ncpu 2>/dev/null || true)"
  elif command -v getconf >/dev/null 2>&1; then
    cpu_count="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
  fi
  if [[ -n "$cpu_count" ]]; then
    printf 'cpu_count=%s\n' "$cpu_count"
  fi

  if [[ -r /proc/loadavg ]]; then
    loadavg="$(awk '{ print $1 " " $2 " " $3 }' /proc/loadavg 2>/dev/null || true)"
    if [[ -n "$loadavg" ]]; then
      printf 'loadavg_1m_5m_15m=%s\n' "$loadavg"
    fi
  elif command -v sysctl >/dev/null 2>&1; then
    loadavg="$(sysctl -n vm.loadavg 2>/dev/null || true)"
    if [[ -n "$loadavg" ]]; then
      printf 'loadavg_1m_5m_15m=%s\n' "$loadavg"
    fi
  fi
  uptime 2>/dev/null || true

  printf 'top_cpu_processes:\n'
  if ps -Ao pcpu,pid,comm >/dev/null 2>&1; then
    ps -Ao pcpu,pid,comm 2>/dev/null \
      | awk 'NR > 1 { print }' \
      | sort -nr \
      | head -20 \
      || true
  elif ps -eo pcpu,pid,comm >/dev/null 2>&1; then
    ps -eo pcpu,pid,comm 2>/dev/null \
      | awk 'NR > 1 { print }' \
      | sort -nr \
      | head -20 \
      || true
  fi
}

write_host_snapshot() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return 0
  local path
  if ! path="$(perf_artifact_path "host" "$1" "txt" 2>/dev/null)"; then
    return 0
  fi
  host_snapshot >"$path" 2>&1 || true
  echo "writing host snapshot to $path"
}

append_phase_summary() {
  [[ -z "$PHASE_SUMMARY" ]] && return
  local first=1 field
  for field in "$@"; do
    if (( first )); then
      first=0
    else
      printf '\t' >>"$PHASE_SUMMARY"
    fi
    tsv_field "$field" >>"$PHASE_SUMMARY"
  done
  printf '\n' >>"$PHASE_SUMMARY"
}

phase_note() {
  case "$1" in
    unimpaired-underlay)
      printf '%s' "e2e loaded-liveness/regression gate on an unimpaired underlay; runs TCP load plus ping/liveness checks, not the peak perf-docker.sh clean throughput lane"
      ;;
    *)
      return 1
      ;;
  esac
}

append_phase_note() {
  [[ -z "$PHASE_NOTES" ]] && return
  local phase="$1" note
  note="$(phase_note "$phase" 2>/dev/null || true)"
  [[ -z "$note" ]] && return
  printf '%s\t%s\n' "$(tsv_field "$phase")" "$(tsv_field "$note")" >>"$PHASE_NOTES"
}

refresh_failure_context() {
  [[ -z "${CURRENT_PHASE:-}" ]] && return

  local after line
  if [[ -n "${CURRENT_DIRECT_BEFORE_A:-}" ]]; then
    after="$(direct_underlay_bytes node-a 2>/dev/null || true)"
    after="${after//$'\r'/}"
    if [[ "$after" =~ ^[0-9]+$ && "$CURRENT_DIRECT_BEFORE_A" =~ ^[0-9]+$ ]]; then
      CURRENT_DIRECT_DELTA_A=$((after - CURRENT_DIRECT_BEFORE_A))
    fi
  fi
  if [[ -n "${CURRENT_DIRECT_BEFORE_B:-}" ]]; then
    after="$(direct_underlay_bytes node-b 2>/dev/null || true)"
    after="${after//$'\r'/}"
    if [[ "$after" =~ ^[0-9]+$ && "$CURRENT_DIRECT_BEFORE_B" =~ ^[0-9]+$ ]]; then
      CURRENT_DIRECT_DELTA_B=$((after - CURRENT_DIRECT_BEFORE_B))
    fi
  fi

  line="$(peak_wait_pipeline_line node-a "${CURRENT_PIPELINE_START_A:-0}" 2>/dev/null || true)"
  if [[ -n "$line" ]]; then
    CURRENT_PIPELINE_A="$line"
  fi
  line="$(peak_wait_pipeline_line node-b "${CURRENT_PIPELINE_START_B:-0}" 2>/dev/null || true)"
  if [[ -n "$line" ]]; then
    CURRENT_PIPELINE_B="$line"
  fi
}

append_failure_summary() {
  [[ -z "$FAILURE_SUMMARY" ]] && return
  refresh_failure_context
  local first=1 field
  for field in \
    "$@" \
    "$CURRENT_PHASE" \
    "$CURRENT_STEP" \
    "$CURRENT_FORWARD_MBIT" \
    "$CURRENT_FORWARD_RETRANS" \
    "$CURRENT_REVERSE_MBIT" \
    "$CURRENT_REVERSE_RETRANS" \
    "$CURRENT_PROBE_MBIT" \
    "$CURRENT_PROBE_RETRANS" \
    "$CURRENT_PING_LOSS" \
    "$CURRENT_PING_AVG" \
    "$CURRENT_PING_P95" \
    "$CURRENT_PING_P99" \
    "$CURRENT_PING_MAX" \
    "$CURRENT_DIRECT_DELTA_A" \
    "$CURRENT_DIRECT_DELTA_B" \
    "$CURRENT_PIPELINE_A" \
    "$CURRENT_PIPELINE_B"; do
    if (( first )); then
      first=0
    else
      printf '\t' >>"$FAILURE_SUMMARY"
    fi
    tsv_field "$field" >>"$FAILURE_SUMMARY"
  done
  printf '\n' >>"$FAILURE_SUMMARY"
}

init_phase_summary() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return
  mkdir -p "$PERF_OUTPUT_DIR"
  PHASE_SUMMARY="$PERF_OUTPUT_DIR/phase-summary.tsv"
  PHASE_NOTES="$PERF_OUTPUT_DIR/phase-notes.tsv"
  FAILURE_SUMMARY="$PERF_OUTPUT_DIR/failure-summary.tsv"
  printf '%s\n' \
    'phase	forward_mbps	forward_retrans	reverse_mbps	reverse_retrans	forward_load_mbps	forward_load_retrans	forward_load_ping_loss_percent	forward_load_ping_avg_ms	forward_load_ping_p95_ms	forward_load_ping_p99_ms	forward_load_ping_max_ms	reverse_load_mbps	reverse_load_retrans	reverse_load_ping_loss_percent	reverse_load_ping_avg_ms	reverse_load_ping_p95_ms	reverse_load_ping_p99_ms	reverse_load_ping_max_ms	post_ping_loss_percent	post_ping_avg_ms	post_ping_p95_ms	post_ping_p99_ms	post_ping_max_ms	direct_bytes_node_a	direct_bytes_node_b	pipeline_top_queue_wait_node_a	pipeline_top_queue_wait_node_b	pipeline_fmp_worker_batch_node_a	pipeline_fmp_worker_batch_node_b	pipeline_fmp_worker_dispatch_spread_node_a	pipeline_fmp_worker_dispatch_spread_node_b	pipeline_decrypt_worker_batch_node_a	pipeline_decrypt_worker_batch_node_b	pipeline_decrypt_worker_spread_node_a	pipeline_decrypt_worker_spread_node_b	pipeline_decrypt_worker_turn_mix_node_a	pipeline_decrypt_worker_turn_mix_node_b	pipeline_linux_bulk_container_node_a	pipeline_linux_bulk_container_node_b	pipeline_udp_send_batch_node_a	pipeline_udp_send_batch_node_b	pipeline_hard_events_node_a	pipeline_hard_events_node_b	pipeline_node_a	pipeline_node_b' \
    >"$PHASE_SUMMARY"
  printf 'phase\tnote\n' >"$PHASE_NOTES"
  printf '%s\n' \
    'label	comparison	actual	threshold	phase	step	forward_mbps	forward_retrans	reverse_mbps	reverse_retrans	probe_mbps	probe_retrans	ping_loss_percent	ping_avg_ms	ping_p95_ms	ping_p99_ms	ping_max_ms	direct_bytes_node_a	direct_bytes_node_b	pipeline_node_a	pipeline_node_b' \
    >"$FAILURE_SUMMARY"
  echo "writing perf phase summary to $PHASE_SUMMARY"
  echo "writing perf phase notes to $PHASE_NOTES"
  echo "writing perf failure summary to $FAILURE_SUMMARY"
  echo "writing raw perf artifacts to $PERF_OUTPUT_DIR/raw"
}

write_perf_metadata() {
  [[ -z "$PERF_OUTPUT_DIR" ]] && return
  OUTPUT_DIR="$PERF_OUTPUT_DIR" \
    NVPN_DOCKER_PIPELINE_TRACE="$PIPELINE_TRACE" \
    NVPN_DOCKER_PIPELINE_INTERVAL_SECS="$PIPELINE_INTERVAL_SECS" \
    NVPN_DOCKER_EXTRA_ENV="${NVPN_DOCKER_EXTRA_ENV:-$EXTRA_ENV}" \
    docker_bench_write_metadata nvpn "$DURATION"
}

begin_phase_context() {
  CURRENT_PHASE="$1"
  CURRENT_STEP="starting"
  CURRENT_FORWARD_MBIT=""
  CURRENT_FORWARD_RETRANS=""
  CURRENT_REVERSE_MBIT=""
  CURRENT_REVERSE_RETRANS=""
  CURRENT_PROBE_MBIT=""
  CURRENT_PROBE_RETRANS=""
  CURRENT_PING_LOSS=""
  CURRENT_PING_AVG=""
  CURRENT_PING_P95=""
  CURRENT_PING_P99=""
  CURRENT_PING_MAX=""
  CURRENT_DIRECT_BEFORE_A="$2"
  CURRENT_DIRECT_BEFORE_B="$3"
  CURRENT_DIRECT_DELTA_A=""
  CURRENT_DIRECT_DELTA_B=""
  CURRENT_PIPELINE_A=""
  CURRENT_PIPELINE_B=""
  CURRENT_PIPELINE_START_A="${4:-0}"
  CURRENT_PIPELINE_START_B="${5:-0}"
}

start_iperf_server() {
  "${COMPOSE[@]}" exec -T node-b sh -c "pkill -9 iperf3 2>/dev/null; true" >/dev/null
  "${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -J -D --logfile /tmp/iperf3-server.log"
  sleep 1
}

run_concurrent_probe() {
  local phase="$1"
  local label="$2"
  local min_tcp="$3"
  local max_loss="$4"
  local max_avg="$5"
  local max_p95="$6"
  local max_p99="$7"
  local max_max="$8"
  shift 8

  local json_path err_path iperf_pid ping_output mbps retrans
  json_path="$(mktemp)"
  err_path="$(mktemp)"
  "${COMPOSE[@]}" exec -T node-a timeout --kill-after=5s "$IPERF_TIMEOUT_SECS" iperf3 \
    -J --get-server-output -c "$BOB_TUNNEL_IP" -t "$LOAD_DURATION" -i "$IPERF_INTERVAL_SECS" -O 1 --connect-timeout 3000 "$@" \
    >"$json_path" 2>"$err_path" &
  iperf_pid=$!
  sleep 1
  ping_output="$("${COMPOSE[@]}" exec -T node-a ping \
    -c "$PING_COUNT" -i "$PING_INTERVAL" -W 2 "$BOB_TUNNEL_IP" 2>&1)"
  write_perf_artifact "$phase" "$label TCP load ping" "ping.txt" "$ping_output"
  local iperf_status=0
  if wait "$iperf_pid"; then
    iperf_status=0
  else
    iperf_status=$?
    copy_perf_artifact "$phase" "$label TCP load" "iperf.json" "$json_path"
    copy_perf_artifact "$phase" "$label TCP load" "iperf.stderr" "$err_path"
    if [[ "$iperf_status" -eq 124 || "$iperf_status" -eq 137 ]]; then
      echo "nvpn+FIPS perf regression e2e failed: $phase $label iperf timed out after ${IPERF_TIMEOUT_SECS}s" >&2
    else
      echo "nvpn+FIPS perf regression e2e failed: $phase $label iperf failed with exit $iperf_status" >&2
    fi
    cat "$err_path" >&2
    exit 1
  fi
  copy_perf_artifact "$phase" "$label TCP load" "iperf.json" "$json_path"
  copy_perf_artifact "$phase" "$label TCP load" "iperf.stderr" "$err_path"
  write_iperf_interval_artifact_from_file "$phase" "$label TCP load" "$json_path"
  write_iperf_server_artifacts_from_file "$phase" "$label TCP load" "$json_path"
  write_iperf_sender_artifacts_from_file "$phase" "$label TCP load" "$json_path"
  if ! mbps="$(iperf_mbps <"$json_path")"; then
    echo "nvpn+FIPS perf regression e2e failed: $phase $label iperf returned no throughput result" >&2
    cat "$err_path" >&2
    cat "$json_path" >&2
    exit 1
  fi
  retrans="$(iperf_retransmits <"$json_path")"
  printf '%s %s TCP load: %.1f Mbps retrans=%s\n' "$phase" "$label" "$mbps" "$retrans"
  CURRENT_STEP="$label TCP load"
  CURRENT_PROBE_MBIT="$mbps"
  CURRENT_PROBE_RETRANS="$retrans"
  assert_float_at_least "$mbps" "$min_tcp" "$phase $label TCP throughput Mbps"
  assert_int_at_most_if_set "$retrans" "$MAX_TCP_RETRANS" "$phase $label TCP load retransmits"
  CURRENT_STEP="$label TCP load progress"
  assert_iperf_progress_file_ok "$phase $label TCP load" "$json_path"
  CURRENT_STEP="$label TCP load ping"
  assert_ping_ok "$phase during $label TCP load" "$ping_output" "$max_loss" "$max_avg" "$max_max" "$max_p95" "$max_p99"
  rm -f "$json_path" "$err_path"
  LAST_PROBE_MBIT="$mbps"
  LAST_PROBE_RETRANS="$retrans"
  LAST_PROBE_PING_LOSS="$LAST_PING_LOSS"
  LAST_PROBE_PING_AVG="$LAST_PING_AVG"
  LAST_PROBE_PING_P95="$LAST_PING_P95"
  LAST_PROBE_PING_P99="$LAST_PING_P99"
  LAST_PROBE_PING_MAX="$LAST_PING_MAX"
}

run_perf_phase() {
  local phase="$1"
  local min_tcp="$2"
  local min_reverse_tcp="$3"
  local max_loss="$4"
  local max_avg="$5"
  local max_p95="$6"
  local max_p99="$7"
  local max_max="$8"
  local post_max_loss="${9:-$max_loss}"
  local post_max_avg="${10:-$max_avg}"
  local post_max_p95="${11:-$max_p95}"
  local post_max_p99="${12:-$max_p99}"
  local post_max_max="${13:-$max_max}"
  local max_retrans_label="unbounded"
  [[ -n "$MAX_TCP_RETRANS" ]] && max_retrans_label="$MAX_TCP_RETRANS"
  local priority_hard_event_label="ignored"
  is_true "$FAIL_ON_PRIORITY_HARD_EVENTS" && priority_hard_event_label="0"
  local priority_wait_label="ignored"
  if awk -v threshold_ms="${MAX_PRIORITY_QUEUE_WAIT_MS:-0}" 'BEGIN { exit !((threshold_ms + 0) > 0) }'; then
    priority_wait_label="${MAX_PRIORITY_QUEUE_WAIT_MS}ms"
  fi

  echo "--- phase: $phase ---"
  local note
  note="$(phase_note "$phase" 2>/dev/null || true)"
  if [[ -n "$note" ]]; then
    echo "phase note: $note"
    append_phase_note "$phase"
  fi
  echo "thresholds: tcp>=${min_tcp}M reverse>=${min_reverse_tcp}M tcp_retrans<=${max_retrans_label} priority_hard_events<=${priority_hard_event_label} priority_queue_wait_max<=${priority_wait_label} during_ping_loss<=${max_loss}% during_ping_avg<=${max_avg}ms during_ping_p95<=${max_p95}ms during_ping_p99<=${max_p99}ms during_ping_max<=${max_max}ms post_ping_loss<=${post_max_loss}% post_ping_avg<=${post_max_avg}ms post_ping_p95<=${post_max_p95}ms post_ping_p99<=${post_max_p99}ms post_ping_max<=${post_max_max}ms"

  local direct_before_a direct_before_b pipeline_start_a pipeline_start_b
  direct_before_a="$(direct_underlay_bytes node-a)"
  direct_before_b="$(direct_underlay_bytes node-b)"
  pipeline_start_a="$(pipeline_line_count node-a)"
  pipeline_start_b="$(pipeline_line_count node-b)"
  begin_phase_context "$phase" "$direct_before_a" "$direct_before_b" "$pipeline_start_a" "$pipeline_start_b"

  start_iperf_server

  local forward_json forward_mbps forward_retrans
  CURRENT_STEP="forward TCP"
  forward_json="$(run_iperf_json "$phase forward TCP")"
  write_perf_artifact "$phase" "forward TCP" "iperf.json" "$forward_json"
  write_iperf_interval_artifact "$phase" "forward TCP" "$forward_json"
  write_iperf_server_artifacts "$phase" "forward TCP" "$forward_json"
  write_iperf_sender_artifacts "$phase" "forward TCP" "$forward_json"
  forward_mbps="$(printf '%s\n' "$forward_json" | iperf_mbps)"
  forward_retrans="$(printf '%s\n' "$forward_json" | iperf_retransmits)"
  printf '%s forward TCP: %.1f Mbps retrans=%s\n' "$phase" "$forward_mbps" "$forward_retrans"
  CURRENT_FORWARD_MBIT="$forward_mbps"
  CURRENT_FORWARD_RETRANS="$forward_retrans"
  assert_float_at_least "$forward_mbps" "$min_tcp" "$phase forward TCP throughput Mbps"
  assert_int_at_most_if_set "$forward_retrans" "$MAX_TCP_RETRANS" "$phase forward TCP retransmits"
  CURRENT_STEP="forward TCP progress"
  assert_iperf_progress_ok "$phase forward TCP" "$forward_json"

  local reverse_json reverse_mbps reverse_retrans
  CURRENT_STEP="reverse TCP"
  reverse_json="$(run_iperf_json "$phase reverse TCP" -R)"
  write_perf_artifact "$phase" "reverse TCP" "iperf.json" "$reverse_json"
  write_iperf_interval_artifact "$phase" "reverse TCP" "$reverse_json"
  write_iperf_server_artifacts "$phase" "reverse TCP" "$reverse_json"
  write_iperf_sender_artifacts "$phase" "reverse TCP" "$reverse_json"
  reverse_mbps="$(printf '%s\n' "$reverse_json" | iperf_mbps)"
  reverse_retrans="$(printf '%s\n' "$reverse_json" | iperf_retransmits)"
  printf '%s reverse TCP: %.1f Mbps retrans=%s\n' "$phase" "$reverse_mbps" "$reverse_retrans"
  CURRENT_REVERSE_MBIT="$reverse_mbps"
  CURRENT_REVERSE_RETRANS="$reverse_retrans"
  assert_float_at_least "$reverse_mbps" "$min_reverse_tcp" "$phase reverse TCP throughput Mbps"
  assert_int_at_most_if_set "$reverse_retrans" "$MAX_TCP_RETRANS" "$phase reverse TCP retransmits"
  CURRENT_STEP="reverse TCP progress"
  assert_iperf_progress_ok "$phase reverse TCP" "$reverse_json"

  local post_ping forward_load_mbps forward_load_retrans
  local forward_load_ping_loss forward_load_ping_avg forward_load_ping_p95 forward_load_ping_p99 forward_load_ping_max
  local reverse_load_mbps reverse_load_retrans
  local reverse_load_ping_loss reverse_load_ping_avg reverse_load_ping_p95 reverse_load_ping_p99 reverse_load_ping_max
  local post_ping_loss post_ping_avg post_ping_p95 post_ping_p99 post_ping_max
  local direct_delta_a direct_delta_b pipeline_a pipeline_b peak_pipeline_a peak_pipeline_b pipeline_top_a pipeline_top_b pipeline_batch_a pipeline_batch_b pipeline_spread_a pipeline_spread_b pipeline_decrypt_batch_a pipeline_decrypt_batch_b pipeline_udp_send_a pipeline_udp_send_b pipeline_hard_a pipeline_hard_b
  run_concurrent_probe "$phase" "forward" "$min_tcp" "$max_loss" "$max_avg" "$max_p95" "$max_p99" "$max_max"
  forward_load_mbps="$LAST_PROBE_MBIT"
  forward_load_retrans="$LAST_PROBE_RETRANS"
  forward_load_ping_loss="$LAST_PROBE_PING_LOSS"
  forward_load_ping_avg="$LAST_PROBE_PING_AVG"
  forward_load_ping_p95="$LAST_PROBE_PING_P95"
  forward_load_ping_p99="$LAST_PROBE_PING_P99"
  forward_load_ping_max="$LAST_PROBE_PING_MAX"
  run_concurrent_probe "$phase" "reverse" "$min_reverse_tcp" "$max_loss" "$max_avg" "$max_p95" "$max_p99" "$max_max" -R
  reverse_load_mbps="$LAST_PROBE_MBIT"
  reverse_load_retrans="$LAST_PROBE_RETRANS"
  reverse_load_ping_loss="$LAST_PROBE_PING_LOSS"
  reverse_load_ping_avg="$LAST_PROBE_PING_AVG"
  reverse_load_ping_p95="$LAST_PROBE_PING_P95"
  reverse_load_ping_p99="$LAST_PROBE_PING_P99"
  reverse_load_ping_max="$LAST_PROBE_PING_MAX"

  post_ping="$("${COMPOSE[@]}" exec -T node-a ping \
    -c "$PING_COUNT" -i "$PING_INTERVAL" -W 2 "$BOB_TUNNEL_IP" 2>&1)"
  write_perf_artifact "$phase" "post-load ping" "ping.txt" "$post_ping"
  CURRENT_STEP="post-load ping"
  assert_ping_ok "$phase after TCP load" "$post_ping" "$post_max_loss" "$post_max_avg" "$post_max_max" "$post_max_p95" "$post_max_p99"
  post_ping_loss="$LAST_PING_LOSS"
  post_ping_avg="$LAST_PING_AVG"
  post_ping_p95="$LAST_PING_P95"
  post_ping_p99="$LAST_PING_P99"
  post_ping_max="$LAST_PING_MAX"
  CURRENT_STEP="direct path node-a"
  assert_direct_counter_advanced node-a "$direct_before_a" "$phase node-a"
  direct_delta_a="$LAST_DIRECT_DELTA"
  CURRENT_STEP="direct path node-b"
  assert_direct_counter_advanced node-b "$direct_before_b" "$phase node-b"
  direct_delta_b="$LAST_DIRECT_DELTA"
  print_pipeline_summary "$phase"
  pipeline_a="$(load_pipeline_line node-a "$pipeline_start_a")"
  pipeline_b="$(load_pipeline_line node-b "$pipeline_start_b")"
  peak_pipeline_a="$(peak_wait_pipeline_line node-a "$pipeline_start_a")"
  peak_pipeline_b="$(peak_wait_pipeline_line node-b "$pipeline_start_b")"
  write_selected_pipeline_summary "$phase" "node-a load" "$pipeline_a"
  write_selected_pipeline_summary "$phase" "node-b load" "$pipeline_b"
  write_selected_pipeline_summary "$phase" "node-a peak-wait" "$peak_pipeline_a"
  write_selected_pipeline_summary "$phase" "node-b peak-wait" "$peak_pipeline_b"
  pipeline_top_a="$(pipeline_queue_wait_top_summary "$pipeline_a")"
  pipeline_top_b="$(pipeline_queue_wait_top_summary "$pipeline_b")"
  pipeline_batch_a="$(pipeline_fmp_worker_batch_summary "$pipeline_a")"
  pipeline_batch_b="$(pipeline_fmp_worker_batch_summary "$pipeline_b")"
  pipeline_spread_a="$(pipeline_fmp_worker_dispatch_spread_summary "$pipeline_a")"
  pipeline_spread_b="$(pipeline_fmp_worker_dispatch_spread_summary "$pipeline_b")"
  pipeline_decrypt_batch_a="$(pipeline_decrypt_worker_batch_summary "$pipeline_a")"
  pipeline_decrypt_batch_b="$(pipeline_decrypt_worker_batch_summary "$pipeline_b")"
  pipeline_decrypt_spread_a="$(pipeline_decrypt_worker_spread_summary "$pipeline_a")"
  pipeline_decrypt_spread_b="$(pipeline_decrypt_worker_spread_summary "$pipeline_b")"
  pipeline_decrypt_turn_mix_a="$(pipeline_decrypt_worker_turn_mix_summary "$pipeline_a")"
  pipeline_decrypt_turn_mix_b="$(pipeline_decrypt_worker_turn_mix_summary "$pipeline_b")"
  pipeline_linux_bulk_a="$(linux_bulk_container_pipeline_summary node-a "$pipeline_start_a")"
  pipeline_linux_bulk_b="$(linux_bulk_container_pipeline_summary node-b "$pipeline_start_b")"
  pipeline_udp_send_a="$(pipeline_udp_send_batch_summary "$pipeline_a")"
  pipeline_udp_send_b="$(pipeline_udp_send_batch_summary "$pipeline_b")"
  pipeline_hard_a="$(pipeline_hard_event_summary node-a "$pipeline_start_a")"
  pipeline_hard_b="$(pipeline_hard_event_summary node-b "$pipeline_start_b")"
  CURRENT_PIPELINE_A="$pipeline_a"
  CURRENT_PIPELINE_B="$pipeline_b"
  CURRENT_STEP="node-a priority hard event guard"
  assert_no_priority_hard_events "$phase node-a" "$pipeline_hard_a"
  CURRENT_STEP="node-b priority hard event guard"
  assert_no_priority_hard_events "$phase node-b" "$pipeline_hard_b"
  CURRENT_STEP="node-a priority queue wait guard"
  assert_priority_queue_wait_ok "$phase node-a" node-a "$pipeline_start_a"
  CURRENT_STEP="node-b priority queue wait guard"
  assert_priority_queue_wait_ok "$phase node-b" node-b "$pipeline_start_b"
  append_phase_summary \
    "$phase" \
    "$forward_mbps" \
    "$forward_retrans" \
    "$reverse_mbps" \
    "$reverse_retrans" \
    "$forward_load_mbps" \
    "$forward_load_retrans" \
    "$forward_load_ping_loss" \
    "$forward_load_ping_avg" \
    "$forward_load_ping_p95" \
    "$forward_load_ping_p99" \
    "$forward_load_ping_max" \
    "$reverse_load_mbps" \
    "$reverse_load_retrans" \
    "$reverse_load_ping_loss" \
    "$reverse_load_ping_avg" \
    "$reverse_load_ping_p95" \
    "$reverse_load_ping_p99" \
    "$reverse_load_ping_max" \
    "$post_ping_loss" \
    "$post_ping_avg" \
    "$post_ping_p95" \
    "$post_ping_p99" \
    "$post_ping_max" \
    "$direct_delta_a" \
    "$direct_delta_b" \
    "$pipeline_top_a" \
    "$pipeline_top_b" \
    "$pipeline_batch_a" \
    "$pipeline_batch_b" \
    "$pipeline_spread_a" \
    "$pipeline_spread_b" \
    "$pipeline_decrypt_batch_a" \
    "$pipeline_decrypt_batch_b" \
    "$pipeline_decrypt_spread_a" \
    "$pipeline_decrypt_spread_b" \
    "$pipeline_decrypt_turn_mix_a" \
    "$pipeline_decrypt_turn_mix_b" \
    "$pipeline_linux_bulk_a" \
    "$pipeline_linux_bulk_b" \
    "$pipeline_udp_send_a" \
    "$pipeline_udp_send_b" \
    "$pipeline_hard_a" \
    "$pipeline_hard_b" \
    "$pipeline_a" \
    "$pipeline_b"
}

stop_connects() {
  "${COMPOSE[@]}" exec -T node-a sh -c "pkill -9 nvpn 2>/dev/null; true" >/dev/null || true
  "${COMPOSE[@]}" exec -T node-b sh -c "pkill -9 nvpn 2>/dev/null; true" >/dev/null || true
  sleep 1
}

start_connects() {
  local rx_maint_fault_ms="$1"
  local extra_env="${2:-}"
  local fault_env=""
  local trace_env=""
  if [[ "$rx_maint_fault_ms" != "0" ]]; then
    fault_env="FIPS_FAULT_INJECT_RX_LOOP_SLOW_MAINTENANCE_MS='$rx_maint_fault_ms'"
  fi
  if [[ "$PIPELINE_TRACE" != "0" ]]; then
    trace_env="NVPN_PIPELINE_TRACE=1 NVPN_PIPELINE_INTERVAL_SECS='$PIPELINE_INTERVAL_SECS' FIPS_PERF_INTERVAL_SECS='$PIPELINE_INTERVAL_SECS'"
  fi

  "${COMPOSE[@]}" exec -d node-a sh -lc \
    "$fault_env $trace_env $EXTRA_ENV $extra_env NVPN_FIPS_NOSTR_DISCOVERY_POLICY='$FIPS_NOSTR_DISCOVERY_POLICY' nvpn connect > /tmp/connect.log 2>&1"
  "${COMPOSE[@]}" exec -d node-b sh -lc \
    "$fault_env $trace_env $EXTRA_ENV $extra_env NVPN_FIPS_NOSTR_DISCOVERY_POLICY='$FIPS_NOSTR_DISCOVERY_POLICY' nvpn connect > /tmp/connect.log 2>&1"
}

underlay_dev_for_peer_cmd='peer="$1"; ip route get "$peer" | sed -n "s/.* dev \([^ ]*\).*/\1/p" | head -n1'

apply_underlay_rate_limit() {
  local service="$1"
  local peer_ip="$2"
  local rate_mbit="$3"
  local burst_kb="$4"
  local latency_ms="$5"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    set -eu
    dev=\$(sh -c '$underlay_dev_for_peer_cmd' _ '$peer_ip')
    if [ -z \"\$dev\" ]; then
      echo \"nvpn+FIPS perf regression e2e failed: could not resolve underlay device for $peer_ip\" >&2
      exit 1
    fi
    tc qdisc replace dev \"\$dev\" root tbf rate '${rate_mbit}'mbit burst '${burst_kb}'kb latency '${latency_ms}'ms
  "
}

clear_underlay_rate_limit() {
  local service="$1"
  local peer_ip="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    dev=\$(sh -c '$underlay_dev_for_peer_cmd' _ '$peer_ip' 2>/dev/null || true)
    if [ -n \"\$dev\" ]; then
      tc qdisc del dev \"\$dev\" root 2>/dev/null || true
    fi
  " >/dev/null 2>&1 || true
}

run_constrained_underlay_phase() {
  if ! phase_enabled "constrained-underlay"; then
    echo "Skipping constrained-underlay phase because NVPN_PERF_PHASES=$PERF_PHASES"
    return
  fi

  if [[ "$CONSTRAINED_RATE_MBIT" == "0" ]]; then
    echo "Skipping constrained-underlay phase because NVPN_PERF_CONSTRAINED_RATE_MBIT=0"
    return
  fi

  # Clean Docker veth can hide sender queues that only appear once the underlay
  # is slower than the app. Cap egress so TCP collapse and ping starvation fail
  # in CI instead of waiting for a real LAN/macOS repro.
  echo "--- applying constrained underlay: ${CONSTRAINED_RATE_MBIT}Mbit burst=${CONSTRAINED_BURST_KB}KB latency=${CONSTRAINED_LATENCY_MS}ms ---"
  apply_underlay_rate_limit node-a 10.203.0.11 "$CONSTRAINED_RATE_MBIT" "$CONSTRAINED_BURST_KB" "$CONSTRAINED_LATENCY_MS"
  apply_underlay_rate_limit node-b 10.203.0.10 "$CONSTRAINED_RATE_MBIT" "$CONSTRAINED_BURST_KB" "$CONSTRAINED_LATENCY_MS"
  run_perf_phase \
    "constrained-underlay" \
    "$CONSTRAINED_MIN_TCP_MBIT" \
    "$CONSTRAINED_MIN_REVERSE_TCP_MBIT" \
    "$CONSTRAINED_MAX_PING_LOSS_PERCENT" \
    "$CONSTRAINED_MAX_PING_AVG_MS" \
    "$CONSTRAINED_MAX_PING_P95_MS" \
    "$CONSTRAINED_MAX_PING_P99_MS" \
    "$CONSTRAINED_MAX_PING_MAX_MS"
  clear_underlay_rate_limit node-a 10.203.0.11
  clear_underlay_rate_limit node-b 10.203.0.10
}

run_rx_maintenance_fault_phase() {
  if ! phase_enabled "rx-maintenance-fault"; then
    echo "Skipping rx-maintenance fault phase because NVPN_PERF_PHASES=$PERF_PHASES"
    return
  fi

  if [[ "$RX_MAINT_FAULT_MS" == "0" ]]; then
    echo "Skipping rx-maintenance fault phase because NVPN_PERF_RX_MAINT_FAULT_MS=0"
    return
  fi

  echo "--- restarting mesh with rx-loop maintenance fault: ${RX_MAINT_FAULT_MS}ms ---"
  stop_connects
  start_connects "$RX_MAINT_FAULT_MS"
  wait_for_mesh
  local saved_max_priority_queue_wait_ms="$MAX_PRIORITY_QUEUE_WAIT_MS"
  MAX_PRIORITY_QUEUE_WAIT_MS="$RX_MAINT_MAX_PRIORITY_QUEUE_WAIT_MS"
  run_perf_phase \
    "rx-maintenance-fault" \
    "$RX_MAINT_MIN_TCP_MBIT" \
    "$RX_MAINT_MIN_REVERSE_TCP_MBIT" \
    "$RX_MAINT_MAX_PING_LOSS_PERCENT" \
    "$RX_MAINT_MAX_PING_AVG_MS" \
    "$RX_MAINT_MAX_PING_P95_MS" \
    "$RX_MAINT_MAX_PING_P99_MS" \
    "$RX_MAINT_MAX_PING_MAX_MS" \
    "$RX_MAINT_MAX_PING_LOSS_PERCENT" \
    "$RX_MAINT_POST_MAX_PING_AVG_MS" \
    "$RX_MAINT_POST_MAX_PING_P95_MS" \
    "$RX_MAINT_POST_MAX_PING_P99_MS" \
    "$RX_MAINT_POST_MAX_PING_MAX_MS"
  MAX_PRIORITY_QUEUE_WAIT_MS="$saved_max_priority_queue_wait_ms"

  for service in node-a node-b; do
    if ! "${COMPOSE[@]}" exec -T "$service" sh -lc \
      "grep -q 'RX loop slow maintenance timed out' /tmp/connect.log"; then
      echo "nvpn+FIPS perf regression e2e failed: $service did not observe forced rx-loop maintenance timeout" >&2
      exit 1
    fi
  done
}

run_worker_queue_pressure_phase() {
  if ! phase_enabled "worker-queue-pressure"; then
    echo "Skipping worker-queue pressure phase because NVPN_PERF_PHASES=$PERF_PHASES"
    return
  fi

  if [[ "$WORKER_QUEUE_PRESSURE_CAP" == "0" ]]; then
    echo "Skipping worker-queue pressure phase because NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=0"
    return
  fi

  echo "--- restarting mesh with worker queue pressure: FIPS_WORKER_CHANNEL_CAP=${WORKER_QUEUE_PRESSURE_CAP} FIPS_DECRYPT_WORKER_CHANNEL_CAP=${WORKER_QUEUE_PRESSURE_DECRYPT_CAP} ---"
  stop_connects
  start_connects 0 "FIPS_WORKER_CHANNEL_CAP='$WORKER_QUEUE_PRESSURE_CAP' FIPS_DECRYPT_WORKER_CHANNEL_CAP='$WORKER_QUEUE_PRESSURE_DECRYPT_CAP'"
  wait_for_mesh
  run_perf_phase \
    "worker-queue-pressure" \
    "$WORKER_QUEUE_PRESSURE_MIN_TCP_MBIT" \
    "$WORKER_QUEUE_PRESSURE_MIN_REVERSE_TCP_MBIT" \
    "$WORKER_QUEUE_PRESSURE_MAX_PING_LOSS_PERCENT" \
    "$WORKER_QUEUE_PRESSURE_MAX_PING_AVG_MS" \
    "$WORKER_QUEUE_PRESSURE_MAX_PING_P95_MS" \
    "$WORKER_QUEUE_PRESSURE_MAX_PING_P99_MS" \
    "$WORKER_QUEUE_PRESSURE_MAX_PING_MAX_MS" \
    "$WORKER_QUEUE_PRESSURE_POST_MAX_PING_LOSS_PERCENT" \
    "$WORKER_QUEUE_PRESSURE_POST_MAX_PING_AVG_MS" \
    "$WORKER_QUEUE_PRESSURE_POST_MAX_PING_P95_MS" \
    "$WORKER_QUEUE_PRESSURE_POST_MAX_PING_P99_MS" \
    "$WORKER_QUEUE_PRESSURE_POST_MAX_PING_MAX_MS"
}

start_compose_services() {
  if is_true "$SKIP_BUILD"; then
    "${COMPOSE[@]}" up -d --no-build node-a node-b >/dev/null
  else
    BUILDKIT_PROGRESS=plain "${COMPOSE[@]}" build node-a node-b
    "${COMPOSE[@]}" up -d node-a node-b >/dev/null
  fi
}

main() {
  parse_args "$@"
  validate_perf_phases
  if ! docker_bench_validate_extra_env_assignments "$EXTRA_ENV"; then
    exit 2
  fi
  trap on_exit EXIT

cleanup
init_phase_summary
write_perf_metadata
write_host_snapshot "start"
start_compose_services
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"
if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "nvpn+FIPS perf regression e2e failed: unable to resolve node npubs from config" >&2
  exit 1
fi
if [[ "$ALICE_NPUB" == "$BOB_NPUB" ]]; then
  echo "nvpn+FIPS perf regression e2e failed: node-a and node-b generated the same nostr pubkey" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null

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
if [[ -z "$ALICE_TUNNEL_IP" || -z "$BOB_TUNNEL_IP" ]]; then
  echo "nvpn+FIPS perf regression e2e failed: unable to derive both tunnel IPs" >&2
  exit 1
fi
if [[ "$ALICE_TUNNEL_IP" == "$BOB_TUNNEL_IP" ]]; then
  echo "nvpn+FIPS perf regression e2e failed: derived duplicate tunnel IP $ALICE_TUNNEL_IP for distinct peers on network_id=$NETWORK_ID" >&2
  echo "  node-a pubkey prefix: ${ALICE_NPUB:0:16}" >&2
  echo "  node-b pubkey prefix: ${BOB_NPUB:0:16}" >&2
  echo "  retry with NVPN_PERF_NETWORK_ID set to a unique value or investigate tunnel IP allocation" >&2
  exit 1
fi

start_connects 0
install_direct_underlay_counter node-a 10.203.0.11
install_direct_underlay_counter node-b 10.203.0.10

wait_for_mesh

baseline_direct_a="$(direct_underlay_bytes node-a)"
baseline_direct_b="$(direct_underlay_bytes node-b)"
if ! "${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$BOB_TUNNEL_IP" >/dev/null; then
  echo "nvpn+FIPS perf regression e2e failed: baseline tunnel ping failed" >&2
  exit 1
fi
assert_direct_counter_advanced node-a "$baseline_direct_a" "baseline node-a"
assert_direct_counter_advanced node-b "$baseline_direct_b" "baseline node-b"
docker_bench_start_cpu_stress

echo "alice tunnel ip: $ALICE_TUNNEL_IP"
echo "bob   tunnel ip: $BOB_TUNNEL_IP"
if phase_enabled "unimpaired-underlay"; then
  run_perf_phase \
    "unimpaired-underlay" \
    "$MIN_TCP_MBIT" \
    "$MIN_REVERSE_TCP_MBIT" \
    "$MAX_PING_LOSS_PERCENT" \
    "$MAX_PING_AVG_MS" \
    "$MAX_PING_P95_MS" \
    "$MAX_PING_P99_MS" \
    "$MAX_PING_MAX_MS"
else
  echo "Skipping unimpaired-underlay phase because NVPN_PERF_PHASES=$PERF_PHASES"
fi
run_constrained_underlay_phase
run_worker_queue_pressure_phase
run_rx_maintenance_fault_phase

echo "nvpn+FIPS perf regression docker e2e passed: throughput stayed above floor and pings did not wedge under or after TCP load"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
