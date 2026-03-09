// cpu/cpudb/intel/corei7_skylake-x.cc

use crate::cpu::cpuid::BxCpuIdTrait;
use crate::cpu::decoder::{features::X86Feature, BX_ISA_EXTENSIONS_ARRAY_SIZE};

/// Set a feature bit in the ISA extensions bitmask.
/// Mirrors Bochs bx_cpuid_t::enable_cpu_extension().
fn enable_extension(bitmask: &mut [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE], feature: X86Feature) {
    let idx = feature as usize;
    bitmask[idx / 32] |= 1 << (idx % 32);
}

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
            0x00000005 => (
                // MONITOR/MWAIT Leaf — Bochs cpuid.cc:86-118
                // Skylake passes edx_power_states = 0x00002020
                64,         // EAX: smallest monitor-line size (cache line = 64)
                64,         // EBX: largest monitor-line size (cache line = 64)
                0x00000003, // ECX: extensions supported (bit 0) + interrupt break-event (bit 1)
                0x00002020, // EDX: C0/C1 sub-states
            ),
            0x00000006 => (0x00000075, 0x00000002, 0x00000009, 0x00000000), // Thermal/Power
            0x00000007 => {
                // Structured Extended Feature Flags — sub-leaf in ECX
                // Bochs corei7_skylake-x.cc:445-493 + cpuid.cc:964-1082
                // Only advertise features that Bochs enables for Skylake-X
                match ecx {
                    0 => (
                        0x00000000, // max sub-leaf
                        // EBX: Bochs-matching dynamic value for Skylake-X ISA extensions:
                        // bit 0:  FSGSBASE
                        // bit 1:  TSC_ADJUST
                        // bit 3:  BMI1
                        // bit 5:  AVX2
                        // bit 6:  FDP_DEPRECATION
                        // bit 7:  SMEP
                        // bit 8:  BMI2
                        // bit 9:  ERMS (enhanced REP MOVSB/STOSB)
                        // bit 10: INVPCID
                        // bit 13: FCS/FDS deprecation
                        // bit 16: AVX512F
                        // bit 17: AVX512DQ
                        // bit 18: RDSEED
                        // bit 19: ADX
                        // bit 20: SMAP
                        // bit 23: CLFLUSHOPT
                        // bit 24: CLWB
                        // bit 28: AVX512CD
                        // bit 30: AVX512BW
                        // bit 31: AVX512VL
                        0xD19F27EB,
                        0x00000000, // ECX
                        0x00000000, // EDX
                    ),
                    _ => (0, 0, 0, 0),
                }
            }
            0x0000000A => (0x07300404, 0x00000000, 0x00000000, 0x00000603), // Perf monitoring
            0x0000000B => {
                // Extended Topology Enumeration — Bochs cpuid.cc:163-177
                match ecx {
                    0 => (
                        0x00000001, // EAX: bits to shift for next level (SMT=1)
                        0x00000001, // EBX: logical procs at this level
                        0x00000100, // ECX: level number=0, type=SMT(1)
                        0x00000000, // EDX: x2APIC ID (filled dynamically)
                    ),
                    1 => (
                        0x00000000, // EAX: bits to shift
                        0x00000001, // EBX: logical procs at this level
                        0x00000201, // ECX: level number=1, type=Core(2)
                        0x00000000, // EDX: x2APIC ID
                    ),
                    _ => (0, 0, 0, 0),
                }
            }
            0x0000000D => {
                // XSAVE state — Bochs cpuid.cc:206-268
                // Subleaf 0: XCR0 valid bits + max sizes
                // NOTE: EAX (xcr0_suppmask) and EBX (size for current XCR0) are
                // fixed up dynamically in cpuid() handler using CPU state.
                // Static values here use xcr0_suppmask=0xE7 (FPU+SSE+YMM+OPMASK+ZMM_HI256+HI_ZMM)
                match ecx {
                    0 => (
                        0x000000E7, // EAX: xcr0_suppmask (overridden dynamically)
                        0x00000240, // EBX: max size for current xcr0 (576 = 0x240 for x87+SSE; overridden dynamically)
                        0x00000A80, // ECX: max size for all features = 2688 (0xA80)
                        0x00000000, // EDX: xcr0 upper 32 bits
                    ),
                    1 => (
                        // XSAVEOPT(bit 0) + compaction(bit 1) + XGETBV ECX=1(bit 2) + XSAVES(bit 3)
                        0x0000000F, // EAX: XSAVEOPT + XSAVEC + XGETBV_ECX1 + XSAVES
                        0x00000000, // EBX: size of XSAVE area for XCRO|XSS
                        0x00000000, // ECX: IA32_XSS lower supported bits
                        0x00000000, // EDX: IA32_XSS upper supported bits
                    ),
                    // Per-component sub-leaves: (len, offset, flags, 0)
                    2 => (256, 576, 0, 0),    // YMM state: len=256, offset=576
                    5 => (64, 1088, 0, 0),    // OPMASK: len=64, offset=1088
                    6 => (512, 1152, 0, 0),   // ZMM_HI256: len=512, offset=1152
                    7 => (1024, 1664, 0, 0),  // HI_ZMM: len=1024, offset=1664
                    _ => (0, 0, 0, 0),
                }
            }
            0x00000015 => (
                // TSC/Crystal Clock Info — Bochs corei7_skylake-x.cc:196
                0x00000002, // EAX: denominator
                0x00000124, // EBX: numerator
                0x00000000, // ECX: nominal frequency (0 = not enumerated)
                0x00000000,
            ),
            0x00000016 => (
                // Processor Frequency Info — Bochs corei7_skylake-x.cc:200
                0x00000DAC, // EAX: base frequency (MHz) = 3500
                0x00000FA0, // EBX: max frequency (MHz) = 4000
                0x00000064, // ECX: bus (reference) frequency (MHz) = 100
                0x00000000,
            ),
            // Extended CPUID Information
            0x80000000 => (
                0x80000008, // Max extended leaf
                0x00000000, 0x00000000, 0x00000000,
            ),
            0x80000001 => (
                0x00000000, 0x00000000, 0x00000121, // ECX: LAHF64, LZCNT, PREFETCHW
                0x2C100000, // EDX: NX, RDTSCP, LM, 1G-pages, SYSCALL
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
                0x00003024,
                // EBX: bit 9 = WBNOINVD (Bochs cpuid.cc:1497-1498 sets for long mode CPUs)
                0x00000200,
                0x00000000,
                0x00000000,
            ),
            // Bochs corei7_skylake-x.cc:198-201: default case falls through
            // to leaf 0x16 (Processor Frequency). This matches real hardware behavior
            // where unknown standard leaves return the last valid leaf's data.
            // Extended leaves > max_ext_leaf return zeros (handled by check below).
            _ => {
                if eax >= 0x80000000 && eax > 0x80000008 {
                    (0, 0, 0, 0) // beyond max extended leaf
                } else if eax > 0x00000016 && eax < 0x80000000 {
                    // Beyond max standard leaf — Bochs returns leaf 0x16 data
                    (0x00000DAC, 0x00000FA0, 0x00000064, 0x00000000)
                } else {
                    (0, 0, 0, 0) // reserved/unhandled standard leaves
                }
            }
        }
    }
}
