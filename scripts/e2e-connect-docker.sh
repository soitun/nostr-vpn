#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="${NVPN_E2E_PROJECT_NAME:-nostr-vpn-e2e-connect}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")
NETWORK_ID="${NVPN_CONNECT_E2E_NETWORK_ID:-docker-connect}"
IDLE_CPU_MAX_PERCENT="${NVPN_E2E_IDLE_CPU_MAX_PERCENT:-80}"
DIRECT_COUNTER_COMMENT="nvpn-connect-direct-udp"
# shellcheck source=scripts/lib-docker-direct-udp.sh
source "$ROOT_DIR/scripts/lib-docker-direct-udp.sh"

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

  echo "connect e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

nostr_pubkey_from_config() {
  local service="$1"
  local config_path="${2:-/root/.config/nvpn/config.toml}"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' '$config_path'
  " | tr -d '\r\"'
}

use_fips_only_control_pubsub() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    cat >> '$CONFIG_PATH' <<'EOF'

[nostr.pubsub]
mode = \"client\"
EOF
  "
}

wait_for_control_event() {
  local service="$1"
  local event_id="$2"
  local events=""
  for _ in $(seq 1 40); do
    events="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      'cat /root/.config/nvpn/control-pubsub-events.json 2>/dev/null || true' | tr -d '\r')"
    if jq -e --arg event_id "$event_id" \
      '.events | any(.id == $event_id)' >/dev/null 2>&1 <<<"$events"; then
      return 0
    fi
    sleep 0.25
  done
  echo "connect e2e failed: control pubsub event $event_id did not reach $service" >&2
  printf '%s\n' "$events" >&2
  return 1
}

assert_idle_cpu_below() {
  local service="$1"
  local pids
  pids="$("${COMPOSE[@]}" exec -T "$service" sh -lc 'pgrep -d, -x nvpn || true' | tr -d '\r')"
  if [[ -z "$pids" ]]; then
    echo "connect e2e failed: no nvpn process found on $service for idle CPU guard" >&2
    exit 1
  fi

  local max_cpu
  max_cpu="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
    "top -b -n 3 -d 1 -p '$pids' | awk '\$12 == \"nvpn\" && \$9 + 0 > max { max = \$9 + 0 } END { printf \"%.1f\", max + 0 }'" \
    | tr -d '\r')"
  echo "--- $service idle nvpn CPU max: ${max_cpu}% ---"
  if awk -v max="$max_cpu" -v limit="$IDLE_CPU_MAX_PERCENT" 'BEGIN { exit !(max > limit) }'; then
    echo "connect e2e failed: $service idle nvpn CPU ${max_cpu}% exceeded ${IDLE_CPU_MAX_PERCENT}%" >&2
    exit 1
  fi
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d node-a node-b >/dev/null
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
CONFIG_PATH="/root/.config/nvpn/config.toml"
use_fips_only_control_pubsub node-a
use_fips_only_control_pubsub node-b
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "connect e2e failed: unable to resolve node npubs from config" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --paid-exit-enabled true \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" >/dev/null

"${COMPOSE[@]}" exec -d node-a sh -lc "nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "nvpn connect > /tmp/connect.log 2>&1"

for _ in $(seq 1 20); do
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
  echo "connect e2e failed: alice mesh did not reach 1/1" >&2
  echo "$ALICE_CONNECT_LOGS"
  exit 1
fi

if ! grep -q "mesh: 1/1 peers connected" <<<"$BOB_CONNECT_LOGS"; then
  echo "connect e2e failed: bob mesh did not reach 1/1" >&2
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

sleep 2

BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip --peer --discover-secs 0 | head -n1 | tr -d '\r')"
ALICE_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip --peer --discover-secs 0 | head -n1 | tr -d '\r')"

if [[ -z "$BOB_TUNNEL_IP" || -z "$ALICE_TUNNEL_IP" ]]; then
  echo "connect e2e failed: unable to resolve FIPS peer tunnel IPs" >&2
  echo "$ALICE_CONNECT_LOGS"
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

install_direct_underlay_counter node-a 10.203.0.11
install_direct_underlay_counter node-b 10.203.0.10
DIRECT_BEFORE_A="$(direct_underlay_bytes node-a)"
DIRECT_BEFORE_B="$(direct_underlay_bytes node-b)"

# Keep real roster traffic moving in both directions while the same product
# processes exchange a signed event through the shared TCP/FIPS pubsub driver.
"${COMPOSE[@]}" exec -T node-a ping -c 20 -i 0.1 -W 2 "$BOB_TUNNEL_IP" >/tmp/ping-a.log &
PING_A_PID=$!
"${COMPOSE[@]}" exec -T node-b ping -c 20 -i 0.1 -W 2 "$ALICE_TUNNEL_IP" >/tmp/ping-b.log &
PING_B_PID=$!

OFFER_JSON="$("${COMPOSE[@]}" exec -T node-a env RUST_LOG=warn nvpn paid-exit offer \
  --config "$CONFIG_PATH" --offer-id iris-stack-process-gate --json | tr -d '\r')"
CONTROL_EVENT="$(jq -c '.event' <<<"$OFFER_JSON")"
CONTROL_EVENT_ID="$(jq -r '.id' <<<"$CONTROL_EVENT")"
printf '%s' "$CONTROL_EVENT" | "${COMPOSE[@]}" exec -T node-a sh -lc \
  'cat > /tmp/iris-stack-control-event.json'
"${COMPOSE[@]}" exec -T node-a nvpn pubsub publish \
  --config "$CONFIG_PATH" --event /tmp/iris-stack-control-event.json --json >/dev/null
wait_for_control_event node-b "$CONTROL_EVENT_ID"

PING_FAILED=0
wait "$PING_A_PID" || PING_FAILED=1
wait "$PING_B_PID" || PING_FAILED=1
if [[ "$PING_FAILED" -ne 0 ]]; then
  echo "connect e2e failed: concurrent roster traffic failed" >&2
  cat /tmp/ping-a.log /tmp/ping-b.log >&2 2>/dev/null || true
  exit 1
fi

DIRECT_AFTER_A="$(direct_underlay_bytes node-a)"
DIRECT_AFTER_B="$(direct_underlay_bytes node-b)"
if (( DIRECT_AFTER_A <= DIRECT_BEFORE_A || DIRECT_AFTER_B <= DIRECT_BEFORE_B )); then
  echo "connect e2e failed: application-owned direct UDP roster links did not carry concurrent traffic" >&2
  echo "node-a direct bytes: $DIRECT_BEFORE_A -> $DIRECT_AFTER_A" >&2
  echo "node-b direct bytes: $DIRECT_BEFORE_B -> $DIRECT_AFTER_B" >&2
  exit 1
fi

assert_idle_cpu_below node-a
assert_idle_cpu_below node-b

echo "--- Alice connect log ---"
echo "$ALICE_CONNECT_LOGS"
echo "--- Bob connect log ---"
echo "$BOB_CONNECT_LOGS"
echo "--- Ping A -> B ---"
cat /tmp/ping-a.log
echo "--- Ping B -> A ---"
cat /tmp/ping-b.log
echo "--- Shared control pubsub ---"
echo "event $CONTROL_EVENT_ID reached node-b"
echo "node-a direct UDP bytes: $DIRECT_BEFORE_A -> $DIRECT_AFTER_A"
echo "node-b direct UDP bytes: $DIRECT_BEFORE_B -> $DIRECT_AFTER_B"

echo "connect docker e2e passed: two nvpn processes kept explicit direct UDP roster links while shared TCP/FIPS control pubsub delivered a signed event"
