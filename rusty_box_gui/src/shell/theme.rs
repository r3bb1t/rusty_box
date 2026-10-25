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
/// lifting off its surface. `CONTROL_FILL_REST` clears WCAG 1.4.11's 3:1
/// non-text contrast against `BG_CARD`, the lightest surface a control sits
/// on, and so against `BG_PANEL`, `BG_BASE` and `BG_WELL` too; and every fill
/// keeps 3:1 under the check mark drawn on it in that state's `fg_stroke` —
/// `TEXT_PRIMARY` at rest, white while hovered and while pressed — which caps
/// the ramp from above. The tests below hold the ordering and both contrasts.
pub(crate) const CONTROL_FILL_REST: Color32 = Color32::from_rgb(0x60, 0x72, 0x8A);
pub(crate) const CONTROL_FILL_HOVERED: Color32 = Color32::from_rgb(0x6E, 0x80, 0x92);
pub(crate) const CONTROL_FILL_ACTIVE: Color32 = Color32::from_rgb(0x7A, 0x8C, 0xA0);
pub(crate) const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE8, 0xEE, 0xF5);
pub(crate) const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x98, 0xA8);
pub(crate) const ACCENT_CYAN: Color32 = Color32::from_rgb(0x46, 0xD9, 0xC7);
pub(crate) const ACCENT_BLUE: Color32 = Color32::from_rgb(0x6A, 0xA8, 0xFF);
pub(crate) const ACCENT_AMBER: Color32 = Color32::from_rgb(0xF2, 0xB8, 0x4B);
pub(crate) const ACCENT_RED: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x6C);

/// The colours that say which machine the shown VM is: its primary verb, the
/// destination the shell is on, a running VM's badge and dot, a measurement
/// it publishes, and the text cursor and selection. Cyan and blue for a PC,
/// red for an Arm `virt` board, so the architecture reads at a glance. The
/// colours that say what happened — info, warning, error, paused — are not
/// the machine's and do not change with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Accent {
    /// The accent itself: the primary verb's fill, the destination's rule,
    /// a running VM's badge.
    pub(crate) strong: Color32,
    /// The data accent: a measurement the shell publishes, such as the
    /// instruction rate.
    pub(crate) data: Color32,
    /// The ground selected text sits on.
    pub(crate) selection: Color32,
}

impl Accent {
    /// A PC's, and the browser's, which builds only a PC.
    pub(crate) const PC: Self = Self {
        strong: ACCENT_CYAN,
        data: ACCENT_BLUE,
        selection: Color32::from_rgb(0x1E, 0x5F, 0x62),
    };

    /// An Arm `virt` board's.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const ARM_VIRT: Self = Self {
        strong: Color32::from_rgb(0xEF, 0x4B, 0x4B),
        data: Color32::from_rgb(0xFF, 0x8A, 0x80),
        selection: Color32::from_rgb(0x62, 0x1E, 0x24),
    };

    /// The accent of a VM that is `machine`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const fn of(machine: crate::config::MachineKind) -> Self {
        match machine {
            crate::config::MachineKind::Pc => Self::PC,
            crate::config::MachineKind::ArmVirt => Self::ARM_VIRT,
        }
    }

    /// The accent the shell is drawn in.
    pub(crate) fn current(ctx: &egui::Context) -> Self {
        ctx.data(|data| data.get_temp::<Self>(Self::id()))
            .unwrap_or(Self::PC)
    }

    /// Draws the shell in this accent from this frame on: the widgets that
    /// read [`Accent::current`], and egui's own text cursor and selection.
    /// Nothing is touched when both already are this accent's; the style is
    /// read rather than trusted, so a reset of egui's visuals made elsewhere
    /// is undone here.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn apply(self, ctx: &egui::Context) {
        let visuals = &ctx.style_of(egui::Theme::Dark).visuals;
        let drawn = visuals.text_cursor.stroke.color == self.strong
            && visuals.selection.bg_fill == self.selection
            && visuals.selection.stroke.color == self.strong;
        if !drawn || ctx.data(|data| data.get_temp::<Self>(Self::id())) != Some(self) {
            self.write(ctx);
        }
    }

    fn write(self, ctx: &egui::Context) {
        use egui::{style::Selection, Theme};
        ctx.data_mut(|data| data.insert_temp(Self::id(), self));
        ctx.style_mut_of(Theme::Dark, |style| {
            style.visuals.text_cursor.stroke.color = self.strong;
            style.visuals.selection = Selection {
                bg_fill: self.selection,
                stroke: Stroke::new(1.0_f32, self.strong),
            };
        });
    }

    fn id() -> egui::Id {
        egui::Id::new("rusty_box_gui::shell::theme::Accent")
    }
}

/// The shell's five type sizes. Nothing outside this list is a legal font size
/// in this crate's shell code — the browser shell sets its own, and the Console
/// page embeds `RustyBoxApp::ui_inner`, which paints its own sizes — and two
/// weights carry every distinction: regular, and `.strong()`.
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SPACE_PAGE: i8 = 20;
/// The page and card margins of a page that has little room (a short or
/// narrow window): half of the roomy ones, so listings keep their rows.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SPACE_PAGE_COMPACT: i8 = 8;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SPACE_CARD_COMPACT: i8 = 8;

pub(crate) fn configure_shell_style(ctx: &egui::Context) {
    use egui::{Theme, ThemePreference, Vec2};

    ctx.set_theme(ThemePreference::Dark);
    // The text cursor and selection are the accent's, a PC's until a shown
    // VM says otherwise (`Accent::apply`).
    Accent::PC.write(ctx);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.visuals.panel_fill = BG_BASE;
        style.visuals.window_fill = BG_PANEL;
        style.visuals.extreme_bg_color = BG_WELL;
        style.visuals.hyperlink_color = ACCENT_BLUE;
        style.visuals.widgets.noninteractive.bg_fill = BG_PANEL;
        // `bg_fill` is a control's own box, not its surface: the checkbox
        // square, the radio disc, the slider rail and handle, and a solid
        // scroll bar's handle over its `BG_WELL` track. Each of the three
        // interactive fills clears 3:1 against every surface a control sits on
        // — the page (`BG_BASE`), a panel (`BG_PANEL`), a card (`BG_CARD`) and
        // the input well (`BG_WELL`) — so a resting box reads as a control
        // wherever it lands; the ramp rises rest → hover → press; and each fill
        // stays dark enough for the check mark in that state's `fg_stroke` —
        // set below for rest and hover, egui's default white when pressed — to
        // keep 3:1 over it. `bg_stroke` keeps egui's default because it frames
        // every button, and buttons fill with `weak_bg_fill`, which this ramp
        // does not touch.
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
    shell_card_frame_with_margin(SPACE_CARD)
}

/// A card's frame with `margin` points inside its border.
pub(crate) fn shell_card_frame_with_margin(margin: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(margin))
}

#[cfg(test)]
mod tests {
    use super::{
        configure_shell_style, Accent, ACCENT_CYAN, BG_BASE, BG_CARD, BG_PANEL, BG_WELL,
        CONTROL_FILL_ACTIVE, CONTROL_FILL_HOVERED, CONTROL_FILL_REST, TEXT_PRIMARY,
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

    /// WCAG contrast ratio of two opaque colours, from 1.0 for a colour against
    /// itself to 21.0 for black against white.
    fn contrast_ratio(a: Color32, b: Color32) -> f64 {
        let a = relative_luminance(a);
        let b = relative_luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// WCAG 1.4.11's floor for a control's boundary and its state indicator.
    const NON_TEXT_CONTRAST: f64 = 3.0;

    /// The interactive widget visuals exactly as the shell configures them,
    /// so the assertions read the fills and strokes a control is drawn with,
    /// not the constants they are meant to be wired to.
    fn configured_widgets() -> egui::style::Widgets {
        let ctx = egui::Context::default();
        configure_shell_style(&ctx);
        ctx.style_of(egui::Theme::Dark).visuals.widgets.clone()
    }

    #[test]
    fn a_resting_control_box_clears_non_text_contrast_against_a_card() {
        let box_fill = configured_widgets().inactive.bg_fill;
        let ratio = contrast_ratio(box_fill, BG_CARD);
        assert!(
            ratio >= NON_TEXT_CONTRAST,
            "a resting box ({box_fill:?}) against BG_CARD ({BG_CARD:?}) is {ratio:.3}:1; \
             WCAG 1.4.11 asks {NON_TEXT_CONTRAST}:1"
        );
    }

    #[test]
    fn a_check_mark_clears_non_text_contrast_against_its_box_in_every_state() {
        let widgets = configured_widgets();
        for (state, visuals) in [
            ("resting", &widgets.inactive),
            ("hovered", &widgets.hovered),
            ("pressed", &widgets.active),
        ] {
            let mark = visuals.fg_stroke.color;
            let box_fill = visuals.bg_fill;
            let ratio = contrast_ratio(mark, box_fill);
            assert!(
                ratio >= NON_TEXT_CONTRAST,
                "the {state} check mark ({mark:?}) against its box ({box_fill:?}) is {ratio:.3}:1; \
                 WCAG 1.4.11 asks {NON_TEXT_CONTRAST}:1"
            );
        }
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

    /// WCAG 1.4.3's floor for body text.
    const TEXT_CONTRAST: f64 = 4.5;

    /// Every machine's accent is legible where the shell draws it: the
    /// primary verb's `BG_BASE` text on its fill, the destination's rule
    /// against a card, the published rate on the panel it sits on, and
    /// selected text on its ground.
    #[test]
    fn every_machines_accent_clears_contrast_where_it_is_drawn() {
        for machine in crate::config::MachineKind::ALL {
            let accent = Accent::of(machine);
            // The listing's current row: the accent at 14% over a card,
            // blended as egui blends a premultiplied colour.
            let tint = accent.strong.gamma_multiply(0.14);
            let current_row = Color32::from_rgb(
                (f32::from(tint.r()) + f32::from(BG_CARD.r()) * 0.86) as u8,
                (f32::from(tint.g()) + f32::from(BG_CARD.g()) * 0.86) as u8,
                (f32::from(tint.b()) + f32::from(BG_CARD.b()) * 0.86) as u8,
            );
            for (what, a, b, floor) in [
                ("primary verb text", BG_BASE, accent.strong, TEXT_CONTRAST),
                ("destination rule", accent.strong, BG_CARD, NON_TEXT_CONTRAST),
                ("published rate", accent.data, BG_PANEL, TEXT_CONTRAST),
                ("selected text", TEXT_PRIMARY, accent.selection, TEXT_CONTRAST),
                ("current row text", TEXT_PRIMARY, current_row, TEXT_CONTRAST),
            ] {
                let ratio = contrast_ratio(a, b);
                assert!(ratio >= floor, "{machine:?} {what}: {ratio:.3}:1 < {floor}:1");
            }
        }
    }

    /// An Arm board's accent is red and a PC's is not, and the shell's text
    /// cursor and selection follow the accent applied.
    #[test]
    fn an_arm_boards_accent_is_red_and_egui_follows_it() {
        // Red is a hue within 20 degrees of 0, at a saturation that is not
        // grey: the red channel leads, and the other two sit well below it
        // and close to each other.
        let red = |color: Color32| {
            let [r, g, b, _] = color.to_array().map(f32::from);
            let (min, chroma) = (g.min(b), r - g.max(b));
            let hue = 60.0 * (g - b) / (r - min);
            chroma > 0.0 && (r - min) / r > 0.3 && hue.abs() <= 20.0
        };
        assert!(red(Accent::ARM_VIRT.strong) && red(Accent::ARM_VIRT.data));
        assert!(!red(Accent::PC.strong) && !red(Accent::PC.data));

        let ctx = egui::Context::default();
        configure_shell_style(&ctx);
        assert_eq!(Accent::current(&ctx), Accent::PC);
        Accent::ARM_VIRT.apply(&ctx);
        assert_eq!(Accent::current(&ctx), Accent::ARM_VIRT);
        let visuals = ctx.style_of(egui::Theme::Dark).visuals.clone();
        assert_eq!(visuals.text_cursor.stroke.color, Accent::ARM_VIRT.strong);
        assert_eq!(visuals.selection.bg_fill, Accent::ARM_VIRT.selection);
        Accent::PC.apply(&ctx);
        assert_eq!(ctx.style_of(egui::Theme::Dark).visuals.selection.stroke.color, ACCENT_CYAN);
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
