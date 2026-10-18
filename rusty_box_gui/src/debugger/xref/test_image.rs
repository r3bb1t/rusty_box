//! PE images built byte by byte for the cross-reference tests, laid out flat
//! (each RVA its own offset), as a loader maps them: headers in the first
//! page, `.text` and `.data` after, an export table and a `RUNTIME_FUNCTION`
//! table inside `.data`.

use crate::debugger::pe::{self, ImageWidth, HEADER_BYTES};
use crate::debugger::xref::image::{ImageOffset, ModuleImage, PageRead, PAGE_BYTES};

/// A `ModuleImage` at `base` as `copy_image` would give it for `spec`, with
/// `text` at `.text`'s start, `data` at `.data`'s free bytes, every page
/// read, and `sync` as the offsets known to start an instruction.
pub(crate) fn module_image(base: u64, spec: &ImageSpec, text: &[u8], data: &[u8], sync: &[u64]) -> ModuleImage {
    let mut bytes = build_image(spec);
    let text_at = spec.text.rva as usize;
    bytes[text_at..text_at + text.len()].copy_from_slice(text);
    let data_at = spec.data_free() as usize;
    bytes[data_at..data_at + data.len()].copy_from_slice(data);
    let sections = pe::parse_sections(&bytes[..HEADER_BYTES]).expect("the builder writes a PE");
    let pages = vec![PageRead::Read; bytes.len().div_ceil(PAGE_BYTES as usize)];
    ModuleImage {
        base,
        width: spec.width,
        bytes,
        pages,
        sections,
        sync: sync.iter().copied().map(ImageOffset).collect(),
    }
}

/// `IMAGE_SCN_CNT_CODE | _MEM_EXECUTE | _MEM_READ` and `_MEM_READ | _MEM_WRITE`.
const CODE: u32 = 0x6000_0020;
const DATA: u32 = 0xC000_0000;
const NT: usize = 0x80;
/// The export directory's and the function table's offsets into `.data`.
const EXPORTS_AT: u32 = 0x0;
const FUNCTIONS_AT: u32 = 0x800;
/// Where `.data`'s free bytes start, past both tables.
pub(crate) const DATA_FREE: u32 = 0x1000;

pub(crate) struct Section {
    pub(crate) rva: u32,
    pub(crate) size: u32,
}

/// One `RUNTIME_FUNCTION`: `begin..end`, no unwind data.
pub(crate) struct Function {
    pub(crate) begin: u32,
    pub(crate) end: u32,
}

pub(crate) struct ImageSpec {
    pub(crate) width: ImageWidth,
    pub(crate) text: Section,
    pub(crate) data: Section,
    pub(crate) entry: u32,
    /// Exported RVAs, in ordinal order.
    pub(crate) exports: Vec<u32>,
    /// Written only into a 64-bit image.
    pub(crate) functions: Vec<Function>,
}

impl ImageSpec {
    /// A 64-bit image: `.text` at 0x1000 of `text` bytes, `.data` after it.
    pub(crate) fn bits64(text: u32) -> ImageSpec {
        let data = 0x1000 + text.next_multiple_of(0x1000);
        ImageSpec {
            width: ImageWidth::Bits64,
            text: Section { rva: 0x1000, size: text },
            data: Section { rva: data, size: 0x4000 },
            entry: 0x1000,
            exports: Vec::new(),
            functions: Vec::new(),
        }
    }

    /// Where `.data` gives free bytes.
    pub(crate) fn data_free(&self) -> u32 {
        self.data.rva + DATA_FREE
    }

    /// The image's size: up to `.data`'s end.
    pub(crate) fn size(&self) -> u32 {
        self.data.rva + self.data.size
    }
}

fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// The image `spec` describes, `spec.size()` bytes, its sections zero.
pub(crate) fn build_image(spec: &ImageSpec) -> Vec<u8> {
    let mut bytes = vec![0u8; spec.size() as usize];
    put_u16(&mut bytes, 0, 0x5A4D);
    put_u32(&mut bytes, 0x3C, NT as u32);
    put_u32(&mut bytes, NT, 0x0000_4550);
    put_u16(&mut bytes, NT + 4 + 2, 2);
    let wide = spec.width == ImageWidth::Bits64;
    let optional_size: u16 = if wide { 0xF0 } else { 0xE0 };
    put_u16(&mut bytes, NT + 4 + 16, optional_size);
    let optional = NT + 4 + 20;
    put_u16(&mut bytes, optional, if wide { 0x20B } else { 0x10B });
    put_u32(&mut bytes, optional + 16, spec.entry);
    put_u32(&mut bytes, optional + 0x38, spec.size());
    let directories = optional + if wide { 0x70 } else { 0x60 };
    let exports = spec.data.rva + EXPORTS_AT;
    if !spec.exports.is_empty() {
        put_u32(&mut bytes, directories, exports);
        put_u32(&mut bytes, directories + 4, 0x100);
        let directory = exports as usize;
        put_u32(&mut bytes, directory + 16, 1);
        put_u32(&mut bytes, directory + 20, spec.exports.len() as u32);
        put_u32(&mut bytes, directory + 28, exports + 0x40);
        for (index, rva) in spec.exports.iter().enumerate() {
            put_u32(&mut bytes, directory + 0x40 + 4 * index, *rva);
        }
    }
    if wide && !spec.functions.is_empty() {
        let table = spec.data.rva + FUNCTIONS_AT;
        // DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION], the fourth entry.
        put_u32(&mut bytes, directories + 3 * 8, table);
        put_u32(&mut bytes, directories + 3 * 8 + 4, 12 * spec.functions.len() as u32);
        for (index, function) in spec.functions.iter().enumerate() {
            let at = table as usize + 12 * index;
            put_u32(&mut bytes, at, function.begin);
            put_u32(&mut bytes, at + 4, function.end);
        }
    }
    let mut at = optional + usize::from(optional_size);
    let headers = [
        SectionHeader { name: b".text", section: &spec.text, characteristics: CODE },
        SectionHeader { name: b".data", section: &spec.data, characteristics: DATA },
    ];
    for header in headers {
        bytes[at..at + 5].copy_from_slice(header.name);
        put_u32(&mut bytes, at + 8, header.section.size);
        put_u32(&mut bytes, at + 12, header.section.rva);
        put_u32(&mut bytes, at + 16, header.section.size);
        put_u32(&mut bytes, at + 36, header.characteristics);
        at += 40;
    }
    bytes
}

/// One `IMAGE_SECTION_HEADER` as the builder writes it.
struct SectionHeader<'a> {
    name: &'a [u8; 5],
    section: &'a Section,
    characteristics: u32,
}
