#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/scripts/release_common.sh"
# shellcheck disable=SC1091
source "$ROOT/scripts/mobile_env.sh"
load_release_env "$ROOT"
load_mobile_env "$ROOT"

ANDROID_PACKAGE="${NVPN_ANDROID_JOIN_E2E_PACKAGE:-fi.siriusbusiness.nvpn.joine2e}"
ANDROID_ACTIVITY="$ANDROID_PACKAGE/org.nostrvpn.app.MainActivity"
ANDROID_ACTION_EXTRA="fi.siriusbusiness.nvpn.DEBUG_ACTION"
ANDROID_JOIN_EXTRA="fi.siriusbusiness.nvpn.DEBUG_JOIN_REQUEST_BASE64"
ANDROID_ADMIN_EXTRA="fi.siriusbusiness.nvpn.DEBUG_ADMIN_DEVICE_ID_BASE64"
ANDROID_NETWORK_EXTRA="fi.siriusbusiness.nvpn.DEBUG_MESH_NETWORK_ID_BASE64"
ANDROID_PARTICIPANT_EXTRA="fi.siriusbusiness.nvpn.DEBUG_PARTICIPANT_DEVICE_ID_BASE64"
IOS_BUNDLE_ID="${NVPN_IOS_JOIN_E2E_BUNDLE_ID:-${NVPN_DEFAULT_IOS_BUNDLE_ID:-fi.siriusbusiness.nvpn}}"
IOS_APP_GROUP_IDENTIFIER="${NVPN_IOS_APP_GROUP_IDENTIFIER:-group.$IOS_BUNDLE_ID.shared}"
export NVPN_IOS_APP_GROUP_IDENTIFIER="$IOS_APP_GROUP_IDENTIFIER"
WAIT_SECS="${NVPN_MOBILE_JOIN_E2E_WAIT_SECS:-15}"
RESULT_DIR="${NVPN_MOBILE_JOIN_E2E_RESULT_DIR:-$ROOT/artifacts/mobile-join-e2e}"
PRIVATE_DIR="$RESULT_DIR/.private-$$"
SUMMARY="$RESULT_DIR/mobile-ios-android-join-summary-$$.json"
BUILD="${NVPN_MOBILE_JOIN_E2E_BUILD:-1}"
INSTALL_IOS="${NVPN_MOBILE_JOIN_E2E_INSTALL_IOS:-1}"
IOS_ORIGINAL_MESH=""
IOS_TEMP_MESH=""
ANDROID_ADMIN_MESH=""
ANDROID_DEVICE_ID=""
QR_IOS_ADMIN_TO_ANDROID_MS=""
QR_ANDROID_ADMIN_TO_IOS_MS=""
MANUAL_ANDROID_ADMIN_TO_IOS_MS=""
MANUAL_IOS_ADMIN_TO_ANDROID_MS=""

mkdir -p "$PRIVATE_DIR"
chmod 700 "$PRIVATE_DIR"

devices_may_need_cleanup=0

capture_failure_diagnostics() {
  local failure_dir="$RESULT_DIR/.failure-$(date -u +%Y%m%dT%H%M%SZ)-$$"
  local ios_runtime_name="mobile-ios-join-runtime-$$.json"
  mkdir -p "$failure_dir"
  chmod 700 "$failure_dir"
  launch_ios --nvpn-debug-runtime-result "$ios_runtime_name" >/dev/null 2>&1 || true
  sleep 2
  copy_ios_file config.toml "$failure_dir/ios-config.toml" >/dev/null 2>&1 || true
  if ! copy_ios_file "$ios_runtime_name" "$failure_dir/ios-runtime-state.json" \
    >/dev/null 2>&1
  then
    copy_ios_file mobile-runtime-state.json "$failure_dir/ios-runtime-state.json" \
      >/dev/null 2>&1 || true
  fi
  copy_ios_file app-debug.log "$failure_dir/ios-app-debug.log" >/dev/null 2>&1 || true
  copy_ios_file nvpn-pkt-debug.log "$failure_dir/ios-packet-tunnel-debug.log" \
    >/dev/null 2>&1 || true
  copy_android_file config.toml "$failure_dir/android-config.toml" 2>/dev/null || true
  copy_android_file mobile-runtime-state.json "$failure_dir/android-runtime-state.json" \
    2>/dev/null || true
  "${ADB[@]}" exec-out run-as "$ANDROID_PACKAGE" sh -c \
    'find files/app-core -maxdepth 2 -type f -print' \
    >"$failure_dir/android-app-core-files.txt" 2>/dev/null || true
  "${ADB[@]}" exec-out run-as "$ANDROID_PACKAGE" sh -c \
    'for file in files/app-core/config.toml.join-roster-outbox/*.json; do test -f "$file" && cat "$file" && echo; done' \
    >"$failure_dir/android-join-outbox.jsonl" 2>/dev/null || true
  "${ADB[@]}" logcat -d -t 1200 >"$failure_dir/android-logcat.txt" 2>/dev/null || true
  for source in android-request.json ios-request.json android-admin-config.toml; do
    if [[ -f "$PRIVATE_DIR/$source" ]]; then
      cp -p "$PRIVATE_DIR/$source" "$failure_dir/$source"
    fi
  done
  printf 'Private failure diagnostics retained at %s\n' "$failure_dir" >&2
}

cleanup_devices_and_private() {
  local status="$?"
  local cleanup_failed=0
  trap - EXIT
  if [[ "$status" -ne 0 && "$devices_may_need_cleanup" -eq 1 ]]; then
    capture_failure_diagnostics
  fi
  if [[ "$devices_may_need_cleanup" -eq 1 ]]; then
    if ! android_action disconnect >/dev/null 2>&1; then
      printf 'physical iOS/Android join e2e cleanup: Android disconnect launch failed\n' >&2
      cleanup_failed=1
    else
      local ignored android_service_active=1
      for ignored in $(seq 1 20); do
        if ! "${ADB[@]}" shell dumpsys activity services "$ANDROID_PACKAGE" 2>/dev/null \
          | grep -Fq 'NostrVpnService'
        then
          android_service_active=0
          break
        fi
        sleep 0.5
      done
      if [[ "$android_service_active" -eq 1 ]]; then
        printf 'physical iOS/Android join e2e cleanup: Android VPN service stayed active\n' >&2
        cleanup_failed=1
      fi
    fi
    if [[ -n "$IOS_TEMP_MESH" ]] && select_ios_mesh "$IOS_TEMP_MESH"; then
      launch_ios --nvpn-debug-remove-active-network >/dev/null 2>&1 || cleanup_failed=1
      sleep 1
    fi
    if [[ -n "$IOS_ORIGINAL_MESH" ]] && ! select_ios_mesh "$IOS_ORIGINAL_MESH"; then
      printf 'physical iOS/Android join e2e cleanup: original iOS network was not restored\n' >&2
      cleanup_failed=1
    fi
    if [[ -n "$ANDROID_DEVICE_ID" && -n "$IOS_ORIGINAL_MESH" ]]; then
      local cleanup_device_b64
      cleanup_device_b64="$(base64_text "$ANDROID_DEVICE_ID")"
      launch_ios --nvpn-debug-remove-participant-base64 "$cleanup_device_b64" \
        >/dev/null 2>&1 || true
      sleep 1
    fi
    if ! env NVPN_IDLE_CPU_GATE=0 NVPN_IOS_BUNDLE_ID="$IOS_BUNDLE_ID" \
      "$ROOT/scripts/mobile-ios-smoke.sh" device --device "$IOS_DEVICE" --disconnect
    then
      printf 'physical iOS/Android join e2e cleanup: iOS disconnect verification failed\n' >&2
      cleanup_failed=1
    fi
    "${ADB[@]}" shell pm clear "$ANDROID_PACKAGE" >/dev/null 2>&1 || cleanup_failed=1
  fi
  rm -rf "$PRIVATE_DIR"
  if [[ "$status" -eq 0 && "$cleanup_failed" -ne 0 ]]; then
    status=1
  fi
  exit "$status"
}
trap cleanup_devices_and_private EXIT

fail() {
  printf 'physical iOS/Android join e2e failed: %s\n' "$*" >&2
  exit 1
}

select_android_device() {
  if [[ -n "${NVPN_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}" ]]; then
    printf '%s\n' "${NVPN_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"
    return
  fi
  adb devices | awk '
    NR > 1 && $2 == "device" && $1 !~ /^emulator-/ { devices[++count] = $1 }
    END { if (count == 1) print devices[1]; else exit 1 }
  '
}

select_ios_device() {
  if [[ -n "${NVPN_IOS_DEVICE:-${NVPN_IOS_DEVICE_ID:-}}" ]]; then
    printf '%s\n' "${NVPN_IOS_DEVICE:-${NVPN_IOS_DEVICE_ID:-}}"
    return
  fi
  xcrun xctrace list devices 2>/dev/null | awk '
    /^== Devices ==/ { in_devices = 1; next }
    /^== Devices Offline ==/ || /^== Simulators ==/ { in_devices = 0 }
    in_devices && /iPhone|iPad/ {
      value = $0
      sub(/^.*\(/, "", value)
      sub(/\)[[:space:]]*$/, "", value)
      devices[++count] = value
    }
    END { if (count == 1) print devices[1]; else exit 1 }
  '
}

ANDROID_SERIAL_SELECTED="$(select_android_device)" || fail "one physical Android device is required"
IOS_DEVICE="$(select_ios_device)" || fail "one physical iPhone/iPad is required"
ADB=(adb -s "$ANDROID_SERIAL_SELECTED")

launch_ios() {
  ios_device_launch "$IOS_DEVICE" "$IOS_BUNDLE_ID" "$@" >/dev/null
}

copy_ios_file() {
  local source="$1"
  local destination="$2"
  local debug_source="Library/Application Support/Nostr VPN Debug Results/$source"
  rm -f "$destination"
  launch_ios \
    --nvpn-debug-export-support-file "$source" \
    --nvpn-debug-export-result "$source" >/dev/null 2>&1 || return 1
  sleep 0.5
  xcrun devicectl device copy from \
    --device "$IOS_DEVICE" \
    --domain-type appDataContainer \
    --domain-identifier "$IOS_BUNDLE_ID" \
    --source "$debug_source" \
    --destination "$destination" \
    --quiet
}

copy_ios_debug_result() {
  local source="$1"
  local destination="$2"
  rm -f "$destination"
  xcrun devicectl device copy from \
    --device "$IOS_DEVICE" \
    --domain-type appDataContainer \
    --domain-identifier "$IOS_BUNDLE_ID" \
    --source "Library/Application Support/Nostr VPN Debug Results/$source" \
    --destination "$destination" \
    --quiet 2>/dev/null
}

copy_android_file() {
  local source="$1"
  local destination="$2"
  "${ADB[@]}" exec-out run-as "$ANDROID_PACKAGE" cat "files/app-core/$source" >"$destination"
}

json_value() {
  python3 - "$1" "$2" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    value = json.load(fh).get(sys.argv[2], "")
if not isinstance(value, str) or not value:
    raise SystemExit(1)
print(value)
PY
}

base64_text() {
  python3 - "$1" <<'PY'
import base64, sys
print(base64.b64encode(sys.argv[1].encode()).decode())
PY
}

current_milliseconds() {
  python3 - <<'PY'
import time
print(time.time_ns() // 1_000_000)
PY
}

elapsed_milliseconds() {
  local started="$1" finished
  finished="$(current_milliseconds)"
  if ((finished < started)); then
    printf '0\n'
  else
    printf '%s\n' "$((finished - started))"
  fi
}

active_mesh_id() {
  python3 - "$1" <<'PY'
import re, sys

text = open(sys.argv[1], encoding="utf-8").read()
for network in re.split(r"(?m)^\s*\[\[networks\]\]\s*(?:#.*)?$", text)[1:]:
    enabled = re.search(r"(?m)^\s*enabled\s*=\s*true\s*(?:#.*)?$", network)
    network_id = re.search(r'(?m)^\s*network_id\s*=\s*"([^"]+)"', network)
    if enabled and network_id:
        print(network_id.group(1))
        raise SystemExit(0)
raise SystemExit(1)
PY
}

admin_mesh_id() {
  python3 - "$1" <<'PY'
import re, sys

text = open(sys.argv[1], encoding="utf-8").read()
nostr = re.search(r"(?ms)^\[nostr\]\s*(.*?)(?=^\[|\Z)", text)
own = re.search(r'(?m)^\s*public_key\s*=\s*"([^"]+)"', nostr.group(1) if nostr else "")
if not own:
    raise SystemExit(1)
own = own.group(1)
candidates = []
for order, network in enumerate(re.split(r"(?m)^\s*\[\[networks\]\]\s*(?:#.*)?$", text)[1:]):
    network_id = re.search(r'(?m)^\s*network_id\s*=\s*"([^"]+)"', network)
    signer = re.search(r'(?m)^\s*shared_roster_signed_by\s*=\s*"([^"]+)"', network)
    admins = re.search(r"(?m)^\s*admins\s*=\s*\[(.*?)\]", network)
    admin_values = re.findall(r'"([^"]+)"', admins.group(1) if admins else "")
    if not network_id or (signer and signer.group(1) != own and own not in admin_values):
        continue
    if (signer and signer.group(1) == own) or own in admin_values:
        candidates.append((0 if signer and signer.group(1) == own else 1, order, network_id.group(1)))
if not candidates:
    raise SystemExit(1)
print(min(candidates)[2])
PY
}

select_ios_mesh() {
  local expected="$1"
  local encoded deadline result_name result_path
  encoded="$(base64_text "$expected")"
  result_name="mobile-ios-network-selection-$$-$RANDOM.json"
  result_path="$PRIVATE_DIR/$result_name"
  launch_ios \
    --nvpn-debug-select-network-base64 "$encoded" \
    --nvpn-debug-select-network-result "$result_name" \
    >/dev/null 2>&1 || return 1
  deadline=$((SECONDS + 15))
  while ((SECONDS < deadline)); do
    if copy_ios_debug_result "$result_name" "$result_path" \
      && python3 - "$result_path" "$expected" <<'PY'
import json, sys

with open(sys.argv[1], encoding="utf-8") as fh:
    result = json.load(fh)
if result.get("ok") is not True:
    raise SystemExit(1)
if result.get("activeNetworkId") != sys.argv[2]:
    raise SystemExit(1)
if result.get("enabledNetworkCount") != 1:
    raise SystemExit(1)
PY
    then
      return 0
    fi
    sleep 1
  done
  return 1
}

connect_ios_joiner() {
  local result_name="mobile-ios-join-connect-$$-$RANDOM.json"
  local result_path="$PRIVATE_DIR/$result_name"
  local deadline=$((SECONDS + 30)) connect_error
  launch_ios --nvpn-debug-connect-result "$result_name" >/dev/null 2>&1 || return 1
  while ((SECONDS < deadline)); do
    if copy_ios_debug_result "$result_name" "$result_path" \
      && python3 - "$result_path" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    result = json.load(fh)
raise SystemExit(0 if result.get("ok") is True and result.get("packetTunnelStatusRawValue") == 3 else 1)
PY
    then
      return 0
    fi
    if [[ -f "$result_path" ]]; then
      connect_error="$(python3 - "$result_path" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    result = json.load(fh)
if result.get("phase") != "finished":
    raise SystemExit(1)
print(result.get("startError") or "packet tunnel did not reach connected state")
PY
      )" || connect_error=""
      if [[ -n "$connect_error" ]]; then
        printf 'iOS VPN connect failed: %s\n' "$connect_error" >&2
        return 1
      fi
    fi
    sleep 1
  done
  return 1
}

config_has_joined_mesh() {
  python3 - "$1" "$2" <<'PY'
import re, sys

text = open(sys.argv[1], encoding="utf-8").read()
expected = sys.argv[2]
for network in re.split(r"(?m)^\s*\[\[networks\]\]\s*(?:#.*)?$", text)[1:]:
    enabled = re.search(r"(?m)^\s*enabled\s*=\s*true\s*(?:#.*)?$", network)
    network_id = re.search(r'(?m)^\s*network_id\s*=\s*"([^"]+)"', network)
    updated = re.search(r"(?m)^\s*shared_roster_updated_at\s*=\s*([1-9][0-9]*)\s*(?:#.*)?$", network)
    signer = re.search(r'(?m)^\s*shared_roster_signed_by\s*=\s*"([^"]+)"', network)
    if enabled and network_id and network_id.group(1) == expected:
        if updated and signer:
            raise SystemExit(0)
raise SystemExit(1)
PY
}

wait_for_android_join() {
  local expected="$1"
  local deadline=$((SECONDS + WAIT_SECS))
  while ((SECONDS < deadline)); do
    if copy_android_file config.toml "$PRIVATE_DIR/android-config.toml" 2>/dev/null \
      && config_has_joined_mesh "$PRIVATE_DIR/android-config.toml" "$expected"
    then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_ios_join() {
  local expected="$1"
  local expected_b64 result_name result_path deadline wait_error
  expected_b64="$(base64_text "$expected")"
  result_name="mobile-ios-joined-network-$$-$RANDOM.json"
  result_path="$PRIVATE_DIR/$result_name"
  deadline=$((SECONDS + WAIT_SECS + 2))
  launch_ios \
    --nvpn-debug-wait-for-joined-network-base64 "$expected_b64" \
    --nvpn-debug-wait-for-joined-network-result "$result_name" \
    --nvpn-debug-wait-for-joined-network-timeout-seconds "$WAIT_SECS" \
    >/dev/null 2>&1 || return 1
  while ((SECONDS < deadline)); do
    if copy_ios_debug_result "$result_name" "$result_path" \
      && python3 - "$result_path" "$expected" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    result = json.load(fh)
raise SystemExit(0 if result.get("ok") is True and result.get("activeNetworkId") == sys.argv[2] else 1)
PY
    then
      return 0
    fi
    if [[ -f "$result_path" ]]; then
      wait_error="$(python3 - "$result_path" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    result = json.load(fh)
if result.get("phase") != "finished":
    raise SystemExit(1)
print(result.get("error") or "joined-network receipt failed")
PY
      )" || wait_error=""
      if [[ -n "$wait_error" ]]; then
        printf 'iOS joined-network wait failed: %s\n' "$wait_error" >&2
        return 1
      fi
    fi
    sleep 0.25
  done
  return 1
}

tap_android_resource_if_present() {
  local resource="$1"
  local dump="$PRIVATE_DIR/android-ui.xml"
  "${ADB[@]}" shell uiautomator dump /sdcard/nvpn-join-e2e.xml >/dev/null 2>&1 || return 1
  "${ADB[@]}" exec-out cat /sdcard/nvpn-join-e2e.xml >"$dump" || return 1
  local coordinates
  coordinates="$(python3 - "$dump" "$resource" <<'PY'
import re, sys
raw = open(sys.argv[1], encoding="utf-8").read()
resource = re.escape(sys.argv[2])
match = re.search(r'resource-id="' + resource + r'"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', raw)
if not match:
    raise SystemExit(1)
x1, y1, x2, y2 = map(int, match.groups())
print((x1 + x2) // 2, (y1 + y2) // 2)
PY
)" || return 1
  # shellcheck disable=SC2086
  "${ADB[@]}" shell input tap $coordinates
}

allow_android_vpn_prompts() {
  local ignored
  for ignored in 1 2 3 4; do
    sleep 1
    tap_android_resource_if_present \
      "com.android.permissioncontroller:id/permission_allow_button" >/dev/null 2>&1 || true
    tap_android_resource_if_present "android:id/button1" >/dev/null 2>&1 || true
  done
}

android_action() {
  local action="$1"
  shift
  "${ADB[@]}" shell am start -W -n "$ANDROID_ACTIVITY" \
    --es "$ANDROID_ACTION_EXTRA" "$action" \
    "$@" >/dev/null
}

if [[ "$BUILD" != "0" ]]; then
  NVPN_ANDROID_PACKAGE="$ANDROID_PACKAGE" "$ROOT/tools/run-android" build
fi
"${ADB[@]}" install -r "$ROOT/android/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
"${ADB[@]}" shell pm clear "$ANDROID_PACKAGE" >/dev/null

if [[ "$INSTALL_IOS" != "0" ]]; then
  NVPN_IOS_INSTALL=1 "$ROOT/scripts/mobile-ios-smoke.sh" device --device "$IOS_DEVICE" --install
fi
launch_ios || fail "the installed iOS development profile is not trusted or launchable"
devices_may_need_cleanup=1

copy_ios_file config.toml "$PRIVATE_DIR/ios-original-config.toml" \
  || fail "could not read the iOS app config"
IOS_ORIGINAL_MESH="$(admin_mesh_id "$PRIVATE_DIR/ios-original-config.toml")" \
  || fail "iOS must have an administered network for the QR-admin direction"
select_ios_mesh "$IOS_ORIGINAL_MESH" \
  || fail "could not select the original iOS admin network"
connect_ios_joiner \
  || fail "iPhone admin tunnel did not become connected before QR approval"

android_action export_join_request
sleep 2
copy_android_file debug-join-request.json "$PRIVATE_DIR/android-request.json" \
  || fail "Android join request export failed"
ANDROID_REQUEST="$(json_value "$PRIVATE_DIR/android-request.json" joinRequest)" \
  || fail "Android join request is empty"
ANDROID_DEVICE_ID="$(json_value "$PRIVATE_DIR/android-request.json" deviceId)" \
  || fail "Android Device ID is empty"

android_action connect
allow_android_vpn_prompts
ANDROID_REQUEST_B64="$(base64_text "$ANDROID_REQUEST")"
delivery_started_ms="$(current_milliseconds)"
launch_ios --nvpn-debug-import-join-request-base64 "$ANDROID_REQUEST_B64" \
  || fail "could not launch the iOS QR approval action"
wait_for_android_join "$IOS_ORIGINAL_MESH" \
  || fail "iPhone admin approval never reached the Pixel joiner"
QR_IOS_ADMIN_TO_ANDROID_MS="$(elapsed_milliseconds "$delivery_started_ms")"

# Remove the isolated Android identity from the user's original iOS network
# before exercising the reverse direction.
ANDROID_DEVICE_B64="$(base64_text "$ANDROID_DEVICE_ID")"
launch_ios --nvpn-debug-remove-participant-base64 "$ANDROID_DEVICE_B64" \
  || fail "could not clean the first-direction iOS participant"
sleep 3

android_action remove_active_network
sleep 2
android_action add_network --es fi.siriusbusiness.nvpn.DEBUG_NETWORK_NAME "Android join e2e"
sleep 3
copy_android_file config.toml "$PRIVATE_DIR/android-admin-config.toml" \
  || fail "could not read Android admin config"
ANDROID_ADMIN_MESH="$(active_mesh_id "$PRIVATE_DIR/android-admin-config.toml")" \
  || fail "Android admin network was not created"
IOS_TEMP_MESH="$ANDROID_ADMIN_MESH"

launch_ios --nvpn-debug-export-join-request ios-join-request.json
sleep 2
copy_ios_debug_result ios-join-request.json "$PRIVATE_DIR/ios-request.json" \
  || fail "iOS join request export failed"
IOS_REQUEST="$(json_value "$PRIVATE_DIR/ios-request.json" joinRequest)" \
  || fail "iOS join request is empty"
IOS_REQUEST_B64="$(base64_text "$IOS_REQUEST")"

connect_ios_joiner \
  || fail "iPhone joiner tunnel did not become connected before Android approval"
delivery_started_ms="$(current_milliseconds)"
android_action import_join_request --es "$ANDROID_JOIN_EXTRA" "$IOS_REQUEST_B64"
wait_for_ios_join "$ANDROID_ADMIN_MESH" \
  || fail "Pixel admin approval never reached the iPhone joiner"
QR_ANDROID_ADMIN_TO_IOS_MS="$(elapsed_milliseconds "$delivery_started_ms")"

# Reuse the temporary Android-admin network to prove the explicit manual flow
# with Android as admin and iOS as joiner. Remove the QR-added member first so
# the manual admin action is the only source of the signed roster entry.
launch_ios --nvpn-debug-remove-active-network \
  || fail "could not remove the QR-joined iOS network"
IOS_DEVICE_ID="$(json_value "$PRIVATE_DIR/ios-request.json" deviceId)" \
  || fail "iOS Device ID is empty"
IOS_DEVICE_B64="$(base64_text "$IOS_DEVICE_ID")"
android_action remove_participant --es "$ANDROID_PARTICIPANT_EXTRA" "$IOS_DEVICE_B64"
sleep 2
android_action add_participant --es "$ANDROID_PARTICIPANT_EXTRA" "$IOS_DEVICE_B64"
ANDROID_DEVICE_B64="$(base64_text "$ANDROID_DEVICE_ID")"
ANDROID_MESH_B64="$(base64_text "$ANDROID_ADMIN_MESH")"
delivery_started_ms="$(current_milliseconds)"
launch_ios \
  --nvpn-debug-manual-join-admin-base64 "$ANDROID_DEVICE_B64" \
  --nvpn-debug-manual-join-network-base64 "$ANDROID_MESH_B64" \
  || fail "could not launch the iOS manual joiner action"
wait_for_ios_join "$ANDROID_ADMIN_MESH" \
  || fail "Android manual admin addition never reached the iPhone joiner"
MANUAL_ANDROID_ADMIN_TO_IOS_MS="$(elapsed_milliseconds "$delivery_started_ms")"

# Then prove the opposite manual roles against the user's original iOS admin
# network, before restoring both devices to their pre-gate network state.
launch_ios --nvpn-debug-remove-active-network \
  || fail "could not remove the manually joined iOS network"
android_action remove_participant --es "$ANDROID_PARTICIPANT_EXTRA" "$IOS_DEVICE_B64"
android_action remove_active_network
select_ios_mesh "$IOS_ORIGINAL_MESH" \
  || fail "could not restore the iOS admin network for the reverse manual direction"
IOS_MESH_B64="$(base64_text "$IOS_ORIGINAL_MESH")"
launch_ios --nvpn-debug-add-participant-base64 "$ANDROID_DEVICE_B64" \
  || fail "could not launch the iOS manual admin action"
delivery_started_ms="$(current_milliseconds)"
android_action manual_join \
  --es "$ANDROID_ADMIN_EXTRA" "$IOS_DEVICE_B64" \
  --es "$ANDROID_NETWORK_EXTRA" "$IOS_MESH_B64"
wait_for_android_join "$IOS_ORIGINAL_MESH" \
  || fail "iPhone manual admin addition never reached the Android joiner"
MANUAL_IOS_ADMIN_TO_ANDROID_MS="$(elapsed_milliseconds "$delivery_started_ms")"
android_action remove_active_network
launch_ios --nvpn-debug-remove-participant-base64 "$ANDROID_DEVICE_B64" \
  || fail "could not clean the manual iOS participant"

mkdir -p "$RESULT_DIR"
python3 - "$SUMMARY" \
  "$WAIT_SECS" \
  "$QR_IOS_ADMIN_TO_ANDROID_MS" \
  "$QR_ANDROID_ADMIN_TO_IOS_MS" \
  "$MANUAL_ANDROID_ADMIN_TO_IOS_MS" \
  "$MANUAL_IOS_ADMIN_TO_ANDROID_MS" <<'PY'
import json, sys
with open(sys.argv[1], "w", encoding="utf-8") as fh:
    json.dump({
        "passed": True,
        "directions": [
            "ios-admin-to-android-joiner",
            "android-admin-to-ios-joiner",
        ],
        "manualDirections": [
            "ios-admin-to-android-joiner",
            "android-admin-to-ios-joiner",
        ],
        "deliveryCeilingMs": int(float(sys.argv[2]) * 1000),
        "deliveryElapsedMs": {
            "qrIosAdminToAndroidJoiner": int(sys.argv[3]),
            "qrAndroidAdminToIosJoiner": int(sys.argv[4]),
            "manualAndroidAdminToIosJoiner": int(sys.argv[5]),
            "manualIosAdminToAndroidJoiner": int(sys.argv[6]),
        },
        "transport": "physical-device-fips-tcp-signed-roster-receipt",
    }, fh, sort_keys=True, indent=2)
    fh.write("\n")
PY

printf 'Physical iOS/Android QR approval and manual join passed in both directions: %s\n' "$SUMMARY"
printf 'Delivery elapsed ms (QR iOS→Android, QR Android→iOS, manual Android→iOS, manual iOS→Android): %s, %s, %s, %s\n' \
  "$QR_IOS_ADMIN_TO_ANDROID_MS" \
  "$QR_ANDROID_ADMIN_TO_IOS_MS" \
  "$MANUAL_ANDROID_ADMIN_TO_IOS_MS" \
  "$MANUAL_IOS_ADMIN_TO_ANDROID_MS"
