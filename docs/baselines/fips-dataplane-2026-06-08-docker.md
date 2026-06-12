# FIPS Dataplane Baseline - 2026-06-08 Docker VM

Baseline for `codex/dataplane-safety-net` before any dataplane architecture
rewrite.

## Environment

- Commit: `ff8a5a30 Add FIPS dataplane safety net`
- Host: local Darwin arm64 host
- Docker server: `29.5.2`
- Target path: Docker Linux VM, two containers on `docker-compose.e2e.yml`
- Local FIPS worktree placeholder: `FIPS_SAFETY_WORKTREE=/path/to/fips-dataplane-safety`
- Validation boundary: this is Linux/container coverage from the local host. It is
  not real Mac-to-Mac Wi-Fi/screenshare coverage.

## Perf Gate

Command:

```sh
./scripts/e2e-fips-perf-regression-docker.sh
```

Raw log captured locally as `/tmp/nvpn-fips-perf-baseline-ff8a5a30.log`.
Log SHA-256:
`03c7a5ead07119a938dfb56a53f50165b7b30391f1f4695496e0106bbef881d1`.

The harness passed. It proved the configured direct UDP underlay path advanced
in every phase and the tunnel did not wedge under or after TCP load.

| Phase | Fwd Mbps | Fwd retrans | Rev Mbps | Rev retrans | Fwd-load Mbps | Fwd-load ping loss/avg/max | Rev-load Mbps | Rev-load ping loss/avg/max | Post ping loss/avg/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | ---: | --- | ---: | --- | --- | --- |
| clean-underlay | 2289.2 | 193 | 2222.4 | 287 | 2272.7 | 0% / 1.869 ms / 4.003 ms | 2083.6 | 0% / 2.325 ms / 6.262 ms | 0% / 1.658 ms / 7.991 ms | 7348164740 / 6939523628 |
| constrained-underlay | 166.9 | 790 | 165.2 | 439 | 165.8 | 0% / 63.254 ms / 131.896 ms | 163.9 | 0% / 78.235 ms / 132.536 ms | 0% / 1.571 ms / 2.551 ms | 543359655 / 537997218 |
| worker-queue-pressure | 805.6 | 3061 | 836.8 | 3380 | 820.0 | 0% / 0.325 ms / 0.510 ms | 796.5 | 1.66667% / 0.384 ms / 0.619 ms | 0% / 1.445 ms / 2.486 ms | 2603017000 / 2618046344 |
| rx-maintenance-fault | 2205.6 | 364 | 2242.5 | 196 | 2268.0 | 0% / 1.820 ms / 4.677 ms | 2223.3 | 0% / 2.077 ms / 3.567 ms | 0% / 1.669 ms / 3.276 ms | 7260683163 / 7196856071 |

Baseline tunnel IPs for this run:

- node-a: assigned container tunnel address
- node-b: assigned container tunnel address

## Full Soak

Command:

```sh
NVPN_SOAK_DURATION_SECS=1800 \
NVPN_SOAK_OUTPUT_DIR=artifacts/fips-soak/baseline-ff8a5a30-20260608-30m \
./scripts/soak-fips-dataplane-docker.sh
```

Ignored sample file:
`artifacts/fips-soak/baseline-ff8a5a30-20260608-30m/samples.ndjson`

Sample SHA-256:
`8e27e1f2df0db4697a4fa2b663721d409b9d44009a9ac8c9c363ba399703910e`.

Raw log captured locally as `/tmp/nvpn-fips-soak-baseline-ff8a5a30-30m.log`.
Log SHA-256:
`9aa31b88ded3e741c31379431d7bcd68894f13aac8c388d8fd3b4f1120c06b64`.

The soak passed with 33 samples from `2026-06-08T00:26:28Z` through
`2026-06-08T00:56:19Z`. Every sample stayed on the configured direct UDP path:

- node-a transport: configured container underlay endpoint
- node-b transport: configured container underlay endpoint

Observed ranges:

- FIPS SRTT: node-a `1-2 ms`, node-b `1-2 ms`
- Ping loss: `0%` both ways in all samples
- Ping avg: node-a to node-b `1.155-2.035 ms`, node-b to node-a
  `1.095-2.002 ms`
- Ping max: node-a to node-b `1.908-12.010 ms`, node-b to node-a
  `1.693-6.767 ms`
- Iperf forward: `2109.306-2304.943 Mbps`
- Iperf reverse: `2131.967-2287.862 Mbps`
- Iperf retransmits: forward `91-362`, reverse `53-284`
- Daemon CPU: node-a `45.6-77.2%`, node-b `45.7-84.2%`
- Final FIPS counters:
  - node-a sent/recv bytes: `62648918954` / `61963786051`
  - node-b sent/recv bytes: `62211719519` / `62655238902`

## Platform Matrix Smoke

Short Linux/docker platform-split smoke against the sibling FIPS safety branch.
This is not a replacement for the full perf gate above; it proves the connected
UDP on/off, worker-count, and explicit backpressure knobs still pass the same
direct-path/TCP/ping phases with local FIPS patches applied.

Environment:

- Nostr-vpn parent before adding the matrix hook: `1ca595aa`
- Local FIPS commit: `42fa43b Avoid repeated connected UDP activation scans`
- Durations: `NVPN_PLATFORM_MATRIX_DURATION_SECS=3`,
  `NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=4`,
  `NVPN_PLATFORM_MATRIX_PING_COUNT=10`,
  `NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=75`

Commands:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=connected-udp-off \
NVPN_PLATFORM_MATRIX_DURATION_SECS=3 \
NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=4 \
NVPN_PLATFORM_MATRIX_PING_COUNT=10 \
NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=75 \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/local-fips-42fa43b-connected-udp-off-smoke \
./scripts/e2e-fips-platform-matrix-docker.sh

NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=connected-udp-on,single-encrypt-worker,tight-backpressure \
NVPN_PLATFORM_MATRIX_DURATION_SECS=3 \
NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=4 \
NVPN_PLATFORM_MATRIX_PING_COUNT=10 \
NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=75 \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/local-fips-42fa43b-remaining-smoke \
./scripts/e2e-fips-platform-matrix-docker.sh
```

All scenarios passed. Every phase advanced the configured direct UDP underlay
counter on both nodes and post-load ping recovered to `0%` loss.

| Scenario | Extra env | Log SHA-256 | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker pressure fwd-load/rev-load Mbps | Rx fault fwd-load/rev-load Mbps | Post-load ping max |
| --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| connected-udp-off | `FIPS_CONNECTED_UDP=0` | `89a34e9d5f866649455bc8e68b37b4e5d5c843339205abf7f7aadc9a797b4a81` | 2324.6 / 2337.3 | 167.1 / 160.5 | 550.7 / 501.6 | 2310.2 / 2391.5 | <= 3.619 ms |
| connected-udp-on | `FIPS_CONNECTED_UDP=1` | `923940268209ccba2fb6bfbed370ae07e10ed9d2bf31da112d9764ef757b218c` | 2180.9 / 2263.9 | 166.8 / 163.8 | 477.3 / 473.5 | 2295.2 / 2236.6 | <= 2.428 ms |
| single-encrypt-worker | `FIPS_CONNECTED_UDP=1 FIPS_ENCRYPT_WORKERS=1 FIPS_DECRYPT_WORKERS=0` | `d9d0554bc89916398eb0ba8e0a75c49086786b3181c9e1ff79308ac1ccf6cc6b` | 2052.7 / 1950.6 | 169.9 / 165.0 | 566.0 / 548.9 | 2029.5 / 1955.0 | <= 2.220 ms |
| tight-backpressure | `FIPS_CONNECTED_UDP=1 FIPS_WORKER_CHANNEL_CAP=4 FIPS_SEND_BACKPRESSURE_SLEEP_AFTER=1 FIPS_SEND_BACKPRESSURE_SLEEP_MICROS=500 FIPS_SEND_BACKPRESSURE_DROP_AFTER=0` | `d9f17e17ca9a684812d0f852d8381fe3f18a0d5e58d115d10706bf93bfbddc0d` | 474.3 / 493.8 | 180.1 / 179.3 | 493.7 / 502.2 | 449.5 / 453.0 | <= 2.329 ms |

## Linux Deterministic Unit Runner

FIPS commit `b95f00f Add Linux dataplane safety test runner` adds:

```sh
./scripts/test-dataplane-safety-linux-docker.sh
```

The default filter list passed from the local host in Docker. It covered:

- Encrypt-worker queue lane policy added in
  `7146193 Name encrypt worker queue lanes`:
  `encrypt_worker_lane_policy_keeps_endpoint_bulk_explicit`.
- Linux fair-worker full-queue/backpressure tests:
  `single_flow_full_backpressures_instead_of_dropping`,
  `new_flow_can_enter_when_hot_flow_reaches_per_flow_cap`,
  `hot_flow_backpressures_when_others_are_waiting`,
  `encrypt_worker_dispatch_preserves_single_flow_worker_and_fifo_order`, and
  `fair_dispatch_does_not_block_rx_loop_on_full_bulk_queue`.
- Decrypt-worker full-queue tests added in
  `e5a0243 Cover decrypt worker queue pressure`:
  `decrypt_worker_full_queue_drops_bulk_without_waiting` and
  `decrypt_worker_register_full_returns_false_without_waiting`.
- Decrypt-worker queue-cap knob test added in
  `47c848e Make decrypt worker queue cap explicit`:
  `decrypt_worker_channel_cap_prefers_specific_then_shared_value`.
- Decrypt-worker priority-lane tests added in
  `c2ad09c Reserve decrypt worker priority lane`:
  `decrypt_worker_priority_packet_classifier_keeps_small_packets_reserved`,
  `decrypt_worker_priority_packet_uses_priority_lane_when_bulk_queue_is_full`,
  and `decrypt_worker_register_uses_priority_lane_when_bulk_queue_is_full`.
- Decrypt-worker bounded fallback-lane tests added in
  `0e58727 Bound decrypt worker fallback lanes`:
  `decrypt_worker_fallback_event_classifier_uses_priority_and_bulk_lanes`
  and `decrypt_worker_fallback_bulk_full_does_not_starve_priority_events`.
- Endpoint priority classification:
  `endpoint_payload_traffic_classifier_prioritizes_control_sized_packets`.
- Direct-vs-fallback route sanity:
  `test_reply_learned_prefers_live_mesh_route_over_stale_direct_peer`,
  `test_reply_learned_prefers_live_mesh_route_over_session_degraded_direct_peer`,
  `test_reply_learned_keeps_configured_static_direct_peer_despite_session_degraded`,
  `test_reply_learned_keeps_configured_static_direct_peer_over_lower_cost_fallback`,
  and `test_tree_routing_skips_session_degraded_direct_peer_for_payload`.
- MMP route robustness:
  `test_stale_session_receiver_reports_do_not_change_route_choice`,
  `test_stale_mmp_receiver_reports_do_not_change_route_choice`,
  `test_session_receiver_loss_degrades_direct_and_uses_fallback`,
  `test_ignores_duplicate_receiver_report_after_valid_sample`, and
  `test_ignores_out_of_order_receiver_report_after_valid_sample`.
- MMP parent-choice robustness:
  `test_parent_reeval_ignores_unmeasured_peer_costs` and
  `test_parent_reeval_ignores_fresh_bogus_metrics_without_valid_rtt`.
- Connected UDP config/budget/rekey tests via the `connected_udp` filter
  (`10` tests in the Linux container).

Follow-up after FIPS commit `df6ee3d Guard encrypt worker single-flow ordering`
adds `encrypt_worker_dispatch_preserves_single_flow_worker_and_fifo_order` to
the default Linux deterministic runner. The test pins two ordering invariants
before shard/send-path refactors: all packets for one TCP-shaped send target
map to one encrypt worker, and the worker's fair queue drains that flow in FIFO
counter order.

Command:

```sh
./scripts/test-dataplane-safety-linux-docker.sh
```

Raw log captured locally as `/tmp/fips-single-flow-order-default-linux.log`.
Log SHA-256:
`9a0faa658c9b84a296c978ab55b56a59bbe33792318daa35e1c3dc3d630ceffd`.

The full default Linux runner passed with the new ordering filter included.
This commit changes only FIPS tests and the deterministic runner script; the
current runtime perf and platform-matrix baseline remains the FIPS `27d1739`
plus nvpn `b620ff56` evidence recorded below.

Follow-up after FIPS commit `ce651a7 Name encrypt worker send targets` makes
the exact encrypt-worker send target explicit as `(socket fd, connected fd,
destination address)` and reuses that key for macOS sender selection and batch
grouping. This is a small architecture-plan step toward selected-send-target
ownership; it does not move crypto/routing state.

Focused deterministic checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  mac_queue_tests -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  flush_batch_routes_each_target_separately \
  encrypt_worker_dispatch_preserves_single_flow_worker_and_fifo_order )
```

Short local-FIPS perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-ce651a7-nvpn-810a84cd-send-target-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-send-target-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Raw log captured locally as `/tmp/nvpn-fips-ce651a7-send-target-smoke.log`.
Log SHA-256:
`fb05bf6b7a3b14a7f7225442738ff35ff69813cabb6622b339643da445c80d6e`.

Phase summary:
`artifacts/fips-perf/fips-ce651a7-nvpn-810a84cd-send-target-smoke/phase-summary.tsv`.
Summary SHA-256:
`8c46d588b5ee9d3f470467015f83faac13a5f9e9303fd306831fafb142885e21`.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, worker-pressure emitted only expected decrypt-worker bulk pressure, and
post-load ping recovered in every phase.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2168.9` | `2263.2` | `2324.3` | `2198.3` | `1.880 / 1.880 / 1.875 ms` | `3.450 / 3.450 / 3.451 ms` | `1.480 / 1.480 / 1.475 ms` | `2336102097 / 2267425628` |
| constrained-underlay | `132.4` | `130.9` | `128.8` | `131.0` | `85.600 / 85.600 / 85.568 ms` | `95.000 / 95.000 / 95.036 ms` | `1.550 / 1.550 / 1.550 ms` | `137275772 / 142172629` |
| worker-queue-pressure | `126.9` | `124.6` | `134.9` | `134.5` | `0.329 / 0.329 / 0.329 ms` | `0.842 / 0.842 / 0.842 ms` | `0.962 / 0.962 / 0.962 ms` | `140132434 / 139163184` |
| rx-maintenance-fault | `2252.8` | `2243.5` | `2309.1` | `2257.9` | `2.650 / 2.650 / 2.652 ms` | `2.520 / 2.520 / 2.523 ms` | `1.040 / 1.040 / 1.041 ms` | `2347935607 / 2316684563` |

Follow-up after FIPS commit `1cabfce Key encrypt fair admission by send target`
extends the exact send-target key from batching into non-macOS worker selection
and fair-admission pressure accounting. The new deterministic guard
`fair_admission_keys_pressure_by_exact_send_target` proves that two sender fds
aimed at the same destination do not share per-target pressure budget, which
would have failed under the older destination-only admission key.

Focused deterministic checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  mac_queue_tests -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  fair_queue_tests \
  flush_batch_routes_each_target_separately \
  encrypt_worker_dispatch_preserves_single_flow_worker_and_fifo_order )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

Short local-FIPS perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-1cabfce-nvpn-cdf6e0d-send-target-key-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-send-target-key-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Raw log captured locally as `/tmp/nvpn-fips-1cabfce-send-target-key-smoke.log`.
Log SHA-256:
`9be0574d3be717c5a60ea61f9976de604295055696fe26d887f369c17ed4eb0d`.

Phase summary:
`artifacts/fips-perf/fips-1cabfce-nvpn-cdf6e0d-send-target-key-smoke/phase-summary.tsv`.
Summary SHA-256:
`238b532932a2196e6f977b23fb5d54331cb518153e39901ff40f7f3fe8025d31`.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, worker-pressure emitted only expected decrypt-worker bulk pressure, and
post-load ping recovered in every phase.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2328.2` | `2261.6` | `2335.9` | `2347.6` | `3.030 / 3.030 / 3.030 ms` | `2.860 / 2.860 / 2.855 ms` | `3.840 / 3.840 / 3.841 ms` | `2395341419 / 2390110904` |
| constrained-underlay | `132.2` | `131.6` | `130.0` | `129.3` | `86.400 / 86.400 / 86.445 ms` | `92.800 / 92.800 / 92.754 ms` | `3.860 / 3.860 / 3.858 ms` | `139299845 / 141354841` |
| worker-queue-pressure | `127.9` | `126.4` | `128.2` | `126.0` | `0.310 / 0.310 / 0.310 ms` | `0.357 / 0.357 / 0.357 ms` | `2.780 / 2.780 / 2.784 ms` | `135057714 / 135098746` |
| rx-maintenance-fault | `2266.2` | `2298.5` | `2291.9` | `2226.6` | `2.760 / 2.760 / 2.763 ms` | `4.010 / 4.010 / 4.011 ms` | `2.000 / 2.000 / 1.998 ms` | `2292638864 / 2321263787` |

Follow-up after FIPS commit `cbd9e4d Let encrypt priority bypass bulk flow cap`
lets non-macOS encrypt-worker priority jobs bypass the bulk per-target fair
admission cap while still remaining bounded by the worker channel. The new
deterministic guard `priority_flow_enters_when_bulk_flow_reaches_per_flow_cap`
fills the direct fast lane, saturates a hot bulk flow's fair budget, proves a
third bulk packet is rejected, and then proves a priority packet for the same
send target can still enter. That would have failed when priority and bulk shared
the same per-flow fair-admission cap.

Focused deterministic checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  mac_queue_tests -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  priority_flow_enters_when_bulk_flow_reaches_per_flow_cap )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  fair_queue_tests )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

Short local-FIPS perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-cbd9e4d-nvpn-65cbe34-priority-bypass-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-priority-bypass-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Raw log captured locally as `/tmp/nvpn-fips-cbd9e4d-priority-bypass-smoke.log`.
Log SHA-256:
`b59fe1e556347e1d6863b57e4a15546596ed29f055fb0ca7978f8b5eae1e1409`.

Phase summary:
`artifacts/fips-perf/fips-cbd9e4d-nvpn-65cbe34-priority-bypass-smoke/phase-summary.tsv`.
Summary SHA-256:
`5f05f2c8ff043a1ad0258f326698b2b19eba11d3e0093d914ba65f0909866fed`.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, worker-pressure emitted only expected decrypt-worker bulk pressure, and
post-load ping recovered in every phase.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2305.1` | `2253.8` | `2164.3` | `2209.7` | `18.900 / 18.900 / 18.933 ms` | `4.450 / 4.450 / 4.447 ms` | `2.050 / 2.050 / 2.050 ms` | `2286577620 / 2271737832` |
| constrained-underlay | `131.1` | `131.1` | `129.3` | `128.3` | `85.100 / 85.100 / 85.070 ms` | `90.200 / 90.200 / 90.232 ms` | `3.260 / 3.260 / 3.255 ms` | `137962953 / 140117751` |
| worker-queue-pressure | `127.0` | `127.0` | `129.4` | `131.8` | `0.414 / 0.414 / 0.414 ms` | `0.357 / 0.357 / 0.357 ms` | `1.410 / 1.410 / 1.406 ms` | `137239702 / 137550765` |
| rx-maintenance-fault | `1405.1` | `1616.5` | `1955.1` | `2012.4` | `2.470 / 2.470 / 2.465 ms` | `5.250 / 5.250 / 5.245 ms` | `1.050 / 1.050 / 1.045 ms` | `1642504479 / 1804833975` |

Platform matrix follow-up for FIPS `cbd9e4d` plus nvpn `3eda64fd`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-cbd9e4d-nvpn-3eda64fd \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-cbd9e4d-matrix \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The full Linux/Docker matrix did not pass cleanly: `connected-udp-off` hit one
constrained-underlay reverse-load ping loss sample (`5%`, above the `2%`
ceiling). The other three scenarios passed, including the single encrypt worker
and tight-backpressure profiles. This is local Docker/platform-split evidence
only; it is not real Mac-to-Mac Wi-Fi/screenshare coverage.

Summary path:
`artifacts/fips-platform-matrix/fips-cbd9e4d-nvpn-3eda64fd/summary.tsv`.
Summary SHA-256:
`7dbc1f0455dde2d2af65530b3f6449a20fee3b72fd792b9ae1fbc23689e962b6`.

| Scenario | Status | Log SHA-256 | Phase Summary SHA-256 | Note |
| --- | --- | --- | --- | --- |
| connected-udp-on | pass | `789bb1c00a58bedd7f86253959544805267e0975bb754618cd8712b9406cc034` | `207ae5ff571dabb6d5f807396d02644c28ac6d8fa1fda0b860c417459fe67c08` | Connected UDP direct path stayed within thresholds. |
| connected-udp-off | fail | `59d0b5333e208391ab2594a0e030330d006dc626b2d29cf43f449ad44a3c3fb1` | `023bde3a49e703b9d3036ee15e4ec8b5d8f2ecb1f3577e38fb2022a625274bfb` | Failed constrained-underlay reverse-load ping loss: `5%` > `2%`. |
| single-encrypt-worker | pass | `bf154c39d1d56c2da705239d5f14ce790ff92672017bc08a7a5e8904b111755c` | `1f9104c36e1d59bc2945618349d51de50b4c84064d66f8f346de6da051ddb356` | Single encrypt worker plus no decrypt workers stayed recoverable. |
| tight-backpressure | pass | `d9aa2b80a53f85e310285680a3992b63664ae9001d2660d3d7ca4c6587810c81` | `0f6a6af3d0c172de43f572daadf6a28e0f56640855daa7942c83484eeabccbaf` | Tight worker/send backpressure emitted bounded bulk drops and stayed within thresholds. |

Targeted rerun for the intermittent `connected-udp-off` failure:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=connected-udp-off \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-cbd9e4d-nvpn-3eda64fd-connected-udp-off-rerun1 \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-cbd9e4d-off-rerun1 \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The rerun passed all four perf phases for `connected-udp-off`. Its constrained
reverse-load ping window had `0%` loss, average `86.272 ms`, p95 `129 ms`, p99
`134 ms`, and max `134.087 ms`. Treat the first failure as an intermittent
safety-net signal to keep this UDP-off matrix row active, not as proof that the
current connected-UDP-off path is clean under every constrained Docker run.

Rerun summary path:
`artifacts/fips-platform-matrix/fips-cbd9e4d-nvpn-3eda64fd-connected-udp-off-rerun1/summary.tsv`.
Rerun summary SHA-256:
`f6ab01547f549ed59b2e15b947f3dd758b7fc81f420f2da57ec07c0dad195d26`.
Rerun log SHA-256:
`f4f6bc1549beeddd6f609e366bc7d439c83543f8f1c260ae1a91b06a9816057e`.
Rerun phase summary SHA-256:
`2ecd661bcf2780031707d48e563a4ac470c85807fad25fb7efe9dbcd15943d50`.

Follow-up harness smoke after adding `NVPN_PLATFORM_MATRIX_ATTEMPTS` so
intermittent matrix rows can be repeated without overwriting the original
attempt artifacts:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=connected-udp-off \
NVPN_PLATFORM_MATRIX_ATTEMPTS=2 \
NVPN_PLATFORM_MATRIX_DURATION_SECS=2 \
NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=3 \
NVPN_PLATFORM_MATRIX_PING_COUNT=8 \
NVPN_PLATFORM_MATRIX_WORKER_QUEUE_PRESSURE_CAP=0 \
NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=0 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=0 \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/repeat-attempt-smoke-fips-cbd9e4d-nvpn-799fc816 \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-repeat-attempt-smoke \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The smoke passed both `connected-udp-off` attempts with local FIPS patches
applied. It disabled constrained, worker-pressure, and rx-maintenance phases to
exercise the repeat artifact path quickly. The matrix summary now preserves the
old seven-column prefix and appends `attempt` / `attempts`, so existing
`scenario`, `status`, `log`, `sha256`, and `phase_summary` fields stay stable.

Summary path:
`artifacts/fips-platform-matrix/repeat-attempt-smoke-fips-cbd9e4d-nvpn-799fc816/summary.tsv`.
Summary SHA-256:
`6ce75a338c926a80045d2209b5d2a1432901bcdfeec8b82005cc5e347f9ae4a5`.

| Attempt | Log SHA-256 | Phase Summary SHA-256 | Clean fwd/rev Mbps | Fwd/rev load ping p95 | Post ping max |
| ---: | --- | --- | ---: | --- | --- |
| 1 | `0be9bee083d5eb04d92a7a0487aac5a83429b594445459c74f304da55b334810` | `35d6ca413c611602aee8775caff4f581e8271446e9ee8e67edc42342a4deeaec` | `2263.2 / 2262.2` | `2.99 / 2.61 ms` | `6.435 ms` |
| 2 | `ca3eaf138dd66021cc95279939925d3cfe37db44140724a9cd05afa602f3df96` | `a48262f918e06946106377d2bbb87390db46bc18793e1914fa17f8f0820b5f9e` | `2236.8 / 2050.0` | `3.11 / 3.14 ms` | `0.524 ms` |

## Parent Re-evaluation Metric Guard

Follow-up deterministic Linux smoke after adding parent-choice coverage in FIPS:
`fd4505d Guard parent reevaluation from unmeasured metrics`.

Command:

```sh
./scripts/test-dataplane-safety-linux-docker.sh \
  test_parent_reeval_ignores_unmeasured_peer_costs
```

Raw log captured locally as `/tmp/fips-parent-reeval-unmeasured-linux.log`.
Log SHA-256:
`799e4c228930303df89bafe83f22106d8f5975340bebbecc54cc1102350258b9`.

The Linux Docker run passed. The test fixes the parent-choice side of the MMP
robustness invariant: a peer without RTT evidence must not be treated as an
artificially cheap default-cost parent during periodic parent re-evaluation,
even when it looks shallower than the current measured parent.

## Decrypt Priority Drain Guard

Follow-up deterministic Linux smoke after adding FIPS decrypt-worker
drain-order coverage and adding the existing worker-bounce observability tests
to the default Linux safety runner.

Command:

```sh
./scripts/test-dataplane-safety-linux-docker.sh \
  decrypt_worker_drain_registers_priority_before_bulk_jobs \
  worker_preserves_fmp_flags_through_fallback \
  worker_reports_fmp_aead_failure_to_rx_loop
```

Raw log captured locally as `/tmp/fips-decrypt-priority-drain-linux.log`.
Log SHA-256:
`2d416843248f55df11d34ee024c4ffbb87bb8c469b330793de28db5299b90daf`.

The Linux Docker run passed. The new priority-drain test queues a session
registration on the priority lane and an invalid bulk decrypt job for that same
session, then drains once. It fails if bulk work runs first, because the job is
silently dropped before the worker receives session state. Passing proves queued
priority control is applied before queued bulk decrypt work. The two worker
bounce tests keep FMP CE/SP flags and AEAD-failure reports observable on the
worker path, protecting MMP RTT/ECN/no-progress diagnosis while the dataplane is
being refactored.

## Decrypt Fallback Lane Guard

Follow-up after FIPS commit `0e58727 Bound decrypt worker fallback lanes`
replaced the decrypt-worker-to-rx-loop unbounded fallback channel with bounded
priority and bulk fallback lanes. Small/control-shaped plaintext bounces and
decrypt-failure reports use the priority fallback lane; large plaintext bounces
use the bulk fallback lane. New perf events make fallback-lane pressure visible:
`decrypt_fallback_bulk_dropped` and `decrypt_fallback_priority_dropped`.
The Docker and host-pair soak harnesses now parse these counters and treat any
non-zero fallback drop as a hard event unless queue events are explicitly
allowed for a pressure experiment.

Commands:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  decrypt_worker_fallback_ -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  decrypt_worker_ -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )

NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-0e58727-bounded-fallback-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-bounded-fallback-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

The deterministic Linux runner passed with the new fallback filters in the
default list. The short perf smoke passed all four phases: clean underlay,
constrained underlay, worker queue pressure, and rx-maintenance fault. The
worker-pressure phase intentionally reported decrypt-worker queue-full and
bulk-drop events while sampled fallback bulk-drop counters stayed at `0/s`.

Phase summary:
`artifacts/fips-perf/fips-0e58727-bounded-fallback-smoke/phase-summary.tsv`.
Summary SHA-256:
`0bd96a316bbeacf8be6fb41e90712e697e3a0071c8e6bd7ea927f27aaba5a798`.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2277.1` | `2289.1` | `2246.3` | `2243.4` | `1.790 / 1.790 / 1.794 ms` | `3.820 / 3.820 / 3.819 ms` | `1.360 / 1.360 / 1.357 ms` | `2334861430 / 2340702977` |
| constrained-underlay | `132.2` | `126.2` | `130.9` | `129.0` | `91.300 / 91.300 / 91.324 ms` | `89.000 / 89.000 / 89.048 ms` | `2.790 / 2.790 / 2.788 ms` | `141434888 / 140315043` |
| worker-queue-pressure | `128.3` | `126.6` | `129.9` | `126.3` | `0.360 / 0.360 / 0.360 ms` | `0.578 / 0.578 / 0.578 ms` | `2.110 / 2.110 / 2.105 ms` | `137402385 / 135172644` |
| rx-maintenance-fault | `2289.2` | `2201.4` | `2255.2` | `2313.3` | `3.360 / 3.360 / 3.360 ms` | `2.410 / 2.410 / 2.409 ms` | `2.130 / 2.130 / 2.125 ms` | `2346156480 / 2353239823` |

Thirty-minute Docker soak after the fallback lane and harness hard-event wiring:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_SOAK_DURATION_SECS=1800 \
NVPN_SOAK_OUTPUT_DIR=artifacts/fips-soak/fips-0e58727-nvpn-d9c24147-30m \
PROJECT_NAME=nostr-vpn-soak-fallback-30m \
./scripts/soak-fips-dataplane-docker.sh
```

Ignored sample file:
`artifacts/fips-soak/fips-0e58727-nvpn-d9c24147-30m/samples.ndjson`.
Sample SHA-256:
`78b1f7d1f28c6ff4f217a01eb3413a25dc3b6c2dd811d64a4b37dac826e57046`.

Raw log captured locally as
`/tmp/nvpn-fips-soak-fips-0e58727-nvpn-d9c24147-30m.log`.
Log SHA-256:
`c88581d72be4f65aae2860eed897eb46d0b56b805d2c7f9b3e541be720561437`.

The soak passed with `33` samples from `2026-06-08T03:59:28Z` through
`2026-06-08T04:29:21Z`. Both peers stayed on one configured direct Docker
underlay endpoint for the entire run. All hard queue/drop events stayed absent,
including `decrypt_fallback_bulk_dropped` and
`decrypt_fallback_priority_dropped`.

Observed ranges:

- FIPS SRTT: node-a `1-2 ms`, node-b `1-2 ms`
- Ping loss: `0%` both ways in all samples
- Ping avg: node-a to node-b `1.109-1.941 ms`, node-b to node-a
  `1.248-2.064 ms`
- Ping max: node-a to node-b `1.800-7.246 ms`, node-b to node-a
  `1.859-8.499 ms`
- Iperf forward: `2202.481-2305.731 Mbps`
- Iperf reverse: `2184.375-2286.383 Mbps`
- Iperf retransmits: forward `70-344`, reverse `95-265`
- Daemon CPU: node-a `45.7-79.8%`, node-b `45.7-86.8%`
- Final FIPS counters:
  - node-a sent/recv bytes: `63608656144` / `62687061869`
  - node-b sent/recv bytes: `62938415533` / `63615008430`

## Encrypt Queue Lane Guard

Follow-up after FIPS commit `7146193 Name encrypt worker queue lanes` made the
outbound worker queue policy explicit as `EncryptWorkerLane::Priority` or
`EncryptWorkerLane::Bulk`, and added
`encrypt_worker_lane_policy_keeps_endpoint_bulk_explicit` to the default Linux
deterministic safety runner.

Commands:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  encrypt_worker_lane_policy_keeps_endpoint_bulk_explicit -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )

NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=0 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=0 \
NVPN_PERF_RX_MAINT_FAULT_MS=0 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-7146193-encrypt-lane-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-encrypt-lane-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

The deterministic Linux runner passed with the new filter at the start of the
default list. The short perf smoke intentionally ran only `clean-underlay`; the
constrained-underlay, worker-pressure, and rx-maintenance phases were skipped by
setting their knobs to `0`.

Phase summary:
`artifacts/fips-perf/fips-7146193-encrypt-lane-smoke/phase-summary.tsv`.
Summary SHA-256:
`54923e1bc0797947b7cf59ecc2f2a1d3fec51d93c360e3c6b1fd7333defae308`.

The clean-underlay smoke passed with direct UDP underlay byte-counter progress
on both nodes and no ping loss under TCP load:

| Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| `2310.7` | `2235.3` | `2280.0` | `2214.3` | `2.770 / 2.770 / 2.767 ms` | `3.130 / 3.130 / 3.128 ms` | `1.340 / 1.340 / 1.344 ms` | `2357181509 / 2290478523` |

## Endpoint Blocking Lane Guard

Follow-up after FIPS commit `714474b Classify blocking endpoint sends by payload
lane` made `FipsEndpoint::blocking_send` use the same priority/bulk endpoint
command channel selection as async `send`. This keeps large blocking endpoint
packets on the bounded bulk lane instead of silently bypassing queue pressure on
the reserved priority lane.

Commands:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  endpoint_command_tx_helper_classifies_priority_and_bulk_payloads -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  endpoint_payload_traffic_classifier_prioritizes_control_sized_packets -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )

NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-714474b-endpoint-blocking-lane-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-endpoint-blocking-lane-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

The focused tests, full deterministic Linux runner, and short perf smoke passed.
The worker-pressure phase intentionally reported decrypt-worker queue-full and
bulk-drop events while tunnel ping stayed at `0%` loss and direct UDP byte
counters advanced on both nodes.

Phase summary:
`artifacts/fips-perf/fips-714474b-endpoint-blocking-lane-smoke/phase-summary.tsv`.
Summary SHA-256:
`3cc795bdd709ab2f3e37371cd339dc17c6bb213acf81f4c034993a9311996279`.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2256.5` | `2297.2` | `2303.4` | `2267.5` | `3.080 / 3.080 / 3.083 ms` | `3.420 / 3.420 / 3.420 ms` | `2.800 / 2.800 / 2.795 ms` | `2348052094 / 2333584201` |
| constrained-underlay | `129.6` | `130.6` | `128.2` | `128.7` | `97.000 / 97.000 / 96.951 ms` | `93.400 / 93.400 / 93.409 ms` | `1.900 / 1.900 / 1.898 ms` | `138818511 / 142014249` |
| worker-queue-pressure | `134.7` | `126.0` | `125.2` | `128.7` | `0.346 / 0.346 / 0.346 ms` | `0.346 / 0.346 / 0.346 ms` | `3.390 / 3.390 / 3.387 ms` | `139038039 / 135371492` |
| rx-maintenance-fault | `2365.7` | `2199.2` | `2225.0` | `2302.9` | `3.480 / 3.480 / 3.476 ms` | `2.510 / 2.510 / 2.511 ms` | `3.320 / 3.320 / 3.316 ms` | `2329401744 / 2313948911` |

## Pending Session Queue Drop Events

Follow-up after FIPS commit `27d1739 Expose pending session queue drops` made
the existing bounded pending-session queues observable. The behavior did not
change: pending TUN packets and endpoint payloads still reject new destinations
when `pending_max_destinations` is full and drop the oldest packet when
`pending_packets_per_dest` is full. The change adds FIPS pipeline counters:
`pending_tun_destination_dropped`, `pending_tun_packet_dropped`,
`pending_endpoint_destination_dropped`, and
`pending_endpoint_packet_dropped`.

Commands:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  pending_session_queues_ -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

The focused tests and full deterministic Linux runner passed. The default
runner now includes both pending-session queue policy tests, so future
dataplane changes keep those bounded drop paths pinned. The Docker and host-pair
soak harnesses parse the new counters and treat them as hard events unless
queue events are explicitly allowed for a pressure experiment.

Latest combined FIPS/nvpn perf smoke after wiring the soak parsers:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-27d1739-nvpn-4c8a98a-current-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-current-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

The smoke passed all four phases against FIPS `27d1739` and nvpn `4c8a98a0`.
Every phase advanced direct UDP byte counters on both nodes. The sampled
pipeline summaries contained no `pending_tun_*_dropped` or
`pending_endpoint_*_dropped` counters.

Phase summary:
`artifacts/fips-perf/fips-27d1739-nvpn-4c8a98a-current-smoke/phase-summary.tsv`.
Summary SHA-256:
`587e4e89b89fc399b91c3c1dd4c6bcb1ff75c877741d0c7004ba59d81c204ee0`.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `1292.2` | `2278.0` | `2276.7` | `2228.9` | `2.940 / 2.940 / 2.935 ms` | `4.030 / 4.030 / 4.029 ms` | `1.500 / 1.500 / 1.500 ms` | `1870837937 / 2277843854` |
| constrained-underlay | `131.2` | `129.4` | `130.2` | `130.3` | `84.700 / 84.700 / 84.680 ms` | `94.400 / 94.400 / 94.386 ms` | `2.250 / 2.250 / 2.250 ms` | `139209153 / 140818223` |
| worker-queue-pressure | `128.4` | `124.6` | `138.8` | `146.1` | `0.366 / 0.366 / 0.366 ms` | `0.365 / 0.365 / 0.365 ms` | `2.950 / 2.950 / 2.953 ms` | `141461690 / 145076644` |
| rx-maintenance-fault | `2237.3` | `2237.0` | `2257.1` | `2227.3` | `2.130 / 2.130 / 2.133 ms` | `3.600 / 3.600 / 3.600 ms` | `1.120 / 1.120 / 1.119 ms` | `2312124653 / 2272961333` |

Current platform matrix smoke against the same FIPS safety commit:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_DURATION_SECS=2 \
NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=3 \
NVPN_PLATFORM_MATRIX_PING_COUNT=8 \
NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=50 \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-27d1739-nvpn-b620ff56-current-smoke \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-current-matrix \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The matrix passed all four scenarios against FIPS `27d1739` and nvpn
`b620ff56`. Every phase advanced direct UDP byte counters on both nodes. The
tight and worker-pressure cases reported only the expected explicit bulk
queue-full/drop events; there was no permanent TCP or ping collapse.

Matrix summary:
`artifacts/fips-platform-matrix/fips-27d1739-nvpn-b620ff56-current-smoke/summary.tsv`.
Summary SHA-256:
`93ca4002947940a0975bbfbb6faa584c7da054675014bd75be329a3e829497fe`.

Phase-summary SHA-256 values:

- `connected-udp-on`: `d62a31e96b049ea074ec801b945b9f626a11f0eaa65b7b52b5cd7ee078aa5d8b`
- `connected-udp-off`: `f293594f1276d349c75380d53f880716dc88bfc8350ea76be15a9686f6e11c79`
- `single-encrypt-worker`: `88587c7d92ce3cbc7cf3e8a7930ef348795af35754ba6822da0ef38cbf684371`
- `tight-backpressure`: `dac5f36b91dbac7ddeff217df2dd983712a76689dea283a43b78563a5c7f5bb3`

| Scenario | Extra env | Elapsed | Log SHA-256 | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker fwd-load/rev-load Mbps | Rx fault fwd-load/rev-load Mbps | Post-load ping max |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | --- |
| connected-udp-on | `FIPS_CONNECTED_UDP=1` | `147s` | `53e0fb763a4acd9cdd2810f1e3e96ad5f57d67e32047263dc247bc8946db3c6f` | `2237.4 / 2240.7` | `178.5 / 179.3` | `125.5 / 129.2` | `2152.9 / 2211.1` | `<= 1.838 ms` |
| connected-udp-off | `FIPS_CONNECTED_UDP=0` | `118s` | `59a388cdc5194c9f16829ec05f18bd0a42e4a81f94e49d52785cc1c2e968a256` | `2320.0 / 2312.4` | `175.0 / 174.6` | `139.7 / 145.2` | `2400.0 / 2412.4` | `<= 2.080 ms` |
| single-encrypt-worker | `FIPS_CONNECTED_UDP=1 FIPS_ENCRYPT_WORKERS=1 FIPS_DECRYPT_WORKERS=0` | `117s` | `fe3a95ff887dab90d4300489b83a88a98823d58e513c0e31c046e28ece552eca` | `1949.1 / 1914.3` | `171.3 / 173.4` | `590.8 / 564.9` | `1984.6 / 2010.2` | `<= 2.016 ms` |
| tight-backpressure | `FIPS_CONNECTED_UDP=1 FIPS_WORKER_CHANNEL_CAP=4 FIPS_SEND_BACKPRESSURE_SLEEP_AFTER=1 FIPS_SEND_BACKPRESSURE_SLEEP_MICROS=500 FIPS_SEND_BACKPRESSURE_DROP_AFTER=0` | `147s` | `eff63fb8b0f3a27a61e3d73f81ef09c9cd15ad8121e569283f859af7140c8e97` | `136.7 / 128.8` | `133.2 / 125.3` | `127.8 / 128.5` | `127.2 / 125.3` | `<= 4.816 ms` |

## Metric Capture Smoke

Follow-up smoke after adding explicit FIPS worker-queue perf events and normal
perf-gate pipeline summaries.

Environment:

- Nostr-vpn parent before the metric hook commit: `870f335e`
- Local FIPS commit: `8535703 Expose worker queue pressure perf events`

Command:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_DURATION_SECS=2 \
NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=3 \
NVPN_PLATFORM_MATRIX_PING_COUNT=8 \
NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=50 \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/local-fips-8535703-metric-smoke \
./scripts/e2e-fips-platform-matrix-docker.sh
```

Result: passed in `146s`. Raw log SHA-256:
`2d36f1d9c4b7fe4762d92500b5d11bdea541c12cea81c79ce8b53fcb221501cb`.

The log verifies the new metric shape:

- Ping lines include `p95=` and `p99=` during load and after load.
- Per-phase pipeline summaries include `fmp_worker_queue_wait` and
  `transport_queue_wait` p95/p99.
- Tight worker/backpressure phases reported explicit
  `encrypt_worker_queue_full` and `encrypt_worker_bulk_dropped` event rates.
- The FIPS event surface now also includes `decrypt_worker_queue_full`,
  `decrypt_worker_bulk_dropped`, and `decrypt_worker_register_full` for
  decrypt-side pressure.
- `c2ad09c Reserve decrypt worker priority lane` adds
  `decrypt_worker_priority_dropped` and keeps small established packets plus
  session registration on a separate bounded decrypt priority lane.
- Direct UDP underlay counters advanced in every phase.

## Decrypt Priority Lane Smoke

The sibling FIPS branch reproduced the decrypt-side starvation shape with a
short local-FIPS smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_DURATION_SECS=2 \
NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=3 \
NVPN_PLATFORM_MATRIX_PING_COUNT=8 \
NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=50 \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/local-fips-47c848e-tight-decrypt-cap-smoke \
./scripts/e2e-fips-platform-matrix-docker.sh
```

With `FIPS_DECRYPT_WORKER_CHANNEL_CAP=4` applied globally in that experiment,
the clean-underlay phase failed before the priority lane: reverse TCP load saw
`37.5%` ping loss, and the log reported decrypt worker bulk drops. After
scoping the decrypt cap to the worker-pressure phase and adding
`c2ad09c Reserve decrypt worker priority lane`, this short perf smoke passed:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=0 \
NVPN_PERF_RX_MAINT_FAULT_MS=0 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_DECRYPT_WORKER_QUEUE_PRESSURE_CAP=4 \
PROJECT_NAME=nostr-vpn-e2e-fips-decrypt-priority-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

The passing worker-pressure phase recorded `0%` ping loss both directions,
about `125-130 Mbps` TCP under the tight caps, direct UDP byte-counter progress
on both nodes, and explicit `decrypt_worker_queue_full` /
`decrypt_worker_bulk_dropped` pipeline events.

## Soak Pipeline Observability Smoke

Follow-up smoke after adding structured pipeline samples and hard-event alerts
to the soak harness.

Command:

```sh
NVPN_SOAK_DURATION_SECS=22 \
NVPN_SOAK_INTERVAL_SECS=8 \
NVPN_SOAK_PING_COUNT=3 \
NVPN_SOAK_IPERF_DURATION_SECS=1 \
NVPN_SOAK_OUTPUT_DIR=artifacts/fips-soak/pipeline-observability-final-smoke \
PROJECT_NAME=nostr-vpn-soak-pipeline-final-smoke \
./scripts/soak-fips-dataplane-docker.sh
```

Sample SHA-256:
`15ce3c23a4dc08545014f34a604cf75c8683c7bb548fe40db390c62d8108a997`.

Raw log captured locally as `/tmp/nvpn-fips-soak-pipeline-final-smoke.log`.
Log SHA-256:
`eeeac30b4e501a75d44a5387a24c259e4d029e385d485d4e9c9293535f76b9d8`.

The smoke passed with 3 samples, `0%` max ping loss, and the configured direct
UDP path in every sample:

- node-a transport: configured container underlay endpoint
- node-b transport: configured container underlay endpoint

The samples included `pipeline.*.fips` / `pipeline.*.nvpn` objects with raw
pipeline lines, current rates, max rates, seen flags, and totals when exposed
by the daemon log format. No hard queue/drop events were observed.

## Soak Drift and Progress Smoke

Follow-up smoke after adding per-run ping/SRTT drift checks and FIPS byte-counter
no-progress checks to the soak harness.

Command:

```sh
NVPN_SOAK_DURATION_SECS=24 \
NVPN_SOAK_INTERVAL_SECS=8 \
NVPN_SOAK_PING_COUNT=3 \
NVPN_SOAK_IPERF_DURATION_SECS=1 \
NVPN_SOAK_OUTPUT_DIR=artifacts/fips-soak/drift-progress-smoke \
PROJECT_NAME=nostr-vpn-soak-drift-progress-smoke \
./scripts/soak-fips-dataplane-docker.sh
```

Sample SHA-256:
`ded8bd3515159b2c2c7af30c891a6359c356089b500507c46a6e1f9078595540`.

Raw log captured locally as `/tmp/nvpn-fips-soak-drift-progress-smoke.log`.
Log SHA-256:
`8d33ab231d6f3d7391ff91b351d9bb93ddf2ebb24b1ade84f463edf4a8eb5f01`.

The smoke passed with 4 samples, `0%` max ping loss, and the configured direct
UDP path in every sample. Ping averages stayed between `0.992-1.483 ms`
node-a to node-b and `1.107-1.592 ms` node-b to node-a. FIPS byte counters
advanced from initial control traffic to roughly `1.8-2.0 GB` per direction,
proving the no-progress checks tolerate normal status timing on the Docker path.

## Perf Phase Summary Artifact Smoke

Follow-up smoke after adding machine-readable phase summaries to the perf gate
and wiring the platform matrix to record each scenario's summary path.

Perf gate command:

```sh
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/phase-summary-smoke \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=0 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=0 \
NVPN_PERF_RX_MAINT_FAULT_MS=0 \
PROJECT_NAME=nostr-vpn-e2e-fips-phase-summary-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Raw log captured locally as
`/tmp/nvpn-fips-perf-phase-summary-smoke.log`.
Log SHA-256:
`cda1f90179cf907bd2d5e8a3ab4a85636a462e216fa0bff2d987204c1c3700d5`.

Phase summary:
`artifacts/fips-perf/phase-summary-smoke/phase-summary.tsv`.
Summary SHA-256:
`e191318f63a3a9d187de1c34891f490da66ccf3831ef4554328fe17c64a323ad`.

The smoke wrote one `clean-underlay` row with `28` fields matching the header,
`0%` ping loss during forward load and after TCP load, and direct UDP underlay
byte deltas of `2329420717` on node-a and `2316934793` on node-b.

Platform matrix wrapper command:

```sh
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/phase-summary-wrapper-smoke \
NVPN_PLATFORM_MATRIX_SCENARIOS=connected-udp-off \
NVPN_PLATFORM_MATRIX_DURATION_SECS=2 \
NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=3 \
NVPN_PLATFORM_MATRIX_PING_COUNT=8 \
NVPN_PLATFORM_MATRIX_WORKER_QUEUE_PRESSURE_CAP=0 \
NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=0 \
./scripts/e2e-fips-platform-matrix-docker.sh
```

Raw log captured locally as
`/tmp/nvpn-fips-platform-phase-summary-wrapper-smoke.log`.
Log SHA-256:
`27ef253216e6b2be8af3381b6e7f2e99e7489971f62c8cce0305ca0aaa097cbd`.

Matrix summary:
`artifacts/fips-platform-matrix/phase-summary-wrapper-smoke/summary.tsv`.
Summary SHA-256:
`d72c6b6a9661d6418f513ddbdebb1f889c725afeee8be8d5c4267f4a9387cfc7`.

Scenario phase summary:
`artifacts/fips-platform-matrix/phase-summary-wrapper-smoke/connected-udp-off-perf/phase-summary.tsv`.
Scenario summary SHA-256:
`6665e4ae675f7d7867ec75b0cc58e0ab6317b95e14ab4ffac510217c00b48ebf`.

The matrix smoke passed. Its `summary.tsv` had `7` fields per row including the
new `phase_summary` path. The nested perf summary had `28` fields per row for
both `clean-underlay` and `constrained-underlay`; both rows kept forward-load
and post-load ping loss at `0%` and advanced direct UDP underlay bytes on both
nodes.

## Connected UDP Cap Ordering Smoke

Follow-up deterministic Linux smoke after FIPS commit `5a82419` added stable
connected-UDP activation ordering for capped large-mesh fast-path selection.

Command:

```sh
./scripts/test-dataplane-safety-linux-docker.sh connected_udp
```

Raw log captured locally as
`/tmp/fips-connected-udp-ordering-linux-docker.log`.
Log SHA-256:
`5c768441dc55608c585a8c6cf8c59ed3462667da59c01443dee6aedff3ee0853`.

The smoke passed `10` connected-UDP tests on Linux, including
`activation_order_prefers_configured_peers_then_node_addr` and the Linux-only
rekey-drain preservation case.

## Host/VM Pair Smoke

Follow-up local-to-Linux/VM host-pair smoke using explicit peer selectors and
strict direct-path expectations. Hostnames, peer keys, tunnel IPs, and underlay
addresses were supplied through environment variables and are intentionally not
recorded here.

Command shape:

```sh
NVPN_HOST_PAIR_SSH="$LINUX_OR_VM_SSH_TARGET" \
NVPN_HOST_PAIR_LOCAL_PEER="$REMOTE_PARTICIPANT_PUBKEY" \
NVPN_HOST_PAIR_REMOTE_PEER="$LOCAL_PARTICIPANT_PUBKEY" \
NVPN_HOST_PAIR_EXPECTED_REMOTE_UNDERLAY_IP="$REMOTE_UNDERLAY_IP" \
NVPN_HOST_PAIR_EXPECTED_LOCAL_UNDERLAY_IP="$LOCAL_UNDERLAY_IP" \
NVPN_HOST_PAIR_DURATION_SECS=55 \
NVPN_HOST_PAIR_INTERVAL_SECS=1 \
NVPN_HOST_PAIR_PING_COUNT=3 \
NVPN_HOST_PAIR_PING_INTERVAL=0.1 \
NVPN_HOST_PAIR_IPERF_DURATION_SECS=1 \
./scripts/soak-fips-dataplane-host-pair.sh
```

The first finalized-script run wrote:

- samples: `/tmp/nvpn-host-pair-smoke-20260608T031102Z/samples.ndjson`
- summary: `/tmp/nvpn-host-pair-smoke-20260608T031102Z/summary.tsv`
- summary SHA-256:
  `b08995b01eefe480dcbd12148e3df84558e973e3e0b6b2af31ca15ce39b7ed5f`

The first sample used the intended direct path in both directions and recorded:

| Fwd ping loss/avg/p99 | Rev ping loss/avg/p99 | Fwd TCP Mbps/retrans | Rev TCP Mbps/retrans | FIPS SRTT local/remote | CPU local/remote |
| --- | --- | --- | --- | --- | --- |
| `0% / 44.046 ms / 67.616 ms` | `0% / 46.685 ms / 106.000 ms` | `0.887 / 64` | `1.050 / 18` | `108 ms / 2163 ms` | `9.2% / 1.9%` |

The rerun then failed on the second sample before writing a second summary row:
remote-to-local tunnel ping loss was `33.333%`, exceeding the `5%` default
ceiling. The failing ping log SHA-256 was
`f830d7491a0146cc11b58503c714d3896a4b6d424955b547c9ac8a9cffbe3978`.

This is a reproduced host/VM path degradation caught by the safety harness, not
a completed 30-60 minute soak. A full host-pair soak still needs a stable
Linux/VM target and should keep the same direct-path, ping p95/p99, TCP burst,
SRTT, byte-counter, daemon CPU, and pipeline-event checks enabled.

Follow-up parser smoke after tightening the host-pair `iperf3 -R` handling:

- summary: `/tmp/nvpn-host-pair-parser-smoke-20260608T031515Z/summary.tsv`
- summary SHA-256:
  `cbb0fae2b30382da5bce05982e00d4967561612095abb39f16c69d6d9ec5ed9e`

That run used relaxed ping-loss limits only to exercise the parser on a lossy
host/VM path. It wrote two summary rows, kept strict direct-path checks enabled,
confirmed byte-counter progress on sample 2, and kept both TCP-collapse counters
at `0`. Reverse TCP Mbps parsed as `1.052` and `1.053`, proving the summary no
longer reports false zero throughput when a short `iperf3 -R` JSON sample has a
zero receiver summary but nonzero endpoint transfer.

Follow-up failure-path smoke after adding the EXIT trap and sanitized
`failure.json`:

- output dir: `/tmp/nvpn-host-pair-metric-failure-20260608T032448Z`
- failure SHA-256:
  `a5c7ea775865bc1e49c39d9f8ee4c75deef63f920f27c402ebdef891bef17716`

The run intentionally set an impossible ping-loss ceiling (`-1%`) and exited
with status `1`. The failure report recorded sample `1`, timestamp, artifact
paths, latest ping metrics, active thresholds, and TCP-collapse counters; the
EXIT trap left zero remote `iperf3` processes.

Longer host/VM soak attempt after the failure-report hardening:

- output dir: `/tmp/nvpn-host-pair-30m-attempt-20260608T032645Z`
- failure SHA-256:
  `2e7ae66859c793814c478e3ade531dbf743acd2612fa0b2dbd611ad4b11845d3`

The run was configured for `1800` seconds with strict direct-path expectations,
`20` tunnel pings per sample, `5` second iperf bursts, and
`NVPN_HOST_PAIR_MAX_SRTT_MS=3000` to avoid failing only because this host/VM
path had already shown high FIPS SRTT. It failed on sample `1` at
`2026-06-08T03:26:48Z` before iperf, SRTT, counter, or CPU metrics were
available: remote-to-local tunnel ping loss was `10%`, exceeding the default
`5%` ceiling. The `summary.tsv` contained only the header because the first
sample failed before a summary row was committed.

| Fwd ping loss/avg/p95/p99/max | Rev ping loss/avg/p95/p99/max | Failure threshold |
| --- | --- | --- |
| `0% / 71.090 ms / 175.697 ms / 196.510 ms / 196.510 ms` | `10% / 63.052 ms / 109.000 ms / 109.000 ms / 108.913 ms` | ping loss `<= 5%` |

This records another host/VM degradation caught immediately by the safety
harness. It still does not satisfy the requested 30-60 minute soak; a stable
Linux/VM target remains required before treating the host-pair soak as green.

Later strict direct-path host/VM attempt using explicit tunnel-matched peer
selectors and expected underlay IPs derived from the current statuses:

- output dir: `/tmp/nvpn-host-pair-30m-current-20260608T043840Z`
- summary SHA-256:
  `529cf64b3e85550a6eb3c4f71f866e3ff088f81218127cc6df43c4f42b78ecc1`
- samples SHA-256:
  `437c8a7e13844ae4439c19506919fcdf17b65024c91054b0c35581d66c31c782`
- log SHA-256:
  `ae4931a5a748708fe27a5dcec15de8cba5fda56962d4a8c71c712cb7dd00afaf`

The harness exited `0` after `13` samples from `2026-06-08T04:39:11Z` through
`2026-06-08T05:08:19Z`. `direct_path_checked=1` in every row, byte-counter
progress was checked after the first sample, FIPS SRTT stayed within `1-10 ms`,
and daemon CPU stayed within `4.6-90%` locally and `8.2-8.3%` remotely. Pipeline
log checks were not enabled for this run because the remote status path required
a temporary sudo wrapper while committed scripts must not bake in local host
details.

Observed ranges:

| Metric | Range |
| --- | --- |
| Forward ping loss / reverse ping loss | `0-5%` / `0%` |
| Forward ping avg / p99 / max | `1.049-25.854 ms` / `2.264-196.091 ms` / `2.264-196.091 ms` |
| Reverse ping avg / p99 / max | `0.747-13.117 ms` / `1.080-132.000 ms` / `1.082-131.654 ms` |
| Forward TCP Mbps / retransmits | `0-296.316` / `0-919` |
| Reverse TCP Mbps / retransmits | `378.188-822.159` / `0-158` |
| Forward / reverse collapse counters | `0-1` / `0` |

This is a completed strict host/VM soak, but not a clean throughput baseline.
The late samples recorded tolerated TCP-burst degradation: sample `10` logged
both forward and reverse `iperf3` command failures, sample `11` recorded forward
TCP `0 Mbps` with forward collapse count `1`, sample `12` reached the default
forward ping-loss ceiling at `5%`, and sample `13` logged a reverse `iperf3`
failure while forward TCP again reported `0 Mbps` with collapse count `1`. The
dataplane did not wedge and the harness recovered enough to finish, which keeps
this as useful safety-net evidence and leaves a clean host/VM performance
baseline as follow-up.

Follow-up host-pair harness hardening added command overrides for status, log
read command overrides for daemon logs, daemon-log path inference from
`nvpn status`, and `NVPN_HOST_PAIR_REQUIRE_PIPELINE_LOGS=1`. A one-sample smoke
with strict direct-path selectors and required pipeline logs failed at sample `1`
because the remote daemon log had no `pipe`/`nvpn-pipe` summaries available to
the harness. Ping, TCP, SRTT, and CPU were otherwise healthy for that sample,
which proves the new guard catches false pipeline-check coverage instead of
marking `pipeline_log_checked=1` from a path alone.

- failure SHA-256:
  `c92e2f39fbfbc69e7ce67af52da7ae570911e6f5049f5c2352f3e4e8e4043d6c`
- log SHA-256:
  `3ef287d147b342648932fd7c2a5a8bf9a28f082c8879f7727494a0bbde92a520`

Follow-up deterministic Linux smoke after FIPS commit `3c3995e Guard parent
choice from bogus MMP samples` added
`test_parent_reeval_ignores_fresh_bogus_metrics_without_valid_rtt` to the
default Linux safety runner.

Focused deterministic checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  test_parent_reeval_ignores_fresh_bogus_metrics_without_valid_rtt -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  test_parent_reeval_ignores_fresh_bogus_metrics_without_valid_rtt )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

Raw log captured locally as `/tmp/fips-fresh-bogus-mmp-default-linux.log`.
Log SHA-256:
`11af1dfb25fe5fb14758eebca5cde1eeb4916ebc3ffaa47dde44034d22bb4e2e`.

The full default Linux runner passed with the new parent-choice filter included.
The test feeds an attractive candidate parent two fresh ReceiverReports with
advanced counters, severe loss, and huge goodput bytes, while the RTT echo is
invalid. The candidate remains without SRTT, periodic parent re-evaluation keeps
the measured parent, and no parent-switch counter advances.

Follow-up host-pair harness self-test after making
`soak-fips-dataplane-host-pair.sh` source-safe for local helper tests:

```sh
bash -n scripts/soak-fips-dataplane-host-pair.sh
bash -n scripts/test-host-pair-harness.sh
./scripts/test-host-pair-harness.sh
```

The self-test passed. It covers local `pipe`/`nvpn-pipe` latest-line parsing,
hard queue/drop event parsing, `NVPN_HOST_PAIR_ALLOW_QUEUE_EVENTS`, strict
`NVPN_HOST_PAIR_REQUIRE_PIPELINE_LOGS` failures for missing summaries, and
`summary.tsv` flags for direct-path, pipeline-log, and counter-progress checks.
This pins the host/VM harness behavior that caught the previous false
pipeline-log coverage attempt; it does not replace a real host/VM soak.

Follow-up deterministic Linux smoke after FIPS commit
`4ed5c02 Pin static direct routes over learned fallback` promoted direct-path
route sanity into the default Linux safety runner.

Focused deterministic checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  test_reply_learned_keeps_configured_static_direct_peer_over_lower_cost_fallback -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  test_reply_learned_prefers_lower_cost_fallback_over_slow_healthy_direct_peer -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  test_reply_learned_prefers_live_mesh_route_over_stale_direct_peer \
  test_reply_learned_prefers_live_mesh_route_over_session_degraded_direct_peer \
  test_reply_learned_keeps_configured_static_direct_peer_despite_session_degraded \
  test_reply_learned_keeps_configured_static_direct_peer_over_lower_cost_fallback \
  test_tree_routing_skips_session_degraded_direct_peer_for_payload )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

Raw log captured locally as `/tmp/fips-static-direct-route-default-linux.log`.
Log SHA-256:
`f0f8b2c0443a08e260eed804d4bb328ec920e12a07771e05eca62a6ec9a70946`.

The full default Linux runner passed with the route sanity filters included.
The new test models a healthy operator-configured static UDP peer whose direct
MMP cost is worse than a learned fallback; route choice stays direct. The
complementary non-static test still passes, so a slow but healthy non-static
direct path may still use a cheaper learned fallback.

Follow-up perf-harness guard after nvpn commit
`a57b239f Record static direct route guard` turns ping p95/p99 from recorded
summary fields into explicit pass/fail thresholds.

Fast harness checks:

```sh
bash -n scripts/e2e-fips-perf-regression-docker.sh
bash -n scripts/test-fips-perf-harness.sh
./scripts/test-fips-perf-harness.sh
```

The self-test passed. It covers the ping percentile parser, p95-specific
failure, p99-specific failure, and the legacy five-argument `assert_ping_ok`
path where the phase max budget also gates p95/p99.

Short Docker clean-underlay smoke with FIPS commit `4ed5c02` patched in:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_PING_INTERVAL=0.1 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=0 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=0 \
NVPN_PERF_RX_MAINT_FAULT_MS=0 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/perf-p95p99-threshold-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-p95p99-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Raw log captured locally as `/tmp/nvpn-fips-perf-p95p99-threshold-smoke.log`.
Log SHA-256:
`bbf1f066357ca10bd531c5019e96051c2065f5fa18ffbcd613563a6abb9643d6`.

Phase summary SHA-256:
`f46133d5477ba3cfa9297c9908e14634492303f9525919f113bce0d31374bdc3`.

The smoke passed and printed the new threshold line:
`during_ping_p95<=1000ms`, `during_ping_p99<=1000ms`,
`post_ping_p95<=1000ms`, and `post_ping_p99<=1000ms`.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | 2339.4 | 2279.9 | 2246.0 | 2203.8 | 3.21 / 3.21 / 3.208 ms | 4.08 / 4.08 / 4.077 ms | 2.3 / 2.3 / 2.300 ms | 2347168368 / 2307895029 |

Follow-up Docker soak harness guard after nvpn commit
`2eca06e0 Gate perf ping tail latency` brings the local Docker/VM soak in line
with the host-pair soak for tail latency. The soak parser now records and
asserts ping p95/p99 in both directions, and later samples also apply p95/p99
drift checks against the first sample.

Fast harness checks:

```sh
bash -n scripts/soak-fips-dataplane-docker.sh
bash -n scripts/test-fips-soak-harness.sh
./scripts/test-fips-soak-harness.sh
```

The self-test passed. It covers ping percentile parsing, p95-specific failure,
p99-specific failure, and p95/p99 drift failure.

Short two-sample Docker soak smoke with FIPS commit `4ed5c02` patched in:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_SOAK_DURATION_SECS=12 \
NVPN_SOAK_INTERVAL_SECS=1 \
NVPN_SOAK_PING_COUNT=4 \
NVPN_SOAK_PING_INTERVAL=0.05 \
NVPN_SOAK_IPERF_DURATION_SECS=1 \
NVPN_SOAK_OUTPUT_DIR=artifacts/fips-soak/docker-p95p99-tail-smoke \
PROJECT_NAME=nostr-vpn-soak-fips-tail-smoke \
./scripts/soak-fips-dataplane-docker.sh
```

Raw log captured locally as `/tmp/nvpn-fips-soak-p95p99-tail-smoke.log`.
Log SHA-256:
`5c57ea835b35db2eb1237042ce442666b31687548967e0773bbbe64a1101e4bc`.

Sample SHA-256:
`184ccfba436a5e9a6cc7db8a313a25cc88f8c314958bcd468abdfeed63a78bfa`.

The smoke passed with `2` samples. A `jq` check confirmed every sample had
numeric ping p95/p99 fields in both directions, both samples stayed on the
configured direct UDP path, and sample 2 advanced FIPS sent/received byte
counters on both nodes.

| Sample | Fwd ping loss/avg/p95/p99/max | Rev ping loss/avg/p95/p99/max | Fwd TCP Mbps/retrans | Rev TCP Mbps/retrans | FIPS SRTT node-a/node-b | CPU node-a/node-b |
| ---: | --- | --- | ---: | ---: | --- | --- |
| 1 | 0% / 1.529 / 2.47 / 2.47 / 2.472 ms | 0% / 1.146 / 1.76 / 1.76 / 1.756 ms | 2283.738 / 36 | 2246.551 / 1 | 3 / 2 ms | 43.9% / 50.3% |
| 2 | 0% / 1.321 / 2.3 / 2.3 / 2.300 ms | 0% / 1.171 / 1.72 / 1.72 / 1.715 ms | 2250.280 / 20 | 2208.262 / 18 | 2 / 2 ms | 68.0% / 75.8% |

Follow-up Docker soak queue-wait guard after nvpn commit
`2b241973 Gate Docker soak ping tail latency` parses queue-wait p95/p99 from
FIPS `pipe` and nvpn `nvpn-pipe` summaries and fails clean soaks when max
observed queue-wait p95/p99 exceeds the configured thresholds.

Fast harness checks:

```sh
bash -n scripts/soak-fips-dataplane-docker.sh
bash -n scripts/test-fips-soak-harness.sh
./scripts/test-fips-soak-harness.sh
```

The self-test passed with synthetic `pipe` lines. It verifies unit conversion
for queue-wait p95/p99 and proves the threshold failure path fires for
`fmp_worker_queue_wait`.

Short two-sample Docker soak smoke with FIPS commit `4ed5c02` patched in:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_SOAK_DURATION_SECS=12 \
NVPN_SOAK_INTERVAL_SECS=1 \
NVPN_SOAK_PING_COUNT=4 \
NVPN_SOAK_PING_INTERVAL=0.05 \
NVPN_SOAK_IPERF_DURATION_SECS=1 \
NVPN_SOAK_OUTPUT_DIR=artifacts/fips-soak/docker-queue-wait-smoke \
PROJECT_NAME=nostr-vpn-soak-fips-queue-wait-smoke \
./scripts/soak-fips-dataplane-docker.sh
```

Raw log captured locally as `/tmp/nvpn-fips-soak-queue-wait-smoke.log`.
Log SHA-256:
`c63ecc91ed68ebab2ae015a6fdd13f04dd75040d4e14f936e64ee88b8885153c`.

Sample SHA-256:
`38fa74b61bdfb34ce2c7fd0a35b104e8ab13d3535c731c18813b73b46c238cc0`.

The smoke passed with `2` samples. A `jq` check confirmed both samples stayed
on the configured direct UDP path, sample 2 advanced FIPS byte counters on both
nodes, FIPS queue-wait fields were present in both samples, nvpn
`nvpn_tun_to_mesh_queue_wait` fields appeared once the `nvpn-pipe` summary was
emitted, and the maximum parsed queue-wait p95/p99 was `2.1 ms`.

| Sample | Node-a FIPS worker p95/p99 | Node-a FIPS transport p95/p99 | Node-b FIPS worker p95/p99 | Node-b FIPS transport p95/p99 | Node-a nvpn TUN queue p95/p99 | Node-b nvpn TUN queue p95/p99 |
| ---: | --- | --- | --- | --- | --- | --- |
| 1 | 0.5243 / 1.0 ms | 0.2621 / 0.5243 ms | 1.0 / 1.0 ms | 2.1 / 2.1 ms | not emitted yet | not emitted yet |
| 2 | 1.0 / 1.0 ms | 0.5243 / 0.5243 ms | 1.0 / 1.0 ms | 2.1 / 2.1 ms | 0.2621 / 0.5243 ms | 0.2621 / 0.5243 ms |

Follow-up host-pair queue-wait guard after the Docker queue-wait guard in
`da035b54 Gate Docker soak queue wait` brings the host/VM soak pipeline parser
in line with the Docker soak. Host-pair `samples.ndjson` and `failure.json`
now include parsed queue-wait p95/p99/max/allmax for
`fmp_worker_queue_wait`, `transport_queue_wait`, and
`nvpn_tun_to_mesh_queue_wait` whenever `pipe`/`nvpn-pipe` summaries are
available, and clean host-pair soaks fail if those p95/p99 values exceed the
configured thresholds.

Fast harness checks:

```sh
bash -n scripts/soak-fips-dataplane-host-pair.sh
bash -n scripts/test-host-pair-harness.sh
./scripts/test-host-pair-harness.sh
```

The self-test passed. It covers queue-wait unit conversion, threshold failure,
`NVPN_HOST_PAIR_ALLOW_QUEUE_WAIT`, and `samples.ndjson` queue-wait fields. This
is local harness validation only; no host/VM pair run was performed in this
commit.

Follow-up connected-UDP app-config escape hatch after nvpn commit
`dcba1f2b Gate host-pair soak queue wait` exposes optional
`[node.connected_udp]` `enabled` and `fd_reserve` fields in the app config and
lowers explicitly set values into the embedded FIPS endpoint config. Omitted
fields leave FIPS defaults in control.

Focused checks:

```sh
cargo fmt --check
cargo test -p nostr-vpn-core connected_udp --test config_tests
cargo test -p nvpn app_connected_udp_config_reaches_fips_endpoint_config
```

The focused tests passed. This is config/lowering coverage only; no perf,
platform-matrix, host/VM, or real Mac-to-Mac soak was run for this small
escape-hatch step.

Follow-up deterministic Linux smoke after FIPS commit
`bbf72ff Name decrypt worker session keys` names the decrypt-worker session
owner key used by registration, packet jobs, unregister, and registered-session
tracking. The new guard proves registration and priority packet jobs for one
FMP receive session route to the same worker shard.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  decrypt_session_key_routes_registration_and_jobs_to_same_worker )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  test_promote_registers_decrypt_worker )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  decrypt_session_key_routes_registration_and_jobs_to_same_worker )
```

All focused checks passed. This is deterministic ownership-boundary coverage
only; no perf, platform-matrix, host/VM, or real Mac-to-Mac soak was run for
this naming step. On the current branch, the initial session-key guard is
superseded by the unregister-aware guard recorded in the next section.

Follow-up deterministic Linux smoke after FIPS commit
`9865d3e Guard decrypt worker unregister lane` makes decrypt-worker unregister
pressure visible and extends the session-key guard through teardown. Unregister
now uses the same `DecryptSessionKey` shard owner, the bounded priority lane,
and a visible full-lane pressure path.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  decrypt_session_key_routes_registration_jobs_and_unregister_to_same_worker )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core decrypt_worker_unregister )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  test_promote_registers_decrypt_worker )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  decrypt_session_key_routes_registration_jobs_and_unregister_to_same_worker \
  decrypt_worker_unregister_uses_priority_lane_when_bulk_queue_is_full \
  decrypt_worker_unregister_full_returns_false_without_waiting )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

All checks passed. The full default Linux deterministic runner passed with the
new unregister filters in its default list. This is deterministic queue and
ownership-boundary coverage only; no perf, platform-matrix, host/VM, or real
Mac-to-Mac soak was run for this teardown guard.

Follow-up deterministic Linux smoke after FIPS commit
`a024fcb Guard decrypt worker unregister drain order` adds an unregister
drain-order guard. It proves queued unregister removes stale worker-owned
session state before queued bulk decrypt work for the same old session can run.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  decrypt_worker_drain_unregisters_priority_before_bulk_jobs )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core decrypt_worker_drain )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  decrypt_worker_drain_unregisters_priority_before_bulk_jobs )
```

All focused checks passed. This is deterministic stale-state teardown coverage
only; no perf, platform-matrix, host/VM, or real Mac-to-Mac soak was run for
this drain-order guard.

Follow-up deterministic Linux smoke after FIPS commit
`7e5c03d Guard UDP send backpressure pacer` extracts the UDP send backpressure
pacer decision into a deterministic helper and adds coverage for the
socket-buffer failure path. The guard pins `WouldBlock` reset/yield behavior,
ENOBUFS/ENOMEM classification, explicit bulk-drop budgets, and sleep throttling
that does not hide sustained pressure.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core send_backpressure )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  send_backpressure )
```

All focused checks passed. This is deterministic UDP send backpressure coverage
only; no perf, platform-matrix, host/VM, or real Mac-to-Mac soak was run for
this pacer guard.

Follow-up deterministic Linux smoke after FIPS commit
`075a0cf Guard pipelined endpoint wire counters` extracts the pipelined
endpoint-data FMP/FSP wire builder and adds coverage for the pre-reserved
counter/header/offset contract. The guard parses the FMP and FSP headers back
out, checks source/destination/path-MTU datagram fields, and proves the worker
FSP seal offsets point at the AAD header and plaintext tail that the worker will
seal.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  pipelined_endpoint_wire_uses_reserved_counters_and_offsets )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  pipelined_endpoint_wire_uses_reserved_counters_and_offsets )
```

All focused checks passed. This is deterministic FMP/FSP endpoint wire layout
coverage only; no perf, platform-matrix, host/VM, or real Mac-to-Mac soak was
run for this builder guard.

Follow-up deterministic Linux smoke after FIPS commit
`461c17a Guard decrypt worker replay ownership` adds a worker-path FMP replay
ownership guard. The test first sends an invalid AEAD frame and proves the
worker-owned replay window is not consumed, then sends an authentic frame and
proves the replay high-water mark advances, then replays the same counter and
proves no fallback plaintext or failure event is emitted.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  decrypt_worker_accepts_fmp_replay_only_after_aead_success )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core decrypt_worker_ )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  decrypt_worker_accepts_fmp_replay_only_after_aead_success )
```

All focused checks passed. This is deterministic decrypt-worker FMP replay
ownership coverage only; no perf, platform-matrix, host/VM, or real Mac-to-Mac
soak was run for this replay guard.

Follow-up deterministic Linux smoke after FIPS commit
`1a93a16 Guard pipelined send counter ownership` adds a named guard for the
off-task send-counter contract. The test reserves two counters through the
session-owned path, encrypts both packets with a cloned worker-side AEAD, proves
clone-side encryption does not advance the session counter, and then verifies a
subsequent inline send continues from the next expected counter.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  test_pipelined_send_counter_reservation_is_single_owner )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  test_pipelined_send_counter_reservation_is_single_owner )
```

All focused checks passed. This is deterministic send-counter ownership
coverage only; no perf, platform-matrix, host/VM, or real Mac-to-Mac soak was
run for this counter guard.

Follow-up full deterministic Linux runner after FIPS commit
`1a93a16 Guard pipelined send counter ownership` and nvpn commit
`7d0fc906 Record pipelined send counter guard`:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

The full default Linux deterministic runner passed with the current filter
list, including the newer `send_backpressure`,
`pipelined_endpoint_wire_uses_reserved_counters_and_offsets`,
`test_pipelined_send_counter_reservation_is_single_owner`, and
`decrypt_worker_accepts_fmp_replay_only_after_aead_success` guards. This is
deterministic Linux container coverage only; no perf, platform-matrix, host/VM,
or real Mac-to-Mac soak was run for this full-run check.

Follow-up deterministic Linux smoke after FIPS commit
`95382b5 Guard decrypt fallback priority pressure` adds a named guard for
bounded decrypt-worker fallback priority pressure. The test fills the priority
fallback lane, attempts another decrypt-failure/control event from a separate
thread, proves the sender returns pressure instead of parking, and proves bulk
fallback capacity remains separate.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  decrypt_worker_fallback_priority_full_returns_false_without_waiting )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  decrypt_worker_fallback_priority_full_returns_false_without_waiting )
```

All focused checks passed. This is deterministic decrypt fallback priority-lane
pressure coverage only; no perf, platform-matrix, host/VM, or real Mac-to-Mac
soak was run for this fallback guard.

Follow-up deterministic Linux smoke after FIPS commit
`a1e6b13 Prefer priority fallback drain heads` makes the rx-loop fallback drain
head-aware. When the outer select has already received a bulk fallback but a
priority fallback is ready before the drain runs, the helper drains the priority
event first, then the selected bulk head, then queued bulk.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  fallback_drain_prefers_ready_priority_over_selected_bulk )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  fallback_drain_prefers_ready_priority_over_selected_bulk )
```

All focused checks passed. This is deterministic rx-loop fallback drain ordering
coverage only; no perf, platform-matrix, host/VM, or real Mac-to-Mac soak was
run for this drain-order guard.

Follow-up full deterministic Linux runner after FIPS commit
`a1e6b13 Prefer priority fallback drain heads` and nvpn commit
`83b2636c Record fallback drain priority guard`:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

The full default Linux deterministic runner passed with the current filter
list, including
`decrypt_worker_fallback_priority_full_returns_false_without_waiting` and
`fallback_drain_prefers_ready_priority_over_selected_bulk`. This is
deterministic Linux container coverage only; no perf, platform-matrix, host/VM,
or real Mac-to-Mac soak was run for this full-run check.

Follow-up local-FIPS platform matrix smoke after FIPS commit
`a1e6b13 Prefer priority fallback drain heads` and nvpn commit
`600a0ad3 Record full runner after fallback drain guard`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_DURATION_SECS=2 \
NVPN_PLATFORM_MATRIX_LOAD_DURATION_SECS=3 \
NVPN_PLATFORM_MATRIX_PING_COUNT=8 \
NVPN_PLATFORM_MATRIX_RX_MAINT_FAULT_MS=50 \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-a1e6b13-nvpn-600a0ad3-current-smoke \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-a1e6b13-matrix \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The matrix passed all four scenarios: `connected-udp-on`,
`connected-udp-off`, `single-encrypt-worker`, and `tight-backpressure`. Direct
underlay byte counters advanced in every phase, expected explicit bulk pressure
counters remained bounded, and no TCP/ping wedge appeared. Load and post-load
ping loss were `0%` across all scenario phases; notably,
`connected-udp-off` constrained reverse-load ping had `0%` loss in this run.

Summary artifact:
`artifacts/fips-platform-matrix/fips-a1e6b13-nvpn-600a0ad3-current-smoke/summary.tsv`.
Summary SHA-256:
`e8a1378f6cd744f7ac73351feaf56954023dbee0302c9b87cb0fd3e0bc26cc89`.

Phase-summary SHA-256:

- `connected-udp-on`:
  `51cfa68d689579a3803dbc0a216a54b585b3ebe09c80f83474e5c3176a5cb0d9`
- `connected-udp-off`:
  `ee5da9418fc6f7de29c2de6e9a4ad1cb206e69786f92facaba5897f28456a57f`
- `single-encrypt-worker`:
  `cadc55270d7972a6dc7875b5f43908344909b5bf27739cc88d4192b7c155bdb6`
- `tight-backpressure`:
  `c4ad11ddbdbc650f43f38bb9d7bd0d0f86887c4538bb0d8917d58ce9a1b3d69b`

| Scenario | Extra env | Elapsed | Log SHA-256 | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker pressure fwd-load/rev-load Mbps | Rx fault fwd-load/rev-load Mbps | Max post-load ping |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| connected-udp-on | `FIPS_CONNECTED_UDP=1` | 146s | `11fff07ffc1b25cd39469332f610f4ff949293924cc1b5b13d7ad95cf7b34748` | 2294.3 / 2312.8 | 165.4 / 164.9 | 124.2 / 127.3 | 2302.8 / 2326.1 | 5.020 ms |
| connected-udp-off | `FIPS_CONNECTED_UDP=0` | 117s | `1cc7730e436eb038c619516952c8184f62918497047f8950d4e0f0352a314d5a` | 2333.4 / 2330.0 | 169.9 / 168.6 | 147.1 / 135.8 | 2344.2 / 2361.8 | 2.911 ms |
| single-encrypt-worker | `FIPS_CONNECTED_UDP=1 FIPS_ENCRYPT_WORKERS=1 FIPS_DECRYPT_WORKERS=0` | 118s | `9262190d49982e14dae20c10d4818015b846d40a061a3c4940045ceaca50d924` | 2024.6 / 1933.0 | 171.5 / 168.4 | 594.1 / 578.7 | 2006.5 / 2052.6 | 13.221 ms |
| tight-backpressure | `FIPS_CONNECTED_UDP=1 FIPS_WORKER_CHANNEL_CAP=4 FIPS_SEND_BACKPRESSURE_SLEEP_AFTER=1 FIPS_SEND_BACKPRESSURE_SLEEP_MICROS=500 FIPS_SEND_BACKPRESSURE_DROP_AFTER=0` | 117s | `067a9fd82d0aeb9a1e9739ceb02fca3a69fbe86bc0bde9b248f72a1225998b0e` | 127.5 / 130.4 | 124.6 / 127.6 | 129.8 / 125.2 | 125.7 / 125.3 | 6.886 ms |

This is local Linux/docker platform-split evidence only. No host/VM soak or
real Mac-to-Mac Wi-Fi/screenshare validation was run for this platform matrix.

Follow-up deterministic connected-UDP scale guard after FIPS commit
`8db775f Bound connected UDP cap scans` makes the explicit `max_peers` cap-tail
path bounded and testable. When the cap is exhausted, the activation tick now
counts the current plus remaining skipped candidates once, records the existing
`connected_udp_peer_cap_skipped` event, stops walking that candidate tail, and
leaves those peers on wildcard UDP.

Focused checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core \
  connected_udp -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh \
  connected_udp )
```

All focused checks passed. This is deterministic connected-UDP cap/escape-hatch
coverage only; no perf, platform-matrix, host/VM, or real Mac-to-Mac soak was
run for this scale guard.

Follow-up full deterministic Linux runner after FIPS commit
`8db775f Bound connected UDP cap scans` and nvpn commit
`3c206418 Record connected UDP cap guard`:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

The full default Linux deterministic runner passed. The final `connected_udp`
filter included
`peer_cap_skip_count_is_zero_while_budget_remains` and
`peer_cap_skip_count_covers_current_and_remaining_candidates`, along with the
existing connected-UDP config, fd-budget, activation-order, stale-peer, and
rekey-drain guards. This is deterministic Linux container coverage only; no
perf, platform-matrix, host/VM, or real Mac-to-Mac soak was run for this
full-run check.

Follow-up Docker soak harness self-test hardening after nvpn commit
`51aa8db0 Record full runner after connected UDP cap guard` pins hard pipeline
event policy in the local self-test. The parser now treats `event=0/s total=0`
as not seen, while preserving bare `event=0/s` as seen because FIPS interval
logs omit zero-delta events and can round low counts down to `0/s`. The
self-test also proves `connected_udp_peer_cap_skipped` remains observable but
is not a hard queue/drop failure.

Checks:

```sh
bash -n scripts/soak-fips-dataplane-docker.sh
bash -n scripts/test-fips-soak-harness.sh
./scripts/test-fips-soak-harness.sh
```

All checks passed. This is local harness validation only; no Docker soak,
host/VM soak, platform matrix, or real Mac-to-Mac validation was run for this
self-test hardening.

Follow-up short local-FIPS Docker soak smoke after FIPS commit
`8db775f Bound connected UDP cap scans` and nvpn commit
`9810c53b Pin Docker soak hard event policy` exercises the hardened parser path
against a live two-node Docker tunnel:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_SOAK_DURATION_SECS=12 \
NVPN_SOAK_INTERVAL_SECS=1 \
NVPN_SOAK_PING_COUNT=4 \
NVPN_SOAK_PING_INTERVAL=0.05 \
NVPN_SOAK_IPERF_DURATION_SECS=1 \
NVPN_SOAK_OUTPUT_DIR=artifacts/fips-soak/fips-8db775f-nvpn-9810c53b-parser-smoke \
PROJECT_NAME=nostr-vpn-soak-parser-smoke \
./scripts/soak-fips-dataplane-docker.sh
```

Samples:
`artifacts/fips-soak/fips-8db775f-nvpn-9810c53b-parser-smoke/samples.ndjson`.
Sample SHA-256:
`6b69ed4ab01a777526c04981a692611c139c6c413ffa39d6d354ff0121ac3156`.

The smoke passed with `2` samples from `2026-06-08T10:21:28Z` through
`2026-06-08T10:21:34Z`. It used the configured direct Docker underlay paths for
both samples, observed `0%` ping loss both ways, and saw no hard
queue/drop/backpressure events. Connected UDP was active after startup
(`connected_udp_installed`, `udp_send_connected` observed; brief wildcard send
events were startup/transition traffic).

Observed ranges:

- Ping avg: node-a to node-b `0.728-1.237 ms`, node-b to node-a
  `1.383-1.864 ms`
- Ping p95/p99: node-a to node-b `1.170-1.820 ms`, node-b to node-a
  `1.850-3.510 ms`
- Iperf forward: `2253.955-2300.576 Mbps`
- Iperf reverse: `2310.930-2324.722 Mbps`
- Iperf retransmits: forward `35-68`, reverse `43-45`
- FIPS SRTT: node-a `2 ms`, node-b `2 ms`
- Daemon CPU: node-a `41.3-65.4%`, node-b `47.5-73.0%`
- Max observed queue-wait p99: node-a FIPS worker `1.0 ms`, node-a FIPS
  transport `0.5243 ms`, node-b FIPS worker `1.0 ms`, node-b FIPS transport
  `2.1 ms`, nvpn TUN-to-mesh `0.5243 ms` on both nodes

This is short Docker parser/runtime smoke evidence only. It does not replace
the recorded 30-minute Docker soak and does not claim host/VM or real
Mac-to-Mac validation.

Follow-up host-pair hard-event parser hardening after nvpn commit
`3c0b9fed Record parser soak smoke` brings host/VM hard-event detection in line
with the Docker soak parser. The host-pair parser now treats bare
`event=0/s` hard-event tokens as seen, because FIPS interval logs omit
zero-delta events and can round low counts down to `0/s`, while explicit
`event=0/s total=0` remains clean. The self-test pins both cases.

Checks:

```sh
bash -n scripts/soak-fips-dataplane-host-pair.sh
bash -n scripts/test-host-pair-harness.sh
./scripts/test-host-pair-harness.sh
```

All checks passed. This is local host-pair harness validation only; no host/VM
pair run, Docker soak, platform matrix, or real Mac-to-Mac validation was run
for this parser hardening.

Follow-up Docker soak no-progress harness guard after nvpn commit
`fb82b241 Pin host-pair low-rate hard events` pins the local Docker soak
self-test for FIPS byte-counter progress failures. The helper still allows the
first sample with no prior counter and accepts increasing counters, but now the
self-test proves non-numeric counters and repeated counters fail before a long
soak can silently miss a no-progress condition.

Checks:

```sh
bash -n scripts/soak-fips-dataplane-docker.sh
bash -n scripts/test-fips-soak-harness.sh
./scripts/test-fips-soak-harness.sh
```

All checks passed. This is local Docker harness validation only; no Docker soak,
host/VM soak, platform matrix, or real Mac-to-Mac validation was run for this
self-test hardening.

Follow-up Docker soak CPU-runaway harness guard after nvpn commit
`5588f71f Pin Docker soak counter progress guard` pins the local Docker soak
self-test for daemon CPU threshold failures. The runtime harness already records
and gates node-a/node-b daemon CPU with `NVPN_SOAK_MAX_CPU_PERCENT`; this
self-test now proves a sample above that ceiling fails before a long soak can
quietly report CPU runaway only as an observation.

Checks:

```sh
bash -n scripts/soak-fips-dataplane-docker.sh
bash -n scripts/test-fips-soak-harness.sh
./scripts/test-fips-soak-harness.sh
```

All checks passed. This is local Docker harness validation only; no Docker soak,
host/VM soak, platform matrix, or real Mac-to-Mac validation was run for this
self-test hardening.

Follow-up host-pair no-progress/CPU harness guard after nvpn commit
`049c967c Pin Docker soak CPU guard` pins the local host/VM soak self-test for
two long-run failure detectors: FIPS byte-counter no-progress after the first
sample and daemon CPU above `NVPN_HOST_PAIR_MAX_CPU_PERCENT`. The runtime
harness already fails those conditions during host-pair soaks; this self-test
now proves the helper failure paths stay wired while remaining source-safe and
remote-free.

Checks:

```sh
bash -n scripts/soak-fips-dataplane-host-pair.sh
bash -n scripts/test-host-pair-harness.sh
./scripts/test-host-pair-harness.sh
```

All checks passed. This is local host-pair harness validation only; no host/VM
pair run, Docker soak, platform matrix, or real Mac-to-Mac validation was run
for this self-test hardening.

Follow-up Docker perf harness guard after nvpn commit
`f9ff77d8 Pin host-pair no-progress CPU guards` extends the source-safe perf
self-test beyond ping percentiles. It now pins iperf throughput and retransmit
parsing, TCP throughput floor failures, and direct-underlay byte-counter
progress failures. That keeps the local helper test aligned with the Docker perf
gate's two core pass/fail signals for TCP collapse and silent path changes.

Checks:

```sh
bash -n scripts/e2e-fips-perf-regression-docker.sh
bash -n scripts/test-fips-perf-harness.sh
./scripts/test-fips-perf-harness.sh
```

All checks passed. This is local perf harness validation only; no Docker perf
run, Docker soak, host/VM pair run, platform matrix, or real Mac-to-Mac
validation was run for this self-test hardening.

Follow-up architecture-plan evidence map after nvpn commit
`6cf8dbac Pin perf harness path throughput guards` updates
`docs/fips-dataplane-architecture-plan.md` with the current source-safe helper
self-tests and a pre-refactor readiness map. The map ties the planned
small-step architecture work to concrete evidence for queue pressure, priority
traffic, TCP/backpressure, direct-route sanity, MMP robustness, and long-run
degradation detection, while preserving the no-real-Mac-to-Mac boundary.

Checks:

```sh
cargo fmt --check
git diff --check
```

All checks passed. This is documentation/evidence-map hardening only; no Docker
perf run, Docker soak, host/VM pair run, platform matrix, or real Mac-to-Mac
validation was run for this update.

Follow-up deterministic Linux runner wiring after FIPS commit
`1a77baf Run endpoint lane helper in safety suite` adds
`endpoint_command_tx_helper_classifies_priority_and_bulk_payloads` to the
default Linux safety runner. The helper test already existed; it now runs in the
same deterministic suite as the lower-level endpoint payload classifier, so the
endpoint command send path keeps ACK/control-shaped payloads on the reserved
priority command lane and large endpoint payloads on the bounded bulk lane.

Checks:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && bash -n scripts/test-dataplane-safety-linux-docker.sh )
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core endpoint_command_tx_helper_classifies_priority_and_bulk_payloads )
( cd "$FIPS_SAFETY_WORKTREE" && cargo fmt --check )
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh endpoint_command_tx_helper_classifies_priority_and_bulk_payloads )
```

All checks passed. This is focused deterministic endpoint lane coverage only;
no full deterministic runner, Docker perf run, Docker soak, host/VM pair run,
platform matrix, or real Mac-to-Mac validation was run for this runner wiring.

Follow-up full deterministic Linux runner after FIPS commit
`1a77baf Run endpoint lane helper in safety suite` and nvpn commit
`7a5f2fd1 Record endpoint lane runner guard`:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

The full default Linux deterministic runner passed. The default filter list now
includes `endpoint_command_tx_helper_classifies_priority_and_bulk_payloads`
alongside the queue/backpressure, route, MMP, connected-UDP, ownership,
fallback, and pending-session guards. The final `connected_udp` filter passed
`12` tests, including the connected-UDP cap-tail guards and stale-peer/rekey
drain coverage.

This is deterministic Linux container coverage only; no Docker perf run, Docker
soak, host/VM pair run, platform matrix, or real Mac-to-Mac validation was run
for this full-run check.

Follow-up default-duration local-FIPS platform matrix after FIPS commit
`1a77baf Run endpoint lane helper in safety suite` and nvpn commit
`518245f9 Record full runner after endpoint lane guard`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-1a77baf-nvpn-518245f9-current-smoke \
./scripts/e2e-fips-platform-matrix-docker.sh
```

This ran the default matrix knobs rather than the shorter smoke durations. The
matrix passed `connected-udp-on`, `connected-udp-off`, and
`single-encrypt-worker`, then failed `tight-backpressure` during the
clean-underlay phase:

```text
fips perf regression e2e failed: clean-underlay reverse TCP throughput Mbps 88.7 below minimum 100.0
```

The failure appeared while decrypt worker pressure was explicit in the logs
(`decrypt_worker_queue_full`, `decrypt_worker_bulk_dropped`, and
`decrypt_fallback_bulk_dropped`). Treat this as current pre-refactor safety-net
evidence: the default-duration platform matrix is wired, reproducible, and not
green for the tight backpressure case.

Summary artifact:
`artifacts/fips-platform-matrix/fips-1a77baf-nvpn-518245f9-current-smoke/summary.tsv`.
Summary SHA-256:
`d9668e31512745101a21202702dedda21f26e60189c3108a263df92e77e7a301`.

Phase-summary SHA-256:

- `connected-udp-on`:
  `7b0eedb4b13515544488633fedd3320278e09d73a35783e7841e4c469ba9565b`
- `connected-udp-off`:
  `06bcd9170c1a5f16f58a77bc9aa2ce52b88f44e125e70229fd2a98177a6122e4`
- `single-encrypt-worker`:
  `720b60c0f95a34338b35a838019596e5a6d5c44c3375eb881c74342eb3f530c6`
- `tight-backpressure`:
  `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78`
  (header-only because the scenario failed before a complete phase row was
  written)

| Scenario | Status | Elapsed | Log SHA-256 | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker pressure fwd-load/rev-load Mbps | Rx fault fwd-load/rev-load Mbps | Max post-load ping |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| connected-udp-on | pass | 192s | `6f00ad2f403570065455509e391a2df83c077423981dffb11352444fc3a14b8e` | 2228.4 / 2332.7 | 172.6 / 172.9 | 128.0 / 128.1 | 2242.0 / 2239.0 | 4.976 ms |
| connected-udp-off | pass | 164s | `2d042d776123d37348fcfdd6e0fa4cae706b9614a86ad5dd1caab2964c10edc6` | 2280.9 / 2374.7 | 174.3 / 171.8 | 143.4 / 150.6 | 2322.5 / 2227.5 | 5.268 ms |
| single-encrypt-worker | pass | 161s | `8d21ad83ebc2872327bb706edd28b97a5889006411e84b8570e660f1c5c01950` | 1977.2 / 1975.5 | 174.7 / 172.6 | 367.6 / 530.5 | 1920.3 / 1942.3 | 4.520 ms |
| tight-backpressure | fail | 32s | `51e51aa38d069ef039a87240e05fa9615f29640bc4154d83fa819ab766b53fb4` | 130.1 / 88.7 | n/a | n/a | n/a | n/a |

For the three passing scenarios, every completed phase advanced direct Docker
underlay byte counters on both nodes and load/post-load ping loss stayed at
`0%`. The failing `tight-backpressure` scenario failed before a completed phase
summary row, but its debug output included direct underlay counter rules for
both nodes and explicit bulk-drop pressure events.

Targeted `tight-backpressure` rerun after the documentation commit:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-1a77baf-nvpn-d5a734fc-tight-backpressure-rerun1 \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-tight-rerun1 \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The rerun failed the same scenario during clean-underlay, but at the reverse
load step rather than the first reverse TCP step:

```text
clean-underlay forward TCP: 126.0 Mbps retrans=909
clean-underlay reverse TCP: 119.7 Mbps retrans=974
clean-underlay forward TCP load: 121.8 Mbps retrans=1276
clean-underlay during forward TCP load ping: loss=0% avg=0.317ms p95=0.458ms p99=0.462ms max=0.462ms
clean-underlay reverse TCP load: 75.3 Mbps retrans=1143
fips perf regression e2e failed: clean-underlay reverse TCP throughput Mbps 75.3 below minimum 100.0
```

Summary artifact:
`artifacts/fips-platform-matrix/fips-1a77baf-nvpn-d5a734fc-tight-backpressure-rerun1/summary.tsv`.
Summary SHA-256:
`1fcd5774c4b73742cff2e6307634c46fcb23560886c066e0a04760e6ec518327`.
Log SHA-256:
`c3c222936d91ce19aba6b3f3113cebae32e202f284226447193dbf1c9a177ffb`.

The targeted rerun again emitted explicit decrypt bulk and fallback-bulk
pressure, and failed before a complete phase-summary row was written. This
reproduces the tight-backpressure weakness enough to keep it as a blocking
pre-refactor regression target instead of relaxing the floor.

Follow-up FIPS fix after commit
`33f840e Split decrypt fallback cap from worker cap` keeps
`FIPS_WORKER_CHANNEL_CAP` scoped to worker admission and leaves the
worker-to-rx-loop fallback bulk lane on its own explicit
`FIPS_DECRYPT_FALLBACK_CHANNEL_CAP` knob. The shared worker cap still forces
explicit worker bulk drops in tight backpressure, but it no longer also shrinks
the fallback return lane to four slots.

Targeted validation command:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-fallback-cap-split-nvpn-d9803083-tight-backpressure \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-fallback-cap \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The targeted `tight-backpressure` scenario passed all four phases. Direct
Docker underlay byte counters advanced in every phase, load/post-load ping loss
stayed at `0%`, and expected `decrypt_worker_queue_full` /
`decrypt_worker_bulk_dropped` events remained visible without
`decrypt_fallback_bulk_dropped` in the captured phase summaries.

Summary artifact:
`artifacts/fips-platform-matrix/fips-fallback-cap-split-nvpn-d9803083-tight-backpressure/summary.tsv`.
Summary SHA-256:
`e534f55204f750736d708a015432809b43eac404b7ca75b6ace021433b431db4`.
Phase-summary SHA-256:
`6a3cc74f3d68a89a0d6ceaa717eb8d22b5c4f1548d190ee519265f095678e7c6`.
Log SHA-256:
`e210f87340bf9b7b8ccfcc993c4353d3e3d744e74328ba31bb45741e13e89cae`.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | 121.6 | 126.6 | 122.5 | 128.3 | 0.428 / 0.727 / 0.727 ms | 0.397 / 0.675 / 0.675 ms | 2.050 / 3.570 / 3.569 ms | 222786774 / 233876818 |
| constrained-underlay | 121.3 | 133.8 | 131.9 | 128.9 | 0.523 / 0.589 / 0.589 ms | 0.369 / 0.412 / 0.412 ms | 1.970 / 2.090 / 2.089 ms | 233492825 / 238309483 |
| worker-queue-pressure | 122.9 | 133.0 | 120.3 | 130.1 | 0.391 / 0.575 / 0.575 ms | 0.300 / 0.383 / 0.383 ms | 2.240 / 3.050 / 3.051 ms | 221361297 / 235484518 |
| rx-maintenance-fault | 121.4 | 122.6 | 126.8 | 129.3 | 0.305 / 0.306 / 0.306 ms | 0.487 / 0.570 / 0.570 ms | 5.060 / 5.520 / 5.524 ms | 227916592 / 233547309 |

This is local Linux/docker platform-split evidence only. No Docker soak,
host/VM pair run, or real Mac-to-Mac Wi-Fi/screenshare validation was run for
this platform matrix.

Follow-up broad validation after FIPS commit `33f840e` and nvpn commit
`2130027a`:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

The full default FIPS Linux deterministic runner passed with the fallback-cap
split guard in the default filter list. This is deterministic Linux container
coverage only; no perf, platform-matrix, host/VM pair, Docker soak, or real
Mac-to-Mac validation is implied by this runner.

Full local-FIPS Docker platform matrix command:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-33f840e-nvpn-2130027-current-smoke \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-33f840e-matrix \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The full matrix did not pass. `connected-udp-on`, `connected-udp-off`, and
`single-encrypt-worker` passed all four phases, with direct Docker underlay byte
counters advancing and `0%` ping loss in the load/post-load checks. The final
`tight-backpressure` scenario failed before a complete phase-summary row was
written: clean-underlay forward TCP measured `75.0 Mbps` against the `100 Mbps`
floor. The saved logs show expected `decrypt_worker_queue_full` /
`decrypt_worker_bulk_dropped` pressure in `tight-backpressure`, but no
`decrypt_fallback_bulk_dropped` events. Treat this as a remaining
matrix/order-sensitive safety-net failure, not as a green broad baseline.

Summary artifact:
`artifacts/fips-platform-matrix/fips-33f840e-nvpn-2130027-current-smoke/summary.tsv`.
Summary SHA-256:
`093cb97f4577fc32f15e488f336039e2cc2156272dd3a82650e1cf87ef8eceed`.

Phase-summary SHA-256:

- `connected-udp-on`:
  `803ea92b711c6a2770bdd78d2a4a0e3ca7ee884a2fa74bec9be3ee19d1b73341`
- `connected-udp-off`:
  `89d56070a95425d365550a8693c68d10f039d7b202e46f4cd95d467e6f97bbc0`
- `single-encrypt-worker`:
  `03e669ce0ba457d85ba0fee6291c7a530fc3e429cccea28570ed71cf21d3c0f6`
- `tight-backpressure`:
  `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78`
  (header-only because the scenario failed before a completed phase row)

| Scenario | Status | Elapsed | Log SHA-256 | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker-pressure fwd/rev Mbps | Rx-maint fwd/rev Mbps |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: |
| connected-udp-on | pass | 201s | `765803896b44760ed25462f69ff17f757ddd7bb5c127abb7739daa921b7f163d` | 2148.5 / 2115.3 | 174.0 / 173.2 | 129.2 / 139.1 | 2213.9 / 2151.6 |
| connected-udp-off | pass | 191s | `07a3e47a508f7b32180bb1e51e9c0d17be18da275cb91175f9ba3bc5fbb884f6` | 2341.5 / 2278.4 | 176.2 / 169.0 | 137.1 / 149.8 | 2353.8 / 2304.0 |
| single-encrypt-worker | pass | 161s | `eda04d9de7492b7f969b481da3196c853930fbfd69c574a4fd3a2f929a2b1cba` | 1929.4 / 1928.4 | 173.8 / 173.4 | 559.7 / 533.6 | 1648.3 / 2007.1 |
| tight-backpressure | fail | 28s | `6ac033f90b304e4735a1abdc51aea7fce40a8402815ced68483c3caf75f65fe7` | 75.0 / n/a | n/a | n/a | n/a |

This is local Linux/docker platform-matrix evidence only. No Docker soak,
host/VM pair run, or real Mac-to-Mac Wi-Fi/screenshare validation was run for
this matrix.

Follow-up failure-summary harness wiring after nvpn commit `6584a30b`:

The perf gate now creates `failure-summary.tsv` beside `phase-summary.tsv` when
`NVPN_PERF_OUTPUT_DIR` is set. Threshold assertions append
`label`, `comparison`, `actual`, and `threshold` before exiting, so a failed
phase leaves a structured record even when the normal phase row is incomplete.
The platform matrix appends a `failure_summary` path at the end of each
`summary.tsv` row while preserving the existing field prefix.

Syntax checks:

```sh
bash -n scripts/e2e-fips-perf-regression-docker.sh \
  scripts/e2e-fips-platform-matrix-docker.sh
```

Fast helper-level failure smoke:

```sh
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/failure-summary-helper-smoke \
bash -lc 'source scripts/e2e-fips-perf-regression-docker.sh; init_phase_summary; assert_float_at_least 75.0 100.0 "clean-underlay forward TCP throughput Mbps"'
```

The helper exited `1` as expected and wrote:

```tsv
label	comparison	actual	threshold
clean-underlay forward TCP throughput Mbps	>=	75.0	100.0
```

Failure-summary SHA-256:
`4bcfc814fb93c957e22dc9dd2fee9edfe1a59aa6f9fa4c165d6647c445ffac5e`.
Raw log captured locally as `/tmp/nvpn-fips-failure-summary-helper-smoke.log`.
Log SHA-256:
`f5d1f58606d19fa13806a371cf534603bb6e1c8404e1cfe431640002c6364c92`.

Platform wrapper field smoke:

```sh
NVPN_PLATFORM_MATRIX_RUNNER=/usr/bin/false \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/failure-summary-wrapper-smoke \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-failure-summary-wrapper \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The wrapper exited `1` as expected. Its `summary.tsv` has `10` fields, with
`failure_summary` as the final header and the failed scenario row pointing at
`tight-backpressure-perf/failure-summary.tsv`.

Summary SHA-256:
`65c84e1698d909fc28493575d544b590853ce5fb93621e0d873b99f66b08d8b1`.
Raw log captured locally as `/tmp/nvpn-fips-failure-summary-wrapper-smoke.log`.
Log SHA-256:
`7375023241aa1e895043dac4b7232b42822751600c229493bbeb65e682d5c74c`.

Short real Docker failure smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/failure-summary-real-smoke \
NVPN_PERF_DURATION_SECS=1 \
NVPN_PERF_LOAD_DURATION_SECS=1 \
NVPN_PERF_PING_COUNT=3 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=0 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=0 \
NVPN_PERF_RX_MAINT_FAULT_MS=0 \
NVPN_PERF_MIN_TCP_MBIT=999999 \
PROJECT_NAME=nostr-vpn-e2e-fips-failure-summary-real-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

The command intentionally failed on the impossible clean-underlay forward TCP
floor and wrote:

```tsv
label	comparison	actual	threshold
clean-underlay forward TCP throughput Mbps	>=	974.4929178018897	999999
```

Failure-summary SHA-256:
`98a0418e1c34162f943e0dc5a7c24286b00dfa1390ed83eb49b43c09815a1974`.
Raw log captured locally as `/tmp/nvpn-fips-failure-summary-real-smoke.log`.
Log SHA-256:
`0598333f9b8b1a9d3ad0935cb3fd31c5cba2b1c14d7963106609ddf8e5f025b3`.

This is harness-observability coverage only. It does not replace a green perf
run, Docker soak, host/VM pair run, or real Mac-to-Mac validation.

Follow-up order-sensitive platform probe after nvpn commit `0c498368`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=single-encrypt-worker,tight-backpressure \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-33f840e-nvpn-0c498368-order-probe-single-then-tight \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-order-probe \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The order probe reproduced a narrower red case with the failure-summary harness
active. `single-encrypt-worker` passed all four phases. The following
`tight-backpressure` scenario passed clean-underlay, constrained-underlay, and
worker-queue-pressure, then failed `rx-maintenance-fault` forward TCP at
`17.1 Mbps` against the `100 Mbps` floor. The completed phases all advanced
direct Docker underlay byte counters and had `0%` ping loss in load/post-load
checks. Expected `decrypt_worker_queue_full` / `decrypt_worker_bulk_dropped`
events were visible; `decrypt_fallback_bulk_dropped` did not appear in the logs.

Summary artifact:
`artifacts/fips-platform-matrix/fips-33f840e-nvpn-0c498368-order-probe-single-then-tight/summary.tsv`.
Summary SHA-256:
`9dbb40504d6176510ffcf460067739a1de54dc3fb14321d022cb1234048654ec`.

Phase-summary SHA-256:

- `single-encrypt-worker`:
  `6bce0b364bedebb16178800f6ecc11e066984869925a472c5c475946c70bf560`
- `tight-backpressure`:
  `1653121e421a638085c7b3e883af4d262d04a748c1dbca22628f80a040859ec8`

Failure-summary SHA-256:

- `single-encrypt-worker`:
  `bb2c45d65f465142e576533bc4a25259d294ef43b526b73b9817c3fd726235ed`
  (header-only; scenario passed)
- `tight-backpressure`:
  `fa43d8c889408f144c477766d45e5a0f71cb84fafe57faf248bbc3ab889ca712`

The `tight-backpressure` failure summary captured:

```tsv
label	comparison	actual	threshold
rx-maintenance-fault forward TCP throughput Mbps	>=	17.104032820902102	100
```

| Scenario | Status | Elapsed | Log SHA-256 | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker-pressure fwd/rev Mbps | Rx-maint fwd/rev Mbps |
| --- | --- | ---: | --- | ---: | ---: | ---: | ---: |
| single-encrypt-worker | pass | 163s | `e5eff25a25600388369adf9d8c264d9dd717f8b63eea11f62a33d8b0d61fcda4` | 1902.7 / 1822.7 | 151.8 / 164.7 | 199.5 / 99.9 | 1738.1 / 1878.9 |
| tight-backpressure | fail | 145s | `a83fbfb0b9b09978d9e6d1596cc1439ac27e10f4c72b9df270635a666738dba9` | 119.0 / 112.6 | 115.5 / 112.1 | 107.2 / 114.0 | 17.1 / n/a |

This is local Linux/docker platform-matrix evidence only. It does not replace a
Docker soak, host/VM pair run, or real Mac-to-Mac validation. It preserves a
concrete pre-refactor target: tight worker/send backpressure plus rx-loop
maintenance delay must degrade recoverably instead of collapsing.

Follow-up phase-selector harness wiring after nvpn commit `ae422764`:

The perf gate now accepts `NVPN_PERF_PHASES=<csv>` and the platform matrix
passes `NVPN_PLATFORM_MATRIX_PHASES=<csv>` through to each perf run. Known
phases are `clean-underlay`, `constrained-underlay`, `worker-queue-pressure`,
and `rx-maintenance-fault`; unknown or empty selections fail fast.

Validation checks:

```sh
bash -n scripts/e2e-fips-perf-regression-docker.sh \
  scripts/e2e-fips-platform-matrix-docker.sh

bash -lc 'source scripts/e2e-fips-perf-regression-docker.sh; PERF_PHASES=" clean-underlay, rx-maintenance-fault "; validate_perf_phases; phase_enabled clean-underlay; phase_enabled rx-maintenance-fault; ! phase_enabled constrained-underlay'

bash -lc 'source scripts/e2e-fips-perf-regression-docker.sh; PERF_PHASES="clean-underlay,nope"; validate_perf_phases'

bash -lc 'source scripts/e2e-fips-perf-regression-docker.sh; PERF_PHASES=" , "; validate_perf_phases'
```

The valid selector check passed. The unknown-phase check exited `2` as expected
with `NVPN_PERF_PHASES contains unknown phase: nope`. The empty-selection check
also exited `2` as expected with `NVPN_PERF_PHASES must include at least one
known phase`.

Targeted local-FIPS Docker probe:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_PHASES=clean-underlay,rx-maintenance-fault \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-33f840e-nvpn-ae422764-tight-rx-phase-selector \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-tight-rx-selector \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The probe failed before a completed phase row was written. The failure summary
captured:

```tsv
label	comparison	actual	threshold
clean-underlay forward TCP throughput Mbps	>=	87.86373271409107	100
```

Artifacts:

- Summary:
  `artifacts/fips-platform-matrix/fips-33f840e-nvpn-ae422764-tight-rx-phase-selector/summary.tsv`
- Scenario log SHA-256:
  `cd14e1fab593b30100b42f66df48824ab036b2d7527ea1642b209b6a6db01256`
- Summary SHA-256:
  `0f5ac839aff25acda7b1422bc0ccac18b2c86ba771eed3f00acb1f7bef34cc26`
- Failure-summary SHA-256:
  `28198609ae3e599b881760e5879a435688c7d16cb857b1b0b677c7cf54f5a5b5`
- Phase-summary SHA-256:
  `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78`
  (header-only because the clean-underlay threshold abort happened before a
  completed row)

This is local Linux/docker platform-matrix evidence only. It does not replace a
Docker soak, host/VM pair run, or real Mac-to-Mac validation. Treat it as a
targeted red-case probe proving the phase selector can isolate tight-profile
failures and preserve machine-readable threshold evidence.

Follow-up failed-phase context harness wiring on top of nvpn commit `466a3e0c`:

The perf gate keeps the existing `failure-summary.tsv` field prefix
(`label`, `comparison`, `actual`, `threshold`) and appends phase/step plus the
latest known throughput, retransmit, ping, direct-byte, and pipeline-summary
context. This is specifically for threshold aborts that happen before a complete
`phase-summary.tsv` row is written.

Checks:

```sh
bash -n scripts/e2e-fips-perf-regression-docker.sh \
  scripts/e2e-fips-platform-matrix-docker.sh \
  scripts/test-fips-perf-harness.sh

./scripts/test-fips-perf-harness.sh
```

The helper suite passed, including the new failure-summary context field-count
and content check.

Short real Docker failure smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/failure-context-real-smoke \
NVPN_PERF_PHASES=clean-underlay \
NVPN_PERF_DURATION_SECS=1 \
NVPN_PERF_LOAD_DURATION_SECS=1 \
NVPN_PERF_PING_COUNT=3 \
NVPN_PERF_MIN_TCP_MBIT=999999 \
PROJECT_NAME=nostr-vpn-e2e-fips-failure-context-real-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

The smoke intentionally failed on the impossible clean-underlay forward TCP
floor and wrote a `21`-field failure row. Key fields:

```tsv
label	comparison	actual	threshold	phase	step	forward_mbps	forward_retrans
clean-underlay forward TCP throughput Mbps	>=	1688.0245103612526	999999	clean-underlay	forward TCP	1688.0245103612526	0
```

The same row recorded direct byte deltas before abort:

- node-a direct bytes: `439598152`
- node-b direct bytes: `4972382`

It also captured the latest `[pipe 5s]` summary line from both nodes. The phase
summary remained header-only because the deliberate threshold abort happened
before the completed phase append.

Artifacts:

- Failure-summary:
  `artifacts/fips-perf/failure-context-real-smoke/failure-summary.tsv`
- Failure-summary SHA-256:
  `41869185f3aa610d6256f7780c610887d13a6cd9a37131fa18391f34468f62eb`
- Phase-summary SHA-256:
  `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78`

This is harness-observability coverage plus one deliberate local Linux/docker
failure smoke. It does not replace a green perf run, platform matrix, Docker
soak, host/VM pair run, or real Mac-to-Mac validation.

Follow-up local-FIPS tight-profile probes after nvpn commit `80c404f5`:

First, a narrow rx-maintenance-only probe exercised the tight backpressure
scenario without the earlier clean/constrained/worker phases:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_PHASES=rx-maintenance-fault \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-33f840e-nvpn-80c404f5-tight-rx-context \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-tight-rx-context \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The narrow probe passed. This suggests the earlier rx-maintenance collapse is
not reproduced by a fresh-start rx-maintenance phase alone.

- Summary SHA-256:
  `d20e3ae8f9b252a0c84e6990691afb32960138ec113d04f340f14774826ded2a`
- Scenario log SHA-256:
  `83bec1d22897465c442e694645286e538e44e2727b6c665ddda4397d8b04ea04`
- Phase-summary SHA-256:
  `5f9dd4536cb7f3cdfa9916409c0d199e55e407523550ac6fac0961ce43379bfd`
- Failure-summary SHA-256:
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
  (header-only; scenario passed)

| Scenario | Phase | Fwd/rev Mbps | Fwd/rev load Mbps | Post ping p99 | Direct bytes node-a/node-b |
| --- | --- | ---: | ---: | ---: | ---: |
| tight-backpressure | rx-maintenance-fault | 123.0 / 130.4 | 120.0 / 122.4 | 2.88 ms | 222158259 / 226404723 |

Then, the order-sensitive probe ran the passing single-worker scenario before
the tight backpressure scenario:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=single-encrypt-worker,tight-backpressure \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-33f840e-nvpn-80c404f5-order-context \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-order-context \
./scripts/e2e-fips-platform-matrix-docker.sh
```

`single-encrypt-worker` passed all four phases. `tight-backpressure` failed
before a completed phase row, during clean-underlay forward TCP:

```tsv
label	comparison	actual	threshold	phase	step	forward_mbps	forward_retrans
clean-underlay forward TCP throughput Mbps	>=	92.19233531964328	100	clean-underlay	forward TCP	92.19233531964328	752
```

The extended failure row had `21` fields and recorded direct byte deltas:

- node-a direct bytes: `73073050`
- node-b direct bytes: `3219714`

The captured pipeline context included `decrypt_worker_queue_full` and
`decrypt_worker_bulk_dropped`; `decrypt_fallback_bulk_dropped` did not appear in
the scanned artifacts. This keeps the remaining pre-refactor target focused on
tight worker/send backpressure and clean-underlay TCP recovery, not on the
fallback-lane cap that FIPS `33f840e` already split out.

Order-probe artifacts:

- Summary:
  `artifacts/fips-platform-matrix/fips-33f840e-nvpn-80c404f5-order-context/summary.tsv`
- Summary SHA-256:
  `26ab5d6357de7bbfc05b0fac32306b5e90af1d5461c8640286682a33d0ca0173`
- `single-encrypt-worker` log SHA-256:
  `3179bb0867e35cebde95febe5374f6a0235b584b0a55164d935970de345d3425`
- `single-encrypt-worker` phase-summary SHA-256:
  `784b5f8e32f88d5069998de430ddf780d737b27ddc2d26fc80fb5e0d0a4a7173`
- `single-encrypt-worker` failure-summary SHA-256:
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
  (header-only; scenario passed)
- `tight-backpressure` log SHA-256:
  `39657c92b54b011b4d1363c5adbf32542881d1c444439236f369fb98ab9e58fb`
- `tight-backpressure` failure-summary SHA-256:
  `993b202dabf8ff8b5e0a50c03c51bf4a8bca30e59c57a3bc130178b6c1f63119`
- `tight-backpressure` phase-summary SHA-256:
  `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78`
  (header-only because the clean-underlay threshold abort happened before a
  completed row)

| Scenario | Status | Elapsed | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker-pressure fwd/rev Mbps | Rx-maint fwd/rev Mbps |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| single-encrypt-worker | pass | 194s | 1686.7 / 1589.2 | 174.7 / 173.7 | 437.0 / 405.6 | 779.3 / 595.0 |
| tight-backpressure | fail | 28s | 92.2 / n/a | n/a | n/a | n/a |

This is local Linux/docker platform-matrix evidence only. It does not replace a
Docker soak, host/VM pair run, or real Mac-to-Mac validation.

## Tight Send-Backpressure Isolation Probe

FIPS `33f840e` and nvpn `d34f8c9b` then ran two focused clean-underlay probes
to separate decrypt-worker admission pressure from encrypt/send backpressure.
Both used local-FIPS Docker patching and selected only the clean-underlay phase.

First, the existing `tight-backpressure` profile was run with an explicit
decrypt-worker cap of `8`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_PHASES=clean-underlay \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-33f840e-nvpn-d34f8c9b-tight-send-cap8-clean \
NVPN_PLATFORM_MATRIX_EXTRA_ENV='FIPS_DECRYPT_WORKER_CHANNEL_CAP=8' \
./scripts/e2e-fips-platform-matrix-docker.sh
```

That still failed before a completed phase row, with clean-underlay forward TCP
at `27.5 Mbps` against the `100 Mbps` floor. The failure row recorded `511`
forward retransmits, direct-byte deltas of `24886475` / `812693`, and
`decrypt_worker_queue_full` / `decrypt_worker_bulk_dropped` in node-b pipeline
context.

Cap-8 artifacts:

- Summary SHA-256:
  `3443d9acf1a4349d3852d7e6fe665ba93ff7af47fc8b8ed94c3d5bc910ad0e13`
- Scenario log SHA-256:
  `9d45cfb671b761483d54c2c9752d9d8336d5b52d91b4d423a17cf2a49324e607`
- Failure-summary SHA-256:
  `9d323b54349692069dfdc11b3c12a17e9ffcd5cb389a02f14b5672d0ef5ed7b2`

Second, decrypt-worker admission was restored to the production-sized default
while leaving the shared worker channel and send-backpressure knobs tight:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-backpressure \
NVPN_PLATFORM_MATRIX_PHASES=clean-underlay \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-33f840e-nvpn-d34f8c9b-tight-send-decrypt-default-clean \
NVPN_PLATFORM_MATRIX_EXTRA_ENV='FIPS_DECRYPT_WORKER_CHANNEL_CAP=32768' \
./scripts/e2e-fips-platform-matrix-docker.sh
```

This made the failure cleaner: the initial clean forward/reverse TCP legs passed
at `455.8` / `378.6 Mbps`, then the concurrent load leg failed at `93.4 Mbps`
against the `100 Mbps` floor with `1136` retransmits. Direct-byte deltas were
`433294897` / `299686172`, and the captured pipeline context showed
`encrypt_worker_queue_full` / `encrypt_worker_bulk_dropped` without decrypt bulk
drops in the failed row. This motivated the dedicated
`tight-send-backpressure` matrix scenario, which sets
`FIPS_DECRYPT_WORKER_CHANNEL_CAP=32768` by default while preserving the tight
shared worker and send-backpressure knobs.

Default-decrypt-admission artifacts:

- Summary SHA-256:
  `798f3fa5d504e63069bcda19cf7232bded1d30744f30b453f6ffd66b8cfb55a1`
- Scenario log SHA-256:
  `d98c36ef0979028718f49b6bac03f9d7ad18fafece08a01b177cee797700a7ef`
- Failure-summary SHA-256:
  `e33fc6c11b427584f0bf597044a893b91629b729cca66ab760b5ab192af68e7a`

## Encrypt Priority-Reserve Follow-Up

FIPS `682ba9f` adds a non-macOS encrypt-worker priority reserve while keeping
bulk fair admission bounded. The full default FIPS Linux deterministic runner
passed after adding `priority_flow_enters_when_bulk_worker_queue_is_full` to
the default filter list:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && ./scripts/test-dataplane-safety-linux-docker.sh )
```

The focused send-side pressure probe then re-ran the dedicated
`tight-send-backpressure` matrix row against nvpn `c0b30b9b`, patched to the
local FIPS worktree, and selected only `clean-underlay`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-send-backpressure \
NVPN_PLATFORM_MATRIX_PHASES=clean-underlay \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-682ba9f-nvpn-c0b30b9b-tight-send-clean \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-prio-split \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The scenario passed in `99s`. It still recorded intentional
`encrypt_worker_queue_full` / `encrypt_worker_bulk_dropped` pressure, but TCP
stayed above the `100 Mbps` floor, load/post-load ping loss stayed at `0%`, and
direct Docker underlay byte counters advanced on both nodes.

Artifacts:

- Summary SHA-256:
  `74794403d9d09a7f3f06e551956bd95d9ed04cab5efc0c5df2b5c4e09dc5fd94`
- Phase-summary SHA-256:
  `e5a5d33af09ec45c775d648b495bfc28a0435c1535c3e8ddd2fe3f89a3e7d1aa`
- Scenario log SHA-256:
  `266c9da215c88fe8153a2afb2328b8703369cfa3f4e4dd13533beb11c3c3eace`

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | 340.7 | 295.5 | 284.6 | 324.0 | 0.437 / 0.456 / 0.456 ms | 3.120 / 3.470 / 3.465 ms | 0.618 / 0.697 / 0.697 ms | 533428378 / 550975668 |

Full local-FIPS platform matrix at nvpn `50e7361e`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-682ba9f-nvpn-50e7361e-full-matrix \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-682ba9f-full \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The full matrix stayed red. `connected-udp-on`, `connected-udp-off`, and
`single-encrypt-worker` passed all four phases. The isolated send-pressure row
failed clean-underlay reverse-load ping loss at `5%` against the `2%` ceiling
while TCP stayed above the floor, and the legacy combined tight-pressure row
failed clean-underlay forward TCP at `73.0 Mbps` against the `100 Mbps` floor.

Summary SHA-256:
`4bd5ebe090f020c598088bccd96cb71fb87cdfd5ace0ccf253559dc2608ec8e4`.

| Scenario | Status | Elapsed | Log SHA-256 | Phase summary SHA-256 | Failure summary SHA-256 |
| --- | --- | ---: | --- | --- | --- |
| connected-udp-on | pass | 164s | `704fb88d697f682765436da5b677ea2717fa28a3ea534301d1e7db6cb3efb707` | `0f524bd3693be302f514e792c011942d7ed4306845701727bb80d86ea295c7e4` | `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214` |
| connected-udp-off | pass | 164s | `510bcd127e2ee266f97ab75934d34c85a13336e62ee9c053ccf023e19e0893da` | `7461fe60a7b0c40187e712db21724826fe01b439edd3ae36c3fa2ea13bbc1f2e` | `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214` |
| single-encrypt-worker | pass | 162s | `0ba9121d4e5b97dd2084521d40015d7b35ec912483f0ec959c71cca646970438` | `a416a24f11cc4c51776591b08aa4554395820bda287422130d678f9002476243` | `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214` |
| tight-send-backpressure | fail | 48s | `b0128293accbf6fa0f688c7496d80272faa133a2f3555c98541b7084e2807b39` | `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78` | `90e9847d51e915502b6a6f57f28b5a77e0eef486cbfae602dcfd6f62f64c61a8` |
| tight-backpressure | fail | 34s | `d992cd6d3ec6664b9f21eaeada816d616f1137fdbd1295f835c42140499c1eec` | `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78` | `b127c6028926dcfd0c87d28f4afe2dbb245b4456e9660ebc1536ea986f39e440` |

Passing-row phase summary:

| Scenario | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker-pressure fwd/rev Mbps | Rx-maint fwd/rev Mbps | Max post ping p95/p99/max | Direct bytes evidence |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| connected-udp-on | 1915.8 / 1996.8 | 167.4 / 175.3 | 83.0 / 89.6 | 1983.9 / 1858.4 | 1.910 / 2.540 / 2.538 ms | all phases advanced direct UDP bytes |
| connected-udp-off | 2032.2 / 2064.8 | 173.6 / 172.7 | 124.7 / 84.3 | 2029.1 / 1987.6 | 1.740 / 3.360 / 3.359 ms | all phases advanced direct UDP bytes |
| single-encrypt-worker | 1702.6 / 1767.2 | 169.9 / 172.4 | 422.0 / 437.1 | 1844.3 / 1813.8 | 0.688 / 2.180 / 2.178 ms | all phases advanced direct UDP bytes |

Current red rows:

- `tight-send-backpressure`: clean-underlay reverse-load ping loss was `5%`
  against a `2%` maximum. Forward/reverse TCP were `319.9` / `315.8 Mbps`;
  reverse-load TCP was `288.3 Mbps`; direct bytes advanced `560959404` /
  `531035139`; captured pipeline context included `encrypt_worker_queue_full`
  and `encrypt_worker_bulk_dropped`.
- `tight-backpressure`: `clean-underlay forward TCP throughput Mbps` was
  `73.0` against the `100` floor, with `724` retransmits. Direct bytes advanced
  `55809808` / `2537550`; captured pipeline context included
  `decrypt_worker_queue_full` and `decrypt_worker_bulk_dropped`.

Linux UDP bulk-drop budget follow-up at FIPS `bc000de`:

FIPS `bc000de` makes the Linux batched `sendmmsg` path honor the existing
explicit UDP send backpressure drop budget for packets already marked
droppable. Linux still defaults `FIPS_SEND_BACKPRESSURE_DROP_AFTER` to `0`, so
the default platform matrix behavior above is unchanged; this pins the explicit
bulk-drop policy for future constrained profiles instead of letting a worker
retry one droppable datagram indefinitely while control traffic waits behind the
active flush.

Validation:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core send_backpressure -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && scripts/test-dataplane-safety-linux-docker.sh send_backpressure )
( cd "$FIPS_SAFETY_WORKTREE" && scripts/test-dataplane-safety-linux-docker.sh )
```

All three passed. The full Linux deterministic safety runner covered encrypt
priority reserve, fair queue admission, decrypt priority/bulk lanes, fallback
priority drain, endpoint priority classification, route/MMP stale-sample
guards, and connected-UDP policy.

Focused explicit-drop probe:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-send-backpressure \
NVPN_PLATFORM_MATRIX_PHASES=clean-underlay \
NVPN_PLATFORM_MATRIX_BACKPRESSURE_DROP_AFTER=2 \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-bc000de-nvpn-8eb1ee74-tight-send-drop2-clean \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-drop2 \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The probe stayed red in `87s`: clean-underlay reverse-load ping loss was `5%`
against the `2%` ceiling. Forward/reverse TCP stayed above the floor at
`374.5` / `428.3 Mbps`, forward-load TCP was `401.0 Mbps`, reverse-load TCP was
`133.3 Mbps`, and direct bytes advanced `694355390` / `437856547`. The failure
summary still points at encrypt-worker pressure (`encrypt_worker_queue_full` /
`encrypt_worker_bulk_dropped`) rather than `udp_send_bulk_dropped`, so the next
red target remains worker admission/drain behavior, not the Linux send retry
budget.

Artifacts:

- Summary SHA-256:
  `f0f5dd655d742721744156679bbf15732bb591038956f847fa8a32fae571df7b`
- Failure-summary SHA-256:
  `2838a6af349fb921f9f9c0d64341a95fb4874d67114f4563c62e6cc5acebccbd`
- Phase-summary SHA-256:
  `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78`
- Scenario log SHA-256:
  `8feb3c301bdcd4e9aa072ea774876e4537e24eb9cab2c2d82207d93bf0738578`

Independent encrypt priority reserve follow-up at FIPS `e1c67a7`:

FIPS `e1c67a7` decouples the non-macOS encrypt priority reserve from the tight
bulk worker channel cap. Tight bulk admission can still force explicit bulk
drops, but `FIPS_ENCRYPT_WORKER_PRIORITY_CHANNEL_CAP` now controls the reserved
priority capacity independently of `FIPS_WORKER_CHANNEL_CAP`. The new
`priority_reserve_does_not_shrink_with_tight_bulk_channel_cap` guard fills a
tiny bulk queue and proves multiple priority jobs can still enqueue; the old
derived priority cap would reject the third priority job.

Validation:

```sh
( cd "$FIPS_SAFETY_WORKTREE" && cargo test -p fips-core priority_ -- --nocapture )
( cd "$FIPS_SAFETY_WORKTREE" && scripts/test-dataplane-safety-linux-docker.sh \
  priority_reserve_does_not_shrink_with_tight_bulk_channel_cap \
  priority_flow_enters_when_bulk_worker_queue_is_full \
  fair_dispatch_does_not_block_rx_loop_on_full_bulk_queue )
( cd "$FIPS_SAFETY_WORKTREE" && scripts/test-dataplane-safety-linux-docker.sh )
```

The full Linux deterministic safety runner passed with the new priority-reserve
guard in its default filter list.

Focused matrix probe after the priority-reserve split:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-send-backpressure \
NVPN_PLATFORM_MATRIX_PHASES=clean-underlay \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-e1c67a7-nvpn-ac679170-tight-send-clean \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-prio-reserve \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The focused row still failed in `92s` on clean-underlay reverse-load ping loss:
`5%` against the `2%` ceiling. Forward/reverse TCP stayed above the floor at
`299.5` / `325.4 Mbps`, reverse-load TCP was `312.5 Mbps`, p95/p99/max ping in
the failed reverse-load window were `2.17` / `2.17` / `2.167 ms`, and direct
bytes advanced `539806590` / `545143665`. The log scan found no priority-full
or UDP bulk-drop warning pattern and no `udp_send_bulk_dropped`; the remaining
red signal is bulk encrypt-worker pressure plus one lost tunnel ping under
concurrent load.

Artifacts:

- Summary SHA-256:
  `0099be2f39bb6829469863b147b8a101da75a768d00081777767eee2d4b9372b`
- Failure-summary SHA-256:
  `d19d4bce8de34bd8bc18b35425f2ac07a841856ef8d54de38da4829c598aac14`
- Phase-summary SHA-256:
  `15abfccc32b489a442f504fca064308656987ecb2ce1377ac3b8d8781af9ff78`
- Scenario log SHA-256:
  `157552e1e3375bd315924da5f796cc7e4f14b1136adbb541d64a6a36ca3a042c`

Matrix ping-window follow-up at nvpn `805b2ff`:

nvpn `805b2ff` aligns the platform matrix default ping count with the perf
gate default (`60` samples). The earlier `20`-ping matrix smoke made the stated
`2%` loss ceiling fail on any single lost echo (`5%`), which is too coarse for
distinguishing an isolated ICMP miss from the TCP/ping collapse this safety net
is meant to catch. The platform-matrix harness self-test now pins both the
default `60` sample window and the `NVPN_PLATFORM_MATRIX_PING_COUNT` override.

Focused committed-head rerun:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-send-backpressure \
NVPN_PLATFORM_MATRIX_PHASES=clean-underlay \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-e1c67a7-nvpn-805b2ff-tight-send-clean-ping60 \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-ping60-committed \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The focused row passed in `54s`. Forward/reverse TCP were `312.3` /
`270.4 Mbps`; forward-load TCP was `271.3 Mbps` with one lost ping out of `60`
(`1.66667%`, below the `2%` ceiling), reverse-load TCP was `296.1 Mbps` with
`0%` ping loss, post-load ping was `0%` loss, and direct bytes advanced
`500981332` / `488038135`. Expected
`encrypt_worker_queue_full` / `encrypt_worker_bulk_dropped` pressure remained
visible, so the run still exercised the intended send-pressure profile.

Artifacts:

- Summary SHA-256:
  `c87a79ef44f198fd3d9f88e24646d3d85f6b8cc768b101f0fd4c7503d8448d77`
- Phase-summary SHA-256:
  `ee7102640aa4563898475f18da1167e3bb94014ea1fea44aa8c95a9c14ed69eb`
- Scenario log SHA-256:
  `5b0370e6b0c5c210117fe5bc498e330352d3689bccb2d25b68ff25eda68caa33`

This clears the short-sample `tight-send-backpressure` clean-underlay artifact.
It does not replace a full platform-matrix rerun, the remaining legacy
`tight-backpressure` target, Docker soak, host/VM pair run, or real Mac-to-Mac
validation.

This is local Linux/docker platform-matrix evidence only. It does not replace a
Docker soak, host/VM pair run, or real Mac-to-Mac validation.

Full 60-ping local-FIPS platform matrix at FIPS `e1c67a7` plus nvpn
`4e113ab`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-e1c67a7-nvpn-4e113ab-full-matrix-ping60 \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-e1c67a7-ping60 \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The full matrix is still red, but the remaining failure is narrower than the
earlier TCP collapse cases. `connected-udp-on`, `connected-udp-off`,
`single-encrypt-worker`, and the legacy `tight-backpressure` row all passed all
four phases. `tight-send-backpressure` passed clean-underlay,
constrained-underlay, and worker-queue-pressure; it failed only
`rx-maintenance-fault` because one reverse-load ping out of `60` was lost
(`1.66667%`) against that phase's strict `0%` loss ceiling. In the failed
window, reverse-load TCP stayed healthy at `440.8 Mbps` with `3049`
retransmits, ping avg/p95/p99/max were `0.322` / `0.465` / `0.707` /
`0.707 ms`, direct bytes advanced `775713567` / `752519145`, and the pipeline
context still showed the intended `encrypt_worker_queue_full` /
`encrypt_worker_bulk_dropped` pressure. Treat this as the current strict red
safety signal before moving hot-path ownership; do not hide it by relaxing the
rx-maintenance threshold without a deliberate policy decision.

Artifacts:

- Summary SHA-256:
  `89e735bff597757f8b96018ebcc1e02813450af199057e8d083cf96635719e4f`
- `connected-udp-on`: pass in `181s`, log SHA-256
  `5744245fa85b9379432c7fd9aae51e313f30e6ba37bc0add5c253eab9d562b15`,
  phase-summary SHA-256
  `aa6aa5b34c69fb7d685b25b70f66be4e63a3325ada0046aab2aaf45d9fee47be`
- `connected-udp-off`: pass in `180s`, log SHA-256
  `da1a64688a3a5382f070deccf7456722d588331b6c8ef9523f910e9c92cc27f4`,
  phase-summary SHA-256
  `c5fffe8b6ade57d7c057508efcfeeae91dce3250139ccde25e24c1632391070e`
- `single-encrypt-worker`: pass in `178s`, log SHA-256
  `9ae0e90750b1a89c168d3d4f0c8d1f809a69a5d684861de5a39e84ecba5bb86c`,
  phase-summary SHA-256
  `23d50beede2f46d2e285bcb798eb450cf071dfa31e31526fb8021fcfb0b21b49`
- `tight-send-backpressure`: fail in `172s`, log SHA-256
  `fba935469ab32bc5a4cee7acec48b950197e914e5be5d178b1865e3decdfe8d4`,
  phase-summary SHA-256
  `514228329a2575fd2acd778b933d8b2c5f65984997adcc81a18ccc0cc336ccdb`,
  failure-summary SHA-256
  `d65b37927434304a4c8b3511751fd88c5d09fe79d14dc92333132ffa827a79cd`
- `tight-backpressure`: pass in `179s`, log SHA-256
  `9a6e4031636183d575809c60f0cd5bbf05b1264ec7d3f7d54cb9ac1c891bf0bb`,
  phase-summary SHA-256
  `00b1cb0c04b3ee16928c4984375b8fee49fbc94781bb080ac7012c29d44ce837`

This is local Linux/docker platform-matrix evidence only. It does not replace a
Docker soak, host/VM pair run, or real Mac-to-Mac validation.

FIPS `ae874b2` then made IPv4 ICMP endpoint payloads use the priority,
non-droppable lane, matching the existing ICMPv6 and small-control-packet
behavior. This matters because the Docker matrix tunnel addresses are IPv4; the
previous tunnel-ping canary could be classified as bulk and intentionally
dropped under send pressure.

Deterministic validation for the FIPS change:

```sh
cargo test -p fips-core endpoint_payload_traffic_classifier_prioritizes_ipv4_icmp_ping
cargo test -p fips-core endpoint_payload_traffic_classifier_prioritizes_control_sized_packets
cargo test -p fips-core endpoint_command_tx_helper_classifies_priority_and_bulk_payloads

./scripts/test-dataplane-safety-linux-docker.sh \
  endpoint_payload_traffic_classifier_prioritizes_ipv4_icmp_ping \
  endpoint_command_tx_helper_classifies_priority_and_bulk_payloads \
  endpoint_payload_traffic_classifier_prioritizes_control_sized_packets

./scripts/test-dataplane-safety-linux-docker.sh
```

A focused `tight-send-backpressure` rerun at FIPS `ae874b2` plus nvpn
`02a1ae2` passed all platform-matrix phases, including the former
`rx-maintenance-fault` ping-loss red row:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-send-backpressure \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-ae874b2-nvpn-02a1ae2-tight-send-ping-priority \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-ae874b2-ping-prio \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The focused row stayed under pressure while keeping tunnel ping loss at `0%` in
all load/post windows. Clean-underlay forward/reverse TCP was `407.5` /
`412.6 Mbps`; constrained-underlay was `184.0` / `135.5 Mbps`;
worker-queue-pressure was `123.6` / `126.8 Mbps`; and
`rx-maintenance-fault` stayed healthy at `387.9` / `416.0 Mbps` with p99 tunnel
ping under `6.04 ms`. Direct UDP bytes advanced in each phase and the expected
`encrypt_worker_queue_full` / `encrypt_worker_bulk_dropped` pressure remained
visible.

Artifacts:

- Summary SHA-256:
  `7c5baca82c8869b354aa42dbead8a79bd241bdd7925afa22da56503ede4e18d3`
- Phase-summary SHA-256:
  `b4f3615b4ea2b767d39a3cf40fc45d3e332cacc1b9de59a0556e0d5199688860`
- Failure-summary SHA-256:
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- Scenario log SHA-256:
  `e03b52f14a73805a244978e2657f3359e44b85717174a53855e44fd97c72c803`

The full local-FIPS platform matrix at the same FIPS/nvpn heads is still red,
but the red target moved from the previous tunnel-ping canary to clean-underlay
TCP throughput under tight bounded pressure:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-ae874b2-nvpn-02a1ae2-full-matrix-ping-priority \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-ae874b2-full \
./scripts/e2e-fips-platform-matrix-docker.sh
```

`connected-udp-on`, `connected-udp-off`, and `single-encrypt-worker` passed all
phases. `tight-send-backpressure` failed before phase-summary rows on
clean-underlay forward TCP at `93.3 Mbps` against the `100 Mbps` floor, with
`encrypt_worker_queue_full` / `encrypt_worker_bulk_dropped` pressure visible.
The harsher `tight-backpressure` row failed clean-underlay forward TCP at
`11.7 Mbps` against the same floor, with
`decrypt_worker_queue_full` / `decrypt_worker_bulk_dropped` pressure visible.
Treat this as the current Linux/docker red safety signal before a hot-path
ownership rewrite: TCP must degrade recoverably under explicit tight pressure
without hiding backpressure in giant queues.

Artifacts:

- Summary SHA-256:
  `0e9bfd2da26c2c06629f5aec02d007dde51e8ceb7ca657a98a2701739ebcf9d8`
- `connected-udp-on`: pass in `178s`, log SHA-256
  `ff89eb495c9ec26ea6fb7f38219d9dadacf4d05e7bfbc0ef22ebbfae05fa7c73`,
  phase-summary SHA-256
  `88a84a338c6e01892ec53ee95b689e8976fd1a51bbaff0d4293f3e3e743c1b78`
- `connected-udp-off`: pass in `180s`, log SHA-256
  `961dc673eee0dc49fd634dd6e826dea81515de7591ef216876fd01609e36f13d`,
  phase-summary SHA-256
  `452ef9e423da4c035a6155fc403d698874ffa78a9db00dc5dbd47d8dc406c1fe`
- `single-encrypt-worker`: pass in `184s`, log SHA-256
  `376d01c22e02e8beb16018c60b14098b18551e06ffafe6bdca395fa08a3b58bc`,
  phase-summary SHA-256
  `351bdd2bc608e6d5e40d1bd9fbaf51c1f93b53ee883b36ad548b40fe2857902f`
- `tight-send-backpressure`: fail in `31s`, log SHA-256
  `2e16a5b5e5ceb9eb486de200f75ab05ea598c1c12b5bab47222f3474ff04f189`,
  failure-summary SHA-256
  `359d9e84ee0242ae8dc9d6c82fccab869ac2d4bde4fe9b6f91b26dc40468ee71`
- `tight-backpressure`: fail in `33s`, log SHA-256
  `f0b743eb2ec8c6bb2c2cb604820fb9e393addea1c3fde7fabc64f5dd6d3287da`,
  failure-summary SHA-256
  `e18ce107a56e969bb4b9053a91970c9e3847be872054713d1a0e4512a9cde8c8`

This is local Linux/docker platform-matrix evidence only. It does not replace a
Docker soak, host/VM pair run, or real Mac-to-Mac validation.

FIPS `24bff11` then bounds the non-macOS encrypt-worker direct fast lane to one
per-flow burst instead of two before fair admission starts. The new
`tight_bulk_cap_limits_single_flow_to_fast_lane_plus_fair_budget` guard would
have failed on the previous policy, where a single flow could queue a third
per-flow burst before reporting pressure.

Deterministic validation:

```sh
./scripts/test-dataplane-safety-linux-docker.sh \
  tight_bulk_cap_limits_single_flow_to_fast_lane_plus_fair_budget \
  hot_flow_backpressures_when_others_are_waiting \
  boosted_flow_gets_larger_queue_budget \
  priority_flow_enters_when_bulk_flow_reaches_per_flow_cap \
  fair_admission_keys_pressure_by_exact_send_target

cargo fmt --check
./scripts/test-dataplane-safety-linux-docker.sh
```

Focused local-FIPS matrix for the two known tight-pressure red rows at FIPS
`24bff11` plus nvpn `4073298`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_SCENARIOS=tight-send-backpressure,tight-backpressure \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-24bff11-nvpn-4073298-tight-pressure \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-24bff11-tight \
./scripts/e2e-fips-platform-matrix-docker.sh
```

Both rows passed. `tight-send-backpressure` passed in `220s`: clean-underlay
forward/reverse TCP was `321.8` / `310.9 Mbps`, constrained-underlay was
`184.5` / `185.6 Mbps`, worker-queue-pressure was `120.7` / `123.7 Mbps`, and
`rx-maintenance-fault` was `323.0` / `315.8 Mbps`. All load/post tunnel-ping
windows had `0%` loss, direct UDP bytes advanced in every phase, and the
expected `encrypt_worker_queue_full` / `encrypt_worker_bulk_dropped` pressure
remained visible.

`tight-backpressure` passed in `213s`: clean-underlay forward/reverse TCP was
`136.2` / `116.2 Mbps`, constrained-underlay was `118.6` / `134.8 Mbps`,
worker-queue-pressure was `128.3` / `112.6 Mbps`, and
`rx-maintenance-fault` was `119.2` / `121.4 Mbps`. All load/post tunnel-ping
windows had `0%` loss, direct UDP bytes advanced in every phase, and the
expected `decrypt_worker_queue_full` / `decrypt_worker_bulk_dropped` pressure
remained visible.

Artifacts:

- Summary SHA-256:
  `512ac3daf85ad21c4013780fceaf74504bf42074038af7342e368081a7dd4536`
- `tight-send-backpressure`: pass in `220s`, log SHA-256
  `2b2eb36fdafd18469d8ae55a91d0fa4f522ca83578f77f3d3518aee2c82f9795`,
  phase-summary SHA-256
  `d9be2b71992e5a46616c1c60f8383b803dd1b1d7f1abe446c1d5a9baef9dd75`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `tight-backpressure`: pass in `213s`, log SHA-256
  `eb192902747f3942a553b5d59dfa20210b1534c3eda75b72300e2a254de95f35`,
  phase-summary SHA-256
  `a9e8a3c151f275c0cf4d437fce8b695ddd5eca642b26015cfcd25ae9b8709567`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`

This clears the two known tight-pressure red rows in a focused local
Linux/docker matrix. It does not replace a full default platform matrix rerun,
Docker soak, host/VM pair run, or real Mac-to-Mac validation.

Full default local-FIPS matrix at FIPS `24bff11` plus nvpn `4073298`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-24bff11-nvpn-4073298-full-matrix \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-24bff11-full \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The full matrix remains red by one threshold. Four scenarios passed:
`connected-udp-on`, `connected-udp-off`, `single-encrypt-worker`, and
`tight-send-backpressure`. `tight-backpressure` passed clean-underlay,
constrained-underlay, and worker-queue-pressure with `0%` load/post tunnel-ping
loss, then failed in `rx-maintenance-fault` before a complete phase row because
reverse TCP was `98.9 Mbps` against the `100 Mbps` floor. The failed phase still
advanced direct UDP bytes (`95876169` / `80429353`) and showed expected
`decrypt_worker_queue_full` / `decrypt_worker_bulk_dropped` pressure.

Selected phase results:

| Scenario | Status | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker pressure fwd/rev Mbps | Rx fault fwd/rev Mbps | Load/post ping loss |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| connected-udp-on | pass | `2300.1` / `2238.9` | `164.1` / `166.9` | `122.5` / `127.0` | `2213.6` / `2114.1` | `0%` |
| connected-udp-off | pass | `2268.8` / `2249.3` | `166.3` / `174.9` | `133.1` / `118.6` | `2245.6` / `2215.2` | `0%` |
| single-encrypt-worker | pass | `1877.7` / `1822.4` | `168.8` / `171.9` | `380.0` / `392.0` | `1929.4` / `1903.8` | `0%` |
| tight-send-backpressure | pass | `296.2` / `303.6` | `189.1` / `189.2` | `120.7` / `122.1` | `289.8` / `301.4` | `0%` |
| tight-backpressure | fail | `133.9` / `129.4` | `120.5` / `125.1` | `120.2` / `117.0` | `126.6` / `98.9` | `0%` before failed phase |

Artifacts:

- Summary SHA-256:
  `c1b0abda441b61115d8258922ae56d396f3bd9e45149bad9ea15794e05944446`
- `connected-udp-on`: pass in `179s`, log SHA-256
  `e8f7134589ca277a3c08ace63d5a147516fb3ae08e8423da5af843e9f7682212`,
  phase-summary SHA-256
  `9541d54c372b8916d00b48a074a55be823b45d5cae3680ff0d069f02cf37a8a3`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `connected-udp-off`: pass in `177s`, log SHA-256
  `614c92bcebc79d587bd8e08196b1ae16461ab0bf67c289695557d9475e33fb15`,
  phase-summary SHA-256
  `37b597db7895c9bc93fcd2da677f83eba35d2cfbb60f3985d5b24ab6fed1e7fa`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `single-encrypt-worker`: pass in `178s`, log SHA-256
  `994799caa195c5d7c8f5af0d656b2d071e80a7e955c2b272a5f3e7da690bed0b`,
  phase-summary SHA-256
  `41e4e63b902ceb50f3bbdac0149d863a00047822f41834f3af01de2cbd618520`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `tight-send-backpressure`: pass in `179s`, log SHA-256
  `a3f421666dcf740fd24f2ce82fadbe5d87b2eae664a3a00401ce59f53fe76562`,
  phase-summary SHA-256
  `207b806eaa253d2265993a360df4f793e21f793249eb63b6525ea2d0626dcd84`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `tight-backpressure`: fail in `161s`, log SHA-256
  `2ac0b3dde7fce090105d58de9d77889c3b3f5a30325199324379540b1277f2ca`,
  phase-summary SHA-256
  `d1e63ab1335f8ac58102521566a778562a2d493ba24f00a40559df6384d51d37`,
  failure-summary SHA-256
  `1641a91913fa97764f9ab49f2c9f33fc91ee035da2d9e592139d230e1f0a8bec`

This is a much narrower red result than the previous full matrix, but it still
keeps the full Linux/docker gate red. Do not use this as rewrite clearance until
the full default matrix is green and a soak confirms no drift.

Focused reruns at FIPS `24bff11` plus nvpn `81359bf` separated the narrow miss
from a deterministic wedge. The isolated `tight-backpressure` /
`rx-maintenance-fault` phase passed with forward/reverse TCP `113.5` /
`123.5 Mbps`, forward/reverse load TCP `118.0` / `112.2 Mbps`, `0%` load/post
tunnel-ping loss, direct UDP byte progress, and expected decrypt queue-full /
bulk-drop pressure. A three-attempt `tight-backpressure` rerun then passed all
attempts and all phases.

Full default local-FIPS matrix rerun at FIPS `24bff11` plus nvpn `81359bf`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PLATFORM_MATRIX_OUTPUT_DIR=artifacts/fips-platform-matrix/fips-24bff11-nvpn-81359bf-full-matrix-rerun \
NVPN_PLATFORM_MATRIX_PROJECT_PREFIX=nostr-vpn-e2e-fips-24bff11-full-rerun \
./scripts/e2e-fips-platform-matrix-docker.sh
```

The full matrix passed all five scenarios. Every phase advanced the configured
direct UDP underlay counters, load/post tunnel-ping loss stayed at `0%`, and
the explicit pressure rows emitted bounded queue-full/bulk-drop counters rather
than hidden latency. This removes the Linux/docker full-matrix red edge, but it
does not replace Docker soak, host/VM soak, or real Mac-to-Mac validation.

Selected phase results:

| Scenario | Status | Clean fwd/rev Mbps | Constrained fwd/rev Mbps | Worker pressure fwd/rev load Mbps | Rx fault fwd/rev load Mbps | Load/post ping loss | Post-load ping max |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- |
| connected-udp-on | pass | `2116.8` / `2214.8` | `172.1` / `174.1` | `119.1` / `129.0` | `2223.4` / `2224.5` | `0%` | `<= 4.799 ms` |
| connected-udp-off | pass | `2244.7` / `2297.5` | `167.3` / `175.9` | `130.4` / `133.0` | `2194.8` / `2149.9` | `0%` | `<= 3.639 ms` |
| single-encrypt-worker | pass | `1840.4` / `1294.1` | `171.0` / `173.7` | `341.4` / `385.9` | `1909.5` / `1802.0` | `0%` | `<= 5.482 ms` |
| tight-send-backpressure | pass | `308.1` / `303.1` | `183.2` / `185.5` | `114.4` / `115.0` | `308.1` / `301.4` | `0%` | `<= 3.960 ms` |
| tight-backpressure | pass | `123.7` / `120.2` | `133.7` / `129.3` | `124.4` / `116.0` | `121.0` / `133.5` | `0%` | `<= 5.480 ms` |

Artifacts:

- Summary SHA-256:
  `889887dcd84309ed8efb3abb9cc302d49a9322caaed00b43108a612f3374c15d`
- `connected-udp-on`: pass in `211s`, log SHA-256
  `553be5ea48d333b0c2aa863d8c194b1828a0d48b0e1a0445d1884dbfb2c9c5cd`,
  phase-summary SHA-256
  `e09b38cbc083d9cfd1497e3472eaa33732c3883ecd8f74400a5a1abcd9edbd09`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `connected-udp-off`: pass in `177s`, log SHA-256
  `06ee8c0df50cce17b773e6356a35902640b618131a103a42f32ef4a8e111744f`,
  phase-summary SHA-256
  `737c3cf7002a437862e33dd01d3de99de41da5964a05d286fc2d57f7f9bb9638`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `single-encrypt-worker`: pass in `180s`, log SHA-256
  `4dca4392b574f39609fea9d68b92de0ece357529cd5aca3b9eeebdb08696dabe`,
  phase-summary SHA-256
  `d7b576e922c9582f17216a00ba17f25d0e7eaa469c183c38feb57da57e5c553c`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `tight-send-backpressure`: pass in `180s`, log SHA-256
  `9861f94d39aaa39a140d74ea2496773c8a437ffa4728272aca1442e27854c877`,
  phase-summary SHA-256
  `278d98468ba68e6d7db7bbe97a5d71660f0850d3cd430a61a520a4cf3a649a5a`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`
- `tight-backpressure`: pass in `180s`, log SHA-256
  `c520c73242cfc4f4e3d2a3816bded037febacb7c0ecc4055b2773dbc314d3c35`,
  phase-summary SHA-256
  `1a48633efccb3f43d8400faf74284a96d73360a5f8e42a571f603675303c72c8`,
  failure-summary SHA-256
  `d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`

During the focused and full reruns, the external hashtree Docker context was
observed changing from `0.2.63` to `0.2.64`. Treat these artifacts as
current-state safety evidence for the local nvpn/FIPS branches, not as a pure
two-repo delta against the prior matrix.

Current 30-minute Docker soak at FIPS `24bff11` plus nvpn `a9bfbcd`:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_SOAK_DURATION_SECS=1800 \
NVPN_SOAK_OUTPUT_DIR=artifacts/fips-soak/fips-24bff11-nvpn-a9bfbcd-30m \
PROJECT_NAME=nostr-vpn-soak-fips-24bff11-a9bfbcd-30m \
./scripts/soak-fips-dataplane-docker.sh
```

Ignored sample file:
`artifacts/fips-soak/fips-24bff11-nvpn-a9bfbcd-30m/samples.ndjson`.
Sample SHA-256:
`784f0da70c2d617e6d421970960249ac6deb0891561d9f035b4cb03c90e5163e`.

Raw log captured locally as
`/tmp/nvpn-fips-soak-fips-24bff11-nvpn-a9bfbcd-30m.log`.
Log SHA-256:
`98dab324436ee48e8848499a44c1016467225d2b36becade7bd5399add18a286`.

The soak passed with `33` samples from `2026-06-08T16:03:13Z` through
`2026-06-08T16:33:09Z`. Both peers stayed on the configured direct Docker
underlay endpoint for the entire run. All hard queue/drop/backpressure events
stayed absent, including worker queue-full, bulk-drop, UDP send backpressure,
connected-UDP activation failure, and nvpn TUN-to-mesh bulk-drop signals.

Observed ranges:

- FIPS SRTT: node-a `1-3 ms`, node-b `1-2 ms`
- Ping loss: `0%` both ways in all samples
- Ping avg: node-a to node-b `0.478-1.452 ms`, node-b to node-a
  `0.505-1.523 ms`
- Ping p95: node-a to node-b `0.515-2.740 ms`, node-b to node-a
  `0.568-3.100 ms`
- Ping p99: node-a to node-b `0.583-7.550 ms`, node-b to node-a
  `0.572-7.760 ms`
- Ping max: node-a to node-b `0.583-7.550 ms`, node-b to node-a
  `0.572-7.763 ms`
- Iperf forward: `448.171-2289.194 Mbps`
- Iperf reverse: `1313.265-2269.222 Mbps`
- Iperf retransmits: forward `24-315`, reverse `58-279`
- Daemon CPU: node-a `39.9-82.1%`, node-b `39.7-89.1%`
- Max observed FIPS queue wait: node-a worker p95/p99/max
  `33.6` / `33.6` / `33.6 ms`, node-b worker p95/p99/max
  `16.8` / `16.8` / `33.6 ms`
- Max observed FIPS transport queue wait: node-a p95/p99/max
  `8.4` / `8.4` / `16.8 ms`, node-b p95/p99/max
  `8.4` / `16.8` / `33.6 ms`
- Max observed nvpn TUN-to-mesh queue wait: node-a p95/p99/max
  `0.5243` / `1.0` / `16.8 ms`, node-b p95/p99/max
  `0.5243` / `2.1` / `16.8 ms`
- Final FIPS counters:
  - node-a sent/recv bytes: `54479209629` / `54695538676`
  - node-b sent/recv bytes: `54716735698` / `54479714560`

The forward TCP low point was a recoverable Docker-host variability sample: it
did not coincide with route drift, ping/SRTT drift, hard queue/drop events, or
byte-counter no-progress. Keep that range visible when comparing future runs.

First ownership-boundary refactor at FIPS `cda112a` plus nvpn `85be91d`:

FIPS `cda112a` introduces an explicit `DecryptWorkerShard` type around the
worker-owned FMP session table. It does not change the public worker API or move
additional state; it makes the existing owner boundary concrete before later
FMP/FSP state moves.

Deterministic validation:

```sh
cargo fmt --check
cargo test -p fips-core decrypt_worker -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh
```

The focused decrypt-worker test filter passed `20` tests locally, including the
new `decrypt_worker_shard_owns_register_and_unregister_state` guard. The full
Linux deterministic runner also passed with that guard added to its default
filter list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-cda112a-nvpn-85be91d-shard-owner-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-shard-owner-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-cda112a-nvpn-85be91d-shard-owner-smoke/phase-summary.tsv`.
Summary SHA-256:
`fcd46b31ef4d64dc3ed9530173dbe3bee0f3b5b4e652e39b46bedf04a2aea032`.

Failure summary:
`artifacts/fips-perf/fips-cda112a-nvpn-85be91d-shard-owner-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed only expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2170.3` | `2181.9` | `2211.5` | `2262.8` | `2.390 / 2.390 / 2.385 ms` | `2.350 / 2.350 / 2.353 ms` | `1.180 / 1.180 / 1.177 ms` | `2256951643 / 2276990130` |
| constrained-underlay | `130.9` | `133.3` | `134.9` | `137.0` | `86.700 / 86.700 / 86.708 ms` | `89.100 / 89.100 / 89.098 ms` | `2.520 / 2.520 / 2.517 ms` | `143441045 / 146852157` |
| worker-queue-pressure | `119.1` | `121.5` | `122.0` | `123.7` | `0.343 / 0.343 / 0.343 ms` | `0.375 / 0.375 / 0.375 ms` | `1.230 / 1.230 / 1.232 ms` | `129384142 / 130302636` |
| rx-maintenance-fault | `2209.8` | `2062.2` | `2196.7` | `2161.9` | `2.240 / 2.240 / 2.241 ms` | `2.570 / 2.570 / 2.574 ms` | `1.380 / 1.380 / 1.377 ms` | `2211226073 / 2207668991` |

FMP worker-send ownership step at FIPS `c33996e` plus nvpn `de4342ac`:

FIPS `c33996e` adds an explicit `FmpWorkerSendReservation` for established FMP
worker sends. The reservation owns the cloned cipher, reserved counter, and
header together. The send path now resolves the worker UDP target before
consuming the counter, so fallback inline encryption remains the only counter
owner when the worker target is unavailable.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core fmp_worker -- --nocapture
cargo test -p fips-core decrypt_worker -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  fmp_worker_send_reservation_owns_counter_header_and_cipher \
  fmp_worker_target_fallback_consumes_one_inline_counter
cargo test -p fips-core
```

The focused `fmp_worker` filter passed the two new local guards. The focused
Linux deterministic runner passed the same two guards inside the container. The
full local `fips-core` suite passed with `1402` tests, `0` failures, and `2`
ignored tests; doctests had `2` ignored tests. The `decrypt_worker` filter
still passed `20` tests after the send-path change.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-c33996e-nvpn-de4342ac-fmp-send-owner-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-fmp-send-owner-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

The Docker build again resolved hashtree crates to `0.2.64`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

Phase summary:
`artifacts/fips-perf/fips-c33996e-nvpn-de4342ac-fmp-send-owner-smoke/phase-summary.tsv`.
Summary SHA-256:
`a396981917c36b2c0c7603d7c5f16cc8fa3c5e6eb56584b508b5417195be9944`.

Failure summary:
`artifacts/fips-perf/fips-c33996e-nvpn-de4342ac-fmp-send-owner-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2231.7` | `2160.1` | `2103.1` | `2182.7` | `1.850 / 1.850 / 1.848 ms` | `2.790 / 2.790 / 2.791 ms` | `1.750 / 1.750 / 1.751 ms` | `2224691127 / 2235867475` |
| constrained-underlay | `128.1` | `126.9` | `135.0` | `137.2` | `92.600 / 92.600 / 92.633 ms` | `87.300 / 87.300 / 87.317 ms` | `1.310 / 1.310 / 1.306 ms` | `140275810 / 144389341` |
| worker-queue-pressure | `124.3` | `131.6` | `115.7` | `119.3` | `0.381 / 0.381 / 0.381 ms` | `0.404 / 0.404 / 0.404 ms` | `1.600 / 1.600 / 1.602 ms` | `127562851 / 131523498` |
| rx-maintenance-fault | `2254.3` | `2149.0` | `2214.5` | `2231.1` | `2.650 / 2.650 / 2.654 ms` | `1.650 / 1.650 / 1.654 ms` | `1.130 / 1.130 / 1.128 ms` | `2292299163 / 2254814523` |

FMP worker-recv open ownership step at FIPS `b02eb10` plus nvpn `03f96c48`:

FIPS `b02eb10` adds `OwnedSessionState::open_fmp_in_place`, a single
worker-owned operation for replay check, AEAD open, and replay accept. This
keeps FMP recv replay mutation inside the session state owner instead of
spreading check/open/accept sequencing across worker glue code.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core owned_session_state_open_fmp_owns_replay_acceptance -- --nocapture
cargo test -p fips-core decrypt_worker_accepts_fmp_replay_only_after_aead_success -- --nocapture
cargo test -p fips-core decrypt_worker -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  owned_session_state_open_fmp_owns_replay_acceptance \
  decrypt_worker_accepts_fmp_replay_only_after_aead_success
cargo test -p fips-core
./scripts/test-dataplane-safety-linux-docker.sh
```

The first full default Linux deterministic runner attempt failed before tests
when the container could not reach Debian package indexes. A retry reached the
test phase and passed the full default filter list, including the new
`owned_session_state_open_fmp_owns_replay_acceptance` guard. Local
`decrypt_worker` passed `21` tests. Full local `fips-core` passed with `1403`
tests, `0` failures, and `2` ignored tests; doctests had `2` ignored tests.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-b02eb10-nvpn-03f96c48-fmp-open-owner-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-fmp-open-owner-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-b02eb10-nvpn-03f96c48-fmp-open-owner-smoke/phase-summary.tsv`.
Summary SHA-256:
`54c1c4f7d79ef2eb7e45ca32d40bca1278aeb2e4f508f2cb9af2468edc599033`.

Failure summary:
`artifacts/fips-perf/fips-b02eb10-nvpn-03f96c48-fmp-open-owner-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2250.4` | `2251.2` | `2029.3` | `2088.0` | `1.480 / 1.480 / 1.480 ms` | `2.260 / 2.260 / 2.263 ms` | `1.930 / 1.930 / 1.925 ms` | `2199132619 / 2193942207` |
| constrained-underlay | `127.1` | `126.0` | `128.5` | `127.1` | `84.900 / 84.900 / 84.931 ms` | `93.900 / 93.900 / 93.922 ms` | `4.090 / 4.090 / 4.089 ms` | `135923214 / 137501566` |
| worker-queue-pressure | `116.0` | `119.5` | `116.1` | `124.3` | `1.300 / 1.300 / 1.295 ms` | `0.512 / 0.512 / 0.512 ms` | `1.800 / 1.800 / 1.802 ms` | `125231559 / 126859549` |
| rx-maintenance-fault | `2004.0` | `2116.2` | `2146.1` | `2147.8` | `1.960 / 1.960 / 1.961 ms` | `1.850 / 1.850 / 1.854 ms` | `4.100 / 4.100 / 4.100 ms` | `2131280016 / 2173204692` |

As with the previous current-state Docker runs, the external hashtree context
was observed resolving to `0.2.64`; treat this as current local-branch safety
evidence rather than a pure FIPS-only delta.

FSP recv open ownership step at FIPS `a93cd50` plus nvpn `201d3c97`:

FIPS `a93cd50` renames the established FSP epoch trial path to
`SessionEntry::open_fsp_established_frame` and gives the all-epochs-failed case
an explicit `FspOpenError`. The session entry owns current/pending/previous
epoch selection plus replay check, AEAD open, and replay accept. Failed epoch
candidates leave replay windows untouched; only the authenticated epoch
advances.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core open_fsp_established_frame_failed_all_epochs_does_not_consume_replay -- --nocapture
cargo test -p fips-core overlapping_epoch_tests -- --nocapture
cargo test -p fips-core open_fsp_established_frame -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  open_fsp_established_frame_failed_all_epochs_does_not_consume_replay
cargo test -p fips-core
./scripts/test-dataplane-safety-linux-docker.sh
```

The focused local FSP guard passed, the overlapping epoch suite passed `12`
tests, and the `open_fsp_established_frame` filter passed `7` tests. Full local
`fips-core` passed with `1404` tests, `0` failures, and `2` ignored tests;
doctests had `2` ignored tests. The focused Linux deterministic runner passed
the new FSP guard inside the container, and the full default Linux deterministic
runner passed with that guard in its default filter list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-a93cd50-nvpn-201d3c97-fsp-open-owner-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-fsp-open-owner-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-a93cd50-nvpn-201d3c97-fsp-open-owner-smoke/phase-summary.tsv`.
Summary SHA-256:
`8932f4d5f5d4cb2a544bd968983bc57e03e69d2c94a354de541952131601c1e1`.

Failure summary:
`artifacts/fips-perf/fips-a93cd50-nvpn-201d3c97-fsp-open-owner-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build again resolved hashtree crates to `0.2.64`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `1953.6` | `2191.3` | `2233.2` | `2135.1` | `2.580 / 2.580 / 2.582 ms` | `2.050 / 2.050 / 2.050 ms` | `3.960 / 3.960 / 3.964 ms` | `2220744850 / 2231152045` |
| constrained-underlay | `131.2` | `130.4` | `130.6` | `130.2` | `90.000 / 90.000 / 90.019 ms` | `89.700 / 89.700 / 89.667 ms` | `1.550 / 1.550 / 1.552 ms` | `139914648 / 141532803` |
| worker-queue-pressure | `122.2` | `110.3` | `122.6` | `128.3` | `0.490 / 0.490 / 0.490 ms` | `0.371 / 0.371 / 0.371 ms` | `2.230 / 2.230 / 2.229 ms` | `128528446 / 125284219` |
| rx-maintenance-fault | `2308.2` | `2330.6` | `2158.2` | `2262.7` | `1.670 / 1.670 / 1.666 ms` | `1.900 / 1.900 / 1.898 ms` | `1.400 / 1.400 / 1.401 ms` | `2299643826 / 2355392306` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

FSP worker-send reservation ownership step at FIPS `4fa502b` plus nvpn
`f330cfe9`:

FIPS `4fa502b` adds `SessionEntry::reserve_fsp_worker_send`, a single
session-owned reservation for the FSP cloned cipher, counter, and header used
by the established endpoint-data worker path. The worker can seal with the
reserved cloned key, but counter sequencing stays with the session entry.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core reserve_fsp_worker_send_owns_counter_header_and_cipher -- --nocapture
cargo test -p fips-core pipelined_endpoint_wire_uses_reserved_counters_and_offsets -- --nocapture
cargo test -p fips-core open_fsp_established_frame -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  reserve_fsp_worker_send_owns_counter_header_and_cipher \
  pipelined_endpoint_wire_uses_reserved_counters_and_offsets
cargo test -p fips-core
./scripts/test-dataplane-safety-linux-docker.sh
```

The focused local FSP send guard passed, the pipelined endpoint wire guard
passed, and the `open_fsp_established_frame` filter passed `7` tests. Full
local `fips-core` passed with `1405` tests, `0` failures, and `2` ignored tests;
doctests had `2` ignored tests. The first focused Linux deterministic runner
attempt failed before tests when the container could not reach Debian package
indexes. A retry passed the focused send/wire guards, and the full default
Linux deterministic runner passed with the FSP send guard in its default filter
list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-4fa502b-nvpn-f330cfe9-fsp-send-owner-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-fsp-send-owner-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-4fa502b-nvpn-f330cfe9-fsp-send-owner-smoke/phase-summary.tsv`.
Summary SHA-256:
`d0650f707d723f965c3cc973ca64de91d1e315768d5ed97418c1c6324ac91f7d`.

Failure summary:
`artifacts/fips-perf/fips-4fa502b-nvpn-f330cfe9-fsp-send-owner-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build again resolved hashtree crates to `0.2.64`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2235.3` | `2192.3` | `2123.8` | `2260.9` | `4.770 / 4.770 / 4.765 ms` | `2.040 / 2.040 / 2.038 ms` | `2.190 / 2.190 / 2.185 ms` | `2241768432 / 2274390997` |
| constrained-underlay | `131.8` | `133.5` | `130.7` | `127.5` | `91.600 / 91.600 / 91.638 ms` | `12.600 / 12.600 / 12.590 ms` | `1.400 / 1.400 / 1.403 ms` | `139931903 / 138291831` |
| worker-queue-pressure | `126.8` | `121.7` | `125.4` | `131.5` | `0.405 / 0.405 / 0.405 ms` | `0.354 / 0.354 / 0.354 ms` | `2.130 / 2.130 / 2.132 ms` | `132229484 / 135461339` |
| rx-maintenance-fault | `2248.9` | `2330.4` | `2275.4` | `2219.7` | `2.440 / 2.440 / 2.441 ms` | `2.110 / 2.110 / 2.114 ms` | `1.450 / 1.450 / 1.446 ms` | `2337951071 / 2316422790` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

Selected send-target ownership step at FIPS `c4c3895` plus nvpn `dd866d34`:

FIPS `c4c3895` adds `SelectedSendTarget`, a job-owned value carrying the UDP
socket, optional connected socket, destination sockaddr, and computed target
key used by FMP worker dispatch, fair admission, macOS ordered flow selection,
and flush grouping. This keeps the selected kernel send target from being
rebuilt independently in multiple worker stages.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core fsp_preseal_runs_before_outer_fmp_seal -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  selected_send_target_key_drives_dispatch_and_admission \
  fair_admission_keys_pressure_by_exact_send_target \
  encrypt_worker_dispatch_preserves_single_flow_worker_and_fifo_order \
  flush_batch_routes_each_target_separately
cargo test -p fips-core
./scripts/test-dataplane-safety-linux-docker.sh
```

The new selected-target guard is Linux-only and passed in the focused Linux
container run. The neighboring fair-admission, worker dispatch/FIFO, and batch
routing guards also passed in that focused container run. Local unix FSP
preseal coverage passed. Full local `fips-core` passed with `1405` tests, `0`
failures, and `2` ignored tests; doctests had `2` ignored tests. The full
default Linux deterministic runner passed with the selected-target guard in its
default filter list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-c4c3895-nvpn-dd866d34-selected-send-target-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-selected-send-target-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-c4c3895-nvpn-dd866d34-selected-send-target-smoke/phase-summary.tsv`.
Summary SHA-256:
`06289414c017eb9fafcbc3acf60bf86dc856f119948c7c540dba3cbfddda20ef`.

Failure summary:
`artifacts/fips-perf/fips-c4c3895-nvpn-dd866d34-selected-send-target-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build resolved hashtree crates to `0.2.65`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2114.6` | `2247.9` | `2229.8` | `2179.2` | `2.000 / 2.000 / 2.001 ms` | `1.810 / 1.810 / 1.813 ms` | `1.570 / 1.570 / 1.574 ms` | `2264137021 / 2223635559` |
| constrained-underlay | `133.7` | `131.7` | `133.3` | `133.8` | `87.800 / 87.800 / 87.847 ms` | `93.100 / 93.100 / 93.079 ms` | `0.715 / 0.715 / 0.715 ms` | `141229963 / 142793681` |
| worker-queue-pressure | `101.6` | `103.4` | `103.9` | `109.1` | `0.421 / 0.421 / 0.421 ms` | `0.500 / 0.500 / 0.500 ms` | `1.040 / 1.040 / 1.039 ms` | `113216143 / 114649165` |
| rx-maintenance-fault | `1965.8` | `1973.2` | `2169.1` | `1870.2` | `2.260 / 2.260 / 2.259 ms` | `2.150 / 2.150 / 2.152 ms` | `0.592 / 0.592 / 0.592 ms` | `2111798274 / 1976897558` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

Selected send-batch ownership step at FIPS `480b549` plus nvpn `e4e86055`:

FIPS `480b549` adds `SelectedSendBatch`, a Unix flush-group owner carrying one
selected target, FIFO wire-packet list, and aggregate drop-on-backpressure
policy. The grouping helper uses the selected target key already carried by the
job; same-target packets stay in FIFO order, different sockets or destinations
stay in separate groups, and one non-droppable packet keeps the whole target
batch retryable under backpressure.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core \
  selected_send_batch_owns_target_fifo_and_drop_policy \
  fsp_preseal_runs_before_outer_fmp_seal \
  flush_batch_routes_each_target_separately -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  selected_send_batch_owns_target_fifo_and_drop_policy \
  selected_send_target_key_drives_dispatch_and_admission \
  flush_batch_routes_each_target_separately \
  send_backpressure
cargo test -p fips-core
./scripts/test-dataplane-safety-linux-docker.sh
```

The new selected-batch guard passed locally and in the focused Linux container
run. The neighboring selected-target, Linux batch-routing, and send
backpressure guards also passed in that focused container run. Full local
`fips-core` passed with `1406` tests, `0` failures, and `2` ignored tests;
doctests had `2` ignored tests. The full default Linux deterministic runner
passed with the selected-batch guard in its default filter list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-480b549-nvpn-e4e86055-selected-send-batch-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-selected-send-batch-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-480b549-nvpn-e4e86055-selected-send-batch-smoke/phase-summary.tsv`.
Summary SHA-256:
`216470e3349f83dad6d3c3dfbe512157b216dcf3246698076cd1342fbcb0fe4f`.

Failure summary:
`artifacts/fips-perf/fips-480b549-nvpn-e4e86055-selected-send-batch-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build resolved hashtree crates to `0.2.65`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2091.1` | `1942.0` | `2173.4` | `2145.9` | `2.610 / 2.610 / 2.612 ms` | `1.780 / 1.780 / 1.783 ms` | `2.830 / 2.830 / 2.829 ms` | `2190759915 / 2135106787` |
| constrained-underlay | `129.5` | `129.4` | `129.4` | `126.0` | `13.700 / 13.700 / 13.659 ms` | `98.900 / 98.900 / 98.869 ms` | `4.570 / 4.570 / 4.571 ms` | `136320057 / 138512474` |
| worker-queue-pressure | `123.0` | `124.3` | `123.9` | `125.3` | `0.366 / 0.366 / 0.366 ms` | `0.338 / 0.338 / 0.338 ms` | `4.050 / 4.050 / 4.047 ms` | `130248891 / 132957487` |
| rx-maintenance-fault | `2234.2` | `2198.8` | `2198.6` | `1367.0` | `2.190 / 2.190 / 2.191 ms` | `8.140 / 8.140 / 8.140 ms` | `1.020 / 1.020 / 1.016 ms` | `2241683980 / 1820706643` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

Linux send-attempt ownership step at FIPS `3bb9d64` plus nvpn `7d29592f`:

FIPS `3bb9d64` adds `LinuxSendBatchAttempt`, a Linux batched-send attempt
owner carrying the selected target, remaining packet cursor, send backpressure
pacer, and current-packet bulk-drop policy. The Linux sendmmsg fallback now
advances the owner after successful partial sends or explicit bulk-drop
decisions, instead of keeping those decisions split across the flush loop.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core send_backpressure -- --nocapture
cargo test -p fips-core selected_send_batch_owns_target_fifo_and_drop_policy -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  linux_send_batch_attempt_owns_cursor_and_backpressure_policy \
  selected_send_batch_owns_target_fifo_and_drop_policy \
  selected_send_target_key_drives_dispatch_and_admission \
  send_backpressure \
  flush_batch_routes_each_target_separately
cargo test -p fips-core
./scripts/test-dataplane-safety-linux-docker.sh
cargo check -p fips-core --release
```

The new Linux send-attempt guard passed in the focused Linux container run.
The neighboring selected-target, selected-batch, batch-routing, and send
backpressure guards also passed in that focused container run. Full local
`fips-core` passed with `1406` tests, `0` failures, and `2` ignored tests;
doctests had `2` ignored tests. The full default Linux deterministic runner
passed with the send-attempt guard in its default filter list. A final
test-only cfg warning fix was then checked with focused local/container guards
and `cargo check -p fips-core --release`.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-3bb9d64-nvpn-7d29592f-linux-send-attempt-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-linux-send-attempt-smoke-clean \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-3bb9d64-nvpn-7d29592f-linux-send-attempt-smoke/phase-summary.tsv`.
Summary SHA-256:
`d28d3b24cd3f94b53102866a27f39608c8245aa87e73a8294cb34daf5b7342f4`.

Failure summary:
`artifacts/fips-perf/fips-3bb9d64-nvpn-7d29592f-linux-send-attempt-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build resolved hashtree crates to `0.2.65`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2093.8` | `2158.2` | `2034.0` | `2012.4` | `1.490 / 1.490 / 1.487 ms` | `2.790 / 2.790 / 2.794 ms` | `1.750 / 1.750 / 1.754 ms` | `2154653501 / 2147618618` |
| constrained-underlay | `130.4` | `127.6` | `131.0` | `130.9` | `92.600 / 92.600 / 92.582 ms` | `93.000 / 93.000 / 93.032 ms` | `2.040 / 2.040 / 2.041 ms` | `138939323 / 137735420` |
| worker-queue-pressure | `116.9` | `125.6` | `121.9` | `111.9` | `0.373 / 0.373 / 0.373 ms` | `0.356 / 0.356 / 0.356 ms` | `1.600 / 1.600 / 1.601 ms` | `129546869 / 127483303` |
| rx-maintenance-fault | `1929.0` | `2143.4` | `2257.4` | `2028.0` | `2.190 / 2.190 / 2.191 ms` | `2.290 / 2.290 / 2.293 ms` | `3.060 / 3.060 / 3.064 ms` | `2173567966 / 2171613319` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

Direct send-attempt ownership step at FIPS `40ce258` plus nvpn `589a8cf6`:

FIPS `40ce258` adds `DirectSendBatchAttempt`, a non-Linux direct-send attempt
owner carrying the selected target, remaining packet cursor, send backpressure
pacer, and current-packet bulk-drop policy. The macOS/BSD direct sender now
advances the owner after successful sends or explicit bulk-drop decisions,
instead of keeping those decisions split across the flush loop and per-packet
send helper.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core direct_send_batch_attempt_owns_cursor_and_backpressure_policy -- --nocapture
cargo test -p fips-core send_backpressure -- --nocapture
cargo test -p fips-core mac_queue_tests -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  linux_send_batch_attempt_owns_cursor_and_backpressure_policy \
  selected_send_batch_owns_target_fifo_and_drop_policy \
  selected_send_target_key_drives_dispatch_and_admission \
  send_backpressure \
  flush_batch_routes_each_target_separately
cargo test -p fips-core
cargo check -p fips-core --release
```

The new direct-send attempt guard passed locally on the macOS cfg path. The
neighboring mac worker queue tests and send-backpressure guards also passed.
The focused Linux container run passed the existing Linux send-target,
send-batch, send-attempt, batch-routing, and send-backpressure guards. Full
local `fips-core` passed with `1407` tests, `0` failures, and `2` ignored
tests; doctests had `2` ignored tests. `cargo check -p fips-core --release`
also passed.

Short local-FIPS Docker no-regression smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-40ce258-nvpn-589a8cf6-direct-send-attempt-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-direct-send-attempt-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-40ce258-nvpn-589a8cf6-direct-send-attempt-smoke/phase-summary.tsv`.
Summary SHA-256:
`11371bd902b41dd0ca49233037396c67e95ae0f7decc8ab270e60406afe12a5c`.

Failure summary:
`artifacts/fips-perf/fips-40ce258-nvpn-589a8cf6-direct-send-attempt-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build resolved hashtree crates to `0.2.65`; treat this as
current-state local-FIPS no-regression evidence. This smoke does not exercise
the non-Linux direct sender that changed in FIPS `40ce258`.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `1961.7` | `2031.0` | `2023.8` | `1997.5` | `3.270 / 3.270 / 3.269 ms` | `1.880 / 1.880 / 1.882 ms` | `0.658 / 0.658 / 0.658 ms` | `2054877949 / 2021727699` |
| constrained-underlay | `134.7` | `135.0` | `134.7` | `138.1` | `90.100 / 90.100 / 90.110 ms` | `92.200 / 92.200 / 92.197 ms` | `0.547 / 0.547 / 0.547 ms` | `142389991 / 146927796` |
| worker-queue-pressure | `110.4` | `115.9` | `106.1` | `112.5` | `0.496 / 0.496 / 0.496 ms` | `0.656 / 0.656 / 0.656 ms` | `0.647 / 0.647 / 0.647 ms` | `116591536 / 120485906` |
| rx-maintenance-fault | `2078.9` | `1958.5` | `1972.5` | `2006.7` | `2.310 / 2.310 / 2.309 ms` | `7.660 / 7.660 / 7.658 ms` | `0.547 / 0.547 / 0.547 ms` | `2045011960 / 2032346309` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

Fair admission reservation ownership step at FIPS `cb3640d` plus nvpn
`1ad2ab8e`:

FIPS `cb3640d` adds `FairAdmissionReservation`, a non-macOS fair-worker
reservation token carrying the selected send target key. `FairAdmission` now
returns that token when reserving capacity, enqueue failure releases it, and
receiver drain consumes it before forwarding the job into the local batch. This
keeps pressure accounting tied to the selected target chosen earlier in the send
path instead of recomputing flow identity at release time.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
./scripts/test-dataplane-safety-linux-docker.sh \
  fair_admission_reservation_owns_release_key \
  fair_admission_keys_pressure_by_exact_send_target \
  selected_send_target_key_drives_dispatch_and_admission \
  priority_flow_enters_when_bulk_worker_queue_is_full \
  fair_dispatch_does_not_block_rx_loop_on_full_bulk_queue
cargo test -p fips-core
cargo check -p fips-core --release
./scripts/test-dataplane-safety-linux-docker.sh
```

The new fair-admission reservation guard passed in the focused Linux container
run. The neighboring selected-target, priority-reserve, and fair-dispatch
guards also passed in that focused run. Full local `fips-core` passed with
`1407` tests, `0` failures, and `2` ignored tests; doctests had `2` ignored
tests. `cargo check -p fips-core --release` passed. The full default Linux
deterministic runner passed with the reservation guard in its default filter
list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-cb3640d-nvpn-1ad2ab8e-fair-admission-reservation-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-fair-admission-reservation-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-cb3640d-nvpn-1ad2ab8e-fair-admission-reservation-smoke/phase-summary.tsv`.
Summary SHA-256:
`1bca8d7c6778399a69a3177b7dbcd284a1eb0dbe5798eab0e98f43210a729fd4`.

Failure summary:
`artifacts/fips-perf/fips-cb3640d-nvpn-1ad2ab8e-fair-admission-reservation-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build resolved hashtree crates to `0.2.65`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2215.2` | `2268.7` | `2287.2` | `2223.8` | `1.990 / 1.990 / 1.992 ms` | `2.510 / 2.510 / 2.507 ms` | `2.690 / 2.690 / 2.689 ms` | `2316317568 / 2274060517` |
| constrained-underlay | `137.1` | `136.6` | `134.9` | `137.4` | `93.400 / 93.400 / 93.410 ms` | `89.600 / 89.600 / 89.607 ms` | `1.270 / 1.270 / 1.270 ms` | `145405543 / 148339662` |
| worker-queue-pressure | `129.9` | `93.9` | `117.4` | `119.3` | `1.040 / 1.040 / 1.039 ms` | `0.376 / 0.376 / 0.376 ms` | `2.690 / 2.690 / 2.692 ms` | `127558534 / 114529730` |
| rx-maintenance-fault | `2228.2` | `2145.5` | `2127.1` | `2140.4` | `1.600 / 1.600 / 1.604 ms` | `2.370 / 2.370 / 2.369 ms` | `1.980 / 1.980 / 1.984 ms` | `2198361195 / 2219740311` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

Encrypt worker shard ownership step at FIPS `7fad0e4` plus nvpn `aa84aa32`:

FIPS `7fad0e4` adds `EncryptWorkerShard`, a worker-loop owner for the reusable
local batch vector and bounded drain/flush cycle used by both Linux and macOS
encrypt workers. The worker queues, dispatch hashing, AEAD, and send policy are
unchanged; this names the next shard boundary before moving seal/send state
behind it.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core encrypt_worker_shard_owns_batch_drain_and_flush_error -- --nocapture
cargo test -p fips-core mac_queue_tests -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  encrypt_worker_shard_owns_batch_drain_and_flush_error \
  encrypt_worker_dispatch_preserves_single_flow_worker_and_fifo_order \
  selected_send_batch_owns_target_fifo_and_drop_policy \
  linux_send_batch_attempt_owns_cursor_and_backpressure_policy \
  fair_dispatch_does_not_block_rx_loop_on_full_bulk_queue
cargo test -p fips-core
cargo check -p fips-core --release
./scripts/test-dataplane-safety-linux-docker.sh
```

The new encrypt-worker shard guard passed locally and in the focused Linux
container run. Neighboring Linux dispatch, selected-batch, send-attempt, and
fair-dispatch guards also passed in that focused run. Local `mac_queue_tests`
passed with `6` tests. Full local `fips-core` passed with `1408` tests, `0`
failures, and `2` ignored tests; doctests had `2` ignored tests.
`cargo check -p fips-core --release` passed. The full default Linux
deterministic runner passed with the shard guard in its default filter list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-7fad0e4-nvpn-aa84aa32-encrypt-worker-shard-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-encrypt-worker-shard-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-7fad0e4-nvpn-aa84aa32-encrypt-worker-shard-smoke/phase-summary.tsv`.
Summary SHA-256:
`9714eed8475fca4ffbb348d813159f80e0af96cbb0c1c89410891ad7e044bd27`.

Failure summary:
`artifacts/fips-perf/fips-7fad0e4-nvpn-aa84aa32-encrypt-worker-shard-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build resolved hashtree crates to `0.2.65`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2175.9` | `2219.5` | `2221.2` | `2204.4` | `2.100 / 2.100 / 2.103 ms` | `1.670 / 1.670 / 1.665 ms` | `1.650 / 1.650 / 1.648 ms` | `2252024967 / 2252703616` |
| constrained-underlay | `134.6` | `135.0` | `136.5` | `137.3` | `86.900 / 86.900 / 86.866 ms` | `89.200 / 89.200 / 89.162 ms` | `1.290 / 1.290 / 1.285 ms` | `143986634 / 147225099` |
| worker-queue-pressure | `121.4` | `114.6` | `121.4` | `122.6` | `0.400 / 0.400 / 0.400 ms` | `0.349 / 0.349 / 0.349 ms` | `1.930 / 1.930 / 1.925 ms` | `129413384 / 131548915` |
| rx-maintenance-fault | `2276.2` | `2216.1` | `2230.3` | `2245.7` | `3.100 / 3.100 / 3.099 ms` | `1.950 / 1.950 / 1.948 ms` | `2.210 / 2.210 / 2.210 ms` | `2314882909 / 2298142779` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

Sealed send-packet ownership step at FIPS `7a9b3de5` plus nvpn `e93b94d9`:

FIPS `7a9b3de5` adds `SealedSendPacket`, a seal-to-send owner inside the
encrypt worker. It consumes an `FmpSendJob`, records worker queue wait, applies
optional FSP seal and outer FMP seal, and then carries the selected send target,
final sealed wire packet, and drop-on-backpressure policy to macOS completion
handling or Unix send batching. The worker queues, dispatch hashing, AEAD
counter reservation, and send policy are unchanged.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core sealed_send_packet_owns_target_wire_and_drop_policy -- --nocapture
cargo test -p fips-core encrypt_worker_shard_owns_batch_drain_and_flush_error -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  sealed_send_packet_owns_target_wire_and_drop_policy \
  encrypt_worker_shard_owns_batch_drain_and_flush_error \
  selected_send_batch_owns_target_fifo_and_drop_policy \
  linux_send_batch_attempt_owns_cursor_and_backpressure_policy \
  fair_dispatch_does_not_block_rx_loop_on_full_bulk_queue
cargo test -p fips-core mac_queue_tests -- --nocapture
cargo test -p fips-core fsp_preseal_runs_before_outer_fmp_seal -- --nocapture
cargo test -p fips-core
cargo check -p fips-core --release
./scripts/test-dataplane-safety-linux-docker.sh
```

The new sealed-packet guard passed locally and in the focused Linux container
run. Neighboring shard, selected-batch, send-attempt, and fair-dispatch guards
also passed in that focused run. Local `mac_queue_tests` passed with `6` tests.
Full local `fips-core` passed with `1409` tests, `0` failures, and `2` ignored
tests; doctests had `2` ignored tests. `cargo check -p fips-core --release`
passed. The full default Linux deterministic runner passed with the
sealed-packet guard in its default filter list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-7a9b3de5-nvpn-e93b94d9-sealed-send-packet-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-sealed-send-packet-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-7a9b3de5-nvpn-e93b94d9-sealed-send-packet-smoke/phase-summary.tsv`.
Summary SHA-256:
`739f267caa07e3abdc2af99c9e0ac0124b7b99b9fcb725ccd3dc00c9b8c81398`.

Failure summary:
`artifacts/fips-perf/fips-7a9b3de5-nvpn-e93b94d9-sealed-send-packet-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build resolved hashtree crates to `0.2.65`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `2181.8` | `2261.7` | `2255.5` | `1997.6` | `2.520 / 2.520 / 2.522 ms` | `2.620 / 2.620 / 2.616 ms` | `1.770 / 1.770 / 1.772 ms` | `2289253096 / 2169627183` |
| constrained-underlay | `135.5` | `133.7` | `135.0` | `135.3` | `96.600 / 96.600 / 96.585 ms` | `82.800 / 82.800 / 82.762 ms` | `1.510 / 1.510 / 1.510 ms` | `144058335 / 146026600` |
| worker-queue-pressure | `123.0` | `128.8` | `120.2` | `128.0` | `0.401 / 0.401 / 0.401 ms` | `0.579 / 0.579 / 0.579 ms` | `2.600 / 2.600 / 2.602 ms` | `131849829 / 136357464` |
| rx-maintenance-fault | `2340.5` | `2165.7` | `2219.4` | `2336.5` | `4.260 / 4.260 / 4.255 ms` | `2.240 / 2.240 / 2.236 ms` | `3.100 / 3.100 / 3.097 ms` | `2314695691 / 2309526276` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.

Queued encrypt-worker message ownership step at FIPS `8792215a` plus nvpn
`61d4ad10`:

FIPS `8792215a` makes `QueuedFmpSendJob` own the selected send-target key and
priority/bulk lane captured at queue-message construction time. Dispatch
hashing, non-macOS fair admission, queue selection, and worker drain now consume
that queued-message identity instead of deriving it later from the wrapped
`FmpSendJob`. Worker queues, AEAD sealing, send batching, and drop/backpressure
policy are unchanged.

Deterministic validation from the FIPS safety worktree:

```sh
cargo fmt --check
cargo test -p fips-core queued_fmp_send_job_owns_lane_and_target_key -- --nocapture
cargo test -p fips-core sealed_send_packet_owns_target_wire_and_drop_policy -- --nocapture
cargo test -p fips-core encrypt_worker_shard_owns_batch_drain_and_flush_error -- --nocapture
./scripts/test-dataplane-safety-linux-docker.sh \
  queued_fmp_send_job_owns_lane_and_target_key \
  sealed_send_packet_owns_target_wire_and_drop_policy \
  encrypt_worker_shard_owns_batch_drain_and_flush_error \
  selected_send_batch_owns_target_fifo_and_drop_policy \
  linux_send_batch_attempt_owns_cursor_and_backpressure_policy \
  fair_admission_reservation_owns_release_key \
  fair_dispatch_does_not_block_rx_loop_on_full_bulk_queue
cargo test -p fips-core mac_queue_tests -- --nocapture
cargo test -p fips-core
cargo check -p fips-core --release
./scripts/test-dataplane-safety-linux-docker.sh
```

The new queued-message guard passed locally and in the focused Linux container
run. Neighboring sealed-packet, shard, selected-batch, send-attempt,
fair-admission, and fair-dispatch guards also passed in that focused run. Local
`mac_queue_tests` passed with `6` tests. Full local `fips-core` passed with
`1410` tests, `0` failures, and `2` ignored tests; doctests had `2` ignored
tests. `cargo check -p fips-core --release` passed. The full default Linux
deterministic runner passed with the queued-message guard in its default filter
list.

Short local-FIPS Docker perf smoke:

```sh
NVPN_PATCH_LOCAL_FIPS=1 \
NVPN_FIPS_REPO_PATH="$FIPS_SAFETY_WORKTREE" \
NVPN_PERF_DURATION_SECS=2 \
NVPN_PERF_LOAD_DURATION_SECS=3 \
NVPN_PERF_PING_COUNT=8 \
NVPN_PERF_CONSTRAINED_RATE_MBIT=180 \
NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=4 \
NVPN_PERF_RX_MAINT_FAULT_MS=50 \
NVPN_PERF_OUTPUT_DIR=artifacts/fips-perf/fips-8792215a-nvpn-61d4ad10-queued-message-smoke \
PROJECT_NAME=nostr-vpn-e2e-fips-queued-message-smoke \
./scripts/e2e-fips-perf-regression-docker.sh
```

Phase summary:
`artifacts/fips-perf/fips-8792215a-nvpn-61d4ad10-queued-message-smoke/phase-summary.tsv`.
Summary SHA-256:
`c48de38a2fc9797370fd10827d7d1dc285a27a643e4695c98e92c8dde7b83908`.

Failure summary:
`artifacts/fips-perf/fips-8792215a-nvpn-61d4ad10-queued-message-smoke/failure-summary.tsv`.
Failure-summary SHA-256:
`d8992f9bc73cbfa74b1651bd8621dac5f37b9ac2e49426eca850bbb809caf214`.

The Docker build resolved hashtree crates to `0.2.66`; treat this as
current-state local-FIPS safety evidence, not a pure isolated FIPS-only delta.

The smoke passed all four phases. Direct UDP byte counters advanced on both
nodes, load/post tunnel-ping loss stayed at `0%`, and the worker-pressure phase
showed expected decrypt-worker queue-full/bulk-drop pressure.

| Phase | Fwd Mbps | Rev Mbps | Fwd-load Mbps | Rev-load Mbps | Fwd-load ping p95/p99/max | Rev-load ping p95/p99/max | Post ping p95/p99/max | Direct bytes node-a/node-b |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| clean-underlay | `1774.9` | `2006.6` | `1883.8` | `1864.7` | `2.450 / 2.450 / 2.449 ms` | `2.120 / 2.120 / 2.117 ms` | `0.895 / 0.895 / 0.895 ms` | `1901304160 / 1928653185` |
| constrained-underlay | `131.4` | `130.2` | `136.7` | `138.2` | `43.700 / 43.700 / 43.693 ms` | `84.000 / 84.000 / 83.958 ms` | `1.560 / 1.560 / 1.558 ms` | `141207824 / 144852488` |
| worker-queue-pressure | `89.0` | `50.3` | `52.7` | `77.5` | `7.190 / 7.190 / 7.189 ms` | `0.967 / 0.967 / 0.967 ms` | `3.960 / 3.960 / 3.959 ms` | `75104740 / 77007410` |
| rx-maintenance-fault | `1828.2` | `1694.3` | `1866.4` | `1803.8` | `3.270 / 3.270 / 3.269 ms` | `4.430 / 4.430 / 4.431 ms` | `3.560 / 3.560 / 3.555 ms` | `1825667189 / 1808726906` |

Real Mac-to-Mac Wi-Fi/screenshare soak remains a separate operator-local
validation step on actual Macs.
