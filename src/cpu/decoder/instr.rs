use bitflags::bitflags;

#[derive(Debug)]
pub struct BxInstruction {}

#[derive(Debug)]
pub enum BxDisasmStyle {
    Intel,
    Gas,
}

pub(crate) const BX_LOCK_PREFIX_USED: bool = true;

bitflags! {
    /// Flags for the metaInfo1 field
    #[derive(Debug, Default)]
    pub struct MetaInfoFlags: u8 {
        const AS32 = 0b0000_0001; // Bit 0
        const AS64 = 0b0000_0010; // Bit 1
        const OS32 = 0b0000_0100; // Bit 2
        const OS64 = 0b0000_1000; // Bit 3
        const MOD_C0 = 0b0001_0000; // Bit 4
        const EXTEND_8BIT = 0b0010_0000; // Bit 5
        const REP_USED = 0b1100_0000; // Bits 6-7 (0=none, 1=0xF0, 2=0xF2, 3=0xF3)
    }
}

struct MetaInfo {
    // 15...0 opcode
    pub ia_opcode: u16,

    ///  7...4 (unused)
    ///  3...0 ilen (0..15)
    pub ilen: u8,

    ///  7...6 lockUsed, repUsed (0=none, 1=0xF0, 2=0xF2, 3=0xF3)
    ///  5...5 extend8bit
    ///  4...4 mod==c0 (modrm)
    ///  3...3 os64
    ///  2...2 os32
    ///  1...1 as64
    ///  0...0 as32
    pub meta_info1: MetaInfoFlags,
}

impl MetaInfoFlags {
    pub(crate) fn set_os32_b(&mut self, bit: bool) {
        let new_value_raw = self.bits() & !(1 << 2) | ((bit as u8) << 2);
        *self = Self::from_bits_truncate(new_value_raw)
    }
}
