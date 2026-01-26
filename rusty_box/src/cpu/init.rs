use alloc::vec::Vec;
use tracing::info;

use super::Result;
use crate::{
    cpu::{
        apic::BX_LAPIC_BASE_ADDR,
        avx::amx::AMX,
        cpu::{CpuActivityState, CpuMode},
        decoder::{features::X86Feature, BxSegregs, BX_GENERAL_REGISTERS, BX_NIL_REGISTER},
        descriptor::{
            BxDataAndCodeDescriptorEnum, SystemAndGateDescriptorEnum, SEG_ACCESS_ROK,
            SEG_ACCESS_WOK, SEG_VALID_CACHE,
        },
        segment_ctrl_pro::parse_selector,
        svm::VmcbCache,
        vmcs::BX_INVALID_VMCSPTR,
        xmm::MXCSR_RESET,
            BxCpuC,
            i387::BxPackedRegister,
    },
    params::BxParams,
};

    // Minimal MXCSR/feature related masks used at reset-time.
    const MXCSR_DAZ: u32 = 1 << 6;
    const MXCSR_MISALIGNED_EXCEPTION_MASK: u32 = 1 << 13;

use super::{
    cpudb::intel::core_i7_skylake::Corei7SkylakeX, cpuid::BxCpuIdTrait, decoder::X86FeatureName,
};

pub(super) fn cpuid_factory() -> impl BxCpuIdTrait {
    // Note: hardcode this for now
    Corei7SkylakeX {}
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ResetReason {
    Software = 10,
    Hardware = 11,
    // Other(u8),
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub fn initialize(&mut self, config: BxParams) -> Result<()> {
        tracing::info!("Initialized cpu model {}", self.cpuid.get_name());

        let _cpuid_features: Vec<X86FeatureName> = config
            .cpu_include_features
            .iter()
            .cloned()
            .filter(|feature| config.cpu_exclude_features.contains(feature))
            .collect();

        self.svm_extensions_bitmask = self.cpuid.get_svm_extensions_bitmask();
        self.svm_extensions_bitmask = self.cpuid.get_svm_extensions_bitmask();

        // Note: sanity_checks() is called separately after initialize() to match original Bochs
        // Original order: initialize() -> sanity_checks() -> register_state()

        self.init_fetch_decode_tables()?;

        self.xsave_xrestor_init();

        #[cfg(feature = "bx_support_amx")]
        {
            self.amx = if self.bx_cpuid_support_isa_extension(X86Feature::IsaAMX) {
                Some(AMX::default())
            } else {
                None
            };
        }

        self.vmcb = if self.bx_cpuid_support_isa_extension(X86Feature::IsaSVM) {
            Some(VmcbCache::default())
        } else {
            None
        };

        self.init_msrs();

        self.smram_map = Self::init_smram()?;

        // Skip msrs stuff for now
        self.init_vmcs();

        self.init_statistics();

        Ok(())
    }

    pub fn reset(&mut self, source: ResetReason) {
        match source {
            ResetReason::Software => info!("cpu hardware reset"),
            ResetReason::Hardware => info!("cpu software reset"),
            _ => info!("cpu reset"),
        }

        for i in 0..BX_GENERAL_REGISTERS {
            self.gen_reg[i].rrx = 0
        }

        self.gen_reg[BX_NIL_REGISTER].rrx = 0;

        self.eflags = 0x2; // Bit1 is always set

        // clearEFlagsOSZAPC();
        if source == ResetReason::Hardware {
            self.icount = 0;
        }

        self.icount_last_sync = self.icount;

        self.inhibit_mask = 0;
        self.inhibit_icount = 0;

        self.activity_state = CpuActivityState::Active;
        self.debug_trap = 0;

        self.prev_rip = 0x0000FFF0;
        self.set_rip(0x0000FFF0);

        /* CS (Code Segment) and descriptor cache */
        /* Note: on a real cpu, CS initially points to upper memory.  After
         * the 1st jump, the descriptor base is zero'd out.  Since I'm just
         * going to jump to my BIOS, I don't need to do this.
         * For future reference:
         *   processor  cs.selector   cs.base    cs.limit    EIP
         *        8086    FFFF          FFFF0        FFFF   0000
         *        286     F000         FF0000        FFFF   FFF0
         *        386+    F000       FFFF0000        FFFF   FFF0
         */
        let cs_index = BxSegregs::Cs as usize;
        parse_selector(0xf000, &mut self.sregs[cs_index].selector);

        self.sregs[cs_index].cache.valid = SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK;
        self.sregs[cs_index].cache.p = true;
        self.sregs[cs_index].cache.dpl = 0;
        self.sregs[cs_index].cache.segment = true; /* data/code segment */
        self.sregs[cs_index].cache.r#type = BxDataAndCodeDescriptorEnum::DataReadWriteAccessed as _;

        self.sregs[cs_index].cache.u.segment.base = 0xFFFF0000;
        self.sregs[cs_index].cache.u.segment.limit_scaled = 0xFFFF;

        self.sregs[cs_index].cache.u.segment.g = false; /* byte granular */
        self.sregs[cs_index].cache.u.segment.d_b = false; /* 16bit default size */
        self.sregs[cs_index].cache.u.segment.l = false; /* 16bit default size */
        self.sregs[cs_index].cache.u.segment.avl = false; /* 16bit default size */

        // flushICaches();

        /* DS (Data Segment) and descriptor cache */
        let ds_index = BxSegregs::Ds as usize;
        parse_selector(0x0000, &mut self.sregs[ds_index].selector);
        self.sregs[ds_index].cache.valid = SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK;
        self.sregs[ds_index].cache.p = true;
        self.sregs[ds_index].cache.dpl = 0;
        self.sregs[ds_index].cache.segment = true; /* data/code segment */
        self.sregs[ds_index].cache.r#type = BxDataAndCodeDescriptorEnum::DataReadWriteAccessed as _;

        self.sregs[ds_index].cache.u.segment.base = 0x00000000;
        self.sregs[ds_index].cache.u.segment.limit_scaled = 0xFFFF;

        self.sregs[ds_index].cache.u.segment.avl = false; /* 16bit default size */
        self.sregs[ds_index].cache.u.segment.g = false; /* byte granular */
        self.sregs[ds_index].cache.u.segment.d_b = false; /* 16bit default size */
        self.sregs[ds_index].cache.u.segment.l = false; /* 16bit default size */

        // use DS segment as template for the others
        self.sregs[BxSegregs::Ss as usize] = self.sregs[ds_index].clone();
        self.sregs[BxSegregs::Es as usize] = self.sregs[ds_index].clone();
        self.sregs[BxSegregs::Fs as usize] = self.sregs[ds_index].clone();
        self.sregs[BxSegregs::Gs as usize] = self.sregs[ds_index].clone();

        /* GDTR (Global Descriptor Table Register) */
        self.gdtr.base = 0x00000000;
        self.gdtr.limit = 0xFFFF;

        /* IDTR (Interrupt Descriptor Table Register) */
        self.idtr.base = 0x00000000;
        self.idtr.limit = 0xFFFF; /* always byte granular */

        /* LDTR (Local Descriptor Table Register) */
        self.ldtr.selector.value = 0x0000;
        self.ldtr.selector.index = 0x0000;
        self.ldtr.selector.ti = 0;
        self.ldtr.selector.rpl = 0;

        self.ldtr.cache.valid = SEG_VALID_CACHE; /* valid */
        self.ldtr.cache.p = true; /* present */
        self.ldtr.cache.dpl = 0; /* field not used */
        self.ldtr.cache.segment = false; /* system segment */
        /* system segment */
        self.ldtr.cache.r#type = SystemAndGateDescriptorEnum::BxSysSegmentLdt as _;
        self.ldtr.cache.u.segment.base = 0x00000000;
        self.ldtr.cache.u.segment.limit_scaled = 0xFFFF;
        self.ldtr.cache.u.segment.avl = false;
        self.ldtr.cache.u.segment.g = false; /* byte granular */

        /* TR (Task Register) */
        self.tr.selector.value = 0x0000;
        self.tr.selector.index = 0x0000; /* undefined */
        self.tr.selector.ti = 0;
        self.tr.selector.rpl = 0;

        self.tr.cache.valid = SEG_VALID_CACHE; /* valid */
        self.tr.cache.p = true; /* present */
        self.tr.cache.dpl = 0; /* field not used */
        self.tr.cache.segment = false; /* system segment */
        /* system segment */
        self.tr.cache.r#type = SystemAndGateDescriptorEnum::BxSysSegmentBusy386Tss as _;
        self.ldtr.cache.u.segment.base = 0x00000000;
        self.ldtr.cache.u.segment.limit_scaled = 0xFFFF;
        self.ldtr.cache.u.segment.avl = false;
        self.ldtr.cache.u.segment.g = false; /* byte granular */

        self.cpu_mode = CpuMode::Ia32Real;

        // DR0 - DR7 (Debug Registers)
        self.dr = [0; 5];

        self.dr6.set32(0xFFFF0FF0);
        self.dr7.set32(0x00000400);

        self.in_smm = false;

        self.pending_event = 0;
        self.event_mask = 0;

        if source == ResetReason::Hardware {
            self.smbase = 0x30000; // do not change SMBASE on INIT
        }

        if self.bx_cpuid_support_isa_extension(X86Feature::IsaX87) {
            self.cr0.set32(0x60000010);
        }

        // handle reserved bits
        self.cr2 = 0;
        self.cr3 = 0;

        self.cr4.set32(0);

        // FIXME: implement this
        // self.cr4_suppmask = get_cr4_allow_mask();

        self.linaddr_width = 48;

        if source == ResetReason::Hardware {
            self.xcr0.set32(0x1);
        }

        // FIXME: implement this
        // BX_CPU_THIS_PTR xcr0_suppmask = get_xcr0_allow_mask();
        // BX_CPU_THIS_PTR ia32_xss_suppmask = get_ia32_xss_allow_mask();

        self.msr.ia32_xss = 0;

        self.msr.ia32_umwait_ctrl = 0;

        self.msr.svm_hsave_pa = 0;
        self.msr.svm_vm_cr = 0; // enable SVME if was disabled, clear LOCK bit

        self.msr.ia32_interrupt_ssp_table = 0;

        self.msr.ia32_cet_control[0] = 0;
        self.msr.ia32_cet_control[1] = 0;

        self.msr.ia32_pl_ssp = [0; 4];

        self.msr.ia32_spec_ctrl = 0;

        /* initialise MSR registers to defaults */
        self.msr.apicbase = BX_LAPIC_BASE_ADDR;
        self.lapic.reset(source as u8);
        self.msr.apicbase |= 0x900;
        self.lapic.set_base(self.msr.apicbase);
        self.lapic.enable_xapic_extensions();

        self.efer.set32(0);
        // Allow-mask helpers are not implemented yet; use conservative default
        self.efer_suppmask = 0;
        self.msr.star = 0;

        if self.bx_cpuid_support_isa_extension(X86Feature::IsaLONG_MODE) {
            if source == ResetReason::Hardware {
                self.msr.lstar = 0;
                self.msr.cstar = 0;
            }
            self.msr.fmask = 0x00020200;
            self.msr.kernelgsbase = 0;

            if source == ResetReason::Hardware {
                self.msr.tsc_aux = 0;
            }
        }

        self.tsc_offset = 0;

        if source == ResetReason::Hardware {
            // Set TSC to 0 on hardware reset
            // Pass 0 for system_ticks since we're at reset time
            self.set_tsc(0, 0);
        }

        if source == ResetReason::Hardware {
            self.msr.sysenter_cs_msr = 0;
            self.msr.sysenter_esp_msr = 0;
            self.msr.sysenter_eip_msr = 0;
        }

        // Do not change MTRR on INIT
        self.msr.mtrrphys = [0; 16];

        self.msr.mtrrfix64k = BxPackedRegister::default(); // all fix range MTRRs undefined according to manual
        self.msr.mtrrfix16k[0] = BxPackedRegister::default();
        self.msr.mtrrfix16k[1] = BxPackedRegister::default();

        self.msr.mtrrfix4k = [BxPackedRegister::default(); 8];

        self.msr.pat = BxPackedRegister {
            U64: 0x0007040600070406,
        };
        self.msr.mtrr_deftype = 0;

        // All configurable MSRs do not change on INIT
        #[cfg(feature = "bx_configure_msrs")]
        {
            self.msrs.iter_mut().for_each(|msr| *msr = Default::default());
        }

        self.ext = false;
        self.last_exception_type = 0;

        // invalidate the code prefetch queue
        self.eip_page_bias = 0;
        self.eip_page_window_size = 0;
        self.eip_fetch_ptr = None;

        // invalidate current stack page
        self.esp_page_bias = 0;
        self.esp_page_window_size = 0;
        self.esp_host_ptr = None;

        #[cfg(not(feature = "bx_support_smp"))]
        {
            self.esp_page_fine_granularity_mapping = 0;
        }

        #[cfg(feature = "bx_debugger")]
        {
            self.stop_reason = 0;
            self.magic_break = 0;
            self.trace = false;
            self.trace_reg = false;
            self.trace_mem = false;
            self.mode_break = false;
            self.vmexit_break = false;
        }

        // Reset the Floating Point Unit
        if source == ResetReason::Hardware {
            self.the_i387.reset();
        }

        self.cpu_state_use_ok = 0;

        // Reset XMM state - unchanged on #INIT
        if source == ResetReason::Hardware {
            self.mxcsr.mxcsr = MXCSR_RESET;
            self.mxcsr_mask = 0x0000ffbf;
            if self.bx_cpuid_support_isa_extension(X86Feature::IsaSSE2) {
                self.mxcsr_mask |= MXCSR_DAZ
            }

            if self.bx_cpuid_support_isa_extension(X86Feature::IsaMISALIGNED_SSE) {
                self.mxcsr_mask |= MXCSR_MISALIGNED_EXCEPTION_MASK
            }

            (0..8)
                .into_iter()
                .for_each(|index| self.bx_write_opmask(index, 0));
        }

        self.in_vmx = false;
        self.in_vmx_guest = false;

        self.in_smm_vmx = false;
        self.in_smm_vmx_guest = false;

        self.vmcsptr = BX_INVALID_VMCSPTR;
        self.vmxonptr = BX_INVALID_VMCSPTR;

        self.set_VMCSPTR(self.vmcsptr);
        if source == ResetReason::Hardware {
            self.msr.ia32_feature_ctrl = 0;
        }

        self.set_VMCBPTR(0);
        self.in_svm_guest = false;
        self.svm_gif = true;

        self.in_event = false;

        self.nmi_unblocking_iret = false;

        // #[cfg(not(feature = "bx_support_smp"))]
        // {
        //     // notice if I'm the bootstrap processor.  If not, do the equivalent of
        //     // a HALT instruction.
        //     let apid_id = self.lapic.get_id();
        //     // TODO: implement this
        //
        //     // if (BX_BOOTSTRAP_PROCESSOR == apic_id) {}
        //     //     // boot normally
        //     //     BX_CPU_THIS_PTR msr.apicbase |=  0x100; /* set bit 8 BSP */
        //     //     BX_INFO(("CPU[%d] is the bootstrap processor", apic_id));
        //     //   } else {}
        //     //     // it's an application processor, halt until IPI is heard.
        //     //     BX_CPU_THIS_PTR msr.apicbase &= ~0x100; /* clear bit 8 BSP */
        //     //     BX_INFO(("CPU[%d] is an application processor. Halting until SIPI.", apic_id));
        //     //     enter_sleep_state(BX_ACTIVITY_STATE_WAIT_FOR_SIPI);
        //     //   }
        // }
        self.handle_cpu_context_change();

        // self.cpuid.dump_cpuid();
        // self.cpuid.dump_features();
    }

    fn write_32bit_regz(&mut self, index: usize, val: u64) {
        self.gen_reg[index].rrx = val
    }

    // Minimal platform housekeeping helpers used during reset/context changes.
    pub(super) fn tlb_flush(&mut self) {
        // Placeholder: concrete TLB invalidation will be implemented elsewhere.
    }

    pub(super) fn invalidate_prefetch_q(&mut self) {
        self.eip_fetch_ptr = None;
        self.eip_page_bias = 0;
        self.eip_page_window_size = 0;
    }

    pub(super) fn invalidate_stack_cache(&mut self) {
        self.esp_host_ptr = None;
        self.esp_page_bias = 0;
        self.esp_page_window_size = 0;
    }

    pub(super) fn handle_interrupt_mask_change(&mut self) {
        // no-op for now; real implementation updates APIC/MSR state
    }

    fn init_statistics(&mut self) {
        // Not now
    }

    /// Perform CPU sanity checks
    /// 
    /// Called after initialize() and before register_state() to match original Bochs order.
    /// Original: BX_CPU(0)->initialize(); BX_CPU(0)->sanity_checks(); BX_CPU(0)->register_state();
    pub fn sanity_checks(&mut self) -> Result<()> {
        // Late
        Ok(())
    }

    /// Register state for save/restore functionality
    /// Called after initialize() and sanity_checks() in original Bochs
    pub fn register_state(&self) {
        // TODO: Implement state registration for save/restore
        tracing::debug!("CPU state registered");
    }

    /// Sets the VMCS pointer and performs associated memory mapping setup
    /// Mirrors the C++ BX_CPU_C::set_VMCSPTR behavior
    fn set_VMCSPTR(&mut self, vmxptr: u64) {
        self.vmcsptr = vmxptr;
        // Note: In a full implementation, this would also set up vmcshostptr
        // via getHostMemAddr() and configure memory type (vmcs_memtype).
        // For now, a simple assignment suffices.
    }

    /// Sets the VMCB pointer for SVM mode
    /// Mirrors the C++ BX_CPU_C::set_VMCBPTR behavior
    fn set_VMCBPTR(&mut self, _vmcb_ptr: u64) {
        // Note: In a full implementation, this would set up the VMCB host
        // pointer and memory mapping. For now, this is a placeholder.
    }}