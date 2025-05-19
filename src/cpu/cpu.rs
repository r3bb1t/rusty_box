use crate::config::{BxAddress, BxPhyAddress, BxPtrEquiv};

use super::{
    apic::BxLocalApic,
    cpuid::BxCpuTrait,
    cpustats::BxCpuStatistics,
    crregs::{BxCr0, BxCr4, BxDr6, BxDr7, Xcr0, MSR},
    decoder::{
        BxSegregs, BX_16BIT_REG_IP, BX_32BIT_REG_EIP, BX_64BIT_REG_RIP, BX_64BIT_REG_SSP,
        BX_GENERAL_REGISTERS, BX_ISA_EXTENSIONS_ARRAY_SIZE, BX_TMP_REGISTER, BX_XMM_REGISTERS,
    },
    descriptor::{BxGlobalSegmentReg, BxSegmentReg},
    i387::{BxPackedRegister, I387},
    icache::BxIcache,
    lazy_flags::BxLazyflagsEntry,
    svm::VmcbCache,
    tlb::BxHostpageaddr,
    vmx::{VmcsCache, VmcsMapping, VmxCap},
    xmm::{BxMxcsr, BxZmmReg},
};

#[cfg(feature = "bx_support_amx")]
use super::avx::amx::AMX;

#[cfg(feature = "bx_support_memtype")]
use super::tlb::BxMemType;

// region:  x64 big endian

#[cfg(feature = "bx_support_x86_64")]
#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenRegWord {
    pub dword_filler: u16,
    pub word_filler: u16,
    pub rx: u16,
    pub byte: BxWordByte,
}

#[cfg(feature = "bx_support_x86_64")]
#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxGenRegDword {
    pub hrx: u32,
    pub erx: u32,
}

#[cfg(feature = "bx_support_x86_64")]
#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenRegWordInner {
    pub rx: y16,
    pub byte: BxWordByte,
}

#[cfg(feature = "bx_support_x86_64")]
#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxWordByte {
    pub rh: u8,
    pub rl: u8,
}

// endregion:  x64 big endian

// region:  x64 little endian

#[cfg(feature = "bx_support_x86_64")]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenReg {
    pub word: BxGenRegWord,
    pub rrx: u64,
    pub dword: BxGenRegDword,
}

#[cfg(feature = "bx_support_x86_64")]
#[cfg(not(feature = "bx_big_endian"))]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenRegWord {
    pub rx: u16,
    pub byte: BxWordByte,
    pub word_filler: u16,
    pub dword_filler: u16,
}

#[cfg(feature = "bx_support_x86_64")]
#[cfg(not(feature = "bx_big_endian"))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxGenRegDword {
    pub erx: u32,
    pub hrx: u32,
}

#[cfg(feature = "bx_support_x86_64")]
#[cfg(not(feature = "bx_big_endian"))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxWordByte {
    pub rl: u8,
    pub rh: u8,
}

// endregion:  x64 little endian

// region:  x86 (32 bit) little endian

#[cfg(not(feature = "bx_support_x86_64"))]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenReg {
    pub dword: BxGenRegDword,
    pub word: BxGenRegWord,
}

#[cfg(not(feature = "bx_support_x86_64"))]
#[cfg(not(feature = "bx_big_endian"))]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenRegWord {
    pub rx: u16,
    pub byte: BxWordByte,
    pub word_filler: u16,
}

#[cfg(not(feature = "bx_support_x86_64"))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxGenRegDword {
    pub erx: u32,
}

#[cfg(not(feature = "bx_support_x86_64"))]
#[cfg(not(feature = "bx_big_endian"))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxWordByte {
    pub rl: u8,
    pub rh: u8,
}

// endregion:  x86 (32 bit) little endian

// region:  x86 (32 bit) big endian

#[cfg(not(feature = "bx_support_x86_64"))]
#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenRegWord {
    pub word_filler: u16,
    pub rx: u16,
    pub byte: BxWordByte,
}

#[cfg(not(feature = "bx_support_x86_64"))]
#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxWordByte {
    pub rh: u8,
    pub rl: u8,
}

// endregion:  x86 (32 bit) big endian

impl core::fmt::Debug for BxGenReg {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        #[cfg(feature = "bx_support_x86_64")]
        write!(f, "{:#x}", unsafe { self.rrx })?;
        #[cfg(not(feature = "bx_support_x86_64"))]
        write!(f, "{:#x}", unsafe { self.dword.erx })?;
        Ok(())
    }
}

const BX_MSR_MAX_INDEX: usize = 0x1000;

#[derive(Debug)]
pub struct BxCpuC<'c, I: BxCpuTrait> {
    bx_cpuid: u32,

    #[cfg(any(
        feature = "bx_cpu_level_4",
        feature = "bx_cpu_level_5",
        feature = "bx_cpu_level_6"
    ))]
    cpuid: I,

    ia_extensions_bitmask: [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE],

    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    vmx_extensions_bitmask: u32,

    #[cfg(feature = "bx_support_svm")]
    svm_extensions_bitmask: u32,

    /// General register set
    /// rax: accumulator
    /// rbx: base
    /// rcx: count
    /// rdx: data
    /// rbp: base pointer
    /// rsi: source index
    /// rdi: destination index
    /// esp: stack pointer
    /// r8..r15 x86-64 extended registers
    /// rip: instruction pointer
    /// ssp: shadow stack pointer
    /// tmp: temp register
    /// nil: null register
    gen_reg: [BxGenReg; BX_GENERAL_REGISTERS + 4],

    //
    // 31|30|29|28| 27|26|25|24| 23|22|21|20| 19|18|17|16
    // ==|==|=====| ==|==|==|==| ==|==|==|==| ==|==|==|==
    //  0| 0| 0| 0|  0| 0| 0| 0|  0| 0|ID|VP| VF|AC|VM|RF
    //
    // 15|14|13|12| 11|10| 9| 8|  7| 6| 5| 4|  3| 2| 1| 0
    // ==|==|=====| ==|==|==|==| ==|==|==|==| ==|==|==|==
    //  0|NT| IOPL| OF|DF|IF|TF| SF|ZF| 0|AF|  0|PF| 1|CF
    //
    eflags: u32, // Raw 32-bit value in x86 bit position.

    /// lazy arithmetic flags state
    oszapc: BxLazyflagsEntry,

    /// so that we can back up when handling faults, exceptions, etc.
    /// we need to store the value of the instruction pointer, before
    /// each fetch/execute cycle.
    prev_rip: BxAddress,
    prev_rsp: BxAddress,

    #[cfg(feature = "bx_support_cet")]
    prev_ssp: BxAddress,
    speculative_rsp: bool,

    icount: u64,
    icount_last_sync: u64,

    /// What events to inhibit at any given time.  Certain instructions
    /// inhibit interrupts, some debug exceptions and single-step traps.
    inhibit_mask: u32,
    inhibit_icount: u64,

    /// user segment register set
    sregs: [BxSegmentReg; 6],

    // system segment registers
    /// global descriptor table register
    gdtr: BxGlobalSegmentReg,
    /// interrupt descriptor table register
    idtr: BxGlobalSegmentReg,
    /// local descriptor table register
    ldtr: BxSegmentReg,
    /// task register
    tr: BxSegmentReg,

    // debug registers DR0-DR7
    /// Dr0-DR3
    dr: [BxAddress; 5],
    dr6: BxDr6,
    dr7: BxDr7,

    /// holds DR6 value (16bit) to be set
    debug_trap: u32,

    // Control registers
    bx_cr0_t: BxCr0,
    cr2: BxAddress,
    cr3: BxAddress,

    #[cfg(any(feature = "bx_cpu_level_5", feature = "bx_cpu_level_6"))]
    cr4: BxCr4,
    #[cfg(any(feature = "bx_cpu_level_5", feature = "bx_cpu_level_6"))]
    cr4_suppmask: u32,

    #[cfg(any(feature = "bx_cpu_level_5", feature = "bx_cpu_level_6"))]
    #[cfg(feature = "bx_support_x86_64")]
    linaddr_width: u32,
    #[cfg(any(feature = "bx_cpu_level_5", feature = "bx_cpu_level_6"))]
    #[cfg(feature = "bx_support_x86_64")]
    efer_suppmask: u32,

    #[cfg(any(feature = "bx_cpu_level_5", feature = "bx_cpu_level_6"))]
    /// TSC: Time Stamp Counter
    /// Instead of storing a counter and incrementing it every instruction, we
    /// remember the time in ticks that it was reset to zero.  With a little
    /// algebra, we can also support setting it to something other than zero.
    /// Don't read this directly; use get_TSC and set_TSC to access the TSC.
    tsc_adjust: i64,

    #[cfg(any(feature = "bx_cpu_level_5", feature = "bx_cpu_level_6"))]
    #[cfg(any(
        feature = "bx_support_vmx_1",
        feature = "bx_support_vmx_2",
        feature = "bx_support_svm"
    ))]
    tsc_offset: i64,

    #[cfg(feature = "bx_cpu_level_6")]
    xcr0: Xcr0,

    #[cfg(feature = "bx_cpu_level_6")]
    xcr0_suppmask: u32,
    #[cfg(feature = "bx_cpu_level_6")]
    ia32_xss_suppmask: u32,

    // protection keys
    #[cfg(feature = "bx_support_pkeys")]
    pkru: u32,
    #[cfg(feature = "bx_support_pkeys")]
    pkrs: u32,

    // unpacked protection keys to be tested together with accessBits from TLB
    // the unpacked key is stored in the accessBits format:
    //     bit 5: Execute from User   privilege is OK
    //     bit 4: Execute from System privilege is OK
    //     bit 3: Write   from User   privilege is OK
    //     bit 2: Write   from System privilege is OK
    //     bit 1: Read    from User   privilege is OK
    //     bit 0: Read    from System privilege is OK
    // But only bits 1 and 3 are relevant, all others should be set to '1
    // When protection key prevents all accesses to the page both bits 1 and 3 are cleared
    // When protection key prevents writes to the page bit 1 will be set and 3 cleared
    // When no protection keys are enabled all bits should be set for all keys
    #[cfg(feature = "bx_support_pkeys")]
    rd_pkey: [u32; 16],
    #[cfg(feature = "bx_support_pkeys")]
    wr_pkey: [u32; 16],

    #[cfg(feature = "bx_support_uintr")]
    uintr: Uintr,

    #[cfg(feature = "bx_support_fpu")]
    the_i387: I387,

    // Vector register set
    // vmm0-vmmN: up to 32 vector registers
    // vtmp: temp register
    #[cfg(feature = "bx_support_evex")]
    vmm: [BxZmmReg; BX_XMM_REGISTERS],
    // Note, didnt check for other features. Basically only aligment changes
    mxcsr: BxMxcsr,
    mxcsr_mask: u32,

    #[cfg(feature = "bx_support_evex")]
    opmask: [BxGenReg; 8],

    #[cfg(feature = "bx_support_monitor_mwait")]
    monitor: MonitorAddr,

    #[cfg(feature = "bx_support_apic")]
    lapic: BxLocalApic<'c, I>,

    /// SMM base register
    smbase: u32,

    #[cfg(any(feature = "bx_cpu_level_5", feature = "bx_cpu_level_6"))]
    msr: BxRegsMsr,

    #[cfg(feature = "bx_configure_msrs")]
    msrs: [MSR; BX_MSR_MAX_INDEX],

    #[cfg(feature = "bx_support_amx")]
    amx: AMX,

    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    in_vmx: bool,
    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    in_vmx_guest: bool,
    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    /// save in_vmx and in_vmx_guest flags when in SMM mode
    in_smm_vmx: bool,
    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    in_smm_vmx_guest: bool,
    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    vmcsptr: u64,

    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    #[cfg(feature = "bx_support_memtype")]
    vmcs_memtype: BxMemType,

    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    vmxonptr: u64,

    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    vmcs: VmcsCache,
    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    vmx_cap: VmxCap,
    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    vmcs_map: VmcsMapping,

    #[cfg(feature = "bx_support_svm")]
    in_svm_guest: bool,
    #[cfg(feature = "bx_support_svm")]
    /// global interrupt enable flag, when zero all external interrupt disabled
    svm_gif: bool,
    #[cfg(feature = "bx_support_svm")]
    vmcbptr: BxPhyAddress,
    #[cfg(feature = "bx_support_svm")]
    vmcbhostptr: BxHostpageaddr,
    #[cfg(feature = "bx_support_svm")]
    #[cfg(feature = "bx_support_memtype")]
    vmcb_memtype: BxMemType,

    #[cfg(feature = "bx_support_svm")]
    vmcb: VmcbCache,

    #[cfg(any(
        feature = "bx_support_vmx_1",
        feature = "bx_support_vmx_2",
        feature = "bx_support_svm"
    ))]
    in_event: bool,

    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    nmi_unblocking_iret: bool,

    /// 1 if processing external interrupt or exception
    /// or if not related to current instruction,
    /// 0 if current CS:EIP caused exception */
    ext: bool,

    // Todo: Maybe enum?
    activity_state: u32,

    pending_event: u32,
    event_mask: u32,
    // keep 32-bit because of BX_ASYNC_EVENT_STOP_TRACE
    async_event: u32,

    in_smm: bool,
    cpu_mode: u32,
    user_pl: bool,

    #[cfg(any(feature = "bx_cpu_level_5", feature = "bx_cpu_level_6"))]
    ignore_bad_msrs: bool,

    cpu_state_use_ok: u32, // format of BX_FETCH_MODE_*

    // FIXME: skipped   static jmp_buf jmp_buf_env;
    last_exception_type: u32,

    #[cfg(feature = "bx_support_handlers_chaining_speedups")]
    cpuloop_stack_anchor: Option<&'c [u8]>,

    // Boundaries of current code page, based on EIP
    eip_page_bias: BxAddress,
    eip_page_window_size: u32,
    eip_fetch_ptr: &'c [u8],
    p_addr_fetch_page: BxPhyAddress, // Guest physical address of current instruction page

    // Boundaries of current stack page, based on ESP
    // Linear address of current stack page
    esp_page_bias: BxAddress,
    esp_page_window_size: u32,
    esp_host_ptr: &'c [u8],
    /// Guest physical address of current stack page
    p_addr_stack_page: BxPhyAddress,

    #[cfg(feature = "bx_support_memtype")]
    espPageMemtype: BxMemType,

    #[cfg(not(feature = "bx_support_smp"))]
    esp_page_fine_granularity_mapping: u32,

    #[cfg(any(
        feature = "bx_cpu_level_4",
        feature = "bx_cpu_level_5",
        feature = "bx_cpu_level_6"
    ))]
    #[cfg(feature = "bx_support_alignment_check")]
    alignment_check_mask: u32,

    stats: BxCpuStatistics,

    #[cfg(feature = "bx_debugger")]
    watchpoint: BxPhyAddress,
    #[cfg(feature = "bx_debugger")]
    break_point: u8,
    #[cfg(feature = "bx_debugger")]
    magic_break: u8,
    #[cfg(feature = "bx_debugger")]
    stop_reason: u8,
    #[cfg(feature = "bx_debugger")]
    trace: bool,
    #[cfg(feature = "bx_debugger")]
    trace_reg: bool,
    #[cfg(feature = "bx_debugger")]
    trace_mem: bool,
    #[cfg(feature = "bx_debugger")]
    mode_break: bool,

    #[cfg(feature = "bx_debugger")]
    #[cfg(any(
        feature = "bx_support_vmx_1",
        feature = "bx_support_vmx_2",
        feature = "bx_support_svm"
    ))]
    vmexit_break: bool,

    #[cfg(feature = "bx_debugger")]
    show_flag: u32,
    #[cfg(feature = "bx_debugger")]
    guard_found: BxGuardFound,

    #[cfg(feature = "bx_instrumentation")]
    far_branch: FarBranch,

    #[cfg(feature = "bx_cpu_level_6")]
    pdptrcache: PdptrCache,

    /// An instruction cache.  Each entry should be exactly 32 bytes, and
    /// this structure should be aligned on a 32-byte boundary to be friendly
    /// with the host cache lines.
    i_cache: BxIcache,
    fetch_mode_mask: u32,

    address_xlation: AddressXlation,
}

// Implement getters and setters
// according to RFC #344, we use "set_<name>" for setters
impl<'c, I: BxCpuTrait> BxCpuC<'c, I> {
    // getters for 8 bit general registers
    #[inline]
    fn al(&self) -> &u8 {
        unsafe { &self.gen_reg[0].word.byte.rl }
    }
    #[inline]
    fn cl(&self) -> &u8 {
        unsafe { &self.gen_reg[1].word.byte.rl }
    }
    #[inline]
    fn dl(&self) -> &u8 {
        unsafe { &self.gen_reg[2].word.byte.rl }
    }
    #[inline]
    fn bl(&self) -> &u8 {
        unsafe { &self.gen_reg[3].word.byte.rl }
    }
    #[inline]
    fn ah(&self) -> &u8 {
        unsafe { &self.gen_reg[0].word.byte.rh }
    }
    #[inline]
    fn ch(&self) -> &u8 {
        unsafe { &self.gen_reg[1].word.byte.rh }
    }
    #[inline]
    fn dh(&self) -> &u8 {
        unsafe { &self.gen_reg[2].word.byte.rh }
    }
    #[inline]
    fn bh(&self) -> &u8 {
        unsafe { &self.gen_reg[3].word.byte.rh }
    }
    #[inline]
    fn tmp_8_l(&self) -> &u8 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].word.byte.rl }
    }

    // setters for 8 bit general registers
    #[inline]
    fn set_al(&mut self, val: u8) {
        self.gen_reg[0].word.byte.rl = val
    }
    #[inline]
    fn set_cl(&mut self, val: u8) {
        self.gen_reg[1].word.byte.rl = val
    }
    #[inline]
    fn set_dl(&mut self, val: u8) {
        self.gen_reg[2].word.byte.rl = val
    }
    #[inline]
    fn set_bl(&mut self, val: u8) {
        self.gen_reg[3].word.byte.rl = val
    }
    #[inline]
    fn set_ah(&mut self, val: u8) {
        self.gen_reg[0].word.byte.rh = val
    }
    #[inline]
    fn set_ch(&mut self, val: u8) {
        self.gen_reg[1].word.byte.rh = val
    }
    #[inline]
    fn set_dh(&mut self, val: u8) {
        self.gen_reg[2].word.byte.rh = val
    }
    #[inline]
    fn set_bh(&mut self, val: u8) {
        self.gen_reg[3].word.byte.rh = val
    }
    #[inline]
    fn set_tmpl_8_l(&mut self, val: u8) {
        self.gen_reg[BX_TMP_REGISTER].word.byte.rl = val
    }

    // getters for 16 bit general registers
    #[inline]
    fn ax(&self) -> &u16 {
        unsafe { &self.gen_reg[0].word.rx }
    }
    #[inline]
    fn cx(&self) -> &u16 {
        unsafe { &self.gen_reg[1].word.rx }
    }
    #[inline]
    fn dx(&self) -> &u16 {
        unsafe { &self.gen_reg[2].word.rx }
    }
    #[inline]
    fn bx(&self) -> &u16 {
        unsafe { &self.gen_reg[3].word.rx }
    }
    #[inline]
    fn sp(&self) -> &u16 {
        unsafe { &self.gen_reg[4].word.rx }
    }
    #[inline]
    fn bp(&self) -> &u16 {
        unsafe { &self.gen_reg[5].word.rx }
    }
    #[inline]
    fn si(&self) -> &u16 {
        unsafe { &self.gen_reg[6].word.rx }
    }
    #[inline]
    fn di(&self) -> &u16 {
        unsafe { &self.gen_reg[7].word.rx }
    }

    // setters for 16 bit general registers
    #[inline]
    fn set_ax(&mut self, val: u16) {
        self.gen_reg[0].word.rx = val
    }
    #[inline]
    fn set_cx(&mut self, val: u16) {
        self.gen_reg[1].word.rx = val
    }
    #[inline]
    fn set_dx(&mut self, val: u16) {
        self.gen_reg[2].word.rx = val
    }
    #[inline]
    fn set_bx(&mut self, val: u16) {
        self.gen_reg[3].word.rx = val
    }
    #[inline]
    fn set_sp(&mut self, val: u16) {
        self.gen_reg[4].word.rx = val
    }
    #[inline]
    fn set_bp(&mut self, val: u16) {
        self.gen_reg[5].word.rx = val
    }
    #[inline]
    fn set_si(&mut self, val: u16) {
        self.gen_reg[6].word.rx = val
    }
    #[inline]
    fn set_di(&mut self, val: u16) {
        self.gen_reg[7].word.rx = val
    }

    // access to 16 bit instruction pointer
    #[inline]
    fn ip(&self) -> &u16 {
        unsafe { &self.gen_reg[BX_16BIT_REG_IP].word.rx }
    }
    #[inline]
    fn set_ip(&mut self, val: u16) {
        self.gen_reg[BX_16BIT_REG_IP].word.rx = val
    }

    #[inline]
    fn tmpl_16(&self) -> &u16 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].word.rx }
    }
    #[inline]
    fn set_tmpl_16(&mut self, val: u16) {
        self.gen_reg[BX_TMP_REGISTER].word.rx = val
    }

    // getters for 32 bit general registers
    #[inline]
    fn eax(&self) -> &u32 {
        unsafe { &self.gen_reg[0].dword.erx }
    }
    #[inline]
    fn ecx(&self) -> &u32 {
        unsafe { &self.gen_reg[1].dword.erx }
    }
    #[inline]
    fn edx(&self) -> &u32 {
        unsafe { &self.gen_reg[2].dword.erx }
    }
    #[inline]
    fn ebx(&self) -> &u32 {
        unsafe { &self.gen_reg[3].dword.erx }
    }
    #[inline]
    fn esp(&self) -> &u32 {
        unsafe { &self.gen_reg[4].dword.erx }
    }
    #[inline]
    fn ebp(&self) -> &u32 {
        unsafe { &self.gen_reg[5].dword.erx }
    }
    #[inline]
    fn esi(&self) -> &u32 {
        unsafe { &self.gen_reg[6].dword.erx }
    }
    #[inline]
    fn edi(&self) -> &u32 {
        unsafe { &self.gen_reg[7].dword.erx }
    }

    // setters for 32 bit general registers
    #[inline]
    fn set_eax(&mut self, val: u32) {
        self.gen_reg[0].dword.erx = val
    }
    #[inline]
    fn set_ecx(&mut self, val: u32) {
        self.gen_reg[1].dword.erx = val
    }
    #[inline]
    fn set_edx(&mut self, val: u32) {
        self.gen_reg[2].dword.erx = val
    }
    #[inline]
    fn set_ebx(&mut self, val: u32) {
        self.gen_reg[3].dword.erx = val
    }
    #[inline]
    fn set_esp(&mut self, val: u32) {
        self.gen_reg[4].dword.erx = val
    }
    #[inline]
    fn set_ebp(&mut self, val: u32) {
        self.gen_reg[5].dword.erx = val
    }
    #[inline]
    fn set_esi(&mut self, val: u32) {
        self.gen_reg[6].dword.erx = val
    }
    #[inline]
    fn set_edi(&mut self, val: u32) {
        self.gen_reg[7].dword.erx = val
    }

    // access to 32 bit instruction pointer
    #[inline]
    fn eip(&self) -> &u32 {
        unsafe { &self.gen_reg[BX_32BIT_REG_EIP].dword.erx }
    }
    #[inline]
    fn set_eip(&mut self, val: u32) {
        self.gen_reg[BX_32BIT_REG_EIP].dword.erx = val
    }

    #[inline]
    fn tmp_32(&self) -> &u32 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].dword.erx }
    }
    #[inline]
    fn set_tmp_32(&mut self, val: u32) {
        self.gen_reg[BX_TMP_REGISTER].dword.erx = val
    }

    // getters for 64 bit general registers
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rax(&self) -> &u64 {
        unsafe { &self.gen_reg[0].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rcx(&self) -> &u64 {
        unsafe { &self.gen_reg[1].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rdx(&self) -> &u64 {
        unsafe { &self.gen_reg[2].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rbx(&self) -> &u64 {
        unsafe { &self.gen_reg[3].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rsp(&self) -> &u64 {
        unsafe { &self.gen_reg[4].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rbp(&self) -> &u64 {
        unsafe { &self.gen_reg[5].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rsi(&self) -> &u64 {
        unsafe { &self.gen_reg[6].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rdi(&self) -> &u64 {
        unsafe { &self.gen_reg[7].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn r8(&self) -> &u64 {
        unsafe { &self.gen_reg[8].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn r9(&self) -> &u64 {
        unsafe { &self.gen_reg[9].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn r10(&self) -> &u64 {
        unsafe { &self.gen_reg[10].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn r11(&self) -> &u64 {
        unsafe { &self.gen_reg[11].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn r12(&self) -> &u64 {
        unsafe { &self.gen_reg[12].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn r13(&self) -> &u64 {
        unsafe { &self.gen_reg[13].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn r14(&self) -> &u64 {
        unsafe { &self.gen_reg[14].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn r15(&self) -> &u64 {
        unsafe { &self.gen_reg[15].rrx }
    }

    // setters for 32 bit general registers
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rax(&mut self, val: u64) {
        self.gen_reg[0].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rcx(&mut self, val: u64) {
        self.gen_reg[1].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rdx(&mut self, val: u64) {
        self.gen_reg[2].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rbx(&mut self, val: u64) {
        self.gen_reg[3].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rsp(&mut self, val: u64) {
        self.gen_reg[4].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rbp(&mut self, val: u64) {
        self.gen_reg[5].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rsi(&mut self, val: u64) {
        self.gen_reg[6].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rdi(&mut self, val: u64) {
        self.gen_reg[7].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_r8(&mut self, val: u64) {
        self.gen_reg[8].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_r9(&mut self, val: u64) {
        self.gen_reg[9].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_r10(&mut self, val: u64) {
        self.gen_reg[10].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_r11(&mut self, val: u64) {
        self.gen_reg[11].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_r12(&mut self, val: u64) {
        self.gen_reg[12].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_r13(&mut self, val: u64) {
        self.gen_reg[13].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_r14(&mut self, val: u64) {
        self.gen_reg[14].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_r15(&mut self, val: u64) {
        self.gen_reg[15].rrx = val
    }

    // access to 32 bit instruction pointer
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn rip(&self) -> &u64 {
        unsafe { &self.gen_reg[BX_64BIT_REG_RIP].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_rip(&mut self, val: u64) {
        self.gen_reg[BX_64BIT_REG_RIP].rrx = val
    }

    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn ssp(&self) -> &u64 {
        unsafe { &self.gen_reg[BX_64BIT_REG_SSP].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_ssp(&mut self, val: u64) {
        self.gen_reg[BX_64BIT_REG_SSP].rrx = val
    }

    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn tmp_64(&self) -> &u64 {
        unsafe { &self.gen_reg[BX_TMP_REGISTER].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_tmp_u64(&mut self, val: u64) {
        self.gen_reg[BX_TMP_REGISTER].rrx = val
    }

    // access to 64 bit MSR registers
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn msr_fsbase(&self) -> &u64 {
        unsafe { &self.gen_reg[BxSegregs::Fs as usize].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_msr_fsbase(&mut self, val: u64) {
        self.gen_reg[BxSegregs::Fs as usize].rrx = val
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn msr_gsbase(&self) -> &u64 {
        unsafe { &self.gen_reg[BxSegregs::Gs as usize].rrx }
    }
    #[cfg(feature = "bx_support_x86_64")]
    #[inline]
    fn set_msr_gsbase(&mut self, val: u64) {
        self.gen_reg[BxSegregs::Gs as usize].rrx = val
    }

    // simplify merge between 32-bit and 64-bit mode
    #[cfg(not(feature = "bx_support_x86_64"))]
    #[inline]
    fn rax(&self) -> &u32 {
        self.eax()
    }
    #[cfg(not(feature = "bx_support_x86_64"))]
    #[inline]
    fn rcx(&self) -> &u32 {
        self.ecx()
    }
    #[cfg(not(feature = "bx_support_x86_64"))]
    #[inline]
    fn rdx(&self) -> &u32 {
        self.edx()
    }
    #[cfg(not(feature = "bx_support_x86_64"))]
    #[inline]
    fn rbx(&self) -> &u32 {
        self.ebx()
    }
    #[cfg(not(feature = "bx_support_x86_64"))]
    #[inline]
    fn rsp(&self) -> &u32 {
        self.esp()
    }
    #[cfg(not(feature = "bx_support_x86_64"))]
    #[inline]
    fn rbp(&self) -> &u32 {
        self.ebp()
    }
    #[cfg(not(feature = "bx_support_x86_64"))]
    #[inline]
    fn rsi(&self) -> &u32 {
        self.esi()
    }
    #[cfg(not(feature = "bx_support_x86_64"))]
    #[inline]
    fn rdi(&self) -> &u32 {
        self.edi()
    }
}

#[derive(Debug)]
struct AddressXlation {
    /// The address offset after resolution
    rm_addr: BxPhyAddress,
    /// physical address after translation of 1st len1 bytes of data
    paddress1: BxPhyAddress,
    /// physical address after translation of 2nd len2 bytes of data
    paddress2: BxPhyAddress,
    /// Number of bytes in page 1
    len1: u32,
    // Number of bytes in page 2
    len2: u32,
    /// Number of pages access spans (1 or 2).  Also used
    /// for the case when a native host pointer is
    /// available for the R-M-W instructions.  The host
    /// pointer is stuffed here.  Since this field has
    /// to be checked anyways (and thus cached), if it
    /// is greated than 2 (the maximum possible for
    /// normal cases) it is a native pointer and is used
    /// for a direct write access.
    pages: BxPtrEquiv,
    #[cfg(feature = "bx_support_memtype")]
    /// memory type of the page 1
    memtype1: BxMemType,
    #[cfg(feature = "bx_support_memtype")]
    /// memory type of the page 1
    memtype2: BxMemType,
}

#[derive(Debug)]
struct PdptrCache {
    pub entry: [u64; 4],
}

#[derive(Debug)]
struct FarBranch {
    pub rev_cs: u16,
    pub rev_rip: BxAddress,
}

#[derive(Debug)]
enum BxCpuActivityState {
    ActivityStateActive = 0,
    ActivityStateHlt,
    ActivityStateShutdown,
    ActivityStateWaitForSipi,
    VmxLastActivityState,
    ActivityStateMwait,
    ActivityStateMwaitIf,
}

// Hack since duplicated 3
impl From<BxCpuActivityState> for u8 {
    fn from(value: BxCpuActivityState) -> Self {
        match value {
            BxCpuActivityState::ActivityStateActive => 0,
            BxCpuActivityState::ActivityStateHlt => 1,
            BxCpuActivityState::ActivityStateShutdown => 2,
            BxCpuActivityState::ActivityStateWaitForSipi
            | BxCpuActivityState::VmxLastActivityState => 3,
            BxCpuActivityState::ActivityStateMwait => 4,
            BxCpuActivityState::ActivityStateMwaitIf => 5,
        }
    }
}

impl Default for BxCpuActivityState {
    fn default() -> Self {
        Self::VmxLastActivityState
    }
}

#[derive(Debug)]
pub struct BxRegsMsr {
    #[cfg(feature = "bx_support_apic")]
    apicbase: BxPhyAddress,

    // SYSCALL/SYSRET instruction msr's
    star: u64,

    #[cfg(feature = "bx_support_x86_64")]
    lstar: u64,
    #[cfg(feature = "bx_support_x86_64")]
    cstar: u64,
    #[cfg(feature = "bx_support_x86_64")]
    fmask: u32,
    #[cfg(feature = "bx_support_x86_64")]
    kernelgsbase: u64,
    #[cfg(feature = "bx_support_x86_64")]
    tsc_aux: u32,

    // SYSENTER/SYSEXIT instruction msr's
    #[cfg(feature = "bx_cpu_level_6")]
    sysenter_cs_msr: u32,
    #[cfg(feature = "bx_cpu_level_6")]
    sysenter_esp_msr: BxAddress,
    #[cfg(feature = "bx_cpu_level_6")]
    sysenter_eip_msr: BxAddress,

    #[cfg(feature = "bx_cpu_level_6")]
    pat: BxPackedRegister,
    #[cfg(feature = "bx_cpu_level_6")]
    mtrrphys: [u64; 16],
    #[cfg(feature = "bx_cpu_level_6")]
    mtrrfix64k: BxPackedRegister,
    #[cfg(feature = "bx_cpu_level_6")]
    mtrrfix16k: [BxPackedRegister; 2],
    #[cfg(feature = "bx_cpu_level_6")]
    mtrrfix4k: [BxPackedRegister; 8],
    #[cfg(feature = "bx_cpu_level_6")]
    mtrr_deftype: u32,

    #[cfg(any(feature = "bx_support_vmx_1", feature = "bx_support_vmx_2"))]
    ia32_feature_ctrl: u32,

    #[cfg(feature = "bx_support_svm")]
    svm_vm_cr: u32,
    #[cfg(feature = "bx_support_svm")]
    svm_hsave_pa: u64,

    #[cfg(feature = "bx_cpu_level_6")]
    ia32_xss: u64,

    #[cfg(feature = "bx_support_cet")]
    ia32_cet_control: [u64; 2], // indexed by CPL==3
    #[cfg(feature = "bx_support_cet")]
    ia32_pl_ssp: [u64; 4],
    #[cfg(feature = "bx_support_cet")]
    ia32_interrupt_ssp_table: u64,

    ia32_umwait_ctrl: u32, // SCA

                           // note from bochs source code:
                           /* TODO finish of the others */
}

#[cfg(feature = "bx_support_monitor_mwait")]
#[derive(Debug)]
pub struct MonitorAddr {
    monitor_addr: BxPhyAddress,
    armed_by: u32,
}

#[cfg(feature = "bx_support_uintr")]
#[derive(Debug)]
struct Uintr {
    ui_handler: BxAddress,
    stack_adjust: u64,
    /// user interrupt notification vector, actually 8 bit
    uinv: u32,
    /// user interrupt target table size
    uitt_size: u32,
    /// user interrupt target table address
    uitt_addr: BxAddress,
    /// user posted-interrupt descriptor address
    upid_addr: BxAddress,
    /// user-interrupt request register
    uirr: u64,
    /// if UIF=0 user interrupt cannot be delivered
    uif: bool,
}

#[cfg(feature = "bx_support_uintr")]
impl Uintr {
    fn senduipi_enabled(&self) -> bool {
        (self.uitt_addr & 0x1) != 0
    }
}

#[cfg(feature = "bx_debugger")]
#[derive(Debug)]
struct BxDbgGuardState {
    /// cs:eip and linear addr of instruction at guard point
    cs: u32,
    eip: BxAddress,
    laddr: BxAddress,
    // 00 - 16 bit, 01 - 32 bit, 10 - 64-bit, 11 - illegal
    code_32_64: u32, // CS seg size at guard point
}

#[cfg(feature = "bx_debugger")]
#[derive(Debug)]
struct BxGuardFound {
    guard_found: u32,
    icount_max: u64, // stop after completing this many instructions
    iaddr_index: u32,
    guard_state: BxDbgGuardState,
}
