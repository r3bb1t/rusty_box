// MSR (Model Specific Register) constants and initialization
// Mirrors Bochs cpu/msr.h

use crate::cpu::decoder::features::X86Feature;
use crate::cpu::BxCpuC;

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

/// IA32_USER_MSR_CTL — the URDMSR/UWRMSR permission bitmap's base.
pub const BX_MSR_IA32_USER_MSR_CTL: u32 = 0x01C;

/// An artificial MSR Bochs uses to serialize RDMSRLIST/WRMSRLIST.
pub const BX_MSR_IA32_BARRIER: u32 = 0x02F;

/// IA32_SPEC_CTRL — speculation-control enables (IBRS/STIBP/SSBD).
pub const BX_MSR_IA32_SPEC_CTRL: u32 = 0x048;

/// IA32_PRED_CMD — write-only indirect-branch prediction barrier.
pub const BX_MSR_IA32_PRED_CMD: u32 = 0x049;

/// IA32_ARCH_CAPABILITIES — read-only enumeration of the SCA mitigations
/// this processor does not need.
pub const BX_MSR_IA32_ARCH_CAPABILITIES: u32 = 0x10A;

/// IA32_FLUSH_CMD — write-only L1 data-cache flush command.
pub const BX_MSR_IA32_FLUSH_CMD: u32 = 0x10B;

/// IA32_XSS — the supervisor state components XSAVES may save.
pub const BX_MSR_XSS: u32 = 0xDA0;

/// IA32_APERF (Actual Performance Frequency Clock Count)
pub const BX_MSR_IA32_APERF: u32 = 0x0E7;

/// IA32_MPERF (Maximum Performance Frequency Clock Count)
pub const BX_MSR_IA32_MPERF: u32 = 0x0E8;

/// IA32_UMWAIT_CONTROL (WAITPKG: TPAUSE/UMWAIT max-delay control)
/// Bochs msr.h BX_MSR_IA32_UMWAIT_CONTROL.
pub const BX_MSR_IA32_UMWAIT_CONTROL: u32 = 0x0E1;

/// MTRR Capability register
pub const BX_MSR_MTRRCAP: u32 = 0x0FE;

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
// VMX MSRs — Bochs msr.h
// =========================================================================

/// IA32_FEATURE_CONTROL — VMX enable + LOCK bits.
pub const BX_MSR_IA32_FEATURE_CONTROL: u32 = 0x03A;

pub const BX_MSR_VMX_BASIC: u32 = 0x480;
pub const BX_MSR_VMX_PINBASED_CTRLS: u32 = 0x481;
pub const BX_MSR_VMX_PROCBASED_CTRLS: u32 = 0x482;
pub const BX_MSR_VMX_VMEXIT_CTRLS: u32 = 0x483;
pub const BX_MSR_VMX_VMENTRY_CTRLS: u32 = 0x484;
pub const BX_MSR_VMX_MISC: u32 = 0x485;
pub const BX_MSR_VMX_CR0_FIXED0: u32 = 0x486;
pub const BX_MSR_VMX_CR0_FIXED1: u32 = 0x487;
pub const BX_MSR_VMX_CR4_FIXED0: u32 = 0x488;
pub const BX_MSR_VMX_CR4_FIXED1: u32 = 0x489;
pub const BX_MSR_VMX_VMCS_ENUM: u32 = 0x48A;
pub const BX_MSR_VMX_PROCBASED_CTRLS2: u32 = 0x48B;
pub const BX_MSR_VMX_EPT_VPID_CAP: u32 = 0x48C;
pub const BX_MSR_VMX_TRUE_PINBASED_CTRLS: u32 = 0x48D;
pub const BX_MSR_VMX_TRUE_PROCBASED_CTRLS: u32 = 0x48E;
pub const BX_MSR_VMX_TRUE_VMEXIT_CTRLS: u32 = 0x48F;
pub const BX_MSR_VMX_TRUE_VMENTRY_CTRLS: u32 = 0x490;
pub const BX_MSR_VMX_VMFUNC: u32 = 0x491;
pub const BX_MSR_VMX_PROCBASED_CTRLS3: u32 = 0x492;
pub const BX_MSR_VMX_VMEXIT_CTRLS2: u32 = 0x493;

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

/// Default MTRRCAP value (WC + 8 variable ranges)
pub const BX_MSR_MTRRCAP_DEFAULT: u64 = 0x0508;

/// The highest MSR index the descriptor table covers. Bochs `cpu.h`
/// `BX_MSR_MAX_INDEX`: above it an MSR carries its own gate instead.
pub const BX_MSR_MAX_INDEX: u32 = 0x1000;

/// What an MSR below [`BX_MSR_MAX_INDEX`] is called, and the ISA extension a
/// CPU must have for it to exist at all.
///
/// Bochs msr.cc builds these as a heap array of `MSR_Descriptor*` in
/// `init_MSRs()` and frees them in `destroy_MSRs()`; here the same table is a
/// match the compiler lowers to a jump table, so a CPU has no MSR bring-up
/// step and no per-machine allocation to get wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsrDescriptor {
    /// The architectural name, for the log line a refusal prints.
    pub name: &'static str,
    /// The extension whose absence makes this MSR nonexistent.
    pub feature: X86Feature,
}

impl MsrDescriptor {
    const fn new(name: &'static str, feature: X86Feature) -> Self {
        Self { name, feature }
    }
}

/// The MSR at `index`, if this architecture defines one there.
///
/// Bochs msr.cc `init_MSRs()`, entry for entry. `None` is its `msr_desc[index]
/// == NULL`: an index with no descriptor is not an architectural MSR, which
/// sends it to the unknown-MSR policy rather than to a refusal.
pub const fn msr_descriptor(index: u32) -> Option<MsrDescriptor> {
    use X86Feature as F;
    let descriptor = match index {
        BX_MSR_TSC => MsrDescriptor::new("BX_IA32_TSC", F::IsaPentium),
        BX_MSR_PLATFORM_ID => MsrDescriptor::new("MSR_PLATFORM_ID", F::IsaPentium),
        BX_MSR_APICBASE => MsrDescriptor::new("MSR_APICBASE", F::IsaPentium),
        BX_MSR_IA32_USER_MSR_CTL => MsrDescriptor::new("MSR_IA32_USER_MSR_CTL", F::IsaUserMsr),
        BX_MSR_IA32_APERF => MsrDescriptor::new("MSR_IA32_APERF", F::IsaPentium),
        BX_MSR_IA32_MPERF => MsrDescriptor::new("MSR_IA32_MPERF", F::IsaPentium),

        BX_MSR_SYSENTER_CS => MsrDescriptor::new("MSR_IA32_SYSENTER_CS", F::IsaSysenterSysexit),
        BX_MSR_SYSENTER_ESP => MsrDescriptor::new("MSR_IA32_SYSENTER_ESP", F::IsaSysenterSysexit),
        BX_MSR_SYSENTER_EIP => MsrDescriptor::new("MSR_IA32_SYSENTER_EIP", F::IsaSysenterSysexit),

        BX_MSR_MTRRCAP => MsrDescriptor::new("MSR_IA32_MTRR_CAP", F::IsaMtrr),
        BX_MSR_MTRRPHYSBASE0..=BX_MSR_MTRRPHYSMASK7 => {
            MsrDescriptor::new("MSR_IA32_MTRRPHYS", F::IsaMtrr)
        }
        BX_MSR_MTRRFIX64K_00000 => MsrDescriptor::new("MSR_IA32_MTRRFIX64K_00000", F::IsaMtrr),
        BX_MSR_MTRRFIX16K_80000..=BX_MSR_MTRRFIX16K_A0000 => {
            MsrDescriptor::new("MSR_IA32_MTRRFIX16K", F::IsaMtrr)
        }
        BX_MSR_MTRRFIX4K_C0000..=BX_MSR_MTRRFIX4K_F8000 => {
            MsrDescriptor::new("MSR_IA32_MTRRFIX4K", F::IsaMtrr)
        }
        BX_MSR_MTRR_DEFTYPE => MsrDescriptor::new("MSR_IA32_MTRR_DEFTYPE", F::IsaMtrr),
        BX_MSR_PAT => MsrDescriptor::new("BX_IA32_PAT", F::IsaPat),

        BX_MSR_TSC_ADJUST => MsrDescriptor::new("BX_IA32_TSC_ADJUST", F::IsaTscAdjust),
        BX_MSR_IA32_UMWAIT_CONTROL => {
            MsrDescriptor::new("MSR_IA32_UMWAIT_CONTROL", F::IsaWaitpkg)
        }
        BX_MSR_XSS => MsrDescriptor::new("MSR_IA32_XSS", F::IsaXsaves),

        BX_MSR_IA32_U_CET => MsrDescriptor::new("MSR_IA32_U_CET", F::IsaCet),
        BX_MSR_IA32_S_CET => MsrDescriptor::new("MSR_IA32_S_CET", F::IsaCet),
        BX_MSR_IA32_PL0_SSP..=BX_MSR_IA32_PL3_SSP => {
            MsrDescriptor::new("MSR_IA32_PLx_SSP", F::IsaCet)
        }
        BX_MSR_IA32_INTERRUPT_SSP_TABLE_ADDR => {
            MsrDescriptor::new("MSR_IA32_INTERRUPT_SSP_TABLE_ADDR", F::IsaCet)
        }

        BX_MSR_IA32_UINTR_RR => MsrDescriptor::new("MSR_IA32_UINTR_RR", F::IsaUintr),
        BX_MSR_IA32_UINTR_HANDLER => MsrDescriptor::new("MSR_IA32_UINTR_HANDLER", F::IsaUintr),
        BX_MSR_IA32_UINTR_STACKADJUST => {
            MsrDescriptor::new("MSR_IA32_UINTR_STACKADJUST", F::IsaUintr)
        }
        BX_MSR_IA32_UINTR_MISC => MsrDescriptor::new("MSR_IA32_UINTR_MISC", F::IsaUintr),
        BX_MSR_IA32_UINTR_PD => MsrDescriptor::new("MSR_IA32_UINTR_PD", F::IsaUintr),
        BX_MSR_IA32_UINTR_TT => MsrDescriptor::new("MSR_IA32_UINTR_TT", F::IsaUintr),

        BX_MSR_IA32_PKRS => MsrDescriptor::new("MSR_IA32_PKRS", F::IsaPks),

        BX_MSR_IA32_FRED_RSP0..=BX_MSR_IA32_FRED_RSP3 => {
            MsrDescriptor::new("MSR_IA32_FRED_RSPx", F::IsaFred)
        }
        BX_MSR_IA32_FRED_STKLVLS => MsrDescriptor::new("MSR_IA32_FRED_STKLVLS", F::IsaFred),
        BX_MSR_IA32_FRED_SSP1..=BX_MSR_IA32_FRED_SSP3 => {
            MsrDescriptor::new("MSR_IA32_FRED_SSPx", F::IsaFred)
        }
        BX_MSR_IA32_FRED_CONFIG => MsrDescriptor::new("BX_MSR_IA32_FRED_CONFIG", F::IsaFred),

        BX_MSR_TSC_DEADLINE => MsrDescriptor::new("MSR_TSC_DEADLINE", F::IsaTscDeadline),
        BX_MSR_IA32_BARRIER => MsrDescriptor::new("BX_MSR_IA32_BARRIER", F::IsaMsrlist),

        BX_MSR_IA32_ARCH_CAPABILITIES => {
            MsrDescriptor::new("MSR_IA32_ARCH_CAPABILITIES", F::IsaScaMitigations)
        }
        BX_MSR_IA32_SPEC_CTRL => MsrDescriptor::new("MSR_IA32_SPEC_CTRL", F::IsaScaMitigations),
        BX_MSR_IA32_PRED_CMD => MsrDescriptor::new("MSR_IA32_PRED_CMD", F::IsaScaMitigations),
        BX_MSR_IA32_FLUSH_CMD => MsrDescriptor::new("MSR_IA32_FLUSH_CMD", F::IsaScaMitigations),

        BX_MSR_IA32_FEATURE_CONTROL => {
            MsrDescriptor::new("MSR_IA32_FEATURE_CONTROL", F::IsaVmx)
        }
        // 0x480..=0x493 is the whole VMX capability block, contiguous in
        // Bochs's table and complete: BASIC, the four control pairs, MISC, the
        // CR0/CR4 fixed values, VMCS_ENUM, the secondary and tertiary
        // controls, EPT/VPID capabilities, the TRUE_ variants and VMFUNC.
        BX_MSR_VMX_BASIC..=BX_MSR_VMX_VMEXIT_CTRLS2 => {
            MsrDescriptor::new("MSR_VMX_CAPABILITY", F::IsaVmx)
        }

        // Bochs registers the performance-event selectors under the Pentium
        // feature and then answers them from the unknown-MSR policy.
        BX_MSR_PERFEVTSEL0..=BX_MSR_PERFEVTSEL7 => {
            MsrDescriptor::new("MSR_IA32_PERFEVTSEL", F::IsaPentium)
        }

        _ => return None,
    };
    Some(descriptor)
}

impl<T: crate::cpu::instrumentation::Instrumentation> BxCpuC<T> {
    /// Initialize MSR infrastructure before reset.
    /// Bochs init.cc: zeros configurable MSR array.
    /// Actual MSR default values are set in reset() matching Bochs init.cc.
    pub(super) fn init_msrs(&mut self) {
        // Bochs zeroes the configurable MSR array here (BX_MSR_MAX_INDEX entries).
        // Our MSR struct fields are initialized via Default, so no additional work needed.
        // The configurable MSR path in reset() handles re-zeroing.
    }
}
