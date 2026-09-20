//! eframe/egui application for the Rusty Box emulator.
//!
//! `RustyBoxApp` implements `eframe::App` and renders the VGA framebuffer
//! from `SharedDisplay` as an egui texture. Keyboard and mouse events are
//! translated by `host_input` into guest key and mouse events, queued on the
//! `SharedDisplay` for the emulator thread to drain.

use super::host_input::HeldModifiers;
use super::touchpad::{TouchSample, TouchZone, Touchpad};
use crate::iodev::scancodes::BxKey;
use super::shared_display::SharedDisplay;

use std::sync::{
    atomic::Ordering,
    {Arc, Mutex},
};

const SERIAL_PANEL_MIN_HEIGHT: f32 = 48.0;
const SERIAL_PANEL_DEFAULT_HEIGHT: f32 = 88.0;
const SERIAL_PANEL_MAX_HEIGHT: f32 = 200.0;

/// The startup notice: a spinner over one row of text.
const STARTUP_SPINNER_SIZE: f32 = 28.0;
const STARTUP_GAP: f32 = 12.0;
const STARTUP_TEXT_SIZE: f32 = 15.0;

/// What the display region shows instead of the framebuffer while the machine
/// is off. The caller that owns the words lays them out — fonts, colours, line
/// breaks — as one `LayoutJob`, and the view places that job as a single label
/// centred in the region. While it is shown, `RustyBoxApp::texture` is not
/// drawn: that texture holds the previous run's last frame.
pub struct ConsolePlaceholder(pub egui::text::LayoutJob);

/// The eframe application that displays the emulator's VGA output.
pub struct RustyBoxApp {
    shared: Arc<Mutex<SharedDisplay>>,
    texture: Option<egui::TextureHandle>,
    // Cache dimensions to detect changes
    last_width: u32,
    last_height: u32,
    // Cached status for status bar (avoids re-locking)
    cached_ips: u32,
    cached_emu_running: bool,
    // Cached serial log for display (updated each frame from shared)
    cached_serial_log: String,
    serial_log_len: usize,
    // Cached transient startup status (e.g. "Creating disk image…"), shown
    // before the guest produces video.
    cached_startup_status: Option<String>,
    serial_input: String,
    /// How the framebuffer fills the region it is drawn in.
    display_scale: DisplayScale,
    /// The shape of one guest pixel: square by default, or 4:3 for the VGA
    /// text and low-resolution modes (720x400, 320x200) a CRT shows as 4:3.
    pixel_aspect: PixelAspect,
    /// Previous PS/2 button bitmask, so a release with no motion is still reported.
    prev_mouse_buttons: u8,
    /// The modifiers held at the end of the previous frame, so each Shift,
    /// Ctrl and Alt edge is forwarded once.
    held_modifiers: HeldModifiers,
    /// What drives the guest's mouse: the host pointer, or touch as a trackpad.
    pointer_mode: PointerMode,
    /// The trackpad's cursor speed over the finger's own travel.
    pointer_speed: f32,
    /// The gestures in progress while touch drives the guest's mouse.
    touchpad: Touchpad,
}

impl RustyBoxApp {
    /// Create a new RustyBoxApp with the given shared display.
    pub fn new(_cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<SharedDisplay>>) -> Self {
        Self::new_embedded(shared)
    }

    /// Create a RustyBoxApp for an already-managed shell surface.
    pub fn new_embedded(shared: Arc<Mutex<SharedDisplay>>) -> Self {
        Self {
            shared,
            texture: None,
            last_width: 0,
            last_height: 0,
            cached_ips: 0,
            cached_emu_running: false,
            cached_serial_log: String::new(),
            serial_log_len: 0,
            cached_startup_status: None,
            serial_input: String::new(),
            display_scale: DisplayScale::Crisp,
            pixel_aspect: PixelAspect::Square,
            prev_mouse_buttons: 0,
            held_modifiers: HeldModifiers::default(),
            pointer_mode: PointerMode::Mouse,
            pointer_speed: 1.0,
            touchpad: Touchpad::new(),
        }
    }

    /// What drives the guest's mouse.
    pub fn set_pointer_mode(&mut self, mode: PointerMode) {
        self.pointer_mode = mode;
    }

    /// The trackpad's cursor speed: at 1.0 the cursor travels as far across
    /// the guest's image as the finger does.
    pub fn set_pointer_speed(&mut self, speed: f32) {
        self.pointer_speed = speed;
    }

    /// How the framebuffer fills the region it is drawn in.
    pub fn set_display_scale(&mut self, scale: DisplayScale) {
        self.display_scale = scale;
    }

    /// Enable 4:3 pixel-aspect correction for non-square VGA modes.
    pub fn set_pixel_aspect_correct(&mut self, pixel_aspect_correct: bool) {
        self.pixel_aspect = if pixel_aspect_correct {
            PixelAspect::Crt4x3
        } else {
            PixelAspect::Square
        };
    }

    fn should_request_repaint(&self) -> bool {
        self.cached_emu_running || self.texture.is_none()
    }

    fn shared_emu_running(&self) -> bool {
        self.shared
            .lock()
            .map(|display| display.emu_running)
            .unwrap_or(false)
    }

    fn send_serial_input(&mut self) {
        if self.serial_input.is_empty() {
            return;
        }
        if let Ok(mut display) = self.shared.lock() {
            if !display.emu_running {
                return;
            }
            display.queue_serial_input_line(&self.serial_input);
            self.serial_input.clear();
        }
    }

    /// Forward this frame's keyboard to the guest through the shared queue.
    ///
    /// The translation is `host_input::translate_egui_keyboard`'s — the one
    /// the browser shell also uses, with the machine itself as its sink; here
    /// the sink is the shared display, which the emulator thread drains. The
    /// events are consumed there, so egui does not also spend them on widget
    /// navigation (Tab then Enter would otherwise press "Restart VM").
    fn process_input(&mut self, ctx: &egui::Context) {
        let Ok(mut display) = self.shared.lock() else {
            return;
        };
        let held = self.held_modifiers;
        self.held_modifiers = ctx.input_mut(|input| {
            super::host_input::translate_egui_keyboard(input, held, &mut *display)
        });
    }

    /// Queue the PS/2 Set-2 sequence for Ctrl+Alt+Del. Needed because the host OS
    /// usually intercepts the real chord before egui sees it.
    pub fn send_ctrl_alt_del(&mut self) {
        const CTRL_ALT_DEL: [(BxKey, bool); 6] = [
            (BxKey::CtrlL, true),
            (BxKey::AltL, true),
            (BxKey::Delete, true),
            (BxKey::Delete, false),
            (BxKey::AltL, false),
            (BxKey::CtrlL, false),
        ];
        if let Ok(mut display) = self.shared.lock() {
            display.pending_keys.extend_from_slice(&CTRL_ALT_DEL);
        }
    }

    /// Toggle whether the display captures mouse/keyboard input for the guest.
    pub fn toggle_mouse_capture(&mut self) {
        if let Ok(mut display) = self.shared.lock() {
            display.mouse_captured = !display.mouse_captured;
        }
    }

    /// Whether the guest currently captures the mouse.
    pub fn mouse_captured(&self) -> bool {
        self.shared
            .lock()
            .map(|display| display.mouse_captured)
            .unwrap_or(false)
    }

    /// Route pointer activity over the display rectangle to the guest.
    ///
    /// While the guest is not captured, a click on the display grabs it. While
    /// captured, relative motion / buttons / wheel are forwarded to the PS/2 aux
    /// device (via the shared queue) and the host cursor is hidden; the user
    /// releases capture from the toolbar toggle.
    fn handle_display_pointer(&mut self, ui: &egui::Ui, image_rect: egui::Rect) {
        if !self.shared_emu_running() {
            return;
        }
        if !ui.rect_contains_pointer(image_rect) {
            return;
        }

        if !self.mouse_captured() {
            // Click-to-capture: consume the click that grabs the guest.
            let clicked = ui
                .ctx()
                .input(|input| input.pointer.button_clicked(egui::PointerButton::Primary));
            if clicked {
                self.toggle_mouse_capture();
            }
            return;
        }

        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        let prev = self.prev_mouse_buttons;
        let new_buttons = if let Ok(mut display) = self.shared.lock() {
            ui.ctx()
                .input(|input| super::host_input::translate_egui_mouse(input, prev, &mut *display))
        } else {
            prev
        };
        self.prev_mouse_buttons = new_buttons;
    }

    /// Touch drives the guest's mouse as a trackpad (see [`Touchpad`]), with
    /// the left and right buttons drawn at the bottom-right of `region`.
    ///
    /// A touch belongs to the trackpad when it starts inside `image_rect`
    /// with no window or area above it, so a menu drawn over the image keeps
    /// its own touches.
    fn handle_touchpad(&mut self, ui: &egui::Ui, image_rect: egui::Rect, region: egui::Rect) {
        let ctx = ui.ctx().clone();
        let buttons = TouchButtons::in_region(region);
        buttons.paint(&ctx, self.touchpad.held_buttons());
        if !self.shared_emu_running() {
            return;
        }

        let guest_pixels_per_point = self.last_width.max(1) as f32 / image_rect.width().max(1.0);
        self.touchpad
            .set_scale(guest_pixels_per_point * self.pointer_speed);
        let (time, touches) = ctx.input(|input| {
            let touches: Vec<(u64, egui::TouchPhase, egui::Pos2)> = input
                .raw
                .events
                .iter()
                .filter_map(|event| {
                    if let egui::Event::Touch { id, phase, pos, .. } = event {
                        Some((id.0, *phase, *pos))
                    } else {
                        None
                    }
                })
                .collect();
            (input.time, touches)
        });
        let Ok(mut display) = self.shared.lock() else {
            return;
        };
        for (id, phase, pos) in touches {
            let zone = if buttons.left.contains(pos) {
                TouchZone::LeftButton
            } else if buttons.right.contains(pos) {
                TouchZone::RightButton
            } else if image_rect.contains(pos) && ctx.layer_id_at(pos).is_none() {
                TouchZone::Pad
            } else {
                TouchZone::Elsewhere
            };
            self.touchpad.touch(
                TouchSample {
                    id,
                    phase,
                    pos,
                    time,
                    zone,
                },
                &mut *display,
            );
        }
    }

    /// Update the egui texture from the shared framebuffer.
    fn update_texture(&mut self, ctx: &egui::Context, update_title: bool) {
        let Some((w, h, framebuffer)) = ({
            let Ok(mut display) = self.shared.lock() else {
                return;
            };

            // Always cache status for the status bar.
            self.cached_emu_running = display.emu_running;
            self.cached_ips = display.ips;
            // Once the guest produces real video, the startup notice has served
            // its purpose — clear it so it doesn't linger over the running guest.
            if display.fb_dirty {
                display.startup_status = None;
            }
            self.cached_startup_status.clone_from(&display.startup_status);

            // Sync serial log if it changed.
            if display.serial_log.len() != self.serial_log_len {
                self.cached_serial_log.clone_from(&display.serial_log);
                self.serial_log_len = display.serial_log.len();
            }

            if !display.fb_dirty && self.texture.is_some() {
                return;
            }

            let w = display.fb_width as usize;
            let h = display.fb_height as usize;

            if w == 0 || h == 0 {
                return;
            }

            // Copy bytes while holding the shared lock, then do the expensive
            // RGBA→Color32 conversion and texture upload after unlocking. This
            // keeps the emulator thread from blocking on egui's per-pixel work.
            let framebuffer = display.framebuffer.clone();
            display.fb_dirty = false;
            Some((w, h, framebuffer))
        }) else {
            return;
        };

        // Convert RGBA bytes to egui ColorImage outside the shared-display lock.
        let pixels: Vec<egui::Color32> = framebuffer
            .chunks_exact(4)
            .map(|rgba| egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]))
            .collect();

        // Pad or truncate if framebuffer size doesn't match exactly.
        let expected = w * h;
        let image = if pixels.len() == expected {
            egui::ColorImage::new([w, h], pixels)
        } else {
            // Safety fallback: create correct-sized image.
            let mut padded = vec![egui::Color32::BLACK; expected];
            let copy_len = pixels.len().min(expected);
            padded[..copy_len].copy_from_slice(&pixels[..copy_len]);
            egui::ColorImage::new([w, h], padded)
        };

        // Crisp square pixels magnify NEAREST (whole multiples stay sharp) and
        // minify LINEAR; every other scale draws at a fractional size, which
        // only LINEAR renders evenly.
        let options = match (self.display_scale, self.pixel_aspect) {
            (DisplayScale::Crisp, PixelAspect::Square) => egui::TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Linear,
                ..Default::default()
            },
            (DisplayScale::Crisp, PixelAspect::Crt4x3)
            | (DisplayScale::Fit | DisplayScale::Stretch, PixelAspect::Square | PixelAspect::Crt4x3) => {
                egui::TextureOptions::LINEAR
            }
        };

        match &mut self.texture {
            Some(tex) if self.last_width == w as u32 && self.last_height == h as u32 => {
                // Update existing texture (fast path)
                tex.set(image, options);
            }
            _ => {
                // Create new texture (size changed or first time)
                self.texture = Some(ctx.load_texture("vga_display", image, options));
                self.last_width = w as u32;
                self.last_height = h as u32;
            }
        }

        if update_title {
            let title = if self.cached_emu_running {
                "Rusty Box - Running"
            } else if self.cached_ips > 0 {
                "Rusty Box - Finished"
            } else {
                "Rusty Box - Stopped"
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.to_owned()));
        }
    }
}
impl RustyBoxApp {
    /// Render the emulator UI inside a parent egui shell without overriding the shell theme.
    pub fn ui_embedded(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ui_embedded_with_serial(ui, frame, true, None);
    }

    /// Render the emulator UI inside a parent egui shell with serial visibility
    /// control. `placeholder` is what the display region shows while the machine
    /// is off; `None` leaves the last framebuffer on screen.
    pub fn ui_embedded_with_serial(
        &mut self,
        ui: &mut egui::Ui,
        frame: &mut eframe::Frame,
        show_serial: bool,
        placeholder: Option<ConsolePlaceholder>,
    ) {
        self.ui_inner(ui, frame, false, show_serial, false, placeholder);
    }

    fn ui_inner(
        &mut self,
        ui: &mut egui::Ui,
        _frame: &mut eframe::Frame,
        apply_theme: bool,
        show_serial: bool,
        show_status_bar: bool,
        placeholder: Option<ConsolePlaceholder>,
    ) {
        let ctx = ui.ctx().clone();
        if apply_theme {
            // Apply dark theme
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = egui::Color32::from_rgb(0x1A, 0x1A, 0x2E);
            visuals.window_fill = egui::Color32::from_rgb(0x1A, 0x1A, 0x2E);
            visuals.extreme_bg_color = egui::Color32::from_rgb(0x0D, 0x0D, 0x1A);
            visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(0x16, 0x16, 0x2B);
            ctx.set_visuals(visuals);
        }

        // Forward the keyboard while the VM runs. When the mouse is captured we
        // bypass egui's focus gate so chords (Ctrl+C, Alt+Tab in the guest) reach
        // the VM even if a host widget holds focus.
        if self.shared_emu_running()
            && (self.mouse_captured() || !ctx.egui_wants_keyboard_input())
        {
            self.process_input(&ctx);
        }
        self.update_texture(&ctx, apply_theme);
        let text_dim = egui::Color32::from_rgb(0x88, 0x8B, 0x99);

        if show_status_bar {
            // Status bar at the bottom — modern dark theme
            let bar_bg = egui::Color32::from_rgb(0x12, 0x12, 0x24);
            let accent_green = egui::Color32::from_rgb(0x4E, 0xC9, 0xB0);
            let accent_blue = egui::Color32::from_rgb(0x56, 0x9C, 0xD6);
            let accent_yellow = egui::Color32::from_rgb(0xDC, 0xDC, 0xAA);

            egui::Panel::bottom("status_bar")
                .exact_size(26.0)
                .frame(
                    egui::Frame::NONE
                        .fill(bar_bg)
                        .inner_margin(egui::Margin::symmetric(12, 4)),
                )
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 20.0;

                        // IPS counter
                        let ips_text = if self.cached_ips > 0 {
                            let ips = self.cached_ips as f64;
                            if ips >= 1_000_000.0 {
                                format!("{:.3}M IPS", ips / 1_000_000.0)
                            } else if ips >= 1_000.0 {
                                format!("{:.0}K IPS", ips / 1_000.0)
                            } else {
                                format!("{:.0} IPS", ips)
                            }
                        } else {
                            "--- IPS".to_string()
                        };
                        ui.label(
                            egui::RichText::new(ips_text)
                                .monospace()
                                .size(11.0)
                                .color(accent_blue),
                        );

                        // Subtle separator
                        ui.label(
                            egui::RichText::new("|")
                                .monospace()
                                .size(11.0)
                                .color(egui::Color32::from_rgb(0x3A, 0x3A, 0x50)),
                        );

                        // Emulator status with color coding
                        let (status_text, status_color) = if self.cached_emu_running {
                            ("Running", accent_green)
                        } else if self.cached_ips > 0 {
                            ("Finished", accent_yellow)
                        } else {
                            ("Stopped", text_dim)
                        };
                        ui.label(
                            egui::RichText::new(status_text)
                                .monospace()
                                .size(11.0)
                                .color(status_color),
                        );

                        // Restart button — right-aligned
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let btn = egui::Button::new(
                                egui::RichText::new("Restart VM")
                                    .monospace()
                                    .size(11.0)
                                    .color(egui::Color32::from_rgb(0xCC, 0x88, 0x44)),
                            )
                            .fill(egui::Color32::from_rgb(0x1E, 0x1E, 0x30))
                            .stroke(egui::Stroke::new(
                                1.0_f32,
                                egui::Color32::from_rgb(0x44, 0x44, 0x66),
                            ));
                            // Use click-only sense to exclude from Tab focus chain.
                            // Without this, Tab+Enter accidentally triggers Restart VM.
                            let btn = btn.sense(egui::Sense::click());
                            if ui.add_enabled(self.cached_emu_running, btn).clicked() {
                                if let Ok(mut d) = self.shared.lock() {
                                    d.stop_flag.store(true, Ordering::Relaxed);
                                    d.reset_requested = true;
                                }
                            }
                        });
                    });
                });
        }

        // Serial console panel — shown when enabled so input can be sent before output appears.
        if show_serial {
            let console_bg = egui::Color32::from_rgb(0x0A, 0x0A, 0x14);
            let console_text = egui::Color32::from_rgb(0x00, 0xCC, 0x66);
            egui::Panel::bottom("serial_console")
                .resizable(true)
                .min_size(SERIAL_PANEL_MIN_HEIGHT)
                .default_size(SERIAL_PANEL_DEFAULT_HEIGHT)
                .max_size(SERIAL_PANEL_MAX_HEIGHT)
                .frame(
                    egui::Frame::NONE
                        .fill(console_bg)
                        .inner_margin(egui::Margin::same(6)),
                )
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Serial Console (ttyS0)")
                            .monospace()
                            .size(10.0)
                            .color(egui::Color32::from_rgb(0x66, 0x66, 0x88)),
                    );
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&self.cached_serial_log)
                                    .monospace()
                                    .size(11.0)
                                    .color(console_text),
                            );
                        });
                    ui.separator();
                    ui.horizontal(|ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.serial_input)
                                .desired_width(260.0)
                                .hint_text("serial input"),
                        );
                        let enter_pressed = response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        let send_enabled = self.cached_emu_running && !self.serial_input.is_empty();
                        if ui
                            .add_enabled(send_enabled, egui::Button::new("Send"))
                            .clicked()
                            || (send_enabled && enter_pressed)
                        {
                            self.send_serial_input();
                        }
                        if ui.button("Copy Log").clicked() {
                            ui.ctx().copy_text(self.cached_serial_log.clone());
                        }
                        ui.add_enabled(false, egui::Button::new("Paste"));
                    });
                });
        }

        // The placeholder stands in for the framebuffer only while the machine
        // is off: a running machine's video wins over whatever the caller passed.
        let powered_off = placeholder.filter(|_| !self.cached_emu_running);

        // Main display area — deep dark background
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(0x0D, 0x0D, 0x1A)))
            .show(ui, |ui| {
                let region = ui.max_rect();
                let mut image_rect = None;
                if let Some(status) = self.cached_startup_status.clone() {
                    // A startup step (e.g. allocating the disk image) is running.
                    // Show it with a spinner instead of a blank panel, so the
                    // window doesn't look frozen while the guest has no video yet.
                    // A top-down layout stacks from the region's top edge, so the
                    // stack is centred by the pad above it: half of what the
                    // region has left after the spinner, the item spacing the
                    // layout inserts below it, the gap, and one row of the text.
                    let text_height = ui.fonts_mut(|fonts| {
                        fonts.row_height(&egui::FontId::proportional(STARTUP_TEXT_SIZE))
                    });
                    let stack_height = STARTUP_SPINNER_SIZE
                        + ui.spacing().item_spacing.y
                        + STARTUP_GAP
                        + text_height;
                    let pad = ((ui.available_height() - stack_height) / 2.0).max(0.0);
                    ui.vertical_centered(|ui| {
                        ui.add_space(pad);
                        ui.add(egui::Spinner::new().size(STARTUP_SPINNER_SIZE));
                        ui.add_space(STARTUP_GAP);
                        ui.label(
                            egui::RichText::new(status)
                                .color(egui::Color32::from_rgb(0xE8, 0xEE, 0xF5))
                                .size(STARTUP_TEXT_SIZE),
                        );
                    });
                } else if let Some(ConsolePlaceholder(job)) = powered_off {
                    // `centered_and_justified` centres exactly one widget, so the
                    // whole block is one label over the caller's layout job.
                    ui.centered_and_justified(|ui| {
                        ui.label(job);
                    });
                } else if let Some(tex) = &self.texture {
                    let available = ui.available_size();
                    let drawn = display_size(
                        available,
                        [self.last_width, self.last_height],
                        ui.ctx().pixels_per_point(),
                        self.display_scale,
                        self.pixel_aspect,
                    );
                    let (draw_w, draw_h) = (drawn.x, drawn.y);

                    // Center the image.
                    let offset_x = ((available.x - draw_w) / 2.0).max(0.0);
                    let offset_y = ((available.y - draw_h) / 2.0).max(0.0);
                    ui.add_space(offset_y);
                    ui.horizontal(|ui| {
                        ui.add_space(offset_x);
                        let response = ui.image(egui::load::SizedTexture::new(
                            tex.id(),
                            egui::vec2(draw_w, draw_h),
                        ));
                        image_rect = Some(response.rect);
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("Waiting for VGA output...")
                                .color(text_dim)
                                .size(14.0),
                        );
                    });
                }
                if let Some(rect) = image_rect {
                    match self.pointer_mode {
                        PointerMode::Mouse => self.handle_display_pointer(ui, rect),
                        PointerMode::Touchpad => self.handle_touchpad(ui, rect, region),
                    }
                }
            });

        // Request continuous repaint while the emulator is running, and while waiting
        // for the first texture so a fast VM stop cannot strand the surface on the placeholder.
        if self.should_request_repaint() {
            ctx.request_repaint();
        }
    }
}

/// What drives the guest's mouse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerMode {
    /// The host's pointer, captured by a click on the image.
    #[default]
    Mouse,
    /// Touch as a trackpad, with on-screen left and right buttons: the form a
    /// phone needs, where a finger is not a mouse.
    Touchpad,
}

/// The side of an on-screen mouse button, points: a comfortable thumb target.
const TOUCH_BUTTON_SIZE: f32 = 44.0;
/// The room between the buttons and the region's corner, points.
const TOUCH_BUTTON_MARGIN: f32 = 12.0;
/// The room between the two buttons, points.
const TOUCH_BUTTON_GAP: f32 = 8.0;

/// Where the on-screen left and right buttons sit.
#[derive(Clone, Copy, Debug)]
struct TouchButtons {
    left: egui::Rect,
    right: egui::Rect,
}

impl TouchButtons {
    /// Side by side at the bottom-right corner of `region`.
    fn in_region(region: egui::Rect) -> Self {
        let size = egui::vec2(TOUCH_BUTTON_SIZE, TOUCH_BUTTON_SIZE);
        let right = egui::Rect::from_min_size(
            region.right_bottom() - size - egui::vec2(TOUCH_BUTTON_MARGIN, TOUCH_BUTTON_MARGIN),
            size,
        );
        let left = right.translate(egui::vec2(-(TOUCH_BUTTON_SIZE + TOUCH_BUTTON_GAP), 0.0));
        Self { left, right }
    }

    /// See-through squares over the guest, lit while held. `held` is the
    /// PS/2 button mask the trackpad holds.
    fn paint(&self, ctx: &egui::Context, held: u8) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("touch_mouse_buttons"),
        ));
        for (rect, label, bit) in [(self.left, "L", 0x01), (self.right, "R", 0x02)] {
            let fill = if held & bit != 0 {
                egui::Color32::from_rgba_unmultiplied(0x46, 0xD9, 0xC7, 150)
            } else {
                egui::Color32::from_black_alpha(110)
            };
            painter.rect_filled(rect, 8.0, fill);
            painter.rect_stroke(
                rect,
                8.0,
                egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(90)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(18.0),
                egui::Color32::from_white_alpha(220),
            );
        }
    }
}

/// How the console's framebuffer fills the region it is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayScale {
    /// Whole physical-pixel multiples for crisp pixels; a fractional shrink
    /// only when the guest is larger than the region.
    #[default]
    Crisp,
    /// The largest size that keeps the guest's shape.
    Fit,
    /// Both axes of the region, whatever the guest's shape.
    Stretch,
}

/// The shape of one guest pixel on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PixelAspect {
    /// Square pixels: the framebuffer's own shape.
    Square,
    /// A 4:3 picture whatever the mode, as a CRT shows 720x400 and 320x200.
    Crt4x3,
}

/// The size, in points, the framebuffer is drawn at inside `available`.
///
/// Scaling is worked out in physical pixels: egui multiplies points by
/// `pixels_per_point`, so a whole multiple in points is a fractional one on a
/// 1.25x or 1.5x display, and NEAREST magnification would draw uneven pixels.
fn display_size(
    available: egui::Vec2,
    texture_px: [u32; 2],
    pixels_per_point: f32,
    scale: DisplayScale,
    aspect: PixelAspect,
) -> egui::Vec2 {
    let four_by_three = || {
        let height = available.y.min(available.x * 3.0 / 4.0);
        egui::vec2(height * 4.0 / 3.0, height)
    };
    let ppp = pixels_per_point.max(f32::EPSILON);
    let tex_w = texture_px[0].max(1) as f32;
    let tex_h = texture_px[1].max(1) as f32;
    let fit = ((available.x * ppp).max(1.0) / tex_w).min((available.y * ppp).max(1.0) / tex_h);
    let scaled = |factor: f32| egui::vec2(tex_w * factor / ppp, tex_h * factor / ppp);
    match (scale, aspect) {
        (DisplayScale::Stretch, PixelAspect::Square | PixelAspect::Crt4x3) => available,
        (DisplayScale::Fit | DisplayScale::Crisp, PixelAspect::Crt4x3) => four_by_three(),
        (DisplayScale::Fit, PixelAspect::Square) => scaled(fit.max(f32::EPSILON)),
        (DisplayScale::Crisp, PixelAspect::Square) if fit < 1.0 => scaled(fit.max(f32::EPSILON)),
        (DisplayScale::Crisp, PixelAspect::Square) => scaled(fit.floor().max(1.0)),
    }
}

impl eframe::App for RustyBoxApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ui_inner(ui, frame, true, true, true, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_input_is_not_queued_while_stopped() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        let mut app = RustyBoxApp::new_embedded(Arc::clone(&shared));
        app.serial_input = "help".to_owned();

        app.send_serial_input();

        assert_eq!(
            shared.lock().unwrap().drain_serial_input(),
            Vec::<u8>::new()
        );
        assert_eq!(app.serial_input, "help");
    }

    #[test]
    fn serial_input_is_queued_while_running() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        shared.lock().unwrap().emu_running = true;
        let mut app = RustyBoxApp::new_embedded(Arc::clone(&shared));
        app.serial_input = "help".to_owned();

        app.send_serial_input();

        assert_eq!(
            shared.lock().unwrap().drain_serial_input(),
            b"help\n".to_vec()
        );
        assert!(app.serial_input.is_empty());
    }

    #[test]
    fn serial_panel_uses_compact_default_size() {
        assert!(SERIAL_PANEL_DEFAULT_HEIGHT <= 96.0);
        assert!(SERIAL_PANEL_MAX_HEIGHT <= 220.0);
    }

    #[test]
    fn stopped_app_repaints_until_first_texture_exists() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        let app = RustyBoxApp::new_embedded(shared);

        assert!(app.should_request_repaint());
    }

    /// `display_size` for square pixels, as a `(width, height)` pair.
    fn size(available: (f32, f32), texture: [u32; 2], ppp: f32, scale: DisplayScale) -> (f32, f32) {
        let drawn = display_size(
            egui::vec2(available.0, available.1),
            texture,
            ppp,
            scale,
            PixelAspect::Square,
        );
        (drawn.x, drawn.y)
    }

    #[test]
    fn stretch_fills_the_whole_region_whatever_the_guest_shape() {
        assert_eq!(
            size((1000.0, 428.0), [640, 480], 2.0, DisplayScale::Stretch),
            (1000.0, 428.0)
        );
    }

    #[test]
    fn fit_keeps_the_guest_shape_at_the_limiting_axis() {
        let (w, h) = size((1000.0, 428.0), [640, 480], 1.0, DisplayScale::Fit);
        assert_eq!(h, 428.0);
        assert!((w - 428.0 * 640.0 / 480.0).abs() < 0.01, "width {w}");
    }

    #[test]
    fn crisp_upscales_by_a_whole_physical_multiple() {
        assert_eq!(
            size((1000.0, 428.0), [320, 200], 1.0, DisplayScale::Crisp),
            (640.0, 400.0)
        );
    }

    #[test]
    fn crisp_shrinks_a_guest_larger_than_the_region() {
        let (w, h) = size((1000.0, 428.0), [2000, 1000], 1.0, DisplayScale::Crisp);
        assert!((w - 856.0).abs() < 0.01 && (h - 428.0).abs() < 0.01, "{w}x{h}");
    }

    #[test]
    fn crt_aspect_draws_a_4x3_box_under_fit() {
        let drawn = display_size(
            egui::vec2(1000.0, 428.0),
            [720, 400],
            1.0,
            DisplayScale::Fit,
            PixelAspect::Crt4x3,
        );
        assert_eq!(drawn.y, 428.0);
        assert!((drawn.x - 428.0 * 4.0 / 3.0).abs() < 0.01, "width {}", drawn.x);
    }
}
