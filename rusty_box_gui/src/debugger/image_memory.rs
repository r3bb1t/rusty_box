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
use crate::symbols::hub::{ImageKey, ImageRetry, ImageState, SymbolsView};

/// What stands behind one module's bytes.
enum Backing {
    File(Arc<ModuleFile>),
    /// The binary is being looked up, has not been asked for, or failed in
    /// a way asking again may mend: a read that needs it names it.
    Awaited(ImageKey),
    /// No binary: an unnamed or ELF module, or one whose binary was not
    /// found.
    Nothing,
}

/// What stands behind `module`'s bytes, as `symbols` has its binary.
fn backing_of(module: &SymModule, symbols: Option<&SymbolsView>) -> Backing {
    let Some(key) = module.identity.known().and_then(ImageKey::of) else {
        return Backing::Nothing;
    };
    if let Some(file) = symbols.and_then(|view| view.image(&key)) {
        return Backing::File(Arc::clone(file));
    }
    match symbols.and_then(|view| view.image_state(&key)) {
        None
        | Some(
            ImageState::Pending | ImageState::Downloading { .. } | ImageState::Failed { retry: ImageRetry::From(_), .. },
        ) => Backing::Awaited(key),
        Some(ImageState::Loaded { .. } | ImageState::NotOnServer | ImageState::Failed { retry: ImageRetry::Never, .. }) => {
            Backing::Nothing
        }
    }
}

/// Whether `[at, at + len)` lies inside `module`.
fn holds(module: &SymModule, at: u64, len: u64) -> bool {
    at.checked_sub(module.base)
        .and_then(|offset| offset.checked_add(len))
        .is_some_and(|end| end <= module.size)
}

/// `memory`, with the binaries of its loaded modules behind it. A module's
/// binary is looked up only when a page of it does not read.
pub(crate) struct ImageBacked<'a, G> {
    memory: &'a mut G,
    modules: &'a [SymModule],
    symbols: Option<&'a SymbolsView>,
    awaited: Vec<ImageKey>,
}

impl<'a, G: GuestMemory> ImageBacked<'a, G> {
    /// Each PE module of `modules` backed by its binary when `symbols` has
    /// it, awaiting it while it is looked up or was never asked for.
    pub(crate) fn new(memory: &'a mut G, modules: &'a [SymModule], symbols: Option<&'a SymbolsView>) -> Self {
        ImageBacked { memory, modules, symbols, awaited: Vec::new() }
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
        let Some(module) = self.modules.iter().find(|module| holds(module, linear, len)) else {
            return GuestRead::Unreadable;
        };
        match backing_of(module, self.symbols) {
            Backing::File(file) => match file.read_loaded(module.base, Rva(linear.wrapping_sub(module.base)), buf) {
                ImageRead::Read => GuestRead::Read,
                ImageRead::Unreadable => GuestRead::Unreadable,
            },
            Backing::Awaited(key) => {
                if !self.awaited.contains(&key) {
                    self.awaited.push(key);
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

    /// The guest's bytes; a page that does not map, or does not read, from
    /// the binary of the module that holds it.
    fn read_linear_page(&mut self, root: Root, linear: u64, chunk: &mut [u8]) -> GuestRead {
        match self.memory.read_linear_page(root, linear, chunk) {
            GuestRead::Read => GuestRead::Read,
            GuestRead::Unreadable => self.from_binary(linear, chunk),
        }
    }
}
