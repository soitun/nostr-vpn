param(
  [Parameter(Mandatory = $true)]
  [string]$Bin,
  [string]$ArtifactRoot,
  [double]$MaxPercent = -1,
  [double]$SampleSeconds = -1,
  [double]$SettleSeconds = -1
)

$ErrorActionPreference = 'Stop'
$Root = Resolve-Path (Join-Path $PSScriptRoot '..')
if (!$ArtifactRoot) { $ArtifactRoot = Join-Path $Root 'artifacts' }
if ($MaxPercent -lt 0) { $MaxPercent = if ($env:NVPN_WINDOWS_DAEMON_IDLE_CPU_MAX_PERCENT) { [double]$env:NVPN_WINDOWS_DAEMON_IDLE_CPU_MAX_PERCENT } else { 2 } }
if ($SampleSeconds -lt 0) { $SampleSeconds = if ($env:NVPN_WINDOWS_DAEMON_IDLE_CPU_SAMPLE_SECONDS) { [double]$env:NVPN_WINDOWS_DAEMON_IDLE_CPU_SAMPLE_SECONDS } else { 60 } }
if ($SettleSeconds -lt 0) { $SettleSeconds = if ($env:NVPN_WINDOWS_DAEMON_IDLE_CPU_SETTLE_SECONDS) { [double]$env:NVPN_WINDOWS_DAEMON_IDLE_CPU_SETTLE_SECONDS } else { 15 } }

$Bin = [IO.Path]::GetFullPath($Bin)
$ArtifactRoot = [IO.Path]::GetFullPath($ArtifactRoot)
$ResultPath = Join-Path $ArtifactRoot 'windows-daemon-idle-cpu.json'
$Fixture = Join-Path $env:TEMP "nvpn-idle-cpu-$PID"
$Config = Join-Path $Fixture 'node-a.toml'
$PeerConfig = Join-Path $Fixture 'node-b.toml'
$daemon = $null

function Get-Npub([string]$Path) {
  $match = Select-String -Path $Path -Pattern '^public_key\s*=\s*"([^"]+)"' | Select-Object -First 1
  if (!$match) { throw "No Nostr public key in $Path" }
  return $match.Matches[0].Groups[1].Value
}

function Find-FixtureDaemon {
  $escapedBin = [regex]::Escape($Bin)
  $escapedConfig = [regex]::Escape($Config)
  $matches = @(Get-CimInstance Win32_Process | Where-Object {
    $_.Name -eq 'nvpn.exe' -and $_.ExecutablePath -eq $Bin -and
    $_.CommandLine -match "^`"?$escapedBin`"?\s+daemon\b" -and
    $_.CommandLine -match $escapedConfig
  })
  if ($matches.Count -ne 1) { return $null }
  return Get-Process -Id $matches[0].ProcessId
}

try {
  if (!(Test-Path $Bin)) { throw "nvpn.exe not found: $Bin" }
  New-Item -ItemType Directory -Force -Path $ArtifactRoot, $Fixture | Out-Null
  & $Bin init --config $Config --force | Out-Null
  & $Bin init --config $PeerConfig --force | Out-Null
  $ownNpub = Get-Npub $Config
  $peerNpub = Get-Npub $PeerConfig
  & $Bin set --config $Config --participant $ownNpub --participant $peerNpub `
    --network-id windows-idle-cpu --endpoint 127.0.0.1:51891 --listen-port 51891 `
    --fips-peer-endpoint "$peerNpub=127.0.0.1:51892" --fips-advertise-endpoint true `
    --fips-nostr-discovery-enabled false --fips-bootstrap-enabled false | Out-Null
  & $Bin start --config $Config --daemon --connect | Out-Null

  $deadline = (Get-Date).AddSeconds(20)
  while ((Get-Date) -lt $deadline -and !$daemon) {
    Start-Sleep -Milliseconds 250
    $daemon = Find-FixtureDaemon
  }
  if (!$daemon) { throw 'Candidate Windows nvpn daemon did not start' }

  Start-Sleep -Milliseconds ([int]($SettleSeconds * 1000))
  $daemon.Refresh()
  $startCpu = $daemon.TotalProcessorTime.TotalSeconds
  $watch = [Diagnostics.Stopwatch]::StartNew()
  Start-Sleep -Milliseconds ([int]($SampleSeconds * 1000))
  $watch.Stop()
  $daemon.Refresh()
  if ($daemon.HasExited) { throw 'Candidate Windows nvpn daemon exited during idle sample' }
  $cpuPercent = [Math]::Max(0, $daemon.TotalProcessorTime.TotalSeconds - $startCpu) * 100 / [Math]::Max(0.001, $watch.Elapsed.TotalSeconds)
  $ok = $cpuPercent -le $MaxPercent
  [pscustomobject]@{
    ok = $ok
    mode = 'windows-active-daemon'
    label = 'Windows nvpn active daemon'
    pids = @($daemon.Id)
    cpuPercent = $cpuPercent
    maxPercent = $MaxPercent
    sampleSeconds = $SampleSeconds
    settleSeconds = $SettleSeconds
    elapsedSeconds = $watch.Elapsed.TotalSeconds
    generatedAt = (Get-Date).ToUniversalTime().ToString('o')
  } | ConvertTo-Json -Depth 4 | Out-File -Encoding utf8 $ResultPath
  if (!$ok) { throw ("Windows daemon idle CPU {0:N3}% > {1:N3}%" -f $cpuPercent, $MaxPercent) }
  Write-Host ("Windows daemon idle CPU ok: {0:N3}% <= {1:N3}%" -f $cpuPercent, $MaxPercent)
  Write-Host "Result: $ResultPath"
} finally {
  if ((Test-Path $Bin) -and (Test-Path $Config)) {
    & $Bin down --config $Config 2>$null | Out-Null
  }
  if ($daemon -and !$daemon.HasExited) { Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue }
  Remove-Item -Recurse -Force $Fixture -ErrorAction SilentlyContinue
}
