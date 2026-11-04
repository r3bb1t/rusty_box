//! Unit headers (DWARF 5 §7.5.1; §7.5.1.2 and Appendix F for versions 2
//! to 4, and `.debug_types` for version 4's type units).

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use super::{short, DwarfError, Format};
use crate::read::Bytes;

/// `unit_length` values from here up are reserved; 0xFFFFFFFF marks 64-bit
/// DWARF (§7.4).
const RESERVED_LENGTH: u32 = 0xFFFF_FFF0;
const DWARF64_MARK: u32 = 0xFFFF_FFFF;
/// §7.5.1.1 unit types.
const DW_UT_COMPILE: u8 = 0x01;
const DW_UT_TYPE: u8 = 0x02;
const DW_UT_PARTIAL: u8 = 0x03;
const DW_UT_SKELETON: u8 = 0x04;
const DW_UT_SPLIT_COMPILE: u8 = 0x05;
const DW_UT_SPLIT_TYPE: u8 = 0x06;

/// Which section a unit lies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitSection {
    /// `.debug_info` (and a supplementary file's).
    Info,
    /// Version 4's `.debug_types`: every unit is a type unit.
    Types,
}

/// What a unit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Compile,
    Partial,
    /// A type unit: the signature others name it by, and its type's entry
    /// offset from the unit's start.
    Type { signature: u64, type_offset: u64 },
    Skeleton,
    SplitCompile,
}

/// A unit's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitHeader {
    /// The unit's first byte in its section.
    pub offset: usize,
    /// One past its last byte.
    pub end: usize,
    pub version: u16,
    pub format: Format,
    pub address_size: u8,
    pub abbrev_offset: u64,
    pub kind: UnitKind,
    /// Its first entry.
    pub entries_at: usize,
}

/// Where a unit's header fields start, once its length is read.
#[derive(Clone, Copy)]
struct Length {
    format: Format,
    length: u64,
    header_at: usize,
}

/// Reads offset-sized fields of one unit.
#[derive(Clone, Copy)]
struct Fields<'a> {
    section: Bytes<'a>,
    format: Format,
}

impl Fields<'_> {
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn offset(self, at: usize) -> Result<u64, DwarfError> {
        match self.format {
            Format::Dwarf32 => self.section.u32_le(at).map(u64::from).map_err(|source| short("unit header", source)),
            Format::Dwarf64 => self.section.u64_le(at).map_err(|source| short("unit header", source)),
        }
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn u8(self, at: usize) -> Result<u8, DwarfError> {
        self.section.u8(at).map_err(|source| short("unit header", source))
    }

    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn u64(self, at: usize) -> Result<u64, DwarfError> {
        self.section.u64_le(at).map_err(|source| short("unit header", source))
    }
}

/// `at + by`, or the error a header past `usize` gives.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn past(at: usize, by: usize) -> Result<usize, DwarfError> {
    at.checked_add(by).ok_or(DwarfError::TooMany { what: "section bytes", count: u64::MAX })
}

/// The unit length (§7.4): 32-bit, or 0xFFFFFFFF and a 64-bit length.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn decode_length(section: Bytes<'_>, at: usize) -> Result<Length, DwarfError> {
    let first = section.u32_le(at).map_err(|source| short("unit header", source))?;
    if first == DWARF64_MARK {
        let length = section.u64_le(past(at, 4)?).map_err(|source| short("unit header", source))?;
        return Ok(Length { format: Format::Dwarf64, length, header_at: past(at, 12)? });
    }
    if first >= RESERVED_LENGTH {
        return Err(DwarfError::ReservedLength { at, value: first });
    }
    Ok(Length { format: Format::Dwarf32, length: u64::from(first), header_at: past(at, 4)? })
}

/// The unit header at `at` in `section`.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_unit_header(section: Bytes<'_>, at: usize, kind_of: UnitSection) -> Result<UnitHeader, DwarfError> {
    let length = decode_length(section, at)?;
    let size = usize::try_from(length.length).map_err(|_| DwarfError::TooMany { what: "unit bytes", count: length.length })?;
    let end = past(length.header_at, size)?;
    // The whole unit must lie in the section.
    section.slice(length.header_at, size).map_err(|source| short("unit", source))?;
    let version = section.u16_le(length.header_at).map_err(|source| short("unit header", source))?;
    if !(2..=5).contains(&version) {
        return Err(DwarfError::BadVersion { at, version });
    }
    let fields = Fields { section, format: length.format };
    let offset_size = length.format.offset_size();
    let after_version = past(length.header_at, 2)?;
    let mut header = UnitHeader {
        offset: at,
        end,
        version,
        format: length.format,
        address_size: 0,
        abbrev_offset: 0,
        kind: UnitKind::Compile,
        entries_at: 0,
    };
    let mut cursor;
    if version >= 5 {
        let unit_type = fields.u8(after_version)?;
        header.address_size = fields.u8(past(after_version, 1)?)?;
        header.abbrev_offset = fields.offset(past(after_version, 2)?)?;
        cursor = past(after_version, past(2, offset_size)?)?;
        header.kind = match unit_type {
            DW_UT_COMPILE => UnitKind::Compile,
            DW_UT_PARTIAL => UnitKind::Partial,
            DW_UT_TYPE | DW_UT_SPLIT_TYPE => {
                let signature = fields.u64(cursor)?;
                let type_offset = fields.offset(past(cursor, 8)?)?;
                cursor = past(cursor, past(8, offset_size)?)?;
                UnitKind::Type { signature, type_offset }
            }
            DW_UT_SKELETON => {
                cursor = past(cursor, 8)?;
                UnitKind::Skeleton
            }
            DW_UT_SPLIT_COMPILE => {
                cursor = past(cursor, 8)?;
                UnitKind::SplitCompile
            }
            other => return Err(DwarfError::UnknownUnitType { at, kind: other }),
        };
    } else {
        header.abbrev_offset = fields.offset(after_version)?;
        header.address_size = fields.u8(past(after_version, offset_size)?)?;
        cursor = past(after_version, past(offset_size, 1)?)?;
        header.kind = match kind_of {
            UnitSection::Info => UnitKind::Compile,
            UnitSection::Types => {
                let signature = fields.u64(cursor)?;
                let type_offset = fields.offset(past(cursor, 8)?)?;
                cursor = past(cursor, past(8, offset_size)?)?;
                UnitKind::Type { signature, type_offset }
            }
        };
    }
    if header.address_size != 4 && header.address_size != 8 {
        return Err(DwarfError::BadAddressSize { at, size: header.address_size });
    }
    header.entries_at = cursor;
    Ok(header)
}
