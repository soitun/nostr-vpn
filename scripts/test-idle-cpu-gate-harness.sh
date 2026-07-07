#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/idle-cpu-gate.py"

fail() {
  printf 'idle CPU gate harness failed: %s\n' "$*" >&2
  exit 1
}

assert_status() {
  local expected="$1"
  local label="$2"
  shift 2
  local out status
  out="$(mktemp)"
  set +e
  "$@" >"$out" 2>&1
  status=$?
  set -e
  if [[ "$status" != "$expected" ]]; then
    cat "$out" >&2 || true
    rm -f "$out"
    fail "$label returned $status, expected $expected"
  fi
  rm -f "$out"
}

assert_json_field() {
  local path="$1"
  local expression="$2"
  python3 - "$path" "$expression" <<'PY'
import json
import sys

path, expression = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as fh:
    data = json.load(fh)
if not eval(expression, {"__builtins__": {}}, {"data": data}):
    raise SystemExit(f"assertion failed: {expression}; data={data!r}")
PY
}

tmp_dir="$(mktemp -d)"
sleep_pid=""
busy_pid=""
cleanup() {
  if [[ -n "$sleep_pid" ]]; then
    kill "$sleep_pid" >/dev/null 2>&1 || true
    wait "$sleep_pid" >/dev/null 2>&1 || true
  fi
  if [[ -n "$busy_pid" ]]; then
    kill "$busy_pid" >/dev/null 2>&1 || true
    wait "$busy_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

sleep 10 &
sleep_pid=$!
idle_json="$tmp_dir/idle.json"
assert_status 0 "idle sleep process" \
  "$SCRIPT" host-pid \
    --pid "$sleep_pid" \
    --label "sleep idle" \
    --artifact "$idle_json" \
    --settle-seconds 0 \
    --sample-seconds 0.2 \
    --max-percent 5
assert_json_field "$idle_json" 'data["ok"] is True and data["cpuPercent"] <= data["maxPercent"]'

python3 -c 'while True: pass' &
busy_pid=$!
busy_json="$tmp_dir/busy.json"
assert_status 1 "busy process" \
  "$SCRIPT" host-pid \
    --pid "$busy_pid" \
    --label "busy loop" \
    --artifact "$busy_json" \
    --settle-seconds 0 \
    --sample-seconds 0.4 \
    --max-percent 1
assert_json_field "$busy_json" 'data["ok"] is False and data["cpuPercent"] > data["maxPercent"]'

missing_json="$tmp_dir/missing.json"
assert_status 1 "missing process" \
  "$SCRIPT" host-pid \
    --pid 999999 \
    --label "missing process" \
    --artifact "$missing_json" \
    --settle-seconds 0 \
    --sample-seconds 0.1 \
    --max-percent 5
assert_json_field "$missing_json" 'data["ok"] is False and "error" in data'

printf 'idle CPU gate harness passed\n'
