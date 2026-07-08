//! CPU state save/restore for the snapshot mechanism.
//! This file lives in cpu/ so it has pub(super) access to BxCpuC fields.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    crregs::{BxCr0, BxCr4, BxDr6, BxDr7, BxEfer, Xcr0},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Save CPU state to a byte vector.
    pub fn save_snapshot_state(&self) -> alloc::vec::Vec<u8> {
        let mut buf = alloc::vec::Vec::with_capacity(16384);

        // General registers (20 × 8 = 160 bytes)
        for i in 0..20 {
            buf.extend_from_slice(&self.gen_reg[i].rrx().to_le_bytes());
        }

        // EFLAGS + instruction state
        buf.extend_from_slice(&self.eflags_materialized().to_le_bytes());
        buf.extend_from_slice(&self.icount.to_le_bytes());
        buf.extend_from_slice(&self.prev_rip.to_le_bytes());
        buf.extend_from_slice(&self.prev_rsp.to_le_bytes());
        buf.extend_from_slice(&self.inhibit_mask.to_le_bytes());
        buf.extend_from_slice(&self.inhibit_icount.to_le_bytes());

        // Lazy flags
        buf.extend_from_slice(&self.oszapc.result.to_le_bytes());
        buf.extend_from_slice(&self.oszapc.auxbits.to_le_bytes());

        // Segment registers (6 user + GDTR + IDTR + LDTR + TR)
        for i in 0..6 {
            write_seg_reg(&mut buf, &self.sregs[i]);
        }
        write_global_seg(&mut buf, &self.gdtr);
        write_global_seg(&mut buf, &self.idtr);
        write_seg_reg(&mut buf, &self.ldtr);
        write_seg_reg(&mut buf, &self.tr);

        // Control registers
        buf.extend_from_slice(&self.cr0.bits().to_le_bytes());
        buf.extend_from_slice(&self.cr2.to_le_bytes());
        buf.extend_from_slice(&self.cr3.to_le_bytes());
        buf.extend_from_slice(&self.cr4.bits().to_le_bytes());
        buf.extend_from_slice(&self.cr4_suppmask.to_le_bytes());
        buf.extend_from_slice(&self.efer.bits().to_le_bytes());
        buf.extend_from_slice(&self.efer_suppmask.to_le_bytes());

        // Debug registers
        for i in 0..5 {
            buf.extend_from_slice(&self.dr[i].to_le_bytes());
        }
        buf.extend_from_slice(&self.dr6.bits().to_le_bytes());
        buf.extend_from_slice(&self.dr7.bits().to_le_bytes());
        buf.extend_from_slice(&self.debug_trap.to_le_bytes());

        // XCR0, protection keys, misc
        buf.extend_from_slice(&self.xcr0.value.to_le_bytes());
        buf.extend_from_slice(&self.xcr0_suppmask.to_le_bytes());
        buf.extend_from_slice(&self.pkru.to_le_bytes());
        buf.extend_from_slice(&self.pkrs.to_le_bytes());
        buf.extend_from_slice(&self.linaddr_width.to_le_bytes());
        buf.extend_from_slice(&self.tsc_adjust.to_le_bytes());
        buf.extend_from_slice(&self.tsc_offset.to_le_bytes());

        // FPU state
        buf.extend_from_slice(&self.the_i387.cwd.to_le_bytes());
        buf.extend_from_slice(&self.the_i387.swd.to_le_bytes());
        buf.extend_from_slice(&self.the_i387.twd.to_le_bytes());
        buf.extend_from_slice(&self.the_i387.foo.to_le_bytes());
        buf.extend_from_slice(&self.the_i387.fip.to_le_bytes());
        buf.extend_from_slice(&self.the_i387.fdp.to_le_bytes());
        buf.extend_from_slice(&self.the_i387.fcs.to_le_bytes());
        buf.extend_from_slice(&self.the_i387.fds.to_le_bytes());
        for i in 0..8 {
            buf.extend_from_slice(&self.the_i387.st_space[i].signif.to_le_bytes());
            buf.extend_from_slice(&self.the_i387.st_space[i].sign_exp.to_le_bytes());
        }

        // Vector registers (32 × 64 = 2048 bytes)
        for i in 0..32 {
            buf.extend_from_slice(self.vmm[i].raw());
        }
        buf.extend_from_slice(&self.mxcsr.mxcsr.to_le_bytes());
        buf.extend_from_slice(&self.mxcsr_mask.to_le_bytes());
        for i in 0..8 {
            buf.extend_from_slice(&self.opmask[i].rrx().to_le_bytes());
        }

        // MSR block
        buf.extend_from_slice(&self.msr.apicbase.to_le_bytes());
        buf.extend_from_slice(&self.msr.star.to_le_bytes());
        buf.extend_from_slice(&self.msr.lstar.to_le_bytes());
        buf.extend_from_slice(&self.msr.cstar.to_le_bytes());
        buf.extend_from_slice(&self.msr.fmask.to_le_bytes());
        buf.extend_from_slice(&self.msr.kernelgsbase.to_le_bytes());
        buf.extend_from_slice(&self.msr.tsc_aux.to_le_bytes());
        buf.extend_from_slice(&self.msr.sysenter_cs_msr.to_le_bytes());
        buf.extend_from_slice(&self.msr.sysenter_esp_msr.to_le_bytes());
        buf.extend_from_slice(&self.msr.sysenter_eip_msr.to_le_bytes());
        buf.extend_from_slice(&self.msr.pat.U64().to_le_bytes());
        for v in &self.msr.mtrrphys {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.extend_from_slice(&self.msr.mtrrfix64k.U64().to_le_bytes());
        for r in &self.msr.mtrrfix16k {
            buf.extend_from_slice(&r.U64().to_le_bytes());
        }
        for r in &self.msr.mtrrfix4k {
            buf.extend_from_slice(&r.U64().to_le_bytes());
        }
        buf.extend_from_slice(&self.msr.mtrr_deftype.to_le_bytes());

        // CPU mode/state
        buf.extend_from_slice(&(self.cpu_mode as u32).to_le_bytes());
        buf.extend_from_slice(&(u8::from(self.activity_state) as u32).to_le_bytes());
        buf.extend_from_slice(&self.pending_event.to_le_bytes());
        buf.extend_from_slice(&self.event_mask.to_le_bytes());
        buf.extend_from_slice(&self.async_event.to_le_bytes());
        buf.push(self.user_pl as u8);
        buf.push(self.in_smm as u8);
        buf.push(self.ext as u8);
        buf.push(self.nmi_unblocking_iret as u8);
        buf.extend_from_slice(&self.last_exception_type.to_le_bytes());
        buf.extend_from_slice(&self.smbase.to_le_bytes());
        buf.extend_from_slice(&self.alignment_check_mask.to_le_bytes());
        buf.extend_from_slice(&self.a20_mask.to_le_bytes());

        // Virtualization mode tracking + anchors (Bochs init.cc
        // register_vmx_state / register_svm_state param trees). The VMCS
        // cache is the authoritative VMCS store in this port (VMREAD/VMWRITE
        // operate on the cache, not guest memory), so the memory section
        // cannot capture it and it must be serialized here.
        buf.push(self.in_vmx as u8);
        buf.push(self.in_vmx_guest as u8);
        buf.push(self.in_smm_vmx as u8);
        buf.push(self.in_smm_vmx_guest as u8);
        buf.push(self.in_svm_guest as u8);
        buf.push(self.svm_gif as u8);
        buf.extend_from_slice(&self.vmcsptr.to_le_bytes());
        buf.extend_from_slice(&self.vmxonptr.to_le_bytes());
        buf.extend_from_slice(&self.vmcbptr.to_le_bytes());
        buf.extend_from_slice(&self.msr.svm_hsave_pa.to_le_bytes());
        buf.extend_from_slice(&self.msr.svm_vm_cr.to_le_bytes());
        save_vmcs_cache(&self.vmcs, &mut buf);
        match &self.vmcb {
            Some(vmcb) => {
                buf.push(1);
                save_vmcb_cache(vmcb, &mut buf);
            }
            None => buf.push(0),
        }

        buf
    }

    /// Restore CPU state from a byte slice.
    pub fn restore_snapshot_state(&mut self, d: &[u8]) {
        let mut off = 0;

        // General registers
        for i in 0..20 {
            self.gen_reg[i].set_rrx(u64_at(d, &mut off));
        }
        self.eflags = EFlags::from_bits_retain(u32_at(d, &mut off));
        self.icount = u64_at(d, &mut off);
        self.prev_rip = u64_at(d, &mut off);
        self.prev_rsp = u64_at(d, &mut off);
        self.inhibit_mask = u32_at(d, &mut off);
        self.inhibit_icount = u64_at(d, &mut off);
        self.oszapc.result = u64_at(d, &mut off);
        self.oszapc.auxbits = u64_at(d, &mut off);

        // Segment registers
        for i in 0..6 {
            read_seg_reg(d, &mut off, &mut self.sregs[i]);
        }
        read_global_seg(d, &mut off, &mut self.gdtr);
        read_global_seg(d, &mut off, &mut self.idtr);
        read_seg_reg(d, &mut off, &mut self.ldtr);
        read_seg_reg(d, &mut off, &mut self.tr);

        // Control registers
        self.cr0 = BxCr0::from_bits_retain(u32_at(d, &mut off));
        self.cr2 = u64_at(d, &mut off);
        self.cr3 = u64_at(d, &mut off);
        self.cr4 = BxCr4::from_bits_retain(u64_at(d, &mut off));
        self.cr4_suppmask = u64_at(d, &mut off);
        self.efer = BxEfer::from_bits_retain(u32_at(d, &mut off));
        self.efer_suppmask = u32_at(d, &mut off);
        for i in 0..5 {
            self.dr[i] = u64_at(d, &mut off);
        }
        self.dr6 = BxDr6::from_bits_retain(u32_at(d, &mut off));
        self.dr7 = BxDr7::from_bits_retain(u32_at(d, &mut off));
        self.debug_trap = u32_at(d, &mut off);
        self.xcr0 = Xcr0 {
            value: u32_at(d, &mut off),
        };
        self.xcr0_suppmask = u32_at(d, &mut off);
        self.pkru = u32_at(d, &mut off);
        self.pkrs = u32_at(d, &mut off);
        self.linaddr_width = d[off];
        off += 1;
        self.tsc_adjust = i64_at(d, &mut off);
        self.tsc_offset = i64_at(d, &mut off);

        // FPU
        self.the_i387.cwd = u16_at(d, &mut off);
        self.the_i387.swd = u16_at(d, &mut off);
        self.the_i387.twd = u16_at(d, &mut off);
        self.the_i387.foo = u16_at(d, &mut off);
        self.the_i387.fip = u64_at(d, &mut off);
        self.the_i387.fdp = u64_at(d, &mut off);
        self.the_i387.fcs = u16_at(d, &mut off);
        self.the_i387.fds = u16_at(d, &mut off);
        for i in 0..8 {
            self.the_i387.st_space[i].signif = u64_at(d, &mut off);
            self.the_i387.st_space[i].sign_exp = u16_at(d, &mut off);
        }

        // Vector registers
        for i in 0..32 {
            self.vmm[i].raw_mut().copy_from_slice(&d[off..off + 64]);
            off += 64;
        }
        self.mxcsr.mxcsr = u32_at(d, &mut off);
        self.mxcsr_mask = u32_at(d, &mut off);
        for i in 0..8 {
            self.opmask[i].set_rrx(u64_at(d, &mut off));
        }

        // MSRs
        self.msr.apicbase = u64_at(d, &mut off) as _;
        self.msr.star = u64_at(d, &mut off);
        self.msr.lstar = u64_at(d, &mut off);
        self.msr.cstar = u64_at(d, &mut off);
        self.msr.fmask = u32_at(d, &mut off);
        self.msr.kernelgsbase = u64_at(d, &mut off);
        self.msr.tsc_aux = u32_at(d, &mut off);
        self.msr.sysenter_cs_msr = u32_at(d, &mut off);
        self.msr.sysenter_esp_msr = u64_at(d, &mut off);
        self.msr.sysenter_eip_msr = u64_at(d, &mut off);
        self.msr.pat.set_U64(u64_at(d, &mut off));
        for v in self.msr.mtrrphys.iter_mut() {
            *v = u64_at(d, &mut off);
        }
        self.msr.mtrrfix64k.set_U64(u64_at(d, &mut off));
        for r in self.msr.mtrrfix16k.iter_mut() {
            r.set_U64(u64_at(d, &mut off));
        }
        for r in self.msr.mtrrfix4k.iter_mut() {
            r.set_U64(u64_at(d, &mut off));
        }
        self.msr.mtrr_deftype = u32_at(d, &mut off);

        // CPU mode/state
        let mode_val = u32_at(d, &mut off) as u8;
        self.cpu_mode = match mode_val {
            0 => super::cpu::CpuMode::Ia32Real,
            1 => super::cpu::CpuMode::Ia32V8086,
            2 => super::cpu::CpuMode::Ia32Protected,
            3 => super::cpu::CpuMode::LongCompat,
            4 => super::cpu::CpuMode::Long64,
            _ => super::cpu::CpuMode::Ia32Real,
        };
        let activity_val = u32_at(d, &mut off) as u8;
        self.activity_state = match activity_val {
            0 => super::cpu::CpuActivityState::Active,
            1 => super::cpu::CpuActivityState::Hlt,
            2 => super::cpu::CpuActivityState::Shutdown,
            3 => super::cpu::CpuActivityState::WaitForSipi,
            4 => super::cpu::CpuActivityState::Mwait,
            5 => super::cpu::CpuActivityState::MwaitIf,
            _ => super::cpu::CpuActivityState::Active,
        };
        self.pending_event = u32_at(d, &mut off);
        self.event_mask = u32_at(d, &mut off);
        self.async_event = u32_at(d, &mut off);
        self.user_pl = d[off] != 0;
        off += 1;
        self.in_smm = d[off] != 0;
        off += 1;
        self.ext = d[off] != 0;
        off += 1;
        self.nmi_unblocking_iret = d[off] != 0;
        off += 1;
        self.last_exception_type = i32_at(d, &mut off);
        self.smbase = u32_at(d, &mut off);
        self.alignment_check_mask = u32_at(d, &mut off);
        self.a20_mask = u64_at(d, &mut off);

        // Virtualization mode tracking + anchors (mirrors the save order)
        self.in_vmx = d[off] != 0;
        off += 1;
        self.in_vmx_guest = d[off] != 0;
        off += 1;
        self.in_smm_vmx = d[off] != 0;
        off += 1;
        self.in_smm_vmx_guest = d[off] != 0;
        off += 1;
        self.in_svm_guest = d[off] != 0;
        off += 1;
        self.svm_gif = d[off] != 0;
        off += 1;
        self.vmcsptr = u64_at(d, &mut off);
        self.vmxonptr = u64_at(d, &mut off);
        self.vmcbptr = u64_at(d, &mut off);
        self.msr.svm_hsave_pa = u64_at(d, &mut off);
        self.msr.svm_vm_cr = u32_at(d, &mut off);
        restore_vmcs_cache(&mut self.vmcs, d, &mut off);
        let has_vmcb = d[off] != 0;
        off += 1;
        if has_vmcb {
            let mut vmcb = super::svm::VmcbCache::default();
            restore_vmcb_cache(&mut vmcb, d, &mut off);
            self.vmcb = Some(vmcb);
        } else {
            self.vmcb = None;
        }
        // Cached host pointer into guest RAM is invalid after a restore —
        // force the slow path to re-resolve it.
        self.vmcbhostptr = 0;

        // Post-restore: flush TLB and icache, clear fetch pointers
        self.tlb_flush();
        self.i_cache.flush_all();
        self.eip_fetch_ptr = None;
        self.esp_host_ptr = None;
    }
}

// ============================================================================
// Segment register helpers
// ============================================================================

use super::descriptor::{BxGlobalSegmentReg, BxSegmentReg};

fn write_seg_reg(buf: &mut alloc::vec::Vec<u8>, seg: &BxSegmentReg) {
    // Selector
    buf.extend_from_slice(&seg.selector.value.to_le_bytes());
    buf.extend_from_slice(&seg.selector.index.to_le_bytes());
    buf.extend_from_slice(&seg.selector.ti.to_le_bytes());
    buf.push(seg.selector.rpl);
    // Descriptor: top-level fields
    buf.extend_from_slice(&seg.cache.valid.to_le_bytes());
    buf.push(seg.cache.p as u8);
    buf.push(seg.cache.dpl);
    buf.push(seg.cache.segment as u8);
    buf.push(seg.cache.r#type);
    buf.extend_from_slice(&seg.cache.u.segment_base().to_le_bytes());
    buf.extend_from_slice(&seg.cache.u.segment_limit_scaled().to_le_bytes());
    buf.push(seg.cache.u.segment_g() as u8);
    buf.push(seg.cache.u.segment_d_b() as u8);
    buf.push(seg.cache.u.segment_l() as u8);
    buf.push(seg.cache.u.segment_avl() as u8);
}

fn write_global_seg(buf: &mut alloc::vec::Vec<u8>, seg: &BxGlobalSegmentReg) {
    buf.extend_from_slice(&seg.base.to_le_bytes());
    buf.extend_from_slice(&seg.limit.to_le_bytes());
}

fn read_seg_reg(d: &[u8], off: &mut usize, seg: &mut BxSegmentReg) {
    seg.selector.value = u16_at(d, off);
    seg.selector.index = u16_at(d, off);
    seg.selector.ti = u16_at(d, off);
    seg.selector.rpl = d[*off];
    *off += 1;
    seg.cache.valid = u32_at(d, off);
    seg.cache.p = d[*off] != 0;
    *off += 1;
    seg.cache.dpl = d[*off];
    *off += 1;
    seg.cache.segment = d[*off] != 0;
    *off += 1;
    seg.cache.r#type = d[*off];
    *off += 1;
    seg.cache.u.set_segment_base(u64_at(d, off));
    seg.cache.u.set_segment_limit_scaled(u32_at(d, off));
    seg.cache.u.set_segment_g(d[*off] != 0);
    *off += 1;
    seg.cache.u.set_segment_d_b(d[*off] != 0);
    *off += 1;
    seg.cache.u.set_segment_l(d[*off] != 0);
    *off += 1;
    seg.cache.u.set_segment_avl(d[*off] != 0);
    *off += 1;
}

fn read_global_seg(d: &[u8], off: &mut usize, seg: &mut BxGlobalSegmentReg) {
    seg.base = u64_at(d, off);
    seg.limit = u16_at(d, off);
}

// ============================================================================
// Binary read helpers
// ============================================================================

fn u16_at(d: &[u8], off: &mut usize) -> u16 {
    let v = u16::from_le_bytes([d[*off], d[*off + 1]]);
    *off += 2;
    v
}

fn u32_at(d: &[u8], off: &mut usize) -> u32 {
    let v = u32::from_le_bytes(
        d[*off..*off + 4]
            .try_into()
            .unwrap_or_else(|_| unreachable!("slice is exactly 4 bytes")),
    );
    *off += 4;
    v
}

fn i32_at(d: &[u8], off: &mut usize) -> i32 {
    let v = i32::from_le_bytes(
        d[*off..*off + 4]
            .try_into()
            .unwrap_or_else(|_| unreachable!("slice is exactly 4 bytes")),
    );
    *off += 4;
    v
}

fn u64_at(d: &[u8], off: &mut usize) -> u64 {
    let v = u64::from_le_bytes(
        d[*off..*off + 8]
            .try_into()
            .unwrap_or_else(|_| unreachable!("slice is exactly 8 bytes")),
    );
    *off += 8;
    v
}

fn i64_at(d: &[u8], off: &mut usize) -> i64 {
    let v = i64::from_le_bytes(
        d[*off..*off + 8]
            .try_into()
            .unwrap_or_else(|_| unreachable!("slice is exactly 8 bytes")),
    );
    *off += 8;
    v
}

// ============================================================================
// Virtualization state (VMX/SVM) serialization
// ============================================================================
//
// The VMCS cache (`BxVmcs`) is the authoritative VMCS store in this port —
// VMREAD/VMWRITE and the vmentry/vmexit paths operate on the cache, never on
// the guest-memory VMCS region — so a snapshot must carry it explicitly.
// Field order matches the `BxVmcs` struct definition exactly; save and
// restore must stay in lockstep.

fn save_vmcs_cache(vm: &super::vmx::BxVmcs, buf: &mut alloc::vec::Vec<u8>) {
    buf.push(vm.launched as u8);
    buf.extend_from_slice(&vm.host_cr0.to_le_bytes());
    buf.extend_from_slice(&vm.host_cr3.to_le_bytes());
    buf.extend_from_slice(&vm.host_cr4.to_le_bytes());
    buf.extend_from_slice(&vm.host_rsp.to_le_bytes());
    buf.extend_from_slice(&vm.host_rip.to_le_bytes());
    buf.extend_from_slice(&vm.host_cs_selector.to_le_bytes());
    buf.extend_from_slice(&vm.host_ss_selector.to_le_bytes());
    buf.extend_from_slice(&vm.host_ds_selector.to_le_bytes());
    buf.extend_from_slice(&vm.host_es_selector.to_le_bytes());
    buf.extend_from_slice(&vm.host_fs_selector.to_le_bytes());
    buf.extend_from_slice(&vm.host_gs_selector.to_le_bytes());
    buf.extend_from_slice(&vm.host_tr_selector.to_le_bytes());
    buf.extend_from_slice(&vm.host_fs_base.to_le_bytes());
    buf.extend_from_slice(&vm.host_gs_base.to_le_bytes());
    buf.extend_from_slice(&vm.host_tr_base.to_le_bytes());
    buf.extend_from_slice(&vm.host_gdtr_base.to_le_bytes());
    buf.extend_from_slice(&vm.host_idtr_base.to_le_bytes());
    buf.extend_from_slice(&vm.host_ia32_efer.to_le_bytes());
    buf.extend_from_slice(&vm.host_ia32_pat.to_le_bytes());
    buf.extend_from_slice(&vm.host_sysenter_cs.to_le_bytes());
    buf.extend_from_slice(&vm.host_sysenter_esp.to_le_bytes());
    buf.extend_from_slice(&vm.host_sysenter_eip.to_le_bytes());
    buf.extend_from_slice(&vm.host_perf_global_ctrl.to_le_bytes());
    buf.extend_from_slice(&vm.host_pkrs.to_le_bytes());
    buf.extend_from_slice(&vm.host_ia32_spec_ctrl.to_le_bytes());
    buf.extend_from_slice(&vm.host_ia32_s_cet.to_le_bytes());
    buf.extend_from_slice(&vm.host_ssp.to_le_bytes());
    buf.extend_from_slice(&vm.host_interrupt_ssp_table_addr.to_le_bytes());
    buf.extend_from_slice(&vm.host_fred_config.to_le_bytes());
    for v in &vm.host_fred_rsp {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&vm.host_fred_stack_levels.to_le_bytes());
    for v in &vm.host_fred_ssp {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&vm.guest_cr0.to_le_bytes());
    buf.extend_from_slice(&vm.guest_cr3.to_le_bytes());
    buf.extend_from_slice(&vm.guest_cr4.to_le_bytes());
    buf.extend_from_slice(&vm.guest_rsp.to_le_bytes());
    buf.extend_from_slice(&vm.guest_rip.to_le_bytes());
    buf.extend_from_slice(&vm.guest_rflags.to_le_bytes());
    buf.extend_from_slice(&vm.guest_dr7.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ia32_efer.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ia32_pat.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ia32_sysenter_cs.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ia32_sysenter_esp.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ia32_sysenter_eip.to_le_bytes());
    buf.extend_from_slice(&vm.guest_cs_selector.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ss_selector.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ds_selector.to_le_bytes());
    buf.extend_from_slice(&vm.guest_es_selector.to_le_bytes());
    buf.extend_from_slice(&vm.guest_fs_selector.to_le_bytes());
    buf.extend_from_slice(&vm.guest_gs_selector.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ldtr_selector.to_le_bytes());
    buf.extend_from_slice(&vm.guest_tr_selector.to_le_bytes());
    buf.extend_from_slice(&vm.guest_cs_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ss_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ds_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_es_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_fs_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_gs_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ldtr_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_tr_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_gdtr_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_idtr_base.to_le_bytes());
    buf.extend_from_slice(&vm.guest_cs_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ss_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ds_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_es_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_fs_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_gs_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ldtr_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_tr_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_gdtr_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_idtr_limit.to_le_bytes());
    buf.extend_from_slice(&vm.guest_cs_ar.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ss_ar.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ds_ar.to_le_bytes());
    buf.extend_from_slice(&vm.guest_es_ar.to_le_bytes());
    buf.extend_from_slice(&vm.guest_fs_ar.to_le_bytes());
    buf.extend_from_slice(&vm.guest_gs_ar.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ldtr_ar.to_le_bytes());
    buf.extend_from_slice(&vm.guest_tr_ar.to_le_bytes());
    buf.extend_from_slice(&vm.guest_activity_state.to_le_bytes());
    buf.extend_from_slice(&vm.guest_interruptibility_state.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ia32_s_cet.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ssp.to_le_bytes());
    buf.extend_from_slice(&vm.guest_interrupt_ssp_table_addr.to_le_bytes());
    buf.extend_from_slice(&vm.guest_fred_config.to_le_bytes());
    for v in &vm.guest_fred_rsp {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&vm.guest_fred_stack_levels.to_le_bytes());
    for v in &vm.guest_fred_ssp {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&vm.guest_pkrs.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ia32_spec_ctrl.to_le_bytes());
    buf.extend_from_slice(&vm.guest_pending_dbg_exceptions.to_le_bytes());
    buf.extend_from_slice(&vm.guest_ia32_debugctl.to_le_bytes());
    buf.extend_from_slice(&vm.guest_smbase.to_le_bytes());
    buf.extend_from_slice(&vm.vmfunc_ctrls.to_le_bytes());
    buf.extend_from_slice(&vm.eptp_list_address.to_le_bytes());
    buf.extend_from_slice(&vm.vm_instruction_error.to_le_bytes());
    buf.extend_from_slice(&vm.exit_reason.to_le_bytes());
    buf.extend_from_slice(&vm.exit_qualification.to_le_bytes());
    buf.extend_from_slice(&vm.exit_intr_info.to_le_bytes());
    buf.extend_from_slice(&vm.exit_intr_error_code.to_le_bytes());
    buf.extend_from_slice(&vm.exit_instruction_length.to_le_bytes());
    buf.extend_from_slice(&vm.exit_instruction_info.to_le_bytes());
    buf.extend_from_slice(&vm.idt_vectoring_info.to_le_bytes());
    buf.extend_from_slice(&vm.idt_vectoring_error_code.to_le_bytes());
    buf.extend_from_slice(&vm.guest_linear_addr.to_le_bytes());
    buf.extend_from_slice(&vm.pin_based_ctls.to_le_bytes());
    buf.extend_from_slice(&vm.proc_based_ctls.to_le_bytes());
    buf.extend_from_slice(&vm.secondary_proc_based_ctls.to_le_bytes());
    buf.extend_from_slice(&vm.vm_exit_ctls.to_le_bytes());
    buf.extend_from_slice(&vm.vm_exit_ctls2.to_le_bytes());
    buf.extend_from_slice(&vm.vm_entry_ctls.to_le_bytes());
    buf.extend_from_slice(&vm.vm_entry_intr_info.to_le_bytes());
    buf.extend_from_slice(&vm.vm_entry_exception_error_code.to_le_bytes());
    buf.extend_from_slice(&vm.vm_entry_instruction_length.to_le_bytes());
    buf.extend_from_slice(&vm.exception_bitmap.to_le_bytes());
    buf.extend_from_slice(&vm.vm_pf_mask.to_le_bytes());
    buf.extend_from_slice(&vm.vm_pf_match.to_le_bytes());
    buf.extend_from_slice(&vm.vm_cr3_target_cnt.to_le_bytes());
    for v in &vm.vm_cr3_target_value {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&vm.cr0_guest_host_mask.to_le_bytes());
    buf.extend_from_slice(&vm.cr4_guest_host_mask.to_le_bytes());
    buf.extend_from_slice(&vm.cr0_read_shadow.to_le_bytes());
    buf.extend_from_slice(&vm.cr4_read_shadow.to_le_bytes());
    buf.extend_from_slice(&vm.vmcs_link_pointer.to_le_bytes());
    buf.extend_from_slice(&vm.tsc_offset.to_le_bytes());
    buf.extend_from_slice(&vm.msr_bitmap_addr.to_le_bytes());
    for v in &vm.io_bitmap_addr {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&vm.vmread_bitmap_addr.to_le_bytes());
    buf.extend_from_slice(&vm.vmwrite_bitmap_addr.to_le_bytes());
    buf.extend_from_slice(&vm.vmx_preemption_timer_value.to_le_bytes());
    buf.extend_from_slice(&vm.tpr_threshold.to_le_bytes());
    buf.extend_from_slice(&vm.virtual_apic_page_addr.to_le_bytes());
    buf.extend_from_slice(&vm.pi_desc_addr.to_le_bytes());
    buf.push(vm.pi_notification_vector);
    buf.extend_from_slice(&vm.vpid.to_le_bytes());
    buf.extend_from_slice(&vm.eptptr.to_le_bytes());
    buf.extend_from_slice(&vm.guest_physical_addr.to_le_bytes());
    buf.extend_from_slice(&vm.vmentry_msr_load_addr.to_le_bytes());
    buf.extend_from_slice(&vm.vmexit_msr_store_addr.to_le_bytes());
    buf.extend_from_slice(&vm.vmexit_msr_load_addr.to_le_bytes());
    buf.extend_from_slice(&vm.vmentry_msr_load_cnt.to_le_bytes());
    buf.extend_from_slice(&vm.vmexit_msr_store_cnt.to_le_bytes());
    buf.extend_from_slice(&vm.vmexit_msr_load_cnt.to_le_bytes());
    buf.push(vm.shadow_stack_prematurely_busy as u8);
}

fn restore_vmcs_cache(vm: &mut super::vmx::BxVmcs, d: &[u8], off: &mut usize) {
    vm.launched = d[*off] != 0;
    *off += 1;
    vm.host_cr0 = u64_at(d, off);
    vm.host_cr3 = u64_at(d, off);
    vm.host_cr4 = u64_at(d, off);
    vm.host_rsp = u64_at(d, off);
    vm.host_rip = u64_at(d, off);
    vm.host_cs_selector = u16_at(d, off);
    vm.host_ss_selector = u16_at(d, off);
    vm.host_ds_selector = u16_at(d, off);
    vm.host_es_selector = u16_at(d, off);
    vm.host_fs_selector = u16_at(d, off);
    vm.host_gs_selector = u16_at(d, off);
    vm.host_tr_selector = u16_at(d, off);
    vm.host_fs_base = u64_at(d, off);
    vm.host_gs_base = u64_at(d, off);
    vm.host_tr_base = u64_at(d, off);
    vm.host_gdtr_base = u64_at(d, off);
    vm.host_idtr_base = u64_at(d, off);
    vm.host_ia32_efer = u64_at(d, off);
    vm.host_ia32_pat = u64_at(d, off);
    vm.host_sysenter_cs = u32_at(d, off);
    vm.host_sysenter_esp = u64_at(d, off);
    vm.host_sysenter_eip = u64_at(d, off);
    vm.host_perf_global_ctrl = u64_at(d, off);
    vm.host_pkrs = u64_at(d, off);
    vm.host_ia32_spec_ctrl = u64_at(d, off);
    vm.host_ia32_s_cet = u64_at(d, off);
    vm.host_ssp = u64_at(d, off);
    vm.host_interrupt_ssp_table_addr = u64_at(d, off);
    vm.host_fred_config = u64_at(d, off);
    for v in vm.host_fred_rsp.iter_mut() {
        *v = u64_at(d, off);
    }
    vm.host_fred_stack_levels = u64_at(d, off);
    for v in vm.host_fred_ssp.iter_mut() {
        *v = u64_at(d, off);
    }
    vm.guest_cr0 = u64_at(d, off);
    vm.guest_cr3 = u64_at(d, off);
    vm.guest_cr4 = u64_at(d, off);
    vm.guest_rsp = u64_at(d, off);
    vm.guest_rip = u64_at(d, off);
    vm.guest_rflags = u64_at(d, off);
    vm.guest_dr7 = u64_at(d, off);
    vm.guest_ia32_efer = u64_at(d, off);
    vm.guest_ia32_pat = u64_at(d, off);
    vm.guest_ia32_sysenter_cs = u32_at(d, off);
    vm.guest_ia32_sysenter_esp = u64_at(d, off);
    vm.guest_ia32_sysenter_eip = u64_at(d, off);
    vm.guest_cs_selector = u16_at(d, off);
    vm.guest_ss_selector = u16_at(d, off);
    vm.guest_ds_selector = u16_at(d, off);
    vm.guest_es_selector = u16_at(d, off);
    vm.guest_fs_selector = u16_at(d, off);
    vm.guest_gs_selector = u16_at(d, off);
    vm.guest_ldtr_selector = u16_at(d, off);
    vm.guest_tr_selector = u16_at(d, off);
    vm.guest_cs_base = u64_at(d, off);
    vm.guest_ss_base = u64_at(d, off);
    vm.guest_ds_base = u64_at(d, off);
    vm.guest_es_base = u64_at(d, off);
    vm.guest_fs_base = u64_at(d, off);
    vm.guest_gs_base = u64_at(d, off);
    vm.guest_ldtr_base = u64_at(d, off);
    vm.guest_tr_base = u64_at(d, off);
    vm.guest_gdtr_base = u64_at(d, off);
    vm.guest_idtr_base = u64_at(d, off);
    vm.guest_cs_limit = u32_at(d, off);
    vm.guest_ss_limit = u32_at(d, off);
    vm.guest_ds_limit = u32_at(d, off);
    vm.guest_es_limit = u32_at(d, off);
    vm.guest_fs_limit = u32_at(d, off);
    vm.guest_gs_limit = u32_at(d, off);
    vm.guest_ldtr_limit = u32_at(d, off);
    vm.guest_tr_limit = u32_at(d, off);
    vm.guest_gdtr_limit = u32_at(d, off);
    vm.guest_idtr_limit = u32_at(d, off);
    vm.guest_cs_ar = u32_at(d, off);
    vm.guest_ss_ar = u32_at(d, off);
    vm.guest_ds_ar = u32_at(d, off);
    vm.guest_es_ar = u32_at(d, off);
    vm.guest_fs_ar = u32_at(d, off);
    vm.guest_gs_ar = u32_at(d, off);
    vm.guest_ldtr_ar = u32_at(d, off);
    vm.guest_tr_ar = u32_at(d, off);
    vm.guest_activity_state = u32_at(d, off);
    vm.guest_interruptibility_state = u32_at(d, off);
    vm.guest_ia32_s_cet = u64_at(d, off);
    vm.guest_ssp = u64_at(d, off);
    vm.guest_interrupt_ssp_table_addr = u64_at(d, off);
    vm.guest_fred_config = u64_at(d, off);
    for v in vm.guest_fred_rsp.iter_mut() {
        *v = u64_at(d, off);
    }
    vm.guest_fred_stack_levels = u64_at(d, off);
    for v in vm.guest_fred_ssp.iter_mut() {
        *v = u64_at(d, off);
    }
    vm.guest_pkrs = u64_at(d, off);
    vm.guest_ia32_spec_ctrl = u64_at(d, off);
    vm.guest_pending_dbg_exceptions = u64_at(d, off);
    vm.guest_ia32_debugctl = u64_at(d, off);
    vm.guest_smbase = u32_at(d, off);
    vm.vmfunc_ctrls = u64_at(d, off);
    vm.eptp_list_address = u64_at(d, off);
    vm.vm_instruction_error = u32_at(d, off);
    vm.exit_reason = u32_at(d, off);
    vm.exit_qualification = u64_at(d, off);
    vm.exit_intr_info = u32_at(d, off);
    vm.exit_intr_error_code = u32_at(d, off);
    vm.exit_instruction_length = u32_at(d, off);
    vm.exit_instruction_info = u32_at(d, off);
    vm.idt_vectoring_info = u32_at(d, off);
    vm.idt_vectoring_error_code = u32_at(d, off);
    vm.guest_linear_addr = u64_at(d, off);
    vm.pin_based_ctls = u32_at(d, off);
    vm.proc_based_ctls = u32_at(d, off);
    vm.secondary_proc_based_ctls = u32_at(d, off);
    vm.vm_exit_ctls = u32_at(d, off);
    vm.vm_exit_ctls2 = u64_at(d, off);
    vm.vm_entry_ctls = u32_at(d, off);
    vm.vm_entry_intr_info = u32_at(d, off);
    vm.vm_entry_exception_error_code = u32_at(d, off);
    vm.vm_entry_instruction_length = u32_at(d, off);
    vm.exception_bitmap = u32_at(d, off);
    vm.vm_pf_mask = u32_at(d, off);
    vm.vm_pf_match = u32_at(d, off);
    vm.vm_cr3_target_cnt = u32_at(d, off);
    for v in vm.vm_cr3_target_value.iter_mut() {
        *v = u64_at(d, off);
    }
    vm.cr0_guest_host_mask = u64_at(d, off);
    vm.cr4_guest_host_mask = u64_at(d, off);
    vm.cr0_read_shadow = u64_at(d, off);
    vm.cr4_read_shadow = u64_at(d, off);
    vm.vmcs_link_pointer = u64_at(d, off);
    vm.tsc_offset = u64_at(d, off);
    vm.msr_bitmap_addr = u64_at(d, off);
    for v in vm.io_bitmap_addr.iter_mut() {
        *v = u64_at(d, off);
    }
    vm.vmread_bitmap_addr = u64_at(d, off);
    vm.vmwrite_bitmap_addr = u64_at(d, off);
    vm.vmx_preemption_timer_value = u32_at(d, off);
    vm.tpr_threshold = u32_at(d, off);
    vm.virtual_apic_page_addr = u64_at(d, off);
    vm.pi_desc_addr = u64_at(d, off);
    vm.pi_notification_vector = d[*off];
    *off += 1;
    vm.vpid = u16_at(d, off);
    vm.eptptr = u64_at(d, off);
    vm.guest_physical_addr = u64_at(d, off);
    vm.vmentry_msr_load_addr = u64_at(d, off);
    vm.vmexit_msr_store_addr = u64_at(d, off);
    vm.vmexit_msr_load_addr = u64_at(d, off);
    vm.vmentry_msr_load_cnt = u32_at(d, off);
    vm.vmexit_msr_store_cnt = u32_at(d, off);
    vm.vmexit_msr_load_cnt = u32_at(d, off);
    vm.shadow_stack_prematurely_busy = d[*off] != 0;
    *off += 1;
}

/// Save the SVM VMCB cache (controls + host state). Field order matches the
/// `SvmControls` / `SvmHostState` struct definitions.
fn save_vmcb_cache(vmcb: &super::svm::VmcbCache, buf: &mut alloc::vec::Vec<u8>) {
    for seg in &vmcb.host_state.sregs {
        write_seg_reg(buf, seg);
    }
    write_global_seg(buf, &vmcb.host_state.gdtr);
    write_global_seg(buf, &vmcb.host_state.idtr);
    buf.extend_from_slice(&vmcb.host_state.efer.bits().to_le_bytes());
    buf.extend_from_slice(&vmcb.host_state.cr0.bits().to_le_bytes());
    buf.extend_from_slice(&vmcb.host_state.cr4.bits().to_le_bytes());
    buf.extend_from_slice(&vmcb.host_state.cr3.to_le_bytes());
    buf.extend_from_slice(&vmcb.host_state.eflags.to_le_bytes());
    buf.extend_from_slice(&vmcb.host_state.rip.to_le_bytes());
    buf.extend_from_slice(&vmcb.host_state.rsp.to_le_bytes());
    buf.extend_from_slice(&vmcb.host_state.rax.to_le_bytes());
    buf.extend_from_slice(&vmcb.host_state.pat_msr.U64().to_le_bytes());

    buf.extend_from_slice(&vmcb.ctrls.cr_rd_ctrl.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.cr_wr_ctrl.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.dr_rd_ctrl.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.dr_wr_ctrl.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.exceptions_intercept.to_le_bytes());
    for v in &vmcb.ctrls.intercept_vector {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&vmcb.ctrls.exitintinfo.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.exitintinfo_error_code.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.eventinj.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.iopm_base.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.msrpm_base.to_le_bytes());
    buf.push(vmcb.ctrls.v_tpr);
    buf.push(vmcb.ctrls.v_intr_prio);
    buf.push(vmcb.ctrls.v_ignore_tpr as u8);
    buf.push(vmcb.ctrls.v_intr_masking as u8);
    buf.push(vmcb.ctrls.v_intr_vector);
    buf.push(vmcb.ctrls.nested_paging as u8);
    buf.extend_from_slice(&vmcb.ctrls.ncr3.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.pause_filter_count.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.pause_filter_threshold.to_le_bytes());
    buf.extend_from_slice(&vmcb.ctrls.last_pause_time.to_le_bytes());
}

fn restore_vmcb_cache(vmcb: &mut super::svm::VmcbCache, d: &[u8], off: &mut usize) {
    for seg in vmcb.host_state.sregs.iter_mut() {
        read_seg_reg(d, off, seg);
    }
    read_global_seg(d, off, &mut vmcb.host_state.gdtr);
    read_global_seg(d, off, &mut vmcb.host_state.idtr);
    vmcb.host_state.efer = super::crregs::BxEfer::from_bits_retain(u32_at(d, off));
    vmcb.host_state.cr0 = BxCr0::from_bits_retain(u32_at(d, off));
    vmcb.host_state.cr4 = BxCr4::from_bits_retain(u64_at(d, off));
    vmcb.host_state.cr3 = u64_at(d, off);
    vmcb.host_state.eflags = u32_at(d, off);
    vmcb.host_state.rip = u64_at(d, off);
    vmcb.host_state.rsp = u64_at(d, off);
    vmcb.host_state.rax = u64_at(d, off);
    vmcb.host_state.pat_msr.set_U64(u64_at(d, off));

    vmcb.ctrls.cr_rd_ctrl = u16_at(d, off);
    vmcb.ctrls.cr_wr_ctrl = u16_at(d, off);
    vmcb.ctrls.dr_rd_ctrl = u16_at(d, off);
    vmcb.ctrls.dr_wr_ctrl = u16_at(d, off);
    vmcb.ctrls.exceptions_intercept = u32_at(d, off);
    for v in vmcb.ctrls.intercept_vector.iter_mut() {
        *v = u32_at(d, off);
    }
    vmcb.ctrls.exitintinfo = u32_at(d, off);
    vmcb.ctrls.exitintinfo_error_code = u32_at(d, off);
    vmcb.ctrls.eventinj = u32_at(d, off);
    vmcb.ctrls.iopm_base = u64_at(d, off);
    vmcb.ctrls.msrpm_base = u64_at(d, off);
    vmcb.ctrls.v_tpr = d[*off];
    *off += 1;
    vmcb.ctrls.v_intr_prio = d[*off];
    *off += 1;
    vmcb.ctrls.v_ignore_tpr = d[*off] != 0;
    *off += 1;
    vmcb.ctrls.v_intr_masking = d[*off] != 0;
    *off += 1;
    vmcb.ctrls.v_intr_vector = d[*off];
    *off += 1;
    vmcb.ctrls.nested_paging = d[*off] != 0;
    *off += 1;
    vmcb.ctrls.ncr3 = u64_at(d, off);
    vmcb.ctrls.pause_filter_count = u16_at(d, off);
    vmcb.ctrls.pause_filter_threshold = u16_at(d, off);
    vmcb.ctrls.last_pause_time = u64_at(d, off);
}

#[cfg(test)]
mod tests {
    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::core_i7_skylake::Corei7SkylakeX;
    use crate::cpu::svm::VmcbCache;
    use crate::cpu::ResetReason;

    /// Round-trip the virtualization (VMX/SVM) state through a CPU snapshot.
    /// The tail assertions (VMCB pause fields) double as a lockstep check:
    /// any save/restore misalignment earlier in the blob corrupts them.
    #[test]
    fn snapshot_round_trips_virtualization_state() {
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.reset(ResetReason::Hardware);

        cpu.in_vmx = true;
        cpu.in_vmx_guest = true;
        cpu.in_smm_vmx = true;
        cpu.in_smm_vmx_guest = true;
        cpu.in_svm_guest = true;
        cpu.svm_gif = false;
        cpu.vmcsptr = 0x0012_3000;
        cpu.vmxonptr = 0x0056_7000;
        cpu.vmcbptr = 0x009A_B000;
        cpu.vmcbhostptr = 0xDEAD_BEEF; // must NOT survive a restore
        cpu.msr.svm_hsave_pa = 0x00DE_F000;
        cpu.msr.svm_vm_cr = 0x2;

        cpu.vmcs.launched = true;
        cpu.vmcs.host_cr0 = 0x8000_0031; // first serialized u64 field
        cpu.vmcs.guest_cs_selector = 0x1234;
        cpu.vmcs.exit_reason = 0x77;
        cpu.vmcs.exit_qualification = 0xABCD_EF01;

        let mut vmcb = VmcbCache::default();
        vmcb.ctrls.intercept_vector[1] = 0xAA55;
        vmcb.ctrls.ncr3 = 0x0004_2000;
        vmcb.ctrls.v_tpr = 0x0F;
        vmcb.ctrls.nested_paging = true;
        vmcb.ctrls.pause_filter_count = 0x1111;
        vmcb.ctrls.pause_filter_threshold = 0x2222;
        vmcb.ctrls.last_pause_time = 0x3333_4444_5555_6666;
        vmcb.host_state.rax = 0x7777_8888;
        vmcb.host_state.eflags = 0x0000_0202;
        cpu.vmcb = Some(vmcb);

        let blob = cpu.save_snapshot_state();

        let mut restored = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        restored.reset(ResetReason::Hardware);
        restored.restore_snapshot_state(&blob);

        assert!(restored.in_vmx);
        assert!(restored.in_vmx_guest);
        assert!(restored.in_smm_vmx);
        assert!(restored.in_smm_vmx_guest);
        assert!(restored.in_svm_guest);
        assert!(!restored.svm_gif);
        assert_eq!(restored.vmcsptr, 0x0012_3000);
        assert_eq!(restored.vmxonptr, 0x0056_7000);
        assert_eq!(restored.vmcbptr, 0x009A_B000);
        assert_eq!(
            restored.vmcbhostptr, 0,
            "cached host pointer must be re-resolved after restore"
        );
        assert_eq!(restored.msr.svm_hsave_pa, 0x00DE_F000);
        assert_eq!(restored.msr.svm_vm_cr, 0x2);

        assert!(restored.vmcs.launched);
        assert_eq!(restored.vmcs.host_cr0, 0x8000_0031);
        assert_eq!(restored.vmcs.guest_cs_selector, 0x1234);
        assert_eq!(restored.vmcs.exit_reason, 0x77);
        assert_eq!(restored.vmcs.exit_qualification, 0xABCD_EF01);

        let vmcb = restored.vmcb.as_ref().expect("VMCB cache must round-trip");
        assert_eq!(vmcb.ctrls.intercept_vector[1], 0xAA55);
        assert_eq!(vmcb.ctrls.ncr3, 0x0004_2000);
        assert_eq!(vmcb.ctrls.v_tpr, 0x0F);
        assert!(vmcb.ctrls.nested_paging);
        assert_eq!(vmcb.host_state.rax, 0x7777_8888);
        assert_eq!(vmcb.host_state.eflags, 0x0000_0202);
        assert_eq!(vmcb.ctrls.pause_filter_count, 0x1111);
        assert_eq!(vmcb.ctrls.pause_filter_threshold, 0x2222);
        assert_eq!(vmcb.ctrls.last_pause_time, 0x3333_4444_5555_6666);
    }

    /// A snapshot taken with no VMCB present must restore to no VMCB.
    #[test]
    fn snapshot_round_trips_absent_vmcb() {
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.reset(ResetReason::Hardware);
        assert!(cpu.vmcb.is_none());

        let blob = cpu.save_snapshot_state();

        let mut restored = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        restored.reset(ResetReason::Hardware);
        restored.vmcb = Some(VmcbCache::default());
        restored.restore_snapshot_state(&blob);

        assert!(restored.vmcb.is_none());
    }
}
