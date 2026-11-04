//! CodeView symbol records (LLVM, "CodeView Symbol Records"; `cvinfo.h`):
//! a 16-bit length (not counting itself), a 16-bit kind, then the kind's
//! fields. The decoders here read the records that name addresses, and the
//! records of a function's scope: its blocks, variables and def-ranges, with
//! the fields LLVM's `SymbolRecordMapping.cpp` maps, in its order.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::read::{Bytes, ReadError};

pub const S_END: u16 = 0x0006;
pub const S_FRAMEPROC: u16 = 0x1012;
pub const S_BLOCK32: u16 = 0x1103;
pub const S_REGISTER: u16 = 0x1106;
pub const S_BPREL32: u16 = 0x110B;
pub const S_LDATA32: u16 = 0x110C;
pub const S_GDATA32: u16 = 0x110D;
pub const S_PUB32: u16 = 0x110E;
pub const S_LPROC32: u16 = 0x110F;
pub const S_GPROC32: u16 = 0x1110;
pub const S_REGREL32: u16 = 0x1111;
pub const S_LOCAL: u16 = 0x113E;
pub const S_DEFRANGE_REGISTER: u16 = 0x1141;
pub const S_DEFRANGE_FRAMEPOINTER_REL: u16 = 0x1142;
pub const S_DEFRANGE_SUBFIELD_REGISTER: u16 = 0x1143;
pub const S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE: u16 = 0x1144;
pub const S_DEFRANGE_REGISTER_REL: u16 = 0x1145;
pub const S_LPROC32_ID: u16 = 0x1146;
pub const S_GPROC32_ID: u16 = 0x1147;
pub const S_INLINESITE: u16 = 0x114D;
pub const S_INLINESITE_END: u16 = 0x114E;
pub const S_PROC_ID_END: u16 = 0x114F;
const MAX_NAME_BYTES: usize = 4096;
/// `S_DEFRANGE_SUBFIELD_REGISTER`'s offset in its parent: the low 12 bits.
const OFFSET_IN_PARENT_MASK: u32 = 0xFFF;

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
    /// Where the prolog ends, and the epilog starts, from the start.
    pub debug_start: u32,
    pub debug_end: u32,
    /// The function's type: `LF_PROCEDURE` or `LF_MFUNCTION`, or 0.
    pub ty: u32,
    pub offset: u32,
    pub segment: u16,
    pub name: &'a [u8],
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_procedure(data: Bytes<'_>) -> Result<ProcedureRecord<'_>, ReadError> {
    Ok(ProcedureRecord {
        len: data.u32_le(12)?,
        debug_start: data.u32_le(16)?,
        debug_end: data.u32_le(20)?,
        ty: data.u32_le(24)?,
        offset: data.u32_le(28)?,
        segment: data.u16_le(32)?,
        name: data.cstr(35, MAX_NAME_BYTES)?,
    })
}

/// `S_BLOCK32`: parent, end, code size, code offset, segment, name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockRecord<'a> {
    pub len: u32,
    pub offset: u32,
    pub segment: u16,
    pub name: &'a [u8],
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_block(data: Bytes<'_>) -> Result<BlockRecord<'_>, ReadError> {
    Ok(BlockRecord {
        len: data.u32_le(8)?,
        offset: data.u32_le(12)?,
        segment: data.u16_le(16)?,
        name: data.cstr(18, MAX_NAME_BYTES)?,
    })
}

/// A variable named by a register, an offset and a type: `S_REGISTER` (no
/// offset), `S_BPREL32` (no register) and `S_REGREL32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VariableRecord<'a> {
    pub ty: u32,
    pub register: u16,
    pub offset: i32,
    pub name: &'a [u8],
}

/// `S_REGISTER`: type, register, name.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_register_variable(data: Bytes<'_>) -> Result<VariableRecord<'_>, ReadError> {
    Ok(VariableRecord { ty: data.u32_le(0)?, register: data.u16_le(4)?, offset: 0, name: data.cstr(6, MAX_NAME_BYTES)? })
}

/// `S_BPREL32`: offset, type, name; its register is the frame pointer.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_bp_relative(data: Bytes<'_>) -> Result<VariableRecord<'_>, ReadError> {
    Ok(VariableRecord {
        ty: data.u32_le(4)?,
        register: 0,
        offset: data.u32_le(0)? as i32,
        name: data.cstr(8, MAX_NAME_BYTES)?,
    })
}

/// `S_REGREL32`: offset, type, register, name.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_register_relative(data: Bytes<'_>) -> Result<VariableRecord<'_>, ReadError> {
    Ok(VariableRecord {
        ty: data.u32_le(4)?,
        register: data.u16_le(8)?,
        offset: data.u32_le(0)? as i32,
        name: data.cstr(10, MAX_NAME_BYTES)?,
    })
}

/// `S_FRAMEPROC`'s flags: five sizes and offsets, the handler's section,
/// then the flags.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_frame_procedure(data: Bytes<'_>) -> Result<u32, ReadError> {
    data.u32_le(22)
}

/// `S_LOCAL`: type, flags (`0x01` a parameter, `0x100` optimized out), name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalRecord<'a> {
    pub ty: u32,
    pub flags: u16,
    pub name: &'a [u8],
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_local(data: Bytes<'_>) -> Result<LocalRecord<'_>, ReadError> {
    Ok(LocalRecord { ty: data.u32_le(0)?, flags: data.u16_le(4)?, name: data.cstr(6, MAX_NAME_BYTES)? })
}

/// `S_LDATA32`/`S_GDATA32`: type, offset, segment, name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataRecord<'a> {
    pub ty: u32,
    pub offset: u32,
    pub segment: u16,
    pub name: &'a [u8],
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_data(data: Bytes<'_>) -> Result<DataRecord<'_>, ReadError> {
    Ok(DataRecord {
        ty: data.u32_le(0)?,
        offset: data.u32_le(4)?,
        segment: data.u16_le(8)?,
        name: data.cstr(10, MAX_NAME_BYTES)?,
    })
}

/// Where a def-range puts its variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefPlace {
    Register { register: u16 },
    FramePointerRel { offset: i32 },
    Subfield { register: u16, offset_in_parent: u32 },
    RegisterRel { register: u16, offset: i32 },
    FullScope { offset: i32 },
}

/// A def-range's code: `len` bytes at `segment:offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefRangeSpan {
    pub offset: u32,
    pub segment: u16,
    pub len: u16,
}

/// A gap: `len` bytes from `start` past the range's start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefGap {
    pub start: u16,
    pub len: u16,
}

/// An `S_DEFRANGE_*` record: its place, its range (none for the full-scope
/// form), and its gaps, four bytes each to the record's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefRange<'a> {
    pub place: DefPlace,
    pub range: Option<DefRangeSpan>,
    gaps: Bytes<'a>,
}

impl DefRange<'_> {
    #[must_use]
    pub fn gap_count(&self) -> usize {
        self.gaps.len().checked_div(4).unwrap_or(0)
    }

    /// Gap `index`, when there is one.
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    #[must_use]
    pub fn gap(&self, index: usize) -> Option<DefGap> {
        let at = index.checked_mul(4)?;
        Some(DefGap { start: self.gaps.u16_le(at).ok()?, len: self.gaps.u16_le(at.checked_add(2)?).ok()? })
    }
}

/// The def-range of record `kind` (one of the five `S_DEFRANGE_*` kinds this
/// reader knows; another kind is refused).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_def_range(kind: u16, data: Bytes<'_>) -> Result<DefRange<'_>, ReadError> {
    let refused = ReadError::Truncated { at: 0, len: 0 };
    let place = match kind {
        S_DEFRANGE_REGISTER => DefPlace::Register { register: data.u16_le(0)? },
        S_DEFRANGE_FRAMEPOINTER_REL => DefPlace::FramePointerRel { offset: data.u32_le(0)? as i32 },
        S_DEFRANGE_SUBFIELD_REGISTER => {
            DefPlace::Subfield { register: data.u16_le(0)?, offset_in_parent: data.u32_le(4)? & OFFSET_IN_PARENT_MASK }
        }
        S_DEFRANGE_REGISTER_REL => DefPlace::RegisterRel { register: data.u16_le(0)?, offset: data.u32_le(4)? as i32 },
        S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE => {
            let place = DefPlace::FullScope { offset: data.u32_le(0)? as i32 };
            return Ok(DefRange { place, range: None, gaps: Bytes::new(&[]) });
        }
        _ => return Err(refused),
    };
    // The range follows the place's header: 8 bytes for the subfield and
    // register-relative forms, 4 for the others.
    let header: usize = match place {
        DefPlace::Subfield { .. } | DefPlace::RegisterRel { .. } => 8,
        DefPlace::Register { .. } | DefPlace::FramePointerRel { .. } | DefPlace::FullScope { .. } => 4,
    };
    let range = DefRangeSpan {
        offset: data.u32_le(header)?,
        segment: data.u16_le(header.checked_add(4).ok_or(refused)?)?,
        len: data.u16_le(header.checked_add(6).ok_or(refused)?)?,
    };
    let gaps = data.tail(header.checked_add(8).ok_or(refused)?)?;
    Ok(DefRange { place, range: Some(range), gaps })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each decoder over its record's shortest whole form, then that cut by
    /// one byte, then nothing.
    #[test]
    fn each_scope_record_refuses_a_short_record() {
        let whole: [Vec<u8>; 7] = [
            [vec![0; 18], b"b\0".to_vec()].concat(), // S_BLOCK32
            [vec![0; 6], b"r\0".to_vec()].concat(),  // S_REGISTER
            [vec![0; 8], b"p\0".to_vec()].concat(),  // S_BPREL32
            [vec![0; 10], b"q\0".to_vec()].concat(), // S_REGREL32
            vec![0; 26],                             // S_FRAMEPROC
            [vec![0; 6], b"l\0".to_vec()].concat(),  // S_LOCAL
            [vec![0; 10], b"d\0".to_vec()].concat(), // S_LDATA32
        ];
        let decoders: [fn(Bytes<'_>) -> bool; 7] = [
            |data| decode_block(data).is_ok(),
            |data| decode_register_variable(data).is_ok(),
            |data| decode_bp_relative(data).is_ok(),
            |data| decode_register_relative(data).is_ok(),
            |data| decode_frame_procedure(data).is_ok(),
            |data| decode_local(data).is_ok(),
            |data| decode_data(data).is_ok(),
        ];
        for (index, decode) in decoders.iter().enumerate() {
            let bytes = &whole[index];
            assert!(decode(Bytes::new(bytes)), "record {index} whole");
            assert!(!decode(Bytes::new(&bytes[..bytes.len() - 1])), "record {index} cut");
            assert!(!decode(Bytes::new(&[])), "record {index} empty");
        }
        for kind in [S_DEFRANGE_REGISTER, S_DEFRANGE_FRAMEPOINTER_REL, S_DEFRANGE_SUBFIELD_REGISTER, S_DEFRANGE_REGISTER_REL] {
            assert!(decode_def_range(kind, Bytes::new(&[0; 4])).is_err(), "{kind:#x} cut");
        }
        assert!(decode_def_range(S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, Bytes::new(&[0; 3])).is_err());
        assert!(decode_def_range(0x1234, Bytes::new(&[0; 32])).is_err(), "not a def-range");
        let procedure = [vec![0; 35], b"f\0".to_vec()].concat();
        assert!(decode_procedure(Bytes::new(&procedure)).is_ok());
        assert!(decode_procedure(Bytes::new(&procedure[..20])).is_err());
    }

    #[test]
    fn a_def_range_reads_its_range_and_gaps() {
        let mut data = 0x14Fu16.to_le_bytes().to_vec(); // register
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0x118u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0x20u16.to_le_bytes());
        for gap in [[4u16, 2], [10, 1]] {
            data.extend_from_slice(&gap[0].to_le_bytes());
            data.extend_from_slice(&gap[1].to_le_bytes());
        }
        let def = decode_def_range(S_DEFRANGE_REGISTER, Bytes::new(&data)).expect("a def-range");
        assert_eq!(def.place, DefPlace::Register { register: 0x14F });
        assert_eq!(def.range, Some(DefRangeSpan { offset: 0x118, segment: 1, len: 0x20 }));
        assert_eq!(def.gap_count(), 2);
        assert_eq!(def.gap(1), Some(DefGap { start: 10, len: 1 }));
        assert_eq!(def.gap(2), None);
        let mut full = 0xFFFF_FFF8u32.to_le_bytes().to_vec();
        full.push(0);
        let def = decode_def_range(S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, Bytes::new(&full)).expect("full scope");
        assert_eq!(def.place, DefPlace::FullScope { offset: -8 });
        assert_eq!(def.range, None);
    }
}
