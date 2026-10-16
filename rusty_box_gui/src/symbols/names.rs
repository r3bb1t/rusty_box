//! Names for addresses, as the panes draw them: the module a pause found an
//! address in and, once that module's tables have loaded, the function and
//! source line. Names are made when drawn, from the hub's latest view, so a
//! download that finishes names addresses at once, without a new pause.

use rusty_box_symbols::tables::SourceLine;
use rusty_box_symbols::units::Rva;

use crate::debugger::inspect::SymbolIndex;

use super::hub::{ModuleSymbols, SymbolsView};

/// Whether a name comes from tables of the module's own build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameTrust {
    Matched,
    /// From a file of another build the owner loaded anyway: drawn amber.
    Forced,
}

/// The function an address lies in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FunctionName {
    pub(crate) name: String,
    /// How far into the function the address lies.
    pub(crate) offset: u64,
    pub(crate) line: Option<SourceLine>,
}

/// An address, named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NamedAddress {
    /// The module's file name (`ntoskrnl.exe`).
    pub(crate) module: String,
    /// How far into the module the address lies.
    pub(crate) module_offset: u64,
    pub(crate) function: Option<FunctionName>,
    pub(crate) trust: NameTrust,
}

impl NamedAddress {
    /// `ntoskrnl!KiSystemCall64+0x12` once the function is known, the
    /// module's name without its extension as WinDbg writes it; else
    /// `ntoskrnl.exe+0x1a2c`, or the bare module name at its base.
    pub(crate) fn label(&self) -> String {
        match &self.function {
            Some(function) => {
                let stem = self.module.rsplit_once('.').map_or(self.module.as_str(), |(stem, _extension)| stem);
                if function.offset == 0 {
                    format!("{stem}!{}", function.name)
                } else {
                    format!("{stem}!{}+{:#x}", function.name, function.offset)
                }
            }
            None if self.module_offset == 0 => self.module.clone(),
            None => format!("{}+{:#x}", self.module, self.module_offset),
        }
    }

    /// Whether the address is where its function begins.
    pub(crate) fn starts_function(&self) -> bool {
        self.function.as_ref().is_some_and(|function| function.offset == 0)
    }

    /// `file:line`, when the tables have line information for the address.
    pub(crate) fn source(&self) -> Option<String> {
        let line = self.function.as_ref()?.line.as_ref()?;
        let file = line.file.rsplit(['\\', '/']).next().unwrap_or(&line.file);
        Some(format!("{file}:{}", line.line))
    }
}

/// Why a `module!name` names no address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameMiss {
    UnknownModule,
    /// The module's tables have not loaded.
    NoSymbols,
    UnknownName,
}

impl NameMiss {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            NameMiss::UnknownModule => "No module of this pause has that name.",
            NameMiss::NoSymbols => "That module's symbols are not loaded.",
            NameMiss::UnknownName => "That module's symbols have no such name.",
        }
    }
}

/// Names addresses against one pause's modules and the engine's tables.
#[derive(Clone, Copy)]
pub(crate) struct Namer<'a> {
    symbols: &'a SymbolIndex,
    view: &'a SymbolsView,
}

impl<'a> Namer<'a> {
    pub(crate) fn new(symbols: &'a SymbolIndex, view: &'a SymbolsView) -> Self {
        Namer { symbols, view }
    }

    /// Where `module!name` is: the module by its file name with or without
    /// its extension, or `nt` for the kernel as WinDbg writes it; the name by
    /// that module's tables.
    pub(crate) fn address_of(&self, module: &str, name: &str) -> Result<u64, NameMiss> {
        let found = self
            .symbols
            .modules
            .iter()
            .find(|candidate| {
                let stem = candidate.name.rsplit_once('.').map_or(candidate.name.as_str(), |(stem, _extension)| stem);
                candidate.name.eq_ignore_ascii_case(module)
                    || stem.eq_ignore_ascii_case(module)
                    || (module.eq_ignore_ascii_case("nt") && self.symbols.kernel == Some(candidate.base))
            })
            .ok_or(NameMiss::UnknownModule)?;
        let tables = found.identity.known().and_then(|identity| self.view.tables(identity)).ok_or(NameMiss::NoSymbols)?;
        let rva = tables.address_of(name).ok_or(NameMiss::UnknownName)?;
        Ok(found.base.wrapping_add(rva.0))
    }

    pub(crate) fn name(&self, address: u64) -> Option<NamedAddress> {
        let module = self.symbols.module_at(address)?;
        // `module_at` found `address` at or above the module's base.
        let module_offset = address.wrapping_sub(module.base);
        let identity = module.identity.known();
        let function = identity
            .and_then(|identity| self.view.tables(identity))
            .and_then(|tables| tables.symbolize(Rva(module_offset)))
            .map(|hit| FunctionName { name: hit.name, offset: hit.offset, line: hit.line });
        let forced = identity
            .and_then(|identity| self.view.state(identity))
            .is_some_and(|state| matches!(state, ModuleSymbols::Forced { .. }));
        Some(NamedAddress {
            module: module.name.clone(),
            module_offset,
            function,
            trust: if forced { NameTrust::Forced } else { NameTrust::Matched },
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::symbols::hub::{ModuleEntry, SymbolHub};
    use crate::symbols::kernel::tests::{kernel_identity, kernel_symbols, look_up_as_a_worker, quiet_hub, KERNEL_BASE, KERNEL_GUID};
    use rusty_box_symbols::identity::{Guid, TablesIdentity};
    use rusty_box_symbols::service::SymbolSource;
    use rusty_box_symbols::tables::SymbolTables;
    use rusty_box_symbols_fixtures::pdb::{build_pdb, FixtureLine, FixtureProcedure, FixturePublic, PdbFixture, PublicKind};
    use std::path::PathBuf;
    use std::sync::Arc;

    /// `KiSystemCall64` at RVA 0x1040 with a line table, and `KeBugCheckEx`
    /// at 0x1100 as a public only.
    pub(crate) fn kernel_code_tables() -> SymbolTables {
        let mut pdb = PdbFixture::new(KERNEL_GUID, 1);
        pdb.procedures.push(FixtureProcedure {
            name: "KiSystemCall64".to_owned(),
            section: 1,
            offset: 0x40,
            len: 0x40,
            file: "d:\\os\\src\\ntos\\ke\\amd64\\trap.asm".to_owned(),
            lines: vec![FixtureLine { offset: 0, line: 100 }, FixtureLine { offset: 0x10, line: 104 }],
        });
        pdb.publics.push(FixturePublic { name: "KiSystemCall64".to_owned(), section: 1, offset: 0x40, kind: PublicKind::Function });
        pdb.publics.push(FixturePublic { name: "KeBugCheckEx".to_owned(), section: 1, offset: 0x100, kind: PublicKind::Function });
        SymbolTables::parse(&build_pdb(&pdb)).expect("parses")
    }

    /// A hub holding the kernel's code tables, loaded as `state` says.
    pub(crate) fn hub_with_kernel_code(state: ModuleSymbols) -> SymbolHub {
        let hub = quiet_hub();
        look_up_as_a_worker(&hub, &kernel_identity(), ModuleEntry { state, tables: Some(Arc::new(kernel_code_tables())) });
        hub
    }

    pub(crate) fn loaded() -> ModuleSymbols {
        ModuleSymbols::Loaded { from: SymbolSource::Store { path: PathBuf::from("symbols/ntkrnlmp.pdb") } }
    }

    #[test]
    fn an_address_is_named_by_its_function_once_the_tables_load() {
        let symbols = kernel_symbols();
        let before = quiet_hub().view();
        let named = Namer::new(&symbols, &before).name(KERNEL_BASE + 0x1052).expect("in the kernel");
        assert_eq!(named.label(), "ntoskrnl.exe+0x1052");
        let view = hub_with_kernel_code(loaded()).view();
        let namer = Namer::new(&symbols, &view);
        let named = namer.name(KERNEL_BASE + 0x1052).expect("in the kernel");
        assert_eq!(named.label(), "ntoskrnl!KiSystemCall64+0x12");
        assert_eq!(named.source().as_deref(), Some("trap.asm:104"));
        assert_eq!(named.trust, NameTrust::Matched);
        let start = namer.name(KERNEL_BASE + 0x1040).expect("in the kernel");
        assert!(start.starts_function());
        assert_eq!(start.label(), "ntoskrnl!KiSystemCall64");
        assert_eq!(namer.name(KERNEL_BASE + 0x1100).map(|n| n.label()).as_deref(), Some("ntoskrnl!KeBugCheckEx"));
        assert_eq!(namer.name(0x1000), None, "outside every module");
    }

    #[test]
    fn a_name_is_found_in_its_modules_tables() {
        let symbols = kernel_symbols();
        let view = hub_with_kernel_code(loaded()).view();
        let namer = Namer::new(&symbols, &view);
        assert_eq!(namer.address_of("ntoskrnl", "KeBugCheckEx"), Ok(KERNEL_BASE + 0x1100));
        assert_eq!(namer.address_of("NTOSKRNL.EXE", "KiSystemCall64"), Ok(KERNEL_BASE + 0x1040));
        assert_eq!(namer.address_of("nt", "KiSystemCall64"), Ok(KERNEL_BASE + 0x1040));
        assert_eq!(namer.address_of("disk", "DiskRead"), Err(NameMiss::UnknownModule));
        assert_eq!(namer.address_of("nt", "NoSuchName"), Err(NameMiss::UnknownName));
        let unloaded = quiet_hub().view();
        assert_eq!(Namer::new(&symbols, &unloaded).address_of("nt", "KeBugCheckEx"), Err(NameMiss::NoSymbols));
    }

    /// Review focus 3: names from a file of another build, loaded anyway,
    /// are marked so the pane draws them amber.
    #[test]
    fn a_name_from_a_forced_file_is_marked_forced() {
        let symbols = kernel_symbols();
        let view = hub_with_kernel_code(ModuleSymbols::Forced {
            found: TablesIdentity::Pdb { guid: Guid::from_le_bytes([0x77; 16]), age: 3 },
            path: PathBuf::from("D:\\other\\ntkrnlmp.pdb"),
        })
        .view();
        let named = Namer::new(&symbols, &view).name(KERNEL_BASE + 0x1040).expect("in the kernel");
        assert_eq!(named.label(), "ntoskrnl!KiSystemCall64");
        assert_eq!(named.trust, NameTrust::Forced);
    }
}
