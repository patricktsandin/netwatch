//! Address scopes.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015).

use netwatch_shared::address::{is_public, is_shared};

#[test]
fn only_internet_routed_addresses_are_public() {
    let public = |a: &str| is_public(a.parse().unwrap());
    assert!(public("81.2.69.142"));
    assert!(public("2a00:1450::1"));
    for private in [
        "192.168.1.1",
        "10.0.0.1",
        "100.64.0.1",
        "169.254.1.1",
        "fe80::1",
        "fd00::1",
        "2001:db8::1",
    ] {
        assert!(!public(private), "{private}");
    }
}

#[test]
fn only_rfc_6598_space_is_shared() {
    let shared = |a: &str| is_shared(a.parse().unwrap());
    assert!(shared("100.64.0.1"));
    assert!(shared("100.127.3.193"));
    assert!(!shared("100.128.0.1"));
    assert!(!shared("192.168.1.1"));
}
