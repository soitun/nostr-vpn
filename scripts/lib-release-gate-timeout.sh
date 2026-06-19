#!/usr/bin/env bash

release_gate_timeout_is_disabled() {
  case "${1:-}" in
    ""|0|false|FALSE|False|no|NO|No|off|OFF|Off|none|NONE|None)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

release_gate_run_with_timeout() {
  local label="$1"
  local timeout_secs="$2"
  shift 2

  if (($# == 0)); then
    printf 'release gate timeout failed: missing command for %s\n' "$label" >&2
    return 2
  fi

  if release_gate_timeout_is_disabled "$timeout_secs"; then
    "$@"
    return $?
  fi

  if [[ ! "$timeout_secs" =~ ^[0-9]+$ ]]; then
    printf 'release gate timeout failed: %s timeout must be seconds or off, got %s\n' "$label" "$timeout_secs" >&2
    return 2
  fi

  printf 'Running %s with %ss timeout\n' "$label" "$timeout_secs"

  local had_errexit=0
  case "$-" in
    *e*)
      had_errexit=1
      set +e
      ;;
  esac

  local status
  if command -v timeout >/dev/null 2>&1; then
    timeout --kill-after=5s "$timeout_secs" "$@"
    status=$?
  elif command -v gtimeout >/dev/null 2>&1; then
    gtimeout --kill-after=5s "$timeout_secs" "$@"
    status=$?
  else
    local marker watchdog pid
    marker="$(mktemp "${TMPDIR:-/tmp}/nvpn-release-gate-timeout.XXXXXX")"
    rm -f "$marker"
    "$@" &
    pid=$!
    (
      sleep "$timeout_secs"
      if kill -0 "$pid" >/dev/null 2>&1; then
        printf '%s timed out after %ss\n' "$label" "$timeout_secs" >&2
        : >"$marker"
        kill "$pid" >/dev/null 2>&1 || true
        sleep 5
        kill -9 "$pid" >/dev/null 2>&1 || true
      fi
    ) &
    watchdog=$!
    wait "$pid"
    status=$?
    kill "$watchdog" >/dev/null 2>&1 || true
    wait "$watchdog" >/dev/null 2>&1 || true
    if [[ -f "$marker" ]]; then
      status=124
    fi
    rm -f "$marker"
  fi

  if ((status == 124 || status == 137)); then
    printf '%s timed out after %ss\n' "$label" "$timeout_secs" >&2
  fi

  if ((had_errexit)); then
    set -e
  fi
  return "$status"
}
