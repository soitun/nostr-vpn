#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LIMIT="${NVPN_SOURCE_FILE_LINE_LIMIT:-1000}"

if ! [[ "$LIMIT" =~ ^[0-9]+$ ]] || ((LIMIT == 0)); then
  echo "NVPN_SOURCE_FILE_LINE_LIMIT must be a positive integer" >&2
  exit 2
fi

status=0
while IFS= read -r -d '' file; do
  rel_path="${file#./}"
  [[ -f "$ROOT_DIR/$rel_path" ]] || continue
  lines="$(wc -l <"$ROOT_DIR/$rel_path")"
  lines="${lines//[[:space:]]/}"
  if ((lines > LIMIT)); then
    if ((status == 0)); then
      printf 'Authored source files over configured line limits:\n' >&2
    fi
    printf '%5d %s (limit %s)\n' "$lines" "$rel_path" "$LIMIT" >&2
    status=1
  fi
done < <(
  cd "$ROOT_DIR"
  git ls-files -z --cached --others --exclude-standard -- \
    '*.rs' '*.swift' '*.kt' '*.cs' '*.xaml' \
    ':(exclude)macos/Bindings/*'
)

if ((status != 0)); then
  exit "$status"
fi

printf 'All authored source files are within configured line limits (default %s).\n' "$LIMIT"
