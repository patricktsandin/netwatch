//! DNS lookups for router names (ADR-0040): PTR records, and the TXT records of Team
//! Cymru's origin service.
//!
//! These aren't probes. They don't measure anything, so they go to the system
//! resolver over an ordinary UDP socket, unpinned, and a lost one just means a name
//! is missing. What this module hides is the DNS wire format: encoding a question,
//! and walking a response's records with their compressed names. That walk takes
//! untrusted bytes, so the parts that index into them ([`read_name`] and
//! [`record_at`]) are plain loops over bytes, which Kani proves never panic.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// The record types looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordType {
    /// A name for an address.
    Ptr,
    /// Free text: Team Cymru's answers.
    Txt,
}

impl RecordType {
    const fn code(self) -> u16 {
        match self {
            RecordType::Ptr => 12,
            RecordType::Txt => 16,
        }
    }
}

const HEADER: usize = 12;
/// The longest name DNS allows, in wire form.
const MAX_NAME: usize = 255;
/// How many compression pointers one name may follow. Each must point backwards,
/// so a loop is impossible anyway; this bounds the work.
const MAX_POINTERS: usize = 16;

/// The name to look up for an address's PTR record: `4.3.2.1.in-addr.arpa`, or the
/// nibbles of an IPv6 address under `ip6.arpa`.
pub fn reverse_name(address: IpAddr) -> String {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, c, d] = v4.octets();
            format!("{d}.{c}.{b}.{a}.in-addr.arpa")
        }
        IpAddr::V6(v6) => {
            let mut name = String::new();
            for byte in v6.octets().iter().rev() {
                name.push_str(&format!("{:x}.{:x}.", byte & 0x0f, byte >> 4));
            }
            name + "ip6.arpa"
        }
    }
}

/// Team Cymru's origin lookup for an address: the reversed address under
/// `origin.asn.cymru.com`, or its nibbles under `origin6.asn.cymru.com`.
pub fn origin_name(address: IpAddr) -> String {
    let reverse = reverse_name(address);
    match address {
        IpAddr::V4(_) => reverse.replace("in-addr.arpa", "origin.asn.cymru.com"),
        IpAddr::V6(_) => reverse.replace("ip6.arpa", "origin6.asn.cymru.com"),
    }
}

/// Team Cymru's lookup for an AS's name: `AS13335.asn.cymru.com`.
pub fn as_name_name(asn: u32) -> String {
    format!("AS{asn}.asn.cymru.com")
}

/// A recursive query for `name`, with `id` as its ID. `None` if `name` can't be a
/// DNS name: an empty label, a label over 63 bytes, or over 255 bytes in all.
pub fn query(id: u16, name: &str, kind: RecordType) -> Option<Vec<u8>> {
    let mut packet = Vec::with_capacity(HEADER + name.len() + 6);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]); // RD; QDCOUNT = 1
    let start = packet.len();
    for label in name.trim_end_matches('.').split('.') {
        let length = u8::try_from(label.len())
            .ok()
            .filter(|&l| (1..=63).contains(&l))?;
        packet.push(length);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    if packet.len() - start > MAX_NAME {
        return None;
    }
    packet.extend_from_slice(&kind.code().to_be_bytes());
    packet.extend_from_slice(&[0, 1]); // class IN
    Some(packet)
}

/// Reads the (possibly compressed) name at `offset` into `out` as dotted text.
/// Returns how many bytes it wrote and the offset just past the name where it
/// starts (not where its pointers lead). `None` for anything malformed: a name past
/// the end, longer than `out`, or with a pointer that doesn't point before every
/// earlier one. Pointer targets only ever decrease, so the walk always ends.
fn read_name<const N: usize>(
    packet: &[u8],
    offset: usize,
    out: &mut [u8; N],
) -> Option<(usize, usize)> {
    let mut at = offset;
    let mut written = 0;
    let mut after = None;
    let mut pointers = 0;
    // Every pointer must lead before this, which starts as the name itself.
    let mut limit = offset;
    loop {
        let length = *packet.get(at)?;
        if length & 0xc0 == 0xc0 {
            let target = (usize::from(length & 0x3f) << 8) | usize::from(*packet.get(at + 1)?);
            if target >= limit.min(at) || pointers == MAX_POINTERS {
                return None;
            }
            pointers += 1;
            limit = target;
            if after.is_none() {
                after = Some(at + 2);
            }
            at = target;
            continue;
        }
        if length & 0xc0 != 0 {
            return None;
        }
        let length = usize::from(length);
        if length == 0 {
            return Some((written, after.unwrap_or(at + 1)));
        }
        if written > 0 {
            if written >= N {
                return None;
            }
            out[written] = b'.';
            written += 1;
        }
        let mut i = 0;
        while i < length {
            let byte = *packet.get(at + 1 + i)?;
            if written >= N {
                return None;
            }
            out[written] = byte;
            written += 1;
            i += 1;
        }
        at += 1 + length;
    }
}

/// Skips the name at `offset`, returning the offset just past it. A pointer ends
/// the name where it stands, so skipping never follows one.
fn skip_name(packet: &[u8], offset: usize) -> Option<usize> {
    let mut at = offset;
    loop {
        let length = *packet.get(at)?;
        match length & 0xc0 {
            0xc0 => return packet.get(at + 1).map(|_| at + 2),
            0 if length == 0 => return Some(at + 1),
            0 => at += 1 + usize::from(length),
            _ => return None,
        }
    }
}

/// One resource record's place in a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    kind: u16,
    data_start: usize,
    data_end: usize,
    /// Where the next record starts.
    next: usize,
}

/// The record whose name starts at `offset`.
fn record_at(packet: &[u8], offset: usize) -> Option<Record> {
    let fixed = skip_name(packet, offset)?;
    let field = |i: usize| -> Option<u16> {
        Some(u16::from_be_bytes([
            *packet.get(fixed + i)?,
            *packet.get(fixed + i + 1)?,
        ]))
    };
    let kind = field(0)?;
    let length = usize::from(field(8)?);
    let data_start = fixed + 10;
    let data_end = data_start + length;
    if data_end > packet.len() {
        return None;
    }
    Some(Record {
        kind,
        data_start,
        data_end,
        next: data_end,
    })
}

/// Parses a response to [`query`] with `id`: every answer of type `kind`, as text
/// (a PTR's name, or a TXT record's strings joined). `None` if it isn't a response
/// to that query, or is malformed. A response with no such answers (NXDOMAIN, say)
/// is `Some` and empty.
pub fn parse_response(packet: &[u8], id: u16, kind: RecordType) -> Option<Vec<String>> {
    let header = packet.get(..HEADER)?;
    let is_response = header[2] & 0x80 != 0;
    if !is_response || u16::from_be_bytes([header[0], header[1]]) != id {
        return None;
    }
    let questions = u16::from_be_bytes([header[4], header[5]]);
    let answers = u16::from_be_bytes([header[6], header[7]]);
    let mut at = HEADER;
    for _ in 0..questions {
        at = skip_name(packet, at)? + 4;
    }
    let mut texts = Vec::new();
    for _ in 0..answers {
        let record = record_at(packet, at)?;
        let data = packet.get(record.data_start..record.data_end)?;
        if record.kind == kind.code() {
            match kind {
                RecordType::Ptr => {
                    let mut name = [0u8; MAX_NAME];
                    let (length, _) = read_name(packet, record.data_start, &mut name)?;
                    texts.push(String::from_utf8_lossy(&name[..length]).into_owned());
                }
                RecordType::Txt => {
                    let mut text = Vec::new();
                    let mut i = 0;
                    while i < data.len() {
                        let length = usize::from(data[i]);
                        text.extend_from_slice(data.get(i + 1..i + 1 + length)?);
                        i += 1 + length;
                    }
                    texts.push(String::from_utf8_lossy(&text).into_owned());
                }
            }
        }
        at = record.next;
    }
    Some(texts)
}

/// How long one attempt waits for an answer, and how many attempts there are.
const TIMEOUT: Duration = Duration::from_secs(2);
const ATTEMPTS: usize = 2;

/// Looks up `name`'s records of type `kind` at `server`. `None` if there was no
/// usable answer; `Some` and empty if the name has none.
pub async fn lookup(server: IpAddr, name: &str, kind: RecordType) -> Option<Vec<String>> {
    let local: SocketAddr = match server {
        IpAddr::V4(_) => (std::net::Ipv4Addr::UNSPECIFIED, 0).into(),
        IpAddr::V6(_) => (std::net::Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let socket = tokio::net::UdpSocket::bind(local).await.ok()?;
    socket.connect((server, 53)).await.ok()?;
    for _ in 0..ATTEMPTS {
        let id = random_id();
        socket.send(&query(id, name, kind)?).await.ok()?;
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        let mut buffer = [0u8; 1500];
        while let Ok(Ok(length)) = tokio::time::timeout_at(deadline, socket.recv(&mut buffer)).await
        {
            if let Some(texts) = parse_response(&buffer[..length], id, kind) {
                return Some(texts);
            }
        }
    }
    None
}

/// An unpredictable query ID, so that an off-path sender can't guess it. The
/// standard library's hasher keys are random per process and per hasher.
fn random_id() -> u16 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(0);
    // Truncation keeps the low 16 bits, which is all an ID holds.
    hasher.finish() as u16
}

/// Kani proof harnesses (ADR-0016).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// Reading a name never panics, whatever the bytes and wherever it starts, and
    /// what it reports is within bounds.
    /// The output buffer is shorter than a real one, which exercises the
    /// too-long checks in fewer steps.
    #[kani::proof]
    #[kani::unwind(10)]
    fn reading_a_name_never_panics() {
        let buffer: [u8; 8] = kani::any();
        let length: usize = kani::any_where(|&l| l <= 8);
        let offset: usize = kani::any_where(|&o| o <= 8);
        let mut out = [0u8; 4];
        if let Some((written, after)) = read_name(&buffer[..length], offset, &mut out) {
            assert!(written <= out.len());
            assert!(after <= length);
        }
    }

    /// Finding a record never panics, and its data is within the packet.
    #[kani::proof]
    #[kani::unwind(14)]
    fn finding_a_record_never_panics() {
        let buffer: [u8; 12] = kani::any();
        let length: usize = kani::any_where(|&l| l <= 12);
        let offset: usize = kani::any_where(|&o| o <= 12);
        if let Some(record) = record_at(&buffer[..length], offset) {
            assert!(record.data_start <= record.data_end && record.data_end <= length);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_names_for_both_families() {
        assert_eq!(
            reverse_name("192.0.2.1".parse().unwrap()),
            "1.2.0.192.in-addr.arpa"
        );
        let v6 = reverse_name("2001:db8::1".parse().unwrap());
        assert!(v6.starts_with("1.0.0.0.0.0.0.0."), "{v6}");
        assert!(v6.ends_with("8.b.d.0.1.0.0.2.ip6.arpa"), "{v6}");
        assert_eq!(
            origin_name("192.0.2.1".parse().unwrap()),
            "1.2.0.192.origin.asn.cymru.com"
        );
    }

    #[test]
    fn invalid_names_have_no_query() {
        assert!(query(1, "a..b", RecordType::Ptr).is_none());
        assert!(query(1, &"x".repeat(64), RecordType::Ptr).is_none());
        assert!(query(1, "example.com", RecordType::Txt).is_some());
    }

    /// A response the way a resolver writes one: our question echoed, then an answer
    /// whose name is a pointer to the question's.
    fn response(id: u16, kind: RecordType, data: &[u8]) -> Vec<u8> {
        let mut packet = query(id, "1.2.0.192.in-addr.arpa", kind).unwrap();
        packet[2] |= 0x80;
        packet[7] = 1; // ANCOUNT = 1
        packet.extend_from_slice(&[0xc0, 12]);
        packet.extend_from_slice(&kind.code().to_be_bytes());
        packet.extend_from_slice(&[0, 1, 0, 0, 0x0e, 0x10]);
        packet.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
        packet.extend_from_slice(data);
        packet
    }

    #[test]
    fn a_ptr_answer_parses_to_its_name() {
        let data = b"\x03ae1\x07example\x03net\x00";
        let packet = response(7, RecordType::Ptr, data);
        assert_eq!(
            parse_response(&packet, 7, RecordType::Ptr),
            Some(vec!["ae1.example.net".to_owned()])
        );
        assert_eq!(
            parse_response(&packet, 8, RecordType::Ptr),
            None,
            "another ID"
        );
    }

    #[test]
    fn a_txt_answer_joins_its_strings() {
        let packet = response(9, RecordType::Txt, b"\x0513335\x08 | 1.1.1");
        assert_eq!(
            parse_response(&packet, 9, RecordType::Txt),
            Some(vec!["13335 | 1.1.1".to_owned()])
        );
    }

    #[test]
    fn a_pointer_that_loops_is_malformed() {
        let mut packet = response(7, RecordType::Ptr, &[0xc0, 0]);
        // The answer's data points forwards, at itself.
        let at = packet.len() - 2;
        packet[at + 1] = u8::try_from(at).unwrap();
        assert_eq!(parse_response(&packet, 7, RecordType::Ptr), None);
    }
}
