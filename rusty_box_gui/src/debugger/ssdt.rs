//! The System Service Descriptor Table (`KeServiceDescriptorTable`), decoded.
//!
//! `KeServiceDescriptorTable` is an array of `KSERVICE_TABLE_DESCRIPTOR`, each
//! 0x20 bytes on x64: `ServiceTable` (`KiServiceTable`) at +0x00, `CounterTable`
//! at +0x08, `ServiceLimit` (a `ULONG` count) at +0x10, `ParamTableBase`
//! (`KiArgumentTable`) at +0x18. The first descriptor is the kernel's services.
//!
//! Each `KiServiceTable` entry is a 32-bit signed value: an offset from the
//! table's own base, with the low 4 bits an argument hint. Windows 7 and later
//! shift the value right by 4 for the offset (`base + (entry >> 4)`); Windows
//! XP / Server 2003 x64 mask the low nibble instead (`base + (entry & !0xF)`).
//! The right one is the one whose routines land inside a loaded module.

/// `KSERVICE_TABLE_DESCRIPTOR.ServiceTable` (`KiServiceTable`) at +0x00.
pub(crate) const DESCRIPTOR_SERVICE_TABLE: u64 = 0x00;
/// `KSERVICE_TABLE_DESCRIPTOR.ServiceLimit` (a `ULONG`) at +0x10.
pub(crate) const DESCRIPTOR_SERVICE_LIMIT: u64 = 0x10;
/// More services than any Windows kernel has: a larger limit is a wrong read.
pub(crate) const MAX_SERVICES: u32 = 1 << 14;

/// How a build encodes a `KiServiceTable` entry into an offset from the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoding {
    /// Windows 7 and later: the offset is the entry shifted right by 4.
    Shift,
    /// Windows XP / Server 2003 x64: the offset is the entry's low nibble
    /// masked off.
    Mask,
}

/// The routine a `KiServiceTable` entry points to: `base` plus the offset the
/// `encoding` reads from `entry`. The offset is signed: a routine may lie
/// before the table.
pub(crate) fn routine_of(base: u64, entry: i32, encoding: Encoding) -> u64 {
    let offset = match encoding {
        Encoding::Shift => i64::from(entry >> 4),
        Encoding::Mask => i64::from(entry & !0xF),
    };
    base.wrapping_add_signed(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shift_encoding_matches_a_known_x64_entry() {
        // From a real x64 kernel: KiServiceTable at 0x1403FE450, entry 4 is
        // 0x02A07C00, which decodes to NtWaitForSingleObject at 0x14069EC10.
        assert_eq!(
            routine_of(0x1_403F_E450, 0x02A0_7C00u32 as i32, Encoding::Shift),
            0x1_4069_EC10
        );
    }

    #[test]
    fn the_mask_encoding_keeps_the_offset_unshifted() {
        assert_eq!(routine_of(0x1000, 0x0000_0024, Encoding::Mask), 0x1020);
        assert_eq!(routine_of(0x1000, 0x0000_0024, Encoding::Shift), 0x1002);
    }

    #[test]
    fn an_offset_can_point_before_the_table() {
        // A negative 32-bit entry decodes to a routine below the base.
        let entry = -0x100i32 << 4; // shifts back to -0x100
        assert_eq!(routine_of(0x1_0000, entry, Encoding::Shift), 0xFF00);
    }
}
