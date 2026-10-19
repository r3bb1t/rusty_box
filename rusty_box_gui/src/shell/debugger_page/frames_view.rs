//! The CPU tab's Call stack, Locals and Watch panes (spec §8, §9): a pause's
//! frames, the picked frame's variables and the watches, as rows.

use std::collections::{BTreeMap, BTreeSet};

use egui::RichText;

use super::references::REFERENCES_TO;
use super::source_view::{file_name, source_at, SourceTip};
use crate::debugger::frames::{FramesRead, InspectAsk, LocalsRead, WatchRead};
use crate::debugger::inspect::SymbolIndex;
use crate::debugger::unwind::{Frame, StackEnd, MAX_FRAMES};
use crate::debugger::variables::{
    Expand, Preview, PreviewItem, RowKind, ShownValue, Signedness, StringEnd, ValuePath, VariableRow,
};
use crate::debugger::xref::service::{AddressRange, XrefTarget};
use crate::shell::listing::{
    char_width, lead_arrow, lead_chevron, lead_hollow_arrow, list_row, Cell, RowSense, Tone,
};
use crate::shell::theme::{ACCENT_AMBER, ACCENT_BLUE, ACCENT_CYAN, TEXT_MUTED, TEXT_PRIMARY};
use crate::symbols::hub::SymbolsView;
use crate::symbols::names::Namer;

pub(crate) const MEMORY: &str = "Memory";
pub(crate) const LOCALS: &str = "Locals";
pub(crate) const WATCH: &str = "Watch";
pub(crate) const STACK: &str = "Stack";
pub(crate) const CALL_STACK: &str = "Call stack";
/// The accessible name of the Watch pane's field.
pub(crate) const ADD_WATCH: &str = "Add watch";
pub(crate) const MORE_ELEMENTS: &str = "Show 100 more";
/// The accessible name of the hollow arrow on a picked caller frame's line.
pub(crate) const CALLER_LINE: &str = "Caller frame line";
/// The accessible name of the hollow arrow on the picked caller's row in
/// the call stack.
pub(crate) const PICKED_FRAME: &str = "Picked frame";
/// The value tip's accessible name, and what it says until its read lands.
pub(crate) const VALUE_TIP: &str = "Value tip";
pub(crate) const TIP_READING: &str = "Reading…";
const REMOVE_WATCH: &str = "Remove watch";
const NOT_READ: &str = "Variables are read at a pause in 32- or 64-bit code.";
const NO_MODULE: &str = "No module holds this frame's code.";
const NO_FUNCTION: &str = "No function's records hold this frame's code.";
const NO_VARIABLES: &str = "No variables are in scope here.";
const READING: &str = "Reading…";
const NO_WATCHES: &str = "Type an expression below: a name, p->field, s.field, a[3], *p or &x.";
const CEILING: &str = "More rows than are listed: close some to see the rest.";
/// The room a row's lead keeps for its chevron, points.
const CHEVRON_LEAD: f32 = 12.0;
/// The widest the name and type columns grow, in characters.
const NAME_CHARS_MAX: usize = 40;
const TYPE_CHARS_MAX: usize = 32;
/// Characters a row is indented per level.
const INDENT_CHARS: usize = 2;

/// What the CPU tab's lower-left pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LowerLeft {
    Memory,
    Locals,
    Watch,
}

impl LowerLeft {
    /// The owner's pick, or, before one, the picked frame's variables
    /// wherever the pause is in the owner's source, else memory.
    pub(crate) fn shown(picked: Option<LowerLeft>, has_source: bool) -> LowerLeft {
        match picked {
            Some(pane) => pane,
            None if has_source => LowerLeft::Locals,
            None => LowerLeft::Memory,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            LowerLeft::Memory => MEMORY,
            LowerLeft::Locals => LOCALS,
            LowerLeft::Watch => WATCH,
        }
    }
}

/// What the CPU tab's lower-right pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LowerRight {
    Stack,
    CallStack,
}

impl LowerRight {
    /// The owner's pick, or, before one, the call stack wherever the pause
    /// is in the owner's source, else the stack's slots.
    pub(crate) fn shown(picked: Option<LowerRight>, has_source: bool) -> LowerRight {
        match picked {
            Some(pane) => pane,
            None if has_source => LowerRight::CallStack,
            None => LowerRight::Stack,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            LowerRight::Stack => STACK,
            LowerRight::CallStack => CALL_STACK,
        }
    }
}

/// What the variable views keep between frames: the watches, the one being
/// typed, the values open, the pages listed of each open array, and the
/// value tip open in the source view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct VariableState {
    pub(crate) watches: Vec<String>,
    pub(crate) watch_text: String,
    pub(crate) expanded: BTreeSet<ValuePath>,
    pub(crate) pages: BTreeMap<ValuePath, u32>,
    pub(crate) tip: Option<SourceTip>,
}

impl VariableState {
    /// What the run thread is asked to read.
    pub(crate) fn ask(&self) -> InspectAsk {
        InspectAsk { watches: self.watches.clone(), expanded: self.expanded.clone(), pages: self.pages.clone() }
    }

    /// Acts on a row's ask: opens or closes a value, lists another page. A
    /// references ask is the page's to act on, and is handed back.
    pub(crate) fn apply(&mut self, ask: RowAsk) -> Option<XrefTarget> {
        match ask {
            RowAsk::Toggle(path) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
                None
            }
            RowAsk::MorePages(path) => {
                let pages = self.pages.entry(path).or_insert(0);
                *pages = pages.saturating_add(1);
                None
            }
            RowAsk::References(target) => Some(target),
        }
    }
}

/// What a click on a variable row asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowAsk {
    Toggle(ValuePath),
    MorePages(ValuePath),
    References(XrefTarget),
}

/// A frame's name: the function it is in, at its return address for a
/// caller (`mydriver!ReadBlock+0x1c`), as WinDbg's `k` names it.
pub(crate) fn frame_label(frame: &Frame, names: Namer<'_>) -> String {
    let code = frame.code_address();
    let rip = frame.registers.rip;
    match names.name(code) {
        Some(mut named) => {
            let past = rip.wrapping_sub(code);
            named.module_offset = named.module_offset.wrapping_add(past);
            if let Some(function) = named.function.as_mut() {
                function.offset = function.offset.wrapping_add(past);
            }
            named.label()
        }
        None => format!("{rip:#x}"),
    }
}

/// A frame's line in the owner's source: `read.c:120`.
pub(crate) fn frame_line(frame: &Frame, index: &SymbolIndex, view: &SymbolsView) -> Option<String> {
    let source = source_at(frame.code_address(), index, view)?;
    Some(format!("{}:{}", file_name(&source.build_path), source.line))
}

/// Why a stack ends where it does, as its last row says; `None` for a stack
/// that reached its outermost frame.
pub(crate) fn stack_end_text(end: StackEnd) -> Option<String> {
    let text = match end {
        StackEnd::Top => return None,
        StackEnd::NoUnwindInfo { rip } => format!("stack ends: no unwind information for {rip:#x}"),
        StackEnd::BadUnwindInfo { at } => format!("stack ends: unwind information at {at:#x} does not read"),
        StackEnd::RegisterNotKnown { .. } => "stack ends: a register this frame needs is not known".to_owned(),
        StackEnd::NoFramePointer => "frame not unwound: no frame pointer".to_owned(),
        StackEnd::NotGrowing => "stack does not unwind further".to_owned(),
        StackEnd::Unreadable { at } => format!("stack ends: memory at {at:#x} is not readable"),
        StackEnd::Ceiling => format!("stack ends: {MAX_FRAMES} frames shown"),
        StackEnd::SixteenBit => "16-bit code: the stack is not unwound".to_owned(),
    };
    Some(text)
}

/// A value as a row shows it (spec §8.2); a pointer into a module is named.
pub(crate) fn value_text(value: &ShownValue, names: Namer<'_>) -> String {
    match value {
        ShownValue::Pointer { address, string } => {
            let mut text = address_text(*address);
            if let Some(named) = names.name(*address) {
                text.push(' ');
                text.push_str(&named.label());
            }
            if let Some(string) = string {
                text.push_str(&quoted_string(&string.text, string.end));
            }
            text
        }
        ShownValue::Integer { .. }
        | ShownValue::Character { .. }
        | ShownValue::Boolean { .. }
        | ShownValue::Real32(_)
        | ShownValue::Real64(_)
        | ShownValue::Hresult(_)
        | ShownValue::Enum { .. }
        | ShownValue::Aggregate { .. }
        | ShownValue::NotAvailable
        | ShownValue::OptimizedAway
        | ShownValue::Unreadable { .. }
        | ShownValue::UnknownType { .. }
        | ShownValue::Problem(_) => plain_text(value),
    }
}

/// [`value_text`] without names.
fn plain_text(value: &ShownValue) -> String {
    match value {
        ShownValue::Integer { raw, size, signedness } => {
            let bits = masked(*raw, *size);
            match signedness {
                Signedness::Signed => format!("{} ({bits:#x})", signed(*raw, *size)),
                Signedness::Unsigned => format!("{bits} ({bits:#x})"),
            }
        }
        ShownValue::Character { code, .. } => format!("{} ({code})", quoted_char(*code)),
        ShownValue::Boolean { raw: 0 } => "false".to_owned(),
        ShownValue::Boolean { raw: 1 } => "true".to_owned(),
        ShownValue::Boolean { raw } => format!("true ({raw:#04x})"),
        ShownValue::Real32(real) => format!("{real}"),
        ShownValue::Real64(real) => format!("{real}"),
        ShownValue::Hresult(code) => format!("{code:#010x}"),
        ShownValue::Pointer { address, string } => {
            let mut text = address_text(*address);
            if let Some(string) = string {
                text.push_str(&quoted_string(&string.text, string.end));
            }
            text
        }
        ShownValue::Enum { raw, names } if names.is_empty() => raw.to_string(),
        ShownValue::Enum { names, .. } => names.join(" | "),
        ShownValue::Aggregate { count, preview, more } => {
            let mut items: Vec<String> = preview.iter().map(preview_text).collect();
            if *more == Preview::Cut {
                items.push("…".to_owned());
            }
            let inner = items.join(", ");
            match count {
                None => format!("{{{inner}}}"),
                Some(count) => format!("[{count}] [{inner}]"),
            }
        }
        ShownValue::NotAvailable => "not available here".to_owned(),
        ShownValue::OptimizedAway => "optimized away".to_owned(),
        ShownValue::Unreadable { at } => format!("memory at {at:#x} is not readable"),
        ShownValue::UnknownType { index, bytes, .. } => {
            let bytes: String = bytes.iter().map(|byte| format!(" {byte:02x}")).collect();
            format!("<type {index:#x}>{bytes}")
        }
        ShownValue::Problem(problem) => problem.clone(),
    }
}

/// One preview item: `name=value` or the value, in its short form.
fn preview_text(item: &PreviewItem) -> String {
    let value = short_text(&item.value);
    match &item.name {
        Some(name) => format!("{name}={value}"),
        None => value,
    }
}

/// A value inside a preview: an integer in decimal only, a pointer as its
/// address, a structure or array as braces.
fn short_text(value: &ShownValue) -> String {
    match value {
        ShownValue::Integer { raw, size, signedness: Signedness::Signed } => signed(*raw, *size).to_string(),
        ShownValue::Integer { raw, size, signedness: Signedness::Unsigned } => masked(*raw, *size).to_string(),
        ShownValue::Pointer { address, .. } => address_text(*address),
        ShownValue::Aggregate { count, preview, more } => {
            let empty = preview.is_empty() && *more == Preview::Whole;
            match count {
                None if empty => "{}".to_owned(),
                None => "{…}".to_owned(),
                Some(_) if empty => "[]".to_owned(),
                Some(_) => "[…]".to_owned(),
            }
        }
        ShownValue::Character { .. }
        | ShownValue::Boolean { .. }
        | ShownValue::Real32(_)
        | ShownValue::Real64(_)
        | ShownValue::Hresult(_)
        | ShownValue::Enum { .. }
        | ShownValue::NotAvailable
        | ShownValue::OptimizedAway
        | ShownValue::Unreadable { .. }
        | ShownValue::UnknownType { .. }
        | ShownValue::Problem(_) => plain_text(value),
    }
}

/// `raw`'s low `size` bytes.
fn masked(raw: u64, size: u8) -> u64 {
    match u32::from(size).checked_mul(8).filter(|bits| *bits < 64) {
        Some(bits) => raw & ((1u64 << bits) - 1),
        None => raw,
    }
}

/// `raw`'s low `size` bytes as a signed number.
fn signed(raw: u64, size: u8) -> i64 {
    let unused = 64u32.saturating_sub(u32::from(size).saturating_mul(8));
    raw.checked_shl(unused).map_or(0, |high| (high as i64) >> unused)
}

/// An address as WinDbg writes one: eight digits, sixteen split by a
/// backtick past 4 GiB.
fn address_text(address: u64) -> String {
    if address > u64::from(u32::MAX) {
        format!("0x{:08x}`{:08x}", address >> 32, address & 0xFFFF_FFFF)
    } else {
        format!("0x{address:08x}")
    }
}

/// A character as C writes it: printable ASCII as itself, else its code.
fn quoted_char(code: u32) -> String {
    match char::from_u32(code) {
        Some(quote @ ('\'' | '\\')) => format!("'\\{quote}'"),
        Some(shown) if shown.is_ascii_graphic() || shown == ' ' => format!("'{shown}'"),
        Some(_) | None if code <= 0xFF => format!("'\\x{code:02X}'"),
        Some(_) | None => format!("'\\u{{{code:04X}}}'"),
    }
}

/// A pointed-at string in quotes, control characters escaped; `…` where
/// it was cut or ran into memory that does not read.
fn quoted_string(text: &str, end: StringEnd) -> String {
    let mut quoted = String::from(" \"");
    for character in text.chars() {
        if character.is_control() {
            quoted.extend(character.escape_default());
        } else if character == '"' {
            quoted.push_str("\\\"");
        } else {
            quoted.push(character);
        }
    }
    match end {
        StringEnd::Terminated => {}
        StringEnd::Cut | StringEnd::Unreadable => quoted.push('…'),
    }
    quoted.push('"');
    quoted
}

/// The colour a value is drawn in: muted where there is none to read,
/// amber where something went wrong.
fn value_color(value: &ShownValue) -> egui::Color32 {
    match value {
        ShownValue::NotAvailable | ShownValue::OptimizedAway => TEXT_MUTED,
        ShownValue::Unreadable { .. } | ShownValue::Problem(_) | ShownValue::UnknownType { .. } => ACCENT_AMBER,
        ShownValue::Integer { .. }
        | ShownValue::Character { .. }
        | ShownValue::Boolean { .. }
        | ShownValue::Real32(_)
        | ShownValue::Real64(_)
        | ShownValue::Hresult(_)
        | ShownValue::Pointer { .. }
        | ShownValue::Enum { .. }
        | ShownValue::Aggregate { .. } => TEXT_PRIMARY,
    }
}

/// The call stack: a row per frame, the innermost arrowed, the picked
/// caller hollow-arrowed; then why the stack ends, when it ends early.
/// Returns the frame clicked.
pub(crate) fn call_stack_rows(
    ui: &mut egui::Ui,
    frames: &FramesRead,
    names: Namer<'_>,
    index: &SymbolIndex,
    view: &SymbolsView,
) -> Option<usize> {
    let mut picked = None;
    let digits = frames.frames.len().to_string().len();
    let lines: Vec<String> =
        frames.frames.iter().map(|frame| frame_line(frame, index, view).unwrap_or_default()).collect();
    let line_chars = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0);
    for (number, frame) in frames.frames.iter().enumerate() {
        let selected = number == frames.selected;
        let tone = if selected { Tone::Selected } else { Tone::Plain };
        let cells = [
            Cell::right(number.to_string(), TEXT_MUTED, digits),
            Cell::fill(frame_label(frame, names), if number == 0 { ACCENT_CYAN } else { TEXT_PRIMARY }),
            Cell::left(lines.get(number).cloned().unwrap_or_default(), TEXT_MUTED, line_chars),
        ];
        let response = list_row(ui, CHEVRON_LEAD, &cells, tone, RowSense::Pick { selected });
        if number == 0 {
            lead_arrow(ui, response.rect, ACCENT_CYAN);
        } else if selected {
            lead_hollow_arrow(ui, response.rect, ACCENT_CYAN, PICKED_FRAME);
        }
        if response.clicked() {
            picked = Some(number);
        }
    }
    if let Some(end) = stack_end_text(frames.end) {
        list_row(ui, CHEVRON_LEAD, &[Cell::fill(end, TEXT_MUTED)], Tone::Plain, RowSense::Look);
    }
    picked
}

/// The Locals pane: the picked frame's variables, or why there are none.
pub(crate) fn locals_rows(ui: &mut egui::Ui, locals: &LocalsRead, names: Namer<'_>, index: &SymbolIndex) -> Option<RowAsk> {
    match locals {
        LocalsRead::NotAsked => muted(ui, NOT_READ),
        LocalsRead::NoModule => muted(ui, NO_MODULE),
        LocalsRead::NoSymbols { module } => muted(ui, &format!("Symbols for {module} are loading or not found.")),
        LocalsRead::NoFunction => muted(ui, NO_FUNCTION),
        LocalsRead::Unreadable { why } => {
            ui.label(RichText::new(why).color(ACCENT_AMBER));
        }
        LocalsRead::Rows(rows) if rows.is_empty() => muted(ui, NO_VARIABLES),
        LocalsRead::Rows(rows) => return variable_rows(ui, rows, names, index),
    }
    None
}

/// The Watch pane: each watch's rows under a way to remove it, then the
/// field that adds one. `reads` is what the last pause read of them.
pub(crate) fn watch_rows(
    ui: &mut egui::Ui,
    reads: &[WatchRead],
    state: &mut VariableState,
    names: Namer<'_>,
    index: &SymbolIndex,
) -> Option<RowAsk> {
    let mut ask = None;
    let mut removed = None;
    if state.watches.is_empty() {
        muted(ui, NO_WATCHES);
    }
    for (number, text) in state.watches.iter().enumerate() {
        ui.horizontal(|ui| {
            let label = format!("{REMOVE_WATCH} {text}");
            let remove = ui.small_button("×");
            remove.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            if remove.on_hover_text(&label).clicked() {
                removed = Some(number);
            }
            ui.label(RichText::new(text).monospace().color(TEXT_MUTED));
        });
        match reads.get(number).filter(|read| read.text == *text) {
            Some(read) => {
                if let Some(asked) = variable_rows(ui, &read.rows, names, index) {
                    ask = Some(asked);
                }
            }
            None => muted(ui, READING),
        }
    }
    if let Some(number) = removed {
        if number < state.watches.len() {
            state.watches.remove(number);
        }
    }
    ui.add_space(4.0);
    let field = ui.add(
        egui::TextEdit::singleline(&mut state.watch_text)
            .font(egui::TextStyle::Monospace)
            .hint_text(ADD_WATCH)
            .desired_width(240.0),
    );
    field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, ADD_WATCH));
    if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
        let typed = state.watch_text.trim().to_owned();
        if !typed.is_empty() {
            state.watches.push(typed);
            state.watch_text.clear();
        }
    }
    ask
}

fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(TEXT_MUTED));
}

/// Variable rows as the Locals and Watch panes draw them: indented by
/// depth, a chevron on a value that opens, name (a parameter's in the
/// accent colour), value and type. A click opens or closes a value; a
/// value in a module's memory offers its references.
pub(crate) fn variable_rows(
    ui: &mut egui::Ui,
    rows: &[VariableRow],
    names: Namer<'_>,
    index: &SymbolIndex,
) -> Option<RowAsk> {
    let mut ask = None;
    let indent = |row: &VariableRow| usize::from(row.depth).saturating_mul(INDENT_CHARS);
    let name_chars =
        rows.iter().map(|row| indent(row) + row.name.chars().count()).max().unwrap_or(0).min(NAME_CHARS_MAX);
    let type_chars = rows.iter().map(|row| row.type_name.chars().count()).max().unwrap_or(0).min(TYPE_CHARS_MAX);
    let character = char_width(ui);
    for row in rows {
        match row.kind {
            RowKind::MoreElements { shown, of } => {
                ui.horizontal(|ui| {
                    ui.add_space(CHEVRON_LEAD + indent(row) as f32 * character);
                    if ui.small_button(MORE_ELEMENTS).clicked() {
                        ask = Some(RowAsk::MorePages(row.path.clone()));
                    }
                    ui.label(RichText::new(format!("{shown} of {of} listed")).color(TEXT_MUTED));
                });
                continue;
            }
            RowKind::Ceiling => {
                list_row(ui, CHEVRON_LEAD, &[Cell::fill(CEILING, ACCENT_AMBER)], Tone::Plain, RowSense::Look);
                continue;
            }
            RowKind::Parameter
            | RowKind::Local
            | RowKind::Static
            | RowKind::Member
            | RowKind::Element
            | RowKind::Target
            | RowKind::Watch => {}
        }
        let name_color = match row.kind {
            RowKind::Parameter => ACCENT_BLUE,
            RowKind::Local
            | RowKind::Static
            | RowKind::Member
            | RowKind::Element
            | RowKind::Target
            | RowKind::Watch
            | RowKind::MoreElements { .. }
            | RowKind::Ceiling => TEXT_PRIMARY,
        };
        let value = value_text(&row.value, names);
        // The whole value, however narrow the pane cuts its cell.
        let whole = format!("{} = {value}\n{}", row.name, row.type_name);
        let cells = [
            Cell::left(format!("{}{}", " ".repeat(indent(row)), row.name), name_color, name_chars),
            Cell::fill(value, value_color(&row.value)),
            Cell::left(row.type_name.clone(), TEXT_MUTED, type_chars),
        ];
        let response =
            list_row(ui, CHEVRON_LEAD, &cells, Tone::Plain, RowSense::Pick { selected: false }).on_hover_text(whole);
        let chevron = response.rect.translate(egui::vec2(indent(row) as f32 * character, 0.0));
        match row.expand {
            Expand::Leaf => {}
            Expand::Closed => lead_chevron(ui, chevron, false, TEXT_MUTED),
            Expand::Open => lead_chevron(ui, chevron, true, TEXT_MUTED),
        }
        if response.clicked() && row.expand != Expand::Leaf {
            ask = Some(RowAsk::Toggle(row.path.clone()));
        }
        let in_module = row.memory.filter(|memory| index.module_at(memory.address).is_some());
        if let Some(memory) = in_module {
            response.context_menu(|ui| {
                if ui.button(format!("{REFERENCES_TO} {}", row.name)).clicked() {
                    let range = AddressRange { start: memory.address, end: memory.address.wrapping_add(memory.size) };
                    ask = Some(RowAsk::References(XrefTarget { ranges: vec![range], label: row.name.clone() }));
                    ui.close();
                }
            });
        }
    }
    ask
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::inspect::SymbolIndex;
    use crate::debugger::unwind::{FrameFound, FrameRegisters};
    use crate::debugger::variables::{PointedString, Preview, PreviewItem, ShownValue, Signedness, StringEnd};
    use crate::symbols::hub::SymbolsView;
    use crate::symbols::kernel::tests::{kernel_symbols, KERNEL_BASE};
    use crate::symbols::names::tests::{hub_with_kernel_code, loaded};

    fn text(value: ShownValue) -> String {
        let index = SymbolIndex::default();
        let view = SymbolsView::default();
        value_text(&value, Namer::new(&index, &view))
    }

    fn int(raw: u64, size: u8, signedness: Signedness) -> ShownValue {
        ShownValue::Integer { raw, size, signedness }
    }

    fn named(name: &str, value: ShownValue) -> PreviewItem {
        PreviewItem { name: Some(name.into()), value }
    }

    fn element(value: ShownValue) -> PreviewItem {
        PreviewItem { name: None, value }
    }

    #[test]
    fn values_read_as_the_spec_writes_them() {
        assert_eq!(text(int(0xFFFF_FFFB, 4, Signedness::Signed)), "-5 (0xfffffffb)");
        assert_eq!(text(int(42, 4, Signedness::Unsigned)), "42 (0x2a)");
        assert_eq!(text(int(u64::MAX, 8, Signedness::Signed)), "-1 (0xffffffffffffffff)");
        assert_eq!(text(ShownValue::Character { code: 65, size: 1 }), "'A' (65)");
        assert_eq!(text(ShownValue::Character { code: 0xE9, size: 1 }), r"'\xE9' (233)");
        assert_eq!(text(ShownValue::Boolean { raw: 0 }), "false");
        assert_eq!(text(ShownValue::Boolean { raw: 1 }), "true");
        assert_eq!(text(ShownValue::Boolean { raw: 2 }), "true (0x02)");
        assert_eq!(text(ShownValue::Real32(1.5)), "1.5");
        assert_eq!(text(ShownValue::Real64(0.1)), "0.1");
        assert_eq!(text(ShownValue::Hresult(0x8000_4005)), "0x80004005");
        assert_eq!(
            text(ShownValue::Pointer {
                address: 0x7FF6_1234_0010,
                string: Some(PointedString { text: "hello".into(), end: StringEnd::Terminated }),
            }),
            "0x00007ff6`12340010 \"hello\""
        );
        assert_eq!(
            text(ShownValue::Pointer { address: 0x40_1000, string: Some(PointedString { text: "hel".into(), end: StringEnd::Cut }) }),
            "0x00401000 \"hel…\""
        );
        assert_eq!(
            text(ShownValue::Pointer { address: 0x40_1000, string: Some(PointedString { text: "x".into(), end: StringEnd::Unreadable }) }),
            "0x00401000 \"x…\""
        );
        assert_eq!(text(ShownValue::Enum { raw: 3, names: vec!["Both".into()] }), "Both");
        assert_eq!(text(ShownValue::Enum { raw: 5, names: vec!["A".into(), "C".into()] }), "A | C");
        assert_eq!(text(ShownValue::Enum { raw: -8, names: Vec::new() }), "-8");
        let flags = ShownValue::Aggregate {
            count: None,
            preview: vec![
                named("Lock", int(0, 4, Signedness::Unsigned)),
                named("VadType", int(3, 4, Signedness::Unsigned)),
                named("Protection", int(31, 4, Signedness::Unsigned)),
                named("PrivateMemory", int(0, 4, Signedness::Unsigned)),
            ],
            more: Preview::Whole,
        };
        assert_eq!(text(flags), "{Lock=0, VadType=3, Protection=31, PrivateMemory=0}");
        let wide = ShownValue::Aggregate {
            count: None,
            preview: vec![
                named("a", int(1, 4, Signedness::Signed)),
                named("b", int(2, 4, Signedness::Signed)),
                named("c", int(3, 4, Signedness::Signed)),
                named("d", int(4, 4, Signedness::Signed)),
                named("e", int(5, 4, Signedness::Signed)),
                named("f", int(6, 4, Signedness::Signed)),
            ],
            more: Preview::Cut,
        };
        assert_eq!(text(wide), "{a=1, b=2, c=3, d=4, e=5, f=6, …}");
        let array = ShownValue::Aggregate {
            count: Some(250),
            preview: (1..=6).map(|value| element(int(value, 4, Signedness::Signed))).collect(),
            more: Preview::Cut,
        };
        assert_eq!(text(array), "[250] [1, 2, 3, 4, 5, 6, …]");
        let nested = ShownValue::Aggregate {
            count: None,
            preview: vec![
                named("inner", ShownValue::Aggregate { count: None, preview: Vec::new(), more: Preview::Cut }),
                named("rows", ShownValue::Aggregate { count: Some(4), preview: Vec::new(), more: Preview::Cut }),
                named("next", ShownValue::Pointer { address: 0x1000, string: None }),
            ],
            more: Preview::Whole,
        };
        assert_eq!(text(nested), "{inner={…}, rows=[…], next=0x00001000}");
        assert_eq!(text(ShownValue::NotAvailable), "not available here");
        assert_eq!(text(ShownValue::OptimizedAway), "optimized away");
        assert_eq!(text(ShownValue::Unreadable { at: 0x5000 }), "memory at 0x5000 is not readable");
        assert_eq!(text(ShownValue::UnknownType { index: 0x1234, leaf: 0x1515, bytes: vec![1, 2, 3] }), "<type 0x1234> 01 02 03");
        assert_eq!(text(ShownValue::Problem("no variable named x".into())), "no variable named x");
    }

    #[test]
    fn a_pointer_into_a_module_is_named() {
        let hub = hub_with_kernel_code(loaded());
        let view = hub.view();
        let index = kernel_symbols();
        let value = ShownValue::Pointer { address: KERNEL_BASE + 0x1044, string: None };
        let shown = value_text(&value, Namer::new(&index, &view));
        assert!(shown.ends_with(" ntoskrnl!KiSystemCall64+0x4"), "{shown}");
    }

    #[test]
    fn stack_ends_read_as_the_spec_writes_them() {
        assert_eq!(stack_end_text(StackEnd::Top), None);
        let cases = [
            Ended { end: StackEnd::NoUnwindInfo { rip: 0x1234 }, text: "stack ends: no unwind information for 0x1234" },
            Ended { end: StackEnd::BadUnwindInfo { at: 0x2000 }, text: "stack ends: unwind information at 0x2000 does not read" },
            Ended { end: StackEnd::RegisterNotKnown { register: 3 }, text: "stack ends: a register this frame needs is not known" },
            Ended { end: StackEnd::NoFramePointer, text: "frame not unwound: no frame pointer" },
            Ended { end: StackEnd::NotGrowing, text: "stack does not unwind further" },
            Ended { end: StackEnd::Unreadable { at: 0x7000 }, text: "stack ends: memory at 0x7000 is not readable" },
            Ended { end: StackEnd::Ceiling, text: "stack ends: 256 frames shown" },
            Ended { end: StackEnd::SixteenBit, text: "16-bit code: the stack is not unwound" },
        ];
        for case in cases {
            assert_eq!(stack_end_text(case.end).as_deref(), Some(case.text));
        }
    }

    /// What a stack end reads as.
    struct Ended {
        end: StackEnd,
        text: &'static str,
    }

    #[test]
    fn the_lower_panes_follow_the_source_until_picked() {
        assert_eq!(LowerLeft::shown(None, true), LowerLeft::Locals);
        assert_eq!(LowerLeft::shown(None, false), LowerLeft::Memory);
        assert_eq!(LowerLeft::shown(Some(LowerLeft::Watch), false), LowerLeft::Watch);
        assert_eq!(LowerLeft::shown(Some(LowerLeft::Memory), true), LowerLeft::Memory);
        assert_eq!(LowerRight::shown(None, true), LowerRight::CallStack);
        assert_eq!(LowerRight::shown(None, false), LowerRight::Stack);
        assert_eq!(LowerRight::shown(Some(LowerRight::Stack), true), LowerRight::Stack);
    }

    #[test]
    fn a_callers_label_names_the_call_it_returns_from() {
        let hub = hub_with_kernel_code(loaded());
        let view = hub.view();
        let index = kernel_symbols();
        let names = Namer::new(&index, &view);
        let innermost = Frame {
            registers: FrameRegisters { rip: KERNEL_BASE + 0x1040, rsp: 0, registers: [None; 16] },
            how: FrameFound::Registers,
        };
        assert_eq!(frame_label(&innermost, names), "ntoskrnl!KiSystemCall64");
        let caller = Frame {
            registers: FrameRegisters { rip: KERNEL_BASE + 0x1051, rsp: 0, registers: [None; 16] },
            how: FrameFound::Unwind,
        };
        assert_eq!(frame_label(&caller, names), "ntoskrnl!KiSystemCall64+0x11");
        assert_eq!(frame_line(&caller, &index, &view).as_deref(), Some("trap.asm:104"));
        let nowhere = Frame { registers: FrameRegisters { rip: 0x1234, rsp: 0, registers: [None; 16] }, how: FrameFound::Leaf };
        assert_eq!(frame_label(&nowhere, names), "0x1234");
        assert_eq!(frame_line(&nowhere, &index, &view), None);
    }
}
