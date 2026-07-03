#!/usr/bin/env bash
# Build one apples-to-apples table from Docker benchmark artifact directories.
set -euo pipefail

OUTPUT_DIR="${NVPN_DOCKER_TABLE_OUTPUT_DIR:-}"

die() {
  printf 'docker benchmark table failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
usage: scripts/summarize-docker-benchmark-table.sh [--output-dir DIR] label=artifact-dir ...

Each artifact dir must contain summary.tsv. metadata.json, raw/ CPU phase
artifacts, and raw/ pipeline hard-event artifacts are used when present.

Outputs:
  stdout: Markdown table
  --output-dir: stress-table.tsv and stress-table.md
EOF
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

summary_file() {
  local input="$1"
  if [[ -d "$input" ]]; then
    [[ -f "$input/summary.tsv" ]] || die "missing summary.tsv in $input"
    printf '%s/summary.tsv\n' "$input"
  elif [[ -f "$input" ]]; then
    printf '%s\n' "$input"
  else
    die "artifact input not found: $input"
  fi
}

artifact_dir_for() {
  local input="$1"
  if [[ -d "$input" ]]; then
    printf '%s\n' "$input"
  else
    dirname "$input"
  fi
}

tsv_value() {
  local file="$1"
  local field="$2"
  awk -v want="$field" -F '\t' '
    NR == 1 {
      for (i = 1; i <= NF; i++) {
        if ($i == want) field_idx = i
      }
      next
    }
    field_idx && NF {
      print $field_idx
      found = 1
      exit
    }
    END {
      if (!field_idx) exit 2
      if (!found) exit 3
    }' "$file"
}

metadata_value() {
  local artifact_dir="$1"
  local filter="$2"
  local metadata="$artifact_dir/metadata.json"
  if [[ ! -f "$metadata" ]]; then
    printf '\n'
    return
  fi
  jq -r "$filter | if . == null then \"\" else . end" "$metadata"
}

tsv_escape() {
  local value="${1:-}"
  value="${value//$'\t'/ }"
  value="${value//$'\n'/ }"
  printf '%s' "$value"
}

write_tsv_row() {
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

loss_zero_status() {
  local udp200_loss="$1"
  local udp1000_loss="$2"
  local ping_loss="$3"
  awk -v udp200="$udp200_loss" -v udp1000="$udp1000_loss" -v ping="$ping_loss" '
    BEGIN {
      if (udp200 == "" || udp1000 == "" || ping == "") {
        print "unknown";
      } else if ((udp200 + 0) == 0 && (udp1000 + 0) == 0 && (ping + 0) == 0) {
        print "true";
      } else {
        print "false";
      }
    }'
}

hard_event_summary() {
  local artifact_dir="$1"
  local summary="$2"
  local raw_dir hard_file
  raw_dir="$(tsv_value "$summary" raw_dir 2>/dev/null || true)"
  for hard_file in \
    "$raw_dir/nvpn-pipeline-hard-event-totals.tsv" \
    "$artifact_dir/raw/nvpn-pipeline-hard-event-totals.tsv"; do
    [[ -n "$hard_file" && -f "$hard_file" ]] || continue
    awk -F '\t' '
      NR == 1 { next }
      NF >= 3 {
        total += $3 + 0
        if (($3 + 0) > 0) {
          item = $1 ":" $3
          events = events == "" ? item : events ";" item
        }
      }
      END {
        if (events == "") events = "none"
        printf "%s\t%s\n", total + 0, events
      }' "$hard_file"
    return
  done
  for hard_file in \
    "$raw_dir/nvpn-pipeline-phase-summary.tsv" \
    "$artifact_dir/raw/nvpn-pipeline-phase-summary.tsv"; do
    [[ -n "$hard_file" && -f "$hard_file" ]] || continue
    awk -F '\t' '
      NR == 1 {
        for (i = 1; i <= NF; i++) {
          if ($i == "hard_events") hard_events_idx = i
        }
        next
      }
      hard_events_idx && $hard_events_idx != "" {
        event_count = split($hard_events_idx, events, ";")
        for (i = 1; i <= event_count; i++) {
          split(events[i], event_parts, ":")
          event_name = event_parts[1]
          total = ""
          detail_count = split(event_parts[2], details, ",")
          for (j = 1; j <= detail_count; j++) {
            if (index(details[j], "total=") == 1) {
              total = details[j]
              sub(/^total=/, "", total)
            }
          }
          if (event_name != "" && total != "") {
            if (!(event_name in seen)) {
              seen[event_name] = 1
              order[++order_count] = event_name
            }
            event_total[event_name] += total + 0
            total_all += total + 0
          }
        }
      }
      END {
        for (i = 1; i <= order_count; i++) {
          event_name = order[i]
          item = event_name ":" event_total[event_name]
          events_out = events_out == "" ? item : events_out ";" item
        }
        if (events_out == "") events_out = "none"
        printf "%s\t%s\n", total_all + 0, events_out
      }' "$hard_file"
    return
  done
  printf 'n/a\tn/a\n'
}

hard_event_total_from_summary() {
  local hard_total="$1"
  local hard_events="$2"
  local event="$3"
  if [[ "$hard_total" == "n/a" || "$hard_events" == "n/a" ]]; then
    printf 'n/a\n'
    return
  fi
  if [[ "$hard_events" == "none" ]]; then
    printf '0\n'
    return
  fi
  awk -v events="$hard_events" -v want="$event" '
    BEGIN {
      count = split(events, items, ";")
      for (i = 1; i <= count; i++) {
        split(items[i], parts, ":")
        if (parts[1] == want) total += parts[2] + 0
      }
      printf "%s\n", total + 0
    }'
}

hard_event_allowed_for_candidate() {
  local event="$1"
  local token
  local allowed="${NVPN_DOCKER_ALLOW_PIPELINE_HARD_EVENTS:-}"

  case "$event" in
    rx_loop_slow_maintenance_skipped)
      return 0
      ;;
  esac

  allowed="${allowed//,/ }"
  for token in $allowed; do
    [[ "$event" == "$token" ]] && return 0
  done
  return 1
}

blocking_hard_event_total_from_summary() {
  local hard_total="$1"
  local hard_events="$2"
  local item event total blocking_total=0

  if [[ "$hard_total" == "n/a" || "$hard_events" == "n/a" ]]; then
    printf 'n/a\n'
    return
  fi
  if [[ "$hard_events" == "none" ]]; then
    printf '0\n'
    return
  fi

  IFS=';' read -r -a items <<<"$hard_events"
  for item in "${items[@]}"; do
    event="${item%%:*}"
    total="${item#*:}"
    [[ -n "$event" && "$total" != "$item" ]] || continue
    hard_event_allowed_for_candidate "$event" && continue
    blocking_total="$(awk -v a="$blocking_total" -v b="$total" 'BEGIN { print a + b }')"
  done
  printf '%s\n' "$blocking_total"
}

first_raw_artifact_match() {
  local artifact_dir="$1"
  local summary="$2"
  local pattern="$3"
  local raw_dir candidate
  raw_dir="$(tsv_value "$summary" raw_dir 2>/dev/null || true)"
  if [[ -n "$raw_dir" ]]; then
    for candidate in "$raw_dir"/$pattern; do
      [[ -f "$candidate" ]] || continue
      printf '%s\n' "$candidate"
      return
    done
  fi
  for candidate in "$artifact_dir/raw"/$pattern; do
    [[ -f "$candidate" ]] || continue
    printf '%s\n' "$candidate"
    return
  done
  return 0
}

iperf_udp_socket_buffer_summary() {
  local artifact_dir="$1"
  local summary="$2"
  local phase="$3"
  local socket_file
  socket_file="$(first_raw_artifact_match "$artifact_dir" "$summary" '*-iperf-socket-buffers.tsv')"
  if [[ -z "$socket_file" ]]; then
    printf 'n/a\n'
    return
  fi
  awk -F '\t' -v want="$phase" '
    NR == 1 {
      for (i = 1; i <= NF; i++) idx[$i] = i
      next
    }
    idx["phase"] && $(idx["phase"]) == want {
      streams = $(idx["streams"])
      requested = $(idx["requested_sock_bufsize"])
      recv = $(idx["actual_recv_buf"])
      send = $(idx["actual_send_buf"])
      if (streams == "") streams = "n/a"
      if (requested == "") requested = "n/a"
      if (recv == "") recv = "n/a"
      if (send == "") send = "n/a"
      print streams ":" requested "/" recv "/" send
      found = 1
      exit
    }
    END {
      if (!found) print "n/a"
    }' "$socket_file"
}

udp_receiver_limit_summary() {
  local artifact_dir="$1"
  local summary="$2"
  local raw_dir candidate
  local files=()
  raw_dir="$(tsv_value "$summary" raw_dir 2>/dev/null || true)"
  if [[ -n "$raw_dir" ]]; then
    for candidate in "$raw_dir"/*-node-*-udp-receiver-limits.tsv; do
      [[ -f "$candidate" ]] && files+=("$candidate")
    done
  fi
  for candidate in "$artifact_dir/raw"/*-node-*-udp-receiver-limits.tsv; do
    [[ -f "$candidate" ]] && files+=("$candidate")
  done
  if ((${#files[@]} == 0)); then
    printf 'n/a\tn/a\n'
    return
  fi
  awk -F '\t' '
    function file_basename(path, parts, count) {
      count = split(path, parts, "/")
      return parts[count]
    }
    function file_service(path, file, parts, count, service) {
      file = file_basename(path)
      count = split(file, parts, "-node-")
      if (count >= 2) {
        service = "node-" parts[2]
        sub(/-udp-receiver-limits.tsv$/, "", service)
        return service
      }
      return file
    }
    function remember(kind, service, pair, keyed) {
      if (pair == "/") return
      keyed = service "=" pair
      if (kind == "rmem") {
        if (!(pair in rmem_pair_seen)) {
          rmem_pair_seen[pair] = 1
          rmem_pair_order[++rmem_pair_count] = pair
        }
        if (!(keyed in rmem_keyed_seen)) {
          rmem_keyed_seen[keyed] = 1
          rmem_keyed_order[++rmem_keyed_count] = keyed
        }
      } else {
        if (!(pair in wmem_pair_seen)) {
          wmem_pair_seen[pair] = 1
          wmem_pair_order[++wmem_pair_count] = pair
        }
        if (!(keyed in wmem_keyed_seen)) {
          wmem_keyed_seen[keyed] = 1
          wmem_keyed_order[++wmem_keyed_count] = keyed
        }
      }
    }
    function compact(pair_count, pair_order, keyed_count, keyed_order,    i, out) {
      if (pair_count == 0) return "n/a"
      if (pair_count == 1) return pair_order[1]
      for (i = 1; i <= keyed_count; i++) {
        out = out == "" ? keyed_order[i] : out ";" keyed_order[i]
      }
      return out
    }
    function flush(service, rmem_pair, wmem_pair) {
      if (current_file == "") return
      service = file_service(current_file)
      rmem_pair = rmem_default "/" rmem_max
      wmem_pair = wmem_default "/" wmem_max
      remember("rmem", service, rmem_pair)
      remember("wmem", service, wmem_pair)
      rmem_default = ""
      rmem_max = ""
      wmem_default = ""
      wmem_max = ""
    }
    FILENAME != current_file {
      flush()
      current_file = FILENAME
    }
    $1 == "net.core.rmem_default" { rmem_default = $2 }
    $1 == "net.core.rmem_max" { rmem_max = $2 }
    $1 == "net.core.wmem_default" { wmem_default = $2 }
    $1 == "net.core.wmem_max" { wmem_max = $2 }
    END {
      flush()
      printf "%s\t%s\n",
        compact(rmem_pair_count, rmem_pair_order, rmem_keyed_count, rmem_keyed_order),
        compact(wmem_pair_count, wmem_pair_order, wmem_keyed_count, wmem_keyed_order)
    }' "${files[@]}"
}

connected_udp_socket_buffer_summary() {
  local artifact_dir="$1"
  local summary="$2"
  local raw_dir socket_file
  raw_dir="$(tsv_value "$summary" raw_dir 2>/dev/null || true)"
  for socket_file in \
    "$raw_dir/nvpn-connected-udp-socket-buffers.tsv" \
    "$artifact_dir/raw/nvpn-connected-udp-socket-buffers.tsv"; do
    [[ -n "$socket_file" && -f "$socket_file" ]] || continue
    awk -F '\t' '
      function remember(kind, service, req, actual, item, keyed) {
        if (req == "" && actual == "") return
        if (service == "") service = "row" row_count
        item = req "/" actual
        keyed = service "=" item
        if (kind == "recv") {
          if (!(item in recv_pair_seen)) {
            recv_pair_seen[item] = 1
            recv_pair_order[++recv_pair_count] = item
          }
          if (!(keyed in recv_keyed_seen)) {
            recv_keyed_seen[keyed] = 1
            recv_keyed_order[++recv_keyed_count] = keyed
          }
        } else {
          if (!(item in send_pair_seen)) {
            send_pair_seen[item] = 1
            send_pair_order[++send_pair_count] = item
          }
          if (!(keyed in send_keyed_seen)) {
            send_keyed_seen[keyed] = 1
            send_keyed_order[++send_keyed_count] = keyed
          }
        }
      }
      function compact(pair_count, pair_order, keyed_count, keyed_order,    i, out) {
        if (pair_count == 0) return "n/a"
        if (pair_count == 1) return pair_order[1]
        for (i = 1; i <= keyed_count; i++) {
          out = out == "" ? keyed_order[i] : out ";" keyed_order[i]
        }
        return out
      }
      NR == 1 {
        for (i = 1; i <= NF; i++) idx[$i] = i
        next
      }
      NF {
        row_count++
        remember("recv", $(idx["service"]), $(idx["requested_recv_buf"]), $(idx["actual_recv_buf"]))
        remember("send", $(idx["service"]), $(idx["requested_send_buf"]), $(idx["actual_send_buf"]))
      }
      END {
        printf "%s\t%s\n",
          compact(recv_pair_count, recv_pair_order, recv_keyed_count, recv_keyed_order),
          compact(send_pair_count, send_pair_order, send_keyed_count, send_keyed_order)
      }' "$socket_file"
    return
  done
  printf 'n/a\tn/a\n'
}

cpu_phase_file() {
  local artifact_dir="$1"
  local summary="$2"
  local backend threads raw_dir candidate
  local candidates=()

  backend="$(tsv_value "$summary" backend 2>/dev/null || true)"
  threads="$(tsv_value "$summary" threads 2>/dev/null || true)"
  raw_dir="$(tsv_value "$summary" raw_dir 2>/dev/null || true)"

  case "$backend" in
    nvpn)
      [[ -n "$raw_dir" ]] && candidates+=("$raw_dir/nvpn-daemon-cpu-phases.tsv")
      candidates+=("$artifact_dir/raw/nvpn-daemon-cpu-phases.tsv")
      ;;
    wireguard-go)
      [[ -n "$raw_dir" ]] && candidates+=("$raw_dir/wireguard-go-cpu-phases.tsv")
      candidates+=("$artifact_dir/raw/wireguard-go-cpu-phases.tsv")
      ;;
    boringtun)
      if [[ -n "$threads" ]]; then
        [[ -n "$raw_dir" ]] && candidates+=("$raw_dir/boringtun-threads-$threads-cpu-phases.tsv")
        candidates+=("$artifact_dir/raw/boringtun-threads-$threads-cpu-phases.tsv")
      fi
      [[ -n "$raw_dir" ]] && candidates+=("$raw_dir/boringtun-cpu-phases.tsv")
      candidates+=("$artifact_dir/raw/boringtun-cpu-phases.tsv")
      ;;
  esac

  if [[ -n "$backend" ]]; then
    [[ -n "$raw_dir" ]] && candidates+=("$raw_dir/$backend-cpu-phases.tsv")
    candidates+=("$artifact_dir/raw/$backend-cpu-phases.tsv")
  fi

  for candidate in "${candidates[@]}"; do
    [[ -n "$candidate" && -f "$candidate" ]] || continue
    printf '%s\n' "$candidate"
    return
  done
  return 0
}

cpu_phase_metric() {
  local artifact_dir="$1"
  local summary="$2"
  local phase="$3"
  local metric="$4"
  local phase_file
  phase_file="$(cpu_phase_file "$artifact_dir" "$summary")"
  if [[ -z "$phase_file" ]]; then
    printf 'n/a\n'
    return
  fi
  awk -F '\t' -v want_phase="$phase" -v want_metric="$metric" '
    NR == 1 {
      for (i = 1; i <= NF; i++) idx[$i] = i
      next
    }
    idx["phase"] && idx["service"] && idx[want_metric] &&
      $(idx["phase"]) == want_phase && $(idx["service"]) == "both" {
        value = $(idx[want_metric])
        print (value == "" ? "n/a" : value)
        found = 1
        exit
      }
    END {
      if (!found) print "n/a"
    }' "$phase_file"
}

candidate_status() {
  local backend="$1"
  local zero_status="$2"
  local hard_total="$3"
  local blocking_hard_total="$4"
  if [[ "$hard_total" == "n/a" && "$backend" != "nvpn" ]]; then
    printf 'reference\n'
  elif [[ "$zero_status" != "true" ]]; then
    printf 'fail\n'
  elif [[ "$hard_total" == "0" || "$blocking_hard_total" == "0" ]]; then
    printf 'pass\n'
  elif [[ "$hard_total" == "n/a" || "$blocking_hard_total" == "n/a" ]]; then
    printf 'missing-hard-events\n'
  else
    printf 'fail\n'
  fi
}

write_header() {
  write_tsv_row \
    label backend git_head fips_head dirty duration_secs stress placement dataplane \
    tcp_single_mbps tcp_single_retrans tcp_4_mbps tcp_4_retrans tcp_8_mbps tcp_8_retrans \
    tcp_single_cpu_s_per_gbyte tcp_4_cpu_s_per_gbyte tcp_8_cpu_s_per_gbyte \
    udp_200_mbps udp_200_loss_pct udp_1000_mbps udp_1000_loss_pct ping_loss_pct ping_avg_ms \
    iperf_udp200_sockbuf iperf_udp1000_sockbuf udp_receiver_rmem udp_receiver_wmem \
    udp_ping_zero hard_events_total hard_events \
    udp_kernel_dropped_total udp_namespace_rcvbuf_errors_total connected_udp_kernel_dropped_total \
    connected_udp_peer_kernel_dropped_total connected_udp_drain_bulk_dropped_total \
    connected_udp_direct_decrypt_bulk_shed_total connected_udp_recv_buf connected_udp_send_buf \
    candidate artifact
}

write_row() {
  local label="$1"
  local input="$2"
  local summary artifact_dir metadata_backend backend threads duration_secs
  local tcp_single tcp_single_retrans tcp_4 tcp_4_retrans tcp_8 tcp_8_retrans
  local tcp_single_cpu_per_gbyte tcp_4_cpu_per_gbyte tcp_8_cpu_per_gbyte
  local udp_200 udp_200_loss udp_1000 udp_1000_loss ping_loss ping_avg
  local git_head fips_head nvpn_dirty fips_dirty dirty stress placement dataplane
  local hard_total hard_events blocking_hard_total zero_status candidate
  local iperf_udp200_sockbuf iperf_udp1000_sockbuf udp_receiver_rmem udp_receiver_wmem
  local udp_kernel_dropped udp_namespace_rcvbuf_errors connected_udp_kernel_dropped
  local connected_udp_peer_kernel_dropped connected_udp_drain_bulk_dropped connected_udp_direct_decrypt_bulk_shed
  local connected_udp_recv_buf connected_udp_send_buf

  summary="$(summary_file "$input")"
  artifact_dir="$(artifact_dir_for "$input")"
  backend="$(tsv_value "$summary" backend)"
  threads="$(tsv_value "$summary" threads 2>/dev/null || true)"
  metadata_backend="$(metadata_value "$artifact_dir" '.backend')"
  if [[ -n "$metadata_backend" && "$metadata_backend" != "$backend" ]]; then
    die "$label metadata backend '$metadata_backend' does not match summary backend '$backend'"
  fi

  duration_secs="$(tsv_value "$summary" duration_secs)"
  tcp_single="$(tsv_value "$summary" tcp_single_mbps)"
  tcp_single_retrans="$(tsv_value "$summary" tcp_single_retrans)"
  tcp_4="$(tsv_value "$summary" tcp_4_mbps)"
  tcp_4_retrans="$(tsv_value "$summary" tcp_4_retrans)"
  tcp_8="$(tsv_value "$summary" tcp_8_mbps)"
  tcp_8_retrans="$(tsv_value "$summary" tcp_8_retrans)"
  tcp_single_cpu_per_gbyte="$(cpu_phase_metric "$artifact_dir" "$summary" tcp-single cpu_seconds_per_gbyte)"
  tcp_4_cpu_per_gbyte="$(cpu_phase_metric "$artifact_dir" "$summary" tcp-4 cpu_seconds_per_gbyte)"
  tcp_8_cpu_per_gbyte="$(cpu_phase_metric "$artifact_dir" "$summary" tcp-8 cpu_seconds_per_gbyte)"
  udp_200="$(tsv_value "$summary" udp_200_mbps)"
  udp_200_loss="$(tsv_value "$summary" udp_200_loss_pct)"
  udp_1000="$(tsv_value "$summary" udp_1000_mbps)"
  udp_1000_loss="$(tsv_value "$summary" udp_1000_loss_pct)"
  ping_loss="$(tsv_value "$summary" ping_loss_pct)"
  ping_avg="$(tsv_value "$summary" ping_avg_ms)"

  git_head="$(metadata_value "$artifact_dir" '.source.nvpn.git_head')"
  fips_head="$(metadata_value "$artifact_dir" '.source.local_fips_patch.git_head')"
  nvpn_dirty="$(metadata_value "$artifact_dir" '.source.nvpn.dirty')"
  fips_dirty="$(metadata_value "$artifact_dir" '.source.local_fips_patch.dirty')"
  dirty="nvpn=${nvpn_dirty:-unknown},fips=${fips_dirty:-unknown}"
  stress="$(metadata_value "$artifact_dir" '.cpu_stress | if .enabled == true then (.sides + ":l" + (.local_workers|tostring) + "/r" + (.remote_workers|tostring)) else "false" end')"
  placement="$(metadata_value "$artifact_dir" '.run_env.placement_profile')"
  dataplane="$(metadata_value "$artifact_dir" '.run_env.dataplane_profile')"

  iperf_udp200_sockbuf="$(iperf_udp_socket_buffer_summary "$artifact_dir" "$summary" udp-200)"
  iperf_udp1000_sockbuf="$(iperf_udp_socket_buffer_summary "$artifact_dir" "$summary" udp-1000)"
  IFS=$'\t' read -r udp_receiver_rmem udp_receiver_wmem < <(udp_receiver_limit_summary "$artifact_dir" "$summary")
  IFS=$'\t' read -r hard_total hard_events < <(hard_event_summary "$artifact_dir" "$summary")
  blocking_hard_total="$(blocking_hard_event_total_from_summary "$hard_total" "$hard_events")"
  udp_kernel_dropped="$(hard_event_total_from_summary "$hard_total" "$hard_events" udp_kernel_dropped)"
  udp_namespace_rcvbuf_errors="$(hard_event_total_from_summary "$hard_total" "$hard_events" udp_namespace_rcvbuf_errors)"
  connected_udp_kernel_dropped="$(hard_event_total_from_summary "$hard_total" "$hard_events" connected_udp_kernel_dropped)"
  connected_udp_peer_kernel_dropped="$(hard_event_total_from_summary "$hard_total" "$hard_events" connected_udp_peer_kernel_dropped)"
  connected_udp_drain_bulk_dropped="$(hard_event_total_from_summary "$hard_total" "$hard_events" connected_udp_drain_bulk_dropped)"
  connected_udp_direct_decrypt_bulk_shed="$(hard_event_total_from_summary "$hard_total" "$hard_events" connected_udp_direct_decrypt_bulk_shed)"
  IFS=$'\t' read -r connected_udp_recv_buf connected_udp_send_buf < <(connected_udp_socket_buffer_summary "$artifact_dir" "$summary")
  zero_status="$(loss_zero_status "$udp_200_loss" "$udp_1000_loss" "$ping_loss")"
  candidate="$(candidate_status "$backend" "$zero_status" "$hard_total" "$blocking_hard_total")"

  if [[ -n "$threads" ]]; then
    backend="$backend/$threads"
  fi

  write_tsv_row \
    "$(tsv_escape "$label")" \
    "$(tsv_escape "$backend")" \
    "$(tsv_escape "$git_head")" \
    "$(tsv_escape "$fips_head")" \
    "$(tsv_escape "$dirty")" \
    "$(tsv_escape "$duration_secs")" \
    "$(tsv_escape "$stress")" \
    "$(tsv_escape "$placement")" \
    "$(tsv_escape "$dataplane")" \
    "$(tsv_escape "$tcp_single")" \
    "$(tsv_escape "$tcp_single_retrans")" \
    "$(tsv_escape "$tcp_4")" \
    "$(tsv_escape "$tcp_4_retrans")" \
    "$(tsv_escape "$tcp_8")" \
    "$(tsv_escape "$tcp_8_retrans")" \
    "$(tsv_escape "$tcp_single_cpu_per_gbyte")" \
    "$(tsv_escape "$tcp_4_cpu_per_gbyte")" \
    "$(tsv_escape "$tcp_8_cpu_per_gbyte")" \
    "$(tsv_escape "$udp_200")" \
    "$(tsv_escape "$udp_200_loss")" \
    "$(tsv_escape "$udp_1000")" \
    "$(tsv_escape "$udp_1000_loss")" \
    "$(tsv_escape "$ping_loss")" \
    "$(tsv_escape "$ping_avg")" \
    "$(tsv_escape "$iperf_udp200_sockbuf")" \
    "$(tsv_escape "$iperf_udp1000_sockbuf")" \
    "$(tsv_escape "$udp_receiver_rmem")" \
    "$(tsv_escape "$udp_receiver_wmem")" \
    "$(tsv_escape "$zero_status")" \
    "$(tsv_escape "$hard_total")" \
    "$(tsv_escape "$hard_events")" \
    "$(tsv_escape "$udp_kernel_dropped")" \
    "$(tsv_escape "$udp_namespace_rcvbuf_errors")" \
    "$(tsv_escape "$connected_udp_kernel_dropped")" \
    "$(tsv_escape "$connected_udp_peer_kernel_dropped")" \
    "$(tsv_escape "$connected_udp_drain_bulk_dropped")" \
    "$(tsv_escape "$connected_udp_direct_decrypt_bulk_shed")" \
    "$(tsv_escape "$connected_udp_recv_buf")" \
    "$(tsv_escape "$connected_udp_send_buf")" \
    "$(tsv_escape "$candidate")" \
    "$(tsv_escape "$artifact_dir")"
}

write_markdown() {
  local tsv="$1"
  awk -F '\t' '
    function cell(value) {
      gsub(/\|/, "\\|", value)
      return value
    }
    NR == 1 {
      printf "|"
      for (i = 1; i <= NF; i++) printf " %s |", cell($i)
      printf "\n|"
      for (i = 1; i <= NF; i++) printf " --- |"
      printf "\n"
      next
    }
    {
      printf "|"
      for (i = 1; i <= NF; i++) printf " %s |", cell($i)
      printf "\n"
    }' "$tsv"
}

need_cmd jq

rows=()
while (($#)); do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --output-dir)
      shift
      [[ $# -gt 0 ]] || die "--output-dir requires a directory"
      OUTPUT_DIR="$1"
      ;;
    *=*)
      rows+=("$1")
      ;;
    *)
      usage
      die "expected label=artifact-dir, got: $1"
      ;;
  esac
  shift
done

(( ${#rows[@]} > 0 )) || {
  usage
  die "at least one label=artifact-dir row is required"
}

tmp_tsv="$(mktemp)"
trap 'rm -f "$tmp_tsv"' EXIT
write_header >"$tmp_tsv"
for row in "${rows[@]}"; do
  label="${row%%=*}"
  input="${row#*=}"
  [[ -n "$label" && -n "$input" ]] || die "empty label or artifact in '$row'"
  write_row "$label" "$input" >>"$tmp_tsv"
done

if [[ -n "$OUTPUT_DIR" ]]; then
  mkdir -p "$OUTPUT_DIR"
  cp "$tmp_tsv" "$OUTPUT_DIR/stress-table.tsv"
  write_markdown "$tmp_tsv" >"$OUTPUT_DIR/stress-table.md"
  cat "$OUTPUT_DIR/stress-table.md"
else
  write_markdown "$tmp_tsv"
fi
