param(
  [Parameter(Mandatory = $true)]
  [string]$Binary,
  [Parameter(Mandatory = $true)]
  [string]$Config,
  [Parameter(Mandatory = $true)]
  [string]$WireGuardConfig,
  [string]$ProbeUrl = "https://example.com/",
  [int]$WaitSeconds = 60,
  [int]$SettleSeconds = 3
)

# Real Windows daemon transition check: Direct -> provider WireGuard -> Direct.
# Run only on a disposable elevated VM. The provider profile remains external
# to the repository and this script never prints its contents.
$ErrorActionPreference = "Stop"

function Invoke-Nvpn {
  param([string[]]$Arguments)
  & $Binary @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "nvpn $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
  }
}

function Get-BestInternetRoute {
  $route = Find-NetRoute -RemoteIPAddress "1.1.1.1" -ErrorAction Stop |
    Where-Object { $_.DestinationPrefix -eq "0.0.0.0/0" } |
    Select-Object -First 1
  if (!$route) {
    throw "Windows has no best IPv4 Internet route"
  }
  $route
}

function Test-ExternalHttps {
  # Windows' built-in curl uses Schannel. Its strict certificate-revocation
  # fetch can stall behind a privacy VPN even when TCP, DNS, and verified TLS
  # are healthy. Best-effort still verifies the certificate and fails real
  # routing/TLS errors without turning an unreachable CRL into a false outage.
  & curl.exe -4 --ssl-revoke-best-effort --fail --silent `
    --max-time 12 --output NUL $ProbeUrl
  $LASTEXITCODE -eq 0
}

function Wait-ForCondition {
  param(
    [string]$Description,
    [scriptblock]$Condition
  )
  $timer = [Diagnostics.Stopwatch]::StartNew()
  while ($timer.Elapsed.TotalSeconds -lt $WaitSeconds) {
    if (& $Condition) {
      return
    }
    Start-Sleep -Milliseconds 250
  }
  throw "timed out after ${WaitSeconds}s waiting for $Description"
}

$isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
  [Security.Principal.WindowsBuiltInRole]::Administrator
)
if (!$isAdmin) {
  throw "Windows WireGuard/Direct e2e requires an elevated Administrator session"
}
foreach ($path in @($Binary, $Config, $WireGuardConfig)) {
  if (!(Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "required file does not exist: $path"
  }
}

$directRoute = Get-BestInternetRoute
$directInterfaceIndex = [uint32]$directRoute.InterfaceIndex
$directInterfaceAlias = [string]$directRoute.InterfaceAlias
$directNextHop = [string]$directRoute.NextHop
if (!(Test-ExternalHttps)) {
  throw "external HTTPS does not work before the test on Direct"
}

$wireGuardInterfaceIndex = $null
try {
  $wireGuardTimer = [Diagnostics.Stopwatch]::StartNew()
  Invoke-Nvpn @(
    "set", "--config", $Config,
    "--wireguard-exit-config-file", $WireGuardConfig,
    "--wireguard-exit-enabled", "true"
  )
  Wait-ForCondition "WireGuard to own the best route with external HTTPS working" {
    $route = Get-BestInternetRoute
    $route.InterfaceIndex -ne $directInterfaceIndex -and (Test-ExternalHttps)
  }
  $wireGuardRoute = Get-BestInternetRoute
  $wireGuardInterfaceIndex = [uint32]$wireGuardRoute.InterfaceIndex

  # Catch delayed route reconciliation that used to invalidate a live tunnel.
  Start-Sleep -Seconds $SettleSeconds
  $settledRoute = Get-BestInternetRoute
  if ($settledRoute.InterfaceIndex -ne $wireGuardInterfaceIndex -or !(Test-ExternalHttps)) {
    throw "WireGuard route or external HTTPS failed after the settle interval"
  }
  $wireGuardElapsed = [Math]::Round($wireGuardTimer.Elapsed.TotalSeconds, 2)

  $directTimer = [Diagnostics.Stopwatch]::StartNew()
  # Windows PowerShell 5 drops empty native arguments, so use clap's
  # --option=value form to express Direct reliably.
  Invoke-Nvpn @("set", "--config", $Config, "--exit-node=")
  Wait-ForCondition "the original Direct route and external HTTPS" {
    $route = Get-BestInternetRoute
    $route.InterfaceIndex -eq $directInterfaceIndex -and
      $route.NextHop -eq $directNextHop -and
      (Test-ExternalHttps)
  }
  if ($wireGuardInterfaceIndex) {
    $staleDefault = Get-NetRoute -AddressFamily IPv4 -DestinationPrefix "0.0.0.0/0" `
      -ErrorAction SilentlyContinue |
      Where-Object { $_.InterfaceIndex -eq $wireGuardInterfaceIndex }
    if ($staleDefault) {
      throw "WireGuard default route remains after switching back to Direct"
    }
  }
  $directElapsed = [Math]::Round($directTimer.Elapsed.TotalSeconds, 2)

  Write-Output "WINDOWS_WG_DIRECT_E2E_OK"
  Write-Output "WireGuard route and external HTTPS stable after ${wireGuardElapsed}s"
  Write-Output "Direct external HTTPS restored on $directInterfaceAlias after ${directElapsed}s"
}
finally {
  # Best-effort fail-safe. This changes only the disposable test VM.
  & $Binary set --config $Config --exit-node= 2>$null | Out-Null
}
