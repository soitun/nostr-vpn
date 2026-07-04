#!/usr/bin/env bash
# Measure an already paired Windows/Linux nvpn host pair with enough evidence to
# make each row auditable: Windows service image hash, running PID image hash,
# underlay symmetry, process CPU deltas, and Windows NIC/Wintun counter deltas.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

WINDOWS_SSH="${NVPN_WINLIN_WINDOWS_SSH:-${1:-}}"
WINDOWS_SSH_JUMP="${NVPN_WINLIN_WINDOWS_SSH_JUMP:-}"
WINDOWS_SSH_PROXY_COMMAND="${NVPN_WINLIN_WINDOWS_SSH_PROXY_COMMAND:-}"
LINUX_SSH="${NVPN_WINLIN_LINUX_SSH:-${2:-}}"
LINUX_SSH_JUMP="${NVPN_WINLIN_LINUX_SSH_JUMP:-}"
LINUX_SSH_PROXY_COMMAND="${NVPN_WINLIN_LINUX_SSH_PROXY_COMMAND:-}"
WINDOWS_REPO="${NVPN_WINLIN_WINDOWS_REPO:-}"
WINDOWS_FIPS_REPO="${NVPN_WINLIN_WINDOWS_FIPS_REPO:-}"
CURRENT_FIPS_REPO="${NVPN_WINLIN_CURRENT_FIPS_REPO:-}"
LINUX_FIPS_REPO="${NVPN_WINLIN_LINUX_FIPS_REPO:-}"
WINDOWS_INSTALLED_NVPN="${NVPN_WINLIN_INSTALLED_NVPN:-}"
WINDOWS_CONFIG="${NVPN_WINLIN_WINDOWS_CONFIG:-}"
LINUX_INSTALLED_NVPN="${NVPN_WINLIN_INSTALLED_LINUX_NVPN:-}"
LINUX_INSTALLED_EXPECTED_HASH="${NVPN_WINLIN_INSTALLED_LINUX_SHA256:-}"
LINUX_CONFIG="${NVPN_WINLIN_LINUX_CONFIG:-}"
LINUX_NVPN="${NVPN_WINLIN_LINUX_NVPN:-nvpn}"
CURRENT_LINUX_NVPN="${NVPN_WINLIN_CURRENT_LINUX_NVPN:-}"
ALLOW_CURRENT_LINUX_AS_INSTALLED="${NVPN_WINLIN_ALLOW_CURRENT_LINUX_AS_INSTALLED:-0}"
ROWS="${NVPN_WINLIN_ROWS:-installed,current}"
OUTPUT_DIR="${NVPN_WINLIN_OUTPUT_DIR:-$ROOT_DIR/artifacts/windows-linux-nvpn/$(date -u +%Y%m%dT%H%M%SZ)}"
DURATION_SECS="${NVPN_WINLIN_DURATION_SECS:-10}"
PROBE_PORT_BASE="${NVPN_WINLIN_PROBE_PORT_BASE:-52310}"
MIN_UNDERLAY_MBPS="${NVPN_WINLIN_MIN_UNDERLAY_MBPS:-250}"
MAX_UNDERLAY_RATIO="${NVPN_WINLIN_MAX_UNDERLAY_RATIO:-2.5}"
ALLOW_NON_DIRECT="${NVPN_WINLIN_ALLOW_NON_DIRECT:-0}"
SKIP_WINDOWS_SYNC="${NVPN_WINLIN_SKIP_WINDOWS_SYNC:-0}"
SKIP_CURRENT_BUILD="${NVPN_WINLIN_SKIP_CURRENT_BUILD:-0}"
CURRENT_WINDOWS_NVPN="${NVPN_WINLIN_CURRENT_WINDOWS_NVPN:-}"
RESTORE_INSTALLED="${NVPN_WINLIN_RESTORE_INSTALLED:-0}"
RESTORE_LINUX_INSTALLED="${NVPN_WINLIN_RESTORE_LINUX_INSTALLED:-$RESTORE_INSTALLED}"
WINDOWS_FIREWALL_RULE="${NVPN_WINLIN_WINDOWS_FIREWALL_RULE:-1}"
KEEP_FIREWALL_RULE="${NVPN_WINLIN_KEEP_FIREWALL_RULE:-0}"
WINDOWS_NVPN_FIREWALL_RULE="${NVPN_WINLIN_WINDOWS_NVPN_FIREWALL_RULE:-1}"
KEEP_NVPN_FIREWALL_RULE="${NVPN_WINLIN_KEEP_NVPN_FIREWALL_RULE:-0}"
DIRECT_FIPS_CAPTURE="${NVPN_WINLIN_DIRECT_FIPS_CAPTURE:-1}"
LINUX_CAPTURE_IFACE="${NVPN_WINLIN_LINUX_CAPTURE_IFACE:-}"
TUNNEL_HEALTH_ATTEMPTS="${NVPN_WINLIN_TUNNEL_HEALTH_ATTEMPTS:-20}"
TUNNEL_HEALTH_INTERVAL_SECS="${NVPN_WINLIN_TUNNEL_HEALTH_INTERVAL_SECS:-3}"
TUNNEL_PING_COUNT="${NVPN_WINLIN_TUNNEL_PING_COUNT:-5}"
TUNNEL_MAX_LOSS_PERCENT="${NVPN_WINLIN_TUNNEL_MAX_LOSS_PERCENT:-0}"
WINDOWS_PIPELINE_TRACE="${NVPN_WINLIN_WINDOWS_PIPELINE_TRACE:-0}"
WINDOWS_PIPELINE_INTERVAL_SECS="${NVPN_WINLIN_WINDOWS_PIPELINE_INTERVAL_SECS:-1}"
WINDOWS_DAEMON_LOG_TAIL_LINES="${NVPN_WINLIN_WINDOWS_DAEMON_LOG_TAIL_LINES:-4000}"
MESH_MTU_PROFILE="${NVPN_WINLIN_MESH_MTU_PROFILE:-}"
MESH_UNDERLAY_UDP_MTU="${NVPN_WINLIN_MESH_UNDERLAY_UDP_MTU:-}"
MESH_TUNNEL_MTU="${NVPN_WINLIN_MESH_TUNNEL_MTU:-}"

if [[ -z "$CURRENT_FIPS_REPO" && -d "$ROOT_DIR/../fips/.git" ]]; then
  CURRENT_FIPS_REPO="$(cd "$ROOT_DIR/../fips" && pwd)"
fi

SUMMARY_TSV="$OUTPUT_DIR/summary.tsv"
RUN_JSON="$OUTPUT_DIR/run.json"
NEXT_PORT="$PROBE_PORT_BASE"
WINDOWS_FIREWALL_RULE_NAME="nvpn-winlin-perf-$RANDOM-$$"
WINDOWS_NVPN_FIREWALL_RULE_PREFIX="nvpn-winlin-nvpn-$RANDOM-$$"
WINDOWS_CONFIG_BACKUP=""
WINDOWS_CONFIG_BACKUP_HASH=""
LINUX_INSTALLED_BACKUP=""
LINUX_INSTALLED_BACKUP_HASH=""
WINDOWS_SERVICE_ENV_APPLIED=0
LINUX_SERVICE_ENV_APPLIED=0

die() {
  printf 'windows-linux nvpn bench failed: %s\n' "$*" >&2
  exit 1
}

is_true() {
  [[ "${1:-}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

sh_q() {
  printf '%q' "$1"
}

ps_sq() {
  local value="${1//\'/\'\'}"
  printf "'%s'" "$value"
}

run_windows_ps() {
  local script="$1"
  local encoded
  local -a ssh_cmd
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64 | tr -d '\n')"
  ssh_cmd=(ssh -o BatchMode=yes -o ConnectTimeout=20)
  if [[ -n "$WINDOWS_SSH_PROXY_COMMAND" ]]; then
    ssh_cmd+=(-o "ProxyCommand=$WINDOWS_SSH_PROXY_COMMAND")
  elif [[ -n "$WINDOWS_SSH_JUMP" ]]; then
    ssh_cmd+=(-J "$WINDOWS_SSH_JUMP")
  fi
  ssh_cmd+=("$WINDOWS_SSH")
  "${ssh_cmd[@]}" \
    powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand "$encoded"
}

run_linux_sh() {
  local cmd="$1"
  local -a ssh_cmd
  ssh_cmd=(ssh -o BatchMode=yes -o ConnectTimeout=20)
  if [[ -n "$LINUX_SSH_PROXY_COMMAND" ]]; then
    ssh_cmd+=(-o "ProxyCommand=$LINUX_SSH_PROXY_COMMAND")
  elif [[ -n "$LINUX_SSH_JUMP" ]]; then
    ssh_cmd+=(-J "$LINUX_SSH_JUMP")
  fi
  ssh_cmd+=("$LINUX_SSH")
  "${ssh_cmd[@]}" "bash -lc $(sh_q "$cmd")"
}

usage() {
  cat >&2 <<'EOF'
usage: NVPN_WINLIN_WINDOWS_SSH=<windows-ssh> \
       NVPN_WINLIN_LINUX_SSH=<linux-ssh> \
       NVPN_WINLIN_WINDOWS_REPO=<windows-repo-path> \
       scripts/bench-windows-linux-nvpn.sh

Rows:
  NVPN_WINLIN_ROWS=installed,current      default
  NVPN_WINLIN_ROWS=installed              measure the installed Windows service image only
  NVPN_WINLIN_ROWS=current                build/switch/measure the current Windows checkout only

Important options:
  NVPN_WINLIN_INSTALLED_NVPN              installed nvpn.exe path; defaults to current service binary
  NVPN_WINLIN_INSTALLED_LINUX_SHA256      optional expected installed Linux nvpn hash; checked before switching
  NVPN_WINLIN_CURRENT_WINDOWS_NVPN        prebuilt current nvpn.exe path; skips path discovery
  NVPN_WINLIN_CURRENT_LINUX_NVPN          prebuilt current Linux nvpn; switches Linux for current row
  NVPN_WINLIN_CURRENT_FIPS_REPO           local FIPS checkout used to build current binaries; defaults to ../fips
  NVPN_WINLIN_WINDOWS_FIPS_REPO           Windows FIPS checkout path for current Windows build
  NVPN_WINLIN_LINUX_FIPS_REPO             Linux FIPS checkout path recorded for current Linux binary provenance
  NVPN_WINLIN_ALLOW_CURRENT_LINUX_AS_INSTALLED=1
                                           allow current Linux hash to equal the installed Linux baseline
  NVPN_WINLIN_WINDOWS_SSH_JUMP            optional ProxyJump for Windows SSH
  NVPN_WINLIN_LINUX_SSH_JUMP              optional ProxyJump for Linux SSH
  NVPN_WINLIN_WINDOWS_SSH_PROXY_COMMAND   optional ProxyCommand for Windows SSH; takes
                                           precedence over Windows ProxyJump
  NVPN_WINLIN_LINUX_SSH_PROXY_COMMAND     optional ProxyCommand for Linux SSH; takes
                                           precedence over Linux ProxyJump
  NVPN_WINLIN_SKIP_CURRENT_BUILD=1        do not run scripts/windows-build.ps1
  NVPN_WINLIN_SKIP_WINDOWS_SYNC=1         do not run scripts/windows-vm-git-sync.sh
  NVPN_WINLIN_DURATION_SECS=10            probe duration per direction
  NVPN_WINLIN_TUNNEL_PING_COUNT=5         ping count before each nvpn row
  NVPN_WINLIN_TUNNEL_MAX_LOSS_PERCENT=0   reject nvpn rows above this tunnel loss
  NVPN_WINLIN_WINDOWS_NVPN_FIREWALL_RULE=1
                                           add temporary inbound UDP allow rule for measured nvpn.exe
  NVPN_WINLIN_ALLOW_NON_DIRECT=1          allow non-direct FIPS peer path
  NVPN_WINLIN_RESTORE_INSTALLED=1         restore installed Windows service image at exit
  NVPN_WINLIN_RESTORE_LINUX_INSTALLED=1   restore Linux /usr/local/bin/nvpn from backup at exit
  NVPN_WINLIN_DIRECT_FIPS_CAPTURE=1       capture Linux UDP/51820 during nvpn rows
  NVPN_WINLIN_LINUX_CAPTURE_IFACE=enp1s0  optional capture interface override
  NVPN_WINLIN_WINDOWS_PIPELINE_TRACE=1    enable nvpn pipeline trace on the measured Windows service
  NVPN_WINLIN_WINDOWS_PIPELINE_INTERVAL_SECS=1
                                           interval for Windows pipeline trace service env
  NVPN_WINLIN_WINDOWS_DAEMON_LOG_TAIL_LINES=4000
                                           max daemon log delta lines to save after nvpn directions
  NVPN_WINLIN_MESH_MTU_PROFILE=lan         set measured services' NVPN_MESH_MTU_PROFILE
  NVPN_WINLIN_MESH_UNDERLAY_UDP_MTU=1420   set measured services' NVPN_MESH_UNDERLAY_UDP_MTU
  NVPN_WINLIN_MESH_TUNNEL_MTU=1290         set measured services' NVPN_MESH_TUNNEL_MTU

The Linux side is intentionally held constant by default. This isolates the
Windows service/dataplane change while still recording the Linux service binary
and counters in every artifact.
EOF
}

require_inputs() {
  [[ -n "$WINDOWS_SSH" ]] || die "set NVPN_WINLIN_WINDOWS_SSH or pass Windows SSH host as arg1"
  [[ -n "$LINUX_SSH" ]] || die "set NVPN_WINLIN_LINUX_SSH or pass Linux SSH host as arg2"
  case ",$ROWS," in
    *,current,*)
      if [[ -z "$CURRENT_WINDOWS_NVPN" && -z "$WINDOWS_REPO" ]]; then
        die "current row needs NVPN_WINLIN_WINDOWS_REPO or NVPN_WINLIN_CURRENT_WINDOWS_NVPN"
      fi
      ;;
  esac
}

probe_source() {
  cat <<'RS'
use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_BUF: usize = 64 * 1024;

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn json(label: &str, direction: &str, bytes: u64, secs: f64, role: &str) -> String {
    let mbps = if secs > 0.0 {
        bytes as f64 * 8.0 / secs / 1_000_000.0
    } else {
        0.0
    };
    format!(
        "{{\"label\":\"{}\",\"direction\":\"{}\",\"role\":\"{}\",\"bytes\":{},\"seconds\":{:.6},\"mbps\":{:.3},\"unix_ms\":{}}}",
        json_escape(label),
        json_escape(direction),
        json_escape(role),
        bytes,
        secs,
        mbps,
        now_unix_ms()
    )
}

fn read_line(reader: &mut BufReader<TcpStream>) -> io::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
}

fn run_server(bind: &str) -> io::Result<()> {
    let listener = TcpListener::bind(bind)?;
    let (stream, peer) = listener.accept()?;
    stream.set_nodelay(true)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let line = read_line(&mut reader)?;
    let mut parts = line.split_whitespace();
    let command = parts.next().unwrap_or_default();
    let label = parts.next().unwrap_or("probe");
    let direction = parts.next().unwrap_or("unknown");
    let duration_secs = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(8);
    let buf_size = parts
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_BUF)
        .clamp(1024, 1024 * 1024);

    match command {
        "SEND" => {
            let mut buf = vec![0u8; buf_size];
            let started = Instant::now();
            let mut bytes = 0u64;
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                bytes += read as u64;
            }
            let elapsed = started.elapsed().as_secs_f64();
            let summary = json(label, direction, bytes, elapsed, "server");
            println!("{summary}");
            let mut writer = stream;
            writeln!(writer, "{summary}")?;
        }
        "RECV" => {
            let mut writer = stream;
            let buf = vec![0u8; buf_size];
            let duration = Duration::from_secs(duration_secs);
            let started = Instant::now();
            let mut bytes = 0u64;
            while started.elapsed() < duration {
                writer.write_all(&buf)?;
                bytes += buf.len() as u64;
            }
            writer.shutdown(Shutdown::Write)?;
            let elapsed = started.elapsed().as_secs_f64();
            let summary = json(label, direction, bytes, elapsed, "server");
            println!("{summary}");
        }
        _ => {
            eprintln!("unknown command from {peer}: {line:?}");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn run_client(
    connect: &str,
    mode: &str,
    duration_secs: u64,
    label: &str,
    direction: &str,
) -> io::Result<()> {
    let mut stream = connect_with_timeout(connect, Duration::from_secs(5))?;
    stream.set_nodelay(true)?;
    let io_timeout = Duration::from_secs(duration_secs.saturating_add(10));
    stream.set_read_timeout(Some(io_timeout))?;
    stream.set_write_timeout(Some(io_timeout))?;
    let buf_size = DEFAULT_BUF;
    match mode {
        "send" => {
            writeln!(stream, "SEND {label} {direction} {duration_secs} {buf_size}")?;
            let buf = vec![0u8; buf_size];
            let duration = Duration::from_secs(duration_secs);
            let started = Instant::now();
            let mut bytes = 0u64;
            while started.elapsed() < duration {
                stream.write_all(&buf)?;
                bytes += buf.len() as u64;
            }
            stream.shutdown(Shutdown::Write)?;
            let elapsed = started.elapsed().as_secs_f64();
            println!("{}", json(label, direction, bytes, elapsed, "client"));
            let mut response = String::new();
            let _ = BufReader::new(stream).read_line(&mut response);
            if !response.trim().is_empty() {
                println!("{}", response.trim());
            }
        }
        "recv" => {
            writeln!(stream, "RECV {label} {direction} {duration_secs} {buf_size}")?;
            let mut buf = vec![0u8; buf_size];
            let started = Instant::now();
            let mut bytes = 0u64;
            loop {
                let read = stream.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                bytes += read as u64;
            }
            let elapsed = started.elapsed().as_secs_f64();
            println!("{}", json(label, direction, bytes, elapsed, "client"));
        }
        _ => {
            eprintln!("mode must be send or recv");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn connect_with_timeout(connect: &str, timeout: Duration) -> io::Result<TcpStream> {
    let mut last_error = None;
    for addr in connect.to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("no socket addresses resolved for {connect}"),
        )
    }))
}

fn usage() -> ! {
    eprintln!(
        "usage: tcp_probe server <bind:port> | tcp_probe client <connect:port> <send|recv> <duration_secs> <label> <direction>"
    );
    std::process::exit(2);
}

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("server") => {
            let bind = args.next().unwrap_or_else(|| usage());
            run_server(&bind)
        }
        Some("client") => {
            let connect = args.next().unwrap_or_else(|| usage());
            let mode = args.next().unwrap_or_else(|| usage());
            let duration_secs = args
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(8);
            let label = args.next().unwrap_or_else(|| "probe".to_string());
            let direction = args.next().unwrap_or_else(|| "unknown".to_string());
            run_client(&connect, &mode, duration_secs, &label, &direction)
        }
        _ => usage(),
    }
}
RS
}

next_port() {
  local port="$NEXT_PORT"
  NEXT_PORT=$((NEXT_PORT + 1))
  printf '%s\n' "$port"
}

ensure_probe_binaries() {
  local source_b64
  source_b64="$(probe_source | base64 | tr -d '\n')"

  run_linux_sh "set -euo pipefail
dir=/tmp/nvpn-winlin-probe
mkdir -p \"\$dir\"
pkill -x tcp_probe >/dev/null 2>&1 || true
printf '%s' $(sh_q "$source_b64") | base64 -d >\"\$dir/tcp_probe.rs\"
rustc -O \"\$dir/tcp_probe.rs\" -o \"\$dir/tcp_probe\""

  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$dir = Join-Path \$env:TEMP 'nvpn-winlin-probe'
New-Item -ItemType Directory -Force -Path \$dir | Out-Null
Get-Process -Name tcp_probe -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
\$source = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($(ps_sq "$source_b64")))
Set-Content -Path (Join-Path \$dir 'tcp_probe.rs') -Value \$source -Encoding UTF8
rustc -O (Join-Path \$dir 'tcp_probe.rs') -o (Join-Path \$dir 'tcp_probe.exe')"

  if is_true "$WINDOWS_FIREWALL_RULE"; then
    run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
\$dir = Join-Path \$env:TEMP 'nvpn-winlin-probe'
\$probe = Join-Path \$dir 'tcp_probe.exe'
\$ports = '${PROBE_PORT_BASE}-$((PROBE_PORT_BASE + 128))'
Remove-NetFirewallRule -DisplayName $(ps_sq "$WINDOWS_FIREWALL_RULE_NAME") -ErrorAction SilentlyContinue | Out-Null
New-NetFirewallRule -DisplayName $(ps_sq "$WINDOWS_FIREWALL_RULE_NAME") -Direction Inbound -Action Allow -Protocol TCP -LocalPort \$ports -Program \$probe -Profile Any | Out-Null" \
      >/dev/null || printf 'warning: failed to install temporary Windows firewall rule for probe ports\n' >&2
  fi
}

cleanup() {
  local windows_installed_hash=""
  if is_true "$RESTORE_INSTALLED" && [[ -n "${WINDOWS_INSTALLED_NVPN:-}" ]]; then
    windows_installed_hash="$(windows_hash "$WINDOWS_INSTALLED_NVPN" 2>/dev/null | tr -d '\r\n' || true)"
    if switch_windows_service "$WINDOWS_INSTALLED_NVPN" "restore-installed" >/dev/null 2>&1; then
      if [[ -n "$windows_installed_hash" ]]; then
        wait_for_windows_hash "$windows_installed_hash" "restore-installed" >/dev/null 2>&1 || true
      fi
    fi
  fi
  if is_true "$RESTORE_INSTALLED" && [[ -n "${WINDOWS_CONFIG_BACKUP:-}" ]]; then
    restore_windows_config >/dev/null 2>&1 || true
  fi
  if is_true "$RESTORE_INSTALLED" && [[ -n "${WINDOWS_INSTALLED_NVPN:-}" ]]; then
    ensure_windows_service_running >/dev/null 2>&1 || true
    if [[ -n "$windows_installed_hash" ]]; then
      wait_for_windows_hash "$windows_installed_hash" "restore-installed-final" >/dev/null 2>&1 || true
    fi
  fi
  if is_true "$RESTORE_LINUX_INSTALLED" && [[ -n "${LINUX_INSTALLED_BACKUP:-}" ]]; then
    if switch_linux_service "$LINUX_INSTALLED_BACKUP" "restore-installed" >/dev/null 2>&1; then
      if [[ -n "${LINUX_INSTALLED_BACKUP_HASH:-}" ]]; then
        wait_for_linux_hash "$LINUX_INSTALLED_BACKUP_HASH" "restore-installed" >/dev/null 2>&1 || true
      fi
    fi
  fi
  if [[ "$WINDOWS_SERVICE_ENV_APPLIED" == "1" ]]; then
    clear_windows_service_env >/dev/null 2>&1 || true
  fi
  if [[ "$LINUX_SERVICE_ENV_APPLIED" == "1" ]]; then
    clear_linux_service_env >/dev/null 2>&1 || true
  fi
  if is_true "$WINDOWS_NVPN_FIREWALL_RULE" && ! is_true "$KEEP_NVPN_FIREWALL_RULE"; then
    run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
Get-NetFirewallRule -DisplayName $(ps_sq "${WINDOWS_NVPN_FIREWALL_RULE_PREFIX}*") -ErrorAction SilentlyContinue |
  Remove-NetFirewallRule -ErrorAction SilentlyContinue | Out-Null" \
      >/dev/null 2>&1 || true
  fi
  if is_true "$WINDOWS_FIREWALL_RULE" && ! is_true "$KEEP_FIREWALL_RULE"; then
    run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
Remove-NetFirewallRule -DisplayName $(ps_sq "$WINDOWS_FIREWALL_RULE_NAME") -ErrorAction SilentlyContinue | Out-Null" \
      >/dev/null 2>&1 || true
  fi
  if is_true "$RESTORE_INSTALLED" && [[ -n "${WINDOWS_INSTALLED_NVPN:-}" ]]; then
    ensure_windows_service_running >/dev/null 2>&1 || true
    if [[ -n "$windows_installed_hash" ]]; then
      wait_for_windows_hash "$windows_installed_hash" "restore-installed-exit" >/dev/null 2>&1 || true
    fi
  fi
}

backup_windows_config() {
  if ! is_true "$RESTORE_INSTALLED" || [[ -z "$WINDOWS_CONFIG" || -n "$WINDOWS_CONFIG_BACKUP" ]]; then
    return 0
  fi
  local backup_json="$OUTPUT_DIR/windows-config-backup.json"
  local config_ps
  config_ps="$(ps_sq "$WINDOWS_CONFIG")"
  printf 'backing up Windows config %s\n' "$WINDOWS_CONFIG" >&2
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$config = $config_ps
\$backup = Join-Path \$env:TEMP ('nvpn-winlin-config-' + [guid]::NewGuid().ToString('N') + '.toml')
if (!(Test-Path -LiteralPath \$config)) { throw \"Windows nvpn config not found: \$config\" }
Copy-Item -LiteralPath \$config -Destination \$backup -Force
\$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath \$backup).Hash.ToLowerInvariant()
[pscustomobject]@{ config = \$config; backup = \$backup; sha256 = \$hash } | ConvertTo-Json -Compress" \
    >"$backup_json"
  WINDOWS_CONFIG_BACKUP="$(jq -r '.backup // empty' "$backup_json")"
  WINDOWS_CONFIG_BACKUP_HASH="$(jq -r '.sha256 // empty' "$backup_json")"
  [[ -n "$WINDOWS_CONFIG_BACKUP" ]] || die "failed to back up Windows config"
}

restore_windows_config() {
  [[ -n "$WINDOWS_CONFIG_BACKUP" && -n "$WINDOWS_CONFIG" ]] || return 0
  local config_ps backup_ps expected_hash_ps
  config_ps="$(ps_sq "$WINDOWS_CONFIG")"
  backup_ps="$(ps_sq "$WINDOWS_CONFIG_BACKUP")"
  expected_hash_ps="$(ps_sq "$WINDOWS_CONFIG_BACKUP_HASH")"
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$config = $config_ps
\$backup = $backup_ps
\$expectedHash = $expected_hash_ps
if (!(Test-Path -LiteralPath \$backup)) { throw \"Windows nvpn config backup not found: \$backup\" }
if (\$expectedHash) {
  \$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath \$backup).Hash.ToLowerInvariant()
  if (\$actualHash -ne \$expectedHash) {
    throw \"Windows nvpn config backup hash mismatch expected=\$expectedHash actual=\$actualHash\"
  }
}
Copy-Item -LiteralPath \$backup -Destination \$config -Force
\$service = Get-Service NvpnService
if (\$service.Status -eq 'Running') {
  Restart-Service NvpnService -Force
} else {
  Start-Service NvpnService
}"
}

ensure_windows_service_running() {
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$service = Get-Service NvpnService
if (\$service.Status -ne 'Running') {
  Start-Service NvpnService
}
\$deadline = (Get-Date).AddSeconds(20)
do {
  \$service = Get-Service NvpnService
  if (\$service.Status -eq 'Running') { exit 0 }
  Start-Sleep -Milliseconds 500
} while ((Get-Date) -lt \$deadline)
throw \"NvpnService did not reach Running after cleanup restore\""
}

windows_snapshot_script() {
  cat <<'PS'
$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'Continue'

function Clean-PathValue($Path) {
  if (!$Path) { return "" }
  $text = [string]$Path
  if ($text.StartsWith('\\?\')) { return $text.Substring(4) }
  return $text
}

function Invoke-JsonCommand($Exe, $ArgsList) {
  try {
    $text = & $Exe @ArgsList 2>$null
    if (!$text) { return $null }
    return ($text | Out-String | ConvertFrom-Json)
  } catch {
    return $null
  }
}

function Service-BinaryFromPathName($PathName) {
  if (!$PathName) { return "" }
  $trimmed = $PathName.Trim()
  if ($trimmed.StartsWith('"')) {
    $idx = $trimmed.IndexOf('"', 1)
    if ($idx -gt 1) { return $trimmed.Substring(1, $idx - 1) }
  }
  return (($trimmed -split '\s+', 2)[0])
}

function File-HashOrEmpty($Path) {
  $clean = Clean-PathValue $Path
  if (!$clean -or !(Test-Path -LiteralPath $clean)) { return "" }
  try { return (Get-FileHash -Algorithm SHA256 -LiteralPath $clean).Hash.ToLowerInvariant() } catch { return "" }
}

$nvpnCommand = (Get-Command nvpn -ErrorAction SilentlyContinue)
$nvpn = if ($nvpnCommand) { $nvpnCommand.Source } else { "C:\Program Files\nostr-vpn\nvpn.exe" }
$serviceStatus = Invoke-JsonCommand $nvpn @("service", "status", "--json")
$daemonStatus = Invoke-JsonCommand $nvpn @("status", "--json", "--discover-secs", "0")
$svc = Get-CimInstance Win32_Service -Filter "Name='NvpnService'" -ErrorAction SilentlyContinue

$servicePid = $null
if ($serviceStatus -and $serviceStatus.pid) { $servicePid = [int]$serviceStatus.pid }
elseif ($svc -and $svc.ProcessId) { $servicePid = [int]$svc.ProcessId }

$proc = $null
$procPath = ""
$procCommandLine = ""
if ($servicePid -and $servicePid -gt 0) {
  $proc = Get-Process -Id $servicePid -ErrorAction SilentlyContinue
  $cimProc = Get-CimInstance Win32_Process -Filter "ProcessId=$servicePid" -ErrorAction SilentlyContinue
  if ($proc -and $proc.Path) { $procPath = $proc.Path }
  elseif ($cimProc -and $cimProc.ExecutablePath) { $procPath = $cimProc.ExecutablePath }
  if ($cimProc -and $cimProc.CommandLine) { $procCommandLine = $cimProc.CommandLine }
}

$configuredBinary = ""
if ($serviceStatus -and $serviceStatus.binary_path) { $configuredBinary = $serviceStatus.binary_path }
elseif ($svc -and $svc.PathName) { $configuredBinary = Service-BinaryFromPathName $svc.PathName }

$defaultGuid = ""
if ($daemonStatus -and $daemonStatus.daemon -and $daemonStatus.daemon.state -and $daemonStatus.daemon.state.network) {
  $defaultGuid = [string]$daemonStatus.daemon.state.network.defaultInterface
}

$adapters = @()
foreach ($adapter in (Get-NetAdapter -ErrorAction SilentlyContinue)) {
  $want = $false
  if ($adapter.Name -eq "nvpn") { $want = $true }
  if ($adapter.InterfaceDescription -match "Wintun|WireGuard|NostrVPN|Nostr VPN") { $want = $true }
  if ($defaultGuid -and ([string]$adapter.InterfaceGuid) -eq $defaultGuid) { $want = $true }
  if (!$want) { continue }
  $stats = Get-NetAdapterStatistics -Name $adapter.Name -ErrorAction SilentlyContinue
  $advanced = @(Get-NetAdapterAdvancedProperty -Name $adapter.Name -ErrorAction SilentlyContinue |
    Select-Object DisplayName, DisplayValue)
  $adapters += [pscustomobject]@{
    name = $adapter.Name
    description = $adapter.InterfaceDescription
    guid = [string]$adapter.InterfaceGuid
    status = [string]$adapter.Status
    link_speed = [string]$adapter.LinkSpeed
    received_bytes = if ($stats) { [uint64]$stats.ReceivedBytes } else { 0 }
    sent_bytes = if ($stats) { [uint64]$stats.SentBytes } else { 0 }
    received_unicast_packets = if ($stats) { [uint64]$stats.ReceivedUnicastPackets } else { 0 }
    sent_unicast_packets = if ($stats) { [uint64]$stats.SentUnicastPackets } else { 0 }
    received_discards = if ($stats) { [uint64]$stats.ReceivedDiscardedPackets } else { 0 }
    sent_discards = if ($stats) { [uint64]$stats.OutboundDiscardedPackets } else { 0 }
    received_errors = if ($stats) { [uint64]$stats.ReceivedPacketErrors } else { 0 }
    sent_errors = if ($stats) { [uint64]$stats.OutboundPacketErrors } else { 0 }
    advanced = $advanced
  }
}

[pscustomobject]@{
  captured_at = (Get-Date).ToUniversalTime().ToString("o")
  nvpn_command = $nvpn
  service_cim = if ($svc) {
    [pscustomobject]@{
      name = $svc.Name
      state = $svc.State
      process_id = [uint32]$svc.ProcessId
      path_name = $svc.PathName
      start_mode = $svc.StartMode
    }
  } else { $null }
  service_status = $serviceStatus
  daemon_status = $daemonStatus
  process = if ($proc) {
    [pscustomobject]@{
      pid = [uint32]$proc.Id
      path = $procPath
      command_line = $procCommandLine
      cpu_seconds = if ($proc.CPU) { [double]$proc.CPU } else { 0.0 }
      working_set_bytes = [uint64]$proc.WorkingSet64
      private_memory_bytes = [uint64]$proc.PrivateMemorySize64
      start_time = try { $proc.StartTime.ToUniversalTime().ToString("o") } catch { "" }
    }
  } else { $null }
  binaries = [pscustomobject]@{
    configured_path = $configuredBinary
    configured_hash = File-HashOrEmpty $configuredBinary
    process_path = $procPath
    process_hash = File-HashOrEmpty $procPath
  }
  adapters = $adapters
} | ConvertTo-Json -Depth 32
PS
}

capture_windows_snapshot() {
  local out="$1"
  run_windows_ps "$(windows_snapshot_script)" >"$out"
}

capture_linux_snapshot() {
  local out="$1"
  local nvpn_q
  nvpn_q="$(sh_q "$LINUX_NVPN")"
  run_linux_sh "set -euo pipefail
tmpdir=\$(mktemp -d)
trap 'rm -rf \"\$tmpdir\"' EXIT
svc=\$tmpdir/service.json
status=\$tmpdir/status.json
proc=\$tmpdir/process.json
adapters=\$tmpdir/adapters.json
if ! $nvpn_q service status --json >\"\$svc\" 2>/dev/null; then printf '{}\n' >\"\$svc\"; fi
if ! $nvpn_q status --json --discover-secs 0 >\"\$status\" 2>/dev/null; then printf '{}\n' >\"\$status\"; fi
pid=\$(jq -r '.pid // empty' \"\$svc\")
service_binary=\$(jq -r '.binary_path // empty' \"\$svc\")
service_hash=\"\"
process_hash=\"\"
hash_file() {
  local target=\"\$1\" digest=\"\"
  [[ -n \"\$target\" && -e \"\$target\" ]] || return 0
  digest=\$(sha256sum \"\$target\" 2>/dev/null | awk '{print \$1}' || true)
  if [[ -n \"\$digest\" ]]; then printf '%s\n' \"\$digest\"; return 0; fi
  digest=\$(sudo -n sha256sum \"\$target\" 2>/dev/null | awk '{print \$1}' || true)
  if [[ -n \"\$digest\" ]]; then printf '%s\n' \"\$digest\"; fi
}
resolve_process_exe() {
  local target=\"\$1\" resolved=\"\"
  resolved=\$(readlink -f \"\$target\" 2>/dev/null || true)
  if [[ -z \"\$resolved\" || \"\$resolved\" == \"\$target\" ]]; then
    resolved=\$(sudo -n readlink -f \"\$target\" 2>/dev/null || true)
  fi
  printf '%s' \"\$resolved\"
}
service_hash=\$(hash_file \"\$service_binary\")
if [[ -n \"\$pid\" && -d \"/proc/\$pid\" ]]; then
  exe=\$(resolve_process_exe \"/proc/\$pid/exe\")
  process_hash=\$(hash_file \"/proc/\$pid/exe\")
  [[ -z \"\$process_hash\" && -n \"\$exe\" ]] && process_hash=\$(hash_file \"\$exe\")
  cpu=\$(ps -p \"\$pid\" -o cputimes= 2>/dev/null | awk '{print \$1 + 0}')
  rss=\$(ps -p \"\$pid\" -o rss= 2>/dev/null | awk '{print (\$1 + 0) * 1024}')
  cmd=\$(tr '\0' ' ' <\"/proc/\$pid/cmdline\" 2>/dev/null || true)
  jq -n --arg pid \"\$pid\" --arg path \"\$exe\" --arg command_line \"\$cmd\" --arg cpu \"\$cpu\" --arg rss \"\$rss\" \
    '{pid:(\$pid|tonumber),path:\$path,command_line:\$command_line,cpu_seconds:(\$cpu|tonumber),rss_bytes:(\$rss|tonumber)}' >\"\$proc\"
else
  printf 'null\n' >\"\$proc\"
fi
default_iface=\$(jq -r '.daemon.state.network.defaultInterface // empty' \"\$status\")
route_iface=\$(ip route show default 2>/dev/null | awk '{print \$5; exit}')
{
  printf '%s\n' nvpn
  [[ -n \"\$default_iface\" ]] && printf '%s\n' \"\$default_iface\"
  [[ -n \"\$route_iface\" ]] && printf '%s\n' \"\$route_iface\"
} | awk 'NF && !seen[\$0]++' | while IFS= read -r iface; do
  [[ -d \"/sys/class/net/\$iface\" ]] || continue
  read_stat() { cat \"/sys/class/net/\$iface/statistics/\$1\" 2>/dev/null || printf '0'; }
  jq -n --arg name \"\$iface\" \
    --arg rx_bytes \"\$(read_stat rx_bytes)\" --arg tx_bytes \"\$(read_stat tx_bytes)\" \
    --arg rx_packets \"\$(read_stat rx_packets)\" --arg tx_packets \"\$(read_stat tx_packets)\" \
    --arg rx_dropped \"\$(read_stat rx_dropped)\" --arg tx_dropped \"\$(read_stat tx_dropped)\" \
    --arg rx_errors \"\$(read_stat rx_errors)\" --arg tx_errors \"\$(read_stat tx_errors)\" \
    '{name:\$name,received_bytes:(\$rx_bytes|tonumber),sent_bytes:(\$tx_bytes|tonumber),received_packets:(\$rx_packets|tonumber),sent_packets:(\$tx_packets|tonumber),received_discards:(\$rx_dropped|tonumber),sent_discards:(\$tx_dropped|tonumber),received_errors:(\$rx_errors|tonumber),sent_errors:(\$tx_errors|tonumber)}'
done | jq -s '.' >\"\$adapters\"
jq -n --slurpfile service \"\$svc\" --slurpfile status \"\$status\" --slurpfile process \"\$proc\" --slurpfile adapters \"\$adapters\" \
  --arg captured_at \"\$(date -u +%Y-%m-%dT%H:%M:%SZ)\" \
  --arg service_binary \"\$service_binary\" \
  --arg service_hash \"\$service_hash\" \
  --arg process_hash \"\$process_hash\" \
  '{captured_at:\$captured_at,service_status:(\$service[0] // {}),daemon_status:(\$status[0] // {}),process:(\$process[0] // null),binaries:{configured_path:\$service_binary,configured_hash:\$service_hash,process_path:(\$process[0].path // \"\"),process_hash:\$process_hash},adapters:(\$adapters[0] // [])}'" >"$out"
}

windows_hash() {
  local path="$1"
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$path = $(ps_sq "$path")
\$clean = [string]\$path
if (\$clean.StartsWith('\\\\?\\')) { \$clean = \$clean.Substring(4) }
if (!(Test-Path -LiteralPath \$clean)) { throw \"missing file: \$clean\" }
(Get-FileHash -Algorithm SHA256 -LiteralPath \$clean).Hash.ToLowerInvariant()"
}

linux_hash() {
  local path="$1"
  run_linux_sh "set -euo pipefail
path=$(sh_q "$path")
[[ -f \"\$path\" ]] || { echo \"missing file: \$path\" >&2; exit 1; }
sha256sum \"\$path\" | awk '{print \$1}'"
}

ensure_windows_nvpn_firewall_rule() {
  local exe="$1"
  local label="$2"
  local hash="$3"
  if ! is_true "$WINDOWS_NVPN_FIREWALL_RULE"; then
    return 0
  fi
  local rule_name="${WINDOWS_NVPN_FIREWALL_RULE_PREFIX}-${label}"
  printf 'ensuring Windows inbound UDP firewall rule for %s (%s)\n' "$label" "$exe" >&2
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$exe = $(ps_sq "$exe")
\$ruleName = $(ps_sq "$rule_name")
\$expectedHash = $(ps_sq "$hash")
\$clean = [string]\$exe
if (\$clean.StartsWith('\\\\?\\')) { \$clean = \$clean.Substring(4) }
if (!(Test-Path -LiteralPath \$clean)) { throw \"nvpn.exe not found for firewall rule: \$clean\" }
\$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath \$clean).Hash.ToLowerInvariant()
if (\$expectedHash -and \$actualHash -ne \$expectedHash) {
  throw \"firewall target hash mismatch for \$clean expected=\$expectedHash actual=\$actualHash\"
}
Remove-NetFirewallRule -DisplayName \$ruleName -ErrorAction SilentlyContinue | Out-Null
New-NetFirewallRule -DisplayName \$ruleName -Direction Inbound -Action Allow -Protocol UDP -Program \$clean -Profile Any | Out-Null" \
    >/dev/null
}

backup_linux_installed_binary() {
  if [[ -z "$CURRENT_LINUX_NVPN" || -n "$LINUX_INSTALLED_BACKUP" ]]; then
    return 0
  fi
  local installed_hash current_hash
  installed_hash="$(linux_hash "$LINUX_INSTALLED_NVPN" | tr -d '\r\n')"
  [[ -n "$installed_hash" ]] || die "could not hash installed Linux nvpn $LINUX_INSTALLED_NVPN"
  if [[ -n "$LINUX_INSTALLED_EXPECTED_HASH" && "$installed_hash" != "$LINUX_INSTALLED_EXPECTED_HASH" ]]; then
    die "installed Linux nvpn hash mismatch for $LINUX_INSTALLED_NVPN expected=$LINUX_INSTALLED_EXPECTED_HASH actual=$installed_hash"
  fi
  current_hash="$(linux_hash "$CURRENT_LINUX_NVPN" | tr -d '\r\n')"
  [[ -n "$current_hash" ]] || die "could not hash current Linux nvpn $CURRENT_LINUX_NVPN"
  if [[ "$installed_hash" == "$current_hash" ]] && ! is_true "$ALLOW_CURRENT_LINUX_AS_INSTALLED"; then
    die "installed Linux nvpn hash already matches current Linux row hash ($installed_hash); restore would not protect the installed baseline. Restore the installed binary first, set NVPN_WINLIN_INSTALLED_LINUX_SHA256, or set NVPN_WINLIN_ALLOW_CURRENT_LINUX_AS_INSTALLED=1 if this is intentional."
  fi
  LINUX_INSTALLED_BACKUP="/tmp/nvpn-winlin-installed-${installed_hash}-$$"
  LINUX_INSTALLED_BACKUP_HASH="$installed_hash"
  printf 'backing up Linux installed nvpn %s to %s\n' "$LINUX_INSTALLED_NVPN" "$LINUX_INSTALLED_BACKUP" >&2
  run_linux_sh "set -euo pipefail
src=$(sh_q "$LINUX_INSTALLED_NVPN")
dst=$(sh_q "$LINUX_INSTALLED_BACKUP")
sudo -n cp -p \"\$src\" \"\$dst\"
sudo -n chmod 755 \"\$dst\"
sha256sum \"\$dst\""
}

switch_linux_service() {
  local exe="$1"
  local label="$2"
  [[ -n "$LINUX_CONFIG" ]] || die "Linux service switch needs NVPN_WINLIN_LINUX_CONFIG or resolvable installed config"
  printf 'switching Linux service for %s to %s\n' "$label" "$exe" >&2
  run_linux_sh "set -euo pipefail
exe=$(sh_q "$exe")
config=$(sh_q "$LINUX_CONFIG")
[[ -x \"\$exe\" ]] || { echo \"nvpn binary not executable: \$exe\" >&2; exit 1; }
sudo -n \"\$exe\" service install --force --config \"\$config\""
  apply_linux_service_env "$label"
}

wait_for_linux_hash() {
  local expected_hash="$1"
  local label="$2"
  local tmp
  tmp="$(mktemp)"
  for _ in $(seq 1 40); do
    if capture_linux_snapshot "$tmp" >/dev/null 2>&1; then
      local running configured_hash process_hash configured
      running="$(jq -r '.service_status.running // false' "$tmp")"
      configured_hash="$(jq -r '.binaries.configured_hash // empty' "$tmp")"
      process_hash="$(jq -r '.binaries.process_hash // empty' "$tmp")"
      configured="$(jq -r '.binaries.configured_path // empty' "$tmp")"
      if [[ "$running" == "true" && "$configured_hash" == "$expected_hash" && "$process_hash" == "$expected_hash" ]]; then
        rm -f "$tmp"
        return 0
      fi
      printf 'waiting for %s Linux service hash: running=%s configured=%s configured_hash=%s process_hash=%s\n' \
        "$label" "$running" "$configured" "$configured_hash" "$process_hash" >&2
    fi
    sleep 1
  done
  cat "$tmp" >&2 || true
  rm -f "$tmp"
  die "Linux service did not switch to expected hash for $label"
}

switch_windows_service() {
  local exe="$1"
  local label="$2"
  local exe_ps
  local config_ps
  local label_ps
  local trace_ps
  local interval_ps
  local mtu_profile_ps
  local mtu_underlay_ps
  local mtu_tunnel_ps
  exe_ps="$(ps_sq "$exe")"
  config_ps="$(ps_sq "$WINDOWS_CONFIG")"
  label_ps="$(ps_sq "$label")"
  trace_ps="$(ps_sq "$WINDOWS_PIPELINE_TRACE")"
  interval_ps="$(ps_sq "$WINDOWS_PIPELINE_INTERVAL_SECS")"
  mtu_profile_ps="$(ps_sq "$MESH_MTU_PROFILE")"
  mtu_underlay_ps="$(ps_sq "$MESH_UNDERLAY_UDP_MTU")"
  mtu_tunnel_ps="$(ps_sq "$MESH_TUNNEL_MTU")"
  printf 'switching Windows service for %s to %s\n' "$label" "$exe" >&2
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$exe = $exe_ps
\$config = $config_ps
\$label = $label_ps
\$pipelineTrace = $trace_ps
\$pipelineInterval = $interval_ps
\$mtuProfile = $mtu_profile_ps
\$mtuUnderlay = $mtu_underlay_ps
\$mtuTunnel = $mtu_tunnel_ps
\$clean = [string]\$exe
if (\$clean.StartsWith('\\?\')) { \$clean = \$clean.Substring(4) }
if (!(Test-Path -LiteralPath \$clean)) { throw \"nvpn.exe not found: \$clean\" }

function Query-NvpnService {
  \$query = & sc.exe queryex NvpnService 2>&1
  [pscustomobject]@{ ExitCode = \$LASTEXITCODE; Text = (\$query | Out-String) }
}

function Get-NvpnServicePid(\$QueryText) {
  if (\$QueryText -match '(?m)^\\s*PID\\s*:\\s*(\\d+)\\s*$') { return [int]\$Matches[1] }
  return 0
}

function Stop-NvpnServiceProcess(\$QueryText) {
  \$pidValue = Get-NvpnServicePid \$QueryText
  if (\$pidValue -gt 0) {
    Stop-Process -Id \$pidValue -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
  }
}

\$query = Query-NvpnService
if (!(\$query.ExitCode -ne 0 -and \$query.Text -match '1060')) {
  Stop-NvpnServiceProcess \$query.Text
  & sc.exe delete NvpnService | Out-Null
  \$deadline = (Get-Date).AddSeconds(30)
  do {
    \$query = Query-NvpnService
    if (\$query.ExitCode -ne 0 -and \$query.Text -match '1060') { break }
    Stop-NvpnServiceProcess \$query.Text
    Start-Sleep -Milliseconds 250
  } while ((Get-Date) -lt \$deadline)
  if (!(\$query.ExitCode -ne 0 -and \$query.Text -match '1060')) {
    throw \"NvpnService did not finish deletion before reinstall: \$(\$query.Text)\"
  }
}
\$args = @('service', 'install', '--force')
if (\$config) { \$args += @('--config', \$config) }
& \$clean @args
if (\$LASTEXITCODE -ne 0) { exit \$LASTEXITCODE }
\$envPath = 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\NvpnService'
\$envValues = @()
if (\$label -ne 'restore-installed' -and \$pipelineTrace -match '^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$') {
  \$intervalValue = if (\$pipelineInterval -match '^\\d+$' -and [int]\$pipelineInterval -gt 0) { \$pipelineInterval } else { '1' }
  \$envValues += @(
    'NVPN_PIPELINE_TRACE=1',
    \"NVPN_PIPELINE_INTERVAL_SECS=\$intervalValue\",
    \"FIPS_PERF_INTERVAL_SECS=\$intervalValue\"
  )
}
if (\$label -ne 'restore-installed' -and \$mtuProfile) {
  \$envValues += \"NVPN_MESH_MTU_PROFILE=\$mtuProfile\"
}
if (\$label -ne 'restore-installed' -and \$mtuUnderlay) {
  \$envValues += \"NVPN_MESH_UNDERLAY_UDP_MTU=\$mtuUnderlay\"
}
if (\$label -ne 'restore-installed' -and \$mtuTunnel) {
  \$envValues += \"NVPN_MESH_TUNNEL_MTU=\$mtuTunnel\"
}
if (\$envValues.Count -gt 0) {
  New-ItemProperty -Path \$envPath -Name Environment -PropertyType MultiString -Value \$envValues -Force | Out-Null
  \$query = Query-NvpnService
  Stop-NvpnServiceProcess \$query.Text
  \$deadline = (Get-Date).AddSeconds(10)
  do {
    \$query = Query-NvpnService
    if (\$query.Text -match '(?m)^\\s*STATE\\s*:\\s*\\d+\\s+RUNNING\\b') { break }
    & sc.exe start NvpnService | Out-Null
    Start-Sleep -Milliseconds 500
  } while ((Get-Date) -lt \$deadline)
  \$query = Query-NvpnService
  if (!(\$query.Text -match '(?m)^\\s*STATE\\s*:\\s*\\d+\\s+RUNNING\\b')) {
    throw \"NvpnService did not restart with measured-service environment: \$(\$query.Text)\"
  }
} else {
  Remove-ItemProperty -Path \$envPath -Name Environment -ErrorAction SilentlyContinue
}
exit 0"
  if [[ "$label" != "restore-installed" ]] && measured_service_env_enabled; then
    WINDOWS_SERVICE_ENV_APPLIED=1
  fi
}

wait_for_windows_hash() {
  local expected_hash="$1"
  local label="$2"
  local tmp
  tmp="$(mktemp)"
  for _ in $(seq 1 40); do
    if capture_windows_snapshot "$tmp" >/dev/null 2>&1; then
      local running configured process_hash configured_hash
      running="$(jq -r '.service_status.running // false' "$tmp")"
      process_hash="$(jq -r '.binaries.process_hash // empty' "$tmp")"
      configured_hash="$(jq -r '.binaries.configured_hash // empty' "$tmp")"
      configured="$(jq -r '.binaries.configured_path // empty' "$tmp")"
      if [[ "$running" == "true" && "$process_hash" == "$expected_hash" && "$configured_hash" == "$expected_hash" ]]; then
        rm -f "$tmp"
        return 0
      fi
      printf 'waiting for %s service hash: running=%s configured=%s configured_hash=%s process_hash=%s\n' \
        "$label" "$running" "$configured" "$configured_hash" "$process_hash" >&2
    fi
    sleep 1
  done
  cat "$tmp" >&2 || true
  rm -f "$tmp"
  die "Windows service did not switch to expected hash for $label"
}

build_current_windows() {
  local build_json="$OUTPUT_DIR/current-windows-build.json"
  mkdir -p "$OUTPUT_DIR"

  if [[ -n "$CURRENT_WINDOWS_NVPN" ]]; then
    local hash
    hash="$(windows_hash "$CURRENT_WINDOWS_NVPN" | tr -d '\r\n')"
    jq -n --arg path "$CURRENT_WINDOWS_NVPN" --arg hash "$hash" \
      '{path:$path,sha256:$hash,source:"NVPN_WINLIN_CURRENT_WINDOWS_NVPN"}' >"$build_json"
    return 0
  fi

  if ! is_true "$SKIP_WINDOWS_SYNC"; then
    NVPN_WINDOWS_SSH_HOST="$WINDOWS_SSH" \
      NVPN_WINDOWS_SSH_PROXY_COMMAND="$WINDOWS_SSH_PROXY_COMMAND" \
      NVPN_WINDOWS_SSH_JUMP="$WINDOWS_SSH_JUMP" \
      NVPN_WINDOWS_GUEST_REPO_PATH="$WINDOWS_REPO" \
      "$ROOT_DIR/scripts/windows-vm-git-sync.sh" "$WINDOWS_SSH"
  fi

  local fips_line=""
  if [[ -n "$WINDOWS_FIPS_REPO" ]]; then
    fips_line="\$env:NVPN_FIPS_REPO_PATH = $(ps_sq "$WINDOWS_FIPS_REPO")"
  fi

  if ! is_true "$SKIP_CURRENT_BUILD"; then
    run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
Set-Location $(ps_sq "$WINDOWS_REPO")
$fips_line
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-build.ps1 -Configuration Release
exit \$LASTEXITCODE"
  fi

  CURRENT_WINDOWS_NVPN="${WINDOWS_REPO}\\target\\release\\nvpn.exe"
  local hash
  hash="$(windows_hash "$CURRENT_WINDOWS_NVPN" | tr -d '\r\n')"
  jq -n \
    --arg path "$CURRENT_WINDOWS_NVPN" \
    --arg hash "$hash" \
    --arg repo "$WINDOWS_REPO" \
    --arg local_head "$(git -C "$ROOT_DIR" rev-parse HEAD)" \
    '{path:$path,sha256:$hash,windows_repo:$repo,local_head:$local_head,source:"windows-build.ps1 Release"}' \
    >"$build_json"
}

resolve_installed_windows_nvpn() {
  if [[ -n "$WINDOWS_INSTALLED_NVPN" ]]; then
    :
  else
    local tmp
    tmp="$(mktemp)"
    capture_windows_snapshot "$tmp"
    WINDOWS_INSTALLED_NVPN="$(jq -r '.binaries.configured_path // .nvpn_command // empty' "$tmp")"
    if [[ -z "$WINDOWS_CONFIG" ]]; then
      WINDOWS_CONFIG="$(jq -r 'try (.service_cim.path_name // "" | capture("--config\\s+\"(?<config>[^\"]+)\"").config) catch ""' "$tmp")"
    fi
    rm -f "$tmp"
  fi
  [[ -n "$WINDOWS_INSTALLED_NVPN" ]] || die "could not resolve installed Windows nvpn path"
  [[ -n "$WINDOWS_CONFIG" ]] || printf 'warning: could not resolve Windows service config path; install will use nvpn default\n' >&2
}

resolve_installed_linux_nvpn() {
  local tmp
  tmp="$(mktemp)"
  capture_linux_snapshot "$tmp"
  if [[ -z "$LINUX_INSTALLED_NVPN" ]]; then
    LINUX_INSTALLED_NVPN="$(jq -r '.service_status.binary_path // .binaries.configured_path // empty' "$tmp")"
  fi
  if [[ -z "$LINUX_CONFIG" ]]; then
    LINUX_CONFIG="$(jq -r 'try (.process.command_line // "" | capture("--config\\s+(?<config>\\S+)").config) catch ""' "$tmp")"
  fi
  rm -f "$tmp"
  [[ -n "$LINUX_INSTALLED_NVPN" ]] || die "could not resolve installed Linux nvpn path"
  [[ -n "$LINUX_CONFIG" ]] || die "could not resolve Linux service config path; set NVPN_WINLIN_LINUX_CONFIG"
}

row_contains_current() {
  case ",$ROWS," in
    *,current,*) return 0 ;;
    *) return 1 ;;
  esac
}

row_contains_installed() {
  case ",$ROWS," in
    *,installed,*) return 0 ;;
    *) return 1 ;;
  esac
}

validate_service_env() {
  if [[ -n "$MESH_MTU_PROFILE" && ! "$MESH_MTU_PROFILE" =~ ^[A-Za-z0-9_.:-]+$ ]]; then
    die "NVPN_WINLIN_MESH_MTU_PROFILE contains unsupported characters"
  fi
  if [[ -n "$MESH_UNDERLAY_UDP_MTU" && ! "$MESH_UNDERLAY_UDP_MTU" =~ ^[0-9]+$ ]]; then
    die "NVPN_WINLIN_MESH_UNDERLAY_UDP_MTU must be numeric"
  fi
  if [[ -n "$MESH_TUNNEL_MTU" && ! "$MESH_TUNNEL_MTU" =~ ^[0-9]+$ ]]; then
    die "NVPN_WINLIN_MESH_TUNNEL_MTU must be numeric"
  fi
}

measured_service_env_enabled() {
  is_true "$WINDOWS_PIPELINE_TRACE" ||
    [[ -n "$MESH_MTU_PROFILE" || -n "$MESH_UNDERLAY_UDP_MTU" || -n "$MESH_TUNNEL_MTU" ]]
}

measured_linux_mtu_env_enabled() {
  [[ -n "$MESH_MTU_PROFILE" || -n "$MESH_UNDERLAY_UDP_MTU" || -n "$MESH_TUNNEL_MTU" ]]
}

clear_windows_service_env() {
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
\$envPath = 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\NvpnService'
Remove-ItemProperty -Path \$envPath -Name Environment -ErrorAction SilentlyContinue
if ((Get-Service NvpnService -ErrorAction SilentlyContinue).Status -eq 'Running') {
  Restart-Service NvpnService -Force -ErrorAction SilentlyContinue
}" >/dev/null
  WINDOWS_SERVICE_ENV_APPLIED=0
}

clear_linux_service_env() {
  run_linux_sh "set -euo pipefail
dropin=/etc/systemd/system/nvpn.service.d/50-nvpn-winlin-perf-env.conf
if [[ -e \"\$dropin\" ]]; then
  sudo -n rm -f \"\$dropin\"
  sudo -n systemctl daemon-reload
  sudo -n systemctl restart nvpn.service
fi"
  LINUX_SERVICE_ENV_APPLIED=0
}

apply_linux_service_env() {
  local label="$1"
  if [[ "$label" == "restore-installed" ]] || ! measured_linux_mtu_env_enabled; then
    clear_linux_service_env >/dev/null 2>&1 || true
    return 0
  fi

  printf 'applying Linux measured-service env for %s\n' "$label" >&2
  run_linux_sh "set -euo pipefail
sudo -n install -d -m 0755 /etc/systemd/system/nvpn.service.d
tmp=\$(mktemp)
{
  printf '%s\n' '[Service]'
  if [[ -n $(sh_q "$MESH_MTU_PROFILE") ]]; then
    printf 'Environment=%s\n' $(sh_q "\"NVPN_MESH_MTU_PROFILE=$MESH_MTU_PROFILE\"")
  fi
  if [[ -n $(sh_q "$MESH_UNDERLAY_UDP_MTU") ]]; then
    printf 'Environment=%s\n' $(sh_q "\"NVPN_MESH_UNDERLAY_UDP_MTU=$MESH_UNDERLAY_UDP_MTU\"")
  fi
  if [[ -n $(sh_q "$MESH_TUNNEL_MTU") ]]; then
    printf 'Environment=%s\n' $(sh_q "\"NVPN_MESH_TUNNEL_MTU=$MESH_TUNNEL_MTU\"")
  fi
} >\"\$tmp\"
sudo -n install -m 0644 \"\$tmp\" /etc/systemd/system/nvpn.service.d/50-nvpn-winlin-perf-env.conf
rm -f \"\$tmp\"
sudo -n systemctl daemon-reload
sudo -n systemctl restart nvpn.service"
  LINUX_SERVICE_ENV_APPLIED=1
}

write_selected_pair_json() {
  local win_status="$1"
  local lin_status="$2"
  local win_underlay="$3"
  local linux_underlay="$4"
  local out="$5"

  jq -n \
    --slurpfile win "$win_status" \
    --slurpfile lin "$lin_status" \
    --arg windows_underlay "$win_underlay" \
    --arg linux_underlay "$linux_underlay" \
    '
    def tunnel_addr($peer): (($peer.tunnel_ip // "") | split("/")[0]);
    def direct_peer($peer; $underlay):
      ($peer != null)
      and (($peer.endpoint // "") == "fips")
      and (($peer.reachable // false) == true)
      and (($peer.error // null) == null)
      and (($peer.fips_transport_type // "") == "udp")
      and (($peer.fips_transport_addr // "") | startswith($underlay + ":"))
      and (($peer.runtime_endpoint // "") == ($peer.fips_transport_addr // ""))
      and ((($peer.fips_last_outbound_route // "") == "") or (($peer.fips_last_outbound_route // "") == "direct"));
    def peer_summary($peer):
      if $peer == null then null else
        {
          participant_pubkey: ($peer.participant_pubkey // ""),
          node_id: ($peer.node_id // ""),
          tunnel_ip: ($peer.tunnel_ip // ""),
          endpoint: ($peer.endpoint // ""),
          fips_endpoint_npub: ($peer.fips_endpoint_npub // ""),
          fips_transport_addr: ($peer.fips_transport_addr // ""),
          fips_transport_type: ($peer.fips_transport_type // ""),
          runtime_endpoint: ($peer.runtime_endpoint // ""),
          reachable: ($peer.reachable // false),
          error: ($peer.error // null),
          last_mesh_seen_at: ($peer.last_mesh_seen_at // null),
          last_fips_seen_at: ($peer.last_fips_seen_at // null),
          last_fips_data_seen_at: ($peer.last_fips_data_seen_at // null),
          last_fips_control_seen_at: ($peer.last_fips_control_seen_at // null),
          last_handshake_at: ($peer.last_handshake_at // null),
          fips_srtt_ms: ($peer.fips_srtt_ms // null),
          fips_last_outbound_route: ($peer.fips_last_outbound_route // null),
          direct_probe_pending: ($peer.direct_probe_pending // null),
          direct_probe_auto_reconnect: ($peer.direct_probe_auto_reconnect // null),
          advertised_routes: ($peer.advertised_routes // [])
        }
      end;
    def reason($ok; $message): if $ok then empty else $message end;
    ($win[0]) as $w |
    ($lin[0]) as $l |
    ([$w.daemon.state.peers[]? | select((.fips_transport_addr // "") | startswith($linux_underlay + ":"))]) as $win_matches |
    ([$l.daemon.state.peers[]? | select((.fips_transport_addr // "") | startswith($windows_underlay + ":"))]) as $lin_matches |
    ($win_matches[0] // null) as $windows_view_peer |
    ($lin_matches[0] // null) as $linux_view_peer |
    (tunnel_addr($windows_view_peer)) as $linux_tunnel |
    (tunnel_addr($linux_view_peer)) as $windows_tunnel |
    ([
      reason(($windows_underlay != "" and $linux_underlay != "" and $windows_underlay != $linux_underlay); "Windows/Linux underlay IPs must be present and distinct"),
      reason((($w.daemon.state.local_endpoint // "") | startswith($windows_underlay + ":")); "Windows local FIPS endpoint is not on the selected Windows underlay"),
      reason((($l.daemon.state.local_endpoint // "") | startswith($linux_underlay + ":")); "Linux local FIPS endpoint is not on the selected Linux underlay"),
      reason(($win_matches | length) == 1; "Windows status must have exactly one peer whose FIPS transport is the Linux underlay"),
      reason(($lin_matches | length) == 1; "Linux status must have exactly one peer whose FIPS transport is the Windows underlay"),
      reason(direct_peer($windows_view_peer; $linux_underlay); "Windows-selected Linux peer is not reachable direct UDP to the Linux underlay"),
      reason(direct_peer($linux_view_peer; $windows_underlay); "Linux-selected Windows peer is not reachable direct UDP to the Windows underlay"),
      reason(($linux_tunnel != "" and $windows_tunnel != "" and $linux_tunnel != $windows_tunnel); "selected peer tunnel IPs must be present and distinct")
    ]) as $reasons |
    {
      windows_underlay: $windows_underlay,
      linux_underlay: $linux_underlay,
      windows_tunnel: $windows_tunnel,
      linux_tunnel: $linux_tunnel,
      windows_view_transport: ($windows_view_peer.fips_transport_addr // ""),
      linux_view_transport: ($linux_view_peer.fips_transport_addr // ""),
      windows_view_peer: peer_summary($windows_view_peer),
      linux_view_peer: peer_summary($linux_view_peer),
      direct_pair: {
        ok: (($reasons | length) == 0),
        reasons: $reasons,
        windows_local_endpoint: ($w.daemon.state.local_endpoint // ""),
        linux_local_endpoint: ($l.daemon.state.local_endpoint // ""),
        windows_status_peer_match_count: ($win_matches | length),
        linux_status_peer_match_count: ($lin_matches | length)
      }
    }' >"$out"
}

assert_selected_pair_direct() {
  local selected_pair="$1"
  if [[ "$(jq -r '.direct_pair.ok // false' "$selected_pair")" != "true" ]]; then
    jq '.direct_pair' "$selected_pair" >&2
    if ! is_true "$ALLOW_NON_DIRECT"; then
      die "selected Windows/Linux peer is not a reciprocal direct FIPS UDP pair"
    fi
  fi
}

select_pair() {
  local row_dir="$1"
  local win_status="$row_dir/windows-status.json"
  local lin_status="$row_dir/linux-status.json"

  capture_windows_snapshot "$row_dir/windows-select-snapshot.json"
  capture_linux_snapshot "$row_dir/linux-select-snapshot.json"
  jq '.daemon_status' "$row_dir/windows-select-snapshot.json" >"$win_status"
  jq '.daemon_status' "$row_dir/linux-select-snapshot.json" >"$lin_status"

  local win_underlay linux_underlay
  win_underlay="$(jq -r '.daemon.state.network.primaryIpv4 // empty' "$win_status")"
  linux_underlay="$(jq -r '.daemon.state.network.primaryIpv4 // empty' "$lin_status")"
  [[ -n "$win_underlay" ]] || die "Windows status missing primary underlay IP"
  [[ -n "$linux_underlay" ]] || die "Linux status missing primary underlay IP"

  write_selected_pair_json "$win_status" "$lin_status" "$win_underlay" "$linux_underlay" "$row_dir/selected-pair.json"
  assert_selected_pair_direct "$row_dir/selected-pair.json"
  local linux_tunnel windows_tunnel
  linux_tunnel="$(jq -r '.linux_tunnel // empty' "$row_dir/selected-pair.json")"
  windows_tunnel="$(jq -r '.windows_tunnel // empty' "$row_dir/selected-pair.json")"
  [[ -n "$linux_tunnel" ]] || die "could not resolve Linux tunnel IP from Windows status"
  [[ -n "$windows_tunnel" ]] || die "could not resolve Windows tunnel IP from Linux status"
}

wait_for_direct_pair() {
  local row="$1"
  local row_dir="$OUTPUT_DIR/$row"
  local tmpdir
  tmpdir="$(mktemp -d)"
  for attempt in $(seq 1 45); do
    local wait_reason="status snapshot failed"
    if capture_windows_snapshot "$tmpdir/windows.json" >/dev/null 2>&1 &&
      capture_linux_snapshot "$tmpdir/linux.json" >/dev/null 2>&1; then
      jq '.daemon_status' "$tmpdir/windows.json" >"$tmpdir/windows-status.json"
      jq '.daemon_status' "$tmpdir/linux.json" >"$tmpdir/linux-status.json"
      local win_underlay linux_underlay linux_tunnel windows_tunnel
      win_underlay="$(jq -r '.daemon.state.network.primaryIpv4 // empty' "$tmpdir/windows-status.json")"
      linux_underlay="$(jq -r '.daemon.state.network.primaryIpv4 // empty' "$tmpdir/linux-status.json")"
      wait_reason="missing primary underlay IP in daemon status"
      if [[ -n "$win_underlay" && -n "$linux_underlay" ]]; then
        linux_tunnel="$(jq -r --arg ip "$linux_underlay" \
          '[.daemon.state.peers[]? | select(((.fips_transport_addr // "") | startswith($ip + ":")))] | first | .tunnel_ip // empty | split("/")[0]' \
          "$tmpdir/windows-status.json")"
        windows_tunnel="$(jq -r --arg ip "$win_underlay" \
          '[.daemon.state.peers[]? | select(((.fips_transport_addr // "") | startswith($ip + ":")))] | first | .tunnel_ip // empty | split("/")[0]' \
          "$tmpdir/linux-status.json")"
        wait_reason="missing reciprocal peer with matching FIPS transport"
        if [[ -n "$linux_tunnel" && -n "$windows_tunnel" ]]; then
          write_selected_pair_json "$tmpdir/windows-status.json" "$tmpdir/linux-status.json" "$win_underlay" "$linux_underlay" "$tmpdir/selected-pair.json"
          if [[ "$(jq -r '.direct_pair.ok // false' "$tmpdir/selected-pair.json")" != "true" ]]; then
            wait_reason="$(jq -r '.direct_pair.reasons | join("; ")' "$tmpdir/selected-pair.json")"
            printf 'waiting for %s reciprocal direct FIPS peer pair: %s\n' "$row" "$wait_reason" >&2
            jq -n \
              --arg row "$row" \
              --arg attempt "$attempt" \
              --arg attempts "45" \
              --arg reason "$wait_reason" \
              '{row:$row,attempt:($attempt|tonumber),attempts:($attempts|tonumber),reason:$reason}' \
              >"$tmpdir/wait-state.json"
            sleep 2
            continue
          fi
          rm -rf "$tmpdir"
          return 0
        fi
      fi
    fi
    jq -n \
      --arg row "$row" \
      --arg attempt "$attempt" \
      --arg attempts "45" \
      --arg reason "$wait_reason" \
      '{row:$row,attempt:($attempt|tonumber),attempts:($attempts|tonumber),reason:$reason}' \
      >"$tmpdir/wait-state.json"
    printf 'waiting for %s direct Windows/Linux peer pair (%s/45): %s\n' "$row" "$attempt" "$wait_reason" >&2
    sleep 2
  done
  local failure_dir="$row_dir/direct-pair-timeout"
  rm -rf "$failure_dir"
  mkdir -p "$failure_dir"
  cp -f "$tmpdir"/* "$failure_dir"/ 2>/dev/null || true
  rm -rf "$tmpdir"
  die "timed out waiting for $row direct Windows/Linux peer pair"
}

start_linux_probe_server() {
  local port="$1"
  local label="$2"
  local log="/tmp/nvpn-winlin-probe/${label}-server.jsonl"
  run_linux_sh "set -euo pipefail
rm -f $(sh_q "$log")
nohup /tmp/nvpn-winlin-probe/tcp_probe server 0.0.0.0:$(sh_q "$port") >$(sh_q "$log") 2>$(sh_q "$log.err") &
printf '%s\n' \"\$!\""
  printf '%s\n' "$log"
}

start_linux_fips_capture() {
  local label="$1"
  local duration=$((DURATION_SECS + 15))
  local base="/tmp/nvpn-winlin-probe/${label}-fips-direct"
  local log="${base}.tcpdump"
  local err="${base}.err"
  local pidfile="${base}.pid"
  run_linux_sh "set -euo pipefail
command -v tcpdump >/dev/null
iface=$(sh_q "$LINUX_CAPTURE_IFACE")
if [[ -z \"\$iface\" ]]; then
  iface=\$(ip -o -4 route show default | awk '{print \$5; exit}')
fi
[[ -n \"\$iface\" ]] || { echo 'could not resolve Linux capture interface' >&2; exit 1; }
rm -f $(sh_q "$log") $(sh_q "$err") $(sh_q "$pidfile")
nohup sudo -n timeout $(sh_q "${duration}s") tcpdump -i \"\$iface\" -n -tt -l -q 'udp and port 51820' >$(sh_q "$log") 2>$(sh_q "$err") &
printf '%s\n' \"\$!\" >$(sh_q "$pidfile")
jq -n --arg iface \"\$iface\" --arg pid \"\$(cat $(sh_q "$pidfile"))\" --arg stdout $(sh_q "$log") --arg stderr $(sh_q "$err") '{iface:\$iface,pid:(\$pid|tonumber),stdout:\$stdout,stderr:\$stderr}'"
}

stop_linux_fips_capture() {
  local pid="$1"
  [[ -n "$pid" && "$pid" != "null" ]] || return 0
  run_linux_sh "sudo -n kill -INT $(sh_q "$pid") >/dev/null 2>&1 || kill -INT $(sh_q "$pid") >/dev/null 2>&1 || true
sleep 1" >/dev/null 2>&1 || true
}

summarize_direct_fips_capture() {
  local row="$1"
  local direction="$2"
  local capture="$3"
  local client_jsonl="$4"
  local selected_pair="$OUTPUT_DIR/$row/selected-pair.json"
  local summary="$capture/summary.json"
  python3 - "$capture/stdout.txt" "$selected_pair" "$client_jsonl" "$summary" "$direction" <<'PY'
import json
import re
import sys
from pathlib import Path

capture_path, pair_path, client_jsonl_path, out_path = map(Path, sys.argv[1:5])
direction = sys.argv[5]
pair = json.loads(pair_path.read_text())
windows = pair["windows_underlay"]
linux = pair["linux_underlay"]
client_summary = {}
for raw_line in client_jsonl_path.read_text(errors="replace").splitlines():
    try:
        parsed = json.loads(raw_line)
    except json.JSONDecodeError:
        continue
    if parsed.get("role") == "client":
        client_summary = parsed
line_re = re.compile(
    r"\bIP\s+(\d+\.\d+\.\d+\.\d+)\.(\d+)\s+>\s+(\d+\.\d+\.\d+\.\d+)\.(\d+):\s+UDP,\s+length\s+(\d+)"
)
packets = 0
direct_packets = 0
non_direct_packets = 0
payload_bytes = 0
direct_payload_bytes = 0
non_direct_payload_bytes = 0
max_payload_bytes = 0
direct_max_payload_bytes = 0
direct_payload_gt_1400 = 0
direct_payload_gt_9000 = 0
direct_flow_stats = {}
peers = {}
for line in capture_path.read_text(errors="replace").splitlines():
    match = line_re.search(line)
    if not match:
        continue
    src, sport, dst, dport, length = match.groups()
    length = int(length)
    if sport != "51820" and dport != "51820":
        continue
    packets += 1
    payload_bytes += length
    max_payload_bytes = max(max_payload_bytes, length)
    direct = {src, dst} == {windows, linux}
    if direct:
        direct_packets += 1
        direct_payload_bytes += length
        direct_max_payload_bytes = max(direct_max_payload_bytes, length)
        direct_payload_gt_1400 += int(length > 1400)
        direct_payload_gt_9000 += int(length > 9000)
        flow = f"{src}>{dst}"
        entry = direct_flow_stats.setdefault(
            flow,
            {
                "packets": 0,
                "payload_bytes": 0,
                "max_payload_bytes": 0,
                "payload_gt_1400": 0,
                "payload_gt_9000": 0,
            },
        )
        entry["packets"] += 1
        entry["payload_bytes"] += length
        entry["max_payload_bytes"] = max(entry["max_payload_bytes"], length)
        entry["payload_gt_1400"] += int(length > 1400)
        entry["payload_gt_9000"] += int(length > 9000)
    else:
        non_direct_packets += 1
        non_direct_payload_bytes += length
        other = dst if src == linux else src if dst == linux else f"{src}>{dst}"
        entry = peers.setdefault(other, {"packets": 0, "payload_bytes": 0})
        entry["packets"] += 1
        entry["payload_bytes"] += length

client_bytes = int(client_summary.get("bytes") or 0)
min_direct_bytes = int(client_bytes * 0.5)
expected_src = windows if direction == "windows_to_linux" else linux
expected_dst = linux if direction == "windows_to_linux" else windows
primary_flow_key = f"{expected_src}>{expected_dst}"
primary_flow_payload_bytes = int(
    direct_flow_stats.get(primary_flow_key, {}).get("payload_bytes", 0)
)
min_primary_direct_bytes = int(client_bytes * 0.5)
max_non_direct_bytes = max(1_000_000, int(max(direct_payload_bytes, 1) * 0.05))
ok = (
    packets > 0
    and direct_payload_bytes >= min_direct_bytes
    and primary_flow_payload_bytes >= min_primary_direct_bytes
    and non_direct_payload_bytes <= max_non_direct_bytes
)
out = {
    "direction": direction,
    "windows_underlay": windows,
    "linux_underlay": linux,
    "client_bytes": client_bytes,
    "packets": packets,
    "payload_bytes": payload_bytes,
    "max_payload_bytes": max_payload_bytes,
    "direct_packets": direct_packets,
    "direct_payload_bytes": direct_payload_bytes,
    "direct_max_payload_bytes": direct_max_payload_bytes,
    "direct_payload_gt_1400": direct_payload_gt_1400,
    "direct_payload_gt_9000": direct_payload_gt_9000,
    "direct_flow_stats": direct_flow_stats,
    "primary_direct_flow": primary_flow_key,
    "primary_direct_payload_bytes": primary_flow_payload_bytes,
    "non_direct_packets": non_direct_packets,
    "non_direct_payload_bytes": non_direct_payload_bytes,
    "non_direct_peers": peers,
    "min_direct_payload_bytes": min_direct_bytes,
    "min_primary_direct_payload_bytes": min_primary_direct_bytes,
    "max_non_direct_payload_bytes": max_non_direct_bytes,
    "direct_payload_ratio": (direct_payload_bytes / client_bytes) if client_bytes else None,
    "primary_direct_payload_ratio": (primary_flow_payload_bytes / client_bytes) if client_bytes else None,
    "non_direct_payload_ratio": (non_direct_payload_bytes / client_bytes) if client_bytes else None,
    "ok": ok,
}
Path(out_path).write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")
if not ok:
    print(json.dumps(out, indent=2, sort_keys=True), file=sys.stderr)
    sys.exit(1)
PY
}

start_windows_probe_server() {
  local port="$1"
  local label="$2"
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$dir = Join-Path \$env:TEMP 'nvpn-winlin-probe'
\$probe = Join-Path \$dir 'tcp_probe.exe'
\$out = Join-Path \$dir ($(ps_sq "${label}-server.jsonl"))
\$err = Join-Path \$dir ($(ps_sq "${label}-server.err"))
Remove-Item -Force \$out, \$err -ErrorAction SilentlyContinue
\$p = Start-Process -FilePath \$probe -ArgumentList @('server', '0.0.0.0:$port') -RedirectStandardOutput \$out -RedirectStandardError \$err -PassThru
[pscustomobject]@{ pid = \$p.Id; stdout = \$out; stderr = \$err } | ConvertTo-Json -Compress"
}

run_windows_probe_client() {
  local connect="$1"
  local port="$2"
  local label="$3"
  local direction="$4"
  local mode="$5"
  local out="$6"
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Stop'
\$probe = Join-Path (Join-Path \$env:TEMP 'nvpn-winlin-probe') 'tcp_probe.exe'
& \$probe client $(ps_sq "${connect}:${port}") $(ps_sq "$mode") $(ps_sq "$DURATION_SECS") $(ps_sq "$label") $(ps_sq "$direction")
exit \$LASTEXITCODE" >"$out"
}

run_linux_probe_client() {
  local connect="$1"
  local port="$2"
  local label="$3"
  local direction="$4"
  local out="$5"
  run_linux_sh "/tmp/nvpn-winlin-probe/tcp_probe client $(sh_q "${connect}:${port}") send $(sh_q "$DURATION_SECS") $(sh_q "$label") $(sh_q "$direction")" >"$out"
}

fetch_windows_file() {
  local remote_path="$1"
  local out="$2"
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
Get-Content -Raw -Path $(ps_sq "$remote_path") -ErrorAction SilentlyContinue" >"$out"
}

fetch_windows_file_tail() {
  local remote_path="$1"
  local lines="$2"
  local out="$3"
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
Get-Content -Tail ([int]$(ps_sq "$lines")) -Path $(ps_sq "$remote_path") -ErrorAction SilentlyContinue" >"$out"
}

capture_windows_daemon_log_marker() {
  local snapshot="$1"
  local out="$2"
  local remote_log
  remote_log="$(jq -r '.daemon_status.daemon.log_file // .daemon_status.log_file // empty' "$snapshot")"
  if [[ -z "$remote_log" || "$remote_log" == "null" ]]; then
    jq -n '{path:"", length:0}' >"$out"
    return 0
  fi
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
\$path = $(ps_sq "$remote_log")
\$length = [int64]0
if (Test-Path -LiteralPath \$path) {
  \$item = Get-Item -LiteralPath \$path -ErrorAction SilentlyContinue
  if (\$item) { \$length = [int64]\$item.Length }
}
[pscustomobject]@{
  path = \$path
  length = \$length
  captured_at = (Get-Date).ToUniversalTime().ToString('o')
} | ConvertTo-Json -Depth 4" >"$out"
}

capture_windows_daemon_log_delta() {
  local marker="$1"
  local out="$2"
  local remote_log offset
  remote_log="$(jq -r '.path // empty' "$marker")"
  offset="$(jq -r '.length // 0' "$marker")"
  if [[ -z "$remote_log" || "$remote_log" == "null" ]]; then
    printf '' >"$out"
    return 0
  fi
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
\$path = $(ps_sq "$remote_log")
\$offset = [int64]$(ps_sq "$offset")
\$maxLines = [int]$(ps_sq "$WINDOWS_DAEMON_LOG_TAIL_LINES")
if (!(Test-Path -LiteralPath \$path)) { exit 0 }
\$stream = [System.IO.File]::Open(\$path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
try {
  if (\$offset -lt 0 -or \$offset -gt \$stream.Length) { \$offset = 0 }
  [void]\$stream.Seek(\$offset, [System.IO.SeekOrigin]::Begin)
  \$reader = [System.IO.StreamReader]::new(\$stream, [System.Text.Encoding]::UTF8, \$true)
  try {
    \$text = \$reader.ReadToEnd()
  } finally {
    \$reader.Dispose()
  }
} finally {
  \$stream.Dispose()
}
if (\$maxLines -gt 0) {
  \$lines = \$text -split '\r?\n'
  if (\$lines.Length -gt \$maxLines) {
    \$lines = \$lines[([Math]::Max(0, \$lines.Length - \$maxLines))..(\$lines.Length - 1)]
  }
  [Console]::Out.Write(\$lines -join [Environment]::NewLine)
} else {
  [Console]::Out.Write(\$text)
}" >"$out"
}

fetch_linux_file() {
  local remote_path="$1"
  local out="$2"
  run_linux_sh "cat $(sh_q "$remote_path") 2>/dev/null || true" >"$out"
}

capture_windows_daemon_log_tail() {
  local snapshot="$1"
  local out="$2"
  local remote_log
  remote_log="$(jq -r '.daemon_status.daemon.log_file // .daemon_status.log_file // empty' "$snapshot")"
  if [[ -z "$remote_log" || "$remote_log" == "null" ]]; then
    printf '' >"$out"
    return 0
  fi
  fetch_windows_file_tail "$remote_log" "$WINDOWS_DAEMON_LOG_TAIL_LINES" "$out"
}

capture_windows_ping() {
  local target="$1"
  local count="$2"
  local out="$3"
  run_windows_ps "\$ProgressPreference = 'SilentlyContinue'
\$ErrorActionPreference = 'Continue'
\$target = $(ps_sq "$target")
\$count = [int]$(ps_sq "$count")
\$raw = & ping.exe -n \$count \$target 2>&1
\$text = \$raw | Out-String
\$sent = \$count
\$received = 0
\$lost = \$count
\$loss = 100.0
\$avgMs = \$null
if (\$text -match 'Sent\\s*=\\s*(\\d+),\\s*Received\\s*=\\s*(\\d+),\\s*Lost\\s*=\\s*(\\d+)\\s*\\((\\d+)%\\s*loss\\)') {
  \$sent = [int]\$Matches[1]
  \$received = [int]\$Matches[2]
  \$lost = [int]\$Matches[3]
  \$loss = [double]\$Matches[4]
}
if (\$text -match 'Average\\s*=\\s*(\\d+)ms') {
  \$avgMs = [double]\$Matches[1]
}
[pscustomobject]@{
  target = \$target
  count = \$count
  sent = \$sent
  received = \$received
  lost = \$lost
  loss_percent = \$loss
  avg_ms = \$avgMs
  raw = \$text
} | ConvertTo-Json -Depth 6" >"$out"
}

capture_linux_ping() {
  local target="$1"
  local count="$2"
  local out="$3"
  run_linux_sh "set +e
target=$(sh_q "$target")
count=$(sh_q "$count")
raw=\$(ping -c \"\$count\" -W 1 \"\$target\" 2>&1)
stats=\$(printf '%s\n' \"\$raw\" | awk '/packets transmitted/ {print; exit}')
sent=\$(printf '%s\n' \"\$stats\" | awk '{print \$1 + 0}')
received=\$(printf '%s\n' \"\$stats\" | awk '{print \$4 + 0}')
loss=\$(printf '%s\n' \"\$stats\" | sed -n 's/.* \([0-9.][0-9.]*\)% packet loss.*/\1/p')
avg=\$(printf '%s\n' \"\$raw\" | awk -F'/' '/rtt min\\/avg\\/max/ {print \$5; exit}')
[[ -n \"\$sent\" && \"\$sent\" != 0 ]] || sent=\"\$count\"
[[ -n \"\$received\" ]] || received=0
[[ -n \"\$loss\" ]] || loss=100
jq -n \
  --arg target \"\$target\" \
  --arg count \"\$count\" \
  --arg sent \"\$sent\" \
  --arg received \"\$received\" \
  --arg loss \"\$loss\" \
  --arg avg \"\$avg\" \
  --arg raw \"\$raw\" \
  '{target:\$target,count:(\$count|tonumber),sent:(\$sent|tonumber),received:(\$received|tonumber),lost:((\$sent|tonumber)-(\$received|tonumber)),loss_percent:(\$loss|tonumber),avg_ms:(if \$avg == \"\" then null else (\$avg|tonumber) end),raw:\$raw}'" >"$out"
}

wait_for_tunnel_health() {
  local row="$1"
  local row_dir="$2"
  local windows_tunnel="$3"
  local linux_tunnel="$4"
  local health_dir="$row_dir/tunnel-health"
  mkdir -p "$health_dir"

  for attempt in $(seq 1 "$TUNNEL_HEALTH_ATTEMPTS"); do
    local attempt_dir="$health_dir/attempt-$attempt"
    local verdict="$attempt_dir/verdict.json"
    mkdir -p "$attempt_dir"

    if ! capture_windows_ping "$linux_tunnel" "$TUNNEL_PING_COUNT" "$attempt_dir/windows-to-linux.json"; then
      jq -n --arg target "$linux_tunnel" --arg error "windows ping command failed" \
        '{target:$target,count:0,sent:0,received:0,lost:0,loss_percent:100,avg_ms:null,error:$error}' \
        >"$attempt_dir/windows-to-linux.json"
    fi
    if ! capture_linux_ping "$windows_tunnel" "$TUNNEL_PING_COUNT" "$attempt_dir/linux-to-windows.json"; then
      jq -n --arg target "$windows_tunnel" --arg error "linux ping command failed" \
        '{target:$target,count:0,sent:0,received:0,lost:0,loss_percent:100,avg_ms:null,error:$error}' \
        >"$attempt_dir/linux-to-windows.json"
    fi

    jq -n \
      --slurpfile w2l "$attempt_dir/windows-to-linux.json" \
      --slurpfile l2w "$attempt_dir/linux-to-windows.json" \
      --arg row "$row" \
      --arg attempt "$attempt" \
      --arg ping_count "$TUNNEL_PING_COUNT" \
      --arg max_loss "$TUNNEL_MAX_LOSS_PERCENT" \
      '
      def num($v): ($v // 0 | tonumber);
      ($w2l[0] // {}) as $w |
      ($l2w[0] // {}) as $l |
      {
        row: $row,
        attempt: ($attempt | tonumber),
        ping_count: ($ping_count | tonumber),
        max_loss_percent: ($max_loss | tonumber),
        windows_to_linux: $w,
        linux_to_windows: $l,
        ok: (
          num($w.sent) >= ($ping_count | tonumber) and
          num($l.sent) >= ($ping_count | tonumber) and
          num($w.loss_percent) <= ($max_loss | tonumber) and
          num($l.loss_percent) <= ($max_loss | tonumber)
        )
      }' >"$verdict"

    cp "$verdict" "$row_dir/tunnel-health-verdict.json"
    if [[ "$(jq -r '.ok' "$verdict")" == "true" ]]; then
      return 0
    fi

    printf 'waiting for %s tunnel health (%s/%s): Windows->Linux received=%s loss=%s%%, Linux->Windows received=%s loss=%s%%\n' \
      "$row" "$attempt" "$TUNNEL_HEALTH_ATTEMPTS" \
      "$(jq -r '.windows_to_linux.received // 0' "$verdict")" \
      "$(jq -r '.windows_to_linux.loss_percent // 100' "$verdict")" \
      "$(jq -r '.linux_to_windows.received // 0' "$verdict")" \
      "$(jq -r '.linux_to_windows.loss_percent // 100' "$verdict")" >&2
    sleep "$TUNNEL_HEALTH_INTERVAL_SECS"
  done

  cat "$row_dir/tunnel-health-verdict.json" >&2 || true
  die "$row tunnel health did not become valid before nvpn measurement"
}

adapter_delta_jq='
  def adapter($obj; $name): (($obj.adapters // []) | map(select(.name == $name)) | first // {});
  def num($v): ($v // 0 | tonumber);
  {
    sent_bytes: (num(adapter($after[0]; $adapter).sent_bytes) - num(adapter($before[0]; $adapter).sent_bytes)),
    received_bytes: (num(adapter($after[0]; $adapter).received_bytes) - num(adapter($before[0]; $adapter).received_bytes)),
    sent_packets: (num(adapter($after[0]; $adapter).sent_unicast_packets) - num(adapter($before[0]; $adapter).sent_unicast_packets)),
    received_packets: (num(adapter($after[0]; $adapter).received_unicast_packets) - num(adapter($before[0]; $adapter).received_unicast_packets)),
    sent_discards: (num(adapter($after[0]; $adapter).sent_discards) - num(adapter($before[0]; $adapter).sent_discards)),
    received_discards: (num(adapter($after[0]; $adapter).received_discards) - num(adapter($before[0]; $adapter).received_discards)),
    sent_errors: (num(adapter($after[0]; $adapter).sent_errors) - num(adapter($before[0]; $adapter).sent_errors)),
    received_errors: (num(adapter($after[0]; $adapter).received_errors) - num(adapter($before[0]; $adapter).received_errors))
  }
'

summarize_direction() {
  local row="$1"
  local path_kind="$2"
  local direction="$3"
  local dir="$4"
  local client="$dir/client.jsonl"
  local server="$dir/server.jsonl"
  local before_win="$dir/windows-before.json"
  local after_win="$dir/windows-after.json"
  local before_linux="$dir/linux-before.json"
  local after_linux="$dir/linux-after.json"
  local summary="$dir/summary.json"
  local selected_pair="$OUTPUT_DIR/$row/selected-pair.json"
  local direct_capture_summary="$dir/fips-direct-capture/summary.json"

  jq -n \
    --arg row "$row" \
    --arg path_kind "$path_kind" \
    --arg direction "$direction" \
    --slurpfile client_json <(grep '"role":"client"' "$client" | tail -n 1) \
    --slurpfile server_json <(grep '"role":"server"' "$server" | tail -n 1) \
    --slurpfile before_win "$before_win" \
    --slurpfile after_win "$after_win" \
    --slurpfile before_linux "$before_linux" \
    --slurpfile after_linux "$after_linux" \
    --slurpfile selected_pair <(if [[ -f "$selected_pair" ]]; then cat "$selected_pair"; fi) \
    --slurpfile direct_capture <(if [[ -f "$direct_capture_summary" ]]; then cat "$direct_capture_summary"; fi) \
    --argjson win_nvpn_delta "$(jq -n --slurpfile before "$before_win" --slurpfile after "$after_win" --arg adapter nvpn "$adapter_delta_jq")" \
    --argjson win_eth_delta "$(jq -n --slurpfile before "$before_win" --slurpfile after "$after_win" --arg adapter Ethernet "$adapter_delta_jq")" \
    '
    def num($v): ($v // 0 | tonumber);
    $client_json[0] as $client |
    $server_json[0] as $server |
    $before_win[0] as $bw |
    $after_win[0] as $aw |
    $before_linux[0] as $bl |
    $after_linux[0] as $al |
    ($selected_pair[0] // {}) as $pair |
    ($direct_capture[0] // {}) as $direct |
    (num($aw.process.cpu_seconds) - num($bw.process.cpu_seconds)) as $win_cpu |
    (num($al.process.cpu_seconds) - num($bl.process.cpu_seconds)) as $linux_cpu |
    (num($client.bytes) / 1000000000.0) as $gb |
    {
      row: $row,
      path: $path_kind,
      direction: $direction,
      tool: "tcp_probe",
      client_mbps: num($client.mbps),
      server_mbps: num($server.mbps),
      client_bytes: num($client.bytes),
      server_bytes: num($server.bytes),
      client_seconds: num($client.seconds),
      server_seconds: num($server.seconds),
      windows_cpu_seconds: $win_cpu,
      linux_cpu_seconds: $linux_cpu,
      windows_cpu_sec_per_gb: (if $gb > 0 then $win_cpu / $gb else null end),
      linux_cpu_sec_per_gb: (if $gb > 0 then $linux_cpu / $gb else null end),
      windows_service_binary: ($aw.binaries.configured_path // ""),
      windows_service_hash: ($aw.binaries.configured_hash // ""),
      windows_process_hash: ($aw.binaries.process_hash // ""),
      windows_hash_match: (($aw.binaries.configured_hash // "") != "" and ($aw.binaries.configured_hash // "") == ($aw.binaries.process_hash // "")),
      linux_service_binary: ($al.binaries.configured_path // ""),
      linux_service_hash: ($al.binaries.configured_hash // ""),
      linux_process_hash: ($al.binaries.process_hash // ""),
      linux_hash_match: (($al.binaries.configured_hash // "") != "" and ($al.binaries.configured_hash // "") == ($al.binaries.process_hash // "")),
      direct_pair_ok: (if $path_kind == "nvpn" then ($pair.direct_pair.ok // null) else null end),
      direct_capture_ok: (if $path_kind == "nvpn" then ($direct.ok // null) else null end),
      primary_direct_payload_ratio: (if $path_kind == "nvpn" then ($direct.primary_direct_payload_ratio // null) else null end),
      direct_payload_ratio: (if $path_kind == "nvpn" then ($direct.direct_payload_ratio // null) else null end),
      non_direct_payload_ratio: (if $path_kind == "nvpn" then ($direct.non_direct_payload_ratio // null) else null end),
      non_direct_payload_bytes: (if $path_kind == "nvpn" then ($direct.non_direct_payload_bytes // null) else null end),
      primary_direct_flow: (if $path_kind == "nvpn" then ($direct.primary_direct_flow // "") else "" end),
      non_direct_peers: (if $path_kind == "nvpn" then ($direct.non_direct_peers // {}) else {} end),
      windows_nvpn_delta: $win_nvpn_delta,
      windows_ethernet_delta: $win_eth_delta,
      linux_process_path: ($al.process.path // "")
    }' >"$summary"

  if [[ ! -f "$SUMMARY_TSV" ]]; then
    printf 'row\tpath\tdirection\ttool\tclient_mbps\tserver_mbps\twindows_cpu_seconds\twindows_cpu_sec_per_gb\tlinux_cpu_seconds\tlinux_cpu_sec_per_gb\twindows_hash_match\twindows_service_binary\twindows_service_hash\twindows_process_hash\tlinux_hash_match\tlinux_service_binary\tlinux_service_hash\tlinux_process_hash\tdirect_pair_ok\tdirect_capture_ok\tprimary_direct_payload_ratio\tdirect_payload_ratio\tnon_direct_payload_ratio\tnon_direct_payload_bytes\tprimary_direct_flow\tnon_direct_peers\twindows_nvpn_rx_bytes\twindows_nvpn_tx_bytes\twindows_ethernet_rx_bytes\twindows_ethernet_tx_bytes\n' >"$SUMMARY_TSV"
  fi
  jq -r '[
    .row,
    .path,
    .direction,
    .tool,
    .client_mbps,
    .server_mbps,
    .windows_cpu_seconds,
    (.windows_cpu_sec_per_gb // ""),
    .linux_cpu_seconds,
    (.linux_cpu_sec_per_gb // ""),
    .windows_hash_match,
    .windows_service_binary,
    .windows_service_hash,
    .windows_process_hash,
    .linux_hash_match,
    .linux_service_binary,
    .linux_service_hash,
    .linux_process_hash,
    (.direct_pair_ok // ""),
    (.direct_capture_ok // ""),
    (.primary_direct_payload_ratio // ""),
    (.direct_payload_ratio // ""),
    (.non_direct_payload_ratio // ""),
    (.non_direct_payload_bytes // ""),
    .primary_direct_flow,
    (.non_direct_peers | tojson),
    .windows_nvpn_delta.received_bytes,
    .windows_nvpn_delta.sent_bytes,
    .windows_ethernet_delta.received_bytes,
    .windows_ethernet_delta.sent_bytes
  ] | @tsv' "$summary" >>"$SUMMARY_TSV"
}

measure_direction() {
  local row="$1"
  local path_kind="$2"
  local direction="$3"
  local server_side="$4"
  local connect_ip="$5"
  local out_dir="$6"
  local client_mode="${7:-send}"
  local label="${row}-${path_kind}-${direction}"
  local port
  port="$(next_port)"
  mkdir -p "$out_dir"

  local capture_info="" capture_pid="" capture_stdout="" capture_stderr="" capture_dir=""
  local windows_log_marker="$out_dir/windows-daemon-marker.json"
  if [[ "$path_kind" == "nvpn" ]] && is_true "$DIRECT_FIPS_CAPTURE"; then
    capture_dir="$out_dir/fips-direct-capture"
    mkdir -p "$capture_dir"
    capture_info="$(start_linux_fips_capture "$label")"
    printf '%s\n' "$capture_info" >"$capture_dir/info.json"
    capture_pid="$(jq -r '.pid // empty' "$capture_dir/info.json")"
    capture_stdout="$(jq -r '.stdout // empty' "$capture_dir/info.json")"
    capture_stderr="$(jq -r '.stderr // empty' "$capture_dir/info.json")"
  fi

  capture_windows_snapshot "$out_dir/windows-before.json"
  capture_linux_snapshot "$out_dir/linux-before.json"
  if [[ "$path_kind" == "nvpn" ]] && is_true "$WINDOWS_PIPELINE_TRACE"; then
    capture_windows_daemon_log_marker "$out_dir/windows-before.json" "$windows_log_marker"
  fi

  if [[ "$server_side" == "linux" ]]; then
    local remote_log
    remote_log="$(start_linux_probe_server "$port" "$label" | tail -n 1)"
    jq -n --arg side linux --arg stdout "$remote_log" --arg port "$port" \
      '{side:$side,stdout:$stdout,port:($port|tonumber)}' >"$out_dir/server-info.json"
    sleep 1
    run_windows_probe_client "$connect_ip" "$port" "$label" "$direction" "$client_mode" "$out_dir/client.jsonl"
    fetch_linux_file "$remote_log" "$out_dir/server.jsonl"
  else
    local server_info remote_log
    server_info="$(start_windows_probe_server "$port" "$label")"
    printf '%s\n' "$server_info" >"$out_dir/server-info.json"
    remote_log="$(jq -r '.stdout' <<<"$server_info")"
    sleep 1
    run_linux_probe_client "$connect_ip" "$port" "$label" "$direction" "$out_dir/client.jsonl"
    fetch_windows_file "$remote_log" "$out_dir/server.jsonl"
  fi

  if [[ -n "$capture_info" ]]; then
    stop_linux_fips_capture "$capture_pid"
    fetch_linux_file "$capture_stdout" "$capture_dir/stdout.txt"
    fetch_linux_file "$capture_stderr" "$capture_dir/stderr.txt"
    summarize_direct_fips_capture "$row" "$direction" "$capture_dir" "$out_dir/client.jsonl"
  fi

  capture_windows_snapshot "$out_dir/windows-after.json"
  capture_linux_snapshot "$out_dir/linux-after.json"
  if [[ "$path_kind" == "nvpn" ]] && is_true "$WINDOWS_PIPELINE_TRACE"; then
    capture_windows_daemon_log_delta "$windows_log_marker" "$out_dir/windows-daemon.log"
  fi
  summarize_direction "$row" "$path_kind" "$direction" "$out_dir"
}

validate_underlay() {
  local row_dir="$1"
  local w2l="$row_dir/underlay/windows_to_linux/summary.json"
  local l2w="$row_dir/underlay/linux_to_windows/summary.json"
  local verdict="$row_dir/underlay-verdict.json"
  jq -n \
    --slurpfile w2l "$w2l" \
    --slurpfile l2w "$l2w" \
    --arg min_mbps "$MIN_UNDERLAY_MBPS" \
    --arg max_ratio "$MAX_UNDERLAY_RATIO" \
    '
    def num($v): ($v // 0 | tonumber);
    (num($w2l[0].client_mbps)) as $a |
    (num($l2w[0].client_mbps)) as $b |
    ([($a), ($b)] | min) as $min |
    ([($a), ($b)] | max) as $max |
    (if $min > 0 then $max / $min else 999999 end) as $ratio |
    {
      windows_to_linux_mbps: $a,
      linux_to_windows_mbps: $b,
      min_required_mbps: ($min_mbps | tonumber),
      max_allowed_ratio: ($max_ratio | tonumber),
      ratio: $ratio,
      ok: ($min >= ($min_mbps | tonumber) and $ratio <= ($max_ratio | tonumber))
    }' >"$verdict"
  if [[ "$(jq -r '.ok' "$verdict")" != "true" ]]; then
    cat "$verdict" >&2
    die "underlay probe is not valid enough for nvpn measurement"
  fi
}

measure_row() {
  local row="$1"
  local expected_exe="$2"
  local expected_linux_exe="${3:-}"
  local row_dir="$OUTPUT_DIR/$row"
  local expected_hash
  mkdir -p "$row_dir"

  expected_hash="$(windows_hash "$expected_exe" | tr -d '\r\n')"
  ensure_windows_nvpn_firewall_rule "$expected_exe" "$row" "$expected_hash"
  if [[ -n "$expected_linux_exe" ]]; then
    local expected_linux_hash
    expected_linux_hash="$(linux_hash "$expected_linux_exe" | tr -d '\r\n')"
    switch_linux_service "$expected_linux_exe" "$row"
    wait_for_linux_hash "$expected_linux_hash" "$row"
  fi
  local current_snapshot current_running current_configured_hash current_process_hash
  current_snapshot="$(mktemp)"
  capture_windows_snapshot "$current_snapshot"
  current_running="$(jq -r '.service_status.running // false' "$current_snapshot")"
  current_configured_hash="$(jq -r '.binaries.configured_hash // empty' "$current_snapshot")"
  current_process_hash="$(jq -r '.binaries.process_hash // empty' "$current_snapshot")"
  rm -f "$current_snapshot"
  local force_windows_switch=0
  if is_true "$WINDOWS_PIPELINE_TRACE" && [[ "$row" != "installed" ]]; then
    force_windows_switch=1
  fi
  if [[ "$force_windows_switch" == "0" && "$current_running" == "true" && "$current_configured_hash" == "$expected_hash" && "$current_process_hash" == "$expected_hash" ]]; then
    printf 'Windows service already running expected %s image\n' "$row" >&2
  else
    switch_windows_service "$expected_exe" "$row"
  fi
  wait_for_windows_hash "$expected_hash" "$row"
  wait_for_direct_pair "$row"
  select_pair "$row_dir"

  local windows_underlay linux_underlay windows_tunnel linux_tunnel
  windows_underlay="$(jq -r '.windows_underlay' "$row_dir/selected-pair.json")"
  linux_underlay="$(jq -r '.linux_underlay' "$row_dir/selected-pair.json")"
  windows_tunnel="$(jq -r '.windows_tunnel' "$row_dir/selected-pair.json")"
  linux_tunnel="$(jq -r '.linux_tunnel' "$row_dir/selected-pair.json")"

  printf 'row %s: underlay Windows->Linux %s, Linux->Windows %s; tunnel Windows->Linux %s, Linux->Windows %s\n' \
    "$row" "$linux_underlay" "$windows_underlay" "$linux_tunnel" "$windows_tunnel" >&2

  measure_direction "$row" underlay windows_to_linux linux "$linux_underlay" "$row_dir/underlay/windows_to_linux" send
  measure_direction "$row" underlay linux_to_windows linux "$linux_underlay" "$row_dir/underlay/linux_to_windows" recv
  validate_underlay "$row_dir"
  wait_for_tunnel_health "$row" "$row_dir" "$windows_tunnel" "$linux_tunnel"

  measure_direction "$row" nvpn windows_to_linux linux "$linux_tunnel" "$row_dir/nvpn/windows_to_linux" send
  measure_direction "$row" nvpn linux_to_windows linux "$linux_tunnel" "$row_dir/nvpn/linux_to_windows" recv
}

git_meta_value() {
  local repo="$1"
  local kind="$2"
  if [[ -z "$repo" ]] || ! git -C "$repo" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    return 0
  fi
  case "$kind" in
    head)
      git -C "$repo" rev-parse HEAD
      ;;
    tree)
      git -C "$repo" rev-parse 'HEAD^{tree}'
      ;;
    dirty)
      if [[ -n "$(git -C "$repo" status --short)" ]]; then
        printf true
      else
        printf false
      fi
      ;;
  esac
}

write_run_metadata() {
  local current_fips_head current_fips_tree current_fips_dirty
  current_fips_head="$(git_meta_value "$CURRENT_FIPS_REPO" head)"
  current_fips_tree="$(git_meta_value "$CURRENT_FIPS_REPO" tree)"
  current_fips_dirty="$(git_meta_value "$CURRENT_FIPS_REPO" dirty)"
  jq -n \
    --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg root "$ROOT_DIR" \
    --arg git_head "$(git -C "$ROOT_DIR" rev-parse HEAD)" \
    --arg rows "$ROWS" \
    --arg windows_ssh "$WINDOWS_SSH" \
    --arg linux_ssh "$LINUX_SSH" \
    --arg current_windows_nvpn "$CURRENT_WINDOWS_NVPN" \
    --arg current_linux_nvpn "$CURRENT_LINUX_NVPN" \
    --arg current_fips_repo "$CURRENT_FIPS_REPO" \
    --arg current_fips_head "$current_fips_head" \
    --arg current_fips_tree "$current_fips_tree" \
    --arg current_fips_dirty "$current_fips_dirty" \
    --arg windows_fips_repo "$WINDOWS_FIPS_REPO" \
    --arg linux_fips_repo "$LINUX_FIPS_REPO" \
    --arg duration_secs "$DURATION_SECS" \
    --arg min_underlay_mbps "$MIN_UNDERLAY_MBPS" \
    --arg max_underlay_ratio "$MAX_UNDERLAY_RATIO" \
    --arg tunnel_health_attempts "$TUNNEL_HEALTH_ATTEMPTS" \
    --arg tunnel_health_interval_secs "$TUNNEL_HEALTH_INTERVAL_SECS" \
    --arg tunnel_ping_count "$TUNNEL_PING_COUNT" \
    --arg tunnel_max_loss_percent "$TUNNEL_MAX_LOSS_PERCENT" \
    --arg direct_fips_capture "$DIRECT_FIPS_CAPTURE" \
    --arg linux_capture_iface "$LINUX_CAPTURE_IFACE" \
    --arg windows_pipeline_trace "$WINDOWS_PIPELINE_TRACE" \
    --arg windows_pipeline_interval_secs "$WINDOWS_PIPELINE_INTERVAL_SECS" \
    --arg windows_daemon_log_tail_lines "$WINDOWS_DAEMON_LOG_TAIL_LINES" \
    --arg mesh_mtu_profile "$MESH_MTU_PROFILE" \
    --arg mesh_underlay_udp_mtu "$MESH_UNDERLAY_UDP_MTU" \
    --arg mesh_tunnel_mtu "$MESH_TUNNEL_MTU" \
    --arg linux_installed_expected_hash "$LINUX_INSTALLED_EXPECTED_HASH" \
    --arg allow_current_linux_as_installed "$(is_true "$ALLOW_CURRENT_LINUX_AS_INSTALLED" && printf true || printf false)" \
    --arg windows_ssh_proxy_command "$([[ -n "$WINDOWS_SSH_PROXY_COMMAND" ]] && printf true || printf false)" \
    --arg linux_ssh_proxy_command "$([[ -n "$LINUX_SSH_PROXY_COMMAND" ]] && printf true || printf false)" \
    --arg windows_ssh_jump "$([[ -n "$WINDOWS_SSH_JUMP" ]] && printf true || printf false)" \
    --arg linux_ssh_jump "$([[ -n "$LINUX_SSH_JUMP" ]] && printf true || printf false)" \
    --arg output_dir "$OUTPUT_DIR" \
    '{
      created_at:$created_at,
      repo:$root,
      git_head:$git_head,
      rows:$rows,
      windows_ssh:$windows_ssh,
      linux_ssh:$linux_ssh,
      current_windows_nvpn:$current_windows_nvpn,
      current_linux_nvpn:$current_linux_nvpn,
      current_fips_repo:$current_fips_repo,
      current_fips_head:$current_fips_head,
      current_fips_tree:$current_fips_tree,
      current_fips_dirty:(if $current_fips_dirty == "" then null else ($current_fips_dirty == "true") end),
      windows_fips_repo:$windows_fips_repo,
      linux_fips_repo:$linux_fips_repo,
      duration_secs:($duration_secs|tonumber),
      min_underlay_mbps:($min_underlay_mbps|tonumber),
      max_underlay_ratio:($max_underlay_ratio|tonumber),
      tunnel_health_attempts:($tunnel_health_attempts|tonumber),
      tunnel_health_interval_secs:($tunnel_health_interval_secs|tonumber),
      tunnel_ping_count:($tunnel_ping_count|tonumber),
      tunnel_max_loss_percent:($tunnel_max_loss_percent|tonumber),
      direct_fips_capture:$direct_fips_capture,
      linux_capture_iface:$linux_capture_iface,
      windows_pipeline_trace:$windows_pipeline_trace,
      windows_pipeline_interval_secs:($windows_pipeline_interval_secs|tonumber),
      windows_daemon_log_tail_lines:($windows_daemon_log_tail_lines|tonumber),
      mesh_mtu_profile:$mesh_mtu_profile,
      mesh_underlay_udp_mtu:$mesh_underlay_udp_mtu,
      mesh_tunnel_mtu:$mesh_tunnel_mtu,
      linux_installed_expected_hash:$linux_installed_expected_hash,
      allow_current_linux_as_installed:($allow_current_linux_as_installed == "true"),
      windows_ssh_proxy_command:($windows_ssh_proxy_command == "true"),
      linux_ssh_proxy_command:($linux_ssh_proxy_command == "true"),
      windows_ssh_jump:($windows_ssh_jump == "true"),
      linux_ssh_jump:($linux_ssh_jump == "true"),
      output_dir:$output_dir
    }' \
    >"$RUN_JSON"
}

main() {
  if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    usage
    exit 0
  fi

  require_inputs
  validate_service_env
  need_cmd ssh
  need_cmd jq
  need_cmd iconv
  need_cmd base64
  if is_true "$DIRECT_FIPS_CAPTURE"; then
    need_cmd python3
  fi

  mkdir -p "$OUTPUT_DIR"
  write_run_metadata
  resolve_installed_windows_nvpn
  resolve_installed_linux_nvpn
  trap cleanup EXIT
  backup_windows_config
  ensure_probe_binaries
  if row_contains_current && [[ -n "$CURRENT_LINUX_NVPN" ]]; then
    backup_linux_installed_binary
  fi

  if row_contains_current; then
    build_current_windows
  fi

  if row_contains_installed; then
    measure_row installed "$WINDOWS_INSTALLED_NVPN"
  fi
  if row_contains_current; then
    measure_row current "$CURRENT_WINDOWS_NVPN" "$CURRENT_LINUX_NVPN"
  fi

  printf 'windows-linux nvpn bench wrote %s\n' "$OUTPUT_DIR"
  printf 'summary: %s\n' "$SUMMARY_TSV"
}

main "$@"
