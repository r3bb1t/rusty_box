// cpu/cpudb/intel/corei7_skylake-x.cc

use crate::cpu::cpuid::BxCpuIdTrait;

#[derive(Debug)]
pub struct Corei7SkylakeX {}

impl BxCpuIdTrait for Corei7SkylakeX {
    //const NAME: &'static str = "corei7_skylake_x";
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

    /// CPUID leaf data matching hardware values from corei7_skylake-x.txt (Logical CPU #0).
    /// Reference: cpp_orig/bochs/cpu/cpudb/intel/corei7_skylake-x.cc
    ///
    /// Leaf 1 EAX = 0x00050654: Family 6, Extended Model 5 → Skylake-X (stepping U0)
    /// Leaf 1 ECX = 0x7FFEFBBF: Full Skylake ECX feature flags
    /// Leaf 1 EDX = 0xBFEBFBFF: Full Skylake EDX feature flags (APIC, MSR, HTT, etc.)
    /// Leaf 0x80000008 EAX = 0x3024: 36 phys / 48 virt address bits (4GB emulator)
    fn get_cpuid_leaf(&self, eax: u32, ecx: u32) -> (u32, u32, u32, u32) {
        match eax {
            // Basic CPUID Information
            0x00000000 => (
                0x00000016, // Max basic leaf = 22
                0x756e6547, // "Genu"
                0x6c65746e, // "ntel"
                0x49656e69, // "ineI" → "GenuineIntel"
            ),
            0x00000001 => (
                0x00050654, // Family 6, Extended Model 5, Model 5 → Skylake-X (stepping U0)
                0x00010800, // Brand index 0, CLFLUSH=8 (64B line), 1 logical proc, APIC ID 0
                0x7FFEFBBF, // ECX: SSE3,PCLMULQDQ,DTES64,MON,DSCPL,VMX,EST,TM2,SSSE3,FMA,
                //      CX16,xTPR,PDCM,PCID,SSE4.1,SSE4.2,X2APIC,MOVBE,POPCNT,
                //      TSC-DL,AES,XSAVE,OSXSAVE,AVX,F16C,RDRAND
                0xBFEBFBFF, // EDX: FPU,VME,DE,PSE,TSC,MSR,PAE,MCE,CX8,APIC,SEP,MTRR,PGE,
                            //      MCA,CMOV,PAT,PSE36,CLFLUSH,DS,ACPI,MMX,FXSR,SSE,SSE2,SS,HTT,TM,PBE
            ),
            0x00000002 => (
                0x76036301, // Cache/TLB descriptor info (from hardware)
                0x00F0B5FF, 0x00000000, 0x00C30000,
            ),
            0x00000004 => {
                // Deterministic Cache Parameters — sub-leaf in ECX
                match ecx {
                    0 => (0x1C004121, 0x01C0003F, 0x0000003F, 0x00000000), // L1D 32KB
                    1 => (0x1C004122, 0x01C0003F, 0x0000003F, 0x00000000), // L1I 32KB
                    2 => (0x1C004143, 0x03C0003F, 0x000003FF, 0x00000000), // L2 1MB
                    3 => (0x1C03C163, 0x0280003F, 0x00002FFF, 0x00000004), // L3 8.25MB
                    _ => (0, 0, 0, 0),
                }
            }
            0x00000006 => (0x00000075, 0x00000002, 0x00000009, 0x00000000), // Thermal/Power
            0x00000007 => {
                // Structured Extended Feature Flags — sub-leaf in ECX
                match ecx {
                    0 => (
                        0x00000000, // max sub-leaf
                        0xD39FFFFB, // EBX: FSGSBASE,TSC_ADJ,BMI1,AVX2,FDP_DEP,SMEP,BMI2,
                        //      ERMS,INVPCID,FCS_FDS,AVX512F,AVX512DQ,RDSEED,ADX,
                        //      SMAP,AVX512CD,CLFLUSHOPT,CLWB,AVX512BW,AVX512VL
                        0x00000000, // ECX
                        0x00000000, // EDX
                    ),
                    _ => (0, 0, 0, 0),
                }
            }
            0x0000000A => (0x07300404, 0x00000000, 0x00000000, 0x00000603), // Perf monitoring
            // Extended CPUID Information
            0x80000000 => (
                0x80000008, // Max extended leaf
                0x00000000, 0x00000000, 0x00000000,
            ),
            0x80000001 => (
                0x00000000, 0x00000000, 0x00000121, // ECX: LAHF64, LZCNT, PREFETCHW
                0x2C100000, // EDX: NX, 1G-pages, RDTSCP, LM, SYSCALL
            ),
            // Brand string: "Intel(R) Core(TM) i7-7800X CPU @ 3.50GHz"
            // Bytes are little-endian in each register (LSB = first char)
            0x80000002 => (0x65746E49, 0x2952286C, 0x726F4320, 0x4D542865), // "Intel(R) Core(TM"
            0x80000003 => (0x37692029, 0x3038372D, 0x43205830, 0x40205550), // ") i7-7800X CPU @"
            0x80000004 => (0x352E3320, 0x7A484730, 0x00000000, 0x00000000), // " 3.50GHz\0..."
            0x80000005 => (0x00000000, 0x00000000, 0x00000000, 0x00000000), // reserved for Intel
            0x80000006 => (
                0x00000000, 0x00000000, 0x01006040, // L2: 512KB, 8-way, 64B line
                0x00000000,
            ),
            0x80000007 => (
                0x00000000, 0x00000000, 0x00000000, 0x00000100, // Invariant TSC
            ),
            0x80000008 => (
                // [7:0]=36 phys addr bits (4GB emulator), [15:8]=48 virt addr bits
                // Hardware has 46 phys bits; we use 36 to match our 4GB address space
                0x00003024, 0x00000000, 0x00000000, 0x00000000,
            ),
            // All other leaves return zeros
            _ => (0, 0, 0, 0),
        }
    }
}
