#!/usr/bin/env bash
# Optional release-gate check for host-pair latency while TCP load is active.
#
# All machine-specific targets come from env vars so hostnames, users, IPs,
# and peer identifiers stay out of committed files.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

MODE="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY:-auto}"
TARGET="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY_SSH:-${NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_SSH:-${NVPN_HOST_PAIR_SSH:-}}}"
DRY_RUN="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY_DRY_RUN:-0}"
CONNECT_TIMEOUT="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY_CONNECT_TIMEOUT:-5}"

LOCAL_NVPN="${NVPN_HOST_PAIR_LOCAL_NVPN:-nvpn}"
REMOTE_IPERF="${NVPN_HOST_PAIR_REMOTE_IPERF:-${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_REMOTE_IPERF:-iperf3}}"
LOCAL_IPERF="${NVPN_HOST_PAIR_IPERF:-${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_IPERF:-iperf3}}"
LOCAL_PEER="${NVPN_HOST_PAIR_LOCAL_PEER:-}"
EXPECTED_REMOTE_UNDERLAY_IP="${NVPN_HOST_PAIR_EXPECTED_REMOTE_UNDERLAY_IP:-}"
REMOTE_TUNNEL_IP="${NVPN_HOST_PAIR_REMOTE_TUNNEL_IP:-}"

DURATION_SECS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_DURATION_SECS:-180}"
SAMPLE_INTERVAL_SECS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_SAMPLE_INTERVAL_SECS:-60}"
PING_INTERVAL_SECS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_PING_INTERVAL_SECS:-0.2}"
IPERF_DURATION_SECS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_IPERF_DURATION_SECS:-10}"
IPERF_CONNECT_TIMEOUT_MS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_IPERF_CONNECT_TIMEOUT_MS:-3000}"
MIN_IPERF_INTERVAL_MBPS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MIN_INTERVAL_MBPS:-10}"
MAX_IPERF_NULL_SAMPLES="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_NULL_SAMPLES:-0}"
MAX_PING_LOSS_PERCENT="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_LOSS_PERCENT:-2}"
MAX_PING_P99_MS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_P99_MS:-1000}"
MAX_PING_GT350="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_GT350:-10}"
MAX_PING_GT350_PERCENT="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_GT350_PERCENT:-1.0}"
MAX_PING_GT1000="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_GT1000:-0}"
MAX_IPERF_SUB_1MBPS_INTERVALS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_SUB_1MBPS_INTERVALS:-0}"
MAX_IPERF_STALL_INTERVALS="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_STALL_INTERVALS:-1}"
OUTPUT_DIR="${NVPN_RELEASE_GATE_HOST_PAIR_LOADED_OUTPUT_DIR:-$ROOT_DIR/artifacts/host-pair-loaded-latency/$(date -u +%Y%m%dT%H%M%SZ)}"

is_disabled() {
  case "$1" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off) return 0 ;;
    *) return 1 ;;
  esac
}

is_enabled() {
  case "$1" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) return 0 ;;
    *) return 1 ;;
  esac
}

is_auto() {
  case "$1" in
    auto|AUTO|Auto|"") return 0 ;;
    *) return 1 ;;
  esac
}

skip() {
  printf 'Skipping host-pair loaded latency gate: %s\n' "$*"
}

fail() {
  printf 'host-pair loaded latency gate failed: %s\n' "$*" >&2
  exit 1
}

q() {
  printf '%q' "$1"
}

ssh_reachable() {
  ssh -o BatchMode=yes -o ConnectTimeout="$CONNECT_TIMEOUT" "$TARGET" true >/dev/null 2>&1
}

first_local_cmd() {
  local candidate
  for candidate in "$@"; do
    if [[ "$candidate" == */* && -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
    if command -v "$candidate" >/dev/null 2>&1; then
      command -v "$candidate"
      return 0
    fi
  done
  return 1
}

ping_count_for_duration() {
  local interval="${1:-$PING_INTERVAL_SECS}"
  awk -v duration="$DURATION_SECS" -v interval="$interval" '
    BEGIN {
      if (interval <= 0) interval = 1;
      count = int((duration / interval) + 0.999);
      if (count < 1) count = 1;
      print count;
    }'
}

effective_ping_interval_secs() {
  if [[ "$(uname -s)" == "Darwin" && "${EUID:-$(id -u)}" -ne 0 ]]; then
    awk -v interval="$PING_INTERVAL_SECS" '
      BEGIN {
        if (interval > 0 && interval < 1) print "1";
        else print interval;
      }'
  else
    printf '%s\n' "$PING_INTERVAL_SECS"
  fi
}

print_dry_run_env() {
  printf 'NVPN_HOST_PAIR_SSH=%q\n' "$TARGET"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_DURATION_SECS=%q\n' "$DURATION_SECS"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_SAMPLE_INTERVAL_SECS=%q\n' "$SAMPLE_INTERVAL_SECS"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_PING_INTERVAL_SECS=%q\n' "$PING_INTERVAL_SECS"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_IPERF_DURATION_SECS=%q\n' "$IPERF_DURATION_SECS"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MIN_INTERVAL_MBPS=%q\n' "$MIN_IPERF_INTERVAL_MBPS"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_LOSS_PERCENT=%q\n' "$MAX_PING_LOSS_PERCENT"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_SUB_1MBPS_INTERVALS=%q\n' "$MAX_IPERF_SUB_1MBPS_INTERVALS"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_STALL_INTERVALS=%q\n' "$MAX_IPERF_STALL_INTERVALS"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_P99_MS=%q\n' "$MAX_PING_P99_MS"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_GT350=%q\n' "$MAX_PING_GT350"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_GT350_PERCENT=%q\n' "$MAX_PING_GT350_PERCENT"
  printf 'NVPN_RELEASE_GATE_HOST_PAIR_LOADED_MAX_GT1000=%q\n' "$MAX_PING_GT1000"
  printf '%q\n' "$ROOT_DIR/scripts/release-gate-host-pair-loaded-latency.sh"
}

select_remote_tunnel_ip() {
  if [[ -n "$REMOTE_TUNNEL_IP" ]]; then
    printf '%s\n' "$REMOTE_TUNNEL_IP"
    return 0
  fi

  "$LOCAL_NVPN" status --json --discover-secs 0 | jq -er \
    --arg peer "$LOCAL_PEER" \
    --arg expected_ip "$EXPECTED_REMOTE_UNDERLAY_IP" '
      def peer_id: (.participant_pubkey // .fips_endpoint_npub // "");
      [
        .daemon.state.peers[]?
        | select(
            if $peer != "" then
              peer_id == $peer
            elif $expected_ip != "" then
              ((.fips_transport_addr // "") | startswith($expected_ip + ":"))
            else
              (.reachable == true)
            end
          )
        | (.tunnel_ip // "")
        | sub("/.*$"; "")
        | select(. != "")
      ]
      | unique
      | if length == 1 then .[0]
        else error("set NVPN_HOST_PAIR_REMOTE_TUNNEL_IP, NVPN_HOST_PAIR_LOCAL_PEER, or NVPN_HOST_PAIR_EXPECTED_REMOTE_UNDERLAY_IP")
        end'
}

start_remote_iperf() {
  local remote_iperf_q logfile_q
  remote_iperf_q="$(q "$REMOTE_IPERF")"
  logfile_q="$(q "/tmp/nvpn-loaded-latency-iperf3.log")"
  ssh -o BatchMode=yes -o ConnectTimeout="$CONNECT_TIMEOUT" "$TARGET" \
    "pgrep -x iperf3 >/dev/null 2>&1 || $remote_iperf_q -s -D --logfile $logfile_q"
}

stop_remote_iperf() {
  ssh -o BatchMode=yes -o ConnectTimeout="$CONNECT_TIMEOUT" "$TARGET" \
    'pkill -9 iperf3 >/dev/null 2>&1 || true' >/dev/null 2>&1 || true
}

parse_ping_summary() {
  local file="$1"
  local loss avg max p95 p99 spikes350 spikes1000 count
  loss="$(awk -F',' '/packet loss/ {for (i=1;i<=NF;i++) if ($i ~ /packet loss/) {gsub(/[^0-9.]/,"",$i); print $i}}' "$file" | tail -n1)"
  avg="$(awk -F'= ' '/round-trip|rtt min/ {split($2,a,"/"); print a[2]}' "$file" | tail -n1)"
  max="$(awk -F'= ' '/round-trip|rtt min/ {split($2,a,"/"); print a[3]}' "$file" | tail -n1)"
  count="$(sed -nE 's/.*time[=<][[:space:]]*([0-9.]+).*/\1/p' "$file" | wc -l | tr -d '[:space:]')"
  p95="$(sed -nE 's/.*time[=<][[:space:]]*([0-9.]+).*/\1/p' "$file" | sort -n | awk 'NF{v[++n]=$1} END{if(!n){print "null"} else {i=int((n*95+99)/100); if(i<1)i=1; if(i>n)i=n; printf "%.3f", v[i]}}')"
  p99="$(sed -nE 's/.*time[=<][[:space:]]*([0-9.]+).*/\1/p' "$file" | sort -n | awk 'NF{v[++n]=$1} END{if(!n){print "null"} else {i=int((n*99+99)/100); if(i<1)i=1; if(i>n)i=n; printf "%.3f", v[i]}}')"
  spikes350="$(sed -nE 's/.*time[=<][[:space:]]*([0-9.]+).*/\1/p' "$file" | awk '$1 > 350 {c++} END{print c+0}')"
  spikes1000="$(sed -nE 's/.*time[=<][[:space:]]*([0-9.]+).*/\1/p' "$file" | awk '$1 > 1000 {c++} END{print c+0}')"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${count:-0}" "${loss:-100}" "${avg:-null}" "$p95" "$p99" "${max:-null}" "$spikes350" "$spikes1000"
}

assert_float_at_most() {
  local actual="$1"
  local max="$2"
  local label="$3"
  awk -v actual="$actual" -v max="$max" -v label="$label" '
    BEGIN {
      if (actual == "" || actual == "null") {
        printf("host-pair loaded latency gate failed: %s missing\n", label) > "/dev/stderr";
        exit 1;
      }
      if ((actual + 0) > (max + 0)) {
        printf("host-pair loaded latency gate failed: %s %.3f exceeds %.3f\n", label, actual, max) > "/dev/stderr";
        exit 1;
      }
    }'
}

assert_int_at_most() {
  local actual="$1"
  local max="$2"
  local label="$3"
  if (( actual > max )); then
    fail "$label $actual exceeds $max"
  fi
}

run_gate() {
  local iperf_bin ping_count ping_interval ping_log remote_tunnel_ip start_epoch iter now sleep_for
  local fwd rev dir file mbps retrans lt1 lt10

  iperf_bin="$(first_local_cmd "$LOCAL_IPERF" /opt/homebrew/bin/iperf3 /usr/local/bin/iperf3 iperf3)" \
    || fail "missing local iperf3"
  command -v jq >/dev/null 2>&1 || fail "missing jq"
  command -v ping >/dev/null 2>&1 || fail "missing ping"
  command -v "$LOCAL_NVPN" >/dev/null 2>&1 || fail "missing local nvpn binary: $LOCAL_NVPN"

  remote_tunnel_ip="$(select_remote_tunnel_ip)"
  ping_interval="$(effective_ping_interval_secs)"
  if [[ "$ping_interval" != "$PING_INTERVAL_SECS" ]]; then
    printf 'host-pair loaded latency gate: clamped ping interval from %s to %s on non-root Darwin\n' \
      "$PING_INTERVAL_SECS" "$ping_interval"
  fi
  ping_count="$(ping_count_for_duration "$ping_interval")"

  mkdir -p "$OUTPUT_DIR"
  printf 'iteration\tdirection\tmbps\tretrans\tstall_lt_1mbps\tstall_lt_min_mbps\n' >"$OUTPUT_DIR/iperf-summary.tsv"
  ping_log="$OUTPUT_DIR/ping.log"

  stop_remote_iperf
  start_remote_iperf
  trap stop_remote_iperf EXIT

  (ping -c "$ping_count" -i "$ping_interval" "$remote_tunnel_ip" >"$ping_log" 2>&1 || true) &
  ping_pid=$!

  start_epoch="$(date +%s)"
  iter=0
  while :; do
    now="$(date +%s)"
    if (( now - start_epoch >= DURATION_SECS )); then
      break
    fi
    iter=$((iter + 1))
    start_remote_iperf || true
    fwd="$OUTPUT_DIR/iperf-forward-$iter.json"
    rev="$OUTPUT_DIR/iperf-reverse-$iter.json"
    "$iperf_bin" -J -c "$remote_tunnel_ip" -t "$IPERF_DURATION_SECS" -O 1 \
      --connect-timeout "$IPERF_CONNECT_TIMEOUT_MS" >"$fwd" 2>"$fwd.err" || true
    "$iperf_bin" -J -c "$remote_tunnel_ip" -t "$IPERF_DURATION_SECS" -O 1 \
      --connect-timeout "$IPERF_CONNECT_TIMEOUT_MS" -R >"$rev" 2>"$rev.err" || true

    for dir in forward reverse; do
      file="$OUTPUT_DIR/iperf-$dir-$iter.json"
      mbps="$(jq -r '([.end.sum_received.bits_per_second?, .end.sum.bits_per_second?, .end.sum_sent.bits_per_second?] | map(select(type=="number")) | if length == 0 then null else (.[0] / 1000000) end)' "$file" 2>/dev/null || printf 'null')"
      retrans="$(jq -r '.end.sum_sent.retransmits // .end.sum.retransmits // null' "$file" 2>/dev/null || printf 'null')"
      lt1="$(jq -r '[.intervals[]?.sum.bits_per_second? // empty | select(. < 1000000)] | length' "$file" 2>/dev/null || printf '0')"
      lt10="$(jq -r --arg min_mbps "$MIN_IPERF_INTERVAL_MBPS" '[.intervals[]?.sum.bits_per_second? // empty | select(. < (($min_mbps | tonumber) * 1000000))] | length' "$file" 2>/dev/null || printf '0')"
      printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$iter" "$dir" "$mbps" "$retrans" "$lt1" "$lt10" >>"$OUTPUT_DIR/iperf-summary.tsv"
    done

    now="$(date +%s)"
    sleep_for=$((start_epoch + iter * SAMPLE_INTERVAL_SECS - now))
    if (( sleep_for > 0 )); then
      sleep "$sleep_for"
    fi
  done

  wait "$ping_pid" || true
  stop_remote_iperf
  trap - EXIT

  {
    printf 'count\tloss_pct\tavg_ms\tp95_ms\tp99_ms\tmax_ms\tspikes_gt_350ms\tspikes_gt_1000ms\n'
    parse_ping_summary "$ping_log"
  } >"$OUTPUT_DIR/ping-summary.tsv"

  printf 'host-pair loaded latency artifact: %s\n' "$OUTPUT_DIR"
  printf 'host-pair loaded latency ping summary:\n'
  cat "$OUTPUT_DIR/ping-summary.tsv"
  printf 'host-pair loaded latency iperf summary:\n'
  cat "$OUTPUT_DIR/iperf-summary.tsv"

  local count loss avg p95 p99 max spikes350 spikes1000 nulls sub1_stalls min_stalls gt350_pct
  read -r count loss avg p95 p99 max spikes350 spikes1000 < <(sed -n '2p' "$OUTPUT_DIR/ping-summary.tsv")
  nulls="$(awk -F '\t' 'NR > 1 && $3 == "null" {c++} END{print c+0}' "$OUTPUT_DIR/iperf-summary.tsv")"
  sub1_stalls="$(awk -F '\t' 'NR > 1 {c += $5} END{print c+0}' "$OUTPUT_DIR/iperf-summary.tsv")"
  min_stalls="$(awk -F '\t' 'NR > 1 {c += $6} END{print c+0}' "$OUTPUT_DIR/iperf-summary.tsv")"
  gt350_pct="$(awk -v spikes="$spikes350" -v count="$count" 'BEGIN{if(count <= 0) print 100; else printf "%.6f", (spikes * 100.0) / count}')"

  assert_float_at_most "$loss" "$MAX_PING_LOSS_PERCENT" "ping loss %"
  assert_float_at_most "$p99" "$MAX_PING_P99_MS" "ping p99 ms"
  assert_int_at_most "$spikes350" "$MAX_PING_GT350" "ping spikes >350 ms"
  assert_float_at_most "$gt350_pct" "$MAX_PING_GT350_PERCENT" "ping spikes >350 ms %"
  assert_int_at_most "$spikes1000" "$MAX_PING_GT1000" "ping spikes >1000 ms"
  assert_int_at_most "$nulls" "$MAX_IPERF_NULL_SAMPLES" "iperf null samples"
  assert_int_at_most "$sub1_stalls" "$MAX_IPERF_SUB_1MBPS_INTERVALS" \
    "iperf intervals below 1 Mbps"
  assert_int_at_most "$min_stalls" "$MAX_IPERF_STALL_INTERVALS" \
    "iperf intervals below ${MIN_IPERF_INTERVAL_MBPS} Mbps"
}

main() {
  if is_disabled "$MODE"; then
    skip "NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY=$MODE"
    return 0
  fi
  if ! is_enabled "$MODE" && ! is_auto "$MODE"; then
    printf 'Unsupported NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY=%s\n' "$MODE" >&2
    exit 2
  fi
  if [[ -z "$TARGET" ]]; then
    if is_enabled "$MODE"; then
      fail "NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY=1 requires an SSH target"
    fi
    skip "set NVPN_RELEASE_GATE_HOST_PAIR_LOADED_LATENCY_SSH, NVPN_RELEASE_GATE_HOST_PAIR_LATENCY_SSH, or NVPN_HOST_PAIR_SSH to enable it"
    return 0
  fi
  if [[ "$DRY_RUN" == "1" ]]; then
    print_dry_run_env
    return 0
  fi
  if ! ssh_reachable; then
    if is_enabled "$MODE"; then
      fail "ssh target is unreachable"
    fi
    skip "ssh target is unreachable"
    return 0
  fi
  run_gate
}

main "$@"
