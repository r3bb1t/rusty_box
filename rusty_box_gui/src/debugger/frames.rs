//! The call stack of a pause and the picked frame's variables and watches
//! (spec §8, §9), read by the run thread with the snapshot.

use std::collections::{BTreeMap, BTreeSet};

use rusty_box::cpu::{CodeSize, CpuDebugView};
use rusty_box_symbols::identity::ModuleIdentity;

use super::guest::{GuestMemory, Root};
use super::image_memory::ImageBacked;
use super::inspect::{linear_of, DebugSnapshot, SymModule, SymbolIndex, Viewing, WOW64_CODE_SELECTOR};
use super::threads::SavedRegisters;
use super::unwind::{
    caller_of, unwind, CodeWidth, Frame, FrameFound, FrameRegisters, Stack, StackEnd, UnwindModule, Unwinding, RBP, RSP,
};
use super::variables::{frame_scope, locals_of, FrameContext, Innermost, PathRoot, ValuePath, VariableRow};
use super::watch::{read_tip, read_watch, unread_watch};
use crate::symbols::hub::{ImageKey, SymbolsView};

/// What the variables panes ask of a pause: the picked frame, the watches,
/// which values are expanded, and how many pages of each expanded array.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InspectRequest {
    pub(crate) frame: usize,
    pub(crate) watches: Vec<String>,
    pub(crate) expanded: BTreeSet<ValuePath>,
    pub(crate) pages: BTreeMap<ValuePath, u32>,
    /// The expression a click in the source view asked to see.
    pub(crate) tip: Option<String>,
}

/// What the value tip's expression read as.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TipRead {
    /// No tip was asked.
    None,
    Rows(WatchRead),
    /// The clicked word `text` names no variable: the tip closes.
    NoVariable { text: String },
}

/// The part of an `InspectRequest` the shell sends each frame; the frame is
/// picked on its own (`RunControl::select_frame`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InspectAsk {
    pub(crate) watches: Vec<String>,
    pub(crate) expanded: BTreeSet<ValuePath>,
    pub(crate) pages: BTreeMap<ValuePath, u32>,
}

/// The picked frame's variables, or why there are none to show.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LocalsRead {
    /// No frame's variables were read (16-bit code; nothing asked yet).
    NotAsked,
    /// The frame's code is in no module of the pause.
    NoModule,
    /// The module's symbols are loading, or were not found.
    NoSymbols { module: String },
    /// No function's records hold the frame's code.
    NoFunction,
    Unreadable { why: String },
    Rows(Vec<VariableRow>),
}

/// One watch, as typed, and its rows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WatchRead {
    pub(crate) text: String,
    pub(crate) rows: Vec<VariableRow>,
}

/// The call stack of a pause, the picked frame, and what was read in it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FramesRead {
    pub(crate) frames: Vec<Frame>,
    pub(crate) end: StackEnd,
    /// The frame the panes show: the request's, or 0 when the stack is
    /// shorter.
    pub(crate) selected: usize,
    pub(crate) locals: LocalsRead,
    pub(crate) watches: Vec<WatchRead>,
    pub(crate) tip: TipRead,
    /// Modules whose tables the read wanted and the symbol view did not have.
    pub(crate) awaiting: Vec<ModuleIdentity>,
    /// Binaries the read wanted, for bytes the guest had not paged in, and
    /// the symbol view did not have yet.
    pub(crate) awaiting_images: Vec<ImageKey>,
}

impl Default for FramesRead {
    fn default() -> Self {
        FramesRead {
            frames: Vec::new(),
            end: StackEnd::Top,
            selected: 0,
            locals: LocalsRead::NotAsked,
            watches: Vec::new(),
            tip: TipRead::None,
            awaiting: Vec::new(),
            awaiting_images: Vec::new(),
        }
    }
}

/// Where a stack is read from: the address space, the code's width (`None`
/// for 16-bit code, which is not unwound), the innermost frame's registers,
/// and its XMM registers when they are known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FramesAt {
    pub(crate) root: Root,
    pub(crate) width: Option<CodeWidth>,
    pub(crate) start: FrameRegisters,
    pub(crate) xmm: Option<[[u8; 16]; 16]>,
}

/// A thread seen from inside, from its trap frame: RIP, RSP and RBP are
/// saved on every kernel entry; the other registers only as the entry kind
/// requires, so they are not known to be the thread's.
pub(crate) fn frames_at_thread(saved: &SavedRegisters, cr3: u64) -> FramesAt {
    let mut registers = [None; 16];
    if let Some(slot) = registers.get_mut(RSP) {
        *slot = Some(saved.rsp);
    }
    if let Some(slot) = registers.get_mut(RBP) {
        *slot = Some(saved.rbp);
    }
    let width = if saved.cs == WOW64_CODE_SELECTOR { CodeWidth::Protected32 } else { CodeWidth::Long64 };
    FramesAt {
        root: Root::Process(cr3),
        width: Some(width),
        start: FrameRegisters { rip: saved.rip, rsp: saved.rsp, registers },
        xmm: None,
    }
}

/// The processor itself: every register known, RIP and RSP as linear
/// addresses at its code's width.
pub(crate) fn frames_at_view(view: &CpuDebugView) -> FramesAt {
    let at = &view.registers;
    let registers = [
        at.rax, at.rcx, at.rdx, at.rbx, at.rsp, at.rbp, at.rsi, at.rdi, at.r8, at.r9, at.r10, at.r11, at.r12, at.r13, at.r14,
        at.r15,
    ]
    .map(Some);
    let width = match view.code_size {
        CodeSize::Bits64 => Some(CodeWidth::Long64),
        CodeSize::Bits32 => Some(CodeWidth::Protected32),
        CodeSize::Bits16 => None,
    };
    FramesAt {
        root: Root::Live,
        width,
        start: FrameRegisters {
            rip: linear_of(view.segment_bases.cs, at.rip, view.code_size),
            rsp: linear_of(view.segment_bases.ss, at.rsp, view.code_size),
            registers,
        },
        xmm: Some(at.xmm),
    }
}

/// Where `snapshot`'s panes stand: the thread it looks from, else its
/// processor.
pub(crate) fn frames_at(snapshot: &DebugSnapshot) -> FramesAt {
    match &snapshot.viewing {
        Viewing::Thread(thread) => frames_at_thread(&thread.saved, thread.cr3),
        Viewing::Live | Viewing::Running { .. } | Viewing::Miss(_) => match snapshot.cpus.get(snapshot.cpu) {
            Some(view) => frames_at_view(view),
            None => FramesAt {
                root: Root::Live,
                width: None,
                start: FrameRegisters { rip: snapshot.rip, rsp: 0, registers: [None; 16] },
                xmm: None,
            },
        },
    }
}

/// Where a caller is looked for: the address space, the modules its code
/// may be in, and the code's width.
#[derive(Clone, Copy)]
pub(crate) struct CallerOf<'a> {
    pub(crate) root: Root,
    pub(crate) modules: &'a [SymModule],
    pub(crate) width: CodeWidth,
}

/// The frame `frame` returns to, the modules' bytes the guest has not paged
/// in read from their binaries: where Step Out runs to.
pub(crate) fn caller_frame<G: GuestMemory>(
    memory: &mut G,
    of: &CallerOf<'_>,
    symbols: Option<&SymbolsView>,
    frame: &Frame,
) -> Option<Frame> {
    let mut memory = ImageBacked::new(memory, of.modules, symbols);
    let modules = unwind_modules(of.modules);
    caller_of(&mut memory, &Unwinding { root: of.root, modules: &modules, width: of.width }, frame).ok()
}

fn unwind_modules(modules: &[SymModule]) -> Vec<UnwindModule> {
    modules.iter().map(|module| UnwindModule { base: module.base, size: module.size }).collect()
}

/// The stack from `at` and the frame `request` picks (the innermost when
/// the stack is shorter).
pub(crate) fn read_frames<G: GuestMemory>(
    memory: &mut G,
    at: &FramesAt,
    index: &SymbolIndex,
    symbols: Option<&SymbolsView>,
    request: &InspectRequest,
) -> FramesRead {
    // A module's bytes the guest has not paged in are read from its binary.
    let mut memory = ImageBacked::new(memory, &index.modules, symbols);
    let stack = match at.width {
        Some(width) => {
            let modules = unwind_modules(&index.modules);
            unwind(&mut memory, &Unwinding { root: at.root, modules: &modules, width }, at.start)
        }
        None => Stack { frames: vec![Frame { registers: at.start, how: FrameFound::Registers }], end: StackEnd::SixteenBit },
    };
    let selected = if request.frame < stack.frames.len() { request.frame } else { 0 };
    let awaiting = stack
        .frames
        .get(selected)
        .and_then(|frame| index.module_at(frame.code_address()))
        .and_then(|module| module.identity.known())
        .filter(|identity| symbols.and_then(|view| view.tables(identity)).is_none())
        .map(|identity| vec![identity.clone()])
        .unwrap_or_default();
    let picked = at.width.and_then(|width| stack.frames.get(selected).map(|frame| Picked { frame, selected, width }));
    let read = match picked {
        Some(picked) => read_in_frame(&mut memory, at, index, symbols, &picked, request),
        None => FrameRead::unread(LocalsRead::NotAsked, request, "16-bit code's variables are not read"),
    };
    let awaiting_images = memory.into_awaited();
    FramesRead {
        frames: stack.frames,
        end: stack.end,
        selected,
        locals: read.locals,
        watches: read.watches,
        tip: read.tip,
        awaiting,
        awaiting_images,
    }
}

/// What was read in the picked frame.
struct FrameRead {
    locals: LocalsRead,
    watches: Vec<WatchRead>,
    tip: TipRead,
}

impl FrameRead {
    /// Nothing read: `locals` says why, and each watch and the tip, as one
    /// row, `why`.
    fn unread(locals: LocalsRead, request: &InspectRequest, why: &str) -> FrameRead {
        let mut watches = Vec::new();
        for (watch, text) in request.watches.iter().enumerate() {
            watches.push(unread_watch(PathRoot::Watch(watch), text, why));
        }
        let tip = match &request.tip {
            Some(text) => TipRead::Rows(unread_watch(PathRoot::Tip, text, why)),
            None => TipRead::None,
        };
        FrameRead { locals, watches, tip }
    }
}

/// The frame the panes show, its place on the stack, and its code's width.
struct Picked<'a> {
    frame: &'a Frame,
    selected: usize,
    width: CodeWidth,
}

/// The picked frame's variables and the watches, when its module's tables
/// are loaded.
fn read_in_frame<G: GuestMemory>(
    memory: &mut G,
    at: &FramesAt,
    index: &SymbolIndex,
    symbols: Option<&SymbolsView>,
    picked: &Picked<'_>,
    request: &InspectRequest,
) -> FrameRead {
    let Some(module) = index.module_at(picked.frame.code_address()) else {
        return FrameRead::unread(LocalsRead::NoModule, request, "the frame's code is in no module");
    };
    let tables = module.identity.known().and_then(|identity| symbols.and_then(|view| view.tables(identity)));
    let Some(tables) = tables else {
        let why = format!("the symbols of {} are not loaded", module.name);
        return FrameRead::unread(LocalsRead::NoSymbols { module: module.name.clone() }, request, &why);
    };
    let context = FrameContext {
        frame: picked.frame,
        innermost: if picked.selected == 0 { Innermost::Yes } else { Innermost::Caller },
        xmm: at.xmm.as_ref(),
        root: at.root,
        width: picked.width,
        module_base: module.base,
        tables,
    };
    let scope = frame_scope(&context);
    let locals = locals_of(memory, &context, &scope, request);
    let in_scope = scope.as_ref().ok().and_then(Option::as_ref);
    let mut watches = Vec::new();
    for (watch, text) in request.watches.iter().enumerate() {
        watches.push(read_watch(memory, &context, in_scope, watch, text, request));
    }
    let tip = match &request.tip {
        Some(text) => read_tip(memory, &context, in_scope, text, request),
        None => TipRead::None,
    };
    FrameRead { locals, watches, tip }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::guest::fake::FakeGuest;
    use crate::debugger::inspect::{ModuleSpace, SymModule};
    use crate::debugger::module_identity::GuestIdentity;
    use crate::debugger::unwind::RBX;
    use crate::debugger::variables::{ShownValue, Signedness};
    use crate::symbols::hub::{ImageEntry, ImageKey, ImageState, ModuleEntry, ModuleSymbols};
    use rusty_box_symbols::identity::PeImageId;
    use rusty_box_symbols::module_file::ModuleFile;
    use rusty_box_symbols::service::SymbolSource;
    use rusty_box_symbols::tables::SymbolTables;
    use rusty_box_symbols_fixtures::pdb::{
        build_pdb, FixtureField, FixtureLine, FixtureLocal, FixturePlace, FixtureProcedure, FixtureScope, FixtureType,
        FixtureUdt, PdbFixture, UdtKind,
    };
    use rusty_box_symbols_fixtures::pe::{pe64_file, Directory, Pe64Spec, SpecSection, SECTION_CODE, SECTION_READ_ONLY};
    use std::sync::Arc;

    /// CodeView's `RSP`.
    const CV_RSP: u16 = 335;

    const CR3: u64 = 0x1000;
    const BASE: u64 = 0x1_4000_0000;
    const MAIN: u64 = BASE + 0x1000;
    const F: u64 = BASE + 0x1100;
    const STACK: u64 = 0x7_0000;

    /// A module with no exception table holding `main` and `f`, paused in
    /// `f`, a leaf called from `main + 5`.
    fn leaf_called_from_main() -> FakeGuest {
        let mut guest = FakeGuest::new(CR3);
        guest.write(CR3, BASE, &[0; 0x2000]);
        guest.write_u64(CR3, STACK, MAIN + 5);
        guest.write_u64(CR3, STACK + 8, 0);
        guest
    }

    fn index() -> SymbolIndex {
        SymbolIndex {
            modules: vec![SymModule {
                base: BASE,
                size: 0x1_0000,
                name: "flat.exe".into(),
                space: ModuleSpace::User,
                identity: GuestIdentity::Unnamed,
            }],
            kernel: None,
        }
    }

    fn registers_at(rip: u64, rsp: u64) -> FrameRegisters {
        FrameRegisters { rip, rsp, registers: [Some(0); 16] }
    }

    fn at_f(width: Option<CodeWidth>) -> FramesAt {
        FramesAt { root: Root::Live, width, start: registers_at(F, STACK), xmm: None }
    }

    fn request(frame: usize) -> InspectRequest {
        InspectRequest { frame, ..InspectRequest::default() }
    }

    #[test]
    fn the_stack_of_a_leaf_called_from_main_has_two_frames() {
        let read = read_frames(&mut leaf_called_from_main(), &at_f(Some(CodeWidth::Long64)), &index(), None, &request(0));
        assert_eq!(read.frames.len(), 2);
        assert_eq!(read.frames[0].how, FrameFound::Registers);
        assert_eq!(read.frames[1].registers.rip, MAIN + 5);
        assert_eq!(read.end, StackEnd::Top);
        assert_eq!(read.selected, 0);
    }

    #[test]
    fn a_selection_past_the_stack_falls_back_to_the_innermost_frame() {
        let read = read_frames(&mut leaf_called_from_main(), &at_f(Some(CodeWidth::Long64)), &index(), None, &request(7));
        assert_eq!(read.selected, 0);
        let read = read_frames(&mut leaf_called_from_main(), &at_f(Some(CodeWidth::Long64)), &index(), None, &request(1));
        assert_eq!(read.selected, 1);
    }

    #[test]
    fn a_module_whose_symbols_have_not_loaded_is_awaited() {
        let identity = ModuleIdentity::Pe {
            file_name: "flat.exe".into(),
            image: PeImageId { time_date_stamp: 1, size_of_image: 0x1_0000 },
            pdb: None,
        };
        let mut known = index();
        known.modules[0].identity = GuestIdentity::Known(identity.clone());
        let read = read_frames(&mut leaf_called_from_main(), &at_f(Some(CodeWidth::Long64)), &known, None, &request(0));
        assert_eq!(read.awaiting, [identity.clone()]);
        let read = read_frames(
            &mut leaf_called_from_main(),
            &at_f(Some(CodeWidth::Long64)),
            &known,
            Some(&SymbolsView::default()),
            &request(1),
        );
        assert_eq!(read.awaiting, [identity], "the caller's module, picked");
        assert_eq!(read.locals, LocalsRead::NoSymbols { module: "flat.exe".into() });
        let unnamed = read_frames(&mut leaf_called_from_main(), &at_f(Some(CodeWidth::Long64)), &index(), None, &request(0));
        assert!(unnamed.awaiting.is_empty(), "nothing names an unnamed module's tables");
        assert_eq!(unnamed.locals, LocalsRead::NoSymbols { module: "flat.exe".into() });
    }

    fn flat_identity() -> ModuleIdentity {
        ModuleIdentity::Pe {
            file_name: "flat.exe".into(),
            image: PeImageId { time_date_stamp: 1, size_of_image: 0x1_0000 },
            pdb: None,
        }
    }

    fn procedure(name: &str, offset: u32) -> FixtureProcedure {
        FixtureProcedure {
            name: name.into(),
            section: 1,
            offset,
            len: 0x40,
            file: r"C:\src\flat.c".into(),
            checksum: None,
            lines: vec![FixtureLine { offset: 0, line: 1 }],
        }
    }

    fn at_rsp(name: &str, ty: FixtureType) -> FixtureLocal {
        FixtureLocal { name: name.into(), ty, place: FixturePlace::RegRel { register: CV_RSP, offset: 0 } }
    }

    /// `main` holding `argc` and the structure `pair` at `[rsp]`, and `f`
    /// holding `x` there.
    fn flat_view() -> SymbolsView {
        let mut fixture = PdbFixture::new([5; 16], 1);
        fixture.procedures = vec![procedure("main", 0), procedure("f", 0x100)];
        fixture.types = vec![FixtureUdt {
            name: "Pair".into(),
            kind: UdtKind::Struct,
            size: 8,
            fields: vec![
                FixtureField { name: "a".into(), offset: 0, ty: FixtureType::I32 },
                FixtureField { name: "b".into(), offset: 4, ty: FixtureType::I32 },
            ],
        }];
        let scopes = [
            ScopeOf {
                procedure: "main",
                locals: vec![at_rsp("argc", FixtureType::I32), at_rsp("pair", FixtureType::Named("Pair".into()))],
            },
            ScopeOf { procedure: "f", locals: vec![at_rsp("x", FixtureType::I32)] },
        ];
        for scope in scopes {
            fixture.scopes.push(FixtureScope {
                procedure: scope.procedure.into(),
                debug_start: 0,
                debug_end: 0x40,
                parameters: None,
                frame: None,
                locals: scope.locals,
                inline_locals: Vec::new(),
                blocks: Vec::new(),
            });
        }
        let tables = SymbolTables::parse(&build_pdb(&fixture)).expect("the fixture parses");
        let mut view = SymbolsView::default();
        view.modules.insert(
            flat_identity(),
            ModuleEntry {
                state: ModuleSymbols::Loaded { from: SymbolSource::Store { path: "flat.pdb".into() } },
                tables: Some(Arc::new(tables)),
            },
        );
        view
    }

    /// A procedure of the fixture and its locals.
    struct ScopeOf {
        procedure: &'static str,
        locals: Vec<FixtureLocal>,
    }

    fn names(locals: &LocalsRead) -> Vec<String> {
        match locals {
            LocalsRead::Rows(rows) => rows.iter().map(|row| row.name.clone()).collect(),
            other => panic!("no rows: {other:?}"),
        }
    }

    #[test]
    fn the_selected_frames_locals_are_read_with_the_stack() {
        let mut known = index();
        known.modules[0].identity = GuestIdentity::Known(flat_identity());
        let view = flat_view();
        let at = at_f(Some(CodeWidth::Long64));
        let innermost = read_frames(&mut leaf_called_from_main(), &at, &known, Some(&view), &request(0));
        assert_eq!(names(&innermost.locals), ["x"]);
        assert!(innermost.awaiting.is_empty());
        let caller = read_frames(&mut leaf_called_from_main(), &at, &known, Some(&view), &request(1));
        assert_eq!(names(&caller.locals), ["argc", "pair"]);
        let LocalsRead::Rows(rows) = &caller.locals else { panic!("rows") };
        assert_eq!(rows[0].value, ShownValue::Integer { raw: 0, size: 4, signedness: Signedness::Signed }, "[rsp] of main is 0");
    }

    #[test]
    fn each_watch_is_read_in_the_selected_frame() {
        let mut known = index();
        known.modules[0].identity = GuestIdentity::Known(flat_identity());
        let view = flat_view();
        let ask = InspectRequest { frame: 1, watches: vec!["argc".into(), "a..b".into()], ..InspectRequest::default() };
        let read = read_frames(&mut leaf_called_from_main(), &at_f(Some(CodeWidth::Long64)), &known, Some(&view), &ask);
        assert_eq!(read.watches.len(), 2);
        assert_eq!(read.watches[0].text, "argc");
        assert_eq!(read.watches[0].rows[0].value, ShownValue::Integer { raw: 0, size: 4, signedness: Signedness::Signed });
        assert_eq!(read.watches[0].rows[0].path.root, PathRoot::Watch(0));
        assert_eq!(read.watches[1].rows[0].value, ShownValue::Problem("unexpected '.' at 2".into()));
        let innermost = read_frames(
            &mut leaf_called_from_main(),
            &at_f(Some(CodeWidth::Long64)),
            &known,
            Some(&view),
            &InspectRequest { frame: 0, ..ask },
        );
        assert_eq!(innermost.watches[0].rows[0].value, ShownValue::Problem("no variable named argc".into()));
        let unloaded = read_frames(
            &mut leaf_called_from_main(),
            &at_f(Some(CodeWidth::Long64)),
            &known,
            None,
            &InspectRequest { frame: 0, watches: vec!["argc".into()], ..InspectRequest::default() },
        );
        assert_eq!(unloaded.watches[0].rows[0].value, ShownValue::Problem("the symbols of flat.exe are not loaded".into()));
    }

    fn tip_in_main(tip: &str, expanded: BTreeSet<ValuePath>) -> TipRead {
        let mut known = index();
        known.modules[0].identity = GuestIdentity::Known(flat_identity());
        let ask = InspectRequest { frame: 1, tip: Some(tip.into()), expanded, ..InspectRequest::default() };
        read_frames(&mut leaf_called_from_main(), &at_f(Some(CodeWidth::Long64)), &known, Some(&flat_view()), &ask).tip
    }

    #[test]
    fn a_tip_is_read_as_a_watch_in_the_selected_frame() {
        let TipRead::Rows(read) = tip_in_main("argc", BTreeSet::new()) else { panic!("rows") };
        assert_eq!(read.text, "argc");
        assert_eq!(read.rows[0].value, ShownValue::Integer { raw: 0, size: 4, signedness: Signedness::Signed });
        assert_eq!(read.rows[0].path.root, PathRoot::Tip);
        let TipRead::Rows(closed) = tip_in_main("pair", BTreeSet::new()) else { panic!("rows") };
        assert_eq!(closed.rows.len(), 1);
        assert!(matches!(&closed.rows[0].value, ShownValue::Aggregate { preview, .. } if preview.len() == 2));
        let open = BTreeSet::from([ValuePath { root: PathRoot::Tip, steps: Vec::new() }]);
        let TipRead::Rows(expanded) = tip_in_main("pair", open) else { panic!("rows") };
        let members: Vec<&str> = expanded.rows.iter().skip(1).map(|row| row.name.as_str()).collect();
        assert_eq!(members, ["a", "b"]);
    }

    #[test]
    fn a_tip_on_a_word_that_names_nothing_reads_as_no_variable() {
        assert_eq!(tip_in_main("nothing", BTreeSet::new()), TipRead::NoVariable { text: "nothing".into() });
        let no_tip = read_frames(&mut leaf_called_from_main(), &at_f(Some(CodeWidth::Long64)), &index(), None, &request(0));
        assert_eq!(no_tip.tip, TipRead::None);
    }

    #[test]
    fn a_frame_in_no_module_says_so() {
        let at = FramesAt { root: Root::Live, width: Some(CodeWidth::Long64), start: registers_at(0x9_0000, STACK), xmm: None };
        let read = read_frames(&mut leaf_called_from_main(), &at, &index(), None, &request(0));
        assert_eq!(read.locals, LocalsRead::NoModule);
        let sixteen = read_frames(&mut leaf_called_from_main(), &at_f(None), &index(), None, &request(0));
        assert_eq!(sixteen.locals, LocalsRead::NotAsked, "16-bit code's variables are not read");
    }

    #[test]
    fn sixteen_bit_code_is_one_frame_that_says_why() {
        let read = read_frames(&mut leaf_called_from_main(), &at_f(None), &index(), None, &request(0));
        assert_eq!(read.frames.len(), 1);
        assert_eq!(read.end, StackEnd::SixteenBit);
    }

    const UNWOUND: PeImageId = PeImageId { time_date_stamp: 0x5E1F_0002, size_of_image: 0x4000 };

    fn unwound_identity() -> ModuleIdentity {
        ModuleIdentity::Pe { file_name: "unwound.exe".into(), image: UNWOUND, pdb: None }
    }

    fn unwound_key() -> ImageKey {
        ImageKey { file_name: "unwound.exe".into(), image: UNWOUND }
    }

    /// `unwound.exe` linked at `BASE`: `main` at 0x1000 and `f` at 0x1100,
    /// whose prolog is `sub rsp, 0x18` (four bytes), its unwind info at
    /// 0x2000 and its `RUNTIME_FUNCTION` at 0x3000.
    fn unwound_binary() -> Vec<u8> {
        let mut text = vec![0x90u8; 0x200];
        text[0x100..0x104].copy_from_slice(&[0x48, 0x83, 0xEC, 0x18]);
        // Version 1, a 4-byte prolog, one code: UWOP_ALLOC_SMALL of 0x18 at 4.
        let info = vec![0x01, 0x04, 0x01, 0x00, 0x04, 0x22, 0x00, 0x00];
        let mut pdata = Vec::new();
        for field in [0x1100u32, 0x1140, 0x2000] {
            pdata.extend_from_slice(&field.to_le_bytes());
        }
        pe64_file(&Pe64Spec {
            time_date_stamp: UNWOUND.time_date_stamp,
            size_of_image: UNWOUND.size_of_image,
            image_base: BASE,
            sections: vec![
                SpecSection { name: b".text", rva: 0x1000, bytes: text, characteristics: SECTION_CODE },
                SpecSection { name: b".rdata", rva: 0x2000, bytes: info, characteristics: SECTION_READ_ONLY },
                SpecSection { name: b".pdata", rva: 0x3000, bytes: pdata, characteristics: SECTION_READ_ONLY },
            ],
            exception: Some(Directory { rva: 0x3000, size: 12 }),
            base_relocations: None,
            import_address_table: None,
        })
    }

    /// Paused in `f` past its prolog, called from `main + 5`; the guest has
    /// paged in the code alone, not the headers or the unwind data.
    fn f_with_unwind_data_paged_out() -> FakeGuest {
        let mut guest = FakeGuest::new(CR3);
        guest.write(CR3, BASE + 0x1000, &[0x90; 0x1000]);
        guest.write_u64(CR3, STACK + 0x18, MAIN + 5);
        guest.write_u64(CR3, STACK + 0x20, 0);
        guest
    }

    fn unwound_index() -> SymbolIndex {
        let mut index = index();
        index.modules[0].name = "unwound.exe".into();
        index.modules[0].size = u64::from(UNWOUND.size_of_image);
        index.modules[0].identity = GuestIdentity::Known(unwound_identity());
        index
    }

    fn view_with_image(entry: ImageEntry) -> SymbolsView {
        let mut view = SymbolsView::default();
        view.images.insert(unwound_key(), entry);
        view
    }

    fn at_f_past_prolog() -> FramesAt {
        FramesAt { root: Root::Live, width: Some(CodeWidth::Long64), start: registers_at(F + 0x10, STACK), xmm: None }
    }

    #[test]
    fn unwind_data_the_guest_has_not_paged_in_is_read_from_the_modules_binary() {
        let file = ModuleFile::parse(unwound_binary()).expect("the fixture parses");
        let view = view_with_image(ImageEntry {
            state: ImageState::Loaded { from: SymbolSource::Store { path: "unwound.exe".into() } },
            file: Some(Arc::new(file)),
        });
        let read = read_frames(&mut f_with_unwind_data_paged_out(), &at_f_past_prolog(), &unwound_index(), Some(&view), &request(0));
        assert_eq!(read.frames.len(), 2, "f, then main: {:?}", read.end);
        assert_eq!(read.frames[1].registers.rip, MAIN + 5);
        assert_eq!(read.frames[1].registers.rsp, STACK + 0x20);
        assert_eq!(read.frames[1].how, FrameFound::Unwind);
        assert!(read.awaiting_images.is_empty());
    }

    /// Step Out finds where `f` returns the same way the call stack does.
    #[test]
    fn the_caller_step_out_runs_to_is_found_through_the_binary() {
        let file = ModuleFile::parse(unwound_binary()).expect("the fixture parses");
        let view = view_with_image(ImageEntry {
            state: ImageState::Loaded { from: SymbolSource::Store { path: "unwound.exe".into() } },
            file: Some(Arc::new(file)),
        });
        let frame = Frame { registers: registers_at(F + 0x10, STACK), how: FrameFound::Registers };
        let index = unwound_index();
        let unwinding = CallerOf { root: Root::Live, modules: &index.modules, width: CodeWidth::Long64 };
        let caller = caller_frame(&mut f_with_unwind_data_paged_out(), &unwinding, Some(&view), &frame);
        assert_eq!(caller.map(|caller| caller.registers.rip), Some(MAIN + 5));
        let without = caller_frame(&mut f_with_unwind_data_paged_out(), &unwinding, None, &frame);
        assert_eq!(without, None, "no binary, no caller");
    }

    #[test]
    fn a_binary_the_view_does_not_have_yet_is_awaited() {
        let read = read_frames(&mut f_with_unwind_data_paged_out(), &at_f_past_prolog(), &unwound_index(), None, &request(0));
        assert_eq!(read.frames.len(), 1);
        assert_eq!(read.end, StackEnd::Unreadable { at: BASE });
        assert_eq!(read.awaiting_images, [unwound_key()]);
        let pending = view_with_image(ImageEntry { state: ImageState::Pending, file: None });
        let read =
            read_frames(&mut f_with_unwind_data_paged_out(), &at_f_past_prolog(), &unwound_index(), Some(&pending), &request(0));
        assert_eq!(read.awaiting_images, [unwound_key()], "still being looked up");
    }

    #[test]
    fn a_binary_that_was_not_found_is_not_awaited_again() {
        let missing = view_with_image(ImageEntry { state: ImageState::NotOnServer, file: None });
        let read =
            read_frames(&mut f_with_unwind_data_paged_out(), &at_f_past_prolog(), &unwound_index(), Some(&missing), &request(0));
        assert_eq!(read.end, StackEnd::Unreadable { at: BASE });
        assert!(read.awaiting_images.is_empty(), "a refresh would find nothing new");
    }

    #[test]
    fn a_thread_view_unwinds_from_its_saved_registers() {
        let saved = SavedRegisters {
            rip: F,
            rsp: STACK,
            rbp: 0x5555,
            rflags: 0x202,
            cs: 0x33,
            ss: 0x2B,
            rax: 1,
            rcx: 2,
            rdx: 3,
            r8: 4,
            r9: 5,
            r10: 6,
            r11: 7,
            rbx: 8,
            rdi: 9,
            rsi: 10,
        };
        let at = frames_at_thread(&saved, 0x8000);
        assert_eq!(at.root, Root::Process(0x8000));
        assert_eq!(at.width, Some(CodeWidth::Long64));
        assert_eq!(at.start.registers[RBP], Some(0x5555));
        assert_eq!(at.start.registers[RSP], Some(STACK));
        assert_eq!(at.start.registers[RBX], None, "a trap frame's other registers may not be the thread's");
        assert_eq!(at.xmm, None);
        let wow64 = frames_at_thread(&SavedRegisters { cs: 0x23, ..saved }, 0x8000);
        assert_eq!(wow64.width, Some(CodeWidth::Protected32));
    }
}
