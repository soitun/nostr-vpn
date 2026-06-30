#!/usr/bin/env bash
# Per-thread CPU breakdown during sustained TCP iperf3.
# Identifies which tokio worker thread (or sync thread) is the hot one.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY_LIB="$ROOT_DIR/scripts/lib-docker-bench-summary.sh"
# shellcheck source=scripts/lib-docker-bench-summary.sh
source "$SUMMARY_LIB"
docker_bench_apply_local_fips_patch_default
PROJECT_NAME="${PROJECT_NAME:-nvpn-perf}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-perf"
DURATION="${DURATION:-25}"
EXTRA_CONNECT_ENV="$(docker_bench_effective_extra_env)"
docker_bench_validate_connect_env_scope "$EXTRA_CONNECT_ENV"
docker_bench_validate_extra_env_assignments "$EXTRA_CONNECT_ENV"
CONNECT_ENV_PREFIX=""
if [[ -n "$EXTRA_CONNECT_ENV" ]]; then
  CONNECT_ENV_PREFIX="$EXTRA_CONNECT_ENV "
fi

cleanup() {
  if [[ -z "${KEEP:-}" ]]; then
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

wait_for_service() {
  local service="$1"
  for _ in $(seq 1 30); do
    cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$cid" ]] && [[ "$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "perf: service '$service' did not start" >&2
  exit 1
}

nostr_pubkey_from_config() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]\$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' /root/.config/nvpn/config.toml
  " | tr -d '\r"'
}

cleanup
"${COMPOSE[@]}" up -d node-a node-b >/dev/null
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" >/dev/null

BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"

"${COMPOSE[@]}" exec -d node-a sh -lc "${CONNECT_ENV_PREFIX}nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "${CONNECT_ENV_PREFIX}nvpn connect > /tmp/connect.log 2>&1"

for _ in $(seq 1 30); do
  a="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  b="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  if grep -q "mesh: 1/1 peers connected" <<<"$a" \
    && grep -q "mesh: 1/1 peers connected" <<<"$b"; then
    break
  fi
  sleep 1
done

"${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$BOB_TUNNEL_IP" >/dev/null

"${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
sleep 1

echo "Starting iperf3 (TCP 1 stream, ${DURATION}s)..."
"${COMPOSE[@]}" exec -d node-a sh -lc "iperf3 -c $BOB_TUNNEL_IP -t $DURATION -i 0 -f m > /tmp/iperf3-client.log 2>&1"

sleep 5

echo "=== node-a (sender) per-thread top ==="
# top -H shows per-thread; pick top 12 by CPU
"${COMPOSE[@]}" exec -T node-a sh -lc 'top -b -H -n 1 -p $(pgrep -d, -f "nvpn connect")' 2>&1 | tail -25

echo
echo "=== node-b (receiver) per-thread top ==="
"${COMPOSE[@]}" exec -T node-b sh -lc 'top -b -H -n 1 -p $(pgrep -d, -f "nvpn connect")' 2>&1 | tail -25

wait_secs=$((DURATION + 5 - 5))
sleep "$wait_secs"

echo
echo "=== iperf3 result ==="
"${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/iperf3-client.log' 2>&1 | tail -10
