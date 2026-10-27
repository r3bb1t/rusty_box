//! A virtio-mmio transport with no device behind it.
//!
//! Provenance (R7): QEMU `hw/virtio/virtio-mmio.c` as `virt` instantiates its
//! 32 transports. Their register layout is the `force-legacy` property:
//! legacy (QEMU's default, `Version` 1) on a board without virtio devices,
//! non-legacy (`Version` 2) on one with them (the `virtio` module says why).
//! With no backend plugged in, `virtio_mmio_read` answers the magic value,
//! the version and the vendor ID whatever the access size, reports
//! `DeviceID` 0 — "no device" in the virtio specification — and reads every
//! other register as zero; writes are ignored. Drivers rely on that to skip
//! the slot. QEMU's memory core splits an eight-byte access into two
//! four-byte ones, low half first.

use crate::bus::{AccessWidth, RegOffset};

use super::virtio::{MAGIC, MODERN_VERSION, VENDOR_QEMU};

const LEGACY_VERSION: u32 = 1;

/// Bytes of register space each transport occupies on `virt`.
pub const TRANSPORT_SIZE: u64 = 0x200;
/// Transports `virt` creates.
pub const TRANSPORT_COUNT: u64 = 32;

/// The register layout of every transport on the board: QEMU's
/// `virtio-mmio.force-legacy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Legacy,
    Modern,
}

pub fn read_empty_transport(offset: RegOffset, width: AccessWidth, layout: Layout) -> u64 {
    let register = |off: u64| -> u64 {
        u64::from(match off {
            0x000 => MAGIC,
            0x004 => match layout {
                Layout::Legacy => LEGACY_VERSION,
                Layout::Modern => MODERN_VERSION,
            },
            0x00C => VENDOR_QEMU,
            _ => 0,
        })
    };
    match width {
        AccessWidth::Dword => register(offset.get()) | (register(offset.get() + 4) << 32),
        AccessWidth::Byte | AccessWidth::Half | AccessWidth::Word => register(offset.get()) & width.value_mask(),
    }
}
