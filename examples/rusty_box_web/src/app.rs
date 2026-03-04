//! WASM-compatible eframe application for Rusty Box.
//!
//! Single-threaded cooperative execution: each frame calls `step_batch()`
//! to advance the emulator, then renders the VGA framebuffer as an egui
//! texture. No threads, no Arc<Mutex<>> — the emulator and display are
//! owned directly by the app.

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::shared_display::SharedDisplay,
};

// Embedded binary assets (compiled into the WASM)
const BIOS_DATA: &[u8] = include_bytes!("../../../cpp_orig/bochs/bios/BIOS-bochs-latest");
const VGA_BIOS_DATA: &[u8] = include_bytes!("../../../cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin");
const DISK_DATA: &[u8] = include_bytes!("../../../dlxlinux/hd10meg.img");

/// DLX Linux disk geometry
const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

/// Instructions per sub-batch (cpu_loop_n_with_io may return early)
const BATCH_SIZE: u64 = 50_000;
/// Total instruction budget per frame (~200K * 60fps = ~12M IPS target)
const FRAME_BUDGET: u64 = 200_000;

// ---- Color palette (rerun.io-inspired dark theme) ----
const BG_DARKEST: egui::Color32 = egui::Color32::from_rgb(0x0B, 0x0B, 0x15);
const BG_DARK: egui::Color32 = egui::Color32::from_rgb(0x12, 0x12, 0x22);
const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(0x1A, 0x1A, 0x2E);
const BG_SURFACE: egui::Color32 = egui::Color32::from_rgb(0x22, 0x22, 0x3A);
const BORDER_SUBTLE: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x2A, 0x44);
const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(0xE0, 0xE0, 0xE8);
const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x88, 0x8B, 0x99);
const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(0x55, 0x58, 0x66);
const ACCENT_BLUE: egui::Color32 = egui::Color32::from_rgb(0x56, 0x9C, 0xD6);
const ACCENT_GREEN: egui::Color32 = egui::Color32::from_rgb(0x4E, 0xC9, 0xB0);
const ACCENT_YELLOW: egui::Color32 = egui::Color32::from_rgb(0xDC, 0xDC, 0xAA);
const ACCENT_CYAN: egui::Color32 = egui::Color32::from_rgb(0x6A, 0xD8, 0xE8);
const ACCENT_RED: egui::Color32 = egui::Color32::from_rgb(0xF4, 0x4D, 0x4D);

/// The eframe application — owns the emulator and display directly.
pub struct WasmEmulatorApp {
    emulator: Option<Box<Emulator<'static, Corei7SkylakeX>>>,
    display: SharedDisplay,
    texture: Option<egui::TextureHandle>,
    initialized: bool,
    init_error: Option<String>,
    shutdown: bool,

    // Metrics
    total_instructions: u64,
    last_ips_time: web_time::Instant,
    last_ips_instructions: u64,
    cached_ips: f64,
    frame_count: u64,
}

impl WasmEmulatorApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            emulator: None,
            display: SharedDisplay::new(),
            texture: None,
            initialized: false,
            init_error: None,
            shutdown: false,
            total_instructions: 0,
            last_ips_time: web_time::Instant::now(),
            last_ips_instructions: 0,
            cached_ips: 0.0,
            frame_count: 0,
        }
    }

    /// One-time emulator initialization with embedded assets.
    fn initialize_emulator(&mut self) {
        let config = EmulatorConfig {
            guest_memory_size: 32 * 1024 * 1024,
            host_memory_size: 32 * 1024 * 1024,
            memory_block_size: 128 * 1024,
            ips: 15_000_000,
            pci_enabled: true,
            ..Default::default()
        };

        let result = (|| -> rusty_box::Result<Box<Emulator<'static, Corei7SkylakeX>>> {
            let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;

            // Initialize memory + PC system
            emu.init_memory_and_pc_system()?;

            // Load BIOS (128 KB)
            let bios_load_addr = !(BIOS_DATA.len() as u64 - 1);
            emu.load_bios(BIOS_DATA, bios_load_addr)?;

            // Load VGA BIOS at 0xC0000 (pad to 512-byte boundary)
            let mut vga_data = VGA_BIOS_DATA.to_vec();
            let remainder = vga_data.len() % 512;
            if remainder != 0 {
                vga_data.resize(vga_data.len() + (512 - remainder), 0);
            }
            emu.load_optional_rom(&vga_data, 0xC0000)?;

            // Initialize CPU + devices
            emu.init_cpu_and_devices()?;

            // Configure CMOS
            emu.configure_memory_in_cmos(640, 31 * 1024);
            emu.configure_disk_geometry_in_cmos(0, DLX_CYLINDERS, DLX_HEADS, DLX_SPT);
            emu.configure_boot_sequence(2, 0, 0);

            // Attach disk from embedded data (rusty_box compiled without std)
            emu.attach_disk_data(0, 0, DISK_DATA.to_vec(), DLX_CYLINDERS, DLX_HEADS, DLX_SPT);

            // Initialize (no GUI needed — we render via update_display)
            emu.init_gui(0, &[])?;
            emu.reset(ResetReason::Hardware)?;
            emu.start();
            emu.force_vga_update();

            Ok(emu)
        })();

        match result {
            Ok(emu) => {
                self.emulator = Some(emu);
                self.initialized = true;
                log::info!("Emulator initialized successfully");
            }
            Err(e) => {
                let msg = format!("{:?}", e);
                log::error!("Emulator init failed: {}", msg);
                self.init_error = Some(msg);
            }
        }
    }

    /// Apply the modern dark theme.
    fn apply_theme(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG_PANEL;
        visuals.window_fill = BG_PANEL;
        visuals.extreme_bg_color = BG_DARKEST;
        visuals.faint_bg_color = BG_SURFACE;

        visuals.widgets.noninteractive.bg_fill = BG_SURFACE;
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(0.5, BORDER_SUBTLE);

        visuals.widgets.inactive.bg_fill = BG_SURFACE;
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(0.5, BORDER_SUBTLE);

        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x2E, 0x2E, 0x4A);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

        visuals.widgets.active.bg_fill = ACCENT_BLUE;
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

        visuals.selection.bg_fill = egui::Color32::from_rgba_premultiplied(0x56, 0x9C, 0xD6, 0x40);
        visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT_BLUE);

        ctx.set_visuals(visuals);
    }

    /// Update IPS counter.
    fn update_ips(&mut self) {
        let now = web_time::Instant::now();
        let elapsed = now.duration_since(self.last_ips_time);
        if elapsed.as_secs_f64() >= 1.0 {
            let delta_instr = self.total_instructions - self.last_ips_instructions;
            self.cached_ips = delta_instr as f64 / elapsed.as_secs_f64();
            self.last_ips_time = now;
            self.last_ips_instructions = self.total_instructions;
        }
    }

    /// Format IPS value for display.
    fn format_ips(ips: f64) -> String {
        if ips >= 1_000_000.0 {
            format!("{:.2}M", ips / 1_000_000.0)
        } else if ips >= 1_000.0 {
            format!("{:.0}K", ips / 1_000.0)
        } else if ips > 0.0 {
            format!("{:.0}", ips)
        } else {
            "---".to_string()
        }
    }

    /// Upload the SharedDisplay framebuffer as an egui texture.
    fn upload_texture(&mut self, ctx: &egui::Context) {
        let w = self.display.fb_width as usize;
        let h = self.display.fb_height as usize;
        if w == 0 || h == 0 {
            return;
        }

        if !self.display.fb_dirty && self.texture.is_some() {
            return;
        }

        let pixels: Vec<egui::Color32> = self
            .display
            .framebuffer
            .chunks_exact(4)
            .map(|rgba| egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]))
            .collect();

        let expected = w * h;
        let image = if pixels.len() == expected {
            egui::ColorImage {
                size: [w, h],
                pixels,
            }
        } else {
            let mut padded = vec![egui::Color32::BLACK; expected];
            let copy_len = pixels.len().min(expected);
            padded[..copy_len].copy_from_slice(&pixels[..copy_len]);
            egui::ColorImage {
                size: [w, h],
                pixels: padded,
            }
        };

        self.display.fb_dirty = false;
        let options = egui::TextureOptions::NEAREST;

        match &mut self.texture {
            Some(tex) => tex.set(image, options),
            None => {
                self.texture = Some(ctx.load_texture("vga_display", image, options));
            }
        }
    }

    /// Process keyboard input and send scancodes to the emulator.
    fn process_keyboard(&mut self, ctx: &egui::Context) {
        let emu = match self.emulator.as_mut() {
            Some(e) => e,
            None => return,
        };

        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Text(text) => {
                        for ch in text.chars() {
                            let seq = rusty_box::gui::char_to_scancode_sequence(ch);
                            for sc in &seq {
                                emu.send_scancode(*sc);
                            }
                        }
                    }
                    egui::Event::Key { key, pressed, .. } => {
                        let seq = egui_key_to_scancodes(*key, *pressed);
                        for sc in &seq {
                            emu.send_scancode(*sc);
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    /// Render the top header bar.
    fn render_header(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .exact_height(36.0)
            .frame(
                egui::Frame::NONE
                    .fill(BG_DARK)
                    .inner_margin(egui::Margin::symmetric(16, 0))
                    .stroke(egui::Stroke::new(0.5, BORDER_SUBTLE)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // App title
                    ui.label(
                        egui::RichText::new("Rusty Box")
                            .strong()
                            .size(14.0)
                            .color(TEXT_PRIMARY),
                    );
                    ui.label(
                        egui::RichText::new("x86 Emulator")
                            .size(11.0)
                            .color(TEXT_MUTED),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Right-aligned info
                        let instr_text = if self.total_instructions > 0 {
                            if self.total_instructions >= 1_000_000 {
                                format!("{}M instr", self.total_instructions / 1_000_000)
                            } else {
                                format!("{}K instr", self.total_instructions / 1_000)
                            }
                        } else {
                            "0 instr".to_string()
                        };
                        ui.label(
                            egui::RichText::new(instr_text)
                                .monospace()
                                .size(11.0)
                                .color(TEXT_DIM),
                        );
                    });
                });
            });
    }

    /// Render the bottom status bar.
    fn render_status_bar(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(28.0)
            .frame(
                egui::Frame::NONE
                    .fill(BG_DARK)
                    .inner_margin(egui::Margin::symmetric(16, 0))
                    .stroke(egui::Stroke::new(0.5, BORDER_SUBTLE)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;

                    // Status indicator (colored dot + text)
                    let (dot_color, status_text) = if self.init_error.is_some() {
                        (ACCENT_RED, "Error")
                    } else if !self.initialized {
                        (ACCENT_YELLOW, "Initializing")
                    } else if self.shutdown {
                        (TEXT_DIM, "Finished")
                    } else {
                        (ACCENT_GREEN, "Running")
                    };

                    // Colored dot
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.5, dot_color);

                    ui.label(
                        egui::RichText::new(status_text)
                            .monospace()
                            .size(11.0)
                            .color(dot_color),
                    );

                    // Separator
                    ui.label(egui::RichText::new("|").size(11.0).color(BORDER_SUBTLE));

                    // IPS
                    let ips_str = Self::format_ips(self.cached_ips);
                    ui.label(
                        egui::RichText::new(format!("{} IPS", ips_str))
                            .monospace()
                            .size(11.0)
                            .color(ACCENT_CYAN),
                    );

                    // Separator
                    ui.label(egui::RichText::new("|").size(11.0).color(BORDER_SUBTLE));

                    // Frame counter
                    ui.label(
                        egui::RichText::new(format!("frame {}", self.frame_count))
                            .monospace()
                            .size(11.0)
                            .color(TEXT_MUTED),
                    );

                    // Right-aligned: batch size info
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("batch: {}K/frame", BATCH_SIZE / 1000))
                                .monospace()
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                    });
                });
            });
    }

    /// Render the main VGA display area.
    fn render_display(&self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG_DARKEST))
            .show(ctx, |ui| {
                if let Some(ref err) = self.init_error {
                    // Error state
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(
                                egui::RichText::new("Initialization Error")
                                    .size(18.0)
                                    .color(ACCENT_RED),
                            );
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(err)
                                    .monospace()
                                    .size(12.0)
                                    .color(TEXT_DIM),
                            );
                        });
                    });
                } else if let Some(ref tex) = self.texture {
                    // VGA display — centered with integer scaling
                    let available = ui.available_size();
                    let tex_w = self.display.fb_width as f32;
                    let tex_h = self.display.fb_height.max(1) as f32;

                    let max_scale_x = (available.x / tex_w).floor().max(1.0);
                    let max_scale_y = (available.y / tex_h).floor().max(1.0);
                    let scale = max_scale_x.min(max_scale_y);
                    let (w, h) = (tex_w * scale, tex_h * scale);

                    let offset_x = (available.x - w) / 2.0;
                    let offset_y = (available.y - h) / 2.0;

                    // Subtle border around the display
                    let display_rect = egui::Rect::from_min_size(
                        ui.min_rect().min + egui::vec2(offset_x, offset_y),
                        egui::vec2(w, h),
                    );

                    // Outer glow / border
                    ui.painter().rect_stroke(
                        display_rect.expand(1.0),
                        0.0,
                        egui::Stroke::new(1.0, BORDER_SUBTLE),
                        egui::StrokeKind::Outside,
                    );

                    ui.add_space(offset_y);
                    ui.horizontal(|ui| {
                        ui.add_space(offset_x);
                        ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(w, h)));
                    });
                } else if !self.initialized {
                    // Loading state
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(
                                egui::RichText::new("Starting emulator...")
                                    .size(16.0)
                                    .color(TEXT_DIM),
                            );
                            ui.add_space(8.0);
                            ui.spinner();
                        });
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("Waiting for VGA output...")
                                .size(14.0)
                                .color(TEXT_DIM),
                        );
                    });
                }
            });
    }
}

impl eframe::App for WasmEmulatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        Self::apply_theme(ctx);
        self.frame_count += 1;

        // Deferred initialization (first frame)
        if !self.initialized && self.init_error.is_none() {
            self.initialize_emulator();
        }

        // Execute multiple batches of instructions per frame.
        // cpu_loop_n_with_io may return early (async events, trace breaks),
        // so we loop until we hit the per-frame budget.
        if self.initialized && !self.shutdown {
            if let Some(ref mut emu) = self.emulator {
                let mut frame_executed = 0u64;
                while frame_executed < FRAME_BUDGET {
                    match emu.step_batch(BATCH_SIZE) {
                        Ok((executed, is_shutdown)) => {
                            frame_executed += executed;
                            if is_shutdown {
                                self.shutdown = true;
                                log::info!(
                                    "CPU shutdown after {} instructions",
                                    self.total_instructions + frame_executed
                                );
                                break;
                            }
                            if executed == 0 {
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!("step_batch error: {:?}", e);
                            self.shutdown = true;
                            break;
                        }
                    }
                }
                self.total_instructions += frame_executed;

                // Render VGA text → pixel framebuffer
                emu.update_display(&mut self.display);

                // Diagnostic: log display state periodically
                if self.frame_count % 300 == 1 {
                    let non_zero = self
                        .display
                        .framebuffer
                        .chunks(4)
                        .filter(|px| px[0] != 0 || px[1] != 0 || px[2] != 0)
                        .count();
                    // Check VGA text memory directly
                    let vga_dump = emu.vga_text_dump();
                    let vga_trimmed = vga_dump.trim();
                    let vga_preview: String = vga_trimmed.chars().take(80).collect();
                    // Check VGA handler probes and memory handler count
                    let probe = emu.vga_probe_summary();
                    let handler_count = emu.memory_handler_count();
                    log::info!(
                        "Display: {}x{}, dirty={}, non_zero_px={}, vga_dump_len={}, preview=[{}]",
                        self.display.fb_width,
                        self.display.fb_height,
                        self.display.fb_dirty,
                        non_zero,
                        vga_trimmed.len(),
                        vga_preview
                    );
                    // Check raw RAM at 0xB8000 (VGA text area) and ROM at 0xC0000 (VGA BIOS)
                    let ram_b8000 = emu.peek_ram_at(0xB8000, 32);
                    let ram_c0000 = emu.peek_ram_at(0xC0000, 16);
                    let ram_non_zero = ram_b8000.iter().filter(|&&b| b != 0).count();
                    let (cs, rip) = emu.get_cs_rip();
                    let mode = emu.get_cpu_mode_str();
                    let cr0 = emu.get_cr0();
                    let if_flag = emu.get_if_flag();
                    let activity = emu.get_activity_str();
                    // Check BIOS ROM at reset vector offset (0x3FFFF0 in 4MB ROM array)
                    let rom_reset = emu.peek_rom(0x3FFFF0, 16);
                    // Check BIOS ROM at 0x3E0000 (start of 128KB BIOS in ROM)
                    let rom_start = emu.peek_rom(0x3E0000, 16);
                    log::info!(
                        "CPU: CS={:#06x} RIP={:#010x} mode={} CR0={:#010x} IF={} activity={} | handlers={} | RAM@B8={}",
                        cs, rip, mode, cr0, if_flag, activity, handler_count, ram_non_zero,
                    );
                    // VGA BIOS at ROM offset 0x400000 (expansion ROM area)
                    let rom_vgabios = emu.peek_rom(0x400000, 16);
                    log::info!(
                        "ROM@reset={:02x?} ROM@start={:02x?} ROM@vga={:02x?} RAM@C0={:02x?} GR6={:#04x} | VGA: {}",
                        &rom_reset[..rom_reset.len().min(16)],
                        &rom_start[..rom_start.len().min(16)],
                        &rom_vgabios[..rom_vgabios.len().min(16)],
                        &ram_c0000[..ram_c0000.len().min(16)],
                        emu.peek_vga_gr6(),
                        probe.trim(),
                    );
                }
            }
        }

        // Process keyboard input
        self.process_keyboard(ctx);

        // Update IPS
        self.update_ips();

        // Upload framebuffer texture
        self.upload_texture(ctx);

        // Render UI
        self.render_header(ctx);
        self.render_status_bar(ctx);
        self.render_display(ctx);

        // Keep repainting while running
        if !self.shutdown {
            ctx.request_repaint();
        }
    }
}

// ---- PS/2 scancode mapping for egui keys ----

fn egui_key_to_scancodes(key: egui::Key, pressed: bool) -> Vec<u8> {
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
