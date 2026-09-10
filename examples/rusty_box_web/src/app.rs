//! WASM-compatible eframe application for Rusty Box.
//!
//! Single-threaded cooperative execution: each frame calls `Emulator::step`
//! until the frame's instruction budget is spent or the guest stops, then
//! renders the VGA framebuffer as an egui
//! texture. No threads, no Arc<Mutex<>> — the emulator and display are
//! owned directly by the app.

use rusty_box::{
    emulator::{
        AtaSlot, BootDevice, BootOrder, DiskGeometry, Emulator, EmulatorConfig, Ips, MemorySize, MachineBuilder, RunBudget,
    },
    gui::shared_display::SharedDisplay,
};

/// The VGA BIOS padded to a whole number of 512-byte option-ROM blocks, which
/// is what the ROM window expects.
fn padded_vga_bios() -> Vec<u8> {
    let mut data = VGA_BIOS_DATA.to_vec();
    let remainder = data.len() % 512;
    if remainder != 0 {
        data.resize(data.len() + (512 - remainder), 0);
    }
    data
}

// Embedded binary assets (compiled into the WASM)
const BIOS_DATA: &[u8] = include_bytes!("../../../cpp_orig/bochs/bochs/bios/BIOS-bochs-latest");
const VGA_BIOS_DATA: &[u8] = include_bytes!(
    "../../../cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"
);
const DISK_DATA: &[u8] = include_bytes!("../../../dlxlinux/hd10meg.img");

/// DLX Linux disk geometry
const DLX_CYLINDERS: u16 = 306;
const DLX_HEADS: u8 = 4;
const DLX_SPT: u8 = 17;

/// Instruction budget of one `Emulator::step` call.
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

#[derive(Clone, PartialEq)]
enum BootMode {
    /// Show launcher screen — user picks what to boot
    Launcher,
    /// DLX Linux from embedded disk image
    Dlx,
    /// Alpine Linux from user-uploaded ISO. Constructed only by the wasm
    /// file-upload flow; host builds still match on it.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Alpine,
}

/// The eframe application — owns the emulator and display directly.
pub struct WasmEmulatorApp {
    boot_mode: BootMode,
    emulator: Option<Box<Emulator>>,
    display: SharedDisplay,
    texture: Option<egui::TextureHandle>,
    initialized: bool,
    init_error: Option<String>,
    shutdown: bool,

    /// Pending Alpine ISO data from file upload
    pending_iso: Option<Vec<u8>>,
    /// Shared slot for file upload result (WASM)
    #[cfg(target_arch = "wasm32")]
    file_slot: std::rc::Rc<core::cell::RefCell<Option<Vec<u8>>>>,

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
            boot_mode: BootMode::Launcher,
            emulator: None,
            display: SharedDisplay::new(),
            texture: None,
            initialized: false,
            init_error: None,
            shutdown: false,
            pending_iso: None,
            #[cfg(target_arch = "wasm32")]
            file_slot: std::rc::Rc::new(core::cell::RefCell::new(None)),
            total_instructions: 0,
            last_ips_time: web_time::Instant::now(),
            last_ips_instructions: 0,
            cached_ips: 0.0,
            frame_count: 0,
        }
    }

    /// Initialize the emulator for DLX Linux (embedded disk).
    fn initialize_dlx(&mut self) {
        let config = EmulatorConfig {
            memory: MemorySize::bytes(32 * 1024 * 1024),
            memory_block_size: 128 * 1024,
            ips: Ips::new(300_000_000),
            pci_enabled: true,
            ..Default::default()
        };

        let result = (|| -> rusty_box::Result<Box<Emulator>> {
            let vga_data = padded_vga_bios();
            let mut emu = MachineBuilder::new(config)
                .bios(BIOS_DATA)
                .vga_bios(&vga_data)
                .boot_order(BootOrder::just(BootDevice::Disk))
                .disk_bytes(
                    AtaSlot::PRIMARY_MASTER,
                    DISK_DATA.to_vec(),
                    DiskGeometry::new(DLX_CYLINDERS.into(), DLX_HEADS, DLX_SPT),
                )
                .build()?;
            emu.display().force_update();
            Ok(emu)
        })();

        self.finish_init(result);
    }

    /// Initialize the emulator for Alpine Linux (user-provided ISO).
    fn initialize_alpine(&mut self, iso_data: Vec<u8>) {
        let ram_size = 256 * 1024 * 1024; // 256 MB for Alpine
        let config = EmulatorConfig {
            memory: MemorySize::bytes(ram_size),
            memory_block_size: 128 * 1024,
            ips: Ips::new(300_000_000),
            pci_enabled: true,
            ..Default::default()
        };

        let result = (|| -> rusty_box::Result<Box<Emulator>> {
            let vga_data = padded_vga_bios();
            // The CMOS memory size comes from the configuration, so the guest
            // is told about all 256 MB.
            let mut emu = MachineBuilder::new(config)
                .bios(BIOS_DATA)
                .vga_bios(&vga_data)
                .boot_order(BootOrder::just(BootDevice::Cdrom))
                .cdrom_bytes(AtaSlot::SECONDARY_MASTER, iso_data)
                .build()?;
            emu.display().force_update();
            Ok(emu)
        })();

        self.finish_init(result);
    }

    fn finish_init(&mut self, result: rusty_box::Result<Box<Emulator>>) {
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
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT_DIM);
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(0.5_f32, BORDER_SUBTLE);

        visuals.widgets.inactive.bg_fill = BG_SURFACE;
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT_PRIMARY);
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(0.5_f32, BORDER_SUBTLE);

        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x2E, 0x2E, 0x4A);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, TEXT_PRIMARY);

        visuals.widgets.active.bg_fill = ACCENT_BLUE;
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, TEXT_PRIMARY);

        visuals.selection.bg_fill = egui::Color32::from_rgba_premultiplied(0x56, 0x9C, 0xD6, 0x40);
        visuals.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT_BLUE);

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
            egui::ColorImage::new([w, h], pixels)
        } else {
            let mut padded = vec![egui::Color32::BLACK; expected];
            let copy_len = pixels.len().min(expected);
            padded[..copy_len].copy_from_slice(&pixels[..copy_len]);
            egui::ColorImage::new([w, h], padded)
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
                for seq in key_sequences(event) {
                    // `scancodes` stops at the first byte the guest's ring
                    // refuses and returns how many it took. Nothing re-sends
                    // the rest: a full ring can leave the guest a make with no
                    // break, or a prefix with no code, so the dropped bytes
                    // are logged.
                    let sent = emu.keyboard().scancodes(&seq);
                    if sent < seq.len() {
                        log::warn!(
                            "guest keyboard buffer full: dropped scancodes {:02X?}",
                            &seq[sent..]
                        );
                    }
                }
            }
        });
    }

    /// Trigger a file picker for Alpine ISO upload (WASM).
    #[cfg(target_arch = "wasm32")]
    fn open_file_picker(&mut self) {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        let document = web_sys::window().unwrap().document().unwrap();
        let input: web_sys::HtmlInputElement = document
            .create_element("input")
            .unwrap()
            .dyn_into()
            .unwrap();
        input.set_type("file");
        input.set_accept(".iso,.img");

        let slot = self.file_slot.clone();

        let closure = Closure::once(move |event: web_sys::Event| {
            let input: web_sys::HtmlInputElement = event.target().unwrap().dyn_into().unwrap();
            if let Some(files) = input.files() {
                if let Some(file) = files.get(0) {
                    let reader = web_sys::FileReader::new().unwrap();
                    let reader_clone = reader.clone();
                    let slot_inner = slot.clone();
                    let onload = Closure::once(move |_: web_sys::Event| {
                        if let Ok(result) = reader_clone.result() {
                            let array = js_sys::Uint8Array::new(&result);
                            let data = array.to_vec();
                            log::info!("FileReader onload: {} bytes", data.len());
                            *slot_inner.borrow_mut() = Some(data);
                        }
                    });
                    reader.set_onload(Some(onload.as_ref().unchecked_ref()));
                    onload.forget();
                    reader.read_as_array_buffer(&file).unwrap();
                }
            }
        });

        input
            .add_event_listener_with_callback("change", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();
        input.click();
    }

    /// Trigger a file picker for Alpine ISO upload (native — stub).
    #[cfg(not(target_arch = "wasm32"))]
    fn open_file_picker(&mut self) {
        log::warn!("File picker not implemented for native builds");
    }

    /// Render the launcher screen directly into ui (no CentralPanel wrapper).
    fn render_launcher(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);

            ui.label(
                egui::RichText::new("Rusty Box")
                    .size(32.0)
                    .strong()
                    .color(TEXT_PRIMARY),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("x86 Emulator — Rust port of Bochs")
                    .size(14.0)
                    .color(TEXT_DIM),
            );

            ui.add_space(48.0);

            let dlx_btn = egui::Button::new(
                egui::RichText::new("Boot DLX Linux")
                    .size(16.0)
                    .color(TEXT_PRIMARY),
            )
            .fill(BG_SURFACE)
            .stroke(egui::Stroke::new(1.0_f32, ACCENT_GREEN))
            .min_size(egui::vec2(280.0, 48.0));

            if ui.add(dlx_btn).clicked() {
                self.boot_mode = BootMode::Dlx;
            }

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("10 MB embedded disk image — boots instantly")
                    .size(11.0)
                    .color(TEXT_MUTED),
            );

            ui.add_space(24.0);

            let alpine_btn = egui::Button::new(
                egui::RichText::new("Load Alpine Linux ISO")
                    .size(16.0)
                    .color(TEXT_PRIMARY),
            )
            .fill(BG_SURFACE)
            .stroke(egui::Stroke::new(1.0_f32, ACCENT_BLUE))
            .min_size(egui::vec2(280.0, 48.0));

            if ui.add(alpine_btn).clicked() {
                self.open_file_picker();
            }

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Upload an Alpine Virtual x86 ISO from alpinelinux.org/downloads",
                )
                .size(11.0)
                .color(TEXT_MUTED),
            );
        });
    }

    /// Render the top header bar.
    fn render_header(&self, ui: &mut egui::Ui) {
        egui::Panel::top("header")
            .exact_size(36.0)
            .frame(
                egui::Frame::NONE
                    .fill(BG_DARK)
                    .inner_margin(egui::Margin::symmetric(16, 0))
                    .stroke(egui::Stroke::new(0.5_f32, BORDER_SUBTLE)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Rusty Box")
                            .strong()
                            .size(14.0)
                            .color(TEXT_PRIMARY),
                    );

                    let mode_str = match self.boot_mode {
                        BootMode::Launcher => "Launcher",
                        BootMode::Dlx => "DLX Linux",
                        BootMode::Alpine => "Alpine Linux",
                    };
                    ui.label(egui::RichText::new(mode_str).size(11.0).color(TEXT_MUTED));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
    fn render_status_bar(&self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar")
            .exact_size(28.0)
            .frame(
                egui::Frame::NONE
                    .fill(BG_DARK)
                    .inner_margin(egui::Margin::symmetric(16, 0))
                    .stroke(egui::Stroke::new(0.5_f32, BORDER_SUBTLE)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;

                    let (dot_color, status_text) = if self.init_error.is_some() {
                        (ACCENT_RED, "Error")
                    } else if !self.initialized {
                        (ACCENT_YELLOW, "Initializing")
                    } else if self.shutdown {
                        (TEXT_DIM, "Finished")
                    } else {
                        (ACCENT_GREEN, "Running")
                    };

                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.5, dot_color);

                    ui.label(
                        egui::RichText::new(status_text)
                            .monospace()
                            .size(11.0)
                            .color(dot_color),
                    );

                    ui.label(egui::RichText::new("|").size(11.0).color(BORDER_SUBTLE));

                    let ips_str = Self::format_ips(self.cached_ips);
                    ui.label(
                        egui::RichText::new(format!("{} IPS", ips_str))
                            .monospace()
                            .size(11.0)
                            .color(ACCENT_CYAN),
                    );

                    ui.label(egui::RichText::new("|").size(11.0).color(BORDER_SUBTLE));

                    ui.label(
                        egui::RichText::new(format!("frame {}", self.frame_count))
                            .monospace()
                            .size(11.0)
                            .color(TEXT_MUTED),
                    );
                });
            });
    }

    /// Render the main VGA display area.
    fn render_display(&self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG_DARKEST))
            .show(ui, |ui| {
                if let Some(ref err) = self.init_error {
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

                    let display_rect = egui::Rect::from_min_size(
                        ui.min_rect().min + egui::vec2(offset_x, offset_y),
                        egui::vec2(w, h),
                    );

                    ui.painter().rect_stroke(
                        display_rect.expand(1.0),
                        0.0,
                        egui::Stroke::new(1.0_f32, BORDER_SUBTLE),
                        egui::StrokeKind::Outside,
                    );

                    ui.add_space(offset_y);
                    ui.horizontal(|ui| {
                        ui.add_space(offset_x);
                        ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(w, h)));
                    });
                } else if !self.initialized {
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        Self::apply_theme(&ctx);
        self.frame_count += 1;

        // Check for pending file upload result
        #[cfg(target_arch = "wasm32")]
        {
            let mut slot = self.file_slot.borrow_mut();
            if let Some(data) = slot.take() {
                log::info!("Received ISO data: {} bytes", data.len());
                self.pending_iso = Some(data);
                self.boot_mode = BootMode::Alpine;
            }
        }

        // Deferred initialization (when boot mode selected)
        if self.boot_mode != BootMode::Launcher && !self.initialized && self.init_error.is_none() {
            match self.boot_mode {
                BootMode::Dlx => self.initialize_dlx(),
                BootMode::Alpine => {
                    if let Some(iso) = self.pending_iso.take() {
                        self.initialize_alpine(iso);
                    }
                }
                BootMode::Launcher => {}
            }
        }

        // Execute batches
        if self.initialized && !self.shutdown {
            if let Some(ref mut emu) = self.emulator {
                let mut frame_executed = 0u64;
                while frame_executed < FRAME_BUDGET {
                    match emu.step(RunBudget::Instructions(BATCH_SIZE)) {
                        Ok(outcome) => {
                            // Whichever unit the machine measures in: this is a
                            // frame budget, not an instruction count.
                            frame_executed += outcome.progress.count();
                            // A terminal outcome ends the run for good. It
                            // includes a guest ACPI power-off, which leaves the
                            // CPU healthy, so the run's end is read from here and
                            // never from the CPU's state.
                            if outcome.is_terminal() {
                                self.shutdown = true;
                                break;
                            }
                            if outcome.progress.stalled() {
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!("step error: {:?}", e);
                            self.shutdown = true;
                            break;
                        }
                    }
                }
                self.total_instructions += frame_executed;
                emu.display().render_into(&mut self.display);
            }
        }

        // Process keyboard input
        self.process_keyboard(&ctx);

        // Update IPS
        self.update_ips();

        // Upload framebuffer texture
        self.upload_texture(&ctx);

        // Render UI — always same panel structure
        if self.boot_mode == BootMode::Launcher {
            self.render_launcher(ui);
        } else {
            self.render_header(ui);
            self.render_status_bar(ui);
            self.render_display(ui);
        }

        // Keep repainting while running
        if !self.shutdown {
            ctx.request_repaint();
        }
    }
}

// ---- PS/2 scancode mapping for egui keys ----

/// The Set 2 scancode sequences one egui input event sends the guest: one per
/// character of an `Event::Text`, one per `Event::Key`, each handed to the
/// keyboard in a single call.
///
/// `Event::Text` carries every key that types a character, the space bar
/// included. `Event::Key` is translated only for the keys
/// [`egui_key_to_scancodes`] maps, none of which eframe reports as text; any
/// other key yields an empty sequence, because a key that types a character
/// reaches the guest through its `Event::Text` and one that types nothing is
/// not sent. So a single press reaches the guest once.
fn key_sequences(event: &egui::Event) -> Vec<Vec<u8>> {
    match event {
        egui::Event::Text(text) => text
            .chars()
            .map(rusty_box::gui::char_to_scancode_sequence)
            .collect(),
        egui::Event::Key { key, pressed, .. } => vec![egui_key_to_scancodes(*key, *pressed)],
        _ => Vec::new(),
    }
}

/// Set 2 scancodes for the keys egui reports without text. A key that types
/// a character is absent here: it arrives as `Event::Text` instead.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn space_key(pressed: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Space,
            physical_key: Some(egui::Key::Space),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// One press of the space bar, in the order egui reports it — the key
    /// going down, the text it typed, the key coming up — reaches the guest
    /// as exactly one make and one break.
    #[test]
    fn one_space_press_reaches_the_guest_once() {
        let press = [
            space_key(true),
            egui::Event::Text(" ".to_owned()),
            space_key(false),
        ];

        let sent: Vec<u8> = press
            .iter()
            .flat_map(key_sequences)
            .flatten()
            .collect();

        assert_eq!(sent, [0x29, 0xF0, 0x29]);
    }
}
