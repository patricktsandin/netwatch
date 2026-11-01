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
    let field = |key: &str| field(text, key);
    let short = |v: &str| (!v.is_empty() && v.len() <= 16).then(|| v.to_owned());
    Some(PublicIdentity {
        address: field("ip")?.parse().ok()?,
        colo: field("colo").and_then(short),
        location: field("loc").and_then(short),
    })
}

/// The value of the first `key=value` line for `key`, trimmed.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let (start, end) = field_range(text.as_bytes(), key.as_bytes())?;
    // `=` and `\n` are ASCII, so both ends fall on character boundaries.
    Some(text.get(start..end)?.trim())
}

/// Where the value of the first `key=value` line for `key` is, as a byte range.
/// Plain index loops, so that Kani can prove it never panics.
fn field_range(bytes: &[u8], key: &[u8]) -> Option<(usize, usize)> {
    let mut line = 0;
    while line < bytes.len() {
        let mut end = line;
        while end < bytes.len() && bytes[end] != b'\n' {
            end += 1;
        }
        let value = line + key.len();
        if value < end && bytes[value] == b'=' && bytes[line..value] == *key {
            return Some((value + 1, end));
        }
        line = end + 1;
    }
    None
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

    /// Finding a field in a trace never panics, whatever the bytes: that's all of the
    /// parsing that is ours. Address parsing is the standard library's. Bounded to 6
    /// bytes, which covers a key, its separator, and line breaks.
    #[kani::proof]
    #[kani::unwind(8)]
    fn finding_a_trace_field_never_panics() {
        let bytes: [u8; 6] = kani::any();
        let length: usize = kani::any_where(|&l| l <= 6);
        let _ = field_range(&bytes[..length], b"ip");
    }
}
