#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::{
    atomic::Ordering,
    mpsc::Sender,
    {Arc, Mutex},
};

use egui::{Color32, RichText, Stroke};
use rusty_box_bximage::{
    calculate_hard_disk_geometry, CreatedImage as BxCreatedImage, FloppyFormat, ImageSize,
    SectorSize,
};
#[cfg(not(target_arch = "wasm32"))]
use rusty_box_bximage::{create_flat_hard_disk, create_floppy, ExistingFilePolicy};

const BG_BASE: Color32 = Color32::from_rgb(0x0B, 0x0F, 0x14);
const BG_PANEL: Color32 = Color32::from_rgb(0x11, 0x18, 0x21);
const BG_CARD: Color32 = Color32::from_rgb(0x17, 0x21, 0x2B);
const STROKE_HAIRLINE: Color32 = Color32::from_rgb(0x26, 0x34, 0x43);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE8, 0xEE, 0xF5);
const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x98, 0xA8);
const ACCENT_CYAN: Color32 = Color32::from_rgb(0x46, 0xD9, 0xC7);
const ACCENT_BLUE: Color32 = Color32::from_rgb(0x6A, 0xA8, 0xFF);
const ACCENT_AMBER: Color32 = Color32::from_rgb(0xF2, 0xB8, 0x4B);
const ACCENT_RED: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x6C);
#[cfg(target_arch = "wasm32")]
const BROWSER_MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
pub enum NativeEmulatorCommand {
    Start(crate::config::ResolvedConfig),
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum ShellNoticeKind {
    Info,
    Warning,
    Error,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellNotice {
    kind: ShellNoticeKind,
    message: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl ShellNotice {
    fn info(message: impl Into<String>) -> Self {
        Self {
            kind: ShellNoticeKind::Info,
            message: message.into(),
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            kind: ShellNoticeKind::Warning,
            message: message.into(),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            kind: ShellNoticeKind::Error,
            message: message.into(),
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn pick_native_file() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_file()
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "android"))]
fn pick_native_file() -> Option<PathBuf> {
    None
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn save_native_file(default_name: &'static str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_file_name(default_name)
        .save_file()
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "android"))]
fn save_native_file(_default_name: &'static str) -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub struct NativeShellApp {
    emulator: rusty_box::gui::RustyBoxApp,
    chrome: ShellChrome,
    disk_creator: DiskCreatorPanel,
    profiles: Vec<NativeVmProfile>,
    config: crate::config::ResolvedConfig,
    settings: NativeVmSettings,
    vm_info: NativeVmInfo,
    command_tx: Sender<NativeEmulatorCommand>,
    shared: Arc<Mutex<rusty_box::gui::shared_display::SharedDisplay>>,
    shell_notice: Option<ShellNotice>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub(crate) struct NativeVmInfo {
    pub name: String,
    pub memory_mib: u32,
    pub ips: u32,
    pub boot: String,
    pub disk: Option<PathBuf>,
    pub cdrom: Option<PathBuf>,
    pub bios: PathBuf,
    pub vga_bios: Option<PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeVmInfo {
    pub(crate) fn from_config(config: &crate::config::ResolvedConfig) -> Self {
        Self::from_config_named(config, "Rusty Box")
    }

    fn from_config_named(config: &crate::config::ResolvedConfig, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            memory_mib: config.memory_mib,
            ips: config.ips,
            boot: config
                .boot_order
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            disk: config.disk.as_ref().map(|disk| disk.path.clone()),
            cdrom: config.cdrom.as_ref().map(|cdrom| cdrom.path.clone()),
            bios: config.bios.clone(),
            vga_bios: config.vga_bios.clone(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeVmSettings {
    memory_mib: u32,
    host_memory_mib: u32,
    memory_block_kib: u32,
    ips: u32,
    boot_device: crate::args::BootDevice,
    pci: bool,
    sync_slowdown: bool,
    max_instructions: u64,
    log_level: crate::args::LogLevel,
    bios_path: String,
    vga_bios_path: String,
    disk_enabled: bool,
    disk_path: String,
    disk_channel: usize,
    disk_drive: usize,
    disk_creation: Option<crate::config::ResolvedDiskCreation>,
    cdrom_enabled: bool,
    cdrom_path: String,
    cdrom_channel: usize,
    cdrom_drive: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeVmSettings {
    fn from_config(config: &crate::config::ResolvedConfig) -> Self {
        let disk = config.disk.as_ref();
        let cdrom = config.cdrom.as_ref();
        Self {
            memory_mib: config.memory_mib,
            host_memory_mib: config.host_memory_mib,
            memory_block_kib: config.memory_block_kib,
            ips: config.ips,
            boot_device: config
                .boot_order
                .first()
                .copied()
                .unwrap_or(crate::args::BootDevice::Cdrom),
            pci: config.pci,
            sync_slowdown: config.sync_slowdown,
            max_instructions: if config.max_instructions == u64::MAX {
                0
            } else {
                config.max_instructions
            },
            log_level: config.log_level,
            bios_path: config.bios.display().to_string(),
            vga_bios_path: config
                .vga_bios
                .as_ref()
                .map_or_else(String::new, |path| path.display().to_string()),
            disk_enabled: disk.is_some(),
            disk_path: disk.map_or_else(String::new, |disk| disk.path.display().to_string()),
            disk_channel: disk.map_or(0, |disk| disk.channel),
            disk_drive: disk.map_or(0, |disk| disk.drive),
            disk_creation: disk.and_then(|disk| disk.creation.clone()),
            cdrom_enabled: cdrom.is_some(),
            cdrom_path: cdrom.map_or_else(String::new, |cdrom| cdrom.path.display().to_string()),
            cdrom_channel: cdrom.map_or(1, |cdrom| cdrom.channel),
            cdrom_drive: cdrom.map_or(0, |cdrom| cdrom.drive),
        }
    }

    fn apply_to_config(&self, config: &mut crate::config::ResolvedConfig) -> Result<(), String> {
        config.memory_mib = self.memory_mib.max(1);
        config.host_memory_mib = self.host_memory_mib.max(1);
        config.memory_block_kib = self.memory_block_kib.max(1);
        config.ips = self.ips.max(1);
        config.pci = self.pci;
        config.sync_slowdown = self.sync_slowdown;
        config.max_instructions = if self.max_instructions == 0 {
            u64::MAX
        } else {
            self.max_instructions
        };
        config.log_level = self.log_level;

        let bios_path = trimmed_optional_path(&self.bios_path)
            .ok_or_else(|| "BIOS path is required".to_owned())?;
        config.bios = bios_path;
        config.vga_bios = trimmed_optional_path(&self.vga_bios_path);

        if self.disk_enabled {
            let path = trimmed_optional_path(&self.disk_path)
                .ok_or_else(|| "Hard disk path is required when hard disk is enabled".to_owned())?;
            let creation = self
                .disk_creation
                .as_ref()
                .filter(|creation| creation.path == path)
                .cloned();
            let geometry = if let Some(creation) = &creation {
                let geometry = calculate_hard_disk_geometry(creation.size, SectorSize::Bytes512)
                    .map_err(|error| format!("Failed to inspect hard disk: {error}"))?;
                if geometry.cylinders > u16::MAX as u64 {
                    return Err(format!(
                        "Hard disk geometry exceeds BIOS limit: {} cylinders",
                        geometry.cylinders
                    ));
                }
                crate::args::DiskGeometry {
                    cylinders: geometry.cylinders as u16,
                    heads: geometry.heads as u8,
                    sectors_per_track: geometry.sectors_per_track as u8,
                }
            } else {
                crate::config::detect_disk_geometry(&path)
                    .map_err(|error| format!("Failed to inspect hard disk: {error}"))?
            };
            config.disk = Some(crate::config::ResolvedDisk {
                path,
                geometry,
                channel: self.disk_channel,
                drive: self.disk_drive,
                creation,
            });
        } else {
            config.disk = None;
        }

        if self.cdrom_enabled {
            let path = trimmed_optional_path(&self.cdrom_path)
                .ok_or_else(|| "CD/DVD path is required when CD/DVD is enabled".to_owned())?;
            config.cdrom = Some(crate::config::ResolvedCdrom {
                path,
                channel: self.cdrom_channel,
                drive: self.cdrom_drive,
            });
        } else {
            config.cdrom = None;
        }

        config.boot_order = self.boot_order_for_attached_media()?;
        Ok(())
    }

    fn boot_order_for_attached_media(&self) -> Result<Vec<crate::args::BootDevice>, String> {
        let mut order = Vec::with_capacity(2);
        let first = self.boot_device;
        if self.is_boot_device_attached(first) {
            order.push(first);
        }
        for device in [
            crate::args::BootDevice::Disk,
            crate::args::BootDevice::Cdrom,
        ] {
            if device != first && self.is_boot_device_attached(device) {
                order.push(device);
            }
        }
        if order.is_empty() {
            Err("At least one bootable hard disk or CD/DVD must be attached".to_owned())
        } else {
            Ok(order)
        }
    }

    fn is_boot_device_attached(&self, device: crate::args::BootDevice) -> bool {
        match device {
            crate::args::BootDevice::Disk => self.disk_enabled,
            crate::args::BootDevice::Cdrom => self.cdrom_enabled,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeVmProfile {
    name: String,
    config: crate::config::ResolvedConfig,
    settings: NativeVmSettings,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeVmProfile {
    fn from_config(name: impl Into<String>, config: crate::config::ResolvedConfig) -> Self {
        let settings = NativeVmSettings::from_config(&config);
        Self {
            name: name.into(),
            config,
            settings,
        }
    }

    fn duplicate(&self, name: impl Into<String>) -> Self {
        let mut copy = self.clone();
        copy.name = name.into();
        copy
    }

    fn apply_settings(&mut self) -> Result<(), String> {
        self.settings.apply_to_config(&mut self.config)
    }

    fn vm_info(&self) -> NativeVmInfo {
        NativeVmInfo::from_config_named(&self.config, &self.name)
    }

    fn library_entry(&self) -> VmLibraryEntry {
        let info = self.vm_info();
        VmLibraryEntry::new(
            &info.name,
            &info.boot,
            format!("{} MB", info.memory_mib),
            format_path_for_summary(info.disk.as_deref()),
            format_path_for_summary(info.cdrom.as_deref()),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellPage {
    Home,
    Console,
    Hardware,
    Images,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HardwareDevice {
    Memory,
    Processors,
    Devices,
    HardDisk,
    CdDvd,
    Display,
}

impl HardwareDevice {
    const ALL: [Self; 6] = [
        Self::Memory,
        Self::Processors,
        Self::Devices,
        Self::HardDisk,
        Self::CdDvd,
        Self::Display,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Memory => "Memory",
            Self::Processors => "Processors",
            Self::Devices => "Devices",
            Self::HardDisk => "Hard Disk",
            Self::CdDvd => "CD/DVD",
            Self::Display => "Display",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VmLibraryEntry {
    name: String,
    boot: String,
    memory: String,
    disk: String,
    cdrom: String,
}

impl VmLibraryEntry {
    fn new(
        name: impl Into<String>,
        boot: impl Into<String>,
        memory: impl Into<String>,
        disk: impl Into<String>,
        cdrom: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            boot: boot.into(),
            memory: memory.into(),
            disk: disk.into(),
            cdrom: cdrom.into(),
        }
    }

    fn matches_filter(&self, filter: &str) -> bool {
        filter.is_empty()
            || self.name.to_ascii_lowercase().contains(filter)
            || self.boot.to_ascii_lowercase().contains(filter)
            || self.disk.to_ascii_lowercase().contains(filter)
            || self.cdrom.to_ascii_lowercase().contains(filter)
    }
}

#[derive(Debug)]
pub(crate) struct ShellChrome {
    selected_page: ShellPage,
    selected_hardware: HardwareDevice,
    selected_vm: usize,
    vm_library: Vec<VmLibraryEntry>,
    library_filter: String,
    show_serial: bool,
    show_library: bool,
    show_about: bool,
}

impl Default for ShellChrome {
    fn default() -> Self {
        Self {
            selected_page: ShellPage::Home,
            selected_hardware: HardwareDevice::Memory,
            selected_vm: 0,
            vm_library: Vec::new(),
            library_filter: String::new(),
            show_serial: true,
            show_library: true,
            show_about: false,
        }
    }
}

impl ShellChrome {
    fn with_library(vm_library: Vec<VmLibraryEntry>) -> Self {
        Self {
            vm_library,
            ..Self::default()
        }
    }

    fn visible_vm_indices(&self) -> Vec<usize> {
        let filter = self.library_filter.trim().to_ascii_lowercase();
        self.vm_library
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.matches_filter(filter.as_str()).then_some(index))
            .collect()
    }
}

fn shell_should_draw_library(chrome: &ShellChrome) -> bool {
    chrome.show_library
}

#[cfg(test)]
fn shell_menu_labels() -> [&'static str; 4] {
    ["File", "Edit", "VM", "Help"]
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellStatus {
    pub running: bool,
    pub ips: u32,
    pub reset_requested: bool,
    pub start_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreatorKind {
    HardDisk,
    Floppy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CreatedImage {
    path: std::path::PathBuf,
    kind: CreatedImageKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreatedImageKind {
    HardDisk,
    Floppy,
}

#[derive(Debug)]
struct DiskCreatorPanel {
    kind: CreatorKind,
    path: String,
    hard_disk_size: String,
    floppy_format: FloppyFormat,
    #[cfg(not(target_arch = "wasm32"))]
    overwrite: bool,
    status: Option<CreatorStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CreatorStatus {
    Success(String),
    Error(String),
}

impl Default for DiskCreatorPanel {
    fn default() -> Self {
        Self {
            kind: CreatorKind::HardDisk,
            path: default_image_path().to_owned(),
            hard_disk_size: default_hard_disk_size().to_owned(),
            floppy_format: FloppyFormat::M1_44,
            #[cfg(not(target_arch = "wasm32"))]
            overwrite: false,
            status: None,
        }
    }
}

fn default_image_path() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "rusty-box.img"
    } else {
        "c.img"
    }
}

fn default_hard_disk_size() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "10M"
    } else {
        "20G"
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
const WEB_HARDWARE_NOTICE: &str =
    "Browser hardware can be changed before boot. Reset the VM to edit it again.";
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_BOOT_MEDIA_ACTION_LABEL: &str = "Boot OS Image";
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_BOOT_MEDIA_ACTION_DESCRIPTION: &str =
    "Upload an ISO or disk image for any supported x86 guest.";
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_MIN_MEMORY_MIB: usize = 1;
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_DEFAULT_MEMORY_MIB: usize = 128;
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_MAX_MEMORY_MIB: usize = 4096;
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_MEMORY_DRAG_SPEED_MIB: f64 = 1.0;

#[cfg(any(test, target_arch = "wasm32"))]
fn web_primary_action_label(has_vm: bool) -> &'static str {
    if has_vm {
        "Console"
    } else {
        "▶ Boot OS Image"
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_upload_replaces_browser_vm(has_existing_vm: bool) -> bool {
    has_existing_vm
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_uploaded_media_summary(name: &str, byte_len: usize) -> String {
    format!(
        "{} ({})",
        web_uploaded_media_name(name),
        web_upload_size_label(byte_len)
    )
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_uploaded_media_name(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Uploaded media"
    } else {
        trimmed
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_upload_size_label(byte_len: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = KIB * 1024;
    if byte_len >= MIB {
        if byte_len % MIB == 0 {
            format!("{} MiB", byte_len / MIB)
        } else {
            format!("{:.1} MiB", byte_len as f64 / MIB as f64)
        }
    } else if byte_len >= KIB {
        if byte_len % KIB == 0 {
            format!("{} KiB", byte_len / KIB)
        } else {
            format!("{:.1} KiB", byte_len as f64 / KIB as f64)
        }
    } else if byte_len == 1 {
        "1 byte".to_owned()
    } else {
        format!("{byte_len} bytes")
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
struct WebUploadedMedia {
    name: String,
    data: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn format_path_for_summary(path: Option<&Path>) -> String {
    path.map_or_else(|| "None".to_owned(), |path| path.display().to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn trimmed_optional_path(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "None" {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn status_snapshot(
    shared: &Arc<Mutex<rusty_box::gui::shared_display::SharedDisplay>>,
) -> ShellStatus {
    match shared.lock() {
        Ok(display) => ShellStatus {
            running: display.emu_running,
            ips: display.ips,
            reset_requested: display.reset_requested,
            start_pending: display.start_pending,
        },
        Err(_) => ShellStatus {
            running: false,
            ips: 0,
            reset_requested: false,
            start_pending: false,
        },
    }
}

fn configure_shell_style(ctx: &egui::Context) {
    use egui::{style::Selection, Theme, ThemePreference, Vec2};

    ctx.set_theme(ThemePreference::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.visuals.panel_fill = BG_BASE;
        style.visuals.window_fill = BG_PANEL;
        style.visuals.extreme_bg_color = Color32::from_rgb(0x07, 0x0A, 0x0E);
        style.visuals.hyperlink_color = ACCENT_BLUE;
        style.visuals.text_cursor.stroke.color = ACCENT_CYAN;
        style.visuals.selection = Selection {
            bg_fill: Color32::from_rgb(0x1E, 0x5F, 0x62),
            stroke: Stroke::new(1.0_f32, ACCENT_CYAN),
        };
        style.visuals.widgets.noninteractive.bg_fill = BG_PANEL;
        style.visuals.widgets.inactive.bg_fill = BG_CARD;
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x1D, 0x2A, 0x36);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(0x21, 0x35, 0x43);
        style.visuals.widgets.inactive.fg_stroke.color = TEXT_PRIMARY;
        style.visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
    });
}

fn shell_card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(16))
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeShellApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: Arc<Mutex<rusty_box::gui::shared_display::SharedDisplay>>,
        command_tx: Sender<NativeEmulatorCommand>,
        config: crate::config::ResolvedConfig,
    ) -> Self {
        configure_shell_style(&cc.egui_ctx);
        let profile = NativeVmProfile::from_config("Rusty Box", config);
        let vm_info = profile.vm_info();
        let settings = profile.settings.clone();
        let config = profile.config.clone();
        let chrome = ShellChrome::with_library(vec![profile.library_entry()]);
        Self {
            emulator: rusty_box::gui::RustyBoxApp::new(cc, Arc::clone(&shared)),
            chrome,
            disk_creator: DiskCreatorPanel::default(),
            profiles: vec![profile],
            config,
            settings,
            vm_info,
            command_tx,
            shared,
            shell_notice: None,
        }
    }

    fn runtime_status(&self) -> ShellStatus {
        status_snapshot(&self.shared)
    }

    fn is_vm_running(&self) -> bool {
        status_snapshot(&self.shared).running
    }

    fn draw_shell_notice(&mut self, ui: &mut egui::Ui) {
        let Some(notice) = self.shell_notice.clone() else {
            return;
        };
        let (label, color) = match notice.kind {
            ShellNoticeKind::Info => ("Info", ACCENT_CYAN),
            ShellNoticeKind::Warning => ("Warning", ACCENT_AMBER),
            ShellNoticeKind::Error => ("Error", ACCENT_RED),
        };

        shell_card_frame().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(label).strong().color(color));
                ui.label(RichText::new(notice.message).color(TEXT_PRIMARY));
                if ui.button("Close").clicked() {
                    self.shell_notice = None;
                }
            });
        });
        ui.add_space(12.0);
    }

    fn take_runtime_error_notice(&mut self) {
        let runtime_error = self
            .shared
            .lock()
            .ok()
            .and_then(|mut display| display.runtime_error.take());

        if let Some(message) = runtime_error {
            self.shell_notice = Some(ShellNotice::error(message));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn handle_native_dropped_files(&mut self, ctx: &egui::Context) {
        if self.chrome.selected_page != ShellPage::Images {
            return;
        }

        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        let Some(file) = dropped.first() else {
            return;
        };

        match &file.path {
            Some(path) => {
                self.disk_creator.path = path.display().to_string();
            }
            None => {
                self.disk_creator.status = Some(CreatorStatus::Error(
                    "dropped file has no host path".to_owned(),
                ));
            }
        }
    }

    fn draw_menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("vm_menu_bar")
            .exact_size(32.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let status = self.runtime_status();
                    let running = status.running;
                    let start_blocked = running || status.start_pending;
                    ui.menu_button("File", |ui| {
                        if ui.button("Open Console").clicked() {
                            self.chrome.selected_page = ShellPage::Console;
                            ui.close();
                        }
                        if ui.button("Duplicate VM Profile").clicked() {
                            self.duplicate_selected_profile();
                            ui.close();
                        }
                        if ui.button("Create Disk Image").clicked() {
                            self.chrome.selected_page = ShellPage::Images;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Quit").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.button("Clear Library Search").clicked() {
                            self.chrome.library_filter.clear();
                            ui.close();
                        }
                    });
                    ui.menu_button("VM", |ui| {
                        if ui
                            .add_enabled(!start_blocked, egui::Button::new("Power On"))
                            .clicked()
                        {
                            self.start_vm();
                            ui.close();
                        }
                        if ui
                            .add_enabled(running, egui::Button::new("Power Off"))
                            .clicked()
                        {
                            self.request_power_off();
                            ui.close();
                        }
                        if ui
                            .add_enabled(running, egui::Button::new("Restart VM"))
                            .clicked()
                        {
                            self.request_reset();
                            ui.close();
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("About Rusty Box Workstation").clicked() {
                            self.chrome.show_about = true;
                            ui.close();
                        }
                    });
                    ui.separator();
                    self.nav_button(ui, ShellPage::Home, "Home");
                    self.nav_button(ui, ShellPage::Console, "Console");
                    self.nav_button(ui, ShellPage::Hardware, "Hardware");
                    self.nav_button(ui, ShellPage::Images, "Images");
                    if ui
                        .add_enabled(!start_blocked, egui::Button::new("Power On"))
                        .clicked()
                    {
                        self.start_vm();
                    }
                });
            });
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("vm_toolbar")
            .exact_size(46.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x0D, 0x13, 0x1A))
                    .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let status = self.runtime_status();
                    let running = status.running;
                    let start_blocked = running || status.start_pending;
                    let primary = if running {
                        "▶ Console"
                    } else if status.start_pending {
                        "▶ Starting…"
                    } else {
                        "▶ Power On"
                    };
                    if ui
                        .add_enabled(running || !start_blocked, egui::Button::new(primary))
                        .clicked()
                    {
                        if running {
                            self.chrome.selected_page = ShellPage::Console;
                        } else {
                            self.start_vm();
                        }
                    }
                    if ui
                        .add_enabled(running, egui::Button::new("■ Power Off"))
                        .clicked()
                    {
                        self.request_power_off();
                    }
                    if ui
                        .add_enabled(running, egui::Button::new("↻ Restart VM"))
                        .clicked()
                    {
                        self.request_reset();
                    }
                    if ui.button("▣ Hardware").clicked() {
                        self.chrome.selected_page = ShellPage::Hardware;
                    }
                    if ui.button("＋ New Image").clicked() {
                        self.chrome.selected_page = ShellPage::Images;
                    }
                    ui.checkbox(&mut self.chrome.show_library, "Library");
                    ui.checkbox(&mut self.chrome.show_serial, "Serial");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(&self.vm_info.name)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                    });
                });
            });
    }

    fn draw_library(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("vm_library")
            .resizable(true)
            .default_size(250.0)
            .min_size(210.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
                    .inner_margin(egui::Margin::same(14)),
            )
            .show_inside(ui, |ui| {
                ui.label(
                    RichText::new("Library")
                        .size(16.0)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                if ui.button("＋ Duplicate VM Profile").clicked() {
                    self.duplicate_selected_profile();
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.chrome.library_filter)
                        .hint_text("Type here to search"),
                );
                ui.add_space(8.0);
                ui.label(RichText::new("▾ My Computer").color(TEXT_MUTED));

                let visible = self.chrome.visible_vm_indices();
                let mut delete_requested = false;
                let mut selected_index = None;
                for index in visible {
                    let mut delete_clicked = false;
                    let clicked = {
                        let entry = &self.chrome.vm_library[index];
                        let selected = ui.selectable_label(
                            self.chrome.selected_vm == index,
                            format!("  ▣ {}", entry.name),
                        );
                        if self.chrome.selected_vm == index {
                            ui.indent(format!("vm_library_metadata_{index}"), |ui| {
                                ui.label(metadata_text("Boot", &entry.boot));
                                ui.label(metadata_text("Memory", &entry.memory));
                                ui.label(metadata_text("Disk", &entry.disk));
                                ui.label(metadata_text("CD/DVD", &entry.cdrom));
                                let status = self.runtime_status();
                                let delete_enabled = !status.running
                                    && !status.start_pending
                                    && self.profiles.len() > 1;
                                delete_clicked = ui
                                    .add_enabled(
                                        delete_enabled,
                                        egui::Button::new("Delete Profile"),
                                    )
                                    .clicked();
                            });
                        }
                        selected.clicked()
                    };
                    if delete_clicked {
                        delete_requested = true;
                    } else if clicked {
                        selected_index = Some(index);
                    }
                }
                if delete_requested {
                    self.delete_selected_profile();
                } else if let Some(index) = selected_index {
                    self.select_profile(index);
                }
                if self.chrome.vm_library.is_empty() {
                    ui.label(RichText::new("No VM sessions registered").color(TEXT_MUTED));
                }
            });
    }

    fn draw_status_strip(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("vm_status_strip")
            .exact_size(30.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
                    .inner_margin(egui::Margin::symmetric(14, 4)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if self.shared.is_poisoned() {
                        status_dot(ui, ACCENT_RED);
                        ui.label(
                            RichText::new("State unavailable")
                                .monospace()
                                .size(11.0)
                                .color(ACCENT_RED),
                        );
                        return;
                    }

                    let snapshot = self.runtime_status();
                    let state = if snapshot.running {
                        "Running"
                    } else if snapshot.start_pending {
                        "Starting"
                    } else {
                        "Stopped"
                    };
                    let state_color = if snapshot.running {
                        ACCENT_CYAN
                    } else if snapshot.start_pending {
                        ACCENT_AMBER
                    } else {
                        TEXT_MUTED
                    };
                    status_dot(ui, state_color);
                    ui.label(
                        RichText::new(state)
                            .monospace()
                            .size(11.0)
                            .color(state_color),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format_ips_u32(snapshot.ips))
                            .monospace()
                            .size(11.0)
                            .color(ACCENT_BLUE),
                    );
                    ui.separator();
                    let reset = if snapshot.reset_requested {
                        "Restart queued"
                    } else {
                        "Ready"
                    };
                    ui.label(
                        RichText::new(reset)
                            .monospace()
                            .size(11.0)
                            .color(TEXT_MUTED),
                    );
                });
            });
    }

    fn draw_central(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.take_runtime_error_notice();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_BASE))
            .show_inside(ui, |ui| match self.chrome.selected_page {
                ShellPage::Home => self.draw_home_page(ui),
                ShellPage::Console => self.draw_console_page(ui, frame),
                ShellPage::Hardware => self.draw_hardware_page(ui),
                ShellPage::Images => self.draw_images_page(ui),
            });
    }

    fn draw_home_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.draw_shell_notice(ui);
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("RUSTY BOX WORKSTATION")
                        .size(26.0)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.label(
                    RichText::new("Graphite VM library for x86 experiments").color(TEXT_MUTED),
                );
            });
            ui.add_space(24.0);
            shell_card_frame().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Selected VM").strong().color(TEXT_PRIMARY));
                    let mut name_changed = false;
                    if let Some(profile) = self.profiles.get_mut(self.chrome.selected_vm) {
                        name_changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut profile.name)
                                    .desired_width(220.0),
                            )
                            .changed();
                    }
                    if ui.button("Duplicate VM Profile").clicked() {
                        self.duplicate_selected_profile();
                    }
                    let status = self.runtime_status();
                    let delete_enabled =
                        !status.running && !status.start_pending && self.profiles.len() > 1;
                    if ui
                        .add_enabled(delete_enabled, egui::Button::new("Delete Profile"))
                        .clicked()
                    {
                        self.delete_selected_profile();
                    }
                    if ui.button("Hardware Settings").clicked() {
                        self.chrome.selected_page = ShellPage::Hardware;
                    }
                    if name_changed {
                        if let Err(message) = self.apply_pending_settings() {
                            self.shell_notice = Some(ShellNotice::error(message));
                        }
                    }
                });
                ui.label(
                    RichText::new(
                        "Profiles are independent launch configurations. Power on starts only the selected VM.",
                    )
                    .color(TEXT_MUTED),
                );
            });
            ui.add_space(16.0);
            let status = self.runtime_status();
            let start_enabled = !status.running && !status.start_pending;
            ui.columns(3, |columns| {
                action_tile_enabled(
                    &mut columns[0],
                    "Power On VM",
                    "Start this VM with the settings selected below.",
                    ACCENT_CYAN,
                    start_enabled,
                    || self.start_vm(),
                );
                action_tile(
                    &mut columns[1],
                    "Create Disk Image",
                    "Build bximage-compatible hard disks and floppies.",
                    ACCENT_BLUE,
                    || self.chrome.selected_page = ShellPage::Images,
                );
                action_tile(
                    &mut columns[2],
                    "Hardware Settings",
                    "Inspect boot media and VM hardware limits.",
                    ACCENT_AMBER,
                    || self.chrome.selected_page = ShellPage::Hardware,
                );
            });
        });
    }

    fn draw_console_page(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.draw_shell_notice(ui);
        self.emulator
            .ui_embedded_with_serial(ui, frame, self.chrome.show_serial);
    }

    fn draw_hardware_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.draw_shell_notice(ui);
            ui.horizontal(|ui| {
                shell_card_frame().show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(190.0);
                        ui.label(RichText::new("Devices").strong().color(TEXT_PRIMARY));
                        ui.add_space(8.0);
                        for device in HardwareDevice::ALL {
                            if ui
                                .selectable_label(
                                    self.chrome.selected_hardware == device,
                                    device.label(),
                                )
                                .clicked()
                            {
                                self.chrome.selected_hardware = device;
                            }
                        }
                    });
                });
                shell_card_frame().show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(520.0);
                        ui.label(
                            RichText::new(format!(
                                "Hardware Summary  |  {}",
                                self.chrome.selected_hardware.label()
                            ))
                            .size(18.0)
                            .strong(),
                        );
                        ui.separator();
                        self.draw_hardware_detail(ui);
                    });
                });
            });
        });
    }

    fn draw_hardware_detail(&mut self, ui: &mut egui::Ui) {
        let status = self.runtime_status();
        let editable = !status.running && !status.start_pending;
        let mut changed = false;

        match self.chrome.selected_hardware {
            HardwareDevice::Memory => {
                hardware_intro(
                    ui,
                    "Memory",
                    "Edit guest memory, host memory, and allocation block size before power-on.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Guest memory").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.memory_mib)
                                    .range(1..=4096)
                                    .suffix(" MB"),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Host memory").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.host_memory_mib)
                                    .range(1..=4096)
                                    .suffix(" MB"),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Memory block").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.memory_block_kib)
                                    .range(1..=65_536)
                                    .suffix(" KiB"),
                            )
                            .changed();
                    });
                });
            }
            HardwareDevice::Processors => {
                hardware_intro(
                    ui,
                    "Virtual CPU",
                    "The IPS target controls pacing. Max instructions of 0 means unlimited.",
                );
                detail_row(ui, "Virtual processors", "1");
                ui.add_enabled_ui(editable, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("IPS target").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.ips)
                                    .range(1..=2_000_000_000)
                                    .speed(1_000_000.0),
                            )
                            .changed();
                    });
                    changed |= ui
                        .checkbox(&mut self.settings.sync_slowdown, "Sync slowdown")
                        .changed();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Max instructions")
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                        changed |= ui
                            .add(egui::DragValue::new(&mut self.settings.max_instructions))
                            .changed();
                    });
                });
                detail_row(ui, "Applied target", &format_ips_u32(self.vm_info.ips));
                detail_row(
                    ui,
                    "Restart behavior",
                    "toolbar restart stops and relaunches the VM",
                );
            }
            HardwareDevice::Devices => {
                hardware_intro(
                    ui,
                    "Devices",
                    "PCI and boot order apply at the next Power On.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    changed |= ui.checkbox(&mut self.settings.pci, "Enable PCI").changed();
                    egui::ComboBox::from_label("Primary boot")
                        .selected_text(self.settings.boot_device.to_string())
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut self.settings.boot_device,
                                    crate::args::BootDevice::Disk,
                                    "disk",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut self.settings.boot_device,
                                    crate::args::BootDevice::Cdrom,
                                    "cdrom",
                                )
                                .changed();
                        });
                });
                detail_row(ui, "Boot order", &self.vm_info.boot);
            }
            HardwareDevice::HardDisk => {
                hardware_intro(
                    ui,
                    "Hard disk",
                    "Attach or detach hard disk media for the next launch.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    changed |= ui
                        .checkbox(&mut self.settings.disk_enabled, "Enable hard disk")
                        .changed();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Disk path").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.disk_path)
                                    .desired_width(360.0),
                            )
                            .changed();
                        if ui.button("Browse").clicked() {
                            if let Some(path) = pick_native_file() {
                                self.settings.disk_path = path.display().to_string();
                                self.settings.disk_enabled = true;
                                changed = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("ATA channel").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(egui::DragValue::new(&mut self.settings.disk_channel).range(0..=1))
                            .changed();
                        ui.label(RichText::new("drive").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(egui::DragValue::new(&mut self.settings.disk_drive).range(0..=1))
                            .changed();
                    });
                });
                if let Some(disk) = &self.config.disk {
                    detail_row(ui, "Detected CHS", &disk.geometry.to_string());
                    detail_row(
                        ui,
                        "Controller",
                        &format!("ATA {}:{}", disk.channel, disk.drive),
                    );
                } else {
                    detail_row(ui, "Attached disk", "None");
                }
                if ui.button("Create disk image").clicked() {
                    self.chrome.selected_page = ShellPage::Images;
                }
            }
            HardwareDevice::CdDvd => {
                hardware_intro(
                    ui,
                    "CD/DVD",
                    "Attach or detach ISO media and optionally boot it first.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    changed |= ui
                        .checkbox(&mut self.settings.cdrom_enabled, "Enable CD/DVD")
                        .changed();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("ISO path").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.cdrom_path)
                                    .desired_width(360.0),
                            )
                            .changed();
                        if ui.button("Browse").clicked() {
                            if let Some(path) = pick_native_file() {
                                self.settings.cdrom_path = path.display().to_string();
                                self.settings.cdrom_enabled = true;
                                changed = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("ATA channel").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.cdrom_channel).range(0..=1),
                            )
                            .changed();
                        ui.label(RichText::new("drive").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(egui::DragValue::new(&mut self.settings.cdrom_drive).range(0..=1))
                            .changed();
                    });
                    let mut boot_cdrom =
                        self.settings.boot_device == crate::args::BootDevice::Cdrom;
                    if ui.checkbox(&mut boot_cdrom, "Boot CD/DVD first").changed() {
                        self.settings.boot_device = if boot_cdrom {
                            crate::args::BootDevice::Cdrom
                        } else {
                            crate::args::BootDevice::Disk
                        };
                        changed = true;
                    }
                });
                if let Some(cdrom) = &self.config.cdrom {
                    detail_row(
                        ui,
                        "Controller",
                        &format!("ATA {}:{}", cdrom.channel, cdrom.drive),
                    );
                } else {
                    detail_row(ui, "Attached CD/DVD", "None");
                }
            }
            HardwareDevice::Display => {
                hardware_intro(
                    ui,
                    "Display and ROMs",
                    "BIOS, VGA BIOS, and logging are applied on the next Power On.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("BIOS path").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.bios_path)
                                    .desired_width(360.0),
                            )
                            .changed();
                        if ui.button("Browse").clicked() {
                            if let Some(path) = pick_native_file() {
                                self.settings.bios_path = path.display().to_string();
                                changed = true;
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("VGA BIOS path").strong().color(TEXT_PRIMARY));
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.vga_bios_path)
                                    .desired_width(360.0),
                            )
                            .changed();
                        if ui.button("Browse").clicked() {
                            if let Some(path) = pick_native_file() {
                                self.settings.vga_bios_path = path.display().to_string();
                                changed = true;
                            }
                        }
                    });
                    egui::ComboBox::from_label("Log level")
                        .selected_text(format!("{:?}", self.settings.log_level))
                        .show_ui(ui, |ui| {
                            for (level, label) in [
                                (crate::args::LogLevel::Trace, "trace"),
                                (crate::args::LogLevel::Debug, "debug"),
                                (crate::args::LogLevel::Info, "info"),
                                (crate::args::LogLevel::Warn, "warn"),
                                (crate::args::LogLevel::Error, "error"),
                            ] {
                                changed |= ui
                                    .selectable_value(&mut self.settings.log_level, level, label)
                                    .changed();
                            }
                        });
                });
                detail_row(ui, "Adapter", "VGA text/graphics framebuffer");
                detail_row(ui, "Applied BIOS", &self.vm_info.bios.display().to_string());
                detail_row(
                    ui,
                    "Applied VGA BIOS",
                    &format_path_for_summary(self.vm_info.vga_bios.as_deref()),
                );
                if ui.button("Open console").clicked() {
                    self.chrome.selected_page = ShellPage::Console;
                }
            }
        }

        if changed {
            if let Err(message) = self.apply_pending_settings() {
                self.shell_notice = Some(ShellNotice::error(message));
            }
        }

        if !editable {
            ui.add_space(8.0);
            ui.label(RichText::new("Power off before changing VM hardware.").color(ACCENT_AMBER));
        }
    }
    fn draw_images_page(&mut self, ui: &mut egui::Ui) {
        self.draw_shell_notice(ui);
        if let Some(created) = self.disk_creator.ui_page(ui) {
            self.handle_created_image(created);
        }
    }

    fn handle_created_image(&mut self, created: CreatedImage) {
        match created.kind {
            CreatedImageKind::HardDisk => {
                let status = self.runtime_status();
                if status.running || status.start_pending {
                    self.shell_notice = Some(ShellNotice::warning(
                        "Disk image created. Stop the VM before attaching it.",
                    ));
                    return;
                }
                self.attach_created_image_to_selected_profile(created.path);
            }
            CreatedImageKind::Floppy => {
                self.shell_notice = Some(ShellNotice::info(
                    "Floppy image created. Floppy drive emulation is not wired yet.",
                ));
            }
        }
    }

    fn attach_created_image_to_selected_profile(&mut self, path: std::path::PathBuf) {
        self.settings.disk_enabled = true;
        self.settings.disk_path = path.display().to_string();
        self.settings.disk_creation = None;
        match self.apply_pending_settings() {
            Ok(()) => {
                self.shell_notice = Some(ShellNotice::info(format!(
                    "Attached created disk image to {}.",
                    self.vm_info.name
                )));
            }
            Err(message) => {
                self.shell_notice = Some(ShellNotice::error(message));
            }
        }
    }

    fn nav_button(&mut self, ui: &mut egui::Ui, page: ShellPage, label: &str) {
        if ui
            .selectable_label(self.chrome.selected_page == page, label)
            .clicked()
        {
            self.chrome.selected_page = page;
        }
    }

    fn apply_pending_settings(&mut self) -> Result<(), String> {
        self.settings.apply_to_config(&mut self.config)?;
        if let Some(profile) = self.profiles.get_mut(self.chrome.selected_vm) {
            profile.config.clone_from(&self.config);
            profile.settings.clone_from(&self.settings);
            self.refresh_selected_profile_metadata()?;
            self.config
                .clone_from(&self.profiles[self.chrome.selected_vm].config);
        } else {
            self.vm_info = NativeVmInfo::from_config(&self.config);
        }
        Ok(())
    }

    fn refresh_selected_profile_metadata(&mut self) -> Result<(), String> {
        let index = self.chrome.selected_vm;
        if index >= self.profiles.len() {
            return Ok(());
        }
        self.profiles[index].apply_settings()?;
        if index < self.chrome.vm_library.len() {
            self.chrome.vm_library[index] = self.profiles[index].library_entry();
        } else {
            self.chrome.vm_library = self
                .profiles
                .iter()
                .map(NativeVmProfile::library_entry)
                .collect();
        }
        self.vm_info = self.profiles[index].vm_info();
        Ok(())
    }

    fn select_profile(&mut self, index: usize) {
        if index >= self.profiles.len() {
            return;
        }
        let status = self.runtime_status();
        if status.running || status.start_pending {
            self.shell_notice = Some(ShellNotice::warning(
                "Stop the running VM before selecting another profile.",
            ));
            return;
        }
        if let Err(message) = self.apply_pending_settings() {
            self.shell_notice = Some(ShellNotice::error(message));
            return;
        }
        self.chrome.selected_vm = index;
        let profile = &self.profiles[index];
        self.config = profile.config.clone();
        self.settings = profile.settings.clone();
        if let Err(message) = self.refresh_selected_profile_metadata() {
            self.shell_notice = Some(ShellNotice::error(message));
            return;
        }
        self.chrome.selected_page = ShellPage::Home;
    }

    fn duplicate_selected_profile(&mut self) {
        if self.profiles.is_empty() {
            return;
        }
        if let Err(message) = self.apply_pending_settings() {
            self.shell_notice = Some(ShellNotice::error(message));
            return;
        }
        let base = self.chrome.selected_vm.min(self.profiles.len() - 1);
        let name = format!("{} Copy {}", self.profiles[base].name, self.profiles.len());
        let profile = self.profiles[base].duplicate(name);
        self.profiles.push(profile);
        self.chrome.vm_library.push(
            self.profiles
                .last()
                .expect("profile was just pushed")
                .library_entry(),
        );
        self.select_profile(self.profiles.len() - 1);
    }

    fn delete_selected_profile(&mut self) {
        let status = self.runtime_status();
        if status.running || status.start_pending {
            self.shell_notice = Some(ShellNotice::warning(
                "Stop the running VM before deleting profiles.",
            ));
            return;
        }
        if self.profiles.len() == 1 {
            self.shell_notice = Some(ShellNotice::warning("At least one VM profile is required."));
            return;
        }
        if let Err(message) = self.apply_pending_settings() {
            self.shell_notice = Some(ShellNotice::error(message));
            return;
        }

        let index = self.chrome.selected_vm.min(self.profiles.len() - 1);
        self.profiles.remove(index);
        if index < self.chrome.vm_library.len() {
            self.chrome.vm_library.remove(index);
        }
        self.chrome.selected_vm = index.min(self.profiles.len() - 1);
        let profile = &self.profiles[self.chrome.selected_vm];
        self.config = profile.config.clone();
        self.settings = profile.settings.clone();
        if let Err(message) = self.refresh_selected_profile_metadata() {
            self.shell_notice = Some(ShellNotice::error(message));
        }
    }

    fn start_vm(&mut self) {
        let snapshot = self.runtime_status();
        if snapshot.running {
            self.chrome.selected_page = ShellPage::Console;
            return;
        }
        if snapshot.start_pending {
            return;
        }

        if let Err(message) = self.apply_pending_settings() {
            self.shell_notice = Some(ShellNotice::error(message));
            return;
        }
        if let Ok(mut display) = self.shared.lock() {
            display.start_pending = true;
        }
        match self
            .command_tx
            .send(NativeEmulatorCommand::Start(self.config.clone()))
        {
            Ok(()) => {
                self.chrome.selected_page = ShellPage::Console;
            }
            Err(_) => {
                if let Ok(mut display) = self.shared.lock() {
                    display.start_pending = false;
                }
                self.shell_notice = Some(ShellNotice::error(
                    "Emulator worker is not available. Restart the application.",
                ));
            }
        }
    }
    fn request_power_off(&mut self) {
        if !self.is_vm_running() {
            return;
        }

        if let Ok(mut display) = self.shared.lock() {
            display.stop_flag.store(true, Ordering::Relaxed);
            display.reset_requested = false;
        }
    }

    fn request_reset(&mut self) {
        if !self.is_vm_running() {
            return;
        }

        if let Ok(mut display) = self.shared.lock() {
            display.stop_flag.store(true, Ordering::Relaxed);
            display.reset_requested = true;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl eframe::App for NativeShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.handle_native_dropped_files(ui.ctx());
        self.draw_menu_bar(ui);
        self.draw_toolbar(ui);
        if shell_should_draw_library(&self.chrome) {
            self.draw_library(ui);
        }
        self.draw_status_strip(ui);
        self.draw_central(ui, frame);
        draw_about_window(ui.ctx(), &mut self.chrome);
    }
}

impl DiskCreatorPanel {
    #[cfg(feature = "gui-egui")]
    fn ui_page(&mut self, ui: &mut egui::Ui) -> Option<CreatedImage> {
        let mut created_image = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(12.0);
            shell_card_frame().show(ui, |ui| {
                ui.label(
                    RichText::new("Disk Images")
                        .size(22.0)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.label(
                    RichText::new(
                        "Create flat hard disks and floppy images using the bximage backend.",
                    )
                    .color(TEXT_MUTED),
                );
            });
            ui.add_space(12.0);

            shell_card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.kind, CreatorKind::HardDisk, "Hard Disk");
                    ui.selectable_value(&mut self.kind, CreatorKind::Floppy, "Floppy");
                });
                ui.separator();

                #[cfg(not(target_arch = "wasm32"))]
                ui.horizontal(|ui| {
                    ui.label("Path");
                    ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(360.0));
                    if ui.button("Browse...").clicked() {
                        self.choose_native_image_path();
                    }
                });

                #[cfg(target_arch = "wasm32")]
                ui.horizontal(|ui| {
                    ui.label("Filename");
                    ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(300.0));
                });

                match self.kind {
                    CreatorKind::HardDisk => {
                        ui.horizontal(|ui| {
                            ui.label("Size");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.hard_disk_size)
                                    .hint_text("20G")
                                    .desired_width(120.0),
                            );
                            ui.label(
                                RichText::new("Examples: 10M, 512M, 20G, 512").color(TEXT_MUTED),
                            );
                        });
                    }
                    CreatorKind::Floppy => {
                        egui::ComboBox::from_label("Floppy format")
                            .selected_text(self.floppy_format.friendly_label())
                            .show_ui(ui, |ui| {
                                for format in FloppyFormat::ALL {
                                    ui.selectable_value(
                                        &mut self.floppy_format,
                                        format,
                                        format.friendly_label(),
                                    );
                                }
                            });
                    }
                }

                #[cfg(not(target_arch = "wasm32"))]
                ui.checkbox(&mut self.overwrite, "Overwrite existing file");

                let action = if cfg!(target_arch = "wasm32") {
                    "Download image"
                } else {
                    "Create image"
                };
                if ui.button(action).clicked() {
                    created_image = self.create_image();
                }

                if let Some(status) = &self.status {
                    match status {
                        CreatorStatus::Success(message) => {
                            ui.colored_label(ACCENT_CYAN, message);
                        }
                        CreatorStatus::Error(message) => {
                            ui.colored_label(ACCENT_RED, message);
                        }
                    }
                }
            });
        });
        created_image
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn default_image_filename(&self) -> &'static str {
        match self.kind {
            CreatorKind::HardDisk => "c.img",
            CreatorKind::Floppy => "floppy.img",
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn choose_native_image_path(&mut self) {
        if let Some(path) = save_native_file(self.default_image_filename()) {
            self.path = path.display().to_string();
        }
    }

    fn create_image(&mut self) -> Option<CreatedImage> {
        let path = self.path.trim().to_owned();
        if path.is_empty() {
            self.status = Some(CreatorStatus::Error("image path is required".to_owned()));
            return None;
        }

        #[cfg(not(target_arch = "wasm32"))]
        let result = {
            let policy = if self.overwrite {
                ExistingFilePolicy::Truncate
            } else {
                ExistingFilePolicy::CreateNew
            };
            match self.kind {
                CreatorKind::HardDisk => self.create_hard_disk(&path, policy),
                CreatorKind::Floppy => create_floppy(&path, self.floppy_format, policy)
                    .map_err(|error| error.to_string()),
            }
        };

        #[cfg(target_arch = "wasm32")]
        let result = match self.kind {
            CreatorKind::HardDisk => ImageSize::parse(&self.hard_disk_size)
                .map_err(|error| error.to_string())
                .and_then(|size| create_browser_hard_disk_bytes(&path, size))
                .and_then(|(bytes, created)| {
                    download_bytes(&path, bytes)?;
                    Ok(created)
                }),
            CreatorKind::Floppy => create_browser_floppy_bytes(&path, self.floppy_format).and_then(
                |(bytes, created)| {
                    download_bytes(&path, bytes)?;
                    Ok(created)
                },
            ),
        };

        match result {
            Ok(created) => {
                self.status = Some(CreatorStatus::Success(
                    crate::disk_images::format_created_image_message(&created),
                ));
                Some(CreatedImage {
                    path: std::path::PathBuf::from(path),
                    kind: match self.kind {
                        CreatorKind::HardDisk => CreatedImageKind::HardDisk,
                        CreatorKind::Floppy => CreatedImageKind::Floppy,
                    },
                })
            }
            Err(error) => {
                self.status = Some(CreatorStatus::Error(error));
                None
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn create_hard_disk(
        &self,
        path: &str,
        policy: ExistingFilePolicy,
    ) -> Result<BxCreatedImage, String> {
        let size = ImageSize::parse(&self.hard_disk_size).map_err(|error| error.to_string())?;
        let geometry = calculate_hard_disk_geometry(size, SectorSize::Bytes512)
            .map_err(|error| error.to_string())?;
        if geometry.cylinders > u16::MAX as u64 {
            return Err(
                "disk is too large for the current Rusty Box BIOS geometry limit; choose 31 GiB or smaller"
                    .to_owned(),
            );
        }

        create_flat_hard_disk(path, size, SectorSize::Bytes512, policy)
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
type WebEmulator =
    Box<rusty_box::emulator::Emulator<'static, rusty_box::cpu::core_i7_skylake::Corei7SkylakeX>>;

#[cfg(target_arch = "wasm32")]
pub struct WebShellApp {
    chrome: ShellChrome,
    disk_creator: DiskCreatorPanel,
    boot_mode: WebBootMode,
    emulator: Option<WebEmulator>,
    display: rusty_box::gui::shared_display::SharedDisplay,
    texture: Option<egui::TextureHandle>,
    initialized: bool,
    init_error: Option<String>,
    shutdown: bool,
    startup: Option<WebStartupState>,
    file_slot: std::rc::Rc<core::cell::RefCell<Option<WebUploadedMedia>>>,
    file_picker: Option<WebFilePicker>,
    uploaded_media_name: Option<String>,
    uploaded_media_bytes: Option<usize>,
    web_memory_mib: usize,
    total_instructions: u64,
    last_ips_time: web_time::Instant,
    last_ips_instructions: u64,
    cached_ips: f64,
    frame_count: u64,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum WebBootMode {
    Launcher,
    UploadedMedia,
}
#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebStartupStage {
    CreateEmulator,
    InitializeMemory,
    LoadBios,
    LoadVgaBios,
    InitializeDevices,
    AttachMedia,
    StartEmulator,
}

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebRuntimeState {
    Error,
    Starting,
    Launcher,
    Stopped,
    Running,
}

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebConsoleSurface {
    Error,
    Starting,
    Display,
    Launcher,
    WaitingForDisplay,
}

#[cfg(target_arch = "wasm32")]
struct WebStartupState {
    stage: WebStartupStage,
    emulator: Option<WebEmulator>,
    iso_data: Option<Vec<u8>>,
    memory_mib: usize,
}

#[cfg(target_arch = "wasm32")]
impl WebStartupState {
    fn new(iso_data: Vec<u8>, memory_mib: usize) -> Self {
        Self {
            stage: WebStartupStage::CreateEmulator,
            emulator: None,
            iso_data: Some(iso_data),
            memory_mib,
        }
    }
}

#[cfg(target_arch = "wasm32")]
struct WebFilePicker {
    input: web_sys::HtmlInputElement,
    change_handler: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
}

#[cfg(target_arch = "wasm32")]
impl WebFilePicker {
    fn new(
        input: web_sys::HtmlInputElement,
        change_handler: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
    ) -> Self {
        Self {
            input,
            change_handler,
        }
    }

    fn activate(&self) {
        debug_assert_eq!(self.input.type_(), "file");
        debug_assert!(self.change_handler.as_ref().is_function());
        self.input.click();
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for WebFilePicker {
    fn drop(&mut self) {
        self.input.set_onchange(None);
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
const WEB_BATCH_SIZE: u64 = 1_000;
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_FRAME_BUDGET: u64 = 8_000;
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_FRAME_TIME_BUDGET_MS: u64 = 6;
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_STARTUP_STEPS_PER_FRAME: usize = 1;

#[cfg(any(test, target_arch = "wasm32"))]
fn web_next_startup_stage(stage: WebStartupStage) -> Option<WebStartupStage> {
    match stage {
        WebStartupStage::CreateEmulator => Some(WebStartupStage::InitializeMemory),
        WebStartupStage::InitializeMemory => Some(WebStartupStage::LoadBios),
        WebStartupStage::LoadBios => Some(WebStartupStage::LoadVgaBios),
        WebStartupStage::LoadVgaBios => Some(WebStartupStage::InitializeDevices),
        WebStartupStage::InitializeDevices => Some(WebStartupStage::AttachMedia),
        WebStartupStage::AttachMedia => Some(WebStartupStage::StartEmulator),
        WebStartupStage::StartEmulator => None,
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_startup_stage_label(stage: WebStartupStage) -> &'static str {
    match stage {
        WebStartupStage::CreateEmulator => "Allocating guest memory",
        WebStartupStage::InitializeMemory => "Allocating guest memory",
        WebStartupStage::LoadBios => "Loading BIOS",
        WebStartupStage::LoadVgaBios => "Loading VGA BIOS",
        WebStartupStage::InitializeDevices => "Initializing devices",
        WebStartupStage::AttachMedia => "Attaching boot media",
        WebStartupStage::StartEmulator => "Starting CPU",
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_memory_label(memory_mib: usize) -> String {
    format!("{memory_mib} MB")
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_memory_mib_is_supported(memory_mib: usize) -> bool {
    (WEB_MIN_MEMORY_MIB..=WEB_MAX_MEMORY_MIB).contains(&memory_mib)
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_can_edit_memory(has_vm: bool) -> bool {
    !has_vm
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_runtime_state(
    has_error: bool,
    startup_pending: bool,
    initialized: bool,
    shutdown: bool,
) -> WebRuntimeState {
    if has_error {
        WebRuntimeState::Error
    } else if startup_pending {
        WebRuntimeState::Starting
    } else if !initialized {
        WebRuntimeState::Launcher
    } else if shutdown {
        WebRuntimeState::Stopped
    } else {
        WebRuntimeState::Running
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_console_surface(
    has_error: bool,
    startup_pending: bool,
    has_texture: bool,
    launcher: bool,
) -> WebConsoleSurface {
    if has_error {
        WebConsoleSurface::Error
    } else if startup_pending {
        WebConsoleSurface::Starting
    } else if has_texture {
        WebConsoleSurface::Display
    } else if launcher {
        WebConsoleSurface::Launcher
    } else {
        WebConsoleSurface::WaitingForDisplay
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_should_pump_emulator_this_frame(advanced_startup: bool, has_input: bool) -> bool {
    !advanced_startup && !has_input
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_should_continue_emulator_frame(frame_executed: u64, elapsed: core::time::Duration) -> bool {
    frame_executed < WEB_FRAME_BUDGET
        && elapsed < core::time::Duration::from_millis(WEB_FRAME_TIME_BUDGET_MS)
}

#[cfg(target_arch = "wasm32")]
fn web_uploaded_media_config(memory_mib: usize) -> rusty_box::emulator::EmulatorConfig {
    let ram_size = memory_mib * 1024 * 1024;
    rusty_box::emulator::EmulatorConfig {
        guest_memory_size: ram_size,
        host_memory_size: ram_size,
        memory_block_size: 128 * 1024,
        ips: 300_000_000,
        pci_enabled: true,
        ..Default::default()
    }
}
#[cfg(target_arch = "wasm32")]
const BIOS_DATA: &[u8] = include_bytes!("../../cpp_orig/bochs/bios/BIOS-bochs-latest");
#[cfg(target_arch = "wasm32")]
const VGA_BIOS_DATA: &[u8] =
    include_bytes!("../../cpp_orig/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin");

#[cfg(target_arch = "wasm32")]
impl WebShellApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_shell_style(&cc.egui_ctx);
        let chrome = ShellChrome::with_library(vec![VmLibraryEntry::new(
            "Rusty Box Web",
            "Media upload",
            &web_memory_label(WEB_DEFAULT_MEMORY_MIB),
            "None",
            "No uploaded media",
        )]);
        Self {
            chrome,
            disk_creator: DiskCreatorPanel::default(),
            boot_mode: WebBootMode::Launcher,
            emulator: None,
            display: rusty_box::gui::shared_display::SharedDisplay::new(),
            texture: None,
            initialized: false,
            init_error: None,
            shutdown: false,
            startup: None,
            file_slot: std::rc::Rc::new(core::cell::RefCell::new(None)),
            file_picker: None,
            uploaded_media_name: None,
            uploaded_media_bytes: None,
            web_memory_mib: WEB_DEFAULT_MEMORY_MIB,
            total_instructions: 0,
            last_ips_time: web_time::Instant::now(),
            last_ips_instructions: 0,
            cached_ips: 0.0,
            frame_count: 0,
        }
    }

    fn begin_uploaded_media_startup(&mut self, upload: WebUploadedMedia) {
        self.record_uploaded_media_metadata(&upload.name, upload.data.len());
        self.startup = Some(WebStartupState::new(upload.data, self.web_memory_mib));
        self.boot_mode = WebBootMode::UploadedMedia;
        self.chrome.selected_page = ShellPage::Console;
        self.initialized = false;
        self.init_error = None;
        self.shutdown = false;
    }

    fn advance_uploaded_media_startup(&mut self) -> bool {
        let mut advanced = false;
        for startup_step in 0..WEB_STARTUP_STEPS_PER_FRAME {
            match self.try_advance_uploaded_media_startup() {
                Ok(Some(emu)) => {
                    advanced = true;
                    self.emulator = Some(emu);
                    self.startup = None;
                    self.initialized = true;
                    self.init_error = None;
                    self.shutdown = false;
                    break;
                }
                Ok(None) => {
                    advanced = true;
                }
                Err(error) => {
                    advanced = true;
                    self.init_error = Some(error);
                    self.startup = None;
                    self.shutdown = true;
                    break;
                }
            }
            if startup_step + 1 >= WEB_STARTUP_STEPS_PER_FRAME {
                break;
            }
        }
        advanced
    }

    fn try_advance_uploaded_media_startup(&mut self) -> Result<Option<WebEmulator>, String> {
        let Some(startup) = self.startup.as_mut() else {
            return Ok(None);
        };

        match startup.stage {
            WebStartupStage::CreateEmulator => {
                let emu = rusty_box::emulator::Emulator::<
                    rusty_box::cpu::core_i7_skylake::Corei7SkylakeX,
                >::new(web_uploaded_media_config(startup.memory_mib))
                .map_err(|error| format!("{error:?}"))?;
                startup.emulator = Some(emu);
            }
            WebStartupStage::InitializeMemory => {
                startup
                    .emulator
                    .as_mut()
                    .expect("startup emulator should exist before memory initialization")
                    .init_memory_and_pc_system()
                    .map_err(|error| format!("{error:?}"))?;
            }
            WebStartupStage::LoadBios => {
                let bios_load_addr = !(BIOS_DATA.len() as u64 - 1);
                startup
                    .emulator
                    .as_mut()
                    .expect("startup emulator should exist before BIOS load")
                    .load_bios(BIOS_DATA, bios_load_addr)
                    .map_err(|error| format!("{error:?}"))?;
            }
            WebStartupStage::LoadVgaBios => {
                let mut vga_data = VGA_BIOS_DATA.to_vec();
                let remainder = vga_data.len() % 512;
                if remainder != 0 {
                    vga_data.resize(vga_data.len() + (512 - remainder), 0);
                }
                startup
                    .emulator
                    .as_mut()
                    .expect("startup emulator should exist before VGA BIOS load")
                    .load_optional_rom(&vga_data, 0xC0000)
                    .map_err(|error| format!("{error:?}"))?;
            }
            WebStartupStage::InitializeDevices => {
                let emu = startup
                    .emulator
                    .as_mut()
                    .expect("startup emulator should exist before device initialization");
                emu.init_cpu_and_devices()
                    .map_err(|error| format!("{error:?}"))?;
                let ram_size = startup.memory_mib * 1024 * 1024;
                let ext_kb = ((ram_size / 1024) - 1024).min(u16::MAX as usize);
                emu.configure_memory_in_cmos(640, ext_kb as u16);
                emu.configure_boot_sequence(3, 0, 0);
            }
            WebStartupStage::AttachMedia => {
                let iso_data = startup.iso_data.take().ok_or_else(|| {
                    "uploaded boot media was not available during startup".to_owned()
                })?;
                startup
                    .emulator
                    .as_mut()
                    .expect("startup emulator should exist before media attach")
                    .attach_cdrom_data(1, 0, iso_data);
            }
            WebStartupStage::StartEmulator => {
                let emu = startup
                    .emulator
                    .as_mut()
                    .expect("startup emulator should exist before emulator start");
                emu.init_gui(0, &[]).map_err(|error| format!("{error:?}"))?;
                emu.reset(rusty_box::cpu::ResetReason::Hardware)
                    .map_err(|error| format!("{error:?}"))?;
                emu.start();
                emu.force_vga_update();
                return Ok(Some(
                    startup
                        .emulator
                        .take()
                        .expect("startup emulator should exist after start"),
                ));
            }
        }

        startup.stage = web_next_startup_stage(startup.stage)
            .expect("startup stage should advance until StartEmulator");
        Ok(None)
    }

    fn web_has_vm(&self) -> bool {
        self.boot_mode != WebBootMode::Launcher
            || self.initialized
            || self.emulator.is_some()
            || self.startup.is_some()
    }

    fn handle_primary_toolbar_action(&mut self) {
        if self.web_has_vm() {
            self.chrome.selected_page = ShellPage::Console;
        } else {
            self.chrome.selected_page = ShellPage::Home;
            self.open_file_picker();
        }
    }

    fn record_uploaded_media_metadata(&mut self, name: &str, byte_len: usize) {
        let display_name = web_uploaded_media_name(name).to_owned();
        self.uploaded_media_name = Some(display_name.clone());
        self.uploaded_media_bytes = Some(byte_len);
        if let Some(entry) = self.chrome.vm_library.get_mut(0) {
            entry.boot = "Uploaded media".to_owned();
            entry.cdrom = web_uploaded_media_summary(&display_name, byte_len);
        }
    }

    fn clear_uploaded_media_metadata(&mut self) {
        self.uploaded_media_name = None;
        self.uploaded_media_bytes = None;
        if let Some(entry) = self.chrome.vm_library.get_mut(0) {
            entry.boot = "Media upload".to_owned();
            entry.cdrom = "No uploaded media".to_owned();
        }
    }

    fn set_web_memory_mib(&mut self, memory_mib: usize) {
        if !web_memory_mib_is_supported(memory_mib) || !web_can_edit_memory(self.web_has_vm()) {
            return;
        }
        self.web_memory_mib = memory_mib;
        if let Some(entry) = self.chrome.vm_library.get_mut(0) {
            entry.memory = web_memory_label(memory_mib);
        }
    }

    fn open_file_picker(&mut self) {
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;

        let Some(window) = web_sys::window() else {
            self.init_error = Some("browser window is unavailable".to_owned());
            return;
        };
        let Some(document) = window.document() else {
            self.init_error = Some("browser document is unavailable".to_owned());
            return;
        };
        let input = match document.create_element("input") {
            Ok(element) => match element.dyn_into::<web_sys::HtmlInputElement>() {
                Ok(input) => input,
                Err(_) => {
                    self.init_error = Some("file input element has unexpected type".to_owned());
                    return;
                }
            },
            Err(error) => {
                self.init_error = Some(js_error(error));
                return;
            }
        };
        input.set_type("file");
        input.set_accept(".iso,.img");

        let slot = std::rc::Rc::clone(&self.file_slot);
        let closure =
            Closure::<dyn FnMut(web_sys::Event)>::wrap(Box::new(move |event: web_sys::Event| {
                let Some(target) = event.target() else {
                    return;
                };
                let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() else {
                    return;
                };
                let Some(files) = input.files() else {
                    return;
                };
                let Some(file) = files.get(0) else {
                    return;
                };

                let upload_name = file.name();
                let slot_inner = std::rc::Rc::clone(&slot);
                let array_buffer = file.array_buffer();
                wasm_bindgen_futures::spawn_local(async move {
                    match wasm_bindgen_futures::JsFuture::from(array_buffer).await {
                        Ok(result) => {
                            let array = js_sys::Uint8Array::new(&result);
                            let data = array.to_vec();
                            *slot_inner.borrow_mut() = Some(WebUploadedMedia {
                                name: upload_name,
                                data,
                            });
                        }
                        Err(_) => {
                            *slot_inner.borrow_mut() = None;
                        }
                    }
                });
            }));

        input.set_onchange(Some(closure.as_ref().unchecked_ref()));
        self.file_picker = Some(WebFilePicker::new(input, closure));
        if let Some(file_picker) = self.file_picker.as_ref() {
            file_picker.activate();
        }
    }

    fn pump_emulator(&mut self) {
        if self.initialized && !self.shutdown {
            if let Some(emu) = &mut self.emulator {
                let frame_start = web_time::Instant::now();
                let mut frame_executed = 0u64;
                while web_should_continue_emulator_frame(
                    frame_executed,
                    web_time::Instant::now().duration_since(frame_start),
                ) {
                    match emu.step_batch(WEB_BATCH_SIZE) {
                        Ok((executed, is_shutdown)) => {
                            frame_executed = frame_executed.saturating_add(executed);
                            if is_shutdown {
                                self.shutdown = true;
                                break;
                            }
                            if executed == 0 {
                                break;
                            }
                        }
                        Err(error) => {
                            self.init_error = Some(format!("{error:?}"));
                            self.shutdown = true;
                            break;
                        }
                    }
                }
                self.total_instructions = self.total_instructions.saturating_add(frame_executed);
                emu.update_display(&mut self.display);
            }
        }
    }

    fn process_keyboard(&mut self, ctx: &egui::Context) {
        let Some(emu) = &mut self.emulator else {
            return;
        };
        ctx.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Text(text) => {
                        for ch in text.chars() {
                            let seq = rusty_box::gui::char_to_scancode_sequence(ch);
                            for scancode in seq {
                                emu.send_scancode(scancode);
                            }
                        }
                    }
                    egui::Event::Key { key, pressed, .. } => {
                        let seq = egui_key_to_scancodes(*key, *pressed);
                        for scancode in seq {
                            emu.send_scancode(scancode);
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    fn update_ips(&mut self) {
        let now = web_time::Instant::now();
        let elapsed = now.duration_since(self.last_ips_time);
        if elapsed.as_secs_f64() >= 1.0 {
            let delta = self.total_instructions - self.last_ips_instructions;
            self.cached_ips = delta as f64 / elapsed.as_secs_f64();
            self.last_ips_time = now;
            self.last_ips_instructions = self.total_instructions;
        }
    }

    fn upload_texture(&mut self, ctx: &egui::Context) {
        let width = self.display.fb_width as usize;
        let height = self.display.fb_height as usize;
        if width == 0 || height == 0 || (!self.display.fb_dirty && self.texture.is_some()) {
            return;
        }

        let pixels: Vec<Color32> = self
            .display
            .framebuffer
            .chunks_exact(4)
            .map(|rgba| Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]))
            .collect();
        let expected = width * height;
        let image = if pixels.len() == expected {
            egui::ColorImage::new([width, height], pixels)
        } else {
            let mut padded = vec![Color32::BLACK; expected];
            let copy_len = pixels.len().min(expected);
            padded[..copy_len].copy_from_slice(&pixels[..copy_len]);
            egui::ColorImage::new([width, height], padded)
        };
        self.display.fb_dirty = false;

        match &mut self.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
            None => {
                self.texture =
                    Some(ctx.load_texture("vga_display", image, egui::TextureOptions::NEAREST));
            }
        }
    }

    fn draw_menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("web_vm_menu_bar")
            .exact_size(32.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button(WEB_BOOT_MEDIA_ACTION_LABEL).clicked() {
                            self.open_file_picker();
                            ui.close();
                        }
                        if ui.button("Create Disk Image").clicked() {
                            self.chrome.selected_page = ShellPage::Images;
                            ui.close();
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.button("Clear Library Search").clicked() {
                            self.chrome.library_filter.clear();
                            ui.close();
                        }
                    });
                    ui.menu_button("VM", |ui| {
                        if ui
                            .add_enabled(self.web_has_vm(), egui::Button::new("Reset Browser VM"))
                            .clicked()
                        {
                            self.reset_web_vm();
                            ui.close();
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("About Rusty Box Workstation").clicked() {
                            self.chrome.show_about = true;
                            ui.close();
                        }
                    });
                    ui.separator();
                    self.nav_button(ui, ShellPage::Home, "Home");
                    self.nav_button(ui, ShellPage::Console, "Console");
                    self.nav_button(ui, ShellPage::Hardware, "Hardware");
                    self.nav_button(ui, ShellPage::Images, "Images");
                });
            });
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("web_vm_toolbar")
            .exact_size(46.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x0D, 0x13, 0x1A))
                    .inner_margin(egui::Margin::symmetric(14, 6)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let has_vm = self.web_has_vm();
                    if ui.button(web_primary_action_label(has_vm)).clicked() {
                        self.handle_primary_toolbar_action();
                    }
                    if ui
                        .add_enabled(has_vm, egui::Button::new("↻ Reset Browser VM"))
                        .clicked()
                    {
                        self.reset_web_vm();
                    }
                    if ui.button("▣ Hardware").clicked() {
                        self.chrome.selected_page = ShellPage::Hardware;
                    }
                    if ui.button("＋ New Image").clicked() {
                        self.chrome.selected_page = ShellPage::Images;
                    }
                    ui.checkbox(&mut self.chrome.show_library, "Library");
                    ui.checkbox(&mut self.chrome.show_serial, "Serial");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Rusty Box Web").strong().color(TEXT_PRIMARY));
                    });
                });
            });
    }

    fn draw_library(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("web_vm_library")
            .resizable(true)
            .default_size(250.0)
            .min_size(210.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .inner_margin(egui::Margin::same(14)),
            )
            .show_inside(ui, |ui| {
                ui.label(
                    RichText::new("Library")
                        .size(16.0)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.chrome.library_filter)
                        .hint_text("Type here to search"),
                );
                ui.add_space(8.0);
                ui.label(RichText::new("▾ My Computer").color(TEXT_MUTED));
                let visible = self.chrome.visible_vm_indices();
                for index in visible {
                    let clicked = {
                        let entry = &self.chrome.vm_library[index];
                        let selected = ui.selectable_label(
                            self.chrome.selected_vm == index,
                            format!("  ▣ {}", entry.name),
                        );
                        if self.chrome.selected_vm == index {
                            ui.indent(format!("web_library_metadata_{index}"), |ui| {
                                ui.label(metadata_text("Boot", &entry.boot));
                                ui.label(metadata_text("Memory", &entry.memory));
                                ui.label(metadata_text("Disk", &entry.disk));
                                ui.label(metadata_text("CD/DVD", &entry.cdrom));
                            });
                        }
                        selected.clicked()
                    };
                    if clicked {
                        self.chrome.selected_vm = index;
                        self.chrome.selected_page = ShellPage::Home;
                    }
                }
            });
    }

    fn draw_status_strip(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("web_vm_status_strip")
            .exact_size(30.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 4)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let state = web_runtime_state(
                        self.init_error.is_some(),
                        self.startup.is_some(),
                        self.initialized,
                        self.shutdown,
                    );
                    let (label, color) = match state {
                        WebRuntimeState::Error => ("Error", ACCENT_RED),
                        WebRuntimeState::Starting => ("Starting", ACCENT_AMBER),
                        WebRuntimeState::Launcher => ("Launcher", ACCENT_AMBER),
                        WebRuntimeState::Stopped => ("Stopped", TEXT_MUTED),
                        WebRuntimeState::Running => ("Running", ACCENT_CYAN),
                    };
                    status_dot(ui, color);
                    ui.label(RichText::new(label).monospace().size(11.0).color(color));
                    ui.separator();
                    ui.label(
                        RichText::new(format!("{} IPS", format_ips_f64(self.cached_ips)))
                            .monospace()
                            .size(11.0)
                            .color(ACCENT_BLUE),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("frame {}", self.frame_count))
                            .monospace()
                            .size(11.0)
                            .color(TEXT_MUTED),
                    );
                });
            });
    }

    fn draw_central(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_BASE))
            .show_inside(ui, |ui| match self.chrome.selected_page {
                ShellPage::Home => self.draw_web_home_page(ui),
                ShellPage::Console => self.draw_web_console_page(ui),
                ShellPage::Hardware => self.draw_web_hardware_page(ui),
                ShellPage::Images => {
                    drop(self.disk_creator.ui_page(ui));
                }
            });
    }

    fn draw_web_home_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("RUSTY BOX WORKSTATION")
                        .size(26.0)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.label(
                    RichText::new("Browser-safe launcher and disk image downloads")
                        .color(TEXT_MUTED),
                );
            });
            ui.add_space(24.0);
            ui.columns(3, |columns| {
                action_tile(
                    &mut columns[0],
                    WEB_BOOT_MEDIA_ACTION_LABEL,
                    WEB_BOOT_MEDIA_ACTION_DESCRIPTION,
                    ACCENT_BLUE,
                    || self.open_file_picker(),
                );
                disabled_tile(
                    &mut columns[1],
                    "Boot DLX sample",
                    "DLX sample is not bundled in this build",
                );
                action_tile(
                    &mut columns[2],
                    "Create Disk Image",
                    "Download bximage-compatible zero-filled images.",
                    ACCENT_CYAN,
                    || self.chrome.selected_page = ShellPage::Images,
                );
            });
        });
    }

    fn draw_web_console_page(&mut self, ui: &mut egui::Ui) {
        match web_console_surface(
            self.init_error.is_some(),
            self.startup.is_some(),
            self.texture.is_some(),
            self.boot_mode == WebBootMode::Launcher,
        ) {
            WebConsoleSurface::Error => {
                let error = self
                    .init_error
                    .as_deref()
                    .unwrap_or("unknown initialization error");
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Initialization Error")
                                .size(18.0)
                                .color(ACCENT_RED),
                        );
                        ui.label(RichText::new(error).monospace().color(TEXT_MUTED));
                    });
                });
            }
            WebConsoleSurface::Starting => {
                let stage = self
                    .startup
                    .as_ref()
                    .map(|startup| startup.stage)
                    .unwrap_or(WebStartupStage::CreateEmulator);
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new(web_startup_stage_label(stage))
                                .size(18.0)
                                .color(TEXT_PRIMARY),
                        );
                        ui.label(
                            RichText::new("Preparing the browser VM. This can take a moment.")
                                .color(TEXT_MUTED),
                        );
                    });
                });
            }
            WebConsoleSurface::Display => {
                let texture = self
                    .texture
                    .as_ref()
                    .expect("display surface requires a texture");
                let available = ui.available_size();
                let tex_w = self.display.fb_width as f32;
                let tex_h = self.display.fb_height.max(1) as f32;
                let max_scale_x = (available.x / tex_w).floor().max(1.0);
                let max_scale_y = (available.y / tex_h).floor().max(1.0);
                let scale = max_scale_x.min(max_scale_y);
                let size = egui::vec2(tex_w * scale, tex_h * scale);
                ui.centered_and_justified(|ui| {
                    ui.image(egui::load::SizedTexture::new(texture.id(), size));
                });
            }
            WebConsoleSurface::Launcher => {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("No VM booted").size(18.0).color(TEXT_PRIMARY));
                        if ui.button(WEB_BOOT_MEDIA_ACTION_LABEL).clicked() {
                            self.open_file_picker();
                        }
                    });
                });
            }
            WebConsoleSurface::WaitingForDisplay => {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Waiting for VGA output...").color(TEXT_MUTED));
                        ui.spinner();
                    });
                });
            }
        }
    }

    fn draw_web_hardware_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            shell_card_frame().show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(190.0);
                    ui.label(RichText::new("Devices").strong().color(TEXT_PRIMARY));
                    ui.add_space(8.0);
                    for device in HardwareDevice::ALL {
                        if ui
                            .selectable_label(
                                self.chrome.selected_hardware == device,
                                device.label(),
                            )
                            .clicked()
                        {
                            self.chrome.selected_hardware = device;
                        }
                    }
                });
            });
            shell_card_frame().show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(520.0);
                    ui.label(
                        RichText::new(format!(
                            "Hardware Summary  |  {}",
                            self.chrome.selected_hardware.label()
                        ))
                        .size(18.0)
                        .strong(),
                    );
                    ui.separator();
                    ui.add(
                        egui::Label::new(RichText::new(WEB_HARDWARE_NOTICE).color(ACCENT_AMBER))
                            .wrap(),
                    );
                    ui.add_space(8.0);
                    self.draw_web_hardware_detail(ui);
                });
            });
        });
    }

    fn draw_web_hardware_detail(&mut self, ui: &mut egui::Ui) {
        match self.chrome.selected_hardware {
            HardwareDevice::Memory => {
                hardware_intro(
                    ui,
                    "Browser memory",
                    "Choose guest RAM before boot. Wasm32 can address up to 4 GiB, but allocation still depends on browser and device memory.",
                );
                detail_row(
                    ui,
                    "Installed memory",
                    &web_memory_label(self.web_memory_mib),
                );
                let can_edit = web_can_edit_memory(self.web_has_vm());
                let mut memory_mib = self.web_memory_mib;
                ui.add_enabled_ui(can_edit, |ui| {
                    let changed = ui
                        .add(
                            egui::DragValue::new(&mut memory_mib)
                                .range(WEB_MIN_MEMORY_MIB..=WEB_MAX_MEMORY_MIB)
                                .speed(WEB_MEMORY_DRAG_SPEED_MIB)
                                .suffix(" MB"),
                        )
                        .changed();
                    ui.label(
                        RichText::new(format!(
                            "Range: {} MB – {} MB",
                            WEB_MIN_MEMORY_MIB, WEB_MAX_MEMORY_MIB
                        ))
                        .color(TEXT_MUTED),
                    );
                    if changed {
                        self.set_web_memory_mib(memory_mib);
                    }
                });
                if !can_edit {
                    ui.label(
                        RichText::new("Reset the browser VM before changing memory.")
                            .color(TEXT_MUTED),
                    );
                }
            }
            HardwareDevice::Processors => {
                hardware_intro(
                    ui,
                    "Cooperative CPU",
                    "Browser execution runs in frame-sized batches to keep the UI responsive while the emulator advances.",
                );
                detail_row(ui, "Virtual processors", "1");
                detail_row(ui, "Execution", "Cooperative frame batches");
            }
            HardwareDevice::Devices => {
                hardware_intro(
                    ui,
                    "Browser devices",
                    "The browser build exposes a fixed virtual machine profile and does not persist hardware edits.",
                );
                detail_row(ui, "Edit mode", "Read-only");
                detail_row(ui, "Boot media", "Upload on Home");
            }
            HardwareDevice::HardDisk => {
                hardware_intro(
                    ui,
                    "Browser disk images",
                    "The browser does not attach host disks. Use Images to download flat disk or floppy images.",
                );
                detail_row(ui, "Attached disk", "None");
                detail_row(ui, "Disk images", "Download from Images page");
            }
            HardwareDevice::CdDvd => {
                let attached_media = match (&self.uploaded_media_name, self.uploaded_media_bytes) {
                    (Some(name), Some(byte_len)) => web_uploaded_media_summary(name, byte_len),
                    _ => "No uploaded media".to_owned(),
                };
                hardware_intro(
                    ui,
                    "Uploaded boot media",
                    "Home opens a browser file picker and attaches the selected image as bootable CD/DVD media.",
                );
                detail_row(ui, "Attached media", &attached_media);
                detail_row(ui, "Boot mode", "Uploaded boot image");
            }
            HardwareDevice::Display => {
                hardware_intro(
                    ui,
                    "Canvas display",
                    "The VGA framebuffer is uploaded as an egui texture and scaled with nearest-neighbor filtering.",
                );
                detail_row(ui, "Adapter", "VGA framebuffer texture");
                detail_row(ui, "Texture filter", "Nearest-neighbor pixel scale");
            }
        }
    }
    fn nav_button(&mut self, ui: &mut egui::Ui, page: ShellPage, label: &str) {
        if ui
            .selectable_label(self.chrome.selected_page == page, label)
            .clicked()
        {
            self.chrome.selected_page = page;
        }
    }

    fn reset_web_vm(&mut self) {
        self.emulator = None;
        self.texture = None;
        self.startup = None;
        self.file_picker = None;
        self.initialized = false;
        self.init_error = None;
        self.shutdown = false;
        self.total_instructions = 0;
        self.last_ips_instructions = 0;
        self.cached_ips = 0.0;
        self.clear_uploaded_media_metadata();
        if self.boot_mode != WebBootMode::Launcher {
            self.boot_mode = WebBootMode::Launcher;
            self.chrome.selected_page = ShellPage::Home;
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl eframe::App for WebShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame_count = self.frame_count.saturating_add(1);
        let uploaded = self.file_slot.borrow_mut().take();
        let mut uploaded_this_frame = false;
        if let Some(upload) = uploaded {
            uploaded_this_frame = true;
            if web_upload_replaces_browser_vm(self.web_has_vm()) {
                self.reset_web_vm();
            }
            self.file_picker = None;
            self.begin_uploaded_media_startup(upload);
        }
        let mut advanced_startup_this_frame = false;
        if !uploaded_this_frame
            && self.boot_mode == WebBootMode::UploadedMedia
            && !self.initialized
            && self.init_error.is_none()
        {
            advanced_startup_this_frame = self.advance_uploaded_media_startup();
        }
        let has_input_this_frame = ui.ctx().input(|input| !input.events.is_empty());
        if web_should_pump_emulator_this_frame(advanced_startup_this_frame, has_input_this_frame) {
            self.pump_emulator();
        }
        if self.chrome.selected_page == ShellPage::Console {
            self.process_keyboard(ui.ctx());
        }
        self.update_ips();
        self.upload_texture(ui.ctx());

        self.draw_menu_bar(ui);
        self.draw_toolbar(ui);
        if shell_should_draw_library(&self.chrome) {
            self.draw_library(ui);
        }
        self.draw_status_strip(ui);
        self.draw_central(ui);
        draw_about_window(ui.ctx(), &mut self.chrome);

        if !self.shutdown {
            ui.ctx().request_repaint();
        }
    }
}

fn draw_about_window(ctx: &egui::Context, chrome: &mut ShellChrome) {
    if !chrome.show_about {
        return;
    }

    egui::Window::new("About Rusty Box Workstation")
        .collapsible(false)
        .resizable(false)
        .open(&mut chrome.show_about)
        .show(ctx, |ui| {
            ui.label(RichText::new("Rusty Box Workstation").size(18.0).strong());
            ui.label("VMware-style shell for Rusty Box emulator sessions.");
            ui.separator();
            ui.label(metadata_text(
                "Console",
                "existing emulator display and keyboard path",
            ));
            ui.label(metadata_text(
                "Images",
                "bximage-backed hard disk and floppy creation",
            ));
            ui.label(metadata_text(
                "Browser",
                "upload ISO, download generated images",
            ));
        });
}
fn metadata_text(label: &str, value: &str) -> RichText {
    RichText::new(format!("{label}: {value}"))
        .size(11.0)
        .color(TEXT_MUTED)
}

fn hardware_intro(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.label(RichText::new(title).size(16.0).strong().color(TEXT_PRIMARY));
    ui.label(RichText::new(body).color(TEXT_MUTED));
    ui.add_space(10.0);
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.set_min_width(150.0);
        ui.label(RichText::new(label).strong().color(TEXT_PRIMARY));
        ui.label(RichText::new(value).color(TEXT_MUTED));
    });
}

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

#[cfg(not(target_arch = "wasm32"))]
fn format_ips_u32(ips: u32) -> String {
    if ips >= 1_000_000 {
        format!("{:.3}M IPS", ips as f64 / 1_000_000.0)
    } else if ips >= 1_000 {
        format!("{}K IPS", ips / 1_000)
    } else if ips > 0 {
        format!("{ips} IPS")
    } else {
        "--- IPS".to_owned()
    }
}

#[cfg(target_arch = "wasm32")]
fn format_ips_f64(ips: f64) -> String {
    if ips >= 1_000_000.0 {
        format!("{:.2}M", ips / 1_000_000.0)
    } else if ips >= 1_000.0 {
        format!("{:.0}K", ips / 1_000.0)
    } else if ips > 0.0 {
        format!("{ips:.0}")
    } else {
        "---".to_owned()
    }
}

fn action_tile(
    ui: &mut egui::Ui,
    title: &str,
    body: &str,
    accent: Color32,
    on_click: impl FnMut(),
) {
    action_tile_enabled(ui, title, body, accent, true, on_click);
}

fn action_tile_enabled(
    ui: &mut egui::Ui,
    title: &str,
    body: &str,
    accent: Color32,
    enabled: bool,
    mut on_click: impl FnMut(),
) {
    shell_card_frame().show(ui, |ui| {
        ui.set_min_height(150.0);
        let title_color = if enabled { TEXT_PRIMARY } else { TEXT_MUTED };
        ui.label(RichText::new(title).size(18.0).strong().color(title_color));
        ui.label(RichText::new(body).color(TEXT_MUTED));
        ui.add_space(16.0);
        let button = egui::Button::new(RichText::new(title).strong())
            .fill(Color32::from_rgb(0x1E, 0x35, 0x43))
            .stroke(Stroke::new(1.0_f32, accent));
        if ui.add_enabled(enabled, button).clicked() {
            on_click();
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn disabled_tile(ui: &mut egui::Ui, title: &str, body: &str) {
    shell_card_frame().show(ui, |ui| {
        ui.set_min_height(150.0);
        ui.label(RichText::new(title).size(18.0).strong().color(TEXT_MUTED));
        ui.label(RichText::new(body).color(TEXT_MUTED));
        ui.add_enabled(false, egui::Button::new("Unavailable"));
    });
}

#[cfg(target_arch = "wasm32")]
fn download_bytes(filename: &str, bytes: Vec<u8>) -> Result<(), String> {
    use wasm_bindgen::JsCast;

    let array = js_sys::Uint8Array::from(bytes.as_slice());
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(js_error)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js_error)?;
    let result = (|| -> Result<(), String> {
        let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_owned())?;
        let document = window
            .document()
            .ok_or_else(|| "browser document is unavailable".to_owned())?;
        let anchor = document
            .create_element("a")
            .map_err(js_error)?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "download anchor has unexpected type".to_owned())?;
        anchor.set_href(&url);
        anchor.set_download(filename);
        anchor.click();
        Ok(())
    })();
    if web_sys::Url::revoke_object_url(&url).is_err() {
        return Err("failed to revoke browser download URL".to_owned());
    }
    result
}

#[cfg(target_arch = "wasm32")]
fn create_browser_hard_disk_bytes(
    filename: &str,
    size: ImageSize,
) -> Result<(Vec<u8>, BxCreatedImage), String> {
    let geometry = calculate_hard_disk_geometry(size, SectorSize::Bytes512)
        .map_err(|error| error.to_string())?;
    if geometry.final_bytes > BROWSER_MAX_DOWNLOAD_BYTES {
        return Err(
            "browser downloads are capped at 64 MiB; use the desktop app for sparse large disks"
                .to_owned(),
        );
    }
    let mut cursor = std::io::Cursor::new(Vec::new());
    let created = rusty_box_bximage::create_flat_hard_disk_to_writer(
        filename,
        &mut cursor,
        size,
        SectorSize::Bytes512,
    )
    .map_err(|error| error.to_string())?;
    Ok((cursor.into_inner(), created))
}

#[cfg(target_arch = "wasm32")]
fn create_browser_floppy_bytes(
    filename: &str,
    format: FloppyFormat,
) -> Result<(Vec<u8>, BxCreatedImage), String> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    let created = rusty_box_bximage::create_floppy_to_writer(filename, &mut cursor, format)
        .map_err(|error| error.to_string())?;
    Ok((cursor.into_inner(), created))
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "browser JavaScript operation failed".to_owned())
}

#[cfg(target_arch = "wasm32")]
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
        egui::Key::ArrowUp => (true, 0x75),
        egui::Key::ArrowDown => (true, 0x72),
        egui::Key::ArrowLeft => (true, 0x6B),
        egui::Key::ArrowRight => (true, 0x74),
        egui::Key::Home => (true, 0x6C),
        egui::Key::End => (true, 0x69),
        egui::Key::PageUp => (true, 0x7D),
        egui::Key::PageDown => (true, 0x7A),
        egui::Key::Delete => (true, 0x71),
        egui::Key::Insert => (true, 0x70),
        egui::Key::Space => (false, 0x29),
        _ => return Vec::new(),
    };
    if extended {
        if pressed {
            vec![0xE0, make_code]
        } else {
            vec![0xE0, 0xF0, make_code]
        }
    } else if pressed {
        vec![make_code]
    } else {
        vec![0xF0, make_code]
    }
}

#[cfg(test)]
#[cfg(feature = "gui-egui")]
mod tests {
    use super::*;
    use std::fs;

    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{nanos}.img", std::process::id()))
    }

    fn remove_test_file(path: &std::path::Path) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove {}: {error}", path.display()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_test_disk(path: &std::path::Path) {
        fs::write(path, vec![0u8; 512 * 16 * 63]).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn test_resolved_config() -> crate::config::ResolvedConfig {
        crate::config::ResolvedConfig {
            memory_mib: 256,
            host_memory_mib: 256,
            memory_block_kib: 128,
            ips: 300_000_000,
            pci: true,
            sync_slowdown: false,
            max_instructions: u64::MAX,
            display: crate::args::DisplayBackend::Egui,
            bios: std::path::PathBuf::from("bios.bin"),
            vga_bios: Some(std::path::PathBuf::from("vgabios.bin")),
            boot_order: vec![crate::args::BootDevice::Cdrom],
            disk: None,
            cdrom: Some(crate::config::ResolvedCdrom {
                path: std::path::PathBuf::from("boot.iso"),
                channel: 1,
                drive: 0,
            }),
            log_level: crate::args::LogLevel::Warn,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn native_test_app() -> (
        NativeShellApp,
        std::sync::mpsc::Receiver<NativeEmulatorCommand>,
    ) {
        let shared = Arc::new(Mutex::new(
            rusty_box::gui::shared_display::SharedDisplay::new(),
        ));
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        let profile = NativeVmProfile::from_config("Rusty Box", test_resolved_config());
        let vm_info = profile.vm_info();
        let settings = profile.settings.clone();
        let config = profile.config.clone();
        let chrome = ShellChrome::with_library(vec![profile.library_entry()]);

        (
            NativeShellApp {
                emulator: rusty_box::gui::RustyBoxApp::new_embedded(Arc::clone(&shared)),
                chrome,
                disk_creator: DiskCreatorPanel::default(),
                profiles: vec![profile],
                config,
                settings,
                vm_info,
                command_tx,
                shared,
                shell_notice: None,
            },
            command_rx,
        )
    }

    #[test]
    fn shell_starts_on_home_page() {
        let chrome = ShellChrome::default();
        assert_eq!(chrome.selected_page, ShellPage::Home);
    }

    #[test]
    fn shell_hardware_list_starts_on_memory_device() {
        let chrome = ShellChrome::default();
        assert_eq!(chrome.selected_hardware, HardwareDevice::Memory);
    }

    #[test]
    fn library_filter_handles_multiple_vms() {
        let mut chrome = ShellChrome::default();
        chrome.vm_library = vec![
            VmLibraryEntry::new("Alpine VM", "cdrom", "256 MB", "None", "alpine.iso"),
            VmLibraryEntry::new("DOS Lab", "disk", "16 MB", "dos.img", "None"),
        ];
        chrome.library_filter = "dos".to_owned();

        assert_eq!(chrome.visible_vm_indices(), vec![1]);
    }

    #[test]
    fn shell_library_sidebar_is_visible_by_default() {
        let chrome = ShellChrome::default();
        assert!(chrome.show_library);
    }

    #[test]
    fn shell_library_sidebar_respects_visibility_toggle() {
        let mut chrome = ShellChrome::default();
        assert!(shell_should_draw_library(&chrome));
        chrome.show_library = false;
        assert!(!shell_should_draw_library(&chrome));
    }

    #[test]
    fn shell_menu_labels_omit_redundant_view_and_tabs() {
        let labels = shell_menu_labels();
        assert_eq!(labels, ["File", "Edit", "VM", "Help"]);
        assert!(!labels.contains(&"View"));
        assert!(!labels.contains(&"Tabs"));
    }

    #[test]
    fn web_primary_action_label_starts_with_boot_media_then_console() {
        assert_eq!(web_primary_action_label(false), "▶ Boot OS Image");
        assert_eq!(web_primary_action_label(true), "Console");
    }

    #[test]
    fn web_uploaded_media_summary_uses_file_name_and_size() {
        assert_eq!(
            web_uploaded_media_summary("alpine.iso", 66 * 1024 * 1024),
            "alpine.iso (66 MiB)"
        );
    }

    #[test]
    fn web_hardware_notice_explains_when_memory_is_editable() {
        assert_eq!(
            WEB_HARDWARE_NOTICE,
            "Browser hardware can be changed before boot. Reset the VM to edit it again."
        );
        assert!(WEB_HARDWARE_NOTICE.len() <= 78);
    }

    #[test]
    fn web_memory_profiles_are_user_selectable() {
        assert_eq!(WEB_DEFAULT_MEMORY_MIB, 128);
        assert_eq!(WEB_MAX_MEMORY_MIB, 4096);
        assert_eq!(web_memory_label(4096), "4096 MB");
        assert_eq!(WEB_MEMORY_DRAG_SPEED_MIB, 1.0);
        assert!(web_memory_mib_is_supported(777));
        assert!(web_memory_mib_is_supported(1));
        assert!(web_memory_mib_is_supported(4096));
        assert!(!web_memory_mib_is_supported(0));
        assert!(!web_memory_mib_is_supported(4097));
        assert!(web_can_edit_memory(false));
        assert!(!web_can_edit_memory(true));
    }

    #[test]
    fn web_upload_recreates_existing_browser_vm() {
        assert!(!web_upload_replaces_browser_vm(false));
        assert!(web_upload_replaces_browser_vm(true));
    }

    #[test]
    fn web_boot_media_labels_are_os_neutral() {
        assert_eq!(WEB_BOOT_MEDIA_ACTION_LABEL, "Boot OS Image");
        assert!(!WEB_BOOT_MEDIA_ACTION_LABEL.contains("Alpine"));
        assert!(!WEB_BOOT_MEDIA_ACTION_DESCRIPTION.contains("Alpine"));
    }

    #[test]
    fn web_emulator_frame_respects_wall_clock_budget() {
        assert!(WEB_FRAME_TIME_BUDGET_MS <= 8);
        assert!(WEB_BATCH_SIZE <= 1_000);
        assert!(web_should_continue_emulator_frame(
            0,
            core::time::Duration::from_millis(0)
        ));
        assert!(!web_should_continue_emulator_frame(
            0,
            core::time::Duration::from_millis(WEB_FRAME_TIME_BUDGET_MS + 1)
        ));
        assert!(!web_should_continue_emulator_frame(
            WEB_FRAME_BUDGET,
            core::time::Duration::from_millis(0)
        ));
    }

    #[test]
    fn web_uploaded_media_startup_is_split_across_frames() {
        assert_eq!(WEB_STARTUP_STEPS_PER_FRAME, 1);
        assert_eq!(
            web_next_startup_stage(WebStartupStage::CreateEmulator),
            Some(WebStartupStage::InitializeMemory)
        );
        assert_eq!(web_next_startup_stage(WebStartupStage::StartEmulator), None);
        assert_eq!(
            web_startup_stage_label(WebStartupStage::CreateEmulator),
            "Allocating guest memory"
        );
        assert_eq!(
            web_startup_stage_label(WebStartupStage::InitializeMemory),
            "Allocating guest memory"
        );
    }

    #[test]
    fn web_console_prefers_startup_message_over_stale_texture() {
        assert_eq!(
            web_console_surface(false, true, true, false),
            WebConsoleSurface::Starting
        );
    }

    #[test]
    fn web_status_reports_starting_while_upload_boots() {
        assert_eq!(
            web_runtime_state(false, true, false, false),
            WebRuntimeState::Starting
        );
    }

    #[test]
    fn web_emulator_pump_yields_to_input_frames() {
        assert!(web_should_pump_emulator_this_frame(false, false));
        assert!(!web_should_pump_emulator_this_frame(true, false));
        assert!(!web_should_pump_emulator_this_frame(false, true));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_settings_update_memory_and_ips_before_start() {
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.memory_mib = 512;
        settings.ips = 123_000_000;

        settings.apply_to_config(&mut config).unwrap();
        assert_eq!(config.memory_mib, 512);
        assert_eq!(config.host_memory_mib, 256);
        assert_eq!(config.ips, 123_000_000);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_vm_settings_attach_disk_from_none() {
        let disk = unique_temp_path("rusty-box-gui-settings-disk");
        write_test_disk(&disk);
        let mut config = test_resolved_config();
        config.disk = None;
        let mut settings = NativeVmSettings::from_config(&config);
        settings.disk_enabled = true;
        settings.disk_path = disk.display().to_string();
        settings.disk_channel = 1;
        settings.disk_drive = 1;
        settings.boot_device = crate::args::BootDevice::Disk;

        settings.apply_to_config(&mut config).unwrap();

        let attached = config.disk.as_ref().expect("disk should attach");
        assert_eq!(attached.path, disk);
        assert_eq!(attached.channel, 1);
        assert_eq!(attached.drive, 1);
        assert_eq!(attached.geometry.cylinders, 1);
        assert_eq!(config.boot_order[0], crate::args::BootDevice::Disk);
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_vm_settings_detach_disk_and_cdrom() {
        let disk = unique_temp_path("rusty-box-gui-settings-detach");
        write_test_disk(&disk);
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.disk_enabled = true;
        settings.disk_path = disk.display().to_string();
        settings.apply_to_config(&mut config).unwrap();

        settings = NativeVmSettings::from_config(&config);
        settings.disk_enabled = false;
        settings.cdrom_enabled = true;
        settings.apply_to_config(&mut config).unwrap();
        assert!(config.disk.is_none());
        assert!(config.cdrom.is_some());

        settings = NativeVmSettings::from_config(&config);
        settings.disk_enabled = true;
        settings.disk_path = disk.display().to_string();
        settings.cdrom_enabled = false;
        settings.boot_device = crate::args::BootDevice::Disk;
        settings.apply_to_config(&mut config).unwrap();
        assert!(config.disk.is_some());
        assert!(config.cdrom.is_none());
        assert_eq!(config.boot_order, vec![crate::args::BootDevice::Disk]);
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_vm_settings_preserve_startup_disk_creation() {
        let mut config = test_resolved_config();
        let disk_path = unique_temp_path("rusty-box-gui-created-settings");
        config.disk = Some(crate::config::ResolvedDisk {
            path: disk_path.clone(),
            geometry: crate::args::DiskGeometry {
                cylinders: 20,
                heads: 16,
                sectors_per_track: 63,
            },
            channel: 0,
            drive: 0,
            creation: Some(crate::config::ResolvedDiskCreation {
                path: disk_path.clone(),
                size: rusty_box_bximage::ImageSize::mib(10),
                overwrite: false,
            }),
        });

        let settings = NativeVmSettings::from_config(&config);
        settings.apply_to_config(&mut config).unwrap();

        let disk = config.disk.expect("created disk should remain attached");
        assert_eq!(disk.path, disk_path);
        assert!(disk.creation.is_some());
        assert_eq!(disk.geometry.cylinders, 20);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_vm_settings_reject_enabled_blank_media_paths() {
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.disk_enabled = true;
        settings.disk_path.clear();
        assert_eq!(
            settings.apply_to_config(&mut config),
            Err("Hard disk path is required when hard disk is enabled".to_owned())
        );

        settings.disk_enabled = false;
        settings.cdrom_enabled = true;
        settings.cdrom_path.clear();
        assert_eq!(
            settings.apply_to_config(&mut config),
            Err("CD/DVD path is required when CD/DVD is enabled".to_owned())
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_vm_settings_require_bios_and_clamp_numbers() {
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.bios_path.clear();
        assert_eq!(
            settings.apply_to_config(&mut config),
            Err("BIOS path is required".to_owned())
        );

        settings.bios_path = "bios.bin".to_owned();
        settings.memory_mib = 0;
        settings.host_memory_mib = 0;
        settings.memory_block_kib = 0;
        settings.ips = 0;
        settings.max_instructions = 0;
        settings.vga_bios_path.clear();
        settings.apply_to_config(&mut config).unwrap();

        assert_eq!(config.memory_mib, 1);
        assert_eq!(config.host_memory_mib, 1);
        assert_eq!(config.memory_block_kib, 1);
        assert_eq!(config.ips, 1);
        assert_eq!(config.max_instructions, u64::MAX);
        assert!(config.vga_bios.is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_start_vm_sends_selected_config_when_stopped() {
        let (mut app, command_rx) = native_test_app();
        app.settings.memory_mib = 640;
        app.settings.ips = 123_000_000;
        app.settings.cdrom_path = "install.iso".to_owned();

        app.start_vm();

        let command = command_rx.try_recv().expect("start command should be sent");
        let NativeEmulatorCommand::Start(config) = command;
        assert_eq!(config.memory_mib, 640);
        assert_eq!(config.host_memory_mib, 256);
        assert_eq!(config.ips, 123_000_000);
        assert_eq!(
            config.cdrom.as_ref().map(|cdrom| cdrom.path.as_path()),
            Some(std::path::Path::new("install.iso"))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_start_vm_ignores_duplicate_start_while_pending() {
        let (mut app, command_rx) = native_test_app();

        app.start_vm();
        app.start_vm();

        command_rx
            .try_recv()
            .expect("first start command should be sent");
        assert!(
            command_rx.try_recv().is_err(),
            "second start command should be suppressed while first launch is pending"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_start_vm_reports_disconnected_worker_on_shell() {
        let (mut app, command_rx) = native_test_app();
        drop(command_rx);
        app.disk_creator.status = Some(CreatorStatus::Success("existing status".to_owned()));

        app.start_vm();

        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::error(
                "Emulator worker is not available. Restart the application."
            ))
        );
        assert_eq!(
            app.disk_creator.status,
            Some(CreatorStatus::Success("existing status".to_owned()))
        );
        assert!(!app.shared.lock().unwrap().start_pending);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_runtime_error_becomes_shell_notice() {
        let (mut app, _command_rx) = native_test_app();
        app.shared.lock().unwrap().runtime_error =
            Some("Emulator startup failed: BIOS missing".to_owned());

        app.take_runtime_error_notice();

        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::error("Emulator startup failed: BIOS missing"))
        );
        assert!(app.shared.lock().unwrap().runtime_error.is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_power_controls_do_not_request_stop_when_stopped() {
        let (mut app, _command_rx) = native_test_app();

        app.request_power_off();
        app.request_reset();

        let display = app
            .shared
            .lock()
            .expect("shared display should not be poisoned");
        assert!(!display.stop_flag.load(Ordering::Relaxed));
        assert!(!display.reset_requested);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_profile_duplicate_keeps_independent_settings() {
        let config = test_resolved_config();
        let mut profile = NativeVmProfile::from_config("Base", config);
        profile.settings.memory_mib = 384;

        let mut copy = profile.duplicate("Second VM");
        copy.settings.memory_mib = 768;

        assert_eq!(profile.name, "Base");
        assert_eq!(profile.settings.memory_mib, 384);
        assert_eq!(copy.name, "Second VM");
        assert_eq!(copy.settings.memory_mib, 768);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_profile_duplicate_select_delete_rename_refreshes_metadata() {
        let (mut app, _command_rx) = native_test_app();
        app.profiles[0].name = "Base VM".to_owned();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();

        app.duplicate_selected_profile();

        assert_eq!(app.profiles.len(), 2);
        assert_eq!(app.chrome.selected_vm, 1);
        assert_eq!(app.chrome.vm_library[1].memory, "512 MB");
        app.profiles[1].name = "Copy VM".to_owned();
        app.apply_pending_settings().unwrap();
        assert_eq!(app.vm_info.name, "Copy VM");
        assert_eq!(app.chrome.vm_library[1].name, "Copy VM");

        app.delete_selected_profile();

        assert_eq!(app.profiles.len(), 1);
        assert_eq!(app.chrome.selected_vm, 0);
        assert_eq!(app.vm_info.name, "Base VM");
        assert_eq!(app.chrome.vm_library[0].name, "Base VM");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_profile_delete_requires_stopped_multiple_profiles() {
        let (mut app, _command_rx) = native_test_app();

        app.delete_selected_profile();

        assert_eq!(app.profiles.len(), 1);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning("At least one VM profile is required."))
        );

        app.duplicate_selected_profile();
        app.shared.lock().unwrap().emu_running = true;
        app.delete_selected_profile();

        assert_eq!(app.profiles.len(), 2);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Stop the running VM before deleting profiles."
            ))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_profile_selection_refuses_while_running() {
        let (mut app, _command_rx) = native_test_app();
        app.duplicate_selected_profile();
        assert_eq!(app.chrome.selected_vm, 1);
        app.shared.lock().unwrap().emu_running = true;

        app.select_profile(0);

        assert_eq!(app.chrome.selected_vm, 1);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Stop the running VM before selecting another profile."
            ))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn created_image_hard_disk_attaches_to_stopped_profile() {
        let (mut app, _command_rx) = native_test_app();
        let disk = unique_temp_path("rusty-box-gui-created-attach");
        write_test_disk(&disk);

        app.handle_created_image(CreatedImage {
            path: disk.clone(),
            kind: CreatedImageKind::HardDisk,
        });

        assert!(app.settings.disk_enabled);
        assert_eq!(app.settings.disk_path, disk.display().to_string());
        assert_eq!(
            app.config.disk.as_ref().map(|disk| disk.path.as_path()),
            Some(disk.as_path())
        );
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::info(
                "Attached created disk image to Rusty Box."
            ))
        );
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn created_image_hard_disk_warns_while_running() {
        let (mut app, _command_rx) = native_test_app();
        let original_disk_path = app.settings.disk_path.clone();
        app.shared.lock().unwrap().emu_running = true;

        app.handle_created_image(CreatedImage {
            path: std::path::PathBuf::from("created.img"),
            kind: CreatedImageKind::HardDisk,
        });

        assert_eq!(app.settings.disk_path, original_disk_path);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Disk image created. Stop the VM before attaching it."
            ))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn created_image_floppy_reports_unwired_notice() {
        let (mut app, _command_rx) = native_test_app();

        app.handle_created_image(CreatedImage {
            path: std::path::PathBuf::from("floppy.img"),
            kind: CreatedImageKind::Floppy,
        });

        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::info(
                "Floppy image created. Floppy drive emulation is not wired yet."
            ))
        );
        assert!(app.config.disk.is_none());
    }

    #[test]
    fn vm_info_formats_missing_media_as_none() {
        assert_eq!(format_path_for_summary(None), "None");
    }

    #[test]
    fn disk_creator_default_filenames_match_kind() {
        let mut panel = DiskCreatorPanel::default();
        panel.kind = CreatorKind::HardDisk;
        assert_eq!(panel.default_image_filename(), "c.img");
        panel.kind = CreatorKind::Floppy;
        assert_eq!(panel.default_image_filename(), "floppy.img");
    }

    #[test]
    fn hard_disk_panel_creates_vmware_like_default() {
        let path = unique_temp_path("rusty-box-gui-panel-hard-disk");
        let mut panel = DiskCreatorPanel::default();
        panel.path = path.display().to_string();
        panel.hard_disk_size = "10M".to_owned();

        panel.create_image();

        assert_eq!(fs::metadata(&path).unwrap().len(), 10_321_920);
        assert!(matches!(panel.status, Some(CreatorStatus::Success(_))));
        remove_test_file(&path);
    }

    #[test]
    fn hard_disk_panel_rejects_non_integer_size() {
        let mut panel = DiskCreatorPanel::default();
        panel.hard_disk_size = "ten".to_owned();

        panel.create_image();

        assert_eq!(
            panel.status,
            Some(CreatorStatus::Error(
                "invalid disk size 'ten'; use whole-number sizes like 20G or 512M".to_owned()
            ))
        );
    }

    #[test]
    fn hard_disk_panel_rejects_too_large_for_gui_attach_limit() {
        let path = unique_temp_path("rusty-box-gui-panel-huge-disk");
        let mut panel = DiskCreatorPanel::default();
        panel.path = path.display().to_string();
        panel.hard_disk_size = "32G".to_owned();

        panel.create_image();

        assert_eq!(
            panel.status,
            Some(CreatorStatus::Error(
                "disk is too large for the current Rusty Box BIOS geometry limit; choose 31 GiB or smaller"
                    .to_owned()
            ))
        );
        assert!(fs::metadata(&path).is_err());
    }

    #[test]
    fn floppy_panel_creates_144m_image() {
        let path = unique_temp_path("rusty-box-gui-panel-floppy");
        let mut panel = DiskCreatorPanel::default();
        panel.kind = CreatorKind::Floppy;
        panel.path = path.display().to_string();
        panel.floppy_format = FloppyFormat::M1_44;

        panel.create_image();

        assert_eq!(fs::metadata(&path).unwrap().len(), 1_474_560);
        assert!(matches!(panel.status, Some(CreatorStatus::Success(_))));
        remove_test_file(&path);
    }

    #[test]
    fn hard_disk_panel_accepts_human_size_suffix() {
        let path = unique_temp_path("rusty-box-gui-human-size");
        let mut panel = DiskCreatorPanel::default();
        panel.path = path.display().to_string();
        panel.hard_disk_size = "10M".to_owned();
        panel.create_image();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 10_321_920);
        remove_test_file(&path);
    }

    #[test]
    fn hard_disk_panel_rejects_existing_without_overwrite() {
        let path = unique_temp_path("rusty-box-gui-existing");
        std::fs::write(&path, b"already here").unwrap();
        let mut panel = DiskCreatorPanel::default();
        panel.path = path.display().to_string();
        panel.hard_disk_size = "10M".to_owned();
        panel.overwrite = false;
        panel.create_image();
        assert!(
            matches!(&panel.status, Some(CreatorStatus::Error(msg)) if msg.contains("already exists"))
        );
        remove_test_file(&path);
    }

    #[test]
    fn floppy_panel_rejects_existing_without_overwrite() {
        let path = unique_temp_path("rusty-box-gui-existing-floppy");
        std::fs::write(&path, b"already here").unwrap();
        let mut panel = DiskCreatorPanel::default();
        panel.kind = CreatorKind::Floppy;
        panel.path = path.display().to_string();
        panel.overwrite = false;
        panel.create_image();
        assert!(
            matches!(&panel.status, Some(CreatorStatus::Error(msg)) if msg.contains("already exists"))
        );
        remove_test_file(&path);
    }
}
