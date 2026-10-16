//! Attribute forms (DWARF 5 §7.5.6, Table 7.6), and the GNU forms of GCC's
//! `dwarf2.def`. A value that refers elsewhere (a string offset, an index,
//! a reference) is returned as that reference; the tables resolve it
//! against the right section.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use super::{short, DwarfError, Format};
use crate::read::Bytes;

pub const DW_FORM_ADDR: u64 = 0x01;
pub const DW_FORM_BLOCK2: u64 = 0x03;
pub const DW_FORM_BLOCK4: u64 = 0x04;
pub const DW_FORM_DATA2: u64 = 0x05;
pub const DW_FORM_DATA4: u64 = 0x06;
pub const DW_FORM_DATA8: u64 = 0x07;
pub const DW_FORM_STRING: u64 = 0x08;
pub const DW_FORM_BLOCK: u64 = 0x09;
pub const DW_FORM_BLOCK1: u64 = 0x0a;
pub const DW_FORM_DATA1: u64 = 0x0b;
pub const DW_FORM_FLAG: u64 = 0x0c;
pub const DW_FORM_SDATA: u64 = 0x0d;
pub const DW_FORM_STRP: u64 = 0x0e;
pub const DW_FORM_UDATA: u64 = 0x0f;
pub const DW_FORM_REF_ADDR: u64 = 0x10;
pub const DW_FORM_REF1: u64 = 0x11;
pub const DW_FORM_REF2: u64 = 0x12;
pub const DW_FORM_REF4: u64 = 0x13;
pub const DW_FORM_REF8: u64 = 0x14;
pub const DW_FORM_REF_UDATA: u64 = 0x15;
pub const DW_FORM_INDIRECT: u64 = 0x16;
pub const DW_FORM_SEC_OFFSET: u64 = 0x17;
pub const DW_FORM_EXPRLOC: u64 = 0x18;
pub const DW_FORM_FLAG_PRESENT: u64 = 0x19;
pub const DW_FORM_STRX: u64 = 0x1a;
pub const DW_FORM_ADDRX: u64 = 0x1b;
pub const DW_FORM_REF_SUP4: u64 = 0x1c;
pub const DW_FORM_STRP_SUP: u64 = 0x1d;
pub const DW_FORM_DATA16: u64 = 0x1e;
pub const DW_FORM_LINE_STRP: u64 = 0x1f;
pub const DW_FORM_REF_SIG8: u64 = 0x20;
pub const DW_FORM_IMPLICIT_CONST: u64 = 0x21;
pub const DW_FORM_LOCLISTX: u64 = 0x22;
pub const DW_FORM_RNGLISTX: u64 = 0x23;
pub const DW_FORM_REF_SUP8: u64 = 0x24;
pub const DW_FORM_STRX1: u64 = 0x25;
pub const DW_FORM_STRX2: u64 = 0x26;
pub const DW_FORM_STRX3: u64 = 0x27;
pub const DW_FORM_STRX4: u64 = 0x28;
pub const DW_FORM_ADDRX1: u64 = 0x29;
pub const DW_FORM_ADDRX2: u64 = 0x2a;
pub const DW_FORM_ADDRX3: u64 = 0x2b;
pub const DW_FORM_ADDRX4: u64 = 0x2c;
pub const DW_FORM_GNU_ADDR_INDEX: u64 = 0x1f01;
pub const DW_FORM_GNU_STR_INDEX: u64 = 0x1f02;
pub const DW_FORM_GNU_REF_ALT: u64 = 0x1f20;
pub const DW_FORM_GNU_STRP_ALT: u64 = 0x1f21;
/// The longest inline string read.
const MAX_STRING_BYTES: usize = 1 << 16;

/// What a unit's values are encoded with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encoding {
    pub version: u16,
    pub format: Format,
    pub address_size: u8,
}

/// One attribute value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    Address(u64),
    /// An index into `.debug_addr`, from the unit's `DW_AT_addr_base`.
    AddressIndex(u64),
    Unsigned(u64),
    Signed(i64),
    Block(&'a [u8]),
    /// An inline string (`DW_FORM_string`).
    String(&'a [u8]),
    /// An offset into `.debug_str`.
    StrOffset(u64),
    /// An offset into `.debug_line_str`.
    LineStrOffset(u64),
    /// An index into `.debug_str_offsets`, from `DW_AT_str_offsets_base`.
    StrIndex(u64),
    /// An offset into the supplementary file's `.debug_str`.
    SupStrOffset(u64),
    /// An entry offset from the unit's start.
    UnitRef(u64),
    /// An entry offset from `.debug_info`'s start.
    InfoRef(u64),
    /// An entry offset in the supplementary file's `.debug_info`.
    SupInfoRef(u64),
    /// A type unit's signature.
    Signature(u64),
    SecOffset(u64),
    RangeListIndex(u64),
    LocationListIndex(u64),
    Flag(u8),
    /// 16 bytes no answer this reader gives uses.
    Data16,
}

/// A value and where the next one starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded<'a> {
    pub value: Value<'a>,
    pub next: usize,
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn past(at: usize, by: usize) -> Result<usize, DwarfError> {
    at.checked_add(by).ok_or(DwarfError::TooMany { what: "section bytes", count: u64::MAX })
}

/// A little-endian unsigned value of `size` bytes (1 to 8).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn fixed(section: Bytes<'_>, at: usize, size: usize) -> Result<u64, DwarfError> {
    let bytes = section.slice(at, size).map_err(|source| short("attribute", source))?;
    let mut value = 0u64;
    for (index, byte) in bytes.iter().take(8).enumerate() {
        value |= u64::from(*byte).wrapping_shl((index as u32).wrapping_mul(8));
    }
    Ok(value)
}

/// A fixed-size value made into a [`Value`] by `make`.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn sized<'a>(section: Bytes<'a>, at: usize, size: usize, make: fn(u64) -> Value<'a>) -> Result<Decoded<'a>, DwarfError> {
    Ok(Decoded { value: make(fixed(section, at, size)?), next: past(at, size)? })
}

/// An unsigned LEB128 value made into a [`Value`] by `make`.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn leb<'a>(section: Bytes<'a>, at: usize, make: fn(u64) -> Value<'a>) -> Result<Decoded<'a>, DwarfError> {
    let leb = section.uleb128(at).map_err(|source| short("attribute", source))?;
    Ok(Decoded { value: make(leb.value), next: leb.next })
}

/// A block of `len` bytes at `at`.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn block(section: Bytes<'_>, at: usize, len: u64) -> Result<Decoded<'_>, DwarfError> {
    let len = usize::try_from(len).map_err(|_| DwarfError::TooMany { what: "block bytes", count: len })?;
    let bytes = section.slice(at, len).map_err(|source| short("block", source))?;
    Ok(Decoded { value: Value::Block(bytes), next: past(at, len)? })
}

/// The value of `form` at `at`; `implicit` is the abbreviation's constant
/// for `DW_FORM_implicit_const`. An indirect form names its form in the
/// data; one indirection is followed, a second is refused.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_value<'a>(section: Bytes<'a>, at: usize, form: u64, implicit: i64, unit: &Encoding) -> Result<Decoded<'a>, DwarfError> {
    if form == DW_FORM_INDIRECT {
        let named = section.uleb128(at).map_err(|source| short("indirect form", source))?;
        return decode_direct(section, named.next, named.value, implicit, unit);
    }
    decode_direct(section, at, form, implicit, unit)
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn decode_direct<'a>(section: Bytes<'a>, at: usize, form: u64, implicit: i64, unit: &Encoding) -> Result<Decoded<'a>, DwarfError> {
    let offset_size = unit.format.offset_size();
    let address_size = usize::from(unit.address_size);
    match form {
        DW_FORM_ADDR => sized(section, at, address_size, Value::Address),
        DW_FORM_DATA1 => sized(section, at, 1, Value::Unsigned),
        DW_FORM_DATA2 => sized(section, at, 2, Value::Unsigned),
        DW_FORM_DATA4 => sized(section, at, 4, Value::Unsigned),
        DW_FORM_DATA8 => sized(section, at, 8, Value::Unsigned),
        DW_FORM_DATA16 => {
            section.slice(at, 16).map_err(|source| short("attribute", source))?;
            Ok(Decoded { value: Value::Data16, next: past(at, 16)? })
        }
        DW_FORM_SDATA => {
            let leb = section.sleb128(at).map_err(|source| short("attribute", source))?;
            Ok(Decoded { value: Value::Signed(leb.value), next: leb.next })
        }
        DW_FORM_UDATA => leb(section, at, Value::Unsigned),
        DW_FORM_FLAG => sized(section, at, 1, |value| Value::Flag(value.to_le_bytes()[0])),
        DW_FORM_FLAG_PRESENT => Ok(Decoded { value: Value::Flag(1), next: at }),
        DW_FORM_STRING => {
            let text = section.cstr(at, MAX_STRING_BYTES).map_err(|source| short("string", source))?;
            Ok(Decoded { value: Value::String(text), next: past(past(at, text.len())?, 1)? })
        }
        DW_FORM_STRP => sized(section, at, offset_size, Value::StrOffset),
        DW_FORM_LINE_STRP => sized(section, at, offset_size, Value::LineStrOffset),
        DW_FORM_STRP_SUP | DW_FORM_GNU_STRP_ALT => sized(section, at, offset_size, Value::SupStrOffset),
        DW_FORM_STRX | DW_FORM_GNU_STR_INDEX => leb(section, at, Value::StrIndex),
        DW_FORM_STRX1 => sized(section, at, 1, Value::StrIndex),
        DW_FORM_STRX2 => sized(section, at, 2, Value::StrIndex),
        DW_FORM_STRX3 => sized(section, at, 3, Value::StrIndex),
        DW_FORM_STRX4 => sized(section, at, 4, Value::StrIndex),
        DW_FORM_ADDRX | DW_FORM_GNU_ADDR_INDEX => leb(section, at, Value::AddressIndex),
        DW_FORM_ADDRX1 => sized(section, at, 1, Value::AddressIndex),
        DW_FORM_ADDRX2 => sized(section, at, 2, Value::AddressIndex),
        DW_FORM_ADDRX3 => sized(section, at, 3, Value::AddressIndex),
        DW_FORM_ADDRX4 => sized(section, at, 4, Value::AddressIndex),
        DW_FORM_REF1 => sized(section, at, 1, Value::UnitRef),
        DW_FORM_REF2 => sized(section, at, 2, Value::UnitRef),
        DW_FORM_REF4 => sized(section, at, 4, Value::UnitRef),
        DW_FORM_REF8 => sized(section, at, 8, Value::UnitRef),
        DW_FORM_REF_UDATA => leb(section, at, Value::UnitRef),
        // DWARF 2 made `ref_addr` address-sized; DWARF 3 offset-sized.
        DW_FORM_REF_ADDR => {
            let size = if unit.version <= 2 { address_size } else { offset_size };
            sized(section, at, size, Value::InfoRef)
        }
        DW_FORM_REF_SUP4 => sized(section, at, 4, Value::SupInfoRef),
        DW_FORM_REF_SUP8 => sized(section, at, 8, Value::SupInfoRef),
        DW_FORM_GNU_REF_ALT => sized(section, at, offset_size, Value::SupInfoRef),
        DW_FORM_REF_SIG8 => sized(section, at, 8, Value::Signature),
        DW_FORM_SEC_OFFSET => sized(section, at, offset_size, Value::SecOffset),
        DW_FORM_RNGLISTX => leb(section, at, Value::RangeListIndex),
        DW_FORM_LOCLISTX => leb(section, at, Value::LocationListIndex),
        DW_FORM_EXPRLOC | DW_FORM_BLOCK => {
            let len = section.uleb128(at).map_err(|source| short("block", source))?;
            block(section, len.next, len.value)
        }
        DW_FORM_BLOCK1 => block(section, past(at, 1)?, fixed(section, at, 1)?),
        DW_FORM_BLOCK2 => block(section, past(at, 2)?, fixed(section, at, 2)?),
        DW_FORM_BLOCK4 => block(section, past(at, 4)?, fixed(section, at, 4)?),
        DW_FORM_IMPLICIT_CONST => Ok(Decoded { value: Value::Signed(implicit), next: at }),
        // Includes DW_FORM_indirect met again: one indirection only.
        other => Err(DwarfError::UnknownForm { form: other }),
    }
}
