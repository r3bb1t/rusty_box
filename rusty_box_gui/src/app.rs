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

#[cfg(not(target_arch = "wasm32"))]
use crate::shell::destination::{SidebarAction, VmBarAction};
use crate::shell::destination::{Destination, ShellPage};
use crate::shell::sidebar::VmLibraryEntry;
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::theme::{SPACE_ITEM, STROKE_HAIRLINE, TEXT_CAPTION, TEXT_DISPLAY};
use crate::shell::theme::{
    configure_shell_style, shell_card_frame, ACCENT_AMBER, ACCENT_BLUE, ACCENT_CYAN, ACCENT_RED,
    BG_BASE, BG_PANEL, SPACE_GROUP, SPACE_PAGE, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
    TEXT_TITLE,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::vm_bar::VmBarState;
#[cfg(target_arch = "wasm32")]
use crate::shell::widgets::disabled_tile;
#[cfg(target_os = "android")]
use crate::shell::widgets::hairline_below;
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::widgets::{
    action_tile_enabled, hairline_above, home_fact, path_field_width, selection_row, status_text,
    RowMark, ShellStateBadge, BROWSE, HOME_FACT_GAP, ROOT_INDENT,
};
use crate::shell::widgets::{
    action_tile, field_row, metadata_text, page_header, primary_button, status_dot,
    ActionTileWeight,
};
#[cfg(target_arch = "wasm32")]
use egui::Color32;
use egui::RichText;
#[cfg(target_os = "android")]
use egui::Stroke;
// The wasm build drives the machine directly from the frame loop below; the
// native build hands it to a runner thread instead.
#[cfg(target_arch = "wasm32")]
use rusty_box::emulator::RunBudget;
use rusty_box::params::{
    BxParams, BX_CPU_CORES_LIMIT, BX_CPU_HT_THREADS_LIMIT, BX_CPU_PROCESSORS_LIMIT,
    BX_MAX_SMP_THREADS_SUPPORTED,
};
use rusty_box_bximage::{
    calculate_hard_disk_geometry, CreatedImage as BxCreatedImage, FloppyFormat, ImageSize,
    SectorSize,
};
#[cfg(not(target_arch = "wasm32"))]
use rusty_box_bximage::{create_flat_hard_disk, create_floppy, ExistingFilePolicy};

/// Common pre-boot VBE resolutions offered by the Display panel picker.
const VGA_MODE_PRESETS: &[(u16, u16)] = &[
    (1024, 768),
    (1280, 720),
    (1280, 1024),
    (1600, 1200),
    (1920, 1080),
];

fn vga_mode_label(mode: Option<crate::config::VgaMode>) -> String {
    match mode {
        None => "Default (VGA / VBE)".to_owned(),
        Some(mode) => format!("{}×{} @ {}bpp", mode.width, mode.height, mode.bpp),
    }
}

/// The device list takes a fixed column; the detail card takes the rest.
/// A card's width is the layout's decision, never the card's own.
#[cfg(not(target_arch = "wasm32"))]
const HARDWARE_LIST_WIDTH: f32 = 150.0;

#[cfg(target_arch = "wasm32")]
const BROWSER_MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
pub enum NativeEmulatorCommand {
    Start(crate::config::ResolvedConfig),
}

/// A path field the shell fills from a file chooser.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowseTarget {
    /// The hard disk image attached at the next power-on.
    HardDisk,
    /// The CD/DVD image attached at the next power-on.
    Cdrom,
    /// The system BIOS ROM.
    Bios,
    /// The VGA BIOS ROM.
    VgaBios,
    /// Where the Images page creates its next image.
    NewImage,
}

/// A Browse press the Android host answers with a file browser of its own.
#[cfg(target_os = "android")]
pub(crate) struct BrowseRequest {
    pub(crate) target: BrowseTarget,
    /// What the field holds now, so the browser can open beside it.
    pub(crate) current: PathBuf,
    /// The name offered for a file still to be created; `None` when an
    /// existing file is chosen.
    pub(crate) save_name: Option<&'static str>,
}

/// Ordered by gravity: an `Error` outranks a `Warning`, which outranks
/// `Info`. `NativeShellApp::notify` keeps the gravest of one frame.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// Whether a phone draws a notice of `kind`: the ones that report something
/// that did not happen, in full or in part — a save that did not reach its
/// file, a launch whose first VM was not finished, a power-on that stopped.
/// A phone has no room for the ones that report something that did.
#[cfg(not(target_arch = "wasm32"))]
fn phone_shows_notice(kind: ShellNoticeKind) -> bool {
    match kind {
        ShellNoticeKind::Info => false,
        ShellNoticeKind::Warning | ShellNoticeKind::Error => true,
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn pick_native_file() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_file()
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn save_native_file(default_name: &'static str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_file_name(default_name)
        .save_file()
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
    /// The folder every library VM's edits are written to.
    library: crate::library::VmLibrary,
    /// Library files that do not load, listed under "Could not load".
    broken_files: Vec<crate::library::BrokenVmFile>,
    /// A destructive step the user has yet to confirm or cancel.
    pending_confirm: Option<PendingConfirm>,
    /// The overwrite creations settled this session, by path: each file the
    /// user agreed to let a startup-disk creation erase, and each one that
    /// did not exist when its VM was powered on, so there was nothing to
    /// erase and nothing to ask. The runner erases an overwrite creation's
    /// file at the first power-on of the session that uses the path and at
    /// no later one, and provisions a plain creation at every power-on
    /// without erasing anything, so one answer per file covers the session.
    overwrite_confirmed: std::collections::HashSet<PathBuf>,
    /// The gravest notice raised in this frame, which a lesser one may not
    /// replace; see `notify`.
    gravest_raised: Option<ShellNoticeKind>,
    /// A Browse press the Android host has yet to answer.
    #[cfg(target_os = "android")]
    browse_request: Option<BrowseTarget>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub(crate) struct NativeVmInfo {
    pub name: String,
    pub memory_mib: u32,
    #[allow(dead_code)]
    pub ips: u32,
    pub cpus: u32,
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
            cpus: config.cpu_params.cpu_count(),
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
    cpu_sockets: u32,
    cpu_cores: u32,
    cpu_threads: u32,
    boot_order: Vec<crate::args::BootDevice>,
    pci: bool,
    sync_slowdown: bool,
    /// Which engine retires the guest's instructions.
    engine: crate::config::Engine,
    max_instructions: u64,
    log_level: crate::args::LogLevel,
    bios_path: String,
    vga_bios_path: String,
    disk_enabled: bool,
    disk_path: String,
    disk_channel: usize,
    disk_drive: usize,
    disk_chs_override: Option<crate::args::DiskGeometry>,
    disk_creation: Option<crate::config::ResolvedDiskCreation>,
    cdrom_enabled: bool,
    cdrom_path: String,
    cdrom_channel: usize,
    cdrom_drive: usize,
    /// Optional pre-boot VBE display mode; `None` keeps the VGA defaults.
    vga_mode: Option<crate::config::VgaMode>,
    /// Register the VGA on PCI (experimental KMS / bochs-drm path).
    pci_vga: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeVmSettings {
    fn from_config(config: &crate::config::ResolvedConfig) -> Self {
        let disk = config.disk.as_ref();
        let cdrom = config.cdrom.as_ref();
        let topology = config.cpu_params.cpu_topology();
        Self {
            memory_mib: config.memory_mib,
            host_memory_mib: config.host_memory_mib,
            memory_block_kib: config.memory_block_kib,
            ips: config.ips,
            cpu_sockets: topology.n_processors(),
            cpu_cores: topology.n_cores(),
            cpu_threads: topology.n_threads(),
            boot_order: config.boot_order.clone(),
            pci: config.pci,
            sync_slowdown: config.sync_slowdown,
            engine: config.engine,
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
            // The geometry the configuration already resolved, carried rather
            // than re-derived. Detecting it a second time here is a second
            // answer for the same disk, and the two disagree whenever the
            // image's sector count is not a multiple of the detected heads
            // times sectors-per-track: the tail of the image becomes
            // unreachable, which is where a boot loader keeps its map.
            disk_chs_override: disk.map(|disk| disk.geometry),
            disk_creation: disk.and_then(|disk| disk.creation.clone()),
            cdrom_enabled: cdrom.is_some(),
            cdrom_path: cdrom.map_or_else(String::new, |cdrom| cdrom.path.display().to_string()),
            cdrom_channel: cdrom.map_or(1, |cdrom| cdrom.channel),
            cdrom_drive: cdrom.map_or(0, |cdrom| cdrom.drive),
            vga_mode: config.vga_mode,
            pci_vga: config.pci_vga,
        }
    }

    fn apply_to_config(&self, config: &mut crate::config::ResolvedConfig) -> Result<(), String> {
        config.memory_mib = self.memory_mib.max(1);
        config.host_memory_mib = self.host_memory_mib.max(1);
        config.memory_block_kib = self.memory_block_kib.max(1);
        config.ips = self.ips.max(1);
        config.cpu_params = BxParams::default()
            .with_topology(
                self.cpu_sockets.max(1),
                self.cpu_cores.max(1),
                self.cpu_threads.max(1),
            )
            .map_err(|error| {
                format!(
                    "Invalid CPU topology: {}",
                    crate::config::topology_error_message(error)
                )
            })?;
        config.pci = self.pci;
        config.sync_slowdown = self.sync_slowdown;
        config.engine = self.engine;
        config.max_instructions = if self.max_instructions == 0 {
            u64::MAX
        } else {
            self.max_instructions
        };
        config.log_level = self.log_level;

        // A blank BIOS path applies as none, as the blank "New VM" has it:
        // the VM can be edited and kept without one, and `start_vm` refuses
        // to power it on until it has one.
        config.bios = trimmed_optional_path(&self.bios_path).unwrap_or_default();
        config.vga_bios = trimmed_optional_path(&self.vga_bios_path);

        if self.disk_enabled {
            let path = trimmed_optional_path(&self.disk_path)
                .ok_or_else(|| "Hard disk path is required when hard disk is enabled".to_owned())?;
            let creation = self
                .disk_creation
                .as_ref()
                .filter(|creation| creation.path == path)
                .cloned();
            let geometry = if let Some(override_chs) = self.disk_chs_override {
                if override_chs.cylinders == 0
                    || override_chs.heads == 0
                    || override_chs.sectors_per_track == 0
                {
                    return Err("Overridden disk CHS values must be non-zero".to_owned());
                }
                override_chs
            } else if let Some(creation) = &creation {
                let geometry = calculate_hard_disk_geometry(creation.size, SectorSize::Bytes512)
                    .map_err(|error| format!("Failed to inspect hard disk: {error}"))?;
                // `calculate_hard_disk_geometry` already caps cylinders at BOCHS_MAX_CYLINDERS
                // (2^24), which fits the u32 geometry field — no extra limit needed.
                crate::args::DiskGeometry {
                    cylinders: geometry.cylinders as u32,
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

        config.vga_mode = self.vga_mode;
        config.pci_vga = self.pci_vga;

        config.boot_order = self.boot_order_for_attached_media();
        Ok(())
    }

    /// The chosen boot order over the attached media. Empty when nothing is
    /// attached, as the resolver has it for the egui shell
    /// (`config::allow_empty_boot_order`): the VM can be edited and kept, and
    /// `start_vm` refuses to power it on until a medium is attached.
    fn boot_order_for_attached_media(&self) -> Vec<crate::args::BootDevice> {
        let mut order = Vec::with_capacity(2);
        // Keep the user's chosen order, dropping unattached or duplicate devices.
        for device in &self.boot_order {
            if self.is_boot_device_attached(*device) && !order.contains(device) {
                order.push(*device);
            }
        }
        // Append any attached device the list didn't mention, so freshly enabled
        // media stays bootable without forcing the user to edit the order first.
        for device in [
            crate::args::BootDevice::Disk,
            crate::args::BootDevice::Cdrom,
        ] {
            if self.is_boot_device_attached(device) && !order.contains(&device) {
                order.push(device);
            }
        }
        order
    }

    fn is_boot_device_attached(&self, device: crate::args::BootDevice) -> bool {
        match device {
            crate::args::BootDevice::Disk => self.disk_enabled,
            crate::args::BootDevice::Cdrom => self.cdrom_enabled,
        }
    }
}

/// Where a VM in the list came from, and so where its edits go.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum VmOrigin {
    /// A library file; every applied edit is written back to it.
    Library(crate::library::VmStem),
    /// In memory only — the command line's machine, or the blank "New VM" —
    /// until the user keeps it in the library.
    Launch,
}

/// Whether a library VM's file holds the VM as it is.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveState {
    /// The file holds the VM as it is. A VM without a file is always `Saved`:
    /// it has nothing to write, and only a library VM is ever marked.
    Saved,
    /// An applied edit is not in the file yet; the next flush, from an idle
    /// frame or from an action, writes it.
    Unsaved,
    /// The last write failed. Only an action's flush — a selection, `+`,
    /// Keep, power-on, exit — tries again, or the next applied edit, which
    /// marks the VM `Unsaved`; an idle frame does not, so a lasting fault is
    /// not retried on every frame.
    WriteFailed,
}

/// What brought a VM's name or config to where it is, for
/// `NativeVmProfile::mark_against_file`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigChange {
    /// An edit the user applied. A VM whose last write failed is `Unsaved`
    /// again, so the next flush tries its file once more.
    Edit,
    /// The shell reading the VM's settings into its config, as a selection
    /// does. A write that failed stays failed, for an action to try again.
    Reading,
}

/// Which of the VMs whose files are behind a flush writes.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlushScope {
    /// `Unsaved` VMs only: an idle frame's flush.
    Unsaved,
    /// `Unsaved` and `WriteFailed` VMs: an action's flush, where a failed
    /// write is tried again.
    UnsavedAndFailed,
}

/// What a library VM's file holds, as last read from or written to it.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct VmFileContents {
    name: String,
    config: crate::config::ResolvedConfig,
}

#[cfg(not(target_arch = "wasm32"))]
impl VmFileContents {
    fn of(name: &str, config: &crate::config::ResolvedConfig) -> Self {
        Self {
            name: name.to_owned(),
            config: config.clone(),
        }
    }

    fn holds(&self, name: &str, config: &crate::config::ResolvedConfig) -> bool {
        self.name == name && &self.config == config
    }
}

/// A destructive step waiting for the user to confirm it in a dialog.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingConfirm {
    /// Deleting the VM that was selected when the delete was asked for.
    /// `index` and `origin` identify it, so a confirm that finds another VM
    /// selected deletes nothing; `name` is what the dialog calls it. A library
    /// VM's file goes with it.
    DeleteVm {
        index: usize,
        origin: VmOrigin,
        name: String,
    },
    /// Deleting a library file that does not load.
    DeleteBroken(PathBuf),
    /// Powering on the VM at `index` with `origin`, whose startup-disk
    /// creation erases the existing file `path`. `index` and `origin`
    /// identify it, so a confirm that finds another VM selected starts
    /// nothing.
    OverwriteDisk {
        index: usize,
        origin: VmOrigin,
        path: PathBuf,
    },
}

/// What a VM lacks to be powered on. A VM without a BIOS path or a medium to
/// boot from is still one the shell edits and keeps — the blank "New VM", a
/// phone's first VM, a bundled VM that names only its ROMs — so completeness
/// is checked here, at power-on, and nowhere earlier.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerOnGap {
    Bios,
    Media,
    BiosAndMedia,
}

#[cfg(not(target_arch = "wasm32"))]
impl PowerOnGap {
    /// What `config` lacks to be powered on; `None` when it has a BIOS path
    /// and a hard disk or CD/DVD attached.
    fn of(config: &crate::config::ResolvedConfig) -> Option<Self> {
        let no_bios = config.bios.as_os_str().is_empty();
        let no_media = config.disk.is_none() && config.cdrom.is_none();
        match (no_bios, no_media) {
            (false, false) => None,
            (true, false) => Some(Self::Bios),
            (false, true) => Some(Self::Media),
            (true, true) => Some(Self::BiosAndMedia),
        }
    }

    /// The notice that refuses the power-on, naming where each missing
    /// setting is made; the BIOS half points where `RunError::MissingBios`
    /// does.
    fn notice(self) -> &'static str {
        match self {
            Self::Bios => "Set a BIOS path under Hardware › Display before powering on.",
            Self::Media => "Attach a hard disk or CD/DVD before powering on.",
            Self::BiosAndMedia => {
                "Set a BIOS path under Hardware › Display and attach a hard disk or CD/DVD \
                 before powering on."
            }
        }
    }
}

/// What a confirmation dialog says.
#[cfg(not(target_arch = "wasm32"))]
struct ConfirmWording {
    title: String,
    body: String,
    verb: &'static str,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeVmProfile {
    name: String,
    config: crate::config::ResolvedConfig,
    settings: NativeVmSettings,
    origin: VmOrigin,
    /// What the VM's file holds, as last read from or written to it; `None`
    /// for a VM with no file. `save_state` is `Saved` exactly when this holds
    /// `name` and `config`, which `apply_pending_settings` and the flush keep
    /// true.
    file: Option<VmFileContents>,
    save_state: SaveState,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeVmProfile {
    /// The VM `config` describes under `name`. A library VM's file holds
    /// what was read from it: `config` under `name`.
    fn from_config(
        name: impl Into<String>,
        config: crate::config::ResolvedConfig,
        origin: VmOrigin,
    ) -> Self {
        let name = name.into();
        let settings = NativeVmSettings::from_config(&config);
        let file = match origin {
            VmOrigin::Library(_) => Some(VmFileContents::of(&name, &config)),
            VmOrigin::Launch => None,
        };
        Self {
            name,
            config,
            settings,
            origin,
            file,
            save_state: SaveState::Saved,
        }
    }

    /// A copy of this VM under `name`, kept where `origin` says. A library
    /// copy's file holds this VM's config under that name:
    /// `add_vm_copying_selected` writes it before making the copy.
    fn duplicate(&self, name: impl Into<String>, origin: VmOrigin) -> Self {
        let name = name.into();
        let file = match origin {
            VmOrigin::Library(_) => Some(VmFileContents::of(&name, &self.config)),
            VmOrigin::Launch => None,
        };
        Self {
            name,
            origin,
            file,
            save_state: SaveState::Saved,
            ..self.clone()
        }
    }

    /// Whether the VM's file, as last known, holds the VM as it is now. A VM
    /// with no file holds nothing.
    fn file_is_current(&self) -> bool {
        self.file
            .as_ref()
            .is_some_and(|file| file.holds(&self.name, &self.config))
    }

    /// Applies the VM's settings to its config, or leaves the config as it
    /// was when they do not apply. A file that held the VM before holds it
    /// after: the settings are applied to the config the file gave, so what
    /// they change is how the shell reads that file — a boot order put in
    /// the shell's own form, say — not what the VM is, and nothing is
    /// written for it.
    fn apply_settings(&mut self) -> Result<(), String> {
        let mut config = self.config.clone();
        self.settings.apply_to_config(&mut config)?;
        let file_was_current = self.file_is_current();
        self.config = config;
        if file_was_current {
            self.file = Some(VmFileContents::of(&self.name, &self.config));
        }
        Ok(())
    }

    /// Marks a library VM after `change`: `Saved` when its file holds it as
    /// it is, and otherwise `Unsaved` — except that a reading leaves a failed
    /// write failed, since only an action or an applied edit tries it again.
    /// A VM with no file has nothing to mark. Every site that changes a VM's
    /// name or config keeps the `save_state` invariant through this one
    /// match.
    fn mark_against_file(&mut self, change: ConfigChange) {
        if let VmOrigin::Library(_) = self.origin {
            self.save_state = match (self.file_is_current(), change, self.save_state) {
                (true, _, _) => SaveState::Saved,
                (false, ConfigChange::Edit, _) => SaveState::Unsaved,
                (false, ConfigChange::Reading, SaveState::WriteFailed) => SaveState::WriteFailed,
                (false, ConfigChange::Reading, SaveState::Saved | SaveState::Unsaved) => {
                    SaveState::Unsaved
                }
            };
        }
    }

    fn vm_info(&self) -> NativeVmInfo {
        NativeVmInfo::from_config_named(&self.config, &self.name)
    }

    /// The sidebar's row for this VM: a temporary VM is marked as unsaved,
    /// and a library VM whose last write failed as not saved.
    fn library_entry(&self) -> VmLibraryEntry {
        let info = self.vm_info();
        let entry = VmLibraryEntry::new(
            &info.name,
            &info.boot,
            format!("{} MB", info.memory_mib),
            format_path_for_summary(info.disk.as_deref()),
            format_path_for_summary(info.cdrom.as_deref()),
        );
        match (&self.origin, self.save_state) {
            (VmOrigin::Launch, _) => entry.unsaved(),
            (VmOrigin::Library(_), SaveState::WriteFailed) => entry.write_failed(),
            (VmOrigin::Library(_), SaveState::Saved | SaveState::Unsaved) => entry,
        }
    }
}

/// The VM list the shell opens with.
#[cfg(not(target_arch = "wasm32"))]
struct OpeningList {
    /// Never empty.
    profiles: Vec<NativeVmProfile>,
    broken: Vec<crate::library::BrokenVmFile>,
    selected: usize,
    notice: Option<ShellNotice>,
}

#[cfg(not(target_arch = "wasm32"))]
impl OpeningList {
    /// The launch VM first, as a temporary VM, then the library's VMs in the
    /// library's order. Selected is the launch VM when there is one, the
    /// library VM the start names when it names one, and otherwise the VM
    /// shown last. With neither a launch VM nor a library VM, a blank
    /// temporary "New VM", so the shell always has a VM to show. A message
    /// the start carries is shown as a warning; a library that cannot be read
    /// opens as empty, with that error as the notice instead, it being the
    /// more serious of the two.
    fn from_start(start: &crate::runner::ShellStart) -> Self {
        let (contents, notice) = match start.library.load() {
            Ok(contents) => (contents, start.notice.clone().map(ShellNotice::warning)),
            Err(error) => (
                crate::library::LibraryContents::default(),
                Some(ShellNotice::error(error.to_string())),
            ),
        };
        let mut profiles = Vec::new();
        let wanted = match &start.opening {
            crate::runner::ShellOpening::Launch(launch) => {
                profiles.push(NativeVmProfile::from_config(
                    launch.name.clone(),
                    launch.config.clone(),
                    VmOrigin::Launch,
                ));
                None
            }
            crate::runner::ShellOpening::LibraryVm(stem) => Some(stem.clone()),
            crate::runner::ShellOpening::LastShown => start.library.last_selected(),
        };
        let mut selected = 0;
        for vm in contents.vms {
            if wanted.as_ref() == Some(&vm.stem) {
                selected = profiles.len();
            }
            profiles.push(NativeVmProfile::from_config(
                vm.name,
                vm.config,
                VmOrigin::Library(vm.stem),
            ));
        }
        if profiles.is_empty() {
            profiles.push(NativeVmProfile::from_config(
                "New VM",
                crate::config::blank_config(),
                VmOrigin::Launch,
            ));
        }
        Self {
            profiles,
            broken: contents.broken,
            selected,
            notice,
        }
    }
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

#[derive(Debug)]
pub(crate) struct ShellChrome {
    destination: Destination,
    selected_hardware: HardwareDevice,
    vm_library: Vec<VmLibraryEntry>,
    library_filter: String,
    show_serial: bool,
    show_library: bool,
    show_about: bool,
}

impl Default for ShellChrome {
    fn default() -> Self {
        Self {
            destination: Destination::default(),
            selected_hardware: HardwareDevice::Memory,
            vm_library: Vec::new(),
            library_filter: String::new(),
            show_serial: true,
            show_library: true,
            show_about: false,
        }
    }
}

impl ShellChrome {
    pub(crate) fn page(&self) -> ShellPage {
        self.destination.page()
    }

    pub(crate) fn selected_vm(&self) -> usize {
        self.destination.vm()
    }

    /// Moves to a page of the VM already shown.
    pub(crate) fn go_to(&mut self, page: ShellPage) {
        self.destination = self.destination.select_page(page);
    }

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
    /// The page's Browse was pressed; the shell answers it, since only the
    /// shell can reach the platform's file chooser.
    #[cfg(not(target_arch = "wasm32"))]
    browse_requested: bool,
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
            #[cfg(not(target_arch = "wasm32"))]
            browse_requested: false,
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
const WEB_MIN_CPU_COUNT: u32 = 1;
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_DEFAULT_CPU_COUNT: u32 = 1;
#[cfg(any(test, target_arch = "wasm32"))]
const WEB_MAX_CPU_COUNT: u32 = BX_MAX_SMP_THREADS_SUPPORTED;

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

#[cfg(not(target_arch = "wasm32"))]
impl NativeShellApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: Arc<Mutex<rusty_box::gui::shared_display::SharedDisplay>>,
        command_tx: Sender<NativeEmulatorCommand>,
        start: crate::runner::ShellStart,
    ) -> Self {
        configure_shell_style(&cc.egui_ctx);
        let emulator = rusty_box::gui::RustyBoxApp::new(cc, Arc::clone(&shared));
        #[cfg(target_os = "android")]
        let emulator = {
            let mut emulator = emulator;
            emulator.set_fit_to_available(true);
            emulator
        };
        Self::with_emulator(emulator, shared, command_tx, start)
    }

    /// The shell around `emulator`, opened on `start`'s VM list. `new` builds
    /// the emulator view from the window; tests build it without one.
    fn with_emulator(
        emulator: rusty_box::gui::RustyBoxApp,
        shared: Arc<Mutex<rusty_box::gui::shared_display::SharedDisplay>>,
        command_tx: Sender<NativeEmulatorCommand>,
        start: crate::runner::ShellStart,
    ) -> Self {
        let opening = OpeningList::from_start(&start);
        let shown = &opening.profiles[opening.selected];
        let config = shown.config.clone();
        let settings = shown.settings.clone();
        let vm_info = shown.vm_info();
        let mut chrome = ShellChrome::with_library(
            opening
                .profiles
                .iter()
                .map(NativeVmProfile::library_entry)
                .collect(),
        );
        chrome.destination = chrome.destination.select_vm(opening.selected);
        #[cfg(target_os = "android")]
        {
            chrome.show_library = false;
            chrome.show_serial = false;
        }
        Self {
            emulator,
            chrome,
            disk_creator: DiskCreatorPanel::default(),
            profiles: opening.profiles,
            config,
            settings,
            vm_info,
            command_tx,
            shared,
            shell_notice: opening.notice,
            library: start.library,
            broken_files: opening.broken,
            pending_confirm: None,
            overwrite_confirmed: std::collections::HashSet::new(),
            gravest_raised: None,
            #[cfg(target_os = "android")]
            browse_request: None,
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
        if cfg!(target_os = "android") && !phone_shows_notice(notice.kind) {
            return;
        }
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
        ui.add_space(SPACE_GROUP);
    }

    /// Shows `notice`, unless a graver one was raised earlier in the same
    /// frame. One frame runs one user action, so within an action the user
    /// sees the worst that happened: a lesser notice never replaces it.
    fn notify(&mut self, notice: ShellNotice) {
        if self
            .gravest_raised
            .is_some_and(|raised| raised > notice.kind)
        {
            return;
        }
        self.gravest_raised = Some(notice.kind);
        self.shell_notice = Some(notice);
    }

    /// Starts a frame: the last frame's notices no longer outrank new ones.
    fn begin_frame(&mut self) {
        self.gravest_raised = None;
    }

    fn take_runtime_error_notice(&mut self) {
        let runtime_error = self
            .shared
            .lock()
            .ok()
            .and_then(|mut display| display.runtime_error.take());

        if let Some(message) = runtime_error {
            self.notify(ShellNotice::error(message));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn handle_native_dropped_files(&mut self, ctx: &egui::Context) {
        if self.chrome.page() != ShellPage::Images {
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

    /// Draws the VM bar and acts on its click. The bar reports what was asked
    /// for; the verbs that change the machine's state go through the same
    /// methods every other surface uses, so the bar cannot reach a state the
    /// rest of the shell cannot.
    fn draw_vm_bar(&mut self, ui: &mut egui::Ui) {
        let status = self.runtime_status();
        let action = crate::shell::vm_bar::draw_vm_bar(
            ui,
            VmBarState {
                name: &self.vm_info.name,
                badge: shell_state_badge(&status, self.has_error_notice()),
                running: status.running,
                start_pending: status.start_pending,
                on_console: self.chrome.page() == ShellPage::Console,
                serial_shown: self.chrome.show_serial,
                mouse_captured: self.emulator.mouse_captured(),
            },
        );
        match action {
            None => {}
            Some(VmBarAction::ToggleSidebar) => {
                self.chrome.show_library = !self.chrome.show_library;
            }
            Some(VmBarAction::PowerOn) => self.start_vm(),
            Some(VmBarAction::PowerOff) => self.request_power_off(),
            Some(VmBarAction::Restart) => self.request_reset(),
            Some(VmBarAction::ToggleSerial) => {
                self.chrome.show_serial = !self.chrome.show_serial;
            }
            Some(VmBarAction::ToggleMouseCapture) => self.emulator.toggle_mouse_capture(),
            Some(VmBarAction::SendCtrlAltDel) => self.emulator.send_ctrl_alt_del(),
            Some(VmBarAction::ShowAbout) => self.chrome.show_about = true,
            Some(VmBarAction::Quit) => {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Draws the tree and hands its click to `handle_sidebar_action`.
    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let badge = shell_state_badge(&self.runtime_status(), self.has_error_notice());
        let visible = self.chrome.visible_vm_indices();
        let action = crate::shell::sidebar::draw_sidebar(
            ui,
            &self.chrome.vm_library,
            &visible,
            self.chrome.destination,
            &mut self.chrome.library_filter,
            badge,
            &self.broken_files,
        );
        if let Some(action) = action {
            self.handle_sidebar_action(action);
        }
    }

    /// Acts on a click in the tree. A page of the VM already shown is a move;
    /// a different VM is a profile switch, which goes through `select_profile`
    /// so that profile's config and settings are loaded too. Deleting a file
    /// that does not load waits for confirmation like every other delete. On
    /// Android the tree is a drawer over the page, so a pick in it closes it
    /// as well.
    fn handle_sidebar_action(&mut self, action: SidebarAction) {
        match action {
            SidebarAction::NewVm => {
                self.add_vm_copying_selected();
                #[cfg(target_os = "android")]
                {
                    self.chrome.show_library = false;
                }
            }
            SidebarAction::DeleteBroken(index) => {
                if let Some(file) = self.broken_files.get(index) {
                    self.pending_confirm = Some(PendingConfirm::DeleteBroken(file.path.clone()));
                }
            }
            SidebarAction::Select(destination) => {
                if destination.vm() == self.chrome.destination.vm() {
                    self.chrome.go_to(destination.page());
                } else {
                    self.select_profile(destination.vm());
                }
                #[cfg(target_os = "android")]
                {
                    self.chrome.show_library = false;
                }
            }
        }
    }

    /// Whether the shell is currently showing the user an error notice; the
    /// state badge reads it as a fault until the notice is dismissed.
    fn has_error_notice(&self) -> bool {
        self.shell_notice
            .as_ref()
            .is_some_and(|notice| notice.kind == ShellNoticeKind::Error)
    }

    fn draw_status_strip(&mut self, ui: &mut egui::Ui) {
        let strip = egui::Panel::bottom("vm_status_strip")
            .exact_size(28.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if self.shared.is_poisoned() {
                        status_dot(ui, ACCENT_RED);
                        ui.label(status_text("State unavailable").color(ACCENT_RED));
                        return;
                    }

                    let snapshot = self.runtime_status();
                    let badge = shell_state_badge(&snapshot, self.has_error_notice());
                    status_dot(ui, badge.color);
                    ui.label(status_text(badge.label).color(badge.color));
                    ui.separator();
                    ui.label(
                        status_text(engine_label(self.settings.engine)).color(TEXT_PRIMARY),
                    );
                    ui.separator();
                    ui.label(
                        status_text(format!(
                            "{} MB · {}",
                            self.vm_info.memory_mib,
                            cpu_count_label(self.vm_info.cpus)
                        ))
                        .color(TEXT_MUTED),
                    );
                    ui.separator();
                    // A published rate is a fact and reads in the data accent;
                    // an absent one is drawn muted so it cannot pass for zero.
                    let ips_color = if snapshot.ips > 0 {
                        ACCENT_BLUE
                    } else {
                        TEXT_MUTED
                    };
                    ui.label(status_text(format_ips_u32(snapshot.ips)).color(ips_color));
                    if snapshot.reset_requested {
                        ui.separator();
                        ui.label(status_text("Restart queued").color(ACCENT_AMBER));
                    }
                });
            });
        hairline_above(ui, strip.response.rect);
    }

    /// The phone form factor's page navigation. Android hides the sidebar, so
    /// on a phone the pages are reached from this strip drawn directly above
    /// them: the current one carries the text weight and an accent rule, the
    /// rest sit muted.
    #[cfg(target_os = "android")]
    fn draw_android_tab_strip(&mut self, ui: &mut egui::Ui) {
        let strip = egui::Panel::top("vm_tab_strip")
            .exact_size(36.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    self.nav_button(ui, ShellPage::Home, "Home");
                    self.nav_button(ui, ShellPage::Console, "Console");
                    self.nav_button(ui, ShellPage::Hardware, "Hardware");
                    self.nav_button(ui, ShellPage::Images, "Images");
                });
            });
        hairline_below(ui, strip.response.rect);
    }

    fn draw_central(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.take_runtime_error_notice();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_BASE))
            .show(ui, |ui| {
                #[cfg(target_os = "android")]
                self.draw_android_tab_strip(ui);
                match self.chrome.page() {
                    ShellPage::Home => self.draw_home_page(ui),
                    ShellPage::Console => self.draw_console_page(ui, frame),
                    ShellPage::Hardware => self.draw_hardware_page(ui),
                    ShellPage::Images => self.draw_images_page(ui),
                }
            });
    }

    fn draw_home_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::same(SPACE_PAGE))
                .show(ui, |ui| {
                    self.draw_shell_notice(ui);
                    self.draw_home_header(ui);
                    ui.add_space(SPACE_GROUP);
                    let status = self.runtime_status();
                    let start_enabled = !status.running && !status.start_pending;
                    ui.columns(3, |columns| {
                        action_tile_enabled(
                            &mut columns[0],
                            "Power On VM",
                            "Start this VM with the settings selected below.",
                            ACCENT_CYAN,
                            ActionTileWeight::Primary,
                            start_enabled,
                            || self.start_vm(),
                        );
                        // Cyan is the primary verb's colour and no other tile
                        // is a verb, so the two shortcuts rest on the hairline.
                        action_tile(
                            &mut columns[1],
                            "Create Disk Image",
                            "Build bximage-compatible hard disks and floppies.",
                            STROKE_HAIRLINE,
                            ActionTileWeight::Secondary,
                            || self.chrome.go_to(ShellPage::Images),
                        );
                        action_tile(
                            &mut columns[2],
                            "Hardware Settings",
                            "Inspect boot media and VM hardware limits.",
                            STROKE_HAIRLINE,
                            ActionTileWeight::Secondary,
                            || self.chrome.go_to(ShellPage::Hardware),
                        );
                    });
                });
        });
    }

    /// The selected VM as the Summary page's headline: its state badge and
    /// engine, its editable name, the facts the tree already summarises about
    /// it, and the profile verbs that have no other home — "Keep in library"
    /// for the temporary VM, and delete or discard. Power belongs to the VM
    /// bar and duplication to the sidebar's `+`, so neither is repeated here.
    fn draw_home_header(&mut self, ui: &mut egui::Ui) {
        let status = self.runtime_status();
        let badge = shell_state_badge(&status, self.has_error_notice());
        shell_card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                status_dot(ui, badge.color);
                ui.label(status_text(badge.label).color(badge.color));
                ui.label(status_text("·").color(TEXT_MUTED));
                ui.label(status_text(engine_label(self.settings.engine)).color(TEXT_MUTED));
            });
            let mut name_changed = false;
            if let Some(profile) = self.profiles.get_mut(self.chrome.selected_vm()) {
                name_changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut profile.name)
                            .font(egui::FontId::proportional(TEXT_DISPLAY))
                            .desired_width(320.0),
                    )
                    .changed();
            }
            if name_changed {
                if let Err(message) = self.apply_pending_settings() {
                    self.notify(ShellNotice::error(message));
                }
            }
            ui.add_space(SPACE_ITEM);
            if let Some(entry) = self.chrome.vm_library.get(self.chrome.selected_vm()) {
                let cpus = cpu_count_label(self.vm_info.cpus);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = HOME_FACT_GAP;
                    home_fact(ui, "Memory", &entry.memory);
                    home_fact(ui, "Processors", &cpus);
                    home_fact(ui, "Boot", &entry.boot);
                    home_fact(ui, "CD/DVD", &entry.cdrom);
                    home_fact(ui, "Disk", &entry.disk);
                });
            }
            ui.add_space(SPACE_ITEM);
            let stopped = !status.running && !status.start_pending;
            let origin = self.profiles[self.chrome.selected_vm()].origin.clone();
            ui.horizontal_wrapped(|ui| {
                if origin == VmOrigin::Launch
                    && ui
                        .add(primary_button("Keep in library"))
                        .on_hover_text("Save this VM to the library so it is listed at every launch")
                        .clicked()
                {
                    self.keep_selected_in_library();
                }
                let verb = match origin {
                    VmOrigin::Library(_) => "Delete VM",
                    VmOrigin::Launch => "Discard",
                };
                if ui
                    .add_enabled(stopped && self.profiles.len() > 1, egui::Button::new(verb))
                    .clicked()
                {
                    self.request_delete_selected();
                }
            });
            let caption = self.summary_caption();
            ui.label(RichText::new(caption).size(TEXT_CAPTION).color(TEXT_MUTED));
        });
    }

    /// The Summary page's caption on where the selected VM is kept: its
    /// library file and whether that file holds it, or that it is temporary.
    fn summary_caption(&self) -> String {
        let profile = &self.profiles[self.chrome.selected_vm()];
        match &profile.origin {
            VmOrigin::Launch => "Temporary VM. Not saved until it is kept in the library.".to_owned(),
            VmOrigin::Library(stem) => {
                let path = self.library.path_of(stem);
                match profile.save_state {
                    SaveState::Saved => format!("Saved automatically to {}", path.display()),
                    SaveState::Unsaved => format!("Saving to {} when the edit ends", path.display()),
                    SaveState::WriteFailed => format!(
                        "Not saved: the last write to {} failed. \
                         It is tried again at the next change, selection or power-on.",
                        path.display()
                    ),
                }
            }
        }
    }

    fn draw_console_page(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.draw_shell_notice(ui);
        let status = self.runtime_status();
        // The embedded view owns the whole console region in every state — the
        // serial panel stays readable while the VM is off. The shell supplies
        // only the words for the powered-off display, and none while the
        // machine runs or is about to.
        let placeholder = if status.running || status.start_pending {
            None
        } else {
            Some(powered_off_placeholder())
        };
        self.emulator
            .ui_embedded_with_serial(ui, frame, self.chrome.show_serial, placeholder);
    }

    fn draw_hardware_page(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .inner_margin(egui::Margin::same(SPACE_PAGE))
            .show(ui, |ui| {
                self.draw_shell_notice(ui);
                page_header(ui, "Hardware", "Settings apply at power-on.");
                let height = ui.available_height();
                // A card stands exactly as tall as its column when its content
                // is the column less the frame's total margin: the padding and
                // the hairline on both edges.
                let card_content_height =
                    (height - shell_card_frame().total_margin().sum().y).max(0.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(HARDWARE_LIST_WIDTH, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            // The list is a navigator: it sits on the panel
                            // surface `selection_row` paints its selection over.
                            shell_card_frame().fill(BG_PANEL).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.set_min_height(card_content_height);
                                for device in HardwareDevice::ALL {
                                    let mark = if self.chrome.selected_hardware == device {
                                        RowMark::Destination
                                    } else {
                                        RowMark::Plain
                                    };
                                    if selection_row(ui, device.label(), ROOT_INDENT, mark, None)
                                        .clicked()
                                    {
                                        self.chrome.selected_hardware = device;
                                    }
                                }
                            });
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            shell_card_frame().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.set_min_height(card_content_height);
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    self.draw_hardware_detail(ui);
                                });
                            });
                        },
                    );
                });
            });
    }

    fn draw_hardware_detail(&mut self, ui: &mut egui::Ui) {
        let status = self.runtime_status();
        let editable = !status.running && !status.start_pending;
        let mut changed = false;

        match self.chrome.selected_hardware {
            HardwareDevice::Memory => {
                let memory_step = if cfg!(target_os = "android") { 64 } else { 1 };
                let memory_block_step = if cfg!(target_os = "android") { 64 } else { 1 };
                page_header(
                    ui,
                    "Memory",
                    "Edit guest memory, host memory, and allocation block size before power-on.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    changed |= field_row(ui, "Guest memory", |ui| {
                        draw_u32_field(
                            ui,
                            &mut self.settings.memory_mib,
                            1,
                            4096,
                            " MB",
                            editable,
                            memory_step,
                            None,
                        )
                    });
                    changed |= field_row(ui, "Host memory", |ui| {
                        draw_u32_field(
                            ui,
                            &mut self.settings.host_memory_mib,
                            1,
                            4096,
                            " MB",
                            editable,
                            memory_step,
                            None,
                        )
                    });
                    changed |= field_row(ui, "Memory block", |ui| {
                        draw_u32_field(
                            ui,
                            &mut self.settings.memory_block_kib,
                            1,
                            65_536,
                            " KiB",
                            editable,
                            memory_block_step,
                            None,
                        )
                    });
                });
            }
            HardwareDevice::Processors => {
                page_header(
                    ui,
                    "Virtual CPU",
                    "Select virtual processors and pacing before power-on. Max instructions of 0 means unlimited.",
                );
                detail_row(ui, "Virtual processors", &self.vm_info.cpus.to_string());
                ui.add_enabled_ui(editable, |ui| {
                    changed |= field_row(ui, "Sockets", |ui| {
                        draw_u32_field(
                            ui,
                            &mut self.settings.cpu_sockets,
                            1,
                            BX_CPU_PROCESSORS_LIMIT,
                            "",
                            editable,
                            1,
                            None,
                        )
                    });
                    changed |= field_row(ui, "Cores / socket", |ui| {
                        draw_u32_field(
                            ui,
                            &mut self.settings.cpu_cores,
                            1,
                            BX_CPU_CORES_LIMIT,
                            "",
                            editable,
                            1,
                            None,
                        )
                    });
                    changed |= field_row(ui, "Threads / core", |ui| {
                        draw_u32_field(
                            ui,
                            &mut self.settings.cpu_threads,
                            1,
                            BX_CPU_HT_THREADS_LIMIT,
                            "",
                            editable,
                            1,
                            None,
                        )
                    });
                    let total = self
                        .settings
                        .cpu_sockets
                        .saturating_mul(self.settings.cpu_cores)
                        .saturating_mul(self.settings.cpu_threads);
                    let total_text = if total > BX_MAX_SMP_THREADS_SUPPORTED {
                        RichText::new(format!(
                            "{total} logical CPUs — exceeds SMP limit of {BX_MAX_SMP_THREADS_SUPPORTED}"
                        ))
                        .color(ACCENT_AMBER)
                    } else {
                        RichText::new(format!("{total} logical CPUs")).color(TEXT_MUTED)
                    };
                    field_row(ui, "", |ui| {
                        ui.label(total_text);
                    });
                    changed |= field_row(ui, "IPS target", |ui| {
                        draw_u32_field(
                            ui,
                            &mut self.settings.ips,
                            1,
                            2_000_000_000,
                            "",
                            editable,
                            1_000_000,
                            Some(1_000_000.0),
                        )
                    });
                    changed |= field_row(ui, "", |ui| {
                        ui.checkbox(&mut self.settings.sync_slowdown, "Sync slowdown")
                            .changed()
                    });
                    // Which engine retires the guest's instructions. The
                    // hypervisor is offered in the builds that carry its path —
                    // Windows, with `hv-whp` and without `guest-trace`, the gate
                    // `runner.rs` compiles the engine under. The host is not
                    // asked here: a host without the platform is refused at
                    // power-on (`RunError::NoHypervisor`).
                    field_row(ui, "Engine", |ui| {
                        egui::ComboBox::from_id_salt("Engine")
                            .selected_text(engine_label(self.settings.engine))
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.settings.engine,
                                        crate::config::Engine::Interpreter,
                                        engine_label(crate::config::Engine::Interpreter),
                                    )
                                    .changed();
                                #[cfg(all(
                                    not(feature = "guest-trace"),
                                    feature = "hv-whp",
                                    windows
                                ))]
                                {
                                    changed |= ui
                                        .selectable_value(
                                            &mut self.settings.engine,
                                            crate::config::Engine::Whp,
                                            engine_label(crate::config::Engine::Whp),
                                        )
                                        .changed();
                                }
                            });
                    });
                    changed |= field_row(ui, "Max instructions", |ui| {
                        draw_u64_field(
                            ui,
                            &mut self.settings.max_instructions,
                            0,
                            u64::MAX,
                            "",
                            editable,
                            1_000_000,
                            Some(1.0),
                        )
                    });
                });
            }
            HardwareDevice::Devices => {
                page_header(
                    ui,
                    "Devices",
                    "PCI and boot order apply at the next Power On.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    changed |= field_row(ui, "", |ui| {
                        ui.checkbox(&mut self.settings.pci, "Enable PCI").changed()
                    });
                    ui.add_space(SPACE_ITEM);
                    ui.label(
                        RichText::new("Boot order (first match boots)")
                            .strong()
                            .color(TEXT_PRIMARY),
                    );

                    let mut move_up: Option<usize> = None;
                    let mut move_down: Option<usize> = None;
                    let mut remove: Option<usize> = None;
                    let len = self.settings.boot_order.len();
                    for (index, device) in self.settings.boot_order.iter().enumerate() {
                        field_row(ui, &format!("{}. {device}", index + 1), |ui| {
                            if ui
                                .add_enabled(index > 0, egui::Button::new("⏶"))
                                .on_hover_text("Move earlier")
                                .clicked()
                            {
                                move_up = Some(index);
                            }
                            if ui
                                .add_enabled(index + 1 < len, egui::Button::new("⏷"))
                                .on_hover_text("Move later")
                                .clicked()
                            {
                                move_down = Some(index);
                            }
                            if ui.button("×").on_hover_text("Remove").clicked() {
                                remove = Some(index);
                            }
                        });
                    }
                    if let Some(index) = move_up {
                        self.settings.boot_order.swap(index, index - 1);
                        changed = true;
                    }
                    if let Some(index) = move_down {
                        self.settings.boot_order.swap(index, index + 1);
                        changed = true;
                    }
                    if let Some(index) = remove {
                        self.settings.boot_order.remove(index);
                        changed = true;
                    }

                    for device in [
                        crate::args::BootDevice::Disk,
                        crate::args::BootDevice::Cdrom,
                    ] {
                        if !self.settings.boot_order.contains(&device) {
                            let attached = self.settings.is_boot_device_attached(device);
                            field_row(ui, "", |ui| {
                                if ui
                                    .add_enabled(
                                        attached,
                                        egui::Button::new(format!("Add {device}")),
                                    )
                                    .clicked()
                                {
                                    self.settings.boot_order.push(device);
                                    changed = true;
                                }
                            });
                        }
                    }
                });
                detail_row(ui, "Effective boot order", &self.vm_info.boot);
            }
            HardwareDevice::HardDisk => {
                page_header(
                    ui,
                    "Hard disk",
                    "Attach or detach hard disk media for the next launch.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    changed |= field_row(ui, "", |ui| {
                        ui.checkbox(&mut self.settings.disk_enabled, "Enable hard disk")
                            .changed()
                    });
                    field_row(ui, "Disk path", |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.disk_path)
                                    .desired_width(path_field_width(ui)),
                            )
                            .changed();
                        if ui.button(BROWSE).clicked() {
                            changed |= self.browse(BrowseTarget::HardDisk);
                        }
                    });
                    field_row(ui, "ATA channel", |ui| {
                        changed |= ui
                            .add(egui::DragValue::new(&mut self.settings.disk_channel).range(0..=1))
                            .changed();
                        ui.label(RichText::new("drive").color(TEXT_MUTED));
                        changed |= ui
                            .add(egui::DragValue::new(&mut self.settings.disk_drive).range(0..=1))
                            .changed();
                    });

                    let mut override_enabled = self.settings.disk_chs_override.is_some();
                    field_row(ui, "", |ui| {
                        if ui
                            .checkbox(&mut override_enabled, "Override CHS geometry")
                            .on_hover_text(
                                "Force a specific cylinders/heads/sectors geometry instead of \
                                 auto-detecting it from the image size.",
                            )
                            .changed()
                        {
                            self.settings.disk_chs_override = override_enabled.then(|| {
                                self.config.disk.as_ref().map_or(
                                    crate::args::DiskGeometry {
                                        cylinders: 16_383,
                                        heads: 16,
                                        sectors_per_track: 63,
                                    },
                                    |disk| disk.geometry,
                                )
                            });
                            changed = true;
                        }
                    });
                    if let Some(chs) = &mut self.settings.disk_chs_override {
                        field_row(ui, "Cylinders", |ui| {
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut chs.cylinders)
                                        .range(1..=(rusty_box_bximage::BOCHS_MAX_CYLINDERS - 1) as u32),
                                )
                                .changed();
                            ui.label(RichText::new("heads").color(TEXT_MUTED));
                            changed |= ui
                                .add(egui::DragValue::new(&mut chs.heads).range(1..=u8::MAX))
                                .changed();
                            ui.label(RichText::new("sectors").color(TEXT_MUTED));
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut chs.sectors_per_track)
                                        .range(1..=u8::MAX),
                                )
                                .changed();
                        });
                    }
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
                    self.chrome.go_to(ShellPage::Images);
                }
            }
            HardwareDevice::CdDvd => {
                page_header(
                    ui,
                    "CD/DVD",
                    "Attach or detach ISO media and optionally boot it first.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    changed |= field_row(ui, "", |ui| {
                        ui.checkbox(&mut self.settings.cdrom_enabled, "Enable CD/DVD")
                            .changed()
                    });
                    field_row(ui, "ISO path", |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.cdrom_path)
                                    .desired_width(path_field_width(ui)),
                            )
                            .changed();
                        if ui.button(BROWSE).clicked() {
                            changed |= self.browse(BrowseTarget::Cdrom);
                        }
                    });
                    field_row(ui, "ATA channel", |ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.cdrom_channel).range(0..=1),
                            )
                            .changed();
                        ui.label(RichText::new("drive").color(TEXT_MUTED));
                        changed |= ui
                            .add(egui::DragValue::new(&mut self.settings.cdrom_drive).range(0..=1))
                            .changed();
                    });
                    let mut boot_cdrom = self.settings.boot_order.first()
                        == Some(&crate::args::BootDevice::Cdrom);
                    field_row(ui, "", |ui| {
                        if ui.checkbox(&mut boot_cdrom, "Boot CD/DVD first").changed() {
                            // Reposition the CD/DVD within the boot order without
                            // disturbing the other devices' relative order.
                            self.settings
                                .boot_order
                                .retain(|device| *device != crate::args::BootDevice::Cdrom);
                            if boot_cdrom {
                                self.settings
                                    .boot_order
                                    .insert(0, crate::args::BootDevice::Cdrom);
                            } else {
                                self.settings.boot_order.push(crate::args::BootDevice::Cdrom);
                            }
                            changed = true;
                        }
                    });
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
                page_header(
                    ui,
                    "Display and ROMs",
                    "BIOS, VGA BIOS, and logging are applied on the next Power On.",
                );
                ui.add_enabled_ui(editable, |ui| {
                    field_row(ui, "BIOS path", |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.bios_path)
                                    .desired_width(path_field_width(ui)),
                            )
                            .changed();
                        if ui.button(BROWSE).clicked() {
                            changed |= self.browse(BrowseTarget::Bios);
                        }
                    });
                    field_row(ui, "VGA BIOS path", |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.vga_bios_path)
                                    .desired_width(path_field_width(ui)),
                            )
                            .changed();
                        if ui.button(BROWSE).clicked() {
                            changed |= self.browse(BrowseTarget::VgaBios);
                        }
                    });
                    field_row(ui, "Log level", |ui| {
                        egui::ComboBox::from_id_salt("Log level")
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

                    field_row(ui, "Display resolution", |ui| {
                        egui::ComboBox::from_id_salt("Display resolution")
                            .selected_text(vga_mode_label(self.settings.vga_mode))
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.settings.vga_mode,
                                        None,
                                        "Default (VGA / VBE)",
                                    )
                                    .changed();
                                for &(w, h) in VGA_MODE_PRESETS {
                                    let mode = crate::config::VgaMode {
                                        width: w,
                                        height: h,
                                        bpp: 32,
                                    };
                                    changed |= ui
                                        .selectable_value(
                                            &mut self.settings.vga_mode,
                                            Some(mode),
                                            format!("{w}×{h} @ 32bpp"),
                                        )
                                        .changed();
                                }
                            });
                    });
                    field_row(ui, "", |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "Raises the VBE ceiling so the guest can select this mode (via \
                                     GRUB gfxpayload / vesafb).",
                                )
                                .color(TEXT_MUTED),
                            )
                            .wrap(),
                        );
                    });
                    changed |= field_row(ui, "", |ui| {
                        ui.checkbox(
                            &mut self.settings.pci_vga,
                            "Register VGA on PCI (experimental KMS / bochs-drm)",
                        )
                        .on_hover_text(
                            "Exposes the adapter as PCI 1234:1111 so Linux bochs-drm can bind for \
                             a full KMS framebuffer. Experimental — verify with a guest boot.",
                        )
                        .changed()
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
                    self.chrome.go_to(ShellPage::Console);
                }
            }
        }

        if changed {
            if let Err(message) = self.apply_pending_settings() {
                self.notify(ShellNotice::error(message));
            }
        }

        ui.horizontal_wrapped(|ui| {
            match &self.profiles[self.chrome.selected_vm()].origin {
                VmOrigin::Library(stem) => {
                    ui.label(metadata_text("File", &self.library.path_of(stem).display().to_string()));
                }
                VmOrigin::Launch => {
                    ui.label(
                        RichText::new("Not saved. Keep this VM in the library from its Summary page.")
                            .color(TEXT_MUTED),
                    );
                }
            }
        });

        if !editable {
            ui.add_space(SPACE_ITEM);
            ui.label(RichText::new("Power off before changing VM hardware.").color(ACCENT_AMBER));
        }
    }

    fn draw_images_page(&mut self, ui: &mut egui::Ui) {
        self.draw_shell_notice(ui);
        if let Some(created) = self.disk_creator.ui_page(ui) {
            self.handle_created_image(created);
        }
        if std::mem::take(&mut self.disk_creator.browse_requested)
            && self.browse(BrowseTarget::NewImage)
        {
            if let Err(message) = self.apply_pending_settings() {
                self.notify(ShellNotice::error(message));
            }
        }
    }

    fn handle_created_image(&mut self, created: CreatedImage) {
        match created.kind {
            CreatedImageKind::HardDisk => {
                let status = self.runtime_status();
                if status.running || status.start_pending {
                    self.notify(ShellNotice::warning(
                        "Disk image created. Stop the VM before attaching it.",
                    ));
                    return;
                }
                self.attach_created_image_to_selected_profile(created.path);
            }
            CreatedImageKind::Floppy => {
                self.notify(ShellNotice::info(
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
                self.notify(ShellNotice::info(format!(
                    "Attached created disk image to {}.",
                    self.vm_info.name
                )));
            }
            Err(message) => {
                self.notify(ShellNotice::error(message));
            }
        }
    }

    /// The phone form factor's navigation. Android hides the sidebar, so on a
    /// phone this strip is the only way off the page it is drawn on.
    #[cfg(target_os = "android")]
    fn nav_button(&mut self, ui: &mut egui::Ui, page: ShellPage, label: &str) {
        let selected = self.chrome.page() == page;
        let text = RichText::new(label)
            .size(14.0)
            .color(if selected { TEXT_PRIMARY } else { TEXT_MUTED });
        let text = if selected { text.strong() } else { text };
        let response = ui.add(
            egui::Button::new(text)
                .frame_when_inactive(false)
                .min_size(egui::vec2(0.0, ui.available_height())),
        );
        if selected {
            let rect = response.rect;
            ui.painter().hline(
                rect.x_range(),
                rect.bottom() - 1.0,
                Stroke::new(2.0_f32, ACCENT_CYAN),
            );
        }
        if response.clicked() {
            self.chrome.go_to(page);
        }
    }

    /// Applies the edited settings to the selected VM in memory and marks a
    /// library VM `Unsaved` when its name or config differs from what its
    /// file holds, `Saved` when not — so an apply that changes nothing, a
    /// selection or a power-on, does not rewrite the file over an edit made
    /// to it outside the shell. The file is written by the next flush, not
    /// here, so a field edited keystroke by keystroke is written once it is
    /// left rather than on every change.
    fn apply_pending_settings(&mut self) -> Result<(), String> {
        self.settings.apply_to_config(&mut self.config)?;
        if let Some(profile) = self.profiles.get_mut(self.chrome.selected_vm()) {
            profile.config.clone_from(&self.config);
            profile.settings.clone_from(&self.settings);
            profile.mark_against_file(ConfigChange::Edit);
            self.refresh_selected_profile_metadata()?;
            self.config
                .clone_from(&self.profiles[self.chrome.selected_vm()].config);
        } else {
            self.vm_info = NativeVmInfo::from_config(&self.config);
        }
        Ok(())
    }

    /// Writes the library VMs `scope` names among those whose files are
    /// behind. A write that succeeds makes the file what the VM is; one that
    /// fails leaves the VM `WriteFailed`, is logged, and is shown as an error
    /// by this attempt alone. Either way the VM's sidebar row says which.
    fn flush(&mut self, scope: FlushScope) {
        for index in 0..self.profiles.len() {
            let profile = &self.profiles[index];
            let VmOrigin::Library(stem) = &profile.origin else {
                continue;
            };
            let due = match (profile.save_state, scope) {
                (SaveState::Saved, _) | (SaveState::WriteFailed, FlushScope::Unsaved) => false,
                (SaveState::Unsaved, _) | (SaveState::WriteFailed, FlushScope::UnsavedAndFailed) => {
                    true
                }
            };
            if !due {
                continue;
            }
            match self.library.save(stem, &profile.name, &profile.config) {
                Ok(()) => {
                    let profile = &mut self.profiles[index];
                    profile.file = Some(VmFileContents::of(&profile.name, &profile.config));
                    profile.save_state = SaveState::Saved;
                }
                Err(error) => {
                    tracing::error!(vm = stem.as_str(), %error, "the VM's file was not written");
                    self.profiles[index].save_state = SaveState::WriteFailed;
                    self.notify(ShellNotice::error(error.to_string()));
                }
            }
            self.refresh_library_entry(index);
        }
    }

    /// Makes the sidebar's row for the VM at `index` say what the VM is now.
    fn refresh_library_entry(&mut self, index: usize) {
        if let Some(entry) = self.chrome.vm_library.get_mut(index) {
            *entry = self.profiles[index].library_entry();
        }
    }

    /// An action's flush: writes every VM whose file is behind, one whose
    /// last write failed included.
    fn flush_unsaved(&mut self) {
        self.flush(FlushScope::UnsavedAndFailed);
    }

    /// The end of a frame's flush. Writes the `Unsaved` VMs, and only while
    /// no widget holds keyboard focus (`Memory::focused`) and no widget is
    /// being dragged (`Context::dragged_id`), so an edit is written once the
    /// field it is typed in is left or the drag ends, not on every change. A
    /// VM whose write failed is left for an action to try again.
    fn flush_unsaved_when_idle(&mut self, ctx: &egui::Context) {
        let editing = ctx.memory(|memory| memory.focused().is_some()) || ctx.dragged_id().is_some();
        if !editing {
            self.flush(FlushScope::Unsaved);
        }
    }

    /// The flush for the frame on which the window went behind another
    /// (`Event::WindowFocused(false)`) — on a phone, the activity leaving the
    /// foreground, after which the process can be killed with no further
    /// frame. An action's flush: an edit in a field that still has focus is
    /// written without waiting for the field to be left, and a write that
    /// failed is tried again. It runs on that frame alone, so a window that
    /// stays unfocused does not retry a lasting fault every frame.
    fn flush_when_window_focus_is_lost(&mut self, ctx: &egui::Context) {
        let lost = ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        });
        if lost {
            self.flush_unsaved();
        }
    }

    /// Applies the selected VM's settings to it, marks it against its file,
    /// and makes its sidebar row and the VM information shown say what the
    /// VM is — whether or not the settings apply: a failed apply leaves the
    /// VM as it was, and the row and the information show it as it is.
    fn refresh_selected_profile_metadata(&mut self) -> Result<(), String> {
        let index = self.chrome.selected_vm();
        let Some(profile) = self.profiles.get_mut(index) else {
            return Ok(());
        };
        let applied = profile.apply_settings();
        profile.mark_against_file(ConfigChange::Reading);
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
        applied
    }

    /// Shows the VM at `index`. The VM being left is applied and written
    /// first, so nothing of it waits on a later frame.
    fn select_profile(&mut self, index: usize) {
        if index >= self.profiles.len() {
            return;
        }
        let status = self.runtime_status();
        if status.running || status.start_pending {
            self.notify(ShellNotice::warning(
                "Stop the running VM before selecting another VM.",
            ));
            return;
        }
        if let Err(message) = self.apply_pending_settings() {
            self.notify(ShellNotice::error(message));
            return;
        }
        self.flush_unsaved();
        self.chrome.destination = self.chrome.destination.select_vm(index);
        let profile = &self.profiles[index];
        self.config = profile.config.clone();
        self.settings = profile.settings.clone();
        if let VmOrigin::Library(stem) = self.profiles[index].origin.clone() {
            self.remember(&stem);
        }
        if let Err(message) = self.refresh_selected_profile_metadata() {
            self.notify(ShellNotice::error(message));
        }
    }

    /// Adds a library VM copied from the selected one and selects it. Its file
    /// is written at once, so it is there at the next launch. Refused while
    /// the machine runs or is starting, when the new VM could not be selected.
    fn add_vm_copying_selected(&mut self) {
        let status = self.runtime_status();
        if status.running || status.start_pending {
            self.notify(ShellNotice::warning(
                "Stop the running VM before adding a VM.",
            ));
            return;
        }
        if let Err(message) = self.apply_pending_settings() {
            self.notify(ShellNotice::error(message));
            return;
        }
        self.flush_unsaved();
        let base = self.chrome.selected_vm().min(self.profiles.len() - 1);
        let name = format!("{} copy", self.profiles[base].name);
        let stem = match self.library.create(&name, &self.profiles[base].config) {
            Ok(stem) => stem,
            Err(error) => {
                self.notify(ShellNotice::error(error.to_string()));
                return;
            }
        };
        let profile = self.profiles[base].duplicate(name, VmOrigin::Library(stem));
        self.chrome.vm_library.push(profile.library_entry());
        self.profiles.push(profile);
        self.select_profile(self.profiles.len() - 1);
    }

    /// Adds the temporary launch VM to the library. From then on it is an
    /// ordinary library VM: its edits are saved and the next launch lists it.
    /// A `remember` warning outranks the saved notice, so it is set last.
    fn keep_selected_in_library(&mut self) {
        if let Err(message) = self.apply_pending_settings() {
            self.notify(ShellNotice::error(message));
            return;
        }
        self.flush_unsaved();
        let index = self.chrome.selected_vm();
        let Some(profile) = self.profiles.get(index) else {
            return;
        };
        if profile.origin != VmOrigin::Launch {
            return;
        }
        match self.library.create(&profile.name, &profile.config) {
            Ok(stem) => {
                let profile = &mut self.profiles[index];
                profile.origin = VmOrigin::Library(stem.clone());
                profile.file = Some(VmFileContents::of(&profile.name, &profile.config));
                profile.save_state = SaveState::Saved;
                self.chrome.vm_library[index] = self.profiles[index].library_entry();
                self.notify(ShellNotice::info(format!(
                    "Saved {} to the VM library.",
                    self.profiles[index].name
                )));
                self.remember(&stem);
            }
            Err(error) => self.notify(ShellNotice::error(error.to_string())),
        }
    }

    /// Removes the selected VM and, for a library VM, its file. The VM goes as
    /// it stands: nothing about it is validated or written first, so a VM
    /// whose settings no longer apply — a disk image that is gone, an empty
    /// BIOS path — is still deleted, and its unsaved edits go with it. A
    /// refresh error outranks a `remember` warning, so the refresh comes last.
    fn delete_selected_profile(&mut self) {
        let status = self.runtime_status();
        if status.running || status.start_pending {
            self.notify(ShellNotice::warning(
                "Stop the running VM before deleting it.",
            ));
            return;
        }
        if self.profiles.len() == 1 {
            self.notify(ShellNotice::warning("At least one VM is required."));
            return;
        }

        let removed = self.chrome.selected_vm().min(self.profiles.len() - 1);
        if let VmOrigin::Library(stem) = &self.profiles[removed].origin {
            if let Err(error) = self.library.delete(stem) {
                self.notify(ShellNotice::error(error.to_string()));
                return;
            }
        }
        self.profiles.remove(removed);
        if removed < self.chrome.vm_library.len() {
            self.chrome.vm_library.remove(removed);
        }
        self.chrome.destination = self
            .chrome
            .destination
            .clamped_after_removal(removed, self.profiles.len());
        let profile = &self.profiles[self.chrome.selected_vm()];
        self.config = profile.config.clone();
        self.settings = profile.settings.clone();
        if let VmOrigin::Library(stem) = self.profiles[self.chrome.selected_vm()].origin.clone() {
            self.remember(&stem);
        }
        if let Err(message) = self.refresh_selected_profile_metadata() {
            self.notify(ShellNotice::error(message));
        }
    }

    /// Asks the user to confirm deleting the selected VM. The VM is captured
    /// here, so the confirm deletes it and nothing else.
    fn request_delete_selected(&mut self) {
        let index = self.chrome.selected_vm();
        let Some(profile) = self.profiles.get(index) else {
            return;
        };
        self.pending_confirm = Some(PendingConfirm::DeleteVm {
            index,
            origin: profile.origin.clone(),
            name: profile.name.clone(),
        });
    }

    /// Runs the step waiting for confirmation, if any. A delete or an
    /// overwrite whose VM is no longer the selected one does nothing and
    /// says so. An agreed overwrite is kept for the session, and the
    /// power-on it stopped runs.
    fn confirm_pending(&mut self) {
        match self.pending_confirm.take() {
            None => {}
            Some(PendingConfirm::DeleteVm { index, origin, .. }) => {
                if self.is_selected(index, &origin) {
                    self.delete_selected_profile();
                } else {
                    self.notify(ShellNotice::warning(
                        "Another VM was selected while the delete waited; nothing was deleted.",
                    ));
                }
            }
            Some(PendingConfirm::DeleteBroken(path)) => self.delete_broken_file(&path),
            Some(PendingConfirm::OverwriteDisk {
                index,
                origin,
                path,
            }) => {
                if self.is_selected(index, &origin) {
                    self.overwrite_confirmed.extend([path]);
                    self.start_vm();
                } else {
                    self.notify(ShellNotice::warning(
                        "Another VM was selected while the power-on waited; nothing was started.",
                    ));
                }
            }
        }
    }

    /// Whether the VM at `index` with `origin` is the one selected now.
    fn is_selected(&self, index: usize, origin: &VmOrigin) -> bool {
        self.chrome.selected_vm() == index
            && self
                .profiles
                .get(index)
                .is_some_and(|profile| &profile.origin == origin)
    }

    /// Drops the step waiting for confirmation.
    fn cancel_pending(&mut self) {
        self.pending_confirm = None;
    }

    fn confirm_wording(&self, pending: &PendingConfirm) -> ConfirmWording {
        match pending {
            PendingConfirm::DeleteVm {
                origin: VmOrigin::Library(stem),
                name,
                ..
            } => ConfirmWording {
                title: format!("Delete {name}?"),
                body: format!(
                    "Its file {} is removed from the VM library. Disk images it uses are kept.",
                    self.library.path_of(stem).display()
                ),
                verb: "Delete",
            },
            PendingConfirm::DeleteVm {
                origin: VmOrigin::Launch,
                name,
                ..
            } => ConfirmWording {
                title: format!("Discard {name}?"),
                body: "This VM is not in the library, so nothing is left of it.".to_owned(),
                verb: "Discard",
            },
            PendingConfirm::DeleteBroken(path) => ConfirmWording {
                title: format!(
                    "Delete {}?",
                    path.file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
                ),
                body: "The file could not be loaded as a VM. It is removed from the VM library."
                    .to_owned(),
                verb: "Delete",
            },
            PendingConfirm::OverwriteDisk { path, .. } => ConfirmWording {
                title: format!("Overwrite {}?", path.display()),
                body: "This VM's startup disk is set to be recreated, which erases the existing file."
                    .to_owned(),
                verb: "Overwrite and power on",
            },
        }
    }

    /// The dialog for the step waiting for confirmation. Its verb runs the
    /// step; Cancel, Escape or a click outside drops it.
    fn draw_pending_confirm(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_confirm.clone() else {
            return;
        };
        let wording = self.confirm_wording(&pending);
        let mut confirmed = false;
        let mut cancelled = false;
        let modal = egui::Modal::new(egui::Id::new("shell_confirm")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(RichText::new(wording.title).strong().color(TEXT_PRIMARY));
            ui.add_space(SPACE_ITEM);
            ui.label(RichText::new(wording.body).color(TEXT_MUTED));
            ui.add_space(SPACE_GROUP);
            ui.horizontal(|ui| {
                confirmed = ui.add(primary_button(wording.verb)).clicked();
                cancelled = ui.button("Cancel").clicked();
            });
        });
        if confirmed {
            self.confirm_pending();
        } else if cancelled || modal.should_close() {
            self.cancel_pending();
        }
    }

    fn delete_broken_file(&mut self, path: &Path) {
        match self.library.delete_file(path) {
            Ok(()) => self.broken_files.retain(|file| file.path != path),
            Err(error) => self.notify(ShellNotice::error(error.to_string())),
        }
    }

    /// Records `stem` as the VM the next launch shows. Failing costs only
    /// that, so it is reported as a warning.
    fn remember(&mut self, stem: &crate::library::VmStem) {
        if let Err(error) = self.library.remember_selected(stem) {
            self.notify(ShellNotice::warning(error.to_string()));
        }
    }

    /// The existing file this power-on's startup-disk creation would erase,
    /// unless the user already agreed to it this session.
    fn unconfirmed_overwrite(&self) -> Option<PathBuf> {
        let creation = self.config.disk.as_ref()?.creation.as_ref()?;
        let erases = creation.overwrite && creation.path.exists();
        (erases && !self.overwrite_confirmed.contains(&creation.path))
            .then(|| creation.path.clone())
    }

    /// The file this power-on's overwrite creation makes where nothing
    /// exists yet: the runner creates it now and erases it at no later
    /// power-on this session, so there is nothing to ask about, now or then.
    fn overwrite_with_nothing_to_erase(&self) -> Option<PathBuf> {
        let creation = self.config.disk.as_ref()?.creation.as_ref()?;
        (creation.overwrite && !creation.path.exists()).then(|| creation.path.clone())
    }

    /// Powers on the selected VM: its edits are applied and written first,
    /// then a VM that lacks a BIOS path or a medium to boot from is refused
    /// with a notice naming what is missing, and a startup-disk creation that
    /// would erase an existing file is put to the user before anything
    /// starts, so a power-on that stops at either has still written the VM.
    fn start_vm(&mut self) {
        let snapshot = self.runtime_status();
        if snapshot.running {
            self.chrome.go_to(ShellPage::Console);
            return;
        }
        if snapshot.start_pending {
            return;
        }

        if let Err(message) = self.apply_pending_settings() {
            self.notify(ShellNotice::error(message));
            return;
        }
        self.flush_unsaved();
        if let Some(gap) = PowerOnGap::of(&self.config) {
            self.notify(ShellNotice::warning(gap.notice()));
            return;
        }
        if let Some(path) = self.unconfirmed_overwrite() {
            let index = self.chrome.selected_vm();
            self.pending_confirm = Some(PendingConfirm::OverwriteDisk {
                index,
                origin: self.profiles[index].origin.clone(),
                path,
            });
            return;
        }
        self.overwrite_confirmed
            .extend(self.overwrite_with_nothing_to_erase());
        if let Ok(mut display) = self.shared.lock() {
            display.start_pending = true;
        }
        match self
            .command_tx
            .send(NativeEmulatorCommand::Start(self.config.clone()))
        {
            Ok(()) => {
                self.chrome.go_to(ShellPage::Console);
            }
            Err(_) => {
                if let Ok(mut display) = self.shared.lock() {
                    display.start_pending = false;
                }
                self.notify(ShellNotice::error(
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

    #[cfg(target_os = "android")]
    fn draw_android_console_header(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("android_console_header")
            .exact_size(32.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let status = self.runtime_status();
                    let running = status.running;
                    let start_blocked = running || status.start_pending;
                    ui.menu_button("File", |ui| {
                        if ui.button("Home").clicked() {
                            self.chrome.go_to(ShellPage::Home);
                            ui.close();
                        }
                        if ui.button("Create Disk Image").clicked() {
                            self.chrome.go_to(ShellPage::Images);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Quit").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
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
                    ui.separator();
                    let state = if running {
                        "Running"
                    } else if status.start_pending {
                        "Starting"
                    } else {
                        "Stopped"
                    };
                    ui.label(
                        RichText::new(state)
                            .monospace()
                            .size(11.0)
                            .color(TEXT_PRIMARY),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format_ips_u32(status.ips))
                            .monospace()
                            .size(11.0)
                            .color(ACCENT_BLUE),
                    );
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
    fn request_reset(&mut self) {
        if !self.is_vm_running() {
            return;
        }

        if let Ok(mut display) = self.shared.lock() {
            display.stop_flag.store(true, Ordering::Relaxed);
            display.reset_requested = true;
        }
    }

    /// Offers a file for the field `target` names and puts the choice there.
    /// Returns whether the VM's settings changed, for the caller to apply with
    /// its other edits. The host's dialog answers before this returns.
    #[cfg(not(target_os = "android"))]
    fn browse(&mut self, target: BrowseTarget) -> bool {
        let chosen = match target {
            BrowseTarget::NewImage => save_native_file(self.disk_creator.default_image_filename()),
            BrowseTarget::HardDisk
            | BrowseTarget::Cdrom
            | BrowseTarget::Bios
            | BrowseTarget::VgaBios => pick_native_file(),
        };
        chosen.is_some_and(|path| self.set_browsed_path(target, path))
    }

    /// Records a Browse press for the Android host, which answers it in a
    /// later frame with a file browser drawn over the shell and hands the
    /// choice back through [`Self::apply_browsed_path`]. Nothing changes yet.
    #[cfg(target_os = "android")]
    fn browse(&mut self, target: BrowseTarget) -> bool {
        self.browse_request = Some(target);
        false
    }

    /// Puts a chosen `path` into the field `target` names. Returns whether the
    /// VM's settings changed: an attached disk, CD or ROM does; the path of an
    /// image still to be created does not.
    fn set_browsed_path(&mut self, target: BrowseTarget, path: PathBuf) -> bool {
        let text = path.display().to_string();
        match target {
            BrowseTarget::HardDisk => {
                self.settings.disk_path = text;
                self.settings.disk_enabled = true;
                true
            }
            BrowseTarget::Cdrom => {
                self.settings.cdrom_path = text;
                self.settings.cdrom_enabled = true;
                true
            }
            BrowseTarget::Bios => {
                self.settings.bios_path = text;
                true
            }
            BrowseTarget::VgaBios => {
                self.settings.vga_bios_path = text;
                true
            }
            BrowseTarget::NewImage => {
                self.disk_creator.path = text;
                false
            }
        }
    }

    /// The Browse press waiting for the Android host, if any, with the path
    /// its field holds now.
    #[cfg(target_os = "android")]
    pub(crate) fn take_browse_request(&mut self) -> Option<BrowseRequest> {
        let target = self.browse_request.take()?;
        let (current, save_name) = match target {
            BrowseTarget::HardDisk => (&self.settings.disk_path, None),
            BrowseTarget::Cdrom => (&self.settings.cdrom_path, None),
            BrowseTarget::Bios => (&self.settings.bios_path, None),
            BrowseTarget::VgaBios => (&self.settings.vga_bios_path, None),
            BrowseTarget::NewImage => (
                &self.disk_creator.path,
                Some(self.disk_creator.default_image_filename()),
            ),
        };
        Some(BrowseRequest {
            target,
            current: PathBuf::from(current.trim()),
            save_name,
        })
    }

    /// Takes the Android host's answer to a Browse press and applies the VM's
    /// settings as an edit on the page would.
    #[cfg(target_os = "android")]
    pub(crate) fn apply_browsed_path(&mut self, target: BrowseTarget, path: PathBuf) {
        if self.set_browsed_path(target, path) {
            if let Err(message) = self.apply_pending_settings() {
                self.notify(ShellNotice::error(message));
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeShellApp {
    /// One frame of the shell: on Android the console page stands alone under
    /// its own header; everywhere else the VM bar, the tree, the status strip
    /// and the page.
    fn draw_shell(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(target_os = "android")]
        if self.chrome.page() == ShellPage::Console {
            self.draw_android_console_header(ui);
            self.draw_status_strip(ui);
            self.draw_central(ui, frame);
            draw_about_window(ui.ctx(), &mut self.chrome);
            return;
        }

        self.draw_vm_bar(ui);
        if shell_should_draw_library(&self.chrome) {
            self.draw_sidebar(ui);
        }
        self.draw_status_strip(ui);
        self.draw_central(ui, frame);
        draw_about_window(ui.ctx(), &mut self.chrome);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl eframe::App for NativeShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.begin_frame();
        self.handle_native_dropped_files(ui.ctx());
        self.draw_pending_confirm(ui.ctx());
        self.draw_shell(ui, frame);
        self.flush_when_window_focus_is_lost(ui.ctx());
        self.flush_unsaved_when_idle(ui.ctx());
    }

    /// eframe's save, made when the window is taken away from the app — on
    /// Android `Event::Suspended`, from the activity's `surfaceDestroyed`,
    /// the last call before the process can be killed — and at exit, and
    /// only when eframe has a storage to save to: the Android runner opens
    /// one so that the call is made (`runner::run_android_shell`). Every edit
    /// still only in memory is written. Nothing is put in the storage: the VM
    /// files are the shell's store.
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.flush_unsaved();
    }

    /// No periodic save: an edit is written when it ends, not on a timer that
    /// would catch a field mid-edit.
    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::MAX
    }

    /// egui's memory — window positions, what is open — is not kept between
    /// runs.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    /// The window is closing: every edit still only in memory is written.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_unsaved();
    }
}

impl DiskCreatorPanel {
    #[cfg(feature = "gui-egui")]
    fn ui_page(&mut self, ui: &mut egui::Ui) -> Option<CreatedImage> {
        let mut created_image = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::same(SPACE_PAGE))
                .show(ui, |ui| {
                    page_header(
                        ui,
                        "Disk images",
                        "Create flat hard disks and floppy images with the bximage backend.",
                    );
                    shell_card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        field_row(ui, "Kind", |ui| {
                            ui.radio_value(&mut self.kind, CreatorKind::HardDisk, "Hard disk");
                            ui.radio_value(&mut self.kind, CreatorKind::Floppy, "Floppy");
                        });

                        #[cfg(not(target_arch = "wasm32"))]
                        field_row(ui, "Path", |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.path)
                                    .desired_width(path_field_width(ui)),
                            );
                            if ui.button(BROWSE).clicked() {
                                self.browse_requested = true;
                            }
                        });

                        #[cfg(target_arch = "wasm32")]
                        field_row(ui, "Filename", |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.path)
                                    .desired_width(320.0),
                            );
                        });

                        match self.kind {
                            CreatorKind::HardDisk => {
                                field_row(ui, "Size", |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.hard_disk_size)
                                            .hint_text("20G")
                                            .desired_width(120.0),
                                    );
                                    ui.label(
                                        RichText::new("Examples: 10M, 512M, 20G, 512")
                                            .size(TEXT_SECONDARY)
                                            .color(TEXT_MUTED),
                                    );
                                });
                            }
                            CreatorKind::Floppy => {
                                field_row(ui, "Floppy format", |ui| {
                                    egui::ComboBox::from_id_salt("Floppy format")
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
                                });
                            }
                        }

                        #[cfg(not(target_arch = "wasm32"))]
                        field_row(ui, "", |ui| {
                            ui.checkbox(&mut self.overwrite, "Overwrite existing file");
                        });

                        ui.add_space(SPACE_GROUP);
                        let action = if cfg!(target_arch = "wasm32") {
                            "Download image"
                        } else {
                            "Create image"
                        };
                        if ui.add(primary_button(action)).clicked() {
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
        // Reject sizes whose physical geometry exceeds the emulator's cylinder limit
        // (BOCHS_MAX_CYLINDERS, 2^24) *before* touching the filesystem. Everything below
        // that — including 32 GiB+ — is a valid disk.
        calculate_hard_disk_geometry(size, SectorSize::Bytes512).map_err(|error| error.to_string())?;

        create_flat_hard_disk(path, size, SectorSize::Bytes512, policy)
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
type WebEmulator =
    Box<rusty_box::emulator::Emulator>;

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
    web_cpu_count: u32,
    total_instructions: u64,
    last_ips_time: web_time::Instant,
    last_ips_instructions: u64,
    cached_ips: f64,
    frame_count: u64,
    /// Previous PS/2 button bitmask for relative mouse forwarding.
    web_prev_mouse_buttons: u8,
    /// The modifiers held at the end of the previous frame, so each Shift,
    /// Ctrl and Alt edge is forwarded once.
    web_held_modifiers: rusty_box::gui::host_input::HeldModifiers,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum WebBootMode {
    Launcher,
    UploadedMedia,
}
#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Assembling a machine is one blocking call, so the browser gets a frame to
/// paint the notice before it runs.
enum WebStartupStage {
    Announce,
    BuildMachine,
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
    iso_data: Option<Vec<u8>>,
    memory_mib: usize,
    cpu_count: u32,
}

#[cfg(target_arch = "wasm32")]
impl WebStartupState {
    fn new(iso_data: Vec<u8>, memory_mib: usize, cpu_count: u32) -> Self {
        Self {
            stage: WebStartupStage::Announce,
            iso_data: Some(iso_data),
            memory_mib,
            cpu_count,
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
        WebStartupStage::Announce => Some(WebStartupStage::BuildMachine),
        WebStartupStage::BuildMachine => None,
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_startup_stage_label(stage: WebStartupStage) -> &'static str {
    match stage {
        WebStartupStage::Announce => "Allocating guest memory",
        WebStartupStage::BuildMachine => "Starting virtual machine",
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
fn web_cpu_count_is_supported(cpu_count: u32) -> bool {
    (WEB_MIN_CPU_COUNT..=WEB_MAX_CPU_COUNT).contains(&cpu_count)
}

#[cfg(any(test, target_arch = "wasm32"))]
fn web_can_edit_cpu_count(has_vm: bool) -> bool {
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

#[cfg(any(test, target_arch = "wasm32"))]
fn web_uploaded_media_config(
    memory_mib: usize,
    cpu_count: u32,
) -> rusty_box::emulator::EmulatorConfig {
    let ram_size = memory_mib * 1024 * 1024;
    rusty_box::emulator::EmulatorConfig {
        memory: rusty_box::emulator::MemorySize::bytes(ram_size),
        memory_block_size: 128 * 1024,
        ips: rusty_box::emulator::Ips::new(300_000_000),
        pci_enabled: true,
        cpu_params: BxParams::default()
            .with_topology(cpu_count, 1, 1)
            .expect("web CPU count is range-checked by the launcher"),
        ..Default::default()
    }
}
#[cfg(target_arch = "wasm32")]
const BIOS_DATA: &[u8] = include_bytes!("../../cpp_orig/bochs/bochs/bios/BIOS-bochs-latest");
#[cfg(target_arch = "wasm32")]
const VGA_BIOS_DATA: &[u8] =
    include_bytes!("../../cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin");

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
            web_cpu_count: WEB_DEFAULT_CPU_COUNT,
            total_instructions: 0,
            last_ips_time: web_time::Instant::now(),
            last_ips_instructions: 0,
            cached_ips: 0.0,
            frame_count: 0,
            web_prev_mouse_buttons: 0,
            web_held_modifiers: rusty_box::gui::host_input::HeldModifiers::default(),
        }
    }

    fn begin_uploaded_media_startup(&mut self, upload: WebUploadedMedia) {
        self.record_uploaded_media_metadata(&upload.name, upload.data.len());
        self.startup = Some(WebStartupState::new(
            upload.data,
            self.web_memory_mib,
            self.web_cpu_count,
        ));
        self.boot_mode = WebBootMode::UploadedMedia;
        self.chrome.go_to(ShellPage::Console);
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
            WebStartupStage::Announce => {}
            WebStartupStage::BuildMachine => {
                let iso_data = startup.iso_data.take().ok_or_else(|| {
                    "uploaded boot media was not available during startup".to_owned()
                })?;
                let mut vga_data = VGA_BIOS_DATA.to_vec();
                let remainder = vga_data.len() % 512;
                if remainder != 0 {
                    vga_data.resize(vga_data.len() + (512 - remainder), 0);
                }
                use rusty_box::emulator::{AtaSlot, BootDevice, BootOrder, MachineBuilder};
                let mut emu = MachineBuilder::new(web_uploaded_media_config(
                    startup.memory_mib,
                    startup.cpu_count,
                ))
                .bios(BIOS_DATA)
                .vga_bios(&vga_data)
                .boot_order(BootOrder::just(BootDevice::Cdrom))
                .cdrom_bytes(AtaSlot::SECONDARY_MASTER, iso_data)
                .build()
                .map_err(|error| format!("{error:?}"))?;
                emu.display().force_update();
                return Ok(Some(emu));
            }
        }

        startup.stage = web_next_startup_stage(startup.stage)
            .expect("startup stage should advance until the machine is built");
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
            self.chrome.go_to(ShellPage::Console);
        } else {
            self.chrome.go_to(ShellPage::Home);
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

    fn set_web_cpu_count(&mut self, cpu_count: u32) {
        if !web_cpu_count_is_supported(cpu_count) || !web_can_edit_cpu_count(self.web_has_vm()) {
            return;
        }
        self.web_cpu_count = cpu_count;
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
                    match emu.step(RunBudget::Instructions(WEB_BATCH_SIZE)) {
                        Ok(outcome) => {
                            // Whichever unit the machine measures in: this
                            // paces a frame, and a frame is over when enough
                            // has happened, not when a particular kind has.
                            frame_executed =
                                frame_executed.saturating_add(outcome.progress.count());
                            // Every terminal cause, not just a CPU shutdown: a
                            // guest that powers itself off through ACPI leaves
                            // the CPU healthy, so testing the CPU alone would
                            // keep pumping a machine that asked to be off.
                            if outcome.is_terminal() {
                                self.shutdown = true;
                                break;
                            }
                            if outcome.progress.stalled() {
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
                emu.display().render_into(&mut self.display);
            }
        }
    }

    /// Forward this frame's keyboard to the guest, with the `Emulator` as the
    /// sink (single-threaded wasm applies events immediately) — the same
    /// translator the native shell feeds through its shared display. A widget
    /// that has asked for the keyboard (the Library search box) keeps it, as
    /// on native; the translator consumes what it forwards.
    fn process_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let Some(emu) = &mut self.emulator else {
            return;
        };
        let held = self.web_held_modifiers;
        self.web_held_modifiers = ctx.input_mut(|input| {
            rusty_box::gui::host_input::translate_egui_keyboard(input, held, &mut **emu)
        });
    }

    /// Forward relative mouse motion / buttons / wheel to the guest while the
    /// pointer is over the display. Uses the shared egui→`HostInputSink`
    /// translator with the `Emulator` acting as the sink (single-threaded wasm
    /// applies events immediately).
    fn process_mouse(&mut self, ctx: &egui::Context, image_rect: egui::Rect) {
        let hovering = ctx
            .input(|input| input.pointer.hover_pos().is_some_and(|p| image_rect.contains(p)));
        if !hovering {
            return;
        }
        let Some(emu) = &mut self.emulator else {
            return;
        };
        let prev = self.web_prev_mouse_buttons;
        self.web_prev_mouse_buttons = ctx.input(|input| {
            rusty_box::gui::host_input::translate_egui_mouse(input, prev, &mut **emu)
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

        // Crisp integer upscale (NEAREST magnify), smooth downscale (LINEAR minify).
        let options = egui::TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        };
        match &mut self.texture {
            Some(texture) => texture.set(image, options),
            None => {
                self.texture = Some(ctx.load_texture("vga_display", image, options));
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
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button(WEB_BOOT_MEDIA_ACTION_LABEL).clicked() {
                            self.open_file_picker();
                            ui.close();
                        }
                        if ui.button("Create Disk Image").clicked() {
                            self.chrome.go_to(ShellPage::Images);
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
            .show(ui, |ui| {
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
                        self.chrome.go_to(ShellPage::Hardware);
                    }
                    if ui.button("+ New Image").clicked() {
                        self.chrome.go_to(ShellPage::Images);
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
            .show(ui, |ui| {
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
                ui.label(RichText::new("⏷ My Computer").color(TEXT_MUTED));
                let visible = self.chrome.visible_vm_indices();
                for index in visible {
                    let clicked = {
                        let entry = &self.chrome.vm_library[index];
                        let selected = ui.selectable_label(
                            self.chrome.selected_vm() == index,
                            format!("  ▣ {}", entry.name),
                        );
                        if self.chrome.selected_vm() == index {
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
                        self.chrome.destination = Destination::new(index, ShellPage::Home);
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
            .show(ui, |ui| {
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
            .show(ui, |ui| match self.chrome.page() {
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
                    ACCENT_CYAN,
                    ActionTileWeight::Primary,
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
                    ActionTileWeight::Secondary,
                    || self.chrome.go_to(ShellPage::Images),
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
                    .unwrap_or(WebStartupStage::Announce);
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
                let tex_w = (self.display.fb_width.max(1)) as f32;
                let tex_h = self.display.fb_height.max(1) as f32;
                // Scale in PHYSICAL pixels (see eframe_app.rs): a point-space integer
                // scale becomes a fractional physical scale on HiDPI -> uneven NEAREST
                // pixels. Snap upscales to an integer physical multiple; fractional-fit
                // a guest larger than the panel (LINEAR minify handles it).
                let ppp = ui.ctx().pixels_per_point().max(f32::EPSILON);
                let fit = ((available.x * ppp) / tex_w).min((available.y * ppp) / tex_h);
                let size = if fit < 1.0 {
                    egui::vec2(tex_w * fit / ppp, tex_h * fit / ppp)
                } else {
                    let iscale = fit.floor().max(1.0);
                    egui::vec2(tex_w * iscale / ppp, tex_h * iscale / ppp)
                };
                let mut image_rect = None;
                ui.centered_and_justified(|ui| {
                    let response = ui.image(egui::load::SizedTexture::new(texture.id(), size));
                    image_rect = Some(response.rect);
                });
                if let Some(rect) = image_rect {
                    self.process_mouse(ui.ctx(), rect);
                }
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
                page_header(
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
                page_header(
                    ui,
                    "Cooperative CPU",
                    "Select virtual processors before boot. Browser execution still uses frame-sized batches to keep the UI responsive.",
                );
                detail_row(ui, "Virtual processors", &self.web_cpu_count.to_string());
                detail_row(ui, "Execution", "Cooperative frame batches");
                let can_edit = web_can_edit_cpu_count(self.web_has_vm());
                let mut cpu_count = self.web_cpu_count;
                ui.add_enabled_ui(can_edit, |ui| {
                    let changed = ui
                        .add(
                            egui::DragValue::new(&mut cpu_count)
                                .range(WEB_MIN_CPU_COUNT..=WEB_MAX_CPU_COUNT)
                                .speed(1.0),
                        )
                        .changed();
                    ui.label(
                        RichText::new(format!(
                            "Range: {} – {}",
                            WEB_MIN_CPU_COUNT, WEB_MAX_CPU_COUNT
                        ))
                        .color(TEXT_MUTED),
                    );
                    if changed {
                        self.set_web_cpu_count(cpu_count);
                    }
                });
                if !can_edit {
                    ui.label(
                        RichText::new("Reset the browser VM before changing processors.")
                            .color(TEXT_MUTED),
                    );
                }
            }
            HardwareDevice::Devices => {
                page_header(
                    ui,
                    "Browser devices",
                    "The browser build exposes a fixed virtual machine profile and does not persist hardware edits.",
                );
                detail_row(ui, "Edit mode", "Read-only");
                detail_row(ui, "Boot media", "Upload on Home");
            }
            HardwareDevice::HardDisk => {
                page_header(
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
                page_header(
                    ui,
                    "Uploaded boot media",
                    "Home opens a browser file picker and attaches the selected image as bootable CD/DVD media.",
                );
                detail_row(ui, "Attached media", &attached_media);
                detail_row(ui, "Boot mode", "Uploaded boot image");
            }
            HardwareDevice::Display => {
                page_header(
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
            .selectable_label(self.chrome.page() == page, label)
            .clicked()
        {
            self.chrome.go_to(page);
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
            self.chrome.go_to(ShellPage::Home);
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
        if self.chrome.page() == ShellPage::Console {
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
            ui.label(RichText::new("Rusty Box Workstation").size(TEXT_TITLE).strong());
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

#[cfg(target_os = "android")]
fn draw_u32_field(
    ui: &mut egui::Ui,
    value: &mut u32,
    min: u32,
    max: u32,
    suffix: &str,
    editable: bool,
    step: u32,
    _speed: Option<f64>,
) -> bool {
    let mut changed = false;
    let step = step.max(1);
    let decrement = value.saturating_sub(step).max(min);
    if ui
        .add_enabled(editable, egui::Button::new("−"))
        .on_hover_text("decrement")
        .clicked()
        && *value != decrement
    {
        *value = decrement;
        changed = true;
    }
    ui.label(RichText::new(format!("{value}{suffix}")).color(TEXT_MUTED));
    let increment = value.saturating_add(step).min(max);
    if ui
        .add_enabled(editable, egui::Button::new("+"))
        .on_hover_text("increment")
        .clicked()
        && *value != increment
    {
        *value = increment;
        changed = true;
    }
    changed
}

#[cfg(not(target_os = "android"))]
fn draw_u32_field(
    ui: &mut egui::Ui,
    value: &mut u32,
    min: u32,
    max: u32,
    suffix: &str,
    editable: bool,
    speed: u32,
    speed_override: Option<f64>,
) -> bool {
    let mut widget = egui::DragValue::new(value).range(min..=max).suffix(suffix);
    if let Some(speed_override) = speed_override {
        widget = widget.speed(speed_override);
    } else {
        widget = widget.speed(speed as f64);
    }
    ui.add_enabled(editable, widget).changed()
}

#[cfg(target_os = "android")]
fn draw_u64_field(
    ui: &mut egui::Ui,
    value: &mut u64,
    min: u64,
    max: u64,
    suffix: &str,
    editable: bool,
    step: u64,
    _speed: Option<f64>,
) -> bool {
    let mut changed = false;
    let step = step.max(1);
    let decrement = value.saturating_sub(step).max(min);
    if ui
        .add_enabled(editable, egui::Button::new("−"))
        .on_hover_text("decrement")
        .clicked()
        && *value != decrement
    {
        *value = decrement;
        changed = true;
    }
    ui.label(RichText::new(format!("{value}{suffix}")).color(TEXT_MUTED));
    let increment = value.saturating_add(step).min(max);
    if ui
        .add_enabled(editable, egui::Button::new("+"))
        .on_hover_text("increment")
        .clicked()
        && *value != increment
    {
        *value = increment;
        changed = true;
    }
    changed
}

#[cfg(not(target_os = "android"))]
fn draw_u64_field(
    ui: &mut egui::Ui,
    value: &mut u64,
    min: u64,
    max: u64,
    suffix: &str,
    editable: bool,
    speed: u64,
    speed_override: Option<f64>,
) -> bool {
    let mut widget = egui::DragValue::new(value).range(min..=max).suffix(suffix);
    if let Some(speed_override) = speed_override {
        widget = widget.speed(speed_override);
    } else {
        widget = widget.speed(speed as f64);
    }
    ui.add_enabled(editable, widget).changed()
}

/// A read-only fact on the pane's grid: the caption in the label column, the
/// value beside it, wrapping so a long path stays inside the card.
fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    field_row(ui, label, |ui| {
        ui.add(egui::Label::new(RichText::new(value).color(TEXT_PRIMARY)).wrap());
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn cpu_count_label(cpus: u32) -> String {
    if cpus == 1 {
        "1 CPU".to_owned()
    } else {
        format!("{cpus} CPUs")
    }
}

/// The badge for a runner status: cyan runs, amber waits, red has faulted, and
/// idle is muted.
#[cfg(not(target_arch = "wasm32"))]
fn shell_state_badge(status: &ShellStatus, faulted: bool) -> ShellStateBadge {
    if status.running {
        ShellStateBadge {
            label: "Running",
            color: ACCENT_CYAN,
        }
    } else if status.start_pending {
        ShellStateBadge {
            label: "Starting",
            color: ACCENT_AMBER,
        }
    } else if faulted {
        ShellStateBadge {
            label: "Faulted",
            color: ACCENT_RED,
        }
    } else {
        ShellStateBadge {
            label: "Stopped",
            color: TEXT_MUTED,
        }
    }
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

/// The Console's powered-off display: the shell's copy on the shell's type
/// scale, laid out as one centred block so the embedded view can place it as a
/// single label. The `rusty_box` crate learns neither the palette nor that the
/// power verbs live in a bar above the console.
#[cfg(not(target_arch = "wasm32"))]
fn powered_off_placeholder() -> rusty_box::gui::ConsolePlaceholder {
    use egui::text::{LayoutJob, TextFormat};
    use egui::{Align, FontId};

    let mut job = LayoutJob::default();
    job.halign = Align::Center;
    job.append(
        "This VM is powered off\n",
        0.0,
        TextFormat {
            font_id: FontId::proportional(TEXT_TITLE),
            color: TEXT_MUTED,
            ..Default::default()
        },
    );
    job.append(
        "Power on in the bar above to start it.",
        0.0,
        TextFormat {
            font_id: FontId::proportional(TEXT_CAPTION),
            color: TEXT_MUTED,
            ..Default::default()
        },
    );
    rusty_box::gui::ConsolePlaceholder(job)
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
            "browser downloads are capped at 64 MiB; use the desktop app for larger disks"
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

/// What an engine is called in the window.
fn engine_label(engine: crate::config::Engine) -> &'static str {
    match engine {
        crate::config::Engine::Interpreter => "Interpreter",
        crate::config::Engine::Whp => "Windows Hypervisor",
    }
}

#[cfg(test)]
#[cfg(feature = "gui-egui")]
mod tests {
    use super::*;
    use std::fs;

    /// A path no other test of this process has: the clock alone is not
    /// enough, since two tests can start within one of its ticks.
    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "{name}-{}-{nanos}-{sequence}.img",
            std::process::id()
        ))
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
            engine: crate::config::Engine::Interpreter,
            cpu_capabilities: crate::config::CpuCapabilities::Preset,
            memory_mib: 256,
            host_memory_mib: 256,
            memory_block_kib: 128,
            ips: 300_000_000,
            pci: true,
            sync_slowdown: false,
            sync_realtime: false,
            smp_quantum: 16,
            cpuid_freq: rusty_box::CpuidFreq::None,
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
            cpu_params: BxParams::default(),
            log_level: crate::args::LogLevel::Warn,
            vga_mode: None,
            pci_vga: false,
        }
    }

    /// A scratch library folder, removed when the test ends.
    #[cfg(not(target_arch = "wasm32"))]
    struct ScratchLibrary {
        dir: std::path::PathBuf,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl ScratchLibrary {
        fn new() -> Self {
            let dir = unique_temp_path("rusty-box-gui-library").with_extension("");
            fs::create_dir_all(&dir).expect("create scratch library");
            Self { dir }
        }

        fn library(&self) -> crate::library::VmLibrary {
            crate::library::VmLibrary::open(self.dir.clone()).expect("open scratch library")
        }

        fn toml_files(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.dir)
                .expect("list scratch library")
                .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".toml"))
                .collect();
            names.sort();
            names
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl Drop for ScratchLibrary {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.dir) {
                eprintln!("could not remove {}: {error}", self.dir.display());
            }
        }
    }

    /// A shell opened the way the command line opens it: `test_resolved_config()`
    /// as the temporary launch VM over an empty scratch library.
    #[cfg(not(target_arch = "wasm32"))]
    fn native_test_app() -> (
        NativeShellApp,
        std::sync::mpsc::Receiver<NativeEmulatorCommand>,
        ScratchLibrary,
    ) {
        let scratch = ScratchLibrary::new();
        let launch = crate::runner::LaunchVm {
            name: "Rusty Box".to_owned(),
            config: test_resolved_config(),
            source: crate::runner::LaunchSource::Flags,
        };
        let (app, command_rx) = native_test_app_over(&scratch, Some(launch), None);
        (app, command_rx, scratch)
    }

    /// A shell opened over `scratch`, on the library it holds: `launch` as
    /// the temporary VM when there is one, and `notice` as the start's message.
    #[cfg(not(target_arch = "wasm32"))]
    fn native_test_app_over(
        scratch: &ScratchLibrary,
        launch: Option<crate::runner::LaunchVm>,
        notice: Option<String>,
    ) -> (
        NativeShellApp,
        std::sync::mpsc::Receiver<NativeEmulatorCommand>,
    ) {
        let shared = Arc::new(Mutex::new(
            rusty_box::gui::shared_display::SharedDisplay::new(),
        ));
        let (command_tx, command_rx) = std::sync::mpsc::channel();
        let emulator = rusty_box::gui::RustyBoxApp::new_embedded(Arc::clone(&shared));
        let start = crate::runner::ShellStart {
            library: scratch.library(),
            opening: launch.map_or(
                crate::runner::ShellOpening::LastShown,
                crate::runner::ShellOpening::Launch,
            ),
            notice,
        };
        (
            NativeShellApp::with_emulator(emulator, shared, command_tx, start),
            command_rx,
        )
    }

    /// A shell opened over a scratch library holding one VM, "Alpine", and
    /// no launch VM, so the library VM is the one selected.
    #[cfg(not(target_arch = "wasm32"))]
    fn library_app() -> (
        NativeShellApp,
        std::sync::mpsc::Receiver<NativeEmulatorCommand>,
        ScratchLibrary,
    ) {
        let scratch = ScratchLibrary::new();
        scratch.library().create("Alpine", &test_resolved_config()).expect("seed");
        let (app, command_rx) = native_test_app_over(&scratch, None, None);
        (app, command_rx, scratch)
    }

    /// What the start could not do is the first thing the shell shows.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_start_notice_opens_the_shell_with_that_warning() {
        let scratch = ScratchLibrary::new();

        let (app, _command_rx) = native_test_app_over(
            &scratch,
            None,
            Some("The VM bundled in C:\\app was not imported: bad file".to_owned()),
        );

        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "The VM bundled in C:\\app was not imported: bad file"
            ))
        );
        assert_eq!(app.profiles[0].name, "New VM");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_launch_vm_comes_first_selected_and_unsaved() {
        let scratch = ScratchLibrary::new();
        scratch.library().create("Win 7", &test_resolved_config()).expect("seed");
        let launch = crate::runner::LaunchVm {
            name: "Command line".to_owned(),
            config: test_resolved_config(),
            source: crate::runner::LaunchSource::Flags,
        };

        let (app, _command_rx) = native_test_app_over(&scratch, Some(launch), None);

        assert_eq!(app.profiles.len(), 2);
        assert_eq!(app.chrome.selected_vm(), 0);
        assert_eq!(app.profiles[0].origin, VmOrigin::Launch);
        assert_eq!(app.chrome.vm_library[0].source, crate::shell::sidebar::EntrySource::Unsaved);
        assert_eq!(app.chrome.vm_library[1].name, "Win 7");
        assert_eq!(app.chrome.vm_library[1].source, crate::shell::sidebar::EntrySource::Saved);
        // Opening the shell writes nothing: the launch VM stays in memory.
        assert_eq!(scratch.toml_files(), ["win-7.toml"]);
    }

    /// The library VM the command line named by its file is the one shown,
    /// over the VM shown last, and no temporary VM is listed beside it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_library_vm_named_on_the_command_line_is_selected_with_no_temporary_vm() {
        let scratch = ScratchLibrary::new();
        let library = scratch.library();
        let alpha = library.create("Alpha", &test_resolved_config()).expect("alpha");
        let beta = library.create("Beta", &test_resolved_config()).expect("beta");
        library.remember_selected(&alpha).expect("remember");
        let start = crate::runner::ShellStart {
            library,
            opening: crate::runner::ShellOpening::LibraryVm(beta.clone()),
            notice: None,
        };

        let opening = OpeningList::from_start(&start);

        assert_eq!(opening.profiles.len(), 2);
        assert!(opening
            .profiles
            .iter()
            .all(|profile| matches!(profile.origin, VmOrigin::Library(_))));
        assert_eq!(opening.profiles[opening.selected].origin, VmOrigin::Library(beta));
        assert_eq!(opening.profiles[opening.selected].name, "Beta");
        assert_eq!(scratch.toml_files(), ["alpha.toml", "beta.toml"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn without_a_launch_vm_the_shell_opens_on_the_vm_shown_last() {
        let scratch = ScratchLibrary::new();
        let library = scratch.library();
        library.create("Alpha", &test_resolved_config()).expect("alpha");
        let beta = library.create("Beta", &test_resolved_config()).expect("beta");
        library.remember_selected(&beta).expect("remember");

        let (app, _command_rx) = native_test_app_over(&scratch, None, None);

        assert_eq!(app.profiles.len(), 2);
        assert_eq!(app.chrome.selected_vm(), 1);
        assert_eq!(app.vm_info.name, "Beta");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn with_nothing_to_show_the_shell_opens_on_a_blank_new_vm() {
        let scratch = ScratchLibrary::new();

        let (app, _command_rx) = native_test_app_over(&scratch, None, None);

        assert_eq!(app.profiles.len(), 1);
        assert_eq!(app.profiles[0].name, "New VM");
        assert_eq!(app.profiles[0].origin, VmOrigin::Launch);
        assert_eq!(app.config, crate::config::blank_config());
        // The blank VM is in memory only; the library is as empty as it was.
        assert_eq!(scratch.toml_files(), Vec::<String>::new());
    }

    /// A library folder that cannot be read does not refuse the shell: it
    /// opens on the blank VM, with the error where the user can see it, ahead
    /// of any message the start carried.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_library_that_cannot_be_read_opens_on_a_blank_vm_with_a_notice() {
        let scratch = ScratchLibrary::new();
        let library = scratch.library();
        fs::remove_dir_all(&scratch.dir).expect("remove the folder under the library");
        let start = crate::runner::ShellStart {
            library,
            opening: crate::runner::ShellOpening::LastShown,
            notice: Some("a bundled VM was not imported".to_owned()),
        };

        let opening = OpeningList::from_start(&start);

        assert_eq!(opening.profiles.len(), 1);
        assert_eq!(opening.profiles[0].name, "New VM");
        assert!(opening.broken.is_empty());
        assert!(
            matches!(
                &opening.notice,
                Some(notice) if notice.kind == ShellNoticeKind::Error
                    && notice.message.contains("failed to read the VM library")
            ),
            "notice: {:?}",
            opening.notice
        );
        fs::create_dir_all(&scratch.dir).expect("restore the folder for the scratch cleanup");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_library_file_that_does_not_load_is_listed_as_broken() {
        let scratch = ScratchLibrary::new();
        fs::write(scratch.dir.join("bad.toml"), "memory_mib = [").expect("write");

        let (app, _command_rx) = native_test_app_over(&scratch, None, None);

        assert_eq!(app.broken_files.len(), 1);
        assert_eq!(app.broken_files[0].path, scratch.dir.join("bad.toml"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_applied_edit_is_written_by_the_next_flush() {
        let (mut app, _command_rx, scratch) = library_app();

        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();

        assert_eq!(app.profiles[0].save_state, SaveState::Unsaved);
        assert_eq!(scratch.library().load().expect("reload").vms[0].config.memory_mib, 256);

        app.flush_unsaved();

        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
        assert_eq!(scratch.library().load().expect("reload").vms[0].config.memory_mib, 512);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_renamed_vm_keeps_its_file() {
        let (mut app, _command_rx, scratch) = library_app();

        app.profiles[0].name = "Alpine edge".to_owned();
        app.apply_pending_settings().unwrap();
        app.flush_unsaved();

        assert_eq!(scratch.toml_files(), ["alpine.toml"]);
        assert_eq!(scratch.library().load().expect("reload").vms[0].name, "Alpine edge");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn edits_to_the_launch_vm_write_nothing() {
        let (mut app, _command_rx, scratch) = native_test_app();

        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        app.flush_unsaved();

        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
        assert!(scratch.toml_files().is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn keeping_the_launch_vm_puts_it_in_the_library() {
        let (mut app, _command_rx, scratch) = native_test_app();

        app.keep_selected_in_library();

        assert_eq!(scratch.toml_files(), ["rusty-box.toml"]);
        assert!(matches!(app.profiles[0].origin, VmOrigin::Library(_)));
        assert_eq!(app.chrome.vm_library[0].source, crate::shell::sidebar::EntrySource::Saved);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::info("Saved Rusty Box to the VM library."))
        );
        app.settings.memory_mib = 768;
        app.apply_pending_settings().unwrap();
        app.flush_unsaved();
        assert_eq!(scratch.library().load().expect("reload").vms[0].config.memory_mib, 768);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_new_vm_is_a_library_copy_of_the_selected_one() {
        let (mut app, _command_rx, scratch) = library_app();
        app.settings.memory_mib = 384;
        app.apply_pending_settings().unwrap();

        app.add_vm_copying_selected();

        assert_eq!(app.profiles.len(), 2);
        assert_eq!(app.chrome.selected_vm(), 1);
        assert_eq!(app.vm_info.name, "Alpine copy");
        assert_eq!(scratch.toml_files(), ["alpine-copy.toml", "alpine.toml"]);
        let reloaded = scratch.library().load().expect("reload");
        assert!(reloaded.vms.iter().all(|vm| vm.config.memory_mib == 384));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn deleting_a_vm_asks_first_and_then_removes_its_file() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();

        app.request_delete_selected();
        assert_eq!(
            app.pending_confirm,
            Some(PendingConfirm::DeleteVm {
                index: 1,
                origin: VmOrigin::Library(crate::library::VmStem::parse("alpine-copy").unwrap()),
                name: "Alpine copy".to_owned(),
            })
        );
        assert_eq!(scratch.toml_files().len(), 2);

        app.confirm_pending();

        assert_eq!(app.pending_confirm, None);
        assert_eq!(app.profiles.len(), 1);
        assert_eq!(scratch.toml_files(), ["alpine.toml"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_cancelled_delete_keeps_the_vm() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();

        app.request_delete_selected();
        app.cancel_pending();

        assert_eq!(app.profiles.len(), 2);
        assert_eq!(scratch.toml_files().len(), 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selecting_a_vm_remembers_it_for_the_next_launch() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();

        app.select_profile(0);

        let reloaded = scratch.library().load().expect("reload");
        let alpine = reloaded
            .vms
            .iter()
            .find(|vm| vm.name == "Alpine")
            .expect("the seeded VM")
            .stem
            .clone();
        assert_eq!(alpine.as_str(), "alpine");
        assert_eq!(scratch.library().last_selected(), Some(alpine));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_broken_file_is_deleted_only_after_confirmation() {
        let scratch = ScratchLibrary::new();
        let bad = scratch.dir.join("bad.toml");
        fs::write(&bad, "memory_mib = [").expect("write");
        let (mut app, _command_rx) = native_test_app_over(&scratch, None, None);

        app.pending_confirm = Some(PendingConfirm::DeleteBroken(bad.clone()));
        assert!(bad.exists());
        app.confirm_pending();

        assert!(!bad.exists());
        assert!(app.broken_files.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selecting_another_vm_writes_the_one_being_left() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();
        app.settings.memory_mib = 640;
        app.apply_pending_settings().unwrap();

        app.select_profile(0);

        let reloaded = scratch.library().load().expect("reload");
        let copy = reloaded
            .vms
            .iter()
            .find(|vm| vm.name == "Alpine copy")
            .expect("the copy");
        assert_eq!(copy.config.memory_mib, 640);
        assert!(app
            .profiles
            .iter()
            .all(|profile| profile.save_state == SaveState::Saved));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_failed_flush_keeps_the_vm_unsaved_and_shows_the_error() {
        let (mut app, _command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        let file = scratch.dir.join("alpine.toml");
        fs::remove_file(&file).expect("remove");
        fs::create_dir(&file).expect("a folder in the file's way");

        app.flush_unsaved();

        assert_eq!(app.profiles[0].save_state, SaveState::WriteFailed);
        assert!(
            matches!(&app.shell_notice, Some(notice) if notice.kind == ShellNoticeKind::Error),
            "notice: {:?}",
            app.shell_notice
        );

        fs::remove_dir(&file).expect("clear the way");
        app.flush_unsaved();

        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
        assert_eq!(scratch.library().load().expect("reload").vms[0].config.memory_mib, 512);
    }

    /// The memory the seeded VM's file holds, reloaded from the folder.
    #[cfg(not(target_arch = "wasm32"))]
    fn memory_in_file(scratch: &ScratchLibrary) -> u32 {
        scratch.library().load().expect("reload").vms[0].config.memory_mib
    }

    /// Runs `body` inside one egui pass with nothing drawn, the way the
    /// frame-end flush runs inside `ui`. Nothing is drawn, so the pass paints
    /// nothing; that is all its output is checked for.
    #[cfg(not(target_arch = "wasm32"))]
    fn in_a_pass(ctx: &egui::Context, body: impl FnOnce(&egui::Context)) {
        in_a_pass_with(ctx, egui::RawInput::default(), body);
    }

    /// `in_a_pass` over the frame's `input`.
    #[cfg(not(target_arch = "wasm32"))]
    fn in_a_pass_with(
        ctx: &egui::Context,
        input: egui::RawInput,
        body: impl FnOnce(&egui::Context),
    ) {
        ctx.begin_pass(input);
        body(ctx);
        let output = ctx.end_pass();
        assert!(output.shapes.is_empty(), "a pass with nothing drawn paints nothing");
    }

    /// The frame on which the window went behind another, as egui-winit
    /// reports it: on Android, the activity leaving the foreground.
    #[cfg(not(target_arch = "wasm32"))]
    fn window_focus_lost() -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::WindowFocused(false)],
            focused: false,
            ..egui::RawInput::default()
        }
    }

    /// eframe's storage as the shell sees it: nothing is ever put in it, so
    /// it keeps nothing. `save` is the shell's hook for the activity's window
    /// being taken away, not a store.
    #[cfg(not(target_arch = "wasm32"))]
    struct NoStore;

    #[cfg(not(target_arch = "wasm32"))]
    impl eframe::Storage for NoStore {
        fn get_string(&self, _key: &str) -> Option<String> {
            None
        }

        fn set_string(&mut self, _key: &str, _value: String) {}

        fn remove_string(&mut self, _key: &str) {}

        fn flush(&mut self) {}
    }

    /// Every entry of the scratch folder by name, dot files included.
    #[cfg(not(target_arch = "wasm32"))]
    fn all_entries(scratch: &ScratchLibrary) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&scratch.dir)
            .expect("list scratch library")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_failed_write_waits_for_the_next_action() {
        let (mut app, command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        let file = scratch.dir.join("alpine.toml");
        fs::remove_file(&file).expect("remove");
        fs::create_dir(&file).expect("a folder in the file's way");
        app.flush_unsaved();
        assert_eq!(app.profiles[0].save_state, SaveState::WriteFailed);
        assert!(
            matches!(&app.shell_notice, Some(notice) if notice.kind == ShellNoticeKind::Error),
            "notice: {:?}",
            app.shell_notice
        );

        // Closed by the user; an idle frame neither tries again nor reopens it.
        app.shell_notice = None;
        let ctx = egui::Context::default();
        in_a_pass(&ctx, |ctx| app.flush_unsaved_when_idle(ctx));

        assert_eq!(app.shell_notice, None);
        assert_eq!(app.profiles[0].save_state, SaveState::WriteFailed);
        assert_eq!(all_entries(&scratch), ["alpine.toml"]);

        // An action tries again: still blocked, so the error is raised once more.
        app.select_profile(0);

        assert!(
            matches!(&app.shell_notice, Some(notice) if notice.kind == ShellNoticeKind::Error),
            "notice: {:?}",
            app.shell_notice
        );
        assert_eq!(app.profiles[0].save_state, SaveState::WriteFailed);

        // The way cleared, the next action writes it.
        fs::remove_dir(&file).expect("clear the way");
        app.start_vm();

        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
        assert_eq!(memory_in_file(&scratch), 512);
        assert!(matches!(
            command_rx.try_recv(),
            Ok(NativeEmulatorCommand::Start(config)) if config.memory_mib == 512
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_summary_caption_says_whether_the_vm_is_saved() {
        let (mut app, _command_rx, scratch) = library_app();
        let file = scratch.dir.join("alpine.toml");

        assert_eq!(
            app.summary_caption(),
            format!("Saved automatically to {}", file.display())
        );

        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();

        assert_eq!(
            app.summary_caption(),
            format!("Saving to {} when the edit ends", file.display())
        );

        fs::remove_file(&file).expect("remove");
        fs::create_dir(&file).expect("a folder in the file's way");
        app.flush_unsaved();

        assert_eq!(
            app.summary_caption(),
            format!(
                "Not saved: the last write to {} failed. \
                 It is tried again at the next change, selection or power-on.",
                file.display()
            )
        );

        let (launch_app, _launch_rx, _launch_scratch) = native_test_app();
        assert_eq!(
            launch_app.summary_caption(),
            "Temporary VM. Not saved until it is kept in the library."
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_vm_whose_write_failed_is_marked_save_failed_in_the_sidebar() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        assert_eq!(app.chrome.vm_library[1].row_label(), "Alpine copy");
        let copy_file = scratch.dir.join("alpine-copy.toml");
        fs::remove_file(&copy_file).expect("remove");
        fs::create_dir(&copy_file).expect("a folder in the file's way");

        app.select_profile(0);

        assert_eq!(app.profiles[1].save_state, SaveState::WriteFailed);
        assert_eq!(
            app.chrome.vm_library[1].source,
            crate::shell::sidebar::EntrySource::WriteFailed
        );
        assert_eq!(app.chrome.vm_library[1].row_label(), "Alpine copy (save failed)");

        fs::remove_dir(&copy_file).expect("clear the way");
        app.flush_unsaved();

        assert_eq!(app.profiles[1].save_state, SaveState::Saved);
        assert_eq!(app.chrome.vm_library[1].row_label(), "Alpine copy");
    }

    /// Appends a line to the file at `path`, the way a hand edit outside the
    /// shell would, and returns what the file then holds.
    #[cfg(not(target_arch = "wasm32"))]
    fn hand_edit(path: &std::path::Path) -> String {
        let hand_edited = format!(
            "{}\n# edited outside the shell\n",
            fs::read_to_string(path).expect("read")
        );
        fs::write(path, &hand_edited).expect("write");
        hand_edited
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_unchanged_vm_is_not_rewritten_by_a_selection_or_a_power_on() {
        let (mut app, command_rx, scratch) = library_app();
        app.add_vm_copying_selected();
        let copy_file = scratch.dir.join("alpine-copy.toml");
        let copy_hand_edited = hand_edit(&copy_file);

        app.select_profile(0);

        assert_eq!(fs::read_to_string(&copy_file).expect("read"), copy_hand_edited);

        let file = scratch.dir.join("alpine.toml");
        let hand_edited = hand_edit(&file);

        app.start_vm();

        assert_eq!(fs::read_to_string(&file).expect("read"), hand_edited);
        assert!(matches!(
            command_rx.try_recv(),
            Ok(NativeEmulatorCommand::Start(_))
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_idle_flush_waits_for_focus_and_drags_to_end() {
        let (mut app, _command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        let ctx = egui::Context::default();
        let field = egui::Id::new("field");

        in_a_pass(&ctx, |ctx| {
            ctx.memory_mut(|memory| memory.request_focus(field));
            app.flush_unsaved_when_idle(ctx);
        });
        assert_eq!(memory_in_file(&scratch), 256);

        in_a_pass(&ctx, |ctx| {
            ctx.memory_mut(|memory| memory.surrender_focus(field));
            ctx.set_dragged_id(field);
            app.flush_unsaved_when_idle(ctx);
        });
        assert_eq!(memory_in_file(&scratch), 256);

        in_a_pass(&ctx, |ctx| {
            ctx.stop_dragging();
            app.flush_unsaved_when_idle(ctx);
        });
        assert_eq!(memory_in_file(&scratch), 512);
        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
    }

    /// An edit in a field that still has focus reaches its file on the frame
    /// the window goes behind another — on a phone, the activity leaving the
    /// foreground — without waiting for the field to be left; a window that
    /// keeps focus waits as before.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_edit_still_being_typed_is_written_when_the_window_loses_focus() {
        let (mut app, _command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        let ctx = egui::Context::default();
        let field = egui::Id::new("field");

        in_a_pass(&ctx, |ctx| {
            ctx.memory_mut(|memory| memory.request_focus(field));
            app.flush_when_window_focus_is_lost(ctx);
            app.flush_unsaved_when_idle(ctx);
        });
        assert_eq!(memory_in_file(&scratch), 256);
        assert_eq!(app.profiles[0].save_state, SaveState::Unsaved);

        in_a_pass_with(&ctx, window_focus_lost(), |ctx| {
            ctx.memory_mut(|memory| memory.request_focus(field));
            app.flush_when_window_focus_is_lost(ctx);
        });
        assert_eq!(memory_in_file(&scratch), 512);
        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
    }

    /// A write that failed is tried again by the loss of focus, as by any
    /// action, and by nothing while the window stays unfocused.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn losing_focus_retries_a_failed_write_once() {
        let (mut app, _command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        let file = scratch.dir.join("alpine.toml");
        fs::remove_file(&file).expect("remove");
        fs::create_dir(&file).expect("a folder in the file's way");
        app.flush_unsaved();
        assert_eq!(app.profiles[0].save_state, SaveState::WriteFailed);
        app.shell_notice = None;
        let ctx = egui::Context::default();

        // Unfocused frames after the loss neither try again nor say anything.
        in_a_pass_with(
            &ctx,
            egui::RawInput {
                focused: false,
                ..egui::RawInput::default()
            },
            |ctx| app.flush_when_window_focus_is_lost(ctx),
        );
        assert_eq!(app.profiles[0].save_state, SaveState::WriteFailed);
        assert_eq!(app.shell_notice, None);

        // The way cleared, the next loss of focus writes it.
        fs::remove_dir(&file).expect("clear the way");
        in_a_pass_with(&ctx, window_focus_lost(), |ctx| {
            app.flush_when_window_focus_is_lost(ctx);
        });
        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
        assert_eq!(memory_in_file(&scratch), 512);
    }

    /// eframe calls `save` when the activity's window is taken away, which
    /// on a phone is the last chance before the process can be killed: the
    /// edits still in memory reach their files then, and nothing else is
    /// persisted — no egui memory, and no periodic save that would write a
    /// field mid-edit.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_activity_whose_window_is_taken_away_writes_its_edits() {
        let (mut app, _command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        assert_eq!(memory_in_file(&scratch), 256);

        eframe::App::save(&mut app, &mut NoStore);

        assert_eq!(memory_in_file(&scratch), 512);
        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
        assert!(!eframe::App::persist_egui_memory(&app));
        assert_eq!(
            eframe::App::auto_save_interval(&app),
            std::time::Duration::MAX
        );
    }

    /// A phone draws the notices that report something that did not happen —
    /// a launch whose seed was not finished, a save that did not reach its
    /// file — and not the ones that report something that did.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_phone_shows_what_did_not_happen_and_not_what_did() {
        let scratch = ScratchLibrary::new();
        let (app, _command_rx) = native_test_app_over(
            &scratch,
            None,
            Some("The settings saved in /data/rusty_box.toml were not imported: bad file.".to_owned()),
        );
        let start = app.shell_notice.clone().expect("the start's notice");
        assert!(phone_shows_notice(start.kind), "hidden on a phone: {start:?}");

        let (mut app, _command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        let file = scratch.dir.join("alpine.toml");
        fs::remove_file(&file).expect("remove");
        fs::create_dir(&file).expect("a folder in the file's way");
        app.flush_unsaved();
        let failed = app.shell_notice.clone().expect("the failed write's notice");
        assert_eq!(failed.kind, ShellNoticeKind::Error);
        assert!(phone_shows_notice(failed.kind), "hidden on a phone: {failed:?}");

        let (mut app, _command_rx, _scratch) = native_test_app();
        app.keep_selected_in_library();
        let kept = app.shell_notice.clone().expect("the kept VM's notice");
        assert_eq!(kept.kind, ShellNoticeKind::Info);
        assert!(!phone_shows_notice(kept.kind), "shown on a phone: {kept:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_power_on_writes_the_vm_first() {
        let (mut app, command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();

        app.start_vm();

        assert_eq!(memory_in_file(&scratch), 512);
        assert!(matches!(
            command_rx.try_recv(),
            Ok(NativeEmulatorCommand::Start(config)) if config.memory_mib == 512
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn closing_the_window_writes_the_unsaved_vm() {
        let (mut app, _command_rx, scratch) = library_app();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();

        eframe::App::on_exit(&mut app, None);

        assert_eq!(memory_in_file(&scratch), 512);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_deleted_vm_leaves_no_unsaved_edit_behind() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        assert_eq!(app.profiles[1].save_state, SaveState::Unsaved);

        app.request_delete_selected();
        app.confirm_pending();
        app.flush_unsaved();

        assert_eq!(scratch.toml_files(), ["alpine.toml"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_flush_error_outranks_the_remember_warning_of_the_same_selection() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();
        let copy_file = scratch.dir.join("alpine-copy.toml");
        fs::remove_file(&copy_file).expect("remove");
        fs::create_dir(&copy_file).expect("a folder in the file's way");
        let record = scratch.dir.join(".last");
        fs::remove_file(&record).expect("remove the record");
        fs::create_dir(&record).expect("a folder where the record goes");

        app.select_profile(0);

        assert_eq!(app.chrome.selected_vm(), 0);
        assert_eq!(app.profiles[1].save_state, SaveState::WriteFailed);
        assert!(
            matches!(&app.shell_notice, Some(notice) if notice.kind == ShellNoticeKind::Error),
            "notice: {:?}",
            app.shell_notice
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_vm_whose_settings_no_longer_apply_is_still_deleted() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();
        app.settings.disk_enabled = true;
        app.settings.disk_path.clear();
        assert_eq!(
            app.apply_pending_settings(),
            Err("Hard disk path is required when hard disk is enabled".to_owned())
        );

        app.request_delete_selected();
        app.confirm_pending();

        assert_eq!(app.profiles.len(), 1);
        assert_eq!(scratch.toml_files(), ["alpine.toml"]);
        assert!(
            !matches!(&app.shell_notice, Some(notice) if notice.kind == ShellNoticeKind::Error),
            "notice: {:?}",
            app.shell_notice
        );
    }

    /// A VM the way a phone's first VM is made, or a bundled VM that names
    /// only its ROMs: a BIOS, no hard disk, no CD, and so no boot order.
    #[cfg(not(target_arch = "wasm32"))]
    fn media_less_config() -> crate::config::ResolvedConfig {
        let mut config = test_resolved_config();
        config.cdrom = None;
        config.boot_order = Vec::new();
        config
    }

    /// The blank "New VM" has no BIOS path and no media. Typing a BIOS path
    /// applies, and Keep puts the VM in the library with it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_blank_new_vm_takes_a_bios_path_and_is_kept_in_the_library() {
        let scratch = ScratchLibrary::new();
        let (mut app, _command_rx) = native_test_app_over(&scratch, None, None);
        assert_eq!(app.profiles[0].name, "New VM");

        app.settings.bios_path = "roms/bios.bin".to_owned();
        assert_eq!(app.apply_pending_settings(), Ok(()));
        app.keep_selected_in_library();

        assert_eq!(scratch.toml_files(), ["new-vm.toml"]);
        assert!(matches!(app.profiles[0].origin, VmOrigin::Library(_)));
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::info("Saved New VM to the VM library."))
        );
        let kept = scratch.library().load().expect("reload");
        assert_eq!(kept.vms[0].name, "New VM");
        assert!(kept.vms[0].config.bios.ends_with("roms/bios.bin"));
        assert!(kept.vms[0].config.boot_order.is_empty());
    }

    /// A library VM with no medium to boot from is renamed like any other:
    /// the rename marks it `Unsaved`, and the flush writes it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_vm_with_no_media_is_renamed_and_its_file_written() {
        let scratch = ScratchLibrary::new();
        scratch
            .library()
            .create("Rusty Box", &media_less_config())
            .expect("seed");
        let (mut app, _command_rx) = native_test_app_over(&scratch, None, None);

        app.profiles[0].name = "Phone".to_owned();
        assert_eq!(app.apply_pending_settings(), Ok(()));

        assert_eq!(app.profiles[0].save_state, SaveState::Unsaved);
        app.flush_unsaved();

        assert_eq!(app.profiles[0].save_state, SaveState::Saved);
        assert_eq!(scratch.toml_files(), ["rusty-box.toml"]);
        assert_eq!(scratch.library().load().expect("reload").vms[0].name, "Phone");
    }

    /// A VM with nothing to boot from is refused at power-on, with the
    /// notice naming what to attach, and no machine is started.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_power_on_with_no_media_is_refused_and_names_what_is_missing() {
        let scratch = ScratchLibrary::new();
        scratch
            .library()
            .create("Rusty Box", &media_less_config())
            .expect("seed");
        let (mut app, command_rx) = native_test_app_over(&scratch, None, None);

        app.start_vm();

        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Attach a hard disk or CD/DVD before powering on."
            ))
        );
        assert!(command_rx.try_recv().is_err());
        assert!(!app.shared.lock().unwrap().start_pending);
    }

    /// The blank "New VM" lacks both, and its refusal names both, pointing
    /// at the BIOS setting where `RunError::MissingBios` does.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_power_on_of_the_blank_new_vm_names_the_bios_and_the_media() {
        let scratch = ScratchLibrary::new();
        let (mut app, command_rx) = native_test_app_over(&scratch, None, None);

        app.start_vm();

        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Set a BIOS path under Hardware › Display and attach a hard disk or CD/DVD \
                 before powering on."
            ))
        );
        assert!(command_rx.try_recv().is_err());

        app.settings.cdrom_enabled = true;
        app.settings.cdrom_path = "boot.iso".to_owned();
        app.start_vm();

        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Set a BIOS path under Hardware › Display before powering on."
            ))
        );
        assert!(command_rx.try_recv().is_err());
        assert!(crate::error::RunError::MissingBios
            .to_string()
            .contains("under Hardware › Display"));
    }

    /// A library VM with no media is selected like any other, and selecting
    /// away from it works.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_vm_with_no_media_can_be_selected_and_left() {
        let scratch = ScratchLibrary::new();
        let library = scratch.library();
        library.create("Alpine", &test_resolved_config()).expect("seed");
        library.create("Bare", &media_less_config()).expect("seed");
        let (mut app, _command_rx) = native_test_app_over(&scratch, None, None);
        let bare = app
            .profiles
            .iter()
            .position(|profile| profile.name == "Bare")
            .expect("the media-less VM is listed");
        let alpine = app
            .profiles
            .iter()
            .position(|profile| profile.name == "Alpine")
            .expect("the seeded VM is listed");

        app.select_profile(bare);
        assert_eq!(app.chrome.selected_vm(), bare);
        assert_eq!(app.vm_info.name, "Bare");

        app.select_profile(alpine);

        assert_eq!(app.chrome.selected_vm(), alpine);
        assert_eq!(app.vm_info.name, "Alpine");
        assert_eq!(app.shell_notice, None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn keeping_a_vm_shows_the_remember_warning_over_the_saved_notice() {
        let (mut app, _command_rx, scratch) = native_test_app();
        fs::create_dir(scratch.dir.join(".last")).expect("a folder where the record goes");

        app.keep_selected_in_library();

        assert_eq!(scratch.toml_files(), ["rusty-box.toml"]);
        assert!(
            matches!(&app.shell_notice, Some(notice) if notice.kind == ShellNoticeKind::Warning),
            "notice: {:?}",
            app.shell_notice
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_delete_confirmed_after_the_selection_moved_deletes_nothing() {
        let (mut app, _command_rx, scratch) = library_app();
        app.add_vm_copying_selected();
        app.request_delete_selected();

        app.select_profile(0);
        app.confirm_pending();

        assert_eq!(app.pending_confirm, None);
        assert_eq!(app.profiles.len(), 2);
        assert_eq!(scratch.toml_files().len(), 2);
        assert!(
            matches!(&app.shell_notice, Some(notice) if notice.kind == ShellNoticeKind::Warning),
            "notice: {:?}",
            app.shell_notice
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn delete_broken_in_the_tree_asks_about_that_file() {
        let scratch = ScratchLibrary::new();
        fs::write(scratch.dir.join("bad-a.toml"), "memory_mib = [").expect("write");
        fs::write(scratch.dir.join("bad-b.toml"), "memory_mib = [").expect("write");
        let (mut app, _command_rx) = native_test_app_over(&scratch, None, None);
        assert_eq!(app.broken_files.len(), 2);

        app.handle_sidebar_action(SidebarAction::DeleteBroken(0));

        assert_eq!(
            app.pending_confirm,
            Some(PendingConfirm::DeleteBroken(scratch.dir.join("bad-a.toml")))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn confirm_wording_names_the_vm_or_file() {
        let (mut app, _command_rx, scratch) = library_app();
        app.request_delete_selected();
        let library = app.confirm_wording(app.pending_confirm.as_ref().expect("pending"));
        assert_eq!(library.title, "Delete Alpine?");
        assert!(library
            .body
            .contains(&scratch.dir.join("alpine.toml").display().to_string()));
        assert_eq!(library.verb, "Delete");

        let (mut launch_app, _launch_rx, _launch_scratch) = native_test_app();
        launch_app.request_delete_selected();
        let launch =
            launch_app.confirm_wording(launch_app.pending_confirm.as_ref().expect("pending"));
        assert_eq!(launch.title, "Discard Rusty Box?");
        assert_eq!(launch.verb, "Discard");

        let broken = app.confirm_wording(&PendingConfirm::DeleteBroken(scratch.dir.join("bad.toml")));
        assert_eq!(broken.title, "Delete bad.toml?");
        assert_eq!(broken.verb, "Delete");

        let disk = scratch.dir.join("disk.img");
        let overwrite = app.confirm_wording(&PendingConfirm::OverwriteDisk {
            index: 0,
            origin: VmOrigin::Launch,
            path: disk.clone(),
        });
        assert_eq!(overwrite.title, format!("Overwrite {}?", disk.display()));
        assert_eq!(
            overwrite.body,
            "This VM's startup disk is set to be recreated, which erases the existing file."
        );
        assert_eq!(overwrite.verb, "Overwrite and power on");
    }

    /// The command line's VM with a startup disk at `disk` that is recreated,
    /// erasing the file, at power-on.
    #[cfg(not(target_arch = "wasm32"))]
    fn overwriting_launch(disk: &std::path::Path) -> crate::runner::LaunchVm {
        let mut config = test_resolved_config();
        config.disk = Some(crate::config::ResolvedDisk {
            path: disk.to_path_buf(),
            geometry: crate::args::DiskGeometry {
                cylinders: 16,
                heads: 16,
                sectors_per_track: 63,
            },
            channel: 0,
            drive: 0,
            creation: Some(crate::config::ResolvedDiskCreation {
                path: disk.to_path_buf(),
                size: rusty_box_bximage::ImageSize::mib(8),
                overwrite: true,
            }),
        });
        config.boot_order = vec![crate::args::BootDevice::Disk];
        crate::runner::LaunchVm {
            name: "Overwriting".to_owned(),
            config,
            source: crate::runner::LaunchSource::Flags,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_power_on_that_would_erase_an_existing_disk_asks_first() {
        let scratch = ScratchLibrary::new();
        let disk = unique_temp_path("rusty-box-gui-overwrite");
        write_test_disk(&disk);
        let (mut app, command_rx) =
            native_test_app_over(&scratch, Some(overwriting_launch(&disk)), None);

        app.start_vm();

        assert_eq!(
            app.pending_confirm,
            Some(PendingConfirm::OverwriteDisk {
                index: 0,
                origin: VmOrigin::Launch,
                path: disk.clone(),
            })
        );
        assert!(command_rx.try_recv().is_err());

        app.confirm_pending();

        assert!(matches!(command_rx.try_recv(), Ok(NativeEmulatorCommand::Start(_))));
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_disk_that_does_not_exist_yet_is_created_without_asking() {
        let scratch = ScratchLibrary::new();
        let disk = unique_temp_path("rusty-box-gui-fresh-disk");
        let (mut app, command_rx) =
            native_test_app_over(&scratch, Some(overwriting_launch(&disk)), None);

        app.start_vm();

        assert_eq!(app.pending_confirm, None);
        assert!(matches!(command_rx.try_recv(), Ok(NativeEmulatorCommand::Start(_))));

        // The runner created the file at that power-on, as here, and erases
        // it at no later one this session, so the next power-on asks nothing
        // either.
        write_test_disk(&disk);
        app.shared.lock().unwrap().start_pending = false;
        app.start_vm();

        assert_eq!(app.pending_confirm, None);
        assert!(matches!(command_rx.try_recv(), Ok(NativeEmulatorCommand::Start(_))));
        remove_test_file(&disk);
    }

    /// A plain creation's power-on settles nothing: when the same file is
    /// then set to be recreated, the power-on that would erase it asks.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_plain_power_on_does_not_settle_a_later_overwrite_of_the_same_file() {
        let scratch = ScratchLibrary::new();
        let disk = unique_temp_path("rusty-box-gui-plain-then-overwrite");
        let mut launch = overwriting_launch(&disk);
        if let Some(creation) = launch
            .config
            .disk
            .as_mut()
            .and_then(|disk| disk.creation.as_mut())
        {
            creation.overwrite = false;
        }
        let (mut app, command_rx) = native_test_app_over(&scratch, Some(launch), None);

        app.start_vm();

        assert_eq!(app.pending_confirm, None);
        assert!(matches!(command_rx.try_recv(), Ok(NativeEmulatorCommand::Start(_))));
        assert!(!app.overwrite_confirmed.contains(&disk));

        write_test_disk(&disk);
        app.shared.lock().unwrap().start_pending = false;
        if let Some(creation) = app.settings.disk_creation.as_mut() {
            creation.overwrite = true;
        }
        app.start_vm();

        assert_eq!(
            app.pending_confirm,
            Some(PendingConfirm::OverwriteDisk {
                index: 0,
                origin: VmOrigin::Launch,
                path: disk.clone(),
            })
        );
        assert!(command_rx.try_recv().is_err());
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_cancelled_overwrite_is_asked_again_and_an_agreed_one_is_not() {
        let scratch = ScratchLibrary::new();
        let disk = unique_temp_path("rusty-box-gui-overwrite-again");
        write_test_disk(&disk);
        let (mut app, command_rx) =
            native_test_app_over(&scratch, Some(overwriting_launch(&disk)), None);

        app.start_vm();
        app.cancel_pending();

        assert_eq!(app.pending_confirm, None);
        assert!(command_rx.try_recv().is_err());

        app.start_vm();

        assert_eq!(
            app.pending_confirm,
            Some(PendingConfirm::OverwriteDisk {
                index: 0,
                origin: VmOrigin::Launch,
                path: disk.clone(),
            })
        );

        app.confirm_pending();

        assert!(matches!(command_rx.try_recv(), Ok(NativeEmulatorCommand::Start(_))));

        // Powered off again. The runner recreated the disk at the first
        // power-on and does not again this session, so nothing is asked.
        app.shared.lock().unwrap().start_pending = false;
        app.start_vm();

        assert_eq!(app.pending_confirm, None);
        assert!(matches!(command_rx.try_recv(), Ok(NativeEmulatorCommand::Start(_))));
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_overwrite_confirmed_for_another_vm_starts_nothing() {
        let scratch = ScratchLibrary::new();
        scratch.library().create("Alpine", &test_resolved_config()).expect("seed");
        let disk = unique_temp_path("rusty-box-gui-overwrite-other");
        write_test_disk(&disk);
        let (mut app, command_rx) =
            native_test_app_over(&scratch, Some(overwriting_launch(&disk)), None);
        app.start_vm();
        assert!(matches!(
            app.pending_confirm,
            Some(PendingConfirm::OverwriteDisk { index: 0, .. })
        ));

        app.select_profile(1);
        app.confirm_pending();

        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Another VM was selected while the power-on waited; nothing was started."
            ))
        );
        assert!(command_rx.try_recv().is_err());
        assert!(!app.overwrite_confirmed.contains(&disk));

        // Back on the overwriting VM, the question is asked again: nothing
        // was recorded for it.
        app.select_profile(0);
        app.start_vm();

        assert_eq!(
            app.pending_confirm,
            Some(PendingConfirm::OverwriteDisk {
                index: 0,
                origin: VmOrigin::Launch,
                path: disk.clone(),
            })
        );
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn adding_a_vm_is_refused_while_the_vm_runs() {
        let (mut app, _command_rx, scratch) = library_app();
        app.shared.lock().unwrap().emu_running = true;

        app.add_vm_copying_selected();

        assert_eq!(app.profiles.len(), 1);
        assert_eq!(scratch.toml_files(), ["alpine.toml"]);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning("Stop the running VM before adding a VM."))
        );
    }

    #[test]
    fn shell_starts_on_home_page() {
        let chrome = ShellChrome::default();
        assert_eq!(chrome.page(), ShellPage::Home);
        assert_eq!(chrome.selected_vm(), 0);
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
    fn web_cpu_profiles_are_user_selectable() {
        assert_eq!(WEB_DEFAULT_CPU_COUNT, 1);
        assert_eq!(WEB_MAX_CPU_COUNT, BX_MAX_SMP_THREADS_SUPPORTED);
        assert!(web_cpu_count_is_supported(1));
        assert!(web_cpu_count_is_supported(8));
        assert!(web_cpu_count_is_supported(BX_MAX_SMP_THREADS_SUPPORTED));
        assert!(!web_cpu_count_is_supported(0));
        assert!(!web_cpu_count_is_supported(
            BX_MAX_SMP_THREADS_SUPPORTED + 1
        ));
        assert_eq!(web_uploaded_media_config(128, 8).cpu_params.cpu_count(), 8);
        assert!(web_can_edit_cpu_count(false));
        assert!(!web_can_edit_cpu_count(true));
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
        // The notice is painted on its own frame, so the browser is never
        // asked to render it and run the blocking build in the same one.
        assert_eq!(
            web_next_startup_stage(WebStartupStage::Announce),
            Some(WebStartupStage::BuildMachine)
        );
        assert_eq!(web_next_startup_stage(WebStartupStage::BuildMachine), None);
        assert_eq!(
            web_startup_stage_label(WebStartupStage::Announce),
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
    fn native_settings_apply_cpu_topology() {
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.cpu_sockets = 2;
        settings.cpu_cores = 2;
        settings.cpu_threads = 2;

        settings.apply_to_config(&mut config).unwrap();

        assert_eq!(config.cpu_params.cpu_count(), 8);
        let topology = config.cpu_params.cpu_topology();
        assert_eq!(topology.n_processors(), 2);
        assert_eq!(topology.n_cores(), 2);
        assert_eq!(topology.n_threads(), 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_settings_apply_vga_mode() {
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.vga_mode = Some(crate::config::VgaMode {
            width: 1280,
            height: 1024,
            bpp: 32,
        });

        settings.apply_to_config(&mut config).unwrap();

        assert_eq!(
            config.vga_mode,
            Some(crate::config::VgaMode {
                width: 1280,
                height: 1024,
                bpp: 32,
            })
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_settings_reject_out_of_range_topology() {
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.cpu_cores = 99;

        let error = settings.apply_to_config(&mut config).unwrap_err();
        assert!(error.contains("CPU topology"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_settings_apply_chs_override() {
        let disk = unique_temp_path("rusty-box-gui-chs-override");
        write_test_disk(&disk);
        let mut config = test_resolved_config();
        config.disk = None;
        let mut settings = NativeVmSettings::from_config(&config);
        settings.disk_enabled = true;
        settings.disk_path = disk.display().to_string();
        settings.disk_chs_override = Some(crate::args::DiskGeometry {
            cylinders: 512,
            heads: 8,
            sectors_per_track: 32,
        });

        settings.apply_to_config(&mut config).unwrap();

        let attached = config.disk.as_ref().expect("disk should attach");
        assert_eq!(attached.geometry.cylinders, 512);
        assert_eq!(attached.geometry.heads, 8);
        assert_eq!(attached.geometry.sectors_per_track, 32);
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_settings_reject_zero_chs_override() {
        let disk = unique_temp_path("rusty-box-gui-chs-zero");
        write_test_disk(&disk);
        let mut config = test_resolved_config();
        config.disk = None;
        let mut settings = NativeVmSettings::from_config(&config);
        settings.disk_enabled = true;
        settings.disk_path = disk.display().to_string();
        settings.disk_chs_override = Some(crate::args::DiskGeometry {
            cylinders: 0,
            heads: 8,
            sectors_per_track: 32,
        });

        let error = settings.apply_to_config(&mut config).unwrap_err();
        assert!(error.contains("non-zero"));
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_settings_apply_boot_order() {
        let disk = unique_temp_path("rusty-box-gui-boot-order");
        write_test_disk(&disk);
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.disk_enabled = true;
        settings.disk_path = disk.display().to_string();
        settings.cdrom_enabled = true;
        settings.boot_order = vec![
            crate::args::BootDevice::Disk,
            crate::args::BootDevice::Cdrom,
        ];

        settings.apply_to_config(&mut config).unwrap();
        assert_eq!(
            config.boot_order,
            vec![
                crate::args::BootDevice::Disk,
                crate::args::BootDevice::Cdrom
            ]
        );
        remove_test_file(&disk);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_settings_boot_order_drops_unattached_and_appends_attached() {
        let disk = unique_temp_path("rusty-box-gui-boot-filter");
        write_test_disk(&disk);
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        // Ask to boot the CD first, but only attach the hard disk.
        settings.disk_enabled = true;
        settings.disk_path = disk.display().to_string();
        settings.cdrom_enabled = false;
        settings.boot_order = vec![crate::args::BootDevice::Cdrom];

        settings.apply_to_config(&mut config).unwrap();
        // Unattached CD dropped; attached disk appended so the VM still boots.
        assert_eq!(config.boot_order, vec![crate::args::BootDevice::Disk]);
        remove_test_file(&disk);
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
        settings.boot_order = vec![crate::args::BootDevice::Disk];

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
        settings.boot_order = vec![crate::args::BootDevice::Disk];
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
    fn native_vm_settings_apply_a_blank_bios_as_none_and_clamp_numbers() {
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.bios_path = "   ".to_owned();
        assert_eq!(settings.apply_to_config(&mut config), Ok(()));
        assert_eq!(config.bios, std::path::PathBuf::new());
        assert_eq!(PowerOnGap::of(&config), Some(PowerOnGap::Bios));

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

    /// With nothing attached the boot order applies empty, as the resolver
    /// leaves it for the egui shell, and the power-on is what refuses.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_vm_settings_apply_no_media_as_an_empty_boot_order() {
        let mut config = test_resolved_config();
        let mut settings = NativeVmSettings::from_config(&config);
        settings.cdrom_enabled = false;
        settings.disk_enabled = false;

        assert_eq!(settings.apply_to_config(&mut config), Ok(()));

        assert!(config.boot_order.is_empty());
        assert_eq!(config.cdrom, None);
        assert_eq!(PowerOnGap::of(&config), Some(PowerOnGap::Media));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_start_vm_sends_selected_config_when_stopped() {
        let (mut app, command_rx, _library) = native_test_app();
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
        let (mut app, command_rx, _library) = native_test_app();

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
        let (mut app, command_rx, _library) = native_test_app();
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
        let (mut app, _command_rx, _library) = native_test_app();
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
        let (mut app, _command_rx, _library) = native_test_app();

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
        let mut profile = NativeVmProfile::from_config("Base", config, VmOrigin::Launch);
        profile.settings.memory_mib = 384;

        let mut copy = profile.duplicate("Second VM", VmOrigin::Launch);
        copy.settings.memory_mib = 768;

        assert_eq!(profile.name, "Base");
        assert_eq!(profile.settings.memory_mib, 384);
        assert_eq!(copy.name, "Second VM");
        assert_eq!(copy.settings.memory_mib, 768);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_profile_duplicate_select_delete_rename_refreshes_metadata() {
        let (mut app, _command_rx, _library) = native_test_app();
        app.profiles[0].name = "Base VM".to_owned();
        app.settings.memory_mib = 512;
        app.apply_pending_settings().unwrap();

        app.add_vm_copying_selected();

        assert_eq!(app.profiles.len(), 2);
        assert_eq!(app.chrome.selected_vm(), 1);
        assert_eq!(app.chrome.vm_library[1].memory, "512 MB");
        app.profiles[1].name = "Copy VM".to_owned();
        app.apply_pending_settings().unwrap();
        assert_eq!(app.vm_info.name, "Copy VM");
        assert_eq!(app.chrome.vm_library[1].name, "Copy VM");

        app.request_delete_selected();
        app.confirm_pending();

        assert_eq!(app.profiles.len(), 1);
        assert_eq!(app.chrome.selected_vm(), 0);
        assert_eq!(app.vm_info.name, "Base VM");
        assert_eq!(app.chrome.vm_library[0].name, "Base VM");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_profile_delete_requires_stopped_multiple_profiles() {
        let (mut app, _command_rx, _library) = native_test_app();

        app.request_delete_selected();
        app.confirm_pending();

        assert_eq!(app.profiles.len(), 1);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning("At least one VM is required."))
        );

        app.add_vm_copying_selected();
        app.shared.lock().unwrap().emu_running = true;
        app.request_delete_selected();
        app.confirm_pending();

        assert_eq!(app.profiles.len(), 2);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Stop the running VM before deleting it."
            ))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_profile_selection_refuses_while_running() {
        let (mut app, _command_rx, _library) = native_test_app();
        app.add_vm_copying_selected();
        assert_eq!(app.chrome.selected_vm(), 1);
        app.shared.lock().unwrap().emu_running = true;

        app.select_profile(0);

        assert_eq!(app.chrome.selected_vm(), 1);
        assert_eq!(
            app.shell_notice,
            Some(ShellNotice::warning(
                "Stop the running VM before selecting another VM."
            ))
        );
    }

    /// A library VM whose file the shell reads into its own form — the boot
    /// order with the attached disk appended — is not rewritten for that by
    /// a selection: selecting it, away and back leaves its file as written.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selecting_away_and_back_does_not_rewrite_a_vm_the_shell_normalises() {
        let scratch = ScratchLibrary::new();
        scratch
            .library()
            .create("Alpine", &test_resolved_config())
            .expect("seed");
        let file = scratch.dir.join("both.toml");
        let written = "[vm]\nname = \"Both\"\n\n[display]\nbackend = \"egui\"\n\n[rom]\n\
                       bios = \"bios.bin\"\n\n[boot]\norder = [\"cdrom\"]\n\n[disk]\n\
                       path = \"disk.img\"\n\
                       chs = { cylinders = 16, heads = 16, sectors_per_track = 63 }\n\n\
                       [cdrom]\npath = \"boot.iso\"\n";
        fs::write(&file, written).expect("write the VM file by hand");
        let (mut app, _command_rx) = native_test_app_over(&scratch, None, None);
        let both = app
            .profiles
            .iter()
            .position(|profile| profile.name == "Both")
            .expect("the hand-written VM is listed");
        let alpine = app
            .profiles
            .iter()
            .position(|profile| profile.name == "Alpine")
            .expect("the seeded VM is listed");

        app.select_profile(both);

        assert_eq!(
            app.profiles[both].config.boot_order,
            vec![crate::args::BootDevice::Cdrom, crate::args::BootDevice::Disk]
        );
        assert_eq!(app.profiles[both].save_state, SaveState::Saved);

        app.select_profile(alpine);
        app.select_profile(both);

        assert_eq!(fs::read_to_string(&file).expect("read"), written);
        assert_eq!(app.profiles[both].save_state, SaveState::Saved);
        assert_eq!(app.shell_notice, None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn created_image_hard_disk_attaches_to_stopped_profile() {
        let (mut app, _command_rx, _library) = native_test_app();
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
        let (mut app, _command_rx, _library) = native_test_app();
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
        let (mut app, _command_rx, _library) = native_test_app();

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
    fn hard_disk_panel_rejects_beyond_bochs_cylinder_limit() {
        // 32 GiB is now accepted; only sizes whose physical geometry exceeds
        // BOCHS_MAX_CYLINDERS (2^24, ~8063 GiB with 16h/63s/512b) are rejected — and
        // rejected before any file is written, so this stays cheap.
        let path = unique_temp_path("rusty-box-gui-panel-huge-disk");
        let mut panel = DiskCreatorPanel::default();
        panel.path = path.display().to_string();
        panel.hard_disk_size = "9000G".to_owned();

        panel.create_image();

        assert!(matches!(
            &panel.status,
            Some(CreatorStatus::Error(message)) if message.contains("exceeds Bochs limit")
        ));
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
