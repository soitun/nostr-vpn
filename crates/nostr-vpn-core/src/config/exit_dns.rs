pub const CLOUDFLARE_DOH_URL: &str = "https://cloudflare-dns.com/dns-query";
pub const CLOUDFLARE_DOH_BOOTSTRAP_IPS: &[&str] = &["1.1.1.1", "1.0.0.1"];
pub const QUAD9_DOH_URL: &str = "https://dns.quad9.net/dns-query";
pub const QUAD9_DOH_BOOTSTRAP_IPS: &[&str] = &["9.9.9.9", "149.112.112.112"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExitDnsMode {
    #[default]
    Automatic,
    Encrypted,
    ThroughExit,
}

impl ExitDnsMode {
    pub fn is_automatic(&self) -> bool {
        *self == Self::Automatic
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Encrypted => "encrypted",
            Self::ThroughExit => "through_exit",
        }
    }
}

impl std::str::FromStr for ExitDnsMode {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "automatic" | "auto" => Ok(Self::Automatic),
            "encrypted" | "encrypted_dns" | "doh" => Ok(Self::Encrypted),
            "through_exit" | "exit" | "dns_through_exit" => Ok(Self::ThroughExit),
            _ => Err("expected one of: automatic, encrypted, through_exit"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExitDohProvider {
    #[default]
    Cloudflare,
    Quad9,
    Custom,
}

impl ExitDohProvider {
    pub fn is_cloudflare(&self) -> bool {
        *self == Self::Cloudflare
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloudflare => "cloudflare",
            Self::Quad9 => "quad9",
            Self::Custom => "custom",
        }
    }
}

impl std::str::FromStr for ExitDohProvider {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cloudflare" => Ok(Self::Cloudflare),
            "quad9" => Ok(Self::Quad9),
            "custom" => Ok(Self::Custom),
            _ => Err("expected one of: cloudflare, quad9, custom"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExitDnsConfig {
    #[serde(default, skip_serializing_if = "ExitDnsMode::is_automatic")]
    pub mode: ExitDnsMode,
    #[serde(default, skip_serializing_if = "ExitDohProvider::is_cloudflare")]
    pub doh_provider: ExitDohProvider,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub custom_doh_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_doh_bootstrap_ips: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub through_exit_servers: Vec<String>,
}

impl Default for ExitDnsConfig {
    fn default() -> Self {
        Self {
            mode: ExitDnsMode::Automatic,
            doh_provider: ExitDohProvider::Cloudflare,
            custom_doh_url: String::new(),
            custom_doh_bootstrap_ips: Vec::new(),
            through_exit_servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitDnsResolverConfig {
    Doh {
        url: String,
        bootstrap_ips: Vec<IpAddr>,
    },
    ThroughExit {
        servers: Vec<IpAddr>,
    },
    FailClosed,
}

impl ExitDnsResolverConfig {
    pub fn through_exit_servers(&self) -> &[IpAddr] {
        match self {
            Self::Doh { .. } => &[],
            Self::ThroughExit { servers } => servers,
            Self::FailClosed => &[],
        }
    }
}

impl ExitDnsConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn normalize(&mut self) {
        self.custom_doh_url = self.custom_doh_url.trim().to_string();
        normalize_string_values(&mut self.custom_doh_bootstrap_ips);
        normalize_string_values(&mut self.through_exit_servers);
    }

    pub fn resolver_config(
        &self,
        wireguard_exit: Option<&WireGuardExitConfig>,
    ) -> Result<ExitDnsResolverConfig> {
        match self.mode {
            ExitDnsMode::Automatic => {
                if let Some(wireguard) = wireguard_exit
                    && !wireguard.dns.is_empty()
                {
                    return Ok(ExitDnsResolverConfig::ThroughExit {
                        servers: parse_exit_dns_ips(&wireguard.dns, "WireGuard profile DNS")?,
                    });
                }
                preset_doh_resolver_config(ExitDohProvider::Cloudflare)
            }
            ExitDnsMode::Encrypted => match self.doh_provider {
                ExitDohProvider::Cloudflare | ExitDohProvider::Quad9 => {
                    preset_doh_resolver_config(self.doh_provider)
                }
                ExitDohProvider::Custom => {
                    if self.custom_doh_url.is_empty() {
                        return Err(anyhow!("custom encrypted DNS requires an HTTPS URL"));
                    }
                    let resolver = ExitDnsResolverConfig::Doh {
                        url: self.custom_doh_url.clone(),
                        bootstrap_ips: parse_exit_dns_ips(
                            &self.custom_doh_bootstrap_ips,
                            "custom encrypted DNS bootstrap IPs",
                        )?,
                    };
                    #[cfg(feature = "secure-dns")]
                    if let ExitDnsResolverConfig::Doh { url, bootstrap_ips } = &resolver {
                        crate::secure_dns::validate_doh_resolver(url, bootstrap_ips)
                            .map_err(|error| anyhow!(error))?;
                    }
                    Ok(resolver)
                }
            },
            ExitDnsMode::ThroughExit => Ok(ExitDnsResolverConfig::ThroughExit {
                servers: parse_exit_dns_ips(
                    &self.through_exit_servers,
                    "DNS-through-exit servers",
                )?,
            }),
        }
    }
}

fn preset_doh_resolver_config(provider: ExitDohProvider) -> Result<ExitDnsResolverConfig> {
    let (url, bootstrap) = match provider {
        ExitDohProvider::Cloudflare => (CLOUDFLARE_DOH_URL, CLOUDFLARE_DOH_BOOTSTRAP_IPS),
        ExitDohProvider::Quad9 => (QUAD9_DOH_URL, QUAD9_DOH_BOOTSTRAP_IPS),
        ExitDohProvider::Custom => {
            return Err(anyhow!("custom encrypted DNS requires explicit settings"));
        }
    };
    Ok(ExitDnsResolverConfig::Doh {
        url: url.to_string(),
        bootstrap_ips: bootstrap
            .iter()
            .map(|ip| ip.parse::<IpAddr>().expect("built-in DNS IP is valid"))
            .collect(),
    })
}

fn parse_exit_dns_ips(values: &[String], label: &str) -> Result<Vec<IpAddr>> {
    if values.is_empty() {
        return Err(anyhow!("{label} require at least one IP address"));
    }
    let mut ips = values
        .iter()
        .map(|value| {
            value
                .parse::<IpAddr>()
                .with_context(|| format!("invalid {label} address '{value}'"))
        })
        .collect::<Result<Vec<_>>>()?;
    ips.sort_unstable();
    ips.dedup();
    Ok(ips)
}

fn normalize_string_values(values: &mut Vec<String>) {
    *values = values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    values.sort();
    values.dedup();
}
