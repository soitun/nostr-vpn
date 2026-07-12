param(
  [string]$AppExe,
  [string]$ArtifactRoot
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (!$ArtifactRoot) { $ArtifactRoot = Join-Path $Root "artifacts\desktop-roster-e2e-windows" }
if (!$AppExe) { $AppExe = Join-Path $Root "windows\NostrVpn.Windows\bin\Debug\net8.0-windows\NostrVpn.Windows.exe" }
$DataDir = Join-Path $ArtifactRoot "app-data"
$Result = Join-Path $ArtifactRoot "result.json"
$FakeNvpn = Join-Path $ArtifactRoot "nvpn-e2e.cmd"

New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $DataDir
Remove-Item -Force -ErrorAction SilentlyContinue $Result

& cargo build -q -p nostr-vpn-app-core --example desktop_roster_e2e_fixture
if ($LASTEXITCODE -ne 0) { throw "desktop roster fixture build failed" }
$CargoTarget = (& cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).target_directory
$Fixture = Join-Path $CargoTarget "debug\examples\desktop_roster_e2e_fixture.exe"
& $Fixture prepare --data-dir $DataDir --result $Result
if ($LASTEXITCODE -ne 0) { throw "desktop roster fixture preparation failed" }
$DebugUrl = (Get-Content -Raw $Result | ConvertFrom-Json).debugUrl

@'
@echo off
if "%1"=="--version" (
  echo nvpn 0.0.0-e2e
  exit /b 0
)
exit /b 1
'@ | Set-Content -Encoding ascii $FakeNvpn

if (!(Test-Path $AppExe)) { throw "Windows app executable not found: $AppExe" }
Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
$env:NVPN_APP_DATA_DIR = $DataDir
$env:NVPN_CLI_PATH = $FakeNvpn
$Process = Start-Process -FilePath $AppExe -ArgumentList $DebugUrl -PassThru

try {
  $Deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $Deadline) {
    Start-Sleep -Milliseconds 250
    & $Fixture verify --data-dir $DataDir --result $Result 2>$null
    if ($LASTEXITCODE -eq 0) {
      Write-Host "DESKTOP_ROSTER_JOIN_E2E_OK"
      Write-Host "Result: $Result"
      exit 0
    }
    $Process.Refresh()
    if ($Process.HasExited) { throw "Windows app exited before accepting the join request" }
  }
  throw "Windows GUI did not persist the accepted device within 30 seconds"
} finally {
  if (!$Process.HasExited) { Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue }
}
