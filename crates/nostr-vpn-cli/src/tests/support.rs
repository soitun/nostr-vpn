use std::net::Ipv4Addr;

pub(crate) fn macos_default_routes_from_netstat(output: &str) -> Vec<crate::MacosRouteSpec> {
    crate::macos_network::macos_default_routes_from_netstat(output)
}

pub(crate) fn macos_ifconfig_has_ipv4(output: &str, needle: Ipv4Addr) -> bool {
    crate::macos_network::macos_ifconfig_has_ipv4(output, needle)
}
