#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ios_simulator="${NVPN_LAB_ALLOCATED_IOS_SIMULATOR:-}"
ios_device="${NVPN_LAB_ALLOCATED_IOS_DEVICE:-}"
android_device="${NVPN_LAB_ALLOCATED_ANDROID:-}"

if [[ -z "$ios_simulator" || -z "$ios_device" || -z "$android_device" ]]; then
  echo "managed native allocation is incomplete; run scripts/verify.sh full" >&2
  exit 75
fi

export NVPN_IOS_SIMULATOR_ID="$ios_simulator"
export NVPN_IOS_DEVICE="$ios_device"
export NVPN_ANDROID_SERIAL="$android_device"

require_health() {
  python3 "$ROOT/scripts/native-lab.py" health --health "$1" >/dev/null || exit 75
}

if [[ "${NVPN_NATIVE_LAB_RESET:-0}" == "1" ]]; then
  export NVPN_NATIVE_LAB_ALLOW_RESET=1
  "$ROOT/scripts/native-lab-reset.sh" ios-simulator --udid "$ios_simulator"
  "$ROOT/scripts/native-lab-reset.sh" ios-device \
    --udid "$ios_device" \
    --bundle-id "${NVPN_IOS_BUNDLE_ID:-fi.siriusbusiness.nvpn}"
  "$ROOT/scripts/native-lab-reset.sh" android \
    --serial "$android_device" \
    --bundle-id "${NVPN_ANDROID_PACKAGE:-fi.siriusbusiness.nvpn}"
fi

require_health "ios-simulator:$ios_simulator"
require_health "android:$android_device"
"$ROOT/scripts/mobile-test-kit.sh" simulator

require_health "ios-device:$ios_device"
require_health "android:$android_device"
"$ROOT/scripts/mobile-test-kit.sh" device

require_health "ssh:$NVPN_WINDOWS_SSH_HOST"
NVPN_RELEASE_GATE_WINDOWS_GUI_SMOKE=1 \
NVPN_RELEASE_GATE_WINDOWS_WG_EXIT_E2E=1 \
NVPN_RELEASE_GATE_MACOS_GUI_SMOKE=1 \
NVPN_RELEASE_GATE_LINUX_GUI_SMOKE=1 \
exec "$ROOT/scripts/release-gate.sh"
