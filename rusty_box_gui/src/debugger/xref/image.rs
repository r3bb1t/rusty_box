//! A loaded module's image, copied out of a paused guest (spec §3): its bytes
//! as mapped, which pages read, its sections, and the offsets known to start
//! an instruction.

use crate::debugger::guest::{GuestMemory, GuestRead, Root};
use crate::debugger::pe::{self, ExportRead, ExportTarget, ImageWidth, PeSection, HEADER_BYTES};
use crate::debugger::unwind::function_starts;

/// An image past this is not copied.
pub(crate) const MAX_IMAGE_BYTES: u64 = 256 << 20;
pub(crate) const PAGE_BYTES: u64 = 0x1000;

/// An offset into a module's image: its RVA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ImageOffset(pub(crate) u64);

/// Where a module is mapped: its base and size, in the address space `root`
/// names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImageAt {
    pub(crate) base: u64,
    pub(crate) size: u64,
    pub(crate) root: Root,
}

/// Whether one page of an image read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageRead {
    Read,
    /// It does not map (paged out, or a section the loader discarded); its
    /// bytes in the copy are zero.
    Hole,
}

/// A module's image as it was mapped when it was copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleImage {
    pub(crate) base: u64,
    pub(crate) width: ImageWidth,
    /// The image as mapped, each RVA its own offset.
    pub(crate) bytes: Vec<u8>,
    /// One per `PAGE_BYTES` of `bytes`.
    pub(crate) pages: Vec<PageRead>,
    pub(crate) sections: Vec<PeSection>,
    /// Offsets inside executable sections known to start an instruction,
    /// ascending, each once.
    pub(crate) sync: Vec<ImageOffset>,
}

/// Why a module was not copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageMiss {
    /// Its header page does not read.
    NoHeaders,
    /// Its headers do not read as PE.
    NotPe,
    TooLarge { bytes: u64 },
    NoMemory,
}

impl ImageMiss {
    pub(crate) fn describe(self) -> String {
        match self {
            ImageMiss::NoHeaders => "its headers do not read".to_owned(),
            ImageMiss::NotPe => "its headers are not a PE image's".to_owned(),
            ImageMiss::TooLarge { bytes } => {
                format!("it is {} MiB, past the {} MiB copied", bytes.wrapping_shr(20), MAX_IMAGE_BYTES.wrapping_shr(20))
            }
            ImageMiss::NoMemory => "there is no memory for its copy".to_owned(),
        }
    }
}

/// Copies the module at `at` out of `memory`: its headers, then every page of
/// `[base, base + size)`, a page that does not read left zero and marked a
/// hole; then gathers the offsets known to start an instruction — the entry
/// point, the exports that lie in code (a forwarder is not code), and for a
/// 64-bit image the begin of every `RUNTIME_FUNCTION`, read from the copy.
pub(crate) fn copy_image<G: GuestMemory>(memory: &mut G, at: ImageAt) -> Result<ModuleImage, ImageMiss> {
    if at.size > MAX_IMAGE_BYTES {
        return Err(ImageMiss::TooLarge { bytes: at.size });
    }
    let size = usize::try_from(at.size).map_err(|_| ImageMiss::TooLarge { bytes: at.size })?;
    let mut header = vec![0u8; HEADER_BYTES];
    if memory.read_linear(at.root, at.base, &mut header) == GuestRead::Unreadable {
        return Err(ImageMiss::NoHeaders);
    }
    let width = pe::image_width(&header).ok_or(ImageMiss::NotPe)?;
    let sections = pe::parse_sections(&header).ok_or(ImageMiss::NotPe)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| ImageMiss::NoMemory)?;
    bytes.resize(size, 0);
    let mut pages = Vec::new();
    for (index, page) in bytes.chunks_mut(PAGE_BYTES as usize).enumerate() {
        let offset = (index as u64).wrapping_mul(PAGE_BYTES);
        match memory.read_linear(at.root, at.base.wrapping_add(offset), page) {
            GuestRead::Read => pages.push(PageRead::Read),
            GuestRead::Unreadable => {
                page.fill(0);
                pages.push(PageRead::Hole);
            }
        }
    }
    let mut image = ModuleImage { base: at.base, width, bytes, pages, sections, sync: Vec::new() };
    image.sync = sync_points(&image, &header);
    Ok(image)
}

impl ModuleImage {
    /// Whether `rva` lies in an executable section.
    pub(crate) fn in_code(&self, rva: u64) -> bool {
        self.sections.iter().any(|section| {
            section.executable
                && rva.checked_sub(u64::from(section.rva)).is_some_and(|into| into < u64::from(section.virtual_size))
        })
    }

    /// The `len` bytes at `rva`, when every page they touch read.
    pub(crate) fn read(&self, rva: u64, len: usize) -> Option<&[u8]> {
        let start = usize::try_from(rva).ok()?;
        let end = start.checked_add(len)?;
        let bytes = self.bytes.get(start..end)?;
        let first = start.checked_div(PAGE_BYTES as usize)?;
        let last = end.saturating_sub(1).checked_div(PAGE_BYTES as usize)?;
        let pages = self.pages.get(first..=last.max(first))?;
        pages.iter().all(|page| *page == PageRead::Read).then_some(bytes)
    }
}

/// The offsets in `image`'s code known to start an instruction.
fn sync_points(image: &ModuleImage, header: &[u8]) -> Vec<ImageOffset> {
    let mut sync: Vec<u64> = Vec::new();
    sync.extend(pe::entry_point(header).map(u64::from));
    let read = |rva: u32, len: usize| image.read(u64::from(rva), len).map(<[u8]>::to_vec);
    if let ExportRead::Listed(exports) = pe::parse_exports(header, read) {
        sync.extend(exports.iter().filter_map(|export| match export.target {
            ExportTarget::Rva(rva) => Some(u64::from(rva)),
            ExportTarget::Forwarded(_) => None,
        }));
    }
    if image.width == ImageWidth::Bits64 {
        if let Some(directory) = pe::exception_directory(header) {
            sync.extend(function_starts(&image.bytes, directory).into_iter().map(u64::from));
        }
    }
    sync.retain(|rva| image.in_code(*rva));
    sync.sort_unstable();
    sync.dedup();
    sync.into_iter().map(ImageOffset).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::guest::fake::{FakeGuest, PAGE};
    use crate::debugger::guest::{GuestMemory, GuestRead, Root};
    use crate::debugger::pe::ImageWidth;
    use crate::debugger::xref::test_image::{build_image, Function, ImageSpec};
    use rusty_box::cpu::{LinearRange, MappingWalk, PageMapping, WalkStep};

    const CR3: u64 = 0x5000;
    const BASE: u64 = 0x1_4000_0000;

    fn spec() -> ImageSpec {
        let mut spec = ImageSpec::bits64(0x3000);
        spec.entry = 0x1200;
        spec.exports = vec![0x1100, 0x2000];
        spec.functions = vec![Function { begin: 0x1100, end: 0x1180 }, Function { begin: 0x1400, end: 0x1500 }];
        spec
    }

    fn guest_with(bytes: &[u8]) -> FakeGuest {
        let mut guest = FakeGuest::new(CR3);
        guest.write(CR3, BASE, bytes);
        guest
    }

    fn at(size: u64) -> ImageAt {
        ImageAt { base: BASE, size, root: Root::Live }
    }

    #[test]
    fn an_image_is_copied_with_its_sections_and_sync_points() {
        let spec = spec();
        let bytes = build_image(&spec);
        let image = copy_image(&mut guest_with(&bytes), at(bytes.len() as u64)).expect("an image");
        assert_eq!(image.width, ImageWidth::Bits64);
        assert_eq!(image.base, BASE);
        assert_eq!(image.bytes, bytes);
        assert!(image.pages.iter().all(|page| *page == PageRead::Read));
        let names: Vec<&str> = image.sections.iter().map(|section| section.name.as_str()).collect();
        assert_eq!(names, [".text", ".data"]);
        // The entry, both exports and both function begins; the export at
        // 0x1100 is also a function's begin and appears once.
        assert_eq!(
            image.sync,
            [ImageOffset(0x1100), ImageOffset(0x1200), ImageOffset(0x1400), ImageOffset(0x2000)]
        );
    }

    #[test]
    fn a_page_that_does_not_read_is_a_hole_of_zeros() {
        let spec = spec();
        let mut bytes = build_image(&spec);
        bytes[0x3000..0x3010].fill(0xCC);
        let mut guest = guest_with(&bytes);
        guest.page_out(CR3, BASE + 0x3000);
        let image = copy_image(&mut guest, at(bytes.len() as u64)).expect("an image");
        assert_eq!(image.pages[3], PageRead::Hole);
        assert!(image.bytes[0x3000..0x4000].iter().all(|byte| *byte == 0));
        assert_eq!(image.pages.iter().filter(|page| **page == PageRead::Hole).count(), 1);
    }

    #[test]
    fn a_pe32_image_is_32_bit_and_has_no_function_table() {
        let mut spec = spec();
        spec.width = ImageWidth::Bits32;
        let bytes = build_image(&spec);
        let image = copy_image(&mut guest_with(&bytes), at(bytes.len() as u64)).expect("an image");
        assert_eq!(image.width, ImageWidth::Bits32);
        assert_eq!(image.sync, [ImageOffset(0x1100), ImageOffset(0x1200), ImageOffset(0x2000)]);
    }

    /// A guest that counts the pages it was asked to translate.
    struct Counting {
        guest: FakeGuest,
        translated: u64,
    }

    impl GuestMemory for Counting {
        fn translate(&mut self, root: Root, linear: u64) -> Option<u64> {
            self.translated += 1;
            self.guest.translate(root, linear)
        }

        fn read_physical(&mut self, physical: u64, buf: &mut [u8]) -> GuestRead {
            self.guest.read_physical(physical, buf)
        }

        fn page_mappings<F>(&mut self, cr3: u64, range: LinearRange, visit: F) -> MappingWalk
        where
            F: FnMut(PageMapping) -> WalkStep,
        {
            self.guest.page_mappings(cr3, range, visit)
        }
    }

    #[test]
    fn an_image_past_256_mib_is_refused_before_it_is_read() {
        let bytes = build_image(&spec());
        let mut counting = Counting { guest: guest_with(&bytes), translated: 0 };
        let size = 257 << 20;
        assert_eq!(copy_image(&mut counting, at(size)).err(), Some(ImageMiss::TooLarge { bytes: size }));
        assert_eq!(counting.translated, 0);
    }

    #[test]
    fn bytes_that_are_not_pe_are_refused() {
        let mut guest = FakeGuest::new(CR3);
        guest.write(CR3, BASE, &[0x90; PAGE as usize * 2]);
        assert_eq!(copy_image(&mut guest, at(PAGE * 2)).err(), Some(ImageMiss::NotPe));
        let mut empty = FakeGuest::new(CR3);
        assert_eq!(copy_image(&mut empty, at(PAGE * 2)).err(), Some(ImageMiss::NoHeaders));
    }
}
