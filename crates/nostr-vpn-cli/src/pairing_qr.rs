use std::path::Path;

use anyhow::{Context, Result};
use qrcode::QrCode;

use nostr_vpn_core::config::AppConfig;

const JOIN_REQUEST_LINK_PREFIX: &str = "nvpn://join-request/";

pub(crate) fn pending_pairing_uri(config_path: &Path) -> Result<String> {
    let app = AppConfig::load(config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;
    app.pending_nostr_join_request_link(JOIN_REQUEST_LINK_PREFIX)
        .context("config has no valid pending device-approval request")
}

pub(crate) fn render_pairing_output(uri: &str) -> Result<String> {
    let code = QrCode::new(uri.as_bytes()).context("failed to encode pairing QR code")?;
    let qr = code
        .render::<char>()
        .quiet_zone(true)
        .module_dimensions(2, 1)
        .dark_color('#')
        .light_color(' ')
        .build();
    Ok(format!("{qr}\n\n{uri}\n"))
}

pub(crate) fn print_pending_pairing_qr(config_path: &Path) -> Result<()> {
    let uri = pending_pairing_uri(config_path)?;
    print!("{}", render_pairing_output(&uri)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn terminal_output_contains_ascii_qr_and_exact_uri() {
        let uri = "nvpn://join-request/eyJkZXZpY2VBcHBLZXlOcHViIjoibnB1YjE";
        let output = render_pairing_output(uri).expect("render pairing output");

        assert!(output.is_ascii());
        assert!(output.lines().any(|line| line.contains("#######")));
        assert_eq!(output.lines().filter(|line| *line == uri).count(), 1);
        assert!(output.ends_with(&format!("\n\n{uri}\n")));
    }

    #[test]
    fn reads_the_canonical_pending_bootstrap_from_config() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nvpn-pairing-qr-{}-{nonce}.toml",
            std::process::id()
        ));
        let mut app = AppConfig::generated();
        app.ensure_pending_nostr_join_request(1_789_000_000)
            .expect("pending request");
        app.save(&path).expect("save config");

        let expected = app
            .pending_nostr_join_request_link(JOIN_REQUEST_LINK_PREFIX)
            .expect("expected URI");
        assert_eq!(pending_pairing_uri(&path).expect("loaded URI"), expected);

        let _ = std::fs::remove_file(path);
    }
}
