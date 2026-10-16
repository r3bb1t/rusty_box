//! The kernel's layouts for the walks, made from the kernel's tables once
//! they load. Making them reads a few dozen type records, so they are made
//! once per loaded file and kept.

use std::fmt;
use std::sync::Arc;

use rusty_box_symbols::tables::SymbolTables;

use crate::debugger::inspect::SymbolIndex;
use crate::debugger::kernel_layout::{KernelLayout, LayoutSource};

use super::hub::{ModuleSymbols, SymbolsView};

/// Layouts made from one loaded file of tables.
pub(crate) struct LayoutCache {
    tables: Arc<SymbolTables>,
    layout: Arc<KernelLayout>,
}

impl fmt::Debug for LayoutCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayoutCache").field("kernel", &self.layout.kernel).finish_non_exhaustive()
    }
}

/// The layouts of the kernel `symbols` found, from its tables in `view`, or
/// `None` while its tables have not loaded. `cache` keeps the last made.
pub(crate) fn kernel_layout(
    symbols: &SymbolIndex,
    view: &SymbolsView,
    cache: &mut Option<LayoutCache>,
) -> Option<Arc<KernelLayout>> {
    let base = symbols.kernel?;
    let kernel = symbols.modules.iter().find(|module| module.base == base)?;
    let identity = kernel.identity.known()?;
    let tables = view.tables(identity)?;
    if let Some(made) = cache.as_ref().filter(|made| Arc::ptr_eq(&made.tables, tables)) {
        return Some(Arc::clone(&made.layout));
    }
    let source = match view.state(identity) {
        Some(ModuleSymbols::Forced { .. }) => LayoutSource::Forced,
        Some(
            ModuleSymbols::Loaded { .. }
            | ModuleSymbols::Pending
            | ModuleSymbols::Downloading { .. }
            | ModuleSymbols::Refused { .. }
            | ModuleSymbols::NotOnServer
            | ModuleSymbols::Failed { .. }
            | ModuleSymbols::Unloaded,
        )
        | None => LayoutSource::Matched,
    };
    let layout = Arc::new(KernelLayout::from_tables(identity.clone(), tables, source));
    *cache = Some(LayoutCache { tables: Arc::clone(tables), layout: Arc::clone(&layout) });
    Some(layout)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::debugger::inspect::{ModuleSpace, SymModule};
    use crate::debugger::kernel_layout::fixture;
    use crate::debugger::module_identity::GuestIdentity;
    use crate::symbols::hub::{HubConfig, Job, ModuleEntry, ModuleSymbols, Priority, SymbolHub, WantedModule};
    use rusty_box_symbols::identity::{Guid, ModuleIdentity, PdbId, PeImageId};
    use rusty_box_symbols::service::{Downloads, ServiceConfig, SymbolSource, DEFAULT_MAX_DOWNLOAD};
    use rusty_box_symbols_fixtures::pdb::build_pdb;
    use std::path::PathBuf;

    pub(crate) const KERNEL_BASE: u64 = 0xFFFF_F800_0100_0000;
    pub(crate) const KERNEL_GUID: [u8; 16] = [0x42; 16];

    pub(crate) fn kernel_identity() -> ModuleIdentity {
        ModuleIdentity::Pe {
            file_name: "ntoskrnl.exe".to_owned(),
            image: PeImageId { time_date_stamp: 0x5E1D_2A4B, size_of_image: 0x10_0000 },
            pdb: Some(PdbId { name: "ntkrnlmp.pdb".to_owned(), guid: Guid::from_le_bytes(KERNEL_GUID), age: 1 }),
        }
    }

    pub(crate) fn kernel_symbols() -> SymbolIndex {
        SymbolIndex {
            modules: vec![SymModule {
                base: KERNEL_BASE,
                size: 0x10_0000,
                name: "ntoskrnl.exe".to_owned(),
                space: ModuleSpace::Kernel,
                identity: GuestIdentity::Known(kernel_identity()),
            }],
            kernel: Some(KERNEL_BASE),
        }
    }

    /// A hub with no workers and downloads off.
    pub(crate) fn quiet_hub() -> SymbolHub {
        SymbolHub::new(HubConfig {
            store: PathBuf::from("symbols"),
            service: ServiceConfig {
                downloads: Downloads::Off,
                microsoft: Vec::new(),
                debuginfod: Vec::new(),
                max_download: DEFAULT_MAX_DOWNLOAD,
            },
            projects: Vec::new(),
        })
    }

    /// Asks `hub` for `identity`, then ends that lookup with `entry` as a
    /// worker does: it takes the job, then finishes it.
    pub(crate) fn look_up_as_a_worker(hub: &SymbolHub, identity: &ModuleIdentity, entry: ModuleEntry) {
        hub.want(&[WantedModule { identity: identity.clone(), priority: Priority::Kernel }]);
        let ticket = hub.try_next_job().expect("the lookup is queued");
        assert_eq!(ticket.job, Job::Locate { identity: identity.clone() });
        hub.finish(identity, entry);
    }

    /// A hub with no workers, the kernel's tables loaded into it as a worker
    /// would load them.
    pub(crate) fn hub_with_kernel_tables() -> SymbolHub {
        let hub = quiet_hub();
        let pdb = build_pdb(&fixture::windows10_kernel(KERNEL_GUID, 1));
        let tables = SymbolTables::parse(&pdb).expect("the fixture parses");
        look_up_as_a_worker(
            &hub,
            &kernel_identity(),
            ModuleEntry {
                state: ModuleSymbols::Loaded { from: SymbolSource::Store { path: PathBuf::from("symbols/ntkrnlmp.pdb") } },
                tables: Some(Arc::new(tables)),
            },
        );
        hub
    }

    /// Tables of another build, loaded on the owner's word, make layouts
    /// that say so.
    #[test]
    fn layouts_from_forced_tables_are_marked_forced() {
        use rusty_box_symbols::identity::TablesIdentity;
        let hub = quiet_hub();
        let pdb = build_pdb(&fixture::windows10_kernel([0x24; 16], 1));
        let tables = SymbolTables::parse(&pdb).expect("the fixture parses");
        let forced = ModuleSymbols::Forced {
            found: TablesIdentity::Pdb { guid: Guid::from_le_bytes([0x24; 16]), age: 1 },
            path: PathBuf::from("Downloads").join("ntkrnlmp.pdb"),
        };
        look_up_as_a_worker(&hub, &kernel_identity(), ModuleEntry { state: forced, tables: Some(Arc::new(tables)) });
        let made = kernel_layout(&kernel_symbols(), &hub.view(), &mut None).expect("the tables loaded");
        assert_eq!(made.source, LayoutSource::Forced);
        let matched = kernel_layout(&kernel_symbols(), &hub_with_kernel_tables().view(), &mut None).expect("loaded");
        assert_eq!(matched.source, LayoutSource::Matched);
    }

    #[test]
    fn the_kernels_layouts_are_made_once_its_tables_load() {
        let mut cache = None;
        assert!(kernel_layout(&kernel_symbols(), &quiet_hub().view(), &mut cache).is_none());
        let view = hub_with_kernel_tables().view();
        let made = kernel_layout(&kernel_symbols(), &view, &mut cache).expect("the tables loaded");
        assert_eq!(made.kernel, kernel_identity());
        assert_eq!(made.process.unique_process_id.map(|offset| offset.0), Some(0x440));
        let again = kernel_layout(&kernel_symbols(), &view, &mut cache).expect("still loaded");
        assert!(Arc::ptr_eq(&made, &again), "made once per loaded file");
    }

    #[test]
    fn no_kernel_found_means_no_layouts() {
        let view = hub_with_kernel_tables().view();
        let mut symbols = kernel_symbols();
        symbols.kernel = None;
        assert!(kernel_layout(&symbols, &view, &mut None).is_none());
    }
}
