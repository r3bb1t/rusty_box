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
        buf.extend_from_slice(&self.eflags.bits().to_le_bytes());
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
        self.xcr0 = Xcr0 { value: u32_at(d, &mut off) };
        self.xcr0_suppmask = u32_at(d, &mut off);
        self.pkru = u32_at(d, &mut off);
        self.pkrs = u32_at(d, &mut off);
        self.linaddr_width = d[off]; off += 1;
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
        self.user_pl = d[off] != 0; off += 1;
        self.in_smm = d[off] != 0; off += 1;
        self.ext = d[off] != 0; off += 1;
        self.nmi_unblocking_iret = d[off] != 0; off += 1;
        self.last_exception_type = u32_at(d, &mut off);
        self.smbase = u32_at(d, &mut off);
        self.alignment_check_mask = u32_at(d, &mut off);
        self.a20_mask = u64_at(d, &mut off);

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
    seg.selector.rpl = d[*off]; *off += 1;
    seg.cache.valid = u32_at(d, off);
    seg.cache.p = d[*off] != 0; *off += 1;
    seg.cache.dpl = d[*off]; *off += 1;
    seg.cache.segment = d[*off] != 0; *off += 1;
    seg.cache.r#type = d[*off]; *off += 1;
    seg.cache.u.set_segment_base(u64_at(d, off));
    seg.cache.u.set_segment_limit_scaled(u32_at(d, off));
    seg.cache.u.set_segment_g(d[*off] != 0); *off += 1;
    seg.cache.u.set_segment_d_b(d[*off] != 0); *off += 1;
    seg.cache.u.set_segment_l(d[*off] != 0); *off += 1;
    seg.cache.u.set_segment_avl(d[*off] != 0); *off += 1;
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
    let v = u32::from_le_bytes(d[*off..*off + 4].try_into().unwrap_or_else(|_| unreachable!("slice is exactly 4 bytes")));
    *off += 4;
    v
}

fn u64_at(d: &[u8], off: &mut usize) -> u64 {
    let v = u64::from_le_bytes(d[*off..*off + 8].try_into().unwrap_or_else(|_| unreachable!("slice is exactly 8 bytes")));
    *off += 8;
    v
}

fn i64_at(d: &[u8], off: &mut usize) -> i64 {
    let v = i64::from_le_bytes(d[*off..*off + 8].try_into().unwrap_or_else(|_| unreachable!("slice is exactly 8 bytes")));
    *off += 8;
    v
}
