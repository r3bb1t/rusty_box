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
use crate::shell::theme::{BG_PANEL, SPACE_ITEM, STROKE_HAIRLINE, TEXT_BODY, TEXT_CAPTION, TEXT_MUTED};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::widgets::{selection_row, RowMark, ShellStateBadge, CHILD_INDENT, ROOT_INDENT};
#[cfg(not(target_arch = "wasm32"))]
use egui::{RichText, Stroke};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VmLibraryEntry {
    pub(crate) name: String,
    pub(crate) boot: String,
    pub(crate) memory: String,
    pub(crate) disk: String,
    pub(crate) cdrom: String,
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
/// expanded into its pages. Draws from borrowed data and reports what was
/// clicked; the caller owns the consequences.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_sidebar(
    ui: &mut egui::Ui,
    entries: &[VmLibraryEntry],
    visible: &[usize],
    destination: Destination,
    filter: &mut String,
    badge: ShellStateBadge,
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
                        .on_hover_text("Duplicate this VM profile")
                        .clicked()
                    {
                        action = Some(SidebarAction::DuplicateSelected);
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
                    RichText::new("No VM profiles")
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
                if selection_row(ui, &entries[index].name, ROOT_INDENT, vm_mark, dot).clicked() {
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
        });
    action
}
