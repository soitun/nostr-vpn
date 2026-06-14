#!/usr/bin/env bash
# Host/VM pair FIPS dataplane baseline/soak for an already configured local
# daemon and one SSH-reachable Linux/VM peer.
#
# This is intentionally environment-driven so local hostnames, usernames, IPs,
# and config paths stay out of committed files. It does not install daemons or
# edit configs; use the dev deployment/setup scripts first when needed.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

REMOTE_SSH="${NVPN_HOST_PAIR_SSH:-${1:-}}"
REMOTE_SSH_PORT="${NVPN_HOST_PAIR_SSH_PORT:-}"
REMOTE_SSH_CONNECT_TIMEOUT="${NVPN_HOST_PAIR_SSH_CONNECT_TIMEOUT:-10}"
LOCAL_NVPN="${NVPN_HOST_PAIR_LOCAL_NVPN:-nvpn}"
REMOTE_NVPN="${NVPN_HOST_PAIR_REMOTE_NVPN:-nvpn}"
LOCAL_NVPN_COMMAND="${NVPN_HOST_PAIR_LOCAL_NVPN_COMMAND:-}"
REMOTE_NVPN_COMMAND="${NVPN_HOST_PAIR_REMOTE_NVPN_COMMAND:-}"
LOCAL_CONFIG="${NVPN_HOST_PAIR_LOCAL_CONFIG:-}"
REMOTE_CONFIG="${NVPN_HOST_PAIR_REMOTE_CONFIG:-}"
LOCAL_DAEMON_LOG="${NVPN_HOST_PAIR_LOCAL_DAEMON_LOG:-}"
REMOTE_DAEMON_LOG="${NVPN_HOST_PAIR_REMOTE_DAEMON_LOG:-}"
LOCAL_LOG_READ_COMMAND="${NVPN_HOST_PAIR_LOCAL_LOG_READ_COMMAND:-}"
REMOTE_LOG_READ_COMMAND="${NVPN_HOST_PAIR_REMOTE_LOG_READ_COMMAND:-}"
REQUIRE_PIPELINE_LOGS="${NVPN_HOST_PAIR_REQUIRE_PIPELINE_LOGS:-0}"
OUTPUT_DIR="${NVPN_HOST_PAIR_OUTPUT_DIR:-$ROOT_DIR/artifacts/fips-host-pair/$(date -u +%Y%m%dT%H%M%SZ)}"
PREFLIGHT="${NVPN_HOST_PAIR_PREFLIGHT:-0}"
DURATION_SECS="${NVPN_HOST_PAIR_DURATION_SECS:-300}"
INTERVAL_SECS="${NVPN_HOST_PAIR_INTERVAL_SECS:-60}"
PING_COUNT="${NVPN_HOST_PAIR_PING_COUNT:-20}"
PING_INTERVAL="${NVPN_HOST_PAIR_PING_INTERVAL:-0.2}"
IPERF_DURATION="${NVPN_HOST_PAIR_IPERF_DURATION_SECS:-5}"
REQUIRE_IPERF="${NVPN_HOST_PAIR_REQUIRE_IPERF:-1}"
ALLOW_NON_DIRECT="${NVPN_HOST_PAIR_ALLOW_NON_DIRECT:-0}"
ALLOW_QUEUE_EVENTS="${NVPN_HOST_PAIR_ALLOW_QUEUE_EVENTS:-0}"
ALLOW_QUEUE_WAIT="${NVPN_HOST_PAIR_ALLOW_QUEUE_WAIT:-$ALLOW_QUEUE_EVENTS}"
MAX_PING_LOSS_PERCENT="${NVPN_HOST_PAIR_MAX_PING_LOSS_PERCENT:-5}"
MAX_PING_AVG_MS="${NVPN_HOST_PAIR_MAX_PING_AVG_MS:-250}"
MAX_PING_P95_MS="${NVPN_HOST_PAIR_MAX_PING_P95_MS:-500}"
MAX_PING_P99_MS="${NVPN_HOST_PAIR_MAX_PING_P99_MS:-750}"
MAX_PING_MAX_MS="${NVPN_HOST_PAIR_MAX_PING_MAX_MS:-1000}"
MAX_PING_AVG_DRIFT_MS="${NVPN_HOST_PAIR_MAX_PING_AVG_DRIFT_MS:-25}"
MAX_PING_AVG_DRIFT_FACTOR="${NVPN_HOST_PAIR_MAX_PING_AVG_DRIFT_FACTOR:-10}"
MAX_SRTT_MS="${NVPN_HOST_PAIR_MAX_SRTT_MS:-1000}"
MAX_SRTT_AGE_MS="${NVPN_HOST_PAIR_MAX_SRTT_AGE_MS:-120000}"
MAX_SRTT_DRIFT_MS="${NVPN_HOST_PAIR_MAX_SRTT_DRIFT_MS:-50}"
MAX_SRTT_DRIFT_FACTOR="${NVPN_HOST_PAIR_MAX_SRTT_DRIFT_FACTOR:-10}"
MAX_CPU_PERCENT="${NVPN_HOST_PAIR_MAX_CPU_PERCENT:-250}"
MAX_PIPELINE_QUEUE_WAIT_P95_MS="${NVPN_HOST_PAIR_MAX_PIPELINE_QUEUE_WAIT_P95_MS:-50}"
MAX_PIPELINE_QUEUE_WAIT_P99_MS="${NVPN_HOST_PAIR_MAX_PIPELINE_QUEUE_WAIT_P99_MS:-100}"
MAX_PRIORITY_QUEUE_WAIT_MS="${NVPN_HOST_PAIR_MAX_PRIORITY_QUEUE_WAIT_MS:-50}"
FAIL_ON_PRIORITY_HARD_EVENTS="${NVPN_HOST_PAIR_FAIL_ON_PRIORITY_HARD_EVENTS:-1}"
MIN_IPERF_MBPS="${NVPN_HOST_PAIR_MIN_IPERF_MBPS:-0.001}"
MAX_CONSECUTIVE_IPERF_COLLAPSES="${NVPN_HOST_PAIR_MAX_CONSECUTIVE_IPERF_COLLAPSES:-2}"
MAX_CONSECUTIVE_REKEY_SAMPLES="${NVPN_HOST_PAIR_MAX_CONSECUTIVE_REKEY_SAMPLES:-2}"
MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES="${NVPN_HOST_PAIR_MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES:-${NVPN_HOST_PAIR_MAX_CONSECUTIVE_DIRECT_PROBE_SAMPLES:-2}}"
MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES="${NVPN_HOST_PAIR_MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES:-2}"
MAX_FIPS_LAST_SEEN_AGE_SECS="${NVPN_HOST_PAIR_MAX_FIPS_LAST_SEEN_AGE_SECS:-180}"
MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS="${NVPN_HOST_PAIR_MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS:-$MAX_FIPS_LAST_SEEN_AGE_SECS}"
MAX_FIPS_DATA_LAST_SEEN_AGE_SECS="${NVPN_HOST_PAIR_MAX_FIPS_DATA_LAST_SEEN_AGE_SECS:-$MAX_FIPS_LAST_SEEN_AGE_SECS}"
MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS="${NVPN_HOST_PAIR_MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS:-5}"
CPU_STRESS="${NVPN_HOST_PAIR_CPU_STRESS:-0}"
CPU_STRESS_SIDES="${NVPN_HOST_PAIR_CPU_STRESS_SIDES:-remote}"
CPU_STRESS_WORKERS="${NVPN_HOST_PAIR_CPU_STRESS_WORKERS:-auto}"
CPU_STRESS_SETTLE_SECS="${NVPN_HOST_PAIR_CPU_STRESS_SETTLE_SECS:-2}"

# Peer selectors. If omitted, each side must expose exactly one daemon peer.
LOCAL_PEER="${NVPN_HOST_PAIR_LOCAL_PEER:-}"     # remote participant pubkey as seen locally
REMOTE_PEER="${NVPN_HOST_PAIR_REMOTE_PEER:-}"   # local participant pubkey as seen remotely

# Optional strict direct-path expectations. When set, the peer's
# fips_transport_addr must start with this underlay IP unless ALLOW_NON_DIRECT=1.
EXPECTED_REMOTE_UNDERLAY_IP="${NVPN_HOST_PAIR_EXPECTED_REMOTE_UNDERLAY_IP:-}" # local status -> remote peer
EXPECTED_LOCAL_UNDERLAY_IP="${NVPN_HOST_PAIR_EXPECTED_LOCAL_UNDERLAY_IP:-}"   # remote status -> local peer

SSH_OPTS=(-o BatchMode=yes -o "ConnectTimeout=$REMOTE_SSH_CONNECT_TIMEOUT" -o StrictHostKeyChecking=accept-new)
if [[ -n "$REMOTE_SSH_PORT" ]]; then
  SSH_OPTS=(-p "$REMOTE_SSH_PORT" "${SSH_OPTS[@]}")
fi

SAMPLES=""
SUMMARY=""
FAILURE_REPORT=""
CURRENT_ITERATION=0
CURRENT_TIMESTAMP=""
LAST_LOCAL_PING_LOG=""
LAST_REMOTE_PING_LOG=""
LAST_FORWARD_IPERF_JSON=""
LAST_REVERSE_IPERF_JSON=""
PING_FORWARD_LOSS=""
PING_FORWARD_AVG=""
PING_FORWARD_P95=""
PING_FORWARD_P99=""
PING_FORWARD_MAX=""
PING_REVERSE_LOSS=""
PING_REVERSE_AVG=""
PING_REVERSE_P95=""
PING_REVERSE_P99=""
PING_REVERSE_MAX=""
IPERF_FORWARD_MBPS=""
IPERF_FORWARD_RETRANS=""
IPERF_REVERSE_MBPS=""
IPERF_REVERSE_RETRANS=""
LOCAL_TRANSPORT_ADDR=""
REMOTE_TRANSPORT_ADDR=""
LOCAL_SRTT=""
REMOTE_SRTT=""
LOCAL_SRTT_AGE_MS=""
REMOTE_SRTT_AGE_MS=""
LOCAL_BYTES_SENT=""
LOCAL_BYTES_RECV=""
REMOTE_BYTES_SENT=""
REMOTE_BYTES_RECV=""
LOCAL_LAST_MESH_SEEN_AT=""
LOCAL_LAST_FIPS_SEEN_AT=""
LOCAL_LAST_FIPS_SEEN_AGE_SECS=""
LOCAL_LAST_FIPS_CONTROL_SEEN_AT=""
LOCAL_LAST_FIPS_CONTROL_SEEN_AGE_SECS=""
LOCAL_LAST_FIPS_DATA_SEEN_AT=""
LOCAL_LAST_FIPS_DATA_SEEN_AGE_SECS=""
LOCAL_LAST_HANDSHAKE_AT=""
REMOTE_LAST_MESH_SEEN_AT=""
REMOTE_LAST_FIPS_SEEN_AT=""
REMOTE_LAST_FIPS_SEEN_AGE_SECS=""
REMOTE_LAST_FIPS_CONTROL_SEEN_AT=""
REMOTE_LAST_FIPS_CONTROL_SEEN_AGE_SECS=""
REMOTE_LAST_FIPS_DATA_SEEN_AT=""
REMOTE_LAST_FIPS_DATA_SEEN_AGE_SECS=""
REMOTE_LAST_HANDSHAKE_AT=""
LOCAL_REKEY_IN_PROGRESS=""
LOCAL_REKEY_DRAINING=""
LOCAL_CURRENT_K_BIT=""
LOCAL_DIRECT_PROBE_PENDING=""
LOCAL_DIRECT_PROBE_AFTER_MS=""
LOCAL_DIRECT_PROBE_RETRY_COUNT=""
LOCAL_DIRECT_PROBE_AUTO_RECONNECT=""
LOCAL_DIRECT_PROBE_EXPIRES_AT_MS=""
LOCAL_NOSTR_TRAVERSAL_FAILURES=""
LOCAL_NOSTR_TRAVERSAL_IN_COOLDOWN=""
LOCAL_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS=""
LOCAL_NOSTR_TRAVERSAL_LAST_SKEW_MS=""
REMOTE_REKEY_IN_PROGRESS=""
REMOTE_REKEY_DRAINING=""
REMOTE_CURRENT_K_BIT=""
REMOTE_DIRECT_PROBE_PENDING=""
REMOTE_DIRECT_PROBE_AFTER_MS=""
REMOTE_DIRECT_PROBE_RETRY_COUNT=""
REMOTE_DIRECT_PROBE_AUTO_RECONNECT=""
REMOTE_DIRECT_PROBE_EXPIRES_AT_MS=""
REMOTE_NOSTR_TRAVERSAL_FAILURES=""
REMOTE_NOSTR_TRAVERSAL_IN_COOLDOWN=""
REMOTE_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS=""
REMOTE_NOSTR_TRAVERSAL_LAST_SKEW_MS=""
LOCAL_REKEY_STUCK_COUNT=0
REMOTE_REKEY_STUCK_COUNT=0
LOCAL_DIRECT_PROBE_PENDING_COUNT=0
REMOTE_DIRECT_PROBE_PENDING_COUNT=0
LOCAL_DIRECT_PROBE_OVERDUE_COUNT=0
REMOTE_DIRECT_PROBE_OVERDUE_COUNT=0
LOCAL_CPU=""
REMOTE_CPU=""
FIPS_PIPELINE_LOCAL=""
FIPS_PIPELINE_REMOTE=""
NVPN_PIPELINE_LOCAL=""
NVPN_PIPELINE_REMOTE=""
FIPS_PIPELINE_LOCAL_COUNT=""
FIPS_PIPELINE_REMOTE_COUNT=""
NVPN_PIPELINE_LOCAL_COUNT=""
NVPN_PIPELINE_REMOTE_COUNT=""
FIPS_PIPELINE_LOCAL_QUEUE_WAIT="{}"
FIPS_PIPELINE_REMOTE_QUEUE_WAIT="{}"
NVPN_PIPELINE_LOCAL_QUEUE_WAIT="{}"
NVPN_PIPELINE_REMOTE_QUEUE_WAIT="{}"
LOCAL_CPU_STRESS_PID_FILE=""
REMOTE_CPU_STRESS_PID_FILE=""
LOCAL_CPU_STRESS_WORKERS_STARTED="0"
REMOTE_CPU_STRESS_WORKERS_STARTED="0"
PREFLIGHT_ROWS=()
PREFLIGHT_RESULT=0

die() {
  printf 'fips host-pair soak failed: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

q() {
  printf '%q' "$1"
}

tsv_escape() {
  local value="$1"
  value="${value//$'\t'/ }"
  value="${value//$'\n'/ }"
  printf '%s' "$value"
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

cleanup_remote_iperf() {
  [[ -n "$REMOTE_SSH" ]] || return 0
  remote_sh "pkill -9 iperf3 >/dev/null 2>&1 || true" >/dev/null 2>&1 || true
}

write_failure_report() {
  local rc="$1"
  [[ -n "$OUTPUT_DIR" ]] || return 0
  command -v jq >/dev/null 2>&1 || return 0
  mkdir -p "$OUTPUT_DIR" 2>/dev/null || return 0
  FAILURE_REPORT="$OUTPUT_DIR/failure.json"
  jq -nc \
    --arg failed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg output_dir "$OUTPUT_DIR" \
    --arg samples "$SAMPLES" \
    --arg summary "$SUMMARY" \
    --arg local_ping_log "$LAST_LOCAL_PING_LOG" \
    --arg remote_ping_log "$LAST_REMOTE_PING_LOG" \
    --arg forward_iperf_json "$LAST_FORWARD_IPERF_JSON" \
    --arg reverse_iperf_json "$LAST_REVERSE_IPERF_JSON" \
    --arg timestamp "$CURRENT_TIMESTAMP" \
    --arg ping_forward_loss "$PING_FORWARD_LOSS" \
    --arg ping_forward_avg "$PING_FORWARD_AVG" \
    --arg ping_forward_p95 "$PING_FORWARD_P95" \
    --arg ping_forward_p99 "$PING_FORWARD_P99" \
    --arg ping_forward_max "$PING_FORWARD_MAX" \
    --arg ping_reverse_loss "$PING_REVERSE_LOSS" \
    --arg ping_reverse_avg "$PING_REVERSE_AVG" \
    --arg ping_reverse_p95 "$PING_REVERSE_P95" \
    --arg ping_reverse_p99 "$PING_REVERSE_P99" \
    --arg ping_reverse_max "$PING_REVERSE_MAX" \
    --arg iperf_forward_mbps "$IPERF_FORWARD_MBPS" \
    --arg iperf_forward_retrans "$IPERF_FORWARD_RETRANS" \
    --arg iperf_reverse_mbps "$IPERF_REVERSE_MBPS" \
    --arg iperf_reverse_retrans "$IPERF_REVERSE_RETRANS" \
    --arg local_srtt "$LOCAL_SRTT" \
    --arg remote_srtt "$REMOTE_SRTT" \
    --arg local_srtt_age_ms "$LOCAL_SRTT_AGE_MS" \
    --arg remote_srtt_age_ms "$REMOTE_SRTT_AGE_MS" \
    --arg local_bytes_sent "$LOCAL_BYTES_SENT" \
    --arg local_bytes_recv "$LOCAL_BYTES_RECV" \
    --arg remote_bytes_sent "$REMOTE_BYTES_SENT" \
    --arg remote_bytes_recv "$REMOTE_BYTES_RECV" \
    --arg local_last_mesh_seen_at "$LOCAL_LAST_MESH_SEEN_AT" \
    --arg local_last_fips_seen_at "$LOCAL_LAST_FIPS_SEEN_AT" \
    --arg local_last_fips_seen_age_secs "$LOCAL_LAST_FIPS_SEEN_AGE_SECS" \
    --arg local_last_fips_control_seen_at "$LOCAL_LAST_FIPS_CONTROL_SEEN_AT" \
    --arg local_last_fips_control_seen_age_secs "$LOCAL_LAST_FIPS_CONTROL_SEEN_AGE_SECS" \
    --arg local_last_fips_data_seen_at "$LOCAL_LAST_FIPS_DATA_SEEN_AT" \
    --arg local_last_fips_data_seen_age_secs "$LOCAL_LAST_FIPS_DATA_SEEN_AGE_SECS" \
    --arg local_last_handshake_at "$LOCAL_LAST_HANDSHAKE_AT" \
    --arg remote_last_mesh_seen_at "$REMOTE_LAST_MESH_SEEN_AT" \
    --arg remote_last_fips_seen_at "$REMOTE_LAST_FIPS_SEEN_AT" \
    --arg remote_last_fips_seen_age_secs "$REMOTE_LAST_FIPS_SEEN_AGE_SECS" \
    --arg remote_last_fips_control_seen_at "$REMOTE_LAST_FIPS_CONTROL_SEEN_AT" \
    --arg remote_last_fips_control_seen_age_secs "$REMOTE_LAST_FIPS_CONTROL_SEEN_AGE_SECS" \
    --arg remote_last_fips_data_seen_at "$REMOTE_LAST_FIPS_DATA_SEEN_AT" \
    --arg remote_last_fips_data_seen_age_secs "$REMOTE_LAST_FIPS_DATA_SEEN_AGE_SECS" \
    --arg remote_last_handshake_at "$REMOTE_LAST_HANDSHAKE_AT" \
    --arg local_rekey_in_progress "$LOCAL_REKEY_IN_PROGRESS" \
    --arg local_rekey_draining "$LOCAL_REKEY_DRAINING" \
    --arg local_current_k_bit "$LOCAL_CURRENT_K_BIT" \
    --arg local_direct_probe_pending "$LOCAL_DIRECT_PROBE_PENDING" \
    --arg local_direct_probe_after_ms "$LOCAL_DIRECT_PROBE_AFTER_MS" \
    --arg local_direct_probe_retry_count "$LOCAL_DIRECT_PROBE_RETRY_COUNT" \
    --arg local_direct_probe_auto_reconnect "$LOCAL_DIRECT_PROBE_AUTO_RECONNECT" \
    --arg local_direct_probe_expires_at_ms "$LOCAL_DIRECT_PROBE_EXPIRES_AT_MS" \
    --arg local_direct_probe_pending_count "$LOCAL_DIRECT_PROBE_PENDING_COUNT" \
    --arg local_direct_probe_overdue_count "$LOCAL_DIRECT_PROBE_OVERDUE_COUNT" \
    --arg local_nostr_traversal_failures "$LOCAL_NOSTR_TRAVERSAL_FAILURES" \
    --arg local_nostr_traversal_in_cooldown "$LOCAL_NOSTR_TRAVERSAL_IN_COOLDOWN" \
    --arg local_nostr_traversal_cooldown_until_ms "$LOCAL_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS" \
    --arg local_nostr_traversal_last_skew_ms "$LOCAL_NOSTR_TRAVERSAL_LAST_SKEW_MS" \
    --arg local_rekey_stuck_count "$LOCAL_REKEY_STUCK_COUNT" \
    --arg remote_rekey_in_progress "$REMOTE_REKEY_IN_PROGRESS" \
    --arg remote_rekey_draining "$REMOTE_REKEY_DRAINING" \
    --arg remote_current_k_bit "$REMOTE_CURRENT_K_BIT" \
    --arg remote_direct_probe_pending "$REMOTE_DIRECT_PROBE_PENDING" \
    --arg remote_direct_probe_after_ms "$REMOTE_DIRECT_PROBE_AFTER_MS" \
    --arg remote_direct_probe_retry_count "$REMOTE_DIRECT_PROBE_RETRY_COUNT" \
    --arg remote_direct_probe_auto_reconnect "$REMOTE_DIRECT_PROBE_AUTO_RECONNECT" \
    --arg remote_direct_probe_expires_at_ms "$REMOTE_DIRECT_PROBE_EXPIRES_AT_MS" \
    --arg remote_direct_probe_pending_count "$REMOTE_DIRECT_PROBE_PENDING_COUNT" \
    --arg remote_direct_probe_overdue_count "$REMOTE_DIRECT_PROBE_OVERDUE_COUNT" \
    --arg remote_nostr_traversal_failures "$REMOTE_NOSTR_TRAVERSAL_FAILURES" \
    --arg remote_nostr_traversal_in_cooldown "$REMOTE_NOSTR_TRAVERSAL_IN_COOLDOWN" \
    --arg remote_nostr_traversal_cooldown_until_ms "$REMOTE_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS" \
    --arg remote_nostr_traversal_last_skew_ms "$REMOTE_NOSTR_TRAVERSAL_LAST_SKEW_MS" \
    --arg remote_rekey_stuck_count "$REMOTE_REKEY_STUCK_COUNT" \
    --arg local_cpu "$LOCAL_CPU" \
    --arg remote_cpu "$REMOTE_CPU" \
    --arg fips_pipeline_local_count "$FIPS_PIPELINE_LOCAL_COUNT" \
    --arg fips_pipeline_remote_count "$FIPS_PIPELINE_REMOTE_COUNT" \
    --arg nvpn_pipeline_local_count "$NVPN_PIPELINE_LOCAL_COUNT" \
    --arg nvpn_pipeline_remote_count "$NVPN_PIPELINE_REMOTE_COUNT" \
    --argjson fips_pipeline_local_queue_wait "$FIPS_PIPELINE_LOCAL_QUEUE_WAIT" \
    --argjson fips_pipeline_remote_queue_wait "$FIPS_PIPELINE_REMOTE_QUEUE_WAIT" \
    --argjson nvpn_pipeline_local_queue_wait "$NVPN_PIPELINE_LOCAL_QUEUE_WAIT" \
    --argjson nvpn_pipeline_remote_queue_wait "$NVPN_PIPELINE_REMOTE_QUEUE_WAIT" \
    --arg max_ping_loss "$MAX_PING_LOSS_PERCENT" \
    --arg max_ping_avg "$MAX_PING_AVG_MS" \
    --arg max_ping_p95 "$MAX_PING_P95_MS" \
    --arg max_ping_p99 "$MAX_PING_P99_MS" \
    --arg max_ping_max "$MAX_PING_MAX_MS" \
    --arg max_srtt "$MAX_SRTT_MS" \
    --arg max_srtt_age_ms "$MAX_SRTT_AGE_MS" \
    --arg max_pipeline_queue_wait_p95 "$MAX_PIPELINE_QUEUE_WAIT_P95_MS" \
    --arg max_pipeline_queue_wait_p99 "$MAX_PIPELINE_QUEUE_WAIT_P99_MS" \
    --arg max_priority_queue_wait_ms "$MAX_PRIORITY_QUEUE_WAIT_MS" \
    --arg fail_on_priority_hard_events "$(is_true "$FAIL_ON_PRIORITY_HARD_EVENTS" && printf true || printf false)" \
    --arg max_consecutive_rekey_samples "$MAX_CONSECUTIVE_REKEY_SAMPLES" \
    --arg max_consecutive_direct_probe_overdue_samples "$MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES" \
    --arg max_fips_last_seen_age_secs "$MAX_FIPS_LAST_SEEN_AGE_SECS" \
    --arg max_fips_control_last_seen_age_secs "$MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS" \
    --arg max_fips_data_last_seen_age_secs "$MAX_FIPS_DATA_LAST_SEEN_AGE_SECS" \
    --arg max_fips_last_seen_future_skew_secs "$MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS" \
    --arg min_iperf "$MIN_IPERF_MBPS" \
    --argjson exit_status "$rc" \
    --argjson iteration "${CURRENT_ITERATION:-0}" \
    --argjson forward_collapse_count "${IPERF_FORWARD_COLLAPSE_COUNT:-0}" \
    --argjson reverse_collapse_count "${IPERF_REVERSE_COLLAPSE_COUNT:-0}" \
    '
    def num($v): if $v == "" or $v == "null" then null else ($v | tonumber) end;
    def bool($v):
      if $v == "" or $v == "null" then null
      elif $v == "true" then true
      elif $v == "false" then false
      else null
      end;
    {
      failed_at: $failed_at,
      exit_status: $exit_status,
      sample: {
        iteration: $iteration,
        timestamp: $timestamp
      },
      artifacts: {
        output_dir: $output_dir,
        samples: $samples,
        summary: $summary,
        local_ping_log: $local_ping_log,
        remote_ping_log: $remote_ping_log,
        forward_iperf_json: $forward_iperf_json,
        reverse_iperf_json: $reverse_iperf_json
      },
      tcp_collapse: {
        forward_count: $forward_collapse_count,
        reverse_count: $reverse_collapse_count
      },
      metrics: {
        ping: {
          local_to_remote: {
            loss_percent: num($ping_forward_loss),
            avg_ms: num($ping_forward_avg),
            p95_ms: num($ping_forward_p95),
            p99_ms: num($ping_forward_p99),
            max_ms: num($ping_forward_max)
          },
          remote_to_local: {
            loss_percent: num($ping_reverse_loss),
            avg_ms: num($ping_reverse_avg),
            p95_ms: num($ping_reverse_p95),
            p99_ms: num($ping_reverse_p99),
            max_ms: num($ping_reverse_max)
          }
        },
        iperf: {
          forward_mbps: num($iperf_forward_mbps),
          forward_retrans: num($iperf_forward_retrans),
          reverse_mbps: num($iperf_reverse_mbps),
          reverse_retrans: num($iperf_reverse_retrans)
        },
        fips: {
          local_srtt_ms: num($local_srtt),
          remote_srtt_ms: num($remote_srtt),
          local_srtt_age_ms: num($local_srtt_age_ms),
          remote_srtt_age_ms: num($remote_srtt_age_ms),
          local_bytes_sent: num($local_bytes_sent),
          local_bytes_recv: num($local_bytes_recv),
          remote_bytes_sent: num($remote_bytes_sent),
          remote_bytes_recv: num($remote_bytes_recv),
          local_last_mesh_seen_at: num($local_last_mesh_seen_at),
          local_last_fips_seen_at: num($local_last_fips_seen_at),
          local_last_fips_seen_age_secs: num($local_last_fips_seen_age_secs),
          local_last_fips_control_seen_at: num($local_last_fips_control_seen_at),
          local_last_fips_control_seen_age_secs: num($local_last_fips_control_seen_age_secs),
          local_last_fips_data_seen_at: num($local_last_fips_data_seen_at),
          local_last_fips_data_seen_age_secs: num($local_last_fips_data_seen_age_secs),
          local_last_handshake_at: num($local_last_handshake_at),
          remote_last_mesh_seen_at: num($remote_last_mesh_seen_at),
          remote_last_fips_seen_at: num($remote_last_fips_seen_at),
          remote_last_fips_seen_age_secs: num($remote_last_fips_seen_age_secs),
          remote_last_fips_control_seen_at: num($remote_last_fips_control_seen_at),
          remote_last_fips_control_seen_age_secs: num($remote_last_fips_control_seen_age_secs),
          remote_last_fips_data_seen_at: num($remote_last_fips_data_seen_at),
          remote_last_fips_data_seen_age_secs: num($remote_last_fips_data_seen_age_secs),
          remote_last_handshake_at: num($remote_last_handshake_at),
          local_rekey_in_progress: bool($local_rekey_in_progress),
          local_rekey_draining: bool($local_rekey_draining),
          local_current_k_bit: bool($local_current_k_bit),
          local_direct_probe_pending: bool($local_direct_probe_pending),
          local_direct_probe_after_ms: num($local_direct_probe_after_ms),
          local_direct_probe_retry_count: num($local_direct_probe_retry_count),
          local_direct_probe_auto_reconnect: bool($local_direct_probe_auto_reconnect),
          local_direct_probe_expires_at_ms: num($local_direct_probe_expires_at_ms),
          local_direct_probe_pending_count: num($local_direct_probe_pending_count),
          local_direct_probe_overdue_count: num($local_direct_probe_overdue_count),
          local_nostr_traversal_failures: num($local_nostr_traversal_failures),
          local_nostr_traversal_in_cooldown: bool($local_nostr_traversal_in_cooldown),
          local_nostr_traversal_cooldown_until_ms: num($local_nostr_traversal_cooldown_until_ms),
          local_nostr_traversal_last_skew_ms: num($local_nostr_traversal_last_skew_ms),
          remote_rekey_in_progress: bool($remote_rekey_in_progress),
          remote_rekey_draining: bool($remote_rekey_draining),
          remote_current_k_bit: bool($remote_current_k_bit),
          remote_direct_probe_pending: bool($remote_direct_probe_pending),
          remote_direct_probe_after_ms: num($remote_direct_probe_after_ms),
          remote_direct_probe_retry_count: num($remote_direct_probe_retry_count),
          remote_direct_probe_auto_reconnect: bool($remote_direct_probe_auto_reconnect),
          remote_direct_probe_expires_at_ms: num($remote_direct_probe_expires_at_ms),
          remote_direct_probe_pending_count: num($remote_direct_probe_pending_count),
          remote_direct_probe_overdue_count: num($remote_direct_probe_overdue_count),
          remote_nostr_traversal_failures: num($remote_nostr_traversal_failures),
          remote_nostr_traversal_in_cooldown: bool($remote_nostr_traversal_in_cooldown),
          remote_nostr_traversal_cooldown_until_ms: num($remote_nostr_traversal_cooldown_until_ms),
          remote_nostr_traversal_last_skew_ms: num($remote_nostr_traversal_last_skew_ms),
          local_rekey_stuck_count: ($local_rekey_stuck_count | tonumber),
          remote_rekey_stuck_count: ($remote_rekey_stuck_count | tonumber)
        },
        daemon_cpu: {
          local_percent: num($local_cpu),
          remote_percent: num($remote_cpu)
        },
        pipeline_queue_wait_ms: {
          fips_local: $fips_pipeline_local_queue_wait,
          fips_remote: $fips_pipeline_remote_queue_wait,
          nvpn_local: $nvpn_pipeline_local_queue_wait,
          nvpn_remote: $nvpn_pipeline_remote_queue_wait
        },
        pipeline_line_counts: {
          fips_local: num($fips_pipeline_local_count),
          fips_remote: num($fips_pipeline_remote_count),
          nvpn_local: num($nvpn_pipeline_local_count),
          nvpn_remote: num($nvpn_pipeline_remote_count)
        }
      },
      thresholds: {
        max_ping_loss_percent: num($max_ping_loss),
        max_ping_avg_ms: num($max_ping_avg),
        max_ping_p95_ms: num($max_ping_p95),
        max_ping_p99_ms: num($max_ping_p99),
        max_ping_max_ms: num($max_ping_max),
        max_srtt_ms: num($max_srtt),
        max_srtt_age_ms: num($max_srtt_age_ms),
        max_pipeline_queue_wait_p95_ms: num($max_pipeline_queue_wait_p95),
        max_pipeline_queue_wait_p99_ms: num($max_pipeline_queue_wait_p99),
        max_priority_queue_wait_ms: num($max_priority_queue_wait_ms),
        fail_on_priority_hard_events: ($fail_on_priority_hard_events == "true"),
        max_consecutive_rekey_samples: num($max_consecutive_rekey_samples),
        max_consecutive_direct_probe_overdue_samples: num($max_consecutive_direct_probe_overdue_samples),
        max_fips_last_seen_age_secs: num($max_fips_last_seen_age_secs),
        max_fips_control_last_seen_age_secs: num($max_fips_control_last_seen_age_secs),
        max_fips_data_last_seen_age_secs: num($max_fips_data_last_seen_age_secs),
        max_fips_last_seen_future_skew_secs: num($max_fips_last_seen_future_skew_secs),
        min_iperf_mbps: num($min_iperf)
      }
    }' >"$FAILURE_REPORT"
}

on_exit() {
  local rc=$?
  if (( rc != 0 )); then
    write_failure_report "$rc"
  fi
  stop_cpu_stress
  cleanup_remote_iperf
  if (( rc != 0 )) && [[ -n "$FAILURE_REPORT" && -f "$FAILURE_REPORT" ]]; then
    printf 'fips host-pair failure report: %s\n' "$FAILURE_REPORT" >&2
  fi
}

local_status() {
  if [[ -n "$LOCAL_NVPN_COMMAND" ]]; then
    local cmd="$LOCAL_NVPN_COMMAND status --json --discover-secs 0"
    if [[ -n "$LOCAL_CONFIG" ]]; then
      cmd+=" --config $(q "$LOCAL_CONFIG")"
    fi
    bash -lc "$cmd" | tr -d '\r'
  else
    local cmd=("$LOCAL_NVPN" status --json --discover-secs 0)
    if [[ -n "$LOCAL_CONFIG" ]]; then
      cmd+=(--config "$LOCAL_CONFIG")
    fi
    "${cmd[@]}" | tr -d '\r'
  fi
}

remote_status() {
  local cmd
  if [[ -n "$REMOTE_NVPN_COMMAND" ]]; then
    cmd="$REMOTE_NVPN_COMMAND status --json --discover-secs 0"
  else
    cmd="$(q "$REMOTE_NVPN") status --json --discover-secs 0"
  fi
  if [[ -n "$REMOTE_CONFIG" ]]; then
    cmd+=" --config $(q "$REMOTE_CONFIG")"
  fi
  remote_sh "$cmd" | tr -d '\r'
}

daemon_log_file() {
  jq -r '.daemon.log_file // ""' <<<"$1"
}

status_for_side() {
  case "$1" in
    local) local_status ;;
    remote) remote_status ;;
    *) die "unknown side: $1" ;;
  esac
}

select_peer() {
  local status="$1"
  local explicit="$2"
  local label="$3"
  local expected_ip="${4:-}"
  local selected=""
  if [[ -n "$explicit" ]]; then
    printf '%s\n' "$explicit"
    return
  fi
  if [[ -n "$expected_ip" ]]; then
    if selected="$(
      jq -er --arg label "$label" --arg expected_ip "$expected_ip" '
        [
          .daemon.state.peers[]?
          | select(((.fips_transport_addr // "") | startswith($expected_ip + ":")))
          | (.participant_pubkey // .fips_endpoint_npub // empty)
        ]
        | unique
        | if length == 1 then .[0]
          else error($label + " status has " + (length | tostring) + " peers on expected underlay; set the matching NVPN_HOST_PAIR_*_PEER")
          end
      ' <<<"$status" 2>/dev/null
    )"; then
      printf '%s\n' "$selected"
      return
    fi
  fi
  jq -er --arg label "$label" '
    [.daemon.state.peers[]? | (.participant_pubkey // .fips_endpoint_npub // empty)]
    | unique
    | if length == 1 then .[0]
      else error($label + " status has " + (length | tostring) + " peers; set the matching NVPN_HOST_PAIR_*_PEER")
      end
  ' <<<"$status"
}

peer_field() {
  local status="$1"
  local peer="$2"
  local field="$3"
  jq -r --arg peer "$peer" --arg field "$field" '
    .daemon.state.peers[]?
    | select(.participant_pubkey == $peer or .fips_endpoint_npub == $peer)
    | .[$field] // ""
  ' <<<"$status" | head -n1
}

strip_cidr() {
  local value="$1"
  value="${value%%/*}"
  printf '%s\n' "$value"
}

peer_tunnel_ip() {
  local status="$1"
  local peer="$2"
  strip_cidr "$(peer_field "$status" "$peer" tunnel_ip)"
}

assert_float_at_most() {
  local actual="$1"
  local max="$2"
  local label="$3"
  awk -v actual="$actual" -v max="$max" -v label="$label" '
    BEGIN {
      if (actual == "" || actual == "null") {
        printf("fips host-pair soak failed: %s missing\n", label) > "/dev/stderr";
        exit 1;
      }
      if ((actual + 0) > (max + 0)) {
        printf("fips host-pair soak failed: %s %.3f exceeds %.3f\n", label, actual, max) > "/dev/stderr";
        exit 1;
      }
    }'
}

assert_float_drift_at_most() {
  local actual="$1"
  local baseline="$2"
  local max_delta="$3"
  local max_factor="$4"
  local label="$5"
  [[ -z "$actual" || "$actual" == "null" || -z "$baseline" || "$baseline" == "null" ]] && return 0
  awk \
    -v actual="$actual" \
    -v baseline="$baseline" \
    -v max_delta="$max_delta" \
    -v max_factor="$max_factor" \
    -v label="$label" '
    BEGIN {
      delta = actual - baseline;
      if (delta < 0) delta = -delta;
      limit = max_delta + 0;
      factor_limit = (baseline + 0) * (max_factor + 0);
      if (factor_limit > limit) limit = factor_limit;
      if (delta > limit) {
        printf("fips host-pair soak failed: %s drift %.3f exceeds %.3f (actual %.3f baseline %.3f)\n", label, delta, limit, actual, baseline) > "/dev/stderr";
        exit 1;
      }
    }'
}

assert_peer_path() {
  local status="$1"
  local peer="$2"
  local expected_ip="$3"
  local label="$4"
  local reachable transport_addr srtt srtt_age_ms
  reachable="$(peer_field "$status" "$peer" reachable)"
  transport_addr="$(peer_field "$status" "$peer" fips_transport_addr)"
  srtt="$(peer_field "$status" "$peer" fips_srtt_ms)"
  srtt_age_ms="$(peer_field "$status" "$peer" fips_srtt_age_ms)"
  if [[ "$reachable" != "true" ]]; then
    printf '%s\n' "$status" >&2
    die "$label peer is not reachable"
  fi
  if [[ "$ALLOW_NON_DIRECT" == "0" && -n "$expected_ip" && "$transport_addr" != "$expected_ip:"* ]]; then
    printf '%s\n' "$status" >&2
    die "$label route changed away from expected direct UDP path (addr=$transport_addr expected_ip=$expected_ip)"
  fi
  if [[ "$srtt" != "null" && -n "$srtt" ]]; then
    assert_float_at_most "$srtt" "$MAX_SRTT_MS" "$label FIPS SRTT ms"
    if [[ "$srtt_age_ms" == "null" || -z "$srtt_age_ms" ]]; then
      die "$label FIPS SRTT age missing"
    fi
    assert_float_at_most "$srtt_age_ms" "$MAX_SRTT_AGE_MS" "$label FIPS SRTT age ms"
  fi
}

wait_for_peer_status() {
  local side="$1"
  local peer="$2"
  local label="$3"
  local deadline=$((SECONDS + 90))
  local status reachable
  while (( SECONDS < deadline )); do
    status="$(status_for_side "$side" || true)"
    reachable="$(peer_field "$status" "$peer" reachable || true)"
    if jq -e '.status_source == "daemon"' >/dev/null 2>&1 <<<"$status" \
      && [[ "$reachable" == "true" ]]; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 2
  done
  printf '%s\n' "$status" >&2
  die "$label did not become reachable via daemon status"
}

ping_command() {
  local target="$1"
  printf 'ping -c %q -i %q %q' "$PING_COUNT" "$PING_INTERVAL" "$target"
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

ping_probe() {
  local side="$1"
  local target="$2"
  local log_path="$3"
  local cmd
  cmd="$(ping_command "$target")"
  set +e
  if [[ "$side" == "local" ]]; then
    bash -lc "$cmd" >"$log_path" 2>&1
  else
    remote_sh "$cmd" >"$log_path" 2>&1
  fi
  set -e
  parse_ping_stats "$log_path"
}

side_has_cmd() {
  local side="$1"
  local name="$2"
  if [[ "$side" == "local" ]]; then
    command -v "$name" >/dev/null 2>&1
  else
    remote_sh "command -v $(q "$name") >/dev/null 2>&1"
  fi
}

start_remote_iperf_server() {
  remote_sh "pkill -9 iperf3 >/dev/null 2>&1 || true; iperf3 -s -D --logfile /tmp/nvpn-host-pair-iperf3.log"
  sleep 1
}

iperf_probe() {
  local label="$1"
  local json_path="$2"
  shift 2
  local rc mbps retrans
  set +e
  iperf3 -J -c "$REMOTE_TUNNEL_IP" -t "$IPERF_DURATION" -O 1 --connect-timeout 3000 "$@" \
    >"$json_path" 2>"$json_path.err"
  rc=$?
  set -e
  if (( rc != 0 )); then
    if [[ "$REQUIRE_IPERF" == "1" ]]; then
      cat "$json_path.err" >&2
      die "iperf $label failed"
    fi
    printf 'null null\n'
    return
  fi
  if ! mbps="$(jq -er '
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
  ' "$json_path")"; then
    if [[ "$REQUIRE_IPERF" == "1" ]]; then
      cat "$json_path" >&2
      die "iperf $label returned no throughput result"
    fi
    printf 'null null\n'
    return
  fi
  retrans="$(jq -r '(.end.sum_sent.retransmits // .end.sum.retransmits // 0)' "$json_path")"
  printf '%s %s\n' "$mbps" "$retrans"
}

record_iperf_progress() {
  local label="$1"
  local value="$2"
  local counter_var="$3"
  local current
  [[ "$value" == "null" || -z "$value" ]] && return 0
  if ! awk -v min="$MIN_IPERF_MBPS" 'BEGIN { exit !((min + 0) > 0) }'; then
    printf -v "$counter_var" '%s' 0
    return 0
  fi
  current="${!counter_var}"
  if awk -v value="$value" -v min="$MIN_IPERF_MBPS" 'BEGIN { exit !((value + 0) < (min + 0)) }'; then
    current=$((current + 1))
    printf -v "$counter_var" '%s' "$current"
    if (( current > MAX_CONSECUTIVE_IPERF_COLLAPSES )); then
      die "$label iperf throughput stayed below ${MIN_IPERF_MBPS} Mbps for ${current} consecutive sample(s)"
    fi
  else
    printf -v "$counter_var" '%s' 0
  fi
}

record_rekey_progress() {
  local label="$1"
  local in_progress="$2"
  local draining="$3"
  local counter_var="$4"
  local current
  if [[ "$in_progress" == "true" || "$draining" == "true" ]]; then
    current="${!counter_var}"
    current=$((current + 1))
    printf -v "$counter_var" '%s' "$current"
    if (( current > MAX_CONSECUTIVE_REKEY_SAMPLES )); then
      die "$label rekey state stayed active for ${current} consecutive sample(s) (in_progress=$in_progress draining=$draining)"
    fi
  else
    printf -v "$counter_var" '%s' 0
  fi
}

record_direct_probe_progress() {
  local label="$1"
  local pending="$2"
  local retry_after_ms="$3"
  local now_ms="$4"
  local pending_counter_var="$5"
  local overdue_counter_var="$6"
  local current_pending current_overdue
  if [[ "$pending" == "true" ]]; then
    current_pending="${!pending_counter_var}"
    current_pending=$((current_pending + 1))
    printf -v "$pending_counter_var" '%s' "$current_pending"
    if ! [[ "$retry_after_ms" =~ ^[0-9]+$ ]] || (( retry_after_ms <= now_ms )); then
      current_overdue="${!overdue_counter_var}"
      current_overdue=$((current_overdue + 1))
      printf -v "$overdue_counter_var" '%s' "$current_overdue"
    else
      printf -v "$overdue_counter_var" '%s' 0
      current_overdue=0
    fi
    if (( current_overdue > MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES )); then
      die "$label direct probe stayed overdue for ${current_overdue} consecutive sample(s) (retry_after_ms=$retry_after_ms now_ms=$now_ms pending_samples=$current_pending)"
    fi
  else
    printf -v "$pending_counter_var" '%s' 0
    printf -v "$overdue_counter_var" '%s' 0
  fi
}

epoch_ms() {
  printf '%s000\n' "$(date -u +%s)"
}

epoch_secs() {
  date -u +%s
}

side_epoch_secs() {
  local side="$1"
  if [[ "$side" == "local" ]]; then
    epoch_secs
  else
    remote_sh "date -u +%s" | tr -d '\r'
  fi
}

is_uint() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

fips_last_seen_age_secs() {
  local last_seen="$1"
  local now="$2"
  if [[ -z "$last_seen" || "$last_seen" == "null" ]] || ! is_uint "$last_seen" || ! is_uint "$now"; then
    printf 'null\n'
    return
  fi
  if (( last_seen > now )); then
    printf '0\n'
  else
    printf '%s\n' $((now - last_seen))
  fi
}

assert_fips_liveness_fresh() {
  local label="$1"
  local last_seen="$2"
  local now="$3"
  assert_fips_timestamp_fresh "$label" "last_fips_seen_at" "$last_seen" "$now" "$MAX_FIPS_LAST_SEEN_AGE_SECS"
}

assert_fips_control_liveness_fresh() {
  local label="$1"
  local last_seen="$2"
  local now="$3"
  assert_fips_timestamp_fresh "$label" "last_fips_control_seen_at" "$last_seen" "$now" "$MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS"
}

assert_fips_data_liveness_fresh() {
  local label="$1"
  local last_seen="$2"
  local now="$3"
  assert_fips_timestamp_fresh "$label" "last_fips_data_seen_at" "$last_seen" "$now" "$MAX_FIPS_DATA_LAST_SEEN_AGE_SECS"
}

assert_fips_timestamp_fresh() {
  local label="$1"
  local field="$2"
  local last_seen="$3"
  local now="$4"
  local max_age="$5"
  local age future_by
  [[ -n "$last_seen" && "$last_seen" != "null" ]] || die "$label $field is missing"
  is_uint "$last_seen" || die "$label $field is not numeric: $last_seen"
  is_uint "$now" || die "$label sample timestamp is not numeric: $now"
  if (( last_seen > now + MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS )); then
    future_by=$((last_seen - now))
    die "$label $field is ${future_by}s in the future ($field=$last_seen now=$now)"
  fi
  age="$(fips_last_seen_age_secs "$last_seen" "$now")"
  if (( age > max_age )); then
    die "$label $field is stale (age=${age}s max=${max_age}s $field=$last_seen now=$now)"
  fi
}

daemon_cpu_percent() {
  local side="$1"
  local cmd
  cmd="(ps -eo pcpu,args 2>/dev/null || ps -axo pcpu,command) | awk '/[n]vpn/ && /daemon|connect/ { sum += \$1 } END { printf \"%.1f\", sum }'"
  if [[ "$side" == "local" ]]; then
    bash -lc "$cmd"
  else
    remote_sh "$cmd"
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
      die "NVPN_HOST_PAIR_CPU_STRESS_WORKERS must be a non-negative integer or auto"
      ;;
    *)
      printf '%s\n' "$CPU_STRESS_WORKERS"
      ;;
  esac
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
      *) die "NVPN_HOST_PAIR_CPU_STRESS_SIDES must contain local, remote, or both" ;;
    esac
  done
  if (( selected == 0 )); then
    die "NVPN_HOST_PAIR_CPU_STRESS_SIDES must contain local, remote, or both"
  fi
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
  if [[ -n "${REMOTE_SSH:-}" && -n "${REMOTE_CPU_STRESS_PID_FILE:-}" ]]; then
    remote_sh "$(cpu_stress_stop_cmd "$REMOTE_CPU_STRESS_PID_FILE")" >/dev/null 2>&1 || true
  fi
}

start_cpu_stress_if_enabled() {
  is_true "$CPU_STRESS" || return 0
  validate_cpu_stress_sides
  printf 'fips host-pair CPU stress enabled: sides=%s workers=%s settle=%ss\n' \
    "$CPU_STRESS_SIDES" "$CPU_STRESS_WORKERS" "$CPU_STRESS_SETTLE_SECS"
  if csv_has_token "$CPU_STRESS_SIDES" "both" || csv_has_token "$CPU_STRESS_SIDES" "local"; then
    start_cpu_stress_side local
  fi
  if csv_has_token "$CPU_STRESS_SIDES" "both" || csv_has_token "$CPU_STRESS_SIDES" "remote"; then
    start_cpu_stress_side remote
  fi
  sleep "$CPU_STRESS_SETTLE_SECS"
}

pipeline_latest_line() {
  local side="$1"
  local path="$2"
  local prefix="$3"
  [[ -n "$path" ]] || return 0
  local cmd log_read_command
  if [[ "$side" == "local" ]]; then
    log_read_command="$LOCAL_LOG_READ_COMMAND"
  else
    log_read_command="$REMOTE_LOG_READ_COMMAND"
  fi
  if [[ -n "$log_read_command" ]]; then
    cmd="$log_read_command $(q "$path") 2>/dev/null | grep '^\\[$prefix ' | tail -n1 || true"
  else
    cmd="grep '^\\[$prefix ' $(q "$path") 2>/dev/null | tail -n1 || true"
  fi
  if [[ "$side" == "local" ]]; then
    bash -lc "$cmd"
  else
    remote_sh "$cmd"
  fi
}

pipeline_line_count() {
  local side="$1"
  local path="$2"
  local prefix="$3"
  [[ -n "$path" ]] || {
    printf '0\n'
    return 0
  }
  local cmd log_read_command
  if [[ "$side" == "local" ]]; then
    log_read_command="$LOCAL_LOG_READ_COMMAND"
  else
    log_read_command="$REMOTE_LOG_READ_COMMAND"
  fi
  if [[ -n "$log_read_command" ]]; then
    cmd="$log_read_command $(q "$path") 2>/dev/null | awk '/^\\[$prefix / { count++ } END { print count + 0 }'"
  else
    cmd="awk '/^\\[$prefix / { count++ } END { print count + 0 }' $(q "$path") 2>/dev/null || printf '0\n'"
  fi
  if [[ "$side" == "local" ]]; then
    bash -lc "$cmd"
  else
    remote_sh "$cmd"
  fi
}

pipeline_count_has_advanced() {
  local count baseline
  count="$(int_value "$1")"
  baseline="$(int_value "$2")"
  (( count > baseline ))
}

pipeline_hard_events() {
  local line="$1"
  [[ -z "$line" ]] && return 0
  local event regex rate total found
  local hard_events=(
    connected_udp_activation_failed
    encrypt_worker_queue_full
    encrypt_worker_bulk_dropped
    decrypt_worker_queue_full
    decrypt_worker_bulk_dropped
    decrypt_worker_register_full
    decrypt_worker_priority_dropped
    decrypt_fallback_bulk_dropped
    decrypt_fallback_priority_dropped
    pending_tun_destination_dropped
    pending_tun_packet_dropped
    pending_endpoint_destination_dropped
    pending_endpoint_packet_dropped
    endpoint_event_backlog_high
    endpoint_event_bulk_dropped
    transport_channel_backlog_high
    transport_bulk_dropped
    udp_send_bulk_dropped
    nvpn_tun_to_mesh_bulk_dropped
  )
  for event in "${hard_events[@]}"; do
    regex="(^|[[:space:]])${event}=([0-9]+([.][0-9]+)?)/s([[:space:]]total=([0-9]+))?([[:space:]]|$)"
    if [[ "$line" =~ $regex ]]; then
      rate="${BASH_REMATCH[2]}"
      total="${BASH_REMATCH[5]:-}"
      if [[ -z "$total" ]] || awk -v rate="$rate" -v total="$total" 'BEGIN { exit !((rate + 0) > 0 || (total + 0) > 0) }'; then
        found+="${found:+,}$event"
      fi
    fi
  done
  printf '%s\n' "${found:-}"
}

pipeline_hard_event_summary() {
  local line csv event found
  local -a events
  found=""
  for line in "$@"; do
    csv="$(pipeline_hard_events "$line")"
    [[ -n "$csv" ]] || continue
    IFS=',' read -r -a events <<<"$csv"
    for event in "${events[@]}"; do
      [[ -n "$event" ]] || continue
      case ",$found," in
        *",$event,"*) ;;
        *) found+="${found:+,}$event" ;;
      esac
    done
  done
  printf '%s\n' "${found:-}"
}

pipeline_queue_wait_json() {
  local line="$1"
  jq -nc --arg line "$line" '
    def duration_ms($value; $unit):
      if $unit == "ns" then ($value / 1000000)
      elif $unit == "us" then ($value / 1000)
      elif $unit == "ms" then $value
      elif $unit == "s" then ($value * 1000)
      else null
      end;
    def wait_point($name):
      ([
        $line
        | try capture(
            "(^| )" + $name
            + "=[0-9]+(?:\\.[0-9]+)?/s"
            + " avg=[0-9]+(?:\\.[0-9]+)?(?:ns|us|ms|s)"
            + " p50<=[0-9]+(?:\\.[0-9]+)?(?:ns|us|ms|s)"
            + " p95<=(?<p95>[0-9]+(?:\\.[0-9]+)?)(?<p95_unit>ns|us|ms|s)"
            + " p99<=(?<p99>[0-9]+(?:\\.[0-9]+)?)(?<p99_unit>ns|us|ms|s)"
            + " max<=(?<max>[0-9]+(?:\\.[0-9]+)?)(?<max_unit>ns|us|ms|s)"
            + " allmax=(?<allmax>[0-9]+(?:\\.[0-9]+)?)(?<allmax_unit>ns|us|ms|s)"
          ) catch null
        | select(. != null)
      ] | first // null)
      | if . == null then null else {
          p95_ms: duration_ms((.p95 | tonumber); .p95_unit),
          p99_ms: duration_ms((.p99 | tonumber); .p99_unit),
          max_ms: duration_ms((.max | tonumber); .max_unit),
          allmax_ms: duration_ms((.allmax | tonumber); .allmax_unit)
        } end;
    {
      endpoint_command_wait: wait_point("endpoint_command_wait"),
      endpoint_priority_command_wait: wait_point("endpoint_priority_command_wait"),
      endpoint_bulk_command_wait: wait_point("endpoint_bulk_command_wait"),
      endpoint_event_wait: wait_point("endpoint_event_wait"),
      endpoint_priority_event_wait: wait_point("endpoint_priority_event_wait"),
      endpoint_bulk_event_wait: wait_point("endpoint_bulk_event_wait"),
      fmp_worker_queue_wait: wait_point("fmp_worker_queue_wait"),
      fmp_worker_priority_queue_wait: wait_point("fmp_worker_priority_queue_wait"),
      fmp_worker_bulk_queue_wait: wait_point("fmp_worker_bulk_queue_wait"),
      decrypt_worker_queue_wait: wait_point("decrypt_worker_queue_wait"),
      decrypt_worker_priority_queue_wait: wait_point("decrypt_worker_priority_queue_wait"),
      decrypt_worker_bulk_queue_wait: wait_point("decrypt_worker_bulk_queue_wait"),
      decrypt_fallback_wait: wait_point("decrypt_fallback_wait"),
      decrypt_fallback_priority_wait: wait_point("decrypt_fallback_priority_wait"),
      decrypt_fallback_bulk_wait: wait_point("decrypt_fallback_bulk_wait"),
      decrypt_authenticated_session_wait: wait_point("decrypt_authenticated_session_wait"),
      decrypt_authenticated_session_priority_wait: wait_point("decrypt_authenticated_session_priority_wait"),
      decrypt_authenticated_session_bulk_wait: wait_point("decrypt_authenticated_session_bulk_wait"),
      decrypt_fsp_worker_queue_wait: wait_point("decrypt_fsp_worker_queue_wait"),
      decrypt_fsp_worker_priority_queue_wait: wait_point("decrypt_fsp_worker_priority_queue_wait"),
      decrypt_fsp_worker_bulk_queue_wait: wait_point("decrypt_fsp_worker_bulk_queue_wait"),
      transport_queue_wait: wait_point("transport_queue_wait"),
      transport_priority_queue_wait: wait_point("transport_priority_queue_wait"),
      transport_bulk_queue_wait: wait_point("transport_bulk_queue_wait"),
      transport_channel_wait: wait_point("transport_channel_wait"),
      transport_priority_channel_wait: wait_point("transport_priority_channel_wait"),
      transport_bulk_channel_wait: wait_point("transport_bulk_channel_wait"),
      transport_rx_loop_wait: wait_point("transport_rx_loop_wait"),
      transport_priority_rx_loop_wait: wait_point("transport_priority_rx_loop_wait"),
      transport_bulk_rx_loop_wait: wait_point("transport_bulk_rx_loop_wait"),
      nvpn_tun_to_mesh_queue_wait: wait_point("nvpn_tun_to_mesh_queue_wait")
    }
  '
}

pipeline_queue_wait_top_summary() {
  printf '%s\n%s\n' "${1:-}" "${2:-}" | awk '
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
      metrics = "endpoint_command_wait endpoint_priority_command_wait endpoint_bulk_command_wait endpoint_event_wait endpoint_priority_event_wait endpoint_bulk_event_wait fmp_worker_queue_wait fmp_worker_priority_queue_wait fmp_worker_bulk_queue_wait decrypt_worker_queue_wait decrypt_worker_priority_queue_wait decrypt_worker_bulk_queue_wait decrypt_fallback_wait decrypt_fallback_priority_wait decrypt_fallback_bulk_wait decrypt_authenticated_session_wait decrypt_authenticated_session_priority_wait decrypt_authenticated_session_bulk_wait decrypt_fsp_worker_queue_wait decrypt_fsp_worker_priority_queue_wait decrypt_fsp_worker_bulk_queue_wait transport_queue_wait transport_priority_queue_wait transport_bulk_queue_wait transport_channel_wait transport_priority_channel_wait transport_bulk_channel_wait transport_rx_loop_wait transport_priority_rx_loop_wait transport_bulk_rx_loop_wait nvpn_tun_to_mesh_queue_wait nvpn_mesh_to_tun_queue_wait"
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

pipeline_queue_wait_violations() {
  local queue_wait="$1"
  jq -r \
    --argjson max_p95 "$MAX_PIPELINE_QUEUE_WAIT_P95_MS" \
    --argjson max_p99 "$MAX_PIPELINE_QUEUE_WAIT_P99_MS" '
    to_entries
    | map(
        select(.value != null)
        | select(
          ((.value.p95_ms // 0) > $max_p95)
          or ((.value.p99_ms // 0) > $max_p99)
        )
        | "\(.key):p95=\(.value.p95_ms // "null")ms,p99=\(.value.p99_ms // "null")ms"
      )
    | join(",")
  ' <<<"$queue_wait"
}

pipeline_priority_hard_events() {
  local line="$1"
  [[ -z "$line" ]] && return 0
  local event regex rate total found
  local hard_events=(
    encrypt_worker_priority_queue_full
    decrypt_worker_priority_dropped
    decrypt_fallback_priority_dropped
    decrypt_fsp_priority_queue_full_fallback
    decrypt_authenticated_session_priority_dropped
  )
  for event in "${hard_events[@]}"; do
    regex="(^|[[:space:]])${event}=([0-9]+([.][0-9]+)?)/s([[:space:]]total=([0-9]+))?([[:space:]]|$)"
    if [[ "$line" =~ $regex ]]; then
      rate="${BASH_REMATCH[2]}"
      total="${BASH_REMATCH[5]:-}"
      if [[ -z "$total" ]] || awk -v rate="$rate" -v total="$total" 'BEGIN { exit !((rate + 0) > 0 || (total + 0) > 0) }'; then
        found+="${found:+,}$event"
      fi
    fi
  done
  printf '%s\n' "${found:-}"
}

pipeline_priority_queue_wait_violations() {
  local queue_wait="$1"
  local threshold_ms="${MAX_PRIORITY_QUEUE_WAIT_MS:-0}"
  if ! awk -v threshold_ms="$threshold_ms" 'BEGIN { exit !((threshold_ms + 0) > 0) }'; then
    return 0
  fi
  jq -r \
    --argjson threshold "$threshold_ms" '
    . as $waits
    | [
        "endpoint_priority_command_wait",
        "endpoint_priority_event_wait",
        "fmp_worker_priority_queue_wait",
        "decrypt_worker_priority_queue_wait",
        "decrypt_fallback_priority_wait",
        "decrypt_authenticated_session_priority_wait",
        "decrypt_fsp_worker_priority_queue_wait",
        "transport_priority_queue_wait",
        "transport_priority_channel_wait",
        "transport_priority_rx_loop_wait"
      ]
    | map(
        . as $name
        | ($waits[$name] // null) as $observed
        | select($observed != null and (($observed.max_ms // 0) > $threshold))
        | "\($name):max=\($observed.max_ms // "null")ms,p99=\($observed.p99_ms // "null")ms"
      )
    | join(",")
  ' <<<"$queue_wait"
}

assert_pipeline_ok() {
  local label="$1"
  local line="$2"
  [[ -z "$line" ]] && return 0
  local hard_events priority_hard_events queue_wait queue_waits priority_queue_waits
  if is_true "$FAIL_ON_PRIORITY_HARD_EVENTS"; then
    priority_hard_events="$(pipeline_priority_hard_events "$line")"
    if [[ -n "$priority_hard_events" ]]; then
      die "$label observed priority/control hard pipeline events: $priority_hard_events"
    fi
  fi
  queue_wait="$(pipeline_queue_wait_json "$line")"
  priority_queue_waits="$(pipeline_priority_queue_wait_violations "$queue_wait")"
  if [[ -n "$priority_queue_waits" ]]; then
    die "$label priority queue wait exceeded threshold: $priority_queue_waits"
  fi
  if [[ "$ALLOW_QUEUE_EVENTS" == "0" ]]; then
    hard_events="$(pipeline_hard_events "$line")"
    if [[ -n "$hard_events" ]]; then
      die "$label observed hard pipeline events: $hard_events"
    fi
  fi
  if [[ "$ALLOW_QUEUE_WAIT" != "1" ]]; then
    queue_waits="$(pipeline_queue_wait_violations "$queue_wait")"
    if [[ -n "$queue_waits" ]]; then
      die "$label queue wait exceeded threshold: $queue_waits"
    fi
  fi
}

assert_pipeline_present_if_required() {
  local label="$1"
  local path="$2"
  local fips_line="$3"
  local nvpn_line="$4"
  [[ "$REQUIRE_PIPELINE_LOGS" == "1" ]] || return 0
  [[ -n "$path" ]] || die "$label pipeline log required but no daemon log path is configured or discoverable"
  [[ -n "$fips_line$nvpn_line" ]] || die "$label pipeline log required but no pipe/nvpn-pipe summary lines were found at $path"
}

assert_pipeline_fresh() {
  local label="$1"
  local count="$2"
  local counter_var="$3"
  local stale_counter_var="$4"
  local previous_count stale_count
  count="$(int_value "$count")"
  previous_count="${!counter_var}"
  if [[ -n "$previous_count" && "$count" -le "$previous_count" ]]; then
    stale_count="${!stale_counter_var}"
    stale_count=$((stale_count + 1))
    printf -v "$stale_counter_var" '%s' "$stale_count"
    if (( stale_count > MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES )); then
      die "$label pipeline summaries did not advance for $stale_count consecutive sample(s) (count=$count previous=$previous_count)"
    fi
    return 0
  fi
  if (( count > 0 )); then
    printf -v "$counter_var" '%s' "$count"
    printf -v "$stale_counter_var" '%s' 0
  fi
}

int_value() {
  local value="$1"
  [[ -z "$value" || "$value" == "null" ]] && value=0
  printf '%s\n' "${value%.*}"
}

assert_counter_progress() {
  local label="$1"
  local sent="$2"
  local recv="$3"
  local prev_sent="$4"
  local prev_recv="$5"
  sent="$(int_value "$sent")"
  recv="$(int_value "$recv")"
  prev_sent="$(int_value "$prev_sent")"
  prev_recv="$(int_value "$prev_recv")"
  if (( sent <= prev_sent && recv <= prev_recv )); then
    die "$label FIPS byte counters did not advance (sent=$sent recv=$recv prev_sent=$prev_sent prev_recv=$prev_recv)"
  fi
}

write_sample() {
  local iteration="$1"
  local timestamp="$2"
  local local_status_path="$3"
  local remote_status_path="$4"
  local pipeline_hard_events
  pipeline_hard_events="$(
    pipeline_hard_event_summary \
      "$FIPS_PIPELINE_LOCAL" \
      "$FIPS_PIPELINE_REMOTE" \
      "$NVPN_PIPELINE_LOCAL" \
      "$NVPN_PIPELINE_REMOTE"
  )"
  jq -nc \
    --arg timestamp "$timestamp" \
    --argjson iteration "$iteration" \
    --arg local_peer "$LOCAL_PEER" \
    --arg remote_peer "$REMOTE_PEER" \
    --arg remote_tunnel_ip "$REMOTE_TUNNEL_IP" \
    --arg local_tunnel_ip "$LOCAL_TUNNEL_IP" \
    --arg local_status "$local_status_path" \
    --arg remote_status "$remote_status_path" \
    --arg local_transport "$LOCAL_TRANSPORT_ADDR" \
    --arg remote_transport "$REMOTE_TRANSPORT_ADDR" \
    --arg local_srtt "$LOCAL_SRTT" \
    --arg remote_srtt "$REMOTE_SRTT" \
    --arg local_srtt_age_ms "$LOCAL_SRTT_AGE_MS" \
    --arg remote_srtt_age_ms "$REMOTE_SRTT_AGE_MS" \
    --arg local_bytes_sent "$LOCAL_BYTES_SENT" \
    --arg local_bytes_recv "$LOCAL_BYTES_RECV" \
    --arg remote_bytes_sent "$REMOTE_BYTES_SENT" \
    --arg remote_bytes_recv "$REMOTE_BYTES_RECV" \
    --arg local_last_mesh_seen_at "$LOCAL_LAST_MESH_SEEN_AT" \
    --arg local_last_fips_seen_at "$LOCAL_LAST_FIPS_SEEN_AT" \
    --arg local_last_fips_seen_age_secs "$LOCAL_LAST_FIPS_SEEN_AGE_SECS" \
    --arg local_last_fips_control_seen_at "$LOCAL_LAST_FIPS_CONTROL_SEEN_AT" \
    --arg local_last_fips_control_seen_age_secs "$LOCAL_LAST_FIPS_CONTROL_SEEN_AGE_SECS" \
    --arg local_last_fips_data_seen_at "$LOCAL_LAST_FIPS_DATA_SEEN_AT" \
    --arg local_last_fips_data_seen_age_secs "$LOCAL_LAST_FIPS_DATA_SEEN_AGE_SECS" \
    --arg local_last_handshake_at "$LOCAL_LAST_HANDSHAKE_AT" \
    --arg remote_last_mesh_seen_at "$REMOTE_LAST_MESH_SEEN_AT" \
    --arg remote_last_fips_seen_at "$REMOTE_LAST_FIPS_SEEN_AT" \
    --arg remote_last_fips_seen_age_secs "$REMOTE_LAST_FIPS_SEEN_AGE_SECS" \
    --arg remote_last_fips_control_seen_at "$REMOTE_LAST_FIPS_CONTROL_SEEN_AT" \
    --arg remote_last_fips_control_seen_age_secs "$REMOTE_LAST_FIPS_CONTROL_SEEN_AGE_SECS" \
    --arg remote_last_fips_data_seen_at "$REMOTE_LAST_FIPS_DATA_SEEN_AT" \
    --arg remote_last_fips_data_seen_age_secs "$REMOTE_LAST_FIPS_DATA_SEEN_AGE_SECS" \
    --arg remote_last_handshake_at "$REMOTE_LAST_HANDSHAKE_AT" \
    --arg local_rekey_in_progress "$LOCAL_REKEY_IN_PROGRESS" \
    --arg local_rekey_draining "$LOCAL_REKEY_DRAINING" \
    --arg local_current_k_bit "$LOCAL_CURRENT_K_BIT" \
    --arg local_direct_probe_pending "$LOCAL_DIRECT_PROBE_PENDING" \
    --arg local_direct_probe_after_ms "$LOCAL_DIRECT_PROBE_AFTER_MS" \
    --arg local_direct_probe_retry_count "$LOCAL_DIRECT_PROBE_RETRY_COUNT" \
    --arg local_direct_probe_auto_reconnect "$LOCAL_DIRECT_PROBE_AUTO_RECONNECT" \
    --arg local_direct_probe_expires_at_ms "$LOCAL_DIRECT_PROBE_EXPIRES_AT_MS" \
    --arg local_direct_probe_pending_count "$LOCAL_DIRECT_PROBE_PENDING_COUNT" \
    --arg local_direct_probe_overdue_count "$LOCAL_DIRECT_PROBE_OVERDUE_COUNT" \
    --arg local_nostr_traversal_failures "$LOCAL_NOSTR_TRAVERSAL_FAILURES" \
    --arg local_nostr_traversal_in_cooldown "$LOCAL_NOSTR_TRAVERSAL_IN_COOLDOWN" \
    --arg local_nostr_traversal_cooldown_until_ms "$LOCAL_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS" \
    --arg local_nostr_traversal_last_skew_ms "$LOCAL_NOSTR_TRAVERSAL_LAST_SKEW_MS" \
    --arg local_rekey_stuck_count "$LOCAL_REKEY_STUCK_COUNT" \
    --arg remote_rekey_in_progress "$REMOTE_REKEY_IN_PROGRESS" \
    --arg remote_rekey_draining "$REMOTE_REKEY_DRAINING" \
    --arg remote_current_k_bit "$REMOTE_CURRENT_K_BIT" \
    --arg remote_direct_probe_pending "$REMOTE_DIRECT_PROBE_PENDING" \
    --arg remote_direct_probe_after_ms "$REMOTE_DIRECT_PROBE_AFTER_MS" \
    --arg remote_direct_probe_retry_count "$REMOTE_DIRECT_PROBE_RETRY_COUNT" \
    --arg remote_direct_probe_auto_reconnect "$REMOTE_DIRECT_PROBE_AUTO_RECONNECT" \
    --arg remote_direct_probe_expires_at_ms "$REMOTE_DIRECT_PROBE_EXPIRES_AT_MS" \
    --arg remote_direct_probe_pending_count "$REMOTE_DIRECT_PROBE_PENDING_COUNT" \
    --arg remote_direct_probe_overdue_count "$REMOTE_DIRECT_PROBE_OVERDUE_COUNT" \
    --arg remote_nostr_traversal_failures "$REMOTE_NOSTR_TRAVERSAL_FAILURES" \
    --arg remote_nostr_traversal_in_cooldown "$REMOTE_NOSTR_TRAVERSAL_IN_COOLDOWN" \
    --arg remote_nostr_traversal_cooldown_until_ms "$REMOTE_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS" \
    --arg remote_nostr_traversal_last_skew_ms "$REMOTE_NOSTR_TRAVERSAL_LAST_SKEW_MS" \
    --arg remote_rekey_stuck_count "$REMOTE_REKEY_STUCK_COUNT" \
    --arg ping_forward_loss "$PING_FORWARD_LOSS" \
    --arg ping_forward_avg "$PING_FORWARD_AVG" \
    --arg ping_forward_p95 "$PING_FORWARD_P95" \
    --arg ping_forward_p99 "$PING_FORWARD_P99" \
    --arg ping_forward_max "$PING_FORWARD_MAX" \
    --arg ping_reverse_loss "$PING_REVERSE_LOSS" \
    --arg ping_reverse_avg "$PING_REVERSE_AVG" \
    --arg ping_reverse_p95 "$PING_REVERSE_P95" \
    --arg ping_reverse_p99 "$PING_REVERSE_P99" \
    --arg ping_reverse_max "$PING_REVERSE_MAX" \
    --arg iperf_forward_mbps "$IPERF_FORWARD_MBPS" \
    --arg iperf_forward_retrans "$IPERF_FORWARD_RETRANS" \
    --arg iperf_reverse_mbps "$IPERF_REVERSE_MBPS" \
    --arg iperf_reverse_retrans "$IPERF_REVERSE_RETRANS" \
    --arg local_cpu "$LOCAL_CPU" \
    --arg remote_cpu "$REMOTE_CPU" \
    --arg fips_pipeline_local "$FIPS_PIPELINE_LOCAL" \
    --arg fips_pipeline_remote "$FIPS_PIPELINE_REMOTE" \
    --arg nvpn_pipeline_local "$NVPN_PIPELINE_LOCAL" \
    --arg nvpn_pipeline_remote "$NVPN_PIPELINE_REMOTE" \
    --arg pipeline_hard_events "$pipeline_hard_events" \
    --arg fips_pipeline_local_count "$FIPS_PIPELINE_LOCAL_COUNT" \
    --arg fips_pipeline_remote_count "$FIPS_PIPELINE_REMOTE_COUNT" \
    --arg nvpn_pipeline_local_count "$NVPN_PIPELINE_LOCAL_COUNT" \
    --arg nvpn_pipeline_remote_count "$NVPN_PIPELINE_REMOTE_COUNT" \
    --argjson fips_pipeline_local_queue_wait "$FIPS_PIPELINE_LOCAL_QUEUE_WAIT" \
    --argjson fips_pipeline_remote_queue_wait "$FIPS_PIPELINE_REMOTE_QUEUE_WAIT" \
    --argjson nvpn_pipeline_local_queue_wait "$NVPN_PIPELINE_LOCAL_QUEUE_WAIT" \
    --argjson nvpn_pipeline_remote_queue_wait "$NVPN_PIPELINE_REMOTE_QUEUE_WAIT" '
    def num($v): if $v == "" or $v == "null" then null else ($v | tonumber) end;
    def bool($v):
      if $v == "" or $v == "null" then null
      elif $v == "true" then true
      elif $v == "false" then false
      else null
      end;
    {
      timestamp: $timestamp,
      iteration: $iteration,
      peers: {
        local_peer: $local_peer,
        remote_peer: $remote_peer,
        local_tunnel_ip: $local_tunnel_ip,
        remote_tunnel_ip: $remote_tunnel_ip,
        local_transport_addr: $local_transport,
        remote_transport_addr: $remote_transport,
        local_srtt_ms: num($local_srtt),
        remote_srtt_ms: num($remote_srtt),
        local_srtt_age_ms: num($local_srtt_age_ms),
        remote_srtt_age_ms: num($remote_srtt_age_ms),
        local_bytes_sent: num($local_bytes_sent),
        local_bytes_recv: num($local_bytes_recv),
        remote_bytes_sent: num($remote_bytes_sent),
        remote_bytes_recv: num($remote_bytes_recv),
        local_last_mesh_seen_at: num($local_last_mesh_seen_at),
        local_last_fips_seen_at: num($local_last_fips_seen_at),
        local_last_fips_seen_age_secs: num($local_last_fips_seen_age_secs),
        local_last_fips_control_seen_at: num($local_last_fips_control_seen_at),
        local_last_fips_control_seen_age_secs: num($local_last_fips_control_seen_age_secs),
        local_last_fips_data_seen_at: num($local_last_fips_data_seen_at),
        local_last_fips_data_seen_age_secs: num($local_last_fips_data_seen_age_secs),
        local_last_handshake_at: num($local_last_handshake_at),
        remote_last_mesh_seen_at: num($remote_last_mesh_seen_at),
        remote_last_fips_seen_at: num($remote_last_fips_seen_at),
        remote_last_fips_seen_age_secs: num($remote_last_fips_seen_age_secs),
        remote_last_fips_control_seen_at: num($remote_last_fips_control_seen_at),
        remote_last_fips_control_seen_age_secs: num($remote_last_fips_control_seen_age_secs),
        remote_last_fips_data_seen_at: num($remote_last_fips_data_seen_at),
        remote_last_fips_data_seen_age_secs: num($remote_last_fips_data_seen_age_secs),
        remote_last_handshake_at: num($remote_last_handshake_at),
        local_rekey_in_progress: bool($local_rekey_in_progress),
        local_rekey_draining: bool($local_rekey_draining),
        local_current_k_bit: bool($local_current_k_bit),
        local_direct_probe_pending: bool($local_direct_probe_pending),
        local_direct_probe_after_ms: num($local_direct_probe_after_ms),
        local_direct_probe_retry_count: num($local_direct_probe_retry_count),
        local_direct_probe_auto_reconnect: bool($local_direct_probe_auto_reconnect),
        local_direct_probe_expires_at_ms: num($local_direct_probe_expires_at_ms),
        local_direct_probe_pending_count: num($local_direct_probe_pending_count),
        local_direct_probe_overdue_count: num($local_direct_probe_overdue_count),
        local_nostr_traversal_failures: num($local_nostr_traversal_failures),
        local_nostr_traversal_in_cooldown: bool($local_nostr_traversal_in_cooldown),
        local_nostr_traversal_cooldown_until_ms: num($local_nostr_traversal_cooldown_until_ms),
        local_nostr_traversal_last_skew_ms: num($local_nostr_traversal_last_skew_ms),
        local_rekey_stuck_count: ($local_rekey_stuck_count | tonumber),
        remote_rekey_in_progress: bool($remote_rekey_in_progress),
        remote_rekey_draining: bool($remote_rekey_draining),
        remote_current_k_bit: bool($remote_current_k_bit),
        remote_direct_probe_pending: bool($remote_direct_probe_pending),
        remote_direct_probe_after_ms: num($remote_direct_probe_after_ms),
        remote_direct_probe_retry_count: num($remote_direct_probe_retry_count),
        remote_direct_probe_auto_reconnect: bool($remote_direct_probe_auto_reconnect),
        remote_direct_probe_expires_at_ms: num($remote_direct_probe_expires_at_ms),
        remote_direct_probe_pending_count: num($remote_direct_probe_pending_count),
        remote_direct_probe_overdue_count: num($remote_direct_probe_overdue_count),
        remote_nostr_traversal_failures: num($remote_nostr_traversal_failures),
        remote_nostr_traversal_in_cooldown: bool($remote_nostr_traversal_in_cooldown),
        remote_nostr_traversal_cooldown_until_ms: num($remote_nostr_traversal_cooldown_until_ms),
        remote_nostr_traversal_last_skew_ms: num($remote_nostr_traversal_last_skew_ms),
        remote_rekey_stuck_count: ($remote_rekey_stuck_count | tonumber)
      },
      ping: {
        local_to_remote: {
          loss_percent: num($ping_forward_loss),
          avg_ms: num($ping_forward_avg),
          p95_ms: num($ping_forward_p95),
          p99_ms: num($ping_forward_p99),
          max_ms: num($ping_forward_max)
        },
        remote_to_local: {
          loss_percent: num($ping_reverse_loss),
          avg_ms: num($ping_reverse_avg),
          p95_ms: num($ping_reverse_p95),
          p99_ms: num($ping_reverse_p99),
          max_ms: num($ping_reverse_max)
        }
      },
      iperf: {
        forward_mbps: num($iperf_forward_mbps),
        forward_retrans: num($iperf_forward_retrans),
        reverse_mbps: num($iperf_reverse_mbps),
        reverse_retrans: num($iperf_reverse_retrans)
      },
      daemon_cpu: {
        local_percent: num($local_cpu),
        remote_percent: num($remote_cpu)
      },
      pipeline: {
        fips_local: $fips_pipeline_local,
        fips_remote: $fips_pipeline_remote,
        nvpn_local: $nvpn_pipeline_local,
        nvpn_remote: $nvpn_pipeline_remote,
        hard_events: (
          if $pipeline_hard_events == "" then []
          else ($pipeline_hard_events | split(",") | map(select(. != "")))
          end
        ),
        line_counts: {
          fips_local: num($fips_pipeline_local_count),
          fips_remote: num($fips_pipeline_remote_count),
          nvpn_local: num($nvpn_pipeline_local_count),
          nvpn_remote: num($nvpn_pipeline_remote_count)
        },
        queue_wait_ms: {
          fips_local: $fips_pipeline_local_queue_wait,
          fips_remote: $fips_pipeline_remote_queue_wait,
          nvpn_local: $nvpn_pipeline_local_queue_wait,
          nvpn_remote: $nvpn_pipeline_remote_queue_wait
        }
      },
      artifacts: {
        local_status: $local_status,
        remote_status: $remote_status
      }
    }'
}

write_summary_row() {
  local direct_path_checked=0
  local pipeline_log_checked=0
  local counter_progress_checked=0
  local fips_liveness_checked=0
  local fips_control_liveness_checked=0
  local fips_data_liveness_checked=0
  local pipeline_hard_events
  local pipeline_top_queue_wait_local pipeline_top_queue_wait_remote
  local -a row
  local field first
  if [[ "$ALLOW_NON_DIRECT" != "1" && -n "$EXPECTED_REMOTE_UNDERLAY_IP" && -n "$EXPECTED_LOCAL_UNDERLAY_IP" ]]; then
    direct_path_checked=1
  fi
  if [[ -n "${FIPS_PIPELINE_LOCAL:-}${FIPS_PIPELINE_REMOTE:-}${NVPN_PIPELINE_LOCAL:-}${NVPN_PIPELINE_REMOTE:-}" ]]; then
    pipeline_log_checked=1
  fi
  if (( iteration > 1 )); then
    counter_progress_checked=1
  fi
  if [[ -n "$LOCAL_LAST_FIPS_SEEN_AT" && -n "$REMOTE_LAST_FIPS_SEEN_AT" ]]; then
    fips_liveness_checked=1
  fi
  if [[ -n "$LOCAL_LAST_FIPS_CONTROL_SEEN_AT" && -n "$REMOTE_LAST_FIPS_CONTROL_SEEN_AT" ]]; then
    fips_control_liveness_checked=1
  fi
  if [[ -n "$LOCAL_LAST_FIPS_DATA_SEEN_AT" && -n "$REMOTE_LAST_FIPS_DATA_SEEN_AT" ]]; then
    fips_data_liveness_checked=1
  fi
  pipeline_hard_events="$(
    pipeline_hard_event_summary \
      "$FIPS_PIPELINE_LOCAL" \
      "$FIPS_PIPELINE_REMOTE" \
      "$NVPN_PIPELINE_LOCAL" \
      "$NVPN_PIPELINE_REMOTE"
  )"
  pipeline_top_queue_wait_local="$(pipeline_queue_wait_top_summary "$FIPS_PIPELINE_LOCAL" "$NVPN_PIPELINE_LOCAL")"
  pipeline_top_queue_wait_remote="$(pipeline_queue_wait_top_summary "$FIPS_PIPELINE_REMOTE" "$NVPN_PIPELINE_REMOTE")"
  row=(
    "$timestamp" "$iteration" \
    "$PING_FORWARD_LOSS" "$PING_FORWARD_AVG" "$PING_FORWARD_P95" "$PING_FORWARD_P99" "$PING_FORWARD_MAX" \
    "$PING_REVERSE_LOSS" "$PING_REVERSE_AVG" "$PING_REVERSE_P95" "$PING_REVERSE_P99" "$PING_REVERSE_MAX" \
    "$IPERF_FORWARD_MBPS" "$IPERF_FORWARD_RETRANS" "$IPERF_REVERSE_MBPS" "$IPERF_REVERSE_RETRANS" \
    "$LOCAL_SRTT" "$REMOTE_SRTT" \
    "$LOCAL_BYTES_SENT" "$LOCAL_BYTES_RECV" "$REMOTE_BYTES_SENT" "$REMOTE_BYTES_RECV" \
    "$LOCAL_CPU" "$REMOTE_CPU" \
    "$direct_path_checked" "$pipeline_log_checked" "$counter_progress_checked" \
    "$IPERF_FORWARD_COLLAPSE_COUNT" "$IPERF_REVERSE_COLLAPSE_COUNT" \
    "$fips_liveness_checked" "$LOCAL_LAST_FIPS_SEEN_AGE_SECS" "$REMOTE_LAST_FIPS_SEEN_AGE_SECS" \
    "$fips_control_liveness_checked" "$LOCAL_LAST_FIPS_CONTROL_SEEN_AGE_SECS" "$REMOTE_LAST_FIPS_CONTROL_SEEN_AGE_SECS" \
    "$fips_data_liveness_checked" "$LOCAL_LAST_FIPS_DATA_SEEN_AGE_SECS" "$REMOTE_LAST_FIPS_DATA_SEEN_AGE_SECS" \
    "${LOCAL_REKEY_IN_PROGRESS:-}" "${LOCAL_REKEY_DRAINING:-}" "${LOCAL_CURRENT_K_BIT:-}" "${LOCAL_REKEY_STUCK_COUNT:-}" \
    "${REMOTE_REKEY_IN_PROGRESS:-}" "${REMOTE_REKEY_DRAINING:-}" "${REMOTE_CURRENT_K_BIT:-}" "${REMOTE_REKEY_STUCK_COUNT:-}" \
    "${LOCAL_DIRECT_PROBE_PENDING:-}" "${LOCAL_DIRECT_PROBE_AFTER_MS:-}" "${LOCAL_DIRECT_PROBE_RETRY_COUNT:-}" "${LOCAL_DIRECT_PROBE_AUTO_RECONNECT:-}" "${LOCAL_DIRECT_PROBE_EXPIRES_AT_MS:-}" "${LOCAL_DIRECT_PROBE_PENDING_COUNT:-}" "${LOCAL_DIRECT_PROBE_OVERDUE_COUNT:-}" \
    "${REMOTE_DIRECT_PROBE_PENDING:-}" "${REMOTE_DIRECT_PROBE_AFTER_MS:-}" "${REMOTE_DIRECT_PROBE_RETRY_COUNT:-}" "${REMOTE_DIRECT_PROBE_AUTO_RECONNECT:-}" "${REMOTE_DIRECT_PROBE_EXPIRES_AT_MS:-}" "${REMOTE_DIRECT_PROBE_PENDING_COUNT:-}" "${REMOTE_DIRECT_PROBE_OVERDUE_COUNT:-}" \
    "${LOCAL_NOSTR_TRAVERSAL_FAILURES:-}" "${LOCAL_NOSTR_TRAVERSAL_IN_COOLDOWN:-}" "${LOCAL_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS:-}" "${LOCAL_NOSTR_TRAVERSAL_LAST_SKEW_MS:-}" \
    "${REMOTE_NOSTR_TRAVERSAL_FAILURES:-}" "${REMOTE_NOSTR_TRAVERSAL_IN_COOLDOWN:-}" "${REMOTE_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS:-}" "${REMOTE_NOSTR_TRAVERSAL_LAST_SKEW_MS:-}" \
    "$pipeline_hard_events" "$pipeline_top_queue_wait_local" "$pipeline_top_queue_wait_remote"
  )
  first=1
  for field in "${row[@]}"; do
    if (( first )); then
      first=0
    else
      printf '\t' >>"$SUMMARY"
    fi
    printf '%s' "$field" >>"$SUMMARY"
  done
  printf '\n' >>"$SUMMARY"
}

write_metadata() {
  jq -nc \
    --arg started_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg remote_tunnel_ip "$REMOTE_TUNNEL_IP" \
    --arg local_tunnel_ip "$LOCAL_TUNNEL_IP" \
    --arg local_peer "$LOCAL_PEER" \
    --arg remote_peer "$REMOTE_PEER" \
    --arg expected_remote_underlay_ip "$EXPECTED_REMOTE_UNDERLAY_IP" \
    --arg expected_local_underlay_ip "$EXPECTED_LOCAL_UNDERLAY_IP" \
    --arg cpu_stress_enabled "$(is_true "$CPU_STRESS" && printf true || printf false)" \
    --arg cpu_stress_sides "$CPU_STRESS_SIDES" \
    --arg local_cpu_stress_workers "$LOCAL_CPU_STRESS_WORKERS_STARTED" \
    --arg remote_cpu_stress_workers "$REMOTE_CPU_STRESS_WORKERS_STARTED" \
    --arg max_consecutive_rekey_samples "$MAX_CONSECUTIVE_REKEY_SAMPLES" \
    --arg max_consecutive_direct_probe_overdue_samples "$MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES" \
    --arg max_srtt_age_ms "$MAX_SRTT_AGE_MS" \
    --arg max_priority_queue_wait_ms "$MAX_PRIORITY_QUEUE_WAIT_MS" \
    --arg fail_on_priority_hard_events "$(is_true "$FAIL_ON_PRIORITY_HARD_EVENTS" && printf true || printf false)" \
    --arg max_fips_last_seen_age_secs "$MAX_FIPS_LAST_SEEN_AGE_SECS" \
    --arg max_fips_control_last_seen_age_secs "$MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS" \
    --arg max_fips_data_last_seen_age_secs "$MAX_FIPS_DATA_LAST_SEEN_AGE_SECS" \
    --arg max_fips_last_seen_future_skew_secs "$MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS" \
    '{
      started_at: $started_at,
      local_peer: $local_peer,
      remote_peer: $remote_peer,
      local_tunnel_ip: $local_tunnel_ip,
      remote_tunnel_ip: $remote_tunnel_ip,
      expected_local_underlay_ip: $expected_local_underlay_ip,
      expected_remote_underlay_ip: $expected_remote_underlay_ip,
      cpu_stress: {
        enabled: ($cpu_stress_enabled == "true"),
        sides: $cpu_stress_sides,
        local_workers: ($local_cpu_stress_workers | tonumber),
        remote_workers: ($remote_cpu_stress_workers | tonumber)
      },
      max_consecutive_rekey_samples: ($max_consecutive_rekey_samples | tonumber),
      max_consecutive_direct_probe_overdue_samples: ($max_consecutive_direct_probe_overdue_samples | tonumber),
      max_srtt_age_ms: ($max_srtt_age_ms | tonumber),
      max_priority_queue_wait_ms: ($max_priority_queue_wait_ms | tonumber),
      fail_on_priority_hard_events: ($fail_on_priority_hard_events == "true"),
      max_fips_last_seen_age_secs: ($max_fips_last_seen_age_secs | tonumber),
      max_fips_control_last_seen_age_secs: ($max_fips_control_last_seen_age_secs | tonumber),
      max_fips_data_last_seen_age_secs: ($max_fips_data_last_seen_age_secs | tonumber),
      max_fips_last_seen_future_skew_secs: ($max_fips_last_seen_future_skew_secs | tonumber)
    }' >"$OUTPUT_DIR/metadata.json"
}

preflight_record() {
  local status="$1"
  local label="$2"
  local detail="${3:-}"
  PREFLIGHT_ROWS+=("$status"$'\t'"$(tsv_escape "$label")"$'\t'"$(tsv_escape "$detail")")
  if [[ -n "$detail" ]]; then
    printf '[%s] %s (%s)\n' "$status" "$label" "$detail"
  else
    printf '[%s] %s\n' "$status" "$label"
  fi
  if [[ "$status" != "ok" ]]; then
    PREFLIGHT_RESULT=1
  fi
}

preflight_ok() {
  preflight_record ok "$1" "${2:-}"
}

preflight_missing() {
  preflight_record missing "$1" "${2:-}"
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

preflight_remote_cmd() {
  local label="$1"
  local cmd="$2"
  if remote_sh "$cmd" >/dev/null 2>&1; then
    preflight_ok "$label"
  else
    preflight_missing "$label"
  fi
}

write_preflight_rows() {
  local path="$OUTPUT_DIR/preflight.tsv"
  local row
  mkdir -p "$OUTPUT_DIR"
  printf 'status\tlabel\tdetail\n' >"$path"
  for row in "${PREFLIGHT_ROWS[@]}"; do
    printf '%s\n' "$row" >>"$path"
  done
  printf '%s\n' "$path"
}

preflight_check_daemon_peer_health() {
  local status="$1"
  local label_prefix="$2"
  local total_peers reachable_peers transport_peers
  total_peers="$(jq -r '[.daemon.state.peers[]?] | length' <<<"$status")"
  reachable_peers="$(jq -r '[.daemon.state.peers[]? | select(.reachable == true)] | length' <<<"$status")"
  transport_peers="$(jq -r '[.daemon.state.peers[]? | select(((.fips_transport_addr // "") != ""))] | length' <<<"$status")"

  if jq -e 'any(.daemon.state.peers[]?; .reachable == true)' >/dev/null 2>&1 <<<"$status"; then
    preflight_ok "$label_prefix daemon has at least one reachable FIPS peer" \
      "reachable_peers=$reachable_peers total_peers=$total_peers"
  else
    preflight_missing "$label_prefix daemon has at least one reachable FIPS peer" \
      "reachable_peers=$reachable_peers total_peers=$total_peers"
  fi

  if jq -e 'any(.daemon.state.peers[]?; ((.fips_transport_addr // "") != ""))' >/dev/null 2>&1 <<<"$status"; then
    preflight_ok "$label_prefix daemon has at least one FIPS transport address" \
      "transport_peers=$transport_peers total_peers=$total_peers"
  else
    preflight_missing "$label_prefix daemon has at least one FIPS transport address" \
      "transport_peers=$transport_peers total_peers=$total_peers"
  fi
}

preflight_check_selected_peer_path_details() {
  local status="$1"
  local peer="$2"
  local expected_ip="$3"
  local label_prefix="$4"
  local reachable transport_addr srtt srtt_age_ms

  reachable="$(peer_field "$status" "$peer" reachable)"
  transport_addr="$(peer_field "$status" "$peer" fips_transport_addr)"
  srtt="$(peer_field "$status" "$peer" fips_srtt_ms)"
  srtt_age_ms="$(peer_field "$status" "$peer" fips_srtt_age_ms)"

  if [[ "$reachable" == "true" ]]; then
    preflight_ok "$label_prefix selected peer is reachable" "peer=$peer reachable=$reachable"
  else
    preflight_missing "$label_prefix selected peer is reachable" "peer=$peer reachable=${reachable:-missing}"
  fi

  if [[ "$ALLOW_NON_DIRECT" == "0" && -n "$expected_ip" ]]; then
    if [[ "$transport_addr" == "$expected_ip:"* ]]; then
      preflight_ok "$label_prefix selected peer uses expected direct path" \
        "peer=$peer expected_ip=$expected_ip transport_addr=${transport_addr:-missing}"
    else
      preflight_missing "$label_prefix selected peer uses expected direct path" \
        "peer=$peer expected_ip=$expected_ip transport_addr=${transport_addr:-missing}"
    fi
  fi

  if [[ "$srtt" != "null" && -n "$srtt" ]]; then
    if assert_float_at_most "$srtt" "$MAX_SRTT_MS" "$label_prefix selected peer FIPS SRTT ms" >/dev/null 2>&1; then
      preflight_ok "$label_prefix selected peer FIPS SRTT is within threshold" \
        "peer=$peer srtt_ms=$srtt max_ms=$MAX_SRTT_MS"
    else
      preflight_missing "$label_prefix selected peer FIPS SRTT is within threshold" \
        "peer=$peer srtt_ms=$srtt max_ms=$MAX_SRTT_MS"
    fi
    if [[ "$srtt_age_ms" == "null" || -z "$srtt_age_ms" ]]; then
      preflight_missing "$label_prefix selected peer FIPS SRTT freshness is available" \
        "peer=$peer fips_srtt_age_ms=missing"
    elif assert_float_at_most "$srtt_age_ms" "$MAX_SRTT_AGE_MS" "$label_prefix selected peer FIPS SRTT age ms" >/dev/null 2>&1; then
      preflight_ok "$label_prefix selected peer FIPS SRTT sample is fresh" \
        "peer=$peer fips_srtt_age_ms=$srtt_age_ms max_ms=$MAX_SRTT_AGE_MS"
    else
      preflight_missing "$label_prefix selected peer FIPS SRTT sample is fresh" \
        "peer=$peer fips_srtt_age_ms=$srtt_age_ms max_ms=$MAX_SRTT_AGE_MS"
    fi
  fi
}

preflight_check_status_side() {
  local side="$1"
  local peer_var="$2"
  local expected_ip="$3"
  local status selected tunnel_ip peer_count reachable transport_addr srtt srtt_age_ms
  local label_prefix="$side"

  if ! status="$(status_for_side "$side" 2>/dev/null)"; then
    preflight_missing "$label_prefix daemon status JSON is available" "side=$side"
    return
  fi
  if ! jq -e '.status_source == "daemon" and (.daemon.state.peers | type == "array")' >/dev/null 2>&1 <<<"$status"; then
    preflight_missing "$label_prefix daemon status JSON is from daemon state" \
      "status_source=$(jq -r '.status_source // "missing"' <<<"$status" 2>/dev/null || printf invalid)"
    return
  fi
  peer_count="$(jq -r '[.daemon.state.peers[]?] | length' <<<"$status")"
  preflight_ok "$label_prefix daemon status JSON is from daemon state" "peer_count=$peer_count"

  preflight_check_daemon_peer_health "$status" "$label_prefix"

  if ! selected="$(select_peer "$status" "${!peer_var}" "$side" "$expected_ip" 2>&1)"; then
    preflight_missing "$label_prefix peer selector resolves one daemon peer" \
      "selector=${!peer_var:-auto} expected_ip=${expected_ip:-any} peer_count=$peer_count error=$selected"
    return
  fi
  printf -v "$peer_var" '%s' "$selected"
  preflight_ok "$label_prefix peer selector resolves one daemon peer" \
    "selector=${!peer_var:-auto} selected_peer=$selected expected_ip=${expected_ip:-any} peer_count=$peer_count"

  tunnel_ip="$(peer_tunnel_ip "$status" "$selected")"
  if [[ -n "$tunnel_ip" ]]; then
    preflight_ok "$label_prefix peer tunnel IP is present" "peer=$selected tunnel_ip=$tunnel_ip"
  else
    preflight_missing "$label_prefix peer tunnel IP is present" "peer=$selected tunnel_ip=missing"
  fi

  preflight_check_selected_peer_path_details "$status" "$selected" "$expected_ip" "$label_prefix"

  reachable="$(peer_field "$status" "$selected" reachable)"
  transport_addr="$(peer_field "$status" "$selected" fips_transport_addr)"
  srtt="$(peer_field "$status" "$selected" fips_srtt_ms)"
  srtt_age_ms="$(peer_field "$status" "$selected" fips_srtt_age_ms)"
  if (assert_peer_path "$status" "$selected" "$expected_ip" "$side") >/dev/null 2>&1; then
    preflight_ok "$label_prefix peer is reachable on expected path" \
      "peer=$selected reachable=${reachable:-missing} expected_ip=${expected_ip:-any} transport_addr=${transport_addr:-missing} srtt_ms=${srtt:-missing} fips_srtt_age_ms=${srtt_age_ms:-missing}"
  else
    preflight_missing "$label_prefix peer is reachable on expected path" \
      "peer=$selected reachable=${reachable:-missing} expected_ip=${expected_ip:-any} transport_addr=${transport_addr:-missing} srtt_ms=${srtt:-missing} fips_srtt_age_ms=${srtt_age_ms:-missing}"
  fi
}

run_preflight() {
  PREFLIGHT_ROWS=()
  PREFLIGHT_RESULT=0

  preflight_cmd "local jq is available" command -v jq
  preflight_cmd "local ssh is available" command -v ssh
  preflight_cmd "local ping is available" command -v ping
  if [[ -z "$LOCAL_NVPN_COMMAND" ]]; then
    preflight_cmd "local nvpn binary is available" command -v "$LOCAL_NVPN"
  else
    preflight_ok "local nvpn command is configured"
  fi

  if remote_sh "printf ok" >/dev/null 2>&1; then
    preflight_ok "remote SSH is reachable"
    preflight_remote_cmd "remote ping is available" "command -v ping"
    if [[ -z "$REMOTE_NVPN_COMMAND" ]]; then
      preflight_remote_cmd "remote nvpn binary is available" "command -v $(q "$REMOTE_NVPN")"
    else
      preflight_ok "remote nvpn command is configured"
    fi
    if [[ "$REQUIRE_IPERF" == "1" ]]; then
      preflight_cmd "local iperf3 is available" command -v iperf3
      preflight_remote_cmd "remote iperf3 is available" "command -v iperf3"
    else
      preflight_ok "iperf3 is optional for this run"
    fi
    if command -v jq >/dev/null 2>&1; then
      preflight_check_status_side local LOCAL_PEER "$EXPECTED_REMOTE_UNDERLAY_IP"
      preflight_check_status_side remote REMOTE_PEER "$EXPECTED_LOCAL_UNDERLAY_IP"
    fi
  else
    preflight_missing "remote SSH is reachable"
  fi

  local path
  path="$(write_preflight_rows)"
  if (( PREFLIGHT_RESULT == 0 )); then
    printf 'fips host-pair preflight passed: wrote %s\n' "$path"
  else
    printf 'fips host-pair preflight failed: wrote %s\n' "$path" >&2
  fi
  return "$PREFLIGHT_RESULT"
}

usage() {
  cat >&2 <<'EOF'
usage: NVPN_HOST_PAIR_SSH=user@host scripts/soak-fips-dataplane-host-pair.sh

Required:
  NVPN_HOST_PAIR_SSH or first arg    SSH target for the Linux/VM peer

Common optional env:
  NVPN_HOST_PAIR_LOCAL_NVPN          local nvpn binary (default: nvpn)
  NVPN_HOST_PAIR_REMOTE_NVPN         remote nvpn binary (default: nvpn)
  NVPN_HOST_PAIR_LOCAL_NVPN_COMMAND  local shell command before "status ..."
  NVPN_HOST_PAIR_REMOTE_NVPN_COMMAND remote shell command before "status ..."
  NVPN_HOST_PAIR_LOCAL_CONFIG        local config path
  NVPN_HOST_PAIR_REMOTE_CONFIG       remote config path
  NVPN_HOST_PAIR_LOCAL_DAEMON_LOG    local daemon log path for pipeline summaries
  NVPN_HOST_PAIR_REMOTE_DAEMON_LOG   remote daemon log path for pipeline summaries
  NVPN_HOST_PAIR_LOCAL_LOG_READ_COMMAND command before local daemon log path
  NVPN_HOST_PAIR_REMOTE_LOG_READ_COMMAND command before remote daemon log path
  NVPN_HOST_PAIR_REQUIRE_PIPELINE_LOGS fail when configured/discovered logs have no summaries
  NVPN_HOST_PAIR_LOCAL_PEER          remote participant pubkey as seen locally
  NVPN_HOST_PAIR_REMOTE_PEER         local participant pubkey as seen remotely
  NVPN_HOST_PAIR_EXPECTED_REMOTE_UNDERLAY_IP
  NVPN_HOST_PAIR_EXPECTED_LOCAL_UNDERLAY_IP
  NVPN_HOST_PAIR_ALLOW_QUEUE_EVENTS   allow hard queue/drop pipeline events
  NVPN_HOST_PAIR_ALLOW_QUEUE_WAIT     allow queue-wait threshold violations
  NVPN_HOST_PAIR_FAIL_ON_PRIORITY_HARD_EVENTS fail priority/control hard events even when queue events are allowed (default 1)
  NVPN_HOST_PAIR_MAX_PRIORITY_QUEUE_WAIT_MS fail priority/control queue wait max above this even when queue wait is allowed; 0 disables (default 50)
  NVPN_HOST_PAIR_MIN_IPERF_MBPS       default 0.001; set 0 to disable collapse guard
  NVPN_HOST_PAIR_MAX_CONSECUTIVE_IPERF_COLLAPSES default 2
  NVPN_HOST_PAIR_MAX_CONSECUTIVE_REKEY_SAMPLES default 2
  NVPN_HOST_PAIR_MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES default 2
  NVPN_HOST_PAIR_MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES default 2
  NVPN_HOST_PAIR_MAX_FIPS_LAST_SEEN_AGE_SECS default 180
  NVPN_HOST_PAIR_MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS default MAX_FIPS_LAST_SEEN_AGE_SECS
  NVPN_HOST_PAIR_MAX_FIPS_DATA_LAST_SEEN_AGE_SECS default MAX_FIPS_LAST_SEEN_AGE_SECS
  NVPN_HOST_PAIR_MAX_PING_P95_MS      default 500
  NVPN_HOST_PAIR_MAX_PING_P99_MS      default 750
  NVPN_HOST_PAIR_MAX_PIPELINE_QUEUE_WAIT_P95_MS default 50
  NVPN_HOST_PAIR_MAX_PIPELINE_QUEUE_WAIT_P99_MS default 100
  NVPN_HOST_PAIR_DURATION_SECS       default 300; use 1800/3600 for soak
  NVPN_HOST_PAIR_OUTPUT_DIR          artifact directory
  NVPN_HOST_PAIR_PREFLIGHT           set 1 to write preflight.tsv and exit
  NVPN_HOST_PAIR_CPU_STRESS          set 1 to run CPU stress during samples
  NVPN_HOST_PAIR_CPU_STRESS_SIDES    remote, local, or both (default: remote)
  NVPN_HOST_PAIR_CPU_STRESS_WORKERS  auto caps at 4, or explicit count
EOF
}

main() {
  case "${1:-}" in
    -h|--help)
      usage
      return 0
      ;;
  esac
  [[ -n "$REMOTE_SSH" ]] || {
    usage
    exit 2
  }
  if is_true "$PREFLIGHT"; then
    run_preflight
    return $?
  fi
  need_cmd jq
  need_cmd ssh
  if [[ -z "$LOCAL_NVPN_COMMAND" ]]; then
    need_cmd "$LOCAL_NVPN"
  fi

  mkdir -p "$OUTPUT_DIR"
  SAMPLES="$OUTPUT_DIR/samples.ndjson"
  SUMMARY="$OUTPUT_DIR/summary.tsv"
  LOCAL_CPU_STRESS_PID_FILE="$OUTPUT_DIR/local-cpu-stress.pids"
  REMOTE_CPU_STRESS_PID_FILE="/tmp/nvpn-fips-host-pair-cpu-stress.pids"
  : >"$SAMPLES"
  printf '%s\n' 'timestamp	iteration	ping_forward_loss_percent	ping_forward_avg_ms	ping_forward_p95_ms	ping_forward_p99_ms	ping_forward_max_ms	ping_reverse_loss_percent	ping_reverse_avg_ms	ping_reverse_p95_ms	ping_reverse_p99_ms	ping_reverse_max_ms	iperf_forward_mbps	iperf_forward_retrans	iperf_reverse_mbps	iperf_reverse_retrans	local_srtt_ms	remote_srtt_ms	local_bytes_sent	local_bytes_recv	remote_bytes_sent	remote_bytes_recv	local_cpu_percent	remote_cpu_percent	direct_path_checked	pipeline_log_checked	counter_progress_checked	iperf_forward_collapse_count	iperf_reverse_collapse_count	fips_liveness_checked	local_last_fips_seen_age_secs	remote_last_fips_seen_age_secs	fips_control_liveness_checked	local_last_fips_control_seen_age_secs	remote_last_fips_control_seen_age_secs	fips_data_liveness_checked	local_last_fips_data_seen_age_secs	remote_last_fips_data_seen_age_secs	local_rekey_in_progress	local_rekey_draining	local_current_k_bit	local_rekey_stuck_count	remote_rekey_in_progress	remote_rekey_draining	remote_current_k_bit	remote_rekey_stuck_count	local_direct_probe_pending	local_direct_probe_after_ms	local_direct_probe_retry_count	local_direct_probe_auto_reconnect	local_direct_probe_expires_at_ms	local_direct_probe_pending_count	local_direct_probe_overdue_count	remote_direct_probe_pending	remote_direct_probe_after_ms	remote_direct_probe_retry_count	remote_direct_probe_auto_reconnect	remote_direct_probe_expires_at_ms	remote_direct_probe_pending_count	remote_direct_probe_overdue_count	local_nostr_traversal_failures	local_nostr_traversal_in_cooldown	local_nostr_traversal_cooldown_until_ms	local_nostr_traversal_last_skew_ms	remote_nostr_traversal_failures	remote_nostr_traversal_in_cooldown	remote_nostr_traversal_cooldown_until_ms	remote_nostr_traversal_last_skew_ms	pipeline_hard_events	pipeline_top_queue_wait_local	pipeline_top_queue_wait_remote' >"$SUMMARY"
  trap on_exit EXIT

  if [[ "$REQUIRE_IPERF" == "1" ]]; then
    need_cmd iperf3
    side_has_cmd remote iperf3 || die "remote host is missing iperf3"
  elif side_has_cmd local iperf3 && side_has_cmd remote iperf3; then
    :
  else
    printf 'iperf3 missing on one side; continuing with ping/status only because NVPN_HOST_PAIR_REQUIRE_IPERF=%s\n' "$REQUIRE_IPERF" >&2
  fi

  local initial_local_status initial_remote_status
  initial_local_status="$(local_status)"
  initial_remote_status="$(remote_status)"
  if [[ -z "$LOCAL_DAEMON_LOG" ]]; then
    LOCAL_DAEMON_LOG="$(daemon_log_file "$initial_local_status")"
  fi
  if [[ -z "$REMOTE_DAEMON_LOG" ]]; then
    REMOTE_DAEMON_LOG="$(daemon_log_file "$initial_remote_status")"
  fi
  LOCAL_PEER="$(select_peer "$initial_local_status" "$LOCAL_PEER" "local" "$EXPECTED_REMOTE_UNDERLAY_IP")"
  REMOTE_PEER="$(select_peer "$initial_remote_status" "$REMOTE_PEER" "remote" "$EXPECTED_LOCAL_UNDERLAY_IP")"

  initial_local_status="$(wait_for_peer_status local "$LOCAL_PEER" "local peer")"
  initial_remote_status="$(wait_for_peer_status remote "$REMOTE_PEER" "remote peer")"
  REMOTE_TUNNEL_IP="$(peer_tunnel_ip "$initial_local_status" "$LOCAL_PEER")"
  LOCAL_TUNNEL_IP="$(peer_tunnel_ip "$initial_remote_status" "$REMOTE_PEER")"
  [[ -n "$REMOTE_TUNNEL_IP" ]] || die "unable to resolve remote tunnel IP from local status"
  [[ -n "$LOCAL_TUNNEL_IP" ]] || die "unable to resolve local tunnel IP from remote status"
  start_cpu_stress_if_enabled

  write_metadata

  local start iteration prev_local_sent prev_local_recv prev_remote_sent prev_remote_recv
  local baseline_fips_pipeline_local_count baseline_fips_pipeline_remote_count
  local baseline_nvpn_pipeline_local_count baseline_nvpn_pipeline_remote_count
  local prev_fips_pipeline_local_count="" prev_fips_pipeline_remote_count=""
  local prev_nvpn_pipeline_local_count="" prev_nvpn_pipeline_remote_count=""
  local stale_fips_pipeline_local_count=0 stale_fips_pipeline_remote_count=0
  local stale_nvpn_pipeline_local_count=0 stale_nvpn_pipeline_remote_count=0
  local baseline_forward_avg="" baseline_reverse_avg="" baseline_local_srtt="" baseline_remote_srtt=""
  IPERF_FORWARD_COLLAPSE_COUNT=0
  IPERF_REVERSE_COLLAPSE_COUNT=0
  LOCAL_REKEY_STUCK_COUNT=0
  REMOTE_REKEY_STUCK_COUNT=0
  start="$SECONDS"
  iteration=0
  prev_local_sent=""
  prev_local_recv=""
  prev_remote_sent=""
  prev_remote_recv=""
  baseline_fips_pipeline_local_count="$(pipeline_line_count local "$LOCAL_DAEMON_LOG" pipe)"
  baseline_fips_pipeline_remote_count="$(pipeline_line_count remote "$REMOTE_DAEMON_LOG" pipe)"
  baseline_nvpn_pipeline_local_count="$(pipeline_line_count local "$LOCAL_DAEMON_LOG" nvpn-pipe)"
  baseline_nvpn_pipeline_remote_count="$(pipeline_line_count remote "$REMOTE_DAEMON_LOG" nvpn-pipe)"

  while (( SECONDS - start < DURATION_SECS || iteration == 0 )); do
    iteration=$((iteration + 1))
    timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    CURRENT_ITERATION="$iteration"
    CURRENT_TIMESTAMP="$timestamp"
    LAST_LOCAL_PING_LOG="$OUTPUT_DIR/ping-local-to-remote-$iteration.log"
    LAST_REMOTE_PING_LOG="$OUTPUT_DIR/ping-remote-to-local-$iteration.log"
    LAST_FORWARD_IPERF_JSON="$OUTPUT_DIR/iperf-forward-$iteration.json"
    LAST_REVERSE_IPERF_JSON="$OUTPUT_DIR/iperf-reverse-$iteration.json"
    printf '%s host-pair sample %s\n' "$timestamp" "$iteration"

    local status_before_local status_before_remote
    status_before_local="$(wait_for_peer_status local "$LOCAL_PEER" "local peer")"
    status_before_remote="$(wait_for_peer_status remote "$REMOTE_PEER" "remote peer")"
    assert_peer_path "$status_before_local" "$LOCAL_PEER" "$EXPECTED_REMOTE_UNDERLAY_IP" "local"
    assert_peer_path "$status_before_remote" "$REMOTE_PEER" "$EXPECTED_LOCAL_UNDERLAY_IP" "remote"

    read -r PING_FORWARD_LOSS PING_FORWARD_AVG PING_FORWARD_P95 PING_FORWARD_P99 PING_FORWARD_MAX \
      <<<"$(ping_probe local "$REMOTE_TUNNEL_IP" "$LAST_LOCAL_PING_LOG")"
    read -r PING_REVERSE_LOSS PING_REVERSE_AVG PING_REVERSE_P95 PING_REVERSE_P99 PING_REVERSE_MAX \
      <<<"$(ping_probe remote "$LOCAL_TUNNEL_IP" "$LAST_REMOTE_PING_LOG")"
    assert_float_at_most "$PING_FORWARD_LOSS" "$MAX_PING_LOSS_PERCENT" "local->remote ping loss %"
    assert_float_at_most "$PING_REVERSE_LOSS" "$MAX_PING_LOSS_PERCENT" "remote->local ping loss %"
    assert_float_at_most "$PING_FORWARD_AVG" "$MAX_PING_AVG_MS" "local->remote ping avg ms"
    assert_float_at_most "$PING_REVERSE_AVG" "$MAX_PING_AVG_MS" "remote->local ping avg ms"
    assert_float_at_most "$PING_FORWARD_P95" "$MAX_PING_P95_MS" "local->remote ping p95 ms"
    assert_float_at_most "$PING_REVERSE_P95" "$MAX_PING_P95_MS" "remote->local ping p95 ms"
    assert_float_at_most "$PING_FORWARD_P99" "$MAX_PING_P99_MS" "local->remote ping p99 ms"
    assert_float_at_most "$PING_REVERSE_P99" "$MAX_PING_P99_MS" "remote->local ping p99 ms"
    assert_float_at_most "$PING_FORWARD_MAX" "$MAX_PING_MAX_MS" "local->remote ping max ms"
    assert_float_at_most "$PING_REVERSE_MAX" "$MAX_PING_MAX_MS" "remote->local ping max ms"
    if [[ -z "$baseline_forward_avg" ]]; then
      baseline_forward_avg="$PING_FORWARD_AVG"
      baseline_reverse_avg="$PING_REVERSE_AVG"
    else
      assert_float_drift_at_most "$PING_FORWARD_AVG" "$baseline_forward_avg" "$MAX_PING_AVG_DRIFT_MS" "$MAX_PING_AVG_DRIFT_FACTOR" "local->remote ping avg ms"
      assert_float_drift_at_most "$PING_REVERSE_AVG" "$baseline_reverse_avg" "$MAX_PING_AVG_DRIFT_MS" "$MAX_PING_AVG_DRIFT_FACTOR" "remote->local ping avg ms"
    fi

    IPERF_FORWARD_MBPS="null"
    IPERF_FORWARD_RETRANS="null"
    IPERF_REVERSE_MBPS="null"
    IPERF_REVERSE_RETRANS="null"
    if side_has_cmd local iperf3 && side_has_cmd remote iperf3; then
      start_remote_iperf_server
      read -r IPERF_FORWARD_MBPS IPERF_FORWARD_RETRANS \
        <<<"$(iperf_probe forward "$LAST_FORWARD_IPERF_JSON")"
      read -r IPERF_REVERSE_MBPS IPERF_REVERSE_RETRANS \
        <<<"$(iperf_probe reverse "$LAST_REVERSE_IPERF_JSON" -R)"
    fi
    record_iperf_progress "forward" "$IPERF_FORWARD_MBPS" IPERF_FORWARD_COLLAPSE_COUNT
    record_iperf_progress "reverse" "$IPERF_REVERSE_MBPS" IPERF_REVERSE_COLLAPSE_COUNT

    local status_after_local status_after_remote local_status_path remote_status_path
    status_after_local="$(wait_for_peer_status local "$LOCAL_PEER" "local peer")"
    status_after_remote="$(wait_for_peer_status remote "$REMOTE_PEER" "remote peer")"
    assert_peer_path "$status_after_local" "$LOCAL_PEER" "$EXPECTED_REMOTE_UNDERLAY_IP" "local"
    assert_peer_path "$status_after_remote" "$REMOTE_PEER" "$EXPECTED_LOCAL_UNDERLAY_IP" "remote"

    local_status_path="$OUTPUT_DIR/status-local-$iteration.json"
    remote_status_path="$OUTPUT_DIR/status-remote-$iteration.json"
    printf '%s\n' "$status_after_local" >"$local_status_path"
    printf '%s\n' "$status_after_remote" >"$remote_status_path"

    LOCAL_TRANSPORT_ADDR="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_transport_addr)"
    REMOTE_TRANSPORT_ADDR="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_transport_addr)"
    LOCAL_SRTT="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_srtt_ms)"
    REMOTE_SRTT="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_srtt_ms)"
    LOCAL_SRTT_AGE_MS="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_srtt_age_ms)"
    REMOTE_SRTT_AGE_MS="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_srtt_age_ms)"
    LOCAL_BYTES_SENT="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_bytes_sent)"
    LOCAL_BYTES_RECV="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_bytes_recv)"
    REMOTE_BYTES_SENT="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_bytes_sent)"
    REMOTE_BYTES_RECV="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_bytes_recv)"
    LOCAL_LAST_MESH_SEEN_AT="$(peer_field "$status_after_local" "$LOCAL_PEER" last_mesh_seen_at)"
    LOCAL_LAST_FIPS_SEEN_AT="$(peer_field "$status_after_local" "$LOCAL_PEER" last_fips_seen_at)"
    LOCAL_LAST_FIPS_CONTROL_SEEN_AT="$(peer_field "$status_after_local" "$LOCAL_PEER" last_fips_control_seen_at)"
    LOCAL_LAST_FIPS_DATA_SEEN_AT="$(peer_field "$status_after_local" "$LOCAL_PEER" last_fips_data_seen_at)"
    LOCAL_LAST_HANDSHAKE_AT="$(peer_field "$status_after_local" "$LOCAL_PEER" last_handshake_at)"
    REMOTE_LAST_MESH_SEEN_AT="$(peer_field "$status_after_remote" "$REMOTE_PEER" last_mesh_seen_at)"
    REMOTE_LAST_FIPS_SEEN_AT="$(peer_field "$status_after_remote" "$REMOTE_PEER" last_fips_seen_at)"
    REMOTE_LAST_FIPS_CONTROL_SEEN_AT="$(peer_field "$status_after_remote" "$REMOTE_PEER" last_fips_control_seen_at)"
    REMOTE_LAST_FIPS_DATA_SEEN_AT="$(peer_field "$status_after_remote" "$REMOTE_PEER" last_fips_data_seen_at)"
    REMOTE_LAST_HANDSHAKE_AT="$(peer_field "$status_after_remote" "$REMOTE_PEER" last_handshake_at)"
    LOCAL_SAMPLE_NOW_SECS="$(side_epoch_secs local)"
    REMOTE_SAMPLE_NOW_SECS="$(side_epoch_secs remote)"
    LOCAL_LAST_FIPS_SEEN_AGE_SECS="$(fips_last_seen_age_secs "$LOCAL_LAST_FIPS_SEEN_AT" "$LOCAL_SAMPLE_NOW_SECS")"
    REMOTE_LAST_FIPS_SEEN_AGE_SECS="$(fips_last_seen_age_secs "$REMOTE_LAST_FIPS_SEEN_AT" "$REMOTE_SAMPLE_NOW_SECS")"
    LOCAL_LAST_FIPS_CONTROL_SEEN_AGE_SECS="$(fips_last_seen_age_secs "$LOCAL_LAST_FIPS_CONTROL_SEEN_AT" "$LOCAL_SAMPLE_NOW_SECS")"
    REMOTE_LAST_FIPS_CONTROL_SEEN_AGE_SECS="$(fips_last_seen_age_secs "$REMOTE_LAST_FIPS_CONTROL_SEEN_AT" "$REMOTE_SAMPLE_NOW_SECS")"
    LOCAL_LAST_FIPS_DATA_SEEN_AGE_SECS="$(fips_last_seen_age_secs "$LOCAL_LAST_FIPS_DATA_SEEN_AT" "$LOCAL_SAMPLE_NOW_SECS")"
    REMOTE_LAST_FIPS_DATA_SEEN_AGE_SECS="$(fips_last_seen_age_secs "$REMOTE_LAST_FIPS_DATA_SEEN_AT" "$REMOTE_SAMPLE_NOW_SECS")"
    LOCAL_REKEY_IN_PROGRESS="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_rekey_in_progress)"
    LOCAL_REKEY_DRAINING="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_rekey_draining)"
    LOCAL_CURRENT_K_BIT="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_current_k_bit)"
    LOCAL_DIRECT_PROBE_PENDING="$(peer_field "$status_after_local" "$LOCAL_PEER" direct_probe_pending)"
    LOCAL_DIRECT_PROBE_AFTER_MS="$(peer_field "$status_after_local" "$LOCAL_PEER" direct_probe_after_ms)"
    LOCAL_DIRECT_PROBE_RETRY_COUNT="$(peer_field "$status_after_local" "$LOCAL_PEER" direct_probe_retry_count)"
    LOCAL_DIRECT_PROBE_AUTO_RECONNECT="$(peer_field "$status_after_local" "$LOCAL_PEER" direct_probe_auto_reconnect)"
    LOCAL_DIRECT_PROBE_EXPIRES_AT_MS="$(peer_field "$status_after_local" "$LOCAL_PEER" direct_probe_expires_at_ms)"
    LOCAL_NOSTR_TRAVERSAL_FAILURES="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_nostr_traversal_failures)"
    LOCAL_NOSTR_TRAVERSAL_IN_COOLDOWN="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_nostr_traversal_in_cooldown)"
    LOCAL_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_nostr_traversal_cooldown_until_ms)"
    LOCAL_NOSTR_TRAVERSAL_LAST_SKEW_MS="$(peer_field "$status_after_local" "$LOCAL_PEER" fips_nostr_traversal_last_observed_skew_ms)"
    REMOTE_REKEY_IN_PROGRESS="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_rekey_in_progress)"
    REMOTE_REKEY_DRAINING="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_rekey_draining)"
    REMOTE_CURRENT_K_BIT="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_current_k_bit)"
    REMOTE_DIRECT_PROBE_PENDING="$(peer_field "$status_after_remote" "$REMOTE_PEER" direct_probe_pending)"
    REMOTE_DIRECT_PROBE_AFTER_MS="$(peer_field "$status_after_remote" "$REMOTE_PEER" direct_probe_after_ms)"
    REMOTE_DIRECT_PROBE_RETRY_COUNT="$(peer_field "$status_after_remote" "$REMOTE_PEER" direct_probe_retry_count)"
    REMOTE_DIRECT_PROBE_AUTO_RECONNECT="$(peer_field "$status_after_remote" "$REMOTE_PEER" direct_probe_auto_reconnect)"
    REMOTE_DIRECT_PROBE_EXPIRES_AT_MS="$(peer_field "$status_after_remote" "$REMOTE_PEER" direct_probe_expires_at_ms)"
    REMOTE_NOSTR_TRAVERSAL_FAILURES="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_nostr_traversal_failures)"
    REMOTE_NOSTR_TRAVERSAL_IN_COOLDOWN="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_nostr_traversal_in_cooldown)"
    REMOTE_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_nostr_traversal_cooldown_until_ms)"
    REMOTE_NOSTR_TRAVERSAL_LAST_SKEW_MS="$(peer_field "$status_after_remote" "$REMOTE_PEER" fips_nostr_traversal_last_observed_skew_ms)"
    sample_now_ms="$(epoch_ms)"
    assert_fips_liveness_fresh "local FIPS" "$LOCAL_LAST_FIPS_SEEN_AT" "$LOCAL_SAMPLE_NOW_SECS"
    assert_fips_liveness_fresh "remote FIPS" "$REMOTE_LAST_FIPS_SEEN_AT" "$REMOTE_SAMPLE_NOW_SECS"
    assert_fips_control_liveness_fresh "local FIPS" "$LOCAL_LAST_FIPS_CONTROL_SEEN_AT" "$LOCAL_SAMPLE_NOW_SECS"
    assert_fips_control_liveness_fresh "remote FIPS" "$REMOTE_LAST_FIPS_CONTROL_SEEN_AT" "$REMOTE_SAMPLE_NOW_SECS"
    assert_fips_data_liveness_fresh "local FIPS" "$LOCAL_LAST_FIPS_DATA_SEEN_AT" "$LOCAL_SAMPLE_NOW_SECS"
    assert_fips_data_liveness_fresh "remote FIPS" "$REMOTE_LAST_FIPS_DATA_SEEN_AT" "$REMOTE_SAMPLE_NOW_SECS"
    record_rekey_progress "local FIPS" "$LOCAL_REKEY_IN_PROGRESS" "$LOCAL_REKEY_DRAINING" LOCAL_REKEY_STUCK_COUNT
    record_rekey_progress "remote FIPS" "$REMOTE_REKEY_IN_PROGRESS" "$REMOTE_REKEY_DRAINING" REMOTE_REKEY_STUCK_COUNT
    record_direct_probe_progress "local FIPS" "$LOCAL_DIRECT_PROBE_PENDING" "$LOCAL_DIRECT_PROBE_AFTER_MS" "$sample_now_ms" LOCAL_DIRECT_PROBE_PENDING_COUNT LOCAL_DIRECT_PROBE_OVERDUE_COUNT
    record_direct_probe_progress "remote FIPS" "$REMOTE_DIRECT_PROBE_PENDING" "$REMOTE_DIRECT_PROBE_AFTER_MS" "$sample_now_ms" REMOTE_DIRECT_PROBE_PENDING_COUNT REMOTE_DIRECT_PROBE_OVERDUE_COUNT
    LOCAL_CPU="$(daemon_cpu_percent local)"
    REMOTE_CPU="$(daemon_cpu_percent remote)"
    FIPS_PIPELINE_LOCAL_COUNT="$(pipeline_line_count local "$LOCAL_DAEMON_LOG" pipe)"
    FIPS_PIPELINE_REMOTE_COUNT="$(pipeline_line_count remote "$REMOTE_DAEMON_LOG" pipe)"
    NVPN_PIPELINE_LOCAL_COUNT="$(pipeline_line_count local "$LOCAL_DAEMON_LOG" nvpn-pipe)"
    NVPN_PIPELINE_REMOTE_COUNT="$(pipeline_line_count remote "$REMOTE_DAEMON_LOG" nvpn-pipe)"
    FIPS_PIPELINE_LOCAL=""
    FIPS_PIPELINE_REMOTE=""
    NVPN_PIPELINE_LOCAL=""
    NVPN_PIPELINE_REMOTE=""
    if pipeline_count_has_advanced "$FIPS_PIPELINE_LOCAL_COUNT" "$baseline_fips_pipeline_local_count"; then
      FIPS_PIPELINE_LOCAL="$(pipeline_latest_line local "$LOCAL_DAEMON_LOG" pipe)"
    fi
    if pipeline_count_has_advanced "$FIPS_PIPELINE_REMOTE_COUNT" "$baseline_fips_pipeline_remote_count"; then
      FIPS_PIPELINE_REMOTE="$(pipeline_latest_line remote "$REMOTE_DAEMON_LOG" pipe)"
    fi
    if pipeline_count_has_advanced "$NVPN_PIPELINE_LOCAL_COUNT" "$baseline_nvpn_pipeline_local_count"; then
      NVPN_PIPELINE_LOCAL="$(pipeline_latest_line local "$LOCAL_DAEMON_LOG" nvpn-pipe)"
    fi
    if pipeline_count_has_advanced "$NVPN_PIPELINE_REMOTE_COUNT" "$baseline_nvpn_pipeline_remote_count"; then
      NVPN_PIPELINE_REMOTE="$(pipeline_latest_line remote "$REMOTE_DAEMON_LOG" nvpn-pipe)"
    fi
    FIPS_PIPELINE_LOCAL_QUEUE_WAIT="$(pipeline_queue_wait_json "$FIPS_PIPELINE_LOCAL")"
    FIPS_PIPELINE_REMOTE_QUEUE_WAIT="$(pipeline_queue_wait_json "$FIPS_PIPELINE_REMOTE")"
    NVPN_PIPELINE_LOCAL_QUEUE_WAIT="$(pipeline_queue_wait_json "$NVPN_PIPELINE_LOCAL")"
    NVPN_PIPELINE_REMOTE_QUEUE_WAIT="$(pipeline_queue_wait_json "$NVPN_PIPELINE_REMOTE")"
    assert_pipeline_present_if_required "local" "$LOCAL_DAEMON_LOG" "$FIPS_PIPELINE_LOCAL" "$NVPN_PIPELINE_LOCAL"
    assert_pipeline_present_if_required "remote" "$REMOTE_DAEMON_LOG" "$FIPS_PIPELINE_REMOTE" "$NVPN_PIPELINE_REMOTE"
    if [[ "$REQUIRE_PIPELINE_LOGS" == "1" ]] || pipeline_count_has_advanced "$FIPS_PIPELINE_LOCAL_COUNT" "$baseline_fips_pipeline_local_count"; then
      assert_pipeline_fresh "local FIPS" "$FIPS_PIPELINE_LOCAL_COUNT" prev_fips_pipeline_local_count stale_fips_pipeline_local_count
    fi
    if [[ "$REQUIRE_PIPELINE_LOGS" == "1" ]] || pipeline_count_has_advanced "$FIPS_PIPELINE_REMOTE_COUNT" "$baseline_fips_pipeline_remote_count"; then
      assert_pipeline_fresh "remote FIPS" "$FIPS_PIPELINE_REMOTE_COUNT" prev_fips_pipeline_remote_count stale_fips_pipeline_remote_count
    fi
    if [[ "$REQUIRE_PIPELINE_LOGS" == "1" ]] || pipeline_count_has_advanced "$NVPN_PIPELINE_LOCAL_COUNT" "$baseline_nvpn_pipeline_local_count"; then
      assert_pipeline_fresh "local nvpn" "$NVPN_PIPELINE_LOCAL_COUNT" prev_nvpn_pipeline_local_count stale_nvpn_pipeline_local_count
    fi
    if [[ "$REQUIRE_PIPELINE_LOGS" == "1" ]] || pipeline_count_has_advanced "$NVPN_PIPELINE_REMOTE_COUNT" "$baseline_nvpn_pipeline_remote_count"; then
      assert_pipeline_fresh "remote nvpn" "$NVPN_PIPELINE_REMOTE_COUNT" prev_nvpn_pipeline_remote_count stale_nvpn_pipeline_remote_count
    fi
    assert_pipeline_ok "local FIPS" "$FIPS_PIPELINE_LOCAL"
    assert_pipeline_ok "remote FIPS" "$FIPS_PIPELINE_REMOTE"
    assert_pipeline_ok "local nvpn" "$NVPN_PIPELINE_LOCAL"
    assert_pipeline_ok "remote nvpn" "$NVPN_PIPELINE_REMOTE"
    assert_float_at_most "$LOCAL_CPU" "$MAX_CPU_PERCENT" "local daemon CPU %"
    assert_float_at_most "$REMOTE_CPU" "$MAX_CPU_PERCENT" "remote daemon CPU %"

    if [[ -z "$baseline_local_srtt" ]]; then
      baseline_local_srtt="$LOCAL_SRTT"
      baseline_remote_srtt="$REMOTE_SRTT"
    else
      assert_float_drift_at_most "$LOCAL_SRTT" "$baseline_local_srtt" "$MAX_SRTT_DRIFT_MS" "$MAX_SRTT_DRIFT_FACTOR" "local FIPS SRTT ms"
      assert_float_drift_at_most "$REMOTE_SRTT" "$baseline_remote_srtt" "$MAX_SRTT_DRIFT_MS" "$MAX_SRTT_DRIFT_FACTOR" "remote FIPS SRTT ms"
    fi

    if [[ -n "$prev_local_sent" ]]; then
      assert_counter_progress local "$LOCAL_BYTES_SENT" "$LOCAL_BYTES_RECV" "$prev_local_sent" "$prev_local_recv"
      assert_counter_progress remote "$REMOTE_BYTES_SENT" "$REMOTE_BYTES_RECV" "$prev_remote_sent" "$prev_remote_recv"
    fi
    prev_local_sent="$LOCAL_BYTES_SENT"
    prev_local_recv="$LOCAL_BYTES_RECV"
    prev_remote_sent="$REMOTE_BYTES_SENT"
    prev_remote_recv="$REMOTE_BYTES_RECV"

    write_sample "$iteration" "$timestamp" "$local_status_path" "$remote_status_path" >>"$SAMPLES"
    write_summary_row

    if (( SECONDS - start >= DURATION_SECS )); then
      break
    fi
    sleep "$INTERVAL_SECS"
  done

  cleanup_remote_iperf
  printf 'fips host-pair soak passed: wrote samples to %s and summary to %s\n' "$SAMPLES" "$SUMMARY"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
