//! The board's own device tree, written from what the board is: its memory
//! map and interrupt wiring (`crate::board`), its processor, the firmware
//! interface its processor answers on, and the identity it names itself
//! by — without an allocator, into a caller's buffer.
//!
//! Provenance (R7): the Devicetree Specification v0.4 — chapter 5 for the
//! flattened format, 2.2.2 for the generic node names (`serial`, `rtc`,
//! `gpio`, `interrupt-controller`, `msi-controller`, `clock`, `flash`,
//! `pcie`, `memory`, `cpus`, `timer`), 2.3 for `dma-coherent` on each
//! device that makes coherent DMA, 3.7 and 3.8 for the processor and cache
//! nodes, whose geometry is what `CCSIDR_EL1` reports ([`crate::cpu::id`])
//! — and, for each node, the binding its `compatible` names (Linux
//! `Documentation/devicetree/bindings`): `arm,cortex-a15-gic` (the GICv2
//! binding's; the board's GIC has no Security Extensions, which every
//! product the binding names otherwise has, and this one is the string
//! EDK2's GIC discovery looks for, `ArmVirtGicPlatformDxe` on its master
//! branch and `ArmVirtGicArchLib` in releases before it),
//! `arm,gic-v2m-frame`,
//! `arm,armv8-timer`, `arm,psci-1.0` and `arm,psci-0.2`
//! ([`PSCI_COMPATIBLE`]),
//! `arm,pl011`, `arm,pl031`, `arm,pl061` with `arm,primecell`,
//! `gpio-keys`, `cfi-flash`, `pci-host-ecam-generic` and `virtio,mmio`.
//! EBBR requires `/chosen/stdout-path`, which names the console through
//! its alias with the rate a directly booted kernel finds it at.
//!
//! The rest is the board's own choice, where a specification leaves one:
//! phandles numbered from 1; nodes in the order a reader meets the
//! machine — what it boots with, its memory, its processor, its firmware
//! interface and timer, its interrupt controller, the clock its PrimeCells
//! run on, the PrimeCells by address, the power button on one of them, the
//! flash, the PCIe host, then the virtio transports by address; the cache
//! hierarchy described down to the L2; the power button named `Power`, on
//! the GPIO line the layout gives it (the first, on the board's own);
//! and every interrupt by the level its line holds.
//!
//! `/chosen` carries no `rng-seed` or `kaslr-seed`. Linux credits an
//! `rng-seed` as entropy (`random.trust_bootloader`), and a deterministic
//! machine would hand the same bytes to every boot of the same
//! configuration and again after each reset, making a guest's keys
//! guessable; QEMU draws fresh ones from the host on every boot. A guest
//! given none gathers its own.

use super::fdt::{DeviceTreeError, Header, Out, FDT_BEGIN_NODE, FDT_END, FDT_END_NODE, HEADER_LEN};
use crate::board::{
    FLASH0_BASE, FLASH1_BASE, FLASH_BANK_SIZE, PPI_HYPERVISOR_TIMER, PPI_PHYSICAL_TIMER,
    PPI_SECURE_PHYSICAL_TIMER, PPI_VIRTUAL_TIMER, RAM_BASE, UART_CLOCK,
};
use crate::bus::PhysAddr;
use crate::cpu::{PsciConduit, PSCI_COMPATIBLE};
use crate::devices::gicv2::GicExtensions;
use crate::devices::gicv2_virt::MAINTENANCE_INTID;
use crate::devices::virtio_mmio::TRANSPORT_COUNT;
use crate::layout::{DeviceLayout, DeviceWindows, GicWindows, Window};
use crate::identity::BoardIdentity;
use crate::interrupt_map::{Spi, Wiring};
use crate::CpuModel;

/// An empty memory reservation block, its terminating entry alone (5.3),
/// after the header; the structure block (5.4) after it.
const RESERVE_MAP_LEN: usize = 16;
const STRUCT_OFFSET: usize = HEADER_LEN + RESERVE_MAP_LEN;

/// The phandles the tree's nodes refer to one another by.
const PHANDLE_CLOCK: u32 = 1;
const PHANDLE_GIC: u32 = 2;
const PHANDLE_MSI: u32 = 3;
const PHANDLE_GPIO: u32 = 4;
const PHANDLE_L2: u32 = 5;

/// The interrupt specifier's first cell: a shared or a private peripheral
/// interrupt (the GIC binding).
const GIC_SPI: u32 = 0;
const GIC_PPI: u32 = 1;
/// Its third cell: level-sensitive, active high, the GICv2's one level
/// polarity (IHI 0048B 1.4.2: an interrupt is level-sensitive active-high
/// or rising-edge-triggered), which every line here is; for a PPI, ORed
/// with the mask of the processors it reaches, the one processor here.
const IRQ_LEVEL_HIGH: u32 = 4;
const PPI_ONE_PROCESSOR: u32 = 1 << 8;

/// Linux `KEY_POWER` (`include/uapi/linux/input-event-codes.h`), the code
/// the power button reports.
const KEY_POWER: u32 = 116;

/// The UART's node name, which [`SERIAL_ALIAS`] names with its address.
const SERIAL_NODE: &str = "serial";
/// The console's alias, which `stdout-path` names with the rate a directly
/// booted kernel finds the UART at ([`super::DIRECT_BOOT_CONSOLE_BAUD`]),
/// the one EDK2 sets it to too (`PcdUartDefaultBaudRate`).
const SERIAL_ALIAS: &str = "serial0";

/// What `/chosen` holds besides the console.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Chosen<'a> {
    /// The kernel's command line, when it has one.
    pub(crate) bootargs: Option<&'a str>,
    /// Where the initrd lies, when there is one.
    pub(crate) initrd: Option<InitrdRange>,
}

/// The guest-physical addresses an initrd starts at and ends before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InitrdRange {
    pub(crate) start: PhysAddr,
    pub(crate) end: PhysAddr,
}

/// Everything the board's tree describes that is not fixed by the board.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoardTree<'a> {
    pub(crate) identity: &'a BoardIdentity,
    pub(crate) ram_len: u64,
    pub(crate) cpu: CpuModel,
    pub(crate) psci: PsciConduit,
    pub(crate) gic: GicExtensions,
    pub(crate) chosen: Chosen<'a>,
    /// The SPIs the devices drive.
    pub(crate) wiring: Wiring,
    /// Where the devices sit, and the line the power button drives.
    pub(crate) layout: DeviceLayout,
}

/// The most property names a tree may use: the strings block the writer
/// keeps. The board's own tree uses fewer, a margin the unit tests hold
/// it to, so a node that would fill the table fails a test rather than a
/// machine's build with [`DeviceTreeError::TooLarge`].
const MAX_NAMES: usize = 64;

/// The longest text the tree formats: a node's unit name, a path to one,
/// or `stdout-path`.
const TEXT_CAPACITY: usize = 64;

/// Text formatted without an allocator (`core::fmt`), into a buffer of
/// [`TEXT_CAPACITY`] bytes.
struct Text {
    buf: [u8; TEXT_CAPACITY],
    len: usize,
}

impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.len.checked_add(s.len()).ok_or(core::fmt::Error)?;
        self.buf
            .get_mut(self.len..end)
            .ok_or(core::fmt::Error)?
            .copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

impl Text {
    /// `args` formatted; text that does not fit is
    /// [`DeviceTreeError::TooLarge`].
    fn of(args: core::fmt::Arguments<'_>) -> Result<Self, DeviceTreeError> {
        let mut text = Self {
            buf: [0; TEXT_CAPACITY],
            len: 0,
        };
        core::fmt::write(&mut text, args).map_err(|_| DeviceTreeError::TooLarge)?;
        Ok(text)
    }

    fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// A 64-bit value as two cells, the high one first (2.2.4).
const fn cells_of(value: u64) -> [u32; 2] {
    [(value >> 32) as u32, value as u32]
}

/// A node's unit name (2.2.1): `base`, `@`, and its unit address in
/// lowercase hexadecimal with no leading zeros.
struct UnitName<'a> {
    base: &'a str,
    addr: u64,
}

impl core::fmt::Display for UnitName<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}@{:x}", self.base, self.addr)
    }
}

/// A flattened tree being written: the structure block from
/// [`STRUCT_OFFSET`], each property name kept for the strings block the
/// first time a property uses it, at an offset the names before it give.
struct Writer<'o> {
    out: Out<'o>,
    names: [&'static str; MAX_NAMES],
    name_count: usize,
}

impl<'o> Writer<'o> {
    fn new(buf: &'o mut [u8]) -> Self {
        Self {
            out: Out::writing_at(buf, STRUCT_OFFSET),
            names: [""; MAX_NAMES],
            name_count: 0,
        }
    }

    /// `FDT_BEGIN_NODE` of a node named `name`.
    fn begin_bytes(&mut self, name: &[u8]) -> Result<(), DeviceTreeError> {
        self.out.put_u32(FDT_BEGIN_NODE)?;
        self.out.put(name)?;
        self.out.put(&[0])?;
        self.out.pad4()
    }

    fn begin(&mut self, name: &str) -> Result<(), DeviceTreeError> {
        self.begin_bytes(name.as_bytes())
    }

    /// `FDT_BEGIN_NODE` of a node named `base` at unit address `addr`.
    fn begin_at(&mut self, base: &str, addr: u64) -> Result<(), DeviceTreeError> {
        let name = Text::of(format_args!("{}", UnitName { base, addr }))?;
        self.begin_bytes(name.bytes())
    }

    fn end(&mut self) -> Result<(), DeviceTreeError> {
        self.out.put_u32(FDT_END_NODE)
    }

    /// The strings-block offset of `name`, given one after the names before
    /// it on its first use.
    fn name_offset(&mut self, name: &'static str) -> Result<usize, DeviceTreeError> {
        let mut offset = 0;
        for kept in &self.names[..self.name_count] {
            if *kept == name {
                return Ok(offset);
            }
            offset += kept.len() + 1;
        }
        if self.name_count == MAX_NAMES {
            return Err(DeviceTreeError::TooLarge);
        }
        self.names[self.name_count] = name;
        self.name_count += 1;
        Ok(offset)
    }

    /// A property's head: its value of `len` bytes follows.
    fn head(&mut self, name: &'static str, len: usize) -> Result<(), DeviceTreeError> {
        let offset = self.name_offset(name)?;
        self.out.put_prop_head(len, offset)
    }

    fn prop(&mut self, name: &'static str, value: &[u8]) -> Result<(), DeviceTreeError> {
        self.head(name, value.len())?;
        self.out.put(value)?;
        self.out.pad4()
    }

    /// A property of no value, true by being there.
    fn flag(&mut self, name: &'static str) -> Result<(), DeviceTreeError> {
        self.head(name, 0)
    }

    /// A string, NUL-terminated.
    fn string(&mut self, name: &'static str, value: &str) -> Result<(), DeviceTreeError> {
        self.string_list(name, core::iter::once(value.as_bytes()))
    }

    /// Formatted text as a string, NUL-terminated.
    fn text(&mut self, name: &'static str, value: &Text) -> Result<(), DeviceTreeError> {
        self.string_list(name, core::iter::once(value.bytes()))
    }

    /// A list of strings, each NUL-terminated.
    fn strings(&mut self, name: &'static str, values: &[&str]) -> Result<(), DeviceTreeError> {
        self.string_list(name, values.iter().map(|v| v.as_bytes()))
    }

    /// Strings given as bytes, each NUL-terminated: the one encoding every
    /// string property here goes through.
    fn string_list<'v, I>(&mut self, name: &'static str, values: I) -> Result<(), DeviceTreeError>
    where
        I: Iterator<Item = &'v [u8]> + Clone,
    {
        self.head(name, values.clone().map(|v| v.len() + 1).sum())?;
        for value in values {
            self.out.put(value)?;
            self.out.put(&[0])?;
        }
        self.out.pad4()
    }

    /// The path of the root's child named `base` at unit address `addr`,
    /// NUL-terminated: the node [`Writer::begin_at`] names so.
    fn path_at(&mut self, name: &'static str, base: &str, addr: u64) -> Result<(), DeviceTreeError> {
        let path = Text::of(format_args!("/{}", UnitName { base, addr }))?;
        self.text(name, &path)
    }

    /// 32-bit cells, big-endian (2.2.4).
    fn cells(&mut self, name: &'static str, cells: &[u32]) -> Result<(), DeviceTreeError> {
        self.head(name, cells.len() * 4)?;
        for cell in cells {
            self.out.put_u32(*cell)?;
        }
        Ok(())
    }

    fn cell(&mut self, name: &'static str, value: u32) -> Result<(), DeviceTreeError> {
        self.cells(name, &[value])
    }

    /// 64-bit values, each two cells ([`cells_of`]): the root's addresses
    /// and sizes, two cells each.
    fn pairs(&mut self, name: &'static str, values: &[u64]) -> Result<(), DeviceTreeError> {
        self.head(name, values.len() * 8)?;
        for value in values {
            for cell in cells_of(*value) {
                self.out.put_u32(cell)?;
            }
        }
        Ok(())
    }

    /// A `reg` naming `windows`, each as its address and size, two cells
    /// each.
    fn windows(&mut self, name: &'static str, windows: &[Window]) -> Result<(), DeviceTreeError> {
        self.head(name, windows.len() * 16)?;
        for window in windows {
            for cell in cells_of(window.base.get()).into_iter().chain(cells_of(window.size)) {
                self.out.put_u32(cell)?;
            }
        }
        Ok(())
    }

    /// A `reg` naming `window` alone ([`Writer::windows`]).
    fn window(&mut self, name: &'static str, window: Window) -> Result<(), DeviceTreeError> {
        self.windows(name, &[window])
    }

    /// `FDT_END`, the strings block after the structure block, then the
    /// header and the empty reservation block in front: the tree's length.
    fn finish(mut self) -> Result<usize, DeviceTreeError> {
        self.out.put_u32(FDT_END)?;
        let strings_at = self.out.pos();
        for name in &self.names[..self.name_count] {
            self.out.put(name.as_bytes())?;
            self.out.put(&[0])?;
        }
        let total = self.out.pos();
        self.out.seek(0);
        self.out.put_header(&Header {
            total,
            struct_at: STRUCT_OFFSET,
            strings_at,
            boot_cpuid: 0,
            strings_len: total - strings_at,
            struct_len: strings_at - STRUCT_OFFSET,
        })?;
        self.out.put(&[0; RESERVE_MAP_LEN])?;
        Ok(total)
    }
}

/// The geometry `CCSIDR_EL1` gives a cache (without FEAT_CCIDX:
/// LineSize [2:0] is log2(bytes) - 4, Associativity [12:3] ways - 1,
/// NumSets [27:13] sets - 1).
struct CacheGeometry {
    line: u32,
    sets: u32,
    size: u32,
}

impl CacheGeometry {
    const fn of(ccsidr: u64) -> Self {
        let line = 1u32 << ((ccsidr & 7) + 4);
        let ways = ((ccsidr >> 3) & 0x3FF) as u32 + 1;
        let sets = ((ccsidr >> 13) & 0x7FFF) as u32 + 1;
        Self {
            line,
            sets,
            size: line * ways * sets,
        }
    }
}

/// Write the board's tree for `tree` into `out`, returning its length.
///
/// # Errors
///
/// [`DeviceTreeError::TooLarge`] when it does not fit in `out`.
pub(crate) fn write(tree: &BoardTree<'_>, out: &mut [u8]) -> Result<usize, DeviceTreeError> {
    let mut w = Writer::new(out);
    w.begin("")?;
    w.cell("#address-cells", 2)?;
    w.cell("#size-cells", 2)?;
    w.prop("compatible", tree.identity.compatible().property())?;
    w.string("model", tree.identity.model())?;
    w.cell("interrupt-parent", PHANDLE_GIC)?;

    chosen(&mut w, &tree.chosen)?;

    let windows = tree.layout.windows();
    w.begin("aliases")?;
    w.path_at(SERIAL_ALIAS, SERIAL_NODE, windows.uart.get())?;
    w.end()?;

    w.begin_at("memory", RAM_BASE)?;
    w.string("device_type", "memory")?;
    w.pairs("reg", &[RAM_BASE, tree.ram_len])?;
    w.end()?;

    cpus(&mut w, tree.cpu)?;

    w.begin("psci")?;
    w.prop("compatible", PSCI_COMPATIBLE)?;
    w.string("method", tree.psci.method())?;
    w.end()?;

    // The binding's order: Secure physical, Non-secure physical, virtual,
    // hypervisor.
    w.begin("timer")?;
    w.string("compatible", "arm,armv8-timer")?;
    let interrupts = [
        PPI_SECURE_PHYSICAL_TIMER,
        PPI_PHYSICAL_TIMER,
        PPI_VIRTUAL_TIMER,
        PPI_HYPERVISOR_TIMER,
    ]
    .map(|intid| [GIC_PPI, intid - 16, PPI_ONE_PROCESSOR | IRQ_LEVEL_HIGH]);
    w.cells("interrupts", interrupts.as_flattened())?;
    w.flag("always-on")?;
    w.end()?;

    gic(&mut w, tree.gic, windows.gic)?;

    w.begin("clock")?;
    w.string("compatible", "fixed-clock")?;
    w.cell("#clock-cells", 0)?;
    w.cell("clock-frequency", UART_CLOCK.get() as u32)?;
    w.cell("phandle", PHANDLE_CLOCK)?;
    w.end()?;

    w.begin_at(SERIAL_NODE, windows.uart.get())?;
    w.strings("compatible", &["arm,pl011", "arm,primecell"])?;
    w.window("reg", windows.uart_window())?;
    w.cells("interrupts", &spi(tree.wiring.uart()))?;
    w.cells("clocks", &[PHANDLE_CLOCK, PHANDLE_CLOCK])?;
    w.strings("clock-names", &["uartclk", "apb_pclk"])?;
    w.end()?;

    w.begin_at("rtc", windows.rtc.get())?;
    w.strings("compatible", &["arm,pl031", "arm,primecell"])?;
    w.window("reg", windows.rtc_window())?;
    w.cells("interrupts", &spi(tree.wiring.rtc()))?;
    w.cell("clocks", PHANDLE_CLOCK)?;
    w.string("clock-names", "apb_pclk")?;
    w.end()?;

    w.begin_at("gpio", windows.gpio.get())?;
    w.strings("compatible", &["arm,pl061", "arm,primecell"])?;
    w.window("reg", windows.gpio_window())?;
    w.cells("interrupts", &spi(tree.wiring.gpio()))?;
    w.flag("gpio-controller")?;
    w.cell("#gpio-cells", 2)?;
    w.cell("clocks", PHANDLE_CLOCK)?;
    w.string("clock-names", "apb_pclk")?;
    w.cell("phandle", PHANDLE_GPIO)?;
    w.end()?;

    w.begin("gpio-keys")?;
    w.string("compatible", "gpio-keys")?;
    w.begin("key-power")?;
    w.string("label", "Power")?;
    w.cell("linux,code", KEY_POWER)?;
    // The line, active high (the binding's flags 0).
    let line = u32::from(tree.layout.power_button_line());
    w.cells("gpios", &[PHANDLE_GPIO, line, 0])?;
    w.end()?;
    w.end()?;

    w.begin_at("flash", FLASH0_BASE)?;
    w.string("compatible", "cfi-flash")?;
    w.pairs(
        "reg",
        &[FLASH0_BASE, FLASH_BANK_SIZE, FLASH1_BASE, FLASH_BANK_SIZE],
    )?;
    w.cell("bank-width", 4)?;
    w.end()?;

    pcie(&mut w, tree.wiring, windows)?;

    for n in 0..TRANSPORT_COUNT {
        let transport = windows.virtio.transport(n);
        w.begin_at("virtio", transport.base.get())?;
        w.string("compatible", "virtio,mmio")?;
        w.window("reg", transport)?;
        // A transport's line is asserted while its InterruptStatus has a
        // bit set (`Transport::irq`): a level.
        w.cells("interrupts", &spi(tree.wiring.virtio(n as u8)))?;
        w.flag("dma-coherent")?;
        w.end()?;
    }

    w.end()?;
    w.finish()
}

/// A shared peripheral interrupt's specifier, level-high: the GIC binding
/// names an SPI by its number among the SPIs.
const fn spi(spi: Spi) -> [u32; 3] {
    [GIC_SPI, spi.number(), IRQ_LEVEL_HIGH]
}

fn chosen(w: &mut Writer<'_>, chosen: &Chosen<'_>) -> Result<(), DeviceTreeError> {
    w.begin("chosen")?;
    // `serial0:115200n8`: the alias, the rate, no parity, eight data bits.
    let stdout_path = Text::of(format_args!(
        "{SERIAL_ALIAS}:{}n8",
        super::DIRECT_BOOT_CONSOLE_BAUD.get()
    ))?;
    w.text("stdout-path", &stdout_path)?;
    if let Some(bootargs) = chosen.bootargs {
        w.string("bootargs", bootargs)?;
    }
    if let Some(initrd) = chosen.initrd {
        w.pairs("linux,initrd-start", &[initrd.start.get()])?;
        w.pairs("linux,initrd-end", &[initrd.end.get()])?;
    }
    w.end()
}

/// The processor and the caches its `CCSIDR_EL1` reports: an L1 data and
/// instruction cache each, and the unified L2 they share (`CLIDR_EL1`),
/// the hierarchy of both processor models, each with its own sizes.
fn cpus(w: &mut Writer<'_>, cpu: CpuModel) -> Result<(), DeviceTreeError> {
    let caches = cpu.implementation().caches;
    let (l1d, l1i, l2) = (
        CacheGeometry::of(caches.l1_data),
        CacheGeometry::of(caches.l1_instruction),
        CacheGeometry::of(caches.l2),
    );
    w.begin("cpus")?;
    w.cell("#address-cells", 1)?;
    w.cell("#size-cells", 0)?;
    w.begin_at("cpu", 0)?;
    w.string("device_type", "cpu")?;
    w.string("compatible", cpu.compatible())?;
    w.cell("reg", 0)?;
    w.cell("d-cache-size", l1d.size)?;
    w.cell("d-cache-sets", l1d.sets)?;
    w.cell("d-cache-block-size", l1d.line)?;
    w.cell("i-cache-size", l1i.size)?;
    w.cell("i-cache-sets", l1i.sets)?;
    w.cell("i-cache-block-size", l1i.line)?;
    w.cell("next-level-cache", PHANDLE_L2)?;
    w.end()?;
    w.begin("l2-cache")?;
    w.string("compatible", "cache")?;
    w.cell("cache-level", 2)?;
    w.flag("cache-unified")?;
    w.cell("cache-size", l2.size)?;
    w.cell("cache-sets", l2.sets)?;
    w.cell("cache-block-size", l2.line)?;
    w.cell("phandle", PHANDLE_L2)?;
    w.end()?;
    w.end()
}

/// The GIC and its MSI frame; with the Virtualization Extensions, the
/// virtual interface's two frames after the distributor's and the CPU
/// interface's, and its maintenance interrupt (the GIC binding). Each
/// `reg` region is the size the frame decodes.
fn gic(
    w: &mut Writer<'_>,
    extensions: GicExtensions,
    gic: GicWindows,
) -> Result<(), DeviceTreeError> {
    w.begin_at("interrupt-controller", gic.dist.get())?;
    w.string("compatible", "arm,cortex-a15-gic")?;
    w.flag("interrupt-controller")?;
    w.cell("#interrupt-cells", 3)?;
    w.cell("#address-cells", 2)?;
    w.cell("#size-cells", 2)?;
    w.flag("ranges")?;
    match extensions {
        GicExtensions::Absent => {
            w.windows("reg", &[gic.dist_window(), gic.cpu_window()])?;
        }
        GicExtensions::Virtualization => {
            w.windows(
                "reg",
                &[
                    gic.dist_window(),
                    gic.cpu_window(),
                    gic.hyp_window(),
                    gic.vcpu_window(),
                ],
            )?;
            w.cells(
                "interrupts",
                &[
                    GIC_PPI,
                    MAINTENANCE_INTID - 16,
                    PPI_ONE_PROCESSOR | IRQ_LEVEL_HIGH,
                ],
            )?;
        }
    }
    w.cell("phandle", PHANDLE_GIC)?;
    w.begin_at("msi-controller", gic.v2m.get())?;
    w.string("compatible", "arm,gic-v2m-frame")?;
    w.flag("msi-controller")?;
    w.window("reg", gic.v2m_window())?;
    w.cell("phandle", PHANDLE_MSI)?;
    w.end()?;
    w.end()
}

/// The PCIe host bridge (`pci-host-ecam-generic`, and the PCI bus binding
/// for `ranges`, `bus-range` and the interrupt map): configuration space
/// over the ECAM window, one bus per MiB of it; I/O, 32-bit and 64-bit
/// memory windows; INTA to INTD of each device slot swizzled onto the
/// bridge's four SPIs, which `wiring` gives, as the PCI-to-PCI bridge
/// specification routes a device's pins; and MSIs through the GIC's MSI
/// frame.
fn pcie(
    w: &mut Writer<'_>,
    wiring: Wiring,
    windows: DeviceWindows,
) -> Result<(), DeviceTreeError> {
    /// `ranges` space codes in the first child address cell (PCI bus
    /// binding: `ss` in phys.hi).
    const SPACE_IO: u32 = 0x0100_0000;
    const SPACE_MEM32: u32 = 0x0200_0000;
    const SPACE_MEM64: u32 = 0x0300_0000;
    /// An `interrupt-map` entry's cells: the device's three address cells
    /// and its pin, the GIC's phandle, its two address cells and its three
    /// interrupt cells.
    const ENTRY: usize = 10;
    /// One `ranges` entry: its space code and the child address, then the
    /// window's address and size, each 64-bit value two cells
    /// ([`cells_of`]).
    const fn range(space: u32, child: u64, window: Window) -> [u32; 7] {
        let [ch, cl] = cells_of(child);
        let [ph, pl] = cells_of(window.base.get());
        let [sh, sl] = cells_of(window.size);
        [space, ch, cl, ph, pl, sh, sl]
    }
    let pcie = windows.pcie;
    // The unit address is the first `reg` address (2.2.1), the ECAM
    // window's.
    w.begin_at("pcie", pcie.ecam.base.get())?;
    w.string("compatible", "pci-host-ecam-generic")?;
    w.string("device_type", "pci")?;
    w.cell("#address-cells", 3)?;
    w.cell("#size-cells", 2)?;
    w.cell("#interrupt-cells", 1)?;
    w.window("reg", pcie.ecam)?;
    let last_bus = (pcie.ecam.size >> 20) as u32 - 1;
    w.cells("bus-range", &[0, last_bus])?;
    // I/O space from 0, and both memory windows at the addresses they have
    // on the bus.
    let ranges = [
        range(SPACE_IO, 0, pcie.pio),
        range(SPACE_MEM32, pcie.mmio32.base.get(), pcie.mmio32),
        range(SPACE_MEM64, pcie.mmio64.base.get(), pcie.mmio64),
    ];
    w.cells("ranges", ranges.as_flattened())?;
    // Device numbers 0 to 3 (phys.hi bits 12:11) by pins INTA to INTD.
    let mut map = [0u32; 16 * ENTRY];
    for (i, entry) in map.chunks_exact_mut(ENTRY).enumerate() {
        let (device, pin) = ((i / 4) as u32, (i % 4) as u32 + 1);
        let swizzled = (device + pin - 1) % 4;
        entry.copy_from_slice(&[
            device << 11,
            0,
            0,
            pin,
            PHANDLE_GIC,
            0,
            0,
            GIC_SPI,
            wiring.pcie_intx(swizzled).number(),
            IRQ_LEVEL_HIGH,
        ]);
    }
    w.cells("interrupt-map", &map)?;
    w.cells("interrupt-map-mask", &[3 << 11, 0, 0, 7])?;
    // The MSI frame takes no MSI specifier, so it is the bus's MSI parent
    // for every requester ID, not a map's target.
    w.cell("msi-parent", PHANDLE_MSI)?;
    w.flag("dma-coherent")?;
    w.end()
}

#[cfg(test)]
#[path = "../../tests/unit/board_tree.rs"]
mod tests;
