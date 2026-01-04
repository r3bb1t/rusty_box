/// 4M BIOS ROM @0xffc00000, must be a power of 2
pub(super) static BIOSROMSZ: usize = 1 << 22;
/// ROMs 0xc0000-0xdffff (area 0xe0000-0xfffff=bios mapped)
pub(super) static EXROMSIZE: usize = 0x20000;

pub(super) static BIOS_MASK: usize = BIOSROMSZ - 1;
pub(super) static EXROM_MASK: usize = EXROMSIZE - 1;

pub(super) fn bios_map_last128k(addr: usize) -> usize {
    ((addr) | 0xfff00000) & BIOS_MASK
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
