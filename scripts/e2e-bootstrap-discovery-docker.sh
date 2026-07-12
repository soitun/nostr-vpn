#!/usr/bin/env bash
# Deterministic e2e for the "find peers over relays" toggle and the static /
# bootstrap transit path. Both nodes run with Nostr relay discovery DISABLED and
# reach each other purely over a direct FIPS static endpoint on the Docker
# bridge — no public relays, no STUN, no NAT traversal. Proves a join request
# still flows over the FIPS control channel when relays are off, which is the
# same mechanism the built-in bootstrap nodes use.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-bootstrap"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-bootstrap-discovery"
REQUESTER_NAME="iphone"

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    docker network inspect "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || break
    sleep 1
  done
}

dump_debug() {
  set +e
  echo "bootstrap-discovery docker e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a node-c; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 160 "$service" || true
    echo "--- $service config ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "cat /root/.config/nvpn/config.toml 2>/dev/null || true" || true
    echo "--- $service daemon.log ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "tail -n 200 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
  done
}

on_exit() {
  local exit_code=$?
  if [[ $exit_code -ne 0 ]]; then
    dump_debug
  fi
  cleanup
  exit "$exit_code"
}
trap on_exit EXIT

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

  echo "bootstrap-discovery docker e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

read_npub() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "nvpn init --force >/dev/null && awk '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' /root/.config/nvpn/config.toml" | tr -d '\r\"'
}

# Relay discovery OFF: the node must connect through configured static endpoints
# (the same path the built-in bootstrap nodes use), not relays.
start_daemon_no_relays() {
  local service="$1"
  if ! "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "nvpn start --daemon --connect >/tmp/nvpn-start.log 2>&1"; then
    echo "bootstrap-discovery docker e2e failed: daemon start failed on $service" >&2
    "${COMPOSE[@]}" exec -T "$service" sh -lc "cat /tmp/nvpn-start.log" >&2 || true
    exit 1
  fi
}

assert_no_relay_streaming() {
  local service="$1"
  # With relay discovery off we must not be opening Nostr relay subscriptions.
  if "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "grep -qiE 'subscrib(e|ing).*relay|relay.*subscrib|Failed to stream events' /root/.config/nvpn/daemon.log 2>/dev/null"; then
    echo "bootstrap-discovery docker e2e failed: $service streamed from relays with discovery off" >&2
    exit 1
  fi
}

wait_for_inbound_join_request() {
  local service="$1"
  local requester="$2"
  local requester_name="$3"
  local found=""

  for _ in $(seq 1 60); do
    found="$("${COMPOSE[@]}" exec -T \
      -e REQUESTER="$requester" \
      -e REQUESTER_NAME="$requester_name" \
      "$service" perl -0ne '
  my $requester = $ENV{REQUESTER};
  my $requester_name = $ENV{REQUESTER_NAME};
  while (/\[\[networks\.inbound_join_requests\]\]\s*\n(.*?)(?=^\[|\z)/msg) {
    my $block = $1;
    if ($block =~ /^requester\s*=\s*"\Q$requester\E"\s*$/m
      && $block =~ /^requester_node_name\s*=\s*"\Q$requester_name\E"\s*$/m) {
      print "yes";
      exit;
    }
  }
' /root/.config/nvpn/config.toml || true)"
    if [[ "$found" == "yes" ]]; then
      return 0
    fi
    sleep 1
  done

  echo "bootstrap-discovery docker e2e failed: admin never persisted request from $requester" >&2
  "${COMPOSE[@]}" exec -T "$service" sh -lc "cat /root/.config/nvpn/config.toml" >&2 || true
  exit 1
}

config_array_contains() {
  local service="$1"
  local key="$2"
  local value="$3"
  "${COMPOSE[@]}" exec -T -e KEY="$key" -e VALUE="$value" "$service" perl -0ne '
    my $key = $ENV{KEY};
    my $value = $ENV{VALUE};
    if (/^\Q$key\E\s*=\s*\[[^\]]*\Q$value\E[^\]]*\]/m) { print "yes" }
  ' /root/.config/nvpn/config.toml
}

wait_for_roster_member() {
  local service="$1"
  local member="$2"
  local description="$3"
  for _ in $(seq 1 80); do
    if [[ "$(config_array_contains "$service" participants "$member" || true)" == "yes" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "bootstrap-discovery docker e2e failed: $description" >&2
  exit 1
}

wait_for_connected_peer() {
  local service="$1"
  local description="$2"
  local count=""
  for _ in $(seq 1 80); do
    count="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
      "nvpn status --json --discover-secs 0 | perl -0ne 'print \$1 if /\"connected_peer_count\"\\s*:\\s*(\\d+)/'" || true)"
    if [[ "$count" == "1" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "bootstrap-discovery docker e2e failed: $description (connected_peer_count='$count')" >&2
  exit 1
}

ping_until_success() {
  local service="$1"
  local target="$2"
  for _ in $(seq 1 30); do
    if "${COMPOSE[@]}" exec -T "$service" ping -c 1 -W 1 "$target" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

accept_join_request_through_gui_api() {
  local service="$1"
  local requester="$2"
  local network_id=""
  network_id="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
    "awk '/^\[\[networks\]\]/{in_network=1; next} in_network && /^id[[:space:]]*=/{gsub(/[[:space:]\"]/, \"\", \$2); print \$2; exit}' /root/.config/nvpn/config.toml" | tr -d '\r')"
  if [[ -z "$network_id" ]]; then
    echo "bootstrap-discovery docker e2e failed: active GUI network ID was unavailable" >&2
    exit 1
  fi
  "${COMPOSE[@]}" exec -d "$service" sh -lc \
    "nostr-vpn-web --listen 127.0.0.1:8081 --config /root/.config/nvpn/config.toml --nvpn /usr/local/bin/nvpn >/tmp/nvpn-web.log 2>&1"
  for _ in $(seq 1 30); do
    if "${COMPOSE[@]}" exec -T "$service" curl -fsS http://127.0.0.1:8081/api/health >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  local http_code=""
  http_code="$("${COMPOSE[@]}" exec -T \
    -e NETWORK_ID="$network_id" \
    -e REQUESTER="$requester" \
    "$service" sh -lc 'curl -sS -o /tmp/accept-join-response.json -w "%{http_code}" -X POST -H "content-type: application/json" \
      --data "{\"networkId\":\"$NETWORK_ID\",\"requesterNpub\":\"$REQUESTER\"}" \
      http://127.0.0.1:8081/api/accept_join_request')"
  if [[ "$http_code" != "200" ]]; then
    echo "bootstrap-discovery docker e2e failed: GUI accept returned HTTP $http_code" >&2
    "${COMPOSE[@]}" exec -T "$service" sh -lc \
      'cat /tmp/accept-join-response.json; tail -n 100 /tmp/nvpn-web.log' >&2 || true
    exit 1
  fi
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d node-a node-c >/dev/null
wait_for_service node-a
wait_for_service node-c

ADMIN_NPUB="$(read_npub node-a)"
REQUESTER_NPUB="$(read_npub node-c)"

if [[ -z "$ADMIN_NPUB" || -z "$REQUESTER_NPUB" ]]; then
  echo "bootstrap-discovery docker e2e failed: unable to resolve npubs" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set --participant "$ADMIN_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-c nvpn set --participant "$REQUESTER_NPUB" >/dev/null

# Admin: accept join requests, advertise its endpoint, relay discovery OFF.
"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name "macos-admin" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --join-requests-enabled true \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false >/dev/null

INVITE="$("${COMPOSE[@]}" exec -T node-a nvpn create-invite | tr -d '\r')"
if [[ -z "$INVITE" ]]; then
  echo "bootstrap-discovery docker e2e failed: admin did not create an invite" >&2
  exit 1
fi

start_daemon_no_relays node-a

# Requester: relay discovery OFF, reach the admin only through a direct static
# FIPS endpoint hint (stands in for a bootstrap transit address).
"${COMPOSE[@]}" exec -T node-c nvpn import-invite "$INVITE" >/dev/null
"${COMPOSE[@]}" exec -T node-c nvpn set \
  --node-name "$REQUESTER_NAME" \
  --endpoint "10.203.0.12:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$ADMIN_NPUB=10.203.0.10:51820" >/dev/null
start_daemon_no_relays node-c

wait_for_inbound_join_request node-a "$REQUESTER_NPUB" "$REQUESTER_NAME"
assert_no_relay_streaming node-a
assert_no_relay_streaming node-c

# Exercise the same app-core action used by the Linux/macOS/Windows Accept
# button. The desktop-shell regression tests drive that action through each
# native GUI's deep-link boundary; this deterministic network lane proves the
# accepted mobile node and desktop then see and can reach one another.
accept_join_request_through_gui_api node-a "$REQUESTER_NPUB"
wait_for_roster_member node-a "$REQUESTER_NPUB" "GUI acceptance did not persist the phone in the admin roster"
"${COMPOSE[@]}" exec -T node-a nvpn reload >/dev/null

wait_for_roster_member node-c "$REQUESTER_NPUB" "phone never applied the accepted roster"
wait_for_connected_peer node-a "desktop never reported the accepted phone online"
wait_for_connected_peer node-c "phone never reported the desktop online"

ADMIN_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
REQUESTER_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-c nvpn ip | tr -d '\r')"
if ! ping_until_success node-a "$REQUESTER_TUNNEL_IP"; then
  echo "bootstrap-discovery docker e2e failed: desktop could not reach the accepted phone" >&2
  exit 1
fi
if ! ping_until_success node-c "$ADMIN_TUNNEL_IP"; then
  echo "bootstrap-discovery docker e2e failed: accepted phone could not reach the desktop" >&2
  exit 1
fi

echo "join request from $REQUESTER_NAME was accepted through the GUI action; desktop and phone reported each other online and passed reciprocal tunnel pings"
