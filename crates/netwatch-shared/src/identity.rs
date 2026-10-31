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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceName(String);

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
        let name = std::str::from_utf8(bytes).map_err(|_| InvalidInterfaceName::NotUtf8)?;
        Ok(Self(name.to_owned()))
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
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

impl fmt::Display for InterfaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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
