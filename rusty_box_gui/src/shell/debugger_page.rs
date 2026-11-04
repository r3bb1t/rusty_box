//! The Debugger page: a paused VM's processors, code, memory and stack, laid
//! out as x64dbg lays out its CPU tab — a fixed grid of code and registers
//! above memory and stack, each pane scrolling on its own — where the page
//! is wide and tall enough for four useful panes, and otherwise as panes at
//! the height of their content in two columns or one, the page scrolling as
//! a whole (`Fit`). The choice is made from the room the page measures, never
//! from the platform. What the last step (or pause) changed is drawn red; the
//! memory pane searches for hex, text or UTF-16. Like the VM bar, it draws
//! from borrowed state and reports what was asked for; the app acts on it.

pub(crate) mod frames_view;
pub(crate) mod references;
pub(crate) mod source_view;

use std::collections::{BTreeSet, HashMap};

use egui::text::{LayoutJob, TextFormat};
use egui::RichText;
use rusty_box::cpu::{
    BreakpointRefusal, CodeBreakpoint, CodeBreakpointHit, CodeSize, CpuDebugView, ExecutionMode, ProcessorActivity,
    MAX_LINEAR_BREAKPOINTS,
};

use crate::debugger::control::{DebugNote, StepCut};
use crate::debugger::source_breakpoints::{
    parse_target, resolve, BreakpointModules, BreakpointTarget, PendingWhy, Resolutions, SourceResolution, TargetError,
    TargetText,
};
use crate::debugger::stepping::{StepCeiling, StepKind, StepUnit};
use crate::source::find::PathMap;
use frames_view::{
    call_stack_rows, locals_rows, watch_rows, LowerLeft, LowerRight, VariableState, CALLER_LINE, CALL_STACK, LOCALS,
    MEMORY, STACK, WATCH,
};
use source_view::{
    line_address, line_marks, line_target, source_at, source_rows, toggled_line, CodeView, PauseSource, PlaceMark, SourceAsk,
    SourcePane, SourceTip, TipView, DISASSEMBLY_SWITCH, NO_SOURCE_HERE, SOURCE_SWITCH,
};
use crate::debugger::inspect::{
    AddressSpace, CodeTarget, CodeText, DebugSnapshot, GuestAddress, MemoryTarget, ModuleSpace, ViewContext,
    Viewing, MEMORY_WINDOW_BYTES, ROW_BYTES, WOW64_CODE_SELECTOR,
};
use crate::debugger::threads::SavedRegisters;
use crate::debugger::xref::service::{XrefScope, XrefTarget};
use crate::symbols::hub::SymbolsView;
use references::{address_target, references_page, ReferencesView, REFERENCES_TO, REINDEX_MODULE};
use crate::symbols::names::{NameMiss, NameTrust, NamedAddress, Namer};

mod symbols_tab;
pub(crate) use symbols_tab::{SymbolFilter, SymbolsAction};
#[cfg(test)]
pub(crate) use symbols_tab::{LOAD_FROM_FILE, SYMBOL_SETTINGS};
use symbols_tab::{module_symbols_body, symbol_counts, symbols_page, ModuleStates, SYMBOL_STATE_CHARS};
use crate::debugger::search::{
    pattern_of, FindOutcome, FindRequest, FoundAt, PatternError, SearchEncoding,
};
use crate::debugger::memory_map::{MemoryMap, RegionOwner};
use crate::debugger::threads::WinThread;
use crate::debugger::pe::{
    ExportRead, ExportTarget, ImportName, ImportRead, ImportedModule, PeExport, PeImport, ThunksEnd,
};
use crate::debugger::windows::{
    DeviceRow, DispatchEntry, DriverDetail, DriverListing, DriverShown, KernelLayouts,
    KernelListing, KernelMiss, ModuleBits, ModuleDetail, ModuleRead, PickedDetail, PickedDriver, PickedModule, PointerSite,
    GuiMiss, GuiServices, ProcessDetail, ProcessListing, SectionsRead, ServiceTableRead, SsdtListing, SsdtService,
    SystemListing, SystemView, TablePlace, TreeRow, WinDriver, WinModule, WinProcess,
};
use crate::shell::listing::{
    card_title, char_width, chip, empty_state, header_row, lead_arrow, lead_chevron, lead_dot, lead_hollow_arrow, lead_ring,
    list_row, mono_font, row_visible, segmented, skip_row, title_label, tree_guides, Cell, RowSense,
    Segment, Tone, ROW_PAD, TREE_INDENT,
};
use crate::shell::theme::{
    shell_card_frame, shell_card_frame_with_margin, ACCENT_AMBER, ACCENT_BLUE, ACCENT_CYAN,
    ACCENT_RED, CONTROL_FILL_REST, SPACE_CARD, SPACE_CARD_COMPACT, SPACE_ITEM, SPACE_PAGE,
    SPACE_PAGE_COMPACT, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};

pub(crate) const NOT_RUNNING: &str =
    "Power the VM on and pause it to see its processors, code and memory.";
const PAUSE_TO_INSPECT: &str = "Pause the VM to see its processors, code and memory.";
const SHOWING_LAST_PAUSE: &str = "Running: showing the state at the last pause.";
const STEP_NEEDS_INTERPRETER: &str =
    "Step needs the interpreter: a VM on the hypervisor retires instructions nothing counts.";
/// The accessible name of the memory pane's address field.
pub(crate) const MEMORY_ADDRESS: &str = "Memory address";
/// The accessible name of the breakpoint field.
pub(crate) const BREAKPOINT_ADDRESS: &str = "Breakpoint address";
const BREAKPOINTS_NEED_INTERPRETER: &str =
    "Breakpoints need the interpreter: a VM on the hypervisor runs its instructions where this debugger cannot stop them.";
const BREAKPOINTS_IDLE: &str = "These breakpoints stay idle while the VM runs on the hypervisor.";
/// What the Breakpoints tab's Remove does.
const REMOVE_BREAKPOINT: &str = "Remove the picked breakpoint.";
/// A code line's menu items.
pub(crate) const SET_BREAKPOINT_HERE: &str = "Set breakpoint";
pub(crate) const REMOVE_BREAKPOINT_HERE: &str = "Remove breakpoint";
/// The accessible name of the memory pane's search field.
pub(crate) const FIND: &str = "Find";
/// What a step changed is drawn in this colour, as x64dbg does.
const CHANGED: egui::Color32 = ACCENT_RED;
/// The narrowest page, points, that fills a grid of four panes: the left
/// column holds a code row and a memory row without scrolling sideways.
const GRID_MIN_WIDTH: f32 = 940.0;
/// The least height, points, a grid pane keeps a useful run of rows in.
const GRID_PANE_MIN_HEIGHT: f32 = 200.0;
/// What a grid page spends above its panes, points: its margins, the header
/// row (two where the chips wrap) and the gaps.
const GRID_CHROME_HEIGHT: f32 = 150.0;
/// The shortest page, points, that fills a grid of two rows of panes.
const GRID_MIN_HEIGHT: f32 = 2.0 * GRID_PANE_MIN_HEIGHT + SPACE_ITEM + GRID_CHROME_HEIGHT;
/// The narrowest page, points, whose panes sit in two columns while the page
/// scrolls: each column then holds the registers and the stack without
/// scrolling sideways.
const TWO_COLUMN_MIN_WIDTH: f32 = 720.0;

/// How many columns a scrolling page lays its panes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Columns {
    One,
    Two,
}

/// How the page lays its panes out. It is decided from the room the page
/// measures for itself — never from the platform — so a short desktop window
/// is laid out as a landscape phone is, and a tall phone as a narrow window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    /// Four panes fill the page, each scrolling on its own, as x64dbg's CPU
    /// tab does: the page is wide and tall enough for every pane to keep a
    /// useful number of rows.
    Grid,
    /// Every pane at the height of its content, in `Columns`; the page
    /// scrolls as a whole, so no pane is ever out of reach.
    Scrolling(Columns),
}

impl Fit {
    /// The layout for a page of `room` points.
    fn of(room: egui::Vec2) -> Self {
        if room.x >= GRID_MIN_WIDTH && room.y >= GRID_MIN_HEIGHT {
            Self::Grid
        } else if room.x >= TWO_COLUMN_MIN_WIDTH {
            Self::Scrolling(Columns::Two)
        } else {
            Self::Scrolling(Columns::One)
        }
    }

    /// A page with room to spare is roomy; one that scrolls packs tighter.
    fn density(self) -> Density {
        match self {
            Self::Grid => Density::Roomy,
            Self::Scrolling(Columns::One | Columns::Two) => Density::Compact,
        }
    }
}

/// How much air the page keeps around and between its panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Density {
    Roomy,
    Compact,
}

impl Density {
    /// The page's margin on every side.
    fn page_margin(self) -> i8 {
        match self {
            Self::Roomy => SPACE_PAGE,
            Self::Compact => SPACE_PAGE_COMPACT,
        }
    }

    /// A card that is as tall as its content.
    fn card_frame(self) -> egui::Frame {
        shell_card_frame_with_margin(match self {
            Self::Roomy => SPACE_CARD,
            Self::Compact => SPACE_CARD_COMPACT,
        })
    }

    /// The gap between panes, and between a row's items.
    fn gap(self) -> f32 {
        match self {
            Self::Roomy => SPACE_ITEM,
            Self::Compact => SPACE_ITEM * 0.75,
        }
    }

    /// The gap between a card's title, its controls and its first row.
    fn title_gap(self) -> f32 {
        match self {
            Self::Roomy => 4.0,
            Self::Compact => 2.0,
        }
    }

    /// The padding inside a header button.
    fn button_padding(self) -> egui::Vec2 {
        match self {
            Self::Roomy => egui::vec2(12.0, 6.0),
            Self::Compact => egui::vec2(8.0, 3.0),
        }
    }
}

/// What a typed address could not be read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddressError {
    Empty,
    NotHex,
    TooLong,
    /// A `module!name` that names no address.
    Name(NameMiss),
}

impl AddressError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Empty => "Type an address or module!name.",
            Self::NotHex => "An address is hexadecimal digits, with or without 0x, or module!name.",
            Self::TooLong => "An address has at most 16 hexadecimal digits.",
            Self::Name(miss) => miss.message(),
        }
    }
}

/// What Go was given: `module!name`, named against the pause's modules and
/// the engine's tables, or an address.
fn go_target(text: &str, names: Namer<'_>) -> Result<u64, AddressError> {
    match text.trim().split_once('!') {
        Some((module, name)) => names.address_of(module.trim(), name.trim()).map_err(AddressError::Name),
        None => parse_address(text),
    }
}

/// Reads a typed address: hexadecimal, with an optional `0x`, and `_` or a
/// backtick between digit groups as x64dbg writes them.
pub(crate) fn parse_address(text: &str) -> Result<u64, AddressError> {
    let trimmed = text.trim();
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let digits: String = digits.chars().filter(|c| *c != '_' && *c != '`').collect();
    if digits.is_empty() {
        return Err(AddressError::Empty);
    }
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AddressError::NotHex);
    }
    if digits.len() > 16 {
        return Err(AddressError::TooLong);
    }
    u64::from_str_radix(&digits, 16).map_err(|_| AddressError::NotHex)
}

/// What the page keeps between frames for one VM.
#[derive(Debug)]
pub(crate) struct DebuggerPane {
    pub(crate) address_text: String,
    pub(crate) space: AddressSpace,
    pub(crate) address_error: Option<AddressError>,
    pub(crate) cpu: usize,
    /// The window start last written into `address_text`, so a new snapshot
    /// updates the field without overwriting what is being typed.
    pub(crate) shown_start: Option<GuestAddress>,
    pub(crate) search_text: String,
    pub(crate) search_encoding: SearchEncoding,
    pub(crate) search_error: Option<PatternError>,
    pub(crate) tab: DebuggerTab,
    /// The process whose modules the Processes view lists (its `EPROCESS`
    /// address); `None` lists the running process's.
    pub(crate) selected_process: Option<u64>,
    pub(crate) process_layout: ProcessLayout,
    /// What a listed process's or module's name must contain.
    pub(crate) list_filter: String,
    /// Whether the Processes view shows modules or the memory map.
    pub(crate) process_detail: ProcessDetail,
    /// Module bases whose sections are expanded in the memory map.
    pub(crate) expanded_modules: std::collections::BTreeSet<u64>,
    /// An address the memory list should scroll to next frame: set by a
    /// minimap click, consumed once.
    pub(crate) scroll_to: Option<u64>,
    /// How the grid's panes divide the page.
    pub(crate) split: PaneSplit,
    /// The kernel module the Kernel modules view details (its base).
    pub(crate) selected_kernel_module: Option<u64>,
    /// Whether that view shows the module's sections or its exports.
    pub(crate) kernel_detail: ModuleDetail,
    /// What a listed export's name must contain.
    pub(crate) export_filter: String,
    /// Which card of the Processes or Kernel modules tab the keys move in.
    pub(crate) list_focus: ListFocus,
    /// The row of that tab's detail card the keys stand on.
    pub(crate) detail_cursor: Option<usize>,
    /// The module of the listed process the Processes view details (its
    /// base), and what is shown of it.
    pub(crate) selected_process_module: Option<u64>,
    pub(crate) process_module_detail: ModuleDetail,
    /// What a listed export or import of that module must contain.
    pub(crate) process_module_filter: String,
    /// The row of that module's card the keys stand on.
    pub(crate) module_cursor: Option<usize>,
    /// The picked module's import groups (by index) shown folded shut, on
    /// either tab.
    pub(crate) folded_imports: BTreeSet<usize>,
    /// The driver the Drivers view details (its `DRIVER_OBJECT` address).
    pub(crate) selected_driver: Option<u64>,
    /// Whether that view shows the driver's dispatch table or its devices.
    pub(crate) driver_detail: DriverDetail,
    /// What a listed driver's name must contain.
    pub(crate) driver_filter: String,
    /// The breakpoint field's text, and why it was not taken.
    pub(crate) breakpoint_text: String,
    pub(crate) breakpoint_error: Option<BreakpointFieldError>,
    /// The Breakpoints tab's picked row.
    pub(crate) breakpoint_cursor: Option<usize>,
    /// The code pane's picked instruction (its address), which F2 sets or
    /// removes a breakpoint on.
    pub(crate) code_cursor: Option<u64>,
    /// The source view's picked line, which F2 and Run to cursor act on.
    pub(crate) source_cursor: Option<u32>,
    /// Source or disassembly, once the owner picked one; it stays picked.
    pub(crate) code_view: Option<CodeView>,
    /// The place the code pane last brought into view: a new one (a pause,
    /// a step, a picked frame, a Go to) is scrolled to once, and the owner's
    /// own scrolling stands until the place changes.
    pub(crate) code_followed: Option<CodePlace>,
    /// How the owner's source lines resolved, kept until the targets, the
    /// pause or the symbols change.
    pub(crate) resolutions: Resolutions,
    /// Which system-service table the SSDT view lists.
    pub(crate) ssdt_table: SsdtTable,
    /// The snapshot whose modules were last asked of the symbol engine.
    pub(crate) symbols_asked: Option<std::sync::Weak<DebugSnapshot>>,
    /// The kernel layouts last made from the kernel's tables.
    pub(crate) kernel_layout: Option<crate::symbols::kernel::LayoutCache>,
    /// Which modules the Symbols tab lists.
    pub(crate) symbol_filter: SymbolFilter,
    /// The Symbols tab's row the keys stand on.
    pub(crate) symbol_cursor: Option<usize>,
    /// The module the Symbols tab acts on (its base).
    pub(crate) picked_symbol: Option<u64>,
    /// The References tab's field, and why its text was not taken.
    pub(crate) references_text: String,
    pub(crate) references_error: Option<AddressError>,
    /// What references were last asked for, and in which modules.
    pub(crate) references: Option<XrefTarget>,
    pub(crate) references_scope: XrefScope,
    /// The CPU tab's lower panes as switched; `None` until the owner picks.
    pub(crate) lower_left: Option<LowerLeft>,
    pub(crate) lower_right: Option<LowerRight>,
    /// The watches and which values are open.
    pub(crate) variables: VariableState,
}

impl Default for DebuggerPane {
    fn default() -> Self {
        Self {
            address_text: String::new(),
            space: AddressSpace::Linear,
            address_error: None,
            cpu: 0,
            shown_start: None,
            search_text: String::new(),
            search_encoding: SearchEncoding::Hex,
            search_error: None,
            tab: DebuggerTab::Cpu,
            selected_process: None,
            process_layout: ProcessLayout::Tree,
            list_filter: String::new(),
            process_detail: ProcessDetail::Modules,
            expanded_modules: BTreeSet::new(),
            scroll_to: None,
            split: PaneSplit::default(),
            selected_kernel_module: None,
            kernel_detail: ModuleDetail::Sections,
            export_filter: String::new(),
            list_focus: ListFocus::List,
            detail_cursor: None,
            selected_process_module: None,
            process_module_detail: ModuleDetail::Sections,
            process_module_filter: String::new(),
            module_cursor: None,
            folded_imports: BTreeSet::new(),
            selected_driver: None,
            driver_detail: DriverDetail::Dispatch,
            driver_filter: String::new(),
            breakpoint_text: String::new(),
            breakpoint_error: None,
            breakpoint_cursor: None,
            code_cursor: None,
            source_cursor: None,
            code_view: None,
            code_followed: None,
            resolutions: Resolutions::default(),
            ssdt_table: SsdtTable::Kernel,
            symbols_asked: None,
            kernel_layout: None,
            symbol_filter: SymbolFilter::All,
            symbol_cursor: None,
            picked_symbol: None,
            references_text: String::new(),
            references_error: None,
            references: None,
            references_scope: XrefScope::TargetModule,
            lower_left: None,
            lower_right: None,
            variables: VariableState::default(),
        }
    }
}

impl DebuggerPane {
    /// What the list tabs' trees show open.
    fn folds(&mut self) -> Folds<'_> {
        Folds {
            map_open: &mut self.expanded_modules,
            imports_shut: &mut self.folded_imports,
        }
    }
}

/// What the page shows under its toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DebuggerTab {
    /// Code, registers, memory and stack.
    Cpu,
    /// The guest's processes and the modules of one.
    Processes,
    /// The guest kernel's modules, in load order.
    KernelModules,
    /// The guest kernel's driver objects and their devices.
    Drivers,
    /// The kernel's system-service dispatch table.
    Ssdt,
    /// What the symbol engine has for each module.
    Symbols,
    /// The debugger's code breakpoints.
    Breakpoints,
    /// Who refers to an address: the cross-reference index's answers.
    References,
}

impl DebuggerTab {
    const ALL: [Self; 8] = [
        Self::Cpu,
        Self::Breakpoints,
        Self::References,
        Self::Processes,
        Self::KernelModules,
        Self::Drivers,
        Self::Ssdt,
        Self::Symbols,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Breakpoints => "Breakpoints",
            Self::References => "References",
            Self::Processes => "Processes",
            Self::KernelModules => "Kernel modules",
            Self::Drivers => "Drivers",
            Self::Ssdt => "SSDT",
            Self::Symbols => "Symbols",
        }
    }

    /// What this tab asks the run thread to list, for what `pane` has picked.
    /// The Symbols tab lists the modules every pause reads; the Breakpoints
    /// tab lists the shell's own set.
    fn system_view(self, pane: &DebuggerPane) -> SystemView {
        match self {
            Self::Cpu | Self::Symbols | Self::Breakpoints | Self::References => SystemView::Off,
            Self::Processes => SystemView::Processes {
                selected: pane.selected_process,
                detail: pane.process_detail,
                module: pane.selected_process_module,
                module_detail: pane.process_module_detail,
            },
            Self::KernelModules => SystemView::KernelModules {
                selected: pane.selected_kernel_module,
                detail: pane.kernel_detail,
            },
            Self::Drivers => SystemView::Drivers {
                selected: pane.selected_driver,
                detail: pane.driver_detail,
            },
            Self::Ssdt => SystemView::Ssdt,
        }
    }
}

pub(crate) struct DebuggerPageState<'a> {
    pub(crate) live: bool,
    pub(crate) paused: bool,
    pub(crate) step_available: bool,
    /// A step is under way.
    pub(crate) stepping: bool,
    pub(crate) dump_pending: bool,
    pub(crate) note: Option<DebugNote>,
    pub(crate) snapshot: Option<&'a DebugSnapshot>,
    /// The snapshot before `snapshot`: what is drawn red changed since it.
    pub(crate) previous: Option<&'a DebugSnapshot>,
    pub(crate) find: Option<FindOutcome>,
    pub(crate) searching: bool,
    pub(crate) pane: &'a mut DebuggerPane,
    /// What the symbol engine has found, which names addresses as drawn.
    pub(crate) symbols: &'a SymbolsView,
    /// The owner's breakpoints as named: addresses and source lines.
    pub(crate) targets: &'a [BreakpointTarget],
    /// How each source line resolved under the pause.
    pub(crate) resolutions: &'a Resolutions,
    /// The code breakpoints they come to, which the machine holds.
    pub(crate) breakpoints: &'a [CodeBreakpoint],
    /// The code breakpoint the run is paused at.
    pub(crate) stopped_at: Option<CodeBreakpointHit>,
    /// What the References tab shows, when something was asked.
    pub(crate) references: Option<&'a ReferencesView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DebuggerAction {
    Step { kind: StepKind, unit: StepUnit },
    /// Stop the step under way.
    CancelStep,
    /// Pick the file the build recorded at `build_path` from disk.
    LocateSource { build_path: String },
    /// Keep a map the owner made by locating a file in the project's
    /// settings.
    SaveRunMap { project: String, map: PathMap },
    DumpRam,
    ShowMemory(MemoryTarget),
    SelectCpu(usize),
    Find(FindRequest),
    /// List these beside the processor panes (or nothing).
    ShowSystem(SystemView),
    /// Show the CPU tab with the memory pane at this linear address: a
    /// module picked from a list.
    OpenModule(u64),
    /// Show the CPU tab with the memory pane at `address` in the address
    /// space of the process at `cr3`: a region of a process other than the
    /// running one.
    ViewInProcess { cr3: u64, address: u64 },
    /// Show the CPU tab looking from here: a process or thread seen from
    /// inside, or the live processor again.
    View(ViewContext),
    /// Something asked of the symbol engine.
    Symbols(SymbolsAction),
    /// The owner's breakpoints become these.
    SetTargets(Vec<BreakpointTarget>),
    /// Show the code here, on the CPU tab: at RIP, or at an address opened.
    ShowCode(CodeTarget),
    /// Find who refers to this, in the References tab's scope.
    FindReferences(XrefTarget),
    /// Stop the copies and indexing under way.
    CancelReferences,
    /// Index these modules (by base) again from their bytes as they are now.
    Reindex(Vec<u64>),
    /// Show frame `n` of the call stack: its variables, and its code.
    SelectFrame(usize),
    /// Show the value of what a click in the source named.
    ShowTip(SourceTip),
    CloseTip,
}

/// The banner's way back from a thread's view.
pub(crate) const BACK_TO_LIVE: &str = "Back to live CPU";
/// The code pane's way back from an address opened.
pub(crate) const BACK_TO_RIP: &str = "Back to RIP";
const STEP_WHILE_VIEWING: &str =
    "Step runs the live processor; go back to the live CPU to step.";

/// What the panes compare against and mark: the snapshot before, and the
/// last search hit; and what they name addresses by.
#[derive(Clone, Copy)]
struct Marks<'a> {
    previous: Option<&'a DebugSnapshot>,
    find: Option<FindOutcome>,
    searching: bool,
    symbols: &'a SymbolsView,
    /// The code breakpoints, which the code pane marks.
    breakpoints: &'a [CodeBreakpoint],
    /// The owner's breakpoints, which a code line's menu changes.
    targets: &'a [BreakpointTarget],
    /// How each source line resolved under the pause.
    resolutions: &'a Resolutions,
    /// The pause's place in the owner's source, when its code has a line.
    pause: Option<&'a PauseSource>,
    /// Whether the code pane follows the innermost frame or a picked caller.
    frame: FrameMark,
}

/// The frame the code pane follows: the innermost, or caller `number`,
/// picked, whose return address the pane was asked to show; `code` is the
/// call it returns from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameMark {
    Innermost,
    Caller { number: usize, code: u64 },
}

impl FrameMark {
    fn of(snapshot: &DebugSnapshot) -> FrameMark {
        let selected = snapshot.frames.selected;
        let Some(frame) = snapshot.frames.frames.get(selected).filter(|_| selected > 0) else {
            return FrameMark::Innermost;
        };
        match snapshot.code_at {
            CodeTarget::At(at) if at.value == frame.registers.rip => {
                FrameMark::Caller { number: selected, code: frame.code_address() }
            }
            CodeTarget::At(_) | CodeTarget::Rip => FrameMark::Innermost,
        }
    }
}

/// Where the code pane's source view stands: a picked caller's call, else
/// the first instruction the pane shows (RIP, or the address opened).
pub(crate) fn pause_source(snapshot: &DebugSnapshot, view: &SymbolsView) -> Option<PauseSource> {
    let address = match FrameMark::of(snapshot) {
        FrameMark::Caller { code, .. } => code,
        FrameMark::Innermost => snapshot.code.first()?.address,
    };
    source_at(address, &snapshot.symbols, view)
}

/// The colour a name is drawn in: amber when it comes from a file of
/// another build loaded anyway, else `plain`.
pub(crate) fn name_color(named: &NamedAddress, plain: egui::Color32) -> egui::Color32 {
    match named.trust {
        NameTrust::Matched => plain,
        NameTrust::Forced => ACCENT_AMBER,
    }
}

/// What the toolbar draws from.
struct Toolbar {
    paused: bool,
    step_available: bool,
    dump_pending: bool,
    /// The panes show a thread's saved state, not the live processor that a
    /// step would run.
    viewing: bool,
    /// What a step moves by: the code pane's source steps by line.
    unit: StepUnit,
    /// Where Run to cursor runs to: the picked line or instruction.
    run_to: Option<u64>,
    /// A step is under way.
    stepping: bool,
}

pub(crate) const STEP_INTO: &str = "Step into";
pub(crate) const STEP_OVER: &str = "Step over";
pub(crate) const STEP_OUT: &str = "Step out";
pub(crate) const RUN_TO_CURSOR: &str = "Run to cursor";
pub(crate) const STOP_STEP: &str = "Stop step";
const NOTHING_PICKED: &str = "Pick a line or an instruction to run to.";

/// One step control: its name, the short text its button shows, its key,
/// and what it asks.
struct StepControl {
    label: &'static str,
    shown: &'static str,
    key: egui::KeyboardShortcut,
    kind: StepControlKind,
}

#[derive(Clone, Copy)]
enum StepControlKind {
    Into,
    Over,
    Out,
    RunToCursor,
}

/// Visual Studio's keys: F11, F10, Shift+F11, Ctrl+F10. The ones with a
/// modifier come first, so the plain key does not take their press.
const STEP_CONTROLS: [StepControl; 4] = [
    StepControl {
        label: STEP_OUT,
        shown: "Out",
        key: egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::F11),
        kind: StepControlKind::Out,
    },
    StepControl {
        label: RUN_TO_CURSOR,
        shown: "To cursor",
        key: egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::F10),
        kind: StepControlKind::RunToCursor,
    },
    StepControl {
        label: STEP_INTO,
        shown: "Into",
        key: egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F11),
        kind: StepControlKind::Into,
    },
    StepControl {
        label: STEP_OVER,
        shown: "Over",
        key: egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F10),
        kind: StepControlKind::Over,
    },
];

/// The order the step buttons are drawn in.
const STEP_BUTTON_ORDER: [usize; 4] = [2, 3, 0, 1];

/// The toolbar's step buttons and their keys.
fn step_controls(ui: &mut egui::Ui, bar: &Toolbar, action: &mut Option<DebuggerAction>) {
    if bar.stepping {
        if ui.button(STOP_STEP).clicked() {
            *action = Some(DebuggerAction::CancelStep);
        }
        return;
    }
    let can_step = bar.paused && bar.step_available && !bar.viewing;
    let ask = |kind: StepControlKind| -> Option<DebuggerAction> {
        let kind = match kind {
            StepControlKind::Into => StepKind::Into,
            StepControlKind::Over => StepKind::Over,
            StepControlKind::Out => StepKind::Out,
            StepControlKind::RunToCursor => StepKind::RunTo { address: bar.run_to? },
        };
        Some(DebuggerAction::Step { kind, unit: bar.unit })
    };
    let enabled = |kind: StepControlKind| can_step && (!matches!(kind, StepControlKind::RunToCursor) || bar.run_to.is_some());
    for index in STEP_BUTTON_ORDER {
        let Some(control) = STEP_CONTROLS.get(index) else {
            continue;
        };
        let on = enabled(control.kind);
        let button = ui.add_enabled(on, egui::Button::new(control.shown));
        button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, control.label));
        let button = if !bar.step_available {
            button.on_disabled_hover_text(STEP_NEEDS_INTERPRETER)
        } else if bar.viewing {
            button.on_disabled_hover_text(STEP_WHILE_VIEWING)
        } else if matches!(control.kind, StepControlKind::RunToCursor) && bar.run_to.is_none() {
            button.on_disabled_hover_text(NOTHING_PICKED)
        } else {
            button.on_hover_text(format!("{} · {}", control.label, ui.ctx().format_shortcut(&control.key)))
        };
        if button.clicked() {
            *action = ask(control.kind);
        }
    }
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    for control in &STEP_CONTROLS {
        if enabled(control.kind) && ui.input_mut(|input| input.consume_shortcut(&control.key)) {
            *action = ask(control.kind);
            return;
        }
    }
}

/// Under the header while the panes look from somewhere other than the live
/// processor: whom they show (or why that cannot be shown) and the way back.
fn viewing_banner(ui: &mut egui::Ui, snapshot: &DebugSnapshot, action: &mut Option<DebuggerAction>) {
    let text = match &snapshot.viewing {
        Viewing::Live => return,
        Viewing::Running { process } => {
            format!("{process} is running on this processor: showing the live CPU.")
        }
        Viewing::Thread(thread) => format!(
            "Viewing {} · PID {} · thread {} · saved state",
            thread.process, thread.pid, thread.tid
        ),
        Viewing::Miss(miss) => miss.describe(),
    };
    egui::Frame::new()
        .fill(ACCENT_AMBER.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0_f32, ACCENT_AMBER.gamma_multiply(0.5)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(text).color(ACCENT_AMBER));
                if ui.button(BACK_TO_LIVE).clicked() {
                    *action = Some(DebuggerAction::View(ViewContext::Live));
                }
            });
        });
}

/// Over the panes while the code pane shows an address other than RIP (a
/// picked caller frame's code, or an address opened): where, and the way
/// back.
fn code_elsewhere_banner(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    names: Namer<'_>,
    frame: FrameMark,
    action: &mut Option<DebuggerAction>,
) {
    let CodeTarget::At(at) = snapshot.code_at else {
        return;
    };
    let text = match frame {
        FrameMark::Caller { number, .. } => format!("Showing caller frame {number}"),
        FrameMark::Innermost => {
            let place = names.name(at.value).map_or_else(|| format!("{:X}", at.value), |named| named.label());
            format!("Showing code at {place}")
        }
    };
    egui::Frame::new()
        .fill(ACCENT_CYAN.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0_f32, ACCENT_CYAN.gamma_multiply(0.5)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(text).color(ACCENT_CYAN));
                if ui.button(BACK_TO_RIP).clicked() {
                    *action = Some(DebuggerAction::ShowCode(CodeTarget::Rip));
                }
            });
        });
}

pub(crate) fn draw_debugger_page(
    ui: &mut egui::Ui,
    state: DebuggerPageState<'_>,
) -> Option<DebuggerAction> {
    let mut action = None;
    let DebuggerPageState {
        live,
        paused,
        step_available,
        stepping,
        dump_pending,
        note,
        snapshot,
        previous,
        find,
        searching,
        pane,
        symbols,
        targets,
        resolutions,
        breakpoints,
        stopped_at,
        references,
    } = state;
    // What was read from somewhere else is no baseline for what changed: a
    // live snapshot and a thread's view are compared with neither.
    let previous = previous.filter(|before| snapshot.is_some_and(|now| now.context == before.context));
    // The pause's place in the owner's source: its first code line is the
    // instruction at RIP.
    let pause = snapshot.and_then(|snapshot| pause_source(snapshot, symbols));
    let source_shown = pause.is_some() && CodeView::shown(pane.code_view, pause.is_some()) == CodeView::Source;
    let run_to = if source_shown {
        pane.source_cursor.and_then(|line| line_address(pause.as_ref()?, line))
    } else {
        pane.code_cursor
    };
    let marks = Marks {
        previous,
        find,
        searching,
        symbols,
        breakpoints,
        targets,
        resolutions,
        pause: pause.as_ref(),
        frame: snapshot.map_or(FrameMark::Innermost, FrameMark::of),
    };
    // The room the page was given, measured here and nowhere else: every
    // layout choice below follows from it.
    let fit = Fit::of(ui.available_size());
    let density = fit.density();
    egui::Frame::new()
        .inner_margin(egui::Margin::same(density.page_margin()))
        .show(ui, |ui| {
            if !live {
                empty_state(ui, "Not running", NOT_RUNNING);
                return;
            }
            let bar = Toolbar {
                paused,
                step_available,
                dump_pending,
                viewing: snapshot.is_some_and(|snapshot| matches!(snapshot.viewing, Viewing::Thread(_))),
                unit: if source_shown { StepUnit::Line } else { StepUnit::Instruction },
                run_to,
                stepping,
            };
            header(ui, &bar, density, snapshot, symbols, pane, &mut action);
            if let Some(note) = note {
                ui.label(RichText::new(note_text(note)).color(ACCENT_AMBER));
            }
            // Names come from the pause shown, when there is one.
            let no_modules = crate::debugger::inspect::SymbolIndex::default();
            let names = Namer::new(snapshot.map_or(&no_modules, |snapshot| &snapshot.symbols), symbols);
            if let Some(hit) = stopped_at {
                breakpoint_banner(ui, hit, names);
            }
            // The Breakpoints tab needs no pause: a breakpoint can be set on a
            // running VM.
            if pane.tab == DebuggerTab::Breakpoints {
                let list = BreakpointList {
                    targets,
                    applied: breakpoints,
                    names,
                    resolutions,
                    available: step_available,
                };
                ui.add_space(density.gap());
                breakpoints_page(ui, density, &list, pane, &mut action);
                return;
            }
            let Some(snapshot) = snapshot else {
                empty_state(ui, "Running", PAUSE_TO_INSPECT);
                return;
            };
            viewing_banner(ui, snapshot, &mut action);
            if pane.tab == DebuggerTab::Cpu {
                code_elsewhere_banner(ui, snapshot, names, marks.frame, &mut action);
            }
            follow_window(pane, snapshot);
            ui.add_space(density.gap());
            if pane.tab == DebuggerTab::Cpu {
                code_key(ui, snapshot, marks, pane, &mut action);
            }
            match (pane.tab, fit) {
                (DebuggerTab::Cpu, Fit::Grid) => {
                    grid_layout(ui, snapshot, marks, pane, &mut action);
                }
                (DebuggerTab::Cpu, Fit::Scrolling(columns)) => {
                    scrolling_layout(ui, columns, density, snapshot, marks, pane, &mut action);
                }
                // Drawn above, before a pause is needed.
                (DebuggerTab::Breakpoints, _) => {}
                (DebuggerTab::References, _) => {
                    references_page(ui, density, references, names, pane, &mut action);
                }
                (
                    DebuggerTab::Processes
                    | DebuggerTab::KernelModules
                    | DebuggerTab::Drivers
                    | DebuggerTab::Ssdt,
                    _,
                ) => {
                    let states = ModuleStates { index: &snapshot.symbols, view: symbols };
                    system_page(ui, fit, snapshot, states, pane, &mut action);
                }
                (DebuggerTab::Symbols, _) => {
                    symbols_page(ui, fit, snapshot, symbols, pane, &mut action);
                }
            }
        });
    action
}

/// What the page says of something the run refused.
fn note_text(note: DebugNote) -> String {
    match note {
        DebugNote::StepNeedsInterpreter => STEP_NEEDS_INTERPRETER.to_owned(),
        DebugNote::BreakpointsNeedInterpreter => BREAKPOINTS_NEED_INTERPRETER.to_owned(),
        DebugNote::TooManyBreakpoints => {
            format!("The machine holds at most {MAX_LINEAR_BREAKPOINTS} breakpoints; the last change was not taken.")
        }
        DebugNote::StepCut(cut) => step_cut_text(cut),
    }
}

pub(crate) const STEP_CANCELLED: &str = "The step was stopped.";
pub(crate) const STEP_NO_CALLER: &str = "Step out did not run: where this function returns is not known.";
pub(crate) const STEP_TIME_CEILING: &str = "The step did not finish: stopped after a minute of running.";

/// What the page says of a step that stopped before it finished.
fn step_cut_text(cut: StepCut) -> String {
    match cut {
        StepCut::Ceiling(StepCeiling::Instructions(count)) => {
            format!("The step did not finish: stopped after {count} instructions.")
        }
        StepCut::Ceiling(StepCeiling::Time) => STEP_TIME_CEILING.to_owned(),
        StepCut::Cancelled => STEP_CANCELLED.to_owned(),
        StepCut::NoCaller => STEP_NO_CALLER.to_owned(),
        StepCut::BreakpointsRefused(BreakpointRefusal::TooManyLinear | BreakpointRefusal::TooManyPhysical) => format!(
            "The step needs breakpoints of its own, and with yours they are more than the {MAX_LINEAR_BREAKPOINTS} the machine holds."
        ),
        StepCut::BreakpointsRefused(BreakpointRefusal::EngineCannotStop) => BREAKPOINTS_NEED_INTERPRETER.to_owned(),
    }
}

/// How a breakpoint reads: its address's name when the pause names it, else
/// the address.
fn breakpoint_label(breakpoint: CodeBreakpoint, names: Namer<'_>) -> String {
    match breakpoint {
        CodeBreakpoint::Linear(address) => names
            .name(address)
            .map_or_else(|| format!("{address:X}"), |named| named.label()),
        CodeBreakpoint::Physical(address) => format!("physical {address:X}"),
    }
}

/// Under the header while the run is paused at a code breakpoint: which one,
/// and on which processor.
fn breakpoint_banner(ui: &mut egui::Ui, hit: CodeBreakpointHit, names: Namer<'_>) {
    let text = format!(
        "Stopped at breakpoint {} · {}",
        breakpoint_label(hit.breakpoint, names),
        cpu_label(hit.cpu)
    );
    egui::Frame::new()
        .fill(ACCENT_RED.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0_f32, ACCENT_RED.gamma_multiply(0.5)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(ACCENT_RED));
        });
}

/// What the Breakpoints tab draws from.
struct BreakpointList<'a> {
    /// The owner's breakpoints as named.
    targets: &'a [BreakpointTarget],
    /// The code breakpoints they come to.
    applied: &'a [CodeBreakpoint],
    names: Namer<'a>,
    /// How each source line resolved under the pause.
    resolutions: &'a Resolutions,
    /// The VM runs on the interpreter, which can stop at one.
    available: bool,
}

/// Said when no breakpoint is set.
pub(crate) const NO_BREAKPOINTS: &str =
    "No breakpoints. Add one above, or pick an instruction on the CPU tab and press F2.";
/// How the picked breakpoint is removed.
const BREAKPOINT_KEYS: &str = "F2 or Delete removes the picked breakpoint.";
/// A source breakpoint no module of the pause has code for.
pub(crate) const PENDING_NO_LINE: &str = "pending: no loaded module has this line";
/// A source breakpoint before a first pause has listed the modules.
pub(crate) const PENDING_NO_PAUSE: &str = "pending: pause to find the modules";

fn breakpoint_columns() -> [Cell; 3] {
    [
        Cell::right("#", TEXT_MUTED, 3),
        Cell::left("Breakpoint", TEXT_MUTED, 24),
        Cell::fill("Where", TEXT_MUTED),
    ]
}

/// Where a breakpoint is, as its row says: an address's name; for a source
/// line, the first place it resolved to, the line that has the code when it
/// is not the one asked, and how many places; or why it is pending.
fn target_place(target: &BreakpointTarget, list: &BreakpointList<'_>) -> String {
    match target {
        BreakpointTarget::Address(CodeBreakpoint::Linear(address)) => {
            list.names.name(*address).map(|named| named.label()).unwrap_or_default()
        }
        BreakpointTarget::Address(CodeBreakpoint::Physical(_)) => "physical address".to_owned(),
        BreakpointTarget::Source(source) => {
            let Some(resolution) = list.resolutions.source(source) else {
                return PENDING_NO_PAUSE.to_owned();
            };
            match resolution {
                SourceResolution::Resolved { places } => {
                    let Some(first) = places.first() else {
                        return PENDING_NO_LINE.to_owned();
                    };
                    // A place in a process the pause does not stand in is
                    // named by its module alone.
                    let mut text = list
                        .names
                        .name(first.address)
                        .map_or_else(|| format!("{}!{:X}", first.module, first.address), |named| named.label());
                    if first.line != source.line {
                        text.push_str(&format!(" · line {} ({} has no code)", first.line, source.line));
                    }
                    if places.len() > 1 {
                        text.push_str(&format!(" · {} places", places.len()));
                    }
                    text
                }
                SourceResolution::Pending(PendingWhy::NoModuleHasLine) => PENDING_NO_LINE.to_owned(),
                SourceResolution::Pending(PendingWhy::SymbolsLoading { modules }) => {
                    format!("pending: symbols for {} are loading", modules.join(", "))
                }
            }
        }
    }
}

/// A breakpoint as its row names it: its address, or `file:line`.
fn target_label(target: &BreakpointTarget) -> String {
    match target {
        BreakpointTarget::Address(CodeBreakpoint::Linear(address) | CodeBreakpoint::Physical(address)) => {
            format!("{address:016X}")
        }
        BreakpointTarget::Source(source) => format!("{}:{}", source.file, source.line),
    }
}

/// The Breakpoints tab: a field taking an address or `module!name` (as Go
/// does) with Add, then every breakpoint a row; a picked row is removed by
/// F2, Delete, Remove or its menu.
fn breakpoints_page(
    ui: &mut egui::Ui,
    density: Density,
    list: &BreakpointList<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    if pane.breakpoint_cursor.is_some_and(|row| row >= list.targets.len()) {
        pane.breakpoint_cursor = None;
    }
    let remove = |row: usize| -> DebuggerAction {
        let mut set = list.targets.to_vec();
        if row < set.len() {
            set.remove(row);
        }
        DebuggerAction::SetTargets(set)
    };
    let typing = ui.ctx().egui_wants_keyboard_input();
    let removing = !typing && ui.input(|input| input.key_pressed(egui::Key::F2) || input.key_pressed(egui::Key::Delete));
    if let Some(row) = pane.breakpoint_cursor.filter(|_| removing) {
        *action = Some(remove(row));
        pane.breakpoint_cursor = None;
    }
    add_breakpoint_row(ui, list, pane, action);
    ui.add_space(density.gap());
    let title = Title::counted("Breakpoints", list.targets.len());
    card(
        ui,
        density,
        title,
        |ui| {
            if !list.targets.is_empty() {
                ui.label(RichText::new(BREAKPOINT_KEYS).small().color(TEXT_MUTED));
                header_row(ui, 0.0, &breakpoint_columns());
            }
        },
        |ui| {
            if list.targets.is_empty() {
                ui.add_space(SPACE_ITEM);
                ui.label(RichText::new(NO_BREAKPOINTS).color(TEXT_MUTED));
                return;
            }
            for (row, target) in list.targets.iter().enumerate() {
                let selected = pane.breakpoint_cursor == Some(row);
                let place = target_place(target, list);
                let pending = place.starts_with("pending:");
                let cells = [
                    Cell::right((row + 1).to_string(), TEXT_MUTED, 3),
                    Cell::left(target_label(target), if pending { TEXT_MUTED } else { ACCENT_RED }, 24),
                    Cell::fill(place, if pending { ACCENT_AMBER } else { TEXT_PRIMARY }),
                ];
                let tone = if selected { Tone::Selected } else { Tone::Plain };
                let response = list_row(ui, 0.0, &cells, tone, RowSense::Pick { selected });
                if response.clicked() {
                    pane.breakpoint_cursor = Some(row);
                }
                response.context_menu(|ui| {
                    if ui.button(REMOVE_BREAKPOINT_HERE).clicked() {
                        *action = Some(remove(row));
                        ui.close();
                    }
                });
            }
            ui.add_space(SPACE_ITEM);
            let picked = pane.breakpoint_cursor;
            let remove_picked = ui
                .add_enabled(picked.is_some(), egui::Button::new("Remove"))
                .on_hover_text(REMOVE_BREAKPOINT);
            if remove_picked.clicked() {
                if let Some(row) = picked {
                    *action = Some(remove(row));
                    pane.breakpoint_cursor = None;
                }
            }
        },
    );
    if !list.available && !list.targets.is_empty() {
        ui.label(RichText::new(BREAKPOINTS_IDLE).color(ACCENT_AMBER));
    }
}

/// F2 on the CPU tab sets a breakpoint on the picked instruction, or removes
/// the one there; with no instruction picked (or the picked one no longer
/// shown) it acts on the instruction at RIP. Not while a field takes the
/// keys.
fn code_key(
    ui: &egui::Ui,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    pane: &DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    if ui.ctx().egui_wants_keyboard_input() || !ui.input(|input| input.key_pressed(egui::Key::F2)) {
        return;
    }
    // In the source view F2 acts on the picked line, else the pause's.
    let source = marks.pause.filter(|_| CodeView::shown(pane.code_view, true) == CodeView::Source);
    if let Some(source) = source {
        let line = pane.source_cursor.unwrap_or(source.line);
        *action = Some(DebuggerAction::SetTargets(toggled_line(
            marks.targets,
            source,
            line,
            Some(BreakpointModules::of(snapshot)),
            marks.symbols,
        )));
        return;
    }
    let shown = |address: &u64| snapshot.code.iter().any(|line| line.address == *address);
    let address = pane
        .code_cursor
        .filter(shown)
        .or_else(|| snapshot.code.first().map(|line| line.address));
    if let Some(address) = address {
        *action = Some(DebuggerAction::SetTargets(toggled(marks.targets, address, Some(BreakpointModules::of(snapshot)), marks.symbols)));
    }
}

/// A field taking an address or `module!name` and Add, which adds a linear
/// breakpoint there.
fn add_breakpoint_row(
    ui: &mut egui::Ui,
    strip: &BreakpointList<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let linear = strip
        .applied
        .iter()
        .filter(|breakpoint| matches!(breakpoint, CodeBreakpoint::Linear(_)))
        .count();
    let full = linear >= MAX_LINEAR_BREAKPOINTS;
    let enabled = strip.available && !full;
    ui.horizontal_wrapped(|ui| {
        let field = ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(&mut pane.breakpoint_text)
                .font(egui::TextStyle::Monospace)
                .hint_text("address, module!name or file:line")
                .desired_width(240.0),
        );
        field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, BREAKPOINT_ADDRESS));
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let add = ui.add_enabled(enabled, egui::Button::new("Add"));
        let add = if !strip.available {
            add.on_disabled_hover_text(BREAKPOINTS_NEED_INTERPRETER)
        } else if full {
            add.on_disabled_hover_text(format!("At most {MAX_LINEAR_BREAKPOINTS} breakpoints."))
        } else {
            add
        };
        if enabled && (add.clicked() || entered) {
            let target = match parse_target(&pane.breakpoint_text) {
                Ok(TargetText::Source(source)) => Ok(BreakpointTarget::Source(source)),
                Ok(TargetText::Address(text)) => go_target(text, strip.names)
                    .map(|address| BreakpointTarget::Address(CodeBreakpoint::Linear(address)))
                    .map_err(BreakpointFieldError::Address),
                Err(error) => Err(BreakpointFieldError::Source(error)),
            };
            match target {
                Ok(target) => {
                    if !strip.targets.contains(&target) {
                        let mut set = strip.targets.to_vec();
                        set.push(target);
                        *action = Some(DebuggerAction::SetTargets(set));
                    }
                    pane.breakpoint_text.clear();
                    pane.breakpoint_error = None;
                }
                Err(error) => pane.breakpoint_error = Some(error),
            }
        }
    });
    if let Some(error) = pane.breakpoint_error {
        ui.label(RichText::new(error.message()).color(ACCENT_RED));
    }
}

/// Why the breakpoint field's text was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BreakpointFieldError {
    Address(AddressError),
    Source(TargetError),
}

impl BreakpointFieldError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Address(error) => error.message(),
            Self::Source(error) => error.message(),
        }
    }
}

/// The owner's breakpoints with the one at `address` removed, or added when
/// none is there: what F2 and a code line's menu do. The breakpoint at
/// `address` may be an address the owner gave or a source line resolving
/// there; removing it removes that line.
fn toggled(
    targets: &[BreakpointTarget],
    address: u64,
    index: Option<BreakpointModules<'_>>,
    view: &SymbolsView,
) -> Vec<BreakpointTarget> {
    let at = BreakpointTarget::Address(CodeBreakpoint::Linear(address));
    let resolves_here = |target: &BreakpointTarget| match target {
        BreakpointTarget::Source(source) => index.is_some_and(|index| {
            matches!(resolve(source, &index, view), SourceResolution::Resolved { places } if places.iter().any(|place| place.address == address))
        }),
        BreakpointTarget::Address(_) => *target == at,
    };
    let kept: Vec<BreakpointTarget> = targets.iter().filter(|target| !resolves_here(target)).cloned().collect();
    if kept.len() < targets.len() {
        return kept;
    }
    let mut set = targets.to_vec();
    set.push(at);
    set
}

/// The shown processor's state as chips: its privilege — user mode (CPL 3)
/// or kernel mode (CPL 0) — its mode and activity, the Windows process it
/// runs for when the guest's structures name one, its address space, and
/// whether the page shows a pause that has since run on. They go into the
/// header's row, after its buttons, and wrap where the row is full.
fn status_chips(ui: &mut egui::Ui, snapshot: &DebugSnapshot, names: Namer<'_>, paused: bool) {
    // Whose state the chips describe: a viewed thread's saved state, or the
    // shown processor's.
    let cr3 = match &snapshot.viewing {
        Viewing::Thread(thread) => {
            let cpl = thread.saved.cs & 3;
            let (text, color) = if cpl == 3 {
                ("User mode", ACCENT_CYAN)
            } else {
                ("Kernel mode", ACCENT_AMBER)
            };
            chip(ui, &format!("{text} · CPL {cpl}"), color);
            let mode = if thread.saved.cs == WOW64_CODE_SELECTOR {
                "Compatibility mode"
            } else {
                "64-bit mode"
            };
            chip(ui, mode, ACCENT_BLUE);
            chip(ui, "Saved state", TEXT_MUTED)
                .on_hover_text("What the kernel saved when this thread last entered it.");
            thread.cr3
        }
        Viewing::Live | Viewing::Running { .. } | Viewing::Miss(_) => {
            let Some(view) = shown_view(snapshot) else {
                return;
            };
            let (text, color) = match view.registers.cpl {
                0 => ("Kernel mode", ACCENT_AMBER),
                3 => ("User mode", ACCENT_CYAN),
                _ => ("Privileged", ACCENT_AMBER),
            };
            chip(ui, &format!("{text} · CPL {}", view.registers.cpl), color);
            chip(ui, mode_label(view), ACCENT_BLUE);
            chip(ui, activity_label(view.activity), TEXT_MUTED);
            view.registers.cr3
        }
    };
    if let Some(path) = &snapshot.process {
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
        chip(ui, &format!("Process {name}"), TEXT_PRIMARY).on_hover_text(path.as_str());
    }
    if let Some(named) = snapshot.code.first().and_then(|line| names.name(line.address)) {
        chip(ui, &format!("at {}", named.label()), name_color(&named, ACCENT_CYAN))
            .on_hover_text("Where RIP points.");
    }
    chip(ui, &format!("CR3 {cr3:X}"), TEXT_MUTED)
        .on_hover_text("The address space: each process has its own page tables.");
    if !paused {
        chip(ui, SHOWING_LAST_PAUSE, ACCENT_AMBER);
    }
}

/// The base of the module the shown processor's RIP is in, when it lies in
/// `space`: the module to mark as current in that space's list.
fn current_module_base(snapshot: &DebugSnapshot, space: ModuleSpace) -> Option<u64> {
    snapshot
        .code
        .first()
        .and_then(|line| snapshot.symbols.resolve(line.address))
        .filter(|symbol| symbol.space == space)
        .map(|symbol| symbol.base)
}

/// A new window start (a new pause, a Page down, RIP or RSP) is written into
/// the address field.
fn follow_window(pane: &mut DebuggerPane, snapshot: &DebugSnapshot) {
    let start = snapshot.memory.start;
    if pane.shown_start != Some(start) {
        pane.address_text = format!("{:X}", start.value);
        pane.space = start.space;
        pane.address_error = None;
        pane.shown_start = Some(start);
    }
}

fn shown_view(snapshot: &DebugSnapshot) -> Option<&CpuDebugView> {
    snapshot.cpus.get(snapshot.cpu)
}

/// The width the code and stack panes were read at: a viewed thread's own
/// mode (32-bit for a WOW64 thread in its 32-bit code), else the shown
/// processor's.
fn pane_code_size(snapshot: &DebugSnapshot) -> CodeSize {
    match &snapshot.viewing {
        Viewing::Thread(thread) if thread.saved.cs == WOW64_CODE_SELECTOR => CodeSize::Bits32,
        Viewing::Thread(_) => CodeSize::Bits64,
        Viewing::Live | Viewing::Running { .. } | Viewing::Miss(_) => {
            shown_view(snapshot).map_or(CodeSize::Bits64, |view| view.code_size)
        }
    }
}

/// The address space the code and stack were read in, and the one the symbol
/// index's user modules belong to: a viewed thread's process, else the shown
/// processor's linear space.
fn vantage_space(snapshot: &DebugSnapshot) -> AddressSpace {
    match &snapshot.viewing {
        Viewing::Thread(thread) => AddressSpace::Process { cr3: thread.cr3 },
        Viewing::Live | Viewing::Running { .. } | Viewing::Miss(_) => AddressSpace::Linear,
    }
}

/// The page's one header row: the buttons and the tab switch, then — once a
/// pause has been read — the shown processor's chips, on the same row after
/// them. The row wraps only where it has no more room.
fn header(
    ui: &mut egui::Ui,
    bar: &Toolbar,
    density: Density,
    snapshot: Option<&DebugSnapshot>,
    symbols: &SymbolsView,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(density.gap(), density.gap());
        ui.spacing_mut().button_padding = density.button_padding();
        step_controls(ui, bar, action);
        let dump = if bar.dump_pending {
            "Saving RAM…"
        } else {
            "Dump RAM…"
        };
        if ui
            .add_enabled(!bar.dump_pending, egui::Button::new(dump))
            .clicked()
        {
            *action = Some(DebuggerAction::DumpRam);
        }
        if let Some(snapshot) = snapshot {
            processor_status(ui, snapshot, symbols, pane, action, bar.paused);
        }
        // The tabs take a row of their own under the buttons and the
        // processor's status.
        ui.end_row();
        let tabs = DebuggerTab::ALL.map(|tab| Segment { value: tab, label: tab.label() });
        if segmented(ui, &mut pane.tab, &tabs) {
            // A tab's keys start on its list, standing on no detail row.
            pane.list_focus = ListFocus::List;
            pane.detail_cursor = None;
            pane.module_cursor = None;
            pane.folded_imports.clear();
            *action = Some(DebuggerAction::ShowSystem(
                pane.tab.system_view(pane),
            ));
        }
    });
}

/// The header's processor switch and status chips, after its buttons.
fn processor_status(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    symbols: &SymbolsView,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
    paused: bool,
) {
    if snapshot.cpus.len() > 1 {
        let mut chosen = pane.cpu;
        egui::ComboBox::from_id_salt("debugger_cpu")
            .selected_text(cpu_label(chosen))
            .show_ui(ui, |ui| {
                for index in 0..snapshot.cpus.len() {
                    ui.selectable_value(&mut chosen, index, cpu_label(index));
                }
            });
        if chosen != pane.cpu {
            pane.cpu = chosen;
            *action = Some(DebuggerAction::SelectCpu(chosen));
        }
    }
    status_chips(ui, snapshot, Namer::new(&snapshot.symbols, symbols), paused);
    if !snapshot.symbols.modules.is_empty() {
        let counts = symbol_counts(&snapshot.symbols, symbols);
        let color = if counts.missing > 0 { ACCENT_AMBER } else { TEXT_MUTED };
        let opened = chip(ui, &counts.label(), color)
            .interact(egui::Sense::click())
            .on_hover_text("Open the Symbols tab.")
            .clicked();
        if opened && pane.tab != DebuggerTab::Symbols {
            pane.tab = DebuggerTab::Symbols;
            *action = Some(DebuggerAction::ShowSystem(DebuggerTab::Symbols.system_view(pane)));
        }
    }
}

fn cpu_label(index: usize) -> String {
    if index == 0 {
        "CPU 0 (boot)".to_owned()
    } else {
        format!("CPU {index}")
    }
}

/// e.g. "64-bit mode".
fn mode_label(view: &CpuDebugView) -> &'static str {
    match view.mode {
        ExecutionMode::Real => "Real mode",
        ExecutionMode::Virtual8086 => "Virtual-8086 mode",
        ExecutionMode::Protected => match view.code_size {
            CodeSize::Bits16 => "16-bit protected mode",
            CodeSize::Bits32 | CodeSize::Bits64 => "32-bit protected mode",
        },
        ExecutionMode::LongCompat => "Compatibility mode",
        ExecutionMode::Long64 => "64-bit mode",
    }
}

fn activity_label(activity: ProcessorActivity) -> &'static str {
    match activity {
        ProcessorActivity::Active => "Running",
        ProcessorActivity::Halted => "Halted",
        ProcessorActivity::Shutdown => "Shut down",
        ProcessorActivity::WaitForSipi => "Waiting for startup",
        ProcessorActivity::Mwait => "Waiting (MWAIT)",
    }
}

fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().size(TEXT_SECONDARY)
}

/// The bytes column of the code listing: at most this many bytes, the rest
/// marked with an ellipsis, so the opcode name stays in view.
const CODE_BYTES_SHOWN: usize = 8;

fn hex_digits(size: CodeSize) -> usize {
    match size {
        CodeSize::Bits64 => 16,
        CodeSize::Bits16 | CodeSize::Bits32 => 8,
    }
}

/// The share of the page's width the left column (code above memory) takes
/// in the grid layout: x64dbg gives its disassembly and dump the wide side.
const LEFT_SHARE: f32 = 0.62;
/// The share of the page's height the top row (code beside registers) takes.
const TOP_SHARE: f32 = 0.58;

/// The page where it is wide enough: four panes in a fixed grid that fills
/// the page (code and registers above, memory and stack below), each pane
/// scrolling on its own, as x64dbg's CPU tab does.
fn grid_layout(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let gap = SPACE_ITEM;
    let page = ui.available_size();
    // The split is read before the panes borrow the pane state, and written
    // back once they are drawn.
    let mut split = pane.split;
    // The stored share is the preference; the layout keeps both columns their
    // minimum at today's width, so a narrower window never squeezes one out.
    let columns = resized(split.columns, 0.0, page.x, gap, SPLIT_MIN);
    let left = ((page.x - gap) * columns).floor();
    let right = page.x - gap - left;
    // The code pane's pick is read before the panes borrow the pane state,
    // and written back once they are drawn, as the split is.
    let mut panes = GridPanes {
        action: &mut *action,
        picks: CodePicks::of(pane),
        lower: LowerPicks::of(pane),
    };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        split_column(
            ui,
            egui::vec2(left, page.y),
            gap,
            Split { share: &mut split.left_rows, default: TOP_SHARE, label: RESIZE_CODE_MEMORY },
            &mut panes,
            |ui, size, panes| {
                fixed_card(ui, size, Title::plain("Code"), |_| {}, |ui| {
                    code_pane(ui, snapshot, marks, &mut panes.picks, &mut panes.lower.variables, panes.action);
                });
            },
            |ui, size, panes| {
                lower_left_card(ui, Room::Fixed(size), snapshot, marks, pane, &mut panes.lower, panes.action);
            },
        );
        match splitter(ui, egui::vec2(gap, page.y), SplitAxis::Columns, RESIZE_COLUMNS) {
            SplitterMove::Still => {}
            SplitterMove::Dragged(delta) => {
                split.columns = resized(columns, delta, page.x, gap, SPLIT_MIN);
            }
            SplitterMove::Reset => split.columns = LEFT_SHARE,
        }
        split_column(
            ui,
            egui::vec2(right, page.y),
            gap,
            Split {
                share: &mut split.right_rows,
                default: TOP_SHARE,
                label: RESIZE_REGISTERS_STACK,
            },
            &mut panes,
            |ui, size, _| {
                fixed_card(ui, size, Title::plain("Registers"), |_| {}, |ui| {
                    register_rows(ui, snapshot, marks);
                });
            },
            |ui, size, panes| {
                lower_right_card(ui, Room::Fixed(size), snapshot, marks, &mut panes.lower, panes.action);
            },
        );
    });
    panes.picks.keep(pane);
    panes.lower.keep(pane);
    pane.split = split;
}

/// What the grid's columns lend their panes in turn: the action, the code
/// pane's picks, and the lower panes'.
struct GridPanes<'a> {
    action: &'a mut Option<DebuggerAction>,
    picks: CodePicks,
    lower: LowerPicks,
}

/// The lower panes' picks, lent to the layouts and written back: which pane
/// each shows, and the variable panes' state.
#[derive(Clone, Debug)]
struct LowerPicks {
    left: Option<LowerLeft>,
    right: Option<LowerRight>,
    variables: VariableState,
}

impl LowerPicks {
    fn of(pane: &mut DebuggerPane) -> Self {
        LowerPicks { left: pane.lower_left, right: pane.lower_right, variables: std::mem::take(&mut pane.variables) }
    }

    fn keep(self, pane: &mut DebuggerPane) {
        pane.lower_left = self.left;
        pane.lower_right = self.right;
        pane.variables = self.variables;
    }
}

/// The lower-left pane, switched between Memory, the picked frame's Locals
/// and the Watches; before the owner picks, Locals where the pause is in the
/// owner's source.
fn lower_left_card(
    ui: &mut egui::Ui,
    room: Room,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    pane: &mut DebuggerPane,
    lower: &mut LowerPicks,
    action: &mut Option<DebuggerAction>,
) {
    let shown = LowerLeft::shown(lower.left, marks.pause.is_some());
    let mut switched = shown;
    let mut ask = None;
    let names = Namer::new(&snapshot.symbols, marks.symbols);
    let variables = &mut lower.variables;
    pane_card(
        ui,
        room,
        Title::plain(shown.label()).inlined(),
        |ui| {
            let switch = [
                Segment { value: LowerLeft::Memory, label: MEMORY },
                Segment { value: LowerLeft::Locals, label: LOCALS },
                Segment { value: LowerLeft::Watch, label: WATCH },
            ];
            ui.horizontal_wrapped(|ui| {
                if segmented(ui, &mut switched, &switch) {
                    lower.left = Some(switched);
                }
            });
            if shown == LowerLeft::Memory {
                memory_bar(ui, snapshot, marks, pane, action);
            }
        },
        |ui| match shown {
            LowerLeft::Memory => memory_rows(ui, snapshot, marks),
            LowerLeft::Locals => ask = locals_rows(ui, &snapshot.frames.locals, names, &snapshot.symbols),
            LowerLeft::Watch => {
                ask = watch_rows(ui, &snapshot.frames.watches, variables, names, &snapshot.symbols);
            }
        },
    );
    if let Some(target) = ask.and_then(|ask| lower.variables.apply(ask)) {
        *action = Some(DebuggerAction::FindReferences(target));
    }
}

/// The lower-right pane, switched between the stack's slots and the call
/// stack; before the owner picks, the call stack where the pause is in the
/// owner's source.
fn lower_right_card(
    ui: &mut egui::Ui,
    room: Room,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    lower: &mut LowerPicks,
    action: &mut Option<DebuggerAction>,
) {
    let shown = LowerRight::shown(lower.right, marks.pause.is_some());
    let mut switched = shown;
    let mut picked = None;
    let names = Namer::new(&snapshot.symbols, marks.symbols);
    pane_card(
        ui,
        room,
        Title::plain(shown.label()).inlined(),
        |ui| {
            let switch = [
                Segment { value: LowerRight::Stack, label: STACK },
                Segment { value: LowerRight::CallStack, label: CALL_STACK },
            ];
            ui.horizontal_wrapped(|ui| {
                if segmented(ui, &mut switched, &switch) {
                    lower.right = Some(switched);
                }
            });
        },
        |ui| match shown {
            LowerRight::Stack => stack_rows(ui, snapshot, marks),
            LowerRight::CallStack => {
                picked = call_stack_rows(ui, &snapshot.frames, names, &snapshot.symbols, marks.symbols);
            }
        },
    );
    if let Some(number) = picked {
        *action = Some(DebuggerAction::SelectFrame(number));
    }
}

/// How the grid's panes divide the page, as shares of the room between
/// each pair: dragged by the gaps between them, kept per VM while the app
/// runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaneSplit {
    /// The CPU tab's left column (code above memory), of the width.
    pub(crate) columns: f32,
    /// Code, of the left column's height.
    pub(crate) left_rows: f32,
    /// Registers, of the right column's height.
    pub(crate) right_rows: f32,
    /// The process list, of the Processes tab's width.
    pub(crate) processes: f32,
    /// The module list, of the Kernel modules tab's width.
    pub(crate) kernel: f32,
    /// A process's modules, of the height they share with the picked one's
    /// detail.
    pub(crate) modules: f32,
    /// The driver list, of the Drivers tab's width.
    pub(crate) drivers: f32,
}

impl Default for PaneSplit {
    fn default() -> Self {
        Self {
            columns: LEFT_SHARE,
            left_rows: TOP_SHARE,
            right_rows: TOP_SHARE,
            processes: PROCESS_LIST_SHARE,
            kernel: PROCESS_LIST_SHARE,
            modules: PROCESS_LIST_SHARE,
            drivers: PROCESS_LIST_SHARE,
        }
    }
}

/// The least a dragged split leaves either pane, points: a title and a few
/// rows.
const SPLIT_MIN: f32 = 96.0;

/// The accessible names of the gaps that resize the panes.
pub(crate) const RESIZE_COLUMNS: &str = "Resize columns";
pub(crate) const RESIZE_CODE_MEMORY: &str = "Resize code and memory";
pub(crate) const RESIZE_REGISTERS_STACK: &str = "Resize registers and stack";
pub(crate) const RESIZE_PROCESS_LIST: &str = "Resize process list";
pub(crate) const RESIZE_MODULE_LIST: &str = "Resize module list";
pub(crate) const RESIZE_MODULE_DETAIL: &str = "Resize modules and module detail";

/// Which way a gap divides its panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitAxis {
    /// Side by side: the gap is dragged left and right.
    Columns,
    /// One above the other: the gap is dragged up and down.
    Rows,
}

/// What a gap was asked this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SplitterMove {
    Still,
    /// Dragged this many points along its axis.
    Dragged(f32),
    /// Double-clicked: back to the default split.
    Reset,
}

/// `share` moved by a `delta`-point drag across `extent` points, of which
/// `gap` is the gap itself, kept so neither pane falls below `min`. Where the
/// room cannot hold two minimums the split sits in the middle.
fn resized(share: f32, delta: f32, extent: f32, gap: f32, min: f32) -> f32 {
    let room = extent - gap;
    if room <= 0.0 {
        return 0.5;
    }
    let low = (min / room).min(0.5);
    (share + delta / room).clamp(low, 1.0 - low)
}

/// The gap between two panes, `size` points, as a handle: drag it to move
/// the split, double-click it to reset. It shows a line while pointed at or
/// dragged, and the resize cursor.
fn splitter(ui: &mut egui::Ui, size: egui::Vec2, axis: SplitAxis, label: &str) -> SplitterMove {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, enabled, label));
    let cursor = match axis {
        SplitAxis::Columns => egui::CursorIcon::ResizeColumn,
        SplitAxis::Rows => egui::CursorIcon::ResizeRow,
    };
    let active = response.hovered() || response.dragged();
    if active {
        ui.ctx().set_cursor_icon(cursor);
        let line = match axis {
            SplitAxis::Columns => egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(2.0, rect.height() * 0.4),
            ),
            SplitAxis::Rows => egui::Rect::from_center_size(
                rect.center(),
                egui::vec2(rect.width() * 0.4, 2.0),
            ),
        };
        ui.painter().rect_filled(line, 1.0, ACCENT_CYAN);
    }
    // egui counts a third quick click as a triple, not a double; it still
    // holds the double-click that resets.
    if response.double_clicked() || response.triple_clicked() {
        return SplitterMove::Reset;
    }
    if response.dragged() {
        let delta = response.drag_delta();
        let along = match axis {
            SplitAxis::Columns => delta.x,
            SplitAxis::Rows => delta.y,
        };
        if along != 0.0 {
            return SplitterMove::Dragged(along);
        }
    }
    SplitterMove::Still
}

/// One of a column's splits: the share it moves, the share a double-click
/// restores, and the gap's accessible name.
struct Split<'a> {
    share: &'a mut f32,
    default: f32,
    label: &'a str,
}

/// A column of `size` points: `top` above a row gap above `bottom`, the gap
/// moving `split`. Each pane is handed `context` in turn, so both can write
/// to it.
fn split_column<C, A, B>(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    gap: f32,
    split: Split<'_>,
    context: &mut C,
    top: A,
    bottom: B,
) where
    A: FnOnce(&mut egui::Ui, egui::Vec2, &mut C),
    B: FnOnce(&mut egui::Ui, egui::Vec2, &mut C),
{
    // Laid out at the share clamped to today's height (see `grid_layout`).
    let share = resized(*split.share, 0.0, size.y, gap, SPLIT_MIN);
    let top_height = ((size.y - gap) * share).floor();
    let bottom_height = size.y - gap - top_height;
    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        top(ui, egui::vec2(size.x, top_height), context);
        match splitter(ui, egui::vec2(size.x, gap), SplitAxis::Rows, split.label) {
            SplitterMove::Still => {}
            SplitterMove::Dragged(delta) => {
                *split.share = resized(share, delta, size.y, gap, SPLIT_MIN);
            }
            SplitterMove::Reset => *split.share = split.default,
        }
        bottom(ui, egui::vec2(size.x, bottom_height), context);
    });
}

/// The page where the grid has no room (a short or narrow window): every pane
/// at the height of its content, the page scrolling as a whole. In two
/// columns the left one — code above memory — takes the wide side, as the
/// grid's does; in one the panes follow each other.
fn scrolling_layout(
    ui: &mut egui::Ui,
    columns: Columns,
    density: Density,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let gap = density.gap();
    // The panes' picks, written back once the panes are drawn.
    let mut panes = GridPanes {
        action: &mut *action,
        picks: CodePicks::of(pane),
        lower: LowerPicks::of(pane),
    };
    let room = Room::Content(density);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match columns {
            Columns::Two => side_by_side(
                ui,
                gap,
                LEFT_SHARE,
                &mut panes,
                |ui, panes| {
                    card(ui, density, Title::plain("Code"), |_| {}, |ui| {
                        code_pane(ui, snapshot, marks, &mut panes.picks, &mut panes.lower.variables, panes.action);
                    });
                    ui.add_space(gap);
                    lower_left_card(ui, room, snapshot, marks, pane, &mut panes.lower, panes.action);
                },
                |ui, panes| {
                    card(ui, density, Title::plain("Registers"), |_| {}, |ui| {
                        register_rows(ui, snapshot, marks);
                    });
                    ui.add_space(gap);
                    lower_right_card(ui, room, snapshot, marks, &mut panes.lower, panes.action);
                },
            ),
            Columns::One => {
                card(ui, density, Title::plain("Registers"), |_| {}, |ui| {
                    register_rows(ui, snapshot, marks);
                });
                ui.add_space(gap);
                card(ui, density, Title::plain("Code"), |_| {}, |ui| {
                    code_pane(ui, snapshot, marks, &mut panes.picks, &mut panes.lower.variables, panes.action);
                });
                ui.add_space(gap);
                lower_right_card(ui, room, snapshot, marks, &mut panes.lower, panes.action);
                ui.add_space(gap);
                lower_left_card(ui, room, snapshot, marks, pane, &mut panes.lower, panes.action);
            }
        });
    panes.picks.keep(pane);
    panes.lower.keep(pane);
}

/// Two columns, the left `left_share` of the width and a `gap` between them,
/// each drawn top-down at its own height. `context` is lent to the left
/// column and then to the right, so both may write the same state.
fn side_by_side<C, L, R>(
    ui: &mut egui::Ui,
    gap: f32,
    left_share: f32,
    context: &mut C,
    left: L,
    right: R,
) where
    L: FnOnce(&mut egui::Ui, &mut C),
    R: FnOnce(&mut egui::Ui, &mut C),
{
    let width = ui.available_width();
    let left_width = ((width - gap) * left_share).floor();
    let right_width = width - gap - left_width;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.vertical(|ui| {
            ui.set_width(left_width);
            left(ui, context);
        });
        ui.vertical(|ui| {
            ui.set_width(right_width);
            right(ui, context);
        });
    });
}

/// A pane's title: its name, and how many entries it lists when it lists.
#[derive(Clone, Copy)]
struct Title<'a> {
    text: &'a str,
    count: Option<usize>,
    /// The header draws the title beside its controls, so the card leaves it
    /// its own row (it still names the scroll area). A list pane does this to
    /// keep a row of height for its rows.
    inline: bool,
}

impl<'a> Title<'a> {
    const fn plain(text: &'a str) -> Self {
        Self { text, count: None, inline: false }
    }

    const fn counted(text: &'a str, count: usize) -> Self {
        Self {
            text,
            count: Some(count),
            inline: false,
        }
    }

    /// The same title, drawn by the header in its controls row.
    const fn inlined(mut self) -> Self {
        self.inline = true;
        self
    }
}

/// A pane of exactly `size`: its title, a fixed `header` (the memory pane's
/// address bar, a table's column titles), and a `body` that scrolls inside
/// the pane when it is taller or wider than the room left.
fn fixed_card<H, B>(ui: &mut egui::Ui, size: egui::Vec2, title: Title<'_>, header: H, body: B)
where
    H: FnOnce(&mut egui::Ui),
    B: FnOnce(&mut egui::Ui),
{
    let frame = shell_card_frame();
    let inner = (size - frame.total_margin().sum()).max(egui::Vec2::ZERO);
    // The pane stacks its title, header and body whatever the row around it
    // lays out.
    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
        frame.show(ui, |ui| {
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            ui.spacing_mut().item_spacing.y = 4.0;
            if !title.inline {
                card_title(ui, title.text, title.count);
            }
            header(ui);
            // Listing rows sit flush, one under the next.
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::both()
                .id_salt(title.text)
                .auto_shrink([false, false])
                .show(ui, body);
        });
    });
}

/// A pane as tall as its content, the width of the column it is in. Its
/// listing scrolls sideways by itself where a row is wider than the column,
/// so the card never grows past the page.
fn card<H, B>(ui: &mut egui::Ui, density: Density, title: Title<'_>, header: H, body: B)
where
    H: FnOnce(&mut egui::Ui),
    B: FnOnce(&mut egui::Ui),
{
    density.card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = density.title_gap();
        if !title.inline {
            card_title(ui, title.text, title.count);
        }
        header(ui);
        // Listing rows sit flush, one under the next.
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::horizontal()
            .id_salt(title.text)
            .auto_shrink([false, true])
            .show(ui, body);
    });
}

/// The room a pane is given: the size it must fill, or whatever its content
/// takes at a density.
#[derive(Clone, Copy)]
enum Room {
    Fixed(egui::Vec2),
    Content(Density),
}

/// A pane in `room`: a `fixed_card` where the page gives it a size, a `card`
/// where the page scrolls.
fn pane_card<H, B>(ui: &mut egui::Ui, room: Room, title: Title<'_>, header: H, body: B)
where
    H: FnOnce(&mut egui::Ui),
    B: FnOnce(&mut egui::Ui),
{
    match room {
        Room::Fixed(size) => fixed_card(ui, size, title, header, body),
        Room::Content(density) => card(ui, density, title, header, body),
    }
}

/// The fewest bytes the code listing shows of an instruction, however narrow
/// the pane: the opcode name is what is read, and keeps its room first.
const CODE_BYTES_MIN: usize = 2;
/// The room the longest opcode names the listing prints keep, in characters.
const CODE_OPCODE_CHARS: f32 = 16.0;
/// The two gaps between the code listing's three columns, in characters.
const CODE_GAP_CHARS: f32 = 4.0;
/// The lead before a listing's first column, where the RIP marker sits.
const MARKER_LEAD: f32 = 12.0;

/// How many of an instruction's bytes the code listing shows in a pane
/// `room_chars` characters wide whose addresses are `digits` characters: the
/// address, the gaps and the opcode keep their room, and the bytes — three
/// characters each and an ellipsis — take the rest, between `CODE_BYTES_MIN`
/// and `CODE_BYTES_SHOWN`.
fn code_bytes_shown(room_chars: f32, digits: usize) -> usize {
    let left = room_chars - digits as f32 - CODE_GAP_CHARS - CODE_OPCODE_CHARS - 1.0;
    let fits = (left / 3.0).floor().max(CODE_BYTES_MIN as f32);
    (fits as usize).min(CODE_BYTES_SHOWN)
}

/// The code at RIP, one instruction a row: address, bytes, opcode, then the
/// named target of a branch or `lea` and the source line, when the tables
/// have them. A function's first instruction is headed by the function's
/// name. The row at RIP is shaded and marked; a row at a breakpoint is marked
/// red. A click picks a row (`cursor`), which F2 sets or removes a breakpoint
/// on, and a row's context menu does the same.
/// The code pane's picks, lent to the layouts and written back: the
/// instruction, the source line, and source or disassembly.
#[derive(Clone, Copy, Debug)]
struct CodePicks {
    instruction: Option<u64>,
    line: Option<u32>,
    view: Option<CodeView>,
    followed: Option<CodePlace>,
}

impl CodePicks {
    fn of(pane: &DebuggerPane) -> Self {
        CodePicks { instruction: pane.code_cursor, line: pane.source_cursor, view: pane.code_view, followed: pane.code_followed }
    }

    fn keep(self, pane: &mut DebuggerPane) {
        pane.code_cursor = self.instruction;
        pane.source_cursor = self.line;
        pane.code_view = self.view;
        pane.code_followed = self.followed;
    }

    /// Whether the pane brings `place` into view this frame: once, the first
    /// frame it is shown.
    fn follow(&mut self, place: CodePlace) -> bool {
        let new = self.followed != Some(place);
        self.followed = Some(place);
        new
    }
}

/// Where the code pane stands: which view, the address it shows (RIP, an
/// address opened, a caller's call) and the source line, in the source view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CodePlace {
    view: CodeView,
    address: u64,
    line: Option<u32>,
}

/// The code pane: a Source | Disassembly switch, then the owner's source at
/// the pause's line or the disassembly at RIP.
fn code_pane(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    picks: &mut CodePicks,
    variables: &mut VariableState,
    action: &mut Option<DebuggerAction>,
) {
    let mut shown = CodeView::shown(picks.view, marks.pause.is_some());
    let switch = [
        Segment { value: CodeView::Source, label: SOURCE_SWITCH },
        Segment { value: CodeView::Disassembly, label: DISASSEMBLY_SWITCH },
    ];
    if segmented(ui, &mut shown, &switch) {
        picks.view = Some(shown);
    }
    let address = match (marks.frame, snapshot.code_at) {
        (FrameMark::Caller { code, .. }, _) => code,
        (FrameMark::Innermost, CodeTarget::At(at)) => at.value,
        (FrameMark::Innermost, CodeTarget::Rip) => snapshot.rip,
    };
    let Some(source) = marks.pause.filter(|_| shown == CodeView::Source) else {
        if shown == CodeView::Source {
            ui.label(RichText::new(NO_SOURCE_HERE).color(TEXT_MUTED));
        }
        let follow = picks.follow(CodePlace { view: CodeView::Disassembly, address, line: None });
        code_rows(ui, snapshot, marks, &mut picks.instruction, follow, action);
        return;
    };
    let follow = picks.follow(CodePlace { view: CodeView::Source, address, line: Some(source.line) });
    let gutter = line_marks(source, marks.breakpoints, marks.targets, marks.resolutions);
    let place = match marks.frame {
        FrameMark::Caller { .. } => PlaceMark::Caller,
        FrameMark::Innermost => match snapshot.code_at {
            CodeTarget::Rip => PlaceMark::Current,
            CodeTarget::At(_) => PlaceMark::Shown,
        },
    };
    let tip = variables.tip.as_ref().map(|tip| TipView {
        tip,
        read: &snapshot.frames.tip,
        names: Namer::new(&snapshot.symbols, marks.symbols),
        index: &snapshot.symbols,
    });
    let pane = SourcePane { source, slot: marks.symbols.sources.get(&source.key), marks: &gutter, place, tip, follow };
    match source_rows(ui, &pane, &mut picks.line) {
        None => {}
        Some(SourceAsk::Toggle(line)) => {
            let targets = toggled_line(marks.targets, source, line, Some(BreakpointModules::of(snapshot)), marks.symbols);
            *action = Some(DebuggerAction::SetTargets(targets));
        }
        Some(SourceAsk::Locate) => {
            *action = Some(DebuggerAction::LocateSource { build_path: source.build_path.clone() });
        }
        Some(SourceAsk::SaveMap { project, map }) => *action = Some(DebuggerAction::SaveRunMap { project, map }),
        Some(SourceAsk::References(line)) => {
            if let Some(target) = line_target(source, line) {
                *action = Some(DebuggerAction::FindReferences(target));
            }
        }
        Some(SourceAsk::ShowTip(tip)) => *action = Some(DebuggerAction::ShowTip(tip)),
        Some(SourceAsk::CloseTip) => *action = Some(DebuggerAction::CloseTip),
        Some(SourceAsk::TipRow(ask)) => {
            if let Some(target) = variables.apply(ask) {
                *action = Some(DebuggerAction::FindReferences(target));
            }
        }
    }
}

fn code_rows(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    cursor: &mut Option<u64>,
    follow: bool,
    action: &mut Option<DebuggerAction>,
) {
    let names = Namer::new(&snapshot.symbols, marks.symbols);
    let digits = hex_digits(pane_code_size(snapshot));
    let room = (ui.available_width() - MARKER_LEAD - 2.0 * ROW_PAD) / char_width(ui);
    let shown = code_bytes_shown(room, digits);
    for (index, line) in snapshot.code.iter().enumerate() {
        let named = names.name(line.address);
        if let Some(start) = named.as_ref().filter(|named| named.starts_function()) {
            let heading = [Cell::fill(format!("{}:", start.label()), name_color(start, ACCENT_BLUE))];
            list_row(ui, MARKER_LEAD, &heading, Tone::Plain, RowSense::Look);
        }
        let mut bytes: Vec<String> = line
            .bytes
            .iter()
            .take(shown)
            .map(|byte| format!("{byte:02X}"))
            .collect();
        if line.bytes.len() > shown {
            bytes.push("…".to_owned());
        }
        // The first line is the instruction at RIP, or the address opened.
        let at_rip = line.address == snapshot.rip;
        let opened = index == 0 && snapshot.code_at != CodeTarget::Rip;
        let mut text_color = match &line.text {
            CodeText::Opcode(_) if at_rip => ACCENT_CYAN,
            CodeText::Opcode(_) => TEXT_PRIMARY,
            CodeText::Undecodable | CodeText::Unreadable => TEXT_MUTED,
        };
        let mut text = match &line.text {
            CodeText::Opcode(name) => name.clone(),
            CodeText::Undecodable => "(undecodable)".to_owned(),
            CodeText::Unreadable => "(unreadable)".to_owned(),
        };
        if let Some(target) = line.target {
            match names.name(target) {
                Some(named) => {
                    text.push_str(&format!(" -> {}", named.label()));
                    text_color = name_color(&named, text_color);
                }
                None => text.push_str(&format!(" -> {target:X}")),
            }
        }
        if let Some(source) = named.as_ref().and_then(NamedAddress::source) {
            text.push_str(&format!("  · {source}"));
        }
        let picked = *cursor == Some(line.address);
        let address_color = if at_rip { ACCENT_CYAN } else { TEXT_MUTED };
        let tone = if picked {
            Tone::Selected
        } else if at_rip {
            Tone::Current
        } else {
            Tone::Plain
        };
        let cells = [
            Cell::left(format!("{:0digits$X}", line.address), address_color, digits),
            Cell::left(bytes.join(" "), TEXT_MUTED, 3 * shown + 1),
            Cell::fill(text, text_color),
        ];
        let response = list_row(ui, MARKER_LEAD, &cells, tone, RowSense::Pick { selected: picked });
        if response.clicked() {
            *cursor = Some(line.address);
        }
        // A new place is brought into view: the row at RIP, or the address
        // opened (a caller's call, a Go to).
        if follow && ((at_rip && snapshot.code_at == CodeTarget::Rip) || opened) {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        let at_breakpoint = marks.breakpoints.contains(&CodeBreakpoint::Linear(line.address));
        match (at_rip, at_breakpoint) {
            (true, true) => lead_arrow(ui, response.rect, ACCENT_RED),
            (true, false) => lead_arrow(ui, response.rect, ACCENT_CYAN),
            (false, true) => lead_dot(ui, response.rect, MARKER_LEAD, ACCENT_RED),
            (false, false) if opened && marks.frame != FrameMark::Innermost => {
                lead_hollow_arrow(ui, response.rect, ACCENT_CYAN, CALLER_LINE);
            }
            (false, false) if opened => lead_ring(ui, response.rect, MARKER_LEAD, ACCENT_CYAN),
            (false, false) => {}
        }
        response.context_menu(|ui| {
            let label = if at_breakpoint { REMOVE_BREAKPOINT_HERE } else { SET_BREAKPOINT_HERE };
            if ui.button(label).clicked() {
                *action =
                    Some(DebuggerAction::SetTargets(toggled(marks.targets, line.address, Some(BreakpointModules::of(snapshot)), marks.symbols)));
                ui.close();
            }
            // Who refers to this instruction, to the function it starts, and
            // to where it goes.
            let mut asks = vec![address_target(line.address, format!("{:#X}", line.address))];
            if let Some(start) = named.as_ref().filter(|named| named.starts_function()) {
                asks.push(address_target(line.address, start.label()));
            }
            if let Some(target) = line.target {
                let label = names.name(target).map_or_else(|| format!("{target:#X}"), |named| named.label());
                asks.push(address_target(target, label));
            }
            for ask in asks {
                if ui.button(format!("{REFERENCES_TO} {}", ask.label)).clicked() {
                    *action = Some(DebuggerAction::FindReferences(ask));
                    ui.close();
                }
            }
        });
    }
}

fn register_rows(ui: &mut egui::Ui, snapshot: &DebugSnapshot, marks: Marks<'_>) {
    if let Viewing::Thread(thread) = &snapshot.viewing {
        saved_register_rows(ui, &thread.saved);
        return;
    }
    let Some(view) = shown_view(snapshot) else {
        return;
    };
    // The same processor as it stood before the last step or pause.
    let before = marks
        .previous
        .and_then(|previous| previous.cpus.get(snapshot.cpu));
    let r = &view.registers;
    let old = before.map(|before| &before.registers);
    let wide = view.code_size == CodeSize::Bits64;
    ui.spacing_mut().item_spacing.y = 2.0;
    {
        section_caption(ui, "General");
        egui::Grid::new("debugger_registers")
            .num_columns(2)
            .spacing(REGISTER_GRID_SPACING)
            .show(ui, |ui| {
                let gprs = [
                    ("RAX", "EAX", r.rax, old.map(|o| o.rax)),
                    ("RBX", "EBX", r.rbx, old.map(|o| o.rbx)),
                    ("RCX", "ECX", r.rcx, old.map(|o| o.rcx)),
                    ("RDX", "EDX", r.rdx, old.map(|o| o.rdx)),
                    ("RSI", "ESI", r.rsi, old.map(|o| o.rsi)),
                    ("RDI", "EDI", r.rdi, old.map(|o| o.rdi)),
                    ("RBP", "EBP", r.rbp, old.map(|o| o.rbp)),
                    ("RSP", "ESP", r.rsp, old.map(|o| o.rsp)),
                    ("R8", "", r.r8, old.map(|o| o.r8)),
                    ("R9", "", r.r9, old.map(|o| o.r9)),
                    ("R10", "", r.r10, old.map(|o| o.r10)),
                    ("R11", "", r.r11, old.map(|o| o.r11)),
                    ("R12", "", r.r12, old.map(|o| o.r12)),
                    ("R13", "", r.r13, old.map(|o| o.r13)),
                    ("R14", "", r.r14, old.map(|o| o.r14)),
                    ("R15", "", r.r15, old.map(|o| o.r15)),
                ];
                for (long, short, value, before) in gprs.iter().take(if wide { 16 } else { 8 }) {
                    let changed = before.is_some_and(|before| before != *value);
                    if wide {
                        register_row(ui, long, format!("{value:016X}"), changed);
                    } else {
                        register_row(ui, short, format!("{:08X}", value & 0xFFFF_FFFF), changed);
                    }
                }
                let rip_changed = old.is_some_and(|o| o.rip != r.rip);
                if wide {
                    register_row(ui, "RIP", format!("{:016X}", r.rip), rip_changed);
                } else {
                    register_row(ui, "EIP", format!("{:08X}", r.rip & 0xFFFF_FFFF), rip_changed);
                }
                let flags_changed = old.is_some_and(|o| o.eflags != r.eflags);
                register_row(ui, "EFLAGS", format!("{:08X}", r.eflags), flags_changed);
            });
        // On a line of its own, so the flags do not widen the value column.
        ui.label(flags_job(r.eflags, old.map(|o| o.eflags)));
        section_caption(ui, "System");
        egui::Grid::new("debugger_system_registers")
            .num_columns(2)
            .spacing(REGISTER_GRID_SPACING)
            .show(ui, |ui| {
                let bases = &view.segment_bases;
                let old_bases = before.map(|before| &before.segment_bases);
                for (name, selector, base, before) in [
                    ("CS", r.cs, bases.cs, old.zip(old_bases).map(|(o, b)| (o.cs, b.cs))),
                    ("SS", r.ss, bases.ss, old.zip(old_bases).map(|(o, b)| (o.ss, b.ss))),
                    ("DS", r.ds, bases.ds, old.zip(old_bases).map(|(o, b)| (o.ds, b.ds))),
                    ("ES", r.es, bases.es, old.zip(old_bases).map(|(o, b)| (o.es, b.es))),
                    ("FS", r.fs, bases.fs, old.zip(old_bases).map(|(o, b)| (o.fs, b.fs))),
                    ("GS", r.gs, bases.gs, old.zip(old_bases).map(|(o, b)| (o.gs, b.gs))),
                ] {
                    let changed = before.is_some_and(|before| before != (selector, base));
                    register_row(ui, name, format!("{selector:04X}  base {base:016X}"), changed);
                }
                let kgs_changed = before.is_some_and(|b| b.kernel_gs_base != view.kernel_gs_base);
                register_row(
                    ui,
                    "KGSBASE",
                    format!("{:016X}", view.kernel_gs_base),
                    kgs_changed,
                )
                .on_hover_text("IA32_KERNEL_GS_BASE: the GS base SWAPGS exchanges with GS's.");
                for (name, value, before) in [
                    ("CR0", r.cr0, old.map(|o| o.cr0)),
                    ("CR2", r.cr2, old.map(|o| o.cr2)),
                    ("CR3", r.cr3, old.map(|o| o.cr3)),
                    ("CR4", r.cr4, old.map(|o| o.cr4)),
                    ("EFER", view.efer, before.map(|b| b.efer)),
                ] {
                    let changed = before.is_some_and(|before| before != value);
                    register_row(ui, name, format!("{value:016X}"), changed);
                }
            });
        ui.add_space(SPACE_ITEM / 2.0);
        ui.collapsing("x87", |ui| {
            egui::Grid::new("debugger_x87").num_columns(2).spacing(REGISTER_GRID_SPACING).show(ui, |ui| {
                for (index, register) in r.fpu_regs.iter().enumerate() {
                    let changed = old.is_some_and(|o| o.fpu_regs[index] != *register);
                    register_row(ui, &format!("ST{index}"), le_hex(register), changed);
                }
                let sw_changed = old.is_some_and(|o| o.fpu_sw != r.fpu_sw);
                register_row(ui, "FSW", format!("{:04X}", r.fpu_sw), sw_changed);
                let cw_changed = old.is_some_and(|o| o.fpu_cw != r.fpu_cw);
                register_row(ui, "FCW", format!("{:04X}", r.fpu_cw), cw_changed);
            });
        });
        ui.collapsing("SSE", |ui| {
            egui::Grid::new("debugger_sse").num_columns(2).spacing(REGISTER_GRID_SPACING).show(ui, |ui| {
                let shown = if wide { 16 } else { 8 };
                for (index, register) in r.xmm.iter().enumerate().take(shown) {
                    let changed = old.is_some_and(|o| o.xmm[index] != *register);
                    register_row(ui, &format!("XMM{index}"), le_hex(register), changed);
                }
                let mxcsr_changed = old.is_some_and(|o| o.mxcsr != r.mxcsr);
                register_row(ui, "MXCSR", format!("{:08X}", r.mxcsr), mxcsr_changed);
            });
        });
    }
}

/// Why the dimmed registers of a viewed thread may not be its own.
const ENTRY_DEPENDENT: &str = "Saved only as the way it entered the kernel requires (system \
     call, interrupt or exception); it may not be this thread's value.";
/// Why R12–R15 have no value in a viewed thread.
const NOT_IN_TRAP_FRAME: &str = "A trap frame does not hold this register.";

/// A viewed thread's registers as its `KTRAP_FRAME` holds them: RIP, RSP,
/// RBP, RFLAGS and the segments, which every kernel entry saves, plainly; the
/// ones that depend on how it entered dimmed; R12–R15, which no trap frame
/// holds, as dashes. The live processor's system, x87 and SSE state belongs
/// to whatever runs, so it is not shown here.
fn saved_register_rows(ui: &mut egui::Ui, saved: &SavedRegisters) {
    ui.spacing_mut().item_spacing.y = 2.0;
    section_caption(ui, "Saved when it entered the kernel");
    egui::Grid::new("debugger_saved_registers")
        .num_columns(2)
        .spacing(REGISTER_GRID_SPACING)
        .show(ui, |ui| {
            for (name, value) in [("RIP", saved.rip), ("RSP", saved.rsp), ("RBP", saved.rbp)] {
                register_row(ui, name, format!("{value:016X}"), false);
            }
            register_row(ui, "RFLAGS", format!("{:08X}", saved.rflags), false);
            for (name, value) in [
                ("RAX", saved.rax),
                ("RBX", saved.rbx),
                ("RCX", saved.rcx),
                ("RDX", saved.rdx),
                ("RSI", saved.rsi),
                ("RDI", saved.rdi),
                ("R8", saved.r8),
                ("R9", saved.r9),
                ("R10", saved.r10),
                ("R11", saved.r11),
            ] {
                faint_register_row(ui, name, format!("{value:016X}")).on_hover_text(ENTRY_DEPENDENT);
            }
            for name in ["R12", "R13", "R14", "R15"] {
                faint_register_row(ui, name, "—".repeat(16)).on_hover_text(NOT_IN_TRAP_FRAME);
            }
            register_row(ui, "CS", format!("{:04X}", saved.cs), false);
            register_row(ui, "SS", format!("{:04X}", saved.ss), false);
        });
    // EFlags is a 32-bit field of the frame.
    ui.label(flags_job(saved.rflags as u32, None));
}

/// A register shown dimmed: present, but not to be trusted as much.
fn faint_register_row(ui: &mut egui::Ui, name: &str, value: String) -> egui::Response {
    ui.label(mono(name).color(TEXT_MUTED));
    let response = ui.label(mono(value).color(TEXT_MUTED.gamma_multiply(0.75)));
    ui.end_row();
    response
}

/// One register: its name, and its value — red when the last step or pause
/// changed it.
fn register_row(ui: &mut egui::Ui, name: &str, value: String, changed: bool) -> egui::Response {
    ui.label(mono(name).color(TEXT_MUTED));
    let color = if changed { CHANGED } else { TEXT_PRIMARY };
    let response = ui.label(mono(value).color(color));
    ui.end_row();
    response
}

/// The gap between a register's name and its value, and between rows.
const REGISTER_GRID_SPACING: egui::Vec2 = egui::vec2(16.0, 2.0);

/// A small heading over a group of registers.
fn section_caption(ui: &mut egui::Ui, text: &str) {
    ui.add_space(SPACE_ITEM / 2.0);
    ui.label(
        RichText::new(text)
            .size(crate::shell::theme::TEXT_CAPTION)
            .color(TEXT_MUTED),
    );
}

/// The nine status and control flags, as x64dbg lists them: a set flag
/// bright, a clear one dim, one the last step flipped red.
fn flags_job(eflags: u32, before: Option<u32>) -> LayoutJob {
    const FLAGS: [(&str, u32); 9] = [
        ("CF", 0),
        ("PF", 2),
        ("AF", 4),
        ("ZF", 6),
        ("SF", 7),
        ("TF", 8),
        ("IF", 9),
        ("DF", 10),
        ("OF", 11),
    ];
    let font = mono_font();
    let mut job = LayoutJob::default();
    for (index, (name, bit)) in FLAGS.iter().enumerate() {
        let set = (eflags >> bit) & 1 == 1;
        let flipped = before.is_some_and(|before| (before >> bit) & 1 != (eflags >> bit) & 1);
        let color = if flipped {
            CHANGED
        } else if set {
            ACCENT_CYAN
        } else {
            TEXT_MUTED.gamma_multiply(0.6)
        };
        let separator = if index == 0 { "" } else { " " };
        job.append(
            &format!("{separator}{name}"),
            0.0,
            TextFormat::simple(font.clone(), color),
        );
    }
    job
}

/// A little-endian register, most significant byte first.
fn le_hex(bytes: &[u8]) -> String {
    bytes.iter().rev().map(|byte| format!("{byte:02X}")).collect()
}

/// The memory pane's controls: the address space, the address to show, the
/// RIP and RSP shortcuts and paging, then the search row.
fn memory_bar(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    {
        ui.horizontal_wrapped(|ui| {
            let mut spaces = vec![
                Segment { value: AddressSpace::Linear, label: "Linear" },
                Segment { value: AddressSpace::Physical, label: "Physical" },
            ];
            // A Process space appears once a process other than the running
            // one is being browsed, so it can be selected and left.
            let process_space = match pane.space {
                AddressSpace::Process { cr3 } => Some(cr3),
                AddressSpace::Linear | AddressSpace::Physical => snapshot.memory_space,
            };
            if let Some(cr3) = process_space {
                spaces.push(Segment { value: AddressSpace::Process { cr3 }, label: "Process" });
            }
            segmented(ui, &mut pane.space, &spaces);
            if let Some(cr3) = process_space {
                chip(ui, &format!("CR3 {cr3:X}"), ACCENT_AMBER)
                    .on_hover_text("Browsing another process's address space.");
            }
            let field = ui.add(
                egui::TextEdit::singleline(&mut pane.address_text)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("Address")
                    .desired_width(150.0),
            );
            let enabled = ui.is_enabled();
            field.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, MEMORY_ADDRESS)
            });
            let entered =
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("Go").clicked() || entered {
                let names = Namer::new(&snapshot.symbols, marks.symbols);
                match go_target(&pane.address_text, names) {
                    Ok(value) => {
                        pane.address_error = None;
                        // A name is a linear address in the panes' own space.
                        let space = if pane.address_text.contains('!') { vantage_space(snapshot) } else { pane.space };
                        *action = Some(DebuggerAction::ShowMemory(MemoryTarget::At(
                            GuestAddress { space, value },
                        )));
                    }
                    Err(error) => pane.address_error = Some(error),
                }
            }
            if ui.button("RIP").clicked() {
                *action = Some(DebuggerAction::ShowMemory(MemoryTarget::Rip));
            }
            if ui.button("RSP").clicked() {
                *action = Some(DebuggerAction::ShowMemory(MemoryTarget::Rsp));
            }
            let start = snapshot.memory.start;
            if ui.button("Page up").clicked() {
                *action = Some(DebuggerAction::ShowMemory(MemoryTarget::At(GuestAddress {
                    space: start.space,
                    value: start.value.wrapping_sub(MEMORY_WINDOW_BYTES),
                })));
            }
            if ui.button("Page down").clicked() {
                *action = Some(DebuggerAction::ShowMemory(MemoryTarget::At(GuestAddress {
                    space: start.space,
                    value: start.value.wrapping_add(MEMORY_WINDOW_BYTES),
                })));
            }
            // The module the window sits in — only in the space the symbol
            // index's user modules belong to (a viewed thread's process, else
            // the shown processor's linear space).
            if start.space == vantage_space(snapshot) {
                let names = Namer::new(&snapshot.symbols, marks.symbols);
                if let Some(named) = names.name(start.value) {
                    chip(ui, &named.label(), name_color(&named, TEXT_MUTED))
                        .on_hover_text("Where this address is.");
                }
            }
        });
        if let Some(error) = pane.address_error {
            ui.label(RichText::new(error.message()).color(ACCENT_RED));
        }
        search_bar(ui, snapshot, marks, pane, action);
    }
}

/// The search row: what to find, how to read it, and what the last search
/// came to. A search starts just past the last hit while that hit is in
/// the window, and at the window's start otherwise.
fn search_bar(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    ui.horizontal_wrapped(|ui| {
        segmented(
            ui,
            &mut pane.search_encoding,
            &[
                Segment { value: SearchEncoding::Hex, label: "Hex" },
                Segment { value: SearchEncoding::Text, label: "Text" },
                Segment { value: SearchEncoding::Utf16, label: "UTF-16" },
            ],
        );
        let label = ui.label(RichText::new(FIND).color(TEXT_MUTED));
        let field = ui
            .add(
                egui::TextEdit::singleline(&mut pane.search_text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(220.0),
            )
            .labelled_by(label.id);
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let next = ui.add_enabled(!marks.searching, egui::Button::new("Find next"));
        if next.clicked() || (entered && !marks.searching) {
            match pattern_of(pane.search_encoding, &pane.search_text) {
                Ok(pattern) => {
                    pane.search_error = None;
                    *action = Some(DebuggerAction::Find(FindRequest {
                        from: search_start(snapshot, marks.find),
                        pattern,
                    }));
                }
                Err(error) => pane.search_error = Some(error),
            }
        }
    });
    if let Some(error) = pane.search_error {
        ui.label(RichText::new(error.message()).color(ACCENT_RED));
    } else if marks.searching {
        ui.label(RichText::new("Searching…").color(TEXT_MUTED));
    } else if let Some(outcome) = marks.find {
        let (text, color) = match outcome {
            FindOutcome::Found(found) => (format!("Found at {:X}", found.at.value), ACCENT_AMBER),
            FindOutcome::NotFound { searched_to } => (
                format!("Not found before {:X}", searched_to.value),
                TEXT_MUTED,
            ),
            FindOutcome::Abandoned => ("Search stopped: the VM is stopping".to_owned(), TEXT_MUTED),
        };
        ui.label(RichText::new(text).color(color));
    }
}

/// Where the next search starts: just past the last hit while it is in the
/// window shown, so Find next walks from hit to hit; the window's start once
/// the window has moved elsewhere.
fn search_start(snapshot: &DebugSnapshot, find: Option<FindOutcome>) -> GuestAddress {
    let window = snapshot.memory.start;
    let end = window.value.wrapping_add(MEMORY_WINDOW_BYTES);
    match find {
        Some(FindOutcome::Found(found))
            if found.at.space == window.space && (window.value..end).contains(&found.at.value) =>
        {
            GuestAddress {
                space: window.space,
                value: found.at.value.wrapping_add(1),
            }
        }
        Some(FindOutcome::Found(_) | FindOutcome::NotFound { .. } | FindOutcome::Abandoned)
        | None => window,
    }
}

/// How many bytes a memory row shows per line in a pane `room_chars` wide: all
/// `ROW_BYTES` where it has the room, otherwise half, so a phone's dump fits
/// without a sideways swipe. A row is an exact number of lines either way.
/// A line is the 16-char address, two gaps, `3n-1` hex characters and `n`
/// ASCII — `19 + 4n` characters.
fn memory_bytes_per_line(room_chars: f32) -> usize {
    if room_chars >= 19.0 + 4.0 * ROW_BYTES as f32 {
        ROW_BYTES
    } else {
        ROW_BYTES / 2
    }
}

/// The memory window: address, up to sixteen bytes and their ASCII per line,
/// folded to eight where the pane is too narrow for sixteen. A byte the last
/// step changed is red; the bytes of the last search hit amber.
fn memory_rows(ui: &mut egui::Ui, snapshot: &DebugSnapshot, marks: Marks<'_>) {
    // The window as it stood before, compared only when it shows the same
    // place: a window that moved has nothing to compare against.
    let before = marks
        .previous
        .filter(|previous| previous.memory.start == snapshot.memory.start)
        .map(|previous| &previous.memory.rows);
    let hit = match marks.find {
        Some(FindOutcome::Found(found)) if found.at.space == snapshot.memory.start.space => {
            Some(found)
        }
        Some(FindOutcome::Found(_) | FindOutcome::NotFound { .. } | FindOutcome::Abandoned)
        | None => None,
    };
    let font = mono_font();
    let room = (ui.available_width() - 2.0 * ROW_PAD) / char_width(ui);
    let per_line = memory_bytes_per_line(room);
    let hex_chars = 3 * per_line - 1;
    for (index, line) in snapshot.memory.rows.iter().enumerate() {
        let tone = Tone::striped(index);
        // A 16-byte row is drawn as one line, or two when it is folded.
        for chunk in (0..ROW_BYTES).step_by(per_line) {
            let base = line.address.wrapping_add(chunk as u64);
            let address = Cell::left(format!("{base:016X}"), TEXT_MUTED, 16);
            let Some(bytes) = line.bytes else {
                let unreadable = vec!["??"; per_line].join(" ");
                let cells =
                    [address, Cell::left(unreadable, TEXT_MUTED.gamma_multiply(0.6), hex_chars)];
                list_row(ui, 0.0, &cells, tone, RowSense::Look);
                continue;
            };
            let old = before.and_then(|rows| rows.get(index)).and_then(|old| old.bytes);
            let mut hex = LayoutJob::default();
            for offset in 0..per_line {
                let column = chunk + offset;
                let byte = bytes[column];
                let at = line.address.wrapping_add(column as u64);
                let color = if old.is_some_and(|old| old[column] != byte) {
                    CHANGED
                } else if hit.is_some_and(|hit| in_hit(hit, at)) {
                    ACCENT_AMBER
                } else if byte == 0 {
                    // Zero bytes recede, so the data in a page stands out.
                    TEXT_MUTED.gamma_multiply(0.7)
                } else {
                    TEXT_PRIMARY
                };
                let separator = if offset == 0 { "" } else { " " };
                hex.append(
                    &format!("{separator}{byte:02X}"),
                    0.0,
                    TextFormat::simple(font.clone(), color),
                );
            }
            let ascii: String = bytes[chunk..chunk + per_line]
                .iter()
                .map(|&byte| {
                    if (0x20..0x7F).contains(&byte) {
                        byte as char
                    } else {
                        '.'
                    }
                })
                .collect();
            let cells = [
                address,
                Cell::job(hex, hex_chars),
                Cell::left(ascii, ACCENT_BLUE, per_line),
            ];
            list_row(ui, 0.0, &cells, tone, RowSense::Look);
        }
    }
}

fn in_hit(hit: FoundAt, address: u64) -> bool {
    hit.at.value <= address && address < hit.at.value.wrapping_add(hit.len)
}

/// The stack from RSP up: the slot RSP points at in cyan, a slot the last
/// step changed in red.
fn stack_rows(ui: &mut egui::Ui, snapshot: &DebugSnapshot, marks: Marks<'_>) {
    let names = Namer::new(&snapshot.symbols, marks.symbols);
    let size = pane_code_size(snapshot);
    let digits = hex_digits(size);
    // A slot is as wide as a pointer at the code size: 2, 4 or 8 bytes.
    let value_digits = 2 * crate::debugger::inspect::slot_bytes(size);
    let before = marks.previous.map(|previous| &previous.stack);
    for (index, slot) in snapshot.stack.iter().enumerate() {
        let value = slot.value.map_or_else(
            || "?".repeat(value_digits),
            |value| format!("{value:0value_digits$X}"),
        );
        // Compared slot by address, so a stack that moved compares only the
        // slots both views hold.
        let changed = before
            .and_then(|stack| stack.iter().find(|old| old.address == slot.address))
            .is_some_and(|old| old.value != slot.value);
        let top = index == 0;
        let value_color = if changed {
            CHANGED
        } else if top {
            ACCENT_CYAN
        } else {
            TEXT_PRIMARY
        };
        let address_color = if top { ACCENT_CYAN } else { TEXT_MUTED };
        let tone = if top { Tone::Current } else { Tone::Plain };
        // A slot that points into a module is a code pointer — most often a
        // return address, so the column reads as the call chain.
        let named = slot.value.and_then(|value| names.name(value));
        let name_cell = match &named {
            Some(named) => Cell::fill(named.label(), name_color(named, TEXT_MUTED)),
            None => Cell::fill(String::new(), TEXT_MUTED),
        };
        let cells = [
            Cell::left(format!("{:0digits$X}", slot.address), address_color, digits),
            Cell::left(value, value_color, value_digits),
            name_cell,
        ];
        let response = list_row(ui, MARKER_LEAD, &cells, tone, RowSense::Look);
        if top {
            lead_arrow(ui, response.rect, ACCENT_CYAN);
        }
    }
}

/// Said while the run thread has not yet listed what a tab asks for.
const READING_LIST: &str = "Reading the guest's lists…";
/// Why the lists are empty: no Windows x64 kernel, or a kernel without the
/// structure a tab lists, and the step that said so.
pub(crate) fn no_windows_kernel(miss: KernelMiss) -> String {
    if miss.kernel_found() {
        format!(
            "The 64-bit Windows kernel was found, but {}.",
            miss.message()
        )
    } else {
        format!(
            "No 64-bit Windows kernel was found in this guest ({}): the lists read \
             Windows x64's own process and driver lists.",
            miss.message()
        )
    }
}

/// [`no_windows_kernel`]'s text for a pause that read the guest of the VMCS
/// at `vmcs` (the processor was paused in the hypervisor): no kernel found
/// there says what else that guest may be.
pub(crate) fn no_windows_kernel_in(miss: KernelMiss, vmcs: Option<u64>) -> String {
    match vmcs {
        Some(vmcs) if !miss.kernel_found() => format!(
            "{} This pause is in the hypervisor, and the guest of its VMCS at {vmcs:#x} was read: \
             under VBS that guest may be the secure kernel (VTL 1) rather than Windows.",
            no_windows_kernel(miss)
        ),
        Some(_) | None => no_windows_kernel(miss),
    }
}

/// What a listing says when it was read as the guest of the VMCS at `vmcs`.
pub(crate) fn vmx_guest_label(vmcs: u64) -> String {
    format!("read as the guest of the VMCS at {vmcs:#x}: this pause is in the hypervisor (VMX root)")
}

/// The heading over [`no_windows_kernel`]'s text.
fn no_windows_kernel_title(miss: KernelMiss) -> &'static str {
    if miss.kernel_found() {
        "Not found in the kernel"
    } else {
        "No Windows kernel"
    }
}
const ANOTHER_ADDRESS_SPACE: &str = "This module lies in another process's address space; \
     pause while that process runs to see its memory.";

/// The Processes and Kernel modules tabs: the guest kernel's own lists.
fn system_page(
    ui: &mut egui::Ui,
    fit: Fit,
    snapshot: &DebugSnapshot,
    states: ModuleStates<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    match (&snapshot.system, pane.tab) {
        (SystemListing::NoWindowsKernel(miss), _) => {
            empty_state(ui, no_windows_kernel_title(*miss), &no_windows_kernel_in(*miss, snapshot.vmcs));
        }
        (SystemListing::Processes(listing), DebuggerTab::Processes) => {
            layouts_line(ui, snapshot);
            let current_user_module = current_module_base(snapshot, ModuleSpace::User);
            processes_page(ui, fit, listing, current_user_module, states, pane, action);
        }
        (SystemListing::KernelModules(listing), DebuggerTab::KernelModules) => {
            layouts_line(ui, snapshot);
            let current = current_module_base(snapshot, ModuleSpace::Kernel);
            kernel_page(ui, fit, listing, current, states, pane, action);
        }
        (SystemListing::Drivers(listing), DebuggerTab::Drivers) => {
            layouts_line(ui, snapshot);
            drivers_page(ui, fit, listing, pane, action);
        }
        (SystemListing::Ssdt(listing), DebuggerTab::Ssdt) => {
            layouts_line(ui, snapshot);
            ssdt_page(ui, fit, listing, Namer::new(states.index, states.view), pane, action);
        }
        // A list for another tab, or none yet: the run thread is reading.
        (
            SystemListing::NotAsked
            | SystemListing::Processes(_)
            | SystemListing::KernelModules(_)
            | SystemListing::Drivers(_)
            | SystemListing::Ssdt(_),
            _,
        ) => {
            empty_state(ui, "Reading", READING_LIST);
        }
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    use crate::symbols::kernel::tests::{kernel_symbols, KERNEL_BASE};
    use crate::symbols::names::tests::hub_with_kernel_code;
    use rusty_box_symbols::identity::{Guid, TablesIdentity};

    /// Review focus 3: a name from a file of another build, loaded anyway,
    /// is drawn amber wherever a pane draws it.
    #[test]
    fn a_forced_module_is_named_in_amber() {
        let view = hub_with_kernel_code(crate::symbols::hub::ModuleSymbols::Forced {
            found: TablesIdentity::Pdb { guid: Guid::from_le_bytes([0x77; 16]), age: 3 },
            path: std::path::PathBuf::from("D:\\other\\ntkrnlmp.pdb"),
        })
        .view();
        let symbols = kernel_symbols();
        let named = Namer::new(&symbols, &view).name(KERNEL_BASE + 0x1040).expect("in the kernel");
        assert_eq!(named.label(), "ntoskrnl!KiSystemCall64");
        assert_eq!(name_color(&named, ACCENT_CYAN), ACCENT_AMBER);
        let matched = Namer::new(&symbols, &hub_with_kernel_code(crate::symbols::names::tests::loaded()).view())
            .name(KERNEL_BASE + 0x1040)
            .expect("in the kernel");
        assert_eq!(name_color(&matched, ACCENT_CYAN), ACCENT_CYAN);
    }

    /// A service is named by its stub, else by its routine's own symbol; a
    /// routine whose symbol is not the stub's service reads rerouted, in
    /// amber; a first jump out of the owner reads an inline hook, in red.
    #[test]
    fn a_service_is_named_by_its_stub_and_flagged_when_its_symbol_differs() {
        use crate::debugger::windows::InlineJump;
        let view = hub_with_kernel_code(crate::symbols::names::tests::loaded()).view();
        let symbols = kernel_symbols();
        let names = Namer::new(&symbols, &view);
        let service = |stub: Option<&str>| SsdtService {
            index: 7,
            routine: KERNEL_BASE + 0x1040,
            site: PointerSite::OwnImage,
            module_name: Some("ntoskrnl.exe".to_owned()),
            stub_name: stub.map(str::to_owned),
            inline: None,
        };
        let plain = service_view(&service(Some("KiSystemCall64")), names);
        assert_eq!(plain.name, "KiSystemCall64");
        assert_eq!(plain.target, "ntoskrnl.exe");
        assert_eq!(plain.color, TEXT_PRIMARY);
        let unnamed = service_view(&service(None), names);
        assert_eq!(unnamed.name, "KiSystemCall64");
        assert_eq!(unnamed.color, TEXT_PRIMARY);
        let rerouted = service_view(&service(Some("NtClose")), names);
        assert_eq!(rerouted.name, "NtClose");
        assert_eq!(rerouted.target, "ntoskrnl.exe  · rerouted to KiSystemCall64");
        assert_eq!(rerouted.color, ACCENT_AMBER);
        let mut hooked = service(Some("NtClose"));
        hooked.inline = Some(InlineJump { target: 0xFFFF_F800_0300_0200, module_name: Some("disk.sys".to_owned()) });
        let inline = service_view(&hooked, names);
        assert_eq!(inline.color, ACCENT_RED);
        assert!(inline.target.contains("inline hook, jumps to disk.sys"), "{}", inline.target);
    }

    /// Go takes `module!name` as well as an address, and says why a name
    /// does not resolve.
    #[test]
    fn go_takes_a_name_or_an_address() {
        let view = hub_with_kernel_code(crate::symbols::names::tests::loaded()).view();
        let symbols = kernel_symbols();
        let names = Namer::new(&symbols, &view);
        assert_eq!(go_target(" nt!KeBugCheckEx ", names), Ok(KERNEL_BASE + 0x1100));
        assert_eq!(go_target("0x1234", names), Ok(0x1234));
        assert_eq!(go_target("nt!Nope", names), Err(AddressError::Name(NameMiss::UnknownName)));
        assert_eq!(go_target("zz", names), Err(AddressError::NotHex));
    }

    fn trap_line(line: u32) -> BreakpointTarget {
        BreakpointTarget::Source(crate::debugger::source_breakpoints::SourceTarget { file: "trap.asm".into(), line })
    }

    /// No module of the owner's runs in another process.
    static NOWHERE: crate::debugger::windows::ModulesElsewhere =
        crate::debugger::windows::ModulesElsewhere { names: Vec::new(), modules: Vec::new() };

    /// A pause's modules: `symbols`, and nothing in other processes.
    fn only(symbols: &crate::debugger::inspect::SymbolIndex) -> BreakpointModules<'_> {
        BreakpointModules { index: symbols, elsewhere: &NOWHERE }
    }

    /// F2 on the code a source breakpoint resolved to removes that source
    /// breakpoint, as its mark says; elsewhere it adds an address.
    #[test]
    fn toggling_at_a_source_breakpoints_code_removes_it() {
        let view = hub_with_kernel_code(crate::symbols::names::tests::loaded()).view();
        let symbols = kernel_symbols();
        let at_line = KERNEL_BASE + 0x1040;
        let targets = [trap_line(100)];
        assert_eq!(toggled(&targets, at_line, Some(only(&symbols)), &view), []);
        let elsewhere = KERNEL_BASE + 0x1100;
        assert_eq!(
            toggled(&targets, elsewhere, Some(only(&symbols)), &view),
            [trap_line(100), BreakpointTarget::Address(CodeBreakpoint::Linear(elsewhere))]
        );
        let address = [BreakpointTarget::Address(CodeBreakpoint::Linear(elsewhere))];
        assert_eq!(toggled(&address, elsewhere, Some(only(&symbols)), &view), []);
        assert_eq!(toggled(&targets, at_line, None, &view).len(), 2, "without a pause nothing resolves");
    }

    /// The code pane brings a place into view once: a new pause, step,
    /// frame or Go to. The owner's own scrolling stands until the place
    /// changes, across frames.
    #[test]
    fn the_code_pane_follows_each_new_place_once() {
        let mut picks = CodePicks::of(&DebuggerPane::default());
        let at_rip = CodePlace { view: CodeView::Source, address: KERNEL_BASE + 0x1040, line: Some(100) };
        assert!(picks.follow(at_rip), "a new pause");
        assert!(!picks.follow(at_rip), "the owner scrolls freely after");
        let stepped = CodePlace { address: KERNEL_BASE + 0x1050, line: Some(104), ..at_rip };
        assert!(picks.follow(stepped), "a step");
        let disassembly = CodePlace { view: CodeView::Disassembly, line: None, ..stepped };
        assert!(picks.follow(disassembly), "the other view brings it into view");
        let mut pane = DebuggerPane::default();
        picks.keep(&mut pane);
        assert!(!CodePicks::of(&pane).follow(disassembly), "kept from one frame to the next");
    }

    /// A source breakpoint's row names where it resolved, and says when the
    /// line that has the code is not the one asked.
    #[test]
    fn a_source_breakpoints_row_says_where_it_resolved() {
        let view = hub_with_kernel_code(crate::symbols::names::tests::loaded()).view();
        let symbols = kernel_symbols();
        let targets = [trap_line(100), trap_line(101), trap_line(200)];
        let resolved = Resolutions::of(&targets, Some(&only(&symbols)), &view);
        let list = BreakpointList {
            targets: &targets,
            applied: &[],
            names: Namer::new(&symbols, &view),
            resolutions: &resolved,
            available: true,
        };
        assert_eq!(target_place(&trap_line(100), &list), "ntoskrnl!KiSystemCall64");
        assert_eq!(target_place(&trap_line(101), &list), "ntoskrnl!KiSystemCall64+0x10 · line 104 (101 has no code)");
        assert_eq!(target_place(&trap_line(200), &list), PENDING_NO_LINE);
        let unresolved = Resolutions::of(&targets, None, &view);
        let before_a_pause = BreakpointList { resolutions: &unresolved, ..list };
        assert_eq!(target_place(&trap_line(100), &before_a_pause), PENDING_NO_PAUSE);
    }
}

/// The lines over a kernel listing that say whose layouts read it — a kernel
/// PDB that disagrees with memory reads amber — and whose page tables, when
/// not the processor's own.
fn layouts_line(ui: &mut egui::Ui, snapshot: &DebugSnapshot) {
    if let Some(layouts) = snapshot.layouts {
        let color = match layouts {
            KernelLayouts::Pdb | KernelLayouts::Discovered => TEXT_MUTED,
            KernelLayouts::PdbDisagrees | KernelLayouts::ForcedPdb => ACCENT_AMBER,
        };
        ui.label(RichText::new(layouts.label()).small().color(color));
    }
    if let Some(through) = snapshot.through.label() {
        ui.label(RichText::new(through).small().color(TEXT_MUTED));
    }
    if let Some(vmcs) = snapshot.vmcs {
        ui.label(RichText::new(vmx_guest_label(vmcs)).small().color(TEXT_MUTED));
    }
}

/// The accessible name of a list's filter field.
pub(crate) const FILTER: &str = "Filter by name";

/// The accessible name of the exports' filter field.
pub(crate) const EXPORT_FILTER: &str = "Filter exports";

/// The filter over a list: what a name (or path) must contain.
fn filter_field(ui: &mut egui::Ui, filter: &mut String) {
    labelled_filter(ui, filter, FILTER);
}

/// A filter field named `label`, which is also its hint.
fn labelled_filter(ui: &mut egui::Ui, filter: &mut String, label: &str) {
    let response = ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text(label)
            .desired_width(220.0),
    );
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, label));
}

/// The accessible name of the imports' filter field.
pub(crate) const IMPORT_FILTER: &str = "Filter imports";

/// Which card of a list tab the keys move in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListFocus {
    /// The processes, or the kernel's modules.
    List,
    /// What is listed beside them for the picked one.
    Detail,
    /// The picked process module's sections, exports or imports, under that
    /// process's modules.
    Module,
}

/// How many cards a list tab shows the keys can move in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cards {
    /// A list and its detail.
    Two,
    /// A process list, its modules, and the picked module's detail.
    Three,
}

impl ListFocus {
    /// The card Tab hands the keys to.
    const fn next(self, cards: Cards) -> Self {
        match (self, cards) {
            (Self::List, _) => Self::Detail,
            (Self::Detail, Cards::Three) => Self::Module,
            (Self::Detail, Cards::Two) | (Self::Module, _) => Self::List,
        }
    }
}

/// The view a module's card shows after `detail`, as its switch orders them.
const fn next_module_detail(detail: ModuleDetail) -> ModuleDetail {
    match detail {
        ModuleDetail::Sections => ModuleDetail::Exports,
        ModuleDetail::Exports => ModuleDetail::Imports,
        ModuleDetail::Imports => ModuleDetail::Symbols,
        ModuleDetail::Symbols => ModuleDetail::Sections,
    }
}

/// Where a click or Enter on an address in a module's detail shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opener {
    /// The memory pane, in the shown processor's address space: the kernel's
    /// modules, and the running process's.
    Live,
    /// The memory pane, in the address space at `cr3`: another process's.
    Process { cr3: u64 },
    /// Nowhere: another process whose address space is not known.
    Nowhere,
}

impl Opener {
    /// A process's: live when it runs, else through its address space.
    const fn of_process(navigable: bool, process_cr3: Option<u64>) -> Self {
        match (navigable, process_cr3) {
            (true, _) => Self::Live,
            (false, Some(cr3)) => Self::Process { cr3 },
            (false, None) => Self::Nowhere,
        }
    }

    /// What opening `address` does.
    fn open(self, address: u64) -> RowOpen {
        match self {
            Self::Live => RowOpen::Act(DebuggerAction::OpenModule(address)),
            Self::Process { cr3 } => RowOpen::Act(DebuggerAction::ViewInProcess { cr3, address }),
            Self::Nowhere => RowOpen::Nothing,
        }
    }

    /// The hover that says what a click does.
    const fn hint(self) -> &'static str {
        match self {
            Self::Live => "Click to show it in memory.",
            Self::Process { .. } => "Click to show it in this process's memory.",
            Self::Nowhere => ANOTHER_ADDRESS_SPACE,
        }
    }
}

/// The keys a list tab took this frame.
#[derive(Clone, Copy)]
struct ListKeys {
    /// Rows to move: down positive, up negative.
    step: isize,
    /// Enter: what a double-click on the row stood on does.
    open: bool,
    /// Right presses less Left presses: a row with rows under it (a module in
    /// the memory map, a module imported from) shows them for each Right and
    /// hides them for each Left.
    sideways: isize,
    /// Tab presses: each hands the keys to the next card.
    switches: usize,
    /// Shift+Tab presses: each shows the next view of the card the keys are
    /// in (the detail's, from the list).
    next_views: usize,
}

impl ListKeys {
    /// Whether a key moves, opens or folds a row: all that walking a card's
    /// rows answers. Most frames press none.
    const fn walks(self) -> bool {
        self.step != 0 || self.open || self.sideways != 0
    }
}

/// The list tabs' keyboard: a widget of no size that holds the focus, with a
/// filter that keeps egui from moving the focus on Tab and the arrows.
const LIST_KEYS: &str = "debugger_list_keys";

/// Takes the keyboard for a list tab and reads what was pressed. The keys
/// are left alone while a text field (a filter) has them or a window above
/// the page does; `None` then, and on the frame the focus is taken, before
/// its filter holds.
fn list_keys(ui: &mut egui::Ui) -> Option<ListKeys> {
    let id = egui::Id::new(LIST_KEYS);
    // Placed every frame: egui drops the focus of a widget a frame did not
    // place.
    ui.interact(
        egui::Rect::from_min_size(ui.cursor().min, egui::Vec2::ZERO),
        id,
        egui::Sense::focusable_noninteractive(),
    );
    let ctx = ui.ctx().clone();
    if !ctx.memory(|memory| memory.has_focus(id)) {
        let free = !ctx.text_edit_focused()
            && ctx.memory(|memory| memory.allows_interaction(ui.layer_id()));
        if free {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
        return None;
    }
    ctx.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: false,
            },
        );
    });
    Some(ctx.input_mut(|input| {
        // Shift+Tab first: a plain Tab's match ignores Shift.
        let next_views = input.count_and_consume_key(egui::Modifiers::SHIFT, egui::Key::Tab);
        let switches = input.count_and_consume_key(egui::Modifiers::NONE, egui::Key::Tab);
        let down = input.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
        let up = input.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
        let open = input.count_and_consume_key(egui::Modifiers::NONE, egui::Key::Enter) > 0;
        let right = input.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
        let left = input.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
        ListKeys {
            step: down as isize - up as isize,
            open,
            sideways: right as isize - left as isize,
            switches,
            next_views,
        }
    }))
}

/// The row the keys stand on in a listing, and whether a key moved them
/// there this frame: a row moved to is drawn even out of sight, and scrolled
/// into view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyRow {
    Resting(usize),
    Moved(usize),
}

impl KeyRow {
    fn row(self) -> usize {
        match self {
            Self::Resting(at) | Self::Moved(at) => at,
        }
    }

    fn at(self, row: usize) -> bool {
        self.row() == row
    }

    fn brings_in(self, row: usize) -> bool {
        self == Self::Moved(row)
    }
}

/// Whether row `row` of a listing is left as room only: out of sight, and
/// not the row a key just moved to.
fn culled(ui: &egui::Ui, key: Option<KeyRow>, row: usize) -> bool {
    !key.is_some_and(|key| key.brings_in(row)) && !row_visible(ui)
}

/// A detail row's shade: selected where the keys stand, striped otherwise.
fn keyed_tone(key: Option<KeyRow>, row: usize) -> Tone {
    if key.is_some_and(|key| key.at(row)) {
        Tone::Selected
    } else {
        Tone::striped(row)
    }
}

/// Scrolls the drawn row `row` into view when a key just moved to it.
fn follow_key(response: &egui::Response, key: Option<KeyRow>, row: usize) {
    if key.is_some_and(|key| key.brings_in(row)) {
        response.scroll_to_me(None);
    }
}

/// Where the keys left a listing of `rows` rows that stood on `from`: the
/// row they stand on now, and the row Enter opened.
struct Walk {
    key: Option<KeyRow>,
    open: Option<usize>,
}

/// Moves from `from` by the keys' step, staying in the listing. Standing on
/// no row is standing just above the first.
fn walk(keys: ListKeys, from: Option<usize>, rows: usize) -> Walk {
    let at = from.filter(|&at| at < rows);
    let key = match (rows.checked_sub(1), keys.step) {
        (None, _) => None,
        (Some(_), 0) => at.map(KeyRow::Resting),
        (Some(last), step) => {
            let to = match at {
                Some(at) => at.saturating_add_signed(step),
                None => step.saturating_sub(1).max(0).unsigned_abs(),
            };
            Some(KeyRow::Moved(to.min(last)))
        }
    };
    let open = if keys.open { key.map(KeyRow::row) } else { None };
    Walk { key, open }
}

/// The rows the keys stand on in a list tab's cards.
#[derive(Clone, Copy, Default)]
struct KeyedRows {
    list: Option<KeyRow>,
    detail: Option<KeyRow>,
    module: Option<KeyRow>,
}

/// A row that has rows under it, shown or hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fold {
    /// A module in the memory map, by its base: its sections.
    MapModule(u64),
    /// A module imported from, by its index in the import table: the
    /// functions imported from it.
    ImportGroup(usize),
}

/// What is shown open: the memory map's modules open (closed by default) and
/// the import groups folded shut (open by default).
struct Folds<'a> {
    map_open: &'a mut BTreeSet<u64>,
    imports_shut: &'a mut BTreeSet<usize>,
}

impl Folds<'_> {
    fn is_open(&self, fold: Fold) -> bool {
        match fold {
            Fold::MapModule(base) => self.map_open.contains(&base),
            Fold::ImportGroup(index) => !self.imports_shut.contains(&index),
        }
    }

    fn set_open(&mut self, fold: Fold, open: bool) {
        match (fold, open) {
            (Fold::MapModule(base), true) => {
                self.map_open.insert(base);
            }
            (Fold::MapModule(base), false) => {
                self.map_open.remove(&base);
            }
            (Fold::ImportGroup(index), true) => {
                self.imports_shut.remove(&index);
            }
            (Fold::ImportGroup(index), false) => {
                self.imports_shut.insert(index);
            }
        }
    }
}

/// What Enter does on a row of a detail card: what a click on it does.
enum RowOpen {
    Act(DebuggerAction),
    /// A row with rows under it: shows or hides them.
    Fold(Fold),
    /// Nothing opens: a module of another address space, a forwarded export.
    Nothing,
}

/// Where a detail row sits in a tree (the memory map's, the imports'), for
/// Left and Right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Branch {
    /// A row with nothing under it and nothing over it.
    Leaf,
    /// A row with rows under it.
    Parent(Fold),
    /// A row shown under the parent row at this index.
    Under(usize),
}

/// A detail row as the keys see it: what Enter does on it, and where it sits.
struct DetailRow {
    open: RowOpen,
    branch: Branch,
}

impl DetailRow {
    const fn leaf(open: RowOpen) -> Self {
        Self {
            open,
            branch: Branch::Leaf,
        }
    }
}

/// Moves a card's cursor by the keys over the rows `rows` lists, then shows
/// or hides a parent row's children for Right and Left, then does what Enter
/// opened. A frame with none of these keys lists no rows.
fn walk_detail<F>(
    keys: ListKeys,
    rows: F,
    cursor: &mut Option<usize>,
    mut folds: Folds<'_>,
    action: &mut Option<DebuggerAction>,
) -> Option<KeyRow>
where
    F: FnOnce(&Folds<'_>) -> Vec<DetailRow>,
{
    if !keys.walks() {
        // A cursor past the rows of a listing that shrank stands on none.
        return cursor.map(KeyRow::Resting);
    }
    let rows = rows(&folds);
    let mut walked = walk(keys, *cursor, rows.len());
    if let Some(KeyRow::Moved(to)) = walked.key {
        *cursor = Some(to);
    }
    // Each press acts on the row the one before it left the keys on.
    for _ in 0..keys.sideways.unsigned_abs() {
        let Some(at) = walked.key.map(KeyRow::row) else {
            break;
        };
        match (rows.get(at).map(|row| row.branch), keys.sideways > 0) {
            (Some(Branch::Parent(fold)), open) => folds.set_open(fold, open),
            // Left on a child steps out to its parent's row.
            (Some(Branch::Under(parent)), false) => {
                *cursor = Some(parent);
                walked.key = Some(KeyRow::Moved(parent));
            }
            (Some(Branch::Under(_) | Branch::Leaf), true) | (Some(Branch::Leaf), false) | (None, _) => {}
        }
    }
    if let Some(opened) = walked.open {
        match rows.into_iter().nth(opened).map(|row| row.open) {
            Some(RowOpen::Act(open)) => *action = Some(open),
            Some(RowOpen::Fold(fold)) => {
                let open = folds.is_open(fold);
                folds.set_open(fold, !open);
            }
            Some(RowOpen::Nothing) | None => {}
        }
    }
    walked.key
}

/// `pane_card`, outlined in cyan while it is `keyed`: the card the keys move
/// in.
fn keyed_pane_card<H, B>(ui: &mut egui::Ui, room: Room, title: Title<'_>, keyed: bool, header: H, body: B)
where
    H: FnOnce(&mut egui::Ui),
    B: FnOnce(&mut egui::Ui),
{
    let card = ui.scope(|ui| pane_card(ui, room, title, header, body)).response.rect;
    if keyed {
        ui.painter().rect_stroke(
            card,
            12.0,
            egui::Stroke::new(1.5_f32, ACCENT_CYAN),
            egui::StrokeKind::Inside,
        );
    }
}

/// The Kernel modules tab: the kernel's modules, and beside them the picked
/// one's sections, exports or imports — laid out as the Processes tab is, the two
/// cards side by side with a handle between them where the page has room,
/// one above the other where it scrolls.
fn kernel_page(
    ui: &mut egui::Ui,
    fit: Fit,
    listing: &KernelListing,
    current: Option<u64>,
    states: ModuleStates<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let filter = pane.list_filter.to_lowercase();
    let shown: Vec<&WinModule> = listing
        .modules
        .iter()
        .filter(|module| module_matches(module, &filter))
        .collect();
    let picked_before = (pane.selected_kernel_module, pane.kernel_detail);
    let keys = list_keys(ui);
    let keyed_rows = keys.map_or_else(KeyedRows::default, |keys| {
        kernel_keys(keys, listing, &shown, pane, action)
    });
    let keyed = keys.map(|_| pane.list_focus);
    let picked_name = pane
        .selected_kernel_module
        .and_then(|base| listing.modules.iter().find(|module| module.base == base))
        .map(|module| module.name.clone().unwrap_or_else(|| format!("{:X}", module.base)));
    // Read before the header's field takes the filter for this frame.
    let detail_filter = pane.export_filter.to_lowercase();
    let module_card = ModuleCard {
        answer: module_answer(listing.picked.as_ref(), pane.selected_kernel_module, pane.kernel_detail),
        has_pick: pane.selected_kernel_module.is_some(),
        filter: &detail_filter,
        // The kernel half is mapped alike in every address space.
        opener: Opener::Live,
        named: &listing.modules,
    };
    let list_title = Title::counted("Kernel modules", listing.modules.len()).inlined();
    let detail_title = Title {
        text: picked_name.as_deref().unwrap_or("Module"),
        count: module_card.count(),
        inline: true,
    };

    let mut list_card = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            list_title,
            keyed == Some(ListFocus::List),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    title_label(ui, list_title.text, list_title.count);
                    filter_field(ui, &mut pane.list_filter);
                });
                header_row(ui, 0.0, &module_columns());
            },
            |ui| {
                module_rows(
                    ui,
                    &shown,
                    ModuleClick {
                        picked: &mut pane.selected_kernel_module,
                        opener: Opener::Live,
                    },
                    current,
                    states,
                    keyed_rows.list,
                    action,
                );
            },
        );
    };
    let mut detail_card = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            detail_title,
            keyed.is_some_and(|focus| focus != ListFocus::List),
            |ui| {
                module_detail_header(
                    ui,
                    detail_title,
                    &mut pane.kernel_detail,
                    &mut pane.export_filter,
                );
            },
            |ui| {
                module_detail_body(ui, module_card, states, keyed_rows.detail, &mut pane.folded_imports, action);
            },
        );
    };
    match fit {
        Fit::Grid => {
            let gap = SPACE_ITEM;
            let page = ui.available_size();
            let share = resized(pane.split.kernel, 0.0, page.x, gap, SPLIT_MIN);
            let left = ((page.x - gap) * share).floor();
            let mut moved: Option<f32> = None;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                list_card(ui, Room::Fixed(egui::vec2(left, page.y)), &mut *action);
                match splitter(ui, egui::vec2(gap, page.y), SplitAxis::Columns, RESIZE_MODULE_LIST) {
                    SplitterMove::Still => {}
                    SplitterMove::Dragged(delta) => {
                        moved = Some(resized(share, delta, page.x, gap, SPLIT_MIN));
                    }
                    SplitterMove::Reset => moved = Some(PROCESS_LIST_SHARE),
                }
                detail_card(
                    ui,
                    Room::Fixed(egui::vec2(page.x - gap - left, page.y)),
                    &mut *action,
                );
            });
            if let Some(share) = moved {
                pane.split.kernel = share;
            }
        }
        Fit::Scrolling(columns) => {
            let density = fit.density();
            let room = Room::Content(density);
            let gap = density.gap();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match columns {
                    Columns::Two => side_by_side(
                        ui,
                        gap,
                        PROCESS_LIST_SHARE,
                        action,
                        |ui, action| list_card(ui, room, action),
                        |ui, action| detail_card(ui, room, action),
                    ),
                    Columns::One => {
                        list_card(ui, room, &mut *action);
                        ui.add_space(gap);
                        detail_card(ui, room, &mut *action);
                    }
                });
        }
    }
    // A new pick or another view asks the run for it, and leaves the cursor
    // on no detail row and every import group open; a double-click that
    // opened a module in memory stands.
    if (pane.selected_kernel_module, pane.kernel_detail) != picked_before {
        pane.detail_cursor = None;
        pane.folded_imports.clear();
        if !matches!(action, Some(DebuggerAction::OpenModule(_))) {
            *action = Some(DebuggerAction::ShowSystem(DebuggerTab::KernelModules.system_view(pane)));
        }
    }
}

/// What a module's card draws from: the run's answer for the picked module,
/// and how the addresses it lists open and are named.
#[derive(Clone, Copy)]
struct ModuleCard<'a> {
    answer: Option<&'a PickedModule>,
    has_pick: bool,
    /// What a listed export or import must contain, lowercase.
    filter: &'a str,
    opener: Opener,
    /// The modules a bound import's address is named against.
    named: &'a [WinModule],
}

impl ModuleCard<'_> {
    /// How many entries the answer lists.
    fn count(&self) -> Option<usize> {
        match &self.answer?.shown {
            PickedDetail::Sections(SectionsRead::Mapped(map)) => Some(map.regions.len()),
            PickedDetail::Exports(ExportRead::Listed(exports)) => Some(exports.len()),
            PickedDetail::Imports(ImportRead::Listed(modules)) => {
                Some(modules.iter().map(|module| module.functions.len()).sum())
            }
            PickedDetail::Sections(SectionsRead::HeaderUnreadable)
            | PickedDetail::Exports(ExportRead::NoTable | ExportRead::NotPe | ExportRead::Unreadable)
            | PickedDetail::Imports(ImportRead::NoTable | ImportRead::NotPe | ImportRead::Unreadable)
            | PickedDetail::Symbols(_) => None,
        }
    }
}

/// The run's answer for the module picked at `selected`, once it is for that
/// module and the `detail` asked.
fn module_answer(
    picked: Option<&PickedModule>,
    selected: Option<u64>,
    detail: ModuleDetail,
) -> Option<&PickedModule> {
    picked.filter(|picked| Some(picked.base) == selected && picked.shown.answers(detail))
}

/// The Kernel modules tab's keys: Tab hands them between the cards, Shift+Tab
/// shows the detail's next view, and Up, Down and Enter pick and open a
/// module of `shown` in the list, or walk and open the detail's rows.
fn kernel_keys(
    keys: ListKeys,
    listing: &KernelListing,
    shown: &[&WinModule],
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) -> KeyedRows {
    if keys.switches % 2 == 1 {
        pane.list_focus = pane.list_focus.next(Cards::Two);
    }
    // A module's four views cycle within four presses.
    if keys.next_views % 4 != 0 {
        for _ in 0..keys.next_views % 4 {
            pane.kernel_detail = next_module_detail(pane.kernel_detail);
        }
        pane.detail_cursor = None;
    }
    match pane.list_focus {
        ListFocus::List => {
            let at = pane
                .selected_kernel_module
                .and_then(|base| shown.iter().position(|module| module.base == base));
            let walked = walk(keys, at, shown.len());
            if let Some(KeyRow::Moved(to)) = walked.key {
                pane.selected_kernel_module = Some(shown[to].base);
            }
            if let Some(opened) = walked.open {
                *action = Some(DebuggerAction::OpenModule(shown[opened].base));
            }
            KeyedRows {
                list: walked.key,
                ..KeyedRows::default()
            }
        }
        // This tab has no third card: the keys there are the detail's.
        ListFocus::Detail | ListFocus::Module => {
            if !keys.walks() {
                return KeyedRows {
                    detail: pane.detail_cursor.map(KeyRow::Resting),
                    ..KeyedRows::default()
                };
            }
            // A view switched this frame has no answer yet, so lists no rows.
            let filter = pane.export_filter.to_lowercase();
            let card = ModuleCard {
                answer: module_answer(
                    listing.picked.as_ref(),
                    pane.selected_kernel_module,
                    pane.kernel_detail,
                ),
                has_pick: pane.selected_kernel_module.is_some(),
                filter: &filter,
                opener: Opener::Live,
                named: &listing.modules,
            };
            let detail = walk_detail(
                keys,
                |folds| module_detail_rows(card, folds),
                &mut pane.detail_cursor,
                Folds {
                    map_open: &mut pane.expanded_modules,
                    imports_shut: &mut pane.folded_imports,
                },
                action,
            );
            KeyedRows {
                detail,
                ..KeyedRows::default()
            }
        }
    }
}

/// The rows a module's card lists, in the order it draws them: a section, or
/// an export or import with an address, opens in memory; an imported-from
/// module's row folds its functions.
fn module_detail_rows(card: ModuleCard<'_>, folds: &Folds<'_>) -> Vec<DetailRow> {
    let Some(picked) = card.answer else {
        return Vec::new();
    };
    let opens = |address: Option<u64>| address.map_or(RowOpen::Nothing, |at| card.opener.open(at));
    match &picked.shown {
        PickedDetail::Sections(SectionsRead::Mapped(map)) => map
            .regions
            .iter()
            .map(|region| DetailRow::leaf(opens(Some(region.start))))
            .collect(),
        PickedDetail::Exports(ExportRead::Listed(exports)) => exports
            .iter()
            .filter(|export| export_matches(export, card.filter))
            .map(|export| DetailRow::leaf(opens(export_address(picked.base, export))))
            .collect(),
        PickedDetail::Imports(ImportRead::Listed(modules)) => {
            import_lines(modules, card.filter, &*folds.imports_shut)
                .into_iter()
                .map(|line| match line {
                    ImportLine::Group { index, .. } => DetailRow {
                        open: RowOpen::Fold(Fold::ImportGroup(index)),
                        branch: Branch::Parent(Fold::ImportGroup(index)),
                    },
                    ImportLine::Function { group, function, parent } => DetailRow {
                        open: opens(modules[group].functions[function].bound),
                        branch: Branch::Under(parent),
                    },
                    ImportLine::PagedOut { parent, .. } => DetailRow {
                        open: RowOpen::Nothing,
                        branch: Branch::Under(parent),
                    },
                })
                .collect()
        }
        // The Symbols view's actions are buttons, not rows.
        PickedDetail::Sections(SectionsRead::HeaderUnreadable)
        | PickedDetail::Exports(ExportRead::NoTable | ExportRead::NotPe | ExportRead::Unreadable)
        | PickedDetail::Imports(ImportRead::NoTable | ImportRead::NotPe | ImportRead::Unreadable)
        | PickedDetail::Symbols(_) => Vec::new(),
    }
}

/// Whether `text` holds the lowercase `filter`, ASCII letters matched in
/// either case, without allocating: PE export and import names are ASCII,
/// and every row is matched every frame.
fn holds_ignoring_case(text: &str, filter: &str) -> bool {
    filter.is_empty()
        || text
            .as_bytes()
            .windows(filter.len())
            .any(|window| window.eq_ignore_ascii_case(filter.as_bytes()))
}

/// Whether an export's name, or a forwarder's target, holds the lowercase
/// `filter`.
fn export_matches(export: &PeExport, filter: &str) -> bool {
    filter.is_empty()
        || export.name.as_deref().is_some_and(|name| holds_ignoring_case(name, filter))
        || match &export.target {
            ExportTarget::Forwarded(to) => holds_ignoring_case(to, filter),
            ExportTarget::Rva(_) => false,
        }
}

/// Where an export of the module at `base` lies; a forwarder has no address.
fn export_address(base: u64, export: &PeExport) -> Option<u64> {
    match &export.target {
        ExportTarget::Rva(rva) => Some(base.wrapping_add(u64::from(*rva))),
        ExportTarget::Forwarded(_) => None,
    }
}

/// A module card's title, its Sections/Exports/Imports/Symbols switch (and
/// the filter over exports or imports), and the column titles of what it
/// lists.
fn module_detail_header(
    ui: &mut egui::Ui,
    title: Title<'_>,
    detail: &mut ModuleDetail,
    filter: &mut String,
) {
    ui.horizontal_wrapped(|ui| {
        title_label(ui, title.text, title.count);
        segmented(
            ui,
            detail,
            &[
                Segment { value: ModuleDetail::Sections, label: "Sections" },
                Segment { value: ModuleDetail::Exports, label: "Exports" },
                Segment { value: ModuleDetail::Imports, label: "Imports" },
                Segment { value: ModuleDetail::Symbols, label: "Symbols" },
            ],
        );
        match detail {
            ModuleDetail::Exports => labelled_filter(ui, filter, EXPORT_FILTER),
            ModuleDetail::Imports => labelled_filter(ui, filter, IMPORT_FILTER),
            ModuleDetail::Sections | ModuleDetail::Symbols => {}
        }
    });
    match detail {
        ModuleDetail::Sections => header_row(ui, 0.0, &section_columns()),
        ModuleDetail::Exports => header_row(ui, 0.0, &export_columns()),
        ModuleDetail::Imports => header_row(ui, TREE_INDENT, &import_columns()),
        ModuleDetail::Symbols => {}
    }
}

/// The picked module's sections, exports or imports, or why they are not
/// listed; the keys stand on row `key` of them, and the import groups in
/// `imports_shut` are folded.
fn module_detail_body(
    ui: &mut egui::Ui,
    card: ModuleCard<'_>,
    states: ModuleStates<'_>,
    key: Option<KeyRow>,
    imports_shut: &mut BTreeSet<usize>,
    action: &mut Option<DebuggerAction>,
) {
    let muted = |ui: &mut egui::Ui, text: &str| {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new(text).color(TEXT_MUTED));
    };
    let Some(picked) = card.answer else {
        let text = if card.has_pick {
            "Reading the module…"
        } else {
            "Pick a module to see its sections, exports, imports and symbols."
        };
        muted(ui, text);
        return;
    };
    match &picked.shown {
        PickedDetail::Sections(SectionsRead::Mapped(map)) => {
            section_rows(ui, map, card.opener, key, action);
        }
        PickedDetail::Sections(SectionsRead::HeaderUnreadable) => {
            muted(ui, "Its headers are paged out, so its sections cannot be read.");
        }
        PickedDetail::Exports(ExportRead::Listed(exports)) => {
            export_rows(ui, picked.base, exports, card, key, action);
        }
        PickedDetail::Exports(ExportRead::NoTable) => muted(ui, "It exports nothing."),
        PickedDetail::Exports(ExportRead::NotPe) | PickedDetail::Imports(ImportRead::NotPe) => {
            muted(ui, "Its headers do not read as a PE image.");
        }
        PickedDetail::Exports(ExportRead::Unreadable) => muted(ui, "Its export table is paged out."),
        PickedDetail::Imports(ImportRead::Listed(modules)) => {
            import_rows(ui, picked.base, modules, card, key, imports_shut, action);
        }
        PickedDetail::Imports(ImportRead::NoTable) => muted(ui, "It imports nothing."),
        PickedDetail::Imports(ImportRead::Unreadable) => muted(ui, "Its import table is paged out."),
        PickedDetail::Symbols(identity) => module_symbols_body(ui, identity, states.view, action),
    }
}

// ───────────────────────────── Drivers tab ──────────────────────────────

const DRIVER_NAME_CHARS: usize = 28;
const DRIVER_MODULE_CHARS: usize = 18;
const DISPATCH_LABEL_CHARS: usize = 26;

/// Whether a driver's namespace name or its module name holds `filter`.
fn driver_matches(driver: &WinDriver, filter: &str) -> bool {
    filter.is_empty()
        || holds_ignoring_case(&driver.name, filter)
        || driver
            .module_name
            .as_deref()
            .is_some_and(|name| holds_ignoring_case(name, filter))
}

fn driver_columns() -> [Cell; 3] {
    [
        Cell::left("Driver", TEXT_MUTED, DRIVER_NAME_CHARS),
        Cell::left("Module", TEXT_MUTED, DRIVER_MODULE_CHARS),
        Cell::right("Devices", TEXT_MUTED, 7),
    ]
}

/// The run's answer for the driver picked at `selected`, once it is for that
/// driver and the `detail` asked.
fn driver_answer(
    picked: Option<&PickedDriver>,
    selected: Option<u64>,
    detail: DriverDetail,
) -> Option<&PickedDriver> {
    picked.filter(|picked| Some(picked.driver_object) == selected && picked.shown.answers(detail))
}

/// One row per driver: its name, matched module (or a flag when it has none),
/// and device count. A flagged driver's name is amber; the picked one and the
/// keyed row are selected. A click picks it; a double-click opens its module.
fn driver_rows(
    ui: &mut egui::Ui,
    drivers: &[&WinDriver],
    picked: &mut Option<u64>,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    if drivers.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No driver matches the filter.").color(TEXT_MUTED));
        return;
    }
    for (n, driver) in drivers.iter().enumerate() {
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        let name_color = if driver.flagged { ACCENT_AMBER } else { TEXT_PRIMARY };
        let module = match (&driver.module_name, driver.module) {
            (Some(name), _) => name.clone(),
            (None, Some(_)) => "(unnamed)".to_owned(),
            (None, None) => "no module".to_owned(),
        };
        let module_color = if driver.module.is_none() { ACCENT_RED } else { TEXT_MUTED };
        let name = if driver.in_namespace {
            driver.name.clone()
        } else {
            format!("{}  · not in namespace", driver.name)
        };
        let cells = [
            Cell::left(name, name_color, DRIVER_NAME_CHARS),
            Cell::left(module, module_color, DRIVER_MODULE_CHARS),
            Cell::right(driver.device_count.to_string(), TEXT_MUTED, 7),
        ];
        let picked_now = *picked == Some(driver.driver_object) || key.is_some_and(|key| key.at(n));
        let tone = if picked_now { Tone::Selected } else { Tone::striped(n) };
        let response = list_row(ui, 0.0, &cells, tone, RowSense::Pick { selected: picked_now });
        follow_key(&response, key, n);
        let response = response.on_hover_text(
            "Click to see its dispatch table and devices; double-click to show its module.",
        );
        if (response.double_clicked() || response.triple_clicked()) && driver.module.is_some() {
            if let Some(base) = driver.module {
                *action = Some(DebuggerAction::OpenModule(base));
            }
        } else if response.clicked() {
            *picked = Some(driver.driver_object);
        }
    }
}

/// The Drivers tab's keys: Tab hands them between the two cards, Shift+Tab
/// shows the detail's other view, and Up, Down and Enter pick and open a
/// driver in the list, or walk and open the detail's rows.
fn driver_keys(
    keys: ListKeys,
    listing: &DriverListing,
    shown: &[&WinDriver],
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) -> KeyedRows {
    if keys.switches % 2 == 1 {
        pane.list_focus = pane.list_focus.next(Cards::Two);
    }
    if keys.next_views % 2 == 1 {
        pane.driver_detail = next_driver_detail(pane.driver_detail);
        pane.detail_cursor = None;
    }
    match pane.list_focus {
        ListFocus::List => {
            let at = pane
                .selected_driver
                .and_then(|addr| shown.iter().position(|driver| driver.driver_object == addr));
            let walked = walk(keys, at, shown.len());
            if let Some(KeyRow::Moved(to)) = walked.key {
                pane.selected_driver = Some(shown[to].driver_object);
            }
            if let Some(opened) = walked.open {
                if let Some(base) = shown[opened].module {
                    *action = Some(DebuggerAction::OpenModule(base));
                }
            }
            KeyedRows {
                list: walked.key,
                ..KeyedRows::default()
            }
        }
        ListFocus::Detail | ListFocus::Module => {
            let answer = driver_answer(
                listing.picked.as_ref(),
                pane.selected_driver,
                pane.driver_detail,
            );
            let detail = walk_detail(
                keys,
                |_| driver_detail_rows(answer),
                &mut pane.detail_cursor,
                Folds {
                    map_open: &mut pane.expanded_modules,
                    imports_shut: &mut pane.folded_imports,
                },
                action,
            );
            KeyedRows {
                detail,
                ..KeyedRows::default()
            }
        }
    }
}

/// The view a driver's card shows after `detail`.
const fn next_driver_detail(detail: DriverDetail) -> DriverDetail {
    match detail {
        DriverDetail::Dispatch => DriverDetail::Devices,
        DriverDetail::Devices => DriverDetail::Dispatch,
    }
}

/// The rows a driver's detail lists, for the keys: a dispatch handler with a
/// module opens it in memory; a device opens nothing.
fn driver_detail_rows(answer: Option<&PickedDriver>) -> Vec<DetailRow> {
    let Some(picked) = answer else {
        return Vec::new();
    };
    match &picked.shown {
        DriverShown::Dispatch(entries) => entries
            .iter()
            .map(|entry| DetailRow::leaf(dispatch_open(entry)))
            .collect(),
        DriverShown::Devices(rows) => {
            rows.iter().map(|_| DetailRow::leaf(RowOpen::Nothing)).collect()
        }
    }
}

/// What Enter or a click on a dispatch row does: show the handler in memory
/// when it lies in a module, else nothing.
fn dispatch_open(entry: &DispatchEntry) -> RowOpen {
    match entry.site {
        PointerSite::OwnImage | PointerSite::InModule { .. } => {
            RowOpen::Act(DebuggerAction::OpenModule(entry.addr))
        }
        PointerSite::KernelDefault | PointerSite::NoModule => RowOpen::Nothing,
    }
}

/// A driver's card title and its Dispatch / Devices switch.
fn driver_detail_header(ui: &mut egui::Ui, title: Title<'_>, detail: &mut DriverDetail) {
    ui.horizontal_wrapped(|ui| {
        title_label(ui, title.text, title.count);
        segmented(
            ui,
            detail,
            &[
                Segment { value: DriverDetail::Dispatch, label: "Dispatch" },
                Segment { value: DriverDetail::Devices, label: "Devices" },
            ],
        );
    });
}

/// The body of a driver's detail card: its dispatch table or its devices.
fn driver_detail_body(
    ui: &mut egui::Ui,
    answer: Option<&PickedDriver>,
    has_pick: bool,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    let muted = |ui: &mut egui::Ui, text: &str| {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new(text).color(TEXT_MUTED));
    };
    let Some(picked) = answer else {
        let text = if has_pick {
            "Reading the driver…"
        } else {
            "Pick a driver to see its dispatch table and devices."
        };
        muted(ui, text);
        return;
    };
    match &picked.shown {
        DriverShown::Dispatch(entries) => dispatch_rows(ui, entries, key, action),
        DriverShown::Devices(rows) => device_rows(ui, rows, key),
    }
}

/// The text and colour a handler's classification shows.
fn site_text(site: PointerSite, module_name: impl Fn(u64) -> String) -> (String, egui::Color32) {
    match site {
        PointerSite::OwnImage => ("own image".to_owned(), TEXT_PRIMARY),
        PointerSite::KernelDefault => ("kernel default".to_owned(), TEXT_MUTED),
        PointerSite::InModule { base, off } => (format!("{}+0x{off:X}", module_name(base)), ACCENT_CYAN),
        PointerSite::NoModule => ("no module".to_owned(), ACCENT_RED),
    }
}

/// One row per dispatch entry: its name, handler address, and where it leads.
/// A row into another module is clickable; a row into none is red.
fn dispatch_rows(
    ui: &mut egui::Ui,
    entries: &[DispatchEntry],
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    for (n, entry) in entries.iter().enumerate() {
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        // The module an in-module handler names is already in its text.
        let (site, color) = site_text(entry.site, |_| String::new());
        let site = match entry.site {
            PointerSite::InModule { off, .. } => format!("+0x{off:X}"),
            _ => site,
        };
        let label_color = match entry.site {
            PointerSite::NoModule => ACCENT_RED,
            _ => TEXT_PRIMARY,
        };
        let address = if entry.addr == 0 {
            "—".to_owned()
        } else {
            format!("{:016X}", entry.addr)
        };
        let cells = [
            Cell::left(entry.label.clone(), label_color, DISPATCH_LABEL_CHARS),
            Cell::left(address, TEXT_MUTED, 16),
            Cell::fill(site, color),
        ];
        let response = list_row(ui, 0.0, &cells, keyed_tone(key, n), RowSense::Pick { selected: false });
        follow_key(&response, key, n);
        let opener = Opener::Live;
        let address = match dispatch_open(entry) {
            RowOpen::Act(DebuggerAction::OpenModule(at)) => Some(at),
            _ => None,
        };
        click_opens(response, opener, address, "", action);
    }
}

/// One row per device: its name, type, characteristics, with its filter stack
/// and `\??\` links beneath.
fn device_rows(ui: &mut egui::Ui, rows: &[DeviceRow], key: Option<KeyRow>) {
    if rows.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("This driver owns no device objects.").color(TEXT_MUTED));
        return;
    }
    for (n, device) in rows.iter().enumerate() {
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        let name = device.name.clone().unwrap_or_else(|| "(unnamed)".to_owned());
        let cells = [
            Cell::left(name, TEXT_PRIMARY, 32),
            Cell::left(device.device_type.clone(), ACCENT_CYAN, 18),
            Cell::fill(format!("char 0x{:X}", device.characteristics), TEXT_MUTED),
        ];
        let response = list_row(ui, 0.0, &cells, keyed_tone(key, n), RowSense::Pick { selected: false });
        follow_key(&response, key, n);
        for driver in &device.attached_above {
            ui.label(mono(format!("    attached above: {driver}")).color(TEXT_MUTED));
        }
        for link in &device.links {
            ui.label(mono(format!("    {link}")).color(TEXT_MUTED));
        }
    }
}

/// The Drivers tab: the driver list, and the picked driver's dispatch table
/// or devices.
fn drivers_page(
    ui: &mut egui::Ui,
    fit: Fit,
    listing: &DriverListing,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let filter = pane.driver_filter.to_lowercase();
    let shown: Vec<&WinDriver> = listing
        .drivers
        .iter()
        .filter(|driver| driver_matches(driver, &filter))
        .collect();
    let picked_before = (pane.selected_driver, pane.driver_detail);
    let keys = list_keys(ui);
    let keyed_rows = keys.map_or_else(KeyedRows::default, |keys| {
        driver_keys(keys, listing, &shown, pane, action)
    });
    let keyed = keys.map(|_| pane.list_focus);
    let picked_name = pane
        .selected_driver
        .and_then(|addr| listing.drivers.iter().find(|driver| driver.driver_object == addr))
        .map(|driver| driver.name.clone());
    let answer = driver_answer(listing.picked.as_ref(), pane.selected_driver, pane.driver_detail);
    let has_pick = pane.selected_driver.is_some();
    let count = match answer.map(|picked| &picked.shown) {
        Some(DriverShown::Dispatch(entries)) => Some(entries.len()),
        Some(DriverShown::Devices(rows)) => Some(rows.len()),
        None => None,
    };
    let list_title = Title::counted("Drivers", listing.drivers.len()).inlined();
    let detail_title = Title {
        text: picked_name.as_deref().unwrap_or("Driver"),
        count,
        inline: true,
    };

    let mut list_card = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            list_title,
            keyed == Some(ListFocus::List),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    title_label(ui, list_title.text, list_title.count);
                    filter_field(ui, &mut pane.driver_filter);
                });
                header_row(ui, 0.0, &driver_columns());
            },
            |ui| {
                driver_rows(ui, &shown, &mut pane.selected_driver, keyed_rows.list, action);
            },
        );
    };
    let mut detail_card = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            detail_title,
            keyed.is_some_and(|focus| focus != ListFocus::List),
            |ui| {
                driver_detail_header(ui, detail_title, &mut pane.driver_detail);
            },
            |ui| {
                driver_detail_body(ui, answer, has_pick, keyed_rows.detail, action);
            },
        );
    };
    match fit {
        Fit::Grid => {
            let gap = SPACE_ITEM;
            let page = ui.available_size();
            let share = resized(pane.split.drivers, 0.0, page.x, gap, SPLIT_MIN);
            let left = ((page.x - gap) * share).floor();
            let mut moved: Option<f32> = None;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                list_card(ui, Room::Fixed(egui::vec2(left, page.y)), &mut *action);
                match splitter(ui, egui::vec2(gap, page.y), SplitAxis::Columns, RESIZE_MODULE_LIST) {
                    SplitterMove::Still => {}
                    SplitterMove::Dragged(delta) => {
                        moved = Some(resized(share, delta, page.x, gap, SPLIT_MIN));
                    }
                    SplitterMove::Reset => moved = Some(PROCESS_LIST_SHARE),
                }
                detail_card(ui, Room::Fixed(egui::vec2(page.x - gap - left, page.y)), &mut *action);
            });
            if let Some(share) = moved {
                pane.split.drivers = share;
            }
        }
        Fit::Scrolling(columns) => {
            let density = fit.density();
            let room = Room::Content(density);
            let gap = density.gap();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match columns {
                    Columns::Two => side_by_side(
                        ui,
                        gap,
                        PROCESS_LIST_SHARE,
                        action,
                        |ui, action| list_card(ui, room, action),
                        |ui, action| detail_card(ui, room, action),
                    ),
                    Columns::One => {
                        list_card(ui, room, &mut *action);
                        ui.add_space(gap);
                        detail_card(ui, room, &mut *action);
                    }
                });
        }
    }
    // A new pick or the other view asks the run for it, and leaves the cursor
    // on no detail row; a double-click that opened a module stands.
    if (pane.selected_driver, pane.driver_detail) != picked_before {
        pane.detail_cursor = None;
        if !matches!(action, Some(DebuggerAction::OpenModule(_))) {
            *action = Some(DebuggerAction::ShowSystem(DebuggerTab::Drivers.system_view(pane)));
        }
    }
}

// ───────────────────────────── SSDT tab ──────────────────────────────

/// Which system-service table the SSDT tab lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SsdtTable {
    /// `KiServiceTable`, the kernel's own.
    Kernel,
    /// `W32pServiceTable`, in `win32k`.
    Gui,
}

impl SsdtTable {
    /// The title over the table's list.
    const fn title(self) -> &'static str {
        match self {
            Self::Kernel => "Kernel services",
            Self::Gui => "GUI services",
        }
    }
}

const SSDT_INDEX_CHARS: usize = 6;
/// The Service column's width: most `NtUser*`/`NtGdi*` names fit.
const SSDT_NAME_CHARS: usize = 30;

fn ssdt_columns() -> [Cell; 4] {
    [
        Cell::right("#", TEXT_MUTED, SSDT_INDEX_CHARS),
        Cell::left("Service", TEXT_MUTED, SSDT_NAME_CHARS),
        Cell::left("Routine", TEXT_MUTED, 16),
        Cell::fill("Target", TEXT_MUTED),
    ]
}

/// How the SSDT tab shows one service.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ServiceView {
    /// The stub's name, else the routine's own symbol when the routine begins
    /// a function in its owner; empty when neither is known.
    name: String,
    /// Where the service leads, and what is wrong with that.
    target: String,
    color: egui::Color32,
}

/// One listed service and how it shows.
struct ShownService<'a> {
    service: &'a SsdtService,
    view: ServiceView,
}

/// How `service` shows, its routine named by `names`. In red: an entry that
/// leads out of the table's owner, or a routine whose first instruction
/// jumps out of it. In amber: an entry whose routine's own symbol is not
/// the service its stub names — rerouted inside the owner, or a routine the
/// linker folded with another of the same code.
fn service_view(service: &SsdtService, names: Namer<'_>) -> ServiceView {
    let symbol = match service.site {
        PointerSite::OwnImage => names
            .name(service.routine)
            .filter(NamedAddress::starts_function)
            .and_then(|named| named.function)
            .map(|function| function.name),
        PointerSite::InModule { .. } | PointerSite::KernelDefault | PointerSite::NoModule => None,
    };
    let name = service.stub_name.clone().or_else(|| symbol.clone()).unwrap_or_default();
    let module = service.module_name.as_deref().unwrap_or("kernel");
    let rerouted = symbol
        .as_deref()
        .filter(|symbol| service.stub_name.as_deref().is_some_and(|stub| stub != *symbol));
    let shown = |target: String, color: egui::Color32| ServiceView {
        name: name.clone(),
        target,
        color,
    };
    if let Some(jump) = &service.inline {
        let landing = match &jump.module_name {
            Some(landing) => format!("{landing} {:016X}", jump.target),
            None => format!("{:016X}, in no module", jump.target),
        };
        return shown(format!("{module}  · inline hook, jumps to {landing}"), ACCENT_RED);
    }
    match service.site {
        PointerSite::OwnImage => match rerouted {
            Some(symbol) => shown(format!("{module}  · rerouted to {symbol}"), ACCENT_AMBER),
            None => shown(module.to_owned(), TEXT_PRIMARY),
        },
        PointerSite::InModule { off, .. } => {
            let landing = service.module_name.as_deref().unwrap_or("module");
            shown(format!("{landing}+0x{off:X}  · hooked"), ACCENT_RED)
        }
        PointerSite::KernelDefault => shown(module.to_owned(), TEXT_PRIMARY),
        PointerSite::NoModule => shown("no module  · hooked".to_owned(), ACCENT_RED),
    }
}

/// Whether a service's name or module name holds `filter`, or its index
/// reads `filter`.
fn ssdt_matches(shown: &ShownService<'_>, filter: &str) -> bool {
    filter.is_empty()
        || holds_ignoring_case(&shown.view.name, filter)
        || shown
            .service
            .module_name
            .as_deref()
            .is_some_and(|name| holds_ignoring_case(name, filter))
        || shown.service.index.to_string().contains(filter)
}

/// Where a click or Enter on a service shows its routine: in memory when it
/// lies in a known module.
fn ssdt_open(service: &SsdtService) -> Option<u64> {
    match service.site {
        PointerSite::OwnImage | PointerSite::InModule { .. } => Some(service.routine),
        PointerSite::KernelDefault | PointerSite::NoModule => None,
    }
}

/// One row per service: its index, name, routine address, and where it
/// leads, coloured as [`service_view`] says. The keys stand on `key`.
fn ssdt_rows(
    ui: &mut egui::Ui,
    services: &[ShownService<'_>],
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    if services.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No service matches the filter.").color(TEXT_MUTED));
        return;
    }
    for (n, shown) in services.iter().enumerate() {
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        let service = shown.service;
        let cells = [
            Cell::right(service.index.to_string(), TEXT_MUTED, SSDT_INDEX_CHARS),
            Cell::left(shown.view.name.clone(), TEXT_PRIMARY, SSDT_NAME_CHARS),
            Cell::left(format!("{:016X}", service.routine), TEXT_MUTED, 16),
            Cell::fill(shown.view.target.clone(), shown.view.color),
        ];
        let response = list_row(ui, 0.0, &cells, keyed_tone(key, n), RowSense::Pick { selected: false });
        follow_key(&response, key, n);
        click_opens(response, Opener::Live, ssdt_open(service), "", action);
    }
}

/// Under the SSDT title: how the shown table's descriptor was found, and a
/// table that lies outside its owner (swapped whole), in red.
fn ssdt_table_lines(ui: &mut egui::Ui, table: &ServiceTableRead) {
    ui.label(RichText::new(table.source.label()).small().color(TEXT_MUTED));
    match table.place {
        TablePlace::InOwner => {}
        TablePlace::Outside => {
            ui.label(
                RichText::new(format!(
                    "The table itself lies outside its owner, at {:016X}: it was swapped whole.",
                    table.base
                ))
                .color(ACCENT_RED),
            );
        }
    }
}

/// The SSDT tab: the kernel's system services or the GUI's, one filterable
/// list, each service named by its stub and classified against the loaded
/// modules so a hooked entry stands out.
fn ssdt_page(
    ui: &mut egui::Ui,
    fit: Fit,
    listing: &SsdtListing,
    names: Namer<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let picked: Result<&ServiceTableRead, GuiMiss> = match pane.ssdt_table {
        SsdtTable::Kernel => Ok(&listing.kernel),
        SsdtTable::Gui => match &listing.gui {
            GuiServices::Listed(table) => Ok(table),
            GuiServices::Missing(miss) => Err(*miss),
        },
    };
    let filter = pane.list_filter.to_lowercase();
    let shown: Vec<ShownService<'_>> = match picked {
        Ok(table) => table
            .services
            .iter()
            .map(|service| ShownService {
                service,
                view: service_view(service, names),
            })
            .filter(|shown| ssdt_matches(shown, &filter))
            .collect(),
        Err(_) => Vec::new(),
    };
    // One list: the keys always move in it, opening a service's routine.
    let keyed_row = list_keys(ui).and_then(|keys| {
        let walked = walk(keys, pane.detail_cursor, shown.len());
        if let Some(KeyRow::Moved(to)) = walked.key {
            pane.detail_cursor = Some(to);
        }
        if let Some(opened) = walked.open {
            if let Some(routine) = shown.get(opened).and_then(|shown| ssdt_open(shown.service)) {
                *action = Some(DebuggerAction::OpenModule(routine));
            }
        }
        walked.key
    });
    let gui_hooked = match &listing.gui {
        GuiServices::Listed(table) => table.any_hooked,
        GuiServices::Missing(_) => false,
    };
    // Hooks in the table not shown are said too, so none hides behind the switch.
    let hooks = match pane.ssdt_table {
        SsdtTable::Kernel if listing.kernel.any_hooked => Some("hooks present"),
        SsdtTable::Gui if gui_hooked => Some("hooks present"),
        SsdtTable::Kernel if gui_hooked => Some("the GUI table has hooks"),
        SsdtTable::Gui if listing.kernel.any_hooked => Some("the kernel table has hooks"),
        SsdtTable::Kernel | SsdtTable::Gui => None,
    };
    let count = match picked {
        Ok(table) => table.services.len(),
        Err(_) => 0,
    };
    let title = Title::counted(pane.ssdt_table.title(), count).inlined();
    let mut card = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            title,
            keyed_row.is_some(),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    title_label(ui, title.text, title.count);
                    let tables = [
                        Segment { value: SsdtTable::Kernel, label: "Kernel" },
                        Segment { value: SsdtTable::Gui, label: "GUI" },
                    ];
                    if segmented(ui, &mut pane.ssdt_table, &tables) {
                        pane.detail_cursor = None;
                    }
                    if let Some(hooks) = hooks {
                        ui.label(RichText::new(hooks).color(ACCENT_RED));
                    }
                    filter_field(ui, &mut pane.list_filter);
                });
                if let Ok(table) = picked {
                    ssdt_table_lines(ui, table);
                    header_row(ui, 0.0, &ssdt_columns());
                }
            },
            |ui| match picked {
                Ok(_) => ssdt_rows(ui, &shown, keyed_row, action),
                Err(miss) => {
                    ui.add_space(SPACE_ITEM);
                    ui.label(RichText::new(miss.label()).color(TEXT_MUTED));
                }
            },
        );
    };
    match fit {
        Fit::Grid => {
            card(ui, Room::Fixed(ui.available_size()), action);
        }
        Fit::Scrolling(_) => {
            let room = Room::Content(fit.density());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| card(ui, room, action));
        }
    }
}

/// Hovers `response` with what a click on its address shows, and asks for it
/// on a click. `None` for `address` says `no_address`.
fn click_opens(
    response: egui::Response,
    opener: Opener,
    address: Option<u64>,
    no_address: &str,
    action: &mut Option<DebuggerAction>,
) {
    match address.map(|at| opener.open(at)) {
        Some(RowOpen::Act(open)) => {
            if response.on_hover_text(opener.hint()).clicked() {
                *action = Some(open);
            }
        }
        Some(RowOpen::Fold(_) | RowOpen::Nothing) => {
            response.on_hover_text(opener.hint());
        }
        None => {
            response.on_hover_text(no_address);
        }
    }
}

const SECTION_NAME_CHARS: usize = 9;

fn section_columns() -> [Cell; 6] {
    [
        Cell::left("Section", TEXT_MUTED, SECTION_NAME_CHARS),
        Cell::left("Address", TEXT_MUTED, 16),
        Cell::right("Size", TEXT_MUTED, MEMORY_SIZE_CHARS),
        Cell::right("Resident", TEXT_MUTED, MEMORY_RESIDENT_CHARS),
        Cell::left("", TEXT_MUTED, MEMORY_GAUGE_CHARS),
        Cell::fill("Access", TEXT_MUTED),
    ]
}

/// A module's header page and sections, one a row: name, address, size, how
/// much is resident, and access — executable code cyan. A click shows the
/// region where `opener` shows it; the keys stand on row `key`.
fn section_rows(
    ui: &mut egui::Ui,
    map: &MemoryMap,
    opener: Opener,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    for (n, region) in map.regions.iter().enumerate() {
        let name = match &region.owner {
            RegionOwner::Header { .. } => "headers",
            RegionOwner::Section { section, .. } => section.as_str(),
            RegionOwner::Stack { .. } | RegionOwner::Private => "(other)",
        };
        let executable = region.access.is_some_and(|access| access.executable);
        let color = if executable { ACCENT_CYAN } else { TEXT_PRIMARY };
        let fraction = region.resident_pages as f32 / region.total_pages.max(1) as f32;
        let gauge = if executable {
            Cell::gauge_colored(fraction, MEMORY_GAUGE_CHARS, ACCENT_CYAN)
        } else {
            Cell::gauge(fraction, MEMORY_GAUGE_CHARS)
        };
        let cells = [
            Cell::left(name, color, SECTION_NAME_CHARS),
            Cell::left(format!("{:016X}", region.start), TEXT_MUTED, 16),
            Cell::right(human_size(region.size), TEXT_MUTED, MEMORY_SIZE_CHARS),
            Cell::right(
                format!("{}/{}", region.resident_pages, region.total_pages),
                TEXT_MUTED,
                MEMORY_RESIDENT_CHARS,
            ),
            gauge,
            Cell::fill(region_access_text(region), color),
        ];
        let response = list_row(ui, 0.0, &cells, keyed_tone(key, n), RowSense::Pick { selected: false });
        follow_key(&response, key, n);
        click_opens(response, opener, Some(region.start), "", action);
    }
}

const EXPORT_ORDINAL_CHARS: usize = 6;

fn export_columns() -> [Cell; 3] {
    [
        Cell::left("Address", TEXT_MUTED, 16),
        Cell::right("Ord", TEXT_MUTED, EXPORT_ORDINAL_CHARS),
        Cell::fill("Name", TEXT_MUTED),
    ]
}

/// A module's exports at `base`, one a row, keeping those whose name (or a
/// forwarder's target) holds the card's filter: address, ordinal, name. A
/// forwarder shows where it leads instead of an address. A click on an
/// export with an address shows it where the card's opener shows it; the
/// keys stand on row `key` of those listed.
fn export_rows(
    ui: &mut egui::Ui,
    base: u64,
    exports: &[PeExport],
    card: ModuleCard<'_>,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    let mut any = false;
    for (n, export) in exports.iter().filter(|export| export_matches(export, card.filter)).enumerate() {
        any = true;
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        let name = export.name.as_deref().unwrap_or("(by ordinal)");
        // Where a click goes: an export with an address; a forwarder has none.
        let target = export_address(base, export);
        let address = target.map_or_else(|| "forwarded".to_owned(), |at| format!("{at:016X}"));
        let label = match &export.target {
            ExportTarget::Rva(_) => name.to_owned(),
            ExportTarget::Forwarded(to) => format!("{name}  forwarded to {to}"),
        };
        let cells = [
            Cell::left(address, TEXT_MUTED, 16),
            Cell::right(export.ordinal.to_string(), TEXT_MUTED, EXPORT_ORDINAL_CHARS),
            Cell::fill(label, TEXT_PRIMARY),
        ];
        let response = list_row(ui, 0.0, &cells, keyed_tone(key, n), RowSense::Pick { selected: false });
        follow_key(&response, key, n);
        if let Some(at) = target {
            response.context_menu(|ui| {
                if ui.button(format!("{REFERENCES_TO} {name}")).clicked() {
                    *action = Some(DebuggerAction::FindReferences(address_target(at, name.to_owned())));
                    ui.close();
                }
            });
        }
        click_opens(
            response,
            card.opener,
            target,
            "Forwarded: the function lives in the module named.",
            action,
        );
    }
    if !any {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No export matches the filter.").color(TEXT_MUTED));
    }
}

/// The widest import name the Name column holds before it clips.
const IMPORT_NAME_CHARS: usize = 28;

fn import_columns() -> [Cell; 3] {
    [
        Cell::left("Slot", TEXT_MUTED, 16),
        Cell::left("Name", TEXT_MUTED, IMPORT_NAME_CHARS),
        Cell::fill("Bound to", TEXT_MUTED),
    ]
}

/// One row of a module's imports as the card lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportLine {
    /// The module imported from, by its index in the import table: how many
    /// of its functions are listed, and whether they show.
    Group { index: usize, listed: usize, open: bool },
    /// Function `function` of the group at `group`, under the group's row
    /// `parent`.
    Function { group: usize, function: usize, parent: usize },
    /// The group's functions past a paged-out entry, under its row `parent`.
    PagedOut { group: usize, parent: usize },
}

/// Whether an import's name holds the lowercase `filter`; one by ordinal
/// matches as `#ordinal`.
fn import_matches(import: &PeImport, filter: &str) -> bool {
    match &import.name {
        ImportName::Named { name, .. } => holds_ignoring_case(name, filter),
        ImportName::Ordinal(ordinal) => format!("#{ordinal}").contains(filter),
        ImportName::Overwritten | ImportName::PagedOut => false,
    }
}

/// The rows of `modules`' imports the card lists: each module imported from
/// whose name, or one of whose functions, holds the lowercase `filter`, its
/// listed functions under it unless it is folded in `shut`.
fn import_lines(modules: &[ImportedModule], filter: &str, shut: &BTreeSet<usize>) -> Vec<ImportLine> {
    let mut lines = Vec::new();
    for (index, module) in modules.iter().enumerate() {
        let whole = filter.is_empty()
            || module.name.as_deref().is_some_and(|name| holds_ignoring_case(name, filter));
        let open = !shut.contains(&index);
        let parent = lines.len();
        // The group's row goes first; its count is known once its functions
        // are matched.
        lines.push(ImportLine::Group { index, listed: 0, open });
        let mut listed = 0;
        for (function, import) in module.functions.iter().enumerate() {
            if whole || import_matches(import, filter) {
                listed += 1;
                if open {
                    lines.push(ImportLine::Function { group: index, function, parent });
                }
            }
        }
        if listed == 0 && !whole {
            lines.truncate(parent);
            continue;
        }
        lines[parent] = ImportLine::Group { index, listed, open };
        if open {
            match module.end {
                ThunksEnd::PagedOut => lines.push(ImportLine::PagedOut { group: index, parent }),
                ThunksEnd::Terminated => {}
            }
        }
    }
    lines
}

/// The module in `named` that holds `address`, by name.
fn module_named_at(named: &[WinModule], address: u64) -> Option<&str> {
    named
        .iter()
        .find(|module| (module.base..module.base.saturating_add(u64::from(module.size))).contains(&address))
        .and_then(|module| module.name.as_deref())
}

/// A module's imports, the image based at `base`: a row per module imported
/// from, folding its functions, and under it a row per function — its IAT
/// slot, its name, and where the loaded slot points, with the module there.
/// A click on a function shows where it was bound; the keys stand on row
/// `key`, and the groups in `shut` are folded.
fn import_rows(
    ui: &mut egui::Ui,
    base: u64,
    modules: &[ImportedModule],
    card: ModuleCard<'_>,
    key: Option<KeyRow>,
    shut: &mut BTreeSet<usize>,
    action: &mut Option<DebuggerAction>,
) {
    let lines = import_lines(modules, card.filter, shut);
    if lines.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No import matches the filter.").color(TEXT_MUTED));
        return;
    }
    let mut toggled = None;
    for (n, line) in lines.iter().enumerate() {
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        match *line {
            ImportLine::Group { index, listed, open } => {
                let name = modules[index].name.as_deref().unwrap_or("(name paged out)");
                let tone = if key.is_some_and(|key| key.at(n)) {
                    Tone::Selected
                } else {
                    Tone::Plain
                };
                let cells = [Cell::fill(format!("{name}  {listed} functions"), TEXT_PRIMARY)];
                let response =
                    list_row(ui, TREE_INDENT, &cells, tone, RowSense::Pick { selected: open });
                lead_chevron(ui, response.rect, open, TEXT_PRIMARY);
                follow_key(&response, key, n);
                if response
                    .on_hover_text("Click to show or hide what it imports from this module.")
                    .clicked()
                {
                    toggled = Some(index);
                }
            }
            ImportLine::Function { group, function, .. } => {
                let import = &modules[group].functions[function];
                let name = match &import.name {
                    ImportName::Named { name, .. } => name.clone(),
                    ImportName::Ordinal(ordinal) => format!("#{ordinal}"),
                    ImportName::Overwritten => "(name overwritten by the loader)".to_owned(),
                    ImportName::PagedOut => "(name paged out)".to_owned(),
                };
                let bound = match import.bound {
                    Some(at) => match module_named_at(card.named, at) {
                        Some(module) => format!("{at:016X}  {module}"),
                        None => format!("{at:016X}"),
                    },
                    None => "(slot paged out)".to_owned(),
                };
                let cells = [
                    Cell::left(
                        format!("{:016X}", base.wrapping_add(u64::from(import.slot))),
                        TEXT_MUTED,
                        16,
                    ),
                    Cell::left(name, TEXT_PRIMARY, IMPORT_NAME_CHARS),
                    Cell::fill(bound, TEXT_MUTED),
                ];
                let response =
                    list_row(ui, TREE_INDENT, &cells, keyed_tone(key, n), RowSense::Pick { selected: false });
                follow_key(&response, key, n);
                click_opens(
                    response,
                    card.opener,
                    import.bound,
                    "Its IAT slot is paged out, so where it was bound is unknown.",
                    action,
                );
            }
            ImportLine::PagedOut { .. } => {
                let cells = [Cell::fill(
                    "The rest of this module's imports is paged out.",
                    TEXT_MUTED,
                )];
                list_row(ui, TREE_INDENT, &cells, keyed_tone(key, n), RowSense::Look);
            }
        }
    }
    if let Some(index) = toggled {
        if !shut.remove(&index) {
            shut.insert(index);
        }
    }
}

/// A module passes a lowercase `filter` when its name or path holds it.
fn module_matches(module: &WinModule, filter: &str) -> bool {
    filter.is_empty()
        || [module.name.as_deref(), module.path.as_deref()]
            .into_iter()
            .flatten()
            .any(|text| text.to_lowercase().contains(filter))
}

fn processes_page(
    ui: &mut egui::Ui,
    fit: Fit,
    listing: &ProcessListing,
    current_user_module: Option<u64>,
    states: ModuleStates<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let chosen = listing
        .modules
        .as_ref()
        .and_then(|modules| listing.processes.iter().find(|p| p.eprocess == modules.eprocess));
    let navigable = chosen.is_some_and(|process| process.current);
    // The chosen process's address space, for browsing it when it is not the
    // running one.
    let process_cr3 = chosen.map(|process| process.cr3);
    let opener = Opener::of_process(navigable, process_cr3);
    let rows = process_rows_shown(
        &listing.processes,
        &listing.tree,
        pane.process_layout,
        &pane.list_filter,
    );
    let listed: &[WinModule] = match listing.modules.as_ref().map(|modules| &modules.read) {
        Some(ModuleRead::Listed(modules)) => modules,
        Some(ModuleRead::NoUserSpace | ModuleRead::Unreadable) | None => &[],
    };
    // What the frame starts from: whatever changes of it by the frame's end
    // is asked of the run again.
    let asked_before = DebuggerTab::Processes.system_view(pane);
    let picked_before = (pane.selected_process, pane.process_detail);
    let module_before = (pane.selected_process_module, pane.process_module_detail);
    let keys = list_keys(ui);
    let keyed_rows = keys.map_or_else(KeyedRows::default, |keys| {
        let tab = ProcessTab {
            listing,
            rows: &rows,
            chosen,
            opener,
            listed,
        };
        process_keys(keys, tab, pane, action)
    });
    let keyed = keys.map(|_| pane.list_focus);
    let title = match chosen {
        Some(process) => format!("Modules of {} (PID {})", process.name, process.pid),
        None => "Modules".to_owned(),
    };
    let module_count = match listing.modules.as_ref().map(|modules| &modules.read) {
        Some(ModuleRead::Listed(modules)) => Some(modules.len()),
        Some(ModuleRead::NoUserSpace | ModuleRead::Unreadable) | None => None,
    };
    let detail = pane.process_detail;
    let right_count = match detail {
        ProcessDetail::Modules => module_count,
        ProcessDetail::Threads => listing
            .modules
            .as_ref()
            .map(|modules| modules.threads.len()),
        ProcessDetail::MemoryMap => listing
            .modules
            .as_ref()
            .and_then(|modules| modules.map.as_ref())
            .map(|map| map.regions.len()),
    };
    let right_title = Title {
        text: &title,
        count: right_count,
        inline: true,
    };
    let process_title = Title::counted("Processes", listing.processes.len()).inlined();
    // The current module is only the running process's; another process's
    // list shares bases (ntdll loads at one address everywhere) but is not
    // where the processor runs, so it gets no mark.
    let highlight = if navigable { current_user_module } else { None };
    let scroll_was_pending = pane.scroll_to.is_some();
    // Read before the module card's field takes the filter for this frame.
    let module_filter = pane.process_module_filter.to_lowercase();
    let module_card = ModuleCard {
        answer: module_answer(
            listing.modules.as_ref().and_then(|modules| modules.picked.as_ref()),
            pane.selected_process_module,
            pane.process_module_detail,
        ),
        has_pick: pane.selected_process_module.is_some(),
        filter: &module_filter,
        opener,
        named: listed,
    };
    let module_name = pane
        .selected_process_module
        .and_then(|base| listed.iter().find(|module| module.base == base))
        .map(|module| module.name.clone().unwrap_or_else(|| format!("{:X}", module.base)));
    let module_title = Title {
        text: module_name.as_deref().unwrap_or("Module"),
        count: module_card.count(),
        inline: true,
    };
    // The Modules/Memory-map switch and the minimap report through these
    // locals, so the right card's two closures borrow disjoint fields.
    let mut switch_to: Option<ProcessDetail> = None;
    let mut minimap_click: Option<u64> = None;
    // The cards, each drawn in the room the layout below gives it. They take
    // `action` as an argument so none holds it between calls.
    let mut process_card = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            process_title,
            keyed == Some(ListFocus::List),
            |ui| process_header(ui, process_title, &mut pane.process_layout, &mut pane.list_filter),
            |ui| {
                process_rows(
                    ui,
                    &listing.processes,
                    &rows,
                    chosen,
                    &mut pane.selected_process,
                    keyed_rows.list,
                    action,
                );
            },
        );
    };
    // The picked module's card: under the modules, in the Modules view.
    let mut module_card_drawn = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            module_title,
            keyed == Some(ListFocus::Module),
            |ui| {
                module_detail_header(
                    ui,
                    module_title,
                    &mut pane.process_module_detail,
                    &mut pane.process_module_filter,
                );
            },
            |ui| {
                module_detail_body(ui, module_card, states, keyed_rows.module, &mut pane.folded_imports, action);
            },
        );
    };
    let mut detail_card = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            right_title,
            keyed == Some(ListFocus::Detail),
            |ui| {
                right_card_header(
                    ui,
                    right_title,
                    pane.process_detail,
                    map_of(listing),
                    &mut switch_to,
                    &mut minimap_click,
                );
            },
            |ui| {
                right_card_body(
                    ui,
                    RightCard {
                        detail,
                        listing,
                        opener,
                        highlight,
                        states,
                    },
                    &mut pane.selected_process_module,
                    &mut pane.expanded_modules,
                    &mut pane.scroll_to,
                    keyed_rows.detail,
                    action,
                );
            },
        );
    };
    let with_module = detail == ProcessDetail::Modules;
    match fit {
        Fit::Grid => {
            let gap = SPACE_ITEM;
            let page = ui.available_size();
            // Laid out at the share clamped to today's width (see
            // `grid_layout`); a drag moves on from there.
            let share = resized(pane.split.processes, 0.0, page.x, gap, SPLIT_MIN);
            let left = ((page.x - gap) * share).floor();
            // Only a drag or a reset changes the stored preference.
            let mut moved: Option<f32> = None;
            let mut modules_share = pane.split.modules;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                process_card(ui, Room::Fixed(egui::vec2(left, page.y)), &mut *action);
                match splitter(ui, egui::vec2(gap, page.y), SplitAxis::Columns, RESIZE_PROCESS_LIST) {
                    SplitterMove::Still => {}
                    SplitterMove::Dragged(delta) => {
                        moved = Some(resized(share, delta, page.x, gap, SPLIT_MIN));
                    }
                    SplitterMove::Reset => moved = Some(PROCESS_LIST_SHARE),
                }
                let right = egui::vec2(page.x - gap - left, page.y);
                if with_module {
                    split_column(
                        ui,
                        right,
                        gap,
                        Split {
                            share: &mut modules_share,
                            default: PROCESS_LIST_SHARE,
                            label: RESIZE_MODULE_DETAIL,
                        },
                        &mut *action,
                        |ui, size, action| detail_card(ui, Room::Fixed(size), action),
                        |ui, size, action| module_card_drawn(ui, Room::Fixed(size), action),
                    );
                } else {
                    detail_card(ui, Room::Fixed(right), &mut *action);
                }
            });
            if let Some(share) = moved {
                pane.split.processes = share;
            }
            pane.split.modules = modules_share;
        }
        Fit::Scrolling(columns) => {
            let density = fit.density();
            let room = Room::Content(density);
            let gap = density.gap();
            let mut right_column = |ui: &mut egui::Ui, action: &mut Option<DebuggerAction>| {
                detail_card(ui, room, action);
                if with_module {
                    ui.add_space(gap);
                    module_card_drawn(ui, room, action);
                }
            };
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match columns {
                    Columns::Two => side_by_side(
                        ui,
                        gap,
                        PROCESS_LIST_SHARE,
                        action,
                        |ui, action| process_card(ui, room, action),
                        |ui, action| right_column(ui, action),
                    ),
                    Columns::One => {
                        process_card(ui, room, &mut *action);
                        ui.add_space(gap);
                        right_column(ui, &mut *action);
                    }
                });
        }
    }
    // A target that was pending when the frame began and is still pending
    // drew no row to consume it: it is dropped, not left to fire later when a
    // row happens to appear.
    if scroll_was_pending && pane.scroll_to.is_some() {
        pane.scroll_to = None;
    }
    // A minimap click this frame becomes next frame's scroll target, after
    // both right-card closures have ended (they borrow disjoint fields). A
    // section of a collapsed module has no row until its module is open, so
    // the module is opened for it.
    if let Some(address) = minimap_click {
        pane.scroll_to = Some(address);
        let section_owner = map_of(listing).and_then(|map| {
            map.regions
                .iter()
                .find(|region| region.start == address)
                .and_then(|region| match &region.owner {
                    RegionOwner::Section { base, .. } => Some(*base),
                    RegionOwner::Header { .. } | RegionOwner::Stack { .. } | RegionOwner::Private => {
                        None
                    }
                })
        });
        if let Some(base) = section_owner {
            pane.expanded_modules.insert(base);
        }
    }
    if let Some(chosen_detail) = switch_to {
        pane.process_detail = chosen_detail;
    }
    // A process picked anew lists its own modules, none of them picked; a
    // new pick or view leaves its card's cursor on no row and every import
    // group open.
    if pane.selected_process != picked_before.0 {
        pane.selected_process_module = None;
    }
    if (pane.selected_process, pane.process_detail) != picked_before {
        pane.detail_cursor = None;
    }
    if (pane.selected_process_module, pane.process_module_detail) != module_before {
        pane.module_cursor = None;
        pane.folded_imports.clear();
    }
    // Whatever changed is asked of the run, unless the frame asked to show
    // something else instead (a double-click, an Enter).
    let asked = DebuggerTab::Processes.system_view(pane);
    let shows_elsewhere = matches!(
        action,
        Some(
            DebuggerAction::View(_)
                | DebuggerAction::OpenModule(_)
                | DebuggerAction::ViewInProcess { .. }
        )
    );
    if asked != asked_before && !shows_elsewhere {
        *action = Some(DebuggerAction::ShowSystem(asked));
    }
}

/// What the Processes tab's keys walk: the listing, the rows its process
/// list shows, the process whose detail is shown, where its addresses open,
/// and its listed modules.
#[derive(Clone, Copy)]
struct ProcessTab<'a> {
    listing: &'a ProcessListing,
    rows: &'a [ShownProcess],
    chosen: Option<&'a WinProcess>,
    opener: Opener,
    listed: &'a [WinModule],
}

/// The detail views in the order the switch shows them, each followed by the
/// one Shift+Tab shows next.
fn next_process_detail(detail: ProcessDetail) -> ProcessDetail {
    match detail {
        ProcessDetail::Modules => ProcessDetail::MemoryMap,
        ProcessDetail::MemoryMap => ProcessDetail::Threads,
        ProcessDetail::Threads => ProcessDetail::Modules,
    }
}

/// The Processes tab's keys: Tab hands them to the next card (the process
/// list, its modules or other detail, the picked module's card); Shift+Tab
/// shows the next view of the module's card when the keys are in it, else
/// of the process's detail; Up, Down and Enter pick and view a process in
/// the list, pick and open a module in the modules, or walk and open the
/// rows of the other cards.
fn process_keys(
    keys: ListKeys,
    tab: ProcessTab<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) -> KeyedRows {
    let cards = |pane: &DebuggerPane| match pane.process_detail {
        ProcessDetail::Modules => Cards::Three,
        ProcessDetail::MemoryMap | ProcessDetail::Threads => Cards::Two,
    };
    // A view without the module's card left the keys there: they are the
    // detail's.
    if pane.list_focus == ListFocus::Module && cards(pane) == Cards::Two {
        pane.list_focus = ListFocus::Detail;
    }
    // Two and three cards both cycle within six presses.
    for _ in 0..keys.switches % 6 {
        pane.list_focus = pane.list_focus.next(cards(pane));
    }
    // A view switched this frame has not been read yet: what the listing
    // holds for it may be an older read, so its rows are not walked.
    // Three process views and four module views both cycle within twelve
    // presses.
    let switched = keys.next_views % 12 != 0;
    for _ in 0..keys.next_views % 12 {
        match pane.list_focus {
            ListFocus::Module => {
                pane.process_module_detail = next_module_detail(pane.process_module_detail);
            }
            ListFocus::List | ListFocus::Detail => {
                pane.process_detail = next_process_detail(pane.process_detail);
            }
        }
    }
    let ProcessTab {
        listing,
        rows,
        chosen,
        opener,
        listed,
    } = tab;
    match pane.list_focus {
        ListFocus::List => {
            // The process picked, or the one shown before any pick.
            let shown = pane.selected_process.or(chosen.map(|process| process.eprocess));
            let at = shown.and_then(|eprocess| {
                rows.iter()
                    .position(|row| listing.processes[row.index].eprocess == eprocess)
            });
            let walked = walk(keys, at, rows.len());
            if let Some(KeyRow::Moved(to)) = walked.key {
                let eprocess = listing.processes[rows[to].index].eprocess;
                if shown != Some(eprocess) {
                    pane.selected_process = Some(eprocess);
                }
            }
            if let Some(opened) = walked.open {
                // What a double-click does (see `process_rows`).
                let process = &listing.processes[rows[opened].index];
                let context = if process.current {
                    ViewContext::Live
                } else {
                    ViewContext::Process {
                        eprocess: process.eprocess,
                    }
                };
                *action = Some(DebuggerAction::View(context));
            }
            KeyedRows {
                list: walked.key,
                ..KeyedRows::default()
            }
        }
        // The modules: the keys pick one, as a click does, and Enter opens it.
        ListFocus::Detail if pane.process_detail == ProcessDetail::Modules => {
            let at = pane
                .selected_process_module
                .and_then(|base| listed.iter().position(|module| module.base == base));
            let walked = walk(keys, at, listed.len());
            if let Some(KeyRow::Moved(to)) = walked.key {
                pane.selected_process_module = Some(listed[to].base);
            }
            if let Some(opened) = walked.open {
                match opener.open(listed[opened].base) {
                    RowOpen::Act(open) => *action = Some(open),
                    RowOpen::Fold(_) | RowOpen::Nothing => {}
                }
            }
            KeyedRows {
                detail: walked.key,
                ..KeyedRows::default()
            }
        }
        ListFocus::Detail => {
            let view = pane.process_detail;
            let mut cursor = pane.detail_cursor;
            let detail = walk_detail(
                keys,
                |folds| {
                    if switched {
                        Vec::new()
                    } else {
                        process_detail_rows(listing, view, opener, folds)
                    }
                },
                &mut cursor,
                pane.folds(),
                action,
            );
            pane.detail_cursor = cursor;
            KeyedRows {
                detail,
                ..KeyedRows::default()
            }
        }
        ListFocus::Module => {
            if !keys.walks() {
                return KeyedRows {
                    module: pane.module_cursor.map(KeyRow::Resting),
                    ..KeyedRows::default()
                };
            }
            let filter = pane.process_module_filter.to_lowercase();
            let card = ModuleCard {
                answer: module_answer(
                    listing.modules.as_ref().and_then(|modules| modules.picked.as_ref()),
                    pane.selected_process_module,
                    pane.process_module_detail,
                ),
                has_pick: pane.selected_process_module.is_some(),
                filter: &filter,
                opener,
                named: listed,
            };
            let mut cursor = pane.module_cursor;
            let module = walk_detail(
                keys,
                |folds| {
                    if switched {
                        Vec::new()
                    } else {
                        module_detail_rows(card, folds)
                    }
                },
                &mut cursor,
                pane.folds(),
                action,
            );
            pane.module_cursor = cursor;
            KeyedRows {
                module,
                ..KeyedRows::default()
            }
        }
    }
}

/// The rows the chosen process's memory map or threads list, in the order
/// the card draws them: the memory map's open modules with their sections
/// under them.
fn process_detail_rows(
    listing: &ProcessListing,
    detail: ProcessDetail,
    opener: Opener,
    folds: &Folds<'_>,
) -> Vec<DetailRow> {
    let Some(modules) = listing.modules.as_ref() else {
        return Vec::new();
    };
    match detail {
        // The keys pick among the modules instead (`process_keys`).
        ProcessDetail::Modules => Vec::new(),
        ProcessDetail::Threads => modules
            .threads
            .iter()
            .map(|thread| {
                DetailRow::leaf(RowOpen::Act(DebuggerAction::View(ViewContext::Thread {
                    eprocess: modules.eprocess,
                    tid: thread.tid,
                })))
            })
            .collect(),
        ProcessDetail::MemoryMap => {
            let Some(map) = modules.map.as_ref() else {
                return Vec::new();
            };
            let mut rows = Vec::new();
            for group in crate::debugger::memory_map::group(map) {
                match group {
                    crate::debugger::memory_map::MapGroup::Region(index) => {
                        rows.push(DetailRow::leaf(opener.open(map.regions[index].start)));
                    }
                    crate::debugger::memory_map::MapGroup::Module { base, regions, .. } => {
                        let module_row = rows.len();
                        let fold = Fold::MapModule(base);
                        rows.push(DetailRow {
                            open: RowOpen::Fold(fold),
                            branch: Branch::Parent(fold),
                        });
                        if folds.is_open(fold) {
                            for index in regions {
                                rows.push(DetailRow {
                                    open: opener.open(map.regions[index].start),
                                    branch: Branch::Under(module_row),
                                });
                            }
                        }
                    }
                }
            }
            rows
        }
    }
}

/// The memory map of the listing's chosen process, if it has been read.
fn map_of(listing: &ProcessListing) -> Option<&MemoryMap> {
    listing.modules.as_ref().and_then(|modules| modules.map.as_ref())
}

/// The right card's header: the Modules/Memory-map switch, the memory-map
/// minimap, and the column titles. A changed switch is reported through
/// `switch_to`; a minimap click through `minimap_click`.
fn right_card_header(
    ui: &mut egui::Ui,
    title: Title<'_>,
    detail: ProcessDetail,
    map: Option<&MemoryMap>,
    switch_to: &mut Option<ProcessDetail>,
    minimap_click: &mut Option<u64>,
) {
    let mut chosen = detail;
    // The title shares the switch's row (the card leaves it out).
    ui.horizontal_wrapped(|ui| {
        title_label(ui, title.text, title.count);
        if segmented(
            ui,
            &mut chosen,
            &[
                Segment { value: ProcessDetail::Modules, label: "Modules" },
                Segment { value: ProcessDetail::MemoryMap, label: "Memory map" },
                Segment { value: ProcessDetail::Threads, label: "Threads" },
            ],
        ) {
            *switch_to = Some(chosen);
        }
    });
    match detail {
        ProcessDetail::Modules => header_row(ui, 0.0, &module_columns()),
        ProcessDetail::Threads => header_row(ui, 0.0, &thread_columns()),
        ProcessDetail::MemoryMap => {
            if let Some(map) = map.filter(|map| !map.regions.is_empty()) {
                let origin = match map.origin {
                    crate::debugger::memory_map::MapOrigin::WindowsRecords => {
                        "from Windows' records"
                    }
                    crate::debugger::memory_map::MapOrigin::PageTables => "from page tables",
                };
                ui.label(RichText::new(origin).size(TEXT_SECONDARY).color(TEXT_MUTED));
                draw_minimap(ui, map, minimap_click);
            }
            header_row(ui, 0.0, &memory_columns());
        }
    }
}

/// The horizontal address-space overview: every region as a segment on a log
/// scale, executable memory cyan. A click jumps the list to that region.
fn draw_minimap(ui: &mut egui::Ui, map: &MemoryMap, minimap_click: &mut Option<u64>) {
    use crate::debugger::minimap;
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, crate::shell::theme::BG_WELL);
    // The lanes drawn are the lanes a click is matched against.
    for lane in minimap::lanes(map, width) {
        let segment = egui::Rect::from_min_size(
            egui::pos2(rect.left() + lane.offset, rect.top() + 2.0),
            egui::vec2(lane.extent, rect.height() - 4.0),
        );
        let color = if lane.executable {
            ACCENT_CYAN
        } else {
            CONTROL_FILL_REST
        };
        painter.rect_filled(segment, 0.0, color);
    }
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            *minimap_click = minimap::address_at(map, width, pos.x - rect.left());
        }
    }
    ui.add_space(2.0);
}

/// What the right card draws: which view of the chosen process, where its
/// addresses open, and the module the processor runs in (marked).
#[derive(Clone, Copy)]
struct RightCard<'a> {
    detail: ProcessDetail,
    listing: &'a ProcessListing,
    opener: Opener,
    highlight: Option<u64>,
    states: ModuleStates<'a>,
}

/// The right card's body: the modules (one picked into `picked_module`), the
/// memory map or the threads, the keys standing on row `key`.
fn right_card_body(
    ui: &mut egui::Ui,
    card: RightCard<'_>,
    picked_module: &mut Option<u64>,
    expanded: &mut BTreeSet<u64>,
    scroll_to: &mut Option<u64>,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    let RightCard {
        detail,
        listing,
        opener,
        highlight,
        states,
    } = card;
    match detail {
        ProcessDetail::Modules => {
            let click = ModuleClick {
                picked: picked_module,
                opener,
            };
            listed_modules(ui, listing, click, highlight, states, key, action);
        }
        ProcessDetail::Threads => match listing.modules.as_ref() {
            Some(modules) => thread_rows(ui, &modules.threads, modules.eprocess, key, action),
            None => {
                ui.add_space(SPACE_ITEM);
                ui.label(RichText::new("Pick a process.").color(TEXT_MUTED));
            }
        },
        ProcessDetail::MemoryMap => match listing.modules.as_ref().and_then(|m| m.map.as_ref()) {
            Some(map) => {
                memory_map_rows(ui, map, opener, expanded, scroll_to, key, action);
            }
            None => {
                ui.add_space(SPACE_ITEM);
                ui.label(RichText::new("Reading the address space…").color(TEXT_MUTED));
            }
        },
    }
}

/// A size in bytes, as a debugger shows it: whole KiB, or MiB/GiB to one
/// place. Guest regions are page-aligned, so KiB is exact.
pub(crate) fn human_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{} KB", bytes / KIB)
    } else {
        format!("{bytes} B")
    }
}

const MEMORY_SIZE_CHARS: usize = 8;
const MEMORY_RESIDENT_CHARS: usize = 9;
const MEMORY_ACCESS_CHARS: usize = 5;
const MEMORY_MODULE_CHARS: usize = 18;

const MEMORY_GAUGE_CHARS: usize = 7;
const MEMORY_TYPE_CHARS: usize = 7;

fn memory_columns() -> [Cell; 8] {
    [
        Cell::left("Address", TEXT_MUTED, 16),
        Cell::right("Size", TEXT_MUTED, MEMORY_SIZE_CHARS),
        Cell::right("Resident", TEXT_MUTED, MEMORY_RESIDENT_CHARS),
        Cell::left("", TEXT_MUTED, MEMORY_GAUGE_CHARS),
        Cell::left("Access", TEXT_MUTED, MEMORY_ACCESS_CHARS),
        Cell::left("Type", TEXT_MUTED, MEMORY_TYPE_CHARS),
        Cell::left("Module", TEXT_MUTED, MEMORY_MODULE_CHARS),
        Cell::fill("Section", TEXT_MUTED),
    ]
}

/// A region's Windows type, when its VAD was read.
fn vad_type_label(region: &crate::debugger::memory_map::MapRegion) -> &'static str {
    use crate::debugger::vad::VadKind;
    match region.vad.map(|info| info.kind) {
        Some(VadKind::Image) => "image",
        Some(VadKind::Mapped) => "mapped",
        Some(VadKind::Private) => "private",
        Some(VadKind::Unknown) => "unknown",
        None => "",
    }
}

/// How a region's access reads: the hardware access of its resident pages,
/// or — for a region with no resident page — Windows' protection from its VAD.
fn region_access_text(region: &crate::debugger::memory_map::MapRegion) -> String {
    match region.access {
        Some(access) => {
            let mut text = String::from("R");
            text.push(if access.writable { 'W' } else { ' ' });
            if access.executable {
                text.push('X');
            }
            text
        }
        None => region
            .vad
            .and_then(|info| info.protection)
            .map_or_else(|| "—".to_owned(), |protection| protection.label().to_owned()),
    }
}

/// The memory map, grouped by module: each module is one row that expands
/// into its header and section rows; private memory stays a plain row.
/// Executable memory is cyan — the one thing to hunt for.
fn memory_map_rows(
    ui: &mut egui::Ui,
    map: &MemoryMap,
    opener: Opener,
    expanded: &mut BTreeSet<u64>,
    scroll_to: &mut Option<u64>,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    if map.regions.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No mapped memory.").color(TEXT_MUTED));
        return;
    }
    // The rows drawn so far: the keys count modules and their open sections
    // alike, in the order `process_detail_rows` lists them.
    let mut row = 0usize;
    let sections = |ui: &mut egui::Ui,
                        regions: &[usize],
                        row: &mut usize,
                        scroll_to: &mut Option<u64>,
                        action: &mut Option<DebuggerAction>| {
        for &index in regions {
            let place = RowPlace { lead: TREE_INDENT, row: *row, key };
            region_row(ui, map, index, place, opener, scroll_to, action);
            *row += 1;
        }
    };
    for group in crate::debugger::memory_map::group(map) {
        match group {
            crate::debugger::memory_map::MapGroup::Region(index) => {
                let place = RowPlace { lead: 0.0, row, key };
                region_row(ui, map, index, place, opener, scroll_to, action);
                row += 1;
            }
            crate::debugger::memory_map::MapGroup::Module {
                base,
                name,
                regions,
                resident_pages,
                total_pages,
            } => {
                let open = expanded.contains(&base);
                let first = &map.regions[regions[0]];
                let module_row = row;
                row += 1;
                // A module row that is out of sight costs nothing but its
                // place in the list; its sections follow when it is open.
                if *scroll_to != Some(first.start) && culled(ui, key, module_row) {
                    skip_row(ui);
                    if open {
                        sections(ui, &regions, &mut row, scroll_to, action);
                    }
                    continue;
                }
                let any_exec = regions
                    .iter()
                    .any(|&i| map.regions[i].access.is_some_and(|a| a.executable));
                let last = &map.regions[*regions.last().unwrap_or(&regions[0])];
                let span = (last.start + last.size).saturating_sub(first.start);
                let fraction = resident_pages as f32 / total_pages.max(1) as f32;
                let gauge = if any_exec {
                    Cell::gauge_colored(fraction, MEMORY_GAUGE_CHARS, ACCENT_CYAN)
                } else {
                    Cell::gauge(fraction, MEMORY_GAUGE_CHARS)
                };
                let name_color = if any_exec { ACCENT_CYAN } else { TEXT_PRIMARY };
                let cells = [
                    Cell::left(format!("{:016X}", first.start), TEXT_MUTED, 16),
                    Cell::right(human_size(span), TEXT_MUTED, MEMORY_SIZE_CHARS),
                    Cell::right(
                        format!("{resident_pages}/{total_pages}"),
                        TEXT_MUTED,
                        MEMORY_RESIDENT_CHARS,
                    ),
                    gauge,
                    Cell::left("", TEXT_MUTED, MEMORY_ACCESS_CHARS),
                    Cell::left("image", TEXT_MUTED, MEMORY_TYPE_CHARS),
                    Cell::left(name.as_str(), name_color, MEMORY_MODULE_CHARS),
                    Cell::fill(
                        format!("{} sections", regions.len().saturating_sub(1)),
                        TEXT_MUTED,
                    ),
                ];
                let tone = if key.is_some_and(|key| key.at(module_row)) {
                    Tone::Selected
                } else {
                    Tone::Plain
                };
                let response =
                    list_row(ui, TREE_INDENT, &cells, tone, RowSense::Pick { selected: open });
                lead_chevron(ui, response.rect, open, name_color);
                follow_key(&response, key, module_row);
                if *scroll_to == Some(first.start) {
                    response.scroll_to_me(Some(egui::Align::TOP));
                    *scroll_to = None;
                }
                if response
                    .on_hover_text("Click to show this module's sections.")
                    .clicked()
                {
                    if open {
                        expanded.remove(&base);
                    } else {
                        expanded.insert(base);
                    }
                }
                // Its open state is read before the click above may change it,
                // as `process_detail_rows` read it.
                if open {
                    sections(ui, &regions, &mut row, scroll_to, action);
                }
            }
        }
    }
}

/// Where a memory-map row sits: `lead` points of indent (a section under its
/// module), row `row` of the map, and the row the keys stand on.
#[derive(Clone, Copy)]
struct RowPlace {
    lead: f32,
    row: usize,
    key: Option<KeyRow>,
}

/// One region row: its address, size, residency gauge, access, and owner,
/// placed at `place`. A click shows the region where `opener` shows it.
fn region_row(
    ui: &mut egui::Ui,
    map: &MemoryMap,
    index: usize,
    place: RowPlace,
    opener: Opener,
    scroll_to: &mut Option<u64>,
    action: &mut Option<DebuggerAction>,
) {
    let region = &map.regions[index];
    let RowPlace { lead, row, key } = place;
    // A row out of sight is left as room only, unless it is the scroll target
    // or the row a key just moved to.
    if *scroll_to != Some(region.start) && culled(ui, key, row) {
        skip_row(ui);
        return;
    }
    let executable = region.access.is_some_and(|a| a.executable);
    let observed = region.access.is_some_and(|a| a.observed);
    let access_text = region_access_text(region);
    let (module, section) = match &region.owner {
        RegionOwner::Header { module, .. } => (module.as_str(), "headers".to_owned()),
        RegionOwner::Section { module, section, .. } => (module.as_str(), section.clone()),
        RegionOwner::Stack { tid } => ("", format!("stack · thread {tid}")),
        RegionOwner::Private => ("", "private".to_owned()),
    };
    let access_color = if executable {
        ACCENT_CYAN
    } else if observed {
        TEXT_PRIMARY
    } else {
        TEXT_MUTED
    };
    let module_color = if executable { ACCENT_CYAN } else { TEXT_PRIMARY };
    let fraction = region.resident_pages as f32 / region.total_pages.max(1) as f32;
    let gauge = if executable {
        Cell::gauge_colored(fraction, MEMORY_GAUGE_CHARS, ACCENT_CYAN)
    } else {
        Cell::gauge(fraction, MEMORY_GAUGE_CHARS)
    };
    let cells = [
        Cell::left(format!("{:016X}", region.start), TEXT_MUTED, 16),
        Cell::right(human_size(region.size), TEXT_MUTED, MEMORY_SIZE_CHARS),
        Cell::right(
            format!("{}/{}", region.resident_pages, region.total_pages),
            TEXT_MUTED,
            MEMORY_RESIDENT_CHARS,
        ),
        gauge,
        Cell::left(access_text, access_color, MEMORY_ACCESS_CHARS),
        Cell::left(vad_type_label(region), TEXT_MUTED, MEMORY_TYPE_CHARS),
        Cell::left(module, module_color, MEMORY_MODULE_CHARS),
        Cell::fill(section, TEXT_MUTED),
    ];
    let tone = if key.is_some_and(|key| key.at(row)) {
        Tone::Selected
    } else {
        Tone::striped(index)
    };
    let response = list_row(ui, lead, &cells, tone, RowSense::Pick { selected: false });
    follow_key(&response, key, row);
    if *scroll_to == Some(region.start) {
        response.scroll_to_me(Some(egui::Align::TOP));
        *scroll_to = None;
    }
    click_opens(response, opener, Some(region.start), "", action);
}

/// How the Processes card orders its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessLayout {
    /// The kernel's order: creation order.
    List,
    /// Each process under the one that created it.
    Tree,
}

/// One row the Processes card draws: which process, and where in the tree.
struct ShownProcess {
    index: usize,
    depth: usize,
    guides: Vec<bool>,
    last: bool,
}

/// The rows the Processes card draws under `layout`, keeping those whose
/// name holds `filter` — and, in the tree (`tree`, read once with the
/// listing), their ancestors, so a match keeps the branch that leads to it.
fn process_rows_shown(
    processes: &[WinProcess],
    tree: &[TreeRow],
    layout: ProcessLayout,
    filter: &str,
) -> Vec<ShownProcess> {
    let filter = filter.to_lowercase();
    let matches = |process: &WinProcess| {
        filter.is_empty() || process.name.to_lowercase().contains(&filter)
    };
    match layout {
        ProcessLayout::List => processes
            .iter()
            .enumerate()
            .filter(|(_, process)| matches(process))
            .map(|(index, _)| ShownProcess {
                index,
                depth: 0,
                guides: Vec::new(),
                last: false,
            })
            .collect(),
        ProcessLayout::Tree => {
            let mut keep = vec![filter.is_empty(); processes.len()];
            if !filter.is_empty() {
                let by_pid: HashMap<u64, usize> = processes
                    .iter()
                    .enumerate()
                    .map(|(index, process)| (process.pid, index))
                    .collect();
                for (index, process) in processes.iter().enumerate() {
                    if !matches(process) {
                        continue;
                    }
                    // The match and every listed ancestor of it.
                    let mut at = Some(index);
                    let mut steps = 0;
                    while let Some(here) = at {
                        if keep[here] || steps > processes.len() {
                            break;
                        }
                        keep[here] = true;
                        steps += 1;
                        at = processes[here]
                            .parent
                            .and_then(|parent| by_pid.get(&parent).copied());
                    }
                }
            }
            tree.iter()
                .filter(|row| keep[row.index])
                .map(|row| ShownProcess {
                    index: row.index,
                    depth: row.depth,
                    guides: row.guides.clone(),
                    last: row.last,
                })
                .collect()
        }
    }
}

/// The share of the page's width the process list takes beside its modules.
const PROCESS_LIST_SHARE: f32 = 0.45;

/// The chosen process's modules, or why there are none to list.
fn listed_modules(
    ui: &mut egui::Ui,
    listing: &ProcessListing,
    click: ModuleClick<'_>,
    highlight: Option<u64>,
    states: ModuleStates<'_>,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    let muted = |ui: &mut egui::Ui, text: &str| {
        ui.label(RichText::new(text).color(TEXT_MUTED));
    };
    match listing.modules.as_ref().map(|modules| &modules.read) {
        Some(ModuleRead::Listed(modules)) => {
            let shown: Vec<&WinModule> = modules.iter().collect();
            module_rows(ui, &shown, click, highlight, states, key, action);
        }
        Some(ModuleRead::NoUserSpace) => {
            muted(ui, "A kernel process: it loads no user-mode modules.");
        }
        Some(ModuleRead::Unreadable) => {
            muted(ui, "Its loader data is paged out or could not be found.");
        }
        None => muted(ui, "Pick a process."),
    }
}

/// The Processes card's title, controls and column titles. The title shares
/// the controls row (the card leaves it out), so the list keeps a row.
fn process_header(ui: &mut egui::Ui, title: Title<'_>, layout: &mut ProcessLayout, filter: &mut String) {
    ui.horizontal_wrapped(|ui| {
        title_label(ui, title.text, title.count);
        segmented(
            ui,
            layout,
            &[Segment { value: ProcessLayout::List, label: "List" }, Segment { value: ProcessLayout::Tree, label: "Tree" }],
        );
        filter_field(ui, filter);
    });
    header_row(ui, PROCESS_LEAD, &process_columns("Name", "PID", "CR3"));
}

/// The room before a process's name: the running-process dot.
const PROCESS_LEAD: f32 = 14.0;
/// The widest PID column, in characters.
const PID_CHARS: usize = 6;
/// A CR3 column, in characters: the table address without leading zeros.
const CR3_CHARS: usize = 10;

fn process_columns(name: &str, pid: &str, cr3: &str) -> [Cell; 3] {
    [
        Cell::fill(name, TEXT_MUTED),
        Cell::right(pid, TEXT_MUTED, PID_CHARS),
        Cell::right(cr3, TEXT_MUTED, CR3_CHARS),
    ]
}

/// One row per shown process: the running one marked with a dot, the listed
/// one selected, the tree's guides in its lead. A row the keys just picked
/// (`key`) is scrolled into view.
fn process_rows(
    ui: &mut egui::Ui,
    processes: &[WinProcess],
    rows: &[ShownProcess],
    chosen: Option<&WinProcess>,
    selected_process: &mut Option<u64>,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    if rows.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No process matches the filter.").color(TEXT_MUTED));
        return;
    }
    for (n, shown) in rows.iter().enumerate() {
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        let process = &processes[shown.index];
        let selected = chosen.is_some_and(|c| c.eprocess == process.eprocess);
        let name_color = if process.current { ACCENT_CYAN } else { TEXT_PRIMARY };
        let lead = shown.depth as f32 * TREE_INDENT + PROCESS_LEAD;
        let cells = [
            Cell::fill(process.name.as_str(), name_color),
            Cell::right(process.pid.to_string(), TEXT_MUTED, PID_CHARS),
            Cell::right(format!("{:X}", process.cr3), TEXT_MUTED, CR3_CHARS),
        ];
        let tone = if selected {
            Tone::Selected
        } else {
            Tone::striped(n)
        };
        let response = list_row(ui, lead, &cells, tone, RowSense::Pick { selected });
        follow_key(&response, key, n);
        tree_guides(ui, response.rect, shown.depth, &shown.guides, shown.last);
        if process.current {
            lead_dot(ui, response.rect, lead, ACCENT_CYAN);
        }
        let parent = process
            .parent
            .map_or_else(|| "unknown".to_owned(), |parent| parent.to_string());
        let running = if process.current {
            "\nThe shown processor runs in this process's address space."
        } else {
            ""
        };
        let response = response.on_hover_text(format!(
            "{} — PID {}, parent {parent}\nEPROCESS {:016X}\nCR3 {:016X}{running}\n\
             Double-click to see the CPU tab from inside it.",
            process.name, process.pid, process.eprocess, process.cr3
        ));
        // A double-click's first click has already picked the row. egui calls
        // a double-click a triple when the click before last (anywhere) came
        // within 0.6 s, so a triple opens the row too. The running process has
        // no state but its live one: it opens the live view.
        if response.double_clicked() || response.triple_clicked() {
            let context = if process.current {
                ViewContext::Live
            } else {
                ViewContext::Process {
                    eprocess: process.eprocess,
                }
            };
            *action = Some(DebuggerAction::View(context));
        } else if response.clicked() && !selected {
            // Asked of the run once the frame is drawn (`processes_page`).
            *selected_process = Some(process.eprocess);
        }
    }
}

/// A module table's columns: base, size, name, what the symbol engine has,
/// and the path in the room left.
fn module_columns() -> [Cell; 5] {
    [
        Cell::left("Base", TEXT_MUTED, 16),
        Cell::right("Size", TEXT_MUTED, MODULE_SIZE_CHARS),
        Cell::left("Name", TEXT_MUTED, MODULE_NAME_CHARS),
        Cell::left("Symbols", TEXT_MUTED, SYMBOL_STATE_CHARS),
        Cell::fill("Path", TEXT_MUTED),
    ]
}

const MODULE_SIZE_CHARS: usize = 8;
const MODULE_NAME_CHARS: usize = 18;

/// What a click on a module row does: it picks the module into `picked`, for
/// its detail; a double-click (or a triple, which holds one) shows it where
/// `opener` shows it.
struct ModuleClick<'a> {
    picked: &'a mut Option<u64>,
    opener: Opener,
}

/// One row per module, with what the symbol engine has for it; the picked
/// one and the row the keys stand on selected, the one the processor runs in
/// marked.
fn module_rows(
    ui: &mut egui::Ui,
    modules: &[&WinModule],
    click: ModuleClick<'_>,
    highlight: Option<u64>,
    states: ModuleStates<'_>,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    if modules.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No module matches the filter.").color(TEXT_MUTED));
        return;
    }
    for (n, module) in modules.iter().enumerate() {
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        let base_name = module.name.as_deref().unwrap_or("(paged out)");
        // A 32-bit module (a WOW64 process's) is tagged beside its name.
        let name = match module.bits {
            ModuleBits::Bits32 => format!("{base_name}  · 32"),
            ModuleBits::Bits64 => base_name.to_owned(),
        };
        let path = module.path.as_deref().unwrap_or("(paged out)");
        // The module the shown processor is executing in.
        let current = highlight == Some(module.base);
        let name_color = if current { ACCENT_CYAN } else { TEXT_PRIMARY };
        let cells = [
            Cell::left(format!("{:016X}", module.base), TEXT_MUTED, 16),
            Cell::right(format!("{:X}", module.size), TEXT_MUTED, MODULE_SIZE_CHARS),
            Cell::left(name, name_color, MODULE_NAME_CHARS),
            states.cell(module.base),
            Cell::fill(path, TEXT_MUTED.gamma_multiply(0.8)),
        ];
        let picked_now = *click.picked == Some(module.base) || key.is_some_and(|key| key.at(n));
        let tone = if picked_now {
            Tone::Selected
        } else if current {
            Tone::Current
        } else {
            Tone::striped(n)
        };
        let response = list_row(
            ui,
            0.0,
            &cells,
            tone,
            RowSense::Pick {
                selected: picked_now || current,
            },
        );
        follow_key(&response, key, n);
        let opens = click.opener.open(module.base);
        let then = match opens {
            RowOpen::Act(_) => "double-click to show it in memory.",
            RowOpen::Fold(_) | RowOpen::Nothing => ANOTHER_ADDRESS_SPACE,
        };
        let response = response.on_hover_text(format!(
            "{path}\nClick to see its sections, exports and imports; {then}"
        ));
        response.context_menu(|ui| {
            if ui.button(REINDEX_MODULE).clicked() {
                *action = Some(DebuggerAction::Reindex(vec![module.base]));
                ui.close();
            }
        });
        if response.double_clicked() || response.triple_clicked() {
            match opens {
                RowOpen::Act(open) => *action = Some(open),
                RowOpen::Fold(_) | RowOpen::Nothing => {}
            }
        } else if response.clicked() {
            *click.picked = Some(module.base);
        }
    }
}

const THREAD_TID_CHARS: usize = 7;
const THREAD_START_CHARS: usize = 16;
const THREAD_STACK_CHARS: usize = 29;

fn thread_columns() -> [Cell; 4] {
    [
        Cell::right("TID", TEXT_MUTED, THREAD_TID_CHARS),
        Cell::left("Start", TEXT_MUTED, THREAD_START_CHARS),
        Cell::left("Stack", TEXT_MUTED, THREAD_STACK_CHARS),
        Cell::fill("Fibres", TEXT_MUTED),
    ]
}

/// One row per thread of the process at `eprocess`: its ID, start address,
/// user stack, and a mark for a thread that runs fibres. Double-clicking one
/// shows the CPU tab from inside it; the keys stand on row `key`.
fn thread_rows(
    ui: &mut egui::Ui,
    threads: &[WinThread],
    eprocess: u64,
    key: Option<KeyRow>,
    action: &mut Option<DebuggerAction>,
) {
    if threads.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No threads listed.").color(TEXT_MUTED));
        return;
    }
    for (n, thread) in threads.iter().enumerate() {
        if culled(ui, key, n) {
            skip_row(ui);
            continue;
        }
        let start = if thread.start == 0 {
            String::new()
        } else {
            format!("{:016X}", thread.start)
        };
        let stack = thread
            .stack
            .map_or_else(String::new, |stack| format!("{:X}–{:X}", stack.start, stack.end));
        let tail = if thread.uses_fibers { "fibres" } else { "" };
        let tail_color = if thread.uses_fibers { ACCENT_AMBER } else { TEXT_MUTED };
        let cells = [
            Cell::right(thread.tid.to_string(), TEXT_PRIMARY, THREAD_TID_CHARS),
            Cell::left(start, TEXT_MUTED, THREAD_START_CHARS),
            Cell::left(stack, TEXT_MUTED, THREAD_STACK_CHARS),
            Cell::fill(tail, tail_color),
        ];
        let response = list_row(ui, 0.0, &cells, keyed_tone(key, n), RowSense::Pick { selected: false });
        follow_key(&response, key, n);
        let response = response.on_hover_text("Double-click to see the CPU tab from inside this thread.");
        // A triple holds the double-click (see `process_rows`).
        if response.double_clicked() || response.triple_clicked() {
            *action = Some(DebuggerAction::View(ViewContext::Thread {
                eprocess,
                tid: thread.tid,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u64, parent: Option<u64>, name: &str) -> WinProcess {
        WinProcess {
            eprocess: pid,
            pid,
            parent,
            name: name.to_owned(),
            cr3: 0,
            current: false,
        }
    }

    /// A filtered tree keeps each match and the branch above it; a filtered
    /// list keeps the matches alone.
    #[test]
    fn a_filter_keeps_the_branch_that_leads_to_a_match() {
        let processes = [
            process(4, None, "System"),
            process(228, Some(4), "smss.exe"),
            process(276, Some(228), "csrss.exe"),
            process(300, Some(228), "winlogon.exe"),
            process(348, Some(300), "services.exe"),
            process(1400, Some(348), "wmiprvse.exe"),
        ];
        let tree = crate::debugger::windows::process_tree(&processes);
        let names = |rows: Vec<ShownProcess>| -> Vec<&str> {
            rows.iter().map(|row| processes[row.index].name.as_str()).collect()
        };
        assert_eq!(
            names(process_rows_shown(&processes, &tree, ProcessLayout::Tree, "WMI")),
            ["System", "smss.exe", "winlogon.exe", "services.exe", "wmiprvse.exe"]
        );
        assert_eq!(
            names(process_rows_shown(&processes, &tree, ProcessLayout::List, "wmi")),
            ["wmiprvse.exe"]
        );
        assert_eq!(
            process_rows_shown(&processes, &tree, ProcessLayout::Tree, "").len(),
            processes.len()
        );
    }

    /// The layout follows the room measured, whatever hosts the window: the
    /// same size gives the same layout on a phone and on a desktop.
    #[test]
    fn the_layout_is_decided_by_the_room_the_page_has() {
        // A desktop window: wide and tall enough for four useful panes.
        assert_eq!(Fit::of(egui::vec2(1400.0, 900.0)), Fit::Grid);
        assert_eq!(Fit::of(egui::vec2(GRID_MIN_WIDTH, GRID_MIN_HEIGHT)), Fit::Grid);
        // A landscape phone: wide, but short — the grid would squeeze every
        // pane to a few rows, so the page scrolls in two columns.
        assert_eq!(Fit::of(egui::vec2(1000.0, 330.0)), Fit::Scrolling(Columns::Two));
        // The same short height on a wide desktop window gets the same.
        assert_eq!(Fit::of(egui::vec2(1800.0, 330.0)), Fit::Scrolling(Columns::Two));
        // A portrait phone, and a tall but narrow window.
        assert_eq!(Fit::of(egui::vec2(400.0, 800.0)), Fit::Scrolling(Columns::One));
        assert_eq!(Fit::of(egui::vec2(700.0, 1400.0)), Fit::Scrolling(Columns::One));
        // One point short of either threshold leaves the grid.
        assert_ne!(Fit::of(egui::vec2(GRID_MIN_WIDTH - 1.0, GRID_MIN_HEIGHT)), Fit::Grid);
        assert_ne!(Fit::of(egui::vec2(GRID_MIN_WIDTH, GRID_MIN_HEIGHT - 1.0)), Fit::Grid);
    }

    /// A narrow pane gives the bytes up before it gives up the opcode: the
    /// listing shows fewer bytes, never fewer than two, and all eight where
    /// the pane has the room.
    #[test]
    fn a_narrow_code_pane_shows_fewer_bytes_to_keep_the_opcode() {
        assert_eq!(code_bytes_shown(120.0, 16), CODE_BYTES_SHOWN);
        // 16 digits, 4 of gaps, 16 for the opcode, 1 for the ellipsis: the
        // 45th character is the first byte's third.
        assert_eq!(code_bytes_shown(45.0, 16), 2);
        assert_eq!(code_bytes_shown(51.0, 16), 4);
        assert_eq!(code_bytes_shown(10.0, 16), CODE_BYTES_MIN);
        // A 32-bit address is eight characters shorter: three more bytes
        // fit in a pane of the same width.
        assert_eq!(code_bytes_shown(51.0, 8), 7);
    }

    /// A live snapshot of a long-mode machine, and one looking from a thread
    /// whose saved code segment is `cs`, in the address space at 0x3000.
    fn snapshots(cs: u16) -> (DebugSnapshot, DebugSnapshot) {
        use crate::debugger::inspect::{read_snapshot, ViewRequest};
        use crate::debugger::threads::{decode_trap_frame, TRAP_FRAME_BYTES};
        use crate::debugger::windows::{ThreadView, WindowsDiscovery};
        use rusty_box::cpu::CpuSetupMode;
        use rusty_box::emulator::{Emulator, EmulatorConfig};
        let mut machine =
            Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)
                .expect("machine");
        let read = |machine: &mut Emulator| {
            read_snapshot(machine, &ViewRequest::default(), &[], &mut WindowsDiscovery::default(), None, None)
        };
        let live = read(&mut machine);
        let mut viewed = read(&mut machine);
        let mut saved = decode_trap_frame(&[0u8; TRAP_FRAME_BYTES]);
        saved.cs = cs;
        viewed.viewing = Viewing::Thread(ThreadView {
            process: "winlogon.exe".to_owned(),
            pid: 0x1E0,
            tid: 0x1F4,
            cr3: 0x3000,
            saved,
            symbols: crate::debugger::inspect::SymbolIndex::default(),
        });
        (live, viewed)
    }

    /// The code and stack panes are as wide as the code they show: a viewed
    /// thread's own mode, not the live processor's.
    #[test]
    fn a_viewed_threads_panes_take_its_own_code_width() {
        let (live, mut viewed) = snapshots(0x33);
        assert_eq!(pane_code_size(&live), CodeSize::Bits64);
        viewed.cpus[0].code_size = CodeSize::Bits32;
        assert_eq!(pane_code_size(&viewed), CodeSize::Bits64, "a 64-bit thread");
        let (_, wow64) = snapshots(WOW64_CODE_SELECTOR);
        assert_eq!(pane_code_size(&wow64), CodeSize::Bits32, "a WOW64 thread in 32-bit code");
    }

    /// Addresses are named against the viewed process's modules only in its
    /// own address space, and against the live process's only in the live one.
    #[test]
    fn addresses_are_named_only_in_the_space_their_modules_belong_to() {
        let (live, viewed) = snapshots(0x33);
        assert_eq!(vantage_space(&live), AddressSpace::Linear);
        assert_eq!(vantage_space(&viewed), AddressSpace::Process { cr3: 0x3000 });
    }

    /// A drag moves a split by its share of the room the two panes have, and
    /// never past the point where either pane would be smaller than `min`.
    #[test]
    fn a_dragged_split_moves_by_the_drag_and_keeps_both_panes_their_minimum() {
        // 1008 points, an 8-point gap: 1000 points for the panes.
        let moved = resized(0.5, 100.0, 1008.0, 8.0, 96.0);
        assert!((moved - 0.6).abs() < 1e-4, "{moved}");
        assert!((resized(0.9, 500.0, 1008.0, 8.0, 96.0) - 0.904).abs() < 1e-4);
        assert!((resized(0.1, -500.0, 1008.0, 8.0, 96.0) - 0.096).abs() < 1e-4);
        // Too little room for two minimums: the split sits in the middle.
        let squeezed = resized(0.8, 50.0, 100.0, 8.0, 96.0);
        assert!((squeezed - 0.5).abs() < 1e-4, "{squeezed}");
    }

    /// A wide memory pane shows all sixteen bytes a line; a narrow one folds
    /// to eight so the dump fits without a sideways swipe.
    #[test]
    fn a_narrow_memory_pane_folds_its_rows_to_eight_bytes() {
        assert_eq!(memory_bytes_per_line(120.0), ROW_BYTES);
        assert_eq!(memory_bytes_per_line(19.0 + 4.0 * ROW_BYTES as f32), ROW_BYTES);
        assert_eq!(memory_bytes_per_line(60.0), ROW_BYTES / 2);
        assert_eq!(memory_bytes_per_line(10.0), ROW_BYTES / 2);
    }

    #[test]
    fn a_page_that_scrolls_packs_tighter_than_one_that_fills() {
        let roomy = Fit::Grid.density();
        let compact = Fit::Scrolling(Columns::Two).density();
        assert_eq!(compact, Fit::Scrolling(Columns::One).density());
        assert!(compact.page_margin() < roomy.page_margin());
        assert!(compact.gap() < roomy.gap());
        assert!(compact.title_gap() < roomy.title_gap());
        assert!(compact.button_padding().y < roomy.button_padding().y);
    }

    #[test]
    fn a_filter_matches_a_name_in_either_case() {
        assert!(holds_ignoring_case("KeWaitForSingleObject", "wait"));
        assert!(holds_ignoring_case("KERNELBASE.dll", "kernelbase"));
        assert!(!holds_ignoring_case("NtClose", "wait"));
        assert!(!holds_ignoring_case("Nt", "ntclose"), "a filter longer than the name");
        assert!(holds_ignoring_case("NtClose", ""), "no filter keeps everything");
    }

    #[test]
    fn the_keys_walk_a_listing_and_stay_inside_it() {
        let keys = |step: isize, open: bool| ListKeys {
            step,
            open,
            sideways: 0,
            switches: 0,
            next_views: 0,
        };
        // Standing on no row is standing just above the first.
        assert_eq!(walk(keys(1, false), None, 5).key, Some(KeyRow::Moved(0)));
        assert_eq!(walk(keys(3, false), None, 5).key, Some(KeyRow::Moved(2)));
        assert_eq!(walk(keys(-1, false), None, 5).key, Some(KeyRow::Moved(0)));
        // Both ends hold.
        assert_eq!(walk(keys(-4, false), Some(2), 5).key, Some(KeyRow::Moved(0)));
        assert_eq!(walk(keys(9, false), Some(2), 5).key, Some(KeyRow::Moved(4)));
        // A row past a listing that shrank is no row; no step leaves it so.
        assert_eq!(walk(keys(0, false), Some(7), 5).key, None);
        assert_eq!(walk(keys(0, false), Some(3), 5).key, Some(KeyRow::Resting(3)));
        // Enter opens the row stood on after the move, and nothing in an
        // empty listing.
        assert_eq!(walk(keys(1, true), Some(3), 5).open, Some(4));
        assert_eq!(walk(keys(0, true), None, 5).open, None);
        assert_eq!(walk(keys(1, true), None, 0).key, None);
        assert_eq!(walk(keys(1, true), None, 0).open, None);
    }

    #[test]
    fn typed_addresses_are_read_as_x64dbg_writes_them() {
        assert_eq!(parse_address("7c00"), Ok(0x7C00));
        assert_eq!(
            parse_address("  0xFFFFF800`00001000 "),
            Ok(0xFFFF_F800_0000_1000)
        );
        assert_eq!(parse_address("0X1_0000"), Ok(0x1_0000));
        assert_eq!(parse_address(""), Err(AddressError::Empty));
        assert_eq!(parse_address("0x"), Err(AddressError::Empty));
        assert_eq!(parse_address("rip"), Err(AddressError::NotHex));
        assert_eq!(
            parse_address("1_0000_0000_0000_0000"),
            Err(AddressError::TooLong)
        );
    }
}
