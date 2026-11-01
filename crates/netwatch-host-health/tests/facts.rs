//! The public identity and the NIC's link facts (ADR-0039).

use netwatch_host_health::facts::{LinkFacts, PublicIdentity, parse_trace};

#[test]
fn a_trace_page_gives_the_public_address_and_where_traffic_enters() {
    let trace = "fl=123f45\nh=one.one.one.one\nip=203.0.113.7\nts=1793545384.1\nvisit_scheme=https\nuag=netwatch\ncolo=ARN\nsliver=none\nhttp=http/1.1\nloc=SE\ntls=TLSv1.3\n";
    assert_eq!(
        parse_trace(trace),
        Some(PublicIdentity {
            address: "203.0.113.7".parse().unwrap(),
            colo: Some("ARN".to_owned()),
            location: Some("SE".to_owned()),
        })
    );
}

#[test]
fn a_trace_without_a_valid_address_is_none() {
    assert_eq!(parse_trace("colo=ARN\nloc=SE\n"), None);
    assert_eq!(parse_trace("ip=not-an-address\n"), None);
    assert_eq!(parse_trace(""), None);
}

#[test]
fn identity_text_names_the_address_and_entry_point() {
    let id = parse_trace("ip=2001:db8::1\ncolo=FRA\nloc=DE\n").unwrap();
    assert_eq!(id.text(), "2001:db8::1 (enters Cloudflare at FRA, DE)");
}

#[test]
fn link_facts_read_as_one_line() {
    let facts = LinkFacts {
        speed_mbps: Some(100),
        full_duplex: Some(true),
        driver: Some("r8152".to_owned()),
        mtu: Some(1500),
        carrier_changes: Some(14),
    };
    assert_eq!(
        facts.summary(),
        "100 Mb/s full duplex · r8152 · MTU 1500 · 14 carrier changes"
    );
    assert_eq!(LinkFacts::default().summary(), "");
}
