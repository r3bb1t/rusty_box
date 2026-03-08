use core::fmt::Debug;

use super::{ia_opcodes::Opcode, instr::MetaInfoFlags, BxSegregs};

// Metadata array indices - matching original Bochs structure
pub(crate) const BX_INSTR_METADATA_DST: usize = 0;
pub(crate) const BX_INSTR_METADATA_SRC1: usize = 1;
#[allow(dead_code)]
pub(crate) const BX_INSTR_METADATA_SRC2: usize = 2;
#[allow(dead_code)]
pub(crate) const BX_INSTR_METADATA_SRC3: usize = 3;
const BX_INSTR_METADATA_CET_SEGOVERRIDE: usize = 3; // share src3
pub(crate) const BX_INSTR_METADATA_SEG: usize = 4;
pub(crate) const BX_INSTR_METADATA_BASE: usize = 5;
pub(crate) const BX_INSTR_METADATA_INDEX: usize = 6;
pub(crate) const BX_INSTR_METADATA_SCALE: usize = 7;

// MetaInfo1 bit flags - now using MetaInfoFlags from instr.rs
// Keeping BX_LOCK_PREFIX_USED as it's used as a value (1), not a bit flag
const BX_LOCK_PREFIX_USED: u8 = 1;

/// Instruction structure matching the original Bochs bxInstruction_c
///
/// This structure holds decoded instruction information including:
/// - Opcode and instruction length
/// - Operand metadata (registers, segments, addressing)
/// - Immediate values and displacements
/// - Prefix information (lock, rep, size overrides)
///
/// Note: In the original C++, there's a union between modRMForm and IqForm.
/// In Rust, we use modrm_form as the primary field. The IqForm (64-bit immediate)
/// can be accessed via the iq() method which safely interprets the same memory.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct Instruction {
    pub meta_info: BxInstructionMetaInfo,
    pub meta_data: [u8; 8],
    /// ModRM form - also used as storage for 64-bit immediates (IqForm) in x86-64
    pub modrm_form: ModRmForm,
}

/// Instruction metadata matching original structure
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct BxInstructionMetaInfo {
    /// IA-32 opcode (15 bits)
    pub ia_opcode: Opcode,
    /// Instruction length (0-15)
    pub ilen: u8,
    /// Meta information flags using bitflags for type safety
    ///  7..6: lockUsed, repUsed (0=none, 1=0xF0, 2=0xF2, 3=0xF3)
    ///  5:    extend8bit
    ///  4:    mod==c0 (modrm)
    ///  3:    os64
    ///  2:    os32
    ///  1:    as64
    ///  0:    as32
    pub metainfo1: MetaInfoFlags,
}

/// ModRM form structure - holds operand and displacement data
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ModRmForm {
    /// Operand data union: Id (u32), Iw (u16[2]), or Ib (u8[4])
    /// Also used for AVX/EVEX attributes:
    ///   Ib[3]: EVEX mask register
    ///   Ib[2]: AVX attributes (VEX.W, EVEX.b, EVEX.z, Round control)
    ///   Ib[1]: AVX VL
    pub operand_data: OperandData,
    /// Displacement data union: displ16u, displ32u, Id2, Iw2, or Ib2
    pub displacement: DisplacementData,
}

/// Operand data - safe wrapper around union
#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct OperandData {
    /// As u32 (Id)
    pub id: u32,
}

impl Debug for OperandData {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "OperandData(Id:{:#x}, Iw:[{:?}], Ib:[{:?}])",
            self.id,
            self.iw(),
            self.ib()
        )
    }
}

impl OperandData {
    /// Access as u32 (Id)
    #[inline]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Access as u16[2] (Iw)
    #[inline]
    pub const fn iw(&self) -> [u16; 2] {
        let bytes = self.id.to_ne_bytes();
        [
            u16::from_ne_bytes([bytes[0], bytes[1]]),
            u16::from_ne_bytes([bytes[2], bytes[3]]),
        ]
    }

    /// Access as u8[4] (Ib)
    #[inline]
    pub fn ib(&self) -> [u8; 4] {
        u32::to_ne_bytes(self.id)
    }

    /// Set as u32
    #[inline]
    pub fn set_id(&mut self, val: u32) {
        self.id = val;
    }

    /// Set as u16[2]
    #[inline]
    pub fn set_iw(&mut self, val: [u16; 2]) {
        let b0 = val[0].to_ne_bytes();
        let b1 = val[1].to_ne_bytes();
        self.id = u32::from_ne_bytes([b0[0], b0[1], b1[0], b1[1]]);
    }

    /// Set as u8[4]
    #[inline]
    pub fn set_ib(&mut self, val: [u8; 4]) {
        self.id = u32::from_ne_bytes(val);
    }
}

/// Displacement data - safe wrapper around union
#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct DisplacementData {
    /// As u32 (displ32u or Id2)
    pub data32: u32,
}

impl Debug for DisplacementData {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "DisplacementData(displ16u:{}, displ32u:{:#x}, Id2:{:#x})",
            self.displ16u(),
            self.displ32u(),
            self.id2()
        )
    }
}

impl DisplacementData {
    /// Access as u16 (displ16u) - derived from low 16 bits of data32
    #[inline]
    pub const fn displ16u(&self) -> u16 {
        self.data32 as u16
    }

    /// Access as u32 (displ32u)
    #[inline]
    pub const fn displ32u(&self) -> u32 {
        self.data32
    }

    /// Access as u32 (Id2)
    #[inline]
    pub const fn id2(&self) -> u32 {
        self.data32
    }

    /// Access as u16[2] (Iw2)
    #[inline]
    pub const fn iw2(&self) -> [u16; 2] {
        let bytes = self.data32.to_ne_bytes();
        [
            u16::from_ne_bytes([bytes[0], bytes[1]]),
            u16::from_ne_bytes([bytes[2], bytes[3]]),
        ]
    }

    /// Access as u8[4] (Ib2)
    #[inline]
    pub fn ib2(&self) -> [u8; 4] {
        u32::to_ne_bytes(self.data32)
    }

    /// Set as u32 (displ32u)
    #[inline]
    pub fn set_displ32u(&mut self, val: u32) {
        self.data32 = val;
    }

    /// Set as u32 (Id2)
    #[inline]
    pub fn set_id2(&mut self, val: u32) {
        self.data32 = val;
    }
}

impl Instruction {
    // ============================================================
    // MetaInfo accessors
    // ============================================================

    /// Get instruction length
    #[inline]
    pub const fn ilen(&self) -> u8 {
        self.meta_info.ilen
    }

    /// Set instruction length
    #[inline]
    pub fn set_ilen(&mut self, ilen: u8) {
        self.meta_info.ilen = ilen;
    }

    /// Get IA-32 opcode
    #[inline]
    pub const fn get_ia_opcode(&self) -> Opcode {
        self.meta_info.ia_opcode
    }

    /// Set IA-32 opcode
    #[inline]
    pub fn set_ia_opcode(&mut self, op: Opcode) {
        self.meta_info.ia_opcode = op;
    }

    // ============================================================
    // Operand size flags (os32L, os64L, osize)
    // ============================================================

    /// Initialize operand and address size flags
    #[inline]
    pub fn init(&mut self, os32: u8, as32: u8, os64: u8, as64: u8) {
        let mut flags = MetaInfoFlags::empty();
        if os32 != 0 {
            flags |= MetaInfoFlags::Os32;
        }
        if os64 != 0 {
            flags |= MetaInfoFlags::Os64;
        }
        if as32 != 0 {
            flags |= MetaInfoFlags::As32;
        }
        if as64 != 0 {
            flags |= MetaInfoFlags::As64;
        }
        self.meta_info.metainfo1 = flags;
    }

    /// Get os32L (logical value, 0 or non-zero)
    #[inline]
    pub const fn os32_l(&self) -> u8 {
        self.meta_info.metainfo1.bits() & MetaInfoFlags::Os32.bits()
    }

    /// Set os32B (boolean)
    #[inline]
    pub fn set_os32_b(&mut self, bit: bool) {
        self.meta_info.metainfo1.set(MetaInfoFlags::Os32, bit);
    }

    /// Assert os32
    #[inline]
    pub fn assert_os32(&mut self) {
        self.meta_info.metainfo1 |= MetaInfoFlags::Os32;
    }

    /// Get os64L (logical value, 0 or non-zero)
    #[inline]
    pub const fn os64_l(&self) -> u8 {
        self.meta_info.metainfo1.bits() & MetaInfoFlags::Os64.bits()
    }

    /// Assert os64
    #[inline]
    pub fn assert_os64(&mut self) {
        self.meta_info.metainfo1 |= MetaInfoFlags::Os64;
    }

    /// Get operand size (0=16-bit, 1=32-bit, 2=64-bit)
    #[inline]
    pub const fn osize(&self) -> u8 {
        let bits = self.meta_info.metainfo1.bits();
        (bits >> 2) & 0x3
    }

    // ============================================================
    // Address size flags (as32L, as64L, asize)
    // ============================================================

    /// Get as32L (logical value, 0 or non-zero)
    #[inline]
    pub const fn as32_l(&self) -> u8 {
        self.meta_info.metainfo1.bits() & MetaInfoFlags::As32.bits()
    }

    /// Set as32B (boolean)
    #[inline]
    pub fn set_as32_b(&mut self, bit: bool) {
        self.meta_info.metainfo1.set(MetaInfoFlags::As32, bit);
    }

    /// Get as64L (logical value, 0 or non-zero)
    #[inline]
    pub const fn as64_l(&self) -> u8 {
        self.meta_info.metainfo1.bits() & MetaInfoFlags::As64.bits()
    }

    /// Clear as64
    #[inline]
    pub fn clear_as64(&mut self) {
        self.meta_info.metainfo1.remove(MetaInfoFlags::As64);
    }

    /// Get address size (0=16-bit, 1=32-bit, 2=64-bit)
    #[inline]
    pub const fn asize(&self) -> u8 {
        self.meta_info.metainfo1.bits() & 0x3
    }

    /// Get extend8bitL (for 64-bit mode)
    #[inline]
    pub const fn extend8bit_l(&self) -> u8 {
        self.meta_info.metainfo1.bits() & MetaInfoFlags::Extend8bit.bits()
    }

    /// Assert extend8bit
    #[inline]
    pub fn assert_extend8bit(&mut self) {
        self.meta_info.metainfo1 |= MetaInfoFlags::Extend8bit;
    }

    // ============================================================
    // Lock/Rep prefix flags
    // ============================================================

    /// Get repUsedL (0=none, 1=0xF0, 2=0xF2, 3=0xF3)
    #[inline]
    pub const fn rep_used_l(&self) -> u8 {
        self.meta_info.metainfo1.bits() >> 7
    }

    /// Get lockRepUsedValue (0=none, 1=0xF0, 2=0xF2, 3=0xF3)
    #[inline]
    pub const fn lock_rep_used_value(&self) -> u8 {
        (self.meta_info.metainfo1.bits() >> 6) & 0x3
    }

    /// Set lockRepUsed
    #[inline]
    pub fn set_lock_rep_used(&mut self, value: u8) {
        let bits = self.meta_info.metainfo1.bits();
        let new_bits = (bits & 0x3f) | (value << 6);
        self.meta_info.metainfo1 = MetaInfoFlags::from_bits_truncate(new_bits);
    }

    /// Set lock prefix
    #[inline]
    pub fn set_lock(&mut self) {
        self.set_lock_rep_used(BX_LOCK_PREFIX_USED);
    }

    /// Get lock prefix
    #[inline]
    pub const fn get_lock(&self) -> bool {
        self.lock_rep_used_value() == BX_LOCK_PREFIX_USED
    }

    /// Get modC0 (mod==0xc0 in ModRM)
    #[inline]
    pub const fn mod_c0(&self) -> bool {
        (self.meta_info.metainfo1.bits() & MetaInfoFlags::ModC0.bits()) != 0
    }

    /// Assert modC0
    #[inline]
    pub fn assert_mod_c0(&mut self) {
        self.meta_info.metainfo1 |= MetaInfoFlags::ModC0;
    }

    // ============================================================
    // Register operand accessors
    // ============================================================

    /// Get destination register
    #[inline]
    pub const fn dst(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_DST]
    }

    /// Get source register 1
    #[inline]
    pub const fn src1(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_SRC1]
    }

    /// Get source register 2
    #[inline]
    pub const fn src2(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_SRC2]
    }

    /// Get source register 3
    #[inline]
    pub const fn src3(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_SRC3]
    }

    /// Get source register (alias for src1)
    #[inline]
    pub const fn src(&self) -> u8 {
        self.src1()
    }

    /// Set source register
    #[inline]
    pub fn set_src_reg(&mut self, src: usize, reg: u8) {
        if src < 4 {
            self.meta_data[src] = reg;
        }
    }

    /// Get source register by index
    #[inline]
    pub const fn get_src_reg(&self, src: usize) -> u8 {
        if src < 4 {
            self.meta_data[src]
        } else {
            0
        }
    }

    // ============================================================
    // Segment register accessors
    // ============================================================

    /// Get segment register
    #[inline]
    pub const fn seg(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_SEG]
    }

    /// Set segment register
    #[inline]
    pub fn set_seg(&mut self, val: BxSegregs) {
        self.meta_data[BX_INSTR_METADATA_SEG] = val as u8;
    }

    /// Get CET segment override
    #[inline]
    pub const fn seg_override_cet(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_CET_SEGOVERRIDE]
    }

    /// Set CET segment override
    #[inline]
    pub fn set_cet_seg_override(&mut self, val: BxSegregs) {
        self.meta_data[BX_INSTR_METADATA_CET_SEGOVERRIDE] = val as u8;
    }

    // ============================================================
    // SIB (Scale-Index-Base) accessors
    // ============================================================

    /// Set SIB scale
    #[inline]
    pub fn set_sib_scale(&mut self, scale: u8) {
        self.meta_data[BX_INSTR_METADATA_SCALE] = scale;
    }

    /// Get SIB scale
    #[inline]
    pub const fn sib_scale(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_SCALE]
    }

    /// Set SIB index
    #[inline]
    pub fn set_sib_index(&mut self, index: u8) {
        self.meta_data[BX_INSTR_METADATA_INDEX] = index;
    }

    /// Get SIB index
    #[inline]
    pub const fn sib_index(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_INDEX]
    }

    /// Set SIB base
    #[inline]
    pub fn set_sib_base(&mut self, base: u8) {
        self.meta_data[BX_INSTR_METADATA_BASE] = base;
    }

    /// Get SIB base
    #[inline]
    pub const fn sib_base(&self) -> u8 {
        self.meta_data[BX_INSTR_METADATA_BASE]
    }

    // ============================================================
    // Immediate value accessors
    // ============================================================

    /// Get 32-bit immediate (Id)
    #[inline]
    pub fn id(&self) -> u32 {
        self.modrm_form.operand_data.id()
    }

    /// Get 16-bit immediate (Iw)
    #[inline]
    pub fn iw(&self) -> u16 {
        self.modrm_form.operand_data.iw()[0]
    }

    /// Get 8-bit immediate (Ib)
    #[inline]
    pub fn ib(&self) -> u8 {
        self.modrm_form.operand_data.ib()[0]
    }

    /// Get 64-bit immediate (Iq) - x86-64 only
    ///
    /// Note: This accesses the same memory as modrm_form (union in C++)
    /// In C++, IqForm overlaps with the first 8 bytes of modRMForm
    #[inline]
    pub fn iq(&self) -> u64 {
        unsafe {
            // Read the first 8 bytes of ModRmForm as u64
            // This matches C++ where IqForm.Iq overlaps with modRMForm
            core::ptr::read(&self.modrm_form as *const ModRmForm as *const u64)
        }
    }

    /// Set 64-bit immediate (Iq) - x86-64 only
    #[inline]
    pub fn set_iq(&mut self, val: u64) {
        unsafe {
            // Write u64 to the first 8 bytes of ModRmForm
            // This matches C++ where IqForm.Iq overlaps with modRMForm
            core::ptr::write(&mut self.modrm_form as *mut ModRmForm as *mut u64, val);
        }
    }

    /// Get second 32-bit immediate (Id2)
    #[inline]
    pub fn id2(&self) -> u32 {
        self.modrm_form.displacement.id2()
    }

    /// Get second 16-bit immediate (Iw2)
    #[inline]
    pub fn iw2(&self) -> u16 {
        self.modrm_form.displacement.iw2()[0]
    }

    /// Get second 8-bit immediate (Ib2)
    #[inline]
    pub fn ib2(&self) -> u8 {
        self.modrm_form.displacement.ib2()[0]
    }

    // ============================================================
    // Displacement accessors
    // ============================================================

    /// Get 32-bit signed displacement
    #[inline]
    pub fn displ32s(&self) -> i32 {
        self.modrm_form.displacement.displ32u() as i32
    }

    /// Get 16-bit signed displacement
    #[inline]
    pub fn displ16s(&self) -> i16 {
        self.modrm_form.displacement.displ16u() as i16
    }

    /// Get 32-bit unsigned displacement
    #[inline]
    pub fn displ32u(&self) -> u32 {
        self.modrm_form.displacement.displ32u()
    }

    /// Get 16-bit unsigned displacement
    #[inline]
    pub fn displ16u(&self) -> u16 {
        self.modrm_form.displacement.displ16u()
    }

    // ============================================================
    // AVX/EVEX attribute accessors
    // ============================================================

    /// Get AVX vector length (VL)
    #[inline]
    pub fn get_vl(&self) -> u8 {
        self.modrm_form.operand_data.ib()[1]
    }

    /// Set AVX vector length (VL)
    #[inline]
    pub fn set_vl(&mut self, value: u8) {
        let mut ib = self.modrm_form.operand_data.ib();
        ib[1] = value;
        self.modrm_form.operand_data.set_ib(ib);
    }

    /// Get VEX.W bit
    #[inline]
    pub fn get_vex_w(&self) -> u8 {
        (self.modrm_form.operand_data.ib()[2] >> 4) & 1
    }

    /// Set VEX.W bit
    #[inline]
    pub fn set_vex_w(&mut self, bit: u8) {
        let mut ib = self.modrm_form.operand_data.ib();
        ib[2] = (ib[2] & !0x10) | ((bit & 1) << 4);
        self.modrm_form.operand_data.set_ib(ib);
    }

    /// Get EVEX opmask register
    #[inline]
    pub fn opmask(&self) -> u8 {
        self.modrm_form.operand_data.ib()[3]
    }

    /// Set EVEX opmask register
    #[inline]
    pub fn set_opmask(&mut self, reg: u8) {
        let mut ib = self.modrm_form.operand_data.ib();
        ib[3] = reg;
        self.modrm_form.operand_data.set_ib(ib);
    }

    /// Get EVEX.b bit (broadcast/RC/SAE control)
    #[inline]
    pub fn get_evex_b(&self) -> u8 {
        (self.modrm_form.operand_data.ib()[2] >> 3) & 1
    }

    /// Set EVEX.b bit
    #[inline]
    pub fn set_evex_b(&mut self, bit: u8) {
        let mut ib = self.modrm_form.operand_data.ib();
        ib[2] = (ib[2] & !0x8) | ((bit & 1) << 3);
        self.modrm_form.operand_data.set_ib(ib);
    }

    /// Get zero masking bit (EVEX.z)
    #[inline]
    pub fn is_zero_masking(&self) -> u8 {
        (self.modrm_form.operand_data.ib()[2] >> 2) & 1
    }

    /// Set zero masking bit
    #[inline]
    pub fn set_zero_masking(&mut self, bit: u8) {
        let mut ib = self.modrm_form.operand_data.ib();
        ib[2] = (ib[2] & !0x4) | ((bit & 1) << 2);
        self.modrm_form.operand_data.set_ib(ib);
    }

    /// Get rounding control (RC)
    #[inline]
    pub fn get_rc(&self) -> u8 {
        self.modrm_form.operand_data.ib()[2] & 0x3
    }

    /// Set rounding control (RC)
    #[inline]
    pub fn set_rc(&mut self, rc: u8) {
        let mut ib = self.modrm_form.operand_data.ib();
        ib[2] = (ib[2] & !0x3) | (rc & 0x3);
        self.modrm_form.operand_data.set_ib(ib);
    }

    // ============================================================
    // Helper methods for instruction handlers
    // ============================================================

    /// Get first byte of opcode (b1) - stored in Iw[0] >> 8 for x87
    #[inline]
    pub fn b1(&self) -> u8 {
        (self.modrm_form.operand_data.iw()[0] >> 8) as u8
    }

    /// Set foo (x87 instruction field) - stored in Iw[0]
    #[inline]
    pub fn set_foo(&mut self, foo: u16) {
        self.modrm_form.operand_data.set_iw([foo, 0]);
    }

    /// Get foo (x87 instruction field)
    #[inline]
    pub fn foo(&self) -> u16 {
        self.modrm_form.operand_data.iw()[0]
    }
}
