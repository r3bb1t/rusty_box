use bitflags::bitflags;

use super::{decoder::BX_ISA_EXTENSIONS_ARRAY_SIZE, BxCpuC, Result};

#[derive(Debug, thiserror::Error)]
pub enum CpuIdError {}

pub(crate) trait BxCpuIdTrait {
    fn get_name(&self) -> &'static str;

    fn init(&mut self) {}

    fn get_cpu_extensions(&self, _extensions: &[u32]) {}

    fn get_vmx_extensions_bitmask(&self) -> Option<VMXExtensions>;
    fn get_svm_extensions_bitmask(&self) -> Option<SVMExtensions>;

    fn sanity_checks(&self) -> Result<()>;
}

bitflags! {
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

pub(crate) struct BxCpuId<'c, I: BxCpuIdTrait> {
    cpu: &'c BxCpuC<'c, I>,
    nprocessors: u32,
    ncores: u32,
    nthreads: u32,

    ia_extensions_bitmask: [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE],
}

impl<'c, I: BxCpuIdTrait> BxCpuId<'c, I> {
    pub fn new(cpu: &'c BxCpuC<I>, nprocessors: u32, ncores: u32, nthreads: u32) -> Self {
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
    /// [8:8]   Reserved
    /// [9:9]   Reserved
    /// [10:10] Pause filter support
    /// [11:11] Reserved
    /// [12:12] Pause filter threshold support
    /// [13:13] Advanced Virtual Interrupt Controller
    /// [14:14] Reserved
    /// [15:15] Nested Virtualization (virtualized VMLOAD and VMSAVE) Support
    /// [16:16] Virtual GIF
    /// [17:17] Guest Mode Execute Trap (CMET)
    pub struct SVMExtensions: u32 {
        const NestedPaging = 1 << 0;
        const LbrVirtualization = 1 << 1;
        const SvmLock = 1 << 2;
        const NripSave = 1 << 3;
        const Tscrate = 1 << 4;
        const VmcbCleanBits = 1 << 5;
        const FlushByAsid = 1 << 6;
        const DecodeAssist = 1 << 7;
        const Reserved8 = 1 << 8;
        const Reserved9 = 1 << 9;
        const PauseFilter = 1 << 10;
        const Reserved11 = 1 << 11;
        const PauseFilterThreshold = 1 << 12;
        const Avic = 1 << 13;
        const Reserved14 = 1 << 14;
        const NestedVirtualization = 1 << 15;
        const VirtualGif = 1 << 16;
        const Cmet = 1 << 17;
    }
}
