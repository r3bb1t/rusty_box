use alloc::borrow::ToOwned;

use super::{
    cpuid::BxCpuIdTrait,
    decoder::{
        BxSegregs, BX_16BIT_REG_IP, BX_32BIT_REG_EIP, BX_64BIT_REG_RIP, BX_64BIT_REG_SSP,
        BX_TMP_REGISTER,
    },
    BxCpuC,
};

// according to RFC #344, we use "set_<name>" for setters
impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    // getters for 8 bit general registers
    #[inline]
    pub fn al(&self) -> u8 {
        unsafe { &self.gen_reg[0].word.byte.rl }.to_owned()
    }
    #[inline]
    pub fn cl(&self) -> u8 {
        unsafe { &self.gen_reg[1].word.byte.rl }.to_owned()
    }
    #[inline]
    pub fn dl(&self) -> u8 {
        unsafe { &self.gen_reg[2].word.byte.rl }.to_owned()
    }
    #[inline]
    pub fn bl(&self) -> u8 {
        unsafe { &self.gen_reg[3].word.byte.rl }.to_owned()
    }
    #[inline]
    pub fn ah(&self) -> u8 {
        unsafe { &self.gen_reg[0].word.byte.rh }.to_owned()
    }
    #[inline]
    pub fn ch(&self) -> u8 {
        unsafe { &self.gen_reg[1].word.byte.rh }.to_owned()
    }
    #[inline]
    pub fn dh(&self) -> u8 {
        unsafe { &self.gen_reg[2].word.byte.rh }.to_owned()
    }
    #[inline]
    pub fn bh(&self) -> u8 {
        unsafe { &self.gen_reg[3].word.byte.rh }.to_owned()
    }
    #[inline]
    pub fn tmp_8_l(&self) -> &u8 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].word.byte.rl }
    }

    // setters for 8 bit general registers
    #[inline]
    pub fn set_al(&mut self, val: u8) {
        self.gen_reg[0].word.byte.rl = val
    }
    #[inline]
    pub fn set_cl(&mut self, val: u8) {
        self.gen_reg[1].word.byte.rl = val
    }
    #[inline]
    pub fn set_dl(&mut self, val: u8) {
        self.gen_reg[2].word.byte.rl = val
    }
    #[inline]
    pub fn set_bl(&mut self, val: u8) {
        self.gen_reg[3].word.byte.rl = val
    }
    #[inline]
    pub fn set_ah(&mut self, val: u8) {
        self.gen_reg[0].word.byte.rh = val
    }
    #[inline]
    pub fn set_ch(&mut self, val: u8) {
        self.gen_reg[1].word.byte.rh = val
    }
    #[inline]
    pub fn set_dh(&mut self, val: u8) {
        self.gen_reg[2].word.byte.rh = val
    }
    #[inline]
    pub fn set_bh(&mut self, val: u8) {
        self.gen_reg[3].word.byte.rh = val
    }
    #[inline]
    pub fn set_tmpl_8_l(&mut self, val: u8) {
        self.gen_reg[BX_TMP_REGISTER].word.byte.rl = val
    }

    // getters for 16 bit general registers
    #[inline]
    pub fn ax(&self) -> u16 {
        unsafe { &self.gen_reg[0].word.rx }.to_owned()
    }
    #[inline]
    pub fn cx(&self) -> u16 {
        unsafe { &self.gen_reg[1].word.rx }.to_owned()
    }
    #[inline]
    pub fn dx(&self) -> u16 {
        unsafe { &self.gen_reg[2].word.rx }.to_owned()
    }
    #[inline]
    pub fn bx(&self) -> u16 {
        unsafe { &self.gen_reg[3].word.rx }.to_owned()
    }
    #[inline]
    pub fn sp(&self) -> u16 {
        unsafe { &self.gen_reg[4].word.rx }.to_owned()
    }
    #[inline]
    pub fn bp(&self) -> u16 {
        unsafe { &self.gen_reg[5].word.rx }.to_owned()
    }
    #[inline]
    pub fn si(&self) -> u16 {
        unsafe { &self.gen_reg[6].word.rx }.to_owned()
    }
    #[inline]
    pub fn di(&self) -> u16 {
        unsafe { &self.gen_reg[7].word.rx }.to_owned()
    }

    // setters for 16 bit general registers
    #[inline]
    pub fn set_ax(&mut self, val: u16) {
        self.gen_reg[0].word.rx = val
    }
    #[inline]
    pub fn set_cx(&mut self, val: u16) {
        self.gen_reg[1].word.rx = val
    }
    #[inline]
    pub fn set_dx(&mut self, val: u16) {
        self.gen_reg[2].word.rx = val
    }
    #[inline]
    pub fn set_bx(&mut self, val: u16) {
        self.gen_reg[3].word.rx = val
    }
    #[inline]
    pub fn set_sp(&mut self, val: u16) {
        self.gen_reg[4].word.rx = val
    }
    #[inline]
    pub fn set_bp(&mut self, val: u16) {
        self.gen_reg[5].word.rx = val
    }
    #[inline]
    pub fn set_si(&mut self, val: u16) {
        self.gen_reg[6].word.rx = val
    }
    #[inline]
    pub fn set_di(&mut self, val: u16) {
        self.gen_reg[7].word.rx = val
    }

    // access to 16 bit instruction pointer
    #[inline]
    pub fn ip(&self) -> &u16 {
        unsafe { &self.gen_reg[BX_16BIT_REG_IP].word.rx }
    }
    #[inline]
    pub fn set_ip(&mut self, val: u16) {
        self.gen_reg[BX_16BIT_REG_IP].word.rx = val
    }

    #[inline]
    pub fn tmpl_16(&self) -> &u16 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].word.rx }
    }
    #[inline]
    pub fn set_tmpl_16(&mut self, val: u16) {
        self.gen_reg[BX_TMP_REGISTER].word.rx = val
    }

    // getters for 32 bit general registers
    #[inline]
    pub fn eax(&self) -> u32 {
        unsafe { &self.gen_reg[0].dword.erx }.to_owned()
    }
    #[inline]
    pub fn ecx(&self) -> u32 {
        unsafe { &self.gen_reg[1].dword.erx }.to_owned()
    }
    #[inline]
    pub fn edx(&self) -> u32 {
        unsafe { &self.gen_reg[2].dword.erx }.to_owned()
    }
    #[inline]
    pub fn ebx(&self) -> u32 {
        unsafe { &self.gen_reg[3].dword.erx }.to_owned()
    }
    #[inline]
    pub fn esp(&self) -> u32 {
        unsafe { &self.gen_reg[4].dword.erx }.to_owned()
    }
    #[inline]
    pub fn ebp(&self) -> u32 {
        unsafe { &self.gen_reg[5].dword.erx }.to_owned()
    }
    #[inline]
    pub fn esi(&self) -> u32 {
        unsafe { &self.gen_reg[6].dword.erx }.to_owned()
    }
    #[inline]
    pub fn edi(&self) -> u32 {
        unsafe { &self.gen_reg[7].dword.erx }.to_owned()
    }

    // setters for 32 bit general registers
    // Bochs BX_WRITE_32BIT_REGZ: zero-extends to 64 bits (clears hrx)
    // Required by x86-64 architecture: 32-bit writes zero-extend to 64-bit
    #[inline]
    pub fn set_eax(&mut self, val: u32) {
        self.gen_reg[0].dword.erx = val;
        self.gen_reg[0].dword.hrx = 0;
    }
    #[inline]
    pub fn set_ecx(&mut self, val: u32) {
        self.gen_reg[1].dword.erx = val;
        self.gen_reg[1].dword.hrx = 0;
    }
    #[inline]
    pub fn set_edx(&mut self, val: u32) {
        self.gen_reg[2].dword.erx = val;
        self.gen_reg[2].dword.hrx = 0;
    }
    #[inline]
    pub fn set_ebx(&mut self, val: u32) {
        self.gen_reg[3].dword.erx = val;
        self.gen_reg[3].dword.hrx = 0;
    }
    #[inline]
    pub fn set_esp(&mut self, val: u32) {
        self.gen_reg[4].dword.erx = val;
        self.gen_reg[4].dword.hrx = 0;
    }
    #[inline]
    pub fn set_ebp(&mut self, val: u32) {
        self.gen_reg[5].dword.erx = val;
        self.gen_reg[5].dword.hrx = 0;
    }
    #[inline]
    pub fn set_esi(&mut self, val: u32) {
        self.gen_reg[6].dword.erx = val;
        self.gen_reg[6].dword.hrx = 0;
    }
    #[inline]
    pub fn set_edi(&mut self, val: u32) {
        self.gen_reg[7].dword.erx = val;
        self.gen_reg[7].dword.hrx = 0;
    }

    // access to 32 bit instruction pointer
    #[inline]
    pub fn eip(&self) -> u32 {
        unsafe { &self.gen_reg[BX_32BIT_REG_EIP].dword.erx }.to_owned()
    }
    #[inline]
    pub fn set_eip(&mut self, val: u32) {
        // EIP and RIP are the same register (index 16), just different views of a union
        // Matching C++ cpu.h:82: #define EIP (BX_CPU_THIS_PTR gen_reg[BX_32BIT_REG_EIP].dword.erx)
        // In C++, when you do "EIP = new_IP;", it directly assigns to dword.erx
        // The union ensures rrx low 32 bits are also updated, but high bits are NOT cleared here
        // High bits are cleared later in prefetch() via BX_CLEAR_64BIT_HIGH(BX_64BIT_REG_RIP)
        // See cpp_orig/bochs/cpu/cpu.cc:648 and ctrl_xfer16.cc:38

        self.gen_reg[BX_32BIT_REG_EIP].dword.erx = val;
        // Note: We do NOT clear high bits here to match C++ behavior
        // High bits will be cleared in prefetch() via bx_clear_64bit_high()
    }

    #[inline]
    pub fn tmp_32(&self) -> u32 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].dword.erx }.to_owned()
    }
    #[inline]
    pub fn set_tmp_32(&mut self, val: u32) {
        self.gen_reg[BX_TMP_REGISTER].dword.erx = val
    }

    /// Get current CPU mode (for diagnostics)
    /// 0=real, 1=v8086, 2=protected, 3=compat, 4=long64
    #[inline]
    pub fn get_cpu_mode(&self) -> u8 {
        // CpuMode doesn't implement Copy, so read the discriminant directly
        unsafe { *(&self.cpu_mode as *const _ as *const u8) }
    }

    /// Get CPU diagnostic string (IF, activity, inhibit)
    pub fn cpu_diag_string(&self) -> alloc::string::String {
        alloc::format!(
            "IF={} activity={:?} inhibit={} async_event={:#x}",
            self.interrupts_enabled(),
            self.activity_state,
            self.interrupts_inhibited(0x01),
            self.async_event,
        )
    }

    /// Get CS selector value (for diagnostics)
    #[inline]
    pub fn get_cs_selector(&self) -> u16 {
        self.sregs[super::decoder::BxSegregs::Cs as usize]
            .selector
            .value
    }

    pub fn get_ss_selector(&self) -> u16 {
        self.sregs[super::decoder::BxSegregs::Ss as usize]
            .selector
            .value
    }

    pub fn get_ss_base(&self) -> u64 {
        unsafe {
            self.sregs[super::decoder::BxSegregs::Ss as usize]
                .cache
                .u
                .segment
                .base
        }
    }

    // getters for 64 bit general registers
    #[inline]
    pub fn rax(&self) -> u64 {
        unsafe { &self.gen_reg[0].rrx }.to_owned()
    }
    #[inline]
    pub fn rcx(&self) -> u64 {
        unsafe { &self.gen_reg[1].rrx }.to_owned()
    }
    #[inline]
    pub fn rdx(&self) -> u64 {
        unsafe { &self.gen_reg[2].rrx }.to_owned()
    }
    #[inline]
    pub fn rbx(&self) -> u64 {
        unsafe { &self.gen_reg[3].rrx }.to_owned()
    }
    #[inline]
    pub fn rsp(&self) -> u64 {
        unsafe { &self.gen_reg[4].rrx }.to_owned()
    }
    #[inline]
    pub fn rbp(&self) -> u64 {
        unsafe { &self.gen_reg[5].rrx }.to_owned()
    }
    #[inline]
    pub fn rsi(&self) -> u64 {
        unsafe { &self.gen_reg[6].rrx }.to_owned()
    }
    #[inline]
    pub fn rdi(&self) -> u64 {
        unsafe { &self.gen_reg[7].rrx }.to_owned()
    }
    #[inline]
    pub fn r8(&self) -> u64 {
        unsafe { &self.gen_reg[8].rrx }.to_owned()
    }
    #[inline]
    pub fn r9(&self) -> u64 {
        unsafe { &self.gen_reg[9].rrx }.to_owned()
    }
    #[inline]
    pub fn r10(&self) -> u64 {
        unsafe { &self.gen_reg[10].rrx }.to_owned()
    }
    #[inline]
    pub fn r11(&self) -> u64 {
        unsafe { &self.gen_reg[11].rrx }.to_owned()
    }
    #[inline]
    pub fn r12(&self) -> u64 {
        unsafe { &self.gen_reg[12].rrx }.to_owned()
    }
    #[inline]
    pub fn r13(&self) -> u64 {
        unsafe { &self.gen_reg[13].rrx }.to_owned()
    }
    #[inline]
    pub fn r14(&self) -> u64 {
        unsafe { &self.gen_reg[14].rrx }.to_owned()
    }
    #[inline]
    pub fn r15(&self) -> u64 {
        unsafe { &self.gen_reg[15].rrx }.to_owned()
    }

    // setters for 32 bit general registers
    #[inline]
    pub fn set_rax(&mut self, val: u64) {
        self.gen_reg[0].rrx = val
    }
    #[inline]
    pub fn set_rcx(&mut self, val: u64) {
        self.gen_reg[1].rrx = val
    }
    #[inline]
    pub fn set_rdx(&mut self, val: u64) {
        self.gen_reg[2].rrx = val
    }
    #[inline]
    pub fn set_rbx(&mut self, val: u64) {
        self.gen_reg[3].rrx = val
    }
    #[inline]
    pub fn set_rsp(&mut self, val: u64) {
        self.gen_reg[4].rrx = val
    }
    #[inline]
    pub fn set_rbp(&mut self, val: u64) {
        self.gen_reg[5].rrx = val
    }
    #[inline]
    pub fn set_rsi(&mut self, val: u64) {
        self.gen_reg[6].rrx = val
    }
    #[inline]
    pub fn set_rdi(&mut self, val: u64) {
        self.gen_reg[7].rrx = val
    }
    #[inline]
    pub fn set_r8(&mut self, val: u64) {
        self.gen_reg[8].rrx = val
    }
    #[inline]
    pub fn set_r9(&mut self, val: u64) {
        self.gen_reg[9].rrx = val
    }
    #[inline]
    pub fn set_r10(&mut self, val: u64) {
        self.gen_reg[10].rrx = val
    }
    #[inline]
    pub fn set_r11(&mut self, val: u64) {
        self.gen_reg[11].rrx = val
    }
    #[inline]
    pub fn set_r12(&mut self, val: u64) {
        self.gen_reg[12].rrx = val
    }
    #[inline]
    pub fn set_r13(&mut self, val: u64) {
        self.gen_reg[13].rrx = val
    }
    #[inline]
    pub fn set_r14(&mut self, val: u64) {
        self.gen_reg[14].rrx = val
    }
    #[inline]
    pub fn set_r15(&mut self, val: u64) {
        self.gen_reg[15].rrx = val
    }

    // access to 32 bit instruction pointer
    #[inline]
    pub fn rip(&self) -> u64 {
        unsafe { &self.gen_reg[BX_64BIT_REG_RIP].rrx }.to_owned()
    }
    #[inline]
    pub fn set_rip(&mut self, val: u64) {
        self.gen_reg[BX_64BIT_REG_RIP].rrx = val
    }

    #[inline]
    pub fn ssp(&self) -> u64 {
        unsafe { &self.gen_reg[BX_64BIT_REG_SSP].rrx }.to_owned()
    }
    #[inline]
    pub fn set_ssp(&mut self, val: u64) {
        self.gen_reg[BX_64BIT_REG_SSP].rrx = val
    }

    #[inline]
    pub fn tmp_64(&self) -> u64 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].rrx }.to_owned()
    }
    #[inline]
    pub fn set_tmp_u64(&mut self, val: u64) {
        self.gen_reg[BX_TMP_REGISTER].rrx = val
    }

    // access to 64 bit MSR registers (FS.BASE / GS.BASE)
    #[inline]
    pub fn msr_fsbase(&self) -> u64 {
        unsafe { self.sregs[BxSegregs::Fs as usize].cache.u.segment.base }
    }
    #[inline]
    pub fn set_msr_fsbase(&mut self, val: u64) {
        unsafe { self.sregs[BxSegregs::Fs as usize].cache.u.segment.base = val }
    }
    #[inline]
    pub fn msr_gsbase(&self) -> u64 {
        unsafe { self.sregs[BxSegregs::Gs as usize].cache.u.segment.base }
    }
    #[inline]
    pub fn set_msr_gsbase(&mut self, val: u64) {
        unsafe { self.sregs[BxSegregs::Gs as usize].cache.u.segment.base = val }
    }

    // =========================================================================
    // Indexed register accessors (by register number)
    // =========================================================================

    /// Get 8-bit register by index (0=AL, 1=CL, 2=DL, 3=BL, 4=AH, 5=CH, 6=DH, 7=BH)
    /// For x86-64 with REX prefix, 4-7 map to SPL, BPL, SIL, DIL instead
    #[inline]
    pub fn get_gpr8(&self, reg: usize) -> u8 {
        if reg < 4 {
            // AL, CL, DL, BL
            unsafe { self.gen_reg[reg].word.byte.rl }
        } else if reg < 8 {
            // AH, CH, DH, BH (legacy mode) or SPL, BPL, SIL, DIL (x86-64 with REX)
            unsafe { self.gen_reg[reg - 4].word.byte.rh }
        } else {
            // R8B-R15B (x86-64)
            unsafe { self.gen_reg[reg].word.byte.rl }
        }
    }

    /// Set 8-bit register by index
    #[inline]
    pub fn set_gpr8(&mut self, reg: usize, val: u8) {
        if reg < 4 {
            self.gen_reg[reg].word.byte.rl = val;
        } else if reg < 8 {
            self.gen_reg[reg - 4].word.byte.rh = val;
        } else {
            self.gen_reg[reg].word.byte.rl = val;
        }
    }

    /// Get 16-bit register by index (0=AX, 1=CX, 2=DX, 3=BX, 4=SP, 5=BP, 6=SI, 7=DI)
    #[inline]
    pub fn get_gpr16(&self, reg: usize) -> u16 {
        unsafe { self.gen_reg[reg].word.rx }
    }

    /// Set 16-bit register by index
    #[inline]
    pub fn set_gpr16(&mut self, reg: usize, val: u16) {
        self.gen_reg[reg].word.rx = val;
    }

    /// Get 64-bit register by index (0=RAX, 1=RCX, ..., 15=R15)
    #[inline]
    pub fn get_gpr64(&self, reg: usize) -> u64 {
        unsafe { self.gen_reg[reg].rrx }
    }

    /// Set 64-bit register by index
    #[inline]
    pub fn set_gpr64(&mut self, reg: usize, val: u64) {
        self.gen_reg[reg].rrx = val;
    }

    /// Get IP (instruction pointer) as u16
    #[inline]
    pub fn get_ip(&self) -> u16 {
        unsafe { self.gen_reg[BX_16BIT_REG_IP].word.rx }
    }

    /// Get CR0 value (for diagnostics)
    #[inline]
    pub fn get_cr0_val(&self) -> u32 {
        self.cr0.get32()
    }

    /// Get CR3 value (for diagnostics)
    #[inline]
    pub fn get_cr3_val(&self) -> u64 {
        self.cr3
    }

    /// Get CR2 value (page-fault linear address, for diagnostics)
    #[inline]
    pub fn get_cr2_val(&self) -> u64 {
        self.cr2
    }

    /// Get IDTR base (for diagnostics)
    #[inline]
    pub fn get_idtr_base(&self) -> u64 {
        self.idtr.base
    }

    /// Get IDTR limit (for diagnostics)
    #[inline]
    pub fn get_idtr_limit(&self) -> u16 {
        self.idtr.limit
    }

    /// Get GDTR base (for diagnostics)
    #[inline]
    pub fn get_gdtr_base(&self) -> u64 {
        self.gdtr.base
    }

    /// Get GDTR limit (for diagnostics)
    #[inline]
    pub fn get_gdtr_limit(&self) -> u16 {
        self.gdtr.limit
    }

    /// Get CS base from cached descriptor (for diagnostics)
    #[inline]
    pub fn get_cs_base(&self) -> u64 {
        unsafe {
            self.sregs[super::decoder::BxSegregs::Cs as usize]
                .cache
                .u
                .segment
                .base
        }
    }

    /// Get DS base from cached descriptor (for diagnostics)
    #[inline]
    pub fn get_ds_base(&self) -> u64 {
        unsafe {
            self.sregs[super::decoder::BxSegregs::Ds as usize]
                .cache
                .u
                .segment
                .base
        }
    }

    /// Get DS selector (for diagnostics)
    #[inline]
    pub fn get_ds_selector(&self) -> u16 {
        self.sregs[super::decoder::BxSegregs::Ds as usize]
            .selector
            .value
    }

    /// Get async_event (for diagnostics)
    #[inline]
    pub fn get_async_event(&self) -> u32 {
        self.async_event
    }

    /// Get activity state (for diagnostics)
    #[inline]
    pub fn get_activity_state(&self) -> &super::cpu::CpuActivityState {
        &self.activity_state
    }

    /// Get handle_async_event interrupt delivery diagnostics
    pub fn get_hae_intr_diag(&self) -> (u64, u64, u64, u64) {
        (
            self.diag_hae_intr_delivered,
            self.diag_hae_intr_if_blocked,
            self.diag_hae_intr_no_pic,
            self.diag_hae_intr_pic_empty,
        )
    }

    /// Get exception counts by vector (0=DE, 6=UD, 13=GP, 14=PF)
    pub fn get_exception_diag(&self) -> &[u64; 32] {
        &self.diag_exception_counts
    }

    /// Get IaError (unimplemented opcode) diagnostics
    pub fn get_ia_error_diag(&self) -> (u64, u64) {
        (self.diag_ia_error_count, self.diag_ia_error_last_rip)
    }

    /// Get interrupt acknowledge vector counts
    pub fn get_iac_vectors(&self) -> &[u64; 256] {
        &self.diag_iac_vectors
    }

    /// Get inject_external_interrupt diagnostics
    pub fn get_inject_ext_intr_diag(&self) -> (u64, &[u64; 256]) {
        (self.diag_inject_ext_intr_count, &self.diag_inject_ext_intr_vectors)
    }

    /// Get software INT (INT nn) vector histogram
    pub fn get_soft_int_vectors(&self) -> &[u64; 256] {
        &self.diag_soft_int_vectors
    }

    /// Get software INT vector histogram for late calls (after BIOS POST, icount > 2M)
    pub fn get_soft_int_vectors_late(&self) -> &[u64; 256] {
        &self.diag_soft_int_vectors_late
    }

    /// Get INT 10h AH subfunction histogram (late calls only)
    pub fn get_int10h_ah_hist(&self) -> &[u64; 256] {
        &self.diag_int10h_ah_hist
    }

    /// Get TTY characters written via INT 10h AH=0Eh
    pub fn get_int10h_tty_chars(&self) -> &[u8] {
        &self.diag_int10h_tty_chars[..self.diag_int10h_tty_count]
    }

    /// Get first HLT in PM diagnostic data
    /// Returns (captured, icount, rip, cs, ss, eflags, regs[8], stack[16])
    pub fn get_first_pm_hlt(&self) -> Option<(u64, u32, u16, u16, u32, [u32; 8], [u32; 16])> {
        if self.diag_first_pm_hlt_captured {
            Some((
                self.diag_first_pm_hlt_icount,
                self.diag_first_pm_hlt_rip,
                self.diag_first_pm_hlt_cs,
                self.diag_first_pm_hlt_ss,
                self.diag_first_pm_hlt_eflags,
                self.diag_first_pm_hlt_regs,
                self.diag_first_pm_hlt_stack,
            ))
        } else {
            None
        }
    }

    /// Get PM↔RM transition counts
    pub fn get_pm_rm_transition_counts(&self) -> (u64, u64) {
        (self.diag_pm_to_rm_count, self.diag_rm_to_pm_count)
    }

    /// Set up address hit tracking. Provide up to 8 (addr, 0) pairs.
    pub fn set_addr_hit_watches(&mut self, watches: &[(u32, u64)]) {
        for (i, &(addr, count)) in watches.iter().enumerate().take(8) {
            self.diag_addr_hits[i] = (addr, count);
        }
    }

    /// Get address hit counters
    pub fn get_addr_hits(&self) -> &[(u32, u64); 8] {
        &self.diag_addr_hits
    }

    /// Check and count address hits (call from cpu_loop hot path)
    #[inline(always)]
    pub(crate) fn check_addr_hits(&mut self, rip: u32) {
        for entry in self.diag_addr_hits.iter_mut() {
            if entry.0 != 0 && entry.0 == rip {
                entry.1 += 1;
            }
        }
    }

    /// Get INT 10h icount range (first, last) and TTY icount range
    pub fn get_int10h_icount_range(&self) -> (u64, u64, u64, u64) {
        (
            self.diag_int10h_first_icount,
            self.diag_int10h_last_icount,
            self.diag_int10h_tty_first_icount,
            self.diag_int10h_tty_last_icount,
        )
    }

    /// Get a raw pointer to the icount field for device synchronization.
    /// SAFETY: The pointer is valid for the lifetime of the CPU struct.
    /// Used by PIT to synchronize counter reads with elapsed CPU time.
    pub fn icount_ptr(&self) -> *const u64 {
        &self.icount as *const u64
    }
}
