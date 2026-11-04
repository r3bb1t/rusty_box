//! Breakpoints on source lines (spec §7.1): a `file:line` the owner types or
//! clicks is kept as typed and resolved against every module of the pause
//! whose tables have the line, into one linear breakpoint per code range
//! start; until one does, it is pending and says why.

use std::sync::{Arc, Weak};

use rusty_box::cpu::CodeBreakpoint;
use rusty_box_symbols::identity::{ModuleIdentity, PeImageId};
use rusty_box_symbols::projects::ProjectIndex;
use rusty_box_symbols::tables::ResolvedLine;

use super::inspect::{DebugSnapshot, ModuleSpace, SymModule};
use super::module_identity::BuildAt;
pub(crate) use super::inspect::SymbolIndex;
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
/// and where it stops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedPlace {
    pub(crate) address: u64,
    pub(crate) module: String,
    pub(crate) line: u32,
    pub(crate) space: PlaceSpace,
}

/// A user module's image as a place in it is bound to: where it is loaded,
/// and which build it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlacedImage {
    pub(crate) base: u64,
    pub(crate) image: PeImageId,
}

/// Where a place stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceSpace {
    /// Kernel code, which every address space maps; or a user module with no
    /// PE build to tell it by.
    Everywhere,
    /// User code of a PE image: it stops in whichever process has that build
    /// loaded at that base, a process started after the pause included, and
    /// in no process where another program sits at the same address.
    Image(PlacedImage),
}

/// What a source breakpoint is resolved against: the pause's modules (the
/// kernel's and those of the process the panes stand in), and the owner's
/// modules found in every other process.
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

/// A breakpoint address that stops only where one of `images` is loaded; an
/// address without one stops wherever it is reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScopedAddress {
    pub(crate) linear: u64,
    pub(crate) images: Vec<PlacedImage>,
}

/// The breakpoints the machine is given, and the addresses of them that
/// stop only where some program is loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AppliedBreakpoints {
    pub(crate) set: Vec<CodeBreakpoint>,
    pub(crate) scopes: Vec<ScopedAddress>,
}

/// The file names of the binaries a pause looks for in every process: the
/// owner's projects' binaries, while any of the owner's breakpoints is a
/// source line; none otherwise, so a pause without one reads no other
/// process.
pub(crate) fn binaries_to_find<'a>(targets: &[BreakpointTarget], projects: &'a ProjectIndex) -> &'a [String] {
    if targets.iter().any(|target| matches!(target, BreakpointTarget::Source(_))) {
        projects.binary_names()
    } else {
        &[]
    }
}

/// Whether a stop at `scope`'s address is one of the owner's: the image
/// loaded at one of its places' bases (`build_at` reads it from the
/// stopped processor's address space) is that place's build. Headers that
/// do not read rule the program out of nothing, so they stop it.
pub(crate) fn admits<F>(scope: &ScopedAddress, mut build_at: F) -> bool
where
    F: FnMut(u64) -> BuildAt,
{
    scope.images.iter().any(|placed| match build_at(placed.base) {
        BuildAt::Image(found) => found == placed.image,
        BuildAt::NoImage => false,
        BuildAt::Unreadable => true,
    })
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

/// One build's answer for a line: looked up once, however many processes
/// load the build.
struct BuildLines<'a> {
    identity: &'a ModuleIdentity,
    lines: Vec<ResolvedLine>,
}

/// Where `module`'s places stop: a user module's PE build at its base, the
/// kernel's everywhere.
fn space_of(module: &SymModule, identity: &ModuleIdentity) -> PlaceSpace {
    match (module.space, identity) {
        (ModuleSpace::User, ModuleIdentity::Pe { image, .. }) => {
            PlaceSpace::Image(PlacedImage { base: module.base, image: *image })
        }
        (ModuleSpace::User, ModuleIdentity::Elf { .. }) | (ModuleSpace::Kernel, _) => PlaceSpace::Everywhere,
    }
}

/// `target` in every module of `modules` whose tables (in `view`) have the
/// line, in the modules' order, each code range's start a place, and each
/// place once: a build loaded at the same base in several processes is one
/// place.
pub(crate) fn resolve(target: &SourceTarget, modules: &BreakpointModules<'_>, view: &SymbolsView) -> SourceResolution {
    let elsewhere = modules.elsewhere.modules.iter().map(|found| &found.module);
    let mut places: Vec<ResolvedPlace> = Vec::new();
    let mut loading = Vec::new();
    let mut looked_up: Vec<BuildLines<'_>> = Vec::new();
    for module in modules.index.modules.iter().chain(elsewhere) {
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
        let known = match looked_up.iter().position(|build| build.identity == identity) {
            Some(known) => known,
            None => {
                let Some(tables) = view.tables(identity) else {
                    continue;
                };
                looked_up.push(BuildLines { identity, lines: tables.lines_of(&target.file, target.line) });
                looked_up.len().saturating_sub(1)
            }
        };
        let Some(build) = looked_up.get(known) else {
            continue;
        };
        let space = space_of(module, identity);
        for resolved in &build.lines {
            for range in &resolved.ranges {
                let place = ResolvedPlace {
                    address: module.base.wrapping_add(range.start.0),
                    module: module.name.clone(),
                    line: resolved.line,
                    space,
                };
                if !places.contains(&place) {
                    places.push(place);
                }
            }
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
    Resolutions::of(targets, modules, view).applied
}

/// A source target and how it resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceResolved {
    target: SourceTarget,
    resolution: SourceResolution,
}

/// What a set of resolutions was worked out under.
#[derive(Clone, Debug)]
struct ResolvedUnder {
    targets: Vec<BreakpointTarget>,
    /// The pause's snapshot; none before a first pause. Held weakly, which
    /// keeps its allocation, so no later snapshot shares its address.
    pause: Option<Weak<DebugSnapshot>>,
    generation: u64,
}

impl ResolvedUnder {
    /// Whether these are `targets` under the pause `snapshot` and `view`.
    fn same(&self, targets: &[BreakpointTarget], snapshot: Option<&Arc<DebugSnapshot>>, view: &SymbolsView) -> bool {
        let same_pause = match (&self.pause, snapshot) {
            (Some(held), Some(shown)) => std::ptr::eq(held.as_ptr(), Arc::as_ptr(shown)),
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        same_pause && self.generation == view.generation && self.targets == targets
    }
}

/// Every source target's resolution under one pause and one symbol view,
/// and the breakpoints the targets come to. The shell reads them every
/// frame; they are worked out again only when the targets, the pause or the
/// view change.
#[derive(Clone, Debug, Default)]
pub(crate) struct Resolutions {
    under: Option<ResolvedUnder>,
    sources: Vec<SourceResolved>,
    applied: AppliedBreakpoints,
}

impl Resolutions {
    /// `targets` resolved under `modules`, none before a first pause.
    pub(crate) fn of(targets: &[BreakpointTarget], modules: Option<&BreakpointModules<'_>>, view: &SymbolsView) -> Resolutions {
        let mut sources: Vec<SourceResolved> = Vec::new();
        if let Some(modules) = modules {
            for target in targets {
                if let BreakpointTarget::Source(source) = target {
                    if !sources.iter().any(|known| known.target == *source) {
                        sources.push(SourceResolved { target: source.clone(), resolution: resolve(source, modules, view) });
                    }
                }
            }
        }
        let applied = applied_of(targets, &sources);
        Resolutions { under: None, sources, applied }
    }

    /// Brings them up to `targets` under the pause `snapshot` and `view`,
    /// worked out again only when one of those changed.
    pub(crate) fn follow(&mut self, targets: &[BreakpointTarget], snapshot: Option<&Arc<DebugSnapshot>>, view: &SymbolsView) {
        if self.under.as_ref().is_some_and(|under| under.same(targets, snapshot, view)) {
            return;
        }
        let modules = snapshot.map(|shown| BreakpointModules::of(shown));
        *self = Resolutions::of(targets, modules.as_ref(), view);
        self.under =
            Some(ResolvedUnder { targets: targets.to_vec(), pause: snapshot.map(Arc::downgrade), generation: view.generation });
    }

    /// How `target` resolved; `None` before a first pause.
    pub(crate) fn source(&self, target: &SourceTarget) -> Option<&SourceResolution> {
        self.sources.iter().find(|known| known.target == *target).map(|known| &known.resolution)
    }

    /// The breakpoints the targets come to.
    pub(crate) fn applied(&self) -> &AppliedBreakpoints {
        &self.applied
    }
}

/// The breakpoints `targets` come to, their source lines resolved as
/// `sources` says: each address once, and the addresses that stop only
/// where some program is loaded with the images they stop in. An address
/// that also stops everywhere (one the owner typed, or kernel code) has no
/// scope.
fn applied_of(targets: &[BreakpointTarget], sources: &[SourceResolved]) -> AppliedBreakpoints {
    let mut set: Vec<CodeBreakpoint> = Vec::new();
    let mut add = |breakpoint: CodeBreakpoint| {
        if !set.contains(&breakpoint) {
            set.push(breakpoint);
        }
    };
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
                let resolution = sources.iter().find(|known| known.target == *source).map(|known| &known.resolution);
                let Some(SourceResolution::Resolved { places }) = resolution else {
                    continue;
                };
                for place in places {
                    add(CodeBreakpoint::Linear(place.address));
                    match place.space {
                        PlaceSpace::Everywhere => everywhere.push(place.address),
                        PlaceSpace::Image(image) => match scopes.iter_mut().find(|scope| scope.linear == place.address) {
                            Some(scope) => {
                                if !scope.images.contains(&image) {
                                    scope.images.push(image);
                                }
                            }
                            None => scopes.push(ScopedAddress { linear: place.address, images: vec![image] }),
                        },
                    }
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
    use crate::debugger::inspect::{AddressSpace, CodeTarget, GuestAddress, MemoryWindow, ViewContext, Viewing};
    use crate::debugger::windows::{ProcessModule, ReadThrough, SystemListing, SystemView};
    use crate::debugger::module_identity::GuestIdentity;
    use crate::symbols::hub::{ModuleEntry, ModuleSymbols};
    use rusty_box_symbols::identity::{Guid, PdbId};
    use rusty_box_symbols::service::SymbolSource;
    use rusty_box_symbols::tables::SymbolTables;
    use rusty_box_symbols_fixtures::pdb::{build_pdb, FixtureLine, FixtureProcedure, PdbFixture};
    use rusty_box_symbols_fixtures::pe::PAGE_ALIGNED;

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

    static NOWHERE: ModulesElsewhere = ModulesElsewhere { names: Vec::new(), modules: Vec::new() };

    const PROGRAM: u64 = 0x1_4000_0000;
    const SECOND: u64 = 0x3B0_0000;
    const MOVED: u64 = 0x5C0_0000;

    fn user(name: &str, base: u64, byte: u8) -> SymModule {
        SymModule { space: ModuleSpace::User, ..module(name, base, byte) }
    }

    fn in_process(cr3: u64, base: u64) -> ProcessModule {
        ProcessModule { cr3, process: "prog.exe".into(), module: user("prog.exe", base, 3) }
    }

    /// `prog.exe`'s build (fixture byte 3) loaded at `base`.
    fn program_at(base: u64) -> PlacedImage {
        PlacedImage { base, image: PeImageId { time_date_stamp: 3, size_of_image: 0x10_0000 } }
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
            names: vec!["prog.exe".into()],
            modules: vec![in_process(SECOND, PROGRAM), in_process(MOVED, PROGRAM + 0x10_0000)],
        };
        let mut view = SymbolsView::default();
        loaded(&mut view, "a.sys", 1, r"d:\b\src\io\read.c");
        loaded(&mut view, "prog.exe", 3, r"d:\b\src\io\read.c");
        Pause { index, elsewhere, view }
    }

    #[test]
    fn a_user_line_resolves_once_per_base_its_build_is_loaded_at_and_a_kernel_line_everywhere() {
        let Pause { index, elsewhere, view } = program_three_times();
        let modules = BreakpointModules { index: &index, elsewhere: &elsewhere };
        let image = |base| PlaceSpace::Image(program_at(base));
        assert_eq!(
            super::resolve(&source("read.c", 10), &modules, &view),
            SourceResolution::Resolved {
                places: vec![
                    place(A + 0x1100, "a.sys", 10),
                    ResolvedPlace { space: image(PROGRAM), ..place(PROGRAM + 0x1100, "prog.exe", 10) },
                    ResolvedPlace { space: image(PROGRAM + 0x10_0000), ..place(PROGRAM + 0x10_1100, "prog.exe", 10) },
                ]
            },
            "the copy at the running one's base is the same place"
        );
    }

    #[test]
    fn a_user_place_stops_where_its_build_is_loaded_and_a_typed_address_everywhere() {
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
                ScopedAddress { linear: PROGRAM + 0x1100, images: vec![program_at(PROGRAM)] },
                ScopedAddress { linear: PROGRAM + 0x10_1100, images: vec![program_at(PROGRAM + 0x10_0000)] },
            ],
            "the driver's line is global"
        );
        let typed = [line[0].clone(), BreakpointTarget::Address(CodeBreakpoint::Linear(PROGRAM + 0x1100))];
        let applied = super::applied_set(&typed, Some(&modules), &view);
        assert_eq!(
            applied.scopes,
            [ScopedAddress { linear: PROGRAM + 0x10_1100, images: vec![program_at(PROGRAM + 0x10_0000)] }],
            "an address the owner typed stops in every process"
        );
    }

    /// The scope names the build, not a process: it needs no process list,
    /// and holds for a process started after the pause.
    #[test]
    fn a_user_line_is_scoped_by_its_build_without_a_process_list() {
        let Pause { index, view, .. } = program_three_times();
        let modules = BreakpointModules { index: &index, elsewhere: &NOWHERE };
        let applied = super::applied_set(&[BreakpointTarget::Source(source("read.c", 10))], Some(&modules), &view);
        assert_eq!(applied.scopes, [ScopedAddress { linear: PROGRAM + 0x1100, images: vec![program_at(PROGRAM)] }]);
        assert_eq!(applied.set.len(), 2);
    }

    /// A thread view stands in another process: its modules are the pause's,
    /// and the running process's copy of the program is found elsewhere. The
    /// line binds in both, by build.
    #[test]
    fn a_thread_views_pause_keeps_the_running_programs_line() {
        let Pause { view, .. } = program_three_times();
        let viewed = SymbolIndex { modules: vec![module("a.sys", A, 1), user("other.exe", PROGRAM, 4)], kernel: None };
        let elsewhere = ModulesElsewhere { names: vec!["prog.exe".into()], modules: vec![in_process(SECOND, PROGRAM)] };
        let modules = BreakpointModules { index: &viewed, elsewhere: &elsewhere };
        let applied = super::applied_set(&[BreakpointTarget::Source(source("read.c", 10))], Some(&modules), &view);
        assert_eq!(applied.set, [CodeBreakpoint::Linear(A + 0x1100), CodeBreakpoint::Linear(PROGRAM + 0x1100)]);
        assert_eq!(
            applied.scopes,
            [ScopedAddress { linear: PROGRAM + 0x1100, images: vec![program_at(PROGRAM)] }],
            "other.exe at the same base is not the program"
        );
    }

    /// The shell asks for the resolutions every frame: they are worked out
    /// again when the targets, the pause or the symbols change, and kept
    /// otherwise.
    #[test]
    fn resolutions_are_kept_until_the_targets_the_pause_or_the_symbols_change() {
        let Pause { index, view, .. } = program_three_times();
        let pause = Arc::new(pause_over(index.clone()));
        let line = [BreakpointTarget::Source(source("read.c", 10))];
        let mut resolutions = Resolutions::default();
        resolutions.follow(&line, None, &view);
        assert_eq!(resolutions.source(&source("read.c", 10)), None, "before a pause");
        resolutions.follow(&line, Some(&pause), &view);
        assert!(matches!(resolutions.source(&source("read.c", 10)), Some(SourceResolution::Resolved { .. })));
        assert_eq!(resolutions.applied().set.len(), 2);
        // The same generation is the same view: nothing is worked out again.
        let mut emptied = SymbolsView { generation: view.generation, ..SymbolsView::default() };
        resolutions.follow(&line, Some(&pause), &emptied);
        assert_eq!(resolutions.applied().set.len(), 2, "kept");
        emptied.generation = view.generation.wrapping_add(1);
        resolutions.follow(&line, Some(&pause), &emptied);
        assert_eq!(resolutions.applied().set, [], "the new view has no tables");
        let another = Arc::new(pause_over(index));
        resolutions.follow(&line, Some(&another), &view);
        assert_eq!(resolutions.applied().set.len(), 2, "a new pause");
        let typed = [line[0].clone(), BreakpointTarget::Address(CodeBreakpoint::Linear(0x1000))];
        resolutions.follow(&typed, Some(&another), &view);
        assert_eq!(resolutions.applied().set.len(), 3, "new targets");
    }

    /// A pause whose modules are `symbols`.
    fn pause_over(symbols: SymbolIndex) -> DebugSnapshot {
        DebugSnapshot {
            cpus: Vec::new(),
            cpu: 0,
            code: Vec::new(),
            code_at: CodeTarget::Rip,
            rip: 0,
            frames: crate::debugger::frames::FramesRead::default(),
            memory: MemoryWindow { start: GuestAddress { space: AddressSpace::Linear, value: 0 }, rows: Vec::new() },
            stack: Vec::new(),
            process: None,
            asked: SystemView::Off,
            system: SystemListing::NotAsked,
            symbols,
            memory_space: None,
            context: ViewContext::Live,
            viewing: Viewing::Live,
            layout: None,
            layouts: None,
            through: ReadThrough::ProcessorTables,
            vmcs: None,
            elsewhere: ModulesElsewhere::default(),
        }
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
            section_alignment: PAGE_ALIGNED,
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

    /// A hit stops where the program's build is loaded at its base (a
    /// restarted program's new process too), and runs on where another
    /// program sits at the same address.
    #[test]
    fn a_hit_is_taken_where_its_programs_build_is_loaded() {
        let program = program_at(PROGRAM);
        let scope = ScopedAddress { linear: PROGRAM + 0x1100, images: vec![program] };
        let image_only_at = |base| if base == PROGRAM { BuildAt::Image(program.image) } else { BuildAt::NoImage };
        assert!(admits(&scope, image_only_at));
        let other = PeImageId { time_date_stamp: 4, size_of_image: 0x10_0000 };
        assert!(!admits(&scope, |_| BuildAt::Image(other)), "another program at the same base");
        assert!(!admits(&scope, |_| BuildAt::NoImage), "code that is no image's");
        assert!(admits(&scope, |_| BuildAt::Unreadable), "headers that do not read rule nothing out");
        let two = ScopedAddress { linear: PROGRAM + 0x1100, images: vec![PlacedImage { base: PROGRAM, image: other }, program] };
        assert!(admits(&two, |_| BuildAt::Image(program.image)), "either of the owner's programs there");
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
