// MSR (Model Specific Register) constants and initialization
// Mirrors Bochs cpu/msr.h

use crate::cpu::{cpuid::BxCpuIdTrait, BxCpuC};

// =========================================================================
// MSR Register Addresses — matching Bochs msr.h
// =========================================================================

/// IA32_TIME_STAMP_COUNTER (TSC)
pub const BX_MSR_TSC: u32 = 0x010;

/// IA32_APICBASE
pub const BX_MSR_APICBASE: u32 = 0x01B;

/// IA32_TSC_ADJUST
pub const BX_MSR_TSC_ADJUST: u32 = 0x03B;

/// MTRR Capability register
pub const BX_MSR_MTRRCAP: u32 = 0x0FE;

/// SYSENTER_CS
pub const BX_MSR_SYSENTER_CS: u32 = 0x174;

/// SYSENTER_ESP
pub const BX_MSR_SYSENTER_ESP: u32 = 0x175;

/// SYSENTER_EIP
pub const BX_MSR_SYSENTER_EIP: u32 = 0x176;

/// MTRR Physical Base/Mask registers (0x200..0x20F)
pub const BX_MSR_MTRRPHYSBASE0: u32 = 0x200;

/// Last MTRR Physical register
pub const BX_MSR_MTRRPHYSMASK7: u32 = 0x20F;

/// IA32_PAT (Page Attribute Table)
pub const BX_MSR_PAT: u32 = 0x277;

/// Fixed MTRR registers
pub const BX_MSR_MTRRFIX64K_00000: u32 = 0x250;
pub const BX_MSR_MTRRFIX16K_80000: u32 = 0x258;
pub const BX_MSR_MTRRFIX16K_A0000: u32 = 0x259;
pub const BX_MSR_MTRRFIX4K_C0000: u32 = 0x268;
pub const BX_MSR_MTRRFIX4K_F8000: u32 = 0x26F;

/// MTRR Default Type register
pub const BX_MSR_MTRR_DEFTYPE: u32 = 0x2FF;

/// IA32_TSC_DEADLINE
pub const BX_MSR_TSC_DEADLINE: u32 = 0x6E0;

/// Default APICBASE value when APIC support is disabled
pub const BX_MSR_APICBASE_DEFAULT: u64 = 0xFEE00900;

/// Default MTRRCAP value (WC + 8 variable ranges)
pub const BX_MSR_MTRRCAP_DEFAULT: u64 = 0x0508;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn init_msrs(&mut self) {
        // TODO: implement later
    }
}
