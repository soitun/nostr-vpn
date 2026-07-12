#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/idle-cpu-gate.py"
RELEASE_GATE="$ROOT_DIR/scripts/release-gate.sh"

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

fake_xcrun="$tmp_dir/xcrun"
cat >"$fake_xcrun" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
action="${2:-}"
input=""
output=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --input) input="$2"; shift ;;
    --output) output="$2"; shift ;;
  esac
  shift
done
if [[ "$action" == "record" ]]; then
  mkdir -p "$output"
  exit 0
fi
cpu_ns=1000000000
if [[ "$input" == *end.trace ]]; then
  cpu_ns="${NVPN_FAKE_IOS_CPU_END_NS:-1002000000}"
fi
cat >"$output" <<XML
<trace-query-result><node><row><start-time/><process fmt="Nostr VPN Tunnel (42)"><pid>42</pid></process><event-time/><boolean/><pid/><process-uid/><duration-on-core>${cpu_ns}</duration-on-core></row></node></trace-query-result>
XML
SH
chmod +x "$fake_xcrun"

ios_json="$tmp_dir/ios.json"
assert_status 0 "idle iOS process" \
  "$SCRIPT" ios-process \
    --xcrun "$fake_xcrun" \
    --device test-phone \
    --process-pattern '^Nostr VPN Tunnel$' \
    --label "iOS packet tunnel" \
    --artifact "$ios_json" \
    --settle-seconds 0 \
    --sample-seconds 0.1 \
    --snapshot-seconds 0.1 \
    --max-percent 5
assert_json_field "$ios_json" 'data["ok"] is True and data["mode"] == "ios-process"'

ios_busy_json="$tmp_dir/ios-busy.json"
assert_status 1 "busy iOS process" \
  env NVPN_FAKE_IOS_CPU_END_NS=1100000000 \
  "$SCRIPT" ios-process \
    --xcrun "$fake_xcrun" \
    --device test-phone \
    --process-pattern '^Nostr VPN Tunnel$' \
    --label "iOS packet tunnel" \
    --artifact "$ios_busy_json" \
    --settle-seconds 0 \
    --sample-seconds 0.1 \
    --snapshot-seconds 0.1 \
    --max-percent 5
assert_json_field "$ios_busy_json" 'data["ok"] is False and data["cpuPercent"] > 5'

grep -Fq './scripts/idle-cpu-gate.py ios-process' "$RELEASE_GATE" \
  || fail "release gate does not run the iOS packet-tunnel idle CPU check"
grep -Fq './scripts/mobile-ios-smoke.sh simulator' "$RELEASE_GATE" \
  || fail "release gate does not run the iOS app idle CPU smoke"
grep -Fq './scripts/mobile-android-smoke.sh' "$RELEASE_GATE" \
  || fail "release gate does not run the Android idle CPU smoke"

printf 'idle CPU gate harness passed\n'
