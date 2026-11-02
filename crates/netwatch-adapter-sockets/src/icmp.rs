//! ICMP echo encoding and parsing for datagram ("ping") sockets.
//!
//! On a datagram ICMP socket, the kernel fills in the identifier and the checksum,
//! and it delivers replies without the IP header. So a request is 8 bytes of ICMP
//! header followed by the 8-byte token, and a reply starts directly at the ICMP
//! header.

use netwatch_probing::{ProbeIndex, ProbeSequence, Reply, ReplyKind};

/// The address family of an echo exchange. It determines the ICMP type numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// ICMP over IPv4.
    V4,
    /// ICMPv6.
    V6,
}

impl Family {
    const fn request_type(self) -> u8 {
        match self {
            Family::V4 => 8,
            Family::V6 => 128,
        }
    }

    const fn reply_type(self) -> u8 {
        match self {
            Family::V4 => 0,
            Family::V6 => 129,
        }
    }
}

/// The length of an echo request: 8 bytes of header plus the 8-byte token.
pub const REQUEST_LEN: usize = 16;

/// An echo request for the probe with `sequence`, carrying `token` as its payload
/// (ADR-0023). The identifier and checksum fields are left as zero for the kernel
/// to fill in.
pub fn echo_request(
    family: Family,
    sequence: ProbeSequence,
    token: ProbeIndex,
) -> [u8; REQUEST_LEN] {
    let mut packet = [0; REQUEST_LEN];
    packet[0] = family.request_type();
    packet[6..8].copy_from_slice(&sequence.get().to_be_bytes());
    packet[8..16].copy_from_slice(&token.get().to_be_bytes());
    packet
}

/// Parses a packet read from a datagram ICMP socket. `None` for anything that isn't
/// an echo reply carrying a full token: other ICMP types, truncated packets,
/// foreign payloads. Such packets are never attributed to any probe.
pub fn parse_echo_reply(family: Family, packet: &[u8]) -> Option<Reply> {
    let header: &[u8; REQUEST_LEN] = packet.get(..REQUEST_LEN)?.try_into().ok()?;
    if header[0] != family.reply_type() || header[1] != 0 {
        return None;
    }
    Some(Reply {
        sequence: ProbeSequence::new(u16::from_be_bytes([header[6], header[7]])),
        token: Some(ProbeIndex::new(u64::from_be_bytes([
            header[8], header[9], header[10], header[11], header[12], header[13], header[14],
            header[15],
        ]))),
        kind: ReplyKind::Echo,
    })
}

/// The length of an echo request's header, which is all of it that a router must
/// quote in an ICMP error (RFC 792).
const HEADER_LEN: usize = 8;

/// Parses one of *our own* echo requests, as the error queue returns it alongside
/// an ICMP error: the sequence number of the probe the error answers, and its token
/// if the router quoted enough of the probe to include it. Many routers quote only
/// the 8-byte header, as RFC 792 allows, and those probes are matched by sequence
/// number alone (ADR-0023).
pub fn parse_own_request(
    family: Family,
    packet: &[u8],
) -> Option<(ProbeSequence, Option<ProbeIndex>)> {
    let header = packet.get(..HEADER_LEN)?;
    if header[0] != family.request_type() {
        return None;
    }
    let token = packet
        .get(HEADER_LEN..REQUEST_LEN)
        .and_then(|t| <[u8; 8]>::try_from(t).ok())
        .map(|t| ProbeIndex::new(u64::from_be_bytes(t)));
    Some((
        ProbeSequence::new(u16::from_be_bytes([header[6], header[7]])),
        token,
    ))
}

/// What an ICMP error means for a probe.
pub fn error_kind(family: Family, icmp_type: u8, icmp_code: u8) -> Option<ReplyKind> {
    match (family, icmp_type) {
        (Family::V4, 11) | (Family::V6, 3) => Some(ReplyKind::TimeExceeded),
        (Family::V4, 3) | (Family::V6, 1) => Some(ReplyKind::Unreachable(
            netwatch_probing::UnreachableCode(icmp_code),
        )),
        _ => None,
    }
}

/// Kani proof harnesses (ADR-0016). Every reply parser of untrusted bytes is checked
/// against every possible input.
#[cfg(kani)]
mod proofs {
    use super::*;

    /// For every input of up to 24 bytes: parsing never panics, and anything it
    /// accepts really is an echo reply, whose fields are the bytes at their offsets.
    #[kani::proof]
    #[kani::unwind(26)]
    fn parse_never_panics_and_reads_the_right_fields() {
        let buffer: [u8; 24] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= buffer.len());
        let family = if kani::any() { Family::V4 } else { Family::V6 };
        let packet = &buffer[..len];

        if let Some(reply) = parse_echo_reply(family, packet) {
            assert!(len >= REQUEST_LEN);
            assert!(packet[0] == family.reply_type() && packet[1] == 0);
            assert!(reply.sequence.get().to_be_bytes() == [packet[6], packet[7]]);
            let token = reply.token.expect("echo replies always carry a token");
            assert!(token.get().to_be_bytes()[..] == packet[8..16]);
        }
    }

    /// A request, turned into a reply by the network (only the type changes), parses
    /// back to the sequence number and token it was sent with.
    #[kani::proof]
    fn a_request_echoed_back_parses_to_its_own_sequence_and_token() {
        let family = if kani::any() { Family::V4 } else { Family::V6 };
        let sequence = ProbeSequence::new(kani::any());
        let token = ProbeIndex::new(kani::any());
        let mut packet = echo_request(family, sequence, token);
        packet[0] = family.reply_type();

        let reply = parse_echo_reply(family, &packet).expect("a well-formed echo reply");
        assert!(reply.sequence == sequence && reply.token == Some(token));
    }

    /// Parsing a quoted request never panics, and it always reads the sequence
    /// number; the token exactly when the quote is long enough to hold it.
    #[kani::proof]
    #[kani::unwind(26)]
    fn parsing_a_quoted_request_never_panics() {
        let buffer: [u8; 24] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= buffer.len());
        let family = if kani::any() { Family::V4 } else { Family::V6 };
        let packet = &buffer[..len];
        if let Some((sequence, token)) = parse_own_request(family, packet) {
            assert!(len >= HEADER_LEN && packet[0] == family.request_type());
            assert!(sequence.get().to_be_bytes() == [packet[6], packet[7]]);
            assert!(token.is_some() == (len >= REQUEST_LEN));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_router_quoting_only_the_header_still_names_the_probe_without_its_token() {
        let request = echo_request(Family::V4, ProbeSequence::new(7), ProbeIndex::new(42));
        assert_eq!(
            parse_own_request(Family::V4, &request[..8]),
            Some((ProbeSequence::new(7), None))
        );
        assert_eq!(
            parse_own_request(Family::V4, &request),
            Some((ProbeSequence::new(7), Some(ProbeIndex::new(42))))
        );
        assert_eq!(parse_own_request(Family::V4, &request[..7]), None);
    }
}
