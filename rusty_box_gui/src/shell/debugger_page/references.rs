//! The References tab (cross-references spec §6): a field taking an address
//! or `module!name`, the scope, the modules the answer is read from with
//! their index states and Reindex, and the references found, each opening in
//! the code pane.

use egui::RichText;

use crate::debugger::guest::Root;
use crate::debugger::inspect::{AddressSpace, CodeTarget, DebugSnapshot, GuestAddress, ModuleSpace, SymModule, Viewing};
use crate::debugger::xref::decode::XrefKind;
use crate::debugger::xref::index::ModuleSpan;
use crate::debugger::xref::service::{
    AddressRange, CopyModule, CopyRequest, FoundSite, IndexSpace, IndexState, ModuleKey, XrefAnswer, XrefScope, XrefTarget,
};
use crate::shell::listing::{header_row, list_row, segmented, Cell, RowSense, Segment, Tone};
use crate::shell::theme::{ACCENT_AMBER, ACCENT_BLUE, SPACE_ITEM, TEXT_MUTED, TEXT_PRIMARY};
use crate::symbols::names::{NamedAddress, Namer};

use super::{card, go_target, DebuggerAction, DebuggerPane, Density, Title};

/// The start of every "References to …" menu entry.
pub(crate) const REFERENCES_TO: &str = "References to";
pub(crate) const REFERENCES_FIELD: &str = "References to address or name";
pub(crate) const THIS_MODULE: &str = "This module";
pub(crate) const ALL_MODULES: &str = "All modules";
pub(crate) const REINDEX: &str = "Reindex";
pub(crate) const REINDEX_ALL: &str = "Reindex all";
/// A module row's menu entry in the Processes and Kernel modules lists.
pub(crate) const REINDEX_MODULE: &str = "Reindex cross references";
pub(crate) const CANCEL: &str = "Cancel";
pub(crate) const NOTHING_ASKED: &str =
    "Pick References to on a code line, a source line or an export, or type an address or module!name.";
pub(crate) const NO_MODULE_HOLDS: &str = "No module of this pause holds the target: try All modules.";
const PLACE_CHARS: usize = 36;
const KIND_CHARS: usize = 12;

/// How a reference reads in a row.
pub(crate) fn kind_text(kind: XrefKind) -> &'static str {
    match kind {
        XrefKind::Call => "call",
        XrefKind::Jump => "jump",
        XrefKind::CallThrough => "call through",
        XrefKind::JumpThrough => "jump through",
        XrefKind::Memory => "reads/writes",
        XrefKind::AddressOf => "address of",
        XrefKind::Pointer => "pointer",
    }
}

/// `value` with a comma between groups of three digits.
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The answer's first line: how many, to what, in how many modules, when the
/// indexes were taken, and what was not searched.
pub(crate) fn answer_header(answer: &XrefAnswer) -> String {
    let mut modules: Vec<&str> = answer.found.iter().map(|found| found.module.as_str()).collect();
    modules.sort_unstable();
    modules.dedup();
    let mut header = format!(
        "{} to {} in {}",
        plural(answer.found.len(), "reference", "references"),
        answer.query.target.label,
        plural(modules.len(), "module", "modules")
    );
    if let Some(taken_at) = answer.taken_at {
        header.push_str(&format!(" · indexed at the pause at instruction {}", grouped(taken_at)));
    }
    if answer.holes > 0 {
        header.push_str(&format!(" · {} not read", plural(answer.holes as usize, "page", "pages")));
    }
    if !answer.waiting.is_empty() {
        header.push_str(&format!(" · {} still indexing", plural(answer.waiting.len(), "module", "modules")));
    }
    header
}

/// A module's index state as its row says it.
pub(crate) fn state_text(state: Option<&IndexState>) -> String {
    match state {
        None => "not indexed".to_owned(),
        Some(IndexState::Copying) => "waiting to be copied".to_owned(),
        Some(IndexState::Queued) => "waiting to be indexed".to_owned(),
        Some(IndexState::Indexing { done, of }) => {
            let percent = done.saturating_mul(100).checked_div(*of).unwrap_or(0);
            format!("indexing {percent}%")
        }
        Some(IndexState::Ready(index)) => format!("indexed at instruction {}", grouped(index.taken_at)),
        Some(IndexState::Failed(reason)) => format!("failed: {reason}"),
    }
}

/// Whether a module's index is still being made.
fn busy(state: Option<&IndexState>) -> bool {
    matches!(state, Some(IndexState::Copying | IndexState::Queued | IndexState::Indexing { .. }))
}

/// The address space the pause's user modules are read in: a viewed
/// thread's process, else the shown processor's.
fn user_space(snapshot: &DebugSnapshot) -> CopySpace {
    match &snapshot.viewing {
        Viewing::Thread(thread) => CopySpace { space: IndexSpace::Process { cr3: thread.cr3 }, root: Root::Process(thread.cr3) },
        Viewing::Live | Viewing::Running { .. } | Viewing::Miss(_) => {
            let cr3 = snapshot.cpus.get(snapshot.cpu).map_or(0, |cpu| cpu.registers.cr3);
            CopySpace { space: IndexSpace::Process { cr3 }, root: Root::Live }
        }
    }
}

/// Where a module is keyed and read.
struct CopySpace {
    space: IndexSpace,
    root: Root,
}

/// `module` of `snapshot`, as it is copied and keyed.
pub(crate) fn copy_module(snapshot: &DebugSnapshot, module: &SymModule) -> CopyModule {
    let at = match module.space {
        ModuleSpace::Kernel => CopySpace { space: IndexSpace::Kernel, root: Root::Live },
        ModuleSpace::User => user_space(snapshot),
    };
    CopyModule {
        key: ModuleKey {
            identity: module.identity.clone(),
            name: module.name.clone(),
            base: module.base,
            size: module.size,
            space: at.space,
        },
        root: at.root,
    }
}

/// The modules an answer for `target` is read from: the module holding its
/// first address, or every module of the pause.
pub(crate) fn scope_modules(snapshot: &DebugSnapshot, scope: XrefScope, target: &XrefTarget) -> Vec<CopyModule> {
    match scope {
        XrefScope::AllModules => snapshot.symbols.modules.iter().map(|module| copy_module(snapshot, module)).collect(),
        XrefScope::TargetModule => target
            .ranges
            .first()
            .and_then(|range| snapshot.symbols.module_at(range.start))
            .map(|module| vec![copy_module(snapshot, module)])
            .unwrap_or_default(),
    }
}

/// What the run thread is asked to copy for `modules`: every module of the
/// pause is a place a data pointer may point into.
pub(crate) fn copy_request(snapshot: &DebugSnapshot, modules: Vec<CopyModule>) -> CopyRequest {
    let spans = snapshot.symbols.modules.iter().map(|module| ModuleSpan { base: module.base, size: module.size }).collect();
    let taken_at = snapshot.cpus.get(snapshot.cpu).map_or(0, |cpu| cpu.registers.icount);
    CopyRequest { modules, spans, taken_at }
}

/// The one byte at `address`, labelled `label`.
pub(crate) fn address_target(address: u64, label: String) -> XrefTarget {
    XrefTarget { ranges: vec![AddressRange { start: address, end: address.saturating_add(1) }], label }
}

/// One module the answer is read from, and how far its index is.
pub(crate) struct ModuleRow {
    pub(crate) key: ModuleKey,
    pub(crate) state: Option<IndexState>,
}

/// What the tab draws: the answer (which holds its query), and the scope's
/// modules.
pub(crate) struct ReferencesView {
    pub(crate) answer: XrefAnswer,
    pub(crate) modules: Vec<ModuleRow>,
}

fn place_of(named: Option<&NamedAddress>, address: u64) -> String {
    named.map_or_else(|| format!("{address:X}"), NamedAddress::label)
}

/// One found site's row, `depth` 1 for a pointer's reader; a click opens it.
fn site_row(ui: &mut egui::Ui, found: &FoundSite, depth: u8, names: Namer<'_>, action: &mut Option<DebuggerAction>) {
    let named = names.name(found.site);
    let mut place = place_of(named.as_ref(), found.site);
    if depth > 0 {
        place.insert_str(0, "↳ ");
    }
    let line = named.as_ref().and_then(NamedAddress::source).unwrap_or_default();
    let cells = [
        Cell::left(format!("{:016X}", found.site), TEXT_MUTED, 16),
        Cell::left(place, if depth > 0 { TEXT_MUTED } else { ACCENT_BLUE }, PLACE_CHARS),
        Cell::left(kind_text(found.kind), TEXT_PRIMARY, KIND_CHARS),
        Cell::fill(line, TEXT_MUTED),
    ];
    let response = list_row(ui, 0.0, &cells, Tone::Plain, RowSense::Pick { selected: false });
    if response.clicked() {
        let at = GuestAddress { space: AddressSpace::Linear, value: found.site };
        *action = Some(DebuggerAction::ShowCode(CodeTarget::At(at)));
    }
}

/// The field and the scope switch.
fn ask_row(ui: &mut egui::Ui, names: Namer<'_>, pane: &mut DebuggerPane, action: &mut Option<DebuggerAction>) {
    ui.horizontal_wrapped(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut pane.references_text)
                .font(egui::TextStyle::Monospace)
                .hint_text("address or module!name")
                .desired_width(240.0),
        );
        field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, REFERENCES_FIELD));
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if ui.button("Find").clicked() || entered {
            match go_target(&pane.references_text, names) {
                Ok(address) => {
                    pane.references_error = None;
                    *action = Some(DebuggerAction::FindReferences(address_target(address, pane.references_text.trim().to_owned())));
                }
                Err(error) => pane.references_error = Some(error),
            }
        }
        let switch = [
            Segment { value: XrefScope::TargetModule, label: THIS_MODULE },
            Segment { value: XrefScope::AllModules, label: ALL_MODULES },
        ];
        if segmented(ui, &mut pane.references_scope, &switch) {
            if let Some(target) = pane.references.clone() {
                *action = Some(DebuggerAction::FindReferences(target));
            }
        }
    });
    if let Some(error) = pane.references_error {
        ui.label(RichText::new(error.message()).color(ACCENT_AMBER));
    }
}

/// The References tab.
pub(super) fn references_page(
    ui: &mut egui::Ui,
    density: Density,
    view: Option<&ReferencesView>,
    names: Namer<'_>,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    ask_row(ui, names, pane, action);
    ui.add_space(density.gap());
    let Some(view) = view else {
        ui.label(RichText::new(NOTHING_ASKED).color(TEXT_MUTED));
        return;
    };
    ui.label(RichText::new(answer_header(&view.answer)).strong());
    if view.modules.is_empty() {
        ui.label(RichText::new(NO_MODULE_HOLDS).color(ACCENT_AMBER));
        return;
    }
    for failed in &view.answer.failed {
        ui.label(RichText::new(failed).color(ACCENT_AMBER));
    }
    let any_busy = view.modules.iter().any(|row| busy(row.state.as_ref()));
    // The header and the rows each ask on their own; whichever did wins.
    let mut header_ask = None;
    let mut row_ask = None;
    card(
        ui,
        density,
        Title::counted("Modules", view.modules.len()),
        |ui| {
            ui.horizontal(|ui| {
                if ui.button(REINDEX_ALL).clicked() {
                    header_ask = Some(DebuggerAction::Reindex(view.modules.iter().map(|row| row.key.base).collect()));
                }
                if any_busy && ui.button(CANCEL).clicked() {
                    header_ask = Some(DebuggerAction::CancelReferences);
                }
            });
        },
        |ui| {
            for row in &view.modules {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&row.key.name).monospace());
                    ui.label(RichText::new(state_text(row.state.as_ref())).color(TEXT_MUTED));
                    let button = ui.button(REINDEX);
                    let name = format!("{REINDEX} {}", row.key.name);
                    button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
                    if button.clicked() {
                        row_ask = Some(DebuggerAction::Reindex(vec![row.key.base]));
                    }
                });
            }
        },
    );
    if let Some(asked) = header_ask.or(row_ask) {
        *action = Some(asked);
    }
    ui.add_space(density.gap());
    card(
        ui,
        density,
        Title::counted("References", view.answer.found.len()),
        |ui| {
            header_row(
                ui,
                0.0,
                &[
                    Cell::left("Address", TEXT_MUTED, 16),
                    Cell::left("Where", TEXT_MUTED, PLACE_CHARS),
                    Cell::left("How", TEXT_MUTED, KIND_CHARS),
                    Cell::fill("Line", TEXT_MUTED),
                ],
            );
        },
        |ui| {
            if view.answer.found.is_empty() {
                ui.add_space(SPACE_ITEM);
                ui.label(RichText::new("None found.").color(TEXT_MUTED));
            }
            for found in &view.answer.found {
                site_row(ui, found, 0, names, action);
                for reader in &found.through {
                    site_row(ui, reader, 1, names, action);
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::xref::index::ModuleIndex;
    use crate::debugger::xref::service::XrefQuery;
    use std::sync::Arc;

    fn answer(found: Vec<FoundSite>, holes: u32, waiting: Vec<String>) -> XrefAnswer {
        XrefAnswer {
            query: XrefQuery { target: address_target(0x1000, "mydriver!ReadBlock".into()), scope: XrefScope::AllModules },
            found,
            waiting,
            failed: Vec::new(),
            holes,
            taken_at: Some(123_456),
        }
    }

    fn site(module: &str, at: u64) -> FoundSite {
        FoundSite { site: at, kind: XrefKind::Call, module: module.into(), through: Vec::new() }
    }

    #[test]
    fn kinds_and_headers_read_as_written() {
        let kinds = [
            XrefKind::Call,
            XrefKind::Jump,
            XrefKind::CallThrough,
            XrefKind::JumpThrough,
            XrefKind::Memory,
            XrefKind::AddressOf,
            XrefKind::Pointer,
        ];
        let texts: Vec<&str> = kinds.into_iter().map(kind_text).collect();
        assert_eq!(texts, ["call", "jump", "call through", "jump through", "reads/writes", "address of", "pointer"]);
        let found = vec![site("a.sys", 1), site("a.sys", 2), site("b.sys", 3)];
        assert_eq!(
            answer_header(&answer(found.clone(), 0, Vec::new())),
            "3 references to mydriver!ReadBlock in 2 modules · indexed at the pause at instruction 123,456"
        );
        assert_eq!(
            answer_header(&answer(found[..1].to_vec(), 2, vec!["c.sys".into()])),
            "1 reference to mydriver!ReadBlock in 1 module · indexed at the pause at instruction 123,456 · 2 pages not read · 1 module still indexing"
        );
    }

    #[test]
    fn index_states_read_as_written() {
        let index = Arc::new(ModuleIndex { base: 0, size: 0, sites: Vec::new(), holes: 0, taken_at: 1_000 });
        assert_eq!(state_text(None), "not indexed");
        assert_eq!(state_text(Some(&IndexState::Copying)), "waiting to be copied");
        assert_eq!(state_text(Some(&IndexState::Queued)), "waiting to be indexed");
        assert_eq!(state_text(Some(&IndexState::Indexing { done: 2, of: 5 })), "indexing 40%");
        assert_eq!(state_text(Some(&IndexState::Ready(index))), "indexed at instruction 1,000");
        assert_eq!(state_text(Some(&IndexState::Failed("its headers do not read".into()))), "failed: its headers do not read");
    }

    #[test]
    fn numbers_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(12_345_678), "12,345,678");
    }
}
