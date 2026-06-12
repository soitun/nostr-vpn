#!/usr/bin/env bash
# Local self-test for host-pair benchmark comparison artifact parsing.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

fail() {
  printf 'host-pair comparison harness self-test failed: %s\n' "$*" >&2
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
  [[ "$haystack" == *"$needle"* ]] || fail "$label: '$haystack' does not contain '$needle'"
}

test_comparison_outputs() {
  local dir nvpn_dir wg_dir out_dir got enforce_output
  dir="$(mktemp -d)"
  nvpn_dir="$dir/nvpn"
  wg_dir="$dir/wg"
  out_dir="$dir/out"
  mkdir -p "$nvpn_dir" "$wg_dir"

  cat >"$nvpn_dir/summary.tsv" <<'EOF'
timestamp	iteration	ping_forward_loss_percent	ping_forward_avg_ms	ping_forward_p95_ms	ping_forward_p99_ms	ping_forward_max_ms	ping_reverse_loss_percent	ping_reverse_avg_ms	ping_reverse_p95_ms	ping_reverse_p99_ms	ping_reverse_max_ms	iperf_forward_mbps	iperf_forward_retrans	iperf_reverse_mbps	iperf_reverse_retrans	local_srtt_ms	remote_srtt_ms	local_bytes_sent	local_bytes_recv	remote_bytes_sent	remote_bytes_recv	local_cpu_percent	remote_cpu_percent	direct_path_checked	pipeline_log_checked	counter_progress_checked	iperf_forward_collapse_count	iperf_reverse_collapse_count	fips_liveness_checked	local_last_fips_seen_age_secs	remote_last_fips_seen_age_secs	fips_control_liveness_checked	local_last_fips_control_seen_age_secs	remote_last_fips_control_seen_age_secs	fips_data_liveness_checked	local_last_fips_data_seen_age_secs	remote_last_fips_data_seen_age_secs	local_rekey_in_progress	local_rekey_draining	local_current_k_bit	local_rekey_stuck_count	remote_rekey_in_progress	remote_rekey_draining	remote_current_k_bit	remote_rekey_stuck_count	local_direct_probe_pending	local_direct_probe_after_ms	local_direct_probe_retry_count	local_direct_probe_auto_reconnect	local_direct_probe_expires_at_ms	local_direct_probe_pending_count	local_direct_probe_overdue_count	remote_direct_probe_pending	remote_direct_probe_after_ms	remote_direct_probe_retry_count	remote_direct_probe_auto_reconnect	remote_direct_probe_expires_at_ms	remote_direct_probe_pending_count	remote_direct_probe_overdue_count	local_nostr_traversal_failures	local_nostr_traversal_in_cooldown	local_nostr_traversal_cooldown_until_ms	local_nostr_traversal_last_skew_ms	remote_nostr_traversal_failures	remote_nostr_traversal_in_cooldown	remote_nostr_traversal_cooldown_until_ms	remote_nostr_traversal_last_skew_ms	pipeline_hard_events	pipeline_top_queue_wait_local	pipeline_top_queue_wait_remote
2026-06-10T00:00:00Z	1	0	1.0	2.0	3.0	4.0	0	1.5	2.5	3.5	4.5	100	1	150	2	10	11	1000	2000	3000	4000	12.5	22.5	1	1	0	0	0	1	2	3	1	4	5	1	6	7	true	false	true	1	false	false	false	0	true	12345	1	true	22222	1	0	false	98765	0	false	33333	0	0	2	true	45678	-250	0	false	87654	125		fmp_worker_bulk_queue_wait:rate_per_sec=10,p95_ms=1,p99_ms=2.1,max_ms=4.2,allmax_ms=4.2	decrypt_worker_bulk_queue_wait:rate_per_sec=12,p95_ms=2.1,p99_ms=4.2,max_ms=8.4,allmax_ms=8.4
2026-06-10T00:00:10Z	2	0	1.1	2.1	3.1	4.1	0	1.6	2.6	3.6	4.6	200	3	300	4	10	11	2000	3000	4000	5000	13.5	23.5	1	1	1	0	0	1	8	9	1	10	11	1	12	13	false	true	false	2	true	false	true	1	true	22345	3	true	33333	3	2	false	88765	0	false	44444	0	0	3	true	55678	-125	1	false	97654	250	endpoint_event_backlog_high,decrypt_worker_bulk_dropped	fmp_worker_bulk_queue_wait:rate_per_sec=20,p95_ms=1,p99_ms=2.1,max_ms=4.2,allmax_ms=4.2	decrypt_worker_bulk_queue_wait:rate_per_sec=30,p95_ms=2.1,p99_ms=8.4,max_ms=16.8,allmax_ms=16.8
EOF
  cat >"$nvpn_dir/metadata.json" <<'JSON'
{
  "cpu_stress": {
    "enabled": true,
    "sides": "remote",
    "local_workers": 0,
    "remote_workers": 4
  }
}
JSON

  cat >"$wg_dir/summary.tsv" <<'EOF'
backend	threads	cpu_stress_enabled	cpu_stress_sides	local_cpu_stress_workers	remote_cpu_stress_workers	local_iface	remote_iface	ping_forward_loss_percent	ping_forward_avg_ms	ping_forward_p95_ms	ping_forward_p99_ms	ping_forward_max_ms	ping_reverse_loss_percent	ping_reverse_avg_ms	ping_reverse_p95_ms	ping_reverse_p99_ms	ping_reverse_max_ms	tcp_forward_mbps	tcp_forward_retrans	tcp_reverse_mbps	tcp_reverse_retrans	local_backend_cpu_percent	remote_backend_cpu_percent
wireguard-go	1	true	remote	0	4	utun77	wgbench77	0	0.8	1.5	2.0	3.0	0	0.9	1.6	2.1	3.1	400	5	400	6	8.0	16.0
EOF
  cat >"$wg_dir/metadata.json" <<'JSON'
{
  "backend": "wireguard-go"
}
JSON

  bash "$ROOT_DIR/scripts/compare-host-pair-benchmarks.sh" "$nvpn_dir" "$wg_dir" "$out_dir" >/dev/null

  [[ -f "$out_dir/comparison.tsv" ]] || fail "comparison.tsv was not written"
  [[ -f "$out_dir/ratios.tsv" ]] || fail "ratios.tsv was not written"
  [[ -f "$out_dir/thresholds.tsv" ]] || fail "thresholds.tsv was not written"
  [[ -f "$out_dir/comparison.json" ]] || fail "comparison.json was not written"

  got="$(awk -F '\t' '$1 == "nvpn" { print $9 }' "$out_dir/comparison.tsv")"
  assert_eq "$got" "200" "comparison uses latest nvpn forward Mbps"

  got="$(awk -F '\t' 'NR == 1 { header = NF } $1 == "nvpn" { print header ":" NF }' "$out_dir/comparison.tsv")"
  assert_eq "$got" "65:65" "comparison nvpn row field count"

  got="$(awk -F '\t' 'NR == 1 { header = NF } $1 == "reference" { print header ":" NF }' "$out_dir/comparison.tsv")"
  assert_eq "$got" "65:65" "comparison reference row field count"

  got="$(awk -F '\t' '$1 == "reference" { print $3 }' "$out_dir/comparison.tsv")"
  assert_eq "$got" "wireguard-go" "comparison reference backend"

  got="$(awk -F '\t' '$1 == "forward_mbps" { print $4 }' "$out_dir/ratios.tsv")"
  assert_eq "$got" "50.0" "forward Mbps ratio"

  got="$(awk -F '\t' '$1 == "reverse_mbps" { print $4 }' "$out_dir/ratios.tsv")"
  assert_eq "$got" "75.0" "reverse Mbps ratio"

  got="$(awk -F '\t' '$1 == "ping_forward_p99_ms" { print $5 }' "$out_dir/ratios.tsv")"
  assert_eq "$got" "1.1" "forward p99 delta in ratios TSV"

  got="$(awk -F '\t' '$1 == "ping_reverse_p99_ms" { print $5 }' "$out_dir/ratios.tsv")"
  assert_eq "$got" "1.5" "reverse p99 delta in ratios TSV"

  got="$(awk -F '\t' 'NR == 1 { header = NF } $1 == "forward_throughput" { print header ":" NF ":" $3 ":" $6 ":" $7 }' "$out_dir/thresholds.tsv")"
  assert_eq "$got" "7:7:fail:>=90%:50.0%" "forward throughput threshold row"

  got="$(awk -F '\t' '$1 == "reverse_throughput" { print $3 ":" $7 }' "$out_dir/thresholds.tsv")"
  assert_eq "$got" "fail:75.0%" "reverse throughput threshold row"

  got="$(awk -F '\t' '$1 == "ping_forward_p99" { print $3 ":" $6 ":" $7 }' "$out_dir/thresholds.tsv")"
  assert_eq "$got" "pass:<=reference+5ms:1.1ms" "forward p99 threshold row"

  got="$(jq -r '.ratios.forward_mbps_pct_of_reference' "$out_dir/comparison.json")"
  assert_eq "$got" "50.0" "JSON forward Mbps ratio"

  got="$(jq -r '.deltas.ping_forward_p99_ms_nvpn_minus_reference' "$out_dir/comparison.json")"
  assert_eq "$got" "1.1" "JSON forward p99 reference delta"

  got="$(jq -r '.deltas.ping_reverse_p99_ms_nvpn_minus_reference' "$out_dir/comparison.json")"
  assert_eq "$got" "1.5" "JSON reverse p99 reference delta"

  got="$(jq -r '.threshold_status.status' "$out_dir/comparison.json")"
  assert_eq "$got" "fail" "JSON threshold status"

  got="$(jq -r '.threshold_status.failures' "$out_dir/comparison.json")"
  assert_eq "$got" "2" "JSON threshold failure count"

  got="$(jq -r '.thresholds[] | select(.check == "forward_throughput") | .comparison' "$out_dir/comparison.json")"
  assert_eq "$got" "50.0%" "JSON forward throughput threshold comparison"

  got="$(jq -r '.threshold_policy.min_throughput_pct' "$out_dir/comparison.json")"
  assert_eq "$got" "90" "JSON threshold policy"

  got="$(jq -r '.nvpn.cpu_stress.remote_workers' "$out_dir/comparison.json")"
  assert_eq "$got" "4" "JSON nvpn CPU stress workers"

  got="$(awk -F '\t' '$1 == "nvpn" { print $19 }' "$out_dir/comparison.tsv")"
  assert_eq "$got" "1" "comparison preserves direct-path check flag"

  got="$(awk -F '\t' '$1 == "nvpn" { print $22 }' "$out_dir/comparison.tsv")"
  assert_eq "$got" "0" "comparison preserves forward collapse count"

  got="$(awk -F '\t' '$1 == "nvpn" { print $30 }' "$out_dir/comparison.tsv")"
  assert_eq "$got" "1" "comparison preserves nvpn data liveness flag"

  got="$(jq -r '.nvpn.safety_checks.counter_progress_checked' "$out_dir/comparison.json")"
  assert_eq "$got" "true" "JSON nvpn counter progress check flag"

  got="$(jq -r '.nvpn.safety_checks.pipeline_hard_events | join(",")' "$out_dir/comparison.json")"
  assert_eq "$got" "endpoint_event_backlog_high,decrypt_worker_bulk_dropped" "JSON nvpn hard pipeline events"

  got="$(jq -r '.nvpn.safety_checks.pipeline_top_queue_wait.local' "$out_dir/comparison.json")"
  assert_eq "$got" "fmp_worker_bulk_queue_wait:rate_per_sec=20,p95_ms=1,p99_ms=2.1,max_ms=4.2,allmax_ms=4.2" "JSON nvpn local top queue wait"

  got="$(jq -r '.nvpn.safety_checks.pipeline_top_queue_wait.remote' "$out_dir/comparison.json")"
  assert_eq "$got" "decrypt_worker_bulk_queue_wait:rate_per_sec=30,p95_ms=2.1,p99_ms=8.4,max_ms=16.8,allmax_ms=16.8" "JSON nvpn remote top queue wait"

  got="$(jq -r '.nvpn.safety_checks.iperf_reverse_collapse_count' "$out_dir/comparison.json")"
  assert_eq "$got" "0" "JSON nvpn reverse collapse count"

  got="$(jq -r '.nvpn.fips_liveness.remote_last_seen_age_secs' "$out_dir/comparison.json")"
  assert_eq "$got" "9" "JSON nvpn FIPS last-seen age"

  got="$(jq -r '.nvpn.fips_control_liveness.local_last_seen_age_secs' "$out_dir/comparison.json")"
  assert_eq "$got" "10" "JSON nvpn control liveness age"

  got="$(jq -r '.nvpn.fips_data_liveness.remote_last_seen_age_secs' "$out_dir/comparison.json")"
  assert_eq "$got" "13" "JSON nvpn data liveness age"

  got="$(jq -r '.nvpn.rekey.local_stuck_count' "$out_dir/comparison.json")"
  assert_eq "$got" "2" "JSON nvpn local rekey stuck count"

  got="$(jq -r '.nvpn.direct_probe.local_retry_count' "$out_dir/comparison.json")"
  assert_eq "$got" "3" "JSON nvpn direct probe retry count"

  got="$(jq -r '.nvpn.direct_probe.local_auto_reconnect' "$out_dir/comparison.json")"
  assert_eq "$got" "true" "JSON nvpn direct probe auto reconnect"

  got="$(jq -r '.nvpn.direct_probe.local_expires_at_ms' "$out_dir/comparison.json")"
  assert_eq "$got" "33333" "JSON nvpn direct probe expiry"

  got="$(jq -r '.nvpn.direct_probe.local_overdue_count' "$out_dir/comparison.json")"
  assert_eq "$got" "2" "JSON nvpn direct probe overdue count"

  got="$(jq -r '.nvpn.nostr_traversal.local_last_skew_ms' "$out_dir/comparison.json")"
  assert_eq "$got" "-125" "JSON nvpn traversal skew"

  if NVPN_HOST_PAIR_COMPARISON_ENFORCE_THRESHOLDS=1 bash "$ROOT_DIR/scripts/compare-host-pair-benchmarks.sh" "$nvpn_dir" "$wg_dir" "$dir/enforced" >"$dir/enforced.stdout" 2>"$dir/enforced.stderr"; then
    fail "host-pair comparison enforcement should fail on threshold violations"
  fi
  enforce_output="$(cat "$dir/enforced.stderr")"
  case "$enforce_output" in
    *"threshold status is fail"*) ;;
    *) fail "host-pair comparison enforcement stderr did not explain threshold failure: $enforce_output" ;;
  esac

  rm -rf "$dir"
}

write_comparison_json() {
  local path="$1"
  local nvpn_forward="$2"
  local nvpn_reverse="$3"
  local ref_forward="$4"
  local ref_reverse="$5"
  local forward_ratio="$6"
  local reverse_ratio="$7"
  local nvpn_ping_forward_p99="${8:-9.1}"
  local nvpn_ping_reverse_p99="${9:-9.2}"
  local ref_ping_forward_p99="${10:-7.1}"
  local ref_ping_reverse_p99="${11:-7.2}"
  local pipeline_hard_events_json="${12:-[]}"
  local ping_forward_delta ping_reverse_delta
  ping_forward_delta="$(awk -v actual="$nvpn_ping_forward_p99" -v reference="$ref_ping_forward_p99" 'BEGIN { printf "%.1f", actual - reference }')"
  ping_reverse_delta="$(awk -v actual="$nvpn_ping_reverse_p99" -v reference="$ref_ping_reverse_p99" 'BEGIN { printf "%.1f", actual - reference }')"
  mkdir -p "$(dirname "$path")"
  cat >"$path" <<JSON
{
  "artifacts": {
    "nvpn_dir": "/tmp/nvpn",
    "reference_dir": "/tmp/reference"
  },
  "nvpn": {
    "backend": "nvpn-fips",
    "cpu_stress": {
      "enabled": true,
      "sides": "remote",
      "local_workers": 0,
      "remote_workers": 4
    },
    "forward_mbps": $nvpn_forward,
    "reverse_mbps": $nvpn_reverse,
    "forward_retrans": 3,
    "reverse_retrans": 4,
    "ping_forward_p99_ms": $nvpn_ping_forward_p99,
    "ping_reverse_p99_ms": $nvpn_ping_reverse_p99,
    "local_cpu_percent": 13.5,
    "remote_cpu_percent": 23.5,
    "safety_checks": {
      "direct_path_checked": true,
      "pipeline_log_checked": true,
      "counter_progress_checked": true,
      "pipeline_hard_events": $pipeline_hard_events_json,
      "pipeline_top_queue_wait": {
        "local": "fmp_worker_bulk_queue_wait:rate_per_sec=20,p95_ms=1,p99_ms=2.1,max_ms=4.2,allmax_ms=4.2",
        "remote": "decrypt_worker_bulk_queue_wait:rate_per_sec=30,p95_ms=2.1,p99_ms=8.4,max_ms=16.8,allmax_ms=16.8"
      },
      "iperf_forward_collapse_count": 0,
      "iperf_reverse_collapse_count": 1
    },
    "fips_liveness": {
      "checked": true,
      "local_last_seen_age_secs": 2,
      "remote_last_seen_age_secs": 3
    },
    "fips_control_liveness": {
      "checked": true,
      "local_last_seen_age_secs": 4,
      "remote_last_seen_age_secs": 5
    },
    "fips_data_liveness": {
      "checked": true,
      "local_last_seen_age_secs": 6,
      "remote_last_seen_age_secs": 7
    },
    "rekey": {
      "local_in_progress": true,
      "local_draining": false,
      "local_current_k_bit": true,
      "local_stuck_count": 2,
      "remote_in_progress": false,
      "remote_draining": true,
      "remote_current_k_bit": false,
      "remote_stuck_count": 1
    },
    "direct_probe": {
      "local_pending": true,
      "local_after_ms": 22345,
      "local_retry_count": 3,
      "local_auto_reconnect": true,
      "local_expires_at_ms": 33333,
      "local_pending_count": 3,
      "local_overdue_count": 2,
      "remote_pending": false,
      "remote_after_ms": 88765,
      "remote_retry_count": 0,
      "remote_auto_reconnect": false,
      "remote_expires_at_ms": 44444,
      "remote_pending_count": 0,
      "remote_overdue_count": 0
    },
    "nostr_traversal": {
      "local_failures": 3,
      "local_in_cooldown": true,
      "local_cooldown_until_ms": 55678,
      "local_last_skew_ms": -125,
      "remote_failures": 1,
      "remote_in_cooldown": false,
      "remote_cooldown_until_ms": 97654,
      "remote_last_skew_ms": 250
    }
  },
  "reference": {
    "backend": "wireguard-go",
    "backend_threads": 1,
    "cpu_stress": {
      "enabled": true,
      "sides": "remote",
      "local_workers": 0,
      "remote_workers": 4
    },
    "forward_mbps": $ref_forward,
    "reverse_mbps": $ref_reverse,
    "forward_retrans": 5,
    "reverse_retrans": 6,
    "ping_forward_p99_ms": $ref_ping_forward_p99,
    "ping_reverse_p99_ms": $ref_ping_reverse_p99,
    "local_cpu_percent": 8.0,
    "remote_cpu_percent": 16.0
  },
  "ratios": {
    "forward_mbps_pct_of_reference": $forward_ratio,
    "reverse_mbps_pct_of_reference": $reverse_ratio
  },
  "deltas": {
    "ping_forward_p99_ms_nvpn_minus_reference": $ping_forward_delta,
    "ping_reverse_p99_ms_nvpn_minus_reference": $ping_reverse_delta
  },
  "threshold_policy": {
    "min_throughput_pct": 90,
    "max_retrans_pct": 150,
    "max_ping_p99_delta_ms": 5
  },
  "threshold_status": {
    "status": "fail",
    "failures": 2,
    "unknowns": 0
  },
  "thresholds": [
    {
      "check": "forward_throughput",
      "metric": "forward_mbps",
      "status": "fail",
      "nvpn": "$nvpn_forward",
      "reference": "$ref_forward",
      "threshold": ">=90%",
      "comparison": "$forward_ratio%"
    }
  ]
}
JSON
}

test_run_summary_outputs() {
  local dir got
  dir="$(mktemp -d)"
  mkdir -p "$dir/clean" "$dir/stress"
  cat >"$dir/manifest.tsv" <<EOF
mode	backend	cpu_stress_enabled	nvpn_dir	reference_dir	comparison_dir
clean	wireguard-go	0	$dir/clean/nvpn	$dir/clean/reference-wireguard-go	$dir/clean/comparison-wireguard-go
stress	wireguard-go	1	$dir/stress/nvpn	$dir/stress/reference-wireguard-go	$dir/stress/comparison-wireguard-go
EOF
  write_comparison_json "$dir/clean/comparison-wireguard-go/comparison.json" 200 300 400 500 50.0 60.0 9.1 9.2 7.1 7.2 '[]'
  write_comparison_json "$dir/stress/comparison-wireguard-go/comparison.json" 120 130 300 260 40.0 50.0 12.5 14.0 8.0 10.0 '["endpoint_event_backlog_high"]'

  bash "$ROOT_DIR/scripts/summarize-host-pair-comparison-run.sh" "$dir" >/dev/null
  [[ -f "$dir/matrix-summary.tsv" ]] || fail "matrix-summary.tsv was not written"
  [[ -f "$dir/matrix-stress-deltas.tsv" ]] || fail "matrix-stress-deltas.tsv was not written"
  [[ -f "$dir/matrix-reliability.tsv" ]] || fail "matrix-reliability.tsv was not written"
  [[ -f "$dir/matrix-reliability.json" ]] || fail "matrix-reliability.json was not written"
  [[ -f "$dir/matrix-summary.json" ]] || fail "matrix-summary.json was not written"

  got="$(awk -F '\t' '$1 == "clean" && $2 == "wireguard-go" { print $8 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "50.0" "clean forward ratio in matrix TSV"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $5 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "130" "stress nvpn reverse Mbps in matrix TSV"

  got="$(jq -r '.rows | length' "$dir/matrix-summary.json")"
  assert_eq "$got" "2" "matrix JSON row count"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .cpu_stress_enabled' "$dir/matrix-summary.json")"
  assert_eq "$got" "true" "matrix JSON stress flag"

  got="$(jq -r '.rows[] | select(.mode == "clean" and .backend == "wireguard-go") | .ratios.reverse_mbps_pct_of_reference' "$dir/matrix-summary.json")"
  assert_eq "$got" "60.0" "matrix JSON reverse ratio"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .threshold_status.status' "$dir/matrix-summary.json")"
  assert_eq "$got" "fail" "matrix JSON threshold status"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .thresholds[0].comparison' "$dir/matrix-summary.json")"
  assert_eq "$got" "40.0%" "matrix JSON threshold row"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "stress" && $2 == "wireguard-go" { print $h["nvpn_ping_forward_p99_delta_vs_reference_ms"] }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "4.5" "matrix TSV forward p99 reference delta"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "stress" && $2 == "wireguard-go" { print $h["nvpn_ping_reverse_p99_delta_vs_reference_ms"] }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "4.0" "matrix TSV reverse p99 reference delta"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .deltas.ping_forward_p99_ms_nvpn_minus_reference' "$dir/matrix-summary.json")"
  assert_eq "$got" "4.5" "matrix JSON forward p99 reference delta"

  got="$(awk -F '\t' '$1 == "clean" && $2 == "wireguard-go" { print $23 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "true" "matrix TSV direct-path flag"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $27 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "1" "matrix TSV reverse collapse count"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $36 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "7" "matrix TSV data liveness age"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .nvpn.safety_checks.iperf_reverse_collapse_count' "$dir/matrix-summary.json")"
  assert_eq "$got" "1" "matrix JSON collapse count"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "stress" && $2 == "wireguard-go" { print $h["nvpn_pipeline_top_queue_wait_local"] }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "fmp_worker_bulk_queue_wait:rate_per_sec=20,p95_ms=1,p99_ms=2.1,max_ms=4.2,allmax_ms=4.2" "matrix TSV local top queue wait"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .nvpn.safety_checks.pipeline_top_queue_wait.remote' "$dir/matrix-summary.json")"
  assert_eq "$got" "decrypt_worker_bulk_queue_wait:rate_per_sec=30,p95_ms=2.1,p99_ms=8.4,max_ms=16.8,allmax_ms=16.8" "matrix JSON remote top queue wait"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .nvpn.fips_control_liveness.remote_last_seen_age_secs' "$dir/matrix-summary.json")"
  assert_eq "$got" "5" "matrix JSON control liveness age"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $40 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "2" "matrix TSV local rekey stuck count"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $47 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "3" "matrix TSV direct probe retry count"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $48 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "true" "matrix TSV direct probe auto reconnect"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $49 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "33333" "matrix TSV direct probe expiry"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $51 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "2" "matrix TSV direct probe overdue count"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .nvpn.direct_probe.local_auto_reconnect' "$dir/matrix-summary.json")"
  assert_eq "$got" "true" "matrix JSON direct probe auto reconnect"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $62 }' "$dir/matrix-summary.tsv")"
  assert_eq "$got" "-125" "matrix TSV traversal skew"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .nvpn.nostr_traversal.remote_last_skew_ms' "$dir/matrix-summary.json")"
  assert_eq "$got" "250" "matrix JSON traversal skew"

  got="$(awk -F '\t' '$1 == "wireguard-go" { print $4 }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "60" "stress deltas TSV nvpn forward pct of clean"

  got="$(awk -F '\t' '$1 == "wireguard-go" { print $7 }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "43.3" "stress deltas TSV nvpn reverse pct of clean"

  got="$(awk -F '\t' '$1 == "wireguard-go" { print $10 }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "75" "stress deltas TSV reference forward pct of clean"

  got="$(awk -F '\t' '$1 == "wireguard-go" { print $16 }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "-10" "stress deltas TSV forward reference-ratio delta"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "wireguard-go" { print $h["clean_ping_forward_p99_nvpn_minus_reference_ms"] }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "2" "stress deltas TSV clean forward p99 reference delta"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "wireguard-go" { print $h["stress_ping_forward_p99_nvpn_minus_reference_ms"] }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "4.5" "stress deltas TSV stress forward p99 reference delta"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "wireguard-go" { print $h["ping_forward_p99_reference_delta_change_ms"] }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "2.5" "stress deltas TSV forward p99 reference delta change"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "wireguard-go" { print $h["clean_ping_reverse_p99_nvpn_minus_reference_ms"] }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "2" "stress deltas TSV clean reverse p99 reference delta"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "wireguard-go" { print $h["stress_ping_reverse_p99_nvpn_minus_reference_ms"] }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "4" "stress deltas TSV stress reverse p99 reference delta"

  got="$(awk -F '\t' 'NR == 1 { for (i = 1; i <= NF; i++) h[$i] = i; next } $1 == "wireguard-go" { print $h["ping_reverse_p99_reference_delta_change_ms"] }' "$dir/matrix-stress-deltas.tsv")"
  assert_eq "$got" "2" "stress deltas TSV reverse p99 reference delta change"

  got="$(jq -r '.stress_deltas | length' "$dir/matrix-summary.json")"
  assert_eq "$got" "1" "matrix JSON stress delta count"

  got="$(jq -r '.stress_deltas[0].nvpn.forward_mbps.stress_pct_of_clean' "$dir/matrix-summary.json")"
  assert_eq "$got" "60" "matrix JSON nvpn forward stress pct"

  got="$(jq -r '.stress_deltas[0].reference.reverse_mbps.stress_pct_of_clean' "$dir/matrix-summary.json")"
  assert_eq "$got" "52" "matrix JSON reference reverse stress pct"

  got="$(jq -r '.stress_deltas[0].ratios.reverse_mbps_pct_of_reference.stress_minus_clean_points' "$dir/matrix-summary.json")"
  assert_eq "$got" "-10" "matrix JSON reverse reference-ratio delta"

  got="$(jq -r '.stress_deltas[0].latency_deltas.ping_forward_p99_ms_nvpn_minus_reference.stress_minus_clean_ms' "$dir/matrix-summary.json")"
  assert_eq "$got" "2.5" "matrix JSON forward p99 reference delta change"

  got="$(jq -r '.stress_deltas[0].latency_deltas.ping_reverse_p99_ms_nvpn_minus_reference.stress_minus_clean_ms' "$dir/matrix-summary.json")"
  assert_eq "$got" "2" "matrix JSON reverse p99 reference delta change"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $4 }' "$dir/matrix-reliability.tsv")"
  assert_eq "$got" "fail" "reliability TSV stress status"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $5 }' "$dir/matrix-reliability.tsv")"
  assert_eq "$got" "5" "reliability TSV failure count"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $7 }' "$dir/matrix-reliability.tsv")"
  assert_contains "$got" "iperf_reverse_collapse" "reliability TSV collapse reason"
  assert_contains "$got" "local_direct_probe_overdue" "reliability TSV direct-probe overdue reason"
  assert_contains "$got" "pipeline_hard_event_endpoint_event_backlog_high" "reliability TSV pipeline hard-event reason"

  got="$(awk -F '\t' '$1 == "stress" && $2 == "wireguard-go" { print $8 }' "$dir/matrix-reliability.tsv")"
  assert_contains "$got" "local_nostr_traversal_cooldown" "reliability TSV traversal warning"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .status' "$dir/matrix-reliability.json")"
  assert_eq "$got" "fail" "reliability JSON stress status"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .failures | index("remote_rekey_stuck") != null' "$dir/matrix-reliability.json")"
  assert_eq "$got" "true" "reliability JSON rekey reason"

  got="$(jq -r '.rows[] | select(.mode == "stress" and .backend == "wireguard-go") | .failures | index("pipeline_hard_event_endpoint_event_backlog_high") != null' "$dir/matrix-reliability.json")"
  assert_eq "$got" "true" "reliability JSON pipeline hard-event reason"

  got="$(jq -r '.reliability | length' "$dir/matrix-summary.json")"
  assert_eq "$got" "2" "matrix summary JSON embeds reliability rows"

  got="$(jq -r '.artifacts.matrix_reliability_tsv' "$dir/matrix-summary.json")"
  assert_eq "$got" "$dir/matrix-reliability.tsv" "matrix summary JSON reliability TSV path"

  rm -rf "$dir"
}

test_comparison_outputs
test_run_summary_outputs

printf 'host-pair comparison harness self-test passed\n'
