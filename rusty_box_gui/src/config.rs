use crate::args::{Args, BootDevice, DiskGeometry, DisplayBackend, LogLevel};
use crate::error::RunError;
#[cfg(not(target_arch = "wasm32"))]
use rusty_box::cpu::decoder::features::X86Feature;
use rusty_box::iodev::PortE9Hack;
use rusty_box::params::{BxParamError, BxParams};
use rusty_box::CpuidFreq;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const DEFAULT_CONFIG_FILE: &str = "rusty_box.toml";

// The value a setting takes when neither the command line nor the file sets
// it. `resolve_config_with_base` and `blank_config` both read these, so a
// "New VM" is what a file with no keys resolves to.
const DEFAULT_MEMORY_MIB: u32 = 32;
const DEFAULT_MEMORY_BLOCK_KIB: u32 = 128;
const DEFAULT_IPS: u32 = 4_000_000;
const DEFAULT_PCI: bool = true;
const DEFAULT_SYNC_SLOWDOWN: bool = false;
const DEFAULT_SYNC_REALTIME: bool = false;
const DEFAULT_SMP_QUANTUM: u32 = 16;
const DEFAULT_CPUID_FREQ: CpuidFreq = CpuidFreq::None;
const DEFAULT_MAX_INSTRUCTIONS: u64 = u64::MAX;
const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Warn;
const DEFAULT_PCI_VGA: bool = false;

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    #[serde(skip_serializing_if = "VmToml::is_empty")]
    pub vm: VmToml,
    pub emulator: EmulatorToml,
    pub display: DisplayToml,
    pub rom: RomToml,
    pub boot: BootToml,
    pub disk: Option<DiskToml>,
    pub cdrom: Option<CdromToml>,
    /// The Arm `virt` board's hardware, for a VM whose `[vm] machine` is
    /// `"arm-virt"`.
    pub virt: Option<VirtToml>,
    pub logging: LoggingToml,
}

/// The `[vm]` table: what a VM library file says about the VM itself rather
/// than its hardware.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct VmToml {
    /// The name the shell shows. A library file without one is shown under
    /// its file name.
    pub name: Option<String>,
    /// Which machine the VM is: `"pc"` (the default) or `"arm-virt"`.
    pub machine: Option<MachineKind>,
}

impl VmToml {
    /// Whether the table says nothing, in which case the file carries no
    /// `[vm]` header at all: a bare header would be an unknown table to a
    /// reader without this key.
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.machine.is_none()
    }
}

/// Which machine a VM is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MachineKind {
    /// The Bochs PC: x86 processors, a BIOS, IDE media.
    #[default]
    Pc,
    /// QEMU's Arm `virt` board with an AArch64 processor
    /// (`rusty_box_aarch64`).
    ArmVirt,
}

impl MachineKind {
    /// Every machine, in the order the shell offers them.
    pub const ALL: [Self; 2] = [Self::Pc, Self::ArmVirt];

    /// The machine's name as the shell and refusals show it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pc => "a PC",
            Self::ArmVirt => "an Arm virt board",
        }
    }
}

impl std::fmt::Display for MachineKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label())
    }
}

/// The `[virt]` table: the Arm `virt` board's hardware.
///
/// `cpu` names its processor (`"cortex-a57"`, the default, or `"armv9.2"`).
/// It boots UEFI firmware from flash (`firmware`, with `firmware_vars` for its
/// variable store) or a Linux kernel directly (`kernel`, with `initrd`,
/// `command_line` and `device_tree`), never both. `display` plugs in a
/// virtio-gpu asking the guest for that size; `keyboard` and `tablet` plug
/// in the virtio keyboard and tablet. `disk` plugs in a virtio-blk over that
/// raw image, which the guest may only read with `disk_read_only`.
// NOTE: field order matters for TOML serialization — `display`, a sub-table,
// must follow the scalar fields.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct VirtToml {
    pub memory_mib: Option<u32>,
    pub ips: Option<u32>,
    pub cpu: Option<VirtCpuToml>,
    pub firmware: Option<PathBuf>,
    pub firmware_vars: Option<PathBuf>,
    pub kernel: Option<PathBuf>,
    pub initrd: Option<PathBuf>,
    pub command_line: Option<String>,
    pub device_tree: Option<PathBuf>,
    pub keyboard: Option<bool>,
    pub tablet: Option<bool>,
    pub disk: Option<PathBuf>,
    pub disk_read_only: Option<bool>,
    pub display: Option<VirtDisplayToml>,
}

/// `virt.cpu`: a processor model, by the name
/// [`rusty_box_aarch64::CpuModel::name`] gives it. A name no model has is
/// refused when the file is read, with the names there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct VirtCpuToml(pub rusty_box_aarch64::CpuModel);

impl TryFrom<String> for VirtCpuToml {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        name.parse().map(Self).map_err(|e| format!("\"{name}\": {e}"))
    }
}

impl From<VirtCpuToml> for String {
    fn from(cpu: VirtCpuToml) -> Self {
        cpu.0.name().to_owned()
    }
}

/// `[virt.display]`: the size a virtio-gpu asks the guest for.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct VirtDisplayToml {
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmulatorToml {
    /// Which engine retires the guest's instructions: `"interpreter"` or
    /// `"whp"`. `--engine` overrides it; absent everywhere, the interpreter.
    pub engine: Option<Engine>,
    /// Which processor the machine offers its guest: `"preset"` or
    /// `"host-shared"`. `--cpu-capabilities` overrides it; absent everywhere,
    /// the preset.
    pub cpu_capabilities: Option<CpuCapabilities>,
    pub memory_mib: Option<u32>,
    pub host_memory_mib: Option<u32>,
    pub memory_block_kib: Option<u32>,
    pub cpus: Option<u32>,
    pub cpu_sockets: Option<u32>,
    pub cpu_cores: Option<u32>,
    pub cpu_threads: Option<u32>,
    pub ips: Option<u32>,
    pub pci: Option<bool>,
    pub sync_slowdown: Option<bool>,
    /// Run the PIT, the ACPI timer and the VGA's vertical retrace on
    /// wall-clock time — Bochs `clock: sync=realtime`. Default false
    /// (`sync=none`): they follow emulated time, PIT calibration measures
    /// exactly `ips`, boots deterministic.
    pub sync_realtime: Option<bool>,
    pub smp_quantum: Option<u32>,
    pub max_instructions: Option<u64>,
    /// How CPUID frequency leaves 0x15/0x16 are reported — Bochs
    /// `cpu: cpuid_freq=` ("hardware" | "none" | "ips"). Default: "none".
    pub cpuid_freq: Option<String>,
    /// Whether port 0xE9 is a debug console — Bochs `port_e9_hack:`
    /// ("off" | "on" | "all-rings"). Default: "off", as upstream.
    pub port_e9_hack: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayToml {
    pub backend: Option<DisplayBackend>,
    /// Pre-boot VBE mode width in pixels (raises the DISPI capability ceiling).
    pub width: Option<u16>,
    /// Pre-boot VBE mode height in pixels.
    pub height: Option<u16>,
    /// Pre-boot VBE bits-per-pixel (8/16/24/32).
    pub bpp: Option<u16>,
    /// Register the VGA as a PCI device so Linux `bochs-drm` can bind for a KMS
    /// framebuffer. Off by default; experimental (needs a guest boot to verify).
    pub pci_vga: Option<bool>,
    /// Stretch the console to fill the screen rather than keep the guest's
    /// shape. The phone's full-screen console reads it. Default false.
    pub stretch: Option<bool>,
    /// The phone trackpad's cursor speed, percent of the finger's own travel
    /// across the guest's image: 50 to 300. Default 100.
    pub pointer_speed_percent: Option<u16>,
}

/// The phone trackpad's cursor speed, in percent of the finger's own travel
/// across the guest's image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerSpeed {
    percent: u16,
}

impl PointerSpeed {
    /// The cursor keeps pace with the finger.
    pub const DEFAULT: Self = Self { percent: 100 };
    /// The slowest and fastest the trackpad offers.
    pub const RANGE_PERCENT: core::ops::RangeInclusive<u16> = 50..=300;

    /// `percent` held to [`Self::RANGE_PERCENT`].
    pub fn from_percent(percent: u16) -> Self {
        Self {
            percent: percent.clamp(*Self::RANGE_PERCENT.start(), *Self::RANGE_PERCENT.end()),
        }
    }

    pub fn percent(self) -> u16 {
        self.percent
    }

    /// The multiplier over the finger's travel.
    pub fn factor(self) -> f32 {
        f32::from(self.percent) / 100.0
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RomToml {
    pub bios: Option<PathBuf>,
    pub vga_bios: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BootToml {
    pub order: Vec<BootDevice>,
}

// NOTE: field order matters for TOML serialization — the `toml` serializer
// requires scalar values to be emitted before any sub-tables. `chs` (an inline
// table) and `create` (a sub-table) must therefore follow the scalar fields.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiskToml {
    pub path: Option<PathBuf>,
    pub channel: Option<usize>,
    pub drive: Option<usize>,
    pub chs: Option<DiskGeometry>,
    pub create: Option<DiskCreateToml>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiskCreateToml {
    pub path: Option<PathBuf>,
    pub size: Option<String>,
    pub overwrite: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CdromToml {
    pub path: Option<PathBuf>,
    pub channel: Option<usize>,
    pub drive: Option<usize>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingToml {
    pub level: Option<LogLevel>,
}

/// Which engine retires the guest's instructions.
///
/// Named rather than a bool because a third is already foreseen — a KVM leaf
/// on Linux — and because a reader of `--engine whp` should not have to know
/// which way round a flag points.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Deserialize,
    serde::Serialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub enum Engine {
    /// This port's own interpreter. Everywhere, and the default.
    #[default]
    Interpreter,
    /// The Windows Hypervisor Platform. Needs a Windows build with `hv-whp`
    /// and without `guest-trace`, and a host that has the platform enabled;
    /// refused rather than downgraded when any of these is missing.
    Whp,
}

/// Which processor the machine offers its guest.
///
/// A machine setting, not an engine one. A guest keeps what it enabled across
/// a switch from the hypervisor to the interpreter, so both must offer the
/// same processor or the switch changes the hardware underneath it.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub enum CpuCapabilities {
    /// This port's own processor model, whole. The same guest sees the same
    /// processor on every host, which is what makes a run reproducible — and
    /// what a guest under study must have.
    #[default]
    Preset,
    /// The model, narrowed to what this host can also carry.
    ///
    /// Needed by a machine that may run on the hypervisor: `XSETBV` is not a
    /// trapped instruction there, so a guest that believes the preset enables
    /// a register file the silicon lacks, and the platform then refuses the
    /// whole processor state.
    HostShared,
}

/// A VM's settings, resolved: the machine and its hardware, and how the
/// front end shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    /// The machine and its hardware.
    pub machine: MachineConfig,
    pub display: DisplayBackend,
    pub log_level: LogLevel,
    /// The phone's full-screen console fills the screen instead of keeping the
    /// guest's shape.
    pub console_stretch: bool,
    /// The phone trackpad's cursor speed.
    pub pointer_speed: PointerSpeed,
}

/// A VM's machine, with the hardware settings that machine has.
///
/// Closed (R5): every machine the runner can build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineConfig {
    Pc(PcConfig),
    ArmVirt(ArmVirtConfig),
}

impl MachineConfig {
    pub fn kind(&self) -> MachineKind {
        match self {
            Self::Pc(_) => MachineKind::Pc,
            Self::ArmVirt(_) => MachineKind::ArmVirt,
        }
    }
}

impl ResolvedConfig {
    /// The PC's settings, when the VM is a PC.
    pub fn pc(&self) -> Option<&PcConfig> {
        match &self.machine {
            MachineConfig::Pc(pc) => Some(pc),
            MachineConfig::ArmVirt(_) => None,
        }
    }

    pub fn pc_mut(&mut self) -> Option<&mut PcConfig> {
        match &mut self.machine {
            MachineConfig::Pc(pc) => Some(pc),
            MachineConfig::ArmVirt(_) => None,
        }
    }

    /// The Arm `virt` board's settings, when the VM is one.
    pub fn arm_virt(&self) -> Option<&ArmVirtConfig> {
        match &self.machine {
            MachineConfig::ArmVirt(virt) => Some(virt),
            MachineConfig::Pc(_) => None,
        }
    }

    pub fn arm_virt_mut(&mut self) -> Option<&mut ArmVirtConfig> {
        match &mut self.machine {
            MachineConfig::ArmVirt(virt) => Some(virt),
            MachineConfig::Pc(_) => None,
        }
    }

    /// The engine that retires the guest's instructions: a PC's chosen one;
    /// the Arm board has only the interpreter.
    pub fn engine(&self) -> Engine {
        match &self.machine {
            MachineConfig::Pc(pc) => pc.engine,
            MachineConfig::ArmVirt(_) => Engine::Interpreter,
        }
    }

    /// The guest's memory, MiB, whatever the machine.
    pub fn memory_mib(&self) -> u32 {
        match &self.machine {
            MachineConfig::Pc(pc) => pc.memory_mib,
            MachineConfig::ArmVirt(virt) => virt.memory_mib,
        }
    }

    /// The hard-disk image a run opens, whatever the machine: a PC's IDE
    /// disk, which its guest writes, or an Arm board's virtio-blk disk.
    pub fn hard_disk(&self) -> Option<HardDiskUse<'_>> {
        match &self.machine {
            MachineConfig::Pc(pc) => pc.disk.as_ref().map(|disk| HardDiskUse {
                path: &disk.path,
                access: DiskAccess::ReadWrite,
            }),
            MachineConfig::ArmVirt(virt) => virt.disk.as_ref().map(|disk| HardDiskUse {
                path: &disk.path,
                access: disk.access,
            }),
        }
    }
}

/// A hard-disk image a run opens, and whether its guest may write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardDiskUse<'a> {
    pub path: &'a Path,
    pub access: DiskAccess,
}

/// The Bochs PC's hardware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcConfig {
    /// Which engine retires the guest's instructions.
    pub engine: Engine,
    /// Which processor the machine offers its guest.
    pub cpu_capabilities: CpuCapabilities,
    pub memory_mib: u32,
    pub host_memory_mib: u32,
    pub memory_block_kib: u32,
    pub ips: u32,
    pub pci: bool,
    pub sync_slowdown: bool,
    pub sync_realtime: bool,
    pub smp_quantum: u32,
    pub cpuid_freq: CpuidFreq,
    pub port_e9_hack: PortE9Hack,
    pub max_instructions: u64,
    pub cpu_params: BxParams,
    pub bios: PathBuf,
    pub vga_bios: Option<PathBuf>,
    pub boot_order: Vec<BootDevice>,
    pub disk: Option<ResolvedDisk>,
    pub cdrom: Option<ResolvedCdrom>,
    /// Optional pre-boot VBE mode (width, height, bpp) applied to the VGA
    /// controller before reset. `None` leaves the built-in defaults.
    pub vga_mode: Option<VgaMode>,
    /// Register the VGA on PCI (experimental KMS / `bochs-drm` path). Default off.
    pub pci_vga: bool,
}

/// The Arm `virt` board's hardware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmVirtConfig {
    pub memory_mib: u32,
    /// The guest's instruction rate, which its timers count against.
    pub ips: u32,
    /// The processor.
    pub cpu: rusty_box_aarch64::CpuModel,
    pub boot: ArmBoot,
    /// The virtio-gpu, by the size it asks the guest for; `None` for none.
    pub display: Option<VirtioDisplaySize>,
    /// A virtio keyboard is plugged in.
    pub keyboard: bool,
    /// A virtio tablet is plugged in.
    pub tablet: bool,
    /// The virtio-blk's disk; `None` for none.
    pub disk: Option<VirtDisk>,
}

/// The disk behind the Arm `virt` board's virtio-blk, as QEMU's `-drive
/// file=…,format=raw,if=none` with a `virtio-blk-device` has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtDisk {
    /// A raw image: its bytes are the disk's, sector for sector.
    pub path: PathBuf,
    /// Whether the guest may write it (QEMU's `readonly=on`).
    pub access: DiskAccess,
}

/// Whether a VM's guest may write its hard disk.
///
/// Closed (R5): a PC's IDE disk is always written; an Arm board's virtio-blk
/// disk may be either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiskAccess {
    ReadWrite,
    /// QEMU's `readonly=on`: the device refuses the guest's writes, and the
    /// image is opened for reading alone.
    ReadOnly,
}

impl DiskAccess {
    /// The access `virt.disk_read_only`, and the Hard disk page's
    /// `Read-only`, name.
    pub fn of_read_only(read_only: bool) -> Self {
        if read_only {
            Self::ReadOnly
        } else {
            Self::ReadWrite
        }
    }

    /// Whether the guest only reads the disk: `virt.disk_read_only`.
    pub fn is_read_only(self) -> bool {
        match self {
            Self::ReadWrite => false,
            Self::ReadOnly => true,
        }
    }
}

/// What the Arm `virt` board boots.
///
/// Closed (R5): the ways `rusty_box_aarch64::MachineBuilder` starts a guest,
/// and nothing chosen yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmBoot {
    /// No firmware and no kernel chosen yet: a VM the shell made and the user
    /// has not finished.
    Unset,
    /// UEFI firmware from flash, with its variable store when one is given.
    Firmware { code: PathBuf, vars: Option<PathBuf> },
    /// A Linux `Image` entered directly, as QEMU's `-kernel` does.
    Linux {
        kernel: PathBuf,
        initrd: Option<PathBuf>,
        command_line: String,
        device_tree: Option<PathBuf>,
    },
}

/// The size a virtio-gpu asks its guest for, in pixels; neither is zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioDisplaySize {
    pub width: u32,
    pub height: u32,
}

/// A pre-boot VBE display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VgaMode {
    pub width: u16,
    pub height: u16,
    pub bpp: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDiskCreation {
    pub path: PathBuf,
    pub size: rusty_box_bximage::ImageSize,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDisk {
    pub path: PathBuf,
    pub geometry: DiskGeometry,
    pub channel: usize,
    pub drive: usize,
    pub creation: Option<ResolvedDiskCreation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCdrom {
    pub path: PathBuf,
    pub channel: usize,
    pub drive: usize,
}

/// The launch configuration: the file `--config` names, if any, with the
/// command line's flags over it. Nothing the command line does not name is
/// read. A `rusty_box.toml` in the working directory or its parent would let
/// anyone who can drop a file there decide what the launcher boots and which
/// file a disk creation overwrites (CWE-427), so neither is consulted.
pub fn load_config(args: &Args) -> Result<ResolvedConfig, RunError> {
    match &args.config {
        Some(path) => {
            let file = load_toml_file(path)?;
            resolve_config_with_base(file, args, path.parent())
        }
        None => resolve_config_with_base(FileConfig::default(), args, None),
    }
}

pub fn resolve_config(file: FileConfig, args: &Args) -> Result<ResolvedConfig, RunError> {
    resolve_config_with_base(file, args, None)
}

/// A configuration with nothing chosen yet: a PC with no firmware, no media,
/// and every setting at the value `resolve_config` gives a missing key. The
/// shell shows it as "New VM" when it has no other VM to show.
pub fn blank_config() -> ResolvedConfig {
    ResolvedConfig {
        machine: MachineConfig::Pc(PcConfig::blank()),
        display: default_display_backend(),
        log_level: DEFAULT_LOG_LEVEL,
        console_stretch: false,
        pointer_speed: PointerSpeed::DEFAULT,
    }
}

impl PcConfig {
    /// A PC with nothing chosen yet: what a file with no keys resolves to,
    /// less the BIOS it would be refused for.
    pub fn blank() -> Self {
        Self {
            engine: Engine::default(),
            cpu_capabilities: CpuCapabilities::default(),
            memory_mib: DEFAULT_MEMORY_MIB,
            host_memory_mib: DEFAULT_MEMORY_MIB,
            memory_block_kib: DEFAULT_MEMORY_BLOCK_KIB,
            ips: DEFAULT_IPS,
            pci: DEFAULT_PCI,
            sync_slowdown: DEFAULT_SYNC_SLOWDOWN,
            sync_realtime: DEFAULT_SYNC_REALTIME,
            smp_quantum: DEFAULT_SMP_QUANTUM,
            cpuid_freq: DEFAULT_CPUID_FREQ,
            port_e9_hack: PortE9Hack::Off,
            max_instructions: DEFAULT_MAX_INSTRUCTIONS,
            cpu_params: BxParams::default(),
            bios: PathBuf::new(),
            vga_bios: None,
            boot_order: Vec::new(),
            disk: None,
            cdrom: None,
            vga_mode: None,
            pci_vga: DEFAULT_PCI_VGA,
        }
    }
}

/// The Arm `virt` board's RAM when a VM names none: QEMU's default.
const DEFAULT_VIRT_MEMORY_MIB: u32 = (rusty_box_aarch64::RamSize::DEFAULT.get() >> 20) as u32;
/// The Arm `virt` board's instruction rate when a VM names none.
const DEFAULT_VIRT_IPS: u32 = {
    let ips = rusty_box_aarch64::VirtConfig::DEFAULT.ips.get();
    assert!(ips <= u32::MAX as u64);
    ips as u32
};

impl ArmVirtConfig {
    /// An Arm `virt` board with nothing chosen yet: what an empty `[virt]`
    /// table resolves to, with nothing to boot.
    pub fn blank() -> Self {
        Self {
            memory_mib: DEFAULT_VIRT_MEMORY_MIB,
            ips: DEFAULT_VIRT_IPS,
            cpu: rusty_box_aarch64::CpuModel::default(),
            boot: ArmBoot::Unset,
            display: None,
            keyboard: false,
            tablet: false,
            disk: None,
        }
    }
}

/// Resolves a file the shell keeps itself — a VM library file, or the VM a
/// first run imports from beside the executable — with no command line over
/// it. Relative paths in it resolve against `dir`, the folder the file lives
/// in.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn resolve_config_in(file: FileConfig, dir: &Path) -> Result<ResolvedConfig, RunError> {
    resolve_config_with_base(file, &Args::default(), Some(dir))
}

fn resolve_config_with_base(
    file: FileConfig,
    args: &Args,
    config_dir: Option<&Path>,
) -> Result<ResolvedConfig, RunError> {
    let display = args
        .display
        .or(file.display.backend)
        .unwrap_or_else(default_display_backend);
    let log_level = args
        .log_level
        .or(file.logging.level)
        .unwrap_or(DEFAULT_LOG_LEVEL);
    let console_stretch = file.display.stretch.unwrap_or(false);
    let pointer_speed = file
        .display
        .pointer_speed_percent
        .map_or(PointerSpeed::DEFAULT, PointerSpeed::from_percent);
    let machine = match file.vm.machine.unwrap_or_default() {
        MachineKind::Pc => {
            if file.virt.is_some() {
                return Err(RunError::NotForMachine {
                    setting: "[virt]",
                    machine: MachineKind::Pc,
                });
            }
            MachineConfig::Pc(resolve_pc(&file, args, config_dir, display)?)
        }
        MachineKind::ArmVirt => MachineConfig::ArmVirt(resolve_arm_virt(&file, args, config_dir)?),
    };
    Ok(ResolvedConfig {
        machine,
        display,
        log_level,
        console_stretch,
        pointer_speed,
    })
}

/// The Arm `virt` board `file`'s `[virt]` table and `args` describe. Every
/// PC setting, in the file or on the command line, is refused: it would be
/// dropped without a word otherwise.
fn resolve_arm_virt(file: &FileConfig, args: &Args, config_dir: Option<&Path>) -> Result<ArmVirtConfig, RunError> {
    let pc_tables = [
        ("[emulator]", file.emulator.is_set()),
        ("[rom]", file.rom.bios.is_some() || file.rom.vga_bios.is_some()),
        ("[boot]", !file.boot.order.is_empty()),
        ("[disk]", file.disk.is_some()),
        ("[cdrom]", file.cdrom.is_some()),
        ("display.width", file.display.width.is_some()),
        ("display.height", file.display.height.is_some()),
        ("display.bpp", file.display.bpp.is_some()),
        ("display.pci_vga", file.display.pci_vga.is_some()),
    ];
    let pc_settings = pc_tables
        .into_iter()
        .filter_map(|(name, set)| set.then_some(name))
        .chain(args.pc_flags());
    if let Some(setting) = pc_settings.into_iter().next() {
        return Err(RunError::NotForMachine {
            setting,
            machine: MachineKind::ArmVirt,
        });
    }
    let virt = file.virt.clone().unwrap_or_default();

    let memory_mib = args.memory_mib.or(virt.memory_mib).unwrap_or(DEFAULT_VIRT_MEMORY_MIB);
    if rusty_box_aarch64::RamSize::mib(u64::from(memory_mib)).is_none() {
        return Err(RunError::InvalidVirtMemory { memory_mib });
    }
    let ips = args.ips.or(virt.ips).unwrap_or(DEFAULT_VIRT_IPS);
    ensure_nonzero("ips", ips)?;

    let path = |p: &Option<PathBuf>| p.clone().map(|p| resolve_toml_path(config_dir, p));
    let boot = match (path(&virt.firmware), path(&virt.kernel)) {
        (Some(_), Some(_)) => {
            return Err(RunError::ConflictingVirtBoot);
        }
        (Some(code), None) => {
            for (name, set) in [
                ("virt.initrd", virt.initrd.is_some()),
                ("virt.command_line", virt.command_line.is_some()),
                ("virt.device_tree", virt.device_tree.is_some()),
            ] {
                if set {
                    return Err(RunError::VirtSettingNeedsKernel { setting: name });
                }
            }
            ArmBoot::Firmware {
                code,
                vars: path(&virt.firmware_vars),
            }
        }
        (None, Some(kernel)) => {
            if virt.firmware_vars.is_some() {
                return Err(RunError::VirtSettingNeedsFirmware {
                    setting: "virt.firmware_vars",
                });
            }
            ArmBoot::Linux {
                kernel,
                initrd: path(&virt.initrd),
                command_line: virt
                    .command_line
                    .clone()
                    .unwrap_or_else(|| rusty_box_aarch64::boot::SERIAL_CONSOLE_COMMAND_LINE.to_owned()),
                device_tree: path(&virt.device_tree),
            }
        }
        (None, None) => {
            for (name, set) in [
                ("virt.firmware_vars", virt.firmware_vars.is_some()),
                ("virt.initrd", virt.initrd.is_some()),
                ("virt.command_line", virt.command_line.is_some()),
                ("virt.device_tree", virt.device_tree.is_some()),
            ] {
                if set {
                    return Err(RunError::VirtSettingWithoutBoot { setting: name });
                }
            }
            ArmBoot::Unset
        }
    };
    let display = match virt.display {
        None => None,
        Some(VirtDisplayToml {
            width: Some(width),
            height: Some(height),
        }) if width != 0 && height != 0 => Some(VirtioDisplaySize { width, height }),
        Some(VirtDisplayToml { width, height }) => {
            return Err(RunError::InvalidVirtDisplay { width, height });
        }
    };
    let disk = match args.disk.clone().or_else(|| path(&virt.disk)) {
        Some(path) => Some(VirtDisk {
            path,
            access: DiskAccess::of_read_only(virt.disk_read_only.unwrap_or(false)),
        }),
        None if virt.disk_read_only.is_some() => {
            return Err(RunError::VirtSettingWithoutDisk {
                setting: "virt.disk_read_only",
            });
        }
        None => None,
    };
    Ok(ArmVirtConfig {
        memory_mib,
        ips,
        cpu: virt.cpu.map_or_else(rusty_box_aarch64::CpuModel::default, |cpu| cpu.0),
        boot,
        display,
        keyboard: virt.keyboard.unwrap_or(false),
        tablet: virt.tablet.unwrap_or(false),
        disk,
    })
}

impl EmulatorToml {
    /// Whether the table sets any key. The destructuring is exhaustive, so a
    /// new key is counted.
    fn is_set(&self) -> bool {
        let Self {
            engine,
            cpu_capabilities,
            memory_mib,
            host_memory_mib,
            memory_block_kib,
            cpus,
            cpu_sockets,
            cpu_cores,
            cpu_threads,
            ips,
            pci,
            sync_slowdown,
            sync_realtime,
            smp_quantum,
            max_instructions,
            cpuid_freq,
            port_e9_hack,
        } = self;
        engine.is_some()
            || cpu_capabilities.is_some()
            || memory_mib.is_some()
            || host_memory_mib.is_some()
            || memory_block_kib.is_some()
            || cpus.is_some()
            || cpu_sockets.is_some()
            || cpu_cores.is_some()
            || cpu_threads.is_some()
            || ips.is_some()
            || pci.is_some()
            || sync_slowdown.is_some()
            || sync_realtime.is_some()
            || smp_quantum.is_some()
            || max_instructions.is_some()
            || cpuid_freq.is_some()
            || port_e9_hack.is_some()
    }
}

/// The PC `file` and `args` describe.
fn resolve_pc(
    file: &FileConfig,
    args: &Args,
    config_dir: Option<&Path>,
    display: DisplayBackend,
) -> Result<PcConfig, RunError> {
    let engine = args.engine.or(file.emulator.engine).unwrap_or_default();
    let cpu_capabilities = args
        .cpu_capabilities
        .or(file.emulator.cpu_capabilities)
        .unwrap_or_default();

    let memory_mib = args
        .memory_mib
        .or(file.emulator.memory_mib)
        .unwrap_or(DEFAULT_MEMORY_MIB);
    ensure_nonzero("memory_mib", memory_mib)?;

    let host_memory_mib = args
        .host_memory_mib
        .or(file.emulator.host_memory_mib)
        .unwrap_or(memory_mib);
    ensure_nonzero("host_memory_mib", host_memory_mib)?;

    let memory_block_kib = args
        .memory_block_kib
        .or(file.emulator.memory_block_kib)
        .unwrap_or(DEFAULT_MEMORY_BLOCK_KIB);
    ensure_nonzero("memory_block_kib", memory_block_kib)?;

    let ips = args.ips.or(file.emulator.ips).unwrap_or(DEFAULT_IPS);
    ensure_nonzero("ips", ips)?;

    let pci = if args.pci {
        true
    } else if args.no_pci {
        false
    } else {
        file.emulator.pci.unwrap_or(DEFAULT_PCI)
    };
    let sync_slowdown = if args.sync_slowdown {
        true
    } else if args.no_sync_slowdown {
        false
    } else {
        file.emulator.sync_slowdown.unwrap_or(DEFAULT_SYNC_SLOWDOWN)
    };
    let max_instructions = args
        .max_instructions
        .or(file.emulator.max_instructions)
        .unwrap_or(DEFAULT_MAX_INSTRUCTIONS);
    // Bochs `clock: sync=realtime`; default none (see EmulatorToml docs).
    let sync_realtime = if args.sync_realtime {
        true
    } else {
        file.emulator.sync_realtime.unwrap_or(DEFAULT_SYNC_REALTIME)
    };
    // Bochs `cpu: quantum=N` (config.cc BXPN_SMP_QUANTUM): range 1-32
    // (config.h BX_SMP_QUANTUM_MIN/MAX), default 16.
    let smp_quantum = args
        .smp_quantum
        .or(file.emulator.smp_quantum)
        .unwrap_or(DEFAULT_SMP_QUANTUM);
    if !(1..=32).contains(&smp_quantum) {
        return Err(RunError::InvalidSmpQuantum { value: smp_quantum });
    }
    // Bochs `cpu: cpuid_freq=hardware|none|ips` (cpuid.cc get_freq_leaf_15/16).
    // rusty_box defaults to "none" so guests calibrate the true tick rate.
    let cpuid_freq = match args
        .cpuid_freq
        .as_deref()
        .or(file.emulator.cpuid_freq.as_deref())
    {
        None => DEFAULT_CPUID_FREQ,
        Some("none") => CpuidFreq::None,
        Some("hardware") => CpuidFreq::Hardware,
        Some("ips") => CpuidFreq::Ips,
        Some(other) => {
            return Err(RunError::InvalidCpuidFreq {
                value: other.to_string(),
            })
        }
    };
    // Bochs `port_e9_hack: enabled=, all_rings=` (unmapped.cc, io.cc
    // allow_io), off unless a configuration turns it on.
    let port_e9_hack = match args
        .port_e9_hack
        .as_deref()
        .or(file.emulator.port_e9_hack.as_deref())
    {
        None => PortE9Hack::Off,
        Some(spelled) => port_e9_hack_from_toml(spelled).ok_or_else(|| {
            RunError::InvalidPortE9Hack {
                value: spelled.to_string(),
            }
        })?,
    };
    let cpu_params = resolve_cpu_topology(file, args)?;

    let bios = match args.bios.clone() {
        Some(path) => path,
        None => file
            .rom
            .bios
            .clone()
            .map(|path| resolve_toml_path(config_dir, path))
            .ok_or(RunError::MissingBios)?,
    };
    let vga_bios = match args.vga_bios.clone() {
        Some(path) => Some(path),
        None => file
            .rom
            .vga_bios
            .clone()
            .map(|path| resolve_toml_path(config_dir, path)),
    };

    // A pre-boot VBE mode requires at least a width and height; bpp defaults to 32.
    let vga_mode = match (file.display.width, file.display.height) {
        (Some(width), Some(height)) if width != 0 && height != 0 => Some(VgaMode {
            width,
            height,
            bpp: file.display.bpp.unwrap_or(32),
        }),
        _ => None,
    };

    let pci_vga = file.display.pci_vga.unwrap_or(DEFAULT_PCI_VGA);

    let disk = resolve_disk(file, args, config_dir)?;
    let cdrom = resolve_cdrom(file, args, config_dir);
    let boot_order = resolve_boot_order(file, args, display, disk.is_some(), cdrom.is_some())?;
    validate_boot_order(&boot_order, disk.is_some(), cdrom.is_some())?;

    Ok(PcConfig {
        engine,
        cpu_capabilities,
        memory_mib,
        host_memory_mib,
        memory_block_kib,
        ips,
        pci,
        sync_slowdown,
        sync_realtime,
        smp_quantum,
        cpuid_freq,
        port_e9_hack,
        max_instructions,
        cpu_params,
        bios,
        vga_bios,
        boot_order,
        disk,
        cdrom,
        vga_mode,
        pci_vga,
    })
}

/// Resolve the CPU topology from CLI flags or TOML. `--cpu-sockets/-cores/-threads`
/// (or their `[emulator]` keys) take precedence when any is present; otherwise the
/// legacy flat `cpus` count maps to `(cpus, 1, 1)`.
fn resolve_cpu_topology(file: &FileConfig, args: &Args) -> Result<BxParams, RunError> {
    let sockets = args.cpu_sockets.or(file.emulator.cpu_sockets);
    let cores = args.cpu_cores.or(file.emulator.cpu_cores);
    let threads = args.cpu_threads.or(file.emulator.cpu_threads);

    let (n_processors, n_cores, n_threads) = if sockets.is_some()
        || cores.is_some()
        || threads.is_some()
    {
        (
            sockets.unwrap_or(1),
            cores.unwrap_or(1),
            threads.unwrap_or(1),
        )
    } else {
        let cpu_count = args.cpus.or(file.emulator.cpus).unwrap_or(1);
        (cpu_count, 1, 1)
    };

    BxParams::default()
        .with_topology(n_processors, n_cores, n_threads)
        .map_err(|error| RunError::InvalidCpuTopology {
            message: topology_error_message(error),
        })
}

pub(crate) fn topology_error_message(error: BxParamError) -> String {
    match error {
        BxParamError::TopologyComponentOutOfRange {
            component,
            value,
            min,
            max,
        } => format!("{component} = {value} is out of range ({min}..={max})"),
        BxParamError::TooManyLogicalProcessors { count, max } => {
            format!("{count} logical processors exceeds the SMP limit of {max}")
        }
    }
}

impl ResolvedConfig {
    /// A serializable [`FileConfig`] snapshot of these settings, as the VM
    /// library writes it. Paths are emitted as they are; `VmLibrary::save`
    /// makes relative ones absolute first, so reading its file back resolves
    /// to an equal configuration.
    pub fn to_file_config(&self) -> FileConfig {
        let mut file = match &self.machine {
            MachineConfig::Pc(pc) => pc.to_file_config(),
            MachineConfig::ArmVirt(virt) => FileConfig {
                vm: VmToml {
                    name: None,
                    machine: Some(MachineKind::ArmVirt),
                },
                virt: Some(virt.to_toml()),
                ..FileConfig::default()
            },
        };
        file.display.backend = Some(self.display);
        // Only persist a stretched console so existing configs stay stable.
        file.display.stretch = self.console_stretch.then_some(true);
        // Only persist a non-default speed so existing configs stay stable.
        file.display.pointer_speed_percent =
            (self.pointer_speed != PointerSpeed::DEFAULT).then_some(self.pointer_speed.percent());
        file.logging = LoggingToml {
            level: Some(self.log_level),
        };
        file
    }
}

impl ArmVirtConfig {
    /// The `[virt]` table that resolves to these settings.
    fn to_toml(&self) -> VirtToml {
        let mut virt = VirtToml {
            memory_mib: Some(self.memory_mib),
            ips: Some(self.ips),
            cpu: Some(VirtCpuToml(self.cpu)),
            keyboard: Some(self.keyboard),
            tablet: Some(self.tablet),
            disk: self.disk.as_ref().map(|disk| disk.path.clone()),
            disk_read_only: self.disk.as_ref().map(|disk| disk.access.is_read_only()),
            display: self.display.map(|size| VirtDisplayToml {
                width: Some(size.width),
                height: Some(size.height),
            }),
            ..VirtToml::default()
        };
        match &self.boot {
            ArmBoot::Unset => {}
            ArmBoot::Firmware { code, vars } => {
                virt.firmware = Some(code.clone());
                virt.firmware_vars = vars.clone();
            }
            ArmBoot::Linux {
                kernel,
                initrd,
                command_line,
                device_tree,
            } => {
                virt.kernel = Some(kernel.clone());
                virt.initrd = initrd.clone();
                virt.command_line = Some(command_line.clone());
                virt.device_tree = device_tree.clone();
            }
        }
        virt
    }
}

impl PcConfig {
    /// The PC tables that resolve to these settings; the front end's keys
    /// are [`ResolvedConfig::to_file_config`]'s to add.
    fn to_file_config(&self) -> FileConfig {
        let topology = self.cpu_params.cpu_topology();
        let emulator = EmulatorToml {
            // Only persist a non-default engine and processor so existing
            // configs stay stable.
            engine: (self.engine != Engine::default()).then_some(self.engine),
            cpu_capabilities: (self.cpu_capabilities != CpuCapabilities::default())
                .then_some(self.cpu_capabilities),
            memory_mib: Some(self.memory_mib),
            host_memory_mib: Some(self.host_memory_mib),
            memory_block_kib: Some(self.memory_block_kib),
            cpus: None,
            cpu_sockets: Some(topology.n_processors()),
            cpu_cores: Some(topology.n_cores()),
            cpu_threads: Some(topology.n_threads()),
            ips: Some(self.ips),
            pci: Some(self.pci),
            sync_slowdown: Some(self.sync_slowdown),
            // Only persist a non-default value so existing configs stay stable.
            sync_realtime: (self.sync_realtime != DEFAULT_SYNC_REALTIME)
                .then_some(self.sync_realtime),
            // Only persist a non-default quantum so existing configs stay stable.
            smp_quantum: (self.smp_quantum != DEFAULT_SMP_QUANTUM).then_some(self.smp_quantum),
            max_instructions: (self.max_instructions != DEFAULT_MAX_INSTRUCTIONS)
                .then_some(self.max_instructions),
            // Only persist a non-default mode so existing configs stay stable.
            cpuid_freq: (self.cpuid_freq != DEFAULT_CPUID_FREQ)
                .then(|| cpuid_freq_toml(self.cpuid_freq).to_owned()),
            port_e9_hack: (self.port_e9_hack != PortE9Hack::Off)
                .then(|| port_e9_hack_toml(self.port_e9_hack).to_owned()),
        };
        let display = DisplayToml {
            backend: None,
            width: self.vga_mode.map(|mode| mode.width),
            height: self.vga_mode.map(|mode| mode.height),
            bpp: self.vga_mode.map(|mode| mode.bpp),
            pci_vga: (self.pci_vga != DEFAULT_PCI_VGA).then_some(self.pci_vga),
            stretch: None,
            pointer_speed_percent: None,
        };
        let rom = RomToml {
            bios: Some(self.bios.clone()),
            vga_bios: self.vga_bios.clone(),
        };
        let boot = BootToml {
            order: self.boot_order.clone(),
        };
        let disk = self.disk.as_ref().map(|disk| match &disk.creation {
            Some(creation) => DiskToml {
                path: None,
                channel: Some(disk.channel),
                drive: Some(disk.drive),
                chs: None,
                create: Some(DiskCreateToml {
                    path: Some(creation.path.clone()),
                    size: Some(image_size_to_toml(creation.size)),
                    overwrite: Some(creation.overwrite),
                }),
            },
            None => DiskToml {
                path: Some(disk.path.clone()),
                channel: Some(disk.channel),
                drive: Some(disk.drive),
                chs: Some(disk.geometry),
                create: None,
            },
        });
        let cdrom = self.cdrom.as_ref().map(|cdrom| CdromToml {
            path: Some(cdrom.path.clone()),
            channel: Some(cdrom.channel),
            drive: Some(cdrom.drive),
        });
        FileConfig {
            vm: VmToml::default(),
            emulator,
            display,
            rom,
            boot,
            disk,
            cdrom,
            virt: None,
            logging: LoggingToml::default(),
        }
    }
}

/// Render an [`ImageSize`](rusty_box_bximage::ImageSize) back into a TOML size
/// string that `ImageSize::parse` reads to the identical value. Disk-creation
/// sizes always come from `parse`/`gib`/`mib`, so they are whole MiB multiples.
/// The spelling `emulator.cpuid_freq` gives `mode` (Bochs `cpu:
/// cpuid_freq=`), the one `resolve_config` reads back.
fn cpuid_freq_toml(mode: CpuidFreq) -> &'static str {
    match mode {
        CpuidFreq::None => "none",
        CpuidFreq::Hardware => "hardware",
        CpuidFreq::Ips => "ips",
    }
}

/// The spelling `emulator.port_e9_hack` and `--port-e9-hack` give `hack`,
/// the one [`port_e9_hack_from_toml`] reads back. One word for each of Bochs's
/// reachable `port_e9_hack: enabled=, all_rings=` combinations.
fn port_e9_hack_toml(hack: PortE9Hack) -> &'static str {
    match hack {
        PortE9Hack::Off => "off",
        PortE9Hack::On => "on",
        PortE9Hack::AllRings => "all-rings",
    }
}

/// The console a spelling names, or `None` for one that names none.
fn port_e9_hack_from_toml(spelled: &str) -> Option<PortE9Hack> {
    [PortE9Hack::Off, PortE9Hack::On, PortE9Hack::AllRings]
        .into_iter()
        .find(|hack| port_e9_hack_toml(*hack) == spelled)
}

fn image_size_to_toml(size: rusty_box_bximage::ImageSize) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    let bytes = size.bytes();
    if bytes % GIB == 0 {
        format!("{}G", bytes / GIB)
    } else {
        format!("{}M", bytes / MIB)
    }
}

pub fn load_toml_file(path: &Path) -> Result<FileConfig, RunError> {
    let contents = fs::read_to_string(path).map_err(|source| RunError::ConfigRead {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&contents).map_err(|source| RunError::ConfigParse {
        path: path.to_owned(),
        source,
    })
}

/// `path` based on the config file's directory when it is relative. An empty
/// path names no file, so there is nothing to base: it stays empty rather
/// than becoming the directory itself.
fn resolve_toml_path(config_dir: Option<&Path>, path: PathBuf) -> PathBuf {
    if path.is_relative() && !path.as_os_str().is_empty() {
        config_dir.map_or(path.clone(), |dir| dir.join(path))
    } else {
        path
    }
}

fn resolve_disk(
    file: &FileConfig,
    args: &Args,
    config_dir: Option<&Path>,
) -> Result<Option<ResolvedDisk>, RunError> {
    let disk_toml = file.disk.as_ref();
    let toml_create = disk_toml.and_then(|disk| disk.create.as_ref());
    let has_cli_create = args.create_disk.is_some()
        || args.create_disk_size.is_some()
        || args.overwrite_created_disk;

    if args.disk.is_some() {
        return resolve_existing_disk(disk_toml, args, config_dir);
    }

    if has_cli_create || toml_create.is_some() {
        return resolve_created_disk(disk_toml, toml_create, args, has_cli_create, config_dir);
    }

    if let Some(disk) = disk_toml {
        if disk.path.is_some() && disk.create.is_some() {
            return Err(RunError::ConflictingDiskOptions {
                first: "disk.path",
                second: "disk.create",
            });
        }
    }

    resolve_existing_disk(disk_toml, args, config_dir)
}

fn resolve_existing_disk(
    disk_toml: Option<&DiskToml>,
    args: &Args,
    config_dir: Option<&Path>,
) -> Result<Option<ResolvedDisk>, RunError> {
    let path = if let Some(path) = args.disk.clone() {
        path
    } else if let Some(path) = disk_toml.and_then(|disk| disk.path.clone()) {
        resolve_toml_path(config_dir, path)
    } else {
        return Ok(None);
    };

    let geometry = match args
        .disk_chs
        .or_else(|| disk_toml.and_then(|disk| disk.chs))
    {
        Some(geometry) => {
            validate_disk_geometry(geometry)?;
            geometry
        }
        None => auto_detect_chs(&path)?,
    };
    let channel = disk_toml.and_then(|disk| disk.channel).unwrap_or(0);
    let drive = disk_toml.and_then(|disk| disk.drive).unwrap_or(0);

    Ok(Some(ResolvedDisk {
        path,
        geometry,
        channel,
        drive,
        creation: None,
    }))
}

fn resolve_created_disk(
    disk_toml: Option<&DiskToml>,
    toml_create: Option<&DiskCreateToml>,
    args: &Args,
    has_cli_create: bool,
    config_dir: Option<&Path>,
) -> Result<Option<ResolvedDisk>, RunError> {
    if !has_cli_create && disk_toml.and_then(|disk| disk.path.as_ref()).is_some() {
        return Err(RunError::ConflictingDiskOptions {
            first: "disk.path",
            second: "disk.create",
        });
    }
    if args.disk_chs.is_some() || (!has_cli_create && disk_toml.and_then(|disk| disk.chs).is_some())
    {
        return Err(RunError::ConflictingDiskOptions {
            first: "disk.chs",
            second: "disk.create",
        });
    }

    let path = if let Some(path) = args.create_disk.clone() {
        path
    } else if let Some(path) = toml_create.and_then(|create| create.path.clone()) {
        resolve_toml_path(config_dir, path)
    } else {
        return Err(RunError::MissingDiskCreatePath);
    };
    let size = match args.create_disk_size {
        Some(size) => size.0,
        None => match toml_create.and_then(|create| create.size.as_deref()) {
            Some(size) => rusty_box_bximage::ImageSize::parse(size)
                .map_err(|source| RunError::DiskCreateSize { source })?,
            None => rusty_box_bximage::DEFAULT_HARD_DISK_SIZE,
        },
    };
    let overwrite = args.overwrite_created_disk
        || toml_create
            .and_then(|create| create.overwrite)
            .unwrap_or(false);

    // `calculate_hard_disk_geometry` already rejects cylinder counts >= BOCHS_MAX_CYLINDERS
    // (2^24) via `BxImageError::CylinderOverflow`, matching Bochs bximage. Anything it
    // returns fits the u32 geometry field, so no extra GUI-side cap is needed.
    let geometry = rusty_box_bximage::calculate_hard_disk_geometry(
        size,
        rusty_box_bximage::SectorSize::Bytes512,
    )
    .map_err(|source| RunError::DiskCreateSize { source })?;

    let channel = disk_toml.and_then(|disk| disk.channel).unwrap_or(0);
    let drive = disk_toml.and_then(|disk| disk.drive).unwrap_or(0);
    let geometry = DiskGeometry {
        cylinders: geometry.cylinders as u32,
        heads: geometry.heads as u8,
        sectors_per_track: geometry.sectors_per_track as u8,
    };

    Ok(Some(ResolvedDisk {
        path: path.clone(),
        geometry,
        channel,
        drive,
        creation: Some(ResolvedDiskCreation {
            path,
            size,
            overwrite,
        }),
    }))
}

fn resolve_cdrom(
    file: &FileConfig,
    args: &Args,
    config_dir: Option<&Path>,
) -> Option<ResolvedCdrom> {
    let path = match args.cdrom.clone() {
        Some(path) => path,
        None => resolve_toml_path(config_dir, file.cdrom.as_ref()?.path.clone()?),
    };
    let channel = file
        .cdrom
        .as_ref()
        .and_then(|cdrom| cdrom.channel)
        .unwrap_or(1);
    let drive = file
        .cdrom
        .as_ref()
        .and_then(|cdrom| cdrom.drive)
        .unwrap_or(0);

    Some(ResolvedCdrom {
        path,
        channel,
        drive,
    })
}

fn resolve_boot_order(
    file: &FileConfig,
    args: &Args,
    display: DisplayBackend,
    has_disk: bool,
    has_cdrom: bool,
) -> Result<Vec<BootDevice>, RunError> {
    let order = if !args.boot.is_empty() {
        args.boot.clone()
    } else if !file.boot.order.is_empty() {
        file.boot.order.clone()
    } else if has_disk {
        vec![BootDevice::Disk]
    } else if has_cdrom {
        vec![BootDevice::Cdrom]
    } else if allow_empty_boot_order(display) {
        Vec::new()
    } else {
        return Err(RunError::EmptyBootOrder);
    };

    if order.len() > 3 {
        return Err(RunError::TooManyBootDevices);
    }

    Ok(order)
}

fn validate_boot_order(
    boot_order: &[BootDevice],
    has_disk: bool,
    has_cdrom: bool,
) -> Result<(), RunError> {
    for (index, device) in boot_order.iter().enumerate() {
        if boot_order[index + 1..].contains(device) {
            return Err(RunError::DuplicateBootDevice { device: *device });
        }
    }

    for device in boot_order {
        match device {
            BootDevice::Disk if !has_disk => {
                return Err(RunError::MissingBootMedia {
                    device: *device,
                    field: "disk.path",
                });
            }
            BootDevice::Cdrom if !has_cdrom => {
                return Err(RunError::MissingBootMedia {
                    device: *device,
                    field: "cdrom.path",
                });
            }
            _ => {}
        }
    }

    Ok(())
}

fn validate_disk_geometry(geometry: DiskGeometry) -> Result<(), RunError> {
    if geometry.cylinders == 0 || geometry.heads == 0 || geometry.sectors_per_track == 0 {
        Err(RunError::ZeroValue { field: "disk.chs" })
    } else {
        Ok(())
    }
}

#[cfg(feature = "gui-egui")]
fn default_display_backend() -> DisplayBackend {
    DisplayBackend::Egui
}

#[cfg(not(feature = "gui-egui"))]
fn default_display_backend() -> DisplayBackend {
    DisplayBackend::Terminal
}

#[cfg(feature = "gui-egui")]
fn allow_empty_boot_order(display: DisplayBackend) -> bool {
    display == DisplayBackend::Egui
}

#[cfg(not(feature = "gui-egui"))]
fn allow_empty_boot_order(_display: DisplayBackend) -> bool {
    false
}

fn ensure_nonzero(field: &'static str, value: u32) -> Result<(), RunError> {
    if value == 0 {
        Err(RunError::ZeroValue { field })
    } else {
        Ok(())
    }
}

fn auto_detect_chs(path: &Path) -> Result<DiskGeometry, RunError> {
    const SECTOR_SIZE: u64 = 512;
    const HEADS: u64 = 16;
    const SECTORS_PER_TRACK: u64 = 63;
    // Bochs auto-geometry (harddrv.cc) derives the physical cylinder count uncapped and
    // only clamps the *reported* IDENTIFY geometry (word 1) to 16383. Match that: reject
    // only when the physical cylinder count would exceed the emulator's limit (2^24).
    const MAX_CYLINDERS: u64 = rusty_box_bximage::BOCHS_MAX_CYLINDERS - 1;

    let metadata = fs::metadata(path).map_err(|source| RunError::FileRead {
        kind: "disk",
        path: path.to_owned(),
        source: map_metadata_error(source),
    })?;
    let len = metadata.len();
    if len == 0 || len % SECTOR_SIZE != 0 {
        return Err(RunError::InvalidDiskSize {
            path: path.to_owned(),
            len,
        });
    }

    let total_sectors = len / SECTOR_SIZE;
    let sectors_per_cylinder = HEADS * SECTORS_PER_TRACK;
    let cylinders = total_sectors.div_ceil(sectors_per_cylinder);
    if cylinders > MAX_CYLINDERS {
        return Err(RunError::DiskTooLargeForChs {
            path: path.to_owned(),
            cylinders,
        });
    }

    Ok(DiskGeometry {
        cylinders: cylinders as u32,
        heads: HEADS as u8,
        sectors_per_track: SECTORS_PER_TRACK as u8,
    })
}

#[cfg(not(target_arch = "wasm32"))]
impl CpuCapabilities {
    /// Narrow `params` to the processor a guest can be offered under this
    /// setting.
    ///
    /// Only the features whose state a guest must enable through `XCR0` are
    /// considered, because those are the ones a host can refuse outright: the
    /// guest turns the component on itself and the register file has to exist.
    /// An instruction the host lacks but whose state it carries is this port's
    /// to emulate either way.
    pub fn narrow(self, params: BxParams) -> BxParams {
        match self {
            Self::Preset => params,
            Self::HostShared => {
                let carried = host_xsave_components();
                /// `CPUID.D:0` bits 5, 6 and 7 — the opmask registers, the
                /// upper half of ZMM0-15, and ZMM16-31. A guest enables them
                /// together, so a host either carries the set or none of it.
                const AVX512_STATE: u64 = (1 << 5) | (1 << 6) | (1 << 7);
                /// Bit 2 — the upper half of the YMM registers.
                const AVX_STATE: u64 = 1 << 2;

                let mut narrowed = params;
                if carried & AVX512_STATE != AVX512_STATE {
                    narrowed = narrowed.excluding(X86Feature::IsaAvx512);
                }
                if carried & AVX_STATE == 0 {
                    narrowed = narrowed.excluding(X86Feature::IsaAvx);
                }
                // `MONITOR`/`MWAIT` unconditionally, whatever the host's own
                // processor can do: the guest does not run on the host's
                // processor directly, it runs in a partition, and the platform
                // does not offer the instruction pair to one. A guest told it
                // has them commits to them permanently — Linux selects
                // `mwait_idle` at boot and never reconsiders — so this cannot
                // be discovered and worked around later.
                narrowed.excluding(X86Feature::IsaMonitorMwait)
            }
        }
    }
}

/// The extended-state components this host's processor can hold, as
/// `CPUID.D:0` reports them in EDX:EAX.
///
/// Answering zero where the leaf cannot be asked is the conservative reading —
/// it narrows the machine rather than widening it — but no host this runs on
/// lacks the leaf.
#[cfg(target_arch = "x86_64")]
fn host_xsave_components() -> u64 {
    let reported = core::arch::x86_64::__cpuid_count(0xD, 0);
    (u64::from(reported.edx) << 32) | u64::from(reported.eax)
}

#[cfg(all(not(target_arch = "x86_64"), not(target_arch = "wasm32")))]
fn host_xsave_components() -> u64 {
    0
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn detect_disk_geometry(path: &Path) -> Result<DiskGeometry, RunError> {
    auto_detect_chs(path)
}

fn map_metadata_error(error: io::Error) -> io::Error {
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use rusty_box_bximage::ImageSize;

    fn args<const N: usize>(values: [&str; N]) -> Args {
        Args::parse_from(values)
    }

    fn config(toml: &str) -> FileConfig {
        toml::from_str(toml).unwrap()
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()))
    }

    fn remove_test_file(path: &Path) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove {}: {error}", path.display()),
        }
    }

    fn remove_test_dir(path: &Path) {
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove {}: {error}", path.display()),
        }
    }

    #[test]
    fn a_vm_table_names_the_vm() {
        let file = config("[vm]\nname = \"Alpine\"\n[rom]\nbios = \"bios.bin\"\n");
        assert_eq!(file.vm.name.as_deref(), Some("Alpine"));
    }

    #[test]
    fn a_file_without_a_vm_table_still_parses() {
        let file = config("[rom]\nbios = \"bios.bin\"\n");
        assert_eq!(file.vm.name, None);
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn a_saved_file_carries_no_vm_name_of_its_own() {
        let file = config("[rom]\nbios = \"bios.bin\"\n");
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();
        assert_eq!(resolved.to_file_config().vm.name, None);
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn a_blank_config_is_what_an_empty_bios_path_resolves_to() {
        let file = FileConfig {
            rom: RomToml {
                bios: Some(PathBuf::new()),
                vga_bios: None,
            },
            ..FileConfig::default()
        };
        assert_eq!(resolve_config(file, &args(["rusty_box_gui"])).unwrap(), blank_config());
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn without_config_the_command_line_alone_describes_the_machine() {
        let resolved = load_config(&args(["rusty_box_gui", "--bios", "b.bin"])).unwrap();
        assert_eq!(resolved.pc().expect("a PC").bios, PathBuf::from("b.bin"));
        assert_eq!(resolved.memory_mib(), 32);
    }
    #[test]
    fn cli_overrides_toml_without_losing_other_toml_values() {
        let file = config(
            r#"
[emulator]
memory_mib = 64

[rom]
bios = "old.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui", "--bios", "new.bin"])).unwrap();

        assert_eq!(resolved.pc().expect("a PC").bios, PathBuf::from("new.bin"));
        assert_eq!(resolved.memory_mib(), 64);
    }

    #[test]
    fn cpuid_freq_defaults_to_none_and_parses_all_modes() {
        let resolved = resolve_config(
            config("[rom]\nbios = \"bios.bin\"\n\n[cdrom]\npath = \"boot.iso\"\n"),
            &args(["rusty_box_gui"]),
        )
        .unwrap();
        assert_eq!(resolved.pc().expect("a PC").cpuid_freq, CpuidFreq::None);

        let toml_hardware = r#"
[emulator]
cpuid_freq = "hardware"

[rom]
bios = "bios.bin"

[cdrom]
path = "boot.iso"
"#;
        let resolved = resolve_config(config(toml_hardware), &args(["rusty_box_gui"])).unwrap();
        assert_eq!(resolved.pc().expect("a PC").cpuid_freq, CpuidFreq::Hardware);

        // CLI overrides the TOML value.
        let resolved = resolve_config(
            config(toml_hardware),
            &args(["rusty_box_gui", "--cpuid-freq", "ips"]),
        )
        .unwrap();
        assert_eq!(resolved.pc().expect("a PC").cpuid_freq, CpuidFreq::Ips);

        // Save-roundtrip: non-default modes persist, the default stays absent.
        assert_eq!(
            resolved.to_file_config().emulator.cpuid_freq.as_deref(),
            Some("ips")
        );
    }

    /// The port-0xE9 console is off unless a VM asks for it, as Bochs's
    /// `port_e9_hack` is; each of its three settings is spelled one way in
    /// the file and on the command line, and the flag beats the file.
    #[test]
    fn port_e9_hack_defaults_to_off_and_parses_all_modes() {
        let resolved = resolve_config(
            config("[rom]\nbios = \"bios.bin\"\n\n[cdrom]\npath = \"boot.iso\"\n"),
            &args(["rusty_box_gui"]),
        )
        .unwrap();
        assert_eq!(resolved.pc().expect("a PC").port_e9_hack, PortE9Hack::Off);
        assert_eq!(
            resolved.to_file_config().emulator.port_e9_hack,
            None,
            "the default is not written, so existing VM files stay as they are"
        );

        let toml_on = r#"
[emulator]
port_e9_hack = "on"

[rom]
bios = "bios.bin"

[cdrom]
path = "boot.iso"
"#;
        let resolved = resolve_config(config(toml_on), &args(["rusty_box_gui"])).unwrap();
        assert_eq!(resolved.pc().expect("a PC").port_e9_hack, PortE9Hack::On);

        let resolved = resolve_config(
            config(toml_on),
            &args(["rusty_box_gui", "--port-e9-hack", "all-rings"]),
        )
        .unwrap();
        assert_eq!(resolved.pc().expect("a PC").port_e9_hack, PortE9Hack::AllRings);
        assert_eq!(
            resolved.to_file_config().emulator.port_e9_hack.as_deref(),
            Some("all-rings")
        );

        let resolved = resolve_config(
            config(toml_on),
            &args(["rusty_box_gui", "--port-e9-hack", "off"]),
        )
        .unwrap();
        assert_eq!(resolved.pc().expect("a PC").port_e9_hack, PortE9Hack::Off, "the flag can turn it back off");
    }

    #[test]
    fn port_e9_hack_rejects_a_mode_bochs_does_not_have() {
        let toml = r#"
[emulator]
port_e9_hack = "enabled"

[rom]
bios = "bios.bin"

[cdrom]
path = "boot.iso"
"#;
        assert!(matches!(
            resolve_config(config(toml), &args(["rusty_box_gui"])),
            Err(RunError::InvalidPortE9Hack { value }) if value == "enabled"
        ));
    }

    const WHP_FILE: &str = r#"
[emulator]
engine = "whp"
cpu_capabilities = "host-shared"

[rom]
bios = "bios.bin"

[cdrom]
path = "boot.iso"
"#;

    #[test]
    fn cli_engine_and_capabilities_beat_the_file_s() {
        let resolved = resolve_config(
            config(WHP_FILE),
            &args([
                "rusty_box_gui",
                "--engine",
                "interpreter",
                "--cpu-capabilities",
                "preset",
            ]),
        )
        .unwrap();

        assert_eq!(resolved.engine(), Engine::Interpreter);
        assert_eq!(resolved.pc().expect("a PC").cpu_capabilities, CpuCapabilities::Preset);
    }

    #[test]
    fn the_file_s_engine_and_capabilities_hold_when_the_command_line_names_none() {
        let resolved = resolve_config(config(WHP_FILE), &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved.engine(), Engine::Whp);
        assert_eq!(resolved.pc().expect("a PC").cpu_capabilities, CpuCapabilities::HostShared);
    }

    #[test]
    fn engine_and_capabilities_default_when_nothing_names_them() {
        let resolved = resolve_config(
            config("[rom]\nbios = \"bios.bin\"\n\n[cdrom]\npath = \"boot.iso\"\n"),
            &args(["rusty_box_gui"]),
        )
        .unwrap();

        assert_eq!(resolved.engine(), Engine::Interpreter);
        assert_eq!(resolved.pc().expect("a PC").cpu_capabilities, CpuCapabilities::Preset);
        // At their defaults, neither is written back.
        let emulator = resolved.to_file_config().emulator;
        assert_eq!(emulator.engine, None);
        assert_eq!(emulator.cpu_capabilities, None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_empty_bios_path_stays_empty_wherever_the_file_lives() {
        let file = config("[rom]\nbios = \"\"\n\n[cdrom]\npath = \"boot.iso\"\n");
        let resolved = resolve_config_in(file, Path::new("library")).unwrap();

        assert_eq!(resolved.pc().expect("a PC").bios, PathBuf::new());
        assert_eq!(
            resolved.pc().expect("a PC").cdrom.as_ref().map(|cdrom| cdrom.path.clone()),
            Some(Path::new("library").join("boot.iso"))
        );
    }

    #[test]
    fn cpuid_freq_rejects_unknown_mode() {
        let file = config(
            r#"
[emulator]
cpuid_freq = "fast"

[rom]
bios = "bios.bin"
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();
        assert!(matches!(error, RunError::InvalidCpuidFreq { value } if value == "fast"));
    }

    #[test]
    fn infers_disk_boot_when_disk_present() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved.pc().expect("a PC").boot_order, vec![BootDevice::Disk]);
    }

    #[test]
    fn resolves_toml_created_disk_with_vmware_like_default() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "c.img"
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();
        let disk = resolved.pc().expect("a PC").disk.clone().unwrap();
        let creation = disk.creation.unwrap();

        assert_eq!(disk.path, PathBuf::from("c.img"));
        assert_eq!(
            disk.geometry,
            DiskGeometry {
                cylinders: 41_610,
                heads: 16,
                sectors_per_track: 63,
            }
        );
        assert_eq!(creation.size, ImageSize::gib(20));
        assert!(!creation.overwrite);
        assert_eq!(resolved.pc().expect("a PC").boot_order, vec![BootDevice::Disk]);
    }

    #[test]
    fn resolves_toml_created_disk_size_string() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "c.img"
size = "512M"
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();
        let creation = resolved.pc().expect("a PC").disk.clone().unwrap().creation.unwrap();

        assert_eq!(creation.size, ImageSize::mib(512));
    }

    #[test]
    fn cli_create_disk_overrides_toml_disk_path() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "old.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved =
            resolve_config(file, &args(["rusty_box_gui", "--create-disk", "new.img"])).unwrap();
        let disk = resolved.pc().expect("a PC").disk.clone().unwrap();

        assert_eq!(disk.path, PathBuf::from("new.img"));
        assert!(disk.creation.is_some());
    }

    #[test]
    fn rejects_toml_disk_path_and_create_without_cli_override() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "old.img"

[disk.create]
path = "new.img"
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(
            error,
            RunError::ConflictingDiskOptions {
                first: "disk.path",
                second: "disk.create"
            }
        ));
    }

    #[test]
    fn rejects_create_size_without_path() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui", "--create-disk-size", "20G"]))
            .unwrap_err();

        assert!(matches!(error, RunError::MissingDiskCreatePath));
    }

    #[test]
    fn resolves_created_disk_larger_than_chs_u16() {
        // A 32 GiB disk needs 66576 cylinders (16h/63s/512b), which does not fit the
        // 16-bit ATA cylinder register but is a valid physical geometry in Bochs
        // (`unsigned cylinders`, addressed via LBA). It must resolve, not be rejected.
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "huge.img"
size = "32G"
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();
        let disk = resolved.pc().expect("a PC").disk.clone().unwrap();

        assert_eq!(disk.path, PathBuf::from("huge.img"));
        assert_eq!(
            disk.geometry,
            DiskGeometry {
                cylinders: 66_576,
                heads: 16,
                sectors_per_track: 63,
            }
        );
        assert_eq!(disk.creation.unwrap().size, ImageSize::gib(32));
    }

    #[test]
    fn rejects_invalid_toml_create_size() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "c.img"
size = "1.5G"
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(error, RunError::DiskCreateSize { .. }));
    }

    #[test]
    fn rejects_duplicate_boot_devices() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[boot]
order = ["disk", "disk"]

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(
            error,
            RunError::DuplicateBootDevice {
                device: BootDevice::Disk
            }
        ));
    }

    #[test]
    fn rejects_zero_toml_disk_geometry() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 0, heads = 4, sectors_per_track = 17 }
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(error, RunError::ZeroValue { field: "disk.chs" }));
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn defaults_to_egui_display_when_feature_enabled() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"
"#,
        );
        let resolved = resolve_config(
            file,
            &args(["rusty_box_gui", "--boot", "cdrom", "--cdrom", "boot.iso"]),
        )
        .unwrap();

        assert_eq!(resolved.display, DisplayBackend::Egui);
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn resolves_egui_display_backend_from_toml() {
        let file = config(
            r#"
[display]
backend = "egui"

[rom]
bios = "bios.bin"
"#,
        );
        let resolved = resolve_config(
            file,
            &args(["rusty_box_gui", "--boot", "cdrom", "--cdrom", "boot.iso"]),
        )
        .unwrap();

        assert_eq!(resolved.display, DisplayBackend::Egui);
    }

    #[cfg(not(feature = "gui-egui"))]
    #[test]
    fn defaults_to_terminal_display_when_egui_disabled() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"
"#,
        );
        let resolved = resolve_config(
            file,
            &args(["rusty_box_gui", "--boot", "cdrom", "--cdrom", "boot.iso"]),
        )
        .unwrap();

        assert_eq!(resolved.display, DisplayBackend::Terminal);
    }

    #[test]
    fn config_relative_paths_resolve_from_toml_directory() {
        let dir = unique_temp_dir("rusty-box-gui-relative-config");
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("rusty_box.toml");
        fs::write(
            &config_path,
            r#"
[rom]
bios = "bios.bin"
vga_bios = "vgabios.bin"

[boot]
order = ["disk", "cdrom"]

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }

[cdrom]
path = "boot.iso"
"#,
        )
        .unwrap();

        let resolved = load_config(&args([
            "rusty_box_gui",
            "--config",
            config_path.to_str().unwrap(),
        ]))
        .unwrap();

        assert_eq!(resolved.pc().expect("a PC").bios, dir.join("bios.bin"));
        assert_eq!(resolved.pc().expect("a PC").vga_bios, Some(dir.join("vgabios.bin")));
        assert_eq!(
            resolved.pc().expect("a PC").disk.as_ref().map(|disk| disk.path.as_path()),
            Some(dir.join("disk.img").as_path())
        );
        assert_eq!(
            resolved.pc().expect("a PC").cdrom.as_ref().map(|cdrom| cdrom.path.as_path()),
            Some(dir.join("boot.iso").as_path())
        );

        remove_test_file(&config_path);
        remove_test_dir(&dir);
    }

    #[test]
    fn config_relative_paths_preserve_cli_override_cwd_semantics() {
        let dir = unique_temp_dir("rusty-box-gui-relative-cli");
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("rusty_box.toml");
        fs::write(
            &config_path,
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        )
        .unwrap();

        let resolved = load_config(&args([
            "rusty_box_gui",
            "--config",
            config_path.to_str().unwrap(),
            "--bios",
            "new.bin",
            "--disk",
            "new.img",
            "--disk-chs",
            "306:4:17",
        ]))
        .unwrap();

        assert_eq!(resolved.pc().expect("a PC").bios, PathBuf::from("new.bin"));
        assert_eq!(
            resolved.pc().expect("a PC").disk.as_ref().map(|disk| disk.path.as_path()),
            Some(Path::new("new.img"))
        );

        remove_test_file(&config_path);
        remove_test_dir(&dir);
    }
    #[cfg(feature = "gui-egui")]
    #[test]
    fn egui_allows_launch_without_boot_media() {
        let file = config(
            r#"
[display]
backend = "egui"

[rom]
bios = "bios.bin"
"#,
        );

        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert!(resolved.pc().expect("a PC").boot_order.is_empty());
        assert!(resolved.pc().expect("a PC").disk.is_none());
        assert!(resolved.pc().expect("a PC").cdrom.is_none());
    }

    #[test]
    fn rejects_missing_bios() {
        let error = resolve_config(FileConfig::default(), &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(error, RunError::MissingBios));
    }

    #[test]
    fn resolves_cpu_topology_from_sockets_cores_threads() {
        let file = config(
            r#"
[emulator]
cpu_sockets = 2
cpu_cores = 2
cpu_threads = 2

[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved.pc().expect("a PC").cpu_params.cpu_count(), 8);
        let topology = resolved.pc().expect("a PC").cpu_params.cpu_topology();
        assert_eq!(topology.n_processors(), 2);
        assert_eq!(topology.n_cores(), 2);
        assert_eq!(topology.n_threads(), 2);
    }

    #[test]
    fn legacy_cpus_maps_to_flat_topology() {
        let file = config(
            r#"
[emulator]
cpus = 4

[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved.pc().expect("a PC").cpu_params.cpu_count(), 4);
        let topology = resolved.pc().expect("a PC").cpu_params.cpu_topology();
        assert_eq!(topology.n_processors(), 4);
        assert_eq!(topology.n_cores(), 1);
        assert_eq!(topology.n_threads(), 1);
    }

    #[test]
    fn topology_fields_take_precedence_over_legacy_cpus() {
        let file = config(
            r#"
[emulator]
cpus = 4
cpu_cores = 2

[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        // Any topology field present ignores the flat count: (1 socket, 2 cores, 1 thread).
        assert_eq!(resolved.pc().expect("a PC").cpu_params.cpu_count(), 2);
    }

    #[test]
    fn rejects_out_of_range_topology() {
        let file = config(
            r#"
[emulator]
cpu_cores = 99

[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(error, RunError::InvalidCpuTopology { .. }));
    }

    #[test]
    fn resolves_vga_mode_from_display_section() {
        let file = config(
            r#"
[display]
width = 1920
height = 1080

[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        // bpp defaults to 32 when omitted.
        assert_eq!(
            resolved.pc().expect("a PC").vga_mode,
            Some(VgaMode {
                width: 1920,
                height: 1080,
                bpp: 32,
            })
        );
    }

    #[test]
    fn console_stretch_is_read_from_the_display_section() {
        let file = config(
            r#"
[display]
stretch = true

[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert!(resolved.console_stretch);
        assert_eq!(resolved.to_file_config().display.stretch, Some(true));
    }

    #[test]
    fn a_file_without_stretch_keeps_the_guest_shape_and_writes_no_key() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert!(!resolved.console_stretch);
        assert_eq!(resolved.to_file_config().display.stretch, None);
    }

    #[test]
    fn vga_mode_requires_both_width_and_height() {
        let file = config(
            r#"
[display]
width = 1920

[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved.pc().expect("a PC").vga_mode, None);
    }

    #[test]
    fn save_round_trip_reparses_equal() {
        let file = config(
            r#"
[emulator]
engine = "whp"
cpu_capabilities = "host-shared"
memory_mib = 512
host_memory_mib = 512
memory_block_kib = 128
cpu_sockets = 2
cpu_cores = 2
cpu_threads = 1
ips = 100000000
pci = true
sync_slowdown = false

[display]
backend = "headless"
width = 1920
height = 1080
bpp = 32
pci_vga = true

[rom]
bios = "bios.bin"
vga_bios = "vgabios.bin"

[boot]
order = ["cdrom", "disk"]

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
channel = 0
drive = 0

[cdrom]
path = "boot.iso"
channel = 1
drive = 0

[logging]
level = "info"
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        let serialized = toml::to_string_pretty(&resolved.to_file_config()).unwrap();
        let reparsed: FileConfig = toml::from_str(&serialized).unwrap();
        let round_tripped = resolve_config(reparsed, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved, round_tripped);
        assert!(serialized.contains("engine = \"whp\""));
        assert!(serialized.contains("cpu_capabilities = \"host-shared\""));
        // A file with nothing to say about the VM carries no `[vm]` table.
        assert!(!serialized.contains("[vm]"));
    }

    #[test]
    fn save_round_trip_preserves_created_disk() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "c.img"
size = "12G"
overwrite = false
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        let serialized = toml::to_string_pretty(&resolved.to_file_config()).unwrap();
        let reparsed: FileConfig = toml::from_str(&serialized).unwrap();
        let round_tripped = resolve_config(reparsed, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved, round_tripped);
        let creation = round_tripped
            .pc().expect("a PC").disk
            .as_ref()
            .and_then(|disk| disk.creation.as_ref())
            .expect("created disk should survive the round trip");
        assert_eq!(creation.size, rusty_box_bximage::ImageSize::gib(12));
    }

    /// A file the shell keeps resolves the same under `Args::default()` as
    /// under an empty command line, which `resolve_config_in` relies on.
    #[test]
    fn a_shell_file_resolves_the_same_under_the_default_args_as_under_no_flags() {
        let empty = Args::try_parse_from(["rusty_box_gui"]).expect("the command line parses");

        assert_eq!(
            resolve_config(config(WHP_FILE), &Args::default()).unwrap(),
            resolve_config(config(WHP_FILE), &empty).unwrap()
        );
    }

    /// Every setting away from its default survives a save and a load: a
    /// field `to_file_config` drops, or writes in a form `resolve_config`
    /// reads differently, fails here. The literal names every field, so a
    /// field added later must be given a value here too.
    #[test]
    fn save_round_trip_keeps_every_setting_away_from_its_default() {
        let size = ImageSize::gib(2);
        let geometry = rusty_box_bximage::calculate_hard_disk_geometry(
            size,
            rusty_box_bximage::SectorSize::Bytes512,
        )
        .unwrap();
        let resolved = ResolvedConfig {
            machine: MachineConfig::Pc(PcConfig {
                engine: Engine::Whp,
                cpu_capabilities: CpuCapabilities::HostShared,
                memory_mib: 512,
                host_memory_mib: 256,
                memory_block_kib: 256,
                ips: 100_000_000,
                pci: !DEFAULT_PCI,
                sync_slowdown: !DEFAULT_SYNC_SLOWDOWN,
                sync_realtime: !DEFAULT_SYNC_REALTIME,
                smp_quantum: 8,
                cpuid_freq: CpuidFreq::Hardware,
                port_e9_hack: PortE9Hack::AllRings,
                max_instructions: 15_000_000_000,
                cpu_params: BxParams::default().with_topology(2, 2, 1).unwrap(),
                bios: PathBuf::from("roms/bios.bin"),
                vga_bios: Some(PathBuf::from("roms/vgabios.bin")),
                boot_order: vec![BootDevice::Cdrom, BootDevice::Disk],
                disk: Some(ResolvedDisk {
                    path: PathBuf::from("c.img"),
                    geometry: DiskGeometry {
                        cylinders: geometry.cylinders as u32,
                        heads: geometry.heads as u8,
                        sectors_per_track: geometry.sectors_per_track as u8,
                    },
                    channel: 0,
                    drive: 1,
                    creation: Some(ResolvedDiskCreation {
                        path: PathBuf::from("c.img"),
                        size,
                        overwrite: true,
                    }),
                }),
                cdrom: Some(ResolvedCdrom {
                    path: PathBuf::from("boot.iso"),
                    channel: 1,
                    drive: 1,
                }),
                vga_mode: Some(VgaMode {
                    width: 1280,
                    height: 1024,
                    bpp: 16,
                }),
                pci_vga: !DEFAULT_PCI_VGA,
            }),
            display: DisplayBackend::Headless,
            log_level: LogLevel::Debug,
            console_stretch: true,
            pointer_speed: PointerSpeed::from_percent(175),
        };

        let serialized = toml::to_string_pretty(&resolved.to_file_config()).unwrap();
        let reparsed: FileConfig = toml::from_str(&serialized).unwrap();
        let round_tripped = resolve_config(reparsed, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(round_tripped, resolved, "{serialized}");
    }

    /// An Arm board read back from what the library writes for it is the
    /// board it wrote, for each way it boots, each processor and each disk.
    #[test]
    fn an_arm_virt_vm_round_trips_through_its_file() {
        let boots = [
            ArmBoot::Unset,
            ArmBoot::Firmware {
                code: PathBuf::from("QEMU_EFI.fd"),
                vars: Some(PathBuf::from("vars.fd")),
            },
            ArmBoot::Linux {
                kernel: PathBuf::from("Image"),
                initrd: Some(PathBuf::from("initramfs.cpio.gz")),
                command_line: "console=ttyAMA0 console=tty0".to_owned(),
                device_tree: Some(PathBuf::from("virt.dtb")),
            },
        ];
        let processors = rusty_box_aarch64::CpuModel::ALL.into_iter().cycle();
        let disks = [
            None,
            Some(VirtDisk {
                path: PathBuf::from("alpine.img"),
                access: DiskAccess::ReadWrite,
            }),
            Some(VirtDisk {
                path: PathBuf::from("base.img"),
                access: DiskAccess::ReadOnly,
            }),
        ];
        for ((boot, cpu), disk) in boots.into_iter().zip(processors).zip(disks) {
            let resolved = ResolvedConfig {
                machine: MachineConfig::ArmVirt(ArmVirtConfig {
                    memory_mib: 1024,
                    ips: 75_000_000,
                    cpu,
                    boot,
                    display: Some(VirtioDisplaySize {
                        width: 1024,
                        height: 768,
                    }),
                    keyboard: true,
                    tablet: true,
                    disk,
                }),
                display: DisplayBackend::Headless,
                log_level: LogLevel::Info,
                console_stretch: true,
                pointer_speed: PointerSpeed::from_percent(150),
            };

            let serialized = toml::to_string_pretty(&resolved.to_file_config()).unwrap();
            assert!(serialized.contains("machine = \"arm-virt\""), "{serialized}");
            let reparsed: FileConfig = toml::from_str(&serialized).unwrap();
            let round_tripped = resolve_config(reparsed, &args(["rusty_box_gui"])).unwrap();

            assert_eq!(round_tripped, resolved, "{serialized}");
        }
    }

    #[test]
    fn an_empty_virt_table_is_a_board_with_nothing_to_boot_at_qemu_defaults() {
        let file: FileConfig = toml::from_str("[vm]\nmachine = \"arm-virt\"\n").unwrap();
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved.arm_virt(), Some(&ArmVirtConfig::blank()));
        assert_eq!(ArmVirtConfig::blank().memory_mib, 256, "QEMU virt's default RAM");
        assert_eq!(ArmVirtConfig::blank().cpu, rusty_box_aarch64::CpuModel::CortexA57);
    }

    /// `virt.cpu` names the processor by its model's name; a name no model
    /// has is refused when the file is read, naming the ones there are.
    #[test]
    fn the_virt_processor_is_named_by_its_model() {
        let board = |cpu: &str| format!("[vm]\nmachine = \"arm-virt\"\n[virt]\ncpu = \"{cpu}\"\n");
        for model in rusty_box_aarch64::CpuModel::ALL {
            let file: FileConfig = toml::from_str(&board(model.name())).unwrap();
            let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();
            assert_eq!(resolved.arm_virt().map(|virt| virt.cpu), Some(model));
        }
        let refused = toml::from_str::<FileConfig>(&board("cortex-a53")).unwrap_err().to_string();
        assert!(
            refused.contains("\"cortex-a53\": no processor model has that name")
                && refused.contains("\"cortex-a57\"")
                && refused.contains("\"armv9.2\""),
            "{refused}"
        );
    }

    #[test]
    fn a_kernel_boot_without_a_command_line_gets_the_serial_console() {
        let file: FileConfig =
            toml::from_str("[vm]\nmachine = \"arm-virt\"\n[virt]\nkernel = \"Image\"\n").unwrap();
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        match &resolved.arm_virt().expect("an Arm board").boot {
            ArmBoot::Linux { command_line, .. } => {
                assert_eq!(command_line, rusty_box_aarch64::boot::SERIAL_CONSOLE_COMMAND_LINE);
            }
            other => panic!("not a kernel boot: {other:?}"),
        }
    }

    /// Each machine refuses the other's settings, in the file and on the
    /// command line, rather than dropping them.
    #[test]
    fn a_setting_of_the_other_machine_is_refused() {
        let pc_with_virt: FileConfig = toml::from_str("[virt]\nmemory_mib = 512\n").unwrap();
        assert!(matches!(
            resolve_config(pc_with_virt, &args(["rusty_box_gui"])),
            Err(RunError::NotForMachine {
                setting: "[virt]",
                machine: MachineKind::Pc
            })
        ));

        let arm = "[vm]\nmachine = \"arm-virt\"\n";
        for (extra, setting) in [
            ("[emulator]\npci = false\n", "[emulator]"),
            ("[rom]\nbios = \"bios.bin\"\n", "[rom]"),
            ("[boot]\norder = [\"cdrom\"]\n", "[boot]"),
            ("[cdrom]\npath = \"boot.iso\"\n", "[cdrom]"),
            ("[display]\nwidth = 1024\n", "display.width"),
        ] {
            let file: FileConfig = toml::from_str(&format!("{arm}{extra}")).unwrap();
            let refused = resolve_config(file, &args(["rusty_box_gui"]));
            assert!(
                matches!(
                    refused,
                    Err(RunError::NotForMachine {
                        setting: named,
                        machine: MachineKind::ArmVirt
                    }) if named == setting
                ),
                "{extra}: {refused:?}"
            );
        }

        let file: FileConfig = toml::from_str(arm).unwrap();
        let refused = resolve_config(file, &args(["rusty_box_gui", "--cdrom", "boot.iso"]));
        assert!(matches!(
            refused,
            Err(RunError::NotForMachine {
                setting: "--cdrom",
                machine: MachineKind::ArmVirt
            })
        ));
    }

    /// The flags every machine reads apply to an Arm board too; `--disk`
    /// replaces the file's disk and keeps how the guest may use it.
    #[test]
    fn memory_rate_and_disk_flags_apply_to_an_arm_board() {
        let file: FileConfig = toml::from_str(
            "[vm]\nmachine = \"arm-virt\"\n[virt]\ndisk = \"file.img\"\ndisk_read_only = true\n",
        )
        .unwrap();
        let resolved = resolve_config(
            file,
            &args([
                "rusty_box_gui",
                "--memory-mib",
                "512",
                "--ips",
                "20000000",
                "--disk",
                "flag.img",
            ]),
        )
        .unwrap();
        let virt = resolved.arm_virt().expect("an Arm board");

        assert_eq!(virt.memory_mib, 512);
        assert_eq!(virt.ips, 20_000_000);
        assert_eq!(
            virt.disk,
            Some(VirtDisk {
                path: PathBuf::from("flag.img"),
                access: DiskAccess::ReadOnly,
            })
        );
    }

    /// `virt.disk` is found beside the VM file, as its other paths are, and
    /// the guest writes it unless `disk_read_only` says otherwise.
    #[test]
    fn the_virt_disk_is_found_beside_its_file_and_written_by_default() {
        let file: FileConfig =
            toml::from_str("[vm]\nmachine = \"arm-virt\"\n[virt]\ndisk = \"alpine.img\"\n").unwrap();
        let dir = Path::new("vms");
        let resolved = resolve_config_with_base(file, &args(["rusty_box_gui"]), Some(dir)).unwrap();

        assert_eq!(
            resolved.arm_virt().and_then(|virt| virt.disk.clone()),
            Some(VirtDisk {
                path: dir.join("alpine.img"),
                access: DiskAccess::ReadWrite,
            })
        );
    }

    #[test]
    fn an_arm_board_refuses_what_its_boot_cannot_use() {
        let refusal = |virt: &str| {
            let file: FileConfig =
                toml::from_str(&format!("[vm]\nmachine = \"arm-virt\"\n[virt]\n{virt}")).unwrap();
            resolve_config(file, &args(["rusty_box_gui"])).unwrap_err()
        };

        assert!(matches!(
            refusal("firmware = \"QEMU_EFI.fd\"\nkernel = \"Image\"\n"),
            RunError::ConflictingVirtBoot
        ));
        assert!(matches!(
            refusal("firmware = \"QEMU_EFI.fd\"\ninitrd = \"initrd\"\n"),
            RunError::VirtSettingNeedsKernel { setting: "virt.initrd" }
        ));
        assert!(matches!(
            refusal("kernel = \"Image\"\nfirmware_vars = \"vars.fd\"\n"),
            RunError::VirtSettingNeedsFirmware { setting: "virt.firmware_vars" }
        ));
        assert!(matches!(
            refusal("command_line = \"quiet\"\n"),
            RunError::VirtSettingWithoutBoot { setting: "virt.command_line" }
        ));
        assert!(matches!(
            refusal("disk_read_only = false\n"),
            RunError::VirtSettingWithoutDisk { setting: "virt.disk_read_only" }
        ));
        assert!(matches!(
            refusal("memory_mib = 0\n"),
            RunError::InvalidVirtMemory { memory_mib: 0 }
        ));
        assert!(matches!(
            refusal("[virt.display]\nwidth = 1024\n"),
            RunError::InvalidVirtDisplay {
                width: Some(1024),
                height: None
            }
        ));
    }
}
