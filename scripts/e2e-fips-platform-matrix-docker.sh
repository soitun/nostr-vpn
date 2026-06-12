#!/usr/bin/env bash
# Short Linux/docker platform-split matrix for FIPS dataplane changes.
#
# This wraps e2e-fips-perf-regression-docker.sh so connected-UDP, worker-count,
# and backpressure knobs are exercised through the same direct-path and
# TCP/ping no-wedge assertions. It is intentionally local/docker coverage only;
# real Mac-to-Mac Wi-Fi/screenshare soak remains operator-local.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="${NVPN_PLATFORM_MATRIX_OUTPUT_DIR:-$ROOT_DIR/artifacts/fips-platform-matrix/$(date -u +%Y%m%dT%H%M%SZ)}"
SCENARIO_CSV="${NVPN_PLATFORM_MATRIX_SCENARIOS:-connected-udp-on,connected-udp-off,single-encrypt-worker,tight-send-backpressure,tight-backpressure}"
PROJECT_PREFIX="${NVPN_PLATFORM_MATRIX_PROJECT_PREFIX:-nostr-vpn-e2e-fips-matrix}"
FAIL_FAST="${NVPN_PLATFORM_MATRIX_FAIL_FAST:-0}"
ATTEMPTS="${NVPN_PLATFORM_MATRIX_ATTEMPTS:-1}"
RUNNER="${NVPN_PLATFORM_MATRIX_RUNNER:-$ROOT_DIR/scripts/e2e-fips-perf-regression-docker.sh}"

DURATION_SECS="${NVPN_PLATFORM_MATRIX_DURATION_SECS:-4}"
LOAD_DURATION_SECS="${NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS:-6}"
# Keep the matrix aligned with the perf gate's default so the 2% loss ceiling
# is not decided by one packet in a short 20-ping window.
PING_COUNT="${NVPN_PLATFORM_MATRIX_PING_COUNT:-60}"
PING_INTERVAL="${NVPN_PLATFORM_MATRIX_PING_INTERVAL:-0.1}"
WORKER_QUEUE_PRESSURE_CAP="${NVPN_PLATFORM_MATRIX_WORKER_QUEUE_PRESSURE_CAP:-4}"
RX_MAINT_FAULT_MS="${NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS:-150}"
PHASES="${NVPN_PLATFORM_MATRIX_PHASES:-}"
EXTRA_ENV="${NVPN_PLATFORM_MATRIX_EXTRA_ENV:-}"

if [[ ! "$ATTEMPTS" =~ ^[1-9][0-9]*$ ]]; then
  echo "NVPN_PLATFORM_MATRIX_ATTEMPTS must be a positive integer, got: $ATTEMPTS" >&2
  exit 2
fi

if [[ ! -x "$RUNNER" ]]; then
  echo "NVPN_PLATFORM_MATRIX_RUNNER is not executable: $RUNNER" >&2
  exit 2
fi

mkdir -p "$OUT_DIR"
SUMMARY="$OUT_DIR/summary.tsv"
printf 'scenario\tstatus\telapsed_secs\tlog\tsha256\tphase_summary\textra_env\tattempt\tattempts\tfailure_summary\n' >"$SUMMARY"

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

safe_name() {
  printf '%s' "$1" | tr -c 'A-Za-z0-9_.-' '-'
}

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  else
    shasum -a 256 "$path" | awk '{print $1}'
  fi
}

scenario_extra_env() {
  local scenario="$1"
  case "$scenario" in
    connected-udp-on)
      printf 'FIPS_CONNECTED_UDP=1'
      ;;
    connected-udp-off)
      printf 'FIPS_CONNECTED_UDP=0'
      ;;
    single-encrypt-worker)
      printf 'FIPS_CONNECTED_UDP=1 FIPS_ENCRYPT_WORKERS=1 FIPS_DECRYPT_WORKERS=0'
      ;;
    tight-send-backpressure)
      printf 'FIPS_CONNECTED_UDP=1 FIPS_WORKER_CHANNEL_CAP=%q FIPS_DECRYPT_WORKER_CHANNEL_CAP=%q FIPS_SEND_BACKPRESSURE_SLEEP_AFTER=1 FIPS_SEND_BACKPRESSURE_SLEEP_MICROS=%q FIPS_SEND_BACKPRESSURE_DROP_AFTER=%q' \
        "${NVPN_PLATFORM_MATRIX_TIGHT_WORKER_CHANNEL_CAP:-4}" \
        "${NVPN_PLATFORM_MATRIX_TIGHT_SEND_DECRYPT_WORKER_CHANNEL_CAP:-32768}" \
        "${NVPN_PLATFORM_MATRIX_BACKPRESSURE_SLEEP_MICROS:-500}" \
        "${NVPN_PLATFORM_MATRIX_BACKPRESSURE_DROP_AFTER:-0}"
      ;;
    tight-backpressure)
      printf 'FIPS_CONNECTED_UDP=1 FIPS_WORKER_CHANNEL_CAP=%q FIPS_SEND_BACKPRESSURE_SLEEP_AFTER=1 FIPS_SEND_BACKPRESSURE_SLEEP_MICROS=%q FIPS_SEND_BACKPRESSURE_DROP_AFTER=%q' \
        "${NVPN_PLATFORM_MATRIX_TIGHT_WORKER_CHANNEL_CAP:-4}" \
        "${NVPN_PLATFORM_MATRIX_BACKPRESSURE_SLEEP_MICROS:-500}" \
        "${NVPN_PLATFORM_MATRIX_BACKPRESSURE_DROP_AFTER:-0}"
      ;;
    *)
      echo "unknown platform matrix scenario: $scenario" >&2
      echo "known scenarios: connected-udp-on, connected-udp-off, single-encrypt-worker, tight-send-backpressure, tight-backpressure" >&2
      return 1
      ;;
  esac
}

failures=0
stop_matrix=0
IFS=',' read -r -a scenarios <<<"$SCENARIO_CSV"
for raw_scenario in "${scenarios[@]}"; do
  scenario="$(trim "$raw_scenario")"
  [[ -z "$scenario" ]] && continue

  scenario_env="$(scenario_extra_env "$scenario")"
  if [[ -n "$EXTRA_ENV" ]]; then
    scenario_env="$scenario_env $EXTRA_ENV"
  fi

  name="$(safe_name "$scenario")"

  for ((attempt = 1; attempt <= ATTEMPTS; attempt++)); do
    attempt_suffix=""
    if (( ATTEMPTS > 1 )); then
      attempt_suffix="-attempt-$attempt"
    fi

    project_name="$PROJECT_PREFIX-$name$attempt_suffix"
    log_path="$OUT_DIR/$name$attempt_suffix.log"
    phase_dir="$OUT_DIR/$name$attempt_suffix-perf"
    phase_summary="$phase_dir/phase-summary.tsv"
    failure_summary="$phase_dir/failure-summary.tsv"

    printf '%s\n' "--- platform matrix scenario: $scenario attempt $attempt/$ATTEMPTS ---"
    printf 'extra env: %s\n' "$scenario_env"
    printf 'log: %s\n' "$log_path"
    printf 'phase summary: %s\n' "$phase_summary"
    printf 'failure summary: %s\n' "$failure_summary"

    started_at="$(date -u +%s)"
    if env \
      PROJECT_NAME="$project_name" \
      NVPN_PERF_DURATION_SECS="$DURATION_SECS" \
      NVPN_PERF_LOAD_DURATION_SECS="$LOAD_DURATION_SECS" \
      NVPN_PERF_PING_COUNT="$PING_COUNT" \
      NVPN_PERF_PING_INTERVAL="$PING_INTERVAL" \
      NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP="$WORKER_QUEUE_PRESSURE_CAP" \
      NVPN_PERF_RX_MAINT_FAULT_MS="$RX_MAINT_FAULT_MS" \
      NVPN_PERF_PHASES="$PHASES" \
      NVPN_PERF_OUTPUT_DIR="$phase_dir" \
      NVPN_PERF_EXTRA_ENV="$scenario_env" \
      "$RUNNER" 2>&1 | tee "$log_path"; then
      status="pass"
    else
      status="fail"
      failures=$((failures + 1))
    fi
    finished_at="$(date -u +%s)"
    elapsed=$((finished_at - started_at))
    sha="$(sha256_file "$log_path")"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$scenario" "$status" "$elapsed" "$log_path" "$sha" "$phase_summary" "$scenario_env" "$attempt" "$ATTEMPTS" "$failure_summary" >>"$SUMMARY"

    printf '%s scenario %s attempt %s/%s in %ss, log sha256=%s\n' "$status" "$scenario" "$attempt" "$ATTEMPTS" "$elapsed" "$sha"
    if [[ "$status" != "pass" && "$FAIL_FAST" != "0" ]]; then
      stop_matrix=1
      break
    fi
  done

  if [[ "$stop_matrix" != "0" ]]; then
    break
  fi
done

printf 'platform matrix summary: %s\n' "$SUMMARY"
if (( failures > 0 )); then
  echo "fips platform matrix failed: $failures attempt(s) failed" >&2
  exit 1
fi

echo "fips platform matrix passed"
