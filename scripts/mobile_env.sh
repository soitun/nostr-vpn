#!/usr/bin/env bash

load_mobile_env() {
  local root="$1"
  local env_file="${NVPN_MOBILE_ENV_FILE:-$root/.env.mobile.local}"

  if [[ -f "$env_file" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$env_file"
    set +a
  fi
}

select_physical_android_serial() {
  local adb="$1"
  local requested="${2:-}"
  local selected=""

  if [[ -n "$requested" ]]; then
    if [[ "$requested" == emulator-* ]]; then
      printf 'Physical Android test refuses emulator serial %s\n' "$requested" >&2
      return 1
    fi
    if ! "$adb" devices 2>/dev/null | awk -v requested="$requested" '
      NR > 1 && $1 == requested && $2 == "device" { found = 1 }
      END { exit !found }
    '; then
      printf 'Requested physical Android device is not online: %s\n' "$requested" >&2
      return 1
    fi
    printf '%s\n' "$requested"
    return
  fi

  selected="$("$adb" devices 2>/dev/null | awk '
    NR > 1 && $2 == "device" && $1 !~ /^emulator-/ {
      print $1
      exit
    }
  ')"
  if [[ -z "$selected" ]]; then
    printf 'No physical Android device is online; emulators do not satisfy this test\n' >&2
    return 1
  fi
  printf '%s\n' "$selected"
}

ios_device_launch() {
  local device="$1"
  local bundle_id="$2"
  shift 2

  if [[ "$#" -eq 0 ]]; then
    xcrun devicectl device process launch \
      --device "$device" \
      "$bundle_id"
    return
  fi

  local encoded_arguments
  encoded_arguments="$(python3 - "$@" <<'PY'
import base64
import json
import sys

payload = json.dumps(sys.argv[1:], separators=(",", ":")).encode()
print(base64.urlsafe_b64encode(payload).decode().rstrip("="))
PY
)"
  xcrun devicectl device process launch \
    --device "$device" \
    --payload-url "nvpn://debug/automation?arguments=$encoded_arguments" \
    "$bundle_id"
}
