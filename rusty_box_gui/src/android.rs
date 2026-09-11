//! The Android front end.
//!
//! NativeActivity loads the `rusty_box_gui_android` example's library and
//! calls its `android_main`, which passes the activity to [`main`]. From there
//! a phone runs the desktop shell — the same `NativeShellApp`, emulator thread
//! and machine start — over a configuration whose files are the ones the APK
//! carries. What a desktop gets from its host and a phone lacks is supplied
//! here: a file browser for the shell's Browse buttons, the storage permission
//! that browser needs, a key pad for a device without a keyboard, and the
//! safe area that keeps the shell clear of the system bars.

use crate::android_support::{
    content_rect_in_points, list_directory, stage_file, DirectoryEntry, EntryKind, FileFilter,
    PixelRect,
};
use crate::app::{BrowseRequest, BrowseTarget, NativeEmulatorCommand, NativeShellApp};
use crate::config::{load_toml_file, resolve_config, FileConfig, ResolvedConfig, DEFAULT_CONFIG_FILE};
use crate::{Args, DisplayBackend, RunError, RunSummary};
use egui::RichText;
use rusty_box::gui::shared_display::SharedDisplay;
use rusty_box::gui::{char_to_bx_key_sequence, HostInputEvent, HostInputSink};
use rusty_box::iodev::scancodes::BxKey;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub use winit::platform::android::activity::AndroidApp;

const BIOS_DATA: &[u8] = include_bytes!("../../cpp_orig/bochs/bochs/bios/BIOS-bochs-latest");
const VGA_BIOS_DATA: &[u8] =
    include_bytes!("../../cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin");
#[cfg(feature = "embedded-alpine")]
const ALPINE_ISO: &[u8] = include_bytes!("../assets/alpine.iso");

/// Guest memory until a saved configuration says otherwise, MiB. A phone
/// shares its RAM with everything else it runs.
const DEFAULT_MEMORY_MIB: u32 = 256;
/// The instruction rate the machine's timers assume until a saved
/// configuration says otherwise.
const DEFAULT_IPS: u32 = 300_000_000;

/// `Build.VERSION_CODES.R`: from Android 11 shared storage is opened by the
/// "All files access" grant rather than READ and WRITE_EXTERNAL_STORAGE.
const ANDROID_11: i32 = 30;
const READ_EXTERNAL_STORAGE: &str = "android.permission.READ_EXTERNAL_STORAGE";
const WRITE_EXTERNAL_STORAGE: &str = "android.permission.WRITE_EXTERNAL_STORAGE";
/// One app's "All files access" page, named by a `package:` URI.
const MANAGE_APP_ALL_FILES_ACCESS: &str = "android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION";
/// The list of apps that may hold "All files access": where the request
/// lands on a device without the per-app page.
const MANAGE_ALL_FILES_ACCESS: &str = "android.settings.MANAGE_ALL_FILES_ACCESS_PERMISSION";
/// `PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: i32 = 0;
/// The code the permission prompt answers with. Nothing listens for it: the
/// browser reads the grant again when the app regains focus.
const STORAGE_REQUEST_CODE: i32 = 1_001;

/// The keys a soft keyboard does not offer, each sent as a press and release.
const KEYPAD_KEYS: [(&str, BxKey); 8] = [
    ("Esc", BxKey::Esc),
    ("Tab", BxKey::Tab),
    ("Enter", BxKey::Enter),
    ("Backspace", BxKey::Backspace),
    ("Left", BxKey::Left),
    ("Up", BxKey::Up),
    ("Down", BxKey::Down),
    ("Right", BxKey::Right),
];

/// Runs the shell for `app`, the activity NativeActivity handed this process,
/// until the activity ends, and logs how the run finished.
pub fn main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    match run(app) {
        Ok(summary) => match summary.instructions_executed {
            Some(count) => log::info!("rusty_box_gui: executed {count} instructions"),
            None => log::info!("rusty_box_gui: the run ended without an instruction count"),
        },
        Err(error) => log::error!("rusty_box_gui: {error}"),
    }
}

fn run(app: AndroidApp) -> Result<RunSummary, RunError> {
    let storage = app.internal_data_path().ok_or(RunError::NoAppStorage)?;
    let config = phone_config(&storage)?;
    let start = crate::runner::ShellStart {
        library: crate::library::VmLibrary::open(storage.join("vms"))?,
        launch: Some(crate::runner::LaunchVm {
            name: crate::library::DEFAULT_VM_NAME.to_owned(),
            config,
        }),
    };
    crate::runner::run_android_shell(start, app)
}

/// The machine a phone powers on: what `rusty_box.toml` in the app's storage
/// says, over defaults that need nothing outside the APK — the Bochs ROMs it
/// carries, a phone-sized memory, and in a build with `embedded-alpine` the
/// Alpine ISO as a CD booted first.
fn phone_config(storage: &Path) -> Result<ResolvedConfig, RunError> {
    let config_path = storage.join(DEFAULT_CONFIG_FILE);
    let mut file = if config_path.exists() {
        load_toml_file(&config_path)?
    } else {
        FileConfig::default()
    };

    // Staged every launch, so an updated APK replaces ROMs a saved
    // configuration still points at.
    let carried = storage.join("carried");
    let bios = stage("BIOS", &carried, "BIOS-bochs-latest", BIOS_DATA)?;
    let vga_bios = stage("VGA BIOS", &carried, "VGABIOS-lgpl-latest.bin", VGA_BIOS_DATA)?;
    if file.rom.bios.is_none() {
        file.rom.bios = Some(bios);
    }
    if file.rom.vga_bios.is_none() {
        file.rom.vga_bios = Some(vga_bios);
    }
    if file.emulator.memory_mib.is_none() {
        file.emulator.memory_mib = Some(DEFAULT_MEMORY_MIB);
    }
    if file.emulator.ips.is_none() {
        file.emulator.ips = Some(DEFAULT_IPS);
    }
    file.display.backend = Some(DisplayBackend::Egui);

    #[cfg(feature = "embedded-alpine")]
    if file.cdrom.is_none() {
        let iso = stage("Alpine ISO", &carried, "alpine.iso", ALPINE_ISO)?;
        file.cdrom = Some(crate::config::CdromToml {
            path: Some(iso),
            channel: None,
            drive: None,
        });
        if file.boot.order.is_empty() {
            file.boot.order = vec![crate::BootDevice::Cdrom];
        }
    }

    resolve_config(file, &Args::default())
}

fn stage(kind: &'static str, dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, RunError> {
    stage_file(dir, name, bytes).map_err(|source| RunError::StageFile {
        kind,
        path: dir.join(name),
        source,
    })
}

/// The shell as a phone shows it: [`NativeShellApp`] inside the platform's
/// safe area, with the file browser and key pad a phone needs drawn over it.
pub(crate) struct AndroidShellApp {
    shell: NativeShellApp,
    shared: Arc<Mutex<SharedDisplay>>,
    app: AndroidApp,
    browser: Option<FileBrowser>,
    keypad: Option<Keypad>,
}

impl AndroidShellApp {
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        shared: Arc<Mutex<SharedDisplay>>,
        command_tx: Sender<NativeEmulatorCommand>,
        start: crate::runner::ShellStart,
        app: AndroidApp,
    ) -> Self {
        let shell = NativeShellApp::new(cc, Arc::clone(&shared), command_tx, start);
        Self {
            shell,
            shared,
            app,
            browser: None,
            keypad: None,
        }
    }

    /// The part of the window the system bars leave uncovered, in points; the
    /// whole content area when the platform reports nothing usable.
    fn safe_rect(&self, ctx: &egui::Context) -> egui::Rect {
        let content = self.app.content_rect();
        content_rect_in_points(
            ctx.viewport_rect(),
            ctx.pixels_per_point(),
            PixelRect {
                left: content.left,
                top: content.top,
                right: content.right,
                bottom: content.bottom,
            },
        )
        .unwrap_or_else(|| ctx.content_rect())
    }

    fn draw_keypad(&mut self, ctx: &egui::Context, safe_rect: egui::Rect) {
        egui::Area::new(egui::Id::new("android_keypad_toggle"))
            .constrain_to(safe_rect)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -36.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if ui.button("Keys").clicked() {
                    self.keypad = match self.keypad.take() {
                        Some(_) => None,
                        None => Some(Keypad {
                            text: String::new(),
                        }),
                    };
                }
            });

        let Some(keypad) = &mut self.keypad else {
            return;
        };
        let shared = &self.shared;
        let mut open = true;
        egui::Window::new("Keys")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .constrain_to(safe_rect)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut keypad.text)
                            .desired_width(200.0)
                            .hint_text("Text to type"),
                    );
                    let submitted =
                        response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    let pressed = ui
                        .add_enabled(!keypad.text.is_empty(), egui::Button::new("Send"))
                        .clicked();
                    if (submitted || pressed) && !keypad.text.is_empty() {
                        let keys: Vec<(BxKey, bool)> =
                            keypad.text.chars().flat_map(char_to_bx_key_sequence).collect();
                        send_keys(shared, &keys);
                        keypad.text.clear();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (label, key) in KEYPAD_KEYS {
                        if ui.button(label).clicked() {
                            send_keys(shared, &chord(&[], key));
                        }
                    }
                    if ui.button("Ctrl+C").clicked() {
                        send_keys(shared, &chord(&[BxKey::CtrlL], BxKey::C));
                    }
                    if ui.button("Ctrl+Alt+Del").clicked() {
                        send_keys(shared, &chord(&[BxKey::CtrlL, BxKey::AltL], BxKey::Delete));
                    }
                });
            });
        if !open {
            self.keypad = None;
        }
    }
}

impl eframe::App for AndroidShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let safe_rect = self.safe_rect(&ctx);
        let mut safe_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(safe_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        safe_ui.set_clip_rect(safe_rect);
        eframe::App::ui(&mut self.shell, &mut safe_ui, frame);

        if let Some(request) = self.shell.take_browse_request() {
            self.browser = Some(FileBrowser::open(request, &self.app));
        }
        if let Some(browser) = &mut self.browser {
            match browser.show(&ctx, safe_rect, &self.app) {
                BrowserOutcome::Browsing => {}
                BrowserOutcome::Chosen { target, path } => {
                    self.shell.apply_browsed_path(target, path);
                    self.browser = None;
                }
                BrowserOutcome::Dismissed => self.browser = None,
            }
        }
        self.draw_keypad(&ctx, safe_rect);
    }
}

/// Typed text waiting to be sent from the key pad.
struct Keypad {
    text: String,
}

/// `key` pressed while `modifiers` are held: every press in order, then every
/// release in reverse, so the guest never sees a modifier let go early.
fn chord(modifiers: &[BxKey], key: BxKey) -> Vec<(BxKey, bool)> {
    let mut keys: Vec<(BxKey, bool)> = modifiers.iter().map(|&modifier| (modifier, true)).collect();
    keys.push((key, true));
    keys.push((key, false));
    keys.extend(modifiers.iter().rev().map(|&modifier| (modifier, false)));
    keys
}

/// Queues `keys` for the guest in order, through the same queue the console's
/// keyboard feeds. The queue grows on the host side and takes every event; a
/// refusal would stop the sequence where it stands rather than leave the
/// guest a press with no release after it.
fn send_keys(shared: &Mutex<SharedDisplay>, keys: &[(BxKey, bool)]) {
    let Ok(mut display) = shared.lock() else {
        log::error!("the shared display is poisoned; key pad input dropped");
        return;
    };
    for &(key, pressed) in keys {
        if !display.push(HostInputEvent::Key(key, pressed)) {
            log::warn!("the guest key queue refused {key:?}; the rest of the sequence was dropped");
            return;
        }
    }
}

/// Whether the app can read all of shared storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorageAccess {
    Granted,
    /// Only the app's own files and whatever scoped storage shows are listed.
    Missing,
}

/// What the browser shows for its directory.
enum Listing {
    Entries(Vec<DirectoryEntry>),
    Unreadable(String),
}

impl Listing {
    fn of(dir: &Path, filter: FileFilter) -> Self {
        match list_directory(dir, filter) {
            Ok(entries) => Self::Entries(entries),
            Err(error) => Self::Unreadable(format!("cannot open {}: {error}", dir.display())),
        }
    }
}

enum BrowserOutcome {
    Browsing,
    Chosen { target: BrowseTarget, path: PathBuf },
    Dismissed,
}

/// The file chooser a phone draws over the shell to answer a Browse press.
struct FileBrowser {
    target: BrowseTarget,
    filter: FileFilter,
    /// The name a file still to be created gets; `None` when an existing file
    /// is chosen.
    file_name: Option<String>,
    dir: PathBuf,
    listing: Listing,
    access: StorageAccess,
    typed_path: String,
    /// Whether the app had focus last frame. Regaining it — back from the
    /// settings page the access request opened — reads the grant again.
    focused: bool,
}

impl FileBrowser {
    /// Opens beside the path the field holds, or in the shared Download folder
    /// when that names no directory, and asks for storage access if the app
    /// lacks it.
    fn open(request: BrowseRequest, app: &AndroidApp) -> Self {
        let filter = match request.target {
            BrowseTarget::Cdrom => FileFilter::Extension("iso"),
            BrowseTarget::HardDisk
            | BrowseTarget::Bios
            | BrowseTarget::VgaBios
            | BrowseTarget::NewImage => FileFilter::Any,
        };
        let dir = request
            .current
            .parent()
            .filter(|parent| parent.is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_else(downloads_dir);
        let file_name = request.save_name.map(|offered| {
            request
                .current
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(offered)
                .to_owned()
        });
        let access = storage_access(app);
        if access == StorageAccess::Missing {
            request_storage_access(app);
        }
        let listing = Listing::of(&dir, filter);
        Self {
            target: request.target,
            filter,
            file_name,
            dir,
            listing,
            access,
            typed_path: request.current.display().to_string(),
            focused: true,
        }
    }

    fn open_dir(&mut self, dir: PathBuf) {
        self.listing = Listing::of(&dir, self.filter);
        self.dir = dir;
    }

    fn refresh(&mut self, app: &AndroidApp) {
        self.access = storage_access(app);
        self.listing = Listing::of(&self.dir, self.filter);
    }

    fn show(&mut self, ctx: &egui::Context, safe_rect: egui::Rect, app: &AndroidApp) -> BrowserOutcome {
        let focused = ctx.input(|input| input.focused);
        if focused && !self.focused {
            self.refresh(app);
        }
        self.focused = focused;

        let mut outcome = BrowserOutcome::Browsing;
        let mut open = true;
        let title = if self.file_name.is_some() {
            "Save disk image"
        } else {
            "Choose a file"
        };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .constrain_to(safe_rect)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let parent = self.dir.parent().map(Path::to_path_buf);
                    if ui
                        .add_enabled(parent.is_some(), egui::Button::new("Up"))
                        .clicked()
                    {
                        if let Some(parent) = parent {
                            self.open_dir(parent);
                        }
                    }
                    if ui.button("Downloads").clicked() {
                        self.open_dir(downloads_dir());
                    }
                    if ui.button("Refresh").clicked() {
                        self.refresh(app);
                    }
                });
                ui.label(
                    RichText::new(self.dir.display().to_string())
                        .monospace()
                        .size(11.0),
                );
                if self.access == StorageAccess::Missing {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Without \"All files access\" only some files are listed.");
                        if ui.button("Grant access").clicked() {
                            request_storage_access(app);
                        }
                    });
                }

                let clicked = egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| match &self.listing {
                        Listing::Unreadable(message) => {
                            ui.label(message.as_str());
                            None
                        }
                        Listing::Entries(entries) if entries.is_empty() => {
                            ui.label("Nothing to choose here.");
                            None
                        }
                        Listing::Entries(entries) => {
                            let mut clicked = None;
                            for entry in entries {
                                let label = match entry.kind {
                                    EntryKind::Directory => format!("{}/", entry.name),
                                    EntryKind::File => entry.name.clone(),
                                };
                                if ui.button(label).clicked() {
                                    clicked = Some(entry.clone());
                                }
                            }
                            clicked
                        }
                    })
                    .inner;
                if let Some(entry) = clicked {
                    match entry.kind {
                        EntryKind::Directory => self.open_dir(entry.path),
                        EntryKind::File => {
                            outcome = BrowserOutcome::Chosen {
                                target: self.target,
                                path: entry.path,
                            };
                        }
                    }
                }

                ui.separator();
                match &mut self.file_name {
                    Some(file_name) => {
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            ui.add(egui::TextEdit::singleline(file_name).desired_width(240.0));
                            let name = file_name.trim();
                            if ui
                                .add_enabled(!name.is_empty(), egui::Button::new("Save here"))
                                .clicked()
                            {
                                outcome = BrowserOutcome::Chosen {
                                    target: self.target,
                                    path: self.dir.join(name),
                                };
                            }
                        });
                    }
                    None => {
                        let mut use_typed = false;
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.typed_path)
                                    .desired_width(360.0)
                                    .hint_text("/storage/emulated/0/Download/alpine.iso"),
                            );
                            use_typed = ui
                                .add_enabled(
                                    !self.typed_path.trim().is_empty(),
                                    egui::Button::new("Use this path"),
                                )
                                .clicked();
                        });
                        if use_typed {
                            let path = PathBuf::from(self.typed_path.trim());
                            if path.is_file() {
                                outcome = BrowserOutcome::Chosen {
                                    target: self.target,
                                    path,
                                };
                            } else {
                                self.listing = Listing::Unreadable(format!(
                                    "{} is not a file this app can read",
                                    path.display()
                                ));
                            }
                        }
                    }
                }
            });
        if open {
            outcome
        } else {
            BrowserOutcome::Dismissed
        }
    }
}

/// The shared Download folder, or the shared storage root when that is not a
/// directory the app can see.
fn downloads_dir() -> PathBuf {
    let shared = std::env::var_os("EXTERNAL_STORAGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/storage/emulated/0"));
    let downloads = shared.join("Download");
    if downloads.is_dir() {
        downloads
    } else {
        shared
    }
}

/// Whether the app can read and write all of shared storage: on Android 11
/// and later the "All files access" grant, before that both READ and
/// WRITE_EXTERNAL_STORAGE. A failed check counts as missing access.
fn storage_access(app: &AndroidApp) -> StorageAccess {
    // SAFETY: android-activity hands out the process's Java VM, which lives as
    // long as the process does.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let granted = vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
        let granted = storage_access_granted(env, app);
        if granted.is_err() {
            env.exception_clear();
        }
        granted
    });
    match granted {
        Ok(true) => StorageAccess::Granted,
        Ok(false) => StorageAccess::Missing,
        Err(error) => {
            log::warn!("could not check the storage permission: {error}");
            StorageAccess::Missing
        }
    }
}

fn storage_access_granted(env: &mut jni::Env<'_>, app: &AndroidApp) -> jni::errors::Result<bool> {
    if sdk_level(env)? >= ANDROID_11 {
        let environment = env.find_class(jni::jni_str!("android/os/Environment"))?;
        return env
            .call_static_method(
                environment,
                jni::jni_str!("isExternalStorageManager"),
                jni::jni_sig!("()Z"),
                &[],
            )?
            .z();
    }
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    // SAFETY: `activity_as_ptr` is a global reference to the NativeActivity's
    // Java object, which android-activity holds for as long as `app` lives.
    let activity = unsafe {
        env.as_cast_raw::<jni::objects::Global<jni::objects::JObject>>(&raw_activity)?
    };
    for name in [READ_EXTERNAL_STORAGE, WRITE_EXTERNAL_STORAGE] {
        let permission = env.new_string(name)?;
        let state = env
            .call_method(
                activity.as_ref(),
                jni::jni_str!("checkSelfPermission"),
                jni::jni_sig!("(Ljava/lang/String;)I"),
                &[jni::objects::JValue::Object(&permission)],
            )?
            .i()?;
        if state != PERMISSION_GRANTED {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Asks for shared-storage access: on Android 11 and later by opening this
/// app's "All files access" page (the list of apps, on a device without that
/// page), before that with the prompt for READ and WRITE_EXTERNAL_STORAGE.
/// The answer arrives after the user returns, and the browser reads it again
/// when the app regains focus.
fn request_storage_access(app: &AndroidApp) {
    // SAFETY: android-activity hands out the process's Java VM, which lives as
    // long as the process does.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let requested = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let requested = ask_for_storage_access(env, app);
        if requested.is_err() {
            env.exception_clear();
        }
        requested
    });
    if let Err(error) = requested {
        log::warn!("could not ask for storage access: {error}");
    }
}

fn ask_for_storage_access(env: &mut jni::Env<'_>, app: &AndroidApp) -> jni::errors::Result<()> {
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    // SAFETY: `activity_as_ptr` is a global reference to the NativeActivity's
    // Java object, which android-activity holds for as long as `app` lives.
    let activity = unsafe {
        env.as_cast_raw::<jni::objects::Global<jni::objects::JObject>>(&raw_activity)?
    };
    if sdk_level(env)? >= ANDROID_11 {
        let intent_class = env.find_class(jni::jni_str!("android/content/Intent"))?;
        let package = env
            .call_method(
                activity.as_ref(),
                jni::jni_str!("getPackageName"),
                jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let scheme = env.new_string("package")?;
        let uri_class = env.find_class(jni::jni_str!("android/net/Uri"))?;
        let uri = env
            .call_static_method(
                uri_class,
                jni::jni_str!("fromParts"),
                jni::jni_sig!(
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;"
                ),
                &[
                    jni::objects::JValue::Object(&scheme),
                    jni::objects::JValue::Object(&package),
                    jni::objects::JValue::Object(&jni::objects::JObject::null()),
                ],
            )?
            .l()?;
        let action = env.new_string(MANAGE_APP_ALL_FILES_ACCESS)?;
        let app_page = env.new_object(
            &intent_class,
            jni::jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[
                jni::objects::JValue::Object(&action),
                jni::objects::JValue::Object(&uri),
            ],
        )?;
        match env.call_method(
            activity.as_ref(),
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[jni::objects::JValue::Object(&app_page)],
        ) {
            Ok(_) => return Ok(()),
            Err(error) => {
                env.exception_clear();
                log::info!(
                    "no per-app \"All files access\" page ({error}); opening the list of apps"
                );
            }
        }
        let action = env.new_string(MANAGE_ALL_FILES_ACCESS)?;
        let app_list = env.new_object(
            &intent_class,
            jni::jni_sig!("(Ljava/lang/String;)V"),
            &[jni::objects::JValue::Object(&action)],
        )?;
        env.call_method(
            activity.as_ref(),
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[jni::objects::JValue::Object(&app_list)],
        )?;
        return Ok(());
    }
    let string_class = env.find_class(jni::jni_str!("java/lang/String"))?;
    let permissions = env.new_object_array(2, string_class, jni::objects::JObject::null())?;
    for (index, name) in [READ_EXTERNAL_STORAGE, WRITE_EXTERNAL_STORAGE]
        .into_iter()
        .enumerate()
    {
        let permission = env.new_string(name)?;
        permissions.set_element(env, index, &permission)?;
    }
    env.call_method(
        activity.as_ref(),
        jni::jni_str!("requestPermissions"),
        jni::jni_sig!("([Ljava/lang/String;I)V"),
        &[
            jni::objects::JValue::Object(&permissions),
            jni::objects::JValue::Int(STORAGE_REQUEST_CODE),
        ],
    )?;
    Ok(())
}

/// `Build.VERSION.SDK_INT`, the API level the device runs.
fn sdk_level(env: &mut jni::Env<'_>) -> jni::errors::Result<i32> {
    let version = env.find_class(jni::jni_str!("android/os/Build$VERSION"))?;
    env.get_static_field(&version, jni::jni_str!("SDK_INT"), jni::jni_sig!("I"))?
        .i()
}
