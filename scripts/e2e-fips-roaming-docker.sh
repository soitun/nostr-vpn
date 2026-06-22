#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-fips-roaming"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-fips-roaming"
CONFIG_PATH="/root/.config/nvpn/config.toml"
UDP_PORT=45454
SAFE_TUNNEL_MTU=1150
PING_PAYLOAD_SIZE=1000
UNDERLAY_PREFIX="${NVPN_E2E_UNDERLAY_PREFIX:-10.203.0}"
export NVPN_E2E_UNDERLAY_SUBNET="${NVPN_E2E_UNDERLAY_SUBNET:-$UNDERLAY_PREFIX.0/24}"
export NVPN_E2E_NODE_A_UNDERLAY_IP="${NVPN_E2E_NODE_A_UNDERLAY_IP:-$UNDERLAY_PREFIX.10}"
export NVPN_E2E_NODE_B_UNDERLAY_IP="${NVPN_E2E_NODE_B_UNDERLAY_IP:-$UNDERLAY_PREFIX.11}"
export NVPN_E2E_NODE_C_UNDERLAY_IP="${NVPN_E2E_NODE_C_UNDERLAY_IP:-$UNDERLAY_PREFIX.12}"
# Normal link-dead detection is 30s. Leave enough room for Docker scheduling,
# status polling, and route-cache handoff before declaring fallback broken.
FALLBACK_DEADLINE_SECS="${NVPN_E2E_ROAMING_FALLBACK_SECS:-60}"
DIRECT_RECOVERY_DEADLINE_SECS="${NVPN_E2E_DIRECT_RECOVERY_SECS:-25}"
FALLBACK_HOLD_SECS="${NVPN_E2E_ROAMING_FALLBACK_HOLD_SECS:-12}"
PAYLOAD_PROBE_INTERVAL_SECS="${NVPN_E2E_ROAMING_PAYLOAD_PROBE_INTERVAL_SECS:-1}"
PAYLOAD_RECOVERY_DEADLINE_SECS="${NVPN_E2E_ROAMING_PAYLOAD_RECOVERY_SECS:-10}"
LOCAL_ROUTE_HANDSHAKE_FAILURE_MAX="${NVPN_E2E_LOCAL_ROUTE_HANDSHAKE_FAILURE_MAX:-24}"
FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}"

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
  echo "fips roaming e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a node-b node-c; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 160 "$service" || true
  done
  for node in node-a node-b node-c; do
    echo "--- $node status ---"
    "${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 || true
    echo "--- $node daemon.state.json ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "cat /root/.config/nvpn/daemon.state.json 2>/dev/null || true" || true
    echo "--- $node daemon.log ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "tail -n 320 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
    echo "--- $node routes ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "ip route || true" || true
    echo "--- $node utun100 ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "ip addr show utun100 || true" || true
    echo "--- $node iptables ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "iptables -S || true" || true
  done
}

on_exit() {
  local exit_code=$?
  if [[ $exit_code -ne 0 ]]; then
    dump_debug
    if [[ "${NVPN_E2E_KEEP_CONTAINERS_ON_FAIL:-}" == "1" ]]; then
      echo "NVPN_E2E_KEEP_CONTAINERS_ON_FAIL=1, leaving docker containers running for inspection" >&2
      exit "$exit_code"
    fi
  fi
  cleanup
  exit "$exit_code"
}
trap on_exit EXIT

compact_json() {
  tr -d '\n\r\t '
}

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

  echo "fips roaming e2e failed: service '$service' did not reach running state" >&2
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
    ' '$CONFIG_PATH'
  " | tr -d '\r\"'
}

replace_peer_aliases() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- \
    "$CONFIG_PATH" "$ALICE_NPUB" "$BOB_NPUB" "$CHARLIE_NPUB" <<'SH'
set -eu
config="$1"
alice="$2"
bob="$3"
charlie="$4"
tmp="$(mktemp)"
awk '
  /^\[peer_aliases\]$/ { skip = 1; next }
  /^\[/ { skip = 0 }
  !skip { print }
' "$config" > "$tmp"
{
  cat "$tmp"
  printf '\n[peer_aliases]\n'
  printf '"%s" = "alice"\n' "$alice"
  printf '"%s" = "bob"\n' "$bob"
  printf '"%s" = "charlie"\n' "$charlie"
} > "$config"
rm -f "$tmp"
SH
}

disable_nat_discovery() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc '
    config=/root/.config/nvpn/config.toml
    tmp="$(mktemp)"
    awk "
      /^\\[nat\\]$/ { in_nat = 1; print; next }
      /^\\[/ { in_nat = 0 }
      in_nat && /^enabled[[:space:]]*=/ { print \"enabled = false\"; next }
      { print }
    " "$config" > "$tmp"
    mv "$tmp" "$config"
  '
}

start_nvpn_daemon() {
  local node="$1"
  "${COMPOSE[@]}" exec -T "$node" env \
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="$FIPS_NOSTR_DISCOVERY_POLICY" \
    NVPN_MESH_MTU_PROFILE=safe \
    NVPN_MESH_UNDERLAY_UDP_MTU=1280 \
    NVPN_MESH_TUNNEL_MTU="$SAFE_TUNNEL_MTU" \
    nvpn start --daemon --connect >/dev/null
}

block_direct_alice_bob_udp() {
  "${COMPOSE[@]}" exec -T node-a sh -s -- "$NVPN_E2E_NODE_B_UNDERLAY_IP" <<'SH'
set -eu
peer_ip="$1"
iptables -C OUTPUT -p udp -d "$peer_ip" -j DROP 2>/dev/null || iptables -I OUTPUT -p udp -d "$peer_ip" -j DROP
iptables -C INPUT -p udp -s "$peer_ip" -j DROP 2>/dev/null || iptables -I INPUT -p udp -s "$peer_ip" -j DROP
SH
  "${COMPOSE[@]}" exec -T node-b sh -s -- "$NVPN_E2E_NODE_A_UNDERLAY_IP" <<'SH'
set -eu
peer_ip="$1"
iptables -C OUTPUT -p udp -d "$peer_ip" -j DROP 2>/dev/null || iptables -I OUTPUT -p udp -d "$peer_ip" -j DROP
iptables -C INPUT -p udp -s "$peer_ip" -j DROP 2>/dev/null || iptables -I INPUT -p udp -s "$peer_ip" -j DROP
SH
}

unblock_direct_alice_bob_udp() {
  "${COMPOSE[@]}" exec -T node-a sh -s -- "$NVPN_E2E_NODE_B_UNDERLAY_IP" <<'SH'
set -eu
peer_ip="$1"
while iptables -D OUTPUT -p udp -d "$peer_ip" -j DROP 2>/dev/null; do :; done
while iptables -D INPUT -p udp -s "$peer_ip" -j DROP 2>/dev/null; do :; done
SH
  "${COMPOSE[@]}" exec -T node-b sh -s -- "$NVPN_E2E_NODE_A_UNDERLAY_IP" <<'SH'
set -eu
peer_ip="$1"
while iptables -D OUTPUT -p udp -d "$peer_ip" -j DROP 2>/dev/null; do :; done
while iptables -D INPUT -p udp -s "$peer_ip" -j DROP 2>/dev/null; do :; done
SH
}

make_direct_alice_bob_unreachable() {
  "${COMPOSE[@]}" exec -T node-a sh -s -- "$NVPN_E2E_NODE_B_UNDERLAY_IP" <<'SH'
set -eu
peer_ip="$1"
ip route replace unreachable "$peer_ip/32" metric 1
ip route flush cache 2>/dev/null || true
SH
  "${COMPOSE[@]}" exec -T node-b sh -s -- "$NVPN_E2E_NODE_A_UNDERLAY_IP" <<'SH'
set -eu
peer_ip="$1"
ip route replace unreachable "$peer_ip/32" metric 1
ip route flush cache 2>/dev/null || true
SH
}

restore_direct_alice_bob_route() {
  "${COMPOSE[@]}" exec -T node-a sh -s -- "$NVPN_E2E_NODE_B_UNDERLAY_IP" <<'SH'
set -eu
peer_ip="$1"
ip route del unreachable "$peer_ip/32" 2>/dev/null || ip route del "$peer_ip/32" 2>/dev/null || true
ip route flush cache 2>/dev/null || true
SH
  "${COMPOSE[@]}" exec -T node-b sh -s -- "$NVPN_E2E_NODE_A_UNDERLAY_IP" <<'SH'
set -eu
peer_ip="$1"
ip route del unreachable "$peer_ip/32" 2>/dev/null || ip route del "$peer_ip/32" 2>/dev/null || true
ip route flush cache 2>/dev/null || true
SH
}

wait_for_mesh() {
  local node="$1"
  local expected="$2"
  local status compact
  for _ in $(seq 1 100); do
    status="$("${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 | tr -d '\r')"
    compact="$(printf '%s' "$status" | compact_json)"
    if grep -q '"status_source":"daemon"' <<<"$compact" \
      && grep -q '"running":true' <<<"$compact" \
      && grep -q '"mesh_ready":true' <<<"$compact" \
      && grep -q "\"connected_peer_count\":$expected" <<<"$compact"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done

  printf '%s\n' "$status"
  return 1
}

status_json() {
  local node="$1"
  "${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 | tr -d '\r'
}

peer_matches_direct_addr() {
  local status="$1"
  local peer_key="$2"
  local direct_addr="$3"
  jq -e --arg peer_key "$peer_key" --arg direct_addr "$direct_addr" '
    .daemon.state.peers
    | any(
      (.participant_pubkey == $peer_key or .fips_endpoint_npub == $peer_key)
      and .reachable == true
      and ((.runtime_endpoint? // "") != "fips")
      and (
        ((.runtime_endpoint? // "") | contains($direct_addr))
        or ((.fips_transport_addr? // "") | contains($direct_addr))
      )
    )
  ' >/dev/null <<<"$status"
}

peer_matches_fallback_with_probe() {
  local status="$1"
  local peer_key="$2"
  local _direct_addr="$3"
  jq -e --arg peer_key "$peer_key" '
    .daemon.state.peers
    | any(
      (.participant_pubkey == $peer_key or .fips_endpoint_npub == $peer_key)
      and .reachable == true
      and (.direct_probe_pending == true or (.direct_probe_after_ms? != null))
      and ((.last_fips_data_seen_at? // 0) > 0)
    )
  ' >/dev/null <<<"$status"
}

wait_for_direct_peer() {
  local node="$1"
  local peer_key="$2"
  local direct_addr="$3"
  local label="$4"
  local deadline="$5"
  local status=""
  local end=$(( $(date +%s) + deadline ))
  while [[ "$(date +%s)" -le "$end" ]]; do
    status="$(status_json "$node")"
    if peer_matches_direct_addr "$status" "$peer_key" "$direct_addr"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done

  echo "fips roaming e2e failed: $label did not use direct UDP $direct_addr within ${deadline}s" >&2
  printf '%s\n' "$status" >&2
  exit 1
}

wait_for_fallback_probe_peer() {
  local node="$1"
  local peer_key="$2"
  local direct_addr="$3"
  local label="$4"
  local deadline="$5"
  local status=""
  local end=$(( $(date +%s) + deadline ))
  while [[ "$(date +%s)" -le "$end" ]]; do
    status="$(status_json "$node")"
    if peer_matches_fallback_with_probe "$status" "$peer_key" "$direct_addr"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done

  echo "fips roaming e2e failed: $label did not show fallback traffic with direct probing within ${deadline}s" >&2
  printf '%s\n' "$status" >&2
  exit 1
}

start_payload_probe() {
  local node="$1"
  local target_ip="$2"
  local output="$3"
  "${COMPOSE[@]}" exec -T "$node" sh -s -- \
    "$target_ip" "$PING_PAYLOAD_SIZE" "$PAYLOAD_PROBE_INTERVAL_SECS" "$output" <<'SH'
set -eu
target_ip="$1"
payload_size="$2"
interval="$3"
output="$4"
rm -f "$output" "${output}.pid"
(
  while :; do
    ts="$(date +%s)"
    if ping -M do -s "$payload_size" -c 1 -W 1 "$target_ip" >/dev/null 2>&1; then
      printf '%s ok\n' "$ts"
    else
      printf '%s fail\n' "$ts"
    fi
    sleep "$interval"
  done
) >"$output" 2>&1 &
printf '%s\n' "$!" >"${output}.pid"
SH
}

stop_payload_probe() {
  local node="$1"
  local output="$2"
  "${COMPOSE[@]}" exec -T "$node" sh -s -- "$output" <<'SH'
set -eu
output="$1"
if [ -s "${output}.pid" ]; then
  kill "$(cat "${output}.pid")" 2>/dev/null || true
  rm -f "${output}.pid"
fi
cat "$output" 2>/dev/null || true
SH
}

assert_payload_probe_success_since() {
  local node="$1"
  local output="$2"
  local since="$3"
  local label="$4"
  local end=$(( $(date +%s) + PAYLOAD_RECOVERY_DEADLINE_SECS ))
  while [[ "$(date +%s)" -le "$end" ]]; do
    if "${COMPOSE[@]}" exec -T "$node" sh -s -- "$output" "$since" <<'SH'
set -eu
output="$1"
since="$2"
awk -v since="$since" '$1 >= since && $2 == "ok" { found = 1 } END { exit found ? 0 : 1 }' "$output"
SH
    then
      return 0
    fi
    sleep "$PAYLOAD_PROBE_INTERVAL_SECS"
  done

  echo "fips roaming e2e failed: $label did not recover tunnel payload after churn within ${PAYLOAD_RECOVERY_DEADLINE_SECS}s" >&2
  "${COMPOSE[@]}" exec -T "$node" sh -lc "cat '$output' 2>/dev/null || true" >&2 || true
  exit 1
}

mark_daemon_log() {
  local node="$1"
  local marker="$2"
  "${COMPOSE[@]}" exec -T "$node" sh -s -- "$marker" <<'SH'
set -eu
marker="$1"
printf 'NVPN_E2E_MARKER %s\n' "$marker" >> /root/.config/nvpn/daemon.log
SH
}

assert_local_route_handshake_failures_bounded() {
  local node="$1"
  local marker="$2"
  local label="$3"
  local count
  count="$("${COMPOSE[@]}" exec -T "$node" sh -s -- "$marker" <<'SH'
set -eu
marker="$1"
awk -v marker="$marker" '
  $0 ~ "NVPN_E2E_MARKER " marker { seen = 1; next }
  seen && /Failed to send handshake message/ && /(Network is unreachable|No route to host|Host is unreachable|os error 51|os error 65)/ {
    count++
  }
  END { print count + 0 }
' /root/.config/nvpn/daemon.log
SH
  )"
  count="${count//$'\r'/}"
  if (( count > LOCAL_ROUTE_HANDSHAKE_FAILURE_MAX )); then
    echo "fips roaming e2e failed: $label saw $count local-route handshake failures after churn marker, expected <= $LOCAL_ROUTE_HANDSHAKE_FAILURE_MAX" >&2
    "${COMPOSE[@]}" exec -T "$node" sh -s -- "$marker" <<'SH' >&2 || true
set -eu
marker="$1"
awk -v marker="$marker" '
  $0 ~ "NVPN_E2E_MARKER " marker { seen = 1; print; next }
  seen && (/Failed to send handshake message/ || /Local route unavailable/ || /request.*stale|stale.*advert/) { print }
' /root/.config/nvpn/daemon.log | tail -n 120
SH
    exit 1
  fi
}

resolve_magic_dns() {
  local node="$1"
  local name="$2"
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "dig +short @127.0.0.1 -p 1053 '$name' A | tail -n1" | tr -d '\r'
}

assert_tunnel_mtu() {
  local node="$1"
  local actual
  actual="$("${COMPOSE[@]}" exec -T "$node" sh -lc \
    "ip -o link show utun100 | awk -F ' mtu ' '{ print \$2 }' | awk '{ print \$1 }'" | tr -d '\r')"
  if [[ "$actual" != "$SAFE_TUNNEL_MTU" ]]; then
    echo "fips roaming e2e failed: $node utun100 MTU was '$actual', expected '$SAFE_TUNNEL_MTU'" >&2
    exit 1
  fi
}

ping_tunnel_payload() {
  local node="$1"
  local target_ip="$2"
  local log_path="$3"
  for _ in $(seq 1 10); do
    if "${COMPOSE[@]}" exec -T "$node" ping -M do -s "$PING_PAYLOAD_SIZE" -c 2 -W 2 "$target_ip" >"$log_path"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

assert_ping_tunnel() {
  local node="$1"
  local target_ip="$2"
  local label="$3"
  local log_path="$4"
  if ! ping_tunnel_payload "$node" "$target_ip" "$log_path"; then
    echo "fips roaming e2e failed: $label could not move $PING_PAYLOAD_SIZE-byte no-fragment ping payload" >&2
    cat "$log_path" >&2 2>/dev/null || true
    exit 1
  fi
}

run_udp_roundtrip() {
  local source_node="$1"
  local target_node="$2"
  local target_ip="$3"
  local payload="$4"
  local output="$5"
  "${COMPOSE[@]}" exec -T "$target_node" sh -lc \
    "rm -f '$output'; timeout 20 nc -u -l -p '$UDP_PORT' > '$output' & echo \$! > '${output}.pid'"
  "${COMPOSE[@]}" exec -T "$source_node" sh -lc \
    "printf '%s' '$payload' | nc -u -w 2 '$target_ip' '$UDP_PORT'"
  for _ in $(seq 1 20); do
    if "${COMPOSE[@]}" exec -T "$target_node" sh -lc "grep -q '$payload' '$output' 2>/dev/null"; then
      return 0
    fi
    sleep 1
  done
  echo "fips roaming e2e failed: UDP payload '$payload' did not arrive at $target_node" >&2
  exit 1
}

assert_no_transit_roster_peer() {
  local status="$1"
  local peer_key="$2"
  local label="$3"
  if jq -e --arg peer_key "$peer_key" '
    .daemon.state.peers
    | any(.participant_pubkey == $peer_key or .fips_endpoint_npub == $peer_key)
  ' >/dev/null <<<"$status"; then
    echo "fips roaming e2e failed: $label exposed transit hop as a data-plane peer" >&2
    printf '%s\n' "$status" >&2
    exit 1
  fi
}

run_roam_flap() {
  local flap_name="$1"
  local flap_mode="${2:-drop}"
  local alice_direct_addr="$NVPN_E2E_NODE_B_UNDERLAY_IP:51820"
  local bob_direct_addr="$NVPN_E2E_NODE_A_UNDERLAY_IP:51820"
  local alice_probe="/tmp/${flap_name}-alice-payload-probe.log"
  local bob_probe="/tmp/${flap_name}-bob-payload-probe.log"
  local alice_marker="${flap_name}-alice-$(date +%s)"
  local bob_marker="${flap_name}-bob-$(date +%s)"
  local churn_started

  echo "--- $flap_name: churn direct Alice<->Bob path ($flap_mode), expect FIPS fallback plus direct probe ---"
  mark_daemon_log node-a "$alice_marker"
  mark_daemon_log node-b "$bob_marker"
  start_payload_probe node-a "$BOB_TUNNEL_IP" "$alice_probe"
  start_payload_probe node-b "$ALICE_TUNNEL_IP" "$bob_probe"
  churn_started="$(date +%s)"
  case "$flap_mode" in
    drop)
      block_direct_alice_bob_udp
      ;;
    route-unreachable)
      make_direct_alice_bob_unreachable
      ;;
    *)
      echo "fips roaming e2e failed: unsupported flap mode '$flap_mode'" >&2
      exit 2
      ;;
  esac

  local alice_fallback bob_fallback
  alice_fallback="$(wait_for_fallback_probe_peer node-a "$BOB_NPUB" "$alice_direct_addr" "alice during $flap_name" "$FALLBACK_DEADLINE_SECS")"
  bob_fallback="$(wait_for_fallback_probe_peer node-b "$ALICE_NPUB" "$bob_direct_addr" "bob during $flap_name" "$FALLBACK_DEADLINE_SECS")"
  assert_payload_probe_success_since node-a "$alice_probe" "$churn_started" "alice continuous payload during $flap_name"
  assert_payload_probe_success_since node-b "$bob_probe" "$churn_started" "bob continuous payload during $flap_name"
  assert_local_route_handshake_failures_bounded node-a "$alice_marker" "alice during $flap_name"
  assert_local_route_handshake_failures_bounded node-b "$bob_marker" "bob during $flap_name"

  assert_ping_tunnel node-a "$BOB_TUNNEL_IP" "alice-to-bob during $flap_name fallback" "/tmp/${flap_name}-alice-to-bob-fallback-ping.log"
  assert_ping_tunnel node-b "$ALICE_TUNNEL_IP" "bob-to-alice during $flap_name fallback" "/tmp/${flap_name}-bob-to-alice-fallback-ping.log"
  sleep "$FALLBACK_HOLD_SECS"

  alice_fallback="$(wait_for_fallback_probe_peer node-a "$BOB_NPUB" "$alice_direct_addr" "alice after $flap_name hold" "$FALLBACK_DEADLINE_SECS")"
  bob_fallback="$(wait_for_fallback_probe_peer node-b "$ALICE_NPUB" "$bob_direct_addr" "bob after $flap_name hold" "$FALLBACK_DEADLINE_SECS")"

  echo "--- $flap_name: restore LAN/direct path, expect quick upgrade away from fallback ---"
  local restore_started
  restore_started="$(date +%s)"
  case "$flap_mode" in
    drop)
      unblock_direct_alice_bob_udp
      ;;
    route-unreachable)
      restore_direct_alice_bob_route
      ;;
  esac

  local alice_direct bob_direct
  alice_direct="$(wait_for_direct_peer node-a "$BOB_NPUB" "$alice_direct_addr" "alice after $flap_name restore" "$DIRECT_RECOVERY_DEADLINE_SECS")"
  bob_direct="$(wait_for_direct_peer node-b "$ALICE_NPUB" "$bob_direct_addr" "bob after $flap_name restore" "$DIRECT_RECOVERY_DEADLINE_SECS")"
  assert_payload_probe_success_since node-a "$alice_probe" "$restore_started" "alice continuous payload after $flap_name restore"
  assert_payload_probe_success_since node-b "$bob_probe" "$restore_started" "bob continuous payload after $flap_name restore"
  local alice_probe_log bob_probe_log
  alice_probe_log="$(stop_payload_probe node-a "$alice_probe")"
  bob_probe_log="$(stop_payload_probe node-b "$bob_probe")"

  assert_ping_tunnel node-a "$BOB_TUNNEL_IP" "alice-to-bob after $flap_name direct restore" "/tmp/${flap_name}-alice-to-bob-direct-ping.log"
  assert_ping_tunnel node-b "$ALICE_TUNNEL_IP" "bob-to-alice after $flap_name direct restore" "/tmp/${flap_name}-bob-to-alice-direct-ping.log"

  echo "--- $flap_name fallback status: alice ---"
  echo "$alice_fallback"
  echo "--- $flap_name fallback status: bob ---"
  echo "$bob_fallback"
  echo "--- $flap_name restored direct status: alice ---"
  echo "$alice_direct"
  echo "--- $flap_name restored direct status: bob ---"
  echo "$bob_direct"
  echo "--- $flap_name continuous payload probe: alice ---"
  echo "$alice_probe_log"
  echo "--- $flap_name continuous payload probe: bob ---"
  echo "$bob_probe_log"
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d node-a node-b node-c >/dev/null
for service in node-a node-b node-c; do
  wait_for_service "$service"
done

for node in node-a node-b node-c; do
  "${COMPOSE[@]}" exec -T "$node" nvpn init --force >/dev/null
done

ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"
CHARLIE_NPUB="$(nostr_pubkey_from_config node-c)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" || -z "$CHARLIE_NPUB" ]]; then
  echo "fips roaming e2e failed: unable to resolve node npubs from config" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set --participant "$ALICE_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn set --participant "$BOB_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-c nvpn set --participant "$CHARLIE_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name alice \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --fips-peer-endpoint "$BOB_NPUB=$NVPN_E2E_NODE_B_UNDERLAY_IP:51820" \
  --fips-peer-endpoint "$CHARLIE_NPUB=$NVPN_E2E_NODE_C_UNDERLAY_IP:51820" \
  --endpoint "$NVPN_E2E_NODE_A_UNDERLAY_IP:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name bob \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --fips-peer-endpoint "$ALICE_NPUB=$NVPN_E2E_NODE_A_UNDERLAY_IP:51820" \
  --fips-peer-endpoint "$CHARLIE_NPUB=$NVPN_E2E_NODE_C_UNDERLAY_IP:51820" \
  --endpoint "$NVPN_E2E_NODE_B_UNDERLAY_IP:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false >/dev/null

"${COMPOSE[@]}" exec -T node-c nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name charlie \
  --fips-peer-endpoint "$ALICE_NPUB=$NVPN_E2E_NODE_A_UNDERLAY_IP:51820" \
  --fips-peer-endpoint "$BOB_NPUB=$NVPN_E2E_NODE_B_UNDERLAY_IP:51820" \
  --endpoint "$NVPN_E2E_NODE_C_UNDERLAY_IP:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false >/dev/null

for node in node-a node-b node-c; do
  replace_peer_aliases "$node"
  disable_nat_discovery "$node"
  start_nvpn_daemon "$node"
done

ALICE_STATUS="$(wait_for_mesh node-a 1)" || {
  echo "fips roaming e2e failed: alice mesh did not reach 1/1" >&2
  echo "$ALICE_STATUS"
  exit 1
}
BOB_STATUS="$(wait_for_mesh node-b 1)" || {
  echo "fips roaming e2e failed: bob mesh did not reach 1/1" >&2
  echo "$BOB_STATUS"
  exit 1
}

assert_no_transit_roster_peer "$ALICE_STATUS" "$CHARLIE_NPUB" "alice"
assert_no_transit_roster_peer "$BOB_STATUS" "$CHARLIE_NPUB" "bob"

ALICE_DIRECT="$(wait_for_direct_peer node-a "$BOB_NPUB" "$NVPN_E2E_NODE_B_UNDERLAY_IP:51820" "alice initial LAN" "$DIRECT_RECOVERY_DEADLINE_SECS")"
BOB_DIRECT="$(wait_for_direct_peer node-b "$ALICE_NPUB" "$NVPN_E2E_NODE_A_UNDERLAY_IP:51820" "bob initial LAN" "$DIRECT_RECOVERY_DEADLINE_SECS")"

BOB_TUNNEL_IP="$(resolve_magic_dns node-a bob.nvpn)"
ALICE_TUNNEL_IP="$(resolve_magic_dns node-b alice.nvpn)"

if [[ -z "$BOB_TUNNEL_IP" || -z "$ALICE_TUNNEL_IP" ]]; then
  echo "fips roaming e2e failed: magic dns did not resolve alice.nvpn/bob.nvpn" >&2
  exit 1
fi

assert_tunnel_mtu node-a
assert_tunnel_mtu node-b
assert_ping_tunnel node-a "$BOB_TUNNEL_IP" "initial alice-to-bob direct LAN" /tmp/initial-alice-to-bob-ping.log
assert_ping_tunnel node-b "$ALICE_TUNNEL_IP" "initial bob-to-alice direct LAN" /tmp/initial-bob-to-alice-ping.log
run_udp_roundtrip node-a node-b "$BOB_TUNNEL_IP" "alice-to-bob-roaming-initial" /tmp/bob-roaming-initial-udp.out

run_roam_flap "mobile-flap-1"
run_roam_flap "mobile-flap-2"
run_roam_flap "route-unreachable-flap" "route-unreachable"

echo "--- Initial direct status: alice ---"
echo "$ALICE_DIRECT"
echo "--- Initial direct status: bob ---"
echo "$BOB_DIRECT"
echo "--- Magic DNS ---"
echo "node-a bob.nvpn -> $BOB_TUNNEL_IP"
echo "node-b alice.nvpn -> $ALICE_TUNNEL_IP"
echo "--- Initial pings ---"
cat /tmp/initial-alice-to-bob-ping.log
cat /tmp/initial-bob-to-alice-ping.log
echo "--- Initial UDP payload ---"
"${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/bob-roaming-initial-udp.out'

echo "fips roaming docker e2e passed: direct LAN path established, mobile/WiFi-style direct drops and a route-unreachable flap used FIPS fallback while direct probing stayed pending, continuous payload recovered during churn, and each restore upgraded back to direct within ${DIRECT_RECOVERY_DEADLINE_SECS}s"
