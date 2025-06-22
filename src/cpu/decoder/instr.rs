use bitflags::bitflags;

use super::{
    ia_opcodes::Opcode,
    instr_generated::{BxInstructionGenerated, BxInstructionMetaInfo, ModRmForm},
    DecodeResult,
};

#[derive(Debug, Default, Clone)]
pub struct BxInstruction {
    pub metainfo: MetaInfo,
    // using 5-bit field for registers (16 regs in 64-bit, RIP, NIL)
    pub meta_data: [u8; 8],
}

impl TryFrom<BxInstructionGenerated> for BxInstruction {
    type Error = super::error::DecodeError;

    fn try_from(value: BxInstructionGenerated) -> DecodeResult<Self> {
        let meta = value.meta_info;

        let metainfo = MetaInfo {
            ia_opcode: meta.ia_opcode,
            ilen: meta.ilen,
            meta_info1: MetaInfoFlags::from_bits_retain(meta.metainfo1),
        };

        let instruction = Self {
            metainfo,
            meta_data: value.meta_data,
        };

        Ok(instruction)
    }
}

impl TryFrom<BxInstruction> for BxInstructionGenerated {
    type Error = super::error::DecodeError;

    fn try_from(value: BxInstruction) -> DecodeResult<Self> {
        let meta = value.metainfo;

        let meta_info = BxInstructionMetaInfo {
            ia_opcode: meta.ia_opcode as _,
            ilen: meta.ilen,
            metainfo1: meta.meta_info1.bits(),
        };

        let instruction_generated = Self {
            opcode: value.metainfo.ia_opcode,
            meta_info,
            meta_data: value.meta_data,
            // NOTE: Losing data here
            modrm_form: ModRmForm::default(),
        };

        Ok(instruction_generated)
    }
}

impl BxInstruction {
    #[inline]
    pub(super) fn osize(&self) -> u32 {
        u32::from((self.metainfo.meta_info1.bits() >> 2) & 0x3)
    }

    #[inline]
    pub(super) fn asize(&self) -> u32 {
        u32::from(self.metainfo.meta_info1.bits() & 0x3)
    }
}

#[derive(Debug)]
pub enum BxDisasmStyle {
    Intel,
    Gas,
}

pub union ModRmFirstForm {
    Id: u32,
    Iw: [u16; 2],
    // use Ib[3] as EVEX mask register
    // use Ib[2] as AVX attributes
    //     7..5 (unused)
    //     4..4 VEX.W
    //     3..3 Broadcast/RC/SAE control (EVEX.b)
    //     2..2 Zeroing/Merging mask (EVEX.z)
    //     1..0 Round control
    // use Ib[1] as AVX VL
    Ib: [u8; 4],
}

pub union ModRmSecondForm {
    displ16u: u16, // for 16-bit modrm forms
    displ32u: u32, // for 32-bit modrm forms

    Id2: u32,
    Iw2: [u16; 2],
    Ib2: [u8; 4],
}
pub struct ModRmform {
    a: ModRmFirstForm,
    b: ModRmSecondForm,
}

pub(crate) const BX_LOCK_PREFIX_USED: bool = true;

bitflags! {
    /// Flags for the metaInfo1 field
    #[derive(Debug, Default, Clone, Copy)]
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

#[derive(Debug, Clone, Copy)]
pub struct MetaInfo {
    // 15...0 opcode
    pub ia_opcode: Opcode,

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

impl Default for MetaInfo {
    fn default() -> Self {
        Self {
            ia_opcode: Opcode::IaError,
            ilen: Default::default(),
            meta_info1: Default::default(),
        }
    }
}

impl MetaInfoFlags {
    pub(crate) fn set_os32_b(&mut self, bit: bool) {
        let new_value_raw = self.bits() & !(1 << 2) | ((bit as u8) << 2);
        *self = Self::from_bits_truncate(new_value_raw)
    }

    pub(super) fn set_lock_rep_used(&mut self, value: u32) {
        let new_value_raw = u32::from(self.bits() & 0x3F) | (value << 6);
        *self = Self::from_bits_truncate(new_value_raw as u8)
    }

    pub(super) fn mod_c0(&self) -> u32 {
        // This is a cheaper way to test for modRM instructions where
        // the mod field is 0xc0.  FetchDecode flags this condition since
        // it is quite common to be tested for.
        u32::from(self.bits() & (1 << 4))
    }
}

impl BxInstructionGenerated {
    #[inline]
    pub(super) fn osize(&self) -> u32 {
        u32::from((self.meta_info.metainfo1 >> 2) & 0x3)
    }

    #[inline]
    pub(super) fn asize(&self) -> u32 {
        u32::from(self.meta_info.metainfo1 & 0x3)
    }

    #[inline]
    pub(super) fn mod_c0(&self) -> u8 {
        self.meta_info.metainfo1 & (1 << 4)
    }

    pub(super) fn init(&mut self, os32: u32, as32: u32, os64: u32, as64: u32) {
        self.meta_info.metainfo1 = ((os32 << 2) | (os64 << 3) | (as32 << 0) | (as64 << 1)) as u8;
    }

    pub(super) fn set_as32_b(&mut self, bit: bool) {
        self.meta_info.metainfo1 = (self.meta_info.metainfo1 & !0x1) | (bit as u8);
    }

    pub(super) fn assert_mod_c0(&mut self) {
        self.meta_info.metainfo1 |= 1 << 4;
    }
}
