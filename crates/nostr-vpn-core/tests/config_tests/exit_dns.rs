use nostr_vpn_core::config::{
    CLOUDFLARE_DOH_URL, ExitDnsConfig, ExitDnsMode, ExitDnsResolverConfig, ExitDohProvider,
    QUAD9_DOH_URL, WireGuardExitConfig,
};

#[test]
fn exit_dns_supported_policy_matrix_selects_exact_resolver() {
    let wireguard = WireGuardExitConfig {
        dns: vec!["10.64.0.1".to_string()],
        ..WireGuardExitConfig::default()
    };

    let cases = [
        (
            "automatic profile DNS",
            ExitDnsConfig::default(),
            Some(&wireguard),
            ExitDnsResolverConfig::ThroughExit {
                servers: vec!["10.64.0.1".parse().unwrap()],
            },
        ),
        (
            "automatic encrypted fallback",
            ExitDnsConfig::default(),
            None,
            ExitDnsResolverConfig::Doh {
                url: CLOUDFLARE_DOH_URL.to_string(),
                bootstrap_ips: vec!["1.1.1.1".parse().unwrap(), "1.0.0.1".parse().unwrap()],
            },
        ),
        (
            "explicit Cloudflare",
            ExitDnsConfig {
                mode: ExitDnsMode::Encrypted,
                doh_provider: ExitDohProvider::Cloudflare,
                ..ExitDnsConfig::default()
            },
            Some(&wireguard),
            ExitDnsResolverConfig::Doh {
                url: CLOUDFLARE_DOH_URL.to_string(),
                bootstrap_ips: vec!["1.1.1.1".parse().unwrap(), "1.0.0.1".parse().unwrap()],
            },
        ),
        (
            "explicit Quad9",
            ExitDnsConfig {
                mode: ExitDnsMode::Encrypted,
                doh_provider: ExitDohProvider::Quad9,
                ..ExitDnsConfig::default()
            },
            Some(&wireguard),
            ExitDnsResolverConfig::Doh {
                url: QUAD9_DOH_URL.to_string(),
                bootstrap_ips: vec![
                    "9.9.9.9".parse().unwrap(),
                    "149.112.112.112".parse().unwrap(),
                ],
            },
        ),
        (
            "custom encrypted DNS",
            ExitDnsConfig {
                mode: ExitDnsMode::Encrypted,
                doh_provider: ExitDohProvider::Custom,
                custom_doh_url: "https://resolver.example/dns-query".to_string(),
                custom_doh_bootstrap_ips: vec!["192.0.2.53".to_string()],
                ..ExitDnsConfig::default()
            },
            Some(&wireguard),
            ExitDnsResolverConfig::Doh {
                url: "https://resolver.example/dns-query".to_string(),
                bootstrap_ips: vec!["192.0.2.53".parse().unwrap()],
            },
        ),
        (
            "explicit DNS through exit",
            ExitDnsConfig {
                mode: ExitDnsMode::ThroughExit,
                through_exit_servers: vec!["9.9.9.9".to_string()],
                ..ExitDnsConfig::default()
            },
            Some(&wireguard),
            ExitDnsResolverConfig::ThroughExit {
                servers: vec!["9.9.9.9".parse().unwrap()],
            },
        ),
    ];

    for (label, policy, upstream, expected) in cases {
        assert_eq!(
            policy.resolver_config(upstream).unwrap(),
            expected,
            "{label}"
        );
    }
}
