//! The desktop's navigation.
//!
//! `VmLibraryEntry` is what a VM profile summarises as — the strings a row
//! shows and a search matches against — and both shells hold their library as
//! a list of them. `draw_sidebar` is the desktop's only navigation: every
//! profile as a `selection_row`, the selected one open to its pages. It draws
//! from borrowed data and reports the click; the app decides what the click
//! means.

#[cfg(not(target_arch = "wasm32"))]
use crate::shell::destination::{Destination, ShellPage, SidebarAction};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::theme::{
    ACCENT_AMBER, BG_PANEL, SPACE_GROUP, SPACE_ITEM, STROKE_HAIRLINE, TEXT_BODY, TEXT_CAPTION,
    TEXT_MUTED,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::widgets::{selection_row, RowMark, ShellStateBadge, CHILD_INDENT, ROOT_INDENT};
#[cfg(not(target_arch = "wasm32"))]
use egui::{RichText, Stroke};

/// Whether a VM in the list is a file in the library, a library file that
/// is behind the VM because its last write failed, or only in memory. The
/// browser shell has no library, so its one entry carries no source.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntrySource {
    /// A library file; the row is the VM's name.
    Saved,
    /// Only in memory — the command line's temporary VM; the row says so.
    Unsaved,
    /// A library file the last write to failed; the row says so.
    WriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VmLibraryEntry {
    pub(crate) name: String,
    pub(crate) boot: String,
    pub(crate) memory: String,
    pub(crate) disk: String,
    pub(crate) cdrom: String,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) source: EntrySource,
}

impl VmLibraryEntry {
    pub(crate) fn new(
        name: impl Into<String>,
        boot: impl Into<String>,
        memory: impl Into<String>,
        disk: impl Into<String>,
        cdrom: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            boot: boot.into(),
            memory: memory.into(),
            disk: disk.into(),
            cdrom: cdrom.into(),
            #[cfg(not(target_arch = "wasm32"))]
            source: EntrySource::Saved,
        }
    }

    /// The same entry, marked as a VM that is not in the library.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn unsaved(self) -> Self {
        Self {
            source: EntrySource::Unsaved,
            ..self
        }
    }

    /// The same entry, marked as a library VM whose last write failed.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn write_failed(self) -> Self {
        Self {
            source: EntrySource::WriteFailed,
            ..self
        }
    }

    /// The text of the entry's row.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn row_label(&self) -> String {
        match self.source {
            EntrySource::Saved => self.name.clone(),
            EntrySource::Unsaved => format!("{} (unsaved)", self.name),
            EntrySource::WriteFailed => format!("{} (save failed)", self.name),
        }
    }

    pub(crate) fn matches_filter(&self, filter: &str) -> bool {
        filter.is_empty()
            || self.name.to_ascii_lowercase().contains(filter)
            || self.boot.to_ascii_lowercase().contains(filter)
            || self.disk.to_ascii_lowercase().contains(filter)
            || self.cdrom.to_ascii_lowercase().contains(filter)
    }
}

/// The width the sidebar opens at, and the narrowest a drag may make it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SIDEBAR_DEFAULT_WIDTH: f32 = 200.0;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 170.0;

/// The desktop's only navigation: every VM profile, with the selected one
/// expanded into its pages, and under them the library files that do not
/// load, each with its Delete. Draws from borrowed data and reports what was
/// clicked; the caller owns the consequences.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_sidebar(
    ui: &mut egui::Ui,
    entries: &[VmLibraryEntry],
    visible: &[usize],
    destination: Destination,
    filter: &mut String,
    badge: ShellStateBadge,
    broken: &[crate::library::BrokenVmFile],
) -> Option<SidebarAction> {
    let mut action = None;
    egui::Panel::left("vm_sidebar")
        .resizable(true)
        .default_size(SIDEBAR_DEFAULT_WIDTH)
        .min_size(SIDEBAR_MIN_WIDTH)
        .frame(
            egui::Frame::new()
                .fill(BG_PANEL)
                .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
                .inner_margin(egui::Margin::symmetric(8, 10)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("My computer")
                        .size(TEXT_CAPTION)
                        .color(TEXT_MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("+")
                        .on_hover_text("New VM (a copy of the selected one)")
                        .clicked()
                    {
                        action = Some(SidebarAction::NewVm);
                    }
                });
            });
            ui.add_space(SPACE_ITEM);
            ui.add(
                egui::TextEdit::singleline(filter)
                    .desired_width(f32::INFINITY)
                    .hint_text("Search VMs"),
            );
            ui.add_space(SPACE_ITEM);

            if entries.is_empty() {
                ui.label(
                    RichText::new("No VMs")
                        .size(TEXT_BODY)
                        .color(TEXT_MUTED),
                );
                return;
            }

            for &index in visible {
                let is_selected_vm = destination.vm() == index;
                let dot = is_selected_vm.then_some(badge.color);
                let vm_mark = if is_selected_vm {
                    RowMark::Expanded
                } else {
                    RowMark::Plain
                };
                if selection_row(ui, &entries[index].row_label(), ROOT_INDENT, vm_mark, dot)
                    .clicked()
                {
                    action = Some(SidebarAction::Select(destination.select_vm(index)));
                }
                if !is_selected_vm {
                    continue;
                }
                for page in ShellPage::ALL {
                    let page_mark = if destination.page() == page {
                        RowMark::Destination
                    } else {
                        RowMark::Plain
                    };
                    if selection_row(ui, page.label(), CHILD_INDENT, page_mark, None).clicked() {
                        action = Some(SidebarAction::Select(destination.select_page(page)));
                    }
                }
            }

            if !broken.is_empty() {
                ui.add_space(SPACE_GROUP);
                ui.label(
                    RichText::new("Could not load")
                        .size(TEXT_CAPTION)
                        .color(TEXT_MUTED),
                );
                for (index, file) in broken.iter().enumerate() {
                    let name = file
                        .path
                        .file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).color(ACCENT_AMBER))
                            .on_hover_text(&file.error);
                        if ui.small_button("Delete").clicked() {
                            action = Some(SidebarAction::DeleteBroken(index));
                        }
                    });
                }
            }
        });
    action
}
