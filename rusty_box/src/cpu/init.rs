#![allow(non_snake_case, dead_code)]

use alloc::vec::Vec;
use tracing::info;

use super::Result;
use crate::cpu::avx::AMX;

use crate::{
    cpu::{
        apic::BX_LAPIC_BASE_ADDR,
        cpu::{CpuActivityState, CpuMode},
        decoder::{features::X86Feature, BxSegregs, BX_GENERAL_REGISTERS, BX_NIL_REGISTER},
        descriptor::{
            BxDataAndCodeDescriptorEnum, SystemAndGateDescriptorEnum, SEG_ACCESS_ROK,
            SEG_ACCESS_WOK, SEG_VALID_CACHE,
        },
        eflags::EFlags,
        i387::BxPackedRegister,
        segment_ctrl_pro::parse_selector,
        svm::VmcbCache,
        vmcs::BX_INVALID_VMCSPTR,
        xmm::MXCSR_RESET,
        BxCpuC,
    },
    params::BxParams,
};

// Minimal MXCSR/feature related masks used at reset-time.
const MXCSR_DAZ: u32 = 1 << 6;
const MXCSR_MISALIGNED_EXCEPTION_MASK: u32 = 1 << 13;

use super::{
    cpudb::intel::core_i7_skylake::Corei7SkylakeX, cpuid::BxCpuIdTrait,
};

pub(super) fn cpuid_factory() -> impl BxCpuIdTrait {
    // Note: hardcode this for now
    Corei7SkylakeX {}
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ResetReason {
    Software = 10,
    Hardware = 11,
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    pub fn initialize(&mut self, config: BxParams) -> Result<()> {
        tracing::debug!("Initialized cpu model {}", self.cpuid.get_name());

        let _cpuid_features: Vec<X86Feature> = config
            .cpu_include_features
            .iter()
            .cloned()
            .filter(|feature| config.cpu_exclude_features.contains(feature))
            .collect();

        // Populate ISA extensions bitmask from CPUID model — matches Bochs init.cc
        self.ia_extensions_bitmask = self.cpuid.get_isa_extensions_bitmask();

        // Populate VMX/SVM bitmasks — matches Bochs init.cc
        self.vmx_extensions_bitmask = self.cpuid.get_vmx_extensions_bitmask();
        self.svm_extensions_bitmask = self.cpuid.get_svm_extensions_bitmask();

        // Note: sanity_checks() is called separately after initialize() to match original Bochs
        // Original order: initialize() -> sanity_checks() -> register_state()

        self.init_fetch_decode_tables()?;

        self.xsave_xrestor_init();

        {
            self.amx = if self.bx_cpuid_support_isa_extension(X86Feature::IsaAmx) {
                Some(AMX::default())
            } else {
                None
            };
        }

        self.vmcb = if self.bx_cpuid_support_isa_extension(X86Feature::IsaSvm) {
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
            ResetReason::Software => info!("cpu software reset"),
            ResetReason::Hardware => info!("cpu hardware reset"),
        }

        for i in 0..BX_GENERAL_REGISTERS {
            self.gen_reg[i].set_rrx(0)
        }

        self.gen_reg[BX_NIL_REGISTER].set_rrx(0);

        // Bochs init.cc: all general registers reset to 0
        // (includes ESP — BIOS sets SS:SP before any stack operations)

        self.eflags = EFlags::from_bits_retain(0x2); // Bit1 is always set

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

        self.sregs[cs_index].cache.u.set_segment_base(0xFFFF0000);
        self.sregs[cs_index].cache.u.set_segment_limit_scaled(0xFFFF);

        self.sregs[cs_index].cache.u.set_segment_g(false); /* byte granular */
        self.sregs[cs_index].cache.u.set_segment_d_b(false); /* 16bit default size */
        self.sregs[cs_index].cache.u.set_segment_l(false); /* 16bit default size */
        self.sregs[cs_index].cache.u.set_segment_avl(false); /* 16bit default size */

        // flushICaches();

        /* DS (Data Segment) and descriptor cache */
        let ds_index = BxSegregs::Ds as usize;
        parse_selector(0x0000, &mut self.sregs[ds_index].selector);
        self.sregs[ds_index].cache.valid = SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK;
        self.sregs[ds_index].cache.p = true;
        self.sregs[ds_index].cache.dpl = 0;
        self.sregs[ds_index].cache.segment = true; /* data/code segment */
        self.sregs[ds_index].cache.r#type = BxDataAndCodeDescriptorEnum::DataReadWriteAccessed as _;

        self.sregs[ds_index].cache.u.set_segment_base(0x00000000);
        self.sregs[ds_index].cache.u.set_segment_limit_scaled(0xFFFF);

        self.sregs[ds_index].cache.u.set_segment_avl(false); /* 16bit default size */
        self.sregs[ds_index].cache.u.set_segment_g(false); /* byte granular */
        self.sregs[ds_index].cache.u.set_segment_d_b(false); /* 16bit default size */
        self.sregs[ds_index].cache.u.set_segment_l(false); /* 16bit default size */

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
        self.ldtr.cache.u.set_segment_base(0x00000000);
        self.ldtr.cache.u.set_segment_limit_scaled(0xFFFF);
        self.ldtr.cache.u.set_segment_avl(false);
        self.ldtr.cache.u.set_segment_g(false); /* byte granular */

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
        self.tr.cache.u.set_segment_base(0x00000000);
        self.tr.cache.u.set_segment_limit_scaled(0xFFFF);
        self.tr.cache.u.set_segment_avl(false);
        self.tr.cache.u.set_segment_g(false); /* byte granular */

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
        } else {
            // Bochs init.cc: without x87, CR0 = 0x60000000 (no ET bit)
            self.cr0.set32(0x60000000);
        }

        // handle reserved bits
        self.cr2 = 0;
        self.cr3 = 0;

        self.cr4.set(0);

        self.cr4_suppmask = self.get_cr4_allow_mask();

        self.linaddr_width = 48;

        if source == ResetReason::Hardware {
            self.xcr0.set32(0x1);
        }

        self.xcr0_suppmask = self.get_xcr0_allow_mask();
        self.ia32_xss_suppmask = self.get_ia32_xss_allow_mask();

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
        self.efer_suppmask = self.get_efer_allow_mask();
        self.msr.star = 0;

        if self.bx_cpuid_support_isa_extension(X86Feature::IsaLongMode) {
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

        // MTRR/PAT registers must NOT change on INIT (software reset).
        // Bochs init.cc: all MTRR init is inside if (source == BX_RESET_HARDWARE).
        if source == ResetReason::Hardware {
            self.msr.mtrrphys = [0; 16];

            // Bochs init.cc: MTRR fix ranges initialized to 0 (not PAT default)
            self.msr.mtrrfix64k = BxPackedRegister { bytes: [0; 8] };
            self.msr.mtrrfix16k[0] = BxPackedRegister { bytes: [0; 8] };
            self.msr.mtrrfix16k[1] = BxPackedRegister { bytes: [0; 8] };

            self.msr.mtrrfix4k = [BxPackedRegister { bytes: [0; 8] }; 8];

            self.msr.pat = BxPackedRegister { bytes: 0x0007040600070406u64.to_le_bytes() };
            self.msr.mtrr_deftype = 0;
        }

        // All configurable MSRs do not change on INIT
        {
            self.msrs
                .iter_mut()
                .for_each(|msr| *msr = Default::default());
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
            if self.bx_cpuid_support_isa_extension(X86Feature::IsaSse2) {
                self.mxcsr_mask |= MXCSR_DAZ
            }

            if self.bx_cpuid_support_isa_extension(X86Feature::IsaMisalignedSse) {
                self.mxcsr_mask |= MXCSR_MISALIGNED_EXCEPTION_MASK
            }

            (0..8).for_each(|index| self.bx_write_opmask(index, 0));
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
        self.fred_event_info = 0;
        self.fred_event_data = 0;

        self.nmi_unblocking_iret = false;

        self.handle_cpu_context_change();

        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_any() {
            let reset_type = match source {
                ResetReason::Hardware => super::instrumentation::ResetType::Hardware,
                ResetReason::Software => super::instrumentation::ResetType::Software,
            };
            self.instrumentation.fire_reset(reset_type);
        }

    }

    fn write_32bit_regz(&mut self, index: usize, val: u64) {
        self.gen_reg[index].set_rrx(val)
    }

    // Minimal platform housekeeping helpers used during reset/context changes.

    /// Flush all TLB entries (both DTLB and ITLB) and invalidate prefetch/stack caches.
    /// Matching Bochs paging.cc TLB_flush(): flushes DTLB, ITLB, prefetch queue,
    /// stack cache, and breaks icache trace links.
    pub(crate) fn tlb_flush(&mut self) {
        self.invalidate_prefetch_q();
        self.invalidate_stack_cache();
        self.dtlb.flush();
        self.itlb.flush();
        // Bochs paging.cc — iCache.breakLinks()
        // Invalidates page-split icache entries and increments trace link timestamp.
        // Without this, page-boundary instructions survive TLB flush and serve
        // stale bytes from old physical pages after page remapping.
        self.i_cache.break_links();
    }

    /// Flush non-global TLB entries only (preserves entries with G bit set).
    /// Used by CR3 writes when CR4.PGE is enabled.
    /// Matching Bochs paging.cc TLB_flushNonGlobal().
    pub(super) fn tlb_flush_non_global(&mut self) {
        self.invalidate_prefetch_q();
        self.invalidate_stack_cache();
        self.dtlb.flush_non_global();
        self.itlb.flush_non_global();
        // Bochs paging.cc — iCache.breakLinks()
        self.i_cache.break_links();
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
        // Based on Bochs flag_ctrl_pro.cc handleInterruptMaskChange
        //
        // Bochs uses event_mask to gate interrupt delivery: when IF=0,
        // BX_EVENT_PENDING_INTR is added to event_mask (masked), so the
        // event stays in pending_event but is_unmasked_event_pending()
        // returns false. When IF=1, the event is unmasked, and if it was
        // pending, async_event is set to trigger delivery at next boundary.
        if self.eflags.contains(super::eflags::EFlags::IF_) {
            // EFLAGS.IF was set — unmask external interrupt events
            // Bochs flag_ctrl_pro.cc: unmask both PIC and LAPIC events
            self.unmask_event(
                Self::BX_EVENT_PENDING_INTR | Self::BX_EVENT_PENDING_LAPIC_INTR,
            );
        } else {
            // EFLAGS.IF was cleared — mask external interrupt events
            // Bochs flag_ctrl_pro.cc: mask both PIC and LAPIC events
            self.mask_event(
                Self::BX_EVENT_PENDING_INTR | Self::BX_EVENT_PENDING_LAPIC_INTR,
            );
        }
    }

    /// Enable VMX in IA32_FEATURE_CONTROL MSR for external firmware.
    ///
    /// Sets the lock bit (bit 0) and VMX-outside-SMX enable bit (bit 2).
    /// The Bochs BIOS does this itself; UEFI firmware (OVMF) relies on the
    /// emulator to pre-configure it via the fw_cfg path.
    pub fn allow_vmx_for_firmware(&mut self) {
        if !self.bx_cpuid_support_isa_extension(X86Feature::IsaVmx) {
            return;
        }
        // Lock bit (0) | VMX outside SMX enable (2)
        const BX_IA32_FEATURE_CONTROL_BITS: u32 = 0x5;
        self.msr.ia32_feature_ctrl |= BX_IA32_FEATURE_CONTROL_BITS;
    }

    /// Configure CPU for direct Linux kernel boot (bypassing BIOS).
    ///
    /// Sets up 32-bit protected mode with flat segments, matching what the
    /// Linux kernel's startup_32 entry point expects:
    /// - CR0.PE = 1 (protected mode)
    /// - CS = flat 32-bit code (selector 0x10)
    /// - DS = ES = FS = GS = SS = flat 32-bit data (selector 0x18)
    /// - GDT loaded with null + code + data entries
    /// - A20 enabled
    /// - Interrupts disabled
    ///
    /// After calling this, set ESP and ESI (boot_params pointer), then
    /// set RIP to the kernel entry point (code32_start, usually 0x100000).
    pub fn setup_for_direct_boot(&mut self, gdt_addr: u64) {
        // Set CR0: PE (protected mode) + ET (x87 extension type)
        self.cr0.set32(0x00000011);

        // Clear EFER (no long mode yet — kernel enables it itself)
        self.efer.set32(0);

        // Interrupts disabled, direction flag clear
        self.eflags = EFlags::from_bits_retain(0x2); // Bit 1 always set

        // Set up GDTR to point to GDT in memory
        self.gdtr.base = gdt_addr;
        self.gdtr.limit = 0x1F; // 4 entries × 8 bytes - 1 = 31

        // IDT: keep limit large enough that vector lookups don't #GP.
        // Entries will be null (zeroed RAM), but that's OK — the kernel
        // sets up its own IDT before enabling interrupts.
        self.idtr.base = 0;
        self.idtr.limit = 0xFFFF;

        // CS: selector 0x10 = GDT entry 2 (flat 32-bit code)
        let cs = BxSegregs::Cs as usize;
        parse_selector(0x0010, &mut self.sregs[cs].selector);
        self.sregs[cs].cache.valid = SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK;
        self.sregs[cs].cache.p = true;
        self.sregs[cs].cache.dpl = 0;
        self.sregs[cs].cache.segment = true;
        self.sregs[cs].cache.r#type =
            BxDataAndCodeDescriptorEnum::CodeExecReadAccessed as u8;
        self.sregs[cs].cache.u.set_segment_base(0);
        self.sregs[cs].cache.u.set_segment_limit_scaled(0xFFFFFFFF);
        self.sregs[cs].cache.u.set_segment_g(true);  // page granular
        self.sregs[cs].cache.u.set_segment_d_b(true); // 32-bit
        self.sregs[cs].cache.u.set_segment_l(false);  // not 64-bit
        self.sregs[cs].cache.u.set_segment_avl(false);

        // DS/ES/FS/GS/SS: selector 0x18 = GDT entry 3 (flat 32-bit data)
        let data_segs = [
            BxSegregs::Ds as usize,
            BxSegregs::Es as usize,
            BxSegregs::Fs as usize,
            BxSegregs::Gs as usize,
            BxSegregs::Ss as usize,
        ];
        for &seg_idx in &data_segs {
            parse_selector(0x0018, &mut self.sregs[seg_idx].selector);
            self.sregs[seg_idx].cache.valid = SEG_VALID_CACHE | SEG_ACCESS_ROK | SEG_ACCESS_WOK;
            self.sregs[seg_idx].cache.p = true;
            self.sregs[seg_idx].cache.dpl = 0;
            self.sregs[seg_idx].cache.segment = true;
            self.sregs[seg_idx].cache.r#type =
                BxDataAndCodeDescriptorEnum::DataReadWriteAccessed as u8;
            self.sregs[seg_idx].cache.u.set_segment_base(0);
            self.sregs[seg_idx].cache.u.set_segment_limit_scaled(0xFFFFFFFF);
            self.sregs[seg_idx].cache.u.set_segment_g(true);
            self.sregs[seg_idx].cache.u.set_segment_d_b(true);
            self.sregs[seg_idx].cache.u.set_segment_l(false);
            self.sregs[seg_idx].cache.u.set_segment_avl(false);
        }

        // CPU mode = protected (kernel transitions to long mode itself)
        self.cpu_mode = CpuMode::Ia32Protected;

        // Update fetch mode mask for 32-bit mode
        self.fetch_mode_mask
            .set(super::opcodes_table::FetchModeMask::D_B, true);
        self.fetch_mode_mask
            .remove(super::opcodes_table::FetchModeMask::LONG64);

        // Mask external interrupts (IF=0)
        self.mask_event(
            Self::BX_EVENT_PENDING_INTR | Self::BX_EVENT_PENDING_LAPIC_INTR,
        );

        info!("CPU configured for direct Linux boot (32-bit protected mode)");
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

    /// Register state for save/restore functionality.
    /// Called after initialize() and sanity_checks() in original Bochs.
    /// In Bochs this registers parameter tree nodes for save/restore.
    /// Our snapshot mechanism uses cpu/snapshot.rs save_snapshot_state() instead.
    pub fn register_state(&self) {
        tracing::trace!("CPU state registered");
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
    }
}
