use clap::{CommandFactory, error::ErrorKind};

use crate::Cli;

#[test]
fn clap_binary_name_is_nvpn() {
    let command = Cli::command();
    assert_eq!(command.get_name(), "nvpn");
}

#[test]
fn clap_supports_root_version_flag() {
    let error = Cli::command()
        .try_get_matches_from(["nvpn", "--version"])
        .expect_err("--version should display version and exit");
    assert_eq!(error.kind(), ErrorKind::DisplayVersion);
    assert!(
        error
            .to_string()
            .contains(&format!("nvpn {}", env!("CARGO_PKG_VERSION"))),
        "version output should include binary name and package version"
    );
}

#[test]
fn build_reports_fips_core_component_version() {
    let version = crate::fips_core_build_version();
    assert!(!version.trim().is_empty());
    #[cfg(feature = "embedded-fips")]
    assert!(version.starts_with(fips_core::version::VERSION));
}

#[test]
fn clap_includes_tailscale_style_commands() {
    let command = Cli::command();
    for name in [
        "start",
        "stop",
        "repair-network",
        "reload",
        "pause",
        "resume",
        "connect",
        "status",
        "set",
        "ping",
        "doctor",
        "ip",
        "whois",
        "install-cli",
        "uninstall-cli",
        "service",
        "version",
    ] {
        assert!(
            command
                .get_subcommands()
                .any(|subcommand| subcommand.get_name() == name),
            "missing subcommand {name}"
        );
    }
}

#[cfg(feature = "paid-exit")]
#[test]
fn clap_includes_paid_exit_command_when_enabled() {
    let command = Cli::command();
    assert!(
        command
            .get_subcommands()
            .any(|subcommand| subcommand.get_name() == "paid-exit"),
        "missing paid-exit subcommand"
    );
}

#[cfg(not(feature = "paid-exit"))]
#[test]
fn clap_omits_paid_exit_command_by_default() {
    let command = Cli::command();
    assert!(
        !command
            .get_subcommands()
            .any(|subcommand| subcommand.get_name() == "paid-exit"),
        "paid-exit subcommand should require the paid-exit feature"
    );
}

#[test]
fn clap_roster_device_commands_keep_participant_aliases() {
    Cli::command()
        .try_get_matches_from(["nvpn", "add-device", "--device", "npub1example"])
        .expect("new device command parses");
    Cli::command()
        .try_get_matches_from(["nvpn", "add-participant", "--participant", "npub1example"])
        .expect("legacy participant command parses");
}

#[test]
fn clap_set_supports_autoconnect_flag() {
    let command = Cli::command();
    let set = command
        .get_subcommands()
        .find(|subcommand| subcommand.get_name() == "set")
        .expect("set subcommand exists");
    assert!(
        set.get_arguments()
            .any(|argument| argument.get_long() == Some("autoconnect")),
        "missing --autoconnect on set command"
    );
}

#[test]
fn clap_set_supports_join_request_listener_flag() {
    let command = Cli::command();
    let set = command
        .get_subcommands()
        .find(|subcommand| subcommand.get_name() == "set")
        .expect("set subcommand exists");
    assert!(
        set.get_arguments()
            .any(|argument| argument.get_long() == Some("join-requests-enabled")),
        "missing --join-requests-enabled on set command"
    );
}

#[test]
fn clap_set_supports_route_advertisement_flags() {
    let command = Cli::command();
    let set = command
        .get_subcommands()
        .find(|subcommand| subcommand.get_name() == "set")
        .expect("set subcommand exists");
    assert!(
        set.get_arguments()
            .any(|argument| argument.get_long() == Some("advertise-routes")),
        "missing --advertise-routes on set command"
    );
    assert!(
        set.get_arguments()
            .any(|argument| argument.get_long() == Some("advertise-exit-node")),
        "missing --advertise-exit-node on set command"
    );
    assert!(
        set.get_arguments()
            .any(|argument| argument.get_long() == Some("exit-node")),
        "missing --exit-node on set command"
    );
    assert!(
        set.get_arguments()
            .any(|argument| argument.get_long() == Some("exit-node-leak-protection")),
        "missing --exit-node-leak-protection on set command"
    );
}

#[cfg(feature = "paid-exit")]
#[test]
fn clap_set_supports_paid_exit_seller_flags() {
    let command = Cli::command();
    let set = command
        .get_subcommands()
        .find(|subcommand| subcommand.get_name() == "set")
        .expect("set subcommand exists");
    for flag in [
        "paid-exit-enabled",
        "paid-exit-meter",
        "paid-exit-upstream",
        "paid-exit-price-msat",
        "paid-exit-per-units",
        "paid-exit-accepted-mints",
        "paid-exit-country-code",
        "paid-exit-region",
        "paid-exit-asn",
        "paid-exit-network-class",
        "paid-exit-ipv4",
        "paid-exit-ipv6",
        "paid-exit-max-channel-capacity-sat",
        "paid-exit-channel-expiry-secs",
        "paid-exit-free-probe-units",
        "paid-exit-grace-units",
    ] {
        assert!(
            set.get_arguments()
                .any(|argument| argument.get_long() == Some(flag)),
            "missing --{flag} on set command"
        );
    }
}

#[test]
fn clap_set_supports_wireguard_exit_flags() {
    let command = Cli::command();
    let set = command
        .get_subcommands()
        .find(|subcommand| subcommand.get_name() == "set")
        .expect("set subcommand exists");
    for flag in [
        "wireguard-exit-enabled",
        "wireguard-exit-address",
        "wireguard-exit-private-key",
        "wireguard-exit-peer-public-key",
        "wireguard-exit-endpoint",
        "wireguard-exit-allowed-ips",
        "wireguard-exit-config",
        "wireguard-exit-config-file",
    ] {
        assert!(
            set.get_arguments()
                .any(|argument| argument.get_long() == Some(flag)),
            "missing --{flag} on set command"
        );
    }
}
