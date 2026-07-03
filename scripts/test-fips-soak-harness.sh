#!/usr/bin/env bash
# Local self-tests for the Docker nvpn+FIPS soak harness helpers.
#
# These tests do not start Docker. They pin the ping parser and latency guard
# behavior that makes long soak samples useful for tail-latency regressions.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SOAK_SCRIPT="$ROOT_DIR/scripts/soak-fips-dataplane-docker.sh"

# shellcheck source=scripts/soak-fips-dataplane-docker.sh
source "$SOAK_SCRIPT"

fail() {
  printf 'nvpn+FIPS soak harness self-test failed: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  local got="$1"
  local want="$2"
  local label="$3"
  [[ "$got" == "$want" ]] || fail "$label: got '$got', want '$want'"
}

assert_contains() {
  local got="$1"
  local want="$2"
  local label="$3"
  [[ "$got" == *"$want"* ]] || fail "$label: missing '$want' in '$got'"
}

assert_file_contains() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  grep -Fq -- "$pattern" "$file" || fail "$label: missing '$pattern' in $file"
}

assert_file_not_contains() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  ! grep -Fq -- "$pattern" "$file" || fail "$label: unexpected '$pattern' in $file"
}

assert_fails_with() {
  local label="$1"
  local pattern="$2"
  shift 2
  local err
  err="$(mktemp)"
  if "$@" 2>"$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "$label: command unexpectedly passed"
  fi
  if ! grep -Fq "$pattern" "$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "$label: expected stderr to contain '$pattern'"
  fi
  rm -f "$err"
}

fixture_ping_output() {
  cat <<'EOF'
PING 198.51.100.1 (198.51.100.1) 56(84) bytes of data.
64 bytes from 198.51.100.1: icmp_seq=1 ttl=64 time=1.0 ms
64 bytes from 198.51.100.1: icmp_seq=2 ttl=64 time=2.0 ms
64 bytes from 198.51.100.1: icmp_seq=3 ttl=64 time=3.0 ms
64 bytes from 198.51.100.1: icmp_seq=4 ttl=64 time=4.0 ms
64 bytes from 198.51.100.1: icmp_seq=5 ttl=64 time=5.0 ms
64 bytes from 198.51.100.1: icmp_seq=6 ttl=64 time=6.0 ms
64 bytes from 198.51.100.1: icmp_seq=7 ttl=64 time=7.0 ms
64 bytes from 198.51.100.1: icmp_seq=8 ttl=64 time=8.0 ms
64 bytes from 198.51.100.1: icmp_seq=9 ttl=64 time=9.0 ms
64 bytes from 198.51.100.1: icmp_seq=10 ttl=64 time=10.0 ms
64 bytes from 198.51.100.1: icmp_seq=11 ttl=64 time=11.0 ms
64 bytes from 198.51.100.1: icmp_seq=12 ttl=64 time=12.0 ms
64 bytes from 198.51.100.1: icmp_seq=13 ttl=64 time=13.0 ms
64 bytes from 198.51.100.1: icmp_seq=14 ttl=64 time=14.0 ms
64 bytes from 198.51.100.1: icmp_seq=15 ttl=64 time=15.0 ms
64 bytes from 198.51.100.1: icmp_seq=16 ttl=64 time=16.0 ms
64 bytes from 198.51.100.1: icmp_seq=17 ttl=64 time=17.0 ms
64 bytes from 198.51.100.1: icmp_seq=18 ttl=64 time=18.0 ms
64 bytes from 198.51.100.1: icmp_seq=19 ttl=64 time=19.0 ms
64 bytes from 198.51.100.1: icmp_seq=20 ttl=64 time=100.0 ms

--- 198.51.100.1 ping statistics ---
20 packets transmitted, 20 received, 0% packet loss, time 1900ms
rtt min/avg/max/mdev = 1.000/5.000/100.000/20.000 ms
EOF
}

fixture_iperf_output() {
  cat <<'EOF'
{
  "end": {
    "sum_received": {
      "bits_per_second": 123000000
    },
    "sum_sent": {
      "retransmits": 7
    }
  }
}
EOF
}

test_ping_parser_percentiles() {
  local stats loss avg p95 p99 max
  stats="$(fixture_ping_output | parse_ping_stats)"
  read -r loss avg p95 p99 max <<<"$stats"

  assert_eq "$loss" "0" "packet loss"
  assert_eq "$avg" "5.000" "average latency"
  assert_eq "$p95" "19" "p95 latency"
  assert_eq "$p99" "100" "p99 latency"
  assert_eq "$max" "100.000" "max latency"
}

test_ping_thresholds() {
  assert_ping_stats_ok "fixture pass" 0 5 19 100 100

  assert_fails_with \
    "p95 threshold" \
    "fixture p95 ping p95 ms" \
    bash -c 'source "$1"; MAX_PING_P95_MS=18; assert_ping_stats_ok "fixture p95" 0 5 19 100 100' \
    bash "$SOAK_SCRIPT"

  assert_fails_with \
    "p99 threshold" \
    "fixture p99 ping p99 ms" \
    bash -c 'source "$1"; MAX_PING_P99_MS=99; assert_ping_stats_ok "fixture p99" 0 5 19 100 100' \
    bash "$SOAK_SCRIPT"
}

test_tail_drift_thresholds() {
  assert_float_drift_at_most 40 5 50 10 "fixture ping p95 ms"

  assert_fails_with \
    "p95 drift" \
    "fixture ping p95 ms" \
    bash -c 'source "$1"; assert_float_drift_at_most 60 5 50 10 "fixture ping p95 ms"' \
    bash "$SOAK_SCRIPT"

  assert_fails_with \
    "p99 drift" \
    "fixture ping p99 ms" \
    bash -c 'source "$1"; assert_float_drift_at_most 85 5 75 10 "fixture ping p99 ms"' \
    bash "$SOAK_SCRIPT"
}

test_counter_progress_policy() {
  assert_counter_advanced 10 "" "fixture"
  assert_counter_advanced 11 10 "fixture"

  assert_fails_with \
    "counter non-numeric" \
    "fixture counter is not numeric" \
    bash -c 'source "$1"; assert_counter_advanced "nan" 10 "fixture"' \
    bash "$SOAK_SCRIPT"

  assert_fails_with \
    "counter no-progress" \
    "fixture counter did not advance" \
    bash -c 'source "$1"; assert_counter_advanced 10 10 "fixture"' \
    bash "$SOAK_SCRIPT"
}

test_srtt_progress_policy() {
  local high_count=0

  record_srtt_progress fixture 12 high_count
  assert_eq "$high_count" "0" "good SRTT clears high counter"

  record_srtt_progress fixture 2500 high_count
  assert_eq "$high_count" "1" "single high SRTT is tolerated"

  record_srtt_progress fixture 9 high_count
  assert_eq "$high_count" "0" "good SRTT clears prior high sample"

  assert_fails_with \
    "consecutive high SRTT" \
    "fixture FIPS SRTT stayed above 1000ms" \
    bash -c 'source "$1"; high_count=2; record_srtt_progress fixture 2500 high_count' \
    bash "$SOAK_SCRIPT"
}

test_iperf_timeout_configuration() {
  local got

  got="$(
    NVPN_SOAK_IPERF_DURATION_SECS=4 \
      bash -c 'source "$1"; printf "%s" "$IPERF_TIMEOUT_SECS"' bash "$SOAK_SCRIPT"
  )"
  assert_eq "$got" "34" "soak iperf timeout follows iperf duration"

  got="$(
    NVPN_SOAK_IPERF_TIMEOUT_SECS=9 \
      bash -c 'source "$1"; printf "%s" "$IPERF_TIMEOUT_SECS"' bash "$SOAK_SCRIPT"
  )"
  assert_eq "$got" "9" "soak iperf timeout honors override"
}

test_iperf_probe_uses_container_timeout() {
  local args_path got
  args_path="$(mktemp)"

  got="$(
    BOB_TUNNEL_IP="198.51.100.1"
    IPERF_DURATION=3
    IPERF_TIMEOUT_SECS=11
    COMPOSE=(fixture_compose_timeout)

    fixture_compose_timeout() {
      printf '%s\n' "$*" >>"$args_path"
      fixture_iperf_output
    }

    iperf_probe forward
  )"

  assert_eq "$got" "123 7" "soak iperf probe parses fixture"
  assert_file_contains "$args_path" "exec -T node-a timeout --kill-after=5s 11 iperf3" "soak iperf probe timeout command"
  assert_file_contains "$args_path" " -t 3 " "soak iperf probe duration"

  rm -f "$args_path"
}

test_iperf_probe_timeout_is_reported() {
  local err
  err="$(mktemp)"

  if (
    BOB_TUNNEL_IP="198.51.100.1"
    IPERF_DURATION=3
    IPERF_TIMEOUT_SECS=2
    COMPOSE=(fixture_compose_timeout_failure)

    fixture_compose_timeout_failure() {
      return 124
    }

    iperf_probe reverse -R
  ) 2>"$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "soak iperf timeout unexpectedly passed"
  fi

  assert_file_contains "$err" "iperf reverse timed out after 2s" "soak iperf timeout message"
  rm -f "$err"
}

test_fips_liveness_policy() {
  local age
  age="$(fips_last_seen_age_secs 100 105)"
  assert_eq "$age" "5" "last seen age"

  age="$(fips_last_seen_age_secs 110 105)"
  assert_eq "$age" "0" "future last seen age clamps for artifact"

  (
    MAX_FIPS_LAST_SEEN_AGE_SECS=10
    MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=5
    assert_fips_liveness_fresh "fixture" 100 105
  )

  assert_fails_with \
    "missing last seen" \
    "fixture last_fips_seen_at is missing" \
    bash -c 'source "$1"; assert_fips_liveness_fresh fixture "" 105' \
    bash "$SOAK_SCRIPT"

  assert_fails_with \
    "stale last seen" \
    "fixture last_fips_seen_at is stale" \
    bash -c 'source "$1"; MAX_FIPS_LAST_SEEN_AGE_SECS=10; assert_fips_liveness_fresh fixture 100 111' \
    bash "$SOAK_SCRIPT"

  assert_fails_with \
    "future last seen" \
    "fixture last_fips_seen_at is 6s in the future" \
    bash -c 'source "$1"; MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=5; assert_fips_liveness_fresh fixture 111 105' \
    bash "$SOAK_SCRIPT"

  (
    MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS=10
    MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=5
    assert_fips_control_liveness_fresh "fixture" 100 105
  )

  assert_fails_with \
    "stale control last seen" \
    "fixture last_fips_control_seen_at is stale" \
    bash -c 'source "$1"; MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS=10; assert_fips_control_liveness_fresh fixture 100 111' \
    bash "$SOAK_SCRIPT"

  (
    MAX_FIPS_DATA_LAST_SEEN_AGE_SECS=10
    MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=5
    assert_fips_data_liveness_fresh "fixture" 100 105
  )

  assert_fails_with \
    "stale data last seen" \
    "fixture last_fips_data_seen_at is stale" \
    bash -c 'source "$1"; MAX_FIPS_DATA_LAST_SEEN_AGE_SECS=10; assert_fips_data_liveness_fresh fixture 100 111' \
    bash "$SOAK_SCRIPT"
}

test_rekey_stuck_policy() {
  local count
  count=0
  MAX_CONSECUTIVE_REKEY_SAMPLES=2
  record_rekey_progress "fixture" true "" count
  assert_eq "$count" "1" "rekey active count"
  record_rekey_progress "fixture" "" "" count
  assert_eq "$count" "0" "rekey inactive resets count"

  assert_fails_with \
    "rekey stuck threshold" \
    "fixture rekey state stayed active for 3 consecutive sample(s)" \
    bash -c 'source "$1"; MAX_CONSECUTIVE_REKEY_SAMPLES=2; count=0; record_rekey_progress "fixture" true false count; record_rekey_progress "fixture" "" true count; record_rekey_progress "fixture" true true count' \
    bash "$SOAK_SCRIPT"
}

test_direct_probe_overdue_policy() {
  local pending_count overdue_count
  pending_count=0
  overdue_count=0
  MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES=2
  record_direct_probe_progress "fixture" true 9999999999999 1000 pending_count overdue_count
  assert_eq "$pending_count" "1" "direct probe future retry pending count"
  assert_eq "$overdue_count" "0" "direct probe future retry not overdue"
  record_direct_probe_progress "fixture" true 900 1000 pending_count overdue_count
  assert_eq "$pending_count" "2" "direct probe overdue pending count"
  assert_eq "$overdue_count" "1" "direct probe overdue count"
  record_direct_probe_progress "fixture" false "" 1000 pending_count overdue_count
  assert_eq "$pending_count" "0" "direct probe inactive resets pending count"
  assert_eq "$overdue_count" "0" "direct probe inactive resets overdue count"

  assert_fails_with \
    "direct probe overdue threshold" \
    "fixture direct probe stayed overdue for 3 consecutive sample(s)" \
    bash -c 'source "$1"; MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES=2; pending_count=0; overdue_count=0; record_direct_probe_progress "fixture" true 1 1000 pending_count overdue_count; record_direct_probe_progress "fixture" true 2 1000 pending_count overdue_count; record_direct_probe_progress "fixture" true 3 1000 pending_count overdue_count' \
    bash "$SOAK_SCRIPT"
}

test_cpu_policy() {
  assert_cpu_ok node-a 42

  assert_fails_with \
    "CPU threshold" \
    "node-a daemon CPU %" \
    bash -c 'source "$1"; MAX_CPU_PERCENT=250; assert_cpu_ok node-a 251' \
    bash "$SOAK_SCRIPT"
}

test_docker_cpu_stress_wiring() {
  assert_file_contains "$SOAK_SCRIPT" "NVPN_DOCKER_CPU_STRESS=1" "CPU stress help"
  assert_file_contains "$SOAK_SCRIPT" "docker_bench_write_metadata fips-soak" "CPU stress metadata"
  assert_file_contains "$SOAK_SCRIPT" "docker_bench_start_cpu_stress" "CPU stress start hook"
  assert_file_contains "$SOAK_SCRIPT" "docker_bench_stop_cpu_stress" "CPU stress cleanup hook"
}

test_docker_dataplane_profile_reaches_daemon_env() {
  local got
  (
    NVPN_DOCKER_DATAPLANE_PROFILE=linux-vnet-lan
    NVPN_DOCKER_PLACEMENT_PROFILE=worker-open
    EXTRA_ENV="FIPS_DECRYPT_WORKERS=2"
    got="$(daemon_env)"
    assert_contains "$got" "NVPN_FIPS_LINUX_TUN_VNET=1" "daemon env linux vnet"
    assert_contains "$got" "NVPN_MESH_UNDERLAY_UDP_MTU=1472" "daemon env LAN MTU"
    assert_contains "$got" "FIPS_DECRYPT_WORKERS=2" "daemon env preserves soak extra env"
    assert_contains "$got" "NVPN_PIPELINE_TRACE=1" "daemon env pipeline trace"
    assert_contains "$got" "NVPN_PIPELINE_INTERVAL_SECS=15" "daemon env pipeline interval"
  )
}

test_sample_json_uses_file_inputs() {
  assert_file_contains "$SOAK_SCRIPT" "write_sample_json_file" "sample JSON temp-file helper"
  assert_file_contains "$SOAK_SCRIPT" "jq -Rsc" "pipeline parser stdin input"
  assert_file_contains "$SOAK_SCRIPT" "--slurpfile fips_pipeline_a" "sample JSON FIPS file input"
  assert_file_contains "$SOAK_SCRIPT" "--slurpfile nvpn_pipeline_a" "sample JSON nvpn file input"
  assert_file_not_contains "$SOAK_SCRIPT" '--arg lines "$lines"' "pipeline parser avoids large argv"
  assert_file_not_contains "$SOAK_SCRIPT" '--argjson fips_pipeline_a "$fips_pipeline_a"' "sample JSON avoids large argv"
  assert_file_not_contains "$SOAK_SCRIPT" '--argjson nvpn_pipeline_a "$nvpn_pipeline_a"' "sample JSON avoids large argv"
}

test_pipeline_queue_wait_parser() {
  local sample got
  sample="$(pipeline_lines_to_json '[pipe 10s] endpoint_command_wait=10/s avg=125.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms endpoint_priority_command_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms endpoint_bulk_command_wait=8/s avg=300.0us p50<=262.1us p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms endpoint_event_wait=10/s avg=250.0us p50<=262.1us p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms fmp_worker_queue_wait=10/s avg=250.0us p50<=262.1us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=4.2ms fmp_worker_priority_queue_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms fmp_worker_bulk_queue_wait=8/s avg=300.0us p50<=262.1us p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms fmp_linux_bulk_container_ready_wait=8/s avg=400.0us p50<=524.3us p95<=4.2ms p99<=8.4ms max<=16.8ms allmax=16.8ms decrypt_worker_queue_wait=10/s avg=150.0us p50<=262.1us p95<=524.3us p99<=1.0ms max<=2.1ms allmax=2.1ms decrypt_fallback_wait=10/s avg=200.0us p50<=262.1us p95<=1.0ms p99<=3.1ms max<=4.2ms allmax=4.2ms fsp_aead_worker_open_queue_wait=10/s avg=400.0us p50<=524.3us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=4.2ms fsp_aead_worker_open_completion_wait=10/s avg=700.0us p50<=1.0ms p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms decrypt_authenticated_session_priority_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms decrypt_fsp_worker_priority_queue_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms transport_queue_wait=10/s avg=125.0us p50<=131.1us p95<=524.3us p99<=1.0ms max<=2.1ms allmax=2.1ms transport_channel_wait=10/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms transport_rx_loop_wait=10/s avg=50.0us p50<=65.5us p95<=131.1us p99<=262.1us max<=524.3us allmax=524.3us udp_send_connected=10/s')"

  got="$(jq -r '.line_count' <<<"$sample")"
  assert_eq "$got" "1" "pipeline line count"

  got="$(jq -r '.queue_wait_ms.endpoint_command_wait.latest.p95_ms' <<<"$sample")"
  assert_eq "$got" "0.2621" "endpoint command wait p95 ms"

  got="$(jq -r '.queue_wait_ms.endpoint_priority_command_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "0.5243" "endpoint priority command wait p99 ms"

  got="$(jq -r '.queue_wait_ms.endpoint_bulk_command_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "4.2" "endpoint bulk command wait p99 ms"

  got="$(jq -r '.queue_wait_ms.endpoint_event_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "4.2" "endpoint event wait p99 ms"

  got="$(jq -r '.queue_wait_ms.fsp_aead_worker_open_queue_wait.latest.p95_ms' <<<"$sample")"
  assert_eq "$got" "1.0" "FSP AEAD worker-open queue wait p95 ms"

  got="$(jq -r '.queue_wait_ms.fsp_aead_worker_open_completion_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "4.2" "FSP AEAD worker-open completion wait p99 ms"

  got="$(jq -r '.queue_wait_ms.fmp_worker_queue_wait.latest.p95_ms' <<<"$sample")"
  assert_eq "$got" "1.0" "FIPS worker queue wait p95 ms"

  got="$(jq -r '.queue_wait_ms.fmp_worker_queue_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "2.1" "FIPS worker queue wait p99 ms"

  got="$(jq -r '.queue_wait_ms.fmp_worker_priority_queue_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "0.5243" "FIPS priority worker queue wait p99 ms"

  got="$(jq -r '.queue_wait_ms.fmp_worker_bulk_queue_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "4.2" "FIPS bulk worker queue wait p99 ms"

  got="$(jq -r '.queue_wait_ms.fmp_linux_bulk_container_ready_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "8.4" "Linux bulk container ready wait p99 ms"

  got="$(jq -r '.queue_wait_ms.decrypt_worker_queue_wait.latest.p95_ms' <<<"$sample")"
  assert_eq "$got" "0.5243" "FIPS decrypt worker queue wait p95 ms"

  got="$(jq -r '.queue_wait_ms.decrypt_fallback_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "3.1" "FIPS decrypt fallback wait p99 ms"

  got="$(jq -r '.queue_wait_ms.decrypt_authenticated_session_priority_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "0.5243" "authenticated session priority wait p99 ms"

  got="$(jq -r '.queue_wait_ms.decrypt_fsp_worker_priority_queue_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "0.5243" "FSP worker priority wait p99 ms"

  got="$(jq -r '.queue_wait_ms.transport_queue_wait.latest.p95_ms' <<<"$sample")"
  assert_eq "$got" "0.5243" "transport queue wait p95 ms"

  got="$(jq -r '.queue_wait_ms.transport_channel_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "0.5243" "transport channel wait p99 ms"

  got="$(jq -r '.queue_wait_ms.transport_rx_loop_wait.latest.p99_ms' <<<"$sample")"
  assert_eq "$got" "0.2621" "transport rx-loop-owned wait p99 ms"
}

test_pipeline_raw_selectors() {
  local sample got
  sample="$(
    pipeline_lines_to_json $'[pipe 10s] fmp_worker_batch_packets=48000/s udp_send_connected=48000/s fmp_worker_queue_wait=48000/s avg=40.0us p50<=65.5us p95<=131.1us p99<=262.1us max<=1.0ms allmax=1.0ms\n[pipe 10s] fmp_worker_batch_packets=12/s udp_send_connected=12/s fmp_worker_queue_wait=12/s avg=2.0ms p50<=2.1ms p95<=4.2ms p99<=8.4ms max<=16.8ms allmax=16.8ms'
  )"

  got="$(jq -r '.raw' <<<"$sample")"
  assert_eq \
    "$got" \
    "[pipe 10s] fmp_worker_batch_packets=12/s udp_send_connected=12/s fmp_worker_queue_wait=12/s avg=2.0ms p50<=2.1ms p95<=4.2ms p99<=8.4ms max<=16.8ms allmax=16.8ms" \
    "pipeline raw keeps latest line"

  got="$(jq -r '.load_raw' <<<"$sample")"
  assert_eq \
    "$got" \
    "[pipe 10s] fmp_worker_batch_packets=48000/s udp_send_connected=48000/s fmp_worker_queue_wait=48000/s avg=40.0us p50<=65.5us p95<=131.1us p99<=262.1us max<=1.0ms allmax=1.0ms" \
    "pipeline load raw selects highest dataplane rate"

  got="$(jq -r '.placement_raw' <<<"$sample")"
  assert_eq \
    "$got" \
    "[pipe 10s] fmp_worker_batch_packets=48000/s udp_send_connected=48000/s fmp_worker_queue_wait=48000/s avg=40.0us p50<=65.5us p95<=131.1us p99<=262.1us max<=1.0ms allmax=1.0ms" \
    "pipeline placement raw falls back to load-shaped line when no FSP bulk receive counters exist"

  got="$(jq -r '.peak_wait_raw' <<<"$sample")"
  assert_eq \
    "$got" \
    "[pipe 10s] fmp_worker_batch_packets=12/s udp_send_connected=12/s fmp_worker_queue_wait=12/s avg=2.0ms p50<=2.1ms p95<=4.2ms p99<=8.4ms max<=16.8ms allmax=16.8ms" \
    "pipeline peak wait raw selects worst wait line"
}

test_pipeline_fsp_owner_placement_guard() {
  local sample ack_sample got
  sample="$(
    pipeline_lines_to_json $'[pipe 10s] fmp_worker_batch_packets=90000/s decrypt_fsp_owner_same=0/s decrypt_fsp_owner_mismatch=13000/s decrypt_fsp_path_local=0/s decrypt_fsp_path_handoff=13000/s decrypt_fsp_path_helper=0/s decrypt_fsp_path_worker_open=0/s decrypt_worker_batch_priority_packets=26000/s decrypt_worker_batch_bulk_packets=0/s\n[pipe 10s] fmp_worker_batch_packets=48000/s decrypt_fsp_owner_same=0/s decrypt_fsp_owner_mismatch=48000/s decrypt_fsp_path_local=0/s decrypt_fsp_path_handoff=0/s decrypt_fsp_path_helper=0/s decrypt_fsp_path_worker_open=48000/s decrypt_worker_batch_bulk_packets=48000/s'
  )"
  ack_sample="$(
    pipeline_lines_to_json '[pipe 10s] fmp_worker_batch_packets=90000/s decrypt_fsp_owner_same=0/s decrypt_fsp_owner_mismatch=13000/s decrypt_fsp_path_local=0/s decrypt_fsp_path_handoff=13000/s decrypt_fsp_path_helper=0/s decrypt_fsp_path_worker_open=0/s decrypt_worker_batch_priority_packets=26000/s decrypt_worker_batch_bulk_packets=0/s'
  )"

  got="$(pipeline_fsp_owner_placement_line "$sample")"
  assert_eq \
    "$got" \
    "[pipe 10s] fmp_worker_batch_packets=48000/s decrypt_fsp_owner_same=0/s decrypt_fsp_owner_mismatch=48000/s decrypt_fsp_path_local=0/s decrypt_fsp_path_handoff=0/s decrypt_fsp_path_helper=0/s decrypt_fsp_path_worker_open=48000/s decrypt_worker_batch_bulk_packets=48000/s" \
    "FSP owner placement line prefers bulk receive over sender load"

  (
    EXPECT_FSP_OWNER_PLACEMENT=worker-open
    assert_expected_fsp_owner_placement_sample "fixture" "$sample"
  )
  (
    EXPECT_FSP_OWNER_PLACEMENT=mismatch
    assert_expected_fsp_owner_placement_sample "fixture" "$sample"
  )

  assert_fails_with \
    "placement mismatch" \
    "expected fixture FSP owner placement handoff, got kind=worker-open summary=owner=mismatch,path=worker-open" \
    bash -c 'source "$1"; EXPECT_FSP_OWNER_PLACEMENT=handoff; assert_expected_fsp_owner_placement_sample "fixture" "$2"' \
    bash "$SOAK_SCRIPT" "$sample"

  (
    EXPECT_FSP_OWNER_PLACEMENT=worker-open
    assert_expected_fsp_owner_placement_any_sample "node-a FIPS" "$sample" "node-b FIPS" "$ack_sample"
  )

  assert_fails_with \
    "placement validation" \
    "unknown expected FSP owner placement" \
    bash -c 'source "$1"; EXPECT_FSP_OWNER_PLACEMENT=bogus; validate_expected_fsp_owner_placement' \
    bash "$SOAK_SCRIPT"
}

test_pipeline_failure_artifact() {
  local sample artifact got output_dir_old soak_running_old iteration_old
  local had_soak_running=0 had_iteration=0
  output_dir_old="$OUTPUT_DIR"
  if [[ ${SOAK_RUNNING+x} ]]; then
    had_soak_running=1
    soak_running_old="$SOAK_RUNNING"
  fi
  if [[ ${iteration+x} ]]; then
    had_iteration=1
    iteration_old="$iteration"
  fi

  OUTPUT_DIR="$(mktemp -d)"
  SOAK_RUNNING=1
  iteration=7
  sample="$(pipeline_lines_to_json '[pipe 10s] udp_send_connected=10/s fmp_worker_queue_wait=10/s avg=250.0us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=4.2ms encrypt_worker_bulk_dropped=2/s total=4')"

  write_pipeline_failure_artifact "Fixture FIPS" "hard-events" "encrypt_worker_bulk_dropped" "$sample" 2>/dev/null
  artifact="$(find "$OUTPUT_DIR" -name 'pipeline-failure-*.json' -print)"
  [[ -n "$artifact" ]] || fail "pipeline failure artifact was not written"

  got="$(jq -r '.iteration' "$artifact")"
  assert_eq "$got" "7" "pipeline failure artifact iteration"

  got="$(jq -r '.label' "$artifact")"
  assert_eq "$got" "Fixture FIPS" "pipeline failure artifact label"

  got="$(jq -r '.sample.load_raw | startswith("[pipe 10s]")' "$artifact")"
  assert_eq "$got" "true" "pipeline failure artifact includes parsed sample"

  rm -rf "$OUTPUT_DIR"
  OUTPUT_DIR="$output_dir_old"
  if (( had_soak_running )); then
    SOAK_RUNNING="$soak_running_old"
  else
    unset SOAK_RUNNING
  fi
  if (( had_iteration )); then
    iteration="$iteration_old"
  else
    unset iteration
  fi
}

test_pipeline_freshness_policy() {
  local previous stale sample
  previous=""
  stale=0
  sample="$(pipeline_lines_to_json $'[pipe 10s] udp_send_connected=1/s\n[pipe 10s] udp_send_connected=2/s')"
  assert_pipeline_fresh "fixture" "$sample" previous stale
  assert_eq "$previous" "2" "pipeline freshness stores count"
  assert_eq "$stale" "0" "pipeline freshness clears stale count"
  assert_pipeline_fresh "fixture" "$sample" previous stale
  assert_eq "$stale" "1" "pipeline freshness tolerates one stale sample"

  assert_fails_with \
    "stale pipeline summaries" \
    "fixture pipeline summaries did not advance" \
    bash -c 'source "$1"; MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES=1; previous=2; stale=1; sample="$(pipeline_lines_to_json "[pipe 10s] udp_send_connected=1/s
[pipe 10s] udp_send_connected=2/s")"; assert_pipeline_fresh "fixture" "$sample" previous stale' \
    bash "$SOAK_SCRIPT"
}

test_pipeline_queue_wait_thresholds() {
  local sample
  sample="$(pipeline_lines_to_json '[pipe 10s] endpoint_event_wait=10/s avg=1.0ms p50<=1.0ms p95<=60.0ms p99<=120.0ms max<=150.0ms allmax=150.0ms transport_queue_wait=10/s avg=125.0us p50<=131.1us p95<=524.3us p99<=1.0ms max<=2.1ms allmax=2.1ms')"

  assert_fails_with \
    "queue wait threshold" \
    "fixture queue wait exceeded threshold: endpoint_event_wait" \
    bash -c 'source "$1"; MAX_PIPELINE_QUEUE_WAIT_P95_MS=50; MAX_PIPELINE_QUEUE_WAIT_P99_MS=100; assert_pipeline_ok "fixture" "$2"' \
    bash "$SOAK_SCRIPT" "$sample"

  (
    MAX_PIPELINE_QUEUE_WAIT_P95_MS=50
    MAX_PIPELINE_QUEUE_WAIT_P99_MS=100
    ALLOW_QUEUE_WAIT=1
    assert_pipeline_ok "fixture" "$sample"
  )
}

test_pipeline_priority_queue_wait_guard() {
  local sample
  sample="$(pipeline_lines_to_json '[pipe 10s] fmp_worker_bulk_queue_wait=10/s avg=10.0ms p50<=8.4ms p95<=60.0ms p99<=120.0ms max<=150.0ms allmax=150.0ms encrypt_worker_bulk_queue_full=5/s total=5 encrypt_worker_bulk_dropped=5/s total=5')"

  (
    ALLOW_QUEUE_EVENTS=1
    ALLOW_QUEUE_WAIT=1
    MAX_PRIORITY_QUEUE_WAIT_MS=50
    assert_pipeline_ok "fixture bulk pressure" "$sample"
  )

  sample="$(pipeline_lines_to_json '[pipe 10s] endpoint_priority_event_wait=1/s avg=20.0ms p50<=16.8ms p95<=33.6ms p99<=33.6ms max<=67.1ms allmax=67.1ms fmp_worker_bulk_queue_wait=10/s avg=10.0ms p50<=8.4ms p95<=60.0ms p99<=120.0ms max<=150.0ms allmax=150.0ms')"

  assert_fails_with \
    "priority queue wait guard" \
    "fixture priority wait priority queue wait exceeded threshold: endpoint_priority_event_wait:max=67.1ms,p99=33.6ms" \
    bash -c 'source "$1"; ALLOW_QUEUE_EVENTS=1; ALLOW_QUEUE_WAIT=1; MAX_PRIORITY_QUEUE_WAIT_MS=50; assert_pipeline_ok "fixture priority wait" "$2"' \
    bash "$SOAK_SCRIPT" "$sample"

  (
    ALLOW_QUEUE_EVENTS=1
    ALLOW_QUEUE_WAIT=1
    MAX_PRIORITY_QUEUE_WAIT_MS=0
    assert_pipeline_ok "fixture priority wait opt-out" "$sample"
  )
}

test_pipeline_hard_event_policy() {
  local sample got
  sample="$(pipeline_lines_to_json '[pipe 10s] connected_udp_activation_failed=1/s connected_udp_peer_cap_skipped=4/s connected_udp_fd_budget_skipped=7/s connected_udp_kernel_dropped=6/s total=6 encrypt_worker_queue_full=0/s total=0 encrypt_worker_bulk_queue_full=2/s total=2 decrypt_worker_bulk_dropped=2/s decrypt_fallback_backlog_high=1/s decrypt_fallback_priority_gated=1/s decrypt_fsp_bulk_queue_full_fallback=1/s decrypt_fsp_helper_window_fallback=1/s decrypt_fsp_open_worker_window_fallback=1/s decrypt_fsp_helper_queue_full_fallback=1/s decrypt_fsp_helper_completion_backlog_fallback=1/s decrypt_fsp_open_worker_completion_backlog_fallback=1/s fmp_aead_completion_aead_failed=1/s total=3 fsp_aead_completion_aead_failed=1/s total=2 fsp_aead_completion_epoch_mismatch=1/s total=4 decrypt_authenticated_session_bulk_dropped=1/s endpoint_event_backlog_high=1/s endpoint_event_bulk_dropped=5/s transport_channel_backlog_high=1/s transport_bulk_dropped=3/s udp_send_bulk_dropped=0/s total=0 nvpn_tun_to_mesh_bulk_dropped=2/s total=20 nvpn_tun_to_mesh_bulk_dropped_batches=1/s total=10 nvpn_tun_to_mesh_bulk_dropped_packet_cap=2/s total=20 nvpn_tun_to_mesh_bulk_dropped_channel_full=0/s total=0')"

  got="$(pipeline_hard_events "$sample")"
  assert_eq \
    "$got" \
    "connected_udp_activation_failed,connected_udp_kernel_dropped,decrypt_worker_bulk_dropped,decrypt_fsp_helper_window_fallback,decrypt_fsp_open_worker_window_fallback,decrypt_fsp_helper_queue_full_fallback,decrypt_fsp_helper_completion_backlog_fallback,decrypt_fsp_open_worker_completion_backlog_fallback,fmp_aead_completion_aead_failed,fsp_aead_completion_aead_failed,fsp_aead_completion_epoch_mismatch,endpoint_event_backlog_high,endpoint_event_bulk_dropped,transport_channel_backlog_high,transport_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped_batches,nvpn_tun_to_mesh_bulk_dropped_packet_cap" \
    "hard pipeline events"

  got="$(jq -r '.seen.decrypt_fallback_backlog_high' <<<"$sample")"
  assert_eq "$got" "true" "decrypt fallback backlog high seen flag"

  got="$(jq -r '.max_rates_per_sec.connected_udp_peer_cap_skipped' <<<"$sample")"
  assert_eq "$got" "4" "connected UDP peer-cap skipped max rate"

  got="$(jq -r '.max_rates_per_sec.connected_udp_fd_budget_skipped' <<<"$sample")"
  assert_eq "$got" "7" "connected UDP fd-budget skipped max rate"

  got="$(jq -r '.seen.connected_udp_fd_budget_skipped' <<<"$sample")"
  assert_eq "$got" "true" "connected UDP fd-budget skipped seen flag"

  got="$(jq -r '.seen.encrypt_worker_bulk_queue_full' <<<"$sample")"
  assert_eq "$got" "true" "encrypt worker bulk queue-full seen flag"

  got="$(jq -r '.seen.decrypt_authenticated_session_bulk_dropped' <<<"$sample")"
  assert_eq "$got" "true" "authenticated session bulk dropped seen flag"

  got="$(jq -r '.max_totals.fmp_aead_completion_aead_failed' <<<"$sample")"
  assert_eq "$got" "3" "FMP AEAD completion failed total"

  got="$(jq -r '.seen.fsp_aead_completion_aead_failed' <<<"$sample")"
  assert_eq "$got" "true" "FSP AEAD completion failed seen flag"

  got="$(jq -r '.max_totals.fsp_aead_completion_epoch_mismatch' <<<"$sample")"
  assert_eq "$got" "4" "FSP AEAD completion epoch mismatch total"

  got="$(jq -r '.rates_per_sec.transport_channel_backlog_high' <<<"$sample")"
  assert_eq "$got" "1" "transport channel backlog high rate"

  got="$(jq -r '.rates_per_sec.transport_bulk_dropped' <<<"$sample")"
  assert_eq "$got" "3" "transport bulk dropped rate"

  got="$(jq -r '.rates_per_sec.endpoint_event_bulk_dropped' <<<"$sample")"
  assert_eq "$got" "5" "endpoint event bulk dropped rate"

  assert_fails_with \
    "hard event policy" \
    "fixture observed hard pipeline events: connected_udp_activation_failed,connected_udp_kernel_dropped,decrypt_worker_bulk_dropped,decrypt_fsp_helper_window_fallback,decrypt_fsp_open_worker_window_fallback,decrypt_fsp_helper_queue_full_fallback,decrypt_fsp_helper_completion_backlog_fallback,decrypt_fsp_open_worker_completion_backlog_fallback,fmp_aead_completion_aead_failed,fsp_aead_completion_aead_failed,fsp_aead_completion_epoch_mismatch,endpoint_event_backlog_high,endpoint_event_bulk_dropped,transport_channel_backlog_high,transport_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped_batches,nvpn_tun_to_mesh_bulk_dropped_packet_cap" \
    bash -c 'source "$1"; ALLOW_QUEUE_EVENTS=0; assert_pipeline_ok "fixture" "$2"' \
    bash "$SOAK_SCRIPT" "$sample"

  (
    ALLOW_QUEUE_EVENTS=1
    assert_pipeline_ok "fixture" "$sample"
  )
}

test_pipeline_priority_hard_event_guard() {
  local sample got
  sample="$(pipeline_lines_to_json '[pipe 10s] encrypt_worker_priority_queue_full=1/s total=1 decrypt_worker_priority_dropped=0/s total=0 decrypt_fsp_priority_queue_full_fallback=2/s total=2 decrypt_authenticated_session_priority_dropped=0/s total=0')"

  got="$(pipeline_priority_hard_events "$sample")"
  assert_eq \
    "$got" \
    "encrypt_worker_priority_queue_full,decrypt_fsp_priority_queue_full_fallback" \
    "priority hard pipeline events"

  assert_fails_with \
    "priority hard event policy" \
    "fixture observed priority/control hard pipeline events: encrypt_worker_priority_queue_full,decrypt_fsp_priority_queue_full_fallback" \
    bash -c 'source "$1"; ALLOW_QUEUE_EVENTS=1; FAIL_ON_PRIORITY_HARD_EVENTS=1; assert_pipeline_ok "fixture" "$2"' \
    bash "$SOAK_SCRIPT" "$sample"

  (
    ALLOW_QUEUE_EVENTS=1
    FAIL_ON_PRIORITY_HARD_EVENTS=0
    assert_pipeline_ok "fixture priority hard opt-out" "$sample"
  )
}

test_ping_parser_percentiles
test_ping_thresholds
test_tail_drift_thresholds
test_counter_progress_policy
test_srtt_progress_policy
test_iperf_timeout_configuration
test_iperf_probe_uses_container_timeout
test_iperf_probe_timeout_is_reported
test_fips_liveness_policy
test_rekey_stuck_policy
test_direct_probe_overdue_policy
test_cpu_policy
test_docker_cpu_stress_wiring
test_docker_dataplane_profile_reaches_daemon_env
test_sample_json_uses_file_inputs
test_pipeline_queue_wait_parser
test_pipeline_raw_selectors
test_pipeline_fsp_owner_placement_guard
test_pipeline_failure_artifact
test_pipeline_freshness_policy
test_pipeline_queue_wait_thresholds
test_pipeline_priority_queue_wait_guard
test_pipeline_hard_event_policy
test_pipeline_priority_hard_event_guard

printf 'nvpn+FIPS soak harness self-test passed\n'
