#!/usr/bin/env bash
# Local self-tests for the Docker FIPS platform-matrix wrapper.
#
# These tests use a fake perf runner and do not start Docker. They pin scenario
# env construction so red-case probes keep testing the intended pressure source.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
MATRIX_SCRIPT="$ROOT_DIR/scripts/e2e-fips-platform-matrix-docker.sh"

fail() {
  printf 'fips platform matrix harness self-test failed: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  local got="$1"
  local want="$2"
  local label="$3"
  [[ "$got" == "$want" ]] || fail "$label: got '$got', want '$want'"
}

write_fake_runner() {
  local runner="$1"
  cat >"$runner" <<'RUNNER'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$NVPN_PERF_EXTRA_ENV" >>"$NVPN_FAKE_RUNNER_ENV_LOG"
if [[ -n "${NVPN_FAKE_RUNNER_PING_LOG:-}" ]]; then
  printf '%s\n' "$NVPN_PERF_PING_COUNT" >>"$NVPN_FAKE_RUNNER_PING_LOG"
fi
RUNNER
  chmod +x "$runner"
}

test_tight_send_backpressure_env_is_isolated_from_decrypt_cap() {
  local dir runner got want summary_env
  dir="$(mktemp -d)"
  runner="$dir/fake-runner.sh"
  write_fake_runner "$runner"

  NVPN_FAKE_RUNNER_ENV_LOG="$dir/env.log" \
  NVPN_PLATFORM_MATRIX_RUNNER="$runner" \
  NVPN_PLATFORM_MATRIX_OUTPUT_DIR="$dir/out" \
  NVPN_PLATFORM_MATRIX_SCENARIOS=tight-send-backpressure \
  NVPN_PLATFORM_MATRIX_TIGHT_WORKER_CHANNEL_CAP=5 \
  NVPN_PLATFORM_MATRIX_TIGHT_SEND_DECRYPT_WORKER_CHANNEL_CAP=1234 \
  NVPN_PLATFORM_MATRIX_BACKPRESSURE_SLEEP_MICROS=600 \
  NVPN_PLATFORM_MATRIX_BACKPRESSURE_DROP_AFTER=2 \
  "$MATRIX_SCRIPT" >"$dir/stdout"

  got="$(cat "$dir/env.log")"
  want="FIPS_WORKER_CHANNEL_CAP=5 FIPS_DECRYPT_WORKER_CHANNEL_CAP=1234 FIPS_SEND_BACKPRESSURE_SLEEP_AFTER=1 FIPS_SEND_BACKPRESSURE_SLEEP_MICROS=600 FIPS_SEND_BACKPRESSURE_DROP_AFTER=2"
  assert_eq "$got" "$want" "tight-send-backpressure runner env"

  summary_env="$(awk -F '\t' 'NR == 2 { print $7 }' "$dir/out/summary.tsv")"
  assert_eq "$summary_env" "$want" "tight-send-backpressure summary env"

  rm -rf "$dir"
}

test_tight_backpressure_keeps_combined_worker_pressure() {
  local dir runner got want
  dir="$(mktemp -d)"
  runner="$dir/fake-runner.sh"
  write_fake_runner "$runner"

  NVPN_FAKE_RUNNER_ENV_LOG="$dir/env.log" \
  NVPN_PLATFORM_MATRIX_RUNNER="$runner" \
  NVPN_PLATFORM_MATRIX_OUTPUT_DIR="$dir/out" \
  NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
  NVPN_PLATFORM_MATRIX_TIGHT_WORKER_CHANNEL_CAP=6 \
  NVPN_PLATFORM_MATRIX_BACKPRESSURE_SLEEP_MICROS=700 \
  "$MATRIX_SCRIPT" >"$dir/stdout"

  got="$(cat "$dir/env.log")"
  want="FIPS_WORKER_CHANNEL_CAP=6 FIPS_SEND_BACKPRESSURE_SLEEP_AFTER=1 FIPS_SEND_BACKPRESSURE_SLEEP_MICROS=700 FIPS_SEND_BACKPRESSURE_DROP_AFTER=0"
  assert_eq "$got" "$want" "tight-backpressure runner env"

  rm -rf "$dir"
}

test_default_scenarios_include_send_and_combined_backpressure() {
  local dir runner scenarios
  dir="$(mktemp -d)"
  runner="$dir/fake-runner.sh"
  write_fake_runner "$runner"

  NVPN_FAKE_RUNNER_ENV_LOG="$dir/env.log" \
  NVPN_PLATFORM_MATRIX_RUNNER="$runner" \
  NVPN_PLATFORM_MATRIX_OUTPUT_DIR="$dir/out" \
  "$MATRIX_SCRIPT" >"$dir/stdout"

  scenarios="$(awk -F '\t' 'NR > 1 { csv = csv sep $1; sep = "," } END { print csv }' "$dir/out/summary.tsv")"
  assert_eq \
    "$scenarios" \
    "default,single-encrypt-worker,tight-send-backpressure,tight-backpressure" \
    "default scenario order"

  rm -rf "$dir"
}

test_unknown_scenario_lists_tight_send_backpressure() {
  local dir runner err
  dir="$(mktemp -d)"
  runner="$dir/fake-runner.sh"
  write_fake_runner "$runner"
  err="$dir/stderr"

  if NVPN_FAKE_RUNNER_ENV_LOG="$dir/env.log" \
    NVPN_PLATFORM_MATRIX_RUNNER="$runner" \
    NVPN_PLATFORM_MATRIX_OUTPUT_DIR="$dir/out" \
    NVPN_PLATFORM_MATRIX_SCENARIOS=does-not-exist \
    "$MATRIX_SCRIPT" >"$dir/stdout" 2>"$err"; then
    cat "$dir/stdout" >&2
    fail "unknown scenario unexpectedly passed"
  fi

  if ! grep -Fq "tight-send-backpressure" "$err"; then
    cat "$err" >&2
    fail "unknown scenario stderr did not list tight-send-backpressure"
  fi

  rm -rf "$dir"
}

test_ping_count_default_and_override_are_forwarded() {
  local dir runner got
  dir="$(mktemp -d)"
  runner="$dir/fake-runner.sh"
  write_fake_runner "$runner"

  NVPN_FAKE_RUNNER_ENV_LOG="$dir/env-default.log" \
  NVPN_FAKE_RUNNER_PING_LOG="$dir/ping-default.log" \
  NVPN_PLATFORM_MATRIX_RUNNER="$runner" \
  NVPN_PLATFORM_MATRIX_OUTPUT_DIR="$dir/out-default" \
  NVPN_PLATFORM_MATRIX_SCENARIOS=default \
  "$MATRIX_SCRIPT" >"$dir/stdout-default"

  got="$(cat "$dir/ping-default.log")"
  assert_eq "$got" "60" "default matrix ping count"

  NVPN_FAKE_RUNNER_ENV_LOG="$dir/env-override.log" \
  NVPN_FAKE_RUNNER_PING_LOG="$dir/ping-override.log" \
  NVPN_PLATFORM_MATRIX_RUNNER="$runner" \
  NVPN_PLATFORM_MATRIX_OUTPUT_DIR="$dir/out-override" \
  NVPN_PLATFORM_MATRIX_SCENARIOS=default \
  NVPN_PLATFORM_MATRIX_PING_COUNT=17 \
  "$MATRIX_SCRIPT" >"$dir/stdout-override"

  got="$(cat "$dir/ping-override.log")"
  assert_eq "$got" "17" "override matrix ping count"

  rm -rf "$dir"
}

test_tight_send_backpressure_env_is_isolated_from_decrypt_cap
test_tight_backpressure_keeps_combined_worker_pressure
test_default_scenarios_include_send_and_combined_backpressure
test_unknown_scenario_lists_tight_send_backpressure
test_ping_count_default_and_override_are_forwarded

printf 'fips platform matrix harness self-test passed\n'
