//! Breakpoints on source lines (spec §7.1): a `file:line` the owner types or
//! clicks is kept as typed and resolved against every module of the pause
//! whose tables have the line, into one linear breakpoint per code range
//! start; until one does, it is pending and says why.

use rusty_box::cpu::CodeBreakpoint;

pub(crate) use super::inspect::SymbolIndex;
pub(crate) use crate::symbols::hub::SymbolsView;
use crate::symbols::hub::ModuleSymbols;

/// A source line as the owner named it: the path's trailing components
/// (`read.c`, `src\io\read.c`) and the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceTarget {
    pub(crate) file: String,
    pub(crate) line: u32,
}

/// One of the owner's breakpoints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BreakpointTarget {
    Address(CodeBreakpoint),
    Source(SourceTarget),
}

/// What a breakpoint field's text is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetText<'a> {
    Source(SourceTarget),
    /// An address or `module!name`, for the address reader.
    Address(&'a str),
}

/// Why a `file:line` was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetError {
    NoLine,
    NoFile,
    /// The line is not a number from 1 up.
    BadLine,
}

impl TargetError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            TargetError::NoLine => "A source breakpoint is file:line; the line is missing.",
            TargetError::NoFile => "A source breakpoint is file:line; the file is missing.",
            TargetError::BadLine => "A line is a number from 1 up.",
        }
    }
}

/// Reads a breakpoint field's text: `file:line` (the last `:` splits them,
/// so a drive letter stays in the path), else an address or `module!name`.
pub(crate) fn parse_target(text: &str) -> Result<TargetText<'_>, TargetError> {
    let text = text.trim();
    if text.contains('!') {
        return Ok(TargetText::Address(text));
    }
    let Some((file, line)) = text.rsplit_once(':') else {
        return Ok(TargetText::Address(text));
    };
    let file = file.trim();
    let line = line.trim();
    if file.is_empty() {
        return Err(TargetError::NoFile);
    }
    if line.is_empty() {
        return Err(TargetError::NoLine);
    }
    // A path with its drive and no line (`C:\src\read.c`).
    if line.contains(['\\', '/']) {
        return Err(TargetError::NoLine);
    }
    match line.parse::<u32>() {
        Ok(line) if line > 0 => Ok(TargetText::Source(SourceTarget { file: file.to_owned(), line })),
        Ok(_) | Err(_) => Err(TargetError::BadLine),
    }
}

/// Where a source breakpoint resolved: a code address, the module it is in,
/// and the line that has the code (the next one when the asked line has
/// none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedPlace {
    pub(crate) address: u64,
    pub(crate) module: String,
    pub(crate) line: u32,
}

/// Why a source breakpoint has no place yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PendingWhy {
    NoModuleHasLine,
    /// These modules' symbols are still being looked up.
    SymbolsLoading { modules: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceResolution {
    Resolved { places: Vec<ResolvedPlace> },
    Pending(PendingWhy),
}

/// `target` in every module of `index` whose tables (in `view`) have the
/// line, in the modules' order, each code range's start a place.
pub(crate) fn resolve(target: &SourceTarget, index: &SymbolIndex, view: &SymbolsView) -> SourceResolution {
    let mut places = Vec::new();
    let mut loading = Vec::new();
    for module in &index.modules {
        let Some(identity) = module.identity.known() else {
            continue;
        };
        match view.state(identity) {
            Some(ModuleSymbols::Pending | ModuleSymbols::Downloading { .. }) => loading.push(module.name.clone()),
            Some(
                ModuleSymbols::Loaded { .. }
                | ModuleSymbols::Refused { .. }
                | ModuleSymbols::Forced { .. }
                | ModuleSymbols::NotOnServer
                | ModuleSymbols::Failed { .. }
                | ModuleSymbols::Unloaded,
            )
            | None => {}
        }
        let Some(tables) = view.tables(identity) else {
            continue;
        };
        for resolved in tables.lines_of(&target.file, target.line) {
            places.extend(resolved.ranges.iter().map(|range| ResolvedPlace {
                address: module.base.wrapping_add(range.start.0),
                module: module.name.clone(),
                line: resolved.line,
            }));
        }
    }
    if !places.is_empty() {
        SourceResolution::Resolved { places }
    } else if loading.is_empty() {
        SourceResolution::Pending(PendingWhy::NoModuleHasLine)
    } else {
        SourceResolution::Pending(PendingWhy::SymbolsLoading { modules: loading })
    }
}

/// The breakpoints the machine is given for `targets`: each address, and
/// each place a source target resolves to under the pause's modules
/// (`index`, none before a first pause), each address once.
pub(crate) fn applied_set(targets: &[BreakpointTarget], index: Option<&SymbolIndex>, view: &SymbolsView) -> Vec<CodeBreakpoint> {
    let mut set: Vec<CodeBreakpoint> = Vec::new();
    let mut add = |breakpoint: CodeBreakpoint| {
        if !set.contains(&breakpoint) {
            set.push(breakpoint);
        }
    };
    for target in targets {
        match target {
            BreakpointTarget::Address(breakpoint) => add(*breakpoint),
            BreakpointTarget::Source(source) => {
                let Some(index) = index else {
                    continue;
                };
                match resolve(source, index, view) {
                    SourceResolution::Resolved { places } => {
                        for place in places {
                            add(CodeBreakpoint::Linear(place.address));
                        }
                    }
                    SourceResolution::Pending(_) => {}
                }
            }
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::inspect::{ModuleSpace, SymModule};
    use crate::debugger::module_identity::GuestIdentity;
    use crate::symbols::hub::{ModuleEntry, ModuleSymbols};
    use rusty_box_symbols::identity::{Guid, ModuleIdentity, PdbId, PeImageId};
    use rusty_box_symbols::service::SymbolSource;
    use rusty_box_symbols::tables::SymbolTables;
    use rusty_box_symbols_fixtures::pdb::{build_pdb, FixtureLine, FixtureProcedure, PdbFixture};
    use std::sync::Arc;

    fn source(file: &str, line: u32) -> SourceTarget {
        SourceTarget { file: file.to_owned(), line }
    }

    #[test]
    fn a_file_and_line_is_a_source_target() {
        assert_eq!(parse_target("read.c:120"), Ok(TargetText::Source(source("read.c", 120))));
        assert_eq!(parse_target(r" src\io\read.c:120 "), Ok(TargetText::Source(source(r"src\io\read.c", 120))));
        assert_eq!(parse_target(r"C:\src\io\read.c:7"), Ok(TargetText::Source(source(r"C:\src\io\read.c", 7))));
    }

    #[test]
    fn an_address_or_a_name_is_left_to_the_address_reader() {
        assert_eq!(parse_target("0x1234"), Ok(TargetText::Address("0x1234")));
        assert_eq!(parse_target("nt!KeBugCheckEx"), Ok(TargetText::Address("nt!KeBugCheckEx")));
        assert_eq!(parse_target("fffff800`12345678"), Ok(TargetText::Address("fffff800`12345678")));
    }

    #[test]
    fn a_target_without_its_line_or_file_is_refused() {
        assert_eq!(parse_target("read.c:"), Err(TargetError::NoLine));
        assert_eq!(parse_target(r"C:\src\read.c"), Err(TargetError::NoLine));
        assert_eq!(parse_target(":3"), Err(TargetError::NoFile));
        assert_eq!(parse_target("read.c:0"), Err(TargetError::BadLine));
        assert_eq!(parse_target("read.c:99999999999"), Err(TargetError::BadLine));
        assert_eq!(parse_target("read.c:1x"), Err(TargetError::BadLine));
    }

    const A: u64 = 0xFFFF_F800_1000_0000;
    const B: u64 = 0xFFFF_F800_2000_0000;

    fn identity(name: &str, byte: u8) -> ModuleIdentity {
        ModuleIdentity::Pe {
            file_name: name.into(),
            image: PeImageId { time_date_stamp: u32::from(byte), size_of_image: 0x10_0000 },
            pdb: Some(PdbId { name: name.replace(".sys", ".pdb"), guid: Guid::from_le_bytes([byte; 16]), age: 1 }),
        }
    }

    /// A module's PDB: `ReadBlock` in `file` at RVA 0x1100, line 10 at +0
    /// and 12 at +8.
    fn tables(byte: u8, file: &str) -> Arc<SymbolTables> {
        let mut fixture = PdbFixture::new([byte; 16], 1);
        fixture.procedures.push(FixtureProcedure {
            name: "ReadBlock".into(),
            section: 1,
            offset: 0x100,
            len: 0x20,
            file: file.into(),
            checksum: None,
            lines: vec![FixtureLine { offset: 0, line: 10 }, FixtureLine { offset: 8, line: 12 }],
        });
        Arc::new(SymbolTables::parse(&build_pdb(&fixture)).expect("the fixture parses"))
    }

    fn module(name: &str, base: u64, byte: u8) -> SymModule {
        SymModule { base, size: 0x10_0000, name: name.into(), space: ModuleSpace::Kernel, identity: GuestIdentity::Known(identity(name, byte)) }
    }

    fn loaded(view: &mut SymbolsView, name: &str, byte: u8, file: &str) {
        let entry = ModuleEntry {
            state: ModuleSymbols::Loaded { from: SymbolSource::Store { path: "store".into() } },
            tables: Some(tables(byte, file)),
        };
        view.modules.insert(identity(name, byte), entry);
    }

    fn pending(view: &mut SymbolsView, name: &str, byte: u8) {
        view.modules.insert(identity(name, byte), ModuleEntry { state: ModuleSymbols::Pending, tables: None });
    }

    fn two_drivers() -> SymbolIndex {
        SymbolIndex { modules: vec![module("a.sys", A, 1), module("b.sys", B, 2)], kernel: None }
    }

    fn place(address: u64, module: &str, line: u32) -> ResolvedPlace {
        ResolvedPlace { address, module: module.into(), line }
    }

    /// A file both drivers were built from (a shared library) breaks in
    /// both.
    #[test]
    fn a_line_resolves_in_every_module_that_has_it() {
        let mut view = SymbolsView::default();
        loaded(&mut view, "a.sys", 1, r"d:\b\src\io\read.c");
        loaded(&mut view, "b.sys", 2, r"e:\other\io\read.c");
        assert_eq!(
            resolve(&source(r"io\read.c", 10), &two_drivers(), &view),
            SourceResolution::Resolved { places: vec![place(A + 0x1100, "a.sys", 10), place(B + 0x1100, "b.sys", 10)] }
        );
    }

    #[test]
    fn a_line_without_code_resolves_to_the_next_line_and_says_so() {
        let mut view = SymbolsView::default();
        loaded(&mut view, "a.sys", 1, r"d:\b\src\io\read.c");
        assert_eq!(
            resolve(&source("read.c", 11), &two_drivers(), &view),
            SourceResolution::Resolved { places: vec![place(A + 0x1108, "a.sys", 12)] }
        );
    }

    #[test]
    fn a_line_no_module_has_is_pending_and_says_why() {
        let mut view = SymbolsView::default();
        loaded(&mut view, "a.sys", 1, r"d:\b\src\io\write.c");
        pending(&mut view, "b.sys", 2);
        assert_eq!(
            resolve(&source("read.c", 10), &two_drivers(), &view),
            SourceResolution::Pending(PendingWhy::SymbolsLoading { modules: vec!["b.sys".into()] })
        );
        loaded(&mut view, "b.sys", 2, r"e:\other\write.c");
        assert_eq!(resolve(&source("read.c", 10), &two_drivers(), &view), SourceResolution::Pending(PendingWhy::NoModuleHasLine));
    }

    #[test]
    fn the_applied_set_follows_the_symbols_as_they_load() {
        let targets = [
            BreakpointTarget::Address(CodeBreakpoint::Linear(0x5000)),
            BreakpointTarget::Source(source("read.c", 10)),
            BreakpointTarget::Address(CodeBreakpoint::Linear(A + 0x1100)),
        ];
        let mut view = SymbolsView::default();
        pending(&mut view, "a.sys", 1);
        let index = two_drivers();
        assert_eq!(
            applied_set(&targets, Some(&index), &view),
            [CodeBreakpoint::Linear(0x5000), CodeBreakpoint::Linear(A + 0x1100)]
        );
        loaded(&mut view, "a.sys", 1, r"d:\b\src\io\read.c");
        assert_eq!(
            applied_set(&targets, Some(&index), &view),
            [CodeBreakpoint::Linear(0x5000), CodeBreakpoint::Linear(A + 0x1100)],
            "the line's address is one breakpoint, however many targets name it"
        );
        loaded(&mut view, "b.sys", 2, r"e:\other\read.c");
        assert_eq!(
            applied_set(&targets, Some(&index), &view),
            [CodeBreakpoint::Linear(0x5000), CodeBreakpoint::Linear(A + 0x1100), CodeBreakpoint::Linear(B + 0x1100)]
        );
        assert_eq!(applied_set(&targets, None, &view), [CodeBreakpoint::Linear(0x5000), CodeBreakpoint::Linear(A + 0x1100)]);
    }
}
