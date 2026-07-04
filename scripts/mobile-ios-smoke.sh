#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/scripts/release_common.sh"
# shellcheck disable=SC1091
source "$ROOT/scripts/mobile_env.sh"
load_release_env "$ROOT"
load_mobile_env "$ROOT"
resolve_shared_build_metadata "$ROOT"
export NVPN_IOS_BUNDLE_ID="${NVPN_IOS_BUNDLE_ID:-${NVPN_DEFAULT_IOS_BUNDLE_ID:-fi.siriusbusiness.nvpn}}"
export NVPN_IOS_PACKET_TUNNEL_BUNDLE_ID="${NVPN_IOS_PACKET_TUNNEL_BUNDLE_ID:-$NVPN_IOS_BUNDLE_ID.PacketTunnel}"
export NVPN_IOS_APP_GROUP_IDENTIFIER="${NVPN_IOS_APP_GROUP_IDENTIFIER:-group.$NVPN_IOS_BUNDLE_ID}"
BUNDLE_ID="$NVPN_IOS_BUNDLE_ID"
PROJECT="$ROOT/ios/NostrVpnIos.xcodeproj"
SCHEME="${NVPN_IOS_SCHEME:-NostrVpnIos}"
DEVICE_CONFIGURATION="${NVPN_IOS_DEVICE_CONFIGURATION:-Debug}"
DEVICE_DERIVED_DATA="${NVPN_IOS_DEVICE_DERIVED_DATA:-$ROOT/ios/.build/DeviceDerivedData}"
DEVICE_DESTINATION="${NVPN_IOS_DEVICE_DESTINATION:-generic/platform=iOS}"
DEVICE_CODE_SIGN_IDENTITY="${NVPN_IOS_CODE_SIGN_IDENTITY:-Apple Development}"
INSTALL_DEVICE_APP="${NVPN_IOS_INSTALL:-0}"
CREATE_NETWORK="${NVPN_IOS_DEBUG_CREATE_NETWORK:-0}"
DEBUG_NETWORK_NAME="${NVPN_IOS_DEBUG_NETWORK_NAME:-iOS smoke}"
VPN_START_WAIT_SECS="${NVPN_IOS_VPN_START_WAIT_SECS:-12}"
VPN_RESULT_WAIT_SECS="${NVPN_IOS_VPN_RESULT_WAIT_SECS:-4}"
VPN_RESULT_NAME="${NVPN_IOS_VPN_RESULT_NAME:-mobile-ios-smoke-vpn-$$.json}"
VPN_RESULT_DIR="${NVPN_IOS_RESULT_DIR:-$ROOT/artifacts/mobile-ios}"
TUN_PACKET_PROBE_TARGET="${NVPN_IOS_TUN_PACKET_PROBE_TARGET:-10.44.255.254}"
TUN_PACKET_PROBE_PORT="${NVPN_IOS_TUN_PACKET_PROBE_PORT:-9}"
TUN_PACKET_PROBE_COUNT="${NVPN_IOS_TUN_PACKET_PROBE_COUNT:-4}"
TUN_PACKET_PROBE_WAIT_SECS="${NVPN_IOS_TUN_PACKET_PROBE_WAIT_SECS:-6}"
cleanup_after_vpn_cycle="${NVPN_IOS_CLEANUP_AFTER_VPN_CYCLE:-1}"
SCREENSHOT="$ROOT/artifacts/nostr-vpn-ios.png"

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-ios-smoke.sh [simulator|device] [--install] [--create-network] [--vpn-cycle] [--device DEVICE] [--leave-vpn-active]

simulator  Builds, installs, launches, and screenshots the simulator app.
device     Launches an already installed development build on a physical device.
--install  Builds and installs the current development-signed iphoneos app
           before launching device mode.
--device DEVICE
           Selects the physical device identifier for this run. Equivalent to
           NVPN_IOS_DEVICE=DEVICE.
--create-network
           Creates a local debug network before the device VPN cycle, for OS
           Packet Tunnel coverage without peer dataplane coverage.
--leave-vpn-active
           Preserve a passing VPN cycle for manual inspection. By default a
           passing --vpn-cycle asks the debug app to disconnect afterwards.

Physical-device mode requires NVPN_IOS_DEVICE or NVPN_IOS_DEVICE_ID. Values may
live in .env.mobile.local or shell env. Keep device identifiers and signing
details out of committed files.

Simulator mode is a launch smoke only; iOS Packet Tunnel dataplane checks need
a physical device, and first-run VPN/profile permission prompts may need a
manual approval before --vpn-cycle can run unattended.

Device install mode requires Xcode signing access for NVPN_IOS_BUNDLE_ID and
NVPN_IOS_PACKET_TUNNEL_BUNDLE_ID. Set NVPN_IOS_TEAM_ID in the shell or local
env file; set NVPN_IOS_ALLOW_PROVISIONING_UPDATES=0 to avoid automatic profile
updates.

The physical-device packet probe defaults to 4 UDP packets toward the debug
non-local tunnel probe target. Override with NVPN_IOS_TUN_PACKET_PROBE_TARGET,
NVPN_IOS_TUN_PACKET_PROBE_PORT, NVPN_IOS_TUN_PACKET_PROBE_COUNT, and
NVPN_IOS_TUN_PACKET_PROBE_WAIT_SECS.
EOF
}

mode="${1:-simulator}"
if [[ $# -gt 0 ]]; then
  shift
fi
vpn_cycle=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install)
      INSTALL_DEVICE_APP=1
      ;;
    --create-network)
      CREATE_NETWORK=1
      ;;
    --device)
      if [[ $# -lt 2 ]]; then
        echo "--device requires a value" >&2
        exit 2
      fi
      export NVPN_IOS_DEVICE="$2"
      shift
      ;;
    --leave-vpn-active)
      cleanup_after_vpn_cycle=0
      ;;
    --vpn-cycle)
      vpn_cycle=1
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

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "iOS smoke requires macOS with Xcode" >&2
  exit 1
fi

run_simulator() {
  "$ROOT/tools/run-ios" run
  if [[ ! -s "$SCREENSHOT" ]]; then
    echo "Expected simulator screenshot at $SCREENSHOT" >&2
    exit 1
  fi
  echo "iOS simulator smoke passed: $SCREENSHOT"
}

launch_device() {
  local device="$1"
  shift
  xcrun devicectl device process launch \
    --device "$device" \
    --terminate-existing \
    "$BUNDLE_ID" \
    "$@"
}

device_app_path() {
  find "$DEVICE_DERIVED_DATA/Build/Products/$DEVICE_CONFIGURATION-iphoneos" \
    -maxdepth 1 -name '*.app' -type d | sort | head -n 1
}

build_device_app() {
  local team="${NVPN_IOS_TEAM_ID:-}"
  if [[ -z "$team" ]]; then
    echo "Set NVPN_IOS_TEAM_ID to build/install a physical iOS device app." >&2
    exit 1
  fi

  "$ROOT/tools/run-ios" xcframework
  "$ROOT/tools/run-ios" project

  local cmd=(xcodebuild)
  if bool_is_true "${NVPN_IOS_XCODEBUILD_QUIET:-1}"; then
    cmd+=(-quiet)
  fi
  if bool_is_true "${NVPN_IOS_ALLOW_PROVISIONING_UPDATES:-1}"; then
    cmd+=(-allowProvisioningUpdates)
  fi
  cmd+=(
    -project "$PROJECT"
    -scheme "$SCHEME"
    -configuration "$DEVICE_CONFIGURATION"
    -derivedDataPath "$DEVICE_DERIVED_DATA"
    -destination "$DEVICE_DESTINATION"
    DEVELOPMENT_TEAM="$team"
    CODE_SIGN_IDENTITY="$DEVICE_CODE_SIGN_IDENTITY"
    build
  )
  "${cmd[@]}"
}

install_device_app() {
  local device="$1"
  local app_path
  build_device_app
  app_path="$(device_app_path)"
  if [[ -z "$app_path" ]]; then
    echo "Built iOS device app not found under $DEVICE_DERIVED_DATA" >&2
    exit 1
  fi
  xcrun devicectl device install app --device "$device" "$app_path"
}

copy_vpn_probe_result() {
  local device="$1"
  local result_path="$VPN_RESULT_DIR/$VPN_RESULT_NAME"
  mkdir -p "$VPN_RESULT_DIR"
  rm -f "$result_path"
  if ! xcrun devicectl device copy from \
    --device "$device" \
    --domain-type appDataContainer \
    --domain-identifier "$BUNDLE_ID" \
    --source "Library/Application Support/Nostr VPN/$VPN_RESULT_NAME" \
    --destination "$result_path" \
    --quiet
  then
    echo "Failed to copy iOS VPN probe result from app data container for $BUNDLE_ID" >&2
    echo "If this is a first run, approve the iOS VPN configuration prompt on the device and retry." >&2
    return 1
  fi
  if [[ ! -s "$result_path" ]]; then
    echo "iOS VPN probe result not found at $result_path" >&2
    return 1
  fi
  printf '%s\n' "$result_path"
}

copy_ios_debug_logs() {
  local device="$1"
  local stem="${VPN_RESULT_NAME%.json}"
  local copied=0
  mkdir -p "$VPN_RESULT_DIR"
  for name in app-debug.log nvpn-pkt-debug.log; do
    local destination="$VPN_RESULT_DIR/$stem-$name"
    rm -f "$destination"
    if xcrun devicectl device copy from \
      --device "$device" \
      --domain-type appDataContainer \
      --domain-identifier "$BUNDLE_ID" \
      --source "Library/Application Support/Nostr VPN/$name" \
      --destination "$destination" \
      --quiet
    then
      if [[ -s "$destination" ]]; then
        echo "Copied iOS debug log: $destination" >&2
        copied=1
      else
        rm -f "$destination"
      fi
    else
      rm -f "$destination"
    fi
  done
  if [[ "$copied" -eq 1 ]]; then
    return 0
  fi
  return 1
}

validate_vpn_probe_result() {
  local result_path="$1"
  python3 - "$result_path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as fh:
    result = json.load(fh)

runtime = None

def counter(value):
    if isinstance(value, int):
        return value
    if isinstance(value, str) and value.isdigit():
        return int(value)
    return None

def probe_summary():
    expected = counter(result.get("tunPacketProbeExpectedPackets"))
    observed = counter(result.get("tunPacketProbeObservedPackets"))
    missing = counter(result.get("tunPacketProbeMissingPackets"))
    loss_pct = "?"
    observed_pct = "?"
    if expected and expected > 0:
        if missing is not None:
            loss_pct = f"{(missing * 100.0 / expected):.1f}%"
        if observed is not None:
            observed_pct = f"{(observed * 100.0 / expected):.1f}%"
    parts = [
        f"read={result.get('tunPacketProbeBaselineRead')}->{result.get('tunPacketProbeFinalRead')}",
        (
            f"observed={result.get('tunPacketProbeObservedPackets')}/"
            f"{result.get('tunPacketProbeExpectedPackets')}"
        ),
        f"observedPct={observed_pct}",
        f"missing={result.get('tunPacketProbeMissingPackets', '?')}",
        f"lossPct={loss_pct}",
        f"bytes={result.get('tunPacketProbeObservedBytesRead')}",
        f"drops={result.get('tunPacketProbeDroppedDelta')}",
        f"firstMs={result.get('tunPacketProbeFirstObservedMs', '?')}",
        f"elapsedMs={result.get('tunPacketProbeElapsedMs')}",
        f"polls={result.get('tunPacketProbePolls', '?')}",
        f"target={result.get('tunPacketProbeTarget')}",
    ]
    if isinstance(runtime, dict):
        parts.extend([
            f"runtimeRead={runtime.get('tunPacketsRead')}",
            f"runtimeWritten={runtime.get('tunPacketsWritten')}",
            f"runtimeDropped={runtime.get('tunPacketsDropped')}",
        ])
    return "iOS TUN packet probe counters: " + " ".join(parts)

errors = []
if result.get("startError"):
    errors.append(f"startError={result['startError']}")
if result.get("packetTunnelStatusRawValue") != 3:
    errors.append(f"packetTunnelStatusRawValue={result.get('packetTunnelStatusRawValue')!r}")
if result.get("vpnEnabled") is not True:
    errors.append(f"vpnEnabled={result.get('vpnEnabled')!r}")
runtime_json = result.get("packetTunnelRuntimeStateJson") or ""
if result.get("packetTunnelStatusRawValue") == 3:
    if not runtime_json:
        errors.append("packetTunnelRuntimeStateJson missing")
    else:
        try:
            runtime = json.loads(runtime_json)
        except json.JSONDecodeError as error:
            errors.append(f"packetTunnelRuntimeStateJson invalid JSON: {error}")
        else:
            if runtime.get("vpnActive") is not True:
                errors.append(f"runtime.vpnActive={runtime.get('vpnActive')!r}")
            for key in (
                "tunPacketsRead",
                "tunBytesRead",
                "tunPacketsWritten",
                "tunBytesWritten",
                "tunPacketsDropped",
            ):
                if key not in runtime:
                    errors.append(f"runtime.{key} missing")
            expected = result.get("tunPacketProbeExpectedPackets")
            sent = result.get("tunPacketProbeSentPackets")
            observed = result.get("tunPacketProbeObservedPackets")
            observed_bytes = counter(result.get("tunPacketProbeObservedBytesRead"))
            dropped_delta = counter(result.get("tunPacketProbeDroppedDelta"))
            if (
                result.get("tunPacketProbeReadIncreased") is not True
                or result.get("tunPacketProbeBytesReadIncreased") is not True
                or result.get("tunPacketProbeDroppedIncreased") is not False
                or not isinstance(expected, int)
                or sent != expected
                or not isinstance(observed, int)
                or observed < expected
                or observed_bytes is None
                or observed_bytes <= 0
                or dropped_delta is None
                or dropped_delta != 0
            ):
                errors.append(
                    "tunPacketProbeReadIncreased="
                    f"{result.get('tunPacketProbeReadIncreased')!r} "
                    f"bytesIncreased={result.get('tunPacketProbeBytesReadIncreased')!r} "
                    f"droppedIncreased={result.get('tunPacketProbeDroppedIncreased')!r} "
                    f"baseline={result.get('tunPacketProbeBaselineRead')!r} "
                    f"final={result.get('tunPacketProbeFinalRead')!r} "
                    f"expected={expected!r} sent={sent!r} observed={observed!r} "
                    f"observedBytes={observed_bytes!r} droppedDelta={dropped_delta!r} "
                    f"error={result.get('tunPacketProbeError')!r} "
                    f"sendError={result.get('tunPacketProbeSendError')!r}"
                )

if errors:
    if result.get("tunPacketProbeBaselineRead") is not None:
        print(probe_summary(), file=sys.stderr)
    print("iOS VPN probe failed: " + ", ".join(errors), file=sys.stderr)
    sys.exit(1)

if result.get("tunPacketProbeReadIncreased") is True:
    print("iOS TUN packet probe passed")
    print(probe_summary())
PY
}

run_vpn_cycle() {
  local device="$1"
  local args=(
    --nvpn-debug-exit-probe
    --nvpn-debug-skip-fetch
    --nvpn-debug-wait-seconds "$VPN_START_WAIT_SECS"
    --nvpn-debug-result "$VPN_RESULT_NAME"
    --nvpn-debug-tun-probe-target "$TUN_PACKET_PROBE_TARGET"
    --nvpn-debug-tun-probe-port "$TUN_PACKET_PROBE_PORT"
    --nvpn-debug-tun-probe-count "$TUN_PACKET_PROBE_COUNT"
    --nvpn-debug-tun-probe-wait-seconds "$TUN_PACKET_PROBE_WAIT_SECS"
  )
  if bool_is_true "$CREATE_NETWORK"; then
    args+=(--nvpn-debug-add-network "$DEBUG_NETWORK_NAME")
  fi
  launch_device "$device" "${args[@]}"
  sleep "$((VPN_START_WAIT_SECS + VPN_RESULT_WAIT_SECS))"
  local result_path
  if ! result_path="$(copy_vpn_probe_result "$device")"; then
    copy_ios_debug_logs "$device" || true
    return 1
  fi
  if ! validate_vpn_probe_result "$result_path"; then
    copy_ios_debug_logs "$device" || true
    return 1
  fi
  echo "iOS device VPN probe passed: $result_path"
  cleanup_ios_vpn_after_pass "$device"
}

cleanup_ios_vpn_after_pass() {
  local device="$1"
  bool_is_true "$cleanup_after_vpn_cycle" || return 0
  if ! launch_device "$device" --nvpn-disconnect >/dev/null; then
    echo "iOS VPN cleanup failed: debug disconnect launch failed" >&2
    return 1
  fi
  sleep 2
  echo "iOS VPN cleanup requested: debug disconnect launched"
}

run_device() {
  local device="${NVPN_IOS_DEVICE:-${NVPN_IOS_DEVICE_ID:-}}"
  if [[ -z "$device" ]]; then
    echo "Set NVPN_IOS_DEVICE to the physical iOS device identifier for device smoke" >&2
    exit 1
  fi

  xcrun devicectl device info details --device "$device" >/dev/null
  if bool_is_true "$INSTALL_DEVICE_APP"; then
    install_device_app "$device"
  fi
  if [[ "$vpn_cycle" -eq 1 ]]; then
    run_vpn_cycle "$device"
  else
    launch_device "$device"
  fi
  echo "iOS device smoke launched bundle $BUNDLE_ID"
}

case "$mode" in
  simulator|sim)
    run_simulator
    ;;
  device)
    run_device
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage
    exit 2
    ;;
esac
