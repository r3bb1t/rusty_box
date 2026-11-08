//! The `virt` board: its devices, where they sit, which [`crate::layout`]
//! decides, and the wiring of their interrupt lines into the GIC, which
//! [`crate::interrupt_map`] decides.
//!
//! Provenance (R7): QEMU `hw/arm/virt.c` for the devices it has. The
//! PCIe host bridge has no devices on it here — QEMU puts its own host-bridge
//! function at 00:00.0 — so configuration reads find nothing and the I/O and
//! memory windows answer like an empty bus (all ones, writes ignored).
//!
//! Each flash bank decodes its whole 64 MiB window, as QEMU's
//! `virt_flash_map` maps it, whatever the length of the array behind it:
//! QEMU insists on a drive of exactly the bank's size, and this board reads
//! the part of the bank an array does not cover as erased flash instead, so
//! a 2 MiB firmware image needs no padding and a directly booted kernel
//! needs no flash images at all.
//!
//! The power button is QEMU's `gpio-key` on the GPIO line the layout names
//! ([`DeviceLayout::power_button_line`]: line 0 on the board's own, line 3
//! on QEMU's): a press drives the line high and a timer releases it 100 ms
//! of guest time later (`hw/gpio/gpio_key.c`, `GPIO_KEY_LATENCY`).

use rusty_box_core::{ClockHz, RingBuffer, VmClock, VmInstant};
use rusty_box_devices::display::Resolution;

use crate::bus::{
    load_le, store_le, AccessWidth, BusFault, CodeEpoch, FetchSource, PhysAddr, PhysBus, Writes,
};
use crate::cpu::TimerLines;
use crate::devices::cfi_flash::{CfiFlash, FlashArray};
use crate::devices::gicv2::{GicExtensions, GicNames, GicShape, GicV2};
use crate::devices::pl011::{BaudRate, Pl011, Received, UartClocks};
use crate::devices::pl031::Pl031;
use crate::devices::pl061::Pl061;
#[cfg(feature = "alloc")]
use crate::devices::virtio::block::Block;
#[cfg(feature = "alloc")]
use crate::devices::virtio::gpu::Gpu;
use crate::devices::virtio::input::{Input, InputKind};
use crate::devices::virtio::queue::GuestRam;
use crate::devices::virtio::{Device, Transport, Vdev, MAX_DEVICES};
use crate::devices::virtio_mmio::{self, Layout};
#[cfg(feature = "alloc")]
use crate::disk::Disk;
use crate::identity::{BoardIdentity, VirtioVendorId};
use crate::interrupt_map::Wiring;
use crate::layout::{DeviceLayout, DeviceWindows, GicWindows, TransportCount, TransportRegister};
use crate::storage::GuestStorage;

pub const FLASH0_BASE: u64 = 0x0000_0000;
pub const FLASH_BANK_SIZE: u64 = 0x0400_0000;
pub const FLASH1_BASE: u64 = FLASH0_BASE + FLASH_BANK_SIZE;
/// The devices' windows in the board's own layout
/// ([`DeviceLayout::Board`]); a machine of another layout
/// ([`crate::VirtConfig::layout`]) has its devices where that layout's
/// [`DeviceLayout::windows`] says. Each window decodes the size its
/// `*_SIZE` gives, which the device tree names; the rest of its page is
/// unassigned.
const OWN: DeviceWindows = DeviceLayout::Board.windows();
/// Each layout's windows, which the bus decodes against as constants.
const BOARD_WINDOWS: DeviceWindows = DeviceLayout::Board.windows();
const QEMU_VIRT_WINDOWS: DeviceWindows = DeviceLayout::QemuVirt.windows();
pub const GIC_DIST_BASE: u64 = OWN.gic.dist.get();
pub const GIC_DIST_SIZE: u64 = GicWindows::DIST_SIZE;
pub const GIC_CPU_BASE: u64 = OWN.gic.cpu.get();
pub const GIC_CPU_SIZE: u64 = GicWindows::CPU_SIZE;
pub const GIC_V2M_BASE: u64 = OWN.gic.v2m.get();
pub const GIC_V2M_SIZE: u64 = GicWindows::V2M_SIZE;
pub const GIC_HYP_BASE: u64 = OWN.gic.hyp.get();
pub const GIC_HYP_SIZE: u64 = GicWindows::HYP_SIZE;
pub const GIC_VCPU_BASE: u64 = OWN.gic.vcpu.get();
pub const GIC_VCPU_SIZE: u64 = GicWindows::VCPU_SIZE;
pub const UART0_BASE: u64 = OWN.uart.get();
pub const RTC_BASE: u64 = OWN.rtc.get();
pub const GPIO_BASE: u64 = OWN.gpio.get();
pub const PRIMECELL_SIZE: u64 = DeviceWindows::PRIMECELL_SIZE;
pub const VIRTIO_MMIO_BASE: u64 = OWN.virtio.first.get();
/// The distance from one virtio-mmio transport to the next, which is the
/// window each decodes.
pub const VIRTIO_MMIO_STRIDE: u64 = OWN.virtio.stride.bytes();
/// How many virtio-mmio transports there are.
pub const VIRTIO_MMIO_COUNT: u8 = OWN.virtio.count.get();
pub const PCIE_MMIO_BASE: u64 = OWN.pcie.mmio32.base.get();
pub const PCIE_MMIO_SIZE: u64 = OWN.pcie.mmio32.size;
pub const PCIE_PIO_BASE: u64 = OWN.pcie.pio.base.get();
pub const PCIE_PIO_SIZE: u64 = OWN.pcie.pio.size;
pub const RAM_BASE: u64 = 0x4000_0000;
pub const PCIE_ECAM_BASE: u64 = OWN.pcie.ecam.base.get();
pub const PCIE_ECAM_SIZE: u64 = OWN.pcie.ecam.size;
pub const PCIE_MMIO_HIGH_BASE: u64 = OWN.pcie.mmio64.base.get();
pub const PCIE_MMIO_HIGH_SIZE: u64 = OWN.pcie.mmio64.size;

/// The most RAM the board can map, in any layout: everything between the
/// base of RAM and the lowest window above it.
pub const RAM_LIMIT: u64 = DeviceLayout::lowest_above_ram().get() - RAM_BASE;

/// GIC interrupt IDs of the timer PPIs (the device tree's `timer` node);
/// the devices' SPIs are the board's
/// [`InterruptMap`](crate::interrupt_map::InterruptMap)'s.
pub const PPI_VIRTUAL_TIMER: u32 = 16 + 11;
pub const PPI_PHYSICAL_TIMER: u32 = 16 + 14;
/// The EL2 physical timer's PPI (QEMU `ARCH_TIMER_NS_EL2_IRQ`), which the
/// device tree's `timer` node lists whether or not the processor has EL2.
pub const PPI_HYPERVISOR_TIMER: u32 = 16 + 10;
/// The Secure EL1 physical timer's PPI (BSA DEN0094C RB_PPI_01's), which the
/// device tree's `timer` node lists first, as its binding orders them,
/// though a board with no EL3 never raises it.
pub const PPI_SECURE_PHYSICAL_TIMER: u32 = 16 + 13;

/// The PL011's reference clock, `UARTCLK`: the device tree's `/clock`
/// fixed clock, 24 MHz, which its `serial` node names as `uartclk`.
/// The rate is QEMU `hw/arm/virt.c` `create_fdt`'s, which stock EDK2 is
/// built for (`PL011UartClkInHz`, a fixed PCD it works the baud divisor
/// out from), so it stays though the clock is the board's to choose
/// (README, "QEMU fingerprints left").
pub const UART_CLOCK: ClockHz = match ClockHz::new(24_000_000) {
    Some(hz) => hz,
    None => unreachable!(),
};

/// The UART's clocks at `clock`: the machine's, and [`UART_CLOCK`].
fn uart_clocks(clock: VmClock) -> UartClocks {
    UartClocks {
        machine: clock,
        uartclk: UART_CLOCK,
    }
}

/// Bytes a host may have sent to the UART that its receive FIFO has not yet
/// taken: the buffer a QEMU character backend holds between the host and
/// `pl011_can_receive`.
pub const SERIAL_INPUT_CAPACITY: usize = 4096;

/// How long QEMU's `gpio-key` holds the power button down.
const POWER_BUTTON_HOLD_MICROS: u64 = 100_000;

/// The time-driven devices: the RTC, the power button and the UART, and in
/// the run loop's tests a device double whose deadline is never cleared.
const DEADLINE_SOURCES: usize = if cfg!(test) { 4 } else { 3 };

/// Which free virtio-mmio transport each device plugged in takes, the
/// devices taken in the order [`VirtioDevices`] gives. The virtio
/// specification (OASIS virtio 1.2 §4.2) leaves where a device sits to the
/// platform, and a guest finds it by the transport's registers and the
/// device tree's node, so the order is the board's choice.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportOrder {
    /// The first transport the devices before it left free: the first
    /// device plugged in at the lowest address, the first the device tree
    /// lists.
    FirstFree,
    /// The last transport the devices before it left free, as QEMU's
    /// `virt` hands `-device` options their bus (`hw/arm/virt.c`
    /// `create_virtio_devices`).
    LastFree,
}

impl TransportOrder {
    /// Every order, in the order a list of them shows.
    pub const ALL: [Self; 2] = [Self::FirstFree, Self::LastFree];

    /// The order a board takes unless its host chooses another.
    pub const DEFAULT: Self = Self::FirstFree;

    /// The order's name for a command line or a configuration file, which
    /// [`str::parse`] reads back.
    pub const fn name(self) -> &'static str {
        match self {
            Self::FirstFree => "first-free",
            Self::LastFree => "last-free",
        }
    }

    /// The transport the `n`th device plugged in takes, of `count`.
    const fn slot(self, n: u8, count: TransportCount) -> u8 {
        match self {
            Self::FirstFree => n,
            Self::LastFree => count.get() - 1 - n,
        }
    }
}

impl Default for TransportOrder {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl core::fmt::Display for TransportOrder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

impl core::str::FromStr for TransportOrder {
    type Err = UnknownTransportOrder;

    /// The order whose [`TransportOrder::name`] is `name`.
    fn from_str(name: &str) -> Result<Self, UnknownTransportOrder> {
        Self::ALL
            .into_iter()
            .find(|order| order.name() == name)
            .ok_or(UnknownTransportOrder)
    }
}

/// A name no [`TransportOrder`] has. It prints the names there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnknownTransportOrder;

impl core::fmt::Display for UnknownTransportOrder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("no transport order has that name; the orders are")?;
        for (i, order) in TransportOrder::ALL.iter().enumerate() {
            let separator = if i == 0 { " " } else { ", " };
            write!(f, "{separator}\"{}\"", order.name())?;
        }
        Ok(())
    }
}

impl core::error::Error for UnknownTransportOrder {}

/// The display and input devices plugged into the board, in the order QEMU
/// would be given them as `-device` options: the display, the keyboard, the
/// tablet. Each takes the transport the board's [`TransportOrder`] gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VirtioDevices {
    /// A virtio-gpu, asking the guest for this size.
    pub(crate) display: Option<Resolution>,
    pub(crate) keyboard: Plugged,
    pub(crate) tablet: Plugged,
}

/// What a builder plugs into the board: the display and input devices, and
/// a disk, which a virtio-blk given after them serves.
pub(crate) struct Plugs {
    pub(crate) devices: VirtioDevices,
    #[cfg(feature = "alloc")]
    pub(crate) disk: Option<Disk>,
}

impl Plugs {
    pub(crate) const NONE: Self = Self {
        devices: VirtioDevices::NONE,
        #[cfg(feature = "alloc")]
        disk: None,
    };
}

/// Whether a device is on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Plugged {
    #[default]
    No,
    Yes,
}

impl VirtioDevices {
    /// None: every transport empty.
    pub const NONE: Self = Self {
        display: None,
        keyboard: Plugged::No,
        tablet: Plugged::No,
    };

    /// A bit per device, for a snapshot's record of how a machine was
    /// built.
    pub(crate) fn mask(&self) -> u8 {
        u8::from(self.display.is_some())
            | (u8::from(self.keyboard == Plugged::Yes) << 1)
            | (u8::from(self.tablet == Plugged::Yes) << 2)
    }
}

/// Everything on the board's bus.
//
// The declared order is the layout (`repr(C)`), chosen for the run loop
// (README "Speed"). The devices come first and what the loop reads on
// every access, trace or instruction (`storage` to `dma`) after them, so a
// device's size moves those fields but never changes how the loop reaches
// them: each access keeps a 32-bit displacement from the board, the
// instruction lengths and the loop's alignment padding stay as they are.
#[repr(C)]
pub struct VirtBoard<S> {
    pub(crate) flash0: CfiFlash,
    pub(crate) flash1: CfiFlash,
    pub(crate) gic: GicV2,
    pub(crate) uart: Pl011,
    pub(crate) rtc: Pl031,
    pub(crate) gpio: Pl061,
    /// The SPIs the devices drive.
    pub(crate) wiring: Wiring,
    /// Where the devices sit.
    pub(crate) layout: DeviceLayout,
    /// What every virtio-mmio transport's `VendorID` reads, the board's
    /// identity's ([`BoardIdentity::virtio_vendor`]).
    pub(crate) virtio_vendor: VirtioVendorId,
    /// Host bytes waiting for room in the UART's receive FIFO.
    pub(crate) serial_input: RingBuffer<u8, SERIAL_INPUT_CAPACITY>,
    /// When the pressed power button is released, if it is held.
    pub(crate) power_button_release: Option<VmInstant>,
    /// The virtio devices, each behind its transport.
    pub(crate) virtio: [Option<Transport>; MAX_DEVICES],
    /// The register layout of every transport, plugged or empty.
    pub(crate) mmio_layout: Layout,
    pub(crate) storage: S,
    /// A guest access moved a device's next deadline (the RTC's alarm was
    /// re-armed), so the machine's next event has to be worked out again.
    pub(crate) deadline_moved: bool,
    /// A guest access reached a device since the machine last looked: any
    /// access outside RAM but a flash read. Only such an access changes
    /// what the interrupt controller drives, a device's deadline, RAM a
    /// device writes, or a flash bank's array; while it is clear, none of
    /// them has changed. A flash read changes none of them either: at most
    /// it returns its own bank to read-array mode, which the next fetch
    /// from the bank asks about afresh (`fetch_source`), and whose array
    /// it leaves as it was.
    pub(crate) touched: bool,
    /// Counts the writes to the flash banks, each of which may change what
    /// a fetch from them reads: their [`CodeEpoch`].
    pub(crate) flash_epoch: u64,
    /// The RAM the devices wrote since the machine last looked.
    pub(crate) dma: Writes,
    /// A device double for the run loop's tests: a deadline it reports and
    /// never clears, however far the devices catch up.
    #[cfg(test)]
    pub(crate) stuck_deadline: Option<VmInstant>,
}

impl<S: GuestStorage> VirtBoard<S> {
    /// A board over `storage` whose RTC starts at `rtc_epoch_seconds`, with
    /// the devices `plugs` names, each naming itself as `identity` says, and
    /// a GIC of `gic_shape` with `gic` (the Virtualization Extensions for a
    /// processor with EL2), whose frames it maps, the devices driving the
    /// SPIs `wiring` gives them and its GICv2m frame having the rest it
    /// gives, and each virtio device in the transport `order` gives it, the
    /// devices where `layout` puts them.
    pub(crate) fn new(
        storage: S,
        rtc_epoch_seconds: u64,
        plugs: Plugs,
        identity: &BoardIdentity,
        gic: GicExtensions,
        gic_shape: GicShape,
        wiring: Wiring,
        order: TransportOrder,
        layout: DeviceLayout,
    ) -> Self {
        let devices = plugs.devices;
        let virtio_vendor = identity.virtio_vendor();
        let mut virtio: [Option<Transport>; MAX_DEVICES] = [None, None, None, None];
        // The `n`th device plugged in fills the `n`th entry and sits behind
        // the transport the order gives the `n`th.
        let count = layout.windows().virtio.count;
        let plug = |device: Device, virtio: &mut [Option<Transport>; MAX_DEVICES]| {
            if let Some(n) = virtio.iter().position(Option::is_none) {
                virtio[n] = Some(Transport::new(order.slot(n as u8, count), device));
            }
        };
        #[cfg(feature = "alloc")]
        if let Some(size) = devices.display {
            let gpu = Gpu::new(size, identity.display(), storage.ram().len());
            plug(Device::Gpu(gpu), &mut virtio);
        }
        if devices.keyboard == Plugged::Yes {
            let keyboard = Input::for_board(InputKind::Keyboard, identity);
            plug(Device::Input(keyboard), &mut virtio);
        }
        if devices.tablet == Plugged::Yes {
            let tablet = Input::for_board(InputKind::Tablet, identity);
            plug(Device::Input(tablet), &mut virtio);
        }
        #[cfg(feature = "alloc")]
        if let Some(disk) = plugs.disk {
            plug(Device::Block(Block::new(disk)), &mut virtio);
        }
        let mmio_layout = if virtio.iter().any(Option::is_some) {
            Layout::Modern
        } else {
            Layout::Legacy
        };
        Self {
            storage,
            flash0: CfiFlash::new(FLASH_BANK_SIZE),
            flash1: CfiFlash::new(FLASH_BANK_SIZE),
            gic: GicV2::with_extensions(gic)
                .shaped(gic_shape)
                .identified_as(GicNames {
                    gic: identity.gic(),
                    msi_frame: identity.msi_frame_iidr(),
                })
                .with_checked_msi_frame(wiring.msi()),
            uart: Pl011::new(),
            rtc: Pl031::new(rtc_epoch_seconds),
            gpio: Pl061::new(),
            wiring,
            layout,
            virtio_vendor,
            serial_input: RingBuffer::new(),
            power_button_release: None,
            deadline_moved: false,
            touched: false,
            flash_epoch: 0,
            dma: Writes::NONE,
            virtio,
            mmio_layout,
            #[cfg(test)]
            stuck_deadline: None,
        }
    }

    /// A bit per transport a device is plugged into, transport `n` bit `n`,
    /// for a snapshot's record of how a machine was built; 32 bits hold
    /// every transport a layout can have.
    pub(crate) fn occupied_transports(&self) -> u32 {
        const _: () = assert!(TransportCount::MAX.get() as u32 <= u32::BITS);
        self.virtio
            .iter()
            .flatten()
            .fold(0, |mask, t| mask | 1 << t.slot())
    }

    /// The transport at `slot`, if a device is plugged into it.
    fn transport_mut(&mut self, slot: u8) -> Option<&mut Transport> {
        self.virtio.iter_mut().flatten().find(|t| t.slot() == slot)
    }

    /// The virtio input device of `kind`, if one is plugged in.
    pub(crate) fn has_input(&self, kind: InputKind) -> bool {
        self.virtio
            .iter()
            .flatten()
            .any(|t| t.input_kind() == Some(kind))
    }

    /// Have the virtio input device of `kind` send the guest what `send`
    /// sends. `false` when there is no such device or it delivered nothing.
    pub(crate) fn input_event<F: FnOnce(&mut Input, &mut Vdev<'_, '_>) -> bool>(
        &mut self,
        kind: InputKind,
        send: F,
    ) -> bool {
        let Some(transport) = self
            .virtio
            .iter_mut()
            .flatten()
            .find(|t| t.input_kind() == Some(kind))
        else {
            return false;
        };
        let mut ram = GuestRam::new(self.storage.ram_mut());
        let delivered = transport.with_device(&mut ram, |device, vdev| match device {
            Device::Input(input) => send(input, vdev),
            #[cfg(feature = "alloc")]
            Device::Gpu(_) | Device::Block(_) => false,
        });
        self.dma.merge(&ram.written());
        self.sync_device_lines();
        delivered
    }

    /// The virtio-gpu, if one is plugged in.
    #[cfg(feature = "alloc")]
    pub(crate) fn gpu(&mut self) -> Option<&mut Gpu> {
        self.virtio
            .iter_mut()
            .flatten()
            .find_map(|t| match &mut t.device {
                Device::Gpu(gpu) => Some(gpu),
                Device::Input(_) | Device::Block(_) => None,
            })
    }

    /// The virtio-blk, if one is plugged in.
    #[cfg(feature = "alloc")]
    pub(crate) fn block(&self) -> Option<&Block> {
        self.virtio.iter().flatten().find_map(|t| match &t.device {
            Device::Block(block) => Some(block),
            Device::Input(_) | Device::Gpu(_) => None,
        })
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn block_mut(&mut self) -> Option<&mut Block> {
        self.virtio
            .iter_mut()
            .flatten()
            .find_map(|t| match &mut t.device {
                Device::Block(block) => Some(block),
                Device::Input(_) | Device::Gpu(_) => None,
            })
    }

    /// Ask the guest for a display of `size`, as a QEMU window being resized
    /// does (`virtio_gpu_ui_info`): a display event and a
    /// configuration-change interrupt.
    #[cfg(feature = "alloc")]
    pub(crate) fn request_display_size(&mut self, size: Resolution) {
        for transport in self.virtio.iter_mut().flatten() {
            let changed = match &mut transport.device {
                Device::Gpu(gpu) => gpu.ui_info(size),
                Device::Input(_) | Device::Block(_) => false,
            };
            if changed {
                transport.notify_config();
            }
        }
        self.sync_device_lines();
    }

    /// A system reset of the board's devices. The RTC stops at the value it
    /// reads and clears its bus-clock registers, as `PRESETn` does
    /// ([`Pl031::reset`]; QEMU's PL031 has no reset). A held power button is
    /// released, then the GPIO's registers clear ([`Pl061::reset`]). The UART
    /// empties its receive FIFO, as `pl011_reset` does;
    /// what the guest transmitted stays for the host to drain, and host bytes
    /// still waiting for the FIFO stay queued for it, which takes them once
    /// the guest switches the receiver back on, at `clock`. Memory is
    /// untouched.
    pub(crate) fn reset_devices(&mut self, clock: VmClock) {
        self.flash0.reset();
        self.flash1.reset();
        self.flash_epoch += 1;
        self.gic.reset();
        self.uart.reset();
        self.rtc.reset(clock);
        if self.power_button_release.take().is_some() {
            self.gpio
                .set_input(self.layout.power_button_line(), false);
        }
        self.gpio.reset();
        for transport in self.virtio.iter_mut().flatten() {
            transport.reset();
        }
        self.pump_serial_input(clock);
        self.sync_device_lines();
    }

    /// Leave the UART as boot firmware leaves a console it has set up, at
    /// `baud` ([`Pl011::set_up_console`]); host bytes waiting for its
    /// receiver go in, at `clock`.
    pub(crate) fn set_up_console(&mut self, baud: BaudRate, clock: VmClock) {
        let deadline = self.uart.next_deadline();
        self.uart.set_up_console(baud, uart_clocks(clock));
        self.pump_serial_input_since(deadline, clock);
        self.sync_device_lines();
    }

    /// Re-drive the devices' interrupt lines into the GIC.
    pub(crate) fn sync_device_lines(&mut self) {
        let (uart, rtc, gpio) = (self.uart.irq(), self.rtc.irq(), self.gpio.irq());
        let wiring = self.wiring;
        self.gic.set_line(wiring.uart().intid(), uart);
        self.gic.set_line(wiring.rtc().intid(), rtc);
        self.gic.set_line(wiring.gpio().intid(), gpio);
        for transport in self.virtio.iter().flatten() {
            self.gic
                .set_line(wiring.virtio(transport.slot()).intid(), transport.irq());
        }
    }

    /// Drive the timer PPIs.
    pub(crate) fn set_timer_lines(&mut self, lines: TimerLines) {
        self.gic.set_line(PPI_PHYSICAL_TIMER, lines.physical);
        self.gic.set_line(PPI_VIRTUAL_TIMER, lines.virtual_);
        self.gic.set_line(PPI_HYPERVISOR_TIMER, lines.hypervisor);
    }

    /// Let time-driven devices catch up to `clock` and report the next
    /// instant one of them changes an output on its own.
    pub(crate) fn advance_devices(&mut self, clock: VmClock) -> Option<VmInstant> {
        self.deadline_moved = false;
        self.rtc.update(clock);
        self.uart.update(clock);
        if let Some(release) = self.power_button_release {
            if clock.now() >= release {
                self.gpio
                    .set_input(self.layout.power_button_line(), false);
                self.power_button_release = None;
            }
        }
        self.sync_device_lines();
        self.next_deadline()
    }

    /// The next instant a time-driven device changes an output on its own,
    /// without letting any of them catch up first.
    pub(crate) fn next_deadline(&self) -> Option<VmInstant> {
        self.deadlines().into_iter().flatten().min()
    }

    /// The first instant strictly after `now` at which a time-driven device
    /// changes an output on its own. A deadline at or before `now` that the
    /// devices' catching up did not clear changes nothing by itself, so it
    /// is passed over.
    pub(crate) fn next_deadline_after(&self, now: VmInstant) -> Option<VmInstant> {
        self.deadlines()
            .into_iter()
            .flatten()
            .filter(|&at| at > now)
            .min()
    }

    /// Every time-driven device's next deadline, if it has one.
    fn deadlines(&self) -> [Option<VmInstant>; DEADLINE_SOURCES] {
        [
            self.rtc.next_deadline(),
            self.power_button_release,
            self.uart.next_deadline(),
            #[cfg(test)]
            self.stuck_deadline,
        ]
    }

    /// QEMU `gpio_key_set_irq`: press the power button now; it releases on
    /// its own. A press while it is held restarts the hold.
    pub(crate) fn press_power_button(&mut self, clock: VmClock) {
        self.gpio.set_input(self.layout.power_button_line(), true);
        self.power_button_release = Some(clock.after_micros(POWER_BUTTON_HOLD_MICROS));
        self.sync_device_lines();
    }

    /// Hand the UART as many waiting host bytes as its receive FIFO has room
    /// for — what a QEMU character backend does whenever `pl011_can_receive`
    /// says there is room. Run after every UART access and on reset; a host
    /// send fills the FIFO itself before it queues anything. Either way,
    /// whenever the guest looks, the FIFO is full, the host buffer empty, or
    /// the receiver off (`UARTCR.UARTEN` or `RXE` clear). In
    /// loopback the UART consumes and drops what it is handed, as QEMU's
    /// `pl011_receive` does. The bytes arrive at `clock`; an arrival that
    /// moves the UART's receive timeout moves the board's next deadline.
    pub(crate) fn pump_serial_input(&mut self, clock: VmClock) {
        let deadline = self.uart.next_deadline();
        self.pump_serial_input_since(deadline, clock);
    }

    /// [`VirtBoard::pump_serial_input`] after a UART access: the board's
    /// next deadline moves if the UART's receive timeout is no longer
    /// `before`, what it was before the access.
    fn pump_serial_input_since(&mut self, before: Option<VmInstant>, clock: VmClock) {
        while self.uart.can_receive() {
            let Some(byte) = self.serial_input.pop_front() else {
                break;
            };
            match self.uart.receive(byte, uart_clocks(clock)) {
                Received::Taken => {}
                Received::NoRoom => break,
            }
        }
        self.note_uart_deadline(before);
    }

    /// Record that the next deadline has to be worked out again if the
    /// UART's receive timeout is no longer `before`.
    fn note_uart_deadline(&mut self, before: Option<VmInstant>) {
        if self.uart.next_deadline() != before {
            self.deadline_moved = true;
        }
    }

    /// Queue host bytes for the UART at `clock`, returning how many fit:
    /// each goes straight into the receive FIFO when nothing waits ahead of
    /// it and the FIFO has room, and into the host buffer otherwise.
    pub(crate) fn send_serial(&mut self, bytes: &[u8], clock: VmClock) -> usize {
        let deadline = self.uart.next_deadline();
        let mut taken = 0;
        for &byte in bytes {
            let received = if self.serial_input.is_empty() && self.uart.can_receive() {
                self.uart.receive(byte, uart_clocks(clock))
            } else if self.serial_input.len() < SERIAL_INPUT_CAPACITY {
                self.serial_input.push_back(byte);
                Received::Taken
            } else {
                Received::NoRoom
            };
            match received {
                Received::Taken => {}
                Received::NoRoom => break,
            }
            taken += 1;
        }
        self.note_uart_deadline(deadline);
        self.sync_device_lines();
        taken
    }

    /// How many host bytes [`VirtBoard::send_serial`] would take now: the
    /// host buffer's free space, plus the FIFO's when nothing waits ahead.
    pub(crate) fn serial_input_room(&self) -> usize {
        let buffered = SERIAL_INPUT_CAPACITY - self.serial_input.len();
        if self.serial_input.is_empty() {
            buffered + self.uart.rx_room()
        } else {
            buffered
        }
    }

    /// Guest RAM.
    pub fn ram(&self) -> &[u8] {
        self.storage.ram()
    }

    /// The variable-store flash bank's array.
    pub fn flash1_contents(&self) -> &[u8] {
        self.storage.vars()
    }

    /// Load an access from RAM; `None` unless it lies entirely in RAM.
    #[inline(always)]
    fn load_ram(&self, addr: u64, width: AccessWidth) -> Option<u64> {
        let off = usize::try_from(addr.checked_sub(RAM_BASE)?).ok()?;
        load_le(self.storage.ram(), off, width)
    }

    /// Store an access to RAM; `None`, with nothing written, unless it lies
    /// entirely in RAM.
    #[inline(always)]
    fn store_ram(&mut self, addr: u64, width: AccessWidth, value: u64) -> Option<()> {
        let off = usize::try_from(addr.checked_sub(RAM_BASE)?).ok()?;
        store_le(self.storage.ram_mut(), off, width, value)
    }
}

impl<S: GuestStorage> PhysBus for VirtBoard<S> {
    #[inline]
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        clock: VmClock,
    ) -> Result<u64, BusFault> {
        if let Some(value) = self.load_ram(addr.get(), width) {
            return Ok(value);
        }
        self.read_outside_ram_in_layout(addr, width, clock)
            .map(|value| value & width.value_mask())
    }

    #[inline]
    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        clock: VmClock,
    ) -> Result<(), BusFault> {
        if self.store_ram(addr.get(), width, value).is_some() {
            return Ok(());
        }
        self.write_outside_ram_in_layout(addr, width, value & width.value_mask(), clock)
    }

    /// RAM, whose every writer the processor hears of (its own stores, and
    /// the devices' through the log of what they wrote), and a flash bank in
    /// read-array mode, whose epoch counts the writes to the banks. A bank
    /// in any other mode answers a fetch with its status or query data, and
    /// may change mode as it does.
    #[inline]
    fn fetch_source(&self, addr: PhysAddr) -> FetchSource {
        let a = addr.get();
        if a.wrapping_sub(RAM_BASE) < self.storage.ram().len() as u64 {
            return FetchSource::Stable(CodeEpoch::new(0));
        }
        let reads_array = if addr.offset_in(FLASH0_BASE, FLASH_BANK_SIZE).is_some() {
            self.flash0.reads_array()
        } else if addr.offset_in(FLASH1_BASE, FLASH_BANK_SIZE).is_some() {
            self.flash1.reads_array()
        } else {
            false
        };
        if reads_array {
            FetchSource::Stable(CodeEpoch::new(self.flash_epoch))
        } else {
            FetchSource::Volatile
        }
    }

    #[inline(always)]
    fn devices_touched(&self) -> bool {
        self.touched
    }
}

impl<S: GuestStorage> VirtBoard<S> {
    /// A read outside RAM, the devices where the board's layout has them.
    #[inline(never)]
    fn read_outside_ram_in_layout(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        clock: VmClock,
    ) -> Result<u64, BusFault> {
        match self.layout {
            DeviceLayout::Board => self.read_outside_ram(BOARD_WINDOWS, addr, width, clock),
            DeviceLayout::QemuVirt => self.read_outside_ram(QEMU_VIRT_WINDOWS, addr, width, clock),
        }
    }

    /// A read outside RAM, the devices at `w`: inlined into each arm of
    /// [`VirtBoard::read_outside_ram_in_layout`] with that layout's windows
    /// as constants, so that the decode compares against immediates.
    #[inline(always)]
    fn read_outside_ram(
        &mut self,
        w: DeviceWindows,
        addr: PhysAddr,
        width: AccessWidth,
        clock: VmClock,
    ) -> Result<u64, BusFault> {
        // A flash read changes at most the bank's own mode, back to read
        // array — which a fetch from the bank was not reading, or it would
        // be in that mode already.
        if let Some(off) = addr.offset_in(FLASH0_BASE, FLASH_BANK_SIZE) {
            return Ok(self.flash0.read(self.storage.code(), off, width));
        }
        if let Some(off) = addr.offset_in(FLASH1_BASE, FLASH_BANK_SIZE) {
            return Ok(self.flash1.read(self.storage.vars(), off, width));
        }
        self.touched = true;
        if let Some(off) = w.gic.dist_window().offset_of(addr) {
            return Ok(self.gic.dist_read(off, width));
        }
        if let Some(off) = w.gic.cpu_window().offset_of(addr) {
            return Ok(self.gic.cpu_read(off, width));
        }
        if let Some(off) = w.gic.v2m_window().offset_of(addr) {
            return Ok(self.gic.v2m_read(off, width));
        }
        match self.gic.extensions() {
            GicExtensions::Virtualization => {
                if let Some(off) = w.gic.hyp_window().offset_of(addr) {
                    return Ok(self.gic.hyp_read(off, width));
                }
                if let Some(off) = w.gic.vcpu_window().offset_of(addr) {
                    return Ok(self.gic.vcpu_read(off, width));
                }
            }
            GicExtensions::Absent => {}
        }
        if let Some(off) = w.uart_window().offset_of(addr) {
            let deadline = self.uart.next_deadline();
            let v = self.uart.read(off, width);
            self.pump_serial_input_since(deadline, clock);
            self.sync_device_lines();
            return Ok(v);
        }
        if let Some(off) = w.rtc_window().offset_of(addr) {
            return Ok(self.rtc.read(off, width, clock));
        }
        if let Some(off) = w.gpio_window().offset_of(addr) {
            return Ok(self.gpio.read(off, width));
        }
        if let Some(TransportRegister { slot, reg }) = w.virtio.slot_of(addr) {
            let layout = self.mmio_layout;
            let vendor = self.virtio_vendor;
            return Ok(match self.transport_mut(slot) {
                Some(transport) => transport.read(reg, width, vendor),
                None => virtio_mmio::read_empty_transport(reg, width, layout, vendor),
            });
        }
        if w.pcie.contains(addr) {
            return Ok(width.value_mask());
        }
        Err(BusFault::Unmapped)
    }

    /// A write outside RAM, the devices where the board's layout has them.
    #[inline(never)]
    fn write_outside_ram_in_layout(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        clock: VmClock,
    ) -> Result<(), BusFault> {
        match self.layout {
            DeviceLayout::Board => self.write_outside_ram(BOARD_WINDOWS, addr, width, value, clock),
            DeviceLayout::QemuVirt => {
                self.write_outside_ram(QEMU_VIRT_WINDOWS, addr, width, value, clock)
            }
        }
    }

    /// A write outside RAM, the devices at `w`, as
    /// [`VirtBoard::read_outside_ram`] reads.
    #[inline(always)]
    fn write_outside_ram(
        &mut self,
        w: DeviceWindows,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        clock: VmClock,
    ) -> Result<(), BusFault> {
        self.touched = true;
        if let Some(off) = addr.offset_in(FLASH0_BASE, FLASH_BANK_SIZE) {
            self.flash0
                .write(FlashArray::ReadOnly(self.storage.code()), off, width, value);
            self.flash_epoch += 1;
            return Ok(());
        }
        if let Some(off) = addr.offset_in(FLASH1_BASE, FLASH_BANK_SIZE) {
            self.flash1.write(
                FlashArray::Writable(self.storage.vars_mut()),
                off,
                width,
                value,
            );
            self.flash_epoch += 1;
            return Ok(());
        }
        if let Some(off) = w.gic.dist_window().offset_of(addr) {
            self.gic.dist_write(off, width, value);
            return Ok(());
        }
        if let Some(off) = w.gic.cpu_window().offset_of(addr) {
            self.gic.cpu_write(off, width, value);
            return Ok(());
        }
        if let Some(off) = w.gic.v2m_window().offset_of(addr) {
            self.gic.v2m_write(off, width, value);
            return Ok(());
        }
        match self.gic.extensions() {
            GicExtensions::Virtualization => {
                if let Some(off) = w.gic.hyp_window().offset_of(addr) {
                    self.gic.hyp_write(off, width, value);
                    return Ok(());
                }
                if let Some(off) = w.gic.vcpu_window().offset_of(addr) {
                    self.gic.vcpu_write(off, width, value);
                    return Ok(());
                }
            }
            GicExtensions::Absent => {}
        }
        if let Some(off) = w.uart_window().offset_of(addr) {
            let deadline = self.uart.next_deadline();
            self.uart.write(off, width, value, uart_clocks(clock));
            self.pump_serial_input_since(deadline, clock);
            self.sync_device_lines();
            return Ok(());
        }
        if let Some(off) = w.rtc_window().offset_of(addr) {
            if self.rtc.write(off, width, value, clock) {
                self.deadline_moved = true;
            }
            self.sync_device_lines();
            return Ok(());
        }
        if let Some(off) = w.gpio_window().offset_of(addr) {
            self.gpio.write(off, width, value);
            self.sync_device_lines();
            return Ok(());
        }
        if let Some(TransportRegister { slot, reg }) = w.virtio.slot_of(addr) {
            if let Some(transport) = self.virtio.iter_mut().flatten().find(|t| t.slot() == slot) {
                let mut ram = GuestRam::new(self.storage.ram_mut());
                transport.write(&mut ram, reg, width, value);
                self.dma.merge(&ram.written());
                self.sync_device_lines();
            }
            return Ok(());
        }
        if w.pcie.contains(addr) {
            return Ok(());
        }
        Err(BusFault::Unmapped)
    }
}
