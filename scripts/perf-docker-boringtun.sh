#!/usr/bin/env bash
# Throughput / latency benchmark over a 2-node WireGuard userspace tunnel
# inside docker, using Cloudflare's `boringtun-cli` as the userspace
# WireGuard implementation.
#
# Same network shape, same iperf3 / ping methodology as
# scripts/perf-docker.sh, so the output format lines up with the nvpn
# bench tables for an apples-to-apples comparison. Bench uses the
# default chacha20poly1305 wire crypto (NEON on aarch64, AVX on x86_64).
#
# Two passes by default:
#   - WG_THREADS=1 — boringtun in single-task mode, the architectural
#     peer to the current single-task nvpn run_rx_loop.
#   - WG_THREADS=4 — boringtun's CLI default, real-world deployment.
#
# Override: WG_THREADS_LIST="1 4 8" bash scripts/perf-docker-boringtun.sh
# Single pass: WG_THREADS=4 SINGLE_PASS=1 bash scripts/perf-docker-boringtun.sh
#
# Optional contention mode matches scripts/perf-docker.sh:
#   NVPN_DOCKER_CPU_STRESS=1
#   NVPN_DOCKER_CPU_STRESS_SIDES=local|remote|both
#   NVPN_DOCKER_CPU_STRESS_{LOCAL,REMOTE}_WORKERS=N
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"
PROJECT_NAME="${PROJECT_NAME:-nvpn-bench-boringtun}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.bench-boringtun.yml")

DURATION="${DURATION:-10}"
IPERF_INTERVAL_SECS="${NVPN_DOCKER_IPERF_INTERVAL_SECS:-0}"
IPERF_TIMEOUT_SECS="${NVPN_DOCKER_IPERF_TIMEOUT_SECS:-$((DURATION + 30))}"
NVPN_DOCKER_IPERF_TIMEOUT_SECS="$IPERF_TIMEOUT_SECS"
IPERF_SOCKET_BUFFER="${NVPN_DOCKER_IPERF_SOCKET_BUFFER:-}"
UDP1000_PARALLEL="${NVPN_DOCKER_UDP1000_PARALLEL:-}"
UDP1000_BANDWIDTH="${NVPN_DOCKER_UDP1000_BANDWIDTH:-1G}"
SKIP_BUILD="${NVPN_DOCKER_SKIP_BUILD:-0}"
BORINGTUN_REPO_PATH="${NVPN_BORINGTUN_REPO_PATH:-$ROOT_DIR/../boringtun}"
OUTPUT_DIR="${NVPN_BORINGTUN_DOCKER_OUTPUT_DIR:-$ROOT_DIR/artifacts/boringtun-docker/$(date -u +%Y%m%dT%H%M%SZ)}"
RAW_DIR="$OUTPUT_DIR/raw"
SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
ALICE_TUN="10.44.0.1"
BOB_TUN="10.44.0.2"
ALICE_BRIDGE="10.203.0.10"
BOB_BRIDGE="10.203.0.11"
WG_PORT="51820"

if [[ -n "$IPERF_SOCKET_BUFFER" && ! "$IPERF_SOCKET_BUFFER" =~ ^[0-9]+([KMG])?$ ]]; then
  echo "perf-boringtun: invalid NVPN_DOCKER_IPERF_SOCKET_BUFFER=$IPERF_SOCKET_BUFFER (expected bytes or K/M/G suffix)" >&2
  exit 2
fi
if [[ -n "$UDP1000_PARALLEL" && ! "$UDP1000_PARALLEL" =~ ^[1-9][0-9]*$ ]]; then
  echo "perf-boringtun: invalid NVPN_DOCKER_UDP1000_PARALLEL=$UDP1000_PARALLEL (expected positive integer)" >&2
  exit 2
fi
if [[ ! "$UDP1000_BANDWIDTH" =~ ^[0-9]+([KMG])?$ ]]; then
  echo "perf-boringtun: invalid NVPN_DOCKER_UDP1000_BANDWIDTH=$UDP1000_BANDWIDTH (expected bits/sec or K/M/G suffix)" >&2
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

if [[ -n "${SINGLE_PASS:-}" ]]; then
  THREADS_LIST=("${WG_THREADS:-4}")
else
  IFS=' ' read -ra THREADS_LIST <<<"${WG_THREADS_LIST:-1 4}"
fi

cleanup() {
  docker_bench_stop_cpu_stress
  if ! is_true "${KEEP:-0}"; then
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  fi
}

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

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
  echo "perf-boringtun: service '$service' did not start" >&2
  exit 1
}

reset_wg() {
  for service in node-a node-b; do
    "${COMPOSE[@]}" exec -T "$service" sh -c "
      pkill -9 boringtun-cli 2>/dev/null
      ip link del wg0 2>/dev/null
      true
    " >/dev/null
  done
}

setup_wg() {
  local threads="$1"

  ALICE_PRIV=$("${COMPOSE[@]}" exec -T node-a wg genkey | tr -d '\r\n')
  ALICE_PUB=$(echo -n "$ALICE_PRIV" | "${COMPOSE[@]}" exec -T node-a wg pubkey | tr -d '\r\n')
  BOB_PRIV=$("${COMPOSE[@]}" exec -T node-b wg genkey | tr -d '\r\n')
  BOB_PUB=$(echo -n "$BOB_PRIV" | "${COMPOSE[@]}" exec -T node-b wg pubkey | tr -d '\r\n')

  "${COMPOSE[@]}" exec -T node-a sh -c "
    set -e
    WG_THREADS=$threads boringtun-cli --disable-drop-privileges wg0 >/dev/null 2>&1
    ip addr add $ALICE_TUN/24 dev wg0
    ip link set wg0 mtu 1420
    ip link set wg0 up
    printf '%s' '$ALICE_PRIV' > /tmp/wg.priv
    wg set wg0 private-key /tmp/wg.priv listen-port $WG_PORT
    wg set wg0 peer '$BOB_PUB' allowed-ips $BOB_TUN/32 endpoint $BOB_BRIDGE:$WG_PORT persistent-keepalive 25
  " >/dev/null

  "${COMPOSE[@]}" exec -T node-b sh -c "
    set -e
    WG_THREADS=$threads boringtun-cli --disable-drop-privileges wg0 >/dev/null 2>&1
    ip addr add $BOB_TUN/24 dev wg0
    ip link set wg0 mtu 1420
    ip link set wg0 up
    printf '%s' '$BOB_PRIV' > /tmp/wg.priv
    wg set wg0 private-key /tmp/wg.priv listen-port $WG_PORT
    wg set wg0 peer '$ALICE_PUB' allowed-ips $ALICE_TUN/32 endpoint $ALICE_BRIDGE:$WG_PORT persistent-keepalive 25
  " >/dev/null

  # Wait for the tunnel to converge (handshake fires on first traffic).
  for _ in $(seq 1 30); do
    if "${COMPOSE[@]}" exec -T node-a ping -c 1 -W 1 "$BOB_TUN" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  echo "perf-boringtun: tunnel did not converge for threads=$threads" >&2
  exit 1
}

boringtun_cpu_sample() {
  docker_bench_process_cpu_sample "$1" boringtun-cli
}

write_boringtun_source_metadata() {
  local commit="" dirty=""
  if [[ -d "$BORINGTUN_REPO_PATH/.git" ]]; then
    commit="$(docker_bench_git_head "$BORINGTUN_REPO_PATH")"
    dirty="$(docker_bench_git_dirty "$BORINGTUN_REPO_PATH")"
  fi
  jq \
    --arg source "boringtun" \
    --arg commit "$commit" \
    --arg dirty "$dirty" \
    '.reference_source = {
      name: $source,
      git_short: (if $commit == "" then null else $commit end),
      dirty: (if $dirty == "" then null else ($dirty == "true") end)
    }' \
    "$OUTPUT_DIR/metadata.json" >"$OUTPUT_DIR/metadata.json.tmp"
  mv "$OUTPUT_DIR/metadata.json.tmp" "$OUTPUT_DIR/metadata.json"
}

run_test_json() {
  local phase="$1"
  local label="$2"
  local json_path="$3"
  local cpu_phases="$4"
  shift 4
  local cpu_start_node_a cpu_start_node_b cpu_end_node_a cpu_end_node_b transfer_bytes
  local is_udp=0
  [[ "${1:-}" == "-u" ]] && is_udp=1
  printf '## %s\n' "$label"
  local err_path="$json_path.stderr"
  local iperf_cmd=(
    timeout --kill-after=5s "$IPERF_TIMEOUT_SECS"
    iperf3 -c "$BOB_TUN" -t "$DURATION" -i "$IPERF_INTERVAL_SECS" -f m
    --connect-timeout 3000 --json
  )
  if (( is_udp )) && [[ -n "$IPERF_SOCKET_BUFFER" ]]; then
    iperf_cmd+=("${IPERF_SOCKET_BUFFER_ARGS[@]}")
  fi
  iperf_cmd+=("$@")
  cpu_start_node_a="$(boringtun_cpu_sample node-a)"
  cpu_start_node_b="$(boringtun_cpu_sample node-b)"
  if ! "${COMPOSE[@]}" exec -T node-a "${iperf_cmd[@]}" >"$json_path" 2>"$err_path"; then
    cat "$err_path" >&2
    cat "$json_path" >&2
    return 1
  fi
  cpu_end_node_a="$(boringtun_cpu_sample node-a)"
  cpu_end_node_b="$(boringtun_cpu_sample node-b)"
  if jq -e 'has("error")' "$json_path" >/dev/null; then
    cat "$err_path" >&2
    cat "$json_path" >&2
    return 1
  fi
  transfer_bytes="$(docker_bench_iperf_transfer_bytes "$json_path")"
  docker_bench_append_cpu_phase_rows \
    "$cpu_phases" \
    "$phase" \
    "$transfer_bytes" \
    "$cpu_start_node_a" \
    "$cpu_start_node_b" \
    "$cpu_end_node_a" \
    "$cpu_end_node_b"
  rm -f "$err_path"
  printf '  receiver: %s Mbps' "$(docker_bench_iperf_mbps "$json_path")"
  if (( is_udp )); then
    printf ', loss: %s%%' "$(docker_bench_iperf_loss_pct "$json_path")"
  else
    printf ', retrans: %s' "$(docker_bench_iperf_retrans "$json_path")"
  fi
  printf '\n\n'
}

write_iperf_socket_buffer_summary() {
  local threads="$1"
  local output_path="$RAW_DIR/boringtun-threads-$threads-iperf-socket-buffers.tsv"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    phase protocol streams requested_sock_bufsize actual_recv_buf actual_send_buf \
    >"$output_path"
  local phase json_path prefix
  prefix="$RAW_DIR/boringtun-threads-$threads"
  for phase in tcp-single tcp-4 tcp-8 udp-200 udp-1000; do
    json_path="$prefix-$phase"
    case "$phase" in
      udp-200) json_path="$prefix-udp-200m.json" ;;
      udp-1000) json_path="$prefix-udp-1000m.json" ;;
      *) json_path="$prefix-$phase.json" ;;
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

write_udp_receiver_limits() {
  local threads="$1"
  local service prefix
  for service in node-a node-b; do
    prefix="$RAW_DIR/boringtun-threads-$threads-$service"
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      'for key in net.core.rmem_default net.core.rmem_max net.core.wmem_default net.core.wmem_max net.ipv4.udp_mem net.ipv4.udp_rmem_min net.ipv4.udp_wmem_min; do printf "%s\t" "$key"; sysctl -n "$key" 2>/dev/null || printf "unavailable\n"; done' \
      >"$prefix-udp-receiver-limits.tsv" 2>/dev/null || true
  done
}

run_ping_summary() {
  local output_path="$1"
  local cpu_phases="$2"
  local cpu_start_node_a cpu_start_node_b cpu_end_node_a cpu_end_node_b
  printf '## ping (300 packets, 10ms apart) over wg0\n'
  cpu_start_node_a="$(boringtun_cpu_sample node-a)"
  cpu_start_node_b="$(boringtun_cpu_sample node-b)"
  "${COMPOSE[@]}" exec -T node-a ping -c 300 -i 0.01 -q "$BOB_TUN" >"$output_path" 2>&1
  cpu_end_node_a="$(boringtun_cpu_sample node-a)"
  cpu_end_node_b="$(boringtun_cpu_sample node-b)"
  docker_bench_append_cpu_phase_rows \
    "$cpu_phases" \
    ping \
    "" \
    "$cpu_start_node_a" \
    "$cpu_start_node_b" \
    "$cpu_end_node_a" \
    "$cpu_end_node_b"
  tail -3 "$output_path"
  echo
}

run_boringtun_pass() {
  local threads="$1"
  local prefix="$RAW_DIR/boringtun-threads-$threads"
  local tcp_single_json="$prefix-tcp-single.json"
  local tcp_4_json="$prefix-tcp-4.json"
  local tcp_8_json="$prefix-tcp-8.json"
  local udp_200_json="$prefix-udp-200m.json"
  local udp_1000_json="$prefix-udp-1000m.json"
  local ping_output="$prefix-ping.txt"
  local cpu_phases="$prefix-cpu-phases.tsv"

  reset_wg
  setup_wg "$threads"
  docker_bench_write_cpu_phase_header "$cpu_phases"

  printf '\n=========================================\n'
  printf '  boringtun WG_THREADS=%s\n' "$threads"
  printf '=========================================\n'
  printf 'alice tunnel ip: %s\n' "$ALICE_TUN"
  printf 'bob   tunnel ip: %s\n\n' "$BOB_TUN"

  # Restart the iperf3 server fresh per pass so socket state from a
  # prior pass can't leak into the next.
  "${COMPOSE[@]}" exec -T node-b sh -c "pkill -9 iperf3 2>/dev/null; true" >/dev/null
  "${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
  sleep 1

  run_test_json tcp-single "TCP single stream" "$tcp_single_json" "$cpu_phases"
  run_test_json tcp-4 "TCP 4 streams" "$tcp_4_json" "$cpu_phases" -P 4
  run_test_json tcp-8 "TCP 8 streams" "$tcp_8_json" "$cpu_phases" -P 8
  run_test_json udp-200 "UDP 200 Mbit target" "$udp_200_json" "$cpu_phases" -u -b 200M
  if [[ ${#UDP1000_PARALLEL_ARGS[@]} -gt 0 ]]; then
    run_test_json udp-1000 "UDP 1000 Mbit target" "$udp_1000_json" "$cpu_phases" -u -b "$UDP1000_PER_STREAM_BANDWIDTH" "${UDP1000_PARALLEL_ARGS[@]}"
  else
    run_test_json udp-1000 "UDP 1000 Mbit target" "$udp_1000_json" "$cpu_phases" -u -b "$UDP1000_BANDWIDTH"
  fi
  write_iperf_socket_buffer_summary "$threads"
  write_udp_receiver_limits "$threads"
  run_ping_summary "$ping_output" "$cpu_phases"
  docker_bench_append_summary_row \
    boringtun \
    "$threads" \
    "$DURATION" \
    "$RAW_DIR" \
    "$tcp_single_json" \
    "$tcp_4_json" \
    "$tcp_8_json" \
    "$udp_200_json" \
    "$udp_1000_json" \
    "$ping_output"
}

main() {
  trap cleanup EXIT
  cleanup
  docker_bench_init_summary
  docker_bench_write_metadata boringtun "$DURATION"
  start_compose_services
  for service in node-a node-b; do
    wait_for_service "$service"
  done
  docker_bench_start_cpu_stress

  local threads
  write_boringtun_source_metadata
  for threads in "${THREADS_LIST[@]}"; do
    run_boringtun_pass "$threads"
  done
  printf 'boringtun docker bench passed: wrote summary to %s\n' "$SUMMARY_TSV"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
