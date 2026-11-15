//! A virtio-mmio transport with no device behind it.
//!
//! Provenance (R7): QEMU `hw/virtio/virtio-mmio.c` as `virt` instantiates its
//! transports; how many the board has and the window each decodes are its
//! layout's ([`crate::layout::VirtioWindows`]). Their register layout is the
//! `force-legacy` property:
//! legacy (QEMU's default, `Version` 1) on a board without virtio devices,
//! non-legacy (`Version` 2) on one with them (the `virtio` module says why).
//! With no backend plugged in, `virtio_mmio_read` answers the magic value,
//! the version and the vendor ID — the board's own
//! ([`crate::identity::BoardIdentity`]) — whatever the access size, reports
//! `DeviceID` 0 — "no device" in the virtio specification — and reads every
//! other register as zero; writes are ignored. Drivers rely on that to skip
//! the slot. QEMU's memory core splits an eight-byte access into two
//! four-byte ones, low half first.

use super::virtio::{MAGIC, MODERN_VERSION};
use crate::bus::{AccessWidth, RegOffset};
use crate::identity::VirtioVendorId;

const LEGACY_VERSION: u32 = 1;

/// The register layout of every transport on the board: QEMU's
/// `virtio-mmio.force-legacy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Legacy,
    Modern,
}

/// A read of `width` at `offset` of an empty transport on a board whose
/// transports have `layout` and whose `VendorID` is `vendor`.
pub fn read_empty_transport(
    offset: RegOffset,
    width: AccessWidth,
    layout: Layout,
    vendor: VirtioVendorId,
) -> u64 {
    let register = |off: u64| -> u64 {
        u64::from(match off {
            0x000 => MAGIC,
            0x004 => match layout {
                Layout::Legacy => LEGACY_VERSION,
                Layout::Modern => MODERN_VERSION,
            },
            0x00C => vendor.get(),
            _ => 0,
        })
    };
    match width {
        AccessWidth::Dword => register(offset.get()) | (register(offset.get() + 4) << 32),
        AccessWidth::Byte | AccessWidth::Half | AccessWidth::Word => {
            register(offset.get()) & width.value_mask()
        }
    }
}
