#!/usr/bin/env bash
# Normalize an nvpn/FIPS host-pair artifact and a userspace WireGuard reference
# artifact into one small comparison bundle.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

NVPN_DIR="${NVPN_HOST_PAIR_COMPARISON_NVPN_DIR:-${NVPN_HOST_PAIR_ARTIFACT_DIR:-${1:-}}}"
REFERENCE_DIR="${NVPN_HOST_PAIR_COMPARISON_REFERENCE_DIR:-${NVPN_WG_HOST_PAIR_ARTIFACT_DIR:-${2:-}}}"
OUTPUT_DIR="${NVPN_HOST_PAIR_COMPARISON_OUTPUT_DIR:-${3:-$ROOT_DIR/artifacts/host-pair-comparisons/$(date -u +%Y%m%dT%H%M%SZ)}}"
MIN_THROUGHPUT_PCT="${NVPN_HOST_PAIR_COMPARISON_MIN_THROUGHPUT_PCT:-90}"
MAX_RETRANS_PCT="${NVPN_HOST_PAIR_COMPARISON_MAX_RETRANS_PCT:-150}"
MAX_PING_P99_DELTA_MS="${NVPN_HOST_PAIR_COMPARISON_MAX_PING_P99_DELTA_MS:-5}"
ENFORCE_THRESHOLDS="${NVPN_HOST_PAIR_COMPARISON_ENFORCE_THRESHOLDS:-0}"

die() {
  printf 'host-pair benchmark comparison failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
usage: scripts/compare-host-pair-benchmarks.sh <nvpn-host-pair-artifact-dir> <userspace-wg-artifact-dir> [output-dir]

Env alternatives:
  NVPN_HOST_PAIR_COMPARISON_NVPN_DIR=<dir>
  NVPN_HOST_PAIR_COMPARISON_REFERENCE_DIR=<dir>
  NVPN_HOST_PAIR_COMPARISON_OUTPUT_DIR=<dir>
  NVPN_HOST_PAIR_COMPARISON_MIN_THROUGHPUT_PCT=90
  NVPN_HOST_PAIR_COMPARISON_MAX_RETRANS_PCT=150
  NVPN_HOST_PAIR_COMPARISON_MAX_PING_P99_DELTA_MS=5
  NVPN_HOST_PAIR_COMPARISON_ENFORCE_THRESHOLDS=0

Inputs:
  <nvpn-dir>/summary.tsv from scripts/soak-fips-dataplane-host-pair.sh
  <wg-dir>/summary.tsv from scripts/bench-userspace-wg-host-pair.sh

Outputs:
  comparison.tsv, ratios.tsv, thresholds.tsv, comparison.json
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

summary_file() {
  local dir="$1"
  [[ -n "$dir" ]] || {
    usage
    die "artifact directory is required"
  }
  [[ -d "$dir" ]] || die "artifact directory not found: $dir"
  [[ -f "$dir/summary.tsv" ]] || die "missing summary.tsv in $dir"
  printf '%s/summary.tsv\n' "$dir"
}

tsv_value() {
  local file="$1"
  local field="$2"
  awk -v want="$field" -F '\t' '
    NR == 1 {
      for (i = 1; i <= NF; i++) {
        if ($i == want) {
          idx = i
        }
      }
      next
    }
    idx && NF {
      value = $idx
      seen = 1
    }
    END {
      if (!idx) {
        exit 2
      }
      if (!seen) {
        exit 3
      }
      print value
    }' "$file"
}

tsv_optional_value() {
  local file="$1"
  local field="$2"
  tsv_value "$file" "$field" 2>/dev/null || true
}

metadata_value() {
  local dir="$1"
  local filter="$2"
  local metadata="$dir/metadata.json"
  [[ -f "$metadata" ]] || {
    printf '\n'
    return
  }
  jq -r "$filter // \"\"" "$metadata"
}

ratio_percent() {
  local actual="$1"
  local reference="$2"
  awk -v actual="$actual" -v reference="$reference" '
    BEGIN {
      if (actual == "" || actual == "null" || reference == "" || reference == "null" || reference + 0 == 0) {
        print "";
      } else {
        printf "%.1f", (actual + 0) * 100.0 / (reference + 0);
      }
    }'
}

delta_value() {
  local actual="$1"
  local reference="$2"
  awk -v actual="$actual" -v reference="$reference" '
    BEGIN {
      if (actual == "" || actual == "null" || reference == "" || reference == "null") {
        print "";
      } else {
        printf "%.1f", (actual + 0) - (reference + 0);
      }
    }'
}

threshold_higher_pct_status() {
  local actual="$1"
  local reference="$2"
  local min_pct="$3"
  awk -v actual="$actual" -v reference="$reference" -v min_pct="$min_pct" '
    BEGIN {
      if (actual == "" || actual == "null" || reference == "" || reference == "null" || reference + 0 == 0 || min_pct == "") {
        print "unknown";
      } else if ((actual + 0) * 100.0 / (reference + 0) >= min_pct + 0) {
        print "pass";
      } else {
        print "fail";
      }
    }'
}

threshold_lower_pct_status() {
  local actual="$1"
  local reference="$2"
  local max_pct="$3"
  awk -v actual="$actual" -v reference="$reference" -v max_pct="$max_pct" '
    BEGIN {
      if (actual == "" || actual == "null" || reference == "" || reference == "null" || max_pct == "") {
        print "unknown";
      } else if (reference + 0 == 0) {
        print ((actual + 0) == 0 ? "pass" : "fail");
      } else if ((actual + 0) * 100.0 / (reference + 0) <= max_pct + 0) {
        print "pass";
      } else {
        print "fail";
      }
    }'
}

threshold_lower_delta_status() {
  local actual="$1"
  local reference="$2"
  local max_delta="$3"
  awk -v actual="$actual" -v reference="$reference" -v max_delta="$max_delta" '
    BEGIN {
      if (actual == "" || actual == "null" || reference == "" || reference == "null" || max_delta == "") {
        print "unknown";
      } else if ((actual + 0) - (reference + 0) <= max_delta + 0) {
        print "pass";
      } else {
        print "fail";
      }
    }'
}

write_normalized_row() {
  local first=1 arg
  for arg in "$@"; do
    if (( first )); then
      first=0
    else
      printf '\t'
    fi
    printf '%s' "$arg"
  done
  printf '\n'
}

write_threshold_higher_pct() {
  local check="$1"
  local metric="$2"
  local actual="$3"
  local reference="$4"
  local min_pct="$5"
  local pct status
  pct="$(ratio_percent "$actual" "$reference")"
  status="$(threshold_higher_pct_status "$actual" "$reference" "$min_pct")"
  write_normalized_row \
    "$check" \
    "$metric" \
    "$status" \
    "$actual" \
    "$reference" \
    ">=$min_pct%" \
    "${pct:+$pct%}"
}

write_threshold_lower_pct() {
  local check="$1"
  local metric="$2"
  local actual="$3"
  local reference="$4"
  local max_pct="$5"
  local pct status
  pct="$(ratio_percent "$actual" "$reference")"
  status="$(threshold_lower_pct_status "$actual" "$reference" "$max_pct")"
  write_normalized_row \
    "$check" \
    "$metric" \
    "$status" \
    "$actual" \
    "$reference" \
    "<=$max_pct%" \
    "${pct:+$pct%}"
}

write_threshold_lower_delta() {
  local check="$1"
  local metric="$2"
  local actual="$3"
  local reference="$4"
  local max_delta="$5"
  local unit="$6"
  local delta status
  delta="$(delta_value "$actual" "$reference")"
  status="$(threshold_lower_delta_status "$actual" "$reference" "$max_delta")"
  write_normalized_row \
    "$check" \
    "$metric" \
    "$status" \
    "$actual" \
    "$reference" \
    "<=reference+$max_delta$unit" \
    "${delta:+$delta$unit}"
}

tsv_to_json() {
  local file="$1"
  jq -R -s '
    split("\n")
    | map(select(length > 0) | split("\t")) as $rows
    | if ($rows | length) == 0 then []
      else
        ($rows[0]) as $header
        | $rows[1:]
        | map(. as $row | reduce range(0; $header | length) as $i ({}; .[$header[$i]] = ($row[$i] // "")))
      end
  ' "$file"
}

main() {
  need_cmd awk
  need_cmd jq

  local nvpn_summary reference_summary
  nvpn_summary="$(summary_file "$NVPN_DIR")"
  reference_summary="$(summary_file "$REFERENCE_DIR")"
  mkdir -p "$OUTPUT_DIR"

  local nvpn_forward nvpn_reverse nvpn_forward_retrans nvpn_reverse_retrans
  local nvpn_ping_f_p95 nvpn_ping_r_p95 nvpn_ping_f_p99 nvpn_ping_r_p99
  local nvpn_local_cpu nvpn_remote_cpu
  local nvpn_direct_path_checked nvpn_pipeline_log_checked nvpn_counter_progress_checked
  local nvpn_pipeline_hard_events
  local nvpn_pipeline_top_queue_wait_local nvpn_pipeline_top_queue_wait_remote
  local nvpn_forward_collapse_count nvpn_reverse_collapse_count
  local nvpn_fips_liveness_checked nvpn_local_fips_age nvpn_remote_fips_age
  local nvpn_fips_control_liveness_checked nvpn_local_fips_control_age nvpn_remote_fips_control_age
  local nvpn_fips_data_liveness_checked nvpn_local_fips_data_age nvpn_remote_fips_data_age
  local nvpn_local_rekey_in_progress nvpn_local_rekey_draining nvpn_local_current_k_bit nvpn_local_rekey_stuck_count
  local nvpn_remote_rekey_in_progress nvpn_remote_rekey_draining nvpn_remote_current_k_bit nvpn_remote_rekey_stuck_count
  local nvpn_local_direct_probe_pending nvpn_local_direct_probe_after_ms nvpn_local_direct_probe_retry_count nvpn_local_direct_probe_auto_reconnect
  local nvpn_local_direct_probe_expires_at_ms nvpn_local_direct_probe_pending_count nvpn_local_direct_probe_overdue_count
  local nvpn_remote_direct_probe_pending nvpn_remote_direct_probe_after_ms nvpn_remote_direct_probe_retry_count nvpn_remote_direct_probe_auto_reconnect
  local nvpn_remote_direct_probe_expires_at_ms nvpn_remote_direct_probe_pending_count nvpn_remote_direct_probe_overdue_count
  local nvpn_local_nostr_traversal_failures nvpn_local_nostr_traversal_in_cooldown nvpn_local_nostr_traversal_cooldown_until_ms nvpn_local_nostr_traversal_last_skew_ms
  local nvpn_remote_nostr_traversal_failures nvpn_remote_nostr_traversal_in_cooldown nvpn_remote_nostr_traversal_cooldown_until_ms nvpn_remote_nostr_traversal_last_skew_ms
  nvpn_forward="$(tsv_value "$nvpn_summary" iperf_forward_mbps)"
  nvpn_reverse="$(tsv_value "$nvpn_summary" iperf_reverse_mbps)"
  nvpn_forward_retrans="$(tsv_value "$nvpn_summary" iperf_forward_retrans)"
  nvpn_reverse_retrans="$(tsv_value "$nvpn_summary" iperf_reverse_retrans)"
  nvpn_ping_f_p95="$(tsv_value "$nvpn_summary" ping_forward_p95_ms)"
  nvpn_ping_r_p95="$(tsv_value "$nvpn_summary" ping_reverse_p95_ms)"
  nvpn_ping_f_p99="$(tsv_value "$nvpn_summary" ping_forward_p99_ms)"
  nvpn_ping_r_p99="$(tsv_value "$nvpn_summary" ping_reverse_p99_ms)"
  nvpn_local_cpu="$(tsv_value "$nvpn_summary" local_cpu_percent)"
  nvpn_remote_cpu="$(tsv_value "$nvpn_summary" remote_cpu_percent)"
  nvpn_direct_path_checked="$(tsv_optional_value "$nvpn_summary" direct_path_checked)"
  nvpn_pipeline_log_checked="$(tsv_optional_value "$nvpn_summary" pipeline_log_checked)"
  nvpn_counter_progress_checked="$(tsv_optional_value "$nvpn_summary" counter_progress_checked)"
  nvpn_pipeline_hard_events="$(tsv_optional_value "$nvpn_summary" pipeline_hard_events)"
  nvpn_pipeline_top_queue_wait_local="$(tsv_optional_value "$nvpn_summary" pipeline_top_queue_wait_local)"
  nvpn_pipeline_top_queue_wait_remote="$(tsv_optional_value "$nvpn_summary" pipeline_top_queue_wait_remote)"
  nvpn_forward_collapse_count="$(tsv_optional_value "$nvpn_summary" iperf_forward_collapse_count)"
  nvpn_reverse_collapse_count="$(tsv_optional_value "$nvpn_summary" iperf_reverse_collapse_count)"
  nvpn_fips_liveness_checked="$(tsv_optional_value "$nvpn_summary" fips_liveness_checked)"
  nvpn_local_fips_age="$(tsv_optional_value "$nvpn_summary" local_last_fips_seen_age_secs)"
  nvpn_remote_fips_age="$(tsv_optional_value "$nvpn_summary" remote_last_fips_seen_age_secs)"
  nvpn_fips_control_liveness_checked="$(tsv_optional_value "$nvpn_summary" fips_control_liveness_checked)"
  nvpn_local_fips_control_age="$(tsv_optional_value "$nvpn_summary" local_last_fips_control_seen_age_secs)"
  nvpn_remote_fips_control_age="$(tsv_optional_value "$nvpn_summary" remote_last_fips_control_seen_age_secs)"
  nvpn_fips_data_liveness_checked="$(tsv_optional_value "$nvpn_summary" fips_data_liveness_checked)"
  nvpn_local_fips_data_age="$(tsv_optional_value "$nvpn_summary" local_last_fips_data_seen_age_secs)"
  nvpn_remote_fips_data_age="$(tsv_optional_value "$nvpn_summary" remote_last_fips_data_seen_age_secs)"
  nvpn_local_rekey_in_progress="$(tsv_optional_value "$nvpn_summary" local_rekey_in_progress)"
  nvpn_local_rekey_draining="$(tsv_optional_value "$nvpn_summary" local_rekey_draining)"
  nvpn_local_current_k_bit="$(tsv_optional_value "$nvpn_summary" local_current_k_bit)"
  nvpn_local_rekey_stuck_count="$(tsv_optional_value "$nvpn_summary" local_rekey_stuck_count)"
  nvpn_remote_rekey_in_progress="$(tsv_optional_value "$nvpn_summary" remote_rekey_in_progress)"
  nvpn_remote_rekey_draining="$(tsv_optional_value "$nvpn_summary" remote_rekey_draining)"
  nvpn_remote_current_k_bit="$(tsv_optional_value "$nvpn_summary" remote_current_k_bit)"
  nvpn_remote_rekey_stuck_count="$(tsv_optional_value "$nvpn_summary" remote_rekey_stuck_count)"
  nvpn_local_direct_probe_pending="$(tsv_optional_value "$nvpn_summary" local_direct_probe_pending)"
  nvpn_local_direct_probe_after_ms="$(tsv_optional_value "$nvpn_summary" local_direct_probe_after_ms)"
  nvpn_local_direct_probe_retry_count="$(tsv_optional_value "$nvpn_summary" local_direct_probe_retry_count)"
  nvpn_local_direct_probe_auto_reconnect="$(tsv_optional_value "$nvpn_summary" local_direct_probe_auto_reconnect)"
  nvpn_local_direct_probe_expires_at_ms="$(tsv_optional_value "$nvpn_summary" local_direct_probe_expires_at_ms)"
  nvpn_local_direct_probe_pending_count="$(tsv_optional_value "$nvpn_summary" local_direct_probe_pending_count)"
  nvpn_local_direct_probe_overdue_count="$(tsv_optional_value "$nvpn_summary" local_direct_probe_overdue_count)"
  nvpn_remote_direct_probe_pending="$(tsv_optional_value "$nvpn_summary" remote_direct_probe_pending)"
  nvpn_remote_direct_probe_after_ms="$(tsv_optional_value "$nvpn_summary" remote_direct_probe_after_ms)"
  nvpn_remote_direct_probe_retry_count="$(tsv_optional_value "$nvpn_summary" remote_direct_probe_retry_count)"
  nvpn_remote_direct_probe_auto_reconnect="$(tsv_optional_value "$nvpn_summary" remote_direct_probe_auto_reconnect)"
  nvpn_remote_direct_probe_expires_at_ms="$(tsv_optional_value "$nvpn_summary" remote_direct_probe_expires_at_ms)"
  nvpn_remote_direct_probe_pending_count="$(tsv_optional_value "$nvpn_summary" remote_direct_probe_pending_count)"
  nvpn_remote_direct_probe_overdue_count="$(tsv_optional_value "$nvpn_summary" remote_direct_probe_overdue_count)"
  nvpn_local_nostr_traversal_failures="$(tsv_optional_value "$nvpn_summary" local_nostr_traversal_failures)"
  nvpn_local_nostr_traversal_in_cooldown="$(tsv_optional_value "$nvpn_summary" local_nostr_traversal_in_cooldown)"
  nvpn_local_nostr_traversal_cooldown_until_ms="$(tsv_optional_value "$nvpn_summary" local_nostr_traversal_cooldown_until_ms)"
  nvpn_local_nostr_traversal_last_skew_ms="$(tsv_optional_value "$nvpn_summary" local_nostr_traversal_last_skew_ms)"
  nvpn_remote_nostr_traversal_failures="$(tsv_optional_value "$nvpn_summary" remote_nostr_traversal_failures)"
  nvpn_remote_nostr_traversal_in_cooldown="$(tsv_optional_value "$nvpn_summary" remote_nostr_traversal_in_cooldown)"
  nvpn_remote_nostr_traversal_cooldown_until_ms="$(tsv_optional_value "$nvpn_summary" remote_nostr_traversal_cooldown_until_ms)"
  nvpn_remote_nostr_traversal_last_skew_ms="$(tsv_optional_value "$nvpn_summary" remote_nostr_traversal_last_skew_ms)"

  local nvpn_stress_enabled nvpn_stress_sides nvpn_stress_local_workers nvpn_stress_remote_workers
  nvpn_stress_enabled="$(metadata_value "$NVPN_DIR" '.cpu_stress.enabled')"
  nvpn_stress_sides="$(metadata_value "$NVPN_DIR" '.cpu_stress.sides')"
  nvpn_stress_local_workers="$(metadata_value "$NVPN_DIR" '.cpu_stress.local_workers')"
  nvpn_stress_remote_workers="$(metadata_value "$NVPN_DIR" '.cpu_stress.remote_workers')"

  local ref_backend ref_threads ref_forward ref_reverse ref_forward_retrans ref_reverse_retrans
  local ref_ping_f_p95 ref_ping_r_p95 ref_ping_f_p99 ref_ping_r_p99 ref_local_cpu ref_remote_cpu
  ref_backend="$(tsv_value "$reference_summary" backend)"
  ref_threads="$(tsv_optional_value "$reference_summary" threads)"
  ref_forward="$(tsv_value "$reference_summary" tcp_forward_mbps)"
  ref_reverse="$(tsv_value "$reference_summary" tcp_reverse_mbps)"
  ref_forward_retrans="$(tsv_value "$reference_summary" tcp_forward_retrans)"
  ref_reverse_retrans="$(tsv_value "$reference_summary" tcp_reverse_retrans)"
  ref_ping_f_p95="$(tsv_value "$reference_summary" ping_forward_p95_ms)"
  ref_ping_r_p95="$(tsv_value "$reference_summary" ping_reverse_p95_ms)"
  ref_ping_f_p99="$(tsv_value "$reference_summary" ping_forward_p99_ms)"
  ref_ping_r_p99="$(tsv_value "$reference_summary" ping_reverse_p99_ms)"
  ref_local_cpu="$(tsv_value "$reference_summary" local_backend_cpu_percent)"
  ref_remote_cpu="$(tsv_value "$reference_summary" remote_backend_cpu_percent)"

  local ref_stress_enabled ref_stress_sides ref_stress_local_workers ref_stress_remote_workers
  ref_stress_enabled="$(tsv_value "$reference_summary" cpu_stress_enabled)"
  ref_stress_sides="$(tsv_value "$reference_summary" cpu_stress_sides)"
  ref_stress_local_workers="$(tsv_value "$reference_summary" local_cpu_stress_workers)"
  ref_stress_remote_workers="$(tsv_value "$reference_summary" remote_cpu_stress_workers)"

  local forward_ratio reverse_ratio ping_forward_p99_delta ping_reverse_p99_delta
  forward_ratio="$(ratio_percent "$nvpn_forward" "$ref_forward")"
  reverse_ratio="$(ratio_percent "$nvpn_reverse" "$ref_reverse")"
  ping_forward_p99_delta="$(delta_value "$nvpn_ping_f_p99" "$ref_ping_f_p99")"
  ping_reverse_p99_delta="$(delta_value "$nvpn_ping_r_p99" "$ref_ping_r_p99")"

  local comparison_tsv ratios_tsv thresholds_tsv comparison_json
  comparison_tsv="$OUTPUT_DIR/comparison.tsv"
  ratios_tsv="$OUTPUT_DIR/ratios.tsv"
  thresholds_tsv="$OUTPUT_DIR/thresholds.tsv"
  comparison_json="$OUTPUT_DIR/comparison.json"

  write_normalized_row \
    label source_dir backend backend_threads cpu_stress_enabled cpu_stress_sides \
    local_cpu_stress_workers remote_cpu_stress_workers forward_mbps reverse_mbps \
    forward_retrans reverse_retrans ping_forward_p95_ms ping_reverse_p95_ms \
    ping_forward_p99_ms ping_reverse_p99_ms local_cpu_percent remote_cpu_percent \
    direct_path_checked pipeline_log_checked counter_progress_checked \
    iperf_forward_collapse_count iperf_reverse_collapse_count \
    fips_liveness_checked local_last_fips_seen_age_secs remote_last_fips_seen_age_secs \
    fips_control_liveness_checked local_last_fips_control_seen_age_secs remote_last_fips_control_seen_age_secs \
    fips_data_liveness_checked local_last_fips_data_seen_age_secs remote_last_fips_data_seen_age_secs \
    local_rekey_in_progress local_rekey_draining local_current_k_bit local_rekey_stuck_count \
    remote_rekey_in_progress remote_rekey_draining remote_current_k_bit remote_rekey_stuck_count \
    local_direct_probe_pending local_direct_probe_after_ms local_direct_probe_retry_count local_direct_probe_auto_reconnect local_direct_probe_expires_at_ms local_direct_probe_pending_count local_direct_probe_overdue_count \
    remote_direct_probe_pending remote_direct_probe_after_ms remote_direct_probe_retry_count remote_direct_probe_auto_reconnect remote_direct_probe_expires_at_ms remote_direct_probe_pending_count remote_direct_probe_overdue_count \
    local_nostr_traversal_failures local_nostr_traversal_in_cooldown local_nostr_traversal_cooldown_until_ms local_nostr_traversal_last_skew_ms \
    remote_nostr_traversal_failures remote_nostr_traversal_in_cooldown remote_nostr_traversal_cooldown_until_ms remote_nostr_traversal_last_skew_ms \
    pipeline_hard_events pipeline_top_queue_wait_local pipeline_top_queue_wait_remote \
    >"$comparison_tsv"
  write_normalized_row \
    nvpn "$NVPN_DIR" nvpn-fips "" "$nvpn_stress_enabled" "$nvpn_stress_sides" \
    "$nvpn_stress_local_workers" "$nvpn_stress_remote_workers" "$nvpn_forward" "$nvpn_reverse" \
    "$nvpn_forward_retrans" "$nvpn_reverse_retrans" "$nvpn_ping_f_p95" "$nvpn_ping_r_p95" \
    "$nvpn_ping_f_p99" "$nvpn_ping_r_p99" "$nvpn_local_cpu" "$nvpn_remote_cpu" \
    "$nvpn_direct_path_checked" "$nvpn_pipeline_log_checked" "$nvpn_counter_progress_checked" \
    "$nvpn_forward_collapse_count" "$nvpn_reverse_collapse_count" \
    "$nvpn_fips_liveness_checked" "$nvpn_local_fips_age" "$nvpn_remote_fips_age" \
    "$nvpn_fips_control_liveness_checked" "$nvpn_local_fips_control_age" "$nvpn_remote_fips_control_age" \
    "$nvpn_fips_data_liveness_checked" "$nvpn_local_fips_data_age" "$nvpn_remote_fips_data_age" \
    "$nvpn_local_rekey_in_progress" "$nvpn_local_rekey_draining" "$nvpn_local_current_k_bit" "$nvpn_local_rekey_stuck_count" \
    "$nvpn_remote_rekey_in_progress" "$nvpn_remote_rekey_draining" "$nvpn_remote_current_k_bit" "$nvpn_remote_rekey_stuck_count" \
    "$nvpn_local_direct_probe_pending" "$nvpn_local_direct_probe_after_ms" "$nvpn_local_direct_probe_retry_count" "$nvpn_local_direct_probe_auto_reconnect" "$nvpn_local_direct_probe_expires_at_ms" "$nvpn_local_direct_probe_pending_count" "$nvpn_local_direct_probe_overdue_count" \
    "$nvpn_remote_direct_probe_pending" "$nvpn_remote_direct_probe_after_ms" "$nvpn_remote_direct_probe_retry_count" "$nvpn_remote_direct_probe_auto_reconnect" "$nvpn_remote_direct_probe_expires_at_ms" "$nvpn_remote_direct_probe_pending_count" "$nvpn_remote_direct_probe_overdue_count" \
    "$nvpn_local_nostr_traversal_failures" "$nvpn_local_nostr_traversal_in_cooldown" "$nvpn_local_nostr_traversal_cooldown_until_ms" "$nvpn_local_nostr_traversal_last_skew_ms" \
    "$nvpn_remote_nostr_traversal_failures" "$nvpn_remote_nostr_traversal_in_cooldown" "$nvpn_remote_nostr_traversal_cooldown_until_ms" "$nvpn_remote_nostr_traversal_last_skew_ms" \
    "$nvpn_pipeline_hard_events" "$nvpn_pipeline_top_queue_wait_local" "$nvpn_pipeline_top_queue_wait_remote" \
    >>"$comparison_tsv"
  write_normalized_row \
    reference "$REFERENCE_DIR" "$ref_backend" "$ref_threads" "$ref_stress_enabled" "$ref_stress_sides" \
    "$ref_stress_local_workers" "$ref_stress_remote_workers" "$ref_forward" "$ref_reverse" \
    "$ref_forward_retrans" "$ref_reverse_retrans" "$ref_ping_f_p95" "$ref_ping_r_p95" \
    "$ref_ping_f_p99" "$ref_ping_r_p99" "$ref_local_cpu" "$ref_remote_cpu" \
    "" "" "" "" "" \
    "" "" "" "" "" "" "" "" "" \
    "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" "" \
    >>"$comparison_tsv"

  printf '%s\n' 'metric	nvpn	reference	nvpn_pct_of_reference	nvpn_minus_reference' >"$ratios_tsv"
  printf 'forward_mbps\t%s\t%s\t%s\t\n' "$nvpn_forward" "$ref_forward" "$forward_ratio" >>"$ratios_tsv"
  printf 'reverse_mbps\t%s\t%s\t%s\t\n' "$nvpn_reverse" "$ref_reverse" "$reverse_ratio" >>"$ratios_tsv"
  printf 'ping_forward_p99_ms\t%s\t%s\t\t%s\n' "$nvpn_ping_f_p99" "$ref_ping_f_p99" "$ping_forward_p99_delta" >>"$ratios_tsv"
  printf 'ping_reverse_p99_ms\t%s\t%s\t\t%s\n' "$nvpn_ping_r_p99" "$ref_ping_r_p99" "$ping_reverse_p99_delta" >>"$ratios_tsv"

  write_normalized_row check metric status nvpn reference threshold comparison >"$thresholds_tsv"
  write_threshold_higher_pct forward_throughput forward_mbps "$nvpn_forward" "$ref_forward" "$MIN_THROUGHPUT_PCT" >>"$thresholds_tsv"
  write_threshold_higher_pct reverse_throughput reverse_mbps "$nvpn_reverse" "$ref_reverse" "$MIN_THROUGHPUT_PCT" >>"$thresholds_tsv"
  write_threshold_lower_pct forward_retrans forward_retrans "$nvpn_forward_retrans" "$ref_forward_retrans" "$MAX_RETRANS_PCT" >>"$thresholds_tsv"
  write_threshold_lower_pct reverse_retrans reverse_retrans "$nvpn_reverse_retrans" "$ref_reverse_retrans" "$MAX_RETRANS_PCT" >>"$thresholds_tsv"
  write_threshold_lower_delta ping_forward_p99 ping_forward_p99_ms "$nvpn_ping_f_p99" "$ref_ping_f_p99" "$MAX_PING_P99_DELTA_MS" ms >>"$thresholds_tsv"
  write_threshold_lower_delta ping_reverse_p99 ping_reverse_p99_ms "$nvpn_ping_r_p99" "$ref_ping_r_p99" "$MAX_PING_P99_DELTA_MS" ms >>"$thresholds_tsv"

  local thresholds threshold_failures threshold_unknowns threshold_status
  thresholds="$(tsv_to_json "$thresholds_tsv")"
  threshold_failures="$(awk -F '\t' 'NR > 1 && $3 == "fail" { count++ } END { print count + 0 }' "$thresholds_tsv")"
  threshold_unknowns="$(awk -F '\t' 'NR > 1 && $3 == "unknown" { count++ } END { print count + 0 }' "$thresholds_tsv")"
  if [[ "$threshold_failures" == "0" && "$threshold_unknowns" == "0" ]]; then
    threshold_status="pass"
  elif [[ "$threshold_failures" == "0" ]]; then
    threshold_status="unknown"
  else
    threshold_status="fail"
  fi

  jq -nc \
    --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg nvpn_dir "$NVPN_DIR" \
    --arg reference_dir "$REFERENCE_DIR" \
    --arg reference_backend "$ref_backend" \
    --arg reference_threads "$ref_threads" \
    --arg nvpn_forward "$nvpn_forward" \
    --arg nvpn_reverse "$nvpn_reverse" \
    --arg nvpn_forward_retrans "$nvpn_forward_retrans" \
    --arg nvpn_reverse_retrans "$nvpn_reverse_retrans" \
    --arg nvpn_ping_f_p95 "$nvpn_ping_f_p95" \
    --arg nvpn_ping_r_p95 "$nvpn_ping_r_p95" \
    --arg nvpn_ping_f_p99 "$nvpn_ping_f_p99" \
    --arg nvpn_ping_r_p99 "$nvpn_ping_r_p99" \
    --arg nvpn_local_cpu "$nvpn_local_cpu" \
    --arg nvpn_remote_cpu "$nvpn_remote_cpu" \
    --arg nvpn_direct_path_checked "$nvpn_direct_path_checked" \
    --arg nvpn_pipeline_log_checked "$nvpn_pipeline_log_checked" \
    --arg nvpn_counter_progress_checked "$nvpn_counter_progress_checked" \
    --arg nvpn_pipeline_hard_events "$nvpn_pipeline_hard_events" \
    --arg nvpn_pipeline_top_queue_wait_local "$nvpn_pipeline_top_queue_wait_local" \
    --arg nvpn_pipeline_top_queue_wait_remote "$nvpn_pipeline_top_queue_wait_remote" \
    --arg nvpn_forward_collapse_count "$nvpn_forward_collapse_count" \
    --arg nvpn_reverse_collapse_count "$nvpn_reverse_collapse_count" \
    --arg nvpn_fips_liveness_checked "$nvpn_fips_liveness_checked" \
    --arg nvpn_local_fips_age "$nvpn_local_fips_age" \
    --arg nvpn_remote_fips_age "$nvpn_remote_fips_age" \
    --arg nvpn_fips_control_liveness_checked "$nvpn_fips_control_liveness_checked" \
    --arg nvpn_local_fips_control_age "$nvpn_local_fips_control_age" \
    --arg nvpn_remote_fips_control_age "$nvpn_remote_fips_control_age" \
    --arg nvpn_fips_data_liveness_checked "$nvpn_fips_data_liveness_checked" \
    --arg nvpn_local_fips_data_age "$nvpn_local_fips_data_age" \
    --arg nvpn_remote_fips_data_age "$nvpn_remote_fips_data_age" \
    --arg nvpn_local_rekey_in_progress "$nvpn_local_rekey_in_progress" \
    --arg nvpn_local_rekey_draining "$nvpn_local_rekey_draining" \
    --arg nvpn_local_current_k_bit "$nvpn_local_current_k_bit" \
    --arg nvpn_local_rekey_stuck_count "$nvpn_local_rekey_stuck_count" \
    --arg nvpn_remote_rekey_in_progress "$nvpn_remote_rekey_in_progress" \
    --arg nvpn_remote_rekey_draining "$nvpn_remote_rekey_draining" \
    --arg nvpn_remote_current_k_bit "$nvpn_remote_current_k_bit" \
    --arg nvpn_remote_rekey_stuck_count "$nvpn_remote_rekey_stuck_count" \
    --arg nvpn_local_direct_probe_pending "$nvpn_local_direct_probe_pending" \
    --arg nvpn_local_direct_probe_after_ms "$nvpn_local_direct_probe_after_ms" \
    --arg nvpn_local_direct_probe_retry_count "$nvpn_local_direct_probe_retry_count" \
    --arg nvpn_local_direct_probe_auto_reconnect "$nvpn_local_direct_probe_auto_reconnect" \
    --arg nvpn_local_direct_probe_expires_at_ms "$nvpn_local_direct_probe_expires_at_ms" \
    --arg nvpn_local_direct_probe_pending_count "$nvpn_local_direct_probe_pending_count" \
    --arg nvpn_local_direct_probe_overdue_count "$nvpn_local_direct_probe_overdue_count" \
    --arg nvpn_remote_direct_probe_pending "$nvpn_remote_direct_probe_pending" \
    --arg nvpn_remote_direct_probe_after_ms "$nvpn_remote_direct_probe_after_ms" \
    --arg nvpn_remote_direct_probe_retry_count "$nvpn_remote_direct_probe_retry_count" \
    --arg nvpn_remote_direct_probe_auto_reconnect "$nvpn_remote_direct_probe_auto_reconnect" \
    --arg nvpn_remote_direct_probe_expires_at_ms "$nvpn_remote_direct_probe_expires_at_ms" \
    --arg nvpn_remote_direct_probe_pending_count "$nvpn_remote_direct_probe_pending_count" \
    --arg nvpn_remote_direct_probe_overdue_count "$nvpn_remote_direct_probe_overdue_count" \
    --arg nvpn_local_nostr_traversal_failures "$nvpn_local_nostr_traversal_failures" \
    --arg nvpn_local_nostr_traversal_in_cooldown "$nvpn_local_nostr_traversal_in_cooldown" \
    --arg nvpn_local_nostr_traversal_cooldown_until_ms "$nvpn_local_nostr_traversal_cooldown_until_ms" \
    --arg nvpn_local_nostr_traversal_last_skew_ms "$nvpn_local_nostr_traversal_last_skew_ms" \
    --arg nvpn_remote_nostr_traversal_failures "$nvpn_remote_nostr_traversal_failures" \
    --arg nvpn_remote_nostr_traversal_in_cooldown "$nvpn_remote_nostr_traversal_in_cooldown" \
    --arg nvpn_remote_nostr_traversal_cooldown_until_ms "$nvpn_remote_nostr_traversal_cooldown_until_ms" \
    --arg nvpn_remote_nostr_traversal_last_skew_ms "$nvpn_remote_nostr_traversal_last_skew_ms" \
    --arg nvpn_stress_enabled "$nvpn_stress_enabled" \
    --arg nvpn_stress_sides "$nvpn_stress_sides" \
    --arg nvpn_stress_local_workers "$nvpn_stress_local_workers" \
    --arg nvpn_stress_remote_workers "$nvpn_stress_remote_workers" \
    --arg ref_forward "$ref_forward" \
    --arg ref_reverse "$ref_reverse" \
    --arg ref_forward_retrans "$ref_forward_retrans" \
    --arg ref_reverse_retrans "$ref_reverse_retrans" \
    --arg ref_ping_f_p95 "$ref_ping_f_p95" \
    --arg ref_ping_r_p95 "$ref_ping_r_p95" \
    --arg ref_ping_f_p99 "$ref_ping_f_p99" \
    --arg ref_ping_r_p99 "$ref_ping_r_p99" \
    --arg ref_local_cpu "$ref_local_cpu" \
    --arg ref_remote_cpu "$ref_remote_cpu" \
    --arg ref_stress_enabled "$ref_stress_enabled" \
    --arg ref_stress_sides "$ref_stress_sides" \
    --arg ref_stress_local_workers "$ref_stress_local_workers" \
    --arg ref_stress_remote_workers "$ref_stress_remote_workers" \
    --arg forward_ratio "$forward_ratio" \
    --arg reverse_ratio "$reverse_ratio" \
    --arg ping_forward_p99_delta "$ping_forward_p99_delta" \
    --arg ping_reverse_p99_delta "$ping_reverse_p99_delta" \
    --arg comparison_tsv "$comparison_tsv" \
    --arg ratios_tsv "$ratios_tsv" \
    --arg thresholds_tsv "$thresholds_tsv" \
    --arg min_throughput_pct "$MIN_THROUGHPUT_PCT" \
    --arg max_retrans_pct "$MAX_RETRANS_PCT" \
    --arg max_ping_p99_delta_ms "$MAX_PING_P99_DELTA_MS" \
    --arg threshold_status "$threshold_status" \
    --arg threshold_failures "$threshold_failures" \
    --arg threshold_unknowns "$threshold_unknowns" \
    --argjson thresholds "$thresholds" \
    'def num($v): if $v == "" or $v == "null" then null else ($v | tonumber) end;
     def bool($v):
       if $v == "" or $v == "null" then null
       elif $v == "1" or $v == "true" then true
       elif $v == "0" or $v == "false" then false
       else null
       end;
     def csv($v):
       if $v == "" or $v == "null" then []
       else ($v | split(",") | map(select(. != "")))
       end;
     {
       created_at: $created_at,
       artifacts: {
         nvpn_dir: $nvpn_dir,
         reference_dir: $reference_dir,
         comparison_tsv: $comparison_tsv,
         ratios_tsv: $ratios_tsv,
         thresholds_tsv: $thresholds_tsv
       },
       nvpn: {
         backend: "nvpn-fips",
         cpu_stress: {
           enabled: bool($nvpn_stress_enabled),
           sides: $nvpn_stress_sides,
           local_workers: num($nvpn_stress_local_workers),
           remote_workers: num($nvpn_stress_remote_workers)
         },
         forward_mbps: num($nvpn_forward),
         reverse_mbps: num($nvpn_reverse),
         forward_retrans: num($nvpn_forward_retrans),
         reverse_retrans: num($nvpn_reverse_retrans),
         ping_forward_p95_ms: num($nvpn_ping_f_p95),
         ping_reverse_p95_ms: num($nvpn_ping_r_p95),
         ping_forward_p99_ms: num($nvpn_ping_f_p99),
         ping_reverse_p99_ms: num($nvpn_ping_r_p99),
         local_cpu_percent: num($nvpn_local_cpu),
         remote_cpu_percent: num($nvpn_remote_cpu),
         safety_checks: {
           direct_path_checked: bool($nvpn_direct_path_checked),
           pipeline_log_checked: bool($nvpn_pipeline_log_checked),
           counter_progress_checked: bool($nvpn_counter_progress_checked),
           pipeline_hard_events: csv($nvpn_pipeline_hard_events),
           pipeline_top_queue_wait: {
             local: (if $nvpn_pipeline_top_queue_wait_local == "" or $nvpn_pipeline_top_queue_wait_local == "null" then null else $nvpn_pipeline_top_queue_wait_local end),
             remote: (if $nvpn_pipeline_top_queue_wait_remote == "" or $nvpn_pipeline_top_queue_wait_remote == "null" then null else $nvpn_pipeline_top_queue_wait_remote end)
           },
           iperf_forward_collapse_count: num($nvpn_forward_collapse_count),
           iperf_reverse_collapse_count: num($nvpn_reverse_collapse_count)
         },
         fips_liveness: {
           checked: bool($nvpn_fips_liveness_checked),
           local_last_seen_age_secs: num($nvpn_local_fips_age),
           remote_last_seen_age_secs: num($nvpn_remote_fips_age)
         },
         fips_control_liveness: {
           checked: bool($nvpn_fips_control_liveness_checked),
           local_last_seen_age_secs: num($nvpn_local_fips_control_age),
           remote_last_seen_age_secs: num($nvpn_remote_fips_control_age)
         },
         fips_data_liveness: {
           checked: bool($nvpn_fips_data_liveness_checked),
           local_last_seen_age_secs: num($nvpn_local_fips_data_age),
           remote_last_seen_age_secs: num($nvpn_remote_fips_data_age)
         },
         rekey: {
           local_in_progress: bool($nvpn_local_rekey_in_progress),
           local_draining: bool($nvpn_local_rekey_draining),
           local_current_k_bit: bool($nvpn_local_current_k_bit),
           local_stuck_count: num($nvpn_local_rekey_stuck_count),
           remote_in_progress: bool($nvpn_remote_rekey_in_progress),
           remote_draining: bool($nvpn_remote_rekey_draining),
           remote_current_k_bit: bool($nvpn_remote_current_k_bit),
           remote_stuck_count: num($nvpn_remote_rekey_stuck_count)
         },
         direct_probe: {
           local_pending: bool($nvpn_local_direct_probe_pending),
           local_after_ms: num($nvpn_local_direct_probe_after_ms),
           local_retry_count: num($nvpn_local_direct_probe_retry_count),
           local_auto_reconnect: bool($nvpn_local_direct_probe_auto_reconnect),
           local_expires_at_ms: num($nvpn_local_direct_probe_expires_at_ms),
           local_pending_count: num($nvpn_local_direct_probe_pending_count),
           local_overdue_count: num($nvpn_local_direct_probe_overdue_count),
           remote_pending: bool($nvpn_remote_direct_probe_pending),
           remote_after_ms: num($nvpn_remote_direct_probe_after_ms),
           remote_retry_count: num($nvpn_remote_direct_probe_retry_count),
           remote_auto_reconnect: bool($nvpn_remote_direct_probe_auto_reconnect),
           remote_expires_at_ms: num($nvpn_remote_direct_probe_expires_at_ms),
           remote_pending_count: num($nvpn_remote_direct_probe_pending_count),
           remote_overdue_count: num($nvpn_remote_direct_probe_overdue_count)
         },
         nostr_traversal: {
           local_failures: num($nvpn_local_nostr_traversal_failures),
           local_in_cooldown: bool($nvpn_local_nostr_traversal_in_cooldown),
           local_cooldown_until_ms: num($nvpn_local_nostr_traversal_cooldown_until_ms),
           local_last_skew_ms: num($nvpn_local_nostr_traversal_last_skew_ms),
           remote_failures: num($nvpn_remote_nostr_traversal_failures),
           remote_in_cooldown: bool($nvpn_remote_nostr_traversal_in_cooldown),
           remote_cooldown_until_ms: num($nvpn_remote_nostr_traversal_cooldown_until_ms),
           remote_last_skew_ms: num($nvpn_remote_nostr_traversal_last_skew_ms)
         }
       },
       reference: {
         backend: $reference_backend,
         backend_threads: num($reference_threads),
         cpu_stress: {
           enabled: bool($ref_stress_enabled),
           sides: $ref_stress_sides,
           local_workers: num($ref_stress_local_workers),
           remote_workers: num($ref_stress_remote_workers)
         },
         forward_mbps: num($ref_forward),
         reverse_mbps: num($ref_reverse),
         forward_retrans: num($ref_forward_retrans),
         reverse_retrans: num($ref_reverse_retrans),
         ping_forward_p95_ms: num($ref_ping_f_p95),
         ping_reverse_p95_ms: num($ref_ping_r_p95),
         ping_forward_p99_ms: num($ref_ping_f_p99),
         ping_reverse_p99_ms: num($ref_ping_r_p99),
         local_cpu_percent: num($ref_local_cpu),
         remote_cpu_percent: num($ref_remote_cpu)
       },
       ratios: {
         forward_mbps_pct_of_reference: num($forward_ratio),
         reverse_mbps_pct_of_reference: num($reverse_ratio)
       },
       deltas: {
         ping_forward_p99_ms_nvpn_minus_reference: num($ping_forward_p99_delta),
         ping_reverse_p99_ms_nvpn_minus_reference: num($ping_reverse_p99_delta)
       },
       threshold_policy: {
         min_throughput_pct: num($min_throughput_pct),
         max_retrans_pct: num($max_retrans_pct),
         max_ping_p99_delta_ms: num($max_ping_p99_delta_ms)
       },
       threshold_status: {
         status: $threshold_status,
         failures: num($threshold_failures),
         unknowns: num($threshold_unknowns)
       },
       thresholds: $thresholds
     }' >"$comparison_json"

  if [[ "$ENFORCE_THRESHOLDS" == "1" && "$threshold_status" != "pass" ]]; then
    die "threshold status is $threshold_status; see $thresholds_tsv"
  fi

  printf 'host-pair benchmark comparison wrote %s, %s, %s, and %s\n' \
    "$comparison_tsv" "$ratios_tsv" "$thresholds_tsv" "$comparison_json"
}

main "$@"
