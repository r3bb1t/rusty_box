use crate::cpu::{
    cpu::BX_MSR_MAX_INDEX, cpuid::BxCpuIdTrait, smm::SMMRAM_Fields, tlb::Tlb, BxCpuC,
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
            cr0: Default::default(),
            cr2: Default::default(),
            cr3: Default::default(),
            cr4: Default::default(),
            cr4_suppmask: Default::default(),
            linaddr_width: Default::default(),
            efer: Default::default(),
            efer_suppmask: Default::default(),
            tsc_adjust: Default::default(),
            tsc_offset: Default::default(),
            xcr0: Default::default(),
            xcr0_suppmask: Default::default(),
            ia32_xss_suppmask: Default::default(),
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
            #[cfg(feature = "bx_configure_msrs")]
            msrs: [Default::default(); BX_MSR_MAX_INDEX],
            amx: Default::default(),
            in_vmx: Default::default(),
            in_vmx_guest: Default::default(),
            in_smm_vmx: Default::default(),
            in_smm_vmx_guest: Default::default(),
            vmcsptr: Default::default(),
            vmcs_memtype: Default::default(),
            vmxonptr: Default::default(),
            vmcs: Default::default(),
            vmx_cap: Default::default(),
            vmcs_map: Default::default(),
            in_svm_guest: Default::default(),
            svm_gif: Default::default(),
            vmcbptr: Default::default(),
            vmcbhostptr: Default::default(),
            vmcb_memtype: Default::default(),
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
            a20_mask: 0xFFFF_FFFF_FFFF_FFFF,
            cpu_state_use_ok: Default::default(),
            last_exception_type: Default::default(),
            cpuloop_stack_anchor: Default::default(),
            perf_icache_miss: 0,
            perf_prefetch: 0,
            eip_page_bias: Default::default(),
            eip_page_window_size: Default::default(),
            eip_fetch_ptr: Default::default(),
            p_addr_fetch_page: Default::default(),
            esp_page_bias: Default::default(),
            esp_page_window_size: Default::default(),
            esp_host_ptr: Default::default(),
            p_addr_stack_page: Default::default(),
            espPageMemtype: Default::default(),
            esp_page_fine_granularity_mapping: Default::default(),
            alignment_check_mask: Default::default(),
            stats: Default::default(),
            #[cfg(feature = "bx_debugger")]
            watchpoint: Default::default(),
            #[cfg(feature = "bx_debugger")]
            break_point: Default::default(),
            #[cfg(feature = "bx_debugger")]
            magic_break: Default::default(),
            #[cfg(feature = "bx_debugger")]
            stop_reason: Default::default(),
            #[cfg(feature = "bx_debugger")]
            trace: Default::default(),
            #[cfg(feature = "bx_debugger")]
            trace_reg: Default::default(),
            #[cfg(feature = "bx_debugger")]
            trace_mem: Default::default(),
            #[cfg(feature = "bx_debugger")]
            mode_break: Default::default(),
            #[cfg(feature = "bx_debugger")]
            vmexit_break: Default::default(),
            #[cfg(feature = "bx_debugger")]
            show_flag: Default::default(),
            #[cfg(feature = "bx_debugger")]
            guard_found: Default::default(),
            #[cfg(feature = "bx_instrumentation")]
            far_branch: Default::default(),
            dtlb: Tlb::new(),
            itlb: Tlb::new(),
            pdptrcache: Default::default(),
            i_cache: Default::default(),
            fetch_mode_mask: Default::default(),
            address_xlation: Default::default(),
            smram_map: [0; SMMRAM_Fields::SMRAM_FIELD_LAST as _],
            phantom: Default::default(),
            mem_ptr: None,
            mem_len: 0,
            mem_host_base: core::ptr::null_mut(),
            mem_host_len: 0,
            mem_bus: None,
            io_bus: None,
            pic_ptr: core::ptr::null_mut(),
            boot_debug_flags: 0,
            diag_hae_intr_delivered: 0,
            diag_hae_intr_if_blocked: 0,
            diag_hae_intr_no_pic: 0,
            diag_hae_intr_pic_empty: 0,
            diag_exception_counts: [0; 32],
            diag_ia_error_count: 0,
            diag_ia_error_last_rip: 0,
            diag_iac_vectors: [0; 256],
            diag_inject_ext_intr_count: 0,
            diag_inject_ext_intr_vectors: [0; 256],
            diag_soft_int_vectors: [0; 256],
            diag_soft_int_vectors_late: [0; 256],
            diag_addr_hits: [(0, 0); 8],
            diag_int10h_ah_hist: [0; 256],
            diag_int10h_tty_chars: [0; 128],
            diag_int10h_tty_count: 0,
            diag_int10h_first_icount: 0,
            diag_int10h_last_icount: 0,
            diag_int10h_tty_first_icount: 0,
            diag_int10h_tty_last_icount: 0,
            diag_first_pm_hlt_captured: false,
            diag_first_pm_hlt_icount: 0,
            diag_first_pm_hlt_regs: [0; 8],
            diag_first_pm_hlt_cs: 0,
            diag_first_pm_hlt_ss: 0,
            diag_first_pm_hlt_eflags: 0,
            diag_first_pm_hlt_rip: 0,
            diag_first_pm_hlt_stack: [0; 16],
            diag_rip_ring: [0; 64],
            diag_opcode_ring: [0; 64],
            diag_rip_ring_idx: 0,
            diag_pm_to_rm_count: 0,
            diag_rm_to_pm_count: 0,
            diag_retf16_count: 0,
        };

        let config = Default::default();
        raw_cpu.initialize(config)?;

        Ok(raw_cpu)
    }
}
