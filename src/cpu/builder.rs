use crate::cpu::{
    cpu::BX_MSR_MAX_INDEX, cpuid::BxCpuIdTrait, smm::SMMRAM_Fields, BxCpuC,
};

use super::Result;

#[derive(Debug)]
pub struct BxCpuBuilder<I: BxCpuIdTrait> {
    cpuid: I,
}

impl<I: BxCpuIdTrait> BxCpuBuilder<I> {
    pub fn new() -> Self {
        let cpuid = I::new();
        Self { cpuid }
    }

    pub fn build(self) -> Result<BxCpuC<'static, I>> {
        let cpuid = I::new();
        //let cpuid = cpuid_factory();

        let mut raw_cpu = BxCpuC {
            bx_cpuid: Default::default(),
            cpuid,
            ia_extensions_bitmask: Default::default(),
            vmx_extensions_bitmask: Default::default(),
            svm_extensions_bitmask: Default::default(),
            gen_reg: Default::default(),
            eflags: Default::default(),
            oszapc: Default::default(),
            prev_rip: Default::default(),
            prev_rsp: Default::default(),
            prev_ssp: Default::default(),
            speculative_rsp: Default::default(),
            icount: Default::default(),
            icount_last_sync: Default::default(),
            inhibit_mask: Default::default(),
            inhibit_icount: Default::default(),
            sregs: Default::default(),
            gdtr: Default::default(),
            idtr: Default::default(),
            ldtr: Default::default(),
            tr: Default::default(),
            dr: Default::default(),
            dr6: Default::default(),
            dr7: Default::default(),
            debug_trap: Default::default(),
            bx_cr0_t: Default::default(),
            r2: Default::default(),
            r3: Default::default(),
            r4: Default::default(),
            r4_suppmask: Default::default(),
            inaddr_width: Default::default(),
            fer_suppmask: Default::default(),
            sc_adjust: Default::default(),
            sc_offset: Default::default(),
            cr0: Default::default(),
            cr0_suppmask: Default::default(),
            a32_xss_suppmask: Default::default(),
            pkru: Default::default(),
            pkrs: Default::default(),
            rd_pkey: Default::default(),
            wr_pkey: Default::default(),
            uintr: Default::default(),
            the_i387: Default::default(),
            vmm: Default::default(),
            mxcsr: Default::default(),
            mxcsr_mask: Default::default(),
            opmask: Default::default(),
            monitor: Default::default(),
            lapic: Default::default(),
            smbase: Default::default(),
            msr: Default::default(),
            msrs: [Default::default(); BX_MSR_MAX_INDEX],
            in_vmx: Default::default(),
            in_vmx_guest: Default::default(),
            in_smm_vmx: Default::default(),
            in_smm_vmx_guest: Default::default(),
            vmcsptr: Default::default(),
            vmxonptr: Default::default(),
            vmcs: Default::default(),
            vmx_cap: Default::default(),
            vmcs_map: Default::default(),
            in_svm_guest: Default::default(),
            svm_gif: Default::default(),
            vmcbptr: Default::default(),
            vmcbhostptr: Default::default(),
            vmcb: Default::default(),
            in_event: Default::default(),
            nmi_unblocking_iret: Default::default(),
            ext: Default::default(),
            activity_state: Default::default(),
            pending_event: Default::default(),
            event_mask: Default::default(),
            async_event: Default::default(),
            in_smm: Default::default(),
            cpu_mode: Default::default(),
            user_pl: Default::default(),
            ignore_bad_msrs: Default::default(),
            cpu_state_use_ok: Default::default(),
            last_exception_type: Default::default(),
            cpuloop_stack_anchor: Default::default(),
            eip_page_bias: Default::default(),
            eip_page_window_size: Default::default(),
            eip_fetch_ptr: Default::default(),
            p_addr_fetch_page: Default::default(),
            esp_page_bias: Default::default(),
            esp_page_window_size: Default::default(),
            esp_host_ptr: Default::default(),
            p_addr_stack_page: Default::default(),
            esp_page_fine_granularity_mapping: Default::default(),
            alignment_check_mask: Default::default(),
            stats: Default::default(),
            pdptrcache: Default::default(),
            i_cache: Default::default(),
            fetch_mode_mask: Default::default(),
            address_xlation: Default::default(),
            smram_map: [0; SMMRAM_Fields::SMRAM_FIELD_LAST as _],
            phantom: Default::default(),
        };

        let config = Default::default();
        raw_cpu.initialize(config)?;

        Ok(raw_cpu)
    }
}
