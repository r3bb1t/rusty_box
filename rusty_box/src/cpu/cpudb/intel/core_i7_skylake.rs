// cpu/cpudb/intel/corei7_skylake-x.cc

use crate::cpu::cpuid::BxCpuIdTrait;
use crate::cpu::decoder::{features::X86Feature, BX_ISA_EXTENSIONS_ARRAY_SIZE};

use bitflags::bitflags;

// ─── CPUID Leaf 1 ECX feature flags (Bochs cpuid.h:313-344) ────────────────

bitflags! {
    /// CPUID Leaf 1 ECX — Extended Feature Flags
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdStd1Ecx: u32 {
        const SSE3           = 1 <<  0;
        const PCLMULQDQ      = 1 <<  1;
        const DTES64         = 1 <<  2;
        const MONITOR_MWAIT  = 1 <<  3;
        const DS_CPL         = 1 <<  4;
        const VMX            = 1 <<  5;
        const SMX            = 1 <<  6;
        const EST            = 1 <<  7;
        const TM2            = 1 <<  8;
        const SSSE3          = 1 <<  9;
        const CNXT_ID        = 1 << 10;
        // bit 11 reserved
        const FMA            = 1 << 12;
        const CMPXCHG16B     = 1 << 13;
        const XTPR           = 1 << 14;
        const PDCM           = 1 << 15;
        // bit 16 reserved
        const PCID           = 1 << 17;
        const DCA            = 1 << 18;
        const SSE4_1         = 1 << 19;
        const SSE4_2         = 1 << 20;
        const X2APIC         = 1 << 21;
        const MOVBE          = 1 << 22;
        const POPCNT         = 1 << 23;
        const TSC_DEADLINE   = 1 << 24;
        const AES            = 1 << 25;
        const XSAVE          = 1 << 26;
        const OSXSAVE        = 1 << 27; // dynamic — set only when CR4.OSXSAVE=1
        const AVX            = 1 << 28;
        const AVX_F16C       = 1 << 29;
        const RDRAND         = 1 << 30;
        // bit 31 reserved
    }
}

// ─── CPUID Leaf 1 EDX feature flags (Bochs cpuid.h:244-275) ────────────────

bitflags! {
    /// CPUID Leaf 1 EDX — Standard Feature Flags
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdStd1Edx: u32 {
        const X87                = 1 <<  0;
        const VME                = 1 <<  1;
        const DEBUG_EXTENSIONS   = 1 <<  2;
        const PSE                = 1 <<  3;
        const TSC                = 1 <<  4;
        const MSR                = 1 <<  5;
        const PAE                = 1 <<  6;
        const MCE                = 1 <<  7;
        const CMPXCHG8B          = 1 <<  8;
        const APIC               = 1 <<  9; // dynamic — cleared when APIC globally disabled
        // bit 10 reserved
        const SYSENTER_SYSEXIT   = 1 << 11;
        const MTRR               = 1 << 12;
        const GLOBAL_PAGES       = 1 << 13;
        const MCA                = 1 << 14;
        const CMOV               = 1 << 15;
        const PAT                = 1 << 16;
        const PSE36              = 1 << 17;
        const PSN                = 1 << 18;
        const CLFLUSH            = 1 << 19;
        // bit 20 reserved
        const DEBUG_STORE        = 1 << 21;
        const ACPI               = 1 << 22;
        const MMX                = 1 << 23;
        const FXSAVE_FXRSTOR    = 1 << 24;
        const SSE                = 1 << 25;
        const SSE2               = 1 << 26;
        const SELF_SNOOP         = 1 << 27;
        const HT                 = 1 << 28;
        const THERMAL_MONITOR    = 1 << 29;
        // bit 30 reserved
        const PBE                = 1 << 31;
    }
}

// ─── CPUID Leaf 7, Subleaf 0 EBX feature flags (Bochs cpuid.h:382-413) ─────

bitflags! {
    /// CPUID Leaf 7 Subleaf 0 EBX — Structured Extended Feature Flags
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdStd7Ebx: u32 {
        const FSGSBASE           = 1 <<  0;
        const TSC_ADJUST         = 1 <<  1;
        const SGX                = 1 <<  2;
        const BMI1               = 1 <<  3;
        const HLE                = 1 <<  4;
        const AVX2               = 1 <<  5;
        const FDP_DEPRECATION    = 1 <<  6;
        const SMEP               = 1 <<  7;
        const BMI2               = 1 <<  8;
        const ERMS               = 1 <<  9; // Enhanced REP MOVSB/STOSB
        const INVPCID            = 1 << 10;
        const RTM                = 1 << 11;
        const QOS_MONITORING     = 1 << 12;
        const DEPRECATE_FCS_FDS  = 1 << 13;
        const MPX                = 1 << 14;
        const QOS_ENFORCEMENT    = 1 << 15;
        const AVX512F            = 1 << 16;
        const AVX512DQ           = 1 << 17;
        const RDSEED             = 1 << 18;
        const ADX                = 1 << 19;
        const SMAP               = 1 << 20;
        const AVX512IFMA52       = 1 << 21;
        // bit 22 reserved
        const CLFLUSHOPT         = 1 << 23;
        const CLWB               = 1 << 24;
        const PROCESSOR_TRACE    = 1 << 25;
        const AVX512PF           = 1 << 26;
        const AVX512ER           = 1 << 27;
        const AVX512CD           = 1 << 28;
        const SHA                = 1 << 29;
        const AVX512BW           = 1 << 30;
        const AVX512VL           = 1 << 31;
    }
}

// ─── CPUID Extended Leaf 0x80000001 ECX (Bochs cpuid.h:763-794) ────────────

bitflags! {
    /// CPUID Leaf 0x80000001 ECX — Extended Feature Flags
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdExt1Ecx: u32 {
        const LAHF_SAHF          = 1 <<  0;
        const CMP_LEGACY         = 1 <<  1;
        const SVM                = 1 <<  2;
        const EXT_APIC_SPACE     = 1 <<  3;
        const ALT_MOV_CR8        = 1 <<  4;
        const LZCNT              = 1 <<  5;
        const SSE4A              = 1 <<  6;
        const MISALIGNED_SSE     = 1 <<  7;
        const PREFETCHW          = 1 <<  8;
    }
}

// ─── CPUID Extended Leaf 0x80000001 EDX (Bochs cpuid.h:712-725) ────────────

bitflags! {
    /// CPUID Leaf 0x80000001 EDX — Extended Feature Flags
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdExt1Edx: u32 {
        const SYSCALL_SYSRET     = 1 << 11; // dynamic — only set in long mode
        const NX                 = 1 << 20;
        const PAGES_1G           = 1 << 26;
        const RDTSCP             = 1 << 27;
        const LONG_MODE          = 1 << 29;
    }
}

// ─── Helper ────────────────────────────────────────────────────────────────

/// Set a feature bit in the ISA extensions bitmask.
/// Mirrors Bochs bx_cpuid_t::enable_cpu_extension().
fn enable_extension(bitmask: &mut [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE], feature: X86Feature) {
    let idx = feature as usize;
    bitmask[idx / 32] |= 1 << (idx % 32);
}

// ─── Skylake-X CPUID model ────────────────────────────────────────────────

/// Skylake-X (i7-7800X) static CPUID base values.
/// Built from ISA extensions + extra bits, matching Bochs computation exactly.
///
/// Leaf 1 ECX base (no OSXSAVE — that's dynamic):
///   ISA: SSE3|PCLMULQDQ|MON|VMX|SSSE3|FMA|CX16|PCID|
///        SSE4.1|SSE4.2|X2APIC|MOVBE|POPCNT|TSC_DL|AES|XSAVE|AVX|F16C|RDRAND
///   Extra: DTES64|DS_CPL|EST|TM2|xTPR|PDCM
const LEAF1_ECX_BASE: CpuIdStd1Ecx = CpuIdStd1Ecx::SSE3
    .union(CpuIdStd1Ecx::PCLMULQDQ)
    .union(CpuIdStd1Ecx::DTES64)       // extra
    .union(CpuIdStd1Ecx::MONITOR_MWAIT)
    .union(CpuIdStd1Ecx::DS_CPL)       // extra
    .union(CpuIdStd1Ecx::VMX)
    .union(CpuIdStd1Ecx::EST)          // extra
    .union(CpuIdStd1Ecx::TM2)          // extra
    .union(CpuIdStd1Ecx::SSSE3)
    .union(CpuIdStd1Ecx::FMA)
    .union(CpuIdStd1Ecx::CMPXCHG16B)
    .union(CpuIdStd1Ecx::XTPR)         // extra
    .union(CpuIdStd1Ecx::PDCM)         // extra
    .union(CpuIdStd1Ecx::PCID)
    .union(CpuIdStd1Ecx::SSE4_1)
    .union(CpuIdStd1Ecx::SSE4_2)
    .union(CpuIdStd1Ecx::X2APIC)
    .union(CpuIdStd1Ecx::MOVBE)
    .union(CpuIdStd1Ecx::POPCNT)
    .union(CpuIdStd1Ecx::TSC_DEADLINE)
    .union(CpuIdStd1Ecx::AES)
    .union(CpuIdStd1Ecx::XSAVE)
    .union(CpuIdStd1Ecx::AVX)
    .union(CpuIdStd1Ecx::AVX_F16C)
    .union(CpuIdStd1Ecx::RDRAND);

/// Leaf 1 EDX base (APIC bit dynamic — cleared when APIC globally disabled):
///   ISA: X87|VME|DE|PSE|TSC|MSR|PAE|MCE|CX8|APIC|SEP|MTRR|PGE|MCA|CMOV|
///        PAT|PSE36|CLFLUSH|MMX|FXSR|SSE|SSE2
///   Extra: DEBUG_STORE|ACPI|SELF_SNOOP|HT|TM|PBE
const LEAF1_EDX_BASE: CpuIdStd1Edx = CpuIdStd1Edx::X87
    .union(CpuIdStd1Edx::VME)
    .union(CpuIdStd1Edx::DEBUG_EXTENSIONS)
    .union(CpuIdStd1Edx::PSE)
    .union(CpuIdStd1Edx::TSC)
    .union(CpuIdStd1Edx::MSR)
    .union(CpuIdStd1Edx::PAE)
    .union(CpuIdStd1Edx::MCE)
    .union(CpuIdStd1Edx::CMPXCHG8B)
    .union(CpuIdStd1Edx::APIC)
    .union(CpuIdStd1Edx::SYSENTER_SYSEXIT)
    .union(CpuIdStd1Edx::MTRR)
    .union(CpuIdStd1Edx::GLOBAL_PAGES)
    .union(CpuIdStd1Edx::MCA)
    .union(CpuIdStd1Edx::CMOV)
    .union(CpuIdStd1Edx::PAT)
    .union(CpuIdStd1Edx::PSE36)
    .union(CpuIdStd1Edx::CLFLUSH)
    .union(CpuIdStd1Edx::DEBUG_STORE)      // extra
    .union(CpuIdStd1Edx::ACPI)             // extra
    .union(CpuIdStd1Edx::MMX)
    .union(CpuIdStd1Edx::FXSAVE_FXRSTOR)
    .union(CpuIdStd1Edx::SSE)
    .union(CpuIdStd1Edx::SSE2)
    .union(CpuIdStd1Edx::SELF_SNOOP)       // extra
    .union(CpuIdStd1Edx::HT)               // extra
    .union(CpuIdStd1Edx::THERMAL_MONITOR)   // extra
    .union(CpuIdStd1Edx::PBE);              // extra

/// Leaf 7 subleaf 0 EBX:
///   ISA: FSGSBASE|TSC_ADJUST|BMI1|AVX2|FDP_DEPR|SMEP|BMI2|INVPCID|
///        FCS_FDS_DEPR|AVX512F|AVX512DQ|RDSEED|ADX|SMAP|CLFLUSHOPT|
///        CLWB|AVX512CD|AVX512BW|AVX512VL
///   Extra: ERMS (Enhanced REP MOVSB/STOSB)
const LEAF7_EBX_BASE: CpuIdStd7Ebx = CpuIdStd7Ebx::FSGSBASE
    .union(CpuIdStd7Ebx::TSC_ADJUST)
    .union(CpuIdStd7Ebx::BMI1)
    .union(CpuIdStd7Ebx::AVX2)
    .union(CpuIdStd7Ebx::FDP_DEPRECATION)
    .union(CpuIdStd7Ebx::SMEP)
    .union(CpuIdStd7Ebx::BMI2)
    .union(CpuIdStd7Ebx::ERMS)             // extra
    .union(CpuIdStd7Ebx::INVPCID)
    .union(CpuIdStd7Ebx::DEPRECATE_FCS_FDS)
    .union(CpuIdStd7Ebx::AVX512F)
    .union(CpuIdStd7Ebx::AVX512DQ)
    .union(CpuIdStd7Ebx::RDSEED)
    .union(CpuIdStd7Ebx::ADX)
    .union(CpuIdStd7Ebx::SMAP)
    .union(CpuIdStd7Ebx::CLFLUSHOPT)
    .union(CpuIdStd7Ebx::CLWB)
    .union(CpuIdStd7Ebx::AVX512CD)
    .union(CpuIdStd7Ebx::AVX512BW)
    .union(CpuIdStd7Ebx::AVX512VL);

/// Extended leaf 0x80000001 ECX:
///   LAHF_SAHF | LZCNT | PREFETCHW
const EXT1_ECX_BASE: CpuIdExt1Ecx = CpuIdExt1Ecx::LAHF_SAHF
    .union(CpuIdExt1Ecx::LZCNT)
    .union(CpuIdExt1Ecx::PREFETCHW);

/// Extended leaf 0x80000001 EDX (SYSCALL_SYSRET is dynamic — only in long mode):
///   NX | 1G_PAGES | RDTSCP | LONG_MODE
const EXT1_EDX_BASE: CpuIdExt1Edx = CpuIdExt1Edx::NX
    .union(CpuIdExt1Edx::PAGES_1G)
    .union(CpuIdExt1Edx::RDTSCP)
    .union(CpuIdExt1Edx::LONG_MODE);

// ─── Skylake-X struct ─────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Corei7SkylakeX {}

impl BxCpuIdTrait for Corei7SkylakeX {
    fn get_name(&self) -> &'static str {
        "corei7_skylake_x"
    }

    fn get_vmx_extensions_bitmask(&self) -> Option<crate::cpu::cpuid::VMXExtensions> {
        use crate::cpu::cpuid::VMXExtensions;
        Some(
            VMXExtensions::TprShadow
                | VMXExtensions::VirtualNmi
                | VMXExtensions::ApicVirtualization
                | VMXExtensions::WbinvdVmexit
                | VMXExtensions::PerfGlobalCtrl
                | VMXExtensions::MonitorTrapFlag
                | VMXExtensions::X2apicVirtualization
                | VMXExtensions::EPT
                | VMXExtensions::VPID
                | VMXExtensions::UnrestrictedGuest
                | VMXExtensions::PreemptionTimer
                | VMXExtensions::PAT
                | VMXExtensions::EFER
                | VMXExtensions::DescriptorTableExit
                | VMXExtensions::PauseLoopExiting
                | VMXExtensions::EptpSwitching
                | VMXExtensions::EptAccessDirty
                | VMXExtensions::VintrDelivery
                | VMXExtensions::PostedInterrupts
                | VMXExtensions::VmcsShadowing
                | VMXExtensions::EptException,
        )
    }

    fn get_svm_extensions_bitmask(&self) -> Option<crate::cpu::cpuid::SVMExtensions> {
        None
    }

    fn sanity_checks(&self) -> crate::cpu::error::Result<()> {
        Ok(())
    }

    fn new() -> Self {
        Self {}
    }

    /// Returns ISA extensions bitmask for Skylake-X.
    /// Matches Bochs corei7_skylake-x.cc constructor lines 42-109.
    fn get_isa_extensions_bitmask(&self) -> [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE] {
        let mut b = [0u32; BX_ISA_EXTENSIONS_ARRAY_SIZE];
        // Bochs base class: BX_ISA_386 always enabled
        enable_extension(&mut b, X86Feature::Isa386);
        // corei7_skylake-x.cc:42-109
        enable_extension(&mut b, X86Feature::IsaX87);
        enable_extension(&mut b, X86Feature::Isa486);
        enable_extension(&mut b, X86Feature::IsaPentium);
        enable_extension(&mut b, X86Feature::IsaP6);
        enable_extension(&mut b, X86Feature::IsaMmx);
        enable_extension(&mut b, X86Feature::IsaSysenterSysexit);
        enable_extension(&mut b, X86Feature::IsaClflush);
        enable_extension(&mut b, X86Feature::IsaDebugExtensions);
        enable_extension(&mut b, X86Feature::IsaVme);
        enable_extension(&mut b, X86Feature::IsaPse);
        enable_extension(&mut b, X86Feature::IsaPae);
        enable_extension(&mut b, X86Feature::IsaPge);
        enable_extension(&mut b, X86Feature::IsaMtrr);
        enable_extension(&mut b, X86Feature::IsaPat);
        enable_extension(&mut b, X86Feature::IsaXapic);
        enable_extension(&mut b, X86Feature::IsaX2apic);
        enable_extension(&mut b, X86Feature::IsaLongMode);
        enable_extension(&mut b, X86Feature::IsaLmLahfSahf);
        enable_extension(&mut b, X86Feature::IsaCmpxchg16b);
        enable_extension(&mut b, X86Feature::IsaNx);
        enable_extension(&mut b, X86Feature::Isa1gPages);
        enable_extension(&mut b, X86Feature::IsaPcid);
        enable_extension(&mut b, X86Feature::IsaTscAdjust);
        enable_extension(&mut b, X86Feature::IsaTscDeadline);
        enable_extension(&mut b, X86Feature::IsaSse);
        enable_extension(&mut b, X86Feature::IsaSse2);
        enable_extension(&mut b, X86Feature::IsaSse3);
        enable_extension(&mut b, X86Feature::IsaSsse3);
        enable_extension(&mut b, X86Feature::IsaSse4_1);
        enable_extension(&mut b, X86Feature::IsaSse4_2);
        enable_extension(&mut b, X86Feature::IsaPopcnt);
        enable_extension(&mut b, X86Feature::IsaMonitorMwait);
        enable_extension(&mut b, X86Feature::IsaVmx);
        enable_extension(&mut b, X86Feature::IsaRdtscp);
        enable_extension(&mut b, X86Feature::IsaXsave);
        enable_extension(&mut b, X86Feature::IsaXsaveopt);
        enable_extension(&mut b, X86Feature::IsaXsavec);
        enable_extension(&mut b, X86Feature::IsaXsaves);
        enable_extension(&mut b, X86Feature::IsaAesPclmulqdq);
        enable_extension(&mut b, X86Feature::IsaMovbe);
        enable_extension(&mut b, X86Feature::IsaAvx);
        enable_extension(&mut b, X86Feature::IsaAvxF16c);
        enable_extension(&mut b, X86Feature::IsaAvx2);
        enable_extension(&mut b, X86Feature::IsaAvxFma);
        enable_extension(&mut b, X86Feature::IsaLzcnt);
        enable_extension(&mut b, X86Feature::IsaBmi1);
        enable_extension(&mut b, X86Feature::IsaBmi2);
        enable_extension(&mut b, X86Feature::IsaFsgsbase);
        enable_extension(&mut b, X86Feature::IsaInvpcid);
        enable_extension(&mut b, X86Feature::IsaSmep);
        enable_extension(&mut b, X86Feature::IsaRdrand);
        enable_extension(&mut b, X86Feature::IsaFcsFdsDeprecation);
        enable_extension(&mut b, X86Feature::IsaRdseed);
        enable_extension(&mut b, X86Feature::IsaAdx);
        enable_extension(&mut b, X86Feature::IsaSmap);
        enable_extension(&mut b, X86Feature::IsaFdpDeprecation);
        enable_extension(&mut b, X86Feature::IsaAvx512);
        enable_extension(&mut b, X86Feature::IsaAvx512Dq);
        enable_extension(&mut b, X86Feature::IsaAvx512Cd);
        enable_extension(&mut b, X86Feature::IsaAvx512Bw);
        enable_extension(&mut b, X86Feature::IsaClflushopt);
        enable_extension(&mut b, X86Feature::IsaClwb);
        b
    }

    /// CPUID leaf data matching Bochs corei7_skylake-x.cc exactly.
    /// Uses bitflags for feature registers for readability and correctness.
    ///
    /// Dynamic bits (patched in cpuid() handler at soft_int.rs):
    ///   - Leaf 1 ECX[27] OSXSAVE: set only when CR4.OSXSAVE=1
    ///   - Leaf 1 EDX[9] APIC: cleared when APIC globally disabled
    ///   - Leaf 0xD subleaf 0 EAX/EBX/ECX: from xcr0_suppmask / current XCR0
    ///   - Leaf 0xD subleaf 1 EBX: from XCR0 | IA32_XSS
    ///   - Leaf 0x80000001 EDX[11] SYSCALL: only in long mode
    fn get_cpuid_leaf(&self, eax: u32, ecx: u32) -> (u32, u32, u32, u32) {
        match eax {
            // ── Basic CPUID Information ─────────────────────────────────
            0x00000000 => (
                0x00000016, // Max basic leaf = 22
                0x756e6547, // "Genu"
                0x6c65746e, // "ntel"
                0x49656e69, // "ineI" → "GenuineIntel"
            ),

            // ── Leaf 1: Version / Feature Flags ─────────────────────────
            // Bochs corei7_skylake-x.cc:260-365
            0x00000001 => (
                0x00050654, // EAX: Family 6, ExtModel 5, Model 5 → Skylake-X (stepping U0)
                0x00010800, // EBX: Brand=0, CLFLUSH=8(64B), 1 logical proc, APIC ID 0
                LEAF1_ECX_BASE.bits(),
                LEAF1_EDX_BASE.bits(),
            ),

            // ── Leaf 2: Cache/TLB descriptors ───────────────────────────
            // Bochs corei7_skylake-x.cc:368-375
            0x00000002 => (0x76036301, 0x00F0B5FF, 0x00000000, 0x00C30000),

            // ── Leaf 3: Processor Serial Number (not supported) ─────────
            0x00000003 => (0, 0, 0, 0),

            // ── Leaf 4: Deterministic Cache Parameters ──────────────────
            // Bochs corei7_skylake-x.cc:380-439
            0x00000004 => {
                match ecx {
                    0 => (0x1C004121, 0x01C0003F, 0x0000003F, 0x00000000), // L1D 32KB
                    1 => (0x1C004122, 0x01C0003F, 0x0000003F, 0x00000000), // L1I 32KB
                    2 => (0x1C004143, 0x03C0003F, 0x000003FF, 0x00000000), // L2 1MB
                    3 => (0x1C03C163, 0x0280003F, 0x00002FFF, 0x00000004), // L3 8.25MB
                    _ => (0, 0, 0, 0),
                }
            }

            // ── Leaf 5: MONITOR/MWAIT ───────────────────────────────────
            // Bochs corei7_skylake-x.cc:441-443
            0x00000005 => (
                64,         // EAX: smallest monitor-line size
                64,         // EBX: largest monitor-line size
                0x00000003, // ECX: extensions + interrupt break-event
                0x00002020, // EDX: C0/C1 sub-states
            ),

            // ── Leaf 6: Thermal/Power ───────────────────────────────────
            0x00000006 => (0x00000075, 0x00000002, 0x00000009, 0x00000000),

            // ── Leaf 7: Structured Extended Features ────────────────────
            // Bochs corei7_skylake-x.cc:445-493
            0x00000007 => {
                match ecx {
                    0 => (
                        0x00000000,             // EAX: max sub-leaf = 0
                        LEAF7_EBX_BASE.bits(),  // EBX: feature flags
                        0x00000000,             // ECX: no features
                        0x00000000,             // EDX: no features
                    ),
                    _ => (0, 0, 0, 0),
                }
            }

            // ── Leaves 8-9: Reserved ────────────────────────────────────
            0x00000008 | 0x00000009 => (0, 0, 0, 0),

            // ── Leaf A: Performance Monitoring ──────────────────────────
            // Bochs corei7_skylake-x.cc:498-533
            0x0000000A => (0x07300404, 0x00000000, 0x00000000, 0x00000603),

            // ── Leaf B: Extended Topology ───────────────────────────────
            // Bochs corei7_skylake-x.cc:535-554
            0x0000000B => {
                match ecx {
                    0 => (
                        0x00000001, // EAX: bits to shift for SMT
                        0x00000001, // EBX: logical procs at this level
                        0x00000100, // ECX: level=0, type=SMT(1)
                        0x00000000, // EDX: x2APIC ID
                    ),
                    1 => (
                        0x00000000, // EAX: bits to shift
                        0x00000001, // EBX: logical procs at this level
                        0x00000201, // ECX: level=1, type=Core(2)
                        0x00000000, // EDX: x2APIC ID
                    ),
                    _ => (0, 0, 0, 0),
                }
            }

            // ── Leaf C: Reserved ────────────────────────────────────────
            0x0000000C => (0, 0, 0, 0),

            // ── Leaf D: XSAVE state ─────────────────────────────────────
            // Bochs cpuid.cc:206-268 — dynamically patched in cpuid() handler
            0x0000000D => {
                match ecx {
                    0 => (
                        0x000000E7, // EAX: xcr0_suppmask (overridden dynamically)
                        0x00000240, // EBX: size for current xcr0 (overridden dynamically)
                        0x00000A80, // ECX: max size for all features = 2688
                        0x00000000, // EDX: xcr0 upper 32 bits
                    ),
                    1 => (
                        // XSAVEOPT(0) + XSAVEC(1) + XGETBV_ECX1(2) + XSAVES(3)
                        0x0000000F,
                        0x00000000, // EBX: overridden dynamically
                        0x00000000, // ECX: IA32_XSS lower supported bits
                        0x00000000, // EDX: IA32_XSS upper supported bits
                    ),
                    // Per-component sub-leaves: (len, offset, flags, 0)
                    2 => (256, 576, 0, 0),    // YMM state
                    5 => (64, 1088, 0, 0),    // OPMASK
                    6 => (512, 1152, 0, 0),   // ZMM_HI256
                    7 => (1024, 1664, 0, 0),  // HI_ZMM
                    _ => (0, 0, 0, 0),
                }
            }

            // ── Leaves E-14: Reserved ───────────────────────────────────
            0x0000000E..=0x00000014 => (0, 0, 0, 0),

            // ── Leaf 15: TSC/Crystal Clock Ratio ────────────────────────
            // Bochs corei7_skylake-x.cc:195-196
            // EAX=2, EBX=0x124 (292), ECX=0 (crystal freq unknown)
            // TSC_freq = crystal_freq * (EBX/EAX).
            // ECX=0 means kernel cannot compute TSC freq from this leaf.
            0x00000015 => (
                0x00000002, // EAX: denominator
                0x00000124, // EBX: numerator
                0x00000000, // ECX: nominal crystal freq (0 = unknown)
                0x00000000,
            ),

            // ── Leaf 16: Processor Frequency ────────────────────────────
            // Bochs corei7_skylake-x.cc:198-201 (also the default case)
            0x00000016 => (
                0x00000DAC, // EAX: base freq = 3500 MHz
                0x00000FA0, // EBX: max freq = 4000 MHz
                0x00000064, // ECX: bus freq = 100 MHz
                0x00000000,
            ),

            // ── Extended CPUID Leaves ───────────────────────────────────

            0x80000000 => (
                0x80000008, // Max extended leaf
                0x00000000, 0x00000000, 0x00000000,
            ),

            // Leaf 0x80000001: Extended Feature Flags
            // Bochs cpuid.cc:761-889 — SYSCALL patched dynamically
            0x80000001 => (
                0x00000000,
                0x00000000,
                EXT1_ECX_BASE.bits(),
                EXT1_EDX_BASE.bits(),
            ),

            // Leaf 0x80000002-4: Brand string
            // "Intel(R) Core(TM) i7-7800X CPU @ 3.50GHz"
            0x80000002 => (0x65746E49, 0x2952286C, 0x726F4320, 0x4D542865),
            0x80000003 => (0x37692029, 0x3038372D, 0x43205830, 0x40205550),
            0x80000004 => (0x352E3320, 0x7A484730, 0x00000000, 0x00000000),

            // Leaf 0x80000005: reserved for Intel
            0x80000005 => (0, 0, 0, 0),

            // Leaf 0x80000006: L2 Cache
            0x80000006 => (0x00000000, 0x00000000, 0x01006040, 0x00000000),

            // Leaf 0x80000007: Advanced Power Management
            0x80000007 => (0x00000000, 0x00000000, 0x00000000, 0x00000100), // Invariant TSC

            // Leaf 0x80000008: Virtual/Physical Address Sizes
            0x80000008 => (
                0x00003024, // [7:0]=36 phys, [15:8]=48 virt
                0x00000200, // EBX: bit 9 = WBNOINVD
                0x00000000,
                0x00000000,
            ),

            // ── Default: beyond max leaf → return leaf 0x16 data ────────
            // Bochs corei7_skylake-x.cc:199-201
            _ => {
                if eax >= 0x80000000 && eax > 0x80000008 {
                    (0, 0, 0, 0) // beyond max extended leaf
                } else if eax > 0x00000016 && eax < 0x80000000 {
                    // Beyond max standard leaf — Bochs returns leaf 0x16 data
                    (0x00000DAC, 0x00000FA0, 0x00000064, 0x00000000)
                } else {
                    (0, 0, 0, 0)
                }
            }
        }
    }
}
