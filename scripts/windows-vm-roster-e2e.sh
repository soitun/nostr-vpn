#!/usr/bin/env bash
# Syncs the current tree to the Windows dev VM, builds the debug app, and
# drives the real WPF shell through the roster-acceptance deep link.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_HOST="${NVPN_WINDOWS_SSH_HOST:-${1:-win11-dev}}"
GUEST_REPO="${NVPN_WINDOWS_GUEST_REPO_PATH:-C:\src\nostr-vpn}"

run_ps() {
  local script="$1"
  local encoded
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64 | tr -d '\n')"
  ssh "$SSH_HOST" powershell.exe -NoProfile -EncodedCommand "$encoded"
}

"$ROOT/scripts/windows-vm-git-sync.sh" "$SSH_HOST"
run_ps "\$ErrorActionPreference = 'Stop'
Set-Location '$GUEST_REPO'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\windows-build.ps1 -Configuration Debug
if (\$LASTEXITCODE -ne 0) { exit \$LASTEXITCODE }
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\e2e-desktop-roster-join.ps1
exit \$LASTEXITCODE"

echo "WINDOWS_VM_ROSTER_JOIN_E2E_OK"
