#!/usr/bin/env bash
# Local self-tests for the Docker FIPS platform-matrix wrapper.
#
# These tests use a fake perf runner and do not start Docker. They pin wrapper
# behavior so the matrix stays a thin profile runner instead of growing stale
# daemon knobs.
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

test_extra_env_is_forwarded_to_runner_and_summary() {
  local dir runner got want summary_env
  dir="$(mktemp -d)"
  runner="$dir/fake-runner.sh"
  write_fake_runner "$runner"

  NVPN_FAKE_RUNNER_ENV_LOG="$dir/env.log" \
  NVPN_PLATFORM_MATRIX_RUNNER="$runner" \
  NVPN_PLATFORM_MATRIX_OUTPUT_DIR="$dir/out" \
  NVPN_PLATFORM_MATRIX_EXTRA_ENV="NVPN_DOCKER_DATAPLANE_PROFILE=linux-vnet-lan" \
  "$MATRIX_SCRIPT" >"$dir/stdout"

  got="$(cat "$dir/env.log")"
  want="NVPN_DOCKER_DATAPLANE_PROFILE=linux-vnet-lan"
  assert_eq "$got" "$want" "extra env runner env"

  summary_env="$(awk -F '\t' 'NR == 2 { print $7 }' "$dir/out/summary.tsv")"
  assert_eq "$summary_env" "$want" "extra env summary env"

  rm -rf "$dir"
}

test_default_scenarios_include_default_only() {
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
    "default" \
    "default scenario order"

  rm -rf "$dir"
}

test_unknown_scenario_lists_default() {
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

  if ! grep -Fq "known scenarios: default" "$err"; then
    cat "$err" >&2
    fail "unknown scenario stderr did not list default"
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

test_extra_env_is_forwarded_to_runner_and_summary
test_default_scenarios_include_default_only
test_unknown_scenario_lists_default
test_ping_count_default_and_override_are_forwarded

printf 'fips platform matrix harness self-test passed\n'
