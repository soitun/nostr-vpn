#!/usr/bin/env bash
# Throughput / latency benchmark over a 2-node FIPS overlay mesh inside docker.
#
# Spins up node-a + node-b on a private bridge subnet (10.203.0.0/24) with
# static peer endpoints, brings the mesh up, then runs iperf3 in both
# directions over the mesh tunnel addresses. Tears down on exit.
#
# Optional contention mode:
#   NVPN_DOCKER_CPU_STRESS=1
#   NVPN_DOCKER_CPU_STRESS_SIDES=local|remote|both
#   NVPN_DOCKER_CPU_STRESS_{LOCAL,REMOTE}_WORKERS=N
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"
PROJECT_NAME="${PROJECT_NAME:-nvpn-perf}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-perf"
DURATION="${DURATION:-10}"
IPERF_INTERVAL_SECS="${NVPN_DOCKER_IPERF_INTERVAL_SECS:-0}"
IPERF_TIMEOUT_SECS="${NVPN_DOCKER_IPERF_TIMEOUT_SECS:-$((DURATION + 30))}"
NVPN_DOCKER_IPERF_TIMEOUT_SECS="$IPERF_TIMEOUT_SECS"
SKIP_BUILD="${NVPN_DOCKER_SKIP_BUILD:-0}"
OUTPUT_DIR="${NVPN_DOCKER_OUTPUT_DIR:-$ROOT_DIR/artifacts/nvpn-docker/$(date -u +%Y%m%dT%H%M%SZ)}"
RAW_DIR="$OUTPUT_DIR/raw"
SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
PIPELINE_TRACE="${NVPN_DOCKER_PIPELINE_TRACE:-1}"
PIPELINE_INTERVAL_SECS="${NVPN_DOCKER_PIPELINE_INTERVAL_SECS:-5}"
EXTRA_CONNECT_ENV="${NVPN_DOCKER_EXTRA_ENV:-}"
REQUIRE_NO_DIRECT_FMP="${NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP:-0}"
NVPN_DOCKER_PIPELINE_TRACE="$PIPELINE_TRACE"
NVPN_DOCKER_PIPELINE_INTERVAL_SECS="$PIPELINE_INTERVAL_SECS"
DIAGNOSTICS_READY=0
DIAGNOSTICS_CAPTURED=0
PIPELINE_START_NODE_A=0
PIPELINE_START_NODE_B=0

cleanup() {
  local status=$?
  if declare -F capture_nvpn_diagnostics >/dev/null; then
    capture_nvpn_diagnostics "$status" || true
  fi
  docker_bench_stop_cpu_stress
  if [[ -z "${KEEP:-}" ]]; then
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

if is_true "$REQUIRE_NO_DIRECT_FMP" && docker_bench_direct_fmp_forced_enabled; then
  echo "perf: NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP=1 rejects FIPS_DIRECT_ENDPOINT_FMP_ONLY in NVPN_DOCKER_EXTRA_ENV" >&2
  exit 2
fi

start_compose_services() {
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

pipeline_line_count() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "if [ -r /tmp/connect.log ]; then grep -Ec '^\\[(pipe|nvpn-pipe) ' /tmp/connect.log || true; else printf '0\\n'; fi" \
    | tr -d '\r' \
    | awk 'NR == 1 { print $1 + 0; found = 1 } END { if (!found) print 0 }'
}

write_pipeline_summary_header() {
  local path="$1"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    service \
    pipeline_line_count \
    benchmark_pipeline_line_count \
    load_top_queue_wait \
    peak_top_queue_wait \
    fmp_worker_batch \
    decrypt_worker_batch \
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
  local prefix="$RAW_DIR/nvpn-$service"
  local all_lines_path="$prefix-pipeline-lines.txt"
  local bench_lines_path="$prefix-pipeline-benchmark-lines.txt"
  local load_path="$prefix-pipeline-load-selected.txt"
  local peak_path="$prefix-pipeline-peak-wait-selected.txt"
  local all_count bench_count load_line peak_line
  local load_top peak_top fmp_batch decrypt_batch udp_send_batch
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
  decrypt_batch="$(docker_bench_pipeline_decrypt_worker_batch_summary "$load_line")"
  udp_send_batch="$(docker_bench_pipeline_udp_send_batch_summary "$load_line")"
  nvpn_tun_read_batch="$(docker_bench_pipeline_nvpn_tun_read_batch_summary "$load_line")"
  nvpn_mesh_send_batch="$(docker_bench_pipeline_nvpn_mesh_send_batch_summary "$load_line")"
  nvpn_mesh_recv_batch="$(docker_bench_pipeline_nvpn_mesh_recv_batch_summary "$load_line")"
  nvpn_tun_write="$(docker_bench_pipeline_nvpn_tun_write_summary_from_stdin <"$bench_lines_path")"
  hard_events="$(docker_bench_pipeline_hard_event_summary_from_stdin "$start_line" <"$all_lines_path")"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$service" \
    "$all_count" \
    "$bench_count" \
    "$(docker_bench_tsv_field "$load_top")" \
    "$(docker_bench_tsv_field "$peak_top")" \
    "$(docker_bench_tsv_field "$fmp_batch")" \
    "$(docker_bench_tsv_field "$decrypt_batch")" \
    "$(docker_bench_tsv_field "$udp_send_batch")" \
    "$(docker_bench_tsv_field "$nvpn_tun_read_batch")" \
    "$(docker_bench_tsv_field "$nvpn_mesh_send_batch")" \
    "$(docker_bench_tsv_field "$nvpn_mesh_recv_batch")" \
    "$(docker_bench_tsv_field "$nvpn_tun_write")" \
    "$(docker_bench_tsv_field "$hard_events")" \
    "$(docker_bench_tsv_field "$load_line")" >>"$summary_path"
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
  write_pipeline_summary_header "$pipeline_summary"

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
    case "$service" in
      node-a) capture_pipeline_for_service "$service" "$PIPELINE_START_NODE_A" "$log_path" "$pipeline_summary" ;;
      node-b) capture_pipeline_for_service "$service" "$PIPELINE_START_NODE_B" "$log_path" "$pipeline_summary" ;;
    esac
  done
}

cleanup
docker_bench_init_summary
docker_bench_write_metadata nvpn "$DURATION"
start_compose_services
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"
if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "perf: unable to resolve node npubs from config" >&2
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

connect_env=""
if is_true "$PIPELINE_TRACE"; then
  connect_env="NVPN_PIPELINE_TRACE=1 NVPN_PIPELINE_INTERVAL_SECS='$PIPELINE_INTERVAL_SECS' FIPS_PERF_INTERVAL_SECS='$PIPELINE_INTERVAL_SECS'"
fi

"${COMPOSE[@]}" exec -d node-a sh -lc "$connect_env $EXTRA_CONNECT_ENV nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "$connect_env $EXTRA_CONNECT_ENV nvpn connect > /tmp/connect.log 2>&1"
DIAGNOSTICS_READY=1

for _ in $(seq 1 30); do
  a="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  b="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  if grep -q "mesh: 1/1 peers connected" <<<"$a" \
    && grep -q "mesh: 1/1 peers connected" <<<"$b"; then
    break
  fi
  sleep 1
done

if ! "${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$BOB_TUNNEL_IP" >/dev/null; then
  echo "perf: ping a->b over mesh failed" >&2
  exit 1
fi

echo "alice tunnel ip: $ALICE_TUNNEL_IP"
echo "bob   tunnel ip: $BOB_TUNNEL_IP"
echo
PIPELINE_START_NODE_A="$(pipeline_line_count node-a)"
PIPELINE_START_NODE_B="$(pipeline_line_count node-b)"
docker_bench_start_cpu_stress

"${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
sleep 1

run_test_json() {
  local label="$1"
  local json_path="$2"
  shift 2
  printf '## %s\n' "$label"
  # --connect-timeout caps the 3WHS so a broken path bails out fast
  # instead of hanging on tcp_synack_retries.
  local err_path="$json_path.stderr"
  local iperf_cmd=(
    timeout --kill-after=5s "$IPERF_TIMEOUT_SECS"
    iperf3 -c "$BOB_TUNNEL_IP" -t "$DURATION" -i "$IPERF_INTERVAL_SECS" -f m
    --connect-timeout 3000 --json "$@"
  )
  if ! "${COMPOSE[@]}" exec -T node-a "${iperf_cmd[@]}" >"$json_path" 2>"$err_path"; then
    cat "$err_path" >&2
    cat "$json_path" >&2
    return 1
  fi
  rm -f "$err_path"
  printf '  receiver: %s Mbps' "$(docker_bench_iperf_mbps "$json_path")"
  if [[ "${1:-}" == "-u" ]]; then
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
ping_output="$RAW_DIR/nvpn-ping.txt"

run_test_json "TCP single stream" "$tcp_single_json"
run_test_json "TCP 4 streams" "$tcp_4_json" -P 4
run_test_json "TCP 8 streams" "$tcp_8_json" -P 8
run_test_json "UDP 200 Mbit target" "$udp_200_json" -u -b 200M
run_test_json "UDP 1000 Mbit target" "$udp_1000_json" -u -b 1G

printf '## ping (300 packets, 10ms apart) over mesh\n'
"${COMPOSE[@]}" exec -T node-a ping -c 300 -i 0.01 -q "$BOB_TUNNEL_IP" >"$ping_output" 2>&1
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
  "$ping_output"

docker_bench_assert_summary_guards "$SUMMARY_TSV"
capture_nvpn_diagnostics 0
printf 'nvpn docker bench passed: wrote summary to %s\n' "$SUMMARY_TSV"
