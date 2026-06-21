#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

source "$ROOT_DIR/scripts/release_common.sh"
source "$ROOT_DIR/scripts/lib-release-gate-timeout.sh"
enable_deterministic_build_env "$ROOT_DIR"

MACOS_WG_EXIT_TIMEOUT_SECS="${NVPN_RELEASE_GATE_MACOS_WG_EXIT_TIMEOUT_SECS:-300}"
WINDOWS_WG_EXIT_TIMEOUT_SECS="${NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_TIMEOUT_SECS:-900}"
LINUX_GUI_SMOKE_TIMEOUT_SECS="${NVPN_RELEASE_GATE_LINUX_GUI_SMOKE_TIMEOUT_SECS:-1800}"
MACOS_GUI_SMOKE_TIMEOUT_SECS="${NVPN_RELEASE_GATE_MACOS_GUI_SMOKE_TIMEOUT_SECS:-900}"
WINDOWS_GUI_SMOKE_TIMEOUT_SECS="${NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE_TIMEOUT_SECS:-1800}"

release_cargo_config_args=()
release_cargo_lock_args=(--locked)
release_cargo_lock_backup=""
release_fips_path=""

restore_release_cargo_lock() {
  if [[ -n "$release_cargo_lock_backup" ]]; then
    cp "$release_cargo_lock_backup" "$ROOT_DIR/Cargo.lock"
    rm -f "$release_cargo_lock_backup"
    release_cargo_lock_backup=""
  fi
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

  release_cargo_lock_backup="$(mktemp "${TMPDIR:-/tmp}/nvpn-release-gate-Cargo.lock.XXXXXX")"
  cp "$ROOT_DIR/Cargo.lock" "$release_cargo_lock_backup"
  trap restore_release_cargo_lock EXIT
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
    cargo test -p fips-core fresh_control_with_unreturned_endpoint_data_blocks_direct_without_known_fallback -- --nocapture
  )
}

release_cargo() {
  if ((${#release_cargo_config_args[@]})); then
    cargo "${release_cargo_config_args[@]}" "$@"
  else
    cargo "$@"
  fi
}

node scripts/sync-versions.mjs
./scripts/check-rust-file-lines.sh
./scripts/security-audit-rust.sh
cargo fmt --check
prepare_release_cargo_config
run_local_fips_regression_tests
release_cargo clippy "${release_cargo_lock_args[@]}" --workspace --all-targets -- -D warnings
release_cargo test "${release_cargo_lock_args[@]}" --workspace
# Mobile VPN basics run in the blocking gate without requiring a device/emulator:
# join request over FIPS, MagicDNS from a TUN packet, and Android WG socket
# startup ordering before VpnService.protect(fd).
release_cargo test "${release_cargo_lock_args[@]}" -p nostr-vpn-app-core mobile_join_request_sends_and_records_over_real_fips_endpoint
release_cargo test "${release_cargo_lock_args[@]}" -p nostr-vpn-app-core mobile_magic_dns_answers_peer_name_from_tun_packet
release_cargo test "${release_cargo_lock_args[@]}" -p nostr-vpn-app-core mobile_wireguard_exit_dns_forwarders_prefer_configured_tunnel_dns
release_cargo test "${release_cargo_lock_args[@]}" -p nostr-vpn-app-core mobile_wireguard_start_returns_before_handshake_watchdog
release_cargo test "${release_cargo_lock_args[@]}" -p nostr-vpn-app-core mobile_fips_exit_node_routes_default_traffic_to_selected_member
# Shared userspace WG dataplane, including the mpsc channel path used by
# Android VpnService and iOS NEPacketTunnelProvider.
release_cargo test "${release_cargo_lock_args[@]}" -p nostr-vpn-core channels_round_trip_plaintext_packets_against_paired_responder
./scripts/e2e-update-cli.sh

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

release_gate_perf_output_dir() {
  if [[ -n "${NVPN_RELEASE_GATE_PERF_OUTPUT_DIR:-}" ]]; then
    printf '%s\n' "$NVPN_RELEASE_GATE_PERF_OUTPUT_DIR"
  elif [[ -n "${NVPN_PERF_OUTPUT_DIR:-}" ]]; then
    printf '%s\n' "$NVPN_PERF_OUTPUT_DIR"
  else
    printf '%s/artifacts/release-gate-fips-perf-%s\n' \
      "$ROOT_DIR" "$(date -u +%Y%m%dT%H%M%SZ)"
  fi
}

run_wireguard_exit_platform_gates() {
  local macos_wg_exit_available=0
  if [[ "$(uname -s)" == "Darwin" ]] \
    && { [[ "${EUID:-$(id -u)}" == "0" ]] || sudo -n true >/dev/null 2>&1; }; then
    macos_wg_exit_available=1
  fi

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
        if [[ "$macos_wg_exit_available" == "1" ]]; then
          release_gate_run_with_timeout "macOS WG exit e2e" "$MACOS_WG_EXIT_TIMEOUT_SECS" \
            ./scripts/e2e-wireguard-exit-host.sh
        else
          echo "Skipping macOS WG exit e2e because passwordless sudo is unavailable."
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
      release_gate_run_with_timeout "Linux GUI launch smoke" "$LINUX_GUI_SMOKE_TIMEOUT_SECS" \
        ./tools/run-linux ./scripts/e2e-smoke.sh
      ;;
  esac

  local macos_gui_smoke="${NVPN_RELEASE_GATE_MACOS_GUI_SMOKE:-auto}"
  case "$macos_gui_smoke" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping macOS app launch smoke because NVPN_RELEASE_GATE_MACOS_GUI_SMOKE=$macos_gui_smoke"
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      release_gate_run_with_timeout "macOS app launch smoke" "$MACOS_GUI_SMOKE_TIMEOUT_SECS" \
        ./scripts/macos-app-launch-smoke.sh
      ;;
    auto|AUTO|Auto|"")
      if [[ "$(uname -s)" == "Darwin" && -d "$ROOT_DIR/macos/Sources" ]]; then
        release_gate_run_with_timeout "macOS app launch smoke" "$MACOS_GUI_SMOKE_TIMEOUT_SECS" \
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

  local windows_gui_smoke="${NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE:-auto}"
  case "$windows_gui_smoke" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping Windows app launch smoke because NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE=$windows_gui_smoke"
      ;;
    1|true|TRUE|True|yes|YES|Yes|on|ON|On|windows-vm)
      release_gate_run_with_timeout "Windows VM app launch smoke" "$WINDOWS_GUI_SMOKE_TIMEOUT_SECS" \
        ./scripts/windows-vm-app-launch-smoke.sh "${NVPN_WINDOWS_SSH_HOST:-win11-dev}"
      ;;
    auto|AUTO|Auto|"")
      run_auto_windows_vm_app_smoke
      ;;
    *)
      echo "Unsupported NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE=$windows_gui_smoke" >&2
      exit 2
      ;;
  esac
}

case "${NVPN_RELEASE_GATE_DOCKER_E2E:-1}" in
  0|false|FALSE|False|no|NO|No|off|OFF|Off)
    echo "Skipping Docker e2e because NVPN_RELEASE_GATE_DOCKER_E2E=${NVPN_RELEASE_GATE_DOCKER_E2E}"
    ;;
  *)
    # The join-request flow is gated by e2e-bootstrap-discovery-docker.sh, which
    # exercises the same FIPS join-request control frame deterministically over a
    # direct static endpoint. e2e-join-request-docker.sh covers the public
    # open-discovery/relay path but depends on public Nostr relays, so it flakes
    # in CI; run it manually (or in a nightly) rather than in the blocking gate.
    ./scripts/e2e-bootstrap-discovery-docker.sh
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
      ./scripts/e2e-fips-routed-udp-docker.sh
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
      ./scripts/e2e-fips-roaming-docker.sh
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
      ./scripts/e2e-fips-nat-safe-mtu-docker.sh
    ./scripts/e2e-wireguard-exit-docker.sh
    ./scripts/e2e-wireguard-exit-userspace-docker.sh
    case "${NVPN_RELEASE_GATE_PERF_E2E:-1}" in
      0|false|FALSE|False|no|NO|No|off|OFF|Off)
        echo "Skipping Docker perf regression e2e because NVPN_RELEASE_GATE_PERF_E2E=${NVPN_RELEASE_GATE_PERF_E2E}"
        ;;
      *)
        perf_output_dir="$(release_gate_perf_output_dir)"
        echo "Writing Docker FIPS perf artifacts to $perf_output_dir"
        NVPN_FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}" \
          NVPN_PERF_OUTPUT_DIR="$perf_output_dir" \
          ./scripts/e2e-fips-perf-regression-docker.sh
        ;;
    esac
    ;;
esac

./scripts/release-gate-host-pair-latency.sh
./scripts/release-gate-host-pair-loaded-latency.sh
run_wireguard_exit_platform_gates
run_desktop_app_launch_smokes
