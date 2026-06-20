#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LIMIT="${NVPN_RUST_FILE_LINE_LIMIT:-1000}"
ALLOWLIST_FILE="$ROOT_DIR/scripts/check-rust-file-lines.allowlist"

if ! [[ "$LIMIT" =~ ^[0-9]+$ ]] || ((LIMIT == 0)); then
  echo "NVPN_RUST_FILE_LINE_LIMIT must be a positive integer" >&2
  exit 2
fi

allowlist_limit_for() {
  local rel_path="$1"
  local path cap rest

  [[ -f "$ALLOWLIST_FILE" ]] || return 1

  while read -r path cap rest || [[ -n "$path" ]]; do
    [[ -z "$path" || "${path:0:1}" == "#" ]] && continue
    if [[ "$path" == "$rel_path" ]]; then
      printf '%s\n' "$cap"
      return 0
    fi
  done <"$ALLOWLIST_FILE"

  return 1
}

if [[ -f "$ALLOWLIST_FILE" ]]; then
  while read -r path cap rest || [[ -n "$path" ]]; do
    [[ -z "$path" || "${path:0:1}" == "#" ]] && continue
    if ! [[ "$cap" =~ ^[0-9]+$ ]] || ((cap <= LIMIT)); then
      printf 'Invalid Rust line allowlist cap for %s: %s\n' "$path" "$cap" >&2
      exit 2
    fi
    if [[ ! -f "$ROOT_DIR/$path" ]]; then
      printf 'Rust line allowlist path does not exist: %s\n' "$path" >&2
      exit 2
    fi
  done <"$ALLOWLIST_FILE"
fi

status=0
while IFS= read -r -d '' file; do
  rel_path="${file#./}"
  lines="$(wc -l <"$ROOT_DIR/$rel_path")"
  lines="${lines//[[:space:]]/}"
  effective_limit="$LIMIT"
  limit_label="$LIMIT"
  if allowlist_limit="$(allowlist_limit_for "$rel_path")"; then
    effective_limit="$allowlist_limit"
    limit_label="$allowlist_limit allowlisted"
  fi
  if ((lines > effective_limit)); then
    if ((status == 0)); then
      printf 'Rust source files over configured line limits:\n' >&2
    fi
    printf '%5d %s (limit %s)\n' "$lines" "$rel_path" "$limit_label" >&2
    status=1
  fi
done < <(
  cd "$ROOT_DIR"
  find . \( -type d \( -name .git -o -name target \) -prune \) -o -type f -name '*.rs' -print0
)

if ((status != 0)); then
  exit "$status"
fi

printf 'All Rust source files are within configured line limits (default %s).\n' "$LIMIT"
