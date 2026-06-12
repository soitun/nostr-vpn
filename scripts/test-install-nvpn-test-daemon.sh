#!/usr/bin/env bash
# Self-test macOS test-daemon installer mode selection without sudo or macOS.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

fail() {
  printf 'install nvpn test daemon self-test failed: %s\n' "$*" >&2
  exit 1
}

make_fake_repo() {
  local dir="$1"
  mkdir -p "$dir/scripts" "$dir/bin" "$dir/built"
  cp "$ROOT/scripts/install-nvpn-test-daemon" "$dir/scripts/install-nvpn-test-daemon"
  printf 'lock\n' >"$dir/Cargo.lock"
}

make_fips_fixture() {
  local dir="$1"
  mkdir -p \
    "$dir/crates/fips-core" \
    "$dir/crates/fips-endpoint" \
    "$dir/crates/fips-identity"
  touch \
    "$dir/crates/fips-core/Cargo.toml" \
    "$dir/crates/fips-endpoint/Cargo.toml" \
    "$dir/crates/fips-identity/Cargo.toml"
}

write_stubs() {
  local dir="$1"
  local stubbin="$dir/bin"
  local built_bin="$dir/built/nvpn"

  cat >"$stubbin/uname" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "-s" ]]; then
  printf 'Darwin\n'
else
  /usr/bin/uname "$@"
fi
EOF

  cat >"$stubbin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
for arg in "$@"; do
  case "$arg" in
    build)
      if [[ -n "${NVPN_TEST_MUTATE_LOCK:-}" ]]; then
        printf '# mutated by fake cargo\n' >> Cargo.lock
      fi
      exit 0
      ;;
    metadata)
      printf '{"packages":[{"name":"nvpn","version":"4.0.72"}]}\n'
      exit 0
      ;;
  esac
done
printf 'unexpected fake cargo args: %s\n' "$*" >&2
exit 2
EOF

  cat >"$dir/scripts/build-output-path" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$NVPN_TEST_BUILT_BIN"
EOF

  cat >"$stubbin/xattr" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'xattr %s\n' "$*" >>"$NVPN_TEST_RECORD"
EOF

  cat >"$stubbin/codesign" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'codesign %s\n' "$*" >>"$NVPN_TEST_RECORD"
EOF

  cat >"$stubbin/sudo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'sudo %s\n' "$*" >>"$NVPN_TEST_RECORD"
exec "$@"
EOF

  cat >"$built_bin" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  version)
    printf '4.0.72\n'
    ;;
  service)
    printf 'nvpn %s\n' "$*" >>"$NVPN_TEST_RECORD"
    ;;
  *)
    printf 'unexpected fake nvpn args: %s\n' "$*" >&2
    exit 2
    ;;
esac
EOF

  chmod +x "$stubbin/uname" "$stubbin/cargo" "$dir/scripts/build-output-path" \
    "$stubbin/xattr" "$stubbin/codesign" "$stubbin/sudo" "$built_bin"
}

run_fake_installer() {
  local dir="$1"
  shift
  env \
    PATH="$dir/bin:$PATH" \
    NVPN_TEST_BUILT_BIN="$dir/built/nvpn" \
    NVPN_TEST_RECORD="$dir/record.log" \
    NVPN_TEST_DAEMON_STAGE_ROOT="$dir/stage" \
    "$@" \
    "$dir/scripts/install-nvpn-test-daemon" >"$dir/output.log"
}

test_auto_falls_back_to_service_install() {
  local dir
  dir="$(mktemp -d)"
  make_fake_repo "$dir"
  write_stubs "$dir"

  run_fake_installer \
    "$dir" \
    NVPN_INSTALL_TEST_DAEMON="$dir/missing-helper"

  grep -Fq 'codesign --force --sign - ' "$dir/record.log" \
    || fail "auto fallback did not sign staged binary"
  grep -Fq 'nvpn service install --force --iface utun --mesh-refresh-interval-secs 20' "$dir/record.log" \
    || fail "auto fallback did not use native service install"
  if grep -Fq 'nvpn-install-test-daemon' "$dir/record.log"; then
    fail "auto fallback unexpectedly used helper"
  fi

  rm -rf "$dir"
}

test_helper_mode_requires_and_uses_helper() {
  local dir helper
  dir="$(mktemp -d)"
  helper="$dir/helper"
  make_fake_repo "$dir"
  write_stubs "$dir"
  cat >"$helper" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'helper %s\n' "$*" >>"$NVPN_TEST_RECORD"
EOF
  chmod +x "$helper"

  run_fake_installer \
    "$dir" \
    NVPN_TEST_DAEMON_INSTALL_MODE=helper \
    NVPN_INSTALL_TEST_DAEMON="$helper"

  grep -Fq "helper $dir/stage/" "$dir/record.log" \
    || fail "helper mode did not invoke installer helper"
  if grep -Fq 'nvpn service install' "$dir/record.log"; then
    fail "helper mode unexpectedly used native service install"
  fi

  rm -rf "$dir"
}

test_service_mode_forwards_config_overrides() {
  local dir config
  dir="$(mktemp -d)"
  config="$dir/config.toml"
  make_fake_repo "$dir"
  write_stubs "$dir"
  printf '# config\n' >"$config"

  run_fake_installer \
    "$dir" \
    NVPN_TEST_DAEMON_INSTALL_MODE=service \
    NVPN_TEST_DAEMON_IFACE=utun9 \
    NVPN_TEST_DAEMON_MESH_REFRESH_INTERVAL_SECS=7 \
    NVPN_TEST_DAEMON_CONFIG="$config"

  grep -Fq "nvpn service install --force --iface utun9 --mesh-refresh-interval-secs 7 --config $config" "$dir/record.log" \
    || fail "service mode did not forward config overrides"

  rm -rf "$dir"
}

test_local_fips_run_restores_lockfile() {
  local dir fips
  dir="$(mktemp -d)"
  fips="$dir/fips"
  make_fake_repo "$dir"
  make_fips_fixture "$fips"
  write_stubs "$dir"

  run_fake_installer \
    "$dir" \
    NVPN_INSTALL_TEST_DAEMON="$dir/missing-helper" \
    NVPN_PATCH_LOCAL_FIPS=1 \
    NVPN_FIPS_REPO_PATH="$fips" \
    NVPN_TEST_MUTATE_LOCK=1

  if [[ "$(cat "$dir/Cargo.lock")" != "lock" ]]; then
    fail "local-FIPS run left Cargo.lock modified"
  fi
  grep -Fq 'restored Cargo.lock after local-FIPS cargo run' "$dir/output.log" \
    || fail "local-FIPS run did not report Cargo.lock restore"

  rm -rf "$dir"
}

test_auto_falls_back_to_service_install
test_helper_mode_requires_and_uses_helper
test_service_mode_forwards_config_overrides
test_local_fips_run_restores_lockfile

printf 'install nvpn test daemon self-test passed\n'
