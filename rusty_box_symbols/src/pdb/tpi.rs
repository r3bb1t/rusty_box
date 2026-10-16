//! The TPI stream's type records (LLVM, "The PDB TPI and IPI Streams";
//! `cvinfo.h`), read for structure layouts. Indices start at the header's
//! `TypeIndexBegin`; a member's type may be a forward reference, resolved
//! to the definition through its unique name (else its name), as MSVC's
//! debuggers resolve it.
//!
//! Tier P: [`decode_numeric`], [`decode_udt`] and [`decode_field`] decode
//! one record or field-list entry. Tier A: [`TypeTables`] indexes the
//! records once and walks them per question. Every walk is bounded: field
//! lists by `MAX_FIELD_LIST_HOPS`, type chains by `MAX_TYPE_HOPS`, member
//! paths by their length.

use std::collections::BTreeMap;

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::read::{Bytes, ReadError};
use crate::tables::{FieldLayout, LayoutError, MemberLocation, TypeLayout};
use crate::units::{BitRange, FieldOffset};

const LF_MODIFIER: u16 = 0x1001;
const LF_POINTER: u16 = 0x1002;
const LF_FIELDLIST: u16 = 0x1203;
const LF_BITFIELD: u16 = 0x1205;
const LF_BCLASS: u16 = 0x1400;
const LF_VBCLASS: u16 = 0x1401;
const LF_IVBCLASS: u16 = 0x1402;
const LF_INDEX: u16 = 0x1404;
const LF_VFUNCTAB: u16 = 0x1409;
const LF_VFUNCOFF: u16 = 0x140C;
const LF_ENUMERATE: u16 = 0x1502;
const LF_ARRAY: u16 = 0x1503;
const LF_CLASS: u16 = 0x1504;
const LF_STRUCTURE: u16 = 0x1505;
const LF_UNION: u16 = 0x1506;
const LF_ENUM: u16 = 0x1507;
const LF_MEMBER: u16 = 0x150D;
const LF_STMEMBER: u16 = 0x150E;
const LF_METHOD: u16 = 0x150F;
const LF_NESTTYPE: u16 = 0x1510;
const LF_ONEMETHOD: u16 = 0x1511;
const LF_NESTTYPEEX: u16 = 0x1512;
const LF_INTERFACE: u16 = 0x1519;
const LF_BINTERFACE: u16 = 0x151A;
const LF_CLASS2: u16 = 0x1608;
const LF_STRUCTURE2: u16 = 0x1609;
const LF_UNION2: u16 = 0x160A;
const LF_INTERFACE2: u16 = 0x160B;
/// Numeric leaves (`cvinfo.h` `LF_NUMERIC` and after).
const LF_CHAR: u16 = 0x8000;
const LF_SHORT: u16 = 0x8001;
const LF_USHORT: u16 = 0x8002;
const LF_LONG: u16 = 0x8003;
const LF_ULONG: u16 = 0x8004;
const LF_QUADWORD: u16 = 0x8009;
const LF_UQUADWORD: u16 = 0x800A;
/// UDT properties: `fwdref` and `hasuniquename`.
const PROPERTY_FORWARD: u32 = 1 << 7;
const PROPERTY_UNIQUE_NAME: u32 = 1 << 9;
/// `CV_methodprop_e`: an introducing (and pure introducing) virtual method
/// carries a vtable offset.
const METHOD_INTRO: u16 = 4;
const METHOD_PURE_INTRO: u16 = 6;
/// The first non-primitive type index.
const FIRST_RECORD: u32 = 0x1000;
/// A TPI stream's header (LLVM's `TpiStreamHeader`).
const HEADER_BYTES: usize = 56;
/// Type records past this are not a real PDB's (a browser's has a few
/// million).
const MAX_TYPES: u32 = 1 << 24;
/// `LF_INDEX` hops in one field list; a real list of thousands of members
/// takes a few hundred.
const MAX_FIELD_LIST_HOPS: u32 = 4096;
/// Modifier, enum and forward-reference hops from a member to its type.
const MAX_TYPE_HOPS: u32 = 64;
const MAX_NAME_BYTES: usize = 4096;

/// A numeric leaf: the value when it is a non-negative integer, and where
/// the bytes after it start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Numeric {
    pub value: Option<u64>,
    pub next: usize,
}

/// A numeric leaf (`cvinfo.h`, "Numeric leaves"): below 0x8000 the value
/// itself, else a leaf naming the value's width.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_numeric(bytes: Bytes<'_>, at: usize) -> Result<Numeric, ReadError> {
    let leaf = bytes.u16_le(at)?;
    let after = at.checked_add(2).ok_or(ReadError::Truncated { at, len: 2 })?;
    if leaf < LF_CHAR {
        return Ok(Numeric { value: Some(u64::from(leaf)), next: after });
    }
    let past = |len: usize| after.checked_add(len).ok_or(ReadError::Truncated { at, len });
    Ok(match leaf {
        LF_CHAR => {
            let value = bytes.u8(after)? as i8;
            Numeric { value: u64::try_from(value).ok(), next: past(1)? }
        }
        LF_SHORT => {
            let value = bytes.u16_le(after)? as i16;
            Numeric { value: u64::try_from(value).ok(), next: past(2)? }
        }
        LF_USHORT => Numeric { value: Some(u64::from(bytes.u16_le(after)?)), next: past(2)? },
        LF_LONG => {
            let value = bytes.u32_le(after)? as i32;
            Numeric { value: u64::try_from(value).ok(), next: past(4)? }
        }
        LF_ULONG => Numeric { value: Some(u64::from(bytes.u32_le(after)?)), next: past(4)? },
        LF_QUADWORD => {
            let value = bytes.u64_le(after)? as i64;
            Numeric { value: u64::try_from(value).ok(), next: past(8)? }
        }
        LF_UQUADWORD => Numeric { value: Some(bytes.u64_le(after)?), next: past(8)? },
        // A real or other non-integer leaf: no value this reader uses, and
        // no width it knows, so the record cannot be walked past it.
        _ => return Err(ReadError::Truncated { at, len: usize::MAX }),
    })
}

/// A structure, class, interface or union record, in either form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UdtRecord<'a> {
    pub property: u32,
    pub field_list: u32,
    pub size: Option<u64>,
    pub name: &'a [u8],
    /// The decorated name, when the record carries one.
    pub unique: Option<&'a [u8]>,
}

/// Whether `kind` is a UDT record kind this reader reads.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn is_udt(kind: u16) -> bool {
    matches!(
        kind,
        LF_CLASS | LF_STRUCTURE | LF_INTERFACE | LF_UNION | LF_CLASS2 | LF_STRUCTURE2 | LF_UNION2 | LF_INTERFACE2
    )
}

/// A UDT record of `kind` (`lfClass`/`lfUnion`; the `*2` forms carry a
/// 32-bit property and the member count after the vtable shape).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_udt(kind: u16, data: Bytes<'_>) -> Result<UdtRecord<'_>, ReadError> {
    let head = match kind {
        LF_CLASS | LF_STRUCTURE | LF_INTERFACE => UdtHead {
            property: u32::from(data.u16_le(2)?),
            field_list: data.u32_le(4)?,
            size_at: 16,
        },
        LF_UNION => UdtHead { property: u32::from(data.u16_le(2)?), field_list: data.u32_le(4)?, size_at: 8 },
        LF_CLASS2 | LF_STRUCTURE2 | LF_UNION2 | LF_INTERFACE2 => UdtHead {
            property: data.u32_le(0)?,
            field_list: data.u32_le(4)?,
            size_at: 18,
        },
        _ => return Err(ReadError::Truncated { at: 0, len: 0 }),
    };
    let size = decode_numeric(data, head.size_at)?;
    let name = data.cstr(size.next, MAX_NAME_BYTES)?;
    let unique = if head.property & PROPERTY_UNIQUE_NAME != 0 {
        let unique_at = size.next.checked_add(name.len()).and_then(|at| at.checked_add(1));
        unique_at.and_then(|at| data.cstr(at, MAX_NAME_BYTES).ok())
    } else {
        None
    };
    Ok(UdtRecord { property: head.property, field_list: head.field_list, size: size.value, name, unique })
}

/// Where one UDT form keeps its fields.
#[derive(Clone, Copy)]
struct UdtHead {
    property: u32,
    field_list: u32,
    size_at: usize,
}

/// One field-list entry, and where the next starts (padding skipped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldRecord<'a> {
    Member { type_index: u32, offset: Option<u64>, name: &'a [u8], next: usize },
    Index { continued: u32, next: usize },
    /// A base class, method, nested type, static member or enumerator:
    /// stepped over.
    Other { next: usize },
}

/// Why a field-list entry did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldError {
    Short(ReadError),
    Unknown { leaf: u16 },
}

impl From<ReadError> for FieldError {
    fn from(error: ReadError) -> Self {
        FieldError::Short(error)
    }
}

/// The offset after the NUL-terminated name at `at`, then after any
/// `LF_PAD` bytes (each 0xF0 + n skips n bytes, n at least 1).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn after_name(list: Bytes<'_>, at: usize) -> Result<usize, ReadError> {
    let name = list.cstr(at, MAX_NAME_BYTES)?;
    let end = at.checked_add(name.len()).and_then(|end| end.checked_add(1)).ok_or(ReadError::Truncated { at, len: 1 })?;
    skip_padding(list, end)
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn skip_padding(list: Bytes<'_>, at: usize) -> Result<usize, ReadError> {
    match list.u8(at) {
        Ok(byte) if byte >= 0xF0 => {
            let skip = usize::from(byte & 0x0F).max(1);
            Ok(at.saturating_add(skip).min(list.len()))
        }
        Ok(_) | Err(_) => Ok(at),
    }
}

/// The field-list entry at `at` (`lfFieldList`'s members).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_field(list: Bytes<'_>, at: usize) -> Result<FieldRecord<'_>, FieldError> {
    let leaf = list.u16_le(at)?;
    let field = |offset: usize| at.checked_add(offset).ok_or(ReadError::Truncated { at, len: offset });
    Ok(match leaf {
        LF_MEMBER => {
            let type_index = list.u32_le(field(4)?)?;
            let offset = decode_numeric(list, field(8)?)?;
            let name = list.cstr(offset.next, MAX_NAME_BYTES)?;
            FieldRecord::Member { type_index, offset: offset.value, name, next: after_name(list, offset.next)? }
        }
        LF_INDEX => FieldRecord::Index { continued: list.u32_le(field(4)?)?, next: skip_padding(list, field(8)?)? },
        LF_BCLASS | LF_BINTERFACE => {
            let offset = decode_numeric(list, field(8)?)?;
            FieldRecord::Other { next: skip_padding(list, offset.next)? }
        }
        LF_VBCLASS | LF_IVBCLASS => {
            let pointer_offset = decode_numeric(list, field(12)?)?;
            let table_offset = decode_numeric(list, pointer_offset.next)?;
            FieldRecord::Other { next: skip_padding(list, table_offset.next)? }
        }
        LF_VFUNCTAB => FieldRecord::Other { next: skip_padding(list, field(8)?)? },
        LF_VFUNCOFF => FieldRecord::Other { next: skip_padding(list, field(12)?)? },
        LF_ENUMERATE => {
            let value = decode_numeric(list, field(4)?)?;
            FieldRecord::Other { next: after_name(list, value.next)? }
        }
        LF_STMEMBER | LF_NESTTYPE | LF_NESTTYPEEX | LF_METHOD => FieldRecord::Other { next: after_name(list, field(8)?)? },
        LF_ONEMETHOD => {
            let attributes = list.u16_le(field(2)?)?;
            let property = attributes.wrapping_shr(2) & 7;
            let name_at = if property == METHOD_INTRO || property == METHOD_PURE_INTRO { field(12)? } else { field(8)? };
            FieldRecord::Other { next: after_name(list, name_at)? }
        }
        other => return Err(FieldError::Unknown { leaf: other }),
    })
}

/// One member as a layout reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Member {
    name: String,
    type_index: u32,
    offset: u64,
}

/// What a member's type is, past modifiers and forward references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resolved {
    /// A structure or union definition.
    Udt { index: u32 },
    Bitfield { storage: u32, position: u8, length: u8 },
    /// Anything else: a primitive, a pointer, an array, an enum.
    Other { index: u32 },
}

/// The TPI records, indexed once.
pub struct TypeTables {
    records: Vec<u8>,
    begin: u32,
    /// Each record's offset in `records`.
    offsets: Vec<u32>,
    /// Definitions (not forward references) by name.
    by_name: BTreeMap<String, u32>,
    /// Definitions by unique name.
    by_unique: BTreeMap<String, u32>,
    /// UDT records that did not decode.
    unreadable: u32,
}

/// One record's kind and payload.
struct Record<'a> {
    kind: u16,
    data: Bytes<'a>,
}

impl TypeTables {
    /// Indexes the TPI stream `stream`.
    pub fn parse(stream: Vec<u8>) -> Result<TypeTables, LayoutError> {
        let header = Bytes::new(&stream);
        let unreadable = |what| LayoutError::Unreadable { what };
        let header_size = header.u32_le(4).map_err(|_| unreadable("TPI header"))? as usize;
        let begin = header.u32_le(8).map_err(|_| unreadable("TPI header"))?;
        let end = header.u32_le(12).map_err(|_| unreadable("TPI header"))?;
        let record_bytes = header.u32_le(16).map_err(|_| unreadable("TPI header"))? as usize;
        if header_size < HEADER_BYTES || begin < FIRST_RECORD || end < begin || end.saturating_sub(begin) > MAX_TYPES {
            return Err(unreadable("TPI header"));
        }
        let records = header.slice(header_size, record_bytes).map_err(|_| unreadable("TPI records"))?.to_vec();
        let count = end.wrapping_sub(begin) as usize;
        let mut offsets = Vec::new();
        offsets.try_reserve_exact(count).map_err(|_| unreadable("TPI index"))?;
        let mut at = 0usize;
        let bytes = Bytes::new(&records);
        while offsets.len() < count && at < bytes.len() {
            offsets.push(u32::try_from(at).map_err(|_| unreadable("TPI records"))?);
            let len = usize::from(bytes.u16_le(at).map_err(|_| unreadable("TPI records"))?);
            at = at.checked_add(2).and_then(|at| at.checked_add(len)).ok_or(unreadable("TPI records"))?;
        }
        let mut tables = TypeTables {
            records,
            begin,
            offsets,
            by_name: BTreeMap::new(),
            by_unique: BTreeMap::new(),
            unreadable: 0,
        };
        tables.index_definitions();
        Ok(tables)
    }

    /// Names every UDT definition; counts UDT records that do not decode.
    fn index_definitions(&mut self) {
        let mut by_name = BTreeMap::new();
        let mut by_unique = BTreeMap::new();
        let mut unreadable = 0u32;
        for position in 0..self.offsets.len() {
            let index = self.begin.wrapping_add(position as u32);
            let Ok(record) = self.record(index) else {
                continue;
            };
            if !is_udt(record.kind) {
                continue;
            }
            let Ok(udt) = decode_udt(record.kind, record.data) else {
                unreadable = unreadable.saturating_add(1);
                continue;
            };
            if udt.property & PROPERTY_FORWARD != 0 {
                continue;
            }
            by_name.entry(String::from_utf8_lossy(udt.name).into_owned()).or_insert(index);
            if let Some(unique) = udt.unique {
                by_unique.entry(String::from_utf8_lossy(unique).into_owned()).or_insert(index);
            }
        }
        self.by_name = by_name;
        self.by_unique = by_unique;
        self.unreadable = unreadable;
    }

    fn record(&self, index: u32) -> Result<Record<'_>, LayoutError> {
        let unreadable = LayoutError::Unreadable { what: "type record" };
        let position = index.checked_sub(self.begin).ok_or(unreadable.clone())? as usize;
        let offset = *self.offsets.get(position).ok_or(unreadable.clone())? as usize;
        let bytes = Bytes::new(&self.records);
        let len = usize::from(bytes.u16_le(offset).map_err(|_| unreadable.clone())?);
        let kind = bytes.u16_le(offset.saturating_add(2)).map_err(|_| unreadable.clone())?;
        let data = bytes
            .sub(offset.saturating_add(4), len.saturating_sub(2))
            .map_err(|_| unreadable)?;
        Ok(Record { kind, data })
    }

    /// The definition `name` names, or why a miss is not an answer.
    fn definition(&self, name: &str) -> Result<Option<u32>, LayoutError> {
        match self.by_name.get(name) {
            Some(index) => Ok(Some(*index)),
            None if self.unreadable > 0 => {
                Err(LayoutError::UnreadableTypeRecords { name: name.to_owned(), count: self.unreadable })
            }
            None => Ok(None),
        }
    }

    /// The UDT at `index`, its forward reference resolved to the definition.
    fn udt(&self, index: u32) -> Result<Option<UdtFound>, LayoutError> {
        let record = self.record(index)?;
        if !is_udt(record.kind) {
            return Ok(None);
        }
        let udt = decode_udt(record.kind, record.data).map_err(|_| LayoutError::Unreadable { what: "structure record" })?;
        if udt.property & PROPERTY_FORWARD == 0 {
            return Ok(Some(UdtFound { index, field_list: udt.field_list, size: udt.size }));
        }
        let unique = udt.unique.map(|unique| String::from_utf8_lossy(unique).into_owned());
        let name = String::from_utf8_lossy(udt.name).into_owned();
        let definition = unique
            .and_then(|unique| self.by_unique.get(&unique).copied())
            .or_else(|| self.by_name.get(&name).copied());
        match definition {
            Some(definition) if definition != index => self.udt(definition),
            Some(_) | None => Ok(None),
        }
    }

    /// The members of field list `list`, following `LF_INDEX` continuations.
    fn members(&self, list: u32, owner: &str) -> Result<Vec<Member>, LayoutError> {
        let mut members = Vec::new();
        let mut next_list = Some(list);
        let mut hops = 0u32;
        while let Some(list) = next_list.take() {
            hops = hops.saturating_add(1);
            if hops > MAX_FIELD_LIST_HOPS {
                return Err(LayoutError::EndlessFieldList { name: owner.to_owned() });
            }
            let record = self.record(list)?;
            if record.kind != LF_FIELDLIST {
                return Err(LayoutError::Unreadable { what: "field list" });
            }
            let mut at = 0usize;
            while at < record.data.len() {
                let entry = decode_field(record.data, at).map_err(|error| match error {
                    FieldError::Unknown { leaf } => LayoutError::UnknownFieldRecord { name: owner.to_owned(), leaf },
                    FieldError::Short(_) => LayoutError::Unreadable { what: "field list" },
                })?;
                let next = match entry {
                    FieldRecord::Member { type_index, offset, name, next } => {
                        let offset = offset.ok_or(LayoutError::BadNumber { name: owner.to_owned() })?;
                        members
                            .try_reserve(1)
                            .map_err(|_| LayoutError::Unreadable { what: "field list (no memory)" })?;
                        members.push(Member { name: String::from_utf8_lossy(name).into_owned(), type_index, offset });
                        next
                    }
                    FieldRecord::Index { continued, next } => {
                        next_list = Some(continued);
                        next
                    }
                    FieldRecord::Other { next } => next,
                };
                if next <= at {
                    return Err(LayoutError::Unreadable { what: "field list" });
                }
                at = next;
            }
        }
        Ok(members)
    }

    /// `index` past modifiers, enums' underlying types stay as themselves.
    fn resolve(&self, index: u32) -> Result<Resolved, LayoutError> {
        let mut current = index;
        for _ in 0..MAX_TYPE_HOPS {
            if current < FIRST_RECORD {
                return Ok(Resolved::Other { index: current });
            }
            let record = self.record(current)?;
            match record.kind {
                LF_MODIFIER => {
                    current = record.data.u32_le(0).map_err(|_| LayoutError::Unreadable { what: "modifier" })?;
                }
                LF_BITFIELD => {
                    let storage = record.data.u32_le(0).map_err(|_| LayoutError::Unreadable { what: "bitfield" })?;
                    let length = record.data.u8(4).map_err(|_| LayoutError::Unreadable { what: "bitfield" })?;
                    let position = record.data.u8(5).map_err(|_| LayoutError::Unreadable { what: "bitfield" })?;
                    return Ok(Resolved::Bitfield { storage, position, length });
                }
                kind if is_udt(kind) => {
                    return Ok(match self.udt(current)? {
                        Some(found) => Resolved::Udt { index: found.index },
                        None => Resolved::Other { index: current },
                    });
                }
                _ => return Ok(Resolved::Other { index: current }),
            }
        }
        Err(LayoutError::EndlessTypeChain { index: u64::from(index) })
    }

    /// The size of type `index` in bytes, when it has one.
    fn size_of(&self, index: u32) -> Result<Option<u64>, LayoutError> {
        let mut current = index;
        for _ in 0..MAX_TYPE_HOPS {
            if current < FIRST_RECORD {
                return Ok(primitive_size(current));
            }
            let record = self.record(current)?;
            match record.kind {
                LF_MODIFIER => {
                    current = record.data.u32_le(0).map_err(|_| LayoutError::Unreadable { what: "modifier" })?;
                }
                LF_BITFIELD => {
                    current = record.data.u32_le(0).map_err(|_| LayoutError::Unreadable { what: "bitfield" })?;
                }
                LF_ENUM => {
                    current = record.data.u32_le(4).map_err(|_| LayoutError::Unreadable { what: "enum" })?;
                }
                LF_POINTER => {
                    let attributes = record.data.u32_le(4).map_err(|_| LayoutError::Unreadable { what: "pointer" })?;
                    return Ok(Some(u64::from(attributes.wrapping_shr(13) & 0x3F)).filter(|size| *size != 0));
                }
                LF_ARRAY => {
                    let size = decode_numeric(record.data, 8).map_err(|_| LayoutError::Unreadable { what: "array" })?;
                    return Ok(size.value);
                }
                kind if is_udt(kind) => return Ok(self.udt(current)?.and_then(|found| found.size)),
                _ => return Ok(None),
            }
        }
        Err(LayoutError::EndlessTypeChain { index: u64::from(index) })
    }

    /// The layout of the structure or union named `name`.
    pub fn layout(&self, name: &str) -> Result<Option<TypeLayout>, LayoutError> {
        let Some(index) = self.definition(name)? else {
            return Ok(None);
        };
        let Some(found) = self.udt(index)? else {
            return Ok(None);
        };
        let size = found.size.ok_or(LayoutError::BadNumber { name: name.to_owned() })?;
        let members = self.members(found.field_list, name)?;
        let mut fields = Vec::new();
        fields
            .try_reserve_exact(members.len())
            .map_err(|_| LayoutError::Unreadable { what: "layout (no memory)" })?;
        for member in members {
            let bits = match self.resolve(member.type_index)? {
                Resolved::Bitfield { position, length, .. } => {
                    Some(BitRange { position: u32::from(position), length: u32::from(length) })
                }
                Resolved::Udt { .. } | Resolved::Other { .. } => None,
            };
            fields.push(FieldLayout { name: member.name, offset: FieldOffset(member.offset), bits });
        }
        Ok(Some(TypeLayout { size, fields }))
    }

    /// Where the member reached by `path` sits in `type_name`.
    pub fn member_location(&self, type_name: &str, path: &[&str]) -> Result<Option<MemberLocation>, LayoutError> {
        let Some(index) = self.definition(type_name)? else {
            return Ok(None);
        };
        let Some(mut current) = self.udt(index)? else {
            return Ok(None);
        };
        let mut offset = 0u64;
        let mut walked = String::new();
        for (step, name) in path.iter().enumerate() {
            if !walked.is_empty() {
                walked.push('.');
            }
            walked.push_str(name);
            let members = self.members(current.field_list, type_name)?;
            let Some(member) = members.into_iter().find(|member| member.name == *name) else {
                return Ok(None);
            };
            offset = offset.checked_add(member.offset).ok_or(LayoutError::BadNumber { name: walked.clone() })?;
            let resolved = self.resolve(member.type_index)?;
            let last = step.saturating_add(1) == path.len();
            if last {
                return Ok(Some(match resolved {
                    Resolved::Bitfield { storage, position, length } => MemberLocation {
                        offset: FieldOffset(offset),
                        bits: Some(BitRange { position: u32::from(position), length: u32::from(length) }),
                        size: self.size_of(storage)?,
                    },
                    Resolved::Udt { index } | Resolved::Other { index } => MemberLocation {
                        offset: FieldOffset(offset),
                        bits: None,
                        size: self.size_of(index)?,
                    },
                }));
            }
            current = match resolved {
                Resolved::Udt { index } => self.udt(index)?.ok_or(LayoutError::Unreadable { what: "structure record" })?,
                Resolved::Bitfield { .. } | Resolved::Other { .. } => {
                    let next = path.get(step.saturating_add(1)).copied().unwrap_or("");
                    return Err(LayoutError::NotAStructure { member: format!("{walked}.{next}") });
                }
            };
        }
        Ok(None)
    }
}

/// A UDT definition's index, field list and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UdtFound {
    index: u32,
    field_list: u32,
    size: Option<u64>,
}

/// A primitive type's size (`cvinfo.h` "Primitive types": the mode in bits
/// 8..11, the kind in bits 0..7).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn primitive_size(index: u32) -> Option<u64> {
    let mode = index.wrapping_shr(8) & 0xF;
    let size = match mode {
        0 => match index & 0xFF {
            0x10 | 0x20 | 0x30 | 0x68 | 0x69 | 0x70 => 1,
            0x11 | 0x21 | 0x31 | 0x71 | 0x72 | 0x73 | 0x7A => 2,
            0x08 | 0x12 | 0x22 | 0x32 | 0x40 | 0x74 | 0x75 | 0x7B => 4,
            0x13 | 0x23 | 0x33 | 0x41 | 0x76 | 0x77 => 8,
            0x42 => 10,
            0x14 | 0x24 | 0x43 | 0x78 | 0x79 => 16,
            _ => return None,
        },
        1 => 2,
        2..=4 => 4,
        5 => 6,
        6 => 8,
        7 => 16,
        _ => return None,
    };
    Some(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdb::tables::PdbTables;
    use crate::tables::{FieldLayout, MemberLocation};
    use crate::units::{BitRange, FieldOffset};
    use rusty_box_symbols_fixtures::pdb::{
        build_pdb, FixtureContainer, FixtureField, FixtureType, FixtureUdt, PdbFixture, RecordForm, UdtKind,
    };

    fn field(name: &str, offset: u64, ty: FixtureType) -> FixtureField {
        FixtureField { name: name.to_owned(), offset, ty }
    }

    fn bits(position: u8, length: u8) -> FixtureType {
        FixtureType::Bitfield { storage: Box::new(FixtureType::U32), position, length }
    }

    fn udt(name: &str, kind: UdtKind, size: u64, fields: Vec<FixtureField>) -> FixtureUdt {
        FixtureUdt { name: name.to_owned(), kind, size, fields }
    }

    /// The shapes the Windows walker reads: a process holding its kernel
    /// half by value, a list entry, a fixed name array, and a VAD whose
    /// flags are bitfields inside an anonymous union.
    fn kernel_types() -> Vec<FixtureUdt> {
        vec![
            udt("_LIST_ENTRY", UdtKind::Struct, 0x10, vec![
                field("Flink", 0, FixtureType::Pointer),
                field("Blink", 8, FixtureType::Pointer),
            ]),
            udt("_KPROCESS", UdtKind::Struct, 0x2D8, vec![
                field("Header", 0, FixtureType::Array { element: Box::new(FixtureType::U8), count: 0x18 }),
                field("ProfileListHead", 0x18, FixtureType::Named("_LIST_ENTRY".into())),
                field("DirectoryTableBase", 0x28, FixtureType::U64),
            ]),
            udt("_EPROCESS", UdtKind::Struct, 0xA40, vec![
                field("Pcb", 0, FixtureType::Named("_KPROCESS".into())),
                field("UniqueProcessId", 0x440, FixtureType::Pointer),
                field("ActiveProcessLinks", 0x448, FixtureType::Named("_LIST_ENTRY".into())),
                field("ImageFileName", 0x5A8, FixtureType::Array { element: Box::new(FixtureType::U8), count: 15 }),
            ]),
            udt("_MMVAD_FLAGS", UdtKind::Struct, 4, vec![
                field("Lock", 0, bits(0, 1)),
                field("VadType", 0, bits(4, 3)),
                field("Protection", 0, bits(7, 5)),
                field("PrivateMemory", 0, bits(20, 1)),
            ]),
            udt("_MMVAD_SHORT", UdtKind::Struct, 0x40, vec![
                field("StartingVpn", 0x18, FixtureType::U32),
                field("u", 0x30, FixtureType::Nested(Box::new(udt("", UdtKind::Union, 4, vec![
                    field("LongFlags", 0, FixtureType::U32),
                    field("VadFlags", 0, FixtureType::Named("_MMVAD_FLAGS".into())),
                ])))),
            ]),
            // Big enough that its size and a member's offset need numeric leaves.
            udt("_BIG", UdtKind::Struct, 0x1_2348, vec![field("Far", 0x1_2340, FixtureType::U64)]),
        ]
    }

    fn tables(container: FixtureContainer, form: RecordForm, broken: u32) -> PdbTables {
        let mut fixture = PdbFixture::new([7; 16], 1);
        fixture.types = kernel_types();
        fixture.form = form;
        fixture.broken_udt_records = broken;
        fixture.container = container;
        PdbTables::parse(&build_pdb(&fixture)).unwrap()
    }

    /// A TPI stream of `records` (each a leaf and its payload), indices
    /// from 0x1000.
    fn tpi(records: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        for record in records {
            body.extend_from_slice(&(record.len() as u16).to_le_bytes());
            body.extend_from_slice(record);
        }
        let mut stream = Vec::new();
        stream.extend_from_slice(&20040203u32.to_le_bytes());
        stream.extend_from_slice(&56u32.to_le_bytes());
        stream.extend_from_slice(&0x1000u32.to_le_bytes());
        stream.extend_from_slice(&(0x1000 + records.len() as u32).to_le_bytes());
        stream.extend_from_slice(&(body.len() as u32).to_le_bytes());
        stream.resize(56, 0);
        stream.extend_from_slice(&body);
        stream
    }

    /// `LF_STRUCTURE` named `name`, `count` members, field list `list`,
    /// size 8.
    fn structure(name: &str, list: u32) -> Vec<u8> {
        let mut record = 0x1505u16.to_le_bytes().to_vec();
        record.extend_from_slice(&1u16.to_le_bytes());
        record.extend_from_slice(&0u16.to_le_bytes());
        record.extend_from_slice(&list.to_le_bytes());
        record.extend_from_slice(&[0; 8]);
        record.extend_from_slice(&8u16.to_le_bytes());
        record.extend_from_slice(name.as_bytes());
        record.push(0);
        while record.len() % 4 != 2 {
            record.push(0xF0 | (4 - (record.len() + 2) % 4) as u8);
        }
        record
    }

    #[test]
    fn a_layout_lists_every_member_across_a_split_field_list() {
        for container in [FixtureContainer::Msf, FixtureContainer::MsfzZstd] {
            for form in [RecordForm::Classic, RecordForm::Wide] {
                let tables = tables(container, form, 0);
                let layout = tables.layout("_EPROCESS").unwrap().expect("_EPROCESS");
                assert_eq!(layout.size, 0xA40);
                let names: Vec<&str> = layout.fields.iter().map(|field| field.name.as_str()).collect();
                assert_eq!(names, ["Pcb", "UniqueProcessId", "ActiveProcessLinks", "ImageFileName"], "{container:?} {form:?}");
                let flags = tables.layout("_MMVAD_FLAGS").unwrap().expect("flags");
                assert_eq!(
                    flags.fields[2],
                    FieldLayout { name: "Protection".into(), offset: FieldOffset(0), bits: Some(BitRange { position: 7, length: 5 }) }
                );
            }
        }
    }

    #[test]
    fn a_member_path_reaches_through_nested_and_forward_referenced_types() {
        for form in [RecordForm::Classic, RecordForm::Wide] {
            let tables = tables(FixtureContainer::Msf, form, 0);
            assert_eq!(
                tables.member_location("_EPROCESS", &["Pcb", "DirectoryTableBase"]).unwrap(),
                Some(MemberLocation { offset: FieldOffset(0x28), bits: None, size: Some(8) })
            );
            assert_eq!(
                tables.member_location("_EPROCESS", &["ActiveProcessLinks", "Blink"]).unwrap(),
                Some(MemberLocation { offset: FieldOffset(0x450), bits: None, size: Some(8) })
            );
            assert_eq!(
                tables.member_location("_EPROCESS", &["ImageFileName"]).unwrap(),
                Some(MemberLocation { offset: FieldOffset(0x5A8), bits: None, size: Some(15) })
            );
            assert_eq!(
                tables.member_location("_MMVAD_SHORT", &["u", "VadFlags", "Protection"]).unwrap(),
                Some(MemberLocation { offset: FieldOffset(0x30), bits: Some(BitRange { position: 7, length: 5 }), size: Some(4) })
            );
            assert_eq!(
                tables.member_location("_BIG", &["Far"]).unwrap(),
                Some(MemberLocation { offset: FieldOffset(0x1_2340), bits: None, size: Some(8) })
            );
        }
    }

    /// An older build lacks a field: no answer, and the walker discovers it.
    #[test]
    fn an_absent_member_or_type_is_no_answer() {
        let tables = tables(FixtureContainer::Msf, RecordForm::Classic, 0);
        assert_eq!(tables.member_location("_EPROCESS", &["WoW64Process"]).unwrap(), None);
        assert_eq!(tables.member_location("_ETHREAD", &["Cid"]).unwrap(), None);
        assert_eq!(tables.layout("_ETHREAD").unwrap(), None);
    }

    #[test]
    fn a_path_through_a_scalar_is_refused() {
        let tables = tables(FixtureContainer::Msf, RecordForm::Classic, 0);
        assert_eq!(
            tables.member_location("_EPROCESS", &["UniqueProcessId", "Low"]),
            Err(LayoutError::NotAStructure { member: "UniqueProcessId.Low".to_owned() })
        );
    }

    /// Records this reader cannot read may hold the type asked for, so a
    /// miss is an error the walker answers with discovery.
    #[test]
    fn a_miss_among_unreadable_records_is_an_error_not_an_absence() {
        let tables = tables(FixtureContainer::Msf, RecordForm::Classic, 2);
        assert_eq!(
            tables.layout("_ETHREAD"),
            Err(LayoutError::UnreadableTypeRecords { name: "_ETHREAD".to_owned(), count: 2 })
        );
        assert!(tables.layout("_EPROCESS").unwrap().is_some());
    }

    /// Review focus 3: an `LF_INDEX` that continues a field list with
    /// itself.
    #[test]
    fn a_field_list_chain_that_loops_ends_with_an_error() {
        let mut list = 0x1203u16.to_le_bytes().to_vec();
        list.extend_from_slice(&0x1404u16.to_le_bytes());
        list.extend_from_slice(&0u16.to_le_bytes());
        list.extend_from_slice(&0x1000u32.to_le_bytes());
        let types = TypeTables::parse(tpi(&[list, structure("Loop", 0x1000)])).unwrap();
        assert_eq!(types.layout("Loop"), Err(LayoutError::EndlessFieldList { name: "Loop".to_owned() }));
    }

    /// Review focus 3: a member whose type is a modifier of itself.
    #[test]
    fn a_type_chain_that_loops_ends_with_an_error() {
        let mut modifier = 0x1001u16.to_le_bytes().to_vec();
        modifier.extend_from_slice(&0x1000u32.to_le_bytes());
        modifier.extend_from_slice(&0u16.to_le_bytes());
        modifier.extend_from_slice(&[0xF2, 0xF1]);
        let mut list = 0x1203u16.to_le_bytes().to_vec();
        list.extend_from_slice(&0x150Du16.to_le_bytes());
        list.extend_from_slice(&3u16.to_le_bytes());
        list.extend_from_slice(&0x1000u32.to_le_bytes());
        list.extend_from_slice(&0u16.to_le_bytes());
        list.extend_from_slice(b"m\0");
        let types = TypeTables::parse(tpi(&[modifier, list, structure("Self", 0x1001)])).unwrap();
        assert_eq!(types.member_location("Self", &["m"]), Err(LayoutError::EndlessTypeChain { index: 0x1000 }));
    }

    #[test]
    fn numeric_leaves_decode_and_negatives_are_refused() {
        let numbers = [0x34, 0x12, 0x02, 0x80, 0xFF, 0xFF, 0x0A, 0x80, 1, 0, 0, 0, 0, 0, 0, 0x80, 0x00, 0x80, 0xFF];
        let bytes = Bytes::new(&numbers);
        assert_eq!(decode_numeric(bytes, 0), Ok(Numeric { value: Some(0x1234), next: 2 }));
        assert_eq!(decode_numeric(bytes, 2), Ok(Numeric { value: Some(0xFFFF), next: 6 }));
        assert_eq!(decode_numeric(bytes, 6), Ok(Numeric { value: Some(0x8000_0000_0000_0001), next: 16 }));
        // LF_CHAR -1.
        assert_eq!(decode_numeric(bytes, 16), Ok(Numeric { value: None, next: 19 }));
    }
}
