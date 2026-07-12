#!/usr/bin/env bash

nvpn_restore_local_fips_workspace() {
  if [[ -n "${NVPN_LOCAL_FIPS_LOCK_SNAPSHOT:-}" \
        && -f "$NVPN_LOCAL_FIPS_LOCK_SNAPSHOT" \
        && -f "$NVPN_LOCAL_FIPS_ROOT/Cargo.lock" ]]; then
    if ! cmp -s "$NVPN_LOCAL_FIPS_LOCK_SNAPSHOT" "$NVPN_LOCAL_FIPS_ROOT/Cargo.lock"; then
      cp -p "$NVPN_LOCAL_FIPS_LOCK_SNAPSHOT" "$NVPN_LOCAL_FIPS_ROOT/Cargo.lock"
      printf 'restored Cargo.lock after local-FIPS cargo run\n'
    fi
  fi
  if [[ -n "${NVPN_LOCAL_FIPS_MANIFEST_SNAPSHOT:-}" \
        && -f "$NVPN_LOCAL_FIPS_MANIFEST_SNAPSHOT" \
        && -f "$NVPN_LOCAL_FIPS_ROOT/Cargo.toml" ]]; then
    if ! cmp -s "$NVPN_LOCAL_FIPS_MANIFEST_SNAPSHOT" "$NVPN_LOCAL_FIPS_ROOT/Cargo.toml"; then
      cp -p "$NVPN_LOCAL_FIPS_MANIFEST_SNAPSHOT" "$NVPN_LOCAL_FIPS_ROOT/Cargo.toml"
      printf 'restored Cargo.toml after local-FIPS cargo run\n'
    fi
  fi
}

nvpn_validated_fips_repo_path() {
  local fips_path="${NVPN_FIPS_REPO_PATH:-}"
  [[ -n "$fips_path" ]] || return 1
  if [[ ! -d "$fips_path/crates/fips-core" \
        || ! -d "$fips_path/crates/fips-endpoint" \
        || ! -d "$fips_path/crates/fips-identity" ]]; then
    echo "NVPN_FIPS_REPO_PATH must point at a fips checkout with fips-core, fips-endpoint, and fips-identity" >&2
    exit 1
  fi
  printf '%s\n' "$fips_path"
}

nvpn_prepare_local_fips_workspace() {
  [[ -n "${NVPN_FIPS_REPO_PATH:-}" ]] || return 0
  [[ -z "${NVPN_LOCAL_FIPS_PREPARED:-}" ]] || return 0

  NVPN_LOCAL_FIPS_ROOT="$1"
  local fips_path
  fips_path="$(nvpn_validated_fips_repo_path)"

  NVPN_LOCAL_FIPS_LOCK_SNAPSHOT="$(mktemp)"
  NVPN_LOCAL_FIPS_MANIFEST_SNAPSHOT="$(mktemp)"
  cp -p "$NVPN_LOCAL_FIPS_ROOT/Cargo.lock" "$NVPN_LOCAL_FIPS_LOCK_SNAPSHOT"
  cp -p "$NVPN_LOCAL_FIPS_ROOT/Cargo.toml" "$NVPN_LOCAL_FIPS_MANIFEST_SNAPSHOT"
  trap nvpn_restore_local_fips_workspace EXIT

  printf '\n[patch.crates-io]\nfips-core = { path = "%s" }\nfips-endpoint = { path = "%s" }\nfips-identity = { path = "%s" }\n' \
    "$fips_path/crates/fips-core" \
    "$fips_path/crates/fips-endpoint" \
    "$fips_path/crates/fips-identity" \
    >>"$NVPN_LOCAL_FIPS_ROOT/Cargo.toml"

  NVPN_LOCAL_FIPS_PREPARED=1
  printf 'using local FIPS crates from %s\n' "$fips_path"
}
