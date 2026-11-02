//! Saving settings is refused unless the request comes from the UI itself (ADR-0035),
//! and nothing is answered to another site (ADR-0050).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use axum::http::{HeaderMap, HeaderValue, header};
use netwatch_adapter_web::{from_the_ui, same_site};

fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for &(name, value) in pairs {
        map.insert(name, HeaderValue::from_str(value).unwrap());
    }
    map
}

fn ui(host: &str) -> HeaderMap {
    headers(&[
        ("x-netwatch", "1"),
        ("content-type", "application/json"),
        ("host", host),
    ])
}

#[test]
fn the_ui_on_a_loopback_or_ip_host_may_save() {
    for host in [
        "127.0.0.1:8732",
        "localhost:8732",
        "[::1]:8732",
        "192.168.1.207:8732",
    ] {
        assert!(from_the_ui(&ui(host)), "{host}");
    }
    let mut same_origin = ui("127.0.0.1:8732");
    same_origin.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:8732"),
    );
    assert!(from_the_ui(&same_origin));
}

#[test]
fn a_cross_site_or_rebound_request_may_not() {
    let mut missing_marker = ui("127.0.0.1:8732");
    missing_marker.remove("x-netwatch");
    assert!(!from_the_ui(&missing_marker), "no X-Netwatch header");

    let mut form = ui("127.0.0.1:8732");
    form.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    assert!(!from_the_ui(&form), "not JSON");

    assert!(!from_the_ui(&ui("evil.example:8732")), "DNS rebinding");

    let mut other_origin = ui("127.0.0.1:8732");
    other_origin.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://evil.example"),
    );
    assert!(!from_the_ui(&other_origin), "another origin");
}

#[test]
fn another_site_may_not_read_either() {
    // A WebSocket from another page: CORS doesn't stop it, the site check does.
    let socket = headers(&[
        ("host", "127.0.0.1:8732"),
        ("origin", "https://evil.example"),
    ]);
    assert!(!same_site(&socket), "another origin");
    assert!(
        !same_site(&headers(&[("host", "evil.example:8732")])),
        "DNS rebinding"
    );

    let ui = headers(&[
        ("host", "127.0.0.1:8732"),
        ("origin", "http://127.0.0.1:8732"),
    ]);
    assert!(same_site(&ui));
    assert!(same_site(&headers(&[("host", "localhost:8732")])), "curl");
}
