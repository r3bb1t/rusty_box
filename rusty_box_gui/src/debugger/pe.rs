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
