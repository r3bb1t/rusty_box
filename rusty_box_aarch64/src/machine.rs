//! The machine: a processor on the `virt` board, the run loop that advances
//! them together, and everything a host drives a guest with.
//!
//! The run loop is where determinism is kept:
//!
//! - interrupts are taken only at instruction boundaries, from line levels
//!   that change only at instructions (an MMIO access, a timer register
//!   write), at a precomputed tick (`next_event`), or at a host action
//!   between runs;
//! - waiting for an interrupt jumps time straight to the next event, and the
//!   processor stays in the waiting state across calls, so a run that is cut
//!   into budgets behaves exactly like one that is not;
//! - nothing reads the host clock.
//!
//! A machine is built by [`MachineBuilder`], whose type says how the guest
//! starts — firmware, a Linux kernel loaded directly, or nothing at all — so
//! a machine with no way to start cannot be built and a kernel command line
//! cannot be given to firmware.

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, sync::Arc, vec};
use core::sync::atomic::{AtomicBool, Ordering};

use rusty_box_core::input::terminal::TerminalKeyboard;
use rusty_box_core::input::{Key, KeyAction, KeyStroke, KeyboardInput, Typed};
use rusty_box_core::machine::{Machine, RunOutcome, SerialPort};
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::{ClockHz, Progress, RunBudget, SnapError, VmClock, VmInstant};

use crate::board::{Plugged, Plugs, VirtBoard, VirtioDevices, FLASH0_BASE, FLASH_BANK_SIZE, RAM_BASE, RAM_LIMIT};
#[cfg(feature = "alloc")]
use crate::disk::Disk;
use crate::devices::virtio::input::InputKind;
use rusty_box_core::input::pointer::{PointerButton, PointerInput, PointerPos, WheelSteps};
#[cfg(feature = "alloc")]
use rusty_box_devices::display::sink::{DisplaySink, Refreshed};
#[cfg(feature = "alloc")]
use rusty_box_devices::display::Resolution;
use crate::boot::fdt::{self, Edit, Value};
use crate::boot::{self, DeviceTreeError, KernelLayout, LinuxImage, PlacementError, DEVICE_TREE_CAPACITY};
use crate::bus::{le_value, AccessWidth, BusFault, PhysAddr, PhysBus, PhysSpan};
use crate::cpu::{
    Activity, Cpu, CpuConfig, CpuState, DebugStops, Pstate, Reg, Step, StopCause, SysReg, SysRegRefusal,
    TranslationFault, UndefinedInsn, VReg,
};
use crate::devices::pl011::TX_CAPACITY;
use crate::instrumentation::{Instrumentation, NoInstrumentation};
#[cfg(feature = "alloc")]
use crate::storage::OwnedStorage;
use crate::storage::{GuestStorage, LentStorage};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// What a machine does when its guest asks for a system reset (PSCI
/// `SYSTEM_RESET`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnResetRequest {
    /// Reset and carry on running, as QEMU does unless told `-no-reboot`.
    Reset,
    /// Stop with [`StopReason::ResetRequested`] and leave the reset to the
    /// host ([`Power::reset`]).
    Stop,
}

/// How a machine runs, whatever it boots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtConfig {
    /// Emulated instructions per second: the rate of the tick count, and so
    /// of every clock the guest can read.
    pub ips: ClockHz,
    /// The generic timer's system counter frequency.
    pub counter_hz: ClockHz,
    /// What the RTC reads at emulated time zero, in seconds since the Unix
    /// epoch.
    pub rtc_epoch_seconds: u64,
    /// Faults that stop the machine instead of reaching the guest.
    pub debug_stops: DebugStops,
    pub on_reset_request: OnResetRequest,
}

impl VirtConfig {
    /// 2026-01-01T00:00:00Z.
    pub const DEFAULT_RTC_EPOCH: u64 = 1_767_225_600;

    pub const DEFAULT: Self = Self {
        ips: ClockHz::BOCHS_DEFAULT,
        counter_hz: CpuConfig::QEMU_VIRT_COUNTER_HZ,
        rtc_epoch_seconds: Self::DEFAULT_RTC_EPOCH,
        debug_stops: DebugStops {
            on_undefined: false,
            on_bus_fault: false,
        },
        on_reset_request: OnResetRequest::Reset,
    };
}

impl Default for VirtConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How much RAM a machine has (R4: a size, not a count of anything else).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RamSize(u64);

impl RamSize {
    /// The RAM the bundled device tree describes (QEMU's `-m 256`). QEMU's
    /// own default for `virt` is 128 MiB (`machine_class_init`
    /// `default_ram_size` in `hw/core/machine.c`); a machine of that size is
    /// `RamSize::mib(128)`.
    pub const DEFAULT: Self = Self(256 << 20);
    /// The least a machine can have: room for the device tree.
    pub const MIN: Self = Self(DEVICE_TREE_CAPACITY as u64);
    /// The most the board can map.
    pub const MAX: Self = Self(RAM_LIMIT);

    /// `bytes` of RAM, if it is a whole number of 4 KiB pages between
    /// [`RamSize::MIN`] and [`RamSize::MAX`].
    pub const fn bytes(bytes: u64) -> Option<Self> {
        if bytes.is_multiple_of(4096) && bytes >= Self::MIN.0 && bytes <= Self::MAX.0 {
            Some(Self(bytes))
        } else {
            None
        }
    }

    /// `mib` mebibytes of RAM, within the same bounds.
    pub const fn mib(mib: u64) -> Option<Self> {
        match mib.checked_mul(1 << 20) {
            Some(bytes) => Self::bytes(bytes),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Why a machine could not be built.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuildError {
    /// The firmware image is larger than its 64 MiB flash bank.
    FirmwareTooLarge,
    /// The variable-store image is larger than its flash bank, or than the
    /// buffer lent for it.
    VariablesTooLarge,
    /// The RAM lent to [`MachineBuilder::build_in`] is not the size the
    /// builder was configured with.
    LentRamSize { lent: u64, configured: u64 },
    /// The device tree could not be edited to describe this machine.
    DeviceTree(DeviceTreeError),
    /// The kernel, initrd or device tree does not fit in RAM.
    Placement(PlacementError),
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FirmwareTooLarge => f.write_str("the firmware is larger than a 64 MiB flash bank"),
            Self::VariablesTooLarge => f.write_str("the variable store is larger than its flash bank"),
            Self::LentRamSize { lent, configured } => {
                write!(f, "{lent} bytes of RAM were lent for a machine configured with {configured}")
            }
            Self::DeviceTree(e) => write!(f, "device tree: {e}"),
            Self::Placement(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::DeviceTree(e) => Some(e),
            Self::Placement(e) => Some(e),
            Self::FirmwareTooLarge | Self::VariablesTooLarge | Self::LentRamSize { .. } => None,
        }
    }
}

impl From<DeviceTreeError> for BuildError {
    fn from(e: DeviceTreeError) -> Self {
        Self::DeviceTree(e)
    }
}

impl From<PlacementError> for BuildError {
    fn from(e: PlacementError) -> Self {
        Self::Placement(e)
    }
}

// ---------------------------------------------------------------------------
// The builder
// ---------------------------------------------------------------------------

/// A builder that has not been told how the guest starts. It cannot build.
#[derive(Debug, Clone, Copy)]
pub struct NoBoot;

/// Boot UEFI firmware from flash bank 0, as QEMU's `-bios`/`-pflash` does.
#[derive(Debug, Clone, Copy)]
pub struct Firmware<'i> {
    code: &'i [u8],
    variables: Option<&'i [u8]>,
}

/// Load a Linux kernel directly, as QEMU's `-kernel` does without firmware.
#[derive(Debug, Clone, Copy)]
pub struct DirectKernel<'i> {
    image: LinuxImage<'i>,
    initrd: &'i [u8],
    command_line: &'i str,
}

/// Load nothing: the processor starts at the base of RAM, for a host that
/// writes its own code there.
#[derive(Debug, Clone, Copy)]
pub struct Bare;

/// Assembles a machine. The `B` parameter records how the guest starts;
/// only a builder that knows ([`Firmware`], [`DirectKernel`], [`Bare`]) has
/// `build` (with `alloc`: the machine owns its memory) and `build_in` (the
/// caller lends it).
///
/// ```
/// use rusty_box_aarch64::machine::{MachineBuilder, MemoryLoan, RamSize, VirtConfig};
///
/// let mut ram = vec![0u8; 16 << 20];
/// let machine = MachineBuilder::new(VirtConfig::default())
///     .ram(RamSize::mib(16).expect("a valid size"))
///     .bare()
///     .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
/// assert_eq!(machine.ticks(), 0);
/// # Ok::<(), rusty_box_aarch64::machine::BuildError>(())
/// ```
#[must_use = "a builder does nothing until it builds"]
pub struct MachineBuilder<'i, B = NoBoot, I = NoInstrumentation> {
    config: VirtConfig,
    ram: RamSize,
    device_tree: &'i [u8],
    boot: B,
    observer: I,
    plugs: Plugs,
}

impl<'i> MachineBuilder<'i, NoBoot, NoInstrumentation> {
    /// A builder with [`RamSize::DEFAULT`] of RAM and the device tree QEMU
    /// generates for this board.
    pub fn new(config: VirtConfig) -> Self {
        Self {
            config,
            ram: RamSize::DEFAULT,
            device_tree: boot::QEMU_VIRT_DEVICE_TREE,
            boot: NoBoot,
            observer: NoInstrumentation,
            plugs: Plugs::NONE,
        }
    }
}

impl<'i, B, I> MachineBuilder<'i, B, I> {
    /// How much RAM the machine has. The device tree's memory node is set to
    /// match.
    pub fn ram(mut self, size: RamSize) -> Self {
        self.ram = size;
        self
    }

    /// Describe the board with this device tree instead of QEMU's. Its
    /// memory node, and for a kernel its `/chosen` node, are set as the
    /// machine requires.
    pub fn device_tree(mut self, tree: &'i [u8]) -> Self {
        self.device_tree = tree;
        self
    }

    /// Watch the guest with `observer`.
    pub fn instrumentation<J: Instrumentation>(self, observer: J) -> MachineBuilder<'i, B, J> {
        MachineBuilder {
            config: self.config,
            ram: self.ram,
            device_tree: self.device_tree,
            boot: self.boot,
            observer,
            plugs: self.plugs,
        }
    }

    /// Plug in a virtio-gpu (QEMU `-device virtio-gpu-device`) whose display
    /// asks the guest for `preferred` pixels. The guest draws through it —
    /// Linux's `virtio_gpu` DRM driver and fbcon, EDK2's `VirtioGpuDxe` —
    /// and the host shows it through [`VirtMachine::display`]. Its resources
    /// are host memory the guest sizes, hence `alloc`.
    #[cfg(feature = "alloc")]
    pub fn virtio_display(mut self, preferred: Resolution) -> Self {
        self.plugs.devices.display = Some(preferred);
        self
    }

    /// Plug in a virtio keyboard (QEMU `-device virtio-keyboard-device`).
    /// [`VirtMachine::keyboard`] then types into it rather than into the
    /// serial console.
    pub fn virtio_keyboard(mut self) -> Self {
        self.plugs.devices.keyboard = Plugged::Yes;
        self
    }

    /// Plug in a virtio tablet (QEMU `-device virtio-tablet-device`), the
    /// absolute pointer [`VirtMachine::pointer`] drives.
    pub fn virtio_tablet(mut self) -> Self {
        self.plugs.devices.tablet = Plugged::Yes;
        self
    }

    /// Plug in a virtio-blk over `disk` (QEMU `-drive
    /// file=…,format=raw,if=none,id=hd0 -device virtio-blk-device,drive=hd0`),
    /// which Linux sees as `/dev/vda`. The machine keeps the disk:
    /// [`VirtMachine::disk`] reaches it again. A disk is host memory or a
    /// host file, hence `alloc`.
    #[cfg(feature = "alloc")]
    pub fn virtio_disk(mut self, disk: Disk) -> Self {
        self.plugs.disk = Some(disk);
        self
    }
}

impl<'i, I> MachineBuilder<'i, NoBoot, I> {
    /// Start from UEFI firmware built for QEMU's `virt` (EDK2
    /// `ArmVirtQemu`). The image goes in flash bank 0 unpadded.
    pub fn firmware(self, code: &'i [u8]) -> MachineBuilder<'i, Firmware<'i>, I> {
        self.with_boot(Firmware { code, variables: None })
    }

    /// Start a Linux kernel directly.
    pub fn linux(self, kernel: LinuxImage<'i>) -> MachineBuilder<'i, DirectKernel<'i>, I> {
        self.with_boot(DirectKernel {
            image: kernel,
            initrd: &[],
            command_line: "",
        })
    }

    /// Start nothing.
    pub fn bare(self) -> MachineBuilder<'i, Bare, I> {
        self.with_boot(Bare)
    }

    fn with_boot<B>(self, boot: B) -> MachineBuilder<'i, B, I> {
        MachineBuilder {
            config: self.config,
            ram: self.ram,
            device_tree: self.device_tree,
            boot,
            observer: self.observer,
            plugs: self.plugs,
        }
    }
}

impl<'i, I> MachineBuilder<'i, Firmware<'i>, I> {
    /// The variable store's initial contents (an EDK2 `VARS.fd`). `build`
    /// puts it at the front of an otherwise erased bank, and without one the
    /// store starts erased; `build_in` copies it over the front of the lent
    /// array, and without one uses that array as it is.
    pub fn variables(mut self, image: &'i [u8]) -> Self {
        self.boot.variables = Some(image);
        self
    }
}

impl<'i, I> MachineBuilder<'i, DirectKernel<'i>, I> {
    /// An initial RAM filesystem.
    pub fn initrd(mut self, initrd: &'i [u8]) -> Self {
        self.boot.initrd = initrd;
        self
    }

    /// The kernel command line (`/chosen/bootargs`). Empty, the default,
    /// leaves the property out as QEMU does; the device tree's `stdout-path`
    /// still gives the kernel the UART as its console.
    /// [`boot::SERIAL_CONSOLE_COMMAND_LINE`] adds early messages.
    pub fn command_line(mut self, command_line: &'i str) -> Self {
        self.boot.command_line = command_line;
        self
    }
}

/// The RAM and variable store a caller lends [`MachineBuilder::build_in`].
///
/// A build that is refused leaves both as they were: everything that can
/// fail is checked before either is written.
pub struct MemoryLoan<'m> {
    /// Guest RAM: exactly the size the builder was configured with. The
    /// builder zeroes it, so a guest never reads what the caller left there.
    pub ram: &'m mut [u8],
    /// The variable store's array, no larger than its 64 MiB flash bank,
    /// used as it is unless the builder was given
    /// [`MachineBuilder::variables`]. Empty is allowed: the bank then reads
    /// as erased flash and ignores programming.
    pub vars: &'m mut [u8],
}

/// How the guest starts, with the builder's type erased into data.
#[derive(Clone, Copy)]
enum BootChoice<'i> {
    Firmware(Firmware<'i>),
    Kernel(DirectKernel<'i>),
    Bare,
}

/// How the machine starts again after a reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootRecord {
    Firmware,
    Kernel { layout: KernelLayout },
    Bare,
}

impl BootRecord {
    fn tag(self) -> u8 {
        match self {
            Self::Firmware => 0,
            Self::Kernel { .. } => 1,
            Self::Bare => 2,
        }
    }
}

macro_rules! build_methods {
    ($boot:ty, $choice:expr) => {
        #[cfg(feature = "alloc")]
        impl<'i, I: Instrumentation> MachineBuilder<'i, $boot, I> {
            /// Build a machine that owns its memory.
            ///
            /// # Errors
            ///
            /// [`BuildError`] when an image does not fit where it has to go.
            pub fn build(self) -> Result<VirtMachine<OwnedStorage, I>, BuildError> {
                let choice = ($choice)(self.boot);
                let plan = plan(self.device_tree, choice, self.ram)?;
                let storage = owned_storage(self.ram, choice)?;
                Ok(assemble(self.config, plan, storage, self.observer, self.plugs))
            }
        }

        impl<'i, I: Instrumentation> MachineBuilder<'i, $boot, I> {
            /// Build a machine that runs in memory the caller lends it,
            /// borrowing the firmware and kernel images where they are.
            ///
            /// # Errors
            ///
            /// [`BuildError`] when the loan is not the configured size or an
            /// image does not fit where it has to go; the loan is untouched.
            pub fn build_in<'m>(self, loan: MemoryLoan<'m>) -> Result<VirtMachine<LentStorage<'m>, I>, BuildError>
            where
                'i: 'm,
            {
                let choice = ($choice)(self.boot);
                let plan = plan(self.device_tree, choice, self.ram)?;
                let storage = lent_storage(self.ram, choice, loan)?;
                Ok(assemble(self.config, plan, storage, self.observer, self.plugs))
            }
        }
    };
}

build_methods!(Firmware<'i>, BootChoice::Firmware);
build_methods!(DirectKernel<'i>, BootChoice::Kernel);
build_methods!(Bare, |_: Bare| BootChoice::Bare);

/// The images a boot keeps for a reset.
fn kernel_images<'i>(choice: BootChoice<'i>) -> (&'i [u8], &'i [u8]) {
    match choice {
        BootChoice::Kernel(k) => (k.image.bytes(), k.initrd),
        BootChoice::Firmware(_) | BootChoice::Bare => (&[], &[]),
    }
}

#[cfg(feature = "alloc")]
fn owned_storage(ram: RamSize, choice: BootChoice<'_>) -> Result<OwnedStorage, BuildError> {
    let (code, vars) = match choice {
        BootChoice::Firmware(fw) => {
            if fw.code.len() as u64 > FLASH_BANK_SIZE {
                return Err(BuildError::FirmwareTooLarge);
            }
            let mut vars = vec![0xFFu8; FLASH_BANK_SIZE as usize].into_boxed_slice();
            if let Some(image) = fw.variables {
                vars.get_mut(..image.len()).ok_or(BuildError::VariablesTooLarge)?.copy_from_slice(image);
            }
            (Box::from(fw.code), vars)
        }
        BootChoice::Kernel(_) | BootChoice::Bare => (Box::default(), Box::default()),
    };
    let (kernel, initrd) = kernel_images(choice);
    Ok(OwnedStorage {
        ram: vec![0u8; ram.get() as usize].into_boxed_slice(),
        code,
        vars,
        kernel: Box::from(kernel),
        initrd: Box::from(initrd),
    })
}

fn lent_storage<'m>(ram: RamSize, choice: BootChoice<'m>, loan: MemoryLoan<'m>) -> Result<LentStorage<'m>, BuildError> {
    if loan.ram.len() as u64 != ram.get() {
        return Err(BuildError::LentRamSize {
            lent: loan.ram.len() as u64,
            configured: ram.get(),
        });
    }
    // Flash bank 1 decodes the lent array whatever the boot, so it is held
    // to the bank's size whatever the boot.
    if loan.vars.len() as u64 > FLASH_BANK_SIZE {
        return Err(BuildError::VariablesTooLarge);
    }
    let (code, variables): (&'m [u8], &'m [u8]) = match choice {
        BootChoice::Firmware(fw) => {
            if fw.code.len() as u64 > FLASH_BANK_SIZE {
                return Err(BuildError::FirmwareTooLarge);
            }
            (fw.code, fw.variables.unwrap_or_default())
        }
        BootChoice::Kernel(_) | BootChoice::Bare => (&[], &[]),
    };
    let initial_vars = loan.vars.get_mut(..variables.len()).ok_or(BuildError::VariablesTooLarge)?;
    // Every check has passed: only now is the loan written.
    initial_vars.copy_from_slice(variables);
    loan.ram.fill(0);
    let (kernel, initrd) = kernel_images(choice);
    Ok(LentStorage {
        ram: loan.ram,
        code,
        vars: loan.vars,
        kernel,
        initrd,
    })
}

/// The edited device tree a machine keeps, so a reset can place it again.
struct DeviceTree {
    bytes: [u8; DEVICE_TREE_CAPACITY],
    len: usize,
}

impl DeviceTree {
    const EMPTY: Self = Self {
        bytes: [0; DEVICE_TREE_CAPACITY],
        len: 0,
    };

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Edit `source` to describe a machine with `ram_len` bytes of RAM booting
/// `choice`, into `out`.
fn describe(source: &[u8], ram_len: u64, choice: BootChoice<'_>, layout: Option<KernelLayout>, out: &mut DeviceTree) -> Result<(), DeviceTreeError> {
    let mut reg = [0u8; 16];
    reg[..8].copy_from_slice(&RAM_BASE.to_be_bytes());
    reg[8..].copy_from_slice(&ram_len.to_be_bytes());
    let initrd = match (choice, layout.and_then(|l| l.initrd_offset)) {
        (BootChoice::Kernel(k), Some(offset)) => {
            let start = RAM_BASE + offset;
            Some((start.to_be_bytes(), (start + k.initrd.len() as u64).to_be_bytes()))
        }
        (BootChoice::Kernel(_), None) | (BootChoice::Firmware(_) | BootChoice::Bare, _) => None,
    };
    let mut edits = [Edit {
        node: "memory@40000000",
        property: "device_type",
        value: Value::String("memory"),
    }; 5];
    edits[1] = Edit {
        node: "memory@40000000",
        property: "reg",
        value: Value::Bytes(&reg),
    };
    let mut count = 2;
    if let BootChoice::Kernel(k) = choice {
        if !k.command_line.is_empty() {
            edits[count] = Edit {
                node: "chosen",
                property: "bootargs",
                value: Value::String(k.command_line),
            };
            count += 1;
        }
    }
    if let Some((start, end)) = &initrd {
        edits[count] = Edit {
            node: "chosen",
            property: "linux,initrd-start",
            value: Value::Bytes(start),
        };
        edits[count + 1] = Edit {
            node: "chosen",
            property: "linux,initrd-end",
            value: Value::Bytes(end),
        };
        count += 2;
    }
    out.len = fdt::rebuild(source, &edits[..count], &mut out.bytes)?;
    Ok(())
}

/// What a build works out before it touches any memory: where the boot's
/// pieces go and the device tree that describes them.
struct Plan {
    boot: BootRecord,
    device_tree: DeviceTree,
}

/// Place the boot's images in `ram` and describe the machine, refusing what
/// does not fit. Nothing is allocated or written yet, so a refusal leaves a
/// caller's loan untouched.
fn plan(source_tree: &[u8], choice: BootChoice<'_>, ram: RamSize) -> Result<Plan, BuildError> {
    let ram_len = ram.get();
    let boot = match choice {
        BootChoice::Firmware(_) => BootRecord::Firmware,
        BootChoice::Kernel(k) => BootRecord::Kernel {
            layout: boot::layout(ram_len, &k.image, k.initrd.len() as u64)?,
        },
        BootChoice::Bare => BootRecord::Bare,
    };
    let mut device_tree = DeviceTree::EMPTY;
    let layout = match boot {
        BootRecord::Kernel { layout } => Some(layout),
        BootRecord::Firmware | BootRecord::Bare => None,
    };
    if !matches!(boot, BootRecord::Bare) {
        describe(source_tree, ram_len, choice, layout, &mut device_tree)?;
        let tree_len = device_tree.len as u64;
        match layout {
            Some(layout) => layout.fit_device_tree(ram_len, tree_len)?,
            None if tree_len > ram_len => return Err(BuildError::DeviceTree(DeviceTreeError::TooLarge)),
            None => {}
        }
    }
    Ok(Plan { boot, device_tree })
}

/// The machine `plan` describes, in `storage`, started.
fn assemble<S: GuestStorage, I: Instrumentation>(config: VirtConfig, plan: Plan, storage: S, observer: I, plugs: Plugs) -> VirtMachine<S, I> {
    let Plan { boot, device_tree } = plan;
    let mut cpu = Cpu::new(CpuConfig {
        ips: config.ips,
        counter_hz: config.counter_hz,
        reset_pc: FLASH0_BASE,
    });
    cpu.set_debug_stops(config.debug_stops);
    #[cfg(feature = "alloc")]
    cpu.cover_code_memory(storage.ram().len() as u64);
    let devices = plugs.devices;
    let mut machine = VirtMachine {
        cpu,
        board: VirtBoard::new(storage, config.rtc_epoch_seconds, plugs),
        observer,
        config,
        boot,
        devices,
        device_tree,
        next_event: 0,
        power: PowerState::Running,
        terminal: TerminalKeyboard::new(),
        exits: Exits::EMPTY,
        stop: StopSource::None,
    };
    machine.start();
    machine
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

/// Why a run returned.
///
/// Closed (R5): each variant names one way a run ends.
/// [`BatchOutcome::is_terminal`] is there for a caller that only needs to
/// know whether stepping again makes progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopReason {
    /// The budget was spent; running again continues.
    BudgetExhausted,
    /// The processor waits for an interrupt and nothing is scheduled to
    /// raise one. With a tick budget, time advanced to the end of it; only
    /// the host (serial input, the power button) can wake the guest now.
    Halted,
    /// A stop handle or stop flag was raised. It stays raised: every run
    /// returns this until the host lowers it.
    StopRequested,
    /// The program counter reached an exit address ([`VirtMachine::set_exits`]
    /// or [`VirtMachine::emu_start`]'s `until`). Nothing at it has executed.
    ExitReached,
    /// PSCI `SYSTEM_OFF`: the guest switched the machine off.
    PoweredOff,
    /// PSCI `SYSTEM_RESET`, with [`OnResetRequest::Stop`]: the guest asked
    /// for a reset and the host decides ([`Power::reset`]).
    ResetRequested,
    /// PSCI `CPU_OFF` on the only processor: nothing can run again until a
    /// reset.
    ProcessorOff,
    /// An UNDEFINED instruction, with `DebugStops::on_undefined` set.
    Undefined(UndefinedInsn),
    /// An access to nothing, with `DebugStops::on_bus_fault` set.
    BusFault { pc: u64, addr: u64 },
}

/// What one run did.
///
/// A named struct rather than a tuple (R0): one field says how far the run
/// got, the other why it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use = "the stop reason says whether stepping again makes progress"]
pub struct BatchOutcome {
    /// Instructions retired. Time a waiting processor skipped is not in it;
    /// [`VirtMachine::ticks`] tells time.
    pub progress: Progress,
    pub stop: StopReason,
}

impl BatchOutcome {
    /// Whether the machine should not be stepped again without host action.
    ///
    /// `BudgetExhausted` and `Halted` are ordinary yields. `ExitReached`
    /// ends the run a caller asked for. The rest mean execution cannot
    /// continue as it is: the guest switched off or asked for a reset, the
    /// host asked to stop, or the processor met something it will meet
    /// again.
    pub fn is_terminal(&self) -> bool {
        match self.stop {
            StopReason::BudgetExhausted | StopReason::Halted => false,
            StopReason::StopRequested
            | StopReason::ExitReached
            | StopReason::PoweredOff
            | StopReason::ResetRequested
            | StopReason::ProcessorOff
            | StopReason::Undefined(_)
            | StopReason::BusFault { .. } => true,
        }
    }
}

/// What state a machine's power is in, asked at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerState {
    /// Executing, or waiting for an interrupt — either way, alive.
    Running,
    /// The guest called PSCI `SYSTEM_OFF`.
    PoweredOff,
    /// The guest called PSCI `CPU_OFF` on the only processor.
    ProcessorOff,
}

/// How often a run looks at its stop flag, in ticks: often enough that a
/// stop lands within a millisecond of host time, rarely enough that the
/// look is free.
const STOP_POLL_TICKS: u64 = 1 << 16;

// A slice bounds the run of a burst (`Cpu::bound_run`), which is shorter
// than 2^32 instructions.
const _: () = assert!(STOP_POLL_TICKS < 1 << 32);

/// The most exit addresses a machine watches at once.
pub const MAX_EXITS: usize = 16;

/// More exit addresses than a machine watches at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TooManyExits {
    pub requested: usize,
}

impl core::fmt::Display for TooManyExits {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} exit addresses requested; a machine watches at most {MAX_EXITS}", self.requested)
    }
}

impl core::error::Error for TooManyExits {}

#[derive(Clone, Copy)]
struct Exits {
    addrs: [u64; MAX_EXITS + 1],
    len: usize,
}

impl Exits {
    const EMPTY: Self = Self {
        addrs: [0; MAX_EXITS + 1],
        len: 0,
    };

    #[inline]
    fn contains(&self, pc: u64) -> bool {
        self.addrs[..self.len].contains(&pc)
    }

    /// The exit addresses.
    #[inline]
    fn addresses(&self) -> &[u64] {
        &self.addrs[..self.len]
    }

    /// How many instructions, at most `limit`, run in sequence from `pc`
    /// before one at an exit address.
    #[inline]
    fn instructions_before(&self, pc: u64, limit: u64) -> u64 {
        self.addrs[..self.len]
            .iter()
            .filter(|&&exit| exit > pc && (exit - pc).is_multiple_of(4))
            .map(|&exit| (exit - pc) / 4)
            .fold(limit, u64::min)
    }
}

/// Where a machine's stop flag comes from.
enum StopSource {
    None,
    #[cfg(feature = "alloc")]
    Shared(Arc<AtomicBool>),
    Static(&'static AtomicBool),
}

impl StopSource {
    #[inline]
    fn raised(&self) -> bool {
        match self {
            Self::None => false,
            #[cfg(feature = "alloc")]
            Self::Shared(flag) => flag.load(Ordering::Relaxed),
            Self::Static(flag) => flag.load(Ordering::Relaxed),
        }
    }
}

/// A clonable handle that stops a machine from another thread.
///
/// Raising it ends the running batch within 65,536 ticks with
/// [`StopReason::StopRequested`], and every batch after, until it is lowered.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct StopHandle(Arc<AtomicBool>);

#[cfg(feature = "alloc")]
impl StopHandle {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Lower the flag, so the machine runs again.
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    pub fn is_stopping(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// What bringing the machine up to date after a device access did to its
/// next event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NextEvent {
    /// A timer register write or a device moved a deadline, and the next
    /// event was worked out again.
    WorkedOutAgain,
    /// It is where it was.
    Unchanged,
}

/// How a slice of the run loop ended.
enum Slice {
    /// It reached its end tick; the run decides whether that was the budget.
    Done,
    Stopped(StopReason),
}

/// The tick a processor waiting for an interrupt at tick `now` sleeps to:
/// the next event, or `slice_end` if that comes first. Time always moves:
/// a next event at or before `now` — a deadline a device reported that
/// working the events out again (`VirtMachine::sync_events`) did not
/// consume — wakes it at the next tick, so the slice still reaches its end
/// and no later event is passed over.
#[inline]
fn wake_tick(next_event: u64, now: u64, slice_end: u64) -> u64 {
    next_event.max(now.saturating_add(1)).min(slice_end)
}

/// A Cortex-A57-shaped processor on QEMU's `virt` board.
///
/// `S` is where its memory lives (`OwnedStorage`, with `alloc`, or
/// [`LentStorage`]); `I`
/// watches it run ([`NoInstrumentation`] unless the builder was given an
/// observer).
//
// The declared order is the layout (`repr(C)`), chosen for `run_until`,
// whose register allocation follows it (README "Speed"). The board is
// last, so that no device's size moves another field. Only fixed-size
// records come before the CPU, and neither the CPU nor the board is at
// offset 0: a field there shares the machine's address, and the loop's
// registers are allocated differently.
#[repr(C)]
pub struct VirtMachine<S, I = NoInstrumentation> {
    stop: StopSource,
    boot: BootRecord,
    cpu: Cpu,
    /// The virtio devices it was built with.
    devices: VirtioDevices,
    config: VirtConfig,
    exits: Exits,
    terminal: TerminalKeyboard,
    device_tree: DeviceTree,
    /// The next tick at which a timer or device output changes by itself.
    next_event: u64,
    power: PowerState,
    observer: I,
    board: VirtBoard<S>,
}

/// Copy `src` into `ram` at `offset`. The builder checked every placement
/// against the RAM it made, so this cannot run past the end; were it ever
/// to, nothing is written.
fn place(ram: &mut [u8], offset: u64, src: &[u8]) {
    let start = offset as usize;
    if let Some(dst) = start.checked_add(src.len()).and_then(|end| ram.get_mut(start..end)) {
        dst.copy_from_slice(src);
    }
}

/// Guest RAM as a bus with nothing else on it, for walks a host makes: a
/// descriptor outside RAM reads as an external abort rather than as a
/// device register read that would change the device.
struct RamView<'a>(&'a [u8]);

impl PhysBus for RamView<'_> {
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _clock: rusty_box_core::VmClock) -> Result<u64, BusFault> {
        let off = addr.get().checked_sub(RAM_BASE).ok_or(BusFault::Unmapped)? as usize;
        let end = off.checked_add(width.bytes() as usize).ok_or(BusFault::Unmapped)?;
        self.0.get(off..end).map(le_value).ok_or(BusFault::Unmapped)
    }

    fn write(&mut self, _addr: PhysAddr, _width: AccessWidth, _value: u64, _clock: rusty_box_core::VmClock) -> Result<(), BusFault> {
        Err(BusFault::Unmapped)
    }
}

/// Why a host memory access failed.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryError {
    /// The physical address is not in RAM. Device windows are reached by
    /// running the guest, not by a host access that would change them.
    NotRam { addr: u64 },
    /// The virtual address has no translation.
    Translation { va: u64, fault: TranslationFault },
}

impl core::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotRam { addr } => write!(f, "{addr:#x} is not in RAM"),
            Self::Translation { va, fault } => write!(f, "{va:#x} does not translate: {fault}"),
        }
    }
}

impl core::error::Error for MemoryError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Translation { fault, .. } => Some(fault),
            Self::NotRam { .. } => None,
        }
    }
}

impl<S: GuestStorage, I: Instrumentation> VirtMachine<S, I> {
    /// Put the guest at its first instruction: the boot images in RAM, the
    /// processor at its entry state, the devices reset. Memory a boot does
    /// not load is left as it is.
    fn start(&mut self) {
        let reset_pc = match self.boot {
            BootRecord::Firmware => FLASH0_BASE,
            BootRecord::Kernel { layout } => RAM_BASE + layout.kernel_offset,
            BootRecord::Bare => RAM_BASE,
        };
        self.cpu.reset_keeping_time(reset_pc);
        self.board.reset_devices(self.cpu.clock());
        let parts = self.board.storage.parts();
        match self.boot {
            // Firmware finds the tree at the base of RAM, as on QEMU.
            BootRecord::Firmware => place(parts.ram, 0, self.device_tree.as_slice()),
            BootRecord::Kernel { layout } => {
                place(parts.ram, layout.device_tree_offset, self.device_tree.as_slice());
                place(parts.ram, layout.kernel_offset, parts.kernel);
                if let Some(offset) = layout.initrd_offset {
                    place(parts.ram, offset, parts.initrd);
                }
                // What QEMU's bootloader_aarch64 leaves: x0 the device tree,
                // x4 the entry it branched through, x1-x3 zero from reset.
                self.cpu.set_reg(Reg::X0, RAM_BASE + layout.device_tree_offset);
                self.cpu.set_reg(Reg::X4, reset_pc);
                // And the console set up, as firmware would leave it.
                self.board.set_up_console(boot::DIRECT_BOOT_CONSOLE_BAUD, self.cpu.clock());
            }
            BootRecord::Bare => {}
        }
        self.power = PowerState::Running;
        self.sync_events();
    }

    /// Re-evaluate everything that changes with time or with a timer
    /// register: the timer PPIs, the time-driven devices, the processor's
    /// interrupt inputs, and the next tick at which any of them changes.
    fn sync_events(&mut self) {
        self.cpu.take_timer_dirty();
        let lines = self.cpu.timer_lines();
        self.board.set_timer_lines(lines.physical, lines.virtual_);
        let device_deadline = self.board.advance_devices(self.cpu.clock()).map(|t| t.ticks());
        let timer_deadline = self.cpu.next_timer_deadline();
        self.next_event = match (timer_deadline, device_deadline) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => u64::MAX,
        };
        self.cpu.set_interrupt_lines(self.board.gic.irq_out(), self.board.gic.fiq_out());
    }

    /// Run until `budget` is spent or something stops the machine.
    ///
    /// An instruction budget retires exactly that many instructions, across
    /// calls as within one. A tick budget advances guest time exactly, which
    /// is what a guest that waits for interrupts needs: a waiting processor
    /// retires nothing while its timers keep running.
    pub fn step(&mut self, budget: RunBudget) -> BatchOutcome {
        self.run::<false>(budget)
    }

    /// Unicorn's `uc_emu_start`: run from `begin` until the program counter
    /// reaches `until` or an exit address, or `count` instructions retire.
    /// `None` leaves that bound off. A processor waiting for an interrupt is
    /// woken to run from `begin`.
    pub fn emu_start(&mut self, begin: u64, until: Option<u64>, count: Option<u64>) -> BatchOutcome {
        self.cpu.set_reg(Reg::Pc, begin);
        self.cpu.activity = Activity::Running;
        let configured = self.exits;
        if let Some(addr) = until {
            self.exits.addrs[self.exits.len] = addr;
            self.exits.len += 1;
        }
        let outcome = self.run::<true>(RunBudget::Instructions(count.unwrap_or(u64::MAX)));
        self.exits = configured;
        outcome
    }

    /// End every run when the program counter reaches one of `exits`, before
    /// the instruction there executes. An empty list watches nothing.
    ///
    /// # Errors
    ///
    /// [`TooManyExits`] past [`MAX_EXITS`] addresses; the list is unchanged.
    pub fn set_exits(&mut self, exits: &[u64]) -> Result<(), TooManyExits> {
        if exits.len() > MAX_EXITS {
            return Err(TooManyExits { requested: exits.len() });
        }
        self.exits.addrs[..exits.len()].copy_from_slice(exits);
        self.exits.len = exits.len();
        Ok(())
    }

    fn run<const WATCH: bool>(&mut self, budget: RunBudget) -> BatchOutcome {
        let started = self.cpu.retired;
        let stop = self.run_until::<WATCH>(budget);
        BatchOutcome {
            progress: Progress::Instructions(self.cpu.retired - started),
            stop,
        }
    }

    fn run_until<const WATCH: bool>(&mut self, budget: RunBudget) -> StopReason {
        match self.power {
            PowerState::Running => {}
            PowerState::PoweredOff => return StopReason::PoweredOff,
            PowerState::ProcessorOff => return StopReason::ProcessorOff,
        }
        // Host actions since the last run (serial input, the power button, a
        // register write) may have changed what the processor sees. No burst
        // is counted yet, so the run starts from the next event as it now
        // is either way.
        match self.after_device_access() {
            NextEvent::WorkedOutAgain | NextEvent::Unchanged => {}
        }
        let (tick_end, instruction_end) = match budget {
            RunBudget::Ticks(n) => (self.cpu.icount.saturating_add(n), u64::MAX),
            RunBudget::Instructions(n) => (u64::MAX, self.cpu.retired.saturating_add(n)),
        };
        // Watch every instruction for an exit only when there are exits.
        let watch = WATCH || self.exits.len > 0;
        loop {
            if self.stop.raised() {
                return StopReason::StopRequested;
            }
            let slice_end = tick_end.min(self.cpu.icount.saturating_add(STOP_POLL_TICKS));
            let slice = if watch {
                self.run_slice::<true>(slice_end, tick_end, instruction_end)
            } else {
                self.run_slice::<false>(slice_end, tick_end, instruction_end)
            };
            match slice {
                Slice::Done if self.cpu.icount >= tick_end => return StopReason::BudgetExhausted,
                Slice::Done => {}
                Slice::Stopped(reason) => return reason,
            }
        }
    }

    /// The interpreter loop, to tick `slice_end` at most.
    ///
    /// The checks between two instructions are split by how often their
    /// answers can change. The time, budget and wait checks run once per
    /// burst: every instruction retires exactly one tick, so they cannot
    /// change until the burst reaches the nearest of the next event, the
    /// slice end and the instruction budget, or an instruction moves the
    /// next event or puts the processor to sleep, which ends the burst.
    ///
    /// Within a burst the processor runs traces, some of them linked: run
    /// on from the trace before without coming back here, with the checks
    /// made here between them made there (`Cpu::linked_trace`). Interrupt
    /// delivery and the exit check happen before each; within one, neither
    /// can change: the
    /// interrupt lines are re-driven only after an instruction that reached
    /// a device or wrote a timer register, which ends the trace, and the
    /// masks change only by instructions that end a trace. A trace runs no
    /// further than what is left of the burst and the next exit address, so
    /// it stops where the per-instruction checks would have — as QEMU's
    /// icount, when fewer instructions are left than a translation block
    /// holds, runs a block of exactly that many (accel/tcg/cpu-exec.c
    /// `cpu_loop_exec_tb`, with `CF_COUNT_MASK`).
    #[inline(always)]
    fn run_slice<const WATCH: bool>(&mut self, slice_end: u64, tick_end: u64, instruction_end: u64) -> Slice {
        'slice: loop {
            if self.cpu.icount >= self.next_event {
                self.sync_events();
            }
            if self.cpu.activity == Activity::WaitingForInterrupt && !self.cpu.wake_if_pending() {
                if self.next_event == u64::MAX {
                    // Nothing will ever wake it but the host: let a tick
                    // budget's time pass at once, as it would idly.
                    if tick_end != u64::MAX {
                        self.cpu.wait_until(VmInstant::from_ticks(tick_end));
                    }
                    return Slice::Stopped(StopReason::Halted);
                }
                let until = wake_tick(self.next_event, self.cpu.icount, slice_end);
                self.cpu.wait_until(VmInstant::from_ticks(until));
                if until == slice_end {
                    return Slice::Done;
                }
                continue;
            }
            if self.cpu.icount >= slice_end {
                return Slice::Done;
            }
            if self.cpu.retired >= instruction_end {
                return Slice::Stopped(StopReason::BudgetExhausted);
            }
            // At least one instruction: a next event already due is worked
            // out again before every instruction until it is in the future.
            let burst = self
                .next_event
                .saturating_sub(self.cpu.icount)
                .max(1)
                .min(slice_end - self.cpu.icount)
                .min(instruction_end - self.cpu.retired);
            let mut left = burst;
            // A burst is at most a slice, fewer than 2^32 instructions.
            self.cpu.bound_run(burst);
            while left > 0 {
                if self.cpu.interrupt_deliverable() {
                    if let Some(taken) = self.cpu.deliver_interrupt() {
                        self.observer.exception(taken);
                    }
                }
                let pc = self.cpu.pc();
                let limit = if WATCH {
                    if self.exits.contains(pc) {
                        return Slice::Stopped(StopReason::ExitReached);
                    }
                    self.exits.instructions_before(pc, left)
                } else {
                    left
                };
                let retired = self.cpu.retired;
                // A counter poll may be fast-forwarded through the rest of
                // the burst, but past no exit address.
                let exits = if WATCH { self.exits.addresses() } else { &[] };
                match self
                    .cpu
                    .run_trace(&mut self.board, &mut self.observer, limit, exits)
                {
                    Step::Retired => {}
                    Step::Waiting => {
                        // The slice starts again either way, and a waiting
                        // processor sleeps to the next event as it now is.
                        match self.after_device_access() {
                            NextEvent::WorkedOutAgain | NextEvent::Unchanged => continue 'slice,
                        }
                    }
                    Step::Stop(cause) => match self.stopped(cause) {
                        Some(reason) => return Slice::Stopped(reason),
                        None => continue 'slice,
                    },
                }
                left -= self.cpu.retired - retired;
                // A burst was counted to the next event: one worked out
                // again ends it.
                if (self.cpu.timer_dirty || self.board.touched) && self.after_device_access() == NextEvent::WorkedOutAgain {
                    continue 'slice;
                }
            }
        }
    }

    /// Bring the processor's interrupt inputs and the next event up to date
    /// after an instruction that may have changed them.
    #[inline]
    fn after_device_access(&mut self) -> NextEvent {
        self.board.touched = false;
        // What a device wrote may be code the processor decoded.
        let written = self.board.dma.take();
        for &span in written.spans() {
            self.cpu.forget_code(span);
        }
        if self.events_moved() {
            self.sync_events();
            NextEvent::WorkedOutAgain
        } else {
            self.cpu.set_interrupt_lines(self.board.gic.irq_out(), self.board.gic.fiq_out());
            NextEvent::Unchanged
        }
    }

    /// Whether a timer register write or a device access has moved the next
    /// event, so it has to be worked out again.
    #[inline(always)]
    fn events_moved(&self) -> bool {
        self.cpu.timer_dirty || self.board.deadline_moved
    }

    /// What the machine does about a step that stopped. `None` means it
    /// dealt with it and the run carries on.
    fn stopped(&mut self, cause: StopCause) -> Option<StopReason> {
        Some(match cause {
            StopCause::PowerOff => {
                self.power = PowerState::PoweredOff;
                StopReason::PoweredOff
            }
            StopCause::CpuOff => {
                self.power = PowerState::ProcessorOff;
                StopReason::ProcessorOff
            }
            StopCause::Reset => match self.config.on_reset_request {
                OnResetRequest::Reset => {
                    self.start();
                    return None;
                }
                OnResetRequest::Stop => StopReason::ResetRequested,
            },
            StopCause::Undefined(i) => StopReason::Undefined(i),
            StopCause::BusFault { pc, addr } => StopReason::BusFault { pc, addr },
        })
    }

    // ----- time -----

    /// Guest time since the machine was built, in ticks.
    pub fn ticks(&self) -> u64 {
        self.cpu.icount
    }

    /// Guest time since the machine was built.
    pub fn now(&self) -> VmInstant {
        self.cpu.now()
    }

    /// Instructions retired since the machine was built.
    pub fn instructions(&self) -> u64 {
        self.cpu.instructions()
    }

    /// How many ticks until a timer or device changes an output by itself —
    /// how far a host may skip a waiting machine with
    /// [`RunBudget::Ticks`]. `None` when nothing is scheduled.
    pub fn ticks_to_next_timer_deadline(&self) -> Option<u64> {
        let now = self.cpu.icount;
        let timer = self.cpu.next_timer_deadline();
        let device = self.board.next_deadline().map(|t| t.ticks());
        let next = match (timer, device) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => return None,
        };
        Some(next.saturating_sub(now))
    }

    // ----- stopping -----

    /// A handle that stops this machine from another thread. Every call
    /// hands out a handle to the same flag.
    #[cfg(feature = "alloc")]
    pub fn stop_handle(&mut self) -> StopHandle {
        match &self.stop {
            StopSource::Shared(flag) => StopHandle(Arc::clone(flag)),
            StopSource::None | StopSource::Static(_) => {
                let flag = Arc::new(AtomicBool::new(false));
                self.stop = StopSource::Shared(Arc::clone(&flag));
                StopHandle(flag)
            }
        }
    }

    /// Stop on a flag the host owns for ever — the form a host without an
    /// allocator has. Replaces any stop handle.
    pub fn set_stop_flag(&mut self, flag: &'static AtomicBool) {
        self.stop = StopSource::Static(flag);
    }

    // ----- devices -----

    /// The UART `port` (0 is the only one `virt` has). `None` for any other.
    pub fn serial(&mut self, port: usize) -> Option<Serial<'_, S>> {
        (port == 0).then_some(Serial {
            board: &mut self.board,
            clock: self.cpu.clock(),
        })
    }

    /// The machine's keyboard: the virtio keyboard if the machine was built
    /// with one, else the keyboard of the terminal on the UART.
    pub fn keyboard(&mut self) -> Keyboard<'_, S> {
        let target = if self.board.has_input(InputKind::Keyboard) {
            KeyboardTarget::Virtio
        } else {
            KeyboardTarget::Terminal
        };
        Keyboard {
            board: &mut self.board,
            terminal: &mut self.terminal,
            target,
            clock: self.cpu.clock(),
        }
    }

    /// The keyboard of the terminal on the UART, whatever else is plugged
    /// in: key events become the bytes an xterm sends for them.
    pub fn terminal_keyboard(&mut self) -> Keyboard<'_, S> {
        Keyboard {
            board: &mut self.board,
            terminal: &mut self.terminal,
            target: KeyboardTarget::Terminal,
            clock: self.cpu.clock(),
        }
    }

    /// The virtio tablet, if the machine was built with one.
    pub fn pointer(&mut self) -> Option<Pointer<'_, S>> {
        if self.board.has_input(InputKind::Tablet) {
            Some(Pointer { board: &mut self.board })
        } else {
            None
        }
    }

    /// The virtio-gpu's display, if the machine was built with one.
    #[cfg(feature = "alloc")]
    pub fn display(&mut self) -> Option<Display<'_, S>> {
        if self.devices.display.is_some() {
            Some(Display { board: &mut self.board })
        } else {
            None
        }
    }

    /// The power controls.
    pub fn power(&mut self) -> Power<'_, S, I> {
        Power { machine: self }
    }

    /// The observer watching the guest.
    pub fn instrumentation(&self) -> &I {
        &self.observer
    }

    pub fn instrumentation_mut(&mut self) -> &mut I {
        &mut self.observer
    }

    // ----- memory -----

    /// Read guest-physical RAM.
    ///
    /// # Errors
    ///
    /// [`MemoryError::NotRam`] when any byte is outside RAM; nothing is read.
    pub fn mem_read(&self, addr: u64, buf: &mut [u8]) -> Result<(), MemoryError> {
        let src = ram_range(self.board.storage.ram(), addr, buf.len())?;
        buf.copy_from_slice(src);
        Ok(())
    }

    /// Write guest-physical RAM.
    ///
    /// # Errors
    ///
    /// [`MemoryError::NotRam`] when any byte is outside RAM; nothing is
    /// written.
    pub fn mem_write(&mut self, addr: u64, data: &[u8]) -> Result<(), MemoryError> {
        let ram = self.board.storage.ram_mut();
        let off = ram_offset(ram.len(), addr, data.len())?;
        ram[off..off + data.len()].copy_from_slice(data);
        if let Some(span) = PhysSpan::new(PhysAddr::new(addr), data.len() as u64) {
            self.cpu.forget_code(span);
        }
        Ok(())
    }

    /// Where data address `va` translates to under the current tables: a
    /// walk that checks no permission, reads descriptors from RAM only, and
    /// neither reads nor fills the TLB. A mapping the guest rewrote without
    /// a `TLBI` shows its new translation here while the guest's own
    /// accesses may still use the cached one.
    ///
    /// # Errors
    ///
    /// [`TranslationFault`] when it has no translation.
    pub fn virt_to_phys(&self, va: u64) -> Result<u64, TranslationFault> {
        self.cpu.debug_translate(&mut RamView(self.board.storage.ram()), va)
    }

    /// Read memory through the guest's page tables, one page at a time.
    ///
    /// # Errors
    ///
    /// The first address that does not translate or is not RAM; `buf` may
    /// hold the pages before it.
    pub fn virt_read(&self, va: u64, buf: &mut [u8]) -> Result<(), MemoryError> {
        let mut done = 0;
        while done < buf.len() {
            let at = va.wrapping_add(done as u64);
            let chunk = page_chunk(at, buf.len() - done);
            let pa = self.virt_to_phys(at).map_err(|fault| MemoryError::Translation { va: at, fault })?;
            self.mem_read(pa, &mut buf[done..done + chunk])?;
            done += chunk;
        }
        Ok(())
    }

    /// Write memory through the guest's page tables, one page at a time.
    /// Permissions are not checked: a host may write what the guest maps
    /// read-only.
    ///
    /// # Errors
    ///
    /// The first address that does not translate or is not RAM; the pages
    /// before it are written.
    pub fn virt_write(&mut self, va: u64, data: &[u8]) -> Result<(), MemoryError> {
        let mut done = 0;
        while done < data.len() {
            let at = va.wrapping_add(done as u64);
            let chunk = page_chunk(at, data.len() - done);
            let pa = self.virt_to_phys(at).map_err(|fault| MemoryError::Translation { va: at, fault })?;
            self.mem_write(pa, &data[done..done + chunk])?;
            done += chunk;
        }
        Ok(())
    }

    // ----- registers -----

    pub fn reg_read(&self, reg: Reg) -> u64 {
        self.cpu.reg(reg)
    }

    /// Write a register. `Nzcv`, `Daif`, `Fpcr` and `Fpsr` keep only the
    /// bits an `MSR` would.
    pub fn reg_write(&mut self, reg: Reg, value: u64) {
        self.cpu.set_reg(reg, value);
    }

    pub fn vreg_read(&self, reg: VReg) -> u128 {
        self.cpu.v(reg)
    }

    pub fn vreg_write(&mut self, reg: VReg, value: u128) {
        self.cpu.set_v(reg, value);
    }

    pub fn pstate(&self) -> Pstate {
        self.cpu.pstate()
    }

    /// Replace `PSTATE`: the exception level, the stack pointer selection,
    /// the masks and the flags. At EL0 `sp_sel` is cleared, since EL0 has
    /// only `SP_EL0`.
    pub fn set_pstate(&mut self, pstate: Pstate) {
        self.cpu.set_pstate(pstate);
    }

    /// Read a system register with EL1's privilege and none of the controls
    /// that trap a guest.
    ///
    /// # Errors
    ///
    /// [`SysRegRefusal`] for a register that does not exist or cannot be
    /// read.
    pub fn sysreg_read(&self, reg: SysReg) -> Result<u64, SysRegRefusal> {
        self.cpu.sysreg(reg)
    }

    /// Write a system register as an EL1 `MSR` would, effects included: a
    /// translation register flushes the TLB, a timer register re-evaluates
    /// the timer lines before the next instruction.
    ///
    /// # Errors
    ///
    /// [`SysRegRefusal`] for a register that does not exist or cannot be
    /// written; nothing changes.
    pub fn sysreg_write(&mut self, reg: SysReg, value: u64) -> Result<(), SysRegRefusal> {
        self.cpu.host_sysreg_write(reg, value).map(|_effect| ())
    }

    /// The architectural state now.
    pub fn cpu_snapshot(&self) -> CpuState {
        self.cpu.state()
    }

    /// Recently executed instruction addresses, oldest first: where the
    /// guest was before it stopped.
    pub fn recent_pcs(&self) -> crate::cpu::RecentPcs {
        self.cpu.recent_pcs()
    }

    // ----- storage -----

    /// The variable store's array, for a host that persists it.
    pub fn variable_store(&self) -> &[u8] {
        self.board.storage.vars()
    }

    /// Guest RAM, whole.
    pub fn ram(&self) -> &[u8] {
        self.board.storage.ram()
    }

    /// The device tree the machine placed in RAM at boot.
    pub fn device_tree(&self) -> &[u8] {
        self.device_tree.as_slice()
    }

    /// The disk behind the virtio-blk, if the machine was built with one
    /// ([`MachineBuilder::virtio_disk`]).
    #[cfg(feature = "alloc")]
    pub fn disk(&self) -> Option<&Disk> {
        self.board.block().map(|block| block.disk())
    }

    /// The disk, for a host that flushes it or takes its I/O errors.
    #[cfg(feature = "alloc")]
    pub fn disk_mut(&mut self) -> Option<&mut Disk> {
        self.board.block_mut().map(|block| block.disk_mut())
    }

    /// How many sectors the disk has, if there is one.
    fn disk_sectors(&self) -> Option<u64> {
        #[cfg(feature = "alloc")]
        if let Some(disk) = self.disk() {
            return Some(disk.sectors());
        }
        None
    }

    // ----- snapshots -----

    /// What a snapshot must match to be restored here.
    fn shape(&self) -> Shape {
        let storage = &self.board.storage;
        let images = storage.boot_images();
        let (kernel_offset, initrd_offset) = match self.boot {
            BootRecord::Kernel { layout } => (layout.kernel_offset, layout.initrd_offset.unwrap_or(u64::MAX)),
            BootRecord::Firmware | BootRecord::Bare => (0, 0),
        };
        Shape {
            ram_len: storage.ram().len() as u64,
            vars_len: storage.vars().len() as u64,
            code_hash: fnv1a(storage.code()),
            kernel_hash: fnv1a(images.kernel),
            initrd_hash: fnv1a(images.initrd),
            ips: self.config.ips.get(),
            counter_hz: self.config.counter_hz.get(),
            boot: self.boot.tag(),
            kernel_offset,
            initrd_offset,
            devices: self.devices.mask(),
            disk: self.disk_sectors(),
        }
    }

    /// Save the whole machine — processor, devices, RAM and the variable
    /// store — to `w`.
    ///
    /// Host-side state is not part of it: serial input not yet taken, the
    /// terminal keyboard's held modifiers, exit addresses, the stop flag and
    /// the observer stay with the machine they belong to. Firmware and
    /// kernel images are recorded by hash, not copied: a machine restores
    /// only an image taken from one built the same way. A disk's contents
    /// are not part of it either, only its size: like a QEMU migration over
    /// shared storage, a snapshot restores onto the disk as it is.
    ///
    /// # Errors
    ///
    /// Whatever `w` reports.
    pub fn save_snapshot<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_bytes(&SNAPSHOT_MAGIC)?;
        w.write_u32(SNAPSHOT_VERSION)?;
        self.shape().save(w)?;
        w.write_u8(match self.power {
            PowerState::Running => 0,
            PowerState::PoweredOff => 1,
            PowerState::ProcessorOff => 2,
        })?;
        w.write_u64(self.device_tree.len as u64)?;
        w.write_bytes(self.device_tree.as_slice())?;
        self.cpu.save(w)?;
        self.board.gic.save(w)?;
        self.board.uart.save(w)?;
        self.board.rtc.save(w)?;
        self.board.gpio.save(w)?;
        self.board.fw_cfg.save(w)?;
        self.board.flash0.save(w)?;
        self.board.flash1.save(w)?;
        for transport in self.board.virtio.iter().flatten() {
            transport.save(w)?;
        }
        match self.board.power_button_release {
            Some(at) => {
                w.write_bool(true)?;
                w.write_u64(at.ticks())?;
            }
            None => w.write_bool(false)?,
        }
        w.write_bytes(self.board.storage.ram())?;
        w.write_bytes(self.board.storage.vars())
    }

    /// Replace the whole machine with one [`VirtMachine::save_snapshot`]
    /// wrote.
    ///
    /// An image from a machine built differently — other RAM, other images,
    /// another boot method or clock — is refused before anything changes.
    /// Serial input waiting to be taken is dropped and the terminal's
    /// modifiers released: type again after restoring.
    ///
    /// # Errors
    ///
    /// [`SnapError`] for an image that is not one, was written by another
    /// version, or describes a different machine (nothing changes); or for
    /// one that is truncated or corrupt past its header, which leaves the
    /// machine switched off ([`PowerState::PoweredOff`]) until
    /// [`Power::reset`] starts it again.
    pub fn restore_snapshot<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let mut magic = [0u8; 8];
        r.read_bytes(&mut magic)?;
        if magic != SNAPSHOT_MAGIC {
            return Err(SnapError::Invalid("not a rusty_box AArch64 machine snapshot"));
        }
        if r.read_u32()? != SNAPSHOT_VERSION {
            return Err(SnapError::Invalid("snapshot written by another version of this machine"));
        }
        if Shape::restore(r)? != self.shape() {
            return Err(SnapError::Invalid("snapshot is of a machine built differently"));
        }
        match self.restore_body(r) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.power = PowerState::PoweredOff;
                Err(e)
            }
        }
    }

    fn restore_body<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let power = match r.read_u8()? {
            0 => PowerState::Running,
            1 => PowerState::PoweredOff,
            2 => PowerState::ProcessorOff,
            _ => return Err(SnapError::Invalid("snapshot power state is not one a machine has")),
        };
        let tree_len = r.read_len(DEVICE_TREE_CAPACITY)?;
        r.read_bytes(&mut self.device_tree.bytes[..tree_len])?;
        self.device_tree.len = tree_len;
        self.cpu.restore(r)?;
        self.board.gic.restore(r)?;
        self.board.uart.restore(r)?;
        self.board.rtc.restore(r)?;
        self.board.gpio.restore(r)?;
        self.board.fw_cfg.restore(r)?;
        self.board.flash0.restore(r)?;
        self.board.flash1.restore(r)?;
        for transport in self.board.virtio.iter_mut().flatten() {
            transport.restore(r)?;
        }
        self.board.power_button_release = if r.read_bool()? {
            Some(VmInstant::from_ticks(r.read_u64()?))
        } else {
            None
        };
        let parts = self.board.storage.parts();
        r.read_bytes(parts.ram)?;
        r.read_bytes(parts.vars)?;
        self.board.serial_input.clear();
        self.terminal = TerminalKeyboard::new();
        self.power = power;
        self.board.sync_device_lines();
        self.sync_events();
        Ok(())
    }
}

const SNAPSHOT_MAGIC: [u8; 8] = *b"RBA64SNP";
const SNAPSHOT_VERSION: u32 = 8;

/// Everything about how a machine was built that a snapshot depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shape {
    ram_len: u64,
    vars_len: u64,
    code_hash: u64,
    kernel_hash: u64,
    initrd_hash: u64,
    ips: u64,
    counter_hz: u64,
    boot: u8,
    kernel_offset: u64,
    initrd_offset: u64,
    /// Which display and input devices are plugged in
    /// ([`VirtioDevices::mask`]).
    devices: u8,
    /// The disk's size in sectors, if there is a disk.
    disk: Option<u64>,
}

impl Shape {
    fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        for v in [
            self.ram_len,
            self.vars_len,
            self.code_hash,
            self.kernel_hash,
            self.initrd_hash,
            self.ips,
            self.counter_hz,
            self.kernel_offset,
            self.initrd_offset,
        ] {
            w.write_u64(v)?;
        }
        w.write_u8(self.boot)?;
        w.write_u8(self.devices)?;
        w.write_bool(self.disk.is_some())?;
        w.write_u64(self.disk.unwrap_or(0))
    }

    fn restore<R: SnapRead>(r: &mut R) -> SnapResult<Self> {
        let mut v = [0u64; 9];
        for value in v.iter_mut() {
            *value = r.read_u64()?;
        }
        let [ram_len, vars_len, code_hash, kernel_hash, initrd_hash, ips, counter_hz, kernel_offset, initrd_offset] = v;
        Ok(Self {
            ram_len,
            vars_len,
            code_hash,
            kernel_hash,
            initrd_hash,
            ips,
            counter_hz,
            boot: r.read_u8()?,
            kernel_offset,
            initrd_offset,
            devices: r.read_u8()?,
            disk: {
                let present = r.read_bool()?;
                let sectors = r.read_u64()?;
                present.then_some(sectors)
            },
        })
    }
}

/// FNV-1a, 64-bit: a fingerprint of an image a snapshot names but does not
/// carry.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// The length of the next piece of a virtual access: to the end of the page
/// `va` is in, or `left`, whichever is less.
fn page_chunk(va: u64, left: usize) -> usize {
    let to_page_end = (0x1000 - (va & 0xFFF)) as usize;
    to_page_end.min(left)
}

fn ram_offset(ram_len: usize, addr: u64, len: usize) -> Result<usize, MemoryError> {
    let not_ram = MemoryError::NotRam { addr };
    let off = usize::try_from(addr.checked_sub(RAM_BASE).ok_or(not_ram)?).map_err(|_| not_ram)?;
    if off.checked_add(len).is_none_or(|end| end > ram_len) {
        return Err(not_ram);
    }
    Ok(off)
}

fn ram_range(ram: &[u8], addr: u64, len: usize) -> Result<&[u8], MemoryError> {
    let off = ram_offset(ram.len(), addr, len)?;
    Ok(&ram[off..off + len])
}

// ---------------------------------------------------------------------------
// Host handles
// ---------------------------------------------------------------------------

/// The machine's UART, as a host driving it sees it.
pub struct Serial<'m, S> {
    board: &'m mut VirtBoard<S>,
    /// The machine's clock, at which sent bytes reach the UART.
    clock: VmClock,
}

impl<S: GuestStorage> Serial<'_, S> {
    /// The bytes the guest has transmitted, oldest first. Draining is
    /// destructive, and the buffer is bounded: a host that never drains
    /// loses the oldest bytes rather than stalling the guest.
    pub fn take_output(&mut self) -> rusty_box_core::ring_buffer::Drain<'_, u8, TX_CAPACITY> {
        self.board.uart.drain_tx()
    }

    /// Send bytes for the guest to receive, returning how many the machine
    /// took. They wait where a QEMU character backend's input would and
    /// reach the UART's receive FIFO as it has room while the guest has the
    /// receiver enabled (`UARTCR.UARTEN` and `RXE`), so a whole command line
    /// goes in at once and the guest reads it at its own pace. A short count
    /// means the guest has stopped reading; send the rest later.
    #[must_use = "a short count means the rest never reached the guest"]
    pub fn send(&mut self, bytes: &[u8]) -> usize {
        self.board.send_serial(bytes, self.clock)
    }

    /// How many more bytes [`Serial::send`] would take now.
    pub fn input_room(&self) -> usize {
        self.board.serial_input_room()
    }
}

/// Where a [`Keyboard`]'s keys go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyboardTarget {
    /// The virtio keyboard, as Linux input events.
    Virtio,
    /// The terminal on the UART, as the bytes its keys send.
    Terminal,
}

/// A keyboard of the machine: the same key vocabulary and the same verbs as
/// an x86 machine's PS/2 keyboard ([`KeyboardInput`]).
///
/// The virtio keyboard ([`MachineBuilder::virtio_keyboard`],
/// [`VirtMachine::keyboard`]) sends keys as Linux input events, once its
/// driver is ready. The terminal's ([`VirtMachine::terminal_keyboard`], and
/// `keyboard` on a machine without a virtio keyboard, where `virt` has no
/// keyboard device) sends what a person types as the bytes their terminal
/// emulator sends, which is what QEMU's `-nographic` console carries,
/// rendered as xterm renders keys. Either way the key verbs are whole or
/// nothing: a key, a tap or a typed character that does not all fit is
/// refused, never cut. Raw [`Keyboard::bytes`] goes to the serial console
/// whatever the keyboard; it is a paste, and takes as much as fits.
pub struct Keyboard<'m, S> {
    board: &'m mut VirtBoard<S>,
    terminal: &'m mut TerminalKeyboard,
    target: KeyboardTarget,
    /// The machine's clock, at which typed bytes reach the UART.
    clock: VmClock,
}

impl<S: GuestStorage> Keyboard<'_, S> {
    /// Send raw bytes, as a terminal's paste would, returning how many were
    /// taken. `bytes[taken..]` is what still has to be sent; the split can
    /// fall inside a multi-byte UTF-8 character, so resume by byte count,
    /// not by characters.
    #[must_use = "a short count means the rest never reached the guest"]
    pub fn bytes(&mut self, bytes: &[u8]) -> usize {
        self.board.send_serial(bytes, self.clock)
    }

    /// Send `events` to the virtio keyboard, or apply them to the terminal
    /// and send what they produce, if all of it fits; otherwise change
    /// nothing.
    fn send_events<E: Iterator<Item = (Key, KeyAction)>>(&mut self, events: E) -> bool {
        match self.target {
            KeyboardTarget::Virtio => self.send_to_virtio(events),
            KeyboardTarget::Terminal => self.send_to_terminal(events),
        }
    }

    fn send_to_virtio<E: Iterator<Item = (Key, KeyAction)>>(&mut self, events: E) -> bool {
        // A stroke is at most Shift around a key: four events.
        let mut batch = [(Key::CtrlL, KeyAction::Press); 4];
        let mut count = 0;
        for event in events {
            let Some(slot) = batch.get_mut(count) else {
                return false;
            };
            *slot = event;
            count += 1;
        }
        self.board.input_event(InputKind::Keyboard, |input, vdev| input.keys(vdev, &batch[..count]))
    }

    fn send_to_terminal<E: Iterator<Item = (Key, KeyAction)>>(&mut self, events: E) -> bool {
        let mut terminal = *self.terminal;
        let mut out = [0u8; 32];
        let mut len = 0;
        for (key, action) in events {
            let produced = terminal.apply(key, action);
            let bytes = produced.as_slice();
            match out.get_mut(len..len + bytes.len()) {
                Some(dst) => dst.copy_from_slice(bytes),
                None => return false,
            }
            len += bytes.len();
        }
        if len > self.board.serial_input_room() {
            return false;
        }
        *self.terminal = terminal;
        let sent = self.board.send_serial(&out[..len], self.clock);
        sent == len
    }
}

impl<S: GuestStorage> KeyboardInput for Keyboard<'_, S> {
    fn key(&mut self, key: Key, action: KeyAction) -> bool {
        self.send_events(core::iter::once((key, action)))
    }

    fn tap(&mut self, key: Key) -> bool {
        self.send_events([(key, KeyAction::Press), (key, KeyAction::Release)].into_iter())
    }

    fn type_text(&mut self, text: &str) -> Typed {
        let mut delivered = 0;
        for ch in text.chars() {
            let Some(stroke) = KeyStroke::for_char(ch) else {
                return Typed::Unmappable { delivered, ch };
            };
            if !self.send_events(stroke.events()) {
                return Typed::Refused { delivered };
            }
            delivered += 1;
        }
        Typed::All { delivered }
    }
}

/// The virtio tablet ([`MachineBuilder::virtio_tablet`]): an absolute
/// pointer, the same verbs every rusty_box machine's pointer has
/// ([`PointerInput`]). An event reaches the guest once the tablet's driver
/// is ready; before that, every one is refused.
pub struct Pointer<'m, S> {
    board: &'m mut VirtBoard<S>,
}

impl<S: GuestStorage> PointerInput for Pointer<'_, S> {
    fn move_to(&mut self, pos: PointerPos) -> bool {
        self.board.input_event(InputKind::Tablet, |input, vdev| input.move_to(vdev, pos))
    }

    fn button(&mut self, button: PointerButton, action: KeyAction) -> bool {
        self.board.input_event(InputKind::Tablet, |input, vdev| input.button(vdev, button, action))
    }

    fn wheel(&mut self, steps: WheelSteps) -> bool {
        self.board.input_event(InputKind::Tablet, |input, vdev| input.wheel(vdev, steps))
    }
}

/// The virtio-gpu's display ([`MachineBuilder::virtio_display`]), as the
/// host sees it.
#[cfg(feature = "alloc")]
pub struct Display<'m, S> {
    board: &'m mut VirtBoard<S>,
}

#[cfg(feature = "alloc")]
impl<S: GuestStorage> Display<'_, S> {
    /// The size of the picture the guest shows, or `None` while it shows
    /// none (before its driver sets a scanout).
    pub fn resolution(&mut self) -> Option<Resolution> {
        self.board.gpu().and_then(|gpu| gpu.resolution())
    }

    /// The size the display asks the guest for.
    pub fn preferred_resolution(&mut self) -> Option<Resolution> {
        self.board.gpu().map(|gpu| gpu.preferred())
    }

    /// Ask the guest to show `size` pixels instead, as a host window being
    /// resized does on QEMU; a size of no pixels says the display is off,
    /// as a closed window does. A guest whose driver follows display events
    /// (Linux's `virtio_gpu`) changes mode; the picture changes size when
    /// it does.
    pub fn request_resolution(&mut self, size: Resolution) {
        self.board.request_display_size(size);
    }

    /// Push what changed since the last refresh into `sink`: a new size
    /// first, then the changed rectangle, with the guest's cursor drawn
    /// over it.
    pub fn refresh<D: DisplaySink>(&mut self, sink: &mut D) -> Refreshed {
        match self.board.gpu() {
            Some(gpu) => gpu.refresh(sink),
            None => Refreshed::Unchanged,
        }
    }
}

// ---------------------------------------------------------------------------
// The verbs every rusty_box machine shares
// ---------------------------------------------------------------------------

impl RunOutcome for BatchOutcome {
    fn progress(&self) -> Progress {
        self.progress
    }

    fn is_terminal(&self) -> bool {
        BatchOutcome::is_terminal(self)
    }
}

impl<S: GuestStorage> SerialPort for Serial<'_, S> {
    type Output<'a>
        = rusty_box_core::ring_buffer::Drain<'a, u8, TX_CAPACITY>
    where
        Self: 'a;

    fn take_output(&mut self) -> Self::Output<'_> {
        Serial::take_output(self)
    }

    fn send(&mut self, bytes: &[u8]) -> usize {
        Serial::send(self, bytes)
    }
}

/// A run of this machine always ends with an outcome, so its error is
/// [`Infallible`](core::convert::Infallible).
impl<S: GuestStorage, I: Instrumentation> Machine for VirtMachine<S, I> {
    type Outcome = BatchOutcome;
    type Error = core::convert::Infallible;
    type Serial<'a>
        = Serial<'a, S>
    where
        Self: 'a;
    type Keyboard<'a>
        = Keyboard<'a, S>
    where
        Self: 'a;

    fn step(&mut self, budget: RunBudget) -> Result<BatchOutcome, core::convert::Infallible> {
        Ok(VirtMachine::step(self, budget))
    }

    fn ticks(&self) -> u64 {
        VirtMachine::ticks(self)
    }

    fn serial(&mut self, port: usize) -> Option<Serial<'_, S>> {
        VirtMachine::serial(self, port)
    }

    fn keyboard(&mut self) -> Keyboard<'_, S> {
        VirtMachine::keyboard(self)
    }
}

/// The machine's power and reset controls.
pub struct Power<'m, S, I> {
    machine: &'m mut VirtMachine<S, I>,
}

impl<S: GuestStorage, I: Instrumentation> Power<'_, S, I> {
    pub fn state(&self) -> PowerState {
        self.machine.power
    }

    /// Reset the machine, as QEMU's system reset does: the processor and the
    /// devices to their reset state, and the boot started again — firmware
    /// from its reset vector, a directly booted kernel loaded afresh with
    /// its initrd and device tree. RAM a boot does not load keeps its
    /// contents; guest time carries on. A machine that was switched off is
    /// on again.
    pub fn reset(&mut self) {
        self.machine.start();
    }

    /// Press the power button (QEMU's `gpio-key` on GPIO line 3), for 100 ms
    /// of guest time. This is a request to the guest, not an order: a guest
    /// that handles `KEY_POWER` shuts itself down, at which point a run
    /// reports [`StopReason::PoweredOff`]; one that does not keeps running,
    /// as it would on the real board.
    pub fn press_power_button(&mut self) {
        let clock = self.machine.cpu.clock();
        self.machine.board.press_power_button(clock);
        // The release is a deadline the run loop has not seen.
        self.machine.sync_events();
    }
}

#[cfg(test)]
#[path = "../tests/unit/machine.rs"]
mod tests;
