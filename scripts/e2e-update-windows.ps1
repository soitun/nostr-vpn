param(
  [ValidateSet("Debug", "Release")]
  [string]$Configuration = "Debug",
  [string]$ArtifactRoot = "C:\Mac\Home\src\nostr-vpn\artifacts",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$FixtureDir = Join-Path $ArtifactRoot "update-fixtures\windows"
$DownloadDir = Join-Path $ArtifactRoot "update-downloads\windows"
$ResultPath = Join-Path $ArtifactRoot "windows-update-e2e.json"
$Tag = if ($env:NVPN_UPDATE_E2E_TAG) { $env:NVPN_UPDATE_E2E_TAG } else { "v99.0.0" }
$Arch = $env:PROCESSOR_ARCHITECTURE
$AssetName = if ($Arch -match "ARM64") {
  "nostr-vpn-$Tag-windows-arm64-setup.exe"
} else {
  "nostr-vpn-$Tag-windows-x64-setup.exe"
}
$ManifestPath = Join-Path $FixtureDir "release.json"
$AssetPath = Join-Path $FixtureDir $AssetName
$AppExe = Join-Path $Root "windows\NostrVpn.Windows\bin\$Configuration\net8.0-windows\NostrVpn.Windows.exe"

New-Item -ItemType Directory -Force -Path $FixtureDir, $DownloadDir, $ArtifactRoot | Out-Null
if (!$env:CARGO_TARGET_DIR) {
  $env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA "nostr-vpn-cargo-target"
}
Set-Content -Path $AssetPath -Value "nostr vpn windows update fixture" -Encoding ascii
@{
  tag = $Tag
  assets = @(@{ name = $AssetName; path = $AssetName })
} | ConvertTo-Json -Depth 5 | Set-Content -Path $ManifestPath -Encoding utf8

if (!$SkipBuild) {
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Root "scripts\windows-build.ps1") -Configuration $Configuration
  if ($LASTEXITCODE -ne 0) {
    throw "windows-build.ps1 failed with exit code $LASTEXITCODE"
  }
}
if (!(Test-Path $AppExe)) {
  throw "Built Windows app not found: $AppExe"
}

Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
Remove-Item -Force $ResultPath -ErrorAction SilentlyContinue

$env:NVPN_UPDATE_MANIFEST_URL = ([Uri](Resolve-Path $ManifestPath).Path).AbsoluteUri
$env:NVPN_UPDATE_E2E_RESULT_PATH = $ResultPath
$env:NVPN_UPDATE_DOWNLOAD_DIR = $DownloadDir
$env:NVPN_UPDATE_SKIP_OPEN = "1"
$env:NVPN_UPDATE_E2E_CURRENT_VERSION = "0.0.0"

$process = Start-Process -FilePath $AppExe `
  -ArgumentList @("--nvpn-e2e-update-check", "--nvpn-e2e-install-update") `
  -PassThru `
  -Wait
if ($process.ExitCode -ne 0) {
  throw "Windows update e2e app exited with code $($process.ExitCode)"
}

$result = Get-Content $ResultPath -Raw | ConvertFrom-Json
if (!$result.ok) {
  throw "Windows update e2e failed: $($result.error)"
}
if (!$result.available) {
  throw "Windows update was not detected as available"
}
if (!$result.assetName -or $result.assetName -notmatch "windows-(x64|arm64)-setup\.exe$") {
  throw "Unexpected Windows asset: $($result.assetName)"
}
if (!$result.downloadedPath -or !(Test-Path $result.downloadedPath)) {
  throw "Windows update download missing: $($result.downloadedPath)"
}
if (!$result.downloadedBytes -or $result.downloadedBytes -le 0) {
  throw "Windows update download was empty"
}

Write-Host "WINDOWS_UPDATE_E2E_OK"
Write-Host "Result: $ResultPath"
