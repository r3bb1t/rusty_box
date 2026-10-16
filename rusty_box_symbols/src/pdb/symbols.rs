//! CodeView symbol records (LLVM, "CodeView Symbol Records"; `cvinfo.h`):
//! a 16-bit length (not counting itself), a 16-bit kind, then the kind's
//! fields. The decoders here read the records that name addresses.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::read::{Bytes, ReadError};

pub const S_LDATA32: u16 = 0x110C;
pub const S_GDATA32: u16 = 0x110D;
pub const S_PUB32: u16 = 0x110E;
pub const S_LPROC32: u16 = 0x110F;
pub const S_GPROC32: u16 = 0x1110;
pub const S_LPROC32_ID: u16 = 0x1146;
pub const S_GPROC32_ID: u16 = 0x1147;
const MAX_NAME_BYTES: usize = 4096;

/// A record's kind, its fields, and where the next record starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordHeader<'a> {
    pub kind: u16,
    pub data: Bytes<'a>,
    pub next: usize,
}

/// The record at `at`. A length below 2 cannot hold its kind, and is
/// refused, so a walk always moves forward.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_record_header(records: Bytes<'_>, at: usize) -> Result<RecordHeader<'_>, ReadError> {
    let len = usize::from(records.u16_le(at)?);
    let after_length = at.checked_add(2).ok_or(ReadError::Truncated { at, len: 2 })?;
    let data_len = len.checked_sub(2).ok_or(ReadError::Truncated { at, len: 4 })?;
    let kind = records.u16_le(after_length)?;
    let data_at = after_length.checked_add(2).ok_or(ReadError::Truncated { at, len: 4 })?;
    let data = records.sub(data_at, data_len)?;
    let next = after_length.checked_add(len).ok_or(ReadError::Truncated { at, len })?;
    Ok(RecordHeader { kind, data, next })
}

/// A named address: `S_PUB32` (flags, offset, segment, name) and
/// `S_GDATA32`/`S_LDATA32` (type, offset, segment, name) share the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedAddress<'a> {
    pub offset: u32,
    pub segment: u16,
    pub name: &'a [u8],
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_named_address(data: Bytes<'_>) -> Result<NamedAddress<'_>, ReadError> {
    Ok(NamedAddress {
        offset: data.u32_le(4)?,
        segment: data.u16_le(8)?,
        name: data.cstr(10, MAX_NAME_BYTES)?,
    })
}

/// `S_GPROC32`/`S_LPROC32` and their `_ID` forms: parent, end, next,
/// length, debug start and end, type, offset, segment, flags, name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcedureRecord<'a> {
    pub len: u32,
    pub offset: u32,
    pub segment: u16,
    pub name: &'a [u8],
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_procedure(data: Bytes<'_>) -> Result<ProcedureRecord<'_>, ReadError> {
    Ok(ProcedureRecord {
        len: data.u32_le(12)?,
        offset: data.u32_le(28)?,
        segment: data.u16_le(32)?,
        name: data.cstr(35, MAX_NAME_BYTES)?,
    })
}
