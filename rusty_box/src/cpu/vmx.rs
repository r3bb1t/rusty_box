// Intel VT-x (VMX) — Bochs cpu/vmx.cc.

#![allow(dead_code, non_camel_case_types)]

use super::cpu::Exception;
use super::decoder::{BxSegregs, Instruction};
use super::instrumentation::Instrumentation;
use super::{BxCpuC, BxCpuIdTrait, Result};

// Bochs vmx.h BX_IA32_FEATURE_CONTROL_* bits.
pub const BX_IA32_FEATURE_CONTROL_LOCK_BIT: u32 = 0x1;
pub const BX_IA32_FEATURE_CONTROL_VMX_ENABLE_BIT: u32 = 0x4;
pub const BX_IA32_FEATURE_CONTROL_BITS: u32 =
    BX_IA32_FEATURE_CONTROL_LOCK_BIT | BX_IA32_FEATURE_CONTROL_VMX_ENABLE_BIT;

/// The VMCS revision ID this implementation advertises via IA32_VMX_BASIC.
/// Bochs uses 1 — kernels treat any value the host returns as authoritative.
pub const BX_VMCS_REVISION_ID: u32 = 1;

/// Fixed offset within the 4 KiB VMCS region where we store the launch-state
/// flag. Bochs picks an implementation-specific offset via its `vmcs_map`;
/// since our table is not ported yet, pin launch-state to bytes 4..8 (right
/// after the revision ID dword at offset 0). This is invisible to guests —
/// they only touch this byte via VMCLEAR / VMLAUNCH / VMRESUME semantics.
pub const VMCS_LAUNCH_STATE_OFFSET: u64 = 4;

pub const VMCS_STATE_CLEAR: u32 = 0;
pub const VMCS_STATE_LAUNCHED: u32 = 1;

// ──────────────────────────────────────────────────────────────────────────
// VMCS field encodings — Bochs cpu/vmx.h. Grouped by width and role.
// ──────────────────────────────────────────────────────────────────────────

// 16-bit guest selectors (Bochs vmx.h VMCS_16BIT_GUEST_*_SELECTOR).
const VMCS_16BIT_GUEST_ES_SELECTOR: u32 = 0x0800;
const VMCS_16BIT_GUEST_CS_SELECTOR: u32 = 0x0802;
const VMCS_16BIT_GUEST_SS_SELECTOR: u32 = 0x0804;
const VMCS_16BIT_GUEST_DS_SELECTOR: u32 = 0x0806;
const VMCS_16BIT_GUEST_FS_SELECTOR: u32 = 0x0808;
const VMCS_16BIT_GUEST_GS_SELECTOR: u32 = 0x080A;
const VMCS_16BIT_GUEST_LDTR_SELECTOR: u32 = 0x080C;
const VMCS_16BIT_GUEST_TR_SELECTOR: u32 = 0x080E;

// 16-bit host selectors (Bochs vmx.h VMCS_16BIT_HOST_*_SELECTOR).
const VMCS_16BIT_HOST_ES_SELECTOR: u32 = 0x0C00;
const VMCS_16BIT_HOST_CS_SELECTOR: u32 = 0x0C02;
const VMCS_16BIT_HOST_SS_SELECTOR: u32 = 0x0C04;
const VMCS_16BIT_HOST_DS_SELECTOR: u32 = 0x0C06;
const VMCS_16BIT_HOST_FS_SELECTOR: u32 = 0x0C08;
const VMCS_16BIT_HOST_GS_SELECTOR: u32 = 0x0C0A;
const VMCS_16BIT_HOST_TR_SELECTOR: u32 = 0x0C0C;

// 64-bit control / guest / host fields.
const VMCS_64BIT_CONTROL_TSC_OFFSET: u32 = 0x2010;
const VMCS_64BIT_GUEST_LINK_POINTER: u32 = 0x2800;
const VMCS_64BIT_GUEST_IA32_PAT: u32 = 0x2804;
const VMCS_64BIT_GUEST_IA32_EFER: u32 = 0x2806;
const VMCS_64BIT_HOST_IA32_PAT: u32 = 0x2C00;
const VMCS_64BIT_HOST_IA32_EFER: u32 = 0x2C02;

// 32-bit control fields.
const VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS: u32 = 0x4000;
const VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS: u32 = 0x4002;
const VMCS_32BIT_CONTROL_EXECUTION_BITMAP: u32 = 0x4004;
const VMCS_32BIT_CONTROL_VMEXIT_CONTROLS: u32 = 0x400C;
const VMCS_32BIT_CONTROL_SECONDARY_VMEXEC_CONTROLS: u32 = 0x401E;
const VMCS_32BIT_CONTROL_VMENTRY_CONTROLS: u32 = 0x4012;
const VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO: u32 = 0x4016;
const VMCS_32BIT_CONTROL_VMENTRY_EXCEPTION_ERR_CODE: u32 = 0x4018;
const VMCS_32BIT_CONTROL_VMENTRY_INSTRUCTION_LENGTH: u32 = 0x401A;

// 32-bit read-only exit data.
const VMCS_32BIT_INSTRUCTION_ERROR: u32 = 0x4400;
const VMCS_32BIT_VMEXIT_REASON: u32 = 0x4402;
const VMCS_32BIT_VMEXIT_INTERRUPTION_INFO: u32 = 0x4404;
const VMCS_32BIT_VMEXIT_INTERRUPTION_ERR_CODE: u32 = 0x4406;
const VMCS_32BIT_IDT_VECTORING_INFO: u32 = 0x4408;
const VMCS_32BIT_IDT_VECTORING_ERR_CODE: u32 = 0x440A;
const VMCS_32BIT_VMEXIT_INSTRUCTION_LENGTH: u32 = 0x440C;
const VMCS_32BIT_VMEXIT_INSTRUCTION_INFO: u32 = 0x440E;

// 32-bit guest state.
const VMCS_32BIT_GUEST_ES_LIMIT: u32 = 0x4800;
const VMCS_32BIT_GUEST_CS_LIMIT: u32 = 0x4802;
const VMCS_32BIT_GUEST_SS_LIMIT: u32 = 0x4804;
const VMCS_32BIT_GUEST_DS_LIMIT: u32 = 0x4806;
const VMCS_32BIT_GUEST_FS_LIMIT: u32 = 0x4808;
const VMCS_32BIT_GUEST_GS_LIMIT: u32 = 0x480A;
const VMCS_32BIT_GUEST_LDTR_LIMIT: u32 = 0x480C;
const VMCS_32BIT_GUEST_TR_LIMIT: u32 = 0x480E;
const VMCS_32BIT_GUEST_GDTR_LIMIT: u32 = 0x4810;
const VMCS_32BIT_GUEST_IDTR_LIMIT: u32 = 0x4812;
const VMCS_32BIT_GUEST_ES_ACCESS_RIGHTS: u32 = 0x4814;
const VMCS_32BIT_GUEST_CS_ACCESS_RIGHTS: u32 = 0x4816;
const VMCS_32BIT_GUEST_SS_ACCESS_RIGHTS: u32 = 0x4818;
const VMCS_32BIT_GUEST_DS_ACCESS_RIGHTS: u32 = 0x481A;
const VMCS_32BIT_GUEST_FS_ACCESS_RIGHTS: u32 = 0x481C;
const VMCS_32BIT_GUEST_GS_ACCESS_RIGHTS: u32 = 0x481E;
const VMCS_32BIT_GUEST_LDTR_ACCESS_RIGHTS: u32 = 0x4820;
const VMCS_32BIT_GUEST_TR_ACCESS_RIGHTS: u32 = 0x4822;
const VMCS_32BIT_GUEST_INTERRUPTIBILITY_STATE: u32 = 0x4824;
const VMCS_32BIT_GUEST_ACTIVITY_STATE: u32 = 0x4826;
const VMCS_32BIT_GUEST_IA32_SYSENTER_CS_MSR: u32 = 0x482A;

// 32-bit host state.
const VMCS_32BIT_HOST_IA32_SYSENTER_CS_MSR: u32 = 0x4C00;

// Natural-width control fields.
const VMCS_CONTROL_CR0_GUEST_HOST_MASK: u32 = 0x6000;
const VMCS_CONTROL_CR4_GUEST_HOST_MASK: u32 = 0x6002;
const VMCS_CONTROL_CR0_READ_SHADOW: u32 = 0x6004;
const VMCS_CONTROL_CR4_READ_SHADOW: u32 = 0x6006;

// Natural-width read-only exit data.
const VMCS_VMEXIT_QUALIFICATION: u32 = 0x6400;
const VMCS_VMEXIT_GUEST_LINEAR_ADDR: u32 = 0x640A;

// Natural-width guest state.
const VMCS_GUEST_CR0: u32 = 0x6800;
const VMCS_GUEST_CR3: u32 = 0x6802;
const VMCS_GUEST_CR4: u32 = 0x6804;
const VMCS_GUEST_ES_BASE: u32 = 0x6806;
const VMCS_GUEST_CS_BASE: u32 = 0x6808;
const VMCS_GUEST_SS_BASE: u32 = 0x680A;
const VMCS_GUEST_DS_BASE: u32 = 0x680C;
const VMCS_GUEST_FS_BASE: u32 = 0x680E;
const VMCS_GUEST_GS_BASE: u32 = 0x6810;
const VMCS_GUEST_LDTR_BASE: u32 = 0x6812;
const VMCS_GUEST_TR_BASE: u32 = 0x6814;
const VMCS_GUEST_GDTR_BASE: u32 = 0x6816;
const VMCS_GUEST_IDTR_BASE: u32 = 0x6818;
const VMCS_GUEST_DR7: u32 = 0x681A;
const VMCS_GUEST_RSP: u32 = 0x681C;
const VMCS_GUEST_RIP: u32 = 0x681E;
const VMCS_GUEST_RFLAGS: u32 = 0x6820;
const VMCS_GUEST_IA32_SYSENTER_ESP_MSR: u32 = 0x6824;
const VMCS_GUEST_IA32_SYSENTER_EIP_MSR: u32 = 0x6826;

// Natural-width host state.
const VMCS_HOST_CR0: u32 = 0x6C00;
const VMCS_HOST_CR3: u32 = 0x6C02;
const VMCS_HOST_CR4: u32 = 0x6C04;
const VMCS_HOST_FS_BASE: u32 = 0x6C06;
const VMCS_HOST_GS_BASE: u32 = 0x6C08;
const VMCS_HOST_TR_BASE: u32 = 0x6C0A;
const VMCS_HOST_GDTR_BASE: u32 = 0x6C0C;
const VMCS_HOST_IDTR_BASE: u32 = 0x6C0E;
const VMCS_HOST_IA32_SYSENTER_ESP_MSR: u32 = 0x6C10;
const VMCS_HOST_IA32_SYSENTER_EIP_MSR: u32 = 0x6C12;
const VMCS_HOST_RSP: u32 = 0x6C14;
const VMCS_HOST_RIP: u32 = 0x6C16;

/// Bochs vmx.h `enum VMX_vmexit_reason` — every reason the host reads from
/// VMCS_EXIT_REASON after a VM-exit. Session 5 port; the individual exit
/// paths that set each reason land incrementally in Sessions 5+.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmxVmexitReason {
    ExceptionNmi = 0,
    ExternalInterrupt = 1,
    TripleFault = 2,
    Init = 3,
    Sipi = 4,
    IoSmi = 5,
    Smi = 6,
    InterruptWindow = 7,
    NmiWindow = 8,
    TaskSwitch = 9,
    Cpuid = 10,
    Getsec = 11,
    Hlt = 12,
    Invd = 13,
    Invlpg = 14,
    Rdpmc = 15,
    Rdtsc = 16,
    Rsm = 17,
    Vmcall = 18,
    Vmclear = 19,
    Vmlaunch = 20,
    Vmptrld = 21,
    Vmptrst = 22,
    Vmread = 23,
    Vmresume = 24,
    Vmwrite = 25,
    Vmxoff = 26,
    Vmxon = 27,
    CrAccess = 28,
    DrAccess = 29,
    IoInstruction = 30,
    Rdmsr = 31,
    Wrmsr = 32,
    VmentryFailureGuestState = 33,
    VmentryFailureMsr = 34,
    Reserved35 = 35,
    Mwait = 36,
    MonitorTrapFlag = 37,
    Reserved38 = 38,
    Monitor = 39,
    Pause = 40,
    VmentryFailureMca = 41,
    Reserved42 = 42,
    TprThreshold = 43,
    ApicAccess = 44,
    VirtualizedEoi = 45,
    GdtrIdtrAccess = 46,
    LdtrTrAccess = 47,
    EptViolation = 48,
    EptMisconfiguration = 49,
    Invept = 50,
    Rdtscp = 51,
    VmxPreemptionTimerExpired = 52,
    Invvpid = 53,
    Wbinvd = 54,
    Xsetbv = 55,
    ApicWrite = 56,
    Rdrand = 57,
    Invpcid = 58,
    Vmfunc = 59,
    Encls = 60,
    Rdseed = 61,
    PmlLogfull = 62,
    Xsaves = 63,
    Xrstors = 64,
    Pconfig = 65,
    Spp = 66,
    Umwait = 67,
    Tpause = 68,
    Loadiwkey = 69,
    Enclv = 70,
    Reserved71 = 71,
    EnqcmdPasid = 72,
    EnqcmdsPasid = 73,
    BusLock = 74,
    InstructionTimeout = 75,
    Seamcall = 76,
    Tdcall = 77,
    Rdmsrlist = 78,
    Wrmsrlist = 79,
    Urdmsr = 80,
    Uwrmsr = 81,
    Reserved82 = 82,
    Reserved83 = 83,
    RdmsrImm = 84,
    Wrmsrns = 85,
}

// ──────────────────────────────────────────────────────────────────────────
// VM-execution control bits — Bochs cpu/vmx_ctrls.h.
//
// The storage in `BxVmcs` is still raw u32 (proc_based_ctls, etc.), so the
// intercept checks below `& mask != 0` against the right control word. A
// later pass can replace the u32s with bitflags once the control-storage
// refactor lands; the bit numbers stay stable either way.
// ──────────────────────────────────────────────────────────────────────────

// Pin-based VM-execution controls.
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_EXTERNAL_INTERRUPT_VMEXIT: u32 = 1 << 0;
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_NMI_EXITING: u32 = 1 << 3;
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_VIRTUAL_NMI: u32 = 1 << 5;
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_VMX_PREEMPTION_TIMER_VMEXIT: u32 = 1 << 6;

// Primary processor-based VM-execution controls (Bochs VmxVmexec1Controls).
pub(super) const VMX_VM_EXEC_CTRL1_INTERRUPT_WINDOW_VMEXIT: u32 = 1 << 2;
pub(super) const VMX_VM_EXEC_CTRL1_HLT_VMEXIT: u32 = 1 << 7;
pub(super) const VMX_VM_EXEC_CTRL1_INVLPG_VMEXIT: u32 = 1 << 9;
pub(super) const VMX_VM_EXEC_CTRL1_MWAIT_VMEXIT: u32 = 1 << 10;
pub(super) const VMX_VM_EXEC_CTRL1_RDPMC_VMEXIT: u32 = 1 << 11;
pub(super) const VMX_VM_EXEC_CTRL1_RDTSC_VMEXIT: u32 = 1 << 12;
pub(super) const VMX_VM_EXEC_CTRL1_CR3_WRITE_VMEXIT: u32 = 1 << 15;
pub(super) const VMX_VM_EXEC_CTRL1_CR3_READ_VMEXIT: u32 = 1 << 16;
pub(super) const VMX_VM_EXEC_CTRL1_CR8_WRITE_VMEXIT: u32 = 1 << 19;
pub(super) const VMX_VM_EXEC_CTRL1_CR8_READ_VMEXIT: u32 = 1 << 20;
pub(super) const VMX_VM_EXEC_CTRL1_NMI_WINDOW_EXITING: u32 = 1 << 22;
pub(super) const VMX_VM_EXEC_CTRL1_DRX_ACCESS_VMEXIT: u32 = 1 << 23;
pub(super) const VMX_VM_EXEC_CTRL1_IO_VMEXIT: u32 = 1 << 24;
pub(super) const VMX_VM_EXEC_CTRL1_IO_BITMAPS: u32 = 1 << 25;
pub(super) const VMX_VM_EXEC_CTRL1_MONITOR_TRAP_FLAG: u32 = 1 << 27;
pub(super) const VMX_VM_EXEC_CTRL1_MSR_BITMAPS: u32 = 1 << 28;
pub(super) const VMX_VM_EXEC_CTRL1_MONITOR_VMEXIT: u32 = 1 << 29;
pub(super) const VMX_VM_EXEC_CTRL1_PAUSE_VMEXIT: u32 = 1 << 30;
pub(super) const VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS: u32 = 1 << 31;

// Secondary processor-based VM-execution controls (Bochs VmxVmexec2Controls).
pub(super) const VMX_VM_EXEC_CTRL2_DESCRIPTOR_TABLE_VMEXIT: u32 = 1 << 2;
pub(super) const VMX_VM_EXEC_CTRL2_RDTSCP: u32 = 1 << 3;
pub(super) const VMX_VM_EXEC_CTRL2_WBINVD_VMEXIT: u32 = 1 << 6;
pub(super) const VMX_VM_EXEC_CTRL2_PAUSE_LOOP_VMEXIT: u32 = 1 << 10;
pub(super) const VMX_VM_EXEC_CTRL2_RDRAND_VMEXIT: u32 = 1 << 11;
pub(super) const VMX_VM_EXEC_CTRL2_INVPCID: u32 = 1 << 12;
pub(super) const VMX_VM_EXEC_CTRL2_RDSEED_VMEXIT: u32 = 1 << 16;
pub(super) const VMX_VM_EXEC_CTRL2_XSAVES_XRSTORS: u32 = 1 << 20;
pub(super) const VMX_VM_EXEC_CTRL2_UMWAIT_TPAUSE_VMEXIT: u32 = 1 << 26;

/// VMX-instruction error codes written into the VMCS 32-bit
/// VMCS_32BIT_INSTRUCTION_ERROR field by `VMfail`.
/// Mirrors Bochs vmx.h `enum VMX_error_code`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmxErr {
    NoError = 0,
    VmcallInVmxRootOperation = 1,
    VmclearWithInvalidAddr = 2,
    VmclearWithVmxonVmcsPtr = 3,
    VmlaunchNonClearVmcs = 4,
    VmresumeNonLaunchedVmcs = 5,
    VmresumeVmcsCorrupted = 6,
    VmentryInvalidVmControlField = 7,
    VmentryInvalidVmHostStateField = 8,
    VmptrldInvalidPhysicalAddress = 9,
    VmptrldWithVmxonPtr = 10,
    VmptrldIncorrectVmcsRevisionId = 11,
    UnsupportedVmcsComponentAccess = 12,
    VmwriteReadOnlyVmcsComponent = 13,
    VmxonInVmxRootOperation = 15,
    VmentryInvalidExecutiveVmcs = 16,
    VmentryNonLaunchedExecutiveVmcs = 17,
    VmentryNotVmxonExecutiveVmcs = 18,
    VmcallNonClearVmcs = 19,
    VmcallInvalidVmexitField = 20,
    VmcallInvalidMsegRevisionId = 22,
    VmxoffWithConfiguredSmmMonitor = 23,
    VmcallWithInvalidSmmMonitorFeatures = 24,
    VmentryInvalidVmControlFieldInExecutiveVmcs = 25,
    VmentryMovSsBlocking = 26,
    InvalidInveptInvvpid = 28,
}

// Legacy VMCS wrapper kept for the (still-stubbed) VMCS memory pointer path.
// Extended incrementally in Sessions 4+ as VMCS fields / caching are added.
pub type VmcsCache = BxVmcs;

#[derive(Debug, Default)]
pub struct VmcsMapping {}

use super::vmx_ctrls::{VmxPinBasedVmexecControls, VmxVmexec1Controls, VmxVmexec2Controls};

/// In-memory VMCS cache mirroring Bochs cpu/vmx.h `VMCS_CACHE`. Holds the
/// host and guest state that VMLAUNCH / VMRESUME / VMEXIT swap between, plus
/// the read-only fields updated by the VMEXIT machinery. Fields are laid
/// out as Rust structures so VMREAD / VMWRITE can dispatch by encoding
/// without a raw byte-offset table.
#[derive(Debug, Default)]
pub struct BxVmcs {
    // Launch state — Bochs sets this to VMCS_STATE_LAUNCHED after the first
    // successful VMLAUNCH; VMCLEAR resets it.
    pub launched: bool,

    // ---- Host state (saved on successful VMENTRY, restored on VMEXIT) ----
    pub host_cr0: u64,
    pub host_cr3: u64,
    pub host_cr4: u64,
    pub host_rsp: u64,
    pub host_rip: u64,
    pub host_cs_selector: u16,
    pub host_ss_selector: u16,
    pub host_ds_selector: u16,
    pub host_es_selector: u16,
    pub host_fs_selector: u16,
    pub host_gs_selector: u16,
    pub host_tr_selector: u16,
    pub host_fs_base: u64,
    pub host_gs_base: u64,
    pub host_tr_base: u64,
    pub host_gdtr_base: u64,
    pub host_idtr_base: u64,
    pub host_ia32_efer: u64,
    pub host_ia32_pat: u64,
    pub host_sysenter_cs: u32,
    pub host_sysenter_esp: u64,
    pub host_sysenter_eip: u64,

    // ---- Guest state (loaded on VMENTRY, saved on VMEXIT) ----
    pub guest_cr0: u64,
    pub guest_cr3: u64,
    pub guest_cr4: u64,
    pub guest_rsp: u64,
    pub guest_rip: u64,
    pub guest_rflags: u64,
    pub guest_dr7: u64,
    pub guest_ia32_efer: u64,
    pub guest_ia32_pat: u64,
    pub guest_ia32_sysenter_cs: u32,
    pub guest_ia32_sysenter_esp: u64,
    pub guest_ia32_sysenter_eip: u64,
    pub guest_cs_selector: u16,
    pub guest_ss_selector: u16,
    pub guest_ds_selector: u16,
    pub guest_es_selector: u16,
    pub guest_fs_selector: u16,
    pub guest_gs_selector: u16,
    pub guest_ldtr_selector: u16,
    pub guest_tr_selector: u16,
    pub guest_cs_base: u64,
    pub guest_ss_base: u64,
    pub guest_ds_base: u64,
    pub guest_es_base: u64,
    pub guest_fs_base: u64,
    pub guest_gs_base: u64,
    pub guest_ldtr_base: u64,
    pub guest_tr_base: u64,
    pub guest_gdtr_base: u64,
    pub guest_idtr_base: u64,
    pub guest_cs_limit: u32,
    pub guest_ss_limit: u32,
    pub guest_ds_limit: u32,
    pub guest_es_limit: u32,
    pub guest_fs_limit: u32,
    pub guest_gs_limit: u32,
    pub guest_ldtr_limit: u32,
    pub guest_tr_limit: u32,
    pub guest_gdtr_limit: u32,
    pub guest_idtr_limit: u32,
    pub guest_cs_ar: u32,
    pub guest_ss_ar: u32,
    pub guest_ds_ar: u32,
    pub guest_es_ar: u32,
    pub guest_fs_ar: u32,
    pub guest_gs_ar: u32,
    pub guest_ldtr_ar: u32,
    pub guest_tr_ar: u32,
    pub guest_activity_state: u32,
    pub guest_interruptibility_state: u32,

    // ---- Exit info (written on VMEXIT) ----
    pub vm_instruction_error: u32,
    pub exit_reason: u32,
    pub exit_qualification: u64,
    pub exit_intr_info: u32,
    pub exit_intr_error_code: u32,
    pub exit_instruction_length: u32,
    pub exit_instruction_info: u32,
    pub idt_vectoring_info: u32,
    pub idt_vectoring_error_code: u32,
    pub guest_linear_addr: u64,

    // ---- Control fields (written by host via VMWRITE before VMLAUNCH) ----
    pub pin_based_ctls: u32,
    pub proc_based_ctls: u32,
    pub secondary_proc_based_ctls: u32,
    pub vm_exit_ctls: u32,
    pub vm_entry_ctls: u32,
    pub vm_entry_intr_info: u32,
    pub vm_entry_exception_error_code: u32,
    pub vm_entry_instruction_length: u32,
    pub exception_bitmap: u32,
    pub cr0_guest_host_mask: u64,
    pub cr4_guest_host_mask: u64,
    pub cr0_read_shadow: u64,
    pub cr4_read_shadow: u64,
    pub vmcs_link_pointer: u64,
    pub tsc_offset: u64,

    // Wire-compat bag kept from earlier scaffolding; some older call sites
    // still reach for these. They stay zero until a VMM populates them.
    pin_vmexec_ctrls: VmxPinBasedVmexecControls,
    vmexec_ctrls1: VmxVmexec1Controls,
    vmexec_ctrls2: VmxVmexec2Controls,
    pub(crate) shadow_stack_prematurely_busy: bool,
}

pub type BxVmxCap = VmxCap;

#[derive(Debug, Default)]
pub struct VmxCap {}

impl<I: BxCpuIdTrait, T: Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // VMX flag-based result helpers — Bochs cpu.h VMsucceed / VMfailInvalid
    // and vmx.cc BX_CPU_C::VMfail.
    // =========================================================================

    /// Bochs cpu.h VMsucceed — clear OSZAPC.
    pub(super) fn vmsucceed(&mut self) {
        self.oszapc.set_oszapc_logic_32(1);
    }

    /// Bochs cpu.h VMfailInvalid — clear OSZAPC then assert CF.
    pub(super) fn vmfail_invalid(&mut self) {
        self.oszapc.set_oszapc_logic_32(1);
        self.oszapc.set_cf(true);
    }

    /// Bochs vmx.cc BX_CPU_C::VMfail — writes the error code into the current
    /// VMCS (if any) and asserts ZF; otherwise asserts CF.
    pub(super) fn vmfail(&mut self, error: VmxErr) {
        self.oszapc.set_oszapc_logic_32(1);
        if self.vmcsptr != super::vmcs::BX_INVALID_VMCSPTR {
            self.oszapc.set_zf(true);
            // Bochs VMwrite32(VMCS_32BIT_INSTRUCTION_ERROR, error).
            self.vmcs.vm_instruction_error = error as u32;
        } else {
            self.oszapc.set_cf(true);
        }
    }

    // =========================================================================
    // VMXON — enter VMX operation mode (opcode F3 0F C7 /6 m64)
    // Bochs vmx.cc BX_CPU_C::VMXON.
    // =========================================================================

    pub(super) fn vmxon(&mut self, instr: &Instruction) -> Result<()> {
        // Bochs vmx.cc: UD if CR4.VMXE clear, not in protected mode, or in
        // long-compat mode.
        if !self.cr4.vmxe() || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }

        if !self.in_vmx {
            // Entering VMX root from outside VMX.
            let cpl = self.cs_rpl();
            if cpl != 0
                || !self.cr0.ne()
                || !self.cr0.pe()
                || !self.a20_enabled()
                || (self.msr.ia32_feature_ctrl & BX_IA32_FEATURE_CONTROL_LOCK_BIT) == 0
                || (self.msr.ia32_feature_ctrl & BX_IA32_FEATURE_CONTROL_VMX_ENABLE_BIT) == 0
            {
                tracing::trace!("VMXON: preconditions not met → #GP(0)");
                return self.exception(Exception::Gp, 0);
            }

            // Operand is a 64-bit physical address of the VMXON region.
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let paddr = if self.long64_mode() {
                self.read_virtual_qword_64(seg, eaddr)?
            } else {
                self.read_virtual_qword(seg, eaddr as u32)?
            };

            // Must be 4 KiB-aligned and within the physical-address width
            // Bochs advertises (BX_PHY_ADDRESS_WIDTH = 40 bits in our config).
            const BX_PHY_ADDRESS_WIDTH: u32 = 40;
            if paddr == 0
                || (paddr & 0xFFF) != 0
                || (paddr >> BX_PHY_ADDRESS_WIDTH) != 0
            {
                tracing::trace!("VMXON: invalid or misaligned paddr {:#x}", paddr);
                self.vmfail_invalid();
                return Ok(());
            }

            // Check revision ID at paddr matches the emulator's.
            let rev = self.vmx_read_revision_id(paddr);
            if rev != BX_VMCS_REVISION_ID {
                tracing::trace!(
                    "VMXON: VMCS revision mismatch at {:#x}: have {:#x} want {:#x}",
                    paddr, rev, BX_VMCS_REVISION_ID
                );
                self.vmfail_invalid();
                return Ok(());
            }

            self.vmcsptr = super::vmcs::BX_INVALID_VMCSPTR;
            self.vmxonptr = paddr;
            self.in_vmx = true;
            self.mask_event(Self::BX_EVENT_INIT);
            self.monitor.reset_monitor();
            self.vmsucceed();
            return Ok(());
        }

        // Already in VMX non-root → VMEXIT (deferred until Session 4 wires the
        // VMX exit path). For now, surface as #GP so guests observe a failure.
        if self.in_vmx_guest {
            tracing::trace!("VMXON: in VMX guest — VMEXIT reason VMXON (stub #GP)");
            return self.exception(Exception::Gp, 0);
        }

        // Already in VMX root operation.
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }
        self.vmfail(VmxErr::VmxonInVmxRootOperation);
        Ok(())
    }

    // =========================================================================
    // VMXOFF — leave VMX operation mode (opcode 0F 01 C4)
    // Bochs vmx.cc BX_CPU_C::VMXOFF.
    // =========================================================================

    pub(super) fn vmxoff(&mut self, _instr: &Instruction) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }

        if self.in_vmx_guest {
            // Bochs VMexit(VMX_VMEXIT_VMXOFF, 0) — full VM-exit path ships in
            // Session 5. For Session 3, collapse to #GP so guest-mode VMXOFF
            // doesn't silently succeed.
            return self.exception(Exception::Gp, 0);
        }

        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        self.vmxonptr = super::vmcs::BX_INVALID_VMCSPTR;
        self.in_vmx = false;
        self.unmask_event(Self::BX_EVENT_INIT);
        self.monitor.reset_monitor();
        self.vmsucceed();
        Ok(())
    }

    // =========================================================================
    // VMCLEAR — initialise a VMCS in memory, mark launch-state clear.
    // Bochs vmx.cc BX_CPU_C::VMCLEAR.
    // =========================================================================

    pub(super) fn vmclear(&mut self, instr: &Instruction) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            // VMEXIT path lands in Session 5 — for now surface as #GP so guest
            // VMCLEAR doesn't silently succeed.
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let paddr = if self.long64_mode() {
            self.read_virtual_qword_64(seg, eaddr)?
        } else {
            self.read_virtual_qword(seg, eaddr as u32)?
        };

        const BX_PHY_ADDRESS_WIDTH: u32 = 40;
        if paddr == 0
            || (paddr & 0xFFF) != 0
            || (paddr >> BX_PHY_ADDRESS_WIDTH) != 0
        {
            self.vmfail(VmxErr::VmclearWithInvalidAddr);
            return Ok(());
        }

        if paddr == self.vmxonptr {
            self.vmfail(VmxErr::VmclearWithVmxonVmcsPtr);
            return Ok(());
        }

        // Clear the VMCS launch-state flag in guest-physical memory.
        self.mem_write_dword(paddr + VMCS_LAUNCH_STATE_OFFSET, VMCS_STATE_CLEAR);

        // If we were using this VMCS as the current one, drop it.
        if paddr == self.vmcsptr {
            self.vmcsptr = super::vmcs::BX_INVALID_VMCSPTR;
        }

        self.vmsucceed();
        Ok(())
    }

    // =========================================================================
    // VMPTRLD — load VMCS pointer from memory operand.
    // Bochs vmx.cc BX_CPU_C::VMPTRLD.
    // =========================================================================

    pub(super) fn vmptrld(&mut self, instr: &Instruction) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let paddr = if self.long64_mode() {
            self.read_virtual_qword_64(seg, eaddr)?
        } else {
            self.read_virtual_qword(seg, eaddr as u32)?
        };

        const BX_PHY_ADDRESS_WIDTH: u32 = 40;
        if paddr == 0
            || (paddr & 0xFFF) != 0
            || (paddr >> BX_PHY_ADDRESS_WIDTH) != 0
        {
            self.vmfail(VmxErr::VmptrldInvalidPhysicalAddress);
            return Ok(());
        }

        if paddr == self.vmxonptr {
            self.vmfail(VmxErr::VmptrldWithVmxonPtr);
            return Ok(());
        }

        let revision = self.vmx_read_revision_id(paddr);
        if revision != BX_VMCS_REVISION_ID {
            tracing::trace!(
                "VMPTRLD: revision mismatch at {:#x}: {:#x} vs {:#x}",
                paddr, revision, BX_VMCS_REVISION_ID
            );
            self.vmfail(VmxErr::VmptrldIncorrectVmcsRevisionId);
            return Ok(());
        }

        self.vmcsptr = paddr;
        self.vmsucceed();
        Ok(())
    }

    // =========================================================================
    // VMPTRST — store current VMCS pointer to memory operand.
    // Bochs vmx.cc BX_CPU_C::VMPTRST.
    // =========================================================================

    pub(super) fn vmptrst(&mut self, instr: &Instruction) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.vmcsptr;
        if self.long64_mode() {
            self.write_virtual_qword_64(seg, eaddr, val)?;
        } else {
            self.write_virtual_qword(seg, eaddr as u32, val)?;
        }
        self.vmsucceed();
        Ok(())
    }

    // =========================================================================
    // VMREAD / VMWRITE — VMCS field access, dispatched by encoding through
    // the named fields on `BxVmcs`. Matches Bochs' VMread16/32/64/natural +
    // VMwrite*. Field encodings mirror Bochs cpu/vmx.h. Unhandled encodings
    // raise VMXERR_UNSUPPORTED_VMCS_COMPONENT_ACCESS so nothing corrupts
    // silently.
    // =========================================================================

    pub(super) fn vmread_impl(&mut self, encoding: u32) -> Result<u64> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            self.exception(Exception::Ud, 0)?;
            unreachable!();
        }
        if self.in_vmx_guest {
            self.exception(Exception::Gp, 0)?;
            unreachable!();
        }
        if self.cs_rpl() != 0 {
            self.exception(Exception::Gp, 0)?;
            unreachable!();
        }
        if self.vmcsptr == super::vmcs::BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(0);
        }

        let v = self.vmcs_read_field(encoding);
        match v {
            Some(val) => {
                self.vmsucceed();
                Ok(val)
            }
            None => {
                self.vmfail(VmxErr::UnsupportedVmcsComponentAccess);
                Ok(0)
            }
        }
    }

    pub(super) fn vmwrite_impl(&mut self, encoding: u32, value: u64) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }
        if self.vmcsptr == super::vmcs::BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(());
        }

        if self.vmcs_write_field(encoding, value) {
            self.vmsucceed();
            Ok(())
        } else {
            self.vmfail(VmxErr::UnsupportedVmcsComponentAccess);
            Ok(())
        }
    }

    /// Dispatch a VMCS field encoding → named field in `self.vmcs`.
    /// Returns `None` if the encoding is not recognised.
    fn vmcs_read_field(&self, encoding: u32) -> Option<u64> {
        let v = &self.vmcs;
        Some(match encoding {
            // 16-bit guest selectors.
            VMCS_16BIT_GUEST_ES_SELECTOR => v.guest_es_selector as u64,
            VMCS_16BIT_GUEST_CS_SELECTOR => v.guest_cs_selector as u64,
            VMCS_16BIT_GUEST_SS_SELECTOR => v.guest_ss_selector as u64,
            VMCS_16BIT_GUEST_DS_SELECTOR => v.guest_ds_selector as u64,
            VMCS_16BIT_GUEST_FS_SELECTOR => v.guest_fs_selector as u64,
            VMCS_16BIT_GUEST_GS_SELECTOR => v.guest_gs_selector as u64,
            VMCS_16BIT_GUEST_LDTR_SELECTOR => v.guest_ldtr_selector as u64,
            VMCS_16BIT_GUEST_TR_SELECTOR => v.guest_tr_selector as u64,
            // 16-bit host selectors.
            VMCS_16BIT_HOST_ES_SELECTOR => v.host_es_selector as u64,
            VMCS_16BIT_HOST_CS_SELECTOR => v.host_cs_selector as u64,
            VMCS_16BIT_HOST_SS_SELECTOR => v.host_ss_selector as u64,
            VMCS_16BIT_HOST_DS_SELECTOR => v.host_ds_selector as u64,
            VMCS_16BIT_HOST_FS_SELECTOR => v.host_fs_selector as u64,
            VMCS_16BIT_HOST_GS_SELECTOR => v.host_gs_selector as u64,
            VMCS_16BIT_HOST_TR_SELECTOR => v.host_tr_selector as u64,
            // 64-bit control / guest / host.
            VMCS_64BIT_GUEST_LINK_POINTER => v.vmcs_link_pointer,
            VMCS_64BIT_CONTROL_TSC_OFFSET => v.tsc_offset,
            VMCS_64BIT_GUEST_IA32_EFER => v.guest_ia32_efer,
            VMCS_64BIT_GUEST_IA32_PAT => v.guest_ia32_pat,
            VMCS_64BIT_HOST_IA32_PAT => v.host_ia32_pat,
            VMCS_64BIT_HOST_IA32_EFER => v.host_ia32_efer,
            // 32-bit control fields.
            VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS => v.pin_based_ctls as u64,
            VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS => v.proc_based_ctls as u64,
            VMCS_32BIT_CONTROL_EXECUTION_BITMAP => v.exception_bitmap as u64,
            VMCS_32BIT_CONTROL_SECONDARY_VMEXEC_CONTROLS => v.secondary_proc_based_ctls as u64,
            VMCS_32BIT_CONTROL_VMEXIT_CONTROLS => v.vm_exit_ctls as u64,
            VMCS_32BIT_CONTROL_VMENTRY_CONTROLS => v.vm_entry_ctls as u64,
            VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO => v.vm_entry_intr_info as u64,
            VMCS_32BIT_CONTROL_VMENTRY_EXCEPTION_ERR_CODE => v.vm_entry_exception_error_code as u64,
            VMCS_32BIT_CONTROL_VMENTRY_INSTRUCTION_LENGTH => v.vm_entry_instruction_length as u64,
            // 32-bit read-only exit data.
            VMCS_32BIT_INSTRUCTION_ERROR => v.vm_instruction_error as u64,
            VMCS_32BIT_VMEXIT_REASON => v.exit_reason as u64,
            VMCS_32BIT_VMEXIT_INTERRUPTION_INFO => v.exit_intr_info as u64,
            VMCS_32BIT_VMEXIT_INTERRUPTION_ERR_CODE => v.exit_intr_error_code as u64,
            VMCS_32BIT_IDT_VECTORING_INFO => v.idt_vectoring_info as u64,
            VMCS_32BIT_IDT_VECTORING_ERR_CODE => v.idt_vectoring_error_code as u64,
            VMCS_32BIT_VMEXIT_INSTRUCTION_LENGTH => v.exit_instruction_length as u64,
            VMCS_32BIT_VMEXIT_INSTRUCTION_INFO => v.exit_instruction_info as u64,
            // 32-bit guest state.
            VMCS_32BIT_GUEST_ES_LIMIT => v.guest_es_limit as u64,
            VMCS_32BIT_GUEST_CS_LIMIT => v.guest_cs_limit as u64,
            VMCS_32BIT_GUEST_SS_LIMIT => v.guest_ss_limit as u64,
            VMCS_32BIT_GUEST_DS_LIMIT => v.guest_ds_limit as u64,
            VMCS_32BIT_GUEST_FS_LIMIT => v.guest_fs_limit as u64,
            VMCS_32BIT_GUEST_GS_LIMIT => v.guest_gs_limit as u64,
            VMCS_32BIT_GUEST_LDTR_LIMIT => v.guest_ldtr_limit as u64,
            VMCS_32BIT_GUEST_TR_LIMIT => v.guest_tr_limit as u64,
            VMCS_32BIT_GUEST_GDTR_LIMIT => v.guest_gdtr_limit as u64,
            VMCS_32BIT_GUEST_IDTR_LIMIT => v.guest_idtr_limit as u64,
            VMCS_32BIT_GUEST_ES_ACCESS_RIGHTS => v.guest_es_ar as u64,
            VMCS_32BIT_GUEST_CS_ACCESS_RIGHTS => v.guest_cs_ar as u64,
            VMCS_32BIT_GUEST_SS_ACCESS_RIGHTS => v.guest_ss_ar as u64,
            VMCS_32BIT_GUEST_DS_ACCESS_RIGHTS => v.guest_ds_ar as u64,
            VMCS_32BIT_GUEST_FS_ACCESS_RIGHTS => v.guest_fs_ar as u64,
            VMCS_32BIT_GUEST_GS_ACCESS_RIGHTS => v.guest_gs_ar as u64,
            VMCS_32BIT_GUEST_LDTR_ACCESS_RIGHTS => v.guest_ldtr_ar as u64,
            VMCS_32BIT_GUEST_TR_ACCESS_RIGHTS => v.guest_tr_ar as u64,
            VMCS_32BIT_GUEST_INTERRUPTIBILITY_STATE => v.guest_interruptibility_state as u64,
            VMCS_32BIT_GUEST_ACTIVITY_STATE => v.guest_activity_state as u64,
            VMCS_32BIT_GUEST_IA32_SYSENTER_CS_MSR => v.guest_ia32_sysenter_cs as u64,
            // 32-bit host state.
            VMCS_32BIT_HOST_IA32_SYSENTER_CS_MSR => v.host_sysenter_cs as u64,
            // Natural-width control.
            VMCS_CONTROL_CR0_GUEST_HOST_MASK => v.cr0_guest_host_mask,
            VMCS_CONTROL_CR4_GUEST_HOST_MASK => v.cr4_guest_host_mask,
            VMCS_CONTROL_CR0_READ_SHADOW => v.cr0_read_shadow,
            VMCS_CONTROL_CR4_READ_SHADOW => v.cr4_read_shadow,
            // Natural-width read-only.
            VMCS_VMEXIT_QUALIFICATION => v.exit_qualification,
            VMCS_VMEXIT_GUEST_LINEAR_ADDR => v.guest_linear_addr,
            // Natural-width guest state.
            VMCS_GUEST_CR0 => v.guest_cr0,
            VMCS_GUEST_CR3 => v.guest_cr3,
            VMCS_GUEST_CR4 => v.guest_cr4,
            VMCS_GUEST_ES_BASE => v.guest_es_base,
            VMCS_GUEST_CS_BASE => v.guest_cs_base,
            VMCS_GUEST_SS_BASE => v.guest_ss_base,
            VMCS_GUEST_DS_BASE => v.guest_ds_base,
            VMCS_GUEST_FS_BASE => v.guest_fs_base,
            VMCS_GUEST_GS_BASE => v.guest_gs_base,
            VMCS_GUEST_LDTR_BASE => v.guest_ldtr_base,
            VMCS_GUEST_TR_BASE => v.guest_tr_base,
            VMCS_GUEST_GDTR_BASE => v.guest_gdtr_base,
            VMCS_GUEST_IDTR_BASE => v.guest_idtr_base,
            VMCS_GUEST_DR7 => v.guest_dr7,
            VMCS_GUEST_RSP => v.guest_rsp,
            VMCS_GUEST_RIP => v.guest_rip,
            VMCS_GUEST_RFLAGS => v.guest_rflags,
            VMCS_GUEST_IA32_SYSENTER_ESP_MSR => v.guest_ia32_sysenter_esp,
            VMCS_GUEST_IA32_SYSENTER_EIP_MSR => v.guest_ia32_sysenter_eip,
            // Natural-width host state.
            VMCS_HOST_CR0 => v.host_cr0,
            VMCS_HOST_CR3 => v.host_cr3,
            VMCS_HOST_CR4 => v.host_cr4,
            VMCS_HOST_FS_BASE => v.host_fs_base,
            VMCS_HOST_GS_BASE => v.host_gs_base,
            VMCS_HOST_TR_BASE => v.host_tr_base,
            VMCS_HOST_GDTR_BASE => v.host_gdtr_base,
            VMCS_HOST_IDTR_BASE => v.host_idtr_base,
            VMCS_HOST_IA32_SYSENTER_ESP_MSR => v.host_sysenter_esp,
            VMCS_HOST_IA32_SYSENTER_EIP_MSR => v.host_sysenter_eip,
            VMCS_HOST_RSP => v.host_rsp,
            VMCS_HOST_RIP => v.host_rip,
            _ => return None,
        })
    }

    /// Dispatch VMWRITE encoding → named field. Returns `false` if the
    /// encoding isn't supported (callers `VMfail` in that case).
    fn vmcs_write_field(&mut self, encoding: u32, value: u64) -> bool {
        let v = &mut self.vmcs;
        match encoding {
            VMCS_16BIT_GUEST_ES_SELECTOR => v.guest_es_selector = value as u16,
            VMCS_16BIT_GUEST_CS_SELECTOR => v.guest_cs_selector = value as u16,
            VMCS_16BIT_GUEST_SS_SELECTOR => v.guest_ss_selector = value as u16,
            VMCS_16BIT_GUEST_DS_SELECTOR => v.guest_ds_selector = value as u16,
            VMCS_16BIT_GUEST_FS_SELECTOR => v.guest_fs_selector = value as u16,
            VMCS_16BIT_GUEST_GS_SELECTOR => v.guest_gs_selector = value as u16,
            VMCS_16BIT_GUEST_LDTR_SELECTOR => v.guest_ldtr_selector = value as u16,
            VMCS_16BIT_GUEST_TR_SELECTOR => v.guest_tr_selector = value as u16,
            VMCS_16BIT_HOST_ES_SELECTOR => v.host_es_selector = value as u16,
            VMCS_16BIT_HOST_CS_SELECTOR => v.host_cs_selector = value as u16,
            VMCS_16BIT_HOST_SS_SELECTOR => v.host_ss_selector = value as u16,
            VMCS_16BIT_HOST_DS_SELECTOR => v.host_ds_selector = value as u16,
            VMCS_16BIT_HOST_FS_SELECTOR => v.host_fs_selector = value as u16,
            VMCS_16BIT_HOST_GS_SELECTOR => v.host_gs_selector = value as u16,
            VMCS_16BIT_HOST_TR_SELECTOR => v.host_tr_selector = value as u16,
            VMCS_64BIT_GUEST_LINK_POINTER => v.vmcs_link_pointer = value,
            VMCS_64BIT_CONTROL_TSC_OFFSET => v.tsc_offset = value,
            VMCS_64BIT_GUEST_IA32_EFER => v.guest_ia32_efer = value,
            VMCS_64BIT_GUEST_IA32_PAT => v.guest_ia32_pat = value,
            VMCS_64BIT_HOST_IA32_PAT => v.host_ia32_pat = value,
            VMCS_64BIT_HOST_IA32_EFER => v.host_ia32_efer = value,
            VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS => v.pin_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS => v.proc_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_EXECUTION_BITMAP => v.exception_bitmap = value as u32,
            VMCS_32BIT_CONTROL_SECONDARY_VMEXEC_CONTROLS => v.secondary_proc_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMEXIT_CONTROLS => v.vm_exit_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_CONTROLS => v.vm_entry_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO => v.vm_entry_intr_info = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_EXCEPTION_ERR_CODE => v.vm_entry_exception_error_code = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_INSTRUCTION_LENGTH => v.vm_entry_instruction_length = value as u32,
            // Read-only VMCS exit-data fields: Bochs VMwriteReadOnlyVmcsComponent
            // returns false. Being lenient would let VMMs that pre-zero these
            // keep going, but Bochs is strict.
            VMCS_32BIT_INSTRUCTION_ERROR
            | VMCS_32BIT_VMEXIT_REASON
            | VMCS_32BIT_VMEXIT_INTERRUPTION_INFO
            | VMCS_32BIT_VMEXIT_INTERRUPTION_ERR_CODE
            | VMCS_32BIT_IDT_VECTORING_INFO
            | VMCS_32BIT_IDT_VECTORING_ERR_CODE
            | VMCS_32BIT_VMEXIT_INSTRUCTION_LENGTH
            | VMCS_32BIT_VMEXIT_INSTRUCTION_INFO
            | VMCS_VMEXIT_QUALIFICATION
            | VMCS_VMEXIT_GUEST_LINEAR_ADDR => return false,
            VMCS_32BIT_GUEST_ES_LIMIT => v.guest_es_limit = value as u32,
            VMCS_32BIT_GUEST_CS_LIMIT => v.guest_cs_limit = value as u32,
            VMCS_32BIT_GUEST_SS_LIMIT => v.guest_ss_limit = value as u32,
            VMCS_32BIT_GUEST_DS_LIMIT => v.guest_ds_limit = value as u32,
            VMCS_32BIT_GUEST_FS_LIMIT => v.guest_fs_limit = value as u32,
            VMCS_32BIT_GUEST_GS_LIMIT => v.guest_gs_limit = value as u32,
            VMCS_32BIT_GUEST_LDTR_LIMIT => v.guest_ldtr_limit = value as u32,
            VMCS_32BIT_GUEST_TR_LIMIT => v.guest_tr_limit = value as u32,
            VMCS_32BIT_GUEST_GDTR_LIMIT => v.guest_gdtr_limit = value as u32,
            VMCS_32BIT_GUEST_IDTR_LIMIT => v.guest_idtr_limit = value as u32,
            VMCS_32BIT_GUEST_ES_ACCESS_RIGHTS => v.guest_es_ar = value as u32,
            VMCS_32BIT_GUEST_CS_ACCESS_RIGHTS => v.guest_cs_ar = value as u32,
            VMCS_32BIT_GUEST_SS_ACCESS_RIGHTS => v.guest_ss_ar = value as u32,
            VMCS_32BIT_GUEST_DS_ACCESS_RIGHTS => v.guest_ds_ar = value as u32,
            VMCS_32BIT_GUEST_FS_ACCESS_RIGHTS => v.guest_fs_ar = value as u32,
            VMCS_32BIT_GUEST_GS_ACCESS_RIGHTS => v.guest_gs_ar = value as u32,
            VMCS_32BIT_GUEST_LDTR_ACCESS_RIGHTS => v.guest_ldtr_ar = value as u32,
            VMCS_32BIT_GUEST_TR_ACCESS_RIGHTS => v.guest_tr_ar = value as u32,
            VMCS_32BIT_GUEST_INTERRUPTIBILITY_STATE => v.guest_interruptibility_state = value as u32,
            VMCS_32BIT_GUEST_ACTIVITY_STATE => v.guest_activity_state = value as u32,
            VMCS_32BIT_GUEST_IA32_SYSENTER_CS_MSR => v.guest_ia32_sysenter_cs = value as u32,
            VMCS_32BIT_HOST_IA32_SYSENTER_CS_MSR => v.host_sysenter_cs = value as u32,
            VMCS_CONTROL_CR0_GUEST_HOST_MASK => v.cr0_guest_host_mask = value,
            VMCS_CONTROL_CR4_GUEST_HOST_MASK => v.cr4_guest_host_mask = value,
            VMCS_CONTROL_CR0_READ_SHADOW => v.cr0_read_shadow = value,
            VMCS_CONTROL_CR4_READ_SHADOW => v.cr4_read_shadow = value,
            VMCS_GUEST_CR0 => v.guest_cr0 = value,
            VMCS_GUEST_CR3 => v.guest_cr3 = value,
            VMCS_GUEST_CR4 => v.guest_cr4 = value,
            VMCS_GUEST_ES_BASE => v.guest_es_base = value,
            VMCS_GUEST_CS_BASE => v.guest_cs_base = value,
            VMCS_GUEST_SS_BASE => v.guest_ss_base = value,
            VMCS_GUEST_DS_BASE => v.guest_ds_base = value,
            VMCS_GUEST_FS_BASE => v.guest_fs_base = value,
            VMCS_GUEST_GS_BASE => v.guest_gs_base = value,
            VMCS_GUEST_LDTR_BASE => v.guest_ldtr_base = value,
            VMCS_GUEST_TR_BASE => v.guest_tr_base = value,
            VMCS_GUEST_GDTR_BASE => v.guest_gdtr_base = value,
            VMCS_GUEST_IDTR_BASE => v.guest_idtr_base = value,
            VMCS_GUEST_DR7 => v.guest_dr7 = value,
            VMCS_GUEST_RSP => v.guest_rsp = value,
            VMCS_GUEST_RIP => v.guest_rip = value,
            VMCS_GUEST_RFLAGS => v.guest_rflags = value,
            VMCS_GUEST_IA32_SYSENTER_ESP_MSR => v.guest_ia32_sysenter_esp = value,
            VMCS_GUEST_IA32_SYSENTER_EIP_MSR => v.guest_ia32_sysenter_eip = value,
            VMCS_HOST_CR0 => v.host_cr0 = value,
            VMCS_HOST_CR3 => v.host_cr3 = value,
            VMCS_HOST_CR4 => v.host_cr4 = value,
            VMCS_HOST_FS_BASE => v.host_fs_base = value,
            VMCS_HOST_GS_BASE => v.host_gs_base = value,
            VMCS_HOST_TR_BASE => v.host_tr_base = value,
            VMCS_HOST_GDTR_BASE => v.host_gdtr_base = value,
            VMCS_HOST_IDTR_BASE => v.host_idtr_base = value,
            VMCS_HOST_IA32_SYSENTER_ESP_MSR => v.host_sysenter_esp = value,
            VMCS_HOST_IA32_SYSENTER_EIP_MSR => v.host_sysenter_eip = value,
            VMCS_HOST_RSP => v.host_rsp = value,
            VMCS_HOST_RIP => v.host_rip = value,
            _ => return false,
        }
        true
    }

    // Top-level VMREAD handlers (32-bit and 64-bit operand size).
    // Bochs vmx.cc BX_CPU_C::VMREAD_EdGd / VMREAD_EqGq.

    pub(super) fn vmread_ed_gd(&mut self, instr: &Instruction) -> Result<()> {
        let enc = self.get_gpr32(instr.src() as usize);
        let val = self.vmread_impl(enc)?;
        if instr.mod_c0() {
            self.set_gpr32(instr.dst() as usize, val as u32);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.write_virtual_dword(seg, eaddr as u32, val as u32)?;
        }
        Ok(())
    }

    pub(super) fn vmread_eq_gq(&mut self, instr: &Instruction) -> Result<()> {
        let enc = self.get_gpr64(instr.src() as usize) as u32;
        let val = self.vmread_impl(enc)?;
        if instr.mod_c0() {
            self.set_gpr64(instr.dst() as usize, val);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.write_virtual_qword_64(seg, eaddr, val)?;
        }
        Ok(())
    }

    pub(super) fn vmwrite_gd_ed(&mut self, instr: &Instruction) -> Result<()> {
        let enc = self.get_gpr32(instr.dst() as usize);
        let src = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize) as u64
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.read_virtual_dword(seg, eaddr as u32)? as u64
        };
        self.vmwrite_impl(enc, src)
    }

    pub(super) fn vmwrite_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let enc = self.get_gpr64(instr.dst() as usize) as u32;
        let src = if instr.mod_c0() {
            self.get_gpr64(instr.src() as usize)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.read_virtual_qword_64(seg, eaddr)?
        };
        self.vmwrite_impl(enc, src)
    }

    // =========================================================================
    // VMLAUNCH / VMRESUME — enter VMX non-root (guest) operation.
    //
    // Bochs cpu/vmx.cc BX_CPU_C::VMLAUNCH + VMRESUME share a handler; the
    // launch-vs-resume bit controls which error code the preconditions
    // report. Full VM-entry validation (Bochs VMenterLoadCheckVmControls /
    // HostState / GuestState) would add ~1500 LOC of field-by-field sanity;
    // in this pass we perform the architecturally-required launch-state
    // check, then do a straightforward host/guest state swap covering the
    // control registers, instruction pointer / stack pointer / RFLAGS and
    // EFER / PAT MSRs. Segment descriptor reload, interruptibility state,
    // and the ~60-field host-state-field integrity tests will grow in-place
    // as real VMMs exercise them.
    // =========================================================================

    pub(super) fn vmlaunch(&mut self, instr: &Instruction) -> Result<()> {
        self.vmlaunch_vmresume(instr, false)
    }

    pub(super) fn vmresume(&mut self, instr: &Instruction) -> Result<()> {
        self.vmlaunch_vmresume(instr, true)
    }

    fn vmlaunch_vmresume(&mut self, _instr: &Instruction, is_resume: bool) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            // Bochs: VM_exit with reason VMLAUNCH / VMRESUME (Session 6
            // intercept wiring handles the full VMEXIT path).
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }
        if self.vmcsptr == super::vmcs::BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(());
        }

        // Launch-state gate — Bochs VMXERR_VMLAUNCH_NON_CLEAR_VMCS /
        // VMXERR_VMRESUME_NON_LAUNCHED_VMCS.
        if is_resume && !self.vmcs.launched {
            self.vmfail(VmxErr::VmresumeNonLaunchedVmcs);
            return Ok(());
        }
        if !is_resume && self.vmcs.launched {
            self.vmfail(VmxErr::VmlaunchNonClearVmcs);
            return Ok(());
        }

        // Save host state from the running CPU. RIP is "the instruction after
        // VMLAUNCH / VMRESUME"; Bochs stashes it so VMEXIT_LOAD_HOST_STATE can
        // jump back. The prefetch queue already advanced past this insn, so
        // `self.rip()` points at the next one.
        self.vmcs.host_cr0 = self.cr0.get32() as u64;
        self.vmcs.host_cr3 = self.cr3;
        self.vmcs.host_cr4 = self.cr4.get() as u64;
        self.vmcs.host_rsp = self.rsp();
        self.vmcs.host_rip = self.rip();
        self.vmcs.host_ia32_efer = self.efer.get32() as u64;
        self.vmcs.host_ia32_pat = self.msr.pat.U64();

        // Load guest state into the running CPU.
        self.cr0.set32(self.vmcs.guest_cr0 as u32);
        self.cr3 = self.vmcs.guest_cr3;
        self.cr4.set_val(self.vmcs.guest_cr4);
        self.set_rsp(self.vmcs.guest_rsp);
        self.set_rip(self.vmcs.guest_rip);
        self.write_eflags(self.vmcs.guest_rflags as u32, 0x003FFFFF);
        self.efer.set32(self.vmcs.guest_ia32_efer as u32);
        self.msr.pat.set_U64(self.vmcs.guest_ia32_pat);

        self.vmcs.launched = true;
        self.in_vmx_guest = true;
        self.vmsucceed();
        // Guest now runs from the loaded RIP — the CPU loop picks up the new
        // prefetch target after this instruction returns.
        self.invalidate_prefetch_q();
        Ok(())
    }

    // =========================================================================
    // VM-exit — return to VMX root with reason + qualification.
    //
    // Bochs' VMexit() is the single entry for every exit reason. It saves
    // guest state into the VMCS, restores host state, clears in_vmx_guest,
    // and returns the CPU loop to the host instruction stream. This is the
    // symmetric counterpart to vmlaunch_vmresume above.
    // =========================================================================

    pub(super) fn vmx_vmexit(
        &mut self,
        reason: VmxVmexitReason,
        qualification: u64,
    ) -> Result<()> {
        if !self.in_vmx_guest {
            return Ok(());
        }

        // Save guest state from the running CPU.
        self.vmcs.guest_cr0 = self.cr0.get32() as u64;
        self.vmcs.guest_cr3 = self.cr3;
        self.vmcs.guest_cr4 = self.cr4.get() as u64;
        self.vmcs.guest_rsp = self.rsp();
        self.vmcs.guest_rip = self.rip();
        self.vmcs.guest_rflags = self.read_eflags() as u64;
        self.vmcs.guest_ia32_efer = self.efer.get32() as u64;
        self.vmcs.guest_ia32_pat = self.msr.pat.U64();

        // Record the exit info the host reads after re-entry.
        self.vmcs.exit_reason = reason as u32;
        self.vmcs.exit_qualification = qualification;

        // Load host state.
        self.cr0.set32(self.vmcs.host_cr0 as u32);
        self.cr3 = self.vmcs.host_cr3;
        self.cr4.set_val(self.vmcs.host_cr4);
        self.set_rsp(self.vmcs.host_rsp);
        self.set_rip(self.vmcs.host_rip);
        self.efer.set32(self.vmcs.host_ia32_efer as u32);
        self.msr.pat.set_U64(self.vmcs.host_ia32_pat);

        self.in_vmx_guest = false;
        self.invalidate_prefetch_q();
        Ok(())
    }

    // =========================================================================
    // Helpers used by VMXON.
    // =========================================================================

    /// Read the VMCS revision ID (first 4 bytes of a VMCS / VMXON region) from
    /// guest-physical memory. Bochs vmx.cc VMXReadRevisionID.
    fn vmx_read_revision_id(&mut self, paddr: u64) -> u32 {
        self.mem_read_dword(paddr)
    }

    /// Bochs cpu.h long_compat_mode — 32-bit compatibility sub-mode of long mode.
    #[inline]
    fn long_compat_mode(&self) -> bool {
        self.long_mode() && !self.long64_mode()
    }

    /// Is A20 masking enabled? Bochs' `BX_GET_ENABLE_A20()` macro pokes
    /// `bx_pc_system.enable_a20`. Our equivalent is `self.a20_mask == !0` —
    /// the mask covers the full address space when A20 is enabled.
    #[inline]
    fn a20_enabled(&self) -> bool {
        // A20 masks bit 20 to 0 when disabled → `a20_mask` lacks bit 20.
        (self.a20_mask & (1u64 << 20)) != 0
    }

    // =========================================================================
    // VM-exit intercept predicates — Bochs cpu/vmx.cc + vmexit.cc.
    //
    // Each `vmexit_check_*` method returns `Ok(true)` if the running guest
    // should trigger a VM-exit for this instruction (and performs the exit
    // as a side effect), `Ok(false)` if the guest keeps executing natively.
    //
    // The caller pattern for wrapped handlers is:
    //
    //     if self.in_vmx_guest && self.vmexit_check_hlt()? { return Ok(()); }
    //
    // Unconditional-intercept opcodes (CPUID, RSM, XSETBV, GETSEC, INVD, all
    // VMX instructions) use `vmexit_unconditional` — there's no control bit
    // to test, the exit is mandated by the SDM.
    // =========================================================================

    #[inline]
    fn proc_based_ctls1(&self) -> u32 {
        self.vmcs.proc_based_ctls
    }

    #[inline]
    fn proc_based_ctls2(&self) -> u32 {
        // The secondary controls are only honoured when the primary
        // ACTIVATE_SECONDARY_CONTROLS bit is set (Bochs gate).
        if self.vmcs.proc_based_ctls & VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS != 0 {
            self.vmcs.secondary_proc_based_ctls
        } else {
            0
        }
    }

    #[inline]
    fn pin_based_ctls(&self) -> u32 {
        self.vmcs.pin_based_ctls
    }

    /// Trigger an unconditional VM-exit (caller has already verified
    /// `self.in_vmx_guest`). Returns `Ok(true)` so call sites can chain.
    pub(super) fn vmexit_unconditional(
        &mut self,
        reason: VmxVmexitReason,
        qualification: u64,
    ) -> Result<bool> {
        self.vmx_vmexit(reason, qualification)?;
        Ok(true)
    }

    /// Conditional VM-exit: `mask` is the control bit(s) in `ctls`; if any
    /// are set, trigger `reason` with `qualification` and return Ok(true).
    pub(super) fn vmexit_if_ctls_set(
        &mut self,
        ctls: u32,
        mask: u32,
        reason: VmxVmexitReason,
        qualification: u64,
    ) -> Result<bool> {
        if ctls & mask != 0 {
            self.vmx_vmexit(reason, qualification)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn vmexit_check_hlt(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(ctls, VMX_VM_EXEC_CTRL1_HLT_VMEXIT, VmxVmexitReason::Hlt, 0)
    }

    pub(super) fn vmexit_check_invlpg(&mut self, linear_addr: u64) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_INVLPG_VMEXIT,
            VmxVmexitReason::Invlpg,
            linear_addr,
        )
    }

    pub(super) fn vmexit_check_rdtsc(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_RDTSC_VMEXIT,
            VmxVmexitReason::Rdtsc,
            0,
        )
    }

    pub(super) fn vmexit_check_rdtscp(&mut self) -> Result<bool> {
        // RDTSCP exits unconditionally unless secondary RDTSCP bit is set AND
        // the primary RDTSC_VMEXIT bit is clear. Bochs vmexit.cc VMexit_Rdtscp.
        if self.proc_based_ctls2() & VMX_VM_EXEC_CTRL2_RDTSCP == 0 {
            // RDTSCP is disabled in the guest → #UD is raised at decode time;
            // we shouldn't reach here. If we do, fall through to native.
            return Ok(false);
        }
        self.vmexit_check_rdtsc()
    }

    pub(super) fn vmexit_check_rdpmc(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_RDPMC_VMEXIT,
            VmxVmexitReason::Rdpmc,
            0,
        )
    }

    pub(super) fn vmexit_check_monitor(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_MONITOR_VMEXIT,
            VmxVmexitReason::Monitor,
            0,
        )
    }

    pub(super) fn vmexit_check_mwait(&mut self, monitor_armed: bool) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        if ctls & VMX_VM_EXEC_CTRL1_MWAIT_VMEXIT == 0 {
            return Ok(false);
        }
        // Bochs vmexit.cc: qualification[0] = monitor hardware armed.
        let qual = if monitor_armed { 1 } else { 0 };
        self.vmx_vmexit(VmxVmexitReason::Mwait, qual)?;
        Ok(true)
    }

    pub(super) fn vmexit_check_pause(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_PAUSE_VMEXIT,
            VmxVmexitReason::Pause,
            0,
        )
    }

    pub(super) fn vmexit_check_wbinvd(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls2();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL2_WBINVD_VMEXIT,
            VmxVmexitReason::Wbinvd,
            0,
        )
    }

    pub(super) fn vmexit_check_invpcid(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls2();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL2_INVPCID,
            VmxVmexitReason::Invpcid,
            0,
        )
    }

    /// RDMSR: unconditional if no MSR bitmaps, otherwise consult bitmap.
    /// Bochs vmx.cc VMexit_MSR. Until the MSR-bitmap walker lands we err on
    /// the side of "exit always when MSR_BITMAPS is clear" — i.e. exit iff
    /// the no-bitmaps policy mandates it.
    pub(super) fn vmexit_check_rdmsr(&mut self, _msr: u32) -> Result<bool> {
        // Without MSR bitmaps, every RDMSR exits unconditionally. With
        // bitmaps, the bitmap walk decides — wired later.
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_MSR_BITMAPS == 0 {
            self.vmx_vmexit(VmxVmexitReason::Rdmsr, 0)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn vmexit_check_wrmsr(&mut self, _msr: u32) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_MSR_BITMAPS == 0 {
            self.vmx_vmexit(VmxVmexitReason::Wrmsr, 0)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// I/O port intercept — Bochs vmx.cc VMexit_IO.
    /// Without I/O bitmaps, any IO_VMEXIT bit triggers exits for every port.
    /// With bitmaps, the per-port bit in io_bitmap_addr decides. Bitmap walk
    /// is deferred; current stub exits-all when IO_VMEXIT is set and bitmaps
    /// are off.
    pub(super) fn vmexit_check_io(
        &mut self,
        port: u16,
        size: u32,
        direction_in: bool,
        string: bool,
        rep: bool,
    ) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        if ctls & (VMX_VM_EXEC_CTRL1_IO_VMEXIT | VMX_VM_EXEC_CTRL1_IO_BITMAPS) == 0 {
            return Ok(false);
        }
        // Qualification bits mirror Bochs vmx.cc VMexit_IO:
        //   [2:0]  access size in bytes - 1
        //   [3]    direction (0 = OUT, 1 = IN)
        //   [4]    string instruction
        //   [5]    REP prefix
        //   [6]    operand encoding (0 = DX, 1 = immediate) — left zero here
        //   [31:16] port number
        let mut qual: u64 = 0;
        qual |= (size.saturating_sub(1) & 0x7) as u64;
        if direction_in {
            qual |= 1 << 3;
        }
        if string {
            qual |= 1 << 4;
        }
        if rep {
            qual |= 1 << 5;
        }
        qual |= (port as u64) << 16;
        self.vmx_vmexit(VmxVmexitReason::IoInstruction, qual)?;
        Ok(true)
    }
}
