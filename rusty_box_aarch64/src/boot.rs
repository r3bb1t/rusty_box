//! Getting a guest started on the board: the device tree it describes
//! itself with, and loading a Linux kernel with no firmware, as QEMU's
//! `-kernel` does.
//!
//! Provenance (R7): Linux's arm64 boot protocol,
//! `Documentation/arch/arm64/booting.rst`, which is the contract between a
//! boot loader and the kernel: the `Image` header (§4), where the kernel,
//! its device tree (§2) and its initrd (§4) may go, and the processor's
//! state at the kernel's entry (§4) — `x0` the device tree's physical
//! address and `x1` to `x3` zero, at the processor's reset level (EL2 with
//! FEAT_EL2, EL1 without) with the MMU off and interrupts masked. Every
//! other register holds its reset value, which for the general-purpose
//! registers and the stack pointer is the board's fill (`cpu::reset`), as
//! the protocol names no value for them. QEMU's
//! `hw/arm/boot.c` (`arm_load_dtb`) for the `/chosen` properties.
//!
//! Where booting.rst leaves the placement open, the rule is the board's own
//! (`layout`): the kernel at the base of RAM, the device tree in the
//! 2 MiB block after the kernel's footprint, the initrd in the next; an
//! `Image` too old to state its size gets the most room the protocol lets
//! a boot loader leave it. QEMU's choices (a boot stub at the base of RAM,
//! with the kernel above it and the entry in `x4`, the initrd at up to
//! 128 MiB, the tree after it) are recorded in the README, "Where the
//! references differ".
//!
//! Firmware finds the device tree at the base of RAM, where stock EDK2 is
//! built to look for it (README, "QEMU fingerprints left").

pub mod fdt;
pub(crate) mod tree;

pub use fdt::DeviceTreeError;

use crate::board::RAM_BASE;
use crate::devices::pl011::BaudRate;

/// The largest device tree a machine keeps, its own or a host's edited:
/// the board's own is about 7 KiB and grows with the kernel's command
/// line, which Linux keeps to 2 KiB (`COMMAND_LINE_SIZE`).
pub const DEVICE_TREE_CAPACITY: usize = 64 * 1024;

/// A command line that sends the kernel's console, and its earliest
/// messages, to the board's UART: `earlycon` with no options finds it
/// through `/chosen/stdout-path` (Linux `param_setup_earlycon`), which every
/// layout's tree names, as EBBR requires.
pub const SERIAL_CONSOLE_COMMAND_LINE: &str = "console=ttyAMA0 earlycon";

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
const GIB: u64 = 1 << 30;
/// booting.rst §4: the load offset of an `Image` whose header gives no
/// size (a kernel before v3.17), whose `text_offset` "can be assumed to be
/// 0x80000".
const UNSIZED_TEXT_OFFSET: u64 = 0x0008_0000;
/// booting.rst §4: the kernel is placed `text_offset` bytes from a base
/// aligned to 2 MiB. §2: a device tree is at most 2 MiB, and the kernel
/// maps it cacheable in blocks of up to 2 MiB, so the board gives it a
/// 2 MiB block of its own.
const BLOCK: u64 = 2 * MIB;
/// booting.rst §2, NOTE: a kernel before v4.2 looks for its device tree
/// only within the 512 MiB from its base.
const EARLY_DEVICE_TREE_REACH: u64 = 512 * MIB;
/// booting.rst §4: an initrd lies "entirely within a 1 GB aligned physical
/// memory window of up to 32 GB in size that fully covers the kernel Image
/// as well".
const INITRD_WINDOW_MAX: u64 = 32 * GIB;
const PAGE: u64 = 4096;
/// The arm64 `Image` header (booting.rst §4).
const HEADER_LEN: usize = 64;

// RAM's base is the kernel's base, and an offset into RAM is as aligned
// as the address it names, to 2 MiB and to 1 GiB.
const _: () = assert!(RAM_BASE.is_multiple_of(BLOCK) && RAM_BASE.is_multiple_of(GIB));

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
            Self::NotArm64Image => {
                "not an arm64 Image (no \"ARM\\x64\" magic; decompress vmlinuz first)"
            }
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
    footprint: Option<u64>,
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
        // booting.rst §4: a non-zero image_size "must be respected", and
        // text_offset is little-endian with it; a zero image_size marks a
        // kernel before v3.17, loaded at 0x80000 whatever its header says.
        let (load_offset, footprint) = if image_size == 0 {
            (UNSIZED_TEXT_OFFSET, None)
        } else {
            (text_offset, Some(image_size.max(bytes.len() as u64)))
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

    /// Where the image loads, as an offset from the base of RAM: its
    /// `text_offset` from that base, which is aligned to 2 MiB (booting.rst
    /// §4).
    pub fn load_offset(&self) -> u64 {
        self.load_offset
    }

    /// How many bytes from its load address the kernel occupies, BSS
    /// included, when its header says (`image_size`, from v3.17); `None`
    /// for an older image, whose header gives no size.
    pub fn footprint(&self) -> Option<u64> {
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
    /// The initrd runs past the end of RAM, or, after a kernel whose
    /// header gives no size, into the kernel.
    InitrdDoesNotFit,
    /// The initrd would run past the 32 GiB window, aligned to 1 GiB and
    /// covering the kernel, that the kernel reaches it in (booting.rst §4).
    InitrdOutOfReach,
    /// No room is left for the device tree in RAM after the kernel, within
    /// the 512 MiB from its base where a kernel whose header gives no size
    /// looks for it.
    DeviceTreeDoesNotFit,
}

impl core::fmt::Display for PlacementError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::KernelDoesNotFit => "the kernel does not fit in RAM",
            Self::InitrdDoesNotFit => "the initrd does not fit in RAM after the kernel",
            Self::InitrdOutOfReach => {
                "the initrd would end beyond the 32 GiB window the kernel reaches it in"
            }
            Self::DeviceTreeDoesNotFit => {
                "no room is left in RAM for the device tree after the kernel"
            }
        })
    }
}

impl core::error::Error for PlacementError {}

/// The board's placement of a directly booted kernel's pieces in RAM of
/// `ram_len` bytes, which booting.rst constrains and leaves open beyond
/// that.
///
/// The kernel's base is the base of RAM, and the kernel sits its load
/// offset above it (§4): an `Image` with flags bit 3 clear asks for a base
/// "as close as possible to the base of DRAM", one with it set for any base
/// below 2^48, and a kernel before v4.6 uses no memory below its base.
///
/// An `Image` whose header gives its footprint needs that much free from
/// its start and nothing more (§4). The device tree takes the 2 MiB block
/// at the first 2 MiB boundary at or after the footprint's end: 8-byte
/// aligned, never crossing a 2 MiB boundary, in no 2 MiB block the kernel
/// maps with attributes of its own (§2), and, for a footprint under
/// 510 MiB, within the 512 MiB from the base that a kernel before v4.2
/// searches. The initrd starts at the next 2 MiB boundary, so it lies in the window
/// of up to 32 GiB, aligned to 1 GiB and covering the kernel, that §4
/// requires whenever it ends within 32 GiB of the kernel's 1 GiB block
/// ([`PlacementError::InitrdOutOfReach`] otherwise). RAM above the initrd
/// stays one free run.
///
/// An `Image` whose header gives no size (a kernel before v3.17) has a
/// footprint that "is effectively unbound", and a boot loader "should
/// attempt to keep as much memory as possible free for use by the kernel
/// immediately after the end of the kernel image" (§4); kernels that old
/// look for the tree within the 512 MiB from the base (§2). So the device
/// tree takes the last 2 MiB block that starts below the end of RAM or of
/// those 512 MiB, whichever comes first, and the initrd ends where that
/// block starts, page-aligned, each above the image's bytes.
pub(crate) fn layout(
    ram_len: u64,
    image: &LinuxImage<'_>,
    initrd_len: u64,
) -> Result<KernelLayout, PlacementError> {
    let kernel_offset = image.load_offset;
    let footprint = image.footprint.unwrap_or(image.bytes.len() as u64);
    let kernel_end = kernel_offset
        .checked_add(footprint)
        .filter(|&end| end <= ram_len)
        .ok_or(PlacementError::KernelDoesNotFit)?;
    let (device_tree_offset, initrd_offset) = match image.footprint {
        Some(_) => {
            let tree = kernel_end
                .checked_next_multiple_of(BLOCK)
                .filter(|&at| at < ram_len)
                .ok_or(PlacementError::DeviceTreeDoesNotFit)?;
            let initrd = if initrd_len == 0 {
                None
            } else {
                // `tree` is below `ram_len`, so one block more cannot
                // overflow.
                let at = tree + BLOCK;
                let end = at
                    .checked_add(initrd_len)
                    .filter(|&end| end <= ram_len)
                    .ok_or(PlacementError::InitrdDoesNotFit)?;
                let window = end
                    .checked_next_multiple_of(GIB)
                    .map(|top| top - kernel_offset / GIB * GIB);
                if !window.is_some_and(|len| len <= INITRD_WINDOW_MAX) {
                    return Err(PlacementError::InitrdOutOfReach);
                }
                Some(at)
            };
            (tree, initrd)
        }
        None => {
            let tree = ram_len
                .min(EARLY_DEVICE_TREE_REACH)
                .checked_sub(1)
                .map(|last| last / BLOCK * BLOCK)
                .filter(|&at| at >= kernel_end)
                .ok_or(PlacementError::DeviceTreeDoesNotFit)?;
            let initrd = if initrd_len == 0 {
                None
            } else {
                let at = tree
                    .checked_sub(initrd_len)
                    .map(|start| start / PAGE * PAGE)
                    .filter(|&start| start >= kernel_end)
                    .ok_or(PlacementError::InitrdDoesNotFit)?;
                Some(at)
            };
            (tree, initrd)
        }
    };
    Ok(KernelLayout {
        kernel_offset,
        initrd_offset,
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

    /// booting.rst §4: an image loads `text_offset` above a 2 MiB-aligned
    /// base, the base of RAM here, and one whose header gives no size at
    /// 0x80000 whatever its `text_offset` reads.
    #[test]
    fn an_image_loads_its_text_offset_above_the_base_of_ram() {
        let modern = header(0, 0x0200_0000);
        let image = LinuxImage::parse(&modern).expect("an Image");
        assert_eq!(image.load_offset(), 0, "at the base of RAM, no stub below");
        assert_eq!(image.footprint(), Some(0x0200_0000));
        let old = header(0x1234, 0);
        let old = LinuxImage::parse(&old).expect("an Image");
        assert_eq!(old.load_offset(), 0x8_0000);
        assert_eq!(old.footprint(), None, "the header gives no size");
        for text_offset in [0x1000, 0x8_0000, 0x10_0000] {
            let h = header(text_offset, 0x1000);
            assert_eq!(
                LinuxImage::parse(&h).expect("an Image").load_offset(),
                text_offset
            );
        }
        assert_eq!(
            LinuxImage::parse(&[0u8; 64]).err(),
            Some(ImageError::NotArm64Image)
        );
        assert_eq!(
            LinuxImage::parse(&[0u8; 20]).err(),
            Some(ImageError::Truncated)
        );
        // A header cut short before its magic is short, not foreign.
        assert_eq!(
            LinuxImage::parse(&modern[..40]).err(),
            Some(ImageError::Truncated)
        );
        assert_eq!(
            LinuxImage::parse(&modern[..60]).err(),
            Some(ImageError::Truncated)
        );
    }

    /// After an image whose header gives its footprint, the device tree
    /// takes the 2 MiB block at the first 2 MiB boundary at or after the
    /// footprint's end, and the initrd starts at the next; whatever does
    /// not fit is refused, as is an initrd ending past the 32 GiB window
    /// booting.rst §4 lets the kernel reach it in.
    #[test]
    fn the_device_tree_and_the_initrd_follow_the_kernels_footprint() {
        let h = header(0, 0x0300_0000);
        let image = LinuxImage::parse(&h).expect("an Image");
        let placed = layout(128 * MIB, &image, 0x10_0001).expect("fits");
        assert_eq!(
            placed,
            KernelLayout {
                kernel_offset: 0,
                device_tree_offset: 48 * MIB,
                initrd_offset: Some(50 * MIB),
            }
        );
        let without = layout(128 * MIB, &image, 0).expect("fits");
        assert_eq!(without.initrd_offset, None);
        assert_eq!(without.device_tree_offset, 48 * MIB);
        assert_eq!(without.fit_device_tree(128 * MIB, 8 * 1024), Ok(()));

        // A footprint ending inside a block: the tree takes the next one.
        let odd = header(0x8_0000, 0x0123_4567);
        let odd = LinuxImage::parse(&odd).expect("an Image");
        let placed = layout(128 * MIB, &odd, 0x1000).expect("fits");
        assert_eq!(placed.kernel_offset, 0x8_0000);
        assert_eq!(placed.device_tree_offset, 20 * MIB, "after 0x12B_4567");
        assert_eq!(placed.initrd_offset, Some(22 * MIB));

        assert_eq!(
            layout(32 * MIB, &image, 0),
            Err(PlacementError::KernelDoesNotFit)
        );
        assert_eq!(
            layout(54 * MIB, &image, 4 * MIB).map(|l| l.initrd_offset),
            Ok(Some(50 * MIB)),
            "an initrd ending at the end of RAM"
        );
        assert_eq!(
            layout(54 * MIB, &image, 4 * MIB + 1),
            Err(PlacementError::InitrdDoesNotFit)
        );
        assert_eq!(
            layout(48 * MIB, &image, 0),
            Err(PlacementError::DeviceTreeDoesNotFit),
            "the kernel ends at the end of RAM"
        );
        assert_eq!(
            without.fit_device_tree(48 * MIB + 0x1000, 0x2000),
            Err(PlacementError::DeviceTreeDoesNotFit)
        );

        let within = 32 * GIB - 50 * MIB;
        assert_eq!(
            layout(64 * GIB, &image, within).map(|l| l.initrd_offset),
            Ok(Some(50 * MIB)),
            "the initrd ends where the window does"
        );
        assert_eq!(
            layout(64 * GIB, &image, within + 1),
            Err(PlacementError::InitrdOutOfReach)
        );
    }

    /// An image whose header gives no size is followed by as much free RAM
    /// as booting.rst lets a boot loader leave: the device tree in the last
    /// 2 MiB block that starts below the end of RAM or of the 512 MiB from
    /// the base, whichever is lower, and the initrd just below it.
    #[test]
    fn the_device_tree_and_the_initrd_leave_an_unsized_kernel_the_most_room() {
        let h = header(0, 0);
        let image = LinuxImage::parse(&h).expect("an Image");
        let placed = layout(128 * MIB, &image, 0x10_0001).expect("fits");
        assert_eq!(
            placed,
            KernelLayout {
                kernel_offset: 0x8_0000,
                device_tree_offset: 126 * MIB,
                initrd_offset: Some(0x07CF_F000),
            },
            "the initrd page-aligned below the tree's block"
        );
        assert_eq!(
            layout(1024 * MIB, &image, 0).map(|l| l.device_tree_offset),
            Ok(510 * MIB),
            "within the 512 MiB a kernel before v4.2 searches"
        );
        let short = layout(3 * MIB, &image, 0).expect("fits");
        assert_eq!(short.device_tree_offset, 2 * MIB, "a block RAM ends inside");
        assert_eq!(short.fit_device_tree(3 * MIB, 8 * 1024), Ok(()));
        assert_eq!(
            layout(MIB, &image, 0),
            Err(PlacementError::DeviceTreeDoesNotFit),
            "the last block starts below the kernel's end"
        );
        assert_eq!(
            layout(4 * MIB, &image, 2 * MIB - 0x8_1000).map(|l| l.initrd_offset),
            Ok(Some(0x8_1000))
        );
        assert_eq!(
            layout(4 * MIB, &image, 2 * MIB - 0x8_0000),
            Err(PlacementError::InitrdDoesNotFit),
            "it would start in the image's bytes"
        );
    }
}
