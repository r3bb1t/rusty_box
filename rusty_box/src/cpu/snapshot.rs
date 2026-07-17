//! CPU state save/restore for the snapshot mechanism.
//! This file lives in cpu/ so it has pub(super) access to BxCpuC fields.

use std::io::{self, Error, ErrorKind, Read, Write};

use crate::snapshot::{
    bounds, checked_snapshot_len_add, checked_snapshot_len_mul, SnapshotReader, SnapshotWriteExt,
};

use super::{
    cpu::{BxCpuC, CpuActivityState, CpuMode, BX_MSR_MAX_INDEX},
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
// V3 streaming CPU record
// ============================================================================

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Exact byte count for one CPU body in the v3 CPU section. The enclosing
    /// CPU section owns its version and per-record `{ cpu_id, state_len }`
    /// framing, so this method deliberately has neither.
    pub(crate) fn snapshot_v3_body_len(&self) -> io::Result<u64> {
        let vmm_count = u64::try_from(self.vmm.len())
            .map_err(|_| snapshot_invalid("vector register count does not fit u64"))?;
        let generic_msr_count = u64::try_from(self.msrs.len())
            .map_err(|_| snapshot_invalid("generic MSR count does not fit u64"))?;
        checked_snapshot_len_mul(vmm_count, 64)?;
        checked_snapshot_len_mul(generic_msr_count, 40)?;

        let mut counter = SnapshotLenWriter::default();
        self.save_snapshot_v3_body(&mut counter)?;
        checked_snapshot_len_add(0, counter.len)
    }

    /// Returns the restored A20 gate mask for the machine-wide cross-check
    /// against the PC system's authoritative gate state.
    #[cfg(feature = "std")]
    pub(crate) const fn snapshot_a20_mask(&self) -> u64 {
        self.a20_mask
    }

    /// Rejects a CPU/APIC pairing whose architectural APIC-base MSR does not
    /// describe the restored LAPIC MMIO base and mode bits.
    #[cfg(feature = "std")]
    pub(crate) fn validate_snapshot_lapic_binding(
        &self,
        lapic_base: u64,
        lapic_mode: u64,
    ) -> io::Result<()> {
        if self.msr.apicbase & !0xfff != lapic_base
            || (self.msr.apicbase >> 10) & 3 != lapic_mode
        {
            return Err(snapshot_invalid(
                "CPU APIC-base MSR disagrees with restored LAPIC",
            ));
        }
        Ok(())
    }

    /// Streams one architectural CPU record. Host mappings, TLBs, decode
    /// caches, instruction handlers, instrumentation, and diagnostics stay
    /// live and are rebuilt by the machine-level post-restore phase.
    pub(crate) fn save_snapshot_v3_body<W: Write + ?Sized>(
        &self,
        writer: &mut W,
    ) -> io::Result<()> {
        if self.vmm.len() > bounds::MAX_SNAPSHOT_COUNT
            || self.msrs.len() > bounds::MAX_SNAPSHOT_COUNT
        {
            return Err(snapshot_invalid("CPU fixed state exceeds snapshot bound"));
        }
        for extension in &self.ia_extensions_bitmask {
            writer.write_u32(*extension)?;
        }
        match self.vmx_extensions_bitmask.as_ref() {
            Some(extensions) => {
                writer.write_bool(true)?;
                writer.write_u32(extensions.bits())?;
            }
            None => writer.write_bool(false)?,
        }
        match self.svm_extensions_bitmask.as_ref() {
            Some(extensions) => {
                writer.write_bool(true)?;
                writer.write_u32(extensions.bits())?;
            }
            None => writer.write_bool(false)?,
        }
        writer.write_u8(self.smp_trace_quantum)?;


        for reg in &self.gen_reg {
            writer.write_u64(reg.rrx())?;
        }

        writer.write_u32(self.eflags_materialized())?;
        writer.write_u64(self.icount)?;
        writer.write_u64(self.icount_last_sync)?;
        writer.write_u64(self.prev_rip)?;
        writer.write_u64(self.prev_rsp)?;
        writer.write_u64(self.prev_ssp)?;
        writer.write_bool(self.speculative_rsp)?;
        writer.write_u32(self.inhibit_mask)?;
        writer.write_u64(self.inhibit_icount)?;
        writer.write_u64(self.oszapc.result)?;
        writer.write_u64(self.oszapc.auxbits)?;

        for seg in &self.sregs {
            write_v3_seg_reg(writer, seg)?;
        }
        write_v3_global_seg(writer, &self.gdtr)?;
        write_v3_global_seg(writer, &self.idtr)?;
        write_v3_seg_reg(writer, &self.ldtr)?;
        write_v3_seg_reg(writer, &self.tr)?;

        writer.write_u32(self.cr0.bits())?;
        writer.write_u64(self.cr2)?;
        writer.write_u64(self.cr3)?;
        writer.write_u64(self.cr4.bits())?;
        writer.write_u64(self.cr4_suppmask)?;
        writer.write_u32(self.efer.bits())?;
        writer.write_u32(self.efer_suppmask)?;
        for entry in &self.pdptrcache.entry {
            writer.write_u64(*entry)?;
        }
        for reg in &self.dr {
            writer.write_u64(*reg)?;
        }
        writer.write_u32(self.dr6.bits())?;
        writer.write_u32(self.dr7.bits())?;
        writer.write_u32(self.debug_trap)?;

        writer.write_u32(self.xcr0.value)?;
        writer.write_u32(self.xcr0_suppmask)?;
        writer.write_u32(self.ia32_xss_suppmask)?;
        writer.write_u32(self.pkru)?;
        writer.write_u32(self.pkrs)?;
        writer.write_u8(self.linaddr_width)?;
        writer.write_i64(self.tsc_adjust)?;
        writer.write_i64(self.tsc_offset)?;

        writer.write_u16(self.the_i387.cwd)?;
        writer.write_u16(self.the_i387.swd)?;
        writer.write_u16(self.the_i387.twd)?;
        writer.write_u16(self.the_i387.foo)?;
        writer.write_u64(self.the_i387.fip)?;
        writer.write_u64(self.the_i387.fdp)?;
        writer.write_u16(self.the_i387.fcs)?;
        writer.write_u16(self.the_i387.fds)?;
        for st in &self.the_i387.st_space {
            writer.write_u64(st.signif)?;
            writer.write_u16(st.sign_exp)?;
        }

        for vmm in &self.vmm {
            writer.write_bytes(vmm.raw())?;
        }
        writer.write_u32(self.mxcsr.mxcsr)?;
        writer.write_u32(self.mxcsr_mask)?;
        for mask in &self.opmask {
            writer.write_u64(mask.rrx())?;
        }

        writer.write_u64(self.monitor.monitor_addr)?;
        writer.write_u32(self.monitor.armed_by)?;

        writer.write_u64(self.uintr.ui_handler)?;
        writer.write_u64(self.uintr.stack_adjust)?;
        writer.write_u32(self.uintr.uinv)?;
        writer.write_u32(self.uintr.uitt_size)?;
        writer.write_u64(self.uintr.uitt_addr)?;
        writer.write_u64(self.uintr.upid_addr)?;
        writer.write_u64(self.uintr.uirr)?;
        writer.write_bool(self.uintr.uif)?;

        save_v3_fixed_msrs(writer, &self.msr)?;
        for msr in &self.msrs {
            writer.write_u32(msr.index)?;
            writer.write_u32(msr.r#type)?;
            writer.write_u64(msr.val64)?;
            writer.write_u64(msr.reset_value)?;
            writer.write_u64(msr.reserved)?;
            writer.write_u64(msr.ignored)?;
        }

        write_v3_amx(writer, self.amx.as_deref())?;

        writer.write_u32(cpu_mode_to_wire(self.cpu_mode))?;
        writer.write_u32(activity_state_to_wire(self.activity_state))?;
        writer.write_u32(self.pending_event)?;
        writer.write_u32(self.event_mask)?;
        writer.write_u32(self.async_event)?;
        writer.write_bool(self.user_pl)?;
        writer.write_bool(self.in_smm)?;
        writer.write_bool(self.in_event)?;
        writer.write_bool(self.ext)?;
        writer.write_bool(self.nmi_unblocking_iret)?;
        write_i32(writer, self.last_exception_type)?;
        writer.write_u32(self.smbase)?;
        writer.write_u32(self.alignment_check_mask)?;
        writer.write_u64(self.a20_mask)?;
        writer.write_u32(self.fred_event_info)?;
        writer.write_u64(self.fred_event_data)?;

        writer.write_bool(self.in_vmx)?;
        writer.write_bool(self.in_vmx_guest)?;
        writer.write_bool(self.in_smm_vmx)?;
        writer.write_bool(self.in_smm_vmx_guest)?;
        writer.write_u64(self.vmcsptr)?;
        writer.write_u32(self.vmcs_memtype)?;
        writer.write_u64(self.vmxonptr)?;
        save_v3_vmcs_cache(writer, &self.vmcs)?;

        writer.write_bool(self.in_svm_guest)?;
        writer.write_bool(self.svm_gif)?;
        writer.write_u64(self.vmcbptr)?;
        writer.write_u32(self.vmcb_memtype)?;
        match &self.vmcb {
            Some(vmcb) => {
                writer.write_bool(true)?;
                save_v3_vmcb_cache(writer, vmcb)?;
            }
            None => writer.write_bool(false)?,
        }

        Ok(())
    }

    /// Restores one bounded CPU body after the container has already validated
    /// the record's `cpu_id` and selected this configured CPU. It intentionally
    /// does not invalidate caches or wire handlers; those machine-wide hooks
    /// run only after every section succeeds. It does rebuild CPU-local
    /// derived state from the decoded architectural inputs.
    pub(crate) fn restore_snapshot_v3_body<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
        expected_cpu_id: u32,
    ) -> io::Result<()> {
        if self.bx_cpuid != expected_cpu_id {
            return Err(snapshot_invalid("CPU record does not match configured CPU ID"));
        }
        if self.vmm.len() > bounds::MAX_SNAPSHOT_COUNT
            || self.msrs.len() != BX_MSR_MAX_INDEX
            || self.msrs.len() > bounds::MAX_SNAPSHOT_COUNT
        {
            return Err(snapshot_invalid("configured CPU fixed state is incompatible"));
        }

        for extension in &self.ia_extensions_bitmask {
            if reader.read_u32()? != *extension {
                return Err(snapshot_invalid("CPU ISA capability set differs from snapshot"));
            }
        }
        let saved_vmx_extensions = if reader.read_bool()? {
            Some(reader.read_u32()?)
        } else {
            None
        };
        if saved_vmx_extensions != self.vmx_extensions_bitmask.as_ref().map(|bits| bits.bits()) {
            return Err(snapshot_invalid("VMX capability set differs from snapshot"));
        }
        let saved_svm_extensions = if reader.read_bool()? {
            Some(reader.read_u32()?)
        } else {
            None
        };
        if saved_svm_extensions != self.svm_extensions_bitmask.as_ref().map(|bits| bits.bits()) {
            return Err(snapshot_invalid("SVM capability set differs from snapshot"));
        }
        let smp_trace_quantum = reader.read_u8()?;
        if smp_trace_quantum > 32 || smp_trace_quantum != self.smp_trace_quantum {
            return Err(snapshot_invalid("SMP trace quantum differs from configured CPU"));
        }
        for reg in &mut self.gen_reg {
            reg.set_rrx(reader.read_u64()?);
        }

        let eflags = EFlags::from_bits(reader.read_u32()?)
            .ok_or_else(|| snapshot_invalid("EFLAGS contains reserved bits"))?;
        if !eflags.contains(EFlags::R1) {
            return Err(snapshot_invalid("EFLAGS required bit is clear"));
        }
        self.eflags = eflags;
        self.icount = reader.read_u64()?;
        self.icount_last_sync = reader.read_u64()?;
        self.prev_rip = reader.read_u64()?;
        self.prev_rsp = reader.read_u64()?;
        self.prev_ssp = reader.read_u64()?;
        self.speculative_rsp = reader.read_bool()?;
        self.inhibit_mask = reader.read_u32()?;
        self.inhibit_icount = reader.read_u64()?;
        self.oszapc.result = reader.read_u64()?;
        self.oszapc.auxbits = reader.read_u64()?;

        for seg in &mut self.sregs {
            read_v3_seg_reg(reader, seg)?;
        }
        read_v3_global_seg(reader, &mut self.gdtr)?;
        read_v3_global_seg(reader, &mut self.idtr)?;
        read_v3_seg_reg(reader, &mut self.ldtr)?;
        read_v3_seg_reg(reader, &mut self.tr)?;

        let cr0 = BxCr0::from_bits(reader.read_u32()?)
            .ok_or_else(|| snapshot_invalid("CR0 contains reserved bits"))?;
        self.cr2 = reader.read_u64()?;
        self.cr3 = reader.read_u64()?;
        let cr4 = BxCr4::from_bits(reader.read_u64()?)
            .ok_or_else(|| snapshot_invalid("CR4 contains reserved bits"))?;
        let cr4_suppmask = reader.read_u64()?;
        if cr4_suppmask != self.cr4_suppmask || (cr4.bits() & !cr4_suppmask) != 0 {
            return Err(snapshot_invalid("CR4 state is incompatible with CPU capabilities"));
        }
        let efer = BxEfer::from_bits(reader.read_u32()?)
            .ok_or_else(|| snapshot_invalid("EFER contains reserved bits"))?;
        let efer_suppmask = reader.read_u32()?;
        if efer_suppmask != self.efer_suppmask || (efer.bits() & !efer_suppmask) != 0 {
            return Err(snapshot_invalid("EFER state is incompatible with CPU capabilities"));
        }
        for entry in &mut self.pdptrcache.entry {
            *entry = reader.read_u64()?;
        }
        self.cr0 = cr0;
        self.cr4 = cr4;
        self.efer = efer;
        for reg in &mut self.dr {
            *reg = reader.read_u64()?;
        }
        self.dr6 = BxDr6::from_bits_retain(reader.read_u32()?);
        self.dr7 = BxDr7::from_bits_retain(reader.read_u32()?);
        self.debug_trap = reader.read_u32()?;

        let xcr0 = reader.read_u32()?;
        let xcr0_suppmask = reader.read_u32()?;
        let ia32_xss_suppmask = reader.read_u32()?;
        if xcr0_suppmask != self.xcr0_suppmask
            || ia32_xss_suppmask != self.ia32_xss_suppmask
            || xcr0 & !xcr0_suppmask != 0
            || xcr0 & 1 == 0
        {
            return Err(snapshot_invalid("XSAVE state is incompatible with CPU capabilities"));
        }
        self.xcr0 = Xcr0 { value: xcr0 };
        self.pkru = reader.read_u32()?;
        self.pkrs = reader.read_u32()?;
        let linaddr_width = reader.read_u8()?;
        let expected_linaddr_width = if cr4.la57() { 57 } else { 48 };
        if linaddr_width != expected_linaddr_width {
            return Err(snapshot_invalid("linear-address width disagrees with CR4"));
        }
        self.linaddr_width = linaddr_width;
        self.tsc_adjust = reader.read_i64()?;
        self.tsc_offset = reader.read_i64()?;

        self.the_i387.cwd = reader.read_u16()?;
        self.the_i387.swd = reader.read_u16()?;
        self.the_i387.twd = reader.read_u16()?;
        self.the_i387.foo = reader.read_u16()?;
        self.the_i387.fip = reader.read_u64()?;
        self.the_i387.fdp = reader.read_u64()?;
        self.the_i387.fcs = reader.read_u16()?;
        self.the_i387.fds = reader.read_u16()?;
        for st in &mut self.the_i387.st_space {
            st.signif = reader.read_u64()?;
            st.sign_exp = reader.read_u16()?;
        }

        for vmm in &mut self.vmm {
            reader.read_bytes(vmm.raw_mut())?;
        }
        let mxcsr = reader.read_u32()?;
        let mxcsr_mask = reader.read_u32()?;
        if mxcsr_mask != self.mxcsr_mask || mxcsr & !mxcsr_mask != 0 {
            return Err(snapshot_invalid("MXCSR state disagrees with configured CPU"));
        }
        self.mxcsr.mxcsr = mxcsr;
        for mask in &mut self.opmask {
            mask.set_rrx(reader.read_u64()?);
        }

        let monitor_addr = reader.read_u64()?;
        let monitor_armed_by = reader.read_u32()?;
        if monitor_addr & 63 != 0 || monitor_armed_by > super::cpu::BX_MONITOR_ARMED_BY_UMONITOR {
            return Err(snapshot_invalid("monitor state is invalid"));
        }
        self.monitor.monitor_addr = monitor_addr;
        self.monitor.armed_by = monitor_armed_by;

        let supports_uintr = self.bx_cpuid_support_isa_extension(
            super::decoder::features::X86Feature::IsaUintr,
        );
        restore_v3_uintr(reader, &mut self.uintr, supports_uintr, self.linaddr_width)?;
        restore_v3_fixed_msrs(reader, &mut self.msr, self.ia32_xss_suppmask)?;
        for msr in &mut self.msrs {
            let index = reader.read_u32()?;
            let kind = reader.read_u32()?;
            let value = reader.read_u64()?;
            let reset_value = reader.read_u64()?;
            let reserved = reader.read_u64()?;
            let ignored = reader.read_u64()?;
            if kind > 2
                || index != msr.index
                || kind != msr.r#type
                || reset_value != msr.reset_value
                || reserved != msr.reserved
                || ignored != msr.ignored
            {
                return Err(snapshot_invalid("generic MSR configuration mismatch"));
            }
            msr.val64 = value;
        }

        restore_v3_amx(reader, &mut self.amx)?;

        let mode = cpu_mode_from_wire(reader.read_u32()?)?;
        let activity_state = activity_state_from_wire(reader.read_u32()?)?;
        validate_mode_state(mode, self.cr0, self.efer, self.eflags, &self.sregs)?;
        self.cpu_mode = mode;
        self.activity_state = activity_state;
        let pending_event = reader.read_u32()?;
        let event_mask = reader.read_u32()?;
        let async_event = reader.read_u32()?;
        const EVENT_BITS: u32 = 0x0000_FEF7;
        const ASYNC_BITS: u32 = 1 | (1 << 30) | (1 << 31);
        if pending_event & !EVENT_BITS != 0
            || event_mask & !EVENT_BITS != 0
            || async_event & !ASYNC_BITS != 0
        {
            return Err(snapshot_invalid("CPU event state contains unknown bits"));
        }
        self.pending_event = pending_event;
        self.event_mask = event_mask;
        self.async_event = async_event;
        self.user_pl = reader.read_bool()?;
        self.in_smm = reader.read_bool()?;
        self.in_event = reader.read_bool()?;
        self.ext = reader.read_bool()?;
        self.nmi_unblocking_iret = reader.read_bool()?;
        self.last_exception_type = read_i32(reader)?;
        validate_last_exception_type(self.last_exception_type)?;
        self.smbase = reader.read_u32()?;
        self.alignment_check_mask = reader.read_u32()?;
        let a20_mask = reader.read_u64()?;
        if a20_mask != u64::MAX && a20_mask != 0xFFFF_FFFF_FFEF_FFFF {
            return Err(snapshot_invalid("CPU A20 mask is invalid"));
        }
        self.a20_mask = a20_mask;
        self.fred_event_info = reader.read_u32()?;
        self.fred_event_data = reader.read_u64()?;

        self.in_vmx = reader.read_bool()?;
        self.in_vmx_guest = reader.read_bool()?;
        self.in_smm_vmx = reader.read_bool()?;
        self.in_smm_vmx_guest = reader.read_bool()?;
        let vmcsptr = reader.read_u64()?;
        let vmcs_memtype = reader.read_u32()?;
        let vmxonptr = reader.read_u64()?;
        let supports_vmx = self.bx_cpuid_support_isa_extension(
            super::decoder::features::X86Feature::IsaVmx,
        );
        validate_vmx_state(
            supports_vmx,
            vmcsptr,
            vmcs_memtype,
            vmxonptr,
            self.in_vmx,
            self.in_vmx_guest,
            self.in_smm_vmx,
            self.in_smm_vmx_guest,
        )?;
        self.vmcsptr = vmcsptr;
        self.vmcs_memtype = vmcs_memtype;
        self.vmxonptr = vmxonptr;
        restore_v3_vmcs_cache(reader, &mut self.vmcs)?;

        self.in_svm_guest = reader.read_bool()?;
        self.svm_gif = reader.read_bool()?;
        let vmcbptr = reader.read_u64()?;
        let vmcb_memtype = reader.read_u32()?;
        let has_vmcb = reader.read_bool()?;
        if vmcb_memtype > 8 || self.in_svm_guest && !has_vmcb {
            return Err(snapshot_invalid("SVM state is invalid"));
        }
        if has_vmcb != self.vmcb.is_some() {
            return Err(snapshot_invalid("SVM capability does not match snapshot"));
        }
        if vmcbptr != 0 && vmcbptr & 0xfff != 0 {
            return Err(snapshot_invalid("VMCB pointer is not page aligned"));
        }
        self.vmcbptr = vmcbptr;
        self.vmcb_memtype = vmcb_memtype;
        if let Some(vmcb) = &mut self.vmcb {
            restore_v3_vmcb_cache(reader, vmcb)?;
        }

        // These fields are caches derived by normal CR/mode writes.  They are
        // intentionally rebuilt only after every architectural input decoded.
        self.set_pkeys(self.pkru, self.pkrs);
        self.handle_fpu_mmx_mode_change();
        self.handle_sse_mode_change();
        self.handle_avx_mode_change();
        self.update_fetch_mode_mask();
        self.handle_alignment_check();

        // vmcbhostptr is a host mapping and remains deliberately invalid until
        // the parent restores memory and runs its post-restore cache hook.
        self.vmcbhostptr = 0;
        Ok(())
    }
}

#[derive(Default)]
struct SnapshotLenWriter {
    len: u64,
}

impl Write for SnapshotLenWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let bytes_len = u64::try_from(bytes.len())
            .map_err(|_| snapshot_invalid("encoded CPU length does not fit u64"))?;
        self.len = checked_snapshot_len_add(self.len, bytes_len)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn snapshot_invalid(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

fn write_i32<W: Write + ?Sized>(writer: &mut W, value: i32) -> io::Result<()> {
    writer.write_u32(u32::from_le_bytes(value.to_le_bytes()))
}

fn read_i32<R: Read>(reader: &mut SnapshotReader<R>) -> io::Result<i32> {
    Ok(i32::from_le_bytes(reader.read_u32()?.to_le_bytes()))
}

fn cpu_mode_to_wire(mode: CpuMode) -> u32 {
    match mode {
        CpuMode::Ia32Real => 0,
        CpuMode::Ia32V8086 => 1,
        CpuMode::Ia32Protected => 2,
        CpuMode::LongCompat => 3,
        CpuMode::Long64 => 4,
    }
}

fn cpu_mode_from_wire(value: u32) -> io::Result<CpuMode> {
    match value {
        0 => Ok(CpuMode::Ia32Real),
        1 => Ok(CpuMode::Ia32V8086),
        2 => Ok(CpuMode::Ia32Protected),
        3 => Ok(CpuMode::LongCompat),
        4 => Ok(CpuMode::Long64),
        _ => Err(snapshot_invalid("CPU mode is invalid")),
    }
}

fn activity_state_to_wire(state: CpuActivityState) -> u32 {
    match state {
        CpuActivityState::Active => 0,
        CpuActivityState::Hlt => 1,
        CpuActivityState::Shutdown => 2,
        CpuActivityState::WaitForSipi | CpuActivityState::VmxLastActivityState => 3,
        CpuActivityState::Mwait => 4,
        CpuActivityState::MwaitIf => 5,
    }
}

fn activity_state_from_wire(value: u32) -> io::Result<CpuActivityState> {
    match value {
        0 => Ok(CpuActivityState::Active),
        1 => Ok(CpuActivityState::Hlt),
        2 => Ok(CpuActivityState::Shutdown),
        3 => Ok(CpuActivityState::WaitForSipi),
        4 => Ok(CpuActivityState::Mwait),
        5 => Ok(CpuActivityState::MwaitIf),
        _ => Err(snapshot_invalid("CPU activity state is invalid")),
    }
}

fn validate_last_exception_type(value: i32) -> io::Result<()> {
    if matches!(value, -1 | 0 | 1 | 2 | 10) {
        Ok(())
    } else {
        Err(snapshot_invalid("last exception type is invalid"))
    }
}

fn validate_mode_state(
    mode: CpuMode,
    cr0: BxCr0,
    efer: BxEfer,
    eflags: EFlags,
    sregs: &[BxSegmentReg; 6],
) -> io::Result<()> {
    let pe = cr0.contains(BxCr0::PE);
    let lma = efer.contains(BxEfer::LMA);
    let vm = eflags.contains(EFlags::VM);
    let cs_long = sregs
        .get(1)
        .ok_or_else(|| snapshot_invalid("CPU segment register topology is invalid"))?
        .cache
        .u
        .segment_l();
    let valid = match mode {
        CpuMode::Ia32Real => !pe && !lma && !vm,
        CpuMode::Ia32V8086 => pe && !lma && vm,
        CpuMode::Ia32Protected => pe && !lma && !vm,
        CpuMode::LongCompat => pe && lma && !vm && !cs_long,
        CpuMode::Long64 => pe && lma && !vm && cs_long,
    };
    if valid {
        Ok(())
    } else {
        Err(snapshot_invalid("CPU mode conflicts with control and segment state"))
    }
}

fn is_canonical_to_width(address: u64, width: u32) -> bool {
    if width == 0 || width > 64 {
        return false;
    }
    let signed = i64::from_ne_bytes(address.to_ne_bytes());
    let shifted = signed >> (width - 1);
    u64::from_ne_bytes(shifted.wrapping_add(1).to_ne_bytes()) < 2
}

// ============================================================================
// Segment register helpers
// ============================================================================

use super::descriptor::{BxGlobalSegmentReg, BxSegmentReg};

fn write_v3_seg_reg<W: Write + ?Sized>(writer: &mut W, seg: &BxSegmentReg) -> io::Result<()> {
    writer.write_u16(seg.selector.value)?;
    writer.write_u16(seg.selector.index)?;
    writer.write_u16(seg.selector.ti)?;
    writer.write_u8(seg.selector.rpl)?;
    writer.write_u32(seg.cache.valid)?;
    writer.write_bool(seg.cache.p)?;
    writer.write_u8(seg.cache.dpl)?;
    writer.write_bool(seg.cache.segment)?;
    writer.write_u8(seg.cache.r#type)?;
    writer.write_u64(seg.cache.u.segment_base())?;
    writer.write_u32(seg.cache.u.segment_limit_scaled())?;
    writer.write_bool(seg.cache.u.segment_g())?;
    writer.write_bool(seg.cache.u.segment_d_b())?;
    writer.write_bool(seg.cache.u.segment_l())?;
    writer.write_bool(seg.cache.u.segment_avl())
}

fn read_v3_seg_reg<R: Read>(
    reader: &mut SnapshotReader<R>,
    seg: &mut BxSegmentReg,
) -> io::Result<()> {
    let selector_value = reader.read_u16()?;
    let selector_index = reader.read_u16()?;
    let selector_ti = reader.read_u16()?;
    let selector_rpl = reader.read_u8()?;
    let expected_index = selector_value >> 3;
    let expected_ti = (selector_value >> 2) & 1;
    let expected_rpl = u8::try_from(selector_value & 3)
        .map_err(|_| snapshot_invalid("selector RPL does not fit u8"))?;
    if selector_index != expected_index || selector_ti != expected_ti || selector_rpl != expected_rpl {
        return Err(snapshot_invalid("segment selector cache is inconsistent"));
    }

    let valid = reader.read_u32()?;
    let present = reader.read_bool()?;
    let dpl = reader.read_u8()?;
    let segment = reader.read_bool()?;
    let descriptor_type = reader.read_u8()?;
    let base = reader.read_u64()?;
    let limit_scaled = reader.read_u32()?;
    let granularity = reader.read_bool()?;
    let default_size = reader.read_bool()?;
    let long_mode = reader.read_bool()?;
    let available = reader.read_bool()?;
    const VALID_CACHE_BITS: u32 = super::descriptor::SegAccess::VALID.bits()
        | super::descriptor::SegAccess::ALL_ACCESS.bits();
    if valid & !VALID_CACHE_BITS != 0
        || dpl > 3
        || descriptor_type > 0x0f
        || granularity && limit_scaled & 0x0fff != 0x0fff
        || long_mode && (!segment || default_size)
    {
        return Err(snapshot_invalid("segment cache is invalid"));
    }

    seg.selector.value = selector_value;
    seg.selector.index = selector_index;
    seg.selector.ti = selector_ti;
    seg.selector.rpl = selector_rpl;
    seg.cache.valid = valid;
    seg.cache.p = present;
    seg.cache.dpl = dpl;
    seg.cache.segment = segment;
    seg.cache.r#type = descriptor_type;
    seg.cache.u.set_segment_base(base);
    seg.cache.u.set_segment_limit_scaled(limit_scaled);
    seg.cache.u.set_segment_g(granularity);
    seg.cache.u.set_segment_d_b(default_size);
    seg.cache.u.set_segment_l(long_mode);
    seg.cache.u.set_segment_avl(available);
    Ok(())
}

fn write_v3_global_seg<W: Write + ?Sized>(
    writer: &mut W,
    seg: &BxGlobalSegmentReg,
) -> io::Result<()> {
    writer.write_u64(seg.base)?;
    writer.write_u16(seg.limit)
}

fn read_v3_global_seg<R: Read>(
    reader: &mut SnapshotReader<R>,
    seg: &mut BxGlobalSegmentReg,
) -> io::Result<()> {
    seg.base = reader.read_u64()?;
    seg.limit = reader.read_u16()?;
    Ok(())
}

fn save_v3_fixed_msrs<W: Write + ?Sized>(
    writer: &mut W,
    msr: &super::cpu::BxRegsMsr,
) -> io::Result<()> {
    writer.write_u64(msr.apicbase)?;
    writer.write_u64(msr.star)?;
    writer.write_u64(msr.lstar)?;
    writer.write_u64(msr.cstar)?;
    writer.write_u32(msr.fmask)?;
    writer.write_u64(msr.kernelgsbase)?;
    writer.write_u32(msr.tsc_aux)?;
    writer.write_u32(msr.sysenter_cs_msr)?;
    writer.write_u64(msr.sysenter_esp_msr)?;
    writer.write_u64(msr.sysenter_eip_msr)?;
    writer.write_u64(msr.pat.U64())?;
    for value in &msr.mtrrphys {
        writer.write_u64(*value)?;
    }
    writer.write_u64(msr.mtrrfix64k.U64())?;
    for value in &msr.mtrrfix16k {
        writer.write_u64(value.U64())?;
    }
    for value in &msr.mtrrfix4k {
        writer.write_u64(value.U64())?;
    }
    writer.write_u32(msr.mtrr_deftype)?;
    writer.write_u32(msr.ia32_feature_ctrl)?;
    writer.write_u32(msr.svm_vm_cr)?;
    writer.write_u64(msr.svm_hsave_pa)?;
    writer.write_u64(msr.ia32_xss)?;
    for value in &msr.ia32_cet_control {
        writer.write_u64(*value)?;
    }
    for value in &msr.ia32_pl_ssp {
        writer.write_u64(*value)?;
    }
    writer.write_u64(msr.ia32_interrupt_ssp_table)?;
    for value in &msr.ia32_fred_rsp {
        writer.write_u64(*value)?;
    }
    for value in &msr.ia32_fred_ssp {
        writer.write_u64(*value)?;
    }
    writer.write_u64(msr.ia32_fred_stack_levels)?;
    writer.write_u64(msr.ia32_fred_cfg)?;
    writer.write_u32(msr.ia32_umwait_ctrl)?;
    writer.write_u32(msr.ia32_spec_ctrl)
}

fn restore_v3_fixed_msrs<R: Read>(
    reader: &mut SnapshotReader<R>,
    msr: &mut super::cpu::BxRegsMsr,
    ia32_xss_suppmask: u32,
) -> io::Result<()> {
    msr.apicbase = reader.read_u64()?;
    msr.star = reader.read_u64()?;
    msr.lstar = reader.read_u64()?;
    msr.cstar = reader.read_u64()?;
    msr.fmask = reader.read_u32()?;
    msr.kernelgsbase = reader.read_u64()?;
    msr.tsc_aux = reader.read_u32()?;
    msr.sysenter_cs_msr = reader.read_u32()?;
    msr.sysenter_esp_msr = reader.read_u64()?;
    msr.sysenter_eip_msr = reader.read_u64()?;
    msr.pat.set_U64(reader.read_u64()?);
    for value in &mut msr.mtrrphys {
        *value = reader.read_u64()?;
    }
    msr.mtrrfix64k.set_U64(reader.read_u64()?);
    for value in &mut msr.mtrrfix16k {
        value.set_U64(reader.read_u64()?);
    }
    for value in &mut msr.mtrrfix4k {
        value.set_U64(reader.read_u64()?);
    }
    msr.mtrr_deftype = reader.read_u32()?;
    msr.ia32_feature_ctrl = reader.read_u32()?;
    msr.svm_vm_cr = reader.read_u32()?;
    msr.svm_hsave_pa = reader.read_u64()?;
    let ia32_xss = reader.read_u64()?;
    if ia32_xss & !u64::from(ia32_xss_suppmask) != 0 {
        return Err(snapshot_invalid("IA32_XSS contains unsupported state bits"));
    }
    msr.ia32_xss = ia32_xss;
    for value in &mut msr.ia32_cet_control {
        *value = reader.read_u64()?;
    }
    for value in &mut msr.ia32_pl_ssp {
        *value = reader.read_u64()?;
    }
    msr.ia32_interrupt_ssp_table = reader.read_u64()?;
    for value in &mut msr.ia32_fred_rsp {
        *value = reader.read_u64()?;
    }
    for value in &mut msr.ia32_fred_ssp {
        *value = reader.read_u64()?;
    }
    msr.ia32_fred_stack_levels = reader.read_u64()?;
    msr.ia32_fred_cfg = reader.read_u64()?;
    msr.ia32_umwait_ctrl = reader.read_u32()?;
    msr.ia32_spec_ctrl = reader.read_u32()?;
    Ok(())
}

fn restore_v3_uintr<R: Read>(
    reader: &mut SnapshotReader<R>,
    uintr: &mut super::cpu::Uintr,
    supports_uintr: bool,
    linaddr_width: u8,
) -> io::Result<()> {
    let ui_handler = reader.read_u64()?;
    let stack_adjust = reader.read_u64()?;
    let uinv = reader.read_u32()?;
    let uitt_size = reader.read_u32()?;
    let uitt_addr = reader.read_u64()?;
    let upid_addr = reader.read_u64()?;
    let uirr = reader.read_u64()?;
    let uif = reader.read_bool()?;
    if uinv > u32::from(u8::MAX)
        || !is_canonical_to_width(ui_handler, u32::from(linaddr_width))
        || !is_canonical_to_width(stack_adjust, u32::from(linaddr_width))
        || !is_canonical_to_width(uitt_addr, u32::from(linaddr_width))
        || !is_canonical_to_width(upid_addr, u32::from(linaddr_width))
        || upid_addr & 0x3f != 0
        || uitt_addr & 0x0e != 0
        || !supports_uintr
            && (ui_handler != 0
                || stack_adjust != 0
                || uinv != 0
                || uitt_size != 0
                || uitt_addr != 0
                || upid_addr != 0
                || uirr != 0
                || uif)
    {
        return Err(snapshot_invalid("UINTR state is invalid"));
    }
    uintr.ui_handler = ui_handler;
    uintr.stack_adjust = stack_adjust;
    uintr.uinv = uinv;
    uintr.uitt_size = uitt_size;
    uintr.uitt_addr = uitt_addr;
    uintr.upid_addr = upid_addr;
    uintr.uirr = uirr;
    uintr.uif = uif;
    Ok(())
}

fn write_v3_amx<W: Write + ?Sized>(
    writer: &mut W,
    amx: Option<&super::avx::AMX>,
) -> io::Result<()> {
    match amx {
        Some(amx) => {
            writer.write_bool(true)?;
            writer.write_u32(amx.palette_id)?;
            writer.write_u32(amx.start_row)?;
            for tilecfg in &amx.tilecfg {
                writer.write_u32(tilecfg.rows)?;
                writer.write_u32(tilecfg.bytes_per_row)?;
            }
            for tile in &amx.tile {
                writer.write_bytes(tile)?;
            }
            writer.write_u8(amx.tile_use_tracker)
        }
        None => writer.write_bool(false),
    }
}

fn restore_v3_amx<R: Read>(
    reader: &mut SnapshotReader<R>,
    amx: &mut Option<alloc::boxed::Box<super::avx::AMX>>,
) -> io::Result<()> {
    let has_amx = reader.read_bool()?;
    if has_amx != amx.is_some() {
        return Err(snapshot_invalid("AMX capability does not match snapshot"));
    }
    let Some(amx) = amx.as_deref_mut() else {
        return Ok(());
    };
    let palette_id = reader.read_u32()?;
    let start_row = reader.read_u32()?;
    if palette_id > 1 || start_row > 16 {
        return Err(snapshot_invalid("AMX palette or start row is invalid"));
    }
    amx.palette_id = palette_id;
    amx.start_row = start_row;
    for tilecfg in &mut amx.tilecfg {
        let rows = reader.read_u32()?;
        let bytes_per_row = reader.read_u32()?;
        if rows > 16 || bytes_per_row > 64 {
            return Err(snapshot_invalid("AMX tile configuration is invalid"));
        }
        tilecfg.rows = rows;
        tilecfg.bytes_per_row = bytes_per_row;
    }
    for tile in &mut amx.tile {
        reader.read_bytes(tile)?;
    }
    amx.tile_use_tracker = reader.read_u8()?;
    Ok(())
}

macro_rules! write_v3_fields {
    ($writer:expr; $($method:ident($value:expr)),* $(,)?) => {
        $(
            $writer.$method($value)?;
        )*
    };
}

macro_rules! read_v3_fields {
    ($reader:expr, $target:ident; $($method:ident => $field:ident),* $(,)?) => {
        $(
            $target.$field = $reader.$method()?;
        )*
    };
}

fn save_v3_vmcs_cache<W: Write + ?Sized>(
    writer: &mut W,
    vm: &super::vmx::VmcsCache,
) -> io::Result<()> {
    write_v3_fields!(writer;
        write_bool(vm.launched),
        write_u64(vm.host_cr0), write_u64(vm.host_cr3), write_u64(vm.host_cr4),
        write_u64(vm.host_rsp), write_u64(vm.host_rip),
        write_u16(vm.host_cs_selector), write_u16(vm.host_ss_selector),
        write_u16(vm.host_ds_selector), write_u16(vm.host_es_selector),
        write_u16(vm.host_fs_selector), write_u16(vm.host_gs_selector),
        write_u16(vm.host_tr_selector),
        write_u64(vm.host_fs_base), write_u64(vm.host_gs_base),
        write_u64(vm.host_tr_base), write_u64(vm.host_gdtr_base),
        write_u64(vm.host_idtr_base), write_u64(vm.host_ia32_efer),
        write_u64(vm.host_ia32_pat), write_u32(vm.host_sysenter_cs),
        write_u64(vm.host_sysenter_esp), write_u64(vm.host_sysenter_eip),
        write_u64(vm.host_perf_global_ctrl), write_u64(vm.host_pkrs),
        write_u64(vm.host_ia32_spec_ctrl), write_u64(vm.host_ia32_s_cet),
        write_u64(vm.host_ssp), write_u64(vm.host_interrupt_ssp_table_addr),
        write_u64(vm.host_fred_config)
    );
    for value in &vm.host_fred_rsp {
        writer.write_u64(*value)?;
    }
    writer.write_u64(vm.host_fred_stack_levels)?;
    for value in &vm.host_fred_ssp {
        writer.write_u64(*value)?;
    }
    write_v3_fields!(writer;
        write_u64(vm.guest_cr0), write_u64(vm.guest_cr3), write_u64(vm.guest_cr4),
        write_u64(vm.guest_rsp), write_u64(vm.guest_rip), write_u64(vm.guest_rflags),
        write_u64(vm.guest_dr7), write_u64(vm.guest_ia32_efer),
        write_u64(vm.guest_ia32_pat), write_u32(vm.guest_ia32_sysenter_cs),
        write_u64(vm.guest_ia32_sysenter_esp), write_u64(vm.guest_ia32_sysenter_eip),
        write_u16(vm.guest_cs_selector), write_u16(vm.guest_ss_selector),
        write_u16(vm.guest_ds_selector), write_u16(vm.guest_es_selector),
        write_u16(vm.guest_fs_selector), write_u16(vm.guest_gs_selector),
        write_u16(vm.guest_ldtr_selector), write_u16(vm.guest_tr_selector),
        write_u64(vm.guest_cs_base), write_u64(vm.guest_ss_base),
        write_u64(vm.guest_ds_base), write_u64(vm.guest_es_base),
        write_u64(vm.guest_fs_base), write_u64(vm.guest_gs_base),
        write_u64(vm.guest_ldtr_base), write_u64(vm.guest_tr_base),
        write_u64(vm.guest_gdtr_base), write_u64(vm.guest_idtr_base),
        write_u32(vm.guest_cs_limit), write_u32(vm.guest_ss_limit),
        write_u32(vm.guest_ds_limit), write_u32(vm.guest_es_limit),
        write_u32(vm.guest_fs_limit), write_u32(vm.guest_gs_limit),
        write_u32(vm.guest_ldtr_limit), write_u32(vm.guest_tr_limit),
        write_u32(vm.guest_gdtr_limit), write_u32(vm.guest_idtr_limit),
        write_u32(vm.guest_cs_ar), write_u32(vm.guest_ss_ar),
        write_u32(vm.guest_ds_ar), write_u32(vm.guest_es_ar),
        write_u32(vm.guest_fs_ar), write_u32(vm.guest_gs_ar),
        write_u32(vm.guest_ldtr_ar), write_u32(vm.guest_tr_ar),
        write_u32(vm.guest_activity_state), write_u32(vm.guest_interruptibility_state),
        write_u64(vm.guest_ia32_s_cet), write_u64(vm.guest_ssp),
        write_u64(vm.guest_interrupt_ssp_table_addr), write_u64(vm.guest_fred_config)
    );
    for value in &vm.guest_fred_rsp {
        writer.write_u64(*value)?;
    }
    writer.write_u64(vm.guest_fred_stack_levels)?;
    for value in &vm.guest_fred_ssp {
        writer.write_u64(*value)?;
    }
    write_v3_fields!(writer;
        write_u64(vm.guest_pkrs), write_u64(vm.guest_ia32_spec_ctrl),
        write_u64(vm.guest_pending_dbg_exceptions), write_u64(vm.guest_ia32_debugctl),
        write_u32(vm.guest_smbase), write_u64(vm.vmfunc_ctrls),
        write_u64(vm.eptp_list_address), write_u32(vm.vm_instruction_error),
        write_u32(vm.exit_reason), write_u64(vm.exit_qualification),
        write_u32(vm.exit_intr_info), write_u32(vm.exit_intr_error_code),
        write_u32(vm.exit_instruction_length), write_u32(vm.exit_instruction_info),
        write_u32(vm.idt_vectoring_info), write_u32(vm.idt_vectoring_error_code),
        write_u64(vm.guest_linear_addr), write_u32(vm.pin_based_ctls),
        write_u32(vm.proc_based_ctls), write_u32(vm.secondary_proc_based_ctls),
        write_u32(vm.vm_exit_ctls), write_u64(vm.vm_exit_ctls2),
        write_u32(vm.vm_entry_ctls), write_u32(vm.vm_entry_intr_info),
        write_u32(vm.vm_entry_exception_error_code), write_u32(vm.vm_entry_instruction_length),
        write_u32(vm.exception_bitmap), write_u32(vm.vm_pf_mask),
        write_u32(vm.vm_pf_match), write_u32(vm.vm_cr3_target_cnt)
    );
    for value in &vm.vm_cr3_target_value {
        writer.write_u64(*value)?;
    }
    write_v3_fields!(writer;
        write_u64(vm.cr0_guest_host_mask), write_u64(vm.cr4_guest_host_mask),
        write_u64(vm.cr0_read_shadow), write_u64(vm.cr4_read_shadow),
        write_u64(vm.vmcs_link_pointer), write_u64(vm.tsc_offset),
        write_u64(vm.msr_bitmap_addr)
    );
    for value in &vm.io_bitmap_addr {
        writer.write_u64(*value)?;
    }
    write_v3_fields!(writer;
        write_u64(vm.vmread_bitmap_addr), write_u64(vm.vmwrite_bitmap_addr),
        write_u32(vm.vmx_preemption_timer_value), write_u32(vm.tpr_threshold),
        write_u64(vm.virtual_apic_page_addr), write_u64(vm.pi_desc_addr),
        write_u8(vm.pi_notification_vector), write_u16(vm.vpid),
        write_u64(vm.eptptr), write_u64(vm.guest_physical_addr),
        write_u64(vm.vmentry_msr_load_addr), write_u64(vm.vmexit_msr_store_addr),
        write_u64(vm.vmexit_msr_load_addr), write_u32(vm.vmentry_msr_load_cnt),
        write_u32(vm.vmexit_msr_store_cnt), write_u32(vm.vmexit_msr_load_cnt),
        write_bool(vm.shadow_stack_prematurely_busy)
    );
    Ok(())
}

fn restore_v3_vmcs_cache<R: Read>(
    reader: &mut SnapshotReader<R>,
    vm: &mut super::vmx::VmcsCache,
) -> io::Result<()> {
    read_v3_fields!(reader, vm;
        read_bool => launched,
        read_u64 => host_cr0, read_u64 => host_cr3, read_u64 => host_cr4,
        read_u64 => host_rsp, read_u64 => host_rip,
        read_u16 => host_cs_selector, read_u16 => host_ss_selector,
        read_u16 => host_ds_selector, read_u16 => host_es_selector,
        read_u16 => host_fs_selector, read_u16 => host_gs_selector,
        read_u16 => host_tr_selector,
        read_u64 => host_fs_base, read_u64 => host_gs_base,
        read_u64 => host_tr_base, read_u64 => host_gdtr_base,
        read_u64 => host_idtr_base, read_u64 => host_ia32_efer,
        read_u64 => host_ia32_pat, read_u32 => host_sysenter_cs,
        read_u64 => host_sysenter_esp, read_u64 => host_sysenter_eip,
        read_u64 => host_perf_global_ctrl, read_u64 => host_pkrs,
        read_u64 => host_ia32_spec_ctrl, read_u64 => host_ia32_s_cet,
        read_u64 => host_ssp, read_u64 => host_interrupt_ssp_table_addr,
        read_u64 => host_fred_config
    );
    for value in &mut vm.host_fred_rsp {
        *value = reader.read_u64()?;
    }
    vm.host_fred_stack_levels = reader.read_u64()?;
    for value in &mut vm.host_fred_ssp {
        *value = reader.read_u64()?;
    }
    read_v3_fields!(reader, vm;
        read_u64 => guest_cr0, read_u64 => guest_cr3, read_u64 => guest_cr4,
        read_u64 => guest_rsp, read_u64 => guest_rip, read_u64 => guest_rflags,
        read_u64 => guest_dr7, read_u64 => guest_ia32_efer,
        read_u64 => guest_ia32_pat, read_u32 => guest_ia32_sysenter_cs,
        read_u64 => guest_ia32_sysenter_esp, read_u64 => guest_ia32_sysenter_eip,
        read_u16 => guest_cs_selector, read_u16 => guest_ss_selector,
        read_u16 => guest_ds_selector, read_u16 => guest_es_selector,
        read_u16 => guest_fs_selector, read_u16 => guest_gs_selector,
        read_u16 => guest_ldtr_selector, read_u16 => guest_tr_selector,
        read_u64 => guest_cs_base, read_u64 => guest_ss_base,
        read_u64 => guest_ds_base, read_u64 => guest_es_base,
        read_u64 => guest_fs_base, read_u64 => guest_gs_base,
        read_u64 => guest_ldtr_base, read_u64 => guest_tr_base,
        read_u64 => guest_gdtr_base, read_u64 => guest_idtr_base,
        read_u32 => guest_cs_limit, read_u32 => guest_ss_limit,
        read_u32 => guest_ds_limit, read_u32 => guest_es_limit,
        read_u32 => guest_fs_limit, read_u32 => guest_gs_limit,
        read_u32 => guest_ldtr_limit, read_u32 => guest_tr_limit,
        read_u32 => guest_gdtr_limit, read_u32 => guest_idtr_limit,
        read_u32 => guest_cs_ar, read_u32 => guest_ss_ar,
        read_u32 => guest_ds_ar, read_u32 => guest_es_ar,
        read_u32 => guest_fs_ar, read_u32 => guest_gs_ar,
        read_u32 => guest_ldtr_ar, read_u32 => guest_tr_ar,
        read_u32 => guest_activity_state, read_u32 => guest_interruptibility_state,
        read_u64 => guest_ia32_s_cet, read_u64 => guest_ssp,
        read_u64 => guest_interrupt_ssp_table_addr, read_u64 => guest_fred_config
    );
    for value in &mut vm.guest_fred_rsp {
        *value = reader.read_u64()?;
    }
    vm.guest_fred_stack_levels = reader.read_u64()?;
    for value in &mut vm.guest_fred_ssp {
        *value = reader.read_u64()?;
    }
    read_v3_fields!(reader, vm;
        read_u64 => guest_pkrs, read_u64 => guest_ia32_spec_ctrl,
        read_u64 => guest_pending_dbg_exceptions, read_u64 => guest_ia32_debugctl,
        read_u32 => guest_smbase, read_u64 => vmfunc_ctrls,
        read_u64 => eptp_list_address, read_u32 => vm_instruction_error,
        read_u32 => exit_reason, read_u64 => exit_qualification,
        read_u32 => exit_intr_info, read_u32 => exit_intr_error_code,
        read_u32 => exit_instruction_length, read_u32 => exit_instruction_info,
        read_u32 => idt_vectoring_info, read_u32 => idt_vectoring_error_code,
        read_u64 => guest_linear_addr, read_u32 => pin_based_ctls,
        read_u32 => proc_based_ctls, read_u32 => secondary_proc_based_ctls,
        read_u32 => vm_exit_ctls, read_u64 => vm_exit_ctls2,
        read_u32 => vm_entry_ctls, read_u32 => vm_entry_intr_info,
        read_u32 => vm_entry_exception_error_code, read_u32 => vm_entry_instruction_length,
        read_u32 => exception_bitmap, read_u32 => vm_pf_mask,
        read_u32 => vm_pf_match, read_u32 => vm_cr3_target_cnt
    );
    if vm.guest_activity_state > 3 || vm.vm_cr3_target_cnt > 4 {
        return Err(snapshot_invalid("VMCS activity or control field is invalid"));
    }
    for value in &mut vm.vm_cr3_target_value {
        *value = reader.read_u64()?;
    }
    read_v3_fields!(reader, vm;
        read_u64 => cr0_guest_host_mask, read_u64 => cr4_guest_host_mask,
        read_u64 => cr0_read_shadow, read_u64 => cr4_read_shadow,
        read_u64 => vmcs_link_pointer, read_u64 => tsc_offset,
        read_u64 => msr_bitmap_addr
    );
    for value in &mut vm.io_bitmap_addr {
        *value = reader.read_u64()?;
    }
    read_v3_fields!(reader, vm;
        read_u64 => vmread_bitmap_addr, read_u64 => vmwrite_bitmap_addr,
        read_u32 => vmx_preemption_timer_value, read_u32 => tpr_threshold,
        read_u64 => virtual_apic_page_addr, read_u64 => pi_desc_addr,
        read_u8 => pi_notification_vector, read_u16 => vpid,
        read_u64 => eptptr, read_u64 => guest_physical_addr,
        read_u64 => vmentry_msr_load_addr, read_u64 => vmexit_msr_store_addr,
        read_u64 => vmexit_msr_load_addr, read_u32 => vmentry_msr_load_cnt,
        read_u32 => vmexit_msr_store_cnt, read_u32 => vmexit_msr_load_cnt,
        read_bool => shadow_stack_prematurely_busy
    );
    if vm.tpr_threshold > 15 {
        return Err(snapshot_invalid("VMCS TPR threshold is invalid"));
    }
    Ok(())
}

fn validate_vmx_state(
    supports_vmx: bool,
    vmcsptr: u64,
    vmcs_memtype: u32,
    vmxonptr: u64,
    in_vmx: bool,
    in_vmx_guest: bool,
    in_smm_vmx: bool,
    in_smm_vmx_guest: bool,
) -> io::Result<()> {
    let invalid_ptr = super::vmx::BX_INVALID_VMCSPTR;
    if vmcs_memtype > 8
        || vmcsptr != invalid_ptr && vmcsptr & 0x0fff != 0
        || vmxonptr != invalid_ptr && vmxonptr & 0x0fff != 0
        || in_vmx_guest && !in_vmx
        || in_smm_vmx_guest && !in_smm_vmx
        || (in_vmx || in_vmx_guest || in_smm_vmx || in_smm_vmx_guest) && !supports_vmx
    {
        return Err(snapshot_invalid("VMX state is invalid"));
    }
    Ok(())
}

fn save_v3_vmcb_cache<W: Write + ?Sized>(
    writer: &mut W,
    vmcb: &super::svm::VmcbCache,
) -> io::Result<()> {
    for seg in &vmcb.host_state.sregs {
        write_v3_seg_reg(writer, seg)?;
    }
    write_v3_global_seg(writer, &vmcb.host_state.gdtr)?;
    write_v3_global_seg(writer, &vmcb.host_state.idtr)?;
    write_v3_fields!(writer;
        write_u32(vmcb.host_state.efer.bits()), write_u32(vmcb.host_state.cr0.bits()),
        write_u64(vmcb.host_state.cr4.bits()), write_u64(vmcb.host_state.cr3),
        write_u32(vmcb.host_state.eflags), write_u64(vmcb.host_state.rip),
        write_u64(vmcb.host_state.rsp), write_u64(vmcb.host_state.rax),
        write_u64(vmcb.host_state.pat_msr.U64()),
        write_u16(vmcb.ctrls.cr_rd_ctrl), write_u16(vmcb.ctrls.cr_wr_ctrl),
        write_u16(vmcb.ctrls.dr_rd_ctrl), write_u16(vmcb.ctrls.dr_wr_ctrl),
        write_u32(vmcb.ctrls.exceptions_intercept)
    );
    for value in &vmcb.ctrls.intercept_vector {
        writer.write_u32(*value)?;
    }
    write_v3_fields!(writer;
        write_u32(vmcb.ctrls.exitintinfo), write_u32(vmcb.ctrls.exitintinfo_error_code),
        write_u32(vmcb.ctrls.eventinj), write_u64(vmcb.ctrls.iopm_base),
        write_u64(vmcb.ctrls.msrpm_base), write_u8(vmcb.ctrls.v_tpr),
        write_u8(vmcb.ctrls.v_intr_prio), write_bool(vmcb.ctrls.v_ignore_tpr),
        write_bool(vmcb.ctrls.v_intr_masking), write_u8(vmcb.ctrls.v_intr_vector),
        write_bool(vmcb.ctrls.nested_paging), write_u64(vmcb.ctrls.ncr3),
        write_u16(vmcb.ctrls.pause_filter_count), write_u16(vmcb.ctrls.pause_filter_threshold),
        write_u64(vmcb.ctrls.last_pause_time)
    );
    Ok(())
}

fn restore_v3_vmcb_cache<R: Read>(
    reader: &mut SnapshotReader<R>,
    vmcb: &mut super::svm::VmcbCache,
) -> io::Result<()> {
    for seg in &mut vmcb.host_state.sregs {
        read_v3_seg_reg(reader, seg)?;
    }
    read_v3_global_seg(reader, &mut vmcb.host_state.gdtr)?;
    read_v3_global_seg(reader, &mut vmcb.host_state.idtr)?;
    vmcb.host_state.efer = BxEfer::from_bits(reader.read_u32()?)
        .ok_or_else(|| snapshot_invalid("VMCB host EFER contains reserved bits"))?;
    vmcb.host_state.cr0 = BxCr0::from_bits(reader.read_u32()?)
        .ok_or_else(|| snapshot_invalid("VMCB host CR0 contains reserved bits"))?;
    vmcb.host_state.cr4 = BxCr4::from_bits(reader.read_u64()?)
        .ok_or_else(|| snapshot_invalid("VMCB host CR4 contains reserved bits"))?;
    vmcb.host_state.cr3 = reader.read_u64()?;
    let host_eflags = reader.read_u32()?;
    EFlags::from_bits(host_eflags)
        .ok_or_else(|| snapshot_invalid("VMCB host EFLAGS contains reserved bits"))?;
    vmcb.host_state.eflags = host_eflags;
    vmcb.host_state.rip = reader.read_u64()?;
    vmcb.host_state.rsp = reader.read_u64()?;
    vmcb.host_state.rax = reader.read_u64()?;
    vmcb.host_state.pat_msr.set_U64(reader.read_u64()?);
    vmcb.ctrls.cr_rd_ctrl = reader.read_u16()?;
    vmcb.ctrls.cr_wr_ctrl = reader.read_u16()?;
    vmcb.ctrls.dr_rd_ctrl = reader.read_u16()?;
    vmcb.ctrls.dr_wr_ctrl = reader.read_u16()?;
    vmcb.ctrls.exceptions_intercept = reader.read_u32()?;
    for value in &mut vmcb.ctrls.intercept_vector {
        *value = reader.read_u32()?;
    }
    vmcb.ctrls.exitintinfo = reader.read_u32()?;
    vmcb.ctrls.exitintinfo_error_code = reader.read_u32()?;
    vmcb.ctrls.eventinj = reader.read_u32()?;
    vmcb.ctrls.iopm_base = reader.read_u64()?;
    vmcb.ctrls.msrpm_base = reader.read_u64()?;
    let v_tpr = reader.read_u8()?;
    let v_intr_prio = reader.read_u8()?;
    let v_ignore_tpr = reader.read_bool()?;
    let v_intr_masking = reader.read_bool()?;
    let v_intr_vector = reader.read_u8()?;
    let nested_paging = reader.read_bool()?;
    if v_tpr > 15 || v_intr_prio > 15 {
        return Err(snapshot_invalid("VMCB virtual interrupt priority is invalid"));
    }
    vmcb.ctrls.v_tpr = v_tpr;
    vmcb.ctrls.v_intr_prio = v_intr_prio;
    vmcb.ctrls.v_ignore_tpr = v_ignore_tpr;
    vmcb.ctrls.v_intr_masking = v_intr_masking;
    vmcb.ctrls.v_intr_vector = v_intr_vector;
    vmcb.ctrls.nested_paging = nested_paging;
    vmcb.ctrls.ncr3 = reader.read_u64()?;
    vmcb.ctrls.pause_filter_count = reader.read_u16()?;
    vmcb.ctrls.pause_filter_threshold = reader.read_u16()?;
    vmcb.ctrls.last_pause_time = reader.read_u64()?;
    Ok(())
}

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
    use std::io::Cursor;

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::core_i7_skylake::Corei7SkylakeX;
    use crate::cpu::crregs::{BxCr0, BxEfer};
    use crate::cpu::decoder::BxSegregs;
    use crate::cpu::eflags::EFlags;
    use crate::cpu::svm::VmcbCache;
    use crate::cpu::ResetReason;
    use super::CpuMode;
    use crate::snapshot::SnapshotReader;

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

    #[test]
    fn v3_restore_rebuilds_cpu_derived_state() {
        let mut source = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        source.reset(ResetReason::Hardware);

        source.cr0.insert(BxCr0::PE | BxCr0::AM | BxCr0::TS);
        source.cpu_mode = CpuMode::Long64;
        source.efer.insert(BxEfer::LMA);
        let cs = &mut source.sregs[BxSegregs::Cs as usize];
        cs.selector.value = (cs.selector.value & !3) | 3;
        cs.selector.rpl = 3;
        cs.cache.u.set_segment_d_b(false);
        cs.cache.u.set_segment_l(true);
        source.eflags.insert(EFlags::AC);

        source.set_pkeys(0b01, 0);
        source.handle_fpu_mmx_mode_change();
        source.handle_sse_mode_change();
        source.handle_avx_mode_change();
        source.update_fetch_mode_mask();
        source.handle_alignment_check();
        let expected_rd_pkey = source.rd_pkey;
        let expected_wr_pkey = source.wr_pkey;
        let expected_fetch_mode_mask = source.fetch_mode_mask;

        // Alignment state is derived from CS.RPL, CR0.AM, and RFLAGS.AC; a
        // stale serialized cache must not override those architectural inputs.
        source.alignment_check_mask = 0;
        let mut bytes = Vec::new();
        source.save_snapshot_v3_body(&mut bytes).unwrap();

        let mut restored = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        restored.reset(ResetReason::Hardware);
        // None of these caches are serialized authoritatively. Seed them with
        // values that cannot accidentally satisfy the restored architecture.
        restored.rd_pkey = [0; 16];
        restored.wr_pkey = [0; 16];
        restored.fetch_mode_mask = Default::default();
        restored.alignment_check_mask = 0;
        let mut reader = SnapshotReader::new(Cursor::new(bytes.clone()), bytes.len() as u64).unwrap();
        restored
            .restore_snapshot_v3_body(&mut reader, source.snapshot_cpu_id())
            .unwrap();
        reader.finish_exact().unwrap();

        assert_eq!(restored.rd_pkey, expected_rd_pkey);
        assert_eq!(restored.wr_pkey, expected_wr_pkey);
        assert_eq!(restored.fetch_mode_mask, expected_fetch_mode_mask);
        assert_eq!(restored.alignment_check_mask, 0xf);
    }
}
