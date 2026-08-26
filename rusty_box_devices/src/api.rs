//! The device-facing contracts a device model may use *during* dispatch.
//!
//! Bochs devices call `DEV_pic_raise_irq` and `bx_pc_system.activate_timer`
//! synchronously from inside their port handlers. This port could not: I/O
//! dispatch reached devices through a raw bus pointer that could not also
//! reach the PIC or the timer wheel, so every such call had to be latched into
//! a fixed side table and replayed at the next scheduler boundary.
//!
//! These traits restore the Bochs shape. A converted device receives a
//! [`DeviceCtx`] carrying the capabilities it may exercise, and calls them
//! directly. The latch tables exist only for devices not yet converted.
//!
//! Everything here is `no_std` and allocation-free: the context borrows, it
//! never owns, and the trait objects are `&mut dyn` rather than boxed.

/// An ISA interrupt line (IRQ0-15) — Bochs `DEV_pic_raise_irq` line numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrqLine(pub u8);

/// Width of a port access. Bochs passes `io_len` as a raw byte count; this
/// makes the three legal widths explicit at the trait boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoLen {
    Byte,
    Word,
    Dword,
}

impl IoLen {
    /// Bochs `io_len` byte count.
    #[inline]
    pub const fn bytes(self) -> u8 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Dword => 4,
        }
    }

    /// Widths other than 1/2/4 are not architecturally reachable: the CPU's
    /// port paths only ever issue those three.
    #[inline]
    pub const fn from_bytes(bytes: u8) -> Option<Self> {
        match bytes {
            1 => Some(Self::Byte),
            2 => Some(Self::Word),
            4 => Some(Self::Dword),
            _ => None,
        }
    }
}

/// Which of a device's memory-mapped windows an access landed in.
///
/// A device may occupy several disjoint physical ranges — the VGA answers the
/// legacy `A0000` aperture, a linear framebuffer and a register block — and
/// Bochs distinguishes them by comparing the address against bases the device
/// keeps for itself. The machine already knows which range it routed, so it
/// says so, and the device matches on a closed set instead of re-deciding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowId(pub u8);

impl WindowId {
    /// The window a device that declares only one gets.
    pub const FIRST: Self = Self(0);
}

/// How far into its window an access landed.
///
/// Distinct from a physical address on purpose (R4): the device never learns
/// where the machine mapped it, so it cannot route on a base it would then have
/// to keep in step with the machine's mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowOffset(pub u64);

impl WindowOffset {
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A guest physical address.
///
/// A plain `u64` for now. Unit J gives guest addresses a type of their own, at
/// which point this alias is where that change lands; `rusty_box` re-exports
/// this one rather than keeping a second definition of the same thing.
pub type BxPhyAddress = u64;

/// One I/O port a device answers on, as the device states it.
///
/// Bochs has each device call `DEV_register_ioread_handler` on the bus from
/// inside its own `init` (vgacore.cc `bx_vgacore_c::init`). Stating it as data
/// instead is what lets a device model be built without a bus to call: the set
/// that owns the bus reads the declaration and does the registering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortDecl {
    pub port: u16,
    /// Diagnostic name, as Bochs passes to the same call.
    pub name: &'static str,
    /// Bochs's `mask`: which access widths this port decodes (1 = byte,
    /// 2 = word, 4 = dword), OR-ed.
    pub widths: u8,
}

/// One physical-address window a device answers on, as the device states it.
///
/// The window carries the [`WindowId`] the device will be handed back on every
/// access, so the name a device routes on and the range the machine maps are
/// declared in one place and cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowDecl {
    pub id: WindowId,
    pub base: u64,
    /// Last byte in the window, inclusive — Bochs `register_memory_handlers`
    /// takes `end_addr` the same way.
    pub end: u64,
}

/// How many windows one device may declare.
///
/// Three is what the busiest model here needs (a display's legacy aperture, its
/// framebuffer and its register block); the fourth is slack. A device that
/// wanted more would be telling us the bound is wrong, which is why
/// [`WindowDecls::push`] reports rather than drops.
pub const MAX_DEVICE_WINDOWS: usize = 4;

/// Whether a declaration was taken.
///
/// A dropped window is a whole aperture the guest writes into and nothing
/// answers — silent, and indistinguishable from a mode the card does not
/// support. So the answer is a value the caller must read (R0), not a `bool`
/// that reads the same either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a window that was not taken is an aperture nothing answers"]
pub enum Declared {
    Accepted,
    /// The device declared more windows than [`MAX_DEVICE_WINDOWS`] allows.
    NoRoom,
}

/// The windows a device declares, gathered without an allocator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowDecls {
    decls: [Option<WindowDecl>; MAX_DEVICE_WINDOWS],
    len: usize,
}

impl WindowDecls {
    pub const fn new() -> Self {
        Self {
            decls: [None; MAX_DEVICE_WINDOWS],
            len: 0,
        }
    }

    pub fn push(&mut self, decl: WindowDecl) -> Declared {
        match self.decls.get_mut(self.len) {
            Some(slot) => {
                *slot = Some(decl);
                self.len += 1;
                Declared::Accepted
            }
            None => Declared::NoRoom,
        }
    }

    /// Every window declared, in the order the device declared it. Bochs
    /// registers in call order and a later range wins an overlap, so the order
    /// is part of what a device is saying.
    pub fn as_slice(&self) -> WindowSlice<'_> {
        WindowSlice {
            decls: &self.decls[..self.len],
        }
    }
}

/// A borrowed run of declared windows.
///
/// A named type rather than `&[Option<WindowDecl>]` (R0): the `Option` is this
/// container's storage, not something a reader should have to unwrap.
#[derive(Debug, Clone, Copy)]
pub struct WindowSlice<'a> {
    decls: &'a [Option<WindowDecl>],
}

impl<'a> IntoIterator for WindowSlice<'a> {
    type Item = WindowDecl;
    type IntoIter = core::iter::Flatten<core::iter::Copied<core::slice::Iter<'a, Option<WindowDecl>>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.decls.iter().copied().flatten()
    }
}

impl WindowSlice<'_> {
    pub fn len(&self) -> usize {
        self.decls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }
}

/// Identifies one timer owned by one device.
///
/// `local` is the device's own index for the timer, so a device owning several
/// (a UART owns an RX FIFO-timeout and a TX shift timer) distinguishes them
/// without the scheduler knowing what they mean. This is the data-carrying
/// replacement for the closed `DeviceTimerOwner` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerKey {
    pub device: DeviceKind,
    pub local: u16,
}

/// Which device model a [`TimerKey`] belongs to.
///
/// Converted devices only. Unconverted devices keep their `DeviceTimerOwner`
/// slot until they are moved across, at which point their variant is added
/// here and removed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Serial,
    Acpi,
    Cmos,
    Pit,
    Keyboard,
    /// The ATA/ATAPI drives and the PIIX bus-master engine share one kind:
    /// they are one controller in hardware, and ids 0-3 are the per-drive seek
    /// timers with 4-5 the two bus-master channels.
    Ide,
}

/// Interrupt delivery, as seen by a device.
///
/// Bochs devices call `DEV_pic_raise_irq`/`DEV_pic_lower_irq` synchronously;
/// `set_level` is the level-oriented primitive both reduce to (Bochs
/// pic.cc `set_irq_level`).
pub trait IrqSink {
    fn set_level(&mut self, line: IrqLine, level: bool);

    /// Current level of `line` at the controller's input.
    ///
    /// Bochs devices never ask — they only drive. This exists because a device
    /// that replays a burst of edges needs to know whether the line was
    /// already asserted to account for them correctly, and the controller is
    /// the only authority on that.
    fn level(&self, line: IrqLine) -> bool;

    #[inline]
    fn raise(&mut self, line: IrqLine) {
        self.set_level(line, true);
    }

    #[inline]
    fn lower(&mut self, line: IrqLine) {
        self.set_level(line, false);
    }
}

/// Timer arming, as seen by a device — Bochs `bx_pc_system.activate_timer`
/// and `deactivate_timer`, reachable from inside a port handler.
pub trait TimerService {
    /// Arm `key` as a one-shot, `delay_usec` microseconds of emulated time
    /// from now. Re-arming an already-armed timer replaces its deadline.
    fn arm_oneshot_usec(&mut self, key: TimerKey, delay_usec: u64);

    /// Arm `key` to fire every `period_usec` microseconds until cancelled —
    /// Bochs `activate_timer(..., continuous = 1)`.
    fn arm_periodic_usec(&mut self, key: TimerKey, period_usec: u64);

    /// Arm `key` as a one-shot `delay_ticks` scheduler ticks from now.
    ///
    /// For deadlines that are inherently tick-denominated rather than
    /// wall-clock — the PIIX bus-master engine schedules its callback one tick
    /// out, and routing that through microseconds would round it away.
    fn arm_oneshot_ticks(&mut self, key: TimerKey, delay_ticks: u64);

    /// Disarm `key`. Disarming an already-idle timer is a no-op.
    fn cancel(&mut self, key: TimerKey);
}

/// The capabilities a device may exercise while handling an access.
///
/// Borrowed, never owned: the dispatcher builds one from disjoint machine
/// fields for the duration of a single device call.
pub struct DeviceCtx<'a> {
    /// Emulated time at the access, in scheduler ticks. Bochs devices read
    /// `bx_pc_system.time_ticks()` for the same purpose.
    pub now_ticks: u64,
    /// Emulated instructions per second — Bochs `bx_pc_system.m_ips`. A device
    /// that converts between ticks and wall-clock microseconds needs it.
    pub ips: u64,
    pub irq: &'a mut dyn IrqSink,
    pub timers: &'a mut dyn TimerService,
}

/// A device that occupies I/O ports.
///
/// Mirrors the Bochs `read_handler`/`write_handler` pair, minus the
/// `void *this_ptr` trampoline: the device is `self`.
pub trait PioDevice {
    fn pio_read(&mut self, port: u16, len: IoLen, ctx: &mut DeviceCtx<'_>) -> u32;
    fn pio_write(&mut self, port: u16, value: u32, len: IoLen, ctx: &mut DeviceCtx<'_>);
}

/// A device that occupies a physical address range.
///
/// Mirrors the Bochs `memory_handler_t` read/write pair, minus the `void
/// *param` trampoline: the device is `self`, and the region it was registered
/// for is identified by a token the platform routes rather than by a pointer
/// the memory subsystem dereferences.
///
/// The access names the window it landed in and how far into it, never the
/// physical address. A device with several windows therefore matches on a
/// closed set rather than comparing against bases of its own, and no device
/// needs to know where the machine mapped it.
///
/// The same [`DeviceCtx`] the port path builds, for the same reason: Bochs
/// hpet.cc arms a comparator and raises its interrupt from inside the MMIO
/// write that programmed it, so a memory-reached device needs the interrupt
/// and timer capabilities a port-reached one has. Narrowing this to a bare
/// clock would put a latch back between the write and its effect.
pub trait MmioDevice {
    fn mmio_read(
        &mut self,
        window: WindowId,
        at: WindowOffset,
        len: u32,
        data: &mut [u8],
        ctx: &mut DeviceCtx<'_>,
    );
    fn mmio_write(
        &mut self,
        window: WindowId,
        at: WindowOffset,
        len: u32,
        data: &[u8],
        ctx: &mut DeviceCtx<'_>,
    );
}

/// The i440FX SMRAM control state — Bochs pci.cc `smram_control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmramControl {
    /// SMRAME clear: the window is closed.
    Disable,
    /// SMRAME set, with the open/close-on-store qualifiers.
    Enable { dopen: bool, dcls: bool },
}

/// Number of shadow-RAM areas the i440FX PAM registers cover.
pub const PAM_AREAS: usize = 13;

/// A change to machine-wide state that a chipset device asks for but cannot
/// perform itself.
///
/// Bochs chipset handlers call `DEV_mem_set_memory_type`, `mem->enable_smram()`
/// and friends directly, so its devices hold the memory bus. This port
/// inherited that: `apply_pam_to_memory(&mut BxMemC)` and three siblings handed
/// a device the whole memory subsystem to poke. A device now describes what it
/// wants in its own terms and the machine — which owns memory — carries it out,
/// the same split the MMIO inversion made in the other direction.
///
/// Each variant carries decoded intent rather than raw configuration bytes, so
/// chipset semantics (which PAM bit means writable, how the I/O APIC base is
/// scaled) stay with the chipset that defines them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipsetEffect {
    /// Shadow-RAM routing for every PAM area, indexed `[area][write]`.
    ///
    /// One effect for the whole table rather than one per area: the PAM
    /// registers are written as a set and applied as a set, and Bochs's own
    /// reset path re-applies all of them in a loop.
    ShadowRam([[bool; 2]; PAM_AREAS]),
    /// SMRAM window control.
    Smram(SmramControl),
    /// PIIX3 XBCS (0x4E): BIOS write-enable and the two ROM access windows.
    BiosRom {
        write_enabled: bool,
        lower: bool,
        extended: bool,
    },
    /// PIIX3 0x4F bit 1: the 1 MB extended BIOS access window.
    BiosRom1Meg(bool),
    /// PIIX3 0x4F/0x80: I/O APIC enable state and its MMIO base offset.
    IoApicEnable { enabled: bool, base_offset: u16 },
    /// A byte one device asks the chipset to store into the CMOS RAM —
    /// Bochs `DEV_cmos_set_reg`.
    CmosByte { index: u8, value: u8 },
}

/// A device that owns scheduler timers.
pub trait TimedDevice {
    /// One or more expirations of the device's `local` timer have come due.
    ///
    /// `fires` is the coalesced count, matching the existing scheduler
    /// transport, which reports how many periods elapsed rather than calling
    /// once per period.
    fn timer_fired(&mut self, local: u16, fires: u32, ctx: &mut DeviceCtx<'_>);
}
