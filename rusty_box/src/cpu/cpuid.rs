#![allow(dead_code)]
use bitflags::bitflags;

use super::{decoder::BX_ISA_EXTENSIONS_ARRAY_SIZE, BxCpuC, Result};

#[derive(Debug, thiserror::Error)]
pub enum CpuIdError {}

pub trait BxCpuIdTrait: core::fmt::Debug {
    fn get_name(&self) -> &'static str;

    fn init(&mut self) {}

    fn get_cpu_extensions(&self, _extensions: &[u32]) {}

    /// Returns the ISA extensions bitmask for this CPU model.
    /// Matches Bochs cpuid.cc enable_cpu_extension() calls in each model's constructor.
    fn get_isa_extensions_bitmask(&self) -> [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE] {
        [0; BX_ISA_EXTENSIONS_ARRAY_SIZE]
    }

    fn get_vmx_extensions_bitmask(&self) -> Option<VMXExtensions>;
    fn get_svm_extensions_bitmask(&self) -> Option<SVMExtensions>;

    fn sanity_checks(&self) -> Result<()>;

    fn new() -> Self;

    /// Returns (EAX, EBX, ECX, EDX) for the given CPUID leaf and sub-leaf.
    /// Default implementation returns zeros for all unimplemented leaves.
    fn get_cpuid_leaf(&self, _eax: u32, _ecx: u32) -> (u32, u32, u32, u32) {
        (0, 0, 0, 0)
    }
}

bitflags! {
    #[derive(Debug)]
    pub struct VMXExtensions: u32 {
        /// TPR shadow
        const TprShadow = 1 << 0;
        /// Virtual NM;
        const VirtualNmi = 1 << 1;
        /// APIC Access Virtualizatio;
        const ApicVirtualization = 1 << 2;
        /// WBINVD VMEXI;
        const WbinvdVmexit = 1 << 3;
        /// Save/Restore MSR_PERF_GLOBAL_CTR;
        const PerfGlobalCtrl = 1 << 4;
        /// Monitor trap Flag (MTF;
        const MonitorTrapFlag = 1 << 5;
        /// Virtualize X2API;
        const X2apicVirtualization = 1 << 6;
        /// Extended Page Tables (EPT;
        const EPT = 1 << 7;
        /// VPI;
        const VPID = 1 << 8;
        /// Unrestricted Gues;
        const UnrestrictedGuest = 1 << 9;
        /// VMX preemption time;
        const PreemptionTimer = 1 << 10;
        /// Disable Save/Restore of MSR_DEBUGCT;
        const SaveDebugctlDisable = 1 << 11;
        /// Save/Restore MSR_PA;
        const PAT = 1 << 12;
        /// Save/Restore MSR_EFE;
        const EFER = 1 << 13;
        /// Descriptor Table VMEXI;
        const DescriptorTableExit = 1 << 14;
        /// Pause Loop Exitin;
        const PauseLoopExiting = 1 << 15;
        /// EPTP switching (VM Function 0;
        const EptpSwitching = 1 << 16;
        /// Extended Page Tables (EPT) A/D Bit;
        const EptAccessDirty = 1 << 17;
        /// Virtual Interrupt Deliver;
        const VintrDelivery = 1 << 18;
        /// Posted Interrupts suppor;
        const PostedInterrupts = 1 << 19;
        /// VMCS Shadowin;
        const VmcsShadowing = 1 << 20;
        /// EPT Violation (#VE) exceptio;
        const EptException = 1 << 21;
        /// Page Modification Loggin;
        const PML = 1 << 22;
        /// Sub Page Protectio;
        const SPP = 1 << 23;
        /// TSC Scalin;
        const TscScaling = 1 << 24;
        /// Allow software interrupt injection with instruction length ;
        const SwInterruptInjectionIlen0 = 1 << 25;
        /// Mode-Based Execution Control (XU/XS;
        const MbeControl = 1 << 26;
        /// Virtualize MSR IA32_SPEC_CTR;
        const SpecCtrlVirtualization = 1 << 27;
    }
}

// TODO: remove self reference

pub(crate) struct BxCpuId<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> {
    cpu: &'c BxCpuC<'c, I, T>,
    nprocessors: u32,
    ncores: u32,
    nthreads: u32,

    ia_extensions_bitmask: [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE],
}

impl<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuId<'c, I, T> {
    pub fn new(cpu: &'c BxCpuC<'c, I, T>, nprocessors: u32, ncores: u32, nthreads: u32) -> Self {
        let ia_extensions_bitmask = [0; BX_ISA_EXTENSIONS_ARRAY_SIZE];

        Self {
            cpu,
            nprocessors,
            ncores,
            nthreads,
            ia_extensions_bitmask,
        }
    }
}

bitflags! {
    /// CPUID defines - SVM features CPUID[0x8000000A].EDX
    /// ----------------------------
    /// [0:0]   NP - Nested paging support
    /// [1:1]   LBR virtualization
    /// [2:2]   SVM Lock
    /// [3:3]   NRIPS - Next RIP save on VMEXIT
    /// [4:4]   TscRate - MSR based TSC ratio control
    /// [5:5]   VMCB Clean bits support
    /// [6:6]   Flush by ASID support
    /// [7:7]   Decode assists support
    /// [8:8]   PMC (Performance Monitoring Counter) virtualization
    /// [9:9]   Reserved
    /// [10:10] Pause filter support
    /// [11:11] Reserved
    /// [12:12] Pause filter threshold support
    /// [13:13] Advanced Virtual Interrupt Controller
    /// [14:14] Reserved
    /// [15:15] Nested Virtualization (virtualized VMLOAD and VMSAVE) Support
    /// [16:16] Virtual GIF
    /// [17:17] Guest Mode Execute Trap (GMET)
    /// [18:18] Advanced Virtual Interrupt Controller X2APIC mode
    /// [19:19] SSS_Check: SVM Supervisor Shadow Stack restrictions
    /// [20:20] SPEC_CTRL MSR virtualization
    /// [21:21] ROGPT: Read Only Guest Page Table support
    /// [22:22] Reserved
    /// [23:23] Host MCE override (when host CR4.MCE=1 and guest CR4.MCE=0)
    /// [24:24] INVLPGB/TLBSYNC hypervisor enable and TLBSYNC intercept
    /// [25:25] NMI virtualization
    /// [26:26] IBS (Instruction Based Sampling) virtualization
    /// [27:27] ExtLvtAvicAccessChg: Extended Interrupt LVT Register AVIC Access changes
    /// [28:28] Guest VMCB address check
    /// [29:29] Bus Lock Threshold
    /// [30:30] Idle HLT intercept
    /// [31:31] Reserved
    #[derive(Debug)]
    pub struct SVMExtensions: u32 {
        const NestedPaging = 1 << 0;
        const LbrVirtualization = 1 << 1;
        const SvmLock = 1 << 2;
        const NripSave = 1 << 3;
        const Tscrate = 1 << 4;
        const VmcbCleanBits = 1 << 5;
        const FlushByAsid = 1 << 6;
        const DecodeAssist = 1 << 7;
        const PmcVirtualization = 1 << 8;
        const Reserved9 = 1 << 9;
        const PauseFilter = 1 << 10;
        const Reserved11 = 1 << 11;
        const PauseFilterThreshold = 1 << 12;
        const Avic = 1 << 13;
        const Reserved14 = 1 << 14;
        const NestedVirtualization = 1 << 15;
        const VirtualGif = 1 << 16;
        const Gmet = 1 << 17;
        const X2avic = 1 << 18;
        const SssCheck = 1 << 19;
        const SpecCtrlVirt = 1 << 20;
        const Rogpt = 1 << 21;
        const Reserved22 = 1 << 22;
        const HostMceOverride = 1 << 23;
        const TlbiCtrl = 1 << 24;
        const Vnmi = 1 << 25;
        const IbsVirtualization = 1 << 26;
        const ExtLvtApicAccessChanges = 1 << 27;
        const NestedVmcbAddrCheck = 1 << 28;
        const BusLockThreshold = 1 << 29;
        const IdleHalt = 1 << 30;
        const Reserved31 = 1 << 31;
    }
}


bitflags! {
    /// CPUID Leaf 7 Subleaf 1 EAX — Structured Extended Feature Flags
    /// [0:0]   SHA-512 support
    /// [1:1]   SM3 support
    /// [2:2]   SM4 support
    /// [3:3]   RAO-INT support
    /// [4:4]   AVX-VNNI support
    /// [5:5]   AVX512-BF16 support
    /// [6:6]   LASS support
    /// [7:7]   CMPccXADD support
    /// [8:8]   Architectural Performance Monitoring
    /// [9:9]   Reserved
    /// [10:10] Fast zero-length REP MOVSB
    /// [11:11] Fast zero-length REP STOSB
    /// [12:12] Fast zero-length REP CMPSB/SCASB
    /// [16:13] Reserved
    /// [17:17] Flexible Return and Event Delivery (FRED) support
    /// [18:18] LKGS instruction support
    /// [19:19] WRMSRNS instruction
    /// [20:20] NMI source reporting
    /// [21:21] AMX-FP16 support
    /// [22:22] HRESET support
    /// [23:23] AVX-IFMA support
    /// [26:24] Reserved
    /// [27:27] MSRLIST support
    /// [31:28] Reserved
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdStd7Subleaf1Eax: u32 {
        const SHA512              = 1 << 0;
        const SM3                 = 1 << 1;
        const SM4                 = 1 << 2;
        const RAO_INT             = 1 << 3;
        const AVX_VNNI            = 1 << 4;
        const AVX512_BF16         = 1 << 5;
        const LASS                = 1 << 6;
        const CMPCCXADD           = 1 << 7;
        const ARCH_PERFMON        = 1 << 8;
        const FAST_ZEROLEN_MOVSB  = 1 << 10;
        const FAST_ZEROLEN_STOSB  = 1 << 11;
        const FAST_ZEROLEN_CMPSB  = 1 << 12;
        const FRED                = 1 << 17;
        const LKGS                = 1 << 18;
        const WRMSRNS             = 1 << 19;
        const AMX_FP16            = 1 << 21;
        const HRESET              = 1 << 22;
        const AVX_IFMA            = 1 << 23;
        const LAM                 = 1 << 26;
        const MSRLIST             = 1 << 27;
    }
}

bitflags! {
    /// CPUID Leaf 7 Subleaf 0 ECX — Structured Extended Feature Flags (Bochs cpuid.h).
    /// Only the bits Bochs defines are listed; PKU/UMIP/etc. live in the Skylake-X
    /// per-CPU file (`cpudb/intel/core_i7_skylake.rs`) until cross-CPU sharing is needed.
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdStd7Subleaf0Ecx: u32 {
        /// CET Shadow Stack support — Bochs cpuid.h.
        const CET_SS              = 1 <<  7;
    }
}

bitflags! {
    /// CPUID Leaf 7 Subleaf 0 EDX — Structured Extended Feature Flags (Bochs cpuid.h).
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdStd7Subleaf0Edx: u32 {
        /// CET Indirect Branch Tracking support — Bochs cpuid.h.
        const CET_IBT             = 1 << 20;
    }
}

bitflags! {
    /// CPUID Leaf 7 Subleaf 1 EDX — Structured Extended Feature Flags (Bochs cpuid.h).
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdStd7Subleaf1Edx: u32 {
        /// CET Supervisor Shadow Stack support — Bochs cpuid.h.
        const CET_SSS             = 1 << 18;
    }
}

bitflags! {
    /// CPUID Leaf 0x1E EAX — AMX Extensions
    /// [0:0] AMX-INT8
    /// [1:1] AMX-BF16
    /// [2:2] AMX-COMPLEX
    /// [3:3] AMX-FP16
    /// [4:4] AMX-FP8
    /// [5:5] AMX-TRANSPOSE (deprecated)
    /// [6:6] AMX-TF32 (FP19)
    /// [7:7] AMX-AVX512
    /// [8:8] AMX-MOVRS
    #[derive(Debug, Clone, Copy)]
    pub struct CpuIdAmxExtensionsEax: u32 {
        const AMX_INT8      = 1 << 0;
        const AMX_BF16      = 1 << 1;
        const AMX_COMPLEX   = 1 << 2;
        const AMX_FP16      = 1 << 3;
        const AMX_FP8       = 1 << 4;
        const AMX_TRANSPOSE = 1 << 5;
        const AMX_TF32      = 1 << 6;
        const AMX_AVX512    = 1 << 7;
        const AMX_MOVRS     = 1 << 8;
    }
}