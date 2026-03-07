//! System Management Mode (SMM) entry/exit
//! Matching Bochs cpu/smm.cc
//!
//! SMM is entered via SMI (System Management Interrupt) and exited via RSM.
//! The CPU saves its entire state to SMRAM at smbase + 0x10000, then enters
//! a special real-mode-like execution environment at smbase + 0x8000.

use super::{
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    descriptor::{
        SEG_ACCESS_ROK, SEG_ACCESS_ROK4_G, SEG_ACCESS_WOK, SEG_ACCESS_WOK4_G, SEG_VALID_CACHE,
    },
    BxCpuC, CpuError, Result,
};

const SMM_SAVE_STATE_MAP_SIZE: u32 = 128;

/// SMM revision ID: indicates 32-bit state save format
/// Bit 17 = SMBASE relocation supported
const SMM_REVISION_ID: u32 = 0x00020000; // SMBASE relocation

/// Number of dwords in the SMRAM save state area
const SMRAM_STATE_SIZE: usize = 128;

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
pub(super) enum SMMRAM_Fields {
    SMRAM_FIELD_SMBASE_OFFSET = 0,
    SMRAM_FIELD_SMM_REVISION_ID,
    SMRAM_FIELD_RAX_HI32,
    SMRAM_FIELD_EAX,
    SMRAM_FIELD_RCX_HI32,
    SMRAM_FIELD_ECX,
    SMRAM_FIELD_RDX_HI32,
    SMRAM_FIELD_EDX,
    SMRAM_FIELD_RBX_HI32,
    SMRAM_FIELD_EBX,
    SMRAM_FIELD_RSP_HI32,
    SMRAM_FIELD_ESP,
    SMRAM_FIELD_RBP_HI32,
    SMRAM_FIELD_EBP,
    SMRAM_FIELD_RSI_HI32,
    SMRAM_FIELD_ESI,
    SMRAM_FIELD_RDI_HI32,
    SMRAM_FIELD_EDI,
    SMRAM_FIELD_R8_HI32,
    SMRAM_FIELD_R8,
    SMRAM_FIELD_R9_HI32,
    SMRAM_FIELD_R9,
    SMRAM_FIELD_R10_HI32,
    SMRAM_FIELD_R10,
    SMRAM_FIELD_R11_HI32,
    SMRAM_FIELD_R11,
    SMRAM_FIELD_R12_HI32,
    SMRAM_FIELD_R12,
    SMRAM_FIELD_R13_HI32,
    SMRAM_FIELD_R13,
    SMRAM_FIELD_R14_HI32,
    SMRAM_FIELD_R14,
    SMRAM_FIELD_R15_HI32,
    SMRAM_FIELD_R15,
    SMRAM_FIELD_RIP_HI32,
    SMRAM_FIELD_EIP,
    SMRAM_FIELD_RFLAGS_HI32, // always zero
    SMRAM_FIELD_EFLAGS,
    SMRAM_FIELD_DR6_HI32, // always zero
    SMRAM_FIELD_DR6,
    SMRAM_FIELD_DR7_HI32, // always zero
    SMRAM_FIELD_DR7,
    SMRAM_FIELD_CR0_HI32, // always zero
    SMRAM_FIELD_CR0,
    SMRAM_FIELD_CR3_HI32, // zero when physical address size 32-bit
    SMRAM_FIELD_CR3,
    SMRAM_FIELD_CR4_HI32, // always zero
    SMRAM_FIELD_CR4,
    SMRAM_FIELD_EFER_HI32, // always zero
    SMRAM_FIELD_EFER,
    SMRAM_FIELD_IO_INSTRUCTION_RESTART,
    SMRAM_FIELD_AUTOHALT_RESTART,
    SMRAM_FIELD_NMI_MASK,
    SMRAM_FIELD_SSP_HI32,
    SMRAM_FIELD_SSP,
    SMRAM_FIELD_TR_BASE_HI32,
    SMRAM_FIELD_TR_BASE,
    SMRAM_FIELD_TR_LIMIT,
    SMRAM_FIELD_TR_SELECTOR_AR,
    SMRAM_FIELD_LDTR_BASE_HI32,
    SMRAM_FIELD_LDTR_BASE,
    SMRAM_FIELD_LDTR_LIMIT,
    SMRAM_FIELD_LDTR_SELECTOR_AR,
    SMRAM_FIELD_IDTR_BASE_HI32,
    SMRAM_FIELD_IDTR_BASE,
    SMRAM_FIELD_IDTR_LIMIT,
    SMRAM_FIELD_GDTR_BASE_HI32,
    SMRAM_FIELD_GDTR_BASE,
    SMRAM_FIELD_GDTR_LIMIT,
    SMRAM_FIELD_ES_BASE_HI32,
    SMRAM_FIELD_ES_BASE,
    SMRAM_FIELD_ES_LIMIT,
    SMRAM_FIELD_ES_SELECTOR_AR,
    SMRAM_FIELD_CS_BASE_HI32,
    SMRAM_FIELD_CS_BASE,
    SMRAM_FIELD_CS_LIMIT,
    SMRAM_FIELD_CS_SELECTOR_AR,
    SMRAM_FIELD_SS_BASE_HI32,
    SMRAM_FIELD_SS_BASE,
    SMRAM_FIELD_SS_LIMIT,
    SMRAM_FIELD_SS_SELECTOR_AR,
    SMRAM_FIELD_DS_BASE_HI32,
    SMRAM_FIELD_DS_BASE,
    SMRAM_FIELD_DS_LIMIT,
    SMRAM_FIELD_DS_SELECTOR_AR,
    SMRAM_FIELD_FS_BASE_HI32,
    SMRAM_FIELD_FS_BASE,
    SMRAM_FIELD_FS_LIMIT,
    SMRAM_FIELD_FS_SELECTOR_AR,
    SMRAM_FIELD_GS_BASE_HI32,
    SMRAM_FIELD_GS_BASE,
    SMRAM_FIELD_GS_LIMIT,
    SMRAM_FIELD_GS_SELECTOR_AR,
    SMRAM_FIELD_LAST,
}

use SMMRAM_Fields::*;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn init_smram() -> Result<[u32; SMRAM_FIELD_LAST as _]> {
        let mut smram_map = [0; SMRAM_FIELD_LAST as _];
        smram_map[SMRAM_FIELD_SMBASE_OFFSET as usize] = smram_translate(0x7f00);
        smram_map[SMRAM_FIELD_SMM_REVISION_ID as usize] = smram_translate(0x7efc);
        smram_map[SMRAM_FIELD_RAX_HI32 as usize] = smram_translate(0x7ffc);
        smram_map[SMRAM_FIELD_EAX as usize] = smram_translate(0x7ff8);
        smram_map[SMRAM_FIELD_RCX_HI32 as usize] = smram_translate(0x7ff4);
        smram_map[SMRAM_FIELD_ECX as usize] = smram_translate(0x7ff0);
        smram_map[SMRAM_FIELD_RDX_HI32 as usize] = smram_translate(0x7fec);
        smram_map[SMRAM_FIELD_EDX as usize] = smram_translate(0x7fe8);
        smram_map[SMRAM_FIELD_RBX_HI32 as usize] = smram_translate(0x7fe4);
        smram_map[SMRAM_FIELD_EBX as usize] = smram_translate(0x7fe0);
        smram_map[SMRAM_FIELD_RSP_HI32 as usize] = smram_translate(0x7fdc);
        smram_map[SMRAM_FIELD_ESP as usize] = smram_translate(0x7fd8);
        smram_map[SMRAM_FIELD_RBP_HI32 as usize] = smram_translate(0x7fd4);
        smram_map[SMRAM_FIELD_EBP as usize] = smram_translate(0x7fd0);
        smram_map[SMRAM_FIELD_RSI_HI32 as usize] = smram_translate(0x7fcc);
        smram_map[SMRAM_FIELD_ESI as usize] = smram_translate(0x7fc8);
        smram_map[SMRAM_FIELD_RDI_HI32 as usize] = smram_translate(0x7fc4);
        smram_map[SMRAM_FIELD_EDI as usize] = smram_translate(0x7fc0);
        smram_map[SMRAM_FIELD_R8_HI32 as usize] = smram_translate(0x7fbc);
        smram_map[SMRAM_FIELD_R8 as usize] = smram_translate(0x7fb8);
        smram_map[SMRAM_FIELD_R9_HI32 as usize] = smram_translate(0x7fb4);
        smram_map[SMRAM_FIELD_R9 as usize] = smram_translate(0x7fb0);
        smram_map[SMRAM_FIELD_R10_HI32 as usize] = smram_translate(0x7fac);
        smram_map[SMRAM_FIELD_R10 as usize] = smram_translate(0x7fa8);
        smram_map[SMRAM_FIELD_R11_HI32 as usize] = smram_translate(0x7fa4);
        smram_map[SMRAM_FIELD_R11 as usize] = smram_translate(0x7fa0);
        smram_map[SMRAM_FIELD_R12_HI32 as usize] = smram_translate(0x7f9c);
        smram_map[SMRAM_FIELD_R12 as usize] = smram_translate(0x7f98);
        smram_map[SMRAM_FIELD_R13_HI32 as usize] = smram_translate(0x7f94);
        smram_map[SMRAM_FIELD_R13 as usize] = smram_translate(0x7f90);
        smram_map[SMRAM_FIELD_R14_HI32 as usize] = smram_translate(0x7f8c);
        smram_map[SMRAM_FIELD_R14 as usize] = smram_translate(0x7f88);
        smram_map[SMRAM_FIELD_R15_HI32 as usize] = smram_translate(0x7f84);
        smram_map[SMRAM_FIELD_R15 as usize] = smram_translate(0x7f80);
        smram_map[SMRAM_FIELD_RIP_HI32 as usize] = smram_translate(0x7f7c);
        smram_map[SMRAM_FIELD_EIP as usize] = smram_translate(0x7f78);
        smram_map[SMRAM_FIELD_RFLAGS_HI32 as usize] = smram_translate(0x7f74);
        smram_map[SMRAM_FIELD_EFLAGS as usize] = smram_translate(0x7f70);
        smram_map[SMRAM_FIELD_DR6_HI32 as usize] = smram_translate(0x7f6c);
        smram_map[SMRAM_FIELD_DR6 as usize] = smram_translate(0x7f68);
        smram_map[SMRAM_FIELD_DR7_HI32 as usize] = smram_translate(0x7f64);
        smram_map[SMRAM_FIELD_DR7 as usize] = smram_translate(0x7f60);
        smram_map[SMRAM_FIELD_CR0_HI32 as usize] = smram_translate(0x7f5c);
        smram_map[SMRAM_FIELD_CR0 as usize] = smram_translate(0x7f58);
        smram_map[SMRAM_FIELD_CR3_HI32 as usize] = smram_translate(0x7f54);
        smram_map[SMRAM_FIELD_CR3 as usize] = smram_translate(0x7f50);
        smram_map[SMRAM_FIELD_CR4_HI32 as usize] = smram_translate(0x7f4c);
        smram_map[SMRAM_FIELD_CR4 as usize] = smram_translate(0x7f48);
        smram_map[SMRAM_FIELD_SSP_HI32 as usize] = smram_translate(0x7f44);
        smram_map[SMRAM_FIELD_SSP as usize] = smram_translate(0x7f40);
        smram_map[SMRAM_FIELD_EFER_HI32 as usize] = smram_translate(0x7ed4);
        smram_map[SMRAM_FIELD_EFER as usize] = smram_translate(0x7ed0);
        smram_map[SMRAM_FIELD_IO_INSTRUCTION_RESTART as usize] = smram_translate(0x7ec8);
        smram_map[SMRAM_FIELD_AUTOHALT_RESTART as usize] = smram_translate(0x7ec8);
        smram_map[SMRAM_FIELD_NMI_MASK as usize] = smram_translate(0x7ec8);
        smram_map[SMRAM_FIELD_TR_BASE_HI32 as usize] = smram_translate(0x7e9c);
        smram_map[SMRAM_FIELD_TR_BASE as usize] = smram_translate(0x7e98);
        smram_map[SMRAM_FIELD_TR_LIMIT as usize] = smram_translate(0x7e94);
        smram_map[SMRAM_FIELD_TR_SELECTOR_AR as usize] = smram_translate(0x7e90);
        smram_map[SMRAM_FIELD_IDTR_BASE_HI32 as usize] = smram_translate(0x7e8c);
        smram_map[SMRAM_FIELD_IDTR_BASE as usize] = smram_translate(0x7e88);
        smram_map[SMRAM_FIELD_IDTR_LIMIT as usize] = smram_translate(0x7e84);
        smram_map[SMRAM_FIELD_LDTR_BASE_HI32 as usize] = smram_translate(0x7e7c);
        smram_map[SMRAM_FIELD_LDTR_BASE as usize] = smram_translate(0x7e78);
        smram_map[SMRAM_FIELD_LDTR_LIMIT as usize] = smram_translate(0x7e74);
        smram_map[SMRAM_FIELD_LDTR_SELECTOR_AR as usize] = smram_translate(0x7e70);
        smram_map[SMRAM_FIELD_GDTR_BASE_HI32 as usize] = smram_translate(0x7e6c);
        smram_map[SMRAM_FIELD_GDTR_BASE as usize] = smram_translate(0x7e68);
        smram_map[SMRAM_FIELD_GDTR_LIMIT as usize] = smram_translate(0x7e64);
        smram_map[SMRAM_FIELD_ES_BASE_HI32 as usize] = smram_translate(0x7e0c);
        smram_map[SMRAM_FIELD_ES_BASE as usize] = smram_translate(0x7e08);
        smram_map[SMRAM_FIELD_ES_LIMIT as usize] = smram_translate(0x7e04);
        smram_map[SMRAM_FIELD_ES_SELECTOR_AR as usize] = smram_translate(0x7e00);
        smram_map[SMRAM_FIELD_CS_BASE_HI32 as usize] = smram_translate(0x7e1c);
        smram_map[SMRAM_FIELD_CS_BASE as usize] = smram_translate(0x7e18);
        smram_map[SMRAM_FIELD_CS_LIMIT as usize] = smram_translate(0x7e14);
        smram_map[SMRAM_FIELD_CS_SELECTOR_AR as usize] = smram_translate(0x7e10);
        smram_map[SMRAM_FIELD_SS_BASE_HI32 as usize] = smram_translate(0x7e2c);
        smram_map[SMRAM_FIELD_SS_BASE as usize] = smram_translate(0x7e28);
        smram_map[SMRAM_FIELD_SS_LIMIT as usize] = smram_translate(0x7e24);
        smram_map[SMRAM_FIELD_SS_SELECTOR_AR as usize] = smram_translate(0x7e20);
        smram_map[SMRAM_FIELD_DS_BASE_HI32 as usize] = smram_translate(0x7e3c);
        smram_map[SMRAM_FIELD_DS_BASE as usize] = smram_translate(0x7e38);
        smram_map[SMRAM_FIELD_DS_LIMIT as usize] = smram_translate(0x7e34);
        smram_map[SMRAM_FIELD_DS_SELECTOR_AR as usize] = smram_translate(0x7e30);
        smram_map[SMRAM_FIELD_FS_BASE_HI32 as usize] = smram_translate(0x7e4c);
        smram_map[SMRAM_FIELD_FS_BASE as usize] = smram_translate(0x7e48);
        smram_map[SMRAM_FIELD_FS_LIMIT as usize] = smram_translate(0x7e44);
        smram_map[SMRAM_FIELD_FS_SELECTOR_AR as usize] = smram_translate(0x7e40);
        smram_map[SMRAM_FIELD_GS_BASE_HI32 as usize] = smram_translate(0x7e5c);
        smram_map[SMRAM_FIELD_GS_BASE as usize] = smram_translate(0x7e58);
        smram_map[SMRAM_FIELD_GS_LIMIT as usize] = smram_translate(0x7e54);
        smram_map[SMRAM_FIELD_GS_SELECTOR_AR as usize] = smram_translate(0x7e50);

        for (index, value) in smram_map.iter().enumerate() {
            let value = *value;
            if value >= SMM_SAVE_STATE_MAP_SIZE {
                return Err(CpuError::SmramMap { index, value });
            }
        }

        Ok(smram_map)
    }

    // ========================================================================
    // RSM — Resume from System Management Mode (opcode 0F AA)
    // Bochs: smm.cc:34-87
    // ========================================================================

    pub(super) fn rsm(&mut self, _instr: &Instruction) -> Result<()> {
        if !self.smm_mode() {
            tracing::debug!("RSM: not in SMM mode, #UD");
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        tracing::debug!("RSM: resuming from SMM (smbase={:#010x})", self.smbase);

        // Read 128 dwords from SMRAM at smbase + 0x10000 (counting down)
        let mut saved_state = [0u32; SMRAM_STATE_SIZE];
        let mut paddr = (self.smbase as u64) + 0x10000;
        for dword in saved_state.iter_mut() {
            paddr -= 4;
            *dword = self.smram_read_physical_dword(paddr);
        }

        // Exit SMM
        self.in_smm = false;

        // Restore CPU state from saved SMRAM
        self.smram_restore_state(&saved_state);

        // Invalidate TLB and context
        self.handle_cpu_context_change();

        Ok(())
    }

    // ========================================================================
    // Enter System Management Mode
    // Bochs: smm.cc:89-215
    // Called when an SMI (System Management Interrupt) is delivered
    // ========================================================================

    pub(super) fn enter_system_management_mode(&mut self) {
        tracing::debug!("enter_system_management_mode: smbase={:#010x}", self.smbase);

        // Set SMM active
        self.in_smm = true;

        // Save CPU state to SMRAM
        let mut saved_state = [0u32; SMRAM_STATE_SIZE];
        self.smram_save_state(&mut saved_state);

        // Write state to SMRAM: smbase + 0x10000 counting down
        let mut paddr = (self.smbase as u64) + 0x10000;
        for &dword in saved_state.iter() {
            paddr -= 4;
            self.smram_write_physical_dword(paddr, dword);
        }

        // Initialize CPU to SMM entry state (Bochs smm.cc:163-214)

        // EFLAGS = 0x2 (bit 1 always set)
        self.set_eflags_internal(0x2);

        // RIP = 0x8000 (SMM entry point within SMRAM)
        self.set_eip(0x00008000);

        // DR7 = 0x400 (breakpoints disabled)
        self.dr7.set32(0x00000400);

        // CR0: clear PE, EM, TS, PG (enter real-mode-like state)
        let cr0_val = self.cr0.get32();
        let new_cr0 = cr0_val & !0x8000_0019; // clear PG(31), TS(3), EM(2), PE(0)
        self.cr0.set32(new_cr0);

        // CR4 = 0
        self.cr4.set32(0);

        // EFER = 0 (clear LME etc.)
        self.efer.set32(0);

        // CS: selector = smbase >> 4, base = smbase, limit = 4GB
        // This is a special 16-bit real-mode-like segment with base = smbase
        let cs_idx = BxSegregs::Cs as usize;
        let cs_sel = (self.smbase >> 4) as u16;
        super::segment_ctrl_pro::parse_selector(cs_sel, &mut self.sregs[cs_idx].selector);
        self.sregs[cs_idx].cache.valid = SEG_VALID_CACHE
            | SEG_ACCESS_ROK
            | SEG_ACCESS_WOK
            | SEG_ACCESS_ROK4_G
            | SEG_ACCESS_WOK4_G;
        self.sregs[cs_idx].cache.p = true;
        self.sregs[cs_idx].cache.dpl = 0;
        self.sregs[cs_idx].cache.segment = true;
        self.sregs[cs_idx].cache.r#type = 0x3; // DATA_READ_WRITE_ACCESSED
        unsafe {
            self.sregs[cs_idx].cache.u.segment.base = self.smbase as u64;
            self.sregs[cs_idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
            self.sregs[cs_idx].cache.u.segment.g = true;
            self.sregs[cs_idx].cache.u.segment.d_b = false; // 16-bit default
            self.sregs[cs_idx].cache.u.segment.avl = false;
            self.sregs[cs_idx].cache.u.segment.l = false;
        }

        // DS/ES/SS/FS/GS: all set to flat data segments with base=0
        for seg in [
            BxSegregs::Ds,
            BxSegregs::Es,
            BxSegregs::Ss,
            BxSegregs::Fs,
            BxSegregs::Gs,
        ] {
            let idx = seg as usize;
            super::segment_ctrl_pro::parse_selector(0, &mut self.sregs[idx].selector);
            self.sregs[idx].cache.valid = SEG_VALID_CACHE
                | SEG_ACCESS_ROK
                | SEG_ACCESS_WOK
                | SEG_ACCESS_ROK4_G
                | SEG_ACCESS_WOK4_G;
            self.sregs[idx].cache.p = true;
            self.sregs[idx].cache.dpl = 0;
            self.sregs[idx].cache.segment = true;
            self.sregs[idx].cache.r#type = 0x3; // DATA_READ_WRITE_ACCESSED
            unsafe {
                self.sregs[idx].cache.u.segment.base = 0;
                self.sregs[idx].cache.u.segment.limit_scaled = 0xFFFF_FFFF;
                self.sregs[idx].cache.u.segment.g = true;
                self.sregs[idx].cache.u.segment.d_b = false; // 16-bit
                self.sregs[idx].cache.u.segment.avl = false;
                self.sregs[idx].cache.u.segment.l = false;
            }
        }

        // Update CPU mode (we cleared PE → real mode)
        self.handle_cpu_mode_change();

        // Invalidate TLB
        self.handle_cpu_context_change();
    }

    // ========================================================================
    // Save CPU state to SMRAM array (32-bit mode)
    // Bochs: smm.cc:537-593
    // ========================================================================

    fn smram_save_state(&self, saved_state: &mut [u32; SMRAM_STATE_SIZE]) {
        let map = &self.smram_map;

        // Helper macro to set a field in the saved state
        macro_rules! smram_set {
            ($field:expr, $val:expr) => {
                saved_state[map[$field as usize] as usize] = $val;
            };
        }

        // GPRs (32-bit only for 32-bit SMM format)
        smram_set!(SMRAM_FIELD_EAX, self.eax());
        smram_set!(SMRAM_FIELD_ECX, self.ecx());
        smram_set!(SMRAM_FIELD_EDX, self.edx());
        smram_set!(SMRAM_FIELD_EBX, self.ebx());
        smram_set!(SMRAM_FIELD_ESP, self.esp());
        smram_set!(SMRAM_FIELD_EBP, self.ebp());
        smram_set!(SMRAM_FIELD_ESI, self.esi());
        smram_set!(SMRAM_FIELD_EDI, self.edi());

        // EIP, EFLAGS
        smram_set!(SMRAM_FIELD_EIP, self.eip());
        smram_set!(SMRAM_FIELD_EFLAGS, self.eflags.bits());

        // DR6, DR7
        smram_set!(SMRAM_FIELD_DR6, self.dr6.get32());
        smram_set!(SMRAM_FIELD_DR7, self.dr7.get32());

        // CR0, CR3, CR4, EFER
        smram_set!(SMRAM_FIELD_CR0, self.cr0.get32());
        smram_set!(SMRAM_FIELD_CR3, self.cr3 as u32);
        smram_set!(SMRAM_FIELD_CR4, self.cr4.get32());
        smram_set!(SMRAM_FIELD_EFER, self.efer.get32());

        // SMBASE, SMM revision ID
        smram_set!(SMRAM_FIELD_SMBASE_OFFSET, self.smbase);
        smram_set!(SMRAM_FIELD_SMM_REVISION_ID, SMM_REVISION_ID);

        // GDTR
        smram_set!(SMRAM_FIELD_GDTR_BASE, self.gdtr.base as u32);
        smram_set!(SMRAM_FIELD_GDTR_LIMIT, self.gdtr.limit as u32);

        // IDTR
        smram_set!(SMRAM_FIELD_IDTR_BASE, self.idtr.base as u32);
        smram_set!(SMRAM_FIELD_IDTR_LIMIT, self.idtr.limit as u32);

        // Save segment registers (TR, LDTR, and 6 segment regs)
        // Each segment stores: base, limit, selector_ar
        // AR format: selector | (ar_byte << 16) where ar_byte = descriptor access rights

        // TR (Task Register)
        let tr_ar = self.pack_seg_ar(&self.tr.cache);
        smram_set!(SMRAM_FIELD_TR_BASE, unsafe {
            self.tr.cache.u.segment.base as u32
        });
        smram_set!(SMRAM_FIELD_TR_LIMIT, unsafe {
            self.tr.cache.u.segment.limit_scaled
        });
        smram_set!(
            SMRAM_FIELD_TR_SELECTOR_AR,
            self.tr.selector.value as u32 | ((tr_ar as u32) << 16)
        );

        // LDTR
        let ldtr_ar = self.pack_seg_ar(&self.ldtr.cache);
        smram_set!(SMRAM_FIELD_LDTR_BASE, unsafe {
            self.ldtr.cache.u.segment.base as u32
        });
        smram_set!(SMRAM_FIELD_LDTR_LIMIT, unsafe {
            self.ldtr.cache.u.segment.limit_scaled
        });
        smram_set!(
            SMRAM_FIELD_LDTR_SELECTOR_AR,
            self.ldtr.selector.value as u32 | ((ldtr_ar as u32) << 16)
        );

        // Segment registers: ES, CS, SS, DS, FS, GS
        let seg_fields = [
            (
                BxSegregs::Es,
                SMRAM_FIELD_ES_BASE,
                SMRAM_FIELD_ES_LIMIT,
                SMRAM_FIELD_ES_SELECTOR_AR,
            ),
            (
                BxSegregs::Cs,
                SMRAM_FIELD_CS_BASE,
                SMRAM_FIELD_CS_LIMIT,
                SMRAM_FIELD_CS_SELECTOR_AR,
            ),
            (
                BxSegregs::Ss,
                SMRAM_FIELD_SS_BASE,
                SMRAM_FIELD_SS_LIMIT,
                SMRAM_FIELD_SS_SELECTOR_AR,
            ),
            (
                BxSegregs::Ds,
                SMRAM_FIELD_DS_BASE,
                SMRAM_FIELD_DS_LIMIT,
                SMRAM_FIELD_DS_SELECTOR_AR,
            ),
            (
                BxSegregs::Fs,
                SMRAM_FIELD_FS_BASE,
                SMRAM_FIELD_FS_LIMIT,
                SMRAM_FIELD_FS_SELECTOR_AR,
            ),
            (
                BxSegregs::Gs,
                SMRAM_FIELD_GS_BASE,
                SMRAM_FIELD_GS_LIMIT,
                SMRAM_FIELD_GS_SELECTOR_AR,
            ),
        ];

        for (seg, base_field, limit_field, selar_field) in seg_fields {
            let idx = seg as usize;
            let ar = self.pack_seg_ar(&self.sregs[idx].cache);
            let sel = self.sregs[idx].selector.value;
            smram_set!(base_field, unsafe {
                self.sregs[idx].cache.u.segment.base as u32
            });
            smram_set!(limit_field, unsafe {
                self.sregs[idx].cache.u.segment.limit_scaled
            });
            smram_set!(selar_field, sel as u32 | ((ar as u32) << 16));
        }
    }

    // ========================================================================
    // Restore CPU state from SMRAM array
    // Bochs: smm.cc:594-644 + resume_from_system_management_mode (648-844)
    // ========================================================================

    fn smram_restore_state(&mut self, saved_state: &[u32; SMRAM_STATE_SIZE]) {
        // Copy the map to avoid borrow conflict with &mut self
        let map = self.smram_map;

        macro_rules! smram_get {
            ($field:expr) => {
                saved_state[map[$field as usize] as usize]
            };
        }

        // Restore GPRs
        self.set_eax(smram_get!(SMRAM_FIELD_EAX));
        self.set_ecx(smram_get!(SMRAM_FIELD_ECX));
        self.set_edx(smram_get!(SMRAM_FIELD_EDX));
        self.set_ebx(smram_get!(SMRAM_FIELD_EBX));
        self.set_esp(smram_get!(SMRAM_FIELD_ESP));
        self.set_ebp(smram_get!(SMRAM_FIELD_EBP));
        self.set_esi(smram_get!(SMRAM_FIELD_ESI));
        self.set_edi(smram_get!(SMRAM_FIELD_EDI));

        // Restore EIP
        let eip = smram_get!(SMRAM_FIELD_EIP);
        self.set_eip(eip);
        self.prev_rip = eip as u64;

        // Restore EFLAGS
        let eflags = smram_get!(SMRAM_FIELD_EFLAGS);
        self.set_eflags_internal(eflags);

        // Restore DR6, DR7
        self.dr6.set32(smram_get!(SMRAM_FIELD_DR6));
        self.dr7.set32(smram_get!(SMRAM_FIELD_DR7));

        // Restore CR0, CR4, EFER, CR3
        let saved_cr0 = smram_get!(SMRAM_FIELD_CR0);
        let saved_cr4 = smram_get!(SMRAM_FIELD_CR4);
        let saved_efer = smram_get!(SMRAM_FIELD_EFER);
        let saved_cr3 = smram_get!(SMRAM_FIELD_CR3);

        self.cr0.set32(saved_cr0);
        self.cr4.set32(saved_cr4);
        self.efer.set32(saved_efer);
        self.cr3 = saved_cr3 as u64;

        // Restore GDTR, IDTR
        self.gdtr.base = smram_get!(SMRAM_FIELD_GDTR_BASE) as u64;
        self.gdtr.limit = smram_get!(SMRAM_FIELD_GDTR_LIMIT) as u16;

        self.idtr.base = smram_get!(SMRAM_FIELD_IDTR_BASE) as u64;
        self.idtr.limit = smram_get!(SMRAM_FIELD_IDTR_LIMIT) as u16;

        // Restore TR
        let tr_selar = smram_get!(SMRAM_FIELD_TR_SELECTOR_AR);
        let tr_sel = (tr_selar & 0xFFFF) as u16;
        let tr_ar = ((tr_selar >> 16) & 0xFFFF) as u16;
        super::segment_ctrl_pro::parse_selector(tr_sel, &mut self.tr.selector);
        unpack_seg_ar(&mut self.tr.cache, tr_ar);
        unsafe {
            self.tr.cache.u.segment.base = smram_get!(SMRAM_FIELD_TR_BASE) as u64;
            self.tr.cache.u.segment.limit_scaled = smram_get!(SMRAM_FIELD_TR_LIMIT);
        }

        // Restore LDTR
        let ldtr_selar = smram_get!(SMRAM_FIELD_LDTR_SELECTOR_AR);
        let ldtr_sel = (ldtr_selar & 0xFFFF) as u16;
        let ldtr_ar = ((ldtr_selar >> 16) & 0xFFFF) as u16;
        super::segment_ctrl_pro::parse_selector(ldtr_sel, &mut self.ldtr.selector);
        unpack_seg_ar(&mut self.ldtr.cache, ldtr_ar);
        unsafe {
            self.ldtr.cache.u.segment.base = smram_get!(SMRAM_FIELD_LDTR_BASE) as u64;
            self.ldtr.cache.u.segment.limit_scaled = smram_get!(SMRAM_FIELD_LDTR_LIMIT);
        }

        // Restore segment registers
        let seg_fields = [
            (
                BxSegregs::Es,
                SMRAM_FIELD_ES_BASE,
                SMRAM_FIELD_ES_LIMIT,
                SMRAM_FIELD_ES_SELECTOR_AR,
            ),
            (
                BxSegregs::Cs,
                SMRAM_FIELD_CS_BASE,
                SMRAM_FIELD_CS_LIMIT,
                SMRAM_FIELD_CS_SELECTOR_AR,
            ),
            (
                BxSegregs::Ss,
                SMRAM_FIELD_SS_BASE,
                SMRAM_FIELD_SS_LIMIT,
                SMRAM_FIELD_SS_SELECTOR_AR,
            ),
            (
                BxSegregs::Ds,
                SMRAM_FIELD_DS_BASE,
                SMRAM_FIELD_DS_LIMIT,
                SMRAM_FIELD_DS_SELECTOR_AR,
            ),
            (
                BxSegregs::Fs,
                SMRAM_FIELD_FS_BASE,
                SMRAM_FIELD_FS_LIMIT,
                SMRAM_FIELD_FS_SELECTOR_AR,
            ),
            (
                BxSegregs::Gs,
                SMRAM_FIELD_GS_BASE,
                SMRAM_FIELD_GS_LIMIT,
                SMRAM_FIELD_GS_SELECTOR_AR,
            ),
        ];

        for (seg, base_field, limit_field, selar_field) in seg_fields {
            let idx = seg as usize;
            let selar = smram_get!(selar_field);
            let sel = (selar & 0xFFFF) as u16;
            let ar = ((selar >> 16) & 0xFFFF) as u16;
            super::segment_ctrl_pro::parse_selector(sel, &mut self.sregs[idx].selector);
            unpack_seg_ar(&mut self.sregs[idx].cache, ar);
            unsafe {
                self.sregs[idx].cache.u.segment.base = smram_get!(base_field) as u64;
                self.sregs[idx].cache.u.segment.limit_scaled = smram_get!(limit_field);
            }
        }

        // Restore SMBASE (if revision ID supports relocation)
        let rev_id = smram_get!(SMRAM_FIELD_SMM_REVISION_ID);
        if (rev_id & 0x00020000) != 0 {
            // SMBASE relocation supported
            self.smbase = smram_get!(SMRAM_FIELD_SMBASE_OFFSET);
        }

        // Update CPU mode based on restored CR0/EFLAGS
        self.handle_cpu_mode_change();
        self.handle_alignment_check();
        self.update_fetch_mode_mask();
    }

    // ========================================================================
    // Helper: Pack descriptor access rights into 16-bit AR format
    // Bochs: (get_descriptor_h(cache) >> 8) & 0xf0ff, with valid bit at bit 8
    // ========================================================================

    fn pack_seg_ar(&self, cache: &super::descriptor::BxDescriptor) -> u16 {
        let mut ar: u16 = 0;

        // Low byte: P(7) + DPL(6:5) + S(4) + Type(3:0)
        ar |= cache.r#type as u16 & 0x0F;
        if cache.segment {
            ar |= 0x10;
        }
        ar |= ((cache.dpl as u16) & 0x03) << 5;
        if cache.p {
            ar |= 0x80;
        }

        // Bit 8: valid
        if cache.valid != 0 {
            ar |= 0x100;
        }

        // High nibble (bits 12-15): G(15) + D/B(14) + L(13) + AVL(12)
        unsafe {
            if cache.u.segment.avl {
                ar |= 0x1000;
            }
            if cache.u.segment.l {
                ar |= 0x2000;
            }
            if cache.u.segment.d_b {
                ar |= 0x4000;
            }
            if cache.u.segment.g {
                ar |= 0x8000;
            }
        }

        ar
    }

    // ========================================================================
    // Physical memory access helpers for SMRAM
    // These bypass paging (SMRAM is always physical)
    // ========================================================================

    fn smram_read_physical_dword(&mut self, paddr: u64) -> u32 {
        if let Some(mem_bus) = self.mem_bus {
            let mem = unsafe { &mut *mem_bus.as_ptr() };
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let mut data = [0u8; 4];
            if mem
                .read_physical_page(&[cpu_ref], paddr as _, 4, &mut data)
                .is_ok()
            {
                return u32::from_le_bytes(data);
            }
        }
        0 // Return 0 if memory not accessible
    }

    fn smram_write_physical_dword(&mut self, paddr: u64, value: u32) {
        if let Some(mem_bus) = self.mem_bus {
            let mem = unsafe { &mut *mem_bus.as_ptr() };
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let mut data = value.to_le_bytes();
            // Use a dummy stamp table for SMRAM writes (no SMC detection needed)
            let mut dummy_mapping: [u32; 0] = [];
            let mut dummy_stamp_table = super::icache::BxPageWriteStampTable {
                fine_granularity_mapping: &mut dummy_mapping,
            };
            // SMM state save write — physical RAM write cannot meaningfully fail
            let _ = mem.write_physical_page(
                &[cpu_ref],
                &mut dummy_stamp_table,
                paddr as _,
                4,
                &mut data,
            );
        }
    }
}

const fn smram_translate(addr: u32) -> u32 {
    ((0x8000 - (addr)) >> 2) - 1
}

/// Unpack 16-bit AR format into descriptor cache (standalone to avoid borrow conflicts)
fn unpack_seg_ar(cache: &mut super::descriptor::BxDescriptor, ar: u16) {
    cache.r#type = (ar & 0x0F) as u8;
    cache.segment = (ar & 0x10) != 0;
    cache.dpl = ((ar >> 5) & 0x03) as u8;
    cache.p = (ar & 0x80) != 0;

    // Bit 8: valid
    if (ar & 0x100) != 0 {
        cache.valid = SEG_VALID_CACHE
            | SEG_ACCESS_ROK
            | SEG_ACCESS_WOK
            | SEG_ACCESS_ROK4_G
            | SEG_ACCESS_WOK4_G;
    } else {
        cache.valid = 0;
    }

    // High nibble
    unsafe {
        cache.u.segment.avl = (ar & 0x1000) != 0;
        cache.u.segment.l = (ar & 0x2000) != 0;
        cache.u.segment.d_b = (ar & 0x4000) != 0;
        cache.u.segment.g = (ar & 0x8000) != 0;
    }
}
