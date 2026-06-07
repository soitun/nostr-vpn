#!/usr/bin/env bash
# Run the native Windows WG-exit dataplane self-test on an SSH-reachable VM.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_HOST="${NVPN_WINDOWS_SSH_HOST:-${1:-win11-dev}}"
GUEST_REPO="${NVPN_WINDOWS_GUEST_REPO_PATH:-C:\\src\\nostr-vpn}"

run_ps() {
  local script="$1"
  local encoded
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64)"
  ssh "$SSH_HOST" powershell.exe -NoProfile -EncodedCommand "$encoded"
}

"$ROOT/scripts/windows-vm-git-sync.sh" "$SSH_HOST"

run_ps "\$ErrorActionPreference = 'Stop'
Set-Location '$GUEST_REPO'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-build.ps1 -Configuration Debug
if (\$LASTEXITCODE -ne 0) { exit \$LASTEXITCODE }
\$Bin = Join-Path (Resolve-Path .) 'target\\debug\\nvpn.exe'
if (!(Test-Path \$Bin)) { throw \"Missing nvpn.exe: \$Bin\" }
\$IsAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (!\$IsAdmin) { throw 'Windows WG exit e2e requires an elevated/Admin SSH session for Wintun and route changes' }
& \$Bin wg-upstream-test --self-test --timeout-secs 15 --scoped-host 10.99.99.1 --ping-count 3
if (\$LASTEXITCODE -ne 0) { exit \$LASTEXITCODE }
Write-Host 'WINDOWS_WIREGUARD_EXIT_E2E_OK'"
