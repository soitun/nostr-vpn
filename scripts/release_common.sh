#!/usr/bin/env bash

release_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd
}

load_release_env() {
  local root="$1"
  local env_file="${NVPN_RELEASE_ENV_FILE:-$root/release.env}"
  if [[ -f "$env_file" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$env_file"
    set +a
  fi
}

bool_is_true() {
  case "${1:-}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

epoch_to_iso8601() {
  local epoch="$1"
  if date -u -r 0 +"%Y-%m-%dT%H:%M:%SZ" >/dev/null 2>&1; then
    date -u -r "$epoch" +"%Y-%m-%dT%H:%M:%SZ"
  else
    date -u -d "@$epoch" +"%Y-%m-%dT%H:%M:%SZ"
  fi
}

git_short_sha() {
  local root="$1"
  git -C "$root" rev-parse --short=12 HEAD 2>/dev/null || printf '%s\n' "unknown"
}

git_commit_timestamp_utc() {
  local root="$1"
  local epoch
  epoch="$(git -C "$root" log -1 --format=%ct HEAD 2>/dev/null || printf '%s' "")"
  if [[ -n "$epoch" ]]; then
    epoch_to_iso8601 "$epoch"
  else
    printf '%s\n' ""
  fi
}

git_commit_epoch() {
  local root="$1"
  git -C "$root" log -1 --format=%ct HEAD 2>/dev/null || printf '%s\n' ""
}

resolve_source_date_epoch() {
  local root="$1"
  local epoch="${SOURCE_DATE_EPOCH:-}"

  if [[ -z "$epoch" ]]; then
    epoch="$(git_commit_epoch "$root")"
  fi

  if [[ -z "$epoch" ]]; then
    epoch=0
  fi

  if [[ ! "$epoch" =~ ^[0-9]+$ ]]; then
    echo "SOURCE_DATE_EPOCH must be a Unix timestamp, got: $epoch" >&2
    return 1
  fi

  printf '%s\n' "$epoch"
}

enable_deterministic_build_env() {
  local root="$1"
  local epoch

  epoch="$(resolve_source_date_epoch "$root")"
  export SOURCE_DATE_EPOCH="$epoch"

  # Cargo incremental artifacts are cache- and path-sensitive. Keep release
  # outputs from depending on whatever happened to be in the local target dir.
  export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

  # Apple archive tooling honors ZERO_AR_DATE by zeroing static-library member
  # timestamps, which keeps Rust staticlibs stable across rebuilds.
  export ZERO_AR_DATE="${ZERO_AR_DATE:-1}"

  # Keep locale/timezone-sensitive helper output stable when scripts package
  # release assets or derive build metadata.
  export LC_ALL="${LC_ALL:-C}"
  export TZ="${TZ:-UTC}"
}

package_version() {
  local root="$1"

  awk '
    /^\[workspace.package\]/ { inside = 1; next }
    /^\[/ { inside = 0 }
    inside && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "$root/Cargo.toml"
}

semantic_version_code() {
  local version="$1"
  local core major minor patch

  core="${version%%[-+]*}"
  if [[ ! "$core" =~ ^([0-9]+)(\.([0-9]+))?(\.([0-9]+))?$ ]]; then
    return 1
  fi

  major="${BASH_REMATCH[1]}"
  minor="${BASH_REMATCH[3]:-0}"
  patch="${BASH_REMATCH[5]:-0}"

  printf '%d\n' "$((10#$major * 1000000 + 10#$minor * 1000 + 10#$patch))"
}

load_appstoreconnect_defaults() {
  local asc_root="${NVPN_ASC_ROOT:-$HOME/.appstoreconnect}"
  local key_path
  local key_name

  if [[ -z "${NVPN_ASC_AUTH_KEY_PATH:-}" ]]; then
    key_path="$(find "$asc_root/private_keys" -maxdepth 1 -type f -name 'AuthKey_*.p8' 2>/dev/null | sort | head -n 1 || true)"
    if [[ -n "$key_path" ]]; then
      NVPN_ASC_AUTH_KEY_PATH="$key_path"
    fi
  fi

  if [[ -z "${NVPN_ASC_AUTH_KEY_ID:-}" && -n "${NVPN_ASC_AUTH_KEY_PATH:-}" ]]; then
    key_name="$(basename "$NVPN_ASC_AUTH_KEY_PATH")"
    key_name="${key_name#AuthKey_}"
    NVPN_ASC_AUTH_KEY_ID="${key_name%.p8}"
  fi

  if [[ -z "${NVPN_ASC_AUTH_KEY_ISSUER_ID:-}" && -f "$asc_root/issuer.txt" ]]; then
    NVPN_ASC_AUTH_KEY_ISSUER_ID="$(tr -d '[:space:]' < "$asc_root/issuer.txt")"
  fi

  export NVPN_ASC_AUTH_KEY_PATH="${NVPN_ASC_AUTH_KEY_PATH:-}"
  export NVPN_ASC_AUTH_KEY_ID="${NVPN_ASC_AUTH_KEY_ID:-}"
  export NVPN_ASC_AUTH_KEY_ISSUER_ID="${NVPN_ASC_AUTH_KEY_ISSUER_ID:-}"
}

resolve_shared_build_metadata() {
  local root="$1"
  local derived_version_code
  local detected_version

  detected_version="$(package_version "$root" || true)"
  NVPN_APP_VERSION_NAME="${NVPN_APP_VERSION_NAME:-${detected_version:-0.1.0}}"
  derived_version_code="$(semantic_version_code "$NVPN_APP_VERSION_NAME" || true)"
  if [[ -z "${NVPN_APP_VERSION_CODE:-}" ]]; then
    NVPN_APP_VERSION_CODE="${derived_version_code:-1}"
  elif [[ -n "${derived_version_code:-}" && "$NVPN_APP_VERSION_CODE" != "$derived_version_code" ]] && ! bool_is_true "${NVPN_APP_VERSION_CODE_MANUAL:-false}"; then
    echo "Using derived version code $derived_version_code for $NVPN_APP_VERSION_NAME (was $NVPN_APP_VERSION_CODE)." >&2
    NVPN_APP_VERSION_CODE="$derived_version_code"
  fi
  NVPN_BUILD_GIT_SHA="${NVPN_BUILD_GIT_SHA:-$(git_short_sha "$root")}"

  if [[ -z "${NVPN_BUILD_TIMESTAMP_UTC:-}" ]]; then
    if [[ -n "${SOURCE_DATE_EPOCH:-}" ]]; then
      NVPN_BUILD_TIMESTAMP_UTC="$(epoch_to_iso8601 "$SOURCE_DATE_EPOCH")"
    else
      NVPN_BUILD_TIMESTAMP_UTC="$(git_commit_timestamp_utc "$root")"
    fi
  fi

  if [[ -z "${NVPN_BUILD_TIMESTAMP_UTC:-}" ]]; then
    NVPN_BUILD_TIMESTAMP_UTC="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  fi

  export NVPN_APP_VERSION_NAME
  export NVPN_APP_VERSION_CODE
  export NVPN_BUILD_GIT_SHA
  export NVPN_BUILD_TIMESTAMP_UTC
}

release_slug() {
  local channel="$1"
  printf 'NostrVPN-%s-%s+%s-%s' \
    "$channel" \
    "$NVPN_APP_VERSION_NAME" \
    "$NVPN_APP_VERSION_CODE" \
    "$NVPN_BUILD_GIT_SHA"
}

ensure_dir() {
  mkdir -p "$1"
}

require_var() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "$name must be set" >&2
    return 1
  fi
}

write_manifest() {
  local path="$1"
  shift

  : > "$path"
  while [[ $# -gt 1 ]]; do
    printf '%s=%s\n' "$1" "$2" >> "$path"
    shift 2
  done
}
