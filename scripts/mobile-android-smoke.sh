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
DEBUG_NETWORK_NAME_EXTRA="${NVPN_ANDROID_DEBUG_NETWORK_NAME_EXTRA:-org.nostrvpn.app.DEBUG_NETWORK_NAME}"
DEBUG_WIREGUARD_CONFIG_BASE64_EXTRA="${NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_BASE64_EXTRA:-org.nostrvpn.app.DEBUG_WIREGUARD_CONFIG_BASE64}"
APK_PATH="${NVPN_ANDROID_APK:-$ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
VPN_START_WAIT_SECS="${NVPN_ANDROID_VPN_START_WAIT_SECS:-15}"
VPN_STOP_WAIT_SECS="${NVPN_ANDROID_VPN_STOP_WAIT_SECS:-10}"
RUNTIME_STATE_WAIT_SECS="${NVPN_ANDROID_RUNTIME_STATE_WAIT_SECS:-12}"
RUNTIME_STATE_MAX_AGE_SECS="${NVPN_ANDROID_RUNTIME_STATE_MAX_AGE_SECS:-60}"
RUNTIME_STATE_RESULT_DIR="${NVPN_ANDROID_RESULT_DIR:-$ROOT/artifacts/mobile-android}"
RUNTIME_STATE_RESULT_NAME="${NVPN_ANDROID_RUNTIME_STATE_RESULT_NAME:-mobile-android-runtime-state-$$.json}"
TUN_PACKET_PROBE="${NVPN_ANDROID_TUN_PACKET_PROBE:-1}"
TUN_PACKET_PROBE_TARGET="${NVPN_ANDROID_TUN_PACKET_PROBE_TARGET:-10.44.255.254}"
TUN_PACKET_PROBE_WAIT_SECS="${NVPN_ANDROID_TUN_PACKET_PROBE_WAIT_SECS:-6}"
TUN_PACKET_PROBE_TIMEOUT_SECS="${NVPN_ANDROID_TUN_PACKET_PROBE_TIMEOUT_SECS:-1}"
DEBUG_SEED_WAIT_SECS="${NVPN_ANDROID_DEBUG_SEED_WAIT_SECS:-2}"
DEBUG_INVITE="${NVPN_ANDROID_DEBUG_INVITE:-}"
DEBUG_EXIT_NODE="${NVPN_ANDROID_DEBUG_EXIT_NODE:-}"
DEBUG_WIREGUARD_CONFIG="${NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG:-}"
DEBUG_WIREGUARD_CONFIG_FILE="${NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_FILE:-}"
DEBUG_NETWORK_NAME="${NVPN_ANDROID_DEBUG_NETWORK_NAME:-Android smoke}"

build=1
clear_state=0
create_network="${NVPN_ANDROID_DEBUG_CREATE_NETWORK:-0}"
accept_vpn_dialog="${NVPN_ANDROID_ACCEPT_VPN_DIALOG:-0}"
vpn_cycle=0
serial="${NVPN_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-android-smoke.sh [--no-build] [--clear] [--vpn-cycle] [--create-network] [--accept-vpn-dialog] [--serial SERIAL]

Builds and installs the debug APK, launches the app through adb, and optionally
cycles the debug VPN action. Values may live in .env.mobile.local, shell env,
or --serial. Keep device identifiers and signing details out of committed files.

First-run Android VPN permission prompts may need manual approval before
--vpn-cycle can run unattended.

For fresh installs, --vpn-cycle needs an active nvpn network. Seed one privately
with NVPN_ANDROID_DEBUG_INVITE. A WireGuard exit config can be layered on with
NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG / NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_FILE,
but it does not create an nvpn network by itself.

Use --create-network for a local OS VPN/TUN smoke when peer dataplane coverage is
not required. It creates a debug-only local network named by
NVPN_ANDROID_DEBUG_NETWORK_NAME, then cycles the VPN.

Use --accept-vpn-dialog only on trusted local test devices; it taps Android's
system VPN consent OK button if the prompt appears.

When --vpn-cycle reaches Android's active VPN service/network state, this script
also copies files/app-core/mobile-runtime-state.json from the debug app sandbox
and requires fresh Rust runtime state with native TUN counter fields.

By default --vpn-cycle also sends one shell ping toward a non-local 10.44/16
address and requires tunPacketsRead to increase. Disable with
NVPN_ANDROID_TUN_PACKET_PROBE=0 if a device image lacks ping.
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
    --create-network)
      create_network=1
      ;;
    --accept-vpn-dialog)
      accept_vpn_dialog=1
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
  local services
  services="$("$ADB" -s "$serial" shell dumpsys activity services "$PACKAGE_NAME" 2>/dev/null | tr -d '\r')" || return 1
  grep -q 'NostrVpnService' <<<"$services"
}

vpn_network_active() {
  local connectivity
  connectivity="$("$ADB" -s "$serial" shell dumpsys connectivity 2>/dev/null | tr -d '\r')" || return 1
  grep -Eq 'NetworkAgentInfo\{.*ni\{VPN CONNECTED' <<<"$connectivity"
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

truthy() {
  [[ "$1" == "1" || "$1" == "true" || "$1" == "yes" ]]
}

android_runtime_state_path() {
  printf '%s/%s\n' "$RUNTIME_STATE_RESULT_DIR" "$RUNTIME_STATE_RESULT_NAME"
}

copy_android_runtime_state() {
  local result_path
  result_path="$(android_runtime_state_path)"
  mkdir -p "$RUNTIME_STATE_RESULT_DIR"
  rm -f "$result_path.tmp"
  if "$ADB" -s "$serial" exec-out \
    run-as "$PACKAGE_NAME" cat files/app-core/mobile-runtime-state.json \
    >"$result_path.tmp" 2>/dev/null && [[ -s "$result_path.tmp" ]]
  then
    mv "$result_path.tmp" "$result_path"
    return 0
  fi
  rm -f "$result_path.tmp"
  return 1
}

validate_android_runtime_state() {
  local result_path
  result_path="$(android_runtime_state_path)"
  python3 - "$result_path" "$RUNTIME_STATE_MAX_AGE_SECS" <<'PY'
import json
import sys
import time

path = sys.argv[1]
max_age = int(sys.argv[2])
with open(path, encoding="utf-8") as fh:
    state = json.load(fh)

errors = []
if state.get("vpnEnabled") is not True:
    errors.append(f"vpnEnabled={state.get('vpnEnabled')!r}")
if state.get("vpnActive") is not True:
    errors.append(f"vpnActive={state.get('vpnActive')!r}")

updated_at = state.get("updatedAt")
now = int(time.time())
if not isinstance(updated_at, int) or updated_at <= 0:
    errors.append(f"updatedAt={updated_at!r}")
elif updated_at - now > 120:
    errors.append(f"updatedAt future skew={updated_at - now}s")
elif now - updated_at > max_age:
    errors.append(f"updatedAt age={now - updated_at}s")

for key in (
    "tunPacketsRead",
    "tunBytesRead",
    "tunPacketsWritten",
    "tunBytesWritten",
    "tunPacketsDropped",
):
    value = state.get(key)
    if not isinstance(value, int) or value < 0:
        errors.append(f"{key}={value!r}")

if errors:
    print("Android runtime state invalid: " + ", ".join(errors), file=sys.stderr)
    sys.exit(1)
PY
}

android_runtime_state_number() {
  local key="$1"
  local result_path
  result_path="$(android_runtime_state_path)"
  python3 - "$result_path" "$key" <<'PY'
import json
import sys

path, key = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as fh:
    value = json.load(fh).get(key)
if not isinstance(value, int):
    sys.exit(1)
print(value)
PY
}

wait_for_android_runtime_state() {
  local start now last_error
  start="$(date +%s)"
  last_error=""
  while true; do
    if copy_android_runtime_state; then
      if last_error="$(validate_android_runtime_state 2>&1)"; then
        echo "Android runtime state passed: $(android_runtime_state_path)"
        return 0
      fi
    else
      last_error="failed to copy files/app-core/mobile-runtime-state.json from debug app sandbox"
    fi
    now="$(date +%s)"
    if (( now - start >= RUNTIME_STATE_WAIT_SECS )); then
      echo "$last_error" >&2
      return 1
    fi
    sleep 1
  done
}

wait_for_tun_packets_read_after() {
  local baseline="$1"
  local start now current last_error
  start="$(date +%s)"
  last_error=""
  while true; do
    if copy_android_runtime_state; then
      if last_error="$(validate_android_runtime_state 2>&1)"; then
        current="$(android_runtime_state_number tunPacketsRead 2>/dev/null || true)"
        if [[ "$current" =~ ^[0-9]+$ ]] && (( current > baseline )); then
          echo "Android TUN packet probe passed: tunPacketsRead $baseline->$current target=$TUN_PACKET_PROBE_TARGET"
          return 0
        fi
        last_error="tunPacketsRead did not increase after probe (baseline=$baseline current=${current:-missing})"
      fi
    else
      last_error="failed to copy files/app-core/mobile-runtime-state.json from debug app sandbox"
    fi
    now="$(date +%s)"
    if (( now - start >= TUN_PACKET_PROBE_WAIT_SECS )); then
      echo "$last_error" >&2
      return 1
    fi
    sleep 1
  done
}

run_android_tun_packet_probe() {
  truthy "$TUN_PACKET_PROBE" || return 0
  local baseline
  baseline="$(android_runtime_state_number tunPacketsRead 2>/dev/null || printf '0')"
  [[ "$baseline" =~ ^[0-9]+$ ]] || baseline=0
  "$ADB" -s "$serial" shell ping -c 1 -W "$TUN_PACKET_PROBE_TIMEOUT_SECS" "$TUN_PACKET_PROBE_TARGET" >/dev/null 2>&1 || true
  wait_for_tun_packets_read_after "$baseline"
}

android_sdk() {
  "$ADB" -s "$serial" shell getprop ro.build.version.sdk | tr -d '\r'
}

grant_permission_if_declared() {
  local permission="$1"
  "$ADB" -s "$serial" shell pm grant "$PACKAGE_NAME" "$permission" >/dev/null 2>&1 || true
}

grant_debug_runtime_permissions() {
  local sdk
  sdk="$(android_sdk)"
  if [[ "$sdk" =~ ^[0-9]+$ && "$sdk" -ge 36 ]]; then
    grant_permission_if_declared android.permission.NEARBY_WIFI_DEVICES
  fi
  if [[ "$sdk" =~ ^[0-9]+$ && "$sdk" -ge 37 ]]; then
    grant_permission_if_declared android.permission.ACCESS_LOCAL_NETWORK
  fi
}

tap_ui_resource() {
  local resource="$1"
  local package="${2:-}"
  local remote="/sdcard/nvpn-window.xml"
  local xml
  local point
  xml="$(mktemp)"
  if ! "$ADB" -s "$serial" shell uiautomator dump "$remote" >/dev/null 2>&1; then
    rm -f "$xml"
    return 1
  fi
  if ! "$ADB" -s "$serial" pull "$remote" "$xml" >/dev/null 2>&1; then
    rm -f "$xml"
    return 1
  fi
  point="$(python3 - "$xml" "$resource" "$package" <<'PY'
import re
import sys

xml_path, resource, package = sys.argv[1], sys.argv[2], sys.argv[3]
xml = open(xml_path, encoding="utf-8").read()
for node in re.findall(r"<node [^>]+>", xml):
    rid = re.search(r'resource-id="([^"]*)"', node)
    pkg = re.search(r'package="([^"]*)"', node)
    bounds = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', node)
    enabled = re.search(r'enabled="([^"]*)"', node)
    if package and (not pkg or pkg.group(1) != package):
        continue
    if rid and rid.group(1) == resource and bounds and (not enabled or enabled.group(1) == "true"):
        left, top, right, bottom = map(int, bounds.groups())
        print((left + right) // 2, (top + bottom) // 2)
        sys.exit(0)
sys.exit(1)
PY
  )" || {
    rm -f "$xml"
    return 1
  }
  rm -f "$xml"
  "$ADB" -s "$serial" shell input tap $point
}

maybe_accept_vpn_dialog() {
  [[ "$accept_vpn_dialog" == "1" || "$accept_vpn_dialog" == "true" ]] || return 0
  local start now tapped
  start="$(date +%s)"
  tapped=0
  while true; do
    if vpn_active; then
      return 0
    fi
    if tap_ui_resource "android:id/button1" "com.android.vpndialogs"; then
      tapped=1
      sleep 1
      continue
    fi
    if [[ "$tapped" -eq 1 ]]; then
      return 0
    fi
    now="$(date +%s)"
    if (( now - start >= 8 )); then
      return 0
    fi
    sleep 1
  done
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
  if [[ "$create_network" == "1" || "$create_network" == "true" ]]; then
    start_main_activity \
      --es "$DEBUG_ACTION_EXTRA" add_network \
      --es "$DEBUG_NETWORK_NAME_EXTRA" "$DEBUG_NETWORK_NAME"
    sleep "$DEBUG_SEED_WAIT_SECS"
  fi

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
  echo "If this device has no active nvpn network, set NVPN_ANDROID_DEBUG_INVITE and approve any app/VPN prompts." >&2
  echo "NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_FILE only configures a WG exit; it does not create the required nvpn network." >&2
  echo >&2
  echo "---- dumpsys activity services $PACKAGE_NAME ----" >&2
  "$ADB" -s "$serial" shell dumpsys activity services "$PACKAGE_NAME" >&2 || true
  echo >&2
  echo "---- dumpsys connectivity VPN agents ----" >&2
  "$ADB" -s "$serial" shell dumpsys connectivity 2>/dev/null \
    | tr -d '\r' \
    | grep -E 'NetworkAgentInfo\{.*ni\{VPN|VpnNetworkProvider' >&2 || true
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

if [[ "$vpn_cycle" -eq 1 ]]; then
  grant_debug_runtime_permissions
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
  maybe_accept_vpn_dialog
  if ! wait_until "$VPN_START_WAIT_SECS" vpn_active; then
    dump_vpn_diagnostics
    echo "Android smoke failed: VPN service and network did not become active after debug connect." >&2
    exit 1
  fi
  if ! wait_for_android_runtime_state; then
    dump_vpn_diagnostics
    echo "Android smoke failed: Rust mobile runtime state did not become fresh after debug connect." >&2
    exit 1
  fi
  if ! run_android_tun_packet_probe; then
    dump_vpn_diagnostics
    echo "Android smoke failed: native TUN read counter did not advance after debug packet probe." >&2
    exit 1
  fi
fi

echo "Android smoke passed on adb serial: $serial"
