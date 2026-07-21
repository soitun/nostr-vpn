#!/usr/bin/env bash
# Run the native Windows WG-exit tests on an SSH-reachable disposable VM.
#
# Without a provider profile this runs the secretless scoped Wintun self-test.
# Set NVPN_WINDOWS_WG_EXIT_CONFIG_FILE to also exercise real Internet routing:
# Direct -> provider WireGuard -> Direct. Set NVPN_WINDOWS_REQUIRE_WG_DIRECT_E2E=1
# when a skipped provider-backed test must fail the run.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_HOST="${NVPN_WINDOWS_SSH_HOST:-${1:-win11-dev}}"
SSH_JUMP="${NVPN_WINDOWS_SSH_JUMP:-}"
SSH_PROXY_COMMAND="${NVPN_WINDOWS_SSH_PROXY_COMMAND:-}"
GUEST_REPO="${NVPN_WINDOWS_GUEST_REPO_PATH:-C:\\src\\nostr-vpn}"
GUEST_FIPS_REPO="${NVPN_WINDOWS_GUEST_FIPS_REPO_PATH:-C:\\src\\fips}"
GUEST_CONFIG="${NVPN_WINDOWS_E2E_CONFIG:-C:\\ProgramData\\Nostr VPN\\config.toml}"
PROVIDER_CONFIG="${NVPN_WINDOWS_WG_EXIT_CONFIG_FILE:-${NVPN_WG_EXIT_CONFIG_FILE:-}}"
REQUIRE_PROVIDER_E2E="${NVPN_WINDOWS_REQUIRE_WG_DIRECT_E2E:-0}"
PROBE_URL="${NVPN_WINDOWS_E2E_INTERNET_URL:-https://example.com/}"
WAIT_SECS="${NVPN_WINDOWS_E2E_WAIT_SECS:-60}"
SETTLE_SECS="${NVPN_WINDOWS_E2E_SETTLE_SECS:-3}"
REMOTE_PROVIDER_CONFIG=""

ps_quote() {
  local value="${1//\'/\'\'}"
  printf "'%s'" "$value"
}

ssh_command() {
  SSH_CMD=(ssh -o BatchMode=yes)
  if [[ -n "$SSH_PROXY_COMMAND" ]]; then
    SSH_CMD+=(-o "ProxyCommand=$SSH_PROXY_COMMAND")
  elif [[ -n "$SSH_JUMP" ]]; then
    SSH_CMD+=(-J "$SSH_JUMP")
  fi
  SSH_CMD+=("$SSH_HOST")
}

scp_command() {
  SCP_CMD=(scp -q -o BatchMode=yes)
  if [[ -n "$SSH_PROXY_COMMAND" ]]; then
    SCP_CMD+=(-o "ProxyCommand=$SSH_PROXY_COMMAND")
  elif [[ -n "$SSH_JUMP" ]]; then
    SCP_CMD+=(-J "$SSH_JUMP")
  fi
}

run_ps() {
  local script="$1"
  local encoded
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64 | tr -d '\n')"
  ssh_command
  "${SSH_CMD[@]}" powershell.exe -NoProfile -EncodedCommand "$encoded"
}

copy_to_guest() {
  local source="$1"
  local destination="$2"
  scp_command
  "${SCP_CMD[@]}" "$source" "${SSH_HOST}:${destination//\\//}"
}

provider_e2e_required() {
  case "$REQUIRE_PROVIDER_E2E" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) return 0 ;;
    0|false|FALSE|False|no|NO|No|off|OFF|Off|"") return 1 ;;
    *)
      echo "Unsupported NVPN_WINDOWS_REQUIRE_WG_DIRECT_E2E=$REQUIRE_PROVIDER_E2E" >&2
      exit 2
      ;;
  esac
}

cleanup_remote_provider_config() {
  if [[ -z "$REMOTE_PROVIDER_CONFIG" ]]; then
    return
  fi
  run_ps "\$Bin = $(ps_quote "${GUEST_BINARY:-}")
\$Config = $(ps_quote "$GUEST_CONFIG")
if ((Test-Path -LiteralPath \$Bin -PathType Leaf) -and (Test-Path -LiteralPath \$Config -PathType Leaf)) {
  & \$Bin set --config \$Config '--exit-node=' 2>\$null | Out-Null
}
Remove-Item -Force -LiteralPath $(ps_quote "$REMOTE_PROVIDER_CONFIG") -ErrorAction SilentlyContinue" \
    >/dev/null 2>&1 || true
}

if [[ -n "$PROVIDER_CONFIG" && ! -r "$PROVIDER_CONFIG" ]]; then
  echo "NVPN_WINDOWS_WG_EXIT_CONFIG_FILE must name a readable provider WireGuard config" >&2
  exit 2
fi
if [[ -z "$PROVIDER_CONFIG" ]] && provider_e2e_required; then
  echo "NVPN_WINDOWS_WG_EXIT_CONFIG_FILE is required for the Windows Direct/WireGuard/Direct e2e" >&2
  exit 2
fi
if [[ ! "$WAIT_SECS" =~ ^[1-9][0-9]*$ || ! "$SETTLE_SECS" =~ ^[0-9]+$ ]]; then
  echo "Windows WG e2e wait/settle values must be non-negative integer seconds" >&2
  exit 2
fi

"$ROOT/scripts/windows-vm-git-sync.sh" "$SSH_HOST"

BUILD_CONFIGURATION="Debug"
BUILD_PROFILE="debug"
if [[ -n "$PROVIDER_CONFIG" ]]; then
  # The production route transition must use the optimized daemon. Debug
  # builds are not representative and can exhaust their stack on a real config.
  BUILD_CONFIGURATION="Release"
  BUILD_PROFILE="release"
fi
GUEST_BINARY="$GUEST_REPO\\target\\$BUILD_PROFILE\\nvpn.exe"

run_ps "\$ErrorActionPreference = 'Stop'
Set-Location $(ps_quote "$GUEST_REPO")
if ($(ps_quote "${NVPN_FIPS_REPO_PATH:-}") -ne '') { \$env:NVPN_FIPS_REPO_PATH = $(ps_quote "$GUEST_FIPS_REPO") }
\$Service = Get-Service -Name 'NvpnService' -ErrorAction SilentlyContinue
\$RestartService = \$Service -and \$Service.Status -ne [System.ServiceProcess.ServiceControllerStatus]::Stopped
try {
  if (\$RestartService) {
    Stop-Service -Name 'NvpnService' -Force
    (Get-Service -Name 'NvpnService').WaitForStatus(
      [System.ServiceProcess.ServiceControllerStatus]::Stopped,
      [TimeSpan]::FromSeconds(15)
    )
  }
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-build.ps1 -Configuration $BUILD_CONFIGURATION -DaemonOnly
  if (\$LASTEXITCODE -ne 0) { throw 'Windows WG e2e daemon build failed' }
}
finally {
  if (\$RestartService) {
    Start-Service -Name 'NvpnService'
    (Get-Service -Name 'NvpnService').WaitForStatus(
      [System.ServiceProcess.ServiceControllerStatus]::Running,
      [TimeSpan]::FromSeconds(15)
    )
  }
}
\$Bin = $(ps_quote "$GUEST_BINARY")
if (!(Test-Path \$Bin)) { throw \"Missing nvpn.exe: \$Bin\" }
\$IsAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (!\$IsAdmin) { throw 'Windows WG exit e2e requires an elevated/Admin SSH session for Wintun and route changes' }
& \$Bin wg-upstream-test --self-test --timeout-secs 15 --scoped-host 10.99.99.1 --ping-count 3
if (\$LASTEXITCODE -ne 0) { exit \$LASTEXITCODE }
Write-Host 'WINDOWS_WIREGUARD_SCOPED_E2E_OK'"

if [[ -z "$PROVIDER_CONFIG" ]]; then
  echo "WINDOWS_WG_DIRECT_E2E_SKIPPED: set NVPN_WINDOWS_WG_EXIT_CONFIG_FILE for the real Internet route transition"
  echo "WINDOWS_WIREGUARD_EXIT_E2E_OK"
  exit 0
fi

REMOTE_PROVIDER_CONFIG="C:\\Windows\\Temp\\nvpn-provider-wg-e2e-$$-$RANDOM.conf"
trap cleanup_remote_provider_config EXIT
copy_to_guest "$PROVIDER_CONFIG" "$REMOTE_PROVIDER_CONFIG"

run_ps "\$ErrorActionPreference = 'Stop'
\$ProviderConfig = $(ps_quote "$REMOTE_PROVIDER_CONFIG")
\$Acl = New-Object System.Security.AccessControl.FileSecurity
\$Acl.SetAccessRuleProtection(\$true, \$false)
foreach (\$SidValue in @('S-1-5-18', 'S-1-5-32-544')) {
  \$Sid = [System.Security.Principal.SecurityIdentifier]::new(\$SidValue)
  \$Rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    \$Sid,
    [System.Security.AccessControl.FileSystemRights]::FullControl,
    [System.Security.AccessControl.AccessControlType]::Allow
  )
  \$Acl.AddAccessRule(\$Rule)
}
Set-Acl -LiteralPath \$ProviderConfig -AclObject \$Acl
Set-Location $(ps_quote "$GUEST_REPO")
\$Bin = $(ps_quote "$GUEST_BINARY")
\$Config = $(ps_quote "$GUEST_CONFIG")
if (!(Test-Path -LiteralPath \$Config -PathType Leaf)) { throw \"nvpn config not found: \$Config\" }

try {
  # Establish Direct before replacing the service binary. The e2e script also
  # has its own finally block that returns the disposable VM to Direct.
  & \$Bin set --config \$Config '--exit-node='
  if (\$LASTEXITCODE -ne 0) { throw 'failed to establish Direct before the Windows WG e2e' }
  & \$Bin service install --force --config \$Config
  if (\$LASTEXITCODE -ne 0) { throw 'failed to install the Windows WG e2e daemon' }

  powershell.exe -NoProfile -ExecutionPolicy Bypass \
    -File .\\scripts\\e2e-windows-wireguard-direct.ps1 \
    -Binary \$Bin \
    -Config \$Config \
    -WireGuardConfig \$ProviderConfig \
    -ProbeUrl $(ps_quote "$PROBE_URL") \
    -WaitSeconds $WAIT_SECS \
    -SettleSeconds $SETTLE_SECS
  if (\$LASTEXITCODE -ne 0) { throw 'Windows Direct/WireGuard/Direct e2e failed' }
}
finally {
  & \$Bin set --config \$Config '--exit-node=' 2>\$null | Out-Null
}"

cleanup_remote_provider_config
trap - EXIT
REMOTE_PROVIDER_CONFIG=""
echo "WINDOWS_WIREGUARD_EXIT_E2E_OK"
