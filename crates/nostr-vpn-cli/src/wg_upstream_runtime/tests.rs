#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_windows_default_route_from_route_print() {
        // Synthetic `route print -4 0.0.0.0` output. Only the
        // 0.0.0.0/0.0.0.0 row matters; all other content is meant to
        // be skipped by the parser.
        let sample = "\
===========================================================================
Interface List
 23...00 ff a1 b2 c3 d4 ......WireGuard Tunnel
 12...c0 d4 fe ff aa bb ......Realtek PCIe GbE
===========================================================================

IPv4 Route Table
===========================================================================
Active Routes:
Network Destination        Netmask          Gateway       Interface  Metric
          0.0.0.0          0.0.0.0      192.168.1.1     192.168.1.42     25
        127.0.0.0        255.0.0.0         On-link         127.0.0.1    331
===========================================================================
";
        let parsed = parse_windows_default_route_columns(sample).expect("default route parsed");
        assert_eq!(parsed.gateway, "192.168.1.1");
        assert_eq!(parsed.interface_ip, "192.168.1.42");
        assert_eq!(parsed.metric, 25);
    }

    #[test]
    fn skips_on_link_default_routes() {
        let sample = "\
Active Routes:
Network Destination        Netmask          Gateway       Interface  Metric
          0.0.0.0          0.0.0.0         On-link        10.0.0.1     50
          0.0.0.0          0.0.0.0      192.168.1.1   192.168.1.42     25
";
        let parsed =
            parse_windows_default_route_columns(sample).expect("non-On-link default parsed");
        assert_eq!(parsed.gateway, "192.168.1.1");
        assert_eq!(parsed.interface_ip, "192.168.1.42");
        assert_eq!(parsed.metric, 25);
    }

    #[test]
    fn chooses_lowest_metric_windows_default_route() {
        let sample = "\
Active Routes:
Network Destination        Netmask          Gateway       Interface  Metric
          0.0.0.0          0.0.0.0      172.20.0.1    172.20.0.22     75
          0.0.0.0          0.0.0.0      192.168.1.1   192.168.1.42     25
";
        let parsed = parse_windows_default_route_columns(sample).expect("default route parsed");
        assert_eq!(parsed.gateway, "192.168.1.1");
        assert_eq!(parsed.interface_ip, "192.168.1.42");
        assert_eq!(parsed.metric, 25);
    }

    #[test]
    fn returns_none_when_no_default_route_present() {
        let sample = "Active Routes:\n      127.0.0.0  255.0.0.0  On-link  127.0.0.1  331\n";
        assert!(parse_windows_default_route_columns(sample).is_none());
    }

    #[test]
    fn parses_windows_ipaddress_alias_from_verbose_netsh() {
        let sample = "\
Address 127.0.0.1 Parameters
---------------------------------------------------------
Interface Luid     : Loopback Pseudo-Interface 1

Address 192.0.2.147 Parameters
---------------------------------------------------------
Interface Luid     : Ethernet
Scope Id           : 0.0
";
        assert_eq!(
            parse_windows_ipaddresses_interface(sample, "192.0.2.147".parse().expect("ip")),
            Some(WindowsAddressInterface::Alias("Ethernet".to_string()))
        );
    }

    #[test]
    fn parses_windows_interface_index_for_alias() {
        let sample = "\
Idx     Met         MTU          State                Name
---  ----------  ----------  ------------  ---------------------------
  1          75  4294967295  connected     Loopback Pseudo-Interface 1
  3          25        1500  connected     Ethernet
 11           5        1150  connected     nvpn
";
        assert_eq!(
            parse_windows_interface_index_for_alias(sample, "Ethernet"),
            Some(3)
        );
        assert_eq!(
            parse_windows_interface_index_for_alias(sample, "Loopback Pseudo-Interface 1"),
            Some(1)
        );
    }

    #[test]
    fn parses_windows_wireguard_latest_handshake_output() {
        assert!(!parse_windows_wireguard_latest_handshakes("abc\t0\n"));
        assert!(parse_windows_wireguard_latest_handshakes(
            "abc\t1778720702\n"
        ));
        assert!(parse_windows_wireguard_latest_handshakes_for_tunnel(
            "nvpn-wg-exit\tabc\t1778720702\n",
            "nvpn-wg-exit"
        ));
        assert!(!parse_windows_wireguard_latest_handshakes_for_tunnel(
            "other\tabc\t1778720702\n",
            "nvpn-wg-exit"
        ));
        assert!(
            parse_windows_wireguard_latest_handshakes_for_single_active_tunnel(
                "nvpn-wg-exit\tabc\t1778720702\n"
            )
        );
        assert!(
            !parse_windows_wireguard_latest_handshakes_for_single_active_tunnel(
                "nvpn-wg-exit\tabc\t1778720702\nother\tdef\t1778720703\n"
            )
        );
        assert!(parse_windows_wireguard_show_handshake(
            "interface: nvpn-wg-exit\n  public key: abc\npeer: def\n  latest handshake: 7 seconds ago\n",
            "nvpn-wg-exit"
        ));
        assert!(!parse_windows_wireguard_show_handshake(
            "interface: other\n  public key: abc\npeer: def\n  latest handshake: 7 seconds ago\n",
            "nvpn-wg-exit"
        ));
        assert!(!parse_windows_wireguard_show_handshake(
            "interface: nvpn-wg-exit\npeer: def\n  latest handshake: never\n",
            "nvpn-wg-exit"
        ));
    }

    #[test]
    fn sanitizes_windows_native_wireguard_tunnel_name() {
        let config = WireGuardExitConfig {
            interface: " nvpn wg/exit ".to_string(),
            ..WireGuardExitConfig::default()
        };
        assert_eq!(
            windows_native_wireguard_tunnel_name(&config),
            "nvpn-wg-exit"
        );
    }
}
