//! The DBI stream (stream 3; LLVM, "The DBI Stream"): a 64-byte header
//! whose sizes lay out its substreams in this order: module information,
//! section contributions, section map, source files, type-server map, the
//! edit-and-continue substream, and the optional debug header, whose slots
//! name the section-header and OMAP streams.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use super::{short, PdbError};
use crate::read::{Bytes, Cursor, ReadError};

pub const HEADER_BYTES: usize = 64;
/// Optional debug header slots (LLVM, "Optional Debug Header Stream").
const OMAP_FROM_SOURCE_SLOT: usize = 4;
const SECTION_HEADERS_SLOT: usize = 5;
const ORIGINAL_SECTION_HEADERS_SLOT: usize = 10;
/// A stream index of 0xFFFF names no stream.
const NO_STREAM: u16 = 0xFFFF;
/// The fixed part of a module-information entry.
const MODULE_FIXED_BYTES: usize = 64;
const MAX_NAME_BYTES: usize = 4096;
/// PE images have at most 96 sections (PE Format); a PDB's section-header
/// stream past this is not a real one.
const MAX_SECTIONS: usize = 1 << 16;
const SECTION_HEADER_BYTES: usize = 40;
const OMAP_ENTRY_BYTES: usize = 8;

/// The DBI header's fields the reader uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DbiHeader {
    pub age: u32,
    pub symbol_records: u16,
    pub module_info_bytes: u32,
    pub section_contribution_bytes: u32,
    pub section_map_bytes: u32,
    pub source_info_bytes: u32,
    pub type_server_map_bytes: u32,
    pub optional_header_bytes: u32,
    pub edit_and_continue_bytes: u32,
}

/// The DBI header (sizes are `i32` in the format; a negative one reads as
/// a size past the stream, which the substream reads then refuse).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_header(stream: Bytes<'_>) -> Result<DbiHeader, ReadError> {
    Ok(DbiHeader {
        age: stream.u32_le(8)?,
        symbol_records: stream.u16_le(20)?,
        module_info_bytes: stream.u32_le(24)?,
        section_contribution_bytes: stream.u32_le(28)?,
        section_map_bytes: stream.u32_le(32)?,
        source_info_bytes: stream.u32_le(36)?,
        type_server_map_bytes: stream.u32_le(40)?,
        optional_header_bytes: stream.u32_le(48)?,
        edit_and_continue_bytes: stream.u32_le(52)?,
    })
}

/// One module's entry: its stream and the sizes of its parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleInfo {
    /// `NO_STREAM` for a module without symbols.
    pub stream: u16,
    /// Symbols, counting the stream's 4-byte signature.
    pub symbol_bytes: u32,
    pub c11_bytes: u32,
    pub c13_bytes: u32,
}

/// A module entry and where the next one starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleEntry {
    pub info: ModuleInfo,
    pub next: usize,
}

/// The module entry at `at` in the module-information substream: 64 fixed
/// bytes, the module and object names, padded to 4.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_module(substream: Bytes<'_>, at: usize) -> Result<ModuleEntry, ReadError> {
    let field = |offset: usize| at.checked_add(offset).ok_or(ReadError::Truncated { at, len: offset });
    let info = ModuleInfo {
        stream: substream.u16_le(field(34)?)?,
        symbol_bytes: substream.u32_le(field(36)?)?,
        c11_bytes: substream.u32_le(field(40)?)?,
        c13_bytes: substream.u32_le(field(44)?)?,
    };
    let mut cursor = Cursor::new(substream, field(MODULE_FIXED_BYTES)?);
    cursor.cstr(MAX_NAME_BYTES)?;
    cursor.cstr(MAX_NAME_BYTES)?;
    cursor.align(4)?;
    Ok(ModuleEntry { info, next: cursor.position() })
}

/// The streams the optional debug header names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionalStreams {
    pub section_headers: Option<u16>,
    pub original_section_headers: Option<u16>,
    pub omap_from_source: Option<u16>,
}

/// Where the module-information substream starts and how long it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub at: usize,
    pub len: usize,
}

/// The module-information substream's span: right after the header.
#[must_use]
pub fn modules_span(header: &DbiHeader) -> Span {
    Span { at: HEADER_BYTES, len: header.module_info_bytes as usize }
}

/// The optional debug header's slots; `None` for an empty slot or one the
/// header is too short to hold.
#[must_use]
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_optional_streams(stream: Bytes<'_>, header: &DbiHeader) -> OptionalStreams {
    let sizes = [
        header.module_info_bytes,
        header.section_contribution_bytes,
        header.section_map_bytes,
        header.source_info_bytes,
        header.type_server_map_bytes,
        header.edit_and_continue_bytes,
    ];
    let at = sizes
        .iter()
        .try_fold(HEADER_BYTES, |at, size| at.checked_add(*size as usize));
    let slot = |index: usize| -> Option<u16> {
        let offset = index.checked_mul(2)?;
        if offset.checked_add(2)? > header.optional_header_bytes as usize {
            return None;
        }
        let value = stream.u16_le(at?.checked_add(offset)?).ok()?;
        (value != NO_STREAM).then_some(value)
    };
    OptionalStreams {
        section_headers: slot(SECTION_HEADERS_SLOT),
        original_section_headers: slot(ORIGINAL_SECTION_HEADERS_SLOT),
        omap_from_source: slot(OMAP_FROM_SOURCE_SLOT),
    }
}

/// Each section's RVA, from a stream of `IMAGE_SECTION_HEADER`s.
pub fn section_rvas(stream: &[u8]) -> Result<Vec<u64>, PdbError> {
    let count = stream.len().checked_div(SECTION_HEADER_BYTES).unwrap_or(0);
    if count > MAX_SECTIONS {
        return Err(PdbError::TooMany { what: "sections", count: count as u64 });
    }
    let mut rvas = Vec::new();
    rvas.try_reserve_exact(count)
        .map_err(|_| PdbError::Allocation { what: "sections", bytes: count as u64 })?;
    for header in stream.chunks_exact(SECTION_HEADER_BYTES) {
        let rva = Bytes::new(header).u32_le(12).map_err(|source| short("section header", source))?;
        rvas.push(u64::from(rva));
    }
    Ok(rvas)
}

/// One OMAP entry: a source RVA and the RVA it moved to (0 if dropped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OmapEntry {
    pub from: u64,
    pub to: u64,
}

/// `OMAP_FROM_SRC`, sorted by source RVA.
pub fn omap(stream: &[u8]) -> Result<Vec<OmapEntry>, PdbError> {
    if stream.len().checked_rem(OMAP_ENTRY_BYTES).unwrap_or(1) != 0 {
        return Err(PdbError::Omap { bytes: stream.len() });
    }
    let count = stream.len().checked_div(OMAP_ENTRY_BYTES).unwrap_or(0);
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(count)
        .map_err(|_| PdbError::Allocation { what: "OMAP", bytes: count as u64 })?;
    for raw in stream.chunks_exact(OMAP_ENTRY_BYTES) {
        let raw = Bytes::new(raw);
        let from = raw.u32_le(0).map_err(|source| short("OMAP", source))?;
        let to = raw.u32_le(4).map_err(|source| short("OMAP", source))?;
        entries.push(OmapEntry { from: u64::from(from), to: u64::from(to) });
    }
    entries.sort_unstable_by_key(|entry| entry.from);
    Ok(entries)
}

/// The RVA in the rewritten image of `source`: the last entry at or below
/// it carries it, offset by how far past that entry it lies; an entry
/// mapping to 0 dropped its range.
#[must_use]
pub fn omap_translate(map: &[OmapEntry], source: u64) -> Option<u64> {
    let index = map.partition_point(|entry| entry.from <= source).checked_sub(1)?;
    let entry = map.get(index)?;
    if entry.to == 0 {
        return None;
    }
    entry.to.checked_add(source.checked_sub(entry.from)?)
}
