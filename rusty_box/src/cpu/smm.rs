#![allow(dead_code)]
//! System Management Mode (SMM) entry/exit
//! Matching Bochs cpu/smm.cc
//!
//! SMM is entered via SMI (System Management Interrupt) and exited via RSM.
//! The CPU saves its entire state to SMRAM at smbase + 0x10000, then enters
//! a special real-mode-like execution environment at smbase + 0x8000.

use super::{
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction, BX_GENERAL_REGISTERS},
    descriptor::{
        SEG_ACCESS_ROK, SEG_ACCESS_ROK4_G, SEG_ACCESS_WOK, SEG_ACCESS_WOK4_G, SEG_VALID_CACHE,
    },
    BxCpuC, CpuError, Result,
};

const SMM_SAVE_STATE_MAP_SIZE: u32 = 128;

/// Bochs smm.h `SMM_IO_INSTRUCTION_RESTART` / `SMM_SMBASE_RELOCATION` feature
/// bits of the SMM revision identifier.
const SMM_SMBASE_RELOCATION: u32 = 0x00020000;

/// Bochs smm.h `SMM_REVISION_ID` for the x86-64 build: the AMD Athlon 64
/// 512-byte save-map revision (low byte 0x64) plus the SMBASE-relocation
/// feature bit. The low byte is load-bearing: the Bochs BIOS SMM relocation
/// handler (rombios32start.S) reads it at SMBASE+0xfefc and does `cmp $0x64`
/// to pick the x86-64 SMBASE slot (SMBASE+0xff00) over the x86-32 one
/// (SMBASE+0xfef8).
const SMM_REVISION_ID: u32 = 0x00000064 | SMM_SMBASE_RELOCATION;

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
    SMRAM_FIELD_CR4_HI32,
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

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
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
    // Bochs: smm.cc
    // ========================================================================

    pub(super) fn rsm(&mut self, _instr: &Instruction) -> Result<()> {
        if !self.smm_mode() {
            tracing::trace!("RSM: not in SMM mode, #UD");
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // Bochs svm.cc SVM_INTERCEPT0_RSM.
        if self.in_svm_guest && self.svm_intercept_check(super::svm::SVM_INTERCEPT0_RSM) {
            return self.svm_vmexit(super::svm::SvmVmexit::Rsm as i32, 0, 0);
        }
        // Bochs smm.cc RSM: VMexit when in VMX guest, #UD in VMX root
        // operation.
        if self.in_vmx_guest {
            return self.vmx_vmexit(super::vmx::VmxVmexitReason::Rsm, 0);
        }
        if self.in_vmx {
            tracing::error!("RSM in VMX root operation !");
            return self.exception(super::cpu::Exception::Ud, 0);
        }

        // Bochs smm.cc RSM: BX_INFO(("RSM: Resuming from System Management Mode")).
        tracing::info!("RSM: Resuming from System Management Mode");

        // Bochs smm.cc RSM: release the events held while in SMM.
        self.unmask_event(
            Self::BX_EVENT_SMI | Self::BX_EVENT_NMI | Self::BX_EVENT_VMX_VIRTUAL_NMI,
        );

        // Read 128 dwords from SMRAM at smbase + 0x10000 (counting down)
        let mut saved_state = [0u32; SMRAM_STATE_SIZE];
        let mut paddr = (self.smbase as u64) + 0x10000;
        for dword in saved_state.iter_mut() {
            paddr -= 4;
            *dword = self.smram_read_physical_dword(paddr);
        }

        // Exit SMM
        self.in_smm = false;

        // Restore CPU state from saved SMRAM. Bochs RSM: an inconsistent
        // image is a BX_PANIC + shutdown() — enter the shutdown state like
        // the triple-fault path does.
        if !self.smram_restore_state(&saved_state) {
            tracing::error!("RSM: Incorrect state when restoring CPU state - shutdown !");
            self.activity_state = super::cpu::CpuActivityState::Shutdown;
            self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
            return Err(super::error::CpuError::CpuLoopRestart);
        }


        // Bochs smm.cc RSM ends with BX_NEXT_TRACE(i): RSM is a serializing
        // trace-terminating instruction. The cpu_loop 'trace loop only breaks
        // to re-fetch when async_event is set (cpu.cc); every other
        // trace-ending control transfer (ctrl_xfer*.cc JMP/CALL/RET/IRET) sets
        // BX_ASYNC_EVENT_STOP_TRACE from its handler for exactly this reason.
        // Without it, after RSM restores the outer RIP the loop advances
        // `instr_idx` into the *next* slot of the now-defunct SMM-handler trace
        // (its trailing InsertedOpcode boundary marker), executing it under the
        // SMM trace's stale real-mode `is_real` and masking RIP to 16 bits.
        self.async_event |= super::cpu::BX_ASYNC_EVENT_STOP_TRACE;

        Ok(())
    }

    // ========================================================================
    // Enter System Management Mode
    // Bochs: smm.cc
    // Called when an SMI (System Management Interrupt) is delivered
    // ========================================================================

    pub(super) fn enter_system_management_mode(&mut self) {
        // Bochs smm.cc enter_system_management_mode:
        // BX_INFO(("Enter to System Management Mode")).
        tracing::info!("Enter to System Management Mode");

        // Bochs smm.cc enter_system_management_mode: SMI delivery leaves VMX
        // operation — CR4.VMXE is cleared and the root/non-root indication is
        // parked in in_smm_vmx / in_smm_vmx_guest until RSM restores it.
        self.cr4.remove(super::crregs::BxCr4::VMXE);
        self.in_smm_vmx = self.in_vmx;
        self.in_smm_vmx_guest = self.in_vmx_guest;
        self.in_vmx = false;
        self.in_vmx_guest = false;

        // Set SMM active
        self.in_smm = true;

        // Bochs smm.cc enter_system_management_mode: SMI, NMI, and VMX
        // virtual-NMI are held pending for the duration of SMM; RSM
        // unmasks them again.
        self.mask_event(
            Self::BX_EVENT_SMI | Self::BX_EVENT_NMI | Self::BX_EVENT_VMX_VIRTUAL_NMI,
        );

        // Save CPU state to SMRAM
        let mut saved_state = [0u32; SMRAM_STATE_SIZE];
        self.smram_save_state(&mut saved_state);

        // Write state to SMRAM: smbase + 0x10000 counting down
        let mut paddr = (self.smbase as u64) + 0x10000;
        for &dword in saved_state.iter() {
            paddr -= 4;
            self.smram_write_physical_dword(paddr, dword);
        }

        // Initialize CPU to SMM entry state (Bochs smm.cc)

        // EFLAGS = 0x2 (bit 1 always set)
        self.set_eflags_internal(0x2);

        // Bochs smm.cc: prev_rip = RIP = 0x00008000 (SMM entry point)
        self.set_rip(0x0000_8000);
        self.prev_rip = 0x0000_8000;

        // DR7 = 0x400 (breakpoints disabled)
        self.dr7.set32(0x00000400);

        // Bochs smm.cc: CR0 — PE, EM, TS, and PG flags set to 0; others
        // unmodified. Mask = PG(31) | TS(3) | EM(2) | PE(0) = 0x8000000D.
        let cr0_val = self.cr0.get32();
        let new_cr0 = cr0_val & !0x8000_000D;
        self.cr0.set32(new_cr0);

        // CR4 = 0
        self.cr4.set_val(0);

        // Bochs smm.cc: EFER is cleared except SVME, which survives SMM entry
        // when it was set.
        if self.efer.svme() {
            self.efer.set32(super::crregs::BxEfer::SVME.bits());
        } else {
            self.efer.set32(0);
        }

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
        self.sregs[cs_idx]
            .cache
            .u
            .set_segment_base(self.smbase as u64);
        self.sregs[cs_idx]
            .cache
            .u
            .set_segment_limit_scaled(0xFFFF_FFFF);
        self.sregs[cs_idx].cache.u.set_segment_g(true);
        self.sregs[cs_idx].cache.u.set_segment_d_b(false); // 16-bit default
        self.sregs[cs_idx].cache.u.set_segment_avl(false);
        self.sregs[cs_idx].cache.u.set_segment_l(false);

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
            self.sregs[idx].cache.u.set_segment_base(0);
            self.sregs[idx]
                .cache
                .u
                .set_segment_limit_scaled(0xFFFF_FFFF);
            self.sregs[idx].cache.u.set_segment_g(true);
            self.sregs[idx].cache.u.set_segment_d_b(false); // 16-bit
            self.sregs[idx].cache.u.set_segment_avl(false);
            self.sregs[idx].cache.u.set_segment_l(false);
        }

        // Bochs smm.cc enter_system_management_mode: handleCpuContextChange()
        // (TLB flush + prefetch/stack-cache invalidation + the mode recompute
        // for the PE clear) then the MONITOR reset (BX_SUPPORT_MONITOR_MWAIT).
        self.handle_cpu_context_change();
        self.monitor.reset_monitor();
    }

    // ========================================================================
    // Save CPU state to SMRAM array (32-bit mode)
    // Bochs: smm.cc
    // ========================================================================

    fn smram_save_state(&self, saved_state: &mut [u32; SMRAM_STATE_SIZE]) {
        let map = &self.smram_map;

        // Helper macro to set a field in the saved state
        macro_rules! smram_set {
            ($field:expr, $val:expr) => {
                saved_state[map[$field as usize] as usize] = $val;
            };
        }

        // GPRs — Bochs smm.cc smram_save_state (x86-64): every register saved
        // as a HI32/LO32 dword pair, RAX..R15 in Bochs register order (the
        // SMRAM_FIELD enum is laid out RAX_HI32, EAX, RCX_HI32, ECX, ... so
        // field index = RAX_HI32 + 2n / EAX + 2n).
        //
        // The bound is BX_GENERAL_REGISTERS (Bochs `for n=0; n<BX_GENERAL_REGISTERS`),
        // NOT gen_reg.len(): the array carries four extra slots after R15
        // (RIP, SSP, BX_TMP_REGISTER, BX_NIL_REGISTER) that are not part of the
        // SMRAM GPR block. Walking into them runs the field index off the end
        // of the GPR pairs and into RIP_HI32/EIP/RFLAGS_HI32/EFLAGS/DR6/DR7.
        for n in 0..BX_GENERAL_REGISTERS {
            let val = self.gen_reg[n].rrx();
            saved_state[map[SMRAM_FIELD_RAX_HI32 as usize + 2 * n] as usize] =
                (val >> 32) as u32;
            saved_state[map[SMRAM_FIELD_EAX as usize + 2 * n] as usize] = val as u32;
        }

        // RIP (64-bit), EFLAGS
        smram_set!(SMRAM_FIELD_RIP_HI32, (self.rip() >> 32) as u32);
        smram_set!(SMRAM_FIELD_EIP, self.rip() as u32);
        smram_set!(SMRAM_FIELD_EFLAGS, self.eflags_materialized());

        // SSP (Bochs smm.cc BX_SUPPORT_CET)
        smram_set!(SMRAM_FIELD_SSP_HI32, (self.ssp() >> 32) as u32);
        smram_set!(SMRAM_FIELD_SSP, self.ssp() as u32);

        // DR6, DR7 (HI32 dwords stay zero — Bochs leaves them unwritten)
        smram_set!(SMRAM_FIELD_DR6, self.dr6.get32());
        smram_set!(SMRAM_FIELD_DR7, self.dr7.get32());

        // CR0, CR3 (64-bit), CR4, EFER
        smram_set!(SMRAM_FIELD_CR0, self.cr0.get32());
        smram_set!(SMRAM_FIELD_CR3_HI32, (self.cr3 >> 32) as u32);
        smram_set!(SMRAM_FIELD_CR3, self.cr3 as u32);
        smram_set!(SMRAM_FIELD_CR4_HI32, (self.cr4.get() >> 32) as u32);
        smram_set!(SMRAM_FIELD_CR4, self.cr4.get32());
        smram_set!(SMRAM_FIELD_EFER, self.efer.get32());

        // SMBASE, SMM revision ID
        smram_set!(SMRAM_FIELD_SMBASE_OFFSET, self.smbase);
        smram_set!(SMRAM_FIELD_SMM_REVISION_ID, SMM_REVISION_ID);

        // GDTR (base is 64-bit)
        smram_set!(SMRAM_FIELD_GDTR_BASE_HI32, (self.gdtr.base >> 32) as u32);
        smram_set!(SMRAM_FIELD_GDTR_BASE, self.gdtr.base as u32);
        smram_set!(SMRAM_FIELD_GDTR_LIMIT, self.gdtr.limit as u32);

        // IDTR (base is 64-bit)
        smram_set!(SMRAM_FIELD_IDTR_BASE_HI32, (self.idtr.base >> 32) as u32);
        smram_set!(SMRAM_FIELD_IDTR_BASE, self.idtr.base as u32);
        smram_set!(SMRAM_FIELD_IDTR_LIMIT, self.idtr.limit as u32);

        // Save segment registers (TR, LDTR, and 6 segment regs)
        // Each segment stores: base (HI32/LO32), limit, selector_ar
        // AR format: selector | (ar_word << 16), Bochs
        // ((get_descriptor_h() >> 8) & 0xf0ff) | (valid << 8)

        // TR (Task Register)
        let tr_ar = self.pack_seg_ar(&self.tr.cache);
        let tr_base = self.tr.cache.u.segment_base();
        smram_set!(SMRAM_FIELD_TR_BASE_HI32, (tr_base >> 32) as u32);
        smram_set!(SMRAM_FIELD_TR_BASE, tr_base as u32);
        smram_set!(SMRAM_FIELD_TR_LIMIT, self.tr.cache.u.segment_limit_scaled());
        smram_set!(
            SMRAM_FIELD_TR_SELECTOR_AR,
            self.tr.selector.value as u32 | ((tr_ar as u32) << 16)
        );

        // LDTR
        let ldtr_ar = self.pack_seg_ar(&self.ldtr.cache);
        let ldtr_base = self.ldtr.cache.u.segment_base();
        smram_set!(SMRAM_FIELD_LDTR_BASE_HI32, (ldtr_base >> 32) as u32);
        smram_set!(SMRAM_FIELD_LDTR_BASE, ldtr_base as u32);
        smram_set!(
            SMRAM_FIELD_LDTR_LIMIT,
            self.ldtr.cache.u.segment_limit_scaled()
        );
        smram_set!(
            SMRAM_FIELD_LDTR_SELECTOR_AR,
            self.ldtr.selector.value as u32 | ((ldtr_ar as u32) << 16)
        );

        // Segment registers: ES, CS, SS, DS, FS, GS — base saved as
        // HI32/LO32 (Bochs smm.cc x86-64 smram_save_state).
        for (seg, base_hi_field, base_field, limit_field, selar_field) in SEG_FIELDS {
            let idx = seg as usize;
            let ar = self.pack_seg_ar(&self.sregs[idx].cache);
            let sel = self.sregs[idx].selector.value;
            let base = self.sregs[idx].cache.u.segment_base();
            smram_set!(base_hi_field, (base >> 32) as u32);
            smram_set!(base_field, base as u32);
            smram_set!(limit_field, self.sregs[idx].cache.u.segment_limit_scaled());
            smram_set!(selar_field, sel as u32 | ((ar as u32) << 16));
        }
    }

    // ========================================================================
    // Restore CPU state from SMRAM array
    // Bochs: smm.cc + resume_from_system_management_mode (648-844)
    // ========================================================================

    /// Bochs smm.cc `smram_restore_state` + `resume_from_system_management_mode`
    /// (x86-64 form): read every field 64-bit wide, run the full consistency
    /// validation, and return `false` when the image is inconsistent — the RSM
    /// caller then shuts the CPU down, exactly like Bochs.
    #[must_use]
    fn smram_restore_state(&mut self, saved_state: &[u32; SMRAM_STATE_SIZE]) -> bool {
        // Copy the map to avoid borrow conflict with &mut self
        let map = self.smram_map;

        macro_rules! smram_get {
            ($field:expr) => {
                saved_state[map[$field as usize] as usize]
            };
        }
        // Bochs smm.cc SMRAM_FIELD64: (hi << 32) | lo.
        macro_rules! smram_get64 {
            ($hi:expr, $lo:expr) => {
                ((smram_get!($hi) as u64) << 32) | smram_get!($lo) as u64
            };
        }

        let mut saved_cr0 = smram_get!(SMRAM_FIELD_CR0);
        let saved_cr3 = smram_get64!(SMRAM_FIELD_CR3_HI32, SMRAM_FIELD_CR3);
        let mut saved_cr4 = smram_get64!(SMRAM_FIELD_CR4_HI32, SMRAM_FIELD_CR4);
        let saved_efer = smram_get!(SMRAM_FIELD_EFER);
        let saved_eflags = smram_get!(SMRAM_FIELD_EFLAGS);

        // Bochs resume_from_system_management_mode: a CR4.VMXE=1 image fails
        // the restore outright (RSM into VMX operation is re-entered via the
        // parked flags below, never via the image bit).
        if (saved_cr4 & super::crregs::BxCr4::VMXE.bits()) != 0 {
            tracing::error!("SMM restore: CR4.VMXE is set in restore image !");
            return false;
        }

        // Bochs smm.cc resume_from_system_management_mode: when the processor
        // returns to VMX operation, the restored state gets CR0.PE/NE/PG and
        // CR4.VMXE forced on, and in_vmx / in_vmx_guest come back from the
        // parked in_smm_vmx / in_smm_vmx_guest flags.
        if self.in_smm_vmx {
            self.in_vmx = true;
            self.in_vmx_guest = self.in_smm_vmx_guest;
            tracing::info!(
                "SMM Restore: enable VMX {} mode",
                if self.in_vmx_guest { "guest" } else { "host" }
            );
            saved_cr0 |= (super::crregs::BxCr0::PG
                | super::crregs::BxCr0::NE
                | super::crregs::BxCr0::PE)
                .bits();
            saved_cr4 |= super::crregs::BxCr4::VMXE.bits();
        }

        // Bochs: EFER reserved-bit check against efer_suppmask, then set.
        if (saved_efer & !self.efer_suppmask) != 0 {
            tracing::error!(
                "SMM restore: Attempt to set EFER reserved bits: {:#010x} !",
                saved_efer
            );
            return false;
        }
        self.efer.set32(saved_efer);

        // Bochs check_CR0(): PG without PE and NW without CD are illegal, plus
        // the VMX-operation constraints.
        let cr0_bits = super::crregs::BxCr0::from_bits_retain(saved_cr0);
        if cr0_bits.contains(super::crregs::BxCr0::PG)
            && !cr0_bits.contains(super::crregs::BxCr0::PE)
        {
            tracing::error!("SMM restore: CR0 consistency check failed (PG without PE) !");
            return false;
        }
        if cr0_bits.contains(super::crregs::BxCr0::NW)
            && !cr0_bits.contains(super::crregs::BxCr0::CD)
        {
            tracing::error!("SMM restore: CR0 consistency check failed (NW without CD) !");
            return false;
        }
        if !self.check_cr0_vmx(saved_cr0 as u64, false) {
            tracing::error!("SMM restore: CR0 consistency check failed (VMX) !");
            return false;
        }

        // Bochs check_CR4(): reserved / unsupported bits.
        if (saved_cr4 & !(self.cr4_suppmask as u64)) != 0 {
            tracing::error!("SMM restore: CR4 consistency check failed !");
            return false;
        }

        self.cr0.set32(saved_cr0);
        self.cr4.set_val(saved_cr4);
        self.cr3 = saved_cr3;

        // Bochs x86-64 consistency block: EFER.LMA must agree with
        // CR4.PAE/CR0.PG/CR0.PE/EFER.LME, RFLAGS.VM must be clear in long
        // mode, and CR4.PCIDE requires long mode.
        const EFLAGS_VM_MASK: u32 = 1 << 17;
        if self.efer.lma() {
            if (saved_eflags & EFLAGS_VM_MASK) != 0 {
                tracing::error!("SMM restore: If EFER.LMA = 1 => RFLAGS.VM=0 !");
                return false;
            }
            if !self.cr4.pae() || !self.cr0.pg() || !self.cr0.pe() || !self.efer.lme() {
                tracing::error!(
                    "SMM restore: If EFER.LMA = 1 <=> CR4.PAE, CR0.PG, CR0.PE, EFER.LME=1 !"
                );
                return false;
            }
        } else if self.cr4.contains(super::crregs::BxCr4::PCIDE) {
            tracing::error!("SMM restore: CR4.PCIDE must be clear when not in long mode !");
            return false;
        }
        if self.cr4.pae() && self.cr0.pg() && self.cr0.pe() && self.efer.lme() && !self.efer.lma()
        {
            tracing::error!(
                "SMM restore: If EFER.LMA = 1 <=> CR4.PAE, CR0.PG, CR0.PE, EFER.LME=1 !"
            );
            return false;
        }

        // Bochs: PAE-paging PDPTR revalidation outside long mode.
        if self.cr0.pg() && self.cr4.pae() && !self.long_mode() {
            match self.check_pdptrs(saved_cr3) {
                Ok(true) => {}
                _ => {
                    tracing::error!("SMM restore: PDPTR check failed !");
                    return false;
                }
            }
        }

        self.set_eflags_internal(saved_eflags);

        // Restore GPRs 64-bit wide (Bochs BX_WRITE_64BIT_REG loop over
        // BX_GENERAL_REGISTERS). The bound matters: gen_reg has four trailing
        // slots (RIP, SSP, BX_TMP_REGISTER, BX_NIL_REGISTER) that are NOT
        // GPRs. Restoring `gen_reg.len()` entries walked the field index past
        // R15 into RIP/EFLAGS/DR6/DR7 and wrote those values into the trailing
        // slots — clobbering BX_NIL_REGISTER, which the decoder relies on
        // being permanently zero as the base of no-base addressing forms
        // (init.rs sets it once at reset), so every later `[disp32]`-style
        // access was displaced by DR7's value.
        for n in 0..BX_GENERAL_REGISTERS {
            let hi = saved_state[map[SMRAM_FIELD_RAX_HI32 as usize + 2 * n] as usize];
            let lo = saved_state[map[SMRAM_FIELD_EAX as usize + 2 * n] as usize];
            self.gen_reg[n].set_rrx(((hi as u64) << 32) | lo as u64);
        }

        // RIP (Bochs: RIP = prev_rip = smm_state->rip), SSP (BX_SUPPORT_CET)
        let rip = smram_get64!(SMRAM_FIELD_RIP_HI32, SMRAM_FIELD_EIP);
        self.set_rip(rip);
        self.prev_rip = rip;
        self.set_ssp(smram_get64!(SMRAM_FIELD_SSP_HI32, SMRAM_FIELD_SSP));

        // Restore DR6, DR7
        self.dr6.set32(smram_get!(SMRAM_FIELD_DR6));
        self.dr7.set32(smram_get!(SMRAM_FIELD_DR7));

        // Restore GDTR, IDTR (64-bit bases)
        self.gdtr.base = smram_get64!(SMRAM_FIELD_GDTR_BASE_HI32, SMRAM_FIELD_GDTR_BASE);
        self.gdtr.limit = smram_get!(SMRAM_FIELD_GDTR_LIMIT) as u16;
        self.idtr.base = smram_get64!(SMRAM_FIELD_IDTR_BASE_HI32, SMRAM_FIELD_IDTR_BASE);
        self.idtr.limit = smram_get!(SMRAM_FIELD_IDTR_LIMIT) as u16;

        // Restore segment registers. Bochs set_segment_ar_data returns whether
        // the segment came back VALID; a valid entry that is not a data/code
        // segment fails the restore.
        for (seg, base_hi_field, base_field, limit_field, selar_field) in SEG_FIELDS {
            let idx = seg as usize;
            let selar = smram_get!(selar_field);
            let sel = (selar & 0xFFFF) as u16;
            let ar = ((selar >> 16) & 0xFFFF) as u16;
            let base = ((smram_get!(base_hi_field) as u64) << 32) | smram_get!(base_field) as u64;
            if super::segment_ctrl_pro::set_segment_ar_data(
                &mut self.sregs[idx],
                ((ar >> 8) & 1) != 0,
                sel,
                base,
                smram_get!(limit_field),
                ar,
            ) && !self.sregs[idx].cache.segment
            {
                tracing::error!("SMM restore: restored valid non segment {} !", idx);
                return false;
            }
        }

        // Restore LDTR — a valid LDTR must be an LDT descriptor (Bochs
        // BX_SYS_SEGMENT_LDT).
        let ldtr_selar = smram_get!(SMRAM_FIELD_LDTR_SELECTOR_AR);
        let ldtr_ar = ((ldtr_selar >> 16) & 0xFFFF) as u16;
        let ldtr_base = smram_get64!(SMRAM_FIELD_LDTR_BASE_HI32, SMRAM_FIELD_LDTR_BASE);
        // Bochs uses segment=false system descriptors here; the type check is
        // on the raw descriptor type value.
        const BX_SYS_SEGMENT_LDT: u8 = 2;
        if super::segment_ctrl_pro::set_segment_ar_data(
            &mut self.ldtr,
            ((ldtr_ar >> 8) & 1) != 0,
            (ldtr_selar & 0xFFFF) as u16,
            ldtr_base,
            smram_get!(SMRAM_FIELD_LDTR_LIMIT),
            ldtr_ar,
        ) && self.ldtr.cache.r#type != BX_SYS_SEGMENT_LDT
        {
            tracing::error!("SMM restore: LDTR is not LDT descriptor type !");
            return false;
        }

        // Restore TR — a valid TR must be a TSS descriptor type.
        let tr_selar = smram_get!(SMRAM_FIELD_TR_SELECTOR_AR);
        let tr_ar = ((tr_selar >> 16) & 0xFFFF) as u16;
        let tr_base = smram_get64!(SMRAM_FIELD_TR_BASE_HI32, SMRAM_FIELD_TR_BASE);
        // Bochs: AVAIL/BUSY 286/386 TSS types.
        if super::segment_ctrl_pro::set_segment_ar_data(
            &mut self.tr,
            ((tr_ar >> 8) & 1) != 0,
            (tr_selar & 0xFFFF) as u16,
            tr_base,
            smram_get!(SMRAM_FIELD_TR_LIMIT),
            tr_ar,
        ) && !matches!(self.tr.cache.r#type, 1 | 3 | 9 | 11)
        {
            tracing::error!("SMM restore: TR is not TSS descriptor type !");
            return false;
        }

        // Restore SMBASE. Bochs gates on its own SMM_REVISION_ID constant
        // (which always has the relocation bit), NOT the value read back from
        // SMRAM — a guest zeroing the revision field does not disable
        // relocation.
        if (SMM_REVISION_ID & SMM_SMBASE_RELOCATION) != 0 {
            self.smbase = smram_get!(SMRAM_FIELD_SMBASE_OFFSET);
        }

        // Bochs resume_from_system_management_mode ends with
        // handleCpuContextChange() (TLB flush + prefetch/stack-cache
        // invalidation + every mode/mask recompute) followed by the MONITOR
        // reset. Both run only on the success path — an inconsistent image
        // returns above and the caller shuts the CPU down.
        self.handle_cpu_context_change();
        self.monitor.reset_monitor();

        true
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
        if cache.u.segment_avl() {
            ar |= 0x1000;
        }
        if cache.u.segment_l() {
            ar |= 0x2000;
        }
        if cache.u.segment_d_b() {
            ar |= 0x4000;
        }
        if cache.u.segment_g() {
            ar |= 0x8000;
        }

        ar
    }

    // ========================================================================
    // Physical memory access helpers for SMRAM
    // These bypass paging (SMRAM is always physical)
    // ========================================================================

    fn smram_read_physical_dword(&mut self, paddr: u64) -> u32 {
        if let Some((policy, mem)) = unsafe { self.mem_bus_with_policy(paddr) } {
            let mut data = [0u8; 4];
        if mem.read_physical_page(self.active_tlb_pins(), policy, paddr as _, 4, &mut data)
            .is_ok()
        {
            return u32::from_le_bytes(data);
        } }
        0 // Return 0 if memory not accessible
    }

    fn smram_write_physical_dword(&mut self, paddr: u64, value: u32) {
        if let Some((policy, mem)) = unsafe { self.mem_bus_with_policy(paddr) } { let mut data = value.to_le_bytes();
        // SMM state save write — physical RAM write cannot meaningfully fail
        let _ = mem.write_physical_page(self.active_tlb_pins(), policy, paddr as _, 4, &mut data);
        // Bochs handleSMC flushes the writer synchronously at the store.
        self.smc_sync_after_phys_write(); }
    }
}

const fn smram_translate(addr: u32) -> u32 {
    ((0x8000 - (addr)) >> 2) - 1
}

/// Per-segment SMRAM field tuple: (segment, BASE_HI32, BASE, LIMIT,
/// SELECTOR_AR) — shared by save and restore (Bochs indexes these as
/// `SMRAM_FIELD_ES_* + 4*segreg`; the explicit list keeps the pairing
/// independent of the segment enum ordering).
const SEG_FIELDS: [(
    BxSegregs,
    SMMRAM_Fields,
    SMMRAM_Fields,
    SMMRAM_Fields,
    SMMRAM_Fields,
); 6] = [
    (
        BxSegregs::Es,
        SMRAM_FIELD_ES_BASE_HI32,
        SMRAM_FIELD_ES_BASE,
        SMRAM_FIELD_ES_LIMIT,
        SMRAM_FIELD_ES_SELECTOR_AR,
    ),
    (
        BxSegregs::Cs,
        SMRAM_FIELD_CS_BASE_HI32,
        SMRAM_FIELD_CS_BASE,
        SMRAM_FIELD_CS_LIMIT,
        SMRAM_FIELD_CS_SELECTOR_AR,
    ),
    (
        BxSegregs::Ss,
        SMRAM_FIELD_SS_BASE_HI32,
        SMRAM_FIELD_SS_BASE,
        SMRAM_FIELD_SS_LIMIT,
        SMRAM_FIELD_SS_SELECTOR_AR,
    ),
    (
        BxSegregs::Ds,
        SMRAM_FIELD_DS_BASE_HI32,
        SMRAM_FIELD_DS_BASE,
        SMRAM_FIELD_DS_LIMIT,
        SMRAM_FIELD_DS_SELECTOR_AR,
    ),
    (
        BxSegregs::Fs,
        SMRAM_FIELD_FS_BASE_HI32,
        SMRAM_FIELD_FS_BASE,
        SMRAM_FIELD_FS_LIMIT,
        SMRAM_FIELD_FS_SELECTOR_AR,
    ),
    (
        BxSegregs::Gs,
        SMRAM_FIELD_GS_BASE_HI32,
        SMRAM_FIELD_GS_BASE,
        SMRAM_FIELD_GS_LIMIT,
        SMRAM_FIELD_GS_SELECTOR_AR,
    ),
];

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::super::builder::BxCpuBuilder;
    use super::*;
    use crate::cpu::core_i7_skylake::Corei7SkylakeX;
    use crate::cpu::ResetReason;
    use crate::memory::{BxMemC, BxMemoryStubC};
    use core::ptr::NonNull;

    const MIB: usize = 1024 * 1024;

    /// BSP with 4 MiB of real backing memory attached — enough to cover the
    /// default SMBASE 0x30000 save area at 0x3fe00..0x40000.
    fn cpu_with_memory() -> (
        alloc::boxed::Box<BxCpuC<'static, Corei7SkylakeX>>,
        alloc::boxed::Box<BxMemC<'static>>,
    ) {
        let mut mem = alloc::boxed::Box::new(BxMemC::new(
            BxMemoryStubC::create_and_init(4 * MIB, 4 * MIB, 128 * 1024)
                .expect("memory allocation"),
            false,
        ));
        mem.set_a20_mask(u64::MAX);
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.reset(ResetReason::Hardware);
        cpu.set_mem_bus_ptr(NonNull::from(mem.as_mut()));
        (cpu, mem)
    }

    #[test]
    fn smm_save_area_matches_the_bios_relocation_contract() {
        // The Bochs BIOS SMM relocation handler (rombios32start.S) is the
        // consumer contract for the save-area layout: it reads the revision
        // byte at SMBASE+0xfefc, takes the x86-64 branch on 0x64, writes the
        // new SMBASE (0xa0000) into SMBASE+0xff00, and RSM must relocate.
        let (mut cpu, _mem) = cpu_with_memory();
        assert_eq!(cpu.smbase, 0x30000, "hardware-reset SMBASE");

        // Distinctive 64-bit state that only survives a 64-bit save map.
        cpu.gen_reg[15].set_rrx(0xdead_beef_cafe_f00d); // R15
        cpu.gen_reg[8].set_rrx(0x1122_3344_5566_7788); // R8
        let gs = super::super::decoder::BxSegregs::Gs as usize;
        cpu.sregs[gs].cache.u.set_segment_base(0xffff_8000_1234_5678);

        cpu.enter_system_management_mode();
        assert!(cpu.in_smm);
        assert_eq!(cpu.rip(), 0x8000, "SMM entry point");

        // Revision dword at SMBASE+0xfefc: low byte 0x64 + relocation bit.
        let rev = cpu.smram_read_physical_dword(0x3fefc);
        assert_eq!(rev, 0x0002_0064, "x86-64 revision id visible to the BIOS");
        assert_eq!(rev & 0xff, 0x64, "the BIOS cmp $0x64 dispatch byte");

        // The relocation handler's store: new SMBASE into SMBASE+0xff00.
        cpu.smram_write_physical_dword(0x3ff00, 0xa0000);

        cpu.rsm(&Instruction::default())
            .expect("RSM restores a self-saved image");
        assert!(!cpu.in_smm);
        assert_eq!(cpu.smbase, 0xa0000, "SMBASE relocated by the guest store");

        // 64-bit state made the roundtrip through the HI32/LO32 pairs.
        assert_eq!(cpu.gen_reg[15].rrx(), 0xdead_beef_cafe_f00d);
        assert_eq!(cpu.gen_reg[8].rrx(), 0x1122_3344_5566_7788);
        assert_eq!(
            cpu.sregs[gs].cache.u.segment_base(),
            0xffff_8000_1234_5678,
            "64-bit GS base survives the save/restore"
        );
    }

    #[test]
    fn smm_entry_environment_matches_bochs() {
        let (mut cpu, _mem) = cpu_with_memory();
        // Set CR0 bits the entry must NOT touch (ET, NE) plus ones it clears.
        let cr0_before = cpu.cr0.get32();
        cpu.enter_system_management_mode();

        // Bochs smm.cc: CR0 PE/EM/TS/PG cleared, everything else unmodified.
        let cr0 = cpu.cr0.get32();
        assert_eq!(cr0 & 0x8000_000D, 0, "PE/EM/TS/PG cleared");
        assert_eq!(
            cr0 & !0x8000_000D,
            cr0_before & !0x8000_000D,
            "all other CR0 bits unmodified (ET included)"
        );
        assert_eq!(cpu.eflags_materialized(), 0x2);
        assert_eq!(cpu.dr7.get32(), 0x400);
        let cs = super::super::decoder::BxSegregs::Cs as usize;
        assert_eq!(cpu.sregs[cs].selector.value, (0x30000 >> 4) as u16);
        assert_eq!(cpu.sregs[cs].cache.u.segment_base(), 0x30000);
    }

    #[test]
    fn rsm_with_inconsistent_image_shuts_down() {
        let (mut cpu, _mem) = cpu_with_memory();
        cpu.enter_system_management_mode();

        // Corrupt the saved CR4 image with unsupported bits (Bochs
        // check_CR4 failure path) — SMBASE+0xff48 is the CR4 slot.
        cpu.smram_write_physical_dword(0x3ff48, 0xffff_ffff);

        let result = cpu.rsm(&Instruction::default());
        assert!(
            matches!(result, Err(super::super::error::CpuError::CpuLoopRestart)),
            "inconsistent image must take the shutdown path"
        );
        assert!(
            matches!(
                cpu.activity_state,
                super::super::cpu::CpuActivityState::Shutdown
            ),
            "Bochs RSM: incorrect restore state shuts the CPU down"
        );
    }

    /// The relocated-SMBASE case, which is the one every real OS actually
    /// exercises: after the Bochs BIOS `smm_init` handshake, SMBASE is 0xa0000
    /// and the SMRAM control register is 0x0a (SMRAME=1, DOPEN=0, DCLS=0), so
    /// the handler at 0xa8000 *and* the 512-byte save area at 0xafe00..0xb0000
    /// live underneath the VGA legacy window. Every SMM access there must be
    /// routed to DRAM (Bochs memory.cc read/writePhysicalPage and misc_mem.cc
    /// getHostMemAddr SMRAM checks) while non-SMM accesses keep going to VGA.
    ///
    /// The POST relocation SMI cannot catch a regression here: it runs at
    /// SMBASE 0x30000, which is plain DRAM with no routing involved.
    #[test]
    fn relocated_smbase_save_area_is_routed_under_the_vga_window() {
        use crate::iodev::vga::BxVgaC;
        use crate::memory::{CpuMemoryPolicy, CpuTlbPin, MemoryDeviceId};

        let (mut cpu, mut mem) = cpu_with_memory();

        // VGA owns 0xa0000-0xbffff, exactly as the machine registers it.
        let mut vga = alloc::boxed::Box::new(BxVgaC::new());
        let vga_id = MemoryDeviceId::Vga(&mut *vga as *mut BxVgaC);
        mem.register_memory_handlers(vga_id, 0xA0000, 0xBFFFF)
            .expect("VGA handler registration");
        // SMRAM control 0x0a: available, not open, not restricted.
        mem.enable_smram(false, false);
        cpu.set_mem_bus_ptr(NonNull::from(mem.as_mut()));

        // Post-relocation SMBASE, and a GPR value that must survive the trip.
        cpu.smbase = 0xa0000;
        cpu.gen_reg[3].set_rrx(0x0bad_c0de_dead_1234); // RBX

        cpu.enter_system_management_mode();
        assert!(cpu.in_smm);
        assert_eq!(cpu.rip(), 0x8000, "SMM entry point");
        let cs = super::super::decoder::BxSegregs::Cs as usize;
        assert_eq!(
            cpu.sregs[cs].cache.u.segment_base(),
            0xa0000,
            "CS base is the relocated SMBASE, so CS:IP resolves to 0xa8000"
        );

        // The save state must be in DRAM, not in the VGA planes: read the
        // backing RAM directly, bypassing every routing decision.
        let pins = [CpuTlbPin::new(&*cpu)];
        let mut raw = [0u8; 4];
        assert_eq!(mem.read_ram(&pins, 0xafefc, &mut raw).unwrap(), 4);
        assert_eq!(
            u32::from_le_bytes(raw),
            SMM_REVISION_ID,
            "SMM entry must write the save state to DRAM under the VGA window"
        );
        assert_eq!(mem.read_ram(&pins, 0xaffe0, &mut raw).unwrap(), 4);
        assert_eq!(
            u32::from_le_bytes(raw),
            0xdead_1234,
            "EBX slot (SMBASE+0xffe0) lands in DRAM"
        );

        // A non-SMM read of the same address must NOT see SMRAM: it belongs to
        // the VGA handler while DOPEN is clear.
        let mut via_vga = [0xffu8; 4];
        mem.read_physical_page(
            &pins,
            CpuMemoryPolicy::default(),
            0xafefc,
            4,
            &mut via_vga,
        )
        .expect("non-SMM read is served by the VGA handler");
        assert_ne!(
            u32::from_le_bytes(via_vga),
            SMM_REVISION_ID,
            "outside SMM the save area must stay hidden behind the VGA window"
        );

        // ...and the CPU's own SMM-mode read of it does see SMRAM.
        assert_eq!(
            cpu.smram_read_physical_dword(0xafefc),
            SMM_REVISION_ID,
            "in SMM the save area reads back from DRAM"
        );

        // A DEVICE access never reaches SMRAM, even with the window wide open.
        // Bochs memory.cc guards the whole SMRAM block with `if (cpu != NULL)`,
        // so DMA and device-issued writes fall through to the VGA handler.
        mem.enable_smram(true, false);
        let mut via_cpu = [0xffu8; 4];
        mem.read_physical_page(&pins, CpuMemoryPolicy::default(), 0xafefc, 4, &mut via_cpu)
            .expect("CPU read with SMRAM open");
        assert_eq!(
            u32::from_le_bytes(via_cpu),
            SMM_REVISION_ID,
            "with SMRAM open a CPU access sees the save area"
        );
        let mut via_device = [0xffu8; 4];
        mem.read_physical_page(&pins, CpuMemoryPolicy::device(), 0xafefc, 4, &mut via_device)
            .expect("device read with SMRAM open");
        assert_ne!(
            u32::from_le_bytes(via_device),
            SMM_REVISION_ID,
            "a device access must never see SMRAM (Bochs memory.cc cpu != NULL)"
        );
        mem.enable_smram(false, false);

        // Instruction fetch inside SMM must get a direct DRAM span for the
        // handler page (Bochs getHostMemAddr: SMRAM direct access is granted
        // for code only), while a data access to the same page is vetoed so it
        // takes the slow, fully-checked physical path.
        let policy = CpuMemoryPolicy::new(true, false);
        assert!(
            mem.get_host_mem_addr_pinned(
                0xa8000,
                crate::cpu::rusty_box::MemoryAccessType::Execute,
                &pins,
                policy,
            )
            .unwrap()
            .is_some(),
            "SMM code fetch at SMBASE+0x8000 must map straight to DRAM"
        );
        assert!(
            mem.get_host_mem_addr_pinned(
                0xa8000,
                crate::cpu::rusty_box::MemoryAccessType::RW,
                &pins,
                policy,
            )
            .unwrap()
            .is_none(),
            "SMM data access must be vetoed here and fall back to the physical path"
        );

        // RSM restores through the same routing.
        cpu.rsm(&Instruction::default())
            .expect("RSM restores a self-saved image from relocated SMRAM");
        assert!(!cpu.in_smm);
        assert_eq!(cpu.smbase, 0xa0000, "SMBASE unchanged by this handler");
        assert_eq!(cpu.gen_reg[3].rrx(), 0x0bad_c0de_dead_1234);
    }

    /// `gen_reg` is `BX_GENERAL_REGISTERS + 4` entries: RAX..R15 followed by
    /// RIP, SSP, `BX_TMP_REGISTER` and `BX_NIL_REGISTER`. Only the first 16 are
    /// GPRs, and the SMRAM save/restore loops must stop there (Bochs smm.cc
    /// iterates `BX_GENERAL_REGISTERS`). Running to `gen_reg.len()` walks the
    /// field index off the end of the GPR pairs into RIP_HI32/EIP,
    /// RFLAGS_HI32/EFLAGS, DR6 and DR7 — and on the restore side writes those
    /// into the trailing slots.
    ///
    /// `BX_NIL_REGISTER` is the damaging one: the decoder emits it as the base
    /// of no-base addressing forms and `init.rs` sets it to zero once at reset,
    /// so a non-zero value silently displaces every later `[disp32]`-style
    /// memory access by that amount. That was invisible in the CPU trace and
    /// only surfaced hundreds of millions of instructions later.
    #[test]
    fn smm_roundtrip_leaves_the_non_gpr_register_slots_alone() {
        use crate::cpu::decoder::{
            BX_GENERAL_REGISTERS, BX_NIL_REGISTER, BX_TMP_REGISTER, BX_64BIT_REG_RIP,
        };

        let (mut cpu, _mem) = cpu_with_memory();

        // Distinctive sentinels that neither DR6 (0xffff0ff0) nor DR7 (0x400)
        // could produce, so a clobber is unambiguous.
        cpu.gen_reg[BX_TMP_REGISTER].set_rrx(0x7777_7777_7777_7777);
        cpu.gen_reg[BX_NIL_REGISTER].set_rrx(0);
        cpu.set_ssp(0x5555_5555_5555_5555);
        for n in 0..BX_GENERAL_REGISTERS {
            cpu.gen_reg[n].set_rrx(0x1000 + n as u64);
        }
        let rip_before = cpu.gen_reg[BX_64BIT_REG_RIP].rrx();

        cpu.enter_system_management_mode();
        cpu.rsm(&Instruction::default()).expect("clean RSM");

        for n in 0..BX_GENERAL_REGISTERS {
            assert_eq!(cpu.gen_reg[n].rrx(), 0x1000 + n as u64, "GPR {n} restored");
        }
        assert_eq!(
            cpu.gen_reg[BX_64BIT_REG_RIP].rrx(),
            rip_before,
            "RIP restored to the interrupted instruction"
        );
        assert_eq!(
            cpu.ssp(),
            0x5555_5555_5555_5555,
            "SSP round-trips through its own SMRAM slot"
        );
        assert_eq!(
            cpu.gen_reg[BX_TMP_REGISTER].rrx(),
            0x7777_7777_7777_7777,
            "BX_TMP_REGISTER is not a GPR and must not be touched by RSM"
        );
        assert_eq!(
            cpu.gen_reg[BX_NIL_REGISTER].rrx(),
            0,
            "BX_NIL_REGISTER must stay zero — it is the base of no-base \
             addressing forms, so any other value displaces every [disp32] access"
        );
    }
}
