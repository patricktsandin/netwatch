//! Identities, and the epochs during which a role holds one (ADR-0002, ADR-0003).

use std::fmt;
use std::net::IpAddr;
use std::num::{NonZeroU32, NonZeroU64};

/// The concrete thing behind a [`Role`](crate::Role) at a given moment.
///
/// Two identities are equal only if every field is equal. Renaming an
/// interface therefore produces a new identity, and so a new epoch. Renames
/// are rare, and a renamed device is worth marking on the graphs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Identity {
    /// A network interface, physical or virtual.
    Interface {
        /// The kernel's index for the interface.
        index: InterfaceIndex,
        /// The interface's name at the time it was observed.
        name: InterfaceName,
    },
    /// An IP address, such as a hop's responding address or a target's address.
    Address(IpAddr),
}

/// The kernel's `ifindex`. It is always positive; zero means "no interface" in the
/// kernel's own interfaces, and that meaning never needs to reach the domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceIndex(NonZeroU32);

impl InterfaceIndex {
    /// Wraps a kernel interface index.
    pub const fn new(index: NonZeroU32) -> Self {
        Self(index)
    }

    /// The index to pass back to the kernel, for example to `IP_UNICAST_IF`.
    pub const fn get(self) -> NonZeroU32 {
        self.0
    }
}

/// A Linux interface name: 1 to 15 bytes (`IFNAMSIZ` minus the terminating NUL),
/// not `.` or `..`, and containing no `/`, `:`, NUL, or byte that the kernel's
/// `isspace` accepts. These are exactly the rules of the kernel's `dev_valid_name`.
/// In addition, netwatch requires valid UTF-8, so that every name can be displayed.
///
/// Stored inline, so a name is `Copy` and never allocates. The bytes after `len` are
/// always zero. Because a name can't contain NUL, the derived ordering is exactly
/// the lexicographic order of the names.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceName {
    bytes: [u8; InterfaceName::MAX_LEN],
    len: u8,
}

/// Why a byte string isn't a valid interface name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidInterfaceName {
    /// The name was empty.
    #[error("interface name is empty")]
    Empty,
    /// The name is longer than the kernel allows.
    #[error("interface name is {0} bytes; the kernel allows at most 15")]
    TooLong(usize),
    /// The name is `.` or `..`.
    #[error("interface name may not be `.` or `..`")]
    Dot,
    /// The name contains a byte the kernel rejects.
    #[error("interface name contains forbidden byte {0:#04x}")]
    ForbiddenByte(u8),
    /// The name is not valid UTF-8. The kernel allows this, but netwatch doesn't,
    /// because every name must be displayable.
    #[error("interface name is not valid UTF-8")]
    NotUtf8,
}

impl InterfaceName {
    /// The longest name the kernel accepts, in bytes.
    pub const MAX_LEN: usize = 15;

    /// Parses an interface name exactly as it arrives from the kernel, for example
    /// the bytes of `IFLA_IFNAME` with the NUL terminator removed.
    pub fn parse(bytes: &[u8]) -> Result<Self, InvalidInterfaceName> {
        if bytes.is_empty() {
            return Err(InvalidInterfaceName::Empty);
        }
        if bytes.len() > Self::MAX_LEN {
            return Err(InvalidInterfaceName::TooLong(bytes.len()));
        }
        if bytes == b"." || bytes == b".." {
            return Err(InvalidInterfaceName::Dot);
        }
        if let Some(&b) = bytes.iter().find(|&&b| is_forbidden_in_name(b)) {
            return Err(InvalidInterfaceName::ForbiddenByte(b));
        }
        std::str::from_utf8(bytes).map_err(|_| InvalidInterfaceName::NotUtf8)?;
        let mut stored = [0; Self::MAX_LEN];
        stored[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: stored,
            len: u8::try_from(bytes.len()).expect("checked above: at most MAX_LEN bytes"),
        })
    }

    /// The name's bytes, exactly as the kernel reported them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.as_bytes()).expect("invariant: validated as UTF-8 by parse")
    }
}

/// The bytes `dev_valid_name` rejects. The kernel's `isspace` is wider than
/// Rust's `is_ascii_whitespace`: it includes vertical tab (0x0b) and, from its
/// Latin-1 table, non-breaking space (0xa0).
fn is_forbidden_in_name(b: u8) -> bool {
    matches!(
        b,
        b'/' | b':' | 0 | b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | 0xa0
    )
}

impl fmt::Debug for InterfaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl fmt::Display for InterfaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A span of time during which a role keeps the same identity (ADR-0020).
///
/// Epochs are globally unique and strictly increasing. Each one names exactly one
/// role and one identity, forever. Only the topology aggregate allocates them;
/// [`IdentityEpoch::new`] is public only so that persistence can read epochs back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdentityEpoch(NonZeroU64);

impl IdentityEpoch {
    /// Wraps an epoch number. Outside the topology aggregate, only use this to
    /// read back an epoch that the aggregate allocated earlier.
    pub const fn new(raw: NonZeroU64) -> Self {
        Self(raw)
    }

    /// The epoch number, as persisted.
    pub const fn get(self) -> NonZeroU64 {
        self.0
    }
}

/// Kani proof harnesses (ADR-0016). `InterfaceName::parse` handles untrusted bytes
/// from the kernel, so it is checked against every possible input, not just against
/// examples. The bound comes from CBMC's cost of reasoning about UTF-8 validation,
/// which roughly doubles with every 2 symbolic bytes:
/// - every input of up to `EXHAUSTIVE_LEN` bytes, covering every rule and every UTF-8
///   sequence length (about 2 minutes)
/// - every input longer than `MAX_LEN` bytes is rejected as too long, before any other
///   check runs
///
/// No rule in `parse` depends on the length, except the empty and too-long checks,
/// and both of those are covered exactly.
#[cfg(kani)]
mod proofs {
    use super::*;

    /// For every input: `parse` never panics. Whatever it accepts satisfies every
    /// rule of `dev_valid_name` and keeps the bytes exactly as given. Whatever it
    /// rejects, it rejects for a reason that is true of the input.
    const EXHAUSTIVE_LEN: usize = 8;

    #[kani::proof]
    #[kani::unwind(10)]
    fn parse_accepts_exactly_valid_names() {
        let buffer: [u8; EXHAUSTIVE_LEN] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= buffer.len());
        let bytes = &buffer[..len];

        match InterfaceName::parse(bytes) {
            Ok(name) => {
                let accepted = name.as_bytes();
                assert_eq!(accepted, bytes);
                assert!(!accepted.is_empty() && accepted.len() <= InterfaceName::MAX_LEN);
                assert!(accepted != b"." && accepted != b"..");
                assert!(accepted.iter().all(|&b| !is_forbidden_in_name(b)));
            }
            Err(InvalidInterfaceName::Empty) => assert!(bytes.is_empty()),
            Err(InvalidInterfaceName::TooLong(n)) => {
                assert!(n == bytes.len() && n > InterfaceName::MAX_LEN);
            }
            Err(InvalidInterfaceName::Dot) => assert!(bytes == b"." || bytes == b".."),
            Err(InvalidInterfaceName::ForbiddenByte(b)) => {
                assert!(is_forbidden_in_name(b) && bytes.contains(&b));
            }
            Err(InvalidInterfaceName::NotUtf8) => assert!(std::str::from_utf8(bytes).is_err()),
        }
    }

    #[kani::proof]
    #[kani::unwind(18)]
    fn overlong_input_is_rejected_whatever_its_contents() {
        let buffer: [u8; InterfaceName::MAX_LEN + 1] = kani::any();
        assert_eq!(
            InterfaceName::parse(&buffer),
            Err(InvalidInterfaceName::TooLong(InterfaceName::MAX_LEN + 1))
        );
    }
}
