#[cfg(target_os = "linux")]
fn linux_route_targets_require_ip_endpoint_bypass(
    route_targets: &[String],
) -> bool {
    crate::route_targets_require_endpoint_bypass(route_targets)
}

#[cfg(any(target_os = "linux", test))]
fn linux_endpoint_bypass_hosts_unchanged(
    current_targets: &[String],
    desired_hosts: &[Ipv4Addr],
) -> bool {
    let mut current_targets = current_targets.to_vec();
    current_targets.sort_unstable();
    current_targets.dedup();

    let mut desired_targets = desired_hosts
        .iter()
        .map(|host| format!("{host}/32"))
        .collect::<Vec<_>>();
    desired_targets.sort_unstable();
    desired_targets.dedup();

    current_targets == desired_targets
}

impl FipsPrivateTunnelRuntime {
    #[cfg(target_os = "linux")]
    async fn apply_linux_network_state(&mut self, config: &FipsPrivateTunnelConfig) -> Result<()> {
        let mut route_targets = config.route_targets.clone();
        let requested_ipv4_exit = route_targets.iter().any(|route| route == "0.0.0.0/0");
        let requested_ipv6_exit = route_targets.iter().any(|route| route == "::/0");
        let requested_exit = requested_ipv4_exit || requested_ipv6_exit;
        let strict_exit = config.exit_node_leak_protection && requested_exit;
        let original_route_targets_require_bypass =
            linux_route_targets_require_ip_endpoint_bypass(&route_targets);
        let mut peer_endpoint_hosts = Vec::new();
        if original_route_targets_require_bypass {
            peer_endpoint_hosts = self.endpoint_bypass_ipv4_hosts(config).await?;
            if route_targets.iter().any(|route| route == "0.0.0.0/0")
                && peer_endpoint_hosts.is_empty()
            {
                eprintln!(
                    "fips: withholding default route until the selected exit peer underlay endpoint is known"
                );
                route_targets.retain(|route| !crate::is_exit_node_route(route));
            }
        }

        let active_ipv4_exit = route_targets.iter().any(|route| route == "0.0.0.0/0");
        let active_ipv6_exit = route_targets.iter().any(|route| route == "::/0");

        if requested_ipv4_exit {
            self.capture_linux_original_default_route();
        } else {
            self.restore_linux_original_default_route();
        }
        if requested_ipv6_exit {
            self.capture_linux_original_default_ipv6_route();
        } else {
            self.restore_linux_original_default_ipv6_route();
        }
        if !strict_exit {
            if requested_ipv4_exit && !active_ipv4_exit {
                self.restore_linux_original_default_route();
            }
            if requested_ipv6_exit && !active_ipv6_exit {
                self.restore_linux_original_default_ipv6_route();
            }
        }

        let endpoint_bypass_specs = if original_route_targets_require_bypass || strict_exit {
            let mut bypass_hosts = config.control_plane_bypass_hosts.clone();
            bypass_hosts.extend(peer_endpoint_hosts);
            bypass_hosts.sort_unstable();
            bypass_hosts.dedup();
            crate::linux_bypass_route_specs_for_hosts(
                bypass_hosts,
                &self.iface,
                self.original_default_route.as_deref(),
            )?
        } else {
            Vec::new()
        };
        self.reconcile_linux_endpoint_bypass_routes(&endpoint_bypass_specs);

        let mut interface_route_targets = route_targets.clone();
        interface_route_targets.sort();
        interface_route_targets.dedup();
        crate::apply_local_interface_network_with_mtu_and_addresses(
            &self.iface,
            &config.interface_addresses(),
            &interface_route_targets,
            config.mesh_mtu.tunnel,
        )
        .with_context(|| format!("failed to configure FIPS tunnel interface {}", self.iface))?;
        apply_linux_tun_tx_queue_len(&self.iface)?;
        if let Err(error) = crate::flush_linux_route_cache() {
            eprintln!("fips: failed to flush linux route cache: {error}");
        }
        if strict_exit {
            if requested_ipv4_exit && !active_ipv4_exit {
                self.block_linux_original_default_route();
            }
            if requested_ipv6_exit && !active_ipv6_exit {
                self.block_linux_original_default_ipv6_route();
            }
        }
        self.reconcile_linux_exit_node_forwarding(
            &config.local_address,
            &config.local_exit_forwarding_routes,
            &config.wireguard_exit,
            config.exit_node_leak_protection,
            config.mesh_mtu.tunnel,
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn capture_linux_original_default_route(&mut self) {
        if self.original_default_route.is_some() {
            return;
        }
        match crate::linux_default_route() {
            Ok(route) if route.dev != self.iface => self.original_default_route = Some(route.line),
            Ok(_) => {}
            Err(error) => eprintln!("fips: failed to capture original default route: {error}"),
        }
    }

    #[cfg(target_os = "linux")]
    fn capture_linux_original_default_ipv6_route(&mut self) {
        if self.original_default_ipv6_route.is_some() {
            return;
        }
        match crate::linux_default_ipv6_route() {
            Ok(route) if route.dev != self.iface => {
                self.original_default_ipv6_route = Some(route.line);
            }
            Ok(_) => {}
            Err(error) => eprintln!("fips: failed to capture original IPv6 default route: {error}"),
        }
    }

    #[cfg(target_os = "linux")]
    fn restore_linux_original_default_route(&mut self) {
        let Some(route) = self.original_default_route.take() else {
            return;
        };
        if let Err(error) = crate::restore_linux_default_route(&route) {
            eprintln!("fips: failed to restore original default route: {error}");
            self.original_default_route = Some(route);
        }
    }

    #[cfg(target_os = "linux")]
    fn restore_linux_original_default_ipv6_route(&mut self) {
        let Some(route) = self.original_default_ipv6_route.take() else {
            return;
        };
        if let Err(error) = crate::restore_linux_default_ipv6_route(&route) {
            eprintln!("fips: failed to restore original IPv6 default route: {error}");
            self.original_default_ipv6_route = Some(route);
        }
    }

    #[cfg(target_os = "linux")]
    fn block_linux_original_default_route(&mut self) {
        match crate::linux_default_route() {
            Ok(route) if Some(route.line.as_str()) == self.original_default_route.as_deref() => {
                if let Err(error) = crate::delete_linux_default_route() {
                    eprintln!("fips: failed to block IPv4 default route: {error}");
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    #[cfg(target_os = "linux")]
    fn block_linux_original_default_ipv6_route(&mut self) {
        match crate::linux_default_ipv6_route() {
            Ok(route)
                if Some(route.line.as_str()) == self.original_default_ipv6_route.as_deref() =>
            {
                if let Err(error) = crate::delete_linux_default_ipv6_route() {
                    eprintln!("fips: failed to block IPv6 default route: {error}");
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    #[cfg(target_os = "linux")]
    fn reconcile_linux_endpoint_bypass_routes(
        &mut self,
        routes: &[crate::LinuxEndpointBypassRoute],
    ) {
        let desired = routes
            .iter()
            .map(|route| route.target.clone())
            .collect::<std::collections::HashSet<_>>();

        let stale = self
            .endpoint_bypass_routes
            .iter()
            .filter(|route| !desired.contains(*route))
            .cloned()
            .collect::<Vec<_>>();
        for route in stale {
            if let Err(error) = crate::delete_linux_endpoint_bypass_route(&route) {
                eprintln!("fips: failed to remove endpoint bypass route {route}: {error}");
            }
        }

        for route in routes {
            if let Err(error) = crate::apply_linux_endpoint_bypass_route(route) {
                eprintln!(
                    "fips: failed to install endpoint bypass route {}: {}",
                    route.target, error
                );
            }
        }

        self.endpoint_bypass_routes = desired.into_iter().collect();
        self.endpoint_bypass_routes.sort();
    }

    #[cfg(target_os = "linux")]
    fn reconcile_linux_exit_node_forwarding(
        &mut self,
        local_address: &str,
        routes: &[String],
        wireguard_exit: &WireGuardExitConfig,
        exit_node_leak_protection: bool,
        tunnel_mtu: u16,
    ) {
        let ipv4_mss_clamp = exit_node_ipv4_mss_clamp(tunnel_mtu);
        let mut route_families = crate::linux_exit_node_default_route_families(routes);
        if route_families.ipv6 {
            eprintln!(
                "fips: IPv6 exit-node forwarding is disabled until nvpn has IPv6 mesh source filtering"
            );
            route_families.ipv6 = false;
        }
        // WG upstream as this host's own egress does not imply mesh
        // exit-node forwarding. Only advertised default routes should
        // turn on ip_forward/NAT below.
        let needs_ipv4_tunnel_source = route_families.ipv4 || wireguard_exit.enabled;
        let ipv4_tunnel_source_cidr = if needs_ipv4_tunnel_source {
            let Some(tunnel_source_cidr) = crate::linux_exit_node_source_cidr(local_address) else {
                eprintln!("fips: invalid IPv4 tunnel address '{local_address}'");
                self.reconcile_linux_exit_node_forwarding_cleanup();
                return;
            };
            Some(tunnel_source_cidr)
        } else {
            None
        };

        let wireguard_exit_iface = if wireguard_exit.enabled {
            let Some(source_cidr) = ipv4_tunnel_source_cidr.as_deref() else {
                self.reconcile_linux_exit_node_forwarding_cleanup();
                return;
            };
            match crate::validate_linux_wireguard_exit_config(wireguard_exit) {
                Ok(iface) => {
                    if !crate::linux_wireguard_exit_ipv6_default(wireguard_exit) {
                        route_families.ipv6 = false;
                    }
                    if let Err(error) =
                        self.apply_linux_wireguard_exit_upstream(wireguard_exit, source_cidr)
                    {
                        eprintln!("fips: failed to configure WireGuard exit upstream: {error}");
                        self.cleanup_linux_exit_node_forwarding_rules();
                        self.cleanup_linux_wireguard_exit_upstream();
                        self.block_linux_wireguard_exit_if_strict(exit_node_leak_protection);
                        return;
                    }
                    Some((iface, source_cidr.to_string()))
                }
                Err(error) => {
                    eprintln!("fips: WireGuard exit upstream is not ready: {error}");
                    self.cleanup_linux_exit_node_forwarding_rules();
                    self.cleanup_linux_wireguard_exit_upstream();
                    self.block_linux_wireguard_exit_if_strict(
                        exit_node_leak_protection && wireguard_exit.enabled,
                    );
                    return;
                }
            }
        } else {
            self.cleanup_linux_wireguard_exit_upstream();
            None
        };

        if !route_families.ipv4 && !route_families.ipv6 {
            self.cleanup_linux_exit_node_forwarding_rules();
            return;
        }

        let ipv4_outbound_iface = if route_families.ipv4 {
            if let Some((iface, _)) = wireguard_exit_iface.as_ref() {
                Some(iface.clone())
            } else {
                match crate::linux_default_route() {
                    Ok(route) => Some(route.dev),
                    Err(error) => {
                        eprintln!("fips: failed to resolve default IPv4 route device: {error}");
                        self.cleanup_linux_exit_node_forwarding_rules();
                        return;
                    }
                }
            }
        } else {
            None
        };

        let ipv6_outbound_iface = None;

        if !route_families.ipv4 && !route_families.ipv6 {
            self.cleanup_linux_exit_node_forwarding_rules();
            return;
        }

        let already_configured = self.exit_node_runtime.ipv4_outbound_iface == ipv4_outbound_iface
            && self.exit_node_runtime.ipv6_outbound_iface == ipv6_outbound_iface
            && self.exit_node_runtime.ipv4_tunnel_source_cidr == ipv4_tunnel_source_cidr
            && self.exit_node_runtime.ipv4_mss_clamp == Some(ipv4_mss_clamp);
        if already_configured {
            return;
        }

        self.cleanup_linux_exit_node_forwarding_rules();

        self.exit_node_runtime.ipv4_outbound_iface = ipv4_outbound_iface.clone();
        self.exit_node_runtime.ipv6_outbound_iface = ipv6_outbound_iface.clone();
        self.exit_node_runtime.ipv4_tunnel_source_cidr = ipv4_tunnel_source_cidr.clone();
        self.exit_node_runtime.ipv4_mss_clamp = Some(ipv4_mss_clamp);

        if route_families.ipv4 {
            match crate::read_linux_ip_forward(crate::LinuxExitNodeIpFamily::V4) {
                Ok(previous) => {
                    self.exit_node_runtime.ipv4_forward_was_enabled = Some(previous);
                    if !previous
                        && let Err(error) =
                            crate::write_linux_ip_forward(crate::LinuxExitNodeIpFamily::V4, true)
                    {
                        eprintln!("fips: failed to enable IPv4 forwarding: {error}");
                        self.cleanup_linux_exit_node_forwarding_rules();
                        return;
                    }
                }
                Err(error) => {
                    eprintln!("fips: failed to read IPv4 forwarding state: {error}");
                    self.cleanup_linux_exit_node_forwarding_rules();
                    return;
                }
            }
        }

        if let (Some(outbound_iface), Some(tunnel_source_cidr)) = (
            ipv4_outbound_iface.as_deref(),
            ipv4_tunnel_source_cidr.as_deref(),
        ) {
            eprintln!(
                "fips: enabling IPv4 exit forwarding on {} via {} source {}",
                self.iface, outbound_iface, tunnel_source_cidr
            );
            self.cleanup_linux_legacy_exit_node_forwarding_rules();
            let forward_in = crate::linux_exit_node_forward_in_rule(
                &self.iface,
                outbound_iface,
                tunnel_source_cidr,
                crate::LinuxExitNodeIpFamily::V4,
            );
            let forward_out = crate::linux_exit_node_forward_out_rule(
                &self.iface,
                outbound_iface,
                crate::LinuxExitNodeIpFamily::V4,
            );
            let masquerade =
                crate::linux_exit_node_ipv4_masquerade_rule(outbound_iface, tunnel_source_cidr);
            let mss_clamp = crate::linux_exit_node_ipv4_mss_clamp_rule(
                &self.iface,
                outbound_iface,
                tunnel_source_cidr,
                ipv4_mss_clamp,
            );

            if let Err(error) = crate::linux_iptables_ensure_rule_at_front(
                crate::LinuxExitNodeIpFamily::V4,
                None,
                &forward_in,
            )
            .and_then(|()| {
                crate::linux_iptables_ensure_rule_at_front(
                    crate::LinuxExitNodeIpFamily::V4,
                    None,
                    &forward_out,
                )
            })
            .and_then(|()| {
                crate::linux_iptables_ensure_rule(
                    crate::LinuxExitNodeIpFamily::V4,
                    Some("nat"),
                    &masquerade,
                )
            })
            .and_then(|()| {
                crate::linux_iptables_ensure_rule_at_front(
                    crate::LinuxExitNodeIpFamily::V4,
                    Some("mangle"),
                    &mss_clamp,
                )
            }) {
                eprintln!("fips: failed to install IPv4 exit firewall rules: {error}");
                self.cleanup_linux_exit_node_forwarding_rules();
                return;
            }
        }

        self.cleanup_linux_legacy_exit_node_forwarding_rules();
    }

    #[cfg(target_os = "linux")]
    fn apply_linux_wireguard_exit_upstream(
        &mut self,
        config: &WireGuardExitConfig,
        source_cidr: &str,
    ) -> Result<()> {
        let mut preserve_created_interface = false;
        let mut previous_runtime = None;
        if let Some(runtime) = self.exit_node_runtime.wireguard_exit.as_ref()
            && (runtime.interface != config.interface || runtime.source_cidr != source_cidr)
        {
            self.cleanup_linux_wireguard_exit_upstream();
        } else if let Some(runtime) = self.exit_node_runtime.wireguard_exit.as_ref() {
            preserve_created_interface = runtime.created_interface;
            previous_runtime = Some(runtime.clone());
        }
        let mut runtime = crate::apply_linux_wireguard_exit_upstream(
            config,
            source_cidr,
            previous_runtime.as_ref(),
            self.original_default_route.as_deref(),
        )?;
        runtime.created_interface |= preserve_created_interface;
        if let Err(error) = self.ensure_linux_wireguard_exit_inbound_guard(&runtime) {
            crate::cleanup_linux_wireguard_exit_upstream(&runtime);
            return Err(error);
        }
        self.exit_node_runtime.wireguard_exit = Some(runtime);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn ensure_linux_wireguard_exit_inbound_guard(
        &self,
        runtime: &crate::LinuxWireGuardExitRuntime,
    ) -> Result<()> {
        let drop_inbound = crate::linux_wireguard_exit_inbound_drop_rule(
            &runtime.interface,
            &self.iface,
            &runtime.source_cidr,
        );
        crate::linux_iptables_ensure_rule_at_front(
            crate::LinuxExitNodeIpFamily::V4,
            None,
            &drop_inbound,
        )
    }

    #[cfg(target_os = "linux")]
    fn cleanup_linux_wireguard_exit_inbound_guard(
        &self,
        runtime: &crate::LinuxWireGuardExitRuntime,
    ) {
        let drop_inbound = crate::linux_wireguard_exit_inbound_drop_rule(
            &runtime.interface,
            &self.iface,
            &runtime.source_cidr,
        );
        if let Err(error) =
            crate::linux_iptables_delete_rule(crate::LinuxExitNodeIpFamily::V4, None, &drop_inbound)
        {
            eprintln!("fips: failed to remove WireGuard inbound guard rule: {error}");
        }
    }

    #[cfg(target_os = "linux")]
    fn block_linux_wireguard_exit_if_strict(&mut self, enabled: bool) {
        if !enabled {
            return;
        }
        self.capture_linux_original_default_route();
        self.block_linux_original_default_route();
    }

    #[cfg(target_os = "linux")]
    fn cleanup_linux_wireguard_exit_upstream(&mut self) {
        let Some(runtime) = self.exit_node_runtime.wireguard_exit.take() else {
            return;
        };
        self.cleanup_linux_wireguard_exit_inbound_guard(&runtime);
        crate::cleanup_linux_wireguard_exit_upstream(&runtime);
    }

    #[cfg(target_os = "linux")]
    fn cleanup_linux_exit_node_forwarding_rules(&mut self) {
        if let (Some(outbound_iface), Some(tunnel_source_cidr)) = (
            self.exit_node_runtime.ipv4_outbound_iface.as_deref(),
            self.exit_node_runtime.ipv4_tunnel_source_cidr.as_deref(),
        ) {
            if let Some(mss) = self.exit_node_runtime.ipv4_mss_clamp {
                let mss_clamp = crate::linux_exit_node_ipv4_mss_clamp_rule(
                    &self.iface,
                    outbound_iface,
                    tunnel_source_cidr,
                    mss,
                );
                if let Err(error) = crate::linux_iptables_delete_rule(
                    crate::LinuxExitNodeIpFamily::V4,
                    Some("mangle"),
                    &mss_clamp,
                ) {
                    eprintln!("fips: failed to remove MSS clamp rule: {error}");
                }
            }
            let forward_in = crate::linux_exit_node_forward_in_rule(
                &self.iface,
                outbound_iface,
                tunnel_source_cidr,
                crate::LinuxExitNodeIpFamily::V4,
            );
            let forward_out = crate::linux_exit_node_forward_out_rule(
                &self.iface,
                outbound_iface,
                crate::LinuxExitNodeIpFamily::V4,
            );
            let masquerade =
                crate::linux_exit_node_ipv4_masquerade_rule(outbound_iface, tunnel_source_cidr);

            if let Err(error) = crate::linux_iptables_delete_rule(
                crate::LinuxExitNodeIpFamily::V4,
                Some("nat"),
                &masquerade,
            ) {
                eprintln!("fips: failed to remove masquerade rule: {error}");
            }
            if let Err(error) = crate::linux_iptables_delete_rule(
                crate::LinuxExitNodeIpFamily::V4,
                None,
                &forward_out,
            ) {
                eprintln!("fips: failed to remove forward-out rule: {error}");
            }
            if let Err(error) = crate::linux_iptables_delete_rule(
                crate::LinuxExitNodeIpFamily::V4,
                None,
                &forward_in,
            ) {
                eprintln!("fips: failed to remove forward-in rule: {error}");
            }
        }

        self.cleanup_linux_legacy_exit_node_forwarding_rules();

        if self.exit_node_runtime.ipv4_forward_was_enabled == Some(false)
            && let Err(error) =
                crate::write_linux_ip_forward(crate::LinuxExitNodeIpFamily::V4, false)
        {
            eprintln!("fips: failed to restore IPv4 forwarding state: {error}");
        }
        if self.exit_node_runtime.ipv6_forward_was_enabled == Some(false)
            && let Err(error) =
                crate::write_linux_ip_forward(crate::LinuxExitNodeIpFamily::V6, false)
        {
            eprintln!("fips: failed to restore IPv6 forwarding state: {error}");
        }

        self.exit_node_runtime.ipv4_outbound_iface = None;
        self.exit_node_runtime.ipv6_outbound_iface = None;
        self.exit_node_runtime.ipv4_tunnel_source_cidr = None;
        self.exit_node_runtime.ipv4_mss_clamp = None;
        self.exit_node_runtime.ipv4_forward_was_enabled = None;
        self.exit_node_runtime.ipv6_forward_was_enabled = None;
    }

    #[cfg(target_os = "linux")]
    fn cleanup_linux_legacy_exit_node_forwarding_rules(&self) {
        for family in [
            crate::LinuxExitNodeIpFamily::V4,
            crate::LinuxExitNodeIpFamily::V6,
        ] {
            let forward_in = crate::linux_exit_node_legacy_forward_in_rule(&self.iface, family);
            let forward_out = crate::linux_exit_node_legacy_forward_out_rule(&self.iface, family);
            let _ = crate::linux_iptables_delete_rule(family, None, &forward_out);
            let _ = crate::linux_iptables_delete_rule(family, None, &forward_in);
        }
    }

    #[cfg(target_os = "linux")]
    fn reconcile_linux_exit_node_forwarding_cleanup(&mut self) {
        self.cleanup_linux_exit_node_forwarding_rules();
        self.cleanup_linux_wireguard_exit_upstream();
        self.exit_node_runtime = crate::LinuxExitNodeRuntime::default();
    }

    #[cfg(target_os = "linux")]
    fn cleanup_linux_network_state(&mut self) {
        self.reconcile_linux_endpoint_bypass_routes(&[]);
        self.reconcile_linux_exit_node_forwarding_cleanup();
        self.restore_linux_original_default_route();
        self.restore_linux_original_default_ipv6_route();
        if let Err(error) = crate::flush_linux_route_cache() {
            eprintln!("fips: failed to flush linux route cache: {error}");
        }
    }
}

#[cfg(target_os = "linux")]
fn apply_linux_tun_tx_queue_len(iface: &str) -> Result<()> {
    let Some(queue_len) = linux_tun_tx_queue_len() else {
        return Ok(());
    };
    let queue_len = queue_len.to_string();
    crate::run_checked(
        ProcessCommand::new("ip")
            .arg("link")
            .arg("set")
            .arg("dev")
            .arg(iface)
            .arg("txqueuelen")
            .arg(&queue_len),
    )
    .with_context(|| format!("failed to set Linux tunnel txqueuelen on {iface}"))?;
    eprintln!("fips: Linux tunnel txqueuelen set on {iface}; txqueuelen={queue_len}");
    Ok(())
}
