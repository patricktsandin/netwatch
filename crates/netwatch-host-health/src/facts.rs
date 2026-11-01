//! Facts about the host's connection that a NOC asks for first (ADR-0039): its public
//! identity, and the NIC's link facts.

use std::net::IpAddr;

/// The address this host's traffic leaves the ISP with, and where it enters
/// Cloudflare (glossary: *public identity*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicIdentity {
    /// The public address.
    pub address: IpAddr,
    /// The Cloudflare site, as an IATA airport code such as `ARN`.
    pub colo: Option<String>,
    /// The country, as Cloudflare places the address.
    pub location: Option<String>,
}

impl PublicIdentity {
    /// One line for people: `203.0.113.7 (enters Cloudflare at ARN, SE)`.
    pub fn text(&self) -> String {
        let at: Vec<&str> = [self.colo.as_deref(), self.location.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        if at.is_empty() {
            self.address.to_string()
        } else {
            format!("{} (enters Cloudflare at {})", self.address, at.join(", "))
        }
    }
}

/// Parses Cloudflare's `/cdn-cgi/trace`: `key=value` lines, of which `ip`, `colo`,
/// and `loc` matter. `None` without a valid `ip`.
pub fn parse_trace(text: &str) -> Option<PublicIdentity> {
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k == key).then(|| v.trim())
        })
    };
    let short = |v: &str| (!v.is_empty() && v.len() <= 16).then(|| v.to_owned());
    Some(PublicIdentity {
        address: field("ip")?.parse().ok()?,
        colo: field("colo").and_then(short),
        location: field("loc").and_then(short),
    })
}

/// A NIC's negotiated link, from sysfs (glossary: *link facts*). Each fact is
/// optional: a link that is down has no speed, and virtual devices have no driver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkFacts {
    /// Negotiated speed, in Mb/s.
    pub speed_mbps: Option<u32>,
    /// Full duplex (`true`) or half.
    pub full_duplex: Option<bool>,
    /// The kernel driver's name.
    pub driver: Option<String>,
    /// The MTU, in bytes.
    pub mtu: Option<u32>,
    /// How many times the link has gone up or down since the device appeared.
    pub carrier_changes: Option<u64>,
}

impl LinkFacts {
    /// One line for the NIC row and the report: `100 Mb/s full duplex · r8152 · …`.
    pub fn summary(&self) -> String {
        let link = match (self.speed_mbps, self.full_duplex) {
            (Some(speed), Some(full)) => Some(format!(
                "{speed} Mb/s {} duplex",
                if full { "full" } else { "half" }
            )),
            (Some(speed), None) => Some(format!("{speed} Mb/s")),
            _ => None,
        };
        [
            link,
            self.driver.clone(),
            self.mtu.map(|m| format!("MTU {m}")),
            self.carrier_changes.map(|c| format!("{c} carrier changes")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Parsing a trace never panics, whatever the text. Bounded to 8 bytes, as for the
    /// other text parsers.
    #[kani::proof]
    #[kani::unwind(10)]
    fn trace_parsing_never_panics() {
        let bytes: [u8; 8] = kani::any();
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let _ = parse_trace(text);
        }
    }
}
