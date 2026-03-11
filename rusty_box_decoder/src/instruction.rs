//! Decoded x86 instruction representation.
//!
//! This module contains the core `Instruction` struct produced by the
//! fetch-decode pipeline, along with typed enums for register indices,
//! operand sizes, and addressing modes.
//!
//! # x86 Instruction Encoding
//!
//! An x86 instruction is a variable-length byte sequence (1-15 bytes):
//!
//! ```text
//!  ┌──────────┬──────────┬────────┬─────┬─────────────┬──────────────┐
//!  │ Prefixes │  Opcode  │ ModR/M │ SIB │Displacement │  Immediate   │
//!  │ 0-4 bytes│ 1-3 bytes│ 0-1 B  │0-1 B│ 0/1/2/4 B   │ 0/1/2/4/8 B  │
//!  └──────────┴──────────┴────────┴─────┴─────────────┴──────────────┘
//! ```
//!
//! The decoder reads these bytes and produces a flat `Instruction` struct
//! containing the opcode, register operands, memory addressing components,
//! immediate value, and displacement.

use bitflags::bitflags;

use super::opcode::Opcode;
use super::BxSegregs;

// ============================================================================
// GprIndex — General-purpose register index enum
// ============================================================================

/// General-purpose register index — used for all register operand fields.
///
/// # Register Encoding in x86
///
/// ```text
///  Index │ 64-bit │ 32-bit │ 16-bit │ 8-bit (no REX) │ 8-bit (REX)
///  ──────┼────────┼────────┼────────┼────────────────┼────────────
///    0   │  RAX   │  EAX   │   AX   │      AL        │     AL
///    1   │  RCX   │  ECX   │   CX   │      CL        │     CL
///    2   │  RDX   │  EDX   │   DX   │      DL        │     DL
///    3   │  RBX   │  EBX   │   BX   │      BL        │     BL
///    4   │  RSP   │  ESP   │   SP   │      AH         │     SPL
///    5   │  RBP   │  EBP   │   BP   │      CH         │     BPL
///    6   │  RSI   │  ESI   │   SI   │      DH         │     SIL
///    7   │  RDI   │  EDI   │   DI   │      BH         │     DIL
///   8-15 │ R8-R15 │R8D-R15D│R8W-R15W│      —          │  R8B-R15B
/// ```
///
/// Note: Indices 4-7 map to high-byte registers (AH/CH/DH/BH) in legacy
/// mode, but to low-byte registers (SPL/BPL/SIL/DIL) when a REX prefix
/// is present. This is the infamous "REX byte register remapping."
///
/// Values 0-15 map to x86 GPRs (RAX-R15). Values 16-19 are special.
/// The same index works for 8/16/32/64-bit access — the operand size
/// determines which portion of the register is used.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GprIndex {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
    /// Instruction pointer (RIP/EIP/IP) — used for RIP-relative addressing
    Rip = 16,
    /// Shadow stack pointer
    Ssp = 17,
    /// Temporary register (decoder internal)
    Tmp = 18,
    /// Nil register — reads as 0, writes discarded
    Nil = 19,
}

impl Default for GprIndex {
    fn default() -> Self {
        GprIndex::Rax
    }
}

impl GprIndex {
    /// Number of architectural general-purpose registers (RAX-R15).
    pub const GENERAL_COUNT: usize = 16;

    /// Convert from raw u8 to GprIndex.
    /// Unknown values map to `Nil`.
    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => GprIndex::Rax,
            1 => GprIndex::Rcx,
            2 => GprIndex::Rdx,
            3 => GprIndex::Rbx,
            4 => GprIndex::Rsp,
            5 => GprIndex::Rbp,
            6 => GprIndex::Rsi,
            7 => GprIndex::Rdi,
            8 => GprIndex::R8,
            9 => GprIndex::R9,
            10 => GprIndex::R10,
            11 => GprIndex::R11,
            12 => GprIndex::R12,
            13 => GprIndex::R13,
            14 => GprIndex::R14,
            15 => GprIndex::R15,
            16 => GprIndex::Rip,
            17 => GprIndex::Ssp,
            18 => GprIndex::Tmp,
            19 => GprIndex::Nil,
            _ => GprIndex::Nil,
        }
    }

    /// Convert to usize for array indexing.
    pub const fn as_usize(self) -> usize {
        self as usize
    }
}

// ============================================================================
// InstructionFlags — prefix and mode flags
// ============================================================================

bitflags! {
    /// Prefix and mode flags for a decoded instruction.
    ///
    /// # Bit Layout
    ///
    /// ```text
    ///   7   6   5   4   3   2   1   0
    ///  ┌───┬───┬───┬───┬───┬───┬───┬───┐
    ///  │lock/rep│ext│mod│os │os │as │as │
    ///  │ (2b)   │8b │c0 │64 │32 │64 │32 │
    ///  └───┴───┴───┴───┴───┴───┴───┴───┘
    /// ```
    ///
    /// Bits 6-7 encode a 2-bit value for lock/rep prefixes:
    /// - 0 = none
    /// - 1 = LOCK (0xF0)
    /// - 2 = REPNE (0xF2)
    /// - 3 = REP/REPE (0xF3)
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct InstructionFlags: u8 {
        /// Address size 32-bit
        const As32 = 1 << 0;
        /// Address size 64-bit
        const As64 = 1 << 1;
        /// Operand size 32-bit
        const Os32 = 1 << 2;
        /// Operand size 64-bit
        const Os64 = 1 << 3;
        /// ModRM mod field == 0xC0 (register-to-register, no memory access)
        const ModC0 = 1 << 4;
        /// REX prefix present — enables SPL/BPL/SIL/DIL register mapping
        const Extend8bit = 1 << 5;
    }
}

// ============================================================================
// Operands — named struct replacing meta_data[8]
// ============================================================================

/// Register operands and memory addressing decoded from ModR/M and SIB bytes.
///
/// # ModR/M Byte Layout
///
/// ```text
///   7   6   5   4   3   2   1   0
///  ┌───┬───┬───────────┬───────────┐
///  │ mod   │    reg     │    r/m    │
///  │ (2b)  │   (3b)     │   (3b)    │
///  └───┴───┴───────────┴───────────┘
///
///  mod=11: register-to-register (no memory access)
///  mod=00: [r/m] — register indirect (no displacement)
///  mod=01: [r/m + disp8] — 8-bit signed displacement
///  mod=10: [r/m + disp32] — 32-bit displacement
///
///  When r/m=100 (ESP), a SIB byte follows:
/// ```
///
/// # SIB Byte Layout
///
/// ```text
///   7   6   5   4   3   2   1   0
///  ┌───┬───┬───────────┬───────────┐
///  │ scale │   index    │   base    │
///  │ (2b)  │   (3b)     │   (3b)    │
///  └───┴───┴───────────┴───────────┘
///
///  Effective address = base + (index × 2^scale) + displacement
///  scale: 0=×1, 1=×2, 2=×4, 3=×8
///  index=100 (ESP): no index register (just base + disp)
///  base=101 + mod=00: no base (disp32 + index×scale)
/// ```
#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct Operands {
    /// Destination register index (see `GprIndex`)
    pub dst: u8,
    /// Source register 1 (or opcode extension for Group instructions)
    pub src1: u8,
    /// Source register 2 (VEX.vvv)
    pub src2: u8,
    /// Source register 3 / CET segment override (shared slot)
    pub src3: u8,
    /// Segment register override (see `BxSegregs`)
    pub segment: u8,
    /// Memory base register index (see `GprIndex`; Nil = no base)
    pub base: u8,
    /// Memory index register index (Rsp/4 = no index)
    pub index: u8,
    /// SIB scale factor (0=×1, 1=×2, 2=×4, 3=×8)
    pub scale: u8,
}

// ============================================================================
// Instruction — the flat, decoded instruction struct
// ============================================================================

/// Decoded x86 instruction — the output of the fetch-decode pipeline.
///
/// Contains the opcode, operand registers, memory addressing info,
/// immediate value, and displacement. Produced by [`fetch_decode32()`]
/// or [`fetch_decode64()`].
///
/// # Layout
///
/// ```text
///  ┌────────┬────────┬───────┬──────────┬───────────┬──────────────┐
///  │ opcode │ length │ flags │ operands │ immediate │ displacement │
///  │  (2B)  │  (1B)  │ (1B)  │   (8B)   │   (4B)    │     (4B)     │
///  └────────┴────────┴───────┴──────────┴───────────┴──────────────┘
/// ```
#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct Instruction {
    /// Decoded opcode identifying the instruction
    pub opcode: Opcode,
    /// Instruction byte length (1-15)
    pub length: u8,
    /// Prefix and mode flags (address/operand size, lock/rep, ModC0, extend8bit)
    pub flags: InstructionFlags,
    /// Register operands and memory addressing components
    pub operands: Operands,
    /// Primary immediate value (also stores AVX attributes in upper bytes)
    pub immediate: u32,
    /// Displacement value (also used as second immediate or upper half of 64-bit immediate)
    pub displacement: u32,
}


// ============================================================================
// Instruction accessor methods
// ============================================================================

const BX_LOCK_PREFIX_USED: u8 = 1;

impl Instruction {
    // ============================================================
    // MetaInfo accessors
    // ============================================================

    /// Get instruction length
    #[inline]
    pub const fn ilen(&self) -> u8 {
        self.length
    }

    /// Set instruction length
    #[inline]
    pub fn set_ilen(&mut self, ilen: u8) {
        self.length = ilen;
    }

    /// Get IA-32 opcode
    #[inline]
    pub const fn get_ia_opcode(&self) -> Opcode {
        self.opcode
    }

    /// Set IA-32 opcode
    #[inline]
    pub fn set_ia_opcode(&mut self, op: Opcode) {
        self.opcode = op;
    }

    // ============================================================
    // Operand size flags (os32L, os64L, osize)
    // ============================================================

    /// Initialize operand and address size flags
    #[inline]
    pub fn init(&mut self, os32: u8, as32: u8, os64: u8, as64: u8) {
        let mut flags = InstructionFlags::empty();
        if os32 != 0 {
            flags |= InstructionFlags::Os32;
        }
        if os64 != 0 {
            flags |= InstructionFlags::Os64;
        }
        if as32 != 0 {
            flags |= InstructionFlags::As32;
        }
        if as64 != 0 {
            flags |= InstructionFlags::As64;
        }
        self.flags = flags;
    }

    /// Get os32L (logical value, 0 or non-zero)
    #[inline]
    pub const fn os32_l(&self) -> u8 {
        self.flags.bits() & InstructionFlags::Os32.bits()
    }

    /// Set os32B (boolean)
    #[inline]
    pub fn set_os32_b(&mut self, bit: bool) {
        self.flags.set(InstructionFlags::Os32, bit);
    }

    /// Assert os32
    #[inline]
    pub fn assert_os32(&mut self) {
        self.flags |= InstructionFlags::Os32;
    }

    /// Get os64L (logical value, 0 or non-zero)
    #[inline]
    pub const fn os64_l(&self) -> u8 {
        self.flags.bits() & InstructionFlags::Os64.bits()
    }

    /// Assert os64
    #[inline]
    pub fn assert_os64(&mut self) {
        self.flags |= InstructionFlags::Os64;
    }

    /// Get operand size (0=16-bit, 1=32-bit, 2=64-bit)
    #[inline]
    pub const fn osize(&self) -> u8 {
        let bits = self.flags.bits();
        (bits >> 2) & 0x3
    }

    // ============================================================
    // Address size flags (as32L, as64L, asize)
    // ============================================================

    /// Get as32L (logical value, 0 or non-zero)
    #[inline]
    pub const fn as32_l(&self) -> u8 {
        self.flags.bits() & InstructionFlags::As32.bits()
    }

    /// Set as32B (boolean)
    #[inline]
    pub fn set_as32_b(&mut self, bit: bool) {
        self.flags.set(InstructionFlags::As32, bit);
    }

    /// Get as64L (logical value, 0 or non-zero)
    #[inline]
    pub const fn as64_l(&self) -> u8 {
        self.flags.bits() & InstructionFlags::As64.bits()
    }

    /// Clear as64
    #[inline]
    pub fn clear_as64(&mut self) {
        self.flags.remove(InstructionFlags::As64);
    }

    /// Get address size (0=16-bit, 1=32-bit, 2=64-bit)
    #[inline]
    pub const fn asize(&self) -> u8 {
        self.flags.bits() & 0x3
    }

    /// Get extend8bitL (for 64-bit mode)
    #[inline]
    pub const fn extend8bit_l(&self) -> u8 {
        self.flags.bits() & InstructionFlags::Extend8bit.bits()
    }

    /// Assert extend8bit
    #[inline]
    pub fn assert_extend8bit(&mut self) {
        self.flags |= InstructionFlags::Extend8bit;
    }

    // ============================================================
    // Lock/Rep prefix flags
    // ============================================================

    /// Get repUsedL (0=none, 1=0xF0, 2=0xF2, 3=0xF3)
    #[inline]
    pub const fn rep_used_l(&self) -> u8 {
        self.flags.bits() >> 7
    }

    /// Get lockRepUsedValue (0=none, 1=0xF0, 2=0xF2, 3=0xF3)
    #[inline]
    pub const fn lock_rep_used_value(&self) -> u8 {
        (self.flags.bits() >> 6) & 0x3
    }

    /// Set lockRepUsed
    #[inline]
    pub fn set_lock_rep_used(&mut self, value: u8) {
        let bits = self.flags.bits();
        let new_bits = (bits & 0x3f) | (value << 6);
        self.flags = InstructionFlags::from_bits_truncate(new_bits);
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

    /// Get modC0 (mod==0xC0 in ModRM — register form, no memory access)
    #[inline]
    pub const fn mod_c0(&self) -> bool {
        (self.flags.bits() & InstructionFlags::ModC0.bits()) != 0
    }

    /// Assert modC0
    #[inline]
    pub fn assert_mod_c0(&mut self) {
        self.flags |= InstructionFlags::ModC0;
    }

    // ============================================================
    // Register operand accessors
    // ============================================================

    /// Get destination register index
    #[inline]
    pub const fn dst(&self) -> u8 {
        self.operands.dst
    }

    /// Get source register 1
    #[inline]
    pub const fn src1(&self) -> u8 {
        self.operands.src1
    }

    /// Get source register 2
    #[inline]
    pub const fn src2(&self) -> u8 {
        self.operands.src2
    }

    /// Get source register 3
    #[inline]
    pub const fn src3(&self) -> u8 {
        self.operands.src3
    }

    /// Get source register (alias for src1)
    #[inline]
    pub const fn src(&self) -> u8 {
        self.src1()
    }

    /// Set source register by index (0=dst, 1=src1, 2=src2, 3=src3)
    #[inline]
    pub fn set_src_reg(&mut self, src: usize, reg: u8) {
        match src {
            0 => self.operands.dst = reg,
            1 => self.operands.src1 = reg,
            2 => self.operands.src2 = reg,
            3 => self.operands.src3 = reg,
            _ => {}
        }
    }

    /// Get source register by index
    #[inline]
    pub const fn get_src_reg(&self, src: usize) -> u8 {
        match src {
            0 => self.operands.dst,
            1 => self.operands.src1,
            2 => self.operands.src2,
            3 => self.operands.src3,
            _ => 0,
        }
    }

    // ============================================================
    // Segment register accessors
    // ============================================================

    /// Get segment register
    #[inline]
    pub const fn seg(&self) -> u8 {
        self.operands.segment
    }

    /// Set segment register
    #[inline]
    pub fn set_seg(&mut self, val: BxSegregs) {
        self.operands.segment = val as u8;
    }

    /// Get CET segment override (shares slot with src3)
    #[inline]
    pub const fn seg_override_cet(&self) -> u8 {
        self.operands.src3
    }

    /// Set CET segment override (shares slot with src3)
    #[inline]
    pub fn set_cet_seg_override(&mut self, val: BxSegregs) {
        self.operands.src3 = val as u8;
    }

    // ============================================================
    // SIB (Scale-Index-Base) accessors
    // ============================================================

    /// Set SIB scale
    #[inline]
    pub fn set_sib_scale(&mut self, scale: u8) {
        self.operands.scale = scale;
    }

    /// Get SIB scale
    #[inline]
    pub const fn sib_scale(&self) -> u8 {
        self.operands.scale
    }

    /// Set SIB index
    #[inline]
    pub fn set_sib_index(&mut self, index: u8) {
        self.operands.index = index;
    }

    /// Get SIB index
    #[inline]
    pub const fn sib_index(&self) -> u8 {
        self.operands.index
    }

    /// Set SIB base
    #[inline]
    pub fn set_sib_base(&mut self, base: u8) {
        self.operands.base = base;
    }

    /// Get SIB base
    #[inline]
    pub const fn sib_base(&self) -> u8 {
        self.operands.base
    }

    // ============================================================
    // Immediate value accessors
    // ============================================================

    /// Get 32-bit immediate (Id)
    #[inline]
    pub const fn id(&self) -> u32 {
        self.immediate
    }

    /// Get 16-bit immediate (Iw)
    #[inline]
    pub const fn iw(&self) -> u16 {
        self.immediate as u16
    }

    /// Get 8-bit immediate (Ib)
    #[inline]
    pub const fn ib(&self) -> u8 {
        self.immediate as u8
    }

    /// Get 64-bit immediate (Iq) — x86-64 only.
    ///
    /// Reconstructed from the immediate (low 32 bits) and displacement
    /// (high 32 bits) fields, which share space with the 64-bit immediate
    /// in the original Bochs `IqForm` union.
    #[inline]
    pub const fn iq(&self) -> u64 {
        let lo = self.immediate as u64;
        let hi = self.displacement as u64;
        lo | (hi << 32)
    }

    /// Set 64-bit immediate (Iq) — x86-64 only
    #[inline]
    pub fn set_iq(&mut self, val: u64) {
        self.immediate = val as u32;
        self.displacement = (val >> 32) as u32;
    }

    /// Get second 32-bit immediate (Id2)
    #[inline]
    pub const fn id2(&self) -> u32 {
        self.displacement
    }

    /// Get second 16-bit immediate (Iw2)
    #[inline]
    pub const fn iw2(&self) -> u16 {
        self.displacement as u16
    }

    /// Get second 8-bit immediate (Ib2)
    #[inline]
    pub const fn ib2(&self) -> u8 {
        self.displacement as u8
    }

    // ============================================================
    // Displacement accessors
    // ============================================================

    /// Get 32-bit signed displacement
    #[inline]
    pub const fn displ32s(&self) -> i32 {
        self.displacement as i32
    }

    /// Get 16-bit signed displacement
    #[inline]
    pub const fn displ16s(&self) -> i16 {
        self.displacement as i16
    }

    /// Get 32-bit unsigned displacement
    #[inline]
    pub const fn displ32u(&self) -> u32 {
        self.displacement
    }

    /// Get 16-bit unsigned displacement
    #[inline]
    pub const fn displ16u(&self) -> u16 {
        self.displacement as u16
    }

    // ============================================================
    // AVX/EVEX attribute accessors
    //
    // These read/write bytes within the `immediate` field (bytes 1-3)
    // which store AVX metadata alongside the 8-bit immediate (byte 0).
    // ============================================================

    /// Get the immediate as native-endian byte array
    #[inline]
    const fn imm_bytes(&self) -> [u8; 4] {
        self.immediate.to_ne_bytes()
    }

    /// Set the immediate from native-endian byte array
    #[inline]
    const fn set_imm_bytes(&mut self, bytes: [u8; 4]) {
        self.immediate = u32::from_ne_bytes(bytes);
    }

    /// Get AVX vector length (VL)
    #[inline]
    pub const fn get_vl(&self) -> u8 {
        self.imm_bytes()[1]
    }

    /// Set AVX vector length (VL)
    #[inline]
    pub const fn set_vl(&mut self, value: u8) {
        let mut ib = self.imm_bytes();
        ib[1] = value;
        self.set_imm_bytes(ib);
    }

    /// Get VEX.W bit
    #[inline]
    pub const fn get_vex_w(&self) -> u8 {
        (self.imm_bytes()[2] >> 4) & 1
    }

    /// Set VEX.W bit
    #[inline]
    pub const fn set_vex_w(&mut self, bit: u8) {
        let mut ib = self.imm_bytes();
        ib[2] = (ib[2] & !0x10) | ((bit & 1) << 4);
        self.set_imm_bytes(ib);
    }

    /// Get EVEX opmask register
    #[inline]
    pub const fn opmask(&self) -> u8 {
        self.imm_bytes()[3]
    }

    /// Set EVEX opmask register
    #[inline]
    pub const fn set_opmask(&mut self, reg: u8) {
        let mut ib = self.imm_bytes();
        ib[3] = reg;
        self.set_imm_bytes(ib);
    }

    /// Get EVEX.b bit (broadcast/RC/SAE control)
    #[inline]
    pub const fn get_evex_b(&self) -> u8 {
        (self.imm_bytes()[2] >> 3) & 1
    }

    /// Set EVEX.b bit
    #[inline]
    pub const fn set_evex_b(&mut self, bit: u8) {
        let mut ib = self.imm_bytes();
        ib[2] = (ib[2] & !0x8) | ((bit & 1) << 3);
        self.set_imm_bytes(ib);
    }

    /// Get zero masking bit (EVEX.z)
    #[inline]
    pub const fn is_zero_masking(&self) -> u8 {
        (self.imm_bytes()[2] >> 2) & 1
    }

    /// Set zero masking bit
    #[inline]
    pub const fn set_zero_masking(&mut self, bit: u8) {
        let mut ib = self.imm_bytes();
        ib[2] = (ib[2] & !0x4) | ((bit & 1) << 2);
        self.set_imm_bytes(ib);
    }

    /// Get rounding control (RC)
    #[inline]
    pub const fn get_rc(&self) -> u8 {
        self.imm_bytes()[2] & 0x3
    }

    /// Set rounding control (RC)
    #[inline]
    pub const fn set_rc(&mut self, rc: u8) {
        let mut ib = self.imm_bytes();
        ib[2] = (ib[2] & !0x3) | (rc & 0x3);
        self.set_imm_bytes(ib);
    }

    // ============================================================
    // Helper methods for instruction handlers
    // ============================================================

    /// Get first byte of opcode (b1) — stored in bits 15:8 of immediate for x87
    #[inline]
    pub fn b1(&self) -> u8 {
        (self.immediate >> 8) as u8
    }

    /// Set foo (x87 instruction field) — stored in low 16 bits of immediate
    #[inline]
    pub fn set_foo(&mut self, foo: u16) {
        // Preserve upper 16 bits, set lower 16 bits
        self.immediate = (self.immediate & 0xFFFF_0000) | (foo as u32);
    }

    /// Get foo (x87 instruction field)
    #[inline]
    pub fn foo(&self) -> u16 {
        self.immediate as u16
    }

    // ============================================================
    // Typed getter methods — return enums instead of raw integers
    // ============================================================

    /// Get the operand size as a typed enum.
    pub const fn operand_size(&self) -> OperandSize {
        match self.osize() {
            0 => OperandSize::Size16,
            1 => OperandSize::Size32,
            _ => OperandSize::Size64,
        }
    }

    /// Get the address size as a typed enum.
    pub const fn address_size(&self) -> AddressSize {
        match self.asize() {
            0 => AddressSize::Size16,
            1 => AddressSize::Size32,
            _ => AddressSize::Size64,
        }
    }

    /// Get the lock/rep prefix as a typed enum.
    pub const fn rep_prefix(&self) -> RepPrefix {
        match self.lock_rep_used_value() {
            1 => RepPrefix::Lock,
            2 => RepPrefix::RepNE,
            3 => RepPrefix::RepE,
            _ => RepPrefix::None,
        }
    }

    /// Get the destination register as a typed `GprIndex`.
    pub const fn dst_reg(&self) -> GprIndex {
        GprIndex::from_u8(self.operands.dst)
    }

    /// Get source register 1 as a typed `GprIndex`.
    pub const fn src1_reg(&self) -> GprIndex {
        GprIndex::from_u8(self.operands.src1)
    }

    /// Get the memory base register as a typed `GprIndex`.
    pub const fn base_reg(&self) -> GprIndex {
        GprIndex::from_u8(self.operands.base)
    }

    /// Get the memory index register as a typed `GprIndex`.
    pub const fn index_reg(&self) -> GprIndex {
        GprIndex::from_u8(self.operands.index)
    }

    /// Get the segment register as a typed `BxSegregs`.
    pub const fn segment_reg(&self) -> BxSegregs {
        BxSegregs::from_u8(self.operands.segment)
    }
}

// ============================================================================
// Typed getter enums
// ============================================================================

/// Operand size for this instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperandSize {
    Size16,
    Size32,
    Size64,
}

/// Address size for this instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressSize {
    Size16,
    Size32,
    Size64,
}

/// Lock/Rep prefix state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepPrefix {
    None,
    Lock,
    RepNE,
    RepE,
}
