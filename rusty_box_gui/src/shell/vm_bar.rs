//! The one bar above a page: what VM this is, what state it is in, and the
//! verbs that change that state.
//!
//! The bar draws from borrowed state and reports the click; the app decides
//! what the click means. The power verbs are always present and enabled by
//! the machine's state alone; the three controls that act on the guest's
//! console (Ctrl+Alt+Del, mouse capture, the serial pane) appear only while
//! the console page is shown — in the bar while it has room for them, inside
//! the `⋯` overflow when it does not. The `⋯` at the far right always holds
//! the two verbs that belong to the application rather than to the VM.

#[cfg(not(target_arch = "wasm32"))]
use crate::shell::destination::VmBarAction;
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::theme::{ACCENT_CYAN, BG_BASE, BG_PANEL, TEXT_BODY, TEXT_CAPTION, TEXT_PRIMARY};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::widgets::{hairline_below, ShellStateBadge};
#[cfg(not(target_arch = "wasm32"))]
use egui::{RichText, Stroke};

/// What the bar needs to know to draw itself. Borrowed, so the bar cannot
/// change any of it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct VmBarState<'a> {
    pub(crate) name: &'a str,
    pub(crate) badge: ShellStateBadge,
    pub(crate) running: bool,
    pub(crate) start_pending: bool,
    pub(crate) on_console: bool,
    pub(crate) serial_shown: bool,
    pub(crate) mouse_captured: bool,
}

/// The verbs' captions, named once because the bar measures each caption
/// before it draws the button that carries it.
#[cfg(not(target_arch = "wasm32"))]
const OVERFLOW: &str = "⋯";
#[cfg(not(target_arch = "wasm32"))]
const RESTART: &str = "↻ Restart";
#[cfg(not(target_arch = "wasm32"))]
const POWER_OFF: &str = "■ Power off";
#[cfg(not(target_arch = "wasm32"))]
const POWER_ON: &str = "▶ Power on";
#[cfg(not(target_arch = "wasm32"))]
const CTRL_ALT_DEL: &str = "Ctrl+Alt+Del";
#[cfg(not(target_arch = "wasm32"))]
const CAPTURE_MOUSE: &str = "Capture mouse";
#[cfg(not(target_arch = "wasm32"))]
const RELEASE_MOUSE: &str = "Release mouse";
#[cfg(not(target_arch = "wasm32"))]
const SHOW_SERIAL: &str = "Show serial";
#[cfg(not(target_arch = "wasm32"))]
const HIDE_SERIAL: &str = "Hide serial";

/// The room the rule between the console's controls and the power verbs takes.
#[cfg(not(target_arch = "wasm32"))]
const SEPARATOR_SPACING: f32 = 6.0;

/// The width `text` takes laid out on one line in `font`, which is how a
/// label sizes itself.
#[cfg(not(target_arch = "wasm32"))]
fn text_width(ui: &egui::Ui, text: impl Into<egui::WidgetText>, font: egui::TextStyle) -> f32 {
    text.into()
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font)
        .size()
        .x
}

/// The width a button captioned `text` takes: the laid-out caption plus the
/// button padding on both sides, which is how `egui::Button` sizes itself.
#[cfg(not(target_arch = "wasm32"))]
fn button_width(ui: &egui::Ui, text: impl Into<egui::WidgetText>) -> f32 {
    text_width(ui, text, egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.x
}

/// The width the verbs that are always in the bar take together: the `⋯`
/// overflow and the three power verbs, with the gaps between them.
#[cfg(not(target_arch = "wasm32"))]
fn power_verbs_width(ui: &egui::Ui) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    button_width(ui, OVERFLOW)
        + button_width(ui, RESTART)
        + button_width(ui, POWER_OFF)
        + button_width(ui, RichText::new(POWER_ON).strong())
        + 3.0 * gap
}

/// The width the console's three controls and their separator take when they
/// sit in the bar. Each toggle is measured at its wider caption, so the bar
/// does not fold and unfold as a toggle flips.
#[cfg(not(target_arch = "wasm32"))]
fn console_controls_width(ui: &egui::Ui) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    button_width(ui, CTRL_ALT_DEL)
        + button_width(ui, CAPTURE_MOUSE).max(button_width(ui, RELEASE_MOUSE))
        + button_width(ui, SHOW_SERIAL).max(button_width(ui, HIDE_SERIAL))
        + SEPARATOR_SPACING
        + 4.0 * gap
}

/// The three verbs that act on the guest's console, drawn wherever the bar
/// has room for them. Reports which one was clicked.
#[cfg(not(target_arch = "wasm32"))]
fn console_controls(ui: &mut egui::Ui, state: &VmBarState<'_>) -> Option<VmBarAction> {
    let mut action = None;
    if ui
        .add_enabled(state.running, egui::Button::new(CTRL_ALT_DEL))
        .clicked()
    {
        action = Some(VmBarAction::SendCtrlAltDel);
    }
    let capture = if state.mouse_captured {
        RELEASE_MOUSE
    } else {
        CAPTURE_MOUSE
    };
    if ui
        .add_enabled(state.running, egui::Button::new(capture))
        .clicked()
    {
        action = Some(VmBarAction::ToggleMouseCapture);
    }
    let serial = if state.serial_shown {
        HIDE_SERIAL
    } else {
        SHOW_SERIAL
    };
    if ui.button(serial).clicked() {
        action = Some(VmBarAction::ToggleSerial);
    }
    action
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_vm_bar(ui: &mut egui::Ui, state: VmBarState<'_>) -> Option<VmBarAction> {
    let mut action = None;
    let bar = egui::Panel::top("vm_bar")
        .exact_size(36.0)
        .frame(
            egui::Frame::new()
                .fill(BG_PANEL)
                .inner_margin(egui::Margin::symmetric(10, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let gap = ui.spacing().item_spacing.x;
                let verbs_width = power_verbs_width(ui);
                if ui
                    .button("☰")
                    .on_hover_text("Show or hide the VM tree")
                    .clicked()
                {
                    action = Some(VmBarAction::ToggleSidebar);
                }
                let badge = RichText::new(state.badge.label)
                    .size(TEXT_CAPTION)
                    .color(state.badge.color);
                // The name is the one elastic element: it is cut to whatever is
                // left once the badge and the power verbs have their room.
                let badge_width = text_width(ui, badge.clone(), egui::TextStyle::Body);
                let name_width =
                    (ui.available_width() - badge_width - verbs_width - 2.0 * gap).max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(name_width, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(state.name)
                                    .size(TEXT_BODY)
                                    .strong()
                                    .color(TEXT_PRIMARY),
                            )
                            .truncate(),
                        );
                    },
                );
                ui.label(badge);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The console's controls sit in the bar only while the whole
                    // right-hand group fits; otherwise they move into `⋯`.
                    let folded = state.on_console
                        && ui.available_width() < verbs_width + console_controls_width(ui);
                    ui.menu_button(OVERFLOW, |ui| {
                        if folded {
                            if let Some(chosen) = console_controls(ui, &state) {
                                action = Some(chosen);
                                ui.close();
                            }
                            ui.separator();
                        }
                        if ui.button("About Rusty Box Workstation").clicked() {
                            action = Some(VmBarAction::ShowAbout);
                            ui.close();
                        }
                        if ui.button("Quit").clicked() {
                            action = Some(VmBarAction::Quit);
                            ui.close();
                        }
                    });
                    if state.on_console && !folded {
                        if let Some(chosen) = console_controls(ui, &state) {
                            action = Some(chosen);
                        }
                        ui.add(egui::Separator::default().spacing(SEPARATOR_SPACING));
                    }
                    if ui
                        .add_enabled(state.running, egui::Button::new(RESTART))
                        .clicked()
                    {
                        action = Some(VmBarAction::Restart);
                    }
                    if ui
                        .add_enabled(state.running, egui::Button::new(POWER_OFF))
                        .clicked()
                    {
                        action = Some(VmBarAction::PowerOff);
                    }
                    // The filled treatment marks the one verb that can be taken
                    // now; a Power on that cannot be pressed rests as a ghost
                    // beside its neighbours.
                    let can_start = !state.running && !state.start_pending;
                    let power_on = if can_start {
                        egui::Button::new(RichText::new(POWER_ON).strong().color(BG_BASE))
                            .fill(ACCENT_CYAN)
                            .stroke(Stroke::NONE)
                    } else {
                        egui::Button::new(POWER_ON)
                    };
                    if ui.add_enabled(can_start, power_on).clicked() {
                        action = Some(VmBarAction::PowerOn);
                    }
                });
            });
        });
    hairline_below(ui, bar.response.rect);
    action
}
