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
SIMULATOR_NAME="${NVPN_IOS_SIMULATOR_NAME:-iPhone 17 Pro}"
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
IOS_IDLE_CPU_RESULT_NAME="${NVPN_IOS_IDLE_CPU_RESULT_NAME:-mobile-ios-idle-cpu-$$.json}"
TUN_PACKET_PROBE_SUMMARY_NAME="${NVPN_IOS_TUN_PACKET_PROBE_SUMMARY_NAME:-mobile-ios-tun-probe-summary-$$.json}"
TUN_PACKET_PROBE_TARGET="${NVPN_IOS_TUN_PACKET_PROBE_TARGET:-10.44.255.254}"
TUN_PACKET_PROBE_PORT="${NVPN_IOS_TUN_PACKET_PROBE_PORT:-9}"
TUN_PACKET_PROBE_COUNT="${NVPN_IOS_TUN_PACKET_PROBE_COUNT:-4}"
TUN_PACKET_PROBE_WAIT_SECS="${NVPN_IOS_TUN_PACKET_PROBE_WAIT_SECS:-6}"
TUN_PACKET_PROBE_REQUIRE_REPLY="${NVPN_IOS_TUN_PACKET_PROBE_REQUIRE_REPLY:-0}"
cleanup_after_vpn_cycle="${NVPN_IOS_CLEANUP_AFTER_VPN_CYCLE:-1}"
IDLE_CPU_GATE="${NVPN_IOS_IDLE_CPU_GATE:-${NVPN_IDLE_CPU_GATE:-1}}"
IDLE_CPU_MAX_PERCENT="${NVPN_IOS_IDLE_CPU_MAX_PERCENT:-${NVPN_IDLE_CPU_MAX_PERCENT:-5}}"
IDLE_CPU_SAMPLE_SECONDS="${NVPN_IOS_IDLE_CPU_SAMPLE_SECONDS:-${NVPN_IDLE_CPU_SAMPLE_SECONDS:-10}}"
IDLE_CPU_SETTLE_SECONDS="${NVPN_IOS_IDLE_CPU_SETTLE_SECONDS:-${NVPN_IDLE_CPU_SETTLE_SECONDS:-3}}"
IOS_SIM_PROCESS_NAME="${NVPN_IOS_SIM_PROCESS_NAME:-Nostr VPN}"
SCREENSHOT="$ROOT/artifacts/nostr-vpn-ios.png"

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-ios-smoke.sh [simulator|device] [--install] [--create-network] [--vpn-cycle] [--device DEVICE] [--leave-vpn-active] [--probe-target IP] [--probe-port PORT] [--probe-count N] [--probe-require-reply]

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

Physical-device mode uses NVPN_IOS_DEVICE/NVPN_IOS_DEVICE_ID when set, or auto-
selects the only connected physical iPhone/iPad. Values may live in
.env.mobile.local or shell env. Keep device identifiers and signing details out
of committed files.

Simulator mode is a launch smoke only; iOS Packet Tunnel dataplane checks need
a physical device, and first-run VPN/profile permission prompts may need a
manual approval before --vpn-cycle can run unattended.

Device install mode requires Xcode signing access for NVPN_IOS_BUNDLE_ID and
NVPN_IOS_PACKET_TUNNEL_BUNDLE_ID. Set NVPN_IOS_TEAM_ID in the shell or local
env file; set NVPN_IOS_ALLOW_PROVISIONING_UPDATES=0 to avoid automatic profile
updates.

The physical-device packet probe defaults to 4 UDP packets toward the debug
non-local tunnel probe target. Use --probe-target, --probe-port, --probe-count,
and --probe-require-reply for a reachable peer row that requires native TUN write
counters to increase. NVPN_IOS_TUN_PACKET_PROBE_WAIT_SECS still controls the
observation window.
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
    --probe-target)
      if [[ $# -lt 2 ]]; then
        echo "--probe-target requires a value" >&2
        exit 2
      fi
      TUN_PACKET_PROBE_TARGET="$2"
      shift
      ;;
    --probe-port)
      if [[ $# -lt 2 ]]; then
        echo "--probe-port requires a value" >&2
        exit 2
      fi
      TUN_PACKET_PROBE_PORT="$2"
      shift
      ;;
    --probe-count)
      if [[ $# -lt 2 ]]; then
        echo "--probe-count requires a value" >&2
        exit 2
      fi
      TUN_PACKET_PROBE_COUNT="$2"
      shift
      ;;
    --probe-require-reply)
      TUN_PACKET_PROBE_REQUIRE_REPLY=1
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
  run_ios_simulator_idle_cpu_gate
  echo "iOS simulator smoke passed: $SCREENSHOT"
}

ios_sim_device_id() {
  local booted
  booted="$(xcrun simctl list devices available \
    | sed -n 's/.*iPhone[^()]*(\([0-9A-F-]\{36\}\)) (Booted).*/\1/p' \
    | head -n 1)"
  if [[ -n "$booted" ]]; then
    printf '%s\n' "$booted"
    return
  fi
  xcrun simctl list devices available \
    | sed -n "s/.*$SIMULATOR_NAME (\([0-9A-F-]\{36\}\)).*/\1/p" \
    | head -n 1
}

ios_simulator_app_pid() {
  local device="$1"
  ps -axo pid=,command= \
    | awk -v device="$device" -v app="$IOS_SIM_PROCESS_NAME.app/$IOS_SIM_PROCESS_NAME" '
        !pid && index($0, device) && index($0, app) { pid = $1 }
        END { if (pid) print pid }
      '
}

run_ios_simulator_idle_cpu_gate() {
  case "$IDLE_CPU_GATE" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping iOS simulator idle CPU gate because NVPN_IOS_IDLE_CPU_GATE=$IDLE_CPU_GATE"
      return
      ;;
  esac

  local device pid result_path
  device="$(ios_sim_device_id)"
  if [[ -z "$device" ]]; then
    echo "iOS simulator idle CPU gate failed: no simulator device id found" >&2
    exit 1
  fi
  pid="$(ios_simulator_app_pid "$device" | head -n 1)"
  if [[ -z "$pid" ]]; then
    echo "iOS simulator idle CPU gate failed: process $IOS_SIM_PROCESS_NAME not found on simulator $device" >&2
    exit 1
  fi
  mkdir -p "$VPN_RESULT_DIR"
  result_path="$VPN_RESULT_DIR/mobile-ios-simulator-idle-cpu-$$.json"
  "$ROOT/scripts/idle-cpu-gate.py" host-pid \
    --pid "$pid" \
    --label "iOS simulator app" \
    --artifact "$result_path" \
    --max-percent "$IDLE_CPU_MAX_PERCENT" \
    --sample-seconds "$IDLE_CPU_SAMPLE_SECONDS" \
    --settle-seconds "$IDLE_CPU_SETTLE_SECONDS"
}

auto_select_ios_device() {
  xcrun xctrace list devices 2>/dev/null | awk '
    /^== Devices ==/ { in_devices = 1; next }
    /^== Simulators ==/ { in_devices = 0 }
    in_devices && /iPhone|iPad/ {
      device = $0
      sub(/^.*\(/, "", device)
      sub(/\)[[:space:]]*$/, "", device)
      if (device ~ /^[0-9A-Fa-f-]{8,}$/) devices[++count] = device
    }
    END {
      if (count == 1) { print devices[1]; exit 0 }
      if (count > 1) exit 2
      exit 1
    }
  '
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
    NVPN_BUILD_GIT_SHA="$NVPN_BUILD_GIT_SHA"
    NVPN_BUILD_TIMESTAMP_UTC="$NVPN_BUILD_TIMESTAMP_UTC"
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
      --quiet \
      2>/dev/null
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
  local summary_path="$VPN_RESULT_DIR/$TUN_PACKET_PROBE_SUMMARY_NAME"
  python3 - "$result_path" "$summary_path" "$NVPN_BUILD_GIT_SHA" "$TUN_PACKET_PROBE_REQUIRE_REPLY" <<'PY'
import json
import sys

path, summary_path, expected_build_git_sha, require_reply_raw = sys.argv[1:5]
require_reply = require_reply_raw.strip().lower() in {"1", "true", "yes", "on"}
with open(path, encoding="utf-8") as fh:
    result = json.load(fh)

runtime = None

def counter(value):
    if isinstance(value, int):
        return value
    if isinstance(value, str) and value.isdigit():
        return int(value)
    return None

def probe_values():
    expected = counter(result.get("tunPacketProbeExpectedPackets"))
    sent = counter(result.get("tunPacketProbeSentPackets"))
    observed = counter(result.get("tunPacketProbeObservedPackets"))
    missing = counter(result.get("tunPacketProbeMissingPackets"))
    observed_bytes = counter(result.get("tunPacketProbeObservedBytesRead"))
    observed_written = counter(result.get("tunPacketProbeObservedWritten"))
    observed_bytes_written = counter(result.get("tunPacketProbeObservedBytesWritten"))
    dropped_delta = counter(result.get("tunPacketProbeDroppedDelta"))
    loss_pct = None
    observed_pct = None
    if expected and expected > 0:
        if missing is not None:
            loss_pct = round(missing * 100.0 / expected, 3)
        if observed is not None:
            observed_pct = round(observed * 100.0 / expected, 3)
    return {
        "target": result.get("tunPacketProbeTarget"),
        "port": counter(result.get("tunPacketProbePort")),
        "expected": expected,
        "sent": sent,
        "observed": observed,
        "missing": missing,
        "observedPct": observed_pct,
        "packetLossPct": loss_pct,
        "observedBytesRead": observed_bytes,
        "observedWritten": observed_written,
        "observedBytesWritten": observed_bytes_written,
        "droppedDelta": dropped_delta,
        "firstObservedMs": counter(result.get("tunPacketProbeFirstObservedMs")),
        "elapsedMs": counter(result.get("tunPacketProbeElapsedMs")),
        "polls": counter(result.get("tunPacketProbePolls")),
        "pollIntervalMs": counter(result.get("tunPacketProbePollIntervalMs")),
        "baselineRead": counter(result.get("tunPacketProbeBaselineRead")),
        "finalRead": counter(result.get("tunPacketProbeFinalRead")),
        "baselineBytesRead": counter(result.get("tunPacketProbeBaselineBytesRead")),
        "finalBytesRead": counter(result.get("tunPacketProbeFinalBytesRead")),
        "baselineWritten": counter(result.get("tunPacketProbeBaselineWritten")),
        "finalWritten": counter(result.get("tunPacketProbeFinalWritten")),
        "baselineBytesWritten": counter(result.get("tunPacketProbeBaselineBytesWritten")),
        "finalBytesWritten": counter(result.get("tunPacketProbeFinalBytesWritten")),
        "baselineDropped": counter(result.get("tunPacketProbeBaselineDropped")),
        "finalDropped": counter(result.get("tunPacketProbeFinalDropped")),
        "readIncreased": result.get("tunPacketProbeReadIncreased"),
        "bytesReadIncreased": result.get("tunPacketProbeBytesReadIncreased"),
        "writtenIncreased": result.get("tunPacketProbeWrittenIncreased"),
        "bytesWrittenIncreased": result.get("tunPacketProbeBytesWrittenIncreased"),
        "droppedIncreased": result.get("tunPacketProbeDroppedIncreased"),
        "replyRequired": require_reply,
        "replyObserved": result.get("tunPacketProbeWrittenIncreased") is True
        and result.get("tunPacketProbeBytesWrittenIncreased") is True
        and result.get("tunPacketProbeDroppedIncreased") is False,
        "error": result.get("tunPacketProbeError"),
        "sendError": result.get("tunPacketProbeSendError"),
        "rawOutput": path,
    }

def display(value, suffix=""):
    if value is None:
        return "?"
    if isinstance(value, float):
        return f"{value:.3f}".rstrip("0").rstrip(".") + suffix
    return f"{value}{suffix}"

def probe_summary():
    values = probe_values()
    parts = [
        f"read={values['baselineRead']}->{values['finalRead']}",
        f"observed={values['observed']}/{values['expected']}",
        f"observedPct={display(values['observedPct'], '%')}",
        f"missing={values['missing']}",
        f"lossPct={display(values['packetLossPct'], '%')}",
        f"bytes={values['observedBytesRead']}",
        f"written={values['observedWritten']}",
        f"bytesWritten={values['observedBytesWritten']}",
        f"drops={values['droppedDelta']}",
        f"firstMs={values['firstObservedMs']}",
        f"elapsedMs={values['elapsedMs']}",
        f"polls={values['polls']}",
        f"target={values['target']}",
    ]
    if isinstance(runtime, dict):
        parts.extend([
            f"runtimeRead={runtime.get('tunPacketsRead')}",
            f"runtimeWritten={runtime.get('tunPacketsWritten')}",
            f"runtimeDropped={runtime.get('tunPacketsDropped')}",
        ])
    return "iOS TUN packet probe counters: " + " ".join(parts)

def write_probe_summary(validation_errors=None):
    values = probe_values()
    if isinstance(runtime, dict):
        values["runtime"] = {
            "tunPacketsRead": counter(runtime.get("tunPacketsRead")),
            "tunBytesRead": counter(runtime.get("tunBytesRead")),
            "tunPacketsWritten": counter(runtime.get("tunPacketsWritten")),
            "tunBytesWritten": counter(runtime.get("tunBytesWritten")),
            "tunPacketsDropped": counter(runtime.get("tunPacketsDropped")),
        }
    values["replyRequired"] = require_reply
    values["passed"] = not validation_errors
    if validation_errors:
        values["validationErrors"] = validation_errors
    for key in (
        "phase",
        "packetTunnelStatusRawValue",
        "packetTunnelConnected",
        "vpnEnabled",
        "vpnActive",
        "startError",
        "vpnStartElapsedMs",
        "vpnWaitRequestedMs",
        "statusCollectionElapsedMs",
        "fetchElapsedMs",
        "debugProbeElapsedMs",
        "startedAt",
        "vpnStartFinishedAt",
        "finishedAt",
    ):
        if key in result:
            values[key] = result[key]
    for key in (
        "appBundleIdentifier",
        "appVersionName",
        "appVersionCode",
        "appBuildGitSha",
        "appBuildTimestampUtc",
    ):
        if key in result:
            values[key] = result[key]
    with open(summary_path, "w", encoding="utf-8") as fh:
        json.dump(values, fh, sort_keys=True, indent=2)
        fh.write("\n")
    return summary_path

errors = []
actual_build_git_sha = result.get("appBuildGitSha")
if expected_build_git_sha:
    if not actual_build_git_sha:
        errors.append(f"appBuildGitSha missing expected={expected_build_git_sha!r}")
    elif actual_build_git_sha != expected_build_git_sha:
        errors.append(
            f"appBuildGitSha={actual_build_git_sha!r} expected={expected_build_git_sha!r}"
        )
if result.get("phase") != "finished" or "finishedAt" not in result:
    errors.append(
        "debug probe did not finish "
        f"phase={result.get('phase')!r} finishedAt={result.get('finishedAt')!r}"
    )
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
            observed_written = counter(result.get("tunPacketProbeObservedWritten"))
            observed_bytes_written = counter(result.get("tunPacketProbeObservedBytesWritten"))
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
            if require_reply and (
                result.get("tunPacketProbeWrittenIncreased") is not True
                or result.get("tunPacketProbeBytesWrittenIncreased") is not True
                or observed_written is None
                or observed_written <= 0
                or observed_bytes_written is None
                or observed_bytes_written <= 0
                or dropped_delta != 0
            ):
                errors.append(
                    "tunPacketProbeWrittenIncreased="
                    f"{result.get('tunPacketProbeWrittenIncreased')!r} "
                    f"bytesWrittenIncreased={result.get('tunPacketProbeBytesWrittenIncreased')!r} "
                    f"baselineWritten={result.get('tunPacketProbeBaselineWritten')!r} "
                    f"finalWritten={result.get('tunPacketProbeFinalWritten')!r} "
                    f"observedWritten={observed_written!r} "
                    f"observedBytesWritten={observed_bytes_written!r} "
                    f"droppedDelta={dropped_delta!r}"
                )

if errors:
    summary_written = write_probe_summary(errors)
    if result.get("tunPacketProbeBaselineRead") is not None:
        print(probe_summary(), file=sys.stderr)
    print("iOS TUN packet probe summary: " + summary_written, file=sys.stderr)
    print("iOS VPN probe failed: " + ", ".join(errors), file=sys.stderr)
    sys.exit(1)

if result.get("tunPacketProbeReadIncreased") is True:
    print("iOS TUN packet probe passed")
    print(probe_summary())
    print("iOS TUN packet probe summary: " + write_probe_summary())
PY
}

run_ios_device_idle_cpu_gate() {
  local device="$1"
  local process_pattern="$2"
  local label="$3"
  case "$IDLE_CPU_GATE" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Skipping iOS physical-device idle CPU gate because NVPN_IOS_IDLE_CPU_GATE=$IDLE_CPU_GATE"
      return
      ;;
  esac
  mkdir -p "$VPN_RESULT_DIR"
  "$ROOT/scripts/idle-cpu-gate.py" ios-process \
    --device "$device" \
    --process-pattern "$process_pattern" \
    --label "$label" \
    --artifact "$VPN_RESULT_DIR/$IOS_IDLE_CPU_RESULT_NAME" \
    --max-percent "$IDLE_CPU_MAX_PERCENT" \
    --sample-seconds "$IDLE_CPU_SAMPLE_SECONDS" \
    --settle-seconds "$IDLE_CPU_SETTLE_SECONDS"
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
  run_ios_device_idle_cpu_gate "$device" '^Nostr VPN Tunnel$' "iOS packet tunnel"
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
    if device="$(auto_select_ios_device)"; then
      echo "iOS device smoke auto-selected the only connected physical mobile device"
    else
      case "$?" in
        2)
          echo "Set NVPN_IOS_DEVICE because multiple physical iOS mobile devices are connected" >&2
          ;;
        *)
          echo "Set NVPN_IOS_DEVICE because no physical iOS mobile device could be auto-selected" >&2
          ;;
      esac
      exit 1
    fi
  fi

  xcrun devicectl device info details --device "$device" >/dev/null
  if bool_is_true "$INSTALL_DEVICE_APP"; then
    install_device_app "$device"
  fi
  if [[ "$vpn_cycle" -eq 1 ]]; then
    run_vpn_cycle "$device"
  else
    launch_device "$device"
    run_ios_device_idle_cpu_gate "$device" '^Nostr VPN$' "iOS foreground app"
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
