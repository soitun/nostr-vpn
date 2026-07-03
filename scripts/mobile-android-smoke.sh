#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/scripts/mobile_env.sh"
load_mobile_env "$ROOT"
PACKAGE_NAME="${NVPN_ANDROID_PACKAGE:-org.nostrvpn.app}"
MAIN_ACTIVITY="${NVPN_ANDROID_ACTIVITY:-org.nostrvpn.app/.MainActivity}"
DEBUG_ACTION_EXTRA="${NVPN_ANDROID_DEBUG_ACTION_EXTRA:-org.nostrvpn.app.DEBUG_ACTION}"
DEBUG_INVITE_EXTRA="${NVPN_ANDROID_DEBUG_INVITE_EXTRA:-org.nostrvpn.app.DEBUG_INVITE}"
DEBUG_EXIT_NODE_EXTRA="${NVPN_ANDROID_DEBUG_EXIT_NODE_EXTRA:-org.nostrvpn.app.DEBUG_EXIT_NODE}"
DEBUG_WIREGUARD_CONFIG_BASE64_EXTRA="${NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_BASE64_EXTRA:-org.nostrvpn.app.DEBUG_WIREGUARD_CONFIG_BASE64}"
APK_PATH="${NVPN_ANDROID_APK:-$ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
VPN_START_WAIT_SECS="${NVPN_ANDROID_VPN_START_WAIT_SECS:-15}"
VPN_STOP_WAIT_SECS="${NVPN_ANDROID_VPN_STOP_WAIT_SECS:-10}"
DEBUG_SEED_WAIT_SECS="${NVPN_ANDROID_DEBUG_SEED_WAIT_SECS:-2}"
DEBUG_INVITE="${NVPN_ANDROID_DEBUG_INVITE:-}"
DEBUG_EXIT_NODE="${NVPN_ANDROID_DEBUG_EXIT_NODE:-}"
DEBUG_WIREGUARD_CONFIG="${NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG:-}"
DEBUG_WIREGUARD_CONFIG_FILE="${NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_FILE:-}"

build=1
clear_state=0
vpn_cycle=0
serial="${NVPN_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-android-smoke.sh [--no-build] [--clear] [--vpn-cycle] [--serial SERIAL]

Builds and installs the debug APK, launches the app through adb, and optionally
cycles the debug VPN action. Values may live in .env.mobile.local, shell env,
or --serial. Keep device identifiers and signing details out of committed files.

First-run Android VPN permission prompts may need manual approval before
--vpn-cycle can run unattended.

For fresh installs, --vpn-cycle needs a usable app config. Seed one privately with
NVPN_ANDROID_DEBUG_INVITE, or set a WireGuard exit with
NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG / NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_FILE.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build)
      build=0
      ;;
    --clear)
      clear_state=1
      ;;
    --vpn-cycle)
      vpn_cycle=1
      ;;
    --serial)
      if [[ $# -lt 2 ]]; then
        echo "--serial requires a value" >&2
        exit 2
      fi
      serial="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

sdk_from_local_properties() {
  local file="$ROOT/android/local.properties"
  if [[ -f "$file" ]]; then
    sed -n 's/^sdk\.dir=//p' "$file" | head -n 1
  fi
}

resolve_adb() {
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "$sdk" ]]; then
    sdk="$(sdk_from_local_properties)"
  fi
  if [[ -z "$sdk" && -d "$HOME/Library/Android/sdk" ]]; then
    sdk="$HOME/Library/Android/sdk"
  fi
  if [[ -n "$sdk" && -x "$sdk/platform-tools/adb" ]]; then
    printf '%s\n' "$sdk/platform-tools/adb"
    return
  fi
  if command -v adb >/dev/null 2>&1; then
    command -v adb
    return
  fi
  echo "adb not found; set ANDROID_HOME/ANDROID_SDK_ROOT or add adb to PATH" >&2
  exit 1
}

select_serial() {
  local adb="$1"
  if [[ -n "$serial" ]]; then
    printf '%s\n' "$serial"
    return
  fi
  "$adb" devices | awk 'NR > 1 && $2 == "device" { print $1; exit }'
}

vpn_service_running() {
  "$ADB" -s "$serial" shell dumpsys activity services "$PACKAGE_NAME" 2>/dev/null \
    | tr -d '\r' \
    | grep -q 'NostrVpnService'
}

vpn_network_active() {
  "$ADB" -s "$serial" shell dumpsys connectivity 2>/dev/null \
    | tr -d '\r' \
    | grep -Eq 'NetworkAgentInfo\{.*Transports: VPN'
}

vpn_active() {
  vpn_service_running && vpn_network_active
}

vpn_state_present() {
  vpn_service_running || vpn_network_active
}

wait_until() {
  local timeout="$1"
  shift
  local start now
  start="$(date +%s)"
  while true; do
    if "$@"; then
      return 0
    fi
    now="$(date +%s)"
    if (( now - start >= timeout )); then
      return 1
    fi
    sleep 1
  done
}

vpn_inactive() {
  ! vpn_state_present
}

base64_no_wrap() {
  base64 | tr -d '\n'
}

wireguard_config() {
  if [[ -n "$DEBUG_WIREGUARD_CONFIG_FILE" ]]; then
    cat "$DEBUG_WIREGUARD_CONFIG_FILE"
    return
  fi
  printf '%s' "$DEBUG_WIREGUARD_CONFIG"
}

start_main_activity() {
  "$ADB" -s "$serial" shell am start -n "$MAIN_ACTIVITY" "$@" >/dev/null
}

seed_debug_config() {
  if [[ -n "$DEBUG_INVITE" ]]; then
    start_main_activity --es "$DEBUG_INVITE_EXTRA" "$DEBUG_INVITE"
    sleep "$DEBUG_SEED_WAIT_SECS"
  fi

  if [[ -n "$DEBUG_EXIT_NODE" ]]; then
    start_main_activity \
      --es "$DEBUG_ACTION_EXTRA" set_fips_exit \
      --es "$DEBUG_EXIT_NODE_EXTRA" "$DEBUG_EXIT_NODE"
    sleep "$DEBUG_SEED_WAIT_SECS"
  fi

  if [[ -n "$DEBUG_WIREGUARD_CONFIG" || -n "$DEBUG_WIREGUARD_CONFIG_FILE" ]]; then
    local encoded
    encoded="$(wireguard_config | base64_no_wrap)"
    start_main_activity \
      --es "$DEBUG_ACTION_EXTRA" set_wireguard_exit \
      --es "$DEBUG_WIREGUARD_CONFIG_BASE64_EXTRA" "$encoded"
    sleep "$DEBUG_SEED_WAIT_SECS"
  fi
}

dump_vpn_diagnostics() {
  echo "Android VPN cycle did not reach the expected service/network state." >&2
  echo "If this is a first run, approve the Android VPN permission prompt and retry." >&2
  echo "If this device has no usable config, set NVPN_ANDROID_DEBUG_INVITE or NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_FILE." >&2
  echo >&2
  echo "---- dumpsys activity services $PACKAGE_NAME ----" >&2
  "$ADB" -s "$serial" shell dumpsys activity services "$PACKAGE_NAME" >&2 || true
  echo >&2
  echo "---- dumpsys connectivity VPN agents ----" >&2
  "$ADB" -s "$serial" shell dumpsys connectivity 2>/dev/null \
    | tr -d '\r' \
    | grep -E 'Active default network|NetworkAgentInfo\{|Transports: VPN|VpnNetworkProvider' >&2 || true
  echo >&2
  echo "---- recent NostrVpnService logcat ----" >&2
  "$ADB" -s "$serial" logcat -d -t 200 2>/dev/null \
    | tr -d '\r' \
    | grep -E 'NostrVpnService|org.nostrvpn.app|AndroidRuntime|ActivityTaskManager' >&2 || true
}

ADB="$(resolve_adb)"

if [[ "$build" -eq 1 ]]; then
  "$ROOT/tools/run-android" build
fi

if [[ ! -f "$APK_PATH" ]]; then
  echo "Debug APK not found at $APK_PATH; run just android-build first" >&2
  exit 1
fi

serial="$(select_serial "$ADB")"
if [[ -z "$serial" ]]; then
  echo "No online Android device or emulator found; set NVPN_ANDROID_SERIAL or start an emulator" >&2
  exit 1
fi

"$ADB" -s "$serial" wait-for-device
"$ADB" -s "$serial" install -r "$APK_PATH"

if [[ "$clear_state" -eq 1 ]]; then
  "$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
fi

start_main_activity
"$ADB" -s "$serial" shell pm path "$PACKAGE_NAME" >/dev/null

if [[ "$vpn_cycle" -eq 1 ]]; then
  seed_debug_config
  start_main_activity --es "$DEBUG_ACTION_EXTRA" disconnect
  if ! wait_until "$VPN_STOP_WAIT_SECS" vpn_inactive; then
    dump_vpn_diagnostics
    echo "Android smoke failed: VPN remained active after debug disconnect." >&2
    exit 1
  fi
  start_main_activity --es "$DEBUG_ACTION_EXTRA" connect
  if ! wait_until "$VPN_START_WAIT_SECS" vpn_active; then
    dump_vpn_diagnostics
    echo "Android smoke failed: VPN service and network did not become active after debug connect." >&2
    exit 1
  fi
fi

echo "Android smoke passed on adb serial: $serial"
