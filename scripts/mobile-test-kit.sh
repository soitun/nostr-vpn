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

Set NVPN_FIPS_REPO_PATH=/path/to/fips when testing mobile Rust code against
unreleased local FIPS crates. Cargo.lock is restored after local-FIPS cargo
runs so platform iteration does not leave lockfile churn behind.

Simulator mode verifies app build/install/launch. Real VPN dataplane checks
need physical devices; first-run OS VPN permission prompts may require a manual
approval before the debug connect/disconnect cycle can run unattended.
EOF
}

LOCK_SNAPSHOT=""
cargo_config_args=()

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
}

prepare_lock_restore() {
  [[ -z "$LOCK_SNAPSHOT" ]] || return 0
  LOCK_SNAPSHOT="$(mktemp)"
  cp -p "$ROOT/Cargo.lock" "$LOCK_SNAPSHOT"
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

prepare_cargo_config() {
  cargo_config_args=()
  if [[ -n "${NVPN_FIPS_REPO_PATH:-}" ]]; then
    local fips_path
    fips_path="$(validated_fips_repo_path)"
    prepare_lock_restore
    cargo_config_args+=(
      --config "patch.crates-io.fips-core.path=\"$fips_path/crates/fips-core\""
      --config "patch.crates-io.fips-endpoint.path=\"$fips_path/crates/fips-endpoint\""
      --config "patch.crates-io.fips-identity.path=\"$fips_path/crates/fips-identity\""
    )
    printf 'using local FIPS crates from %s\n' "$fips_path"
  fi
}

cargo_test() {
  prepare_cargo_config
  if ((${#cargo_config_args[@]})); then
    (cd "$ROOT" && cargo test "${cargo_config_args[@]}" "$@")
  else
    (cd "$ROOT" && cargo test "$@")
  fi
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
