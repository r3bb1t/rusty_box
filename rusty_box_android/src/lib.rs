use eframe::egui;
use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::{shared_display::SharedDisplay, BxGui, DisplayMode, VgaTextModeInfo},
    params::BxParams,
};
use rusty_box_gui::{
    app::{NativeEmulatorCommand, NativeShellApp},
    args::LogLevel,
    config::ResolvedCdrom,
    BootDevice, DisplayBackend, ResolvedConfig,
};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const BIOS_DATA: &[u8] = include_bytes!("../../cpp_orig/bochs/bochs/bios/BIOS-bochs-latest");
const VGA_BIOS_DATA: &[u8] =
    include_bytes!("../../cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin");
#[cfg(feature = "embedded-alpine")]
const ALPINE_ISO: &[u8] = include_bytes!("../assets/alpine.iso");

const ANDROID_MEMORY_MIB: usize = 256;
const ANDROID_IPS: u32 = 300_000_000;
const BATCH_SIZE: u64 = 50_000;
const FRAME_BUDGET: u64 = 200_000;
const ANDROID_EMULATOR_STACK_SIZE: usize = 256 * 1024 * 1024;
const SERIAL_LOG_LIMIT: usize = 65_536;
const SERIAL_LOG_RETAIN: usize = 49_152;
#[cfg(target_os = "android")]
const ANDROID_READ_EXTERNAL_STORAGE_PERMISSION: &str = "android.permission.READ_EXTERNAL_STORAGE";
#[cfg(target_os = "android")]
const ANDROID_STORAGE_PERMISSION_REQUEST_CODE: i32 = 1_001;
#[cfg(target_os = "android")]
const ANDROID_STORAGE_PERMISSION_REQUEST_THROTTLE: Duration = Duration::from_millis(500);
#[cfg(target_os = "android")]
const ANDROID_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION: &str =
    "android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION";
type AndroidEmulator =
    Box<rusty_box::emulator::Emulator<'static, rusty_box::cpu::core_i7_skylake::Corei7SkylakeX>>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum AndroidIsoSource {
    EmbeddedAlpine,
    File(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AndroidIsoBrowserEntryKind {
    Directory,
    IsoFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AndroidIsoBrowserEntry {
    name: String,
    path: PathBuf,
    kind: AndroidIsoBrowserEntryKind,
}

impl AndroidIsoBrowserEntry {
    fn directory(name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            name: name.into(),
            path,
            kind: AndroidIsoBrowserEntryKind::Directory,
        }
    }

    fn iso_file(name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            name: name.into(),
            path,
            kind: AndroidIsoBrowserEntryKind::IsoFile,
        }
    }

    fn label(&self) -> String {
        match self.kind {
            AndroidIsoBrowserEntryKind::Directory => format!("[dir] {}", self.name),
            AndroidIsoBrowserEntryKind::IsoFile => format!("[iso] {}", self.name),
        }
    }
}

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
    configure_android_window_for_safe_content(&app);
    set_android_game_mode_flags(&app);

    let app_for_safe_area = app.clone();
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };

    eframe::run_native(
        "Rusty Box",
        options,
        Box::new(move |cc| {
            Ok(Box::new(
                RustyBoxAndroidApp::new(cc).with_android_app(app_for_safe_area.clone()),
            ))
        }),
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
    screen: NativeShellApp,
    gui_command_rx: Receiver<NativeEmulatorCommand>,
    worker: Option<JoinHandle<()>>,
    total_instructions_shared: Arc<AtomicU64>,
    iso_source: AndroidIsoSource,
    initialized: bool,
    init_error: Option<String>,
    shutdown: bool,
    total_instructions: u64,
    last_ips_time: Instant,
    last_ips_instructions: u64,
    key_text: String,
    iso_path_text: String,
    iso_status: Option<String>,
    iso_browser_dir: PathBuf,
    iso_browser_entries: Vec<AndroidIsoBrowserEntry>,
    iso_browser_status: Option<String>,
    iso_browser_loaded: bool,
    show_keypad: bool,
    show_iso_picker: bool,
    #[cfg(target_os = "android")]
    storage_permission_prompted_at: Option<Instant>,
    #[cfg(target_os = "android")]
    android_app: Option<winit::platform::android::activity::AndroidApp>,
    #[cfg(target_os = "android")]
    android_game_ui_applied: bool,
}

fn android_gui_config() -> ResolvedConfig {
    ResolvedConfig {
        memory_mib: ANDROID_MEMORY_MIB as u32,
        host_memory_mib: ANDROID_MEMORY_MIB as u32,
        memory_block_kib: 128,
        ips: ANDROID_IPS,
        pci: true,
        sync_slowdown: false,
        max_instructions: u64::MAX,
        cpu_params: BxParams::default(),
        display: DisplayBackend::Egui,
        bios: PathBuf::from("embedded://bochs-bios"),
        vga_bios: Some(PathBuf::from("embedded://vgabios")),
        boot_order: vec![BootDevice::Cdrom],
        disk: None,
        cdrom: Some(ResolvedCdrom {
            path: PathBuf::from("embedded://alpine.iso"),
            channel: 1,
            drive: 0,
        }),
        log_level: LogLevel::Warn,
        config_path: None,
        vga_mode: None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AndroidSafeAreaWindowFlagBits {
    add: u32,
    remove: u32,
}

fn android_safe_area_window_flag_bits() -> AndroidSafeAreaWindowFlagBits {
    AndroidSafeAreaWindowFlagBits {
        add: 0x0000_0100 | 0x0000_0800 | 0x0001_0000,
        remove: 0x0000_0400 | 0x0000_0200,
    }
}

#[cfg(target_os = "android")]
fn configure_android_window_for_safe_content(app: &winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::activity::WindowManagerFlags;

    let add = WindowManagerFlags::LAYOUT_IN_SCREEN
        | WindowManagerFlags::FORCE_NOT_FULLSCREEN
        | WindowManagerFlags::LAYOUT_INSET_DECOR;
    let remove = WindowManagerFlags::FULLSCREEN | WindowManagerFlags::LAYOUT_NO_LIMITS;
    let expected = android_safe_area_window_flag_bits();
    debug_assert_eq!(add.bits(), expected.add);
    debug_assert_eq!(remove.bits(), expected.remove);
}

#[cfg(target_os = "android")]
fn set_android_game_mode_flags(app: &winit::platform::android::activity::AndroidApp) -> bool {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    if let Err(error) = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
        let activity = unsafe {
            env.as_cast_raw::<jni::objects::Global<jni::objects::JObject>>(&raw_activity)?
        };
        let window = env
            .call_method(
                activity.as_ref(),
                jni::jni_str!("getWindow"),
                jni::jni_sig!("()Landroid/view/Window;"),
                &[],
            )?
            .l()?;
        let decor_view = env
            .call_method(
                &window,
                jni::jni_str!("getDecorView"),
                jni::jni_sig!("()Landroid/view/View;"),
                &[],
            )?
            .l()?;
        let system_ui_flags: i32 =
            0x0000_0100 | 0x0000_0200 | 0x0000_0002 | 0x0000_0004 | 0x0000_1000;
        env.call_method(
            &decor_view,
            jni::jni_str!("setSystemUiVisibility"),
            jni::jni_sig!("(I)V"),
            &[jni::objects::JValue::Int(system_ui_flags)],
        )?;
        let soft_input_mode: i32 = 0x0000_0002 | 0x0000_0030;
        env.call_method(
            &window,
            jni::jni_str!("setSoftInputMode"),
            jni::jni_sig!("(I)V"),
            &[jni::objects::JValue::Int(soft_input_mode)],
        )?;
        Ok(())
    }) {
        log::warn!("Failed to enforce Android game-mode UI flags: {error}");
        false
    } else {
        true
    }
}

fn safe_content_rect_from_android_pixels(
    viewport_rect: egui::Rect,
    pixels_per_point: f32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> Option<egui::Rect> {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 || right <= left || bottom <= top {
        return None;
    }

    let rect = egui::Rect::from_min_max(
        egui::pos2(
            left as f32 / pixels_per_point,
            top as f32 / pixels_per_point,
        ),
        egui::pos2(
            right as f32 / pixels_per_point,
            bottom as f32 / pixels_per_point,
        ),
    )
    .intersect(viewport_rect);

    (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
}

impl RustyBoxAndroidApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let display = Arc::new(Mutex::new(SharedDisplay::new()));
        let (gui_command_tx, gui_command_rx) = mpsc::channel();
        let screen = NativeShellApp::new(
            cc,
            Arc::clone(&display),
            gui_command_tx,
            android_gui_config(),
        );
        Self {
            display,
            screen,
            gui_command_rx,
            worker: None,
            total_instructions_shared: Arc::new(AtomicU64::new(0)),
            iso_source: AndroidIsoSource::EmbeddedAlpine,
            initialized: false,
            init_error: None,
            shutdown: false,
            total_instructions: 0,
            last_ips_time: Instant::now(),
            last_ips_instructions: 0,
            key_text: String::new(),
            iso_path_text: String::new(),
            iso_status: None,
            iso_browser_dir: default_android_iso_browser_dir(),
            iso_browser_entries: Vec::new(),
            iso_browser_status: None,
            iso_browser_loaded: false,
            show_keypad: false,
            show_iso_picker: false,
            #[cfg(target_os = "android")]
            storage_permission_prompted_at: None,
            #[cfg(target_os = "android")]
            android_app: None,
            #[cfg(target_os = "android")]
            android_game_ui_applied: false,
        }
    }
    #[cfg(target_os = "android")]
    pub fn with_android_app(mut self, app: winit::platform::android::activity::AndroidApp) -> Self {
        self.android_app = Some(app);
        self
    }

    #[cfg(target_os = "android")]
    fn ensure_android_game_mode(&mut self) {
        if self.android_game_ui_applied {
            return;
        }
        let Some(android_app) = self.android_app.as_ref() else {
            return;
        };
        self.android_game_ui_applied = set_android_game_mode_flags(android_app);
    }

    #[cfg(target_os = "android")]
    fn request_storage_permission_if_needed(&mut self) -> bool {
        let Some(android_app) = self.android_app.as_ref() else {
            return false;
        };
        let vm = unsafe { jni::JavaVM::from_raw(android_app.vm_as_ptr().cast()) };

        let now = Instant::now();
        let should_request = match self.storage_permission_prompted_at {
            Some(last_request_at) => {
                now.duration_since(last_request_at) >= ANDROID_STORAGE_PERMISSION_REQUEST_THROTTLE
            }
            None => true,
        };
        if !should_request {
            return false;
        }

        let is_permission_granted = vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
            let version_class = env.find_class(jni::jni_str!("android/os/Build$VERSION"))?;
            let sdk = env
                .get_static_field(&version_class, jni::jni_str!("SDK_INT"), jni::jni_sig!("I"))?
                .i()?;
            if sdk >= 30 {
                let environment_class = env.find_class(jni::jni_str!("android/os/Environment"))?;
                return env
                    .call_static_method(
                        environment_class,
                        jni::jni_str!("isExternalStorageManager"),
                        jni::jni_sig!("()Z"),
                        &[],
                    )
                    .map(|value| value.z().unwrap_or(false));
            }

            let raw_activity = android_app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe {
                env.as_cast_raw::<jni::objects::Global<jni::objects::JObject>>(&raw_activity)?
            };
            let permission = env.new_string(ANDROID_READ_EXTERNAL_STORAGE_PERMISSION)?;
            let granted = env
                .call_method(
                    activity.as_ref(),
                    jni::jni_str!("checkSelfPermission"),
                    jni::jni_sig!("(Ljava/lang/String;)I"),
                    &[jni::objects::JValue::Object(&permission)],
                )?
                .i()?;
            Ok(granted == 0)
        });
        let is_permission_granted = match is_permission_granted {
            Ok(is_permission_granted) => is_permission_granted,
            Err(error) => {
                log::warn!("Failed to check Android storage permission: {error}");
                false
            }
        };
        if is_permission_granted {
            self.storage_permission_prompted_at = None;
            return true;
        }

        let request = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let raw_activity = android_app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe {
                env.as_cast_raw::<jni::objects::Global<jni::objects::JObject>>(&raw_activity)?
            };

            let version_class = env.find_class(jni::jni_str!("android/os/Build$VERSION"))?;
            let sdk = env
                .get_static_field(&version_class, jni::jni_str!("SDK_INT"), jni::jni_sig!("I"))?
                .i()?;
            if sdk >= 30 {
                let action = env.new_string(ANDROID_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)?;
                let intent_class = env.find_class(jni::jni_str!("android/content/Intent"))?;
                let intent = env.new_object(
                    &intent_class,
                    jni::jni_sig!("(Ljava/lang/String;)V"),
                    &[jni::objects::JValue::Object(&action)],
                )?;
                env.call_method(
                    activity.as_ref(),
                    jni::jni_str!("startActivity"),
                    jni::jni_sig!("(Landroid/content/Intent;)V"),
                    &[jni::objects::JValue::Object(&intent)],
                )?;
                return Ok(());
            }

            let permission = env.new_string(ANDROID_READ_EXTERNAL_STORAGE_PERMISSION)?;
            let string_class = env.find_class(jni::jni_str!("java/lang/String"))?;
            let permissions =
                env.new_object_array(1, string_class, jni::objects::JObject::null())?;
            env.set_object_array_element(&permissions, 0, &permission)?;
            env.call_method(
                activity.as_ref(),
                jni::jni_str!("requestPermissions"),
                jni::jni_sig!("([Ljava/lang/String;I)V"),
                &[
                    jni::objects::JValue::Object(&permissions),
                    jni::objects::JValue::Int(ANDROID_STORAGE_PERMISSION_REQUEST_CODE),
                ],
            )?;
            Ok(())
        });
        if let Err(error) = request {
            log::warn!("Failed to request Android storage permission: {error}");
            return false;
        }
        self.storage_permission_prompted_at = Some(now);
        false
    }

    fn safe_content_rect(&self, ctx: &egui::Context) -> egui::Rect {
        #[cfg(target_os = "android")]
        if let Some(android_app) = &self.android_app {
            let rect = android_app.content_rect();
            if let Some(rect) = safe_content_rect_from_android_pixels(
                ctx.viewport_rect(),
                ctx.pixels_per_point(),
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
            ) {
                return rect;
            }
        }

        ctx.content_rect()
    }

    fn initialize_alpine(&mut self) {
        if self.initialized {
            return;
        }

        let shared = Arc::clone(&self.display);
        let total_instructions = Arc::clone(&self.total_instructions_shared);
        let iso_source = self.iso_source.clone();
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
                if let Err(error) =
                    run_alpine_emulator_worker(shared, total_instructions, iso_source)
                {
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

    fn update_android_ips(&mut self) {
        self.total_instructions = self.total_instructions_shared.load(Ordering::Relaxed);
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_ips_time);
        if elapsed.as_secs_f64() < 0.5 {
            return;
        }

        let delta = self
            .total_instructions
            .saturating_sub(self.last_ips_instructions);
        let ips = (delta as f64 / elapsed.as_secs_f64()).min(u32::MAX as f64) as u32;
        self.last_ips_time = now;
        self.last_ips_instructions = self.total_instructions;

        if let Ok(mut display) = self.display.lock() {
            display.ips = ips;
        }
    }

    fn restart_emulator(&mut self) {
        if let Ok(display) = self.display.lock() {
            display.stop_flag.store(true, Ordering::Relaxed);
        }
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                self.initialized = false;
                self.shutdown = true;
                self.init_error =
                    Some("Android emulator worker panicked while rebooting".to_owned());
                return;
            }
        }
        self.initialized = false;
        self.shutdown = false;
        self.init_error = None;
        self.initialize_alpine();
    }

    fn reboot_from_embedded_iso(&mut self) {
        self.iso_source = AndroidIsoSource::EmbeddedAlpine;
        self.iso_status = Some("Booting embedded Alpine ISO".to_owned());
        self.restart_emulator();
    }

    fn reboot_from_custom_iso(&mut self) {
        match validate_android_iso_path(&self.iso_path_text) {
            Ok(path) => {
                self.iso_source = AndroidIsoSource::File(path.clone());
                self.iso_status = Some(format!("Booting {}", path.display()));
                self.restart_emulator();
            }
            Err(error) => {
                self.iso_status = Some(error);
            }
        }
    }

    fn refresh_iso_browser(&mut self) {
        match scan_android_iso_browser_dir(&self.iso_browser_dir) {
            Ok(entries) => {
                self.iso_browser_status = Some(format!(
                    "{} ISO/director{} in {}",
                    entries.len(),
                    if entries.len() == 1 { "y" } else { "ies" },
                    self.iso_browser_dir.display()
                ));
                self.iso_browser_entries = entries;
            }
            Err(error) => {
                self.iso_browser_entries.clear();
                #[cfg(target_os = "android")]
                let error = if error.contains("Permission denied")
                    || error.contains("EACCES")
                    || error.contains("Operation not permitted")
                {
                    format!("{error} (storage permission may be denied)")
                } else {
                    error
                };
                #[cfg(not(target_os = "android"))]
                let error = error;
                self.iso_browser_status = Some(error);
            }
        }
        self.iso_browser_loaded = true;
    }

    fn open_iso_browser_dir(&mut self, dir: PathBuf) {
        self.iso_browser_dir = dir;
        self.refresh_iso_browser();
    }

    fn select_iso_browser_file(&mut self, path: PathBuf) {
        self.iso_path_text = path.display().to_string();
        self.iso_status = Some(format!("Selected {}", path.display()));
    }

    fn drain_gui_commands(&mut self) {
        while let Ok(command) = self.gui_command_rx.try_recv() {
            match command {
                NativeEmulatorCommand::Start(_) => {
                    if self.worker.is_none() {
                        self.shutdown = false;
                        self.init_error = None;
                        self.initialize_alpine();
                    }
                }
            }
        }
    }

    fn draw_android_overlays(&mut self, ctx: &egui::Context, safe_rect: egui::Rect) {
        egui::Area::new(egui::Id::new("android_overlay_buttons"))
            .constrain_to(safe_rect)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 8.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Keys").clicked() {
                        self.show_keypad = true;
                    }
                    if ui.button("ISO").clicked() {
                        self.show_iso_picker = true;
                    }
                });
            });
        self.draw_android_keypad(ctx, safe_rect);
        self.draw_iso_picker(ctx, safe_rect);
    }

    fn draw_android_keypad(&mut self, ctx: &egui::Context, safe_rect: egui::Rect) {
        if !self.show_keypad {
            return;
        }

        let mut open = self.show_keypad;
        egui::Window::new("Android PS/2 keys")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .constrain_to(safe_rect)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.key_text)
                            .desired_width(180.0)
                            .hint_text("Type PS/2 keys"),
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
                });

                ui.horizontal_wrapped(|ui| {
                    for (label, key) in [
                        ("Esc", egui::Key::Escape),
                        ("Tab", egui::Key::Tab),
                        ("Enter", egui::Key::Enter),
                        ("Left", egui::Key::ArrowLeft),
                        ("Up", egui::Key::ArrowUp),
                        ("Down", egui::Key::ArrowDown),
                        ("Right", egui::Key::ArrowRight),
                    ] {
                        if ui.button(label).clicked() {
                            queue_touch_key(&self.display, key);
                        }
                    }
                    if ui.button("Ctrl").clicked() {
                        queue_ps2_ctrl_key(&self.display);
                    }
                });
            });
        self.show_keypad = open;
    }

    fn draw_iso_picker(&mut self, ctx: &egui::Context, safe_rect: egui::Rect) {
        if !self.show_iso_picker {
            return;
        }
        #[cfg(target_os = "android")]
        if self.request_storage_permission_if_needed() {
            if !self.iso_browser_loaded {
                self.refresh_iso_browser();
            }
        } else {
            self.iso_browser_loaded = false;
            self.iso_browser_entries.clear();
            self.iso_browser_status = Some(
                "Storage permission required to browse files. On Android 11+, open 'All files access' in settings and return."
                    .to_owned(),
            );
        }
        #[cfg(not(target_os = "android"))]
        if !self.iso_browser_loaded {
            self.refresh_iso_browser();
        }

        let mut window_open = self.show_iso_picker;
        let mut close_after_boot = false;
        let default_iso_hint = default_android_iso_browser_dir().join("alpine.iso");
        egui::Window::new("Boot ISO from Android filesystem")
            .open(&mut window_open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .constrain_to(safe_rect)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Refresh").clicked() {
                        self.refresh_iso_browser();
                    }
                    let parent_dir = self.iso_browser_dir.parent().map(Path::to_path_buf);
                    if ui
                        .add_enabled(parent_dir.is_some(), egui::Button::new("Up"))
                        .clicked()
                    {
                        if let Some(parent_dir) = parent_dir {
                            self.open_iso_browser_dir(parent_dir);
                        }
                    }
                    if ui.button("Downloads").clicked() {
                        self.open_iso_browser_dir(default_android_iso_browser_dir());
                    }
                });

                ui.label(
                    egui::RichText::new(self.iso_browser_dir.display().to_string())
                        .monospace()
                        .size(11.0)
                        .color(egui::Color32::from_rgb(0x88, 0x8B, 0x99)),
                );

                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let entries = self.iso_browser_entries.clone();
                        if entries.is_empty() {
                            ui.label("No folders or .iso files found here.");
                        }
                        for entry in entries {
                            if ui.button(entry.label()).clicked() {
                                match entry.kind {
                                    AndroidIsoBrowserEntryKind::Directory => {
                                        self.open_iso_browser_dir(entry.path);
                                    }
                                    AndroidIsoBrowserEntryKind::IsoFile => {
                                        self.select_iso_browser_file(entry.path.clone());
                                        self.reboot_from_custom_iso();
                                        close_after_boot = true;
                                    }
                                }
                            }
                        }
                    });

                if let Some(status) = &self.iso_browser_status {
                    ui.label(
                        egui::RichText::new(status.as_str())
                            .monospace()
                            .size(11.0)
                            .color(egui::Color32::from_rgb(0x88, 0x8B, 0x99)),
                    );
                }

                ui.separator();
                ui.label("Selected ISO path:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.iso_path_text)
                        .desired_width(460.0)
                        .hint_text(default_iso_hint.display().to_string()),
                );
                let boot_custom = (!self.iso_path_text.trim().is_empty()
                    && response.lost_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                    || ui
                        .add_enabled(
                            !self.iso_path_text.trim().is_empty(),
                            egui::Button::new("Boot selected ISO"),
                        )
                        .clicked();
                if boot_custom {
                    self.reboot_from_custom_iso();
                    close_after_boot = true;
                }
                if ui.button("Boot embedded Alpine").clicked() {
                    self.reboot_from_embedded_iso();
                    close_after_boot = true;
                }
                if let Some(status) = &self.iso_status {
                    ui.label(
                        egui::RichText::new(status.as_str())
                            .monospace()
                            .size(11.0)
                            .color(egui::Color32::from_rgb(0x88, 0x8B, 0x99)),
                    );
                }
            });
        self.show_iso_picker = if close_after_boot { false } else { window_open };
    }
}

impl eframe::App for RustyBoxAndroidApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(target_os = "android")]
        self.ensure_android_game_mode();
        self.sync_worker_status();
        self.drain_gui_commands();
        self.sync_worker_status();
        self.update_android_ips();
        let safe_rect = self.safe_content_rect(ui.ctx());
        let mut safe_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(safe_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        safe_ui.set_clip_rect(safe_rect);
        eframe::App::ui(&mut self.screen, &mut safe_ui, frame);
        self.draw_android_overlays(ui.ctx(), safe_rect);

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
    iso_source: AndroidIsoSource,
) -> Result<(), String> {
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
    attach_android_iso(&mut emu, iso_source)?;
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

fn attach_android_iso(emu: &mut AndroidEmulator, source: AndroidIsoSource) -> Result<(), String> {
    match source {
        AndroidIsoSource::EmbeddedAlpine => {
            let iso = embedded_alpine_iso().map_err(str::to_string)?;
            emu.attach_cdrom_data_ref(1, 0, iso);
        }
        AndroidIsoSource::File(path) => {
            let data = fs::read(&path)
                .map_err(|error| format!("failed to read ISO '{}': {error}", path.display()))?;
            if data.is_empty() {
                return Err(format!("ISO '{}' is empty", path.display()));
            }
            emu.attach_cdrom_data(1, 0, data);
        }
    }
    Ok(())
}

fn validate_android_iso_path(input: &str) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Enter an ISO file path first.".to_owned());
    }
    if trimmed.starts_with("content://") {
        return Err(
            "content:// picker URIs need an Android SAF bridge; use a readable /sdcard/... path for now."
                .to_owned(),
        );
    }

    let path = PathBuf::from(trimmed);
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("cannot read ISO '{}': {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("ISO path '{}' is not a file", path.display()));
    }
    Ok(path)
}

fn default_android_iso_browser_dir() -> PathBuf {
    android_iso_browser_dir_candidates()
        .into_iter()
        .find(|path| fs::read_dir(path).is_ok())
        .unwrap_or_else(|| PathBuf::from("/sdcard/Download"))
}

fn android_iso_browser_dir_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(external_storage) = std::env::var("EXTERNAL_STORAGE") {
        if !external_storage.trim().is_empty() {
            let external_storage = PathBuf::from(external_storage);
            candidates.push(external_storage.join("Download"));
            candidates.push(external_storage.join("Downloads"));
        }
    }
    candidates.push(PathBuf::from("/storage/emulated/0/Download"));
    candidates.push(PathBuf::from("/storage/emulated/0/Downloads"));
    candidates.push(PathBuf::from("/storage/self/primary/Download"));
    candidates.push(PathBuf::from("/storage/self/primary/Downloads"));
    candidates.push(PathBuf::from("/sdcard/Download"));
    candidates.push(PathBuf::from("/sdcard/Downloads"));
    candidates
}

fn scan_android_iso_browser_dir(dir: &Path) -> Result<Vec<AndroidIsoBrowserEntry>, String> {
    let mut entries = Vec::new();
    for entry in
        fs::read_dir(dir).map_err(|error| format!("cannot open '{}': {error}", dir.display()))?
    {
        let entry =
            entry.map_err(|error| format!("cannot read entry in '{}': {error}", dir.display()))?;
        let path = entry.path();
        let metadata = entry.metadata().ok();
        let name = entry.file_name().to_string_lossy().into_owned();
        match metadata {
            Some(metadata) if metadata.is_dir() => {
                entries.push(AndroidIsoBrowserEntry::directory(name, path));
            }
            Some(metadata) if metadata.is_file() && is_android_iso_file(&path) => {
                entries.push(AndroidIsoBrowserEntry::iso_file(name, path));
            }
            None if is_android_iso_file(&path) => {
                entries.push(AndroidIsoBrowserEntry::iso_file(name, path));
            }
            _ => {}
        }
    }
    entries.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(entries)
}

fn is_android_iso_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("iso"))
        .unwrap_or(false)
}

fn append_serial_log(log: &mut String, bytes: &[u8]) {
    log.push_str(&String::from_utf8_lossy(bytes));
    if log.len() > SERIAL_LOG_LIMIT {
        let drain = log.len() - SERIAL_LOG_RETAIN;
        log.drain(..drain);
    }
}

fn queue_touch_key(shared: &Arc<Mutex<SharedDisplay>>, key: egui::Key) {
    queue_raw_scancodes(shared, &egui_key_to_scancodes(key, true));
    queue_raw_scancodes(shared, &egui_key_to_scancodes(key, false));
}

fn queue_ps2_ctrl_key(shared: &Arc<Mutex<SharedDisplay>>) {
    // PS/2 left control: make (0x14), break (0xF0 0x14)
    queue_raw_scancodes(shared, &[0x14, 0xF0, 0x14]);
}

fn queue_raw_scancodes(shared: &Arc<Mutex<SharedDisplay>>, codes: &[u8]) {
    if codes.is_empty() {
        return;
    }
    if let Ok(mut display) = shared.lock() {
        display.pending_scancodes.extend_from_slice(codes);
    }
}

fn queue_text_keys(shared: &Arc<Mutex<SharedDisplay>>, text: &str) {
    let mut scancodes = Vec::new();
    for ch in text.chars() {
        scancodes.extend_from_slice(&rusty_box::gui::char_to_scancode_sequence(ch));
    }
    queue_raw_scancodes(shared, &scancodes);
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
    fn android_safe_content_rect_converts_platform_pixels_to_egui_points() {
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 548.0));

        let safe_rect = safe_content_rect_from_android_pixels(viewport, 2.0, 0, 48, 2496, 1096)
            .expect("valid content rect");

        assert_eq!(safe_rect.min, egui::pos2(0.0, 24.0));
        assert_eq!(safe_rect.max, egui::pos2(1248.0, 548.0));
    }

    #[test]
    fn android_safe_area_window_flags_request_decorated_content_rect() {
        let flags = android_safe_area_window_flag_bits();

        assert_eq!(flags.add, 0x0001_0900);
        assert_eq!(flags.remove, 0x0000_0600);
    }

    #[test]
    fn android_gui_config_describes_embedded_boot_media() {
        let config = android_gui_config();
        assert_eq!(config.memory_mib, ANDROID_MEMORY_MIB as u32);
        assert_eq!(config.boot_order, vec![BootDevice::Cdrom]);
        assert_eq!(
            config.cdrom.expect("embedded cdrom").path,
            PathBuf::from("embedded://alpine.iso")
        );
    }

    #[test]
    fn android_iso_path_rejects_picker_uri_without_saf_bridge() {
        let error = validate_android_iso_path("content://downloads/document/1")
            .expect_err("content URI needs SAF bridge");
        assert!(error.contains("content://"));
    }

    #[test]
    fn android_iso_path_accepts_regular_file_path() {
        let path = std::env::temp_dir().join(format!(
            "rusty_box_android_iso_path_{}.iso",
            std::process::id()
        ));
        fs::write(&path, b"iso").expect("write temp iso");

        assert_eq!(
            validate_android_iso_path(path.to_str().expect("utf-8 temp path")).expect("valid iso"),
            path
        );

        fs::remove_file(&path).expect("remove temp iso");
    }

    #[test]
    fn android_iso_browser_lists_directories_and_iso_files() {
        let root = std::env::temp_dir().join(format!(
            "rusty_box_android_iso_browser_{}",
            std::process::id()
        ));
        let nested = root.join("nested");
        let first_iso = root.join("alpine.iso");
        let second_iso = root.join("BOOT.ISO");
        let ignored = root.join("notes.txt");
        fs::create_dir_all(&nested).expect("create nested dir");
        fs::write(&first_iso, b"iso").expect("write first iso");
        fs::write(&second_iso, b"iso").expect("write second iso");
        fs::write(&ignored, b"text").expect("write ignored file");

        let entries = scan_android_iso_browser_dir(&root).expect("scan browser dir");

        assert_eq!(
            entries,
            vec![
                AndroidIsoBrowserEntry::directory("nested", nested),
                AndroidIsoBrowserEntry::iso_file("BOOT.ISO", second_iso),
                AndroidIsoBrowserEntry::iso_file("alpine.iso", first_iso),
            ]
        );

        fs::remove_dir_all(&root).expect("remove browser temp dir");
    }

    #[test]
    fn touch_key_buttons_queue_ps2_scancodes() {
        let shared = Arc::new(Mutex::new(SharedDisplay::new()));

        queue_touch_key(&shared, egui::Key::Enter);

        assert_eq!(shared.lock().unwrap().pending_scancodes, [0x5A, 0xF0, 0x5A]);
    }

    #[test]
    fn text_input_queues_existing_gui_scancodes() {
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
