//! Settings › Symbols: the symbol engine's settings, which every VM shares.
//! The fields edit a draft; each shows beside it the value in use and where
//! that came from. Save writes `settings.toml`, and the engine runs on the
//! saved settings at once.

// The page draws a file the owner may have edited by hand, so nothing here
// may panic (spec §11): no indexing, no unchecked arithmetic, no unwrap.
// Test code is free to; `gui clippy` holds the library to it.
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

use egui::RichText;

use crate::shell::debugger_page::human_size;
use crate::shell::listing::{segmented, Segment};
use crate::shell::theme::{ACCENT_AMBER, ACCENT_RED, SPACE_GROUP, SPACE_ITEM, SPACE_PAGE, TEXT_MUTED, TEXT_PRIMARY};
use crate::symbol_settings::{DownloadToml, Origin, ProjectDraft, SettingsDraft, SettingsState};
use crate::symbols::hub::StoreMeasure;

/// What the page draws from.
pub(crate) struct SymbolsPageState<'a> {
    pub(crate) draft: &'a mut SettingsDraft,
    pub(crate) settings: &'a SettingsState,
    /// How much the store in use holds, once measured.
    pub(crate) usage: Option<&'a StoreMeasure>,
    /// Why the last Save did not write.
    pub(crate) save_problem: Option<&'a str>,
}

/// A folder field a Browse fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FolderField {
    Store,
    /// A project's symbol folders, by the project's index.
    ProjectSymbols(usize),
    /// A project's source folders, by the project's index.
    ProjectSources(usize),
}

/// What a click on the page asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SymbolsPageAction {
    Save,
    Revert,
    /// Pick a folder for a field: desktops have a folder picker; on Android
    /// folders are typed.
    #[cfg(not(target_os = "android"))]
    Browse(FolderField),
    /// Open the store in use in the system's file manager.
    #[cfg(not(target_os = "android"))]
    OpenStore,
}

/// The page's heading, which a test finds it by.
pub(crate) const SYMBOLS_HEADING: &str = "Symbols";

/// What a privacy-minded owner needs to know before downloads run.
const PRIVACY: &str = "A download tells the symbol server which module it is for (its file \
     name and build) and your IP address.";

/// Where a value in use came from, in words.
fn origin_text(origin: Origin) -> String {
    match origin {
        Origin::File => "from settings.toml".to_owned(),
        Origin::Environment { variable } => format!("from {variable}"),
        Origin::Default => "the default".to_owned(),
    }
}

/// A field's caption and, under it, the value in use.
fn in_use(ui: &mut egui::Ui, value: &str, origin: Origin) {
    ui.label(RichText::new(format!("In use: {value} ({})", origin_text(origin))).color(TEXT_MUTED));
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(SPACE_GROUP);
    ui.label(RichText::new(title).strong().color(TEXT_PRIMARY));
}

fn lines_field(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    ui.add(egui::TextEdit::multiline(text).hint_text(hint).desired_rows(2).desired_width(f32::INFINITY));
}

/// Draws the page and reports what was clicked.
pub(crate) fn draw_symbols_page(ui: &mut egui::Ui, state: SymbolsPageState<'_>) -> Option<SymbolsPageAction> {
    let SymbolsPageState { draft, settings, usage, save_problem } = state;
    let resolved = &settings.resolved;
    let mut action = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::same(SPACE_PAGE)).show(ui, |ui| {
            ui.heading("Settings");
            ui.label(RichText::new(SYMBOLS_HEADING).size(18.0).color(TEXT_PRIMARY));
            ui.label(RichText::new(format!("Kept in {}, for every VM.", settings.path.display())).color(TEXT_MUTED));
            if let Some(problem) = &settings.problem {
                ui.label(
                    RichText::new(format!(
                        "{problem}. The engine runs on the environment and the defaults until the settings are saved; \
                         saving first keeps that file beside it, as .broken."
                    ))
                    .color(ACCENT_AMBER),
                );
            }
            if let Some(aside) = &settings.set_aside {
                ui.label(
                    RichText::new(format!("The file that did not load is kept as {}.", aside.display())).color(TEXT_MUTED),
                );
            }

            section(ui, "Symbol store");
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut draft.store)
                        .hint_text("Unset: the environment's or the default")
                        .desired_width(360.0),
                );
                #[cfg(not(target_os = "android"))]
                {
                    if ui.button("Browse…").clicked() {
                        action = Some(SymbolsPageAction::Browse(FolderField::Store));
                    }
                    if ui.button("Open").clicked() {
                        action = Some(SymbolsPageAction::OpenStore);
                    }
                }
            });
            in_use(ui, &resolved.store.value.display().to_string(), resolved.store.origin);
            let held = match usage {
                Some(StoreMeasure::Measured(usage)) => format!("It holds {} files, {}.", usage.files, human_size(usage.bytes)),
                Some(StoreMeasure::Failed(reason)) => format!("Its size did not read: {reason}"),
                None => "Measuring it…".to_owned(),
            };
            ui.label(RichText::new(held).color(TEXT_MUTED));

            section(ui, "Downloads");
            segmented(
                ui,
                &mut draft.downloads,
                &[
                    Segment { value: None, label: "Unset" },
                    Segment { value: Some(DownloadToml::Automatic), label: "Automatic" },
                    Segment { value: Some(DownloadToml::Off), label: "Off" },
                ],
            );
            let downloads = match resolved.downloads.value {
                rusty_box_symbols::service::Downloads::Automatic => "automatic",
                rusty_box_symbols::service::Downloads::Off => "off",
            };
            in_use(ui, downloads, resolved.downloads.origin);
            ui.label(RichText::new(PRIVACY).color(TEXT_MUTED));

            section(ui, "Microsoft symbol servers, one a line, tried in order");
            lines_field(ui, &mut draft.microsoft, "Unset: the environment's or the default");
            in_use(ui, &resolved.microsoft.value.join(", "), resolved.microsoft.origin);

            section(ui, "debuginfod servers, one a line, tried in order");
            lines_field(ui, &mut draft.debuginfod, "Unset: the environment's or the default");
            in_use(ui, &resolved.debuginfod.value.join(", "), resolved.debuginfod.origin);

            section(ui, "Projects: your own builds, matched by their identity");
            let mut removed = None;
            for (index, project) in draft.projects.iter_mut().enumerate() {
                ui.add_space(SPACE_ITEM);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut project.name);
                        if ui.button("Remove project").clicked() {
                            removed = Some(index);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Symbol folders, one a line");
                        #[cfg(not(target_os = "android"))]
                        if ui.small_button("Browse…").clicked() {
                            action = Some(SymbolsPageAction::Browse(FolderField::ProjectSymbols(index)));
                        }
                    });
                    lines_field(ui, &mut project.symbols, "");
                    ui.horizontal(|ui| {
                        ui.label("Source folders, one a line");
                        #[cfg(not(target_os = "android"))]
                        if ui.small_button("Browse…").clicked() {
                            action = Some(SymbolsPageAction::Browse(FolderField::ProjectSources(index)));
                        }
                    });
                    lines_field(ui, &mut project.sources, "");
                    ui.label("Path maps, one a line: the build machine's folder => yours");
                    lines_field(ui, &mut project.map, "C:\\agent\\_work\\1\\s => D:\\work\\mydriver");
                });
            }
            // `removed` came from this frame's own enumeration of the list.
            if let Some(index) = removed.filter(|index| *index < draft.projects.len()) {
                draft.projects.remove(index);
            }
            if ui.button("Add project").clicked() {
                draft.projects.push(ProjectDraft::default());
            }

            ui.add_space(SPACE_GROUP);
            if let Some(problem) = save_problem {
                ui.label(RichText::new(problem).color(ACCENT_RED));
            }
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    action = Some(SymbolsPageAction::Save);
                }
                if ui.button("Revert").clicked() {
                    action = Some(SymbolsPageAction::Revert);
                }
            });
        });
    });
    action
}
