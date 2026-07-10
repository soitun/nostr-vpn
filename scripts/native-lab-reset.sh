#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage:
  NVPN_NATIVE_LAB_ALLOW_RESET=1 scripts/native-lab-reset.sh ios-simulator --udid ID
  NVPN_NATIVE_LAB_ALLOW_RESET=1 scripts/native-lab-reset.sh ios-device --udid ID --bundle-id ID
  NVPN_NATIVE_LAB_ALLOW_RESET=1 scripts/native-lab-reset.sh android --serial ID --bundle-id ID

Use only while scripts/native-lab.py holds the matching host/device reservation.
EOF
}

if [[ "${NVPN_NATIVE_LAB_ALLOW_RESET:-0}" != "1" ]]; then
  echo "native lab reset requires NVPN_NATIVE_LAB_ALLOW_RESET=1" >&2
  exit 75
fi

reset_ios_simulator() {
  local udid=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --udid) udid="$2"; shift 2 ;;
      *) usage; exit 2 ;;
    esac
  done
  [[ -n "$udid" ]] || { usage; exit 2; }
  xcrun simctl list devices available --json | python3 -c \
    'import json,sys; u=sys.argv[1]; d=json.load(sys.stdin).get("devices",{}); raise SystemExit(0 if any(x.get("udid")==u and x.get("isAvailable") for xs in d.values() for x in xs) else 75)' \
    "$udid"
  xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
  xcrun simctl erase "$udid" || exit 75
  xcrun simctl boot "$udid" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$udid" -b >/dev/null || exit 75
}

reset_ios_device() {
  local udid="" bundle_id=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --udid) udid="$2"; shift 2 ;;
      --bundle-id) bundle_id="$2"; shift 2 ;;
      *) usage; exit 2 ;;
    esac
  done
  [[ -n "$udid" && -n "$bundle_id" ]] || { usage; exit 2; }
  xcrun devicectl device uninstall app --device "$udid" "$bundle_id" >/dev/null 2>&1 || exit 75
}

reset_android() {
  local serial="" bundle_id=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --serial) serial="$2"; shift 2 ;;
      --bundle-id) bundle_id="$2"; shift 2 ;;
      *) usage; exit 2 ;;
    esac
  done
  [[ -n "$serial" && -n "$bundle_id" ]] || { usage; exit 2; }
  adb -s "$serial" get-state 2>/dev/null | grep -qx device || exit 75
  adb -s "$serial" shell am force-stop "$bundle_id" >/dev/null 2>&1 || true
  adb -s "$serial" shell pm clear "$bundle_id" >/dev/null || exit 75
}

case "${1:-}" in
  ios-simulator) shift; reset_ios_simulator "$@" ;;
  ios-device) shift; reset_ios_device "$@" ;;
  android) shift; reset_android "$@" ;;
  *) usage; exit 2 ;;
esac
