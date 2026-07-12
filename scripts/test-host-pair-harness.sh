#!/usr/bin/env bash
# Local self-tests for the host-pair dataplane soak harness helpers.
#
# These tests do not contact a remote host. They pin parser and guard behavior
# that makes host/VM soak evidence meaningful: pipeline summaries must be real
# when required, hard queue/drop events must fail, and summary rows must not claim
# pipeline coverage from a configured path alone.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

# shellcheck source=scripts/soak-fips-dataplane-host-pair.sh
source "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

fail() {
  printf 'host-pair harness self-test failed: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  local got="$1"
  local want="$2"
  local label="$3"
  [[ "$got" == "$want" ]] || fail "$label: got '$got', want '$want'"
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle' in '$haystack'"
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

peer_status_fixture() {
  local reachable="$1"
  local transport_addr="$2"
  local srtt="${3:-3}"
  local srtt_age_ms="${4:-250}"
  jq -nc \
    --argjson reachable "$reachable" \
    --arg transport_addr "$transport_addr" \
    --argjson srtt "$srtt" \
    --argjson srtt_age_ms "$srtt_age_ms" \
    '{
      status_source: "daemon",
      daemon: {
        state: {
          peers: [
            {
              participant_pubkey: "peer-a",
              fips_endpoint_npub: "peer-a",
              reachable: $reachable,
              fips_transport_addr: $transport_addr,
              fips_srtt_ms: $srtt,
              fips_srtt_age_ms: $srtt_age_ms,
              tunnel_ip: "100.64.0.2/32"
            }
          ]
        }
      }
    }'
}

multi_peer_status_fixture() {
  jq -nc '{
    status_source: "daemon",
    daemon: {
      state: {
        peers: [
          {
            participant_pubkey: "peer-a",
            fips_endpoint_npub: "peer-a",
            reachable: true,
            fips_transport_addr: "198.51.100.10:51820",
            fips_srtt_ms: 3,
            fips_srtt_age_ms: 250,
            tunnel_ip: "100.64.0.2/32"
          },
          {
            participant_pubkey: "peer-b",
            fips_endpoint_npub: "peer-b",
            reachable: true,
            fips_transport_addr: "198.51.100.20:51820",
            fips_srtt_ms: 4,
            fips_srtt_age_ms: 250,
            tunnel_ip: "100.64.0.3/32"
          },
          {
            participant_pubkey: "peer-c",
            fips_endpoint_npub: "peer-c",
            reachable: true,
            fips_transport_addr: "198.51.100.20:51821",
            fips_srtt_ms: 5,
            fips_srtt_age_ms: 250,
            tunnel_ip: "100.64.0.4/32"
          }
        ]
      }
    }
  }'
}

test_pipeline_latest_line() {
  local log got
  log="$(mktemp)"
  cat >"$log" <<'EOF'
noise before
[pipe 10s] encrypt_worker_queue_full=0/s total=0
[nvpn-pipe 10s] nvpn_tun_to_mesh_bulk_dropped=0/s total=0
[pipe 20s] decrypt_worker_bulk_dropped=0/s total=0
EOF

  got="$(pipeline_latest_line local "$log" pipe)"
  assert_eq "$got" "[pipe 20s] decrypt_worker_bulk_dropped=0/s total=0" "latest FIPS pipeline line"

  got="$(pipeline_latest_line local "$log" nvpn-pipe)"
  assert_eq "$got" "[nvpn-pipe 10s] nvpn_tun_to_mesh_bulk_dropped=0/s total=0" "latest nvpn pipeline line"

  got="$(pipeline_line_count local "$log" pipe)"
  assert_eq "$got" "2" "FIPS pipeline line count"

  got="$(pipeline_line_count local "$log" nvpn-pipe)"
  assert_eq "$got" "1" "nvpn pipeline line count"

  rm -f "$log"
}

test_pipeline_hard_events() {
  local line got
  line="[pipe 10s] udp_send_backpressure=12/s encrypt_worker_queue_full=0/s total=0 decrypt_worker_bulk_dropped=2.5/s total=0 decrypt_fallback_backlog_high=1/s fmp_aead_completion_aead_failed=1/s total=2 fsp_aead_completion_aead_failed=0.5/s total=1 fsp_aead_completion_epoch_mismatch=0.4/s total=1 endpoint_bulk_fast_path_prepare_failed=1/s total=3 endpoint_bulk_fast_path_stage_full=0/s total=0 endpoint_bulk_fast_path_feedback_full=0.5/s total=4 endpoint_event_backlog_high=1/s endpoint_event_bulk_dropped=5/s transport_channel_backlog_high=1/s transport_bulk_dropped=3/s udp_send_bulk_dropped=0/s total=4 nvpn_tun_to_mesh_bulk_dropped=2/s total=8 nvpn_tun_to_mesh_bulk_dropped_batches=1/s total=4 nvpn_tun_to_mesh_bulk_dropped_channel_full=2/s total=8"
  got="$(pipeline_hard_events "$line")"
  assert_eq "$got" "decrypt_worker_bulk_dropped,fmp_aead_completion_aead_failed,fsp_aead_completion_aead_failed,fsp_aead_completion_epoch_mismatch,endpoint_bulk_fast_path_prepare_failed,endpoint_bulk_fast_path_feedback_full,endpoint_event_backlog_high,endpoint_event_bulk_dropped,transport_channel_backlog_high,transport_bulk_dropped,udp_send_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped,nvpn_tun_to_mesh_bulk_dropped_batches,nvpn_tun_to_mesh_bulk_dropped_channel_full" "hard pipeline events"

  line="[pipe 10s] encrypt_worker_queue_full=0/s total=0 decrypt_worker_bulk_dropped=0/s total=0"
  got="$(pipeline_hard_events "$line")"
  assert_eq "$got" "" "zero-rate hard events"

  line="[pipe 10s] connected_udp_activation_failed=0/s connected_udp_fd_budget_skipped=7/s encrypt_worker_queue_full=0/s total=0"
  got="$(pipeline_hard_events "$line")"
  assert_eq "$got" "connected_udp_activation_failed" "rounded-down hard event without total"
}

test_pipeline_ok_policy() {
  local line
  line="[pipe 10s] encrypt_worker_queue_full=1/s total=0"

  assert_fails_with \
    "hard pipeline event" \
    "observed hard pipeline events: encrypt_worker_queue_full" \
    bash -c 'source "$1"; ALLOW_QUEUE_EVENTS=0; assert_pipeline_ok "fixture" "$2"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" "$line"

  (
    ALLOW_QUEUE_EVENTS=1
    assert_pipeline_ok "fixture" "$line"
  )
}

test_pipeline_priority_hard_event_policy() {
  local line got
  line="[pipe 10s] encrypt_worker_priority_queue_full=1/s total=1 decrypt_worker_priority_dropped=0/s total=0 decrypt_fsp_priority_queue_full_fallback=2/s total=2 decrypt_authenticated_session_priority_dropped=0/s total=0"

  got="$(pipeline_priority_hard_events "$line")"
  assert_eq "$got" "encrypt_worker_priority_queue_full,decrypt_fsp_priority_queue_full_fallback" "priority hard pipeline events"

  assert_fails_with \
    "priority hard pipeline event" \
    "observed priority/control hard pipeline events: encrypt_worker_priority_queue_full,decrypt_fsp_priority_queue_full_fallback" \
    bash -c 'source "$1"; ALLOW_QUEUE_EVENTS=1; FAIL_ON_PRIORITY_HARD_EVENTS=1; assert_pipeline_ok "fixture" "$2"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" "$line"

  (
    ALLOW_QUEUE_EVENTS=1
    FAIL_ON_PRIORITY_HARD_EVENTS=0
    assert_pipeline_ok "fixture" "$line"
  )
}

test_pipeline_queue_wait_parser() {
  local line got
  line="[pipe 10s] endpoint_command_wait=10/s avg=125.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms endpoint_priority_command_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms endpoint_bulk_command_wait=8/s avg=300.0us p50<=262.1us p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms endpoint_event_wait=10/s avg=250.0us p50<=262.1us p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms fmp_worker_queue_wait=10/s avg=250.0us p50<=262.1us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=4.2ms fmp_worker_priority_queue_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms fmp_worker_bulk_queue_wait=8/s avg=300.0us p50<=262.1us p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms fmp_linux_bulk_container_ready_wait=8/s avg=400.0us p50<=524.3us p95<=4.2ms p99<=8.4ms max<=16.8ms allmax=16.8ms decrypt_worker_queue_wait=10/s avg=150.0us p50<=262.1us p95<=524.3us p99<=1.0ms max<=2.1ms allmax=2.1ms decrypt_fallback_wait=10/s avg=200.0us p50<=262.1us p95<=1.0ms p99<=3.1ms max<=4.2ms allmax=4.2ms fsp_aead_worker_open_queue_wait=10/s avg=400.0us p50<=524.3us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=4.2ms fsp_aead_worker_open_completion_wait=10/s avg=700.0us p50<=1.0ms p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms decrypt_authenticated_session_priority_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms decrypt_fsp_worker_priority_queue_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms transport_queue_wait=10/s avg=125.0us p50<=131.1us p95<=524.3us p99<=1.0ms max<=2.1ms allmax=2.1ms transport_channel_wait=10/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms transport_rx_loop_wait=10/s avg=50.0us p50<=65.5us p95<=131.1us p99<=262.1us max<=524.3us allmax=524.3us"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.endpoint_command_wait.p95_ms')"
  assert_eq "$got" "0.2621" "endpoint command wait p95 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.endpoint_priority_command_wait.p99_ms')"
  assert_eq "$got" "0.5243" "endpoint priority command wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.endpoint_bulk_command_wait.p99_ms')"
  assert_eq "$got" "4.2" "endpoint bulk command wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.endpoint_event_wait.p99_ms')"
  assert_eq "$got" "4.2" "endpoint event wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.fmp_worker_queue_wait.p95_ms')"
  assert_eq "$got" "1.0" "FIPS worker queue wait p95 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.fmp_worker_queue_wait.p99_ms')"
  assert_eq "$got" "2.1" "FIPS worker queue wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.fmp_worker_priority_queue_wait.p99_ms')"
  assert_eq "$got" "0.5243" "FIPS priority worker queue wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.fmp_worker_bulk_queue_wait.p99_ms')"
  assert_eq "$got" "4.2" "FIPS bulk worker queue wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.fmp_linux_bulk_container_ready_wait.p99_ms')"
  assert_eq "$got" "8.4" "Linux bulk container ready wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.decrypt_worker_queue_wait.p95_ms')"
  assert_eq "$got" "0.5243" "FIPS decrypt worker queue wait p95 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.decrypt_fallback_wait.p99_ms')"
  assert_eq "$got" "3.1" "FIPS decrypt fallback wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.fsp_aead_worker_open_queue_wait.p95_ms')"
  assert_eq "$got" "1.0" "FSP AEAD worker-open queue wait p95 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.fsp_aead_worker_open_completion_wait.p99_ms')"
  assert_eq "$got" "4.2" "FSP AEAD worker-open completion wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.decrypt_authenticated_session_priority_wait.p99_ms')"
  assert_eq "$got" "0.5243" "authenticated session priority wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.decrypt_fsp_worker_priority_queue_wait.p99_ms')"
  assert_eq "$got" "0.5243" "FSP worker priority wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.transport_queue_wait.p95_ms')"
  assert_eq "$got" "0.5243" "transport queue wait p95 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.transport_channel_wait.p99_ms')"
  assert_eq "$got" "0.5243" "transport channel wait p99 ms"

  got="$(pipeline_queue_wait_json "$line" | jq -r '.transport_rx_loop_wait.p99_ms')"
  assert_eq "$got" "0.2621" "transport rx-loop-owned wait p99 ms"
}

test_pipeline_queue_wait_policy() {
  local line
  line="[pipe 10s] endpoint_event_wait=10/s avg=1.0ms p50<=1.0ms p95<=60.0ms p99<=120.0ms max<=150.0ms allmax=150.0ms"

  assert_fails_with \
    "queue wait threshold" \
    "fixture queue wait exceeded threshold: endpoint_event_wait" \
    bash -c 'source "$1"; MAX_PIPELINE_QUEUE_WAIT_P95_MS=50; MAX_PIPELINE_QUEUE_WAIT_P99_MS=100; assert_pipeline_ok "fixture" "$2"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" "$line"

  (
    MAX_PIPELINE_QUEUE_WAIT_P95_MS=50
    MAX_PIPELINE_QUEUE_WAIT_P99_MS=100
    ALLOW_QUEUE_WAIT=1
    assert_pipeline_ok "fixture" "$line"
  )
}

test_pipeline_top_queue_wait_summary() {
  local line_a line_b got
  line_a="[pipe 10s] fmp_worker_queue_wait=20/s avg=1.0ms p50<=1.0ms p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms fmp_worker_bulk_queue_wait=20/s avg=1.0ms p50<=1.0ms p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms"
  line_b="[nvpn-pipe 10s] nvpn_tun_to_mesh_queue_wait=10/s avg=2.0ms p50<=2.1ms p95<=4.2ms p99<=8.4ms max<=16.8ms allmax=16.8ms"

  got="$(pipeline_queue_wait_top_summary "$line_a" "$line_b")"
  assert_eq "$got" "nvpn_tun_to_mesh_queue_wait:rate_per_sec=10,p95_ms=4.2,p99_ms=8.4,max_ms=16.8,allmax_ms=16.8" "top queue wait picks worst p99"

  got="$(pipeline_queue_wait_top_summary "$line_a" "")"
  assert_eq "$got" "fmp_worker_bulk_queue_wait:rate_per_sec=20,p95_ms=2.1,p99_ms=4.2,max_ms=8.4,allmax_ms=8.4" "top queue wait prefers lane-specific tie"

  line_a="[pipe 10s] fmp_worker_bulk_queue_wait=20/s avg=1.0ms p50<=1.0ms p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms fmp_linux_bulk_container_all_slots_wait=20/s avg=2.0ms p50<=2.1ms p95<=4.2ms p99<=16.8ms max<=33.6ms allmax=33.6ms"
  got="$(pipeline_queue_wait_top_summary "$line_a" "")"
  assert_eq "$got" "fmp_linux_bulk_container_all_slots_wait:rate_per_sec=20,p95_ms=4.2,p99_ms=16.8,max_ms=33.6,allmax_ms=33.6" "top queue wait includes Linux bulk container waits"

  line_a="[pipe 10s] fsp_aead_worker_open_completion_wait=5/s avg=7.0ms p50<=8.4ms p95<=16.8ms p99<=33.6ms max<=67.1ms allmax=67.1ms"
  got="$(pipeline_queue_wait_top_summary "$line_a" "")"
  assert_eq "$got" "fsp_aead_worker_open_completion_wait:rate_per_sec=5,p95_ms=16.8,p99_ms=33.6,max_ms=67.1,allmax_ms=67.1" "top queue wait includes FSP worker-open waits"
}

test_pipeline_priority_queue_wait_policy() {
  local line
  line="[pipe 10s] fmp_worker_bulk_queue_wait=10/s avg=10.0ms p50<=8.4ms p95<=60.0ms p99<=120.0ms max<=150.0ms allmax=150.0ms encrypt_worker_bulk_queue_full=5/s total=5 encrypt_worker_bulk_dropped=5/s total=5"

  (
    ALLOW_QUEUE_EVENTS=1
    ALLOW_QUEUE_WAIT=1
    MAX_PRIORITY_QUEUE_WAIT_MS=50
    assert_pipeline_ok "fixture bulk pressure" "$line"
  )

  line="[pipe 10s] endpoint_priority_event_wait=1/s avg=20.0ms p50<=16.8ms p95<=33.6ms p99<=33.6ms max<=67.1ms allmax=67.1ms fmp_worker_bulk_queue_wait=10/s avg=10.0ms p50<=8.4ms p95<=60.0ms p99<=120.0ms max<=150.0ms allmax=150.0ms"
  assert_fails_with \
    "priority queue wait threshold" \
    "fixture priority wait priority queue wait exceeded threshold: endpoint_priority_event_wait:max=67.1ms,p99=33.6ms" \
    bash -c 'source "$1"; ALLOW_QUEUE_EVENTS=1; ALLOW_QUEUE_WAIT=1; MAX_PRIORITY_QUEUE_WAIT_MS=50; assert_pipeline_ok "fixture priority wait" "$2"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" "$line"

  (
    ALLOW_QUEUE_EVENTS=1
    ALLOW_QUEUE_WAIT=1
    MAX_PRIORITY_QUEUE_WAIT_MS=0
    assert_pipeline_ok "fixture priority wait opt-out" "$line"
  )
}

test_required_pipeline_presence() {
  assert_fails_with \
    "missing required pipeline path" \
    "pipeline log required but no daemon log path" \
    bash -c 'source "$1"; REQUIRE_PIPELINE_LOGS=1; assert_pipeline_present_if_required "remote" "" "" ""' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

  assert_fails_with \
    "missing required pipeline line" \
    "no pipe/nvpn-pipe summary lines were found" \
    bash -c 'source "$1"; REQUIRE_PIPELINE_LOGS=1; assert_pipeline_present_if_required "remote" "/tmp/daemon.log" "" ""' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

  (
    REQUIRE_PIPELINE_LOGS=1
    assert_pipeline_present_if_required "remote" "/tmp/daemon.log" "[pipe 10s] ok=0/s" ""
  )
}

test_pipeline_freshness_policy() {
  local previous stale
  previous=""
  stale=0
  assert_pipeline_fresh "fixture" 2 previous stale
  assert_eq "$previous" "2" "pipeline freshness stores count"
  assert_eq "$stale" "0" "pipeline freshness clears stale count"
  assert_pipeline_fresh "fixture" 2 previous stale
  assert_eq "$stale" "1" "pipeline freshness tolerates one stale sample"

  assert_fails_with \
    "stale pipeline summaries" \
    "fixture pipeline summaries did not advance" \
    bash -c 'source "$1"; MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES=1; previous=2; stale=1; assert_pipeline_fresh "fixture" 2 previous stale' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
}

test_counter_progress_policy() {
  assert_counter_progress local 11 10 10 10
  assert_counter_progress local 10 11 10 10

  assert_fails_with \
    "counter no-progress" \
    "local FIPS byte counters did not advance" \
    bash -c 'source "$1"; assert_counter_progress local 10 10 10 10' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
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
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

  assert_fails_with \
    "stale last seen" \
    "fixture last_fips_seen_at is stale" \
    bash -c 'source "$1"; MAX_FIPS_LAST_SEEN_AGE_SECS=10; assert_fips_liveness_fresh fixture 100 111' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

  assert_fails_with \
    "future last seen" \
    "fixture last_fips_seen_at is 6s in the future" \
    bash -c 'source "$1"; MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=5; assert_fips_liveness_fresh fixture 111 105' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

  (
    MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS=10
    MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=5
    assert_fips_control_liveness_fresh "fixture" 100 105
    assert_fips_control_liveness_fresh "fixture" "" 105
    assert_fips_control_liveness_fresh "fixture" null 105
  )

  assert_fails_with \
    "stale control last seen" \
    "fixture last_fips_control_seen_at is stale" \
    bash -c 'source "$1"; MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS=10; assert_fips_control_liveness_fresh fixture 100 111' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

  (
    MAX_FIPS_DATA_LAST_SEEN_AGE_SECS=10
    MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=5
    assert_fips_data_liveness_fresh "fixture" 100 105
    assert_fips_data_liveness_fresh "fixture" "" 105
    assert_fips_data_liveness_fresh "fixture" null 105
  )

  assert_fails_with \
    "stale data last seen" \
    "fixture last_fips_data_seen_at is stale" \
    bash -c 'source "$1"; MAX_FIPS_DATA_LAST_SEEN_AGE_SECS=10; assert_fips_data_liveness_fresh fixture 100 111' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
}

test_direct_path_policy() {
  local direct_status fallback_status unreachable_status stale_srtt_status missing_srtt_age_status
  direct_status="$(peer_status_fixture true "198.51.100.10:51820")"
  fallback_status="$(peer_status_fixture true "203.0.113.99:51820")"
  unreachable_status="$(peer_status_fixture false "198.51.100.10:51820")"
  stale_srtt_status="$(peer_status_fixture true "198.51.100.10:51820" 42 120001)"
  missing_srtt_age_status="$(peer_status_fixture true "198.51.100.10:51820" 42 null)"

  (
    ALLOW_NON_DIRECT=0
    MAX_SRTT_AGE_MS=120000
    assert_peer_path "$direct_status" "peer-a" "198.51.100.10" "local"
  )

  assert_fails_with \
    "non-direct route rejected" \
    "route changed away from expected direct UDP path" \
    bash -c 'source "$1"; ALLOW_NON_DIRECT=0; assert_peer_path "$2" "peer-a" "198.51.100.10" "local"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" "$fallback_status"

  (
    ALLOW_NON_DIRECT=1
    assert_peer_path "$fallback_status" "peer-a" "198.51.100.10" "local"
  )

  (
    ALLOW_NON_DIRECT=0
    MAX_SRTT_AGE_MS=120000
    assert_peer_path "$missing_srtt_age_status" "peer-a" "198.51.100.10" "local"
  )

  assert_fails_with \
    "unreachable peer rejected" \
    "local peer is not reachable" \
    bash -c 'source "$1"; ALLOW_NON_DIRECT=0; assert_peer_path "$2" "peer-a" "198.51.100.10" "local"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" "$unreachable_status"

  assert_fails_with \
    "stale SRTT rejected" \
    "local FIPS SRTT age ms" \
    bash -c 'source "$1"; ALLOW_NON_DIRECT=0; MAX_SRTT_AGE_MS=120000; assert_peer_path "$2" "peer-a" "198.51.100.10" "local"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" "$stale_srtt_status"
}

test_select_peer_uses_expected_underlay_when_status_has_multiple_peers() {
  local status got
  status="$(multi_peer_status_fixture)"

  got="$(select_peer "$status" "" local "198.51.100.10")"
  assert_eq "$got" "peer-a" "expected-underlay peer selection"

  got="$(select_peer "$status" "peer-b" local "198.51.100.10")"
  assert_eq "$got" "peer-b" "explicit peer selector overrides expected underlay"

  assert_fails_with \
    "ambiguous expected-underlay peer selection" \
    "set the matching NVPN_HOST_PAIR_*_PEER" \
    bash -c 'source "$1"; select_peer "$2" "" local "198.51.100.20"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" "$status"
}

test_cpu_policy() {
  assert_float_at_most 42 250 "local daemon CPU %"

  assert_fails_with \
    "CPU threshold" \
    "local daemon CPU %" \
    bash -c 'source "$1"; assert_float_at_most 251 250 "local daemon CPU %"' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
}

test_open_fd_budget_policy() {
  MAX_OPEN_FD_GROWTH=8
  MAX_OPEN_FD_UTILIZATION_PERCENT=80

  assert_open_fd_budget 28 20 256 "fixture daemon"
  assert_open_fd_budget "" "" "" "old daemon"
  assert_fails_with \
    "open FD growth" \
    "open file descriptors grew by 9" \
    assert_open_fd_budget 29 20 256 "fixture daemon"
  assert_fails_with \
    "open FD utilization" \
    "open file descriptor utilization exceeds 80%" \
    assert_open_fd_budget 205 200 256 "fixture daemon"
}

test_cpu_stress_helpers() {
  local cmd got

  csv_has_token " local , remote " local || fail "csv_has_token did not match local"
  csv_has_token " local , remote " remote || fail "csv_has_token did not match remote"
  if csv_has_token " local , remote " both; then
    fail "csv_has_token matched absent token"
  fi

  cmd="$(cpu_stress_start_cmd 2 /tmp/nvpn-host-pair-cpu-stress.pids)"
  assert_contains "$cmd" "while :; do :; done" "CPU stress busy loop"
  assert_contains "$cmd" "/tmp/nvpn-host-pair-cpu-stress.pids" "CPU stress pid file"

  cmd="$(cpu_stress_stop_cmd /tmp/nvpn-host-pair-cpu-stress.pids)"
  assert_contains "$cmd" "kill" "CPU stress stop kills pids"
  assert_contains "$cmd" "rm -f" "CPU stress stop removes pid file"

  got="$(
    bash -c 'source "$1"; CPU_STRESS_WORKERS=3; cpu_stress_worker_count local' \
      bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
  )"
  assert_eq "$got" "3" "explicit CPU stress worker count"

  assert_fails_with \
    "invalid CPU stress worker count" \
    "NVPN_HOST_PAIR_CPU_STRESS_WORKERS must be a non-negative integer or auto" \
    bash -c 'source "$1"; CPU_STRESS_WORKERS=bad; cpu_stress_worker_count local' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

  assert_fails_with \
    "invalid CPU stress side" \
    "NVPN_HOST_PAIR_CPU_STRESS_SIDES must contain local, remote, or both" \
    bash -c 'source "$1"; CPU_STRESS_SIDES=remote,other; validate_cpu_stress_sides' \
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
}

test_metadata_cpu_stress_shape() {
  local dir got
  dir="$(mktemp -d)"

  OUTPUT_DIR="$dir"
  LOCAL_PEER="local-peer"
  REMOTE_PEER="remote-peer"
  LOCAL_TUNNEL_IP="100.64.0.1"
  REMOTE_TUNNEL_IP="100.64.0.2"
  EXPECTED_REMOTE_UNDERLAY_IP="198.51.100.10"
  EXPECTED_LOCAL_UNDERLAY_IP="198.51.100.20"
  CPU_STRESS=1
  CPU_STRESS_SIDES=both
  LOCAL_CPU_STRESS_WORKERS_STARTED=2
  REMOTE_CPU_STRESS_WORKERS_STARTED=3
  MAX_CONSECUTIVE_REKEY_SAMPLES=4
  MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES=5
  MAX_FIPS_LAST_SEEN_AGE_SECS=120
  MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS=45
  MAX_FIPS_DATA_LAST_SEEN_AGE_SECS=30
  MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS=6
  MAX_PRIORITY_QUEUE_WAIT_MS=40
  FAIL_ON_PRIORITY_HARD_EVENTS=1

  write_metadata

  got="$(jq -r '.local_peer' "$dir/metadata.json")"
  assert_eq "$got" "local-peer" "metadata local peer"
  got="$(jq -r '.remote_peer' "$dir/metadata.json")"
  assert_eq "$got" "remote-peer" "metadata remote peer"
  got="$(jq -r '.cpu_stress.enabled' "$dir/metadata.json")"
  assert_eq "$got" "true" "metadata CPU stress enabled"
  got="$(jq -r '.cpu_stress.sides' "$dir/metadata.json")"
  assert_eq "$got" "both" "metadata CPU stress sides"
  got="$(jq -r '.cpu_stress.local_workers' "$dir/metadata.json")"
  assert_eq "$got" "2" "metadata local CPU stress workers"
  got="$(jq -r '.cpu_stress.remote_workers' "$dir/metadata.json")"
  assert_eq "$got" "3" "metadata remote CPU stress workers"
  got="$(jq -r '.max_consecutive_rekey_samples' "$dir/metadata.json")"
  assert_eq "$got" "4" "metadata rekey stuck threshold"
  got="$(jq -r '.max_consecutive_direct_probe_overdue_samples' "$dir/metadata.json")"
  assert_eq "$got" "5" "metadata direct probe overdue threshold"
  got="$(jq -r '.max_fips_last_seen_age_secs' "$dir/metadata.json")"
  assert_eq "$got" "120" "metadata FIPS last seen age threshold"
  got="$(jq -r '.max_fips_control_last_seen_age_secs' "$dir/metadata.json")"
  assert_eq "$got" "45" "metadata FIPS control last seen age threshold"
  got="$(jq -r '.max_fips_data_last_seen_age_secs' "$dir/metadata.json")"
  assert_eq "$got" "30" "metadata FIPS data last seen age threshold"
  got="$(jq -r '.max_fips_last_seen_future_skew_secs' "$dir/metadata.json")"
  assert_eq "$got" "6" "metadata FIPS last seen future skew threshold"
  got="$(jq -r '.max_priority_queue_wait_ms' "$dir/metadata.json")"
  assert_eq "$got" "40" "metadata priority queue wait threshold"
  got="$(jq -r '.fail_on_priority_hard_events' "$dir/metadata.json")"
  assert_eq "$got" "true" "metadata priority hard-event policy"

  rm -rf "$dir"
}

test_preflight_rows_shape() {
  local dir got
  dir="$(mktemp -d)"

  OUTPUT_DIR="$dir"
  PREFLIGHT_ROWS=()
  PREFLIGHT_RESULT=0

  preflight_ok "local jq is available" >/dev/null
  preflight_missing "remote SSH is reachable" >/dev/null

  got="$(write_preflight_rows)"
  assert_eq "$got" "$dir/preflight.tsv" "preflight path"
  assert_eq "$(sed -n '1p' "$got")" $'status\tlabel\tdetail' "preflight header"
  assert_eq "$(sed -n '2p' "$got")" $'ok\tlocal jq is available\t' "preflight ok row"
  assert_eq "$(sed -n '3p' "$got")" $'missing\tremote SSH is reachable\t' "preflight missing row"
  assert_eq "$PREFLIGHT_RESULT" "1" "preflight missing sets result"

  rm -rf "$dir"
}

test_help_flag_exits_before_remote_checks() {
  local err
  err="$(mktemp)"
  if ! bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh" --help >/dev/null 2>"$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "help flag should exit successfully"
  fi
  if ! grep -Fq "usage: NVPN_HOST_PAIR_SSH=user@host scripts/soak-fips-dataplane-host-pair.sh" "$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "help flag should print usage"
  fi
  if grep -Fq "remote host is missing iperf3" "$err"; then
    cat "$err" >&2
    rm -f "$err"
    fail "help flag attempted remote checks"
  fi
  rm -f "$err"
}

test_preflight_daemon_peer_health_rows() {
  local status

  PREFLIGHT_ROWS=()
  PREFLIGHT_RESULT=0
  status="$(peer_status_fixture true "198.51.100.10:51820")"
  preflight_check_daemon_peer_health "$status" "local" >/dev/null
  assert_eq "${PREFLIGHT_ROWS[0]}" $'ok\tlocal daemon has at least one reachable FIPS peer\treachable_peers=1 total_peers=1' "reachable peer preflight row"
  assert_eq "${PREFLIGHT_ROWS[1]}" $'ok\tlocal daemon has at least one FIPS transport address\ttransport_peers=1 total_peers=1' "transport address preflight row"
  assert_eq "$PREFLIGHT_RESULT" "0" "healthy daemon peer preflight result"

  PREFLIGHT_ROWS=()
  PREFLIGHT_RESULT=0
  status="$(peer_status_fixture false "")"
  preflight_check_daemon_peer_health "$status" "remote" >/dev/null
  assert_eq "${PREFLIGHT_ROWS[0]}" $'missing\tremote daemon has at least one reachable FIPS peer\treachable_peers=0 total_peers=1' "missing reachable peer preflight row"
  assert_eq "${PREFLIGHT_ROWS[1]}" $'missing\tremote daemon has at least one FIPS transport address\ttransport_peers=0 total_peers=1' "missing transport address preflight row"
  assert_eq "$PREFLIGHT_RESULT" "1" "unhealthy daemon peer preflight result"
}

test_preflight_selected_peer_path_detail_rows() {
  local status

  PREFLIGHT_ROWS=()
  PREFLIGHT_RESULT=0
  MAX_SRTT_MS=1000
  MAX_SRTT_AGE_MS=120000
  status="$(peer_status_fixture true "198.51.100.10:51820" 42 250)"
  preflight_check_selected_peer_path_details "$status" "peer-a" "198.51.100.10" "local" >/dev/null
  assert_eq "${PREFLIGHT_ROWS[0]}" $'ok\tlocal selected peer is reachable\tpeer=peer-a reachable=true' "selected peer reachable row"
  assert_eq "${PREFLIGHT_ROWS[1]}" $'ok\tlocal selected peer uses expected direct path\tpeer=peer-a expected_ip=198.51.100.10 transport_addr=198.51.100.10:51820' "selected peer direct-path row"
  assert_eq "${PREFLIGHT_ROWS[2]}" $'ok\tlocal selected peer FIPS SRTT is within threshold\tpeer=peer-a srtt_ms=42 max_ms=1000' "selected peer SRTT threshold row"
  assert_eq "${PREFLIGHT_ROWS[3]}" $'ok\tlocal selected peer FIPS SRTT sample is fresh\tpeer=peer-a fips_srtt_age_ms=250 max_ms=120000' "selected peer SRTT freshness row"
  assert_eq "$PREFLIGHT_RESULT" "0" "healthy selected peer path result"

  PREFLIGHT_ROWS=()
  PREFLIGHT_RESULT=0
  status="$(peer_status_fixture true "198.51.100.10:51820" 2163 250)"
  preflight_check_selected_peer_path_details "$status" "peer-a" "198.51.100.10" "remote" >/dev/null
  assert_eq "${PREFLIGHT_ROWS[0]}" $'ok\tremote selected peer is reachable\tpeer=peer-a reachable=true' "high-SRTT reachable row"
  assert_eq "${PREFLIGHT_ROWS[1]}" $'ok\tremote selected peer uses expected direct path\tpeer=peer-a expected_ip=198.51.100.10 transport_addr=198.51.100.10:51820' "high-SRTT direct-path row"
  assert_eq "${PREFLIGHT_ROWS[2]}" $'missing\tremote selected peer FIPS SRTT is within threshold\tpeer=peer-a srtt_ms=2163 max_ms=1000' "high-SRTT threshold row"
  assert_eq "${PREFLIGHT_ROWS[3]}" $'ok\tremote selected peer FIPS SRTT sample is fresh\tpeer=peer-a fips_srtt_age_ms=250 max_ms=120000' "high-SRTT freshness row"
  assert_eq "$PREFLIGHT_RESULT" "1" "high-SRTT selected peer path result"

  PREFLIGHT_ROWS=()
  PREFLIGHT_RESULT=0
  status="$(peer_status_fixture true "198.51.100.10:51820" 42 120001)"
  preflight_check_selected_peer_path_details "$status" "peer-a" "198.51.100.10" "remote" >/dev/null
  assert_eq "${PREFLIGHT_ROWS[2]}" $'ok\tremote selected peer FIPS SRTT is within threshold\tpeer=peer-a srtt_ms=42 max_ms=1000' "stale-SRTT threshold row"
  assert_eq "${PREFLIGHT_ROWS[3]}" $'missing\tremote selected peer FIPS SRTT sample is fresh\tpeer=peer-a fips_srtt_age_ms=120001 max_ms=120000' "stale-SRTT freshness row"
  assert_eq "$PREFLIGHT_RESULT" "1" "stale selected peer path result"
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
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
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
    bash "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
}

test_sample_queue_wait_fields() {
  local sample got
  timestamp="2026-06-08T00:00:00Z"
  LOCAL_PEER="local-peer"
  REMOTE_PEER="remote-peer"
  LOCAL_TUNNEL_IP="100.64.0.1"
  REMOTE_TUNNEL_IP="100.64.0.2"
  LOCAL_TRANSPORT_ADDR="198.51.100.20:51820"
  REMOTE_TRANSPORT_ADDR="198.51.100.10:51820"
  LOCAL_SRTT=1
  REMOTE_SRTT=1
  LOCAL_BYTES_SENT=1
  LOCAL_BYTES_RECV=1
  REMOTE_BYTES_SENT=1
  REMOTE_BYTES_RECV=1
  LOCAL_LAST_MESH_SEEN_AT=200
  LOCAL_LAST_FIPS_SEEN_AT=200
  LOCAL_LAST_FIPS_SEEN_AGE_SECS=5
  LOCAL_LAST_FIPS_CONTROL_SEEN_AT=198
  LOCAL_LAST_FIPS_CONTROL_SEEN_AGE_SECS=7
  LOCAL_LAST_FIPS_DATA_SEEN_AT=200
  LOCAL_LAST_FIPS_DATA_SEEN_AGE_SECS=5
  LOCAL_LAST_HANDSHAKE_AT=200
  REMOTE_LAST_MESH_SEEN_AT=300
  REMOTE_LAST_FIPS_SEEN_AT=300
  REMOTE_LAST_FIPS_SEEN_AGE_SECS=7
  REMOTE_LAST_FIPS_CONTROL_SEEN_AT=296
  REMOTE_LAST_FIPS_CONTROL_SEEN_AGE_SECS=11
  REMOTE_LAST_FIPS_DATA_SEEN_AT=300
  REMOTE_LAST_FIPS_DATA_SEEN_AGE_SECS=7
  REMOTE_LAST_HANDSHAKE_AT=300
  PING_FORWARD_LOSS=0
  PING_FORWARD_AVG=1
  PING_FORWARD_P95=1
  PING_FORWARD_P99=1
  PING_FORWARD_MAX=1
  PING_REVERSE_LOSS=0
  PING_REVERSE_AVG=1
  PING_REVERSE_P95=1
  PING_REVERSE_P99=1
  PING_REVERSE_MAX=1
  IPERF_FORWARD_MBPS=1
  IPERF_FORWARD_RETRANS=0
  IPERF_REVERSE_MBPS=1
  IPERF_REVERSE_RETRANS=0
  LOCAL_REKEY_IN_PROGRESS=true
  LOCAL_REKEY_DRAINING=false
  LOCAL_CURRENT_K_BIT=true
  REMOTE_REKEY_IN_PROGRESS=false
  REMOTE_REKEY_DRAINING=true
  REMOTE_CURRENT_K_BIT=false
  LOCAL_REKEY_STUCK_COUNT=2
  REMOTE_REKEY_STUCK_COUNT=1
  LOCAL_DIRECT_PROBE_PENDING=true
  LOCAL_DIRECT_PROBE_AFTER_MS=12345
  LOCAL_DIRECT_PROBE_RETRY_COUNT=4
  LOCAL_DIRECT_PROBE_AUTO_RECONNECT=true
  LOCAL_DIRECT_PROBE_EXPIRES_AT_MS=67890
  LOCAL_DIRECT_PROBE_PENDING_COUNT=3
  LOCAL_DIRECT_PROBE_OVERDUE_COUNT=2
  LOCAL_NOSTR_TRAVERSAL_FAILURES=3
  LOCAL_NOSTR_TRAVERSAL_IN_COOLDOWN=true
  LOCAL_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS=45678
  LOCAL_NOSTR_TRAVERSAL_LAST_SKEW_MS=-250
  REMOTE_DIRECT_PROBE_PENDING=false
  REMOTE_DIRECT_PROBE_AFTER_MS=""
  REMOTE_DIRECT_PROBE_RETRY_COUNT=0
  REMOTE_DIRECT_PROBE_AUTO_RECONNECT=false
  REMOTE_DIRECT_PROBE_EXPIRES_AT_MS=""
  REMOTE_DIRECT_PROBE_PENDING_COUNT=0
  REMOTE_DIRECT_PROBE_OVERDUE_COUNT=0
  REMOTE_NOSTR_TRAVERSAL_FAILURES=0
  REMOTE_NOSTR_TRAVERSAL_IN_COOLDOWN=false
  REMOTE_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS=""
  REMOTE_NOSTR_TRAVERSAL_LAST_SKEW_MS=125
  LOCAL_CPU=1
  REMOTE_CPU=1
  FIPS_PIPELINE_LOCAL="[pipe 10s] endpoint_event_wait=10/s avg=250.0us p50<=262.1us p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms fmp_worker_queue_wait=10/s avg=250.0us p50<=262.1us p95<=1.0ms p99<=2.1ms max<=4.2ms allmax=4.2ms fmp_worker_priority_queue_wait=2/s avg=100.0us p50<=131.1us p95<=262.1us p99<=524.3us max<=1.0ms allmax=1.0ms fmp_worker_bulk_queue_wait=8/s avg=300.0us p50<=262.1us p95<=2.1ms p99<=4.2ms max<=8.4ms allmax=8.4ms decrypt_fallback_backlog_high=1/s endpoint_event_backlog_high=1/s endpoint_event_bulk_dropped=5/s transport_channel_backlog_high=1/s transport_bulk_dropped=3/s"
  FIPS_PIPELINE_REMOTE=""
  NVPN_PIPELINE_LOCAL=""
  NVPN_PIPELINE_REMOTE=""
  FIPS_PIPELINE_LOCAL_COUNT=4
  FIPS_PIPELINE_REMOTE_COUNT=0
  NVPN_PIPELINE_LOCAL_COUNT=2
  NVPN_PIPELINE_REMOTE_COUNT=0
  FIPS_PIPELINE_LOCAL_QUEUE_WAIT="$(pipeline_queue_wait_json "$FIPS_PIPELINE_LOCAL")"
  FIPS_PIPELINE_REMOTE_QUEUE_WAIT="$(pipeline_queue_wait_json "$FIPS_PIPELINE_REMOTE")"
  NVPN_PIPELINE_LOCAL_QUEUE_WAIT="$(pipeline_queue_wait_json "$NVPN_PIPELINE_LOCAL")"
  NVPN_PIPELINE_REMOTE_QUEUE_WAIT="$(pipeline_queue_wait_json "$NVPN_PIPELINE_REMOTE")"

  sample="$(write_sample 1 "$timestamp" "/tmp/local-status.json" "/tmp/remote-status.json")"
  got="$(jq -r '.pipeline.queue_wait_ms.fips_local.fmp_worker_queue_wait.p99_ms' <<<"$sample")"
  assert_eq "$got" "2.1" "sample FIPS queue wait p99 ms"

  got="$(jq -r '.pipeline.queue_wait_ms.fips_local.fmp_worker_priority_queue_wait.p99_ms' <<<"$sample")"
  assert_eq "$got" "0.5243" "sample FIPS priority queue wait p99 ms"

  got="$(jq -r '.pipeline.queue_wait_ms.fips_local.fmp_worker_bulk_queue_wait.p99_ms' <<<"$sample")"
  assert_eq "$got" "4.2" "sample FIPS bulk queue wait p99 ms"

  got="$(jq -r '.pipeline.queue_wait_ms.fips_local.endpoint_event_wait.p99_ms' <<<"$sample")"
  assert_eq "$got" "4.2" "sample endpoint event wait p99 ms"

  got="$(jq -r '.pipeline.line_counts.fips_local' <<<"$sample")"
  assert_eq "$got" "4" "sample FIPS pipeline line count"

  got="$(jq -r '.pipeline.line_counts.nvpn_local' <<<"$sample")"
  assert_eq "$got" "2" "sample nvpn pipeline line count"

  got="$(jq -r '.pipeline.hard_events | join(",")' <<<"$sample")"
  assert_eq "$got" "endpoint_event_backlog_high,endpoint_event_bulk_dropped,transport_channel_backlog_high,transport_bulk_dropped" "sample hard pipeline events"

  got="$(jq -r '.peers.local_rekey_in_progress' <<<"$sample")"
  assert_eq "$got" "true" "sample local rekey in progress"

  got="$(jq -r '.peers.remote_rekey_draining' <<<"$sample")"
  assert_eq "$got" "true" "sample remote rekey draining"

  got="$(jq -r '.peers.local_current_k_bit' <<<"$sample")"
  assert_eq "$got" "true" "sample local current k bit"

  got="$(jq -r '.peers.local_rekey_stuck_count' <<<"$sample")"
  assert_eq "$got" "2" "sample local rekey stuck count"

  got="$(jq -r '.peers.local_direct_probe_pending' <<<"$sample")"
  assert_eq "$got" "true" "sample local direct probe pending"

  got="$(jq -r '.peers.local_direct_probe_after_ms' <<<"$sample")"
  assert_eq "$got" "12345" "sample local direct probe after ms"

  got="$(jq -r '.peers.local_direct_probe_retry_count' <<<"$sample")"
  assert_eq "$got" "4" "sample local direct probe retry count"

  got="$(jq -r '.peers.local_direct_probe_auto_reconnect' <<<"$sample")"
  assert_eq "$got" "true" "sample local direct probe auto reconnect"

  got="$(jq -r '.peers.local_direct_probe_expires_at_ms' <<<"$sample")"
  assert_eq "$got" "67890" "sample local direct probe expiry"

  got="$(jq -r '.peers.local_direct_probe_pending_count' <<<"$sample")"
  assert_eq "$got" "3" "sample local direct probe pending count"

  got="$(jq -r '.peers.local_direct_probe_overdue_count' <<<"$sample")"
  assert_eq "$got" "2" "sample local direct probe overdue count"

  got="$(jq -r '.peers.local_last_fips_seen_at' <<<"$sample")"
  assert_eq "$got" "200" "sample local last FIPS seen timestamp"

  got="$(jq -r '.peers.local_last_fips_seen_age_secs' <<<"$sample")"
  assert_eq "$got" "5" "sample local last FIPS seen age"

  got="$(jq -r '.peers.local_last_fips_control_seen_at' <<<"$sample")"
  assert_eq "$got" "198" "sample local last FIPS control seen timestamp"

  got="$(jq -r '.peers.local_last_fips_control_seen_age_secs' <<<"$sample")"
  assert_eq "$got" "7" "sample local last FIPS control seen age"

  got="$(jq -r '.peers.remote_last_fips_data_seen_age_secs' <<<"$sample")"
  assert_eq "$got" "7" "sample remote last FIPS data seen age"

  got="$(jq -r '.peers.remote_last_handshake_at' <<<"$sample")"
  assert_eq "$got" "300" "sample remote last handshake timestamp"

  got="$(jq -r '.peers.local_nostr_traversal_failures' <<<"$sample")"
  assert_eq "$got" "3" "sample local Nostr traversal failures"

  got="$(jq -r '.peers.local_nostr_traversal_in_cooldown' <<<"$sample")"
  assert_eq "$got" "true" "sample local Nostr traversal cooldown"

  got="$(jq -r '.peers.local_nostr_traversal_cooldown_until_ms' <<<"$sample")"
  assert_eq "$got" "45678" "sample local Nostr traversal cooldown deadline"

  got="$(jq -r '.peers.local_nostr_traversal_last_skew_ms' <<<"$sample")"
  assert_eq "$got" "-250" "sample local Nostr traversal skew"

  got="$(jq -r '.peers.remote_nostr_traversal_last_skew_ms' <<<"$sample")"
  assert_eq "$got" "125" "sample remote Nostr traversal skew"
}

test_summary_pipeline_flag() {
  local summary last_row columns
  summary="$(mktemp)"
  SUMMARY="$summary"
  timestamp="2026-06-08T00:00:00Z"
  iteration=1
  PING_FORWARD_LOSS=0
  PING_FORWARD_AVG=1
  PING_FORWARD_P95=1
  PING_FORWARD_P99=1
  PING_FORWARD_MAX=1
  PING_REVERSE_LOSS=0
  PING_REVERSE_AVG=1
  PING_REVERSE_P95=1
  PING_REVERSE_P99=1
  PING_REVERSE_MAX=1
  IPERF_FORWARD_MBPS=1
  IPERF_FORWARD_RETRANS=0
  IPERF_REVERSE_MBPS=1
  IPERF_REVERSE_RETRANS=0
  LOCAL_SRTT=1
  REMOTE_SRTT=1
  LOCAL_BYTES_SENT=1
  LOCAL_BYTES_RECV=1
  REMOTE_BYTES_SENT=1
  REMOTE_BYTES_RECV=1
  LOCAL_LAST_FIPS_SEEN_AT=200
  LOCAL_LAST_FIPS_SEEN_AGE_SECS=5
  LOCAL_LAST_FIPS_CONTROL_SEEN_AT=198
  LOCAL_LAST_FIPS_CONTROL_SEEN_AGE_SECS=7
  LOCAL_LAST_FIPS_DATA_SEEN_AT=200
  LOCAL_LAST_FIPS_DATA_SEEN_AGE_SECS=5
  REMOTE_LAST_FIPS_SEEN_AT=300
  REMOTE_LAST_FIPS_SEEN_AGE_SECS=7
  REMOTE_LAST_FIPS_CONTROL_SEEN_AT=296
  REMOTE_LAST_FIPS_CONTROL_SEEN_AGE_SECS=11
  REMOTE_LAST_FIPS_DATA_SEEN_AT=300
  REMOTE_LAST_FIPS_DATA_SEEN_AGE_SECS=7
  LOCAL_CPU=1
  REMOTE_CPU=1
  EXPECTED_REMOTE_UNDERLAY_IP=198.51.100.10
  EXPECTED_LOCAL_UNDERLAY_IP=198.51.100.20
  ALLOW_NON_DIRECT=0
  IPERF_FORWARD_COLLAPSE_COUNT=0
  IPERF_REVERSE_COLLAPSE_COUNT=0
  LOCAL_REKEY_IN_PROGRESS=true
  LOCAL_REKEY_DRAINING=false
  LOCAL_CURRENT_K_BIT=true
  LOCAL_REKEY_STUCK_COUNT=2
  REMOTE_REKEY_IN_PROGRESS=false
  REMOTE_REKEY_DRAINING=true
  REMOTE_CURRENT_K_BIT=false
  REMOTE_REKEY_STUCK_COUNT=1
  LOCAL_DIRECT_PROBE_PENDING=true
  LOCAL_DIRECT_PROBE_AFTER_MS=12345
  LOCAL_DIRECT_PROBE_RETRY_COUNT=4
  LOCAL_DIRECT_PROBE_AUTO_RECONNECT=true
  LOCAL_DIRECT_PROBE_EXPIRES_AT_MS=67890
  LOCAL_DIRECT_PROBE_PENDING_COUNT=3
  LOCAL_DIRECT_PROBE_OVERDUE_COUNT=2
  REMOTE_DIRECT_PROBE_PENDING=false
  REMOTE_DIRECT_PROBE_AFTER_MS=98765
  REMOTE_DIRECT_PROBE_RETRY_COUNT=0
  REMOTE_DIRECT_PROBE_AUTO_RECONNECT=false
  REMOTE_DIRECT_PROBE_EXPIRES_AT_MS=87654
  REMOTE_DIRECT_PROBE_PENDING_COUNT=0
  REMOTE_DIRECT_PROBE_OVERDUE_COUNT=0
  LOCAL_NOSTR_TRAVERSAL_FAILURES=3
  LOCAL_NOSTR_TRAVERSAL_IN_COOLDOWN=true
  LOCAL_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS=45678
  LOCAL_NOSTR_TRAVERSAL_LAST_SKEW_MS=-250
  REMOTE_NOSTR_TRAVERSAL_FAILURES=0
  REMOTE_NOSTR_TRAVERSAL_IN_COOLDOWN=false
  REMOTE_NOSTR_TRAVERSAL_COOLDOWN_UNTIL_MS=87654
  REMOTE_NOSTR_TRAVERSAL_LAST_SKEW_MS=125

  FIPS_PIPELINE_LOCAL=""
  FIPS_PIPELINE_REMOTE=""
  NVPN_PIPELINE_LOCAL=""
  NVPN_PIPELINE_REMOTE=""
  write_summary_row
  last_row="$(tail -n1 "$summary")"
  IFS=$'\t' read -r -a columns <<<"$last_row"
  assert_eq "${columns[24]}" "1" "direct-path summary flag"
  assert_eq "${columns[25]}" "0" "missing pipeline summary flag"
  assert_eq "${columns[26]}" "0" "first-sample counter-progress flag"
  assert_eq "${columns[29]}" "1" "FIPS liveness summary flag"
  assert_eq "${columns[30]}" "5" "local last-seen age summary field"
  assert_eq "${columns[31]}" "7" "remote last-seen age summary field"
  assert_eq "${columns[32]}" "1" "FIPS control liveness summary flag"
  assert_eq "${columns[33]}" "7" "local control last-seen age summary field"
  assert_eq "${columns[34]}" "11" "remote control last-seen age summary field"
  assert_eq "${columns[35]}" "1" "FIPS data liveness summary flag"
  assert_eq "${columns[36]}" "5" "local data last-seen age summary field"
  assert_eq "${columns[37]}" "7" "remote data last-seen age summary field"
  assert_eq "${columns[38]}" "true" "local rekey-in-progress summary field"
  assert_eq "${columns[41]}" "2" "local rekey stuck summary field"
  assert_eq "${columns[45]}" "1" "remote rekey stuck summary field"
  assert_eq "${columns[46]}" "true" "local direct-probe pending summary field"
  assert_eq "${columns[48]}" "4" "local direct-probe retry count summary field"
  assert_eq "${columns[49]}" "true" "local direct-probe auto-reconnect summary field"
  assert_eq "${columns[50]}" "67890" "local direct-probe expiry summary field"
  assert_eq "${columns[52]}" "2" "local direct-probe overdue summary field"
  assert_eq "${columns[60]}" "3" "local Nostr traversal failures summary field"
  assert_eq "${columns[63]}" "-250" "local Nostr traversal skew summary field"
  assert_eq "${columns[67]}" "125" "remote Nostr traversal skew summary field"
  assert_eq "${columns[68]:-}" "" "clean pipeline hard-events summary field"

  iteration=2
  FIPS_PIPELINE_LOCAL="[pipe 10s] endpoint_bulk_event_wait=5/s avg=1.0ms p50<=1.0ms p95<=2.1ms p99<=8.4ms max<=16.8ms allmax=16.8ms encrypt_worker_queue_full=0/s total=0 decrypt_fallback_backlog_high=1/s endpoint_event_backlog_high=1/s endpoint_event_bulk_dropped=5/s transport_channel_backlog_high=1/s transport_bulk_dropped=3/s"
  write_summary_row
  last_row="$(tail -n1 "$summary")"
  IFS=$'\t' read -r -a columns <<<"$last_row"
  assert_eq "${columns[25]}" "1" "present pipeline summary flag"
  assert_eq "${columns[26]}" "1" "later-sample counter-progress flag"
  assert_eq "${columns[68]}" "endpoint_event_backlog_high,endpoint_event_bulk_dropped,transport_channel_backlog_high,transport_bulk_dropped" "pipeline hard-events summary field"
  assert_eq "${columns[69]}" "endpoint_bulk_event_wait:rate_per_sec=5,p95_ms=2.1,p99_ms=8.4,max_ms=16.8,allmax_ms=16.8" "pipeline top queue-wait summary field"

  rm -f "$summary"
}

test_pipeline_latest_line
test_pipeline_hard_events
test_pipeline_ok_policy
test_pipeline_priority_hard_event_policy
test_pipeline_queue_wait_parser
test_pipeline_queue_wait_policy
test_pipeline_top_queue_wait_summary
test_pipeline_priority_queue_wait_policy
test_required_pipeline_presence
test_pipeline_freshness_policy
test_counter_progress_policy
test_fips_liveness_policy
test_direct_path_policy
test_select_peer_uses_expected_underlay_when_status_has_multiple_peers
test_cpu_policy
test_open_fd_budget_policy
test_cpu_stress_helpers
test_metadata_cpu_stress_shape
test_preflight_rows_shape
test_help_flag_exits_before_remote_checks
test_preflight_daemon_peer_health_rows
test_preflight_selected_peer_path_detail_rows
test_rekey_stuck_policy
test_direct_probe_overdue_policy
test_sample_queue_wait_fields
test_summary_pipeline_flag

printf 'host-pair harness self-test passed\n'
