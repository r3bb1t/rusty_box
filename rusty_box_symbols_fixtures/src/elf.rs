//! ELF test files: an executable (`ET_EXEC` linked at 0x400000, or a PIE
//! linked at 0; 32- or 64-bit) with a GNU build-id note, a `.text` of two
//! functions, an optional `.symtab`, and any extra sections a test passes
//! in, plain or compressed as the gABI or GNU's `.zdebug` convention does.

use object::elf;
use object::write::elf::{FileHeader, ProgramHeader, SectionHeader, Sym, Writer};
use object::write::StringId;
use object::{Endianness, Object, ObjectSegment};

use crate::compress::{zlib, zstd_frame, Checksum, ZstdOptions};

/// gABI `ELFCOMPRESS_ZLIB` and `ELFCOMPRESS_ZSTD`, and `SHF_COMPRESSED`.
const ELFCOMPRESS_ZLIB: u32 = 1;
const ELFCOMPRESS_ZSTD: u32 = 2;
const SHF_COMPRESSED: u64 = 0x800;

pub const BUILD_ID: [u8; 20] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x10, 0x32, 0x54, 0x76, 0x98, 0xBA, 0xDC, 0xFE,
    0xA5, 0x5A, 0xC3, 0x3C,
];
/// `.text` sits this far past the link base (file offset = RVA: the one
/// `PT_LOAD` maps the file from offset 0).
pub const TEXT_RVA: u64 = 0x1000;
/// `widget_init` occupies `.text[0x00..0x10]`.
pub const FUNC_LEN: u64 = 0x10;
/// `helper_no_dwarf` occupies `.text[0x10..0x20]`; only `.symtab` names it.
pub const HELPER_LEN: u64 = 0x10;
pub const TEXT_LEN: u64 = FUNC_LEN + HELPER_LEN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Elf32,
    Elf64,
}

#[derive(Clone, Copy, Debug)]
pub enum Symtab {
    With,
    Without,
}

/// `.text` with its bytes, or as `SHT_NOBITS`: the shape of
/// `objcopy --only-keep-debug`.
#[derive(Clone, Copy, Debug)]
pub enum Text {
    Bits,
    NoBits,
}

#[derive(Clone, Copy, Debug)]
pub struct ElfSpec {
    pub class: Class,
    pub e_type: elf::FileType,
    pub link_base: u64,
    pub symtab: Symtab,
    pub text: Text,
    /// The GNU build-id note's descriptor.
    pub build_id: [u8; 20],
}

impl ElfSpec {
    /// A 64-bit PIE executable or shared object, linked at 0.
    pub fn pie() -> Self {
        ElfSpec {
            class: Class::Elf64,
            e_type: elf::ET_DYN,
            link_base: 0,
            symtab: Symtab::With,
            text: Text::Bits,
            build_id: BUILD_ID,
        }
    }

    /// A classic 64-bit executable, linked at 0x400000.
    pub fn exec() -> Self {
        ElfSpec { e_type: elf::ET_EXEC, link_base: 0x40_0000, ..ElfSpec::pie() }
    }

    /// A 32-bit PIE, linked at 0.
    pub fn pie32() -> Self {
        ElfSpec { class: Class::Elf32, ..ElfSpec::pie() }
    }
}

/// How an extra section's bytes are stored.
#[derive(Clone, Copy, Debug)]
pub enum Encoding {
    Plain,
    /// `SHF_COMPRESSED` with `ELFCOMPRESS_ZLIB`.
    Zlib,
    /// `SHF_COMPRESSED` with `ELFCOMPRESS_ZSTD`.
    Zstd,
    /// GNU's older convention: `.zdebug_*`, `ZLIB`, a big-endian size.
    Zdebug,
}

/// A section the test adds: its name as a reader asks for it (`.debug_info`),
/// its uncompressed bytes, and how they are stored.
pub struct ExtraSection {
    pub name: String,
    pub data: Vec<u8>,
    pub encoding: Encoding,
}

/// A section as written: its stored name, bytes and flags.
struct Stored {
    name: Vec<u8>,
    bytes: Vec<u8>,
    flags: u64,
}

fn stored(class: Class, section: &ExtraSection) -> Stored {
    let header = |kind: u32| -> Vec<u8> {
        let size = section.data.len() as u64;
        let mut header = Vec::new();
        header.extend_from_slice(&kind.to_le_bytes());
        match class {
            Class::Elf64 => {
                header.extend_from_slice(&0u32.to_le_bytes());
                header.extend_from_slice(&size.to_le_bytes());
                header.extend_from_slice(&1u64.to_le_bytes());
            }
            Class::Elf32 => {
                header.extend_from_slice(&(size as u32).to_le_bytes());
                header.extend_from_slice(&1u32.to_le_bytes());
            }
        }
        header
    };
    match section.encoding {
        Encoding::Plain => Stored { name: section.name.clone().into_bytes(), bytes: section.data.clone(), flags: 0 },
        Encoding::Zlib => {
            let mut bytes = header(ELFCOMPRESS_ZLIB);
            bytes.extend_from_slice(&zlib(&section.data, 6));
            Stored { name: section.name.clone().into_bytes(), bytes, flags: SHF_COMPRESSED }
        }
        Encoding::Zstd => {
            let mut bytes = header(ELFCOMPRESS_ZSTD);
            let options = ZstdOptions { level: 3, checksum: Checksum::Without, window_log: 0 };
            bytes.extend_from_slice(&zstd_frame(&section.data, options));
            Stored { name: section.name.clone().into_bytes(), bytes, flags: SHF_COMPRESSED }
        }
        Encoding::Zdebug => {
            let mut bytes = b"ZLIB".to_vec();
            bytes.extend_from_slice(&(section.data.len() as u64).to_be_bytes());
            bytes.extend_from_slice(&zlib(&section.data, 6));
            let name = section.name.replacen(".debug", ".zdebug", 1).into_bytes();
            Stored { name, bytes, flags: 0 }
        }
    }
}

/// The GNU build-id note: `Elf_Nhdr` (4-byte fields on both classes),
/// `"GNU\0"`, then the id, padded to 4 bytes.
pub fn build_id_note(id: &[u8]) -> Vec<u8> {
    let mut note = Vec::new();
    note.extend_from_slice(&4u32.to_le_bytes());
    note.extend_from_slice(&(id.len() as u32).to_le_bytes());
    note.extend_from_slice(&elf::NT_GNU_BUILD_ID.0.to_le_bytes());
    note.extend_from_slice(b"GNU\0");
    note.extend_from_slice(id);
    while note.len() % 4 != 0 {
        note.push(0);
    }
    note
}

/// A placed extra section.
struct Placed<'a> {
    name: StringId,
    offset: u64,
    stored: &'a Stored,
}

/// A `.symtab` function symbol.
struct FixtureSymbol {
    name: StringId,
    value: u64,
    size: u64,
}

/// The whole file: ELF header, two program headers (`PT_LOAD` over
/// `[0, TEXT_RVA + TEXT_LEN)` and `PT_NOTE`), the note, `.text` at file
/// offset `TEXT_RVA`, the extra sections, `.symtab`, `.strtab`,
/// `.shstrtab`, then the section headers.
pub fn build_elf(spec: ElfSpec, extra: &[ExtraSection]) -> Vec<u8> {
    let is_64 = spec.class == Class::Elf64;
    let text_va = spec.link_base + TEXT_RVA;
    let note = build_id_note(&spec.build_id);
    let mut text = vec![0x90u8; TEXT_LEN as usize];
    text[FUNC_LEN as usize - 1] = 0xC3;
    text[TEXT_LEN as usize - 1] = 0xC3;
    let stored_sections: Vec<Stored> = extra.iter().map(|section| stored(spec.class, section)).collect();

    let mut bytes = Vec::new();
    let mut writer = Writer::new(Endianness::Little, is_64, &mut bytes);
    writer.reserve_file_header();
    writer.reserve_program_headers(2);
    let note_offset = writer.reserve(note.len() as u64, 4);
    let text_offset = writer.reserve(TEXT_LEN, TEXT_RVA);

    writer.reserve_null_section_index();
    let note_name = writer.add_section_name(b".note.gnu.build-id");
    writer.reserve_section_index();
    let text_name = writer.add_section_name(b".text");
    let text_index = writer.reserve_section_index();
    let mut placed = Vec::new();
    for section in &stored_sections {
        let name = writer.add_section_name(&section.name);
        writer.reserve_section_index();
        let offset = writer.reserve(section.bytes.len() as u64, 8);
        placed.push(Placed { name, offset, stored: section });
    }
    let symbols = match spec.symtab {
        Symtab::With => {
            let symbols = [
                FixtureSymbol { name: writer.add_string(b"widget_init"), value: text_va, size: FUNC_LEN },
                FixtureSymbol { name: writer.add_string(b"helper_no_dwarf"), value: text_va + FUNC_LEN, size: HELPER_LEN },
            ];
            writer.reserve_null_symbol_index();
            writer.reserve_symbol_index(Some(text_index));
            writer.reserve_symbol_index(Some(text_index));
            writer.reserve_symtab_section_index();
            writer.reserve_symtab();
            writer.reserve_strtab_section_index();
            writer.reserve_strtab().expect("strtab");
            Some(symbols)
        }
        Symtab::Without => None,
    };
    writer.reserve_shstrtab_section_index();
    writer.reserve_shstrtab().expect("shstrtab");
    writer.reserve_section_headers();

    writer
        .write_file_header(&FileHeader {
            os_abi: elf::ELFOSABI_SYSV,
            abi_version: 0,
            e_type: spec.e_type,
            e_machine: if is_64 { elf::EM_X86_64 } else { elf::EM_386 },
            e_entry: text_va,
            e_flags: elf::FileFlags(0),
        })
        .expect("file header");
    writer.write_align_program_headers();
    writer.write_program_header(&ProgramHeader {
        p_type: elf::PT_LOAD,
        p_flags: elf::PF_R | elf::PF_X,
        p_offset: 0,
        p_vaddr: spec.link_base,
        p_paddr: spec.link_base,
        p_filesz: TEXT_RVA + TEXT_LEN,
        p_memsz: TEXT_RVA + TEXT_LEN,
        p_align: 0x1000,
    });
    writer.write_program_header(&ProgramHeader {
        p_type: elf::PT_NOTE,
        p_flags: elf::PF_R,
        p_offset: note_offset,
        p_vaddr: spec.link_base + note_offset,
        p_paddr: spec.link_base + note_offset,
        p_filesz: note.len() as u64,
        p_memsz: note.len() as u64,
        p_align: 4,
    });
    writer.pad_until(note_offset);
    writer.write(&note);
    writer.pad_until(text_offset);
    writer.write(&text);
    for section in &placed {
        writer.pad_until(section.offset);
        writer.write(&section.stored.bytes);
    }
    if let Some(symbols) = &symbols {
        writer.write_null_symbol();
        for symbol in symbols {
            let st_name = writer.string_offset(Some(symbol.name));
            writer.write_symbol(&Sym {
                section: Some(text_index.0),
                st_name,
                st_info: elf::STB_GLOBAL | elf::STT_FUNC,
                st_other: elf::SymbolOther::default(),
                st_shndx: elf::SymbolSection::default(),
                st_value: symbol.value,
                st_size: symbol.size,
            });
        }
        writer.write_strtab();
    }
    writer.write_shstrtab();

    writer.write_null_section_header();
    let note_sh_name = writer.section_name_offset(Some(note_name));
    writer.write_section_header(&SectionHeader {
        sh_name: note_sh_name,
        sh_type: elf::SHT_NOTE,
        sh_flags: elf::SHF_ALLOC,
        sh_addr: spec.link_base + note_offset,
        sh_offset: note_offset,
        sh_size: note.len() as u64,
        sh_link: 0,
        sh_info: 0,
        sh_addralign: 4,
        sh_entsize: 0,
    });
    let text_sh_name = writer.section_name_offset(Some(text_name));
    writer.write_section_header(&SectionHeader {
        sh_name: text_sh_name,
        sh_type: match spec.text {
            Text::Bits => elf::SHT_PROGBITS,
            Text::NoBits => elf::SHT_NOBITS,
        },
        sh_flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        sh_addr: text_va,
        sh_offset: text_offset,
        sh_size: TEXT_LEN,
        sh_link: 0,
        sh_info: 0,
        sh_addralign: 16,
        sh_entsize: 0,
    });
    for section in &placed {
        let sh_name = writer.section_name_offset(Some(section.name));
        writer.write_section_header(&SectionHeader {
            sh_name,
            sh_type: elf::SHT_PROGBITS,
            sh_flags: elf::SectionFlags(section.stored.flags),
            sh_addr: 0,
            sh_offset: section.offset,
            sh_size: section.stored.bytes.len() as u64,
            sh_link: 0,
            sh_info: 0,
            sh_addralign: 1,
            sh_entsize: 0,
        });
    }
    if symbols.is_some() {
        // Only the null symbol is local.
        writer.write_symtab_section_header(1);
        writer.write_strtab_section_header();
    }
    writer.write_shstrtab_section_header();
    drop(writer);
    bytes
}

/// What a loader puts in memory: each `PT_LOAD`'s bytes at
/// `p_vaddr - link_base`, `None` where nothing is mapped.
pub fn load_image(file: &[u8], link_base: u64) -> Vec<Option<u8>> {
    let parsed = object::File::parse(file).expect("the fixture parses");
    let mut image: Vec<Option<u8>> = Vec::new();
    for segment in parsed.segments() {
        let rva = (segment.address() - link_base) as usize;
        let memory_size = segment.size() as usize;
        let file_range = segment.file_range();
        let offset = file_range.0 as usize;
        let file_size = file_range.1 as usize;
        if image.len() < rva + memory_size {
            image.resize(rva + memory_size, None);
        }
        for index in 0..memory_size {
            image[rva + index] = Some(if index < file_size { file[offset + index] } else { 0 });
        }
    }
    image
}
