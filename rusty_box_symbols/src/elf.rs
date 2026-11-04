//! ELF files and loaded ELF images, little-endian, 32- and 64-bit: the
//! headers, the section table and its names, symbol tables, notes (the GNU
//! build-id), `.gnu_debugaltlink`, and sections compressed per the gABI
//! (`SHF_COMPRESSED`) or GNU's older `.zdebug_*` convention. Offsets follow
//! the System V ABI and its ELF-64 supplement.
//!
//! Tier P: the `decode_*` functions and [`find_note`] turn one record into
//! numbers and borrowed bytes. Tier A: [`ElfFile`] and
//! [`identity_from_image`] collect them into tables sized from capped
//! counts.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::decompress::{decompress_to_vec, Codec, DecompressError};
use crate::identity::{BuildId, ModuleIdentity};
use crate::pe::ImageRead;
use crate::read::{Bytes, ReadError};
use crate::units::Rva;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const SHT_SYMTAB: u32 = 2;
const SHT_NOTE: u32 = 7;
const SHT_NOBITS: u32 = 8;
const SHT_DYNSYM: u32 = 11;
/// gABI "Compressed Sections".
const SHF_COMPRESSED: u64 = 0x800;
const ELFCOMPRESS_ZLIB: u32 = 1;
const ELFCOMPRESS_ZSTD: u32 = 2;
const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const NT_GNU_BUILD_ID: u32 = 3;
const GNU_NOTE_NAME: &[u8] = b"GNU\0";
const STT_OBJECT: u8 = 1;
const STT_FUNC: u8 = 2;
const SHN_UNDEF: u16 = 0;
/// gABI "Sections": an `e_shstrndx` of `SHN_XINDEX` lives in section 0's
/// `sh_link`; an `e_shnum` of 0 with a section table, in its `sh_size`.
const SHN_XINDEX: u16 = 0xFFFF;
/// The most sections read: a large C++ object nears 2^16 with
/// `-ffunction-sections`; past this a table is not a real file's.
const MAX_SECTIONS: u64 = 1 << 20;
/// The most program headers read; real executables have a dozen.
const MAX_PROGRAM_HEADERS: u16 = 4096;
/// The most symbols read: a browser's `.symtab` holds about a million.
const MAX_SYMBOLS: u64 = 1 << 23;
/// The longest section or symbol name read; longer ones are cut there.
const MAX_NAME_BYTES: usize = 4096;
/// GNU build-ids are 20 bytes (SHA-1) or 16 (MD5); 64 is generous.
const MAX_BUILD_ID_BYTES: usize = 64;
/// The largest section decompressed; DWARF of the largest binaries stays
/// below this.
const MAX_SECTION_BYTES: u64 = 1 << 31;
/// The most of a loaded image's note segment read.
const MAX_NOTE_BYTES: usize = 4096;
/// Program and ELF headers lie in a loaded image's first page.
const ELF_HEADER_READ: usize = 64;

/// The file's class: the width of its addresses and offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfClass {
    Elf32,
    Elf64,
}

/// Record sizes of a class.
#[derive(Clone, Copy)]
struct RecordSizes {
    section: usize,
    program: usize,
    symbol: usize,
    compression_header: usize,
}

impl ElfClass {
    #[cfg_attr(all(test, not(debug_assertions)), no_panic)]
    fn sizes(self) -> RecordSizes {
        match self {
            ElfClass::Elf32 => RecordSizes { section: 40, program: 32, symbol: 16, compression_header: 12 },
            ElfClass::Elf64 => RecordSizes { section: 64, program: 56, symbol: 24, compression_header: 24 },
        }
    }
}

/// The fields of the ELF header the reader uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElfHeader {
    pub class: ElfClass,
    pub kind: u16,
    pub machine: u16,
    pub program_offset: u64,
    pub section_offset: u64,
    pub program_entry_size: u16,
    pub program_count: u16,
    pub section_entry_size: u16,
    pub section_count: u16,
    pub names_index: u16,
}

/// One section header, its name still an offset into `.shstrtab`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawSection {
    pub name: u32,
    pub kind: u32,
    pub flags: u64,
    pub address: u64,
    pub offset: u64,
    pub size: u64,
    pub link: u32,
    pub align: u64,
    pub entry_size: u64,
}

/// One program header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramHeader {
    pub kind: u32,
    pub offset: u64,
    pub address: u64,
    pub file_size: u64,
    pub align: u64,
}

/// One symbol-table entry, its name still an offset into its string table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawSymbol {
    pub name: u32,
    pub info: u8,
    pub section: u16,
    pub value: u64,
    pub size: u64,
}

/// A section with its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElfSection {
    pub name: String,
    pub kind: u32,
    pub flags: u64,
    pub address: u64,
    pub offset: u64,
    pub size: u64,
    pub link: u32,
    pub align: u64,
    pub entry_size: u64,
}

/// A section's contents: borrowed from the file, or decompressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionBytes<'a> {
    Borrowed(&'a [u8]),
    Decompressed(Vec<u8>),
}

impl SectionBytes<'_> {
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            SectionBytes::Borrowed(bytes) => bytes,
            SectionBytes::Decompressed(bytes) => bytes,
        }
    }
}

/// What a symbol names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Object,
}

/// A defined function or data symbol at its link-time address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElfSymbol {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub kind: SymbolKind,
}

/// `.gnu_debugaltlink`: the dwz supplement a debug file shares strings and
/// types with, named by file and by build-id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AltLink {
    pub file_name: String,
    pub build_id: BuildId,
}

/// Why an ELF file did not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ElfError {
    #[error("the bytes are not an ELF file")]
    NotElf,
    #[error("the file is big-endian, which no x86 guest uses")]
    BigEndian,
    #[error("ELF class {class} is neither 32- nor 64-bit")]
    BadClass { class: u8 },
    #[error("ELF data encoding {data} is not little-endian")]
    BadData { data: u8 },
    #[error("the {what} is cut short: {source}")]
    Short {
        what: &'static str,
        #[source]
        source: ReadError,
    },
    #[error("{what} entries of {size} bytes are smaller than the class's")]
    BadEntrySize { what: &'static str, size: u64 },
    #[error("{count} {what} is more than a real file has")]
    TooMany { what: &'static str, count: u64 },
    #[error("section {index} does not exist")]
    BadSectionIndex { index: u64 },
    #[error("compression type {kind} is not zlib or zstd")]
    UnknownCompression { kind: u32 },
    #[error("a compressed section does not decompress: {0}")]
    Decompress(#[from] DecompressError),
    #[error("no memory for {count} {what}")]
    Allocation { what: &'static str, count: u64 },
}

#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn short(what: &'static str, source: ReadError) -> ElfError {
    ElfError::Short { what, source }
}

/// `value` rounded up to `align`, a power of two.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn align_up(value: usize, align: usize) -> Option<usize> {
    let mask = align.checked_sub(1)?;
    value.checked_add(mask).map(|value| value & !mask)
}

/// The ELF header (gABI "ELF Header").
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_header(file: Bytes<'_>) -> Result<ElfHeader, ElfError> {
    match file.array::<4>(0) {
        Ok(magic) if magic == ELF_MAGIC => {}
        Ok(_) | Err(_) => return Err(ElfError::NotElf),
    }
    let what = "ELF header";
    let class = match file.u8(4).map_err(|source| short(what, source))? {
        1 => ElfClass::Elf32,
        2 => ElfClass::Elf64,
        other => return Err(ElfError::BadClass { class: other }),
    };
    match file.u8(5).map_err(|source| short(what, source))? {
        1 => {}
        2 => return Err(ElfError::BigEndian),
        other => return Err(ElfError::BadData { data: other }),
    }
    let half = |at: usize| file.u16_le(at).map_err(|source| short(what, source));
    let word = |at: usize| file.u32_le(at).map(u64::from).map_err(|source| short(what, source));
    let long = |at: usize| file.u64_le(at).map_err(|source| short(what, source));
    let kind = half(16)?;
    let machine = half(18)?;
    Ok(match class {
        ElfClass::Elf32 => ElfHeader {
            class,
            kind,
            machine,
            program_offset: word(28)?,
            section_offset: word(32)?,
            program_entry_size: half(42)?,
            program_count: half(44)?,
            section_entry_size: half(46)?,
            section_count: half(48)?,
            names_index: half(50)?,
        },
        ElfClass::Elf64 => ElfHeader {
            class,
            kind,
            machine,
            program_offset: long(32)?,
            section_offset: long(40)?,
            program_entry_size: half(54)?,
            program_count: half(56)?,
            section_entry_size: half(58)?,
            section_count: half(60)?,
            names_index: half(62)?,
        },
    })
}

/// One section header (gABI "Sections").
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_section_header(raw: Bytes<'_>, class: ElfClass) -> Result<RawSection, ReadError> {
    Ok(match class {
        ElfClass::Elf32 => RawSection {
            name: raw.u32_le(0)?,
            kind: raw.u32_le(4)?,
            flags: u64::from(raw.u32_le(8)?),
            address: u64::from(raw.u32_le(12)?),
            offset: u64::from(raw.u32_le(16)?),
            size: u64::from(raw.u32_le(20)?),
            link: raw.u32_le(24)?,
            align: u64::from(raw.u32_le(32)?),
            entry_size: u64::from(raw.u32_le(36)?),
        },
        ElfClass::Elf64 => RawSection {
            name: raw.u32_le(0)?,
            kind: raw.u32_le(4)?,
            flags: raw.u64_le(8)?,
            address: raw.u64_le(16)?,
            offset: raw.u64_le(24)?,
            size: raw.u64_le(32)?,
            link: raw.u32_le(40)?,
            align: raw.u64_le(48)?,
            entry_size: raw.u64_le(56)?,
        },
    })
}

/// One program header (gABI "Program Header").
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_program_header(raw: Bytes<'_>, class: ElfClass) -> Result<ProgramHeader, ReadError> {
    Ok(match class {
        ElfClass::Elf32 => ProgramHeader {
            kind: raw.u32_le(0)?,
            offset: u64::from(raw.u32_le(4)?),
            address: u64::from(raw.u32_le(8)?),
            file_size: u64::from(raw.u32_le(16)?),
            align: u64::from(raw.u32_le(28)?),
        },
        ElfClass::Elf64 => ProgramHeader {
            kind: raw.u32_le(0)?,
            offset: raw.u64_le(8)?,
            address: raw.u64_le(16)?,
            file_size: raw.u64_le(32)?,
            align: raw.u64_le(48)?,
        },
    })
}

/// One symbol-table entry (gABI "Symbol Table").
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_symbol(raw: Bytes<'_>, class: ElfClass) -> Result<RawSymbol, ReadError> {
    Ok(match class {
        ElfClass::Elf32 => RawSymbol {
            name: raw.u32_le(0)?,
            value: u64::from(raw.u32_le(4)?),
            size: u64::from(raw.u32_le(8)?),
            info: raw.u8(12)?,
            section: raw.u16_le(14)?,
        },
        ElfClass::Elf64 => RawSymbol {
            name: raw.u32_le(0)?,
            info: raw.u8(4)?,
            section: raw.u16_le(6)?,
            value: raw.u64_le(8)?,
            size: raw.u64_le(16)?,
        },
    })
}

/// The descriptor of the first note named `name` of type `kind` in `notes`,
/// each note padded to `align` (gABI "Note Section"). Every note is at
/// least 12 bytes, so the walk ends within `notes.len() / 12` steps.
#[must_use]
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn find_note<'a>(notes: Bytes<'a>, align: usize, name: &[u8], kind: u32) -> Option<&'a [u8]> {
    let mut at = 0usize;
    while at < notes.len() {
        let name_size = notes.u32_le(at).ok()? as usize;
        let descriptor_size = notes.u32_le(at.checked_add(4)?).ok()? as usize;
        let note_kind = notes.u32_le(at.checked_add(8)?).ok()?;
        let name_at = at.checked_add(12)?;
        let note_name = notes.slice(name_at, name_size).ok()?;
        let descriptor_at = align_up(name_at.checked_add(name_size)?, align)?;
        let descriptor = notes.slice(descriptor_at, descriptor_size).ok()?;
        if note_kind == kind && note_name == name {
            return Some(descriptor);
        }
        at = align_up(descriptor_at.checked_add(descriptor_size)?, align)?;
    }
    None
}

/// The note alignment a section or segment of alignment `align` uses: 8
/// for 8-aligned note sections (`.note.gnu.property`), else 4.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn note_alignment(align: u64) -> usize {
    if align == 8 {
        8
    } else {
        4
    }
}

/// A build-id descriptor as a [`BuildId`], when its length is plausible.
fn build_id_of(descriptor: &[u8]) -> Option<BuildId> {
    (!descriptor.is_empty() && descriptor.len() <= MAX_BUILD_ID_BYTES).then(|| BuildId(descriptor.to_vec()))
}

/// An ELF file's tables, over its bytes.
pub struct ElfFile<'a> {
    bytes: Bytes<'a>,
    class: ElfClass,
    sections: Vec<ElfSection>,
    program_headers: Vec<ProgramHeader>,
}

/// `base + index * size`, or `None` past `u64`.
fn nth(base: u64, index: u64, size: u64) -> Option<u64> {
    base.checked_add(index.checked_mul(size)?)
}

impl<'a> ElfFile<'a> {
    /// Parses the headers and both tables.
    pub fn parse(file: &'a [u8]) -> Result<ElfFile<'a>, ElfError> {
        let bytes = Bytes::new(file);
        let header = decode_header(bytes)?;
        let sizes = header.class.sizes();
        let raw_sections = Self::raw_sections(bytes, &header)?;
        let names_index = match header.names_index {
            SHN_XINDEX => u64::from(raw_sections.first().map_or(0, |first| first.link)),
            index => u64::from(index),
        };
        let names = match raw_sections.get(names_index as usize) {
            Some(table) if table.kind != SHT_NOBITS => {
                let at = usize::try_from(table.offset).map_err(|_| ElfError::BadSectionIndex { index: names_index })?;
                let len = usize::try_from(table.size).map_err(|_| ElfError::BadSectionIndex { index: names_index })?;
                bytes.sub(at, len).map_err(|source| short("section name table", source))?
            }
            Some(_) | None if raw_sections.is_empty() => Bytes::new(&[]),
            Some(_) | None => return Err(ElfError::BadSectionIndex { index: names_index }),
        };
        let mut sections = Vec::new();
        sections
            .try_reserve_exact(raw_sections.len())
            .map_err(|_| ElfError::Allocation { what: "sections", count: raw_sections.len() as u64 })?;
        for raw in &raw_sections {
            let name = names.cstr(raw.name as usize, MAX_NAME_BYTES).unwrap_or(&[]);
            sections.push(ElfSection {
                name: String::from_utf8_lossy(name).into_owned(),
                kind: raw.kind,
                flags: raw.flags,
                address: raw.address,
                offset: raw.offset,
                size: raw.size,
                link: raw.link,
                align: raw.align,
                entry_size: raw.entry_size,
            });
        }
        let program_count = header.program_count.min(MAX_PROGRAM_HEADERS);
        if program_count > 0 && usize::from(header.program_entry_size) < sizes.program {
            return Err(ElfError::BadEntrySize { what: "program header", size: u64::from(header.program_entry_size) });
        }
        let mut program_headers = Vec::new();
        program_headers
            .try_reserve_exact(usize::from(program_count))
            .map_err(|_| ElfError::Allocation { what: "program headers", count: u64::from(program_count) })?;
        for index in 0..u64::from(program_count) {
            let at = nth(header.program_offset, index, u64::from(header.program_entry_size))
                .and_then(|at| usize::try_from(at).ok())
                .ok_or(ElfError::TooMany { what: "program headers", count: u64::from(program_count) })?;
            let raw = bytes.sub(at, sizes.program).map_err(|source| short("program header", source))?;
            program_headers.push(decode_program_header(raw, header.class).map_err(|source| short("program header", source))?);
        }
        Ok(ElfFile { bytes, class: header.class, sections, program_headers })
    }

    /// The section table, with gABI extended numbering.
    fn raw_sections(bytes: Bytes<'_>, header: &ElfHeader) -> Result<Vec<RawSection>, ElfError> {
        let sizes = header.class.sizes();
        if header.section_offset == 0 {
            return Ok(Vec::new());
        }
        if usize::from(header.section_entry_size) < sizes.section {
            return Err(ElfError::BadEntrySize { what: "section header", size: u64::from(header.section_entry_size) });
        }
        let entry = u64::from(header.section_entry_size);
        let read = |index: u64| -> Result<RawSection, ElfError> {
            let at = nth(header.section_offset, index, entry)
                .and_then(|at| usize::try_from(at).ok())
                .ok_or(ElfError::BadSectionIndex { index })?;
            let raw = bytes.sub(at, sizes.section).map_err(|source| short("section header", source))?;
            decode_section_header(raw, header.class).map_err(|source| short("section header", source))
        };
        let count = match header.section_count {
            0 => read(0)?.size,
            count => u64::from(count),
        };
        if count > MAX_SECTIONS {
            return Err(ElfError::TooMany { what: "sections", count });
        }
        let mut sections = Vec::new();
        sections
            .try_reserve_exact(count as usize)
            .map_err(|_| ElfError::Allocation { what: "sections", count })?;
        for index in 0..count {
            sections.push(read(index)?);
        }
        Ok(sections)
    }

    #[must_use]
    pub fn class(&self) -> ElfClass {
        self.class
    }

    #[must_use]
    pub fn sections(&self) -> &[ElfSection] {
        &self.sections
    }

    /// The section named `name`; `.debug_x` also finds `.zdebug_x`.
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&ElfSection> {
        self.sections.iter().find(|section| section.name == name).or_else(|| {
            let rest = name.strip_prefix(".debug")?;
            let zdebug = format!(".zdebug{rest}");
            self.sections.iter().find(|section| section.name == zdebug)
        })
    }

    /// A section's contents, decompressed when it is compressed.
    pub fn section_bytes(&self, section: &ElfSection) -> Result<SectionBytes<'a>, ElfError> {
        if section.kind == SHT_NOBITS {
            return Ok(SectionBytes::Borrowed(&[]));
        }
        let at = usize::try_from(section.offset).map_err(|_| short("section", ReadError::Truncated { at: usize::MAX, len: 0 }))?;
        let len = usize::try_from(section.size).map_err(|_| short("section", ReadError::Truncated { at, len: usize::MAX }))?;
        let raw = self.bytes.sub(at, len).map_err(|source| short("section", source))?;
        if section.flags & SHF_COMPRESSED != 0 {
            let sizes = self.class.sizes();
            let kind = raw.u32_le(0).map_err(|source| short("compression header", source))?;
            let size = match self.class {
                ElfClass::Elf32 => u64::from(raw.u32_le(4).map_err(|source| short("compression header", source))?),
                ElfClass::Elf64 => raw.u64_le(8).map_err(|source| short("compression header", source))?,
            };
            let codec = match kind {
                ELFCOMPRESS_ZLIB => Codec::Zlib,
                ELFCOMPRESS_ZSTD => Codec::Zstd,
                other => return Err(ElfError::UnknownCompression { kind: other }),
            };
            let body = raw.tail(sizes.compression_header).map_err(|source| short("compression header", source))?;
            return Ok(SectionBytes::Decompressed(decompress_to_vec(codec, body.as_slice(), size, MAX_SECTION_BYTES)?));
        }
        if section.name.starts_with(".zdebug") {
            let magic = raw.array::<4>(0).map_err(|source| short(".zdebug header", source))?;
            if &magic == b"ZLIB" {
                let size = u64::from_be_bytes(raw.array::<8>(4).map_err(|source| short(".zdebug header", source))?);
                let body = raw.tail(12).map_err(|source| short(".zdebug header", source))?;
                return Ok(SectionBytes::Decompressed(decompress_to_vec(Codec::Zlib, body.as_slice(), size, MAX_SECTION_BYTES)?));
            }
        }
        Ok(SectionBytes::Borrowed(raw.as_slice()))
    }

    /// The GNU build-id, from a note section, else from a `PT_NOTE`.
    pub fn build_id(&self) -> Result<Option<BuildId>, ElfError> {
        for section in self.sections.iter().filter(|section| section.kind == SHT_NOTE) {
            let notes = self.section_bytes(section)?;
            let found = find_note(Bytes::new(notes.as_slice()), note_alignment(section.align), GNU_NOTE_NAME, NT_GNU_BUILD_ID);
            if let Some(id) = found.and_then(build_id_of) {
                return Ok(Some(id));
            }
        }
        for segment in self.program_headers.iter().filter(|segment| segment.kind == PT_NOTE) {
            let Ok(at) = usize::try_from(segment.offset) else {
                continue;
            };
            let Ok(len) = usize::try_from(segment.file_size) else {
                continue;
            };
            let Ok(notes) = self.bytes.sub(at, len) else {
                continue;
            };
            if let Some(id) = find_note(notes, note_alignment(segment.align), GNU_NOTE_NAME, NT_GNU_BUILD_ID).and_then(build_id_of) {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// `.gnu_debugaltlink`: a path, its NUL, then the supplement's build-id.
    pub fn alt_link(&self) -> Result<Option<AltLink>, ElfError> {
        let Some(section) = self.section(".gnu_debugaltlink") else {
            return Ok(None);
        };
        let bytes = self.section_bytes(section)?;
        let link = Bytes::new(bytes.as_slice());
        let path = link.cstr(0, MAX_NAME_BYTES).map_err(|source| short(".gnu_debugaltlink", source))?;
        let rest = link.tail(path.len().saturating_add(1)).map_err(|source| short(".gnu_debugaltlink", source))?;
        let Some(build_id) = build_id_of(rest.as_slice()) else {
            return Ok(None);
        };
        let path = String::from_utf8_lossy(path);
        let file_name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        Ok(Some(AltLink { file_name, build_id }))
    }

    /// Every defined function and data symbol of `.symtab`, else of
    /// `.dynsym`, at its link-time address.
    pub fn symbols(&self) -> Result<Vec<ElfSymbol>, ElfError> {
        let table = self
            .sections
            .iter()
            .find(|section| section.kind == SHT_SYMTAB)
            .or_else(|| self.sections.iter().find(|section| section.kind == SHT_DYNSYM));
        let Some(table) = table else {
            return Ok(Vec::new());
        };
        let entry = (self.class.sizes().symbol as u64).max(table.entry_size);
        let count = table.size.checked_div(entry).unwrap_or(0);
        if count > MAX_SYMBOLS {
            return Err(ElfError::TooMany { what: "symbols", count });
        }
        let strings_section = self
            .sections
            .get(table.link as usize)
            .ok_or(ElfError::BadSectionIndex { index: u64::from(table.link) })?;
        let strings_bytes = self.section_bytes(strings_section)?;
        let strings = Bytes::new(strings_bytes.as_slice());
        let entries_bytes = self.section_bytes(table)?;
        let entries = Bytes::new(entries_bytes.as_slice());
        let mut symbols = Vec::new();
        symbols
            .try_reserve(count as usize)
            .map_err(|_| ElfError::Allocation { what: "symbols", count })?;
        for index in 0..count {
            let at = index.checked_mul(entry).and_then(|at| usize::try_from(at).ok()).ok_or(ElfError::TooMany { what: "symbols", count })?;
            let raw = entries.sub(at, self.class.sizes().symbol).map_err(|source| short("symbol", source))?;
            let symbol = decode_symbol(raw, self.class).map_err(|source| short("symbol", source))?;
            let kind = match symbol.info & 0x0F {
                STT_FUNC => SymbolKind::Function,
                STT_OBJECT => SymbolKind::Object,
                _ => continue,
            };
            if symbol.section == SHN_UNDEF || symbol.value == 0 {
                continue;
            }
            let name = strings.cstr(symbol.name as usize, MAX_NAME_BYTES).unwrap_or(&[]);
            if name.is_empty() {
                continue;
            }
            symbols.push(ElfSymbol {
                name: String::from_utf8_lossy(name).into_owned(),
                address: symbol.value,
                size: symbol.size,
                kind,
            });
        }
        Ok(symbols)
    }

    /// The lowest `PT_LOAD` address, page-aligned down: 0 for a PIE or a
    /// shared object, 0x400000 for a classic x86-64 executable; 0 when the
    /// file has no loadable segment.
    #[must_use]
    pub fn link_base(&self) -> u64 {
        link_base(&self.program_headers).unwrap_or(0)
    }
}

/// The lowest `PT_LOAD` address, rounded down to 4 KiB.
fn link_base(program_headers: &[ProgramHeader]) -> Option<u64> {
    program_headers
        .iter()
        .filter(|header| header.kind == PT_LOAD)
        .map(|header| header.address & !0xFFF)
        .min()
}

/// What a loaded ELF image says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElfImageIdentity {
    /// `None` when no `PT_NOTE` carries `NT_GNU_BUILD_ID`.
    pub build_id: Option<BuildId>,
    /// The link-time address of the image's first mapped byte: a guest
    /// address minus the module's base is an [`Rva`], and `link_base + Rva`
    /// is the address DWARF and `.symtab` use.
    pub link_base: u64,
}

impl ElfImageIdentity {
    /// This image as a module named `file_name`, when it has a build-id: an
    /// ELF file without one cannot be matched to its debug information.
    #[must_use]
    pub fn module_identity(&self, file_name: String) -> Option<ModuleIdentity> {
        self.build_id.clone().map(|build_id| ModuleIdentity::Elf { file_name, build_id })
    }
}

/// Why a loaded ELF image's identity did not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ElfImageError {
    #[error("the bytes at the module's base are not an ELF header")]
    NotElf,
    #[error("the ELF headers do not parse: {0}")]
    Headers(ElfError),
    #[error("the {what} ({len} bytes at RVA {:#x}) did not read", rva.0)]
    Unreadable { what: &'static str, rva: Rva, len: usize },
    #[error("the image has no PT_LOAD segment")]
    NoLoadSegment,
    #[error("the build-id note ({len} bytes at RVA {:#x}) did not read", rva.0)]
    NoteUnreadable { rva: Rva, len: usize },
    #[error("no memory for {count} program headers")]
    Allocation { count: u16 },
}

/// The identity of the ELF image a guest has loaded; `read` gives the
/// image's bytes at `base + rva`. Section headers are not mapped, so the
/// build-id is found through the program headers' `PT_NOTE` segments; the
/// ELF and program headers sit in the image's first page.
pub fn identity_from_image<R>(read: &mut R) -> Result<ElfImageIdentity, ElfImageError>
where
    R: FnMut(Rva, &mut [u8]) -> ImageRead,
{
    let mut first = [0u8; ELF_HEADER_READ];
    match read(Rva(0), &mut first) {
        ImageRead::Read => {}
        ImageRead::Unreadable => {
            return Err(ElfImageError::Unreadable { what: "ELF header", rva: Rva(0), len: ELF_HEADER_READ })
        }
    }
    let header = match decode_header(Bytes::new(&first)) {
        Ok(header) => header,
        Err(ElfError::NotElf) => return Err(ElfImageError::NotElf),
        Err(other) => return Err(ElfImageError::Headers(other)),
    };
    let sizes = header.class.sizes();
    let count = header.program_count.min(MAX_PROGRAM_HEADERS);
    let mut program_headers = Vec::new();
    program_headers
        .try_reserve_exact(usize::from(count))
        .map_err(|_| ElfImageError::Allocation { count })?;
    for index in 0..u64::from(count) {
        let at = nth(header.program_offset, index, u64::from(header.program_entry_size))
            .ok_or(ElfImageError::Headers(ElfError::TooMany { what: "program headers", count: u64::from(count) }))?;
        let mut raw = [0u8; 56];
        let part = raw.get_mut(..sizes.program).unwrap_or(&mut []);
        match read(Rva(at), part) {
            ImageRead::Read => {}
            ImageRead::Unreadable => {
                return Err(ElfImageError::Unreadable { what: "program header", rva: Rva(at), len: sizes.program })
            }
        }
        let decoded = decode_program_header(Bytes::new(part), header.class)
            .map_err(|source| ElfImageError::Headers(short("program header", source)))?;
        program_headers.push(decoded);
    }
    let link_base = link_base(&program_headers).ok_or(ElfImageError::NoLoadSegment)?;
    for segment in program_headers.iter().filter(|segment| segment.kind == PT_NOTE) {
        let rva = Rva(segment.address.wrapping_sub(link_base));
        let len = usize::try_from(segment.file_size).map_or(MAX_NOTE_BYTES, |len| len.min(MAX_NOTE_BYTES));
        let mut raw = [0u8; MAX_NOTE_BYTES];
        let notes = raw.get_mut(..len).unwrap_or(&mut []);
        match read(rva, notes) {
            ImageRead::Read => {}
            ImageRead::Unreadable => return Err(ElfImageError::NoteUnreadable { rva, len }),
        }
        if let Some(id) = find_note(Bytes::new(notes), note_alignment(segment.align), GNU_NOTE_NAME, NT_GNU_BUILD_ID).and_then(build_id_of) {
            return Ok(ElfImageIdentity { build_id: Some(id), link_base });
        }
    }
    Ok(ElfImageIdentity { build_id: None, link_base })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::compress::sample_text;
    use rusty_box_symbols_fixtures::elf::{
        build_elf, load_image, ElfSpec, Encoding, ExtraSection, BUILD_ID, FUNC_LEN, TEXT_RVA,
    };

    fn info_section(encoding: Encoding) -> ExtraSection {
        ExtraSection { name: ".debug_info".to_owned(), data: sample_text(20_000), encoding }
    }

    /// Reads a loaded image whose `None` bytes are paged out.
    fn read_image(image: &[Option<u8>], rva: Rva, buf: &mut [u8]) -> ImageRead {
        let start = rva.0 as usize;
        let Some(bytes) = image.get(start..start + buf.len()) else {
            return ImageRead::Unreadable;
        };
        for (to, from) in buf.iter_mut().zip(bytes) {
            match from {
                Some(byte) => *to = *byte,
                None => return ImageRead::Unreadable,
            }
        }
        ImageRead::Read
    }

    #[test]
    fn a_file_names_its_build_id_symbols_and_link_base() {
        for spec in [ElfSpec::pie(), ElfSpec::exec(), ElfSpec::pie32()] {
            let bytes = build_elf(spec, &[]);
            let file = ElfFile::parse(&bytes).unwrap();
            assert_eq!(file.build_id().unwrap(), Some(BuildId(BUILD_ID.to_vec())), "{spec:?}");
            assert_eq!(file.link_base(), spec.link_base, "{spec:?}");
            let symbols = file.symbols().unwrap();
            let names: Vec<&str> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
            assert_eq!(names, ["widget_init", "helper_no_dwarf"], "{spec:?}");
            assert_eq!(symbols[0].address, spec.link_base + TEXT_RVA);
            assert_eq!(symbols[0].size, FUNC_LEN);
            assert_eq!(symbols[0].kind, SymbolKind::Function);
        }
    }

    #[test]
    fn compressed_sections_read_as_their_plain_bytes() {
        let plain = sample_text(20_000);
        for encoding in [Encoding::Plain, Encoding::Zlib, Encoding::Zstd, Encoding::Zdebug] {
            for spec in [ElfSpec::pie(), ElfSpec::pie32()] {
                let bytes = build_elf(spec, &[info_section(encoding)]);
                let file = ElfFile::parse(&bytes).unwrap();
                let section = file.section(".debug_info").expect("found by its plain name");
                assert_eq!(file.section_bytes(section).unwrap().as_slice(), plain.as_slice(), "{encoding:?} {spec:?}");
            }
        }
    }

    #[test]
    fn a_debugaltlink_names_its_supplement() {
        let mut data = b"/usr/lib/debug/.dwz/x86_64-redhat-linux-gnu/foo.debug\0".to_vec();
        data.extend_from_slice(&[0xAB; 20]);
        let section = ExtraSection { name: ".gnu_debugaltlink".to_owned(), data, encoding: Encoding::Plain };
        let bytes = build_elf(ElfSpec::pie(), &[section]);
        let file = ElfFile::parse(&bytes).unwrap();
        assert_eq!(
            file.alt_link().unwrap(),
            Some(AltLink { file_name: "foo.debug".to_owned(), build_id: BuildId(vec![0xAB; 20]) })
        );
    }

    #[test]
    fn big_endian_and_other_bytes_are_refused() {
        let mut bytes = build_elf(ElfSpec::pie(), &[]);
        bytes[5] = 2;
        assert!(matches!(ElfFile::parse(&bytes), Err(ElfError::BigEndian)));
        assert!(matches!(ElfFile::parse(b"MZ\x90\x00"), Err(ElfError::NotElf)));
        assert!(matches!(ElfFile::parse(b"\x7FELF"), Err(ElfError::Short { .. })));
    }

    #[test]
    fn a_section_table_past_the_end_is_refused() {
        let mut bytes = build_elf(ElfSpec::pie(), &[]);
        // e_shoff (64-bit, offset 40) far past the file.
        bytes[40..48].copy_from_slice(&0x7FFF_FFFF_0000u64.to_le_bytes());
        assert!(matches!(ElfFile::parse(&bytes), Err(ElfError::Short { .. })));
    }

    #[test]
    fn a_loaded_image_names_its_build_id_and_link_base() {
        for spec in [ElfSpec::pie(), ElfSpec::exec(), ElfSpec::pie32()] {
            let bytes = build_elf(spec, &[]);
            let image = load_image(&bytes, spec.link_base);
            let identity = identity_from_image(&mut |rva, buf: &mut [u8]| read_image(&image, rva, buf)).unwrap();
            assert_eq!(identity.build_id, Some(BuildId(BUILD_ID.to_vec())), "{spec:?}");
            assert_eq!(identity.link_base, spec.link_base, "{spec:?}");
            assert_eq!(
                identity.module_identity("widget".to_owned()),
                Some(ModuleIdentity::Elf { file_name: "widget".to_owned(), build_id: BuildId(BUILD_ID.to_vec()) })
            );
        }
    }

    #[test]
    fn a_paged_out_note_is_reported_not_guessed() {
        let bytes = build_elf(ElfSpec::pie(), &[]);
        let mut image = load_image(&bytes, 0);
        let note = ElfFile::parse(&bytes).unwrap().section(".note.gnu.build-id").unwrap().offset as usize;
        image[note] = None;
        assert!(matches!(
            identity_from_image(&mut |rva, buf: &mut [u8]| read_image(&image, rva, buf)),
            Err(ElfImageError::NoteUnreadable { .. })
        ));
    }

    #[test]
    fn bytes_that_are_not_an_elf_image_are_refused() {
        let mut image: Vec<Option<u8>> = vec![Some(0); 64];
        image[0] = Some(b'M');
        image[1] = Some(b'Z');
        assert!(matches!(
            identity_from_image(&mut |rva, buf: &mut [u8]| read_image(&image, rva, buf)),
            Err(ElfImageError::NotElf)
        ));
    }
}
