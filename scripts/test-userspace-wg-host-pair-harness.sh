#!/usr/bin/env bash
# Local self-tests for the userspace WG host-pair baseline harness helpers.
#
# These tests do not create TUN devices or contact a remote host.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

# shellcheck source=scripts/bench-userspace-wg-host-pair.sh
source "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"

fail() {
  printf 'userspace WG host-pair harness self-test failed: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  local got="$1"
  local want="$2"
  local label="$3"
  [[ "$got" == "$want" ]] || fail "$label: got '$got', want '$want'"
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  [[ "$haystack" == *"$needle"* ]] || fail "$label: missing '$needle' in '$haystack'"
}

assert_not_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"
  [[ "$haystack" != *"$needle"* ]] || fail "$label: unexpectedly contained '$needle' in '$haystack'"
}

assert_fails() {
  local label="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$label: command unexpectedly passed"
  fi
}

assert_fails_with() {
  local label="$1"
  local pattern="$2"
  shift 2
  local out
  out="$(mktemp)"
  if "$@" >"$out" 2>&1; then
    cat "$out" >&2
    rm -f "$out"
    fail "$label: command unexpectedly passed"
  fi
  if ! grep -Fq "$pattern" "$out"; then
    cat "$out" >&2
    rm -f "$out"
    fail "$label: expected output to contain '$pattern'"
  fi
  rm -f "$out"
}

fixture_iperf_json() {
  cat <<'JSON'
{
  "end": {
    "sum_sent": {
      "bits_per_second": 123456789,
      "retransmits": 7
    },
    "sum_received": {
      "bits_per_second": 120000000
    }
  }
}
JSON
}

test_backend_start_command() {
  local cmd

  BACKEND=boringtun
  WG_THREADS=
  cmd="$(backend_start_command boringtun /usr/local/bin/boringtun-cli wgbench0 /tmp/tun /tmp/log /tmp/pid)"
  assert_contains "$cmd" "WG_TUN_NAME_FILE=/tmp/tun" "boringtun tun-name env"
  assert_not_contains "$cmd" "WG_THREADS=" "boringtun default leaves threads unset"
  assert_contains "$cmd" "--disable-drop-privileges" "boringtun drop-privileges flag"
  assert_contains "$cmd" "/usr/local/bin/boringtun-cli" "boringtun binary"
  assert_contains "$cmd" "wgbench0" "boringtun interface"

  WG_THREADS=4
  cmd="$(backend_start_command boringtun /usr/local/bin/boringtun-cli wgbench0 /tmp/tun /tmp/log /tmp/pid)"
  assert_contains "$cmd" "WG_THREADS=4" "boringtun explicit threads env"
  assert_eq "$(backend_threads_arg)" "4" "boringtun explicit threads summary"

  WG_THREADS=default
  cmd="$(backend_start_command boringtun /usr/local/bin/boringtun-cli wgbench0 /tmp/tun /tmp/log /tmp/pid)"
  assert_not_contains "$cmd" "WG_THREADS=" "boringtun default label leaves threads unset"
  assert_eq "$(backend_threads_arg)" "" "boringtun default threads summary"

  WG_THREADS=4
  BACKEND=wireguard-go
  cmd="$(backend_start_command wireguard-go /usr/local/bin/wireguard-go utun /tmp/tun /tmp/log /tmp/pid)"
  assert_contains "$cmd" "WG_TUN_NAME_FILE=/tmp/tun" "wireguard-go tun-name env"
  assert_not_contains "$cmd" "WG_THREADS=" "wireguard-go leaves threads unset"
  assert_contains "$cmd" "--foreground" "wireguard-go foreground flag"
  assert_contains "$cmd" "/usr/local/bin/wireguard-go" "wireguard-go binary"
  assert_contains "$cmd" "utun" "wireguard-go interface"
  assert_eq "$(backend_threads_arg)" "" "wireguard-go threads summary"
}

test_ping_parser() {
  local log got
  log="$(mktemp)"
  cat >"$log" <<'EOF'
64 bytes from 10.44.77.2: icmp_seq=1 ttl=64 time=1.0 ms
64 bytes from 10.44.77.2: icmp_seq=2 ttl=64 time=2.0 ms
64 bytes from 10.44.77.2: icmp_seq=3 ttl=64 time=3.0 ms
64 bytes from 10.44.77.2: icmp_seq=4 ttl=64 time=4.0 ms

--- 10.44.77.2 ping statistics ---
4 packets transmitted, 4 packets received, 0.0% packet loss
round-trip min/avg/max/stddev = 1.000/2.500/4.000/1.118 ms
EOF

  got="$(parse_ping_stats "$log")"
  assert_eq "$got" "0.0 2.500 4.000 4.000 4.000" "Darwin ping stats"
  rm -f "$log"
}

test_iperf_parser() {
  local json got
  json="$(mktemp)"
  fixture_iperf_json >"$json"

  got="$(iperf_mbps "$json")"
  assert_eq "$got" "120" "iperf Mbps prefers receiver"

  got="$(iperf_retransmits "$json")"
  assert_eq "$got" "7" "iperf retransmits"
  rm -f "$json"
}

test_validate_backend() {
  BACKEND=boringtun
  WG_THREADS=
  validate_backend
  WG_THREADS=default
  validate_backend
  WG_THREADS=8
  validate_backend
  BACKEND=wireguard-go
  WG_THREADS=
  validate_backend

  assert_fails \
    "invalid backend" \
    bash -c 'source "$1"; BACKEND=bad-backend; validate_backend' \
    bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"

  assert_fails_with \
    "invalid WG thread count" \
    "invalid NVPN_WG_HOST_PAIR_THREADS=bad" \
    bash -c 'source "$1"; BACKEND=boringtun; WG_THREADS=bad; validate_backend' \
    bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"
}

test_cpu_stress_helpers() {
  local cmd got

  csv_has_token " local , remote " local || fail "csv_has_token did not match local"
  csv_has_token " local , remote " remote || fail "csv_has_token did not match remote"
  if csv_has_token " local , remote " both; then
    fail "csv_has_token matched absent token"
  fi

  cmd="$(cpu_stress_start_cmd 2 /tmp/nvpn-cpu-stress.pids)"
  assert_contains "$cmd" "while :; do :; done" "CPU stress busy loop"
  assert_contains "$cmd" "/tmp/nvpn-cpu-stress.pids" "CPU stress pid file"

  cmd="$(cpu_stress_stop_cmd /tmp/nvpn-cpu-stress.pids)"
  assert_contains "$cmd" "kill" "CPU stress stop kills pids"
  assert_contains "$cmd" "rm -f" "CPU stress stop removes pid file"

  got="$(
    bash -c 'source "$1"; CPU_STRESS_WORKERS=3; cpu_stress_worker_count local' \
      bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"
  )"
  assert_eq "$got" "3" "explicit CPU stress worker count"

  assert_fails_with \
    "invalid CPU stress worker count" \
    "NVPN_WG_HOST_PAIR_CPU_STRESS_WORKERS must be a non-negative integer or auto" \
    bash -c 'source "$1"; CPU_STRESS_WORKERS=bad; cpu_stress_worker_count local' \
    bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"

  assert_fails_with \
    "invalid CPU stress side" \
    "NVPN_WG_HOST_PAIR_CPU_STRESS_SIDES must contain local, remote, or both" \
    bash -c 'source "$1"; CPU_STRESS_SIDES=remote,other; validate_cpu_stress_sides' \
    bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"
}

test_cpu_stress_summary_and_metadata_shape() {
  local dir header row got
  dir="$(mktemp -d)"

  OUTPUT_DIR="$dir"
  SUMMARY="$dir/summary.tsv"
  BACKEND=boringtun
  WG_THREADS=2
  CPU_STRESS=1
  CPU_STRESS_SIDES=both
  LOCAL_CPU_STRESS_WORKERS_STARTED=2
  REMOTE_CPU_STRESS_WORKERS_STARTED=3
  LOCAL_IFACE=utun77
  REMOTE_IFACE=wgbench77
  LOCAL_TUNNEL_IP=10.44.77.1
  REMOTE_TUNNEL_IP=10.44.77.2

  write_summary_header
  write_summary_row 0.0 1.0 1.5 2.0 2.5 0.0 1.1 1.6 2.1 2.6 417 4 360 5 12.5 22.5
  write_metadata

  header="$(head -n1 "$SUMMARY")"
  assert_contains "$header" "cpu_stress_enabled" "summary CPU stress enabled header"
  assert_contains "$header" "local_cpu_stress_workers" "summary local stress workers header"
  assert_contains "$header" "remote_cpu_stress_workers" "summary remote stress workers header"

  row="$(tail -n1 "$SUMMARY")"
  IFS=$'\t' read -r -a columns <<<"$row"
  assert_eq "${columns[0]}" "boringtun" "summary backend"
  assert_eq "${columns[1]}" "2" "summary backend threads"
  assert_eq "${columns[2]}" "true" "summary CPU stress enabled"
  assert_eq "${columns[3]}" "both" "summary CPU stress sides"
  assert_eq "${columns[4]}" "2" "summary local CPU stress workers"
  assert_eq "${columns[5]}" "3" "summary remote CPU stress workers"
  assert_eq "${columns[18]}" "417" "summary forward throughput"
  assert_eq "${columns[20]}" "360" "summary reverse throughput"

  got="$(jq -r '.cpu_stress.enabled' "$dir/metadata.json")"
  assert_eq "$got" "true" "metadata CPU stress enabled"
  got="$(jq -r '.cpu_stress.sides' "$dir/metadata.json")"
  assert_eq "$got" "both" "metadata CPU stress sides"
  got="$(jq -r '.cpu_stress.local_workers' "$dir/metadata.json")"
  assert_eq "$got" "2" "metadata local CPU stress workers"
  got="$(jq -r '.cpu_stress.remote_workers' "$dir/metadata.json")"
  assert_eq "$got" "3" "metadata remote CPU stress workers"
  got="$(jq -r '.summary' "$dir/metadata.json")"
  assert_eq "$got" "$SUMMARY" "metadata summary path"
  got="$(jq -r '.threads' "$dir/metadata.json")"
  assert_eq "$got" "2" "metadata explicit backend threads"

  rm -rf "$dir"
}

test_preflight_reports_blockers_without_setup() {
  local dir
  dir="$(mktemp -d)"
  assert_fails_with \
    "preflight blockers" \
    "userspace WG host-pair preflight found blockers" \
    bash -c 'source "$1"; BACKEND=boringtun; LOCAL_BACKEND_BIN=sh; REMOTE_BACKEND_BIN=sh; REMOTE_SSH=""; LOCAL_UNDERLAY_IP=""; REMOTE_UNDERLAY_IP=""; OUTPUT_DIR="$2"; run_preflight' \
    bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh" "$dir"
  [[ -f "$dir/preflight.tsv" ]] || fail "preflight artifact was not written"
  grep -Fq $'missing\tremote SSH target is configured' "$dir/preflight.tsv" \
    || fail "preflight artifact did not record missing SSH target"
  rm -rf "$dir"
}

test_priv_helper_self_test() {
  "$ROOT_DIR/scripts/nvpn-wg-host-pair-priv-helper" self-test >/dev/null
}

test_default_local_priv_helper_path() {
  local dir helper got
  dir="$(mktemp -d)"
  helper="$dir/default-helper"
  printf '#!/usr/bin/env bash\nexit 0\n' >"$helper"
  chmod +x "$helper"

  got="$(
    NVPN_WG_HOST_PAIR_DEFAULT_LOCAL_PRIV_HELPER="$helper" \
      bash -c 'source "$1"; printf "%s" "$LOCAL_PRIV_HELPER"' \
      bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"
  )"
  assert_eq "$got" "$helper" "default local privileged helper path"

  rm -rf "$dir"
}

test_remote_ssh_identity_option() {
  local got
  got="$(
    NVPN_WG_HOST_PAIR_SSH_IDENTITY_FILE=/tmp/nvpn-wg-ref-key \
    NVPN_WG_HOST_PAIR_SSH_KNOWN_HOSTS_FILE=/tmp/nvpn-wg-ref-known-hosts \
    NVPN_WG_HOST_PAIR_SSH_STRICT_HOST_KEY_CHECKING=yes \
      bash -c 'source "$1"; printf "%s\n" "${SSH_OPTS[@]}"' \
      bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"
  )"
  assert_contains "$got" "-i" "SSH identity option flag"
  assert_contains "$got" "/tmp/nvpn-wg-ref-key" "SSH identity option path"
  assert_contains "$got" "UserKnownHostsFile=/tmp/nvpn-wg-ref-known-hosts" "SSH known_hosts option"
  assert_contains "$got" "StrictHostKeyChecking=yes" "SSH strict host key option"
}

test_local_backend_tun_preflight_guard() {
  local got

  got="$(
    NVPN_WG_HOST_PAIR_DEFAULT_LOCAL_PRIV_HELPER=/no/such/helper bash -c '
      source "$1"
      id() { if [[ "${1:-}" == "-u" ]]; then printf "501\n"; else command id "$@"; fi; }
      uname() { printf "Darwin\n"; }
      LOCAL_IFACE_REQUEST=utun
      ASSUME_LOCAL_BACKEND_TUN=0
      if local_backend_tun_available; then printf "ok"; else printf "missing"; fi
    ' bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"
  )"
  assert_eq "$got" "missing" "Darwin utun backend privilege guard"

  got="$(
    NVPN_WG_HOST_PAIR_DEFAULT_LOCAL_PRIV_HELPER=/no/such/helper bash -c '
      source "$1"
      id() { if [[ "${1:-}" == "-u" ]]; then printf "501\n"; else command id "$@"; fi; }
      uname() { printf "Darwin\n"; }
      LOCAL_IFACE_REQUEST=utun
      ASSUME_LOCAL_BACKEND_TUN=1
      if local_backend_tun_available; then printf "ok"; else printf "missing"; fi
    ' bash "$ROOT_DIR/scripts/bench-userspace-wg-host-pair.sh"
  )"
  assert_eq "$got" "ok" "Darwin utun backend guard override"
}

test_local_priv_helper_path() {
  local dir helper log
  dir="$(mktemp -d)"
  helper="$dir/helper"
  log="$dir/helper.log"
  cat >"$helper" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$HELPER_LOG"
case "${1:-}" in
  check|configure-iface|wg-set|wg-show|cleanup-iface) exit 0 ;;
  *) exit 2 ;;
esac
EOF
  chmod +x "$helper"

  sudo() {
    if [[ "${1:-}" == "-n" ]]; then
      shift
    fi
    "$@"
  }

  export HELPER_LOG="$log"
  LOCAL_PRIV_HELPER="$helper"
  LOCAL_IFACE=utun7
  LOCAL_TUNNEL_IP=10.44.77.1
  REMOTE_TUNNEL_IP=10.44.77.2
  WG_MTU=1420
  LOCAL_LISTEN_PORT=51871
  REMOTE_LISTEN_PORT=51872
  REMOTE_UNDERLAY_IP=192.0.2.10
  LOCAL_PRIV_FILE="$dir/local.key"
  printf '%s\n' 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa=' >"$LOCAL_PRIV_FILE"

  local_priv_helper_available || fail "fake local privileged helper was not detected"
  have_local_privilege || fail "fake local privileged helper did not satisfy privilege check"
  configure_local_iface
  configure_local_wg_peer 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb='
  local_wg_show >/dev/null
  cleanup_local_iface

  unset -f sudo

  grep -Fq 'check' "$log" || fail "helper check was not called"
  grep -Fq 'configure-iface utun7 10.44.77.1 10.44.77.2 1420' "$log" \
    || fail "helper configure-iface command not recorded"
  grep -Fq 'wg-set utun7 51871 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb= 10.44.77.2/32 192.0.2.10:51872' "$log" \
    || fail "helper wg-set command not recorded"
  grep -Fq 'wg-show utun7' "$log" || fail "helper wg-show command not recorded"
  grep -Fq 'cleanup-iface utun7 10.44.77.2' "$log" \
    || fail "helper cleanup-iface command not recorded"
  rm -rf "$dir"
}

test_local_backend_helper_path() {
  local dir helper log state
  dir="$(mktemp -d)"
  helper="$dir/helper"
  log="$dir/helper.log"
  state="$dir/state"
  cat >"$helper" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$HELPER_LOG"
case "${1:-}" in
  check|check-backend)
    exit 0
    ;;
  start-backend)
    mkdir -p "$HELPER_STATE"
    printf '%s\n' 424242 >"$HELPER_STATE/backend.pid"
    printf '%s\n' utun9 >"$HELPER_STATE/tun-name"
    printf '%s\n' fake-backend-log >"$HELPER_STATE/backend.log"
    printf '%s\n' "$HELPER_STATE"
    ;;
  stop-backend)
    printf 'stopped\n' >>"$HELPER_STATE/backend.log"
    ;;
  *)
    exit 2
    ;;
esac
EOF
  chmod +x "$helper"

  sudo() {
    if [[ "${1:-}" == "-n" ]]; then
      shift
    fi
    "$@"
  }

  export HELPER_LOG="$log"
  export HELPER_STATE="$state"
  LOCAL_PRIV_HELPER="$helper"
  BACKEND=boringtun
  LOCAL_BACKEND_BIN=boringtun-cli
  LOCAL_IFACE_REQUEST=utun
  WG_THREADS=
  OUTPUT_DIR="$dir/out"

  local_backend_helper_available || fail "fake backend helper was not detected"
  local_backend_tun_available || fail "fake backend helper did not satisfy TUN creation check"
  start_local_backend
  assert_eq "$LOCAL_BACKEND_STATE_DIR" "$state" "helper backend state dir"
  assert_eq "$LOCAL_PID_FILE" "$state/backend.pid" "helper backend pid file"
  assert_eq "$LOCAL_TUN_NAME_FILE" "$state/tun-name" "helper backend tun-name file"
  [[ -f "$OUTPUT_DIR/local-backend.log" ]] || fail "helper backend log was not copied"
  [[ -f "$OUTPUT_DIR/local-backend.pid" ]] || fail "helper backend pid was not copied"
  [[ -f "$OUTPUT_DIR/local-tun-name" ]] || fail "helper backend tun-name was not copied"
  stop_local_backend

  unset -f sudo

  grep -Fq 'check-backend boringtun boringtun-cli' "$log" \
    || fail "helper check-backend command not recorded"
  grep -Eq '^start-backend boringtun boringtun-cli utun ?$' "$log" \
    || fail "helper start-backend command not recorded"
  grep -Fq "stop-backend $state" "$log" \
    || fail "helper stop-backend command not recorded"
  rm -rf "$dir"
}

test_remote_darwin_helper_path() {
  local dir log
  dir="$(mktemp -d)"
  log="$dir/remote.log"

  (
    remote_os() { printf 'Darwin\n'; }
    remote_sh() { printf 'remote-sh %s\n' "$1" >>"$log"; }
    run_remote_priv_helper() { printf 'helper %s\n' "$*" >>"$log"; }
    run_remote_priv_helper_with_stdin() {
      local input="$1"
      shift
      printf 'helper-stdin %s | %s\n' "$input" "$*" >>"$log"
    }
    configure_local_wg_peer() { printf 'local-peer %s\n' "$1" >>"$log"; }

    REMOTE_OS_RESOLVED=Darwin
    REMOTE_IFACE_REQUEST=""
    resolve_remote_iface_request
    assert_eq "$REMOTE_IFACE_REQUEST" "utun" "Darwin remote default interface"

    REMOTE_SSH=macos-fixture
    REMOTE_IFACE=utun8
    REMOTE_BACKEND_STATE_DIR=/opt/nvpn/run/wg-host-pair/backend.fixture
    REMOTE_TUNNEL_IP=10.44.77.2
    LOCAL_TUNNEL_IP=10.44.77.1
    LOCAL_UNDERLAY_IP=192.0.2.1
    LOCAL_LISTEN_PORT=51871
    REMOTE_LISTEN_PORT=51872
    WG_MTU=1060
    LOCAL_PRIV_FILE="$dir/local.key"

    configure_remote_iface
    configure_wg local-private remote-private local-public remote-public
    remote_wg_show >/dev/null
    cleanup_remote_side
  )

  grep -Fq 'helper configure-iface utun8 10.44.77.2 10.44.77.1 1060' "$log" \
    || fail "Darwin remote configure-iface helper command not recorded"
  grep -Fq 'helper-stdin remote-private | wg-set utun8 51872 local-public 10.44.77.1/32 192.0.2.1:51871' "$log" \
    || fail "Darwin remote wg-set helper command not recorded"
  grep -Fq 'helper wg-show utun8' "$log" \
    || fail "Darwin remote wg-show helper command not recorded"
  grep -Fq 'helper cleanup-iface utun8 10.44.77.1' "$log" \
    || fail "Darwin remote cleanup-iface helper command not recorded"
  grep -Fq 'helper stop-backend /opt/nvpn/run/wg-host-pair/backend.fixture' "$log" \
    || fail "Darwin remote stop-backend helper command not recorded"
  rm -rf "$dir"
}

test_remote_linux_wg_set_uses_stdin() {
  local dir log
  dir="$(mktemp -d)"
  log="$dir/remote.log"
  (
    remote_sh() { printf '%s\n' "$1" >"$log"; }
    REMOTE_WG_BIN=/usr/bin/wg
    remote_wg_set_with_stdin remote-private wgbench0 private-key /dev/stdin listen-port 51872 peer local-public allowed-ips 10.44.77.1/32 endpoint 192.0.2.1:51871 persistent-keepalive 25
  )
  grep -Fq '/dev/stdin' "$log" || fail "Linux remote wg-set did not use /dev/stdin"
  if grep -Fq 'remote.key' "$log"; then
    fail "Linux remote wg-set still referenced remote.key"
  fi
  rm -rf "$dir"
}

test_backend_start_command
test_ping_parser
test_iperf_parser
test_validate_backend
test_cpu_stress_helpers
test_cpu_stress_summary_and_metadata_shape
test_preflight_reports_blockers_without_setup
test_priv_helper_self_test
test_default_local_priv_helper_path
test_remote_ssh_identity_option
test_local_backend_tun_preflight_guard
test_local_priv_helper_path
test_local_backend_helper_path
test_remote_darwin_helper_path
test_remote_linux_wg_set_uses_stdin

printf 'userspace WG host-pair harness self-test passed\n'
