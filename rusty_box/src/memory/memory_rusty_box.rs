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

/// The guest-physical address of RAM-store offset `offset`: the inverse of
/// [`bx_translate_gpa_to_linear`] over RAM. RAM below the PCI hole sits at
/// its own address; the rest sits from 4 GiB up.
#[inline]
pub(crate) fn bx_translate_linear_to_gpa(offset: u64) -> u64 {
    if offset >= BX_PCI_HOLE_START {
        offset + BX_PCI_HOLE_SIZE
    } else {
        offset
    }
}

/// How many bytes from RAM-store `offset` lie on one side of the hole, at
/// most `len`: a read of RAM in storage order never spans the hole.
#[inline]
pub(crate) fn bx_ram_stretch(offset: u64, len: usize) -> usize {
    if offset < BX_PCI_HOLE_START {
        usize::try_from(BX_PCI_HOLE_START - offset).map_or(len, |left| left.min(len))
    } else {
        len
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ram_offset_maps_back_to_the_address_it_came_from() {
        for gpa in [0, 0x1000, BX_PCI_HOLE_START - 1, BX_PCI_HOLE_END, BX_PCI_HOLE_END + 0x1234] {
            assert_eq!(bx_translate_linear_to_gpa(bx_translate_gpa_to_linear(gpa)), gpa);
        }
    }

    #[test]
    fn a_stretch_stops_at_the_hole_and_runs_free_above_it() {
        assert_eq!(bx_ram_stretch(BX_PCI_HOLE_START - 16, 64), 16);
        assert_eq!(bx_ram_stretch(0, 64), 64);
        assert_eq!(bx_ram_stretch(BX_PCI_HOLE_START, 64), 64);
    }
}
