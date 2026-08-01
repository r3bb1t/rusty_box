// cpu/cpudb/intel/corei7_skylake-x.cc

use crate::cpu::cpuid::{BxCpuIdTrait, CpuidFreq, CpuidLeaf};
use crate::cpu::decoder::{features::X86Feature, BX_ISA_EXTENSIONS_ARRAY_SIZE};

use bitflags::bitflags;

/// When RUSTY_BOX_NO_AVX is set, strip AVX/FMA/AVX2/BMI1/BMI2/AVX-512 from
/// CPUID and ISA extensions. Forces kernel to SSE2-only code paths for
/// diagnosing instruction emulation bugs.
#[cfg(feature = "std")]
fn no_avx_mode() -> bool {
    static NO_AVX: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NO_AVX.get_or_init(|| {
        let active = std::env::var("RUSTY_BOX_NO_AVX").is_ok();
        if active {
            tracing::debug!("[CPUID] RUSTY_BOX_NO_AVX: stripping AVX/FMA/AVX2/BMI1/BMI2/AVX-512");
        }
        active
    })
}

#[cfg(not(feature = "std"))]
fn no_avx_mode() -> bool {
    false
}

// ─── CPUID Leaf 1 ECX feature flags (Bochs cpuid.h) ────────────────

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

// ─── CPUID Leaf 1 EDX feature flags (Bochs cpuid.h) ────────────────

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

// ─── CPUID Leaf 7, Subleaf 0 EBX feature flags (Bochs cpuid.h) ─────

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

// ─── CPUID Extended Leaf 0x80000001 ECX (Bochs cpuid.h) ────────────

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

// ─── CPUID Extended Leaf 0x80000001 EDX (Bochs cpuid.h) ────────────

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
    .union(CpuIdStd1Ecx::DTES64) // extra
    .union(CpuIdStd1Ecx::MONITOR_MWAIT)
    .union(CpuIdStd1Ecx::DS_CPL) // extra
    .union(CpuIdStd1Ecx::VMX) // VMX MSRs + #UD on VMXON (stubs)
    .union(CpuIdStd1Ecx::EST) // extra
    .union(CpuIdStd1Ecx::TM2) // extra
    .union(CpuIdStd1Ecx::SSSE3)
    .union(CpuIdStd1Ecx::FMA)
    .union(CpuIdStd1Ecx::CMPXCHG16B)
    .union(CpuIdStd1Ecx::XTPR) // extra
    .union(CpuIdStd1Ecx::PDCM) // extra
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
    .union(CpuIdStd1Edx::DEBUG_STORE) // extra
    .union(CpuIdStd1Edx::ACPI) // extra
    .union(CpuIdStd1Edx::MMX)
    .union(CpuIdStd1Edx::FXSAVE_FXRSTOR)
    .union(CpuIdStd1Edx::SSE)
    .union(CpuIdStd1Edx::SSE2)
    .union(CpuIdStd1Edx::SELF_SNOOP) // extra
    .union(CpuIdStd1Edx::HT) // extra
    .union(CpuIdStd1Edx::THERMAL_MONITOR) // extra
    .union(CpuIdStd1Edx::PBE); // extra

/// Leaf 7 subleaf 0 EBX:
///   ISA: FSGSBASE|TSC_ADJUST|BMI1|FDP_DEPR|SMEP|BMI2|INVPCID|
///        FCS_FDS_DEPR|RDSEED|ADX|SMAP|CLFLUSHOPT|
///        CLWB
///   Extra: ERMS (Enhanced REP MOVSB/STOSB)
/// NOTE: AVX-512 disabled — not all 512-bit handlers implemented.
/// AVX2 re-enabled for instruction-level tracing of SHA-1 hash bug.
const LEAF7_EBX_BASE: CpuIdStd7Ebx = CpuIdStd7Ebx::FSGSBASE
    .union(CpuIdStd7Ebx::TSC_ADJUST)
    .union(CpuIdStd7Ebx::BMI1)
    .union(CpuIdStd7Ebx::AVX2)
    .union(CpuIdStd7Ebx::FDP_DEPRECATION)
    .union(CpuIdStd7Ebx::SMEP)
    .union(CpuIdStd7Ebx::BMI2)
    .union(CpuIdStd7Ebx::ERMS) // extra
    .union(CpuIdStd7Ebx::INVPCID)
    .union(CpuIdStd7Ebx::DEPRECATE_FCS_FDS)
    // FIXME(AVX512 parity): Re-enable AVX512F/DQ/CD/BW/VL and leaf 0xD
    // OPMASK/ZMM XSAVE state together, after the EVEX executor covers the full
    // guest-visible Skylake-X AVX-512 surface: mask/merge/zero semantics,
    // upper-ZMM lanes, byte/word/dword/qword ops, memory forms, and exceptions.
    // Ubuntu/glibc IFUNC smoke coverage must prove those optimized paths run.
    // Advertising AVX512F/VL alone made Linux userspace select libc IFUNCs that
    // exercised unimplemented EVEX lanes.
    // .union(CpuIdStd7Ebx::AVX512F)
    // .union(CpuIdStd7Ebx::AVX512DQ)
    .union(CpuIdStd7Ebx::RDSEED)
    .union(CpuIdStd7Ebx::ADX)
    .union(CpuIdStd7Ebx::SMAP)
    .union(CpuIdStd7Ebx::CLFLUSHOPT)
    // AVX512CD/BW/VL also remain disabled.
    .union(CpuIdStd7Ebx::CLWB);

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
pub struct Corei7SkylakeX {
    /// How the CPUID frequency leaves 0x15/0x16 are reported.
    /// Bochs cpuid.cc get_freq_leaf_15/get_freq_leaf_16 (bochs-emu/Bochs#791).
    cpuid_freq: CpuidFreq,
    /// Emulated tick rate, consulted only in `CpuidFreq::Ips` mode.
    ips: u32,
}

impl Corei7SkylakeX {
    /// Leaf 0x15 (TSC / core crystal clock) per the configured mode.
    /// Bochs cpuid.cc bx_cpuid_t::get_freq_leaf_15.
    fn get_freq_leaf_15(&self) -> CpuidLeaf {
        match self.cpuid_freq {
            // EBX=0: TSC/crystal ratio not enumerated, ECX=0: crystal not
            // enumerated; guests fall back to timer calibration and measure
            // the true rate.
            CpuidFreq::None => CpuidLeaf::zeros(),
            // TSC frequency = core crystal clock * EBX/EAX: report the true
            // tick rate as a crystal running at `ips` Hz with a 1/1 ratio.
            CpuidFreq::Ips => CpuidLeaf {
                eax: 1,
                ebx: 1,
                ecx: self.ips,
                edx: 0,
            },
            // Hardware dump: Bochs corei7_skylake-x.cc — denominator 2,
            // numerator 0x124 (292), nominal crystal not enumerated.
            CpuidFreq::Hardware => CpuidLeaf {
                eax: 0x0000_0002,
                ebx: 0x0000_0124,
                ecx: 0x0000_0000,
                edx: 0x0000_0000,
            },
        }
    }

    /// Leaf 0x16 (processor frequency information) per the configured mode.
    /// Bochs cpuid.cc bx_cpuid_t::get_freq_leaf_16. Also serves leaves above
    /// the max standard leaf (Bochs corei7_skylake-x.cc `default:` arm).
    fn get_freq_leaf_16(&self) -> CpuidLeaf {
        match self.cpuid_freq {
            // EAX=0: processor base frequency not enumerated.
            CpuidFreq::None => CpuidLeaf::zeros(),
            // Base and max frequency of the emulated tick rate in MHz.
            CpuidFreq::Ips => {
                let mhz = ((self.ips as u64 + 500_000) / 1_000_000) as u32;
                CpuidLeaf {
                    eax: mhz,
                    ebx: mhz,
                    ecx: 100,
                    edx: 0,
                }
            }
            // Hardware dump: Bochs corei7_skylake-x.cc —
            // 3500 MHz base / 4000 MHz max / 100 MHz bus.
            CpuidFreq::Hardware => CpuidLeaf {
                eax: 0x0000_0DAC,
                ebx: 0x0000_0FA0,
                ecx: 0x0000_0064,
                edx: 0x0000_0000,
            },
        }
    }
}

impl BxCpuIdTrait for Corei7SkylakeX {
    fn get_name(&self) -> &'static str {
        "corei7_skylake_x"
    }

    fn set_cpuid_freq(&mut self, freq: CpuidFreq, ips: u32) {
        self.cpuid_freq = freq;
        self.ips = ips;
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
        Self {
            cpuid_freq: CpuidFreq::default(),
            ips: 4_000_000, // Bochs config.cc BXPN_IPS default; overwritten via set_cpuid_freq()
        }
    }

    /// Returns ISA extensions bitmask for Skylake-X.
    /// Matches Bochs corei7_skylake-x.cc constructor lines 42-109.
    fn get_isa_extensions_bitmask(&self) -> [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE] {
        let mut b = [0u32; BX_ISA_EXTENSIONS_ARRAY_SIZE];
        // Bochs base class: BX_ISA_386 always enabled
        enable_extension(&mut b, X86Feature::Isa386);
        // corei7_skylake-x.cc
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
        // See the AVX512 parity FIXME above. Keep the internal ISA bitmask,
        // leaf 7 feature bits, and leaf 0xD XSAVE state in lockstep.
        // enable_extension(&mut b, X86Feature::IsaAvx512);
        // enable_extension(&mut b, X86Feature::IsaAvx512Dq);
        // enable_extension(&mut b, X86Feature::IsaAvx512Cd);
        // enable_extension(&mut b, X86Feature::IsaAvx512Bw);
        enable_extension(&mut b, X86Feature::IsaClflushopt);
        enable_extension(&mut b, X86Feature::IsaClwb);
        if no_avx_mode() {
            let disable = |b: &mut [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE], f: X86Feature| {
                let idx = f as usize;
                b[idx / 32] &= !(1 << (idx % 32));
            };
            disable(&mut b, X86Feature::IsaAvx);
            disable(&mut b, X86Feature::IsaAvxFma);
            disable(&mut b, X86Feature::IsaAvx2);
            disable(&mut b, X86Feature::IsaAvxF16c);
            disable(&mut b, X86Feature::IsaBmi1);
            disable(&mut b, X86Feature::IsaBmi2);
            disable(&mut b, X86Feature::IsaAvx512);
        }
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
            // Bochs corei7_skylake-x.cc: max_std_leaf = 0x16.
            // How leaves 0x15/0x16 report frequencies is governed by the
            // cpuid_freq option (get_freq_leaf_15/16 below) — with the
            // default CpuidFreq::None they read as not enumerated and the
            // kernel PIT-calibrates the true tick rate (= IPS setting).
            0x00000000 => (
                0x00000016, // Max basic leaf (Bochs corei7_skylake-x.cc)
                0x756e6547, // "Genu"
                0x6c65746e, // "ntel"
                0x49656e69, // "ineI" → "GenuineIntel"
            ),

            // ── Leaf 1: Version / Feature Flags ─────────────────────────
            // Bochs corei7_skylake-x.cc
            0x00000001 => {
                let mut ecx = LEAF1_ECX_BASE;
                if no_avx_mode() {
                    ecx = ecx
                        .difference(CpuIdStd1Ecx::FMA)
                        .difference(CpuIdStd1Ecx::AVX)
                        .difference(CpuIdStd1Ecx::AVX_F16C);
                }
                (0x00050654, 0x00100800, ecx.bits(), LEAF1_EDX_BASE.bits())
            }

            // ── Leaf 2: Cache/TLB descriptors ───────────────────────────
            // Bochs corei7_skylake-x.cc
            0x00000002 => (0x76036301, 0x00F0B5FF, 0x00000000, 0x00C30000),

            // ── Leaf 3: Processor Serial Number (not supported) ─────────
            0x00000003 => (0, 0, 0, 0),

            // ── Leaf 4: Deterministic Cache Parameters ──────────────────
            // Bochs corei7_skylake-x.cc
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
            // Bochs corei7_skylake-x.cc
            0x00000005 => (
                64,         // EAX: smallest monitor-line size
                64,         // EBX: largest monitor-line size
                0x00000003, // ECX: extensions + interrupt break-event
                0x00002020, // EDX: C0/C1 sub-states
            ),

            // ── Leaf 6: Thermal/Power ───────────────────────────────────
            0x00000006 => (0x00000075, 0x00000002, 0x00000009, 0x00000000),

            // ── Leaf 7: Structured Extended Features ────────────────────
            // Bochs corei7_skylake-x.cc
            0x00000007 => {
                match ecx {
                    0 => {
                        let mut ebx = LEAF7_EBX_BASE;
                        if no_avx_mode() {
                            ebx = ebx
                                .difference(CpuIdStd7Ebx::AVX2)
                                .difference(CpuIdStd7Ebx::BMI1)
                                .difference(CpuIdStd7Ebx::BMI2)
                                .difference(CpuIdStd7Ebx::AVX512F)
                                .difference(CpuIdStd7Ebx::AVX512VL);
                        }
                        (
                            0x00000000, // EAX: max sub-leaf = 0
                            ebx.bits(), // EBX: feature flags
                            0x00000000, // ECX: no features
                            0x00000000, // EDX: no features
                        )
                    }
                    _ => (0, 0, 0, 0),
                }
            }

            // ── Leaves 8-9: Reserved ────────────────────────────────────
            0x00000008 | 0x00000009 => (0, 0, 0, 0),

            // ── Leaf A: Performance Monitoring ──────────────────────────
            // Bochs corei7_skylake-x.cc
            0x0000000A => (0x07300404, 0x00000000, 0x00000000, 0x00000603),

            // ── Leaf B: Extended Topology ───────────────────────────────
            // Bochs corei7_skylake-x.cc
            0x0000000B => {
                match ecx {
                    0 => (
                        0x00000001, // EAX: bits to shift for SMT
                        0x00000002, // EBX: logical threads at SMT level
                        0x00000100, // ECX: level=0, type=SMT(1)
                        0x00000000, // EDX: x2APIC ID
                    ),
                    1 => (
                        0x00000004, // EAX: bits to shift for core/package level
                        0x0000000C, // EBX: logical procs at this level
                        0x00000201, // ECX: level=1, type=Core(2)
                        0x00000000, // EDX: x2APIC ID
                    ),
                    _ => (0, 0, 0, 0),
                }
            }

            // ── Leaf C: Reserved ────────────────────────────────────────
            0x0000000C => (0, 0, 0, 0),

            // ── Leaf D: XSAVE state ─────────────────────────────────────
            // Bochs cpuid.cc — dynamically patched in cpuid() handler
            // Keep in lockstep with the AVX512 parity FIXME above: do not expose
            // OPMASK/ZMM XSAVE bits or subleaves 5-7 until the guest-visible EVEX
            // executor surface is complete.
            0x0000000D => {
                match ecx {
                    0 => (
                        0x00000007, // EAX: x87/SSE/YMM xcr0_suppmask (overridden dynamically)
                        0x00000240, // EBX: size for current xcr0 (overridden dynamically)
                        0x00000340, // ECX: max size for x87/SSE/YMM features = 832
                        0x00000000, // EDX: xcr0 upper 32 bits
                    ),
                    1 => (
                        // XSAVEOPT(0) + XSAVEC(1) + XGETBV_ECX1(2) + XSAVES(3)
                        0x0000000F, 0x00000000, // EBX: overridden dynamically
                        0x00000000, // ECX: IA32_XSS lower supported bits
                        0x00000000, // EDX: IA32_XSS upper supported bits
                    ),
                    // Per-component sub-leaves: (len, offset, flags, 0)
                    2 => (256, 576, 0, 0), // YMM state
                    _ => (0, 0, 0, 0),
                }
            }

            // ── Leaves E-14: Reserved ───────────────────────────────────
            0x0000000E..=0x00000014 => (0, 0, 0, 0),

            // ── Leaf 15: TSC/Crystal Clock Ratio ────────────────────────
            // TSC_freq = crystal_freq * (EBX/EAX); reporting mode governed
            // by cpuid_freq. Bochs cpuid.cc get_freq_leaf_15.
            0x00000015 => self.get_freq_leaf_15().as_tuple(),

            // ── Leaf 16: Processor Frequency ────────────────────────────
            // Reporting mode governed by cpuid_freq.
            // Bochs cpuid.cc get_freq_leaf_16 (also the default case).
            0x00000016 => self.get_freq_leaf_16().as_tuple(),

            // ── Extended CPUID Leaves ───────────────────────────────────
            0x80000000 => (
                0x80000008, // Max extended leaf
                0x00000000, 0x00000000, 0x00000000,
            ),

            // Leaf 0x80000001: Extended Feature Flags
            // Bochs cpuid.cc — SYSCALL patched dynamically
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
                0x00000000, 0x00000000,
            ),

            // ── Default: beyond max leaf → return leaf 0x16 data ────────
            // Bochs corei7_skylake-x.cc
            _ => {
                if eax > 0x80000008 {
                    (0, 0, 0, 0) // beyond max extended leaf
                } else if eax > 0x00000016 && eax < 0x80000000 {
                    // Beyond max standard leaf — Bochs returns leaf 0x16 data
                    // (corei7_skylake-x.cc `case 0x16: default:` merged arm),
                    // subject to the same cpuid_freq mode.
                    self.get_freq_leaf_16().as_tuple()
                } else {
                    (0, 0, 0, 0)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skylake_x_extended_topology_matches_bochs() {
        let cpuid = Corei7SkylakeX::new();

        assert_eq!(
            cpuid.get_cpuid_leaf(0x0000_0001, 0).1,
            0x0010_0800,
            "leaf 1 EBX must advertise the Bochs Skylake-X logical-processor count"
        );

        assert_eq!(
            cpuid.get_cpuid_leaf(0x0000_000B, 0),
            (0x0000_0001, 0x0000_0002, 0x0000_0100, 0x0000_0000)
        );
        assert_eq!(
            cpuid.get_cpuid_leaf(0x0000_000B, 1),
            (0x0000_0004, 0x0000_000C, 0x0000_0201, 0x0000_0000)
        );
    }

    #[test]
    fn skylake_x_does_not_advertise_incomplete_avx512() {
        let cpuid = Corei7SkylakeX::new();
        let (_, leaf7_ebx, _, _) = cpuid.get_cpuid_leaf(0x0000_0007, 0);
        assert_eq!(leaf7_ebx & CpuIdStd7Ebx::AVX512F.bits(), 0);
        assert_eq!(leaf7_ebx & CpuIdStd7Ebx::AVX512VL.bits(), 0);

        let bitmask = cpuid.get_isa_extensions_bitmask();
        let idx = X86Feature::IsaAvx512 as usize;
        assert_eq!(bitmask[idx / 32] & (1 << (idx % 32)), 0);

        let (leaf_d0_eax, _, leaf_d0_ecx, _) = cpuid.get_cpuid_leaf(0x0000_000D, 0);
        assert_eq!(leaf_d0_eax & 0x0000_00E0, 0);
        assert_eq!(leaf_d0_ecx, 0x0000_0340);
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_000D, 5), (0, 0, 0, 0));
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_000D, 6), (0, 0, 0, 0));
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_000D, 7), (0, 0, 0, 0));
    }

    #[test]
    fn skylake_x_advertises_fma_after_vex_fma3_support() {
        let cpuid = Corei7SkylakeX::new();
        let (_, _, leaf1_ecx, _) = cpuid.get_cpuid_leaf(0x0000_0001, 0);
        assert_ne!(leaf1_ecx & CpuIdStd1Ecx::FMA.bits(), 0);

        let bitmask = cpuid.get_isa_extensions_bitmask();
        let idx = X86Feature::IsaAvxFma as usize;
        assert_ne!(bitmask[idx / 32] & (1 << (idx % 32)), 0);
    }

    #[test]
    fn skylake_x_max_std_leaf_matches_bochs_in_every_cpuid_freq_mode() {
        // Bochs corei7_skylake-x.cc: max_std_leaf = 0x16 regardless of the
        // cpuid_freq mode — only the leaf CONTENTS change.
        for freq in [CpuidFreq::Hardware, CpuidFreq::None, CpuidFreq::Ips] {
            let mut cpuid = Corei7SkylakeX::new();
            cpuid.set_cpuid_freq(freq, 4_000_000);
            assert_eq!(cpuid.get_cpuid_leaf(0, 0).0, 0x0000_0016);
        }
    }

    #[test]
    fn skylake_x_cpuid_freq_default_reports_leaves_not_enumerated() {
        // rusty_box default is CpuidFreq::None: all-zero leaves 0x15/0x16
        // (the SDM-sanctioned opt-out) so guests PIT-calibrate the true rate.
        let cpuid = Corei7SkylakeX::new();
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_0015, 0), (0, 0, 0, 0));
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_0016, 0), (0, 0, 0, 0));
        // The above-max-leaf fallthrough serves leaf 0x16 data (Bochs
        // corei7_skylake-x.cc `default:` arm) and must follow the mode too.
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_0017, 0), (0, 0, 0, 0));
    }

    #[test]
    fn skylake_x_cpuid_freq_hardware_reports_bochs_dump_values() {
        // Bochs corei7_skylake-x.cc leaf 0x15/0x16 hardware dump.
        let mut cpuid = Corei7SkylakeX::new();
        cpuid.set_cpuid_freq(CpuidFreq::Hardware, 4_000_000);
        assert_eq!(
            cpuid.get_cpuid_leaf(0x0000_0015, 0),
            (0x0000_0002, 0x0000_0124, 0x0000_0000, 0x0000_0000)
        );
        assert_eq!(
            cpuid.get_cpuid_leaf(0x0000_0016, 0),
            (0x0000_0DAC, 0x0000_0FA0, 0x0000_0064, 0x0000_0000)
        );
        assert_eq!(
            cpuid.get_cpuid_leaf(0x0000_0017, 0),
            (0x0000_0DAC, 0x0000_0FA0, 0x0000_0064, 0x0000_0000)
        );
    }

    #[test]
    fn skylake_x_cpuid_freq_ips_reports_true_tick_rate() {
        // Bochs cpuid.cc get_freq_leaf_15/16 in `ips` mode: leaf 0x15 is a
        // crystal of `ips` Hz with a 1/1 ratio; leaf 0x16 is `ips` in MHz
        // (rounded), 100 MHz bus.
        let mut cpuid = Corei7SkylakeX::new();
        cpuid.set_cpuid_freq(CpuidFreq::Ips, 4_000_000);
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_0015, 0), (1, 1, 4_000_000, 0));
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_0016, 0), (4, 4, 100, 0));

        // Rounding: 120_600_000 -> 121 MHz; u32::MAX ips must not overflow.
        cpuid.set_cpuid_freq(CpuidFreq::Ips, 120_600_000);
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_0016, 0).0, 121);
        cpuid.set_cpuid_freq(CpuidFreq::Ips, u32::MAX);
        assert_eq!(cpuid.get_cpuid_leaf(0x0000_0016, 0).0, 4295);
    }
}
