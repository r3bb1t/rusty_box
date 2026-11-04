//! PE images and files laid out by hand from the PE/COFF specification
//! (Microsoft Learn, "PE Format").

/// `IMAGE_DEBUG_TYPE_CODEVIEW`.
pub const IMAGE_DEBUG_TYPE_CODEVIEW: u32 = 2;

/// An RSDS record: GUID bytes as stored, the age, the recorded path.
pub fn rsds(guid: [u8; 16], age: u32, path: &str) -> Vec<u8> {
    let mut record = b"RSDS".to_vec();
    record.extend_from_slice(&guid);
    record.extend_from_slice(&age.to_le_bytes());
    record.extend_from_slice(path.as_bytes());
    record.push(0);
    record
}

pub fn put16(image: &mut [u8], at: usize, value: u16) {
    image[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

pub fn put32(image: &mut [u8], at: usize, value: u32) {
    image[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// A PE32+ image as a guest holds it (RVA = offset): headers, one `.rdata`
/// section at RVA 0x1000 holding a debug directory whose one CodeView entry
/// points at `codeview`, at RVA 0x1100. TimeDateStamp 0x12345678,
/// SizeOfImage 0x2000.
pub fn loaded_image(codeview: &[u8]) -> Vec<u8> {
    let mut image = vec![0u8; 0x2000];
    image[0..2].copy_from_slice(b"MZ");
    put32(&mut image, 0x3C, 0x80);
    let nt = 0x80;
    image[nt..nt + 4].copy_from_slice(b"PE\0\0");
    put16(&mut image, nt + 4, 0x8664);
    put16(&mut image, nt + 6, 1);
    put32(&mut image, nt + 8, 0x1234_5678);
    put16(&mut image, nt + 20, 240);
    let opt = nt + 24;
    put16(&mut image, opt, 0x20B);
    put32(&mut image, opt + 56, 0x2000);
    put32(&mut image, opt + 60, 0x400);
    put32(&mut image, opt + 108, 16);
    put32(&mut image, opt + 112 + 48, 0x1000);
    put32(&mut image, opt + 112 + 52, 28);
    let section = opt + 240;
    image[section..section + 6].copy_from_slice(b".rdata");
    put32(&mut image, section + 8, 0x1000);
    put32(&mut image, section + 12, 0x1000);
    put32(&mut image, section + 16, 0x1000);
    put32(&mut image, section + 20, 0x1000);
    put32(&mut image, 0x1000 + 12, IMAGE_DEBUG_TYPE_CODEVIEW);
    put32(&mut image, 0x1000 + 16, codeview.len() as u32);
    put32(&mut image, 0x1000 + 20, 0x1100);
    put32(&mut image, 0x1000 + 24, 0x1100);
    image[0x1100..0x1100 + codeview.len()].copy_from_slice(codeview);
    image
}

/// `IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ`.
pub const SECTION_CODE: u32 = 0x6000_0020;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ`.
pub const SECTION_READ_ONLY: u32 = 0x4000_0040;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE`.
pub const SECTION_WRITABLE: u32 = 0xC000_0040;

/// `IMAGE_REL_BASED_HIGHLOW`.
pub const REL_BASED_HIGHLOW: u16 = 3;
/// `IMAGE_REL_BASED_DIR64`.
pub const REL_BASED_DIR64: u16 = 10;

/// A data-directory entry of [`pe64_file`].
#[derive(Clone, Copy, Debug)]
pub struct Directory {
    pub rva: u32,
    pub size: u32,
}

/// A section of [`pe64_file`]: its bytes are its raw data, and its virtual
/// size is their length.
#[derive(Clone, Debug)]
pub struct SpecSection {
    pub name: &'static [u8],
    pub rva: u32,
    pub bytes: Vec<u8>,
    pub characteristics: u32,
}

/// What [`pe64_file`] lays out.
#[derive(Clone, Debug)]
pub struct Pe64Spec {
    pub time_date_stamp: u32,
    pub size_of_image: u32,
    pub image_base: u64,
    /// `SectionAlignment`: [`PAGE_ALIGNED`] as linkers write it by default,
    /// or less, as `/ALIGN` makes a driver's.
    pub section_alignment: u32,
    pub sections: Vec<SpecSection>,
    pub exception: Option<Directory>,
    pub base_relocations: Option<Directory>,
    pub import_address_table: Option<Directory>,
}

/// A data directory at its place in the optional header.
struct DirectorySlot {
    index: usize,
    directory: Option<Directory>,
}

/// The headers' size in a [`pe64_file`], and where its first section's raw
/// data starts.
pub const PE64_HEADERS: u32 = 0x400;
const FILE_ALIGNMENT: usize = 0x200;
/// A page: the `SectionAlignment` linkers write by default.
pub const PAGE_ALIGNED: u32 = 0x1000;

/// A PE32+ file as a linker writes it: headers in the first
/// [`PE64_HEADERS`] bytes, then each section's raw data in turn, each
/// padded to 0x200, so no section sits at its RVA.
pub fn pe64_file(spec: &Pe64Spec) -> Vec<u8> {
    let mut file = vec![0u8; PE64_HEADERS as usize];
    file[0..2].copy_from_slice(b"MZ");
    put32(&mut file, 0x3C, 0x80);
    let nt = 0x80;
    file[nt..nt + 4].copy_from_slice(b"PE\0\0");
    put16(&mut file, nt + 4, 0x8664);
    put16(&mut file, nt + 6, spec.sections.len() as u16);
    put32(&mut file, nt + 8, spec.time_date_stamp);
    put16(&mut file, nt + 20, 240);
    let opt = nt + 24;
    put16(&mut file, opt, 0x20B);
    file[opt + 24..opt + 32].copy_from_slice(&spec.image_base.to_le_bytes());
    put32(&mut file, opt + 32, spec.section_alignment);
    put32(&mut file, opt + 56, spec.size_of_image);
    put32(&mut file, opt + 60, PE64_HEADERS);
    put32(&mut file, opt + 108, 16);
    let directories = [
        DirectorySlot { index: 3, directory: spec.exception },
        DirectorySlot { index: 5, directory: spec.base_relocations },
        DirectorySlot { index: 12, directory: spec.import_address_table },
    ];
    for slot in directories {
        if let Some(directory) = slot.directory {
            put32(&mut file, opt + 112 + slot.index * 8, directory.rva);
            put32(&mut file, opt + 112 + slot.index * 8 + 4, directory.size);
        }
    }
    let table = opt + 240;
    for (index, section) in spec.sections.iter().enumerate() {
        let raw = file.len();
        let raw_size = section.bytes.len().div_ceil(FILE_ALIGNMENT) * FILE_ALIGNMENT;
        file.extend_from_slice(&section.bytes);
        file.resize(raw + raw_size, 0);
        let at = table + index * 40;
        file[at..at + section.name.len()].copy_from_slice(section.name);
        put32(&mut file, at + 8, section.bytes.len() as u32);
        put32(&mut file, at + 12, section.rva);
        put32(&mut file, at + 16, raw_size as u32);
        put32(&mut file, at + 20, raw as u32);
        put32(&mut file, at + 36, section.characteristics);
    }
    file
}

/// One base-relocation block: the page's RVA, then each entry as
/// `kind << 12 | offset`, padded to a 4-byte multiple as linkers pad it.
pub fn relocation_block(page_rva: u32, entries: &[u16]) -> Vec<u8> {
    let mut entries = entries.to_vec();
    if entries.len() % 2 == 1 {
        entries.push(0);
    }
    let mut block = page_rva.to_le_bytes().to_vec();
    block.extend_from_slice(&((8 + entries.len() * 2) as u32).to_le_bytes());
    for entry in entries {
        block.extend_from_slice(&entry.to_le_bytes());
    }
    block
}

/// A section of [`pe32_file`]: its name, RVA and file offset.
struct FileSection {
    name: &'static [u8],
    rva: u32,
    raw: u32,
}

/// A PE32 file whose sections do not sit at their RVAs: `.text` at RVA
/// 0x1000 from file offset 0x400, `.rdata` at RVA 0x2000 from file offset
/// 0x600, the debug directory at RVA 0x2000 and the CodeView record at RVA
/// 0x2100 (file offset 0x700). TimeDateStamp 0x0BADF00D, SizeOfImage 0x3000.
pub fn pe32_file(codeview: &[u8]) -> Vec<u8> {
    let mut file = vec![0u8; 0x800];
    file[0..2].copy_from_slice(b"MZ");
    put32(&mut file, 0x3C, 0x80);
    let nt = 0x80;
    file[nt..nt + 4].copy_from_slice(b"PE\0\0");
    put16(&mut file, nt + 4, 0x014C);
    put16(&mut file, nt + 6, 2);
    put32(&mut file, nt + 8, 0x0BAD_F00D);
    put16(&mut file, nt + 20, 224);
    let opt = nt + 24;
    put16(&mut file, opt, 0x10B);
    put32(&mut file, opt + 56, 0x3000);
    put32(&mut file, opt + 60, 0x400);
    put32(&mut file, opt + 92, 16);
    put32(&mut file, opt + 96 + 48, 0x2000);
    put32(&mut file, opt + 96 + 52, 28);
    let table = opt + 224;
    let sections = [
        FileSection { name: b".text", rva: 0x1000, raw: 0x400 },
        FileSection { name: b".rdata", rva: 0x2000, raw: 0x600 },
    ];
    for (index, section) in sections.iter().enumerate() {
        let at = table + index * 40;
        file[at..at + section.name.len()].copy_from_slice(section.name);
        put32(&mut file, at + 8, 0x1000);
        put32(&mut file, at + 12, section.rva);
        put32(&mut file, at + 16, 0x200);
        put32(&mut file, at + 20, section.raw);
    }
    put32(&mut file, 0x600 + 12, IMAGE_DEBUG_TYPE_CODEVIEW);
    put32(&mut file, 0x600 + 16, codeview.len() as u32);
    put32(&mut file, 0x600 + 20, 0x2100);
    put32(&mut file, 0x600 + 24, 0x700);
    file[0x700..0x700 + codeview.len()].copy_from_slice(codeview);
    file
}
