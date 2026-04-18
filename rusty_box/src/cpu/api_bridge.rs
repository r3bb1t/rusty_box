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
        self.cr4.get()
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
            BX_MSR_PLATFORM_ID => 0,
            BX_MSR_IA32_APERF | BX_MSR_IA32_MPERF => self.get_tsc(self.system_ticks()),
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
            BX_MSR_PLATFORM_ID => return Err(super::CpuError::UnimplementedInstruction), // read-only
            BX_MSR_IA32_APERF | BX_MSR_IA32_MPERF => { /* ignore write */ }
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

    pub(crate) fn translate_linear_with_cr3_for_api(&self, laddr: u64, cr3: u64) -> Option<u64> {
        self.translate_linear_with_cr3(laddr, cr3)
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

    // ── Generic register read/write by enum tag ──
    //
    // Single source of truth for `Emulator::reg_read` / `reg_write` and for
    // `CpuAccess::reg_read` / `reg_write` used by HookCtx. All register width
    // views (64/32/16/8-bit, segments, CRs, DRs, FPU scalars) go through
    // this dispatch.

    pub(crate) fn api_reg_read(&self, reg: X86Reg) -> u64 {
        match reg {
            X86Reg::Rax => self.rax(),
            X86Reg::Rcx => self.rcx(),
            X86Reg::Rdx => self.rdx(),
            X86Reg::Rbx => self.rbx(),
            X86Reg::Rsp => self.rsp(),
            X86Reg::Rbp => self.rbp(),
            X86Reg::Rsi => self.rsi(),
            X86Reg::Rdi => self.rdi(),
            X86Reg::R8 => self.r8(),
            X86Reg::R9 => self.r9(),
            X86Reg::R10 => self.r10(),
            X86Reg::R11 => self.r11(),
            X86Reg::R12 => self.r12(),
            X86Reg::R13 => self.r13(),
            X86Reg::R14 => self.r14(),
            X86Reg::R15 => self.r15(),

            X86Reg::Eax => self.rax() & 0xFFFF_FFFF,
            X86Reg::Ecx => self.rcx() & 0xFFFF_FFFF,
            X86Reg::Edx => self.rdx() & 0xFFFF_FFFF,
            X86Reg::Ebx => self.rbx() & 0xFFFF_FFFF,
            X86Reg::Esp => self.rsp() & 0xFFFF_FFFF,
            X86Reg::Ebp => self.rbp() & 0xFFFF_FFFF,
            X86Reg::Esi => self.rsi() & 0xFFFF_FFFF,
            X86Reg::Edi => self.rdi() & 0xFFFF_FFFF,
            X86Reg::R8d => self.r8() & 0xFFFF_FFFF,
            X86Reg::R9d => self.r9() & 0xFFFF_FFFF,
            X86Reg::R10d => self.r10() & 0xFFFF_FFFF,
            X86Reg::R11d => self.r11() & 0xFFFF_FFFF,
            X86Reg::R12d => self.r12() & 0xFFFF_FFFF,
            X86Reg::R13d => self.r13() & 0xFFFF_FFFF,
            X86Reg::R14d => self.r14() & 0xFFFF_FFFF,
            X86Reg::R15d => self.r15() & 0xFFFF_FFFF,

            X86Reg::Ax => u64::from(self.get_gpr16(0)),
            X86Reg::Cx => u64::from(self.get_gpr16(1)),
            X86Reg::Dx => u64::from(self.get_gpr16(2)),
            X86Reg::Bx => u64::from(self.get_gpr16(3)),
            X86Reg::Sp => u64::from(self.get_gpr16(4)),
            X86Reg::Bp => u64::from(self.get_gpr16(5)),
            X86Reg::Si => u64::from(self.get_gpr16(6)),
            X86Reg::Di => u64::from(self.get_gpr16(7)),
            X86Reg::R8w => u64::from(self.get_gpr16(8)),
            X86Reg::R9w => u64::from(self.get_gpr16(9)),
            X86Reg::R10w => u64::from(self.get_gpr16(10)),
            X86Reg::R11w => u64::from(self.get_gpr16(11)),
            X86Reg::R12w => u64::from(self.get_gpr16(12)),
            X86Reg::R13w => u64::from(self.get_gpr16(13)),
            X86Reg::R14w => u64::from(self.get_gpr16(14)),
            X86Reg::R15w => u64::from(self.get_gpr16(15)),

            X86Reg::Al => u64::from(self.get_gpr8(0)),
            X86Reg::Cl => u64::from(self.get_gpr8(1)),
            X86Reg::Dl => u64::from(self.get_gpr8(2)),
            X86Reg::Bl => u64::from(self.get_gpr8(3)),
            X86Reg::Ah => u64::from(self.get_gpr8(4)),
            X86Reg::Ch => u64::from(self.get_gpr8(5)),
            X86Reg::Dh => u64::from(self.get_gpr8(6)),
            X86Reg::Bh => u64::from(self.get_gpr8(7)),
            X86Reg::Spl => u64::from(self.get_gpr8(4)),
            X86Reg::Bpl => u64::from(self.get_gpr8(5)),
            X86Reg::Sil => u64::from(self.get_gpr8(6)),
            X86Reg::Dil => u64::from(self.get_gpr8(7)),
            X86Reg::R8b => u64::from(self.get_gpr8(8)),
            X86Reg::R9b => u64::from(self.get_gpr8(9)),
            X86Reg::R10b => u64::from(self.get_gpr8(10)),
            X86Reg::R11b => u64::from(self.get_gpr8(11)),
            X86Reg::R12b => u64::from(self.get_gpr8(12)),
            X86Reg::R13b => u64::from(self.get_gpr8(13)),
            X86Reg::R14b => u64::from(self.get_gpr8(14)),
            X86Reg::R15b => u64::from(self.get_gpr8(15)),

            X86Reg::Rip => self.rip(),
            X86Reg::Eip => u64::from(self.eip()),
            X86Reg::Ip => self.rip() & 0xFFFF,
            X86Reg::Rflags => self.rflags_for_api(),
            X86Reg::Eflags => self.rflags_for_api() & 0xFFFF_FFFF,
            X86Reg::Flags => self.rflags_for_api() & 0xFFFF,

            X86Reg::Cs => u64::from(self.get_cs_selector()),
            X86Reg::Ss => u64::from(self.get_ss_selector()),
            X86Reg::Ds => u64::from(self.get_ds_selector()),
            X86Reg::Es => u64::from(self.seg_selector_for_api(0)),
            X86Reg::Fs => u64::from(self.seg_selector_for_api(4)),
            X86Reg::Gs => u64::from(self.seg_selector_for_api(5)),
            X86Reg::FsBase => self.msr_fsbase(),
            X86Reg::GsBase => self.msr_gsbase(),

            X86Reg::Cr0 => u64::from(self.get_cr0_val()),
            X86Reg::Cr2 => self.cr2_for_api(),
            X86Reg::Cr3 => self.get_cr3_val(),
            X86Reg::Cr4 => self.cr4_for_api(),
            X86Reg::Cr8 => self.cr8_for_api(),

            X86Reg::Dr0 => self.dr_for_api(0),
            X86Reg::Dr1 => self.dr_for_api(1),
            X86Reg::Dr2 => self.dr_for_api(2),
            X86Reg::Dr3 => self.dr_for_api(3),
            X86Reg::Dr6 => self.dr6_for_api(),
            X86Reg::Dr7 => self.dr7_for_api(),

            X86Reg::GdtrBase => self.gdtr_base_for_api(),
            X86Reg::GdtrLimit => self.gdtr_limit_for_api(),
            X86Reg::IdtrBase => self.idtr_base_for_api(),
            X86Reg::IdtrLimit => self.idtr_limit_for_api(),
            X86Reg::LdtrSelector => u64::from(self.ldtr_selector_for_api()),
            X86Reg::TrSelector => u64::from(self.tr_selector_for_api()),

            X86Reg::Tsc => self.tsc_for_api(),
            X86Reg::Efer => self.efer_for_api(),

            X86Reg::FpSw => u64::from(self.fpu_sw_for_api()),
            X86Reg::FpCw => u64::from(self.fpu_cw_for_api()),
            X86Reg::FpTag => u64::from(self.fpu_tag_for_api()),
            X86Reg::Mxcsr => u64::from(self.mxcsr_for_api()),
            X86Reg::Opmask0 => self.opmask_read_for_api(0),
            X86Reg::Opmask1 => self.opmask_read_for_api(1),
            X86Reg::Opmask2 => self.opmask_read_for_api(2),
            X86Reg::Opmask3 => self.opmask_read_for_api(3),
            X86Reg::Opmask4 => self.opmask_read_for_api(4),
            X86Reg::Opmask5 => self.opmask_read_for_api(5),
            X86Reg::Opmask6 => self.opmask_read_for_api(6),
            X86Reg::Opmask7 => self.opmask_read_for_api(7),

            // Wide registers handled by dedicated reg_read_{fp80,xmm,ymm,zmm} —
            // scalar path returns 0 as sentinel.
            X86Reg::Fpr0 | X86Reg::Fpr1 | X86Reg::Fpr2 | X86Reg::Fpr3
            | X86Reg::Fpr4 | X86Reg::Fpr5 | X86Reg::Fpr6 | X86Reg::Fpr7
            | X86Reg::Xmm0 | X86Reg::Xmm1 | X86Reg::Xmm2 | X86Reg::Xmm3
            | X86Reg::Xmm4 | X86Reg::Xmm5 | X86Reg::Xmm6 | X86Reg::Xmm7
            | X86Reg::Xmm8 | X86Reg::Xmm9 | X86Reg::Xmm10 | X86Reg::Xmm11
            | X86Reg::Xmm12 | X86Reg::Xmm13 | X86Reg::Xmm14 | X86Reg::Xmm15
            | X86Reg::Ymm0 | X86Reg::Ymm1 | X86Reg::Ymm2 | X86Reg::Ymm3
            | X86Reg::Ymm4 | X86Reg::Ymm5 | X86Reg::Ymm6 | X86Reg::Ymm7
            | X86Reg::Ymm8 | X86Reg::Ymm9 | X86Reg::Ymm10 | X86Reg::Ymm11
            | X86Reg::Ymm12 | X86Reg::Ymm13 | X86Reg::Ymm14 | X86Reg::Ymm15
            | X86Reg::Zmm0 | X86Reg::Zmm1 | X86Reg::Zmm2 | X86Reg::Zmm3
            | X86Reg::Zmm4 | X86Reg::Zmm5 | X86Reg::Zmm6 | X86Reg::Zmm7
            | X86Reg::Zmm8 | X86Reg::Zmm9 | X86Reg::Zmm10 | X86Reg::Zmm11
            | X86Reg::Zmm12 | X86Reg::Zmm13 | X86Reg::Zmm14 | X86Reg::Zmm15
            | X86Reg::Zmm16 | X86Reg::Zmm17 | X86Reg::Zmm18 | X86Reg::Zmm19
            | X86Reg::Zmm20 | X86Reg::Zmm21 | X86Reg::Zmm22 | X86Reg::Zmm23
            | X86Reg::Zmm24 | X86Reg::Zmm25 | X86Reg::Zmm26 | X86Reg::Zmm27
            | X86Reg::Zmm28 | X86Reg::Zmm29 | X86Reg::Zmm30 | X86Reg::Zmm31 => 0,
        }
    }

    pub(crate) fn api_reg_write(&mut self, reg: X86Reg, val: u64) {
        match reg {
            X86Reg::Rax => self.set_rax(val),
            X86Reg::Rcx => self.set_rcx(val),
            X86Reg::Rdx => self.set_rdx(val),
            X86Reg::Rbx => self.set_rbx(val),
            X86Reg::Rsp => self.set_rsp(val),
            X86Reg::Rbp => self.set_rbp(val),
            X86Reg::Rsi => self.set_rsi(val),
            X86Reg::Rdi => self.set_rdi(val),
            X86Reg::R8 => self.set_r8(val),
            X86Reg::R9 => self.set_r9(val),
            X86Reg::R10 => self.set_r10(val),
            X86Reg::R11 => self.set_r11(val),
            X86Reg::R12 => self.set_r12(val),
            X86Reg::R13 => self.set_r13(val),
            X86Reg::R14 => self.set_r14(val),
            X86Reg::R15 => self.set_r15(val),

            // 32-bit writes: x86-64 rule — low 32 replaces, upper 32 zeros.
            // `val & 0xFFFF_FFFF` gives exactly that in u64 form.
            X86Reg::Eax => self.set_rax(val & 0xFFFF_FFFF),
            X86Reg::Ecx => self.set_rcx(val & 0xFFFF_FFFF),
            X86Reg::Edx => self.set_rdx(val & 0xFFFF_FFFF),
            X86Reg::Ebx => self.set_rbx(val & 0xFFFF_FFFF),
            X86Reg::Esp => self.set_rsp(val & 0xFFFF_FFFF),
            X86Reg::Ebp => self.set_rbp(val & 0xFFFF_FFFF),
            X86Reg::Esi => self.set_rsi(val & 0xFFFF_FFFF),
            X86Reg::Edi => self.set_rdi(val & 0xFFFF_FFFF),
            X86Reg::R8d => self.set_r8(val & 0xFFFF_FFFF),
            X86Reg::R9d => self.set_r9(val & 0xFFFF_FFFF),
            X86Reg::R10d => self.set_r10(val & 0xFFFF_FFFF),
            X86Reg::R11d => self.set_r11(val & 0xFFFF_FFFF),
            X86Reg::R12d => self.set_r12(val & 0xFFFF_FFFF),
            X86Reg::R13d => self.set_r13(val & 0xFFFF_FFFF),
            X86Reg::R14d => self.set_r14(val & 0xFFFF_FFFF),
            X86Reg::R15d => self.set_r15(val & 0xFFFF_FFFF),

            // 16-bit writes: low 16 replaces, upper 48 preserved.
            // `set_gpr16` requires u16; truncation here is architectural.
            X86Reg::Ax => self.set_gpr16(0, trunc_u16(val)),
            X86Reg::Cx => self.set_gpr16(1, trunc_u16(val)),
            X86Reg::Dx => self.set_gpr16(2, trunc_u16(val)),
            X86Reg::Bx => self.set_gpr16(3, trunc_u16(val)),
            X86Reg::Sp => self.set_gpr16(4, trunc_u16(val)),
            X86Reg::Bp => self.set_gpr16(5, trunc_u16(val)),
            X86Reg::Si => self.set_gpr16(6, trunc_u16(val)),
            X86Reg::Di => self.set_gpr16(7, trunc_u16(val)),
            X86Reg::R8w => self.set_gpr16(8, trunc_u16(val)),
            X86Reg::R9w => self.set_gpr16(9, trunc_u16(val)),
            X86Reg::R10w => self.set_gpr16(10, trunc_u16(val)),
            X86Reg::R11w => self.set_gpr16(11, trunc_u16(val)),
            X86Reg::R12w => self.set_gpr16(12, trunc_u16(val)),
            X86Reg::R13w => self.set_gpr16(13, trunc_u16(val)),
            X86Reg::R14w => self.set_gpr16(14, trunc_u16(val)),
            X86Reg::R15w => self.set_gpr16(15, trunc_u16(val)),

            X86Reg::Al => self.set_gpr8(0, trunc_u8(val)),
            X86Reg::Cl => self.set_gpr8(1, trunc_u8(val)),
            X86Reg::Dl => self.set_gpr8(2, trunc_u8(val)),
            X86Reg::Bl => self.set_gpr8(3, trunc_u8(val)),
            X86Reg::Ah => self.set_gpr8(4, trunc_u8(val)),
            X86Reg::Ch => self.set_gpr8(5, trunc_u8(val)),
            X86Reg::Dh => self.set_gpr8(6, trunc_u8(val)),
            X86Reg::Bh => self.set_gpr8(7, trunc_u8(val)),
            X86Reg::Spl => self.set_gpr8(4, trunc_u8(val)),
            X86Reg::Bpl => self.set_gpr8(5, trunc_u8(val)),
            X86Reg::Sil => self.set_gpr8(6, trunc_u8(val)),
            X86Reg::Dil => self.set_gpr8(7, trunc_u8(val)),
            X86Reg::R8b => self.set_gpr8(8, trunc_u8(val)),
            X86Reg::R9b => self.set_gpr8(9, trunc_u8(val)),
            X86Reg::R10b => self.set_gpr8(10, trunc_u8(val)),
            X86Reg::R11b => self.set_gpr8(11, trunc_u8(val)),
            X86Reg::R12b => self.set_gpr8(12, trunc_u8(val)),
            X86Reg::R13b => self.set_gpr8(13, trunc_u8(val)),
            X86Reg::R14b => self.set_gpr8(14, trunc_u8(val)),
            X86Reg::R15b => self.set_gpr8(15, trunc_u8(val)),

            X86Reg::Rip => self.set_rip(val),
            X86Reg::Eip => self.set_eip(trunc_u32(val)),
            X86Reg::Ip => {
                // Preserve upper bits of RIP when writing 16-bit IP.
                let upper = self.rip() & !0xFFFF;
                self.set_rip(upper | (val & 0xFFFF));
            }

            X86Reg::Rflags => self.set_rflags_for_api(val),
            X86Reg::Eflags => self.set_rflags_for_api(val & 0xFFFF_FFFF),
            X86Reg::Flags => {
                let preserved = self.rflags_for_api() & !0xFFFF;
                self.set_rflags_for_api(preserved | (val & 0xFFFF));
            }

            X86Reg::Cs | X86Reg::Ss | X86Reg::Ds
            | X86Reg::Es | X86Reg::Fs | X86Reg::Gs => {
                // Raw selector write without descriptor-cache reload. Callers
                // should use `setup_cpu_mode` for correct protected-mode setup.
                self.set_seg_selector_raw_for_api(reg, trunc_u16(val));
            }

            X86Reg::FsBase => self.set_msr_fsbase(val),
            X86Reg::GsBase => self.set_msr_gsbase(val),

            X86Reg::Cr0 => self.set_cr0_raw_for_api(trunc_u32(val)),
            X86Reg::Cr2 => self.set_cr2_for_api(val),
            X86Reg::Cr3 => self.set_cr3_raw_for_api(val),
            X86Reg::Cr4 => self.set_cr4_raw_for_api(trunc_u32(val)),
            X86Reg::Cr8 => self.set_cr8_for_api(val),

            X86Reg::Dr0 => self.set_dr_for_api(0, val),
            X86Reg::Dr1 => self.set_dr_for_api(1, val),
            X86Reg::Dr2 => self.set_dr_for_api(2, val),
            X86Reg::Dr3 => self.set_dr_for_api(3, val),
            X86Reg::Dr6 => self.set_dr6_for_api(val),
            X86Reg::Dr7 => self.set_dr7_for_api(val),

            X86Reg::GdtrBase => self.set_gdtr_base_for_api(val),
            X86Reg::GdtrLimit => self.set_gdtr_limit_for_api(trunc_u32(val)),
            X86Reg::IdtrBase => self.set_idtr_base_for_api(val),
            X86Reg::IdtrLimit => self.set_idtr_limit_for_api(trunc_u32(val)),
            X86Reg::LdtrSelector => self.set_ldtr_selector_for_api(trunc_u16(val)),
            X86Reg::TrSelector => self.set_tr_selector_for_api(trunc_u16(val)),

            X86Reg::Tsc => self.set_tsc_for_api(val),
            X86Reg::Efer => self.set_efer_for_api(val),

            X86Reg::FpSw => self.set_fpu_sw_for_api(trunc_u16(val)),
            X86Reg::FpCw => self.set_fpu_cw_for_api(trunc_u16(val)),
            X86Reg::FpTag => self.set_fpu_tag_for_api(trunc_u16(val)),
            X86Reg::Mxcsr => self.set_mxcsr_for_api(trunc_u32(val)),
            X86Reg::Opmask0 => self.opmask_write_for_api(0, val),
            X86Reg::Opmask1 => self.opmask_write_for_api(1, val),
            X86Reg::Opmask2 => self.opmask_write_for_api(2, val),
            X86Reg::Opmask3 => self.opmask_write_for_api(3, val),
            X86Reg::Opmask4 => self.opmask_write_for_api(4, val),
            X86Reg::Opmask5 => self.opmask_write_for_api(5, val),
            X86Reg::Opmask6 => self.opmask_write_for_api(6, val),
            X86Reg::Opmask7 => self.opmask_write_for_api(7, val),

            // Wide registers — use dedicated reg_write_{fp80,xmm,ymm,zmm}.
            // Scalar path ignores.
            X86Reg::Fpr0 | X86Reg::Fpr1 | X86Reg::Fpr2 | X86Reg::Fpr3
            | X86Reg::Fpr4 | X86Reg::Fpr5 | X86Reg::Fpr6 | X86Reg::Fpr7
            | X86Reg::Xmm0 | X86Reg::Xmm1 | X86Reg::Xmm2 | X86Reg::Xmm3
            | X86Reg::Xmm4 | X86Reg::Xmm5 | X86Reg::Xmm6 | X86Reg::Xmm7
            | X86Reg::Xmm8 | X86Reg::Xmm9 | X86Reg::Xmm10 | X86Reg::Xmm11
            | X86Reg::Xmm12 | X86Reg::Xmm13 | X86Reg::Xmm14 | X86Reg::Xmm15
            | X86Reg::Ymm0 | X86Reg::Ymm1 | X86Reg::Ymm2 | X86Reg::Ymm3
            | X86Reg::Ymm4 | X86Reg::Ymm5 | X86Reg::Ymm6 | X86Reg::Ymm7
            | X86Reg::Ymm8 | X86Reg::Ymm9 | X86Reg::Ymm10 | X86Reg::Ymm11
            | X86Reg::Ymm12 | X86Reg::Ymm13 | X86Reg::Ymm14 | X86Reg::Ymm15
            | X86Reg::Zmm0 | X86Reg::Zmm1 | X86Reg::Zmm2 | X86Reg::Zmm3
            | X86Reg::Zmm4 | X86Reg::Zmm5 | X86Reg::Zmm6 | X86Reg::Zmm7
            | X86Reg::Zmm8 | X86Reg::Zmm9 | X86Reg::Zmm10 | X86Reg::Zmm11
            | X86Reg::Zmm12 | X86Reg::Zmm13 | X86Reg::Zmm14 | X86Reg::Zmm15
            | X86Reg::Zmm16 | X86Reg::Zmm17 | X86Reg::Zmm18 | X86Reg::Zmm19
            | X86Reg::Zmm20 | X86Reg::Zmm21 | X86Reg::Zmm22 | X86Reg::Zmm23
            | X86Reg::Zmm24 | X86Reg::Zmm25 | X86Reg::Zmm26 | X86Reg::Zmm27
            | X86Reg::Zmm28 | X86Reg::Zmm29 | X86Reg::Zmm30 | X86Reg::Zmm31 => {}
        }
    }
}

// Explicit truncation helpers. The only place in the CPU API surface where
// `as` is used — it's architectural (x86 register writes silently truncate)
// and concentrating it in named helpers makes every callsite self-documenting.
#[inline] fn trunc_u16(v: u64) -> u16 { (v & 0xFFFF) as u16 }
#[inline] fn trunc_u8(v: u64) -> u8 { (v & 0xFF) as u8 }
#[inline] fn trunc_u32(v: u64) -> u32 { (v & 0xFFFF_FFFF) as u32 }

// ─────────────────────────── CpuAccess impl for HookCtx ───────────────────────────
//
// `CpuAccess` is the type-erased view of the CPU passed into hook callbacks
// via `HookCtx`. Delegates reg_read / reg_write to the `api_reg_*` dispatch
// above — one source of truth for every Rust client of the CPU.

use crate::cpu::instrumentation::CpuAccess;

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> CpuAccess for BxCpuC<'_, I, T> {
    fn reg_read(&self, reg: X86Reg) -> u64 {
        self.api_reg_read(reg)
    }

    fn reg_write(&mut self, reg: X86Reg, val: u64) {
        self.api_reg_write(reg, val)
    }

    fn mem_read(&self, addr: u64, buf: &mut [u8]) -> bool {
        for (i, slot) in buf.iter_mut().enumerate() {
            *slot = self.mem_read_byte(addr + i as u64);
        }
        true
    }

    fn mem_write(&mut self, addr: u64, data: &[u8]) -> bool {
        for (i, &byte) in data.iter().enumerate() {
            self.mem_write_byte(addr + i as u64, byte);
        }
        true
    }

    fn virt_read(&self, vaddr: u64, buf: &mut [u8]) -> bool {
        virt_read_chunked(buf, vaddr, |va| self.translate_linear_for_diag(va),
                          |pa| self.mem_read_byte(pa))
    }

    fn virt_read_with_cr3(&self, vaddr: u64, cr3: u64, buf: &mut [u8]) -> bool {
        virt_read_chunked(buf, vaddr, |va| self.translate_linear_with_cr3(va, cr3),
                          |pa| self.mem_read_byte(pa))
    }

    fn stop(&mut self) {
        self.instrumentation.stop_request = true;
    }

    fn rip(&self) -> u64 { BxCpuC::rip(self) }
    fn icount(&self) -> u64 { self.icount }
    fn cr3(&self) -> u64 { self.cr3 }
}

/// Page-aware chunked virtual memory read. Walks `buf` page-by-page, calling
/// `translate(va) -> Option<pa>` once per page and `read_byte(pa) -> u8` for
/// each byte. Returns `true` on success, `false` if any page translation
/// fails (partial read leaves earlier pages populated).
fn virt_read_chunked<Tr, Rd>(
    buf: &mut [u8],
    start_va: u64,
    translate: Tr,
    read_byte: Rd,
) -> bool
where
    Tr: Fn(u64) -> Option<u64>,
    Rd: Fn(u64) -> u8,
{
    let mut off: usize = 0;
    while off < buf.len() {
        let va = start_va.wrapping_add(off as u64);
        let page_off = usize::try_from(va & 0xFFF).expect("page offset fits usize");
        let chunk = (0x1000 - page_off).min(buf.len() - off);
        let Some(pa) = translate(va) else { return false; };
        for i in 0..chunk {
            buf[off + i] = read_byte(pa + i as u64);
        }
        off += chunk;
    }
    true
}

// ─────────────────────────── pre_* hook firing ───────────────────────────
//
// These methods live on BxCpuC (not on the registry) because they need to
// build a `HookCtx` wrapping `&mut BxCpuC` while simultaneously calling into
// the tracer. We split-borrow by `take()`-ing the tracer out of the registry
// Option slot, running the hook, then putting it back. `None` is visible
// only during the hook call — user code can't observe it.

#[cfg(feature = "instrumentation")]
use crate::cpu::instrumentation::{HookCtx, InstrAction};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Fire the `pre_syscall` trait hook. Called from `syscall()` /
    /// `sysenter()` BEFORE the architectural CS/RIP transition. The hook
    /// returns an `InstrAction` which the caller inspects to decide whether
    /// to execute the transition, skip it, stop the loop, or both.
    #[cfg(feature = "instrumentation")]
    pub(crate) fn fire_pre_syscall(&mut self) -> InstrAction {
        let Some(mut tracer) = self.instrumentation.tracer.take() else {
            return InstrAction::Continue;
        };
        let action = {
            let mut ctx = HookCtx::new(self);
            tracer.pre_syscall(&mut ctx)
        };
        self.instrumentation.tracer = Some(tracer);
        action
    }
}