//! Where the board's devices sit in the physical address space.
//!
//! Provenance (R7): RAM at 0x4000_0000 and the two flash banks at 0 and
//! 0x0400_0000 are where stock EDK2 `ArmVirtQemu` is built to find them
//! (README, "QEMU fingerprints left"), so every layout keeps them; EDK2 and
//! Linux find every other device through the device tree. The board's own
//! layout ([`DeviceLayout::Board`]) follows Arm's *Principles of ARM Memory
//! Maps* (DEN0001C): its devices in the 32-bit map's region for ROM, RAM
//! and SoC I/O (4.1.2), each in 64 KiB pages of its own (P29), from the
//! first page after the flash; the GIC's frames as the GIC-400 lays them
//! out in one block (DDI 0471B Table 3-1), the GIC-400 being one
//! peripheral, behind its one AXI4 slave interface (DDI 0471B 1.3), whose
//! 32 KiB block takes one page; the PCIe host's configuration
//! space and 64-bit window in the 40-bit map's mapped I/O, from 256 GiB
//! (DEN0001C 4.3.2), and its 32-bit window, which the 32-bit map would put in
//! 1 GiB to 2 GiB where this board has RAM, in the top half of the I/O
//! region below RAM, each window aligned to its size. QEMU's `virt` layout
//! ([`DeviceLayout::QemuVirt`], `hw/arm/virt.c` `base_memmap` and
//! `extended_memmap` with `highmem`) is a host's choice, for a guest whose
//! device tree or firmware variable store was made for QEMU's addresses.

use core::fmt;

use crate::bus::{PhysAddr, RegOffset};
use crate::devices::virtio_mmio::{TRANSPORT_COUNT, TRANSPORT_SIZE};

/// A device's register window: `size` bytes from `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Window {
    pub base: PhysAddr,
    pub size: u64,
}

impl Window {
    pub const fn new(base: PhysAddr, size: u64) -> Self {
        Self { base, size }
    }

    /// `addr`'s offset in the window, if it is in it.
    #[inline]
    pub const fn offset_of(self, addr: PhysAddr) -> Option<RegOffset> {
        addr.offset_in(self.base.get(), self.size)
    }

    /// Whether `addr` is in the window.
    #[inline]
    pub const fn contains(self, addr: PhysAddr) -> bool {
        self.offset_of(addr).is_some()
    }

    /// The first address past it.
    pub const fn end(self) -> PhysAddr {
        PhysAddr::new(self.base.get() + self.size)
    }
}

/// The GIC's frames, each the size the GIC decodes ([`GicWindows::DIST_SIZE`]
/// and the others), and the GICv2m frame's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GicWindows {
    pub dist: PhysAddr,
    pub cpu: PhysAddr,
    pub v2m: PhysAddr,
    /// The virtual interface control registers, with EL2.
    pub hyp: PhysAddr,
    /// The virtual CPU interface, with EL2.
    pub vcpu: PhysAddr,
}

impl GicWindows {
    /// The distributor's registers (QEMU `gic_init_irqs_and_mmio`'s
    /// `gic_dist`; DDI 0471B Table 3-1).
    pub const DIST_SIZE: u64 = 0x1000;
    /// The CPU interface's registers, to `GICC_DIR` at 0x1000.
    pub const CPU_SIZE: u64 = 0x2000;
    /// The GICv2m frame's registers (BSA DEN0094C I.7).
    pub const V2M_SIZE: u64 = 0x1000;
    /// The virtual interface control registers (`GICH_*`, IHI 0048B
    /// §5.3): the common base and the processor-specific bases IHI 0048B
    /// §5.2.7 requires, 4 KiB each, as the GIC-400 places them (DDI 0471B
    /// §3.2).
    pub const HYP_SIZE: u64 = 0x2000;
    /// The virtual CPU interface (`GICV_*`, §5.5), to `GICV_DIR` at
    /// 0x1000.
    pub const VCPU_SIZE: u64 = 0x2000;

    pub const fn dist_window(self) -> Window {
        Window {
            base: self.dist,
            size: Self::DIST_SIZE,
        }
    }

    pub const fn cpu_window(self) -> Window {
        Window {
            base: self.cpu,
            size: Self::CPU_SIZE,
        }
    }

    pub const fn v2m_window(self) -> Window {
        Window {
            base: self.v2m,
            size: Self::V2M_SIZE,
        }
    }

    pub const fn hyp_window(self) -> Window {
        Window {
            base: self.hyp,
            size: Self::HYP_SIZE,
        }
    }

    pub const fn vcpu_window(self) -> Window {
        Window {
            base: self.vcpu,
            size: Self::VCPU_SIZE,
        }
    }
}

/// How far apart the virtio-mmio transports are: a power of two no smaller
/// than a transport's [`TRANSPORT_SIZE`] registers, so that an address's
/// transport and register are its offset's high and low bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportStride {
    shift: u8,
}

impl TransportStride {
    /// The stride of `bytes`, if it is a power of two of at least
    /// [`TRANSPORT_SIZE`].
    pub const fn new(bytes: u64) -> Option<Self> {
        if bytes.is_power_of_two() && bytes >= TRANSPORT_SIZE {
            Some(Self {
                shift: bytes.trailing_zeros() as u8,
            })
        } else {
            None
        }
    }

    pub const fn bytes(self) -> u64 {
        1 << self.shift
    }
}

/// A stride known to be one at compile time.
const fn stride(bytes: u64) -> TransportStride {
    match TransportStride::new(bytes) {
        Some(stride) => stride,
        None => panic!("a transport stride is a power of two of a transport at least"),
    }
}

/// The virtio-mmio transports: transport `n`'s registers at `first + n ×
/// stride`, each [`TRANSPORT_SIZE`] bytes, the rest of its stride
/// unassigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VirtioWindows {
    pub first: PhysAddr,
    pub stride: TransportStride,
}

impl VirtioWindows {
    /// Transport `slot`'s registers.
    pub const fn transport(self, slot: u64) -> Window {
        Window {
            base: PhysAddr::new(self.first.get() + (slot << self.stride.shift)),
            size: TRANSPORT_SIZE,
        }
    }

    /// The address range every transport is in.
    pub const fn span(self) -> Window {
        Window {
            base: self.first,
            size: TRANSPORT_COUNT << self.stride.shift,
        }
    }

    /// The transport `addr` is in and its offset in the transport's
    /// registers, if it is in one; an address in the rest of a stride is in
    /// none.
    #[inline]
    pub const fn slot_of(self, addr: PhysAddr) -> Option<TransportRegister> {
        match self.span().offset_of(addr) {
            Some(off) => {
                let reg = off.get() & (self.stride.bytes() - 1);
                if reg < TRANSPORT_SIZE {
                    Some(TransportRegister {
                        slot: off.get() >> self.stride.shift,
                        reg: RegOffset::new(reg),
                    })
                } else {
                    None
                }
            }
            None => None,
        }
    }
}

/// A register of a virtio-mmio transport, by the transport and the offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportRegister {
    pub slot: u64,
    pub reg: RegOffset,
}

/// The PCIe host bridge's windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PcieWindows {
    /// The configuration space (ECAM), a MiB a bus.
    pub ecam: Window,
    /// The I/O space, from I/O address 0.
    pub pio: Window,
    /// The 32-bit memory window, at the same addresses on the bus.
    pub mmio32: Window,
    /// The 64-bit memory window, at the same addresses on the bus.
    pub mmio64: Window,
}

impl PcieWindows {
    /// Whether `addr` is in one of the windows.
    #[inline]
    pub const fn contains(self, addr: PhysAddr) -> bool {
        self.ecam.contains(addr)
            || self.pio.contains(addr)
            || self.mmio32.contains(addr)
            || self.mmio64.contains(addr)
    }
}

/// Where every device of a layout sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceWindows {
    pub gic: GicWindows,
    /// The PL011's, the PL031's and the PL061's registers, each
    /// [`DeviceWindows::PRIMECELL_SIZE`] bytes.
    pub uart: PhysAddr,
    pub rtc: PhysAddr,
    pub gpio: PhysAddr,
    pub virtio: VirtioWindows,
    pub pcie: PcieWindows,
}

impl DeviceWindows {
    /// A PrimeCell's registers, with its identification registers at the
    /// top (DDI 0183F, DDI 0224C, DDI 0190B).
    pub const PRIMECELL_SIZE: u64 = 0x1000;

    pub const fn uart_window(self) -> Window {
        Window {
            base: self.uart,
            size: Self::PRIMECELL_SIZE,
        }
    }

    pub const fn rtc_window(self) -> Window {
        Window {
            base: self.rtc,
            size: Self::PRIMECELL_SIZE,
        }
    }

    pub const fn gpio_window(self) -> Window {
        Window {
            base: self.gpio,
            size: Self::PRIMECELL_SIZE,
        }
    }
}

/// A 64 KiB page, the unit each peripheral of the board's own layout
/// occupies (DEN0001C P29).
const PAGE: u64 = 0x1_0000;
/// The board's own layout's first device page: the first after the two
/// 64 MiB flash banks.
const DEVICES: u64 = 0x0800_0000;
/// Its GIC-400 block (DDI 0471B Table 3-1: the distributor at 0x1000, the
/// CPU interfaces at 0x2000, the virtual interface control at 0x4000 and
/// 0x5000, the virtual CPU interfaces at 0x6000).
const GIC_BLOCK: u64 = DEVICES;

/// The board's own layout.
const BOARD: DeviceWindows = DeviceWindows {
    gic: GicWindows {
        dist: PhysAddr::new(GIC_BLOCK + 0x1000),
        cpu: PhysAddr::new(GIC_BLOCK + 0x2000),
        v2m: PhysAddr::new(DEVICES + PAGE),
        hyp: PhysAddr::new(GIC_BLOCK + 0x4000),
        vcpu: PhysAddr::new(GIC_BLOCK + 0x6000),
    },
    uart: PhysAddr::new(DEVICES + 2 * PAGE),
    rtc: PhysAddr::new(DEVICES + 3 * PAGE),
    gpio: PhysAddr::new(DEVICES + 4 * PAGE),
    virtio: VirtioWindows {
        first: PhysAddr::new(DEVICES + 5 * PAGE),
        stride: stride(PAGE),
    },
    pcie: PcieWindows {
        ecam: Window::new(PhysAddr::new(0x40_0000_0000), 0x1000_0000),
        pio: Window::new(PhysAddr::new(DEVICES + (5 + TRANSPORT_COUNT) * PAGE), PAGE),
        mmio32: Window::new(PhysAddr::new(0x2000_0000), 0x2000_0000),
        mmio64: Window::new(PhysAddr::new(0x60_0000_0000), 0x20_0000_0000),
    },
};

/// QEMU `virt`'s (`base_memmap`, `extended_memmap` with `highmem`).
const QEMU_VIRT: DeviceWindows = DeviceWindows {
    gic: GicWindows {
        dist: PhysAddr::new(0x0800_0000),
        cpu: PhysAddr::new(0x0801_0000),
        v2m: PhysAddr::new(0x0802_0000),
        hyp: PhysAddr::new(0x0803_0000),
        vcpu: PhysAddr::new(0x0804_0000),
    },
    uart: PhysAddr::new(0x0900_0000),
    rtc: PhysAddr::new(0x0901_0000),
    gpio: PhysAddr::new(0x0903_0000),
    virtio: VirtioWindows {
        first: PhysAddr::new(0x0A00_0000),
        stride: stride(TRANSPORT_SIZE),
    },
    pcie: PcieWindows {
        ecam: Window::new(PhysAddr::new(0x40_1000_0000), 0x1000_0000),
        pio: Window::new(PhysAddr::new(0x3EFF_0000), 0x1_0000),
        mmio32: Window::new(PhysAddr::new(0x1000_0000), 0x2EFF_0000),
        mmio64: Window::new(PhysAddr::new(0x80_0000_0000), 0x80_0000_0000),
    },
};

/// Where the board's devices sit, RAM and the flash aside.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceLayout {
    /// The board's own, after Arm's memory map principles (the module's
    /// provenance).
    Board,
    /// QEMU `virt`'s.
    QemuVirt,
}

impl DeviceLayout {
    /// Every layout, in the order a list of them shows.
    pub const ALL: [Self; 2] = [Self::Board, Self::QemuVirt];

    /// The layout a board takes unless its host chooses another.
    pub const DEFAULT: Self = Self::Board;

    /// Its windows.
    pub const fn windows(self) -> DeviceWindows {
        match self {
            Self::Board => BOARD,
            Self::QemuVirt => QEMU_VIRT,
        }
    }

    /// The layout's name for a command line or a configuration file, which
    /// [`str::parse`] reads back.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Board => "board",
            Self::QemuVirt => "qemu-virt",
        }
    }

    /// The layout as a snapshot's machine shape records it, fixed per
    /// layout whatever order the variants are declared in.
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Self::Board => 0,
            Self::QemuVirt => 1,
        }
    }

    /// The first address of the lowest window above RAM in any layout,
    /// which RAM ends below ([`crate::RamSize::MAX`]).
    pub const fn lowest_above_ram() -> PhysAddr {
        let mut lowest = u64::MAX;
        let mut i = 0;
        while i < Self::ALL.len() {
            let pcie = Self::ALL[i].windows().pcie;
            if pcie.ecam.base.get() < lowest {
                lowest = pcie.ecam.base.get();
            }
            if pcie.mmio64.base.get() < lowest {
                lowest = pcie.mmio64.base.get();
            }
            i += 1;
        }
        PhysAddr::new(lowest)
    }
}

impl Default for DeviceLayout {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for DeviceLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl core::str::FromStr for DeviceLayout {
    type Err = UnknownDeviceLayout;

    /// The layout whose [`DeviceLayout::name`] is `name`.
    fn from_str(name: &str) -> Result<Self, UnknownDeviceLayout> {
        Self::ALL
            .into_iter()
            .find(|layout| layout.name() == name)
            .ok_or(UnknownDeviceLayout)
    }
}

/// A name no [`DeviceLayout`] has. It prints the names there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnknownDeviceLayout;

impl fmt::Display for UnknownDeviceLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no device layout has that name; the layouts are")?;
        for (i, layout) in DeviceLayout::ALL.iter().enumerate() {
            let separator = if i == 0 { " " } else { ", " };
            write!(f, "{separator}\"{}\"", layout.name())?;
        }
        Ok(())
    }
}

impl core::error::Error for UnknownDeviceLayout {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every window of a layout, by name.
    fn windows(w: DeviceWindows) -> [(&'static str, Window); 13] {
        [
            ("GICD", w.gic.dist_window()),
            ("GICC", w.gic.cpu_window()),
            ("GICv2m", w.gic.v2m_window()),
            ("GICH", w.gic.hyp_window()),
            ("GICV", w.gic.vcpu_window()),
            ("UART", w.uart_window()),
            ("RTC", w.rtc_window()),
            ("GPIO", w.gpio_window()),
            ("virtio", w.virtio.span()),
            ("ECAM", w.pcie.ecam),
            ("PIO", w.pcie.pio),
            ("MMIO32", w.pcie.mmio32),
            ("MMIO64", w.pcie.mmio64),
        ]
    }

    /// No two windows of a layout overlap, none overlaps the flash
    /// (0 to 128 MiB) or RAM (1 GiB to the lowest window above it), and
    /// the low windows end below RAM.
    #[test]
    fn a_layouts_windows_are_apart_from_each_other_the_flash_and_ram() {
        let ram_base = PhysAddr::new(0x4000_0000);
        let ram = Window::new(ram_base, DeviceLayout::lowest_above_ram().get() - ram_base.get());
        let flash = Window::new(PhysAddr::new(0), 0x0800_0000);
        for layout in DeviceLayout::ALL {
            let all = windows(layout.windows());
            for (i, (a, wa)) in all.iter().enumerate() {
                for (b, wb) in &all[i + 1..] {
                    assert!(
                        wa.end() <= wb.base || wb.end() <= wa.base,
                        "{layout}: {a} and {b} overlap"
                    );
                }
                for (other, w) in [("flash", flash), ("RAM", ram)] {
                    assert!(
                        wa.end() <= w.base || w.end() <= wa.base,
                        "{layout}: {a} overlaps the {other}"
                    );
                }
            }
        }
    }

    /// The board's own layout puts each peripheral in 64 KiB pages of its
    /// own (DEN0001C P29), the GIC's frames as the GIC-400 lays them out in
    /// one block (DDI 0471B Table 3-1), and its PCIe windows aligned to
    /// their sizes, the ECAM and 64-bit window in the 40-bit map's mapped
    /// I/O from 256 GiB (DEN0001C 4.3.2).
    #[test]
    fn the_boards_layout_follows_arms_memory_map_principles() {
        let w = DeviceLayout::Board.windows();
        let block = w.gic.dist.get() - 0x1000;
        assert_eq!(block % PAGE, 0);
        assert_eq!(
            [
                w.gic.dist.get(),
                w.gic.cpu.get(),
                w.gic.hyp.get(),
                w.gic.vcpu.get()
            ],
            [block + 0x1000, block + 0x2000, block + 0x4000, block + 0x6000]
        );
        let pages = [w.gic.v2m, w.uart, w.rtc, w.gpio, w.pcie.pio.base];
        for page in pages {
            assert_eq!(page.get() % PAGE, 0, "{page:?}");
        }
        assert_eq!(w.virtio.stride.bytes(), PAGE);
        assert_eq!(w.virtio.first.get() % PAGE, 0);
        for window in [w.pcie.ecam, w.pcie.mmio32, w.pcie.mmio64] {
            assert_eq!(window.base.get() % window.size, 0, "{window:?}");
        }
        let mapped_io = 0x40_0000_0000..0x80_0000_0000;
        assert!(mapped_io.contains(&w.pcie.ecam.base.get()));
        assert!(w.pcie.mmio64.end().get() <= mapped_io.end);
        assert!(w.pcie.mmio32.end() <= PhysAddr::new(0x4000_0000), "below RAM");
    }

    /// An address of a transport's registers names the transport and the
    /// register; one in the rest of its stride names none.
    #[test]
    fn a_transports_registers_are_its_first_bytes() {
        for layout in DeviceLayout::ALL {
            let v = layout.windows().virtio;
            let stride = v.stride.bytes();
            let at = |offset| PhysAddr::new(v.first.get() + offset);
            assert_eq!(
                v.slot_of(at(3 * stride + 0x70)),
                Some(TransportRegister {
                    slot: 3,
                    reg: RegOffset::new(0x70)
                }),
                "{layout}"
            );
            assert_eq!(
                v.transport(31),
                Window::new(PhysAddr::new(v.first.get() + 31 * stride), TRANSPORT_SIZE),
                "{layout}"
            );
            assert_eq!(v.slot_of(at(stride * TRANSPORT_COUNT)), None, "{layout}");
            assert_eq!(
                v.slot_of(at(31 * stride + TRANSPORT_SIZE - 4)),
                Some(TransportRegister {
                    slot: 31,
                    reg: RegOffset::new(TRANSPORT_SIZE - 4)
                }),
                "{layout}: the last transport's last register"
            );
            if stride > TRANSPORT_SIZE {
                assert_eq!(v.slot_of(at(TRANSPORT_SIZE)), None, "{layout}");
            }
        }
    }

    /// A window holds its first byte to its last and nothing either side,
    /// an address below it as much as one above.
    #[test]
    fn a_window_holds_its_bytes_and_none_either_side() {
        let w = Window::new(PhysAddr::new(0x0800_1000), 0x1000);
        for (addr, held) in [
            (0, None),
            (0x0800_0FFF, None),
            (0x0800_1000, Some(0)),
            (0x0800_1FFF, Some(0xFFF)),
            (0x0800_2000, None),
            (u64::MAX, None),
        ] {
            assert_eq!(w.offset_of(PhysAddr::new(addr)).map(RegOffset::get), held, "{addr:#x}");
        }
    }

    /// A stride is a power of two no smaller than a transport.
    #[test]
    fn a_transport_stride_is_a_power_of_two_of_a_transport_at_least() {
        for bytes in [TRANSPORT_SIZE, 0x400, PAGE] {
            assert_eq!(TransportStride::new(bytes).map(TransportStride::bytes), Some(bytes));
        }
        for bytes in [0, TRANSPORT_SIZE / 2, 0x300, PAGE + TRANSPORT_SIZE] {
            assert_eq!(TransportStride::new(bytes), None, "{bytes:#x}");
        }
    }

    #[test]
    fn layouts_are_named_and_read_back() {
        for layout in DeviceLayout::ALL {
            assert_eq!(layout.name().parse(), Ok(layout));
        }
        assert_eq!("arm".parse::<DeviceLayout>(), Err(UnknownDeviceLayout));
    }
}
