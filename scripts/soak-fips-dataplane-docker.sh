#!/usr/bin/env bash
# Long-running nvpn+FIPS dataplane soak for the docker/VM path.
#
# Defaults to 30 minutes. Override NVPN_SOAK_DURATION_SECS for shorter local
# checks, or 3600 for a one-hour run. This intentionally validates only the
# Linux/container path; real Mac Wi-Fi/screenshare soak must run on actual Macs.
#
# Optional CPU contention mode:
#   NVPN_DOCKER_CPU_STRESS=1
#   NVPN_DOCKER_CPU_STRESS_SIDES=local|remote|both
#   NVPN_DOCKER_CPU_STRESS_{LOCAL,REMOTE}_WORKERS=N
#
# Benchmark-compatible daemon profiles:
#   NVPN_DOCKER_DATAPLANE_PROFILE=linux-vnet-lan
#   NVPN_DOCKER_PLACEMENT_PROFILE=worker-open
#   NVPN_SOAK_EXTRA_ENV="NVPN_PIPELINE_TRACE=1 ..."
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"
docker_bench_apply_local_fips_patch_default
PROJECT_NAME="${PROJECT_NAME:-nostr-vpn-soak-fips}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="${NVPN_SOAK_NETWORK_ID:-docker-fips-soak}"
CONFIG_PATH="/root/.config/nvpn/config.toml"
OUTPUT_DIR="${NVPN_SOAK_OUTPUT_DIR:-$ROOT_DIR/artifacts/fips-soak/$(date -u +%Y%m%dT%H%M%SZ)}"
SKIP_BUILD="${NVPN_SOAK_SKIP_BUILD:-0}"
DURATION_SECS="${NVPN_SOAK_DURATION_SECS:-1800}"
INTERVAL_SECS="${NVPN_SOAK_INTERVAL_SECS:-60}"
PING_COUNT="${NVPN_SOAK_PING_COUNT:-20}"
PING_INTERVAL="${NVPN_SOAK_PING_INTERVAL:-0.1}"
IPERF_DURATION="${NVPN_SOAK_IPERF_DURATION_SECS:-5}"
IPERF_TIMEOUT_SECS="${NVPN_SOAK_IPERF_TIMEOUT_SECS:-$((IPERF_DURATION + 30))}"
NVPN_SOAK_IPERF_TIMEOUT_SECS="$IPERF_TIMEOUT_SECS"
MAX_PING_LOSS_PERCENT="${NVPN_SOAK_MAX_PING_LOSS_PERCENT:-5}"
MAX_PING_AVG_MS="${NVPN_SOAK_MAX_PING_AVG_MS:-250}"
MAX_PING_P95_MS="${NVPN_SOAK_MAX_PING_P95_MS:-500}"
MAX_PING_P99_MS="${NVPN_SOAK_MAX_PING_P99_MS:-750}"
MAX_PING_MAX_MS="${NVPN_SOAK_MAX_PING_MAX_MS:-1000}"
MAX_SRTT_MS="${NVPN_SOAK_MAX_SRTT_MS:-1000}"
MAX_CPU_PERCENT="${NVPN_SOAK_MAX_CPU_PERCENT:-250}"
MAX_PING_AVG_DRIFT_MS="${NVPN_SOAK_MAX_PING_AVG_DRIFT_MS:-25}"
MAX_PING_AVG_DRIFT_FACTOR="${NVPN_SOAK_MAX_PING_AVG_DRIFT_FACTOR:-10}"
MAX_PING_P95_DRIFT_MS="${NVPN_SOAK_MAX_PING_P95_DRIFT_MS:-50}"
MAX_PING_P95_DRIFT_FACTOR="${NVPN_SOAK_MAX_PING_P95_DRIFT_FACTOR:-10}"
MAX_PING_P99_DRIFT_MS="${NVPN_SOAK_MAX_PING_P99_DRIFT_MS:-75}"
MAX_PING_P99_DRIFT_FACTOR="${NVPN_SOAK_MAX_PING_P99_DRIFT_FACTOR:-10}"
MAX_SRTT_DRIFT_MS="${NVPN_SOAK_MAX_SRTT_DRIFT_MS:-50}"
MAX_SRTT_DRIFT_FACTOR="${NVPN_SOAK_MAX_SRTT_DRIFT_FACTOR:-10}"
MAX_PIPELINE_QUEUE_WAIT_P95_MS="${NVPN_SOAK_MAX_PIPELINE_QUEUE_WAIT_P95_MS:-50}"
MAX_PIPELINE_QUEUE_WAIT_P99_MS="${NVPN_SOAK_MAX_PIPELINE_QUEUE_WAIT_P99_MS:-100}"
MAX_PRIORITY_QUEUE_WAIT_MS="${NVPN_SOAK_MAX_PRIORITY_QUEUE_WAIT_MS:-50}"
FAIL_ON_PRIORITY_HARD_EVENTS="${NVPN_SOAK_FAIL_ON_PRIORITY_HARD_EVENTS:-1}"
PIPELINE_INTERVAL_SECS="${NVPN_SOAK_PIPELINE_INTERVAL_SECS:-${NVPN_DOCKER_PIPELINE_INTERVAL_SECS:-15}}"
MAX_CONSECUTIVE_REKEY_SAMPLES="${NVPN_SOAK_MAX_CONSECUTIVE_REKEY_SAMPLES:-2}"
MAX_CONSECUTIVE_HIGH_SRTT_SAMPLES="${NVPN_SOAK_MAX_CONSECUTIVE_HIGH_SRTT_SAMPLES:-2}"
MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES="${NVPN_SOAK_MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES:-${NVPN_SOAK_MAX_CONSECUTIVE_DIRECT_PROBE_SAMPLES:-2}}"
MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES="${NVPN_SOAK_MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES:-2}"
MAX_FIPS_LAST_SEEN_AGE_SECS="${NVPN_SOAK_MAX_FIPS_LAST_SEEN_AGE_SECS:-180}"
MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS="${NVPN_SOAK_MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS:-$MAX_FIPS_LAST_SEEN_AGE_SECS}"
MAX_FIPS_DATA_LAST_SEEN_AGE_SECS="${NVPN_SOAK_MAX_FIPS_DATA_LAST_SEEN_AGE_SECS:-$MAX_FIPS_LAST_SEEN_AGE_SECS}"
MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS="${NVPN_SOAK_MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS:-5}"
EXPECT_FSP_OWNER_PLACEMENT="${NVPN_SOAK_EXPECT_FSP_OWNER_PLACEMENT:-${NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT:-}}"
ALLOW_NON_DIRECT="${NVPN_SOAK_ALLOW_NON_DIRECT:-0}"
ALLOW_QUEUE_EVENTS="${NVPN_SOAK_ALLOW_QUEUE_EVENTS:-${NVPN_SOAK_ALLOW_QUEUE_DROPS:-0}}"
ALLOW_QUEUE_WAIT="${NVPN_SOAK_ALLOW_QUEUE_WAIT:-$ALLOW_QUEUE_EVENTS}"
FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}"
EXTRA_ENV="${NVPN_SOAK_EXTRA_ENV:-}"

cleanup() {
  docker_bench_stop_cpu_stress
  if [[ -z "${KEEP:-}" ]]; then
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  fi
}

dump_debug() {
  set +e
  echo "nvpn+FIPS soak failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a node-b; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 120 "$service" || true
    echo "--- $service status ---"
    "${COMPOSE[@]}" exec -T "$service" nvpn status --json --discover-secs 0 || true
    echo "--- $service daemon.state.json ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "cat /root/.config/nvpn/daemon.state.json 2>/dev/null || true" || true
    echo "--- $service daemon.log ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "tail -n 240 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
    echo "--- $service routes ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "ip route || true" || true
    echo "--- $service nvpn cpu ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "ps -eo pid,pcpu,rss,etime,args | grep '[n]vpn' || true" || true
  done
}

on_exit() {
  local exit_code=$?
  if [[ $exit_code -ne 0 ]]; then
    dump_debug
  fi
  cleanup
  exit "$exit_code"
}

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

validate_expected_fsp_owner_placement() {
  case "$EXPECT_FSP_OWNER_PLACEMENT" in
    "" | any | same | owner-same | mismatch | owner-mismatch | local | handoff | worker-open) ;;
    *)
      echo "nvpn+FIPS soak failed: unknown expected FSP owner placement '$EXPECT_FSP_OWNER_PLACEMENT' (known: any, same, owner-same, mismatch, owner-mismatch, local, handoff, worker-open)" >&2
      return 2
      ;;
  esac
}

pipeline_fsp_owner_placement_line() {
  local sample="$1"
  local line summary
  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    summary="$(docker_bench_pipeline_fsp_owner_placement_summary "$line")"
    if [[ -n "$summary" ]]; then
      printf '%s\n' "$line"
      return 0
    fi
  done < <(jq -r '(.placement_raw // empty), (.load_raw // empty), (.raw // empty), (.recent[]? // empty)' <<<"$sample")
}

assert_expected_fsp_owner_placement_sample() {
  local label="$1"
  local sample="$2"
  local expected="$EXPECT_FSP_OWNER_PLACEMENT"
  local line summary kind ok=0
  [[ -n "$expected" && "$expected" != "any" ]] || return 0

  line="$(pipeline_fsp_owner_placement_line "$sample")"
  summary="$(docker_bench_pipeline_fsp_owner_placement_summary "$line")"
  kind="$(docker_bench_pipeline_fsp_owner_placement_kind "$line")"
  case "$expected" in
    same | owner-same)
      [[ "$summary" == owner=same,* ]] && ok=1
      ;;
    mismatch | owner-mismatch)
      [[ "$summary" == owner=mismatch,* ]] && ok=1
      ;;
    local | handoff | worker-open)
      [[ "$kind" == "$expected" ]] && ok=1
      ;;
  esac

  if [[ "$ok" != "1" ]]; then
    printf 'nvpn+FIPS soak failed: expected %s FSP owner placement %s, got kind=%s summary=%s\n' \
      "$label" "$expected" "${kind:-unknown}" "${summary:-none}" >&2
    return 2
  fi
}

assert_expected_fsp_owner_placement_any_sample() {
  local label sample line summary kind summaries="" ok=0
  [[ -n "$EXPECT_FSP_OWNER_PLACEMENT" && "$EXPECT_FSP_OWNER_PLACEMENT" != "any" ]] || return 0

  while [[ $# -gt 1 ]]; do
    label="$1"
    sample="$2"
    shift 2
    line="$(pipeline_fsp_owner_placement_line "$sample")"
    summary="$(docker_bench_pipeline_fsp_owner_placement_summary "$line")"
    kind="$(docker_bench_pipeline_fsp_owner_placement_kind "$line")"
    summaries+="${label}:kind=${kind:-unknown},summary=${summary:-none}; "
    case "$EXPECT_FSP_OWNER_PLACEMENT" in
      same | owner-same)
        [[ "$summary" == owner=same,* ]] && ok=1
        ;;
      mismatch | owner-mismatch)
        [[ "$summary" == owner=mismatch,* ]] && ok=1
        ;;
      local | handoff | worker-open)
        [[ "$kind" == "$EXPECT_FSP_OWNER_PLACEMENT" ]] && ok=1
        ;;
    esac
  done

  if [[ "$ok" != "1" ]]; then
    printf 'nvpn+FIPS soak failed: expected any FIPS FSP owner placement %s, got %s\n' \
      "$EXPECT_FSP_OWNER_PLACEMENT" "$summaries" >&2
    return 2
  fi
}

wait_for_service() {
  local service="$1"
  local container_id=""
  for _ in $(seq 1 30); do
    container_id="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$container_id" ]] \
      && [[ "$(docker inspect -f '{{.State.Running}}' "$container_id" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done

  echo "nvpn+FIPS soak failed: service '$service' did not reach running state" >&2
  exit 1
}

nostr_pubkey_from_config() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' '$CONFIG_PATH'
  " | tr -d '\r\"'
}

append_env_assignment() {
  local name="$1"
  local value="$2"
  [[ -z "$value" ]] && return
  printf " %s=%q" "$name" "$value"
}

append_env_assignments_string() {
  local value="$1"
  [[ -z "$value" ]] && return
  printf " %s" "$value"
}

validate_daemon_extra_env() {
  local profile_env combined_env
  profile_env="$(docker_bench_effective_extra_env)" || return $?
  docker_bench_validate_connect_env_scope "$profile_env" || return $?
  combined_env="$(docker_bench_join_env_assignments "$profile_env" "$EXTRA_ENV")"
  docker_bench_validate_extra_env_assignments "$combined_env"
}

daemon_env() {
  local env_string profile_env
  profile_env="$(docker_bench_effective_extra_env)"
  env_string=""
  env_string+="$(append_env_assignments_string "$profile_env")"
  env_string+="$(append_env_assignments_string "$EXTRA_ENV")"
  env_string+=" NVPN_PIPELINE_TRACE=1 NVPN_PIPELINE_INTERVAL_SECS=$PIPELINE_INTERVAL_SECS"
  env_string+=" NVPN_FIPS_NOSTR_DISCOVERY_POLICY='$FIPS_NOSTR_DISCOVERY_POLICY'"
  printf '%s' "$env_string"
}

start_daemon() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "$(daemon_env) nvpn start --daemon --connect >/dev/null"
}

stop_daemon() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "nvpn stop --force >/dev/null 2>&1 || pkill -9 nvpn 2>/dev/null || true"
}

wait_for_peer_status() {
  local service="$1"
  local expected_peer="$2"
  local expected_ip="$3"
  local label="$4"
  local attempts="$5"
  local status compact
  for _ in $(seq 1 "$attempts"); do
    status="$("${COMPOSE[@]}" exec -T "$service" nvpn status --json --discover-secs 0 | tr -d '\r')"
    compact="$(printf '%s' "$status" | tr -d '\n\r\t ')"
    if grep -q '"status_source":"daemon"' <<<"$compact" \
      && grep -q '"running":true' <<<"$compact" \
      && grep -q '"mesh_ready":true' <<<"$compact" \
      && jq -e \
        --arg peer "$expected_peer" \
        --arg expected_ip "$expected_ip" \
        --arg allow_non_direct "$ALLOW_NON_DIRECT" '
        .daemon.state.peers
        | any(
            (.participant_pubkey == $peer or .fips_endpoint_npub == $peer)
            and .reachable == true
            and (
              $allow_non_direct == "1"
              or ((.fips_transport_addr // "") | startswith($expected_ip + ":"))
            )
          )
      ' >/dev/null <<<"$status"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done

  printf '%s\n' "$status"
  echo "nvpn+FIPS soak failed: $label did not converge to expected direct path $expected_ip" >&2
  return 1
}

wait_for_mesh() {
  wait_for_peer_status "$1" "$2" "$3" "$4" 90
}

parse_ping_stats() {
  awk '
    /time=/ {
      sample = $0
      sub(/^.*time=/, "", sample)
      sub(/[[:space:]].*$/, "", sample)
      if (sample != "") {
        times[++count] = sample + 0
      }
    }
    /packets transmitted/ {
      loss = $0
      sub(/^.*received, /, "", loss)
      sub(/% packet loss.*$/, "", loss)
    }
    /^rtt / || /^round-trip / {
      split($0, parts, "=")
      split(parts[2], values, "/")
      avg = values[2]
      max = values[3]
      sub(/^ /, "", avg)
      sub(/^ /, "", max)
    }
    END {
      if (loss == "" || avg == "" || max == "" || count == 0) {
        exit 1
      }
      for (i = 1; i <= count; i++) {
        for (j = i + 1; j <= count; j++) {
          if (times[j] < times[i]) {
            tmp = times[i]
            times[i] = times[j]
            times[j] = tmp
          }
        }
      }
      p95_idx = int((count * 95 + 99) / 100)
      p99_idx = int((count * 99 + 99) / 100)
      if (p95_idx < 1) p95_idx = 1
      if (p99_idx < 1) p99_idx = 1
      if (p95_idx > count) p95_idx = count
      if (p99_idx > count) p99_idx = count
      printf "%s %s %s %s %s\n", loss, avg, times[p95_idx], times[p99_idx], max
    }
  '
}

assert_float_at_most() {
  local actual="$1"
  local max="$2"
  local label="$3"
  awk -v actual="$actual" -v max="$max" -v label="$label" '
    BEGIN {
      if ((actual + 0) > (max + 0)) {
        printf "nvpn+FIPS soak failed: %s %.1f above maximum %.1f\n", label, actual, max > "/dev/stderr"
        exit 1
      }
    }
  '
}

assert_float_drift_at_most() {
  local actual="$1"
  local baseline="$2"
  local max_delta="$3"
  local max_factor="$4"
  local label="$5"
  [[ -z "$actual" || "$actual" == "null" || -z "$baseline" || "$baseline" == "null" ]] && return
  awk \
    -v actual="$actual" \
    -v baseline="$baseline" \
    -v max_delta="$max_delta" \
    -v max_factor="$max_factor" \
    -v label="$label" '
    BEGIN {
      delta_limit = baseline + max_delta
      factor_limit = baseline * max_factor
      limit = (delta_limit > factor_limit) ? delta_limit : factor_limit
      if ((actual + 0) > limit) {
        printf "nvpn+FIPS soak failed: %s %.1f drifted above baseline %.1f limit %.1f\n", label, actual, baseline, limit > "/dev/stderr"
        exit 1
      }
    }
  '
}

is_number() {
  [[ "$1" =~ ^[0-9]+([.][0-9]+)?$ ]]
}

is_uint() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

epoch_ms() {
  printf '%s000\n' "$(date -u +%s)"
}

fips_last_seen_age_secs() {
  local last_seen="$1"
  local now="$2"
  if [[ -z "$last_seen" || "$last_seen" == "null" ]] || ! is_uint "$last_seen" || ! is_uint "$now"; then
    printf 'null\n'
    return
  fi
  if (( last_seen > now )); then
    printf '0\n'
  else
    printf '%s\n' $((now - last_seen))
  fi
}

assert_fips_liveness_fresh() {
  local label="$1"
  local last_seen="$2"
  local now="$3"
  assert_fips_timestamp_fresh "$label" "last_fips_seen_at" "$last_seen" "$now" "$MAX_FIPS_LAST_SEEN_AGE_SECS"
}

assert_fips_control_liveness_fresh() {
  local label="$1"
  local last_seen="$2"
  local now="$3"
  assert_fips_timestamp_fresh "$label" "last_fips_control_seen_at" "$last_seen" "$now" "$MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS"
}

assert_fips_data_liveness_fresh() {
  local label="$1"
  local last_seen="$2"
  local now="$3"
  assert_fips_timestamp_fresh "$label" "last_fips_data_seen_at" "$last_seen" "$now" "$MAX_FIPS_DATA_LAST_SEEN_AGE_SECS"
}

assert_fips_timestamp_fresh() {
  local label="$1"
  local field="$2"
  local last_seen="$3"
  local now="$4"
  local max_age="$5"
  local age future_by
  if [[ -z "$last_seen" || "$last_seen" == "null" ]]; then
    echo "nvpn+FIPS soak failed: $label $field is missing" >&2
    exit 1
  fi
  if ! is_uint "$last_seen"; then
    echo "nvpn+FIPS soak failed: $label $field is not numeric: $last_seen" >&2
    exit 1
  fi
  if ! is_uint "$now"; then
    echo "nvpn+FIPS soak failed: $label sample timestamp is not numeric: $now" >&2
    exit 1
  fi
  if (( last_seen > now + MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS )); then
    future_by=$((last_seen - now))
    echo "nvpn+FIPS soak failed: $label $field is ${future_by}s in the future ($field=$last_seen now=$now)" >&2
    exit 1
  fi
  age="$(fips_last_seen_age_secs "$last_seen" "$now")"
  if (( age > max_age )); then
    echo "nvpn+FIPS soak failed: $label $field is stale (age=${age}s max=${max_age}s $field=$last_seen now=$now)" >&2
    exit 1
  fi
}

assert_counter_advanced() {
  local actual="$1"
  local previous="$2"
  local label="$3"
  [[ -z "$previous" ]] && return
  if [[ ! "$actual" =~ ^[0-9]+$ || ! "$previous" =~ ^[0-9]+$ ]]; then
    echo "nvpn+FIPS soak failed: $label counter is not numeric (actual=$actual previous=$previous)" >&2
    exit 1
  fi
  if (( actual <= previous )); then
    echo "nvpn+FIPS soak failed: $label counter did not advance (actual=$actual previous=$previous)" >&2
    exit 1
  fi
}

record_rekey_progress() {
  local label="$1"
  local in_progress="$2"
  local draining="$3"
  local counter_var="$4"
  local current
  if [[ "$in_progress" == "true" || "$draining" == "true" ]]; then
    current="${!counter_var}"
    current=$((current + 1))
    printf -v "$counter_var" '%s' "$current"
    if (( current > MAX_CONSECUTIVE_REKEY_SAMPLES )); then
      echo "nvpn+FIPS soak failed: $label rekey state stayed active for ${current} consecutive sample(s) (in_progress=$in_progress draining=$draining)" >&2
      exit 1
    fi
  else
    printf -v "$counter_var" '%s' 0
  fi
}

record_srtt_progress() {
  local label="$1"
  local srtt="$2"
  local counter_var="$3"
  local current
  if ! is_number "$srtt"; then
    return
  fi
  if awk -v actual="$srtt" -v max="$MAX_SRTT_MS" 'BEGIN { exit !((actual + 0) <= (max + 0)) }'; then
    printf -v "$counter_var" '%s' 0
    return
  fi
  current="${!counter_var}"
  current=$((current + 1))
  printf -v "$counter_var" '%s' "$current"
  if (( current > MAX_CONSECUTIVE_HIGH_SRTT_SAMPLES )); then
    echo "nvpn+FIPS soak failed: $label FIPS SRTT stayed above ${MAX_SRTT_MS}ms for ${current} consecutive sample(s) (srtt_ms=$srtt)" >&2
    exit 1
  fi
}

record_direct_probe_progress() {
  local label="$1"
  local pending="$2"
  local retry_after_ms="$3"
  local now_ms="$4"
  local pending_counter_var="$5"
  local overdue_counter_var="$6"
  local current_pending current_overdue
  if [[ "$pending" == "true" ]]; then
    current_pending="${!pending_counter_var}"
    current_pending=$((current_pending + 1))
    printf -v "$pending_counter_var" '%s' "$current_pending"
    if ! is_uint "$retry_after_ms" || (( retry_after_ms <= now_ms )); then
      current_overdue="${!overdue_counter_var}"
      current_overdue=$((current_overdue + 1))
      printf -v "$overdue_counter_var" '%s' "$current_overdue"
    else
      printf -v "$overdue_counter_var" '%s' 0
      current_overdue=0
    fi
    if (( current_overdue > MAX_CONSECUTIVE_DIRECT_PROBE_OVERDUE_SAMPLES )); then
      echo "nvpn+FIPS soak failed: $label direct probe stayed overdue for ${current_overdue} consecutive sample(s) (retry_after_ms=$retry_after_ms now_ms=$now_ms pending_samples=$current_pending)" >&2
      exit 1
    fi
  else
    printf -v "$pending_counter_var" '%s' 0
    printf -v "$overdue_counter_var" '%s' 0
  fi
}

assert_ping_stats_ok() {
  local label="$1"
  local loss="$2"
  local avg="$3"
  local p95="$4"
  local p99="$5"
  local max="$6"
  assert_float_at_most "$loss" "$MAX_PING_LOSS_PERCENT" "$label ping loss %"
  assert_float_at_most "$avg" "$MAX_PING_AVG_MS" "$label ping avg ms"
  assert_float_at_most "$p95" "$MAX_PING_P95_MS" "$label ping p95 ms"
  assert_float_at_most "$p99" "$MAX_PING_P99_MS" "$label ping p99 ms"
  assert_float_at_most "$max" "$MAX_PING_MAX_MS" "$label ping max ms"
}

ping_probe() {
  local service="$1"
  local target_ip="$2"
  local label="$3"
  local output stats loss avg p95 p99 max
  output="$("${COMPOSE[@]}" exec -T "$service" ping \
    -c "$PING_COUNT" -i "$PING_INTERVAL" -W 2 "$target_ip" 2>&1)"
  if ! stats="$(printf '%s\n' "$output" | parse_ping_stats)"; then
    echo "nvpn+FIPS soak failed: could not parse ping stats for $label" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  read -r loss avg p95 p99 max <<<"$stats"
  assert_ping_stats_ok "$label" "$loss" "$avg" "$p95" "$p99" "$max"
  printf '%s %s %s %s %s' "$loss" "$avg" "$p95" "$p99" "$max"
}

start_iperf_server() {
  "${COMPOSE[@]}" exec -T node-b sh -c "pkill -9 iperf3 2>/dev/null; true" >/dev/null
  "${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
  sleep 1
}

iperf_probe() {
  local label="$1"
  shift
  local json code mbps retrans
  if json="$("${COMPOSE[@]}" exec -T node-a timeout --kill-after=5s "$IPERF_TIMEOUT_SECS" iperf3 \
    -J -c "$BOB_TUNNEL_IP" -t "$IPERF_DURATION" -O 1 --connect-timeout 3000 "$@" 2>&1)"; then
    code=0
  else
    code=$?
    if [[ "$code" -eq 124 || "$code" -eq 137 ]]; then
      echo "nvpn+FIPS soak failed: iperf $label timed out after ${IPERF_TIMEOUT_SECS}s" >&2
    else
      echo "nvpn+FIPS soak failed: iperf $label failed with exit $code" >&2
    fi
    printf '%s\n' "$json" >&2
    exit 1
  fi
  if ! mbps="$(printf '%s\n' "$json" | jq -er '((.end.sum_received.bits_per_second // .end.sum.bits_per_second // .end.sum_sent.bits_per_second) | select(type == "number")) / 1000000')"; then
    echo "nvpn+FIPS soak failed: iperf $label returned no throughput result" >&2
    printf '%s\n' "$json" >&2
    exit 1
  fi
  retrans="$(printf '%s\n' "$json" | jq -r '(.end.sum_sent.retransmits // .end.sum.retransmits // 0)')"
  printf '%s %s' "$mbps" "$retrans"
}

peer_field() {
  local status="$1"
  local peer="$2"
  local field="$3"
  jq -r --arg peer "$peer" --arg field "$field" '
    .daemon.state.peers[]
    | select(.participant_pubkey == $peer or .fips_endpoint_npub == $peer)
    | .[$field] // ""
  ' <<<"$status" | head -n1
}

assert_peer_path() {
  local status="$1"
  local peer="$2"
  local expected_ip="$3"
  local label="$4"
  local reachable transport_addr
  reachable="$(peer_field "$status" "$peer" reachable)"
  transport_addr="$(peer_field "$status" "$peer" fips_transport_addr)"
  if [[ "$reachable" != "true" ]]; then
    echo "nvpn+FIPS soak failed: $label peer is not reachable" >&2
    printf '%s\n' "$status" >&2
    exit 1
  fi
  if [[ "$ALLOW_NON_DIRECT" == "0" && "$transport_addr" != "$expected_ip:"* ]]; then
    echo "nvpn+FIPS soak failed: $label route changed away from direct UDP path (addr=$transport_addr expected_ip=$expected_ip)" >&2
    printf '%s\n' "$status" >&2
    exit 1
  fi
}

daemon_cpu_percent() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "ps -eo pcpu,args | awk '/[n]vpn/ && /daemon|connect/ { sum += \$1 } END { printf \"%.1f\", sum }'" | tr -d '\r'
}

assert_cpu_ok() {
  local service="$1"
  local cpu="$2"
  assert_float_at_most "$cpu" "$MAX_CPU_PERCENT" "$service daemon CPU %"
}

pipeline_lines_to_json() {
  local lines="$1"
  printf '%s' "$lines" | jq -Rsc '
    . as $lines
    |
    def line_array: ($lines | split("\n") | map(select(length > 0)));
    def latest: (line_array | last // "");
    def duration_ms($value; $unit):
      if $unit == "ns" then ($value / 1000000)
      elif $unit == "us" then ($value / 1000)
      elif $unit == "ms" then $value
      elif $unit == "s" then ($value * 1000)
      else null
      end;
    def event_rate_in($line; $name):
      ([
        $line
        | capture("(^| )" + $name + "=(?<value>[0-9]+(?:\\.[0-9]+)?)/s(?: |$)")?
        | .value
        | tonumber
      ] | first // null);
    def event_rate($name):
      event_rate_in(latest; $name);
    def event_max_rate($name):
      ([line_array[] | event_rate_in(.; $name) | select(. != null)] | max // null);
    def event_total($name):
      ([
        line_array[]
        | capture("(^| )" + $name + "=[0-9]+(?:\\.[0-9]+)?/s total=(?<value>[0-9]+)(?: |$)")?
        | .value
        | tonumber
      ] | max // null);
    def event_seen($name):
      any(
        line_array[];
        (
          capture("(^| )" + $name + "=(?<rate>[0-9]+(?:\\.[0-9]+)?)/s(?: total=(?<total>[0-9]+))?(?: |$)")?
          // null
        ) as $event
        | $event != null
          and (
            (($event.rate | tonumber) > 0)
            or ($event.total == null)
            or (($event.total | tonumber) > 0)
          )
      );
    def wait_point($line; $name):
      (
        $line
        | capture(
            "(^| )" + $name
            + "=[0-9]+(?:\\.[0-9]+)?/s"
            + " avg=[0-9]+(?:\\.[0-9]+)?(?:ns|us|ms|s)"
            + " p50<=[0-9]+(?:\\.[0-9]+)?(?:ns|us|ms|s)"
            + " p95<=(?<p95>[0-9]+(?:\\.[0-9]+)?)(?<p95_unit>ns|us|ms|s)"
            + " p99<=(?<p99>[0-9]+(?:\\.[0-9]+)?)(?<p99_unit>ns|us|ms|s)"
            + " max<=(?<max>[0-9]+(?:\\.[0-9]+)?)(?<max_unit>ns|us|ms|s)"
            + " allmax=(?<allmax>[0-9]+(?:\\.[0-9]+)?)(?<allmax_unit>ns|us|ms|s)"
          )?
      )
      | if . == null then null else {
          p95_ms: duration_ms((.p95 | tonumber); .p95_unit),
          p99_ms: duration_ms((.p99 | tonumber); .p99_unit),
          max_ms: duration_ms((.max | tonumber); .max_unit),
          allmax_ms: duration_ms((.allmax | tonumber); .allmax_unit)
        } end;
    def wait_latest($name):
      ([latest | wait_point(.; $name) | select(. != null)] | first // null);
    def wait_max($name; $field):
      ([line_array[] | wait_point(.; $name) | select(. != null) | .[$field]] | max // null);
    def wait_metric($name):
      {
        latest: wait_latest($name),
        max_observed: {
          p95_ms: wait_max($name; "p95_ms"),
          p99_ms: wait_max($name; "p99_ms"),
          max_ms: wait_max($name; "max_ms"),
          allmax_ms: wait_max($name; "allmax_ms")
        }
      };
    def load_score($line):
      reduce [
        "fmp_worker_batch_packets",
        "udp_send_connected",
        "connected_udp_direct_decrypt",
        "endpoint_send",
        "fmp_decrypt",
        "nvpn_tun_read",
        "nvpn_tun_write",
        "nvpn_tun_to_mesh_queue_wait",
        "nvpn_mesh_send"
      ][] as $name (-1;
        (event_rate_in($line; $name) // -1) as $score
        | if $score > . then $score else . end
      );
    def placement_score($line):
      (
        [
          event_rate_in($line; "decrypt_worker_batch_bulk_packets"),
          event_rate_in($line; "decrypt_worker_select_bulk_packets"),
          event_rate_in($line; "decrypt_worker_drain_bulk_packets"),
          event_rate_in($line; "decrypt_authenticated_session_bulk_wait"),
          event_rate_in($line; "decrypt_fsp_path_worker_open"),
          event_rate_in($line; "decrypt_fsp_path_worker_open_striped")
        ]
        | map(. // 0)
        | max
      ) as $score
      | if $score > 0 then (1000000000000 + $score) else load_score($line) end;
    def peak_wait_score($line):
      (
        [
          "endpoint_command_wait",
          "endpoint_priority_command_wait",
          "endpoint_bulk_command_wait",
          "endpoint_event_wait",
          "endpoint_priority_event_wait",
          "endpoint_bulk_event_wait",
          "fmp_worker_queue_wait",
          "fmp_worker_priority_queue_wait",
          "fmp_worker_bulk_queue_wait",
          "fmp_linux_bulk_container_queue_wait",
          "fmp_linux_bulk_container_ready_wait",
          "fmp_linux_bulk_container_first_slot_wait",
          "fmp_linux_bulk_container_all_slots_wait",
          "decrypt_worker_queue_wait",
          "decrypt_worker_priority_queue_wait",
          "decrypt_worker_bulk_queue_wait",
          "decrypt_fallback_wait",
          "decrypt_fallback_priority_wait",
          "decrypt_fallback_bulk_wait",
          "fsp_aead_worker_open_queue_wait",
          "fsp_aead_worker_open_completion_wait",
          "decrypt_authenticated_session_wait",
          "decrypt_authenticated_session_priority_wait",
          "decrypt_authenticated_session_bulk_wait",
          "decrypt_direct_session_commit_wait",
          "decrypt_direct_session_data_wait",
          "decrypt_fsp_worker_queue_wait",
          "decrypt_fsp_worker_priority_queue_wait",
          "decrypt_fsp_worker_bulk_queue_wait",
          "transport_queue_wait",
          "transport_priority_queue_wait",
          "transport_bulk_queue_wait",
          "transport_channel_wait",
          "transport_priority_channel_wait",
          "transport_bulk_channel_wait",
          "transport_rx_loop_wait",
          "transport_priority_rx_loop_wait",
          "transport_bulk_rx_loop_wait",
          "connected_udp_drain_ring_wait",
          "connected_udp_drain_priority_ring_wait",
          "connected_udp_drain_bulk_ring_wait",
          "nvpn_tun_to_mesh_queue_wait"
        ] as $names
        | [
            $names[] as $name
            | wait_point($line; $name)
            | select(. != null)
            | .p99_ms, .p95_ms, .max_ms, .allmax_ms
          ]
        | max // -1
      );
    def load_raw:
      (
        reduce line_array[] as $line ({score: -1, line: ""};
          (load_score($line)) as $score
          | if $score >= .score then {score: $score, line: $line} else . end
        )
        | .line
      );
    def peak_wait_raw:
      (
        reduce line_array[] as $line ({score: -1, line: ""};
          (peak_wait_score($line)) as $score
          | if $score >= .score then {score: $score, line: $line} else . end
        )
        | .line
      );
    def placement_raw:
      (
        reduce line_array[] as $line ({score: -1, line: ""};
          (placement_score($line)) as $score
          | if $score >= .score then {score: $score, line: $line} else . end
        )
        | .line
      );
    {
      line_count: (line_array | length),
      raw: (if latest == "" then null else latest end),
      load_raw: (load_raw | if . == "" then null else . end),
      placement_raw: (placement_raw | if . == "" then null else . end),
      peak_wait_raw: (peak_wait_raw | if . == "" then null else . end),
      recent: (line_array | .[-3:]),
      rates_per_sec: {
        udp_send_connected: event_rate("udp_send_connected"),
        udp_send_wildcard: event_rate("udp_send_wildcard"),
        udp_send_backpressure: event_rate("udp_send_backpressure"),
        udp_send_backpressure_sleep: event_rate("udp_send_backpressure_sleep"),
        udp_send_bulk_dropped: event_rate("udp_send_bulk_dropped"),
        connected_udp_installed: event_rate("connected_udp_installed"),
        connected_udp_activation_failed: event_rate("connected_udp_activation_failed"),
        connected_udp_peer_cap_skipped: event_rate("connected_udp_peer_cap_skipped"),
        connected_udp_fd_budget_skipped: event_rate("connected_udp_fd_budget_skipped"),
        connected_udp_kernel_dropped: event_rate("connected_udp_kernel_dropped"),
        connected_udp_peer_kernel_dropped: event_rate("connected_udp_peer_kernel_dropped"),
        connected_udp_drain_bulk_dropped: event_rate("connected_udp_drain_bulk_dropped"),
        encrypt_worker_queue_full: event_rate("encrypt_worker_queue_full"),
        encrypt_worker_priority_queue_full: event_rate("encrypt_worker_priority_queue_full"),
        encrypt_worker_bulk_queue_full: event_rate("encrypt_worker_bulk_queue_full"),
        encrypt_worker_bulk_dropped: event_rate("encrypt_worker_bulk_dropped"),
        decrypt_worker_queue_full: event_rate("decrypt_worker_queue_full"),
        decrypt_worker_bulk_dropped: event_rate("decrypt_worker_bulk_dropped"),
        decrypt_worker_register_full: event_rate("decrypt_worker_register_full"),
        decrypt_worker_priority_dropped: event_rate("decrypt_worker_priority_dropped"),
        decrypt_fallback_priority_gated: event_rate("decrypt_fallback_priority_gated"),
        decrypt_fsp_priority_queue_full_fallback: event_rate("decrypt_fsp_priority_queue_full_fallback"),
        decrypt_fsp_bulk_queue_full_fallback: event_rate("decrypt_fsp_bulk_queue_full_fallback"),
        decrypt_fsp_helper_window_fallback: event_rate("decrypt_fsp_helper_window_fallback"),
        decrypt_fsp_open_worker_window_fallback: event_rate("decrypt_fsp_open_worker_window_fallback"),
        decrypt_fsp_helper_queue_full_fallback: event_rate("decrypt_fsp_helper_queue_full_fallback"),
        decrypt_fsp_helper_completion_backlog_fallback: event_rate("decrypt_fsp_helper_completion_backlog_fallback"),
        decrypt_fsp_open_worker_completion_backlog_fallback: event_rate("decrypt_fsp_open_worker_completion_backlog_fallback"),
        decrypt_fsp_worker_replay_dropped: event_rate("decrypt_fsp_worker_replay_dropped"),
        fmp_aead_completion_aead_failed: event_rate("fmp_aead_completion_aead_failed"),
        fsp_aead_completion_aead_failed: event_rate("fsp_aead_completion_aead_failed"),
        fsp_aead_completion_epoch_mismatch: event_rate("fsp_aead_completion_epoch_mismatch"),
        decrypt_authenticated_session_priority_dropped: event_rate("decrypt_authenticated_session_priority_dropped"),
        decrypt_authenticated_session_bulk_dropped: event_rate("decrypt_authenticated_session_bulk_dropped"),
        decrypt_fallback_backlog_high: event_rate("decrypt_fallback_backlog_high"),
        decrypt_fallback_bulk_dropped: event_rate("decrypt_fallback_bulk_dropped"),
        decrypt_fallback_priority_dropped: event_rate("decrypt_fallback_priority_dropped"),
        pending_tun_destination_dropped: event_rate("pending_tun_destination_dropped"),
        pending_tun_packet_dropped: event_rate("pending_tun_packet_dropped"),
        pending_endpoint_destination_dropped: event_rate("pending_endpoint_destination_dropped"),
        pending_endpoint_packet_dropped: event_rate("pending_endpoint_packet_dropped"),
        endpoint_event_backlog_high: event_rate("endpoint_event_backlog_high"),
        endpoint_event_bulk_dropped: event_rate("endpoint_event_bulk_dropped"),
        transport_channel_backlog_high: event_rate("transport_channel_backlog_high"),
        transport_bulk_dropped: event_rate("transport_bulk_dropped"),
        nvpn_tun_to_mesh_bulk_dropped: event_rate("nvpn_tun_to_mesh_bulk_dropped"),
        nvpn_tun_to_mesh_bulk_dropped_batches: event_rate("nvpn_tun_to_mesh_bulk_dropped_batches"),
        nvpn_tun_to_mesh_bulk_dropped_packet_cap: event_rate("nvpn_tun_to_mesh_bulk_dropped_packet_cap"),
        nvpn_tun_to_mesh_bulk_dropped_channel_full: event_rate("nvpn_tun_to_mesh_bulk_dropped_channel_full")
      },
      max_rates_per_sec: {
        udp_send_connected: event_max_rate("udp_send_connected"),
        udp_send_wildcard: event_max_rate("udp_send_wildcard"),
        udp_send_backpressure: event_max_rate("udp_send_backpressure"),
        udp_send_backpressure_sleep: event_max_rate("udp_send_backpressure_sleep"),
        udp_send_bulk_dropped: event_max_rate("udp_send_bulk_dropped"),
        connected_udp_installed: event_max_rate("connected_udp_installed"),
        connected_udp_activation_failed: event_max_rate("connected_udp_activation_failed"),
        connected_udp_peer_cap_skipped: event_max_rate("connected_udp_peer_cap_skipped"),
        connected_udp_fd_budget_skipped: event_max_rate("connected_udp_fd_budget_skipped"),
        connected_udp_kernel_dropped: event_max_rate("connected_udp_kernel_dropped"),
        connected_udp_peer_kernel_dropped: event_max_rate("connected_udp_peer_kernel_dropped"),
        connected_udp_drain_bulk_dropped: event_max_rate("connected_udp_drain_bulk_dropped"),
        encrypt_worker_queue_full: event_max_rate("encrypt_worker_queue_full"),
        encrypt_worker_priority_queue_full: event_max_rate("encrypt_worker_priority_queue_full"),
        encrypt_worker_bulk_queue_full: event_max_rate("encrypt_worker_bulk_queue_full"),
        encrypt_worker_bulk_dropped: event_max_rate("encrypt_worker_bulk_dropped"),
        decrypt_worker_queue_full: event_max_rate("decrypt_worker_queue_full"),
        decrypt_worker_bulk_dropped: event_max_rate("decrypt_worker_bulk_dropped"),
        decrypt_worker_register_full: event_max_rate("decrypt_worker_register_full"),
        decrypt_worker_priority_dropped: event_max_rate("decrypt_worker_priority_dropped"),
        decrypt_fallback_priority_gated: event_max_rate("decrypt_fallback_priority_gated"),
        decrypt_fsp_priority_queue_full_fallback: event_max_rate("decrypt_fsp_priority_queue_full_fallback"),
        decrypt_fsp_bulk_queue_full_fallback: event_max_rate("decrypt_fsp_bulk_queue_full_fallback"),
        decrypt_fsp_helper_window_fallback: event_max_rate("decrypt_fsp_helper_window_fallback"),
        decrypt_fsp_open_worker_window_fallback: event_max_rate("decrypt_fsp_open_worker_window_fallback"),
        decrypt_fsp_helper_queue_full_fallback: event_max_rate("decrypt_fsp_helper_queue_full_fallback"),
        decrypt_fsp_helper_completion_backlog_fallback: event_max_rate("decrypt_fsp_helper_completion_backlog_fallback"),
        decrypt_fsp_open_worker_completion_backlog_fallback: event_max_rate("decrypt_fsp_open_worker_completion_backlog_fallback"),
        decrypt_fsp_worker_replay_dropped: event_max_rate("decrypt_fsp_worker_replay_dropped"),
        fmp_aead_completion_aead_failed: event_max_rate("fmp_aead_completion_aead_failed"),
        fsp_aead_completion_aead_failed: event_max_rate("fsp_aead_completion_aead_failed"),
        fsp_aead_completion_epoch_mismatch: event_max_rate("fsp_aead_completion_epoch_mismatch"),
        decrypt_authenticated_session_priority_dropped: event_max_rate("decrypt_authenticated_session_priority_dropped"),
        decrypt_authenticated_session_bulk_dropped: event_max_rate("decrypt_authenticated_session_bulk_dropped"),
        decrypt_fallback_backlog_high: event_max_rate("decrypt_fallback_backlog_high"),
        decrypt_fallback_bulk_dropped: event_max_rate("decrypt_fallback_bulk_dropped"),
        decrypt_fallback_priority_dropped: event_max_rate("decrypt_fallback_priority_dropped"),
        pending_tun_destination_dropped: event_max_rate("pending_tun_destination_dropped"),
        pending_tun_packet_dropped: event_max_rate("pending_tun_packet_dropped"),
        pending_endpoint_destination_dropped: event_max_rate("pending_endpoint_destination_dropped"),
        pending_endpoint_packet_dropped: event_max_rate("pending_endpoint_packet_dropped"),
        endpoint_event_backlog_high: event_max_rate("endpoint_event_backlog_high"),
        endpoint_event_bulk_dropped: event_max_rate("endpoint_event_bulk_dropped"),
        transport_channel_backlog_high: event_max_rate("transport_channel_backlog_high"),
        transport_bulk_dropped: event_max_rate("transport_bulk_dropped"),
        nvpn_tun_to_mesh_bulk_dropped: event_max_rate("nvpn_tun_to_mesh_bulk_dropped"),
        nvpn_tun_to_mesh_bulk_dropped_batches: event_max_rate("nvpn_tun_to_mesh_bulk_dropped_batches"),
        nvpn_tun_to_mesh_bulk_dropped_packet_cap: event_max_rate("nvpn_tun_to_mesh_bulk_dropped_packet_cap"),
        nvpn_tun_to_mesh_bulk_dropped_channel_full: event_max_rate("nvpn_tun_to_mesh_bulk_dropped_channel_full")
      },
      max_totals: {
        udp_send_connected: event_total("udp_send_connected"),
        udp_send_wildcard: event_total("udp_send_wildcard"),
        udp_send_backpressure: event_total("udp_send_backpressure"),
        udp_send_backpressure_sleep: event_total("udp_send_backpressure_sleep"),
        udp_send_bulk_dropped: event_total("udp_send_bulk_dropped"),
        connected_udp_installed: event_total("connected_udp_installed"),
        connected_udp_activation_failed: event_total("connected_udp_activation_failed"),
        connected_udp_peer_cap_skipped: event_total("connected_udp_peer_cap_skipped"),
        connected_udp_fd_budget_skipped: event_total("connected_udp_fd_budget_skipped"),
        connected_udp_kernel_dropped: event_total("connected_udp_kernel_dropped"),
        connected_udp_peer_kernel_dropped: event_total("connected_udp_peer_kernel_dropped"),
        connected_udp_drain_bulk_dropped: event_total("connected_udp_drain_bulk_dropped"),
        encrypt_worker_queue_full: event_total("encrypt_worker_queue_full"),
        encrypt_worker_priority_queue_full: event_total("encrypt_worker_priority_queue_full"),
        encrypt_worker_bulk_queue_full: event_total("encrypt_worker_bulk_queue_full"),
        encrypt_worker_bulk_dropped: event_total("encrypt_worker_bulk_dropped"),
        decrypt_worker_queue_full: event_total("decrypt_worker_queue_full"),
        decrypt_worker_bulk_dropped: event_total("decrypt_worker_bulk_dropped"),
        decrypt_worker_register_full: event_total("decrypt_worker_register_full"),
        decrypt_worker_priority_dropped: event_total("decrypt_worker_priority_dropped"),
        decrypt_fallback_priority_gated: event_total("decrypt_fallback_priority_gated"),
        decrypt_fsp_priority_queue_full_fallback: event_total("decrypt_fsp_priority_queue_full_fallback"),
        decrypt_fsp_bulk_queue_full_fallback: event_total("decrypt_fsp_bulk_queue_full_fallback"),
        decrypt_fsp_helper_window_fallback: event_total("decrypt_fsp_helper_window_fallback"),
        decrypt_fsp_open_worker_window_fallback: event_total("decrypt_fsp_open_worker_window_fallback"),
        decrypt_fsp_helper_queue_full_fallback: event_total("decrypt_fsp_helper_queue_full_fallback"),
        decrypt_fsp_helper_completion_backlog_fallback: event_total("decrypt_fsp_helper_completion_backlog_fallback"),
        decrypt_fsp_open_worker_completion_backlog_fallback: event_total("decrypt_fsp_open_worker_completion_backlog_fallback"),
        decrypt_fsp_worker_replay_dropped: event_total("decrypt_fsp_worker_replay_dropped"),
        fmp_aead_completion_aead_failed: event_total("fmp_aead_completion_aead_failed"),
        fsp_aead_completion_aead_failed: event_total("fsp_aead_completion_aead_failed"),
        fsp_aead_completion_epoch_mismatch: event_total("fsp_aead_completion_epoch_mismatch"),
        decrypt_authenticated_session_priority_dropped: event_total("decrypt_authenticated_session_priority_dropped"),
        decrypt_authenticated_session_bulk_dropped: event_total("decrypt_authenticated_session_bulk_dropped"),
        decrypt_fallback_backlog_high: event_total("decrypt_fallback_backlog_high"),
        decrypt_fallback_bulk_dropped: event_total("decrypt_fallback_bulk_dropped"),
        decrypt_fallback_priority_dropped: event_total("decrypt_fallback_priority_dropped"),
        pending_tun_destination_dropped: event_total("pending_tun_destination_dropped"),
        pending_tun_packet_dropped: event_total("pending_tun_packet_dropped"),
        pending_endpoint_destination_dropped: event_total("pending_endpoint_destination_dropped"),
        pending_endpoint_packet_dropped: event_total("pending_endpoint_packet_dropped"),
        endpoint_event_backlog_high: event_total("endpoint_event_backlog_high"),
        endpoint_event_bulk_dropped: event_total("endpoint_event_bulk_dropped"),
        transport_channel_backlog_high: event_total("transport_channel_backlog_high"),
        transport_bulk_dropped: event_total("transport_bulk_dropped"),
        nvpn_tun_to_mesh_bulk_dropped: event_total("nvpn_tun_to_mesh_bulk_dropped"),
        nvpn_tun_to_mesh_bulk_dropped_batches: event_total("nvpn_tun_to_mesh_bulk_dropped_batches"),
        nvpn_tun_to_mesh_bulk_dropped_packet_cap: event_total("nvpn_tun_to_mesh_bulk_dropped_packet_cap"),
        nvpn_tun_to_mesh_bulk_dropped_channel_full: event_total("nvpn_tun_to_mesh_bulk_dropped_channel_full")
      },
      seen: {
        udp_send_connected: event_seen("udp_send_connected"),
        udp_send_wildcard: event_seen("udp_send_wildcard"),
        udp_send_backpressure: event_seen("udp_send_backpressure"),
        udp_send_backpressure_sleep: event_seen("udp_send_backpressure_sleep"),
        udp_send_bulk_dropped: event_seen("udp_send_bulk_dropped"),
        connected_udp_installed: event_seen("connected_udp_installed"),
        connected_udp_activation_failed: event_seen("connected_udp_activation_failed"),
        connected_udp_peer_cap_skipped: event_seen("connected_udp_peer_cap_skipped"),
        connected_udp_fd_budget_skipped: event_seen("connected_udp_fd_budget_skipped"),
        connected_udp_kernel_dropped: event_seen("connected_udp_kernel_dropped"),
        connected_udp_peer_kernel_dropped: event_seen("connected_udp_peer_kernel_dropped"),
        connected_udp_drain_bulk_dropped: event_seen("connected_udp_drain_bulk_dropped"),
        encrypt_worker_queue_full: event_seen("encrypt_worker_queue_full"),
        encrypt_worker_priority_queue_full: event_seen("encrypt_worker_priority_queue_full"),
        encrypt_worker_bulk_queue_full: event_seen("encrypt_worker_bulk_queue_full"),
        encrypt_worker_bulk_dropped: event_seen("encrypt_worker_bulk_dropped"),
        decrypt_worker_queue_full: event_seen("decrypt_worker_queue_full"),
        decrypt_worker_bulk_dropped: event_seen("decrypt_worker_bulk_dropped"),
        decrypt_worker_register_full: event_seen("decrypt_worker_register_full"),
        decrypt_worker_priority_dropped: event_seen("decrypt_worker_priority_dropped"),
        decrypt_fallback_priority_gated: event_seen("decrypt_fallback_priority_gated"),
        decrypt_fsp_priority_queue_full_fallback: event_seen("decrypt_fsp_priority_queue_full_fallback"),
        decrypt_fsp_bulk_queue_full_fallback: event_seen("decrypt_fsp_bulk_queue_full_fallback"),
        decrypt_fsp_helper_window_fallback: event_seen("decrypt_fsp_helper_window_fallback"),
        decrypt_fsp_open_worker_window_fallback: event_seen("decrypt_fsp_open_worker_window_fallback"),
        decrypt_fsp_helper_queue_full_fallback: event_seen("decrypt_fsp_helper_queue_full_fallback"),
        decrypt_fsp_helper_completion_backlog_fallback: event_seen("decrypt_fsp_helper_completion_backlog_fallback"),
        decrypt_fsp_open_worker_completion_backlog_fallback: event_seen("decrypt_fsp_open_worker_completion_backlog_fallback"),
        decrypt_fsp_worker_replay_dropped: event_seen("decrypt_fsp_worker_replay_dropped"),
        fmp_aead_completion_aead_failed: event_seen("fmp_aead_completion_aead_failed"),
        fsp_aead_completion_aead_failed: event_seen("fsp_aead_completion_aead_failed"),
        fsp_aead_completion_epoch_mismatch: event_seen("fsp_aead_completion_epoch_mismatch"),
        decrypt_authenticated_session_priority_dropped: event_seen("decrypt_authenticated_session_priority_dropped"),
        decrypt_authenticated_session_bulk_dropped: event_seen("decrypt_authenticated_session_bulk_dropped"),
        decrypt_fallback_backlog_high: event_seen("decrypt_fallback_backlog_high"),
        decrypt_fallback_bulk_dropped: event_seen("decrypt_fallback_bulk_dropped"),
        decrypt_fallback_priority_dropped: event_seen("decrypt_fallback_priority_dropped"),
        pending_tun_destination_dropped: event_seen("pending_tun_destination_dropped"),
        pending_tun_packet_dropped: event_seen("pending_tun_packet_dropped"),
        pending_endpoint_destination_dropped: event_seen("pending_endpoint_destination_dropped"),
        pending_endpoint_packet_dropped: event_seen("pending_endpoint_packet_dropped"),
        endpoint_event_backlog_high: event_seen("endpoint_event_backlog_high"),
        endpoint_event_bulk_dropped: event_seen("endpoint_event_bulk_dropped"),
        transport_channel_backlog_high: event_seen("transport_channel_backlog_high"),
        transport_bulk_dropped: event_seen("transport_bulk_dropped"),
        nvpn_tun_to_mesh_bulk_dropped: event_seen("nvpn_tun_to_mesh_bulk_dropped"),
        nvpn_tun_to_mesh_bulk_dropped_batches: event_seen("nvpn_tun_to_mesh_bulk_dropped_batches"),
        nvpn_tun_to_mesh_bulk_dropped_packet_cap: event_seen("nvpn_tun_to_mesh_bulk_dropped_packet_cap"),
        nvpn_tun_to_mesh_bulk_dropped_channel_full: event_seen("nvpn_tun_to_mesh_bulk_dropped_channel_full")
      },
      queue_wait_ms: {
        endpoint_command_wait: wait_metric("endpoint_command_wait"),
        endpoint_priority_command_wait: wait_metric("endpoint_priority_command_wait"),
        endpoint_bulk_command_wait: wait_metric("endpoint_bulk_command_wait"),
        endpoint_event_wait: wait_metric("endpoint_event_wait"),
        endpoint_priority_event_wait: wait_metric("endpoint_priority_event_wait"),
        endpoint_bulk_event_wait: wait_metric("endpoint_bulk_event_wait"),
        fmp_worker_queue_wait: wait_metric("fmp_worker_queue_wait"),
        fmp_worker_priority_queue_wait: wait_metric("fmp_worker_priority_queue_wait"),
        fmp_worker_bulk_queue_wait: wait_metric("fmp_worker_bulk_queue_wait"),
        fmp_linux_bulk_container_queue_wait: wait_metric("fmp_linux_bulk_container_queue_wait"),
        fmp_linux_bulk_container_ready_wait: wait_metric("fmp_linux_bulk_container_ready_wait"),
        fmp_linux_bulk_container_first_slot_wait: wait_metric("fmp_linux_bulk_container_first_slot_wait"),
        fmp_linux_bulk_container_all_slots_wait: wait_metric("fmp_linux_bulk_container_all_slots_wait"),
        decrypt_worker_queue_wait: wait_metric("decrypt_worker_queue_wait"),
        decrypt_worker_priority_queue_wait: wait_metric("decrypt_worker_priority_queue_wait"),
        decrypt_worker_bulk_queue_wait: wait_metric("decrypt_worker_bulk_queue_wait"),
        decrypt_fallback_wait: wait_metric("decrypt_fallback_wait"),
        decrypt_fallback_priority_wait: wait_metric("decrypt_fallback_priority_wait"),
        decrypt_fallback_bulk_wait: wait_metric("decrypt_fallback_bulk_wait"),
        fsp_aead_worker_open_queue_wait: wait_metric("fsp_aead_worker_open_queue_wait"),
        fsp_aead_worker_open_completion_wait: wait_metric("fsp_aead_worker_open_completion_wait"),
        decrypt_authenticated_session_wait: wait_metric("decrypt_authenticated_session_wait"),
        decrypt_authenticated_session_priority_wait: wait_metric("decrypt_authenticated_session_priority_wait"),
        decrypt_authenticated_session_bulk_wait: wait_metric("decrypt_authenticated_session_bulk_wait"),
        decrypt_direct_session_commit_wait: wait_metric("decrypt_direct_session_commit_wait"),
        decrypt_direct_session_data_wait: wait_metric("decrypt_direct_session_data_wait"),
        decrypt_fsp_worker_queue_wait: wait_metric("decrypt_fsp_worker_queue_wait"),
        decrypt_fsp_worker_priority_queue_wait: wait_metric("decrypt_fsp_worker_priority_queue_wait"),
        decrypt_fsp_worker_bulk_queue_wait: wait_metric("decrypt_fsp_worker_bulk_queue_wait"),
        transport_queue_wait: wait_metric("transport_queue_wait"),
        transport_priority_queue_wait: wait_metric("transport_priority_queue_wait"),
        transport_bulk_queue_wait: wait_metric("transport_bulk_queue_wait"),
        transport_channel_wait: wait_metric("transport_channel_wait"),
        transport_priority_channel_wait: wait_metric("transport_priority_channel_wait"),
        transport_bulk_channel_wait: wait_metric("transport_bulk_channel_wait"),
        transport_rx_loop_wait: wait_metric("transport_rx_loop_wait"),
        transport_priority_rx_loop_wait: wait_metric("transport_priority_rx_loop_wait"),
        transport_bulk_rx_loop_wait: wait_metric("transport_bulk_rx_loop_wait"),
        connected_udp_drain_ring_wait: wait_metric("connected_udp_drain_ring_wait"),
        connected_udp_drain_priority_ring_wait: wait_metric("connected_udp_drain_priority_ring_wait"),
        connected_udp_drain_bulk_ring_wait: wait_metric("connected_udp_drain_bulk_ring_wait"),
        nvpn_tun_to_mesh_queue_wait: wait_metric("nvpn_tun_to_mesh_queue_wait")
      }
    }
  '
}

pipeline_sample_json() {
  local service="$1"
  local prefix="$2"
  local lines
  lines="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
    "grep '^\\[$prefix ' /root/.config/nvpn/daemon.log 2>/dev/null || true" | tr -d '\r')"
  pipeline_lines_to_json "$lines"
}

write_pipeline_failure_artifact() {
  local label="$1"
  local reason="$2"
  local details="$3"
  local sample="$4"
  local now safe_label safe_reason iteration_value path tmp

  if [[ "${SOAK_RUNNING:-0}" != "1" || -z "${OUTPUT_DIR:-}" || ! -d "$OUTPUT_DIR" ]]; then
    return 0
  fi

  now="$(date -u +%Y%m%dT%H%M%SZ)"
  safe_label="$(
    printf '%s' "$label" \
      | LC_ALL=C tr '[:upper:]' '[:lower:]' \
      | LC_ALL=C sed 's/[^a-z0-9_.-]/-/g; s/--*/-/g; s/^-//; s/-$//'
  )"
  safe_reason="$(
    printf '%s' "$reason" \
      | LC_ALL=C tr '[:upper:]' '[:lower:]' \
      | LC_ALL=C sed 's/[^a-z0-9_.-]/-/g; s/--*/-/g; s/^-//; s/-$//'
  )"
  [[ -n "$safe_label" ]] || safe_label="pipeline"
  [[ -n "$safe_reason" ]] || safe_reason="failure"
  iteration_value="${iteration:-unknown}"
  path="$OUTPUT_DIR/pipeline-failure-${iteration_value}-${safe_label}-${safe_reason}-${now}.json"
  tmp="${path}.tmp"

  if jq -c -n \
    --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg label "$label" \
    --arg reason "$reason" \
    --arg details "$details" \
    --arg iteration "$iteration_value" \
    --argjson sample "$sample" \
    '{
      ts: $ts,
      iteration: ($iteration | tonumber? // $iteration),
      label: $label,
      reason: $reason,
      details: $details,
      sample: $sample
    }' >"$tmp"; then
    mv "$tmp" "$path"
    echo "wrote pipeline failure artifact: $path" >&2
  else
    rm -f "$tmp"
  fi
}

assert_pipeline_fresh() {
  local label="$1"
  local sample="$2"
  local counter_var="$3"
  local stale_counter_var="$4"
  local count previous_count stale_count
  count="$(jq -r '.line_count // 0' <<<"$sample")"
  if [[ ! "$count" =~ ^[0-9]+$ ]]; then
    echo "nvpn+FIPS soak failed: $label pipeline line count is not numeric (count=$count)" >&2
    exit 1
  fi
  previous_count="${!counter_var}"
  if [[ -n "$previous_count" && "$count" -le "$previous_count" ]]; then
    stale_count="${!stale_counter_var}"
    stale_count=$((stale_count + 1))
    printf -v "$stale_counter_var" '%s' "$stale_count"
    if (( stale_count > MAX_CONSECUTIVE_PIPELINE_STALE_SAMPLES )); then
      echo "nvpn+FIPS soak failed: $label pipeline summaries did not advance for $stale_count consecutive sample(s) (count=$count previous=$previous_count)" >&2
      write_pipeline_failure_artifact "$label" "stale" "count=$count previous=$previous_count stale=$stale_count" "$sample"
      jq -c '{line_count, raw, load_raw, peak_wait_raw, recent}' <<<"$sample" >&2
      exit 1
    fi
    return 0
  fi
  if (( count > 0 )); then
    printf -v "$counter_var" '%s' "$count"
    printf -v "$stale_counter_var" '%s' 0
  fi
}

pipeline_hard_events() {
  local sample="$1"
  jq -r '
    . as $sample
    |
    [
      "connected_udp_activation_failed",
      "connected_udp_kernel_dropped",
      "connected_udp_peer_kernel_dropped",
      "connected_udp_drain_bulk_dropped",
      "encrypt_worker_queue_full",
      "encrypt_worker_bulk_dropped",
      "decrypt_worker_queue_full",
      "decrypt_worker_bulk_dropped",
      "decrypt_worker_register_full",
      "decrypt_worker_priority_dropped",
      "decrypt_fsp_helper_window_fallback",
      "decrypt_fsp_open_worker_window_fallback",
      "decrypt_fsp_helper_queue_full_fallback",
      "decrypt_fsp_helper_completion_backlog_fallback",
      "decrypt_fsp_open_worker_completion_backlog_fallback",
      "fmp_aead_completion_aead_failed",
      "fsp_aead_completion_aead_failed",
      "fsp_aead_completion_epoch_mismatch",
      "decrypt_fallback_bulk_dropped",
      "decrypt_fallback_priority_dropped",
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
    ]
    | map(select(. as $name | ($sample.seen[$name] // false)))
    | join(",")
  ' <<<"$sample"
}

pipeline_priority_hard_events() {
  local sample="$1"
  jq -r '
    . as $sample
    |
    [
      "encrypt_worker_priority_queue_full",
      "decrypt_worker_priority_dropped",
      "decrypt_fallback_priority_dropped",
      "decrypt_fsp_priority_queue_full_fallback",
      "decrypt_authenticated_session_priority_dropped"
    ]
    | map(select(. as $name | ($sample.seen[$name] // false)))
    | join(",")
  ' <<<"$sample"
}

pipeline_queue_wait_violations() {
  local sample="$1"
  jq -r \
    --argjson max_p95 "$MAX_PIPELINE_QUEUE_WAIT_P95_MS" \
    --argjson max_p99 "$MAX_PIPELINE_QUEUE_WAIT_P99_MS" '
    (.queue_wait_ms // {})
    | to_entries
    | map(
        select(
          ((.value.max_observed.p95_ms // 0) > $max_p95)
          or ((.value.max_observed.p99_ms // 0) > $max_p99)
        )
        | "\(.key):p95=\(.value.max_observed.p95_ms // "null")ms,p99=\(.value.max_observed.p99_ms // "null")ms"
      )
    | join(",")
  ' <<<"$sample"
}

pipeline_priority_queue_wait_violations() {
  local sample="$1"
  local threshold_ms="${MAX_PRIORITY_QUEUE_WAIT_MS:-0}"
  if ! awk -v threshold_ms="$threshold_ms" 'BEGIN { exit !((threshold_ms + 0) > 0) }'; then
    return 0
  fi
  jq -r \
    --argjson threshold "$threshold_ms" '
    (.queue_wait_ms // {}) as $waits
    | [
        "endpoint_priority_command_wait",
        "endpoint_priority_event_wait",
        "fmp_worker_priority_queue_wait",
        "decrypt_worker_priority_queue_wait",
        "decrypt_fallback_priority_wait",
        "decrypt_authenticated_session_priority_wait",
        "decrypt_fsp_worker_priority_queue_wait",
        "transport_priority_queue_wait",
        "transport_priority_channel_wait",
        "transport_priority_rx_loop_wait",
        "connected_udp_drain_priority_ring_wait"
      ]
    | map(
        . as $name
        | ($waits[$name].max_observed // null) as $observed
        | select($observed != null and (($observed.max_ms // 0) > $threshold))
        | "\($name):max=\($observed.max_ms // "null")ms,p99=\($observed.p99_ms // "null")ms"
      )
    | join(",")
  ' <<<"$sample"
}

assert_pipeline_ok() {
  local label="$1"
  local sample="$2"
  local hard_events priority_hard_events queue_waits priority_queue_waits
  if is_true "$FAIL_ON_PRIORITY_HARD_EVENTS"; then
    priority_hard_events="$(pipeline_priority_hard_events "$sample")"
    if [[ -n "$priority_hard_events" ]]; then
      echo "nvpn+FIPS soak failed: $label observed priority/control hard pipeline events: $priority_hard_events" >&2
      write_pipeline_failure_artifact "$label" "priority-hard-events" "$priority_hard_events" "$sample"
      jq -c '{raw, load_raw, peak_wait_raw, rates_per_sec, max_rates_per_sec, max_totals, seen}' <<<"$sample" >&2
      exit 1
    fi
  fi
  priority_queue_waits="$(pipeline_priority_queue_wait_violations "$sample")"
  if [[ -n "$priority_queue_waits" ]]; then
    echo "nvpn+FIPS soak failed: $label priority queue wait exceeded threshold: $priority_queue_waits" >&2
    write_pipeline_failure_artifact "$label" "priority-queue-wait" "$priority_queue_waits" "$sample"
    jq -c '{raw, load_raw, peak_wait_raw, queue_wait_ms}' <<<"$sample" >&2
    exit 1
  fi
  if [[ "$ALLOW_QUEUE_EVENTS" != "1" ]]; then
    hard_events="$(pipeline_hard_events "$sample")"
    if [[ -n "$hard_events" ]]; then
      echo "nvpn+FIPS soak failed: $label observed hard pipeline events: $hard_events" >&2
      write_pipeline_failure_artifact "$label" "hard-events" "$hard_events" "$sample"
      jq -c '{raw, load_raw, peak_wait_raw, rates_per_sec, max_rates_per_sec, max_totals, seen}' <<<"$sample" >&2
      exit 1
    fi
  fi
  if [[ "$ALLOW_QUEUE_WAIT" != "1" ]]; then
    queue_waits="$(pipeline_queue_wait_violations "$sample")"
    if [[ -n "$queue_waits" ]]; then
      echo "nvpn+FIPS soak failed: $label queue wait exceeded threshold: $queue_waits" >&2
      write_pipeline_failure_artifact "$label" "queue-wait" "$queue_waits" "$sample"
      jq -c '{raw, load_raw, peak_wait_raw, queue_wait_ms}' <<<"$sample" >&2
      exit 1
    fi
  fi
}

write_sample_json_file() {
  local label="$1"
  local json="$2"
  local path
  path="$(mktemp "$OUTPUT_DIR/sample-$iteration-$label.XXXXXX.json")"
  printf '%s\n' "$json" >"$path"
  printf '%s\n' "$path"
}

main() {
trap on_exit EXIT

validate_expected_fsp_owner_placement
validate_daemon_extra_env
mkdir -p "$OUTPUT_DIR"
SOAK_RUNNING=1
SAMPLES="$OUTPUT_DIR/samples.ndjson"
echo "writing soak artifacts to $OUTPUT_DIR"
NVPN_DOCKER_PIPELINE_TRACE=1 \
  NVPN_DOCKER_PIPELINE_INTERVAL_SECS="$PIPELINE_INTERVAL_SECS" \
  docker_bench_write_metadata fips-soak "$DURATION_SECS"

cleanup
if is_true "$SKIP_BUILD"; then
  "${COMPOSE[@]}" up -d --no-build node-a node-b >/dev/null
else
  BUILDKIT_PROGRESS=plain "${COMPOSE[@]}" build node-a node-b
  "${COMPOSE[@]}" up -d node-a node-b >/dev/null
fi
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" >/dev/null

ALICE_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"

stop_daemon node-a
stop_daemon node-b
start_daemon node-a
start_daemon node-b

STATUS_A="$(wait_for_mesh node-a "$BOB_NPUB" "10.203.0.11" "node-a")"
STATUS_B="$(wait_for_mesh node-b "$ALICE_NPUB" "10.203.0.10" "node-b")"
assert_peer_path "$STATUS_A" "$BOB_NPUB" "10.203.0.11" "node-a"
assert_peer_path "$STATUS_B" "$ALICE_NPUB" "10.203.0.10" "node-b"

start_iperf_server
docker_bench_start_cpu_stress
end_at=$((SECONDS + DURATION_SECS))
iteration=0
baseline_ping_ab_avg=""
baseline_ping_ba_avg=""
baseline_ping_ab_p95=""
baseline_ping_ba_p95=""
baseline_ping_ab_p99=""
baseline_ping_ba_p99=""
baseline_node_a_srtt=""
baseline_node_b_srtt=""
prev_node_a_bytes_sent=""
prev_node_a_bytes_recv=""
prev_node_b_bytes_sent=""
prev_node_b_bytes_recv=""
node_a_rekey_stuck_count=0
node_b_rekey_stuck_count=0
node_a_high_srtt_count=0
node_b_high_srtt_count=0
node_a_direct_probe_pending_count=0
node_b_direct_probe_pending_count=0
node_a_direct_probe_overdue_count=0
node_b_direct_probe_overdue_count=0
prev_fips_pipeline_a_count=""
prev_fips_pipeline_b_count=""
prev_nvpn_pipeline_a_count=""
prev_nvpn_pipeline_b_count=""
stale_fips_pipeline_a_count=0
stale_fips_pipeline_b_count=0
stale_nvpn_pipeline_a_count=0
stale_nvpn_pipeline_b_count=0
while (( SECONDS < end_at )); do
  iteration=$((iteration + 1))
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  iter_started_seconds="$SECONDS"

  STATUS_A="$(wait_for_peer_status node-a "$BOB_NPUB" "10.203.0.11" "node-a" 5)"
  STATUS_B="$(wait_for_peer_status node-b "$ALICE_NPUB" "10.203.0.10" "node-b" 5)"
  assert_peer_path "$STATUS_A" "$BOB_NPUB" "10.203.0.11" "node-a"
  assert_peer_path "$STATUS_B" "$ALICE_NPUB" "10.203.0.10" "node-b"

  read -r ping_ab_loss ping_ab_avg ping_ab_p95 ping_ab_p99 ping_ab_max <<<"$(ping_probe node-a "$BOB_TUNNEL_IP" "node-a to node-b")"
  read -r ping_ba_loss ping_ba_avg ping_ba_p95 ping_ba_p99 ping_ba_max <<<"$(ping_probe node-b "$ALICE_TUNNEL_IP" "node-b to node-a")"
  if [[ -z "$baseline_ping_ab_avg" ]]; then
    baseline_ping_ab_avg="$ping_ab_avg"
    baseline_ping_ab_p95="$ping_ab_p95"
    baseline_ping_ab_p99="$ping_ab_p99"
  else
    assert_float_drift_at_most "$ping_ab_avg" "$baseline_ping_ab_avg" "$MAX_PING_AVG_DRIFT_MS" "$MAX_PING_AVG_DRIFT_FACTOR" "node-a to node-b ping avg ms"
    assert_float_drift_at_most "$ping_ab_p95" "$baseline_ping_ab_p95" "$MAX_PING_P95_DRIFT_MS" "$MAX_PING_P95_DRIFT_FACTOR" "node-a to node-b ping p95 ms"
    assert_float_drift_at_most "$ping_ab_p99" "$baseline_ping_ab_p99" "$MAX_PING_P99_DRIFT_MS" "$MAX_PING_P99_DRIFT_FACTOR" "node-a to node-b ping p99 ms"
  fi
  if [[ -z "$baseline_ping_ba_avg" ]]; then
    baseline_ping_ba_avg="$ping_ba_avg"
    baseline_ping_ba_p95="$ping_ba_p95"
    baseline_ping_ba_p99="$ping_ba_p99"
  else
    assert_float_drift_at_most "$ping_ba_avg" "$baseline_ping_ba_avg" "$MAX_PING_AVG_DRIFT_MS" "$MAX_PING_AVG_DRIFT_FACTOR" "node-b to node-a ping avg ms"
    assert_float_drift_at_most "$ping_ba_p95" "$baseline_ping_ba_p95" "$MAX_PING_P95_DRIFT_MS" "$MAX_PING_P95_DRIFT_FACTOR" "node-b to node-a ping p95 ms"
    assert_float_drift_at_most "$ping_ba_p99" "$baseline_ping_ba_p99" "$MAX_PING_P99_DRIFT_MS" "$MAX_PING_P99_DRIFT_FACTOR" "node-b to node-a ping p99 ms"
  fi
  read -r iperf_forward_mbps iperf_forward_retrans <<<"$(iperf_probe forward)"
  read -r iperf_reverse_mbps iperf_reverse_retrans <<<"$(iperf_probe reverse -R)"

  STATUS_A="$(wait_for_peer_status node-a "$BOB_NPUB" "10.203.0.11" "node-a" 5)"
  STATUS_B="$(wait_for_peer_status node-b "$ALICE_NPUB" "10.203.0.10" "node-b" 5)"
  printf '%s\n' "$STATUS_A" >"$OUTPUT_DIR/status-node-a-$iteration.json"
  printf '%s\n' "$STATUS_B" >"$OUTPUT_DIR/status-node-b-$iteration.json"
  assert_peer_path "$STATUS_A" "$BOB_NPUB" "10.203.0.11" "node-a"
  assert_peer_path "$STATUS_B" "$ALICE_NPUB" "10.203.0.10" "node-b"
  node_a_transport="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_transport_addr)"
  node_b_transport="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_transport_addr)"
  node_a_srtt="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_srtt_ms)"
  node_b_srtt="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_srtt_ms)"
  node_a_bytes_sent="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_bytes_sent)"
  node_a_bytes_recv="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_bytes_recv)"
  node_b_bytes_sent="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_bytes_sent)"
  node_b_bytes_recv="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_bytes_recv)"
  node_a_last_mesh_seen_at="$(peer_field "$STATUS_A" "$BOB_NPUB" last_mesh_seen_at)"
  node_a_last_fips_seen_at="$(peer_field "$STATUS_A" "$BOB_NPUB" last_fips_seen_at)"
  node_a_last_fips_control_seen_at="$(peer_field "$STATUS_A" "$BOB_NPUB" last_fips_control_seen_at)"
  node_a_last_fips_data_seen_at="$(peer_field "$STATUS_A" "$BOB_NPUB" last_fips_data_seen_at)"
  node_a_last_handshake_at="$(peer_field "$STATUS_A" "$BOB_NPUB" last_handshake_at)"
  node_b_last_mesh_seen_at="$(peer_field "$STATUS_B" "$ALICE_NPUB" last_mesh_seen_at)"
  node_b_last_fips_seen_at="$(peer_field "$STATUS_B" "$ALICE_NPUB" last_fips_seen_at)"
  node_b_last_fips_control_seen_at="$(peer_field "$STATUS_B" "$ALICE_NPUB" last_fips_control_seen_at)"
  node_b_last_fips_data_seen_at="$(peer_field "$STATUS_B" "$ALICE_NPUB" last_fips_data_seen_at)"
  node_b_last_handshake_at="$(peer_field "$STATUS_B" "$ALICE_NPUB" last_handshake_at)"
  node_a_rekey_in_progress="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_rekey_in_progress)"
  node_a_rekey_draining="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_rekey_draining)"
  node_a_current_k_bit="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_current_k_bit)"
  node_a_direct_probe_pending="$(peer_field "$STATUS_A" "$BOB_NPUB" direct_probe_pending)"
  node_a_direct_probe_after_ms="$(peer_field "$STATUS_A" "$BOB_NPUB" direct_probe_after_ms)"
  node_a_direct_probe_retry_count="$(peer_field "$STATUS_A" "$BOB_NPUB" direct_probe_retry_count)"
  node_a_direct_probe_auto_reconnect="$(peer_field "$STATUS_A" "$BOB_NPUB" direct_probe_auto_reconnect)"
  node_a_direct_probe_expires_at_ms="$(peer_field "$STATUS_A" "$BOB_NPUB" direct_probe_expires_at_ms)"
  node_a_nostr_traversal_failures="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_nostr_traversal_failures)"
  node_a_nostr_traversal_in_cooldown="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_nostr_traversal_in_cooldown)"
  node_a_nostr_traversal_cooldown_until_ms="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_nostr_traversal_cooldown_until_ms)"
  node_a_nostr_traversal_last_skew_ms="$(peer_field "$STATUS_A" "$BOB_NPUB" fips_nostr_traversal_last_observed_skew_ms)"
  node_b_rekey_in_progress="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_rekey_in_progress)"
  node_b_rekey_draining="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_rekey_draining)"
  node_b_current_k_bit="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_current_k_bit)"
  node_b_direct_probe_pending="$(peer_field "$STATUS_B" "$ALICE_NPUB" direct_probe_pending)"
  node_b_direct_probe_after_ms="$(peer_field "$STATUS_B" "$ALICE_NPUB" direct_probe_after_ms)"
  node_b_direct_probe_retry_count="$(peer_field "$STATUS_B" "$ALICE_NPUB" direct_probe_retry_count)"
  node_b_direct_probe_auto_reconnect="$(peer_field "$STATUS_B" "$ALICE_NPUB" direct_probe_auto_reconnect)"
  node_b_direct_probe_expires_at_ms="$(peer_field "$STATUS_B" "$ALICE_NPUB" direct_probe_expires_at_ms)"
  node_b_nostr_traversal_failures="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_nostr_traversal_failures)"
  node_b_nostr_traversal_in_cooldown="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_nostr_traversal_in_cooldown)"
  node_b_nostr_traversal_cooldown_until_ms="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_nostr_traversal_cooldown_until_ms)"
  node_b_nostr_traversal_last_skew_ms="$(peer_field "$STATUS_B" "$ALICE_NPUB" fips_nostr_traversal_last_observed_skew_ms)"
  sample_now_ms="$(epoch_ms)"
  sample_now_secs="${sample_now_ms%000}"
  node_a_last_fips_seen_age_secs="$(fips_last_seen_age_secs "$node_a_last_fips_seen_at" "$sample_now_secs")"
  node_b_last_fips_seen_age_secs="$(fips_last_seen_age_secs "$node_b_last_fips_seen_at" "$sample_now_secs")"
  node_a_last_fips_control_seen_age_secs="$(fips_last_seen_age_secs "$node_a_last_fips_control_seen_at" "$sample_now_secs")"
  node_b_last_fips_control_seen_age_secs="$(fips_last_seen_age_secs "$node_b_last_fips_control_seen_at" "$sample_now_secs")"
  node_a_last_fips_data_seen_age_secs="$(fips_last_seen_age_secs "$node_a_last_fips_data_seen_at" "$sample_now_secs")"
  node_b_last_fips_data_seen_age_secs="$(fips_last_seen_age_secs "$node_b_last_fips_data_seen_at" "$sample_now_secs")"
  assert_fips_liveness_fresh "node-a FIPS" "$node_a_last_fips_seen_at" "$sample_now_secs"
  assert_fips_liveness_fresh "node-b FIPS" "$node_b_last_fips_seen_at" "$sample_now_secs"
  assert_fips_control_liveness_fresh "node-a FIPS" "$node_a_last_fips_control_seen_at" "$sample_now_secs"
  assert_fips_control_liveness_fresh "node-b FIPS" "$node_b_last_fips_control_seen_at" "$sample_now_secs"
  assert_fips_data_liveness_fresh "node-a FIPS" "$node_a_last_fips_data_seen_at" "$sample_now_secs"
  assert_fips_data_liveness_fresh "node-b FIPS" "$node_b_last_fips_data_seen_at" "$sample_now_secs"
  record_rekey_progress "node-a FIPS" "$node_a_rekey_in_progress" "$node_a_rekey_draining" node_a_rekey_stuck_count
  record_rekey_progress "node-b FIPS" "$node_b_rekey_in_progress" "$node_b_rekey_draining" node_b_rekey_stuck_count
  record_srtt_progress "node-a FIPS" "$node_a_srtt" node_a_high_srtt_count
  record_srtt_progress "node-b FIPS" "$node_b_srtt" node_b_high_srtt_count
  record_direct_probe_progress "node-a FIPS" "$node_a_direct_probe_pending" "$node_a_direct_probe_after_ms" "$sample_now_ms" node_a_direct_probe_pending_count node_a_direct_probe_overdue_count
  record_direct_probe_progress "node-b FIPS" "$node_b_direct_probe_pending" "$node_b_direct_probe_after_ms" "$sample_now_ms" node_b_direct_probe_pending_count node_b_direct_probe_overdue_count
  assert_counter_advanced "$node_a_bytes_sent" "$prev_node_a_bytes_sent" "node-a FIPS bytes sent"
  assert_counter_advanced "$node_a_bytes_recv" "$prev_node_a_bytes_recv" "node-a FIPS bytes recv"
  assert_counter_advanced "$node_b_bytes_sent" "$prev_node_b_bytes_sent" "node-b FIPS bytes sent"
  assert_counter_advanced "$node_b_bytes_recv" "$prev_node_b_bytes_recv" "node-b FIPS bytes recv"
  prev_node_a_bytes_sent="$node_a_bytes_sent"
  prev_node_a_bytes_recv="$node_a_bytes_recv"
  prev_node_b_bytes_sent="$node_b_bytes_sent"
  prev_node_b_bytes_recv="$node_b_bytes_recv"
  if is_number "$node_a_srtt" && (( node_a_high_srtt_count == 0 )); then
    if [[ -z "$baseline_node_a_srtt" ]]; then
      baseline_node_a_srtt="$node_a_srtt"
    else
      assert_float_drift_at_most "$node_a_srtt" "$baseline_node_a_srtt" "$MAX_SRTT_DRIFT_MS" "$MAX_SRTT_DRIFT_FACTOR" "node-a FIPS SRTT ms"
    fi
  fi
  if is_number "$node_b_srtt" && (( node_b_high_srtt_count == 0 )); then
    if [[ -z "$baseline_node_b_srtt" ]]; then
      baseline_node_b_srtt="$node_b_srtt"
    else
      assert_float_drift_at_most "$node_b_srtt" "$baseline_node_b_srtt" "$MAX_SRTT_DRIFT_MS" "$MAX_SRTT_DRIFT_FACTOR" "node-b FIPS SRTT ms"
    fi
  fi

  cpu_a="$(daemon_cpu_percent node-a)"
  cpu_b="$(daemon_cpu_percent node-b)"
  assert_cpu_ok node-a "$cpu_a"
  assert_cpu_ok node-b "$cpu_b"
  fips_pipeline_a="$(pipeline_sample_json node-a pipe)"
  fips_pipeline_b="$(pipeline_sample_json node-b pipe)"
  nvpn_pipeline_a="$(pipeline_sample_json node-a nvpn-pipe)"
  nvpn_pipeline_b="$(pipeline_sample_json node-b nvpn-pipe)"
  assert_pipeline_fresh "node-a FIPS" "$fips_pipeline_a" prev_fips_pipeline_a_count stale_fips_pipeline_a_count
  assert_pipeline_fresh "node-b FIPS" "$fips_pipeline_b" prev_fips_pipeline_b_count stale_fips_pipeline_b_count
  assert_pipeline_fresh "node-a nvpn" "$nvpn_pipeline_a" prev_nvpn_pipeline_a_count stale_nvpn_pipeline_a_count
  assert_pipeline_fresh "node-b nvpn" "$nvpn_pipeline_b" prev_nvpn_pipeline_b_count stale_nvpn_pipeline_b_count
  assert_expected_fsp_owner_placement_any_sample \
    "node-a FIPS" "$fips_pipeline_a" \
    "node-b FIPS" "$fips_pipeline_b"
  assert_pipeline_ok "node-a FIPS" "$fips_pipeline_a"
  assert_pipeline_ok "node-b FIPS" "$fips_pipeline_b"
  assert_pipeline_ok "node-a nvpn" "$nvpn_pipeline_a"
  assert_pipeline_ok "node-b nvpn" "$nvpn_pipeline_b"

  sample_pipeline_files=()
  fips_pipeline_a_file="$(write_sample_json_file node-a-fips "$fips_pipeline_a")"
  fips_pipeline_b_file="$(write_sample_json_file node-b-fips "$fips_pipeline_b")"
  nvpn_pipeline_a_file="$(write_sample_json_file node-a-nvpn "$nvpn_pipeline_a")"
  nvpn_pipeline_b_file="$(write_sample_json_file node-b-nvpn "$nvpn_pipeline_b")"
  sample_pipeline_files=(
    "$fips_pipeline_a_file"
    "$fips_pipeline_b_file"
    "$nvpn_pipeline_a_file"
    "$nvpn_pipeline_b_file"
  )

  if ! jq -nc \
    --arg ts "$started_at" \
    --argjson iteration "$iteration" \
    --arg alice_tunnel_ip "$ALICE_TUNNEL_IP" \
    --arg bob_tunnel_ip "$BOB_TUNNEL_IP" \
    --arg node_a_transport "$node_a_transport" \
    --arg node_b_transport "$node_b_transport" \
    --arg node_a_srtt "$node_a_srtt" \
    --arg node_b_srtt "$node_b_srtt" \
    --arg node_a_bytes_sent "$node_a_bytes_sent" \
    --arg node_a_bytes_recv "$node_a_bytes_recv" \
    --arg node_b_bytes_sent "$node_b_bytes_sent" \
    --arg node_b_bytes_recv "$node_b_bytes_recv" \
    --arg node_a_last_mesh_seen_at "$node_a_last_mesh_seen_at" \
    --arg node_a_last_fips_seen_at "$node_a_last_fips_seen_at" \
    --arg node_a_last_fips_seen_age_secs "$node_a_last_fips_seen_age_secs" \
    --arg node_a_last_fips_control_seen_at "$node_a_last_fips_control_seen_at" \
    --arg node_a_last_fips_control_seen_age_secs "$node_a_last_fips_control_seen_age_secs" \
    --arg node_a_last_fips_data_seen_at "$node_a_last_fips_data_seen_at" \
    --arg node_a_last_fips_data_seen_age_secs "$node_a_last_fips_data_seen_age_secs" \
    --arg node_a_last_handshake_at "$node_a_last_handshake_at" \
    --arg node_b_last_mesh_seen_at "$node_b_last_mesh_seen_at" \
    --arg node_b_last_fips_seen_at "$node_b_last_fips_seen_at" \
    --arg node_b_last_fips_seen_age_secs "$node_b_last_fips_seen_age_secs" \
    --arg node_b_last_fips_control_seen_at "$node_b_last_fips_control_seen_at" \
    --arg node_b_last_fips_control_seen_age_secs "$node_b_last_fips_control_seen_age_secs" \
    --arg node_b_last_fips_data_seen_at "$node_b_last_fips_data_seen_at" \
    --arg node_b_last_fips_data_seen_age_secs "$node_b_last_fips_data_seen_age_secs" \
    --arg node_b_last_handshake_at "$node_b_last_handshake_at" \
    --arg node_a_rekey_in_progress "$node_a_rekey_in_progress" \
    --arg node_a_rekey_draining "$node_a_rekey_draining" \
    --arg node_a_current_k_bit "$node_a_current_k_bit" \
    --argjson node_a_rekey_stuck_count "$node_a_rekey_stuck_count" \
    --arg node_a_direct_probe_pending "$node_a_direct_probe_pending" \
    --arg node_a_direct_probe_after_ms "$node_a_direct_probe_after_ms" \
    --arg node_a_direct_probe_retry_count "$node_a_direct_probe_retry_count" \
    --arg node_a_direct_probe_auto_reconnect "$node_a_direct_probe_auto_reconnect" \
    --arg node_a_direct_probe_expires_at_ms "$node_a_direct_probe_expires_at_ms" \
    --argjson node_a_direct_probe_pending_count "$node_a_direct_probe_pending_count" \
    --argjson node_a_direct_probe_overdue_count "$node_a_direct_probe_overdue_count" \
    --arg node_a_nostr_traversal_failures "$node_a_nostr_traversal_failures" \
    --arg node_a_nostr_traversal_in_cooldown "$node_a_nostr_traversal_in_cooldown" \
    --arg node_a_nostr_traversal_cooldown_until_ms "$node_a_nostr_traversal_cooldown_until_ms" \
    --arg node_a_nostr_traversal_last_skew_ms "$node_a_nostr_traversal_last_skew_ms" \
    --arg node_b_rekey_in_progress "$node_b_rekey_in_progress" \
    --arg node_b_rekey_draining "$node_b_rekey_draining" \
    --arg node_b_current_k_bit "$node_b_current_k_bit" \
    --argjson node_b_rekey_stuck_count "$node_b_rekey_stuck_count" \
    --arg node_b_direct_probe_pending "$node_b_direct_probe_pending" \
    --arg node_b_direct_probe_after_ms "$node_b_direct_probe_after_ms" \
    --arg node_b_direct_probe_retry_count "$node_b_direct_probe_retry_count" \
    --arg node_b_direct_probe_auto_reconnect "$node_b_direct_probe_auto_reconnect" \
    --arg node_b_direct_probe_expires_at_ms "$node_b_direct_probe_expires_at_ms" \
    --argjson node_b_direct_probe_pending_count "$node_b_direct_probe_pending_count" \
    --argjson node_b_direct_probe_overdue_count "$node_b_direct_probe_overdue_count" \
    --arg node_b_nostr_traversal_failures "$node_b_nostr_traversal_failures" \
    --arg node_b_nostr_traversal_in_cooldown "$node_b_nostr_traversal_in_cooldown" \
    --arg node_b_nostr_traversal_cooldown_until_ms "$node_b_nostr_traversal_cooldown_until_ms" \
    --arg node_b_nostr_traversal_last_skew_ms "$node_b_nostr_traversal_last_skew_ms" \
    --arg ping_ab_loss "$ping_ab_loss" \
    --arg ping_ab_avg "$ping_ab_avg" \
    --arg ping_ab_p95 "$ping_ab_p95" \
    --arg ping_ab_p99 "$ping_ab_p99" \
    --arg ping_ab_max "$ping_ab_max" \
    --arg ping_ba_loss "$ping_ba_loss" \
    --arg ping_ba_avg "$ping_ba_avg" \
    --arg ping_ba_p95 "$ping_ba_p95" \
    --arg ping_ba_p99 "$ping_ba_p99" \
    --arg ping_ba_max "$ping_ba_max" \
    --arg iperf_forward_mbps "$iperf_forward_mbps" \
    --arg iperf_forward_retrans "$iperf_forward_retrans" \
    --arg iperf_reverse_mbps "$iperf_reverse_mbps" \
    --arg iperf_reverse_retrans "$iperf_reverse_retrans" \
    --arg cpu_a "$cpu_a" \
    --arg cpu_b "$cpu_b" \
    --slurpfile fips_pipeline_a "$fips_pipeline_a_file" \
    --slurpfile fips_pipeline_b "$fips_pipeline_b_file" \
    --slurpfile nvpn_pipeline_a "$nvpn_pipeline_a_file" \
    --slurpfile nvpn_pipeline_b "$nvpn_pipeline_b_file" \
    'def num($v): if $v == "" or $v == "null" then null else ($v | tonumber) end;
    def bool($v):
      if $v == "" or $v == "null" then null
      elif $v == "true" then true
      elif $v == "false" then false
      else null
      end;
    {
      ts: $ts,
      iteration: $iteration,
      tunnel: { node_a: $alice_tunnel_ip, node_b: $bob_tunnel_ip },
      path: { node_a_transport: $node_a_transport, node_b_transport: $node_b_transport },
      fips: {
        node_a_srtt_ms: ($node_a_srtt | tonumber?),
        node_b_srtt_ms: ($node_b_srtt | tonumber?),
        node_a_bytes_sent: ($node_a_bytes_sent | tonumber?),
        node_a_bytes_recv: ($node_a_bytes_recv | tonumber?),
        node_b_bytes_sent: ($node_b_bytes_sent | tonumber?),
        node_b_bytes_recv: ($node_b_bytes_recv | tonumber?),
        node_a_last_mesh_seen_at: num($node_a_last_mesh_seen_at),
        node_a_last_fips_seen_at: num($node_a_last_fips_seen_at),
        node_a_last_fips_seen_age_secs: num($node_a_last_fips_seen_age_secs),
        node_a_last_fips_control_seen_at: num($node_a_last_fips_control_seen_at),
        node_a_last_fips_control_seen_age_secs: num($node_a_last_fips_control_seen_age_secs),
        node_a_last_fips_data_seen_at: num($node_a_last_fips_data_seen_at),
        node_a_last_fips_data_seen_age_secs: num($node_a_last_fips_data_seen_age_secs),
        node_a_last_handshake_at: num($node_a_last_handshake_at),
        node_b_last_mesh_seen_at: num($node_b_last_mesh_seen_at),
        node_b_last_fips_seen_at: num($node_b_last_fips_seen_at),
        node_b_last_fips_seen_age_secs: num($node_b_last_fips_seen_age_secs),
        node_b_last_fips_control_seen_at: num($node_b_last_fips_control_seen_at),
        node_b_last_fips_control_seen_age_secs: num($node_b_last_fips_control_seen_age_secs),
        node_b_last_fips_data_seen_at: num($node_b_last_fips_data_seen_at),
        node_b_last_fips_data_seen_age_secs: num($node_b_last_fips_data_seen_age_secs),
        node_b_last_handshake_at: num($node_b_last_handshake_at),
        node_a_rekey_in_progress: bool($node_a_rekey_in_progress),
        node_a_rekey_draining: bool($node_a_rekey_draining),
        node_a_current_k_bit: bool($node_a_current_k_bit),
        node_a_rekey_stuck_count: $node_a_rekey_stuck_count,
        node_a_direct_probe_pending: bool($node_a_direct_probe_pending),
        node_a_direct_probe_after_ms: num($node_a_direct_probe_after_ms),
        node_a_direct_probe_retry_count: num($node_a_direct_probe_retry_count),
        node_a_direct_probe_auto_reconnect: bool($node_a_direct_probe_auto_reconnect),
        node_a_direct_probe_expires_at_ms: num($node_a_direct_probe_expires_at_ms),
        node_a_direct_probe_pending_count: $node_a_direct_probe_pending_count,
        node_a_direct_probe_overdue_count: $node_a_direct_probe_overdue_count,
        node_a_nostr_traversal_failures: num($node_a_nostr_traversal_failures),
        node_a_nostr_traversal_in_cooldown: bool($node_a_nostr_traversal_in_cooldown),
        node_a_nostr_traversal_cooldown_until_ms: num($node_a_nostr_traversal_cooldown_until_ms),
        node_a_nostr_traversal_last_skew_ms: num($node_a_nostr_traversal_last_skew_ms),
        node_b_rekey_in_progress: bool($node_b_rekey_in_progress),
        node_b_rekey_draining: bool($node_b_rekey_draining),
        node_b_current_k_bit: bool($node_b_current_k_bit),
        node_b_rekey_stuck_count: $node_b_rekey_stuck_count,
        node_b_direct_probe_pending: bool($node_b_direct_probe_pending),
        node_b_direct_probe_after_ms: num($node_b_direct_probe_after_ms),
        node_b_direct_probe_retry_count: num($node_b_direct_probe_retry_count),
        node_b_direct_probe_auto_reconnect: bool($node_b_direct_probe_auto_reconnect),
        node_b_direct_probe_expires_at_ms: num($node_b_direct_probe_expires_at_ms),
        node_b_direct_probe_pending_count: $node_b_direct_probe_pending_count,
        node_b_direct_probe_overdue_count: $node_b_direct_probe_overdue_count,
        node_b_nostr_traversal_failures: num($node_b_nostr_traversal_failures),
        node_b_nostr_traversal_in_cooldown: bool($node_b_nostr_traversal_in_cooldown),
        node_b_nostr_traversal_cooldown_until_ms: num($node_b_nostr_traversal_cooldown_until_ms),
        node_b_nostr_traversal_last_skew_ms: num($node_b_nostr_traversal_last_skew_ms)
      },
      ping: {
        a_to_b: {
          loss_percent: ($ping_ab_loss | tonumber),
          avg_ms: ($ping_ab_avg | tonumber),
          p95_ms: ($ping_ab_p95 | tonumber),
          p99_ms: ($ping_ab_p99 | tonumber),
          max_ms: ($ping_ab_max | tonumber)
        },
        b_to_a: {
          loss_percent: ($ping_ba_loss | tonumber),
          avg_ms: ($ping_ba_avg | tonumber),
          p95_ms: ($ping_ba_p95 | tonumber),
          p99_ms: ($ping_ba_p99 | tonumber),
          max_ms: ($ping_ba_max | tonumber)
        }
      },
      iperf: {
        forward_mbps: ($iperf_forward_mbps | tonumber),
        forward_retrans: ($iperf_forward_retrans | tonumber),
        reverse_mbps: ($iperf_reverse_mbps | tonumber),
        reverse_retrans: ($iperf_reverse_retrans | tonumber)
      },
      cpu: { node_a_percent: ($cpu_a | tonumber), node_b_percent: ($cpu_b | tonumber) },
      pipeline: {
        node_a: { fips: $fips_pipeline_a[0], nvpn: $nvpn_pipeline_a[0] },
        node_b: { fips: $fips_pipeline_b[0], nvpn: $nvpn_pipeline_b[0] }
      }
    }' | tee -a "$SAMPLES"; then
    rm -f "${sample_pipeline_files[@]}"
    exit 1
  fi
  rm -f "${sample_pipeline_files[@]}"

  elapsed=$((SECONDS - iter_started_seconds))
  sleep_for=$((INTERVAL_SECS - elapsed))
  if (( sleep_for > 0 && SECONDS + sleep_for < end_at )); then
    sleep "$sleep_for"
  fi
done

echo "nvpn+FIPS soak passed: wrote samples to $SAMPLES"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
