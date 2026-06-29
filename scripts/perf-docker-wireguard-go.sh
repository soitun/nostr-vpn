#!/usr/bin/env bash
# Throughput / latency benchmark over a 2-node WireGuard userspace tunnel
# inside docker, using upstream wireguard-go built from a local checkout.
#
# Same network shape, same iperf3 / ping methodology as
# scripts/perf-docker.sh and scripts/perf-docker-boringtun.sh, so the output
# can be compared with scripts/compare-docker-benchmarks.sh.
#
# Optional contention mode matches the other Docker perf scripts:
#   NVPN_DOCKER_CPU_STRESS=1
#   NVPN_DOCKER_CPU_STRESS_SIDES=local|remote|both
#   NVPN_DOCKER_CPU_STRESS_{LOCAL,REMOTE}_WORKERS=N
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"
PROJECT_NAME="${PROJECT_NAME:-nvpn-bench-wireguard-go}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.bench-wireguard-go.yml")

DURATION="${DURATION:-10}"
IPERF_INTERVAL_SECS="${NVPN_DOCKER_IPERF_INTERVAL_SECS:-0}"
IPERF_TIMEOUT_SECS="${NVPN_DOCKER_IPERF_TIMEOUT_SECS:-$((DURATION + 30))}"
NVPN_DOCKER_IPERF_TIMEOUT_SECS="$IPERF_TIMEOUT_SECS"
IPERF_SOCKET_BUFFER="${NVPN_DOCKER_IPERF_SOCKET_BUFFER:-}"
UDP1000_PARALLEL="${NVPN_DOCKER_UDP1000_PARALLEL:-}"
UDP1000_BANDWIDTH="${NVPN_DOCKER_UDP1000_BANDWIDTH:-1G}"
SKIP_BUILD="${NVPN_DOCKER_SKIP_BUILD:-0}"
WIREGUARD_GO_REPO_PATH="${NVPN_WIREGUARD_GO_REPO_PATH:-$ROOT_DIR/../wireguard-go}"
OUTPUT_DIR="${NVPN_WIREGUARD_GO_DOCKER_OUTPUT_DIR:-$ROOT_DIR/artifacts/wireguard-go-docker/$(date -u +%Y%m%dT%H%M%SZ)}"
RAW_DIR="$OUTPUT_DIR/raw"
SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
WIREGUARD_GO_CPU_PHASES="$RAW_DIR/wireguard-go-cpu-phases.tsv"
ALICE_TUN="10.44.0.1"
BOB_TUN="10.44.0.2"
ALICE_BRIDGE="10.203.0.10"
BOB_BRIDGE="10.203.0.11"
WG_PORT="51820"

if [[ -n "$IPERF_SOCKET_BUFFER" && ! "$IPERF_SOCKET_BUFFER" =~ ^[0-9]+([KMG])?$ ]]; then
  echo "perf-wireguard-go: invalid NVPN_DOCKER_IPERF_SOCKET_BUFFER=$IPERF_SOCKET_BUFFER (expected bytes or K/M/G suffix)" >&2
  exit 2
fi
if [[ -n "$UDP1000_PARALLEL" && ! "$UDP1000_PARALLEL" =~ ^[1-9][0-9]*$ ]]; then
  echo "perf-wireguard-go: invalid NVPN_DOCKER_UDP1000_PARALLEL=$UDP1000_PARALLEL (expected positive integer)" >&2
  exit 2
fi
if [[ ! "$UDP1000_BANDWIDTH" =~ ^[0-9]+([KMG])?$ ]]; then
  echo "perf-wireguard-go: invalid NVPN_DOCKER_UDP1000_BANDWIDTH=$UDP1000_BANDWIDTH (expected bits/sec or K/M/G suffix)" >&2
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
  docker_bench_stop_cpu_stress
  if [[ -z "${KEEP:-}" ]]; then
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  fi
}

configure_iperf_socket_buffer_limits() {
  [[ -n "$IPERF_SOCKET_BUFFER" ]] || return 0
  local bytes service sysctl_log actual_rmem actual_wmem
  bytes="$(docker_bench_size_to_bytes "$IPERF_SOCKET_BUFFER")" || {
    echo "perf-wireguard-go: invalid NVPN_DOCKER_IPERF_SOCKET_BUFFER=$IPERF_SOCKET_BUFFER (expected bytes or K/M/G suffix)" >&2
    exit 2
  }
  for service in node-a node-b; do
    sysctl_log="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      "sysctl -w net.core.rmem_max=$bytes net.core.wmem_max=$bytes" 2>&1 || true)"
    actual_rmem="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      'sysctl -n net.core.rmem_max' 2>/dev/null || true)"
    actual_wmem="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      'sysctl -n net.core.wmem_max' 2>/dev/null || true)"
    if [[ ! "$actual_rmem" =~ ^[0-9]+$ ]] \
      || [[ ! "$actual_wmem" =~ ^[0-9]+$ ]] \
      || (( actual_rmem < bytes || actual_wmem < bytes )); then
      echo "perf-wireguard-go: failed to raise UDP socket buffer sysctls in $service for NVPN_DOCKER_IPERF_SOCKET_BUFFER=$IPERF_SOCKET_BUFFER (wanted >=$bytes, got rmem_max=${actual_rmem:-unknown}, wmem_max=${actual_wmem:-unknown})" >&2
      if [[ -n "$sysctl_log" ]]; then
        printf '%s\n' "$sysctl_log" >&2
      fi
      exit 1
    fi
  done
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
  local cid
  for _ in $(seq 1 30); do
    cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$cid" ]] && [[ "$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "perf-wireguard-go: service '$service' did not start" >&2
  exit 1
}

reset_wg() {
  for service in node-a node-b; do
    "${COMPOSE[@]}" exec -T "$service" sh -c "
      pkill -9 wireguard-go 2>/dev/null
      ip link del wg0 2>/dev/null
      true
    " >/dev/null
  done
}

start_wireguard_go() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -c "
    set -e
    rm -f /tmp/wireguard-go.log /tmp/wireguard-go.pid /tmp/wg-tun-name
    nohup env WG_PROCESS_FOREGROUND=1 WG_I_PREFER_BUGGY_USERSPACE_TO_POLISHED_KMOD=1 WG_TUN_NAME_FILE=/tmp/wg-tun-name \
      wireguard-go --foreground wg0 >/tmp/wireguard-go.log 2>&1 &
    echo \$! >/tmp/wireguard-go.pid
  " >/dev/null

  for _ in $(seq 1 30); do
    if "${COMPOSE[@]}" exec -T "$service" ip link show dev wg0 >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  echo "perf-wireguard-go: $service did not create wg0" >&2
  "${COMPOSE[@]}" exec -T "$service" sh -c 'cat /tmp/wireguard-go.log 2>/dev/null || true' >&2 || true
  exit 1
}

setup_wg() {
  ALICE_PRIV=$("${COMPOSE[@]}" exec -T node-a wg genkey | tr -d '\r\n')
  ALICE_PUB=$(echo -n "$ALICE_PRIV" | "${COMPOSE[@]}" exec -T node-a wg pubkey | tr -d '\r\n')
  BOB_PRIV=$("${COMPOSE[@]}" exec -T node-b wg genkey | tr -d '\r\n')
  BOB_PUB=$(echo -n "$BOB_PRIV" | "${COMPOSE[@]}" exec -T node-b wg pubkey | tr -d '\r\n')

  start_wireguard_go node-a
  start_wireguard_go node-b

  "${COMPOSE[@]}" exec -T node-a sh -c "
    set -e
    ip addr add $ALICE_TUN/24 dev wg0
    ip link set wg0 mtu 1420
    ip link set wg0 up
    printf '%s' '$ALICE_PRIV' > /tmp/wg.priv
    wg set wg0 private-key /tmp/wg.priv listen-port $WG_PORT
    wg set wg0 peer '$BOB_PUB' allowed-ips $BOB_TUN/32 endpoint $BOB_BRIDGE:$WG_PORT persistent-keepalive 25
  " >/dev/null

  "${COMPOSE[@]}" exec -T node-b sh -c "
    set -e
    ip addr add $BOB_TUN/24 dev wg0
    ip link set wg0 mtu 1420
    ip link set wg0 up
    printf '%s' '$BOB_PRIV' > /tmp/wg.priv
    wg set wg0 private-key /tmp/wg.priv listen-port $WG_PORT
    wg set wg0 peer '$ALICE_PUB' allowed-ips $ALICE_TUN/32 endpoint $ALICE_BRIDGE:$WG_PORT persistent-keepalive 25
  " >/dev/null

  for _ in $(seq 1 30); do
    if "${COMPOSE[@]}" exec -T node-a ping -c 1 -W 1 "$BOB_TUN" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  echo "perf-wireguard-go: tunnel did not converge" >&2
  exit 1
}

write_wireguard_go_source_metadata() {
  local commit=""
  if [[ -d "$WIREGUARD_GO_REPO_PATH/.git" ]]; then
    commit="$(git -C "$WIREGUARD_GO_REPO_PATH" rev-parse --short HEAD 2>/dev/null || true)"
  fi
  jq \
    --arg source "wireguard-go" \
    --arg commit "$commit" \
    '.reference_source = {
      name: $source,
      git_short: (if $commit == "" then null else $commit end)
    }' \
    "$OUTPUT_DIR/metadata.json" >"$OUTPUT_DIR/metadata.json.tmp"
  mv "$OUTPUT_DIR/metadata.json.tmp" "$OUTPUT_DIR/metadata.json"
}

collect_backend_artifacts() {
  local service
  for service in node-a node-b; do
    "${COMPOSE[@]}" exec -T "$service" sh -c 'cat /tmp/wireguard-go.log 2>/dev/null || true' \
      >"$RAW_DIR/wireguard-go-$service.log" || true
    "${COMPOSE[@]}" exec -T "$service" sh -c 'wireguard-go --version 2>/dev/null || true' \
      >"$RAW_DIR/wireguard-go-$service-version.txt" || true
    "${COMPOSE[@]}" exec -T "$service" sh -c 'wg show wg0 2>/dev/null || true' \
      >"$RAW_DIR/wireguard-go-$service-wg-show.txt" || true
  done
}

write_wireguard_go_cpu_phase_header() {
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    phase service pid_start pid_end \
    cpu_jiffies_start cpu_jiffies_end clk_tck \
    cpu_seconds transfer_bytes cpu_seconds_per_gbit cpu_seconds_per_gbyte \
    >"$WIREGUARD_GO_CPU_PHASES"
}

wireguard_go_cpu_sample() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc '
    pid="$(pgrep -x wireguard-go 2>/dev/null | head -n 1 || true)"
    clk="$(getconf CLK_TCK 2>/dev/null || printf 100)"
    if [ -n "$pid" ] && [ -r "/proc/$pid/stat" ]; then
      jiffies="$(awk "{ print \$14 + \$15 }" "/proc/$pid/stat" 2>/dev/null || true)"
      printf "%s\t%s\t%s\n" "${pid:-na}" "${jiffies:-na}" "${clk:-100}"
    else
      printf "na\tna\t%s\n" "${clk:-100}"
    fi
  ' 2>/dev/null | tr -d '\r' || printf 'na\tna\t100\n'
}

wireguard_go_cpu_sample_cpu_seconds() {
  local start_sample="$1"
  local end_sample="$2"
  local start_pid start_jiffies start_clk end_pid end_jiffies end_clk clk_tck
  IFS=$'\t' read -r start_pid start_jiffies start_clk <<<"$start_sample"
  IFS=$'\t' read -r end_pid end_jiffies end_clk <<<"$end_sample"
  if [[ "$start_pid" != "$end_pid" || ! "$start_pid" =~ ^[0-9]+$ ]]; then
    return 0
  fi
  clk_tck="$end_clk"
  [[ "$clk_tck" =~ ^[1-9][0-9]*$ ]] || clk_tck="$start_clk"
  docker_bench_cpu_seconds_from_jiffies "$start_jiffies" "$end_jiffies" "$clk_tck"
}

append_wireguard_go_cpu_phase_service_row() {
  local phase="$1"
  local service="$2"
  local transfer_bytes="$3"
  local start_sample="$4"
  local end_sample="$5"
  local start_pid start_jiffies start_clk end_pid end_jiffies end_clk clk_tck
  local cpu_seconds cpu_per_gbit cpu_per_gbyte
  IFS=$'\t' read -r start_pid start_jiffies start_clk <<<"$start_sample"
  IFS=$'\t' read -r end_pid end_jiffies end_clk <<<"$end_sample"
  clk_tck="$end_clk"
  [[ "$clk_tck" =~ ^[1-9][0-9]*$ ]] || clk_tck="$start_clk"
  if [[ "$start_pid" == "$end_pid" && "$start_pid" =~ ^[0-9]+$ ]]; then
    cpu_seconds="$(docker_bench_cpu_seconds_from_jiffies "$start_jiffies" "$end_jiffies" "$clk_tck")"
  else
    cpu_seconds=""
  fi
  cpu_per_gbit="$(docker_bench_cpu_seconds_per_gbit "$cpu_seconds" "$transfer_bytes")"
  cpu_per_gbyte="$(docker_bench_cpu_seconds_per_gbyte "$cpu_seconds" "$transfer_bytes")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$phase" \
    "$service" \
    "$(docker_bench_tsv_field "${start_pid:-na}")" \
    "$(docker_bench_tsv_field "${end_pid:-na}")" \
    "$(docker_bench_tsv_field "${start_jiffies:-na}")" \
    "$(docker_bench_tsv_field "${end_jiffies:-na}")" \
    "$(docker_bench_tsv_field "${clk_tck:-na}")" \
    "$cpu_seconds" \
    "$(docker_bench_tsv_field "$transfer_bytes")" \
    "$cpu_per_gbit" \
    "$cpu_per_gbyte" >>"$WIREGUARD_GO_CPU_PHASES"
}

append_wireguard_go_cpu_phase_rows() {
  local phase="$1"
  local transfer_bytes="$2"
  local start_a="$3"
  local start_b="$4"
  local end_a="$5"
  local end_b="$6"
  local cpu_a cpu_b cpu_both cpu_per_gbit cpu_per_gbyte
  append_wireguard_go_cpu_phase_service_row "$phase" node-a "$transfer_bytes" "$start_a" "$end_a"
  append_wireguard_go_cpu_phase_service_row "$phase" node-b "$transfer_bytes" "$start_b" "$end_b"
  cpu_a="$(wireguard_go_cpu_sample_cpu_seconds "$start_a" "$end_a")"
  cpu_b="$(wireguard_go_cpu_sample_cpu_seconds "$start_b" "$end_b")"
  if docker_bench_is_number "$cpu_a" && docker_bench_is_number "$cpu_b"; then
    cpu_both="$(awk -v a="$cpu_a" -v b="$cpu_b" 'BEGIN { printf "%.6f", a + b }')"
  else
    cpu_both=""
  fi
  cpu_per_gbit="$(docker_bench_cpu_seconds_per_gbit "$cpu_both" "$transfer_bytes")"
  cpu_per_gbyte="$(docker_bench_cpu_seconds_per_gbyte "$cpu_both" "$transfer_bytes")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$phase" both na na na na na \
    "$cpu_both" \
    "$(docker_bench_tsv_field "$transfer_bytes")" \
    "$cpu_per_gbit" \
    "$cpu_per_gbyte" >>"$WIREGUARD_GO_CPU_PHASES"
}

run_test_json() {
  local phase="$1"
  local label="$2"
  local json_path="$3"
  shift 3
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
  cpu_start_node_a="$(wireguard_go_cpu_sample node-a)"
  cpu_start_node_b="$(wireguard_go_cpu_sample node-b)"
  if ! "${COMPOSE[@]}" exec -T node-a "${iperf_cmd[@]}" >"$json_path" 2>"$err_path"; then
    cat "$err_path" >&2
    cat "$json_path" >&2
    return 1
  fi
  cpu_end_node_a="$(wireguard_go_cpu_sample node-a)"
  cpu_end_node_b="$(wireguard_go_cpu_sample node-b)"
  if jq -e 'has("error")' "$json_path" >/dev/null; then
    cat "$err_path" >&2
    cat "$json_path" >&2
    return 1
  fi
  transfer_bytes="$(docker_bench_iperf_transfer_bytes "$json_path")"
  append_wireguard_go_cpu_phase_rows \
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
  local output_path="$RAW_DIR/wireguard-go-iperf-socket-buffers.tsv"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    phase protocol streams requested_sock_bufsize actual_recv_buf actual_send_buf \
    >"$output_path"
  local phase json_path
  for phase in tcp-single tcp-4 tcp-8 udp-200 udp-1000; do
    case "$phase" in
      tcp-single) json_path="$RAW_DIR/wireguard-go-tcp-single.json" ;;
      tcp-4) json_path="$RAW_DIR/wireguard-go-tcp-4.json" ;;
      tcp-8) json_path="$RAW_DIR/wireguard-go-tcp-8.json" ;;
      udp-200) json_path="$RAW_DIR/wireguard-go-udp-200m.json" ;;
      udp-1000) json_path="$RAW_DIR/wireguard-go-udp-1000m.json" ;;
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
  local service
  for service in node-a node-b; do
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      'for key in net.core.rmem_default net.core.rmem_max net.core.wmem_default net.core.wmem_max net.ipv4.udp_mem net.ipv4.udp_rmem_min net.ipv4.udp_wmem_min; do printf "%s\t" "$key"; sysctl -n "$key" 2>/dev/null || printf "unavailable\n"; done' \
      >"$RAW_DIR/wireguard-go-$service-udp-receiver-limits.tsv" 2>/dev/null || true
  done
}

run_ping_summary() {
  local output_path="$1"
  local cpu_start_node_a cpu_start_node_b cpu_end_node_a cpu_end_node_b
  printf '## ping (300 packets, 10ms apart) over wg0\n'
  cpu_start_node_a="$(wireguard_go_cpu_sample node-a)"
  cpu_start_node_b="$(wireguard_go_cpu_sample node-b)"
  "${COMPOSE[@]}" exec -T node-a ping -c 300 -i 0.01 -q "$BOB_TUN" >"$output_path" 2>&1
  cpu_end_node_a="$(wireguard_go_cpu_sample node-a)"
  cpu_end_node_b="$(wireguard_go_cpu_sample node-b)"
  append_wireguard_go_cpu_phase_rows \
    ping \
    "" \
    "$cpu_start_node_a" \
    "$cpu_start_node_b" \
    "$cpu_end_node_a" \
    "$cpu_end_node_b"
  tail -3 "$output_path"
  echo
}

run_wireguard_go_pass() {
  local prefix="$RAW_DIR/wireguard-go"
  local tcp_single_json="$prefix-tcp-single.json"
  local tcp_4_json="$prefix-tcp-4.json"
  local tcp_8_json="$prefix-tcp-8.json"
  local udp_200_json="$prefix-udp-200m.json"
  local udp_1000_json="$prefix-udp-1000m.json"
  local ping_output="$prefix-ping.txt"

  reset_wg
  setup_wg

  printf '\n=========================================\n'
  printf '  wireguard-go\n'
  printf '=========================================\n'
  printf 'alice tunnel ip: %s\n' "$ALICE_TUN"
  printf 'bob   tunnel ip: %s\n\n' "$BOB_TUN"

  "${COMPOSE[@]}" exec -T node-b sh -c "pkill -9 iperf3 2>/dev/null; true" >/dev/null
  "${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
  sleep 1

  run_test_json tcp-single "TCP single stream" "$tcp_single_json"
  run_test_json tcp-4 "TCP 4 streams" "$tcp_4_json" -P 4
  run_test_json tcp-8 "TCP 8 streams" "$tcp_8_json" -P 8
  run_test_json udp-200 "UDP 200 Mbit target" "$udp_200_json" -u -b 200M
  if [[ ${#UDP1000_PARALLEL_ARGS[@]} -gt 0 ]]; then
    run_test_json udp-1000 "UDP 1000 Mbit target" "$udp_1000_json" -u -b "$UDP1000_PER_STREAM_BANDWIDTH" "${UDP1000_PARALLEL_ARGS[@]}"
  else
    run_test_json udp-1000 "UDP 1000 Mbit target" "$udp_1000_json" -u -b "$UDP1000_BANDWIDTH"
  fi
  write_iperf_socket_buffer_summary
  write_udp_receiver_limits
  run_ping_summary "$ping_output"
  collect_backend_artifacts
  docker_bench_append_summary_row \
    wireguard-go \
    "" \
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
  write_wireguard_go_cpu_phase_header
  docker_bench_write_metadata wireguard-go "$DURATION"
  write_wireguard_go_source_metadata
  start_compose_services
  for service in node-a node-b; do
    wait_for_service "$service"
  done
  configure_iperf_socket_buffer_limits
  docker_bench_start_cpu_stress
  run_wireguard_go_pass
  printf 'wireguard-go docker bench passed: wrote summary to %s\n' "$SUMMARY_TSV"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
