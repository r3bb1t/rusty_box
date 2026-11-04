//! An entry's address ranges: `.debug_ranges` (DWARF 4 §2.17.3: address
//! pairs relative to the unit's base, a base-selection entry, a 0/0 end)
//! and `.debug_rnglists` (DWARF 5 §2.17.3 and §7.25: typed entries, an
//! offset table indexed by `DW_FORM_rnglistx`). Each list is bounded by
//! `MAX_RANGES`, and every entry consumes bytes, so a walk ends.

use super::forms::Encoding;
use super::{push, short, DwarfError, Format};
use crate::read::{Bytes, Cursor};

const DW_RLE_END_OF_LIST: u8 = 0;
const DW_RLE_BASE_ADDRESSX: u8 = 1;
const DW_RLE_STARTX_ENDX: u8 = 2;
const DW_RLE_STARTX_LENGTH: u8 = 3;
const DW_RLE_OFFSET_PAIR: u8 = 4;
const DW_RLE_BASE_ADDRESS: u8 = 5;
const DW_RLE_START_END: u8 = 6;
const DW_RLE_START_LENGTH: u8 = 7;
/// Ranges of one entry; a function split by hot/cold placement has a few.
const MAX_RANGES: usize = 4096;

/// `[start, end)`, at link-time addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

/// An address of `size` bytes (4 or 8).
fn address(cursor: &mut Cursor<'_>, size: u8) -> Result<u64, DwarfError> {
    match size {
        4 => cursor.u32_le().map(u64::from).map_err(|source| short("range list", source)),
        _ => cursor.u64_le().map_err(|source| short("range list", source)),
    }
}

/// Adds `[start, end)` when it is not empty.
fn add(ranges: &mut Vec<Range>, start: u64, end: u64) -> Result<(), DwarfError> {
    if ranges.len() >= MAX_RANGES {
        return Err(DwarfError::TooMany { what: "ranges", count: ranges.len() as u64 });
    }
    if start < end {
        push(ranges, Range { start, end }, "ranges")?;
    }
    Ok(())
}

/// The `.debug_ranges` list at `at`, relative to `base` (the unit's low
/// address) until a base-selection entry changes it.
pub fn debug_ranges(section: Bytes<'_>, at: usize, address_size: u8, base: u64) -> Result<Vec<Range>, DwarfError> {
    let selection = if address_size == 4 { u64::from(u32::MAX) } else { u64::MAX };
    let mut cursor = Cursor::new(section, at);
    let mut base = base;
    let mut ranges = Vec::new();
    loop {
        let begin = address(&mut cursor, address_size)?;
        let end = address(&mut cursor, address_size)?;
        if begin == 0 && end == 0 {
            return Ok(ranges);
        }
        if begin == selection {
            base = end;
            continue;
        }
        add(&mut ranges, base.wrapping_add(begin), base.wrapping_add(end))?;
    }
}

/// The offset of list `index` of a unit whose `DW_AT_rnglists_base` is
/// `base`: the offset table's entry, relative to `base`.
pub fn rnglist_offset(section: Bytes<'_>, base: u64, index: u64, format: Format) -> Result<usize, DwarfError> {
    let size = format.offset_size() as u64;
    let entry = index
        .checked_mul(size)
        .and_then(|offset| offset.checked_add(base))
        .and_then(|at| usize::try_from(at).ok())
        .ok_or(DwarfError::BadReference { offset: base })?;
    let relative = match format {
        Format::Dwarf32 => u64::from(section.u32_le(entry).map_err(|source| short("range-list offsets", source))?),
        Format::Dwarf64 => section.u64_le(entry).map_err(|source| short("range-list offsets", source))?,
    };
    relative
        .checked_add(base)
        .and_then(|at| usize::try_from(at).ok())
        .ok_or(DwarfError::BadReference { offset: relative })
}

/// The `.debug_rnglists` list at `at`; `address` resolves an index into
/// `.debug_addr`.
pub fn debug_rnglists<A>(section: Bytes<'_>, at: usize, encoding: &Encoding, base: u64, address_at: A) -> Result<Vec<Range>, DwarfError>
where
    A: Fn(u64) -> Option<u64>,
{
    let mut cursor = Cursor::new(section, at);
    let mut base = base;
    let mut ranges = Vec::new();
    let indexed = |index: u64| address_at(index).ok_or(DwarfError::BadReference { offset: index });
    loop {
        let kind = cursor.u8().map_err(|source| short("range list", source))?;
        let leb = |cursor: &mut Cursor<'_>| cursor.uleb128().map_err(|source| short("range list", source));
        match kind {
            DW_RLE_END_OF_LIST => return Ok(ranges),
            DW_RLE_BASE_ADDRESSX => base = indexed(leb(&mut cursor)?)?,
            DW_RLE_STARTX_ENDX => {
                let start = indexed(leb(&mut cursor)?)?;
                let end = indexed(leb(&mut cursor)?)?;
                add(&mut ranges, start, end)?;
            }
            DW_RLE_STARTX_LENGTH => {
                let start = indexed(leb(&mut cursor)?)?;
                let length = leb(&mut cursor)?;
                add(&mut ranges, start, start.wrapping_add(length))?;
            }
            DW_RLE_OFFSET_PAIR => {
                let start = leb(&mut cursor)?;
                let end = leb(&mut cursor)?;
                add(&mut ranges, base.wrapping_add(start), base.wrapping_add(end))?;
            }
            DW_RLE_BASE_ADDRESS => base = address(&mut cursor, encoding.address_size)?,
            DW_RLE_START_END => {
                let start = address(&mut cursor, encoding.address_size)?;
                let end = address(&mut cursor, encoding.address_size)?;
                add(&mut ranges, start, end)?;
            }
            DW_RLE_START_LENGTH => {
                let start = address(&mut cursor, encoding.address_size)?;
                let length = leb(&mut cursor)?;
                add(&mut ranges, start, start.wrapping_add(length))?;
            }
            other => return Err(DwarfError::UnknownRangeEntry { kind: other }),
        }
    }
}
