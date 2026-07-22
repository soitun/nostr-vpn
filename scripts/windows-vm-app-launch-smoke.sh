#!/usr/bin/env bash
# Build the Windows installer on an SSH-reachable Windows VM, install it, and
# verify NostrVpn.Windows starts and stays alive instead of exiting at startup.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_HOST="${NVPN_WINDOWS_SSH_HOST:-${1:-win11-dev}}"
SSH_JUMP="${NVPN_WINDOWS_SSH_JUMP:-}"
SSH_PROXY_COMMAND="${NVPN_WINDOWS_SSH_PROXY_COMMAND:-}"
GUEST_REPO="${NVPN_WINDOWS_GUEST_REPO_PATH:-C:\\src\\nostr-vpn}"
GUEST_FIPS_REPO="${NVPN_WINDOWS_GUEST_FIPS_REPO_PATH:-C:\\src\\fips}"
GUEST_ARTIFACT_ROOT="${GUEST_ARTIFACT_ROOT:-C:\\src\\nostr-vpn\\artifacts}"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$ROOT/artifacts}"
SMOKE_TAG="${NVPN_WINDOWS_APP_SMOKE_TAG:-v0.0.0}"

mkdir -p "$ARTIFACT_ROOT"

ssh_command() {
  SSH_CMD=(ssh -o BatchMode=yes)
  if [[ -n "$SSH_PROXY_COMMAND" ]]; then
    SSH_CMD+=(-o "ProxyCommand=$SSH_PROXY_COMMAND")
  elif [[ -n "$SSH_JUMP" ]]; then
    SSH_CMD+=(-J "$SSH_JUMP")
  fi
  SSH_CMD+=("$SSH_HOST")
}

run_ps() {
  local script="$1"
  local encoded
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64 | tr -d '\n')"
  ssh_command
  "${SSH_CMD[@]}" powershell.exe -NoProfile -EncodedCommand "$encoded"
}

case "${NVPN_WINDOWS_SKIP_GIT_SYNC:-0}" in
  1|true|TRUE|True|yes|YES|Yes|on|ON|On)
    echo "Skipping Windows VM git sync; release-gate lane already synced the candidate."
    ;;
  *)
    "$ROOT/scripts/windows-vm-git-sync.sh" "$SSH_HOST"
    ;;
esac

run_ps "\$ErrorActionPreference = 'Stop'
Set-Location '$GUEST_REPO'
New-Item -ItemType Directory -Force -Path '$GUEST_ARTIFACT_ROOT' | Out-Null
if ('${NVPN_FIPS_REPO_PATH:-}' -ne '') { \$env:NVPN_FIPS_REPO_PATH = '$GUEST_FIPS_REPO' }
\$env:CARGO_TARGET_DIR = Join-Path '$GUEST_ARTIFACT_ROOT' 'windows-smoke-cargo'
\$targetPrefix = [IO.Path]::GetFullPath(\$env:CARGO_TARGET_DIR).TrimEnd([char]92) + [char]92
Get-CimInstance Win32_Process -Filter \"Name = 'nvpn.exe'\" |
  Where-Object {
    \$_.ExecutablePath -and
    [IO.Path]::GetFullPath(\$_.ExecutablePath).StartsWith(\$targetPrefix, [StringComparison]::OrdinalIgnoreCase)
  } |
  ForEach-Object { Stop-Process -Id \$_.ProcessId -Force -ErrorAction Stop }
\$installer = Join-Path '$GUEST_ARTIFACT_ROOT' 'nostr-vpn-$SMOKE_TAG-windows-x64-setup.exe'
Remove-Item -Force \$installer -ErrorAction SilentlyContinue
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-build.ps1 -Configuration Release -Installer -Tag '$SMOKE_TAG' -OutputDir '$GUEST_ARTIFACT_ROOT'
if (\$LASTEXITCODE -ne 0) { throw ('windows-build.ps1 failed with exit code {0}' -f \$LASTEXITCODE) }
if (!(Test-Path \$installer)) { throw ('Windows installer was not created: {0}' -f \$installer) }
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-installer-smoke.ps1 -InstallerPath \$installer -ArtifactRoot '$GUEST_ARTIFACT_ROOT'
if (\$LASTEXITCODE -ne 0) { throw ('windows-installer-smoke.ps1 failed with exit code {0}' -f \$LASTEXITCODE) }
\$candidate = Join-Path \$env:CARGO_TARGET_DIR 'release\\nvpn.exe'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-daemon-idle-cpu.ps1 -Bin \$candidate -ArtifactRoot '$GUEST_ARTIFACT_ROOT'
if (\$LASTEXITCODE -ne 0) { throw ('windows-daemon-idle-cpu.ps1 failed with exit code {0}' -f \$LASTEXITCODE) }
exit \$LASTEXITCODE"

echo "WINDOWS_VM_APP_LAUNCH_SMOKE_OK"
