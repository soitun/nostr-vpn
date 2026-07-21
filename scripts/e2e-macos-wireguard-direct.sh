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
WAIT_SECS="${NVPN_E2E_WAIT_SECS:-60}"
DROP_UNDERLAY_DEFAULT="${NVPN_E2E_DROP_UNDERLAY_DEFAULT:-0}"

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

external_dns_works() {
  dscacheutil -q host -a name example.com 2>/dev/null \
    | grep -Eq '(^|[[:space:]])ip_address:'
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
    && external_dns_works \
    && internet_works
}

direct_route_is_live() {
  [[ -z "$(split_default_owner 1.0.0.1)" \
    && -z "$(split_default_owner 129.0.0.1)" \
    && "$(route_field default interface)" == "$DIRECT_IFACE" \
    && "$(route_field default gateway)" == "$DIRECT_GATEWAY" ]] \
    && external_dns_works \
    && internet_works
}

restore_direct() {
  "$NVPN_BIN" set --config "$CONFIG" --exit-node "" >/dev/null 2>&1 || true
  if [[ "$DROP_UNDERLAY_DEFAULT" == "1" ]]; then
    if [[ -n "${SCOPED_DEFAULT_IFACE:-}" ]]; then
      sudo -n /sbin/route -n delete -ifscope "$SCOPED_DEFAULT_IFACE" \
        default -interface "$SCOPED_DEFAULT_IFACE" >/dev/null 2>&1 || true
    fi
    sudo -n /sbin/route -n add default "$DIRECT_GATEWAY" >/dev/null 2>&1 || true
  fi
}

DIRECT_IFACE="$(route_field default interface)"
DIRECT_GATEWAY="$(route_field default gateway)"
if [[ -z "$DIRECT_IFACE" || -z "$DIRECT_GATEWAY" || "$DIRECT_GATEWAY" == link#* ]]; then
  echo "could not capture the pre-test physical Direct route" >&2
  exit 1
fi
SCOPED_DEFAULT_IFACE=""
if [[ "$DROP_UNDERLAY_DEFAULT" == "1" ]]; then
  for iface in $(ifconfig -l); do
    if [[ "$iface" == utun* ]] && ifconfig "$iface" 2>/dev/null | grep -q '^[[:space:]]*inet '; then
      SCOPED_DEFAULT_IFACE="$iface"
      break
    fi
  done
  if [[ -z "$SCOPED_DEFAULT_IFACE" ]]; then
    echo "the scoped-default recovery test needs an existing IPv4 utun" >&2
    exit 1
  fi
fi
trap restore_direct EXIT

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

if [[ "$DROP_UNDERLAY_DEFAULT" == "1" ]]; then
  # Reproduce recovery from older nVPN builds that could remove the physical
  # default during WG cleanup. Retain an interface-scoped utun default shaped
  # like a packet-tunnel Network Extension (for example Tailscale), too. The
  # live WG /1 routes keep Internet working while the Direct transition is
  # asked to restore the DHCP underlay alongside that foreign-looking route.
  sudo -n /sbin/route -n add -ifscope "$SCOPED_DEFAULT_IFACE" \
    default -interface "$SCOPED_DEFAULT_IFACE" >/dev/null
  sudo -n /sbin/route -n delete default "$DIRECT_GATEWAY" >/dev/null
  if netstat -rn -f inet \
    | awk -v iface="$DIRECT_IFACE" '$1 == "default" && $NF == iface { found=1 } END { exit found ? 0 : 1 }'
  then
    echo "failed to remove the physical default for the recovery test" >&2
    exit 1
  fi
  if ! netstat -rn -f inet \
    | awk -v iface="$SCOPED_DEFAULT_IFACE" '$1 == "default" && $NF == iface { found=1 } END { exit found ? 0 : 1 }'
  then
    echo "failed to install the scoped utun default for the recovery test" >&2
    exit 1
  fi
  wireguard_route_is_live
fi

direct_start=$SECONDS
"$NVPN_BIN" set --config "$CONFIG" --exit-node "" >/dev/null
wait_until "the original Direct route and external HTTPS" direct_route_is_live
direct_elapsed=$((SECONDS - direct_start))

if [[ -n "$SCOPED_DEFAULT_IFACE" ]]; then
  sudo -n /sbin/route -n delete -ifscope "$SCOPED_DEFAULT_IFACE" \
    default -interface "$SCOPED_DEFAULT_IFACE" >/dev/null
  SCOPED_DEFAULT_IFACE=""
  direct_route_is_live
fi

trap - EXIT
echo "MACOS_WG_DIRECT_E2E_OK"
echo "WireGuard route, DNS, and HTTPS stable after ${wg_elapsed}s; Direct route, DNS, and HTTPS restored after ${direct_elapsed}s"
