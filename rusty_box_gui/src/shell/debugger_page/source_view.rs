//! The CPU tab's source view (spec §6): the owner's file around the line
//! the pause is at, a breakpoint gutter, and a header saying how the file
//! was found and whether it is the one the module was built from.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use rusty_box::cpu::CodeBreakpoint;
use rusty_box_symbols::service::SymbolSource;
use rusty_box_symbols::source_path::{components, ends_with, trailing_components_in_common};
use rusty_box_symbols::tables::SymbolTables;
use rusty_box_symbols::units::Rva;

use egui::{Color32, RichText};

use crate::debugger::inspect::SymbolIndex;
use crate::debugger::source_breakpoints::{resolve, BreakpointModules, BreakpointTarget, SourceResolution, SourceTarget};
use crate::debugger::xref::service::{AddressRange, XrefTarget};
use super::references::REFERENCES_TO;
use super::frames_view::{variable_rows, RowAsk, CALLER_LINE, TIP_READING, VALUE_TIP};
use crate::debugger::frames::TipRead;
use crate::debugger::watch::expression_at;
use crate::shell::listing::{
    char_at, char_width, char_x, lead_arrow, lead_dot, lead_hollow_arrow, lead_ring, list_row, Cell, RowSense, Tone,
    ROW_PAD,
};
use crate::symbols::names::Namer;
use crate::shell::theme::{ACCENT_AMBER, ACCENT_CYAN, ACCENT_RED, TEXT_MUTED, TEXT_PRIMARY};
use crate::source::find::{FoundBy, PathMap, SourceCheck, SourceFound};
use crate::symbols::hub::{ModuleSymbols, SourceKey, SourceSlot, SymbolsView};

/// A code address as a place in the owner's source: the file (as the
/// module's tables name it) and line, and the tables that say so.
#[derive(Clone)]
pub(crate) struct PauseSource {
    pub(crate) key: SourceKey,
    pub(crate) line: u32,
    /// The path the build recorded.
    pub(crate) build_path: String,
    pub(crate) module_base: u64,
    pub(crate) tables: Arc<SymbolTables>,
    /// The project the module's symbols came from, when one did.
    pub(crate) project: Option<String>,
}

/// `address` as a source place, when a module of `index` holds it and its
/// tables have a line row for it.
pub(crate) fn source_at(address: u64, index: &SymbolIndex, view: &SymbolsView) -> Option<PauseSource> {
    let module = index.module_at(address)?;
    let identity = module.identity.known()?;
    let tables = Arc::clone(view.tables(identity)?);
    let row = tables.line_at(Rva(address.checked_sub(module.base)?))?;
    let build_path = tables.files().get(row.file)?.path.clone();
    let project = match view.state(identity) {
        Some(ModuleSymbols::Loaded { from: SymbolSource::Project { name, .. } }) => Some(name.clone()),
        Some(_) | None => None,
    };
    Some(PauseSource {
        key: SourceKey { identity: identity.clone(), file: row.file },
        line: row.line,
        build_path,
        module_base: module.base,
        tables,
        project,
    })
}

/// A gutter mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineMark {
    /// A breakpoint the machine holds on the line's code.
    Set,
    /// A source breakpoint on the line that resolves nowhere yet.
    Pending,
}

/// A line's gutter mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineMarkAt {
    pub(crate) line: u32,
    pub(crate) mark: LineMark,
}

/// The line `address` is code of in `source`'s file, when it is.
fn line_of(source: &PauseSource, address: u64) -> Option<u32> {
    let row = source.tables.line_at(Rva(address.checked_sub(source.module_base)?))?;
    (row.file == source.key.file).then_some(row.line)
}

/// Whether a source target names `source`'s file.
fn names_file(target: &SourceTarget, source: &PauseSource) -> bool {
    ends_with(&source.build_path, &target.file)
}

/// The gutter's marks for `source`'s file, by line: a line whose code holds
/// a breakpoint the machine has, and a source breakpoint on the file that
/// resolves nowhere.
pub(crate) fn line_marks(
    source: &PauseSource,
    applied: &[CodeBreakpoint],
    targets: &[BreakpointTarget],
    index: Option<BreakpointModules<'_>>,
    view: &SymbolsView,
) -> Vec<LineMarkAt> {
    let mut marks: BTreeMap<u32, LineMark> = BTreeMap::new();
    for breakpoint in applied {
        if let CodeBreakpoint::Linear(address) = breakpoint {
            if let Some(line) = line_of(source, *address) {
                marks.insert(line, LineMark::Set);
            }
        }
    }
    for target in targets {
        let BreakpointTarget::Source(target) = target else {
            continue;
        };
        let pending = !index.is_some_and(|index| matches!(resolve(target, &index, view), SourceResolution::Resolved { .. }));
        if pending && names_file(target, source) {
            marks.entry(target.line).or_insert(LineMark::Pending);
        }
    }
    marks.into_iter().map(|(line, mark)| LineMarkAt { line, mark }).collect()
}

/// The owner's breakpoints after a gutter click on `line` of `source`'s
/// file: what made the line's mark (a source breakpoint on it, an address
/// in its code) is removed; a line without one gets a source breakpoint by
/// the file's full build path.
pub(crate) fn toggled_line(
    targets: &[BreakpointTarget],
    source: &PauseSource,
    line: u32,
    index: Option<BreakpointModules<'_>>,
    view: &SymbolsView,
) -> Vec<BreakpointTarget> {
    let marks_line = |target: &BreakpointTarget| match target {
        BreakpointTarget::Source(target) => {
            names_file(target, source)
                && (target.line == line
                    || index.is_some_and(|index| match resolve(target, &index, view) {
                        SourceResolution::Resolved { places } => places.iter().any(|place| line_of(source, place.address) == Some(line)),
                        SourceResolution::Pending(_) => false,
                    }))
        }
        BreakpointTarget::Address(CodeBreakpoint::Linear(address)) => line_of(source, *address) == Some(line),
        BreakpointTarget::Address(CodeBreakpoint::Physical(_)) => false,
    };
    let kept: Vec<BreakpointTarget> = targets.iter().filter(|target| !marks_line(target)).cloned().collect();
    if kept.len() < targets.len() {
        return kept;
    }
    let mut set = targets.to_vec();
    set.push(BreakpointTarget::Source(SourceTarget { file: source.build_path.clone(), line }));
    set
}

/// Where `line` of `source`'s file starts: its first code range, or the
/// next line's when it has none.
pub(crate) fn line_address(source: &PauseSource, line: u32) -> Option<u64> {
    let resolved = source.tables.lines_of(&source.build_path, line);
    let first = resolved.iter().flat_map(|resolved| resolved.ranges.iter()).find(|range| range.file == source.key.file)?;
    Some(source.module_base.wrapping_add(first.start.0))
}

/// The map a located file makes: the build path's folders above the tail it
/// shares with `found` become `found`'s; with no shared tail (the file was
/// renamed), the one file maps to the other.
pub(crate) fn located_map(build_path: &str, found: &Path) -> PathMap {
    let shared = trailing_components_in_common(build_path, &found.to_string_lossy());
    let parts: Vec<&str> = components(build_path).collect();
    let kept = parts.len().saturating_sub(shared);
    let whole = || PathMap { from: build_path.to_owned(), to: found.to_path_buf() };
    if shared == 0 || kept == 0 {
        return whole();
    }
    let Some(to) = found.ancestors().nth(shared) else {
        return whole();
    };
    let separator = if build_path.contains('\\') { "\\" } else { "/" };
    let mut from = parts.get(..kept).map(|parts| parts.join(separator)).unwrap_or_default();
    if build_path.starts_with('/') {
        from.insert(0, '/');
    }
    PathMap { from, to: to.to_path_buf() }
}

/// What the code pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodeView {
    Source,
    Disassembly,
}

impl CodeView {
    /// The owner's pick, or, before one, the source wherever the pause has
    /// a line.
    pub(crate) fn shown(picked: Option<CodeView>, has_source: bool) -> CodeView {
        match picked {
            Some(view) => view,
            None if has_source => CodeView::Source,
            None => CodeView::Disassembly,
        }
    }
}

pub(crate) const SOURCE_SWITCH: &str = "Source";
pub(crate) const DISASSEMBLY_SWITCH: &str = "Disassembly";
pub(crate) const SOURCE_READING: &str = "Reading the source…";
pub(crate) const SOURCE_DIFFERS: &str = "This file differs from the one the module was built from.";
pub(crate) const SOURCE_COMPRESSED: &str =
    "The PDB holds this file compressed, in a form this debugger does not undo.";
pub(crate) const NO_SOURCE_HERE: &str = "No source line for this address: the disassembly is shown.";
pub(crate) const LOCATE: &str = "Locate…";
pub(crate) const SAVE_TO_SETTINGS: &str = "Save to settings";
/// Lines shown above the current one, and in all.
const LINES_ABOVE: u32 = 12;
const LINES_SHOWN: u32 = 80;
/// The gutter: breakpoint marks and the current-line arrow.
const GUTTER: f32 = 12.0;

/// What a click in the source view asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceAsk {
    /// The gutter of this line was clicked.
    Toggle(u32),
    Locate,
    /// Keep this map the owner made by locating the file in its project's
    /// settings.
    SaveMap { project: String, map: PathMap },
    /// Who refers to this line's code.
    References(u32),
    /// A click on a name: show its value.
    ShowTip(SourceTip),
    /// Escape, a click outside the tip, or a word that names nothing.
    CloseTip,
    /// A row of the tip was clicked.
    TipRow(RowAsk),
}

/// A value tip: the expression a click on line `line`, character `column`,
/// named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceTip {
    pub(crate) expression: String,
    pub(crate) line: u32,
    pub(crate) column: usize,
}

/// The open tip and what the pause read of it, with what names its values.
pub(crate) struct TipView<'a> {
    pub(crate) tip: &'a SourceTip,
    pub(crate) read: &'a TipRead,
    pub(crate) names: Namer<'a>,
    pub(crate) index: &'a SymbolIndex,
}

/// The cells of a source line, as `source_rows` lays one out.
fn line_cells(number: String, digits: usize, text: String, color: Color32) -> [Cell; 2] {
    [Cell::right(number, TEXT_MUTED, digits), Cell::fill(text, color)]
}

/// Where character `column` of a source line in `row` starts, its numbers
/// `digits` wide, `char_width` a character.
pub(crate) fn line_char_x(row: egui::Rect, digits: usize, char_width: f32, column: usize) -> Option<f32> {
    let cells = line_cells(String::new(), digits, String::new(), TEXT_PRIMARY);
    char_x(row, GUTTER, &cells, char_width, 1, column)
}

fn line_char_at(row: egui::Rect, digits: usize, char_width: f32, x: f32) -> Option<usize> {
    let cells = line_cells(String::new(), digits, String::new(), TEXT_PRIMARY);
    char_at(row, GUTTER, &cells, char_width, 1, x)
}

/// Every code range of `line` of `source`'s file, as a cross-reference
/// target named `file:line`; `None` when the line has no code.
pub(crate) fn line_target(source: &PauseSource, line: u32) -> Option<XrefTarget> {
    let ranges: Vec<AddressRange> = source
        .tables
        .lines_of(&source.build_path, line)
        .iter()
        .flat_map(|resolved| resolved.ranges.iter())
        .filter(|range| range.file == source.key.file)
        .map(|range| AddressRange {
            start: source.module_base.wrapping_add(range.start.0),
            end: source.module_base.wrapping_add(range.end.0),
        })
        .collect();
    (!ranges.is_empty()).then(|| XrefTarget { ranges, label: format!("{}:{line}", file_name(&source.build_path)) })
}

/// What the source view draws from.
pub(crate) struct SourcePane<'a> {
    pub(crate) source: &'a PauseSource,
    pub(crate) slot: Option<&'a SourceSlot>,
    pub(crate) marks: &'a [LineMarkAt],
    /// Whether `source`'s line is where the processor stands, or a place the
    /// owner opened.
    pub(crate) place: PlaceMark,
    /// The value tip, when one is open.
    pub(crate) tip: Option<TipView<'a>>,
    /// The line is new to the pane: it is scrolled into view this frame.
    pub(crate) follow: bool,
}

/// What the source view's line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceMark {
    /// The line the processor runs next: the arrow.
    Current,
    /// A line the owner opened (a cross reference, a Go to): ringed, since
    /// the arrow means RIP.
    Shown,
    /// The call a picked caller frame returns from: the hollow arrow.
    Caller,
}

/// What a line's gutter draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lead {
    Arrow(Color32),
    /// The hollow arrow of a caller frame's line.
    Hollow,
    /// The ring of a line the owner opened.
    Shown,
    Dot(Color32),
    Ring(Color32),
    Nothing,
}

/// The gutter of a line: `current` is whether it is the view's line.
pub(crate) fn line_lead(current: bool, place: PlaceMark, mark: Option<LineMark>) -> Lead {
    match (current, place, mark) {
        (true, PlaceMark::Shown, _) => Lead::Shown,
        (true, PlaceMark::Caller, _) => Lead::Hollow,
        (true, PlaceMark::Current, Some(LineMark::Set)) => Lead::Arrow(ACCENT_RED),
        (true, PlaceMark::Current, Some(LineMark::Pending) | None) => Lead::Arrow(ACCENT_CYAN),
        (false, _, Some(LineMark::Set)) => Lead::Dot(ACCENT_RED),
        (false, _, Some(LineMark::Pending)) => Lead::Ring(ACCENT_RED),
        (false, _, None) => Lead::Nothing,
    }
}

pub(crate) fn file_name(path: &str) -> &str {
    components(path).last().unwrap_or(path)
}

fn found_by(by: FoundBy) -> &'static str {
    match by {
        FoundBy::Located => "located this run",
        FoundBy::PathMap => "by the project's path map",
        FoundBy::RecordedPath => "where the build put it",
        FoundBy::SourceFolder => "in the project's source folders",
    }
}

/// How a check reads in the header.
struct CheckShown {
    text: &'static str,
    color: Color32,
}

fn check_shown(check: SourceCheck) -> CheckShown {
    match check {
        SourceCheck::Matches => CheckShown { text: "matches the build", color: TEXT_MUTED },
        SourceCheck::Differs => CheckShown { text: "differs from the build", color: ACCENT_AMBER },
        SourceCheck::NoChecksum => CheckShown { text: "no checksum recorded: not checked", color: TEXT_MUTED },
    }
}

/// The source view: a header saying how the file was found (or why it is
/// not shown), then its lines around the current one. Clicking a line picks
/// it; clicking its gutter toggles its breakpoint.
pub(crate) fn source_rows(ui: &mut egui::Ui, pane: &SourcePane<'_>, cursor: &mut Option<u32>) -> Option<SourceAsk> {
    let mut ask = None;
    let loaded = match pane.slot {
        Some(SourceSlot::Read(loaded)) => loaded,
        Some(SourceSlot::Reading) | None => {
            ui.label(RichText::new(SOURCE_READING).color(TEXT_MUTED));
            return None;
        }
    };
    let name = file_name(&pane.source.build_path);
    let locate = |ui: &mut egui::Ui, ask: &mut Option<SourceAsk>| {
        if ui.button(LOCATE).clicked() {
            *ask = Some(SourceAsk::Locate);
        }
    };
    match &loaded.found {
        SourceFound::Disk { path, by, check } => {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(name).strong());
                ui.label(RichText::new(format!("· {}", found_by(*by))).color(TEXT_MUTED));
                let shown = check_shown(*check);
                ui.label(RichText::new(format!("· {}", shown.text)).color(shown.color));
                if *by == FoundBy::Located {
                    if let Some(project) = &pane.source.project {
                        if ui.button(SAVE_TO_SETTINGS).clicked() {
                            let map = located_map(&pane.source.build_path, path);
                            ask = Some(SourceAsk::SaveMap { project: project.clone(), map });
                        }
                    }
                }
            })
            .response
            .on_hover_text(path.display().to_string());
            if *check == SourceCheck::Differs {
                ui.label(RichText::new(SOURCE_DIFFERS).color(ACCENT_AMBER));
            }
        }
        SourceFound::Embedded { check } => {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(name).strong());
                ui.label(RichText::new("· embedded in the PDB").color(TEXT_MUTED));
                let shown = check_shown(*check);
                ui.label(RichText::new(format!("· {}", shown.text)).color(shown.color));
            });
        }
        SourceFound::EmbeddedCompressed { .. } => {
            ui.label(RichText::new(SOURCE_COMPRESSED).color(ACCENT_AMBER));
            locate(ui, &mut ask);
        }
        SourceFound::Ambiguous { candidates } => {
            ui.label(RichText::new(format!("Several files match {name}; none is picked:")).color(ACCENT_AMBER));
            for candidate in candidates {
                ui.label(RichText::new(candidate.display().to_string()).monospace().color(TEXT_MUTED));
            }
            locate(ui, &mut ask);
        }
        SourceFound::Missing { build_path } => {
            ui.label(RichText::new(format!("Source not found: {build_path}")).color(ACCENT_AMBER));
            locate(ui, &mut ask);
        }
        SourceFound::Unreadable { path, reason } => {
            ui.label(RichText::new(format!("{} does not read: {reason}", path.display())).color(ACCENT_AMBER));
            locate(ui, &mut ask);
        }
    }
    let Some(text) = &loaded.text else {
        return ask;
    };
    let code = pane.source.tables.code_lines(pane.source.key.file);
    let first = pane.source.line.saturating_sub(LINES_ABOVE).max(1);
    let last = first.saturating_add(LINES_SHOWN);
    let digits = last.to_string().len();
    let character = char_width(ui);
    // Where the open tip hangs: under its word, on its line's row.
    let mut tip_anchor = None;
    for number in first..last {
        let Some(line) = usize::try_from(number).ok().and_then(|number| number.checked_sub(1)).and_then(|at| text.lines.get(at))
        else {
            break;
        };
        let current = number == pane.source.line;
        let has_code = code.binary_search(&number).is_ok();
        let color = if current {
            ACCENT_CYAN
        } else if has_code {
            TEXT_PRIMARY
        } else {
            TEXT_MUTED
        };
        let picked = *cursor == Some(number);
        let tone = if picked {
            Tone::Selected
        } else if current {
            Tone::Current
        } else {
            Tone::Plain
        };
        let shown = line.replace('\t', "    ");
        let cells = line_cells(number.to_string(), digits, shown.clone(), color);
        let response = list_row(ui, GUTTER, &cells, tone, RowSense::Pick { selected: picked });
        if current && pane.follow {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        if let Some(view) = pane.tip.as_ref().filter(|view| view.tip.line == number) {
            let x = line_char_x(response.rect, digits, character, view.tip.column).unwrap_or(response.rect.left());
            tip_anchor = Some(egui::pos2(x, response.rect.bottom()));
        }
        let mark = pane.marks.iter().find(|at| at.line == number).map(|at| at.mark);
        match line_lead(current, pane.place, mark) {
            Lead::Arrow(color) => lead_arrow(ui, response.rect, color),
            Lead::Hollow => lead_hollow_arrow(ui, response.rect, ACCENT_CYAN, CALLER_LINE),
            Lead::Shown => lead_ring(ui, response.rect, GUTTER, ACCENT_CYAN),
            Lead::Dot(color) => lead_dot(ui, response.rect, GUTTER, color),
            Lead::Ring(color) => lead_ring(ui, response.rect, GUTTER, color),
            Lead::Nothing => {}
        }
        response.context_menu(|ui| {
            if ui.button(format!("{REFERENCES_TO} line {number}")).clicked() {
                ask = Some(SourceAsk::References(number));
                ui.close();
            }
        });
        if response.clicked() {
            let in_gutter = response
                .interact_pointer_pos()
                .is_some_and(|at| at.x < response.rect.left() + ROW_PAD + GUTTER);
            if in_gutter {
                ask = Some(SourceAsk::Toggle(number));
            } else {
                *cursor = Some(number);
                let clicked = response
                    .interact_pointer_pos()
                    .and_then(|at| line_char_at(response.rect, digits, character, at.x))
                    .and_then(|column| expression_at(&shown, column).map(|expression| SourceTip { expression, line: number, column }));
                if let Some(tip) = clicked {
                    ask = Some(SourceAsk::ShowTip(tip));
                }
            }
        }
    }
    if let Some(view) = &pane.tip {
        let tip_ask = tip_popup(ui, view, tip_anchor);
        if ask.is_none() {
            ask = tip_ask;
        }
    }
    ask
}

/// The widest a value tip grows, points.
const TIP_WIDTH: f32 = 520.0;

/// The value tip under its word (`anchor`, where its line is drawn): the
/// expression, then its rows as the Locals pane draws them. Escape, a click
/// outside it, or a word that names nothing closes it.
fn tip_popup(ui: &mut egui::Ui, view: &TipView<'_>, anchor: Option<egui::Pos2>) -> Option<SourceAsk> {
    if let TipRead::NoVariable { text } = view.read {
        if *text == view.tip.expression {
            return Some(SourceAsk::CloseTip);
        }
    }
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        return Some(SourceAsk::CloseTip);
    }
    let anchor = anchor?;
    let mut ask = None;
    let area = egui::Area::new(ui.id().with(VALUE_TIP)).order(egui::Order::Foreground).fixed_pos(anchor).show(ui.ctx(), |ui| {
        let frame = egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_max_width(TIP_WIDTH);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(&view.tip.expression).monospace().strong());
            ui.add_space(4.0);
            match view.read {
                TipRead::Rows(read) if read.text == view.tip.expression => {
                    if let Some(row) = variable_rows(ui, &read.rows, view.names, view.index) {
                        ask = Some(SourceAsk::TipRow(row));
                    }
                }
                TipRead::Rows(_) | TipRead::None | TipRead::NoVariable { .. } => {
                    ui.label(RichText::new(TIP_READING).color(TEXT_MUTED));
                }
            }
        });
        frame.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, VALUE_TIP));
    });
    let clicked_outside = ui.input(|input| {
        input.pointer.primary_clicked() && input.pointer.interact_pos().is_some_and(|at| !area.response.rect.contains(at))
    });
    if ask.is_none() && clicked_outside {
        ask = Some(SourceAsk::CloseTip);
    }
    ask
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::kernel::tests::{kernel_identity, kernel_symbols, KERNEL_BASE};
    use std::path::PathBuf;
    use crate::symbols::names::tests::{hub_with_kernel_code, loaded};

    const TRAP: &str = r"d:\os\src\ntos\ke\amd64\trap.asm";

    fn at_line_100() -> PauseSource {
        let view = hub_with_kernel_code(loaded()).view();
        source_at(KERNEL_BASE + 0x1044, &kernel_symbols(), &view).expect("a line")
    }

    #[test]
    fn a_pause_in_code_with_lines_is_a_source_place() {
        let source = at_line_100();
        assert_eq!(source.line, 100);
        assert_eq!(source.build_path, TRAP);
        assert_eq!(source.key, SourceKey { identity: kernel_identity(), file: 0 });
        assert_eq!(source.module_base, KERNEL_BASE);
        let view = hub_with_kernel_code(loaded()).view();
        assert_eq!(source_at(KERNEL_BASE + 0x1100, &kernel_symbols(), &view).map(|source| source.line), None, "a public has no lines");
        assert!(source_at(0x1234, &kernel_symbols(), &view).is_none(), "outside every module");
    }

    fn trap_line(line: u32) -> BreakpointTarget {
        BreakpointTarget::Source(SourceTarget { file: "trap.asm".into(), line })
    }

    /// No module of the owner's runs in another process.
    static NOWHERE: crate::debugger::windows::ModulesElsewhere =
        crate::debugger::windows::ModulesElsewhere { current: None, modules: Vec::new() };

    /// A pause's modules: `symbols`, and nothing in other processes.
    fn only(symbols: &SymbolIndex) -> BreakpointModules<'_> {
        BreakpointModules { index: symbols, elsewhere: &NOWHERE }
    }

    #[test]
    fn lines_with_breakpoints_are_marked_set_or_pending() {
        let view = hub_with_kernel_code(loaded()).view();
        let symbols = kernel_symbols();
        let source = at_line_100();
        // Line 104 by its address, line 100 by source, line 300 pending.
        let applied = [CodeBreakpoint::Linear(KERNEL_BASE + 0x1050), CodeBreakpoint::Linear(KERNEL_BASE + 0x1040)];
        let targets = [trap_line(100), BreakpointTarget::Address(applied[0]), trap_line(300)];
        let marks = line_marks(&source, &applied, &targets, Some(only(&symbols)), &view);
        assert_eq!(
            marks.into_iter().collect::<Vec<LineMarkAt>>(),
            [
                LineMarkAt { line: 100, mark: LineMark::Set },
                LineMarkAt { line: 104, mark: LineMark::Set },
                LineMarkAt { line: 300, mark: LineMark::Pending },
            ]
        );
    }

    /// A gutter click on a line with a breakpoint removes whatever made it
    /// (a source line or an address in it); on a line without, it adds the
    /// line by its full build path.
    #[test]
    fn a_gutter_click_toggles_the_lines_breakpoint() {
        let view = hub_with_kernel_code(loaded()).view();
        let symbols = kernel_symbols();
        let source = at_line_100();
        let address = BreakpointTarget::Address(CodeBreakpoint::Linear(KERNEL_BASE + 0x1050));
        assert_eq!(toggled_line(&[trap_line(100)], &source, 100, Some(only(&symbols)), &view), []);
        assert_eq!(toggled_line(&[address.clone()], &source, 104, Some(only(&symbols)), &view), []);
        let added = BreakpointTarget::Source(SourceTarget { file: TRAP.into(), line: 104 });
        assert_eq!(toggled_line(&[trap_line(100)], &source, 104, Some(only(&symbols)), &view), [trap_line(100), added]);
    }

    /// The shown line of a Go to is ringed, not arrowed: the arrow means RIP.
    #[test]
    fn the_source_view_marks_a_shown_line_not_the_current_one() {
        assert_eq!(line_lead(true, PlaceMark::Current, None), Lead::Arrow(ACCENT_CYAN));
        assert_eq!(line_lead(true, PlaceMark::Current, Some(LineMark::Set)), Lead::Arrow(ACCENT_RED));
        assert_eq!(line_lead(true, PlaceMark::Shown, None), Lead::Shown);
        assert_eq!(line_lead(true, PlaceMark::Shown, Some(LineMark::Set)), Lead::Shown);
        assert_eq!(line_lead(true, PlaceMark::Caller, Some(LineMark::Set)), Lead::Hollow, "a caller's line");
        assert_eq!(line_lead(false, PlaceMark::Caller, Some(LineMark::Set)), Lead::Dot(ACCENT_RED));
        assert_eq!(line_lead(false, PlaceMark::Shown, Some(LineMark::Set)), Lead::Dot(ACCENT_RED));
        assert_eq!(line_lead(false, PlaceMark::Current, Some(LineMark::Pending)), Lead::Ring(ACCENT_RED));
        assert_eq!(line_lead(false, PlaceMark::Current, None), Lead::Nothing);
    }

    #[test]
    fn a_lines_code_starts_at_its_first_row() {
        let source = at_line_100();
        assert_eq!(line_address(&source, 104), Some(KERNEL_BASE + 0x1050));
        assert_eq!(line_address(&source, 101), Some(KERNEL_BASE + 0x1050), "a line without code runs to the next");
        assert_eq!(line_address(&source, 900), None);
    }

    #[test]
    fn a_located_file_maps_the_folders_it_shares_a_tail_with() {
        let map = located_map(r"C:\agent\src\io\read.c", &PathBuf::from(r"D:\work\src\io\read.c"));
        assert_eq!(components(&map.from).collect::<Vec<&str>>(), ["C:", "agent"]);
        assert_eq!(map.to, PathBuf::from(r"D:\work"));
        let renamed = located_map(r"C:\a\read.c", &PathBuf::from(r"D:\b\other.c"));
        assert_eq!(renamed, PathMap { from: r"C:\a\read.c".into(), to: PathBuf::from(r"D:\b\other.c") });
    }
}
