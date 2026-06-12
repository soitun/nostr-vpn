#!/usr/bin/env bash
# Summarize one host-pair comparison run bundle across modes/backends.
set -euo pipefail

RUN_DIR="${NVPN_HOST_PAIR_COMPARISON_RUN_DIR:-${1:-}}"
SUMMARY_TSV="${NVPN_HOST_PAIR_COMPARISON_MATRIX_TSV:-${2:-}}"
SUMMARY_JSON="${NVPN_HOST_PAIR_COMPARISON_MATRIX_JSON:-${3:-}}"
STRESS_DELTAS_TSV="${NVPN_HOST_PAIR_COMPARISON_STRESS_DELTAS_TSV:-}"
RELIABILITY_TSV="${NVPN_HOST_PAIR_COMPARISON_RELIABILITY_TSV:-}"
RELIABILITY_JSON="${NVPN_HOST_PAIR_COMPARISON_RELIABILITY_JSON:-}"

die() {
  printf 'host-pair comparison run summary failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
usage: scripts/summarize-host-pair-comparison-run.sh <comparison-run-dir> [summary.tsv] [summary.json]

Reads:
  <comparison-run-dir>/manifest.tsv from scripts/run-host-pair-comparison.sh
  each listed <comparison-dir>/comparison.json

Writes:
  matrix-summary.tsv, matrix-stress-deltas.tsv, matrix-reliability.tsv,
  matrix-reliability.json, and matrix-summary.json in the run dir by default
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

json_value() {
  local json="$1"
  local filter="$2"
  jq -r "$filter // \"\"" "$json"
}

write_summary_row() {
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

main() {
  [[ -n "$RUN_DIR" ]] || {
    usage
    die "comparison run directory is required"
  }
  [[ -d "$RUN_DIR" ]] || die "comparison run directory not found: $RUN_DIR"

  need_cmd jq

  local manifest="$RUN_DIR/manifest.tsv"
  [[ -f "$manifest" ]] || die "missing manifest.tsv in $RUN_DIR"

  SUMMARY_TSV="${SUMMARY_TSV:-$RUN_DIR/matrix-summary.tsv}"
  SUMMARY_JSON="${SUMMARY_JSON:-$RUN_DIR/matrix-summary.json}"
  STRESS_DELTAS_TSV="${STRESS_DELTAS_TSV:-$RUN_DIR/matrix-stress-deltas.tsv}"
  RELIABILITY_TSV="${RELIABILITY_TSV:-$RUN_DIR/matrix-reliability.tsv}"
  RELIABILITY_JSON="${RELIABILITY_JSON:-$RUN_DIR/matrix-reliability.json}"

  write_summary_row \
    mode backend cpu_stress_enabled \
    nvpn_forward_mbps nvpn_reverse_mbps reference_forward_mbps reference_reverse_mbps \
    nvpn_forward_pct_of_reference nvpn_reverse_pct_of_reference \
    nvpn_forward_retrans nvpn_reverse_retrans reference_forward_retrans reference_reverse_retrans \
    nvpn_ping_forward_p99_ms nvpn_ping_reverse_p99_ms \
    reference_ping_forward_p99_ms reference_ping_reverse_p99_ms \
    nvpn_local_cpu_percent nvpn_remote_cpu_percent \
    reference_local_cpu_percent reference_remote_cpu_percent comparison_dir \
    nvpn_direct_path_checked nvpn_pipeline_log_checked nvpn_counter_progress_checked \
    nvpn_iperf_forward_collapse_count nvpn_iperf_reverse_collapse_count \
    nvpn_fips_liveness_checked nvpn_local_fips_seen_age_secs nvpn_remote_fips_seen_age_secs \
    nvpn_fips_control_liveness_checked nvpn_local_fips_control_seen_age_secs nvpn_remote_fips_control_seen_age_secs \
    nvpn_fips_data_liveness_checked nvpn_local_fips_data_seen_age_secs nvpn_remote_fips_data_seen_age_secs \
    nvpn_local_rekey_in_progress nvpn_local_rekey_draining nvpn_local_current_k_bit nvpn_local_rekey_stuck_count \
    nvpn_remote_rekey_in_progress nvpn_remote_rekey_draining nvpn_remote_current_k_bit nvpn_remote_rekey_stuck_count \
    nvpn_local_direct_probe_pending nvpn_local_direct_probe_after_ms nvpn_local_direct_probe_retry_count nvpn_local_direct_probe_auto_reconnect nvpn_local_direct_probe_expires_at_ms nvpn_local_direct_probe_pending_count nvpn_local_direct_probe_overdue_count \
    nvpn_remote_direct_probe_pending nvpn_remote_direct_probe_after_ms nvpn_remote_direct_probe_retry_count nvpn_remote_direct_probe_auto_reconnect nvpn_remote_direct_probe_expires_at_ms nvpn_remote_direct_probe_pending_count nvpn_remote_direct_probe_overdue_count \
    nvpn_local_nostr_traversal_failures nvpn_local_nostr_traversal_in_cooldown nvpn_local_nostr_traversal_cooldown_until_ms nvpn_local_nostr_traversal_last_skew_ms \
    nvpn_remote_nostr_traversal_failures nvpn_remote_nostr_traversal_in_cooldown nvpn_remote_nostr_traversal_cooldown_until_ms nvpn_remote_nostr_traversal_last_skew_ms \
    nvpn_pipeline_hard_events nvpn_pipeline_top_queue_wait_local nvpn_pipeline_top_queue_wait_remote \
    nvpn_ping_forward_p99_delta_vs_reference_ms nvpn_ping_reverse_p99_delta_vs_reference_ms \
    >"$SUMMARY_TSV"

  local mode backend cpu_stress_enabled nvpn_dir reference_dir comparison_dir
  local comparison_json
  local nvpn_forward nvpn_reverse ref_forward ref_reverse forward_ratio reverse_ratio
  local nvpn_forward_retrans nvpn_reverse_retrans ref_forward_retrans ref_reverse_retrans
  local nvpn_ping_f_p99 nvpn_ping_r_p99 ref_ping_f_p99 ref_ping_r_p99
  local ping_forward_p99_delta ping_reverse_p99_delta
  local nvpn_local_cpu nvpn_remote_cpu ref_local_cpu ref_remote_cpu
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
  local rows_json
  rows_json="$(mktemp "${TMPDIR:-/tmp}/nvpn-host-pair-matrix.XXXXXX")"
  printf '%s\n' '[]' >"$rows_json"
  while IFS=$'\t' read -r mode backend cpu_stress_enabled nvpn_dir reference_dir comparison_dir; do
    [[ "$mode" == "mode" ]] && continue
    [[ -n "${mode:-}${backend:-}${comparison_dir:-}" ]] || continue
    comparison_json="$comparison_dir/comparison.json"
    [[ -f "$comparison_json" ]] || die "missing comparison.json for $mode/$backend at $comparison_json"

    nvpn_forward="$(json_value "$comparison_json" '.nvpn.forward_mbps')"
    nvpn_reverse="$(json_value "$comparison_json" '.nvpn.reverse_mbps')"
    ref_forward="$(json_value "$comparison_json" '.reference.forward_mbps')"
    ref_reverse="$(json_value "$comparison_json" '.reference.reverse_mbps')"
    forward_ratio="$(json_value "$comparison_json" '.ratios.forward_mbps_pct_of_reference')"
    reverse_ratio="$(json_value "$comparison_json" '.ratios.reverse_mbps_pct_of_reference')"
    nvpn_forward_retrans="$(json_value "$comparison_json" '.nvpn.forward_retrans')"
    nvpn_reverse_retrans="$(json_value "$comparison_json" '.nvpn.reverse_retrans')"
    ref_forward_retrans="$(json_value "$comparison_json" '.reference.forward_retrans')"
    ref_reverse_retrans="$(json_value "$comparison_json" '.reference.reverse_retrans')"
    nvpn_ping_f_p99="$(json_value "$comparison_json" '.nvpn.ping_forward_p99_ms')"
    nvpn_ping_r_p99="$(json_value "$comparison_json" '.nvpn.ping_reverse_p99_ms')"
    ref_ping_f_p99="$(json_value "$comparison_json" '.reference.ping_forward_p99_ms')"
    ref_ping_r_p99="$(json_value "$comparison_json" '.reference.ping_reverse_p99_ms')"
    ping_forward_p99_delta="$(json_value "$comparison_json" '.deltas.ping_forward_p99_ms_nvpn_minus_reference')"
    ping_reverse_p99_delta="$(json_value "$comparison_json" '.deltas.ping_reverse_p99_ms_nvpn_minus_reference')"
    nvpn_local_cpu="$(json_value "$comparison_json" '.nvpn.local_cpu_percent')"
    nvpn_remote_cpu="$(json_value "$comparison_json" '.nvpn.remote_cpu_percent')"
    ref_local_cpu="$(json_value "$comparison_json" '.reference.local_cpu_percent')"
    ref_remote_cpu="$(json_value "$comparison_json" '.reference.remote_cpu_percent')"
    nvpn_direct_path_checked="$(json_value "$comparison_json" '.nvpn.safety_checks.direct_path_checked')"
    nvpn_pipeline_log_checked="$(json_value "$comparison_json" '.nvpn.safety_checks.pipeline_log_checked')"
    nvpn_counter_progress_checked="$(json_value "$comparison_json" '.nvpn.safety_checks.counter_progress_checked')"
    nvpn_pipeline_hard_events="$(json_value "$comparison_json" '(.nvpn.safety_checks.pipeline_hard_events // []) | join(",")')"
    nvpn_pipeline_top_queue_wait_local="$(json_value "$comparison_json" '.nvpn.safety_checks.pipeline_top_queue_wait.local')"
    nvpn_pipeline_top_queue_wait_remote="$(json_value "$comparison_json" '.nvpn.safety_checks.pipeline_top_queue_wait.remote')"
    nvpn_forward_collapse_count="$(json_value "$comparison_json" '.nvpn.safety_checks.iperf_forward_collapse_count')"
    nvpn_reverse_collapse_count="$(json_value "$comparison_json" '.nvpn.safety_checks.iperf_reverse_collapse_count')"
    nvpn_fips_liveness_checked="$(json_value "$comparison_json" '.nvpn.fips_liveness.checked')"
    nvpn_local_fips_age="$(json_value "$comparison_json" '.nvpn.fips_liveness.local_last_seen_age_secs')"
    nvpn_remote_fips_age="$(json_value "$comparison_json" '.nvpn.fips_liveness.remote_last_seen_age_secs')"
    nvpn_fips_control_liveness_checked="$(json_value "$comparison_json" '.nvpn.fips_control_liveness.checked')"
    nvpn_local_fips_control_age="$(json_value "$comparison_json" '.nvpn.fips_control_liveness.local_last_seen_age_secs')"
    nvpn_remote_fips_control_age="$(json_value "$comparison_json" '.nvpn.fips_control_liveness.remote_last_seen_age_secs')"
    nvpn_fips_data_liveness_checked="$(json_value "$comparison_json" '.nvpn.fips_data_liveness.checked')"
    nvpn_local_fips_data_age="$(json_value "$comparison_json" '.nvpn.fips_data_liveness.local_last_seen_age_secs')"
    nvpn_remote_fips_data_age="$(json_value "$comparison_json" '.nvpn.fips_data_liveness.remote_last_seen_age_secs')"
    nvpn_local_rekey_in_progress="$(json_value "$comparison_json" '.nvpn.rekey.local_in_progress')"
    nvpn_local_rekey_draining="$(json_value "$comparison_json" '.nvpn.rekey.local_draining')"
    nvpn_local_current_k_bit="$(json_value "$comparison_json" '.nvpn.rekey.local_current_k_bit')"
    nvpn_local_rekey_stuck_count="$(json_value "$comparison_json" '.nvpn.rekey.local_stuck_count')"
    nvpn_remote_rekey_in_progress="$(json_value "$comparison_json" '.nvpn.rekey.remote_in_progress')"
    nvpn_remote_rekey_draining="$(json_value "$comparison_json" '.nvpn.rekey.remote_draining')"
    nvpn_remote_current_k_bit="$(json_value "$comparison_json" '.nvpn.rekey.remote_current_k_bit')"
    nvpn_remote_rekey_stuck_count="$(json_value "$comparison_json" '.nvpn.rekey.remote_stuck_count')"
    nvpn_local_direct_probe_pending="$(json_value "$comparison_json" '.nvpn.direct_probe.local_pending')"
    nvpn_local_direct_probe_after_ms="$(json_value "$comparison_json" '.nvpn.direct_probe.local_after_ms')"
    nvpn_local_direct_probe_retry_count="$(json_value "$comparison_json" '.nvpn.direct_probe.local_retry_count')"
    nvpn_local_direct_probe_auto_reconnect="$(json_value "$comparison_json" '.nvpn.direct_probe.local_auto_reconnect')"
    nvpn_local_direct_probe_expires_at_ms="$(json_value "$comparison_json" '.nvpn.direct_probe.local_expires_at_ms')"
    nvpn_local_direct_probe_pending_count="$(json_value "$comparison_json" '.nvpn.direct_probe.local_pending_count')"
    nvpn_local_direct_probe_overdue_count="$(json_value "$comparison_json" '.nvpn.direct_probe.local_overdue_count')"
    nvpn_remote_direct_probe_pending="$(json_value "$comparison_json" '.nvpn.direct_probe.remote_pending')"
    nvpn_remote_direct_probe_after_ms="$(json_value "$comparison_json" '.nvpn.direct_probe.remote_after_ms')"
    nvpn_remote_direct_probe_retry_count="$(json_value "$comparison_json" '.nvpn.direct_probe.remote_retry_count')"
    nvpn_remote_direct_probe_auto_reconnect="$(json_value "$comparison_json" '.nvpn.direct_probe.remote_auto_reconnect')"
    nvpn_remote_direct_probe_expires_at_ms="$(json_value "$comparison_json" '.nvpn.direct_probe.remote_expires_at_ms')"
    nvpn_remote_direct_probe_pending_count="$(json_value "$comparison_json" '.nvpn.direct_probe.remote_pending_count')"
    nvpn_remote_direct_probe_overdue_count="$(json_value "$comparison_json" '.nvpn.direct_probe.remote_overdue_count')"
    nvpn_local_nostr_traversal_failures="$(json_value "$comparison_json" '.nvpn.nostr_traversal.local_failures')"
    nvpn_local_nostr_traversal_in_cooldown="$(json_value "$comparison_json" '.nvpn.nostr_traversal.local_in_cooldown')"
    nvpn_local_nostr_traversal_cooldown_until_ms="$(json_value "$comparison_json" '.nvpn.nostr_traversal.local_cooldown_until_ms')"
    nvpn_local_nostr_traversal_last_skew_ms="$(json_value "$comparison_json" '.nvpn.nostr_traversal.local_last_skew_ms')"
    nvpn_remote_nostr_traversal_failures="$(json_value "$comparison_json" '.nvpn.nostr_traversal.remote_failures')"
    nvpn_remote_nostr_traversal_in_cooldown="$(json_value "$comparison_json" '.nvpn.nostr_traversal.remote_in_cooldown')"
    nvpn_remote_nostr_traversal_cooldown_until_ms="$(json_value "$comparison_json" '.nvpn.nostr_traversal.remote_cooldown_until_ms')"
    nvpn_remote_nostr_traversal_last_skew_ms="$(json_value "$comparison_json" '.nvpn.nostr_traversal.remote_last_skew_ms')"

    write_summary_row \
      "$mode" "$backend" "$cpu_stress_enabled" \
      "$nvpn_forward" "$nvpn_reverse" "$ref_forward" "$ref_reverse" \
      "$forward_ratio" "$reverse_ratio" \
      "$nvpn_forward_retrans" "$nvpn_reverse_retrans" "$ref_forward_retrans" "$ref_reverse_retrans" \
      "$nvpn_ping_f_p99" "$nvpn_ping_r_p99" "$ref_ping_f_p99" "$ref_ping_r_p99" \
      "$nvpn_local_cpu" "$nvpn_remote_cpu" "$ref_local_cpu" "$ref_remote_cpu" "$comparison_dir" \
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
      "$ping_forward_p99_delta" "$ping_reverse_p99_delta" \
      >>"$SUMMARY_TSV"

    jq \
      --arg mode "$mode" \
      --arg backend "$backend" \
      --arg cpu_stress_enabled "$cpu_stress_enabled" \
      --arg nvpn_dir "$nvpn_dir" \
      --arg reference_dir "$reference_dir" \
      --arg comparison_dir "$comparison_dir" \
      --slurpfile comparison "$comparison_json" \
      'def bool($v):
         if $v == "1" or $v == "true" then true
         elif $v == "0" or $v == "false" then false
         else null
         end;
       . + [{
         mode: $mode,
         backend: $backend,
         cpu_stress_enabled: bool($cpu_stress_enabled),
         artifacts: {
           nvpn_dir: $nvpn_dir,
           reference_dir: $reference_dir,
           comparison_dir: $comparison_dir
         },
         nvpn: $comparison[0].nvpn,
         reference: $comparison[0].reference,
         ratios: $comparison[0].ratios,
         deltas: $comparison[0].deltas,
         threshold_policy: ($comparison[0].threshold_policy // null),
         threshold_status: ($comparison[0].threshold_status // null),
         thresholds: ($comparison[0].thresholds // [])
       }]' "$rows_json" >"$rows_json.next"
    mv "$rows_json.next" "$rows_json"
  done <"$manifest"

  local stress_deltas_json
  stress_deltas_json="$(mktemp "${TMPDIR:-/tmp}/nvpn-host-pair-stress-deltas.XXXXXX")"
  jq '
    def num($v):
      if $v == null or $v == "" then null
      elif ($v | type) == "number" then $v
      else ($v | tonumber?)
      end;
    def pct($stress; $clean):
      (num($stress)) as $s
      | (num($clean)) as $c
      | if $s == null or $c == null or $c == 0 then null
        else ((($s * 1000 / $c) | round) / 10)
        end;
    def delta($stress; $clean):
      (num($stress)) as $s
      | (num($clean)) as $c
      | if $s == null or $c == null then null
        else ((($s - $c) * 10 | round) / 10)
        end;
    . as $rows
    | [
        $rows[]
        | select(.mode == "clean")
        | . as $clean
        | $rows[]
        | select(.mode == "stress" and .backend == $clean.backend)
        | . as $stress
        | (delta($clean.nvpn.ping_forward_p99_ms; $clean.reference.ping_forward_p99_ms)) as $clean_forward_p99_reference_delta
        | (delta($stress.nvpn.ping_forward_p99_ms; $stress.reference.ping_forward_p99_ms)) as $stress_forward_p99_reference_delta
        | (delta($clean.nvpn.ping_reverse_p99_ms; $clean.reference.ping_reverse_p99_ms)) as $clean_reverse_p99_reference_delta
        | (delta($stress.nvpn.ping_reverse_p99_ms; $stress.reference.ping_reverse_p99_ms)) as $stress_reverse_p99_reference_delta
        | {
            backend: $clean.backend,
            modes: {
              clean: $clean.mode,
              stress: $stress.mode
            },
            nvpn: {
              forward_mbps: {
                clean: num($clean.nvpn.forward_mbps),
                stress: num($stress.nvpn.forward_mbps),
                stress_pct_of_clean: pct($stress.nvpn.forward_mbps; $clean.nvpn.forward_mbps)
              },
              reverse_mbps: {
                clean: num($clean.nvpn.reverse_mbps),
                stress: num($stress.nvpn.reverse_mbps),
                stress_pct_of_clean: pct($stress.nvpn.reverse_mbps; $clean.nvpn.reverse_mbps)
              },
              ping_forward_p99_ms: {
                clean: num($clean.nvpn.ping_forward_p99_ms),
                stress: num($stress.nvpn.ping_forward_p99_ms),
                stress_minus_clean_ms: delta($stress.nvpn.ping_forward_p99_ms; $clean.nvpn.ping_forward_p99_ms)
              },
              ping_reverse_p99_ms: {
                clean: num($clean.nvpn.ping_reverse_p99_ms),
                stress: num($stress.nvpn.ping_reverse_p99_ms),
                stress_minus_clean_ms: delta($stress.nvpn.ping_reverse_p99_ms; $clean.nvpn.ping_reverse_p99_ms)
              }
            },
            reference: {
              forward_mbps: {
                clean: num($clean.reference.forward_mbps),
                stress: num($stress.reference.forward_mbps),
                stress_pct_of_clean: pct($stress.reference.forward_mbps; $clean.reference.forward_mbps)
              },
              reverse_mbps: {
                clean: num($clean.reference.reverse_mbps),
                stress: num($stress.reference.reverse_mbps),
                stress_pct_of_clean: pct($stress.reference.reverse_mbps; $clean.reference.reverse_mbps)
              },
              ping_forward_p99_ms: {
                clean: num($clean.reference.ping_forward_p99_ms),
                stress: num($stress.reference.ping_forward_p99_ms),
                stress_minus_clean_ms: delta($stress.reference.ping_forward_p99_ms; $clean.reference.ping_forward_p99_ms)
              },
              ping_reverse_p99_ms: {
                clean: num($clean.reference.ping_reverse_p99_ms),
                stress: num($stress.reference.ping_reverse_p99_ms),
                stress_minus_clean_ms: delta($stress.reference.ping_reverse_p99_ms; $clean.reference.ping_reverse_p99_ms)
              }
            },
            ratios: {
              forward_mbps_pct_of_reference: {
                clean: num($clean.ratios.forward_mbps_pct_of_reference),
                stress: num($stress.ratios.forward_mbps_pct_of_reference),
                stress_minus_clean_points: delta($stress.ratios.forward_mbps_pct_of_reference; $clean.ratios.forward_mbps_pct_of_reference)
              },
              reverse_mbps_pct_of_reference: {
                clean: num($clean.ratios.reverse_mbps_pct_of_reference),
                stress: num($stress.ratios.reverse_mbps_pct_of_reference),
                stress_minus_clean_points: delta($stress.ratios.reverse_mbps_pct_of_reference; $clean.ratios.reverse_mbps_pct_of_reference)
              }
            },
            latency_deltas: {
              ping_forward_p99_ms_nvpn_minus_reference: {
                clean: $clean_forward_p99_reference_delta,
                stress: $stress_forward_p99_reference_delta,
                stress_minus_clean_ms: delta($stress_forward_p99_reference_delta; $clean_forward_p99_reference_delta)
              },
              ping_reverse_p99_ms_nvpn_minus_reference: {
                clean: $clean_reverse_p99_reference_delta,
                stress: $stress_reverse_p99_reference_delta,
                stress_minus_clean_ms: delta($stress_reverse_p99_reference_delta; $clean_reverse_p99_reference_delta)
              }
            },
            cpu_percent: {
              nvpn_local: {
                clean: num($clean.nvpn.local_cpu_percent),
                stress: num($stress.nvpn.local_cpu_percent),
                stress_minus_clean: delta($stress.nvpn.local_cpu_percent; $clean.nvpn.local_cpu_percent)
              },
              nvpn_remote: {
                clean: num($clean.nvpn.remote_cpu_percent),
                stress: num($stress.nvpn.remote_cpu_percent),
                stress_minus_clean: delta($stress.nvpn.remote_cpu_percent; $clean.nvpn.remote_cpu_percent)
              },
              reference_local: {
                clean: num($clean.reference.local_cpu_percent),
                stress: num($stress.reference.local_cpu_percent),
                stress_minus_clean: delta($stress.reference.local_cpu_percent; $clean.reference.local_cpu_percent)
              },
              reference_remote: {
                clean: num($clean.reference.remote_cpu_percent),
                stress: num($stress.reference.remote_cpu_percent),
                stress_minus_clean: delta($stress.reference.remote_cpu_percent; $clean.reference.remote_cpu_percent)
              }
            }
          }
      ]
  ' "$rows_json" >"$stress_deltas_json"

  write_summary_row \
    backend \
    nvpn_clean_forward_mbps nvpn_stress_forward_mbps nvpn_forward_stress_pct_of_clean \
    nvpn_clean_reverse_mbps nvpn_stress_reverse_mbps nvpn_reverse_stress_pct_of_clean \
    reference_clean_forward_mbps reference_stress_forward_mbps reference_forward_stress_pct_of_clean \
    reference_clean_reverse_mbps reference_stress_reverse_mbps reference_reverse_stress_pct_of_clean \
    nvpn_clean_forward_pct_of_reference nvpn_stress_forward_pct_of_reference nvpn_forward_reference_pct_point_delta \
    nvpn_clean_reverse_pct_of_reference nvpn_stress_reverse_pct_of_reference nvpn_reverse_reference_pct_point_delta \
    nvpn_clean_ping_forward_p99_ms nvpn_stress_ping_forward_p99_ms nvpn_ping_forward_p99_delta_ms \
    nvpn_clean_ping_reverse_p99_ms nvpn_stress_ping_reverse_p99_ms nvpn_ping_reverse_p99_delta_ms \
    clean_ping_forward_p99_nvpn_minus_reference_ms stress_ping_forward_p99_nvpn_minus_reference_ms ping_forward_p99_reference_delta_change_ms \
    clean_ping_reverse_p99_nvpn_minus_reference_ms stress_ping_reverse_p99_nvpn_minus_reference_ms ping_reverse_p99_reference_delta_change_ms \
    >"$STRESS_DELTAS_TSV"

  jq -r '
    .[]
    | [
        .backend,
        .nvpn.forward_mbps.clean,
        .nvpn.forward_mbps.stress,
        .nvpn.forward_mbps.stress_pct_of_clean,
        .nvpn.reverse_mbps.clean,
        .nvpn.reverse_mbps.stress,
        .nvpn.reverse_mbps.stress_pct_of_clean,
        .reference.forward_mbps.clean,
        .reference.forward_mbps.stress,
        .reference.forward_mbps.stress_pct_of_clean,
        .reference.reverse_mbps.clean,
        .reference.reverse_mbps.stress,
        .reference.reverse_mbps.stress_pct_of_clean,
        .ratios.forward_mbps_pct_of_reference.clean,
        .ratios.forward_mbps_pct_of_reference.stress,
        .ratios.forward_mbps_pct_of_reference.stress_minus_clean_points,
        .ratios.reverse_mbps_pct_of_reference.clean,
        .ratios.reverse_mbps_pct_of_reference.stress,
        .ratios.reverse_mbps_pct_of_reference.stress_minus_clean_points,
        .nvpn.ping_forward_p99_ms.clean,
        .nvpn.ping_forward_p99_ms.stress,
        .nvpn.ping_forward_p99_ms.stress_minus_clean_ms,
        .nvpn.ping_reverse_p99_ms.clean,
        .nvpn.ping_reverse_p99_ms.stress,
        .nvpn.ping_reverse_p99_ms.stress_minus_clean_ms,
        .latency_deltas.ping_forward_p99_ms_nvpn_minus_reference.clean,
        .latency_deltas.ping_forward_p99_ms_nvpn_minus_reference.stress,
        .latency_deltas.ping_forward_p99_ms_nvpn_minus_reference.stress_minus_clean_ms,
        .latency_deltas.ping_reverse_p99_ms_nvpn_minus_reference.clean,
        .latency_deltas.ping_reverse_p99_ms_nvpn_minus_reference.stress,
        .latency_deltas.ping_reverse_p99_ms_nvpn_minus_reference.stress_minus_clean_ms
      ]
    | @tsv
  ' "$stress_deltas_json" >>"$STRESS_DELTAS_TSV"

  local reliability_rows_json
  reliability_rows_json="$(mktemp "${TMPDIR:-/tmp}/nvpn-host-pair-reliability.XXXXXX")"
  jq '
    def num($v):
      if $v == null or $v == "" then null
      elif ($v | type) == "number" then $v
      else ($v | tonumber?)
      end;
    def count($v): (num($v) // 0);
    def arr($v): if $v == null then [] elif ($v | type) == "array" then $v else [] end;
    def reason($cond; $reason):
      if $cond then [$reason] else [] end;
    [
      .[]
      | . as $row
      | (
          []
          + reason($row.nvpn.safety_checks.direct_path_checked != true; "direct_path_unchecked")
          + reason($row.nvpn.safety_checks.pipeline_log_checked != true; "pipeline_log_unchecked")
          + reason($row.nvpn.safety_checks.counter_progress_checked != true; "counter_progress_unchecked")
          + reason(count($row.nvpn.safety_checks.iperf_forward_collapse_count) > 0; "iperf_forward_collapse")
          + reason(count($row.nvpn.safety_checks.iperf_reverse_collapse_count) > 0; "iperf_reverse_collapse")
          + reason($row.nvpn.fips_liveness.checked != true; "fips_liveness_unchecked")
          + reason($row.nvpn.fips_liveness.local_last_seen_age_secs == null; "missing_local_fips_seen_age")
          + reason($row.nvpn.fips_liveness.remote_last_seen_age_secs == null; "missing_remote_fips_seen_age")
          + reason($row.nvpn.fips_control_liveness.checked != true; "fips_control_liveness_unchecked")
          + reason($row.nvpn.fips_control_liveness.local_last_seen_age_secs == null; "missing_local_fips_control_seen_age")
          + reason($row.nvpn.fips_control_liveness.remote_last_seen_age_secs == null; "missing_remote_fips_control_seen_age")
          + reason($row.nvpn.fips_data_liveness.checked != true; "fips_data_liveness_unchecked")
          + reason($row.nvpn.fips_data_liveness.local_last_seen_age_secs == null; "missing_local_fips_data_seen_age")
          + reason($row.nvpn.fips_data_liveness.remote_last_seen_age_secs == null; "missing_remote_fips_data_seen_age")
          + reason(count($row.nvpn.rekey.local_stuck_count) > 0; "local_rekey_stuck")
          + reason(count($row.nvpn.rekey.remote_stuck_count) > 0; "remote_rekey_stuck")
          + reason(count($row.nvpn.direct_probe.local_overdue_count) > 0; "local_direct_probe_overdue")
          + reason(count($row.nvpn.direct_probe.remote_overdue_count) > 0; "remote_direct_probe_overdue")
          + (arr($row.nvpn.safety_checks.pipeline_hard_events) | map("pipeline_hard_event_" + .))
        ) as $failures
      | (
          []
          + reason($row.nvpn.rekey.local_in_progress == true; "local_rekey_in_progress")
          + reason($row.nvpn.rekey.remote_in_progress == true; "remote_rekey_in_progress")
          + reason($row.nvpn.rekey.local_draining == true; "local_rekey_draining")
          + reason($row.nvpn.rekey.remote_draining == true; "remote_rekey_draining")
          + reason(count($row.nvpn.direct_probe.local_pending_count) > 0; "local_direct_probe_pending")
          + reason(count($row.nvpn.direct_probe.remote_pending_count) > 0; "remote_direct_probe_pending")
          + reason(count($row.nvpn.nostr_traversal.local_failures) > 0; "local_nostr_traversal_failures")
          + reason(count($row.nvpn.nostr_traversal.remote_failures) > 0; "remote_nostr_traversal_failures")
          + reason($row.nvpn.nostr_traversal.local_in_cooldown == true; "local_nostr_traversal_cooldown")
          + reason($row.nvpn.nostr_traversal.remote_in_cooldown == true; "remote_nostr_traversal_cooldown")
        ) as $warnings
      | {
          mode: $row.mode,
          backend: $row.backend,
          cpu_stress_enabled: $row.cpu_stress_enabled,
          status: (
            if ($failures | length) > 0 then "fail"
            elif ($warnings | length) > 0 then "warn"
            else "pass"
            end
          ),
          failure_count: ($failures | length),
          warning_count: ($warnings | length),
          failures: $failures,
          warnings: $warnings,
          artifacts: $row.artifacts,
          metrics: {
            nvpn_forward_mbps: num($row.nvpn.forward_mbps),
            nvpn_reverse_mbps: num($row.nvpn.reverse_mbps),
            reference_forward_mbps: num($row.reference.forward_mbps),
            reference_reverse_mbps: num($row.reference.reverse_mbps),
            nvpn_forward_pct_of_reference: num($row.ratios.forward_mbps_pct_of_reference),
            nvpn_reverse_pct_of_reference: num($row.ratios.reverse_mbps_pct_of_reference)
          }
        }
    ]
  ' "$rows_json" >"$reliability_rows_json"

  write_summary_row \
    mode backend cpu_stress_enabled reliability_status failure_count warning_count \
    failure_reasons warning_reasons \
    nvpn_forward_mbps nvpn_reverse_mbps reference_forward_mbps reference_reverse_mbps \
    nvpn_forward_pct_of_reference nvpn_reverse_pct_of_reference \
    comparison_dir \
    >"$RELIABILITY_TSV"

  jq -r '
    .[]
    | [
        .mode,
        .backend,
        .cpu_stress_enabled,
        .status,
        .failure_count,
        .warning_count,
        (.failures | join(",")),
        (.warnings | join(",")),
        .metrics.nvpn_forward_mbps,
        .metrics.nvpn_reverse_mbps,
        .metrics.reference_forward_mbps,
        .metrics.reference_reverse_mbps,
        .metrics.nvpn_forward_pct_of_reference,
        .metrics.nvpn_reverse_pct_of_reference,
        .artifacts.comparison_dir
      ]
    | @tsv
  ' "$reliability_rows_json" >>"$RELIABILITY_TSV"

  jq -n \
    --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg run_dir "$RUN_DIR" \
    --arg manifest "$manifest" \
    --arg reliability_tsv "$RELIABILITY_TSV" \
    --argjson rows "$(cat "$reliability_rows_json")" \
    '{
       created_at: $created_at,
       artifacts: {
         run_dir: $run_dir,
         manifest_tsv: $manifest,
         matrix_reliability_tsv: $reliability_tsv
       },
       rows: $rows
     }' >"$RELIABILITY_JSON"

  jq -n \
    --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg run_dir "$RUN_DIR" \
    --arg manifest "$manifest" \
    --arg summary_tsv "$SUMMARY_TSV" \
    --arg stress_deltas_tsv "$STRESS_DELTAS_TSV" \
    --arg reliability_tsv "$RELIABILITY_TSV" \
    --arg reliability_json "$RELIABILITY_JSON" \
    --argjson rows "$(cat "$rows_json")" \
    --argjson stress_deltas "$(cat "$stress_deltas_json")" \
    --argjson reliability "$(cat "$reliability_rows_json")" \
    '{
       created_at: $created_at,
       artifacts: {
         run_dir: $run_dir,
         manifest_tsv: $manifest,
         matrix_summary_tsv: $summary_tsv,
         matrix_stress_deltas_tsv: $stress_deltas_tsv,
         matrix_reliability_tsv: $reliability_tsv,
         matrix_reliability_json: $reliability_json
       },
       rows: $rows,
       stress_deltas: $stress_deltas,
       reliability: $reliability
     }' >"$SUMMARY_JSON"
  rm -f "$rows_json" "$stress_deltas_json" "$reliability_rows_json"

  printf 'host-pair comparison run summary wrote %s, %s, %s, %s, and %s\n' \
    "$SUMMARY_TSV" "$STRESS_DELTAS_TSV" "$RELIABILITY_TSV" "$RELIABILITY_JSON" "$SUMMARY_JSON"
}

main "$@"
