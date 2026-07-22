#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/mobile_env.sh"

fail() {
  printf 'mobile physical-device selection harness failed: %s\n' "$*" >&2
  exit 1
}

tmp="$(mktemp -d "${TMPDIR:-/tmp}/nvpn-mobile-device-selection.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/adb" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == "devices" ]] || exit 2
printf 'List of devices attached\n'
printf 'emulator-5554\tdevice\n'
printf 'physical-device\tdevice\n'
printf 'offline-device\toffline\n'
EOF
chmod +x "$tmp/adb"

selected="$(select_physical_android_serial "$tmp/adb" "")"
[[ "$selected" == "physical-device" ]] \
  || fail "automatic selection chose '$selected' instead of the physical device"

selected="$(select_physical_android_serial "$tmp/adb" "physical-device")"
[[ "$selected" == "physical-device" ]] \
  || fail "explicit physical selection returned '$selected'"

if select_physical_android_serial "$tmp/adb" "emulator-5554" >/dev/null 2>&1; then
  fail "physical-only selection accepted an emulator"
fi

cat >"$tmp/adb" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == "devices" ]] || exit 2
printf 'List of devices attached\n'
printf 'emulator-5554\tdevice\n'
EOF
chmod +x "$tmp/adb"

if select_physical_android_serial "$tmp/adb" "" >/dev/null 2>&1; then
  fail "physical-only selection fell back to an emulator"
fi

printf 'mobile physical-device selection harness passed\n'
