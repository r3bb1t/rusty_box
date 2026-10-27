//! The `virt` board: QEMU's memory map, its devices, and the wiring of their
//! interrupt lines into the GIC.
//!
//! Provenance (R7): QEMU `hw/arm/virt.c`, `base_memmap`, `extended_memmap`
//! and `a15irqmap`, for a machine with `highmem` on (so the PCIe ECAM sits at
//! 0x40_1000_0000, as the device tree this board is built with says). The
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
//! The power button is QEMU's `gpio-key` on GPIO line 3: a press drives the
//! line high and a timer releases it 100 ms of guest time later
//! (`hw/gpio/gpio_key.c`, `GPIO_KEY_LATENCY`).

use rusty_box_core::{ClockHz, RingBuffer, VmClock, VmInstant};
use rusty_box_devices::display::Resolution;

use crate::bus::{le_value, store_le, AccessWidth, BusFault, CodeEpoch, FetchSource, PhysAddr, PhysBus, RegOffset, Writes};
use crate::devices::cfi_flash::{CfiFlash, FlashArray};
use crate::devices::fw_cfg::FwCfg;
use crate::devices::gicv2::GicV2;
use crate::devices::pl011::{Pl011, UartClocks};
use crate::devices::pl031::Pl031;
use crate::devices::pl061::{Pl061, POWER_BUTTON_LINE};
#[cfg(feature = "alloc")]
use crate::devices::virtio::block::Block;
use crate::devices::virtio::input::{Input, InputKind};
use crate::devices::virtio::queue::GuestRam;
#[cfg(feature = "alloc")]
use crate::devices::virtio::gpu::Gpu;
#[cfg(feature = "alloc")]
use crate::disk::Disk;
use crate::devices::virtio::{Device, Transport, Vdev, MAX_DEVICES};
use crate::devices::virtio_mmio::{self, Layout};
use crate::storage::GuestStorage;

pub const FLASH0_BASE: u64 = 0x0000_0000;
pub const FLASH_BANK_SIZE: u64 = 0x0400_0000;
pub const FLASH1_BASE: u64 = FLASH0_BASE + FLASH_BANK_SIZE;
pub const GIC_DIST_BASE: u64 = 0x0800_0000;
/// The distributor's region (QEMU `gic_init_irqs_and_mmio`'s `gic_dist`).
/// `virt` reserves 64 KiB for it; the rest of that window is unassigned.
pub const GIC_DIST_SIZE: u64 = 0x1000;
pub const GIC_CPU_BASE: u64 = 0x0801_0000;
/// The CPU interface's region (QEMU's GICv2 `gic_cpu`, which reaches
/// `GICC_DIR` at 0x1000). `virt` reserves 64 KiB for it; the rest of that
/// window is unassigned.
pub const GIC_CPU_SIZE: u64 = 0x2000;
pub const GIC_V2M_BASE: u64 = 0x0802_0000;
pub const GIC_V2M_SIZE: u64 = 0x1000;
pub const UART0_BASE: u64 = 0x0900_0000;
pub const RTC_BASE: u64 = 0x0901_0000;
pub const FW_CFG_BASE: u64 = 0x0902_0000;
pub const FW_CFG_SIZE: u64 = 0x18;
pub const GPIO_BASE: u64 = 0x0903_0000;
pub const PRIMECELL_SIZE: u64 = 0x1000;
pub const VIRTIO_MMIO_BASE: u64 = 0x0A00_0000;
pub const PCIE_MMIO_BASE: u64 = 0x1000_0000;
pub const PCIE_MMIO_SIZE: u64 = 0x2EFF_0000;
pub const PCIE_PIO_BASE: u64 = 0x3EFF_0000;
pub const PCIE_PIO_SIZE: u64 = 0x1_0000;
pub const RAM_BASE: u64 = 0x4000_0000;
pub const PCIE_ECAM_BASE: u64 = 0x40_1000_0000;
pub const PCIE_ECAM_SIZE: u64 = 0x1000_0000;
pub const PCIE_MMIO_HIGH_BASE: u64 = 0x80_0000_0000;
pub const PCIE_MMIO_HIGH_SIZE: u64 = 0x80_0000_0000;

/// The most RAM the board can map: everything between the base of RAM and
/// the PCIe configuration window above it.
pub const RAM_LIMIT: u64 = PCIE_ECAM_BASE - RAM_BASE;

/// GIC interrupt IDs of the board's devices (SPI number + 32, QEMU
/// `a15irqmap`) and of the timer PPIs (the device tree's `timer` node).
pub const IRQ_UART0: u32 = 32 + 1;
pub const IRQ_RTC: u32 = 32 + 2;
pub const IRQ_GPIO: u32 = 32 + 7;
/// Virtio-mmio transport `n` raises SPI `16 + n` (QEMU `a15irqmap`
/// `VIRT_MMIO`).
pub const IRQ_VIRTIO_BASE: u32 = 32 + 16;
pub const PPI_VIRTUAL_TIMER: u32 = 16 + 11;
pub const PPI_PHYSICAL_TIMER: u32 = 16 + 14;

/// The PL011's reference clock, `UARTCLK`: the device tree's `/apb-pclk`
/// fixed clock, 24 MHz, which its `pl011@9000000` node names as `uartclk`
/// (QEMU `hw/arm/virt.c` `create_fdt`).
pub const UART_CLOCK: ClockHz = match ClockHz::new(24_000_000) {
    Some(hz) => hz,
    None => ClockHz::BOCHS_DEFAULT,
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

/// The display and input devices plugged into the board, in the order QEMU
/// would be given them as `-device` options: the display, the keyboard, the
/// tablet. Each takes the highest transport the ones before it left free.
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
        u8::from(self.display.is_some()) | (u8::from(self.keyboard == Plugged::Yes) << 1) | (u8::from(self.tablet == Plugged::Yes) << 2)
    }
}

/// Everything on the board's bus.
pub struct VirtBoard<S> {
    pub(crate) storage: S,
    pub(crate) flash0: CfiFlash,
    pub(crate) flash1: CfiFlash,
    pub(crate) gic: GicV2,
    pub(crate) uart: Pl011,
    pub(crate) rtc: Pl031,
    pub(crate) gpio: Pl061,
    pub(crate) fw_cfg: FwCfg,
    /// Host bytes waiting for room in the UART's receive FIFO.
    pub(crate) serial_input: RingBuffer<u8, SERIAL_INPUT_CAPACITY>,
    /// When the pressed power button is released, if it is held.
    pub(crate) power_button_release: Option<VmInstant>,
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
    /// The virtio devices, each behind its transport.
    pub(crate) virtio: [Option<Transport>; MAX_DEVICES],
    /// The register layout of every transport, plugged or empty.
    pub(crate) mmio_layout: Layout,
}

impl<S: GuestStorage> VirtBoard<S> {
    pub(crate) fn new(storage: S, rtc_epoch_seconds: u64, plugs: Plugs) -> Self {
        let devices = plugs.devices;
        let mut virtio: [Option<Transport>; MAX_DEVICES] = [None, None, None, None];
        let mut next_slot = (virtio_mmio::TRANSPORT_COUNT - 1) as u8;
        let mut plug = |device: Device, virtio: &mut [Option<Transport>; MAX_DEVICES]| {
            if let Some(free) = virtio.iter_mut().find(|t| t.is_none()) {
                *free = Some(Transport::new(next_slot, device));
                next_slot -= 1;
            }
        };
        #[cfg(feature = "alloc")]
        if let Some(size) = devices.display {
            plug(Device::Gpu(Gpu::new(size)), &mut virtio);
        }
        if devices.keyboard == Plugged::Yes {
            plug(Device::Input(Input::new(InputKind::Keyboard)), &mut virtio);
        }
        if devices.tablet == Plugged::Yes {
            plug(Device::Input(Input::new(InputKind::Tablet)), &mut virtio);
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
            gic: GicV2::new(),
            uart: Pl011::new(),
            rtc: Pl031::new(rtc_epoch_seconds),
            gpio: Pl061::new(),
            fw_cfg: FwCfg::new(),
            serial_input: RingBuffer::new(),
            power_button_release: None,
            deadline_moved: false,
            touched: false,
            flash_epoch: 0,
            dma: Writes::NONE,
            virtio,
            mmio_layout,
        }
    }

    /// The transport at `slot`, if a device is plugged into it.
    fn transport_mut(&mut self, slot: u64) -> Option<&mut Transport> {
        self.virtio.iter_mut().flatten().find(|t| u64::from(t.slot()) == slot)
    }

    /// The virtio input device of `kind`, if one is plugged in.
    pub(crate) fn has_input(&self, kind: InputKind) -> bool {
        self.virtio.iter().flatten().any(|t| t.input_kind() == Some(kind))
    }

    /// Have the virtio input device of `kind` send the guest what `send`
    /// sends. `false` when there is no such device or it delivered nothing.
    pub(crate) fn input_event<F: FnOnce(&mut Input, &mut Vdev<'_, '_>) -> bool>(&mut self, kind: InputKind, send: F) -> bool {
        let Some(transport) = self.virtio.iter_mut().flatten().find(|t| t.input_kind() == Some(kind)) else {
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
        self.virtio.iter_mut().flatten().find_map(|t| match &mut t.device {
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
        self.virtio.iter_mut().flatten().find_map(|t| match &mut t.device {
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

    /// QEMU's system reset of the board's devices. The RTC keeps counting
    /// and keeps its alarm (QEMU's `pl031_class_init` installs no reset
    /// handler). The UART empties its receive FIFO, as `pl011_reset` does;
    /// what the guest transmitted stays for the host to drain, and host bytes
    /// still waiting for the FIFO stay queued for it, which takes them once
    /// the guest switches the receiver back on, at `clock`. Memory is
    /// untouched.
    pub(crate) fn reset_devices(&mut self, clock: VmClock) {
        self.flash0.reset();
        self.flash1.reset();
        self.flash_epoch += 1;
        self.gic = GicV2::new();
        self.uart.reset();
        self.gpio = Pl061::new();
        self.fw_cfg = FwCfg::new();
        self.power_button_release = None;
        for transport in self.virtio.iter_mut().flatten() {
            transport.reset();
        }
        self.pump_serial_input(clock);
        self.sync_device_lines();
    }

    /// Leave the UART as boot firmware leaves a console it has set up, at
    /// `baud` ([`Pl011::set_up_console`]); host bytes waiting for its
    /// receiver go in, at `clock`.
    pub(crate) fn set_up_console(&mut self, baud: u32, clock: VmClock) {
        self.uart.set_up_console(baud, uart_clocks(clock));
        self.pump_serial_input(clock);
        self.sync_device_lines();
    }

    /// Re-drive the devices' interrupt lines into the GIC.
    pub(crate) fn sync_device_lines(&mut self) {
        let (uart, rtc, gpio) = (self.uart.irq(), self.rtc.irq(), self.gpio.irq());
        self.gic.set_line(IRQ_UART0, uart);
        self.gic.set_line(IRQ_RTC, rtc);
        self.gic.set_line(IRQ_GPIO, gpio);
        for transport in self.virtio.iter().flatten() {
            self.gic.set_line(IRQ_VIRTIO_BASE + u32::from(transport.slot()), transport.irq());
        }
    }

    /// Drive the timer PPIs.
    pub(crate) fn set_timer_lines(&mut self, physical: bool, virtual_: bool) {
        self.gic.set_line(PPI_PHYSICAL_TIMER, physical);
        self.gic.set_line(PPI_VIRTUAL_TIMER, virtual_);
    }

    /// Let time-driven devices catch up to `clock` and report the next
    /// instant one of them changes an output on its own.
    pub(crate) fn advance_devices(&mut self, clock: VmClock) -> Option<VmInstant> {
        self.deadline_moved = false;
        self.rtc.update(clock);
        self.uart.update(clock);
        if let Some(release) = self.power_button_release {
            if clock.now() >= release {
                self.gpio.set_input(POWER_BUTTON_LINE, false);
                self.power_button_release = None;
            }
        }
        self.sync_device_lines();
        self.next_deadline()
    }

    /// The next instant a time-driven device changes an output on its own,
    /// without letting any of them catch up first.
    pub(crate) fn next_deadline(&self) -> Option<VmInstant> {
        [self.rtc.next_deadline(), self.power_button_release, self.uart.next_deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    /// QEMU `gpio_key_set_irq`: press the power button now; it releases on
    /// its own. A press while it is held restarts the hold.
    pub(crate) fn press_power_button(&mut self, clock: VmClock) {
        self.gpio.set_input(POWER_BUTTON_LINE, true);
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
        while self.uart.can_receive() {
            let Some(byte) = self.serial_input.pop_front() else {
                break;
            };
            if !self.uart.receive(byte, uart_clocks(clock)) {
                break;
            }
        }
        self.note_uart_deadline(deadline);
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
            let accepted = if self.serial_input.is_empty() && self.uart.can_receive() {
                self.uart.receive(byte, uart_clocks(clock))
            } else if self.serial_input.len() < SERIAL_INPUT_CAPACITY {
                self.serial_input.push_back(byte);
                true
            } else {
                false
            };
            if !accepted {
                break;
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

    /// The RAM bytes an access covers, if it lies entirely in RAM.
    #[inline(always)]
    fn ram_bytes(&self, addr: u64, width: AccessWidth) -> Option<&[u8]> {
        let off = addr.checked_sub(RAM_BASE)? as usize;
        self.storage.ram().get(off..off.checked_add(width.bytes() as usize)?)
    }

    #[inline(always)]
    fn ram_bytes_mut(&mut self, addr: u64, width: AccessWidth) -> Option<&mut [u8]> {
        let off = addr.checked_sub(RAM_BASE)? as usize;
        self.storage.ram_mut().get_mut(off..off.checked_add(width.bytes() as usize)?)
    }
}

/// An access inside a window that answers like an empty PCI bus.
#[inline]
fn in_pci_window(a: u64) -> bool {
    (PCIE_MMIO_BASE..PCIE_MMIO_BASE + PCIE_MMIO_SIZE).contains(&a)
        || (PCIE_PIO_BASE..PCIE_PIO_BASE + PCIE_PIO_SIZE).contains(&a)
        || (PCIE_ECAM_BASE..PCIE_ECAM_BASE + PCIE_ECAM_SIZE).contains(&a)
        || (PCIE_MMIO_HIGH_BASE..PCIE_MMIO_HIGH_BASE + PCIE_MMIO_HIGH_SIZE).contains(&a)
}

impl<S: GuestStorage> PhysBus for VirtBoard<S> {
    #[inline]
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get();
        if let Some(bytes) = self.ram_bytes(a, width) {
            return Ok(le_value(bytes));
        }
        self.read_outside_ram(addr, width, clock)
    }

    #[inline]
    fn write(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, clock: VmClock) -> Result<(), BusFault> {
        if let Some(bytes) = self.ram_bytes_mut(addr.get(), width) {
            store_le(bytes, value);
            return Ok(());
        }
        self.write_outside_ram(addr, width, value, clock)
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
    #[inline(never)]
    fn read_outside_ram(&mut self, addr: PhysAddr, width: AccessWidth, clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get();
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
        if let Some(off) = addr.offset_in(GIC_DIST_BASE, GIC_DIST_SIZE) {
            return Ok(self.gic.dist_read(off, width));
        }
        if let Some(off) = addr.offset_in(GIC_CPU_BASE, GIC_CPU_SIZE) {
            return Ok(self.gic.cpu_read(off, width));
        }
        if let Some(off) = addr.offset_in(GIC_V2M_BASE, GIC_V2M_SIZE) {
            return Ok(self.gic.v2m_read(off, width));
        }
        if let Some(off) = addr.offset_in(UART0_BASE, PRIMECELL_SIZE) {
            let deadline = self.uart.next_deadline();
            let v = self.uart.read(off, width);
            self.pump_serial_input(clock);
            self.note_uart_deadline(deadline);
            self.sync_device_lines();
            return Ok(v);
        }
        if let Some(off) = addr.offset_in(RTC_BASE, PRIMECELL_SIZE) {
            return Ok(self.rtc.read(off, width, clock));
        }
        if let Some(off) = addr.offset_in(FW_CFG_BASE, FW_CFG_SIZE) {
            return Ok(self.fw_cfg.read(off, width));
        }
        if let Some(off) = addr.offset_in(GPIO_BASE, PRIMECELL_SIZE) {
            return Ok(self.gpio.read(off, width));
        }
        if let Some(off) = addr.offset_in(
            VIRTIO_MMIO_BASE,
            virtio_mmio::TRANSPORT_SIZE * virtio_mmio::TRANSPORT_COUNT,
        ) {
            let reg = RegOffset::new(off.get() % virtio_mmio::TRANSPORT_SIZE);
            let layout = self.mmio_layout;
            return Ok(match self.transport_mut(off.get() / virtio_mmio::TRANSPORT_SIZE) {
                Some(transport) => transport.read(reg, width),
                None => virtio_mmio::read_empty_transport(reg, width, layout),
            });
        }
        if in_pci_window(a) {
            return Ok(width.value_mask());
        }
        Err(BusFault::Unmapped)
    }

    #[inline(never)]
    fn write_outside_ram(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, clock: VmClock) -> Result<(), BusFault> {
        self.touched = true;
        let a = addr.get();
        if let Some(off) = addr.offset_in(FLASH0_BASE, FLASH_BANK_SIZE) {
            self.flash0.write(FlashArray::ReadOnly(self.storage.code()), off, width, value);
            self.flash_epoch += 1;
            return Ok(());
        }
        if let Some(off) = addr.offset_in(FLASH1_BASE, FLASH_BANK_SIZE) {
            self.flash1.write(FlashArray::Writable(self.storage.vars_mut()), off, width, value);
            self.flash_epoch += 1;
            return Ok(());
        }
        if let Some(off) = addr.offset_in(GIC_DIST_BASE, GIC_DIST_SIZE) {
            self.gic.dist_write(off, width, value);
            return Ok(());
        }
        if let Some(off) = addr.offset_in(GIC_CPU_BASE, GIC_CPU_SIZE) {
            self.gic.cpu_write(off, width, value);
            return Ok(());
        }
        if let Some(off) = addr.offset_in(GIC_V2M_BASE, GIC_V2M_SIZE) {
            self.gic.v2m_write(off, width, value);
            return Ok(());
        }
        if let Some(off) = addr.offset_in(UART0_BASE, PRIMECELL_SIZE) {
            let deadline = self.uart.next_deadline();
            self.uart.write(off, width, value, uart_clocks(clock));
            self.pump_serial_input(clock);
            self.note_uart_deadline(deadline);
            self.sync_device_lines();
            return Ok(());
        }
        if let Some(off) = addr.offset_in(RTC_BASE, PRIMECELL_SIZE) {
            if self.rtc.write(off, width, value, clock) {
                self.deadline_moved = true;
            }
            self.sync_device_lines();
            return Ok(());
        }
        if let Some(off) = addr.offset_in(FW_CFG_BASE, FW_CFG_SIZE) {
            self.fw_cfg.write(off, width, value);
            return Ok(());
        }
        if let Some(off) = addr.offset_in(GPIO_BASE, PRIMECELL_SIZE) {
            self.gpio.write(off, width, value);
            self.sync_device_lines();
            return Ok(());
        }
        if let Some(off) = addr.offset_in(VIRTIO_MMIO_BASE, virtio_mmio::TRANSPORT_SIZE * virtio_mmio::TRANSPORT_COUNT) {
            let slot = off.get() / virtio_mmio::TRANSPORT_SIZE;
            let reg = RegOffset::new(off.get() % virtio_mmio::TRANSPORT_SIZE);
            if let Some(transport) = self.virtio.iter_mut().flatten().find(|t| u64::from(t.slot()) == slot) {
                let mut ram = GuestRam::new(self.storage.ram_mut());
                transport.write(&mut ram, reg, width, value);
                self.dma.merge(&ram.written());
                self.sync_device_lines();
            }
            return Ok(());
        }
        if in_pci_window(a) {
            return Ok(());
        }
        Err(BusFault::Unmapped)
    }
}
