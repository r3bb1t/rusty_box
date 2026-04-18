use crate::config::{BxAddress, BxPhyAddress};

use super::crregs::{BxCr0, BxCr4, BxEfer};
use super::descriptor::{BxGlobalSegmentReg, BxSegmentReg};
use super::i387::BxPackedRegister;

// =====================
//  SVM revision
// =====================

pub const BX_SVM_REVISION: u32 = 0x01;

// =====================
//  SVM intercept codes
// =====================

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SvmVmexit {
    Cr0Read = 0x0,
    Cr2Read = 0x2,
    Cr3Read = 0x3,
    Cr4Read = 0x4,
    Cr8Read = 0x8,
    Cr0Write = 0x10,
    Cr2Write = 0x12,
    Cr3Write = 0x13,
    Cr4Write = 0x14,
    Cr8Write = 0x18,
    Dr0Read = 0x20,
    Dr0Write = 0x30,
    Exception = 0x40,
    // PfException = 0x40 + BX_PF_EXCEPTION (14) = 0x4e
    PfException = 0x4e,
    Intr = 0x60,
    Nmi = 0x61,
    Smi = 0x62,
    Init = 0x63,
    Vintr = 0x64,
    Cr0SelWrite = 0x65,
    IdtrRead = 0x66,
    GdtrRead = 0x67,
    LdtrRead = 0x68,
    TrRead = 0x69,
    IdtrWrite = 0x6a,
    GdtrWrite = 0x6b,
    LdtrWrite = 0x6c,
    TrWrite = 0x6d,
    Rdtsc = 0x6e,
    Rdpmc = 0x6f,
    Pushf = 0x70,
    Popf = 0x71,
    Cpuid = 0x72,
    Rsm = 0x73,
    Iret = 0x74,
    SoftwareInterrupt = 0x75,
    Invd = 0x76,
    Pause = 0x77,
    Hlt = 0x78,
    Invlpg = 0x79,
    Invlpga = 0x7a,
    Io = 0x7b,
    Msr = 0x7c,
    TaskSwitch = 0x7d,
    FerrFreeze = 0x7e,
    Shutdown = 0x7f,
    Vmrun = 0x80,
    Vmmcall = 0x81,
    Vmload = 0x82,
    Vmsave = 0x83,
    Stgi = 0x84,
    Clgi = 0x85,
    Skinit = 0x86,
    Rdtscp = 0x87,
    Icebp = 0x88,
    Wbinvd = 0x89,
    Monitor = 0x8a,
    Mwait = 0x8b,
    MwaitConditional = 0x8c,
    Xsetbv = 0x8d,
    Rdpru = 0x8e,
    EferWriteTrap = 0x8f,
    Cr0WriteTrap = 0x90,
    Cr3WriteTrap = 0x93,
    Cr4WriteTrap = 0x94,
    Invlpgb = 0xa0,
    InvlpgbIllegal = 0xa1,
    Invpcid = 0xa2,
    Mcommit = 0xa3,
    Tlbsync = 0xa4,
    Buslock = 0xa5,
    IdleHlt = 0xa6,
    Npf = 0x400,
    AvicIncompleteIpi = 0x401,
    AvicNoaccel = 0x402,
    Vmgexit = 0x403,
}

pub const SVM_VMEXIT_INVALID: i32 = -1;

// =====================
//  VMCB control fields
// =====================

pub const SVM_CONTROL16_INTERCEPT_CR_READ: u32 = 0x000;
pub const SVM_CONTROL16_INTERCEPT_CR_WRITE: u32 = 0x002;
pub const SVM_CONTROL16_INTERCEPT_DR_READ: u32 = 0x004;
pub const SVM_CONTROL16_INTERCEPT_DR_WRITE: u32 = 0x006;
pub const SVM_CONTROL32_INTERCEPT_EXCEPTIONS: u32 = 0x008;
pub const SVM_CONTROL32_INTERCEPT1: u32 = 0x00c;
pub const SVM_CONTROL32_INTERCEPT2: u32 = 0x010;
pub const SVM_CONTROL32_INTERCEPT3: u32 = 0x014;

pub const SVM_CONTROL16_PAUSE_FILTER_THRESHOLD: u32 = 0x03c;
pub const SVM_CONTROL16_PAUSE_FILTER_COUNT: u32 = 0x03e;
pub const SVM_CONTROL64_IOPM_BASE_PHY_ADDR: u32 = 0x040;
pub const SVM_CONTROL64_MSRPM_BASE_PHY_ADDR: u32 = 0x048;
pub const SVM_CONTROL64_TSC_OFFSET: u32 = 0x050;
pub const SVM_CONTROL32_GUEST_ASID: u32 = 0x058;
pub const SVM_CONTROL32_TLB_CONTROL: u32 = 0x05c;
pub const SVM_CONTROL_VTPR: u32 = 0x060;
pub const SVM_CONTROL_VIRQ: u32 = 0x061;
pub const SVM_CONTROL_VINTR_PRIO_IGN_TPR: u32 = 0x062;
pub const SVM_CONTROL_VINTR_MASKING: u32 = 0x063;
pub const SVM_CONTROL_VINTR_VECTOR: u32 = 0x064;
pub const SVM_CONTROL_INTERRUPT_SHADOW: u32 = 0x068;
pub const SVM_CONTROL64_EXITCODE: u32 = 0x070;
pub const SVM_CONTROL64_EXITINFO1: u32 = 0x078;
pub const SVM_CONTROL64_EXITINFO2: u32 = 0x080;
pub const SVM_CONTROL32_EXITINTINFO: u32 = 0x088;
pub const SVM_CONTROL32_EXITINTINFO_ERROR_CODE: u32 = 0x08c;
pub const SVM_CONTROL_NESTED_PAGING_ENABLE: u32 = 0x090;

pub const SVM_VIRTUAL_APIC_BAR: u32 = 0x098;

pub const SVM_CONTROL32_EVENT_INJECTION: u32 = 0x0a8;
pub const SVM_CONTROL32_EVENT_INJECTION_ERRORCODE: u32 = 0x0ac;
pub const SVM_CONTROL64_NESTED_PAGING_HOST_CR3: u32 = 0x0b0;
pub const SVM_CONTROL_LBR_VIRTUALIZATION_ENABLE: u32 = 0x0b8;
pub const SVM_CONTROL32_VMCB_CLEAN_BITS: u32 = 0x0c0;
pub const SVM_CONTROL64_NRIP: u32 = 0x0c8;

pub const SVM_CONTROL64_GUEST_INSTR_BYTES: u32 = 0x0d0;
pub const SVM_CONTROL64_GUEST_INSTR_BYTES_HI: u32 = 0x0d8;

pub const SVM_AVIC_BACKING_PAGE: u32 = 0x0e0;
pub const SVM_AVIC_LOGICAL_TABLE_PTR: u32 = 0x0f0;
pub const SVM_AVIC_PHYSICAL_TABLE_PTR: u32 = 0x0f8;

// ======================
//  VMCB save state area
// ======================

pub const SVM_GUEST_ES_SELECTOR: u32 = 0x400;
pub const SVM_GUEST_ES_ATTR: u32 = 0x402;
pub const SVM_GUEST_ES_LIMIT: u32 = 0x404;
pub const SVM_GUEST_ES_BASE: u32 = 0x408;

pub const SVM_GUEST_CS_SELECTOR: u32 = 0x410;
pub const SVM_GUEST_CS_ATTR: u32 = 0x412;
pub const SVM_GUEST_CS_LIMIT: u32 = 0x414;
pub const SVM_GUEST_CS_BASE: u32 = 0x418;

pub const SVM_GUEST_SS_SELECTOR: u32 = 0x420;
pub const SVM_GUEST_SS_ATTR: u32 = 0x422;
pub const SVM_GUEST_SS_LIMIT: u32 = 0x424;
pub const SVM_GUEST_SS_BASE: u32 = 0x428;

pub const SVM_GUEST_DS_SELECTOR: u32 = 0x430;
pub const SVM_GUEST_DS_ATTR: u32 = 0x432;
pub const SVM_GUEST_DS_LIMIT: u32 = 0x434;
pub const SVM_GUEST_DS_BASE: u32 = 0x438;

pub const SVM_GUEST_FS_SELECTOR: u32 = 0x440;
pub const SVM_GUEST_FS_ATTR: u32 = 0x442;
pub const SVM_GUEST_FS_LIMIT: u32 = 0x444;
pub const SVM_GUEST_FS_BASE: u32 = 0x448;

pub const SVM_GUEST_GS_SELECTOR: u32 = 0x450;
pub const SVM_GUEST_GS_ATTR: u32 = 0x452;
pub const SVM_GUEST_GS_LIMIT: u32 = 0x454;
pub const SVM_GUEST_GS_BASE: u32 = 0x458;

pub const SVM_GUEST_GDTR_LIMIT: u32 = 0x464;
pub const SVM_GUEST_GDTR_BASE: u32 = 0x468;

pub const SVM_GUEST_LDTR_SELECTOR: u32 = 0x470;
pub const SVM_GUEST_LDTR_ATTR: u32 = 0x472;
pub const SVM_GUEST_LDTR_LIMIT: u32 = 0x474;
pub const SVM_GUEST_LDTR_BASE: u32 = 0x478;

pub const SVM_GUEST_IDTR_LIMIT: u32 = 0x484;
pub const SVM_GUEST_IDTR_BASE: u32 = 0x488;

pub const SVM_GUEST_TR_SELECTOR: u32 = 0x490;
pub const SVM_GUEST_TR_ATTR: u32 = 0x492;
pub const SVM_GUEST_TR_LIMIT: u32 = 0x494;
pub const SVM_GUEST_TR_BASE: u32 = 0x498;

pub const SVM_GUEST_PL0_SSP: u32 = 0x4a0;
pub const SVM_GUEST_PL1_SSP: u32 = 0x4a8;
pub const SVM_GUEST_PL2_SSP: u32 = 0x4b0;
pub const SVM_GUEST_PL3_SSP: u32 = 0x4b8;
pub const SVM_GUEST_U_CET: u32 = 0x4c0;

pub const SVM_GUEST_CPL: u32 = 0x4cb;
pub const SVM_GUEST_EFER_MSR: u32 = 0x4d0;
pub const SVM_GUEST_EFER_MSR_HI: u32 = 0x4d4;

pub const SVM_GUEST_XSS: u32 = 0x540;
pub const SVM_GUEST_CR4: u32 = 0x548;
pub const SVM_GUEST_CR4_HI: u32 = 0x54c;
pub const SVM_GUEST_CR3: u32 = 0x550;
pub const SVM_GUEST_CR0: u32 = 0x558;
pub const SVM_GUEST_CR0_HI: u32 = 0x55c;
pub const SVM_GUEST_DR7: u32 = 0x560;
pub const SVM_GUEST_DR7_HI: u32 = 0x564;
pub const SVM_GUEST_DR6: u32 = 0x568;
pub const SVM_GUEST_DR6_HI: u32 = 0x56c;
pub const SVM_GUEST_RFLAGS: u32 = 0x570;
pub const SVM_GUEST_RFLAGS_HI: u32 = 0x574;
pub const SVM_GUEST_RIP: u32 = 0x578;
pub const SVM_GUEST_RSP: u32 = 0x5d8;
pub const SVM_GUEST_S_CET: u32 = 0x5e0;
pub const SVM_GUEST_SSP: u32 = 0x5e8;
pub const SVM_GUEST_ISST_ADDR: u32 = 0x5f0;
pub const SVM_GUEST_RAX: u32 = 0x5f8;
pub const SVM_GUEST_STAR_MSR: u32 = 0x600;
pub const SVM_GUEST_LSTAR_MSR: u32 = 0x608;
pub const SVM_GUEST_CSTAR_MSR: u32 = 0x610;
pub const SVM_GUEST_FMASK_MSR: u32 = 0x618;
pub const SVM_GUEST_KERNEL_GSBASE_MSR: u32 = 0x620;
pub const SVM_GUEST_SYSENTER_CS_MSR: u32 = 0x628;
pub const SVM_GUEST_SYSENTER_ESP_MSR: u32 = 0x630;
pub const SVM_GUEST_SYSENTER_EIP_MSR: u32 = 0x638;
pub const SVM_GUEST_CR2: u32 = 0x640;

pub const SVM_GUEST_PAT: u32 = 0x668;
pub const SVM_GUEST_DBGCTL_MSR: u32 = 0x670;
pub const SVM_GUEST_BR_FROM_MSR: u32 = 0x678;
pub const SVM_GUEST_BR_TO_MSR: u32 = 0x680;
pub const SVM_GUEST_LAST_EXCEPTION_FROM_MSR: u32 = 0x688;
pub const SVM_GUEST_LAST_EXCEPTION_TO_MSR: u32 = 0x690;

pub const SVM_GUEST_SPEC_CTRL: u32 = 0x6e0;
pub const SVM_GUEST_PKRU: u32 = 0x6e8;
pub const SVM_GUEST_TSC_AUX: u32 = 0x6ec;
pub const SVM_GUEST_TSC_SCALE: u32 = 0x6f0;
pub const SVM_GUEST_TSC_OFFSET: u32 = 0x6f8;

pub const SVM_GUEST_RCX: u32 = 0x708;
pub const SVM_GUEST_RDX: u32 = 0x710;
pub const SVM_GUEST_RBX: u32 = 0x718;
pub const SVM_GUEST_SECURE_AVIC_CTL: u32 = 0x720;
pub const SVM_GUEST_RBP: u32 = 0x728;
pub const SVM_GUEST_RSI: u32 = 0x730;
pub const SVM_GUEST_RDI: u32 = 0x738;
pub const SVM_GUEST_R8: u32 = 0x740;
pub const SVM_GUEST_R9: u32 = 0x748;
pub const SVM_GUEST_R10: u32 = 0x750;
pub const SVM_GUEST_R11: u32 = 0x758;
pub const SVM_GUEST_R12: u32 = 0x760;
pub const SVM_GUEST_R13: u32 = 0x768;
pub const SVM_GUEST_R14: u32 = 0x770;
pub const SVM_GUEST_R15: u32 = 0x778;

// ========================
//  SVM intercept controls
// ========================

// vector0[15:00]: intercept reads of CR0-CR15
// vector0[31:16]: intercept writes of CR0-CR15
// vector1[15:00]: intercept reads of DR0-DR15
// vector1[31:16]: intercept writes of DR0-DR15
// vector2[31:00]: intercept exception vectors 0-31
// vector3[31:00]:

#[allow(non_camel_case_types)]
pub const SVM_INTERCEPT0_INTR: u32 = 0;
pub const SVM_INTERCEPT0_NMI: u32 = 1;
pub const SVM_INTERCEPT0_SMI: u32 = 2;
pub const SVM_INTERCEPT0_INIT: u32 = 3;
pub const SVM_INTERCEPT0_VINTR: u32 = 4;
pub const SVM_INTERCEPT0_CR0_WRITE_NO_TS_MP: u32 = 5;
pub const SVM_INTERCEPT0_IDTR_READ: u32 = 6;
pub const SVM_INTERCEPT0_GDTR_READ: u32 = 7;
pub const SVM_INTERCEPT0_LDTR_READ: u32 = 8;
pub const SVM_INTERCEPT0_TR_READ: u32 = 9;
pub const SVM_INTERCEPT0_IDTR_WRITE: u32 = 10;
pub const SVM_INTERCEPT0_GDTR_WRITE: u32 = 11;
pub const SVM_INTERCEPT0_LDTR_WRITE: u32 = 12;
pub const SVM_INTERCEPT0_TR_WRITE: u32 = 13;
pub const SVM_INTERCEPT0_RDTSC: u32 = 14;
pub const SVM_INTERCEPT0_RDPMC: u32 = 15;
pub const SVM_INTERCEPT0_PUSHF: u32 = 16;
pub const SVM_INTERCEPT0_POPF: u32 = 17;
pub const SVM_INTERCEPT0_CPUID: u32 = 18;
pub const SVM_INTERCEPT0_RSM: u32 = 19;
pub const SVM_INTERCEPT0_IRET: u32 = 20;
pub const SVM_INTERCEPT0_SOFTINT: u32 = 21;
pub const SVM_INTERCEPT0_INVD: u32 = 22;
pub const SVM_INTERCEPT0_PAUSE: u32 = 23;
pub const SVM_INTERCEPT0_HLT: u32 = 24;
pub const SVM_INTERCEPT0_INVLPG: u32 = 25;
pub const SVM_INTERCEPT0_INVLPGA: u32 = 26;
pub const SVM_INTERCEPT0_IO: u32 = 27;
pub const SVM_INTERCEPT0_MSR: u32 = 28;
pub const SVM_INTERCEPT0_TASK_SWITCH: u32 = 29;
pub const SVM_INTERCEPT0_FERR_FREEZE: u32 = 30;
pub const SVM_INTERCEPT0_SHUTDOWN: u32 = 31;

// vector4[16:00]:
// vector4[31:16]: Intercept writes of CR0-CR15 (trap)
pub const SVM_INTERCEPT1_VMRUN: u32 = 32;
pub const SVM_INTERCEPT1_VMMCALL: u32 = 33;
pub const SVM_INTERCEPT1_VMLOAD: u32 = 34;
pub const SVM_INTERCEPT1_VMSAVE: u32 = 35;
pub const SVM_INTERCEPT1_STGI: u32 = 36;
pub const SVM_INTERCEPT1_CLGI: u32 = 37;
pub const SVM_INTERCEPT1_SKINIT: u32 = 38;
pub const SVM_INTERCEPT1_RDTSCP: u32 = 39;
pub const SVM_INTERCEPT1_ICEBP: u32 = 40;
pub const SVM_INTERCEPT1_WBINVD: u32 = 41;
pub const SVM_INTERCEPT1_MONITOR: u32 = 42;
pub const SVM_INTERCEPT1_MWAIT: u32 = 43;
pub const SVM_INTERCEPT1_MWAIT_ARMED: u32 = 44;
pub const SVM_INTERCEPT1_XSETBV: u32 = 45;
pub const SVM_INTERCEPT1_RDPRU: u32 = 46;
pub const SVM_INTERCEPT1_EFER_WRITE_TRAP: u32 = 47;
pub const SVM_INTERCEPT1_CR0_WRITE_TRAP: u32 = 48;

// vector5[31:00]:
pub const SVM_INTERCEPT2_INVLPGB: u32 = 64;
pub const SVM_INTERCEPT2_INVLPGB_ILLEGAL: u32 = 65;
pub const SVM_INTERCEPT2_INVPCID: u32 = 66;
pub const SVM_INTERCEPT2_MCOMMIT: u32 = 67;
pub const SVM_INTERCEPT2_TLBSYNC: u32 = 68;

// ========================
//  SVM data structures
// ========================

#[derive(Default, Clone)]
pub struct SvmHostState {
    pub sregs: [BxSegmentReg; 4],
    pub gdtr: BxGlobalSegmentReg,
    pub idtr: BxGlobalSegmentReg,
    pub efer: BxEfer,
    pub cr0: BxCr0,
    pub cr4: BxCr4,
    pub cr3: BxPhyAddress,
    pub eflags: u32,
    pub rip: u64,
    pub rsp: u64,
    pub rax: u64,
    pub pat_msr: BxPackedRegister,
}

#[derive(Default, Clone)]
pub struct SvmGuestState {
    pub sregs: [BxSegmentReg; 4],
    pub gdtr: BxGlobalSegmentReg,
    pub idtr: BxGlobalSegmentReg,
    pub efer: BxEfer,
    pub cr0: BxCr0,
    pub cr4: BxCr4,
    pub cr2: BxAddress,
    pub dr6: u32,
    pub dr7: u32,
    pub cr3: BxPhyAddress,
    pub pat_msr: BxPackedRegister,
    pub eflags: u32,
    pub rip: u64,
    pub rsp: u64,
    pub rax: u64,
    pub cpl: u32,
    pub inhibit_interrupts: bool,
}

#[derive(Debug, Default, Clone)]
pub struct SvmControls {
    pub cr_rd_ctrl: u16,
    pub cr_wr_ctrl: u16,
    pub dr_rd_ctrl: u16,
    pub dr_wr_ctrl: u16,
    pub exceptions_intercept: u32,

    pub intercept_vector: [u32; 3],

    pub exitintinfo: u32,
    pub exitintinfo_error_code: u32,

    pub eventinj: u32,

    pub iopm_base: BxPhyAddress,
    pub msrpm_base: BxPhyAddress,

    pub v_tpr: u8,
    pub v_intr_prio: u8,
    pub v_ignore_tpr: bool,
    pub v_intr_masking: bool,
    pub v_intr_vector: u8,

    pub nested_paging: bool,
    pub ncr3: u64,

    pub pause_filter_count: u16,
    pub pause_filter_threshold: u16,
    pub last_pause_time: u64,
}

#[derive(Default, Clone)]
pub struct VmcbCache {
    pub host_state: SvmHostState,
    pub ctrls: SvmControls,
}

/// Check if a specific SVM intercept bit is set.
/// intercept_bitnum values are SVM_INTERCEPT0_*, SVM_INTERCEPT1_*, SVM_INTERCEPT2_*.
#[inline]
pub fn svm_intercept(ctrls: &SvmControls, intercept_bitnum: u32) -> bool {
    let vector_idx = (intercept_bitnum / 32) as usize;
    let bit = intercept_bitnum & 31;
    (ctrls.intercept_vector[vector_idx] & (1 << bit)) != 0
}

/// Check if an exception vector is intercepted.
#[inline]
pub fn svm_exception_intercepted(ctrls: &SvmControls, vector: u32) -> bool {
    (ctrls.exceptions_intercept & (1 << vector)) != 0
}
