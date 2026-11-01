//! Just enough BTF to read one enum's names and values: `skb_drop_reason`, from the
//! running kernel's `/sys/kernel/btf/vmlinux`, so drop reasons are named the way this
//! kernel names them (ADR-0008).
//!
//! The bytes come from the kernel, but they are parsed defensively all the same:
//! every offset is checked, and anything malformed is `None`, never a panic
//! (Kani-proven for small inputs).

const MAGIC: u16 = 0xeb9f;
const KIND_ENUM: u32 = 6;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Whether the string at `offset` is exactly `name`: its bytes, then a NUL. Reads
/// only `name.len() + 1` bytes, never scanning for the end.
fn names(strings: &[u8], offset: u32, name: &[u8]) -> bool {
    let Some(start) = usize::try_from(offset).ok() else {
        return false;
    };
    let Some(end) = start.checked_add(name.len()) else {
        return false;
    };
    match (strings.get(start..end), strings.get(end)) {
        (Some(found), Some(0)) => found.iter().zip(name).all(|(a, b)| a == b),
        _ => false,
    }
}

/// The NUL-terminated bytes at `offset` in the string section.
fn bytes_at(strings: &[u8], offset: u32) -> Option<&[u8]> {
    let rest = strings.get(usize::try_from(offset).ok()?..)?;
    let end = rest.iter().position(|&b| b == 0)?;
    rest.get(..end)
}

/// How many bytes follow a type's 12-byte header, by kind. `None` for a kind this
/// parser doesn't know, after which nothing can be read reliably.
fn trailing(kind: u32, vlen: usize) -> Option<usize> {
    match kind {
        2 | 7..=12 | 16 | 18 => Some(0),
        1 | 14 | 17 => Some(4),
        3 => Some(12),
        4 | 5 | 15 | 19 => vlen.checked_mul(12),
        6 | 13 => vlen.checked_mul(8),
        _ => None,
    }
}

/// Where the enum called `name` is: its entries' offset in the type section, how
/// many there are, and the two sections. All the untrusted offsets are handled
/// here, on raw bytes, which is what the Kani harness proves.
struct Found<'a> {
    types: &'a [u8],
    strings: &'a [u8],
    entries: usize,
    count: usize,
}

fn find<'a>(bytes: &'a [u8], name: &[u8]) -> Option<Found<'a>> {
    if u16_at(bytes, 0)? != MAGIC {
        return None;
    }
    let at = |field: usize| -> Option<usize> { usize::try_from(u32_at(bytes, field)?).ok() };
    let header = at(4)?;
    let (type_off, type_len, str_off, str_len) = (at(8)?, at(12)?, at(16)?, at(20)?);
    let types_start = header.checked_add(type_off)?;
    let types = bytes.get(types_start..types_start.checked_add(type_len)?)?;
    let strings_start = header.checked_add(str_off)?;
    let strings = bytes.get(strings_start..strings_start.checked_add(str_len)?)?;
    let mut offset = 0usize;
    while offset < types.len() {
        let name_off = u32_at(types, offset)?;
        let info = u32_at(types, offset.checked_add(4)?)?;
        let kind = (info >> 24) & 0x1f;
        let vlen = usize::try_from(info & 0xffff).ok()?;
        let body = offset.checked_add(12)?;
        if kind == KIND_ENUM && names(strings, name_off, name) {
            return Some(Found {
                types,
                strings,
                entries: body,
                count: vlen,
            });
        }
        offset = body.checked_add(trailing(kind, vlen)?)?;
    }
    None
}

/// The names and values of the enum called `name`, in declaration order. `None` if
/// `bytes` isn't little-endian BTF, is malformed, or has no such enum.
pub fn enum_variants(bytes: &[u8], name: &str) -> Option<Vec<(String, u32)>> {
    let found = find(bytes, name.as_bytes())?;
    (0..found.count)
        .map(|i| {
            let entry = found.entries.checked_add(i.checked_mul(8)?)?;
            let variant = bytes_at(found.strings, u32_at(found.types, entry)?)?;
            let value = u32_at(found.types, entry.checked_add(4)?)?;
            Some((String::from_utf8(variant.to_vec()).ok()?, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BTF blob with an int type, then `enum skb_drop_reason { A = 2, B = 5 }`.
    fn blob() -> Vec<u8> {
        let strings = b"\0int\0skb_drop_reason\0SKB_DROP_REASON_A\0SKB_DROP_REASON_B\0";
        let mut types = Vec::new();
        let word = |v: u32, out: &mut Vec<u8>| out.extend_from_slice(&v.to_le_bytes());
        // int: name "int" (1), kind 1, size 4, then its 4-byte encoding.
        for v in [1, 1 << 24, 4, 32] {
            word(v, &mut types);
        }
        // enum: name at 5, kind 6, vlen 2, size 4, then (name, value) pairs.
        for v in [5, 6 << 24 | 2, 4, 21, 2, 39, 5] {
            word(v, &mut types);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC.to_le_bytes());
        out.extend_from_slice(&[1, 0]);
        for v in [
            24,
            0,
            types.len() as u32,
            types.len() as u32,
            strings.len() as u32,
        ] {
            word(v, &mut out);
        }
        out.extend_from_slice(&types);
        out.extend_from_slice(strings);
        out
    }

    #[test]
    fn finds_an_enum_after_skipping_other_types() {
        assert_eq!(
            enum_variants(&blob(), "skb_drop_reason"),
            Some(vec![
                ("SKB_DROP_REASON_A".to_owned(), 2),
                ("SKB_DROP_REASON_B".to_owned(), 5)
            ])
        );
    }

    #[test]
    fn malformed_or_foreign_bytes_are_none() {
        assert_eq!(enum_variants(&blob(), "no_such_enum"), None);
        let blob = blob();
        for cut in 0..blob.len() {
            assert_eq!(
                enum_variants(&blob[..cut], "skb_drop_reason"),
                None,
                "cut at {cut}"
            );
        }
        assert_eq!(enum_variants(b"\x7fELF", "skb_drop_reason"), None);
    }

    #[test]
    fn reads_the_running_kernels_drop_reasons() {
        let Ok(bytes) = std::fs::read("/sys/kernel/btf/vmlinux") else {
            return; // no BTF here: nothing to compare with
        };
        let reasons = enum_variants(&bytes, "skb_drop_reason").unwrap();
        assert!(
            reasons
                .iter()
                .any(|(name, value)| name == "SKB_DROP_REASON_NOT_SPECIFIED" && *value == 2)
        );
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Finding the enum never panics, whatever the bytes: every offset in the header
    /// and the type section is checked. Bounded to 40 bytes, which reaches past the
    /// header into the type section. Decoding names afterwards only reads within
    /// what `find` returned.
    #[kani::proof]
    #[kani::unwind(5)]
    fn finding_an_enum_never_panics() {
        let bytes: [u8; 40] = kani::any();
        let length: usize = kani::any_where(|&l| l <= 40);
        let _ = find(&bytes[..length], b"a");
    }
}
