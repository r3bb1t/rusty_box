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
    pub fn al(&self) -> &u8 {
        unsafe { &self.gen_reg[0].word.byte.rl }
    }
    #[inline]
    pub fn cl(&self) -> &u8 {
        unsafe { &self.gen_reg[1].word.byte.rl }
    }
    #[inline]
    pub fn dl(&self) -> &u8 {
        unsafe { &self.gen_reg[2].word.byte.rl }
    }
    #[inline]
    pub fn bl(&self) -> &u8 {
        unsafe { &self.gen_reg[3].word.byte.rl }
    }
    #[inline]
    pub fn ah(&self) -> &u8 {
        unsafe { &self.gen_reg[0].word.byte.rh }
    }
    #[inline]
    pub fn ch(&self) -> &u8 {
        unsafe { &self.gen_reg[1].word.byte.rh }
    }
    #[inline]
    pub fn dh(&self) -> &u8 {
        unsafe { &self.gen_reg[2].word.byte.rh }
    }
    #[inline]
    pub fn bh(&self) -> &u8 {
        unsafe { &self.gen_reg[3].word.byte.rh }
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
    pub fn ax(&self) -> &u16 {
        unsafe { &self.gen_reg[0].word.rx }
    }
    #[inline]
    pub fn cx(&self) -> &u16 {
        unsafe { &self.gen_reg[1].word.rx }
    }
    #[inline]
    pub fn dx(&self) -> &u16 {
        unsafe { &self.gen_reg[2].word.rx }
    }
    #[inline]
    pub fn bx(&self) -> &u16 {
        unsafe { &self.gen_reg[3].word.rx }
    }
    #[inline]
    pub fn sp(&self) -> &u16 {
        unsafe { &self.gen_reg[4].word.rx }
    }
    #[inline]
    pub fn bp(&self) -> &u16 {
        unsafe { &self.gen_reg[5].word.rx }
    }
    #[inline]
    pub fn si(&self) -> &u16 {
        unsafe { &self.gen_reg[6].word.rx }
    }
    #[inline]
    pub fn di(&self) -> &u16 {
        unsafe { &self.gen_reg[7].word.rx }
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
    pub fn eax(&self) -> &u32 {
        unsafe { &self.gen_reg[0].dword.erx }
    }
    #[inline]
    pub fn ecx(&self) -> &u32 {
        unsafe { &self.gen_reg[1].dword.erx }
    }
    #[inline]
    pub fn edx(&self) -> &u32 {
        unsafe { &self.gen_reg[2].dword.erx }
    }
    #[inline]
    pub fn ebx(&self) -> &u32 {
        unsafe { &self.gen_reg[3].dword.erx }
    }
    #[inline]
    pub fn esp(&self) -> &u32 {
        unsafe { &self.gen_reg[4].dword.erx }
    }
    #[inline]
    pub fn ebp(&self) -> &u32 {
        unsafe { &self.gen_reg[5].dword.erx }
    }
    #[inline]
    pub fn esi(&self) -> &u32 {
        unsafe { &self.gen_reg[6].dword.erx }
    }
    #[inline]
    pub fn edi(&self) -> &u32 {
        unsafe { &self.gen_reg[7].dword.erx }
    }

    // setters for 32 bit general registers
    #[inline]
    pub fn set_eax(&mut self, val: u32) {
        self.gen_reg[0].dword.erx = val
    }
    #[inline]
    pub fn set_ecx(&mut self, val: u32) {
        self.gen_reg[1].dword.erx = val
    }
    #[inline]
    pub fn set_edx(&mut self, val: u32) {
        self.gen_reg[2].dword.erx = val
    }
    #[inline]
    pub fn set_ebx(&mut self, val: u32) {
        self.gen_reg[3].dword.erx = val
    }
    #[inline]
    pub fn set_esp(&mut self, val: u32) {
        self.gen_reg[4].dword.erx = val
    }
    #[inline]
    pub fn set_ebp(&mut self, val: u32) {
        self.gen_reg[5].dword.erx = val
    }
    #[inline]
    pub fn set_esi(&mut self, val: u32) {
        self.gen_reg[6].dword.erx = val
    }
    #[inline]
    pub fn set_edi(&mut self, val: u32) {
        self.gen_reg[7].dword.erx = val
    }

    // access to 32 bit instruction pointer
    #[inline]
    pub fn eip(&self) -> &u32 {
        unsafe { &self.gen_reg[BX_32BIT_REG_EIP].dword.erx }
    }
    #[inline]
    pub fn set_eip(&mut self, val: u32) {
        self.gen_reg[16].dword.erx = val
    }

    #[inline]
    pub fn tmp_32(&self) -> &u32 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].dword.erx }
    }
    #[inline]
    pub fn set_tmp_32(&mut self, val: u32) {
        self.gen_reg[BX_TMP_REGISTER].dword.erx = val
    }

    // getters for 64 bit general registers
    #[inline]
    pub fn rax(&self) -> &u64 {
        unsafe { &self.gen_reg[0].rrx }
    }
    #[inline]
    pub fn rcx(&self) -> &u64 {
        unsafe { &self.gen_reg[1].rrx }
    }
    #[inline]
    pub fn rdx(&self) -> &u64 {
        unsafe { &self.gen_reg[2].rrx }
    }
    #[inline]
    pub fn rbx(&self) -> &u64 {
        unsafe { &self.gen_reg[3].rrx }
    }
    #[inline]
    pub fn rsp(&self) -> &u64 {
        unsafe { &self.gen_reg[4].rrx }
    }
    #[inline]
    pub fn rbp(&self) -> &u64 {
        unsafe { &self.gen_reg[5].rrx }
    }
    #[inline]
    pub fn rsi(&self) -> &u64 {
        unsafe { &self.gen_reg[6].rrx }
    }
    #[inline]
    pub fn rdi(&self) -> &u64 {
        unsafe { &self.gen_reg[7].rrx }
    }
    #[inline]
    pub fn r8(&self) -> &u64 {
        unsafe { &self.gen_reg[8].rrx }
    }
    #[inline]
    pub fn r9(&self) -> &u64 {
        unsafe { &self.gen_reg[9].rrx }
    }
    #[inline]
    pub fn r10(&self) -> &u64 {
        unsafe { &self.gen_reg[10].rrx }
    }
    #[inline]
    pub fn r11(&self) -> &u64 {
        unsafe { &self.gen_reg[11].rrx }
    }
    #[inline]
    pub fn r12(&self) -> &u64 {
        unsafe { &self.gen_reg[12].rrx }
    }
    #[inline]
    pub fn r13(&self) -> &u64 {
        unsafe { &self.gen_reg[13].rrx }
    }
    #[inline]
    pub fn r14(&self) -> &u64 {
        unsafe { &self.gen_reg[14].rrx }
    }
    #[inline]
    pub fn r15(&self) -> &u64 {
        unsafe { &self.gen_reg[15].rrx }
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
    pub fn rip(&self) -> &u64 {
        unsafe { &self.gen_reg[BX_64BIT_REG_RIP].rrx }
    }
    #[inline]
    pub fn set_rip(&mut self, val: u64) {
        self.gen_reg[BX_64BIT_REG_RIP].rrx = val
    }

    #[inline]
    pub fn ssp(&self) -> &u64 {
        unsafe { &self.gen_reg[BX_64BIT_REG_SSP].rrx }
    }
    #[inline]
    pub fn set_ssp(&mut self, val: u64) {
        self.gen_reg[BX_64BIT_REG_SSP].rrx = val
    }

    #[inline]
    pub fn tmp_64(&self) -> &u64 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].rrx }
    }
    #[inline]
    pub fn set_tmp_u64(&mut self, val: u64) {
        self.gen_reg[BX_TMP_REGISTER].rrx = val
    }

    // access to 64 bit MSR registers
    #[inline]
    pub fn msr_fsbase(&self) -> &u64 {
        unsafe { &self.gen_reg[BxSegregs::Fs as usize].rrx }
    }
    #[inline]
    pub fn set_msr_fsbase(&mut self, val: u64) {
        self.gen_reg[BxSegregs::Fs as usize].rrx = val
    }
    #[inline]
    pub fn msr_gsbase(&self) -> &u64 {
        unsafe { &self.gen_reg[BxSegregs::Gs as usize].rrx }
    }
    #[inline]
    pub fn set_msr_gsbase(&mut self, val: u64) {
        self.gen_reg[BxSegregs::Gs as usize].rrx = val
    }
}
