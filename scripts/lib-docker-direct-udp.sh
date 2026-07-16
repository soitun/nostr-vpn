#!/usr/bin/env bash

# Shared by Docker product and performance gates. Callers provide the COMPOSE
# command array and a unique DIRECT_COUNTER_COMMENT.
install_direct_underlay_counter() {
  local service="$1"
  local peer_ip="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- "$peer_ip" "$DIRECT_COUNTER_COMMENT" <<'SH'
set -eu
peer_ip="$1"
comment="$2"
while iptables -D OUTPUT -p udp -d "$peer_ip" --dport 51820 -m comment --comment "$comment" 2>/dev/null; do :; done
iptables -I OUTPUT 1 -p udp -d "$peer_ip" --dport 51820 -m comment --comment "$comment"
SH
}

direct_underlay_bytes() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- "$DIRECT_COUNTER_COMMENT" <<'SH' | tr -d '\r'
set -eu
comment="$1"
iptables-save -c 2>/dev/null | awk -v comment="$comment" '
  index($0, comment) {
    gsub(/^\[/, "", $1)
    gsub(/\]$/, "", $1)
    split($1, counters, ":")
    print counters[2]
    found = 1
    exit
  }
  END { if (!found) print 0 }
'
SH
}
