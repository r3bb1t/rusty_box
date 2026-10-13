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
};
use crate::debugger::search::{
    pattern_of, FindOutcome, FindRequest, FoundAt, PatternError, SearchEncoding,
};
use crate::shell::theme::{
    shell_card_frame, ACCENT_AMBER, ACCENT_CYAN, ACCENT_RED, SPACE_ITEM, SPACE_PAGE, TEXT_MUTED,
    TEXT_PRIMARY, TEXT_SECONDARY,
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
                ui.label(RichText::new(NOT_RUNNING).color(TEXT_MUTED));
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
                ui.label(RichText::new(PAUSE_TO_INSPECT).color(TEXT_MUTED));
                return;
            };
            if !paused {
                ui.label(RichText::new(SHOWING_LAST_PAUSE).color(ACCENT_AMBER));
            }
            privilege_line(ui, snapshot);
            follow_window(pane, snapshot);
            ui.add_space(SPACE_ITEM);
            if ui.available_width() >= TWO_COLUMN_WIDTH {
                grid_layout(ui, snapshot, marks, pane, &mut action);
            } else {
                stacked_layout(ui, snapshot, marks, pane, &mut action);
            }
        });
    action
}

/// Which privilege the shown processor runs at, as a badge — user mode
/// (CPL 3) or kernel mode (CPL 0) — and, for a Windows guest, the process
/// it runs for.
fn privilege_line(ui: &mut egui::Ui, snapshot: &DebugSnapshot) {
    let Some(view) = shown_view(snapshot) else {
        return;
    };
    ui.horizontal(|ui| {
        let (text, color) = match view.registers.cpl {
            0 => ("Kernel mode", ACCENT_AMBER),
            3 => ("User mode", ACCENT_CYAN),
            _ => ("Privileged", ACCENT_AMBER),
        };
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0_f32, color))
            .corner_radius(4)
            .inner_margin(egui::Margin::symmetric(6, 1))
            .show(ui, |ui| {
                ui.label(RichText::new(format!("{text} · CPL {}", view.registers.cpl)).color(color));
            });
        match &snapshot.process {
            Some(path) => {
                let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
                ui.label(RichText::new(format!("Process: {name}")).color(TEXT_PRIMARY))
                    .on_hover_text(path);
            }
            None => {
                ui.label(
                    RichText::new("Process: unknown (not a Windows user thread)")
                        .color(TEXT_MUTED),
                );
            }
        }
        ui.label(RichText::new(format!("CR3 {:X}", view.registers.cr3)).color(TEXT_MUTED))
            .on_hover_text("The address space: each process has its own page tables.");
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
        if let Some(view) = shown_view(snapshot) {
            ui.label(RichText::new(mode_label(view)).color(TEXT_MUTED));
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

/// e.g. "64-bit mode · CPL 0 · running".
fn mode_label(view: &CpuDebugView) -> String {
    let mode = match view.mode {
        ExecutionMode::Real => "Real mode",
        ExecutionMode::Virtual8086 => "Virtual-8086 mode",
        ExecutionMode::Protected => match view.code_size {
            CodeSize::Bits16 => "16-bit protected mode",
            CodeSize::Bits32 | CodeSize::Bits64 => "32-bit protected mode",
        },
        ExecutionMode::LongCompat => "Compatibility mode",
        ExecutionMode::Long64 => "64-bit mode",
    };
    let activity = match view.activity {
        ProcessorActivity::Active => "running",
        ProcessorActivity::Halted => "halted",
        ProcessorActivity::Shutdown => "shut down",
        ProcessorActivity::WaitForSipi => "waiting for startup",
        ProcessorActivity::Mwait => "waiting (MWAIT)",
    };
    format!("{mode} · {activity}")
}

fn pane_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).strong().color(TEXT_PRIMARY));
}

fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().size(TEXT_SECONDARY)
}

/// One listing row: cut at the pane's edge rather than wrapped, so columns
/// of hex stay aligned.
fn row(ui: &mut egui::Ui, text: RichText) {
    ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Truncate));
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
        fixed_card(ui, egui::vec2(left, top), "Code", |_| {}, |ui| code_rows(ui, snapshot));
        fixed_card(
            ui,
            egui::vec2(right, top),
            "Registers",
            |_| {},
            |ui| register_rows(ui, snapshot, marks),
        );
    });
    ui.horizontal_top(|ui| {
        fixed_card(
            ui,
            egui::vec2(left, bottom),
            "Memory",
            |ui| memory_bar(ui, snapshot, marks, pane, action),
            |ui| memory_rows(ui, snapshot, marks),
        );
        fixed_card(
            ui,
            egui::vec2(right, bottom),
            "Stack",
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
            card(ui, "Registers", |ui| register_rows(ui, snapshot, marks));
            card(ui, "Code", |ui| code_rows(ui, snapshot));
            card(ui, "Stack", |ui| stack_rows(ui, snapshot, marks));
            card(ui, "Memory", |ui| {
                memory_bar(ui, snapshot, marks, pane, action);
                memory_rows(ui, snapshot, marks);
            });
        });
}

/// A pane of exactly `size`: its title, a fixed `header` (the memory pane's
/// address bar), and a `body` that scrolls inside the pane when it is
/// taller or wider than the room left.
fn fixed_card<H, B>(ui: &mut egui::Ui, size: egui::Vec2, title: &str, header: H, body: B)
where
    H: FnOnce(&mut egui::Ui),
    B: FnOnce(&mut egui::Ui),
{
    let frame = shell_card_frame();
    let inner = (size - frame.total_margin().sum()).max(egui::Vec2::ZERO);
    ui.allocate_ui(size, |ui| {
        frame.show(ui, |ui| {
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            pane_title(ui, title);
            header(ui);
            egui::ScrollArea::both()
                .id_salt(title)
                .auto_shrink([false, false])
                .show(ui, body);
        });
    });
}

/// A pane as tall as its content, the page's full width.
fn card<B: FnOnce(&mut egui::Ui)>(ui: &mut egui::Ui, title: &str, body: B) {
    shell_card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        pane_title(ui, title);
        body(ui);
    });
}

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
        let text = match &line.text {
            CodeText::Opcode(name) => name.as_str(),
            CodeText::Undecodable => "(undecodable)",
            CodeText::Unreadable => "(unreadable)",
        };
        let listing = format!(
            "{:0digits$X}  {:<24}  {text}",
            line.address,
            bytes.join(" ")
        );
        // The first line is the instruction at RIP.
        let color = if index == 0 { ACCENT_CYAN } else { TEXT_PRIMARY };
        row(ui, mono(listing).color(color));
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
    {
        egui::Grid::new("debugger_registers")
            .num_columns(2)
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
        let flags_color = if old.is_some_and(|o| o.eflags != r.eflags) {
            CHANGED
        } else {
            TEXT_MUTED
        };
        row(ui, mono(flags_text(r.eflags)).color(flags_color));
        egui::Grid::new("debugger_system_registers")
            .num_columns(2)
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
        ui.collapsing("x87", |ui| {
            egui::Grid::new("debugger_x87").num_columns(2).show(ui, |ui| {
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
            egui::Grid::new("debugger_sse").num_columns(2).show(ui, |ui| {
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

/// The nine status and control flags, as x64dbg lists them.
fn flags_text(eflags: u32) -> String {
    [
        ("CF", 0),
        ("PF", 2),
        ("AF", 4),
        ("ZF", 6),
        ("SF", 7),
        ("TF", 8),
        ("IF", 9),
        ("DF", 10),
        ("OF", 11),
    ]
    .iter()
    .map(|(name, bit)| format!("{name} {}", (eflags >> bit) & 1))
    .collect::<Vec<_>>()
    .join("  ")
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
            ui.selectable_value(&mut pane.space, AddressSpace::Linear, "Linear");
            ui.selectable_value(&mut pane.space, AddressSpace::Physical, "Physical");
            let label = ui.label(RichText::new(MEMORY_ADDRESS).color(TEXT_MUTED));
            let field = ui
                .add(
                    egui::TextEdit::singleline(&mut pane.address_text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(160.0),
                )
                .labelled_by(label.id);
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
        ui.selectable_value(&mut pane.search_encoding, SearchEncoding::Hex, "Hex");
        ui.selectable_value(&mut pane.search_encoding, SearchEncoding::Text, "Text");
        ui.selectable_value(&mut pane.search_encoding, SearchEncoding::Utf16, "UTF-16");
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
    let font = egui::FontId::monospace(TEXT_SECONDARY);
    for (index, line) in snapshot.memory.rows.iter().enumerate() {
        let Some(bytes) = line.bytes else {
            let text = format!("{:016X}  {}", line.address, ["??"; 16].join(" "));
            row(ui, mono(text).color(TEXT_MUTED));
            continue;
        };
        let old = before.and_then(|rows| rows.get(index)).and_then(|old| old.bytes);
        let mut job = LayoutJob::default();
        let plain = TextFormat::simple(font.clone(), TEXT_PRIMARY);
        job.append(&format!("{:016X}  ", line.address), 0.0, plain.clone());
        for (column, byte) in bytes.iter().enumerate() {
            let address = line.address.wrapping_add(column as u64);
            let color = if old.is_some_and(|old| old[column] != *byte) {
                CHANGED
            } else if hit.is_some_and(|hit| in_hit(hit, address)) {
                ACCENT_AMBER
            } else {
                TEXT_PRIMARY
            };
            job.append(&format!("{byte:02X} "), 0.0, TextFormat::simple(font.clone(), color));
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
        job.append(&format!(" {ascii}"), 0.0, TextFormat::simple(font.clone(), TEXT_MUTED));
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Truncate));
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
        let color = if changed {
            CHANGED
        } else if index == 0 {
            ACCENT_CYAN
        } else {
            TEXT_PRIMARY
        };
        row(
            ui,
            mono(format!("{:0digits$X}  {value}", slot.address)).color(color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
