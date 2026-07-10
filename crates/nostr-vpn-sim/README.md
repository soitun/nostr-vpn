# nostr-vpn-sim

Production-backed, in-process adversarial simulations for Nostr VPN.

The simulator starts real `FipsEndpoint` nodes over FIPS `SimNetwork` and the
real Nostr VPN control-pubsub runtime. It does not create system TUN devices,
spawn daemons, or exercise the UI.

The default scenario starts 100 instances, publishes an honest baseline event,
injects unanswered inventory spam and valid signed rating spam from 20 peers,
then measures honest event delivery under pressure.

```sh
cargo run -p nostr-vpn-sim
cargo test -p nostr-vpn-sim hundred_instance -- --ignored --nocapture
```

The 12-instance full-path scenario runs in the normal test suite. The
100-instance scenario is an explicit adversarial lane because it takes roughly
34 seconds on a development machine.
