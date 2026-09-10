//! The desktop's navigation, and the row it is built from.
//!
//! `VmLibraryEntry` is what a VM profile summarises as — the strings a row
//! shows and a search matches against — and both shells hold their library as
//! a list of them. `draw_sidebar` is the desktop's only navigation: every
//! profile as a row, the selected one open to its pages. It draws from
//! borrowed data and reports the click; the app decides what the click means.

#[cfg(not(target_arch = "wasm32"))]
use crate::shell::destination::{Destination, ShellPage, SidebarAction};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::theme::{
    ACCENT_CYAN, BG_CARD, BG_PANEL, SPACE_ITEM, STROKE_HAIRLINE, TEXT_BODY, TEXT_CAPTION,
    TEXT_MUTED, TEXT_PRIMARY,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::widgets::ShellStateBadge;
#[cfg(not(target_arch = "wasm32"))]
use egui::{Color32, RichText, Stroke, WidgetInfo, WidgetType};

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

#[cfg(not(target_arch = "wasm32"))]
const ROW_HEIGHT: f32 = 24.0;
#[cfg(not(target_arch = "wasm32"))]
const CHILD_INDENT: f32 = 22.0;
/// How far in from the row's right edge a trailing state dot is centred.
#[cfg(not(target_arch = "wasm32"))]
const DOT_INSET: f32 = 10.0;

/// How a tree row is marked. Exactly one row in the tree is `Destination`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(not(target_arch = "wasm32"))]
enum RowMark {
    /// The row the shell is showing: the accent bar over a card fill.
    Destination,
    /// The VM whose pages are listed: brighter than its siblings, unmarked.
    Expanded,
    /// Everything else.
    Plain,
}

/// One row of the tree. The shell's single selection idiom — a two-point
/// accent bar on the left edge over a card fill — marks the one `Destination`
/// row; an `Expanded` row is told apart by its text alone, and any row without
/// the fill tints on hover. The row is the whole click target, so a name and
/// its indent never disagree about what was hit, and it is one accessibility
/// node: a selectable named by its full label, selected only when it is the
/// `Destination`.
#[cfg(not(target_arch = "wasm32"))]
fn tree_row(
    ui: &mut egui::Ui,
    label: &str,
    indent: f32,
    mark: RowMark,
    trailing_dot: Option<Color32>,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::click());
    // Assistive technology hears exactly one selected row, the same one the
    // accent bar marks: the `Expanded` VM is the parent of the selection, not
    // the selection.
    let is_destination = match mark {
        RowMark::Destination => true,
        RowMark::Expanded | RowMark::Plain => false,
    };
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, is_destination, label)
    });
    match mark {
        RowMark::Destination => {
            ui.painter().rect_filled(rect, 6.0, BG_CARD);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.left_top(), egui::vec2(2.0, rect.height())),
                0.0,
                ACCENT_CYAN,
            );
        }
        RowMark::Expanded | RowMark::Plain => {
            if response.hovered() {
                ui.painter().rect_filled(rect, 6.0, BG_CARD.gamma_multiply(0.5));
            }
        }
    }
    let text_color = match mark {
        RowMark::Destination | RowMark::Expanded => TEXT_PRIMARY,
        RowMark::Plain => TEXT_MUTED,
    };
    // The label is cut at the row's edge — short of the dot when there is
    // one — so a long name never runs under the dot or past the panel.
    let label_right = match trailing_dot {
        Some(_) => rect.right() - 2.0 * DOT_INSET,
        None => rect.right() - SPACE_ITEM,
    };
    let label_clip = egui::Rect::from_min_max(rect.min, egui::pos2(label_right, rect.max.y));
    ui.painter().with_clip_rect(label_clip).text(
        rect.left_center() + egui::vec2(indent, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(TEXT_BODY),
        text_color,
    );
    if let Some(color) = trailing_dot {
        ui.painter()
            .circle_filled(rect.right_center() - egui::vec2(DOT_INSET, 0.0), 3.5, color);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

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
                if tree_row(ui, &entries[index].name, 8.0, vm_mark, dot).clicked() {
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
                    if tree_row(ui, page.label(), CHILD_INDENT, page_mark, None).clicked() {
                        action = Some(SidebarAction::Select(destination.select_page(page)));
                    }
                }
            }
        });
    action
}
