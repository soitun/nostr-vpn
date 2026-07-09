#!/usr/bin/env bash
# Local self-tests for simple Docker benchmark summary helpers.
#
# These tests do not start Docker. They pin the JSON/ping parsers and TSV row
# contract used by scripts/perf-docker.sh and scripts/perf-docker-boringtun.sh.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
COMPARE_SCRIPT="$ROOT_DIR/scripts/compare-docker-benchmarks.sh"
TABLE_SCRIPT="$ROOT_DIR/scripts/summarize-docker-benchmark-table.sh"

# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"

fail() {
  printf 'docker benchmark summary self-test failed: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  local got="$1"
  local want="$2"
  local label="$3"
  [[ "$got" == "$want" ]] || fail "$label: got '$got', want '$want'"
}

assert_file_contains() {
  local path="$1"
  local needle="$2"
  local label="$3"
  grep -Fq "$needle" "$path" || fail "$label: missing '$needle' in $path"
}

assert_file_not_contains() {
  local path="$1"
  local needle="$2"
  local label="$3"
  ! grep -Fq "$needle" "$path" || fail "$label: unexpected '$needle' in $path"
}

table_values() {
  local path="$1"
  local label="$2"
  shift 2
  local fields="$*"
  awk -F '\t' -v label="$label" -v fields="$fields" '
    BEGIN {
      want_count = split(fields, want, " ")
    }
    NR == 1 {
      for (i = 1; i <= NF; i++) header[$i] = i
      next
    }
    $1 == label {
      for (i = 1; i <= want_count; i++) {
        idx = header[want[i]]
        if (!idx) exit 2
        printf "%s%s", (i == 1 ? "" : "\t"), $idx
      }
      printf "\n"
      found = 1
      exit
    }
    END {
      if (!found) exit 3
    }' "$path"
}

write_tcp_json() {
  local path="$1"
  cat >"$path" <<'EOF'
{
  "end": {
    "sum_received": {
      "bits_per_second": 1234567890,
      "bytes": 125000000
    },
    "sum_sent": {
      "retransmits": 7
    }
  }
}
EOF
}

write_udp_json() {
  local path="$1"
  cat >"$path" <<'EOF'
{
  "end": {
    "sum": {
      "bits_per_second": 987654321,
      "bytes": 62500000,
      "lost_percent": 1.25
    }
  }
}
EOF
}

write_ping_output() {
  local path="$1"
  cat >"$path" <<'EOF'
PING 10.44.0.2 (10.44.0.2) 56(84) bytes of data.
64 bytes from 10.44.0.2: icmp_seq=1 ttl=64 time=0.400 ms
64 bytes from 10.44.0.2: icmp_seq=2 ttl=64 time=1.200 ms
64 bytes from 10.44.0.2: icmp_seq=3 ttl=64 time=8.900 ms

--- 10.44.0.2 ping statistics ---
300 packets transmitted, 299 received, 0.333333% packet loss, time 3017ms
rtt min/avg/max/mdev = 0.400/1.234/8.900/0.500 ms
EOF
}

test_json_and_ping_parsers() {
  local dir tcp_json udp_json ping_output stats loss avg
  local tail_stats ping_mdev ping_p95 ping_p99 ping_max ping_samples ping_gt1 ping_gt2 ping_gt10
  dir="$(mktemp -d)"
  tcp_json="$dir/tcp.json"
  udp_json="$dir/udp.json"
  ping_output="$dir/ping.txt"
  write_tcp_json "$tcp_json"
  write_udp_json "$udp_json"
  write_ping_output "$ping_output"

  assert_eq "$(docker_bench_iperf_mbps "$tcp_json")" "1234.568" "TCP receiver Mbps"
  assert_eq "$(docker_bench_iperf_retrans "$tcp_json")" "7" "TCP retransmits"
  assert_eq "$(docker_bench_iperf_transfer_bytes "$tcp_json")" "125000000" "TCP transfer bytes"
  assert_eq "$(docker_bench_iperf_mbps "$udp_json")" "987.654" "UDP receiver Mbps"
  assert_eq "$(docker_bench_iperf_loss_pct "$udp_json")" "1.25" "UDP loss"
  assert_eq "$(docker_bench_iperf_transfer_bytes "$udp_json")" "62500000" "UDP transfer bytes"
  stats="$(docker_bench_parse_ping_loss_avg "$ping_output")"
  read -r loss avg <<<"$stats"
  assert_eq "$loss" "0.333333" "ping loss"
  assert_eq "$avg" "1.234" "ping avg"
  tail_stats="$(IFS=$'\t'; docker_bench_parse_ping_tail_stats "$ping_output")"
  IFS=$'\t' read -r ping_mdev ping_p95 ping_p99 ping_max ping_samples ping_gt1 ping_gt2 ping_gt10 \
    <<<"$tail_stats"
  assert_eq "$ping_mdev" "0.500" "ping mdev"
  assert_eq "$ping_p95" "8.900" "ping p95"
  assert_eq "$ping_p99" "8.900" "ping p99"
  assert_eq "$ping_max" "8.900" "ping max"
  assert_eq "$ping_samples" "3" "ping sample count"
  assert_eq "$ping_gt1" "2" "ping >1ms count"
  assert_eq "$ping_gt2" "1" "ping >2ms count"
  assert_eq "$ping_gt10" "0" "ping >10ms count"

  rm -rf "$dir"
}

test_cpu_accounting_helpers() {
  local invalid
  assert_eq "$(docker_bench_cpu_seconds_from_jiffies 100 250 100)" "1.500000" "CPU jiffies to seconds"
  assert_eq "$(docker_bench_cpu_seconds_per_gbyte 2.5 125000000)" "20.000000" "CPU seconds per GByte"
  invalid="$(docker_bench_cpu_seconds_from_jiffies 250 100 100)"
  assert_eq "$invalid" "" "invalid negative jiffies delta"
  invalid="$(docker_bench_cpu_seconds_per_gbyte 2.5 0)"
  assert_eq "$invalid" "" "missing transfer bytes"
}

test_udp1000_parallel_bandwidth_helpers_preserve_total_target() {
  local per_stream default_streams
  per_stream="$(
    NVPN_DOCKER_UDP1000_BANDWIDTH=1G \
    NVPN_DOCKER_UDP1000_PARALLEL=4 \
      docker_bench_udp1000_per_stream_bandwidth
  )"
  default_streams="$(docker_bench_udp1000_parallel_streams)"

  assert_eq "$per_stream" "250000000" "UDP1000 P4 per-stream bandwidth"
  assert_eq "$default_streams" "1" "UDP1000 default stream count"
}

test_summary_row() {
  local dir tcp_single tcp_4 tcp_8 udp_200 udp_1000 ping_output
  local header row nvpn_row fields nvpn_fields lane_header lane_row
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  RAW_DIR="$OUTPUT_DIR/raw"
  SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
  DURATION=3
  docker_bench_init_summary

  tcp_single="$dir/tcp-single.json"
  tcp_4="$dir/tcp-4.json"
  tcp_8="$dir/tcp-8.json"
  udp_200="$dir/udp-200.json"
  udp_1000="$dir/udp-1000.json"
  ping_output="$dir/ping.txt"
  write_tcp_json "$tcp_single"
  write_tcp_json "$tcp_4"
  write_tcp_json "$tcp_8"
  write_udp_json "$udp_200"
  write_udp_json "$udp_1000"
  write_ping_output "$ping_output"

  docker_bench_append_summary_row boringtun 1 "$DURATION" "$RAW_DIR" "$tcp_single" "$tcp_4" "$tcp_8" "$udp_200" "$udp_1000" "$ping_output"
  docker_bench_append_summary_row nvpn "" "$DURATION" "$RAW_DIR" "$tcp_single" "$tcp_4" "$tcp_8" "$udp_200" "$udp_1000" "$ping_output"
  header="$(awk -F '\t' 'NR == 1 { print $1 "\t" $2 "\t" $3 "\t" $16 }' "$SUMMARY_TSV")"
  row="$(awk -F '\t' 'NR == 2 { print $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5 "\t" $10 "\t" $11 "\t" $14 "\t" $15 }' "$SUMMARY_TSV")"
  nvpn_row="$(awk -F '\t' 'NR == 3 { print $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5 "\t" $10 "\t" $11 "\t" $14 "\t" $15 }' "$SUMMARY_TSV")"
  fields="$(awk -F '\t' 'NR == 2 { print NF }' "$SUMMARY_TSV")"
  nvpn_fields="$(awk -F '\t' 'NR == 3 { print NF }' "$SUMMARY_TSV")"

  assert_eq "$header" $'backend\tthreads\tduration_secs\traw_dir' "summary header"
  assert_eq "$row" $'boringtun\t1\t3\t1234.568\t7\t987.654\t1.25\t0.333333\t1.234' "summary row"
  assert_eq "$nvpn_row" $'nvpn\t\t3\t1234.568\t7\t987.654\t1.25\t0.333333\t1.234' "nvpn summary row"
  assert_eq "$fields" "34" "summary field count"
  assert_eq "$nvpn_fields" "34" "nvpn summary field count"

  rm -rf "$dir"

  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  RAW_DIR="$OUTPUT_DIR/raw"
  SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
  DURATION=3
  docker_bench_init_summary

  tcp_single="$dir/tcp-single.json"
  tcp_4="$dir/tcp-4.json"
  tcp_8="$dir/tcp-8.json"
  udp_200="$dir/udp-200.json"
  udp_1000="$dir/udp-1000.json"
  ping_output="$dir/ping.txt"
  write_tcp_json "$tcp_single"
  write_tcp_json "$tcp_4"
  write_tcp_json "$tcp_8"
  write_udp_json "$udp_200"
  write_udp_json "$udp_1000"
  write_ping_output "$ping_output"

  (
    export NVPN_DOCKER_CPU_STRESS=1
    export NVPN_DOCKER_CPU_STRESS_SIDES=both
    export NVPN_DOCKER_CPU_STRESS_LOCAL_WORKERS=2
    export NVPN_DOCKER_CPU_STRESS_REMOTE_WORKERS=3
    export NVPN_DOCKER_IPERF_SOCKET_BUFFER=4M
    export NVPN_DOCKER_UDP1000_PARALLEL=4
    export NVPN_DOCKER_UDP1000_BANDWIDTH=1G
    export NVPN_DOCKER_DATAPLANE_PROFILE=linux-vnet-lan
    export NVPN_DOCKER_PLACEMENT_PROFILE=worker-open
    docker_bench_append_summary_row nvpn "" "$DURATION" "$RAW_DIR" "$tcp_single" "$tcp_4" "$tcp_8" "$udp_200" "$udp_1000" "$ping_output"
  )
  lane_header="$(awk -F '\t' 'NR == 1 { print $17 "\t" $18 "\t" $19 "\t" $20 "\t" $21 "\t" $22 "\t" $23 "\t" $24 "\t" $25 "\t" $26 }' "$SUMMARY_TSV")"
  lane_row="$(awk -F '\t' 'NR == 2 { print $17 "\t" $18 "\t" $19 "\t" $20 "\t" $21 "\t" $22 "\t" $23 "\t" $24 "\t" $25 "\t" $26 }' "$SUMMARY_TSV")"

  assert_eq "$lane_header" $'cpu_stress_enabled\tcpu_stress_sides\tcpu_stress_local_workers\tcpu_stress_remote_workers\tiperf_socket_buffer\tudp1000_parallel\tudp1000_bandwidth\tudp1000_per_stream_bandwidth\tdataplane_profile\tplacement_profile' "summary lane metadata header"
  assert_eq "$lane_row" $'true\tboth\t2\t3\t4M\t4\t1G\t250000000\tlinux-vnet-lan\tworker-open' "summary lane metadata row"

  rm -rf "$dir"
}

test_metadata_writer_records_cpu_stress() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    unset NVPN_FIPS_REPO_PATH
    unset NVPN_PATCH_LOCAL_FIPS
    export NVPN_DOCKER_CPU_STRESS=1
    export NVPN_DOCKER_CPU_STRESS_SIDES=remote
    export NVPN_DOCKER_CPU_STRESS_WORKERS=2
    export NVPN_DOCKER_CPU_STRESS_REMOTE_WORKERS=5
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.backend' "$metadata")" "nvpn" "metadata backend"
  assert_eq "$(jq -r '.cpu_stress.enabled' "$metadata")" "true" "metadata stress enabled"
  assert_eq "$(jq -r '.cpu_stress.sides' "$metadata")" "remote" "metadata stress sides"
  assert_eq "$(jq -r '.cpu_stress.local_workers' "$metadata")" "0" "metadata local workers"
  assert_eq "$(jq -r '.cpu_stress.remote_workers' "$metadata")" "5" "metadata remote workers"
  assert_eq "$(jq -r '.source.local_fips_patch.enabled' "$metadata")" "false" "metadata local FIPS default"

  rm -rf "$dir"
}

test_local_fips_repo_path_defaults_to_patch() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR" "$dir/fips"

  (
    unset NVPN_PATCH_LOCAL_FIPS
    export NVPN_FIPS_REPO_PATH="$dir/fips"
    docker_bench_apply_local_fips_patch_default
    assert_eq "${NVPN_PATCH_LOCAL_FIPS:-}" "1" "local FIPS path exports patch default"
    docker_bench_write_metadata nvpn 3
  )
  assert_eq "$(jq -r '.source.local_fips_patch.enabled' "$metadata")" "true" "metadata local FIPS defaults on with repo path"

  OUTPUT_DIR="$dir/out-explicit-off"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"
  (
    export NVPN_FIPS_REPO_PATH="$dir/fips"
    export NVPN_PATCH_LOCAL_FIPS=0
    docker_bench_apply_local_fips_patch_default
    assert_eq "$NVPN_PATCH_LOCAL_FIPS" "0" "explicit local FIPS patch opt-out is preserved"
    docker_bench_write_metadata nvpn 3
  )
  assert_eq "$(jq -r '.source.local_fips_patch.enabled' "$metadata")" "false" "metadata local FIPS explicit opt-out"

  rm -rf "$dir"
}

test_metadata_writer_records_pipeline_trace() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_PIPELINE_TRACE=1
    export NVPN_DOCKER_PIPELINE_INTERVAL_SECS=2
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.pipeline_trace.enabled' "$metadata")" "true" "metadata pipeline trace enabled"
  assert_eq "$(jq -r '.pipeline_trace.interval_secs' "$metadata")" "2" "metadata pipeline trace interval"

  rm -rf "$dir"
}

test_metadata_writer_records_iperf_interval() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_IPERF_INTERVAL_SECS=1
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.iperf.interval_secs' "$metadata")" "1" "metadata iperf interval"

  rm -rf "$dir"
}

test_metadata_writer_records_iperf_timeout() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_IPERF_TIMEOUT_SECS=11
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.iperf.timeout_secs' "$metadata")" "11" "metadata iperf timeout"

  rm -rf "$dir"
}

test_metadata_writer_records_udp1000_per_stream_bandwidth() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_UDP1000_BANDWIDTH=1G
    export NVPN_DOCKER_UDP1000_PARALLEL=4
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.iperf.udp1000_bandwidth' "$metadata")" "1G" "metadata UDP1000 total bandwidth"
  assert_eq "$(jq -r '.iperf.udp1000_parallel' "$metadata")" "4" "metadata UDP1000 parallel"
  assert_eq "$(jq -r '.iperf.udp1000_per_stream_bandwidth' "$metadata")" "250000000" "metadata UDP1000 per-stream bandwidth"

  rm -rf "$dir"
}

test_metadata_writer_records_guard_thresholds() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_MIN_TCP_MBPS=1000
    export NVPN_DOCKER_MIN_TCP_SINGLE_MBPS=1200
    export NVPN_DOCKER_MAX_TCP_RETRANS=9000
    export NVPN_DOCKER_MAX_TCP_8_RETRANS=12000
    export NVPN_DOCKER_MAX_UDP_LOSS_PCT=2
    export NVPN_DOCKER_MAX_UDP1000_LOSS_PCT=5
    export NVPN_DOCKER_MAX_PING_LOSS_PCT=0
    export NVPN_DOCKER_MAX_CONNECTED_UDP_DRAIN_BULK_DROPPED=0
    export NVPN_DOCKER_MAX_CONNECTED_UDP_DIRECT_DECRYPT_BULK_SHED=0
    export NVPN_DOCKER_MAX_ENDPOINT_EVENT_BULK_DROPPED=0
    export NVPN_DOCKER_MAX_TUN_RX_DROPPED=0
    export NVPN_DOCKER_MAX_TUN_TX_DROPPED=0
    export NVPN_DOCKER_MAX_DECRYPT_WORKER_QUEUE_FULL=0
    export NVPN_DOCKER_MAX_DECRYPT_WORKER_BULK_DROPPED=0
    export NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRESSURE_DRAIN=0
    export NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRIORITY_GATED=0
    export NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_COMPLETION_BACKLOG_FALLBACK=0
    export NVPN_DOCKER_MAX_DECRYPT_FSP_WORKER_REPLAY_DROPPED=0
    export NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_AEAD_FAILED=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_AEAD_FAILED=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_EPOCH_MISMATCH=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_SESSION=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_ORDER=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_TICKET=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_DUPLICATE_TICKET=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_WINDOW_EXCEEDED=0
    export NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_REPLAY_DROPPED=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED=0
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.guard_thresholds.min_tcp_mbps' "$metadata")" "1000" "metadata common TCP min guard"
  assert_eq "$(jq -r '.guard_thresholds.min_tcp_single_mbps' "$metadata")" "1200" "metadata TCP single min guard"
  assert_eq "$(jq -r '.guard_thresholds.max_tcp_retrans' "$metadata")" "9000" "metadata common retrans guard"
  assert_eq "$(jq -r '.guard_thresholds.max_tcp_8_retrans' "$metadata")" "12000" "metadata TCP 8 retrans guard"
  assert_eq "$(jq -r '.guard_thresholds.max_udp_loss_pct' "$metadata")" "2" "metadata common UDP loss guard"
  assert_eq "$(jq -r '.guard_thresholds.max_udp1000_loss_pct' "$metadata")" "5" "metadata UDP1000 loss guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_loss_pct' "$metadata")" "0" "metadata ping loss guard"
  assert_eq "$(jq -r '.guard_thresholds.max_connected_udp_drain_bulk_dropped' "$metadata")" "0" "metadata connected UDP drain bulk drop guard"
  assert_eq "$(jq -r '.guard_thresholds.max_connected_udp_direct_decrypt_bulk_shed' "$metadata")" "0" "metadata connected UDP direct-decrypt shed guard"
  assert_eq "$(jq -r '.guard_thresholds.max_endpoint_event_bulk_dropped' "$metadata")" "0" "metadata endpoint event bulk drop guard"
  assert_eq "$(jq -r '.guard_thresholds.max_tun_rx_dropped' "$metadata")" "0" "metadata TUN RX drop guard"
  assert_eq "$(jq -r '.guard_thresholds.max_tun_tx_dropped' "$metadata")" "0" "metadata TUN TX drop guard"
  assert_eq "$(jq -r '.guard_thresholds.max_decrypt_worker_queue_full' "$metadata")" "0" "metadata decrypt worker queue-full guard"
  assert_eq "$(jq -r '.guard_thresholds.max_decrypt_worker_bulk_dropped' "$metadata")" "0" "metadata decrypt worker bulk drop guard"
  assert_eq "$(jq -r '.guard_thresholds.max_decrypt_fallback_pressure_drain' "$metadata")" "0" "metadata decrypt fallback pressure drain guard"
  assert_eq "$(jq -r '.guard_thresholds.max_decrypt_fallback_priority_gated' "$metadata")" "0" "metadata decrypt fallback priority gated guard"
  assert_eq "$(jq -r '.guard_thresholds.max_decrypt_fsp_open_worker_completion_backlog_fallback' "$metadata")" "0" "metadata FSP open-worker completion backlog fallback guard"
  assert_eq "$(jq -r '.guard_thresholds.max_decrypt_fsp_worker_replay_dropped' "$metadata")" "0" "metadata FSP worker replay drop guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fmp_aead_completion_aead_failed' "$metadata")" "0" "metadata FMP AEAD completion failed guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_aead_completion_aead_failed' "$metadata")" "0" "metadata FSP AEAD completion failed guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_aead_completion_epoch_mismatch' "$metadata")" "0" "metadata FSP AEAD completion epoch mismatch guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_aead_completion_stale_session' "$metadata")" "0" "metadata FSP stale session guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_aead_completion_stale_order' "$metadata")" "0" "metadata FSP stale order guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_aead_completion_stale_ticket' "$metadata")" "0" "metadata FSP stale ticket guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_aead_completion_duplicate_ticket' "$metadata")" "0" "metadata FSP duplicate ticket guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_aead_completion_window_exceeded' "$metadata")" "0" "metadata FSP completion window guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fmp_aead_completion_replay_dropped' "$metadata")" "0" "metadata FMP AEAD completion replay drop guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_aead_completion_replay_dropped' "$metadata")" "0" "metadata FSP AEAD completion replay drop guard"
  assert_eq "$(jq -r '.guard_thresholds.max_udp200_loss_pct' "$metadata")" "null" "metadata unset guard"

  rm -rf "$dir"
}

test_metadata_writer_records_fips_soak_thresholds() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    MAX_PING_LOSS_PERCENT=0
    MAX_PING_AVG_MS=25
    MAX_PING_P95_MS=50
    MAX_PING_P99_MS=75
    MAX_PING_MAX_MS=100
    MAX_PING_AVG_DRIFT_MS=5
    MAX_PING_AVG_DRIFT_FACTOR=3
    MAX_PING_P95_DRIFT_MS=10
    MAX_PING_P95_DRIFT_FACTOR=4
    MAX_PING_P99_DRIFT_MS=15
    MAX_PING_P99_DRIFT_FACTOR=5
    MAX_SRTT_MS=250
    MAX_SRTT_DRIFT_MS=20
    MAX_SRTT_DRIFT_FACTOR=6
    MAX_CONSECUTIVE_HIGH_SRTT_SAMPLES=2
    MAX_FIPS_LAST_SEEN_AGE_SECS=30
    MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS=10
    MAX_FIPS_DATA_LAST_SEEN_AGE_SECS=20
    MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=3
    EXPECT_FSP_OWNER_PLACEMENT=worker-open
    docker_bench_write_metadata fips-soak 600
  )

  assert_eq "$(jq -r '.run_env.expected_fsp_owner_placement' "$metadata")" "worker-open" "metadata soak expected placement"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_loss_pct' "$metadata")" "0" "metadata soak ping loss guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_avg_ms' "$metadata")" "25" "metadata soak ping avg guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_p95_ms' "$metadata")" "50" "metadata soak ping p95 guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_p99_ms' "$metadata")" "75" "metadata soak ping p99 guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_max_ms' "$metadata")" "100" "metadata soak ping max guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_avg_drift_ms' "$metadata")" "5" "metadata soak ping avg drift guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_avg_drift_factor' "$metadata")" "3" "metadata soak ping avg drift factor"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_p95_drift_ms' "$metadata")" "10" "metadata soak ping p95 drift guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_p95_drift_factor' "$metadata")" "4" "metadata soak ping p95 drift factor"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_p99_drift_ms' "$metadata")" "15" "metadata soak ping p99 drift guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_p99_drift_factor' "$metadata")" "5" "metadata soak ping p99 drift factor"
  assert_eq "$(jq -r '.guard_thresholds.max_srtt_ms' "$metadata")" "250" "metadata soak SRTT guard"
  assert_eq "$(jq -r '.guard_thresholds.max_srtt_drift_ms' "$metadata")" "20" "metadata soak SRTT drift guard"
  assert_eq "$(jq -r '.guard_thresholds.max_srtt_drift_factor' "$metadata")" "6" "metadata soak SRTT drift factor"
  assert_eq "$(jq -r '.guard_thresholds.max_consecutive_high_srtt_samples' "$metadata")" "2" "metadata soak consecutive high SRTT guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fips_last_seen_age_secs' "$metadata")" "30" "metadata soak FIPS liveness guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fips_control_last_seen_age_secs' "$metadata")" "10" "metadata soak FIPS control liveness guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fips_data_last_seen_age_secs' "$metadata")" "20" "metadata soak FIPS data liveness guard"
  assert_eq "$(jq -r '.guard_thresholds.max_fips_last_seen_future_skew_secs' "$metadata")" "3" "metadata soak FIPS future skew guard"

  rm -rf "$dir"
}

test_metadata_writer_records_run_provenance() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_EXTRA_ENV="FIPS_LINUX_BULK_CONTAINERS=1"
    export NVPN_PATCH_LOCAL_FIPS=1
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.run_env.extra_connect_env' "$metadata")" "FIPS_LINUX_BULK_CONTAINERS=1" "metadata extra env"
  assert_eq "$(jq -r '.run_env.dataplane_profile' "$metadata")" "null" "metadata dataplane profile default"
  assert_eq "$(jq -r '.run_env.direct_fmp_forced' "$metadata")" "false" "metadata direct-FMP forced default"
  assert_eq "$(jq -r '.run_env.require_no_direct_fmp' "$metadata")" "false" "metadata no-direct requirement default"
  assert_eq "$(jq -r '.run_env.require_no_fsp_aead_helpers' "$metadata")" "false" "metadata no-FSP-helper requirement default"
  assert_eq "$(jq -r '.host | has("load1")' "$metadata")" "true" "metadata host load1 field"
  assert_eq "$(jq -r '.host | has("load5")' "$metadata")" "true" "metadata host load5 field"
  assert_eq "$(jq -r '.host | has("load15")' "$metadata")" "true" "metadata host load15 field"
  assert_eq "$(jq -r '.host | has("online_cpus")' "$metadata")" "true" "metadata host CPU field"
  assert_eq "$(jq -r '.host | has("load1_per_cpu")' "$metadata")" "true" "metadata host load-per-CPU field"
  assert_eq "$(jq -r '.source.local_fips_patch.enabled' "$metadata")" "true" "metadata local FIPS enabled"
  assert_eq "$(jq -r '.source.nvpn | has("git_head")' "$metadata")" "true" "metadata nvpn head field"
  assert_eq "$(jq -r '.source.local_fips_patch | has("git_head")' "$metadata")" "true" "metadata FIPS head field"

  rm -rf "$dir"
}

test_host_quiet_guard_rejects_invalid_threshold() {
  local output status
  set +e
  output="$(
    NVPN_DOCKER_MAX_HOST_LOAD_PER_CPU=bogus \
      docker_bench_validate_host_quiet test-host 2>&1
  )"
  status=$?
  set -e

  [[ "$status" != "0" ]] || fail "invalid host quiet threshold was accepted"
  case "$output" in
    *"invalid NVPN_DOCKER_MAX_HOST_LOAD_PER_CPU=bogus"*) ;;
    *) fail "invalid host quiet threshold diagnostic missing: $output" ;;
  esac
}

test_dataplane_profile_expands_connect_env() {
  local dir metadata effective
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_DATAPLANE_PROFILE=linux-vnet-lan
    effective="$(docker_bench_effective_extra_env)"
    printf '%s' "$effective" >"$dir/effective-env"
    docker_bench_validate_connect_env_scope "$effective"
    docker_bench_write_metadata nvpn 3
  )

  effective="$(cat "$dir/effective-env")"
  assert_eq "$effective" "NVPN_MESH_UNDERLAY_UDP_MTU=1472" "profile effective connect env"
  assert_eq "$(jq -r '.run_env.dataplane_profile' "$metadata")" "linux-vnet-lan" "metadata dataplane profile"
  assert_eq "$(jq -r '.run_env.extra_connect_env' "$metadata")" "$effective" "metadata effective extra env"

  rm -rf "$dir"
}

test_placement_profile_expands_worker_open_connect_env() {
  local dir metadata effective
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_PLACEMENT_PROFILE=worker-open
    export NVPN_DOCKER_MAX_FSP_OWNER_PLACEMENT_OTHER_PATH_RATE=0
    export NVPN_DOCKER_PLACEMENT_PREFLIGHT=1
    export NVPN_DOCKER_PLACEMENT_PREFLIGHT_MODE=tcp
    export NVPN_DOCKER_PLACEMENT_PREFLIGHT_DURATION=3
    export NVPN_DOCKER_PLACEMENT_PREFLIGHT_STREAMS=4
    effective="$(docker_bench_effective_extra_env)"
    printf '%s' "$effective" >"$dir/effective-env"
    docker_bench_write_metadata nvpn 3
  )

  effective="$(cat "$dir/effective-env")"
  assert_eq "$effective" "" "placement profile effective connect env"
  assert_eq "$(jq -r '.run_env.placement_profile' "$metadata")" "worker-open" "metadata placement profile"
  assert_eq "$(jq -r '.run_env.expected_fsp_owner_placement' "$metadata")" "worker-open" "metadata expected placement"
  assert_eq "$(jq -r '.run_env.expected_fsp_owner_placement_exclusive' "$metadata")" "true" "metadata expected exclusive placement"
  assert_eq "$(jq -r '.run_env.placement_preflight' "$metadata")" "1" "metadata placement preflight"
  assert_eq "$(jq -r '.run_env.placement_preflight_mode' "$metadata")" "tcp" "metadata placement preflight mode"
  assert_eq "$(jq -r '.run_env.placement_preflight_duration_secs' "$metadata")" "3" "metadata placement preflight duration"
  assert_eq "$(jq -r '.run_env.placement_preflight_streams' "$metadata")" "4" "metadata placement preflight streams"
  assert_eq "$(jq -r '.run_env.setup_ping_attempts' "$metadata")" "8" "metadata setup ping attempts"
  assert_eq "$(jq -r '.run_env.setup_ping_wait_secs' "$metadata")" "1" "metadata setup ping wait"
  assert_eq "$(jq -r '.guard_thresholds.max_fsp_owner_placement_other_path_rate' "$metadata")" "0" "metadata max alternate placement path rate"
  assert_eq "$(jq -r '.run_env.extra_connect_env' "$metadata")" "$effective" "metadata placement effective env"

  rm -rf "$dir"
}

test_placement_profile_allows_explicit_nonexclusive_worker_open_guard() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_PLACEMENT_PROFILE=worker-open
    export NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE=0
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.run_env.expected_fsp_owner_placement' "$metadata")" "worker-open" "metadata default worker-open expected placement"
  assert_eq "$(jq -r '.run_env.expected_fsp_owner_placement_exclusive' "$metadata")" "false" "metadata explicit nonexclusive placement"

  rm -rf "$dir"
}

test_dataplane_profile_rejects_unknown_profile() {
  local output status
  set +e
  output="$(
    NVPN_DOCKER_DATAPLANE_PROFILE=bogus docker_bench_effective_extra_env 2>&1
  )"
  status=$?
  set -e

  [[ "$status" != "0" ]] || fail "unknown dataplane profile was accepted"
  case "$output" in
    *"unknown NVPN_DOCKER_DATAPLANE_PROFILE=bogus"*) ;;
    *) fail "unknown dataplane profile diagnostic missing: $output" ;;
  esac
}

test_connect_env_scope_rejects_stranded_outer_env() {
  local output status
  set +e
  output="$(
    NVPN_FIPS_LINUX_TUN_TX_QUEUE_LEN=500 \
      docker_bench_validate_connect_env_scope "" 2>&1
  )"
  status=$?
  set -e

  [[ "$status" != "0" ]] || fail "stranded outer connect env was accepted"
  case "$output" in
    *"NVPN_FIPS_LINUX_TUN_TX_QUEUE_LEN=500 is set outside the daemon connect env"*) ;;
    *) fail "stranded qlen connect env diagnostic missing: $output" ;;
  esac
}

test_metadata_writer_records_direct_fmp_guard() {
  local dir metadata helper_result
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_EXTRA_ENV="FIPS_LINUX_BULK_CONTAINERS=1 FIPS_DIRECT_ENDPOINT_FMP_ONLY=1"
    export NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP=1
    if docker_bench_direct_fmp_forced_enabled; then
      printf 'yes'
    else
      printf 'no'
    fi >"$dir/helper-result"
    docker_bench_write_metadata nvpn 3
  )
  helper_result="$(cat "$dir/helper-result")"

  assert_eq "$helper_result" "yes" "direct-FMP env detector"
  assert_eq "$(jq -r '.run_env.direct_fmp_forced' "$metadata")" "true" "metadata direct-FMP forced"
  assert_eq "$(jq -r '.run_env.require_no_direct_fmp' "$metadata")" "true" "metadata no-direct requirement"

  rm -rf "$dir"
}

test_metadata_writer_records_fsp_aead_helper_guard() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_REQUIRE_NO_FSP_AEAD_HELPERS=1
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.run_env.require_no_fsp_aead_helpers' "$metadata")" "true" "metadata no-FSP-helper requirement"

  rm -rf "$dir"
}

test_metadata_writer_records_pipeline_hard_event_guard() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_REQUIRE_NO_PIPELINE_HARD_EVENTS=1
    export NVPN_DOCKER_ALLOW_PIPELINE_HARD_EVENTS=nvpn_tun_to_mesh_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped_batches
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.run_env.require_no_pipeline_hard_events' "$metadata")" "true" "metadata no-pipeline-hard-events requirement"
  assert_eq "$(jq -r '.run_env.allowed_pipeline_hard_events' "$metadata")" "nvpn_tun_to_mesh_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped_batches" "metadata allowed pipeline hard events"

  rm -rf "$dir"
}

test_extra_env_validation_rejects_retired_dataplane_knob_names() {
  local output status
  set +e
  output="$(
    docker_bench_validate_extra_env_assignments \
      "FIPS_DECRYPT_FMP_AEAD_HELPERS=2 FIPS_DECRYPT_FMP_SOURCE_AFFINE_SESSION_OWNER=1 FIPS_DECRYPT_FSP_LOCAL_BULK_OPEN_WORKER=1 FIPS_DECRYPT_FSP_REMOTE_BULK_OPEN_WORKER=1 FIPS_DECRYPT_FSP_OPEN_WORKER_MAX_COMPLETION_BACKLOG=64 FIPS_DECRYPT_FSP_AEAD_COMPLETION_BATCH_MAX=64 FIPS_LINUX_BULK_UDP_PACE_MBPS=2500 FIPS_LINUX_BULK_UDP_PACE_BURST_BYTES=65536 FIPS_LINUX_BULK_UDP_PACE_SPIN_NS=200000 FIPS_MACOS_ORDERED_SENDER=1 FIPS_MACOS_WORKER_STRIDE=16 FIPS_MACOS_SEND_FLOW_IDLE_MS=60000 NVPN_FIPS_LINUX_TUN_VNET=1" \
      2>&1
  )"
  status=$?
  set -e

  [[ "$status" != "0" ]] || fail "retired dataplane knob env names were accepted"
  case "$output" in
    *"FIPS_DECRYPT_FMP_AEAD_HELPERS is retired"*) ;;
    *) fail "retired FMP AEAD helper diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_DECRYPT_FMP_SOURCE_AFFINE_SESSION_OWNER is retired"*) ;;
    *) fail "retired source-affine diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_DECRYPT_FSP_LOCAL_BULK_OPEN_WORKER is retired"*) ;;
    *) fail "retired FSP local worker-open diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_DECRYPT_FSP_REMOTE_BULK_OPEN_WORKER is retired"*) ;;
    *) fail "retired FSP remote worker-open diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_DECRYPT_FSP_OPEN_WORKER_MAX_COMPLETION_BACKLOG is retired"*) ;;
    *) fail "retired FSP worker-open backlog diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_DECRYPT_FSP_AEAD_COMPLETION_BATCH_MAX is retired"*) ;;
    *) fail "retired FSP completion batch diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_LINUX_BULK_UDP_PACE_MBPS is retired"*) ;;
    *) fail "retired Linux bulk pacer diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_MACOS_ORDERED_SENDER is retired"*) ;;
    *) fail "retired macOS ordered sender diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"NVPN_FIPS_LINUX_TUN_VNET is retired"*) ;;
    *) fail "retired Linux vnet diagnostic missing: $output" ;;
  esac
}

test_extra_env_validation_rejects_stale_fips_helper_names() {
  local output status
  set +e
  output="$(
    docker_bench_validate_extra_env_assignments \
      "FIPS_FSP_AEAD_HELPERS=2 FIPS_FSP_AEAD_HELPER_COMPLETIONS=1 FIPS_FMP_AEAD_HELPERS=2 FIPS_DECRYPT_FSP_ORDERED_AEAD_HELPERS=2 FIPS_DECRYPT_FMP_PREOWNER_AEAD_HELPERS=1" \
      2>&1
  )"
  status=$?
  set -e

  [[ "$status" != "0" ]] || fail "stale FIPS helper env names were accepted"
  case "$output" in
    *"FIPS_FSP_AEAD_HELPERS is not read by current FIPS"*) ;;
    *) fail "stale FSP helper diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_DECRYPT_FSP_ORDERED_AEAD_HELPERS is retired"*) ;;
    *) fail "retired FSP helper diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_FSP_AEAD_HELPER_COMPLETIONS is not read by current FIPS"*) ;;
    *) fail "stale FSP helper completion diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_FMP_AEAD_HELPERS is retired"*) ;;
    *) fail "stale FMP helper diagnostic missing: $output" ;;
  esac
  case "$output" in
    *"FIPS_DECRYPT_FMP_PREOWNER_AEAD_HELPERS was removed"*) ;;
    *) fail "removed FMP preowner diagnostic missing: $output" ;;
  esac
}

test_pipeline_summary_helpers() {
  local dir lines all after load peak top worker_open worker_top fmp decrypt_spread linux_bulk udp tun mesh_send mesh_recv tun_write direct_endpoint hard
  local placement helper_other handoff_other
  dir="$(mktemp -d)"
  lines="$dir/pipeline.txt"
  cat >"$lines" <<'EOF'
[pipe 5s] fmp_worker_batch_flush=2/s fmp_worker_batch_packets=20/s udp_send_connected=20/s endpoint_event_wait=2/s avg=40.0us p50<=32.8us p95<=262.1us p99<=524.3us max<=1.0ms allmax=5.7ms
[pipe 5s] fmp_worker_batch_flush=10/s fmp_worker_batch_packets=420/s fmp_worker_batch_priority_packets=105/s fmp_worker_batch_bulk_packets=315/s fmp_worker_batch_full=9/s fmp_worker_batch_single=0.5/s fmp_send_group=12/s fmp_send_group_packets=420/s fmp_send_group_single=3/s decrypt_worker_batch_bulk_packets=315/s decrypt_worker_batch_worker0=210/s decrypt_worker_batch_worker2=105/s decrypt_fsp_owner_mismatch=420/s decrypt_fsp_path_helper=400/s decrypt_fsp_path_helper_bulk=400/s decrypt_fsp_path_handoff=20/s decrypt_fsp_path_handoff_priority=20/s decrypt_fsp_path_handoff_bulk=0/s udp_send_gso_batch=10/s udp_send_gso_packets=420/s udp_send_gso_batch_ge32=7/s udp_send_gso_batch_ge48=3/s udp_send_gso_batch_eq64=1/s udp_send_sendmmsg_batch=2/s udp_send_sendmmsg_packets=4/s udp_send_sendmmsg_batch_ge32=1/s udp_send_sendmmsg_batch_ge48=0/s udp_send_sendmmsg_batch_eq64=0/s fmp_worker_bulk_queue_wait=10/s avg=1.1ms p50<=2.1ms p95<=2.1ms p99<=8.4ms max<=16.8ms allmax=11.1ms fsp_aead_worker_open_queue_wait=10/s avg=500.0us p50<=524.3us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=4.2ms fsp_aead_worker_open_completion_wait=10/s avg=800.0us p50<=1.0ms p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms fmp_linux_bulk_container_enqueued=12/s total=60 fmp_linux_bulk_container_packets=420/s total=2100 fmp_linux_bulk_container_sent=12/s total=60 fmp_linux_bulk_container_sent_packets=420/s total=2100 fmp_linux_bulk_container_queue_wait=12/s avg=1.0ms p50<=2.1ms p95<=2.1ms p99<=8.4ms max<=16.8ms allmax=11.1ms fmp_linux_bulk_container_ready_wait=10/s avg=2.0ms p50<=2.1ms p95<=4.2ms p99<=16.8ms max<=33.6ms allmax=33.6ms fmp_linux_bulk_container_first_slot_wait=12/s avg=40.0us p50<=32.8us p95<=131.1us p99<=262.1us max<=1.0ms allmax=1.2ms fmp_linux_bulk_container_all_slots_wait=12/s avg=260.0us p50<=262.1us p95<=524.3us p99<=1.0ms max<=2.1ms allmax=1.9ms connected_udp_kernel_dropped=0.3/s total=3 connected_udp_drain_bulk_dropped=0.4/s total=4 encrypt_worker_bulk_queue_full=2/s total=10 decrypt_fsp_worker_replay_dropped_too_old=0.6/s total=3 decrypt_fsp_worker_replay_dropped_too_old_lag_ge_2x_window=0.2/s total=1 fmp_aead_completion_aead_failed=0.2/s total=1 fsp_aead_completion_aead_failed=0.4/s total=2 fsp_aead_completion_epoch_mismatch=0.6/s total=6 fsp_aead_completion_stale_order=0.2/s total=4 fsp_aead_completion_duplicate_ticket=0.2/s total=1 fsp_aead_completion_replay_dropped_worker_open=1/s total=5 fsp_aead_completion_replay_dropped_duplicate=1/s total=5 fsp_aead_completion_replay_dropped_too_old=0.2/s total=1 fsp_aead_completion_replay_dropped_too_old_lag_ge_2x_window=0.2/s total=1 fsp_aead_completion_replay_dropped_too_old_lag_ge_4x_window=0.2/s total=1 dataplane_live_retired_drops=7/s total=21 dataplane_live_output_drops=2/s total=4 dataplane_drop_stale_completion_generation=3/s total=9 | [nvpn-pipe 5s] nvpn_tun_read=300/s nvpn_mesh_send=300/s nvpn_tun_to_mesh_queue_wait=10/s avg=31.0us p50<=32.8us p95<=131.1us p99<=524.3us max<=1.0ms allmax=2.5ms nvpn_tun_read_batch_flush=12/s total=60 nvpn_tun_read_batch_packets=300/s total=1500 nvpn_tun_read_batch_full=3/s total=15 nvpn_tun_read_batch_single=1/s total=5 nvpn_tun_read_packet_bytes=360000/s total=1800000 nvpn_tun_read_vnet_gso_frames=20/s total=100 nvpn_tun_read_vnet_gso_segments=260/s total=1300 nvpn_tun_read_vnet_gso_segment_bytes=338000/s total=1690000 nvpn_mesh_send_batch_flush=12/s total=60 nvpn_mesh_send_batch_input_packets=300/s total=1500 nvpn_mesh_send_batch_routed_packets=285/s total=1425 nvpn_mesh_send_batch_runs=15/s total=75 nvpn_mesh_send_batch_full=3/s total=15 nvpn_mesh_recv_batch_flush=6/s total=30 nvpn_mesh_recv_batch_events=300/s total=1500 nvpn_mesh_recv_batch_packets=240/s total=1200 nvpn_mesh_recv_packet_bytes=288000/s total=1440000 nvpn_mesh_recv_batch_full=2/s total=10 nvpn_mesh_recv_batch_single_packet=1/s total=5 nvpn_tun_write_packets=240/s total=1200 nvpn_tun_write_packet_bytes=288000/s total=1440000 nvpn_tun_write_would_block=3/s total=15
[nvpn-pipe 5s] nvpn_tun_read=1/s nvpn_mesh_send=1/s nvpn_tun_to_mesh_queue_wait=1/s avg=9.0ms p50<=8.4ms p95<=16.8ms p99<=33.6ms max<=67.1ms allmax=67.1ms
EOF

  after="$(docker_bench_pipeline_lines_after_start_from_stdin 1 <"$lines" | wc -l | tr -d ' ')"
  load="$(docker_bench_load_pipeline_line_from_stdin <"$lines")"
  peak="$(docker_bench_peak_wait_pipeline_line_from_stdin <"$lines")"
  top="$(docker_bench_pipeline_queue_wait_top_summary "$load")"
  worker_open="$(docker_bench_pipeline_fsp_worker_open_wait_summary "$load")"
  worker_top="$(docker_bench_pipeline_queue_wait_top_summary '[pipe 5s] fsp_aead_worker_open_completion_wait=5/s avg=7.0ms p50<=8.4ms p95<=16.8ms p99<=33.6ms max<=67.1ms allmax=67.1ms')"
  fmp="$(docker_bench_pipeline_fmp_worker_batch_summary "$load")"
  decrypt_spread="$(docker_bench_pipeline_decrypt_worker_spread_summary "$load")"
  linux_bulk="$(docker_bench_pipeline_linux_bulk_container_summary "$load")"
  udp="$(docker_bench_pipeline_udp_send_batch_summary "$load")"
  tun="$(docker_bench_pipeline_nvpn_tun_read_batch_summary "$load")"
  mesh_send="$(docker_bench_pipeline_nvpn_mesh_send_batch_summary "$load")"
  mesh_recv="$(docker_bench_pipeline_nvpn_mesh_recv_batch_summary "$load")"
  tun_write="$(docker_bench_pipeline_nvpn_tun_write_summary "$load")"
  direct_endpoint="$(docker_bench_pipeline_nvpn_direct_endpoint_summary '[nvpn-pipe 5s] nvpn_direct_endpoint_queue=6/s avg=700.0us p50<=1.0ms p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=16.8ms nvpn_direct_endpoint_wake=5/s avg=80.0us p50<=65.5us p95<=262.1us p99<=524.3us max<=1.0ms allmax=2.1ms nvpn_direct_endpoint_backlog=3/s avg=300.0us p50<=262.1us p95<=524.3us p99<=1.0ms max<=2.1ms allmax=4.2ms nvpn_direct_endpoint_consumer_busy=2/s avg=400.0us p50<=524.3us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=8.4ms nvpn_direct_endpoint_recv=6/s avg=20.0us p50<=16.4us p95<=65.5us p99<=131.1us max<=262.1us allmax=524.3us nvpn_direct_endpoint_finalize=6/s avg=512ns p50<=512ns p95<=1.0us p99<=2.0us max<=4.1us allmax=8.2us nvpn_tun_write_batch=6/s avg=500.0us p50<=524.3us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=8.4ms nvpn_direct_endpoint_rx_limit_splits=4/s total=20 nvpn_direct_endpoint_rx_limit_tail_packets=512/s total=2560')"
  placement="$(docker_bench_pipeline_fsp_owner_placement_summary "$load")"
  helper_other="$(docker_bench_pipeline_fsp_owner_placement_other_path_max "$load" helper)"
  handoff_other="$(docker_bench_pipeline_fsp_owner_placement_other_path_max "$load" handoff)"
  hard="$(docker_bench_pipeline_hard_event_summary_from_stdin 0 <"$lines")"

  assert_eq "$after" "2" "pipeline lines after start"
  case "$load" in
    *"fmp_worker_batch_packets=420/s"*) ;;
    *) fail "load pipeline selector did not choose packet-bearing line: $load" ;;
  esac
  case "$peak" in
    *"nvpn_tun_to_mesh_queue_wait=1/s avg=9.0ms"*) ;;
    *) fail "peak pipeline selector did not choose highest wait line: $peak" ;;
  esac
  assert_eq "$top" "fmp_linux_bulk_container_ready_wait:rate_per_sec=10,p95_ms=4.2,p99_ms=16.8,max_ms=33.6,allmax_ms=33.6" "pipeline top queue wait"
  assert_eq "$worker_open" "queue_rate_per_sec=10,queue_p95_ms=1,queue_p99_ms=2.1,queue_max_ms=4.2,queue_allmax_ms=4.2,completion_rate_per_sec=10,completion_p95_ms=2.1,completion_p99_ms=4.2,completion_max_ms=8.4,completion_allmax_ms=8.4" "FSP worker-open wait summary"
  assert_eq "$worker_top" "fsp_aead_worker_open_completion_wait:rate_per_sec=5,p95_ms=16.8,p99_ms=33.6,max_ms=67.1,allmax_ms=67.1" "worker-open waits can be top queue wait"
  assert_eq "$fmp" "avg_packets=42.0,full_pct=90.0,single_pct=5.0,priority_pct=25.0,bulk_pct=75.0,flush_per_sec=10,packets_per_sec=420,priority_packets_per_sec=105,bulk_packets_per_sec=315,send_groups_per_flush=1.2,send_group_avg_packets=35.0,send_group_single_pct=25.0,send_groups_per_sec=12,send_group_packets_per_sec=420" "FMP worker batch summary"
  assert_eq "$decrypt_spread" "active_workers=2,workers_ge1pct=2,top_worker=w0,top_pct=66.7,total_packets_per_sec=315,worker_packet_rates=w0:210;w2:105" "decrypt worker spread summary"
  assert_eq "$linux_bulk" "avg_packets=35.0,avg_sent_packets=35.0,enqueued_per_sec=12,packets_per_sec=420,sent_per_sec=12,sent_packets_per_sec=420,queue_p95_ms=2.1,queue_p99_ms=8.4,ready_p95_ms=4.2,ready_p99_ms=16.8,first_slot_p95_ms=0.1311,first_slot_p99_ms=0.2621,all_slots_p95_ms=0.5243,all_slots_p99_ms=1" "Linux bulk container summary"
  assert_eq "$udp" "gso_packet_pct=99.1,sendmmsg_packet_pct=0.9,avg_packets=35.3,gso_avg_packets=42.0,sendmmsg_avg_packets=2.0,gso_ge32_pct=70.0,gso_ge48_pct=30.0,gso_eq64_pct=10.0,sendmmsg_ge32_pct=50.0,sendmmsg_ge48_pct=0.0,sendmmsg_eq64_pct=0.0,gso_batch_per_sec=10,gso_packets_per_sec=420,sendmmsg_batch_per_sec=2,sendmmsg_packets_per_sec=4,total_packets_per_sec=424" "UDP send batch summary"
  assert_eq "$tun" "avg_packets=25.0,full_pct=25.0,single_pct=8.3,avg_packet_bytes=1200.0,flush_per_sec=12,packets_per_sec=300,bytes_per_sec=360000,vnet_gso_frames_per_sec=20,vnet_gso_segments_per_sec=260,vnet_gso_avg_segments=13.0,vnet_gso_avg_segment_bytes=1300.0" "nvpn TUN read batch summary"
  assert_eq "$mesh_send" "avg_input_packets=25.0,avg_routed_packets=23.8,avg_runs=1.2,routed_pct=95.0,full_pct=25.0,flush_per_sec=12,input_packets_per_sec=300,routed_packets_per_sec=285,runs_per_sec=15" "nvpn mesh send batch summary"
  assert_eq "$mesh_recv" "avg_events=50.0,avg_packets=40.0,full_pct=33.3,single_packet_pct=16.7,avg_packet_bytes=1200.0,flush_per_sec=6,events_per_sec=300,packets_per_sec=240,bytes_per_sec=288000" "nvpn mesh receive batch summary"
  assert_eq "$tun_write" "packets_per_sec=240,bytes_per_sec=288000,avg_packet_bytes=1200.0,would_block_per_sec=3" "nvpn TUN write summary"
  assert_eq "$direct_endpoint" "queue_rate_per_sec=6,queue_avg_ms=0.7,queue_p95_ms=2.1,queue_p99_ms=4.2,queue_max_ms=8.4,queue_allmax_ms=16.8,wake_rate_per_sec=5,wake_avg_ms=0.08,wake_p95_ms=0.2621,wake_p99_ms=0.5243,wake_max_ms=1,wake_allmax_ms=2.1,backlog_rate_per_sec=3,backlog_avg_ms=0.3,backlog_p95_ms=0.5243,backlog_p99_ms=1,backlog_max_ms=2.1,backlog_allmax_ms=4.2,consumer_busy_rate_per_sec=2,consumer_busy_avg_ms=0.4,consumer_busy_p95_ms=1,consumer_busy_p99_ms=2.1,consumer_busy_max_ms=4.2,consumer_busy_allmax_ms=8.4,recv_rate_per_sec=6,recv_avg_ms=0.02,recv_p95_ms=0.0655,recv_p99_ms=0.1311,recv_max_ms=0.2621,recv_allmax_ms=0.5243,finalize_rate_per_sec=6,finalize_avg_ms=0.000512,finalize_p95_ms=0.001,finalize_p99_ms=0.002,finalize_max_ms=0.0041,finalize_allmax_ms=0.0082,tun_batch_rate_per_sec=6,tun_batch_avg_ms=0.5,tun_batch_p95_ms=1,tun_batch_p99_ms=2.1,tun_batch_max_ms=4.2,tun_batch_allmax_ms=8.4,limit_splits_per_sec=4,limit_splits_total=20,limit_tail_packets_per_sec=512,limit_tail_packets_total=2560" "nvpn direct endpoint turn summary"
  assert_eq "$placement" "owner=mismatch,path=helper,bulk_path=helper,priority_path=handoff,owner_same_per_sec=0,owner_mismatch_per_sec=420,path_local_per_sec=0,path_handoff_per_sec=20,path_helper_per_sec=400,path_worker_open_per_sec=0,path_worker_open_striped_per_sec=0,path_local_priority_per_sec=0,path_local_bulk_per_sec=0,path_handoff_priority_per_sec=20,path_handoff_bulk_per_sec=0,path_helper_bulk_per_sec=400,path_worker_open_bulk_per_sec=0,bulk_packets_per_sec=315,select_bulk_packets_per_sec=0,drain_bulk_packets_per_sec=0" "FSP owner placement summary"
  assert_eq "$helper_other" $'\t0' "helper alternate placement path ignores priority handoff"
  assert_eq "$handoff_other" $'helper\t400' "handoff alternate placement path"
  assert_eq "$hard" "connected_udp_kernel_dropped:max_rate_per_sec=0.3,total=3;connected_udp_drain_bulk_dropped:max_rate_per_sec=0.4,total=4;encrypt_worker_bulk_queue_full:max_rate_per_sec=2,total=10;decrypt_fsp_worker_replay_dropped_too_old:max_rate_per_sec=0.6,total=3;decrypt_fsp_worker_replay_dropped_too_old_lag_ge_2x_window:max_rate_per_sec=0.2,total=1;fmp_aead_completion_aead_failed:max_rate_per_sec=0.2,total=1;fsp_aead_completion_aead_failed:max_rate_per_sec=0.4,total=2;fsp_aead_completion_epoch_mismatch:max_rate_per_sec=0.6,total=6;fsp_aead_completion_stale_order:max_rate_per_sec=0.2,total=4;fsp_aead_completion_duplicate_ticket:max_rate_per_sec=0.2,total=1;fsp_aead_completion_replay_dropped_worker_open:max_rate_per_sec=1,total=5;fsp_aead_completion_replay_dropped_duplicate:max_rate_per_sec=1,total=5;fsp_aead_completion_replay_dropped_too_old:max_rate_per_sec=0.2,total=1;fsp_aead_completion_replay_dropped_too_old_lag_ge_2x_window:max_rate_per_sec=0.2,total=1;fsp_aead_completion_replay_dropped_too_old_lag_ge_4x_window:max_rate_per_sec=0.2,total=1;dataplane_live_retired_drops:max_rate_per_sec=7,total=21;dataplane_live_output_drops:max_rate_per_sec=2,total=4;dataplane_drop_stale_completion_generation:max_rate_per_sec=3,total=9" "pipeline hard event summary"

  rm -rf "$dir"
}

test_nvpn_tun_write_summary_prefers_coalesced_frame_interval() {
  local dir lines summary
  dir="$(mktemp -d)"
  lines="$dir/nvpn-pipe.txt"
  cat >"$lines" <<'EOF'
[nvpn-pipe 1s] nvpn_tun_write_packets=1000/s total=1000 nvpn_tun_write_packet_bytes=1150000/s total=1150000 nvpn_tun_write_frames=1000/s total=1000 nvpn_tun_write_frame_bytes=1160000/s total=1160000
[nvpn-pipe 1s] nvpn_tun_write_packets=330000/s total=331000 nvpn_tun_write_packet_bytes=379500000/s total=380650000 nvpn_tun_write_frames=10000/s total=11000 nvpn_tun_write_frame_bytes=380000000/s total=381160000
[nvpn-pipe 1s] nvpn_tun_write_packets=90000/s total=421000 nvpn_tun_write_packet_bytes=4680000/s total=385330000 nvpn_tun_write_frames=90000/s total=101000 nvpn_tun_write_frame_bytes=5580000/s total=386740000
EOF

  summary="$(docker_bench_pipeline_nvpn_tun_write_summary_from_stdin <"$lines")"
  assert_eq "$summary" "packets_per_sec=330000,bytes_per_sec=3.795e+08,avg_packet_bytes=1150.0,frames_per_sec=10000,avg_packets_per_frame=33.0,avg_frame_bytes=38000.0,would_block_per_sec=0" "nvpn TUN write coalesced interval summary"

  rm -rf "$dir"
}

write_summary_fixture() {
  local path="$1"
  local backend="$2"
  local threads="$3"
  local tcp_single="$4"
  local tcp_single_retrans="$5"
  local tcp_4="$6"
  local tcp_4_retrans="$7"
  local tcp_8="$8"
  local tcp_8_retrans="$9"
  local udp_200="${10}"
  local udp_200_loss="${11}"
  local udp_1000="${12}"
  local udp_1000_loss="${13}"
  local ping_loss="${14}"
  local ping_avg="${15}"
  local raw_dir="${16}"
  local ping_mdev="${17:-0.1}"
  local ping_p95="${18:-0.9}"
  local ping_p99="${19:-1.2}"
  local ping_max="${20:-1.5}"

  mkdir -p "$(dirname "$path")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    backend threads duration_secs \
    tcp_single_mbps tcp_single_retrans \
    tcp_4_mbps tcp_4_retrans \
    tcp_8_mbps tcp_8_retrans \
    udp_200_mbps udp_200_loss_pct \
    udp_1000_mbps udp_1000_loss_pct \
    ping_loss_pct ping_avg_ms \
    ping_mdev_ms ping_p95_ms ping_p99_ms ping_max_ms raw_dir >"$path"
  printf '%s\t%s\t3\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$backend" "$threads" \
    "$tcp_single" "$tcp_single_retrans" \
    "$tcp_4" "$tcp_4_retrans" \
    "$tcp_8" "$tcp_8_retrans" \
    "$udp_200" "$udp_200_loss" \
    "$udp_1000" "$udp_1000_loss" \
    "$ping_loss" "$ping_avg" \
    "$ping_mdev" "$ping_p95" "$ping_p99" "$ping_max" "$raw_dir" >>"$path"
}

write_cpu_phases_fixture() {
  local raw_dir="$1"
  local backend="$2"
  local tcp_single="$3"
  local tcp_4="$4"
  local tcp_8="$5"
  local udp_200="$6"
  local udp_1000="$7"
  mkdir -p "$raw_dir"
  cat >"$raw_dir/${backend}-cpu-phases.tsv" <<EOF
phase	service	pid_start	pid_end	cpu_jiffies_start	cpu_jiffies_end	clk_tck	cpu_seconds	transfer_bytes	cpu_seconds_per_gbyte
tcp-single	both	na	na	na	na	na	na	na	$tcp_single
tcp-4	both	na	na	na	na	na	na	na	$tcp_4
tcp-8	both	na	na	na	na	na	na	na	$tcp_8
udp-200	both	na	na	na	na	na	na	na	$udp_200
udp-1000	both	na	na	na	na	na	na	na	$udp_1000
EOF
}

write_metadata_fixture() {
  local dir="$1"
  local backend="$2"
  local enabled="$3"
  local sides="$4"
  local local_workers="$5"
  local remote_workers="$6"
  local pipeline_trace_enabled="${7:-false}"
  local pipeline_trace_interval_secs="${8:-}"
  local extra_connect_env="${9:-}"
  local local_fips_patch_enabled="${10:-false}"
  local nvpn_git_head="${11:-}"
  local nvpn_git_dirty="${12:-false}"
  local fips_git_head="${13:-}"
  local fips_git_dirty="${14:-false}"
  local iperf_socket_buffer="${15:-}"
  local iperf_udp1000_parallel="${16:-}"
  local iperf_udp1000_bandwidth="${17:-1G}"
  local iperf_udp1000_per_stream_bandwidth="${18:-}"
  mkdir -p "$dir"
  jq -n \
    --arg backend "$backend" \
    --arg enabled "$enabled" \
    --arg sides "$sides" \
    --arg local_workers "$local_workers" \
    --arg remote_workers "$remote_workers" \
    --arg pipeline_trace_enabled "$pipeline_trace_enabled" \
    --arg pipeline_trace_interval_secs "$pipeline_trace_interval_secs" \
    --arg extra_connect_env "$extra_connect_env" \
    --arg local_fips_patch_enabled "$local_fips_patch_enabled" \
    --arg nvpn_git_head "$nvpn_git_head" \
    --arg nvpn_git_dirty "$nvpn_git_dirty" \
    --arg fips_git_head "$fips_git_head" \
    --arg fips_git_dirty "$fips_git_dirty" \
    --arg iperf_socket_buffer "$iperf_socket_buffer" \
    --arg iperf_udp1000_parallel "$iperf_udp1000_parallel" \
    --arg iperf_udp1000_bandwidth "$iperf_udp1000_bandwidth" \
    --arg iperf_udp1000_per_stream_bandwidth "$iperf_udp1000_per_stream_bandwidth" \
    '{
      backend: $backend,
      run_env: {
        extra_connect_env: $extra_connect_env
      },
      cpu_stress: {
        enabled: ($enabled == "true"),
        sides: $sides,
        local_workers: ($local_workers | tonumber),
        remote_workers: ($remote_workers | tonumber)
      },
      pipeline_trace: {
        enabled: ($pipeline_trace_enabled == "true"),
        interval_secs: (
          if $pipeline_trace_interval_secs == "" then null
          else ($pipeline_trace_interval_secs | tonumber)
          end
        )
      },
      iperf: {
        socket_buffer: (if $iperf_socket_buffer == "" then null else $iperf_socket_buffer end),
        udp1000_parallel: (
          if $iperf_udp1000_parallel == "" then null
          else ($iperf_udp1000_parallel | tonumber)
          end
        ),
        udp1000_bandwidth: (if $iperf_udp1000_bandwidth == "" then null else $iperf_udp1000_bandwidth end),
        udp1000_per_stream_bandwidth: (if $iperf_udp1000_per_stream_bandwidth == "" then null else $iperf_udp1000_per_stream_bandwidth end)
      },
      source: {
        nvpn: {
          git_head: (if $nvpn_git_head == "" then null else $nvpn_git_head end),
          dirty: ($nvpn_git_dirty == "true")
        },
        local_fips_patch: {
          enabled: ($local_fips_patch_enabled == "true"),
          git_head: (if $fips_git_head == "" then null else $fips_git_head end),
          dirty: ($fips_git_dirty == "true")
        }
      }
    }' >"$dir/metadata.json"
}

test_docker_comparison_outputs() {
  local dir out comparison_fields ratio_fields threshold_fields tcp_ratio ping_delta ping_p99_delta json_metric
  local threshold_tcp_4 threshold_udp1000_zero threshold_ping_p99 threshold_status threshold_failures effective_udp_delta enforce_output stress_fields stress_json
  local pipeline_fields pipeline_json provenance_json iperf_json cpu_fields cpu_ratio cpu_json_metric
  dir="$(mktemp -d)"
  write_summary_fixture \
    "$dir/nvpn/summary.tsv" \
    nvpn "" \
    300 10 \
    400 20 \
    500 30 \
    199 0.5 \
    990 1 \
    0 0.8 \
    "$dir/nvpn/raw"
  write_cpu_phases_fixture "$dir/nvpn/raw" nvpn 6 7 8 40 50
  write_metadata_fixture "$dir/nvpn" nvpn true remote 0 4 true 2 "FIPS_LINUX_BULK_CONTAINERS=1" true nvpnabc false fipsabc true 208K 1 1G 1G
  write_summary_fixture \
    "$dir/reference/summary.tsv" \
    boringtun 1 \
    250 5 \
    500 25 \
    400 40 \
    200 0.25 \
    980 2 \
    0 0.4 \
    "$dir/reference/raw"
  write_cpu_phases_fixture "$dir/reference/raw" boringtun 3 4 5 80 100
  write_metadata_fixture "$dir/reference" boringtun false both 0 0 false "" "" false "" false "" false 208K 1 1G 1G
  out="$dir/out"

  "$COMPARE_SCRIPT" "$dir/nvpn" "$dir/reference" "$out" >/dev/null

  comparison_fields="$(awk -F '\t' 'NR == 2 { print NF }' "$out/comparison.tsv")"
  ratio_fields="$(awk -F '\t' 'NR == 2 { print NF }' "$out/ratios.tsv")"
  threshold_fields="$(awk -F '\t' 'NR == 2 { print NF }' "$out/thresholds.tsv")"
  tcp_ratio="$(awk -F '\t' '$1 == "tcp_single_mbps" { print $6 "\t" $7 }' "$out/ratios.tsv")"
  ping_delta="$(awk -F '\t' '$1 == "ping_avg_ms" { print $6 "\t" $7 }' "$out/ratios.tsv")"
  ping_p99_delta="$(awk -F '\t' '$1 == "ping_p99_ms" { print $3 "\t" $6 "\t" $7 }' "$out/ratios.tsv")"
  cpu_fields="$(awk -F '\t' '$1 == "nvpn" { print $28 "\t" $29 "\t" $30 "\t" $31 "\t" $32 }' "$out/comparison.tsv")"
  cpu_ratio="$(awk -F '\t' '$1 == "tcp_8_cpu_sec_per_gb" { print $3 "\t" $6 "\t" $7 }' "$out/ratios.tsv")"
  json_metric="$(jq -r '.ratios[] | select(.metric == "udp_1000_loss_pct") | .better_when + "\t" + .nvpn_minus_reference' "$out/comparison.json")"
  cpu_json_metric="$(jq -r '.ratios[] | select(.metric == "udp_1000_cpu_sec_per_gb") | .better_when + "\t" + .nvpn_percent_of_reference + "\t" + .nvpn_minus_reference' "$out/comparison.json")"
  threshold_tcp_4="$(awk -F '\t' '$1 == "tcp_4_throughput" { print $3 "\t" $6 "\t" $7 }' "$out/thresholds.tsv")"
  threshold_udp1000_zero="$(awk -F '\t' '$1 == "nvpn_udp_1000_zero_loss" { print $3 "\t" $4 "\t" $6 }' "$out/thresholds.tsv")"
  threshold_ping_p99="$(awk -F '\t' '$1 == "ping_p99" { print $3 "\t" $6 "\t" $7 }' "$out/thresholds.tsv")"
  threshold_status="$(jq -r '.threshold_status.status' "$out/comparison.json")"
  threshold_failures="$(jq -r '.threshold_status.failures' "$out/comparison.json")"
  effective_udp_delta="$(jq -r '.threshold_policy.effective_udp_loss_delta_pct' "$out/comparison.json")"
  stress_fields="$(awk -F '\t' '$1 == "nvpn" { print $6 "\t" $7 "\t" $8 "\t" $9 }' "$out/comparison.tsv")"
  stress_json="$(jq -r '.cpu_stress.nvpn.enabled, .cpu_stress.nvpn.remote_workers, .cpu_stress.reference.enabled' "$out/comparison.json" | paste -sd ':' -)"
  pipeline_fields="$(awk -F '\t' '$1 == "nvpn" { print $10 "\t" $11 }' "$out/comparison.tsv")"
  pipeline_json="$(jq -r '.pipeline_trace.mismatch, .pipeline_trace.nvpn.enabled, .pipeline_trace.nvpn.interval_secs, .pipeline_trace.reference.enabled' "$out/comparison.json" | paste -sd ':' -)"
  provenance_json="$(jq -r '.provenance.nvpn.run_env.extra_connect_env, .provenance.nvpn.local_fips_patch.enabled, .provenance.nvpn.local_fips_patch.git_head, .provenance.nvpn.local_fips_patch.dirty' "$out/comparison.json" | paste -sd ':' -)"
  iperf_json="$(jq -r '.iperf.mismatch, .iperf.nvpn.socket_buffer, .iperf.reference.socket_buffer, .iperf.nvpn.udp1000_bandwidth, .iperf.nvpn.udp1000_per_stream_bandwidth' "$out/comparison.json" | paste -sd ':' -)"

  assert_eq "$comparison_fields" "33" "Docker comparison field count"
  assert_eq "$ratio_fields" "7" "Docker ratio field count"
  assert_eq "$threshold_fields" "7" "Docker threshold field count"
  assert_eq "$tcp_ratio" $'120.0\t50.000' "Docker TCP single ratio"
  assert_eq "$ping_delta" $'200.0\t0.400' "Docker ping avg delta"
  assert_eq "$ping_p99_delta" $'lower\t100.0\t0.000' "Docker ping p99 ratio"
  assert_eq "$cpu_fields" $'6\t7\t8\t40\t50' "Docker comparison CPU columns"
  assert_eq "$cpu_ratio" $'lower\t160.0\t3.000' "Docker TCP8 CPU ratio"
  assert_eq "$json_metric" $'lower\t-1.000' "Docker comparison JSON ratio"
  assert_eq "$cpu_json_metric" $'lower\t50.0\t-50.000' "Docker comparison JSON CPU ratio"
  assert_eq "$threshold_tcp_4" $'fail\t>=90%\t80.0%' "Docker throughput threshold"
  assert_eq "$threshold_udp1000_zero" $'fail\t1\t==0' "Docker UDP1000 zero-loss candidate threshold"
  assert_eq "$threshold_ping_p99" $'pass\t<=reference+2ms\t0.000ms' "Docker ping p99 threshold"
  assert_eq "$threshold_status" "fail" "Docker threshold JSON status"
  assert_eq "$threshold_failures" "4" "Docker threshold JSON failure count"
  assert_eq "$effective_udp_delta" "1" "Docker clean/default UDP loss threshold"
  assert_eq "$stress_fields" $'true\tremote\t0\t4' "Docker comparison stress columns"
  assert_eq "$stress_json" "true:4:false" "Docker comparison stress JSON"
  assert_eq "$pipeline_fields" $'true\t2' "Docker comparison pipeline columns"
  assert_eq "$pipeline_json" "true:true:2:false" "Docker comparison pipeline JSON"
  assert_eq "$provenance_json" "FIPS_LINUX_BULK_CONTAINERS=1:true:fipsabc:true" "Docker comparison provenance JSON"
  assert_eq "$iperf_json" "false:208K:208K:1G:1G" "Docker comparison iperf JSON"

  if NVPN_DOCKER_COMPARISON_ENFORCE_THRESHOLDS=1 "$COMPARE_SCRIPT" "$dir/nvpn" "$dir/reference" "$dir/enforced" >"$dir/enforced.stdout" 2>"$dir/enforced.stderr"; then
    fail "Docker comparison enforcement should fail on threshold violations"
  fi
  enforce_output="$(cat "$dir/enforced.stderr")"
  case "$enforce_output" in
    *"threshold status is fail"*) ;;
    *) fail "Docker comparison enforcement stderr did not explain threshold failure: $enforce_output" ;;
  esac

  rm -rf "$dir"
}

test_docker_benchmark_table_outputs() {
  local dir out fields current_status current_events current_socket published_status published_events legacy_status wg_status
  local current_receiver published_receiver current_provenance current_pipeline_shape markdown_row
  dir="$(mktemp -d)"
  write_summary_fixture \
    "$dir/current/summary.tsv" \
    nvpn "" \
    3000 10 \
    2800 20 \
    2600 30 \
    200 0 \
    1000 0 \
    0 0.3 \
    "$dir/current/raw"
  write_metadata_fixture "$dir/current" nvpn true both 1 1 false "" "" true nvpnabc false fipsabc false
  mkdir -p "$dir/current/raw"
  {
    printf 'event\tmax_rate_per_sec\ttotal\n'
    printf 'rx_loop_slow_maintenance_skipped\t1\t7\n'
  } >"$dir/current/raw/nvpn-pipeline-hard-event-totals.tsv"
  {
    printf 'phase\tservice\tload_top_queue_wait\tpeak_top_queue_wait\tnvpn_tun_read_batch\tnvpn_mesh_send_batch\tnvpn_mesh_recv_batch\tnvpn_tun_write\tnvpn_direct_endpoint\thard_events\n'
    printf 'tcp-8\tnode-a\tnvpn_direct_endpoint_queue:rate_per_sec=11,p95_ms=0.2,p99_ms=0.5,max_ms=1,allmax_ms=2\tnvpn_direct_endpoint_queue:rate_per_sec=12,p95_ms=0.2,p99_ms=1,max_ms=2,allmax_ms=3\tavg_packets=120.0,flush_per_sec=10,packets_per_sec=1200\tavg_input_packets=120.0,avg_routed_packets=120.0,avg_runs=1.0\tavg_events=1.0,avg_packets=1.0\tpackets_per_sec=1\tqueue_rate_per_sec=1\t\n'
    printf 'tcp-8\tnode-b\tnvpn_direct_endpoint_queue:rate_per_sec=21,p95_ms=1,p99_ms=2,max_ms=4,allmax_ms=8\tnvpn_direct_endpoint_queue:rate_per_sec=22,p95_ms=2,p99_ms=4,max_ms=8,allmax_ms=16\tavg_packets=1.0\tavg_input_packets=1.0\tavg_events=100.0,avg_packets=100.0,flush_per_sec=8\tpackets_per_sec=1000,frames_per_sec=30,avg_packets_per_frame=33.3\tqueue_rate_per_sec=21,queue_p95_ms=1,queue_p99_ms=2,tun_batch_p99_ms=1\t\n'
  } >"$dir/current/raw/nvpn-pipeline-phase-summary.tsv"
  {
    printf 'service\tpeer_addr\trequested_recv_buf\tactual_recv_buf\trequested_send_buf\tactual_send_buf\n'
    printf 'node-a\t10.0.0.2:51820\t16777216\t33554432\t8388608\t16777216\n'
    printf 'node-b\t10.0.0.1:51820\t16777216\t33554432\t8388608\t16777216\n'
  } >"$dir/current/raw/nvpn-connected-udp-socket-buffers.tsv"
  {
    printf 'phase\tprotocol\tstreams\trequested_sock_bufsize\tactual_recv_buf\tactual_send_buf\n'
    printf 'udp-200\tUDP\t1\t0\t212992\t212992\n'
    printf 'udp-1000\tUDP\t4\t4194304\t8388608\t8388608\n'
  } >"$dir/current/raw/nvpn-iperf-socket-buffers.tsv"
  for service in node-a node-b; do
    {
      printf 'net.core.rmem_default\t212992\n'
      printf 'net.core.rmem_max\t212992\n'
      printf 'net.core.wmem_default\t212992\n'
      printf 'net.core.wmem_max\t212992\n'
      printf 'net.ipv4.udp_rmem_min\t4096\n'
      printf 'net.ipv4.udp_wmem_min\t4096\n'
    } >"$dir/current/raw/nvpn-$service-udp-receiver-limits.tsv"
  done
  {
    printf 'UdpInDatagrams                  1000               0.0\n'
    printf 'UdpInErrors                     0                  0.0\n'
    printf 'UdpOutDatagrams                 100                0.0\n'
    printf 'UdpRcvbufErrors                 0                  0.0\n'
  } >"$dir/current/raw/nvpn-node-b-udp-stats.txt"

  write_summary_fixture \
    "$dir/published/summary.tsv" \
    nvpn "" \
    2200 100 \
    2100 200 \
    2000 300 \
    199 0 \
    998 0.2 \
    0 0.5 \
    "$dir/published/raw"
  write_metadata_fixture "$dir/published" nvpn true both 1 1 false "" "" false publishedabc false "" false
  mkdir -p "$dir/published/raw"
  {
    printf 'event\tmax_rate_per_sec\ttotal\n'
    printf 'udp_namespace_rcvbuf_errors\t0.1\t5\n'
    printf 'connected_udp_kernel_dropped\t0.5\t3\n'
    printf 'connected_udp_peer_kernel_dropped\t0.2\t2\n'
  } >"$dir/published/raw/nvpn-pipeline-hard-event-totals.tsv"
  {
    printf 'UdpInDatagrams                  1000               0.0\n'
    printf 'UdpInErrors                     5                  0.0\n'
    printf 'UdpOutDatagrams                 100                0.0\n'
    printf 'UdpRcvbufErrors                 5                  0.0\n'
  } >"$dir/published/raw/nvpn-node-b-udp-stats.txt"

  write_summary_fixture \
    "$dir/legacy/summary.tsv" \
    nvpn "" \
    4300 10 \
    4600 20 \
    4700 30 \
    200 0 \
    1000 0 \
    0 0.4 \
    "$dir/legacy/raw"
  write_metadata_fixture "$dir/legacy" nvpn true both 1 1 false "" "" true legacyabc false fipsabc true
  mkdir -p "$dir/legacy/raw"
  {
    printf 'phase\tservice\thard_events\n'
    printf 'tcp-single\tnode-b\tdecrypt_fallback_pressure_drain:max_rate_per_sec=1,total=11\n'
    printf 'tcp-4\tnode-b\tdecrypt_fallback_pressure_drain:max_rate_per_sec=1,total=4\n'
  } >"$dir/legacy/raw/nvpn-pipeline-phase-summary.tsv"

  write_summary_fixture \
    "$dir/wg/summary.tsv" \
    wireguard-go "" \
    5000 5 \
    7000 6 \
    8000 7 \
    200 0 \
    1000 0 \
    0 0.2 \
    "$dir/wg/raw"
  write_metadata_fixture "$dir/wg" wireguard-go true both 1 1
  out="$dir/out"

  "$TABLE_SCRIPT" \
    --output-dir "$out" \
    current="$dir/current" \
    published="$dir/published" \
    legacy="$dir/legacy" \
    wg="$dir/wg" >/dev/null

  fields="$(awk -F '\t' 'NR == 2 { print NF }' "$out/stress-table.tsv")"
  current_status="$(table_values "$out/stress-table.tsv" current udp_ping_zero hard_events_total hard_events candidate)"
  current_events="$(table_values "$out/stress-table.tsv" current udp_kernel_dropped_total udp_namespace_rcvbuf_errors_total connected_udp_kernel_dropped_total connected_udp_peer_kernel_dropped_total connected_udp_drain_bulk_dropped_total connected_udp_direct_decrypt_bulk_shed_total)"
  current_socket="$(table_values "$out/stress-table.tsv" current connected_udp_recv_buf connected_udp_send_buf)"
  current_receiver="$(table_values "$out/stress-table.tsv" current iperf_udp200_sockbuf iperf_udp1000_sockbuf udp_receiver_rmem udp_receiver_wmem node_b_udp_rcvbuf_errors udp_loss_attribution)"
  current_pipeline_shape="$(table_values "$out/stress-table.tsv" current tcp8_node_a_top_queue_wait tcp8_node_b_top_queue_wait tcp8_node_a_nvpn_tun_read_batch tcp8_node_a_nvpn_mesh_send_batch tcp8_node_b_nvpn_mesh_recv_batch tcp8_node_b_nvpn_tun_write tcp8_node_b_nvpn_direct_endpoint)"
  published_status="$(table_values "$out/stress-table.tsv" published udp_ping_zero hard_events_total hard_events candidate)"
  published_receiver="$(table_values "$out/stress-table.tsv" published node_b_udp_rcvbuf_errors udp_loss_attribution)"
  published_events="$(table_values "$out/stress-table.tsv" published udp_kernel_dropped_total udp_namespace_rcvbuf_errors_total connected_udp_kernel_dropped_total connected_udp_peer_kernel_dropped_total connected_udp_drain_bulk_dropped_total connected_udp_direct_decrypt_bulk_shed_total)"
  legacy_status="$(table_values "$out/stress-table.tsv" legacy udp_ping_zero hard_events_total hard_events candidate)"
  wg_status="$(table_values "$out/stress-table.tsv" wg backend udp_ping_zero hard_events_total candidate)"
  current_provenance="$(table_values "$out/stress-table.tsv" current git_head fips_head dirty stress)"
  markdown_row="$(grep -F '| current | nvpn |' "$out/stress-table.md")"

  assert_eq "$fields" "50" "Docker benchmark table field count"
  assert_eq "$current_status" $'true\t7\trx_loop_slow_maintenance_skipped:7\tpass' "Docker benchmark table current status"
  assert_eq "$current_events" $'0\t0\t0\t0\t0\t0' "Docker benchmark table current event split"
  assert_eq "$current_socket" $'16777216/33554432\t8388608/16777216' "Docker benchmark table connected UDP buffer summary"
  assert_eq "$current_receiver" $'1:0/212992/212992\t4:4194304/8388608/8388608\t212992/212992\t212992/212992\t0\tnone' "Docker benchmark table receiver buffer summary"
  assert_eq "$current_pipeline_shape" $'nvpn_direct_endpoint_queue:rate_per_sec=11,p95_ms=0.2,p99_ms=0.5,max_ms=1,allmax_ms=2\tnvpn_direct_endpoint_queue:rate_per_sec=21,p95_ms=1,p99_ms=2,max_ms=4,allmax_ms=8\tavg_packets=120.0,flush_per_sec=10,packets_per_sec=1200\tavg_input_packets=120.0,avg_routed_packets=120.0,avg_runs=1.0\tavg_events=100.0,avg_packets=100.0,flush_per_sec=8\tpackets_per_sec=1000,frames_per_sec=30,avg_packets_per_frame=33.3\tqueue_rate_per_sec=21,queue_p95_ms=1,queue_p99_ms=2,tun_batch_p99_ms=1' "Docker benchmark table TCP8 pipeline shape"
  assert_eq "$published_status" $'false\t10\tudp_namespace_rcvbuf_errors:5;connected_udp_kernel_dropped:3;connected_udp_peer_kernel_dropped:2\tfail' "Docker benchmark table published status"
  assert_eq "$published_receiver" $'5\tmixed-hard+receiver-rcvbuf' "Docker benchmark table UDP loss attribution"
  assert_eq "$published_events" $'0\t5\t3\t2\t0\t0' "Docker benchmark table published event split"
  assert_eq "$legacy_status" $'true\t15\tdecrypt_fallback_pressure_drain:15\tfail' "Docker benchmark table legacy phase hard-event fallback"
  assert_eq "$wg_status" $'wireguard-go\ttrue\tn/a\treference' "Docker benchmark table WG reference status"
  assert_eq "$current_provenance" $'nvpnabc\tfipsabc\tnvpn=false,fips=false\tboth:l1/r1' "Docker benchmark table provenance"
  case "$markdown_row" in
    *"| 3000 | 10 | 2800 |"*) ;;
    *) fail "Docker benchmark markdown row missing current throughput: $markdown_row" ;;
  esac

  rm -rf "$dir"
}

test_docker_benchmark_summary_guards_are_opt_in() {
  local dir summary failure_path
  dir="$(mktemp -d)"
  summary="$dir/summary.tsv"
  write_summary_fixture \
    "$summary" \
    nvpn "" \
    650 4500 \
    700 5000 \
    800 6000 \
    197 1.5 \
    200 80 \
    2.5 0.8 \
    "$dir/raw"

  (
    OUTPUT_DIR="$dir/no-guard"
    docker_bench_assert_summary_guards "$summary"
  )

  if (
    OUTPUT_DIR="$dir/guarded"
    export NVPN_DOCKER_MIN_TCP_MBPS=1000
    export NVPN_DOCKER_MAX_TCP_SINGLE_RETRANS=1000
    export NVPN_DOCKER_MAX_UDP1000_LOSS_PCT=10
    export NVPN_DOCKER_MAX_PING_LOSS_PCT=1
    docker_bench_assert_summary_guards "$summary"
  ) 2>"$dir/guarded.stderr"; then
    fail "Docker benchmark guard should fail collapsed fixture"
  fi

  failure_path="$dir/guarded/guard-failures.tsv"
  assert_file_contains "$failure_path" $'tcp_single_mbps\t>=\t650\t1000' "guard TCP single throughput failure"
  assert_file_contains "$failure_path" $'tcp_4_mbps\t>=\t700\t1000' "guard TCP 4 throughput failure"
  assert_file_contains "$failure_path" $'tcp_8_mbps\t>=\t800\t1000' "guard TCP 8 throughput failure"
  assert_file_contains "$failure_path" $'tcp_single_retrans\t<=\t4500\t1000' "guard TCP single retrans failure"
  assert_file_contains "$failure_path" $'udp_1000_loss_pct\t<=\t80\t10' "guard UDP1000 loss failure"
  assert_file_contains "$failure_path" $'ping_loss_pct\t<=\t2.5\t1' "guard ping loss failure"
  assert_file_contains "$dir/guarded.stderr" "docker bench guard failed: wrote $failure_path" "guard stderr failure path"

  rm -rf "$dir"
}

test_docker_benchmark_summary_guards_accept_healthy_fixture() {
  local dir summary
  dir="$(mktemp -d)"
  summary="$dir/summary.tsv"
  write_summary_fixture \
    "$summary" \
    nvpn "" \
    3300 200 \
    3200 1200 \
    3100 5500 \
    199 0 \
    995 0.5 \
    0 0.8 \
    "$dir/raw"

  (
    OUTPUT_DIR="$dir/guarded"
    export NVPN_DOCKER_MIN_TCP_MBPS=1000
    export NVPN_DOCKER_MAX_TCP_SINGLE_RETRANS=1000
    export NVPN_DOCKER_MAX_UDP1000_LOSS_PCT=10
    export NVPN_DOCKER_MAX_PING_LOSS_PCT=1
    docker_bench_assert_summary_guards "$summary"
  )

  [[ ! -f "$dir/guarded/guard-failures.tsv" ]] \
    || fail "healthy guarded fixture unexpectedly wrote guard failures"

  rm -rf "$dir"
}

test_docker_benchmark_tun_drop_guards_are_opt_in() {
  local dir tun_summary failure_path
  dir="$(mktemp -d)"
  tun_summary="$dir/nvpn-linux-tun-netdev.tsv"
  {
    printf 'service\tiface\ttx_queue_len\trx_packets\trx_dropped\ttx_packets\ttx_dropped\n'
    printf 'node-a\tutun100\t4096\t1000\t0\t2000\t15\n'
    printf 'node-b\tutun100\t4096\t2000\t2\t1000\t0\n'
  } >"$tun_summary"

  (
    OUTPUT_DIR="$dir/no-guard"
    RAW_DIR="$dir/raw"
    mkdir -p "$OUTPUT_DIR"
    docker_bench_assert_tun_drop_guards "$tun_summary"
  )

  if (
    OUTPUT_DIR="$dir/guarded"
    RAW_DIR="$dir/raw"
    mkdir -p "$OUTPUT_DIR"
    export NVPN_DOCKER_MAX_TUN_RX_DROPPED=0
    export NVPN_DOCKER_MAX_TUN_TX_DROPPED=0
    docker_bench_assert_tun_drop_guards "$tun_summary"
  ) 2>"$dir/guarded.stderr"; then
    fail "Docker TUN drop guard should fail dropped fixture"
  fi

  failure_path="$dir/guarded/guard-failures.tsv"
  assert_file_contains "$failure_path" $'tun_rx_dropped_total\t<=\t2\t0' "TUN RX drop guard failure"
  assert_file_contains "$failure_path" $'tun_tx_dropped_total\t<=\t15\t0' "TUN TX drop guard failure"
  assert_file_contains "$dir/guarded.stderr" "docker bench guard failed: wrote $failure_path" "TUN drop guard stderr failure path"

  rm -rf "$dir"
}

test_docker_benchmark_pipeline_hard_event_guards_are_opt_in() {
  local dir phase_summary failure_path
  dir="$(mktemp -d)"
  phase_summary="$dir/phase-summary.tsv"
  {
    printf 'phase\tpeer\thard_events\n'
    printf 'benchmark\tnode-b\t%s\n' \
      'connected_udp_direct_decrypt_bulk_shed:max_rate_per_sec=3,total=11;endpoint_event_bulk_dropped:max_rate_per_sec=2,total=10;decrypt_worker_queue_full:max_rate_per_sec=4,total=13;decrypt_worker_bulk_dropped:max_rate_per_sec=5,total=17;decrypt_fallback_pressure_drain:max_rate_per_sec=1,total=19;decrypt_fallback_priority_gated:max_rate_per_sec=1,total=23;decrypt_fsp_helper_window_fallback:max_rate_per_sec=1,total=29;decrypt_fsp_open_worker_window_fallback:max_rate_per_sec=1,total=30;decrypt_fsp_helper_queue_full_fallback:max_rate_per_sec=1,total=31;decrypt_fsp_helper_completion_backlog_fallback:max_rate_per_sec=1,total=37;decrypt_fsp_open_worker_completion_backlog_fallback:max_rate_per_sec=1,total=5;decrypt_fsp_worker_replay_dropped:max_rate_per_sec=2,total=7;fmp_aead_completion_aead_failed:max_rate_per_sec=1,total=2;fsp_aead_completion_aead_failed:max_rate_per_sec=1,total=3;fsp_aead_completion_epoch_mismatch:max_rate_per_sec=1,total=4;fsp_aead_completion_stale_session:max_rate_per_sec=1,total=44;fsp_aead_completion_stale_order:max_rate_per_sec=1,total=45;fsp_aead_completion_stale_ticket:max_rate_per_sec=1,total=46;fsp_aead_completion_duplicate_ticket:max_rate_per_sec=1,total=47;fsp_aead_completion_window_exceeded:max_rate_per_sec=1,total=48;fmp_aead_completion_replay_dropped:max_rate_per_sec=1,total=39;fsp_aead_completion_replay_dropped:max_rate_per_sec=1,total=40;fsp_aead_completion_replay_dropped_helper:max_rate_per_sec=1,total=41;fsp_aead_completion_replay_dropped_helper_returned:max_rate_per_sec=1,total=43'
  } >"$phase_summary"

  (
    OUTPUT_DIR="$dir/no-guard"
    mkdir -p "$OUTPUT_DIR"
    docker_bench_assert_pipeline_hard_event_guards "$phase_summary"
  )

  if (
    OUTPUT_DIR="$dir/guarded"
    mkdir -p "$OUTPUT_DIR"
    export NVPN_DOCKER_MAX_CONNECTED_UDP_DIRECT_DECRYPT_BULK_SHED=0
    export NVPN_DOCKER_MAX_ENDPOINT_EVENT_BULK_DROPPED=0
    export NVPN_DOCKER_MAX_DECRYPT_WORKER_QUEUE_FULL=0
    export NVPN_DOCKER_MAX_DECRYPT_WORKER_BULK_DROPPED=0
    export NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRESSURE_DRAIN=0
    export NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRIORITY_GATED=0
    export NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_WINDOW_FALLBACK=0
    export NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_WINDOW_FALLBACK=0
    export NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_QUEUE_FULL_FALLBACK=0
    export NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_COMPLETION_BACKLOG_FALLBACK=0
    export NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_COMPLETION_BACKLOG_FALLBACK=0
    export NVPN_DOCKER_MAX_DECRYPT_FSP_WORKER_REPLAY_DROPPED=0
    export NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_AEAD_FAILED=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_AEAD_FAILED=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_EPOCH_MISMATCH=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_SESSION=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_ORDER=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_TICKET=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_DUPLICATE_TICKET=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_WINDOW_EXCEEDED=0
    export NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_REPLAY_DROPPED=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED_HELPER=0
    export NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED_HELPER_RETURNED=0
    docker_bench_assert_pipeline_hard_event_guards "$phase_summary"
  ) 2>"$dir/guarded.stderr"; then
    fail "Docker pipeline hard event guard should fail open-worker completion backlog fixture"
  fi

  failure_path="$dir/guarded/guard-failures.tsv"
  assert_file_contains "$failure_path" $'connected_udp_direct_decrypt_bulk_shed_total\t<=\t11\t0' "pipeline hard event connected direct-decrypt shed failure"
  assert_file_contains "$failure_path" $'endpoint_event_bulk_dropped_total\t<=\t10\t0' "pipeline hard event endpoint event bulk drop failure"
  assert_file_contains "$failure_path" $'decrypt_worker_queue_full_total\t<=\t13\t0' "pipeline hard event decrypt worker queue-full failure"
  assert_file_contains "$failure_path" $'decrypt_worker_bulk_dropped_total\t<=\t17\t0' "pipeline hard event decrypt worker bulk drop failure"
  assert_file_contains "$failure_path" $'decrypt_fallback_pressure_drain_total\t<=\t19\t0' "pipeline hard event decrypt fallback pressure drain failure"
  assert_file_contains "$failure_path" $'decrypt_fallback_priority_gated_total\t<=\t23\t0' "pipeline hard event decrypt fallback priority gated failure"
  assert_file_contains "$failure_path" $'decrypt_fsp_helper_window_fallback_total\t<=\t29\t0' "pipeline hard event FSP helper window fallback failure"
  assert_file_contains "$failure_path" $'decrypt_fsp_open_worker_window_fallback_total\t<=\t30\t0' "pipeline hard event FSP open-worker window fallback failure"
  assert_file_contains "$failure_path" $'decrypt_fsp_helper_queue_full_fallback_total\t<=\t31\t0' "pipeline hard event FSP helper queue-full fallback failure"
  assert_file_contains "$failure_path" $'decrypt_fsp_helper_completion_backlog_fallback_total\t<=\t37\t0' "pipeline hard event FSP helper completion backlog failure"
  assert_file_contains "$failure_path" $'decrypt_fsp_open_worker_completion_backlog_fallback_total\t<=\t5\t0' "pipeline hard event FSP open-worker backlog failure"
  assert_file_contains "$failure_path" $'decrypt_fsp_worker_replay_dropped_total\t<=\t7\t0' "pipeline hard event FSP worker replay drop failure"
  assert_file_contains "$failure_path" $'fmp_aead_completion_aead_failed_total\t<=\t2\t0' "pipeline hard event FMP AEAD failed failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_aead_failed_total\t<=\t3\t0' "pipeline hard event FSP AEAD failed failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_epoch_mismatch_total\t<=\t4\t0' "pipeline hard event FSP epoch mismatch failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_stale_session_total\t<=\t44\t0' "pipeline hard event FSP stale session failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_stale_order_total\t<=\t45\t0' "pipeline hard event FSP stale order failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_stale_ticket_total\t<=\t46\t0' "pipeline hard event FSP stale ticket failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_duplicate_ticket_total\t<=\t47\t0' "pipeline hard event FSP duplicate ticket failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_window_exceeded_total\t<=\t48\t0' "pipeline hard event FSP window exceeded failure"
  assert_file_contains "$failure_path" $'fmp_aead_completion_replay_dropped_total\t<=\t39\t0' "pipeline hard event FMP AEAD replay drop failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_replay_dropped_total\t<=\t40\t0' "pipeline hard event FSP AEAD replay drop failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_replay_dropped_helper_total\t<=\t41\t0' "pipeline hard event FSP helper replay drop failure"
  assert_file_contains "$failure_path" $'fsp_aead_completion_replay_dropped_helper_returned_total\t<=\t43\t0' "pipeline hard event FSP helper returned replay drop failure"
  assert_file_contains "$dir/guarded.stderr" "docker bench guard failed: wrote $failure_path" "pipeline hard event guard stderr failure path"

  rm -rf "$dir"
}

test_docker_benchmark_pipeline_hard_event_guard_can_require_all_zero() {
  local dir phase_summary failure_path
  dir="$(mktemp -d)"
  phase_summary="$dir/phase-summary.tsv"
  {
    printf 'phase\tpeer\thard_events\n'
    printf 'tcp-4\tnode-a\t%s\n' \
      'nvpn_tun_to_mesh_bulk_dropped:max_rate_per_sec=289,total=356;nvpn_tun_to_mesh_bulk_dropped_batches:max_rate_per_sec=7,total=8;connected_udp_peer_kernel_dropped:max_rate_per_sec=1,total=1;dataplane_live_retired_drops:max_rate_per_sec=2,total=5'
    printf 'udp-1000\tnode-b\t%s\n' \
      'connected_udp_peer_kernel_dropped:max_rate_per_sec=2,total=3'
  } >"$phase_summary"

  (
    OUTPUT_DIR="$dir/no-guard"
    mkdir -p "$OUTPUT_DIR"
    docker_bench_assert_pipeline_hard_event_guards "$phase_summary"
  )

  if (
    OUTPUT_DIR="$dir/guarded"
    mkdir -p "$OUTPUT_DIR"
    export NVPN_DOCKER_REQUIRE_NO_PIPELINE_HARD_EVENTS=1
    docker_bench_assert_pipeline_hard_event_guards "$phase_summary"
  ) 2>"$dir/guarded.stderr"; then
    fail "Docker pipeline hard event zero guard should fail any nonzero event fixture"
  fi

  failure_path="$dir/guarded/guard-failures.tsv"
  assert_file_contains "$failure_path" $'nvpn_tun_to_mesh_bulk_dropped_total\t<=\t356\t0' "pipeline hard event global zero TUN-to-mesh drop failure"
  assert_file_contains "$failure_path" $'nvpn_tun_to_mesh_bulk_dropped_batches_total\t<=\t8\t0' "pipeline hard event global zero TUN-to-mesh batch failure"
  assert_file_contains "$failure_path" $'connected_udp_peer_kernel_dropped_total\t<=\t4\t0' "pipeline hard event global zero connected peer drop failure"
  assert_file_contains "$failure_path" $'dataplane_live_retired_drops_total\t<=\t5\t0' "pipeline hard event global zero dataplane retired drop failure"
  assert_file_contains "$dir/guarded.stderr" "docker bench guard failed: wrote $failure_path" "pipeline hard event global zero stderr failure path"

  rm -rf "$dir"
}

test_docker_benchmark_pipeline_hard_event_guard_allows_named_bulk_events() {
  local dir phase_summary failure_path
  dir="$(mktemp -d)"
  phase_summary="$dir/phase-summary.tsv"
  {
    printf 'phase\tpeer\thard_events\n'
    printf 'tcp-8\tnode-a\t%s\n' \
      'nvpn_tun_to_mesh_bulk_dropped:max_rate_per_sec=131,total=131;nvpn_tun_to_mesh_bulk_dropped_batches:max_rate_per_sec=1,total=1;nvpn_tun_to_mesh_bulk_dropped_packet_cap:max_rate_per_sec=131,total=131;endpoint_event_bulk_dropped:max_rate_per_sec=2,total=2'
  } >"$phase_summary"

  if (
    OUTPUT_DIR="$dir/guarded"
    mkdir -p "$OUTPUT_DIR"
    export NVPN_DOCKER_REQUIRE_NO_PIPELINE_HARD_EVENTS=1
    export NVPN_DOCKER_ALLOW_PIPELINE_HARD_EVENTS=nvpn_tun_to_mesh_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped_batches,nvpn_tun_to_mesh_bulk_dropped_packet_cap
    docker_bench_assert_pipeline_hard_event_guards "$phase_summary"
  ) 2>"$dir/guarded.stderr"; then
    fail "Docker pipeline hard event zero guard should still fail unallowed endpoint event"
  fi

  failure_path="$dir/guarded/guard-failures.tsv"
  assert_file_not_contains "$failure_path" "nvpn_tun_to_mesh_bulk_dropped_total" "allowed TUN-to-mesh bulk drop should not fail global guard"
  assert_file_contains "$failure_path" $'endpoint_event_bulk_dropped_total\t<=\t2\t0' "unallowed endpoint event should still fail global guard"

  (
    OUTPUT_DIR="$dir/allowed"
    mkdir -p "$OUTPUT_DIR"
    export NVPN_DOCKER_REQUIRE_NO_PIPELINE_HARD_EVENTS=1
    export NVPN_DOCKER_ALLOW_PIPELINE_HARD_EVENTS=nvpn_tun_to_mesh_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped_batches,nvpn_tun_to_mesh_bulk_dropped_packet_cap,endpoint_event_bulk_dropped
    docker_bench_assert_pipeline_hard_event_guards "$phase_summary"
  )

  rm -rf "$dir"
}

test_docker_comparison_relaxes_udp_bulk_loss_under_cpu_stress() {
  local dir out threshold_status threshold_failures udp_threshold ping_threshold effective_udp_delta
  dir="$(mktemp -d)"
  write_summary_fixture \
    "$dir/nvpn/summary.tsv" \
    nvpn "" \
    300 10 \
    400 20 \
    500 30 \
    199 0.5 \
    990 4.5 \
    0 0.8 \
    "$dir/nvpn/raw"
  write_metadata_fixture "$dir/nvpn" nvpn true remote 0 1
  write_summary_fixture \
    "$dir/reference/summary.tsv" \
    boringtun 1 \
    250 10 \
    390 20 \
    490 30 \
    198 0.1 \
    950 0.0 \
    0 0.4 \
    "$dir/reference/raw"
  write_metadata_fixture "$dir/reference" boringtun true remote 0 1
  out="$dir/out"

  NVPN_DOCKER_COMPARISON_REQUIRE_NVPN_UDP_ZERO_LOSS=0 "$COMPARE_SCRIPT" "$dir/nvpn" "$dir/reference" "$out" >/dev/null

  threshold_status="$(jq -r '.threshold_status.status' "$out/comparison.json")"
  threshold_failures="$(jq -r '.threshold_status.failures' "$out/comparison.json")"
  effective_udp_delta="$(jq -r '.threshold_policy.effective_udp_loss_delta_pct' "$out/comparison.json")"
  udp_threshold="$(awk -F '\t' '$1 == "udp_1000_loss" { print $3 "\t" $6 "\t" $7 }' "$out/thresholds.tsv")"
  ping_threshold="$(awk -F '\t' '$1 == "ping_loss" { print $3 "\t" $6 "\t" $7 }' "$out/thresholds.tsv")"

  assert_eq "$threshold_status" "pass" "both-stressed UDP bulk loss threshold status"
  assert_eq "$threshold_failures" "0" "both-stressed UDP bulk loss failures"
  assert_eq "$effective_udp_delta" "5" "both-stressed effective UDP loss threshold"
  assert_eq "$udp_threshold" $'pass\t<=reference+5pp\t4.500pp' "both-stressed UDP loss row"
  assert_eq "$ping_threshold" $'pass\t<=reference+1pp\t0.000pp' "ping loss remains strict"

  rm -rf "$dir"
}

test_docker_comparison_selects_wireguard_go_reference() {
  local dir out backend threads threshold_status tcp_ratio
  dir="$(mktemp -d)"
  write_summary_fixture \
    "$dir/nvpn/summary.tsv" \
    nvpn "" \
    300 10 \
    400 20 \
    500 30 \
    199 0 \
    990 0 \
    0 0.8 \
    "$dir/nvpn/raw"
  write_metadata_fixture "$dir/nvpn" nvpn true both 1 1
  write_summary_fixture \
    "$dir/reference/summary.tsv" \
    wireguard-go "" \
    250 10 \
    390 20 \
    490 30 \
    198 0 \
    950 0 \
    0 0.4 \
    "$dir/reference/raw"
  write_metadata_fixture "$dir/reference" wireguard-go true both 1 1
  out="$dir/out"

  NVPN_DOCKER_COMPARISON_REFERENCE_BACKEND=wireguard-go \
    "$COMPARE_SCRIPT" "$dir/nvpn" "$dir/reference" "$out" >/dev/null

  backend="$(awk -F '\t' '$1 == "reference" { print $3 }' "$out/comparison.tsv")"
  threads="$(awk -F '\t' '$1 == "reference" { print $4 }' "$out/comparison.tsv")"
  threshold_status="$(jq -r '.threshold_status.status' "$out/comparison.json")"
  tcp_ratio="$(awk -F '\t' '$1 == "tcp_single_mbps" { print $6 }' "$out/ratios.tsv")"

  assert_eq "$backend" "wireguard-go" "wireguard-go comparison backend"
  assert_eq "$threads" "" "wireguard-go comparison thread column"
  assert_eq "$threshold_status" "pass" "wireguard-go comparison status"
  assert_eq "$tcp_ratio" "120.0" "wireguard-go TCP ratio"

  rm -rf "$dir"
}

test_docker_comparison_labels_same_backend_profiles() {
  local dir out nvpn_label reference_label reference_backend json_labels tcp_ratio
  dir="$(mktemp -d)"
  write_summary_fixture \
    "$dir/helper2/summary.tsv" \
    nvpn "" \
    600 10 \
    610 20 \
    620 30 \
    199 0 \
    990 0 \
    0 0.8 \
    "$dir/helper2/raw"
  write_metadata_fixture "$dir/helper2" nvpn false both 0 0 false
  write_summary_fixture \
    "$dir/defaultoff/summary.tsv" \
    nvpn "" \
    300 10 \
    400 20 \
    500 30 \
    199 0 \
    990 0 \
    0 0.4 \
    "$dir/defaultoff/raw"
  write_metadata_fixture "$dir/defaultoff" nvpn false both 0 0 false
  out="$dir/out"

  NVPN_DOCKER_COMPARISON_NVPN_LABEL=helper2 \
    NVPN_DOCKER_COMPARISON_REFERENCE_BACKEND=nvpn \
    NVPN_DOCKER_COMPARISON_REFERENCE_LABEL=defaultoff \
    "$COMPARE_SCRIPT" "$dir/helper2" "$dir/defaultoff" "$out" >/dev/null

  nvpn_label="$(awk -F '\t' 'NR == 2 { print $1 }' "$out/comparison.tsv")"
  reference_label="$(awk -F '\t' 'NR == 3 { print $1 }' "$out/comparison.tsv")"
  reference_backend="$(awk -F '\t' 'NR == 3 { print $3 }' "$out/comparison.tsv")"
  json_labels="$(jq -r '.inputs.nvpn_label, .inputs.reference_label, .inputs.reference_backend' "$out/comparison.json" | paste -sd ':' -)"
  tcp_ratio="$(awk -F '\t' '$1 == "tcp_single_mbps" { print $6 }' "$out/ratios.tsv")"

  assert_eq "$nvpn_label" "helper2" "same-backend comparison nvpn label"
  assert_eq "$reference_label" "defaultoff" "same-backend comparison reference label"
  assert_eq "$reference_backend" "nvpn" "same-backend comparison reference backend"
  assert_eq "$json_labels" "helper2:defaultoff:nvpn" "same-backend comparison JSON labels"
  assert_eq "$tcp_ratio" "200.0" "same-backend comparison TCP ratio"

  rm -rf "$dir"
}

test_nvpn_perf_docker_records_daemon_cpu_phase_artifact() {
  local script="$ROOT_DIR/scripts/perf-docker.sh"
  assert_file_contains "$script" "nvpn-daemon-cpu-phases.tsv" "nvpn Docker perf daemon CPU artifact"
  assert_file_contains "$script" "append_daemon_cpu_phase_rows" "nvpn Docker perf phase CPU rows"
  assert_file_contains "$script" "docker_bench_iperf_transfer_bytes" "nvpn Docker perf transfer-byte accounting"
}

test_wireguard_go_perf_docker_records_cpu_phase_artifact() {
  local script="$ROOT_DIR/scripts/perf-docker-wireguard-go.sh"
  assert_file_contains "$script" "wireguard-go-cpu-phases.tsv" "wireguard-go Docker perf CPU artifact"
  assert_file_contains "$script" "append_wireguard_go_cpu_phase_rows" "wireguard-go Docker perf phase CPU rows"
  assert_file_contains "$script" "docker_bench_iperf_transfer_bytes" "wireguard-go Docker perf transfer-byte accounting"
}

test_docker_perf_scripts_share_iperf_socket_buffer_limit_helper() {
  local helper="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
  assert_file_contains "$helper" "docker_bench_configure_iperf_socket_buffer_limits" "Docker perf shared socket-buffer limit helper"
  assert_file_contains "$helper" 'net.core.rmem_max=$bytes net.core.wmem_max=$bytes' "Docker perf shared helper raises UDP receiver sysctls"
  assert_file_contains "$helper" "failed to raise UDP socket buffer sysctls" "Docker perf shared helper fails closed on capped receiver buffers"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker.sh" "docker_bench_configure_iperf_socket_buffer_limits perf" "nvpn Docker perf uses shared socket-buffer helper"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker-wireguard-go.sh" "docker_bench_configure_iperf_socket_buffer_limits perf-wireguard-go" "wireguard-go Docker perf uses shared socket-buffer helper"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker-boringtun.sh" "docker_bench_configure_iperf_socket_buffer_limits perf-boringtun" "boringtun Docker perf uses shared socket-buffer helper"
}

test_docker_perf_scripts_reject_translated_processes() {
  local helper="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
  local native rosetta qemu
  native='30 /usr/local/bin/nvpn connect'
  rosetta='30 /run/rosetta/rosetta /usr/local/bin/nvpn daemon --config /cfg/container.toml'
  qemu='30 /usr/bin/qemu-x86_64 /usr/local/bin/wireguard-go --foreground wg0'

  if docker_bench_process_uses_translation "$native"; then
    fail "native process was classified as translated"
  fi
  if ! docker_bench_process_uses_translation "$rosetta"; then
    fail "Rosetta process was not classified as translated"
  fi
  if ! docker_bench_process_uses_translation "$qemu"; then
    fail "QEMU process was not classified as translated"
  fi

  assert_file_contains "$helper" "docker_bench_assert_native_processes" "Docker perf shared native-process guard"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker.sh" "docker_bench_assert_native_processes perf nvpn node-a node-b" "nvpn Docker perf rejects translated nvpn"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker-wireguard-go.sh" "docker_bench_assert_native_processes perf-wireguard-go wireguard-go node-a node-b" "wireguard-go Docker perf rejects translated wireguard-go"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker-boringtun.sh" "docker_bench_assert_native_processes perf-boringtun boringtun-cli node-a node-b" "boringtun Docker perf rejects translated boringtun"
}

test_docker_perf_scripts_share_host_quiet_guard() {
  local helper="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
  assert_file_contains "$helper" "docker_bench_validate_host_quiet" "Docker perf shared host quiet guard"
  assert_file_contains "$helper" "NVPN_DOCKER_MAX_HOST_LOAD_PER_CPU" "Docker perf host quiet guard env"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker.sh" "docker_bench_validate_host_quiet perf" "nvpn Docker perf uses host quiet guard"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker-wireguard-go.sh" "docker_bench_validate_host_quiet perf-wireguard-go" "wireguard-go Docker perf uses host quiet guard"
  assert_file_contains "$ROOT_DIR/scripts/perf-docker-boringtun.sh" "docker_bench_validate_host_quiet perf-boringtun" "boringtun Docker perf uses host quiet guard"
}

test_json_and_ping_parsers
test_cpu_accounting_helpers
test_udp1000_parallel_bandwidth_helpers_preserve_total_target
test_summary_row
test_metadata_writer_records_cpu_stress
test_local_fips_repo_path_defaults_to_patch
test_metadata_writer_records_pipeline_trace
test_metadata_writer_records_iperf_interval
test_metadata_writer_records_iperf_timeout
test_metadata_writer_records_udp1000_per_stream_bandwidth
test_metadata_writer_records_guard_thresholds
test_metadata_writer_records_fips_soak_thresholds
test_metadata_writer_records_run_provenance
test_host_quiet_guard_rejects_invalid_threshold
test_dataplane_profile_expands_connect_env
test_placement_profile_expands_worker_open_connect_env
test_placement_profile_allows_explicit_nonexclusive_worker_open_guard
test_dataplane_profile_rejects_unknown_profile
test_connect_env_scope_rejects_stranded_outer_env
test_metadata_writer_records_direct_fmp_guard
test_metadata_writer_records_fsp_aead_helper_guard
test_metadata_writer_records_pipeline_hard_event_guard
test_extra_env_validation_rejects_retired_dataplane_knob_names
test_extra_env_validation_rejects_stale_fips_helper_names
test_pipeline_summary_helpers
test_nvpn_tun_write_summary_prefers_coalesced_frame_interval
test_docker_comparison_outputs
test_docker_benchmark_table_outputs
test_docker_benchmark_summary_guards_are_opt_in
test_docker_benchmark_summary_guards_accept_healthy_fixture
test_docker_benchmark_tun_drop_guards_are_opt_in
test_docker_benchmark_pipeline_hard_event_guards_are_opt_in
test_docker_benchmark_pipeline_hard_event_guard_can_require_all_zero
test_docker_benchmark_pipeline_hard_event_guard_allows_named_bulk_events
test_docker_comparison_relaxes_udp_bulk_loss_under_cpu_stress
test_docker_comparison_selects_wireguard_go_reference
test_docker_comparison_labels_same_backend_profiles
test_nvpn_perf_docker_records_daemon_cpu_phase_artifact
test_wireguard_go_perf_docker_records_cpu_phase_artifact
test_docker_perf_scripts_share_iperf_socket_buffer_limit_helper
test_docker_perf_scripts_reject_translated_processes
test_docker_perf_scripts_share_host_quiet_guard

printf 'docker benchmark summary self-test passed\n'
