//! x86 EFLAGS register type using bitflags.
//!
//! Mirrors the x86 EFLAGS bit layout:
//!
//! ```text
//! 31|30|29|28| 27|26|25|24| 23|22|21|20| 19|18|17|16
//! ==|==|=====| ==|==|==|==| ==|==|==|==| ==|==|==|==
//!  0| 0| 0| 0|  0| 0| 0| 0|  0| 0|ID|VP| VF|AC|VM|RF
//!
//! 15|14|13|12| 11|10| 9| 8|  7| 6| 5| 4|  3| 2| 1| 0
//! ==|==|=====| ==|==|==|==| ==|==|==|==| ==|==|==|==
//!  0|NT| IOPL| OF|DF|IF|TF| SF|ZF| 0|AF|  0|PF| 1|CF
//! ```

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct EFlags: u32 {
        const CF    = 1 << 0;   // Carry Flag
        const R1    = 1 << 1;   // Reserved (always 1)
        const PF    = 1 << 2;   // Parity Flag
        const AF    = 1 << 4;   // Auxiliary Carry Flag
        const ZF    = 1 << 6;   // Zero Flag
        const SF    = 1 << 7;   // Sign Flag
        const TF    = 1 << 8;   // Trap Flag
        const IF_   = 1 << 9;   // Interrupt Enable Flag (IF is keyword)
        const DF    = 1 << 10;  // Direction Flag
        const OF    = 1 << 11;  // Overflow Flag
        const IOPL1 = 1 << 12;  // I/O Privilege Level bit 0
        const IOPL2 = 1 << 13;  // I/O Privilege Level bit 1
        const NT    = 1 << 14;  // Nested Task
        const RF    = 1 << 16;  // Resume Flag
        const VM    = 1 << 17;  // Virtual 8086 Mode
        const AC    = 1 << 18;  // Alignment Check
        const VIF   = 1 << 19;  // Virtual Interrupt Flag
        const VIP   = 1 << 20;  // Virtual Interrupt Pending
        const ID    = 1 << 21;  // ID Flag
    }
}

impl EFlags {
    /// Both IOPL bits combined (mask = 0x3000)
    pub const IOPL_MASK: EFlags = Self::IOPL1.union(Self::IOPL2);

    /// Common flag group: OF, SF, ZF, AF, PF, CF
    pub const OSZAPC: EFlags = Self::OF
        .union(Self::SF)
        .union(Self::ZF)
        .union(Self::AF)
        .union(Self::PF)
        .union(Self::CF);

    /// Logic operation flags: OF=0, SF, ZF, PF, CF=0 (AF undefined)
    pub const LOGIC_MASK: EFlags = Self::OF
        .union(Self::SF)
        .union(Self::ZF)
        .union(Self::PF)
        .union(Self::CF);

    /// Supported EFLAGS bits mask (Bochs EFlagsSupportMask)
    /// All bits that the CPU implements: CF|PF|AF|ZF|SF|TF|IF|DF|OF|IOPL|NT|RF|VM|AC|VIF|VIP|ID
    pub const SUPPORT_MASK: EFlags = Self::CF
        .union(Self::PF)
        .union(Self::AF)
        .union(Self::ZF)
        .union(Self::SF)
        .union(Self::TF)
        .union(Self::IF_)
        .union(Self::DF)
        .union(Self::OF)
        .union(Self::IOPL1)
        .union(Self::IOPL2)
        .union(Self::NT)
        .union(Self::RF)
        .union(Self::VM)
        .union(Self::AC)
        .union(Self::VIF)
        .union(Self::VIP)
        .union(Self::ID);

    /// Valid EFLAGS mask (Bochs EFlagsValidMask) — same as SUPPORT_MASK for P6+
    pub const VALID_MASK: EFlags = Self::SUPPORT_MASK;

    /// IRET32 real-mode changeMask: VIF, VIP, VM unchanged (Bochs 0x00257fd5)
    pub const IRET32_REAL_CHANGE: EFlags = Self::CF
        .union(Self::PF)
        .union(Self::AF)
        .union(Self::ZF)
        .union(Self::SF)
        .union(Self::TF)
        .union(Self::IF_)
        .union(Self::DF)
        .union(Self::OF)
        .union(Self::IOPL1)
        .union(Self::IOPL2)
        .union(Self::NT)
        .union(Self::RF)
        .union(Self::AC)
        .union(Self::ID);

    /// Arithmetic flags: CF|PF|AF|ZF|SF|OF = 0x08D5
    pub const ARITH_FLAGS: EFlags = Self::OSZAPC;

    /// Get the IOPL value (0-3)
    #[inline]
    pub const fn iopl(self) -> u8 {
        ((self.bits() >> 12) & 3) as u8
    }

    /// Set the IOPL value (0-3), preserving other bits
    #[inline]
    pub fn set_iopl(&mut self, level: u8) {
        let raw = (self.bits() & !Self::IOPL_MASK.bits()) | (((level & 3) as u32) << 12);
        *self = Self::from_bits_retain(raw);
    }
}
