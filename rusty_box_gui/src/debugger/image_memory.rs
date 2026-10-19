//! A paused guest's memory with its loaded modules' binaries behind it: a
//! byte of a module the guest has not paged in (an exception directory the
//! process never needed, unwind data, code not run yet) is read from the
//! module's own binary, as a debugger reads an image from its symbol path.
//! The binary serves only what the loader maps unchanged from the file
//! ([`ModuleFile::read_loaded`]); every other byte is the guest's alone.

use std::sync::Arc;

use rusty_box::cpu::{LinearRange, MappingWalk, PageMapping, WalkStep};
use rusty_box_symbols::module_file::ModuleFile;
use rusty_box_symbols::pe::ImageRead;
use rusty_box_symbols::units::Rva;

use super::guest::{GuestMemory, GuestRead, Root};
use super::inspect::SymModule;
use crate::symbols::hub::{ImageKey, ImageState, SymbolsView};

const PAGE_BYTES: u64 = 0x1000;

/// What stands behind one module's bytes.
enum Backing {
    File(Arc<ModuleFile>),
    /// The binary is being looked up, or has not been asked for: a read
    /// that needs it names it.
    Awaited(ImageKey),
    /// No binary: an unnamed or ELF module, or one whose binary was not
    /// found.
    Nothing,
}

struct BackedModule {
    base: u64,
    size: u64,
    backing: Backing,
}

impl BackedModule {
    /// Whether `[at, at + len)` lies inside the module.
    fn holds(&self, at: u64, len: u64) -> bool {
        at.checked_sub(self.base)
            .and_then(|offset| offset.checked_add(len))
            .is_some_and(|end| end <= self.size)
    }
}

/// `memory`, with the binaries of its loaded modules behind it.
pub(crate) struct ImageBacked<'a, G> {
    memory: &'a mut G,
    modules: Vec<BackedModule>,
    awaited: Vec<ImageKey>,
}

impl<'a, G: GuestMemory> ImageBacked<'a, G> {
    /// Each PE module of `modules` backed by its binary when `symbols` has
    /// it, awaiting it while it is looked up or was never asked for.
    pub(crate) fn new(memory: &'a mut G, modules: &[SymModule], symbols: Option<&SymbolsView>) -> Self {
        let modules = modules
            .iter()
            .map(|module| {
                let key = module.identity.known().and_then(ImageKey::of);
                let backing = match key {
                    Some(key) => match symbols.and_then(|view| view.image(&key)) {
                        Some(file) => Backing::File(Arc::clone(file)),
                        None => match symbols.and_then(|view| view.image_state(&key)) {
                            None | Some(ImageState::Pending | ImageState::Downloading { .. }) => Backing::Awaited(key),
                            Some(ImageState::Loaded { .. } | ImageState::NotOnServer | ImageState::Failed { .. }) => {
                                Backing::Nothing
                            }
                        },
                    },
                    None => Backing::Nothing,
                };
                BackedModule { base: module.base, size: module.size, backing }
            })
            .collect();
        ImageBacked { memory, modules, awaited: Vec::new() }
    }

    /// The binaries reads wanted and did not have, each once, in the order
    /// first wanted.
    pub(crate) fn into_awaited(self) -> Vec<ImageKey> {
        self.awaited
    }

    /// `buf`, the bytes at `linear` within one page, from the binary of the
    /// module holding them.
    fn from_binary(&mut self, linear: u64, buf: &mut [u8]) -> GuestRead {
        let len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        let Some(module) = self.modules.iter().find(|module| module.holds(linear, len)) else {
            return GuestRead::Unreadable;
        };
        match &module.backing {
            Backing::File(file) => match file.read_loaded(module.base, Rva(linear.wrapping_sub(module.base)), buf) {
                ImageRead::Read => GuestRead::Read,
                ImageRead::Unreadable => GuestRead::Unreadable,
            },
            Backing::Awaited(key) => {
                if !self.awaited.contains(key) {
                    self.awaited.push(key.clone());
                }
                GuestRead::Unreadable
            }
            Backing::Nothing => GuestRead::Unreadable,
        }
    }
}

impl<G: GuestMemory> GuestMemory for ImageBacked<'_, G> {
    fn translate(&mut self, root: Root, linear: u64) -> Option<u64> {
        self.memory.translate(root, linear)
    }

    fn read_physical(&mut self, physical: u64, buf: &mut [u8]) -> GuestRead {
        self.memory.read_physical(physical, buf)
    }

    fn page_mappings<F>(&mut self, cr3: u64, range: LinearRange, visit: F) -> MappingWalk
    where
        F: FnMut(PageMapping) -> WalkStep,
    {
        self.memory.page_mappings(cr3, range, visit)
    }

    /// The guest's bytes a page at a time, as [`GuestMemory::read_linear`]
    /// reads them; a page that does not map, or does not read, from the
    /// binary of the module that holds it.
    fn read_linear(&mut self, root: Root, linear: u64, buf: &mut [u8]) -> GuestRead {
        let mut at = linear;
        let mut rest: &mut [u8] = buf;
        while !rest.is_empty() {
            let in_page = PAGE_BYTES.wrapping_sub(at & (PAGE_BYTES - 1));
            let take = usize::try_from(in_page).map_or(rest.len(), |in_page| in_page.min(rest.len()));
            let Some((chunk, tail)) = std::mem::take(&mut rest).split_at_mut_checked(take) else {
                return GuestRead::Unreadable;
            };
            let guest = match self.memory.translate(root, at) {
                Some(physical) => self.memory.read_physical(physical, chunk),
                None => GuestRead::Unreadable,
            };
            match guest {
                GuestRead::Read => {}
                GuestRead::Unreadable => match self.from_binary(at, chunk) {
                    GuestRead::Read => {}
                    GuestRead::Unreadable => return GuestRead::Unreadable,
                },
            }
            rest = tail;
            at = at.wrapping_add(in_page);
        }
        GuestRead::Read
    }
}
