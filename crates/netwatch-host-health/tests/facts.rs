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

#[test]
fn cymru_answers_give_the_first_origin_and_the_as_name() {
    use netwatch_host_health::facts::{RouterName, parse_as_name, parse_origin};
    assert_eq!(
        parse_origin("13335 | 1.1.1.0/24 | AU | apnic | 2011-08-11"),
        Some(13335)
    );
    assert_eq!(
        parse_origin("3356 1299 | 4.0.0.0/9 | US | arin | 1992-12-01"),
        Some(3356)
    );
    assert_eq!(parse_origin("not an answer"), None);
    assert_eq!(
        parse_as_name("13335 | US | arin | 2010-07-14 | CLOUDFLARENET, US").as_deref(),
        Some("CLOUDFLARENET, US")
    );
    assert_eq!(parse_as_name("13335 | US"), None);
    let name = RouterName {
        ptr: Some("ae1.cr1.lon2.example.net".to_owned()),
        asn: Some(64500),
        as_name: Some("EXAMPLE".to_owned()),
    };
    assert_eq!(
        name.text().as_deref(),
        Some("ae1.cr1.lon2.example.net · AS64500 EXAMPLE")
    );
    assert_eq!(RouterName::default().text(), None);
}
