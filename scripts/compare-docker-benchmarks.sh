#!/usr/bin/env bash
# Compare one simple nvpn Docker benchmark row against one Docker reference row.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

NVPN_INPUT="${NVPN_DOCKER_COMPARISON_NVPN_DIR:-${1:-}}"
REFERENCE_INPUT="${NVPN_DOCKER_COMPARISON_REFERENCE_DIR:-${2:-}}"
OUTPUT_DIR="${NVPN_DOCKER_COMPARISON_OUTPUT_DIR:-${3:-$ROOT_DIR/artifacts/docker-reference-comparisons/$(date -u +%Y%m%dT%H%M%SZ)}}"
NVPN_BACKEND="${NVPN_DOCKER_COMPARISON_NVPN_BACKEND:-nvpn}"
NVPN_THREADS="${NVPN_DOCKER_COMPARISON_NVPN_THREADS:-}"
NVPN_LABEL="${NVPN_DOCKER_COMPARISON_NVPN_LABEL:-nvpn}"
REFERENCE_BACKEND="${NVPN_DOCKER_COMPARISON_REFERENCE_BACKEND:-boringtun}"
REFERENCE_THREADS="${NVPN_DOCKER_COMPARISON_REFERENCE_THREADS:-}"
REFERENCE_LABEL="${NVPN_DOCKER_COMPARISON_REFERENCE_LABEL:-reference}"
MIN_THROUGHPUT_PCT="${NVPN_DOCKER_COMPARISON_MIN_THROUGHPUT_PCT:-90}"
MAX_RETRANS_PCT="${NVPN_DOCKER_COMPARISON_MAX_RETRANS_PCT:-150}"
MAX_LOSS_DELTA_PCT="${NVPN_DOCKER_COMPARISON_MAX_LOSS_DELTA_PCT:-1}"
MAX_STRESS_UDP_LOSS_DELTA_PCT="${NVPN_DOCKER_COMPARISON_STRESS_UDP_LOSS_DELTA_PCT:-5}"
MAX_PING_AVG_DELTA_MS="${NVPN_DOCKER_COMPARISON_MAX_PING_AVG_DELTA_MS:-1}"
MAX_PING_P95_DELTA_MS="${NVPN_DOCKER_COMPARISON_MAX_PING_P95_DELTA_MS:-$MAX_PING_AVG_DELTA_MS}"
MAX_PING_P99_DELTA_MS="${NVPN_DOCKER_COMPARISON_MAX_PING_P99_DELTA_MS:-2}"
MAX_PING_MAX_DELTA_MS="${NVPN_DOCKER_COMPARISON_MAX_PING_MAX_DELTA_MS:-5}"
REQUIRE_NVPN_UDP_ZERO_LOSS="${NVPN_DOCKER_COMPARISON_REQUIRE_NVPN_UDP_ZERO_LOSS:-1}"
REQUIRE_NVPN_PING_ZERO_LOSS="${NVPN_DOCKER_COMPARISON_REQUIRE_NVPN_PING_ZERO_LOSS:-1}"
ENFORCE_THRESHOLDS="${NVPN_DOCKER_COMPARISON_ENFORCE_THRESHOLDS:-0}"

die() {
  printf 'docker benchmark comparison failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
usage: scripts/compare-docker-benchmarks.sh <nvpn-artifact-dir|summary.tsv> <reference-artifact-dir|summary.tsv> [output-dir]

Env alternatives:
  NVPN_DOCKER_COMPARISON_NVPN_DIR=<dir-or-summary>
  NVPN_DOCKER_COMPARISON_REFERENCE_DIR=<dir-or-summary>
  NVPN_DOCKER_COMPARISON_OUTPUT_DIR=<dir>
  NVPN_DOCKER_COMPARISON_NVPN_BACKEND=nvpn
  NVPN_DOCKER_COMPARISON_NVPN_LABEL=nvpn
  NVPN_DOCKER_COMPARISON_REFERENCE_BACKEND=boringtun
  NVPN_DOCKER_COMPARISON_REFERENCE_LABEL=reference
  NVPN_DOCKER_COMPARISON_REFERENCE_THREADS=1
  NVPN_DOCKER_COMPARISON_MIN_THROUGHPUT_PCT=90
  NVPN_DOCKER_COMPARISON_MAX_RETRANS_PCT=150
  NVPN_DOCKER_COMPARISON_MAX_LOSS_DELTA_PCT=1
  NVPN_DOCKER_COMPARISON_STRESS_UDP_LOSS_DELTA_PCT=5
  NVPN_DOCKER_COMPARISON_MAX_PING_AVG_DELTA_MS=1
  NVPN_DOCKER_COMPARISON_MAX_PING_P95_DELTA_MS=1
  NVPN_DOCKER_COMPARISON_MAX_PING_P99_DELTA_MS=2
  NVPN_DOCKER_COMPARISON_MAX_PING_MAX_DELTA_MS=5
  NVPN_DOCKER_COMPARISON_REQUIRE_NVPN_UDP_ZERO_LOSS=1
  NVPN_DOCKER_COMPARISON_REQUIRE_NVPN_PING_ZERO_LOSS=1
  NVPN_DOCKER_COMPARISON_ENFORCE_THRESHOLDS=0

Inputs are summary.tsv artifacts from scripts/perf-docker.sh and
scripts/perf-docker-boringtun.sh.

Outputs:
  comparison.tsv, ratios.tsv, thresholds.tsv, comparison.json
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

is_true_value() {
  case "${1:-}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) return 0 ;;
    *) return 1 ;;
  esac
}

summary_file() {
  local input="$1"
  [[ -n "$input" ]] || {
    usage
    die "input artifact directory or summary.tsv is required"
  }
  if [[ -d "$input" ]]; then
    [[ -f "$input/summary.tsv" ]] || die "missing summary.tsv in $input"
    printf '%s/summary.tsv\n' "$input"
  elif [[ -f "$input" ]]; then
    printf '%s\n' "$input"
  else
    die "input not found: $input"
  fi
}

metadata_value() {
  local input="$1"
  local filter="$2"
  local metadata
  if [[ -d "$input" ]]; then
    metadata="$input/metadata.json"
  else
    metadata="$(dirname "$input")/metadata.json"
  fi
  [[ -f "$metadata" ]] || {
    printf '\n'
    return
  }
  jq -r "$filter | if . == null then \"\" else . end" "$metadata"
}

tsv_value() {
  local file="$1"
  local field="$2"
  local backend="$3"
  local threads="$4"
  awk -v want="$field" -v backend="$backend" -v threads="$threads" -F '\t' '
    NR == 1 {
      for (i = 1; i <= NF; i++) {
        if ($i == want) field_idx = i
        if ($i == "backend") backend_idx = i
        if ($i == "threads") threads_idx = i
      }
      next
    }
    field_idx && backend_idx && NF {
      if ($backend_idx == backend && (threads == "" || (threads_idx && $threads_idx == threads))) {
        print $field_idx
        found = 1
        exit
      }
    }
    END {
      if (!field_idx) exit 2
      if (!found) exit 3
    }' "$file"
}

optional_tsv_value() {
  tsv_value "$@" 2>/dev/null || true
}

require_bidirectional_summary() {
  local summary="$1"
  local label="$2"
  local backend="$3"
  local threads="$4"
  local field value
  local missing=()
  for field in \
    forward_direction reverse_direction \
    tcp_single_mbps tcp_single_retrans tcp_4_mbps tcp_4_retrans \
    tcp_8_mbps tcp_8_retrans udp_200_mbps udp_200_loss_pct \
    udp_1000_mbps udp_1000_loss_pct \
    tcp_single_b_to_a_mbps tcp_single_b_to_a_retrans \
    tcp_4_b_to_a_mbps tcp_4_b_to_a_retrans \
    tcp_8_b_to_a_mbps tcp_8_b_to_a_retrans \
    udp_200_b_to_a_mbps udp_200_b_to_a_loss_pct \
    udp_1000_b_to_a_mbps udp_1000_b_to_a_loss_pct; do
    value="$(tsv_value "$summary" "$field" "$backend" "$threads" 2>/dev/null || true)"
    [[ -n "$value" ]] || missing+=("$field")
  done
  (( ${#missing[@]} == 0 )) \
    || die "$label summary is missing mandatory bidirectional fields: ${missing[*]}"

  local forward_direction reverse_direction
  forward_direction="$(tsv_value "$summary" forward_direction "$backend" "$threads")"
  reverse_direction="$(tsv_value "$summary" reverse_direction "$backend" "$threads")"
  [[ "$forward_direction" == "a_to_b" && "$reverse_direction" == "b_to_a" ]] \
    || die "$label summary directions must be a_to_b and b_to_a, got $forward_direction and $reverse_direction"
}

cpu_phase_file() {
  local summary="$1"
  local backend="$2"
  local threads="$3"
  local raw_dir file
  raw_dir="$(tsv_value "$summary" raw_dir "$backend" "$threads" 2>/dev/null || true)"
  [[ -n "$raw_dir" ]] || return 0
  for file in \
    "$raw_dir/${backend}-cpu-phases.tsv" \
    "$raw_dir/${backend}-daemon-cpu-phases.tsv" \
    "$raw_dir/nvpn-daemon-cpu-phases.tsv" \
    "$raw_dir"/*cpu-phases.tsv \
    "$raw_dir"/*daemon-cpu-phases.tsv; do
    if [[ -f "$file" ]]; then
      printf '%s\n' "$file"
      return 0
    fi
  done
}

cpu_phase_value() {
  local summary="$1"
  local backend="$2"
  local threads="$3"
  local phase="$4"
  local field="$5"
  local file
  file="$(cpu_phase_file "$summary" "$backend" "$threads")"
  [[ -n "$file" ]] || {
    printf '\n'
    return 0
  }
  awk -v phase="$phase" -v service="both" -v want="$field" -F '\t' '
    NR == 1 {
      for (i = 1; i <= NF; i++) {
        if ($i == want) field_idx = i
        if ($i == "phase") phase_idx = i
        if ($i == "service") service_idx = i
      }
      next
    }
    field_idx && phase_idx && service_idx && $phase_idx == phase && $service_idx == service {
      print $field_idx
      found = 1
      exit
    }
    END {
      if (!field_idx || !found) print ""
    }' "$file"
}

ratio_percent() {
  local actual="$1"
  local reference="$2"
  awk -v actual="$actual" -v reference="$reference" '
    BEGIN {
      if (actual == "" || reference == "" || reference + 0 == 0) {
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
      if (actual == "" || reference == "") {
        print "";
      } else {
        printf "%.3f", (actual + 0) - (reference + 0);
      }
    }'
}

threshold_higher_pct_status() {
  local actual="$1"
  local reference="$2"
  local min_pct="$3"
  awk -v actual="$actual" -v reference="$reference" -v min_pct="$min_pct" '
    BEGIN {
      if (actual == "" || reference == "" || reference + 0 == 0 || min_pct == "") {
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
      if (actual == "" || reference == "" || max_pct == "") {
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
      if (actual == "" || reference == "" || max_delta == "") {
        print "unknown";
      } else if ((actual + 0) - (reference + 0) <= max_delta + 0) {
        print "pass";
      } else {
        print "fail";
      }
    }'
}

threshold_zero_status() {
  local actual="$1"
  awk -v actual="$actual" '
    BEGIN {
      if (actual == "") {
        print "unknown";
      } else if ((actual + 0) == 0) {
        print "pass";
      } else {
        print "fail";
      }
    }'
}

write_row() {
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

write_normalized_summary_row() {
  local label="$1"
  local source="$2"
  local backend="$3"
  local threads="$4"
  local input="${5:-$source}"
  local pipeline_trace_enabled pipeline_trace_interval_secs
  pipeline_trace_enabled="$(metadata_value "$input" '.pipeline_trace.enabled')"
  pipeline_trace_interval_secs="$(metadata_value "$input" '.pipeline_trace.interval_secs')"
  write_row \
    "$label" \
    "$source" \
    "$(tsv_value "$source" backend "$backend" "$threads")" \
    "$(tsv_value "$source" threads "$backend" "$threads")" \
    "$(tsv_value "$source" duration_secs "$backend" "$threads")" \
    "$(metadata_value "$input" '.cpu_stress.enabled')" \
    "$(metadata_value "$input" '.cpu_stress.sides')" \
    "$(metadata_value "$input" '.cpu_stress.local_workers')" \
    "$(metadata_value "$input" '.cpu_stress.remote_workers')" \
    "$pipeline_trace_enabled" \
    "$pipeline_trace_interval_secs" \
    "$(tsv_value "$source" forward_direction "$backend" "$threads")" \
    "$(tsv_value "$source" reverse_direction "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_single_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_single_retrans "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_4_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_4_retrans "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_8_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_8_retrans "$backend" "$threads")" \
    "$(tsv_value "$source" udp_200_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" udp_200_loss_pct "$backend" "$threads")" \
    "$(tsv_value "$source" udp_1000_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" udp_1000_loss_pct "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_single_b_to_a_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_single_b_to_a_retrans "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_4_b_to_a_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_4_b_to_a_retrans "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_8_b_to_a_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" tcp_8_b_to_a_retrans "$backend" "$threads")" \
    "$(tsv_value "$source" udp_200_b_to_a_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" udp_200_b_to_a_loss_pct "$backend" "$threads")" \
    "$(tsv_value "$source" udp_1000_b_to_a_mbps "$backend" "$threads")" \
    "$(tsv_value "$source" udp_1000_b_to_a_loss_pct "$backend" "$threads")" \
    "$(tsv_value "$source" ping_loss_pct "$backend" "$threads")" \
    "$(tsv_value "$source" ping_avg_ms "$backend" "$threads")" \
    "$(optional_tsv_value "$source" ping_mdev_ms "$backend" "$threads")" \
    "$(optional_tsv_value "$source" ping_p95_ms "$backend" "$threads")" \
    "$(optional_tsv_value "$source" ping_p99_ms "$backend" "$threads")" \
    "$(optional_tsv_value "$source" ping_max_ms "$backend" "$threads")" \
    "$(cpu_phase_value "$source" "$backend" "$threads" tcp-single cpu_seconds_per_gbyte)" \
    "$(cpu_phase_value "$source" "$backend" "$threads" tcp-4 cpu_seconds_per_gbyte)" \
    "$(cpu_phase_value "$source" "$backend" "$threads" tcp-8 cpu_seconds_per_gbyte)" \
    "$(cpu_phase_value "$source" "$backend" "$threads" udp-200 cpu_seconds_per_gbyte)" \
    "$(cpu_phase_value "$source" "$backend" "$threads" udp-1000 cpu_seconds_per_gbyte)" \
    "$(tsv_value "$source" raw_dir "$backend" "$threads")"
}

write_metric_ratio() {
  local metric="$1"
  local unit="$2"
  local better_when="$3"
  local nvpn_summary="$4"
  local reference_summary="$5"
  local nvpn_value reference_value
  nvpn_value="$(tsv_value "$nvpn_summary" "$metric" "$NVPN_BACKEND" "$NVPN_THREADS")"
  reference_value="$(tsv_value "$reference_summary" "$metric" "$REFERENCE_BACKEND" "$REFERENCE_THREADS")"
  write_row \
    "$metric" \
    "$unit" \
    "$better_when" \
    "$nvpn_value" \
    "$reference_value" \
    "$(ratio_percent "$nvpn_value" "$reference_value")" \
    "$(delta_value "$nvpn_value" "$reference_value")"
}

write_optional_metric_ratio() {
  local metric="$1"
  local unit="$2"
  local better_when="$3"
  local nvpn_summary="$4"
  local reference_summary="$5"
  local nvpn_value reference_value
  nvpn_value="$(optional_tsv_value "$nvpn_summary" "$metric" "$NVPN_BACKEND" "$NVPN_THREADS")"
  reference_value="$(optional_tsv_value "$reference_summary" "$metric" "$REFERENCE_BACKEND" "$REFERENCE_THREADS")"
  write_row \
    "$metric" \
    "$unit" \
    "$better_when" \
    "$nvpn_value" \
    "$reference_value" \
    "$(ratio_percent "$nvpn_value" "$reference_value")" \
    "$(delta_value "$nvpn_value" "$reference_value")"
}

write_cpu_metric_ratio() {
  local metric="$1"
  local phase="$2"
  local nvpn_summary="$3"
  local reference_summary="$4"
  local nvpn_value reference_value
  nvpn_value="$(cpu_phase_value "$nvpn_summary" "$NVPN_BACKEND" "$NVPN_THREADS" "$phase" cpu_seconds_per_gbyte)"
  reference_value="$(cpu_phase_value "$reference_summary" "$REFERENCE_BACKEND" "$REFERENCE_THREADS" "$phase" cpu_seconds_per_gbyte)"
  write_row \
    "$metric" \
    sec_per_gb \
    lower \
    "$nvpn_value" \
    "$reference_value" \
    "$(ratio_percent "$nvpn_value" "$reference_value")" \
    "$(delta_value "$nvpn_value" "$reference_value")"
}

write_threshold_higher_pct() {
  local check="$1"
  local metric="$2"
  local min_pct="$3"
  local nvpn_summary="$4"
  local reference_summary="$5"
  local nvpn_value reference_value pct status
  nvpn_value="$(tsv_value "$nvpn_summary" "$metric" "$NVPN_BACKEND" "$NVPN_THREADS")"
  reference_value="$(tsv_value "$reference_summary" "$metric" "$REFERENCE_BACKEND" "$REFERENCE_THREADS")"
  pct="$(ratio_percent "$nvpn_value" "$reference_value")"
  status="$(threshold_higher_pct_status "$nvpn_value" "$reference_value" "$min_pct")"
  write_row \
    "$check" \
    "$metric" \
    "$status" \
    "$nvpn_value" \
    "$reference_value" \
    ">=$min_pct%" \
    "${pct:+$pct%}"
}

write_threshold_lower_pct() {
  local check="$1"
  local metric="$2"
  local max_pct="$3"
  local nvpn_summary="$4"
  local reference_summary="$5"
  local nvpn_value reference_value pct status
  nvpn_value="$(tsv_value "$nvpn_summary" "$metric" "$NVPN_BACKEND" "$NVPN_THREADS")"
  reference_value="$(tsv_value "$reference_summary" "$metric" "$REFERENCE_BACKEND" "$REFERENCE_THREADS")"
  pct="$(ratio_percent "$nvpn_value" "$reference_value")"
  status="$(threshold_lower_pct_status "$nvpn_value" "$reference_value" "$max_pct")"
  write_row \
    "$check" \
    "$metric" \
    "$status" \
    "$nvpn_value" \
    "$reference_value" \
    "<=$max_pct%" \
    "${pct:+$pct%}"
}

write_threshold_lower_delta() {
  local check="$1"
  local metric="$2"
  local max_delta="$3"
  local unit="$4"
  local nvpn_summary="$5"
  local reference_summary="$6"
  local nvpn_value reference_value delta status
  nvpn_value="$(tsv_value "$nvpn_summary" "$metric" "$NVPN_BACKEND" "$NVPN_THREADS")"
  reference_value="$(tsv_value "$reference_summary" "$metric" "$REFERENCE_BACKEND" "$REFERENCE_THREADS")"
  delta="$(delta_value "$nvpn_value" "$reference_value")"
  status="$(threshold_lower_delta_status "$nvpn_value" "$reference_value" "$max_delta")"
  write_row \
    "$check" \
    "$metric" \
    "$status" \
    "$nvpn_value" \
    "$reference_value" \
    "<=reference+$max_delta$unit" \
    "${delta:+$delta$unit}"
}

write_optional_threshold_lower_delta() {
  local check="$1"
  local metric="$2"
  local max_delta="$3"
  local unit="$4"
  local nvpn_summary="$5"
  local reference_summary="$6"
  local nvpn_value reference_value delta status
  nvpn_value="$(optional_tsv_value "$nvpn_summary" "$metric" "$NVPN_BACKEND" "$NVPN_THREADS")"
  reference_value="$(optional_tsv_value "$reference_summary" "$metric" "$REFERENCE_BACKEND" "$REFERENCE_THREADS")"
  delta="$(delta_value "$nvpn_value" "$reference_value")"
  status="$(threshold_lower_delta_status "$nvpn_value" "$reference_value" "$max_delta")"
  write_row \
    "$check" \
    "$metric" \
    "$status" \
    "$nvpn_value" \
    "$reference_value" \
    "<=reference+$max_delta$unit" \
    "${delta:+$delta$unit}"
}

write_threshold_zero() {
  local check="$1"
  local metric="$2"
  local nvpn_summary="$3"
  local nvpn_value status
  nvpn_value="$(tsv_value "$nvpn_summary" "$metric" "$NVPN_BACKEND" "$NVPN_THREADS")"
  status="$(threshold_zero_status "$nvpn_value")"
  write_row \
    "$check" \
    "$metric" \
    "$status" \
    "$nvpn_value" \
    "" \
    "==0" \
    ""
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
  nvpn_summary="$(summary_file "$NVPN_INPUT")"
  reference_summary="$(summary_file "$REFERENCE_INPUT")"
  require_bidirectional_summary "$nvpn_summary" "$NVPN_LABEL" "$NVPN_BACKEND" "$NVPN_THREADS"
  require_bidirectional_summary "$reference_summary" "$REFERENCE_LABEL" "$REFERENCE_BACKEND" "$REFERENCE_THREADS"
  mkdir -p "$OUTPUT_DIR"

  local nvpn_stress_enabled nvpn_stress_sides nvpn_stress_local_workers nvpn_stress_remote_workers
  local reference_stress_enabled reference_stress_sides reference_stress_local_workers reference_stress_remote_workers
  local nvpn_pipeline_trace_enabled nvpn_pipeline_trace_interval_secs
  local reference_pipeline_trace_enabled reference_pipeline_trace_interval_secs
  local nvpn_iperf_socket_buffer nvpn_iperf_udp1000_parallel nvpn_iperf_udp1000_bandwidth nvpn_iperf_udp1000_per_stream_bandwidth
  local reference_iperf_socket_buffer reference_iperf_udp1000_parallel reference_iperf_udp1000_bandwidth reference_iperf_udp1000_per_stream_bandwidth
  local nvpn_extra_connect_env reference_extra_connect_env
  local nvpn_source_head nvpn_source_dirty nvpn_fips_patch_enabled nvpn_fips_patch_head nvpn_fips_patch_dirty
  local reference_source_head reference_source_dirty reference_fips_patch_enabled reference_fips_patch_head reference_fips_patch_dirty
  local pipeline_trace_mismatch
  nvpn_stress_enabled="$(metadata_value "$NVPN_INPUT" '.cpu_stress.enabled')"
  nvpn_stress_sides="$(metadata_value "$NVPN_INPUT" '.cpu_stress.sides')"
  nvpn_stress_local_workers="$(metadata_value "$NVPN_INPUT" '.cpu_stress.local_workers')"
  nvpn_stress_remote_workers="$(metadata_value "$NVPN_INPUT" '.cpu_stress.remote_workers')"
  reference_stress_enabled="$(metadata_value "$REFERENCE_INPUT" '.cpu_stress.enabled')"
  reference_stress_sides="$(metadata_value "$REFERENCE_INPUT" '.cpu_stress.sides')"
  reference_stress_local_workers="$(metadata_value "$REFERENCE_INPUT" '.cpu_stress.local_workers')"
  reference_stress_remote_workers="$(metadata_value "$REFERENCE_INPUT" '.cpu_stress.remote_workers')"
  nvpn_pipeline_trace_enabled="$(metadata_value "$NVPN_INPUT" '.pipeline_trace.enabled')"
  nvpn_pipeline_trace_interval_secs="$(metadata_value "$NVPN_INPUT" '.pipeline_trace.interval_secs')"
  reference_pipeline_trace_enabled="$(metadata_value "$REFERENCE_INPUT" '.pipeline_trace.enabled')"
  reference_pipeline_trace_interval_secs="$(metadata_value "$REFERENCE_INPUT" '.pipeline_trace.interval_secs')"
  nvpn_iperf_socket_buffer="$(metadata_value "$NVPN_INPUT" '.iperf.socket_buffer')"
  nvpn_iperf_udp1000_parallel="$(metadata_value "$NVPN_INPUT" '.iperf.udp1000_parallel')"
  nvpn_iperf_udp1000_bandwidth="$(metadata_value "$NVPN_INPUT" '.iperf.udp1000_bandwidth')"
  nvpn_iperf_udp1000_per_stream_bandwidth="$(metadata_value "$NVPN_INPUT" '.iperf.udp1000_per_stream_bandwidth')"
  reference_iperf_socket_buffer="$(metadata_value "$REFERENCE_INPUT" '.iperf.socket_buffer')"
  reference_iperf_udp1000_parallel="$(metadata_value "$REFERENCE_INPUT" '.iperf.udp1000_parallel')"
  reference_iperf_udp1000_bandwidth="$(metadata_value "$REFERENCE_INPUT" '.iperf.udp1000_bandwidth')"
  reference_iperf_udp1000_per_stream_bandwidth="$(metadata_value "$REFERENCE_INPUT" '.iperf.udp1000_per_stream_bandwidth')"
  nvpn_extra_connect_env="$(metadata_value "$NVPN_INPUT" '.run_env.extra_connect_env')"
  reference_extra_connect_env="$(metadata_value "$REFERENCE_INPUT" '.run_env.extra_connect_env')"
  nvpn_source_head="$(metadata_value "$NVPN_INPUT" '.source.nvpn.git_head')"
  nvpn_source_dirty="$(metadata_value "$NVPN_INPUT" '.source.nvpn.dirty')"
  nvpn_fips_patch_enabled="$(metadata_value "$NVPN_INPUT" '.source.local_fips_patch.enabled')"
  nvpn_fips_patch_head="$(metadata_value "$NVPN_INPUT" '.source.local_fips_patch.git_head')"
  nvpn_fips_patch_dirty="$(metadata_value "$NVPN_INPUT" '.source.local_fips_patch.dirty')"
  reference_source_head="$(metadata_value "$REFERENCE_INPUT" '.source.nvpn.git_head')"
  reference_source_dirty="$(metadata_value "$REFERENCE_INPUT" '.source.nvpn.dirty')"
  reference_fips_patch_enabled="$(metadata_value "$REFERENCE_INPUT" '.source.local_fips_patch.enabled')"
  reference_fips_patch_head="$(metadata_value "$REFERENCE_INPUT" '.source.local_fips_patch.git_head')"
  reference_fips_patch_dirty="$(metadata_value "$REFERENCE_INPUT" '.source.local_fips_patch.dirty')"
  pipeline_trace_mismatch="false"
  if [[ -n "$nvpn_pipeline_trace_enabled" \
      && -n "$reference_pipeline_trace_enabled" \
      && "$nvpn_pipeline_trace_enabled" != "$reference_pipeline_trace_enabled" ]]; then
    pipeline_trace_mismatch="true"
  fi
  local effective_udp_loss_delta_pct
  effective_udp_loss_delta_pct="$MAX_LOSS_DELTA_PCT"
  if is_true_value "$nvpn_stress_enabled" && is_true_value "$reference_stress_enabled"; then
    effective_udp_loss_delta_pct="$MAX_STRESS_UDP_LOSS_DELTA_PCT"
  fi
  local iperf_mismatch="false"
  if [[ "$nvpn_iperf_socket_buffer" != "$reference_iperf_socket_buffer" \
      || "$nvpn_iperf_udp1000_parallel" != "$reference_iperf_udp1000_parallel" \
      || "$nvpn_iperf_udp1000_bandwidth" != "$reference_iperf_udp1000_bandwidth" \
      || "$nvpn_iperf_udp1000_per_stream_bandwidth" != "$reference_iperf_udp1000_per_stream_bandwidth" ]]; then
    iperf_mismatch="true"
  fi

  local comparison_tsv ratios_tsv comparison_json
  local thresholds_tsv
  comparison_tsv="$OUTPUT_DIR/comparison.tsv"
  ratios_tsv="$OUTPUT_DIR/ratios.tsv"
  thresholds_tsv="$OUTPUT_DIR/thresholds.tsv"
  comparison_json="$OUTPUT_DIR/comparison.json"

  write_row \
    label source_summary backend threads duration_secs \
    cpu_stress_enabled cpu_stress_sides local_cpu_stress_workers remote_cpu_stress_workers \
    pipeline_trace_enabled pipeline_trace_interval_secs \
    forward_direction reverse_direction \
    tcp_single_mbps tcp_single_retrans tcp_4_mbps tcp_4_retrans \
    tcp_8_mbps tcp_8_retrans udp_200_mbps udp_200_loss_pct \
    udp_1000_mbps udp_1000_loss_pct \
    tcp_single_b_to_a_mbps tcp_single_b_to_a_retrans \
    tcp_4_b_to_a_mbps tcp_4_b_to_a_retrans \
    tcp_8_b_to_a_mbps tcp_8_b_to_a_retrans \
    udp_200_b_to_a_mbps udp_200_b_to_a_loss_pct \
    udp_1000_b_to_a_mbps udp_1000_b_to_a_loss_pct \
    ping_loss_pct ping_avg_ms \
    ping_mdev_ms ping_p95_ms ping_p99_ms ping_max_ms \
    tcp_single_cpu_sec_per_gb tcp_4_cpu_sec_per_gb tcp_8_cpu_sec_per_gb \
    udp_200_cpu_sec_per_gb udp_1000_cpu_sec_per_gb raw_dir \
    >"$comparison_tsv"
  write_normalized_summary_row "$NVPN_LABEL" "$nvpn_summary" "$NVPN_BACKEND" "$NVPN_THREADS" "$NVPN_INPUT" >>"$comparison_tsv"
  write_normalized_summary_row "$REFERENCE_LABEL" "$reference_summary" "$REFERENCE_BACKEND" "$REFERENCE_THREADS" "$REFERENCE_INPUT" >>"$comparison_tsv"

  write_row metric unit better_when nvpn reference nvpn_percent_of_reference nvpn_minus_reference >"$ratios_tsv"
  write_row check metric status nvpn reference threshold comparison >"$thresholds_tsv"
  local direction_suffix metric
  for direction_suffix in "" _b_to_a; do
    for metric in tcp_single tcp_4 tcp_8 udp_200 udp_1000; do
      write_metric_ratio "${metric}${direction_suffix}_mbps" Mbps higher "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
      write_threshold_higher_pct "${metric}${direction_suffix}_throughput" "${metric}${direction_suffix}_mbps" "$MIN_THROUGHPUT_PCT" "$nvpn_summary" "$reference_summary" >>"$thresholds_tsv"
    done
    for metric in tcp_single tcp_4 tcp_8; do
      write_metric_ratio "${metric}${direction_suffix}_retrans" count lower "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
      write_threshold_lower_pct "${metric}${direction_suffix}_retrans" "${metric}${direction_suffix}_retrans" "$MAX_RETRANS_PCT" "$nvpn_summary" "$reference_summary" >>"$thresholds_tsv"
    done
    for metric in udp_200 udp_1000; do
      write_metric_ratio "${metric}${direction_suffix}_loss_pct" pct lower "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
      write_threshold_lower_delta "${metric}${direction_suffix}_loss" "${metric}${direction_suffix}_loss_pct" "$effective_udp_loss_delta_pct" pp "$nvpn_summary" "$reference_summary" >>"$thresholds_tsv"
      [[ "$REQUIRE_NVPN_UDP_ZERO_LOSS" != "1" ]] \
        || write_threshold_zero "nvpn_${metric}${direction_suffix}_zero_loss" "${metric}${direction_suffix}_loss_pct" "$nvpn_summary" >>"$thresholds_tsv"
    done
  done
  write_metric_ratio ping_loss_pct pct lower "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_metric_ratio ping_avg_ms ms lower "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_optional_metric_ratio ping_mdev_ms ms lower "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_optional_metric_ratio ping_p95_ms ms lower "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_optional_metric_ratio ping_p99_ms ms lower "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_optional_metric_ratio ping_max_ms ms lower "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_cpu_metric_ratio tcp_single_cpu_sec_per_gb tcp-single "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_cpu_metric_ratio tcp_4_cpu_sec_per_gb tcp-4 "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_cpu_metric_ratio tcp_8_cpu_sec_per_gb tcp-8 "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_cpu_metric_ratio udp_200_cpu_sec_per_gb udp-200 "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"
  write_cpu_metric_ratio udp_1000_cpu_sec_per_gb udp-1000 "$nvpn_summary" "$reference_summary" >>"$ratios_tsv"

  write_threshold_lower_delta ping_loss ping_loss_pct "$MAX_LOSS_DELTA_PCT" pp "$nvpn_summary" "$reference_summary" >>"$thresholds_tsv"
  write_threshold_lower_delta ping_avg ping_avg_ms "$MAX_PING_AVG_DELTA_MS" ms "$nvpn_summary" "$reference_summary" >>"$thresholds_tsv"
  write_optional_threshold_lower_delta ping_p95 ping_p95_ms "$MAX_PING_P95_DELTA_MS" ms "$nvpn_summary" "$reference_summary" >>"$thresholds_tsv"
  write_optional_threshold_lower_delta ping_p99 ping_p99_ms "$MAX_PING_P99_DELTA_MS" ms "$nvpn_summary" "$reference_summary" >>"$thresholds_tsv"
  write_optional_threshold_lower_delta ping_max ping_max_ms "$MAX_PING_MAX_DELTA_MS" ms "$nvpn_summary" "$reference_summary" >>"$thresholds_tsv"
  if [[ "$REQUIRE_NVPN_PING_ZERO_LOSS" == "1" ]]; then
    write_threshold_zero nvpn_ping_zero_loss ping_loss_pct "$nvpn_summary" >>"$thresholds_tsv"
  fi

  local comparison_rows ratios thresholds threshold_failures threshold_unknowns threshold_status
  comparison_rows="$(tsv_to_json "$comparison_tsv")"
  ratios="$(tsv_to_json "$ratios_tsv")"
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
  jq -n \
    --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg nvpn_summary "$nvpn_summary" \
    --arg reference_summary "$reference_summary" \
    --arg nvpn_backend "$NVPN_BACKEND" \
    --arg reference_backend "$REFERENCE_BACKEND" \
    --arg nvpn_label "$NVPN_LABEL" \
    --arg reference_label "$REFERENCE_LABEL" \
    --arg nvpn_threads "$NVPN_THREADS" \
    --arg reference_threads "$REFERENCE_THREADS" \
    --arg nvpn_stress_enabled "$nvpn_stress_enabled" \
    --arg nvpn_stress_sides "$nvpn_stress_sides" \
    --arg nvpn_stress_local_workers "$nvpn_stress_local_workers" \
    --arg nvpn_stress_remote_workers "$nvpn_stress_remote_workers" \
    --arg reference_stress_enabled "$reference_stress_enabled" \
    --arg reference_stress_sides "$reference_stress_sides" \
    --arg reference_stress_local_workers "$reference_stress_local_workers" \
    --arg reference_stress_remote_workers "$reference_stress_remote_workers" \
    --arg nvpn_pipeline_trace_enabled "$nvpn_pipeline_trace_enabled" \
    --arg nvpn_pipeline_trace_interval_secs "$nvpn_pipeline_trace_interval_secs" \
    --arg reference_pipeline_trace_enabled "$reference_pipeline_trace_enabled" \
    --arg reference_pipeline_trace_interval_secs "$reference_pipeline_trace_interval_secs" \
    --arg pipeline_trace_mismatch "$pipeline_trace_mismatch" \
    --arg nvpn_iperf_socket_buffer "$nvpn_iperf_socket_buffer" \
    --arg nvpn_iperf_udp1000_parallel "$nvpn_iperf_udp1000_parallel" \
    --arg nvpn_iperf_udp1000_bandwidth "$nvpn_iperf_udp1000_bandwidth" \
    --arg nvpn_iperf_udp1000_per_stream_bandwidth "$nvpn_iperf_udp1000_per_stream_bandwidth" \
    --arg reference_iperf_socket_buffer "$reference_iperf_socket_buffer" \
    --arg reference_iperf_udp1000_parallel "$reference_iperf_udp1000_parallel" \
    --arg reference_iperf_udp1000_bandwidth "$reference_iperf_udp1000_bandwidth" \
    --arg reference_iperf_udp1000_per_stream_bandwidth "$reference_iperf_udp1000_per_stream_bandwidth" \
    --arg iperf_mismatch "$iperf_mismatch" \
    --arg nvpn_extra_connect_env "$nvpn_extra_connect_env" \
    --arg reference_extra_connect_env "$reference_extra_connect_env" \
    --arg nvpn_source_head "$nvpn_source_head" \
    --arg nvpn_source_dirty "$nvpn_source_dirty" \
    --arg nvpn_fips_patch_enabled "$nvpn_fips_patch_enabled" \
    --arg nvpn_fips_patch_head "$nvpn_fips_patch_head" \
    --arg nvpn_fips_patch_dirty "$nvpn_fips_patch_dirty" \
    --arg reference_source_head "$reference_source_head" \
    --arg reference_source_dirty "$reference_source_dirty" \
    --arg reference_fips_patch_enabled "$reference_fips_patch_enabled" \
    --arg reference_fips_patch_head "$reference_fips_patch_head" \
    --arg reference_fips_patch_dirty "$reference_fips_patch_dirty" \
    --arg min_throughput_pct "$MIN_THROUGHPUT_PCT" \
    --arg max_retrans_pct "$MAX_RETRANS_PCT" \
    --arg max_loss_delta_pct "$MAX_LOSS_DELTA_PCT" \
    --arg max_stress_udp_loss_delta_pct "$MAX_STRESS_UDP_LOSS_DELTA_PCT" \
    --arg effective_udp_loss_delta_pct "$effective_udp_loss_delta_pct" \
    --arg max_ping_avg_delta_ms "$MAX_PING_AVG_DELTA_MS" \
    --arg max_ping_p95_delta_ms "$MAX_PING_P95_DELTA_MS" \
    --arg max_ping_p99_delta_ms "$MAX_PING_P99_DELTA_MS" \
    --arg max_ping_max_delta_ms "$MAX_PING_MAX_DELTA_MS" \
    --arg require_nvpn_udp_zero_loss "$REQUIRE_NVPN_UDP_ZERO_LOSS" \
    --arg require_nvpn_ping_zero_loss "$REQUIRE_NVPN_PING_ZERO_LOSS" \
    --arg threshold_status "$threshold_status" \
    --arg threshold_failures "$threshold_failures" \
    --arg threshold_unknowns "$threshold_unknowns" \
    --argjson comparison "$comparison_rows" \
    --argjson ratios "$ratios" \
    --argjson thresholds "$thresholds" \
    'def num($v): if $v == "" or $v == "null" then null else ($v | tonumber) end;
     def bool($v):
       if $v == "" or $v == "null" then null
       elif $v == "1" or $v == "true" then true
       elif $v == "0" or $v == "false" then false
       else null
       end;
     {
      generated_at: $generated_at,
      inputs: {
        nvpn_summary: $nvpn_summary,
        reference_summary: $reference_summary,
        nvpn_backend: $nvpn_backend,
        reference_backend: $reference_backend,
        nvpn_label: $nvpn_label,
        reference_label: $reference_label,
        nvpn_threads: $nvpn_threads,
        reference_threads: $reference_threads
      },
      cpu_stress: {
        nvpn: {
          enabled: bool($nvpn_stress_enabled),
          sides: $nvpn_stress_sides,
          local_workers: num($nvpn_stress_local_workers),
          remote_workers: num($nvpn_stress_remote_workers)
        },
        reference: {
          enabled: bool($reference_stress_enabled),
          sides: $reference_stress_sides,
          local_workers: num($reference_stress_local_workers),
          remote_workers: num($reference_stress_remote_workers)
        }
      },
      pipeline_trace: {
        mismatch: bool($pipeline_trace_mismatch),
        nvpn: {
          enabled: bool($nvpn_pipeline_trace_enabled),
          interval_secs: num($nvpn_pipeline_trace_interval_secs)
        },
        reference: {
          enabled: bool($reference_pipeline_trace_enabled),
          interval_secs: num($reference_pipeline_trace_interval_secs)
        }
      },
      iperf: {
        mismatch: bool($iperf_mismatch),
        nvpn: {
          socket_buffer: (if $nvpn_iperf_socket_buffer == "" then null else $nvpn_iperf_socket_buffer end),
          udp1000_parallel: num($nvpn_iperf_udp1000_parallel),
          udp1000_bandwidth: (if $nvpn_iperf_udp1000_bandwidth == "" then null else $nvpn_iperf_udp1000_bandwidth end),
          udp1000_per_stream_bandwidth: (if $nvpn_iperf_udp1000_per_stream_bandwidth == "" then null else $nvpn_iperf_udp1000_per_stream_bandwidth end)
        },
        reference: {
          socket_buffer: (if $reference_iperf_socket_buffer == "" then null else $reference_iperf_socket_buffer end),
          udp1000_parallel: num($reference_iperf_udp1000_parallel),
          udp1000_bandwidth: (if $reference_iperf_udp1000_bandwidth == "" then null else $reference_iperf_udp1000_bandwidth end),
          udp1000_per_stream_bandwidth: (if $reference_iperf_udp1000_per_stream_bandwidth == "" then null else $reference_iperf_udp1000_per_stream_bandwidth end)
        }
      },
      provenance: {
        nvpn: {
          run_env: {
            extra_connect_env: $nvpn_extra_connect_env
          },
          source: {
            git_head: (if $nvpn_source_head == "" then null else $nvpn_source_head end),
            dirty: bool($nvpn_source_dirty)
          },
          local_fips_patch: {
            enabled: bool($nvpn_fips_patch_enabled),
            git_head: (if $nvpn_fips_patch_head == "" then null else $nvpn_fips_patch_head end),
            dirty: bool($nvpn_fips_patch_dirty)
          }
        },
        reference: {
          run_env: {
            extra_connect_env: $reference_extra_connect_env
          },
          source: {
            git_head: (if $reference_source_head == "" then null else $reference_source_head end),
            dirty: bool($reference_source_dirty)
          },
          local_fips_patch: {
            enabled: bool($reference_fips_patch_enabled),
            git_head: (if $reference_fips_patch_head == "" then null else $reference_fips_patch_head end),
            dirty: bool($reference_fips_patch_dirty)
          }
        }
      },
      threshold_policy: {
        min_throughput_pct: num($min_throughput_pct),
        max_retrans_pct: num($max_retrans_pct),
        max_loss_delta_pct: num($max_loss_delta_pct),
        max_stress_udp_loss_delta_pct: num($max_stress_udp_loss_delta_pct),
        effective_udp_loss_delta_pct: num($effective_udp_loss_delta_pct),
        max_ping_avg_delta_ms: num($max_ping_avg_delta_ms),
        max_ping_p95_delta_ms: num($max_ping_p95_delta_ms),
        max_ping_p99_delta_ms: num($max_ping_p99_delta_ms),
        max_ping_max_delta_ms: num($max_ping_max_delta_ms),
        require_nvpn_udp_zero_loss: bool($require_nvpn_udp_zero_loss),
        require_nvpn_ping_zero_loss: bool($require_nvpn_ping_zero_loss)
      },
      threshold_status: {
        status: $threshold_status,
        failures: num($threshold_failures),
        unknowns: num($threshold_unknowns)
      },
      comparison: $comparison,
      ratios: $ratios,
      thresholds: $thresholds
    }' >"$comparison_json"

  if [[ "$ENFORCE_THRESHOLDS" == "1" && "$threshold_status" != "pass" ]]; then
    die "threshold status is $threshold_status; see $thresholds_tsv"
  fi

  printf 'docker benchmark comparison wrote %s, %s, %s, and %s\n' \
    "$comparison_tsv" "$ratios_tsv" "$thresholds_tsv" "$comparison_json"
}

main "$@"
