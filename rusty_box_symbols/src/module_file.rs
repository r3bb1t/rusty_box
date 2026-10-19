//! A module's binary file read as the image the loader made of it, for the
//! bytes of a loaded module the guest has not paged in. Only what the loader
//! maps unchanged from the file is served: the headers and the sections it
//! maps read-only, with the base relocations applied for the base the
//! module is loaded at. A writable section, and the import address table the
//! loader fills, hold the guest's own values, which no file has.

use crate::identity::PeImageId;
use crate::pe::{DataDirectory, FileImage, HeaderError, ImageRead, PeHeaders};
use crate::read::Bytes;
use crate::units::Rva;

/// `IMAGE_SCN_MEM_WRITE`.
const SECTION_WRITABLE: u32 = 0x8000_0000;
/// `IMAGE_REL_BASED_ABSOLUTE`: padding, no fixup.
const REL_BASED_ABSOLUTE: u16 = 0;
/// `IMAGE_REL_BASED_HIGHLOW`.
const REL_BASED_HIGHLOW: u16 = 3;
/// `IMAGE_REL_BASED_HIGHADJ`: its entry is followed by one more slot.
const REL_BASED_HIGHADJ: u16 = 4;
/// `IMAGE_REL_BASED_DIR64`.
const REL_BASED_DIR64: u16 = 10;
/// The widest fixup: how far before a read a fixup it covers can start.
const WIDEST_FIXUP: u64 = 8;
/// A relocation directory past this is not a real image's (the largest
/// system images keep theirs to a few hundred KiB).
const MAX_RELOCATION_BYTES: u32 = 64 << 20;

/// What one base relocation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FixupKind {
    /// A 32-bit address.
    HighLow,
    /// A 64-bit address.
    Dir64,
    /// A kind this reader does not apply: the bytes it may cover are not
    /// served at another base.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fixup {
    rva: u64,
    kind: FixupKind,
}

impl FixupKind {
    /// The bytes it changes; for a kind not applied, the most any changes.
    const fn width(self) -> u64 {
        match self {
            FixupKind::HighLow => 4,
            FixupKind::Dir64 | FixupKind::Other => WIDEST_FIXUP,
        }
    }
}

/// The image's base relocations.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Relocations {
    /// Sorted by RVA.
    Fixups(Vec<Fixup>),
    /// The image has none: it loads only at its linked base.
    Absent,
    /// The directory does not read or parse: no byte is served at another
    /// base.
    Unreadable,
}

/// A PE file, read as its image loaded at some base.
pub struct ModuleFile {
    file: FileImage<Vec<u8>>,
    relocations: Relocations,
}

impl ModuleFile {
    /// Parses the file's headers and its base relocations.
    pub fn parse(bytes: Vec<u8>) -> Result<ModuleFile, HeaderError> {
        let file = FileImage::new(bytes)?;
        let relocations = match file.headers().base_relocations {
            Some(directory) => read_relocations(&file, directory),
            None => Relocations::Absent,
        };
        Ok(ModuleFile { file, relocations })
    }

    #[must_use]
    pub fn image_id(&self) -> PeImageId {
        self.file.headers().image_id()
    }

    #[must_use]
    pub fn headers(&self) -> &PeHeaders {
        self.file.headers()
    }
    /// `buf.len()` bytes at `rva` of the image loaded at `base`, when every
    /// one of them is a byte the loader maps unchanged from the file (base
    /// relocations aside).
    pub fn read_loaded(&self, base: u64, rva: Rva, buf: &mut [u8]) -> ImageRead {
        let Some(end) = u64::try_from(buf.len()).ok().and_then(|len| rva.0.checked_add(len)) else {
            return ImageRead::Unreadable;
        };
        let headers = self.headers();
        if end > u64::from(headers.size_of_image) {
            return ImageRead::Unreadable;
        }
        if headers.import_address_table.is_some_and(|table| overlaps(table, rva.0, end)) {
            return ImageRead::Unreadable;
        }
        let mut cursor = rva.0;
        let mut filled = 0usize;
        while cursor < end {
            let Some(region) = self.region_of(cursor) else {
                return ImageRead::Unreadable;
            };
            // `region_of` gives a region ending past `cursor`.
            let take = region.end.min(end).saturating_sub(cursor);
            let from_file = take.min(region.file_end.saturating_sub(cursor));
            let Ok(take_len) = usize::try_from(take) else {
                return ImageRead::Unreadable;
            };
            let Ok(file_len) = usize::try_from(from_file) else {
                return ImageRead::Unreadable;
            };
            let Some(piece) = filled.checked_add(take_len).and_then(|piece_end| buf.get_mut(filled..piece_end)) else {
                return ImageRead::Unreadable;
            };
            let Some((file_part, zero_part)) = piece.split_at_mut_checked(file_len) else {
                return ImageRead::Unreadable;
            };
            if !file_part.is_empty() {
                match self.file.read_into(Rva(cursor), file_part) {
                    ImageRead::Read => {}
                    ImageRead::Unreadable => return ImageRead::Unreadable,
                }
            }
            zero_part.fill(0);
            filled = filled.saturating_add(take_len);
            cursor = cursor.saturating_add(take);
        }
        let delta = base.wrapping_sub(headers.image_base);
        if delta == 0 {
            return ImageRead::Read;
        }
        match &self.relocations {
            Relocations::Fixups(fixups) => self.relocate(fixups, delta, rva.0, buf),
            Relocations::Absent | Relocations::Unreadable => ImageRead::Unreadable,
        }
    }

    /// The region the loader maps unchanged that holds `rva`: the headers'
    /// page, or the pages of a section it maps read-only. Each is the file's
    /// bytes up to `file_end` and zeros from there to the end of its last
    /// page; `None` for a writable section's bytes and for bytes no region
    /// holds.
    fn region_of(&self, rva: u64) -> Option<Region> {
        let headers = self.headers();
        let size_of_headers = u64::from(headers.size_of_headers);
        let headers_end = page_end(size_of_headers);
        if rva < headers_end {
            return Some(Region { file_end: size_of_headers, end: headers_end });
        }
        let section = headers
            .sections
            .iter()
            .find(|section| rva.checked_sub(section.virtual_address.0).is_some_and(|within| within < page_end(section.span())))?;
        if section.characteristics & SECTION_WRITABLE != 0 {
            return None;
        }
        let start = section.virtual_address.0;
        Some(Region { file_end: start.saturating_add(section.span()), end: start.saturating_add(page_end(section.span())) })
    }

    /// Applies, to `buf` read at `start`, every fixup whose bytes it covers,
    /// for the image moved by `delta`.
    fn relocate(&self, fixups: &[Fixup], delta: u64, start: u64, buf: &mut [u8]) -> ImageRead {
        let Some(end) = u64::try_from(buf.len()).ok().and_then(|len| start.checked_add(len)) else {
            return ImageRead::Unreadable;
        };
        let first = fixups.partition_point(|fixup| fixup.rva.saturating_add(WIDEST_FIXUP) <= start);
        for fixup in fixups.iter().skip(first).take_while(|fixup| fixup.rva < end) {
            let width = fixup.kind.width();
            if fixup.rva.saturating_add(width) <= start {
                continue;
            }
            let mut moved = [0u8; 8];
            let Some(raw) = usize::try_from(width).ok().and_then(|width| moved.get_mut(..width)) else {
                return ImageRead::Unreadable;
            };
            match self.file.read_into(Rva(fixup.rva), raw) {
                ImageRead::Read => {}
                ImageRead::Unreadable => return ImageRead::Unreadable,
            }
            match fixup.kind {
                FixupKind::HighLow => {
                    let Some(linked) = raw.first_chunk::<4>().copied() else {
                        return ImageRead::Unreadable;
                    };
                    // A 32-bit fixup adds the delta's low half.
                    let value = u32::from_le_bytes(linked).wrapping_add(delta as u32);
                    raw.copy_from_slice(&value.to_le_bytes());
                }
                FixupKind::Dir64 => {
                    let Some(linked) = raw.first_chunk::<8>().copied() else {
                        return ImageRead::Unreadable;
                    };
                    raw.copy_from_slice(&u64::from_le_bytes(linked).wrapping_add(delta).to_le_bytes());
                }
                FixupKind::Other => return ImageRead::Unreadable,
            }
            for (offset, byte) in (0u64..).zip(raw.iter()) {
                let at = fixup.rva.saturating_add(offset);
                let slot = at
                    .checked_sub(start)
                    .filter(|_| at < end)
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| buf.get_mut(index));
                if let Some(slot) = slot {
                    *slot = *byte;
                }
            }
        }
        ImageRead::Read
    }
}

/// A stretch of the image the loader maps from the file: its bytes up to
/// `file_end`, zeros from there to `end`.
#[derive(Clone, Copy, Debug)]
struct Region {
    file_end: u64,
    end: u64,
}

/// The loader maps whole pages.
const PAGE_BYTES: u64 = 0x1000;

/// `size` rounded up to whole pages.
fn page_end(size: u64) -> u64 {
    size.saturating_add(PAGE_BYTES - 1) & !(PAGE_BYTES - 1)
}

/// Whether `[start, end)` and `directory`'s bytes share one.
fn overlaps(directory: DataDirectory, start: u64, end: u64) -> bool {
    let table_end = directory.rva.0.saturating_add(u64::from(directory.size));
    start < table_end && directory.rva.0 < end
}

/// The fixups of the relocation directory, sorted by RVA: blocks of a page
/// RVA, the block's size, and 16-bit entries of a kind (high four bits) and
/// an offset in the page (PE Format, "The .reloc Section").
fn read_relocations(file: &FileImage<Vec<u8>>, directory: DataDirectory) -> Relocations {
    if directory.size > MAX_RELOCATION_BYTES {
        return Relocations::Unreadable;
    }
    let Ok(size) = usize::try_from(directory.size) else {
        return Relocations::Unreadable;
    };
    let mut bytes = vec![0u8; size];
    match file.read_into(directory.rva, &mut bytes) {
        ImageRead::Read => {}
        ImageRead::Unreadable => return Relocations::Unreadable,
    }
    let table = Bytes::new(&bytes);
    let mut fixups = Vec::new();
    let mut at = 0usize;
    while at < size {
        let Ok(page) = table.u32_le(at) else {
            return Relocations::Unreadable;
        };
        let Ok(block_size) = table.u32_le(at.saturating_add(4)) else {
            return Relocations::Unreadable;
        };
        let Some(block_end) = usize::try_from(block_size).ok().and_then(|block| at.checked_add(block)) else {
            return Relocations::Unreadable;
        };
        if block_size < 8 || block_end > size {
            return Relocations::Unreadable;
        }
        let mut entry_at = at.saturating_add(8);
        while entry_at.saturating_add(2) <= block_end {
            let Ok(entry) = table.u16_le(entry_at) else {
                return Relocations::Unreadable;
            };
            entry_at = entry_at.saturating_add(2);
            let rva = u64::from(page).saturating_add(u64::from(entry & 0x0FFF));
            let kind = match entry >> 12 {
                REL_BASED_ABSOLUTE => continue,
                REL_BASED_HIGHLOW => FixupKind::HighLow,
                REL_BASED_DIR64 => FixupKind::Dir64,
                REL_BASED_HIGHADJ => {
                    entry_at = entry_at.saturating_add(2);
                    FixupKind::Other
                }
                _ => FixupKind::Other,
            };
            fixups.push(Fixup { rva, kind });
        }
        at = block_end;
    }
    fixups.sort_by_key(|fixup| fixup.rva);
    Relocations::Fixups(fixups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::pe::{
        pe64_file, relocation_block, Directory, Pe64Spec, SpecSection, PE64_HEADERS, REL_BASED_DIR64, REL_BASED_HIGHLOW,
        SECTION_CODE, SECTION_READ_ONLY, SECTION_WRITABLE,
    };

    const LINKED: u64 = 0x1_4000_0000;
    const MOVED: u64 = 0x7FF_1230_0000;
    /// `.rdata`'s pointer to `.data`, as linked.
    const POINTER: u64 = LINKED + 0x3010;

    /// `.text` at 0x1000 (a `ret` run), `.rdata` at 0x2000 holding the
    /// import address table at +0x20 and a 64-bit pointer at +0x40, `.data`
    /// (writable) at 0x3000, and `.reloc` at 0x4000 with one DIR64 fixup for
    /// the pointer, one HIGHLOW at `.text`+0x10 and `extra` fixups more.
    fn spec(extra: &[u16]) -> Pe64Spec {
        let mut rdata = vec![0x11u8; 0x100];
        rdata[0x40..0x48].copy_from_slice(&POINTER.to_le_bytes());
        let mut text = vec![0xC3u8; 0x80];
        text[0x10..0x14].copy_from_slice(&0x4000_3020u32.to_le_bytes());
        let mut text_entries = vec![REL_BASED_HIGHLOW << 12 | 0x10];
        text_entries.extend_from_slice(extra);
        let mut relocations = relocation_block(0x2000, &[REL_BASED_DIR64 << 12 | 0x40]);
        relocations.extend(relocation_block(0x1000, &text_entries));
        let size = relocations.len() as u32;
        Pe64Spec {
            time_date_stamp: 0x5E1F_0001,
            size_of_image: 0x5000,
            image_base: LINKED,
            sections: vec![
                SpecSection { name: b".text", rva: 0x1000, bytes: text, characteristics: SECTION_CODE },
                SpecSection { name: b".rdata", rva: 0x2000, bytes: rdata, characteristics: SECTION_READ_ONLY },
                SpecSection { name: b".data", rva: 0x3000, bytes: vec![0x22; 0x40], characteristics: SECTION_WRITABLE },
                SpecSection { name: b".reloc", rva: 0x4000, bytes: relocations, characteristics: SECTION_READ_ONLY },
            ],
            exception: None,
            base_relocations: Some(Directory { rva: 0x4000, size }),
            import_address_table: Some(Directory { rva: 0x2020, size: 0x10 }),
        }
    }

    fn module(extra: &[u16]) -> ModuleFile {
        ModuleFile::parse(pe64_file(&spec(extra))).unwrap()
    }

    fn read(file: &ModuleFile, base: u64, rva: u64, len: usize) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; len];
        match file.read_loaded(base, Rva(rva), &mut buf) {
            ImageRead::Read => Some(buf),
            ImageRead::Unreadable => None,
        }
    }

    #[test]
    fn code_and_read_only_data_read_as_the_loader_maps_them() {
        let file = module(&[]);
        assert_eq!(read(&file, LINKED, 0x1000, 4), Some(vec![0xC3; 4]));
        assert_eq!(read(&file, LINKED, 0x2000, 0x20), Some(vec![0x11; 0x20]));
        assert_eq!(read(&file, LINKED, 0x20F0, 0x10), Some(vec![0x11; 0x10]));
        assert_eq!(read(&file, LINKED, 0, 2), Some(b"MZ".to_vec()), "the headers are mapped");
    }

    #[test]
    fn the_rest_of_the_header_page_and_of_a_sections_last_page_reads_as_zeros() {
        let file = module(&[]);
        let page = read(&file, LINKED, 0, 0x1000).expect("the whole header page");
        assert_eq!(&page[..2], b"MZ");
        assert!(page[PE64_HEADERS as usize..].iter().all(|byte| *byte == 0), "past SizeOfHeaders");
        let text = read(&file, LINKED, 0x1000, 0x1000).expect("the whole page of .text");
        assert_eq!(&text[..4], &[0xC3; 4]);
        assert!(text[0x200..].iter().all(|byte| *byte == 0), "past the section's raw data");
        assert_eq!(read(&file, LINKED, 0x1FFC, 8), Some(vec![0, 0, 0, 0, 0x11, 0x11, 0x11, 0x11]), "across into .rdata");
        assert_eq!(read(&file, LINKED, 0x2FFC, 8), None, "across into the writable .data");
    }

    #[test]
    fn a_writable_section_holds_the_guests_values_and_is_not_served() {
        let file = module(&[]);
        assert_eq!(read(&file, LINKED, 0x3000, 4), None);
        assert_eq!(read(&file, LINKED, 0x2FFE, 4), None, "a read reaching into it");
    }

    #[test]
    fn the_import_address_table_is_filled_by_the_loader_and_is_not_served() {
        let file = module(&[]);
        assert_eq!(read(&file, LINKED, 0x2020, 8), None);
        assert_eq!(read(&file, LINKED, 0x201C, 8), None, "a read overlapping its start");
        assert_eq!(read(&file, LINKED, 0x2018, 8), Some(vec![0x11; 8]), "the bytes before it");
        assert_eq!(read(&file, LINKED, 0x2030, 8), Some(vec![0x11; 8]), "the bytes after it");
    }

    #[test]
    fn at_another_base_every_fixup_the_read_covers_is_applied() {
        let file = module(&[]);
        let delta = MOVED - LINKED;
        let pointer = read(&file, MOVED, 0x2040, 8).unwrap();
        assert_eq!(u64::from_le_bytes(pointer.try_into().unwrap()), POINTER + delta);
        let narrow = read(&file, MOVED, 0x1010, 4).unwrap();
        assert_eq!(u32::from_le_bytes(narrow.try_into().unwrap()), 0x4000_3020u32.wrapping_add(delta as u32));
        // A read that starts inside a fixup gets the relocated bytes it covers.
        let moved = (POINTER + delta).to_le_bytes();
        assert_eq!(read(&file, MOVED, 0x2044, 6).unwrap(), [&moved[4..8], &[0x11, 0x11][..]].concat());
        assert_eq!(read(&file, MOVED, 0x203E, 4).unwrap(), [&[0x11, 0x11][..], &moved[..2]].concat());
        // At the linked base nothing moves.
        assert_eq!(read(&file, LINKED, 0x2040, 8).unwrap(), POINTER.to_le_bytes());
    }

    #[test]
    fn a_fixup_of_a_kind_not_applied_refuses_the_bytes_it_covers_at_another_base() {
        // IMAGE_REL_BASED_HIGH at .text+0x40.
        let file = module(&[1 << 12 | 0x40]);
        assert_eq!(read(&file, MOVED, 0x1040, 2), None);
        assert_eq!(read(&file, MOVED, 0x1000, 4), Some(vec![0xC3; 4]), "bytes away from it");
        assert_eq!(read(&file, LINKED, 0x1040, 2), Some(vec![0xC3; 2]), "at the linked base it is not applied");
    }

    #[test]
    fn an_image_without_relocations_is_served_only_at_its_linked_base() {
        let mut spec = spec(&[]);
        spec.base_relocations = None;
        let file = ModuleFile::parse(pe64_file(&spec)).unwrap();
        assert_eq!(read(&file, LINKED, 0x1000, 4), Some(vec![0xC3; 4]));
        assert_eq!(read(&file, MOVED, 0x1000, 4), None);
    }

    #[test]
    fn a_read_past_the_image_or_wrapping_is_refused() {
        let file = module(&[]);
        assert_eq!(read(&file, LINKED, 0x5000, 1), None);
        assert_eq!(read(&file, LINKED, u64::MAX, 2), None);
        assert_eq!(read(&file, LINKED, 0x1000, 0), Some(Vec::new()), "nothing asked reads");
    }

    #[test]
    fn the_file_names_its_image() {
        assert_eq!(module(&[]).image_id(), PeImageId { time_date_stamp: 0x5E1F_0001, size_of_image: 0x5000 });
    }
}
