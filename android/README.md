# Android Native Shell

Target shell: Kotlin with Jetpack Compose.

Responsibilities:

- bind to `nostr-vpn-app-core` through the shared JSON C ABI and Android JNI exports
- render `UiState` with native Compose screens
- dispatch `NativeAppAction` values into the shared Rust core
- own Keystore access, camera/image QR scanning, share intents, deep links, and Android `VpnService` permission/control
- preserve the Android VPN runtime behavior while replacing the legacy webview UI

The parity checklist is in `docs/native-ui-parity-matrix.md`.

## Build

```bash
just android-build
```

The build task cross-compiles `nostr-vpn-app-core` for `arm64-v8a` with `cargo-ndk`
and packages it into the debug APK.

## Install

```bash
just android-install
```

## Smoke

```bash
just android-smoke
```

Use `NVPN_ANDROID_SERIAL=<adb-serial>` or `ANDROID_SERIAL=<adb-serial>` when more
than one device or emulator is online. `just android-smoke-vpn` also cycles the
debug VPN action and expects the VPN permission/config path to be usable on that
device. On a fresh install, approve its signed join request from an admin device,
or use `scripts/mobile-android-smoke.sh --vpn-cycle --create-network` for local
OS VPN/TUN coverage without peer dataplane coverage. On trusted local test
devices, add `--accept-vpn-dialog` to tap Android's system VPN consent prompt.
A WireGuard config can be layered on with
`NVPN_ANDROID_DEBUG_WIREGUARD_CONFIG(_FILE)`, but it does not create the
required nvpn network by itself. The command verifies that both `NostrVpnService`
and an Android VPN network become active after debug connect, then saves the
runtime-state JSON, Android VPN `ip -s link` counters, ping-probe output, a
ping latency/loss summary JSON, and a native TUN counter summary JSON under
`artifacts/mobile-android`.

The native shell includes state, join-request, roster, routing, diagnostics,
deep-link, VPN permission surfaces, and Android `VpnService` packet handling
backed by the shared Rust core.

For a production-like physical-device exit test, run
`just mobile-test-kit-exit`. It starts a LAN-reachable WireGuard provider and
profile DNS in Docker, proves default-route and DNS traffic at the provider,
checks public Internet access, disconnects, and requires native device DNS and
Internet to work again. The same command runs the corresponding iOS test.
