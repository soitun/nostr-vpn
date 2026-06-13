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
SKIP_BUILD="${NVPN_DOCKER_SKIP_BUILD:-0}"
OUTPUT_DIR="${NVPN_BORINGTUN_DOCKER_OUTPUT_DIR:-$ROOT_DIR/artifacts/boringtun-docker/$(date -u +%Y%m%dT%H%M%SZ)}"
RAW_DIR="$OUTPUT_DIR/raw"
SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
ALICE_TUN="10.44.0.1"
BOB_TUN="10.44.0.2"
ALICE_BRIDGE="10.203.0.10"
BOB_BRIDGE="10.203.0.11"
WG_PORT="51820"

if [[ -n "${SINGLE_PASS:-}" ]]; then
  THREADS_LIST=("${WG_THREADS:-4}")
else
  IFS=' ' read -ra THREADS_LIST <<<"${WG_THREADS_LIST:-1 4}"
fi

cleanup() {
  docker_bench_stop_cpu_stress
  if [[ -z "${KEEP:-}" ]]; then
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

run_test_json() {
  local label="$1"
  local json_path="$2"
  shift 2
  printf '## %s\n' "$label"
  local err_path="$json_path.stderr"
  local iperf_cmd=(
    timeout --kill-after=5s "$IPERF_TIMEOUT_SECS"
    iperf3 -c "$BOB_TUN" -t "$DURATION" -i "$IPERF_INTERVAL_SECS" -f m
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

run_ping_summary() {
  local output_path="$1"
  printf '## ping (300 packets, 10ms apart) over wg0\n'
  "${COMPOSE[@]}" exec -T node-a ping -c 300 -i 0.01 -q "$BOB_TUN" >"$output_path" 2>&1
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

  reset_wg
  setup_wg "$threads"

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

  run_test_json "TCP single stream" "$tcp_single_json"
  run_test_json "TCP 4 streams" "$tcp_4_json" -P 4
  run_test_json "TCP 8 streams" "$tcp_8_json" -P 8
  run_test_json "UDP 200 Mbit target" "$udp_200_json" -u -b 200M
  run_test_json "UDP 1000 Mbit target" "$udp_1000_json" -u -b 1G
  run_ping_summary "$ping_output"
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
  for threads in "${THREADS_LIST[@]}"; do
    run_boringtun_pass "$threads"
  done
  printf 'boringtun docker bench passed: wrote summary to %s\n' "$SUMMARY_TSV"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
