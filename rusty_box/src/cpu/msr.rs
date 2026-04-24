#![allow(dead_code)]
// MSR (Model Specific Register) constants and initialization
// Mirrors Bochs cpu/msr.h

use crate::cpu::{cpuid::BxCpuIdTrait, BxCpuC};

// =========================================================================
// MSR Register Addresses — matching Bochs msr.h
// =========================================================================

/// IA32_TIME_STAMP_COUNTER (TSC)
pub const BX_MSR_TSC: u32 = 0x010;

/// IA32_PLATFORM_ID
pub const BX_MSR_PLATFORM_ID: u32 = 0x017;

/// IA32_APICBASE
pub const BX_MSR_APICBASE: u32 = 0x01B;

/// IA32_TSC_ADJUST
pub const BX_MSR_TSC_ADJUST: u32 = 0x03B;

/// IA32_BIOS_SIGN_ID (microcode revision)
pub const BX_MSR_BIOS_SIGN_ID: u32 = 0x08B;

/// IA32_APERF (Actual Performance Frequency Clock Count)
pub const BX_MSR_IA32_APERF: u32 = 0x0E7;

/// IA32_MPERF (Maximum Performance Frequency Clock Count)
pub const BX_MSR_IA32_MPERF: u32 = 0x0E8;

/// IA32_UMWAIT_CONTROL (WAITPKG: TPAUSE/UMWAIT max-delay control)
/// Bochs msr.h BX_MSR_IA32_UMWAIT_CONTROL.
pub const BX_MSR_IA32_UMWAIT_CONTROL: u32 = 0x0E1;

/// MTRR Capability register
pub const BX_MSR_MTRRCAP: u32 = 0x0FE;

/// IA32_PMC0..7 (Performance Monitoring Counters)
pub const BX_MSR_PMC0: u32 = 0x0C1;
pub const BX_MSR_PMC7: u32 = 0x0C8;

/// IA32_PERFEVTSEL0..7 (Performance Event Select)
pub const BX_MSR_PERFEVTSEL0: u32 = 0x186;
pub const BX_MSR_PERFEVTSEL7: u32 = 0x18D;

/// IA32_FRED_RSP0..RSP3 (FRED Return Stack Pointers)
pub const BX_MSR_IA32_FRED_RSP0: u32 = 0x1CC;
pub const BX_MSR_IA32_FRED_RSP3: u32 = 0x1CF;

/// IA32_FRED_STKLVLS (FRED Stack Levels)
pub const BX_MSR_IA32_FRED_STKLVLS: u32 = 0x1D0;

/// IA32_FRED_SSP1..SSP3 (FRED Shadow Stack Pointers)
pub const BX_MSR_IA32_FRED_SSP1: u32 = 0x1D1;
pub const BX_MSR_IA32_FRED_SSP3: u32 = 0x1D3;

/// IA32_FRED_CONFIG
pub const BX_MSR_IA32_FRED_CONFIG: u32 = 0x1D4;

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

// =========================================================================
// CET MSRs — Bochs msr.h
// =========================================================================

/// IA32_U_CET — user-mode CET control (shadow stack + ENDBRANCH).
pub const BX_MSR_IA32_U_CET: u32 = 0x6A0;
/// IA32_S_CET — supervisor-mode CET control.
pub const BX_MSR_IA32_S_CET: u32 = 0x6A2;
/// IA32_PL0_SSP — Privilege Level 0 Shadow Stack Pointer.
pub const BX_MSR_IA32_PL0_SSP: u32 = 0x6A4;
/// IA32_PL3_SSP — Privilege Level 3 Shadow Stack Pointer (last in PLn_SSP block).
pub const BX_MSR_IA32_PL3_SSP: u32 = 0x6A7;
/// IA32_INTERRUPT_SSP_TABLE_ADDR — interrupt-SSP-table base address.
pub const BX_MSR_IA32_INTERRUPT_SSP_TABLE_ADDR: u32 = 0x6A8;

// =========================================================================
// User Interrupts (UINTR) MSRs — Bochs msr.h
// =========================================================================

/// IA32_UINTR_RR — user-level interrupt request register.
pub const BX_MSR_IA32_UINTR_RR: u32 = 0x985;
/// IA32_UINTR_HANDLER — user-level interrupt handler address (canonical).
pub const BX_MSR_IA32_UINTR_HANDLER: u32 = 0x986;
/// IA32_UINTR_STACKADJUST — user-level stack adjustment.
pub const BX_MSR_IA32_UINTR_STACKADJUST: u32 = 0x987;
/// IA32_UINTR_MISC — low 32 = UITT_SIZE, high 32 = UINV (notification vector).
pub const BX_MSR_IA32_UINTR_MISC: u32 = 0x988;
/// IA32_UINTR_PD — user-level posted-interrupt descriptor address.
pub const BX_MSR_IA32_UINTR_PD: u32 = 0x989;
/// IA32_UINTR_TT — user-level interrupt target table address.
pub const BX_MSR_IA32_UINTR_TT: u32 = 0x98A;

/// IA32_PKRS — Supervisor Protection Key Rights (PKS). Bochs msr.h.
pub const BX_MSR_IA32_PKRS: u32 = 0x6E1;

// =========================================================================
// Long-mode MSRs (AMD64/Intel EM64T) — Bochs msr.h
// =========================================================================

/// EFER (Extended Feature Enable Register)
pub const BX_MSR_EFER: u32 = 0xC000_0080;

/// STAR — SYSCALL/SYSRET target CS/SS and EIP (32-bit mode)
pub const BX_MSR_STAR: u32 = 0xC000_0081;

/// LSTAR — SYSCALL target RIP (64-bit mode)
pub const BX_MSR_LSTAR: u32 = 0xC000_0082;

/// CSTAR — SYSCALL target RIP (compatibility mode)
pub const BX_MSR_CSTAR: u32 = 0xC000_0083;

/// FMASK — SYSCALL RFLAGS mask
pub const BX_MSR_FMASK: u32 = 0xC000_0084;

/// FS.base — 64-bit FS segment base address
pub const BX_MSR_FSBASE: u32 = 0xC000_0100;

/// GS.base — 64-bit GS segment base address
pub const BX_MSR_GSBASE: u32 = 0xC000_0101;

/// KernelGSbase — used by SWAPGS instruction
pub const BX_MSR_KERNELGSBASE: u32 = 0xC000_0102;

/// TSC_AUX — auxiliary TSC value (returned by RDTSCP in ECX)
pub const BX_MSR_TSC_AUX: u32 = 0xC000_0103;

/// Default APICBASE value when APIC support is disabled
pub const BX_MSR_APICBASE_DEFAULT: u64 = 0xFEE00900;

/// Default MTRRCAP value (WC + 8 variable ranges)
pub const BX_MSR_MTRRCAP_DEFAULT: u64 = 0x0508;

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Initialize MSR infrastructure before reset.
    /// Bochs init.cc: zeros configurable MSR array.
    /// Actual MSR default values are set in reset() matching Bochs init.cc.
    pub(super) fn init_msrs(&mut self) {
        // Bochs zeroes the configurable MSR array here (BX_MSR_MAX_INDEX entries).
        // Our MSR struct fields are initialized via Default, so no additional work needed.
        // The configurable MSR path in reset() handles re-zeroing.
    }
}
