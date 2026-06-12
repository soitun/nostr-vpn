#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LIMIT="${NVPN_RUST_FILE_LINE_LIMIT:-1000}"

if ! [[ "$LIMIT" =~ ^[0-9]+$ ]] || ((LIMIT == 0)); then
  echo "NVPN_RUST_FILE_LINE_LIMIT must be a positive integer" >&2
  exit 2
fi

status=0
while IFS= read -r -d '' file; do
  lines="$(wc -l <"$file")"
  lines="${lines//[[:space:]]/}"
  if ((lines > LIMIT)); then
    if ((status == 0)); then
      printf 'Rust source files over %s lines:\n' "$LIMIT" >&2
    fi
    printf '%5d %s\n' "$lines" "${file#"$ROOT_DIR/"}" >&2
    status=1
  fi
done < <(
  cd "$ROOT_DIR"
  find . \( -type d \( -name .git -o -name target \) -prune \) -o -type f -name '*.rs' -print0
)

if ((status != 0)); then
  exit "$status"
fi

printf 'All Rust source files are at or below %s lines.\n' "$LIMIT"
