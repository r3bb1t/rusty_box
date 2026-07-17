#![allow(dead_code)]
/// 4M BIOS ROM @0xffc00000, must be a power of 2
pub(super) static BIOSROMSZ: usize = 1 << 22;
/// ROMs 0xc0000-0xdffff (area 0xe0000-0xfffff=bios mapped)
pub(super) static EXROMSIZE: usize = 0x20000;

pub(super) static BIOS_MASK: usize = BIOSROMSZ - 1;
pub(super) static EXROM_MASK: usize = EXROMSIZE - 1;

pub(super) fn bios_map_last128k(addr: usize) -> usize {
    ((addr) | 0xfff00000) & BIOS_MASK
}

// PCI hole constants for systems with >3GB RAM
pub(crate) const BX_PCI_HOLE_START: u64 = 0xC000_0000; // 3GB
pub(crate) const BX_PCI_HOLE_END: u64 = 0x1_0000_0000; // 4GB
pub(crate) const BX_PCI_HOLE_SIZE: u64 = 0x4000_0000; // 1GB

/// Returns true if the guest physical address falls in the PCI MMIO hole (3GB-4GB).
#[inline]
pub(crate) fn bx_is_pci_hole_addr(gpa: u64) -> bool {
    gpa >= BX_PCI_HOLE_START && gpa < BX_PCI_HOLE_END
}

/// Translate a guest physical address to a linear RAM offset.
#[inline]
pub(crate) fn bx_translate_gpa_to_linear(gpa: u64) -> u64 {
    if gpa >= BX_PCI_HOLE_END {
        gpa - BX_PCI_HOLE_SIZE
    } else {
        gpa
    }
}

/// Return the complete linear-RAM span for a guest physical access.
///
/// The PCI aperture is never RAM. Addresses above it are stored densely after
/// the aperture, matching Bochs's physical-to-host-RAM translation. `None`
/// means any requested byte is not direct guest RAM; an empty span retains its
/// checked translated insertion point.
#[inline]
pub(crate) fn bx_guest_ram_span(
    gpa: u64,
    len: usize,
    ram_len: usize,
) -> Option<core::ops::Range<usize>> {
    let end_gpa = gpa.checked_add(u64::try_from(len).ok()?)?;
    if len != 0 && gpa < BX_PCI_HOLE_END && end_gpa > BX_PCI_HOLE_START {
        return None;
    }
    let start = usize::try_from(bx_translate_gpa_to_linear(gpa)).ok()?;
    let end = start.checked_add(len)?;
    (end <= ram_len).then_some(start..end)
}

pub(super) enum MemoryAreaT {
    C0000 = 0,
    C4000,
    C8000,
    CC000,
    D0000,
    D4000,
    D8000,
    DC000,
    E0000,
    E4000,
    E8000,
    EC000,
    F0000,
}
