#!/usr/bin/env bash

load_mobile_env() {
  local root="$1"
  local env_file="${NVPN_MOBILE_ENV_FILE:-$root/.env.mobile.local}"

  if [[ -f "$env_file" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$env_file"
    set +a
  fi
}
