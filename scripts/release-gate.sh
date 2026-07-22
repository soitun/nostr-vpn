#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

source "$ROOT_DIR/scripts/release_common.sh"
source "$ROOT_DIR/scripts/lib-release-gate-timeout.sh"
source "$ROOT_DIR/scripts/lib-release-gate-parallel.sh"
source "$ROOT_DIR/scripts/mobile_env.sh"
load_mobile_env "$ROOT_DIR"
enable_deterministic_build_env "$ROOT_DIR"

export NVPN_IDLE_CPU_GATE="${NVPN_RELEASE_GATE_IDLE_CPU:-${NVPN_IDLE_CPU_GATE:-1}}"
export NVPN_IDLE_CPU_MAX_PERCENT="${NVPN_RELEASE_GATE_IDLE_CPU_MAX_PERCENT:-${NVPN_IDLE_CPU_MAX_PERCENT:-2}}"
export NVPN_IDLE_CPU_SAMPLE_SECONDS="${NVPN_RELEASE_GATE_IDLE_CPU_SAMPLE_SECONDS:-${NVPN_IDLE_CPU_SAMPLE_SECONDS:-60}}"
export NVPN_IDLE_CPU_SETTLE_SECONDS="${NVPN_RELEASE_GATE_IDLE_CPU_SETTLE_SECONDS:-${NVPN_IDLE_CPU_SETTLE_SECONDS:-15}}"
# The Android VPN fixture maintains the two production bootstrap adjacencies,
# unlike the foreground/UI idle gates. Keep a separate bound for that active
# encrypted overlay while retaining the packet/TUN correctness probe below.
ANDROID_ACTIVE_OVERLAY_IDLE_CPU_MAX_PERCENT="${NVPN_ANDROID_ACTIVE_OVERLAY_IDLE_CPU_MAX_PERCENT:-4}"

MACOS_WG_EXIT_TIMEOUT_SECS="${NVPN_RELEASE_GATE_MACOS_WG_EXIT_TIMEOUT_SECS:-300}"
WINDOWS_WG_EXIT_TIMEOUT_SECS="${NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_TIMEOUT_SECS:-1800}"
LINUX_GUI_SMOKE_TIMEOUT_SECS="${NVPN_RELEASE_GATE_LINUX_GUI_SMOKE_TIMEOUT_SECS:-1800}"
MACOS_GUI_SMOKE_TIMEOUT_SECS="${NVPN_RELEASE_GATE_MACOS_GUI_SMOKE_TIMEOUT_SECS:-900}"
MACOS_DAEMON_IDLE_CPU_TIMEOUT_SECS="${NVPN_RELEASE_GATE_MACOS_DAEMON_IDLE_CPU_TIMEOUT_SECS:-600}"
WINDOWS_GUI_SMOKE_TIMEOUT_SECS="${NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE_TIMEOUT_SECS:-1800}"
MOBILE_GUI_SMOKE_TIMEOUT_SECS="${NVPN_RELEASE_GATE_MOBILE_GUI_SMOKE_TIMEOUT_SECS:-1800}"
IOS_TUNNEL_IDLE_CPU_TIMEOUT_SECS="${NVPN_RELEASE_GATE_IOS_TUNNEL_IDLE_CPU_TIMEOUT_SECS:-180}"
MOBILE_WG_EXIT_TIMEOUT_SECS="${NVPN_RELEASE_GATE_MOBILE_WG_EXIT_TIMEOUT_SECS:-3600}"

release_cargo_config_args=()
release_cargo_config_backup=""
release_cargo_config_existed=0
release_cargo_config_path="$ROOT_DIR/.cargo/config.toml"
release_cargo_lock_args=(--locked)
release_cargo_lock_backup=""
release_cargo_wrapper_dir=""
release_fips_path=""

restore_release_cargo_lock() {
  if [[ -n "$release_cargo_config_backup" ]]; then
    if [[ "$release_cargo_config_existed" == "1" ]]; then
      cp "$release_cargo_config_backup" "$release_cargo_config_path"
    else
      rm -f "$release_cargo_config_path"
    fi
    rm -f "$release_cargo_config_backup"
    release_cargo_config_backup=""
  fi
  if [[ -n "$release_cargo_lock_backup" ]]; then
    cp "$release_cargo_lock_backup" "$ROOT_DIR/Cargo.lock"
    rm -f "$release_cargo_lock_backup"
    release_cargo_lock_backup=""
  fi
  if [[ -n "$release_cargo_wrapper_dir" ]]; then
    rm -rf "$release_cargo_wrapper_dir"
    release_cargo_wrapper_dir=""
  fi
}

install_release_cargo_wrapper() {
  if ((${#release_cargo_config_args[@]} == 0)) || [[ -n "$release_cargo_wrapper_dir" ]]; then
    return
  fi

  local real_cargo
  real_cargo="$(command -v cargo)"
  release_cargo_wrapper_dir="$(mktemp -d "${TMPDIR:-/tmp}/nvpn-release-gate-cargo.XXXXXX")"
  {
    printf '#!/usr/bin/env bash\n'
    printf 'exec %q' "$real_cargo"
    printf ' %q' "${release_cargo_config_args[@]}"
    printf ' "$@"\n'
  } >"$release_cargo_wrapper_dir/cargo"
  chmod +x "$release_cargo_wrapper_dir/cargo"
  export PATH="$release_cargo_wrapper_dir:$PATH"
}

toml_string() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '"%s"' "$value"
}

install_release_cargo_config() {
  if ((${#release_cargo_config_args[@]} == 0)) || [[ -n "$release_cargo_config_backup" ]]; then
    return
  fi

  mkdir -p "$(dirname "$release_cargo_config_path")"
  release_cargo_config_backup="$(mktemp "${TMPDIR:-/tmp}/nvpn-release-gate-cargo-config.XXXXXX")"
  if [[ -f "$release_cargo_config_path" ]]; then
    release_cargo_config_existed=1
    cp "$release_cargo_config_path" "$release_cargo_config_backup"
  else
    release_cargo_config_existed=0
    : >"$release_cargo_config_backup"
  fi

  cat >"$release_cargo_config_path" <<EOF
[patch.crates-io]
fips-core = { path = $(toml_string "$release_fips_path/crates/fips-core") }
fips-endpoint = { path = $(toml_string "$release_fips_path/crates/fips-endpoint") }
fips-identity = { path = $(toml_string "$release_fips_path/crates/fips-identity") }
EOF
}

prepare_release_cargo_config() {
  if [[ -z "${NVPN_FIPS_REPO_PATH:-}" ]]; then
    return
  fi

  local fips_path="$NVPN_FIPS_REPO_PATH"
  if [[ ! -d "$fips_path" ]]; then
    echo "NVPN_FIPS_REPO_PATH does not exist: $fips_path" >&2
    exit 2
  fi
  fips_path="$(cd "$fips_path" && pwd -P)"
  release_fips_path="$fips_path"
  for crate in fips-core fips-endpoint fips-identity; do
    if [[ ! -f "$fips_path/crates/$crate/Cargo.toml" ]]; then
      echo "NVPN_FIPS_REPO_PATH is missing crates/$crate/Cargo.toml: $fips_path" >&2
      exit 2
    fi
  done

  release_cargo_config_args+=(
    --config "patch.crates-io.fips-core.path=\"$fips_path/crates/fips-core\""
    --config "patch.crates-io.fips-endpoint.path=\"$fips_path/crates/fips-endpoint\""
    --config "patch.crates-io.fips-identity.path=\"$fips_path/crates/fips-identity\""
  )
  echo "Using local FIPS crates from $fips_path"
  case "${NVPN_PATCH_LOCAL_FIPS:-1}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      export NVPN_PATCH_LOCAL_FIPS=1
      echo "Using local FIPS crates in Docker e2e builds."
      ;;
    *)
      cat >&2 <<EOF
NVPN_FIPS_REPO_PATH requires NVPN_PATCH_LOCAL_FIPS=1 during the release gate so
Docker e2e builds test the same local FIPS crates as host Cargo.
EOF
      exit 2
      ;;
  esac
  install_release_cargo_config
  install_release_cargo_wrapper

  release_cargo_lock_backup="$(mktemp "${TMPDIR:-/tmp}/nvpn-release-gate-Cargo.lock.XXXXXX")"
  cp "$ROOT_DIR/Cargo.lock" "$release_cargo_lock_backup"
  if release_cargo metadata --locked --format-version=1 >/dev/null 2>/dev/null; then
    echo "Local FIPS crates satisfy existing Cargo.lock; skipping temporary lock refresh."
    return
  fi

  echo "Preparing temporary Cargo.lock for local FIPS path patches."
  release_cargo metadata --format-version=1 >/dev/null
  if ! release_cargo metadata --offline --format-version=1 >/dev/null; then
    cat >&2 <<EOF
Local FIPS crates do not satisfy Cargo.lock after a temporary metadata refresh.
Publish/update the FIPS crate versions and update nvpn's dependency/lock before
running the release gate with NVPN_FIPS_REPO_PATH.
EOF
    exit 2
  fi
  release_cargo_lock_args=(--offline)
  echo "Using offline Cargo resolution for local FIPS path patches."
}

run_local_fips_regression_tests() {
  if [[ -z "$release_fips_path" ]]; then
    return
  fi

  (
    cd "$release_fips_path"
    cargo test -p fips-core overlay_adverts -- --nocapture
    cargo test -p fips-core update_peers -- --nocapture
    cargo test -p fips-core test_reply_learned_moves_configured_static_direct_peer_when_session_degraded -- --nocapture
    cargo test -p fips-core traversal_path_liveness_keeps_mobile_safe_floor -- --nocapture
    cargo test -p fips-core poll_nostr_discovery_configured_only_drops_nonconfigured_handoff -- --nocapture
    cargo test -p fips-core fresh_control_with_unreturned_endpoint_data_blocks_direct_without_known_fallback -- --nocapture
    cargo test -p fips-core outbound_fmp_send_does_not_refresh_direct_path_liveness -- --nocapture
  )
}

release_cargo() {
  if ((${#release_cargo_config_args[@]})); then
    cargo "${release_cargo_config_args[@]}" "$@"
  else
    cargo "$@"
  fi
}

release_cargo_test_filter() (
  set -o pipefail
  local package="$1"
  local filter="$2"
  local output
  output="$(mktemp "${TMPDIR:-/tmp}/nvpn-release-gate-test.XXXXXX")"
  if ! release_cargo test "${release_cargo_lock_args[@]}" -p "$package" "$filter" 2>&1 \
    | tee "$output"; then
    rm -f "$output"
    return 1
  fi
  if ! grep -E "^test .*${filter} .*\\.\\.\\. ok$" "$output" >/dev/null; then
    printf 'Release gate test selector matched no passing test: %s (%s)\n' \
      "$filter" "$package" >&2
    rm -f "$output"
    return 1
  fi
  rm -f "$output"
)

run_release_gate_preflight() {
  node scripts/sync-versions.mjs
  npm ci
  npm run check
  npm run build
  ./scripts/check-source-file-lines.sh
  ./scripts/test-release-gate-parallel-harness.sh
  ./scripts/test-idle-cpu-gate-harness.sh
  ./scripts/test-macos-sdk-compat-harness.sh
  cargo fmt --check
}

run_rust_validation_lane() {
  ./scripts/security-audit-rust.sh
  run_local_fips_regression_tests
  release_cargo clippy "${release_cargo_lock_args[@]}" --workspace --all-targets -- -D warnings
  export RUST_MIN_STACK="${RUST_MIN_STACK:-8388608}"
  # This fixture contains a strict end-to-end latency assertion. Keep it out of
  # the workspace suite while the cold Docker image may be compiling, then run
  # and measure it once after joining that build below.
  release_cargo test "${release_cargo_lock_args[@]}" --workspace -- \
    --test-threads=1 \
    --skip websocket_seed_router_delivers_join_roster_to_guest_without_preconfigured_admin
  # Mobile VPN basics run without requiring a device/emulator: join over FIPS,
  # MagicDNS from TUN, exit DNS policy, and Android WG socket startup ordering.
  release_cargo_test_filter nostr-vpn-app-core mobile_join_request_sends_and_records_over_real_fips_endpoint
  release_cargo_test_filter nostr-vpn-app-core mobile_magic_dns_answers_peer_name_from_tun_packet
  release_cargo_test_filter nostr-vpn-app-core mobile_config_wireguard_exit_keeps_local_stub_and_uses_profile_dns_only_while_active
  release_cargo_test_filter nostr-vpn-core exit_dns_supported_policy_matrix_selects_exact_resolver
  release_cargo_test_filter nostr-vpn-app-core settings_patch_validates_exit_dns_atomically_and_exposes_saved_policy
  release_cargo_test_filter nostr-vpn-app-core mobile_wireguard_start_returns_before_handshake_watchdog
  release_cargo_test_filter nostr-vpn-app-core mobile_fips_exit_node_routes_default_traffic_to_selected_member
  # Shared userspace WG dataplane, including the mpsc channel path used by
  # Android VpnService and iOS NEPacketTunnelProvider.
  release_cargo_test_filter nostr-vpn-core channels_round_trip_plaintext_packets_against_paired_responder
  ./scripts/e2e-update-cli.sh
}

run_auto_windows_vm_app_smoke() {
  local host="${NVPN_WINDOWS_SSH_HOST:-win11-dev}"
  if ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" hostname >/dev/null 2>&1; then
    release_gate_run_with_timeout "Windows VM app launch smoke" "$WINDOWS_GUI_SMOKE_TIMEOUT_SECS" \
      ./scripts/windows-vm-app-launch-smoke.sh "$host"
  else
    echo "Skipping Windows VM app launch smoke because ssh $host is unreachable."
  fi
}

run_auto_windows_vm_wireguard_exit_e2e() {
  local host="${NVPN_WINDOWS_SSH_HOST:-win11-dev}"
  if ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" hostname >/dev/null 2>&1; then
    release_gate_run_with_timeout "Windows WG exit e2e" "$WINDOWS_WG_EXIT_TIMEOUT_SECS" \
      ./scripts/windows-vm-wireguard-exit-e2e.sh "$host"
  else
    echo "Skipping Windows WG exit e2e because ssh $host is unreachable."
  fi
}

release_gate_mode_disabled() {
  case "${1:-}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off) return 0 ;;
    *) return 1 ;;
  esac
}

windows_platform_lane_requested() {
  ! release_gate_mode_disabled "${NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_E2E:-auto}" \
    || ! release_gate_mode_disabled "${NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE:-auto}"
}

prepare_windows_platform_lane_sync() {
  WINDOWS_LANE_PRE_SYNCED=0
  windows_platform_lane_requested || return 0

  local host="${NVPN_WINDOWS_SSH_HOST:-win11-dev}"
  if ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" hostname >/dev/null 2>&1; then
    ./scripts/windows-vm-git-sync.sh "$host"
    WINDOWS_LANE_PRE_SYNCED=1
  fi
}

run_windows_wireguard_exit_gate() {
  case "${NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_E2E:-auto}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping Windows WG exit e2e because NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_E2E=${NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_E2E}"
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On|windows-vm)
      release_gate_run_with_timeout "Windows WG exit e2e" "$WINDOWS_WG_EXIT_TIMEOUT_SECS" \
        ./scripts/windows-vm-wireguard-exit-e2e.sh "${NVPN_WINDOWS_SSH_HOST:-win11-dev}"
      ;;
    auto|AUTO|Auto|"")
      run_auto_windows_vm_wireguard_exit_e2e
      ;;
    *)
      echo "Unsupported NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_E2E=${NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_E2E}" >&2
      return 2
      ;;
  esac
}

run_windows_app_launch_gate() {
  local mode="${NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE:-auto}"
  case "$mode" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping Windows app launch smoke because NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE=$mode"
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On|windows-vm)
      release_gate_run_with_timeout "Windows VM app launch smoke" "$WINDOWS_GUI_SMOKE_TIMEOUT_SECS" \
        ./scripts/windows-vm-app-launch-smoke.sh "${NVPN_WINDOWS_SSH_HOST:-win11-dev}"
      ;;
    auto|AUTO|Auto|"")
      run_auto_windows_vm_app_smoke
      ;;
    *)
      echo "Unsupported NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE=$mode" >&2
      return 2
      ;;
  esac
}

run_windows_platform_lane() {
  if [[ "${WINDOWS_LANE_PRE_SYNCED:-0}" == "1" ]]; then
    export NVPN_WINDOWS_SKIP_GIT_SYNC=1
  fi
  run_windows_wireguard_exit_gate
  run_windows_app_launch_gate
}

release_gate_perf_output_dir() {
  if [[ -n "${NVPN_RELEASE_GATE_PERF_OUTPUT_DIR:-}" ]]; then
    printf '%s\n' "$NVPN_RELEASE_GATE_PERF_OUTPUT_DIR"
  elif [[ -n "${NVPN_PERF_OUTPUT_DIR:-}" ]]; then
    printf '%s\n' "$NVPN_PERF_OUTPUT_DIR"
  else
    printf '%s/artifacts/release-gate-nvpn-fips-perf-%s\n' \
      "$ROOT_DIR" "$(date -u +%Y%m%dT%H%M%SZ)"
  fi
}

run_wireguard_exit_platform_gates() {
  case "${NVPN_RELEASE_GATE_MACOS_WG_EXIT_E2E:-auto}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping macOS WG exit e2e because NVPN_RELEASE_GATE_MACOS_WG_EXIT_E2E=${NVPN_RELEASE_GATE_MACOS_WG_EXIT_E2E}"
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      release_gate_run_with_timeout "macOS WG exit e2e" "$MACOS_WG_EXIT_TIMEOUT_SECS" \
        ./scripts/e2e-wireguard-exit-host.sh
      ;;
    auto|AUTO|Auto|"")
      if [[ "$(uname -s)" == "Darwin" ]]; then
        if [[ "${EUID:-$(id -u)}" == "0" ]]; then
          release_gate_run_with_timeout "macOS WG exit e2e" "$MACOS_WG_EXIT_TIMEOUT_SECS" \
            ./scripts/e2e-wireguard-exit-host.sh
        else
          echo "Skipping macOS WG exit e2e in auto mode because privileged access is not predeclared."
          echo "Set NVPN_RELEASE_GATE_MACOS_WG_EXIT_E2E=1 to run the exact sudo-scoped test command."
        fi
      else
        echo "Skipping macOS WG exit e2e on this host."
      fi
      ;;
    *)
      echo "Unsupported NVPN_RELEASE_GATE_MACOS_WG_EXIT_E2E=${NVPN_RELEASE_GATE_MACOS_WG_EXIT_E2E}" >&2
      exit 2
      ;;
  esac

}

run_desktop_app_launch_smokes() {
  local linux_gui_smoke_default=1
  case "${NVPN_RELEASE_GATE_DOCKER_E2E:-1}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      linux_gui_smoke_default=0
      ;;
  esac

  local linux_gui_smoke="${NVPN_RELEASE_GATE_LINUX_GUI_SMOKE:-$linux_gui_smoke_default}"
  case "$linux_gui_smoke" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping Linux GUI launch smoke because NVPN_RELEASE_GATE_LINUX_GUI_SMOKE=$linux_gui_smoke"
      ;;
    *)
      if [[ -n "$release_fips_path" ]]; then
        release_gate_run_with_timeout "Linux GUI launch smoke" "$LINUX_GUI_SMOKE_TIMEOUT_SECS" \
          env NVPN_LINUX_NONINTERACTIVE=1 NVPN_LINUX_FIPS_REPO_PATH="$release_fips_path" \
          ./tools/run-linux env NVPN_PATCH_LOCAL_FIPS=1 NVPN_FIPS_REPO_PATH=/workspace/fips ./scripts/e2e-smoke.sh
      else
        release_gate_run_with_timeout "Linux GUI launch smoke" "$LINUX_GUI_SMOKE_TIMEOUT_SECS" \
          env NVPN_LINUX_NONINTERACTIVE=1 ./tools/run-linux ./scripts/e2e-smoke.sh
      fi
      ;;
  esac

  local macos_gui_smoke="${NVPN_RELEASE_GATE_MACOS_GUI_SMOKE:-auto}"
  case "$macos_gui_smoke" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping macOS app launch smoke because NVPN_RELEASE_GATE_MACOS_GUI_SMOKE=$macos_gui_smoke"
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      release_gate_run_with_timeout "macOS app launch smoke" "$MACOS_GUI_SMOKE_TIMEOUT_SECS" \
        env NVPN_MACOS_RUST_PROFILE=release NVPN_MACOS_XCODE_CONFIGURATION=Release \
        ./scripts/macos-app-launch-smoke.sh
      ;;
    auto|AUTO|Auto|"")
      if [[ "$(uname -s)" == "Darwin" && -d "$ROOT_DIR/macos/Sources" ]]; then
        release_gate_run_with_timeout "macOS app launch smoke" "$MACOS_GUI_SMOKE_TIMEOUT_SECS" \
          env NVPN_MACOS_RUST_PROFILE=release NVPN_MACOS_XCODE_CONFIGURATION=Release \
          ./scripts/macos-app-launch-smoke.sh
      else
        echo "Skipping macOS app launch smoke on this host."
      fi
      ;;
    *)
      echo "Unsupported NVPN_RELEASE_GATE_MACOS_GUI_SMOKE=$macos_gui_smoke" >&2
      exit 2
      ;;
  esac

}

run_macos_daemon_idle_cpu_gate() {
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "Skipping macOS daemon idle CPU gate on this host."
    return
  fi
  if ! { [[ "${EUID:-$(id -u)}" == "0" ]] \
    || sudo -n /usr/local/sbin/nvpn-install-test-daemon --help >/dev/null 2>&1; }; then
    echo "Skipping macOS daemon idle CPU gate because passwordless sudo is unavailable."
    return
  fi
  release_gate_run_with_timeout "macOS daemon idle CPU" "$MACOS_DAEMON_IDLE_CPU_TIMEOUT_SECS" \
    env NVPN_RUN_MACOS_SERVICE_E2E=1 ./scripts/e2e-macos-service.sh
}

run_mobile_idle_cpu_gates() {
  case "$NVPN_IDLE_CPU_GATE" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping mobile idle CPU gates because NVPN_IDLE_CPU_GATE=$NVPN_IDLE_CPU_GATE"
      return
      ;;
  esac

  if [[ "$(uname -s)" == "Darwin" ]]; then
    release_gate_run_with_timeout "iOS simulator idle CPU smoke" "$MOBILE_GUI_SMOKE_TIMEOUT_SECS" \
      env NVPN_IOS_RUST_PROFILE=release ./scripts/mobile-ios-smoke.sh simulator
  else
    echo "Skipping iOS simulator idle CPU smoke on this host."
  fi

  if command -v adb >/dev/null 2>&1 \
    && adb devices 2>/dev/null | awk 'NR > 1 && $2 == "device" { found = 1 } END { exit !found }'; then
    release_gate_run_with_timeout "Android idle CPU smoke" "$MOBILE_GUI_SMOKE_TIMEOUT_SECS" \
      env NVPN_ANDROID_PACKAGE="fi.siriusbusiness.nvpn.releasegate" \
      NVPN_IDLE_CPU_MAX_PERCENT="$ANDROID_ACTIVE_OVERLAY_IDLE_CPU_MAX_PERCENT" \
      ./scripts/mobile-android-smoke.sh --vpn-cycle --create-network --accept-vpn-dialog
  else
    echo "Skipping Android idle CPU smoke because no adb device is online."
  fi

  local ios_device="${NVPN_IOS_DEVICE:-${NVPN_IOS_DEVICE_ID:-}}"
  if [[ "$(uname -s)" == "Darwin" && -n "$ios_device" ]]; then
    release_gate_run_with_timeout "iOS packet tunnel idle CPU" "$IOS_TUNNEL_IDLE_CPU_TIMEOUT_SECS" \
      env \
        NVPN_IOS_RUST_PROFILE=release \
        NVPN_IOS_IDLE_CPU_MAX_PERCENT="${NVPN_IOS_PACKET_TUNNEL_IDLE_CPU_MAX_PERCENT:-$NVPN_IDLE_CPU_MAX_PERCENT}" \
        NVPN_IOS_IDLE_CPU_SETTLE_SECONDS="${NVPN_IOS_PACKET_TUNNEL_IDLE_CPU_SETTLE_SECONDS:-15}" \
        NVPN_IOS_IDLE_CPU_SAMPLE_SECONDS="${NVPN_IOS_PACKET_TUNNEL_IDLE_CPU_SAMPLE_SECONDS:-60}" \
        ./scripts/mobile-ios-smoke.sh device --device "$ios_device" --install --create-network --vpn-cycle
  else
    echo "Skipping iOS packet tunnel idle CPU gate because no physical device is configured."
  fi
}

run_mobile_wireguard_exit_gates() {
  local mode="${NVPN_RELEASE_GATE_MOBILE_WG_EXIT_E2E:-auto}"
  case "$mode" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping mobile WireGuard exit e2e because NVPN_RELEASE_GATE_MOBILE_WG_EXIT_E2E=$mode"
      return
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      ;;
    auto|AUTO|Auto|"")
      if [[ "$(uname -s)" != "Darwin" ]] \
        || ! command -v docker >/dev/null 2>&1 \
        || ! command -v wg >/dev/null 2>&1 \
        || ! command -v adb >/dev/null 2>&1 \
        || ! adb devices 2>/dev/null | awk 'NR > 1 && $2 == "device" && $1 !~ /^emulator-/ { found = 1 } END { exit !found }' \
        || ! xcrun xctrace list devices 2>/dev/null | awk '
          /^== Devices ==/ { devices = 1; next }
          /^== Devices Offline ==/ { devices = 0 }
          devices && /iPhone|iPad/ { found = 1 }
          END { exit !found }
        '
      then
        echo "Skipping mobile WireGuard exit e2e because both physical mobile devices and the local fixture tools are not available."
        return
      fi
      ;;
    *)
      echo "Unsupported NVPN_RELEASE_GATE_MOBILE_WG_EXIT_E2E=$mode" >&2
      exit 2
      ;;
  esac

  release_gate_run_with_timeout "Android/iOS WireGuard exit e2e" "$MOBILE_WG_EXIT_TIMEOUT_SECS" \
    env NVPN_ANDROID_IDLE_CPU_MAX_PERCENT="$ANDROID_ACTIVE_OVERLAY_IDLE_CPU_MAX_PERCENT" \
    ./scripts/mobile-wireguard-exit-e2e.sh all
}

docker_release_gates_enabled() {
  ! release_gate_mode_disabled "${NVPN_RELEASE_GATE_DOCKER_E2E:-1}"
}

build_release_gate_docker_node_image() {
  docker compose \
    -p nostr-vpn-release-gate-image \
    -f "$ROOT_DIR/docker-compose.e2e.yml" \
    build node-a
}

run_docker_signal_gates() {
  if ! docker_release_gates_enabled; then
    echo "Skipping Docker e2e because NVPN_RELEASE_GATE_DOCKER_E2E=${NVPN_RELEASE_GATE_DOCKER_E2E}"
    return
  fi

  NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_ROUTED_UDP_DISCOVERY_POLICY:-open}" \
    ./scripts/e2e-fips-routed-udp-docker.sh
  NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
    ./scripts/e2e-fips-roaming-docker.sh
}

run_userspace_wireguard_exit_docker_gate() {
  # The kernel and userspace fixtures use the same Compose topology. Give the
  # userspace copy its own benchmark-network subnets so Docker can create both
  # projects concurrently without overlapping IPAM pools.
  env \
    NVPN_WG_EXIT_INTERNET_SUBNET="${NVPN_WG_EXIT_USERSPACE_INTERNET_SUBNET:-198.19.243.0/24}" \
    NVPN_WG_EXIT_PUBLIC_SUBNET="${NVPN_WG_EXIT_USERSPACE_PUBLIC_SUBNET:-198.19.244.0/24}" \
    NVPN_WG_EXIT_UPSTREAM_IP="${NVPN_WG_EXIT_USERSPACE_UPSTREAM_IP:-198.19.243.20}" \
    NVPN_WG_EXIT_UPSTREAM_PUBLIC_IP="${NVPN_WG_EXIT_USERSPACE_UPSTREAM_PUBLIC_IP:-198.19.244.20}" \
    NVPN_WG_EXIT_NODE_A_IP="${NVPN_WG_EXIT_USERSPACE_NODE_A_IP:-198.19.243.10}" \
    NVPN_WG_EXIT_NODE_B_IP="${NVPN_WG_EXIT_USERSPACE_NODE_B_IP:-198.19.243.11}" \
    NVPN_WG_EXIT_TARGET_IP="${NVPN_WG_EXIT_USERSPACE_TARGET_IP:-198.19.244.100}" \
    ./scripts/e2e-wireguard-exit-userspace-docker.sh
}

run_docker_isolated_functional_gates() {
  docker_release_gates_enabled || return 0

  local lanes=()
  release_gate_parallel_start "Docker NAT-safe MTU" \
    env NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
    ./scripts/e2e-fips-nat-safe-mtu-docker.sh
  lanes+=("$RELEASE_GATE_PARALLEL_LAST_INDEX")

  release_gate_parallel_start "Docker kernel WireGuard exit" \
    ./scripts/e2e-wireguard-exit-docker.sh
  lanes+=("$RELEASE_GATE_PARALLEL_LAST_INDEX")

  release_gate_parallel_start "Docker userspace WireGuard exit" \
    run_userspace_wireguard_exit_docker_gate
  lanes+=("$RELEASE_GATE_PARALLEL_LAST_INDEX")

  local lane
  for lane in "${lanes[@]}"; do
    release_gate_parallel_wait "$lane"
  done
}

run_docker_perf_gate() {
  docker_release_gates_enabled || return 0
  case "${NVPN_RELEASE_GATE_PERF_E2E:-1}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping Docker perf regression e2e because NVPN_RELEASE_GATE_PERF_E2E=${NVPN_RELEASE_GATE_PERF_E2E}"
      ;;
    *)
      local perf_output_dir
      perf_output_dir="$(release_gate_perf_output_dir)"
      echo "Writing Docker nvpn+FIPS perf artifacts to $perf_output_dir"
      NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
        NVPN_PERF_OUTPUT_DIR="$perf_output_dir" \
        ./scripts/e2e-fips-perf-regression-docker.sh
      ;;
  esac
}

release_gate_cleanup() {
  release_gate_parallel_cancel_all
  restore_release_cargo_lock
}

main() {
  local started_at
  started_at="$(date +%s)"
  local log_dir="${NVPN_RELEASE_GATE_LOG_DIR:-$ROOT_DIR/artifacts/release-gate-logs/$(date -u +%Y%m%dT%H%M%SZ)}"
  release_gate_parallel_init "$log_dir"
  trap release_gate_cleanup EXIT

  # Fail fast on cheap deterministic checks before dispatching remote or heavy
  # work, then sync one immutable candidate to the Windows VM exactly once.
  run_release_gate_preflight
  prepare_windows_platform_lane_sync

  local windows_lane=""
  if windows_platform_lane_requested; then
    release_gate_parallel_start "Windows platform" run_windows_platform_lane
    windows_lane="$RELEASE_GATE_PARALLEL_LAST_INDEX"
  fi

  prepare_release_cargo_config

  # The remote Windows lane and the shared Docker image build do not consume
  # measurement devices. Build them alongside host Rust validation, then join
  # the image before any Docker functional/performance test starts.
  local docker_build_lane=""
  if docker_release_gates_enabled; then
    export NVPN_E2E_NODE_IMAGE="${NVPN_RELEASE_GATE_E2E_NODE_IMAGE:-${NVPN_E2E_NODE_IMAGE:-nostr-vpn-e2e-node}}"
    export NVPN_EXIT_NODE_E2E_IMAGE="$NVPN_E2E_NODE_IMAGE"
    release_gate_parallel_start "Docker node image build" build_release_gate_docker_node_image
    docker_build_lane="$RELEASE_GATE_PARALLEL_LAST_INDEX"
  fi

  run_rust_validation_lane

  if [[ -n "$docker_build_lane" ]]; then
    release_gate_parallel_wait "$docker_build_lane"
    export NVPN_E2E_SKIP_NODE_BUILD=1
    export NVPN_PERF_SKIP_BUILD=1
  fi

  release_cargo_test_filter nostr-vpn-app-core \
    websocket_seed_router_delivers_join_roster_to_guest_without_preconfigured_admin

  # Routed idle CPU and roaming remain serial. The remaining functional Docker
  # projects have isolated names/subnets and no timing assertions, so overlap
  # them and join before throughput or any host/device measurement begins.
  run_docker_signal_gates
  run_docker_isolated_functional_gates
  run_docker_perf_gate
  ./scripts/release-gate-host-pair-latency.sh
  ./scripts/release-gate-host-pair-loaded-latency.sh
  run_wireguard_exit_platform_gates
  run_desktop_app_launch_smokes
  run_macos_daemon_idle_cpu_gate
  run_mobile_idle_cpu_gates
  run_mobile_wireguard_exit_gates

  if [[ -n "$windows_lane" ]]; then
    release_gate_parallel_wait "$windows_lane"
  fi

  printf 'Release gate passed in %ss. Lane logs: %s\n' \
    "$(( $(date +%s) - started_at ))" "$log_dir"
}

main "$@"
