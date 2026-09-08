//! The pieces a pane is built from: the header every page opens with, the
//! labelled row its controls sit on, the status dot and badge, the hairlines
//! that join stacked panels, and the action tiles.

#[cfg(not(target_arch = "wasm32"))]
use crate::shell::theme::ACCENT_CYAN;
use crate::shell::theme::{
    shell_card_frame, BG_BASE, BG_CARD, BG_PANEL, SPACE_GROUP, STROKE_HAIRLINE, TEXT_BODY,
    TEXT_CAPTION, TEXT_MUTED, TEXT_PRIMARY, TEXT_TITLE,
};
use egui::{Color32, RichText, Stroke};

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
/// for, and the label is painted into that rect; an empty label therefore
/// holds its column as fully as a long one, and the caller's widgets begin on
/// the same x in every row.
pub(crate) fn field_row<R>(
    ui: &mut egui::Ui,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(FIELD_LABEL_WIDTH, ui.spacing().interact_size.y),
            egui::Sense::hover(),
        );
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

/// A row in a flat selectable list, wearing the shell's one selection idiom.
/// The selected fill is `BG_CARD`, one step above the `BG_PANEL` surface a
/// list of these rows sits on.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn hardware_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 6.0, BG_CARD);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(2.0, rect.height())),
            0.0,
            ACCENT_CYAN,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, BG_CARD.gamma_multiply(0.5));
    }
    ui.painter().text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(TEXT_BODY),
        if selected { TEXT_PRIMARY } else { TEXT_MUTED },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(crate) fn metadata_text(label: &str, value: &str) -> RichText {
    RichText::new(format!("{label}: {value}"))
        .size(11.0)
        .color(TEXT_MUTED)
}

pub(crate) fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// The small monospace face the status strip and state badges share.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn status_text(text: impl Into<String>) -> RichText {
    RichText::new(text).monospace().size(11.0)
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

/// A labelled fact in a page header: a muted caption over its value.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn home_fact(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label).size(TEXT_CAPTION).color(TEXT_MUTED));
        ui.label(RichText::new(value).size(TEXT_BODY).color(TEXT_PRIMARY));
    });
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
                    ui.label(RichText::new(title).size(16.0).strong().color(title_color));
                    ui.label(RichText::new(body).size(12.5).color(TEXT_MUTED));
                    let button = match weight {
                        ActionTileWeight::Primary => {
                            egui::Button::new(RichText::new(title).strong().color(BG_BASE))
                                .fill(accent)
                                .stroke(Stroke::NONE)
                        }
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
        ui.label(RichText::new(title).size(16.0).strong().color(TEXT_MUTED));
        ui.label(RichText::new(body).size(12.5).color(TEXT_MUTED));
        action_tile_footer(ui, egui::Button::new("Unavailable"), false);
    });
}
