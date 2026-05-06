# Linux Native Shell

Rust GTK4/libadwaita shell over `nostr-vpn-app-core`.

Run it from the repo root:

```bash
just run-linux
```

The dev target runs inside Docker with a small Xvfb/Fluxbox desktop and VNC on
`localhost:5902`. The VNC password is `nostrvpn`.

Useful commands:

```bash
just linux-build
./tools/run-linux cargo check
./tools/run-linux cargo run
```

The shell currently owns the same core flows as the Swift UI: connect/disconnect,
roster presence, invite QR/import, LAN pairing, exit-node selection, advertised
routes, relays, service install, and diagnostics. Remaining Linux-native work is
desktop portal integration, file/camera QR scanning, tray/status notifier support,
and packaged update UX.

The parity checklist is in `../docs/native-ui-parity-matrix.md`.
