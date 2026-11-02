//! Address scopes: whether an address is routed on the internet, or held inside an
//! ISP's carrier-grade NAT. Several contexts ask: Probing, to pick an address that
//! hop probes leave the LAN toward (ADR-0043), and the report, for the *first
//! public hop*.

use std::net::IpAddr;

/// Whether `address` is in the shared address space of RFC 6598 (100.64.0.0/10),
/// which ISPs use inside carrier-grade NAT.
pub fn is_shared(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            a == 100 && (64..128).contains(&b)
        }
        IpAddr::V6(_) => false,
    }
}

/// Whether `address` is routed on the internet: not private, shared (CGNAT),
/// loopback, link-local, or documentation space. The first hop with such an
/// address is the *first public hop*.
pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let shared = is_shared(address);
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_documentation()
                || v4.is_broadcast()
                || shared)
        }
        IpAddr::V6(v6) => {
            let [first, second, ..] = v6.segments();
            // Global unicast is 2000::/3, less documentation space (2001:db8::/32).
            (0x2000..0x4000).contains(&first) && !(first == 0x2001 && second == 0x0db8)
        }
    }
}
