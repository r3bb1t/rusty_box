//! C13 debug subsections in a module stream (LLVM, "The Module Information
//! Stream", "C13 Line Information"): `DEBUG_S_LINES` maps code offsets to
//! line numbers per source file; `DEBUG_S_FILECHKSMS` names each file by an
//! offset into the `/names` stream.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::read::{Bytes, ReadError};

pub const DEBUG_S_LINES: u32 = 0xF2;
pub const DEBUG_S_FILECHKSMS: u32 = 0xF4;
/// A line entry's line number: its low 24 bits.
const LINE_MASK: u32 = 0x00FF_FFFF;
const LINE_ENTRY_BYTES: usize = 8;
const BLOCK_HEADER_BYTES: usize = 12;

/// A subsection, and where the next starts (4-aligned).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subsection<'a> {
    pub kind: u32,
    pub data: Bytes<'a>,
    pub next: usize,
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_subsection(c13: Bytes<'_>, at: usize) -> Result<Subsection<'_>, ReadError> {
    let kind = c13.u32_le(at)?;
    let len = c13.u32_le(at.checked_add(4).ok_or(ReadError::Truncated { at, len: 8 })?)? as usize;
    let data_at = at.checked_add(8).ok_or(ReadError::Truncated { at, len: 8 })?;
    let data = c13.sub(data_at, len)?;
    let end = data_at.checked_add(len).ok_or(ReadError::Truncated { at, len })?;
    let next = end.checked_add(3).map(|end| end & !3).ok_or(ReadError::Truncated { at, len })?;
    Ok(Subsection { kind, data, next })
}

/// A lines subsection's head: the code it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinesHead {
    pub offset: u32,
    pub segment: u16,
    pub size: u32,
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_lines_head(data: Bytes<'_>) -> Result<LinesHead, ReadError> {
    Ok(LinesHead {
        offset: data.u32_le(0)?,
        segment: data.u16_le(4)?,
        size: data.u32_le(8)?,
    })
}

/// One file's block of line entries, and where the next block starts.
/// Column entries, when present, lie inside `block_bytes` and are skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineBlock<'a> {
    pub file: u32,
    pub entries: Bytes<'a>,
    pub count: u32,
    pub next: usize,
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_block(data: Bytes<'_>, at: usize) -> Result<LineBlock<'_>, ReadError> {
    let field = |offset: usize| at.checked_add(offset).ok_or(ReadError::Truncated { at, len: offset });
    let file = data.u32_le(at)?;
    let count = data.u32_le(field(4)?)?;
    let block_bytes = data.u32_le(field(8)?)? as usize;
    if block_bytes < BLOCK_HEADER_BYTES {
        return Err(ReadError::Truncated { at, len: BLOCK_HEADER_BYTES });
    }
    let entries_len = (count as usize)
        .checked_mul(LINE_ENTRY_BYTES)
        .ok_or(ReadError::Truncated { at, len: usize::MAX })?;
    let entries = data.sub(field(BLOCK_HEADER_BYTES)?, entries_len)?;
    Ok(LineBlock { file, entries, count, next: field(block_bytes)? })
}

/// Line entry `index`: its code offset and line number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEntry {
    pub offset: u32,
    pub line: u32,
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_line(entries: Bytes<'_>, index: u32) -> Result<LineEntry, ReadError> {
    let at = (index as usize)
        .checked_mul(LINE_ENTRY_BYTES)
        .ok_or(ReadError::Truncated { at: usize::MAX, len: LINE_ENTRY_BYTES })?;
    Ok(LineEntry {
        offset: entries.u32_le(at)?,
        line: entries.u32_le(at.checked_add(4).ok_or(ReadError::Truncated { at, len: 8 })?)? & LINE_MASK,
    })
}

/// The `/names` offset of the file a block names by its checksum entry.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn checksum_name(checksums: Bytes<'_>, file: u32) -> Result<u32, ReadError> {
    checksums.u32_le(file as usize)
}

/// A string of the `/names` stream: a header (signature 0xEFFEEFFE,
/// version, buffer size), then the buffer the offsets point into.
#[must_use]
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn names_string(names: Bytes<'_>, offset: u32) -> Option<&[u8]> {
    if names.u32_le(0).ok()? != 0xEFFE_EFFE {
        return None;
    }
    let size = names.u32_le(8).ok()? as usize;
    names.sub(12, size).ok()?.cstr(offset as usize, 4096).ok()
}
