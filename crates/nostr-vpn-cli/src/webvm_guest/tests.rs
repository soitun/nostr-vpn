use super::*;
use nostr_sdk::ToBech32;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "nvpn-webvm-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ))
}

#[test]
fn first_boot_persists_one_stable_compact_join_bootstrap() {
    let path = temp_path("config").with_extension("toml");
    let first = load_or_initialize_config(&path, 1_778_998_000).expect("first boot");
    let route = first
        .nostr_keys()
        .expect("route keys")
        .public_key()
        .to_bech32()
        .expect("route npub");
    let first_uri = webvm_pairing_uri(&first, &route).expect("first URI");
    let second = load_or_initialize_config(&path, 1_778_998_100).expect("second boot");
    let second_uri = webvm_pairing_uri(&second, &route).expect("second URI");
    assert_eq!(first_uri, second_uri);
    assert!(first_uri.starts_with(JOIN_REQUEST_LINK_PREFIX));
    assert!(
        first_uri.len() <= 420,
        "pairing URI was {} bytes",
        first_uri.len()
    );
    let bootstrap =
        nostr_vpn_core::identity_bridge::parse_nostr_identity_device_approval_bootstrap(
            first_uri.split_once('?').expect("return route").0,
            &[JOIN_REQUEST_LINK_PREFIX],
        )
        .expect("parse compact bootstrap")
        .expect("bootstrap payload");
    assert_eq!(
        serde_json::to_value(&bootstrap)
            .expect("serialize bootstrap")
            .as_object()
            .expect("bootstrap object")
            .len(),
        4
    );
    assert!(bootstrap.label.is_some());
    let pending = second.pending_nostr_join_request.expect("pending request");
    assert_ne!(
        pending.request.request_pubkey,
        pending.request.device_app_key_pubkey
    );
    assert_eq!(bootstrap.request_secret, pending.request.request_secret);

    AppConfig::delete_persisted_secrets_for_path(&path).expect("delete secrets");
    let _ = fs::remove_file(path);
}

#[test]
fn pairing_uri_replace_is_atomic_and_private() {
    let path = temp_path("pairing-uri");
    write_pairing_uri(&path, "nvpn://join-request/first").expect("first write");
    write_pairing_uri(&path, "nvpn://join-request/second").expect("second write");
    assert_eq!(
        fs::read_to_string(&path).expect("read pairing URI"),
        "nvpn://join-request/second\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&path)
                .expect("pairing metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    remove_pairing_uri(&path).expect("remove pairing URI");
    assert!(!path.exists());
}

#[test]
fn invalid_webvm_arguments_are_rejected_before_networking() {
    let args = WebvmGuestArgs {
        config: PathBuf::from("/tmp/config.toml"),
        ethernet_interface: "eth0".to_string(),
        discovery_scope: "fips-overlay-v1".to_string(),
        host_hint_port: FIPS_NOSTR_PUBSUB_SERVICE_PORT + 1,
        pairing_uri_file: PathBuf::from("/run/webvm/pairing-uri"),
        tun_interface: "nvpn0".to_string(),
    };
    assert!(
        validate_args(&args)
            .expect_err("wrong service port")
            .to_string()
            .contains("7368")
    );
}

#[test]
fn webvm_ethernet_underlay_rejects_any_l3_address() {
    validate_ethernet_underlay_snapshot("eth0", "", "", "")
        .expect("unconfigured Ethernet underlay");

    for addresses in [
        "2: eth0    inet 192.0.2.2/24 scope global eth0\n",
        "2: eth0    inet6 fe80::1/64 scope link\n",
    ] {
        let error = validate_ethernet_underlay_snapshot("eth0", addresses, "", "")
            .expect_err("L3 address must fail closed");
        assert!(error.to_string().contains("L3 address"));
    }
}

#[test]
fn webvm_ethernet_underlay_rejects_ipv4_or_ipv6_default_route() {
    for (ipv4_defaults, ipv6_defaults) in [
        ("default via 192.0.2.1 dev eth0\n", ""),
        ("", "default via fe80::1 dev eth0 metric 1024\n"),
    ] {
        let error = validate_ethernet_underlay_snapshot("eth0", "", ipv4_defaults, ipv6_defaults)
            .expect_err("default route must fail closed");
        assert!(error.to_string().contains("default route"));
    }
}

#[test]
fn approved_webvm_config_requires_selected_exit_in_signed_roster() {
    use nostr_sdk::prelude::Keys;

    let mut app = AppConfig::generated();
    let own_pubkey = app.own_nostr_pubkey_hex().expect("own AppKey");
    let exit_pubkey = Keys::generate().public_key().to_hex();
    app.networks[0].enabled = true;
    app.networks[0].devices = vec![exit_pubkey.clone()];
    app.networks[0].admins = vec![own_pubkey.clone()];
    app.networks[0].shared_roster_updated_at = 1;
    app.networks[0].shared_roster_signed_by = own_pubkey.clone();
    app.exit_node = exit_pubkey;
    app.ensure_defaults();

    assert!(!app.participant_pubkeys_hex().contains(&own_pubkey));
    validate_approved_config(&app).expect("rostered Nostr VPN exit");
    let hint = webvm_mesh_ingress_hint(&app).expect("mesh ingress hint");
    assert_eq!(hint.len(), 41);
    assert_eq!(&hint[..9], b"NVPNMESH1");
    assert_eq!(hex::encode(&hint[9..]), app.exit_node);
    app.exit_node = Keys::generate().public_key().to_hex();
    assert!(
        validate_approved_config(&app)
            .expect_err("unrostered exit must fail")
            .to_string()
            .contains("signed roster")
    );
}
