//! `InterfaceName` accepts exactly what the kernel's `dev_valid_name` accepts,
//! restricted to valid UTF-8.

use netwatch_shared::{InterfaceName, InvalidInterfaceName};

fn parse(bytes: &[u8]) -> Result<InterfaceName, InvalidInterfaceName> {
    InterfaceName::parse(bytes)
}

#[test]
fn accepts_names_seen_on_real_hosts() {
    for name in [
        "lo",
        "eth0",
        "wlp0s20f3",
        "wwan0",
        "wg0",
        "enp0s31f6",
        "br-1a2b3c4d5e6f",
    ] {
        assert_eq!(parse(name.as_bytes()).unwrap().as_str(), name);
    }
}

#[test]
fn accepts_exactly_fifteen_bytes() {
    assert!(parse(b"abcdefghijklmno").is_ok());
}

#[test]
fn rejects_sixteen_bytes() {
    assert_eq!(
        parse(b"abcdefghijklmnop"),
        Err(InvalidInterfaceName::TooLong(16))
    );
}

#[test]
fn rejects_empty() {
    assert_eq!(parse(b""), Err(InvalidInterfaceName::Empty));
}

#[test]
fn rejects_dot_and_dot_dot() {
    assert_eq!(parse(b"."), Err(InvalidInterfaceName::Dot));
    assert_eq!(parse(b".."), Err(InvalidInterfaceName::Dot));
}

#[test]
fn accepts_dots_inside_a_name() {
    assert!(parse(b"eth0.100").is_ok());
}

#[test]
fn rejects_every_byte_the_kernel_rejects() {
    for b in [b'/', b':', 0, b' ', b'\t', b'\n', 0x0b, 0x0c, b'\r'] {
        assert_eq!(
            parse(&[b'a', b, b'b']),
            Err(InvalidInterfaceName::ForbiddenByte(b)),
            "byte {b:#04x}"
        );
    }
}

#[test]
fn rejects_invalid_utf8() {
    assert_eq!(parse(&[b'a', 0xff]), Err(InvalidInterfaceName::NotUtf8));
}

#[test]
fn accepts_non_ascii_utf8() {
    assert!(parse("eth-é".as_bytes()).is_ok());
}
