#!/usr/bin/env bash
# Shared parsers/summary writers for simple Docker VPN benchmark scripts.

docker_bench_init_summary() {
  mkdir -p "$RAW_DIR"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    backend threads duration_secs \
    tcp_single_mbps tcp_single_retrans \
    tcp_4_mbps tcp_4_retrans \
    tcp_8_mbps tcp_8_retrans \
    udp_200_mbps udp_200_loss_pct \
    udp_1000_mbps udp_1000_loss_pct \
    ping_loss_pct ping_avg_ms raw_dir >"$SUMMARY_TSV"
}

docker_bench_tsv_field() {
  local value="$1"
  value="${value//$'\t'/ }"
  value="${value//$'\n'/ }"
  printf '%s' "$value"
}

docker_bench_bool_enabled() {
  case "${1:-}" in
    1 | true | TRUE | yes | YES | on | ON) return 0 ;;
    *) return 1 ;;
  esac
}

docker_bench_env_bool_assignment_enabled() {
  local name="$1"
  local token value
  for token in ${2:-}; do
    case "$token" in
      "$name"=*)
        value="${token#*=}"
        if docker_bench_bool_enabled "$value"; then
          return 0
        fi
        ;;
    esac
  done
  return 1
}

docker_bench_direct_fmp_forced_enabled() {
  docker_bench_env_bool_assignment_enabled \
    FIPS_DIRECT_ENDPOINT_FMP_ONLY \
    "${NVPN_DOCKER_EXTRA_ENV:-}"
}

docker_bench_cpu_stress_enabled() {
  docker_bench_bool_enabled "${NVPN_DOCKER_CPU_STRESS:-0}"
}

docker_bench_cpu_stress_sides() {
  printf '%s\n' "${NVPN_DOCKER_CPU_STRESS_SIDES:-both}"
}

docker_bench_cpu_stress_side_enabled() {
  local side="$1"
  local sides
  sides="$(docker_bench_cpu_stress_sides)"
  case ",$sides," in
    *,all,* | *,both,*) return 0 ;;
  esac
  if [[ "$side" == "local" ]]; then
    case ",$sides," in
      *,local,* | *,client,* | *,node-a,*) return 0 ;;
    esac
  else
    case ",$sides," in
      *,remote,* | *,server,* | *,node-b,*) return 0 ;;
    esac
  fi
  return 1
}

docker_bench_cpu_stress_workers() {
  local side="$1"
  local default_workers="${NVPN_DOCKER_CPU_STRESS_WORKERS:-1}"
  local value
  case "$side" in
    local) value="${NVPN_DOCKER_CPU_STRESS_LOCAL_WORKERS:-$default_workers}" ;;
    remote) value="${NVPN_DOCKER_CPU_STRESS_REMOTE_WORKERS:-$default_workers}" ;;
    *) value="$default_workers" ;;
  esac
  awk -v value="$value" 'BEGIN {
    if (value ~ /^[0-9]+$/ && value + 0 > 0) {
      print value + 0
    } else {
      print 0
    }
  }'
}

docker_bench_git_head() {
  local dir="$1"
  command -v git >/dev/null 2>&1 || return 0
  git -C "$dir" rev-parse --short=12 HEAD 2>/dev/null || true
}

docker_bench_git_dirty() {
  local dir="$1"
  command -v git >/dev/null 2>&1 || return 0
  git -C "$dir" rev-parse --is-inside-work-tree >/dev/null 2>&1 || return 0
  if [[ -n "$(git -C "$dir" status --porcelain 2>/dev/null)" ]]; then
    printf 'true\n'
  else
    printf 'false\n'
  fi
}

docker_bench_start_cpu_stress_for_service() {
  local service="$1"
  local workers="$2"
  (( workers > 0 )) || return 0
  "${COMPOSE[@]}" exec -T "$service" sh -lc '
    if [ -f /tmp/nvpn-docker-cpu-stress.pids ]; then
      while IFS= read -r pid; do
        kill "$pid" 2>/dev/null || true
      done < /tmp/nvpn-docker-cpu-stress.pids
      rm -f /tmp/nvpn-docker-cpu-stress.pids
    fi
  ' >/dev/null 2>&1 || true
  "${COMPOSE[@]}" exec -d "$service" sh -lc "
    rm -f /tmp/nvpn-docker-cpu-stress.pids
    i=0
    while [ \"\$i\" -lt $workers ]; do
      (while :; do :; done) &
      echo \$! >> /tmp/nvpn-docker-cpu-stress.pids
      i=\$((i + 1))
    done
    wait
  " >/dev/null
}

docker_bench_start_cpu_stress() {
  docker_bench_cpu_stress_enabled || return 0
  local local_workers remote_workers
  local_workers=0
  remote_workers=0
  if docker_bench_cpu_stress_side_enabled local; then
    local_workers="$(docker_bench_cpu_stress_workers local)"
    docker_bench_start_cpu_stress_for_service node-a "$local_workers"
  fi
  if docker_bench_cpu_stress_side_enabled remote; then
    remote_workers="$(docker_bench_cpu_stress_workers remote)"
    docker_bench_start_cpu_stress_for_service node-b "$remote_workers"
  fi
  printf 'docker CPU stress enabled: sides=%s local_workers=%s remote_workers=%s\n' \
    "$(docker_bench_cpu_stress_sides)" "$local_workers" "$remote_workers"
}

docker_bench_stop_cpu_stress() {
  local service
  for service in node-a node-b; do
    "${COMPOSE[@]}" exec -T "$service" sh -lc '
      if [ -f /tmp/nvpn-docker-cpu-stress.pids ]; then
        while IFS= read -r pid; do
          kill "$pid" 2>/dev/null || true
        done < /tmp/nvpn-docker-cpu-stress.pids
        rm -f /tmp/nvpn-docker-cpu-stress.pids
      fi
    ' >/dev/null 2>&1 || true
  done
}

docker_bench_write_metadata() {
  local backend="$1"
  local duration="$2"
  local metadata_path="${OUTPUT_DIR}/metadata.json"
  local stress_enabled=0
  local local_workers=0
  local remote_workers=0
  local pipeline_trace_enabled=0
  local pipeline_trace_interval_secs=""
  local iperf_interval_secs="${NVPN_DOCKER_IPERF_INTERVAL_SECS:-0}"
  local iperf_timeout_secs="${NVPN_DOCKER_IPERF_TIMEOUT_SECS:-}"
  local require_no_direct_fmp=0
  local direct_fmp_forced=0
  local patch_local_fips_enabled=0
  local nvpn_git_head=""
  local nvpn_git_dirty=""
  local fips_git_head=""
  local fips_git_dirty=""
  if docker_bench_cpu_stress_enabled; then
    stress_enabled=1
    if docker_bench_cpu_stress_side_enabled local; then
      local_workers="$(docker_bench_cpu_stress_workers local)"
    fi
    if docker_bench_cpu_stress_side_enabled remote; then
      remote_workers="$(docker_bench_cpu_stress_workers remote)"
    fi
  fi
  if docker_bench_bool_enabled "${NVPN_DOCKER_PIPELINE_TRACE:-0}"; then
    pipeline_trace_enabled=1
    pipeline_trace_interval_secs="${NVPN_DOCKER_PIPELINE_INTERVAL_SECS:-5}"
  fi
  if docker_bench_bool_enabled "${NVPN_PATCH_LOCAL_FIPS:-0}"; then
    patch_local_fips_enabled=1
  fi
  if docker_bench_bool_enabled "${NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP:-0}"; then
    require_no_direct_fmp=1
  fi
  if docker_bench_direct_fmp_forced_enabled; then
    direct_fmp_forced=1
  fi
  if [[ -n "${ROOT_DIR:-}" ]]; then
    nvpn_git_head="$(docker_bench_git_head "$ROOT_DIR")"
    nvpn_git_dirty="$(docker_bench_git_dirty "$ROOT_DIR")"
  fi
  if [[ -n "${NVPN_FIPS_REPO_PATH:-}" ]]; then
    fips_git_head="$(docker_bench_git_head "$NVPN_FIPS_REPO_PATH")"
    fips_git_dirty="$(docker_bench_git_dirty "$NVPN_FIPS_REPO_PATH")"
  fi
  jq -n \
    --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg backend "$backend" \
    --arg duration_secs "$duration" \
    --arg stress_enabled "$stress_enabled" \
    --arg stress_sides "$(docker_bench_cpu_stress_sides)" \
    --arg local_workers "$local_workers" \
    --arg remote_workers "$remote_workers" \
    --arg pipeline_trace_enabled "$pipeline_trace_enabled" \
    --arg pipeline_trace_interval_secs "$pipeline_trace_interval_secs" \
    --arg iperf_interval_secs "$iperf_interval_secs" \
    --arg iperf_timeout_secs "$iperf_timeout_secs" \
    --arg extra_connect_env "${NVPN_DOCKER_EXTRA_ENV:-}" \
    --arg require_no_direct_fmp "$require_no_direct_fmp" \
    --arg direct_fmp_forced "$direct_fmp_forced" \
    --arg patch_local_fips_enabled "$patch_local_fips_enabled" \
    --arg nvpn_git_head "$nvpn_git_head" \
    --arg nvpn_git_dirty "$nvpn_git_dirty" \
    --arg fips_git_head "$fips_git_head" \
    --arg fips_git_dirty "$fips_git_dirty" \
    --arg guard_min_tcp_mbps "${NVPN_DOCKER_MIN_TCP_MBPS:-}" \
    --arg guard_min_tcp_single_mbps "${NVPN_DOCKER_MIN_TCP_SINGLE_MBPS:-}" \
    --arg guard_min_tcp_4_mbps "${NVPN_DOCKER_MIN_TCP_4_MBPS:-}" \
    --arg guard_min_tcp_8_mbps "${NVPN_DOCKER_MIN_TCP_8_MBPS:-}" \
    --arg guard_max_tcp_retrans "${NVPN_DOCKER_MAX_TCP_RETRANS:-}" \
    --arg guard_max_tcp_single_retrans "${NVPN_DOCKER_MAX_TCP_SINGLE_RETRANS:-}" \
    --arg guard_max_tcp_4_retrans "${NVPN_DOCKER_MAX_TCP_4_RETRANS:-}" \
    --arg guard_max_tcp_8_retrans "${NVPN_DOCKER_MAX_TCP_8_RETRANS:-}" \
    --arg guard_max_udp_loss_pct "${NVPN_DOCKER_MAX_UDP_LOSS_PCT:-}" \
    --arg guard_max_udp200_loss_pct "${NVPN_DOCKER_MAX_UDP200_LOSS_PCT:-}" \
    --arg guard_max_udp1000_loss_pct "${NVPN_DOCKER_MAX_UDP1000_LOSS_PCT:-}" \
    --arg guard_max_ping_loss_pct "${NVPN_DOCKER_MAX_PING_LOSS_PCT:-}" \
    'def bool_or_null($v):
       if $v == "" then null
       elif $v == "true" then true
       elif $v == "false" then false
       else null
       end;
     def string_or_null($v):
       if $v == "" then null else $v end;
     {
      generated_at: $generated_at,
      backend: $backend,
      duration_secs: ($duration_secs | tonumber),
      run_env: {
        extra_connect_env: $extra_connect_env,
        require_no_direct_fmp: ($require_no_direct_fmp == "1"),
        direct_fmp_forced: ($direct_fmp_forced == "1")
      },
      cpu_stress: {
        enabled: ($stress_enabled == "1"),
        sides: $stress_sides,
        local_workers: ($local_workers | tonumber),
        remote_workers: ($remote_workers | tonumber)
      },
      pipeline_trace: {
        enabled: ($pipeline_trace_enabled == "1"),
        interval_secs: (
          if $pipeline_trace_interval_secs == "" then null
          else ($pipeline_trace_interval_secs | tonumber)
          end
        )
      },
      iperf: {
        interval_secs: ($iperf_interval_secs | tonumber),
        timeout_secs: (
          if $iperf_timeout_secs == "" then null
          else ($iperf_timeout_secs | tonumber)
          end
        )
      },
      guard_thresholds: {
        min_tcp_mbps: string_or_null($guard_min_tcp_mbps),
        min_tcp_single_mbps: string_or_null($guard_min_tcp_single_mbps),
        min_tcp_4_mbps: string_or_null($guard_min_tcp_4_mbps),
        min_tcp_8_mbps: string_or_null($guard_min_tcp_8_mbps),
        max_tcp_retrans: string_or_null($guard_max_tcp_retrans),
        max_tcp_single_retrans: string_or_null($guard_max_tcp_single_retrans),
        max_tcp_4_retrans: string_or_null($guard_max_tcp_4_retrans),
        max_tcp_8_retrans: string_or_null($guard_max_tcp_8_retrans),
        max_udp_loss_pct: string_or_null($guard_max_udp_loss_pct),
        max_udp200_loss_pct: string_or_null($guard_max_udp200_loss_pct),
        max_udp1000_loss_pct: string_or_null($guard_max_udp1000_loss_pct),
        max_ping_loss_pct: string_or_null($guard_max_ping_loss_pct)
      },
      source: {
        nvpn: {
          git_head: (if $nvpn_git_head == "" then null else $nvpn_git_head end),
          dirty: bool_or_null($nvpn_git_dirty)
        },
        local_fips_patch: {
          enabled: ($patch_local_fips_enabled == "1"),
          git_head: (if $fips_git_head == "" then null else $fips_git_head end),
          dirty: bool_or_null($fips_git_dirty)
        }
      }
    }' >"$metadata_path"
}

docker_bench_json_number() {
  local filter="$1"
  local json_path="$2"
  jq -r "$filter | if . == null then \"\" else ((. * 1000 | round) / 1000 | tostring) end" "$json_path"
}

docker_bench_iperf_mbps() {
  docker_bench_json_number '((.end.sum_received.bits_per_second // .end.sum.bits_per_second // 0) / 1000000)' "$1"
}

docker_bench_iperf_retrans() {
  jq -r '(.end.sum_sent.retransmits // .end.sum.retransmits // 0)' "$1"
}

docker_bench_iperf_loss_pct() {
  docker_bench_json_number '(.end.sum.lost_percent // .end.sum_received.lost_percent // 0)' "$1"
}

docker_bench_parse_ping_loss_avg() {
  awk '
    /packets transmitted/ {
      for (i = 1; i <= NF; i++) {
        if ($i == "packet" && $(i + 1) ~ /^loss/) {
          loss = $(i - 1)
          sub(/%$/, "", loss)
        }
      }
    }
    /^(rtt|round-trip)/ {
      split($0, parts, "=")
      split(parts[2], vals, "/")
      avg = vals[2]
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", avg)
    }
    END {
      printf "%s %s\n", loss, avg
    }
  ' "$1"
}

docker_bench_append_summary_row() {
  local backend="$1"
  local threads="$2"
  local duration="$3"
  local raw_dir="$4"
  local tcp_single_json="$5"
  local tcp_4_json="$6"
  local tcp_8_json="$7"
  local udp_200_json="$8"
  local udp_1000_json="$9"
  local ping_output="${10}"
  local ping_loss ping_avg

  read -r ping_loss ping_avg <<<"$(docker_bench_parse_ping_loss_avg "$ping_output")"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$backend" \
    "$threads" \
    "$duration" \
    "$(docker_bench_iperf_mbps "$tcp_single_json")" \
    "$(docker_bench_iperf_retrans "$tcp_single_json")" \
    "$(docker_bench_iperf_mbps "$tcp_4_json")" \
    "$(docker_bench_iperf_retrans "$tcp_4_json")" \
    "$(docker_bench_iperf_mbps "$tcp_8_json")" \
    "$(docker_bench_iperf_retrans "$tcp_8_json")" \
    "$(docker_bench_iperf_mbps "$udp_200_json")" \
    "$(docker_bench_iperf_loss_pct "$udp_200_json")" \
    "$(docker_bench_iperf_mbps "$udp_1000_json")" \
    "$(docker_bench_iperf_loss_pct "$udp_1000_json")" \
    "$ping_loss" \
    "$ping_avg" \
    "$raw_dir" >>"$SUMMARY_TSV"
}

docker_bench_guard_threshold() {
  local specific_name="$1"
  local common_name="$2"
  local specific_value="${!specific_name:-}"
  local common_value="${!common_name:-}"
  if [[ -n "$specific_value" ]]; then
    printf '%s' "$specific_value"
  else
    printf '%s' "$common_value"
  fi
}

docker_bench_is_number() {
  [[ "${1:-}" =~ ^[0-9]+([.][0-9]+)?$ ]]
}

docker_bench_guard_record_failure() {
  local failure_path="$1"
  local label="$2"
  local comparison="$3"
  local actual="$4"
  local threshold="$5"
  if [[ ! -f "$failure_path" ]]; then
    printf '%s\t%s\t%s\t%s\n' label comparison actual threshold >"$failure_path"
  fi
  printf '%s\t%s\t%s\t%s\n' \
    "$(docker_bench_tsv_field "$label")" \
    "$comparison" \
    "$(docker_bench_tsv_field "$actual")" \
    "$(docker_bench_tsv_field "$threshold")" >>"$failure_path"
  printf 'docker bench guard failed: %s %s %s (threshold %s)\n' \
    "$label" "$actual" "$comparison" "$threshold" >&2
}

docker_bench_guard_check_at_least() {
  local failure_path="$1"
  local label="$2"
  local actual="$3"
  local threshold="$4"
  [[ -n "$threshold" ]] || return 0
  if ! docker_bench_is_number "$threshold"; then
    docker_bench_guard_record_failure "$failure_path" "$label" ">=" "$actual" "invalid:$threshold"
    return 1
  fi
  if ! docker_bench_is_number "$actual"; then
    docker_bench_guard_record_failure "$failure_path" "$label" ">=" "missing:$actual" "$threshold"
    return 1
  fi
  if ! awk -v actual="$actual" -v threshold="$threshold" \
    'BEGIN { exit !((actual + 0) >= (threshold + 0)) }'; then
    docker_bench_guard_record_failure "$failure_path" "$label" ">=" "$actual" "$threshold"
    return 1
  fi
}

docker_bench_guard_check_at_most() {
  local failure_path="$1"
  local label="$2"
  local actual="$3"
  local threshold="$4"
  [[ -n "$threshold" ]] || return 0
  if ! docker_bench_is_number "$threshold"; then
    docker_bench_guard_record_failure "$failure_path" "$label" "<=" "$actual" "invalid:$threshold"
    return 1
  fi
  if ! docker_bench_is_number "$actual"; then
    docker_bench_guard_record_failure "$failure_path" "$label" "<=" "missing:$actual" "$threshold"
    return 1
  fi
  if ! awk -v actual="$actual" -v threshold="$threshold" \
    'BEGIN { exit !((actual + 0) <= (threshold + 0)) }'; then
    docker_bench_guard_record_failure "$failure_path" "$label" "<=" "$actual" "$threshold"
    return 1
  fi
}

docker_bench_assert_summary_guards() {
  local summary_tsv="${1:-$SUMMARY_TSV}"
  local failure_path="${OUTPUT_DIR}/guard-failures.tsv"
  local failure_count
  local backend threads duration tcp_single tcp_single_retrans tcp_4 tcp_4_retrans
  local tcp_8 tcp_8_retrans udp_200 udp_200_loss udp_1000 udp_1000_loss
  local ping_loss ping_avg raw_dir
  local tsv_value

  tsv_value() {
    awk -F '\t' -v idx="$1" 'END { print $idx }' "$summary_tsv"
  }
  backend="$(tsv_value 1)"
  threads="$(tsv_value 2)"
  duration="$(tsv_value 3)"
  tcp_single="$(tsv_value 4)"
  tcp_single_retrans="$(tsv_value 5)"
  tcp_4="$(tsv_value 6)"
  tcp_4_retrans="$(tsv_value 7)"
  tcp_8="$(tsv_value 8)"
  tcp_8_retrans="$(tsv_value 9)"
  udp_200="$(tsv_value 10)"
  udp_200_loss="$(tsv_value 11)"
  udp_1000="$(tsv_value 12)"
  udp_1000_loss="$(tsv_value 13)"
  ping_loss="$(tsv_value 14)"
  ping_avg="$(tsv_value 15)"
  raw_dir="$(tsv_value 16)"

  mkdir -p "$OUTPUT_DIR"
  rm -f "$failure_path"
  failure_count=0

  docker_bench_guard_check_at_least "$failure_path" "tcp_single_mbps" "$tcp_single" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_SINGLE_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_least "$failure_path" "tcp_4_mbps" "$tcp_4" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_4_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_least "$failure_path" "tcp_8_mbps" "$tcp_8" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_8_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))

  docker_bench_guard_check_at_most "$failure_path" "tcp_single_retrans" "$tcp_single_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_SINGLE_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_most "$failure_path" "tcp_4_retrans" "$tcp_4_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_4_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_most "$failure_path" "tcp_8_retrans" "$tcp_8_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_8_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))

  docker_bench_guard_check_at_most "$failure_path" "udp_200_loss_pct" "$udp_200_loss" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_UDP200_LOSS_PCT NVPN_DOCKER_MAX_UDP_LOSS_PCT)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_most "$failure_path" "udp_1000_loss_pct" "$udp_1000_loss" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_UDP1000_LOSS_PCT NVPN_DOCKER_MAX_UDP_LOSS_PCT)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_most "$failure_path" "ping_loss_pct" "$ping_loss" "${NVPN_DOCKER_MAX_PING_LOSS_PCT:-}" || failure_count=$((failure_count + 1))

  if (( failure_count > 0 )); then
    printf 'docker bench guard failed: wrote %s\n' "$failure_path" >&2
    return 1
  fi
}

docker_bench_pipeline_lines_after_start_from_stdin() {
  local start_line="${1:-0}"
  awk -v start_line="$start_line" 'NR > start_line'
}

docker_bench_peak_wait_pipeline_line_from_stdin() {
  awk '
    function duration_us(value, number) {
      number = value + 0
      if (value ~ /ns$/) {
        return number / 1000
      }
      if (value ~ /ms$/) {
        return number * 1000
      }
      if (value ~ /s$/ && value !~ /us$/ && value !~ /ms$/ && value !~ /ns$/) {
        return number * 1000000
      }
      return number
    }
    function metric_avg_us(line, metric, start, rest, avg_start, parts) {
      start = index(line, metric "=")
      if (start == 0) {
        return -1
      }
      rest = substr(line, start)
      avg_start = index(rest, " avg=")
      if (avg_start == 0) {
        return -1
      }
      rest = substr(rest, avg_start + 5)
      split(rest, parts, " ")
      return duration_us(parts[1])
    }
    function fips_score(line, score, value) {
      score = -1
      value = metric_avg_us(line, "endpoint_event_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_priority_event_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_bulk_event_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "transport_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "transport_priority_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "transport_bulk_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_worker_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_worker_priority_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_worker_bulk_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_linux_bulk_container_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_linux_bulk_container_ready_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_linux_bulk_container_first_slot_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fmp_linux_bulk_container_all_slots_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "decrypt_fallback_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "decrypt_fallback_priority_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "decrypt_fallback_bulk_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_command_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_priority_command_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_bulk_command_wait")
      if (value > score) score = value
      return score
    }
    function nvpn_score(line, score, value) {
      score = -1
      value = metric_avg_us(line, "nvpn_tun_to_mesh_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "nvpn_mesh_to_tun_queue_wait")
      if (value > score) score = value
      return score
    }
    /^\[pipe / {
      score = fips_score($0)
      if (fips == "" || score > fips_score_best || score == fips_score_best) {
        fips = $0
        fips_score_best = score
      }
    }
    /^\[nvpn-pipe / {
      score = nvpn_score($0)
      if (nvpn == "" || score > nvpn_score_best || score == nvpn_score_best) {
        nvpn = $0
        nvpn_score_best = score
      }
    }
    END {
      if (fips != "" && nvpn != "") {
        print fips " | " nvpn
      } else if (fips != "") {
        print fips
      } else if (nvpn != "") {
        print nvpn
      }
    }
  '
}

docker_bench_load_pipeline_line_from_stdin() {
  awk '
    function metric_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return -1
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    function fips_load_score(line, score, value) {
      score = -1
      value = metric_rate(line, "fmp_worker_batch_packets")
      if (value > score) score = value
      value = metric_rate(line, "udp_send_connected")
      if (value > score) score = value
      value = metric_rate(line, "connected_udp_direct_decrypt")
      if (value > score) score = value
      value = metric_rate(line, "endpoint_send")
      if (value > score) score = value
      value = metric_rate(line, "fmp_decrypt")
      if (value > score) score = value
      return score
    }
    function nvpn_load_score(line, score, value) {
      score = -1
      value = metric_rate(line, "nvpn_tun_read")
      if (value > score) score = value
      value = metric_rate(line, "nvpn_tun_write")
      if (value > score) score = value
      value = metric_rate(line, "nvpn_tun_to_mesh_queue_wait")
      if (value > score) score = value
      value = metric_rate(line, "nvpn_mesh_send")
      if (value > score) score = value
      return score
    }
    /^\[pipe / {
      score = fips_load_score($0)
      if (fips == "" || score > fips_score_best || score == fips_score_best) {
        fips = $0
        fips_score_best = score
      }
    }
    /^\[nvpn-pipe / {
      score = nvpn_load_score($0)
      if (nvpn == "" || score > nvpn_score_best || score == nvpn_score_best) {
        nvpn = $0
        nvpn_score_best = score
      }
    }
    END {
      if (fips != "" && nvpn != "") {
        print fips " | " nvpn
      } else if (fips != "") {
        print fips
      } else if (nvpn != "") {
        print nvpn
      }
    }
  '
}

docker_bench_pipeline_queue_wait_top_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function duration_ms(value, number) {
      number = value + 0
      if (value ~ /ns$/) {
        return number / 1000000
      }
      if (value ~ /us$/) {
        return number / 1000
      }
      if (value ~ /ms$/) {
        return number
      }
      if (value ~ /s$/) {
        return number * 1000
      }
      return number
    }
    function parse_wait(line, metric, start, rest, parts, rate_raw, p95_raw, p99_raw, max_raw, allmax_raw) {
      start = index(line, metric "=")
      if (start == 0) {
        return 0
      }
      rest = substr(line, start)
      split(rest, parts, " ")
      if (parts[4] !~ /^p95<=/ || parts[5] !~ /^p99<=/ || parts[6] !~ /^max<=/ || parts[7] !~ /^allmax=/) {
        return 0
      }
      rate_raw = parts[1]
      p95_raw = parts[4]
      p99_raw = parts[5]
      max_raw = parts[6]
      allmax_raw = parts[7]
      sub(/^[^=]+=/, "", rate_raw)
      sub(/\/s$/, "", rate_raw)
      sub(/^p95<=/, "", p95_raw)
      sub(/^p99<=/, "", p99_raw)
      sub(/^max<=/, "", max_raw)
      sub(/^allmax=/, "", allmax_raw)
      metric_rate = rate_raw + 0
      metric_p95 = duration_ms(p95_raw)
      metric_p99 = duration_ms(p99_raw)
      metric_max = duration_ms(max_raw)
      metric_allmax = duration_ms(allmax_raw)
      return 1
    }
    function metric_specificity(metric) {
      return metric ~ /_(priority|bulk)_/ ? 1 : 0
    }
    function metric_is_better(metric, specificity) {
      specificity = metric_specificity(metric)
      if (best_name == "") {
        return 1
      }
      if (metric_p99 != best_p99) {
        return metric_p99 > best_p99
      }
      if (metric_p95 != best_p95) {
        return metric_p95 > best_p95
      }
      if (metric_max != best_max) {
        return metric_max > best_max
      }
      if (specificity != best_specificity) {
        return specificity > best_specificity
      }
      return 0
    }
    BEGIN {
      metrics = "endpoint_command_wait endpoint_priority_command_wait endpoint_bulk_command_wait endpoint_event_wait endpoint_priority_event_wait endpoint_bulk_event_wait fmp_worker_queue_wait fmp_worker_priority_queue_wait fmp_worker_bulk_queue_wait fmp_linux_bulk_container_queue_wait fmp_linux_bulk_container_ready_wait fmp_linux_bulk_container_first_slot_wait fmp_linux_bulk_container_all_slots_wait decrypt_worker_queue_wait decrypt_worker_priority_queue_wait decrypt_worker_bulk_queue_wait decrypt_fallback_wait decrypt_fallback_priority_wait decrypt_fallback_bulk_wait decrypt_authenticated_session_wait decrypt_authenticated_session_priority_wait decrypt_authenticated_session_bulk_wait decrypt_fsp_worker_queue_wait decrypt_fsp_worker_priority_queue_wait decrypt_fsp_worker_bulk_queue_wait transport_queue_wait transport_priority_queue_wait transport_bulk_queue_wait transport_channel_wait transport_priority_channel_wait transport_bulk_channel_wait transport_rx_loop_wait transport_priority_rx_loop_wait transport_bulk_rx_loop_wait nvpn_tun_to_mesh_queue_wait nvpn_mesh_to_tun_queue_wait"
      metric_count = split(metrics, names, " ")
      best_p99 = -1
      best_p95 = -1
      best_max = -1
      best_allmax = -1
      best_specificity = -1
    }
    {
      line = $0
      for (i = 1; i <= metric_count; i++) {
        name = names[i]
        if (!parse_wait(line, name)) {
          continue
        }
        if (metric_is_better(name)) {
          best_name = name
          best_p95 = metric_p95
          best_p99 = metric_p99
          best_max = metric_max
          best_allmax = metric_allmax
          best_rate = metric_rate
          best_specificity = metric_specificity(name)
        }
      }
    }
    END {
      if (best_name != "") {
        printf "%s:rate_per_sec=%g,p95_ms=%g,p99_ms=%g,max_ms=%g,allmax_ms=%g\n", best_name, best_rate, best_p95, best_p99, best_max, best_allmax
      }
    }
  '
}

docker_bench_pipeline_worker_batch_summary() {
  local line="$1"
  local prefix="$2"
  printf '%s\n' "$line" | awk -v prefix="$prefix" '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    {
      flush = parse_rate($0, prefix "_flush")
      if (flush <= 0) {
        next
      }
      packets = parse_rate($0, prefix "_packets")
      full = parse_rate($0, prefix "_full")
      single = parse_rate($0, prefix "_single")
      priority_packets = parse_rate($0, prefix "_priority_packets")
      bulk_packets = parse_rate($0, prefix "_bulk_packets")
      send_groups = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group") : 0
      send_group_packets = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_packets") : 0
      send_group_single = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_single") : 0
      avg = packets / flush
      full_pct = (full / flush) * 100
      single_pct = (single / flush) * 100
      send_groups_per_flush = send_groups > 0 ? send_groups / flush : 0
      send_group_avg = send_groups > 0 ? send_group_packets / send_groups : 0
      send_group_single_pct = send_groups > 0 ? (send_group_single / send_groups) * 100 : 0
      lane_packets = priority_packets + bulk_packets
      if (lane_packets > 0) {
        priority_pct = (priority_packets / lane_packets) * 100
        bulk_pct = (bulk_packets / lane_packets) * 100
        summary = sprintf("avg_packets=%.1f,full_pct=%.1f,single_pct=%.1f,priority_pct=%.1f,bulk_pct=%.1f,flush_per_sec=%g,packets_per_sec=%g,priority_packets_per_sec=%g,bulk_packets_per_sec=%g", avg, full_pct, single_pct, priority_pct, bulk_pct, flush, packets, priority_packets, bulk_packets)
      } else {
        summary = sprintf("avg_packets=%.1f,full_pct=%.1f,single_pct=%.1f,flush_per_sec=%g,packets_per_sec=%g", avg, full_pct, single_pct, flush, packets)
      }
      if (send_groups > 0) {
        summary = summary sprintf(",send_groups_per_flush=%.1f,send_group_avg_packets=%.1f,send_group_single_pct=%.1f,send_groups_per_sec=%g,send_group_packets_per_sec=%g", send_groups_per_flush, send_group_avg, send_group_single_pct, send_groups, send_group_packets)
      }
      print summary
      exit
    }
  '
}

docker_bench_pipeline_fmp_worker_batch_summary() {
  docker_bench_pipeline_worker_batch_summary "$1" "fmp_worker_batch"
}

docker_bench_pipeline_decrypt_worker_batch_summary() {
  docker_bench_pipeline_worker_batch_summary "$1" "decrypt_worker_batch"
}

docker_bench_pipeline_linux_bulk_container_summary() {
  if [[ $# -gt 0 ]]; then
    printf '%s\n' "$1"
  else
    cat
  fi | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    function duration_ms(raw, number) {
      number = raw
      sub(/(ns|us|ms|s)$/, "", number)
      number += 0
      if (raw ~ /ns$/) {
        return number / 1000000
      }
      if (raw ~ /us$/) {
        return number / 1000
      }
      if (raw ~ /ms$/) {
        return number
      }
      if (raw ~ /s$/) {
        return number * 1000
      }
      return number
    }
    function parse_wait(line, metric, suffix, start, rest, parts, p95_raw, p99_raw) {
      start = index(line, metric "=")
      if (start == 0) {
        delete waits[suffix "_p95"]
        delete waits[suffix "_p99"]
        return
      }
      rest = substr(line, start)
      split(rest, parts, " ")
      if (parts[4] !~ /^p95<=/ || parts[5] !~ /^p99<=/) {
        delete waits[suffix "_p95"]
        delete waits[suffix "_p99"]
        return
      }
      p95_raw = parts[4]
      p99_raw = parts[5]
      sub(/^p95<=/, "", p95_raw)
      sub(/^p99<=/, "", p99_raw)
      waits[suffix "_p95"] = duration_ms(p95_raw)
      waits[suffix "_p99"] = duration_ms(p99_raw)
    }
    function append_wait(summary, key, label) {
      if (!(key in waits)) {
        return summary
      }
      return summary sprintf(",%s_ms=%g", label, waits[key])
    }
    function build_summary(line, enqueued, packets, sent, sent_packets, avg_packets, avg_sent_packets, summary) {
      parse_wait(line, "fmp_linux_bulk_container_queue_wait", "queue")
      parse_wait(line, "fmp_linux_bulk_container_ready_wait", "ready")
      parse_wait(line, "fmp_linux_bulk_container_first_slot_wait", "first_slot")
      parse_wait(line, "fmp_linux_bulk_container_all_slots_wait", "all_slots")
      avg_packets = enqueued > 0 ? packets / enqueued : 0
      avg_sent_packets = sent > 0 ? sent_packets / sent : 0
      summary = sprintf("avg_packets=%.1f,avg_sent_packets=%.1f,enqueued_per_sec=%g,packets_per_sec=%g,sent_per_sec=%g,sent_packets_per_sec=%g", avg_packets, avg_sent_packets, enqueued, packets, sent, sent_packets)
      summary = append_wait(summary, "queue_p95", "queue_p95")
      summary = append_wait(summary, "queue_p99", "queue_p99")
      summary = append_wait(summary, "ready_p95", "ready_p95")
      summary = append_wait(summary, "ready_p99", "ready_p99")
      summary = append_wait(summary, "first_slot_p95", "first_slot_p95")
      summary = append_wait(summary, "first_slot_p99", "first_slot_p99")
      summary = append_wait(summary, "all_slots_p95", "all_slots_p95")
      summary = append_wait(summary, "all_slots_p99", "all_slots_p99")
      return summary
    }
    {
      line = $0
      enqueued = parse_rate(line, "fmp_linux_bulk_container_enqueued")
      packets = parse_rate(line, "fmp_linux_bulk_container_packets")
      sent = parse_rate(line, "fmp_linux_bulk_container_sent")
      sent_packets = parse_rate(line, "fmp_linux_bulk_container_sent_packets")
      if (enqueued <= 0 && packets <= 0 && sent <= 0 && sent_packets <= 0) {
        next
      }
      score = packets > sent_packets ? packets : sent_packets
      if (best == "" || score > best_score || (score == best_score && enqueued > best_enqueued)) {
        best = build_summary(line, enqueued, packets, sent, sent_packets)
        best_score = score
        best_enqueued = enqueued
      }
    }
    END {
      if (best != "") {
        print best
      }
    }
  '
}

docker_bench_pipeline_udp_send_batch_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    {
      gso_batches = parse_rate($0, "udp_send_gso_batch")
      gso_packets = parse_rate($0, "udp_send_gso_packets")
      gso_ge32 = parse_rate($0, "udp_send_gso_batch_ge32")
      gso_ge48 = parse_rate($0, "udp_send_gso_batch_ge48")
      gso_eq64 = parse_rate($0, "udp_send_gso_batch_eq64")
      sendmmsg_batches = parse_rate($0, "udp_send_sendmmsg_batch")
      sendmmsg_packets = parse_rate($0, "udp_send_sendmmsg_packets")
      sendmmsg_ge32 = parse_rate($0, "udp_send_sendmmsg_batch_ge32")
      sendmmsg_ge48 = parse_rate($0, "udp_send_sendmmsg_batch_ge48")
      sendmmsg_eq64 = parse_rate($0, "udp_send_sendmmsg_batch_eq64")
      total_batches = gso_batches + sendmmsg_batches
      total_packets = gso_packets + sendmmsg_packets
      if (total_batches <= 0 && total_packets <= 0) {
        next
      }
      gso_pct = total_packets > 0 ? (gso_packets / total_packets) * 100 : 0
      sendmmsg_pct = total_packets > 0 ? (sendmmsg_packets / total_packets) * 100 : 0
      gso_avg = gso_batches > 0 ? gso_packets / gso_batches : 0
      sendmmsg_avg = sendmmsg_batches > 0 ? sendmmsg_packets / sendmmsg_batches : 0
      gso_ge32_pct = gso_batches > 0 ? (gso_ge32 / gso_batches) * 100 : 0
      gso_ge48_pct = gso_batches > 0 ? (gso_ge48 / gso_batches) * 100 : 0
      gso_eq64_pct = gso_batches > 0 ? (gso_eq64 / gso_batches) * 100 : 0
      sendmmsg_ge32_pct = sendmmsg_batches > 0 ? (sendmmsg_ge32 / sendmmsg_batches) * 100 : 0
      sendmmsg_ge48_pct = sendmmsg_batches > 0 ? (sendmmsg_ge48 / sendmmsg_batches) * 100 : 0
      sendmmsg_eq64_pct = sendmmsg_batches > 0 ? (sendmmsg_eq64 / sendmmsg_batches) * 100 : 0
      avg = total_batches > 0 ? total_packets / total_batches : 0
      printf "gso_packet_pct=%.1f,sendmmsg_packet_pct=%.1f,avg_packets=%.1f,gso_avg_packets=%.1f,sendmmsg_avg_packets=%.1f,gso_ge32_pct=%.1f,gso_ge48_pct=%.1f,gso_eq64_pct=%.1f,sendmmsg_ge32_pct=%.1f,sendmmsg_ge48_pct=%.1f,sendmmsg_eq64_pct=%.1f,gso_batch_per_sec=%g,gso_packets_per_sec=%g,sendmmsg_batch_per_sec=%g,sendmmsg_packets_per_sec=%g,total_packets_per_sec=%g\n", gso_pct, sendmmsg_pct, avg, gso_avg, sendmmsg_avg, gso_ge32_pct, gso_ge48_pct, gso_eq64_pct, sendmmsg_ge32_pct, sendmmsg_ge48_pct, sendmmsg_eq64_pct, gso_batches, gso_packets, sendmmsg_batches, sendmmsg_packets, total_packets
      exit
    }
  '
}

docker_bench_pipeline_nvpn_tun_read_batch_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    {
      flush = parse_rate($0, "nvpn_tun_read_batch_flush")
      packets = parse_rate($0, "nvpn_tun_read_batch_packets")
      full = parse_rate($0, "nvpn_tun_read_batch_full")
      single = parse_rate($0, "nvpn_tun_read_batch_single")
      bytes = parse_rate($0, "nvpn_tun_read_packet_bytes")
      if (flush <= 0 && packets <= 0 && bytes <= 0) {
        next
      }
      avg_packets = flush > 0 ? packets / flush : 0
      full_pct = flush > 0 ? (full / flush) * 100 : 0
      single_pct = flush > 0 ? (single / flush) * 100 : 0
      avg_bytes = packets > 0 ? bytes / packets : 0
      printf "avg_packets=%.1f,full_pct=%.1f,single_pct=%.1f,avg_packet_bytes=%.1f,flush_per_sec=%g,packets_per_sec=%g,bytes_per_sec=%g\n", avg_packets, full_pct, single_pct, avg_bytes, flush, packets, bytes
      exit
    }
  '
}

docker_bench_pipeline_nvpn_mesh_send_batch_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    {
      flush = parse_rate($0, "nvpn_mesh_send_batch_flush")
      input_packets = parse_rate($0, "nvpn_mesh_send_batch_input_packets")
      routed_packets = parse_rate($0, "nvpn_mesh_send_batch_routed_packets")
      runs = parse_rate($0, "nvpn_mesh_send_batch_runs")
      full = parse_rate($0, "nvpn_mesh_send_batch_full")
      if (flush <= 0 && input_packets <= 0 && routed_packets <= 0 && runs <= 0) {
        next
      }
      avg_input_packets = flush > 0 ? input_packets / flush : 0
      avg_routed_packets = flush > 0 ? routed_packets / flush : 0
      avg_runs = flush > 0 ? runs / flush : 0
      routed_pct = input_packets > 0 ? (routed_packets / input_packets) * 100 : 0
      full_pct = flush > 0 ? (full / flush) * 100 : 0
      printf "avg_input_packets=%.1f,avg_routed_packets=%.1f,avg_runs=%.1f,routed_pct=%.1f,full_pct=%.1f,flush_per_sec=%g,input_packets_per_sec=%g,routed_packets_per_sec=%g,runs_per_sec=%g\n", avg_input_packets, avg_routed_packets, avg_runs, routed_pct, full_pct, flush, input_packets, routed_packets, runs
      exit
    }
  '
}

docker_bench_pipeline_nvpn_mesh_recv_batch_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    {
      flush = parse_rate($0, "nvpn_mesh_recv_batch_flush")
      events = parse_rate($0, "nvpn_mesh_recv_batch_events")
      packets = parse_rate($0, "nvpn_mesh_recv_batch_packets")
      bytes = parse_rate($0, "nvpn_mesh_recv_packet_bytes")
      full = parse_rate($0, "nvpn_mesh_recv_batch_full")
      single = parse_rate($0, "nvpn_mesh_recv_batch_single_packet")
      if (flush <= 0 && events <= 0 && packets <= 0 && bytes <= 0) {
        next
      }
      avg_events = flush > 0 ? events / flush : 0
      avg_packets = flush > 0 ? packets / flush : 0
      full_pct = flush > 0 ? (full / flush) * 100 : 0
      single_packet_pct = flush > 0 ? (single / flush) * 100 : 0
      avg_bytes = packets > 0 ? bytes / packets : 0
      printf "avg_events=%.1f,avg_packets=%.1f,full_pct=%.1f,single_packet_pct=%.1f,avg_packet_bytes=%.1f,flush_per_sec=%g,events_per_sec=%g,packets_per_sec=%g,bytes_per_sec=%g\n", avg_events, avg_packets, full_pct, single_packet_pct, avg_bytes, flush, events, packets, bytes
      exit
    }
  '
}

docker_bench_pipeline_nvpn_tun_write_summary() {
  local line="$1"
  printf '%s\n' "$line" | docker_bench_pipeline_nvpn_tun_write_summary_from_stdin
}

docker_bench_pipeline_nvpn_tun_write_summary_from_stdin() {
  awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return ""
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    function consider(line, packets, bytes, would_block, frames, frame_bytes, ratio, score) {
      packets = parse_rate(line, "nvpn_tun_write_packets")
      bytes = parse_rate(line, "nvpn_tun_write_packet_bytes")
      would_block = parse_rate(line, "nvpn_tun_write_would_block")
      frames = parse_rate(line, "nvpn_tun_write_frames")
      frame_bytes = parse_rate(line, "nvpn_tun_write_frame_bytes")
      if (would_block < 0) would_block = 0
      if (frames < 0) frames = 0
      if (frame_bytes < 0) frame_bytes = 0
      if (packets <= 0 && bytes <= 0 && would_block <= 0 && frames <= 0 && frame_bytes <= 0) {
        return
      }
      avg_bytes = packets > 0 ? bytes / packets : 0
      avg_packets_per_frame = frames > 0 ? packets / frames : 0
      avg_frame_bytes = frames > 0 ? frame_bytes / frames : 0
      ratio = frames > 0 ? packets / frames : 0
      score = ratio > 0 ? ratio : packets
      if (best == "" || score > best_score || (score == best_score && packets > best_packets)) {
        best_score = score
        best_packets = packets
        summary = sprintf("packets_per_sec=%g,bytes_per_sec=%g,avg_packet_bytes=%.1f", packets, bytes, avg_bytes)
        if (frames > 0 || frame_bytes > 0) {
          summary = summary sprintf(",frames_per_sec=%g,avg_packets_per_frame=%.1f,avg_frame_bytes=%.1f", frames, avg_packets_per_frame, avg_frame_bytes)
        }
        summary = summary sprintf(",would_block_per_sec=%g", would_block)
        best = summary
      }
    }
    {
      consider($0)
    }
    END {
      if (best != "") {
        print best
      }
    }
  '
}

docker_bench_pipeline_hard_event_summary_from_stdin() {
  local start_line="${1:-0}"
  awk -v start_line="$start_line" '
    function parse_event(line, name, in_phase, start, rest, parts, rate_raw, rate, total_raw, total, i) {
      start = index(line, name "=")
      if (start == 0) {
        return
      }
      rest = substr(line, start + length(name) + 1)
      split(rest, parts, " ")
      rate_raw = parts[1]
      sub(/\/s$/, "", rate_raw)
      rate = rate_raw + 0
      if (in_phase && (!(name in max_rate) || rate > max_rate[name])) {
        max_rate[name] = rate
      }
      for (i = 2; i <= 6; i++) {
        if (!(i in parts)) {
          break
        }
        if (parts[i] ~ /^total=/) {
          total_raw = parts[i]
          sub(/^total=/, "", total_raw)
          total = total_raw + 0
          if (in_phase && (!(name in max_total) || total > max_total[name])) {
            max_total[name] = total
          } else if (!in_phase && (!(name in base_total) || total > base_total[name])) {
            base_total[name] = total
          }
          break
        }
      }
    }
    BEGIN {
      events = "udp_send_backpressure udp_send_backpressure_sleep udp_send_bulk_dropped connected_udp_activation_failed connected_udp_peer_cap_skipped connected_udp_fd_budget_skipped encrypt_worker_queue_full encrypt_worker_priority_queue_full encrypt_worker_bulk_queue_full encrypt_worker_bulk_dropped fmp_linux_bulk_container_queue_full fmp_linux_bulk_container_queue_full_packets endpoint_direct_fmp_receive_dropped endpoint_direct_fmp_receive_dropped_packets decrypt_worker_queue_full decrypt_worker_bulk_dropped decrypt_worker_register_full decrypt_worker_priority_dropped decrypt_fallback_backlog_high rx_loop_slow_maintenance_timeout rx_loop_slow_maintenance_skipped decrypt_fallback_bulk_dropped decrypt_fallback_priority_dropped decrypt_fallback_pressure_drain decrypt_fallback_priority_gated decrypt_fsp_priority_queue_full_fallback decrypt_fsp_bulk_queue_full_fallback decrypt_fsp_worker_replay_dropped decrypt_authenticated_session_priority_dropped decrypt_authenticated_session_bulk_dropped pending_tun_destination_dropped pending_tun_packet_dropped pending_endpoint_destination_dropped pending_endpoint_packet_dropped endpoint_event_backlog_high endpoint_command_bulk_dropped endpoint_event_bulk_dropped transport_channel_backlog_high transport_bulk_dropped nvpn_tun_to_mesh_bulk_dropped"
      event_count = split(events, names, " ")
    }
    {
      in_phase = (NR > start_line)
      for (i = 1; i <= event_count; i++) {
        parse_event($0, names[i], in_phase)
      }
    }
    END {
      first = 1
      for (i = 1; i <= event_count; i++) {
        name = names[i]
        rate = (name in max_rate) ? max_rate[name] : 0
        base = (name in base_total) ? base_total[name] : 0
        total = (name in max_total) ? max_total[name] - base : 0
        if (total < 0) {
          total = 0
        }
        if (rate <= 0 && total <= 0) {
          continue
        }
        if (!first) {
          printf ";"
        }
        first = 0
        printf "%s:max_rate_per_sec=%g,total=%g", name, rate, total
      }
      if (!first) {
        printf "\n"
      }
    }
  '
}
