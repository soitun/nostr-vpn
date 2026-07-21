#!/usr/bin/env bash
# Real macOS daemon transition check: Direct -> WireGuard -> Direct.
#
# This deliberately changes the selected internet source and host routes. Run it
# only on a disposable macOS host/VM with a working provider WireGuard profile.
set -euo pipefail

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "macOS-only e2e; skipping on $(uname -s)"
    exit 0
    ;;
esac

case "${NVPN_RUN_MACOS_WG_DIRECT_E2E:-0}" in
  1|true|TRUE|True|yes|YES|Yes|on|ON|On) ;;
  *)
    echo "Skipping destructive macOS WireGuard/Direct e2e."
    echo "Set NVPN_RUN_MACOS_WG_DIRECT_E2E=1 on a disposable host or VM."
    exit 0
    ;;
esac

NVPN_BIN="${NVPN_E2E_BINARY:-nvpn}"
CONFIG="${NVPN_E2E_CONFIG:-$HOME/Library/Application Support/nvpn/config.toml}"
WG_CONFIG="${NVPN_WG_EXIT_CONFIG_FILE:-}"
PROBE_URL="${NVPN_E2E_INTERNET_URL:-https://example.com/}"
WAIT_SECS="${NVPN_E2E_WAIT_SECS:-20}"

if [[ -z "$WG_CONFIG" || ! -f "$WG_CONFIG" ]]; then
  echo "NVPN_WG_EXIT_CONFIG_FILE must name a readable provider WireGuard config" >&2
  exit 2
fi
if [[ ! -x "$NVPN_BIN" ]] && ! command -v "$NVPN_BIN" >/dev/null 2>&1; then
  echo "nvpn binary not found: $NVPN_BIN" >&2
  exit 2
fi
if [[ ! -f "$CONFIG" ]]; then
  echo "nvpn config not found: $CONFIG" >&2
  exit 2
fi

route_field() {
  local target="$1" field="$2"
  route -n get "$target" 2>/dev/null \
    | awk -v field="$field" '$1 == field ":" { print $2; exit }'
}

split_default_owner() {
  local target="$1" mask iface
  mask="$(route_field "$target" mask)"
  [[ "$mask" == "128.0.0.0" ]] || return 0
  iface="$(route_field "$target" interface)"
  printf '%s\n' "$iface"
}

internet_works() {
  curl -4fsS --max-time 12 "$PROBE_URL" >/dev/null
}

wait_until() {
  local description="$1"
  shift
  local deadline=$((SECONDS + WAIT_SECS))
  until "$@"; do
    if ((SECONDS >= deadline)); then
      echo "timed out waiting for $description" >&2
      return 1
    fi
    sleep 1
  done
}

wireguard_route_is_live() {
  [[ -n "$(split_default_owner 1.0.0.1)" \
    && -n "$(split_default_owner 129.0.0.1)" ]] \
    && internet_works
}

direct_route_is_live() {
  [[ -z "$(split_default_owner 1.0.0.1)" \
    && -z "$(split_default_owner 129.0.0.1)" \
    && "$(route_field default interface)" == "$DIRECT_IFACE" ]] \
    && internet_works
}

force_direct() {
  "$NVPN_BIN" set --config "$CONFIG" --exit-node "" >/dev/null 2>&1 || true
}

DIRECT_IFACE="$(route_field default interface)"
if [[ -z "$DIRECT_IFACE" ]]; then
  echo "could not capture the pre-test Direct interface" >&2
  exit 1
fi
trap force_direct EXIT

start=$SECONDS
"$NVPN_BIN" set --config "$CONFIG" \
  --wireguard-exit-config-file "$WG_CONFIG" \
  --wireguard-exit-enabled true >/dev/null
wait_until "stable WireGuard routes and internet" wireguard_route_is_live

# The route monitor fires after the WG /1s are installed. The old bug removed
# those routes during this ordinary refresh while retaining a live WG handle.
sleep 3
wireguard_route_is_live
wg_elapsed=$((SECONDS - start))

direct_start=$SECONDS
"$NVPN_BIN" set --config "$CONFIG" --exit-node "" >/dev/null
wait_until "the original Direct route and external HTTPS" direct_route_is_live
direct_elapsed=$((SECONDS - direct_start))

trap - EXIT
echo "MACOS_WG_DIRECT_E2E_OK"
echo "WireGuard stable after ${wg_elapsed}s; Direct internet restored after ${direct_elapsed}s"
