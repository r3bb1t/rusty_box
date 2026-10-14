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
    bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
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
    let file_header = nt + 4;
    let number_of_sections = read_u16(header, file_header + 2)?;
    let optional_header_size = read_u16(header, file_header + 16)? as usize;
    if number_of_sections == 0 || number_of_sections > MAX_SECTIONS {
        return None;
    }
    let mut at = file_header + 20 + optional_header_size;
    let mut sections = Vec::with_capacity(number_of_sections as usize);
    for _ in 0..number_of_sections {
        let entry = header.get(at..at + SECTION_HEADER_LEN)?;
        let end = entry[..8].iter().position(|&b| b == 0).unwrap_or(8);
        let name = String::from_utf8_lossy(&entry[..end]).into_owned();
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
        at += SECTION_HEADER_LEN;
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
    Some(nt + 4 + 20)
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
    let directory_at = match read_u16(header, optional) {
        Some(PE32_PLUS_MAGIC) => optional + PE32_PLUS_EXPORT_DIRECTORY,
        Some(PE32_MAGIC) => optional + PE32_EXPORT_DIRECTORY,
        Some(_) | None => return ExportRead::NotPe,
    };
    let (Some(table_rva), Some(table_size)) =
        (read_u32(header, directory_at), read_u32(header, directory_at + 4))
    else {
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
    let u32s = |bytes: Vec<u8>| -> Vec<u32> {
        bytes
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    };
    let Some(function_rvas) = read(field(28), functions as usize * 4).map(u32s) else {
        return ExportRead::Unreadable;
    };
    let Some(name_rvas) = read(field(32), names as usize * 4).map(u32s) else {
        return ExportRead::Unreadable;
    };
    let Some(name_ordinals) = read(field(36), names as usize * 2) else {
        return ExportRead::Unreadable;
    };
    // Each name names the function its ordinal index points at; the first
    // name for a function is kept.
    let mut name_of: Vec<Option<String>> = vec![None; function_rvas.len()];
    for (name_rva, index) in name_rvas
        .iter()
        .zip(name_ordinals.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])))
    {
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
        let in_page = (PAGE_BYTES - at % PAGE_BYTES) as usize;
        let take = in_page.min(MAX_EXPORT_NAME - bytes.len());
        let chunk = read(at, take)?;
        match chunk.iter().position(|&b| b == 0) {
            Some(end) => {
                bytes.extend_from_slice(&chunk[..end]);
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            None => bytes.extend_from_slice(&chunk),
        }
        at = at.checked_add(take as u32)?;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
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
}
