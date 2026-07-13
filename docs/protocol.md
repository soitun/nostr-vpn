# Protocol

This document describes the protocol shape that `nostr-vpn` currently implements after the FIPS-only private mesh cleanup.

The `README.md` stays product-facing. Protocol details live here so they can track the code more closely.

## Scope

`nostr-vpn` is split into three layers:

- Out-of-band bootstrapping with invite payloads and QR codes
- Network membership and admin roster state
- A FIPS-backed private mesh data plane for tunnel traffic

Only the active network participates in the live runtime. `nostr-vpn` no longer runs its legacy relay-announced peer roster or WireGuard mesh mode.

## Identities And Stable IDs

| Name | Purpose | Format |
| --- | --- | --- |
| Nostr identity keypair | Authenticates device identity, invites, admin actions, and FIPS discovery identity | `nsec`/`npub` at the edges, normalized to hex internally |
| `network_id` | Stable mesh identifier used for roster scope and tunnel-IP derivation | String |

Important details:

- `network_id` is normalized at runtime by stripping the legacy `nostr-vpn:` prefix if present.
- Each configured network carries its own participant and admin set.
- If tunnel IP auto-configuration is enabled, the local node derives its `/32` as:
  - `SHA256(network_id + "\n" + own_nostr_pubkey_hex)`
  - `10.44.(digest[0] % 254 + 1).(digest[1] % 254 + 1)/32`

## Invites

Network invites are `nvpn://invite` payloads. They carry enough information for another device to join or request access:

- network name and stable `network_id`
- inviter identity
- optional admin and participant metadata
- optional relay list for the discovery layer

Relay URLs in invites configure the discovery/rendezvous layer used by FIPS. They are not a nostr-vpn relay roster, and importing an invite does not enable the removed legacy peer-announcement protocol.

## Admin Roster Sync

Network membership is represented as an admin-signed roster. Operationally, the roster is the authority for:

- network name
- participants
- admins
- join-request settings
- per-peer aliases and route settings

When a newer valid roster arrives, peers reload the active network membership and keep the FIPS mesh runtime aligned with that roster.

## FIPS Private Mesh

`nvpn` uses FIPS as the only private mesh data plane.

The CLI builds FIPS peer configuration from the active network participants:

- each participant is mapped to its derived tunnel address
- advertised routes are included in peer allowed IPs
- static FIPS peer endpoints may be supplied with `fips_peer_endpoints`
- FIPS discovery may use configured Nostr relays internally

The daemon reports each peer with FIPS-specific state:

- `endpoint` and `runtime_endpoint` are `"fips"` or a FIPS transport address
- `fips_endpoint_npub`, `fips_transport_addr`, and `fips_transport_type` describe the selected FIPS link
- `last_mesh_seen_at` and `last_fips_seen_at` replace the legacy presence/signal timestamps
- packet and byte counters are read from FIPS link state

`nostr-vpn` should not publish or consume its old Nostr relay peer announcements. If FIPS uses relays, that behavior belongs to FIPS discovery/rendezvous, not to a separate nostr-vpn signaling model.

## NAT Discovery

NAT discovery remains a local endpoint aid:

- STUN servers can discover a public UDP endpoint
- discovery is bound to the active listen port when possible
- port-mapping state is surfaced in diagnostics

The discovered endpoint is input to FIPS configuration. It is not a WireGuard endpoint announcement.

## Routing

Route targets come from the active network roster and local node settings. FIPS receives the route map and carries private mesh traffic for the selected peers.

Exit-node behavior is represented in config and UI state. A node can optionally use a local WireGuard upstream while offering FIPS exit-node service. The provider routes its own default internet traffic and forwarded member exit traffic through that upstream, while peers still see only the normal FIPS exit-node route advertisement. WireGuard is not a mesh data plane and nostr-vpn does not announce or signal WireGuard peers through its old relay model.

## Exit DNS privacy

When a default exit route is active, the platform resolver points only at
nostr-vpn's local DNS stub. MagicDNS names are answered locally. When the active
exit is a configured WireGuard profile with DNS IP addresses, the stub forwards
public DNS wire messages to those resolvers through the WireGuard data path.
Those resolvers become active only after the WireGuard runtime is active and
are removed before that exit is torn down; there is no underlay DNS fallback.

For every other exit source, public queries are sent as DNS wire messages over
authenticated HTTPS to Cloudflare's DoH service, using fixed bootstrap
addresses so resolving the DoH hostname cannot itself leak through plaintext
DNS. Plain HTTP, redirects, system proxies, and plaintext DNS fallback are
disabled; an unavailable or invalid response fails closed.

With DoH, this prevents the exit operator from reading or spoofing DNS questions
and answers. With a WireGuard profile resolver, the WireGuard provider can
process those questions and answers, matching the profile's explicit DNS
policy, while the underlay cannot read them outside the encrypted tunnel. This
is not anonymity: the selected DNS provider can process the question, and the
exit can still observe destination IP addresses, traffic timing and volume, and
possibly TLS hostnames when the destination connection does not use ECH.

Selected FIPS exit peers are also treated as hostile inbound networks. The
buyer admits TCP, UDP, and echo replies only for locally originated flows, plus
ICMP errors that quote a tracked flow. Unsolicited connections, malformed or
fragmented packets, and packets with private, loopback, link-local, multicast,
or spoofed mesh sources are dropped before reaching the TUN. This state
survives route-table refreshes and applies equally to IPv4 and IPv6.

## Canonical Source

If this document and the code diverge, the code wins. The main protocol implementations currently live in:

- `crates/nostr-vpn-core/src/fips_control.rs`
- `crates/nostr-vpn-core/src/fips_mesh.rs`
- `crates/nostr-vpn-core/src/join_requests.rs`
- `crates/nostr-vpn-core/src/config.rs`
- `crates/nostr-vpn-cli/src/fips_private_mesh.rs`
- `crates/nostr-vpn-cli/src/session_runtime.rs`
- `crates/nostr-vpn-app-core/src/ffi.rs`
