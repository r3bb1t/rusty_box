//! A PE image's identity, read through a reader the caller supplies, so one
//! parser serves an image a guest has loaded (RVA = offset from its base,
//! with pages that may be paged out) and a binary on disk (RVA mapped
//! through its section table, [`FileImage`]). Field offsets follow the
//! PE/COFF specification (Microsoft Learn, "PE Format").
//!
//! Tier P (spec §15): the `decode_*` functions turn one header or record,
//! already read into a fixed buffer, into numbers and borrowed bytes. Tier A:
//! [`read_headers`] and [`read_pe_identity`] call the caller's reader into
//! fixed buffers and collect the results; the reader itself is not under the
//! proof, since a guest's reader reaches far into the emulator.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use crate::identity::{Guid, ModuleIdentity, PdbId, PeImageId};
use crate::read::{copy_prefix, Bytes, ReadError};
use crate::units::Rva;

/// `IMAGE_DOS_SIGNATURE`.
const MZ: u16 = 0x5A4D;
/// The DOS header's size; `e_lfanew` sits at +0x3C.
const DOS_HEADER_BYTES: usize = 0x40;
/// `PE\0\0` and the 20-byte `IMAGE_FILE_HEADER`.
const NT_PREFIX_BYTES: usize = 24;
/// An `e_lfanew` past this is not a real image's (the loader maps the
/// headers from the image's first pages).
const MAX_LFANEW: u32 = 0x1000_0000;
/// PE Format, "COFF File Header": the Windows loader limits the number of
/// sections to 96.
const MAX_SECTIONS: u16 = 96;
const SECTION_HEADER_BYTES: usize = 40;
/// The optional-header bytes read: every field used lies in the first 168
/// (PE32+'s debug data-directory entry ends there); 240 is PE32+'s size.
const OPTIONAL_READ_BYTES: usize = 240;
/// `IMAGE_DIRECTORY_ENTRY_DEBUG`.
const DEBUG_DIRECTORY_INDEX: u32 = 6;
const DEBUG_ENTRY_BYTES: usize = 28;
/// A debug directory with more entries than this is not a real image's
/// (link.exe writes a handful: CodeView, POGO, VC feature, repro).
const MAX_DEBUG_ENTRIES: usize = 64;
/// The most of a CodeView record read: a 24-byte RSDS head and a path, which
/// Windows tools keep far shorter than this.
const MAX_CODEVIEW_BYTES: usize = 0x1000;
/// `IMAGE_DEBUG_TYPE_CODEVIEW`.
pub const IMAGE_DEBUG_TYPE_CODEVIEW: u32 = 2;

/// What a reader of image bytes answers: the whole buffer filled, or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum ImageRead {
    Read,
    Unreadable,
}

/// Which optional header an image carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalHeaderKind {
    /// Magic 0x10B: a 32-bit image.
    Pe32,
    /// Magic 0x20B: a 64-bit image.
    Pe32Plus,
}

/// One entry of the optional header's data directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataDirectory {
    pub rva: Rva,
    pub size: u32,
}

/// The fields of `IMAGE_FILE_HEADER` the identity needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileHeader {
    pub machine: u16,
    pub number_of_sections: u16,
    pub time_date_stamp: u32,
    pub size_of_optional_header: u16,
}

/// The fields of the optional header the identity needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionalHeader {
    pub kind: OptionalHeaderKind,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    /// Data directory 6, when the header has it and it is not empty.
    pub debug_directory: Option<DataDirectory>,
}

/// Where one kind's optional-header fields sit.
#[derive(Clone, Copy)]
struct OptionalLayout {
    kind: OptionalHeaderKind,
    rva_count_at: usize,
    directories_at: usize,
}

/// One `IMAGE_SECTION_HEADER`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionHeader {
    pub name: [u8; 8],
    pub virtual_size: u32,
    pub virtual_address: Rva,
    pub size_of_raw_data: u32,
    pub pointer_to_raw_data: u32,
    pub characteristics: u32,
}

impl SectionHeader {
    /// The name, NUL padding removed.
    #[must_use]
    pub fn name_text(&self) -> String {
        let end = self.name.iter().position(|byte| *byte == 0).unwrap_or(8);
        String::from_utf8_lossy(self.name.get(..end).unwrap_or(&self.name)).into_owned()
    }

    /// How far the loader maps the section: its virtual size, or its raw
    /// size when that is larger.
    fn span(&self) -> u64 {
        u64::from(self.virtual_size.max(self.size_of_raw_data))
    }

    /// Whether `rva` lies inside the section as the loader maps it.
    #[must_use]
    pub fn contains(&self, rva: Rva) -> bool {
        rva.0
            .checked_sub(self.virtual_address.0)
            .is_some_and(|within| within < self.span())
    }
}

/// One `IMAGE_DEBUG_DIRECTORY` entry: the fields the CodeView lookup reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebugEntry {
    pub kind: u32,
    pub size_of_data: u32,
    pub address_of_raw_data: u32,
    pub pointer_to_raw_data: u32,
}

/// A CodeView record's PDB reference, its path borrowed from the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeView<'a> {
    /// PDB 7.0: a GUID, an age and the PDB's recorded path.
    Rsds { guid: Guid, age: u32, path: &'a [u8] },
    /// PDB 2.0: a timestamp signature, no GUID.
    Nb10 { signature: u32, age: u32, path: &'a [u8] },
    Malformed { reason: &'static str },
    Unknown { signature: [u8; 4] },
}

/// The headers an identity is read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeHeaders {
    pub machine: u16,
    pub time_date_stamp: u32,
    pub kind: OptionalHeaderKind,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub debug_directory: Option<DataDirectory>,
    pub sections: Vec<SectionHeader>,
}

impl PeHeaders {
    #[must_use]
    pub fn image_id(&self) -> PeImageId {
        PeImageId {
            time_date_stamp: self.time_date_stamp,
            size_of_image: self.size_of_image,
        }
    }

    #[must_use]
    pub fn section_of(&self, rva: Rva) -> Option<&SectionHeader> {
        self.sections.iter().find(|section| section.contains(rva))
    }
}

/// Why an image's headers did not parse: the module has no identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    #[error("the {what} ({len} bytes at RVA {:#x}) did not read", rva.0)]
    Unreadable {
        what: &'static str,
        rva: Rva,
        len: usize,
    },
    #[error("the {what} is cut short: {source}")]
    Short {
        what: &'static str,
        #[source]
        source: ReadError,
    },
    #[error("no MZ signature (found {found:#06x})")]
    BadDosMagic { found: u16 },
    #[error("e_lfanew {e_lfanew:#x} is out of range")]
    BadLfanew { e_lfanew: u32 },
    #[error("no PE signature (found {found:02x?})")]
    BadPeSignature { found: [u8; 4] },
    #[error("unknown optional-header magic {found:#06x}")]
    BadOptionalMagic { found: u16 },
    #[error("{count} sections is not a real image's")]
    TooManySections { count: u16 },
    #[error("no memory for {count} {what}")]
    Allocation { what: &'static str, count: usize },
}

/// Why an image's PDB identity is not known.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PdbUnavailable {
    #[error("the image has no debug directory")]
    NoDebugDirectory,
    #[error("the debug directory ({size} bytes at RVA {:#x}) did not read", rva.0)]
    DebugDirectoryUnreadable { rva: Rva, size: u32 },
    #[error("the debug directory has no CodeView entry")]
    NoCodeViewEntry,
    #[error("the CodeView record is in the file only (file offset {pointer_to_raw_data:#x})")]
    CodeViewNotMapped { pointer_to_raw_data: u32 },
    #[error("the CodeView record ({size} bytes at RVA {:#x}) did not read", rva.0)]
    CodeViewUnreadable { rva: Rva, size: u32 },
    #[error("the CodeView record is malformed: {reason}")]
    CodeViewMalformed { reason: &'static str },
    #[error("the CodeView record's signature {signature:02x?} is not RSDS or NB10")]
    UnknownCodeViewSignature { signature: [u8; 4] },
    /// A PDB 2.0 reference: a timestamp, no GUID, so no symbol-store key.
    #[error("the image names a PDB 2.0 file ({pdb_path}), which has no GUID")]
    Nb10 {
        signature: u32,
        age: u32,
        pdb_path: String,
    },
}

/// The PDB half of a module's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdbLookup {
    Found(PdbId),
    Unavailable(PdbUnavailable),
}

/// What an image says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeIdentity {
    pub headers: PeHeaders,
    pub image: PeImageId,
    pub pdb: PdbLookup,
}

impl PeIdentity {
    /// This image as a module named `file_name`: its PDB half is known only
    /// when its CodeView record read.
    #[must_use]
    pub fn module_identity(&self, file_name: String) -> ModuleIdentity {
        let pdb = match &self.pdb {
            PdbLookup::Found(pdb) => Some(pdb.clone()),
            PdbLookup::Unavailable(_) => None,
        };
        ModuleIdentity::Pe {
            file_name,
            image: self.image,
            pdb,
        }
    }
}

/// [`HeaderError::Short`] for the named header part.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn cut_short(what: &'static str, source: ReadError) -> HeaderError {
    HeaderError::Short { what, source }
}

/// The DOS header: its magic, and `e_lfanew`, where the NT headers start.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_dos_header(dos: Bytes<'_>) -> Result<u32, HeaderError> {
    let magic = dos.u16_le(0).map_err(|source| cut_short("DOS header", source))?;
    if magic != MZ {
        return Err(HeaderError::BadDosMagic { found: magic });
    }
    let e_lfanew = dos.u32_le(0x3C).map_err(|source| cut_short("DOS header", source))?;
    if !(0x40..=MAX_LFANEW).contains(&e_lfanew) {
        return Err(HeaderError::BadLfanew { e_lfanew });
    }
    Ok(e_lfanew)
}

/// `PE\0\0` and `IMAGE_FILE_HEADER`.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_file_header(prefix: Bytes<'_>) -> Result<FileHeader, HeaderError> {
    let what = "NT headers";
    let signature = prefix.array::<4>(0).map_err(|source| cut_short(what, source))?;
    if &signature != b"PE\0\0" {
        return Err(HeaderError::BadPeSignature { found: signature });
    }
    let machine = prefix.u16_le(4).map_err(|source| cut_short(what, source))?;
    let number_of_sections = prefix.u16_le(6).map_err(|source| cut_short(what, source))?;
    let time_date_stamp = prefix.u32_le(8).map_err(|source| cut_short(what, source))?;
    let size_of_optional_header = prefix.u16_le(20).map_err(|source| cut_short(what, source))?;
    if number_of_sections > MAX_SECTIONS {
        return Err(HeaderError::TooManySections {
            count: number_of_sections,
        });
    }
    Ok(FileHeader {
        machine,
        number_of_sections,
        time_date_stamp,
        size_of_optional_header,
    })
}

/// The optional header's first bytes. SizeOfImage (+56) and SizeOfHeaders
/// (+60) sit at the same offset in both kinds; NumberOfRvaAndSizes and the
/// data directories do not. A directory past the header's declared size, or
/// past `NumberOfRvaAndSizes`, is absent.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_optional_header(optional: Bytes<'_>, declared_size: u16) -> Result<OptionalHeader, HeaderError> {
    let what = "optional header";
    let magic = optional.u16_le(0).map_err(|source| cut_short(what, source))?;
    let layout = match magic {
        0x10B => OptionalLayout {
            kind: OptionalHeaderKind::Pe32,
            rva_count_at: 92,
            directories_at: 96,
        },
        0x20B => OptionalLayout {
            kind: OptionalHeaderKind::Pe32Plus,
            rva_count_at: 108,
            directories_at: 112,
        },
        found => return Err(HeaderError::BadOptionalMagic { found }),
    };
    let size_of_image = optional.u32_le(56).map_err(|source| cut_short(what, source))?;
    let size_of_headers = optional.u32_le(60).map_err(|source| cut_short(what, source))?;
    let rva_count = optional.u32_le(layout.rva_count_at).unwrap_or(0);
    // Directory 6, 8 bytes each: offsets far below usize's limit.
    let debug_at = layout.directories_at.wrapping_add(48);
    let in_header = debug_at.wrapping_add(8) <= usize::from(declared_size);
    let debug_directory = if rva_count > DEBUG_DIRECTORY_INDEX && in_header {
        match (optional.u32_le(debug_at), optional.u32_le(debug_at.wrapping_add(4))) {
            (Ok(rva), Ok(size)) if rva != 0 && size != 0 => Some(DataDirectory {
                rva: Rva(u64::from(rva)),
                size,
            }),
            (Ok(_) | Err(_), Ok(_) | Err(_)) => None,
        }
    } else {
        None
    };
    Ok(OptionalHeader {
        kind: layout.kind,
        size_of_image,
        size_of_headers,
        debug_directory,
    })
}

/// One `IMAGE_SECTION_HEADER` (40 bytes).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_section(raw: Bytes<'_>) -> Result<SectionHeader, ReadError> {
    Ok(SectionHeader {
        name: raw.array::<8>(0)?,
        virtual_size: raw.u32_le(8)?,
        virtual_address: Rva(u64::from(raw.u32_le(12)?)),
        size_of_raw_data: raw.u32_le(16)?,
        pointer_to_raw_data: raw.u32_le(20)?,
        characteristics: raw.u32_le(36)?,
    })
}

/// One `IMAGE_DEBUG_DIRECTORY` entry (28 bytes).
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_debug_entry(raw: Bytes<'_>) -> Result<DebugEntry, ReadError> {
    Ok(DebugEntry {
        kind: raw.u32_le(12)?,
        size_of_data: raw.u32_le(16)?,
        address_of_raw_data: raw.u32_le(20)?,
        pointer_to_raw_data: raw.u32_le(24)?,
    })
}

/// The bytes before the first NUL, or all of them when there is none: a
/// record cut at the cap still names what it holds.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn until_nul(bytes: &[u8]) -> &[u8] {
    match bytes.iter().position(|byte| *byte == 0) {
        Some(end) => bytes.get(..end).unwrap_or(bytes),
        None => bytes,
    }
}

/// A CodeView record's PDB reference: RSDS (PDB 7.0) gives an identity;
/// NB10 (PDB 2.0) is reported, since it carries no GUID.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_codeview(record: Bytes<'_>) -> CodeView<'_> {
    let Ok(signature) = record.array::<4>(0) else {
        return CodeView::Malformed {
            reason: "shorter than its signature",
        };
    };
    match &signature {
        b"RSDS" => match (record.array::<16>(4), record.u32_le(20)) {
            (Ok(guid), Ok(age)) => CodeView::Rsds {
                guid: Guid::from_le_bytes(guid),
                age,
                path: until_nul(record.tail(24).map_or(&[][..], Bytes::as_slice)),
            },
            (Ok(_) | Err(_), Ok(_) | Err(_)) => CodeView::Malformed {
                reason: "RSDS shorter than 24 bytes",
            },
        },
        b"NB10" => match (record.u32_le(8), record.u32_le(12)) {
            (Ok(signature), Ok(age)) => CodeView::Nb10 {
                signature,
                age,
                path: until_nul(record.tail(16).map_or(&[][..], Bytes::as_slice)),
            },
            (Ok(_) | Err(_), Ok(_) | Err(_)) => CodeView::Malformed {
                reason: "NB10 shorter than 16 bytes",
            },
        },
        _ => CodeView::Unknown { signature },
    }
}

/// The file-name part of a recorded PDB path, which may use either separator.
#[must_use]
pub fn pdb_file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Fills `buf` from `rva`, or says what did not read.
fn read_exact<R>(read: &mut R, what: &'static str, rva: u64, buf: &mut [u8]) -> Result<(), HeaderError>
where
    R: FnMut(Rva, &mut [u8]) -> ImageRead,
{
    match read(Rva(rva), buf) {
        ImageRead::Read => Ok(()),
        ImageRead::Unreadable => Err(HeaderError::Unreadable {
            what,
            rva: Rva(rva),
            len: buf.len(),
        }),
    }
}

/// `base + index * size`, which the header limits keep far below `u64`'s.
fn nth(base: u64, index: u16, size: usize) -> Option<u64> {
    let size = u64::try_from(size).ok()?;
    base.checked_add(u64::from(index).checked_mul(size)?)
}

/// The DOS header, the NT headers and the section table.
pub fn read_headers<R>(read: &mut R) -> Result<PeHeaders, HeaderError>
where
    R: FnMut(Rva, &mut [u8]) -> ImageRead,
{
    let mut dos = [0u8; DOS_HEADER_BYTES];
    read_exact(read, "DOS header", 0, &mut dos)?;
    let nt_rva = u64::from(decode_dos_header(Bytes::new(&dos))?);
    let mut prefix = [0u8; NT_PREFIX_BYTES];
    read_exact(read, "NT headers", nt_rva, &mut prefix)?;
    let file = decode_file_header(Bytes::new(&prefix))?;
    let unreadable = HeaderError::Unreadable {
        what: "optional header",
        rva: Rva(nt_rva),
        len: usize::from(file.size_of_optional_header),
    };
    // `nt_rva` is at most MAX_LFANEW, so neither sum can overflow.
    let optional_rva = nt_rva.checked_add(24).ok_or(unreadable)?;
    let mut optional_bytes = [0u8; OPTIONAL_READ_BYTES];
    let wanted = usize::from(file.size_of_optional_header).min(OPTIONAL_READ_BYTES);
    let optional_part = optional_bytes.get_mut(..wanted).ok_or(unreadable)?;
    read_exact(read, "optional header", optional_rva, optional_part)?;
    let optional = decode_optional_header(Bytes::new(optional_part), file.size_of_optional_header)?;
    let table_rva = optional_rva
        .checked_add(u64::from(file.size_of_optional_header))
        .ok_or(unreadable)?;
    let count = usize::from(file.number_of_sections);
    let mut sections = Vec::new();
    sections
        .try_reserve_exact(count)
        .map_err(|_| HeaderError::Allocation { what: "section headers", count })?;
    for index in 0..file.number_of_sections {
        let at = nth(table_rva, index, SECTION_HEADER_BYTES).ok_or(unreadable)?;
        let mut raw = [0u8; SECTION_HEADER_BYTES];
        read_exact(read, "section header", at, &mut raw)?;
        let section = decode_section(Bytes::new(&raw)).map_err(|source| cut_short("section header", source))?;
        sections.push(section);
    }
    Ok(PeHeaders {
        machine: file.machine,
        time_date_stamp: file.time_date_stamp,
        kind: optional.kind,
        size_of_image: optional.size_of_image,
        size_of_headers: optional.size_of_headers,
        debug_directory: optional.debug_directory,
        sections,
    })
}

/// The PDB half of the identity: the debug directory's CodeView entry and
/// the record it points at, each read into a fixed buffer.
fn read_pdb_lookup<R>(headers: &PeHeaders, read: &mut R) -> PdbLookup
where
    R: FnMut(Rva, &mut [u8]) -> ImageRead,
{
    let Some(directory) = headers.debug_directory else {
        return PdbLookup::Unavailable(PdbUnavailable::NoDebugDirectory);
    };
    let count = usize::try_from(directory.size)
        .map_or(MAX_DEBUG_ENTRIES, |size| size.checked_div(DEBUG_ENTRY_BYTES).unwrap_or(0))
        .min(MAX_DEBUG_ENTRIES);
    let mut raw_entries = [0u8; DEBUG_ENTRY_BYTES * MAX_DEBUG_ENTRIES];
    let Some(entries) = count
        .checked_mul(DEBUG_ENTRY_BYTES)
        .and_then(|len| raw_entries.get_mut(..len))
        .filter(|entries| !entries.is_empty())
    else {
        return PdbLookup::Unavailable(PdbUnavailable::NoDebugDirectory);
    };
    match read(directory.rva, entries) {
        ImageRead::Read => {}
        ImageRead::Unreadable => {
            return PdbLookup::Unavailable(PdbUnavailable::DebugDirectoryUnreadable {
                rva: directory.rva,
                size: directory.size,
            })
        }
    }
    let codeview = entries
        .chunks_exact(DEBUG_ENTRY_BYTES)
        .filter_map(|raw| decode_debug_entry(Bytes::new(raw)).ok())
        .find(|entry| entry.kind == IMAGE_DEBUG_TYPE_CODEVIEW);
    let Some(entry) = codeview else {
        return PdbLookup::Unavailable(PdbUnavailable::NoCodeViewEntry);
    };
    if entry.address_of_raw_data == 0 {
        return PdbLookup::Unavailable(PdbUnavailable::CodeViewNotMapped {
            pointer_to_raw_data: entry.pointer_to_raw_data,
        });
    }
    let rva = Rva(u64::from(entry.address_of_raw_data));
    let size = usize::try_from(entry.size_of_data).map_or(MAX_CODEVIEW_BYTES, |size| size.min(MAX_CODEVIEW_BYTES));
    let mut raw_record = [0u8; MAX_CODEVIEW_BYTES];
    let Some(record) = raw_record.get_mut(..size) else {
        return PdbLookup::Unavailable(PdbUnavailable::CodeViewMalformed { reason: "larger than its buffer" });
    };
    match read(rva, record) {
        ImageRead::Read => {}
        ImageRead::Unreadable => {
            return PdbLookup::Unavailable(PdbUnavailable::CodeViewUnreadable {
                rva,
                size: entry.size_of_data,
            })
        }
    }
    match decode_codeview(Bytes::new(record)) {
        CodeView::Rsds { guid, age, path } => {
            let path = String::from_utf8_lossy(path);
            PdbLookup::Found(PdbId {
                name: pdb_file_name(&path).to_owned(),
                guid,
                age,
            })
        }
        CodeView::Nb10 { signature, age, path } => PdbLookup::Unavailable(PdbUnavailable::Nb10 {
            signature,
            age,
            pdb_path: String::from_utf8_lossy(path).into_owned(),
        }),
        CodeView::Malformed { reason } => PdbLookup::Unavailable(PdbUnavailable::CodeViewMalformed { reason }),
        CodeView::Unknown { signature } => {
            PdbLookup::Unavailable(PdbUnavailable::UnknownCodeViewSignature { signature })
        }
    }
}

/// A module's identity: the headers must read; the PDB half is read when it
/// can be, and says why when it cannot.
pub fn read_pe_identity<R>(read: &mut R) -> Result<PeIdentity, HeaderError>
where
    R: FnMut(Rva, &mut [u8]) -> ImageRead,
{
    let headers = read_headers(read)?;
    let image = headers.image_id();
    let pdb = read_pdb_lookup(&headers, read);
    Ok(PeIdentity { headers, image, pdb })
}

/// `buf.len()` bytes of `file` at offset `at`, or unreadable.
fn read_file(file: &[u8], at: u64, buf: &mut [u8]) -> ImageRead {
    let source = usize::try_from(at)
        .ok()
        .and_then(|at| Bytes::new(file).slice(at, buf.len()).ok());
    match source {
        Some(source) => {
            let copied = copy_prefix(buf, source);
            if copied == buf.len() {
                ImageRead::Read
            } else {
                ImageRead::Unreadable
            }
        }
        None => ImageRead::Unreadable,
    }
}

/// A PE file's bytes, owned or borrowed, read by RVA through its section
/// table.
pub struct FileImage<B: AsRef<[u8]>> {
    bytes: B,
    headers: PeHeaders,
}

impl<B: AsRef<[u8]>> FileImage<B> {
    /// Parses the headers, which sit at the same offsets in a file and in
    /// the image the loader makes of it.
    pub fn new(bytes: B) -> Result<FileImage<B>, HeaderError> {
        let headers = {
            let mut read = |rva: Rva, buf: &mut [u8]| read_file(bytes.as_ref(), rva.0, buf);
            read_headers(&mut read)?
        };
        Ok(FileImage { bytes, headers })
    }

    #[must_use]
    pub fn headers(&self) -> &PeHeaders {
        &self.headers
    }

    /// `buf.len()` bytes at `rva` as the loader would map them: file bytes,
    /// then zeros past a section's raw data up to its virtual size.
    /// Unreadable when a byte lies outside the headers and every section,
    /// or past the end of the file.
    pub fn read_into(&self, rva: Rva, buf: &mut [u8]) -> ImageRead {
        let headers_end = u64::from(self.headers.size_of_headers);
        let mut filled = 0usize;
        while filled < buf.len() {
            let Some(cursor) = u64::try_from(filled).ok().and_then(|filled| rva.0.checked_add(filled)) else {
                return ImageRead::Unreadable;
            };
            let Some(rest) = buf.get_mut(filled..) else {
                return ImageRead::Unreadable;
            };
            let want = u64::try_from(rest.len()).unwrap_or(u64::MAX);
            let took = if cursor < headers_end {
                let take = want.min(headers_end.wrapping_sub(cursor));
                self.copy_from_file(cursor, take, 0, rest)
            } else {
                let Some(section) = self.headers.section_of(Rva(cursor)) else {
                    return ImageRead::Unreadable;
                };
                // `contains` held, so `within < span` and `take >= 1`.
                let within = cursor.wrapping_sub(section.virtual_address.0);
                let take = want.min(section.span().wrapping_sub(within));
                let from_file = take.min(u64::from(section.size_of_raw_data).saturating_sub(within));
                let file_at = u64::from(section.pointer_to_raw_data).checked_add(within);
                match file_at {
                    Some(file_at) => self.copy_from_file(file_at, from_file, take.wrapping_sub(from_file), rest),
                    None => None,
                }
            };
            let Some(took) = took else {
                return ImageRead::Unreadable;
            };
            filled = filled.saturating_add(took);
        }
        ImageRead::Read
    }

    /// Copies `from_file` bytes of the file at `file_at` into the front of
    /// `out`, then `zeros` zero bytes after them; the number of bytes placed,
    /// or `None` when the file is too short.
    fn copy_from_file(&self, file_at: u64, from_file: u64, zeros: u64, out: &mut [u8]) -> Option<usize> {
        let from_file = usize::try_from(from_file).ok()?;
        let zeros = usize::try_from(zeros).ok()?;
        let total = from_file.checked_add(zeros)?;
        let placed = out.get_mut(..total)?;
        let (file_part, zero_part) = placed.split_at_mut_checked(from_file)?;
        // Bytes wholly past a section's raw data come from no file offset.
        if !file_part.is_empty() {
            match read_file(self.bytes.as_ref(), file_at, file_part) {
                ImageRead::Read => {}
                ImageRead::Unreadable => return None,
            }
        }
        zero_part.fill(0);
        Some(total)
    }
}

/// Why a binary fetched by its image key did not give the PDB identity.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BinaryPdbError {
    #[error("the binary's headers do not parse: {0}")]
    Headers(HeaderError),
    #[error("the binary is image {}, not the wanted {}", found.store_key(), wanted.store_key())]
    WrongImage { wanted: PeImageId, found: PeImageId },
    #[error("the binary names no PDB: {0}")]
    NoPdb(PdbUnavailable),
}

/// The PDB identity of the binary in `bytes`, owned or borrowed, which was
/// fetched by `wanted` because the guest's copy of its debug directory did
/// not read.
pub fn pdb_from_binary<B: AsRef<[u8]>>(bytes: B, wanted: PeImageId) -> Result<PdbId, BinaryPdbError> {
    let file = FileImage::new(bytes).map_err(BinaryPdbError::Headers)?;
    let found = file.headers().image_id();
    if found != wanted {
        return Err(BinaryPdbError::WrongImage { wanted, found });
    }
    let mut read = |rva: Rva, buf: &mut [u8]| file.read_into(rva, buf);
    match read_pdb_lookup(file.headers(), &mut read) {
        PdbLookup::Found(pdb) => Ok(pdb),
        PdbLookup::Unavailable(reason) => Err(BinaryPdbError::NoPdb(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::pe::{loaded_image, pe32_file, put16, put32, rsds};

    const GUID: [u8; 16] = [
        0xF6, 0x72, 0x7B, 0x49, 0x0A, 0x39, 0xFC, 0x44, 0x87, 0x8E, 0x5A, 0x2D, 0x63, 0xB6, 0xCC,
        0x4B,
    ];

    /// Every byte of `image` reads: a guest image with nothing paged out.
    fn whole(image: &[u8], rva: Rva, buf: &mut [u8]) -> ImageRead {
        let at = rva.0 as usize;
        match image.get(at..at + buf.len()) {
            Some(source) => {
                buf.copy_from_slice(source);
                ImageRead::Read
            }
            None => ImageRead::Unreadable,
        }
    }

    #[test]
    fn a_loaded_image_names_its_pdb_and_itself() {
        let image = loaded_image(&rsds(GUID, 3, r"D:\build\out\driver.pdb"));
        let identity = read_pe_identity(&mut |rva, buf: &mut [u8]| whole(&image, rva, buf)).unwrap();
        assert_eq!(identity.image, PeImageId { time_date_stamp: 0x1234_5678, size_of_image: 0x2000 });
        assert_eq!(identity.headers.kind, OptionalHeaderKind::Pe32Plus);
        assert_eq!(identity.headers.sections[0].name_text(), ".rdata");
        assert_eq!(
            identity.module_identity("driver.sys".to_owned()),
            ModuleIdentity::Pe {
                file_name: "driver.sys".to_owned(),
                image: identity.image,
                pdb: Some(PdbId { name: "driver.pdb".to_owned(), guid: Guid::from_le_bytes(GUID), age: 3 }),
            }
        );
    }

    #[test]
    fn a_pdb_2_reference_is_reported_without_an_identity() {
        let mut record = b"NB10".to_vec();
        record.extend_from_slice(&0u32.to_le_bytes());
        record.extend_from_slice(&0x3B7D_8410u32.to_le_bytes());
        record.extend_from_slice(&2u32.to_le_bytes());
        record.extend_from_slice(b"old.pdb\0");
        let image = loaded_image(&record);
        let identity = read_pe_identity(&mut |rva, buf: &mut [u8]| whole(&image, rva, buf)).unwrap();
        assert_eq!(
            identity.pdb,
            PdbLookup::Unavailable(PdbUnavailable::Nb10 {
                signature: 0x3B7D_8410,
                age: 2,
                pdb_path: "old.pdb".to_owned(),
            })
        );
        assert!(matches!(identity.module_identity("old.dll".to_owned()), ModuleIdentity::Pe { pdb: None, .. }));
    }

    #[test]
    fn an_unreadable_header_page_means_no_identity() {
        let image = loaded_image(&rsds(GUID, 1, "a.pdb"));
        let mut read = |rva: Rva, buf: &mut [u8]| {
            if rva.0 >= 0x80 { whole(&image, rva, buf) } else { ImageRead::Unreadable }
        };
        assert_eq!(
            read_pe_identity(&mut read),
            Err(HeaderError::Unreadable { what: "DOS header", rva: Rva(0), len: 0x40 })
        );
    }

    /// `ntdll.dll`'s `.rdata` is pageable. Its headers still give the image
    /// key the binary is fetched by.
    #[test]
    fn a_paged_out_debug_directory_keeps_the_image_identity() {
        let image = loaded_image(&rsds(GUID, 1, "a.pdb"));
        let mut read = |rva: Rva, buf: &mut [u8]| {
            if rva.0 < 0x1000 { whole(&image, rva, buf) } else { ImageRead::Unreadable }
        };
        let identity = read_pe_identity(&mut read).unwrap();
        assert_eq!(identity.image, PeImageId { time_date_stamp: 0x1234_5678, size_of_image: 0x2000 });
        assert_eq!(
            identity.pdb,
            PdbLookup::Unavailable(PdbUnavailable::DebugDirectoryUnreadable { rva: Rva(0x1000), size: 28 })
        );
    }

    #[test]
    fn a_file_is_read_through_its_section_table() {
        let file = FileImage::new(pe32_file(&rsds(GUID, 1, "wntdll.pdb"))).unwrap();
        assert_eq!(file.headers().kind, OptionalHeaderKind::Pe32);
        // RVA 0x2100 is file offset 0x700: an identity reader would miss it.
        let mut four = [0u8; 4];
        assert_eq!(file.read_into(Rva(0x2100), &mut four), ImageRead::Read);
        assert_eq!(&four, b"RSDS");
        // Past a section's raw data, up to its virtual size, the loader zeros.
        let mut two = [0xAAu8; 2];
        assert_eq!(file.read_into(Rva(0x2300), &mut two), ImageRead::Read);
        assert_eq!(two, [0, 0]);
        // Outside every section nothing is mapped.
        assert_eq!(file.read_into(Rva(0x5000), &mut two), ImageRead::Unreadable);
        assert_eq!(file.read_into(Rva(u64::MAX), &mut two), ImageRead::Unreadable);
        let mut read = |rva: Rva, buf: &mut [u8]| file.read_into(rva, buf);
        let identity = read_pe_identity(&mut read).unwrap();
        assert_eq!(
            identity.pdb,
            PdbLookup::Found(PdbId { name: "wntdll.pdb".to_owned(), guid: Guid::from_le_bytes(GUID), age: 1 })
        );
    }

    #[test]
    fn a_binary_fetched_by_its_image_key_gives_its_pdb() {
        let bytes = pe32_file(&rsds(GUID, 7, r"c:\sym\x.pdb"));
        let wanted = PeImageId { time_date_stamp: 0x0BAD_F00D, size_of_image: 0x3000 };
        assert_eq!(
            pdb_from_binary(bytes.clone(), wanted),
            Ok(PdbId { name: "x.pdb".to_owned(), guid: Guid::from_le_bytes(GUID), age: 7 })
        );
        let other = PeImageId { time_date_stamp: 1, size_of_image: 0x3000 };
        assert_eq!(pdb_from_binary(bytes, other), Err(BinaryPdbError::WrongImage { wanted: other, found: wanted }));
    }

    #[test]
    fn a_recorded_path_names_its_pdb_by_its_last_component() {
        assert_eq!(pdb_file_name(r"C:\a\b\ntkrnlmp.pdb"), "ntkrnlmp.pdb");
        assert_eq!(pdb_file_name("/a/b/c.pdb"), "c.pdb");
        assert_eq!(pdb_file_name("plain.pdb"), "plain.pdb");
        assert_eq!(pdb_file_name(""), "");
    }

    #[test]
    fn headers_no_real_image_has_are_refused() {
        let mut image = loaded_image(&rsds(GUID, 1, "a.pdb"));
        put32(&mut image, 0x3C, 0x2000_0000);
        assert_eq!(
            read_pe_identity(&mut |rva, buf: &mut [u8]| whole(&image, rva, buf)),
            Err(HeaderError::BadLfanew { e_lfanew: 0x2000_0000 })
        );
        let mut image = loaded_image(&rsds(GUID, 1, "a.pdb"));
        put16(&mut image, 0x80 + 6, 97);
        assert_eq!(
            read_pe_identity(&mut |rva, buf: &mut [u8]| whole(&image, rva, buf)),
            Err(HeaderError::TooManySections { count: 97 })
        );
        let mut image = loaded_image(&rsds(GUID, 1, "a.pdb"));
        put16(&mut image, 0x80 + 24, 0x30B);
        assert_eq!(
            read_pe_identity(&mut |rva, buf: &mut [u8]| whole(&image, rva, buf)),
            Err(HeaderError::BadOptionalMagic { found: 0x30B })
        );
    }

    #[test]
    fn a_malformed_codeview_record_is_reported() {
        assert_eq!(decode_codeview(Bytes::new(b"RS")), CodeView::Malformed { reason: "shorter than its signature" });
        assert_eq!(decode_codeview(Bytes::new(b"RSDS\x01\x02")), CodeView::Malformed { reason: "RSDS shorter than 24 bytes" });
        assert_eq!(decode_codeview(Bytes::new(b"XXXXyyyy")), CodeView::Unknown { signature: *b"XXXX" });
    }

    /// A record whose declared size passes the cap is read up to the cap;
    /// its path is cut there, and it still names its GUID and age.
    #[test]
    fn a_codeview_record_longer_than_the_cap_still_names_its_pdb() {
        let long_path = format!("{}\\long.pdb", "d".repeat(0x1800));
        let mut image = loaded_image(&rsds(GUID, 9, "x.pdb"));
        let record = rsds(GUID, 9, &long_path);
        image.resize(0x1100 + record.len(), 0);
        image[0x1100..0x1100 + record.len()].copy_from_slice(&record);
        put32(&mut image, 0x1000 + 16, record.len() as u32);
        let identity = read_pe_identity(&mut |rva, buf: &mut [u8]| whole(&image, rva, buf)).unwrap();
        let PdbLookup::Found(pdb) = identity.pdb else { panic!("found") };
        assert_eq!(pdb.guid, Guid::from_le_bytes(GUID));
        assert_eq!(pdb.age, 9);
    }
}
