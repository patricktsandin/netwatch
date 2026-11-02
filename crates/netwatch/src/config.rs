//! Configuration: a TOML file, parsed once into validated values (ADR-0014).
//!
//! The daemon reads the file named by `NETWATCH_CONFIG`, or
//! `/etc/netwatch/netwatch.toml` if it exists, or else the built-in defaults
//! (`deploy/netwatch.toml`). `NETWATCH_DB` and `NETWATCH_LISTEN` override the file.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, bail};
use netwatch_adapter_sockets::http::{Expected, HttpsTarget};
use netwatch_shared::TargetId;
use serde::Deserialize;

const DEFAULTS: &str = include_str!("../../../deploy/netwatch.toml");
const SYSTEM_PATH: &str = "/etc/netwatch/netwatch.toml";

#[derive(Debug)]
pub struct Config {
    /// The file it was read from, or `None` for the built-in defaults. The settings
    /// page edits this file (ADR-0035).
    pub source: Option<PathBuf>,
    pub listen: SocketAddr,
    pub database: PathBuf,
    pub targets: Vec<Target>,
    /// Whether router names include Team Cymru's origin AS (ADR-0040).
    pub asn_lookups: bool,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub id: TargetId,
    pub name: String,
    pub probe: TargetProbe,
}

#[derive(Debug, Clone)]
pub enum TargetProbe {
    Echo(IpAddr),
    Dns(IpAddr),
    Https(HttpsTarget),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    listen: String,
    database: PathBuf,
    #[serde(rename = "target", default)]
    targets: Vec<RawTarget>,
    asn_lookups: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    id: u32,
    name: Option<String>,
    icmp: Option<IpAddr>,
    /// A server address, or `"system"` for the system's own upstream resolver.
    dns: Option<String>,
    https: Option<String>,
    expect_status: Option<u16>,
    expect_body: Option<String>,
}

/// Loads the configuration from wherever it lives, then applies the environment's
/// overrides.
pub fn load() -> anyhow::Result<Config> {
    let (text, origin, source) = match std::env::var_os("NETWATCH_CONFIG") {
        Some(path) => (
            std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?,
            PathBuf::from(&path).display().to_string(),
            Some(PathBuf::from(path)),
        ),
        None if std::path::Path::new(SYSTEM_PATH).exists() => (
            std::fs::read_to_string(SYSTEM_PATH).context("reading /etc/netwatch/netwatch.toml")?,
            SYSTEM_PATH.to_owned(),
            Some(PathBuf::from(SYSTEM_PATH)),
        ),
        None => (DEFAULTS.to_owned(), "built-in defaults".to_owned(), None),
    };
    let mut config = parse(&text).with_context(|| format!("invalid configuration in {origin}"))?;
    config.source = source;
    if let Some(database) = std::env::var_os("NETWATCH_DB") {
        config.database = PathBuf::from(database);
    }
    if let Ok(listen) = std::env::var("NETWATCH_LISTEN") {
        config.listen = listen
            .parse()
            .context("NETWATCH_LISTEN must be an address:port")?;
    }
    tracing::info!(%origin, targets = config.targets.len(), "configuration loaded");
    Ok(config)
}

pub fn parse(text: &str) -> anyhow::Result<Config> {
    let raw: RawConfig = toml::from_str(text)?;
    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    for t in raw.targets {
        if !seen.insert(t.id) {
            bail!(
                "target id {} is used twice; ids must be unique and stable",
                t.id
            );
        }
        let system = t.dns.as_deref() == Some("system");
        let probe = match (t.icmp, t.dns, t.https) {
            (Some(address), None, None) => TargetProbe::Echo(address),
            (None, Some(server), None) if server == "system" => TargetProbe::Dns(
                system_resolver().context("no upstream resolver found for dns = \"system\"")?,
            ),
            (None, Some(server), None) => TargetProbe::Dns(server.parse().with_context(|| {
                format!("dns = {server:?} is neither an address nor \"system\"")
            })?),
            (None, None, Some(url)) => {
                TargetProbe::Https(https(&url, t.expect_status, t.expect_body)?)
            }
            _ => bail!(
                "target {} must have exactly one of icmp, dns, or https",
                t.id
            ),
        };
        let name = t.name.unwrap_or_else(|| match &probe {
            TargetProbe::Echo(address) => format!("{address} (ICMP)"),
            TargetProbe::Dns(server) if system => {
                format!("system resolver {server} (DNS)")
            }
            TargetProbe::Dns(server) => format!("{server} (DNS)"),
            TargetProbe::Https(target) => format!("https://{}{}", target.host, target.path),
        });
        targets.push(Target {
            id: TargetId::new(t.id),
            name,
            probe,
        });
    }
    Ok(Config {
        source: None,
        listen: raw
            .listen
            .parse()
            .context("listen must be an address:port")?,
        database: raw.database,
        targets,
        asn_lookups: raw.asn_lookups.unwrap_or(true),
    })
}

/// The first upstream resolver the system uses (ADR-0009). With systemd-resolved,
/// `/etc/resolv.conf` names only its local stub, and probing that would measure
/// loopback, so the upstream list in `/run/systemd/resolve/resolv.conf` comes first.
pub fn system_resolver() -> Option<IpAddr> {
    ["/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"]
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .find_map(|text| first_upstream(&text))
}

/// The first `nameserver` in a resolv.conf that isn't a loopback address.
fn first_upstream(resolv_conf: &str) -> Option<IpAddr> {
    resolv_conf
        .lines()
        .filter_map(|line| line.strip_prefix("nameserver"))
        .filter_map(|rest| rest.trim().parse::<IpAddr>().ok())
        .find(|address| !address.is_loopback())
}

fn https(url: &str, status: Option<u16>, body: Option<String>) -> anyhow::Result<HttpsTarget> {
    let Some(rest) = url.strip_prefix("https://") else {
        bail!("{url:?} must start with https://");
    };
    let (host, path) = rest
        .find('/')
        .map_or((rest, "/"), |slash| rest.split_at(slash));
    if host.is_empty() || host.contains(['@', ':', '?', '#']) {
        bail!("{url:?} must name a plain host (no userinfo, port, query, or fragment)");
    }
    let Some(status) = status else {
        bail!("https target {url:?} needs expect_status");
    };
    Ok(HttpsTarget {
        host: host.to_owned(),
        path: path.to_owned(),
        expected: match body {
            Some(text) => Expected::StatusWithBody(status, text),
            None => Expected::Status(status),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_defaults_are_valid() {
        let config = parse(DEFAULTS).unwrap();
        assert_eq!(config.targets.len(), 13);
        assert_eq!(config.listen, "127.0.0.1:8732".parse().unwrap());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let text = "listen = \"127.0.0.1:1\"\ndatabase = \"x\"\n[[target]]\nid = 1\nicmp = \"1.1.1.1\"\n[[target]]\nid = 1\ndns = \"1.1.1.1\"\n";
        assert!(parse(text).unwrap_err().to_string().contains("used twice"));
    }

    #[test]
    fn a_target_needs_exactly_one_kind() {
        let text = "listen = \"127.0.0.1:1\"\ndatabase = \"x\"\n[[target]]\nid = 1\nicmp = \"1.1.1.1\"\ndns = \"1.1.1.1\"\n";
        assert!(parse(text).is_err());
    }

    #[test]
    fn https_needs_a_plain_host_and_an_expected_status() {
        assert!(https("http://example.com/", Some(200), None).is_err());
        assert!(https("https://user@example.com/", Some(200), None).is_err());
        assert!(https("https://example.com/", None, None).is_err());
        let target = https("https://example.com", Some(204), None).unwrap();
        assert_eq!(
            (target.host.as_str(), target.path.as_str()),
            ("example.com", "/")
        );
    }

    #[test]
    fn the_system_resolver_skips_loopback_stubs() {
        let resolv = "# generated\nnameserver 127.0.0.53\nnameserver 192.168.1.1\nsearch lan\n";
        assert_eq!(first_upstream(resolv), Some("192.168.1.1".parse().unwrap()));
        assert_eq!(first_upstream("nameserver 127.0.0.53\n"), None);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(parse("listen = \"127.0.0.1:1\"\ndatabase = \"x\"\nlisten_on = 1\n").is_err());
    }
}
