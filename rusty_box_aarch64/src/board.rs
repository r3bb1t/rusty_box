//! The `virt` board: QEMU's memory map, its devices, and the wiring of their
//! interrupt lines into the GIC.
//!
//! Provenance (R7): QEMU `hw/arm/virt.c`, `base_memmap`, `extended_memmap`
//! and `a15irqmap`, for a machine with `highmem` on (so the PCIe ECAM sits at
//! 0x40_1000_0000, as the device tree this board is built with says). The
//! PCIe host bridge has no devices on it here — QEMU puts its own host-bridge
//! function at 00:00.0 — so configuration reads find nothing and the I/O and
//! memory windows answer like an empty bus (all ones, writes ignored).

use rusty_box_core::{VmClock, VmInstant};

use crate::bus::{le_value, store_le, AccessWidth, BusFault, PhysAddr, PhysBus, RegOffset};
use crate::devices::cfi_flash::CfiFlash;
use crate::devices::fw_cfg::FwCfg;
use crate::devices::gicv2::GicV2;
use crate::devices::pl011::Pl011;
use crate::devices::pl031::Pl031;
use crate::devices::pl061::Pl061;
use crate::devices::virtio_mmio;

pub const FLASH0_BASE: u64 = 0x0000_0000;
pub const FLASH_BANK_SIZE: u64 = 0x0400_0000;
pub const FLASH1_BASE: u64 = FLASH0_BASE + FLASH_BANK_SIZE;
pub const GIC_DIST_BASE: u64 = 0x0800_0000;
pub const GIC_DIST_SIZE: u64 = 0x1_0000;
pub const GIC_CPU_BASE: u64 = 0x0801_0000;
pub const GIC_CPU_SIZE: u64 = 0x1_0000;
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

/// GIC interrupt IDs of the board's devices (SPI number + 32, QEMU
/// `a15irqmap`) and of the timer PPIs (the device tree's `timer` node).
pub const IRQ_UART0: u32 = 32 + 1;
pub const IRQ_RTC: u32 = 32 + 2;
pub const IRQ_GPIO: u32 = 32 + 7;
pub const PPI_VIRTUAL_TIMER: u32 = 16 + 11;
pub const PPI_PHYSICAL_TIMER: u32 = 16 + 14;

/// Everything on the board's bus.
pub struct VirtBoard<'m> {
    ram: &'m mut [u8],
    pub(crate) flash0: CfiFlash<'m>,
    pub(crate) flash1: CfiFlash<'m>,
    pub(crate) gic: GicV2,
    pub(crate) uart: Pl011,
    pub(crate) rtc: Pl031,
    pub(crate) gpio: Pl061,
    pub(crate) fw_cfg: FwCfg,
}

impl<'m> VirtBoard<'m> {
    pub(crate) fn new(ram: &'m mut [u8], flash0: CfiFlash<'m>, flash1: CfiFlash<'m>, rtc_epoch_seconds: u64) -> Self {
        Self {
            ram,
            flash0,
            flash1,
            gic: GicV2::new(),
            uart: Pl011::new(),
            rtc: Pl031::new(rtc_epoch_seconds),
            gpio: Pl061::new(),
            fw_cfg: FwCfg::new(),
        }
    }

    /// Guest RAM.
    pub fn ram(&self) -> &[u8] {
        self.ram
    }

    /// The variable-store flash bank's contents.
    pub fn flash1_contents(&self) -> &[u8] {
        self.flash1.contents()
    }

    /// Re-drive the devices' interrupt lines into the GIC.
    pub(crate) fn sync_device_lines(&mut self) {
        let (uart, rtc, gpio) = (self.uart.irq(), self.rtc.irq(), self.gpio.irq());
        self.gic.set_line(IRQ_UART0, uart);
        self.gic.set_line(IRQ_RTC, rtc);
        self.gic.set_line(IRQ_GPIO, gpio);
    }

    /// Drive the timer PPIs.
    pub(crate) fn set_timer_lines(&mut self, physical: bool, virtual_: bool) {
        self.gic.set_line(PPI_PHYSICAL_TIMER, physical);
        self.gic.set_line(PPI_VIRTUAL_TIMER, virtual_);
    }

    /// Let time-driven devices catch up to `clock` and report the next
    /// instant one of them changes an output on its own.
    pub(crate) fn advance_devices(&mut self, clock: VmClock) -> Option<VmInstant> {
        self.rtc.update(clock);
        self.sync_device_lines();
        self.rtc.next_deadline(clock)
    }

    /// The RAM bytes an access covers, if it lies entirely in RAM.
    #[inline(always)]
    fn ram_bytes(&self, addr: u64, width: AccessWidth) -> Option<&[u8]> {
        let off = addr.checked_sub(RAM_BASE)? as usize;
        self.ram.get(off..off.checked_add(width.bytes() as usize)?)
    }

    #[inline(always)]
    fn ram_bytes_mut(&mut self, addr: u64, width: AccessWidth) -> Option<&mut [u8]> {
        let off = addr.checked_sub(RAM_BASE)? as usize;
        self.ram.get_mut(off..off.checked_add(width.bytes() as usize)?)
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

impl PhysBus for VirtBoard<'_> {
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
}

impl VirtBoard<'_> {
    #[inline(never)]
    fn read_outside_ram(&mut self, addr: PhysAddr, width: AccessWidth, clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get();
        if let Some(off) = addr.offset_in(FLASH0_BASE, self.flash0.len()) {
            return Ok(self.flash0.read(off, width));
        }
        if let Some(off) = addr.offset_in(FLASH1_BASE, self.flash1.len()) {
            return Ok(self.flash1.read(off, width));
        }
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
            let v = self.uart.read(off, width);
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
            return Ok(virtio_mmio::read_empty_transport(
                RegOffset::new(off.get() % virtio_mmio::TRANSPORT_SIZE),
                width,
            ));
        }
        if in_pci_window(a) {
            return Ok(width.value_mask());
        }
        Err(BusFault::Unmapped)
    }

    #[inline(never)]
    fn write_outside_ram(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, clock: VmClock) -> Result<(), BusFault> {
        let a = addr.get();
        if let Some(off) = addr.offset_in(FLASH0_BASE, self.flash0.len()) {
            self.flash0.write(off, width, value);
            return Ok(());
        }
        if let Some(off) = addr.offset_in(FLASH1_BASE, self.flash1.len()) {
            self.flash1.write(off, width, value);
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
            self.uart.write(off, width, value);
            self.sync_device_lines();
            return Ok(());
        }
        if let Some(off) = addr.offset_in(RTC_BASE, PRIMECELL_SIZE) {
            self.rtc.write(off, width, value, clock);
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
        if addr
            .offset_in(VIRTIO_MMIO_BASE, virtio_mmio::TRANSPORT_SIZE * virtio_mmio::TRANSPORT_COUNT)
            .is_some()
            || in_pci_window(a)
        {
            return Ok(());
        }
        Err(BusFault::Unmapped)
    }
}
