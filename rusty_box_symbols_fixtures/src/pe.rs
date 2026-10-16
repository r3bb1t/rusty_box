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
