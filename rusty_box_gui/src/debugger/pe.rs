//! A loaded image's section table, parsed from its headers.
//!
//! The format is Microsoft's PE/COFF, documented in the PE specification:
//! `IMAGE_DOS_HEADER.e_lfanew` at offset 0x3C points at the NT headers
//! (`PE\0\0` signature, then the 20-byte COFF file header, then the optional
//! header). The section table follows the optional header, forty bytes per
//! `IMAGE_SECTION_HEADER`. PE32 and PE32+ differ only inside the optional
//! header, whose length is read from the file header rather than assumed, so
//! one parser serves both.

/// How many bytes of an image's start hold its headers: the DOS header, the
/// NT headers and the section table fit inside this for every image a loader
/// maps (the loader itself requires the headers to lie in the first page).
pub(crate) const HEADER_BYTES: usize = 0x1000;

/// One `IMAGE_SECTION_HEADER` on disk.
const SECTION_HEADER_LEN: usize = 40;
/// The most sections the loader accepts.
const MAX_SECTIONS: u16 = 96;

/// `IMAGE_SCN_CNT_CODE`, `_MEM_EXECUTE`, `_MEM_READ`, `_MEM_WRITE`.
const SCN_CNT_CODE: u32 = 0x0000_0020;
const SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const SCN_MEM_READ: u32 = 0x4000_0000;
const SCN_MEM_WRITE: u32 = 0x8000_0000;

/// One section as the image lays it out in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeSection {
    /// The eight-byte name, cut at its first NUL.
    pub(crate) name: String,
    /// Where the section sits, relative to the image base.
    pub(crate) rva: u32,
    /// Its size once mapped.
    pub(crate) virtual_size: u32,
    /// Its size on disk; zero for a section that is only reserved (`.bss`).
    pub(crate) raw_size: u32,
    pub(crate) readable: bool,
    pub(crate) writable: bool,
    pub(crate) executable: bool,
    /// The section holds code (`IMAGE_SCN_CNT_CODE`).
    pub(crate) code: bool,
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    bytes
        .get(at..)
        .and_then(<[u8]>::first_chunk)
        .copied()
        .map(u16::from_le_bytes)
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..)
        .and_then(<[u8]>::first_chunk)
        .copied()
        .map(u32::from_le_bytes)
}

/// The bytes of `bytes` before its first NUL, or all of them when it has none.
fn before_nul(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|&byte| byte == 0)
        .and_then(|end| bytes.get(..end))
        .unwrap_or(bytes)
}

/// The sections of the image whose headers begin at `header`, or `None` when
/// the bytes do not read as a PE image.
pub(crate) fn parse_sections(header: &[u8]) -> Option<Vec<PeSection>> {
    // IMAGE_DOS_HEADER.e_magic == "MZ".
    if read_u16(header, 0)? != 0x5A4D {
        return None;
    }
    let nt = read_u32(header, 0x3C)? as usize;
    // IMAGE_NT_HEADERS.Signature == "PE\0\0".
    if read_u32(header, nt)? != 0x0000_4550 {
        return None;
    }
    // IMAGE_FILE_HEADER: NumberOfSections at +2, SizeOfOptionalHeader at +16,
    // relative to the signature's end.
    let file_header = nt.checked_add(4)?;
    let number_of_sections = read_u16(header, file_header.checked_add(2)?)?;
    let optional_header_size = usize::from(read_u16(header, file_header.checked_add(16)?)?);
    if number_of_sections == 0 || number_of_sections > MAX_SECTIONS {
        return None;
    }
    let mut at = file_header
        .checked_add(20)?
        .checked_add(optional_header_size)?;
    let mut sections = Vec::with_capacity(usize::from(number_of_sections));
    for _ in 0..number_of_sections {
        let entry: &[u8; SECTION_HEADER_LEN] = header.get(at..).and_then(<[u8]>::first_chunk)?;
        let name = String::from_utf8_lossy(before_nul(entry.get(..8)?)).into_owned();
        let characteristics = read_u32(entry, 36)?;
        sections.push(PeSection {
            name,
            virtual_size: read_u32(entry, 8)?,
            rva: read_u32(entry, 12)?,
            raw_size: read_u32(entry, 16)?,
            readable: characteristics & SCN_MEM_READ != 0,
            writable: characteristics & SCN_MEM_WRITE != 0,
            executable: characteristics & SCN_MEM_EXECUTE != 0,
            code: characteristics & SCN_CNT_CODE != 0,
        });
        at = at.checked_add(SECTION_HEADER_LEN)?;
    }
    Some(sections)
}

/// `IMAGE_OPTIONAL_HEADER.Magic` for PE32 and PE32+.
const PE32_MAGIC: u16 = 0x10B;
const PE32_PLUS_MAGIC: u16 = 0x20B;
/// `DataDirectory[IMAGE_DIRECTORY_ENTRY_EXPORT]` inside each optional header.
const PE32_EXPORT_DIRECTORY: usize = 0x60;
const PE32_PLUS_EXPORT_DIRECTORY: usize = 0x70;
/// One `IMAGE_EXPORT_DIRECTORY`.
const EXPORT_DIRECTORY_LEN: usize = 40;
/// The most functions an export table is read for: more than any image
/// exports (ntoskrnl.exe has a few thousand), so a count past it is a
/// corrupt table.
const MAX_EXPORTS: u32 = 1 << 14;
/// The longest export or forwarder name read.
const MAX_EXPORT_NAME: usize = 256;
const PAGE_BYTES: u32 = 0x1000;

/// Where an export leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExportTarget {
    /// Code or data in the image, at this RVA.
    Rva(u32),
    /// Another module's export, named `MODULE.Name`: the function's RVA
    /// points inside the export table, at that string.
    Forwarded(String),
}

/// One exported function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeExport {
    /// Its name, when the table names it; an export by ordinal only has none.
    pub(crate) name: Option<String>,
    pub(crate) ordinal: u32,
    pub(crate) target: ExportTarget,
}

/// What an image's export table came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExportRead {
    /// In address order; forwarders after the rest.
    Listed(Vec<PeExport>),
    /// The image has no export table.
    NoTable,
    /// The headers do not read as a PE image.
    NotPe,
    /// The table, or one of its arrays, is paged out or corrupt.
    Unreadable,
}

/// The offset of the optional header in an image whose headers begin at
/// `header`, when they read as PE.
fn optional_header(header: &[u8]) -> Option<usize> {
    if read_u16(header, 0)? != 0x5A4D {
        return None;
    }
    let nt = read_u32(header, 0x3C)? as usize;
    if read_u32(header, nt)? != 0x0000_4550 {
        return None;
    }
    // The optional header follows the 4-byte signature and the 20-byte file
    // header.
    nt.checked_add(24)
}

/// The little-endian `u32`s of `bytes`, four bytes each; a trailing part
/// shorter than four is not one.
fn u32s(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .filter_map(|chunk| <[u8; 4]>::try_from(chunk).ok())
        .map(u32::from_le_bytes)
        .collect()
}

/// The exports of the image whose headers begin at `header`. `read(rva, len)`
/// reads `len` bytes of the mapped image at `rva`, or `None` where they do
/// not read. The layout is the PE specification's `IMAGE_EXPORT_DIRECTORY`:
/// Base (+16), NumberOfFunctions (+20), NumberOfNames (+24), and the
/// AddressOfFunctions (+28), AddressOfNames (+32) and AddressOfNameOrdinals
/// (+36) arrays; a function whose RVA lies inside the table is a forwarder.
pub(crate) fn parse_exports<R>(header: &[u8], mut read: R) -> ExportRead
where
    R: FnMut(u32, usize) -> Option<Vec<u8>>,
{
    let Some(optional) = optional_header(header) else {
        return ExportRead::NotPe;
    };
    let directory = match read_u16(header, optional) {
        Some(PE32_PLUS_MAGIC) => PE32_PLUS_EXPORT_DIRECTORY,
        Some(PE32_MAGIC) => PE32_EXPORT_DIRECTORY,
        Some(_) | None => return ExportRead::NotPe,
    };
    let Some(directory_at) = optional.checked_add(directory) else {
        return ExportRead::NotPe;
    };
    let (Some(table_rva), Some(table_size)) = (
        read_u32(header, directory_at),
        directory_at
            .checked_add(4)
            .and_then(|at| read_u32(header, at)),
    ) else {
        return ExportRead::NotPe;
    };
    if table_rva == 0 || table_size == 0 {
        return ExportRead::NoTable;
    }
    let table = table_rva..table_rva.saturating_add(table_size);
    let Some(directory) = read(table_rva, EXPORT_DIRECTORY_LEN) else {
        return ExportRead::Unreadable;
    };
    let field = |at: usize| read_u32(&directory, at).unwrap_or(0);
    let (base, functions, names) = (field(16), field(20), field(24));
    if functions > MAX_EXPORTS || names > MAX_EXPORTS {
        return ExportRead::Unreadable;
    }
    // Both counts are at most MAX_EXPORTS, so the byte lengths cannot wrap.
    let (functions, names) = (functions as usize, names as usize);
    let Some(function_rvas) = read(field(28), functions.wrapping_mul(4)).map(|bytes| u32s(&bytes))
    else {
        return ExportRead::Unreadable;
    };
    let Some(name_rvas) = read(field(32), names.wrapping_mul(4)).map(|bytes| u32s(&bytes)) else {
        return ExportRead::Unreadable;
    };
    let Some(name_ordinals) = read(field(36), names.wrapping_mul(2)) else {
        return ExportRead::Unreadable;
    };
    // Each name names the function its ordinal index points at; the first
    // name for a function is kept.
    let mut name_of: Vec<Option<String>> = vec![None; function_rvas.len()];
    for (name_rva, index) in name_rvas.iter().zip(
        name_ordinals
            .chunks_exact(2)
            .filter_map(|chunk| <[u8; 2]>::try_from(chunk).ok())
            .map(u16::from_le_bytes),
    ) {
        if let Some(slot) = name_of.get_mut(usize::from(index)) {
            if slot.is_none() {
                *slot = read_c_string(&mut read, *name_rva);
            }
        }
    }
    let mut exports: Vec<PeExport> = Vec::new();
    for (index, (&rva, name)) in function_rvas.iter().zip(name_of).enumerate() {
        // An unused slot in the ordinal range.
        if rva == 0 {
            continue;
        }
        let target = if table.contains(&rva) {
            ExportTarget::Forwarded(read_c_string(&mut read, rva).unwrap_or_default())
        } else {
            ExportTarget::Rva(rva)
        };
        exports.push(PeExport {
            name,
            ordinal: base.saturating_add(index as u32),
            target,
        });
    }
    exports.sort_by(|a, b| match (&a.target, &b.target) {
        (ExportTarget::Rva(x), ExportTarget::Rva(y)) => x.cmp(y),
        (ExportTarget::Rva(_), ExportTarget::Forwarded(_)) => core::cmp::Ordering::Less,
        (ExportTarget::Forwarded(_), ExportTarget::Rva(_)) => core::cmp::Ordering::Greater,
        (ExportTarget::Forwarded(_), ExportTarget::Forwarded(_)) => a.ordinal.cmp(&b.ordinal),
    });
    ExportRead::Listed(exports)
}

/// `DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT]` inside each optional header.
const PE32_IMPORT_DIRECTORY: usize = 0x68;
const PE32_PLUS_IMPORT_DIRECTORY: usize = 0x78;
/// One `IMAGE_IMPORT_DESCRIPTOR`.
const IMPORT_DESCRIPTOR_LEN: u32 = 20;
/// The most modules and functions an import table is read for: past what
/// any image imports, so more is a corrupt table.
const MAX_IMPORT_MODULES: u32 = 1024;
const MAX_IMPORTS: usize = 1 << 14;

/// How an imported function is named in its module's name table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImportName {
    /// `IMAGE_IMPORT_BY_NAME`: the loader's hint into the exporter's name
    /// table, and the name.
    Named { hint: u16, name: String },
    /// By ordinal: the name-table entry has its top bit set.
    Ordinal(u16),
    /// The module has no name table apart from its IAT, which the loader
    /// overwrote with addresses: the name is gone from the loaded image.
    Overwritten,
    /// The `IMAGE_IMPORT_BY_NAME` it points at is paged out.
    PagedOut,
}

/// One imported function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeImport {
    pub(crate) name: ImportName,
    /// The RVA of its IAT slot.
    pub(crate) slot: u32,
    /// What the loaded slot holds: the address the import was bound to, or
    /// `None` where the slot does not read.
    pub(crate) bound: Option<u64>,
}

/// Where a module's list of imported functions stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThunksEnd {
    /// At the table's terminating zero entry.
    Terminated,
    /// At an entry that is paged out, or that lies past the image's end: the
    /// functions after it are unknown.
    PagedOut,
}

/// The functions an image imports from one module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportedModule {
    /// The module's name, as the descriptor names it; `None` where that
    /// string is paged out.
    pub(crate) name: Option<String>,
    pub(crate) functions: Vec<PeImport>,
    pub(crate) end: ThunksEnd,
}

/// What an image's import table came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImportRead {
    /// In the table's order.
    Listed(Vec<ImportedModule>),
    /// The image has no import table.
    NoTable,
    /// The headers do not read as a PE image.
    NotPe,
    /// The descriptor array is paged out or corrupt.
    Unreadable,
}

/// The imports of the image whose headers begin at `header`, read through
/// `read(rva, len)` as `parse_exports` reads. The layout is the PE
/// specification's `IMAGE_IMPORT_DESCRIPTOR` array, ended by a zero
/// descriptor: OriginalFirstThunk (+0), Name (+12) and FirstThunk (+16). The
/// name table (OriginalFirstThunk, or FirstThunk when there is none) holds
/// pointer-sized entries, the top bit marking an ordinal; the IAT
/// (FirstThunk) holds, once loaded, the address each import was bound to.
pub(crate) fn parse_imports<R>(header: &[u8], mut read: R) -> ImportRead
where
    R: FnMut(u32, usize) -> Option<Vec<u8>>,
{
    let Some(optional) = optional_header(header) else {
        return ImportRead::NotPe;
    };
    let (directory, format) = match read_u16(header, optional) {
        Some(PE32_PLUS_MAGIC) => (PE32_PLUS_IMPORT_DIRECTORY, THUNK64),
        Some(PE32_MAGIC) => (PE32_IMPORT_DIRECTORY, THUNK32),
        Some(_) | None => return ImportRead::NotPe,
    };
    let thunk = format.width;
    let ordinal_flag = format.ordinal_flag;
    let Some(directory_at) = optional.checked_add(directory) else {
        return ImportRead::NotPe;
    };
    let Some(table_rva) = read_u32(header, directory_at) else {
        return ImportRead::NotPe;
    };
    let table_size = directory_at
        .checked_add(4)
        .and_then(|at| read_u32(header, at))
        .unwrap_or(0);
    if table_rva == 0 || table_size == 0 {
        return ImportRead::NoTable;
    }
    // A thunk array is read no further than the image: what lies past it is
    // not the image's. A header that gives no size bounds nothing.
    let image_end = optional
        .checked_add(SIZE_OF_IMAGE)
        .and_then(|at| read_u32(header, at))
        .filter(|&size| size != 0)
        .unwrap_or(u32::MAX);
    let mut modules = Vec::new();
    let mut functions_read = 0usize;
    for index in 0..MAX_IMPORT_MODULES {
        // `index` stays below MAX_IMPORT_MODULES, so the product cannot wrap.
        let at = table_rva.wrapping_add(index.wrapping_mul(IMPORT_DESCRIPTOR_LEN));
        let Some(descriptor) = read(at, IMPORT_DESCRIPTOR_LEN as usize) else {
            return ImportRead::Unreadable;
        };
        let field = |at: usize| read_u32(&descriptor, at).unwrap_or(0);
        let (names_rva, name_rva, iat_rva) = (field(0), field(12), field(16));
        if descriptor.iter().all(|&byte| byte == 0) {
            return ImportRead::Listed(modules);
        }
        // Without OriginalFirstThunk the IAT is the name table, and the
        // loader has written addresses over it.
        let overwritten = names_rva == 0;
        let names_rva = if overwritten { iat_rva } else { names_rva };
        let mut names = ThunkPages::new(thunk, image_end);
        let mut iat = ThunkPages::new(thunk, image_end);
        let mut functions = Vec::new();
        let mut end = ThunksEnd::PagedOut;
        for n in 0u32.. {
            if functions_read >= MAX_IMPORTS {
                return ImportRead::Unreadable;
            }
            let offset = n.wrapping_mul(thunk);
            let Some(entry) = names.entry(&mut read, names_rva.wrapping_add(offset)) else {
                break;
            };
            if entry == 0 {
                end = ThunksEnd::Terminated;
                break;
            }
            let slot = iat_rva.wrapping_add(offset);
            let name = if overwritten {
                ImportName::Overwritten
            } else if entry & ordinal_flag != 0 {
                ImportName::Ordinal(entry as u16)
            } else {
                read_by_name(&mut read, entry as u32)
            };
            let bound = if overwritten {
                Some(entry)
            } else {
                iat.entry(&mut read, slot)
            };
            functions.push(PeImport { name, slot, bound });
            functions_read = functions_read.saturating_add(1);
        }
        modules.push(ImportedModule {
            name: read_c_string(&mut read, name_rva),
            functions,
            end,
        });
    }
    ImportRead::Unreadable
}

/// `DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION]` inside each optional
/// header.
const PE32_EXCEPTION_DIRECTORY: usize = 0x78;
const PE32_PLUS_EXCEPTION_DIRECTORY: usize = 0x88;

/// One `IMAGE_DATA_DIRECTORY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DataDirectory {
    pub(crate) rva: u32,
    pub(crate) size: u32,
}

/// The exception directory (x64: the `RUNTIME_FUNCTION` table) of the image
/// whose headers begin at `header`; `None` when it has none or the headers
/// do not read as PE.
pub(crate) fn exception_directory(header: &[u8]) -> Option<DataDirectory> {
    let optional = optional_header(header)?;
    let entry = match read_u16(header, optional)? {
        PE32_PLUS_MAGIC => PE32_PLUS_EXCEPTION_DIRECTORY,
        PE32_MAGIC => PE32_EXCEPTION_DIRECTORY,
        _ => return None,
    };
    let at = optional.checked_add(entry)?;
    let directory = DataDirectory { rva: read_u32(header, at)?, size: read_u32(header, at.checked_add(4)?)? };
    (directory.rva != 0 && directory.size != 0).then_some(directory)
}

/// `IMAGE_OPTIONAL_HEADER.SizeOfImage`, at one offset in PE32 and PE32+.
const SIZE_OF_IMAGE: usize = 0x38;

/// A name-table entry's shape: its width in bytes, and the bit that marks an
/// import by ordinal.
#[derive(Clone, Copy)]
struct ThunkFormat {
    width: u32,
    ordinal_flag: u64,
}

/// `IMAGE_THUNK_DATA32` with `IMAGE_ORDINAL_FLAG32`.
const THUNK32: ThunkFormat = ThunkFormat {
    width: 4,
    ordinal_flag: 1 << 31,
};
/// `IMAGE_THUNK_DATA64` with `IMAGE_ORDINAL_FLAG64`.
const THUNK64: ThunkFormat = ThunkFormat {
    width: 8,
    ordinal_flag: 1 << 63,
};

/// A thunk array's entries, `width` bytes each, read a page at a time and
/// never past `image_end`.
struct ThunkPages {
    width: u32,
    image_end: u32,
    /// The bytes last read, from RVA `at`.
    at: u32,
    bytes: Vec<u8>,
}

impl ThunkPages {
    fn new(width: u32, image_end: u32) -> Self {
        Self {
            width,
            image_end,
            at: 0,
            bytes: Vec::new(),
        }
    }

    /// The entry at `rva`, or `None` where it lies past the image or does
    /// not read. A read brings in the rest of the entry's page.
    fn entry<R>(&mut self, read: &mut R, rva: u32) -> Option<u64>
    where
        R: FnMut(u32, usize) -> Option<Vec<u8>>,
    {
        let end = rva.checked_add(self.width).filter(|&end| end <= self.image_end)?;
        let held = rva >= self.at
            && u64::from(end) <= u64::from(self.at).saturating_add(self.bytes.len() as u64);
        if !held {
            // `rva` fits in 32 bits, so the next page boundary fits in 64.
            let page_end = (u64::from(rva) | u64::from(PAGE_BYTES - 1)).wrapping_add(1);
            // An entry that crosses a page boundary is read whole. Both
            // `page_end` and `end` lie past `rva`, and `end <= image_end`, so
            // the fill length cannot wrap.
            let fill_end = page_end.max(u64::from(end)).min(u64::from(self.image_end));
            self.bytes = read(rva, fill_end.wrapping_sub(u64::from(rva)) as usize)?;
            self.at = rva;
        }
        // Held bytes start at or before `rva`, so the offset cannot wrap.
        let from = rva.wrapping_sub(self.at) as usize;
        let entry = self.bytes.get(from..)?.get(..self.width as usize)?;
        let mut raw = [0u8; 8];
        for (to, &byte) in raw.iter_mut().zip(entry) {
            *to = byte;
        }
        Some(u64::from_le_bytes(raw))
    }
}

/// The `IMAGE_IMPORT_BY_NAME` at `rva` — a 2-byte hint, then the name — read
/// with its page in one read; a name that runs on into the next page is read
/// on from there.
fn read_by_name<R>(read: &mut R, rva: u32) -> ImportName
where
    R: FnMut(u32, usize) -> Option<Vec<u8>>,
{
    let in_page = bytes_left_in_page(rva);
    // A hint split across a page boundary is read whole.
    let take = in_page.clamp(2, 2 + MAX_EXPORT_NAME);
    let Some(chunk) = read(rva, take) else {
        return ImportName::PagedOut;
    };
    let Some((hint, text)) = chunk.split_first_chunk::<2>() else {
        return ImportName::PagedOut;
    };
    let hint = u16::from_le_bytes(*hint);
    let name = match text.iter().position(|&byte| byte == 0).and_then(|end| text.get(..end)) {
        Some(name) => String::from_utf8_lossy(name).into_owned(),
        None => {
            let Some(rest) = rva
                .checked_add(take as u32)
                .and_then(|next| read_c_string(read, next))
            else {
                return ImportName::PagedOut;
            };
            format!("{}{rest}", String::from_utf8_lossy(text))
        }
    };
    ImportName::Named { hint, name }
}

/// The NUL-terminated ASCII string at `rva`, up to `MAX_EXPORT_NAME` bytes,
/// read a page at a time so a short name near a page's end reads even when
/// the next page does not.
fn read_c_string<R>(read: &mut R, rva: u32) -> Option<String>
where
    R: FnMut(u32, usize) -> Option<Vec<u8>>,
{
    let mut bytes = Vec::new();
    let mut at = rva;
    while bytes.len() < MAX_EXPORT_NAME {
        let take = bytes_left_in_page(at).min(MAX_EXPORT_NAME.saturating_sub(bytes.len()));
        let chunk = read(at, take)?;
        match chunk.iter().position(|&b| b == 0).and_then(|end| chunk.get(..end)) {
            Some(text) => {
                bytes.extend_from_slice(text);
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            None => bytes.extend_from_slice(&chunk),
        }
        at = at.checked_add(take as u32)?;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// How many bytes from `rva` to the end of its page: 1 through `PAGE_BYTES`.
fn bytes_left_in_page(rva: u32) -> usize {
    PAGE_BYTES.wrapping_sub(rva & (PAGE_BYTES - 1)) as usize
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// A minimal PE whose optional-header size is honoured, not assumed: two
    /// sections, read-execute code and read-write data, each with its flags.
    fn image() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x200];
        put_u16(&mut bytes, 0, 0x5A4D); // MZ
        let nt = 0x80usize;
        put_u32(&mut bytes, 0x3C, nt as u32); // e_lfanew
        put_u32(&mut bytes, nt, 0x0000_4550); // PE\0\0
        put_u16(&mut bytes, nt + 4 + 2, 2); // NumberOfSections
        let optional = 0x10usize;
        put_u16(&mut bytes, nt + 4 + 16, optional as u16); // SizeOfOptionalHeader
        let mut at = nt + 4 + 20 + optional;
        // .text: vsize 0x8C00, rva 0x1000, raw 0x8C00, CODE|EXECUTE|READ.
        bytes[at..at + 5].copy_from_slice(b".text");
        put_u32(&mut bytes, at + 8, 0x8C00);
        put_u32(&mut bytes, at + 12, 0x1000);
        put_u32(&mut bytes, at + 16, 0x8C00);
        put_u32(&mut bytes, at + 36, SCN_CNT_CODE | SCN_MEM_EXECUTE | SCN_MEM_READ);
        at += SECTION_HEADER_LEN;
        // .data: vsize 0x2A00, rva 0xA000, raw 0x2000, READ|WRITE.
        bytes[at..at + 5].copy_from_slice(b".data");
        put_u32(&mut bytes, at + 8, 0x2A00);
        put_u32(&mut bytes, at + 12, 0xA000);
        put_u32(&mut bytes, at + 16, 0x2000);
        put_u32(&mut bytes, at + 36, SCN_MEM_READ | SCN_MEM_WRITE);
        bytes
    }

    /// Headers of a PE32 (`wide` false) or PE32+ image whose exception
    /// directory is `directory`.
    fn with_exception_directory(wide: bool, directory: DataDirectory) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x200];
        put_u16(&mut bytes, 0, 0x5A4D);
        put_u32(&mut bytes, 0x3C, 0x80);
        put_u32(&mut bytes, 0x80, 0x0000_4550);
        let optional = 0x98;
        put_u16(&mut bytes, optional, if wide { PE32_PLUS_MAGIC } else { PE32_MAGIC });
        let entry = optional + if wide { 0x88 } else { 0x78 };
        put_u32(&mut bytes, entry, directory.rva);
        put_u32(&mut bytes, entry + 4, directory.size);
        bytes
    }

    #[test]
    fn the_exception_directory_is_read_from_either_header() {
        let directory = DataDirectory { rva: 0x5000, size: 0x30 };
        for wide in [false, true] {
            assert_eq!(exception_directory(&with_exception_directory(wide, directory)), Some(directory), "wide {wide}");
        }
        let empty = DataDirectory { rva: 0, size: 0 };
        assert_eq!(exception_directory(&with_exception_directory(true, empty)), None);
        assert_eq!(exception_directory(b"MZ"), None);
    }

    #[test]
    fn sections_are_read_with_their_access() {
        let sections = parse_sections(&image()).expect("a PE image");
        assert_eq!(
            sections,
            [
                PeSection {
                    name: ".text".to_owned(),
                    rva: 0x1000,
                    virtual_size: 0x8C00,
                    raw_size: 0x8C00,
                    readable: true,
                    writable: false,
                    executable: true,
                    code: true,
                },
                PeSection {
                    name: ".data".to_owned(),
                    rva: 0xA000,
                    virtual_size: 0x2A00,
                    raw_size: 0x2000,
                    readable: true,
                    writable: true,
                    executable: false,
                    code: false,
                },
            ]
        );
    }

    /// A PE32+ image laid out flat (each RVA its own offset) with an export
    /// table at RVA 0x2000: `IoCreateDevice` by name, one export by ordinal
    /// only, and `KeFwd` forwarded to `HAL.HalFoo`.
    fn exporting_image() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x3000];
        put_u16(&mut bytes, 0, 0x5A4D);
        let nt = 0x80usize;
        put_u32(&mut bytes, 0x3C, nt as u32);
        put_u32(&mut bytes, nt, 0x0000_4550);
        put_u16(&mut bytes, nt + 4 + 16, 0xF0); // SizeOfOptionalHeader (PE32+)
        let optional = nt + 4 + 20;
        put_u16(&mut bytes, optional, 0x20B); // PE32+ magic
        put_u32(&mut bytes, optional + 0x70, 0x2000); // DataDirectory[EXPORT].VirtualAddress
        put_u32(&mut bytes, optional + 0x74, 0x200); // .Size
        let dir = 0x2000usize;
        put_u32(&mut bytes, dir + 16, 1); // Base: ordinals start at 1
        put_u32(&mut bytes, dir + 20, 3); // NumberOfFunctions
        put_u32(&mut bytes, dir + 24, 2); // NumberOfNames
        put_u32(&mut bytes, dir + 28, 0x2028); // AddressOfFunctions
        put_u32(&mut bytes, dir + 32, 0x2034); // AddressOfNames
        put_u32(&mut bytes, dir + 36, 0x203C); // AddressOfNameOrdinals
        put_u32(&mut bytes, 0x2028, 0x1100); // [0] IoCreateDevice
        put_u32(&mut bytes, 0x202C, 0x1200); // [1] by ordinal only
        put_u32(&mut bytes, 0x2030, 0x2050); // [2] inside the table: a forwarder
        put_u32(&mut bytes, 0x2034, 0x2060); // name 0
        put_u32(&mut bytes, 0x2038, 0x2070); // name 1
        put_u16(&mut bytes, 0x203C, 0); // name 0 → function 0
        put_u16(&mut bytes, 0x203E, 2); // name 1 → function 2
        bytes[0x2050..0x205B].copy_from_slice(b"HAL.HalFoo\0");
        bytes[0x2060..0x206F].copy_from_slice(b"IoCreateDevice\0");
        bytes[0x2070..0x2076].copy_from_slice(b"KeFwd\0");
        bytes
    }

    fn reader(image: &[u8]) -> impl FnMut(u32, usize) -> Option<Vec<u8>> + '_ {
        move |rva, len| image.get(rva as usize..rva as usize + len).map(<[u8]>::to_vec)
    }

    #[test]
    fn exports_are_read_with_their_names_ordinals_and_forwarders() {
        let image = exporting_image();
        let read = parse_exports(&image[..HEADER_BYTES], reader(&image));
        assert_eq!(
            read,
            ExportRead::Listed(vec![
                PeExport {
                    name: Some("IoCreateDevice".to_owned()),
                    ordinal: 1,
                    target: ExportTarget::Rva(0x1100),
                },
                PeExport {
                    name: None,
                    ordinal: 2,
                    target: ExportTarget::Rva(0x1200),
                },
                PeExport {
                    name: Some("KeFwd".to_owned()),
                    ordinal: 3,
                    target: ExportTarget::Forwarded("HAL.HalFoo".to_owned()),
                },
            ])
        );
    }

    #[test]
    fn an_image_without_an_export_table_lists_none() {
        let mut image = exporting_image();
        put_u32(&mut image, 0x80 + 4 + 20 + 0x70, 0);
        assert_eq!(parse_exports(&image[..HEADER_BYTES], reader(&image)), ExportRead::NoTable);
    }

    #[test]
    fn an_export_table_that_does_not_read_says_so() {
        let image = exporting_image();
        let header_only = |rva: u32, len: usize| -> Option<Vec<u8>> {
            (rva < 0x2000).then(|| image[rva as usize..rva as usize + len].to_vec())
        };
        assert_eq!(parse_exports(&image[..HEADER_BYTES], header_only), ExportRead::Unreadable);
    }

    #[test]
    fn bytes_that_are_not_a_pe_list_no_exports() {
        assert_eq!(parse_exports(&[0u8; 0x200], |_, _| None), ExportRead::NotPe);
    }

    /// A flat image (each RVA its own offset) with an import table at RVA
    /// 0x2400: ntdll.dll's `NtClose` by name and ordinal 16, and OLD.dll,
    /// whose name table is its IAT, so the loader overwrote its names.
    /// `wide` lays it out as PE32+ (8-byte thunks), else as PE32.
    pub(crate) fn importing_image(wide: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x3000];
        put_u16(&mut bytes, 0, 0x5A4D);
        let nt = 0x80usize;
        put_u32(&mut bytes, 0x3C, nt as u32);
        put_u32(&mut bytes, nt, 0x0000_4550);
        let optional = nt + 4 + 20;
        let (magic, directory, thunk, ordinal_flag) = if wide {
            (0x20B, 0x78, 8, 1u64 << 63)
        } else {
            (0x10B, 0x68, 4, 1u64 << 31)
        };
        put_u16(&mut bytes, optional, magic);
        put_u32(&mut bytes, optional + 0x38, 0x3000); // SizeOfImage
        put_u32(&mut bytes, optional + directory, 0x2400); // DataDirectory[IMPORT]
        put_u32(&mut bytes, optional + directory + 4, 0x3C);
        let put_thunk = |bytes: &mut Vec<u8>, at: usize, value: u64| {
            bytes[at..at + thunk].copy_from_slice(&value.to_le_bytes()[..thunk]);
        };
        // ntdll.dll: OriginalFirstThunk, Name, FirstThunk.
        put_u32(&mut bytes, 0x2400, 0x2500);
        put_u32(&mut bytes, 0x2400 + 12, 0x2600);
        put_u32(&mut bytes, 0x2400 + 16, 0x2700);
        put_thunk(&mut bytes, 0x2500, 0x2800);
        put_thunk(&mut bytes, 0x2500 + thunk, ordinal_flag | 16);
        put_thunk(&mut bytes, 0x2700, 0x7FF8_1380);
        put_thunk(&mut bytes, 0x2700 + thunk, 0x7FF8_2000);
        put_u16(&mut bytes, 0x2800, 5); // hint
        bytes[0x2802..0x280A].copy_from_slice(b"NtClose\0");
        bytes[0x2600..0x260A].copy_from_slice(b"ntdll.dll\0");
        // OLD.dll: no OriginalFirstThunk.
        put_u32(&mut bytes, 0x2414 + 12, 0x2610);
        put_u32(&mut bytes, 0x2414 + 16, 0x2720);
        put_thunk(&mut bytes, 0x2720, 0x7FF9_0000);
        bytes[0x2610..0x2618].copy_from_slice(b"OLD.dll\0");
        bytes
    }

    fn imported_by_both() -> Vec<ImportedModule> {
        vec![
            ImportedModule {
                name: Some("ntdll.dll".to_owned()),
                functions: vec![
                    PeImport {
                        name: ImportName::Named {
                            hint: 5,
                            name: "NtClose".to_owned(),
                        },
                        slot: 0x2700,
                        bound: Some(0x7FF8_1380),
                    },
                    PeImport {
                        name: ImportName::Ordinal(16),
                        slot: 0,
                        bound: Some(0x7FF8_2000),
                    },
                ],
                end: ThunksEnd::Terminated,
            },
            ImportedModule {
                name: Some("OLD.dll".to_owned()),
                functions: vec![PeImport {
                    name: ImportName::Overwritten,
                    slot: 0x2720,
                    bound: Some(0x7FF9_0000),
                }],
                end: ThunksEnd::Terminated,
            },
        ]
    }

    #[test]
    fn imports_are_read_by_name_and_ordinal_with_where_each_slot_points() {
        for (wide, thunk) in [(true, 8), (false, 4)] {
            let image = importing_image(wide);
            let mut expected = imported_by_both();
            expected[0].functions[1].slot = 0x2700 + thunk;
            assert_eq!(
                parse_imports(&image[..HEADER_BYTES], reader(&image)),
                ImportRead::Listed(expected),
                "PE32+ is {wide}"
            );
        }
    }

    #[test]
    fn an_image_without_an_import_table_lists_none() {
        let mut image = importing_image(true);
        put_u32(&mut image, 0x80 + 4 + 20 + 0x78, 0);
        assert_eq!(parse_imports(&image[..HEADER_BYTES], reader(&image)), ImportRead::NoTable);
    }

    #[test]
    fn an_import_table_that_does_not_read_says_so() {
        let image = importing_image(true);
        let header_only = |rva: u32, len: usize| -> Option<Vec<u8>> {
            (rva < 0x2400).then(|| image[rva as usize..rva as usize + len].to_vec())
        };
        assert_eq!(parse_imports(&image[..HEADER_BYTES], header_only), ImportRead::Unreadable);
        assert_eq!(parse_imports(&[0u8; 0x200], |_, _| None), ImportRead::NotPe);
    }

    /// The name tables and the IAT are read a page at a time, and a name
    /// with its hint in one read: the three descriptors, the two module names,
    /// ntdll's name table, its IAT and its one name, and OLD.dll's IAT.
    #[test]
    fn an_import_table_is_read_a_page_at_a_time() {
        let image = importing_image(true);
        let mut reads = 0usize;
        let counted = |rva: u32, len: usize| {
            reads += 1;
            image.get(rva as usize..rva as usize + len).map(<[u8]>::to_vec)
        };
        let read = parse_imports(&image[..HEADER_BYTES], counted);
        assert!(matches!(read, ImportRead::Listed(_)), "{read:?}");
        assert_eq!(reads, 9);
    }

    /// A name table that runs on past the image's end is read only up to
    /// it: what lies beyond is not the image's.
    #[test]
    fn a_name_table_is_read_no_further_than_the_image() {
        let mut image = importing_image(true);
        put_u32(&mut image, 0x80 + 4 + 20 + 0x38, 0x2508); // SizeOfImage
        let ImportRead::Listed(modules) = parse_imports(&image[..HEADER_BYTES], reader(&image)) else {
            panic!("the table reads");
        };
        assert_eq!(modules[0].functions.len(), 1);
        assert_eq!(modules[0].end, ThunksEnd::PagedOut);
    }

    /// A name table that runs into a page that is paged out keeps what read
    /// before it, and says the rest is paged out.
    #[test]
    fn a_name_table_paged_out_partway_keeps_what_read() {
        let mut image = importing_image(true);
        image.resize(0x4000, 0);
        put_u32(&mut image, 0x80 + 4 + 20 + 0x38, 0x4000); // SizeOfImage
        // ntdll's name table ends its page with NtClose; the page after it,
        // where its next entry lies, is paged out.
        put_u32(&mut image, 0x2400, 0x2FF8);
        image[0x2FF8..0x3000].copy_from_slice(&0x2800u64.to_le_bytes());
        let short = |rva: u32, len: usize| -> Option<Vec<u8>> {
            (rva as usize + len <= 0x3000).then(|| image[rva as usize..rva as usize + len].to_vec())
        };
        let ImportRead::Listed(modules) = parse_imports(&image[..HEADER_BYTES], short) else {
            panic!("the table reads");
        };
        assert_eq!(modules[0].functions.len(), 1);
        assert_eq!(modules[0].end, ThunksEnd::PagedOut);
        assert_eq!(modules[1].end, ThunksEnd::Terminated);
    }

    #[test]
    fn bytes_that_are_not_a_pe_read_as_none() {
        assert_eq!(parse_sections(&[0u8; 0x200]), None);
        assert_eq!(parse_sections(b"MZ"), None);
        // MZ with e_lfanew pointing past the blob.
        let mut truncated = vec![0u8; 0x40];
        put_u16(&mut truncated, 0, 0x5A4D);
        put_u32(&mut truncated, 0x3C, 0x1000);
        assert_eq!(parse_sections(&truncated), None);
    }

    /// Every corruption of the test images — each truncation, each field set
    /// to 0, its maximum or one past itself, seeded bit flips — parses to an
    /// answer or a refusal; none reads out of range.
    #[test]
    fn the_header_parsers_survive_every_corruption_of_their_images() {
        use rusty_box_symbols_fixtures::mutate::{variants, Budget};
        struct Parsed {
            sections: usize,
            exports: usize,
            imports: usize,
        }
        let mut parsed = Parsed { sections: 0, exports: 0, imports: 0 };
        for original in [image(), importing_image(false), importing_image(true)] {
            for mutation in variants(original.len(), Budget::STANDARD) {
                let bytes = mutation.apply(&original);
                let read = |rva: u32, len: usize| {
                    let start = usize::try_from(rva).ok()?;
                    bytes.get(start..start.checked_add(len)?).map(<[u8]>::to_vec)
                };
                if parse_sections(&bytes).is_some() {
                    parsed.sections += 1;
                }
                if !matches!(parse_exports(&bytes, read), ExportRead::NotPe) {
                    parsed.exports += 1;
                }
                if !matches!(parse_imports(&bytes, read), ImportRead::NotPe) {
                    parsed.imports += 1;
                }
            }
        }
        // Most variants leave the headers whole: each parser still answers.
        assert!(parsed.sections > 0 && parsed.exports > 0 && parsed.imports > 0);
    }
}
