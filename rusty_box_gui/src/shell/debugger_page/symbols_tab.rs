//! The Symbols tab: one row per module of the kernel and of the shown
//! process — its name, what the symbol engine has for it, and where its
//! tables came from — and, for the picked row, Load from file and the one
//! action that fixes its state. The header chip counts the same rows.

// What this tab draws comes from guest memory and from files on disk, so
// nothing here may panic (spec §11): no indexing, no unchecked arithmetic,
// no unwrap. Test code is free to; `gui clippy` holds the library to it.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented,
        clippy::arithmetic_side_effects,
        clippy::string_slice,
        clippy::get_unwrap,
        clippy::panic_in_result_fn
    )
)]

use std::path::PathBuf;

use rusty_box_symbols::http::Retry;
use rusty_box_symbols::identity::ModuleIdentity;
use rusty_box_symbols::projects::IndexProblem;
use rusty_box_symbols::service::{Kept, SymbolSource};

use super::*;
use crate::debugger::inspect::{SymModule, SymbolIndex};
use crate::debugger::module_identity::GuestIdentity;
use crate::symbols::hub::ModuleSymbols;

/// Which modules the tab lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SymbolFilter {
    #[default]
    All,
    /// Those whose names are not loaded.
    Missing,
}

/// What the Symbols tab asks of the symbol engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SymbolsAction {
    /// Ask which file to load for the module.
    LoadFromFile(ModuleIdentity),
    /// Load the file refused as another build's.
    LoadAnyway { identity: ModuleIdentity, path: PathBuf },
    Retry(ModuleIdentity),
    Cancel(ModuleIdentity),
    Unload(ModuleIdentity),
    /// Index the project folders again.
    Rescan,
    /// Open the symbol store's folder in the system's file manager, which
    /// Android has none of.
    #[cfg(not(target_os = "android"))]
    OpenStore,
    /// Show the app's Settings, where the store and the servers are set.
    OpenSettings,
}

/// The tab's way to the app's Settings; the drawer's own row is labelled
/// "Settings", and two controls of one name would be ambiguous.
pub(crate) const SYMBOL_SETTINGS: &str = "Symbol settings";

/// The one action that fixes a module's state.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Fix {
    Cancel,
    Unload,
    LoadAnyway(PathBuf),
    Retry,
}

impl Fix {
    const fn label(&self) -> &'static str {
        match self {
            Fix::Cancel => "Cancel",
            Fix::Unload => "Unload",
            Fix::LoadAnyway(_) => "Load anyway",
            Fix::Retry => "Retry",
        }
    }

    fn action(&self, identity: &ModuleIdentity) -> SymbolsAction {
        match self {
            Fix::Cancel => SymbolsAction::Cancel(identity.clone()),
            Fix::Unload => SymbolsAction::Unload(identity.clone()),
            Fix::LoadAnyway(path) => SymbolsAction::LoadAnyway { identity: identity.clone(), path: path.clone() },
            Fix::Retry => SymbolsAction::Retry(identity.clone()),
        }
    }
}

/// How a module's symbols stand, for the chip's counts and the filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    Loaded,
    /// Asked for; no worker has taken it yet.
    Waiting,
    Downloading,
    Missing,
}

/// A module's state as the tab says it.
struct StateText {
    state: String,
    color: egui::Color32,
    source: String,
    standing: Standing,
    fix: Option<Fix>,
}

/// What a module of `identity`, whose engine state is `state`, shows.
fn state_text(identity: &GuestIdentity, state: Option<&ModuleSymbols>) -> StateText {
    let missing = |state: String, fix: Option<Fix>| StateText {
        state,
        color: ACCENT_AMBER,
        source: String::new(),
        standing: Standing::Missing,
        fix,
    };
    let reason = match identity {
        GuestIdentity::Known(_) => None,
        GuestIdentity::HeadersUnreadable(error) => Some(format!("no identity: {error}")),
        GuestIdentity::Unnamed => Some("no identity: the loader names no file".to_owned()),
    };
    if let Some(reason) = reason {
        return missing(reason, None);
    }
    match state {
        None => missing("not asked for yet".to_owned(), Some(Fix::Retry)),
        Some(ModuleSymbols::Pending) => StateText {
            state: "waiting".to_owned(),
            color: TEXT_MUTED,
            source: String::new(),
            standing: Standing::Waiting,
            fix: Some(Fix::Cancel),
        },
        Some(ModuleSymbols::Downloading { done, total }) => StateText {
            state: match total {
                Some(total) => format!("downloading {} of {}", human_size(*done), human_size(*total)),
                None => format!("downloading {}", human_size(*done)),
            },
            color: ACCENT_BLUE,
            source: String::new(),
            standing: Standing::Downloading,
            fix: Some(Fix::Cancel),
        },
        Some(ModuleSymbols::Loaded { from }) => StateText {
            state: "loaded".to_owned(),
            color: ACCENT_CYAN,
            source: source_text(from),
            standing: Standing::Loaded,
            fix: Some(Fix::Unload),
        },
        Some(ModuleSymbols::Forced { found, path }) => StateText {
            state: format!("loaded anyway: the file is {found}"),
            color: ACCENT_AMBER,
            source: path.display().to_string(),
            standing: Standing::Loaded,
            fix: Some(Fix::Unload),
        },
        Some(ModuleSymbols::Refused { found, path }) => {
            let mut text = missing(format!("refused: the file is {found}"), Some(Fix::LoadAnyway(path.clone())));
            text.source = path.display().to_string();
            text
        }
        Some(ModuleSymbols::NotOnServer) => missing("not on the symbol servers".to_owned(), Some(Fix::Retry)),
        Some(ModuleSymbols::Failed { reason, retry }) => {
            let fix = match retry {
                Retry::Retryable => Some(Fix::Retry),
                Retry::Final => None,
            };
            let mut text = missing(format!("failed: {reason}"), fix);
            text.color = ACCENT_RED;
            text
        }
        Some(ModuleSymbols::Unloaded) => missing("unloaded".to_owned(), Some(Fix::Retry)),
    }
}

/// Where tables came from, in words.
fn source_text(source: &SymbolSource) -> String {
    match source {
        SymbolSource::Attached { path } => format!("attached {}", path.display()),
        SymbolSource::Project { name, path } => format!("project {name}: {}", path.display()),
        SymbolSource::Store { path } => format!("store: {}", path.display()),
        SymbolSource::Server { url, kept } => match kept {
            Kept::InStore { .. } => format!("downloaded from {url}"),
            Kept::Replaced { bad, .. } => {
                format!("downloaded from {url}; the store's copy that did not load is now {}", bad.display())
            }
            Kept::NotKept { reason } => format!("downloaded from {url}, not kept: {reason}"),
        },
    }
}

/// What the tab says when the pause named no module: symbols are looked up
/// for a 64-bit Windows guest's kernel and processes, which the walks found
/// none of.
const NO_MODULES: &str = "No modules in this pause. Symbols are looked up for the kernel and processes of a 64-bit \
                          Windows guest; the Processes tab says why none was found.";

/// How many project files that did not index are named one by one.
const PROBLEMS_SHOWN: usize = 3;

/// The project files the index could not read, each with why; `None` when
/// every file read.
fn index_problems_text(problems: &[IndexProblem]) -> Option<String> {
    if problems.is_empty() {
        return None;
    }
    let noun = if problems.len() == 1 { "file" } else { "files" };
    let mut text = format!("{} project {noun} did not index:", problems.len());
    for problem in problems.iter().take(PROBLEMS_SHOWN) {
        text.push_str(&format!("\n{}: {}", problem.path.display(), problem.reason));
    }
    let more = problems.len().saturating_sub(PROBLEMS_SHOWN);
    if more > 0 {
        text.push_str(&format!("\nand {more} more"));
    }
    Some(text)
}

/// How many of the pause's modules stand each way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SymbolCounts {
    pub(crate) loaded: usize,
    pub(crate) waiting: usize,
    pub(crate) downloading: usize,
    pub(crate) missing: usize,
}

/// One of the chip's counts, with what it counts.
struct Counted {
    count: usize,
    what: &'static str,
}

impl SymbolCounts {
    /// "14 loaded · 1 waiting · 2 downloading · 1 missing", leaving out what
    /// is none.
    pub(crate) fn label(self) -> String {
        let parts: Vec<String> = [
            Counted { count: self.loaded, what: "loaded" },
            Counted { count: self.waiting, what: "waiting" },
            Counted { count: self.downloading, what: "downloading" },
            Counted { count: self.missing, what: "missing" },
        ]
        .into_iter()
        .filter(|part| part.count > 0)
        .map(|part| format!("{} {}", part.count, part.what))
        .collect();
        if parts.is_empty() {
            "no modules".to_owned()
        } else {
            parts.join(" · ")
        }
    }
}

fn state_of<'a>(identity: &GuestIdentity, view: &'a SymbolsView) -> Option<&'a ModuleSymbols> {
    identity.known().and_then(|identity| view.state(identity))
}

/// The counts the header chip shows for `symbols`.
pub(crate) fn symbol_counts(symbols: &SymbolIndex, view: &SymbolsView) -> SymbolCounts {
    let mut counts = SymbolCounts::default();
    for module in &symbols.modules {
        match state_text(&module.identity, state_of(&module.identity, view)).standing {
            Standing::Loaded => counts.loaded = counts.loaded.saturating_add(1),
            Standing::Waiting => counts.waiting = counts.waiting.saturating_add(1),
            Standing::Downloading => counts.downloading = counts.downloading.saturating_add(1),
            Standing::Missing => counts.missing = counts.missing.saturating_add(1),
        }
    }
    counts
}

const MODULE_CHARS: usize = 18;
const STATE_CHARS: usize = 34;

fn symbol_columns() -> [Cell; 3] {
    [
        Cell::left("Module", TEXT_MUTED, MODULE_CHARS),
        Cell::left("Symbols", TEXT_MUTED, STATE_CHARS),
        Cell::fill("Source", TEXT_MUTED),
    ]
}

/// The tab: the filter, Rescan projects and Open store folder; the picked
/// module's identity, Load from file and its fix; then the rows.
pub(crate) fn symbols_page(
    ui: &mut egui::Ui,
    fit: Fit,
    snapshot: &DebugSnapshot,
    view: &SymbolsView,
    pane: &mut DebuggerPane,
    action: &mut Option<DebuggerAction>,
) {
    let shown: Vec<&SymModule> = snapshot
        .symbols
        .modules
        .iter()
        .filter(|module| match pane.symbol_filter {
            SymbolFilter::All => true,
            SymbolFilter::Missing => {
                let state = state_of(&module.identity, view);
                state_text(&module.identity, state).standing == Standing::Missing
            }
        })
        .collect();
    let keyed_row = list_keys(ui).and_then(|keys| {
        let walked = walk(keys, pane.symbol_cursor, shown.len());
        if let Some(KeyRow::Moved(to)) = walked.key {
            pane.symbol_cursor = Some(to);
            pane.picked_symbol = shown.get(to).map(|module| module.base);
        }
        walked.key
    });
    let picked = pane.picked_symbol.and_then(|base| snapshot.symbols.modules.iter().find(|module| module.base == base));
    let title = Title::counted("Symbols", snapshot.symbols.modules.len()).inlined();
    let mut card = |ui: &mut egui::Ui, room: Room, action: &mut Option<DebuggerAction>| {
        keyed_pane_card(
            ui,
            room,
            title,
            keyed_row.is_some(),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    title_label(ui, title.text, title.count);
                    segmented(
                        ui,
                        &mut pane.symbol_filter,
                        &[
                            Segment { value: SymbolFilter::All, label: "All" },
                            Segment { value: SymbolFilter::Missing, label: "Missing only" },
                        ],
                    );
                    if ui.button("Rescan projects").clicked() {
                        *action = Some(DebuggerAction::Symbols(SymbolsAction::Rescan));
                    }
                    #[cfg(not(target_os = "android"))]
                    if ui.button("Open store folder").clicked() {
                        *action = Some(DebuggerAction::Symbols(SymbolsAction::OpenStore));
                    }
                    if ui.button(SYMBOL_SETTINGS).clicked() {
                        *action = Some(DebuggerAction::Symbols(SymbolsAction::OpenSettings));
                    }
                });
                if let Some(problem) = &view.projects_problem {
                    ui.label(RichText::new(format!("The project folders were not indexed: {problem}")).color(ACCENT_RED));
                }
                if let Some(problems) = index_problems_text(&view.projects.problems) {
                    ui.label(RichText::new(problems).color(ACCENT_AMBER));
                }
                if let Some(module) = picked {
                    picked_symbol_bar(ui, &module.identity, view, action);
                }
                header_row(ui, 0.0, &symbol_columns());
            },
            |ui| {
                if snapshot.symbols.modules.is_empty() {
                    ui.label(RichText::new(NO_MODULES).color(TEXT_MUTED));
                }
                for (n, module) in shown.iter().enumerate() {
                    if culled(ui, keyed_row, n) {
                        skip_row(ui);
                        continue;
                    }
                    let text = state_text(&module.identity, state_of(&module.identity, view));
                    let tone = if pane.picked_symbol == Some(module.base) { Tone::Selected } else { keyed_tone(keyed_row, n) };
                    let cells = [
                        Cell::left(module.name.as_str(), TEXT_PRIMARY, MODULE_CHARS),
                        Cell::left(text.state, text.color, STATE_CHARS),
                        Cell::fill(text.source, TEXT_MUTED),
                    ];
                    let response =
                        list_row(ui, 0.0, &cells, tone, RowSense::Pick { selected: pane.picked_symbol == Some(module.base) });
                    follow_key(&response, keyed_row, n);
                    if response.clicked() {
                        pane.picked_symbol = Some(module.base);
                        pane.symbol_cursor = Some(n);
                    }
                }
            },
        );
    };
    match fit {
        Fit::Grid => card(ui, Room::Fixed(ui.available_size()), action),
        Fit::Scrolling(_) => {
            let room = Room::Content(fit.density());
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| card(ui, room, action));
        }
    }
}

/// A module's identity, Load from file, and the fix its state takes.
fn picked_symbol_bar(ui: &mut egui::Ui, guest: &GuestIdentity, view: &SymbolsView, action: &mut Option<DebuggerAction>) {
    let text = state_text(guest, state_of(guest, view));
    let Some(identity) = guest.known() else {
        ui.label(RichText::new(text.state).color(ACCENT_AMBER));
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(identity.to_string()).color(TEXT_PRIMARY));
        if ui.button(LOAD_FROM_FILE).clicked() {
            *action = Some(DebuggerAction::Symbols(SymbolsAction::LoadFromFile(identity.clone())));
        }
        if let Some(fix) = &text.fix {
            if ui.button(fix.label()).clicked() {
                *action = Some(DebuggerAction::Symbols(fix.action(identity)));
            }
        }
    });
}

/// A picked module's Symbols view in a module card: its identity, what the
/// engine has for it and where that came from, and the same actions as the
/// Symbols tab.
pub(super) fn module_symbols_body(
    ui: &mut egui::Ui,
    guest: &GuestIdentity,
    view: &SymbolsView,
    action: &mut Option<DebuggerAction>,
) {
    let text = state_text(guest, state_of(guest, view));
    ui.add_space(SPACE_ITEM);
    if guest.known().is_some() {
        ui.label(RichText::new(text.state).color(text.color));
        if !text.source.is_empty() {
            ui.label(RichText::new(text.source).color(TEXT_MUTED));
        }
    }
    picked_symbol_bar(ui, guest, view, action);
}

/// What the module lists' Symbols column reads its states from: the pause's
/// named modules, and what the engine has.
#[derive(Clone, Copy)]
pub(super) struct ModuleStates<'a> {
    pub(super) index: &'a SymbolIndex,
    pub(super) view: &'a SymbolsView,
}

/// The Symbols column's width.
pub(super) const SYMBOL_STATE_CHARS: usize = 9;

impl ModuleStates<'_> {
    /// The Symbols cell of the module at `base`: a dash for a module this
    /// pause did not name (another process's).
    pub(super) fn cell(self, base: u64) -> Cell {
        let Some(module) = self.index.modules.iter().find(|module| module.base == base) else {
            return Cell::left("-", TEXT_MUTED, SYMBOL_STATE_CHARS);
        };
        let state = state_of(&module.identity, self.view);
        let forced = matches!(state, Some(ModuleSymbols::Forced { .. }));
        let cell = |word: &str, color| Cell::left(word, color, SYMBOL_STATE_CHARS);
        match state_text(&module.identity, state).standing {
            Standing::Loaded if forced => cell("forced", ACCENT_AMBER),
            Standing::Loaded => cell("loaded", ACCENT_CYAN),
            Standing::Waiting => cell("waiting", TEXT_MUTED),
            Standing::Downloading => cell("loading", ACCENT_BLUE),
            Standing::Missing => cell("missing", ACCENT_AMBER),
        }
    }
}

/// The button that asks for a file of debug information.
pub(crate) const LOAD_FROM_FILE: &str = "Load symbols from file…";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::inspect::ModuleSpace;
    use rusty_box_symbols::identity::{Guid, PeImageId, TablesIdentity};
    use rusty_box_symbols::pe::HeaderError;
    use rusty_box_symbols::units::Rva;

    fn module(identity: GuestIdentity) -> SymModule {
        SymModule { base: 0x1000, size: 0x1000, name: "disk.sys".to_owned(), space: ModuleSpace::Kernel, identity }
    }

    fn known() -> GuestIdentity {
        GuestIdentity::Known(ModuleIdentity::Pe {
            file_name: "disk.sys".to_owned(),
            image: PeImageId { time_date_stamp: 1, size_of_image: 0x1000 },
            pdb: None,
        })
    }

    struct Case {
        state: Option<ModuleSymbols>,
        fix: Option<Fix>,
        standing: Standing,
    }

    /// Every state says what it is and offers the one action that fixes it.
    #[test]
    fn each_state_offers_the_action_that_fixes_it() {
        let found = TablesIdentity::Pdb { guid: Guid::from_le_bytes([1; 16]), age: 2 };
        let path = PathBuf::from("D:\\x\\disk.pdb");
        let case = |state, fix, standing| Case { state, fix, standing };
        let cases = [
            case(None, Some(Fix::Retry), Standing::Missing),
            case(Some(ModuleSymbols::Pending), Some(Fix::Cancel), Standing::Waiting),
            case(Some(ModuleSymbols::Downloading { done: 1024, total: Some(4096) }), Some(Fix::Cancel), Standing::Downloading),
            case(
                Some(ModuleSymbols::Loaded { from: SymbolSource::Store { path: path.clone() } }),
                Some(Fix::Unload),
                Standing::Loaded,
            ),
            case(Some(ModuleSymbols::Forced { found: found.clone(), path: path.clone() }), Some(Fix::Unload), Standing::Loaded),
            case(Some(ModuleSymbols::Refused { found, path: path.clone() }), Some(Fix::LoadAnyway(path)), Standing::Missing),
            case(Some(ModuleSymbols::NotOnServer), Some(Fix::Retry), Standing::Missing),
            case(
                Some(ModuleSymbols::Failed { reason: "timed out".to_owned(), retry: Retry::Retryable }),
                Some(Fix::Retry),
                Standing::Missing,
            ),
            case(Some(ModuleSymbols::Failed { reason: "bad URL".to_owned(), retry: Retry::Final }), None, Standing::Missing),
            case(Some(ModuleSymbols::Unloaded), Some(Fix::Retry), Standing::Missing),
        ];
        for case in cases {
            let text = state_text(&module(known()).identity, case.state.as_ref());
            assert!(!text.state.is_empty(), "{:?} says what it is", case.state);
            assert_eq!(text.fix, case.fix, "{:?}", case.state);
            assert_eq!(text.standing, case.standing, "{:?}", case.state);
        }
    }

    /// A download that replaced a store copy which did not load says where
    /// that copy went: the store may be a debugger's too.
    #[test]
    fn a_replaced_store_copy_is_named_where_it_went() {
        let bad = PathBuf::from("D:\\Symbols\\disk.pdb\\0123\\disk.pdb.bad");
        let from = SymbolSource::Server {
            url: "https://msdl.example/disk.pdb".to_owned(),
            kept: Kept::Replaced { path: PathBuf::from("D:\\Symbols\\disk.pdb\\0123\\disk.pdb"), bad: bad.clone() },
        };
        let text = state_text(&known(), Some(&ModuleSymbols::Loaded { from }));
        assert!(text.source.contains(&bad.display().to_string()), "{}", text.source);
    }

    #[test]
    fn index_problems_name_the_first_few_and_count_the_rest() {
        assert_eq!(index_problems_text(&[]), None);
        let problems: Vec<IndexProblem> = (0..5)
            .map(|n| IndexProblem { path: PathBuf::from(format!("p{n}.pdb")), reason: "cut short".to_owned() })
            .collect();
        let text = index_problems_text(&problems).expect("listed");
        assert!(text.starts_with("5 project files did not index:"), "{text}");
        assert!(text.contains("p2.pdb: cut short") && !text.contains("p3.pdb"), "{text}");
        assert!(text.ends_with("and 2 more"), "{text}");
    }

    /// A module with no identity says why, and offers nothing to look up.
    #[test]
    fn a_module_without_an_identity_says_why() {
        let unreadable = module(GuestIdentity::HeadersUnreadable(HeaderError::Unreadable {
            what: "DOS header",
            rva: Rva(0),
            len: 0x40,
        }));
        let text = state_text(&unreadable.identity, None);
        assert!(text.state.starts_with("no identity: the DOS header"), "{}", text.state);
        assert_eq!(text.fix, None);
        assert_eq!(state_text(&GuestIdentity::Unnamed, None).standing, Standing::Missing);
    }

    /// A module asked for but not yet taken by a worker counts as waiting,
    /// not as downloading.
    #[test]
    fn the_chip_counts_waiting_apart_from_downloading() {
        let view = |state: ModuleSymbols| {
            let mut view = SymbolsView::default();
            if let GuestIdentity::Known(identity) = known() {
                view.modules.insert(identity, crate::symbols::hub::ModuleEntry { state, tables: None });
            }
            view
        };
        let index = SymbolIndex { modules: vec![module(known())], kernel: None };
        let waiting = symbol_counts(&index, &view(ModuleSymbols::Pending));
        assert_eq!(waiting, SymbolCounts { loaded: 0, waiting: 1, downloading: 0, missing: 0 });
        assert_eq!(waiting.label(), "1 waiting");
        let downloading = symbol_counts(&index, &view(ModuleSymbols::Downloading { done: 1, total: None }));
        assert_eq!(downloading.label(), "1 downloading");
    }

    #[test]
    fn the_chip_counts_only_what_there_is() {
        let counts = SymbolCounts { loaded: 14, waiting: 0, downloading: 2, missing: 1 };
        assert_eq!(counts.label(), "14 loaded · 2 downloading · 1 missing");
        assert_eq!(SymbolCounts { loaded: 3, waiting: 0, downloading: 0, missing: 0 }.label(), "3 loaded");
        assert_eq!(SymbolCounts::default().label(), "no modules");
    }
}
