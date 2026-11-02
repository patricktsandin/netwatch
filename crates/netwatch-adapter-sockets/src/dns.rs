//! DNS probe encoding and parsing (ADR-0027).
//!
//! The probe asks for the root zone's NS records (`. IN NS`). Every resolver answers
//! that from cache, so the probe measures the resolver, not the health of some
//! domain. The DNS message ID is the probe's sequence number. A DNS response can't
//! carry our 64-bit token, so matching relies on the sequence window (ADR-0023).

use netwatch_probing::{ProbeSequence, Reply, ReplyKind};

/// The question every probe asks: the root name, type NS, class IN.
const QUESTION: [u8; 5] = [0x00, 0x00, 0x02, 0x00, 0x01];
const HEADER: usize = 12;
/// The length of a query: header and question.
pub const QUERY_LEN: usize = HEADER + QUESTION.len();

/// A recursive query for `. IN NS`, with the sequence number as its ID.
pub fn query(sequence: ProbeSequence) -> [u8; QUERY_LEN] {
    let mut packet = [0; QUERY_LEN];
    packet[0..2].copy_from_slice(&sequence.get().to_be_bytes());
    packet[2] = 0x01; // RD: recursion desired
    packet[5] = 1; // QDCOUNT = 1
    packet[HEADER..].copy_from_slice(&QUESTION);
    packet
}

/// Parses a response to one of our queries. `None` for anything that isn't a
/// response to exactly our question: a query, a truncated packet, or a response
/// about some other question. NOERROR is an answer; any other rcode is the server
/// refusing, with that rcode.
pub fn parse_response(packet: &[u8]) -> Option<Reply> {
    let header = packet.get(..HEADER)?;
    let is_response = header[2] & 0x80 != 0;
    let opcode = (header[2] >> 3) & 0x0f;
    let questions = u16::from_be_bytes([header[4], header[5]]);
    if !is_response || opcode != 0 || questions != 1 {
        return None;
    }
    if packet.get(HEADER..QUERY_LEN)? != QUESTION {
        return None;
    }
    let rcode = header[3] & 0x0f;
    Some(Reply {
        sequence: ProbeSequence::new(u16::from_be_bytes([header[0], header[1]])),
        token: None,
        kind: if rcode == 0 {
            ReplyKind::Echo
        } else {
            ReplyKind::Refused(rcode)
        },
    })
}

/// Parses our own query, as an ICMP error quotes it on the error queue: its ID is
/// the sequence of the probe the error answers (ADR-0049). `None` for anything that
/// isn't exactly one of our queries, including one quoted too short to tell.
pub fn parse_own_query(packet: &[u8]) -> Option<ProbeSequence> {
    let header = packet.get(..HEADER)?;
    let is_query = header[2] & 0x80 == 0;
    let questions = u16::from_be_bytes([header[4], header[5]]);
    if !is_query || questions != 1 || packet.get(HEADER..QUERY_LEN)? != QUESTION {
        return None;
    }
    Some(ProbeSequence::new(u16::from_be_bytes([
        header[0], header[1],
    ])))
}

/// Kani proof harnesses (ADR-0016).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// For every input of up to 24 bytes, parsing never panics, and whatever it
    /// accepts is a response to exactly our question.
    #[kani::proof]
    #[kani::unwind(26)]
    fn parse_never_panics_and_accepts_only_our_question() {
        let buffer: [u8; 24] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= buffer.len());
        let packet = &buffer[..len];
        if let Some(reply) = parse_response(packet) {
            assert!(len >= QUERY_LEN);
            assert!(packet[2] & 0x80 != 0);
            assert!(packet[HEADER..QUERY_LEN] == QUESTION);
            assert!(reply.sequence.get().to_be_bytes() == [packet[0], packet[1]]);
        }
    }

    /// For every input of up to 24 bytes, parsing a quoted query never panics, and
    /// whatever it accepts is a query for exactly our question.
    #[kani::proof]
    #[kani::unwind(26)]
    fn parse_own_query_never_panics_and_accepts_only_our_query() {
        let buffer: [u8; 24] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= buffer.len());
        let packet = &buffer[..len];
        if let Some(sequence) = parse_own_query(packet) {
            assert!(len >= QUERY_LEN);
            assert!(packet[2] & 0x80 == 0);
            assert!(packet[HEADER..QUERY_LEN] == QUESTION);
            assert!(sequence.get().to_be_bytes() == [packet[0], packet[1]]);
        }
    }

    /// Our query, quoted whole, parses back to its ID.
    #[kani::proof]
    fn our_query_quoted_parses_to_its_sequence() {
        let sequence = ProbeSequence::new(kani::any());
        assert!(parse_own_query(&query(sequence)) == Some(sequence));
    }

    /// A query, turned into a NOERROR response by a server, parses back to its ID.
    #[kani::proof]
    fn our_query_answered_parses_to_its_sequence() {
        let sequence = ProbeSequence::new(kani::any());
        let mut packet = query(sequence);
        packet[2] |= 0x80;
        let reply = parse_response(&packet).expect("a response to our question");
        assert!(reply.sequence == sequence && reply.kind == ReplyKind::Echo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servfail_is_a_refusal_with_its_rcode() {
        let mut packet = query(ProbeSequence::new(7)).to_vec();
        packet[2] |= 0x80;
        packet[3] = 0x02;
        assert_eq!(
            parse_response(&packet).map(|r| r.kind),
            Some(ReplyKind::Refused(2))
        );
    }

    #[test]
    fn a_quoted_query_is_matched_by_its_id_and_a_short_quote_by_nothing() {
        let packet = query(ProbeSequence::new(7));
        assert_eq!(parse_own_query(&packet), Some(ProbeSequence::new(7)));
        // A router that quotes only the UDP header leaves nothing to match by.
        assert_eq!(parse_own_query(&packet[..2]), None);
        let mut response = packet;
        response[2] |= 0x80;
        assert_eq!(parse_own_query(&response), None);
    }

    #[test]
    fn a_response_about_another_question_is_ignored() {
        let mut packet = query(ProbeSequence::new(7)).to_vec();
        packet[2] |= 0x80;
        packet[HEADER + 2] = 0x01; // type A instead of NS
        assert_eq!(parse_response(&packet), None);
    }
}
