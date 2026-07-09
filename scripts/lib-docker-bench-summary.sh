#!/usr/bin/env bash
# Shared parsers/summary writers for simple Docker VPN benchmark scripts.

docker_bench_write_tsv_row() {
  local first="$1"
  shift
  printf '%s' "$first"
  printf '\t%s' "$@"
  printf '\n'
}

docker_bench_init_summary() {
  mkdir -p "$RAW_DIR"
  docker_bench_write_tsv_row \
    backend threads duration_secs \
    tcp_single_mbps tcp_single_retrans \
    tcp_4_mbps tcp_4_retrans \
    tcp_8_mbps tcp_8_retrans \
    udp_200_mbps udp_200_loss_pct \
    udp_1000_mbps udp_1000_loss_pct \
    ping_loss_pct ping_avg_ms raw_dir \
    cpu_stress_enabled cpu_stress_sides \
    cpu_stress_local_workers cpu_stress_remote_workers \
    iperf_socket_buffer udp1000_parallel \
    udp1000_bandwidth udp1000_per_stream_bandwidth \
    dataplane_profile placement_profile \
    ping_mdev_ms ping_p95_ms ping_p99_ms ping_max_ms \
    ping_samples ping_gt1ms ping_gt2ms ping_gt10ms \
    forward_direction reverse_direction \
    tcp_single_b_to_a_mbps tcp_single_b_to_a_retrans \
    tcp_4_b_to_a_mbps tcp_4_b_to_a_retrans \
    tcp_8_b_to_a_mbps tcp_8_b_to_a_retrans \
    udp_200_b_to_a_mbps udp_200_b_to_a_loss_pct \
    udp_1000_b_to_a_mbps udp_1000_b_to_a_loss_pct >"$SUMMARY_TSV"
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

docker_bench_size_to_bytes() {
  local raw="$1"
  local number suffix
  [[ "$raw" =~ ^([0-9]+)([KMG])?$ ]] || return 1
  number="${BASH_REMATCH[1]}"
  suffix="${BASH_REMATCH[2]:-}"
  case "$suffix" in
    K) printf '%s\n' "$((number * 1024))" ;;
    M) printf '%s\n' "$((number * 1024 * 1024))" ;;
    G) printf '%s\n' "$((number * 1024 * 1024 * 1024))" ;;
    *) printf '%s\n' "$number" ;;
  esac
}

docker_bench_configure_iperf_socket_buffer_limits() {
  local log_prefix="$1"
  local socket_buffer="$2"
  [[ -n "$socket_buffer" ]] || return 0

  local bytes service sysctl_log actual_rmem actual_wmem
  bytes="$(docker_bench_size_to_bytes "$socket_buffer")" || {
    printf '%s: invalid NVPN_DOCKER_IPERF_SOCKET_BUFFER=%s (expected bytes or K/M/G suffix)\n' \
      "$log_prefix" "$socket_buffer" >&2
    return 2
  }

  for service in node-a node-b; do
    sysctl_log="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      "sysctl -w net.core.rmem_max=$bytes net.core.wmem_max=$bytes" 2>&1 || true)"
    actual_rmem="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      'sysctl -n net.core.rmem_max' 2>/dev/null || true)"
    actual_wmem="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      'sysctl -n net.core.wmem_max' 2>/dev/null || true)"
    if [[ ! "$actual_rmem" =~ ^[0-9]+$ ]] \
      || [[ ! "$actual_wmem" =~ ^[0-9]+$ ]] \
      || (( actual_rmem < bytes || actual_wmem < bytes )); then
      printf '%s: failed to raise UDP socket buffer sysctls in %s for NVPN_DOCKER_IPERF_SOCKET_BUFFER=%s (wanted >=%s, got rmem_max=%s, wmem_max=%s)\n' \
        "$log_prefix" \
        "$service" \
        "$socket_buffer" \
        "$bytes" \
        "${actual_rmem:-unknown}" \
        "${actual_wmem:-unknown}" >&2
      if [[ -n "$sysctl_log" ]]; then
        printf '%s\n' "$sysctl_log" >&2
      fi
      return 1
    fi
  done
}

docker_bench_process_uses_translation() {
  local process_lines="$1"
  grep -Eq '(^|[[:space:]])/run/rosetta/rosetta([[:space:]]|$)|(^|[[:space:]])([^[:space:]]*/)?qemu-[^[:space:]]*([[:space:]]|$)' \
    <<<"$process_lines"
}

docker_bench_process_lines() {
  local service="$1"
  local process_name="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- "$process_name" <<'SH'
set -eu
process_name="$1"
ps -eo pid=,comm=,args= | awk -v name="$process_name" '
  $2 ~ /^(sh|bash|dash|grep|awk|ps)$/ { next }
  $2 == name || index($0, "/" name) || index($0, " " name " ") { print }
'
SH
}

docker_bench_assert_native_processes() {
  local log_prefix="$1"
  local process_name="$2"
  shift 2

  local service process_lines
  for service in "$@"; do
    process_lines="$(docker_bench_process_lines "$service" "$process_name")"
    if [[ -z "$process_lines" ]]; then
      printf '%s: no %s process found in %s after tunnel setup\n' \
        "$log_prefix" "$process_name" "$service" >&2
      return 1
    fi
    if docker_bench_process_uses_translation "$process_lines"; then
      printf '%s: %s %s is running through Rosetta/QEMU; rebuild the Docker image for native architecture\n' \
        "$log_prefix" "$service" "$process_name" >&2
      printf '%s\n' "$process_lines" >&2
      return 1
    fi
  done
}

docker_bench_local_fips_patch_enabled() {
  if [[ -n "${NVPN_PATCH_LOCAL_FIPS+x}" ]]; then
    docker_bench_bool_enabled "$NVPN_PATCH_LOCAL_FIPS"
    return
  fi

  [[ -n "${NVPN_FIPS_REPO_PATH:-}" ]]
}

docker_bench_apply_local_fips_patch_default() {
  if [[ -z "${NVPN_PATCH_LOCAL_FIPS+x}" && -n "${NVPN_FIPS_REPO_PATH:-}" ]]; then
    export NVPN_PATCH_LOCAL_FIPS=1
  fi
}

docker_bench_parse_rate_bps() {
  local raw="$1"
  local number suffix
  [[ "$raw" =~ ^([0-9]+)([KMG])?$ ]] || return 1
  number="${BASH_REMATCH[1]}"
  suffix="${BASH_REMATCH[2]:-}"
  case "$suffix" in
    K) printf '%s\n' "$((number * 1000))" ;;
    M) printf '%s\n' "$((number * 1000 * 1000))" ;;
    G) printf '%s\n' "$((number * 1000 * 1000 * 1000))" ;;
    *) printf '%s\n' "$number" ;;
  esac
}

docker_bench_udp1000_parallel_streams() {
  local streams="${NVPN_DOCKER_UDP1000_PARALLEL:-}"
  if [[ -z "$streams" ]]; then
    printf '1\n'
  else
    printf '%s\n' "$streams"
  fi
}

docker_bench_udp1000_per_stream_bandwidth() {
  local total="${NVPN_DOCKER_UDP1000_BANDWIDTH:-1G}"
  local streams total_bps per_stream_bps
  streams="$(docker_bench_udp1000_parallel_streams)"
  total_bps="$(docker_bench_parse_rate_bps "$total")" || return 1
  if (( streams <= 1 )); then
    printf '%s\n' "$total"
    return 0
  fi
  per_stream_bps=$(((total_bps + streams - 1) / streams))
  printf '%s\n' "$per_stream_bps"
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

docker_bench_env_assignment_present() {
  local name="$1"
  local token
  for token in ${2:-}; do
    case "$token" in
      "$name"=*) return 0 ;;
    esac
  done
  return 1
}

docker_bench_dataplane_profile_env() {
  local profile="${NVPN_DOCKER_DATAPLANE_PROFILE:-}"
  case "$profile" in
    "" | plain | default)
      return 0
      ;;
    linux-vnet)
      return 0
      ;;
    linux-vnet-lan)
      printf '%s\n' "NVPN_MESH_UNDERLAY_UDP_MTU=1472"
      ;;
    *)
      printf 'perf: unknown NVPN_DOCKER_DATAPLANE_PROFILE=%s (known: plain, linux-vnet, linux-vnet-lan)\n' \
        "$profile" >&2
      return 2
      ;;
  esac
}

docker_bench_placement_profile_env() {
  local profile="${NVPN_DOCKER_PLACEMENT_PROFILE:-}"
  case "$profile" in
    "" | plain | default)
      return 0
      ;;
    worker-open)
      return 0
      ;;
    *)
      printf 'perf: unknown NVPN_DOCKER_PLACEMENT_PROFILE=%s (known: plain, worker-open)\n' \
        "$profile" >&2
      return 2
      ;;
  esac
}

docker_bench_join_env_assignments() {
  local left="${1:-}"
  local right="${2:-}"
  if [[ -n "$left" && -n "$right" ]]; then
    printf '%s %s\n' "$left" "$right"
  elif [[ -n "$left" ]]; then
    printf '%s\n' "$left"
  else
    printf '%s\n' "$right"
  fi
}

docker_bench_effective_extra_env() {
  local profile_env placement_env
  profile_env="$(docker_bench_dataplane_profile_env)" || return $?
  placement_env="$(docker_bench_placement_profile_env)" || return $?
  docker_bench_join_env_assignments \
    "$(docker_bench_join_env_assignments "$profile_env" "$placement_env")" \
    "${NVPN_DOCKER_EXTRA_ENV:-}"
}

docker_bench_validate_connect_env_scope() {
  local effective_env="$1"
  local name value failed=0
  for name in \
    FIPS_DECRYPT_FMP_SOURCE_AFFINE_SESSION_OWNER \
    FIPS_DECRYPT_FSP_LOCAL_BULK_OPEN_WORKER \
    FIPS_DECRYPT_FSP_REMOTE_BULK_OPEN_WORKER \
    NVPN_FIPS_LINUX_TUN_VNET; do
    value="${!name:-}"
    [[ -n "$value" ]] || continue
    printf 'perf: %s\n' "$(docker_bench_extra_env_stale_assignment_message "$name")" >&2
    failed=1
  done
  for name in \
    NVPN_FIPS_LINUX_TUN_TX_QUEUE_LEN \
    NVPN_MESH_UNDERLAY_UDP_MTU \
    NVPN_MESH_MTU_PROFILE \
    FIPS_DECRYPT_FSP_OPEN_POOL; do
    value="${!name:-}"
    [[ -n "$value" ]] || continue
    if ! docker_bench_env_assignment_present "$name" "$effective_env"; then
      printf 'perf: %s=%s is set outside the daemon connect env; use NVPN_DOCKER_EXTRA_ENV or NVPN_DOCKER_DATAPLANE_PROFILE so nvpn connect sees it\n' \
        "$name" "$value" >&2
      failed=1
    fi
  done
  [[ "$failed" == "0" ]]
}

docker_bench_direct_fmp_forced_enabled() {
  local effective_env
  effective_env="$(docker_bench_effective_extra_env)" || return $?
  docker_bench_env_bool_assignment_enabled \
    FIPS_DIRECT_ENDPOINT_FMP_ONLY \
    "$effective_env"
}

docker_bench_extra_env_stale_assignment_message() {
  local name="$1"
  case "$name" in
    FIPS_FMP_AEAD_HELPERS|FIPS_DECRYPT_FMP_AEAD_HELPERS)
      printf '%s\n' \
        "$name is retired; FMP decrypt stays on the source-affine owner path"
      ;;
    FIPS_DECRYPT_FMP_SOURCE_AFFINE_SESSION_OWNER)
      printf '%s\n' \
        "FIPS_DECRYPT_FMP_SOURCE_AFFINE_SESSION_OWNER is retired; FMP decrypt always uses the canonical source-affine owner path"
      ;;
    FIPS_FSP_AEAD_HELPERS)
      printf '%s\n' \
        "FIPS_FSP_AEAD_HELPERS is not read by current FIPS; the FSP helper lane is retired, use worker-open placement"
      ;;
    FIPS_FSP_AEAD_HELPER_COMPLETIONS)
      printf '%s\n' \
        "FIPS_FSP_AEAD_HELPER_COMPLETIONS is not read by current FIPS; remove it"
      ;;
    FIPS_DECRYPT_FSP_ORDERED_AEAD_HELPERS)
      printf '%s\n' \
        "FIPS_DECRYPT_FSP_ORDERED_AEAD_HELPERS is retired; use worker-open placement instead of the FSP helper lane"
      ;;
    FIPS_DECRYPT_FSP_LOCAL_BULK_OPEN_WORKER)
      printf '%s\n' \
        "FIPS_DECRYPT_FSP_LOCAL_BULK_OPEN_WORKER is retired; same-owner bulk worker-open is the default when multiple decrypt workers are available"
      ;;
    FIPS_DECRYPT_FSP_REMOTE_BULK_OPEN_WORKER)
      printf '%s\n' \
        "FIPS_DECRYPT_FSP_REMOTE_BULK_OPEN_WORKER is retired; remote FSP worker-open bench placement is no longer a production dataplane knob"
      ;;
    FIPS_DECRYPT_FSP_OPEN_WORKER_MAX_COMPLETION_BACKLOG)
      printf '%s\n' \
        "FIPS_DECRYPT_FSP_OPEN_WORKER_MAX_COMPLETION_BACKLOG is retired; worker-open pressure is bounded by the ordered receive window and worker queues"
      ;;
    FIPS_DECRYPT_FSP_AEAD_COMPLETION_BATCH_MAX)
      printf '%s\n' \
        "FIPS_DECRYPT_FSP_AEAD_COMPLETION_BATCH_MAX is retired; FSP completion batching uses the accepted fixed worker-open width"
      ;;
    FIPS_LINUX_BULK_UDP_PACE_MBPS|FIPS_LINUX_BULK_UDP_PACE_BURST_BYTES|FIPS_LINUX_BULK_UDP_PACE_SPIN_NS)
      printf '%s\n' \
        "$name is retired; Linux bulk UDP sends use the accepted unpaced deferred/WG-batch sender shape"
      ;;
    FIPS_MACOS_ORDERED_SENDER|FIPS_MACOS_WORKER_STRIDE|FIPS_MACOS_SEND_FLOW_IDLE_MS)
      printf '%s\n' \
        "$name is retired; macOS uses the accepted hash-by-send-target worker sender"
      ;;
    FIPS_DECRYPT_FMP_PREOWNER_AEAD_HELPERS)
      printf '%s\n' \
        "FIPS_DECRYPT_FMP_PREOWNER_AEAD_HELPERS was removed; FMP uses the canonical source-affine owner path"
      ;;
    FIPS_ENCRYPT_WORKERS|FIPS_DECRYPT_WORKERS)
      printf '%s\n' \
        "$name is retired; current FIPS sizes dataplane workers from available parallelism"
      ;;
    NVPN_FIPS_LINUX_TUN_VNET)
      printf '%s\n' \
        "NVPN_FIPS_LINUX_TUN_VNET is retired; Linux FIPS TUN always uses vnet headers"
      ;;
    FIPS_WORKER_CHANNEL_CAP|FIPS_DECRYPT_WORKER_CHANNEL_CAP|FIPS_DECRYPT_WORKER_PRIORITY_CHANNEL_CAP)
      printf '%s\n' \
        "$name is not read by current FIPS; remove the stale worker queue cap override"
      ;;
    FIPS_SEND_BACKPRESSURE_SLEEP_AFTER|FIPS_SEND_BACKPRESSURE_SLEEP_MICROS|FIPS_SEND_BACKPRESSURE_DROP_AFTER)
      printf '%s\n' \
        "$name is not read by current FIPS; Linux sends use the canonical bulk sender"
      ;;
  esac
}

docker_bench_validate_extra_env_assignments() {
  local env_string="${1:-}"
  local token name message failed=0
  for token in $env_string; do
    case "$token" in
      *=*)
        name="${token%%=*}"
        message="$(docker_bench_extra_env_stale_assignment_message "$name")"
        if [[ -n "$message" ]]; then
          printf 'perf: stale benchmark extra-env assignment: %s\n' "$message" >&2
          failed=1
        fi
        ;;
    esac
  done
  [[ "$failed" == "0" ]]
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

docker_bench_host_loadavg() {
  if [[ -r /proc/loadavg ]]; then
    awk '{print $1 "\t" $2 "\t" $3}' /proc/loadavg
    return
  fi

  if command -v sysctl >/dev/null 2>&1; then
    local sysctl_load
    sysctl_load="$(sysctl -n vm.loadavg 2>/dev/null | awk '
      {
        for (i = 1; i <= NF; i++) {
          if ($i ~ /^[0-9]+([.][0-9]+)?$/) {
            values[++count] = $i
          }
        }
      }
      END {
        if (count >= 3) {
          printf "%s\t%s\t%s\n", values[1], values[2], values[3]
        }
      }' || true)"
    if [[ -n "$sysctl_load" ]]; then
      printf '%s\n' "$sysctl_load"
      return
    fi
  fi

  uptime 2>/dev/null | awk -F 'load averages?: ' '
    NF > 1 {
      gsub(/,/, "", $2)
      split($2, values, " ")
      if (values[1] != "" && values[2] != "" && values[3] != "") {
        printf "%s\t%s\t%s\n", values[1], values[2], values[3]
      }
    }' || true
}

docker_bench_host_online_cpus() {
  local cpus
  cpus="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
  if [[ "$cpus" =~ ^[1-9][0-9]*$ ]]; then
    printf '%s\n' "$cpus"
    return
  fi

  cpus="$(nproc 2>/dev/null || true)"
  if [[ "$cpus" =~ ^[1-9][0-9]*$ ]]; then
    printf '%s\n' "$cpus"
    return
  fi

  cpus="$(sysctl -n hw.ncpu 2>/dev/null || true)"
  if [[ "$cpus" =~ ^[1-9][0-9]*$ ]]; then
    printf '%s\n' "$cpus"
  fi
}

docker_bench_host_load_per_cpu() {
  local load_avg="$1"
  local cpus="$2"
  awk -v load_avg="$load_avg" -v cpus="$cpus" 'BEGIN {
    if (load_avg ~ /^[0-9]+([.][0-9]+)?$/ && cpus ~ /^[1-9][0-9]*$/) {
      printf "%.6f\n", load_avg / cpus
    }
  }'
}

docker_bench_validate_host_quiet() {
  local log_prefix="$1"
  local max_load_per_cpu="${NVPN_DOCKER_MAX_HOST_LOAD_PER_CPU:-}"
  [[ -n "$max_load_per_cpu" ]] || return 0
  if [[ ! "$max_load_per_cpu" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
    printf '%s: invalid NVPN_DOCKER_MAX_HOST_LOAD_PER_CPU=%s (expected non-negative number)\n' \
      "$log_prefix" "$max_load_per_cpu" >&2
    return 2
  fi

  local load1 load5 load15 cpus load_per_cpu
  IFS=$'\t' read -r load1 load5 load15 < <(docker_bench_host_loadavg) || true
  cpus="$(docker_bench_host_online_cpus)"
  load_per_cpu="$(docker_bench_host_load_per_cpu "$load1" "$cpus")"
  if [[ -z "$load_per_cpu" ]]; then
    printf '%s: unable to read host load for NVPN_DOCKER_MAX_HOST_LOAD_PER_CPU guard\n' \
      "$log_prefix" >&2
    return 2
  fi

  if ! awk -v actual="$load_per_cpu" -v max="$max_load_per_cpu" 'BEGIN { exit(actual <= max ? 0 : 1) }'; then
    printf '%s: host load1_per_cpu=%s exceeds NVPN_DOCKER_MAX_HOST_LOAD_PER_CPU=%s (load1=%s cpus=%s)\n' \
      "$log_prefix" "$load_per_cpu" "$max_load_per_cpu" "$load1" "$cpus" >&2
    return 1
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

docker_bench_phase_list_contains() {
  local list="$1"
  local phase="$2"
  [[ -n "$list" ]] || return 1
  local token
  for token in ${list//,/ }; do
    [[ "$token" == "all" || "$token" == "$phase" ]] && return 0
  done
  return 1
}

docker_bench_host_pids_for_service_process() {
  local service="$1"
  local process_name="$2"
  local cid
  cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
  [[ -n "$cid" ]] || return 0
  docker top "$cid" -eo pid,comm 2>/dev/null \
    | awk -v name="$process_name" 'NR > 1 && $2 == name { print $1 }'
}

docker_bench_start_phase_perf() {
  DOCKER_BENCH_PHASE_PERF_PID=""
  local artifact_prefix="$1"
  local process_name="$2"
  local phase="$3"
  local duration="$4"
  local raw_dir="$5"
  local perf_phases="$6"
  local perf_freq="$7"
  docker_bench_phase_list_contains "$perf_phases" "$phase" || return 0
  mkdir -p "$raw_dir/perf"

  local pids=()
  local service pid
  for service in node-a node-b; do
    while IFS= read -r pid; do
      [[ -n "$pid" ]] && pids+=("$pid")
    done < <(docker_bench_host_pids_for_service_process "$service" "$process_name")
  done
  if [[ "${#pids[@]}" == "0" ]]; then
    printf 'perf: no %s host PIDs found for phase %s\n' "$process_name" "$phase" \
      >"$raw_dir/perf/$artifact_prefix-$phase.log"
    return 0
  fi

  local pid_csv data_path log_path
  pid_csv="$(IFS=,; printf '%s' "${pids[*]}")"
  data_path="$raw_dir/perf/$artifact_prefix-$phase.data"
  log_path="$raw_dir/perf/$artifact_prefix-$phase.log"
  printf 'phase=%s\nprocess=%s\npids=%s\nfreq=%s\n' \
    "$phase" "$process_name" "$pid_csv" "$perf_freq" >"$log_path"
  sudo -n perf record -F "$perf_freq" -g -p "$pid_csv" -o "$data_path" \
    -- sleep "$((duration + 1))" >>"$log_path" 2>&1 &
  DOCKER_BENCH_PHASE_PERF_PID="$!"
}

docker_bench_finish_phase_perf() {
  local artifact_prefix="$1"
  local phase="$2"
  local raw_dir="$3"
  local perf_pid="$4"
  [[ -n "$perf_pid" ]] || return 0
  wait "$perf_pid" || true

  local data_path="$raw_dir/perf/$artifact_prefix-$phase.data"
  local report_path="$raw_dir/perf/$artifact_prefix-$phase-report.txt"
  local log_path="$raw_dir/perf/$artifact_prefix-$phase.log"
  [[ -s "$data_path" ]] || return 0
  sudo -n perf report --stdio --no-children --sort comm,dso,symbol \
    -i "$data_path" >"$report_path" 2>>"$log_path" || true
}

docker_bench_binary_linkage() {
  case "$1" in
    *"statically linked"* | *"not a dynamic executable"*) printf 'static\n' ;;
    *"libc.musl-"* | *"ld-musl-"*) printf 'dynamic-musl\n' ;;
    *"libc.so.6"* | *"ld-linux-"*) printf 'dynamic-glibc\n' ;;
    *) printf 'unknown\n' ;;
  esac
}

docker_bench_acquire_perf_lock() {
  local label="$1"
  local lock_path="/tmp/nostr-vpn-docker-perf.lock"
  local owner
  command -v flock >/dev/null 2>&1 || {
    printf '%s: missing required command: flock\n' "$label" >&2
    return 1
  }
  exec 9>>"$lock_path"
  if ! flock -n 9; then
    owner="$(cat "$lock_path" 2>/dev/null || true)"
    printf '%s: another Docker benchmark holds %s (pid %s)\n' \
      "$label" "$lock_path" "${owner:-unknown}" >&2
    return 1
  fi
  printf '%s\n' "$$" >"$lock_path"
}

docker_bench_runtime_service_provenance() {
  local service="$1"
  local backend="$2"
  local binary_name cid image_id binary_sha256 ldd_output binary_linkage
  local version_output nvpn_version fips_core_version online_cpus cpuset_cpus
  if ! declare -p COMPOSE >/dev/null 2>&1; then
    printf '{}\n'
    return
  fi
  cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
  if [[ -z "$cid" ]]; then
    printf '{}\n'
    return
  fi

  case "$backend" in
    wireguard-go) binary_name="wireguard-go" ;;
    boringtun) binary_name="boringtun-cli" ;;
    *) binary_name="nvpn" ;;
  esac
  image_id="$(docker inspect --format '{{.Image}}' "$cid" 2>/dev/null || true)"
  binary_sha256="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
    'sha256sum "$(command -v "$1")"' sh "$binary_name" 2>/dev/null || true)"
  binary_sha256="${binary_sha256%% *}"
  ldd_output="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
    'ldd "$(command -v "$1")" 2>&1' sh "$binary_name" 2>/dev/null || true)"
  binary_linkage="$(docker_bench_binary_linkage "$ldd_output")"
  version_output=""
  if [[ "$binary_name" == "nvpn" ]]; then
    version_output="$("${COMPOSE[@]}" exec -T "$service" nvpn version --verbose 2>/dev/null || true)"
  fi
  nvpn_version="${version_output%%$'\n'*}"
  fips_core_version="$(printf '%s\n' "$version_output" | sed -n 's/^fips_core_version: //p')"
  online_cpus="$("${COMPOSE[@]}" exec -T "$service" getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
  cpuset_cpus="$(docker inspect --format '{{.HostConfig.CpusetCpus}}' "$cid" 2>/dev/null || true)"

  jq -n \
    --arg image_id "$image_id" \
    --arg binary_name "$binary_name" \
    --arg binary_sha256 "$binary_sha256" \
    --arg binary_linkage "$binary_linkage" \
    --arg nvpn_version "$nvpn_version" \
    --arg fips_core_version "$fips_core_version" \
    --arg online_cpus "$online_cpus" \
    --arg cpuset_cpus "$cpuset_cpus" \
    '{
      image_id: (if $image_id == "" then null else $image_id end),
      binary_name: $binary_name,
      binary_sha256: (if $binary_sha256 == "" then null else $binary_sha256 end),
      binary_linkage: $binary_linkage,
      nvpn_version: (if $nvpn_version == "" then null else $nvpn_version end),
      fips_core_version: (if $fips_core_version == "" then null else $fips_core_version end),
      online_cpus: (if $online_cpus == "" then null else ($online_cpus | tonumber) end),
      cpuset_cpus: (if $cpuset_cpus == "" then null else $cpuset_cpus end)
    }'
}

docker_bench_write_metadata() {
  local backend="$1"
  local duration="$2"
  local iperf_directions="${3:-a_to_b}"
  local metadata_path="${OUTPUT_DIR}/metadata.json"
  local stress_enabled=0
  local local_workers=0
  local remote_workers=0
  local pipeline_trace_enabled=0
  local pipeline_trace_interval_secs=""
  local iperf_interval_secs="${NVPN_DOCKER_IPERF_INTERVAL_SECS:-0}"
  local iperf_timeout_secs="${NVPN_DOCKER_IPERF_TIMEOUT_SECS:-}"
  local perf_phases="${NVPN_DOCKER_PERF_PHASES:-}"
  local perf_freq="${NVPN_DOCKER_PERF_FREQ:-}"
  local iperf_udp1000_per_stream_bandwidth
  local dataplane_profile="${NVPN_DOCKER_DATAPLANE_PROFILE:-}"
  local placement_profile="${NVPN_DOCKER_PLACEMENT_PROFILE:-}"
  local extra_connect_env=""
  local require_no_direct_fmp=0
  local require_no_fsp_aead_helpers=0
  local require_no_pipeline_hard_events=0
  local allowed_pipeline_hard_events="${NVPN_DOCKER_ALLOW_PIPELINE_HARD_EVENTS:-}"
  local direct_fmp_forced=0
  local patch_local_fips_enabled=0
  local nvpn_git_head=""
  local nvpn_git_dirty=""
  local fips_git_head=""
  local fips_git_dirty=""
  local runtime_node_a runtime_node_b
  local nostr_identity_source="${NVPN_DOCKER_NOSTR_IDENTITY_SOURCE:-}"
  local node_id_source="${NVPN_DOCKER_NODE_ID_SOURCE:-}"
  local node_a_public_key="${NVPN_DOCKER_NODE_A_RUNTIME_PUBLIC_KEY:-${NVPN_DOCKER_NODE_A_NOSTR_PUBLIC_KEY_EFFECTIVE:-}}"
  local node_b_public_key="${NVPN_DOCKER_NODE_B_RUNTIME_PUBLIC_KEY:-${NVPN_DOCKER_NODE_B_NOSTR_PUBLIC_KEY_EFFECTIVE:-}}"
  local node_a_node_id="${NVPN_DOCKER_NODE_A_RUNTIME_NODE_ID:-${NVPN_DOCKER_NODE_A_ID_EFFECTIVE:-}}"
  local node_b_node_id="${NVPN_DOCKER_NODE_B_RUNTIME_NODE_ID:-${NVPN_DOCKER_NODE_B_ID_EFFECTIVE:-}}"
  local node_a_tunnel_ip="${NVPN_DOCKER_NODE_A_RUNTIME_TUNNEL_IP:-}"
  local node_b_tunnel_ip="${NVPN_DOCKER_NODE_B_RUNTIME_TUNNEL_IP:-}"
  local host_load1=""
  local host_load5=""
  local host_load15=""
  local host_online_cpus=""
  local host_load1_per_cpu=""
  local host_max_load1_per_cpu="${NVPN_DOCKER_MAX_HOST_LOAD_PER_CPU:-}"
  local expected_fsp_owner_placement="${NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT:-}"
  local expected_fsp_owner_placement_exclusive="${NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE:-}"
  local max_fsp_owner_placement_other_path_rate="${NVPN_DOCKER_MAX_FSP_OWNER_PLACEMENT_OTHER_PATH_RATE:-}"
  local placement_preflight="${NVPN_DOCKER_PLACEMENT_PREFLIGHT:-}"
  local placement_preflight_mode="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_MODE:-tcp}"
  local placement_preflight_duration="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_DURATION:-3}"
  local placement_preflight_streams="${NVPN_DOCKER_PLACEMENT_PREFLIGHT_STREAMS:-4}"
  local setup_ping_attempts="${NVPN_DOCKER_SETUP_PING_ATTEMPTS:-8}"
  local setup_ping_wait_secs="${NVPN_DOCKER_SETUP_PING_WAIT_SECS:-1}"
  local soak_max_ping_loss_pct=""
  local soak_max_ping_avg_ms=""
  local soak_max_ping_p95_ms=""
  local soak_max_ping_p99_ms=""
  local soak_max_ping_max_ms=""
  local soak_max_ping_avg_drift_ms=""
  local soak_max_ping_avg_drift_factor=""
  local soak_max_ping_p95_drift_ms=""
  local soak_max_ping_p95_drift_factor=""
  local soak_max_ping_p99_drift_ms=""
  local soak_max_ping_p99_drift_factor=""
  local soak_max_srtt_ms=""
  local soak_max_srtt_drift_ms=""
  local soak_max_srtt_drift_factor=""
  local soak_max_consecutive_high_srtt_samples=""
  local soak_max_fips_last_seen_age_secs=""
  local soak_max_fips_control_last_seen_age_secs=""
  local soak_max_fips_data_last_seen_age_secs=""
  local soak_max_fips_last_seen_future_skew_secs=""
  if [[ "$backend" == "fips-soak" ]]; then
    if [[ -z "$expected_fsp_owner_placement" ]]; then
      expected_fsp_owner_placement="${EXPECT_FSP_OWNER_PLACEMENT:-${NVPN_SOAK_EXPECT_FSP_OWNER_PLACEMENT:-}}"
    fi
    soak_max_ping_loss_pct="${MAX_PING_LOSS_PERCENT:-${NVPN_SOAK_MAX_PING_LOSS_PERCENT:-}}"
    soak_max_ping_avg_ms="${MAX_PING_AVG_MS:-${NVPN_SOAK_MAX_PING_AVG_MS:-}}"
    soak_max_ping_p95_ms="${MAX_PING_P95_MS:-${NVPN_SOAK_MAX_PING_P95_MS:-}}"
    soak_max_ping_p99_ms="${MAX_PING_P99_MS:-${NVPN_SOAK_MAX_PING_P99_MS:-}}"
    soak_max_ping_max_ms="${MAX_PING_MAX_MS:-${NVPN_SOAK_MAX_PING_MAX_MS:-}}"
    soak_max_ping_avg_drift_ms="${MAX_PING_AVG_DRIFT_MS:-${NVPN_SOAK_MAX_PING_AVG_DRIFT_MS:-}}"
    soak_max_ping_avg_drift_factor="${MAX_PING_AVG_DRIFT_FACTOR:-${NVPN_SOAK_MAX_PING_AVG_DRIFT_FACTOR:-}}"
    soak_max_ping_p95_drift_ms="${MAX_PING_P95_DRIFT_MS:-${NVPN_SOAK_MAX_PING_P95_DRIFT_MS:-}}"
    soak_max_ping_p95_drift_factor="${MAX_PING_P95_DRIFT_FACTOR:-${NVPN_SOAK_MAX_PING_P95_DRIFT_FACTOR:-}}"
    soak_max_ping_p99_drift_ms="${MAX_PING_P99_DRIFT_MS:-${NVPN_SOAK_MAX_PING_P99_DRIFT_MS:-}}"
    soak_max_ping_p99_drift_factor="${MAX_PING_P99_DRIFT_FACTOR:-${NVPN_SOAK_MAX_PING_P99_DRIFT_FACTOR:-}}"
    soak_max_srtt_ms="${MAX_SRTT_MS:-${NVPN_SOAK_MAX_SRTT_MS:-}}"
    soak_max_srtt_drift_ms="${MAX_SRTT_DRIFT_MS:-${NVPN_SOAK_MAX_SRTT_DRIFT_MS:-}}"
    soak_max_srtt_drift_factor="${MAX_SRTT_DRIFT_FACTOR:-${NVPN_SOAK_MAX_SRTT_DRIFT_FACTOR:-}}"
    soak_max_consecutive_high_srtt_samples="${MAX_CONSECUTIVE_HIGH_SRTT_SAMPLES:-${NVPN_SOAK_MAX_CONSECUTIVE_HIGH_SRTT_SAMPLES:-}}"
    soak_max_fips_last_seen_age_secs="${MAX_FIPS_LAST_SEEN_AGE_SECS:-${NVPN_SOAK_MAX_FIPS_LAST_SEEN_AGE_SECS:-}}"
    soak_max_fips_control_last_seen_age_secs="${MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS:-${NVPN_SOAK_MAX_FIPS_CONTROL_LAST_SEEN_AGE_SECS:-}}"
    soak_max_fips_data_last_seen_age_secs="${MAX_FIPS_DATA_LAST_SEEN_AGE_SECS:-${NVPN_SOAK_MAX_FIPS_DATA_LAST_SEEN_AGE_SECS:-}}"
    soak_max_fips_last_seen_future_skew_secs="${MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS:-${NVPN_SOAK_MAX_FIPS_LAST_SEEN_FUTURE_SKEW_SECS:-}}"
  fi
  if [[ "$placement_profile" == "worker-open" ]]; then
    if [[ -z "$expected_fsp_owner_placement" ]]; then
      expected_fsp_owner_placement="worker-open"
    fi
    if [[ -z "${NVPN_DOCKER_EXPECT_FSP_OWNER_PLACEMENT_EXCLUSIVE+x}" ]]; then
      expected_fsp_owner_placement_exclusive="1"
    fi
  fi
  expected_fsp_owner_placement_exclusive="${expected_fsp_owner_placement_exclusive:-0}"
  extra_connect_env="$(docker_bench_effective_extra_env)"
  if docker_bench_cpu_stress_enabled; then
    stress_enabled=1
    if docker_bench_cpu_stress_side_enabled local; then
      local_workers="$(docker_bench_cpu_stress_workers local)"
    fi
    if docker_bench_cpu_stress_side_enabled remote; then
      remote_workers="$(docker_bench_cpu_stress_workers remote)"
    fi
  fi
  iperf_udp1000_per_stream_bandwidth="$(docker_bench_udp1000_per_stream_bandwidth)"
  if docker_bench_bool_enabled "${NVPN_DOCKER_PIPELINE_TRACE:-0}"; then
    pipeline_trace_enabled=1
    pipeline_trace_interval_secs="${NVPN_DOCKER_PIPELINE_INTERVAL_SECS:-5}"
  fi
  if docker_bench_local_fips_patch_enabled; then
    patch_local_fips_enabled=1
  fi
  if docker_bench_bool_enabled "${NVPN_DOCKER_REQUIRE_NO_DIRECT_FMP:-0}"; then
    require_no_direct_fmp=1
  fi
  if docker_bench_bool_enabled "${NVPN_DOCKER_REQUIRE_NO_FSP_AEAD_HELPERS:-0}"; then
    require_no_fsp_aead_helpers=1
  fi
  if docker_bench_bool_enabled "${NVPN_DOCKER_REQUIRE_NO_PIPELINE_HARD_EVENTS:-0}"; then
    require_no_pipeline_hard_events=1
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
  IFS=$'\t' read -r host_load1 host_load5 host_load15 < <(docker_bench_host_loadavg) || true
  host_online_cpus="$(docker_bench_host_online_cpus)"
  host_load1_per_cpu="$(docker_bench_host_load_per_cpu "$host_load1" "$host_online_cpus")"
  runtime_node_a="$(docker_bench_runtime_service_provenance node-a "$backend")"
  runtime_node_b="$(docker_bench_runtime_service_provenance node-b "$backend")"
  local runtime_linkage_a runtime_linkage_b
  runtime_linkage_a="$(jq -r '.binary_linkage // ""' <<<"$runtime_node_a")"
  runtime_linkage_b="$(jq -r '.binary_linkage // ""' <<<"$runtime_node_b")"
  if [[ -n "$runtime_linkage_a" && -n "$runtime_linkage_b" \
      && "$runtime_linkage_a" != "$runtime_linkage_b" ]]; then
    printf 'docker bench metadata failed: node linkage mismatch: node-a=%s node-b=%s\n' \
      "$runtime_linkage_a" "$runtime_linkage_b" >&2
    return 1
  fi
  jq -n \
    --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg backend "$backend" \
    --arg duration_secs "$duration" \
    --arg host_load1 "$host_load1" \
    --arg host_load5 "$host_load5" \
    --arg host_load15 "$host_load15" \
    --arg host_online_cpus "$host_online_cpus" \
    --arg host_load1_per_cpu "$host_load1_per_cpu" \
    --arg host_max_load1_per_cpu "$host_max_load1_per_cpu" \
    --arg stress_enabled "$stress_enabled" \
    --arg stress_sides "$(docker_bench_cpu_stress_sides)" \
    --arg local_workers "$local_workers" \
    --arg remote_workers "$remote_workers" \
    --arg pipeline_trace_enabled "$pipeline_trace_enabled" \
    --arg pipeline_trace_interval_secs "$pipeline_trace_interval_secs" \
    --arg iperf_interval_secs "$iperf_interval_secs" \
    --arg iperf_timeout_secs "$iperf_timeout_secs" \
    --arg iperf_directions "$iperf_directions" \
    --arg perf_phases "$perf_phases" \
    --arg perf_freq "$perf_freq" \
    --arg iperf_socket_buffer "${NVPN_DOCKER_IPERF_SOCKET_BUFFER:-}" \
    --arg iperf_udp1000_parallel "${NVPN_DOCKER_UDP1000_PARALLEL:-}" \
    --arg iperf_udp1000_bandwidth "${NVPN_DOCKER_UDP1000_BANDWIDTH:-1G}" \
    --arg iperf_udp1000_per_stream_bandwidth "$iperf_udp1000_per_stream_bandwidth" \
    --arg dataplane_profile "$dataplane_profile" \
    --arg placement_profile "$placement_profile" \
    --arg extra_connect_env "$extra_connect_env" \
    --arg expected_fsp_owner_placement "$expected_fsp_owner_placement" \
    --arg expected_fsp_owner_placement_exclusive "$expected_fsp_owner_placement_exclusive" \
    --arg placement_preflight "$placement_preflight" \
    --arg placement_preflight_mode "$placement_preflight_mode" \
    --arg placement_preflight_duration "$placement_preflight_duration" \
    --arg placement_preflight_streams "$placement_preflight_streams" \
    --arg setup_ping_attempts "$setup_ping_attempts" \
    --arg setup_ping_wait_secs "$setup_ping_wait_secs" \
    --arg require_no_direct_fmp "$require_no_direct_fmp" \
    --arg require_no_fsp_aead_helpers "$require_no_fsp_aead_helpers" \
    --arg require_no_pipeline_hard_events "$require_no_pipeline_hard_events" \
    --arg allowed_pipeline_hard_events "$allowed_pipeline_hard_events" \
    --arg direct_fmp_forced "$direct_fmp_forced" \
    --arg patch_local_fips_enabled "$patch_local_fips_enabled" \
    --arg nvpn_git_head "$nvpn_git_head" \
    --arg nvpn_git_dirty "$nvpn_git_dirty" \
    --arg fips_git_head "$fips_git_head" \
    --arg fips_git_dirty "$fips_git_dirty" \
    --argjson runtime_node_a "$runtime_node_a" \
    --argjson runtime_node_b "$runtime_node_b" \
    --arg nostr_identity_source "$nostr_identity_source" \
    --arg node_id_source "$node_id_source" \
    --arg node_a_public_key "$node_a_public_key" \
    --arg node_b_public_key "$node_b_public_key" \
    --arg node_a_node_id "$node_a_node_id" \
    --arg node_b_node_id "$node_b_node_id" \
    --arg node_a_tunnel_ip "$node_a_tunnel_ip" \
    --arg node_b_tunnel_ip "$node_b_tunnel_ip" \
    --arg guard_min_tcp_mbps "${NVPN_DOCKER_MIN_TCP_MBPS:-}" \
    --arg guard_min_tcp_single_mbps "${NVPN_DOCKER_MIN_TCP_SINGLE_MBPS:-}" \
    --arg guard_min_tcp_4_mbps "${NVPN_DOCKER_MIN_TCP_4_MBPS:-}" \
    --arg guard_min_tcp_8_mbps "${NVPN_DOCKER_MIN_TCP_8_MBPS:-}" \
    --arg guard_min_udp200_mbps "${NVPN_DOCKER_MIN_UDP200_MBPS:-}" \
    --arg guard_min_udp1000_mbps "${NVPN_DOCKER_MIN_UDP1000_MBPS:-}" \
    --arg guard_max_tcp_retrans "${NVPN_DOCKER_MAX_TCP_RETRANS:-}" \
    --arg guard_max_tcp_single_retrans "${NVPN_DOCKER_MAX_TCP_SINGLE_RETRANS:-}" \
    --arg guard_max_tcp_4_retrans "${NVPN_DOCKER_MAX_TCP_4_RETRANS:-}" \
    --arg guard_max_tcp_8_retrans "${NVPN_DOCKER_MAX_TCP_8_RETRANS:-}" \
    --arg guard_max_udp_loss_pct "${NVPN_DOCKER_MAX_UDP_LOSS_PCT:-}" \
    --arg guard_max_udp200_loss_pct "${NVPN_DOCKER_MAX_UDP200_LOSS_PCT:-}" \
    --arg guard_max_udp1000_loss_pct "${NVPN_DOCKER_MAX_UDP1000_LOSS_PCT:-}" \
    --arg guard_max_ping_loss_pct "${NVPN_DOCKER_MAX_PING_LOSS_PCT:-$soak_max_ping_loss_pct}" \
    --arg guard_max_ping_avg_ms "$soak_max_ping_avg_ms" \
    --arg guard_max_ping_p95_ms "$soak_max_ping_p95_ms" \
    --arg guard_max_ping_p99_ms "$soak_max_ping_p99_ms" \
    --arg guard_max_ping_max_ms "$soak_max_ping_max_ms" \
    --arg guard_max_ping_avg_drift_ms "$soak_max_ping_avg_drift_ms" \
    --arg guard_max_ping_avg_drift_factor "$soak_max_ping_avg_drift_factor" \
    --arg guard_max_ping_p95_drift_ms "$soak_max_ping_p95_drift_ms" \
    --arg guard_max_ping_p95_drift_factor "$soak_max_ping_p95_drift_factor" \
    --arg guard_max_ping_p99_drift_ms "$soak_max_ping_p99_drift_ms" \
    --arg guard_max_ping_p99_drift_factor "$soak_max_ping_p99_drift_factor" \
    --arg guard_max_srtt_ms "$soak_max_srtt_ms" \
    --arg guard_max_srtt_drift_ms "$soak_max_srtt_drift_ms" \
    --arg guard_max_srtt_drift_factor "$soak_max_srtt_drift_factor" \
    --arg guard_max_consecutive_high_srtt_samples "$soak_max_consecutive_high_srtt_samples" \
    --arg guard_max_fips_last_seen_age_secs "$soak_max_fips_last_seen_age_secs" \
    --arg guard_max_fips_control_last_seen_age_secs "$soak_max_fips_control_last_seen_age_secs" \
    --arg guard_max_fips_data_last_seen_age_secs "$soak_max_fips_data_last_seen_age_secs" \
    --arg guard_max_fips_last_seen_future_skew_secs "$soak_max_fips_last_seen_future_skew_secs" \
    --arg guard_max_udp_kernel_dropped "${NVPN_DOCKER_MAX_UDP_KERNEL_DROPPED:-}" \
    --arg guard_max_udp_socket_kernel_dropped "${NVPN_DOCKER_MAX_UDP_SOCKET_KERNEL_DROPPED:-}" \
    --arg guard_max_udp_namespace_rcvbuf_errors "${NVPN_DOCKER_MAX_UDP_NAMESPACE_RCVBUF_ERRORS:-}" \
    --arg guard_max_connected_udp_kernel_dropped "${NVPN_DOCKER_MAX_CONNECTED_UDP_KERNEL_DROPPED:-}" \
    --arg guard_max_connected_udp_peer_kernel_dropped "${NVPN_DOCKER_MAX_CONNECTED_UDP_PEER_KERNEL_DROPPED:-}" \
    --arg guard_max_connected_udp_drain_bulk_dropped "${NVPN_DOCKER_MAX_CONNECTED_UDP_DRAIN_BULK_DROPPED:-}" \
    --arg guard_max_connected_udp_direct_decrypt_bulk_shed "${NVPN_DOCKER_MAX_CONNECTED_UDP_DIRECT_DECRYPT_BULK_SHED:-}" \
    --arg guard_max_endpoint_event_bulk_dropped "${NVPN_DOCKER_MAX_ENDPOINT_EVENT_BULK_DROPPED:-}" \
    --arg guard_max_tun_rx_dropped "${NVPN_DOCKER_MAX_TUN_RX_DROPPED:-}" \
    --arg guard_max_tun_tx_dropped "${NVPN_DOCKER_MAX_TUN_TX_DROPPED:-}" \
    --arg guard_max_decrypt_worker_queue_full "${NVPN_DOCKER_MAX_DECRYPT_WORKER_QUEUE_FULL:-}" \
    --arg guard_max_decrypt_worker_bulk_dropped "${NVPN_DOCKER_MAX_DECRYPT_WORKER_BULK_DROPPED:-}" \
    --arg guard_max_decrypt_fallback_pressure_drain "${NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRESSURE_DRAIN:-}" \
    --arg guard_max_decrypt_fallback_priority_gated "${NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRIORITY_GATED:-}" \
    --arg guard_max_decrypt_fsp_helper_window_fallback "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_WINDOW_FALLBACK:-}" \
    --arg guard_max_decrypt_fsp_open_worker_window_fallback "${NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_WINDOW_FALLBACK:-}" \
    --arg guard_max_decrypt_fsp_helper_queue_full_fallback "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_QUEUE_FULL_FALLBACK:-}" \
    --arg guard_max_decrypt_fsp_helper_completion_backlog_fallback "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_COMPLETION_BACKLOG_FALLBACK:-}" \
    --arg guard_max_decrypt_fsp_owner_handoff_dropped "${NVPN_DOCKER_MAX_DECRYPT_FSP_OWNER_HANDOFF_DROPPED:-}" \
    --arg guard_max_decrypt_fsp_open_worker_completion_backlog_fallback "${NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_COMPLETION_BACKLOG_FALLBACK:-}" \
    --arg guard_max_decrypt_fsp_worker_replay_dropped "${NVPN_DOCKER_MAX_DECRYPT_FSP_WORKER_REPLAY_DROPPED:-}" \
    --arg guard_max_fmp_aead_completion_aead_failed "${NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_AEAD_FAILED:-}" \
    --arg guard_max_fsp_aead_completion_aead_failed "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_AEAD_FAILED:-}" \
    --arg guard_max_fsp_aead_completion_epoch_mismatch "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_EPOCH_MISMATCH:-}" \
    --arg guard_max_fsp_aead_completion_stale_session "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_SESSION:-}" \
    --arg guard_max_fsp_aead_completion_stale_order "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_ORDER:-}" \
    --arg guard_max_fsp_aead_completion_stale_ticket "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_TICKET:-}" \
    --arg guard_max_fsp_aead_completion_duplicate_ticket "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_DUPLICATE_TICKET:-}" \
    --arg guard_max_fsp_aead_completion_window_exceeded "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_WINDOW_EXCEEDED:-}" \
    --arg guard_max_fmp_aead_completion_replay_dropped "${NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_REPLAY_DROPPED:-}" \
    --arg guard_max_fsp_aead_completion_replay_dropped "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED:-}" \
    --arg guard_max_fsp_aead_completion_replay_dropped_helper "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED_HELPER:-}" \
    --arg guard_max_fsp_aead_completion_replay_dropped_helper_returned "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED_HELPER_RETURNED:-}" \
    --arg guard_max_fsp_owner_placement_other_path_rate "$max_fsp_owner_placement_other_path_rate" \
    'def bool_or_null($v):
       if $v == "" then null
       elif ($v | test("^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$")) then true
       elif ($v | test("^(0|false|FALSE|False|no|NO|No|off|OFF|Off)$")) then false
       else null
       end;
     def string_or_null($v):
       if $v == "" then null else $v end;
     def number_or_null($v):
       if $v == "" then null
       elif ($v | test("^[0-9]+([.][0-9]+)?$")) then ($v | tonumber)
       else null
       end;
     {
      generated_at: $generated_at,
      backend: $backend,
      duration_secs: ($duration_secs | tonumber),
      host: {
        load1: number_or_null($host_load1),
        load5: number_or_null($host_load5),
        load15: number_or_null($host_load15),
        online_cpus: number_or_null($host_online_cpus),
        load1_per_cpu: number_or_null($host_load1_per_cpu),
        max_load1_per_cpu: number_or_null($host_max_load1_per_cpu)
      },
      run_env: {
        dataplane_profile: string_or_null($dataplane_profile),
        placement_profile: string_or_null($placement_profile),
        extra_connect_env: $extra_connect_env,
        expected_fsp_owner_placement: string_or_null($expected_fsp_owner_placement),
        expected_fsp_owner_placement_exclusive: bool_or_null($expected_fsp_owner_placement_exclusive),
        placement_preflight: string_or_null($placement_preflight),
        placement_preflight_mode: string_or_null($placement_preflight_mode),
        placement_preflight_duration_secs: (
          if $placement_preflight_duration == "" then null
          else ($placement_preflight_duration | tonumber)
          end
        ),
        placement_preflight_streams: (
          if $placement_preflight_streams == "" then null
          else ($placement_preflight_streams | tonumber)
          end
        ),
        setup_ping_attempts: (
          if $setup_ping_attempts == "" then null
          else ($setup_ping_attempts | tonumber)
          end
        ),
        setup_ping_wait_secs: (
          if $setup_ping_wait_secs == "" then null
          else ($setup_ping_wait_secs | tonumber)
          end
        ),
        require_no_direct_fmp: ($require_no_direct_fmp == "1"),
        require_no_fsp_aead_helpers: ($require_no_fsp_aead_helpers == "1"),
        require_no_pipeline_hard_events: ($require_no_pipeline_hard_events == "1"),
        allowed_pipeline_hard_events: string_or_null($allowed_pipeline_hard_events),
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
      perf: {
        phases: string_or_null($perf_phases),
        freq: (
          if $perf_freq == "" then null
          else ($perf_freq | tonumber)
          end
        )
      },
      iperf: {
        directions: (
          $iperf_directions | split(",") | map({
            id: .,
            sender: (if . == "b_to_a" then "node-b" else "node-a" end),
            receiver: (if . == "b_to_a" then "node-a" else "node-b" end),
            iperf_reverse: (. == "b_to_a")
          })
        ),
        interval_secs: ($iperf_interval_secs | tonumber),
        timeout_secs: (
          if $iperf_timeout_secs == "" then null
          else ($iperf_timeout_secs | tonumber)
          end
        ),
        socket_buffer: string_or_null($iperf_socket_buffer),
        udp1000_parallel: (
          if $iperf_udp1000_parallel == "" then null
          else ($iperf_udp1000_parallel | tonumber)
          end
        ),
        udp1000_bandwidth: string_or_null($iperf_udp1000_bandwidth),
        udp1000_per_stream_bandwidth: string_or_null($iperf_udp1000_per_stream_bandwidth)
      },
      runtime_identity: {
        nostr_identity_source: string_or_null($nostr_identity_source),
        node_id_source: string_or_null($node_id_source),
        node_a: {
          public_key: string_or_null($node_a_public_key),
          node_id: string_or_null($node_a_node_id),
          tunnel_ip: string_or_null($node_a_tunnel_ip)
        },
        node_b: {
          public_key: string_or_null($node_b_public_key),
          node_id: string_or_null($node_b_node_id),
          tunnel_ip: string_or_null($node_b_tunnel_ip)
        }
      },
      guard_thresholds: {
        min_tcp_mbps: string_or_null($guard_min_tcp_mbps),
        min_tcp_single_mbps: string_or_null($guard_min_tcp_single_mbps),
        min_tcp_4_mbps: string_or_null($guard_min_tcp_4_mbps),
        min_tcp_8_mbps: string_or_null($guard_min_tcp_8_mbps),
        min_udp200_mbps: string_or_null($guard_min_udp200_mbps),
        min_udp1000_mbps: string_or_null($guard_min_udp1000_mbps),
        max_tcp_retrans: string_or_null($guard_max_tcp_retrans),
        max_tcp_single_retrans: string_or_null($guard_max_tcp_single_retrans),
        max_tcp_4_retrans: string_or_null($guard_max_tcp_4_retrans),
        max_tcp_8_retrans: string_or_null($guard_max_tcp_8_retrans),
        max_udp_loss_pct: string_or_null($guard_max_udp_loss_pct),
        max_udp200_loss_pct: string_or_null($guard_max_udp200_loss_pct),
        max_udp1000_loss_pct: string_or_null($guard_max_udp1000_loss_pct),
        max_ping_loss_pct: string_or_null($guard_max_ping_loss_pct),
        max_ping_avg_ms: string_or_null($guard_max_ping_avg_ms),
        max_ping_p95_ms: string_or_null($guard_max_ping_p95_ms),
        max_ping_p99_ms: string_or_null($guard_max_ping_p99_ms),
        max_ping_max_ms: string_or_null($guard_max_ping_max_ms),
        max_ping_avg_drift_ms: string_or_null($guard_max_ping_avg_drift_ms),
        max_ping_avg_drift_factor: string_or_null($guard_max_ping_avg_drift_factor),
        max_ping_p95_drift_ms: string_or_null($guard_max_ping_p95_drift_ms),
        max_ping_p95_drift_factor: string_or_null($guard_max_ping_p95_drift_factor),
        max_ping_p99_drift_ms: string_or_null($guard_max_ping_p99_drift_ms),
        max_ping_p99_drift_factor: string_or_null($guard_max_ping_p99_drift_factor),
        max_srtt_ms: string_or_null($guard_max_srtt_ms),
        max_srtt_drift_ms: string_or_null($guard_max_srtt_drift_ms),
        max_srtt_drift_factor: string_or_null($guard_max_srtt_drift_factor),
        max_consecutive_high_srtt_samples: string_or_null($guard_max_consecutive_high_srtt_samples),
        max_fips_last_seen_age_secs: string_or_null($guard_max_fips_last_seen_age_secs),
        max_fips_control_last_seen_age_secs: string_or_null($guard_max_fips_control_last_seen_age_secs),
        max_fips_data_last_seen_age_secs: string_or_null($guard_max_fips_data_last_seen_age_secs),
        max_fips_last_seen_future_skew_secs: string_or_null($guard_max_fips_last_seen_future_skew_secs),
        max_udp_kernel_dropped: string_or_null($guard_max_udp_kernel_dropped),
        max_udp_socket_kernel_dropped: string_or_null($guard_max_udp_socket_kernel_dropped),
        max_udp_namespace_rcvbuf_errors: string_or_null($guard_max_udp_namespace_rcvbuf_errors),
        max_connected_udp_kernel_dropped: string_or_null($guard_max_connected_udp_kernel_dropped),
        max_connected_udp_peer_kernel_dropped: string_or_null($guard_max_connected_udp_peer_kernel_dropped),
        max_connected_udp_drain_bulk_dropped: string_or_null($guard_max_connected_udp_drain_bulk_dropped),
        max_connected_udp_direct_decrypt_bulk_shed: string_or_null($guard_max_connected_udp_direct_decrypt_bulk_shed),
        max_endpoint_event_bulk_dropped: string_or_null($guard_max_endpoint_event_bulk_dropped),
        max_tun_rx_dropped: string_or_null($guard_max_tun_rx_dropped),
        max_tun_tx_dropped: string_or_null($guard_max_tun_tx_dropped),
        max_decrypt_worker_queue_full: string_or_null($guard_max_decrypt_worker_queue_full),
        max_decrypt_worker_bulk_dropped: string_or_null($guard_max_decrypt_worker_bulk_dropped),
        max_decrypt_fallback_pressure_drain: string_or_null($guard_max_decrypt_fallback_pressure_drain),
        max_decrypt_fallback_priority_gated: string_or_null($guard_max_decrypt_fallback_priority_gated),
        max_decrypt_fsp_helper_window_fallback: string_or_null($guard_max_decrypt_fsp_helper_window_fallback),
        max_decrypt_fsp_open_worker_window_fallback: string_or_null($guard_max_decrypt_fsp_open_worker_window_fallback),
        max_decrypt_fsp_helper_queue_full_fallback: string_or_null($guard_max_decrypt_fsp_helper_queue_full_fallback),
        max_decrypt_fsp_helper_completion_backlog_fallback: string_or_null($guard_max_decrypt_fsp_helper_completion_backlog_fallback),
        max_decrypt_fsp_owner_handoff_dropped: string_or_null($guard_max_decrypt_fsp_owner_handoff_dropped),
        max_decrypt_fsp_open_worker_completion_backlog_fallback: string_or_null($guard_max_decrypt_fsp_open_worker_completion_backlog_fallback),
        max_decrypt_fsp_worker_replay_dropped: string_or_null($guard_max_decrypt_fsp_worker_replay_dropped),
        max_fmp_aead_completion_aead_failed: string_or_null($guard_max_fmp_aead_completion_aead_failed),
        max_fsp_aead_completion_aead_failed: string_or_null($guard_max_fsp_aead_completion_aead_failed),
        max_fsp_aead_completion_epoch_mismatch: string_or_null($guard_max_fsp_aead_completion_epoch_mismatch),
        max_fsp_aead_completion_stale_session: string_or_null($guard_max_fsp_aead_completion_stale_session),
        max_fsp_aead_completion_stale_order: string_or_null($guard_max_fsp_aead_completion_stale_order),
        max_fsp_aead_completion_stale_ticket: string_or_null($guard_max_fsp_aead_completion_stale_ticket),
        max_fsp_aead_completion_duplicate_ticket: string_or_null($guard_max_fsp_aead_completion_duplicate_ticket),
        max_fsp_aead_completion_window_exceeded: string_or_null($guard_max_fsp_aead_completion_window_exceeded),
        max_fmp_aead_completion_replay_dropped: string_or_null($guard_max_fmp_aead_completion_replay_dropped),
        max_fsp_aead_completion_replay_dropped: string_or_null($guard_max_fsp_aead_completion_replay_dropped),
        max_fsp_aead_completion_replay_dropped_helper: string_or_null($guard_max_fsp_aead_completion_replay_dropped_helper),
        max_fsp_aead_completion_replay_dropped_helper_returned: string_or_null($guard_max_fsp_aead_completion_replay_dropped_helper_returned),
        max_fsp_owner_placement_other_path_rate: string_or_null($guard_max_fsp_owner_placement_other_path_rate)
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
        },
        runtime: {
          node_a: $runtime_node_a,
          node_b: $runtime_node_b
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

docker_bench_iperf_transfer_bytes() {
  jq -r '(.end.sum_received.bytes // .end.sum.bytes // .end.sum_sent.bytes // 0)' "$1"
}

docker_bench_iperf_loss_pct() {
  docker_bench_json_number '(.end.sum.lost_percent // .end.sum_received.lost_percent // 0)' "$1"
}

docker_bench_iperf_optional() {
  local parser="$1"
  local path="$2"
  [[ -n "$path" ]] && "$parser" "$path"
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

docker_bench_parse_ping_tail_stats() {
  local file="$1"
  local IFS=$' \t\n'
  local mdev max count p95 p99 gt1 gt2 gt10
  read -r max mdev <<<"$(
    awk -F'= ' '/round-trip|rtt min/ {split($2,a,"/"); split(a[4], b, " "); print a[3], b[1]}' "$file" \
      | tail -n1
  )"
  read -r count p95 p99 gt1 gt2 gt10 <<<"$(
    sed -nE 's/.*time[=<][[:space:]]*([0-9.]+).*/\1/p' "$file" \
      | sort -n \
      | awk '
        NF {
          v[++n] = $1
          if ($1 > 1) gt1 += 1
          if ($1 > 2) gt2 += 1
          if ($1 > 10) gt10 += 1
        }
        END {
          if (!n) {
            print "0 null null 0 0 0"
          } else {
            p95_idx = int((n * 95 + 99) / 100)
            p99_idx = int((n * 99 + 99) / 100)
            if (p95_idx < 1) p95_idx = 1
            if (p99_idx < 1) p99_idx = 1
            if (p95_idx > n) p95_idx = n
            if (p99_idx > n) p99_idx = n
            printf "%d %.3f %.3f %d %d %d\n", n, v[p95_idx], v[p99_idx], gt1 + 0, gt2 + 0, gt10 + 0
          }
        }'
  )"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${mdev:-null}" "${p95:-null}" "${p99:-null}" "${max:-null}" "${count:-0}" "${gt1:-0}" "${gt2:-0}" "${gt10:-0}"
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
  local tcp_single_reverse_json="${11:-}"
  local tcp_4_reverse_json="${12:-}"
  local tcp_8_reverse_json="${13:-}"
  local udp_200_reverse_json="${14:-}"
  local udp_1000_reverse_json="${15:-}"
  local ping_loss ping_avg
  local stress_enabled=false local_workers=0 remote_workers=0
  local udp1000_parallel udp1000_per_stream_bandwidth
  local reverse_direction=""
  local ping_mdev ping_p95 ping_p99 ping_max ping_samples ping_gt1 ping_gt2 ping_gt10 ping_tail_stats

  read -r ping_loss ping_avg <<<"$(docker_bench_parse_ping_loss_avg "$ping_output")"
  ping_tail_stats="$(docker_bench_parse_ping_tail_stats "$ping_output")"
  IFS=$'\t' read -r ping_mdev ping_p95 ping_p99 ping_max ping_samples ping_gt1 ping_gt2 ping_gt10 \
    <<<"$ping_tail_stats"
  if docker_bench_cpu_stress_enabled; then
    stress_enabled=true
    if docker_bench_cpu_stress_side_enabled local; then
      local_workers="$(docker_bench_cpu_stress_workers local)"
    fi
    if docker_bench_cpu_stress_side_enabled remote; then
      remote_workers="$(docker_bench_cpu_stress_workers remote)"
    fi
  fi
  udp1000_parallel="$(docker_bench_udp1000_parallel_streams)"
  udp1000_per_stream_bandwidth="$(docker_bench_udp1000_per_stream_bandwidth)"
  [[ -z "$tcp_single_reverse_json" ]] || reverse_direction=b_to_a

  docker_bench_write_tsv_row \
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
    "$raw_dir" \
    "$stress_enabled" \
    "$(docker_bench_tsv_field "$(docker_bench_cpu_stress_sides)")" \
    "$local_workers" \
    "$remote_workers" \
    "$(docker_bench_tsv_field "${NVPN_DOCKER_IPERF_SOCKET_BUFFER:-}")" \
    "$udp1000_parallel" \
    "$(docker_bench_tsv_field "${NVPN_DOCKER_UDP1000_BANDWIDTH:-1G}")" \
    "$udp1000_per_stream_bandwidth" \
    "$(docker_bench_tsv_field "${NVPN_DOCKER_DATAPLANE_PROFILE:-}")" \
    "$(docker_bench_tsv_field "${NVPN_DOCKER_PLACEMENT_PROFILE:-}")" \
    "$ping_mdev" \
    "$ping_p95" \
    "$ping_p99" \
    "$ping_max" \
    "$ping_samples" \
    "$ping_gt1" \
    "$ping_gt2" \
    "$ping_gt10" \
    a_to_b \
    "$reverse_direction" \
    "$(docker_bench_iperf_optional docker_bench_iperf_mbps "$tcp_single_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_retrans "$tcp_single_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_mbps "$tcp_4_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_retrans "$tcp_4_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_mbps "$tcp_8_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_retrans "$tcp_8_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_mbps "$udp_200_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_loss_pct "$udp_200_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_mbps "$udp_1000_reverse_json")" \
    "$(docker_bench_iperf_optional docker_bench_iperf_loss_pct "$udp_1000_reverse_json")" >>"$SUMMARY_TSV"
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

docker_bench_cpu_seconds_from_jiffies() {
  local start_jiffies="$1"
  local end_jiffies="$2"
  local clk_tck="$3"
  if [[ ! "$start_jiffies" =~ ^[0-9]+$ ]] \
    || [[ ! "$end_jiffies" =~ ^[0-9]+$ ]] \
    || [[ ! "$clk_tck" =~ ^[1-9][0-9]*$ ]] \
    || (( end_jiffies < start_jiffies )); then
    return 0
  fi
  awk -v start="$start_jiffies" -v end="$end_jiffies" -v clk="$clk_tck" \
    'BEGIN { printf "%.6f", (end - start) / clk }'
}

docker_bench_cpu_seconds_per_gbyte() {
  local cpu_seconds="$1"
  local transfer_bytes="$2"
  if ! docker_bench_is_number "$cpu_seconds" \
    || [[ ! "$transfer_bytes" =~ ^[0-9]+$ ]] \
    || (( transfer_bytes <= 0 )); then
    return 0
  fi
  awk -v cpu="$cpu_seconds" -v bytes="$transfer_bytes" \
    'BEGIN { printf "%.6f", cpu / (bytes / 1000000000) }'
}

docker_bench_write_cpu_phase_header() {
  local output_path="$1"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    phase service pid_start pid_end \
    cpu_jiffies_start cpu_jiffies_end clk_tck \
    cpu_seconds transfer_bytes cpu_seconds_per_gbyte \
    >"$output_path"
}

docker_bench_process_cpu_sample() {
  local service="$1"
  local process_name="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -lc '
    process_name="$1"
    pids="$(pgrep -x "$process_name" 2>/dev/null | sort -n || true)"
    clk="$(getconf CLK_TCK 2>/dev/null || printf 100)"
    if [ -z "$pids" ]; then
      printf "na\tna\t%s\n" "${clk:-100}"
      exit 0
    fi
    pid_list=""
    total_jiffies=0
    for pid in $pids; do
      [ -r "/proc/$pid/stat" ] || continue
      jiffies="$(awk "{ print \$14 + \$15 }" "/proc/$pid/stat" 2>/dev/null || true)"
      case "$jiffies" in
        ""|*[!0-9]*) continue ;;
      esac
      pid_list="${pid_list:+$pid_list,}$pid"
      total_jiffies=$((total_jiffies + jiffies))
    done
    if [ -n "$pid_list" ]; then
      printf "%s\t%s\t%s\n" "$pid_list" "$total_jiffies" "${clk:-100}"
    else
      printf "na\tna\t%s\n" "${clk:-100}"
    fi
  ' sh "$process_name" 2>/dev/null | tr -d '\r' || printf 'na\tna\t100\n'
}

docker_bench_cpu_sample_cpu_seconds() {
  local start_sample="$1"
  local end_sample="$2"
  local start_pid start_jiffies start_clk end_pid end_jiffies end_clk clk_tck
  IFS=$'\t' read -r start_pid start_jiffies start_clk <<<"$start_sample"
  IFS=$'\t' read -r end_pid end_jiffies end_clk <<<"$end_sample"
  if [[ "$start_pid" == "na" || "$start_pid" != "$end_pid" ]]; then
    return 0
  fi
  clk_tck="$end_clk"
  [[ "$clk_tck" =~ ^[1-9][0-9]*$ ]] || clk_tck="$start_clk"
  docker_bench_cpu_seconds_from_jiffies "$start_jiffies" "$end_jiffies" "$clk_tck"
}

docker_bench_append_cpu_phase_service_row() {
  local output_path="$1"
  local phase="$2"
  local service="$3"
  local transfer_bytes="$4"
  local start_sample="$5"
  local end_sample="$6"
  local start_pid start_jiffies start_clk end_pid end_jiffies end_clk clk_tck
  local cpu_seconds cpu_per_gbyte
  IFS=$'\t' read -r start_pid start_jiffies start_clk <<<"$start_sample"
  IFS=$'\t' read -r end_pid end_jiffies end_clk <<<"$end_sample"
  clk_tck="$end_clk"
  [[ "$clk_tck" =~ ^[1-9][0-9]*$ ]] || clk_tck="$start_clk"
  if [[ "$start_pid" != "na" && "$start_pid" == "$end_pid" ]]; then
    cpu_seconds="$(docker_bench_cpu_seconds_from_jiffies "$start_jiffies" "$end_jiffies" "$clk_tck")"
  else
    cpu_seconds=""
  fi
  cpu_per_gbyte="$(docker_bench_cpu_seconds_per_gbyte "$cpu_seconds" "$transfer_bytes")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$phase" \
    "$service" \
    "$(docker_bench_tsv_field "${start_pid:-na}")" \
    "$(docker_bench_tsv_field "${end_pid:-na}")" \
    "$(docker_bench_tsv_field "${start_jiffies:-na}")" \
    "$(docker_bench_tsv_field "${end_jiffies:-na}")" \
    "$(docker_bench_tsv_field "${clk_tck:-na}")" \
    "$cpu_seconds" \
    "$(docker_bench_tsv_field "$transfer_bytes")" \
    "$cpu_per_gbyte" >>"$output_path"
}

docker_bench_append_cpu_phase_rows() {
  local output_path="$1"
  local phase="$2"
  local transfer_bytes="$3"
  local start_a="$4"
  local start_b="$5"
  local end_a="$6"
  local end_b="$7"
  local cpu_a cpu_b cpu_both cpu_per_gbyte
  docker_bench_append_cpu_phase_service_row "$output_path" "$phase" node-a "$transfer_bytes" "$start_a" "$end_a"
  docker_bench_append_cpu_phase_service_row "$output_path" "$phase" node-b "$transfer_bytes" "$start_b" "$end_b"
  cpu_a="$(docker_bench_cpu_sample_cpu_seconds "$start_a" "$end_a")"
  cpu_b="$(docker_bench_cpu_sample_cpu_seconds "$start_b" "$end_b")"
  if docker_bench_is_number "$cpu_a" && docker_bench_is_number "$cpu_b"; then
    cpu_both="$(awk -v a="$cpu_a" -v b="$cpu_b" 'BEGIN { printf "%.6f", a + b }')"
  else
    cpu_both=""
  fi
  cpu_per_gbyte="$(docker_bench_cpu_seconds_per_gbyte "$cpu_both" "$transfer_bytes")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$phase" both na na na na na \
    "$cpu_both" \
    "$(docker_bench_tsv_field "$transfer_bytes")" \
    "$cpu_per_gbyte" >>"$output_path"
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
  local reverse_direction tcp_single_reverse tcp_single_reverse_retrans
  local tcp_4_reverse tcp_4_reverse_retrans tcp_8_reverse tcp_8_reverse_retrans
  local udp_200_reverse udp_200_reverse_loss udp_1000_reverse udp_1000_reverse_loss
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
  reverse_direction="$(tsv_value 36)"
  tcp_single_reverse="$(tsv_value 37)"
  tcp_single_reverse_retrans="$(tsv_value 38)"
  tcp_4_reverse="$(tsv_value 39)"
  tcp_4_reverse_retrans="$(tsv_value 40)"
  tcp_8_reverse="$(tsv_value 41)"
  tcp_8_reverse_retrans="$(tsv_value 42)"
  udp_200_reverse="$(tsv_value 43)"
  udp_200_reverse_loss="$(tsv_value 44)"
  udp_1000_reverse="$(tsv_value 45)"
  udp_1000_reverse_loss="$(tsv_value 46)"

  mkdir -p "$OUTPUT_DIR"
  rm -f "$failure_path"
  failure_count=0

  docker_bench_guard_check_at_least "$failure_path" "tcp_single_mbps" "$tcp_single" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_SINGLE_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_least "$failure_path" "tcp_4_mbps" "$tcp_4" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_4_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_least "$failure_path" "tcp_8_mbps" "$tcp_8" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_8_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_least "$failure_path" "udp_200_mbps" "$udp_200" "${NVPN_DOCKER_MIN_UDP200_MBPS:-}" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_least "$failure_path" "udp_1000_mbps" "$udp_1000" "${NVPN_DOCKER_MIN_UDP1000_MBPS:-}" || failure_count=$((failure_count + 1))

  docker_bench_guard_check_at_most "$failure_path" "tcp_single_retrans" "$tcp_single_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_SINGLE_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_most "$failure_path" "tcp_4_retrans" "$tcp_4_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_4_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_most "$failure_path" "tcp_8_retrans" "$tcp_8_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_8_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))

  docker_bench_guard_check_at_most "$failure_path" "udp_200_loss_pct" "$udp_200_loss" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_UDP200_LOSS_PCT NVPN_DOCKER_MAX_UDP_LOSS_PCT)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_most "$failure_path" "udp_1000_loss_pct" "$udp_1000_loss" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_UDP1000_LOSS_PCT NVPN_DOCKER_MAX_UDP_LOSS_PCT)" || failure_count=$((failure_count + 1))
  docker_bench_guard_check_at_most "$failure_path" "ping_loss_pct" "$ping_loss" "${NVPN_DOCKER_MAX_PING_LOSS_PCT:-}" || failure_count=$((failure_count + 1))

  if [[ "$reverse_direction" == "b_to_a" ]]; then
    docker_bench_guard_check_at_least "$failure_path" "tcp_single_b_to_a_mbps" "$tcp_single_reverse" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_SINGLE_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_least "$failure_path" "tcp_4_b_to_a_mbps" "$tcp_4_reverse" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_4_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_least "$failure_path" "tcp_8_b_to_a_mbps" "$tcp_8_reverse" "$(docker_bench_guard_threshold NVPN_DOCKER_MIN_TCP_8_MBPS NVPN_DOCKER_MIN_TCP_MBPS)" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_least "$failure_path" "udp_200_b_to_a_mbps" "$udp_200_reverse" "${NVPN_DOCKER_MIN_UDP200_MBPS:-}" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_least "$failure_path" "udp_1000_b_to_a_mbps" "$udp_1000_reverse" "${NVPN_DOCKER_MIN_UDP1000_MBPS:-}" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_most "$failure_path" "tcp_single_b_to_a_retrans" "$tcp_single_reverse_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_SINGLE_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_most "$failure_path" "tcp_4_b_to_a_retrans" "$tcp_4_reverse_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_4_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_most "$failure_path" "tcp_8_b_to_a_retrans" "$tcp_8_reverse_retrans" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_TCP_8_RETRANS NVPN_DOCKER_MAX_TCP_RETRANS)" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_most "$failure_path" "udp_200_b_to_a_loss_pct" "$udp_200_reverse_loss" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_UDP200_LOSS_PCT NVPN_DOCKER_MAX_UDP_LOSS_PCT)" || failure_count=$((failure_count + 1))
    docker_bench_guard_check_at_most "$failure_path" "udp_1000_b_to_a_loss_pct" "$udp_1000_reverse_loss" "$(docker_bench_guard_threshold NVPN_DOCKER_MAX_UDP1000_LOSS_PCT NVPN_DOCKER_MAX_UDP_LOSS_PCT)" || failure_count=$((failure_count + 1))
  fi

  if (( failure_count > 0 )); then
    printf 'docker bench guard failed: wrote %s\n' "$failure_path" >&2
    return 1
  fi
}

docker_bench_tun_drop_total() {
  local tun_summary="$1"
  local column_name="$2"
  [[ -s "$tun_summary" ]] || {
    printf 'missing\n'
    return 0
  }
  awk -F '\t' -v column_name="$column_name" '
    NR == 1 {
      for (i = 1; i <= NF; i++) {
        if ($i == column_name) {
          column = i
          break
        }
      }
      next
    }
    column > 0 && $column != "" {
      rows += 1
      total += $column + 0
    }
    END {
      if (column == 0 || rows == 0) {
        print "missing"
      } else {
        print total + 0
      }
    }
  ' "$tun_summary"
}

docker_bench_assert_tun_drop_guards() {
  local tun_summary="${1:-$RAW_DIR/nvpn-linux-tun-netdev.tsv}"
  local failure_path="${OUTPUT_DIR}/guard-failures.tsv"
  local failure_count=0
  local threshold rx_drops tx_drops

  if [[ -z "${NVPN_DOCKER_MAX_TUN_RX_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_TUN_TX_DROPPED:-}" ]]; then
    return 0
  fi

  threshold="${NVPN_DOCKER_MAX_TUN_RX_DROPPED:-}"
  if [[ -n "$threshold" ]]; then
    rx_drops="$(docker_bench_tun_drop_total "$tun_summary" rx_dropped)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "tun_rx_dropped_total" \
      "$rx_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  threshold="${NVPN_DOCKER_MAX_TUN_TX_DROPPED:-}"
  if [[ -n "$threshold" ]]; then
    tx_drops="$(docker_bench_tun_drop_total "$tun_summary" tx_dropped)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "tun_tx_dropped_total" \
      "$tx_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  if (( failure_count > 0 )); then
    printf 'docker bench guard failed: wrote %s\n' "$failure_path" >&2
    return 1
  fi
}

docker_bench_pipeline_hard_event_total() {
  local phase_summary="$1"
  local event_name="$2"
  [[ -s "$phase_summary" ]] || {
    printf '0\n'
    return 0
  }
  awk -F '\t' -v event_name="$event_name" '
    NR == 1 {
      for (i = 1; i <= NF; i++) {
        if ($i == "hard_events") {
          hard_events_idx = i
          break
        }
      }
      next
    }
    hard_events_idx > 0 {
      event_count = split($hard_events_idx, events, ";")
      for (i = 1; i <= event_count; i++) {
        split(events[i], event_parts, ":")
        if (event_parts[1] != event_name) {
          continue
        }
        metric_count = split(event_parts[2], metrics, ",")
        for (j = 1; j <= metric_count; j++) {
          split(metrics[j], kv, "=")
          if (kv[1] == "total") {
            total += kv[2] + 0
          }
        }
      }
    }
    END {
      print total + 0
    }
  ' "$phase_summary"
}

docker_bench_write_pipeline_hard_event_totals() {
  local phase_summary="$1"
  local output_path="$2"
  printf '%s\t%s\t%s\n' event max_rate_per_sec total >"$output_path"
  [[ -s "$phase_summary" ]] || return 0
  awk -F '\t' '
    NR == 1 {
      for (i = 1; i <= NF; i++) {
        if ($i == "hard_events") {
          hard_events_idx = i
          break
        }
      }
      next
    }
    hard_events_idx > 0 {
      event_count = split($hard_events_idx, events, ";")
      for (i = 1; i <= event_count; i++) {
        split(events[i], event_parts, ":")
        name = event_parts[1]
        if (name == "") {
          continue
        }
        metric_count = split(event_parts[2], metrics, ",")
        for (j = 1; j <= metric_count; j++) {
          split(metrics[j], kv, "=")
          if (kv[1] == "max_rate_per_sec" && kv[2] + 0 > max_rate[name]) {
            max_rate[name] = kv[2] + 0
          } else if (kv[1] == "total") {
            total[name] += kv[2] + 0
          }
        }
        seen[name] = 1
      }
    }
    END {
      for (name in seen) {
        printf "%s\t%g\t%g\n", name, max_rate[name] + 0, total[name] + 0
      }
    }
  ' "$phase_summary" | LC_ALL=C sort >>"$output_path"
}

docker_bench_guard_pipeline_hard_event_at_most() {
  local phase_summary="$1"
  local failure_path="$2"
  local event_name="$3"
  local label="$4"
  local threshold="$5"
  local total

  [[ -n "$threshold" ]] || return 0
  total="$(docker_bench_pipeline_hard_event_total "$phase_summary" "$event_name")"
  docker_bench_guard_check_at_most \
    "$failure_path" \
    "$label" \
    "$total" \
    "$threshold"
}

docker_bench_pipeline_hard_event_allowed() {
  local event="$1"
  local token
  local allowed="${NVPN_DOCKER_ALLOW_PIPELINE_HARD_EVENTS:-}"
  allowed="${allowed//,/ }"
  for token in $allowed; do
    [[ "$event" == "$token" ]] && return 0
  done
  return 1
}

docker_bench_assert_pipeline_hard_event_guards() {
  local phase_summary="$1"
  local failure_path="${OUTPUT_DIR}/guard-failures.tsv"
  local failure_count=0
  local require_no_pipeline_hard_events=0
  local threshold peer_drops global_drops socket_drops namespace_drops bulk_drops
  local completion_backlog_fallbacks fsp_worker_replay_drops
  local event max_rate total totals_path

  if docker_bench_bool_enabled "${NVPN_DOCKER_REQUIRE_NO_PIPELINE_HARD_EVENTS:-0}"; then
    require_no_pipeline_hard_events=1
  fi

  if [[ "$require_no_pipeline_hard_events" != "1" &&
        -z "${NVPN_DOCKER_MAX_UDP_KERNEL_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_UDP_SOCKET_KERNEL_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_UDP_NAMESPACE_RCVBUF_ERRORS:-}" &&
        -z "${NVPN_DOCKER_MAX_CONNECTED_UDP_KERNEL_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_CONNECTED_UDP_PEER_KERNEL_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_CONNECTED_UDP_DRAIN_BULK_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_CONNECTED_UDP_DIRECT_DECRYPT_BULK_SHED:-}" &&
        -z "${NVPN_DOCKER_MAX_ENDPOINT_EVENT_BULK_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_WORKER_QUEUE_FULL:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_WORKER_BULK_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRESSURE_DRAIN:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRIORITY_GATED:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_WINDOW_FALLBACK:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_WINDOW_FALLBACK:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_QUEUE_FULL_FALLBACK:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_COMPLETION_BACKLOG_FALLBACK:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_COMPLETION_BACKLOG_FALLBACK:-}" &&
        -z "${NVPN_DOCKER_MAX_DECRYPT_FSP_WORKER_REPLAY_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_AEAD_FAILED:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_AEAD_FAILED:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_EPOCH_MISMATCH:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_SESSION:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_ORDER:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_TICKET:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_DUPLICATE_TICKET:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_WINDOW_EXCEEDED:-}" &&
        -z "${NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_REPLAY_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED_HELPER:-}" &&
        -z "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED_HELPER_RETURNED:-}" ]]; then
    return 0
  fi

  if [[ ! -s "$phase_summary" ]]; then
    docker_bench_guard_record_failure \
      "$failure_path" \
      "pipeline_hard_events" \
      "present" \
      "missing" \
      "present"
    return 1
  fi

  if [[ "$require_no_pipeline_hard_events" == "1" ]]; then
    totals_path="$(mktemp "${OUTPUT_DIR}/pipeline-hard-events.XXXXXX.tsv")"
    docker_bench_write_pipeline_hard_event_totals "$phase_summary" "$totals_path"
    while IFS=$'\t' read -r event max_rate total; do
      [[ "$event" != "event" ]] || continue
      [[ -n "$event" ]] || continue
      docker_bench_pipeline_hard_event_allowed "$event" && continue
      docker_bench_guard_check_at_most \
        "$failure_path" \
        "${event}_total" \
        "$total" \
        "0" || failure_count=$((failure_count + 1))
    done <"$totals_path"
    rm -f "$totals_path"
  fi

  threshold="${NVPN_DOCKER_MAX_UDP_KERNEL_DROPPED:-}"
  if [[ -n "$threshold" ]]; then
    global_drops="$(docker_bench_pipeline_hard_event_total "$phase_summary" udp_kernel_dropped)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "udp_kernel_dropped_total" \
      "$global_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  threshold="${NVPN_DOCKER_MAX_UDP_SOCKET_KERNEL_DROPPED:-}"
  if [[ -n "$threshold" ]]; then
    socket_drops="$(docker_bench_pipeline_hard_event_total "$phase_summary" udp_socket_kernel_dropped)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "udp_socket_kernel_dropped_total" \
      "$socket_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  threshold="${NVPN_DOCKER_MAX_UDP_NAMESPACE_RCVBUF_ERRORS:-}"
  if [[ -n "$threshold" ]]; then
    namespace_drops="$(docker_bench_pipeline_hard_event_total "$phase_summary" udp_namespace_rcvbuf_errors)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "udp_namespace_rcvbuf_errors_total" \
      "$namespace_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  threshold="${NVPN_DOCKER_MAX_CONNECTED_UDP_KERNEL_DROPPED:-}"
  if [[ -n "$threshold" ]]; then
    global_drops="$(docker_bench_pipeline_hard_event_total "$phase_summary" connected_udp_kernel_dropped)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "connected_udp_kernel_dropped_total" \
      "$global_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  threshold="${NVPN_DOCKER_MAX_CONNECTED_UDP_PEER_KERNEL_DROPPED:-}"
  if [[ -n "$threshold" ]]; then
    peer_drops="$(docker_bench_pipeline_hard_event_total "$phase_summary" connected_udp_peer_kernel_dropped)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "connected_udp_peer_kernel_dropped_total" \
      "$peer_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  threshold="${NVPN_DOCKER_MAX_CONNECTED_UDP_DRAIN_BULK_DROPPED:-}"
  if [[ -n "$threshold" ]]; then
    bulk_drops="$(docker_bench_pipeline_hard_event_total "$phase_summary" connected_udp_drain_bulk_dropped)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "connected_udp_drain_bulk_dropped_total" \
      "$bulk_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    connected_udp_direct_decrypt_bulk_shed \
    connected_udp_direct_decrypt_bulk_shed_total \
    "${NVPN_DOCKER_MAX_CONNECTED_UDP_DIRECT_DECRYPT_BULK_SHED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    endpoint_event_bulk_dropped \
    endpoint_event_bulk_dropped_total \
    "${NVPN_DOCKER_MAX_ENDPOINT_EVENT_BULK_DROPPED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_worker_queue_full \
    decrypt_worker_queue_full_total \
    "${NVPN_DOCKER_MAX_DECRYPT_WORKER_QUEUE_FULL:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_worker_bulk_dropped \
    decrypt_worker_bulk_dropped_total \
    "${NVPN_DOCKER_MAX_DECRYPT_WORKER_BULK_DROPPED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_fallback_pressure_drain \
    decrypt_fallback_pressure_drain_total \
    "${NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRESSURE_DRAIN:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_fallback_priority_gated \
    decrypt_fallback_priority_gated_total \
    "${NVPN_DOCKER_MAX_DECRYPT_FALLBACK_PRIORITY_GATED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_fsp_helper_window_fallback \
    decrypt_fsp_helper_window_fallback_total \
    "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_WINDOW_FALLBACK:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_fsp_open_worker_window_fallback \
    decrypt_fsp_open_worker_window_fallback_total \
    "${NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_WINDOW_FALLBACK:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_fsp_helper_queue_full_fallback \
    decrypt_fsp_helper_queue_full_fallback_total \
    "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_QUEUE_FULL_FALLBACK:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_fsp_helper_completion_backlog_fallback \
    decrypt_fsp_helper_completion_backlog_fallback_total \
    "${NVPN_DOCKER_MAX_DECRYPT_FSP_HELPER_COMPLETION_BACKLOG_FALLBACK:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    decrypt_fsp_owner_handoff_dropped \
    decrypt_fsp_owner_handoff_dropped_total \
    "${NVPN_DOCKER_MAX_DECRYPT_FSP_OWNER_HANDOFF_DROPPED:-}" \
    || failure_count=$((failure_count + 1))

  threshold="${NVPN_DOCKER_MAX_DECRYPT_FSP_OPEN_WORKER_COMPLETION_BACKLOG_FALLBACK:-}"
  if [[ -n "$threshold" ]]; then
    completion_backlog_fallbacks="$(docker_bench_pipeline_hard_event_total "$phase_summary" decrypt_fsp_open_worker_completion_backlog_fallback)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "decrypt_fsp_open_worker_completion_backlog_fallback_total" \
      "$completion_backlog_fallbacks" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  threshold="${NVPN_DOCKER_MAX_DECRYPT_FSP_WORKER_REPLAY_DROPPED:-}"
  if [[ -n "$threshold" ]]; then
    fsp_worker_replay_drops="$(docker_bench_pipeline_hard_event_total "$phase_summary" decrypt_fsp_worker_replay_dropped)"
    docker_bench_guard_check_at_most \
      "$failure_path" \
      "decrypt_fsp_worker_replay_dropped_total" \
      "$fsp_worker_replay_drops" \
      "$threshold" || failure_count=$((failure_count + 1))
  fi

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fmp_aead_completion_aead_failed \
    fmp_aead_completion_aead_failed_total \
    "${NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_AEAD_FAILED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_aead_failed \
    fsp_aead_completion_aead_failed_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_AEAD_FAILED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_epoch_mismatch \
    fsp_aead_completion_epoch_mismatch_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_EPOCH_MISMATCH:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_stale_session \
    fsp_aead_completion_stale_session_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_SESSION:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_stale_order \
    fsp_aead_completion_stale_order_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_ORDER:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_stale_ticket \
    fsp_aead_completion_stale_ticket_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_STALE_TICKET:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_duplicate_ticket \
    fsp_aead_completion_duplicate_ticket_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_DUPLICATE_TICKET:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_window_exceeded \
    fsp_aead_completion_window_exceeded_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_WINDOW_EXCEEDED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fmp_aead_completion_replay_dropped \
    fmp_aead_completion_replay_dropped_total \
    "${NVPN_DOCKER_MAX_FMP_AEAD_COMPLETION_REPLAY_DROPPED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_replay_dropped \
    fsp_aead_completion_replay_dropped_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_replay_dropped_helper \
    fsp_aead_completion_replay_dropped_helper_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED_HELPER:-}" \
    || failure_count=$((failure_count + 1))

  docker_bench_guard_pipeline_hard_event_at_most \
    "$phase_summary" \
    "$failure_path" \
    fsp_aead_completion_replay_dropped_helper_returned \
    fsp_aead_completion_replay_dropped_helper_returned_total \
    "${NVPN_DOCKER_MAX_FSP_AEAD_COMPLETION_REPLAY_DROPPED_HELPER_RETURNED:-}" \
    || failure_count=$((failure_count + 1))

  if (( failure_count > 0 )); then
    printf 'docker bench guard failed: wrote %s\n' "$failure_path" >&2
    return 1
  fi
}

docker_bench_pipeline_lines_after_start_from_stdin() {
  local start_line="${1:-0}"
  awk -v start_line="$start_line" 'NR > start_line'
}

docker_bench_pipeline_lines_in_range_from_stdin() {
  local start_line="${1:-0}"
  local end_line="${2:-0}"
  awk -v start_line="$start_line" -v end_line="$end_line" \
    'NR > start_line && (end_line <= 0 || NR <= end_line)'
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
      value = metric_avg_us(line, "fsp_aead_worker_open_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "fsp_aead_worker_open_completion_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_command_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_priority_command_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "endpoint_bulk_command_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "connected_udp_drain_ring_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "connected_udp_drain_priority_ring_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "connected_udp_drain_bulk_ring_wait")
      if (value > score) score = value
      return score
    }
    function nvpn_score(line, score, value) {
      score = -1
      value = metric_avg_us(line, "nvpn_tun_to_mesh_queue_wait")
      if (value > score) score = value
      value = metric_avg_us(line, "nvpn_direct_endpoint_queue")
      if (value > score) score = value
      value = metric_avg_us(line, "nvpn_direct_endpoint_wake")
      if (value > score) score = value
      value = metric_avg_us(line, "nvpn_direct_endpoint_backlog")
      if (value > score) score = value
      value = metric_avg_us(line, "nvpn_direct_endpoint_consumer_busy")
      if (value > score) score = value
      value = metric_avg_us(line, "nvpn_tun_write_batch")
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
      metrics = "endpoint_command_wait endpoint_priority_command_wait endpoint_bulk_command_wait endpoint_event_wait endpoint_priority_event_wait endpoint_bulk_event_wait connected_udp_drain_recv connected_udp_fast_path_dispatch connected_udp_drain_ring_wait connected_udp_drain_priority_ring_wait connected_udp_drain_bulk_ring_wait fmp_worker_queue_wait fmp_worker_priority_queue_wait fmp_worker_bulk_queue_wait fmp_linux_bulk_container_queue_wait fmp_linux_bulk_container_ready_wait fmp_linux_bulk_container_first_slot_wait fmp_linux_bulk_container_all_slots_wait decrypt_worker_queue_wait decrypt_worker_priority_queue_wait decrypt_worker_bulk_queue_wait decrypt_fallback_wait decrypt_fallback_priority_wait decrypt_fallback_bulk_wait fsp_aead_worker_open_queue_wait fsp_aead_worker_open_completion_wait decrypt_authenticated_session_wait decrypt_authenticated_session_priority_wait decrypt_authenticated_session_bulk_wait decrypt_direct_session_commit_wait decrypt_direct_session_data_wait decrypt_fsp_worker_queue_wait decrypt_fsp_worker_priority_queue_wait decrypt_fsp_worker_bulk_queue_wait transport_queue_wait transport_priority_queue_wait transport_bulk_queue_wait transport_channel_wait transport_priority_channel_wait transport_bulk_channel_wait transport_rx_loop_wait transport_priority_rx_loop_wait transport_bulk_rx_loop_wait nvpn_tun_to_mesh_queue_wait nvpn_direct_endpoint_queue nvpn_direct_endpoint_wake nvpn_direct_endpoint_backlog nvpn_direct_endpoint_consumer_busy"
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

docker_bench_pipeline_fsp_worker_open_wait_summary() {
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
    function append_summary(prefix) {
      part = sprintf("%s_rate_per_sec=%g,%s_p95_ms=%g,%s_p99_ms=%g,%s_max_ms=%g,%s_allmax_ms=%g",
        prefix, metric_rate, prefix, metric_p95, prefix, metric_p99, prefix, metric_max, prefix, metric_allmax)
      if (summary == "") {
        summary = part
      } else {
        summary = summary "," part
      }
    }
    {
      if (parse_wait($0, "fsp_aead_worker_open_queue_wait")) {
        append_summary("queue")
      }
      if (parse_wait($0, "fsp_aead_worker_open_completion_wait")) {
        append_summary("completion")
      }
    }
    END {
      if (summary != "") {
        print summary
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
      split_target = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_split_target") : 0
      split_lane = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_split_lane") : 0
      split_backpressure = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_split_backpressure") : 0
      split_packet_cap = prefix == "fmp_worker_batch" ? parse_rate($0, "fmp_send_group_split_packet_cap") : 0
      split_total = split_target + split_lane + split_backpressure + split_packet_cap
      committed_dispatch = prefix == "fmp_worker_batch" ? parse_rate($0, "endpoint_committed_bulk_dispatch_batch") : 0
      committed_packets = prefix == "fmp_worker_batch" ? parse_rate($0, "endpoint_committed_bulk_dispatch_packets") : 0
      committed_merged_batches = prefix == "fmp_worker_batch" ? parse_rate($0, "endpoint_committed_bulk_dispatch_merged_batch") : 0
      committed_merged_packets = prefix == "fmp_worker_batch" ? parse_rate($0, "endpoint_committed_bulk_dispatch_merged_packets") : 0
      committed_avg = committed_dispatch > 0 ? committed_packets / committed_dispatch : 0
      select_priority = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_select_priority") : 0
      select_fmp_completion = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_select_fmp_completion") : 0
      select_fsp_completion_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_select_fsp_completion_packets") : 0
      select_bulk_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_select_bulk_packets") : 0
      drain_priority = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_drain_priority") : 0
      drain_completion_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_drain_aead_completion_packets") : 0
      drain_bulk_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_drain_bulk_packets") : 0
      interleave_completion_packets = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_bulk_interleave_aead_completion_packets") : 0
      interleave_budget_exhausted = prefix == "decrypt_worker_batch" ? parse_rate($0, "decrypt_worker_bulk_interleave_budget_exhausted") : 0
      scheduler_total = select_priority + select_fmp_completion + select_fsp_completion_packets + select_bulk_packets + drain_priority + drain_completion_packets + drain_bulk_packets + interleave_completion_packets + interleave_budget_exhausted
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
      if (split_total > 0) {
        summary = summary sprintf(",send_group_split_total_per_sec=%g,send_group_split_target_per_sec=%g,send_group_split_lane_per_sec=%g,send_group_split_backpressure_per_sec=%g,send_group_split_packet_cap_per_sec=%g", split_total, split_target, split_lane, split_backpressure, split_packet_cap)
      }
      if (committed_dispatch > 0) {
        summary = summary sprintf(",committed_bulk_dispatch_avg_packets=%.1f,committed_bulk_dispatch_per_sec=%g,committed_bulk_dispatch_packets_per_sec=%g,committed_bulk_merged_batches_per_sec=%g,committed_bulk_merged_packets_per_sec=%g", committed_avg, committed_dispatch, committed_packets, committed_merged_batches, committed_merged_packets)
      }
      if (scheduler_total > 0) {
        summary = summary sprintf(",select_priority_per_sec=%g,select_fmp_completion_per_sec=%g,select_fsp_completion_packets_per_sec=%g,select_bulk_packets_per_sec=%g,drain_priority_per_sec=%g,drain_completion_packets_per_sec=%g,drain_bulk_packets_per_sec=%g,bulk_interleave_completion_packets_per_sec=%g,bulk_interleave_budget_exhausted_per_sec=%g", select_priority, select_fmp_completion, select_fsp_completion_packets, select_bulk_packets, drain_priority, drain_completion_packets, drain_bulk_packets, interleave_completion_packets, interleave_budget_exhausted)
      }
      print summary
      exit
    }
  '
}

docker_bench_fsp_owner_placement_line_from_stdin() {
  awk '
    function metric_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return 0
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    function max(a, b) {
      return a > b ? a : b
    }
    /^\[pipe / {
      worker_open = metric_rate($0, "decrypt_fsp_path_worker_open")
      worker_open_striped = metric_rate($0, "decrypt_fsp_path_worker_open_striped")
      owner = metric_rate($0, "decrypt_fsp_owner_same") + metric_rate($0, "decrypt_fsp_owner_mismatch")
      path = metric_rate($0, "decrypt_fsp_path_local") + metric_rate($0, "decrypt_fsp_path_handoff") + metric_rate($0, "decrypt_fsp_path_helper") + worker_open
      bulk = max(metric_rate($0, "decrypt_worker_batch_bulk_packets"), metric_rate($0, "decrypt_worker_select_bulk_packets"))
      bulk = max(bulk, metric_rate($0, "decrypt_worker_drain_bulk_packets"))
      bulk = max(bulk, metric_rate($0, "decrypt_authenticated_session_bulk_wait"))
      score = bulk + worker_open
      fallback_score = owner + path
      if (best == "" || score > best_score || (score == best_score && fallback_score >= best_fallback_score)) {
        best = $0
        best_score = score
        best_fallback_score = fallback_score
      }
    }
    END {
      if (best != "") {
        print best
      }
    }
  '
}

docker_bench_pipeline_fmp_worker_batch_summary() {
  docker_bench_pipeline_worker_batch_summary "$1" "fmp_worker_batch"
}

docker_bench_pipeline_fmp_worker_dispatch_spread_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return 0
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    function append_rate(summary, label, value) {
      if (value <= 0) {
        return summary
      }
      if (summary != "") {
        summary = summary ";"
      }
      return summary sprintf("%s:%g", label, value)
    }
    {
      total = 0
      active = 0
      top_rate = 0
      top_worker = ""
      rate_count = 0
      for (i = 0; i <= 7; i++) {
        metric = "fmp_worker_dispatch_worker" i
        rate = parse_rate($0, metric)
        rates[i] = rate
        total += rate
        if (rate > 0) {
          active++
          rate_count++
        }
        if (rate > top_rate) {
          top_rate = rate
          top_worker = "w" i
        }
      }
      other = parse_rate($0, "fmp_worker_dispatch_worker_other")
      total += other
      if (other > 0) {
        active++
        rate_count++
      }
      if (other > top_rate) {
        top_rate = other
        top_worker = "other"
      }
      if (total <= 0) {
        next
      }

      meaningful = 0
      for (i = 0; i <= 7; i++) {
        if (rates[i] > 0 && (rates[i] / total) >= 0.01) {
          meaningful++
        }
      }
      if (other > 0 && (other / total) >= 0.01) {
        meaningful++
      }

      keyed = parse_rate($0, "fmp_worker_dispatch_flow_keyed")
      target_only = parse_rate($0, "fmp_worker_dispatch_target_only")
      classified = keyed + target_only
      top_pct = (top_rate / total) * 100
      keyed_pct = classified > 0 ? (keyed / classified) * 100 : 0
      target_only_pct = classified > 0 ? (target_only / classified) * 100 : 0

      worker_rates = ""
      for (i = 0; i <= 7; i++) {
        worker_rates = append_rate(worker_rates, "w" i, rates[i])
      }
      worker_rates = append_rate(worker_rates, "other", other)

      printf "active_workers=%d,workers_ge1pct=%d,top_worker=%s,top_pct=%.1f,flow_keyed_pct=%.1f,target_only_pct=%.1f,total_per_sec=%g,worker_rates=%s\n", active, meaningful, top_worker, top_pct, keyed_pct, target_only_pct, total, worker_rates
      exit
    }
  '
}

docker_bench_pipeline_fsp_owner_placement_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return 0
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    function classify_path(local, handoff, helper, worker_open) {
      if (worker_open > 0 && worker_open >= local && worker_open >= handoff && worker_open >= helper) {
        return "worker-open"
      }
      if (helper > 0 && helper >= local && helper >= handoff) {
        return "helper"
      }
      if (handoff > 0 && handoff >= local) {
        return "handoff"
      }
      if (local > 0) {
        return "local"
      }
      return "unknown"
    }
    {
      same = parse_rate($0, "decrypt_fsp_owner_same")
      mismatch = parse_rate($0, "decrypt_fsp_owner_mismatch")
      local = parse_rate($0, "decrypt_fsp_path_local")
      handoff = parse_rate($0, "decrypt_fsp_path_handoff")
      helper = parse_rate($0, "decrypt_fsp_path_helper")
      worker_open = parse_rate($0, "decrypt_fsp_path_worker_open")
      worker_open_striped = parse_rate($0, "decrypt_fsp_path_worker_open_striped")
      local_priority = parse_rate($0, "decrypt_fsp_path_local_priority")
      local_bulk = parse_rate($0, "decrypt_fsp_path_local_bulk")
      handoff_priority = parse_rate($0, "decrypt_fsp_path_handoff_priority")
      handoff_bulk = parse_rate($0, "decrypt_fsp_path_handoff_bulk")
      helper_bulk = parse_rate($0, "decrypt_fsp_path_helper_bulk")
      worker_open_bulk = parse_rate($0, "decrypt_fsp_path_worker_open_bulk")
      bulk_packets = parse_rate($0, "decrypt_worker_batch_bulk_packets")
      select_bulk_packets = parse_rate($0, "decrypt_worker_select_bulk_packets")
      drain_bulk_packets = parse_rate($0, "decrypt_worker_drain_bulk_packets")
      if (same <= 0 && mismatch <= 0 && local <= 0 && handoff <= 0 && helper <= 0 && worker_open <= 0) {
        next
      }
      owner = same >= mismatch ? "same" : "mismatch"
      path = classify_path(local, handoff, helper, worker_open)
      bulk_path = classify_path(local_bulk, handoff_bulk, helper_bulk, worker_open_bulk)
      priority_path = classify_path(local_priority, handoff_priority, 0, 0)
      printf "owner=%s,path=%s,bulk_path=%s,priority_path=%s,owner_same_per_sec=%g,owner_mismatch_per_sec=%g,path_local_per_sec=%g,path_handoff_per_sec=%g,path_helper_per_sec=%g,path_worker_open_per_sec=%g,path_worker_open_striped_per_sec=%g,path_local_priority_per_sec=%g,path_local_bulk_per_sec=%g,path_handoff_priority_per_sec=%g,path_handoff_bulk_per_sec=%g,path_helper_bulk_per_sec=%g,path_worker_open_bulk_per_sec=%g,bulk_packets_per_sec=%g,select_bulk_packets_per_sec=%g,drain_bulk_packets_per_sec=%g\n", owner, path, bulk_path, priority_path, same, mismatch, local, handoff, helper, worker_open, worker_open_striped, local_priority, local_bulk, handoff_priority, handoff_bulk, helper_bulk, worker_open_bulk, bulk_packets, select_bulk_packets, drain_bulk_packets
      exit
    }
  '
}

docker_bench_pipeline_fsp_owner_placement_other_path_max() {
  local line="$1"
  local expected="$2"
  local summary
  summary="$(docker_bench_pipeline_fsp_owner_placement_summary "$line")"
  printf '%s\n' "$summary" | awk -v expected="$expected" '
    function field_value(name, count, i, kv) {
      count = split($0, fields, ",")
      for (i = 1; i <= count; i++) {
        split(fields[i], kv, "=")
        if (kv[1] == name) {
          return kv[2] + 0
        }
      }
      return 0
    }
    function consider(path, rate) {
      if (path == expected) {
        return
      }
      if (rate > max_rate) {
        max_rate = rate
        max_path = path
      }
    }
    {
      if ($0 == "") {
        next
      }
      if (expected == "worker-open" || expected == "helper") {
        consider("local", field_value("path_local_bulk_per_sec"))
        consider("handoff", field_value("path_handoff_bulk_per_sec"))
        consider("helper", field_value("path_helper_bulk_per_sec"))
        consider("worker-open", field_value("path_worker_open_bulk_per_sec"))
      } else {
        consider("local", field_value("path_local_per_sec"))
        consider("handoff", field_value("path_handoff_per_sec"))
        consider("helper", field_value("path_helper_per_sec"))
        consider("worker-open", field_value("path_worker_open_per_sec"))
      }
      printf "%s\t%g\n", max_path, max_rate
      exit
    }
  '
}

docker_bench_pipeline_fsp_owner_placement_kind() {
  local line="$1"
  local summary
  summary="$(docker_bench_pipeline_fsp_owner_placement_summary "$line")"
  case "$summary" in
    *path=worker-open*) printf 'worker-open\n' ;;
    *path=helper*) printf 'helper\n' ;;
    *path=handoff*) printf 'handoff\n' ;;
    *path=local*) printf 'local\n' ;;
    owner=same,*) printf 'same\n' ;;
    owner=mismatch,*) printf 'mismatch\n' ;;
  esac
}

docker_bench_pipeline_decrypt_worker_batch_summary() {
  docker_bench_pipeline_worker_batch_summary "$1" "decrypt_worker_batch"
}

docker_bench_pipeline_decrypt_worker_spread_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return 0
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    function append_rate(summary, label, value) {
      if (value <= 0) {
        return summary
      }
      if (summary != "") {
        summary = summary ";"
      }
      return summary sprintf("%s:%g", label, value)
    }
    {
      total = 0
      active = 0
      top_rate = 0
      top_worker = ""
      for (i = 0; i <= 7; i++) {
        metric = "decrypt_worker_batch_worker" i
        rate = parse_rate($0, metric)
        rates[i] = rate
        total += rate
        if (rate > 0) {
          active++
        }
        if (rate > top_rate) {
          top_rate = rate
          top_worker = "w" i
        }
      }
      other = parse_rate($0, "decrypt_worker_batch_worker_other")
      total += other
      if (other > 0) {
        active++
      }
      if (other > top_rate) {
        top_rate = other
        top_worker = "other"
      }
      if (total <= 0) {
        next
      }

      meaningful = 0
      for (i = 0; i <= 7; i++) {
        if (rates[i] > 0 && (rates[i] / total) >= 0.01) {
          meaningful++
        }
      }
      if (other > 0 && (other / total) >= 0.01) {
        meaningful++
      }

      worker_rates = ""
      for (i = 0; i <= 7; i++) {
        worker_rates = append_rate(worker_rates, "w" i, rates[i])
      }
      worker_rates = append_rate(worker_rates, "other", other)

      printf "active_workers=%d,workers_ge1pct=%d,top_worker=%s,top_pct=%.1f,total_packets_per_sec=%g,worker_packet_rates=%s\n", active, meaningful, top_worker, (top_rate / total) * 100, total, worker_rates
      exit
    }
  '
}

docker_bench_pipeline_decrypt_worker_turn_mix_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function parse_rate(line, metric, start, rest, parts, value) {
      start = index(line, metric "=")
      if (start == 0) {
        return 0
      }
      rest = substr(line, start + length(metric) + 1)
      split(rest, parts, " ")
      value = parts[1]
      sub(/\/s$/, "", value)
      return value + 0
    }
    {
      select_fmp_completion_batches = parse_rate($0, "decrypt_worker_select_fmp_completion")
      select_fsp_completion_batches = parse_rate($0, "decrypt_worker_select_fsp_completion_batch")
      select_fsp_completion_packets = parse_rate($0, "decrypt_worker_select_fsp_completion_packets")
      select_bulk_packets = parse_rate($0, "decrypt_worker_select_bulk_packets")
      drain_completion_batches = parse_rate($0, "decrypt_worker_drain_aead_completion_batch")
      drain_completion_packets = parse_rate($0, "decrypt_worker_drain_aead_completion_packets")
      drain_bulk_packets = parse_rate($0, "decrypt_worker_drain_bulk_packets")
      interleave_completion_batches = parse_rate($0, "decrypt_worker_bulk_interleave_aead_completion_batch")
      interleave_completion_packets = parse_rate($0, "decrypt_worker_bulk_interleave_aead_completion_packets")
      interleave_budget_exhausted = parse_rate($0, "decrypt_worker_bulk_interleave_budget_exhausted")

      completion_batches = select_fmp_completion_batches + select_fsp_completion_batches + drain_completion_batches + interleave_completion_batches
      known_completion_packets = select_fsp_completion_packets + drain_completion_packets + interleave_completion_packets
      bulk_packets = select_bulk_packets + drain_bulk_packets
      total_known_packets = known_completion_packets + bulk_packets
      if (completion_batches <= 0 && known_completion_packets <= 0 && bulk_packets <= 0) {
        next
      }
      completion_packet_pct = total_known_packets > 0 ? (known_completion_packets / total_known_packets) * 100 : 0

      printf "completion_packet_pct=%.1f,completion_batches_per_sec=%g,known_completion_packets_per_sec=%g,bulk_packets_per_sec=%g,select_fmp_completion_batches_per_sec=%g,select_fsp_completion_batches_per_sec=%g,select_fsp_completion_packets_per_sec=%g,select_bulk_packets_per_sec=%g,drain_completion_batches_per_sec=%g,drain_completion_packets_per_sec=%g,drain_bulk_packets_per_sec=%g,interleave_completion_batches_per_sec=%g,interleave_completion_packets_per_sec=%g,interleave_budget_exhausted_per_sec=%g\n", completion_packet_pct, completion_batches, known_completion_packets, bulk_packets, select_fmp_completion_batches, select_fsp_completion_batches, select_fsp_completion_packets, select_bulk_packets, drain_completion_batches, drain_completion_packets, drain_bulk_packets, interleave_completion_batches, interleave_completion_packets, interleave_budget_exhausted
      exit
    }
  '
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
      gso_frames = parse_rate($0, "nvpn_tun_read_vnet_gso_frames")
      gso_segments = parse_rate($0, "nvpn_tun_read_vnet_gso_segments")
      gso_segment_bytes = parse_rate($0, "nvpn_tun_read_vnet_gso_segment_bytes")
      if (flush <= 0 && packets <= 0 && bytes <= 0) {
        next
      }
      avg_packets = flush > 0 ? packets / flush : 0
      full_pct = flush > 0 ? (full / flush) * 100 : 0
      single_pct = flush > 0 ? (single / flush) * 100 : 0
      avg_bytes = packets > 0 ? bytes / packets : 0
      if (gso_frames > 0 || gso_segments > 0) {
        gso_avg_segments = gso_frames > 0 ? gso_segments / gso_frames : 0
        gso_avg_segment_bytes = gso_segments > 0 ? gso_segment_bytes / gso_segments : 0
        printf "avg_packets=%.1f,full_pct=%.1f,single_pct=%.1f,avg_packet_bytes=%.1f,flush_per_sec=%g,packets_per_sec=%g,bytes_per_sec=%g,vnet_gso_frames_per_sec=%g,vnet_gso_segments_per_sec=%g,vnet_gso_avg_segments=%.1f,vnet_gso_avg_segment_bytes=%.1f\n", avg_packets, full_pct, single_pct, avg_bytes, flush, packets, bytes, gso_frames, gso_segments, gso_avg_segments, gso_avg_segment_bytes
      } else {
        printf "avg_packets=%.1f,full_pct=%.1f,single_pct=%.1f,avg_packet_bytes=%.1f,flush_per_sec=%g,packets_per_sec=%g,bytes_per_sec=%g\n", avg_packets, full_pct, single_pct, avg_bytes, flush, packets, bytes
      }
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

docker_bench_pipeline_nvpn_direct_endpoint_summary() {
  local line="$1"
  printf '%s\n' "$line" | awk '
    function duration_ms(value, number) {
      number = value + 0
      if (value ~ /ns$/) return number / 1000000
      if (value ~ /us$/) return number / 1000
      if (value ~ /ms$/) return number
      if (value ~ /s$/) return number * 1000
      return number
    }
    function append(name, value) {
      if (summary != "") summary = summary ","
      summary = summary name "=" value
    }
    function parse_stage(line, metric, prefix, start, rest, parts, rate_raw, avg_raw, p95_raw, p99_raw, max_raw, allmax_raw) {
      start = index(line, metric "=")
      if (start == 0) return
      rest = substr(line, start)
      split(rest, parts, " ")
      if (parts[2] !~ /^avg=/ || parts[4] !~ /^p95<=/ || parts[5] !~ /^p99<=/ || parts[6] !~ /^max<=/ || parts[7] !~ /^allmax=/) return
      rate_raw = parts[1]
      avg_raw = parts[2]
      p95_raw = parts[4]
      p99_raw = parts[5]
      max_raw = parts[6]
      allmax_raw = parts[7]
      sub(/^[^=]+=/, "", rate_raw)
      sub(/\/s$/, "", rate_raw)
      sub(/^avg=/, "", avg_raw)
      sub(/^p95<=/, "", p95_raw)
      sub(/^p99<=/, "", p99_raw)
      sub(/^max<=/, "", max_raw)
      sub(/^allmax=/, "", allmax_raw)
      append(prefix "_rate_per_sec", rate_raw + 0)
      append(prefix "_avg_ms", duration_ms(avg_raw))
      append(prefix "_p95_ms", duration_ms(p95_raw))
      append(prefix "_p99_ms", duration_ms(p99_raw))
      append(prefix "_max_ms", duration_ms(max_raw))
      append(prefix "_allmax_ms", duration_ms(allmax_raw))
    }
    function parse_counter(line, metric, prefix, start, rest, parts, rate_raw, total_raw) {
      start = index(line, metric "=")
      if (start == 0) return
      rest = substr(line, start)
      split(rest, parts, " ")
      if (parts[2] !~ /^total=/) return
      rate_raw = parts[1]
      total_raw = parts[2]
      sub(/^[^=]+=/, "", rate_raw)
      sub(/\/s$/, "", rate_raw)
      sub(/^total=/, "", total_raw)
      append(prefix "_per_sec", rate_raw + 0)
      append(prefix "_total", total_raw + 0)
    }
    {
      parse_stage($0, "nvpn_direct_endpoint_queue", "queue")
      parse_stage($0, "nvpn_direct_endpoint_wake", "wake")
      parse_stage($0, "nvpn_direct_endpoint_backlog", "backlog")
      parse_stage($0, "nvpn_direct_endpoint_consumer_busy", "consumer_busy")
      parse_stage($0, "nvpn_direct_endpoint_recv", "recv")
      parse_stage($0, "nvpn_direct_endpoint_finalize", "finalize")
      parse_stage($0, "nvpn_tun_write_batch", "tun_batch")
      parse_counter($0, "nvpn_direct_endpoint_rx_limit_splits", "limit_splits")
      parse_counter($0, "nvpn_direct_endpoint_rx_limit_tail_packets", "limit_tail_packets")
    }
    END {
      if (summary != "") print summary
    }
  '
}

docker_bench_pipeline_hard_event_summary_from_stdin() {
  local start_line="${1:-0}"
  local end_line="${2:-0}"
  awk -v start_line="$start_line" -v end_line="$end_line" '
    function parse_event(line, name, in_phase, pattern, start, rest, parts, rate_raw, rate, total_raw, total, i) {
      pattern = "(^|[]] | )" name "="
      if (match(line, pattern) == 0) {
        return
      }
      start = RSTART + RLENGTH
      rest = substr(line, start)
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
          } else if (before_phase && (!(name in base_total) || total > base_total[name])) {
            base_total[name] = total
          }
          break
        }
      }
    }
    BEGIN {
      events = "udp_send_backpressure udp_send_backpressure_sleep udp_send_bulk_dropped udp_kernel_dropped udp_socket_kernel_dropped udp_namespace_rcvbuf_errors linux_bulk_udp_pace_wait connected_udp_activation_failed connected_udp_peer_cap_skipped connected_udp_fd_budget_skipped connected_udp_kernel_dropped connected_udp_peer_kernel_dropped connected_udp_drain_bulk_dropped connected_udp_direct_decrypt_bulk_shed encrypt_worker_queue_full encrypt_worker_priority_queue_full encrypt_worker_bulk_queue_full encrypt_worker_bulk_dropped fmp_linux_bulk_container_queue_full fmp_linux_bulk_container_queue_full_packets endpoint_bulk_fast_path_prepare_failed endpoint_bulk_fast_path_stage_full endpoint_bulk_fast_path_feedback_full decrypt_worker_queue_full decrypt_worker_bulk_dropped decrypt_worker_register_full decrypt_worker_priority_dropped decrypt_fallback_backlog_high rx_loop_slow_maintenance_timeout rx_loop_slow_maintenance_skipped decrypt_fallback_bulk_dropped decrypt_fallback_priority_dropped decrypt_fallback_pressure_drain decrypt_fallback_priority_gated decrypt_fsp_priority_queue_full_fallback decrypt_fsp_bulk_queue_full_fallback decrypt_fsp_owner_handoff_dropped decrypt_fsp_helper_window_fallback decrypt_fsp_open_worker_window_fallback decrypt_fsp_helper_queue_full_fallback decrypt_fsp_helper_completion_backlog_fallback decrypt_fsp_open_pool_queue_full_fallback decrypt_fsp_open_worker_completion_backlog_fallback decrypt_fsp_worker_replay_dropped decrypt_fsp_worker_replay_dropped_duplicate decrypt_fsp_worker_replay_dropped_too_old decrypt_fsp_worker_replay_dropped_too_old_lag_ge_2x_window decrypt_fsp_worker_replay_dropped_too_old_lag_ge_4x_window decrypt_fsp_worker_replay_dropped_too_old_lag_ge_16x_window decrypt_fsp_worker_replay_dropped_too_old_lag_ge_64x_window fmp_aead_completion_aead_failed fmp_aead_completion_replay_dropped fmp_aead_completion_replay_dropped_prechecked fmp_aead_completion_replay_dropped_deferred fmp_aead_completion_replay_dropped_duplicate fmp_aead_completion_replay_dropped_too_old fmp_aead_completion_replay_dropped_too_old_lag_ge_2x_window fmp_aead_completion_replay_dropped_too_old_lag_ge_4x_window fmp_aead_completion_replay_dropped_too_old_lag_ge_16x_window fmp_aead_completion_replay_dropped_too_old_lag_ge_64x_window fsp_aead_completion_aead_failed fsp_aead_completion_epoch_mismatch fsp_aead_completion_stale_session fsp_aead_completion_stale_order fsp_aead_completion_stale_ticket fsp_aead_completion_duplicate_ticket fsp_aead_completion_window_exceeded fsp_aead_completion_replay_dropped fsp_aead_completion_replay_dropped_helper fsp_aead_completion_replay_dropped_helper_returned fsp_aead_completion_replay_dropped_worker_open fsp_aead_completion_replay_dropped_worker_open_returned fsp_aead_completion_replay_dropped_duplicate fsp_aead_completion_replay_dropped_too_old fsp_aead_completion_replay_dropped_too_old_lag_ge_2x_window fsp_aead_completion_replay_dropped_too_old_lag_ge_4x_window fsp_aead_completion_replay_dropped_too_old_lag_ge_16x_window fsp_aead_completion_replay_dropped_too_old_lag_ge_64x_window decrypt_authenticated_session_priority_dropped decrypt_authenticated_session_bulk_dropped pending_tun_destination_dropped pending_tun_packet_dropped pending_endpoint_destination_dropped pending_endpoint_packet_dropped endpoint_event_backlog_high endpoint_command_bulk_dropped endpoint_event_bulk_dropped transport_channel_backlog_high transport_bulk_dropped dataplane_live_retired_drops dataplane_live_output_drops dataplane_drop_admission dataplane_drop_unknown_owner dataplane_drop_replay dataplane_drop_stale_generation dataplane_drop_stale_completion_generation dataplane_drop_crypto_failed nvpn_tun_to_mesh_bulk_dropped nvpn_tun_to_mesh_bulk_dropped_batches nvpn_tun_to_mesh_bulk_dropped_packet_cap nvpn_tun_to_mesh_bulk_dropped_channel_full"
      event_count = split(events, names, " ")
    }
    {
      before_phase = (NR <= start_line)
      in_phase = (NR > start_line && (end_line <= 0 || NR <= end_line))
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
