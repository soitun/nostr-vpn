#!/usr/bin/env bash
# Summarize one Docker nvpn+FIPS soak samples.ndjson artifact.
set -euo pipefail

INPUT="${1:-}"
SUMMARY_TSV="${2:-}"
SUMMARY_JSON="${3:-}"

die() {
  printf 'nvpn+FIPS soak summary failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
usage: scripts/summarize-fips-soak-docker.sh <soak-output-dir|samples.ndjson> [summary.tsv] [summary.json]

Reads samples.ndjson from scripts/soak-fips-dataplane-docker.sh and writes a
single-row TSV plus a JSON summary. Defaults write soak-summary.tsv/json next to
samples.ndjson.
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  else
    shasum -a 256 "$path" | awk '{print $1}'
  fi
}

write_tsv_from_json() {
  local json="$1"
  jq -r '
    [
      "sample_count",
      "first_ts",
      "last_ts",
      "min_forward_mbps",
      "avg_forward_mbps",
      "max_forward_mbps",
      "min_reverse_mbps",
      "avg_reverse_mbps",
      "max_reverse_mbps",
      "max_forward_retrans",
      "max_reverse_retrans",
      "max_ping_loss_percent",
      "max_ping_avg_ms",
      "max_ping_p95_ms",
      "max_ping_p99_ms",
      "max_ping_max_ms",
      "max_fips_seen_age_secs",
      "max_fips_control_seen_age_secs",
      "max_fips_data_seen_age_secs",
      "max_rekey_stuck_count",
      "max_direct_probe_pending_count",
      "max_direct_probe_overdue_count",
      "max_nostr_traversal_failures",
      "max_cpu_percent",
      "node_a_fips_top_queue_wait",
      "node_b_fips_top_queue_wait",
      "node_a_fips_top_priority_wait",
      "node_b_fips_top_priority_wait",
      "node_a_nvpn_top_queue_wait",
      "node_b_nvpn_top_queue_wait",
      "node_a_fips_hard_events",
      "node_b_fips_hard_events",
      "node_a_nvpn_hard_events",
      "node_b_nvpn_hard_events",
      "samples_sha256"
    ],
    [
      .summary.sample_count,
      .summary.first_ts,
      .summary.last_ts,
      .summary.min_forward_mbps,
      .summary.avg_forward_mbps,
      .summary.max_forward_mbps,
      .summary.min_reverse_mbps,
      .summary.avg_reverse_mbps,
      .summary.max_reverse_mbps,
      .summary.max_forward_retrans,
      .summary.max_reverse_retrans,
      .summary.max_ping_loss_percent,
      .summary.max_ping_avg_ms,
      .summary.max_ping_p95_ms,
      .summary.max_ping_p99_ms,
      .summary.max_ping_max_ms,
      .summary.max_fips_seen_age_secs,
      .summary.max_fips_control_seen_age_secs,
      .summary.max_fips_data_seen_age_secs,
      .summary.max_rekey_stuck_count,
      .summary.max_direct_probe_pending_count,
      .summary.max_direct_probe_overdue_count,
      .summary.max_nostr_traversal_failures,
      .summary.max_cpu_percent,
      .pipeline.node_a_fips_top_queue_wait,
      .pipeline.node_b_fips_top_queue_wait,
      .pipeline.node_a_fips_top_priority_wait,
      .pipeline.node_b_fips_top_priority_wait,
      .pipeline.node_a_nvpn_top_queue_wait,
      .pipeline.node_b_nvpn_top_queue_wait,
      .pipeline.node_a_fips_hard_events,
      .pipeline.node_b_fips_hard_events,
      .pipeline.node_a_nvpn_hard_events,
      .pipeline.node_b_nvpn_hard_events,
      .source.samples_sha256
    ]
    | @tsv
  ' "$json"
}

main() {
  [[ -n "$INPUT" ]] || {
    usage
    die "soak output directory or samples.ndjson is required"
  }

  need_cmd jq

  local samples output_dir samples_sha tmp_json
  if [[ -d "$INPUT" ]]; then
    output_dir="$INPUT"
    samples="$INPUT/samples.ndjson"
  else
    samples="$INPUT"
    output_dir="$(cd "$(dirname "$samples")" && pwd)"
  fi
  [[ -f "$samples" ]] || die "samples.ndjson not found: $samples"
  [[ -s "$samples" ]] || die "samples.ndjson is empty: $samples"

  SUMMARY_TSV="${SUMMARY_TSV:-$output_dir/soak-summary.tsv}"
  SUMMARY_JSON="${SUMMARY_JSON:-$output_dir/soak-summary.json}"
  samples_sha="$(sha256_file "$samples")"
  tmp_json="${SUMMARY_JSON}.tmp"

  jq -s \
    --arg samples_path "$samples" \
    --arg output_dir "$output_dir" \
    --arg samples_sha "$samples_sha" '
    . as $samples
    | if ($samples | length) == 0 then
        error("samples.ndjson has no JSON rows")
      else . end
    | def nums(f): [$samples[] | f | numbers];
      def max_of(f): (nums(f) | max // null);
      def min_of(f): (nums(f) | min // null);
      def avg_of(f): (nums(f) as $xs | if ($xs | length) == 0 then null else (($xs | add) / ($xs | length)) end);
      def top_wait($node; $kind; $priority_only):
        [
          $samples[]
          | (.pipeline[$node][$kind].queue_wait_ms // {})
          | to_entries[]
          | select(($priority_only | not) or (.key | contains("priority")))
          | (.value.max_observed // {}) as $w
          | {
              metric: .key,
              p95_ms: $w.p95_ms,
              p99_ms: $w.p99_ms,
              max_ms: $w.max_ms,
              allmax_ms: $w.allmax_ms,
              score: ([$w.p99_ms, $w.p95_ms, $w.max_ms, $w.allmax_ms] | map(select(type == "number")) | max // -1)
            }
          | select(.score >= 0)
        ]
        | max_by(.score) // null;
      def wait_string($w):
        if $w == null then "" else
          "\($w.metric):p95_ms=\($w.p95_ms // "null"),p99_ms=\($w.p99_ms // "null"),max_ms=\($w.max_ms // "null"),allmax_ms=\($w.allmax_ms // "null")"
        end;
      def hard_event_names:
        [
          "connected_udp_activation_failed",
          "encrypt_worker_queue_full",
          "encrypt_worker_bulk_dropped",
          "decrypt_worker_queue_full",
          "decrypt_worker_bulk_dropped",
          "decrypt_worker_register_full",
          "decrypt_worker_priority_dropped",
          "decrypt_fallback_bulk_dropped",
          "decrypt_fallback_priority_dropped",
          "fmp_aead_completion_aead_failed",
          "fsp_aead_completion_aead_failed",
          "fsp_aead_completion_epoch_mismatch",
          "pending_tun_destination_dropped",
          "pending_tun_packet_dropped",
          "pending_endpoint_destination_dropped",
          "pending_endpoint_packet_dropped",
          "endpoint_event_backlog_high",
          "endpoint_event_bulk_dropped",
          "transport_channel_backlog_high",
          "transport_bulk_dropped",
          "udp_send_bulk_dropped",
          "nvpn_tun_to_mesh_bulk_dropped",
          "nvpn_tun_to_mesh_bulk_dropped_batches",
          "nvpn_tun_to_mesh_bulk_dropped_packet_cap",
          "nvpn_tun_to_mesh_bulk_dropped_channel_full"
        ];
      def hard_events($node; $kind):
        [
          hard_event_names[] as $name
          | {
              name: $name,
              seen: any($samples[]; (.pipeline[$node][$kind].seen[$name] // false)),
              max_rate_per_sec: max_of(.pipeline[$node][$kind].max_rates_per_sec[$name]),
              total: max_of(.pipeline[$node][$kind].max_totals[$name])
            }
          | select(.seen)
          | "\(.name):max_rate_per_sec=\(.max_rate_per_sec // 0),total=\(.total // 0)"
        ]
        | join(";");
      {
        source: {
          output_dir: $output_dir,
          samples_path: $samples_path,
          samples_sha256: $samples_sha
        },
        summary: {
          sample_count: ($samples | length),
          first_ts: ($samples[0].ts // ""),
          last_ts: ($samples[-1].ts // ""),
          min_forward_mbps: min_of(.iperf.forward_mbps),
          avg_forward_mbps: avg_of(.iperf.forward_mbps),
          max_forward_mbps: max_of(.iperf.forward_mbps),
          min_reverse_mbps: min_of(.iperf.reverse_mbps),
          avg_reverse_mbps: avg_of(.iperf.reverse_mbps),
          max_reverse_mbps: max_of(.iperf.reverse_mbps),
          max_forward_retrans: max_of(.iperf.forward_retrans),
          max_reverse_retrans: max_of(.iperf.reverse_retrans),
          max_ping_loss_percent: max_of([.ping.a_to_b.loss_percent, .ping.b_to_a.loss_percent][]),
          max_ping_avg_ms: max_of([.ping.a_to_b.avg_ms, .ping.b_to_a.avg_ms][]),
          max_ping_p95_ms: max_of([.ping.a_to_b.p95_ms, .ping.b_to_a.p95_ms][]),
          max_ping_p99_ms: max_of([.ping.a_to_b.p99_ms, .ping.b_to_a.p99_ms][]),
          max_ping_max_ms: max_of([.ping.a_to_b.max_ms, .ping.b_to_a.max_ms][]),
          max_fips_seen_age_secs: max_of([.fips.node_a_last_fips_seen_age_secs, .fips.node_b_last_fips_seen_age_secs][]),
          max_fips_control_seen_age_secs: max_of([.fips.node_a_last_fips_control_seen_age_secs, .fips.node_b_last_fips_control_seen_age_secs][]),
          max_fips_data_seen_age_secs: max_of([.fips.node_a_last_fips_data_seen_age_secs, .fips.node_b_last_fips_data_seen_age_secs][]),
          max_rekey_stuck_count: max_of([.fips.node_a_rekey_stuck_count, .fips.node_b_rekey_stuck_count][]),
          max_direct_probe_pending_count: max_of([.fips.node_a_direct_probe_pending_count, .fips.node_b_direct_probe_pending_count][]),
          max_direct_probe_overdue_count: max_of([.fips.node_a_direct_probe_overdue_count, .fips.node_b_direct_probe_overdue_count][]),
          max_nostr_traversal_failures: max_of([.fips.node_a_nostr_traversal_failures, .fips.node_b_nostr_traversal_failures][]),
          max_cpu_percent: max_of([.cpu.node_a_percent, .cpu.node_b_percent][])
        },
        pipeline: {
          node_a_fips_top_queue_wait: wait_string(top_wait("node_a"; "fips"; false)),
          node_b_fips_top_queue_wait: wait_string(top_wait("node_b"; "fips"; false)),
          node_a_fips_top_priority_wait: wait_string(top_wait("node_a"; "fips"; true)),
          node_b_fips_top_priority_wait: wait_string(top_wait("node_b"; "fips"; true)),
          node_a_nvpn_top_queue_wait: wait_string(top_wait("node_a"; "nvpn"; false)),
          node_b_nvpn_top_queue_wait: wait_string(top_wait("node_b"; "nvpn"; false)),
          node_a_fips_hard_events: hard_events("node_a"; "fips"),
          node_b_fips_hard_events: hard_events("node_b"; "fips"),
          node_a_nvpn_hard_events: hard_events("node_a"; "nvpn"),
          node_b_nvpn_hard_events: hard_events("node_b"; "nvpn")
        }
      }
  ' "$samples" >"$tmp_json"
  mv "$tmp_json" "$SUMMARY_JSON"
  write_tsv_from_json "$SUMMARY_JSON" >"$SUMMARY_TSV"

  printf 'wrote %s and %s\n' "$SUMMARY_TSV" "$SUMMARY_JSON"
}

main "$@"
