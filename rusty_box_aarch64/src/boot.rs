//! Getting a guest started on the board: the device tree it describes
//! itself with, and loading a Linux kernel the way QEMU does when it is
//! given `-kernel` and no firmware.
//!
//! Provenance (R7): QEMU `hw/arm/boot.c` — `load_aarch64_image` (where an
//! arm64 `Image` goes), `arm_setup_direct_kernel_boot` (where the initrd and
//! the device tree go), `arm_load_dtb` (the `/chosen` properties), and
//! `bootloader_aarch64`, whose effect this reproduces directly: the processor
//! starts at the kernel's entry with `x0` holding the device tree's address,
//! `x1`–`x3` zero and `x4` the entry, at EL1 with the MMU off and interrupts
//! masked — the register state Linux's `Documentation/arch/arm64/booting.rst`
//! asks for and QEMU's six-instruction stub leaves.
//!
//! Firmware finds the device tree at the base of RAM, where QEMU's `virt`
//! puts it for `-bios` and `-pflash`. A directly booted kernel finds it where
//! QEMU puts it for an `Image`: after the initrd (or where the initrd would
//! start), aligned up to 2 MiB, since an early arm64 kernel maps the 2 MiB
//! around it. Nothing else shares that region, whatever the kernel's load
//! offset.

pub mod fdt;

use crate::board::UART0_BASE;
use crate::devices::pl011::BaudRate;
pub use fdt::DeviceTreeError;

/// The device tree QEMU 11.1.2 generates for `-M virt -cpu cortex-a57 -m 256`
/// (`-machine dumpdtb=`), which describes exactly this board, as generated
/// with two upstream bugs fixed (docs/qemu-upstream-bugs.md): the CPU node's
/// L1 data cache is the Cortex-A57's 2-way one, as the TRM gives it and
/// this board's CCSIDR reports it ([`crate::cpu::id::CCSIDR_L1D`]), and it
/// names no `next-level-cache` (QEMU names phandle 0 when it makes no L2
/// node). Its `/chosen` `rng-seed` and `kaslr-seed` are those of the dump,
/// fixed: QEMU draws fresh ones on every run, and a fixed pair keeps runs
/// deterministic. A machine patches its memory node to the RAM it was built
/// with, so the 256 MiB it was dumped with is only a default.
pub const QEMU_VIRT_DEVICE_TREE: &[u8] = include_bytes!("../assets/qemu-virt-a57-256m.dtb");

/// The largest edited device tree a machine keeps (QEMU's is under 8 KiB).
pub const DEVICE_TREE_CAPACITY: usize = 64 * 1024;

/// A command line that sends the kernel's console, and its earliest
/// messages, to the board's UART.
pub const SERIAL_CONSOLE_COMMAND_LINE: &str = "console=ttyAMA0 earlycon=pl011,0x9000000";
const _: () = assert!(UART0_BASE == 0x0900_0000, "the earlycon address names the UART");

/// The rate at which a directly booted kernel finds the UART's console set
/// up, eight data bits, no parity, one stop bit, FIFOs enabled: the state
/// boot firmware leaves a console it has initialised in (TF-A's QEMU
/// platforms run theirs at 115200, `PLAT_QEMU_CONSOLE_BAUDRATE`), which
/// Linux's `earlycon=pl011` writes to as it finds it. QEMU's direct boot
/// leaves the UART at reset, transmitting regardless, where the PL011 would
/// transmit nothing (README, "Where the references differ").
pub const DIRECT_BOOT_CONSOLE_BAUD: BaudRate = match BaudRate::new(115_200) {
    Some(rate) => rate,
    None => unreachable!(),
};

const MIB: u64 = 1 << 20;
/// QEMU `KERNEL64_LOAD_ADDR`: where a kernel too old to say goes.
const LEGACY_LOAD_OFFSET: u64 = 0x0008_0000;
/// QEMU `BOOTLOADER_MAX_SIZE`: a requested offset below this would overlap
/// QEMU's stub, so QEMU moves the kernel up by 2 MiB; this board does the
/// same so the two load a kernel at the same address.
const BOOTLOADER_MAX_SIZE: u64 = 0x1000;
const PAGE: u64 = 4096;
/// The arm64 `Image` header (booting.rst §4).
const HEADER_LEN: usize = 64;

/// Why bytes are not an arm64 kernel `Image`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageError {
    /// No `ARM\x64` magic at offset 56: not an arm64 `Image`. A compressed
    /// `vmlinuz` must be decompressed first.
    NotArm64Image,
    /// Shorter than the 64-byte header.
    Truncated,
}

impl core::fmt::Display for ImageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotArm64Image => "not an arm64 Image (no \"ARM\\x64\" magic; decompress vmlinuz first)",
            Self::Truncated => "shorter than an arm64 Image header",
        })
    }
}

impl core::error::Error for ImageError {}

/// An arm64 kernel `Image` whose header has been read (a witness: holding one
/// proves the bytes are an `Image`).
#[derive(Debug, Clone, Copy)]
pub struct LinuxImage<'a> {
    bytes: &'a [u8],
    load_offset: u64,
    footprint: u64,
}

impl<'a> LinuxImage<'a> {
    /// Read the header of an arm64 `Image` (booting.rst §4).
    ///
    /// # Errors
    ///
    /// [`ImageError`] when the bytes are not one.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ImageError> {
        let Some(header) = bytes.first_chunk::<HEADER_LEN>() else {
            return Err(ImageError::Truncated);
        };
        if header[56..60] != *b"ARM\x64" {
            return Err(ImageError::NotArm64Image);
        }
        let field = |at: usize| {
            let mut le = [0u8; 8];
            le.copy_from_slice(&header[at..at + 8]);
            u64::from_le_bytes(le)
        };
        let text_offset = field(8);
        let image_size = field(16);
        // QEMU `load_aarch64_image`: text_offset means something only when
        // image_size is non-zero (kernels from v3.17 on).
        let (load_offset, footprint) = if image_size == 0 {
            (LEGACY_LOAD_OFFSET, bytes.len() as u64)
        } else if text_offset < BOOTLOADER_MAX_SIZE {
            (text_offset + 2 * MIB, image_size.max(bytes.len() as u64))
        } else {
            (text_offset, image_size.max(bytes.len() as u64))
        };
        Ok(Self {
            bytes,
            load_offset,
            footprint,
        })
    }

    /// The image's bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Where the image loads, as an offset from the base of RAM.
    pub fn load_offset(&self) -> u64 {
        self.load_offset
    }

    /// How many bytes from its load address the kernel occupies, BSS
    /// included.
    pub fn footprint(&self) -> u64 {
        self.footprint
    }
}

/// Where a directly booted kernel's pieces go in RAM, as offsets from its
/// base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KernelLayout {
    pub(crate) kernel_offset: u64,
    /// The initrd's offset, when there is one.
    pub(crate) initrd_offset: Option<u64>,
    pub(crate) device_tree_offset: u64,
}

impl KernelLayout {
    /// Check that a device tree of `len` bytes fits at its offset.
    ///
    /// # Errors
    ///
    /// [`PlacementError::DeviceTreeDoesNotFit`] when it runs past the end of
    /// RAM.
    pub(crate) fn fit_device_tree(&self, ram_len: u64, len: u64) -> Result<(), PlacementError> {
        match self.device_tree_offset.checked_add(len) {
            Some(end) if end <= ram_len => Ok(()),
            Some(_) | None => Err(PlacementError::DeviceTreeDoesNotFit),
        }
    }
}

/// Why a kernel cannot be placed in the RAM it was given.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlacementError {
    /// The kernel, BSS included, runs past the end of RAM.
    KernelDoesNotFit,
    /// The initrd runs past the end of RAM.
    InitrdDoesNotFit,
    /// No room is left for the device tree at the 2 MiB boundary after the
    /// initrd.
    DeviceTreeDoesNotFit,
}

impl core::fmt::Display for PlacementError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::KernelDoesNotFit => "the kernel does not fit in RAM",
            Self::InitrdDoesNotFit => "the initrd does not fit in RAM",
            Self::DeviceTreeDoesNotFit => "no room is left in RAM for the device tree after the initrd",
        })
    }
}

impl core::error::Error for PlacementError {}

/// QEMU's placement (`arm_setup_direct_kernel_boot`): the kernel at its load
/// offset; the initrd at min(RAM / 2, 128 MiB), or above the kernel's
/// footprint if that is higher, page-aligned; the device tree at the first
/// 2 MiB boundary after the initrd, or after where it would start when there
/// is none.
pub(crate) fn layout(ram_len: u64, image: &LinuxImage<'_>, initrd_len: u64) -> Result<KernelLayout, PlacementError> {
    let kernel_end = image.load_offset.checked_add(image.footprint).ok_or(PlacementError::KernelDoesNotFit)?;
    if kernel_end > ram_len {
        return Err(PlacementError::KernelDoesNotFit);
    }
    let initrd_at = (ram_len / 2).min(128 * MIB).max(kernel_end).next_multiple_of(PAGE);
    let initrd_end = initrd_at.checked_add(initrd_len).ok_or(PlacementError::InitrdDoesNotFit)?;
    if initrd_len != 0 && initrd_end > ram_len {
        return Err(PlacementError::InitrdDoesNotFit);
    }
    let device_tree_offset = initrd_end.checked_next_multiple_of(2 * MIB).ok_or(PlacementError::DeviceTreeDoesNotFit)?;
    if device_tree_offset >= ram_len {
        return Err(PlacementError::DeviceTreeDoesNotFit);
    }
    Ok(KernelLayout {
        kernel_offset: image.load_offset,
        initrd_offset: (initrd_len != 0).then_some(initrd_at),
        device_tree_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(text_offset: u64, image_size: u64) -> [u8; 64] {
        let mut h = [0u8; 64];
        h[8..16].copy_from_slice(&text_offset.to_le_bytes());
        h[16..24].copy_from_slice(&image_size.to_le_bytes());
        h[56..60].copy_from_slice(b"ARM\x64");
        h
    }

    #[test]
    fn qemu_loads_a_modern_image_two_mib_up_and_an_old_one_at_512_kib() {
        let modern = header(0, 0x0200_0000);
        let image = LinuxImage::parse(&modern).expect("an Image");
        assert_eq!(image.load_offset(), 2 * MIB);
        assert_eq!(image.footprint(), 0x0200_0000);
        let old = header(0x8_0000, 0);
        assert_eq!(LinuxImage::parse(&old).expect("an Image").load_offset(), LEGACY_LOAD_OFFSET);
        let high = header(0x10_0000, 0x1000);
        assert_eq!(LinuxImage::parse(&high).expect("an Image").load_offset(), 0x10_0000);
        assert_eq!(LinuxImage::parse(&[0u8; 64]).err(), Some(ImageError::NotArm64Image));
        assert_eq!(LinuxImage::parse(&[0u8; 20]).err(), Some(ImageError::Truncated));
        // A header cut short before its magic is short, not foreign.
        assert_eq!(LinuxImage::parse(&modern[..40]).err(), Some(ImageError::Truncated));
        assert_eq!(LinuxImage::parse(&modern[..60]).err(), Some(ImageError::Truncated));
    }

    #[test]
    fn the_initrd_goes_at_half_of_ram_capped_at_128_mib_and_above_the_kernel() {
        let h = header(0, 0x0300_0000);
        let image = LinuxImage::parse(&h).expect("an Image");
        let small = layout(128 * MIB, &image, 0x10_0000).expect("fits");
        assert_eq!(small.initrd_offset, Some(64 * MIB));
        let big = layout(1024 * MIB, &image, 0x10_0000).expect("fits");
        assert_eq!(big.initrd_offset, Some(128 * MIB));
        let tight = layout(64 * MIB, &image, 0x10_0000).expect("fits");
        assert_eq!(tight.initrd_offset, Some(2 * MIB + 0x0300_0000), "above the kernel's footprint");
        assert_eq!(layout(32 * MIB, &image, 0), Err(PlacementError::KernelDoesNotFit));
        assert_eq!(layout(54 * MIB, &image, 8 * MIB), Err(PlacementError::InitrdDoesNotFit));
    }

    /// The device tree follows the initrd at a 2 MiB boundary, or the place
    /// the initrd would start when there is none — clear of a kernel loaded
    /// low, which the base of RAM would not be.
    #[test]
    fn the_device_tree_goes_at_the_2_mib_boundary_after_the_initrd() {
        let h = header(0, 0x0300_0000);
        let image = LinuxImage::parse(&h).expect("an Image");
        let with_initrd = layout(128 * MIB, &image, 0x10_0001).expect("fits");
        assert_eq!(with_initrd.device_tree_offset, 66 * MIB);
        let without = layout(128 * MIB, &image, 0).expect("fits");
        assert_eq!(without.initrd_offset, None);
        assert_eq!(without.device_tree_offset, 64 * MIB);
        assert_eq!(without.fit_device_tree(128 * MIB, 8 * 1024), Ok(()));
        assert_eq!(
            layout(128 * MIB, &image, 64 * MIB - 1).err(),
            Some(PlacementError::DeviceTreeDoesNotFit),
            "the boundary after the initrd is the end of RAM"
        );
        assert_eq!(
            without.fit_device_tree(64 * MIB + 0x1000, 0x2000),
            Err(PlacementError::DeviceTreeDoesNotFit)
        );

        let low = header(0x1000, 0x1000);
        let low = LinuxImage::parse(&low).expect("an Image");
        assert_eq!(low.load_offset(), 0x1000, "a kernel may ask to load just above the stub");
        let placed = layout(128 * MIB, &low, 0).expect("fits");
        assert!(placed.device_tree_offset >= placed.kernel_offset + low.footprint());
    }

    /// The CPU node's L1 cache properties (Devicetree Specification v0.4,
    /// 3.7.3) describe the caches `CCSIDR_EL1` reports, which are the
    /// Cortex-A57's (TRM DDI 0488C, 6.1: a 32 KiB 2-way data cache and a
    /// 48 KiB 3-way instruction cache, 64-byte lines); no cache node exists
    /// for a `next-level-cache` to name.
    #[test]
    fn the_device_tree_describes_the_l1_caches_ccsidr_reports() {
        use crate::cpu::id::{CCSIDR_L1D, CCSIDR_L1I};
        let cell = |name: &str| -> u64 {
            let bytes = fdt::property(QEMU_VIRT_DEVICE_TREE, "cpus/cpu@0", name).expect("well formed").expect(name);
            u64::from(u32::from_be_bytes(bytes.try_into().expect("one cell")))
        };
        // CCSIDR_EL1 without FEAT_CCIDX: LineSize [2:0] is log2(bytes) - 4,
        // Associativity [12:3] ways - 1, NumSets [27:13] sets - 1.
        let caches = [
            (CCSIDR_L1D, 2, 32 * 1024, ["d-cache-block-size", "d-cache-sets", "d-cache-size"]),
            (CCSIDR_L1I, 3, 48 * 1024, ["i-cache-block-size", "i-cache-sets", "i-cache-size"]),
        ];
        for (ccsidr, ways, size, [block_size, set_count, total]) in caches {
            let line = 1u64 << ((ccsidr & 7) + 4);
            let associativity = ((ccsidr >> 3) & 0x3FF) + 1;
            let sets = ((ccsidr >> 13) & 0x7FFF) + 1;
            assert_eq!((line, associativity, line * associativity * sets), (64, ways, size), "{total}");
            assert_eq!(cell(block_size), line);
            assert_eq!(cell(set_count), sets);
            assert_eq!(cell(total), size);
        }
        assert_eq!(fdt::property(QEMU_VIRT_DEVICE_TREE, "cpus/cpu@0", "next-level-cache"), Ok(None));
    }
}
