//! eframe/egui application for the Rusty Box emulator.
//!
//! `RustyBoxApp` implements `eframe::App` and renders the VGA framebuffer
//! from `SharedDisplay` as an egui texture. Keyboard events are converted
//! to PS/2 scancodes and pushed into the shared scancode queue.

use super::keymap::char_to_scancode_sequence;
use super::shared_display::SharedDisplay;

use std::sync::{
    atomic::Ordering,
    {Arc, Mutex},
};

const SERIAL_PANEL_MIN_HEIGHT: f32 = 48.0;
const SERIAL_PANEL_DEFAULT_HEIGHT: f32 = 88.0;
const SERIAL_PANEL_MAX_HEIGHT: f32 = 200.0;

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
    serial_input: String,
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
            serial_input: String::new(),
        }
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

    /// Process keyboard input from egui and convert to PS/2 scancodes.
    ///
    /// Handles three event types:
    /// - `Event::Text` — printable characters from the platform text input system
    /// - `Event::Ime(ImeEvent::Commit)` — characters from IME (Windows may use this path)
    /// - `Event::Key` — special keys (arrows, F-keys, Enter, etc.)
    ///
    /// Letter/number keys are handled via Text/Ime events (which include proper
    /// OS-level shift/layout handling). The Key handler covers non-printable keys
    /// and also serves as a fallback for letters when Text events aren't produced.
    fn process_input(&mut self, ctx: &egui::Context) {
        let mut scancodes = Vec::new();

        ctx.input_mut(|i| {
            // Pass 1: check if any Text or Ime::Commit events exist in this frame.
            // If so, we rely on them for printable characters and skip the Key fallback
            // (avoids double-sending since Key events fire BEFORE Text events).
            let has_text_events = i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Text(_) | egui::Event::Ime(egui::ImeEvent::Commit(_))
                )
            });

            // Pass 2: process events and CONSUME them so egui doesn't use them
            // for widget navigation (Tab = focus change, Enter = button click).
            // Without this, Tab+Enter accidentally triggers the Reset button.
            i.events.retain(|event| {
                match event {
                    egui::Event::Text(text) => {
                        for ch in text.chars() {
                            let seq = char_to_scancode_sequence(ch);
                            scancodes.extend_from_slice(&seq);
                        }
                        false
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                        for ch in text.chars() {
                            let seq = char_to_scancode_sequence(ch);
                            scancodes.extend_from_slice(&seq);
                        }
                        false
                    }
                    egui::Event::Key { key, pressed, .. } => {
                        let seq = egui_key_to_scancodes(*key, *pressed);
                        if !seq.is_empty() {
                            scancodes.extend_from_slice(&seq);
                        } else if *pressed && !has_text_events {
                            if let Some(ch) = egui_key_to_char(*key) {
                                let seq = char_to_scancode_sequence(ch);
                                scancodes.extend_from_slice(&seq);
                            }
                        }
                        false
                    }
                    _ => true, // keep non-keyboard events for egui
                }
            });
        });

        if !scancodes.is_empty() {
            if let Ok(mut display) = self.shared.lock() {
                display.pending_scancodes.extend_from_slice(&scancodes);
            }
        }
    }

    /// Update the egui texture from the shared framebuffer.
    fn update_texture(&mut self, ctx: &egui::Context, update_title: bool) {
        let Ok(mut display) = self.shared.lock() else {
            return;
        };

        // Always cache status for the status bar
        self.cached_emu_running = display.emu_running;
        self.cached_ips = display.ips;

        // Sync serial log if it changed
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

        // Convert RGBA bytes to egui ColorImage
        let pixels: Vec<egui::Color32> = display
            .framebuffer
            .chunks_exact(4)
            .map(|rgba| egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]))
            .collect();

        // Pad or truncate if framebuffer size doesn't match exactly
        let expected = w * h;
        let image = if pixels.len() == expected {
            egui::ColorImage::new([w, h], pixels)
        } else {
            // Safety fallback: create correct-sized image
            let mut padded = vec![egui::Color32::BLACK; expected];
            let copy_len = pixels.len().min(expected);
            padded[..copy_len].copy_from_slice(&pixels[..copy_len]);
            egui::ColorImage::new([w, h], padded)
        };

        display.fb_dirty = false;
        drop(display);

        let options = egui::TextureOptions::NEAREST; // Pixel-perfect rendering

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
        self.ui_embedded_with_serial(ui, frame, true);
    }

    /// Render the emulator UI inside a parent egui shell with serial visibility control.
    pub fn ui_embedded_with_serial(
        &mut self,
        ui: &mut egui::Ui,
        frame: &mut eframe::Frame,
        show_serial: bool,
    ) {
        self.ui_inner(ui, frame, false, show_serial, false);
    }

    fn ui_inner(
        &mut self,
        ui: &mut egui::Ui,
        _frame: &mut eframe::Frame,
        apply_theme: bool,
        show_serial: bool,
        show_status_bar: bool,
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

        if self.shared_emu_running() && !ctx.egui_wants_keyboard_input() {
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
                .show_inside(ui, |ui| {
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
                .show_inside(ui, |ui| {
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

        // Main display area — deep dark background
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(0x0D, 0x0D, 0x1A)))
            .show_inside(ui, |ui| {
                if let Some(tex) = &self.texture {
                    let available = ui.available_size();
                    let tex_w = self.last_width as f32;
                    let tex_h = self.last_height.max(1) as f32;

                    // Integer scaling for crisp pixels
                    let max_scale_x = (available.x / tex_w).floor().max(1.0);
                    let max_scale_y = (available.y / tex_h).floor().max(1.0);
                    let scale = max_scale_x.min(max_scale_y);
                    let (w, h) = (tex_w * scale, tex_h * scale);

                    // Center the image
                    let offset_x = (available.x - w) / 2.0;
                    let offset_y = (available.y - h) / 2.0;
                    ui.add_space(offset_y);
                    ui.horizontal(|ui| {
                        ui.add_space(offset_x);
                        ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(w, h)));
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
            });

        // Request continuous repaint while emulator is running
        if self.cached_emu_running {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for RustyBoxApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ui_inner(ui, frame, true, true, true);
    }
}

/// Convert an egui Key to PS/2 scancode set 2 sequence.
///
/// Returns make codes for pressed=true, break codes (0xF0 + make) for pressed=false.
/// Extended keys use 0xE0 prefix.
fn egui_key_to_scancodes(key: egui::Key, pressed: bool) -> Vec<u8> {
    // Map egui keys to PS/2 scancode set 2
    let (extended, make_code) = match key {
        egui::Key::Escape => (false, 0x76u8),
        egui::Key::F1 => (false, 0x05),
        egui::Key::F2 => (false, 0x06),
        egui::Key::F3 => (false, 0x04),
        egui::Key::F4 => (false, 0x0C),
        egui::Key::F5 => (false, 0x03),
        egui::Key::F6 => (false, 0x0B),
        egui::Key::F7 => (false, 0x83),
        egui::Key::F8 => (false, 0x0A),
        egui::Key::F9 => (false, 0x01),
        egui::Key::F10 => (false, 0x09),
        egui::Key::F11 => (false, 0x78),
        egui::Key::F12 => (false, 0x07),

        egui::Key::Enter => (false, 0x5A),
        egui::Key::Tab => (false, 0x0D),
        egui::Key::Backspace => (false, 0x66),
        egui::Key::Space => (false, 0x29),
        egui::Key::Delete => (true, 0x71),
        egui::Key::Insert => (true, 0x70),
        egui::Key::Home => (true, 0x6C),
        egui::Key::End => (true, 0x69),
        egui::Key::PageUp => (true, 0x7D),
        egui::Key::PageDown => (true, 0x7A),

        egui::Key::ArrowUp => (true, 0x75),
        egui::Key::ArrowDown => (true, 0x72),
        egui::Key::ArrowLeft => (true, 0x6B),
        egui::Key::ArrowRight => (true, 0x74),

        // These keys are already handled by Text events for printable chars,
        // so only handle them as special keys for key-down/up tracking.
        // Don't generate scancodes here to avoid double-sending.
        _ => return Vec::new(),
    };

    let mut seq = Vec::with_capacity(4);
    if pressed {
        if extended {
            seq.push(0xE0);
        }
        seq.push(make_code);
    } else {
        if extended {
            seq.push(0xE0);
        }
        seq.push(0xF0);
        seq.push(make_code);
    }
    seq
}

/// Fallback: convert an egui Key to a lowercase ASCII character.
///
/// Used when `Event::Text` / `Event::Ime` don't fire (e.g., certain IME states,
/// accessibility tools, or platform edge cases). Returns lowercase because
/// `char_to_scancode_sequence` handles shift detection from the character itself.
fn egui_key_to_char(key: egui::Key) -> Option<char> {
    match key {
        egui::Key::A => Some('a'),
        egui::Key::B => Some('b'),
        egui::Key::C => Some('c'),
        egui::Key::D => Some('d'),
        egui::Key::E => Some('e'),
        egui::Key::F => Some('f'),
        egui::Key::G => Some('g'),
        egui::Key::H => Some('h'),
        egui::Key::I => Some('i'),
        egui::Key::J => Some('j'),
        egui::Key::K => Some('k'),
        egui::Key::L => Some('l'),
        egui::Key::M => Some('m'),
        egui::Key::N => Some('n'),
        egui::Key::O => Some('o'),
        egui::Key::P => Some('p'),
        egui::Key::Q => Some('q'),
        egui::Key::R => Some('r'),
        egui::Key::S => Some('s'),
        egui::Key::T => Some('t'),
        egui::Key::U => Some('u'),
        egui::Key::V => Some('v'),
        egui::Key::W => Some('w'),
        egui::Key::X => Some('x'),
        egui::Key::Y => Some('y'),
        egui::Key::Z => Some('z'),
        egui::Key::Num0 => Some('0'),
        egui::Key::Num1 => Some('1'),
        egui::Key::Num2 => Some('2'),
        egui::Key::Num3 => Some('3'),
        egui::Key::Num4 => Some('4'),
        egui::Key::Num5 => Some('5'),
        egui::Key::Num6 => Some('6'),
        egui::Key::Num7 => Some('7'),
        egui::Key::Num8 => Some('8'),
        egui::Key::Num9 => Some('9'),
        egui::Key::Minus => Some('-'),
        egui::Key::Equals => Some('='),
        egui::Key::OpenBracket => Some('['),
        egui::Key::CloseBracket => Some(']'),
        egui::Key::Backslash => Some('\\'),
        egui::Key::Semicolon => Some(';'),
        egui::Key::Quote => Some('\''),
        egui::Key::Backtick => Some('`'),
        egui::Key::Comma => Some(','),
        egui::Key::Period => Some('.'),
        egui::Key::Slash => Some('/'),
        _ => None,
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
}
