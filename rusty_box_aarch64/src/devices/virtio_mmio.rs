//! A virtio-mmio transport with no device behind it.
//!
//! Provenance (R7): QEMU `hw/virtio/virtio-mmio.c` as `virt` instantiates its
//! 32 transports: legacy register layout (`force-legacy` is the default for
//! the machine types whose device tree this board reuses), so `Version`
//! reads 1. With no backend plugged in, QEMU reports `DeviceID` 0 — "no
//! device" in the virtio specification — and every other register reads as
//! zero and ignores writes, which is what drivers rely on to skip the slot.

use crate::bus::{AccessWidth, RegOffset};

/// "virt", little-endian.
const MAGIC: u32 = 0x7472_6976;
const LEGACY_VERSION: u32 = 1;
/// QEMU's vendor ID, "QEMU" little-endian.
const VENDOR_QEMU: u32 = 0x554D_4551;

/// Bytes of register space each transport occupies on `virt`.
pub const TRANSPORT_SIZE: u64 = 0x200;
/// Transports `virt` creates.
pub const TRANSPORT_COUNT: u64 = 32;

pub fn read_empty_transport(offset: RegOffset, _width: AccessWidth) -> u64 {
    u64::from(match offset.get() {
        0x000 => MAGIC,
        0x004 => LEGACY_VERSION,
        0x00C => VENDOR_QEMU,
        _ => 0,
    })
}
