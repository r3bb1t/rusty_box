use eframe::egui;
use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::{shared_display::SharedDisplay, BxGui, DisplayMode, RustyBoxApp, VgaTextModeInfo},
};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

const BIOS_DATA: &[u8] = include_bytes!("../../cpp_orig/bochs/bios/BIOS-bochs-latest");
const VGA_BIOS_DATA: &[u8] =
    include_bytes!("../../cpp_orig/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin");
#[cfg(feature = "embedded-alpine")]
const ALPINE_ISO: &[u8] = include_bytes!("../assets/alpine.iso");

const ANDROID_MEMORY_MIB: usize = 256;
const ANDROID_IPS: u32 = 300_000_000;
const BATCH_SIZE: u64 = 50_000;
const FRAME_BUDGET: u64 = 200_000;
const ANDROID_EMULATOR_STACK_SIZE: usize = 256 * 1024 * 1024;
const SERIAL_LOG_LIMIT: usize = 65_536;
const SERIAL_LOG_RETAIN: usize = 49_152;

type AndroidEmulator =
    Box<rusty_box::emulator::Emulator<'static, rusty_box::cpu::core_i7_skylake::Corei7SkylakeX>>;

#[cfg(feature = "embedded-alpine")]
fn embedded_alpine_iso() -> Result<&'static [u8], &'static str> {
    Ok(ALPINE_ISO)
}

#[cfg(not(feature = "embedded-alpine"))]
fn embedded_alpine_iso() -> Result<&'static [u8], &'static str> {
    Err("embedded Alpine ISO not compiled; copy C:/Users/Aslan/Downloads/alpine-virt-3.23.3-x86_64.iso to rusty_box_android/assets/alpine.iso and build with --features embedded-alpine")
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };

    eframe::run_native(
        "Rusty Box",
        options,
        Box::new(|cc| Ok(Box::new(RustyBoxAndroidApp::new(cc)))),
    )
    .unwrap();
}

struct AndroidBridgeGui {
    shared: Arc<Mutex<SharedDisplay>>,
    local_scancodes: VecDeque<u8>,
    display_mode: DisplayMode,
}

impl AndroidBridgeGui {
    fn new(shared: Arc<Mutex<SharedDisplay>>) -> Self {
        Self {
            shared,
            local_scancodes: VecDeque::new(),
            display_mode: DisplayMode::Sim,
        }
    }
}

impl BxGui for AndroidBridgeGui {
    fn specific_init(&mut self, _argc: i32, _argv: &[&str], _header_bar_y: u32) {
        log::debug!("AndroidBridgeGui initialized");
    }

    fn text_update(
        &mut self,
        _old_text: &[u8],
        new_text: &[u8],
        cursor_x: u32,
        cursor_y: u32,
        tm_info: &VgaTextModeInfo,
    ) {
        if let Ok(mut display) = self.shared.lock() {
            display.render_vga_text_update(new_text, cursor_x, cursor_y, tm_info);
        }
    }

    fn graphics_tile_update(&mut self, _tile: &[u8], _x: u32, _y: u32) {
        // Graphics modes are outside the first Android milestone.
    }

    fn handle_events(&mut self) {
        if let Ok(mut display) = self.shared.lock() {
            for scancode in display.pending_scancodes.drain(..) {
                self.local_scancodes.push_back(scancode);
            }
        }
    }

    fn flush(&mut self) {}

    fn clear_screen(&mut self) {
        if let Ok(mut display) = self.shared.lock() {
            display.framebuffer.fill(0);
            display.fb_dirty = true;
        }
    }

    fn palette_change(&mut self, index: u8, red: u8, green: u8, blue: u8) -> bool {
        if let Ok(mut display) = self.shared.lock() {
            if let Some(color) = display.palette.get_mut(index as usize) {
                *color = [red, green, blue];
            }
        }
        true
    }

    fn dimension_update(&mut self, x: u32, y: u32, fheight: u32, fwidth: u32, _bpp: u32) {
        if let Ok(mut display) = self.shared.lock() {
            let cols = x.checked_div(fwidth).unwrap_or(x);
            let rows = y.checked_div(fheight).unwrap_or(y);
            display.resize(cols, rows, fwidth, fheight);
        }
    }

    fn create_bitmap(&mut self, _bmap: &[u8], _xdim: u32, _ydim: u32) -> u32 {
        0
    }

    fn headerbar_bitmap(
        &mut self,
        _bmap_id: u32,
        _alignment: u32,
        _callback: Box<dyn Fn()>,
    ) -> u32 {
        0
    }

    fn replace_bitmap(&mut self, _hbar_id: u32, _bmap_id: u32) {}
    fn show_headerbar(&mut self) {}
    fn get_clipboard_text(&mut self) -> Option<Vec<u8>> {
        None
    }
    fn set_clipboard_text(&mut self, _text: &str) -> bool {
        false
    }
    fn mouse_enabled_changed_specific(&mut self, _val: bool) {}

    fn exit(&mut self) {
        if let Ok(mut display) = self.shared.lock() {
            display.emu_running = false;
        }
    }

    fn set_display_mode(&mut self, mode: DisplayMode) {
        self.display_mode = mode;
    }

    fn show_ips(&mut self, ips_count: u32) {
        if let Ok(mut display) = self.shared.lock() {
            display.ips = ips_count;
        }
    }

    fn get_pending_scancodes(&mut self) -> Vec<u8> {
        self.local_scancodes.drain(..).collect()
    }

    fn get_pending_serial_input(&mut self) -> Vec<u8> {
        self.shared
            .lock()
            .map(|mut display| display.drain_serial_input())
            .unwrap_or_default()
    }

    fn append_serial_log(&self, text: &str) {
        if let Ok(mut display) = self.shared.lock() {
            append_serial_log(&mut display.serial_log, text.as_bytes());
        }
    }
}

pub struct RustyBoxAndroidApp {
    display: Arc<Mutex<SharedDisplay>>,
    screen: RustyBoxApp,
    worker: Option<JoinHandle<()>>,
    total_instructions_shared: Arc<AtomicU64>,
    initialized: bool,
    init_error: Option<String>,
    shutdown: bool,
    total_instructions: u64,
    last_ips_time: Instant,
    last_ips_instructions: u64,
    cached_ips: f64,
    key_text: String,
}

impl RustyBoxAndroidApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let display = Arc::new(Mutex::new(SharedDisplay::new()));
        let screen = build_core_screen_view(Arc::clone(&display));

        Self {
            display,
            screen,
            worker: None,
            total_instructions_shared: Arc::new(AtomicU64::new(0)),
            initialized: false,
            init_error: None,
            shutdown: false,
            total_instructions: 0,
            last_ips_time: Instant::now(),
            last_ips_instructions: 0,
            cached_ips: 0.0,
            key_text: String::new(),
        }
    }

    fn initialize_alpine(&mut self) {
        if self.initialized {
            return;
        }

        let shared = Arc::clone(&self.display);
        let total_instructions = Arc::clone(&self.total_instructions_shared);
        self.total_instructions_shared.store(0, Ordering::Relaxed);
        if let Ok(mut display) = shared.lock() {
            display.stop_flag.store(false, Ordering::Relaxed);
            display.emu_running = false;
            display.start_pending = true;
            display.reset_requested = false;
            display.runtime_error = None;
            display.serial_log.clear();
            drop(display.drain_serial_input());
        }

        match thread::Builder::new()
            .name("rusty-box-android-emulator".to_string())
            .stack_size(ANDROID_EMULATOR_STACK_SIZE)
            .spawn(move || {
                let shared_for_error = Arc::clone(&shared);
                if let Err(error) = run_alpine_emulator_worker(shared, total_instructions) {
                    if let Ok(mut display) = shared_for_error.lock() {
                        display.emu_running = false;
                        display.start_pending = false;
                        display.runtime_error = Some(error.clone());
                    }
                    log::error!("Android emulator worker failed: {error}");
                }
            }) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.initialized = true;
                self.init_error = None;
            }
            Err(error) => {
                let message = format!("failed to spawn Android emulator worker: {error}");
                if let Ok(mut display) = self.display.lock() {
                    display.emu_running = false;
                    display.start_pending = false;
                    display.runtime_error = Some(message.clone());
                }
                self.init_error = Some(message);
                self.initialized = false;
            }
        }
    }

    fn sync_worker_status(&mut self) {
        self.total_instructions = self.total_instructions_shared.load(Ordering::Relaxed);

        if let Ok(display) = self.display.lock() {
            if let Some(error) = display.runtime_error.clone() {
                self.init_error = Some(error);
            }
        }

        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(worker) = self.worker.take() {
                if worker.join().is_err() {
                    self.init_error = Some("Android emulator worker panicked".to_string());
                }
            }
            if let Ok(mut display) = self.display.lock() {
                display.emu_running = false;
                display.start_pending = false;
            }
            self.shutdown = true;
        }
    }

    fn update_ips(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_ips_time);
        if elapsed.as_secs_f64() >= 1.0 {
            let delta = self
                .total_instructions
                .saturating_sub(self.last_ips_instructions);
            self.cached_ips = delta as f64 / elapsed.as_secs_f64();
            self.last_ips_time = now;
            self.last_ips_instructions = self.total_instructions;
        }
    }

    fn format_ips(ips: f64) -> String {
        if ips >= 1_000_000.0 {
            format!("{:.2}M", ips / 1_000_000.0)
        } else if ips >= 1_000.0 {
            format!("{:.0}K", ips / 1_000.0)
        } else if ips > 0.0 {
            format!("{ips:.0}")
        } else {
            "---".to_string()
        }
    }

    fn render_control_row(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Rusty Box Android").strong());
            ui.separator();
            ui.label(format!("{} instr", self.total_instructions));
            ui.separator();
            ui.label(format!("{} IPS", Self::format_ips(self.cached_ips)));
            ui.separator();
            ui.label(if self.shutdown { "shutdown" } else { "running" });
            ui.separator();

            let response = ui.add(
                egui::TextEdit::singleline(&mut self.key_text)
                    .desired_width(120.0)
                    .hint_text("PS/2 keys"),
            );
            let send_text = (!self.key_text.is_empty()
                && response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                || ui
                    .add_enabled(!self.key_text.is_empty(), egui::Button::new("Send"))
                    .clicked();
            if send_text {
                queue_text_keys(&self.display, &self.key_text);
                self.key_text.clear();
            }

            for (label, key) in [
                ("Esc", egui::Key::Escape),
                ("Tab", egui::Key::Tab),
                ("Enter", egui::Key::Enter),
                ("←", egui::Key::ArrowLeft),
                ("↑", egui::Key::ArrowUp),
                ("↓", egui::Key::ArrowDown),
                ("→", egui::Key::ArrowRight),
            ] {
                if ui.add(egui::Button::new(label)).clicked() {
                    queue_touch_key(&self.display, key);
                }
            }
        });
    }
}

impl eframe::App for RustyBoxAndroidApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.init_error.is_none() && !self.initialized {
            self.initialize_alpine();
        }

        self.sync_worker_status();

        if let Some(error) = self.init_error.clone() {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.colored_label(egui::Color32::RED, error);
                    ui.label(
                        "Rebuild with --features embedded-alpine after copying the Alpine ISO.",
                    );
                });
            });
            return;
        }

        if !self.initialized {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("Booting Alpine from embedded ISO...");
                });
            });
            ui.ctx().request_repaint();
            return;
        }

        self.sync_worker_status();
        self.update_ips();
        self.render_control_row(ui);
        self.screen.ui_embedded_with_serial(ui, frame, false);

        if !self.shutdown {
            ui.ctx().request_repaint();
        }
    }
}

impl Drop for RustyBoxAndroidApp {
    fn drop(&mut self) {
        if let Ok(display) = self.display.lock() {
            display.stop_flag.store(true, Ordering::Relaxed);
        }
    }
}

fn run_alpine_emulator_worker(
    shared: Arc<Mutex<SharedDisplay>>,
    total_instructions: Arc<AtomicU64>,
) -> Result<(), String> {
    let iso = embedded_alpine_iso().map_err(str::to_string)?;
    let ram_size = ANDROID_MEMORY_MIB * 1024 * 1024;
    let config = EmulatorConfig {
        guest_memory_size: ram_size,
        host_memory_size: ram_size,
        memory_block_size: 128 * 1024,
        ips: ANDROID_IPS,
        pci_enabled: true,
        ..Default::default()
    };

    let stop_flag = shared
        .lock()
        .map(|display| Arc::clone(&display.stop_flag))
        .unwrap_or_else(|_| Arc::new(std::sync::atomic::AtomicBool::new(false)));

    let mut emu: AndroidEmulator =
        Emulator::<Corei7SkylakeX>::new(config).map_err(|error| format!("{error:?}"))?;
    emu.stop_flag = stop_flag;
    emu.set_gui(AndroidBridgeGui::new(Arc::clone(&shared)));
    emu.init_memory_and_pc_system()
        .map_err(|error| format!("{error:?}"))?;
    emu.load_bios(BIOS_DATA, !(BIOS_DATA.len() as u64 - 1))
        .map_err(|error| format!("{error:?}"))?;

    let mut vga_data = VGA_BIOS_DATA.to_vec();
    let remainder = vga_data.len() % 512;
    if remainder != 0 {
        vga_data.resize(vga_data.len() + (512 - remainder), 0);
    }
    emu.load_optional_rom(&vga_data, 0xC0000)
        .map_err(|error| format!("{error:?}"))?;

    emu.init_cpu_and_devices()
        .map_err(|error| format!("{error:?}"))?;
    emu.configure_memory_in_cmos_from_config();
    emu.configure_boot_sequence(3, 0, 0);
    emu.attach_cdrom_data_ref(1, 0, iso);
    emu.init_gui(0, &[]).map_err(|error| format!("{error:?}"))?;
    emu.reset(ResetReason::Hardware)
        .map_err(|error| format!("{error:?}"))?;
    emu.init_gui_signal_handlers();
    emu.prepare_run();
    emu.force_vga_update();

    if let Ok(mut display) = shared.lock() {
        display.emu_running = true;
        display.start_pending = false;
    }

    let interactive_budget = FRAME_BUDGET.max(BATCH_SIZE);

    let run_result = loop {
        if emu.stop_flag.load(Ordering::Relaxed) {
            break Ok(());
        }

        match emu.run_interactive(interactive_budget) {
            Ok(executed) => {
                total_instructions.fetch_add(executed, Ordering::Relaxed);
                if emu.cpu.is_in_shutdown() {
                    break Ok(());
                }
                if executed == 0 {
                    thread::yield_now();
                }
            }
            Err(error) => break Err(format!("{error:?}")),
        }
    };

    if let Ok(mut display) = shared.lock() {
        display.emu_running = false;
        display.start_pending = false;
        if let Err(error) = &run_result {
            display.runtime_error = Some(error.clone());
        }
    }

    run_result
}

fn build_core_screen_view(shared: Arc<Mutex<SharedDisplay>>) -> RustyBoxApp {
    RustyBoxApp::new_embedded(shared)
}

fn queue_touch_key(shared: &Arc<Mutex<SharedDisplay>>, key: egui::Key) {
    let mut scancodes = egui_key_to_scancodes(key, true);
    scancodes.extend_from_slice(&egui_key_to_scancodes(key, false));
    if scancodes.is_empty() {
        return;
    }

    if let Ok(mut display) = shared.lock() {
        display.pending_scancodes.extend_from_slice(&scancodes);
    }
}

fn queue_text_keys(shared: &Arc<Mutex<SharedDisplay>>, text: &str) {
    let mut scancodes = Vec::new();
    for ch in text.chars() {
        scancodes.extend_from_slice(&rusty_box::gui::char_to_scancode_sequence(ch));
    }
    if scancodes.is_empty() {
        return;
    }

    if let Ok(mut display) = shared.lock() {
        display.pending_scancodes.extend_from_slice(&scancodes);
    }
}

fn append_serial_log(log: &mut String, bytes: &[u8]) {
    log.push_str(&String::from_utf8_lossy(bytes));
    if log.len() > SERIAL_LOG_LIMIT {
        let drain = log.len() - SERIAL_LOG_RETAIN;
        log.drain(..drain);
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "embedded-alpine"))]
    #[test]
    fn embedded_alpine_iso_returns_documented_error_without_feature() {
        let error = embedded_alpine_iso().expect_err("feature should be disabled by default");
        assert!(error.contains("embedded Alpine ISO not compiled"));
        assert!(error.contains("--features embedded-alpine"));
    }

    #[test]
    fn android_boot_constants_are_fixed() {
        assert_eq!(ANDROID_MEMORY_MIB, 256);
        assert_eq!(ANDROID_IPS, 300_000_000);
        assert_eq!(BATCH_SIZE, 50_000);
        assert_eq!(FRAME_BUDGET, 200_000);
        assert_eq!(ANDROID_EMULATOR_STACK_SIZE, 256 * 1024 * 1024);
    }

    #[test]
    fn enter_key_maps_to_ps2_set_two_make_and_break() {
        assert_eq!(egui_key_to_scancodes(egui::Key::Enter, true), [0x5A]);
        assert_eq!(egui_key_to_scancodes(egui::Key::Enter, false), [0xF0, 0x5A]);
    }

    #[test]
    fn app_dependency_exposes_character_scancode_mapping() {
        assert!(!rusty_box::gui::char_to_scancode_sequence('a').is_empty());
    }

    #[test]
    fn android_screen_view_comes_from_core_gui_app() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));

        let _screen = build_core_screen_view(Arc::clone(&shared));
    }

    #[test]
    fn touch_key_buttons_queue_ps2_scancodes() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));

        queue_touch_key(&shared, egui::Key::Enter);

        assert_eq!(shared.lock().unwrap().pending_scancodes, [0x5A, 0xF0, 0x5A]);
    }

    #[test]
    fn touch_text_input_queues_ps2_scancodes() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));

        queue_text_keys(&shared, "a");

        assert_eq!(
            shared.lock().unwrap().pending_scancodes,
            rusty_box::gui::char_to_scancode_sequence('a')
        );
    }
    #[test]
    fn append_serial_log_appends_lossy_text_and_caps_retained_output() {
        let mut log = String::from("start:");
        append_serial_log(&mut log, &[0xFF, b'a']);
        assert!(log.contains("�a"));

        let mut long_log = "x".repeat(SERIAL_LOG_LIMIT);
        append_serial_log(&mut long_log, "0123456789".as_bytes());
        assert!(long_log.len() <= SERIAL_LOG_RETAIN);
        assert!(long_log.ends_with("0123456789"));
    }

    #[test]
    fn android_bridge_uses_existing_gui_serial_hooks() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));
        let mut bridge = AndroidBridgeGui::new(Arc::clone(&shared));

        {
            let mut display = shared.lock().expect("display lock");
            display.queue_serial_input_line("uname -a");
        }
        assert_eq!(bridge.get_pending_serial_input(), b"uname -a\n");

        bridge.append_serial_log("booting\n");
        let display = shared.lock().expect("display lock");
        assert_eq!(display.serial_log, "booting\n");
    }
}
