#!/usr/bin/env bash
# Local self-tests for simple Docker benchmark summary helpers.
#
# These tests do not start Docker. They pin the JSON/ping parsers and TSV row
# contract used by scripts/perf-docker.sh and scripts/perf-docker-boringtun.sh.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
COMPARE_SCRIPT="$ROOT_DIR/scripts/compare-docker-benchmarks.sh"

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

write_tcp_json() {
  local path="$1"
  cat >"$path" <<'EOF'
{
  "end": {
    "sum_received": {
      "bits_per_second": 1234567890
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

--- 10.44.0.2 ping statistics ---
300 packets transmitted, 299 received, 0.333333% packet loss, time 3017ms
rtt min/avg/max/mdev = 0.400/1.234/8.900/0.500 ms
EOF
}

test_json_and_ping_parsers() {
  local dir tcp_json udp_json ping_output stats loss avg
  dir="$(mktemp -d)"
  tcp_json="$dir/tcp.json"
  udp_json="$dir/udp.json"
  ping_output="$dir/ping.txt"
  write_tcp_json "$tcp_json"
  write_udp_json "$udp_json"
  write_ping_output "$ping_output"

  assert_eq "$(docker_bench_iperf_mbps "$tcp_json")" "1234.568" "TCP receiver Mbps"
  assert_eq "$(docker_bench_iperf_retrans "$tcp_json")" "7" "TCP retransmits"
  assert_eq "$(docker_bench_iperf_mbps "$udp_json")" "987.654" "UDP receiver Mbps"
  assert_eq "$(docker_bench_iperf_loss_pct "$udp_json")" "1.25" "UDP loss"
  stats="$(docker_bench_parse_ping_loss_avg "$ping_output")"
  read -r loss avg <<<"$stats"
  assert_eq "$loss" "0.333333" "ping loss"
  assert_eq "$avg" "1.234" "ping avg"

  rm -rf "$dir"
}

test_summary_row() {
  local dir tcp_single tcp_4 tcp_8 udp_200 udp_1000 ping_output
  local header row nvpn_row fields nvpn_fields
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
  assert_eq "$fields" "16" "summary field count"
  assert_eq "$nvpn_fields" "16" "nvpn summary field count"

  rm -rf "$dir"
}

test_metadata_writer_records_cpu_stress() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
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
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.guard_thresholds.min_tcp_mbps' "$metadata")" "1000" "metadata common TCP min guard"
  assert_eq "$(jq -r '.guard_thresholds.min_tcp_single_mbps' "$metadata")" "1200" "metadata TCP single min guard"
  assert_eq "$(jq -r '.guard_thresholds.max_tcp_retrans' "$metadata")" "9000" "metadata common retrans guard"
  assert_eq "$(jq -r '.guard_thresholds.max_tcp_8_retrans' "$metadata")" "12000" "metadata TCP 8 retrans guard"
  assert_eq "$(jq -r '.guard_thresholds.max_udp_loss_pct' "$metadata")" "2" "metadata common UDP loss guard"
  assert_eq "$(jq -r '.guard_thresholds.max_udp1000_loss_pct' "$metadata")" "5" "metadata UDP1000 loss guard"
  assert_eq "$(jq -r '.guard_thresholds.max_ping_loss_pct' "$metadata")" "0" "metadata ping loss guard"
  assert_eq "$(jq -r '.guard_thresholds.max_udp200_loss_pct' "$metadata")" "null" "metadata unset guard"

  rm -rf "$dir"
}

test_metadata_writer_records_run_provenance() {
  local dir metadata
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_EXTRA_ENV="FIPS_LINUX_BULK_CONTAINERS=1 NVPN_FIPS_LINUX_TUN_VNET=1"
    export NVPN_PATCH_LOCAL_FIPS=1
    docker_bench_write_metadata nvpn 3
  )

  assert_eq "$(jq -r '.run_env.extra_connect_env' "$metadata")" "FIPS_LINUX_BULK_CONTAINERS=1 NVPN_FIPS_LINUX_TUN_VNET=1" "metadata extra env"
  assert_eq "$(jq -r '.run_env.direct_fmp_forced' "$metadata")" "false" "metadata direct-FMP forced default"
  assert_eq "$(jq -r '.run_env.require_no_direct_fmp' "$metadata")" "false" "metadata no-direct requirement default"
  assert_eq "$(jq -r '.source.local_fips_patch.enabled' "$metadata")" "true" "metadata local FIPS enabled"
  assert_eq "$(jq -r '.source.nvpn | has("git_head")' "$metadata")" "true" "metadata nvpn head field"
  assert_eq "$(jq -r '.source.local_fips_patch | has("git_head")' "$metadata")" "true" "metadata FIPS head field"

  rm -rf "$dir"
}

test_metadata_writer_records_direct_fmp_guard() {
  local dir metadata helper_result
  dir="$(mktemp -d)"
  OUTPUT_DIR="$dir/out"
  metadata="$OUTPUT_DIR/metadata.json"
  mkdir -p "$OUTPUT_DIR"

  (
    export NVPN_DOCKER_EXTRA_ENV="FIPS_LINUX_BULK_CONTAINERS=1 FIPS_DIRECT_ENDPOINT_FMP_ONLY=1 NVPN_FIPS_LINUX_TUN_VNET=1"
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

test_pipeline_summary_helpers() {
  local dir lines all after load peak top fmp linux_bulk udp tun mesh_send mesh_recv tun_write hard
  dir="$(mktemp -d)"
  lines="$dir/pipeline.txt"
  cat >"$lines" <<'EOF'
[pipe 5s] fmp_worker_batch_flush=2/s fmp_worker_batch_packets=20/s udp_send_connected=20/s endpoint_event_wait=2/s avg=40.0us p50<=32.8us p95<=262.1us p99<=524.3us max<=1.0ms allmax=5.7ms
[pipe 5s] fmp_worker_batch_flush=10/s fmp_worker_batch_packets=420/s fmp_worker_batch_priority_packets=105/s fmp_worker_batch_bulk_packets=315/s fmp_worker_batch_full=9/s fmp_worker_batch_single=0.5/s fmp_send_group=12/s fmp_send_group_packets=420/s fmp_send_group_single=3/s udp_send_gso_batch=10/s udp_send_gso_packets=420/s udp_send_gso_batch_ge32=7/s udp_send_gso_batch_ge48=3/s udp_send_gso_batch_eq64=1/s udp_send_sendmmsg_batch=2/s udp_send_sendmmsg_packets=4/s udp_send_sendmmsg_batch_ge32=1/s udp_send_sendmmsg_batch_ge48=0/s udp_send_sendmmsg_batch_eq64=0/s fmp_worker_bulk_queue_wait=10/s avg=1.1ms p50<=2.1ms p95<=2.1ms p99<=8.4ms max<=16.8ms allmax=11.1ms fmp_linux_bulk_container_enqueued=12/s total=60 fmp_linux_bulk_container_packets=420/s total=2100 fmp_linux_bulk_container_sent=12/s total=60 fmp_linux_bulk_container_sent_packets=420/s total=2100 fmp_linux_bulk_container_queue_wait=12/s avg=1.0ms p50<=2.1ms p95<=2.1ms p99<=8.4ms max<=16.8ms allmax=11.1ms fmp_linux_bulk_container_ready_wait=10/s avg=2.0ms p50<=2.1ms p95<=4.2ms p99<=16.8ms max<=33.6ms allmax=33.6ms fmp_linux_bulk_container_first_slot_wait=12/s avg=40.0us p50<=32.8us p95<=131.1us p99<=262.1us max<=1.0ms allmax=1.2ms fmp_linux_bulk_container_all_slots_wait=12/s avg=260.0us p50<=262.1us p95<=524.3us p99<=1.0ms max<=2.1ms allmax=1.9ms encrypt_worker_bulk_queue_full=2/s total=10 | [nvpn-pipe 5s] nvpn_tun_read=300/s nvpn_mesh_send=300/s nvpn_tun_to_mesh_queue_wait=10/s avg=31.0us p50<=32.8us p95<=131.1us p99<=524.3us max<=1.0ms allmax=2.5ms nvpn_tun_read_batch_flush=12/s total=60 nvpn_tun_read_batch_packets=300/s total=1500 nvpn_tun_read_batch_full=3/s total=15 nvpn_tun_read_batch_single=1/s total=5 nvpn_tun_read_packet_bytes=360000/s total=1800000 nvpn_mesh_send_batch_flush=12/s total=60 nvpn_mesh_send_batch_input_packets=300/s total=1500 nvpn_mesh_send_batch_routed_packets=285/s total=1425 nvpn_mesh_send_batch_runs=15/s total=75 nvpn_mesh_send_batch_full=3/s total=15 nvpn_mesh_recv_batch_flush=6/s total=30 nvpn_mesh_recv_batch_events=300/s total=1500 nvpn_mesh_recv_batch_packets=240/s total=1200 nvpn_mesh_recv_packet_bytes=288000/s total=1440000 nvpn_mesh_recv_batch_full=2/s total=10 nvpn_mesh_recv_batch_single_packet=1/s total=5 nvpn_tun_write_packets=240/s total=1200 nvpn_tun_write_packet_bytes=288000/s total=1440000 nvpn_tun_write_would_block=3/s total=15
[nvpn-pipe 5s] nvpn_tun_read=1/s nvpn_mesh_send=1/s nvpn_tun_to_mesh_queue_wait=1/s avg=9.0ms p50<=8.4ms p95<=16.8ms p99<=33.6ms max<=67.1ms allmax=67.1ms
EOF

  after="$(docker_bench_pipeline_lines_after_start_from_stdin 1 <"$lines" | wc -l | tr -d ' ')"
  load="$(docker_bench_load_pipeline_line_from_stdin <"$lines")"
  peak="$(docker_bench_peak_wait_pipeline_line_from_stdin <"$lines")"
  top="$(docker_bench_pipeline_queue_wait_top_summary "$load")"
  fmp="$(docker_bench_pipeline_fmp_worker_batch_summary "$load")"
  linux_bulk="$(docker_bench_pipeline_linux_bulk_container_summary "$load")"
  udp="$(docker_bench_pipeline_udp_send_batch_summary "$load")"
  tun="$(docker_bench_pipeline_nvpn_tun_read_batch_summary "$load")"
  mesh_send="$(docker_bench_pipeline_nvpn_mesh_send_batch_summary "$load")"
  mesh_recv="$(docker_bench_pipeline_nvpn_mesh_recv_batch_summary "$load")"
  tun_write="$(docker_bench_pipeline_nvpn_tun_write_summary "$load")"
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
  assert_eq "$fmp" "avg_packets=42.0,full_pct=90.0,single_pct=5.0,priority_pct=25.0,bulk_pct=75.0,flush_per_sec=10,packets_per_sec=420,priority_packets_per_sec=105,bulk_packets_per_sec=315,send_groups_per_flush=1.2,send_group_avg_packets=35.0,send_group_single_pct=25.0,send_groups_per_sec=12,send_group_packets_per_sec=420" "FMP worker batch summary"
  assert_eq "$linux_bulk" "avg_packets=35.0,avg_sent_packets=35.0,enqueued_per_sec=12,packets_per_sec=420,sent_per_sec=12,sent_packets_per_sec=420,queue_p95_ms=2.1,queue_p99_ms=8.4,ready_p95_ms=4.2,ready_p99_ms=16.8,first_slot_p95_ms=0.1311,first_slot_p99_ms=0.2621,all_slots_p95_ms=0.5243,all_slots_p99_ms=1" "Linux bulk container summary"
  assert_eq "$udp" "gso_packet_pct=99.1,sendmmsg_packet_pct=0.9,avg_packets=35.3,gso_avg_packets=42.0,sendmmsg_avg_packets=2.0,gso_ge32_pct=70.0,gso_ge48_pct=30.0,gso_eq64_pct=10.0,sendmmsg_ge32_pct=50.0,sendmmsg_ge48_pct=0.0,sendmmsg_eq64_pct=0.0,gso_batch_per_sec=10,gso_packets_per_sec=420,sendmmsg_batch_per_sec=2,sendmmsg_packets_per_sec=4,total_packets_per_sec=424" "UDP send batch summary"
  assert_eq "$tun" "avg_packets=25.0,full_pct=25.0,single_pct=8.3,avg_packet_bytes=1200.0,flush_per_sec=12,packets_per_sec=300,bytes_per_sec=360000" "nvpn TUN read batch summary"
  assert_eq "$mesh_send" "avg_input_packets=25.0,avg_routed_packets=23.8,avg_runs=1.2,routed_pct=95.0,full_pct=25.0,flush_per_sec=12,input_packets_per_sec=300,routed_packets_per_sec=285,runs_per_sec=15" "nvpn mesh send batch summary"
  assert_eq "$mesh_recv" "avg_events=50.0,avg_packets=40.0,full_pct=33.3,single_packet_pct=16.7,avg_packet_bytes=1200.0,flush_per_sec=6,events_per_sec=300,packets_per_sec=240,bytes_per_sec=288000" "nvpn mesh receive batch summary"
  assert_eq "$tun_write" "packets_per_sec=240,bytes_per_sec=288000,avg_packet_bytes=1200.0,would_block_per_sec=3" "nvpn TUN write summary"
  assert_eq "$hard" "encrypt_worker_bulk_queue_full:max_rate_per_sec=2,total=10" "pipeline hard event summary"

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

  mkdir -p "$(dirname "$path")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    backend threads duration_secs \
    tcp_single_mbps tcp_single_retrans \
    tcp_4_mbps tcp_4_retrans \
    tcp_8_mbps tcp_8_retrans \
    udp_200_mbps udp_200_loss_pct \
    udp_1000_mbps udp_1000_loss_pct \
    ping_loss_pct ping_avg_ms raw_dir >"$path"
  printf '%s\t%s\t3\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$backend" "$threads" \
    "$tcp_single" "$tcp_single_retrans" \
    "$tcp_4" "$tcp_4_retrans" \
    "$tcp_8" "$tcp_8_retrans" \
    "$udp_200" "$udp_200_loss" \
    "$udp_1000" "$udp_1000_loss" \
    "$ping_loss" "$ping_avg" "$raw_dir" >>"$path"
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
  local dir out comparison_fields ratio_fields threshold_fields tcp_ratio ping_delta json_metric
  local threshold_tcp_4 threshold_status threshold_failures effective_udp_delta enforce_output stress_fields stress_json
  local pipeline_fields pipeline_json provenance_json
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
  write_metadata_fixture "$dir/nvpn" nvpn true remote 0 4 true 2 "FIPS_LINUX_BULK_CONTAINERS=1" true nvpnabc false fipsabc true
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
  write_metadata_fixture "$dir/reference" boringtun false both 0 0 false
  out="$dir/out"

  "$COMPARE_SCRIPT" "$dir/nvpn" "$dir/reference" "$out" >/dev/null

  comparison_fields="$(awk -F '\t' 'NR == 2 { print NF }' "$out/comparison.tsv")"
  ratio_fields="$(awk -F '\t' 'NR == 2 { print NF }' "$out/ratios.tsv")"
  threshold_fields="$(awk -F '\t' 'NR == 2 { print NF }' "$out/thresholds.tsv")"
  tcp_ratio="$(awk -F '\t' '$1 == "tcp_single_mbps" { print $6 "\t" $7 }' "$out/ratios.tsv")"
  ping_delta="$(awk -F '\t' '$1 == "ping_avg_ms" { print $6 "\t" $7 }' "$out/ratios.tsv")"
  json_metric="$(jq -r '.ratios[] | select(.metric == "udp_1000_loss_pct") | .better_when + "\t" + .nvpn_minus_reference' "$out/comparison.json")"
  threshold_tcp_4="$(awk -F '\t' '$1 == "tcp_4_throughput" { print $3 "\t" $6 "\t" $7 }' "$out/thresholds.tsv")"
  threshold_status="$(jq -r '.threshold_status.status' "$out/comparison.json")"
  threshold_failures="$(jq -r '.threshold_status.failures' "$out/comparison.json")"
  effective_udp_delta="$(jq -r '.threshold_policy.effective_udp_loss_delta_pct' "$out/comparison.json")"
  stress_fields="$(awk -F '\t' '$1 == "nvpn" { print $6 "\t" $7 "\t" $8 "\t" $9 }' "$out/comparison.tsv")"
  stress_json="$(jq -r '.cpu_stress.nvpn.enabled, .cpu_stress.nvpn.remote_workers, .cpu_stress.reference.enabled' "$out/comparison.json" | paste -sd ':' -)"
  pipeline_fields="$(awk -F '\t' '$1 == "nvpn" { print $10 "\t" $11 }' "$out/comparison.tsv")"
  pipeline_json="$(jq -r '.pipeline_trace.mismatch, .pipeline_trace.nvpn.enabled, .pipeline_trace.nvpn.interval_secs, .pipeline_trace.reference.enabled' "$out/comparison.json" | paste -sd ':' -)"
  provenance_json="$(jq -r '.provenance.nvpn.run_env.extra_connect_env, .provenance.nvpn.local_fips_patch.enabled, .provenance.nvpn.local_fips_patch.git_head, .provenance.nvpn.local_fips_patch.dirty' "$out/comparison.json" | paste -sd ':' -)"

  assert_eq "$comparison_fields" "24" "Docker comparison field count"
  assert_eq "$ratio_fields" "7" "Docker ratio field count"
  assert_eq "$threshold_fields" "7" "Docker threshold field count"
  assert_eq "$tcp_ratio" $'120.0\t50.000' "Docker TCP single ratio"
  assert_eq "$ping_delta" $'200.0\t0.400' "Docker ping avg delta"
  assert_eq "$json_metric" $'lower\t-1.000' "Docker comparison JSON ratio"
  assert_eq "$threshold_tcp_4" $'fail\t>=90%\t80.0%' "Docker throughput threshold"
  assert_eq "$threshold_status" "fail" "Docker threshold JSON status"
  assert_eq "$threshold_failures" "2" "Docker threshold JSON failure count"
  assert_eq "$effective_udp_delta" "1" "Docker clean/default UDP loss threshold"
  assert_eq "$stress_fields" $'true\tremote\t0\t4' "Docker comparison stress columns"
  assert_eq "$stress_json" "true:4:false" "Docker comparison stress JSON"
  assert_eq "$pipeline_fields" $'true\t2' "Docker comparison pipeline columns"
  assert_eq "$pipeline_json" "true:true:2:false" "Docker comparison pipeline JSON"
  assert_eq "$provenance_json" "FIPS_LINUX_BULK_CONTAINERS=1:true:fipsabc:true" "Docker comparison provenance JSON"

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

  "$COMPARE_SCRIPT" "$dir/nvpn" "$dir/reference" "$out" >/dev/null

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

test_json_and_ping_parsers
test_summary_row
test_metadata_writer_records_cpu_stress
test_metadata_writer_records_pipeline_trace
test_metadata_writer_records_iperf_interval
test_metadata_writer_records_iperf_timeout
test_metadata_writer_records_guard_thresholds
test_metadata_writer_records_run_provenance
test_metadata_writer_records_direct_fmp_guard
test_pipeline_summary_helpers
test_nvpn_tun_write_summary_prefers_coalesced_frame_interval
test_docker_comparison_outputs
test_docker_benchmark_summary_guards_are_opt_in
test_docker_benchmark_summary_guards_accept_healthy_fixture
test_docker_comparison_relaxes_udp_bulk_loss_under_cpu_stress
test_docker_comparison_selects_wireguard_go_reference

printf 'docker benchmark summary self-test passed\n'
