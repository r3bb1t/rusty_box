//! Breakpoints on source lines (spec §7.1): a `file:line` the owner types or
//! clicks is kept as typed and resolved against every module of the pause
//! whose tables have the line, into one linear breakpoint per code range
//! start; until one does, it is pending and says why.

use rusty_box::cpu::CodeBreakpoint;
use rusty_box_symbols::projects::ProjectIndex;

use super::inspect::{DebugSnapshot, ModuleSpace, SymModule};
pub(crate) use super::inspect::SymbolIndex;
use super::windows::same_address_space;
pub(crate) use super::windows::ModulesElsewhere;
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
/// the line that has the code (the next one when the asked line has none),
/// and the address spaces it stops in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedPlace {
    pub(crate) address: u64,
    pub(crate) module: String,
    pub(crate) line: u32,
    pub(crate) space: PlaceSpace,
}

/// The address spaces a place stops in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceSpace {
    /// Kernel code, which every address space maps; or a user module when no
    /// process list says whose it is.
    Everywhere,
    /// User code of the process whose `DirectoryTableBase` is `cr3`: another
    /// program may sit at the same address in another process.
    Process { cr3: u64 },
}

/// What a source breakpoint is resolved against: the pause's modules (the
/// kernel's and the running process's), and the owner's modules found in
/// every other process.
#[derive(Clone, Copy)]
pub(crate) struct BreakpointModules<'a> {
    pub(crate) index: &'a SymbolIndex,
    pub(crate) elsewhere: &'a ModulesElsewhere,
}

impl<'a> BreakpointModules<'a> {
    /// What `snapshot`'s pause resolves source breakpoints against.
    pub(crate) fn of(snapshot: &'a DebugSnapshot) -> Self {
        BreakpointModules { index: &snapshot.symbols, elsewhere: &snapshot.elsewhere }
    }
}

/// A breakpoint address that stops only in some address spaces, by their
/// `DirectoryTableBase`s; an address without one stops in every space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScopedAddress {
    pub(crate) linear: u64,
    pub(crate) spaces: Vec<u64>,
}

/// The breakpoints the machine is given, and the addresses of them that
/// stop only in some address spaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AppliedBreakpoints {
    pub(crate) set: Vec<CodeBreakpoint>,
    pub(crate) scopes: Vec<ScopedAddress>,
}

/// The file names of the binaries a pause looks for in every process: the
/// owner's projects' binaries, while any of the owner's breakpoints is a
/// source line; none otherwise, so a pause without one reads no other
/// process.
pub(crate) fn binaries_to_find(targets: &[BreakpointTarget], projects: &ProjectIndex) -> Vec<String> {
    if targets.iter().any(|target| matches!(target, BreakpointTarget::Source(_))) {
        projects.binary_names()
    } else {
        Vec::new()
    }
}

/// Whether a stop at `linear` in the address space `cr3` is one of the
/// owner's: an address with no scope stops everywhere.
pub(crate) fn admits(scopes: &[ScopedAddress], linear: u64, cr3: u64) -> bool {
    match scopes.iter().find(|scope| scope.linear == linear) {
        Some(scope) => scope.spaces.iter().any(|space| same_address_space(*space, cr3)),
        None => true,
    }
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

/// A module a place may be in, and the spaces its places stop in.
struct Placed<'a> {
    module: &'a SymModule,
    space: PlaceSpace,
}

/// `target` in every module of `modules` whose tables (in `view`) have the
/// line, in the modules' order, each code range's start a place.
pub(crate) fn resolve(target: &SourceTarget, modules: &BreakpointModules<'_>, view: &SymbolsView) -> SourceResolution {
    // The pause's own modules: the kernel's stop everywhere; the running
    // process's in its space, when the process list names it.
    let running = |module: &SymModule| match (module.space, modules.elsewhere.current) {
        (ModuleSpace::User, Some(cr3)) => PlaceSpace::Process { cr3 },
        (ModuleSpace::User, None) | (ModuleSpace::Kernel, _) => PlaceSpace::Everywhere,
    };
    let pause = modules.index.modules.iter().map(|module| Placed { module, space: running(module) });
    let elsewhere = modules
        .elsewhere
        .modules
        .iter()
        .map(|found| Placed { module: &found.module, space: PlaceSpace::Process { cr3: found.cr3 } });
    let mut places = Vec::new();
    let mut loading = Vec::new();
    for Placed { module, space } in pause.chain(elsewhere) {
        let Some(identity) = module.identity.known() else {
            continue;
        };
        match view.state(identity) {
            Some(ModuleSymbols::Pending | ModuleSymbols::Downloading { .. }) => {
                if !loading.contains(&module.name) {
                    loading.push(module.name.clone());
                }
            }
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
                space,
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
/// (`modules`, none before a first pause), each address once.
pub(crate) fn applied_set(
    targets: &[BreakpointTarget],
    modules: Option<&BreakpointModules<'_>>,
    view: &SymbolsView,
) -> AppliedBreakpoints {
    let mut set: Vec<CodeBreakpoint> = Vec::new();
    let mut add = |breakpoint: CodeBreakpoint| {
        if !set.contains(&breakpoint) {
            set.push(breakpoint);
        }
    };
    // Every address that stops everywhere, and the spaces of every one that
    // does not; an address both kinds name stops everywhere.
    let mut everywhere: Vec<u64> = Vec::new();
    let mut scopes: Vec<ScopedAddress> = Vec::new();
    for target in targets {
        match target {
            BreakpointTarget::Address(breakpoint) => {
                add(*breakpoint);
                if let CodeBreakpoint::Linear(linear) = breakpoint {
                    everywhere.push(*linear);
                }
            }
            BreakpointTarget::Source(source) => {
                let Some(modules) = modules else {
                    continue;
                };
                match resolve(source, modules, view) {
                    SourceResolution::Resolved { places } => {
                        for place in places {
                            add(CodeBreakpoint::Linear(place.address));
                            match place.space {
                                PlaceSpace::Everywhere => everywhere.push(place.address),
                                PlaceSpace::Process { cr3 } => match scopes.iter_mut().find(|scope| scope.linear == place.address) {
                                    Some(scope) => {
                                        if !scope.spaces.contains(&cr3) {
                                            scope.spaces.push(cr3);
                                        }
                                    }
                                    None => scopes.push(ScopedAddress { linear: place.address, spaces: vec![cr3] }),
                                },
                            }
                        }
                    }
                    SourceResolution::Pending(_) => {}
                }
            }
        }
    }
    scopes.retain(|scope| !everywhere.contains(&scope.linear));
    AppliedBreakpoints { set, scopes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::inspect::{ModuleSpace, SymModule};
    use crate::debugger::windows::ProcessModule;
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
        ResolvedPlace { address, module: module.into(), line, space: PlaceSpace::Everywhere }
    }

    fn resolve(target: &SourceTarget, index: &SymbolIndex, view: &SymbolsView) -> SourceResolution {
        super::resolve(target, &BreakpointModules { index, elsewhere: &ModulesElsewhere::default() }, view)
    }

    fn applied_set(targets: &[BreakpointTarget], index: Option<&SymbolIndex>, view: &SymbolsView) -> Vec<CodeBreakpoint> {
        let modules = index.map(|index| BreakpointModules { index, elsewhere: &NOWHERE });
        super::applied_set(targets, modules.as_ref(), view).set
    }

    static NOWHERE: ModulesElsewhere = ModulesElsewhere { current: None, modules: Vec::new() };

    const PROGRAM: u64 = 0x1_4000_0000;
    const RUNNING: u64 = 0x494_F000;
    const SECOND: u64 = 0x3B0_0000;
    const MOVED: u64 = 0x5C0_0000;

    fn user(name: &str, base: u64, byte: u8) -> SymModule {
        SymModule { space: ModuleSpace::User, ..module(name, base, byte) }
    }

    fn in_process(cr3: u64, base: u64) -> ProcessModule {
        ProcessModule { cr3, process: "prog.exe".into(), module: user("prog.exe", base, 3) }
    }

    /// A pause's modules, the owner's modules in other processes, and the
    /// symbols loaded for them.
    struct Pause {
        index: SymbolIndex,
        elsewhere: ModulesElsewhere,
        view: SymbolsView,
    }

    /// `prog.exe` runs, and two more copies of it: one at the same base, one
    /// moved; `a.sys` is a driver.
    fn program_three_times() -> Pause {
        let index = SymbolIndex { modules: vec![module("a.sys", A, 1), user("prog.exe", PROGRAM, 3)], kernel: None };
        let elsewhere = ModulesElsewhere {
            current: Some(RUNNING),
            modules: vec![in_process(SECOND, PROGRAM), in_process(MOVED, PROGRAM + 0x10_0000)],
        };
        let mut view = SymbolsView::default();
        loaded(&mut view, "a.sys", 1, r"d:\b\src\io\read.c");
        loaded(&mut view, "prog.exe", 3, r"d:\b\src\io\read.c");
        Pause { index, elsewhere, view }
    }

    #[test]
    fn a_user_line_resolves_in_every_process_running_its_module_and_a_kernel_line_everywhere() {
        let Pause { index, elsewhere, view } = program_three_times();
        let modules = BreakpointModules { index: &index, elsewhere: &elsewhere };
        let process = |cr3| PlaceSpace::Process { cr3 };
        assert_eq!(
            super::resolve(&source("read.c", 10), &modules, &view),
            SourceResolution::Resolved {
                places: vec![
                    place(A + 0x1100, "a.sys", 10),
                    ResolvedPlace { space: process(RUNNING), ..place(PROGRAM + 0x1100, "prog.exe", 10) },
                    ResolvedPlace { space: process(SECOND), ..place(PROGRAM + 0x1100, "prog.exe", 10) },
                    ResolvedPlace { space: process(MOVED), ..place(PROGRAM + 0x10_1100, "prog.exe", 10) },
                ]
            }
        );
    }

    #[test]
    fn a_user_place_fires_only_in_its_processes_and_a_typed_address_everywhere() {
        let Pause { index, elsewhere, view } = program_three_times();
        let modules = BreakpointModules { index: &index, elsewhere: &elsewhere };
        let line = [BreakpointTarget::Source(source("read.c", 10))];
        let applied = super::applied_set(&line, Some(&modules), &view);
        assert_eq!(
            applied.set,
            [CodeBreakpoint::Linear(A + 0x1100), CodeBreakpoint::Linear(PROGRAM + 0x1100), CodeBreakpoint::Linear(PROGRAM + 0x10_1100)]
        );
        assert_eq!(
            applied.scopes,
            [
                ScopedAddress { linear: PROGRAM + 0x1100, spaces: vec![RUNNING, SECOND] },
                ScopedAddress { linear: PROGRAM + 0x10_1100, spaces: vec![MOVED] },
            ],
            "the driver's line is global"
        );
        let typed = [line[0].clone(), BreakpointTarget::Address(CodeBreakpoint::Linear(PROGRAM + 0x1100))];
        let applied = super::applied_set(&typed, Some(&modules), &view);
        assert_eq!(
            applied.scopes,
            [ScopedAddress { linear: PROGRAM + 0x10_1100, spaces: vec![MOVED] }],
            "an address the owner typed stops in every process"
        );
    }

    #[test]
    fn a_user_line_with_no_running_process_known_is_global() {
        let Pause { index, view, .. } = program_three_times();
        let modules = BreakpointModules { index: &index, elsewhere: &NOWHERE };
        let applied = super::applied_set(&[BreakpointTarget::Source(source("read.c", 10))], Some(&modules), &view);
        assert!(applied.scopes.is_empty(), "no Windows process list: nothing to scope by");
        assert_eq!(applied.set.len(), 2);
    }

    /// Other processes are searched only while a source line is set, and
    /// only for the binaries the owner's projects hold.
    #[test]
    fn the_projects_binaries_are_looked_for_only_while_a_line_is_set() {
        let mut files = rusty_box_symbols::memory_store::MemoryStore::default();
        let binary = rusty_box_symbols_fixtures::pe::pe64_file(&rusty_box_symbols_fixtures::pe::Pe64Spec {
            time_date_stamp: 1,
            size_of_image: 0x2000,
            image_base: PROGRAM,
            sections: Vec::new(),
            exception: None,
            base_relocations: None,
            import_address_table: None,
        });
        files.insert(std::path::PathBuf::from("build").join("rbdemo.exe"), binary);
        let folders = [rusty_box_symbols::projects::ProjectFolders { name: "rbdemo".into(), symbols: vec!["build".into()] }];
        let projects = rusty_box_symbols::projects::ProjectIndex::build(&mut files, &folders).expect("indexed");
        let address = BreakpointTarget::Address(CodeBreakpoint::Linear(0x1000));
        assert!(binaries_to_find(&[address.clone()], &projects).is_empty(), "no source line: nothing to look for");
        let line = BreakpointTarget::Source(source("rbdemo.c", 92));
        assert_eq!(binaries_to_find(&[address, line], &projects), ["rbdemo.exe"]);
    }

    #[test]
    fn a_hit_is_taken_only_in_the_spaces_of_its_address() {
        let scopes = [ScopedAddress { linear: PROGRAM + 0x1100, spaces: vec![RUNNING, SECOND] }];
        assert!(admits(&scopes, PROGRAM + 0x1100, RUNNING));
        assert!(admits(&scopes, PROGRAM + 0x1100, SECOND | 0x18), "CR3's flag bits are not its address space");
        assert!(!admits(&scopes, PROGRAM + 0x1100, MOVED));
        assert!(admits(&scopes, A + 0x1100, MOVED), "an address with no scope stops everywhere");
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
