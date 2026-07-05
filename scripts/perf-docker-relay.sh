#!/usr/bin/env bash
# 3-node nvpn+FIPS overlay perf bench: A and B can only reach each other through C.
#
# Topology (docker bridge 10.203.0.0/24):
#   A (.10)  ──┐                    ┌──  B (.11)
#              └──> C (.12) <───────┘
#
# A and B are the only private-mesh roster peers. C is configured only as a
# static non-roster FIPS transit seed, so it can ferry endpoint/session
# traffic without receiving private-network routes. Direct A<->B underlay
# traffic is dropped; every A<->B tunnel byte must cross C.
#
# iperf3 between A's mesh tunnel IP and B's mesh tunnel IP exercises that
# transit path.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"
PROJECT_NAME="${PROJECT_NAME:-nvpn-perf-relay}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-perf-relay"
FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}"
DURATION="${DURATION:-10}"
IPERF_INTERVAL_SECS="${NVPN_DOCKER_IPERF_INTERVAL_SECS:-0}"
IPERF_TIMEOUT_SECS="${NVPN_DOCKER_IPERF_TIMEOUT_SECS:-$((DURATION + 30))}"
NVPN_DOCKER_IPERF_TIMEOUT_SECS="$IPERF_TIMEOUT_SECS"
OUTPUT_DIR="${NVPN_DOCKER_RELAY_OUTPUT_DIR:-${NVPN_DOCKER_OUTPUT_DIR:-$ROOT_DIR/artifacts/nvpn-docker-relay/$(date -u +%Y%m%dT%H%M%SZ)}}"
RAW_DIR="$OUTPUT_DIR/raw"
SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
PIPELINE_PHASE_RANGES="$RAW_DIR/nvpn-relay-pipeline-phase-ranges.tsv"
PIPELINE_PHASE_SUMMARY="$RAW_DIR/nvpn-relay-pipeline-phase-summary.tsv"
PIPELINE_TRACE="${NVPN_DOCKER_PIPELINE_TRACE:-0}"
PIPELINE_INTERVAL_SECS="${NVPN_DOCKER_PIPELINE_INTERVAL_SECS:-5}"
EXTRA_CONNECT_ENV="${NVPN_DOCKER_EXTRA_ENV:-}"
NVPN_DOCKER_PIPELINE_TRACE="$PIPELINE_TRACE"
NVPN_DOCKER_PIPELINE_INTERVAL_SECS="$PIPELINE_INTERVAL_SECS"
MIN_TRANSIT_BYTES="${NVPN_DOCKER_RELAY_MIN_TRANSIT_BYTES:-10000000}"
COUNTER_A_TO_C="nvpn-relay-a-to-c"
COUNTER_C_TO_B="nvpn-relay-c-to-b"
DIAGNOSTICS_READY=0
DIAGNOSTICS_CAPTURED=0
PIPELINE_START_NODE_A=0
PIPELINE_START_NODE_B=0
PIPELINE_START_NODE_C=0
TRANSIT_A_TO_C_BEFORE=0
TRANSIT_C_TO_B_BEFORE=0

cleanup() {
  local status=$?
  if declare -F capture_nvpn_diagnostics >/dev/null; then
    capture_nvpn_diagnostics "$status" || true
  fi
  if ! is_true "${KEEP:-0}"; then
    docker_bench_stop_cpu_stress
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

if ! docker_bench_validate_extra_env_assignments "$EXTRA_CONNECT_ENV"; then
  exit 2
fi

wait_for_service() {
  local service="$1"
  for _ in $(seq 1 30); do
    cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$cid" ]] && [[ "$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "perf-relay: service '$service' did not start" >&2
  exit 1
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

install_udp_output_counter() {
  local service="$1"
  local peer_ip="$2"
  local comment="$3"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- "$peer_ip" "$comment" <<'SH'
set -eu
peer_ip="$1"
comment="$2"
while iptables -D OUTPUT -p udp -d "$peer_ip" --dport 51820 -m comment --comment "$comment" 2>/dev/null; do :; done
iptables -I OUTPUT 1 -p udp -d "$peer_ip" --dport 51820 -m comment --comment "$comment"
SH
}

udp_output_counter_bytes() {
  local service="$1"
  local comment="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- "$comment" <<'SH' | tr -d '\r'
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

assert_transit_counter_delta() {
  local service="$1"
  local comment="$2"
  local before="$3"
  local min_bytes="$4"
  local label="$5"
  local after delta
  after="$(udp_output_counter_bytes "$service" "$comment")"
  delta=$((after - before))
  printf '%s UDP underlay bytes: +%s (total=%s)\n' "$label" "$delta" "$after"
  if (( delta < min_bytes )); then
    echo "perf-relay: $label UDP underlay byte delta $delta below required $min_bytes; transit path not proven" >&2
    exit 1
  fi
  LAST_TRANSIT_DELTA="$delta"
}

write_transit_counter_summary() {
  local a_delta="$1"
  local c_delta="$2"
  [[ -n "$RAW_DIR" ]] || return 0
  printf '%s\t%s\t%s\n' service label bytes_delta >"$RAW_DIR/nvpn-relay-transit-counters.tsv"
  printf '%s\t%s\t%s\n' node-a a-to-c "$a_delta" >>"$RAW_DIR/nvpn-relay-transit-counters.tsv"
  printf '%s\t%s\t%s\n' node-c c-to-b "$c_delta" >>"$RAW_DIR/nvpn-relay-transit-counters.tsv"
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
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    phase \
    node_a_start \
    node_a_end \
    node_b_start \
    node_b_end \
    node_c_start \
    node_c_end >"$path"
}

write_pipeline_summary_header() {
  local path="$1"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
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
    hard_events \
    selected_load_pipeline >"$path"
}

write_pipeline_phase_summary_header() {
  local path="$1"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
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
    hard_events \
    selected_load_pipeline >"$path"
}

capture_pipeline_for_service() {
  local service="$1"
  local start_line="$2"
  local log_path="$3"
  local summary_path="$4"
  local prefix="$RAW_DIR/nvpn-relay-$service"
  local all_lines_path="$prefix-pipeline-lines.txt"
  local bench_lines_path="$prefix-pipeline-benchmark-lines.txt"
  local load_path="$prefix-pipeline-load-selected.txt"
  local peak_path="$prefix-pipeline-peak-wait-selected.txt"
  local all_count bench_count load_line peak_line
  local load_top peak_top fmp_batch fmp_spread decrypt_batch decrypt_spread decrypt_turn_mix fsp_worker_open_wait udp_send_batch
  local nvpn_tun_read_batch nvpn_mesh_send_batch nvpn_mesh_recv_batch nvpn_tun_write hard_events

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
  hard_events="$(docker_bench_pipeline_hard_event_summary_from_stdin "$start_line" <"$all_lines_path")"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
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
    "$(docker_bench_tsv_field "$hard_events")" \
    "$(docker_bench_tsv_field "$load_line")" >>"$summary_path"
}

append_pipeline_phase_range() {
  local phase="$1"
  local node_a_start="$2"
  local node_a_end="$3"
  local node_b_start="$4"
  local node_b_end="$5"
  local node_c_start="$6"
  local node_c_end="$7"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$phase" \
    "$node_a_start" \
    "$node_a_end" \
    "$node_b_start" \
    "$node_b_end" \
    "$node_c_start" \
    "$node_c_end" >>"$PIPELINE_PHASE_RANGES"
}

capture_pipeline_phase_for_service() {
  local phase="$1"
  local service="$2"
  local start_line="$3"
  local end_line="$4"
  local all_lines_path="$5"
  local summary_path="$6"
  local phase_lines_path="$RAW_DIR/nvpn-relay-$service-pipeline-$phase-lines.txt"
  local phase_count load_line peak_line
  local load_top peak_top fmp_batch fmp_spread decrypt_batch decrypt_spread decrypt_turn_mix fsp_worker_open_wait udp_send_batch
  local nvpn_tun_read_batch nvpn_mesh_send_batch nvpn_mesh_recv_batch nvpn_tun_write hard_events

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
  hard_events="$(docker_bench_pipeline_hard_event_summary_from_stdin "$start_line" "$end_line" <"$all_lines_path")"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
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
    "$(docker_bench_tsv_field "$hard_events")" \
    "$(docker_bench_tsv_field "$load_line")" >>"$summary_path"
}

write_pipeline_phase_summary() {
  local ranges_path="$1"
  local summary_path="$2"
  [[ -s "$ranges_path" ]] || return 0
  write_pipeline_phase_summary_header "$summary_path"
  local phase node_a_start node_a_end node_b_start node_b_end node_c_start node_c_end
  while IFS=$'\t' read -r phase node_a_start node_a_end node_b_start node_b_end node_c_start node_c_end; do
    [[ "$phase" != "phase" ]] || continue
    capture_pipeline_phase_for_service \
      "$phase" node-a "$node_a_start" "$node_a_end" \
      "$RAW_DIR/nvpn-relay-node-a-pipeline-lines.txt" "$summary_path"
    capture_pipeline_phase_for_service \
      "$phase" node-b "$node_b_start" "$node_b_end" \
      "$RAW_DIR/nvpn-relay-node-b-pipeline-lines.txt" "$summary_path"
    capture_pipeline_phase_for_service \
      "$phase" node-c "$node_c_start" "$node_c_end" \
      "$RAW_DIR/nvpn-relay-node-c-pipeline-lines.txt" "$summary_path"
  done <"$ranges_path"
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
    --arg node_c_pipeline_start "$PIPELINE_START_NODE_C" \
    '{
      captured_at: $captured_at,
      benchmark_exit_status: ($exit_status | tonumber),
      pipeline_start_lines: {
        "node-a": ($node_a_pipeline_start | tonumber),
        "node-b": ($node_b_pipeline_start | tonumber),
        "node-c": ($node_c_pipeline_start | tonumber)
      }
    }' >"$RAW_DIR/nvpn-relay-diagnostics.json" 2>/dev/null || true

  local pipeline_summary="$RAW_DIR/nvpn-relay-pipeline-summary.tsv"
  write_pipeline_summary_header "$pipeline_summary"

  local service prefix log_path status_path stderr_path start_line
  for service in node-a node-b node-c; do
    prefix="$RAW_DIR/nvpn-relay-$service"
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
    case "$service" in
      node-a) start_line="$PIPELINE_START_NODE_A" ;;
      node-b) start_line="$PIPELINE_START_NODE_B" ;;
      node-c) start_line="$PIPELINE_START_NODE_C" ;;
    esac
    capture_pipeline_for_service "$service" "$start_line" "$log_path" "$pipeline_summary"
  done
  write_pipeline_phase_summary "$PIPELINE_PHASE_RANGES" "$PIPELINE_PHASE_SUMMARY"
}

cleanup
docker_bench_init_summary
write_pipeline_phase_range_header "$PIPELINE_PHASE_RANGES"
docker_bench_write_metadata nvpn-relay "$DURATION"
"${COMPOSE[@]}" up -d node-a node-b node-c >/dev/null
for service in node-a node-b node-c; do
  wait_for_service "$service"
done

# Force the test to actually exercise the transit path. Block direct underlay
# traffic between A and B at the IP layer; C remains reachable from both.
"${COMPOSE[@]}" exec -T node-a iptables -I OUTPUT -d 10.203.0.11 -j DROP
"${COMPOSE[@]}" exec -T node-a iptables -I INPUT  -s 10.203.0.11 -j DROP
"${COMPOSE[@]}" exec -T node-b iptables -I OUTPUT -d 10.203.0.10 -j DROP
"${COMPOSE[@]}" exec -T node-b iptables -I INPUT  -s 10.203.0.10 -j DROP
install_udp_output_counter node-a 10.203.0.12 "$COUNTER_A_TO_C"
install_udp_output_counter node-c 10.203.0.11 "$COUNTER_C_TO_B"

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-c nvpn init --force >/dev/null
A_NPUB="$(nostr_pubkey_from_config node-a)"
B_NPUB="$(nostr_pubkey_from_config node-b)"
C_NPUB="$(nostr_pubkey_from_config node-c)"

# A and B are the only private-mesh participants. C is present in the FIPS
# endpoint layer as a fallback transit seed, not as a routed mesh peer.
"${COMPOSE[@]}" exec -T node-a nvpn set \
  --participant "$A_NPUB" \
  --participant "$B_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn set \
  --participant "$A_NPUB" \
  --participant "$B_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-c nvpn set \
  --participant "$C_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$A_NPUB" \
  --participant "$B_NPUB" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$B_NPUB=10.203.0.11:51820" \
  --fips-peer-endpoint "$C_NPUB=10.203.0.12:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$A_NPUB" \
  --participant "$B_NPUB" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$A_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$C_NPUB=10.203.0.12:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-c nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$C_NPUB" \
  --endpoint "10.203.0.12:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$A_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$B_NPUB=10.203.0.11:51820" >/dev/null

A_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
B_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"
C_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-c nvpn ip | tr -d '\r')"

connect_env="NVPN_FIPS_NOSTR_DISCOVERY_POLICY='$FIPS_NOSTR_DISCOVERY_POLICY'"
if is_true "$PIPELINE_TRACE"; then
  connect_env="$connect_env NVPN_PIPELINE_TRACE=1 NVPN_PIPELINE_INTERVAL_SECS='$PIPELINE_INTERVAL_SECS' FIPS_PERF_INTERVAL_SECS='$PIPELINE_INTERVAL_SECS'"
fi

"${COMPOSE[@]}" exec -d node-a sh -lc "$connect_env $EXTRA_CONNECT_ENV nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "$connect_env $EXTRA_CONNECT_ENV nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-c sh -lc "$connect_env $EXTRA_CONNECT_ENV nvpn connect > /tmp/connect.log 2>&1"
DIAGNOSTICS_READY=1

# Wait until the routed roster peers report mesh: 1/1 peers connected. C is a
# transit endpoint, not a private-mesh roster peer.
mesh_up=0
for _ in $(seq 1 45); do
  a="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  b="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  c="$("${COMPOSE[@]}" exec -T node-c sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  if grep -q "mesh: 1/1 peers connected" <<<"$a" \
    && grep -q "mesh: 1/1 peers connected" <<<"$b" \
    && [[ -n "$c" ]]; then
    mesh_up=1
    break
  fi
  sleep 1
done

dump_logs() {
  echo "--- node-a connect log (last 15) ---"
  "${COMPOSE[@]}" exec -T node-a sh -lc 'tail -15 /tmp/connect.log 2>/dev/null' || true
  echo "--- node-b connect log (last 15) ---"
  "${COMPOSE[@]}" exec -T node-b sh -lc 'tail -15 /tmp/connect.log 2>/dev/null' || true
  echo "--- node-c connect log (last 15) ---"
  "${COMPOSE[@]}" exec -T node-c sh -lc 'tail -15 /tmp/connect.log 2>/dev/null' || true
}

if [[ $mesh_up -ne 1 ]]; then
  echo "perf-relay: mesh did not converge to 1/1 within 45s; aborting" >&2
  dump_logs
  exit 1
fi

if ! "${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$B_TUNNEL_IP" >/dev/null 2>&1; then
  echo "perf-relay: ping a->b over mesh failed; transit not established" >&2
  dump_logs
  exit 1
fi

echo "alice tunnel ip: $A_TUNNEL_IP"
echo "bob   tunnel ip: $B_TUNNEL_IP"
echo "carol tunnel ip: $C_TUNNEL_IP   (transit relay)"
echo
PIPELINE_START_NODE_A="$(pipeline_line_count node-a)"
PIPELINE_START_NODE_B="$(pipeline_line_count node-b)"
PIPELINE_START_NODE_C="$(pipeline_line_count node-c)"
TRANSIT_A_TO_C_BEFORE="$(udp_output_counter_bytes node-a "$COUNTER_A_TO_C")"
TRANSIT_C_TO_B_BEFORE="$(udp_output_counter_bytes node-c "$COUNTER_C_TO_B")"
docker_bench_start_cpu_stress

"${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
sleep 1

run_test_json() {
  local phase="$1"
  local label="$2"
  local json_path="$3"
  shift 3
  local phase_start_node_a phase_start_node_b phase_start_node_c
  local phase_end_node_a phase_end_node_b phase_end_node_c
  local err_path is_udp arg
  printf '## %s\n' "$label"
  # --connect-timeout caps the initial 3WHS so a black-holed transit path
  # bails out in 3s instead of hanging tcp_synack_retries (~120s).
  err_path="$json_path.stderr"
  is_udp=0
  for arg in "$@"; do
    [[ "$arg" == "-u" ]] && is_udp=1
  done
  phase_start_node_a="$(pipeline_line_count node-a)"
  phase_start_node_b="$(pipeline_line_count node-b)"
  phase_start_node_c="$(pipeline_line_count node-c)"
  if ! "${COMPOSE[@]}" exec -T node-a \
    timeout --kill-after=5s "$IPERF_TIMEOUT_SECS" \
    iperf3 -c "$B_TUNNEL_IP" -t "$DURATION" -i "$IPERF_INTERVAL_SECS" -f m \
      --connect-timeout 3000 --json --get-server-output "$@" \
    >"$json_path" 2>"$err_path"; then
    cat "$err_path" >&2
    cat "$json_path" >&2
    return 1
  fi
  phase_end_node_a="$(pipeline_line_count node-a)"
  phase_end_node_b="$(pipeline_line_count node-b)"
  phase_end_node_c="$(pipeline_line_count node-c)"
  append_pipeline_phase_range \
    "$phase" \
    "$phase_start_node_a" \
    "$phase_end_node_a" \
    "$phase_start_node_b" \
    "$phase_end_node_b" \
    "$phase_start_node_c" \
    "$phase_end_node_c"
  rm -f "$err_path"
  printf '  receiver: %s Mbps' "$(docker_bench_iperf_mbps "$json_path")"
  if [[ "$is_udp" == "1" ]]; then
    printf ', loss: %s%%' "$(docker_bench_iperf_loss_pct "$json_path")"
  else
    printf ', retrans: %s' "$(docker_bench_iperf_retrans "$json_path")"
  fi
  printf '\n\n'
}

tcp_single_json="$RAW_DIR/nvpn-relay-tcp-single.json"
tcp_4_json="$RAW_DIR/nvpn-relay-tcp-4.json"
tcp_8_json="$RAW_DIR/nvpn-relay-tcp-8.json"
udp_200_json="$RAW_DIR/nvpn-relay-udp-200m.json"
udp_1000_json="$RAW_DIR/nvpn-relay-udp-1000m.json"
ping_output="$RAW_DIR/nvpn-relay-ping.txt"

run_test_json tcp-single "TCP single stream (A -> C -> B)" "$tcp_single_json"
run_test_json tcp-4 "TCP 4 streams" "$tcp_4_json" -P 4
run_test_json tcp-8 "TCP 8 streams" "$tcp_8_json" -P 8
run_test_json udp-200 "UDP 200 Mbit target" "$udp_200_json" -u -b 200M
run_test_json udp-1000 "UDP 1000 Mbit target" "$udp_1000_json" -u -b 1G

printf '## ping (300 packets, 10ms apart) over mesh transit (A -> C -> B)\n'
ping_start_node_a="$(pipeline_line_count node-a)"
ping_start_node_b="$(pipeline_line_count node-b)"
ping_start_node_c="$(pipeline_line_count node-c)"
"${COMPOSE[@]}" exec -T node-a ping -c 300 -i 0.01 -q "$B_TUNNEL_IP" >"$ping_output" 2>&1
ping_end_node_a="$(pipeline_line_count node-a)"
ping_end_node_b="$(pipeline_line_count node-b)"
ping_end_node_c="$(pipeline_line_count node-c)"
append_pipeline_phase_range \
  ping \
  "$ping_start_node_a" \
  "$ping_end_node_a" \
  "$ping_start_node_b" \
  "$ping_end_node_b" \
  "$ping_start_node_c" \
  "$ping_end_node_c"
tail -3 "$ping_output"

docker_bench_append_summary_row \
  nvpn-relay \
  "" \
  "$DURATION" \
  "$RAW_DIR" \
  "$tcp_single_json" \
  "$tcp_4_json" \
  "$tcp_8_json" \
  "$udp_200_json" \
  "$udp_1000_json" \
  "$ping_output"

assert_transit_counter_delta node-a "$COUNTER_A_TO_C" "$TRANSIT_A_TO_C_BEFORE" "$MIN_TRANSIT_BYTES" "node-a -> node-c"
a_to_c_delta="$LAST_TRANSIT_DELTA"
assert_transit_counter_delta node-c "$COUNTER_C_TO_B" "$TRANSIT_C_TO_B_BEFORE" "$MIN_TRANSIT_BYTES" "node-c -> node-b"
c_to_b_delta="$LAST_TRANSIT_DELTA"
write_transit_counter_summary "$a_to_c_delta" "$c_to_b_delta"
docker_bench_assert_summary_guards "$SUMMARY_TSV"
capture_nvpn_diagnostics 0
printf 'nvpn docker relay bench passed: wrote summary to %s\n' "$SUMMARY_TSV"
