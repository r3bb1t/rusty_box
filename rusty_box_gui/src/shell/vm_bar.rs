//! The one bar above a page: what VM this is, what state it is in, and the
//! verbs that change that state.
//!
//! The bar draws from borrowed state and reports the click; the app decides
//! what the click means. The power verbs are always present and enabled by
//! the machine's state alone; the three controls that act on the guest's
//! console (Ctrl+Alt+Del, mouse capture, the serial pane) appear only while
//! the console page is shown — in the bar while it has room for them, inside
//! the `…` overflow when it does not. The `…` at the far right always holds
//! the two verbs that belong to the application rather than to the VM.

#[cfg(target_os = "android")]
use crate::shell::destination::ShellPage;
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::destination::VmBarAction;
#[cfg(target_os = "android")]
use crate::shell::theme::{ACCENT_CYAN, TEXT_MUTED};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::theme::{BG_PANEL, TEXT_BODY, TEXT_PRIMARY};
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use crate::shell::theme::TEXT_CAPTION;
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::widgets::{button_width, hairline_below, primary_button};
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use crate::shell::widgets::{text_width, ShellStateBadge};
#[cfg(not(target_arch = "wasm32"))]
use egui::RichText;

/// What the bar needs to know to draw itself. Borrowed, so the bar cannot
/// change any of it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct VmBarState<'a> {
    pub(crate) name: &'a str,
    /// The machine's state beside its name. A phone's bar carries the page
    /// tabs there instead, and its status strip carries the state.
    #[cfg(not(target_os = "android"))]
    pub(crate) badge: ShellStateBadge,
    /// The page shown, which a phone's bar marks among its page tabs.
    #[cfg(target_os = "android")]
    pub(crate) page: ShellPage,
    pub(crate) running: bool,
    pub(crate) start_pending: bool,
    pub(crate) on_console: bool,
    pub(crate) serial_shown: bool,
    pub(crate) mouse_captured: bool,
}

/// The verbs' captions, named once because the bar measures each caption
/// before it draws the button that carries it. Every glyph among them is one
/// egui's default fonts hold; `shell::tests` checks each against the font.
#[cfg(not(target_arch = "wasm32"))]
const OVERFLOW: &str = "…";
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

/// The bar's height where its controls fit it, points.
#[cfg(not(target_arch = "wasm32"))]
const BAR_HEIGHT: f32 = 36.0;
/// The bar's padding above and below its controls, points.
#[cfg(not(target_arch = "wasm32"))]
const BAR_MARGIN_Y: f32 = 4.0;

/// The width the verbs that are always in the bar take together: the `…`
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

/// A phone's page tabs, in the order a desktop's tree lists the pages: the
/// shown page carries the text weight and an accent rule, the rest sit muted.
/// Reports the page tapped.
#[cfg(target_os = "android")]
fn page_tabs(ui: &mut egui::Ui, shown: ShellPage) -> Option<ShellPage> {
    let mut tapped = None;
    for page in ShellPage::ALL {
        let is_shown = page == shown;
        let text = RichText::new(page.label())
            .size(TEXT_BODY)
            .color(if is_shown { TEXT_PRIMARY } else { TEXT_MUTED });
        let text = if is_shown { text.strong() } else { text };
        let response = ui.add(egui::Button::new(text).frame_when_inactive(false));
        if is_shown {
            ui.painter().hline(
                response.rect.x_range(),
                response.rect.bottom() - 1.0,
                egui::Stroke::new(2.0_f32, ACCENT_CYAN),
            );
        }
        if response.clicked() {
            tapped = Some(page);
        }
    }
    tapped
}

/// The width the page tabs take together, each measured at its shown weight
/// so the row does not shift as the page changes.
#[cfg(target_os = "android")]
fn page_tabs_width(ui: &egui::Ui) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    ShellPage::ALL
        .iter()
        .map(|page| {
            button_width(ui, RichText::new(page.label()).size(TEXT_BODY).strong()) + gap
        })
        .sum()
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
    // Tall enough for the controls it holds: a phone's touch targets are
    // taller than a desktop's, and the bar grows to keep them whole.
    let height = BAR_HEIGHT.max(ui.spacing().interact_size.y + 2.0 * BAR_MARGIN_Y);
    let bar = egui::Panel::top("vm_bar")
        .exact_size(height)
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
                #[cfg(not(target_os = "android"))]
                let badge = RichText::new(state.badge.label)
                    .size(TEXT_CAPTION)
                    .color(state.badge.color);
                // The name is the one elastic element: it is cut to whatever is
                // left once the badge (a phone's page tabs) and the power verbs
                // have their room.
                #[cfg(not(target_os = "android"))]
                let beside_name = text_width(ui, badge.clone(), egui::TextStyle::Body);
                #[cfg(target_os = "android")]
                let beside_name = page_tabs_width(ui);
                let name_width =
                    (ui.available_width() - beside_name - verbs_width - 2.0 * gap).max(0.0);
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
                #[cfg(not(target_os = "android"))]
                ui.label(badge);
                #[cfg(target_os = "android")]
                if let Some(page) = page_tabs(ui, state.page) {
                    action = Some(VmBarAction::GoTo(page));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The console's controls sit in the bar only while the whole
                    // right-hand group fits; otherwise they move into `…`.
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
                        if ui.button("Create floppy image…").clicked() {
                            action = Some(VmBarAction::CreateFloppy);
                            ui.close();
                        }
                        if ui.button("About Rusty Box Workstation").clicked() {
                            action = Some(VmBarAction::ShowAbout);
                            ui.close();
                        }
                        if ui.button("Quit").clicked() {
                            action = Some(VmBarAction::Quit);
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("More");
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
                        primary_button(POWER_ON)
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
