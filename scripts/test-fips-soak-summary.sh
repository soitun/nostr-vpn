#!/usr/bin/env bash
# Local self-test for the Docker nvpn+FIPS soak summarizer.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARIZER="$ROOT_DIR/scripts/summarize-fips-soak-docker.sh"

fail() {
  printf 'nvpn+FIPS soak summary self-test failed: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  local got="$1"
  local want="$2"
  local label="$3"
  [[ "$got" == "$want" ]] || fail "$label: got '$got', want '$want'"
}

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cat >"$tmpdir/samples.ndjson" <<'EOF'
{"ts":"2026-06-12T10:00:00Z","ping":{"a_to_b":{"loss_percent":0,"avg_ms":1.1,"p95_ms":2.2,"p99_ms":3.3,"max_ms":4.4},"b_to_a":{"loss_percent":0,"avg_ms":1.2,"p95_ms":2.3,"p99_ms":3.4,"max_ms":4.5}},"iperf":{"forward_mbps":100,"forward_retrans":1,"reverse_mbps":90,"reverse_retrans":2},"cpu":{"node_a_percent":50,"node_b_percent":40},"fips":{"node_a_last_fips_seen_age_secs":1,"node_b_last_fips_seen_age_secs":2,"node_a_last_fips_control_seen_age_secs":1,"node_b_last_fips_control_seen_age_secs":2,"node_a_last_fips_data_seen_age_secs":1,"node_b_last_fips_data_seen_age_secs":2,"node_a_rekey_stuck_count":0,"node_b_rekey_stuck_count":0,"node_a_direct_probe_pending_count":0,"node_b_direct_probe_pending_count":0,"node_a_direct_probe_overdue_count":0,"node_b_direct_probe_overdue_count":0,"node_a_nostr_traversal_failures":0,"node_b_nostr_traversal_failures":0},"pipeline":{"node_a":{"fips":{"queue_wait_ms":{"fmp_worker_bulk_queue_wait":{"max_observed":{"p95_ms":1,"p99_ms":2,"max_ms":4,"allmax_ms":3}},"fmp_worker_priority_queue_wait":{"max_observed":{"p95_ms":0.1,"p99_ms":0.2,"max_ms":0.3,"allmax_ms":0.4}}},"seen":{"encrypt_worker_bulk_dropped":true,"fmp_aead_completion_aead_failed":true,"fsp_aead_completion_aead_failed":true,"fsp_aead_completion_epoch_mismatch":true},"max_rates_per_sec":{"encrypt_worker_bulk_dropped":5,"fmp_aead_completion_aead_failed":1,"fsp_aead_completion_aead_failed":2,"fsp_aead_completion_epoch_mismatch":4},"max_totals":{"encrypt_worker_bulk_dropped":10,"fmp_aead_completion_aead_failed":1,"fsp_aead_completion_aead_failed":2,"fsp_aead_completion_epoch_mismatch":4}},"nvpn":{"queue_wait_ms":{"nvpn_tun_to_mesh_queue_wait":{"max_observed":{"p95_ms":0.1,"p99_ms":0.2,"max_ms":0.3,"allmax_ms":0.4}}},"seen":{},"max_rates_per_sec":{},"max_totals":{}}},"node_b":{"fips":{"queue_wait_ms":{"decrypt_worker_bulk_queue_wait":{"max_observed":{"p95_ms":0.5,"p99_ms":0.6,"max_ms":0.7,"allmax_ms":0.8}}},"seen":{},"max_rates_per_sec":{},"max_totals":{}},"nvpn":{"queue_wait_ms":{},"seen":{},"max_rates_per_sec":{},"max_totals":{}}}}}
{"ts":"2026-06-12T10:01:00Z","ping":{"a_to_b":{"loss_percent":1,"avg_ms":1.5,"p95_ms":2.5,"p99_ms":3.5,"max_ms":4.5},"b_to_a":{"loss_percent":0,"avg_ms":1.6,"p95_ms":2.6,"p99_ms":3.6,"max_ms":4.6}},"iperf":{"forward_mbps":80,"forward_retrans":3,"reverse_mbps":95,"reverse_retrans":4},"cpu":{"node_a_percent":55,"node_b_percent":45},"fips":{"node_a_last_fips_seen_age_secs":3,"node_b_last_fips_seen_age_secs":4,"node_a_last_fips_control_seen_age_secs":3,"node_b_last_fips_control_seen_age_secs":4,"node_a_last_fips_data_seen_age_secs":5,"node_b_last_fips_data_seen_age_secs":6,"node_a_rekey_stuck_count":1,"node_b_rekey_stuck_count":0,"node_a_direct_probe_pending_count":2,"node_b_direct_probe_pending_count":0,"node_a_direct_probe_overdue_count":1,"node_b_direct_probe_overdue_count":0,"node_a_nostr_traversal_failures":1,"node_b_nostr_traversal_failures":0},"pipeline":{"node_a":{"fips":{"queue_wait_ms":{"endpoint_priority_event_wait":{"max_observed":{"p95_ms":0.4,"p99_ms":0.5,"max_ms":0.6,"allmax_ms":0.7}}},"seen":{},"max_rates_per_sec":{},"max_totals":{}},"nvpn":{"queue_wait_ms":{"nvpn_tun_to_mesh_queue_wait":{"max_observed":{"p95_ms":0.2,"p99_ms":0.3,"max_ms":0.4,"allmax_ms":0.5}}},"seen":{"nvpn_tun_to_mesh_bulk_dropped":true,"nvpn_tun_to_mesh_bulk_dropped_batches":true,"nvpn_tun_to_mesh_bulk_dropped_packet_cap":true},"max_rates_per_sec":{"nvpn_tun_to_mesh_bulk_dropped":2,"nvpn_tun_to_mesh_bulk_dropped_batches":1,"nvpn_tun_to_mesh_bulk_dropped_packet_cap":2},"max_totals":{"nvpn_tun_to_mesh_bulk_dropped":7,"nvpn_tun_to_mesh_bulk_dropped_batches":3,"nvpn_tun_to_mesh_bulk_dropped_packet_cap":7}}},"node_b":{"fips":{"queue_wait_ms":{"decrypt_worker_priority_queue_wait":{"max_observed":{"p95_ms":0.7,"p99_ms":0.8,"max_ms":0.9,"allmax_ms":1}}},"seen":{"transport_bulk_dropped":true},"max_rates_per_sec":{"transport_bulk_dropped":3},"max_totals":{"transport_bulk_dropped":8}},"nvpn":{"queue_wait_ms":{},"seen":{},"max_rates_per_sec":{},"max_totals":{}}}}}
EOF

"$SUMMARIZER" "$tmpdir" "$tmpdir/summary.tsv" "$tmpdir/summary.json" >/dev/null

[[ -s "$tmpdir/summary.tsv" ]] || fail "summary.tsv was not written"
[[ -s "$tmpdir/summary.json" ]] || fail "summary.json was not written"
assert_eq "$(wc -l <"$tmpdir/summary.tsv" | tr -d ' ')" "2" "summary TSV line count"

assert_eq "$(jq -r '.summary.sample_count' "$tmpdir/summary.json")" "2" "sample count"
assert_eq "$(jq -r '.summary.min_forward_mbps' "$tmpdir/summary.json")" "80" "min forward Mbps"
assert_eq "$(jq -r '.summary.max_ping_loss_percent' "$tmpdir/summary.json")" "1" "max ping loss"
assert_eq "$(jq -r '.summary.max_fips_control_seen_age_secs' "$tmpdir/summary.json")" "4" "max control age"
assert_eq "$(jq -r '.summary.max_fips_data_seen_age_secs' "$tmpdir/summary.json")" "6" "max data age"
assert_eq "$(jq -r '.summary.max_direct_probe_overdue_count' "$tmpdir/summary.json")" "1" "direct probe overdue count"
assert_eq "$(jq -r '.pipeline.node_a_fips_top_queue_wait' "$tmpdir/summary.json")" "fmp_worker_bulk_queue_wait:p95_ms=1,p99_ms=2,max_ms=4,allmax_ms=3" "node-a FIPS top wait"
assert_eq "$(jq -r '.pipeline.node_b_fips_top_priority_wait' "$tmpdir/summary.json")" "decrypt_worker_priority_queue_wait:p95_ms=0.7,p99_ms=0.8,max_ms=0.9,allmax_ms=1" "node-b FIPS top priority wait"
assert_eq "$(jq -r '.pipeline.node_a_fips_hard_events' "$tmpdir/summary.json")" "encrypt_worker_bulk_dropped:max_rate_per_sec=5,total=10;fmp_aead_completion_aead_failed:max_rate_per_sec=1,total=1;fsp_aead_completion_aead_failed:max_rate_per_sec=2,total=2;fsp_aead_completion_epoch_mismatch:max_rate_per_sec=4,total=4" "node-a FIPS hard events"
assert_eq "$(jq -r '.pipeline.node_a_nvpn_hard_events' "$tmpdir/summary.json")" "nvpn_tun_to_mesh_bulk_dropped:max_rate_per_sec=2,total=7;nvpn_tun_to_mesh_bulk_dropped_batches:max_rate_per_sec=1,total=3;nvpn_tun_to_mesh_bulk_dropped_packet_cap:max_rate_per_sec=2,total=7" "node-a nvpn hard events"

printf 'nvpn+FIPS soak summary self-test passed\n'
