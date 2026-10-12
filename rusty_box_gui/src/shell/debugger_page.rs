//! The Debugger page: a paused VM's processors, code, memory and stack. It is
//! laid out as x64dbg lays out its CPU tab (code above memory on the left,
//! registers above the stack on the right) and stacked in one column where
//! the page is too narrow for two. Like the VM bar, it draws from borrowed
//! state and reports what was asked for; the app acts on it.

use egui::RichText;
use rusty_box::cpu::{CodeSize, CpuDebugView, ExecutionMode, ProcessorActivity};

use crate::debugger::control::DebugNote;
use crate::debugger::inspect::{
    AddressSpace, CodeText, DebugSnapshot, GuestAddress, MemoryTarget, MEMORY_WINDOW_BYTES,
};
use crate::shell::theme::{
    shell_card_frame, ACCENT_AMBER, ACCENT_CYAN, ACCENT_RED, SPACE_ITEM, SPACE_PAGE, TEXT_MUTED,
    TEXT_PRIMARY,
};

pub(crate) const NOT_RUNNING: &str =
    "Power the VM on and pause it to see its processors, code and memory.";
const PAUSE_TO_INSPECT: &str = "Pause the VM to see its processors, code and memory.";
const SHOWING_LAST_PAUSE: &str = "Running: showing the state at the last pause.";
const STEP_NEEDS_INTERPRETER: &str =
    "Step needs the interpreter: a VM on the hypervisor retires instructions nothing counts.";
/// The accessible name of the memory pane's address field.
pub(crate) const MEMORY_ADDRESS: &str = "Memory address";
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
}

impl Default for DebuggerPane {
    fn default() -> Self {
        Self {
            address_text: String::new(),
            space: AddressSpace::Linear,
            address_error: None,
            cpu: 0,
            shown_start: None,
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
    pub(crate) pane: &'a mut DebuggerPane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DebuggerAction {
    Step,
    DumpRam,
    ShowMemory(MemoryTarget),
    SelectCpu(usize),
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
        pane,
    } = state;
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
            follow_window(pane, snapshot);
            ui.add_space(SPACE_ITEM);
            if ui.available_width() >= TWO_COLUMN_WIDTH {
                ui.columns(2, |columns| {
                    code_pane(&mut columns[0], snapshot);
                    memory_pane(&mut columns[0], snapshot, pane, &mut action);
                    registers_pane(&mut columns[1], snapshot);
                    stack_pane(&mut columns[1], snapshot);
                });
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    registers_pane(ui, snapshot);
                    code_pane(ui, snapshot);
                    stack_pane(ui, snapshot);
                    memory_pane(ui, snapshot, pane, &mut action);
                });
            }
        });
    action
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
    format!("{mode} · CPL {} · {activity}", view.registers.cpl)
}

fn pane_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).strong().color(TEXT_PRIMARY));
}

fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace()
}

fn hex_digits(size: CodeSize) -> usize {
    match size {
        CodeSize::Bits64 => 16,
        CodeSize::Bits16 | CodeSize::Bits32 => 8,
    }
}

fn code_pane(ui: &mut egui::Ui, snapshot: &DebugSnapshot) {
    shell_card_frame().show(ui, |ui| {
        pane_title(ui, "Code");
        let digits = shown_view(snapshot).map_or(16, |view| hex_digits(view.code_size));
        for (index, line) in snapshot.code.iter().enumerate() {
            let bytes: Vec<String> = line.bytes.iter().map(|byte| format!("{byte:02X}")).collect();
            let text = match &line.text {
                CodeText::Opcode(name) => name.as_str(),
                CodeText::Undecodable => "(undecodable)",
                CodeText::Unreadable => "(unreadable)",
            };
            let row = format!(
                "{:0digits$X}  {:<30}  {text}",
                line.address,
                bytes.join(" ")
            );
            // The first line is the instruction at RIP.
            let color = if index == 0 { ACCENT_CYAN } else { TEXT_PRIMARY };
            ui.label(mono(row).color(color));
        }
    });
}

fn registers_pane(ui: &mut egui::Ui, snapshot: &DebugSnapshot) {
    let Some(view) = shown_view(snapshot) else {
        return;
    };
    let r = &view.registers;
    let wide = view.code_size == CodeSize::Bits64;
    shell_card_frame().show(ui, |ui| {
        pane_title(ui, "Registers");
        egui::Grid::new("debugger_registers")
            .num_columns(2)
            .show(ui, |ui| {
                let gprs = [
                    ("RAX", "EAX", r.rax),
                    ("RBX", "EBX", r.rbx),
                    ("RCX", "ECX", r.rcx),
                    ("RDX", "EDX", r.rdx),
                    ("RSI", "ESI", r.rsi),
                    ("RDI", "EDI", r.rdi),
                    ("RBP", "EBP", r.rbp),
                    ("RSP", "ESP", r.rsp),
                    ("R8", "", r.r8),
                    ("R9", "", r.r9),
                    ("R10", "", r.r10),
                    ("R11", "", r.r11),
                    ("R12", "", r.r12),
                    ("R13", "", r.r13),
                    ("R14", "", r.r14),
                    ("R15", "", r.r15),
                ];
                for (long, short, value) in gprs.iter().take(if wide { 16 } else { 8 }) {
                    if wide {
                        register_row(ui, long, format!("{value:016X}"));
                    } else {
                        register_row(ui, short, format!("{:08X}", value & 0xFFFF_FFFF));
                    }
                }
                if wide {
                    register_row(ui, "RIP", format!("{:016X}", r.rip));
                } else {
                    register_row(ui, "EIP", format!("{:08X}", r.rip & 0xFFFF_FFFF));
                }
                register_row(ui, "EFLAGS", format!("{:08X}", r.eflags));
                register_row(ui, "", flags_text(r.eflags));
                let bases = &view.segment_bases;
                for (name, selector, base) in [
                    ("CS", r.cs, bases.cs),
                    ("SS", r.ss, bases.ss),
                    ("DS", r.ds, bases.ds),
                    ("ES", r.es, bases.es),
                    ("FS", r.fs, bases.fs),
                    ("GS", r.gs, bases.gs),
                ] {
                    register_row(ui, name, format!("{selector:04X}  base {base:016X}"));
                }
                for (name, value) in [
                    ("CR0", r.cr0),
                    ("CR2", r.cr2),
                    ("CR3", r.cr3),
                    ("CR4", r.cr4),
                    ("EFER", view.efer),
                ] {
                    register_row(ui, name, format!("{value:016X}"));
                }
            });
        ui.collapsing("x87", |ui| {
            egui::Grid::new("debugger_x87").num_columns(2).show(ui, |ui| {
                for (index, register) in r.fpu_regs.iter().enumerate() {
                    register_row(ui, &format!("ST{index}"), le_hex(register));
                }
                register_row(ui, "FSW", format!("{:04X}", r.fpu_sw));
                register_row(ui, "FCW", format!("{:04X}", r.fpu_cw));
            });
        });
        ui.collapsing("SSE", |ui| {
            egui::Grid::new("debugger_sse").num_columns(2).show(ui, |ui| {
                let shown = if wide { 16 } else { 8 };
                for (index, register) in r.xmm.iter().enumerate().take(shown) {
                    register_row(ui, &format!("XMM{index}"), le_hex(register));
                }
                register_row(ui, "MXCSR", format!("{:08X}", r.mxcsr));
            });
        });
    });
}

fn register_row(ui: &mut egui::Ui, name: &str, value: String) {
    ui.label(mono(name).color(TEXT_MUTED));
    ui.label(mono(value).color(TEXT_PRIMARY));
    ui.end_row();
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

fn memory_pane(
    ui: &mut egui::Ui,
    snapshot: &DebugSnapshot,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    shell_card_frame().show(ui, |ui| {
        pane_title(ui, "Memory");
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
        for row in &snapshot.memory.rows {
            let text = match row.bytes {
                Some(bytes) => {
                    let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
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
                    format!("{:016X}  {}  {ascii}", row.address, hex.join(" "))
                }
                None => format!("{:016X}  {}", row.address, ["??"; 16].join(" ")),
            };
            ui.label(mono(text).color(TEXT_PRIMARY));
        }
    });
}

fn stack_pane(ui: &mut egui::Ui, snapshot: &DebugSnapshot) {
    let digits = shown_view(snapshot).map_or(16, |view| hex_digits(view.code_size));
    shell_card_frame().show(ui, |ui| {
        pane_title(ui, "Stack");
        for (index, slot) in snapshot.stack.iter().enumerate() {
            let value = slot
                .value
                .map_or_else(|| "?".repeat(digits), |value| format!("{value:0digits$X}"));
            let marker = if index == 0 { "  top" } else { "" };
            ui.label(
                mono(format!("{:0digits$X}  {value}{marker}", slot.address)).color(TEXT_PRIMARY),
            );
        }
    });
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
