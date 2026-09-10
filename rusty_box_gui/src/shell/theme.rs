//! The shell's palette, its type and spacing scale, and the two functions that
//! apply them: one to the egui context, one to a card frame.

use egui::{Color32, Stroke};

pub(crate) const BG_BASE: Color32 = Color32::from_rgb(0x0B, 0x0F, 0x14);
pub(crate) const BG_PANEL: Color32 = Color32::from_rgb(0x11, 0x18, 0x21);
pub(crate) const BG_CARD: Color32 = Color32::from_rgb(0x17, 0x21, 0x2B);
/// The input well: the ground a text field's text and a scroll bar's track
/// sit in, darker than every surface around it.
pub(crate) const BG_WELL: Color32 = Color32::from_rgb(0x07, 0x0A, 0x0E);
pub(crate) const STROKE_HAIRLINE: Color32 = Color32::from_rgb(0x26, 0x34, 0x43);
/// A control's own box — a checkbox's square, a radio's disc, a slider's
/// handle — at rest, while the pointer is over it, and while it is pressed or
/// focused. The three rise in that order, so interaction reads as the control
/// lifting off its surface, and `CONTROL_FILL_REST` is lighter than every
/// surface a control sits on — `BG_BASE`, `BG_PANEL`, `BG_CARD` and `BG_WELL`
/// — so a resting box is visible wherever it lands. The tests below hold both
/// orderings.
pub(crate) const CONTROL_FILL_REST: Color32 = Color32::from_rgb(0x26, 0x34, 0x43);
pub(crate) const CONTROL_FILL_HOVERED: Color32 = Color32::from_rgb(0x2E, 0x3F, 0x51);
pub(crate) const CONTROL_FILL_ACTIVE: Color32 = Color32::from_rgb(0x36, 0x49, 0x5D);
pub(crate) const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE8, 0xEE, 0xF5);
pub(crate) const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x98, 0xA8);
pub(crate) const ACCENT_CYAN: Color32 = Color32::from_rgb(0x46, 0xD9, 0xC7);
pub(crate) const ACCENT_BLUE: Color32 = Color32::from_rgb(0x6A, 0xA8, 0xFF);
pub(crate) const ACCENT_AMBER: Color32 = Color32::from_rgb(0xF2, 0xB8, 0x4B);
pub(crate) const ACCENT_RED: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x6C);

/// The shell's five type sizes. Nothing outside this list is a legal font size
/// in this crate's shell code — the browser shell sets its own, and the Console
/// page embeds `RustyBoxApp::ui_inner`, which paints its own sizes — and two
/// weights carry every distinction: regular, and `.strong()`.
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
        style.visuals.extreme_bg_color = BG_WELL;
        style.visuals.hyperlink_color = ACCENT_BLUE;
        style.visuals.text_cursor.stroke.color = ACCENT_CYAN;
        style.visuals.selection = Selection {
            bg_fill: Color32::from_rgb(0x1E, 0x5F, 0x62),
            stroke: Stroke::new(1.0_f32, ACCENT_CYAN),
        };
        style.visuals.widgets.noninteractive.bg_fill = BG_PANEL;
        // `bg_fill` is a control's own box, not its surface: the checkbox
        // square, the radio disc, the slider rail and handle, and a solid
        // scroll bar's handle over its `BG_WELL` track. Each of the three
        // interactive fills is lighter than every surface a control sits on —
        // the page (`BG_BASE`), a panel (`BG_PANEL`), a card (`BG_CARD`) and
        // the input well (`BG_WELL`) — so a resting box is visible wherever it
        // lands, and the ramp rises rest → hover → press. `bg_stroke` keeps
        // egui's default because it frames every button, and buttons fill with
        // `weak_bg_fill`, which this ramp does not touch.
        style.visuals.widgets.inactive.bg_fill = CONTROL_FILL_REST;
        style.visuals.widgets.hovered.bg_fill = CONTROL_FILL_HOVERED;
        style.visuals.widgets.active.bg_fill = CONTROL_FILL_ACTIVE;
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

#[cfg(test)]
mod tests {
    use super::{
        BG_BASE, BG_CARD, BG_PANEL, BG_WELL, CONTROL_FILL_ACTIVE, CONTROL_FILL_HOVERED,
        CONTROL_FILL_REST,
    };
    use egui::Color32;

    /// WCAG relative luminance of an opaque sRGB colour: 0.0 is black, 1.0 is
    /// white. Each channel is linearised before the weighted sum, so two
    /// colours compare as the eye orders them, not as their hex digits do.
    fn relative_luminance(color: Color32) -> f64 {
        fn linear(channel: u8) -> f64 {
            let srgb = f64::from(channel) / 255.0;
            if srgb <= 0.04045 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            }
        }
        let [r, g, b, _] = color.to_array();
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    #[test]
    fn a_resting_control_box_is_lighter_than_every_surface_it_sits_on() {
        let rest = relative_luminance(CONTROL_FILL_REST);
        for (name, surface) in [
            ("BG_CARD", BG_CARD),
            ("BG_PANEL", BG_PANEL),
            ("BG_BASE", BG_BASE),
            ("BG_WELL", BG_WELL),
        ] {
            let surface = relative_luminance(surface);
            assert!(
                rest > surface,
                "CONTROL_FILL_REST ({rest:.4}) must be lighter than {name} ({surface:.4})"
            );
        }
    }

    #[test]
    fn the_control_fill_ramp_rises_from_rest_through_hover_to_press() {
        let rest = relative_luminance(CONTROL_FILL_REST);
        let hovered = relative_luminance(CONTROL_FILL_HOVERED);
        let active = relative_luminance(CONTROL_FILL_ACTIVE);
        assert!(
            rest < hovered,
            "CONTROL_FILL_REST ({rest:.4}) must be darker than CONTROL_FILL_HOVERED ({hovered:.4})"
        );
        assert!(
            hovered < active,
            "CONTROL_FILL_HOVERED ({hovered:.4}) must be darker than CONTROL_FILL_ACTIVE ({active:.4})"
        );
    }
}
