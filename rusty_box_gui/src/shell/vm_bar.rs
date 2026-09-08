//! The one bar above a page: what VM this is, what state it is in, and the
//! verbs that change that state.
//!
//! The bar draws from borrowed state and reports the click; the app decides
//! what the click means. The power verbs are always present and enabled by
//! the machine's state alone; the three controls that act on the guest's
//! console (Ctrl+Alt+Del, mouse capture, the serial pane) appear only while
//! the console page is shown. The `⋯` overflow at the far right holds the two
//! verbs that belong to the application rather than to the VM.

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
                if ui
                    .button("☰")
                    .on_hover_text("Show or hide the VM tree")
                    .clicked()
                {
                    action = Some(VmBarAction::ToggleSidebar);
                }
                ui.label(
                    RichText::new(state.name)
                        .size(TEXT_BODY)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.label(
                    RichText::new(state.badge.label)
                        .size(TEXT_CAPTION)
                        .color(state.badge.color),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("⋯", |ui| {
                        if ui.button("About Rusty Box Workstation").clicked() {
                            action = Some(VmBarAction::ShowAbout);
                            ui.close();
                        }
                        if ui.button("Quit").clicked() {
                            action = Some(VmBarAction::Quit);
                            ui.close();
                        }
                    });
                    if state.on_console {
                        if ui
                            .add_enabled(state.running, egui::Button::new("Ctrl+Alt+Del"))
                            .clicked()
                        {
                            action = Some(VmBarAction::SendCtrlAltDel);
                        }
                        let capture = if state.mouse_captured {
                            "Release mouse"
                        } else {
                            "Capture mouse"
                        };
                        if ui
                            .add_enabled(state.running, egui::Button::new(capture))
                            .clicked()
                        {
                            action = Some(VmBarAction::ToggleMouseCapture);
                        }
                        let serial = if state.serial_shown {
                            "Hide serial"
                        } else {
                            "Show serial"
                        };
                        if ui.button(serial).clicked() {
                            action = Some(VmBarAction::ToggleSerial);
                        }
                        ui.separator();
                    }
                    if ui
                        .add_enabled(state.running, egui::Button::new("↻ Restart"))
                        .clicked()
                    {
                        action = Some(VmBarAction::Restart);
                    }
                    if ui
                        .add_enabled(state.running, egui::Button::new("■ Power off"))
                        .clicked()
                    {
                        action = Some(VmBarAction::PowerOff);
                    }
                    // The one filled button in the shell.
                    let can_start = !state.running && !state.start_pending;
                    let power_on = egui::Button::new(
                        RichText::new("▶ Power on").strong().color(BG_BASE),
                    )
                    .fill(ACCENT_CYAN)
                    .stroke(Stroke::NONE);
                    if ui.add_enabled(can_start, power_on).clicked() {
                        action = Some(VmBarAction::PowerOn);
                    }
                });
            });
        });
    hairline_below(ui, bar.response.rect);
    action
}
