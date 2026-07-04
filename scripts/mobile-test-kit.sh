#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-test-kit.sh [rust|fast|simulator|device]

Modes:
  rust       Run shared Rust mobile/core tests only.
  fast       Run Rust tests plus Android and iOS debug builds.
  simulator  Run fast checks, then iOS simulator and Android adb launch smokes.
  device     Run opt-in physical-device smokes using env-provided identifiers.

Device identifiers are intentionally not stored in the repo. Use environment
variables such as NVPN_ANDROID_SERIAL and NVPN_IOS_DEVICE when needed, or copy
.env.mobile.example to .env.mobile.local for local ignored values. Set
NVPN_IOS_INSTALL=1 when physical iOS should build/install the exact current
development-signed app before launching; set NVPN_IOS_DEBUG_CREATE_NETWORK=1
for local iOS Packet Tunnel coverage without a private invite fixture.
Android --vpn-cycle validates the app-private Rust runtime-state file after the
OS VPN network becomes active; tune with NVPN_ANDROID_RUNTIME_STATE_WAIT_SECS
or NVPN_ANDROID_RUNTIME_STATE_MAX_AGE_SECS if a slow device needs it. It also
requires tunPacketsRead to increase after a small shell ping burst; disable only
for device images without ping via NVPN_ANDROID_TUN_PACKET_PROBE=0.

Set NVPN_FIPS_REPO_PATH=/path/to/fips when testing mobile Rust code against
unreleased local FIPS crates. Cargo.toml and Cargo.lock are restored after
local-FIPS cargo runs so platform iteration does not leave workspace churn
behind.

Simulator mode verifies app build/install/launch. Real VPN dataplane checks
need physical devices; first-run OS VPN permission prompts may require a manual
approval before the debug connect/disconnect cycle can run unattended.
EOF
}

LOCK_SNAPSHOT=""
MANIFEST_SNAPSHOT=""

fail() {
  printf 'mobile test kit failed: %s\n' "$*" >&2
  exit 1
}

restore_lock() {
  if [[ -n "$LOCK_SNAPSHOT" && -f "$LOCK_SNAPSHOT" && -f "$ROOT/Cargo.lock" ]]; then
    if ! cmp -s "$LOCK_SNAPSHOT" "$ROOT/Cargo.lock"; then
      cp -p "$LOCK_SNAPSHOT" "$ROOT/Cargo.lock"
      printf 'restored Cargo.lock after local-FIPS cargo run\n'
    fi
  fi
  if [[ -n "$MANIFEST_SNAPSHOT" && -f "$MANIFEST_SNAPSHOT" && -f "$ROOT/Cargo.toml" ]]; then
    if ! cmp -s "$MANIFEST_SNAPSHOT" "$ROOT/Cargo.toml"; then
      cp -p "$MANIFEST_SNAPSHOT" "$ROOT/Cargo.toml"
      printf 'restored Cargo.toml after local-FIPS cargo run\n'
    fi
  fi
}

prepare_lock_restore() {
  [[ -z "$LOCK_SNAPSHOT" ]] || return 0
  LOCK_SNAPSHOT="$(mktemp)"
  MANIFEST_SNAPSHOT="$(mktemp)"
  cp -p "$ROOT/Cargo.lock" "$LOCK_SNAPSHOT"
  cp -p "$ROOT/Cargo.toml" "$MANIFEST_SNAPSHOT"
  trap restore_lock EXIT
}

validated_fips_repo_path() {
  local fips_path="${NVPN_FIPS_REPO_PATH:-}"
  [[ -n "$fips_path" ]] || fail "NVPN_FIPS_REPO_PATH is empty"
  [[ -d "$fips_path/crates/fips-core" ]] || fail "missing $fips_path/crates/fips-core"
  [[ -d "$fips_path/crates/fips-endpoint" ]] || fail "missing $fips_path/crates/fips-endpoint"
  [[ -d "$fips_path/crates/fips-identity" ]] || fail "missing $fips_path/crates/fips-identity"
  printf '%s\n' "$fips_path"
}

prepare_local_fips_manifest() {
  if [[ -n "${NVPN_FIPS_REPO_PATH:-}" ]]; then
    local fips_path
    fips_path="$(validated_fips_repo_path)"
    prepare_lock_restore
    NVPN_LOCAL_FIPS_CORE_PATH="$fips_path/crates/fips-core" \
    NVPN_LOCAL_FIPS_ENDPOINT_PATH="$fips_path/crates/fips-endpoint" \
    NVPN_LOCAL_FIPS_IDENTITY_PATH="$fips_path/crates/fips-identity" \
      perl -0pi -e '
        s#fips-core = \{ version = "([^"]+)", path = "[^"]*" \}#fips-core = { version = "$1", path = "$ENV{NVPN_LOCAL_FIPS_CORE_PATH}" }#;
        s#fips-endpoint = \{ version = "([^"]+)", path = "[^"]*" \}#fips-endpoint = { version = "$1", path = "$ENV{NVPN_LOCAL_FIPS_ENDPOINT_PATH}" }#;
        s#fips-identity = \{ version = "([^"]+)", path = "[^"]*" \}#fips-identity = { version = "$1", path = "$ENV{NVPN_LOCAL_FIPS_IDENTITY_PATH}" }#;
      ' "$ROOT/Cargo.toml"
    printf 'using local FIPS crates from %s\n' "$fips_path"
  fi
}

cargo_test() {
  prepare_local_fips_manifest
  (cd "$ROOT" && cargo test "$@")
}

run_rust() {
  cargo_test -p nostr-vpn-app-core
  cargo_test -p nvpn platform_routing
}

run_android_build() {
  "$ROOT/tools/run-android" build
}

run_ios_build() {
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "Skipping iOS build on non-Darwin host"
    return
  fi
  "$ROOT/tools/run-ios" build
}

run_fast() {
  run_rust
  run_android_build
  run_ios_build
}

mode="${1:-fast}"
case "$mode" in
  rust)
    run_rust
    ;;
  fast)
    run_fast
    ;;
  simulator|sim)
    run_fast
    "$ROOT/scripts/mobile-ios-smoke.sh" simulator
    "$ROOT/scripts/mobile-android-smoke.sh" --no-build
    ;;
  device)
    "$ROOT/scripts/mobile-android-smoke.sh" --vpn-cycle
    "$ROOT/scripts/mobile-ios-smoke.sh" device --vpn-cycle
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage
    exit 2
    ;;
esac
