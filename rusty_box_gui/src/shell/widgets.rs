//! The pieces a pane is built from: the header a settings pane opens with,
//! the labelled row its controls sit on, the selection row the sidebar tree
//! and the Hardware device list share, the status dot and badge, the
//! hairlines that join stacked panels, and the action tiles.

#[cfg(not(target_arch = "wasm32"))]
use crate::shell::theme::SPACE_ITEM;
use crate::shell::theme::{
    shell_card_frame, ACCENT_CYAN, BG_BASE, BG_CARD, BG_PANEL, SPACE_GROUP, STROKE_HAIRLINE,
    TEXT_BODY, TEXT_CAPTION, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TITLE,
};
use egui::{Color32, RichText, Stroke, WidgetInfo, WidgetType};

/// Every pane opens with exactly this: the pane's name over one line saying
/// what it does, then the gap that separates a header from its content.
pub(crate) fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(TEXT_TITLE).strong().color(TEXT_PRIMARY));
    ui.label(RichText::new(subtitle).size(TEXT_CAPTION).color(TEXT_MUTED));
    ui.add_space(SPACE_GROUP);
}

/// The width every label column in the shell reserves, so that two inputs in
/// the same pane start on the same x whatever their labels say. It holds the
/// longest label the panes carry, `Display resolution`, at `TEXT_BODY`.
pub(crate) const FIELD_LABEL_WIDTH: f32 = 112.0;

/// One labelled control on a pane's grid. The label column is reserved with
/// `allocate_exact_size`, which advances the row by exactly the width asked
/// for, and the label is painted into that rect and registered as the
/// column's accessible name; an empty label therefore holds its column as
/// fully as a long one, names nothing, and the caller's widgets begin on the
/// same x in every row.
pub(crate) fn field_row<R>(
    ui: &mut egui::Ui,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(FIELD_LABEL_WIDTH, ui.spacing().interact_size.y),
            egui::Sense::hover(),
        );
        // A nameless label is noise to a screen reader, so an empty caption
        // registers no node at all.
        if !label.is_empty() {
            let enabled = ui.is_enabled();
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, enabled, label));
        }
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(TEXT_BODY),
            TEXT_MUTED,
        );
        add_contents(ui)
    })
    .inner
}

/// The width `text` takes laid out on one line in `font`, which is how a
/// label sizes itself.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn text_width(
    ui: &egui::Ui,
    text: impl Into<egui::WidgetText>,
    font: egui::TextStyle,
) -> f32 {
    text.into()
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font)
        .size()
        .x
}

/// The width a button captioned `text` takes: the laid-out caption plus the
/// button padding on both sides, which is how `egui::Button` sizes itself.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn button_width(ui: &egui::Ui, text: impl Into<egui::WidgetText>) -> f32 {
    text_width(ui, text, egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.x
}

/// The caption of every button that opens the host's file picker, named once
/// because a path row measures it before drawing the field beside it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const BROWSE: &str = "Browse…";

/// The widest a path's text field is drawn, so a wide window does not stretch
/// it across the pane.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const PATH_FIELD_MAX_WIDTH: f32 = 360.0;

/// The width a path's text field takes beside its `BROWSE` button: what is
/// left of the row once the button and the gap before it have their room,
/// capped at `PATH_FIELD_MAX_WIDTH`. The field is the row's one elastic
/// element, so the button stays inside the row however narrow the pane.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn path_field_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - button_width(ui, BROWSE) - ui.spacing().item_spacing.x)
        .min(PATH_FIELD_MAX_WIDTH)
        .max(0.0)
}

/// The height of every row in a selectable list.
#[cfg(not(target_arch = "wasm32"))]
const ROW_HEIGHT: f32 = 24.0;
/// How far in from the row's left edge a top-level label starts: a VM in the
/// tree, or a device in the Hardware list.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const ROOT_INDENT: f32 = 8.0;
/// How far in the label of a row nested under another starts: a page under
/// its VM.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const CHILD_INDENT: f32 = 22.0;
/// How far in from the row's right edge a trailing state dot is centred.
#[cfg(not(target_arch = "wasm32"))]
const DOT_INSET: f32 = 10.0;

/// How a selection row is marked. Exactly one row in a list is `Destination`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum RowMark {
    /// The row the shell is showing: the accent bar over a card fill.
    Destination,
    /// The VM whose pages are listed: brighter than its siblings, unmarked.
    Expanded,
    /// Everything else.
    Plain,
}

/// One row of a selectable list — the sidebar tree and the Hardware device
/// list are both built from it — wearing the shell's single selection idiom:
/// a two-point accent bar on the left edge over a `BG_CARD` fill, one step
/// above the `BG_PANEL` surface a list of these rows sits on, marks the one
/// `Destination` row; an `Expanded` row is told apart by its text alone, and
/// any row without the fill tints on hover. The row is the whole click
/// target, so a name and its indent never disagree about what was hit, and it
/// is one accessibility node: a selectable named by its full label, selected
/// only when it is the `Destination`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn selection_row(
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

pub(crate) fn metadata_text(label: &str, value: &str) -> RichText {
    RichText::new(format!("{label}: {value}"))
        .size(TEXT_CAPTION)
        .color(TEXT_MUTED)
}

pub(crate) fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// The small monospace face the status strip and state badges share.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn status_text(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().size(TEXT_CAPTION)
}

/// The one-word state the status strip, the sidebar and the VM bar all show,
/// with the accent that state owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct ShellStateBadge {
    pub(crate) label: &'static str,
    pub(crate) color: Color32,
}

/// A one-point rule along the bottom edge of a panel's `rect`, so two stacked
/// panels meet on a single line instead of two framed strokes.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn hairline_below(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0_f32, STROKE_HAIRLINE),
    );
}

/// The same rule along the top edge, for a panel that sits below its neighbour.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn hairline_above(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0_f32, STROKE_HAIRLINE),
    );
}

/// The gap between two facts in a header's facts row, wide enough that each
/// caption-over-value pair reads as its own column.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const HOME_FACT_GAP: f32 = 28.0;

/// A labelled fact in a page header: a muted caption over its value.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn home_fact(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label).size(TEXT_CAPTION).color(TEXT_MUTED));
        ui.label(RichText::new(value).size(TEXT_BODY).color(TEXT_PRIMARY));
    });
}

/// The one filled button a pane carries: its primary verb, as strong text in
/// `BG_BASE` on an `ACCENT_CYAN` fill with no stroke. Every other button in
/// the shell rests on the hairline, so the fill alone says which verb is the
/// pane's.
pub(crate) fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).strong().color(BG_BASE))
        .fill(ACCENT_CYAN)
        .stroke(Stroke::NONE)
}

/// Which card in a row of actions carries the eye. The primary card keeps its
/// accent outline at rest and a filled accent button; a secondary card rests
/// on the hairline and only takes its accent when hovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionTileWeight {
    Primary,
    Secondary,
}

/// The card's resting height; every tile in a row shares it so the row reads
/// as one band.
pub(crate) const ACTION_TILE_MIN_HEIGHT: f32 = 112.0;

pub(crate) fn action_tile(
    ui: &mut egui::Ui,
    title: &str,
    body: &str,
    accent: Color32,
    weight: ActionTileWeight,
    on_click: impl FnMut(),
) {
    action_tile_enabled(ui, title, body, accent, weight, true, on_click);
}

/// An action card that is the target as a whole: the frame senses the click,
/// hover tints the fill toward the accent, and the button inside is the same
/// action spelled out.
pub(crate) fn action_tile_enabled(
    ui: &mut egui::Ui,
    title: &str,
    body: &str,
    accent: Color32,
    weight: ActionTileWeight,
    enabled: bool,
    mut on_click: impl FnMut(),
) {
    let egui::InnerResponse {
        inner: button_clicked,
        response: card,
    } = ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(title)
            .sense(egui::Sense::click()),
        |ui| {
            let hovered = enabled && ui.response().hovered();
            let stroke_color = match (weight, enabled, hovered) {
                (_, false, _) => STROKE_HAIRLINE,
                (ActionTileWeight::Primary, true, _) => accent,
                (ActionTileWeight::Secondary, true, true) => accent,
                (ActionTileWeight::Secondary, true, false) => STROKE_HAIRLINE,
            };
            let fill = if hovered {
                BG_CARD.lerp_to_gamma(accent, 0.06)
            } else {
                BG_CARD
            };
            shell_card_frame()
                .fill(fill)
                .stroke(Stroke::new(1.0_f32, stroke_color))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(ACTION_TILE_MIN_HEIGHT);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let title_color = if enabled { TEXT_PRIMARY } else { TEXT_MUTED };
                    ui.label(
                        RichText::new(title)
                            .size(TEXT_TITLE)
                            .strong()
                            .color(title_color),
                    );
                    ui.label(RichText::new(body).size(TEXT_SECONDARY).color(TEXT_MUTED));
                    let button = match weight {
                        ActionTileWeight::Primary => primary_button(title),
                        ActionTileWeight::Secondary => {
                            egui::Button::new(RichText::new(title).color(TEXT_PRIMARY))
                                .fill(BG_PANEL)
                                .stroke(Stroke::new(1.0_f32, accent))
                        }
                    };
                    action_tile_footer(ui, button, enabled).clicked()
                })
                .inner
        },
    );
    let card = if enabled {
        card.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        card
    };
    if button_clicked || (enabled && card.clicked()) {
        on_click();
    }
}

/// Places a tile's button on the card's bottom edge, so a row of tiles whose
/// bodies wrap to different line counts still shares one button baseline.
fn action_tile_footer(
    ui: &mut egui::Ui,
    button: egui::Button<'_>,
    enabled: bool,
) -> egui::Response {
    const FOOTER_MIN_HEIGHT: f32 = 36.0;
    // The cursor already sits one item-spacing below the last label; measuring
    // from the content's top edge is what stays true after `set_min_height`.
    let used = ui.cursor().top() - ui.max_rect().top();
    let footer = (ACTION_TILE_MIN_HEIGHT - used).max(FOOTER_MIN_HEIGHT);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), footer),
        egui::Layout::bottom_up(egui::Align::Min).with_cross_justify(true),
        |ui| ui.add_enabled(enabled, button),
    )
    .inner
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn disabled_tile(ui: &mut egui::Ui, title: &str, body: &str) {
    shell_card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(ACTION_TILE_MIN_HEIGHT);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new(title).size(TEXT_TITLE).strong().color(TEXT_MUTED));
        ui.label(RichText::new(body).size(TEXT_SECONDARY).color(TEXT_MUTED));
        action_tile_footer(ui, egui::Button::new("Unavailable"), false);
    });
}
