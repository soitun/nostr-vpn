fn local_fips_endpoint_hints(
    app: &AppConfig,
    local_ipv4_candidates: Vec<Ipv4Addr>,
    local_advertised_endpoints: &[OverlayEndpointAdvert],
) -> Vec<PeerEndpointHint> {
    let mut endpoints = Vec::new();

    let configured = endpoint_with_listen_port(&app.node.endpoint, app.node.listen_port);
    if endpoint_is_gossipable_direct_hint(&configured, true)
        && !endpoint_uses_tunnel_ip(&configured, &app.node.tunnel_ip)
    {
        endpoints.push(configured);
    }

    if app.lan_discovery_enabled {
        for ip in local_ipv4_candidates {
            if !ipv4_is_lan_endpoint_hint(ip) {
                continue;
            }
            endpoints.push(SocketAddrV4::new(ip, app.node.listen_port).to_string());
        }
    }

    for endpoint in local_advertised_endpoints {
        if let Some(addr) = local_advertised_udp_endpoint_hint_addr(endpoint) {
            endpoints.push(addr);
        }
    }

    endpoints.sort();
    endpoints.dedup();
    endpoints.into_iter().map(PeerEndpointHint::udp).collect()
}
