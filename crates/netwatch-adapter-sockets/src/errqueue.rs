//! Parsing `IP_RECVERR` / `IPV6_RECVERR` control messages (see `ip(7)`, `ipv6(7)`).
//!
//! When an ICMP error answers one of our probes, the kernel queues it on the socket's
//! error queue. The data is *our own original packet* (so its sequence number and
//! token are exact), and a control message carries a `sock_extended_err` followed by
//! the offender: the address of the router that sent the error.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// What a `sock_extended_err` control message says, in domain-relevant terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtendedError {
    /// The ICMP or ICMPv6 type of the error.
    pub icmp_type: u8,
    /// Its code.
    pub icmp_code: u8,
    /// The router or host that sent it.
    pub offender: Option<IpAddr>,
}

// From <linux/errqueue.h> and <linux/in.h>; the same on every Linux architecture.
const SOL_IP: i32 = 0;
const IP_RECVERR: i32 = 11;
const SOL_IPV6: i32 = 41;
const IPV6_RECVERR: i32 = 25;
const SO_EE_ORIGIN_ICMP: u8 = 2;
const SO_EE_ORIGIN_ICMP6: u8 = 3;
const AF_INET: u16 = 2;
const AF_INET6: u16 = 10;

/// `struct cmsghdr` on 64-bit Linux: `size_t len; int level; int type;`, and data
/// aligned to 8 bytes.
const CMSG_HEADER: usize = 16;
const CMSG_ALIGN: usize = 8;
/// `struct sock_extended_err`: errno (u32), origin, type, code, pad (u8 each), info
/// (u32), data (u32).
const EXTENDED_ERR: usize = 16;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn usize_at(bytes: &[u8], at: usize) -> Option<usize> {
    Some(usize::from_ne_bytes(
        bytes.get(at..at + 8)?.try_into().ok()?,
    ))
}

/// Finds the ICMP extended error among the control messages in `control`, if any.
/// Anything malformed yields `None`, never a panic. Kani proves that for every
/// input up to its bound.
pub fn parse(control: &[u8]) -> Option<ExtendedError> {
    let mut at = 0;
    while at + CMSG_HEADER <= control.len() {
        let len = usize_at(control, at)?;
        let level = i32_at(control, at + 8)?;
        let kind = i32_at(control, at + 12)?;
        if len < CMSG_HEADER || len > control.len() - at {
            return None;
        }
        let data = control.get(at + CMSG_HEADER..at + len)?;
        if (level, kind) == (SOL_IP, IP_RECVERR) || (level, kind) == (SOL_IPV6, IPV6_RECVERR) {
            return extended_error(data);
        }
        at += len.div_ceil(CMSG_ALIGN) * CMSG_ALIGN;
    }
    None
}

fn extended_error(data: &[u8]) -> Option<ExtendedError> {
    let header = data.get(..EXTENDED_ERR)?;
    let (origin, icmp_type, icmp_code) = (header[4], header[5], header[6]);
    if origin != SO_EE_ORIGIN_ICMP && origin != SO_EE_ORIGIN_ICMP6 {
        return None;
    }
    let offender = data.get(EXTENDED_ERR..).and_then(sockaddr);
    Some(ExtendedError {
        icmp_type,
        icmp_code,
        offender,
    })
}

/// A `sockaddr_in` or `sockaddr_in6`.
fn sockaddr(bytes: &[u8]) -> Option<IpAddr> {
    match u16_at(bytes, 0)? {
        AF_INET => {
            let octets: [u8; 4] = bytes.get(4..8)?.try_into().ok()?;
            Some(IpAddr::V4(Ipv4Addr::from(octets)))
        }
        AF_INET6 => {
            let octets: [u8; 16] = bytes.get(8..24)?.try_into().ok()?;
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => None,
    }
}

/// Kani proof harnesses (ADR-0016): the control buffer comes from the kernel, but
/// its parser must never panic, whatever the bytes are.
#[cfg(kani)]
mod proofs {
    use super::*;

    #[kani::proof]
    #[kani::unwind(6)]
    fn parse_never_panics() {
        let buffer: [u8; 64] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= buffer.len());
        let _ = parse(&buffer[..len]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmsg(level: i32, kind: i32, data: &[u8]) -> Vec<u8> {
        let len = CMSG_HEADER + data.len();
        let mut out = Vec::new();
        out.extend_from_slice(&len.to_ne_bytes());
        out.extend_from_slice(&level.to_ne_bytes());
        out.extend_from_slice(&kind.to_ne_bytes());
        out.extend_from_slice(data);
        out.resize(len.div_ceil(CMSG_ALIGN) * CMSG_ALIGN, 0);
        out
    }

    fn time_exceeded_from(router: [u8; 4]) -> Vec<u8> {
        let mut data = vec![0u8; EXTENDED_ERR];
        data[4] = SO_EE_ORIGIN_ICMP;
        data[5] = 11;
        data.extend_from_slice(&AF_INET.to_ne_bytes());
        data.extend_from_slice(&[0, 0]);
        data.extend_from_slice(&router);
        data.extend_from_slice(&[0; 8]);
        cmsg(SOL_IP, IP_RECVERR, &data)
    }

    #[test]
    fn reads_a_time_exceeded_error_and_its_offender() {
        assert_eq!(
            parse(&time_exceeded_from([192, 168, 1, 1])),
            Some(ExtendedError {
                icmp_type: 11,
                icmp_code: 0,
                offender: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
            })
        );
    }

    #[test]
    fn skips_other_control_messages() {
        let mut control = cmsg(1, 29, &[0; 16]); // SOL_SOCKET / SO_TIMESTAMPNS
        control.extend(time_exceeded_from([10, 0, 0, 1]));
        assert_eq!(
            parse(&control).and_then(|e| e.offender),
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
        );
    }

    #[test]
    fn rejects_a_length_that_overruns_the_buffer() {
        let mut control = time_exceeded_from([10, 0, 0, 1]);
        control[..8].copy_from_slice(&1_000usize.to_ne_bytes());
        assert_eq!(parse(&control), None);
    }
}
