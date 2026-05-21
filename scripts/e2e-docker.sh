#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-basic"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-vpn"
FIPS_HOST_IFACE="nvpnfips0"
FIPS_HOST_MTU="1280"
FIPS_HOST_TCP_PORT="18080"
FIPS_HOST_TCP_PAYLOAD="alice-to-bob-fips-tcp"

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    docker network inspect "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || break
    sleep 1
  done
}
trap cleanup EXIT

wait_for_service() {
  local service="$1"
  local container_id=""
  for _ in $(seq 1 30); do
    container_id="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$container_id" ]] \
      && [[ "$(docker inspect -f '{{.State.Running}}' "$container_id" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done

  echo "docker e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

nostr_pubkey_from_config() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' /root/.config/nvpn/config.toml
  " | tr -d '\r\"'
}

fips_dns_aaaa() {
  local service="$1"
  local npub="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "dig @::1 -p 5354 +short AAAA '${npub}.fips' | tail -n 1" | tr -d '\r'
}

wait_for_fips_dns_aaaa() {
  local service="$1"
  local npub="$2"
  local resolved=""
  for _ in $(seq 1 20); do
    resolved="$(fips_dns_aaaa "$service" "$npub")"
    if [[ -n "$resolved" ]]; then
      printf '%s\n' "$resolved"
      return 0
    fi
    sleep 1
  done
  return 1
}

assert_fips_host_tunnel() {
  local service="$1"
  local peer_npub="$2"
  local private_link fips_link fips_route resolved

  private_link="$("${COMPOSE[@]}" exec -T "$service" ip link show dev utun100)"
  fips_link="$("${COMPOSE[@]}" exec -T "$service" ip link show dev "$FIPS_HOST_IFACE")"
  fips_route="$("${COMPOSE[@]}" exec -T "$service" ip -6 route show fd00::/8 || true)"

  if ! grep -q "mtu 1150" <<<"$private_link"; then
    echo "docker e2e failed: $service private mesh TUN did not keep safe MTU 1150" >&2
    echo "$private_link" >&2
    exit 1
  fi
  if ! grep -q "mtu $FIPS_HOST_MTU" <<<"$fips_link"; then
    echo "docker e2e failed: $service .fips TUN did not use IPv6 MTU $FIPS_HOST_MTU" >&2
    echo "$fips_link" >&2
    exit 1
  fi
  if ! grep -q "dev $FIPS_HOST_IFACE" <<<"$fips_route"; then
    echo "docker e2e failed: $service fd00::/8 route did not use $FIPS_HOST_IFACE" >&2
    echo "$fips_route" >&2
    exit 1
  fi
  if grep -q "dev utun100" <<<"$fips_route"; then
    echo "docker e2e failed: $service fd00::/8 route leaked onto private mesh TUN" >&2
    echo "$fips_route" >&2
    exit 1
  fi

  resolved="$(wait_for_fips_dns_aaaa "$service" "$peer_npub" || true)"
  if [[ -z "$resolved" ]]; then
    echo "docker e2e failed: $service could not resolve ${peer_npub}.fips" >&2
    exit 1
  fi
}

assert_fips_tcp_crosses() {
  local peer_npub="$1"
  local peer_fips_ip=""

  peer_fips_ip="$(wait_for_fips_dns_aaaa node-a "$peer_npub" || true)"
  if [[ -z "$peer_fips_ip" ]]; then
    echo "docker e2e failed: alice could not resolve bob .fips address for TCP check" >&2
    exit 1
  fi

  "${COMPOSE[@]}" exec -T node-b sh -lc 'rm -f /tmp/bob-fips-tcp.out'
  "${COMPOSE[@]}" exec -d node-b sh -lc "nc -6 -l -p '$FIPS_HOST_TCP_PORT' > /tmp/bob-fips-tcp.out"
  sleep 1

  for _ in $(seq 1 20); do
    printf '%s' "$FIPS_HOST_TCP_PAYLOAD" \
      | "${COMPOSE[@]}" exec -T node-a sh -lc "cat | nc -6 -w 3 '$peer_fips_ip' '$FIPS_HOST_TCP_PORT'" \
      >/dev/null 2>&1 || true
    if "${COMPOSE[@]}" exec -T node-b sh -lc "grep -q '$FIPS_HOST_TCP_PAYLOAD' /tmp/bob-fips-tcp.out 2>/dev/null"; then
      return 0
    fi
    sleep 1
  done

  echo "docker e2e failed: alice could not send TCP to bob over .fips" >&2
  echo "bob .fips address: $peer_fips_ip" >&2
  "${COMPOSE[@]}" exec -T node-a sh -lc "ip -6 route show fd00::/8; ip link show dev '$FIPS_HOST_IFACE'" >&2 || true
  "${COMPOSE[@]}" exec -T node-b sh -lc "ip -6 route show fd00::/8; ip link show dev '$FIPS_HOST_IFACE'; cat /tmp/bob-fips-tcp.out 2>/dev/null || true" >&2 || true
  exit 1
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d node-a node-b >/dev/null
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "docker e2e failed: unable to resolve node npubs from config" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" \
  --fips-host-inbound-tcp-ports "$FIPS_HOST_TCP_PORT" >/dev/null

ALICE_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"

"${COMPOSE[@]}" exec -d node-a sh -lc "nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "nvpn connect > /tmp/connect.log 2>&1"

for _ in $(seq 1 30); do
  ALICE_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  BOB_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"

  if grep -q "mesh: 1/1 peers connected" <<<"$ALICE_CONNECT_LOGS" \
    && grep -q "mesh: 1/1 peers connected" <<<"$BOB_CONNECT_LOGS"; then
    break
  fi

  sleep 1
done

ALICE_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
BOB_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"

if ! grep -q "mesh: 1/1 peers connected" <<<"$ALICE_CONNECT_LOGS"; then
  echo "docker e2e failed: alice mesh did not reach 1/1" >&2
  echo "$ALICE_CONNECT_LOGS"
  exit 1
fi

if ! grep -q "mesh: 1/1 peers connected" <<<"$BOB_CONNECT_LOGS"; then
  echo "docker e2e failed: bob mesh did not reach 1/1" >&2
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

assert_fips_host_tunnel node-a "$BOB_NPUB"
assert_fips_host_tunnel node-b "$ALICE_NPUB"
assert_fips_tcp_crosses "$BOB_NPUB"

if ! "${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$BOB_TUNNEL_IP" >/tmp/ping-a.log; then
  echo "docker e2e failed: ping A -> B failed" >&2
  echo "$ALICE_CONNECT_LOGS"
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

if ! "${COMPOSE[@]}" exec -T node-b ping -c 3 -W 2 "$ALICE_TUNNEL_IP" >/tmp/ping-b.log; then
  echo "docker e2e failed: ping B -> A failed" >&2
  echo "$ALICE_CONNECT_LOGS"
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

echo "--- Alice connect log ---"
echo "$ALICE_CONNECT_LOGS"
echo "--- Bob connect log ---"
echo "$BOB_CONNECT_LOGS"
echo "--- Ping A -> B ---"
cat /tmp/ping-a.log
echo "--- Ping B -> A ---"
cat /tmp/ping-b.log

echo "docker e2e passed: FIPS private mesh established, .fips resolver/tunnel carried TCP, and tunnel pings succeeded"
