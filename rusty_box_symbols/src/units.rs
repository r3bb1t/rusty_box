//! The units an address or a structure layout is measured in (R4): an offset
//! from a module's base is not an address, and a member's offset is neither.

/// An offset from a module's load base. For a PE image it is the image's
/// RVA; for an ELF file it is the link-time address minus the file's link
/// base, so `load base + Rva` is the guest address either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rva(pub u64);

/// A member's byte offset from the start of the structure that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldOffset(pub u64);

/// A type as a PDB's TPI stream indexes it: below 0x1000 a primitive type
/// (its kind and pointer mode), from 0x1000 a type record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeRef(pub u32);

/// A register as CodeView numbers it (LLVM `CodeViewRegisters.def`): the
/// number a variable's record names, before it is mapped to a machine
/// register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CvRegister(pub u16);

/// A bitfield's bits inside the storage unit at its member's offset, least
/// significant bit first: `position` is the lowest bit, `length` the width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitRange {
    pub position: u32,
    pub length: u32,
}

impl BitRange {
    /// The field's value inside `unit`, the storage unit as read from memory.
    /// A range reaching past bit 63 reads the bits that exist; one starting
    /// past it reads 0.
    #[must_use]
    pub fn extract(self, unit: u64) -> u64 {
        let shifted = unit.checked_shr(self.position).unwrap_or(0);
        // `1 << length` overflows exactly when every remaining bit is kept.
        let mask = 1u64
            .checked_shl(self.length)
            .map_or(u64::MAX, |bit| bit.wrapping_sub(1));
        shifted & mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bitfield_reads_its_bits_from_the_least_significant_end() {
        let flags = BitRange { position: 5, length: 3 };
        assert_eq!(flags.extract(0b1110_0000), 0b111);
        assert_eq!(flags.extract(0b1010_0000), 0b101);
        assert_eq!(flags.extract(0b0001_1111), 0);
    }

    #[test]
    fn a_bitfield_at_the_top_bit_or_past_it_reads_without_panicking() {
        assert_eq!(BitRange { position: 63, length: 1 }.extract(u64::MAX), 1);
        assert_eq!(BitRange { position: 64, length: 1 }.extract(u64::MAX), 0);
        assert_eq!(BitRange { position: u32::MAX, length: u32::MAX }.extract(u64::MAX), 0);
        assert_eq!(BitRange { position: 0, length: 64 }.extract(0xDEAD_BEEF), 0xDEAD_BEEF);
        assert_eq!(BitRange { position: 0, length: 200 }.extract(7), 7);
        assert_eq!(BitRange { position: 3, length: 0 }.extract(u64::MAX), 0);
    }
}
