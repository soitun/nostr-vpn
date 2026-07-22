#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/mobile_env.sh"
IMAGE="${NVPN_MOBILE_WG_EXIT_IMAGE:-nostr-vpn-mobile-wireguard-exit-e2e}"
CONTAINER="${NVPN_MOBILE_WG_EXIT_CONTAINER:-nostr-vpn-mobile-wireguard-exit-e2e}"
HOST_PORT="${NVPN_MOBILE_WG_EXIT_HOST_PORT:-51886}"
TUNNEL_SERVER_IP="${NVPN_MOBILE_WG_EXIT_SERVER_IP:-10.99.77.1}"
TUNNEL_CLIENT_IP="${NVPN_MOBILE_WG_EXIT_CLIENT_IP:-10.99.77.2}"
DNS_NAME="${NVPN_MOBILE_WG_EXIT_DNS_NAME:-wireguard-exit.nvpn-e2e.test}"
DIRECT_HOST="${NVPN_MOBILE_WG_EXIT_DIRECT_HOST:-example.com}"
DIRECT_URL="${NVPN_MOBILE_WG_EXIT_DIRECT_URL:-https://example.com/}"
PLATFORMS="${NVPN_MOBILE_WG_EXIT_PLATFORMS:-android,ios}"
INSTALL_IOS="${NVPN_MOBILE_WG_EXIT_INSTALL_IOS:-1}"
FIXTURE_DIR=""
ANDROID_DEVICE_SERIAL=""

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-wireguard-exit-e2e.sh [android|ios|all]

Runs a real WireGuard exit and DNS resolver in Docker, then proves on selected
physical mobile devices that:
  - native device DNS and Internet work before the VPN starts;
  - default traffic crosses the WireGuard exit;
  - the WireGuard profile DNS resolves a fixture-only name;
  - public HTTPS works through the exit; and
  - native device DNS and Internet return after disconnect.

The host and devices must share a LAN. Override the endpoint address with
NVPN_MOBILE_WG_EXIT_HOST_IP when automatic en0 discovery is unsuitable.
EOF
}

case "${1:-all}" in
  all) PLATFORMS="android,ios" ;;
  android|ios) PLATFORMS="$1" ;;
  -h|--help|help) usage; exit 0 ;;
  *) usage; exit 2 ;;
esac

has_platform() {
  local requested="$1"
  [[ ",${PLATFORMS// /}," == *",$requested,"* ]]
}

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  if [[ -n "$FIXTURE_DIR" ]]; then
    rm -rf "$FIXTURE_DIR"
  fi
}
trap cleanup EXIT INT TERM

for command in docker wg; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "mobile WireGuard exit e2e requires $command" >&2
    exit 1
  fi
done

if has_platform android; then
  if ! command -v adb >/dev/null 2>&1; then
    echo "mobile WireGuard exit e2e requires adb for the physical Android device" >&2
    exit 1
  fi
  ANDROID_DEVICE_SERIAL="$(select_physical_android_serial \
    "$(command -v adb)" \
    "${NVPN_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}")"
fi

HOST_IP="${NVPN_MOBILE_WG_EXIT_HOST_IP:-}"
if [[ -z "$HOST_IP" && "$(uname -s)" == "Darwin" ]]; then
  HOST_IP="$(ipconfig getifaddr en0 2>/dev/null || true)"
fi
if [[ -z "$HOST_IP" && "$(uname -s)" == "Linux" ]]; then
  HOST_IP="$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{ for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit } }')"
fi
if [[ -z "$HOST_IP" ]]; then
  echo "Could not resolve a LAN host address; set NVPN_MOBILE_WG_EXIT_HOST_IP" >&2
  exit 1
fi

FIXTURE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nvpn-mobile-wg-exit.XXXXXX")"
chmod 700 "$FIXTURE_DIR"
umask 077
wg genkey >"$FIXTURE_DIR/server.key"
wg pubkey <"$FIXTURE_DIR/server.key" >"$FIXTURE_DIR/server.pub"
wg genkey >"$FIXTURE_DIR/client.key"
wg pubkey <"$FIXTURE_DIR/client.key" >"$FIXTURE_DIR/client.pub"

cat >"$FIXTURE_DIR/client.conf" <<EOF
[Interface]
PrivateKey = $(<"$FIXTURE_DIR/client.key")
Address = $TUNNEL_CLIENT_IP/32
DNS = $TUNNEL_SERVER_IP
MTU = 1280

[Peer]
PublicKey = $(<"$FIXTURE_DIR/server.pub")
Endpoint = $HOST_IP:$HOST_PORT
AllowedIPs = 0.0.0.0/0
PersistentKeepalive = 2
EOF

docker build -q -f "$ROOT/Dockerfile.mobile-wireguard-exit-e2e" -t "$IMAGE" "$ROOT" >/dev/null
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
docker run -d \
  --name "$CONTAINER" \
  --cap-add NET_ADMIN \
  --device /dev/net/tun \
  --sysctl net.ipv4.ip_forward=1 \
  -p "$HOST_PORT:51820/udp" \
  -v "$FIXTURE_DIR:/fixture" \
  -e "NVPN_MOBILE_WG_TUNNEL_CIDR=$TUNNEL_SERVER_IP/24" \
  -e "NVPN_MOBILE_WG_CLIENT_IP=$TUNNEL_CLIENT_IP" \
  -e "NVPN_MOBILE_WG_DNS_NAME=$DNS_NAME" \
  "$IMAGE" >/dev/null

for _ in $(seq 1 100); do
  [[ -f "$FIXTURE_DIR/ready" ]] && break
  if [[ "$(docker inspect -f '{{.State.Running}}' "$CONTAINER" 2>/dev/null || true)" != "true" ]]; then
    docker logs "$CONTAINER" >&2 || true
    echo "mobile WireGuard exit fixture stopped before becoming ready" >&2
    exit 1
  fi
  sleep 0.1
done
if [[ ! -f "$FIXTURE_DIR/ready" ]]; then
  docker logs "$CONTAINER" >&2 || true
  echo "mobile WireGuard exit fixture did not become ready" >&2
  exit 1
fi

wg_bytes() {
  docker exec "$CONTAINER" wg show wg0 transfer \
    | awk '{ rx += $2; tx += $3 } END { printf "%d\t%d\n", rx, tx }'
}

forward_packets() {
  docker exec "$CONTAINER" iptables -L nvpn-mobile-wg-forward -v -n -x \
    | awk '$3 == "ACCEPT" && ($7 == "wg0" || $8 == "wg0") { packets += $1 } END { print packets + 0 }'
}

dns_query_count() {
  local name="$1"
  if [[ ! -f "$FIXTURE_DIR/dns.log" ]]; then
    echo 0
    return
  fi
  grep -Fci "$name" "$FIXTURE_DIR/dns.log" || true
}

assert_platform_traffic() {
  local platform="$1" before_bytes="$2" before_forward="$3" before_dns="$4"
  local after_bytes after_forward after_dns before_rx before_tx after_rx after_tx
  after_bytes="$(wg_bytes)"
  after_forward="$(forward_packets)"
  after_dns="$(dns_query_count "$DNS_NAME")"
  IFS=$'\t' read -r before_rx before_tx <<<"$before_bytes"
  IFS=$'\t' read -r after_rx after_tx <<<"$after_bytes"
  if (( after_rx <= before_rx || after_tx <= before_tx )); then
    echo "$platform mobile exit e2e failed: WireGuard transfer counters did not increase (rx $before_rx->$after_rx, tx $before_tx->$after_tx)" >&2
    exit 1
  fi
  if (( after_forward <= before_forward )); then
    echo "$platform mobile exit e2e failed: no forwarded Internet traffic crossed wg0 ($before_forward->$after_forward packets)" >&2
    exit 1
  fi
  if (( after_dns <= before_dns )); then
    echo "$platform mobile exit e2e failed: WireGuard DNS did not receive a $DNS_NAME query ($before_dns->$after_dns)" >&2
    exit 1
  fi
  echo "$platform WireGuard exit passed: transfer rx=$before_rx->$after_rx tx=$before_tx->$after_tx forwarded=$before_forward->$after_forward dnsQueries=$before_dns->$after_dns"
}

run_android() {
  local before_bytes before_forward before_dns
  before_bytes="$(wg_bytes)"
  before_forward="$(forward_packets)"
  before_dns="$(dns_query_count "$DNS_NAME")"
  env \
    NVPN_ANDROID_SERIAL="$ANDROID_DEVICE_SERIAL" \
    NVPN_ANDROID_PACKAGE="${NVPN_ANDROID_PACKAGE:-fi.siriusbusiness.nvpn.mobileexit}" \
    NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG_FILE="$FIXTURE_DIR/client.conf" \
    NVPN_ANDROID_EXIT_PROBE_HOST="$DNS_NAME" \
    NVPN_ANDROID_EXIT_PROBE_EXPECTED_IP="$TUNNEL_SERVER_IP" \
    NVPN_ANDROID_DIRECT_PROBE_HOST="$DIRECT_HOST" \
    "$ROOT/scripts/mobile-android-smoke.sh" \
      --create-network \
      --accept-vpn-dialog \
      --vpn-cycle \
      --probe-target "$TUNNEL_SERVER_IP" \
      --probe-count 4 \
      --probe-require-reply
  assert_platform_traffic Android "$before_bytes" "$before_forward" "$before_dns"
}

run_ios() {
  local before_bytes before_forward before_dns
  local ios_args=(
    device
    --create-network
    --vpn-cycle
    --probe-target "$TUNNEL_SERVER_IP"
    --probe-port 9
    --probe-count 4
    --probe-require-reply
  )
  case "$INSTALL_IOS" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off) ;;
    *) ios_args=(device --install "${ios_args[@]:1}") ;;
  esac
  before_bytes="$(wg_bytes)"
  before_forward="$(forward_packets)"
  before_dns="$(dns_query_count "$DNS_NAME")"
  env \
    NVPN_IOS_DEBUG_WIREGUARD_CONFIG_FILE="$FIXTURE_DIR/client.conf" \
    NVPN_IOS_EXIT_PROBE_HOST="$DNS_NAME" \
    NVPN_IOS_EXIT_PROBE_EXPECTED_IP="$TUNNEL_SERVER_IP" \
    NVPN_IOS_EXIT_PROBE_URL="$DIRECT_URL" \
    NVPN_IOS_DIRECT_PROBE_HOST="$DIRECT_HOST" \
    NVPN_IOS_DIRECT_PROBE_URL="$DIRECT_URL" \
    NVPN_IOS_VERIFY_DIRECT_RESTORATION=1 \
    "$ROOT/scripts/mobile-ios-smoke.sh" "${ios_args[@]}"
  assert_platform_traffic iOS "$before_bytes" "$before_forward" "$before_dns"
}

if has_platform android; then
  run_android
fi
if has_platform ios; then
  run_ios
fi

echo "Mobile WireGuard exit e2e passed for: $PLATFORMS"
