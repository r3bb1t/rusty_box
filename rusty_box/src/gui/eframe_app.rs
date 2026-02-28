//! eframe/egui application for the Rusty Box emulator.
//!
//! `RustyBoxApp` implements `eframe::App` and renders the VGA framebuffer
//! from `SharedDisplay` as an egui texture. Keyboard events are converted
//! to PS/2 scancodes and pushed into the shared scancode queue.

use super::keymap::char_to_scancode_sequence;
use super::shared_display::SharedDisplay;

use std::sync::{Arc, Mutex};

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
}

impl RustyBoxApp {
    /// Create a new RustyBoxApp with the given shared display.
    pub fn new(_cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<SharedDisplay>>) -> Self {
        Self {
            shared,
            texture: None,
            last_width: 0,
            last_height: 0,
            cached_ips: 0,
            cached_emu_running: true,
        }
    }

    /// Process keyboard input from egui and convert to PS/2 scancodes.
    fn process_input(&mut self, ctx: &egui::Context) {
        let mut scancodes = Vec::new();

        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Text(text) => {
                        // Printable characters
                        for ch in text.chars() {
                            let seq = char_to_scancode_sequence(ch);
                            scancodes.extend_from_slice(&seq);
                        }
                    }
                    egui::Event::Key { key, pressed, .. } => {
                        // Special keys (not covered by Text events)
                        let seq = egui_key_to_scancodes(*key, *pressed);
                        scancodes.extend_from_slice(&seq);
                    }
                    _ => {}
                }
            }
        });

        if !scancodes.is_empty() {
            if let Ok(mut display) = self.shared.lock() {
                display.pending_scancodes.extend_from_slice(&scancodes);
            }
        }
    }

    /// Update the egui texture from the shared framebuffer.
    fn update_texture(&mut self, ctx: &egui::Context) {
        let Ok(mut display) = self.shared.lock() else {
            return;
        };

        // Always cache status for the status bar
        self.cached_emu_running = display.emu_running;
        self.cached_ips = display.ips;

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
            egui::ColorImage {
                size: [w, h],
                pixels,
            }
        } else {
            // Safety fallback: create correct-sized image
            let mut padded = vec![egui::Color32::BLACK; expected];
            let copy_len = pixels.len().min(expected);
            padded[..copy_len].copy_from_slice(&pixels[..copy_len]);
            egui::ColorImage {
                size: [w, h],
                pixels: padded,
            }
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

        // Update window title
        let title = if self.cached_emu_running {
            "Rusty Box - Running".to_string()
        } else if self.cached_ips > 0 {
            "Rusty Box - Finished".to_string()
        } else {
            "Rusty Box - Stopped".to_string()
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

impl eframe::App for RustyBoxApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply dark theme
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(0x1A, 0x1A, 0x2E);
        visuals.window_fill = egui::Color32::from_rgb(0x1A, 0x1A, 0x2E);
        visuals.extreme_bg_color = egui::Color32::from_rgb(0x0D, 0x0D, 0x1A);
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(0x16, 0x16, 0x2B);
        ctx.set_visuals(visuals);

        self.process_input(ctx);
        self.update_texture(ctx);

        // Status bar at the bottom — modern dark theme
        let bar_bg = egui::Color32::from_rgb(0x12, 0x12, 0x24);
        let text_dim = egui::Color32::from_rgb(0x88, 0x8B, 0x99);
        let accent_green = egui::Color32::from_rgb(0x4E, 0xC9, 0xB0);
        let accent_blue = egui::Color32::from_rgb(0x56, 0x9C, 0xD6);
        let accent_yellow = egui::Color32::from_rgb(0xDC, 0xDC, 0xAA);

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(26.0)
            .frame(
                egui::Frame::NONE
                    .fill(bar_bg)
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ctx, |ui| {
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
                });
            });

        // Main display area — deep dark background
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(0x0D, 0x0D, 0x1A)))
            .show(ctx, |ui| {
                if let Some(ref tex) = self.texture {
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
