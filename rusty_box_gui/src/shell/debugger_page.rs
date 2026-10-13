//! The Debugger page: a paused VM's processors, code, memory and stack, laid
//! out as x64dbg lays out its CPU tab — a fixed grid of code and registers
//! above memory and stack, each pane scrolling on its own — and stacked in
//! one scrolling column where the page is too narrow for the grid. What the
//! last step (or pause) changed is drawn red; the memory pane searches for
//! hex, text or UTF-16. Like the VM bar, it draws from borrowed state and
//! reports what was asked for; the app acts on it.

use egui::text::{LayoutJob, TextFormat};
use egui::RichText;
use rusty_box::cpu::{CodeSize, CpuDebugView, ExecutionMode, ProcessorActivity};

use crate::debugger::control::DebugNote;
use crate::debugger::inspect::{
    AddressSpace, CodeText, DebugSnapshot, GuestAddress, MemoryTarget, MEMORY_WINDOW_BYTES,
    ROW_BYTES,
};
use crate::debugger::search::{
    pattern_of, FindOutcome, FindRequest, FoundAt, PatternError, SearchEncoding,
};
use crate::debugger::windows::{
    process_tree, KernelMiss, ModuleRead, ProcessListing, SystemListing, SystemView, WinModule,
    WinProcess,
};
use crate::shell::listing::{
    card_title, chip, empty_state, header_row, lead_arrow, lead_dot, list_row,
    mono_font, segmented, tree_guides, Cell, RowSense, Tone, TREE_INDENT,
};
use crate::shell::theme::{
    shell_card_frame, ACCENT_AMBER, ACCENT_BLUE, ACCENT_CYAN, ACCENT_RED, SPACE_ITEM, SPACE_PAGE,
    TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};

pub(crate) const NOT_RUNNING: &str =
    "Power the VM on and pause it to see its processors, code and memory.";
const PAUSE_TO_INSPECT: &str = "Pause the VM to see its processors, code and memory.";
const SHOWING_LAST_PAUSE: &str = "Running: showing the state at the last pause.";
const STEP_NEEDS_INTERPRETER: &str =
    "Step needs the interpreter: a VM on the hypervisor retires instructions nothing counts.";
/// The accessible name of the memory pane's address field.
pub(crate) const MEMORY_ADDRESS: &str = "Memory address";
/// The accessible name of the memory pane's search field.
pub(crate) const FIND: &str = "Find";
/// What a step changed is drawn in this colour, as x64dbg does.
const CHANGED: egui::Color32 = ACCENT_RED;
/// The page width from which the panes sit in two columns, points.
const TWO_COLUMN_WIDTH: f32 = 900.0;

/// What a typed address could not be read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddressError {
    Empty,
    NotHex,
    TooLong,
}

impl AddressError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Empty => "Type an address.",
            Self::NotHex => "An address is hexadecimal digits, with or without 0x.",
            Self::TooLong => "An address has at most 16 hexadecimal digits.",
        }
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
    /// The guest kernel's drivers.
    KernelModules,
}

impl DebuggerTab {
    const ALL: [Self; 3] = [Self::Cpu, Self::Processes, Self::KernelModules];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Processes => "Processes",
            Self::KernelModules => "Kernel modules",
        }
    }

    /// What this tab asks the run thread to list.
    fn system_view(self, selected_process: Option<u64>) -> SystemView {
        match self {
            Self::Cpu => SystemView::Off,
            Self::Processes => SystemView::Processes {
                selected: selected_process,
            },
            Self::KernelModules => SystemView::KernelModules,
        }
    }
}

pub(crate) struct DebuggerPageState<'a> {
    pub(crate) live: bool,
    pub(crate) paused: bool,
    pub(crate) step_available: bool,
    pub(crate) dump_pending: bool,
    pub(crate) note: Option<DebugNote>,
    pub(crate) snapshot: Option<&'a DebugSnapshot>,
    /// The snapshot before `snapshot`: what is drawn red changed since it.
    pub(crate) previous: Option<&'a DebugSnapshot>,
    pub(crate) find: Option<FindOutcome>,
    pub(crate) searching: bool,
    pub(crate) pane: &'a mut DebuggerPane,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DebuggerAction {
    Step,
    DumpRam,
    ShowMemory(MemoryTarget),
    SelectCpu(usize),
    Find(FindRequest),
    /// List these beside the processor panes (or nothing).
    ShowSystem(SystemView),
    /// Show the CPU tab with the memory pane at this linear address: a
    /// module picked from a list.
    OpenModule(u64),
}

/// What the panes compare against and mark: the snapshot before, and the
/// last search hit.
#[derive(Clone, Copy)]
struct Marks<'a> {
    previous: Option<&'a DebugSnapshot>,
    find: Option<FindOutcome>,
    searching: bool,
}

/// What the toolbar draws from.
struct Toolbar {
    paused: bool,
    step_available: bool,
    dump_pending: bool,
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
        dump_pending,
        note,
        snapshot,
        previous,
        find,
        searching,
        pane,
    } = state;
    let marks = Marks {
        previous,
        find,
        searching,
    };
    egui::Frame::new()
        .inner_margin(egui::Margin::same(SPACE_PAGE))
        .show(ui, |ui| {
            if !live {
                empty_state(ui, "Not running", NOT_RUNNING);
                return;
            }
            let bar = Toolbar {
                paused,
                step_available,
                dump_pending,
            };
            toolbar(ui, &bar, snapshot, pane, &mut action);
            if let Some(DebugNote::StepNeedsInterpreter) = note {
                ui.label(RichText::new(STEP_NEEDS_INTERPRETER).color(ACCENT_AMBER));
            }
            let Some(snapshot) = snapshot else {
                empty_state(ui, "Running", PAUSE_TO_INSPECT);
                return;
            };
            status_chips(ui, snapshot, paused);
            follow_window(pane, snapshot);
            ui.add_space(SPACE_ITEM);
            let wide = ui.available_width() >= TWO_COLUMN_WIDTH;
            match pane.tab {
                DebuggerTab::Cpu if wide => grid_layout(ui, snapshot, marks, pane, &mut action),
                DebuggerTab::Cpu => stacked_layout(ui, snapshot, marks, pane, &mut action),
                DebuggerTab::Processes | DebuggerTab::KernelModules => {
                    system_page(ui, wide, snapshot, pane, &mut action);
                }
            }
        });
    action
}

/// The shown processor's state as a row of chips: its privilege — user mode
/// (CPL 3) or kernel mode (CPL 0) — its mode and activity, the Windows
/// process it runs for when the guest's structures name one, its address
/// space, and whether the page shows a pause that has since run on.
fn status_chips(ui: &mut egui::Ui, snapshot: &DebugSnapshot, paused: bool) {
    let Some(view) = shown_view(snapshot) else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (text, color) = match view.registers.cpl {
            0 => ("Kernel mode", ACCENT_AMBER),
            3 => ("User mode", ACCENT_CYAN),
            _ => ("Privileged", ACCENT_AMBER),
        };
        chip(ui, &format!("{text} · CPL {}", view.registers.cpl), color);
        chip(ui, mode_label(view), ACCENT_BLUE);
        chip(ui, activity_label(view.activity), TEXT_MUTED);
        if let Some(path) = &snapshot.process {
            let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
            chip(ui, &format!("Process {name}"), TEXT_PRIMARY).on_hover_text(path.as_str());
        }
        chip(ui, &format!("CR3 {:X}", view.registers.cr3), TEXT_MUTED)
            .on_hover_text("The address space: each process has its own page tables.");
        if !paused {
            chip(ui, SHOWING_LAST_PAUSE, ACCENT_AMBER);
        }
    });
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

fn toolbar(
    ui: &mut egui::Ui,
    bar: &Toolbar,
    snapshot: Option<&DebugSnapshot>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    ui.horizontal(|ui| {
        let step = ui.add_enabled(bar.paused && bar.step_available, egui::Button::new("Step"));
        let step = if bar.step_available {
            step
        } else {
            step.on_disabled_hover_text(STEP_NEEDS_INTERPRETER)
        };
        if step.clicked() {
            *action = Some(DebuggerAction::Step);
        }
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
        ui.add_space(SPACE_ITEM);
        let tabs = DebuggerTab::ALL.map(|tab| (tab, tab.label()));
        if segmented(ui, &mut pane.tab, &tabs) {
            *action = Some(DebuggerAction::ShowSystem(
                pane.tab.system_view(pane.selected_process),
            ));
        }
        let Some(snapshot) = snapshot else {
            return;
        };
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
    });
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
    let left = ((page.x - gap) * LEFT_SHARE).floor();
    let right = page.x - gap - left;
    let top = ((page.y - gap) * TOP_SHARE).floor();
    let bottom = page.y - gap - top;
    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
    ui.horizontal_top(|ui| {
        fixed_card(
            ui,
            egui::vec2(left, top),
            Title::plain("Code"),
            |_| {},
            |ui| code_rows(ui, snapshot),
        );
        fixed_card(
            ui,
            egui::vec2(right, top),
            Title::plain("Registers"),
            |_| {},
            |ui| register_rows(ui, snapshot, marks),
        );
    });
    ui.horizontal_top(|ui| {
        fixed_card(
            ui,
            egui::vec2(left, bottom),
            Title::plain("Memory"),
            |ui| memory_bar(ui, snapshot, marks, pane, action),
            |ui| memory_rows(ui, snapshot, marks),
        );
        fixed_card(
            ui,
            egui::vec2(right, bottom),
            Title::plain("Stack"),
            |_| {},
            |ui| stack_rows(ui, snapshot, marks),
        );
    });
}

/// The page where it is too narrow for the grid (a phone): the panes one
/// under the other, the page scrolling as a whole.
fn stacked_layout(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    marks: Marks<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let gap = SPACE_ITEM;
            card(ui, Title::plain("Registers"), |_| {}, |ui| {
                register_rows(ui, snapshot, marks);
            });
            ui.add_space(gap);
            card(ui, Title::plain("Code"), |_| {}, |ui| code_rows(ui, snapshot));
            ui.add_space(gap);
            card(ui, Title::plain("Stack"), |_| {}, |ui| {
                stack_rows(ui, snapshot, marks);
            });
            ui.add_space(gap);
            card(
                ui,
                Title::plain("Memory"),
                |ui| memory_bar(ui, snapshot, marks, pane, action),
                |ui| memory_rows(ui, snapshot, marks),
            );
        });
}

/// A pane's title: its name, and how many entries it lists when it lists.
#[derive(Clone, Copy)]
struct Title<'a> {
    text: &'a str,
    count: Option<usize>,
}

impl<'a> Title<'a> {
    const fn plain(text: &'a str) -> Self {
        Self { text, count: None }
    }

    const fn counted(text: &'a str, count: usize) -> Self {
        Self {
            text,
            count: Some(count),
        }
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
            card_title(ui, title.text, title.count);
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

/// A pane as tall as its content, the page's full width.
fn card<H, B>(ui: &mut egui::Ui, title: Title<'_>, header: H, body: B)
where
    H: FnOnce(&mut egui::Ui),
    B: FnOnce(&mut egui::Ui),
{
    shell_card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        card_title(ui, title.text, title.count);
        header(ui);
        ui.spacing_mut().item_spacing.y = 0.0;
        body(ui);
    });
}

/// The room the code listing gives an instruction's bytes, in characters:
/// `CODE_BYTES_SHOWN` bytes and an ellipsis.
const CODE_BYTES_CHARS: usize = 3 * CODE_BYTES_SHOWN + 1;
/// The lead before a listing's first column, where the RIP marker sits.
const MARKER_LEAD: f32 = 12.0;

/// The code at RIP, one instruction a row: address, bytes, opcode. The row at
/// RIP is shaded and marked.
fn code_rows(ui: &mut egui::Ui, snapshot: &DebugSnapshot) {
    let digits = shown_view(snapshot).map_or(16, |view| hex_digits(view.code_size));
    for (index, line) in snapshot.code.iter().enumerate() {
        let mut bytes: Vec<String> = line
            .bytes
            .iter()
            .take(CODE_BYTES_SHOWN)
            .map(|byte| format!("{byte:02X}"))
            .collect();
        if line.bytes.len() > CODE_BYTES_SHOWN {
            bytes.push("…".to_owned());
        }
        let (text, text_color) = match &line.text {
            CodeText::Opcode(name) => (name.as_str(), TEXT_PRIMARY),
            CodeText::Undecodable => ("(undecodable)", TEXT_MUTED),
            CodeText::Unreadable => ("(unreadable)", TEXT_MUTED),
        };
        // The first line is the instruction at RIP.
        let at_rip = index == 0;
        let (address_color, tone) = if at_rip {
            (ACCENT_CYAN, Tone::Current)
        } else {
            (TEXT_MUTED, Tone::Plain)
        };
        let cells = [
            Cell::left(format!("{:0digits$X}", line.address), address_color, digits),
            Cell::left(bytes.join(" "), TEXT_MUTED, CODE_BYTES_CHARS),
            Cell::fill(text, if at_rip { ACCENT_CYAN } else { text_color }),
        ];
        let response = list_row(ui, MARKER_LEAD, &cells, tone, RowSense::Look);
        if at_rip {
            lead_arrow(ui, response.rect, ACCENT_CYAN);
        }
    }
}

fn register_rows(ui: &mut egui::Ui, snapshot: &DebugSnapshot, marks: Marks<'_>) {
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
            segmented(
                ui,
                &mut pane.space,
                &[
                    (AddressSpace::Linear, "Linear"),
                    (AddressSpace::Physical, "Physical"),
                ],
            );
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
                match parse_address(&pane.address_text) {
                    Ok(value) => {
                        pane.address_error = None;
                        *action = Some(DebuggerAction::ShowMemory(MemoryTarget::At(
                            GuestAddress {
                                space: pane.space,
                                value,
                            },
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
                (SearchEncoding::Hex, "Hex"),
                (SearchEncoding::Text, "Text"),
                (SearchEncoding::Utf16, "UTF-16"),
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

/// The memory window: address, sixteen bytes and their ASCII per row. A byte
/// the last step changed is red; the bytes of the last search hit amber.
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
    // Sixteen bytes, each two digits and a space but the last.
    const HEX_CHARS: usize = 3 * ROW_BYTES - 1;
    for (index, line) in snapshot.memory.rows.iter().enumerate() {
        let address = Cell::left(format!("{:016X}", line.address), TEXT_MUTED, 16);
        let tone = Tone::striped(index);
        let Some(bytes) = line.bytes else {
            let unreadable = ["??"; ROW_BYTES].join(" ");
            let cells = [address, Cell::left(unreadable, TEXT_MUTED.gamma_multiply(0.6), HEX_CHARS)];
            list_row(ui, 0.0, &cells, tone, RowSense::Look);
            continue;
        };
        let old = before.and_then(|rows| rows.get(index)).and_then(|old| old.bytes);
        let mut hex = LayoutJob::default();
        for (column, byte) in bytes.iter().enumerate() {
            let at = line.address.wrapping_add(column as u64);
            let color = if old.is_some_and(|old| old[column] != *byte) {
                CHANGED
            } else if hit.is_some_and(|hit| in_hit(hit, at)) {
                ACCENT_AMBER
            } else if *byte == 0 {
                // Zero bytes recede, so the data in a page stands out.
                TEXT_MUTED.gamma_multiply(0.7)
            } else {
                TEXT_PRIMARY
            };
            let separator = if column == 0 { "" } else { " " };
            hex.append(
                &format!("{separator}{byte:02X}"),
                0.0,
                TextFormat::simple(font.clone(), color),
            );
        }
        let ascii: String = bytes
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
            Cell::job(hex, HEX_CHARS),
            Cell::left(ascii, ACCENT_BLUE, ROW_BYTES),
        ];
        list_row(ui, 0.0, &cells, tone, RowSense::Look);
    }
}

fn in_hit(hit: FoundAt, address: u64) -> bool {
    hit.at.value <= address && address < hit.at.value.wrapping_add(hit.len)
}

/// The stack from RSP up: the slot RSP points at in cyan, a slot the last
/// step changed in red.
fn stack_rows(ui: &mut egui::Ui, snapshot: &DebugSnapshot, marks: Marks<'_>) {
    let size = shown_view(snapshot).map(|view| view.code_size);
    let digits = size.map_or(16, hex_digits);
    // A slot is as wide as a pointer at the code size: 2, 4 or 8 bytes.
    let value_digits = size.map_or(16, |size| 2 * crate::debugger::inspect::slot_bytes(size));
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
        let cells = [
            Cell::left(format!("{:0digits$X}", slot.address), address_color, digits),
            Cell::left(value, value_color, value_digits),
        ];
        let response = list_row(ui, MARKER_LEAD, &cells, tone, RowSense::Look);
        if top {
            lead_arrow(ui, response.rect, ACCENT_CYAN);
        }
    }
}

/// Said while the run thread has not yet listed what a tab asks for.
const READING_LIST: &str = "Reading the guest's lists…";
/// Why the lists are empty: no Windows x64 kernel, and the step that said so.
pub(crate) fn no_windows_kernel(miss: KernelMiss) -> String {
    format!(
        "No 64-bit Windows kernel was found in this guest ({}): the lists read \
         Windows x64's own process and driver lists.",
        miss.message()
    )
}
const ANOTHER_ADDRESS_SPACE: &str = "This module lies in another process's address space; \
     pause while that process runs to see its memory.";

/// The Processes and Kernel modules tabs: the guest kernel's own lists.
fn system_page(
    ui: &mut egui::Ui,
    wide: bool,
    snapshot: &DebugSnapshot,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    match (&snapshot.system, pane.tab) {
        (SystemListing::NoWindowsKernel(miss), _) => {
            empty_state(ui, "No Windows kernel", &no_windows_kernel(*miss));
        }
        (SystemListing::Processes(listing), DebuggerTab::Processes) => {
            processes_page(ui, wide, listing, pane, action);
        }
        (SystemListing::KernelModules(modules), DebuggerTab::KernelModules) => {
            let filter = pane.list_filter.to_lowercase();
            let shown: Vec<&WinModule> = modules
                .iter()
                .filter(|module| module_matches(module, &filter))
                .collect();
            let page = ui.available_size();
            fixed_card(
                ui,
                page,
                Title::counted("Kernel modules", modules.len()),
                |ui| {
                    filter_field(ui, &mut pane.list_filter);
                    header_row(ui, 0.0, &module_columns());
                },
                |ui| module_rows(ui, &shown, true, action),
            );
        }
        // A list for another tab, or none yet: the run thread is reading.
        (
            SystemListing::NotAsked | SystemListing::Processes(_) | SystemListing::KernelModules(_),
            _,
        ) => {
            empty_state(ui, "Reading", READING_LIST);
        }
    }
}

/// The accessible name of a list's filter field.
pub(crate) const FILTER: &str = "Filter by name";

/// The filter over a list: what a name (or path) must contain.
fn filter_field(ui: &mut egui::Ui, filter: &mut String) {
    let response = ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text(FILTER)
            .desired_width(220.0),
    );
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, FILTER));
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
    wide: bool,
    listing: &ProcessListing,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let chosen = listing
        .modules
        .as_ref()
        .and_then(|modules| listing.processes.iter().find(|p| p.eprocess == modules.eprocess));
    let title = match chosen {
        Some(process) => format!("Modules of {} (PID {})", process.name, process.pid),
        None => "Modules".to_owned(),
    };
    let module_count = match listing.modules.as_ref().map(|modules| &modules.read) {
        Some(ModuleRead::Listed(modules)) => Some(modules.len()),
        Some(ModuleRead::NoUserSpace | ModuleRead::Unreadable) | None => None,
    };
    let module_title = Title {
        text: &title,
        count: module_count,
    };
    let process_title = Title::counted("Processes", listing.processes.len());
    let navigable = chosen.is_some_and(|process| process.current);
    let rows = process_rows_shown(&listing.processes, pane.process_layout, &pane.list_filter);
    if wide {
        let gap = SPACE_ITEM;
        let page = ui.available_size();
        let left = ((page.x - gap) * PROCESS_LIST_SHARE).floor();
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        ui.horizontal_top(|ui| {
            fixed_card(
                ui,
                egui::vec2(left, page.y),
                process_title,
                |ui| process_header(ui, &mut pane.process_layout, &mut pane.list_filter),
                |ui| process_rows(ui, &listing.processes, &rows, chosen, &mut pane.selected_process, action),
            );
            fixed_card(
                ui,
                egui::vec2(page.x - gap - left, page.y),
                module_title,
                |ui| header_row(ui, 0.0, &module_columns()),
                |ui| listed_modules(ui, listing, navigable, action),
            );
        });
    } else {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                card(
                    ui,
                    process_title,
                    |ui| process_header(ui, &mut pane.process_layout, &mut pane.list_filter),
                    |ui| process_rows(ui, &listing.processes, &rows, chosen, &mut pane.selected_process, action),
                );
                ui.add_space(SPACE_ITEM);
                card(
                    ui,
                    module_title,
                    |ui| header_row(ui, 0.0, &module_columns()),
                    |ui| listed_modules(ui, listing, navigable, action),
                );
            });
    }
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
/// name holds `filter` — and, in the tree, their ancestors, so a match keeps
/// the branch that leads to it.
fn process_rows_shown(
    processes: &[WinProcess],
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
            let tree = process_tree(processes);
            let mut keep = vec![filter.is_empty(); processes.len()];
            if !filter.is_empty() {
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
                            .and_then(|parent| processes.iter().position(|p| p.pid == parent));
                    }
                }
            }
            tree.into_iter()
                .filter(|row| keep[row.index])
                .map(|row| ShownProcess {
                    index: row.index,
                    depth: row.depth,
                    guides: row.guides,
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
    navigable: bool,
    action: &mut Option<DebuggerAction>,
) {
    let muted = |ui: &mut egui::Ui, text: &str| {
        ui.label(RichText::new(text).color(TEXT_MUTED));
    };
    match listing.modules.as_ref().map(|modules| &modules.read) {
        Some(ModuleRead::Listed(modules)) => {
            let shown: Vec<&WinModule> = modules.iter().collect();
            module_rows(ui, &shown, navigable, action);
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

/// The Processes card's controls and column titles.
fn process_header(ui: &mut egui::Ui, layout: &mut ProcessLayout, filter: &mut String) {
    ui.horizontal(|ui| {
        segmented(
            ui,
            layout,
            &[(ProcessLayout::List, "List"), (ProcessLayout::Tree, "Tree")],
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
/// one selected, the tree's guides in its lead.
fn process_rows(
    ui: &mut egui::Ui,
    processes: &[WinProcess],
    rows: &[ShownProcess],
    chosen: Option<&WinProcess>,
    selected_process: &mut Option<u64>,
    action: &mut Option<DebuggerAction>,
) {
    if rows.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No process matches the filter.").color(TEXT_MUTED));
        return;
    }
    for (n, shown) in rows.iter().enumerate() {
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
            "{} — PID {}, parent {parent}\nEPROCESS {:016X}\nCR3 {:016X}{running}",
            process.name, process.pid, process.eprocess, process.cr3
        ));
        if response.clicked() && !selected {
            *selected_process = Some(process.eprocess);
            *action = Some(DebuggerAction::ShowSystem(SystemView::Processes {
                selected: *selected_process,
            }));
        }
    }
}

/// A module table's columns: base, size, name, and the path in the room left.
fn module_columns() -> [Cell; 4] {
    [
        Cell::left("Base", TEXT_MUTED, 16),
        Cell::right("Size", TEXT_MUTED, MODULE_SIZE_CHARS),
        Cell::left("Name", TEXT_MUTED, MODULE_NAME_CHARS),
        Cell::fill("Path", TEXT_MUTED),
    ]
}

const MODULE_SIZE_CHARS: usize = 8;
const MODULE_NAME_CHARS: usize = 18;

/// One row per module. A module in the running address space opens the CPU
/// tab's memory pane at its base; one elsewhere says why it cannot.
fn module_rows(
    ui: &mut egui::Ui,
    modules: &[&WinModule],
    navigable: bool,
    action: &mut Option<DebuggerAction>,
) {
    if modules.is_empty() {
        ui.add_space(SPACE_ITEM);
        ui.label(RichText::new("No module matches the filter.").color(TEXT_MUTED));
        return;
    }
    for (n, module) in modules.iter().enumerate() {
        let name = module.name.as_deref().unwrap_or("(paged out)");
        let path = module.path.as_deref().unwrap_or("(paged out)");
        let cells = [
            Cell::left(format!("{:016X}", module.base), TEXT_MUTED, 16),
            Cell::right(format!("{:X}", module.size), TEXT_MUTED, MODULE_SIZE_CHARS),
            Cell::left(name, TEXT_PRIMARY, MODULE_NAME_CHARS),
            Cell::fill(path, TEXT_MUTED.gamma_multiply(0.8)),
        ];
        let response = list_row(
            ui,
            0.0,
            &cells,
            Tone::striped(n),
            RowSense::Pick { selected: false },
        );
        if navigable {
            if response
                .on_hover_text(format!("{path}\nClick to show it in memory."))
                .clicked()
            {
                *action = Some(DebuggerAction::OpenModule(module.base));
            }
        } else {
            response.on_hover_text(format!("{path}\n{ANOTHER_ADDRESS_SPACE}"));
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
        let names = |rows: Vec<ShownProcess>| -> Vec<&str> {
            rows.iter().map(|row| processes[row.index].name.as_str()).collect()
        };
        assert_eq!(
            names(process_rows_shown(&processes, ProcessLayout::Tree, "WMI")),
            ["System", "smss.exe", "winlogon.exe", "services.exe", "wmiprvse.exe"]
        );
        assert_eq!(
            names(process_rows_shown(&processes, ProcessLayout::List, "wmi")),
            ["wmiprvse.exe"]
        );
        assert_eq!(
            process_rows_shown(&processes, ProcessLayout::Tree, "").len(),
            processes.len()
        );
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
