use anyhow::{Context, Result};
use fips_core::Identity;
use fips_core::host_firewall::{HostFirewallConfig, HostFirewallGuard};
use nostr_vpn_core::config::AppConfig;
use std::net::Ipv6Addr;
#[cfg(target_os = "linux")]
use std::process::Command as ProcessCommand;

const LEGACY_FIPS_HOST_IFACE: &str = "nvpnfips0";
const HOST_FIREWALL_LINUX_TABLE_NAME: &str = "nvpn_fips_host";
const HOST_FIREWALL_MACOS_ANCHOR_NAME: &str = "com.apple/to.nostrvpn/fips-host";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FipsHostTunnelConfig {
    pub(crate) fips_address: Ipv6Addr,
    pub(crate) inbound_tcp_ports: Vec<u16>,
}

impl FipsHostTunnelConfig {
    pub(crate) fn from_app(app: &AppConfig) -> Result<Option<Self>> {
        if !app.fips_host_tunnel_enabled {
            return Ok(None);
        }
        if !HostFirewallGuard::platform_available() {
            eprintln!("fips-host: disabled because no FIPS host firewall is available");
            return Ok(None);
        }

        let identity = Identity::from_secret_str(&app.nostr.secret_key)
            .context("failed to derive .fips identity from nostr secret key")?;
        let mut inbound_tcp_ports = app.fips_host_inbound_tcp_ports.clone();
        inbound_tcp_ports.sort_unstable();
        inbound_tcp_ports.dedup();

        Ok(Some(Self {
            fips_address: identity.address().to_ipv6(),
            inbound_tcp_ports,
        }))
    }
}

pub(crate) struct FipsHostTunnelRuntime {
    interface: String,
    config: FipsHostTunnelConfig,
    firewall: Option<HostFirewallGuard>,
}

impl FipsHostTunnelRuntime {
    pub(crate) fn start(interface: &str, config: FipsHostTunnelConfig) -> Result<Self> {
        let firewall =
            HostFirewallGuard::install(&host_firewall_config(interface, &config.inbound_tcp_ports))
                .context("failed to install .fips host firewall")?;
        Ok(Self {
            interface: interface.to_string(),
            config,
            firewall: Some(firewall),
        })
    }

    pub(crate) fn requires_restart(&self, interface: &str, config: &FipsHostTunnelConfig) -> bool {
        self.interface != interface || self.config != *config
    }

    pub(crate) fn stop(mut self) {
        drop(self.firewall.take());
    }

    pub(crate) fn cleanup_disabled_artifacts() {
        HostFirewallGuard::cleanup_disabled_artifacts(&host_firewall_config(
            LEGACY_FIPS_HOST_IFACE,
            &[],
        ));
        cleanup_legacy_resolver_artifacts();
    }
}

fn host_firewall_config(iface: &str, inbound_tcp_ports: &[u16]) -> HostFirewallConfig {
    HostFirewallConfig::new(iface)
        .with_inbound_tcp_ports(inbound_tcp_ports.iter().copied())
        .with_linux_table_name(HOST_FIREWALL_LINUX_TABLE_NAME)
        .with_macos_anchor_name(HOST_FIREWALL_MACOS_ANCHOR_NAME)
}

fn cleanup_legacy_resolver_artifacts() {
    #[cfg(target_os = "macos")]
    {
        remove_legacy_owned_file("/etc/resolver/fips");
    }
    #[cfg(target_os = "linux")]
    {
        if remove_legacy_owned_file("/etc/systemd/resolved.conf.d/nostr-vpn-fips.conf") {
            restart_service("systemd-resolved");
        }
        if remove_legacy_owned_file("/etc/dnsmasq.d/nostr-vpn-fips.conf") {
            reload_service("dnsmasq");
        }
        if remove_legacy_owned_file("/etc/NetworkManager/dnsmasq.d/nostr-vpn-fips.conf") {
            reload_service("NetworkManager");
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn remove_legacy_owned_file(path: &str) -> bool {
    matches!(
        std::fs::read_to_string(path),
        Ok(contents) if contents.contains("Managed by nostr-vpn")
    ) && std::fs::remove_file(path).is_ok()
}

#[cfg(target_os = "linux")]
fn restart_service(service: &str) {
    let _ = ProcessCommand::new("systemctl")
        .arg("restart")
        .arg(service)
        .status();
}

#[cfg(target_os = "linux")]
fn reload_service(service: &str) {
    let _ = ProcessCommand::new("systemctl")
        .arg("reload")
        .arg(service)
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_config_builds_integrated_host_tunnel_config() {
        let mut app = AppConfig::generated();
        app.fips_host_tunnel_enabled = true;
        app.fips_host_inbound_tcp_ports = vec![443, 22, 22];

        let maybe_config = FipsHostTunnelConfig::from_app(&app).expect("valid fips host config");
        if !HostFirewallGuard::platform_available() {
            assert!(maybe_config.is_none());
            return;
        }
        let config = maybe_config.expect("enabled when host firewall is available");
        assert_eq!(config.inbound_tcp_ports, vec![22, 443]);
        assert_eq!(
            config.fips_address,
            Identity::from_secret_str(&app.nostr.secret_key)
                .expect("app identity")
                .address()
                .to_ipv6()
        );
    }

    #[test]
    fn app_config_can_disable_host_tunnel() {
        let mut app = AppConfig::generated();
        app.fips_host_tunnel_enabled = false;

        assert!(
            FipsHostTunnelConfig::from_app(&app)
                .expect("valid disabled config")
                .is_none()
        );
    }

    #[test]
    fn host_firewall_config_uses_nvpn_artifact_names() {
        let config = host_firewall_config("utun8", &[443, 22, 22]);

        assert_eq!(config.interface(), "utun8");
        assert_eq!(config.inbound_tcp_ports(), &[22, 443]);
        assert_eq!(config.linux_table_name(), HOST_FIREWALL_LINUX_TABLE_NAME);
        assert_eq!(config.macos_anchor_name(), HOST_FIREWALL_MACOS_ANCHOR_NAME);
    }
}
