//! Small bridge layer exposing BxCpuC internals to the public Emulator API
//! without widening the `pub(super)` surface of every helper that already
//! exists. Everything here is `pub(crate)` so only `emulator_api.rs` can see
//! it.
//!
//! Grouped by the public API that consumes it:
//! - `Emulator::reg_read` / `reg_write`
//! - `Emulator::msr_read` / `msr_write`
//! - `Emulator::cpu_snapshot`
//! - `Emulator::setup_cpu_mode` (and friends)

use super::{BxCpuC, BxCpuIdTrait};
use super::decoder::BxSegregs;
use super::instrumentation::X86Reg;

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ── RFLAGS / EFLAGS ────────────────────────────────────────────────

    #[inline]
    pub(crate) fn rflags_for_api(&self) -> u64 {
        self.eflags.bits() as u64
    }

    #[inline]
    pub(crate) fn set_rflags_for_api(&mut self, v: u64) {
        self.eflags = super::eflags::EFlags::from_bits_retain(v as u32);
    }

    // ── Segment selectors (raw) ────────────────────────────────────────

    /// Read segment selector by BxSegregs index.
    /// index map: 0=ES, 1=CS, 2=SS, 3=DS, 4=FS, 5=GS.
    #[inline]
    pub(crate) fn seg_selector_for_api(&self, seg_index: usize) -> u16 {
        self.sregs[seg_index].selector.value
    }

    /// Raw selector write. Does NOT update the descriptor cache — callers
    /// using protected-mode setup should use `set_seg_for_api` instead.
    pub(crate) fn set_seg_selector_raw_for_api(&mut self, reg: X86Reg, val: u16) {
        let idx = match reg {
            X86Reg::Es => BxSegregs::Es as usize,
            X86Reg::Cs => BxSegregs::Cs as usize,
            X86Reg::Ss => BxSegregs::Ss as usize,
            X86Reg::Ds => BxSegregs::Ds as usize,
            X86Reg::Fs => BxSegregs::Fs as usize,
            X86Reg::Gs => BxSegregs::Gs as usize,
            _ => return,
        };
        self.sregs[idx].selector.value = val;
        self.sregs[idx].selector.rpl = (val & 0x3) as u8;
        self.sregs[idx].selector.ti = ((val >> 2) & 1) as u16;
        self.sregs[idx].selector.index = val >> 3;
    }

    /// Set segment to a flat-cache state used by `CpuSetupMode::*`.
    /// Writes both selector and descriptor cache so later instructions see
    /// a valid, flat segment without needing a GDT reload.
    pub(crate) fn set_seg_for_api(
        &mut self,
        reg: X86Reg,
        selector: u16,
        base: u64,
        limit: u32,
        code16: bool,
        long: bool,
    ) {
        let idx = match reg {
            X86Reg::Es => BxSegregs::Es as usize,
            X86Reg::Cs => BxSegregs::Cs as usize,
            X86Reg::Ss => BxSegregs::Ss as usize,
            X86Reg::Ds => BxSegregs::Ds as usize,
            X86Reg::Fs => BxSegregs::Fs as usize,
            X86Reg::Gs => BxSegregs::Gs as usize,
            _ => return,
        };
        let s = &mut self.sregs[idx];
        s.selector.value = selector;
        s.selector.rpl = (selector & 0x3) as u8;
        s.selector.ti = ((selector >> 2) & 1) as u16;
        s.selector.index = selector >> 3;
        s.cache.valid = super::descriptor::SEG_VALID_CACHE;
        s.cache.segment = true;
        s.cache.p = true;
        // DPL follows RPL for the flat-setup path.
        s.cache.dpl = s.selector.rpl;
        // type: code=0xB (exec/read/accessed), data=0x3 (read/write/accessed)
        s.cache.r#type = if matches!(reg, X86Reg::Cs) { 0xB } else { 0x3 };
        s.cache.u.set_segment_base(base);
        s.cache.u.set_segment_limit_scaled(limit);
        s.cache.u.set_segment_g(true);
        s.cache.u.set_segment_d_b(!code16 && !long);
        s.cache.u.set_segment_l(long);
        s.cache.u.set_segment_avl(false);
    }

    /// Enable CR0.PE and update fetch mode / alignment state.
    pub(crate) fn enter_protected_mode_for_api(&mut self) {
        let cr0 = self.cr0.get32();
        self.cr0.set32(cr0 | 0x1); // PE
        self.handle_alignment_check();
        self.handle_cpu_mode_change();
        self.update_fetch_mode_mask();
    }

    /// Enable CR0.PE+PG, CR4.PAE, EFER.LME+LMA, set CR3, and refresh mode.
    pub(crate) fn enter_long_mode_for_api(&mut self, cr3: u64) {
        // CR4.PAE = 1
        let cr4 = self.cr4.get32();
        self.cr4.set32(cr4 | (1 << 5));
        // CR3 = page table base
        self.cr3 = cr3;
        // EFER.LME=1, LMA=1
        self.efer.set_lme(1);
        self.efer.set_lma(1);
        // CR0.PE=1, PG=1
        let cr0 = self.cr0.get32();
        self.cr0.set32(cr0 | 0x8000_0001);
        self.linaddr_width = if self.cr4.la57() { 57 } else { 48 };
        self.handle_alignment_check();
        self.handle_cpu_mode_change();
        self.update_fetch_mode_mask();
        // Flush stale TLBs from the prior (16/32-bit) mapping.
        self.tlb_flush();
    }

    // ── CR2 / CR4 / CR8 ────────────────────────────────────────────────

    #[inline]
    pub(crate) fn cr2_for_api(&self) -> u64 {
        self.cr2
    }

    #[inline]
    pub(crate) fn set_cr2_for_api(&mut self, v: u64) {
        self.cr2 = v;
    }

    #[inline]
    pub(crate) fn cr4_for_api(&self) -> u64 {
        self.cr4.get32() as u64
    }

    #[inline]
    pub(crate) fn set_cr4_raw_for_api(&mut self, v: u32) {
        self.cr4.set32(v);
    }

    /// CR8 is not modeled as a dedicated field — it's sourced from the
    /// task priority register on the local APIC. Return the LAPIC TPR >> 4.
    #[inline]
    pub(crate) fn cr8_for_api(&self) -> u64 {
        {
            (self.lapic.get_tpr() as u64) >> 4
        }
    }

    #[inline]
    pub(crate) fn set_cr8_for_api(&mut self, v: u64) {
        {
            self.lapic.set_tpr(((v & 0xF) << 4) as u8);
        }
    }

    // ── CR0 / CR3 raw writes (for `reg_write`) ─────────────────────────

    /// Write CR0 without BOCHS-level checks. Used by `reg_write` where
    /// the caller has taken responsibility for validity.
    #[inline]
    pub(crate) fn set_cr0_raw_for_api(&mut self, v: u32) {
        self.cr0.set32(v);
        self.handle_alignment_check();
        self.handle_cpu_mode_change();
        self.update_fetch_mode_mask();
    }

    #[inline]
    pub(crate) fn set_cr3_raw_for_api(&mut self, v: u64) {
        self.cr3 = v;
        self.tlb_flush();
    }

    // ── Debug registers ────────────────────────────────────────────────

    #[inline]
    pub(crate) fn dr_for_api(&self, idx: usize) -> u64 {
        self.dr[idx] as u64
    }

    #[inline]
    pub(crate) fn set_dr_for_api(&mut self, idx: usize, v: u64) {
        self.dr[idx] = v;
    }

    #[inline]
    pub(crate) fn dr6_for_api(&self) -> u64 {
        self.dr6.get32() as u64
    }

    #[inline]
    pub(crate) fn set_dr6_for_api(&mut self, v: u64) {
        self.dr6.set32(v as u32);
    }

    #[inline]
    pub(crate) fn dr7_for_api(&self) -> u64 {
        self.dr7.get32() as u64
    }

    #[inline]
    pub(crate) fn set_dr7_for_api(&mut self, v: u64) {
        self.dr7.set32(v as u32);
    }

    // ── Descriptor tables ─────────────────────────────────────────────

    #[inline]
    pub(crate) fn gdtr_base_for_api(&self) -> u64 {
        self.gdtr.base
    }
    #[inline]
    pub(crate) fn set_gdtr_base_for_api(&mut self, v: u64) {
        self.gdtr.base = v;
    }
    #[inline]
    pub(crate) fn gdtr_limit_for_api(&self) -> u64 {
        self.gdtr.limit as u64
    }
    #[inline]
    pub(crate) fn set_gdtr_limit_for_api(&mut self, v: u32) {
        self.gdtr.limit = v as u16;
    }

    #[inline]
    pub(crate) fn idtr_base_for_api(&self) -> u64 {
        self.idtr.base
    }
    #[inline]
    pub(crate) fn set_idtr_base_for_api(&mut self, v: u64) {
        self.idtr.base = v;
    }
    #[inline]
    pub(crate) fn idtr_limit_for_api(&self) -> u64 {
        self.idtr.limit as u64
    }
    #[inline]
    pub(crate) fn set_idtr_limit_for_api(&mut self, v: u32) {
        self.idtr.limit = v as u16;
    }

    #[inline]
    pub(crate) fn ldtr_selector_for_api(&self) -> u16 {
        self.ldtr.selector.value
    }
    #[inline]
    pub(crate) fn set_ldtr_selector_for_api(&mut self, v: u16) {
        self.ldtr.selector.value = v;
    }

    #[inline]
    pub(crate) fn tr_selector_for_api(&self) -> u16 {
        self.tr.selector.value
    }
    #[inline]
    pub(crate) fn set_tr_selector_for_api(&mut self, v: u16) {
        self.tr.selector.value = v;
    }

    // ── TSC ────────────────────────────────────────────────────────────

    #[inline]
    pub(crate) fn tsc_for_api(&self) -> u64 {
        self.get_tsc(self.system_ticks())
    }
    #[inline]
    pub(crate) fn set_tsc_for_api(&mut self, v: u64) {
        let t = self.system_ticks();
        self.set_tsc(v, t);
    }

    // ── EFER ───────────────────────────────────────────────────────────

    #[inline]
    pub(crate) fn efer_for_api(&self) -> u64 {
        self.efer.get32() as u64
    }
    #[inline]
    pub(crate) fn set_efer_for_api(&mut self, v: u64) {
        self.efer.set32(v as u32);
    }

    // ── CPL / icount ──────────────────────────────────────────────────

    #[inline]
    pub(crate) fn cpl_for_api(&self) -> u8 {
        self.sregs[BxSegregs::Cs as usize].selector.rpl
    }

    #[inline]
    pub(crate) fn icount_for_api(&self) -> u64 {
        self.icount
    }

    // ── Generic MSR bridge ────────────────────────────────────────────

    /// Read an MSR by index. For the first iteration we cover the
    /// commonly-used set (SYSENTER, STAR/LSTAR/CSTAR/FMASK, KERNELGSBASE,
    /// TSC_AUX, EFER, APICBASE, IA32_XSS, PAT, plus MTRRs); returns
    /// `Err(UnimplementedInstruction)` for unknown MSRs.
    pub(crate) fn read_msr_for_api(&self, msr: u32) -> super::Result<u64> {
        use super::msr::*;
        let apicbase = self.msr.apicbase as u64;
        let v = match msr {
            BX_MSR_TSC => self.get_tsc(self.system_ticks()),
            BX_MSR_APICBASE => apicbase,
            BX_MSR_SYSENTER_CS => self.msr.sysenter_cs_msr as u64,
            BX_MSR_SYSENTER_ESP => self.msr.sysenter_esp_msr,
            BX_MSR_SYSENTER_EIP => self.msr.sysenter_eip_msr,
            BX_MSR_STAR => self.msr.star,
            BX_MSR_LSTAR => self.msr.lstar,
            BX_MSR_CSTAR => self.msr.cstar,
            BX_MSR_FMASK => self.msr.fmask as u64,
            BX_MSR_KERNELGSBASE => self.msr.kernelgsbase,
            BX_MSR_TSC_AUX => self.msr.tsc_aux as u64,
            BX_MSR_EFER => self.efer.get32() as u64,
            BX_MSR_FSBASE => self.msr_fsbase(),
            BX_MSR_GSBASE => self.msr_gsbase(),
            _ => return Err(super::CpuError::UnimplementedInstruction),
        };
        Ok(v)
    }

    /// Write an MSR by index. Validates per-MSR rules like the in-CPU path;
    /// returns `Err(UnimplementedInstruction)` for unknown MSRs.
    pub(crate) fn write_msr_for_api(&mut self, msr: u32, val: u64) -> super::Result<()> {
        use super::msr::*;
        match msr {
            BX_MSR_TSC => {
                let t = self.system_ticks();
                self.set_tsc(val, t);
            }
            BX_MSR_APICBASE => {
                self.msr.apicbase = val as _;
            }
            BX_MSR_SYSENTER_CS => self.msr.sysenter_cs_msr = val as u32,
            BX_MSR_SYSENTER_ESP => self.msr.sysenter_esp_msr = val,
            BX_MSR_SYSENTER_EIP => self.msr.sysenter_eip_msr = val,
            BX_MSR_STAR => self.msr.star = val,
            BX_MSR_LSTAR => self.msr.lstar = val,
            BX_MSR_CSTAR => self.msr.cstar = val,
            BX_MSR_FMASK => self.msr.fmask = val as u32,
            BX_MSR_KERNELGSBASE => self.msr.kernelgsbase = val,
            BX_MSR_TSC_AUX => self.msr.tsc_aux = val as u32,
            BX_MSR_EFER => self.efer.set32(val as u32),
            BX_MSR_FSBASE => self.set_msr_fsbase(val),
            BX_MSR_GSBASE => self.set_msr_gsbase(val),
            _ => return Err(super::CpuError::UnimplementedInstruction),
        }
        Ok(())
    }

    /// Translate a linear (virtual) address to physical using current page tables.
    /// Returns Err if the translation faults (page not present, protection violation).
    pub(crate) fn translate_linear_for_api(&self, laddr: u64) -> super::Result<u64> {
        self.translate_linear_system_read(laddr)
    }

    // ── FPU read/write ─────────────────────────────────────────────

    /// Read an x87 FPU register as 10 raw bytes (80-bit extended precision).
    /// `index` is 0-7 (ST(0) through ST(7), physical index = (tos + index) & 7).
    pub(crate) fn fpu_read_st(&self, index: usize) -> [u8; 10] {
        let phys = (self.the_i387.tos as usize + index) & 7;
        let reg = self.the_i387.st_space[phys];
        let mut buf = [0u8; 10];
        buf[..8].copy_from_slice(&reg.signif.to_le_bytes());
        buf[8..10].copy_from_slice(&reg.sign_exp.to_le_bytes());
        buf
    }

    /// Write an x87 FPU register from 10 raw bytes.
    pub(crate) fn fpu_write_st(&mut self, index: usize, val: [u8; 10]) {
        use super::softfloat3e::softfloat_types::floatx80;
        let phys = (self.the_i387.tos as usize + index) & 7;
        let signif = u64::from_le_bytes(val[..8].try_into().unwrap());
        let sign_exp = u16::from_le_bytes(val[8..10].try_into().unwrap());
        self.the_i387.st_space[phys] = floatx80 { signif, sign_exp };
    }

    // ── XMM/YMM/ZMM read/write ─────────────────────────────────────

    pub(crate) fn xmm_read_for_api(&self, index: usize) -> [u8; 16] {
        self.vmm[index].zmm128(0).bytes
    }

    pub(crate) fn xmm_write_for_api(&mut self, index: usize, val: [u8; 16]) {
        use super::xmm::BxPackedXmmRegister;
        let xmm = BxPackedXmmRegister { bytes: val };
        self.vmm[index].set_zmm128(0, xmm);
    }

    pub(crate) fn ymm_read_for_api(&self, index: usize) -> [u8; 32] {
        self.vmm[index].zmm256(0).bytes
    }

    pub(crate) fn ymm_write_for_api(&mut self, index: usize, val: [u8; 32]) {
        use super::xmm::BxPackedYmmRegister;
        let ymm = BxPackedYmmRegister { bytes: val };
        // Clear upper 256 bits, then set lower 256
        self.vmm[index].clear();
        self.vmm[index].set_zmm256(0, ymm);
    }

    pub(crate) fn zmm_read_for_api(&self, index: usize) -> [u8; 64] {
        self.vmm[index].bytes
    }

    pub(crate) fn zmm_write_for_api(&mut self, index: usize, val: [u8; 64]) {
        self.vmm[index].bytes = val;
    }

    pub(crate) fn opmask_read_for_api(&self, index: usize) -> u64 {
        self.opmask[index].rrx()
    }

    pub(crate) fn opmask_write_for_api(&mut self, index: usize, val: u64) {
        self.opmask[index].set_rrx(val);
    }

    // ── FPU scalar register access ───────────────────────────────────

    pub(crate) fn fpu_sw_for_api(&self) -> u16 { self.the_i387.swd }
    pub(crate) fn set_fpu_sw_for_api(&mut self, v: u16) { self.the_i387.swd = v; }
    pub(crate) fn fpu_cw_for_api(&self) -> u16 { self.the_i387.cwd }
    pub(crate) fn set_fpu_cw_for_api(&mut self, v: u16) { self.the_i387.cwd = v; }
    pub(crate) fn fpu_tag_for_api(&self) -> u16 { self.the_i387.twd }
    pub(crate) fn set_fpu_tag_for_api(&mut self, v: u16) { self.the_i387.twd = v; }
    pub(crate) fn mxcsr_for_api(&self) -> u32 { self.mxcsr.mxcsr }
    pub(crate) fn set_mxcsr_for_api(&mut self, v: u32) { self.mxcsr.mxcsr = v; }
}