//! PDB files built in memory, from ms-pdb's own pieces: its MSF and MSFZ
//! writers, `PdbiStream::to_bytes`, `SymBuilder`,
//! `build_global_symbols_index` and the zerocopy stream headers. ms-pdb has
//! no whole-PDB writer, so the streams are laid out here as link.exe lays
//! them out: PDBI (1), TPI (2), DBI (3), IPI (4), then the named and indexed
//! streams below.

use std::collections::BTreeMap;
use std::io::{self, Cursor, Write};
use std::sync::Mutex;

use ms_pdb::dbi::section_map::{SectionMapEntry, SectionMapHeader};
use ms_pdb::dbi::{
    DbiStreamHeader, ModuleInfoFixed, SectionContribEntry, DBI_STREAM_VERSION_V70,
    SECTION_CONTRIBUTIONS_SUBSTREAM_VER60,
};
use ms_pdb::globals::build_global_symbols_index;
use ms_pdb::globals::name_table::get_v1_default_bucket;
use ms_pdb::lines::SubsectionKind;
use ms_pdb::modi::CV_SIGNATURE_C13;
use ms_pdb::msf::{CreateOptions, Msf};
use ms_pdb::msfz::{Compression, MsfzWriter};
use ms_pdb::pdbi::{FeatureCode, NamedStreams, PdbiStream, PDBI_VERSION_VC140, PDBI_VERSION_VC70};
use ms_pdb::syms::builder::SymBuilder;
use ms_pdb::syms::{OffsetSegment, ProcFixed, SymKind};
use ms_pdb::tpi::{TypeStreamHeader, TYPE_STREAM_VERSION_2004};
use ms_pdb::types::{Leaf, TypeIndex, TypeIndexLe};
use ms_pdb::{BStr, ReadAt, StreamIndexU16, Uuid, WriteAt};
use zerocopy::{IntoBytes, I32, U16, U32};

const NAMES: u32 = 5;
const GSS: u32 = 6;
const GSI: u32 = 7;
const PSI: u32 = 8;
const MODULE: u32 = 9;
const SECTION_HEADERS: u32 = 10;
const ORIGINAL_SECTION_HEADERS: u32 = 11;
const OMAP_FROM_SOURCE: u32 = 12;

/// `IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ`.
pub const CODE_SECTION: u32 = 0x6000_0020;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE`.
pub const DATA_SECTION: u32 = 0xC000_0040;

/// Which container the PDB is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixtureContainer {
    /// The classic multi-stream file (MSF 7.00).
    Msf,
    /// MSFZ, zstd chunks (ms-pdb's default).
    MsfzZstd,
    /// MSFZ, raw DEFLATE chunks.
    MsfzDeflate,
    /// MSFZ, zstd chunks of 4 KiB, so streams span several chunks.
    MsfzSmallChunks,
    /// MSFZ whose streams are stored uncompressed in the file.
    MsfzStored,
}

/// Whether UDTs are written as the classic records or the `*2` forms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordForm {
    /// `LF_STRUCTURE` (0x1505), `LF_UNION` (0x1506).
    Classic,
    /// `LF_STRUCTURE2` (0x1609), `LF_UNION2` (0x160A).
    Wide,
}

/// An image section as the PDB's section-header stream records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureSection {
    pub name: &'static str,
    pub rva: u32,
    pub size: u32,
    pub characteristics: u32,
}

/// What a public symbol names (`cvpsfFunction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicKind {
    Function,
    Data,
}

/// An `S_PUB32` at `offset` in section `section` (1-based, as CodeView
/// numbers them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixturePublic {
    pub name: String,
    pub section: u16,
    pub offset: u32,
    pub kind: PublicKind,
}

/// One line-table record: `offset` bytes into its procedure starts `line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixtureLine {
    pub offset: u32,
    pub line: u32,
}

/// An `S_GPROC32` with its line table, in the module stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureProcedure {
    pub name: String,
    pub section: u16,
    pub offset: u32,
    pub len: u32,
    pub file: String,
    /// The checksum the file's C13 checksum entry records, if any.
    pub checksum: Option<FixtureChecksum>,
    pub lines: Vec<FixtureLine>,
}

/// A C13 file checksum: `kind` 1 MD5, 2 SHA-1, 3 SHA-256 (another value is
/// written as given), and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureChecksum {
    pub kind: u8,
    pub bytes: Vec<u8>,
}

/// OMAP, as a binary rewritten after linking carries it: the sections as
/// the linker laid them out, and the map from those addresses to the
/// rewritten image's (`OMAP_FROM_SRC`), sorted by source address. A `to` of
/// 0 says the source range was dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureOmap {
    pub original_sections: Vec<FixtureSection>,
    pub from_source: Vec<FixtureOmapEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixtureOmapEntry {
    pub from: u32,
    pub to: u32,
}

/// A member's type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixtureType {
    U8,
    U16,
    U32,
    U64,
    /// `void *` in a 64-bit image.
    Pointer,
    Array { element: Box<FixtureType>, count: u64 },
    /// `length` bits at bit `position` of a `storage` unit.
    Bitfield { storage: Box<FixtureType>, position: u8, length: u8 },
    /// A structure or union of the fixture, by name, reached through its
    /// forward reference as MSVC emits a member of structure type.
    Named(String),
    /// A structure or union defined in place (an `<unnamed-tag>`).
    Nested(Box<FixtureUdt>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UdtKind {
    Struct,
    Union,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureField {
    pub name: String,
    pub offset: u64,
    pub ty: FixtureType,
}

/// A structure or union. Its field list is written in records of at most
/// three members chained by `LF_INDEX`, so every layout read follows a
/// chain, as a large real type's does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureUdt {
    pub name: String,
    pub kind: UdtKind,
    pub size: u64,
    pub fields: Vec<FixtureField>,
}

/// Everything a fixture PDB holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdbFixture {
    /// The GUID as an RSDS record stores it.
    pub guid: [u8; 16],
    /// The DBI age: the one an image's RSDS record carries.
    pub age: u32,
    /// The PDB info stream's age; link.exe can leave it above the DBI age.
    pub pdbi_age: u32,
    pub sections: Vec<FixtureSection>,
    pub publics: Vec<FixturePublic>,
    pub procedures: Vec<FixtureProcedure>,
    pub types: Vec<FixtureUdt>,
    pub form: RecordForm,
    /// `LF_STRUCTURE` records cut short, written after the types: records a
    /// reader cannot read, among which a type it misses may be.
    pub broken_udt_records: u32,
    pub omap: Option<FixtureOmap>,
    pub container: FixtureContainer,
    /// Source files embedded in the PDB: the `/src/headerblock` stream and
    /// one `/src/files/<lower-cased path>` stream each.
    pub embedded: Vec<FixtureEmbedded>,
    /// Cuts the `/src/headerblock` stream to this many bytes.
    pub header_block_len: Option<usize>,
}

/// A source file embedded in the PDB, as `/src/headerblock` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureEmbedded {
    pub path: String,
    pub contents: Vec<u8>,
    /// The entry's compression byte; 0 is none.
    pub compression: u8,
}

/// `/src/headerblock`'s version, in its header and in each entry.
pub const SOURCE_BLOCK_VERSION: u32 = 19_980_827;
const HEADER_BLOCK: u32 = 13;
const FIRST_EMBEDDED_FILE: u32 = 14;

impl PdbFixture {
    /// A PDB of `guid` and `age` with a `.text` at RVA 0x1000 and a `.data`
    /// at RVA 0x3000, each 0x2000 bytes, and nothing in them yet.
    pub fn new(guid: [u8; 16], age: u32) -> Self {
        PdbFixture {
            guid,
            age,
            pdbi_age: age,
            sections: vec![
                FixtureSection { name: ".text", rva: 0x1000, size: 0x2000, characteristics: CODE_SECTION },
                FixtureSection { name: ".data", rva: 0x3000, size: 0x2000, characteristics: DATA_SECTION },
            ],
            publics: Vec::new(),
            procedures: Vec::new(),
            types: Vec::new(),
            form: RecordForm::Classic,
            broken_udt_records: 0,
            omap: None,
            container: FixtureContainer::Msf,
            embedded: Vec::new(),
            header_block_len: None,
        }
    }
}

/// An embedded file's names in `/names`.
struct EmbeddedNames {
    file: u32,
    virtual_file: u32,
}

/// `/src/headerblock` (LLVM `SrcHeaderBlockHeader`, then the
/// `HashTable<SrcHeaderBlockEntry>`): a 64-byte header; size, capacity, the
/// present and deleted bit vectors; a `FileNI` key and a 40-byte entry per
/// file.
fn header_block(fixture: &PdbFixture, names: &[EmbeddedNames]) -> Vec<u8> {
    let count = fixture.embedded.len() as u32;
    let capacity = count.max(1);
    let mut out = Vec::new();
    out.extend_from_slice(&SOURCE_BLOCK_VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // the stream's size, patched below
    out.extend_from_slice(&0u64.to_le_bytes()); // file time
    out.extend_from_slice(&1u32.to_le_bytes()); // age
    out.extend_from_slice(&[0; 44]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&capacity.to_le_bytes());
    let words = capacity.div_ceil(32);
    out.extend_from_slice(&words.to_le_bytes());
    for word in 0..words {
        let mut bits = 0u32;
        for slot in word * 32..(word * 32 + 32).min(count) {
            bits |= 1 << (slot % 32);
        }
        out.extend_from_slice(&bits.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // no deleted buckets
    for (embedded, names) in fixture.embedded.iter().zip(names) {
        out.extend_from_slice(&names.file.to_le_bytes()); // the key
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&SOURCE_BLOCK_VERSION.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // CRC
        out.extend_from_slice(&(embedded.contents.len() as u32).to_le_bytes());
        out.extend_from_slice(&names.file.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // object name: the empty string
        out.extend_from_slice(&names.virtual_file.to_le_bytes());
        out.push(embedded.compression);
        out.push(0); // not virtual
        out.extend_from_slice(&[0; 2 + 8]);
    }
    let size = out.len() as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    if let Some(len) = fixture.header_block_len {
        out.truncate(len);
    }
    out
}

/// One stream of a fixture PDB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureStream {
    pub index: u32,
    pub bytes: Vec<u8>,
}

/// An in-memory random-access file, so the MSF writer (`ReadAt + WriteAt`)
/// builds a PDB without touching the disk.
#[derive(Default)]
struct MemFile {
    data: Mutex<Vec<u8>>,
}

impl ReadAt for MemFile {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        self.data.lock().expect("fixture file").as_slice().read_at(buf, offset)
    }

    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.data.lock().expect("fixture file").as_slice().read_exact_at(buf, offset)
    }
}

impl WriteAt for MemFile {
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<usize> {
        self.write_all_at(buf, offset)?;
        Ok(buf.len())
    }

    fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        let mut data = self.data.lock().expect("fixture file");
        let start = offset as usize;
        let end = start + buf.len();
        if end > data.len() {
            data.resize(end, 0);
        }
        data[start..end].copy_from_slice(buf);
        Ok(())
    }
}

fn pad_zero(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn strz(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
    out.push(0);
}

/// A type stream (TPI or IPI) of `records`, whose indices run from
/// `TypeIndex::MIN_BEGIN` to `end`.
fn type_stream(records: &[u8], end: u32) -> Vec<u8> {
    let mut header = TypeStreamHeader::empty();
    header.version = U32::new(TYPE_STREAM_VERSION_2004);
    header.type_index_begin = TypeIndexLe::from(TypeIndex::MIN_BEGIN);
    header.type_index_end = TypeIndexLe::from(TypeIndex(end));
    header.type_record_bytes = U32::new(records.len() as u32);
    header.hash_key_size = U32::new(4);
    let mut out = header.as_bytes().to_vec();
    out.extend_from_slice(records);
    out
}

/// The `/names` stream, and where each of its strings starts in it.
struct NamesStream {
    bytes: Vec<u8>,
    offsets: Vec<u32>,
}

fn names_stream(strings: &[&str]) -> NamesStream {
    // The table conventionally starts with the empty string.
    let mut data = vec![0u8];
    let mut offsets = Vec::with_capacity(strings.len());
    for text in strings {
        offsets.push(data.len() as u32);
        strz(&mut data, text);
    }
    pad_zero(&mut data);
    let mut out = Vec::new();
    out.extend_from_slice(&0xEFFE_EFFEu32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    // The hash table is not read by the reader; its count is.
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    NamesStream { bytes: out, offsets }
}

/// An `IMAGE_SECTION_HEADER` per section.
fn section_headers(sections: &[FixtureSection]) -> Vec<u8> {
    let mut out = Vec::with_capacity(sections.len() * 40);
    for section in sections {
        let mut name = [0u8; 8];
        let raw = section.name.as_bytes();
        name[..raw.len().min(8)].copy_from_slice(&raw[..raw.len().min(8)]);
        out.extend_from_slice(&name);
        out.extend_from_slice(&section.size.to_le_bytes());
        out.extend_from_slice(&section.rva.to_le_bytes());
        out.extend_from_slice(&section.size.to_le_bytes());
        out.extend_from_slice(&0x400u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&section.characteristics.to_le_bytes());
    }
    out
}

fn global_symbols(fixture: &PdbFixture) -> Vec<u8> {
    let mut builder = SymBuilder::new();
    for public in &fixture.publics {
        // cvpsfFunction is bit 1 of a public's flags.
        let flags = match public.kind {
            PublicKind::Function => 2,
            PublicKind::Data => 0,
        };
        builder.pub32(flags, public.offset, public.section, &public.name);
    }
    builder.finish()
}

struct ModuleStream {
    bytes: Vec<u8>,
    sym_byte_size: u32,
    c13_byte_size: u32,
}

/// The one module's stream: an `S_GPROC32` + `S_END` per procedure, then
/// the C13 file checksums and one LINES subsection per procedure.
fn module_stream(fixture: &PdbFixture, file_names: &[u32]) -> ModuleStream {
    let mut builder = SymBuilder::new();
    {
        let mut record = builder.record(SymKind::S_OBJNAME);
        record.enc.u32(0);
        record.enc.strz(BStr::new("fixture.obj"));
    }
    for procedure in &fixture.procedures {
        let start = builder.buffer.len();
        let fixed = ProcFixed {
            p_parent: U32::new(0),
            p_end: U32::new(0),
            p_next: U32::new(0),
            proc_len: U32::new(procedure.len),
            debug_start: U32::new(0),
            debug_end: U32::new(procedure.len),
            proc_type: TypeIndexLe::from(TypeIndex(0)),
            offset_segment: OffsetSegment::new(procedure.offset, procedure.section),
            flags: 0,
        };
        {
            let mut record = builder.record(SymKind::S_GPROC32);
            record.enc.t(&fixed);
            record.enc.strz(BStr::new(&procedure.name));
        }
        // Offsets in a module stream count its 4-byte CodeView signature;
        // `p_end` (after the record's length and kind, and `p_parent`)
        // points at the `S_END` that closes the procedure.
        let end_offset = (builder.buffer.len() + 4) as u32;
        drop(builder.record(SymKind::S_END));
        let p_end_at = start + 8;
        builder.buffer[p_end_at..p_end_at + 4].copy_from_slice(&end_offset.to_le_bytes());
    }
    let symbols = builder.finish();

    // One checksum entry per procedure's file: name, size, kind, the bytes,
    // padded to 4. A LINES block names its file by its entry's offset.
    let mut checksums = Vec::new();
    let mut entry_offsets = Vec::with_capacity(file_names.len());
    for (name, procedure) in file_names.iter().zip(&fixture.procedures) {
        entry_offsets.push(checksums.len() as u32);
        checksums.extend_from_slice(&name.to_le_bytes());
        match &procedure.checksum {
            Some(checksum) => {
                checksums.push(checksum.bytes.len() as u8);
                checksums.push(checksum.kind);
                checksums.extend_from_slice(&checksum.bytes);
            }
            None => {
                checksums.push(0);
                checksums.push(0);
            }
        }
        pad_zero(&mut checksums);
    }
    let mut c13 = Vec::new();
    subsection(&mut c13, SubsectionKind::FILE_CHECKSUMS, &checksums);
    for (index, procedure) in fixture.procedures.iter().enumerate() {
        let file_index = entry_offsets[index];
        let mut payload = Vec::new();
        payload.extend_from_slice(&procedure.offset.to_le_bytes());
        payload.extend_from_slice(&procedure.section.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&procedure.len.to_le_bytes());
        payload.extend_from_slice(&file_index.to_le_bytes());
        payload.extend_from_slice(&(procedure.lines.len() as u32).to_le_bytes());
        payload.extend_from_slice(&((12 + 8 * procedure.lines.len()) as u32).to_le_bytes());
        for line in &procedure.lines {
            payload.extend_from_slice(&line.offset.to_le_bytes());
            // The high bit marks a statement.
            payload.extend_from_slice(&(line.line | 0x8000_0000).to_le_bytes());
        }
        subsection(&mut c13, SubsectionKind::LINES, &payload);
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&CV_SIGNATURE_C13.to_le_bytes());
    bytes.extend_from_slice(&symbols);
    let sym_byte_size = bytes.len() as u32;
    bytes.extend_from_slice(&c13);
    // The global-references size: none.
    bytes.extend_from_slice(&0u32.to_le_bytes());
    ModuleStream { bytes, sym_byte_size, c13_byte_size: c13.len() as u32 }
}

fn subsection(out: &mut Vec<u8>, kind: SubsectionKind, payload: &[u8]) {
    out.extend_from_slice(&kind.0.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    pad_zero(out);
}

fn dbi_stream(fixture: &PdbFixture, module: &ModuleStream, files: &[String]) -> Vec<u8> {
    let contribution = SectionContribEntry {
        section: U16::new(1),
        padding1: [0; 2],
        offset: I32::new(0),
        size: I32::new(0x100),
        characteristics: U32::new(CODE_SECTION),
        module_index: U16::new(0),
        padding2: [0; 2],
        data_crc: U32::new(0),
        reloc_crc: U32::new(0),
    };
    let info = ModuleInfoFixed {
        unused1: U32::new(0),
        section_contrib: contribution.clone(),
        flags: U16::new(0),
        stream: StreamIndexU16(U16::new(MODULE as u16)),
        sym_byte_size: U32::new(module.sym_byte_size),
        c11_byte_size: U32::new(0),
        c13_byte_size: U32::new(module.c13_byte_size),
        source_file_count: U16::new(files.len() as u16),
        padding: [0; 2],
        unused2: U32::new(0),
        source_file_name_index: U32::new(0),
        pdb_file_path_name_index: U32::new(0),
    };
    let mut modules = info.as_bytes().to_vec();
    strz(&mut modules, "fixture.obj");
    strz(&mut modules, "fixture.obj");
    pad_zero(&mut modules);

    let mut contributions = SECTION_CONTRIBUTIONS_SUBSTREAM_VER60.to_le_bytes().to_vec();
    contributions.extend_from_slice(contribution.as_bytes());

    let count = fixture.sections.len() as u16;
    let mut section_map = SectionMapHeader { num_segments: U16::new(count), num_logical_segments: U16::new(count) }
        .as_bytes()
        .to_vec();
    for (index, section) in fixture.sections.iter().enumerate() {
        let entry = SectionMapEntry {
            // READ | ADDRESS_IS32_BIT | IS_SELECTOR
            flags: U16::new(0x0109),
            overlay: U16::new(0),
            group: U16::new(0),
            frame: U16::new((index + 1) as u16),
            section_name: U16::new(0xFFFF),
            class_name: U16::new(0xFFFF),
            offset: U32::new(0),
            section_length: U32::new(section.size),
        };
        section_map.extend_from_slice(entry.as_bytes());
    }

    let mut sources = Vec::new();
    sources.extend_from_slice(&1u16.to_le_bytes());
    sources.extend_from_slice(&(files.len() as u16).to_le_bytes());
    sources.extend_from_slice(&0u16.to_le_bytes());
    sources.extend_from_slice(&(files.len() as u16).to_le_bytes());
    let mut names = Vec::new();
    for file in files {
        sources.extend_from_slice(&(names.len() as u32).to_le_bytes());
        strz(&mut names, file);
    }
    sources.extend_from_slice(&names);
    pad_zero(&mut sources);

    // The optional debug header: 11 stream indices, 0xFFFF for none.
    let mut optional = [0xFFFFu16; 11];
    optional[5] = SECTION_HEADERS as u16;
    if fixture.omap.is_some() {
        optional[4] = OMAP_FROM_SOURCE as u16;
        optional[10] = ORIGINAL_SECTION_HEADERS as u16;
    }
    let mut optional_bytes = Vec::with_capacity(22);
    for index in optional {
        optional_bytes.extend_from_slice(&index.to_le_bytes());
    }

    let header = DbiStreamHeader {
        signature: I32::new(-1),
        version: U32::new(DBI_STREAM_VERSION_V70),
        age: U32::new(fixture.age),
        global_symbol_index_stream: StreamIndexU16(U16::new(GSI as u16)),
        build_number: U16::new(0x8E1D),
        public_symbol_index_stream: StreamIndexU16(U16::new(PSI as u16)),
        pdb_dll_version: U16::new(0),
        global_symbol_stream: StreamIndexU16(U16::new(GSS as u16)),
        pdb_dll_rbld: U16::new(0),
        mod_info_size: I32::new(modules.len() as i32),
        section_contribution_size: I32::new(contributions.len() as i32),
        section_map_size: I32::new(section_map.len() as i32),
        source_info_size: I32::new(sources.len() as i32),
        type_server_map_size: I32::new(0),
        mfc_type_server_index: U32::new(0),
        optional_dbg_header_size: I32::new(optional_bytes.len() as i32),
        edit_and_continue_size: I32::new(0),
        flags: U16::new(0),
        machine: U16::new(0x8664),
        padding: U32::new(0),
    };
    let mut out = header.as_bytes().to_vec();
    out.extend_from_slice(&modules);
    out.extend_from_slice(&contributions);
    out.extend_from_slice(&section_map);
    out.extend_from_slice(&sources);
    out.extend_from_slice(&optional_bytes);
    out
}

const T_UCHAR: u32 = 0x0020;
const T_USHORT: u32 = 0x0021;
const T_ULONG: u32 = 0x0022;
const T_UQUAD: u32 = 0x0023;
/// A 64-bit pointer to `void`: mode 6 (64-bit near) over `T_VOID`.
const T_64PVOID: u32 = 0x0603;
/// `LF_MEMBER` access: public.
const ATTR_PUBLIC: u16 = 3;
/// UDT properties: a forward reference; a decorated unique name follows the
/// display name.
const PROP_FWDREF: u32 = 1 << 7;
const PROP_HASUNIQUENAME: u32 = 1 << 9;
/// Members per `LF_FIELDLIST` record before an `LF_INDEX` continues it.
const MEMBERS_PER_RECORD: usize = 3;
/// `LF_STRUCTURE2` and `LF_UNION2` (ms-codeview `types/kind.rs`).
const LF_STRUCTURE2: u16 = 0x1609;
const LF_UNION2: u16 = 0x160A;

/// A CodeView numeric leaf: the value itself below 0x8000, else a leaf
/// naming its width, then the value.
fn numeric(out: &mut Vec<u8>, value: u64) {
    if value < 0x8000 {
        out.extend_from_slice(&(value as u16).to_le_bytes());
    } else if let Ok(value) = u16::try_from(value) {
        out.extend_from_slice(&Leaf::LF_USHORT.0.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    } else if let Ok(value) = u32::try_from(value) {
        out.extend_from_slice(&Leaf::LF_ULONG.0.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    } else {
        out.extend_from_slice(&Leaf::LF_UQUADWORD.0.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// CodeView padding to 4 bytes from `base`: `LF_PAD3 LF_PAD2 LF_PAD1`,
/// each byte at or above 0xF0, so a field-list reader steps over it.
fn pad_cv(buf: &mut Vec<u8>, base: usize) {
    let rem = (buf.len() - base) % 4;
    if rem != 0 {
        for pad in (1..=4 - rem).rev() {
            buf.push(0xF0 + pad as u8);
        }
    }
}

/// Type records, numbered in the order they are written.
struct TypeWriter {
    bytes: Vec<u8>,
    next: u32,
}

impl TypeWriter {
    /// Appends one record (`u16` length, `u16` leaf, payload, padding) and
    /// returns its index.
    fn record(&mut self, leaf: u16, payload: &[u8]) -> TypeIndex {
        let index = TypeIndex(self.next);
        self.next += 1;
        let start = self.bytes.len();
        self.bytes.extend_from_slice(&[0, 0]);
        self.bytes.extend_from_slice(&leaf.to_le_bytes());
        self.bytes.extend_from_slice(payload);
        pad_cv(&mut self.bytes, start);
        let len = (self.bytes.len() - start - 2) as u16;
        self.bytes[start..start + 2].copy_from_slice(&len.to_le_bytes());
        index
    }
}

/// A UDT's decorated name, as MSVC writes a struct's (`.?AU<name>@@`).
fn unique_name(name: &str) -> String {
    format!(".?AU{name}@@")
}

/// The fields of a UDT record, in either form.
struct UdtRecord<'a> {
    kind: UdtKind,
    form: RecordForm,
    property: u32,
    count: usize,
    field_list: TypeIndex,
    size: u64,
    name: &'a str,
    unique: Option<&'a str>,
}

impl UdtRecord<'_> {
    fn leaf(&self) -> u16 {
        match (self.form, self.kind) {
            (RecordForm::Classic, UdtKind::Struct) => Leaf::LF_STRUCTURE.0,
            (RecordForm::Classic, UdtKind::Union) => Leaf::LF_UNION.0,
            (RecordForm::Wide, UdtKind::Struct) => LF_STRUCTURE2,
            (RecordForm::Wide, UdtKind::Union) => LF_UNION2,
        }
    }

    fn payload(&self) -> Vec<u8> {
        let property = match self.unique {
            Some(_) => self.property | PROP_HASUNIQUENAME,
            None => self.property,
        };
        let mut out = Vec::new();
        match self.form {
            RecordForm::Classic => {
                out.extend_from_slice(&(self.count as u16).to_le_bytes());
                out.extend_from_slice(&(property as u16).to_le_bytes());
                out.extend_from_slice(&self.field_list.0.to_le_bytes());
                match self.kind {
                    // A structure's derivation list and vtable shape: none.
                    UdtKind::Struct => out.extend_from_slice(&[0u8; 8]),
                    UdtKind::Union => {}
                }
            }
            RecordForm::Wide => {
                out.extend_from_slice(&property.to_le_bytes());
                out.extend_from_slice(&self.field_list.0.to_le_bytes());
                // Derivation list and vtable shape, for unions too.
                out.extend_from_slice(&[0u8; 8]);
                out.extend_from_slice(&(self.count as u16).to_le_bytes());
            }
        }
        numeric(&mut out, self.size);
        strz(&mut out, self.name);
        if let Some(unique) = self.unique {
            strz(&mut out, unique);
        }
        out
    }
}

/// What the type writer needs to resolve a member's type.
struct TypeScope<'a> {
    types: &'a [FixtureUdt],
    form: RecordForm,
    /// Each named UDT's forward reference.
    forwards: BTreeMap<String, TypeIndex>,
}

impl TypeScope<'_> {
    fn size(&self, ty: &FixtureType) -> u64 {
        match ty {
            FixtureType::U8 => 1,
            FixtureType::U16 => 2,
            FixtureType::U32 => 4,
            FixtureType::U64 | FixtureType::Pointer => 8,
            FixtureType::Array { element, count } => self.size(element) * count,
            FixtureType::Bitfield { storage, .. } => self.size(storage),
            FixtureType::Named(name) => {
                self.types.iter().find(|udt| &udt.name == name).expect("a named type the fixture defines").size
            }
            FixtureType::Nested(udt) => udt.size,
        }
    }

    fn index(&self, writer: &mut TypeWriter, ty: &FixtureType) -> TypeIndex {
        match ty {
            FixtureType::U8 => TypeIndex(T_UCHAR),
            FixtureType::U16 => TypeIndex(T_USHORT),
            FixtureType::U32 => TypeIndex(T_ULONG),
            FixtureType::U64 => TypeIndex(T_UQUAD),
            FixtureType::Pointer => TypeIndex(T_64PVOID),
            FixtureType::Array { element, count } => {
                let element_index = self.index(writer, element);
                let mut payload = Vec::new();
                payload.extend_from_slice(&element_index.0.to_le_bytes());
                payload.extend_from_slice(&T_ULONG.to_le_bytes());
                numeric(&mut payload, self.size(element) * count);
                strz(&mut payload, "");
                writer.record(Leaf::LF_ARRAY.0, &payload)
            }
            FixtureType::Bitfield { storage, position, length } => {
                let storage_index = self.index(writer, storage);
                let mut payload = storage_index.0.to_le_bytes().to_vec();
                payload.push(*length);
                payload.push(*position);
                writer.record(Leaf::LF_BITFIELD.0, &payload)
            }
            FixtureType::Named(name) => *self.forwards.get(name).expect("a named type the fixture defines"),
            FixtureType::Nested(udt) => self.define(writer, udt, None),
        }
    }

    /// Writes `udt`'s member types, its field list (chunks chained by
    /// `LF_INDEX`, the last written first so each can name the next), and
    /// its complete definition.
    fn define(&self, writer: &mut TypeWriter, udt: &FixtureUdt, unique: Option<&str>) -> TypeIndex {
        let mut members: Vec<Vec<u8>> = Vec::with_capacity(udt.fields.len());
        for field in &udt.fields {
            let ty = self.index(writer, &field.ty);
            let mut member = Vec::new();
            member.extend_from_slice(&Leaf::LF_MEMBER.0.to_le_bytes());
            member.extend_from_slice(&ATTR_PUBLIC.to_le_bytes());
            member.extend_from_slice(&ty.0.to_le_bytes());
            numeric(&mut member, field.offset);
            strz(&mut member, &field.name);
            pad_cv(&mut member, 0);
            members.push(member);
        }
        let mut next: Option<TypeIndex> = None;
        let chunks: Vec<&[Vec<u8>]> = members.chunks(MEMBERS_PER_RECORD).collect();
        for chunk in chunks.iter().rev() {
            let mut list: Vec<u8> = chunk.concat();
            if let Some(continued) = next {
                list.extend_from_slice(&Leaf::LF_INDEX.0.to_le_bytes());
                list.extend_from_slice(&0u16.to_le_bytes());
                list.extend_from_slice(&continued.0.to_le_bytes());
            }
            next = Some(writer.record(Leaf::LF_FIELDLIST.0, &list));
        }
        let field_list = match next {
            Some(first) => first,
            None => writer.record(Leaf::LF_FIELDLIST.0, &[]),
        };
        let name = if udt.name.is_empty() { "<unnamed-tag>" } else { &udt.name };
        let record = UdtRecord {
            kind: udt.kind,
            form: self.form,
            property: 0,
            count: udt.fields.len(),
            field_list,
            size: udt.size,
            name,
            unique,
        };
        writer.record(record.leaf(), &record.payload())
    }
}

/// The TPI stream's records and the first index past them.
struct TypeRecords {
    bytes: Vec<u8>,
    end: u32,
}

/// Every fixture UDT as MSVC writes one: a forward reference first (which
/// members of that type point at), then its complete definition; then the
/// broken records the fixture asks for.
fn type_records(fixture: &PdbFixture) -> TypeRecords {
    let mut writer = TypeWriter { bytes: Vec::new(), next: TypeIndex::MIN_BEGIN.0 };
    let mut scope = TypeScope { types: &fixture.types, form: fixture.form, forwards: BTreeMap::new() };
    for udt in &fixture.types {
        let unique = unique_name(&udt.name);
        let forward = UdtRecord {
            kind: udt.kind,
            form: fixture.form,
            property: PROP_FWDREF,
            count: 0,
            field_list: TypeIndex(0),
            size: 0,
            name: &udt.name,
            unique: Some(&unique),
        };
        let index = writer.record(forward.leaf(), &forward.payload());
        scope.forwards.insert(udt.name.clone(), index);
    }
    for udt in &fixture.types {
        let unique = unique_name(&udt.name);
        scope.define(&mut writer, udt, Some(&unique));
    }
    for _ in 0..fixture.broken_udt_records {
        // A member count and half a property: the record ends before its
        // field-list index.
        writer.record(Leaf::LF_STRUCTURE.0, &[1, 0, 0]);
    }
    TypeRecords { bytes: writer.bytes, end: writer.next }
}

/// The fixture's streams, by stream index.
pub fn fixture_streams(fixture: &PdbFixture) -> Vec<FixtureStream> {
    let files: Vec<String> = fixture.procedures.iter().map(|procedure| procedure.file.clone()).collect();
    // Procedures' files first (a module's checksum entries name them by
    // these offsets), then embedded files' paths and virtual names not
    // among them.
    let virtual_names: Vec<String> = fixture.embedded.iter().map(|embedded| embedded.path.to_lowercase()).collect();
    let mut strings: Vec<&str> = files.iter().map(String::as_str).collect();
    for text in fixture.embedded.iter().map(|embedded| embedded.path.as_str()).chain(virtual_names.iter().map(String::as_str)) {
        if !strings.contains(&text) {
            strings.push(text);
        }
    }
    let names = names_stream(&strings);
    let offset_of = |text: &str| {
        let place = strings.iter().position(|known| *known == text).expect("a string of /names");
        names.offsets[place]
    };
    let embedded_names: Vec<EmbeddedNames> = fixture
        .embedded
        .iter()
        .zip(&virtual_names)
        .map(|(embedded, virtual_name)| EmbeddedNames { file: offset_of(&embedded.path), virtual_file: offset_of(virtual_name) })
        .collect();
    let gss = global_symbols(fixture);
    let indexes = build_global_symbols_index(&gss, get_v1_default_bucket(false)).expect("symbol indexes");
    let module = module_stream(fixture, &names.offsets);

    let mut named = NamedStreams::default();
    assert!(named.insert("/names", NAMES), "the /names stream is named once");
    let mut embedded_streams = Vec::new();
    if !fixture.embedded.is_empty() {
        assert!(named.insert("/src/headerblock", HEADER_BLOCK), "the header block is named once");
        embedded_streams.push(FixtureStream { index: HEADER_BLOCK, bytes: header_block(fixture, &embedded_names) });
        for (place, (embedded, virtual_name)) in fixture.embedded.iter().zip(&virtual_names).enumerate() {
            let index = FIRST_EMBEDDED_FILE + place as u32;
            assert!(named.insert(&format!("/src/files/{virtual_name}"), index), "each embedded file is named once");
            embedded_streams.push(FixtureStream { index, bytes: embedded.contents.clone() });
        }
    }
    let pdbi = PdbiStream {
        signature: 0x6700_0000,
        version: PDBI_VERSION_VC70,
        age: fixture.pdbi_age,
        unique_id: Some(Uuid::from_bytes_le(fixture.guid)),
        named_streams: named,
        features: vec![FeatureCode(PDBI_VERSION_VC140)],
    };
    let types = type_records(fixture);
    let mut streams = vec![
        FixtureStream { index: 1, bytes: pdbi.to_bytes().expect("PDBI stream") },
        FixtureStream { index: 2, bytes: type_stream(&types.bytes, types.end) },
        FixtureStream { index: 3, bytes: dbi_stream(fixture, &module, &files) },
        FixtureStream { index: 4, bytes: type_stream(&[], TypeIndex::MIN_BEGIN.0) },
        FixtureStream { index: NAMES, bytes: names.bytes },
        FixtureStream { index: GSS, bytes: gss },
        FixtureStream { index: GSI, bytes: indexes.global_symbol_index_stream_data },
        FixtureStream { index: PSI, bytes: indexes.public_symbol_index_stream_data },
        FixtureStream { index: MODULE, bytes: module.bytes },
        FixtureStream { index: SECTION_HEADERS, bytes: section_headers(&fixture.sections) },
    ];
    if let Some(omap) = &fixture.omap {
        streams.push(FixtureStream { index: ORIGINAL_SECTION_HEADERS, bytes: section_headers(&omap.original_sections) });
        let mut map = Vec::with_capacity(omap.from_source.len() * 8);
        for entry in &omap.from_source {
            map.extend_from_slice(&entry.from.to_le_bytes());
            map.extend_from_slice(&entry.to.to_le_bytes());
        }
        streams.push(FixtureStream { index: OMAP_FROM_SOURCE, bytes: map });
    }
    streams.extend(embedded_streams);
    streams
}

/// The fixture as a PDB file.
pub fn build_pdb(fixture: &PdbFixture) -> Vec<u8> {
    let streams = fixture_streams(fixture);
    match fixture.container {
        FixtureContainer::Msf => {
            let mut msf = Msf::create_with_file(MemFile::default(), CreateOptions::default()).expect("MSF");
            for stream in &streams {
                let mut writer = msf.write_stream(stream.index).expect("MSF stream");
                writer.set_contents(&stream.bytes).expect("MSF stream contents");
            }
            assert!(msf.commit().expect("MSF commit"), "the MSF commit wrote nothing");
            msf.into_file().data.into_inner().expect("fixture file")
        }
        FixtureContainer::MsfzZstd
        | FixtureContainer::MsfzDeflate
        | FixtureContainer::MsfzSmallChunks
        | FixtureContainer::MsfzStored => {
            let mut writer = MsfzWriter::new(Cursor::new(Vec::new())).expect("MSFZ");
            match fixture.container {
                FixtureContainer::MsfzDeflate => writer.set_chunk_compression_mode(Compression::Deflate),
                FixtureContainer::MsfzSmallChunks => writer.set_uncompressed_chunk_size_threshold(4096),
                FixtureContainer::Msf | FixtureContainer::MsfzZstd | FixtureContainer::MsfzStored => {}
            }
            let last = streams.iter().map(|stream| stream.index).max().unwrap_or(4);
            writer.reserve_num_streams(last as usize + 1);
            for stream in &streams {
                let mut stream_writer = writer.stream_writer(stream.index).expect("MSFZ stream");
                if fixture.container == FixtureContainer::MsfzStored {
                    stream_writer.set_compression_enabled(false);
                }
                stream_writer.write_all(&stream.bytes).expect("MSFZ stream contents");
            }
            let finished = writer.finish().expect("MSFZ finish");
            finished.1.into_inner()
        }
    }
}
