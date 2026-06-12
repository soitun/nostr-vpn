#!/usr/bin/env bash
# Run one nvpn/FIPS host-pair row, one or more userspace WireGuard reference
# rows, then normalize their artifacts into comparison bundles.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

REMOTE_SSH="${NVPN_HOST_PAIR_COMPARISON_SSH:-${1:-}}"
LOCAL_UNDERLAY_IP="${NVPN_HOST_PAIR_COMPARISON_LOCAL_UNDERLAY_IP:-}"
REMOTE_UNDERLAY_IP="${NVPN_HOST_PAIR_COMPARISON_REMOTE_UNDERLAY_IP:-}"
REMOTE_SSH_PORT="${NVPN_HOST_PAIR_COMPARISON_SSH_PORT:-}"
REMOTE_SSH_CONNECT_TIMEOUT="${NVPN_HOST_PAIR_COMPARISON_SSH_CONNECT_TIMEOUT:-10}"
LOCAL_PEER="${NVPN_HOST_PAIR_COMPARISON_LOCAL_PEER:-}"
REMOTE_PEER="${NVPN_HOST_PAIR_COMPARISON_REMOTE_PEER:-}"
LOCAL_NVPN="${NVPN_HOST_PAIR_COMPARISON_LOCAL_NVPN:-}"
REMOTE_NVPN="${NVPN_HOST_PAIR_COMPARISON_REMOTE_NVPN:-}"
LOCAL_NVPN_COMMAND="${NVPN_HOST_PAIR_COMPARISON_LOCAL_NVPN_COMMAND:-}"
REMOTE_NVPN_COMMAND="${NVPN_HOST_PAIR_COMPARISON_REMOTE_NVPN_COMMAND:-}"
LOCAL_CONFIG="${NVPN_HOST_PAIR_COMPARISON_LOCAL_CONFIG:-}"
REMOTE_CONFIG="${NVPN_HOST_PAIR_COMPARISON_REMOTE_CONFIG:-}"
BACKEND="${NVPN_HOST_PAIR_COMPARISON_BACKEND:-boringtun}"
BACKENDS="${NVPN_HOST_PAIR_COMPARISON_BACKENDS:-$BACKEND}"
CPU_STRESS_MODES="${NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_MODES:-}"
RUN_ID="${NVPN_HOST_PAIR_COMPARISON_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
OUTPUT_DIR="${NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR:-$ROOT_DIR/artifacts/host-pair-comparison-runs/$RUN_ID}"
NVPN_OUTPUT_DIR="${NVPN_HOST_PAIR_COMPARISON_NVPN_OUTPUT_DIR:-$OUTPUT_DIR/nvpn}"
REFERENCE_OUTPUT_DIR="${NVPN_HOST_PAIR_COMPARISON_REFERENCE_OUTPUT_DIR:-$OUTPUT_DIR/reference}"
COMPARISON_OUTPUT_DIR="${NVPN_HOST_PAIR_COMPARISON_OUTPUT_DIR:-$OUTPUT_DIR/comparison}"
DRY_RUN="${NVPN_HOST_PAIR_COMPARISON_DRY_RUN:-0}"
PREFLIGHT_ONLY="${NVPN_HOST_PAIR_COMPARISON_PREFLIGHT_ONLY:-0}"
RUN_WG_PREFLIGHT="${NVPN_HOST_PAIR_COMPARISON_WG_PREFLIGHT:-1}"
RUN_NVPN_PREFLIGHT="${NVPN_HOST_PAIR_COMPARISON_NVPN_PREFLIGHT:-1}"
CPU_STRESS="${NVPN_HOST_PAIR_COMPARISON_CPU_STRESS:-0}"
CPU_STRESS_SIDES="${NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_SIDES:-remote}"
CPU_STRESS_WORKERS="${NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_WORKERS:-auto}"
CPU_STRESS_SETTLE_SECS="${NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_SETTLE_SECS:-2}"
NVPN_DURATION_SECS="${NVPN_HOST_PAIR_COMPARISON_DURATION_SECS:-}"
NVPN_INTERVAL_SECS="${NVPN_HOST_PAIR_COMPARISON_INTERVAL_SECS:-}"
PING_COUNT="${NVPN_HOST_PAIR_COMPARISON_PING_COUNT:-}"
PING_INTERVAL="${NVPN_HOST_PAIR_COMPARISON_PING_INTERVAL:-}"
IPERF_DURATION_SECS="${NVPN_HOST_PAIR_COMPARISON_IPERF_DURATION_SECS:-}"

die() {
  printf 'host-pair comparison run failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
usage: NVPN_HOST_PAIR_COMPARISON_SSH=user@host \
       NVPN_HOST_PAIR_COMPARISON_LOCAL_UNDERLAY_IP=<local-ip> \
       NVPN_HOST_PAIR_COMPARISON_REMOTE_UNDERLAY_IP=<remote-ip> \
       scripts/run-host-pair-comparison.sh

This runner maps one shared host-pair comparison config into:
  1. scripts/soak-fips-dataplane-host-pair.sh
  2. one or more scripts/bench-userspace-wg-host-pair.sh reference rows
  3. one scripts/compare-host-pair-benchmarks.sh result per reference row

It assumes the nvpn/FIPS daemons are already configured and running for the
nvpn row. Each userspace WireGuard reference row creates its own temporary
tunnel and still needs that harness's sudo/TUN prerequisites.

Common optional env:
  NVPN_HOST_PAIR_COMPARISON_BACKEND             boringtun or wireguard-go
  NVPN_HOST_PAIR_COMPARISON_BACKENDS            comma/space list, e.g. boringtun,wireguard-go
  NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR      output bundle root
  NVPN_HOST_PAIR_COMPARISON_DRY_RUN             print commands without running
  NVPN_HOST_PAIR_COMPARISON_PREFLIGHT_ONLY      run preflights/artifacts only
  NVPN_HOST_PAIR_COMPARISON_WG_PREFLIGHT        run WG preflight first (default 1)
  NVPN_HOST_PAIR_COMPARISON_NVPN_PREFLIGHT      run nvpn/FIPS preflight first (default 1)
  NVPN_HOST_PAIR_COMPARISON_LOCAL_PEER          remote peer selector for local nvpn status
  NVPN_HOST_PAIR_COMPARISON_REMOTE_PEER         local peer selector for remote nvpn status
  NVPN_HOST_PAIR_COMPARISON_LOCAL_NVPN          local nvpn binary for nvpn/FIPS row
  NVPN_HOST_PAIR_COMPARISON_REMOTE_NVPN         remote nvpn binary for nvpn/FIPS row
  NVPN_HOST_PAIR_COMPARISON_LOCAL_NVPN_COMMAND  local command before "status ..."
  NVPN_HOST_PAIR_COMPARISON_REMOTE_NVPN_COMMAND remote command before "status ..."
  NVPN_HOST_PAIR_COMPARISON_LOCAL_CONFIG        local nvpn config for nvpn/FIPS row
  NVPN_HOST_PAIR_COMPARISON_REMOTE_CONFIG       remote nvpn config for nvpn/FIPS row
  NVPN_HOST_PAIR_COMPARISON_CPU_STRESS          set 1 for CPU stress in both rows
  NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_MODES    comma/space list, e.g. clean,stress
  NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_SIDES    remote, local, or both
  NVPN_HOST_PAIR_COMPARISON_CPU_STRESS_WORKERS  auto caps at 4, or explicit count
  NVPN_HOST_PAIR_COMPARISON_DURATION_SECS       nvpn soak duration
  NVPN_HOST_PAIR_COMPARISON_INTERVAL_SECS       nvpn soak sample interval
  NVPN_HOST_PAIR_COMPARISON_PING_COUNT          forwarded to both harnesses
  NVPN_HOST_PAIR_COMPARISON_PING_INTERVAL       forwarded to both harnesses
  NVPN_HOST_PAIR_COMPARISON_IPERF_DURATION_SECS forwarded to both harnesses
EOF
}

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

tsv_escape() {
  local value="$1"
  value="${value//$'\t'/ }"
  value="${value//$'\n'/ }"
  printf '%s' "$value"
}

record_preflight_source() {
  local scope="$1"
  local mode="$2"
  local backend="$3"
  local source_path="$4"
  preflight_sources+=("$scope"$'\037'"$mode"$'\037'"$backend"$'\037'"$source_path")
}

write_preflight_summary() {
  local summary_path="$OUTPUT_DIR/preflight-summary.tsv"
  local status_path="$OUTPUT_DIR/preflight-status.tsv"
  local blockers_path="$OUTPUT_DIR/preflight-blockers.tsv"
  local source_row scope mode backend source_path status check detail extra

  mkdir -p "$OUTPUT_DIR"
  printf 'scope\tmode\tbackend\tstatus\tcheck\tdetail\tsource\n' >"$summary_path"
  for source_row in "${preflight_sources[@]}"; do
    IFS=$'\037' read -r scope mode backend source_path <<<"$source_row"
    if [[ ! -f "$source_path" ]]; then
      printf '%s\t%s\t%s\tmissing\t%s\t%s\t%s\n' \
        "$(tsv_escape "$scope")" \
        "$(tsv_escape "$mode")" \
        "$(tsv_escape "$backend")" \
        "preflight artifact is present" \
        "" \
        "$(tsv_escape "$source_path")" >>"$summary_path"
      continue
    fi

    while IFS=$'\t' read -r status check detail extra; do
      [[ "$status" == "status" ]] && continue
      [[ -n "$status" ]] || continue
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$(tsv_escape "$scope")" \
        "$(tsv_escape "$mode")" \
        "$(tsv_escape "$backend")" \
        "$(tsv_escape "$status")" \
        "$(tsv_escape "$check")" \
        "$(tsv_escape "${detail:-}")" \
        "$(tsv_escape "$source_path")" >>"$summary_path"
    done <"$source_path"
  done
  printf 'host-pair comparison preflight summary: %s\n' "$summary_path"

  awk -v status_path="$status_path" -F '\t' '
    BEGIN {
      print "status\tcount" > status_path
      ordered[1] = "ok"
      ordered[2] = "missing"
    }
    NR > 1 && $4 != "" {
      counts[$4]++
    }
    END {
      for (i = 1; i <= 2; i++) {
        status = ordered[i]
        if (status in counts) {
          print status "\t" counts[status] > status_path
          emitted[status] = 1
        }
      }
      for (status in counts) {
        if (!(status in emitted)) {
          print status "\t" counts[status] > status_path
        }
      }
    }
  ' "$summary_path"

  awk -v blockers_path="$blockers_path" -F '\t' '
    BEGIN {
      print "scope\tbackend\tcheck\tdetails\toccurrences\tmodes" > blockers_path
    }
    NR > 1 && $4 != "ok" {
      key = $1 SUBSEP $3 SUBSEP $5
      if (!(key in seen)) {
        seen[key] = 1
        order[++order_len] = key
        scope[key] = $1
        backend[key] = $3
        check[key] = $5
      }
      occurrences[key]++
      mode = $2 == "" ? "-" : $2
      mode_key = key SUBSEP mode
      if (!(mode_key in mode_seen)) {
        mode_seen[mode_key] = 1
        modes[key] = modes[key] == "" ? mode : modes[key] "," mode
      }
      detail = $6 == "" ? "-" : $6
      detail_key = key SUBSEP detail
      if (!(detail_key in detail_seen)) {
        detail_seen[detail_key] = 1
        details[key] = details[key] == "" ? detail : details[key] "; " detail
      }
    }
    END {
      for (i = 1; i <= order_len; i++) {
        key = order[i]
        print scope[key] "\t" backend[key] "\t" check[key] "\t" details[key] "\t" occurrences[key] "\t" modes[key] > blockers_path
      }
    }
  ' "$summary_path"
  printf 'host-pair comparison preflight status: %s\n' "$status_path"
  printf 'host-pair comparison preflight blockers: %s\n' "$blockers_path"
}

add_nvpn_env() {
  local name="$1"
  local value="$2"
  [[ -n "$value" ]] || return 0
  nvpn_env+=("$name=$value")
}

add_wg_env() {
  local name="$1"
  local value="$2"
  [[ -n "$value" ]] || return 0
  wg_env+=("$name=$value")
}

build_nvpn_env() {
  local output_dir="$1"
  local stress_enabled="$2"
  nvpn_env=(
    "NVPN_HOST_PAIR_SSH=$REMOTE_SSH"
    "NVPN_HOST_PAIR_OUTPUT_DIR=$output_dir"
    "NVPN_HOST_PAIR_EXPECTED_LOCAL_UNDERLAY_IP=$LOCAL_UNDERLAY_IP"
    "NVPN_HOST_PAIR_EXPECTED_REMOTE_UNDERLAY_IP=$REMOTE_UNDERLAY_IP"
    "NVPN_HOST_PAIR_CPU_STRESS=$stress_enabled"
    "NVPN_HOST_PAIR_CPU_STRESS_SIDES=$CPU_STRESS_SIDES"
    "NVPN_HOST_PAIR_CPU_STRESS_WORKERS=$CPU_STRESS_WORKERS"
    "NVPN_HOST_PAIR_CPU_STRESS_SETTLE_SECS=$CPU_STRESS_SETTLE_SECS"
  )
  add_nvpn_env NVPN_HOST_PAIR_SSH_PORT "$REMOTE_SSH_PORT"
  add_nvpn_env NVPN_HOST_PAIR_SSH_CONNECT_TIMEOUT "$REMOTE_SSH_CONNECT_TIMEOUT"
  add_nvpn_env NVPN_HOST_PAIR_LOCAL_PEER "$LOCAL_PEER"
  add_nvpn_env NVPN_HOST_PAIR_REMOTE_PEER "$REMOTE_PEER"
  add_nvpn_env NVPN_HOST_PAIR_LOCAL_NVPN "$LOCAL_NVPN"
  add_nvpn_env NVPN_HOST_PAIR_REMOTE_NVPN "$REMOTE_NVPN"
  add_nvpn_env NVPN_HOST_PAIR_LOCAL_NVPN_COMMAND "$LOCAL_NVPN_COMMAND"
  add_nvpn_env NVPN_HOST_PAIR_REMOTE_NVPN_COMMAND "$REMOTE_NVPN_COMMAND"
  add_nvpn_env NVPN_HOST_PAIR_LOCAL_CONFIG "$LOCAL_CONFIG"
  add_nvpn_env NVPN_HOST_PAIR_REMOTE_CONFIG "$REMOTE_CONFIG"
  add_nvpn_env NVPN_HOST_PAIR_DURATION_SECS "$NVPN_DURATION_SECS"
  add_nvpn_env NVPN_HOST_PAIR_INTERVAL_SECS "$NVPN_INTERVAL_SECS"
  add_nvpn_env NVPN_HOST_PAIR_PING_COUNT "$PING_COUNT"
  add_nvpn_env NVPN_HOST_PAIR_PING_INTERVAL "$PING_INTERVAL"
  add_nvpn_env NVPN_HOST_PAIR_IPERF_DURATION_SECS "$IPERF_DURATION_SECS"
}

build_wg_env() {
  local backend="$1"
  local output_dir="$2"
  local stress_enabled="$3"
  wg_env=(
    "NVPN_WG_HOST_PAIR_SSH=$REMOTE_SSH"
    "NVPN_WG_HOST_PAIR_OUTPUT_DIR=$output_dir"
    "NVPN_WG_HOST_PAIR_LOCAL_UNDERLAY_IP=$LOCAL_UNDERLAY_IP"
    "NVPN_WG_HOST_PAIR_REMOTE_UNDERLAY_IP=$REMOTE_UNDERLAY_IP"
    "NVPN_WG_HOST_PAIR_BACKEND=$backend"
    "NVPN_WG_HOST_PAIR_CPU_STRESS=$stress_enabled"
    "NVPN_WG_HOST_PAIR_CPU_STRESS_SIDES=$CPU_STRESS_SIDES"
    "NVPN_WG_HOST_PAIR_CPU_STRESS_WORKERS=$CPU_STRESS_WORKERS"
    "NVPN_WG_HOST_PAIR_CPU_STRESS_SETTLE_SECS=$CPU_STRESS_SETTLE_SECS"
  )
  add_wg_env NVPN_WG_HOST_PAIR_SSH_PORT "$REMOTE_SSH_PORT"
  add_wg_env NVPN_WG_HOST_PAIR_SSH_CONNECT_TIMEOUT "$REMOTE_SSH_CONNECT_TIMEOUT"
  add_wg_env NVPN_WG_HOST_PAIR_PING_COUNT "$PING_COUNT"
  add_wg_env NVPN_WG_HOST_PAIR_PING_INTERVAL "$PING_INTERVAL"
  add_wg_env NVPN_WG_HOST_PAIR_IPERF_DURATION_SECS "$IPERF_DURATION_SECS"
}

print_command() {
  local first=1 arg
  for arg in "$@"; do
    if (( first )); then
      first=0
    else
      printf ' '
    fi
    printf '%q' "$arg"
  done
  printf '\n'
}

run_step() {
  local label="$1"
  shift
  printf '== %s ==\n' "$label"
  if is_true "$DRY_RUN"; then
    print_command "$@"
  else
    "$@"
  fi
}

run_preflight_step() {
  local label="$1"
  shift
  if is_true "$PREFLIGHT_ONLY"; then
    if run_step "$label" "$@"; then
      return 0
    fi
    return 1
  fi
  run_step "$label" "$@"
}

parse_backends() {
  local raw="${1//,/ }"
  local token
  local -a parsed
  read -r -a parsed <<<"$raw"
  backends=()
  for token in "${parsed[@]}"; do
    [[ -z "$token" ]] && continue
    case "$token" in
      boringtun|wireguard-go) backends+=("$token") ;;
      *) die "unsupported comparison backend: $token; expected boringtun or wireguard-go" ;;
    esac
  done
  ((${#backends[@]} > 0)) || die "set at least one comparison backend"
}

parse_stress_modes() {
  local raw="$1"
  local token
  local -a parsed
  modes=()
  if [[ -z "$raw" ]]; then
    if is_true "$CPU_STRESS"; then
      modes=(stress)
    else
      modes=(clean)
    fi
    return 0
  fi
  raw="${raw//,/ }"
  read -r -a parsed <<<"$raw"
  for token in "${parsed[@]}"; do
    [[ -z "$token" ]] && continue
    case "$token" in
      clean|off|none|0) modes+=(clean) ;;
      stress|on|1) modes+=(stress) ;;
      *) die "unsupported CPU stress mode: $token; expected clean or stress" ;;
    esac
  done
  ((${#modes[@]} > 0)) || die "set at least one CPU stress mode"
}

stress_enabled_for_mode() {
  case "$1" in
    clean) printf '0\n' ;;
    stress) printf '1\n' ;;
    *) die "unsupported CPU stress mode: $1" ;;
  esac
}

step_label() {
  local base="$1"
  local mode="$2"
  local backend="$3"
  local multi_mode="$4"
  local multi_backend="$5"
  local suffix=""
  if (( multi_mode && multi_backend )); then
    suffix=" ($mode/$backend)"
  elif (( multi_mode )); then
    suffix=" ($mode)"
  elif (( multi_backend )); then
    suffix=" ($backend)"
  fi
  printf '%s%s\n' "$base" "$suffix"
}

mode_nvpn_dir() {
  local mode="$1"
  local multi_mode="$2"
  if (( multi_mode )); then
    printf '%s/%s/nvpn\n' "$OUTPUT_DIR" "$mode"
  else
    printf '%s\n' "$NVPN_OUTPUT_DIR"
  fi
}

backend_reference_dir() {
  local mode="$1"
  local backend="$2"
  local multi_mode="$3"
  local multi_backend="$4"
  if (( multi_mode && multi_backend )); then
    printf '%s/%s/reference-%s\n' "$OUTPUT_DIR" "$mode" "$backend"
  elif (( multi_mode )); then
    printf '%s/%s/reference\n' "$OUTPUT_DIR" "$mode"
  elif (( multi_backend )); then
    printf '%s/reference-%s\n' "$OUTPUT_DIR" "$backend"
  else
    printf '%s\n' "$REFERENCE_OUTPUT_DIR"
  fi
}

backend_comparison_dir() {
  local mode="$1"
  local backend="$2"
  local multi_mode="$3"
  local multi_backend="$4"
  if (( multi_mode && multi_backend )); then
    printf '%s/%s/comparison-%s\n' "$OUTPUT_DIR" "$mode" "$backend"
  elif (( multi_mode )); then
    printf '%s/%s/comparison\n' "$OUTPUT_DIR" "$mode"
  elif (( multi_backend )); then
    printf '%s/comparison-%s\n' "$OUTPUT_DIR" "$backend"
  else
    printf '%s\n' "$COMPARISON_OUTPUT_DIR"
  fi
}

main() {
  if [[ -z "$REMOTE_SSH" ]]; then
    usage
    die "set SSH target"
  fi
  if ! is_true "$PREFLIGHT_ONLY" && [[ -z "$LOCAL_UNDERLAY_IP" || -z "$REMOTE_UNDERLAY_IP" ]]; then
    usage
    die "set local and remote underlay IPs"
  fi

  local -a nvpn_env wg_env compare_cmd preflight_env backends modes preflight_sources
  parse_backends "$BACKENDS"
  parse_stress_modes "$CPU_STRESS_MODES"
  local multi_backend=0
  if ((${#backends[@]} > 1)); then
    multi_backend=1
  fi
  local multi_mode=0
  if ((${#modes[@]} > 1)); then
    multi_mode=1
  fi
  if (( multi_backend || multi_mode )) && [[ -n "${NVPN_HOST_PAIR_COMPARISON_REFERENCE_OUTPUT_DIR:-}${NVPN_HOST_PAIR_COMPARISON_OUTPUT_DIR:-}" ]]; then
    die "per-reference output dir overrides are ambiguous with comparison matrices; set NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR instead"
  fi
  if (( multi_mode )) && [[ -n "${NVPN_HOST_PAIR_COMPARISON_NVPN_OUTPUT_DIR:-}" ]]; then
    die "per-mode nvpn output dirs are ambiguous with CPU stress sweeps; set NVPN_HOST_PAIR_COMPARISON_RUN_OUTPUT_DIR instead"
  fi

  if ! is_true "$DRY_RUN" && ! is_true "$PREFLIGHT_ONLY"; then
    mkdir -p "$OUTPUT_DIR"
    printf '%s\n' 'mode	backend	cpu_stress_enabled	nvpn_dir	reference_dir	comparison_dir' >"$OUTPUT_DIR/manifest.tsv"
  fi

  local mode stress_enabled nvpn_dir backend reference_dir comparison_dir label
  local preflight_result=0
  for mode in "${modes[@]}"; do
    stress_enabled="$(stress_enabled_for_mode "$mode")"
    nvpn_dir="$(mode_nvpn_dir "$mode" "$multi_mode")"

    build_nvpn_env "$nvpn_dir" "$stress_enabled"
    if is_true "$RUN_NVPN_PREFLIGHT"; then
      local -a nvpn_preflight_env
      nvpn_preflight_env=("${nvpn_env[@]}" "NVPN_HOST_PAIR_PREFLIGHT=1")
      label="$(step_label "nvpn/FIPS host-pair preflight" "$mode" "" "$multi_mode" 0)"
      if is_true "$PREFLIGHT_ONLY"; then
        record_preflight_source nvpn "$mode" "" "$nvpn_dir/preflight.tsv"
        if ! run_preflight_step "$label" \
          env "${nvpn_preflight_env[@]}" "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"; then
          preflight_result=1
        fi
      else
        run_preflight_step "$label" \
          env "${nvpn_preflight_env[@]}" "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"
      fi
    fi

    if is_true "$RUN_WG_PREFLIGHT"; then
      for backend in "${backends[@]}"; do
        reference_dir="$(backend_reference_dir "$mode" "$backend" "$multi_mode" "$multi_backend")"
        build_wg_env "$backend" "$reference_dir" "$stress_enabled"
        preflight_env=("${wg_env[@]}" "NVPN_WG_HOST_PAIR_PREFLIGHT=1")
        label="$(step_label "userspace WireGuard preflight" "$mode" "$backend" "$multi_mode" "$multi_backend")"
        if is_true "$PREFLIGHT_ONLY"; then
          record_preflight_source reference "$mode" "$backend" "$reference_dir/preflight.tsv"
          if ! run_preflight_step "$label" \
            env "${preflight_env[@]}" "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"; then
            preflight_result=1
          fi
        else
          run_preflight_step "$label" \
            env "${preflight_env[@]}" "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"
        fi
      done
    fi

    if is_true "$PREFLIGHT_ONLY"; then
      continue
    fi

    label="$(step_label "nvpn/FIPS host-pair row" "$mode" "" "$multi_mode" 0)"
    run_step "$label" \
      env "${nvpn_env[@]}" "$ROOT_DIR/scripts/soak-fips-dataplane-host-pair.sh"

    for backend in "${backends[@]}"; do
      reference_dir="$(backend_reference_dir "$mode" "$backend" "$multi_mode" "$multi_backend")"
      comparison_dir="$(backend_comparison_dir "$mode" "$backend" "$multi_mode" "$multi_backend")"
      build_wg_env "$backend" "$reference_dir" "$stress_enabled"

      label="$(step_label "userspace WireGuard reference row" "$mode" "$backend" "$multi_mode" "$multi_backend")"
      run_step "$label" \
        env "${wg_env[@]}" "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"

      compare_cmd=(
        "$ROOT_DIR/scripts/compare-host-pair-benchmarks.sh"
        "$nvpn_dir"
        "$reference_dir"
        "$comparison_dir"
      )
      label="$(step_label "normalize comparison artifacts" "$mode" "$backend" "$multi_mode" "$multi_backend")"
      run_step "$label" "${compare_cmd[@]}"
      if ! is_true "$DRY_RUN"; then
        printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$mode" "$backend" "$stress_enabled" "$nvpn_dir" "$reference_dir" "$comparison_dir" >>"$OUTPUT_DIR/manifest.tsv"
      fi
    done
  done

  if is_true "$PREFLIGHT_ONLY"; then
    if ! is_true "$DRY_RUN"; then
      write_preflight_summary
    fi
    printf 'host-pair comparison preflight bundle: %s\n' "$OUTPUT_DIR"
    return "$preflight_result"
  fi

  if ! is_true "$DRY_RUN"; then
    run_step "summarize comparison matrix" \
      "$ROOT_DIR/scripts/summarize-host-pair-comparison-run.sh" "$OUTPUT_DIR"
  fi

  printf 'host-pair comparison bundle: %s\n' "$OUTPUT_DIR"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
