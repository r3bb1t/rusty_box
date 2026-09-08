//! The shell's palette, its type and spacing scale, and the two functions that
//! apply them: one to the egui context, one to a card frame.

use egui::{Color32, Stroke};

pub(crate) const BG_BASE: Color32 = Color32::from_rgb(0x0B, 0x0F, 0x14);
pub(crate) const BG_PANEL: Color32 = Color32::from_rgb(0x11, 0x18, 0x21);
pub(crate) const BG_CARD: Color32 = Color32::from_rgb(0x17, 0x21, 0x2B);
pub(crate) const STROKE_HAIRLINE: Color32 = Color32::from_rgb(0x26, 0x34, 0x43);
pub(crate) const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE8, 0xEE, 0xF5);
pub(crate) const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x98, 0xA8);
pub(crate) const ACCENT_CYAN: Color32 = Color32::from_rgb(0x46, 0xD9, 0xC7);
pub(crate) const ACCENT_BLUE: Color32 = Color32::from_rgb(0x6A, 0xA8, 0xFF);
pub(crate) const ACCENT_AMBER: Color32 = Color32::from_rgb(0xF2, 0xB8, 0x4B);
pub(crate) const ACCENT_RED: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x6C);

/// The shell's five type sizes. Nothing outside this list is a legal font size
/// in a pane, and two weights carry every distinction: regular, and `.strong()`.
pub(crate) const TEXT_DISPLAY: f32 = 22.0;
pub(crate) const TEXT_TITLE: f32 = 16.0;
pub(crate) const TEXT_BODY: f32 = 14.0;
pub(crate) const TEXT_SECONDARY: f32 = 12.5;
pub(crate) const TEXT_CAPTION: f32 = 11.0;

/// Vertical rhythm, in multiples of four: between items inside one group,
/// between groups, inside a card's border, and around a page's content.
pub(crate) const SPACE_ITEM: f32 = 8.0;
pub(crate) const SPACE_GROUP: f32 = 12.0;
pub(crate) const SPACE_CARD: i8 = 16;
pub(crate) const SPACE_PAGE: i8 = 20;

pub(crate) fn configure_shell_style(ctx: &egui::Context) {
    use egui::{style::Selection, Theme, ThemePreference, Vec2};

    ctx.set_theme(ThemePreference::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.visuals.panel_fill = BG_BASE;
        style.visuals.window_fill = BG_PANEL;
        style.visuals.extreme_bg_color = Color32::from_rgb(0x07, 0x0A, 0x0E);
        style.visuals.hyperlink_color = ACCENT_BLUE;
        style.visuals.text_cursor.stroke.color = ACCENT_CYAN;
        style.visuals.selection = Selection {
            bg_fill: Color32::from_rgb(0x1E, 0x5F, 0x62),
            stroke: Stroke::new(1.0_f32, ACCENT_CYAN),
        };
        style.visuals.widgets.noninteractive.bg_fill = BG_PANEL;
        style.visuals.widgets.inactive.bg_fill = BG_CARD;
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x1D, 0x2A, 0x36);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(0x21, 0x35, 0x43);
        style.visuals.widgets.inactive.fg_stroke.color = TEXT_PRIMARY;
        style.visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
    });
}

pub(crate) fn shell_card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(SPACE_CARD))
}
