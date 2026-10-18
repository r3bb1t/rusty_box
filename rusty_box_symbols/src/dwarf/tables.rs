//! An ELF file's tables: its `.symtab`/`.dynsym` symbols, and from its DWARF
//! the functions' address ranges, the line table and the structure
//! layouts; with a dwz supplementary file, the entries and strings it holds
//! too. Names and lines are extracted when the file is parsed, so a lookup
//! cannot fail; layouts are read from the entries when asked.
//!
//! A unit that does not read is passed by and counted: the names it held
//! are missing (never wrong), and a layout question that misses while any
//! unit was passed by is an error, not an absence.

use std::collections::BTreeMap;

use super::abbrev::{Abbreviations, Children};
use super::entries::{unit_abbreviations, Entry, Step, UnitContext};
use super::forms::Value;
use super::line::{parse_line_program, LineContext, Sequence};
use super::ranges::{debug_ranges, debug_rnglists, rnglist_offset, Range};
use super::unit::{decode_unit_header, UnitHeader, UnitKind, UnitSection};
use super::{push, DwarfError, Format};
use crate::elf::{ElfFile, ElfSymbol};
use crate::identity::{BuildId, TablesIdentity};
use crate::read::Bytes;
use crate::line_index::{CodeSpan, LineIndex};
use crate::tables::{
    FieldLayout, LayoutError, LineRow, MemberLocation, ParseError, ProcedureSpan, ResolvedLine, SourceFile, SourceLine,
    SymbolHit, TypeLayout,
};
use crate::units::{BitRange, FieldOffset, Rva};

const DW_TAG_ARRAY_TYPE: u64 = 0x01;
const DW_TAG_CLASS_TYPE: u64 = 0x02;
const DW_TAG_ENUMERATION_TYPE: u64 = 0x04;
const DW_TAG_MEMBER: u64 = 0x0d;
const DW_TAG_POINTER_TYPE: u64 = 0x0f;
const DW_TAG_REFERENCE_TYPE: u64 = 0x10;
const DW_TAG_STRUCTURE_TYPE: u64 = 0x13;
const DW_TAG_TYPEDEF: u64 = 0x16;
const DW_TAG_UNION_TYPE: u64 = 0x17;
const DW_TAG_PTR_TO_MEMBER_TYPE: u64 = 0x1f;
const DW_TAG_SUBRANGE_TYPE: u64 = 0x21;
const DW_TAG_CONST_TYPE: u64 = 0x26;
const DW_TAG_SUBPROGRAM: u64 = 0x2e;
const DW_TAG_VOLATILE_TYPE: u64 = 0x35;
const DW_TAG_RESTRICT_TYPE: u64 = 0x37;
const DW_TAG_RVALUE_REFERENCE_TYPE: u64 = 0x42;
const DW_TAG_ATOMIC_TYPE: u64 = 0x47;
const DW_AT_NAME: u64 = 0x03;
const DW_AT_BYTE_SIZE: u64 = 0x0b;
const DW_AT_BIT_OFFSET: u64 = 0x0c;
const DW_AT_BIT_SIZE: u64 = 0x0d;
const DW_AT_STMT_LIST: u64 = 0x10;
const DW_AT_LOW_PC: u64 = 0x11;
const DW_AT_HIGH_PC: u64 = 0x12;
const DW_AT_COMP_DIR: u64 = 0x1b;
const DW_AT_LOWER_BOUND: u64 = 0x22;
const DW_AT_UPPER_BOUND: u64 = 0x2f;
const DW_AT_ABSTRACT_ORIGIN: u64 = 0x31;
const DW_AT_COUNT: u64 = 0x37;
const DW_AT_DATA_MEMBER_LOCATION: u64 = 0x38;
const DW_AT_DECLARATION: u64 = 0x3c;
const DW_AT_SPECIFICATION: u64 = 0x47;
const DW_AT_TYPE: u64 = 0x49;
const DW_AT_RANGES: u64 = 0x55;
const DW_AT_DATA_BIT_OFFSET: u64 = 0x6b;
const DW_AT_LINKAGE_NAME: u64 = 0x6e;
const DW_AT_STR_OFFSETS_BASE: u64 = 0x72;
const DW_AT_ADDR_BASE: u64 = 0x73;
const DW_AT_RNGLISTS_BASE: u64 = 0x74;
const DW_AT_MIPS_LINKAGE_NAME: u64 = 0x2007;
const DW_OP_CONSTU: u8 = 0x10;
const DW_OP_PLUS: u8 = 0x22;
const DW_OP_PLUS_UCONST: u8 = 0x23;
/// `DW_AT_type` hops from a member to a type: typedefs and qualifiers
/// nest a handful deep; a cycle in a hostile file ends here.
const MAX_TYPE_HOPS: u32 = 64;
/// `DW_AT_specification`/`DW_AT_abstract_origin` hops to a name.
const MAX_NAME_HOPS: u32 = 8;
/// Nested arrays a size question follows.
const MAX_ARRAY_DEPTH: u32 = 16;
/// Units past this are not a real file's (a kernel's has a few thousand).
const MAX_UNITS: usize = 1 << 20;
const MAX_NAME_BYTES: usize = 4096;

/// Which file a unit lies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Main,
    Supplement,
}

/// One file's DWARF sections, owned, so the tables outlive the file's
/// bytes.
#[derive(Default)]
struct Sections {
    info: Vec<u8>,
    abbrev: Vec<u8>,
    str: Vec<u8>,
    line: Vec<u8>,
    line_str: Vec<u8>,
    str_offsets: Vec<u8>,
    addr: Vec<u8>,
    ranges: Vec<u8>,
    rnglists: Vec<u8>,
    types: Vec<u8>,
}

/// A section's bytes, decompressed if need be, copied into a `Vec` whose
/// room is reserved first.
fn owned(file: &ElfFile<'_>, name: &str) -> Result<Vec<u8>, ParseError> {
    let Some(section) = file.section(name) else {
        return Ok(Vec::new());
    };
    let bytes = file.section_bytes(section)?;
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.as_slice().len())
        .map_err(|_| ParseError::Allocation { what: "DWARF section" })?;
    out.extend_from_slice(bytes.as_slice());
    Ok(out)
}

impl Sections {
    fn read(file: &ElfFile<'_>) -> Result<Sections, ParseError> {
        Ok(Sections {
            info: owned(file, ".debug_info")?,
            abbrev: owned(file, ".debug_abbrev")?,
            str: owned(file, ".debug_str")?,
            line: owned(file, ".debug_line")?,
            line_str: owned(file, ".debug_line_str")?,
            str_offsets: owned(file, ".debug_str_offsets")?,
            addr: owned(file, ".debug_addr")?,
            ranges: owned(file, ".debug_ranges")?,
            rnglists: owned(file, ".debug_rnglists")?,
            types: owned(file, ".debug_types")?,
        })
    }
}

/// A unit's `DW_AT_*_base` attributes and its base address.
#[derive(Clone, Copy, Debug, Default)]
struct Bases {
    str_offsets: Option<u64>,
    addr: Option<u64>,
    rnglists: Option<u64>,
    low_pc: u64,
}

/// One indexed unit.
struct UnitInfo {
    origin: Origin,
    section: UnitSection,
    header: UnitHeader,
    abbreviations: Abbreviations,
    bases: Bases,
}

/// An entry anywhere: its unit (an index into the tables' units) and its
/// section offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EntryRef {
    unit: usize,
    offset: usize,
}

/// A named address range, at link-time addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Span {
    name: String,
    start: u64,
    end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NamedAddress {
    name: String,
    address: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineRange {
    start: u64,
    end: u64,
    file: usize,
    line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NamedEntry {
    name: String,
    entry: EntryRef,
}

/// What one unit holds.
#[derive(Default)]
struct UnitFindings {
    functions: Vec<Span>,
    types: Vec<NamedEntry>,
    lines: Vec<LineRange>,
    files: Vec<SourceFile>,
}

/// A member, by entry and name.
struct MemberRef {
    entry: EntryRef,
    name: String,
}

/// Where a member sits in its structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placement {
    offset: u64,
    bits: Option<BitRange>,
}

/// A member's type, past typedefs and qualifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resolved {
    /// A structure, class or union definition.
    Udt(EntryRef),
    Other(EntryRef),
    Void,
}

/// An ELF file's names, lines and layouts.
pub struct DwarfTables {
    build_id: Option<BuildId>,
    link_base: u64,
    /// DWARF subprograms' ranges, by start.
    functions: Vec<Span>,
    /// `.symtab`/`.dynsym` functions and objects, by start; a symbol of
    /// size 0 covers up to the next one.
    symbols: Vec<Span>,
    /// Functions, then symbols, sorted by name.
    names: Vec<NamedAddress>,
    lines: LineIndex,
    /// Every unit's files, each path once.
    files: Vec<SourceFile>,
    main: Sections,
    supplement: Option<Sections>,
    units: Vec<UnitInfo>,
    /// Indices into `units`, per origin and section, in offset order.
    main_info: Vec<usize>,
    main_types: Vec<usize>,
    supplement_info: Vec<usize>,
    types_by_name: BTreeMap<String, EntryRef>,
    signatures: BTreeMap<u64, EntryRef>,
    /// Units that did not read, and were passed by.
    broken_units: u32,
}

/// The identity an ELF file declares: its GNU build-id, from its headers
/// alone.
pub fn elf_identity(bytes: &[u8]) -> Result<TablesIdentity, ParseError> {
    let file = ElfFile::parse(bytes)?;
    Ok(TablesIdentity::Elf { build_id: file.build_id()? })
}

/// An offset-sized value at `at`.
fn offset_at(bytes: &[u8], at: u64, format: Format) -> Option<u64> {
    let at = usize::try_from(at).ok()?;
    match format {
        Format::Dwarf32 => Bytes::new(bytes).u32_le(at).ok().map(u64::from),
        Format::Dwarf64 => Bytes::new(bytes).u64_le(at).ok(),
    }
}

/// An unsigned attribute value.
fn unsigned(value: Option<Value<'_>>) -> Option<u64> {
    match value? {
        Value::Unsigned(value) => Some(value),
        Value::Signed(value) => u64::try_from(value).ok(),
        Value::SecOffset(value) => Some(value),
        Value::Address(_)
        | Value::AddressIndex(_)
        | Value::Block(_)
        | Value::String(_)
        | Value::StrOffset(_)
        | Value::LineStrOffset(_)
        | Value::StrIndex(_)
        | Value::SupStrOffset(_)
        | Value::UnitRef(_)
        | Value::InfoRef(_)
        | Value::SupInfoRef(_)
        | Value::Signature(_)
        | Value::RangeListIndex(_)
        | Value::LocationListIndex(_)
        | Value::Flag(_)
        | Value::Data16(_) => None,
    }
}

/// Joins a unit's files to `files` by path, so a file several units name
/// (a header, or a unit's own file named again in its file table) is listed
/// once, with the first checksum any unit recorded for it. `index` maps each
/// listed path to its place. Answers where each of the unit's files landed.
fn join_files(
    files: &mut Vec<SourceFile>,
    index: &mut BTreeMap<String, usize>,
    unit: Vec<SourceFile>,
) -> Result<Vec<usize>, ParseError> {
    let mut landed = Vec::new();
    landed.try_reserve_exact(unit.len()).map_err(|_| ParseError::Allocation { what: "source files" })?;
    for file in unit {
        let place = match index.get(&file.path) {
            Some(&place) => {
                if let Some(known) = files.get_mut(place) {
                    if known.checksum.is_none() {
                        known.checksum = file.checksum;
                    }
                }
                place
            }
            None => {
                let place = files.len();
                index.insert(file.path.clone(), place);
                push(files, file, "source files")?;
                place
            }
        };
        landed.push(place);
    }
    Ok(landed)
}

/// A `DW_AT_data_member_location` expression this reader evaluates:
/// `DW_OP_plus_uconst n`, or `DW_OP_constu n; DW_OP_plus`.
fn location_expression(expression: &[u8]) -> Option<u64> {
    let bytes = Bytes::new(expression);
    match bytes.u8(0).ok()? {
        DW_OP_PLUS_UCONST => {
            let value = bytes.uleb128(1).ok()?;
            (value.next == expression.len()).then_some(value.value)
        }
        DW_OP_CONSTU => {
            let value = bytes.uleb128(1).ok()?;
            let plus = bytes.u8(value.next).ok()?;
            (plus == DW_OP_PLUS && value.next.checked_add(1) == Some(expression.len())).then_some(value.value)
        }
        _ => None,
    }
}

/// The default `DW_AT_*_base` when a unit has none: just past the
/// section's header, whose 32-bit size is `header_32` (64-bit DWARF adds
/// the 8 bytes of its longer length field).
fn default_base(format: Format, header_32: u64) -> u64 {
    match format {
        Format::Dwarf32 => header_32,
        Format::Dwarf64 => header_32.saturating_add(8),
    }
}

impl DwarfTables {
    /// Parses an ELF file and its DWARF.
    pub fn parse(bytes: &[u8]) -> Result<DwarfTables, ParseError> {
        DwarfTables::parse_with_supplement(bytes, None)
    }

    /// Parses an ELF file with the dwz supplementary file its
    /// `.gnu_debugaltlink` names, when that file is at hand: without it,
    /// names and layouts that live there are missing, never wrong.
    pub fn parse_with_supplement(bytes: &[u8], supplement: Option<&[u8]>) -> Result<DwarfTables, ParseError> {
        let file = ElfFile::parse(bytes)?;
        let supplement = match supplement {
            Some(bytes) => Some(Sections::read(&ElfFile::parse(bytes)?)?),
            None => None,
        };
        let mut tables = DwarfTables {
            build_id: file.build_id()?,
            link_base: file.link_base(),
            functions: Vec::new(),
            symbols: Vec::new(),
            names: Vec::new(),
            lines: LineIndex::default(),
            files: Vec::new(),
            main: Sections::read(&file)?,
            supplement,
            units: Vec::new(),
            main_info: Vec::new(),
            main_types: Vec::new(),
            supplement_info: Vec::new(),
            types_by_name: BTreeMap::new(),
            signatures: BTreeMap::new(),
            broken_units: 0,
        };
        tables.read_symbols(&file.symbols()?)?;
        tables.index_units(Origin::Main, UnitSection::Info)?;
        tables.index_units(Origin::Main, UnitSection::Types)?;
        if tables.supplement.is_some() {
            tables.index_units(Origin::Supplement, UnitSection::Info)?;
        }
        let mut file_index = BTreeMap::new();
        let mut rows = Vec::new();
        for unit in 0..tables.units.len() {
            match tables.scan_unit(unit) {
                Ok(findings) => tables.merge(findings, &mut file_index, &mut rows)?,
                Err(_) => tables.broken_units = tables.broken_units.saturating_add(1),
            }
        }
        tables.functions.sort_unstable_by_key(|function| function.start);
        let mut spans = Vec::new();
        spans
            .try_reserve_exact(tables.functions.len())
            .map_err(|_| ParseError::Allocation { what: "function spans" })?;
        let link_base = tables.link_base;
        spans.extend(tables.functions.iter().filter_map(|function| {
            Some(CodeSpan { start: function.start.checked_sub(link_base)?, end: function.end.checked_sub(link_base)? })
        }));
        tables.lines = LineIndex::build(rows, &spans)?;
        for function in &tables.functions {
            push(&mut tables.names, NamedAddress { name: function.name.clone(), address: function.start }, "name index")
                .map_err(ParseError::from)?;
        }
        for symbol in &tables.symbols {
            push(&mut tables.names, NamedAddress { name: symbol.name.clone(), address: symbol.start }, "name index")
                .map_err(ParseError::from)?;
        }
        tables.names.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tables)
    }

    /// `.symtab`/`.dynsym` functions and objects, sorted, each sizeless
    /// one reaching to the next.
    fn read_symbols(&mut self, symbols: &[ElfSymbol]) -> Result<(), ParseError> {
        for symbol in symbols {
            let span = Span { name: symbol.name.clone(), start: symbol.address, end: symbol.address.saturating_add(symbol.size) };
            push(&mut self.symbols, span, "symbols")?;
        }
        self.symbols.sort_unstable_by(|a, b| a.start.cmp(&b.start).then_with(|| a.name.cmp(&b.name)));
        self.symbols.dedup_by(|later, earlier| later.start == earlier.start && later.name == earlier.name);
        let starts: Vec<u64> = self.symbols.iter().map(|symbol| symbol.start).collect();
        for (index, symbol) in self.symbols.iter_mut().enumerate() {
            if symbol.end == symbol.start {
                let next = starts.iter().skip(index.saturating_add(1)).find(|start| **start > symbol.start);
                symbol.end = next.copied().unwrap_or(symbol.start.saturating_add(1));
            }
        }
        Ok(())
    }

    fn sections(&self, origin: Origin) -> Option<&Sections> {
        match origin {
            Origin::Main => Some(&self.main),
            Origin::Supplement => self.supplement.as_ref(),
        }
    }

    fn unit_bytes(&self, origin: Origin, section: UnitSection) -> &[u8] {
        match (self.sections(origin), section) {
            (Some(sections), UnitSection::Info) => &sections.info,
            (Some(sections), UnitSection::Types) => &sections.types,
            (None, UnitSection::Info | UnitSection::Types) => &[],
        }
    }

    /// Indexes every unit of one section: headers, abbreviations, bases,
    /// and type-unit signatures. A unit whose header does not read ends
    /// the section's walk (the next unit's start is unknown).
    fn index_units(&mut self, origin: Origin, section: UnitSection) -> Result<(), ParseError> {
        let mut at = 0usize;
        loop {
            let bytes = self.unit_bytes(origin, section);
            if at >= bytes.len() {
                return Ok(());
            }
            let Ok(header) = decode_unit_header(Bytes::new(bytes), at, section) else {
                self.broken_units = self.broken_units.saturating_add(1);
                return Ok(());
            };
            at = header.end;
            if self.units.len() >= MAX_UNITS {
                return Err(DwarfError::TooMany { what: "units", count: self.units.len() as u64 }.into());
            }
            let abbreviations = match self.sections(origin) {
                Some(sections) => unit_abbreviations(&header, Bytes::new(&sections.abbrev)),
                None => return Ok(()),
            };
            let Ok(abbreviations) = abbreviations else {
                self.broken_units = self.broken_units.saturating_add(1);
                continue;
            };
            let bases = {
                let context = UnitContext::new(Bytes::new(self.unit_bytes(origin, section)), header, &abbreviations);
                root_bases(&context).unwrap_or_default()
            };
            let index = self.units.len();
            push(&mut self.units, UnitInfo { origin, section, header, abbreviations, bases }, "units")?;
            let list = match (origin, section) {
                (Origin::Main, UnitSection::Info) => &mut self.main_info,
                (Origin::Main, UnitSection::Types) => &mut self.main_types,
                (Origin::Supplement, UnitSection::Info | UnitSection::Types) => &mut self.supplement_info,
            };
            push(list, index, "unit index")?;
            if let UnitKind::Type { signature, type_offset } = header.kind {
                let offset = usize::try_from(type_offset).ok().and_then(|offset| header.offset.checked_add(offset));
                if let Some(offset) = offset {
                    self.signatures.entry(signature).or_insert(EntryRef { unit: index, offset });
                }
            }
        }
    }

    /// A unit's line ranges become rows, by RVA, with their files
    /// renumbered into the tables' file list; a range whose file index the
    /// unit's table does not hold names no file, and one below the link base
    /// lies outside the image and is left out.
    fn merge(
        &mut self,
        findings: UnitFindings,
        file_index: &mut BTreeMap<String, usize>,
        rows: &mut Vec<LineRow>,
    ) -> Result<(), ParseError> {
        let renumbered = join_files(&mut self.files, file_index, findings.files)?;
        for range in findings.lines {
            let file = renumbered.get(range.file).copied().unwrap_or(usize::MAX);
            let Some(start) = range.start.checked_sub(self.link_base) else {
                continue;
            };
            let Some(end) = range.end.checked_sub(self.link_base) else {
                continue;
            };
            push(rows, LineRow { start: Rva(start), end: Rva(end), file, line: range.line }, "line ranges")?;
        }
        for function in findings.functions {
            push(&mut self.functions, function, "functions")?;
        }
        for named in findings.types {
            self.types_by_name.entry(named.name).or_insert(named.entry);
        }
        Ok(())
    }

    fn context(&self, unit: usize) -> Result<UnitContext<'_>, DwarfError> {
        let info = self.units.get(unit).ok_or(DwarfError::BadReference { offset: unit as u64 })?;
        Ok(UnitContext::new(Bytes::new(self.unit_bytes(info.origin, info.section)), info.header, &info.abbreviations))
    }

    /// A string-valued attribute's text.
    fn text(&self, unit: usize, value: Value<'_>) -> Option<String> {
        let info = self.units.get(unit)?;
        let sections = self.sections(info.origin)?;
        let at = |bytes: &[u8], offset: u64| -> Option<String> {
            let text = Bytes::new(bytes).cstr(usize::try_from(offset).ok()?, MAX_NAME_BYTES).ok()?;
            Some(String::from_utf8_lossy(text).into_owned())
        };
        match value {
            Value::String(text) => Some(String::from_utf8_lossy(text).into_owned()),
            Value::StrOffset(offset) => at(&sections.str, offset),
            Value::LineStrOffset(offset) => at(&sections.line_str, offset),
            Value::StrIndex(index) => {
                let format = info.header.format;
                let base = info.bases.str_offsets.unwrap_or(default_base(format, 8));
                let entry = index.checked_mul(format.offset_size() as u64)?.checked_add(base)?;
                at(&sections.str, offset_at(&sections.str_offsets, entry, format)?)
            }
            Value::SupStrOffset(offset) => at(&self.supplement.as_ref()?.str, offset),
            Value::Address(_)
            | Value::AddressIndex(_)
            | Value::Unsigned(_)
            | Value::Signed(_)
            | Value::Block(_)
            | Value::UnitRef(_)
            | Value::InfoRef(_)
            | Value::SupInfoRef(_)
            | Value::Signature(_)
            | Value::SecOffset(_)
            | Value::RangeListIndex(_)
            | Value::LocationListIndex(_)
            | Value::Flag(_)
            | Value::Data16(_) => None,
        }
    }

    /// An address-valued attribute's address.
    fn address(&self, unit: usize, value: Value<'_>) -> Option<u64> {
        match value {
            Value::Address(address) => Some(address),
            Value::AddressIndex(index) => self.indexed_address(unit, index),
            Value::Unsigned(_)
            | Value::Signed(_)
            | Value::Block(_)
            | Value::String(_)
            | Value::StrOffset(_)
            | Value::LineStrOffset(_)
            | Value::StrIndex(_)
            | Value::SupStrOffset(_)
            | Value::UnitRef(_)
            | Value::InfoRef(_)
            | Value::SupInfoRef(_)
            | Value::Signature(_)
            | Value::SecOffset(_)
            | Value::RangeListIndex(_)
            | Value::LocationListIndex(_)
            | Value::Flag(_)
            | Value::Data16(_) => None,
        }
    }

    /// `.debug_addr` entry `index` of `unit`.
    fn indexed_address(&self, unit: usize, index: u64) -> Option<u64> {
        let info = self.units.get(unit)?;
        let sections = self.sections(info.origin)?;
        let size = u64::from(info.header.address_size);
        let base = info.bases.addr.unwrap_or(default_base(info.header.format, 8));
        let at = usize::try_from(index.checked_mul(size)?.checked_add(base)?).ok()?;
        match size {
            4 => Bytes::new(&sections.addr).u32_le(at).ok().map(u64::from),
            _ => Bytes::new(&sections.addr).u64_le(at).ok(),
        }
    }

    /// The unit of `list` whose entries contain section offset `offset`.
    fn unit_containing(&self, list: &[usize], offset: usize) -> Option<usize> {
        let position = list
            .partition_point(|unit| self.units.get(*unit).is_some_and(|info| info.header.offset <= offset))
            .checked_sub(1)?;
        let unit = *list.get(position)?;
        let info = self.units.get(unit)?;
        (offset < info.header.end).then_some(unit)
    }

    /// The entry a reference attribute names.
    fn reference(&self, unit: usize, value: Value<'_>) -> Option<EntryRef> {
        let info = self.units.get(unit)?;
        match value {
            Value::UnitRef(relative) => {
                let offset = info.header.offset.checked_add(usize::try_from(relative).ok()?)?;
                Some(EntryRef { unit, offset })
            }
            Value::InfoRef(absolute) => {
                let offset = usize::try_from(absolute).ok()?;
                let list = match info.origin {
                    Origin::Main => &self.main_info,
                    Origin::Supplement => &self.supplement_info,
                };
                Some(EntryRef { unit: self.unit_containing(list, offset)?, offset })
            }
            Value::SupInfoRef(absolute) => {
                let offset = usize::try_from(absolute).ok()?;
                Some(EntryRef { unit: self.unit_containing(&self.supplement_info, offset)?, offset })
            }
            Value::Signature(signature) => self.signatures.get(&signature).copied(),
            Value::Address(_)
            | Value::AddressIndex(_)
            | Value::Unsigned(_)
            | Value::Signed(_)
            | Value::Block(_)
            | Value::String(_)
            | Value::StrOffset(_)
            | Value::LineStrOffset(_)
            | Value::StrIndex(_)
            | Value::SupStrOffset(_)
            | Value::SecOffset(_)
            | Value::RangeListIndex(_)
            | Value::LocationListIndex(_)
            | Value::Flag(_)
            | Value::Data16(_) => None,
        }
    }

    /// An entry's name: its own, its linkage name, or that of the entry
    /// its `DW_AT_specification` or `DW_AT_abstract_origin` names.
    fn entry_name(&self, start: EntryRef) -> Result<Option<String>, DwarfError> {
        let mut current = start;
        for _ in 0..MAX_NAME_HOPS {
            let context = self.context(current.unit)?;
            let entry = context.entry_at(current.offset)?;
            for name in [DW_AT_NAME, DW_AT_LINKAGE_NAME, DW_AT_MIPS_LINKAGE_NAME] {
                if let Some(text) = context.attribute(&entry, name)?.and_then(|value| self.text(current.unit, value)) {
                    return Ok(Some(text));
                }
            }
            let next = match context.attribute(&entry, DW_AT_SPECIFICATION)? {
                Some(value) => Some(value),
                None => context.attribute(&entry, DW_AT_ABSTRACT_ORIGIN)?,
            };
            match next.and_then(|value| self.reference(current.unit, value)) {
                Some(target) => current = target,
                None => return Ok(None),
            }
        }
        Ok(None)
    }

    /// An entry's code ranges: `DW_AT_low_pc` with `DW_AT_high_pc` (an
    /// address, or a length from DWARF 4 on), else `DW_AT_ranges`.
    fn entry_ranges(&self, unit: usize, context: &UnitContext<'_>, entry: &Entry) -> Result<Vec<Range>, DwarfError> {
        let mut ranges = Vec::new();
        if let Some(low) = context.attribute(entry, DW_AT_LOW_PC)?.and_then(|value| self.address(unit, value)) {
            let high = match context.attribute(entry, DW_AT_HIGH_PC)? {
                Some(Value::Unsigned(length)) => Some(low.saturating_add(length)),
                Some(value) => self.address(unit, value),
                None => None,
            };
            if let Some(high) = high.filter(|high| *high > low) {
                push(&mut ranges, Range { start: low, end: high }, "ranges")?;
            }
            return Ok(ranges);
        }
        let Some(value) = context.attribute(entry, DW_AT_RANGES)? else {
            return Ok(ranges);
        };
        let info = self.units.get(unit).ok_or(DwarfError::BadReference { offset: unit as u64 })?;
        let sections = self.sections(info.origin).ok_or(DwarfError::BadReference { offset: unit as u64 })?;
        if info.header.version >= 5 {
            let rnglists = Bytes::new(&sections.rnglists);
            let at = if let Value::RangeListIndex(index) = value {
                let base = info.bases.rnglists.unwrap_or(default_base(info.header.format, 12));
                rnglist_offset(rnglists, base, index, info.header.format)?
            } else {
                usize::try_from(unsigned(Some(value)).ok_or(DwarfError::BadReference { offset: 0 })?)
                    .map_err(|_| DwarfError::BadReference { offset: 0 })?
            };
            debug_rnglists(rnglists, at, &context.encoding, info.bases.low_pc, |index| self.indexed_address(unit, index))
        } else {
            let at = usize::try_from(unsigned(Some(value)).ok_or(DwarfError::BadReference { offset: 0 })?)
                .map_err(|_| DwarfError::BadReference { offset: 0 })?;
            debug_ranges(Bytes::new(&sections.ranges), at, info.header.address_size, info.bases.low_pc)
        }
    }

    /// Every function and named type of a unit, and its line table.
    fn scan_unit(&self, unit: usize) -> Result<UnitFindings, DwarfError> {
        let context = self.context(unit)?;
        let info = self.units.get(unit).ok_or(DwarfError::BadReference { offset: unit as u64 })?;
        let mut findings = UnitFindings::default();
        let mut at = info.header.entries_at;
        while at < info.header.end {
            let entry = match context.step(at)? {
                Step::Null { next } => {
                    at = next;
                    continue;
                }
                Step::Entry(entry) => entry,
            };
            at = entry.next;
            let here = EntryRef { unit, offset: entry.offset };
            match entry.tag {
                DW_TAG_SUBPROGRAM => {
                    let ranges = self.entry_ranges(unit, &context, &entry)?;
                    if ranges.is_empty() {
                        continue;
                    }
                    if let Some(name) = self.entry_name(here)? {
                        for range in ranges {
                            push(&mut findings.functions, Span { name: name.clone(), start: range.start, end: range.end }, "functions")?;
                        }
                    }
                }
                DW_TAG_STRUCTURE_TYPE | DW_TAG_CLASS_TYPE | DW_TAG_UNION_TYPE => {
                    if context.attribute(&entry, DW_AT_DECLARATION)?.is_some() {
                        continue;
                    }
                    if let Some(name) = context.attribute(&entry, DW_AT_NAME)?.and_then(|value| self.text(unit, value)) {
                        push(&mut findings.types, NamedEntry { name, entry: here }, "types")?;
                    }
                }
                _ => {}
            }
        }
        if info.origin == Origin::Main && info.section == UnitSection::Info {
            self.scan_lines(unit, &context, &mut findings)?;
        }
        Ok(findings)
    }

    /// The unit's line program, as ranges: each row covers the addresses
    /// up to the next row of its sequence.
    fn scan_lines(&self, unit: usize, context: &UnitContext<'_>, findings: &mut UnitFindings) -> Result<(), DwarfError> {
        let root = context.entry_at(context.header.entries_at)?;
        let Some(offset) = unsigned(context.attribute(&root, DW_AT_STMT_LIST)?) else {
            return Ok(());
        };
        let at = usize::try_from(offset).map_err(|_| DwarfError::BadReference { offset })?;
        let comp_dir = context.attribute(&root, DW_AT_COMP_DIR)?.and_then(|value| self.text(unit, value)).unwrap_or_default();
        let unit_name = context.attribute(&root, DW_AT_NAME)?.and_then(|value| self.text(unit, value)).unwrap_or_default();
        let line_context = LineContext { encoding: context.encoding, comp_dir, unit_name };
        let program = parse_line_program(Bytes::new(&self.main.line), at, &line_context, |value| self.text(unit, value))?;
        for (index, row) in program.rows.iter().enumerate() {
            if row.sequence == Sequence::Ends {
                continue;
            }
            let Some(next) = program.rows.get(index.saturating_add(1)) else {
                continue;
            };
            if next.address > row.address {
                let file = usize::try_from(row.file).unwrap_or(usize::MAX);
                push(&mut findings.lines, LineRange { start: row.address, end: next.address, file, line: row.line }, "line ranges")?;
            }
        }
        findings.files = program.files;
        Ok(())
    }

    #[must_use]
    pub fn identity(&self) -> TablesIdentity {
        TablesIdentity::Elf { build_id: self.build_id.clone() }
    }

    /// The link-time address of the image's first mapped byte: an `Rva` is
    /// an offset from it.
    #[must_use]
    pub fn link_base(&self) -> u64 {
        self.link_base
    }

    /// The DWARF function `rva` lies in, else the `.symtab` symbol, with
    /// the line covering it.
    #[must_use]
    pub fn symbolize(&self, rva: Rva) -> Option<SymbolHit> {
        let address = self.link_base.checked_add(rva.0)?;
        let containing = |spans: &[Span]| -> Option<Span> {
            let index = spans.partition_point(|span| span.start <= address).checked_sub(1)?;
            spans.get(index).filter(|span| address < span.end).cloned()
        };
        let line = self
            .lines
            .line_at(rva)
            .and_then(|row| Some(SourceLine { file: self.files.get(row.file)?.path.clone(), line: row.line }));
        if let Some(function) = containing(&self.functions) {
            return Some(SymbolHit { name: function.name, offset: address.saturating_sub(function.start), line });
        }
        let symbol = containing(&self.symbols)?;
        Some(SymbolHit { name: symbol.name, offset: address.saturating_sub(symbol.start), line })
    }

    #[must_use]
    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    #[must_use]
    pub fn line_at(&self, rva: Rva) -> Option<LineRow> {
        self.lines.line_at(rva)
    }

    #[must_use]
    pub fn lines_of(&self, file: &str, line: u32) -> Vec<ResolvedLine> {
        self.lines.lines_of(&self.files, file, line)
    }

    #[must_use]
    pub fn code_lines(&self, file: usize) -> Vec<u32> {
        self.lines.code_lines(file)
    }

    /// The DWARF subprogram holding `rva`.
    #[must_use]
    pub fn procedure_starts(&self) -> Vec<Rva> {
        let mut starts: Vec<Rva> =
            self.functions.iter().filter_map(|span| span.start.checked_sub(self.link_base).map(Rva)).collect();
        starts.dedup();
        starts
    }

    #[must_use]
    pub fn procedure_at(&self, rva: Rva) -> Option<ProcedureSpan> {
        let address = self.link_base.checked_add(rva.0)?;
        let index = self.functions.partition_point(|span| span.start <= address).checked_sub(1)?;
        let function = self.functions.get(index).filter(|span| address < span.end)?;
        Some(ProcedureSpan {
            name: function.name.clone(),
            start: Rva(function.start.checked_sub(self.link_base)?),
            end: Rva(function.end.checked_sub(self.link_base)?),
        })
    }

    /// A DWARF function's address, else a symbol's.
    #[must_use]
    pub fn address_of(&self, name: &str) -> Option<Rva> {
        let first = self.names.partition_point(|named| named.name.as_str() < name);
        let named = self.names.get(first).filter(|named| named.name == name)?;
        named.address.checked_sub(self.link_base).map(Rva)
    }

    /// The definition named `name`, or why a miss is not an answer.
    fn definition(&self, name: &str) -> Result<Option<EntryRef>, LayoutError> {
        match self.types_by_name.get(name) {
            Some(entry) => Ok(Some(*entry)),
            None if self.broken_units > 0 => {
                Err(LayoutError::UnreadableTypeRecords { name: name.to_owned(), count: self.broken_units })
            }
            None => Ok(None),
        }
    }

    fn entry(&self, at: EntryRef) -> Result<Entry, LayoutError> {
        self.context(at.unit)
            .and_then(|context| context.entry_at(at.offset))
            .map_err(|_| LayoutError::Unreadable { what: "DWARF entry" })
    }

    fn attribute(&self, at: EntryRef, name: u64) -> Result<Option<Value<'_>>, LayoutError> {
        let context = self.context(at.unit).map_err(|_| LayoutError::Unreadable { what: "DWARF unit" })?;
        let entry = context.entry_at(at.offset).map_err(|_| LayoutError::Unreadable { what: "DWARF entry" })?;
        context.attribute(&entry, name).map_err(|_| LayoutError::Unreadable { what: "DWARF attribute" })
    }

    /// The direct `DW_TAG_member` children of `parent`.
    fn members(&self, parent: EntryRef) -> Result<Vec<MemberRef>, LayoutError> {
        let unreadable = |_| LayoutError::Unreadable { what: "DWARF members" };
        let context = self.context(parent.unit).map_err(unreadable)?;
        let entry = context.entry_at(parent.offset).map_err(unreadable)?;
        let mut members = Vec::new();
        if entry.children == Children::No {
            return Ok(members);
        }
        let mut depth = 1u32;
        let mut at = entry.next;
        while depth > 0 && at < context.header.end {
            match context.step(at).map_err(unreadable)? {
                Step::Null { next } => {
                    depth = depth.saturating_sub(1);
                    at = next;
                }
                Step::Entry(child) => {
                    if depth == 1 && child.tag == DW_TAG_MEMBER {
                        let here = EntryRef { unit: parent.unit, offset: child.offset };
                        let name = context
                            .attribute(&child, DW_AT_NAME)
                            .map_err(unreadable)?
                            .and_then(|value| self.text(parent.unit, value))
                            .unwrap_or_default();
                        members.try_reserve(1).map_err(|_| LayoutError::Unreadable { what: "members (no memory)" })?;
                        members.push(MemberRef { entry: here, name });
                    }
                    if child.children == Children::Yes {
                        depth = depth.saturating_add(1);
                    }
                    at = child.next;
                }
            }
        }
        Ok(members)
    }

    /// Where `member` sits: its byte offset (`DW_AT_data_member_location`),
    /// and for a bitfield its bits, from DWARF 4's `DW_AT_data_bit_offset`
    /// (counted from the structure's start) or DWARF 2's `DW_AT_bit_offset`
    /// (counted from the storage unit's most significant bit).
    fn placement(&self, member: &MemberRef) -> Result<Placement, LayoutError> {
        let unsupported = || LayoutError::UnsupportedMemberLocation { member: member.name.clone() };
        let bad = || LayoutError::BadNumber { name: member.name.clone() };
        let location = match self.attribute(member.entry, DW_AT_DATA_MEMBER_LOCATION)? {
            None => 0,
            Some(Value::Unsigned(value)) => value,
            Some(Value::Signed(value)) => u64::try_from(value).map_err(|_| bad())?,
            Some(Value::Block(expression)) => location_expression(expression).ok_or_else(unsupported)?,
            Some(_) => return Err(unsupported()),
        };
        let Some(bit_size) = unsigned(self.attribute(member.entry, DW_AT_BIT_SIZE)?) else {
            return Ok(Placement { offset: location, bits: None });
        };
        let length = u32::try_from(bit_size).map_err(|_| bad())?;
        if let Some(bit_offset) = unsigned(self.attribute(member.entry, DW_AT_DATA_BIT_OFFSET)?) {
            let unit_bytes = self.size_of(self.member_type(member.entry)?, 0)?.filter(|size| *size > 0).unwrap_or(1);
            let unit_bits = unit_bytes.checked_mul(8).ok_or_else(bad)?;
            let total = location.checked_mul(8).and_then(|bits| bits.checked_add(bit_offset)).ok_or_else(bad)?;
            let position = total.checked_rem(unit_bits).ok_or_else(bad)?;
            let placement = if position.saturating_add(bit_size) > unit_bits {
                Placement {
                    offset: total.checked_div(8).ok_or_else(bad)?,
                    bits: Some(BitRange { position: u32::try_from(total.checked_rem(8).ok_or_else(bad)?).map_err(|_| bad())?, length }),
                }
            } else {
                Placement {
                    offset: total.checked_div(unit_bits).and_then(|units| units.checked_mul(unit_bytes)).ok_or_else(bad)?,
                    bits: Some(BitRange { position: u32::try_from(position).map_err(|_| bad())?, length }),
                }
            };
            return Ok(placement);
        }
        let Some(bit_offset) = unsigned(self.attribute(member.entry, DW_AT_BIT_OFFSET)?) else {
            return Ok(Placement { offset: location, bits: Some(BitRange { position: 0, length }) });
        };
        let byte_size = match unsigned(self.attribute(member.entry, DW_AT_BYTE_SIZE)?) {
            Some(size) => size,
            None => self.size_of(self.member_type(member.entry)?, 0)?.ok_or_else(bad)?,
        };
        let position = byte_size
            .checked_mul(8)
            .and_then(|bits| bits.checked_sub(bit_offset))
            .and_then(|bits| bits.checked_sub(bit_size))
            .ok_or_else(bad)?;
        Ok(Placement { offset: location, bits: Some(BitRange { position: u32::try_from(position).map_err(|_| bad())?, length }) })
    }

    /// The type of `member`, past typedefs and qualifiers. A type in a dwz
    /// supplementary file that is not loaded is an error, never a guess.
    fn member_type(&self, member: EntryRef) -> Result<Resolved, LayoutError> {
        let Some(value) = self.attribute(member, DW_AT_TYPE)? else {
            return Ok(Resolved::Void);
        };
        if let Value::SupInfoRef(offset) = value {
            if self.supplement.is_none() {
                return Err(LayoutError::MissingSupplement { offset });
            }
        }
        match self.reference(member.unit, value) {
            Some(start) => self.resolve_type(start),
            None => Ok(Resolved::Void),
        }
    }

    /// `start` past typedefs and qualifiers, a declaration resolved to its
    /// definition by name; at most `MAX_TYPE_HOPS` hops.
    fn resolve_type(&self, start: EntryRef) -> Result<Resolved, LayoutError> {
        let mut current = start;
        for _ in 0..MAX_TYPE_HOPS {
            let entry = self.entry(current)?;
            match entry.tag {
                DW_TAG_TYPEDEF | DW_TAG_CONST_TYPE | DW_TAG_VOLATILE_TYPE | DW_TAG_RESTRICT_TYPE | DW_TAG_ATOMIC_TYPE => {
                    match self.attribute(current, DW_AT_TYPE)?.and_then(|value| self.reference(current.unit, value)) {
                        Some(next) => current = next,
                        None => return Ok(Resolved::Void),
                    }
                }
                DW_TAG_STRUCTURE_TYPE | DW_TAG_CLASS_TYPE | DW_TAG_UNION_TYPE => {
                    if self.attribute(current, DW_AT_DECLARATION)?.is_none() {
                        return Ok(Resolved::Udt(current));
                    }
                    let name = self.attribute(current, DW_AT_NAME)?.and_then(|value| self.text(current.unit, value));
                    return Ok(match name.and_then(|name| self.types_by_name.get(&name).copied()) {
                        Some(definition) => Resolved::Udt(definition),
                        None => Resolved::Other(current),
                    });
                }
                _ => return Ok(Resolved::Other(current)),
            }
        }
        Err(LayoutError::EndlessTypeChain { index: start.offset as u64 })
    }

    /// The size of a resolved type in bytes, when it has one.
    fn size_of(&self, resolved: Resolved, depth: u32) -> Result<Option<u64>, LayoutError> {
        let at = match resolved {
            Resolved::Void => return Ok(None),
            Resolved::Udt(at) | Resolved::Other(at) => at,
        };
        if let Some(size) = unsigned(self.attribute(at, DW_AT_BYTE_SIZE)?) {
            return Ok(Some(size));
        }
        let entry = self.entry(at)?;
        match entry.tag {
            DW_TAG_POINTER_TYPE | DW_TAG_REFERENCE_TYPE | DW_TAG_RVALUE_REFERENCE_TYPE | DW_TAG_PTR_TO_MEMBER_TYPE => {
                let info = self.units.get(at.unit).ok_or(LayoutError::Unreadable { what: "DWARF unit" })?;
                Ok(Some(u64::from(info.header.address_size)))
            }
            DW_TAG_ARRAY_TYPE if depth < MAX_ARRAY_DEPTH => self.array_size(at, depth),
            DW_TAG_ENUMERATION_TYPE => self.size_of(self.member_type(at)?, depth.saturating_add(1)),
            _ => Ok(None),
        }
    }

    /// An array's size: its element's size times each dimension's count.
    fn array_size(&self, array: EntryRef, depth: u32) -> Result<Option<u64>, LayoutError> {
        let Some(element) = self.size_of(self.member_type(array)?, depth.saturating_add(1))? else {
            return Ok(None);
        };
        let unreadable = |_| LayoutError::Unreadable { what: "DWARF array" };
        let context = self.context(array.unit).map_err(unreadable)?;
        let entry = context.entry_at(array.offset).map_err(unreadable)?;
        if entry.children == Children::No {
            return Ok(None);
        }
        let mut total = element;
        let mut depth_in = 1u32;
        let mut at = entry.next;
        while depth_in > 0 && at < context.header.end {
            match context.step(at).map_err(unreadable)? {
                Step::Null { next } => {
                    depth_in = depth_in.saturating_sub(1);
                    at = next;
                }
                Step::Entry(child) => {
                    if depth_in == 1 && child.tag == DW_TAG_SUBRANGE_TYPE {
                        let count = match unsigned(context.attribute(&child, DW_AT_COUNT).map_err(unreadable)?) {
                            Some(count) => count,
                            None => {
                                let Some(upper) = unsigned(context.attribute(&child, DW_AT_UPPER_BOUND).map_err(unreadable)?) else {
                                    return Ok(None);
                                };
                                let lower = unsigned(context.attribute(&child, DW_AT_LOWER_BOUND).map_err(unreadable)?).unwrap_or(0);
                                upper.saturating_add(1).saturating_sub(lower)
                            }
                        };
                        total = total.checked_mul(count).ok_or(LayoutError::BadNumber { name: "array".to_owned() })?;
                    }
                    if child.children == Children::Yes {
                        depth_in = depth_in.saturating_add(1);
                    }
                    at = child.next;
                }
            }
        }
        Ok(Some(total))
    }

    /// The layout of the structure, class or union named `name`.
    pub fn layout(&self, name: &str) -> Result<Option<TypeLayout>, LayoutError> {
        let Some(found) = self.definition(name)? else {
            return Ok(None);
        };
        let size = unsigned(self.attribute(found, DW_AT_BYTE_SIZE)?).ok_or(LayoutError::BadNumber { name: name.to_owned() })?;
        let members = self.members(found)?;
        let mut fields = Vec::new();
        fields
            .try_reserve_exact(members.len())
            .map_err(|_| LayoutError::Unreadable { what: "layout (no memory)" })?;
        for member in &members {
            let placement = self.placement(member)?;
            fields.push(FieldLayout { name: member.name.clone(), offset: FieldOffset(placement.offset), bits: placement.bits });
        }
        Ok(Some(TypeLayout { size, fields }))
    }

    /// Where the member reached through `path` sits in `type_name`.
    pub fn member_location(&self, type_name: &str, path: &[&str]) -> Result<Option<MemberLocation>, LayoutError> {
        let Some(mut current) = self.definition(type_name)? else {
            return Ok(None);
        };
        let mut offset = 0u64;
        let mut walked = String::new();
        for (step, name) in path.iter().enumerate() {
            if !walked.is_empty() {
                walked.push('.');
            }
            walked.push_str(name);
            let members = self.members(current)?;
            let Some(member) = members.into_iter().find(|member| member.name == *name) else {
                return Ok(None);
            };
            let placement = self.placement(&member)?;
            offset = offset.checked_add(placement.offset).ok_or(LayoutError::BadNumber { name: walked.clone() })?;
            let resolved = self.member_type(member.entry)?;
            if step.saturating_add(1) == path.len() {
                return Ok(Some(MemberLocation { offset: FieldOffset(offset), bits: placement.bits, size: self.size_of(resolved, 0)? }));
            }
            current = match resolved {
                Resolved::Udt(next) => next,
                Resolved::Other(_) | Resolved::Void => {
                    let next = path.get(step.saturating_add(1)).copied().unwrap_or("");
                    return Err(LayoutError::NotAStructure { member: format!("{walked}.{next}") });
                }
            };
        }
        Ok(None)
    }
}

/// A unit's bases, from its root entry.
fn root_bases(context: &UnitContext<'_>) -> Result<Bases, DwarfError> {
    let root = context.entry_at(context.header.entries_at)?;
    Ok(Bases {
        str_offsets: unsigned(context.attribute(&root, DW_AT_STR_OFFSETS_BASE)?),
        addr: unsigned(context.attribute(&root, DW_AT_ADDR_BASE)?),
        rnglists: unsigned(context.attribute(&root, DW_AT_RNGLISTS_BASE)?),
        low_pc: match context.attribute(&root, DW_AT_LOW_PC)? {
            Some(Value::Address(address)) => address,
            Some(_) | None => 0,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{BitRange, FieldOffset};
    use crate::tables::{ChecksumKind, FileChecksum, LineRow, ProcedureSpan, ResolvedLine, SourceFile};
    use rusty_box_symbols_fixtures::dwarf::{
        build_dwarf, looping_typedefs, split_forms_dwarf, DwarfFormat, DwarfSpec, MAIN_C_MD5,
    };
    use rusty_box_symbols_fixtures::elf::{
        build_elf, ElfSpec, Encoding, ExtraSection, Symtab, Text, BUILD_ID, FUNC_LEN, TEXT_LEN, TEXT_RVA,
    };

    const MAIN_C: &str = "/build/widget/src/main.c";

    /// A file to read, and what was asked of the fixture.
    struct Shape {
        spec: ElfSpec,
        version: u16,
        format: DwarfFormat,
        bytes: Vec<u8>,
    }

    /// DWARF 2 to 5 (and 64-bit DWARF 5), a PIE and an executable, without
    /// `.symtab`, and the shape of an `objcopy --only-keep-debug` file.
    fn every_shape() -> Vec<Shape> {
        let mut shapes = Vec::new();
        let specs = [
            ElfSpec::pie(),
            ElfSpec::exec(),
            ElfSpec { symtab: Symtab::Without, ..ElfSpec::pie() },
            ElfSpec { text: Text::NoBits, ..ElfSpec::exec() },
        ];
        let encodings = [
            DwarfSpec { version: 2, format: DwarfFormat::Dwarf32, text_va: 0 },
            DwarfSpec { version: 3, format: DwarfFormat::Dwarf32, text_va: 0 },
            DwarfSpec { version: 4, format: DwarfFormat::Dwarf32, text_va: 0 },
            DwarfSpec { version: 5, format: DwarfFormat::Dwarf32, text_va: 0 },
            DwarfSpec { version: 5, format: DwarfFormat::Dwarf64, text_va: 0 },
        ];
        for dwarf in encodings {
            for spec in specs {
                let sections = build_dwarf(DwarfSpec { text_va: spec.link_base + TEXT_RVA, ..dwarf });
                shapes.push(Shape { spec, version: dwarf.version, format: dwarf.format, bytes: build_elf(spec, &sections) });
            }
        }
        shapes
    }

    fn hit(name: &str, offset: u64, line: Option<u32>) -> Option<SymbolHit> {
        Some(SymbolHit { name: name.to_owned(), offset, line: line.map(|line| SourceLine { file: MAIN_C.to_owned(), line }) })
    }

    #[test]
    fn an_elf_file_names_addresses_from_dwarf_then_symtab() {
        for shape in every_shape() {
            let tables = DwarfTables::parse(&shape.bytes).unwrap();
            let named = format!("{:?} DWARF {} {:?}", shape.spec, shape.version, shape.format);
            assert_eq!(tables.link_base(), shape.spec.link_base, "{named}");
            assert_eq!(tables.identity(), TablesIdentity::Elf { build_id: Some(BuildId(BUILD_ID.to_vec())) });
            assert_eq!(tables.symbolize(Rva(TEXT_RVA)), hit("widget_init", 0, Some(10)), "{named}");
            assert_eq!(tables.symbolize(Rva(TEXT_RVA + 9)), hit("widget_init", 9, Some(11)), "{named}");
            // Only `.symtab` knows the helper.
            let helper = tables.symbolize(Rva(TEXT_RVA + FUNC_LEN + 4));
            match shape.spec.symtab {
                Symtab::With => assert_eq!(helper, hit("helper_no_dwarf", 4, None), "{named}"),
                Symtab::Without => assert_eq!(helper, None, "{named}"),
            }
            assert_eq!(tables.symbolize(Rva(TEXT_RVA + TEXT_LEN)), None, "{named}");
            assert_eq!(tables.address_of("widget_init"), Some(Rva(TEXT_RVA)), "{named}");
            assert_eq!(tables.address_of("nope"), None);
        }
    }

    /// DWARF 5 file entries carry the MD5 of the file; earlier versions
    /// have no field for one. The unit's own file and its file entry name
    /// the same path, listed once.
    #[test]
    fn a_dwarf_5_unit_lists_md5_checksums() {
        for shape in every_shape() {
            let tables = DwarfTables::parse(&shape.bytes).unwrap();
            let named = format!("{:?} DWARF {} {:?}", shape.spec, shape.version, shape.format);
            let checksum = (shape.version >= 5).then(|| FileChecksum { kind: ChecksumKind::Md5, bytes: MAIN_C_MD5.to_vec() });
            assert_eq!(tables.files(), [SourceFile { path: MAIN_C.to_owned(), checksum }], "{named}");
        }
    }

    /// `widget_init`: line 10 at `.text`, 11 from +8 to the function's end.
    #[test]
    fn dwarf_line_at_gives_the_whole_row() {
        for shape in every_shape() {
            let tables = DwarfTables::parse(&shape.bytes).unwrap();
            let named = format!("{:?} DWARF {} {:?}", shape.spec, shape.version, shape.format);
            let row = |start: u64, end: u64, line: u32| LineRow { start: Rva(start), end: Rva(end), file: 0, line };
            assert_eq!(tables.line_at(Rva(TEXT_RVA + 9)), Some(row(TEXT_RVA + 8, TEXT_RVA + FUNC_LEN, 11)), "{named}");
            assert_eq!(tables.line_at(Rva(TEXT_RVA)), Some(row(TEXT_RVA, TEXT_RVA + 8, 10)), "{named}");
            assert_eq!(tables.line_at(Rva(TEXT_RVA + FUNC_LEN)), None, "{named}");
        }
    }

    #[test]
    fn dwarf_lines_of_finds_the_line() {
        for shape in every_shape() {
            let tables = DwarfTables::parse(&shape.bytes).unwrap();
            let named = format!("{:?} DWARF {} {:?}", shape.spec, shape.version, shape.format);
            let ten = LineRow { start: Rva(TEXT_RVA), end: Rva(TEXT_RVA + 8), file: 0, line: 10 };
            assert_eq!(tables.lines_of("src/main.c", 10), [ResolvedLine { line: 10, ranges: vec![ten] }], "{named}");
            assert_eq!(tables.lines_of(r"widget\SRC\main.c", 10), [ResolvedLine { line: 10, ranges: vec![ten] }]);
            assert_eq!(tables.lines_of("ain.c", 10), [], "{named}");
            assert_eq!(tables.lines_of("main.c", 12), [], "past the function");
        }
    }

    #[test]
    fn dwarf_procedure_at_gives_the_span() {
        for shape in every_shape() {
            let tables = DwarfTables::parse(&shape.bytes).unwrap();
            let span = ProcedureSpan { name: "widget_init".into(), start: Rva(TEXT_RVA), end: Rva(TEXT_RVA + FUNC_LEN) };
            assert_eq!(tables.procedure_at(Rva(TEXT_RVA + 4)), Some(span));
            // `.symtab`'s helper is a symbol, not a DWARF function.
            assert_eq!(tables.procedure_at(Rva(TEXT_RVA + FUNC_LEN + 4)), None);
        }
    }

    /// A header two units include: listed once, at its first place, and a
    /// checksum the second unit records fills the one the first lacked.
    #[test]
    fn units_naming_one_file_share_its_entry() {
        let md5 = || Some(FileChecksum { kind: ChecksumKind::Md5, bytes: vec![9; 16] });
        let file = |path: &str, checksum: Option<FileChecksum>| SourceFile { path: path.to_owned(), checksum };
        let mut files = Vec::new();
        let mut index = BTreeMap::new();
        let first = join_files(&mut files, &mut index, vec![file("/a.c", None), file("/h.h", None)]).unwrap();
        let second = join_files(&mut files, &mut index, vec![file("/b.c", md5()), file("/h.h", md5())]).unwrap();
        assert_eq!(first, [0, 1]);
        assert_eq!(second, [2, 1]);
        assert_eq!(files, [file("/a.c", None), file("/h.h", md5()), file("/b.c", md5())]);
        // A third unit's other checksum does not replace the one recorded.
        let other = Some(FileChecksum { kind: ChecksumKind::Md5, bytes: vec![1; 16] });
        join_files(&mut files, &mut index, vec![file("/h.h", other)]).unwrap();
        assert_eq!(files.get(1), Some(&file("/h.h", md5())));
    }

    #[test]
    fn a_layout_reads_both_bitfield_encodings_and_skips_declarations() {
        for shape in every_shape() {
            let tables = DwarfTables::parse(&shape.bytes).unwrap();
            let layout = tables.layout("Widget").unwrap().expect("Widget");
            assert_eq!(layout.size, 16, "the definition, not the declaration");
            assert_eq!(
                layout.fields,
                vec![
                    FieldLayout { name: "a".into(), offset: FieldOffset(0), bits: None },
                    FieldLayout { name: "b".into(), offset: FieldOffset(8), bits: None },
                    FieldLayout { name: "c".into(), offset: FieldOffset(4), bits: Some(BitRange { position: 5, length: 3 }) },
                    FieldLayout { name: "d".into(), offset: FieldOffset(4), bits: Some(BitRange { position: 8, length: 2 }) },
                ],
                "DWARF {} {:?}",
                shape.version,
                shape.format
            );
            assert_eq!(tables.layout("Gadget").unwrap(), None);
        }
    }

    #[test]
    fn a_member_path_reaches_into_a_nested_structure_through_its_declaration() {
        let shape = &every_shape()[0];
        let tables = DwarfTables::parse(&shape.bytes).unwrap();
        assert_eq!(
            tables.member_location("Outer", &["inner", "c"]).unwrap(),
            Some(MemberLocation { offset: FieldOffset(12), bits: Some(BitRange { position: 5, length: 3 }), size: Some(4) })
        );
        assert_eq!(
            tables.member_location("Outer", &["inner", "b"]).unwrap(),
            Some(MemberLocation { offset: FieldOffset(16), bits: None, size: Some(8) })
        );
        assert_eq!(tables.member_location("Outer", &["inner", "zz"]).unwrap(), None);
        assert_eq!(
            tables.member_location("Outer", &["x", "y"]),
            Err(LayoutError::NotAStructure { member: "x.y".to_owned() })
        );
    }

    #[test]
    fn strx_addrx_rnglistx_and_type_units_are_read() {
        let bytes = build_elf(ElfSpec::pie(), &split_forms_dwarf(TEXT_RVA));
        let tables = DwarfTables::parse(&bytes).unwrap();
        assert_eq!(tables.symbolize(Rva(TEXT_RVA + 4)).map(|hit| hit.name), Some("split_init".to_owned()));
        let ranged = tables.symbolize(Rva(TEXT_RVA + 0x14)).unwrap();
        assert_eq!(ranged.name, "split_ranges");
        assert_eq!(ranged.offset, 4);
        let point = tables.layout("Point").unwrap().expect("Point, defined in a type unit");
        assert_eq!(point.size, 16);
        assert_eq!(
            tables.member_location("Holder", &["held", "y"]).unwrap(),
            Some(MemberLocation { offset: FieldOffset(16), bits: None, size: Some(8) })
        );
    }

    /// Review focus 3: a `DW_AT_type` chain that loops.
    #[test]
    fn a_type_chain_that_loops_ends_with_an_error() {
        let bytes = build_elf(ElfSpec::pie(), &looping_typedefs());
        let tables = DwarfTables::parse(&bytes).unwrap();
        assert!(matches!(tables.member_location("Loop", &["m"]), Err(LayoutError::EndlessTypeChain { .. })));
        assert!(matches!(tables.layout("Loop"), Ok(Some(_))), "the layout itself does not need the member's type");
    }

    #[test]
    fn compressed_dwarf_reads_like_plain_dwarf() {
        for encoding in [Encoding::Zlib, Encoding::Zstd, Encoding::Zdebug] {
            let sections: Vec<ExtraSection> = build_dwarf(DwarfSpec { version: 5, format: DwarfFormat::Dwarf32, text_va: TEXT_RVA })
                .into_iter()
                .map(|section| ExtraSection { encoding, ..section })
                .collect();
            let tables = DwarfTables::parse(&build_elf(ElfSpec::pie(), &sections)).unwrap();
            assert_eq!(tables.symbolize(Rva(TEXT_RVA)), hit("widget_init", 0, Some(10)), "{encoding:?}");
        }
    }

    #[test]
    fn the_tables_are_shared_across_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<DwarfTables>();
    }

    use rusty_box_symbols_fixtures::dwarf::{dwz_pair, DwzForms};

    #[test]
    fn a_dwz_file_reads_names_and_types_from_its_supplement() {
        for forms in [DwzForms::Gnu, DwzForms::Standard] {
            let pair = dwz_pair(forms, TEXT_RVA);
            let spec = ElfSpec { symtab: Symtab::Without, ..ElfSpec::pie() };
            let main = build_elf(spec, &pair.main);
            let tables = DwarfTables::parse_with_supplement(&main, Some(&pair.supplement)).unwrap();
            assert_eq!(tables.symbolize(Rva(TEXT_RVA)).map(|hit| hit.name), Some("dwz_function".to_owned()), "{forms:?}");
            assert_eq!(tables.symbolize(Rva(TEXT_RVA + 0x14)).map(|hit| hit.name), Some("own_function".to_owned()));
            assert_eq!(
                tables.member_location("Local", &["shared", "value"]).unwrap(),
                Some(MemberLocation { offset: FieldOffset(8), bits: None, size: Some(8) }),
                "{forms:?}"
            );
            assert_eq!(tables.layout("Shared").unwrap().map(|layout| layout.size), Some(8));
        }
    }

    /// Review focus 5: the supplement is unavailable (offline, a 404).
    #[test]
    fn a_dwz_file_without_its_supplement_still_names_its_own_functions() {
        for forms in [DwzForms::Gnu, DwzForms::Standard] {
            let pair = dwz_pair(forms, TEXT_RVA);
            let spec = ElfSpec { symtab: Symtab::Without, ..ElfSpec::pie() };
            let tables = DwarfTables::parse(&build_elf(spec, &pair.main)).unwrap();
            assert_eq!(tables.symbolize(Rva(TEXT_RVA + 0x14)).map(|hit| hit.name), Some("own_function".to_owned()));
            // The name that lives in the supplement is missing, not wrong.
            assert_eq!(tables.symbolize(Rva(TEXT_RVA)), None, "{forms:?}");
            assert_eq!(tables.layout("Shared").unwrap(), None);
            assert!(matches!(
                tables.member_location("Local", &["shared", "value"]),
                Err(LayoutError::MissingSupplement { .. })
            ));
        }
    }
}
