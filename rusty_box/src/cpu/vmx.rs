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
/// Physical-address width used for VMX paddr-validity checks.
///
/// `BX_PHY_ADDRESS_WIDTH=40` is the universal lower bound for x86_64
/// physical address width across all CPUID models rusty_box exposes;
/// Bochs threads the per-model value through its CPU model. The eptptr-,
/// VMXON-, VMCLEAR-, and VMPTRLD-validity checks here only use the upper
/// bound to reject reserved bits, so 40 is a safe (slightly conservative)
/// constant — see Intel SDM Vol. 3C 28.2.1. If a future CPU model widens
/// MAXPHYADDR, thread a `phy_address_width()` accessor through instead of
/// editing this constant.
pub(super) const BX_PHY_ADDRESS_WIDTH: u32 = 40;


pub const VMCS_STATE_CLEAR: u32 = 0;
pub const VMCS_STATE_LAUNCHED: u32 = 1;

/// Bochs vmx.h `VMX_vmabort_code`. Written to the VMCS abort-indicator
/// field by `VMabort` before the CPU shuts down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum VmxAbortCode {
    SavingGuestMsrsFailure = 1,
    HostPdptrCorrupted = 2,
    VmexitVmcsCorrupted = 3,
    LoadingHostMsrs = 4,
    VmexitMachineCheckError = 5,
    VmexitTo32BitHostFrom64BitGuest = 6,
}

/// Reasons rusty_box surfaces a `BX_PANIC` from Bochs' VMX path as a
/// recoverable error instead of aborting the process. Each variant
/// pinpoints a host-side implementation bug — the guest could never
/// trigger one of these in correct VMM code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmxInternalReason {
    /// `vmx_vmexit` was invoked while `in_vmx_guest == false` and the
    /// reason did not have the bit-31 vmentry-failure flag set. Bochs
    /// `BX_PANIC("VMEXIT not in VMX guest mode !")` at vmx.cc VMexit.
    VmexitOutsideGuestMode,
}

// ──────────────────────────────────────────────────────────────────────────
// VMCS field encodings — Bochs cpu/vmx.h. Grouped by width and role.
// ──────────────────────────────────────────────────────────────────────────

// 16-bit guest selectors (Bochs vmx.h VMCS_16BIT_GUEST_*_SELECTOR).
const VMCS_16BIT_CONTROL_VPID: u32 = 0x0000;
// Posted-interrupt notification vector — Bochs vmx.h
// VMCS_16BIT_CONTROL_POSTED_INTERRUPT_VECTOR.
const VMCS_16BIT_CONTROL_POSTED_INTERRUPT_NOTIFICATION_VECTOR: u32 = 0x0002;
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
const VMCS_64BIT_CONTROL_IO_BITMAP_A: u32 = 0x2000;
const VMCS_64BIT_CONTROL_IO_BITMAP_B: u32 = 0x2002;
const VMCS_64BIT_CONTROL_MSR_BITMAPS: u32 = 0x2004;
// Bochs vmx.h VMCS_64BIT_CONTROL_VMREAD_BITMAP_ADDR /
// VMCS_64BIT_CONTROL_VMWRITE_BITMAP_ADDR — guest-physical
// addresses of the 4KiB VMREAD / VMWRITE bitmaps consulted when
// VMCS_SHADOWING is enabled.
const VMCS_64BIT_CONTROL_VMREAD_BITMAP_ADDR: u32 = 0x2026;
const VMCS_64BIT_CONTROL_VMWRITE_BITMAP_ADDR: u32 = 0x2028;
const VMCS_64BIT_CONTROL_VMEXIT_MSR_STORE_ADDR: u32 = 0x2006;
const VMCS_64BIT_CONTROL_VMEXIT_MSR_LOAD_ADDR: u32 = 0x2008;
const VMCS_64BIT_CONTROL_VMENTRY_MSR_LOAD_ADDR: u32 = 0x200A;
const VMCS_32BIT_CONTROL_VMEXIT_MSR_STORE_COUNT: u32 = 0x400E;
const VMCS_32BIT_CONTROL_VMEXIT_MSR_LOAD_COUNT: u32 = 0x4010;
const VMCS_32BIT_CONTROL_VMENTRY_MSR_LOAD_COUNT: u32 = 0x4014;
const VMCS_64BIT_CONTROL_TSC_OFFSET: u32 = 0x2010;
const VMCS_64BIT_CONTROL_VIRTUAL_APIC_PAGE_ADDR: u32 = 0x2012;
const VMCS_32BIT_CONTROL_TPR_THRESHOLD: u32 = 0x401C;
const VMCS_64BIT_CONTROL_EPTPTR: u32 = 0x201A;
// Posted-interrupt descriptor address — Bochs vmx.h
// VMCS_64BIT_CONTROL_POSTED_INTERRUPT_DESC_ADDR.
const VMCS_64BIT_CONTROL_POSTED_INTERRUPT_DESC_ADDR: u32 = 0x2016;
const VMCS_64BIT_GUEST_PHYSICAL_ADDR: u32 = 0x2400;

// Bochs vmx.h VMCS_FIELD_WIDTH(encoding) bits [14:13].
const VMCS_FIELD_WIDTH_16BIT: u32 = 0;
const VMCS_FIELD_WIDTH_64BIT: u32 = 1;
const VMCS_FIELD_WIDTH_32BIT: u32 = 2;
// Bits set outside the legal encoding (Bochs vmx.h
// `VMCS_ENCODING_RESERVED_BITS`): bit 12 plus bits [31:15].
const VMCS_ENCODING_RESERVED_BITS: u32 = 0xffff_9000;

// Per-(width, type) base offsets for the canonical Bochs VMCS layout
// produced by `init_generic_mapping` (cpu/vmcs.cc). The shadow VMCS
// only requires symmetric read/write across VMREAD_Shadow and
// VMWRITE_Shadow, so this layout doubles as rusty_box's authoritative
// shadow-VMCS map. Order: width index 0..3 (16/64/32/natural), each
// width carrying four field types (control / read-only / guest /
// host) of `encodings_per_width[width] * 4` bytes apiece.
const VMCS_TYPE_BASE_OFFSET: [u32; 16] = [
    0x010, 0x090, 0x110, 0x190, // 16-bit
    0x210, 0x360, 0x4B0, 0x600, // 64-bit
    0x750, 0x810, 0x8D0, 0x990, // 32-bit
    0xA50, 0xB10, 0xBD0, 0xC90, // natural-width
];
const VMCS_FIELD_LIMITS: [u32; 4] = [0x20, 0x54, 0x30, 0x30];

/// Bochs cpu/vmcs.cc `VMCS_Mapping::vmcs_field_offset` — maps a
/// raw VMCS field encoding to its byte offset inside a 4 KiB shadow
/// VMCS region. Returns `None` when the encoding's reserved bits are
/// non-zero or its field index sits past the per-width limit.
fn vmcs_field_byte_offset(encoding: u32) -> Option<u32> {
    if encoding & VMCS_ENCODING_RESERVED_BITS != 0 {
        return None;
    }
    let field = encoding & 0x3ff;
    let width = (encoding >> 13) & 0x3;
    let field_type = (encoding >> 10) & 0x3;
    if field >= VMCS_FIELD_LIMITS[width as usize] {
        return None;
    }
    let type_index = (width << 2) + field_type;
    Some(VMCS_TYPE_BASE_OFFSET[type_index as usize] + field * 4)
}

bitflags::bitflags! {
    /// Memory access classification — Bochs `bochs.h` enum
    /// `{BX_READ, BX_WRITE, BX_EXECUTE, BX_RW, BX_SHADOW_STACK_READ,
    /// BX_SHADOW_STACK_WRITE, BX_SHADOW_STACK_INVALID}`.
    ///
    /// The Bochs paging.cc bit-pattern tests map directly onto bitflags
    /// idioms. The crate's docs warn against `contains` / `intersects`
    /// against zero-bit flags, but rusty_box never tests against `Read`
    /// itself (we use `is_empty()` and direct equality `== Self::Read`
    /// when needed). The helpers below stay non-zero masks so contains/
    /// intersects behave correctly.
    ///
    /// Bit semantics:
    ///   bit 0 — write happens (`rw & 1` in Bochs)
    ///   bit 1 — non-read indicator (set for EXECUTE / RW / SS_INVALID)
    ///   bit 2 — shadow-stack access (`rw & 4` in Bochs)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct BxRwAccess: u32 {
        // Single-bit flags used by the helper predicates.
        const WRITE_BIT        = 1 << 0;
        const NON_READ_BIT     = 1 << 1;
        const SHADOW_STACK_BIT = 1 << 2;

        // Bochs enum value aliases (`bochs.h`).
        const Read               = 0;
        const Write              = Self::WRITE_BIT.bits();
        const Execute            = Self::NON_READ_BIT.bits();
        const ReadWrite          = Self::WRITE_BIT.bits() | Self::NON_READ_BIT.bits();
        const ShadowStackRead    = Self::SHADOW_STACK_BIT.bits();
        const ShadowStackWrite   = Self::WRITE_BIT.bits() | Self::SHADOW_STACK_BIT.bits();
        const ShadowStackInvalid = Self::NON_READ_BIT.bits() | Self::SHADOW_STACK_BIT.bits();
    }
}

impl BxRwAccess {
    /// Bochs `rw & 1` — the access writes (WRITE / RW / SHADOW_STACK_WRITE).
    #[inline]
    pub(super) const fn is_write(self) -> bool {
        self.contains(Self::WRITE_BIT)
    }

    /// Bochs `(rw & 3) == 0` — plain read or shadow-stack read. Tests
    /// the union mask (non-zero) via `intersects` so the zero-bit-flag
    /// caveat doesn't apply.
    #[inline]
    pub(super) fn is_read_like(self) -> bool {
        !self.intersects(Self::WRITE_BIT.union(Self::NON_READ_BIT))
    }

    /// Bochs `rw & 4` — shadow-stack access (EPT-violation qualification
    /// bit 13).
    #[inline]
    pub(super) const fn is_shadow_stack(self) -> bool {
        self.contains(Self::SHADOW_STACK_BIT)
    }
}

bitflags::bitflags! {
    /// Per-page EPT permission bits — Bochs paging.cc
    /// `BX_EPT_READ / _WRITE / _EXECUTE / _MBE_USER_EXECUTE /
    /// _MBE_SUPERVISOR_EXECUTE`. Used both in the access-mask the
    /// walker tests against and in the cumulative `combined_access`
    /// it computes from the path.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct EptPerm: u32 {
        const READ                  = 1 << 0;
        const WRITE                 = 1 << 1;
        const EXECUTE               = 1 << 2;
        /// Bit 10 — when MBE_CTRL is enabled, user-mode pages take
        /// EPT_MBE_USER_EXECUTE in place of EPT_EXECUTE.
        const MBE_USER_EXEC         = 1 << 10;
    }

    /// Single-bit flags of the I/O VMEXIT qualification — Bochs
    /// vmexit.cc VMexit_IO. Multi-bit fields stay packed:
    ///   [2:0]   access size - 1
    ///   [31:16] port number
    /// Bits 7 and 12-15 are reserved by the SDM.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct IoExitQual: u64 {
        /// Bit 3: direction is IN (1) vs OUT (0).
        const PORT_IN   = 1 << 3;
        /// Bit 4: string instruction (INS / OUTS).
        const STRING    = 1 << 4;
        /// Bit 5: REP prefix is in effect.
        const REP       = 1 << 5;
        /// Bit 6: immediate-port encoding (`IN AL,Ib`-form). When clear
        /// the port is taken from DX.
        const IMMEDIATE = 1 << 6;
    }

    /// Full EPT_VIOLATION VMEXIT qualification — Bochs paging.cc
    /// builder. All bit fields named so the builder is one bitflags
    /// value end-to-end (no mixed `flags.bits() | packed_int` assembly).
    /// The MBE_USER_EXEC bit is bit 6 here because that's where Bochs
    /// places the "user-execute" indicator under MBE_CTRL+EXECUTE,
    /// not the EPT-permission bit 10 used by EptPerm.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct EptViolationQual: u64 {
        // [2:0] — access mask: Bochs `vmexit_qualification = access_mask`.
        /// Bit 0: read access requested.
        const ACCESS_R     = 1 << 0;
        /// Bit 1: write access requested.
        const ACCESS_W     = 1 << 1;
        /// Bit 2: instruction-fetch access requested. Doubles as the
        /// "instruction fetch" indicator under MBE_CTRL+EXECUTE.
        const ACCESS_X     = 1 << 2;
        // [5:3] — combined access actually granted by the EPT path.
        const GRANTED_R    = 1 << 3;
        const GRANTED_W    = 1 << 4;
        const GRANTED_X    = 1 << 5;
        /// Bit 6: under MBE_CTRL+EXECUTE, the leaf page is user-
        /// executable (Bochs `(combined_access & BX_EPT_MBE_USER_
        /// EXECUTE)`).
        const MBE_USER_EXEC = 1 << 6;
        /// Bit 7: guest_laddr field is valid.
        const LADDR_VALID  = 1 << 7;
        /// Bit 8: this was a data/instruction access (not a page-walk
        /// paging-structure access).
        const DATA_ACCESS  = 1 << 8;
        /// Bit 9: the leaf page maps user-mode memory (MBE_CONTROL).
        const USER_PAGE    = 1 << 9;
        /// Bit 10: the leaf page is writeable in the guest paging
        /// structures (MBE_CONTROL).
        const WRITEABLE    = 1 << 10;
        /// Bit 11: the leaf page has the NX bit set (MBE_CONTROL).
        const NX_PAGE      = 1 << 11;
        /// Bit 12: the access happened on the instruction boundary
        /// where IRET unblocked NMI delivery (Bochs
        /// `nmi_unblocking_iret`).
        const NMI_UNBLOCK  = 1 << 12;
        /// Bit 13: shadow-stack access (Bochs `rw & 4`).
        const SHADOW_STACK = 1 << 13;
        /// Bit 14: leaf entry has the EPT supervisor-shadow-stack page
        /// indicator set (Bochs `BX_SUPERVISOR_SHADOW_STACK_PAGE`,
        /// leaf bit 60). Gated on `EPTPTR.bit7` (the EPT-CET-SS control)
        /// being enabled at VM-entry.
        const SSS_PAGE = 1 << 14;
    }
}
const VMCS_64BIT_CONTROL_SECONDARY_VMEXIT_CONTROLS: u32 = 0x2044;
const VMCS_64BIT_GUEST_LINK_POINTER: u32 = 0x2800;
const VMCS_64BIT_GUEST_IA32_PAT: u32 = 0x2804;
const VMCS_64BIT_GUEST_IA32_EFER: u32 = 0x2806;
const VMCS_64BIT_HOST_IA32_PAT: u32 = 0x2C00;
const VMCS_64BIT_HOST_IA32_EFER: u32 = 0x2C02;
const VMCS_64BIT_HOST_IA32_PERF_GLOBAL_CTRL: u32 = 0x2C04;
const VMCS_64BIT_HOST_IA32_PKRS: u32 = 0x2C06;
const VMCS_64BIT_HOST_IA32_FRED_CONFIG: u32 = 0x2C08;
const VMCS_64BIT_HOST_IA32_FRED_RSP1: u32 = 0x2C0A;
const VMCS_64BIT_HOST_IA32_FRED_RSP2: u32 = 0x2C0C;
const VMCS_64BIT_HOST_IA32_FRED_RSP3: u32 = 0x2C0E;
const VMCS_64BIT_HOST_IA32_FRED_STACK_LEVELS: u32 = 0x2C10;
const VMCS_64BIT_HOST_IA32_FRED_SSP1: u32 = 0x2C12;
const VMCS_64BIT_HOST_IA32_FRED_SSP2: u32 = 0x2C14;
const VMCS_64BIT_HOST_IA32_FRED_SSP3: u32 = 0x2C16;
const VMCS_64BIT_HOST_IA32_SPEC_CTRL: u32 = 0x2C1A;
const VMCS_HOST_IA32_S_CET: u32 = 0x6C18;
const VMCS_HOST_SSP: u32 = 0x6C1A;
const VMCS_HOST_INTERRUPT_SSP_TABLE_ADDR: u32 = 0x6C1C;

// Guest CET / FRED / PKRS / SPEC_CTRL / DEBUGCTL / pending-DBG / SMBASE
// VMCS field encodings — Bochs cpu/vmx.h.
const VMCS_64BIT_GUEST_IA32_DEBUGCTL: u32 = 0x2802;
const VMCS_64BIT_GUEST_IA32_PKRS: u32 = 0x2818;
const VMCS_64BIT_GUEST_IA32_FRED_CONFIG: u32 = 0x281A;
const VMCS_64BIT_GUEST_IA32_FRED_RSP1: u32 = 0x281C;
const VMCS_64BIT_GUEST_IA32_FRED_RSP2: u32 = 0x281E;
const VMCS_64BIT_GUEST_IA32_FRED_RSP3: u32 = 0x2820;
const VMCS_64BIT_GUEST_IA32_FRED_STACK_LEVELS: u32 = 0x2822;
const VMCS_64BIT_GUEST_IA32_FRED_SSP1: u32 = 0x2824;
const VMCS_64BIT_GUEST_IA32_FRED_SSP2: u32 = 0x2826;
const VMCS_64BIT_GUEST_IA32_FRED_SSP3: u32 = 0x2828;
const VMCS_64BIT_GUEST_IA32_SPEC_CTRL: u32 = 0x282E;
const VMCS_32BIT_GUEST_SMBASE: u32 = 0x4828;
const VMCS_GUEST_PENDING_DBG_EXCEPTIONS: u32 = 0x6822;
const VMCS_GUEST_IA32_S_CET: u32 = 0x6828;
const VMCS_GUEST_SSP: u32 = 0x682A;
const VMCS_GUEST_INTERRUPT_SSP_TABLE_ADDR: u32 = 0x682C;

// VMFUNC + EPTP-list controls — Bochs cpu/vmx.h.
const VMCS_64BIT_CONTROL_VMFUNC_CTRLS: u32 = 0x2018;
const VMCS_64BIT_CONTROL_EPTP_LIST_ADDRESS: u32 = 0x2024;

// 32-bit control fields.
const VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS: u32 = 0x4000;
const VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS: u32 = 0x4002;
const VMCS_32BIT_CONTROL_EXECUTION_BITMAP: u32 = 0x4004;
const VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MASK: u32 = 0x4006;
const VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MATCH: u32 = 0x4008;
const VMCS_32BIT_CONTROL_CR3_TARGET_COUNT: u32 = 0x400A;
const VMCS_CR3_TARGET0: u32 = 0x6008;
const VMCS_CR3_TARGET1: u32 = 0x600A;
const VMCS_CR3_TARGET2: u32 = 0x600C;
const VMCS_CR3_TARGET3: u32 = 0x600E;
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
const VMCS_32BIT_GUEST_PREEMPTION_TIMER_VALUE: u32 = 0x482E;

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
/// VMCS_EXIT_REASON after a VM-exit.
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

impl VmxVmexitReason {
    /// Bochs vmx.h `IS_TRAP_LIKE_VMEXIT(reason)` — VMEXIT reasons whose
    /// architectural delivery is *trap-like*: the RIP/RSP/SSP rollback
    /// step is skipped because the exit happens AFTER the offending
    /// instruction completes.
    #[inline]
    pub(super) fn is_trap_like(self) -> bool {
        matches!(
            self,
            VmxVmexitReason::TprThreshold
                | VmxVmexitReason::VirtualizedEoi
                | VmxVmexitReason::ApicWrite
                | VmxVmexitReason::BusLock
        )
    }
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
pub(super) const VMX_VM_EXEC_CTRL1_TPR_SHADOW: u32 = 1 << 21;
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_VMX_PREEMPTION_TIMER_VMEXIT: u32 = 1 << 6;
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_PROCESS_POSTED_INTERRUPTS: u32 = 1 << 7;

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
pub(super) const VMX_VM_EXEC_CTRL2_VMFUNC_ENABLE: u32 = 1 << 13;
pub(super) const VMX_VM_EXEC_CTRL2_VMCS_SHADOWING: u32 = 1 << 14;
pub(super) const VMX_VM_EXEC_CTRL2_MBE_CTRL: u32 = 1 << 22;
// Virtualised-APIC secondary controls — Bochs vapic.cc / vmx.h.
// rusty_box has not ported the APIC-access page, register virtualisation,
// or virtual-interrupt delivery datapaths; VMENTRY rejects these features
// rather than silently accepting them (see vmenter check_vm_controls).
pub(super) const VMX_VM_EXEC_CTRL2_VIRTUALIZE_APIC_ACCESSES: u32 = 1 << 0;
pub(super) const VMX_VM_EXEC_CTRL2_VIRTUALIZE_APIC_REGISTERS: u32 = 1 << 8;
pub(super) const VMX_VM_EXEC_CTRL2_VIRTUAL_INT_DELIVERY: u32 = 1 << 9;

// VM-exit control bits — Bochs vmx_ctrls.h.
pub(super) const VMX_VMEXIT_CTRL1_LOAD_PERF_GLOBAL_CTRL_MSR: u32 = 1 << 12;
pub(super) const VMX_VMEXIT_CTRL1_HOST_ADDR_SPACE_SIZE: u32 = 1 << 9;
pub(super) const VMX_VMEXIT_CTRL1_INTA_ON_VMEXIT: u32 = 1 << 15;
pub(super) const VMX_VMEXIT_CTRL1_LOAD_PAT_MSR: u32 = 1 << 19;
pub(super) const VMX_VMEXIT_CTRL1_LOAD_EFER_MSR: u32 = 1 << 21;
pub(super) const VMX_VMEXIT_CTRL1_STORE_VMX_PREEMPTION_TIMER: u32 = 1 << 22;
pub(super) const VMX_VMEXIT_CTRL1_LOAD_HOST_CET_STATE: u32 = 1 << 28;
pub(super) const VMX_VMEXIT_CTRL1_LOAD_HOST_PKRS: u32 = 1 << 29;

// VM-entry / VM-exit bits introduced for the Phase D guest-state load/save.
// Bochs vmx_ctrls.h.
pub(super) const VMX_VMEXIT_CTRL1_SAVE_DBG_CTRLS: u32 = 1 << 2;
pub(super) const VMX_VMEXIT_CTRL1_STORE_PAT_MSR: u32 = 1 << 18;
pub(super) const VMX_VMEXIT_CTRL1_STORE_EFER_MSR: u32 = 1 << 20;
pub(super) const VMX_VMEXIT_CTRL2_SAVE_GUEST_FRED: u64 = 1 << 0;

/// Bochs `IsValidPageAlignedPhyAddr` — page-aligned and within the
/// emulator's physical address width (see [`BX_PHY_ADDRESS_WIDTH`]).
fn is_valid_page_aligned_phy_addr(paddr: u64) -> bool {
    paddr & 0xFFF == 0 && (paddr >> BX_PHY_ADDRESS_WIDTH) == 0
}

/// Bochs `IsValidPhyAddr` — fits in [`BX_PHY_ADDRESS_WIDTH`] bits.
fn is_valid_phy_addr(paddr: u64) -> bool {
    (paddr >> BX_PHY_ADDRESS_WIDTH) == 0
}

/// Bochs `isValidMSR_PAT`. Each of the eight 8-bit entries must encode
/// a supported memory type: 0 (UC), 1 (WC), 4 (WT), 5 (WP), 6 (WB),
/// 7 (UC-). Other values (2, 3, ≥8) are reserved.
fn is_valid_pat_msr(value: u64) -> bool {
    for byte in 0..8 {
        let memtype = (value >> (byte * 8)) & 0xFF;
        if !matches!(memtype, 0 | 1 | 4 | 5 | 6 | 7) {
            return false;
        }
    }
    true
}

/// Bochs `isValidMSR_IA32_SPEC_CTRL`. Bits documented in Bochs:
/// IBRS (1<<0), STIBP (1<<1), SSBD (1<<2), IPRED_DIS_U/S (3..=4),
/// RRSBA_DIS_U/S (5..=6), PSFD (1<<7), DDPD_U (1<<8), BHI_DIS_S (1<<10).
/// Other bits are reserved.
fn is_valid_spec_ctrl(value: u64) -> bool {
    const ALLOWED: u64 =
        (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4)
            | (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 10);
    (value & !ALLOWED) == 0
}

bitflags::bitflags! {
    /// Decoded segment access-rights field as it lives in the VMCS —
    /// Bochs `vmx_unpack_ar_field`. Layout (32-bit packed):
    ///   [3:0]  = type (segment-type-specific, 4-bit value)
    ///   [4]    = S (segment vs system)
    ///   [6:5]  = DPL (privilege level)
    ///   [7]    = P (present)
    ///   [12]   = AVL (available)
    ///   [13]   = L (long-mode code)
    ///   [14]   = D/B (default operand size)
    ///   [15]   = G (granularity)
    ///   [16]   = unusable
    /// The `TYPE_*` and `DPL_*` constants are expressed as bit masks so
    /// callers can use `intersection().bits()` to read the packed values
    /// without leaving the bitflags namespace.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct SegAr: u32 {
        const TYPE_MASK = 0xF;
        const S_BIT     = 1 << 4;
        const DPL_MASK  = 0x3 << 5;
        const P_BIT     = 1 << 7;
        const AVL_BIT   = 1 << 12;
        const L_BIT     = 1 << 13;
        const DB_BIT    = 1 << 14;
        const G_BIT     = 1 << 15;
        const UNUSABLE  = 1 << 16;
    }
}

impl SegAr {
    /// Extract a packed sub-field defined as a bitflags mask. The shift
    /// amount is derived from the mask itself (`trailing_zeros`), so the
    /// extractor stays correct if the mask is ever repositioned and the
    /// position information lives in exactly one place — the mask const.
    #[inline]
    fn extract(self, mask: Self) -> u32 {
        self.intersection(mask).bits() >> mask.bits().trailing_zeros()
    }
    #[inline]
    pub(super) fn type_field(self) -> u32 {
        self.extract(Self::TYPE_MASK)
    }
    #[inline]
    pub(super) fn dpl(self) -> u8 {
        self.extract(Self::DPL_MASK) as u8
    }
}

/// Bochs `IsLimitAccessRightsConsistent` (cpu/segment_ctrl_pro.cc):
/// when G=1 the low 12 bits of the byte-limit must all be 1; when G=0
/// the limit must fit in 20 bits (otherwise the descriptor would
/// overrun the `limit_scaled` field in the segment cache).
fn is_limit_access_rights_consistent(limit: u32, ar: SegAr) -> bool {
    if ar.contains(SegAr::G_BIT) {
        // Page granularity: bottom 12 bits of stored limit must be 0xFFF.
        if limit & 0xFFF != 0xFFF {
            return false;
        }
    } else if limit > 0xFFFFF {
        // Byte granularity: limit must fit in 20 bits.
        return false;
    }
    true
}

// Bochs descriptor type constants used by VMENTRY guest-state checks.
// Matches the four-bit type encoding for code/data segments and the
// pair of TSS types we care about.
const BX_SEG_TYPE_DATA_RW_ACCESSED: u32 = 0x3;        // R/W data, accessed
const BX_SEG_TYPE_DATA_RW_EXP_DOWN_ACCESSED: u32 = 0x7; // R/W expand-down, accessed
const BX_SEG_TYPE_CODE_EXEC_ONLY_ACCESSED: u32 = 0x9;
const BX_SEG_TYPE_CODE_EXEC_READ_ACCESSED: u32 = 0xB;
const BX_SEG_TYPE_CODE_EXEC_ONLY_CONF_ACCESSED: u32 = 0xD;
const BX_SEG_TYPE_CODE_EXEC_READ_CONF_ACCESSED: u32 = 0xF;
const BX_SEG_TYPE_LDT: u32 = 0x2;
const BX_SEG_TYPE_BUSY_286_TSS: u32 = 0x3;
const BX_SEG_TYPE_BUSY_386_TSS: u32 = 0xB;

// VMX-control allowed-{0,1} bit masks — Bochs vmx.cc reads these from
// IA32_VMX_*_CTLS MSRs and uses the low half ("allowed-0", bits that
// MUST be 1) and the high half ("allowed-1", bits that MAY be 1) to
// validate guest-supplied control values at VMENTRY. These must stay
// in lock-step with the values rdmsr_value returns for those MSRs in
// proc_ctrl.rs.
pub(super) const VMX_PINBASED_CTLS_ALLOWED_0: u32 = 0x0000_003F;
pub(super) const VMX_PINBASED_CTLS_ALLOWED_1: u32 = 0x0000_003F;
pub(super) const VMX_PROCBASED_CTLS_ALLOWED_0: u32 = 0x0401_E172;
pub(super) const VMX_PROCBASED_CTLS_ALLOWED_1: u32 = 0x0401_E172;
pub(super) const VMX_EXIT_CTLS_ALLOWED_0: u32 = 0;
pub(super) const VMX_EXIT_CTLS_ALLOWED_1: u32 = 0x0003_6FFF;
pub(super) const VMX_ENTRY_CTLS_ALLOWED_0: u32 = 0x0000_0011;
pub(super) const VMX_ENTRY_CTLS_ALLOWED_1: u32 = 0x0000_FFFF;
pub(super) const VMX_PROCBASED_CTLS2_ALLOWED_0: u32 = 0;
pub(super) const VMX_PROCBASED_CTLS2_ALLOWED_1: u32 =
    VMX_VM_EXEC_CTRL2_EPT_ENABLE
        | VMX_VM_EXEC_CTRL2_VPID_ENABLE
        | VMX_VM_EXEC_CTRL2_INVPCID;

/// INVEPT type field — Bochs vmx.cc INVEPT decodes this from the GPR
/// dereferenced by `i->dst()`. Numeric values are part of the SDM ABI.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InveptType {
    SingleContext = 1,
    AllContext = 2,
}

/// INVVPID type field — Bochs vmx.cc INVVPID. Same SDM-defined values.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InvvpidType {
    IndividualAddress = 0,
    SingleContext = 1,
    AllContext = 2,
    SingleContextNonGlobal = 3,
}

impl InveptType {
    /// Convert from the raw `type` field carried in the GPR. Returns
    /// `None` when the value is reserved — Bochs `VMfail` path.
    pub(super) const fn from_raw(v: u64) -> Option<Self> {
        match v {
            1 => Some(Self::SingleContext),
            2 => Some(Self::AllContext),
            _ => None,
        }
    }
}

impl InvvpidType {
    pub(super) const fn from_raw(v: u64) -> Option<Self> {
        match v {
            0 => Some(Self::IndividualAddress),
            1 => Some(Self::SingleContext),
            2 => Some(Self::AllContext),
            3 => Some(Self::SingleContextNonGlobal),
            _ => None,
        }
    }
}

// VM-execution secondary control bits used outside vmx.rs callers.
pub(super) const VMX_VM_EXEC_CTRL2_EPT_ENABLE: u32 = 1 << 1;
pub(super) const VMX_VM_EXEC_CTRL2_VPID_ENABLE: u32 = 1 << 5;
pub(super) const VMX_VM_EXEC_CTRL2_UNRESTRICTED_GUEST: u32 = 1 << 7;

// VM-exit secondary control bits.
pub(super) const VMX_VMEXIT_CTRL2_LOAD_HOST_FRED: u64 = 1 << 1;
pub(super) const VMX_VMEXIT_CTRL2_LOAD_HOST_IA32_SPEC_CTRL: u64 = 1 << 2;

// VM-entry control bits — Bochs vmx_ctrls.h.
pub(super) const VMX_VMENTRY_CTRL_X86_64_GUEST: u32 = 1 << 9;
pub(super) const VMX_VMENTRY_CTRL_LOAD_GUEST_EFER_MSR: u32 = 1 << 15;
pub(super) const VMX_VMENTRY_CTRL_LOAD_GUEST_PAT_MSR: u32 = 1 << 14;
pub(super) const VMX_VMENTRY_CTRL_LOAD_DBG_CTRLS: u32 = 1 << 2;
pub(super) const VMX_VMENTRY_CTRL_LOAD_GUEST_CET_STATE: u32 = 1 << 20;
pub(super) const VMX_VMENTRY_CTRL_LOAD_GUEST_PKRS: u32 = 1 << 22;
pub(super) const VMX_VMENTRY_CTRL_LOAD_GUEST_FRED: u32 = 1 << 23;
pub(super) const VMX_VMENTRY_CTRL_LOAD_GUEST_IA32_SPEC_CTRL: u32 = 1 << 24;

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

/// Sentinel for `vmcsptr` and `vmxonptr` indicating no VMCS / no VMXON region.
/// Bochs vmcs.h BX_INVALID_VMCSPTR.
pub(super) const BX_INVALID_VMCSPTR: u64 = 0xFFFFFFFFFFFFFFFF;

pub type VmcsCache = BxVmcs;

#[derive(Debug, Default)]
pub struct VmcsMapping {}

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
    /// Bochs `host_perf_global_ctrl` — IA32_PERF_GLOBAL_CTRL value loaded
    /// when `LOAD_PERF_GLOBAL_CTRL_MSR` exit control is set.
    pub host_perf_global_ctrl: u64,
    /// Bochs `host_pkrs` — IA32_PKRS value loaded when `LOAD_HOST_PKRS`
    /// exit control is set.
    pub host_pkrs: u64,
    /// Bochs `host_ia32_spec_ctrl` — IA32_SPEC_CTRL value loaded when
    /// `LOAD_HOST_IA32_SPEC_CTRL` (vmexit2 ctrl) is set.
    pub host_ia32_spec_ctrl: u64,
    /// CET host-state — Bochs `host_ia32_s_cet`, `host_ssp`,
    /// `host_intr_ssp_table_addr`. Loaded when `LOAD_HOST_CET_STATE`
    /// (vmexit1 ctrl bit 28) is set.
    pub host_ia32_s_cet: u64,
    pub host_ssp: u64,
    pub host_interrupt_ssp_table_addr: u64,
    /// FRED host-state. Loaded when `LOAD_HOST_FRED` (vmexit2 ctrl
    /// bit 1) is set. Bochs `host_fred_*`.
    pub host_fred_config: u64,
    pub host_fred_rsp: [u64; 4],
    pub host_fred_stack_levels: u64,
    pub host_fred_ssp: [u64; 4],

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

    // Guest CET state — loaded when VMX_VMENTRY_CTRL_LOAD_GUEST_CET_STATE is set,
    // saved unconditionally on VMEXIT (Bochs vmx.cc VMexitSaveGuestState).
    pub guest_ia32_s_cet: u64,
    pub guest_ssp: u64,
    pub guest_interrupt_ssp_table_addr: u64,

    // Guest FRED state — LOAD_GUEST_FRED / VMEXIT_CTRL2_SAVE_GUEST_FRED.
    pub guest_fred_config: u64,
    pub guest_fred_rsp: [u64; 4],
    pub guest_fred_stack_levels: u64,
    pub guest_fred_ssp: [u64; 4],

    // Guest IA32_PKRS — LOAD_GUEST_PKRS.
    pub guest_pkrs: u64,

    // Guest IA32_SPEC_CTRL — LOAD_GUEST_IA32_SPEC_CTRL.
    pub guest_ia32_spec_ctrl: u64,

    // Guest pending #DB exceptions — LOAD_DBG_CTRLS / SAVE_DBG_CTRLS.
    pub guest_pending_dbg_exceptions: u64,

    // Guest IA32_DEBUGCTL — LOAD_DBG_CTRLS / SAVE_DBG_CTRLS.
    pub guest_ia32_debugctl: u64,

    // Guest SMBASE — Bochs vmx.cc VMenterLoadCheckGuestState.
    pub guest_smbase: u32,

    // VM Functions — enable mask + EPTP-list address used by VMFUNC #0
    // (EPTP-switching). Bochs vmx.h VMCS_64BIT_CONTROL_VMFUNC_CTRLS /
    // VMCS_64BIT_CONTROL_EPTP_LIST_ADDRESS.
    pub vmfunc_ctrls: u64,
    pub eptp_list_address: u64,

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
    /// Bochs `vmexit_ctrls2` — VMCS 0x2044, holds VMX_VMEXIT_CTRL2_*
    /// (LOAD_HOST_FRED, LOAD_HOST_IA32_SPEC_CTRL, etc.).
    pub vm_exit_ctls2: u64,
    pub vm_entry_ctls: u32,
    pub vm_entry_intr_info: u32,
    pub vm_entry_exception_error_code: u32,
    pub vm_entry_instruction_length: u32,
    pub exception_bitmap: u32,
    // Page-fault error code mask/match for VMEXIT on #PF:
    // a #PF takes a VMEXIT iff `(errcode & vm_pf_mask) == vm_pf_match` equals
    // the exception_bitmap bit for #PF. Bochs vm_pf_mask / vm_pf_match.
    pub vm_pf_mask: u32,
    pub vm_pf_match: u32,
    // CR3-target filter for MOV CR3 writes (Bochs vm_cr3_target_cnt /
    // vm_cr3_target_value). A CR3 write that matches any enabled target
    // value does *not* VMEXIT even when CR3_WRITE_VMEXIT is set.
    pub vm_cr3_target_cnt: u32,
    pub vm_cr3_target_value: [u64; 4],
    pub cr0_guest_host_mask: u64,
    pub cr4_guest_host_mask: u64,
    pub cr0_read_shadow: u64,
    pub cr4_read_shadow: u64,
    pub vmcs_link_pointer: u64,
    pub tsc_offset: u64,
    /// Guest-physical address of the 4KiB MSR bitmap when
    /// `VMX_VM_EXEC_CTRL1_MSR_BITMAPS` is set. Bochs `msr_bitmap_addr`.
    pub msr_bitmap_addr: u64,
    /// Guest-physical addresses of the two 4KiB I/O permission bitmaps
    /// (A: ports 0x0000..=0x7FFF, B: ports 0x8000..=0xFFFF) when
    /// `VMX_VM_EXEC_CTRL1_IO_BITMAPS` is set. Bochs `io_bitmap_addr[2]`.
    pub io_bitmap_addr: [u64; 2],
    /// Bochs `vmread_bitmap_addr` — guest-physical address of the
    /// 4KiB VMREAD bitmap consulted when `VMCS_SHADOWING` is enabled.
    /// A bit SET at index `encoding` means the field is intercepted
    /// (VMEXIT to host); a clear bit lets the guest read from the
    /// shadow VMCS at `vmcs_link_pointer`.
    pub vmread_bitmap_addr: u64,
    /// Bochs `vmwrite_bitmap_addr` — guest-physical address of the
    /// 4KiB VMWRITE bitmap, same semantics as `vmread_bitmap_addr`
    /// but for VMWRITE.
    pub vmwrite_bitmap_addr: u64,
    /// 32-bit countdown value loaded into the VMX preemption timer at
    /// VMENTER and (when STORE_VMX_PREEMPTION_TIMER is set) snapshotted
    /// back at VMEXIT. Bochs reads this from the guest VMCS in
    /// vmlaunch/vmresume; ticking happens through the LAPIC.
    pub vmx_preemption_timer_value: u32,
    /// VMX TPR threshold — Bochs `tpr_threshold`. Bochs vmenter check
    /// requires this to be ≤ 15 because only the high 4 bits of TPR are
    /// virtualised (CR8 = TPR[7:4]).
    pub tpr_threshold: u32,
    /// Guest-physical base of the 4 KiB virtual-APIC page -- Bochs
    /// `virtual_apic_page_addr`. Used by the TPR-threshold VMEXIT path
    /// (vapic.cc) to read the virtual TPR byte at offset 0x80 when
    /// `VMX_VM_EXEC_CTRL1_TPR_SHADOW` is enabled.
    pub virtual_apic_page_addr: u64,
    /// Posted-interrupt descriptor address — Bochs `pid_addr`. 64-byte-
    /// aligned guest-physical address of the posted-interrupt descriptor
    /// when `VMX_PIN_BASED_VMEXEC_CTRL_PROCESS_POSTED_INTERRUPTS` is set.
    pub pi_desc_addr: u64,
    /// Posted-interrupt notification vector — Bochs
    /// `posted_intr_notification_vector`. Bochs validates < 256 at VMENTRY,
    /// so the value fits in u8.
    pub pi_notification_vector: u8,
    /// Virtual Processor Identifier — Bochs `vpid`. VMCS_16BIT_CONTROL_VPID
    /// (encoding 0x0). When `VPID_ENABLE` is set the guest's TLB entries
    /// are tagged with this value; must be non-zero per VMENTRY check.
    pub vpid: u16,
    /// Extended Page Table Pointer — Bochs `eptptr`. VMCS_64BIT_CONTROL_
    /// EPTPTR (0x201A). Valid when `VMX_VM_EXEC_CTRL2_EPT_ENABLE` is set;
    /// low 12 bits encode memory type + page-walk length, bits [51:12]
    /// are the EPT root paging-structure host-physical address.
    pub eptptr: u64,
    /// Guest-physical address that triggered the most recent EPT VMEXIT —
    /// VMCS_64BIT_GUEST_PHYSICAL_ADDR (0x2400). Bochs writes this on
    /// EPT violation / EPT misconfiguration exits.
    pub guest_physical_addr: u64,
    /// MSR load/store list addresses + counts — Bochs vmx.cc LoadMSRs /
    /// StoreMSRs. Each list is an array of 16-byte entries (low 4 bytes
    /// = MSR index, high 8 bytes = value).
    pub vmentry_msr_load_addr: u64,
    pub vmexit_msr_store_addr: u64,
    pub vmexit_msr_load_addr: u64,
    pub vmentry_msr_load_cnt: u32,
    pub vmexit_msr_store_cnt: u32,
    pub vmexit_msr_load_cnt: u32,

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
        if self.vmcsptr != BX_INVALID_VMCSPTR {
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
            // (see module-level [`BX_PHY_ADDRESS_WIDTH`]).
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

            self.vmcsptr = BX_INVALID_VMCSPTR;
            self.vmxonptr = paddr;
            self.in_vmx = true;
            self.mask_event(Self::BX_EVENT_INIT);
            self.monitor.reset_monitor();
            self.vmsucceed();
            return Ok(());
        }

        // Bochs vmx.cc VMXON in non-root operation: VMEXIT with reason
        // VMX_VMEXIT_VMXON. The qualification is zero (Bochs writes
        // exit_qualification=0 in VMexit_Instruction for VMX instruction
        // intercepts).
        if self.in_vmx_guest {
            self.vmx_vmexit(VmxVmexitReason::Vmxon, 0)?;
            return Ok(());
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
            // Bochs vmx.cc VMXOFF: VMexit(VMX_VMEXIT_VMXOFF, 0). The full
            // intercept-driven VMEXIT path is not wired here — collapse to
            // #GP so guest-mode VMXOFF doesn't silently succeed.
            return self.exception(Exception::Gp, 0);
        }

        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        self.vmxonptr = BX_INVALID_VMCSPTR;
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
            // Bochs vmx.cc VMCLEAR: VMexit(VMX_VMEXIT_VMCLEAR, 0). Until the
            // intercept-driven VMEXIT path is wired, surface as #GP so guest
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
            self.vmcsptr = BX_INVALID_VMCSPTR;
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

        // Bochs vmx.cc VMREAD_EdGd / VMREAD_EqGq: in non-root
        // operation, the architecturally-defined behaviour is to
        // consult the secondary-control VMCS_SHADOWING bit and the
        // vmread bitmap (Bochs vmexit.cc Vmexit_Vmread). When the
        // bit is clear the read is serviced from the shadow VMCS at
        // `vmcs_link_pointer`; otherwise the field VMEXITs to the
        // host. The full VMEXIT_VMREAD path is not yet modelled in
        // rusty_box, so an intercepted access still raises #GP —
        // matching the long-standing fallback while letting the
        // shadow path service guest reads correctly.
        let shadow_read = if self.in_vmx_guest {
            if self.vmread_intercepted(encoding) {
                self.exception(Exception::Gp, 0)?;
                unreachable!();
            }
            true
        } else {
            false
        };
        if self.cs_rpl() != 0 {
            self.exception(Exception::Gp, 0)?;
            unreachable!();
        }

        let target_vmcs = if shadow_read {
            self.vmcs.vmcs_link_pointer
        } else {
            self.vmcsptr
        };
        if target_vmcs == BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(0);
        }

        let v = if shadow_read {
            self.vmread_shadow_value(encoding, target_vmcs)
        } else {
            self.vmcs_read_field(encoding)
        };
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

        // Bochs vmx.cc VMWRITE_GdEd / VMWRITE_GqEq: mirror image of
        // VMREAD; the vmwrite bitmap and VMCS_SHADOWING gate whether
        // the write targets the shadow VMCS or VMEXITs.
        let shadow_write = if self.in_vmx_guest {
            if self.vmwrite_intercepted(encoding) {
                return self.exception(Exception::Gp, 0);
            }
            true
        } else {
            false
        };
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let target_vmcs = if shadow_write {
            self.vmcs.vmcs_link_pointer
        } else {
            self.vmcsptr
        };
        if target_vmcs == BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(());
        }

        // Bochs vmx.cc VMWRITE: VMCS_FIELD_TYPE bits [11:10] == 1 marks
        // a read-only (vm-exit info) field. Writes to those are rejected
        // with VMXERR_VMWRITE_READ_ONLY_VMCS_COMPONENT unless
        // IA32_VMX_MISC[29] (VMX_MISC_SUPPORT_VMWRITE_READ_ONLY_FIELDS)
        // is advertised. Our IA32_VMX_MISC reads 0 so the bit is clear
        // and read-only writes always fail. The check applies to both
        // primary and shadow VMCS writes.
        const VMCS_FIELD_TYPE_READ_ONLY: u32 = 1;
        if (encoding >> 10) & 0x3 == VMCS_FIELD_TYPE_READ_ONLY {
            self.vmfail(VmxErr::VmwriteReadOnlyVmcsComponent);
            return Ok(());
        }

        let ok = if shadow_write {
            self.vmwrite_shadow_value(encoding, target_vmcs, value)
        } else {
            self.vmcs_write_field(encoding, value)
        };
        if ok {
            self.vmsucceed();
            Ok(())
        } else {
            self.vmfail(VmxErr::UnsupportedVmcsComponentAccess);
            Ok(())
        }
    }

    /// Bochs vmexit.cc `Vmexit_Vmread` — returns true when a guest
    /// VMREAD must VMEXIT to the host instead of being serviced from
    /// the shadow VMCS. The bitmap is indexed by the raw 32-bit VMCS
    /// field encoding (one bit per encoding); bits beyond 0x7fff and
    /// the secondary-control gate force interception.
    fn vmread_intercepted(&mut self, encoding: u32) -> bool {
        if self.vmcs.secondary_proc_based_ctls & VMX_VM_EXEC_CTRL2_VMCS_SHADOWING == 0 {
            return true;
        }
        if encoding > 0x7fff {
            return true;
        }
        let pa = self.vmcs.vmread_bitmap_addr | (encoding as u64 >> 3);
        let byte = self.read_physical_byte(pa);
        byte & (1u8 << (encoding & 7)) != 0
    }

    /// Bochs vmexit.cc `Vmexit_Vmwrite` — mirror of
    /// `vmread_intercepted` for VMWRITE.
    fn vmwrite_intercepted(&mut self, encoding: u32) -> bool {
        if self.vmcs.secondary_proc_based_ctls & VMX_VM_EXEC_CTRL2_VMCS_SHADOWING == 0 {
            return true;
        }
        if encoding > 0x7fff {
            return true;
        }
        let pa = self.vmcs.vmwrite_bitmap_addr | (encoding as u64 >> 3);
        let byte = self.read_physical_byte(pa);
        byte & (1u8 << (encoding & 7)) != 0
    }

    /// Bochs vmx.cc `vmread_shadow` — fetches `encoding` from the
    /// shadow VMCS at `vmcs_pa`. The byte offset is computed from the
    /// canonical Bochs `vmcs_field_offset` mapping (vmcs.cc
    /// `init_generic_mapping`); width selects 16/32/64-bit access.
    /// Returns `None` for encodings outside Bochs' supported range so
    /// the caller maps it to VMXERR_UNSUPPORTED_VMCS_COMPONENT_ACCESS.
    fn vmread_shadow_value(&mut self, encoding: u32, vmcs_pa: u64) -> Option<u64> {
        let off = vmcs_field_byte_offset(encoding)?;
        let pa = vmcs_pa + off as u64;
        let width = (encoding >> 13) & 3;
        let is_hi = encoding & 1 != 0;
        Some(match width {
            VMCS_FIELD_WIDTH_16BIT => self.read_phys_word(pa) as u64,
            VMCS_FIELD_WIDTH_32BIT => self.read_phys_dword(pa) as u64,
            VMCS_FIELD_WIDTH_64BIT => {
                if is_hi {
                    self.read_phys_dword(pa) as u64
                } else {
                    self.read_phys_qword(pa)
                }
            }
            _ /* VMCS_FIELD_WIDTH_NATURAL */ => self.read_phys_qword(pa),
        })
    }

    /// Bochs vmx.cc `vmwrite_shadow` — stores `value` into the
    /// shadow VMCS at `vmcs_pa`. Width-specific writes match the read
    /// helper so VMREAD reproduces the same value.
    fn vmwrite_shadow_value(&mut self, encoding: u32, vmcs_pa: u64, value: u64) -> bool {
        let off = match vmcs_field_byte_offset(encoding) {
            Some(o) => o,
            None => return false,
        };
        let pa = vmcs_pa + off as u64;
        let width = (encoding >> 13) & 3;
        let is_hi = encoding & 1 != 0;
        match width {
            VMCS_FIELD_WIDTH_16BIT => self.write_phys_word(pa, value as u16),
            VMCS_FIELD_WIDTH_32BIT => self.write_phys_dword(pa, value as u32),
            VMCS_FIELD_WIDTH_64BIT => {
                if is_hi {
                    self.write_phys_dword(pa, value as u32);
                } else {
                    self.write_phys_qword(pa, value);
                }
            }
            _ /* VMCS_FIELD_WIDTH_NATURAL */ => self.write_phys_qword(pa, value),
        }
        true
    }

    fn read_phys_word(&mut self, paddr: u64) -> u16 {
        let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() else {
            return 0xffff;
        };
        let mut data = [0u8; 2];
        if let Err(e) = mem.read_physical_page(&[cpu_ref], paddr, 2, &mut data) {
            tracing::warn!("read_phys_word({:#018x}) failed: {:?}", paddr, e);
            return 0xffff;
        }
        u16::from_le_bytes(data)
    }

    fn read_phys_dword(&mut self, paddr: u64) -> u32 {
        let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() else {
            return 0xffff_ffff;
        };
        let mut data = [0u8; 4];
        if let Err(e) = mem.read_physical_page(&[cpu_ref], paddr, 4, &mut data) {
            tracing::warn!("read_phys_dword({:#018x}) failed: {:?}", paddr, e);
            return 0xffff_ffff;
        }
        u32::from_le_bytes(data)
    }

    fn read_phys_qword(&mut self, paddr: u64) -> u64 {
        let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() else {
            return u64::MAX;
        };
        let mut data = [0u8; 8];
        if let Err(e) = mem.read_physical_page(&[cpu_ref], paddr, 8, &mut data) {
            tracing::warn!("read_phys_qword({:#018x}) failed: {:?}", paddr, e);
            return u64::MAX;
        }
        u64::from_le_bytes(data)
    }

    fn write_phys_word(&mut self, paddr: u64, val: u16) {
        let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() else { return; };
        let mut data = val.to_le_bytes();
        let mut dummy_mapping: [u32; 0] = [];
        let mut stamp = super::icache::BxPageWriteStampTable {
            fine_granularity_mapping: &mut dummy_mapping,
        };
        if let Err(e) = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 2, &mut data) {
            tracing::warn!("write_phys_word({:#018x}) failed: {:?}", paddr, e);
        }
    }

    fn write_phys_dword(&mut self, paddr: u64, val: u32) {
        let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() else { return; };
        let mut data = val.to_le_bytes();
        let mut dummy_mapping: [u32; 0] = [];
        let mut stamp = super::icache::BxPageWriteStampTable {
            fine_granularity_mapping: &mut dummy_mapping,
        };
        if let Err(e) = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 4, &mut data) {
            tracing::warn!("write_phys_dword({:#018x}) failed: {:?}", paddr, e);
        }
    }

    fn write_phys_qword(&mut self, paddr: u64, val: u64) {
        let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() else { return; };
        let mut data = val.to_le_bytes();
        let mut dummy_mapping: [u32; 0] = [];
        let mut stamp = super::icache::BxPageWriteStampTable {
            fine_granularity_mapping: &mut dummy_mapping,
        };
        if let Err(e) = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 8, &mut data) {
            tracing::warn!("write_phys_qword({:#018x}) failed: {:?}", paddr, e);
        }
    }

    /// Dispatch a VMCS field encoding → named field in `self.vmcs`.
    /// Returns `None` if the encoding is not recognised.
    fn vmcs_read_field(&self, encoding: u32) -> Option<u64> {
        let v = &self.vmcs;
        Some(match encoding {
            // 16-bit guest selectors.
            VMCS_16BIT_CONTROL_VPID => u64::from(v.vpid),
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
            VMCS_64BIT_CONTROL_IO_BITMAP_A => v.io_bitmap_addr[0],
            VMCS_64BIT_CONTROL_IO_BITMAP_B => v.io_bitmap_addr[1],
            VMCS_64BIT_CONTROL_MSR_BITMAPS => v.msr_bitmap_addr,
            VMCS_64BIT_CONTROL_VMREAD_BITMAP_ADDR => v.vmread_bitmap_addr,
            VMCS_64BIT_CONTROL_VMWRITE_BITMAP_ADDR => v.vmwrite_bitmap_addr,
            VMCS_64BIT_CONTROL_VMEXIT_MSR_STORE_ADDR => v.vmexit_msr_store_addr,
            VMCS_64BIT_CONTROL_VMEXIT_MSR_LOAD_ADDR => v.vmexit_msr_load_addr,
            VMCS_64BIT_CONTROL_VMENTRY_MSR_LOAD_ADDR => v.vmentry_msr_load_addr,
            VMCS_32BIT_CONTROL_VMEXIT_MSR_STORE_COUNT => v.vmexit_msr_store_cnt as u64,
            VMCS_32BIT_CONTROL_VMEXIT_MSR_LOAD_COUNT => v.vmexit_msr_load_cnt as u64,
            VMCS_32BIT_CONTROL_VMENTRY_MSR_LOAD_COUNT => v.vmentry_msr_load_cnt as u64,
            VMCS_64BIT_CONTROL_TSC_OFFSET => v.tsc_offset,
            VMCS_64BIT_CONTROL_VIRTUAL_APIC_PAGE_ADDR => v.virtual_apic_page_addr,
            VMCS_64BIT_CONTROL_EPTPTR => v.eptptr,
            VMCS_64BIT_GUEST_PHYSICAL_ADDR => v.guest_physical_addr,
            VMCS_64BIT_CONTROL_SECONDARY_VMEXIT_CONTROLS => v.vm_exit_ctls2,
            VMCS_64BIT_GUEST_IA32_EFER => v.guest_ia32_efer,
            VMCS_64BIT_GUEST_IA32_PAT => v.guest_ia32_pat,
            VMCS_64BIT_HOST_IA32_PAT => v.host_ia32_pat,
            VMCS_64BIT_HOST_IA32_EFER => v.host_ia32_efer,
            VMCS_64BIT_HOST_IA32_PERF_GLOBAL_CTRL => v.host_perf_global_ctrl,
            VMCS_64BIT_HOST_IA32_PKRS => v.host_pkrs,
            VMCS_64BIT_HOST_IA32_SPEC_CTRL => v.host_ia32_spec_ctrl,
            VMCS_64BIT_HOST_IA32_FRED_CONFIG => v.host_fred_config,
            VMCS_64BIT_HOST_IA32_FRED_RSP1 => v.host_fred_rsp[1],
            VMCS_64BIT_HOST_IA32_FRED_RSP2 => v.host_fred_rsp[2],
            VMCS_64BIT_HOST_IA32_FRED_RSP3 => v.host_fred_rsp[3],
            VMCS_64BIT_HOST_IA32_FRED_STACK_LEVELS => v.host_fred_stack_levels,
            VMCS_64BIT_HOST_IA32_FRED_SSP1 => v.host_fred_ssp[1],
            VMCS_64BIT_HOST_IA32_FRED_SSP2 => v.host_fred_ssp[2],
            VMCS_64BIT_HOST_IA32_FRED_SSP3 => v.host_fred_ssp[3],
            VMCS_HOST_IA32_S_CET => v.host_ia32_s_cet,
            VMCS_HOST_SSP => v.host_ssp,
            VMCS_HOST_INTERRUPT_SSP_TABLE_ADDR => v.host_interrupt_ssp_table_addr,
            // 32-bit control fields.
            VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS => v.pin_based_ctls as u64,
            VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS => v.proc_based_ctls as u64,
            VMCS_32BIT_CONTROL_EXECUTION_BITMAP => v.exception_bitmap as u64,
            VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MASK => v.vm_pf_mask as u64,
            VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MATCH => v.vm_pf_match as u64,
            VMCS_32BIT_CONTROL_CR3_TARGET_COUNT => v.vm_cr3_target_cnt as u64,
            VMCS_CR3_TARGET0 => v.vm_cr3_target_value[0],
            VMCS_CR3_TARGET1 => v.vm_cr3_target_value[1],
            VMCS_CR3_TARGET2 => v.vm_cr3_target_value[2],
            VMCS_CR3_TARGET3 => v.vm_cr3_target_value[3],
            VMCS_32BIT_CONTROL_SECONDARY_VMEXEC_CONTROLS => v.secondary_proc_based_ctls as u64,
            VMCS_32BIT_CONTROL_VMEXIT_CONTROLS => v.vm_exit_ctls as u64,
            VMCS_32BIT_CONTROL_VMENTRY_CONTROLS => v.vm_entry_ctls as u64,
            VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO => v.vm_entry_intr_info as u64,
            VMCS_32BIT_CONTROL_TPR_THRESHOLD => v.tpr_threshold as u64,
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
            VMCS_32BIT_GUEST_PREEMPTION_TIMER_VALUE => v.vmx_preemption_timer_value as u64,
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
            VMCS_16BIT_CONTROL_VPID => v.vpid = value as u16,
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
            VMCS_64BIT_CONTROL_IO_BITMAP_A => v.io_bitmap_addr[0] = value,
            VMCS_64BIT_CONTROL_IO_BITMAP_B => v.io_bitmap_addr[1] = value,
            VMCS_64BIT_CONTROL_MSR_BITMAPS => v.msr_bitmap_addr = value,
            VMCS_64BIT_CONTROL_VMREAD_BITMAP_ADDR => v.vmread_bitmap_addr = value,
            VMCS_64BIT_CONTROL_VMWRITE_BITMAP_ADDR => v.vmwrite_bitmap_addr = value,
            VMCS_64BIT_CONTROL_VMEXIT_MSR_STORE_ADDR => v.vmexit_msr_store_addr = value,
            VMCS_64BIT_CONTROL_VMEXIT_MSR_LOAD_ADDR => v.vmexit_msr_load_addr = value,
            VMCS_64BIT_CONTROL_VMENTRY_MSR_LOAD_ADDR => v.vmentry_msr_load_addr = value,
            VMCS_32BIT_CONTROL_VMEXIT_MSR_STORE_COUNT => v.vmexit_msr_store_cnt = value as u32,
            VMCS_32BIT_CONTROL_VMEXIT_MSR_LOAD_COUNT => v.vmexit_msr_load_cnt = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_MSR_LOAD_COUNT => v.vmentry_msr_load_cnt = value as u32,
            VMCS_64BIT_CONTROL_TSC_OFFSET => v.tsc_offset = value,
            VMCS_64BIT_CONTROL_VIRTUAL_APIC_PAGE_ADDR => v.virtual_apic_page_addr = value,
            VMCS_64BIT_CONTROL_EPTPTR => v.eptptr = value,
            VMCS_64BIT_CONTROL_SECONDARY_VMEXIT_CONTROLS => v.vm_exit_ctls2 = value,
            VMCS_64BIT_GUEST_IA32_EFER => v.guest_ia32_efer = value,
            VMCS_64BIT_GUEST_IA32_PAT => v.guest_ia32_pat = value,
            VMCS_64BIT_HOST_IA32_PAT => v.host_ia32_pat = value,
            VMCS_64BIT_HOST_IA32_EFER => v.host_ia32_efer = value,
            VMCS_64BIT_HOST_IA32_PERF_GLOBAL_CTRL => v.host_perf_global_ctrl = value,
            VMCS_64BIT_HOST_IA32_PKRS => v.host_pkrs = value,
            VMCS_64BIT_HOST_IA32_SPEC_CTRL => v.host_ia32_spec_ctrl = value,
            VMCS_64BIT_HOST_IA32_FRED_CONFIG => v.host_fred_config = value,
            VMCS_64BIT_HOST_IA32_FRED_RSP1 => v.host_fred_rsp[1] = value,
            VMCS_64BIT_HOST_IA32_FRED_RSP2 => v.host_fred_rsp[2] = value,
            VMCS_64BIT_HOST_IA32_FRED_RSP3 => v.host_fred_rsp[3] = value,
            VMCS_64BIT_HOST_IA32_FRED_STACK_LEVELS => v.host_fred_stack_levels = value,
            VMCS_64BIT_HOST_IA32_FRED_SSP1 => v.host_fred_ssp[1] = value,
            VMCS_64BIT_HOST_IA32_FRED_SSP2 => v.host_fred_ssp[2] = value,
            VMCS_64BIT_HOST_IA32_FRED_SSP3 => v.host_fred_ssp[3] = value,
            VMCS_HOST_IA32_S_CET => v.host_ia32_s_cet = value,
            VMCS_HOST_SSP => v.host_ssp = value,
            VMCS_HOST_INTERRUPT_SSP_TABLE_ADDR => v.host_interrupt_ssp_table_addr = value,
            VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS => v.pin_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS => v.proc_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_EXECUTION_BITMAP => v.exception_bitmap = value as u32,
            VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MASK => v.vm_pf_mask = value as u32,
            VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MATCH => v.vm_pf_match = value as u32,
            VMCS_32BIT_CONTROL_CR3_TARGET_COUNT => v.vm_cr3_target_cnt = value as u32,
            VMCS_CR3_TARGET0 => v.vm_cr3_target_value[0] = value,
            VMCS_CR3_TARGET1 => v.vm_cr3_target_value[1] = value,
            VMCS_CR3_TARGET2 => v.vm_cr3_target_value[2] = value,
            VMCS_CR3_TARGET3 => v.vm_cr3_target_value[3] = value,
            VMCS_32BIT_CONTROL_SECONDARY_VMEXEC_CONTROLS => v.secondary_proc_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMEXIT_CONTROLS => v.vm_exit_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_CONTROLS => v.vm_entry_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO => v.vm_entry_intr_info = value as u32,
            VMCS_32BIT_CONTROL_TPR_THRESHOLD => v.tpr_threshold = value as u32,
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
            VMCS_32BIT_GUEST_PREEMPTION_TIMER_VALUE => v.vmx_preemption_timer_value = value as u32,
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
            // Bochs vmx.cc VMLAUNCH/VMRESUME from guest mode is a #UD-or-VMexit
            // path; until that intercept is wired here, surface as #GP.
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }
        if self.vmcsptr == BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(());
        }

        // Bochs vmx.cc VMLAUNCH/VMRESUME: a pending MOV_SS interrupt
        // shadow makes VMENTRY illegal — the inhibition would otherwise
        // suppress the host's first guest event.
        if self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS) {
            self.vmfail(VmxErr::VmentryMovSsBlocking);
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

        // Bochs vmx.cc steps 1-3: validate VMX-execution controls + host
        // state + guest state before swapping. Failures here surface as
        // VMfail with the matching error code (Bochs VMXERR_VMENTRY_*)
        // and the VMENTRY abandons before any state is swapped.
        if let Some(err) = self.vmenter_load_check_vm_controls() {
            self.vmfail(err);
            return Ok(());
        }
        if let Some(err) = self.vmenter_load_check_host_state() {
            self.vmfail(err);
            return Ok(());
        }
        // Bochs vmx.cc VMLAUNCH: guest-state failure is a VMEXIT with
        // VMX_VMEXIT_VMENTRY_FAILURE_GUEST_STATE | (1<<31) and a per-check
        // qualification — not a VMfail. The error code threaded out of
        // vmenter_load_check_guest_state currently doubles as the qualification;
        // wiring per-check qualifications (Bochs writes a non-zero VMENTER_ERR_*
        // code at each failure site) is tracked separately.
        if let Some(_err) = self.vmenter_load_check_guest_state() {
            return self
                .vmx_vmexit_vmentry_failure(VmxVmexitReason::VmentryFailureGuestState, 0);
        }

        // Save host state from the running CPU. RIP is "the instruction after
        // VMLAUNCH / VMRESUME"; Bochs stashes it so VMEXIT_LOAD_HOST_STATE can
        // jump back. The prefetch queue already advanced past this insn, so
        // `self.rip()` points at the next one.
        self.vmenter_save_host_state();

        // Load full guest state from the VMCS — mirrors Bochs vmx.cc
        // VMenterLoadCheckGuestState (load step). Includes CRs, EFER,
        // PAT, segments + LDTR + TR, GDTR/IDTR, sysenter MSRs, activity
        // state, optional CET / FRED / PKRS / IA32_SPEC_CTRL, DR7,
        // pending #DB \u2192 debug_trap, and CPL recompute.
        self.vmenter_load_guest_state()?;

        // Bochs vmx.cc VMLAUNCH/VMRESUME ordering: enter guest mode
        // first, then unmask INIT (allowed in non-root operation), then
        // set up TSC offset / preemption timer / event signaling.
        // `launched` is updated only on the VMLAUNCH path AFTER the
        // VM-entry MSR-load succeeds — moved below.
        self.in_vmx_guest = true;
        self.unmask_event(Self::BX_EVENT_INIT);

        // Bochs vmx.cc VMenter — apply guest STI/MOV_SS shadow before
        // any interrupt-relevant event signaling:
        //   if (interruptibility_state & STI)         inhibit_interrupts(BX_INHIBIT_INTERRUPTS);
        //   else if (interruptibility_state & MOV_SS) inhibit_interrupts(BX_INHIBIT_INTERRUPTS_BY_MOVSS);
        //   else                                      inhibit_mask = 0;
        const BX_VMX_INTERRUPTS_BLOCKED_BY_STI: u32 = 1 << 0;
        const BX_VMX_INTERRUPTS_BLOCKED_BY_MOV_SS: u32 = 1 << 1;
        const BX_VMX_INTERRUPTS_BLOCKED_NMI_BLOCKED: u32 = 1 << 3;
        let interruptibility = self.vmcs.guest_interruptibility_state;
        if interruptibility & BX_VMX_INTERRUPTS_BLOCKED_BY_STI != 0 {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS);
        } else if interruptibility & BX_VMX_INTERRUPTS_BLOCKED_BY_MOV_SS != 0 {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS);
        } else {
            self.inhibit_mask = 0;
        }

        // Bochs vmx.cc VMenter — initial NMI mask state:
        //   unmask_event(BX_EVENT_VMX_VIRTUAL_NMI | BX_EVENT_NMI);
        //   if (interruptibility_state & NMI_BLOCKED) {
        //     if (VIRTUAL_NMI()) mask_event(VMX_VIRTUAL_NMI);
        //     else               mask_event(NMI);
        //   }
        self.unmask_event(Self::BX_EVENT_VMX_VIRTUAL_NMI | Self::BX_EVENT_NMI);
        if interruptibility & BX_VMX_INTERRUPTS_BLOCKED_NMI_BLOCKED != 0 {
            if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_VIRTUAL_NMI != 0 {
                self.mask_event(Self::BX_EVENT_VMX_VIRTUAL_NMI);
            } else {
                self.mask_event(Self::BX_EVENT_NMI);
            }
        }

        // Bochs vmx.cc VMenter post-state-load event signaling:
        //   if (MONITOR_TRAP_FLAG) { signal_event(MTF); mask_event(MTF); }
        //   if (NMI_WINDOW_EXITING)         signal_event(VMX_VIRTUAL_NMI);
        //   if (INTERRUPT_WINDOW_VMEXIT)    signal_event(VMX_INTERRUPT_WINDOW_EXITING);
        // The MTF signal+mask pair lets the next instruction-boundary
        // unmask MTF (Bochs event.cc Priority-3 branch) so the MTF
        // VMEXIT fires AFTER the first guest instruction, not during
        // VMENTRY itself. All three events are cleared at VMEXIT.
        let proc1 = self.vmcs.proc_based_ctls;
        if proc1 & VMX_VM_EXEC_CTRL1_MONITOR_TRAP_FLAG != 0 {
            self.signal_event(Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG);
            self.mask_event(Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG);
        }
        if proc1 & VMX_VM_EXEC_CTRL1_NMI_WINDOW_EXITING != 0 {
            self.signal_event(Self::BX_EVENT_VMX_VIRTUAL_NMI);
        }
        if proc1 & VMX_VM_EXEC_CTRL1_INTERRUPT_WINDOW_VMEXIT != 0 {
            self.signal_event(Self::BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING);
        }

        // Bochs vmx.cc step 5: walk the VM-entry MSR-load list and write
        // each (msr, value) pair into the guest. A non-zero failing index
        // is a VMENTRY-failure VMEXIT with reason VMX_VMEXIT_VMENTRY_
        // FAILURE_MSR | (1<<31) and the failing index as qualification —
        // not a host #UD.
        let load_cnt = self.vmcs.vmentry_msr_load_cnt;
        let load_addr = self.vmcs.vmentry_msr_load_addr;
        if load_cnt != 0 {
            let failing = self.vmx_load_msrs(load_cnt, load_addr)?;
            if failing != 0 {
                tracing::error!(
                    "VMENTRY MSR load list rejected entry {failing}; VMENTRY-failure VMEXIT"
                );
                return self.vmx_vmexit_vmentry_failure(
                    VmxVmexitReason::VmentryFailureMsr,
                    u64::from(failing),
                );
            }
        }

        // Bochs vmx.cc VMLAUNCH/VMRESUME step 6: only the VMLAUNCH path
        // promotes the launch state to LAUNCHED, and only AFTER the
        // VM-entry MSR-load succeeds. VMRESUME leaves the launch state
        // alone — Bochs gates this on the `vmlaunch` flag.
        if !is_resume {
            self.vmcs.launched = true;
        }

        // Bochs vmx.cc step 6: arm the preemption timer when the pin-based
        // control is set. Reads vmx_preemption_timer_value from the VMCS.
        self.vmenter_arm_preemption_timer();
        // Bochs vmx.cc step 7: inject any event the host queued in
        // VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO. Failure to deliver
        // is propagated up through the standard exception path.
        if let Err(e) = self.vmenter_inject_events() {
            self.invalidate_prefetch_q();
            return Err(e);
        }
        self.vmsucceed();
        // Guest now runs from the loaded RIP — the CPU loop picks up the new
        // prefetch target after this instruction returns.
        self.invalidate_prefetch_q();
        Ok(())
    }

    /// VMCALL — Bochs vmx.cc VMCALL.
    pub(super) fn vmcall(&mut self, _instr: &Instruction) -> Result<()> {
        if !self.in_vmx {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            return self.vmx_vmexit(VmxVmexitReason::Vmcall, 0);
        }
        // Bochs VMCALL: VM/compat-mode \u2192 #UD; non-CPL0 \u2192 #GP; otherwise
        // VMfail when the VMCS is invalid or already launched. The full
        // dual-monitor-SMI path Bochs panics on stays unimplemented;
        // surface it as VMfail too — the host gets the canonical
        // \"VMCALL non-clear VMCS\" error rather than a silent NOP.
        if self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }
        if self.vmcsptr == BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(());
        }
        if self.vmcs.launched {
            self.vmfail(VmxErr::VmcallNonClearVmcs);
            return Ok(());
        }
        // Dual-monitor SMI treatment unimplemented — Bochs panics here.
        tracing::warn!("VMCALL: dual-monitor SMM treatment not implemented");
        self.vmfail(VmxErr::VmcallNonClearVmcs);
        Ok(())
    }

    /// VMFUNC — Bochs vmfunc.cc VMFUNC.
    pub(super) fn vmfunc(&mut self, _instr: &Instruction) -> Result<()> {
        if !self.in_vmx_guest
            || self.vmcs.secondary_proc_based_ctls & VMX_VM_EXEC_CTRL2_VMFUNC_ENABLE == 0
        {
            return self.exception(Exception::Ud, 0);
        }
        let function = self.eax();
        if function >= 64 {
            return self.exception(Exception::Ud, 0);
        }
        if self.vmcs.vmfunc_ctrls & (1u64 << function) == 0 {
            return self.vmx_vmexit(VmxVmexitReason::Vmfunc, 0);
        }
        match function {
            0 => {
                // EPTP-switching — Bochs vmfunc_eptp_switching.
                let ecx = self.ecx() as u64;
                if ecx >= 512 {
                    return self.vmx_vmexit(VmxVmexitReason::Vmfunc, 0);
                }
                let paddr = self.vmcs.eptp_list_address.wrapping_add(ecx * 8);
                let bytes = [
                    self.read_physical_byte(paddr),
                    self.read_physical_byte(paddr + 1),
                    self.read_physical_byte(paddr + 2),
                    self.read_physical_byte(paddr + 3),
                    self.read_physical_byte(paddr + 4),
                    self.read_physical_byte(paddr + 5),
                    self.read_physical_byte(paddr + 6),
                    self.read_physical_byte(paddr + 7),
                ];
                let new_eptp = u64::from_le_bytes(bytes);
                if !self.is_eptptr_valid(new_eptp) {
                    return self.vmx_vmexit(VmxVmexitReason::Vmfunc, 0);
                }
                self.vmcs.eptptr = new_eptp;
                // VPID-tagged TLB not modelled — invalidate the prefetch
                // queue so the next fetch re-walks under the new EPTP.
                self.invalidate_prefetch_q();
                Ok(())
            }
            _ => self.vmx_vmexit(VmxVmexitReason::Vmfunc, 0),
        }
    }

    // =========================================================================
    // TPR-threshold VMEXIT trigger — Bochs vapic.cc VMX_TPR_Threshold_Vmexit.
    //
    // Wired up: this helper runs from the CR8 write paths (and, when the xAPIC
    // MMIO TPR write hook acquires a CpuC back-reference, from the LAPIC TPR
    // write hook). It raises VmxVmexitReason::TprThreshold whenever the upper
    // nibble of the virtual TPR drops below the 4-bit threshold field.
    //
    // Not yet ported: full Bochs vapic.cc — APIC-access page virtualisation,
    // APIC register virtualisation, and virtual-interrupt delivery. VMENTRY
    // validation rejects those controls (see vmenter check_vm_controls)
    // pending a follow-up port; see cpp_orig/bochs/cpu/vapic.cc.
    // =========================================================================

    /// Bochs vapic.cc `VMX_TPR_Threshold_Vmexit` -- raises a TPR-threshold
    /// VMEXIT (trap-like, qualification = 0) when the guest's virtual TPR
    /// upper nibble drops below the host's 4-bit threshold. Called from the
    /// CR8 / xAPIC TPR write path. No-op outside VMX guest mode or when the
    /// TPR_SHADOW execution control is clear.
    pub(super) fn vmx_tpr_threshold_vmexit(&mut self) -> Result<()> {
        if !self.in_vmx || !self.in_vmx_guest {
            return Ok(());
        }
        if self.vmcs.proc_based_ctls & VMX_VM_EXEC_CTRL1_TPR_SHADOW == 0 {
            return Ok(());
        }
        // VMENTRY check_vm_controls already rejected `tpr_threshold > 15`,
        // but mask defensively in case the field is later extended.
        let threshold = (self.vmcs.tpr_threshold & 0xF) as u8;
        let virt_tpr_high = self.read_virtual_apic_tpr_byte() >> 4;
        if virt_tpr_high < threshold {
            return self.vmx_vmexit(VmxVmexitReason::TprThreshold, 0);
        }
        Ok(())
    }

    /// Read the virtual-APIC TPR byte (offset 0x80) from the guest's
    /// virtual-APIC page. Bochs vapic.cc `VMX_Read_Virtual_APIC`.
    fn read_virtual_apic_tpr_byte(&mut self) -> u8 {
        let virt_apic = self.vmcs.virtual_apic_page_addr;
        if virt_apic == 0 {
            // Bochs requires a valid virtual-APIC page when TPR_SHADOW is
            // set; if rusty_box hasn't loaded one yet, return 0 so the
            // threshold check never spuriously fires.
            return 0;
        }
        self.read_physical_byte(virt_apic + 0x80)
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
        // Bochs vmx.cc `BX_CPU_C::VMexit` head-guard: a VMEXIT outside
        // VMX-guest mode without the bit-31 vmentry-failure flag is a
        // host-side CPU implementation bug — Bochs `BX_PANIC`s. Our
        // regular VMEXIT path is never called with bit-31 set (that's
        // `vmx_vmexit_vmentry_failure`), so the guard reduces to a
        // VmxInternalError when not in guest mode.
        if !self.in_vmx || !self.in_vmx_guest {
            tracing::error!(
                "VMEXIT not in VMX guest mode (reason={reason:?}, qualification={qualification:#x})"
            );
            return Err(super::error::CpuError::VmxInternalError {
                reason: VmxInternalReason::VmexitOutsideGuestMode,
            });
        }

        // ── Bochs `BX_CPU_C::VMexit` STEP 0 ──────────────────────────
        // Disarm the preemption timer (snapshots back into VMCS when
        // STORE_VMX_PREEMPTION_TIMER is set), record reason +
        // qualification, write the VMEXIT_INSTRUCTION_LENGTH field
        // (`(RIP - prev_rip) & 0xf`).
        self.vmexit_disarm_preemption_timer();
        self.vmcs.exit_reason = reason as u32;
        self.vmcs.exit_qualification = qualification;
        self.vmcs.exit_instruction_length =
            ((self.rip().wrapping_sub(self.prev_rip)) & 0xF) as u32;

        // Bochs vmx.cc VMexit: when the reason is EXCEPTION_NMI the
        // vector is the low byte of VMCS_32BIT_VMEXIT_INTERRUPTION_INFO;
        // otherwise the field gets cleared.
        let vector = if reason == VmxVmexitReason::ExceptionNmi {
            (self.vmcs.exit_intr_info & 0xFF) as u8
        } else {
            0
        };
        if reason != VmxVmexitReason::ExceptionNmi
            && reason != VmxVmexitReason::ExternalInterrupt
        {
            self.vmcs.exit_intr_info = 0;
        }

        // Bochs vmx.cc VMexit: surface the IDT-vectoring info captured
        // by `interrupt()` when the exit happened *during* event
        // delivery (`in_event`). The valid bit (bit 31) is OR'd in to
        // tell the host the field is meaningful. When no event was in
        // flight the field is cleared to 0.
        if self.in_event {
            self.vmcs.idt_vectoring_info = self.vmcs.idt_vectoring_info | 0x8000_0000;
            // idt_vectoring_error_code already populated by the event
            // dispatcher — leave it untouched.
            self.in_event = false;
        } else {
            self.vmcs.idt_vectoring_info = 0;
            self.vmcs.idt_vectoring_error_code = 0;
        }

        self.nmi_unblocking_iret = false;

        // Bochs vmx.cc VMexit: VMEXITs are *fault-like* — restore RIP
        // (and RSP/SSP if the instruction speculatively advanced them)
        // to the value before the faulting instruction. Trap-like
        // exits (TPR_THRESHOLD, VIRTUALIZED_EOI, APIC_WRITE, BUS_LOCK)
        // are taken AFTER the instruction completes, so they keep the
        // post-instruction RIP/RSP/SSP.
        if !reason.is_trap_like() {
            self.set_rip(self.prev_rip);
            if self.speculative_rsp {
                self.set_rsp(self.prev_rsp);
                self.set_ssp(self.prev_ssp);
            }
        }
        self.speculative_rsp = false;

        // ── STEP 1: save guest state + walk MSR-STORE list ───────────
        // Bochs vmx.cc gates this on non-vmentry-failure reasons. The
        // regular vmx_vmexit path is never entered with these reasons
        // (failure goes through vmx_vmexit_vmentry_failure), but the
        // explicit guard mirrors Bochs and protects against future
        // mis-dispatch.
        if reason != VmxVmexitReason::VmentryFailureGuestState
            && reason != VmxVmexitReason::VmentryFailureMsr
        {
            // Clear the VMENTRY interruption-info valid bit so a
            // re-entry doesn't re-inject the previous event.
            self.vmcs.vm_entry_intr_info &= !0x8000_0000;

            // Bochs vmx.cc VMexitSaveGuestState. CRs / RSP / RIP /
            // RFLAGS are saved unconditionally; DR7 + IA32_DEBUGCTL
            // gate on SAVE_DBG_CTRLS; PAT / EFER on STORE_*_MSR; FRED
            // on SAVE_GUEST_FRED; CET / PKRS / SPEC_CTRL are written
            // unconditionally to mirror Bochs (no SAVE_GUEST_* gate).
            let exit_ctls = self.vmcs.vm_exit_ctls;
            let exit_ctls2 = self.vmcs.vm_exit_ctls2;

            self.vmcs.guest_cr0 = self.cr0.get32() as u64;
            self.vmcs.guest_cr3 = self.cr3;
            self.vmcs.guest_cr4 = self.cr4.get() as u64;
            self.vmcs.guest_rsp = self.rsp();
            self.vmcs.guest_rip = self.rip();
            self.vmcs.guest_rflags = self.read_eflags() as u64;

            if exit_ctls & VMX_VMEXIT_CTRL1_SAVE_DBG_CTRLS != 0 {
                self.vmcs.guest_dr7 = u64::from(self.dr7.bits());
                self.vmcs.guest_ia32_debugctl = 0;
            }
            if exit_ctls & VMX_VMEXIT_CTRL1_STORE_PAT_MSR != 0 {
                self.vmcs.guest_ia32_pat = self.msr.pat.U64();
            }
            if exit_ctls & VMX_VMEXIT_CTRL1_STORE_EFER_MSR != 0 {
                self.vmcs.guest_ia32_efer = self.efer.get32() as u64;
            }

            // CET state — Bochs saves unconditionally when CET is
            // supported (the loaded value is meaningless when the guest
            // never enabled CET, but mirrors Bochs).
            self.vmcs.guest_ia32_s_cet = self.msr.ia32_cet_control[0];
            self.vmcs.guest_ssp = self.ssp();
            self.vmcs.guest_interrupt_ssp_table_addr = self.msr.ia32_interrupt_ssp_table;

            // PKRS — Bochs saves unconditionally when PKS supported.
            self.vmcs.guest_pkrs = u64::from(self.pkrs);

            // FRED — gated on the secondary VMEXIT control bit.
            if exit_ctls2 & VMX_VMEXIT_CTRL2_SAVE_GUEST_FRED != 0 {
                self.vmcs.guest_fred_config = self.msr.ia32_fred_cfg;
                self.vmcs.guest_fred_stack_levels = self.msr.ia32_fred_stack_levels;
                for i in 1..4 {
                    self.vmcs.guest_fred_rsp[i] = self.msr.ia32_fred_rsp[i];
                    self.vmcs.guest_fred_ssp[i] = self.msr.ia32_fred_ssp[i];
                }
            }

            // IA32_SPEC_CTRL — Bochs saves unconditionally.
            self.vmcs.guest_ia32_spec_ctrl = u64::from(self.msr.ia32_spec_ctrl);

            // Six data/code segments + LDTR + TR — Bochs vmx.cc.
            self.vmexit_save_guest_seg(BxSegregs::Es);
            self.vmexit_save_guest_seg(BxSegregs::Cs);
            self.vmexit_save_guest_seg(BxSegregs::Ss);
            self.vmexit_save_guest_seg(BxSegregs::Ds);
            self.vmexit_save_guest_seg(BxSegregs::Fs);
            self.vmexit_save_guest_seg(BxSegregs::Gs);

            // LDTR.
            self.vmcs.guest_ldtr_selector = self.ldtr.selector.value;
            self.vmcs.guest_ldtr_base = self.ldtr.cache.u.segment_base();
            self.vmcs.guest_ldtr_limit = self.ldtr.cache.u.segment_limit_scaled();
            self.vmcs.guest_ldtr_ar = {
                let ar_byte = self.ldtr.cache.get_ar_byte() as u32;
                let mut ar = ar_byte & 0xFF;
                ar |= (self.ldtr.cache.u.segment_avl() as u32) << 12;
                ar |= (self.ldtr.cache.u.segment_l() as u32) << 13;
                ar |= (self.ldtr.cache.u.segment_d_b() as u32) << 14;
                ar |= (self.ldtr.cache.u.segment_g() as u32) << 15;
                if self.ldtr.cache.valid == 0 { ar |= 1 << 16; }
                ar
            };

            // TR.
            self.vmcs.guest_tr_selector = self.tr.selector.value;
            self.vmcs.guest_tr_base = self.tr.cache.u.segment_base();
            self.vmcs.guest_tr_limit = self.tr.cache.u.segment_limit_scaled();
            self.vmcs.guest_tr_ar = {
                let ar_byte = self.tr.cache.get_ar_byte() as u32;
                let mut ar = ar_byte & 0xFF;
                ar |= (self.tr.cache.u.segment_avl() as u32) << 12;
                ar |= (self.tr.cache.u.segment_l() as u32) << 13;
                ar |= (self.tr.cache.u.segment_d_b() as u32) << 14;
                ar |= (self.tr.cache.u.segment_g() as u32) << 15;
                if self.tr.cache.valid == 0 { ar |= 1 << 16; }
                ar
            };

            // GDTR / IDTR.
            self.vmcs.guest_gdtr_base = self.gdtr.base;
            self.vmcs.guest_gdtr_limit = self.gdtr.limit as u32;
            self.vmcs.guest_idtr_base = self.idtr.base;
            self.vmcs.guest_idtr_limit = self.idtr.limit as u32;

            // Sysenter MSRs (always saved — Bochs).
            self.vmcs.guest_ia32_sysenter_cs = self.msr.sysenter_cs_msr;
            self.vmcs.guest_ia32_sysenter_esp = self.msr.sysenter_esp_msr;
            self.vmcs.guest_ia32_sysenter_eip = self.msr.sysenter_eip_msr;

            // Activity state — Bochs vmx.cc maps MWAIT-family states
            // back to ACTIVE on VMEXIT (the architecturally-defined
            // VMX-visible activity-state set is {Active, Hlt, Shutdown,
            // WaitForSipi}).
            self.vmcs.guest_activity_state = match self.activity_state {
                super::cpu::CpuActivityState::Active
                | super::cpu::CpuActivityState::Mwait
                | super::cpu::CpuActivityState::MwaitIf => 0,
                super::cpu::CpuActivityState::Hlt => 1,
                super::cpu::CpuActivityState::Shutdown => 2,
                super::cpu::CpuActivityState::WaitForSipi
                | super::cpu::CpuActivityState::VmxLastActivityState => 3,
            };

            // Interruptibility state — synthesise from inhibit_mask
            // and NMI-blocked event bits, mirroring Bochs vmx.cc:2781-
            // 2803.
            let mut interruptibility = 0u32;
            if self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS) {
                if self.interrupts_inhibited(Self::BX_INHIBIT_DEBUG) {
                    interruptibility |= 1 << 1; // BLOCKED_BY_MOV_SS
                } else {
                    interruptibility |= 1 << 0; // BLOCKED_BY_STI
                }
            }
            if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_VIRTUAL_NMI != 0 {
                if (self.event_mask & Self::BX_EVENT_VMX_VIRTUAL_NMI) != 0 {
                    interruptibility |= 1 << 3;
                }
            } else if (self.event_mask & Self::BX_EVENT_NMI) != 0 {
                interruptibility |= 1 << 3;
            }
            self.vmcs.guest_interruptibility_state = interruptibility;

            // Pending #DB exceptions — Bochs vmx.cc:2747-2772.
            let trap_like = reason.is_trap_like();
            let clear_dbg = !self.interrupts_inhibited(Self::BX_INHIBIT_DEBUG)
                && !trap_like
                && !matches!(
                    reason,
                    VmxVmexitReason::Init
                        | VmxVmexitReason::Smi
                        | VmxVmexitReason::MonitorTrapFlag
                );
            self.vmcs.guest_pending_dbg_exceptions = if clear_dbg {
                0
            } else {
                let mut tmp = u64::from(self.debug_trap) & 0x0000_400F;
                if tmp & 0xF != 0 {
                    tmp |= 1 << 12;
                }
                tmp
            };

            let store_cnt = self.vmcs.vmexit_msr_store_cnt;
            let store_addr = self.vmcs.vmexit_msr_store_addr;
            if store_cnt != 0 {
                match self.vmx_store_msrs(store_cnt, store_addr) {
                    Ok(0) => {}
                    Ok(failing) => {
                        tracing::error!(
                            "VMABORT: error saving guest MSR number {failing}"
                        );
                        return Err(self.vmx_abort(VmxAbortCode::SavingGuestMsrsFailure));
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        // Bochs vmx.cc VMexit: `in_vmx_guest = false` AFTER step 1 and
        // BEFORE the `clear_event` mask + host-state load.
        self.in_vmx_guest = false;
        // Clear ALL VMX-guest-only pending events — Bochs vmx.cc clears
        // the union of:
        //   VTPR_UPDATE | VEOI_UPDATE | VIRTUAL_APIC_WRITE |
        //   MONITOR_TRAP_FLAG | INTERRUPT_WINDOW_EXITING |
        //   PREEMPTION_TIMER_EXPIRED | VIRTUAL_NMI |
        //   PENDING_VMX_VIRTUAL_INTR
        self.clear_event(
            Self::BX_EVENT_VMX_VTPR_UPDATE
                | Self::BX_EVENT_VMX_VEOI_UPDATE
                | Self::BX_EVENT_VMX_VIRTUAL_APIC_WRITE
                | Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG
                | Self::BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING
                | Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED
                | Self::BX_EVENT_VMX_VIRTUAL_NMI
                | Self::BX_EVENT_PENDING_VMX_VIRTUAL_INTR,
        );

        // ── STEP 2: load host state ──────────────────────────────────
        // A failure here means the host VMCS is corrupt — propagate so
        // the surrounding cpu loop can raise the exception rather than
        // silently drop the error.
        self.vmexit_load_host_state()?;

        // ── STEP 3: walk the host MSR-LOAD list (Bochs vmx.cc) ───────
        let load_cnt = self.vmcs.vmexit_msr_load_cnt;
        let load_addr = self.vmcs.vmexit_msr_load_addr;
        if load_cnt != 0 {
            match self.vmx_load_msrs(load_cnt, load_addr) {
                Ok(0) => {}
                Ok(failing) => {
                    tracing::error!(
                        "VMABORT: error loading host MSR number {failing}"
                    );
                    return Err(self.vmx_abort(VmxAbortCode::LoadingHostMsrs));
                }
                Err(e) => return Err(e),
            }
        }

        // ── STEP 4: VMX-root bookkeeping ─────────────────────────────
        //   mask_event(BX_EVENT_INIT)             // disabled in VMX root
        //   if (reason == EXCEPTION_NMI && vec==2) mask_event(BX_EVENT_NMI)
        //   EXT = 0
        //   last_exception_type = BX_ET_NONE
        self.mask_event(Self::BX_EVENT_INIT);
        if reason == VmxVmexitReason::ExceptionNmi && vector == 2 {
            self.mask_event(Self::BX_EVENT_NMI);
        }
        self.ext = false;
        self.last_exception_type = -1; // BX_ET_NONE

        self.invalidate_prefetch_q();
        Ok(())
    }

    /// Bochs vmx.cc `BX_CPU_C::VMabort`. Architecturally fatal — the
    /// VMM's state has lost integrity. Bochs writes the abort code to
    /// the VMCS abort-indicator field, deactivates the preemption
    /// timer, then `shutdown()`s the CPU. We mirror the timer disarm
    /// and surface the abort as `CpuError::VmxAbort` so the cpu loop
    /// can terminate the emulated CPU — silently swallowing the error
    /// would mask a serious VMM bug.
    pub(super) fn vmx_abort(&mut self, code: VmxAbortCode) -> super::error::CpuError {
        self.lapic.deactivate_vmx_preemption_timer();
        tracing::error!("VMX abort (Bochs VMabort): code={code:?}");
        super::error::CpuError::VmxAbort { code }
    }

    /// VMENTRY-failure VMEXIT — Bochs `BX_CPU_C::VMexit` with bit 31 of
    /// the reason set. The architectural difference from a regular VMEXIT
    /// is narrow (Bochs vmx.cc VMexit gates only STEP 1 on the
    /// VMENTRY-failure reasons):
    ///   - STEP 1 (guest-state save + VMEXIT MSR-STORE list) is skipped
    ///     because the VMENTRY never loaded any guest state.
    ///   - STEP 2 (host-state load) and STEP 3 (host MSR-LOAD list) run
    ///     unconditionally, mirroring the regular VMEXIT path.
    ///   - The early `!in_vmx_guest` panic-guard at the top of Bochs
    ///     VMexit is suppressed (bit 31 set explicitly opts out).
    /// Bit 31 is OR'd into the recorded exit reason so the host can
    /// distinguish a VMENTRY failure from a normal exit. Used by VMLAUNCH
    /// / VMRESUME when guest-state checking or the VMENTRY MSR-load list
    /// rejects the entry.
    pub(super) fn vmx_vmexit_vmentry_failure(
        &mut self,
        reason: VmxVmexitReason,
        qualification: u64,
    ) -> Result<()> {
        // ── Bochs `BX_CPU_C::VMexit` STEP 0 ──────────────────────────
        // Disarm preemption timer (idempotent — we never armed it for
        // this VMENTRY since failure happens before step 6); record
        // reason + qualification with bit 31 set; write the
        // VMEXIT_INSTRUCTION_LENGTH field; clear nmi_unblocking_iret.
        self.vmexit_disarm_preemption_timer();
        self.vmcs.exit_reason = (reason as u32) | 0x8000_0000;
        self.vmcs.exit_qualification = qualification;
        self.vmcs.exit_instruction_length =
            ((self.rip().wrapping_sub(self.prev_rip)) & 0xF) as u32;
        self.nmi_unblocking_iret = false;

        // Bochs vmx.cc VMexit ordering (lines 3144-3154): in_vmx_guest
        // first, then unconditional clear of the VMX-guest-pending
        // events. The window-exit / MTF / preemption-timer events
        // signaled at the start of this VMENTRY would otherwise stay
        // armed and fire as phantom VMEXITs once the host re-enters.
        self.in_vmx_guest = false;
        self.clear_event(
            Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG
                | Self::BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING
                | Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED
                | Self::BX_EVENT_VMX_VIRTUAL_NMI,
        );

        // STEP 2: load host state. The host VMCS fields were validated
        // by vmenter_load_check_host_state earlier; even though the live
        // CPU is still the host (no guest swap happened), Bochs always
        // re-loads the host VMCS fields here for architectural
        // consistency.
        self.vmexit_load_host_state()?;

        // STEP 3: host MSR load list — Bochs vmx.cc VMexit walks
        // VMEXIT_MSR_LOAD unconditionally, including for VMENTRY-failure
        // reasons. A non-zero failing index triggers `VMabort` (Bochs
        // VMABORT_LOADING_HOST_MSRS).
        let load_cnt = self.vmcs.vmexit_msr_load_cnt;
        let load_addr = self.vmcs.vmexit_msr_load_addr;
        if load_cnt != 0 {
            match self.vmx_load_msrs(load_cnt, load_addr) {
                Ok(0) => {}
                Ok(failing) => {
                    tracing::error!(
                        "VMENTRY-failure VMEXIT: host MSR-load list rejected entry {failing}"
                    );
                    return Err(self.vmx_abort(VmxAbortCode::LoadingHostMsrs));
                }
                Err(e) => return Err(e),
            }
        }

        // Bochs vmx.cc VMexit step 4 (lines 3175-3181): mask INIT (VMX
        // root mode disables it), clear EXT and last_exception_type.
        // The NMI-mask branch from the regular path doesn't apply here —
        // VMENTRY-failure reasons are never EXCEPTION_NMI.
        self.mask_event(Self::BX_EVENT_INIT);
        self.ext = false;
        self.last_exception_type = -1; // BX_ET_NONE

        self.invalidate_prefetch_q();
        Ok(())
    }

    /// Snapshot the running CPU's host context into the VMCS so VMEXIT can
    /// later restore it — Bochs vmx.cc VMexit "host state" save split. Bochs
    /// keeps these fields in the VMCS so the host can VMWRITE custom values
    /// before VMLAUNCH; we mirror the live CPU at the VMENTRY boundary so any
    /// fields the host did not explicitly write inherit reasonable defaults.
    /// Validate VM-execution control fields — Bochs vmx.cc
    /// `vmenter_load_check_vm_controls`. Returns `Some(error)` if the
    /// VMENTRY must be aborted with VMfail; `None` if the controls are
    /// internally consistent enough to proceed.
    ///
    /// Implemented checks (matches Bochs error returns):
    ///   - CR3-target count ≤ 4.
    ///   - Page-aligned, in-physical-range MSR bitmap (when MSR_BITMAPS).
    ///   - Page-aligned, in-physical-range I/O bitmaps (when IO_BITMAPS).
    ///   - UNRESTRICTED_GUEST requires EPT_ENABLE.
    ///   - VPID_ENABLE requires non-zero VPID (VMCS_16BIT_CONTROL_VPID).
    ///   - VM-entry interruption info: when valid bit set, vector + type
    ///     fields must be sane (Bochs vmenter_inject_events preconditions).
    /// Bochs MSR-list address check (vmx.cc:929-955). Used for the
    /// VMEXIT-store / VMEXIT-load / VMENTRY-load lists. When `count`
    /// is non-zero the base address must be 16-byte aligned and
    /// `[addr, addr + count*16 - 1]` must lie inside the host physical
    /// address space. Returns `Some(VmentryInvalidVmControlField)` to
    /// be propagated by `?`.
    fn check_msr_list_addr(
        count: u32,
        addr: u64,
        what: &'static str,
    ) -> Option<VmxErr> {
        if count == 0 {
            return None;
        }
        if (addr & 0xF) != 0 || !is_valid_phy_addr(addr) {
            tracing::warn!("VMENTRY check: {what} addr {:#018x} malformed", addr);
            return Some(VmxErr::VmentryInvalidVmControlField);
        }
        let last = addr.wrapping_add(u64::from(count).saturating_mul(16)).wrapping_sub(1);
        if !is_valid_phy_addr(last) {
            tracing::warn!(
                "VMENTRY check: {what} count {count} pushes last byte beyond phys range"
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }
        None
    }

    /// Bochs control-mask test: each bit of `value` must be permitted by
    /// `allowed_1` and every required-1 bit (set in `allowed_0`) must be
    /// present. Returns true when the value violates either constraint.
    fn ctls_out_of_bounds(value: u32, allowed_0: u32, allowed_1: u32) -> bool {
        // Required-1 bits missing → fail.
        if !value & allowed_0 != 0 {
            return true;
        }
        // Bits set that aren't permitted → fail.
        if value & !allowed_1 != 0 {
            return true;
        }
        false
    }

    fn vmenter_load_check_vm_controls(&mut self) -> Option<VmxErr> {
        // Bochs vmx.cc:580-621 — every control field must respect its
        // IA32_VMX_*_CTLS allowed-0 / allowed-1 mask: bits cleared in
        // the value that are required by allowed-0 fail; bits set in
        // the value that aren't permitted by allowed-1 fail.
        let pin = self.vmcs.pin_based_ctls;
        if Self::ctls_out_of_bounds(pin, VMX_PINBASED_CTLS_ALLOWED_0, VMX_PINBASED_CTLS_ALLOWED_1) {
            tracing::warn!("VMENTRY check_vm_controls: pin-based controls out of bounds");
            return Some(VmxErr::VmentryInvalidVmControlField);
        }
        let proc1 = self.vmcs.proc_based_ctls;
        if Self::ctls_out_of_bounds(
            proc1,
            VMX_PROCBASED_CTLS_ALLOWED_0,
            VMX_PROCBASED_CTLS_ALLOWED_1,
        ) {
            tracing::warn!("VMENTRY check_vm_controls: primary proc-based controls out of bounds");
            return Some(VmxErr::VmentryInvalidVmControlField);
        }
        // Bochs vmx.cc:599-602: secondary controls only consulted when
        // ACTIVATE_SECONDARY_CONTROLS is set.
        if proc1 & VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS != 0 {
            let proc2 = self.vmcs.secondary_proc_based_ctls;
            if Self::ctls_out_of_bounds(
                proc2,
                VMX_PROCBASED_CTLS2_ALLOWED_0,
                VMX_PROCBASED_CTLS2_ALLOWED_1,
            ) {
                tracing::warn!(
                    "VMENTRY check_vm_controls: secondary proc-based controls out of bounds"
                );
                return Some(VmxErr::VmentryInvalidVmControlField);
            }
        }
        let exit_ctls = self.vmcs.vm_exit_ctls;
        if Self::ctls_out_of_bounds(exit_ctls, VMX_EXIT_CTLS_ALLOWED_0, VMX_EXIT_CTLS_ALLOWED_1) {
            tracing::warn!("VMENTRY check_vm_controls: VM-exit controls out of bounds");
            return Some(VmxErr::VmentryInvalidVmControlField);
        }
        let entry_ctls = self.vmcs.vm_entry_ctls;
        if Self::ctls_out_of_bounds(entry_ctls, VMX_ENTRY_CTLS_ALLOWED_0, VMX_ENTRY_CTLS_ALLOWED_1)
        {
            tracing::warn!("VMENTRY check_vm_controls: VM-entry controls out of bounds");
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        // CR3 target count.
        if self.vmcs.vm_cr3_target_cnt > 4 {
            tracing::warn!(
                "VMENTRY check_vm_controls: vm_cr3_target_cnt={} > 4",
                self.vmcs.vm_cr3_target_cnt
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        let ctls1 = self.proc_based_ctls1();
        let ctls2 = self.proc_based_ctls2();

        // Bitmap address validity. Bochs requires page-aligned and within
        // the physical address width.
        if ctls1 & VMX_VM_EXEC_CTRL1_MSR_BITMAPS != 0
            && !is_valid_page_aligned_phy_addr(self.vmcs.msr_bitmap_addr)
        {
            tracing::warn!(
                "VMENTRY check_vm_controls: bad msr_bitmap_addr={:#018x}",
                self.vmcs.msr_bitmap_addr
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }
        if ctls1 & VMX_VM_EXEC_CTRL1_IO_BITMAPS != 0 {
            for (i, addr) in self.vmcs.io_bitmap_addr.iter().enumerate() {
                if !is_valid_page_aligned_phy_addr(*addr) {
                    tracing::warn!(
                        "VMENTRY check_vm_controls: bad io_bitmap_addr[{i}]={:#018x}",
                        addr
                    );
                    return Some(VmxErr::VmentryInvalidVmControlField);
                }
            }
        }

        // Secondary controls only apply when ACTIVATE_SECONDARY_CONTROLS is
        // set — proc_based_ctls2() already returns 0 in that case so the
        // checks below are no-ops.
        let ept_enabled = ctls2 & VMX_VM_EXEC_CTRL2_EPT_ENABLE != 0;
        if ctls2 & VMX_VM_EXEC_CTRL2_UNRESTRICTED_GUEST != 0 && !ept_enabled {
            tracing::warn!(
                "VMENTRY check_vm_controls: UNRESTRICTED_GUEST without EPT"
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }
        if ctls2 & VMX_VM_EXEC_CTRL2_VPID_ENABLE != 0 && self.vmcs.vpid == 0 {
            tracing::warn!("VMENTRY check_vm_controls: VPID_ENABLE with VPID=0");
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        // Bochs vmx.cc:789-805: when EPT_ENABLE is set the EPTPTR must
        // pass is_eptptr_valid; when it's clear, UNRESTRICTED_GUEST and
        // MBE_CTRL are illegal (each requires EPT).
        if ept_enabled {
            if !self.is_eptptr_valid(self.vmcs.eptptr) {
                tracing::warn!(
                    "VMENTRY check_vm_controls: invalid EPTPTR={:#018x}",
                    self.vmcs.eptptr
                );
                return Some(VmxErr::VmentryInvalidVmControlField);
            }
        } else if ctls2 & VMX_VM_EXEC_CTRL2_MBE_CTRL != 0 {
            tracing::warn!("VMENTRY check_vm_controls: MBE_CTRL without EPT");
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        // Bochs vmx.cc:923-926: STORE_VMX_PREEMPTION_TIMER VMEXIT control
        // requires the pin-based VMX_PREEMPTION_TIMER_VMEXIT.
        if exit_ctls & VMX_VMEXIT_CTRL1_STORE_VMX_PREEMPTION_TIMER != 0
            && self.vmcs.pin_based_ctls
                & VMX_PIN_BASED_VMEXEC_CTRL_VMX_PREEMPTION_TIMER_VMEXIT
                == 0
        {
            tracing::warn!(
                "VMENTRY check_vm_controls: STORE_VMX_PREEMPTION_TIMER without pin-based timer"
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        // Bochs vmx.cc:929-955: VMEXIT MSR-store / -load areas — when
        // count > 0 the address must be 16-byte aligned, in physical
        // range, and `addr + count*16 - 1` must also be in range.
        // ? on Option propagates None, but here the success value IS
        // None — propagate failure (Some) explicitly.
        if let Some(err) = Self::check_msr_list_addr(
            self.vmcs.vmexit_msr_store_cnt,
            self.vmcs.vmexit_msr_store_addr,
            "VMEXIT msr-store",
        ) {
            return Some(err);
        }
        if let Some(err) = Self::check_msr_list_addr(
            self.vmcs.vmexit_msr_load_cnt,
            self.vmcs.vmexit_msr_load_addr,
            "VMEXIT msr-load",
        ) {
            return Some(err);
        }
        if let Some(err) = Self::check_msr_list_addr(
            self.vmcs.vmentry_msr_load_cnt,
            self.vmcs.vmentry_msr_load_addr,
            "VMENTRY msr-load",
        ) {
            return Some(err);
        }

        // Bochs vmx.cc:982-987: DEACTIVATE_DUAL_MONITOR_TREATMENT VM-entry
        // control requires the CPU to be in SMM.
        const VMX_VMENTRY_CTRL_DEACTIVATE_DUAL_MONITOR: u32 = 1 << 10;
        if entry_ctls & VMX_VMENTRY_CTRL_DEACTIVATE_DUAL_MONITOR != 0 && !self.in_smm {
            tracing::warn!(
                "VMENTRY check_vm_controls: DEACTIVATE_DUAL_MONITOR_TREATMENT outside SMM"
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        // Bochs vmx.cc:665-673 NMI/VIRTUAL_NMI consistency:
        //   - VIRTUAL_NMI requires NMI_EXITING.
        //   - NMI_WINDOW_EXITING requires VIRTUAL_NMI.
        let pin = self.vmcs.pin_based_ctls;
        if pin & VMX_PIN_BASED_VMEXEC_CTRL_VIRTUAL_NMI != 0
            && pin & VMX_PIN_BASED_VMEXEC_CTRL_NMI_EXITING == 0
        {
            tracing::warn!("VMENTRY check_vm_controls: VIRTUAL_NMI without NMI_EXITING");
            return Some(VmxErr::VmentryInvalidVmControlField);
        }
        if ctls1 & VMX_VM_EXEC_CTRL1_NMI_WINDOW_EXITING != 0
            && pin & VMX_PIN_BASED_VMEXEC_CTRL_VIRTUAL_NMI == 0
        {
            tracing::warn!(
                "VMENTRY check_vm_controls: NMI_WINDOW_EXITING without VIRTUAL_NMI"
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        // Bochs vmx.cc:699-761 TPR-shadow checks. The threshold's high-4-bit
        // bound (0..=15) is enforced here; the TPR-threshold VMEXIT itself is
        // raised by vmx_tpr_threshold_vmexit() from the CR8 write path.
        if ctls1 & VMX_VM_EXEC_CTRL1_TPR_SHADOW != 0 && self.vmcs.tpr_threshold > 15 {
            tracing::warn!(
                "VMENTRY check_vm_controls: TPR_THRESHOLD={} > 15",
                self.vmcs.tpr_threshold
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        // Bochs vapic.cc enables three secondary processor-based controls that
        // rusty_box has not ported: APIC-access page virtualisation, APIC
        // register virtualisation, and virtual-interrupt delivery. VMENTRY
        // rejects them rather than silently letting the guest run with
        // degraded semantics. The TPR-threshold VMEXIT path is wired (see
        // vmx_tpr_threshold_vmexit).
        const UNSUPPORTED_VAPIC_SECONDARY: u32 = VMX_VM_EXEC_CTRL2_VIRTUALIZE_APIC_ACCESSES
            | VMX_VM_EXEC_CTRL2_VIRTUALIZE_APIC_REGISTERS
            | VMX_VM_EXEC_CTRL2_VIRTUAL_INT_DELIVERY;
        if ctls2 & UNSUPPORTED_VAPIC_SECONDARY != 0 {
            tracing::error!(
                "VMENTRY check_vm_controls: unsupported virtualised-APIC secondary control(s) {:#x}",
                ctls2 & UNSUPPORTED_VAPIC_SECONDARY
            );
            return Some(VmxErr::VmentryInvalidVmControlField);
        }

        // VM-entry event injection field. Bochs validates the type/vector
        // pair and the instruction-length when the valid bit is set.
        let info = self.vmcs.vm_entry_intr_info;
        if info & (1u32 << 31) != 0 {
            let bochs_type = (info >> 8) & 0x7;
            let vector = info & 0xFF;
            // Reserved type bits 1 and (8..=10) are illegal.
            match bochs_type {
                0 | 2..=7 => {}
                _ => {
                    tracing::warn!(
                        "VMENTRY check_vm_controls: reserved injection type {bochs_type}"
                    );
                    return Some(VmxErr::VmentryInvalidVmControlField);
                }
            }
            // Software interrupt / privileged software interrupt /
            // software exception (types 4/5/6) require instr_length 1..15.
            if matches!(bochs_type, 4 | 5 | 6) {
                let len = self.vmcs.vm_entry_instruction_length;
                if !(1..=15).contains(&len) {
                    tracing::warn!(
                        "VMENTRY check_vm_controls: bad instr_length {len} for type {bochs_type}"
                    );
                    return Some(VmxErr::VmentryInvalidVmControlField);
                }
            }
            // NMI vector must be 2.
            if bochs_type == 2 && vector != 2 {
                tracing::warn!(
                    "VMENTRY check_vm_controls: NMI injection vector {vector} != 2"
                );
                return Some(VmxErr::VmentryInvalidVmControlField);
            }
            // Bochs vmx.cc VMenterInjectEvents: type 7 (BX_EVENT_OTHER) is
            // only valid as the MTF marker (vector=0). Any other vector at
            // type 7 is rejected at VMENTRY rather than reaching the
            // injection switch's panic arm.
            if bochs_type == 7 && vector != 0 {
                tracing::warn!(
                    "VMENTRY check_vm_controls: type=7 (event-other) requires vector=0, got {vector}"
                );
                return Some(VmxErr::VmentryInvalidVmControlField);
            }
        }

        None
    }

    /// Validate host-state fields — Bochs vmx.cc VMenterLoadCheckHostState
    /// (vmx.cc:1143-1435). Each failure surfaces with
    /// VMXERR_VMENTRY_INVALID_VM_HOST_STATE_FIELD.
    fn vmenter_load_check_host_state(&mut self) -> Option<VmxErr> {
        let exit_ctls = self.vmcs.vm_exit_ctls;
        let entry_ctls = self.vmcs.vm_entry_ctls;
        let x86_64_host = exit_ctls & VMX_VMEXIT_CTRL1_HOST_ADDR_SPACE_SIZE != 0;
        let x86_64_guest = entry_ctls & VMX_VMENTRY_CTRL_X86_64_GUEST != 0;

        // Bochs vmx.cc:1156-1169 address-space-size consistency.
        if self.long_mode() {
            if !x86_64_host {
                tracing::warn!(
                    "VMENTRY check_host_state: long-mode host without X86_64_HOST"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        } else if x86_64_host || x86_64_guest {
            tracing::warn!(
                "VMENTRY check_host_state: x86-64 host/guest control set in non-long mode"
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // CR0 / CR4 VMX-mandatory bits (Bochs vmx.cc:1175-1202).
        if !self.check_cr0_vmx(self.vmcs.host_cr0, false) {
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if !self.check_cr4_vmx(self.vmcs.host_cr4) {
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // CR3 must be a valid physical address (Bochs vmx.cc:1188).
        if !is_valid_phy_addr(self.vmcs.host_cr3) {
            tracing::warn!(
                "VMENTRY check_host_state: bad host CR3={:#018x}",
                self.vmcs.host_cr3
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // Segment selectors: TI must be clear and RPL must be 0 for all
        // six host segment registers (Bochs vmx.cc:1204-1210).
        let segs = [
            ("ES", self.vmcs.host_es_selector),
            ("CS", self.vmcs.host_cs_selector),
            ("SS", self.vmcs.host_ss_selector),
            ("DS", self.vmcs.host_ds_selector),
            ("FS", self.vmcs.host_fs_selector),
            ("GS", self.vmcs.host_gs_selector),
        ];
        for (name, sel) in segs {
            if sel & 7 != 0 {
                tracing::warn!(
                    "VMENTRY check_host_state: host {name} selector {:#06x} TI/RPL != 0",
                    sel
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }
        if self.vmcs.host_cs_selector == 0 {
            tracing::warn!("VMENTRY check_host_state: host CS selector is 0");
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if !x86_64_host && self.vmcs.host_ss_selector == 0 {
            tracing::warn!(
                "VMENTRY check_host_state: 32-bit host with SS selector 0"
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if self.vmcs.host_tr_selector == 0 || self.vmcs.host_tr_selector & 7 != 0 {
            tracing::warn!(
                "VMENTRY check_host_state: bad host TR selector {:#06x}",
                self.vmcs.host_tr_selector
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // Canonical checks for the natural-width host fields
        // (Bochs vmx.cc:1230-1276 — only meaningful in long mode).
        let canonical_fields = [
            ("TR base", self.vmcs.host_tr_base),
            ("FS base", self.vmcs.host_fs_base),
            ("GS base", self.vmcs.host_gs_base),
            ("GDTR base", self.vmcs.host_gdtr_base),
            ("IDTR base", self.vmcs.host_idtr_base),
            ("SYSENTER ESP", self.vmcs.host_sysenter_esp),
            ("SYSENTER EIP", self.vmcs.host_sysenter_eip),
        ];
        for (name, val) in canonical_fields {
            if !self.is_canonical(val) {
                tracing::warn!(
                    "VMENTRY check_host_state: host {name}={:#018x} not canonical",
                    val
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }

        // PAT validity (Bochs vmx.cc:1279-1284): each of the eight 8-bit
        // entries must encode a supported memory type (UC/WC/WT/WP/WB/UC-).
        if exit_ctls & VMX_VMEXIT_CTRL1_LOAD_PAT_MSR != 0
            && !is_valid_pat_msr(self.vmcs.host_ia32_pat)
        {
            tracing::warn!(
                "VMENTRY check_host_state: invalid host PAT={:#018x}",
                self.vmcs.host_ia32_pat
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        // SPEC_CTRL validity (Bochs vmx.cc:1287-1291): only documented
        // bits may be set. We accept the same masks Bochs documents.
        if self.vmcs.vm_exit_ctls2 & VMX_VMEXIT_CTRL2_LOAD_HOST_IA32_SPEC_CTRL != 0
            && !is_valid_spec_ctrl(self.vmcs.host_ia32_spec_ctrl)
        {
            tracing::warn!(
                "VMENTRY check_host_state: invalid host SPEC_CTRL={:#018x}",
                self.vmcs.host_ia32_spec_ctrl
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        // EFER bits, when LOAD_EFER_MSR is set, must match the
        // x86_64_host control (Bochs vmx.cc:1295-1308).
        if exit_ctls & VMX_VMEXIT_CTRL1_LOAD_EFER_MSR != 0 {
            use super::crregs::BxEfer;
            let efer = self.vmcs.host_ia32_efer;
            // Reserved bits must be clear.
            const EFER_RESERVED: u64 =
                !(BxEfer::SCE.bits() as u64
                    | BxEfer::LME.bits() as u64
                    | BxEfer::LMA.bits() as u64
                    | BxEfer::NXE.bits() as u64
                    | BxEfer::SVME.bits() as u64
                    | BxEfer::FFXSR.bits() as u64);
            if efer & EFER_RESERVED != 0 {
                tracing::warn!(
                    "VMENTRY check_host_state: host EFER {:#018x} has reserved bits",
                    efer
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            let lme = efer & BxEfer::LME.bits() as u64 != 0;
            let lma = efer & BxEfer::LMA.bits() as u64 != 0;
            if lma != x86_64_host || lme != x86_64_host {
                tracing::warn!(
                    "VMENTRY check_host_state: host EFER LME/LMA disagree with X86_64_HOST"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }
        None
    }

    /// Validate guest-state fields — Bochs vmx.cc
    /// `vmenter_load_check_guest_state` (vmx.cc:1436-2310).
    fn vmenter_load_check_guest_state(&mut self) -> Option<VmxErr> {
        let entry_ctls = self.vmcs.vm_entry_ctls;
        let ctls2 = self.vmcs.secondary_proc_based_ctls;
        let x86_64_guest = entry_ctls & VMX_VMENTRY_CTRL_X86_64_GUEST != 0;
        let unrestricted = ctls2 & VMX_VM_EXEC_CTRL2_UNRESTRICTED_GUEST != 0;

        // RFLAGS validation (Bochs vmx.cc:1449-1473).
        // Reserved bits [63:22], bit 15, bit 5, bit 3 must be zero;
        // bit 1 must be 1; VM=1 incompatible with x86_64_guest.
        const RFLAGS_RESERVED: u64 = 0xFFFF_FFFF_FFC0_8028;
        let rflags = self.vmcs.guest_rflags;
        if rflags & RFLAGS_RESERVED != 0 {
            tracing::warn!(
                "VMENTRY check_guest_state: RFLAGS {:#018x} reserved bits set",
                rflags
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if rflags & 0x2 == 0 {
            tracing::warn!("VMENTRY check_guest_state: RFLAGS[1] cleared");
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        const EFLAGS_VM_MASK: u64 = 1 << 17;
        let v8086_guest = rflags & EFLAGS_VM_MASK != 0;
        if x86_64_guest && v8086_guest {
            tracing::warn!(
                "VMENTRY check_guest_state: x86-64 guest with RFLAGS.VM=1"
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // CR0 (Bochs vmx.cc:1475-1503). Under UNRESTRICTED_GUEST the
        // CR0_FIXED0 mask is relaxed to drop PE+PG (the VMM may run a
        // real-mode guest); otherwise the standard fixed-0 mask applies.
        // We delegate the NE / PE / PG bit logic to check_cr0_vmx with
        // vmenter=true; an additional explicit `PG without PE` check
        // applies under UNRESTRICTED_GUEST.
        if !self.check_cr0_vmx(self.vmcs.guest_cr0, true) {
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if unrestricted {
            use super::crregs::BxCr0;
            let cr0 = BxCr0::from_bits_truncate(self.vmcs.guest_cr0 as u32);
            if cr0.contains(BxCr0::PG) && !cr0.contains(BxCr0::PE) {
                tracing::warn!(
                    "VMENTRY check_guest_state: UNRESTRICTED guest CR0.PG without CR0.PE"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }

        // CR3 must fit in physical address width (Bochs vmx.cc:1513).
        if !is_valid_phy_addr(self.vmcs.guest_cr3) {
            tracing::warn!(
                "VMENTRY check_guest_state: bad guest CR3={:#018x}",
                self.vmcs.guest_cr3
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // CR4 — VMX-mandatory + long-mode consistency (Bochs vmx.cc:1519-1548).
        if !self.check_cr4_vmx(self.vmcs.guest_cr4) {
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        use super::crregs::BxCr4;
        let cr4 = BxCr4::from_bits_truncate(self.vmcs.guest_cr4);
        if x86_64_guest {
            if !cr4.contains(BxCr4::PAE) {
                tracing::warn!(
                    "VMENTRY check_guest_state: x86-64 guest with CR4.PAE=0"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        } else if cr4.contains(BxCr4::PCIDE) {
            tracing::warn!(
                "VMENTRY check_guest_state: 32-bit guest with CR4.PCIDE=1"
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // DR7 — when LOAD_DBG_CTRLS is set the upper 32 bits must be zero
        // (Bochs vmx.cc:1550-1556).
        const VMX_VMENTRY_CTRL_LOAD_DBG_CTRLS: u32 = 1 << 2;
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_DBG_CTRLS != 0
            && (self.vmcs.guest_dr7 >> 32) != 0
        {
            tracing::warn!(
                "VMENTRY check_guest_state: bad guest DR7={:#018x}",
                self.vmcs.guest_dr7
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // CET interlock — CR4.CET requires CR0.WP (Bochs vmx.cc:1560-1563).
        use super::crregs::BxCr0;
        let cr0_bits = BxCr0::from_bits_truncate(self.vmcs.guest_cr0 as u32);
        if cr4.contains(BxCr4::CET) && !cr0_bits.contains(BxCr0::WP) {
            tracing::warn!(
                "VMENTRY check_guest_state: CR4.CET=1 with CR0.WP=0"
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // Long-mode-only canonical checks (Bochs vmx.cc:1670-1691). RSP
        // is naturally signed, RIP for x86-64 must be canonical too.
        if x86_64_guest {
            if !self.is_canonical(self.vmcs.guest_rip) {
                tracing::warn!(
                    "VMENTRY check_guest_state: guest RIP={:#018x} non-canonical",
                    self.vmcs.guest_rip
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        } else {
            // 32-bit guest: RIP must fit in 32 bits.
            if (self.vmcs.guest_rip >> 32) != 0 {
                tracing::warn!(
                    "VMENTRY check_guest_state: 32-bit guest RIP={:#018x} > 32 bits",
                    self.vmcs.guest_rip
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }

        // SYSENTER ESP / EIP canonical (Bochs vmx.cc — guest state
        // checks). guest_ia32_sysenter_* fields are populated via
        // VMWRITE before VMENTRY.
        let canonical = [
            ("SYSENTER ESP", self.vmcs.guest_ia32_sysenter_esp),
            ("SYSENTER EIP", self.vmcs.guest_ia32_sysenter_eip),
        ];
        for (name, val) in canonical {
            if !self.is_canonical(val) {
                tracing::warn!(
                    "VMENTRY check_guest_state: guest {name}={:#018x} non-canonical",
                    val
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }

        // Guest EFER under LOAD_GUEST_EFER must agree with X86_64_GUEST
        // in the LME/LMA bits and have only documented bits (Bochs
        // vmx.cc:1738-1771).
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_GUEST_EFER_MSR != 0 {
            use super::crregs::BxEfer;
            let efer = self.vmcs.guest_ia32_efer;
            const EFER_RESERVED: u64 =
                !(BxEfer::SCE.bits() as u64
                    | BxEfer::LME.bits() as u64
                    | BxEfer::LMA.bits() as u64
                    | BxEfer::NXE.bits() as u64
                    | BxEfer::SVME.bits() as u64
                    | BxEfer::FFXSR.bits() as u64);
            if efer & EFER_RESERVED != 0 {
                tracing::warn!(
                    "VMENTRY check_guest_state: guest EFER {:#018x} reserved bits set",
                    efer
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            let lme = efer & BxEfer::LME.bits() as u64 != 0;
            let lma = efer & BxEfer::LMA.bits() as u64 != 0;
            if lma != x86_64_guest {
                tracing::warn!(
                    "VMENTRY check_guest_state: guest EFER.LMA disagrees with X86_64_GUEST"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            // CR0.PG=1 + LME=1 demands LMA=1 (long mode active).
            if cr0_bits.contains(BxCr0::PG) && lme && !lma {
                tracing::warn!(
                    "VMENTRY check_guest_state: CR0.PG && EFER.LME without LMA"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }

        // Guest PAT — same memory-type validation Bochs applies for the
        // host PAT (Bochs vmx.cc).
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_GUEST_PAT_MSR != 0
            && !is_valid_pat_msr(self.vmcs.guest_ia32_pat)
        {
            tracing::warn!(
                "VMENTRY check_guest_state: invalid guest PAT={:#018x}",
                self.vmcs.guest_ia32_pat
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // Activity state must be one of the four documented values
        // (0=Active, 1=HLT, 2=Shutdown, 3=Wait-for-SIPI). Bochs
        // vmx.cc validates against a per-CPU allow-list; we accept the
        // SDM-documented set.
        if self.vmcs.guest_activity_state > 3 {
            tracing::warn!(
                "VMENTRY check_guest_state: invalid activity_state={}",
                self.vmcs.guest_activity_state
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // Interruptibility state: only bits 0..=4 are defined (Bochs
        // vmx.cc); reserved bits must be zero.
        if self.vmcs.guest_interruptibility_state & !0x1F != 0 {
            tracing::warn!(
                "VMENTRY check_guest_state: bad interruptibility_state={:#x}",
                self.vmcs.guest_interruptibility_state
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // Per-segment validation (Bochs vmx.cc:1632-1837). The order
        // matters because CS/SS DPL/RPL relations consult both.
        self.check_guest_segments(v8086_guest, x86_64_guest, unrestricted)?;

        // GDTR/IDTR — Bochs vmx.cc:1840-1857. Limit ≤ 0xFFFF and base
        // canonical (in long mode).
        if self.vmcs.guest_gdtr_limit > 0xFFFF {
            tracing::warn!(
                "VMENTRY check_guest_state: GDTR limit {:#x} > 0xFFFF",
                self.vmcs.guest_gdtr_limit
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if self.vmcs.guest_idtr_limit > 0xFFFF {
            tracing::warn!(
                "VMENTRY check_guest_state: IDTR limit {:#x} > 0xFFFF",
                self.vmcs.guest_idtr_limit
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if !self.is_canonical(self.vmcs.guest_gdtr_base)
            || !self.is_canonical(self.vmcs.guest_idtr_base)
        {
            tracing::warn!(
                "VMENTRY check_guest_state: GDTR/IDTR base non-canonical"
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }

        // LDTR — Bochs vmx.cc:1860-1899. Only checked when not unusable.
        let ldtr_ar = SegAr::from_bits_truncate(self.vmcs.guest_ldtr_ar);
        if !ldtr_ar.contains(SegAr::UNUSABLE) {
            // TI bit (bit 2 of selector) must be clear (must be in GDT).
            if self.vmcs.guest_ldtr_selector & 4 != 0 {
                tracing::warn!(
                    "VMENTRY check_guest_state: LDTR selector TI=1"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            if ldtr_ar.type_field() != BX_SEG_TYPE_LDT {
                tracing::warn!(
                    "VMENTRY check_guest_state: LDTR type {} != LDT",
                    ldtr_ar.type_field()
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            // S=0 (system descriptor) required.
            if ldtr_ar.contains(SegAr::S_BIT) {
                tracing::warn!(
                    "VMENTRY check_guest_state: LDTR is not a system segment"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            if !ldtr_ar.contains(SegAr::P_BIT) {
                tracing::warn!("VMENTRY check_guest_state: LDTR not present");
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            if !is_limit_access_rights_consistent(self.vmcs.guest_ldtr_limit, ldtr_ar)
            {
                tracing::warn!(
                    "VMENTRY check_guest_state: LDTR AR/limit malformed"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            if !self.is_canonical(self.vmcs.guest_ldtr_base) {
                tracing::warn!(
                    "VMENTRY check_guest_state: LDTR base non-canonical"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }

        // TR — Bochs vmx.cc:1905-1952. Always required (must be valid).
        let tr_ar = SegAr::from_bits_truncate(self.vmcs.guest_tr_ar);
        if !self.is_canonical(self.vmcs.guest_tr_base) {
            tracing::warn!("VMENTRY check_guest_state: TR base non-canonical");
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if tr_ar.contains(SegAr::UNUSABLE) {
            tracing::warn!("VMENTRY check_guest_state: TR unusable");
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if self.vmcs.guest_tr_selector & 4 != 0 {
            tracing::warn!("VMENTRY check_guest_state: TR selector TI=1");
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if tr_ar.contains(SegAr::S_BIT) {
            tracing::warn!(
                "VMENTRY check_guest_state: TR is not a system segment"
            );
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if !tr_ar.contains(SegAr::P_BIT) {
            tracing::warn!("VMENTRY check_guest_state: TR not present");
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        if !is_limit_access_rights_consistent(self.vmcs.guest_tr_limit, tr_ar) {
            tracing::warn!("VMENTRY check_guest_state: TR AR/limit malformed");
            return Some(VmxErr::VmentryInvalidVmHostStateField);
        }
        // Type: BUSY_386_TSS always allowed; BUSY_286_TSS only for non-
        // x86-64 guests.
        match tr_ar.type_field() {
            t if t == BX_SEG_TYPE_BUSY_386_TSS => {}
            t if t == BX_SEG_TYPE_BUSY_286_TSS && !x86_64_guest => {}
            t => {
                tracing::warn!(
                    "VMENTRY check_guest_state: TR type {} invalid (x86_64_guest={x86_64_guest})",
                    t
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }

        // VMCS link pointer — Bochs vmx.cc:2028-2049. When != BX_INVALID
        // _VMCSPTR (~0u64), must be page-aligned and have the matching
        // VMCS revision ID. Reading the revision ID requires a physical
        // memory access; mirror Bochs. We don't yet model VMCS_SHADOWING
        // so the shadow-bit branch reduces to the basic revision check.
        const BX_INVALID_VMCSPTR: u64 = !0u64;
        let linkptr = self.vmcs.vmcs_link_pointer;
        if linkptr != BX_INVALID_VMCSPTR {
            if !is_valid_page_aligned_phy_addr(linkptr) {
                tracing::warn!(
                    "VMENTRY check_guest_state: VMCS link pointer {:#018x} malformed",
                    linkptr
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            // Bochs reads the revision-ID dword at linkptr; we approximate
            // with the same dword read used during VMPTRLD.
            let revision = self.vmx_read_revision_id(linkptr);
            // BX_VMCS_SHADOW_BIT_MASK is bit 31 of the revision word —
            // unset when VMCS_SHADOWING is off (our model).
            if revision & 0x8000_0000 != 0 {
                tracing::warn!(
                    "VMENTRY check_guest_state: linked VMCS is a shadow VMCS but VMCS_SHADOWING off"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            // Bochs also checks the ID matches `vmcs_map->get_vmcs_revision_id()`;
            // our model uses 1 (Skylake-X-style fixed revision).
            if revision != 1 {
                tracing::warn!(
                    "VMENTRY check_guest_state: linked VMCS revision {revision} != 1"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
        }

        None
    }

    /// Per-segment guest validation — Bochs vmx.cc:1632-1837. Order:
    /// validate ES, CS, SS, DS, FS, GS individually, then enforce
    /// CS/SS DPL+RPL relations and unrestricted-guest exceptions.
    fn check_guest_segments(
        &mut self,
        v8086_guest: bool,
        x86_64_guest: bool,
        unrestricted: bool,
    ) -> Option<VmxErr> {
        // Snapshot per-segment fields so we don't borrow self mutably
        // while comparing CS/SS later.
        let segs: [(&'static str, u16, u64, u32, u32, BxSegregs); 6] = [
            ("ES", self.vmcs.guest_es_selector, self.vmcs.guest_es_base, self.vmcs.guest_es_limit, self.vmcs.guest_es_ar, BxSegregs::Es),
            ("CS", self.vmcs.guest_cs_selector, self.vmcs.guest_cs_base, self.vmcs.guest_cs_limit, self.vmcs.guest_cs_ar, BxSegregs::Cs),
            ("SS", self.vmcs.guest_ss_selector, self.vmcs.guest_ss_base, self.vmcs.guest_ss_limit, self.vmcs.guest_ss_ar, BxSegregs::Ss),
            ("DS", self.vmcs.guest_ds_selector, self.vmcs.guest_ds_base, self.vmcs.guest_ds_limit, self.vmcs.guest_ds_ar, BxSegregs::Ds),
            ("FS", self.vmcs.guest_fs_selector, self.vmcs.guest_fs_base, self.vmcs.guest_fs_limit, self.vmcs.guest_fs_ar, BxSegregs::Fs),
            ("GS", self.vmcs.guest_gs_selector, self.vmcs.guest_gs_base, self.vmcs.guest_gs_limit, self.vmcs.guest_gs_ar, BxSegregs::Gs),
        ];

        let cs_ar = SegAr::from_bits_truncate(self.vmcs.guest_cs_ar);
        let ss_ar = SegAr::from_bits_truncate(self.vmcs.guest_ss_ar);
        let cs_l = cs_ar.contains(SegAr::L_BIT);

        for (name, selector, base, limit, ar_raw, seg) in segs {
            let ar = SegAr::from_bits_truncate(ar_raw);
            let invalid = ar.contains(SegAr::UNUSABLE);

            // v8086 mode (Bochs vmx.cc:1647-1664): all six segments must
            // have base = (selector << 4), limit = 0xFFFF, AR = 0xF3.
            if v8086_guest {
                if base != (u64::from(selector) << 4) {
                    tracing::warn!(
                        "VMENTRY check_guest_state: v8086 {name}.base != selector<<4"
                    );
                    return Some(VmxErr::VmentryInvalidVmHostStateField);
                }
                if limit != 0xFFFF {
                    tracing::warn!(
                        "VMENTRY check_guest_state: v8086 {name}.limit != 0xFFFF"
                    );
                    return Some(VmxErr::VmentryInvalidVmHostStateField);
                }
                if ar_raw != 0xF3 {
                    tracing::warn!(
                        "VMENTRY check_guest_state: v8086 {name}.ar {:#x} != 0xF3",
                        ar_raw
                    );
                    return Some(VmxErr::VmentryInvalidVmHostStateField);
                }
                continue;
            }

            // FS/GS base canonical (long-mode-style — bases for these
            // are loaded in long mode regardless of guest mode).
            if matches!(seg, BxSegregs::Fs | BxSegregs::Gs) && !self.is_canonical(base) {
                tracing::warn!(
                    "VMENTRY check_guest_state: {name}.base {:#018x} non-canonical",
                    base
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }

            // Unusable segments (other than CS) skip the rest.
            if !matches!(seg, BxSegregs::Cs) && invalid {
                continue;
            }

            // SS=NULL allowed in 64-bit guest mode when CS.L=1 (long-
            // mode kernel transition trick).
            if matches!(seg, BxSegregs::Ss)
                && (selector & 3) == 0
                && x86_64_guest
                && cs_l
            {
                continue;
            }

            // ES/CS/SS/DS bases must fit in 32 bits (Bochs vmx.cc:1685-1690).
            if matches!(seg, BxSegregs::Es | BxSegregs::Cs | BxSegregs::Ss | BxSegregs::Ds)
                && (base >> 32) != 0
            {
                tracing::warn!(
                    "VMENTRY check_guest_state: {name}.base {:#018x} > 32 bits",
                    base
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }

            // S=1 (segment, not system).
            if !ar.contains(SegAr::S_BIT) {
                tracing::warn!("VMENTRY check_guest_state: {name} not segment");
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            if !ar.contains(SegAr::P_BIT) {
                tracing::warn!("VMENTRY check_guest_state: {name} not present");
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }
            if !is_limit_access_rights_consistent(limit, ar) {
                tracing::warn!(
                    "VMENTRY check_guest_state: {name} AR/limit malformed"
                );
                return Some(VmxErr::VmentryInvalidVmHostStateField);
            }

            let ty = ar.type_field();
            match seg {
                BxSegregs::Cs => {
                    let allowed_code = matches!(
                        ty,
                        t if t == BX_SEG_TYPE_CODE_EXEC_ONLY_ACCESSED
                            || t == BX_SEG_TYPE_CODE_EXEC_READ_ACCESSED
                            || t == BX_SEG_TYPE_CODE_EXEC_ONLY_CONF_ACCESSED
                            || t == BX_SEG_TYPE_CODE_EXEC_READ_CONF_ACCESSED
                    );
                    if !allowed_code {
                        // UNRESTRICTED_GUEST permits CS as a R/W data
                        // segment (real-mode entry) with DPL=0.
                        if unrestricted && ty == BX_SEG_TYPE_DATA_RW_ACCESSED {
                            if ar.dpl() != 0 {
                                tracing::warn!(
                                    "VMENTRY check_guest_state: unrestricted CS.DPL != 0"
                                );
                                return Some(VmxErr::VmentryInvalidVmHostStateField);
                            }
                        } else {
                            tracing::warn!(
                                "VMENTRY check_guest_state: CS.type {} invalid",
                                ty
                            );
                            return Some(VmxErr::VmentryInvalidVmHostStateField);
                        }
                    }
                    if x86_64_guest && cs_ar.contains(SegAr::DB_BIT) && cs_ar.contains(SegAr::L_BIT)
                    {
                        tracing::warn!(
                            "VMENTRY check_guest_state: x86-64 CS.D_B and CS.L both set"
                        );
                        return Some(VmxErr::VmentryInvalidVmHostStateField);
                    }
                }
                BxSegregs::Ss => {
                    let valid = ty == BX_SEG_TYPE_DATA_RW_ACCESSED
                        || ty == BX_SEG_TYPE_DATA_RW_EXP_DOWN_ACCESSED;
                    if !valid {
                        tracing::warn!("VMENTRY check_guest_state: SS.type {} invalid", ty);
                        return Some(VmxErr::VmentryInvalidVmHostStateField);
                    }
                }
                _ => {
                    // DS / ES / FS / GS: accessed bit must be set (low
                    // bit of type), and code segments must be readable
                    // (bit 1 of type).
                    if ty & 0x1 == 0 {
                        tracing::warn!(
                            "VMENTRY check_guest_state: {name} not ACCESSED"
                        );
                        return Some(VmxErr::VmentryInvalidVmHostStateField);
                    }
                    if ty & 0x8 != 0 && ty & 0x2 == 0 {
                        tracing::warn!(
                            "VMENTRY check_guest_state: {name} CODE not READABLE"
                        );
                        return Some(VmxErr::VmentryInvalidVmHostStateField);
                    }
                    // Conforming-vs-non-conforming RPL/DPL relation,
                    // skipped under UNRESTRICTED_GUEST.
                    if !unrestricted && ty <= 11 {
                        let rpl = (selector & 3) as u8;
                        if rpl > ar.dpl() {
                            tracing::warn!(
                                "VMENTRY check_guest_state: non-conforming {name}.RPL > DPL"
                            );
                            return Some(VmxErr::VmentryInvalidVmHostStateField);
                        }
                    }
                }
            }
        }

        // Cross-segment CS/SS DPL relations (Bochs vmx.cc:1799-1814).
        let cs_ty = cs_ar.type_field();
        let ss_dpl = ss_ar.dpl();
        let cs_dpl = cs_ar.dpl();
        match cs_ty {
            t if t == BX_SEG_TYPE_CODE_EXEC_ONLY_ACCESSED
                || t == BX_SEG_TYPE_CODE_EXEC_READ_ACCESSED =>
            {
                // Non-conforming code: CS.DPL must equal SS.DPL.
                if cs_dpl != ss_dpl {
                    tracing::warn!(
                        "VMENTRY check_guest_state: non-conforming CS.DPL != SS.DPL"
                    );
                    return Some(VmxErr::VmentryInvalidVmHostStateField);
                }
            }
            t if t == BX_SEG_TYPE_CODE_EXEC_ONLY_CONF_ACCESSED
                || t == BX_SEG_TYPE_CODE_EXEC_READ_CONF_ACCESSED =>
            {
                // Conforming code: CS.DPL ≤ SS.DPL.
                if cs_dpl > ss_dpl {
                    tracing::warn!(
                        "VMENTRY check_guest_state: conforming CS.DPL > SS.DPL"
                    );
                    return Some(VmxErr::VmentryInvalidVmHostStateField);
                }
            }
            _ => {}
        }

        // RPL relations between CS and SS (Bochs vmx.cc:1816-1836).
        if !v8086_guest {
            let cs_rpl = (self.vmcs.guest_cs_selector & 3) as u8;
            let ss_rpl = (self.vmcs.guest_ss_selector & 3) as u8;
            if !unrestricted {
                if ss_rpl != cs_rpl {
                    tracing::warn!(
                        "VMENTRY check_guest_state: SS.RPL != CS.RPL"
                    );
                    return Some(VmxErr::VmentryInvalidVmHostStateField);
                }
                if ss_rpl != ss_dpl {
                    tracing::warn!(
                        "VMENTRY check_guest_state: SS.RPL != SS.DPL"
                    );
                    return Some(VmxErr::VmentryInvalidVmHostStateField);
                }
            } else {
                // Unrestricted guest: in real-mode-like CS or actual
                // real mode, SS.DPL must be 0.
                use super::crregs::BxCr0;
                let cr0 = BxCr0::from_bits_truncate(self.vmcs.guest_cr0 as u32);
                let real_mode_guest = !cr0.contains(BxCr0::PE);
                if (real_mode_guest || cs_ty == BX_SEG_TYPE_DATA_RW_ACCESSED)
                    && ss_dpl != 0
                {
                    tracing::warn!(
                        "VMENTRY check_guest_state: unrestricted SS.DPL != 0"
                    );
                    return Some(VmxErr::VmentryInvalidVmHostStateField);
                }
            }
        }

        None
    }

    fn vmenter_save_host_state(&mut self) {
        self.vmcs.host_cr0 = u64::from(self.cr0.get32());
        self.vmcs.host_cr3 = self.cr3;
        self.vmcs.host_cr4 = self.cr4.get();
        self.vmcs.host_rsp = self.rsp();
        self.vmcs.host_rip = self.rip();
        self.vmcs.host_ia32_efer = u64::from(self.efer.get32());
        self.vmcs.host_ia32_pat = self.msr.pat.U64();

        // Segment selectors (Bochs vmx.cc VMexitSaveHostState).
        self.vmcs.host_es_selector = self.sregs[BxSegregs::Es as usize].selector.value;
        self.vmcs.host_cs_selector = self.sregs[BxSegregs::Cs as usize].selector.value;
        self.vmcs.host_ss_selector = self.sregs[BxSegregs::Ss as usize].selector.value;
        self.vmcs.host_ds_selector = self.sregs[BxSegregs::Ds as usize].selector.value;
        self.vmcs.host_fs_selector = self.sregs[BxSegregs::Fs as usize].selector.value;
        self.vmcs.host_gs_selector = self.sregs[BxSegregs::Gs as usize].selector.value;
        self.vmcs.host_tr_selector = self.tr.selector.value;
        self.vmcs.host_fs_base = self.sregs[BxSegregs::Fs as usize].cache.u.segment_base();
        self.vmcs.host_gs_base = self.sregs[BxSegregs::Gs as usize].cache.u.segment_base();
        self.vmcs.host_tr_base = self.tr.cache.u.segment_base();

        self.vmcs.host_gdtr_base = self.gdtr.base;
        self.vmcs.host_idtr_base = self.idtr.base;

        self.vmcs.host_sysenter_cs = self.msr.sysenter_cs_msr;
        self.vmcs.host_sysenter_esp = self.msr.sysenter_esp_msr;
        self.vmcs.host_sysenter_eip = self.msr.sysenter_eip_msr;

        // Optional host MSR / CET / FRED / PKRS / SPEC_CTRL state. Bochs
        // saves these unconditionally so VMEXIT can restore them when the
        // matching LOAD_HOST_* bits are set; we mirror the snapshot here
        // so the host state stays self-consistent across the VMENTRY/EXIT
        // round trip.
        self.vmcs.host_perf_global_ctrl = 0; // PMU not modelled in rusty_box
        self.vmcs.host_pkrs = u64::from(self.pkrs);
        self.vmcs.host_ia32_spec_ctrl = u64::from(self.msr.ia32_spec_ctrl);
        self.vmcs.host_ia32_s_cet = self.msr.ia32_cet_control[0];
        self.vmcs.host_ssp = self.msr.ia32_pl_ssp[3];
        self.vmcs.host_interrupt_ssp_table_addr = self.msr.ia32_interrupt_ssp_table;
        self.vmcs.host_fred_config = self.msr.ia32_fred_cfg;
        for i in 0..4 {
            self.vmcs.host_fred_rsp[i] = self.msr.ia32_fred_rsp[i];
            self.vmcs.host_fred_ssp[i] = self.msr.ia32_fred_ssp[i];
        }
        self.vmcs.host_fred_stack_levels = self.msr.ia32_fred_stack_levels;
    }

    /// Restore host state on VMEXIT — Bochs vmx.cc VMexitLoadHostState.
    /// Restores all CR/segment/MSR/DR state listed in SDM Vol. 3C §28.5.
    /// Bochs uses raw `fetch_raw_descriptor` + `parse_descriptor` (no full
    /// validation) because host state was already vetted at VMENTRY; we
    /// mirror that. Optional MSR/CET/FRED/PKRS/SPEC_CTRL/PERF_GLOBAL_CTRL
    /// areas are gated on the matching `LOAD_HOST_*` exit-control bits —
    /// when the bit is clear the host inherits the guest value, per the
    /// SDM. PMU host-state isn't modelled, so PERF_GLOBAL_CTRL just logs.
    fn vmexit_load_host_state(&mut self) -> Result<()> {
        let exit_ctls = self.vmcs.vm_exit_ctls;
        let x86_64_host = exit_ctls & VMX_VMEXIT_CTRL1_HOST_ADDR_SPACE_SIZE != 0;

        // Bochs vmx.cc VMexitLoadHostState: VMABORT when a 64-bit guest
        // exits to a host configured as 32-bit (the host can't continue
        // safely after coming out of long mode mid-flight).
        if self.long64_mode() && !x86_64_host {
            tracing::error!(
                "VMABORT: VMEXIT to 32-bit host from 64-bit guest"
            );
            // Bochs panics with VMABORT_VMEXIT_TO_32BIT_HOST_FROM_64BIT_GUEST.
            // Without a panic-on-abort path we surface a #UD into the host
            // so the failure is visible.
            self.in_vmx_guest = false;
            self.invalidate_prefetch_q();
            return self.exception(Exception::Ud, 0);
        }

        // EFER must be set BEFORE CR4/CR0 because long-mode bits influence
        // paging-mode validation downstream (Bochs comment).
        if exit_ctls & VMX_VMEXIT_CTRL1_LOAD_EFER_MSR != 0 {
            self.efer.set32(self.vmcs.host_ia32_efer as u32);
        } else {
            // Bochs vmx.cc:2858-2861 fallback: when LOAD_EFER_MSR is clear,
            // EFER.LME and EFER.LMA track x86_64_host directly.
            use super::crregs::BxEfer;
            let mut efer = self.efer.bits();
            if x86_64_host {
                efer |= (BxEfer::LME | BxEfer::LMA).bits();
            } else {
                efer &= !(BxEfer::LME | BxEfer::LMA).bits();
            }
            self.efer = BxEfer::from_bits_truncate(efer);
        }

        self.cr0.set32(self.vmcs.host_cr0 as u32);
        self.cr3 = self.vmcs.host_cr3;
        self.cr4.set_val(self.vmcs.host_cr4);
        self.set_rsp(self.vmcs.host_rsp);
        self.set_rip(self.vmcs.host_rip);

        // Bochs vmx.cc:2879-2883: 32-bit PAE host requires PDPTR
        // validation. A failure aborts the VMEXIT (Bochs VMABORT_HOST_
        // PDPTR_CORRUPTED). We surface #UD into the host so the failure
        // is visible.
        if !x86_64_host
            && super::crregs::BxCr4::from_bits_truncate(self.vmcs.host_cr4)
                .contains(super::crregs::BxCr4::PAE)
            && !self.check_pdptrs(self.vmcs.host_cr3)?
        {
            tracing::error!(
                "VMABORT: host PDPTRs corrupted (cr3={:#018x})",
                self.vmcs.host_cr3
            );
            self.in_vmx_guest = false;
            self.invalidate_prefetch_q();
            return self.exception(Exception::Ud, 0);
        }

        // Optional MSR loads — Bochs vmx.cc VMexitLoadHostState gates each
        // on the corresponding LOAD_HOST_* bit; with the bit clear the
        // host inherits the guest's value (per SDM 28.5).
        if exit_ctls & VMX_VMEXIT_CTRL1_LOAD_PAT_MSR != 0 {
            self.msr.pat.set_U64(self.vmcs.host_ia32_pat);
        }
        if exit_ctls & VMX_VMEXIT_CTRL1_LOAD_PERF_GLOBAL_CTRL_MSR != 0 {
            // PMU not modelled; the host VMCS field is preserved so the
            // VMM still observes the value it wrote.
            tracing::trace!(
                "VMEXIT LOAD_PERF_GLOBAL_CTRL_MSR={:#018x} (PMU not modelled)",
                self.vmcs.host_perf_global_ctrl
            );
        }
        if exit_ctls & VMX_VMEXIT_CTRL1_LOAD_HOST_PKRS != 0 {
            // Bochs crregs.cc set_PKeys recomputes allow masks from PKRU
            // and PKRS together.
            self.set_pkeys(self.pkru, self.vmcs.host_pkrs as u32);
        }
        if exit_ctls & VMX_VMEXIT_CTRL1_LOAD_HOST_CET_STATE != 0 {
            // Bochs vmx.cc loads supervisor-CET MSR + SSP + interrupt SSP
            // table; user-CET stays at its current value.
            self.msr.ia32_cet_control[0] = self.vmcs.host_ia32_s_cet;
            self.msr.ia32_pl_ssp[3] = self.vmcs.host_ssp;
            self.msr.ia32_interrupt_ssp_table = self.vmcs.host_interrupt_ssp_table_addr;
        }
        let exit_ctls2 = self.vmcs.vm_exit_ctls2;
        if exit_ctls2 & VMX_VMEXIT_CTRL2_LOAD_HOST_FRED != 0 {
            self.msr.ia32_fred_cfg = self.vmcs.host_fred_config;
            for i in 1..4 {
                self.msr.ia32_fred_rsp[i] = self.vmcs.host_fred_rsp[i];
                self.msr.ia32_fred_ssp[i] = self.vmcs.host_fred_ssp[i];
            }
            self.msr.ia32_fred_stack_levels = self.vmcs.host_fred_stack_levels;
        }
        if exit_ctls2 & VMX_VMEXIT_CTRL2_LOAD_HOST_IA32_SPEC_CTRL != 0 {
            self.msr.ia32_spec_ctrl = self.vmcs.host_ia32_spec_ctrl as u32;
        }

        // Segment selectors. Bochs LoadHostSeg sequence: parse selector →
        // fetch raw descriptor → parse → write into segment cache.
        // GDTR/IDTR must be restored first so the descriptor fetches use
        // the host's tables.
        self.gdtr.base = self.vmcs.host_gdtr_base;
        self.idtr.base = self.vmcs.host_idtr_base;
        self.idtr.limit = 0xFFFF;

        self.vmexit_load_host_seg(BxSegregs::Es, self.vmcs.host_es_selector)?;
        self.vmexit_load_host_seg(BxSegregs::Cs, self.vmcs.host_cs_selector)?;
        self.vmexit_load_host_seg(BxSegregs::Ss, self.vmcs.host_ss_selector)?;
        self.vmexit_load_host_seg(BxSegregs::Ds, self.vmcs.host_ds_selector)?;
        self.vmexit_load_host_seg(BxSegregs::Fs, self.vmcs.host_fs_selector)?;
        self.vmexit_load_host_seg(BxSegregs::Gs, self.vmcs.host_gs_selector)?;
        // FS / GS base override — Bochs vmx.cc gates this on
        // `x86_64_host || segreg.cache.valid`. In 32-bit hosts a null
        // selector leaves the descriptor cache unusable; overriding the
        // base would corrupt the segment record.
        if x86_64_host || self.sregs[BxSegregs::Fs as usize].cache.valid != 0 {
            self.sregs[BxSegregs::Fs as usize]
                .cache
                .u
                .set_segment_base(self.vmcs.host_fs_base);
        }
        if x86_64_host || self.sregs[BxSegregs::Gs as usize].cache.valid != 0 {
            self.sregs[BxSegregs::Gs as usize]
                .cache
                .u
                .set_segment_base(self.vmcs.host_gs_base);
        }

        // TR + LDTR. Bochs marks LDTR unusable (valid=0) and parses TR from
        // the host GDT, then overrides TR.base from the VMCS field.
        super::segment_ctrl_pro::parse_selector(
            self.vmcs.host_tr_selector,
            &mut self.tr.selector,
        );
        let (d1, d2) = self.fetch_raw_descriptor(&self.tr.selector.clone())?;
        self.tr.cache = self.parse_descriptor(d1, d2)?;
        self.tr.cache.u.set_segment_base(self.vmcs.host_tr_base);
        self.tr.cache.valid = 1;
        self.ldtr.cache.valid = 0;

        self.msr.sysenter_cs_msr = self.vmcs.host_sysenter_cs;
        self.msr.sysenter_esp_msr = self.vmcs.host_sysenter_esp;
        self.msr.sysenter_eip_msr = self.vmcs.host_sysenter_eip;

        // Bochs VMexitLoadHostState: DR7 reset, RFLAGS to reserved-bit only,
        // debug/inhibit/activity reset, monitor disarmed. inhibit_mask
        // tracks STI / MOV-SS shadow + NMI block; the host comes back fresh.
        self.dr7 = super::crregs::BxDr7::from_bits_retain(0x400);
        self.write_eflags(0x2, 0x003F_FFFF);
        self.debug_trap = 0;
        self.inhibit_mask = 0;
        self.activity_state = super::cpu::CpuActivityState::Active;
        self.monitor.reset_monitor();
        Ok(())
    }

    /// Bochs LoadHostSeg helper inlined per segment register. A null
    /// selector marks the cache unusable (Bochs `valid = 0`); a non-null
    /// selector goes through fetch_raw_descriptor + parse_descriptor and
    /// the cache is populated directly.
    fn vmexit_load_host_seg(
        &mut self,
        seg: BxSegregs,
        raw_selector: u16,
    ) -> Result<()> {
        super::segment_ctrl_pro::parse_selector(
            raw_selector,
            &mut self.sregs[seg as usize].selector,
        );
        if (raw_selector & 0xFFFC) == 0 {
            self.sregs[seg as usize].cache.valid = 0;
            return Ok(());
        }
        let sel = self.sregs[seg as usize].selector.clone();
        let (d1, d2) = self.fetch_raw_descriptor(&sel)?;
        self.sregs[seg as usize].cache = self.parse_descriptor(d1, d2)?;
        self.sregs[seg as usize].cache.valid = 1;
        Ok(())
    }

    /// Load a single guest segment cache directly from the VMCS-supplied
    /// selector / base / limit / access-rights tuple. Mirrors Bochs
    /// segment_ctrl_pro.cc `set_segment_ar_data` — the cache is
    /// synthesised from the AR encoding rather than re-fetched through
    /// the GDT, because the VMCS stores the host's authoritative view of
    /// the segment.
    fn vmenter_load_guest_seg(
        &mut self,
        seg: BxSegregs,
        selector: u16,
        base: u64,
        limit: u32,
        ar: u32,
    ) -> Result<()> {
        super::segment_ctrl_pro::parse_selector(
            selector,
            &mut self.sregs[seg as usize].selector,
        );
        let unusable = (ar >> 16) & 1 != 0;
        let cache = &mut self.sregs[seg as usize].cache;
        cache.set_ar_byte((ar & 0xFF) as u8);
        cache.valid = if unusable { 0 } else { 1 };
        cache.u.set_segment_base(base);
        cache.u.set_segment_limit_scaled(limit);
        cache.u.set_segment_g((ar >> 15) & 1 != 0);
        cache.u.set_segment_d_b((ar >> 14) & 1 != 0);
        cache.u.set_segment_l((ar >> 13) & 1 != 0);
        cache.u.set_segment_avl((ar >> 12) & 1 != 0);
        Ok(())
    }

    /// Pack a populated segment cache back into the VMCS access-rights
    /// encoding. Mirrors Bochs vmx.cc VMexitSaveGuestState segment block.
    fn vmexit_pack_guest_seg_ar(seg: &super::descriptor::BxSegmentReg) -> u32 {
        let ar_byte = seg.cache.get_ar_byte() as u32;
        let mut ar = ar_byte & 0xFF;
        ar |= (seg.cache.u.segment_avl() as u32) << 12;
        ar |= (seg.cache.u.segment_l() as u32) << 13;
        ar |= (seg.cache.u.segment_d_b() as u32) << 14;
        ar |= (seg.cache.u.segment_g() as u32) << 15;
        if seg.cache.valid == 0 {
            ar |= 1 << 16;
        }
        ar
    }

    /// Load full guest state on VMENTRY. Bochs vmx.cc
    /// VMenterLoadCheckGuestState (load step). Validation already
    /// happened in `vmenter_load_check_guest_state`; here we only
    /// transcribe the validated values into running CPU state.
    fn vmenter_load_guest_state(&mut self) -> Result<()> {
        let entry_ctls = self.vmcs.vm_entry_ctls;

        // EFER first — long-mode bits gate downstream paging mode
        // checks (Bochs vmx.cc).
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_GUEST_EFER_MSR != 0 {
            self.efer.set32(self.vmcs.guest_ia32_efer as u32);
        }

        // CRs.
        self.cr0.set32(self.vmcs.guest_cr0 as u32);
        self.cr3 = self.vmcs.guest_cr3;
        self.cr4.set_val(self.vmcs.guest_cr4);

        // DR7 + IA32_DEBUGCTL only when LOAD_DBG_CTRLS is set.
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_DBG_CTRLS != 0 {
            // Bochs vmx.cc:2272 forces bits 15:14 clear, bit 10 set.
            self.dr7 = super::crregs::BxDr7::from_bits_retain(
                ((self.vmcs.guest_dr7 & !0xC000) | 0x400) as u32,
            );
        }

        // RIP / RSP / RFLAGS.
        self.set_rsp(self.vmcs.guest_rsp);
        self.set_rip(self.vmcs.guest_rip);
        self.prev_rip = self.vmcs.guest_rip;
        self.write_eflags(self.vmcs.guest_rflags as u32, 0x003F_FFFF);

        // PAT only when LOAD_PAT_MSR is set.
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_GUEST_PAT_MSR != 0 {
            self.msr.pat.set_U64(self.vmcs.guest_ia32_pat);
        }

        // SYSENTER MSRs are loaded unconditionally (Bochs vmx.cc).
        self.msr.sysenter_cs_msr = self.vmcs.guest_ia32_sysenter_cs;
        self.msr.sysenter_esp_msr = self.vmcs.guest_ia32_sysenter_esp;
        self.msr.sysenter_eip_msr = self.vmcs.guest_ia32_sysenter_eip;

        // Activity state (Bochs vmx.cc maps the 4 SDM values into the
        // CpuActivityState enum; unsupported codes fall back to Active).
        self.activity_state = match self.vmcs.guest_activity_state {
            0 => super::cpu::CpuActivityState::Active,
            1 => super::cpu::CpuActivityState::Hlt,
            2 => super::cpu::CpuActivityState::Shutdown,
            3 => super::cpu::CpuActivityState::WaitForSipi,
            other => {
                tracing::warn!("VMENTRY: unsupported guest activity_state {other}");
                super::cpu::CpuActivityState::Active
            }
        };

        // GDTR / IDTR.
        self.gdtr.base = self.vmcs.guest_gdtr_base;
        self.gdtr.limit = self.vmcs.guest_gdtr_limit as u16;
        self.idtr.base = self.vmcs.guest_idtr_base;
        self.idtr.limit = self.vmcs.guest_idtr_limit as u16;

        // Six data/code segments — loaded directly from VMCS without
        // a GDT walk because the VMCS already carries selector/base/
        // limit/AR per Bochs VMenterLoadCheckGuestState.
        self.vmenter_load_guest_seg(
            BxSegregs::Es,
            self.vmcs.guest_es_selector,
            self.vmcs.guest_es_base,
            self.vmcs.guest_es_limit,
            self.vmcs.guest_es_ar,
        )?;
        self.vmenter_load_guest_seg(
            BxSegregs::Cs,
            self.vmcs.guest_cs_selector,
            self.vmcs.guest_cs_base,
            self.vmcs.guest_cs_limit,
            self.vmcs.guest_cs_ar,
        )?;
        self.vmenter_load_guest_seg(
            BxSegregs::Ss,
            self.vmcs.guest_ss_selector,
            self.vmcs.guest_ss_base,
            self.vmcs.guest_ss_limit,
            self.vmcs.guest_ss_ar,
        )?;
        self.vmenter_load_guest_seg(
            BxSegregs::Ds,
            self.vmcs.guest_ds_selector,
            self.vmcs.guest_ds_base,
            self.vmcs.guest_ds_limit,
            self.vmcs.guest_ds_ar,
        )?;
        self.vmenter_load_guest_seg(
            BxSegregs::Fs,
            self.vmcs.guest_fs_selector,
            self.vmcs.guest_fs_base,
            self.vmcs.guest_fs_limit,
            self.vmcs.guest_fs_ar,
        )?;
        self.vmenter_load_guest_seg(
            BxSegregs::Gs,
            self.vmcs.guest_gs_selector,
            self.vmcs.guest_gs_base,
            self.vmcs.guest_gs_limit,
            self.vmcs.guest_gs_ar,
        )?;

        // LDTR — same direct-from-VMCS synthesis but writing to
        // self.ldtr; mirrors Bochs set_segment_ar_data on guest.ldtr.
        super::segment_ctrl_pro::parse_selector(
            self.vmcs.guest_ldtr_selector,
            &mut self.ldtr.selector,
        );
        let ldtr_ar = self.vmcs.guest_ldtr_ar;
        let ldtr_unusable = (ldtr_ar >> 16) & 1 != 0;
        self.ldtr.cache.set_ar_byte((ldtr_ar & 0xFF) as u8);
        self.ldtr.cache.valid = if ldtr_unusable { 0 } else { 1 };
        self.ldtr.cache.u.set_segment_base(self.vmcs.guest_ldtr_base);
        self.ldtr.cache.u.set_segment_limit_scaled(self.vmcs.guest_ldtr_limit);
        self.ldtr.cache.u.set_segment_g((ldtr_ar >> 15) & 1 != 0);
        self.ldtr.cache.u.set_segment_d_b((ldtr_ar >> 14) & 1 != 0);
        self.ldtr.cache.u.set_segment_l((ldtr_ar >> 13) & 1 != 0);
        self.ldtr.cache.u.set_segment_avl((ldtr_ar >> 12) & 1 != 0);

        // TR — always usable per VMX rules.
        super::segment_ctrl_pro::parse_selector(
            self.vmcs.guest_tr_selector,
            &mut self.tr.selector,
        );
        let tr_ar = self.vmcs.guest_tr_ar;
        self.tr.cache.set_ar_byte((tr_ar & 0xFF) as u8);
        self.tr.cache.valid = 1;
        self.tr.cache.u.set_segment_base(self.vmcs.guest_tr_base);
        self.tr.cache.u.set_segment_limit_scaled(self.vmcs.guest_tr_limit);
        self.tr.cache.u.set_segment_g((tr_ar >> 15) & 1 != 0);
        self.tr.cache.u.set_segment_d_b((tr_ar >> 14) & 1 != 0);
        self.tr.cache.u.set_segment_l((tr_ar >> 13) & 1 != 0);
        self.tr.cache.u.set_segment_avl((tr_ar >> 12) & 1 != 0);

        // CPL recompute — Bochs vmx.cc:2320 sets CPL = guest SS DPL.
        // rusty_box surfaces CPL via CS.selector.rpl (cs_rpl()), so
        // override it here AFTER the segment loads.
        let new_cpl = ((self.vmcs.guest_ss_ar >> 5) & 0x3) as u8;
        self.sregs[BxSegregs::Cs as usize].selector.rpl = new_cpl;
        self.sregs[BxSegregs::Ss as usize].selector.rpl = new_cpl;
        self.sregs[BxSegregs::Cs as usize].cache.dpl = new_cpl;
        self.sregs[BxSegregs::Ss as usize].cache.dpl = new_cpl;

        // Optional gated MSR loads — Bochs vmx.cc post-CR/segment
        // block.
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_GUEST_CET_STATE != 0 {
            self.msr.ia32_cet_control[0] = self.vmcs.guest_ia32_s_cet;
            self.set_ssp(self.vmcs.guest_ssp);
            self.msr.ia32_interrupt_ssp_table = self.vmcs.guest_interrupt_ssp_table_addr;
        }
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_GUEST_PKRS != 0 {
            self.set_pkeys(self.pkru, self.vmcs.guest_pkrs as u32);
        }
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_GUEST_IA32_SPEC_CTRL != 0 {
            self.msr.ia32_spec_ctrl = self.vmcs.guest_ia32_spec_ctrl as u32;
        }
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_GUEST_FRED != 0 {
            self.msr.ia32_fred_cfg = self.vmcs.guest_fred_config;
            for i in 1..4 {
                self.msr.ia32_fred_rsp[i] = self.vmcs.guest_fred_rsp[i];
                self.msr.ia32_fred_ssp[i] = self.vmcs.guest_fred_ssp[i];
            }
            self.msr.ia32_fred_stack_levels = self.vmcs.guest_fred_stack_levels;
        }

        // debug_trap from pending_dbg_exceptions — Bochs vmx.cc:
        // skipped when an event is being injected (the injection path
        // resets debug_trap to 0).
        if entry_ctls & VMX_VMENTRY_CTRL_LOAD_DBG_CTRLS != 0
            && (self.vmcs.vm_entry_intr_info & 0x8000_0000) == 0
        {
            let pending = self.vmcs.guest_pending_dbg_exceptions;
            self.debug_trap = if pending & (1 << 12) != 0 {
                (pending & 0x0000_400F) as u32
            } else {
                (pending & 0x0000_4000) as u32
            };
        }

        Ok(())
    }

    /// Save a single guest segment cache back into the VMCS — Bochs
    /// vmx.cc VMexitSaveGuestState segment block.
    fn vmexit_save_guest_seg(&mut self, seg: BxSegregs) {
        let s = &self.sregs[seg as usize];
        let sel = s.selector.value;
        let base: u64 = s.cache.u.segment_base();
        let limit: u32 = s.cache.u.segment_limit_scaled();
        let ar: u32 = Self::vmexit_pack_guest_seg_ar(s);
        match seg {
            BxSegregs::Es => {
                self.vmcs.guest_es_selector = sel;
                self.vmcs.guest_es_base = base;
                self.vmcs.guest_es_limit = limit;
                self.vmcs.guest_es_ar = ar;
            }
            BxSegregs::Cs => {
                self.vmcs.guest_cs_selector = sel;
                self.vmcs.guest_cs_base = base;
                self.vmcs.guest_cs_limit = limit;
                self.vmcs.guest_cs_ar = ar;
            }
            BxSegregs::Ss => {
                self.vmcs.guest_ss_selector = sel;
                self.vmcs.guest_ss_base = base;
                self.vmcs.guest_ss_limit = limit;
                self.vmcs.guest_ss_ar = ar;
            }
            BxSegregs::Ds => {
                self.vmcs.guest_ds_selector = sel;
                self.vmcs.guest_ds_base = base;
                self.vmcs.guest_ds_limit = limit;
                self.vmcs.guest_ds_ar = ar;
            }
            BxSegregs::Fs => {
                self.vmcs.guest_fs_selector = sel;
                self.vmcs.guest_fs_base = base;
                self.vmcs.guest_fs_limit = limit;
                self.vmcs.guest_fs_ar = ar;
            }
            BxSegregs::Gs => {
                self.vmcs.guest_gs_selector = sel;
                self.vmcs.guest_gs_base = base;
                self.vmcs.guest_gs_limit = limit;
                self.vmcs.guest_gs_ar = ar;
            }
            BxSegregs::Null => unreachable!("vmexit_save_guest_seg called with Null seg"),
        }
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
    pub(super) fn long_compat_mode(&self) -> bool {
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

    /// External-interrupt pin-based VMEXIT — Bochs vmexit.cc
    /// `VMexit_ExtInterrupt`. Called from the event-delivery path BEFORE the
    /// PIC/LAPIC is acknowledged. With `INTA_ON_VMEXIT` cleared, the
    /// interrupt stays pending in the controller; with it set we let the
    /// caller acknowledge first and route through `vmexit_check_event_intr`
    /// below so the vector ends up in `exit_intr_info`.
    ///
    /// Returns `Ok(true)` when the no-ack VMEXIT path was taken.
    pub(super) fn vmexit_check_ext_intr_no_ack(&mut self) -> Result<bool> {
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_EXTERNAL_INTERRUPT_VMEXIT == 0 {
            return Ok(false);
        }
        if self.vmcs.vm_exit_ctls & VMX_VMEXIT_CTRL1_INTA_ON_VMEXIT != 0 {
            // Defer to the post-ack path; caller will acknowledge then
            // vmexit_check_event_intr.
            return Ok(false);
        }
        // No INTA on exit: interruption-info field is invalid.
        self.vmcs.exit_intr_info = 0;
        self.vmx_vmexit(VmxVmexitReason::ExternalInterrupt, 0)?;
        Ok(true)
    }

    /// Pin-based NMI VMEXIT — Bochs vmexit.cc `VMexit_Event` with
    /// `type == BX_NMI`. Called immediately before delivering NMI to the
    /// guest. Records interruption info `vector=2 | type=NMI<<8 | valid<<31`.
    pub(super) fn vmexit_check_nmi(&mut self) -> Result<bool> {
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_NMI_EXITING == 0 {
            return Ok(false);
        }
        // intr_info: vector=2, type=NMI(2)<<8, valid bit 31.
        self.vmcs.exit_intr_info = 2 | (2 << 8) | (1u32 << 31);
        self.vmcs.exit_intr_error_code = 0;
        self.vmx_vmexit(VmxVmexitReason::ExceptionNmi, 0)?;
        Ok(true)
    }

    /// Read the 64-byte posted-interrupt descriptor at
    /// `self.vmcs.pi_desc_addr`. Bochs vapic.cc `VMX_Posted_Interrupt_Processing`
    /// reads the descriptor in two pieces (PIR + ON byte); we read the full
    /// 64-byte block in one shot for simplicity.
    ///
    /// Layout (Bochs vapic.cc comment):
    ///   bytes [0..32]   = Posted Interrupt Requests (PIR), one bit per vector
    ///   byte  [32]      = bit 0 = Outstanding Notification (PID.ON)
    ///   bytes [33..64]  = reserved / user available
    fn read_posted_interrupt_descriptor(&mut self) -> [u8; 64] {
        let mut buf = [0u8; 64];
        let paddr = self.vmcs.pi_desc_addr;
        for (i, slot) in buf.iter_mut().enumerate() {
            *slot = self.read_physical_byte(paddr + i as u64);
        }
        buf
    }

    /// Write a single byte to guest-physical memory. Mirrors Bochs
    /// `BX_CPU_C::write_physical_byte` used by vapic.cc to clear PID.ON via
    /// atomic RMW. Logs and continues on failure to match the lenient
    /// posture of `read_physical_byte`.
    fn write_physical_byte(&mut self, paddr: u64, val: u8) {
        let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() else {
            tracing::warn!("write_physical_byte({:#018x}): no mem bus", paddr);
            return;
        };
        let mut data = [val];
        let mut dummy_mapping: [u32; 0] = [];
        let mut stamp = super::icache::BxPageWriteStampTable {
            fine_granularity_mapping: &mut dummy_mapping,
        };
        if let Err(e) = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 1, &mut data) {
            tracing::warn!(
                "write_physical_byte({:#018x}) failed: {:?}; byte dropped",
                paddr,
                e
            );
        }
    }

    /// Bochs vapic.cc `VMX_Posted_Interrupt_Processing` — fast probe that
    /// answers whether a posted interrupt is waiting. Returns true when
    /// PROCESS_POSTED_INTERRUPTS is enabled, PID.ON is set, and at least
    /// one PIR bit is set.
    pub(super) fn posted_interrupt_pending(&mut self) -> bool {
        if !self.in_vmx_guest {
            return false;
        }
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_PROCESS_POSTED_INTERRUPTS == 0 {
            return false;
        }
        let desc = self.read_posted_interrupt_descriptor();
        if desc[32] & 1 == 0 {
            return false;
        }
        desc[..32].iter().any(|&b| b != 0)
    }

    /// Bochs vapic.cc `VMX_Posted_Interrupt_Processing` — clear PID.ON
    /// and signal a pending virtual interrupt. Bochs additionally folds PIR
    /// into the virtual-APIC IRR and recomputes RVI; without a fully
    /// virtualised LAPIC we still must clear ON so the host can re-arm and
    /// raise `BX_EVENT_PENDING_VMX_VIRTUAL_INTR` so the deferred vector is
    /// delivered at the next instruction boundary.
    pub(super) fn process_posted_interrupts(&mut self) -> Result<()> {
        if !self.in_vmx_guest {
            return Ok(());
        }
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_PROCESS_POSTED_INTERRUPTS == 0 {
            return Ok(());
        }
        let desc = self.read_posted_interrupt_descriptor();
        if desc[32] & 1 == 0 {
            return Ok(());
        }
        let any_pir = desc[..32].iter().any(|&b| b != 0);

        // RMW on the ON byte mirrors Bochs vapic.cc:
        //   pid_ON = read_physical_byte(pid_addr + 32);
        //   pid_ON &= ~0x1;
        //   write_physical_byte(pid_addr + 32, pid_ON);
        // The full PIR -> VIRR fold and RVI update require a virtualised LAPIC
        // (Bochs vapic.cc handles them inline) — not ported.
        let paddr = self.vmcs.pi_desc_addr;
        let on_byte = self.read_physical_byte(paddr + 32);
        self.write_physical_byte(paddr + 32, on_byte & !0x1);

        if any_pir {
            self.signal_event(Self::BX_EVENT_PENDING_VMX_VIRTUAL_INTR);
        }
        Ok(())
    }

    /// VM-entry / VM-exit MSR load helper — Bochs vmx.cc LoadMSRs.
    /// Walks `count` 16-byte entries starting at `phys_addr`. Each entry is
    /// `(msr_index : Bit32, _reserved : Bit32, value : Bit64)`. Returns the
    /// 1-based index of the failing MSR or `0` on full success. Index `0`
    /// is reserved (Bochs counts from `msr = 1`).
    ///
    /// Validation matches Bochs:
    ///   - High 32 bits of the index field must be zero.
    ///   - FSBASE / GSBASE cannot be restored via this list — those use the
    ///     dedicated VMCS host segment-base fields.
    ///   - X2APIC MSR range (0x800..=0x8FF) is also rejected.
    ///
    /// On a wrmsr that itself raises an exception (e.g. canonical-address
    /// check failure), the exception propagates to the caller; Bochs
    /// triggers a VMX abort in that case which the caller handles.
    pub(super) fn vmx_load_msrs(
        &mut self,
        count: u32,
        phys_addr: u64,
    ) -> Result<u32> {
        let mut paddr = phys_addr;
        for msr in 1..=count {
            let lo = self.mem_read_qword(paddr);
            let value = self.mem_read_qword(paddr + 8);
            paddr = paddr.wrapping_add(16);
            if (lo >> 32) != 0 {
                tracing::warn!(
                    "VMX LoadMSRs[{msr}]: broken msr index {:#018x}",
                    lo
                );
                return Ok(msr);
            }
            let index = lo as u32;
            // Bochs rejects FSBASE (0xC0000100) / GSBASE (0xC0000101) loads
            // — host saves these via dedicated VMCS host segment-base fields.
            if index == 0xC000_0100 || index == 0xC000_0101 {
                tracing::warn!(
                    "VMX LoadMSRs[{msr}]: cannot restore FSBASE/GSBASE via list"
                );
                return Ok(msr);
            }
            if (0x800..=0x8FF).contains(&index) {
                tracing::warn!(
                    "VMX LoadMSRs[{msr}]: X2APIC MSR {:#x} not allowed",
                    index
                );
                return Ok(msr);
            }
            // wrmsr_value bypasses CPL / VMX-intercept gates that the
            // instruction handler enforces; the list is host-supplied so
            // those checks would be inappropriate here.
            self.wrmsr_value(index, value)?;
        }
        Ok(0)
    }

    /// VM-exit MSR store helper — Bochs vmx.cc StoreMSRs. Walks the store
    /// list, calls `rdmsr_value` for each index, and writes the result into
    /// the high qword of each 16-byte entry. Returns the 1-based failing
    /// index or `0` on success. X2APIC MSRs are rejected per Bochs.
    pub(super) fn vmx_store_msrs(
        &mut self,
        count: u32,
        phys_addr: u64,
    ) -> Result<u32> {
        let mut paddr = phys_addr;
        for msr in 1..=count {
            let lo = self.mem_read_qword(paddr);
            if (lo >> 32) != 0 {
                tracing::warn!(
                    "VMX StoreMSRs[{msr}]: broken msr index {:#018x}",
                    lo
                );
                return Ok(msr);
            }
            let index = lo as u32;
            if (0x800..=0x8FF).contains(&index) {
                tracing::warn!(
                    "VMX StoreMSRs[{msr}]: X2APIC MSR {:#x} not allowed",
                    index
                );
                return Ok(msr);
            }
            let value = self.rdmsr_value(index)?;
            self.mem_write_qword(paddr + 8, value);
            paddr = paddr.wrapping_add(16);
        }
        Ok(0)
    }

    /// VM-entry event injection — Bochs vmx.cc VMenterInjectEvents
    /// (~vmx.cc:2421-2511). When the high bit (valid) of
    /// `VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO` is set, the host has
    /// requested an immediate interrupt/exception in the guest. Vector,
    /// type, and push-error flag are decoded from the field; error code is
    /// taken from `VMENTRY_EXCEPTION_ERR_CODE`. For software-int /
    /// soft-exception types, RIP is advanced by `vmentry_instr_length`
    /// before delivery so the handler returns to the next instruction.
    pub(super) fn vmenter_inject_events(&mut self) -> Result<()> {
        let info = self.vmcs.vm_entry_intr_info;
        if info & (1u32 << 31) == 0 {
            return Ok(()); // valid bit clear → nothing to inject
        }
        let vector = (info & 0xFF) as u8;
        let bochs_type = (info >> 8) & 0x7;
        let push_error = (info & (1 << 11)) != 0;
        let error_code = if push_error {
            self.vmcs.vm_entry_exception_error_code
        } else {
            0
        };

        // Bochs BX_EXTERNAL_INTERRUPT=0, BX_NMI=2, BX_HARDWARE_EXCEPTION=3,
        // BX_SOFTWARE_INTERRUPT=4, BX_PRIVILEGED_SOFTWARE_INTERRUPT=5,
        // BX_SOFTWARE_EXCEPTION=6, BX_EVENT_OTHER=7.
        let mut is_int = false;
        let intr_type = match bochs_type {
            0 => {
                self.ext = true;
                super::exception::InterruptType::ExternalInterrupt
            }
            2 => {
                // Bochs vmx.cc VMenterInjectEvents BX_NMI: with VIRTUAL_NMI
                // set the guest-side virtual-NMI tracking is masked (the
                // host's real NMI line is independent); without VIRTUAL_NMI
                // the standard NMI mask is set.
                if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_VIRTUAL_NMI != 0 {
                    self.mask_event(Self::BX_EVENT_VMX_VIRTUAL_NMI);
                } else {
                    self.mask_event(Self::BX_EVENT_NMI);
                }
                self.ext = true;
                super::exception::InterruptType::Nmi
            }
            3 => {
                self.ext = true;
                super::exception::InterruptType::HardwareException
            }
            4 => {
                is_int = true;
                super::exception::InterruptType::SoftwareInterrupt
            }
            5 => {
                self.ext = true;
                is_int = true;
                super::exception::InterruptType::PrivilegedSoftwareInterrupt
            }
            6 => {
                is_int = true;
                super::exception::InterruptType::SoftwareException
            }
            7 => {
                // Bochs vmx.cc VMenterInjectEvents BX_EVENT_OTHER: the
                // MTF marker (vector=0) signals BX_EVENT_VMX_MONITOR_
                // TRAP_FLAG so the next instruction-boundary fires the
                // MTF VMEXIT. Any other vector at this type is rejected
                // by `vmenter_load_check_vm_controls` before we get
                // here; if a stale value sneaks through, abort the
                // VMENTRY rather than silently fabricating an event.
                if vector == 0 {
                    self.signal_event(Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG);
                    return Ok(());
                }
                tracing::error!(
                    "VMENTRY inject_events: unsupported type=7 vector={vector} reached injection (controls check missed it)"
                );
                return Err(super::error::CpuError::CpuLoopRestart);
            }
            // Bochs `default: BX_PANIC`. Unreachable in practice — types
            // 1 and 8..=15 are rejected at VMENTRY validation. Restart
            // the cpu loop rather than silently swallow.
            _ => {
                tracing::error!(
                    "VMENTRY inject_events: reserved type {bochs_type} reached injection"
                );
                return Err(super::error::CpuError::CpuLoopRestart);
            }
        };

        if is_int {
            let new_rip = self
                .rip()
                .wrapping_add(u64::from(self.vmcs.vm_entry_instruction_length));
            self.set_rip(new_rip);
        }

        // Bochs vmx.cc:2484-2488 records the exception classification
        // for HARDWARE_EXCEPTION injections so a fault during delivery
        // can be classified for double-fault detection.
        if bochs_type == 3 && (vector as usize) < super::cpu::BX_CPU_HANDLED_EXCEPTIONS as usize {
            self.last_exception_type = self.exception_type_for(vector);
        }

        // Bochs records the injection in IDT-vectoring info (with valid bit
        // cleared per spec) so a fault during delivery is attributed correctly.
        self.vmcs.idt_vectoring_info = info & !(1u32 << 31);
        self.vmcs.idt_vectoring_error_code = error_code;

        let err16 = (error_code & 0xFFFF) as u16;
        let res = self.interrupt(vector, intr_type, push_error, push_error, err16);
        self.ext = false;
        // Bochs vmx.cc:2510: clear last_exception_type after delivery so
        // subsequent unrelated faults aren't classified as double-fault
        // continuations.
        self.last_exception_type = -1; // BX_ET_NONE
        res
    }

    // ── VMX preemption-timer audit (Bochs vmx.cc + apic.cc, 2026-04-26) ──
    // Comparison vs Bochs:
    // * Arm path: Bochs `BX_CPU_C::VMenterLoadCheckGuestState` step 6 reads
    //   `VMCS_32BIT_GUEST_PREEMPTION_TIMER_VALUE`; if 0 signals
    //   BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED, else calls
    //   `lapic->set_vmx_preemption_timer(value)`. Matches `vmenter_arm_preemption_timer`.
    // * Disarm path: Bochs `BX_CPU_C::VMexit` calls
    //   `lapic->deactivate_vmx_preemption_timer()` + clear_event, then
    //   `VMwrite32(GUEST_PREEMPTION_TIMER_VALUE, lapic->read_vmx_preemption_timer())`
    //   when `vm_exit_ctls.STORE_VMX_PREEMPTION_TIMER` is set. Matches
    //   `vmexit_disarm_preemption_timer`; the snapshot order (STORE-then-deactivate)
    //   is observably equivalent because `read_vmx_preemption_timer` does not
    //   depend on `vmx_timer_active`.
    // * Tick rate: Bochs `apic.cc` uses `>> vmx_preemption_timer_rate` /
    //   `<< vmx_preemption_timer_rate` with the rate sourced from
    //   `IA32_VMX_MISC[4:0]` (`VMX_MISC_PREEMPTION_TIMER_RATE = 0` by default).
    //   Identical formula in `apic::set_vmx_preemption_timer` /
    //   `read_vmx_preemption_timer` — the 64-bit `wrapping_add` matches Bochs's
    //   implicit `Bit64u` arithmetic.
    // * Wraparound: `((initial >> rate) + value) << rate` is computed in 64 bits;
    //   `read_vmx_preemption_timer` saturates to 0 when elapsed ≥ value, matching
    //   Bochs's `if (vmx_preemption_timer_value < diff) return 0`.
    // * VMEXIT reason: `VmxPreemptionTimerExpired` (52) — matches Bochs
    //   `VMX_VMEXIT_VMX_PREEMPTION_TIMER_EXPIRED`.
    // * Gating: `event.rs::handle_async_event` guards `poll_vmx_preemption_timer`
    //   on `in_vmx_guest`, so the LAPIC tick is observed only inside guest mode.
    //   Bochs achieves the same effect by registering the `vmx_timer` callback
    //   only while the timer is `active`, and deactivating it on every VMEXIT.
    // * Architectural divergence (intentional): Bochs registers a real
    //   `bx_pc_system` timer that fires the expiry callback asynchronously;
    //   rusty_box has no background timer service and instead polls
    //   `lapic.vmx_preemption_timer_expired` at every async-event boundary.
    //   Both signal `BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED` exactly when
    //   `system_ticks() >= fire_deadline`, so guest-visible behaviour is
    //   identical for any clock-advancement schedule the cpu loop produces.
    // * Defensive deltas vs Bochs: vmenter calls `deactivate_vmx_preemption_timer`
    //   when the pin control is OFF and clear_event before re-arm. Bochs relies
    //   on the prior VMEXIT having deactivated; the calls here are idempotent.
    // * VPID-tagged TLB: the preemption timer never touches the TLB, so VPID
    //   semantics are unaffected by this code path.
    /// Arm the VMX preemption timer at VM-entry — Bochs vmx.cc step 6.
    /// Reads `vmx_preemption_timer_value` from the cached VMCS, hands it to
    /// `lapic.set_vmx_preemption_timer`, and clears any stale expiry event.
    /// A zero-valued timer signals the expiry event immediately (Bochs
    /// `signal_event(BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED)`).
    pub(super) fn vmenter_arm_preemption_timer(&mut self) {
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_VMX_PREEMPTION_TIMER_VMEXIT == 0 {
            self.lapic.deactivate_vmx_preemption_timer();
            self.clear_event(Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED);
            return;
        }
        let value = self.vmcs.vmx_preemption_timer_value;
        self.clear_event(Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED);
        if value == 0 {
            self.signal_event(Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED);
        } else {
            let now = self.system_ticks();
            self.lapic.set_vmx_preemption_timer(value, now);
        }
    }

    /// Disarm the VMX preemption timer at VM-exit — Bochs vmx.cc VMexit
    /// step 1. When `STORE_VMX_PREEMPTION_TIMER` is set the remaining
    /// countdown (from `lapic.read_vmx_preemption_timer`) is snapshotted
    /// back into the VMCS field; the LAPIC timer and the pending event
    /// are then cleared.
    pub(super) fn vmexit_disarm_preemption_timer(&mut self) {
        if self.vmcs.vm_exit_ctls & VMX_VMEXIT_CTRL1_STORE_VMX_PREEMPTION_TIMER != 0 {
            let now = self.system_ticks();
            self.vmcs.vmx_preemption_timer_value =
                self.lapic.read_vmx_preemption_timer(now);
        }
        self.lapic.deactivate_vmx_preemption_timer();
        self.clear_event(Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED);
    }

    /// Preemption-timer expiry check — Bochs event.cc
    /// `BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED` branch. Called from
    /// `handle_async_event` after the LAPIC tick has had a chance to
    /// signal the event; exits with reason
    /// `VMX_VMEXIT_VMX_PREEMPTION_TIMER_EXPIRED`.
    pub(super) fn vmexit_check_preemption_timer(&mut self) -> Result<bool> {
        if !self.is_unmasked_event_pending(Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED) {
            return Ok(false);
        }
        self.clear_event(Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED);
        self.vmx_vmexit(VmxVmexitReason::VmxPreemptionTimerExpired, 0)?;
        Ok(true)
    }

    /// Monitor-trap-flag VMEXIT — Bochs event.cc
    /// `BX_EVENT_VMX_MONITOR_TRAP_FLAG` branch (event.cc lines 312-319):
    ///   if (is_pending(MTF)) {
    ///     if (is_unmasked_event_pending(MTF)) VMexit(...);
    ///     else                                unmask_event(MTF);
    ///   }
    /// The masked-then-unmask path is required so the MTF fires on the
    /// NEXT instruction boundary after VMENTRY masked it for one
    /// instruction.
    pub(super) fn vmexit_check_monitor_trap_flag(&mut self) -> Result<bool> {
        if (self.pending_event & Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG) == 0 {
            return Ok(false);
        }
        if self.is_unmasked_event_pending(Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG) {
            self.clear_event(Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG);
            self.vmx_vmexit(VmxVmexitReason::MonitorTrapFlag, 0)?;
            return Ok(true);
        }
        self.unmask_event(Self::BX_EVENT_VMX_MONITOR_TRAP_FLAG);
        Ok(false)
    }

    /// Per-tick LAPIC poll — Bochs `vmx_preemption_timer_expired` callback.
    /// Returns true after signalling the event the first time the LAPIC
    /// reports the timer has fired, so the next async-event boundary
    /// triggers VMEXIT.
    pub(super) fn poll_vmx_preemption_timer(&mut self) -> bool {
        let now = self.system_ticks();
        if self.lapic.vmx_preemption_timer_expired(now) {
            self.signal_event(Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED);
            return true;
        }
        false
    }

    /// NMI-window VMEXIT — Bochs event.cc
    /// `is_unmasked_event_pending(BX_EVENT_VMX_VIRTUAL_NMI)` branch.
    /// VMENTRY signals the event when `NMI_WINDOW_EXITING` is set in
    /// proc-based controls; virtual-NMI blocking masks it. The exit
    /// fires when the event is both pending and unmasked.
    pub(super) fn vmexit_check_nmi_window(&mut self) -> Result<bool> {
        if !self.is_unmasked_event_pending(Self::BX_EVENT_VMX_VIRTUAL_NMI) {
            return Ok(false);
        }
        self.vmx_vmexit(VmxVmexitReason::NmiWindow, 0)?;
        Ok(true)
    }

    /// Interrupt-window VMEXIT — Bochs event.cc
    /// `is_pending(BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING) && get_IF()`
    /// branch. VMENTRY signals the event when `INTERRUPT_WINDOW_VMEXIT`
    /// is set in proc-based controls. The exit fires whenever the event
    /// is pending, `RFLAGS.IF=1`, and external-interrupt inhibition is
    /// clear (the inhibit gate is in the caller's chain).
    pub(super) fn vmexit_check_interrupt_window(&mut self) -> Result<bool> {
        if (self.pending_event & Self::BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING) == 0 {
            return Ok(false);
        }
        if !self.eflags.contains(super::eflags::EFlags::IF_) {
            return Ok(false);
        }
        self.vmx_vmexit(VmxVmexitReason::InterruptWindow, 0)?;
        Ok(true)
    }

    /// External-interrupt VMEXIT after the controller has been acknowledged
    /// — Bochs vmexit.cc `VMexit_Event` with `type == BX_EXTERNAL_INTERRUPT`.
    /// Records the acknowledged vector in `exit_intr_info` so the host can
    /// re-deliver it.
    pub(super) fn vmexit_check_event_intr(&mut self, vector: u8) -> Result<bool> {
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_EXTERNAL_INTERRUPT_VMEXIT == 0 {
            return Ok(false);
        }
        // type=ExternalInterrupt(0), vector, valid bit 31.
        self.vmcs.exit_intr_info = u32::from(vector) | (1u32 << 31);
        self.vmcs.exit_intr_error_code = 0;
        self.vmx_vmexit(VmxVmexitReason::ExternalInterrupt, 0)?;
        Ok(true)
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

    /// INVEPT in non-root operation always exits — Bochs vmx.cc INVEPT
    /// `if (in_vmx_guest) VMexit_Instruction(VMX_VMEXIT_INVEPT, BX_WRITE)`.
    pub(super) fn vmexit_check_invept(&mut self) -> Result<bool> {
        if !self.in_vmx_guest {
            return Ok(false);
        }
        self.vmx_vmexit(VmxVmexitReason::Invept, 0)?;
        Ok(true)
    }

    /// INVVPID in non-root operation always exits — Bochs vmx.cc INVVPID.
    pub(super) fn vmexit_check_invvpid(&mut self) -> Result<bool> {
        if !self.in_vmx_guest {
            return Ok(false);
        }
        self.vmx_vmexit(VmxVmexitReason::Invvpid, 0)?;
        Ok(true)
    }

    /// True when guest-physical → host-physical translation must go
    /// through the EPT walker. Bochs precondition for every call into
    /// `translate_guest_physical`.
    #[inline]
    pub(super) fn ept_active(&self) -> bool {
        self.in_vmx_guest
            && self.proc_based_ctls2() & VMX_VM_EXEC_CTRL2_EPT_ENABLE != 0
    }

    /// EPT translation for a paging-structure access — Bochs uses
    /// `translate_guest_physical(paddr, laddr, true, true, /*user*/false,
    /// /*writeable*/false, /*nx*/false, BX_READ)` from inside page walks.
    /// When EPT is inactive returns the input unchanged.
    pub(super) fn ept_translate_for_walk(
        &mut self,
        guest_paddr: u64,
        guest_laddr: u64,
    ) -> Result<u64> {
        if !self.ept_active() {
            return Ok(guest_paddr);
        }
        self.translate_guest_physical(
            guest_paddr,
            guest_laddr,
            true,
            true,
            false,
            false,
            false,
            BxRwAccess::Read,
        )
    }

    /// EPT translation for a data access — Bochs `translate_guest_physical`
    /// with `is_page_walk=false`. The user/writeable/nx page metadata
    /// flows from the inner page walk and feeds the EPT-violation
    /// qualification.
    pub(super) fn ept_translate_for_data(
        &mut self,
        guest_paddr: u64,
        guest_laddr: u64,
        user_page: bool,
        writeable_page: bool,
        nx_page: bool,
        rw: BxRwAccess,
    ) -> Result<u64> {
        if !self.ept_active() {
            return Ok(guest_paddr);
        }
        self.translate_guest_physical(
            guest_paddr,
            guest_laddr,
            true,
            false,
            user_page,
            writeable_page,
            nx_page,
            rw,
        )
    }

    /// EPT 4-level walker — Bochs paging.cc `translate_guest_physical`.
    /// Translates a 52-bit guest-physical address to a host-physical
    /// address through the EPT page tables rooted at `vmcs.eptptr`. On
    /// permission failure raises `EPT_VIOLATION`; on a malformed EPT
    /// entry raises `EPT_MISCONFIGURATION`. Both VMEXITs return
    /// `Err(CpuLoopRestart)` so the caller's page walk aborts.
    ///
    /// `rw` is one of `EPT_RW_READ` / `EPT_RW_WRITE` / `EPT_RW_EXEC`.
    /// `is_page_walk` is true when the caller is fetching a paging
    /// structure (Bochs sets the qualification bit accordingly).
    pub(super) fn translate_guest_physical(
        &mut self,
        guest_paddr: u64,
        guest_laddr: u64,
        guest_laddr_valid: bool,
        is_page_walk: bool,
        user_page: bool,
        writeable_page: bool,
        nx_page: bool,
        rw: BxRwAccess,
    ) -> Result<u64> {
        let eptptr = self.vmcs.eptptr;
        let mut ppf = eptptr & 0x000F_FFFF_FFFF_F000;
        let mbe_ctrl = self.vmcs.secondary_proc_based_ctls
            & VMX_VM_EXEC_CTRL2_MBE_CTRL
            != 0;
        // Bochs `BX_VMX_EPT_ACCESS_DIRTY_ENABLED` (paging.cc): EPTP bit 6.
        let ept_ad_enabled = (eptptr & 0x40) != 0;

        // Bochs paging.cc translate_guest_physical: when EPT-A/D is on
        // and we're walking a guest paging structure, treat the access
        // as a write so the EPT permission check requires WRITE — the
        // companion `update_ept_access_dirty` (called below on success)
        // updates the EPT entries' own A/D bits.
        let rw = if ept_ad_enabled && is_page_walk && guest_laddr_valid {
            BxRwAccess::Write
        } else {
            rw
        };

        // combined_access starts as "all permissions granted"; the walker
        // ANDs each level's bits in. Under MBE_CTRL the user-execute bit
        // additionally rides along (Bochs paging.cc).
        let mut combined_access = EptPerm::READ | EptPerm::WRITE | EptPerm::EXECUTE;
        if mbe_ctrl {
            combined_access |= EptPerm::MBE_USER_EXEC;
        }
        // Bochs paging.cc access-mask construction (BxRwAccess helpers
        // mirror the `rw == BX_EXECUTE`, `rw & 1`, `(rw & 3) == BX_READ`
        // predicates). Shadow-stack reads still set the READ bit because
        // `(rw & 3) == 0` matches both BX_READ and BX_SHADOW_STACK_READ.
        let mut access_mask = EptPerm::empty();
        if rw == BxRwAccess::Execute {
            // Under MBE_CTRL the EXECUTE bit is split into user / supervisor
            // variants; for the user-page path Bochs sets EPT_MBE_USER_EXEC.
            // Without MBE_CTRL the legacy EPT_EXECUTE bit applies.
            if mbe_ctrl {
                access_mask |= if user_page {
                    EptPerm::MBE_USER_EXEC
                } else {
                    EptPerm::EXECUTE
                };
            } else {
                access_mask |= EptPerm::EXECUTE;
            }
        }
        if rw.is_write() {
            access_mask |= EptPerm::WRITE;
        }
        if rw.is_read_like() {
            access_mask |= EptPerm::READ;
        }

        let mut entry = [0u64; 4];
        let mut entry_addr = [0u64; 4];
        let mut offset_mask: u64 = 0x0000_FFFF_FFFF_FFFF;
        let mut leaf: i32 = 3; // BX_LEVEL_PML4
        let mut vmexit_reason: Option<VmxVmexitReason> = None;

        loop {
            let level = leaf as u32;
            entry_addr[leaf as usize] =
                ppf + ((guest_paddr >> (9 + 9 * level)) & 0xFF8);
            entry[leaf as usize] = self.mem_read_qword(entry_addr[leaf as usize]);
            offset_mask >>= 9;
            let curr = entry[leaf as usize];
            // Bochs paging.cc: per-entry access bits are R/W/X plus the
            // MBE-user-execute bit (only meaningful when MBE_CTRL set).
            let mut curr_access = EptPerm::from_bits_truncate((curr as u32) & 0x7);
            if mbe_ctrl {
                curr_access |= EptPerm::from_bits_truncate(
                    (curr as u32) & EptPerm::MBE_USER_EXEC.bits(),
                );
            }

            // R=0/W=0/X=0 → entry not present (Bochs `BX_EPT_ENTRY_NOT_
            // PRESENT`).
            if curr_access.is_empty() {
                vmexit_reason = Some(VmxVmexitReason::EptViolation);
                break;
            }
            // R=0/W=1/X=0 is the illegal "write-only" combination; Bochs
            // `BX_EPT_ENTRY_WRITE_ONLY` triggers EPT_MISCONFIGURATION.
            if (curr_access & (EptPerm::READ | EptPerm::WRITE)) == EptPerm::WRITE {
                vmexit_reason = Some(VmxVmexitReason::EptMisconfiguration);
                break;
            }
            // Memory type validity (Bochs isMemTypeValidMTRR): UC(0)/WC(1)/
            // WT(4)/WP(5)/WB(6) accepted; 2/3/7 reject.
            let memtype = ((curr >> 3) & 7) as u32;
            if !matches!(memtype, 0 | 1 | 4 | 5 | 6) {
                vmexit_reason = Some(VmxVmexitReason::EptMisconfiguration);
                break;
            }
            // Bochs PAGING_EPT_RESERVED_BITS = BX_PHY_ADDRESS_RESERVED_BITS
            // & 0xFFFFFFFFFFFFF (paging.cc) — only bits [51:phy_width] are
            // reserved. Bits [63:52] are NOT reserved here: architecturally
            // they hold #VE-suppress (63), SPP (61), SSS (60), paging-write
            // (58), VGP (57), so the EPT walker must not flag them as
            // misconfig. With BX_PHY_ADDRESS_WIDTH=40 only bits [51:40]
            // are reserved at the phy-addr boundary.
            const PAGING_EPT_RESERVED_BITS: u64 = ((1u64 << 12) - 1) << 40;
            if curr & PAGING_EPT_RESERVED_BITS != 0 {
                vmexit_reason = Some(VmxVmexitReason::EptMisconfiguration);
                break;
            }
            ppf = curr & 0x000F_FFFF_FFFF_F000;

            if leaf == 0 {
                break; // BX_LEVEL_PTE
            }
            if curr & 0x80 != 0 {
                // Large-page leaf. Bochs allows the leaf at PDE level
                // unconditionally and at PDPTE level only when the CPU
                // model advertises BX_ISA_1G_PAGES; PML4/PML5 large-page
                // entries are reserved.
                let max_large_page_leaf = if self
                    .bx_cpuid_support_isa_extension(super::decoder::features::X86Feature::Isa1gPages)
                {
                    2 // PDPTE
                } else {
                    1 // PDE
                };
                if leaf > max_large_page_leaf {
                    vmexit_reason = Some(VmxVmexitReason::EptMisconfiguration);
                    break;
                }
                // Bochs paging.cc: `ppf &= 0x000fffffffffe000` — clears
                // bit 12 only on top of the 4 KiB ppf mask. The
                // subsequent `ppf & offset_mask` reserved-bit check then
                // sees bits [20:13] (2 MiB leaf) or [29:13] (1 GiB leaf)
                // and rejects unaligned PFNs. Clearing more bits here
                // hides reserved-bit violations.
                ppf &= 0x000F_FFFF_FFFF_E000;
                if ppf & offset_mask != 0 {
                    vmexit_reason = Some(VmxVmexitReason::EptMisconfiguration);
                    break;
                }
                ppf += guest_paddr & offset_mask;
                break;
            }
            // Non-leaf entry must have memtype-low/leaf reserved bits clear
            if ((curr >> 3) & 0xF) != 0 {
                vmexit_reason = Some(VmxVmexitReason::EptMisconfiguration);
                break;
            }
            combined_access &= curr_access;
            leaf -= 1;
        }

        if vmexit_reason.is_none() {
            combined_access &= EptPerm::from_bits_truncate(entry[leaf as usize] as u32);
            if (access_mask & combined_access) != access_mask {
                vmexit_reason = Some(VmxVmexitReason::EptViolation);
            }
        }

        if let Some(reason) = vmexit_reason {
            // EPT_MISCONFIGURATION VMEXIT carries no qualification (Bochs).
            // EPT_VIOLATION qualification bits per Bochs vmexit.cc:
            //   [2:0]   access_mask (R/W/X bits requested)
            //   [5:3]   combined_access (R/W/X bits actually granted)
            //   [7]     guest_laddr valid
            //   [8]     is data access (not page walk)
            //   [9]     user-mode access
            //   [10]    writeable page
            //   [11]    NX page
            // Bochs paging.cc EPT VMEXIT qualification builder. Bits 9/10/11
            // (user / writeable / nx page) gate on the CPU model
            // advertising BX_VMX_MBE_CONTROL via vmx_extensions_bitmask;
            // bit 12 reads `nmi_unblocking_iret` from CPU state; bit 13
            // sets on shadow-stack accesses (rw & 4) per BX_SUPPORT_CET.
            let qual = if reason == VmxVmexitReason::EptViolation {
                combined_access &= EptPerm::from_bits_truncate(entry[leaf as usize] as u32);
                // Bochs paging.cc qualification builder: always start with
                // access_mask at [2:0] | combined_access at [5:3]. The
                // MBE+execute branch then masks to bits [5:0] (preserving
                // the granted bits at [5:3]), forces bit 2 (instruction
                // fetch), and copies user-execute up to bit 6.
                let mut q = EptViolationQual::empty();
                q.set(EptViolationQual::ACCESS_R, access_mask.contains(EptPerm::READ));
                q.set(EptViolationQual::ACCESS_W, access_mask.contains(EptPerm::WRITE));
                q.set(EptViolationQual::ACCESS_X, access_mask.contains(EptPerm::EXECUTE));
                q.set(EptViolationQual::GRANTED_R, combined_access.contains(EptPerm::READ));
                q.set(EptViolationQual::GRANTED_W, combined_access.contains(EptPerm::WRITE));
                q.set(EptViolationQual::GRANTED_X, combined_access.contains(EptPerm::EXECUTE));
                if mbe_ctrl && rw == BxRwAccess::Execute {
                    // Force bit 2 (instruction fetch) and lift the leaf's
                    // MBE_USER_EXECUTE permission into bit 6.
                    q |= EptViolationQual::ACCESS_X;
                    q.set(
                        EptViolationQual::MBE_USER_EXEC,
                        combined_access.contains(EptPerm::MBE_USER_EXEC),
                    );
                }
                if guest_laddr_valid {
                    q |= EptViolationQual::LADDR_VALID;
                    if !is_page_walk {
                        q |= EptViolationQual::DATA_ACCESS;
                        // Bochs paging.cc gates the advanced VM-exit
                        // information bits on BX_VMX_MBE_CONTROL.
                        let mbe = self
                            .vmx_extensions_bitmask
                            .as_ref()
                            .map_or(false, |m| {
                                m.contains(super::cpuid::VMXExtensions::MbeControl)
                            });
                        if mbe {
                            q.set(EptViolationQual::USER_PAGE, user_page);
                            q.set(EptViolationQual::WRITEABLE, writeable_page);
                            q.set(EptViolationQual::NX_PAGE, nx_page);
                        }
                    }
                }
                // Bochs paging.cc: bit 12 from the CPU's nmi_unblocking_iret
                // flag (set by IRET when the IRET unblocked NMI delivery,
                // cleared on next instruction-boundary).
                q.set(EptViolationQual::NMI_UNBLOCK, self.nmi_unblocking_iret);
                // Bochs paging.cc gates the shadow-stack bit on
                // BX_SUPPORT_CET; only set when the CPU model supports
                // CET (otherwise `rw.is_shadow_stack()` cannot be
                // observed in practice but the literal source check
                // matters for forward-compat models).
                if self
                    .bx_cpuid_support_isa_extension(super::decoder::features::X86Feature::IsaCet)
                {
                    q.set(EptViolationQual::SHADOW_STACK, rw.is_shadow_stack());
                    // Bochs paging.cc bit 14:
                    //   `BX_VMX_EPT_SUPERVISOR_SHADOW_STACK_CTRL_ENABLED &&
                    //    ept_supervisor_shadow_stack_page_bit(entry[leaf])`
                    // The first conjunct is `EPTPTR & 0x80`; the second is
                    // bit 60 of the EPT leaf entry.
                    let leaf_idx = leaf as usize;
                    let leaf_entry = entry[leaf_idx];
                    let ept_sss_ctrl = (eptptr & 0x80) != 0;
                    let leaf_sss_page = (leaf_entry & (1u64 << 60)) != 0;
                    q.set(EptViolationQual::SSS_PAGE, ept_sss_ctrl && leaf_sss_page);
                }
                self.vmcs.guest_linear_addr = guest_laddr;
                self.vmcs.guest_physical_addr = guest_paddr;
                q.bits()
            } else {
                self.vmcs.guest_physical_addr = guest_paddr;
                0
            };
            self.vmx_vmexit(reason, qual)?;
            return Err(super::error::CpuError::CpuLoopRestart);
        }

        // Bochs paging.cc translate_guest_physical: when EPT-A/D is enabled,
        // write the EPT entries' own A/D bits back. Cross-checked against
        // Bochs paging.cc::translate_guest_physical and update_ept_access_dirty:
        // the leaf gets A and (when the access is a write) D; every
        // higher-level entry walked gets A.
        if ept_ad_enabled {
            self.update_ept_access_dirty(&entry_addr, &mut entry, leaf as usize, rw.is_write());
        }

        Ok(ppf | (guest_paddr & 0xFFF))
    }

    /// Bochs paging.cc `BX_CPU_C::update_ept_access_dirty` (cross-checked
    /// against `translate_guest_physical` A/D update at lines ~2116-2123 +
    /// definition at ~2130-2145). Walks from PML4 down to the leaf level,
    /// setting the A bit (0x100, bit 8) on every non-leaf entry that doesn't
    /// yet have it. The leaf entry gets A AND (when the access is a write) D
    /// (0x200, bit 9). Updates are written back to the EPT entry addresses
    /// cached during the walk. Bochs flags both writes "should be done with
    /// locked RMW" — rusty_box mirrors the same non-atomic sequence; SMP-
    /// correctness for concurrent EPT walks is a known Bochs-parity gap.
    fn update_ept_access_dirty(
        &mut self,
        entry_addr: &[u64; 4],
        entry: &mut [u64; 4],
        leaf: usize,
        write: bool,
    ) {
        // Non-leaf levels (PML4 down to just above leaf): set A bit.
        for level in ((leaf + 1)..4).rev() {
            if entry[level] & 0x100 == 0 {
                entry[level] |= 0x100;
                self.mem_write_qword(entry_addr[level], entry[level]);
            }
        }
        // Leaf: set A, plus D if this was a write access.
        let needed = 0x100 | if write { 0x200 } else { 0 };
        if (entry[leaf] & needed) != needed {
            entry[leaf] |= needed;
            self.mem_write_qword(entry_addr[leaf], entry[leaf]);
        }
    }

    /// EPTPTR-validity check — Bochs vmx.cc `is_eptptr_valid`.
    /// Validates the EPT memory type, walk length, and reserved bit
    /// pattern. Returns `true` when the host has set up a usable EPT.
    pub(super) fn is_eptptr_valid(&self, eptptr: u64) -> bool {
        const BX_MEMTYPE_UC: u64 = 0;
        const BX_MEMTYPE_WB: u64 = 6;

        let memtype = eptptr & 7;
        if memtype != BX_MEMTYPE_UC && memtype != BX_MEMTYPE_WB {
            return false;
        }
        // [5:3] is `walk_length - 1`. Bochs only accepts 4-level paging.
        let walk_length = (eptptr >> 3) & 7;
        if walk_length != 3 {
            return false;
        }
        // [6] EPT A/D — gated on the CPU model's EptAccessDirty
        // extension. translate_guest_physical reads this bit to drive
        // the rw=BX_WRITE upgrade for guest-paging walks and to call
        // update_ept_access_dirty after a successful walk. Models that
        // don't advertise EPT-A/D must reject this bit to match Bochs
        // is_eptptr_valid (vmx.cc).
        if eptptr & 0x40 != 0 {
            let supported = self
                .vmx_extensions_bitmask
                .as_ref()
                .map_or(false, |m| {
                    m.contains(super::cpuid::VMXExtensions::EptAccessDirty)
                });
            if !supported {
                tracing::trace!("is_eptptr_valid: EPTP A/D bit set but model lacks EptAccessDirty");
                return false;
            }
        }
        // [7] CET supervisor shadow stack control — Bochs is_eptptr_valid
        // (vmx.cc) only rejects this bit when `BX_SUPPORT_CET` is off.
        // CET-supporting models accept it (and the bit is observed by the
        // EPT-violation qualification builder).
        if eptptr & 0x80 != 0
            && !self.bx_cpuid_support_isa_extension(super::decoder::features::X86Feature::IsaCet)
        {
            tracing::trace!("is_eptptr_valid: EPTPTR CET-SS bit set without IsaCet support");
            return false;
        }
        // [11:8] reserved.
        if eptptr & 0xF00 != 0 {
            tracing::trace!("is_eptptr_valid: EPTPTR reserved bits set");
            return false;
        }
        // [BX_PHY_ADDRESS_WIDTH-1:12] page-frame address.
        if (eptptr >> BX_PHY_ADDRESS_WIDTH) != 0 {
            return false;
        }
        true
    }

    /// MSR bitmap walker — Bochs vmexit.cc VMexit_MSR.
    ///
    /// Layout of the 4 KiB bitmap at `msr_bitmap_addr`:
    /// - Bytes `0x000..0x400` — read bitmap for low MSRs `0x00000000..=0x00001FFF`
    /// - Bytes `0x400..0x800` — read bitmap for high MSRs `0xC0000000..=0xC0001FFF`
    /// - Bytes `0x800..0xC00` — write bitmap for low MSRs
    /// - Bytes `0xC00..0x1000` — write bitmap for high MSRs
    ///
    /// MSR indices outside both ranges always force a VMEXIT.
    fn msr_bitmap_says_vmexit(&mut self, msr: u32, readmsr: bool) -> bool {
        const LO_END: u32 = 0x0000_1FFF;
        const HI_START: u32 = 0xC000_0000;
        const HI_END: u32 = 0xC000_1FFF;

        let bitmap = self.vmcs.msr_bitmap_addr;
        let write_off: u64 = if readmsr { 0 } else { 2048 };

        if msr >= HI_START {
            if msr > HI_END {
                return true;
            }
            let paddr = bitmap
                + u64::from((msr - HI_START) >> 3)
                + 1024
                + write_off;
            let field = self.read_physical_byte(paddr);
            (field & (1 << (msr & 7))) != 0
        } else {
            if msr > LO_END {
                return true;
            }
            let paddr = bitmap + u64::from(msr >> 3) + write_off;
            let field = self.read_physical_byte(paddr);
            (field & (1 << (msr & 7))) != 0
        }
    }

    /// RDMSR intercept — Bochs vmexit.cc VMexit_MSR with `readmsr=true`.
    pub(super) fn vmexit_check_rdmsr(&mut self, msr: u32) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_MSR_BITMAPS == 0 {
            // Without bitmaps every RDMSR exits unconditionally; qualification 0.
            self.vmx_vmexit(VmxVmexitReason::Rdmsr, 0)?;
            return Ok(true);
        }
        if self.msr_bitmap_says_vmexit(msr, true) {
            self.vmx_vmexit(VmxVmexitReason::Rdmsr, 0)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// WRMSR intercept — Bochs vmexit.cc VMexit_MSR with `readmsr=false`.
    pub(super) fn vmexit_check_wrmsr(&mut self, msr: u32) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_MSR_BITMAPS == 0 {
            self.vmx_vmexit(VmxVmexitReason::Wrmsr, 0)?;
            return Ok(true);
        }
        if self.msr_bitmap_says_vmexit(msr, false) {
            self.vmx_vmexit(VmxVmexitReason::Wrmsr, 0)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// LGDT / SGDT / LIDT / SIDT intercept — Bochs protect_ctrl.cc gates each
    /// on `vmexec_ctrls2.DESCRIPTOR_TABLE_VMEXIT()`. Qualification here carries
    /// the resolved displacement / effective address per Bochs vmexit.cc
    /// VMexit_Instruction; the INSTRUCTION_INFO field is left unpopulated until
    /// a VMM needs it.
    pub(super) fn vmexit_check_gdtr_idtr_access(
        &mut self,
        qualification: u64,
    ) -> Result<bool> {
        let ctls = self.proc_based_ctls2();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL2_DESCRIPTOR_TABLE_VMEXIT,
            VmxVmexitReason::GdtrIdtrAccess,
            qualification,
        )
    }

    /// LLDT / SLDT / LTR / STR intercept — same gate as GDTR/IDTR above but
    /// reported as `LdtrTrAccess`.
    pub(super) fn vmexit_check_ldtr_tr_access(
        &mut self,
        qualification: u64,
    ) -> Result<bool> {
        let ctls = self.proc_based_ctls2();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL2_DESCRIPTOR_TABLE_VMEXIT,
            VmxVmexitReason::LdtrTrAccess,
            qualification,
        )
    }

    /// MOV from CR3 intercept — Bochs vmexit.cc VMexit_CR3_Read.
    /// Qualification layout for CR-access VM-exits (Bochs vmexit.cc):
    ///   [3:0]   CR number
    ///   [5:4]   access type: 0 = MOV to CR, 1 = MOV from CR, 2 = CLTS, 3 = LMSW
    ///   [6]     LMSW memory operand flag (not used here)
    ///   [11:8]  source/destination GPR
    ///   [31:16] LMSW source data (cleared for CR access)
    pub(super) fn vmexit_check_cr3_read(&mut self, gpr: u8) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_CR3_READ_VMEXIT == 0 {
            return Ok(false);
        }
        let qual: u64 = 3 | (1 << 4) | ((u64::from(gpr) & 0xF) << 8);
        self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
        Ok(true)
    }

    /// MOV to CR3 intercept — Bochs vmexit.cc VMexit_CR3_Write. The CR3-target
    /// list provides a fast-path: if the new value matches any enabled target
    /// value, the write is allowed without VMEXIT.
    pub(super) fn vmexit_check_cr3_write(&mut self, val: u64, gpr: u8) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_CR3_WRITE_VMEXIT == 0 {
            return Ok(false);
        }
        let cnt = usize::try_from(self.vmcs.vm_cr3_target_cnt).unwrap_or(0);
        let cnt = cnt.min(self.vmcs.vm_cr3_target_value.len());
        for i in 0..cnt {
            if self.vmcs.vm_cr3_target_value[i] == val {
                return Ok(false);
            }
        }
        let qual: u64 = 3 | ((u64::from(gpr) & 0xF) << 8);
        self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
        Ok(true)
    }

    /// MOV from CR8 intercept — Bochs vmexit.cc VMexit_CR8_Read. Gated on
    /// `CR8_READ_VMEXIT`; qualification matches the standard CR-access
    /// layout with `CR# = 8`, access type = MOV from CR (1), `gpr` in
    /// `[11:8]`.
    pub(super) fn vmexit_check_cr8_read(&mut self, gpr: u8) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_CR8_READ_VMEXIT == 0 {
            return Ok(false);
        }
        let qual: u64 = 8 | (1 << 4) | ((u64::from(gpr) & 0xF) << 8);
        self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
        Ok(true)
    }

    /// MOV to CR8 intercept — Bochs vmexit.cc VMexit_CR8_Write. Gated on
    /// `CR8_WRITE_VMEXIT`; qualification has `CR# = 8`, access type = MOV
    /// to CR (0), `gpr` in `[11:8]`.
    pub(super) fn vmexit_check_cr8_write(&mut self, gpr: u8) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_CR8_WRITE_VMEXIT == 0 {
            return Ok(false);
        }
        let qual: u64 = 8 | ((u64::from(gpr) & 0xF) << 8);
        self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
        Ok(true)
    }

    /// MOV to CR0 intercept — Bochs vmexit.cc VMexit_CR0_Write. The guest
    /// cannot touch bits pinned by `cr0_guest_host_mask` (aka `vm_cr0_mask`);
    /// an attempted change triggers a VMEXIT, otherwise the write proceeds
    /// but masked bits keep their hardware value (read shadow merge).
    ///
    /// Returns `(exited, effective_val)`. When `exited` is true the caller
    /// must return `Ok(())` immediately; otherwise it must write
    /// `effective_val` (not the raw `val`) to CR0.
    pub(super) fn vmexit_check_cr0_write(
        &mut self,
        val: u64,
        gpr: u8,
    ) -> Result<(bool, u64)> {
        let mask = self.vmcs.cr0_guest_host_mask;
        let shadow = self.vmcs.cr0_read_shadow;
        if (mask & shadow) != (mask & val) {
            self.vmx_vmexit(VmxVmexitReason::CrAccess, (u64::from(gpr) & 0xF) << 8)?;
            return Ok((true, val));
        }
        // Keep bits set in the mask untouched.
        let cur = u64::from(self.cr0.get32());
        Ok((false, (cur & mask) | (val & !mask)))
    }

    /// MOV to CR4 intercept — Bochs vmexit.cc VMexit_CR4_Write. Same shape
    /// as CR0: VMEXIT when the masked shadow bits change, else merge.
    pub(super) fn vmexit_check_cr4_write(
        &mut self,
        val: u64,
        gpr: u8,
    ) -> Result<(bool, u64)> {
        let mask = self.vmcs.cr4_guest_host_mask;
        let shadow = self.vmcs.cr4_read_shadow;
        if (mask & shadow) != (mask & val) {
            self.vmx_vmexit(
                VmxVmexitReason::CrAccess,
                4 | ((u64::from(gpr) & 0xF) << 8),
            )?;
            return Ok((true, val));
        }
        let cur = self.cr4.get();
        Ok((false, (cur & mask) | (val & !mask)))
    }

    /// CLTS intercept — Bochs vmexit.cc VMexit_CLTS. The TS bit (CR0 bit 3)
    /// is masked: when both the host-mask and read-shadow have TS=1, CLTS
    /// triggers a VMEXIT with `access type = 2`. Independently, if the host
    /// pinned TS=0 in the shadow while masking it, CLTS is suppressed and
    /// CR0.TS is left untouched.
    ///
    /// Returns `(exited, suppress_clear)`:
    /// - `exited`: caller must return `Ok(())` immediately.
    /// - `suppress_clear`: when true (and `exited` is false), the handler
    ///   must skip the CR0.TS clear but otherwise complete normally.
    pub(super) fn vmexit_check_clts(&mut self) -> Result<(bool, bool)> {
        let mask = self.vmcs.cr0_guest_host_mask;
        let shadow = self.vmcs.cr0_read_shadow;
        if (mask & shadow & 0x8) != 0 {
            // Access type 2 (CLTS) << 4. CR# = 0, GPR = 0.
            self.vmx_vmexit(VmxVmexitReason::CrAccess, 2u64 << 4)?;
            return Ok((true, false));
        }
        let suppress = (mask & 0x8) != 0 && (shadow & 0x8) == 0;
        Ok((false, suppress))
    }

    /// LMSW intercept — Bochs vmexit.cc VMexit_LMSW. LMSW touches only the
    /// low 4 bits of CR0; an attempted change to a masked bit (relative to
    /// the read shadow) triggers a VMEXIT. Bit 0 (PE) is one-way: a 0→1
    /// transition is significant only when shadow.PE=0. Bits 1..3 use plain
    /// equality against the masked shadow.
    ///
    /// Returns `(exited, effective_msw)`. When `exited` is false the caller
    /// must build the merged value `(cr0 & mask) | (msw & !mask)` for the
    /// low 4 bits using the returned `effective_msw`.
    pub(super) fn vmexit_check_lmsw(
        &mut self,
        msw: u32,
        is_memory: bool,
        laddr: u64,
    ) -> Result<(bool, u32)> {
        let mask = (self.vmcs.cr0_guest_host_mask as u32) & 0xF;
        let shadow = self.vmcs.cr0_read_shadow as u32;
        let mut vmexit = false;
        if (mask & msw & 0x1) != 0 && (shadow & 0x1) == 0 {
            vmexit = true;
        }
        if (mask & shadow & 0xE) != (mask & msw & 0xE) {
            vmexit = true;
        }
        if vmexit {
            let mut qual: u64 = (3u64 << 4) | (u64::from(msw) << 16);
            if is_memory {
                qual |= 1 << 6;
                self.vmcs.guest_linear_addr = laddr;
            }
            self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
            return Ok((true, msw));
        }
        // Merge: keep masked bits at their CR0 value.
        let cr0_lo = self.cr0.get32() & 0xF;
        let mask_lo = mask & 0xF;
        let merged = (cr0_lo & mask_lo) | (msw & !mask_lo);
        Ok((false, merged & 0xF))
    }

    /// MOV to/from DR intercept — Bochs vmexit.cc VMexit_DR_Access.
    /// Qualification layout for DR-access VM-exits:
    ///   [3:0]   DR number
    ///   [4]     direction: 0 = MOV to DR, 1 = MOV from DR
    ///   [11:8]  source/destination GPR
    pub(super) fn vmexit_check_dr_access(
        &mut self,
        read: bool,
        dr: u8,
        gpr: u8,
    ) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_DRX_ACCESS_VMEXIT == 0 {
            return Ok(false);
        }
        let mut qual: u64 = (u64::from(dr) & 0xF) | ((u64::from(gpr) & 0xF) << 8);
        if read {
            qual |= 1 << 4;
        }
        self.vmx_vmexit(VmxVmexitReason::DrAccess, qual)?;
        Ok(true)
    }

    /// Hardware-exception intercept — Bochs vmexit.cc VMexit_Event (the
    /// `BX_HARDWARE_EXCEPTION` branch). Consults `exception_bitmap` for every
    /// vector; for #PF the decision additionally depends on the error-code
    /// mask/match pair. If the guest takes the VMEXIT, the VMCS interruption
    /// info + error code are recorded and the qualification encodes CR2 for
    /// #PF or the masked `debug_trap` for #DB. Returns `Ok(true)` when the
    /// VMEXIT is taken.
    pub(super) fn vmexit_check_exception(
        &mut self,
        vector: u32,
        error_code: u32,
        push_error: bool,
    ) -> Result<bool> {
        // Bochs vmexit.cc VMexit_Event: #PF does `(err & pf_mask) == pf_match`
        // XNOR'd with the bitmap; all other vectors just look up the bit.
        let pf_vector = Exception::Pf as u32;
        let db_vector = Exception::Db as u32;
        let vmexit = if vector == pf_vector {
            let err_match = (error_code & self.vmcs.vm_pf_mask) == self.vmcs.vm_pf_match;
            let bitmap = (self.vmcs.exception_bitmap >> pf_vector) & 1 != 0;
            err_match == bitmap
        } else {
            (self.vmcs.exception_bitmap >> vector) & 1 != 0
        };
        if !vmexit {
            return Ok(false);
        }

        // Qualification per Bochs: CR2 for #PF, masked debug_trap for #DB,
        // 0 otherwise. On #DB, Bochs also clears debug_trap.
        let qualification = if vector == pf_vector {
            self.cr2
        } else if vector == db_vector {
            let q = self.debug_trap & 0x0000_600F;
            self.debug_trap = 0;
            u64::from(q)
        } else {
            0
        };

        // Interruption info layout (Bochs vmexit.cc):
        //   [7:0] vector, [10:8] type (3 = hardware exception), [11] error
        //   code delivered, [31] valid. Bits 12 (NMI-unblock) and 13 (FRED
        //   nested) are not populated yet.
        const BX_HARDWARE_EXCEPTION: u32 = 3;
        let mut intr_info = vector | (BX_HARDWARE_EXCEPTION << 8) | (1 << 31);
        if push_error {
            intr_info |= 1 << 11;
        }
        self.vmcs.exit_intr_info = intr_info;
        self.vmcs.exit_intr_error_code = error_code;

        self.vmx_vmexit(VmxVmexitReason::ExceptionNmi, qualification)?;
        Ok(true)
    }

    /// Task-switch intercept — unconditional in VMX (Bochs vmexit.cc
    /// VMexit_TaskSwitch has no control-bit gate; fires whenever a task
    /// switch is attempted from the guest). Qualification layout matches
    /// Bochs: `tss_selector | (source << 30)`.
    pub(super) fn vmexit_check_task_switch(
        &mut self,
        tss_selector: u16,
        source: u32,
    ) -> Result<bool> {
        let qual = u64::from(tss_selector) | (u64::from(source) << 30);
        self.vmx_vmexit(VmxVmexitReason::TaskSwitch, qual)?;
        Ok(true)
    }

    /// I/O port bitmap walker — Bochs vmexit.cc VMexit_IO. The pair of 4 KiB
    /// bitmaps `io_bitmap_addr[0..1]` covers ports `0x0000..0x7FFF` and
    /// `0x8000..0xFFFF` respectively. A multi-byte access whose port range
    /// straddles the 0x8000 split must consult both bitmaps; access ranges
    /// wrapping past 0xFFFF always force a VMEXIT.
    fn io_bitmap_says_vmexit(&mut self, port: u16, len: u32) -> bool {
        // Wrap-around forces VMEXIT (Bochs guard).
        let end = u32::from(port) + len;
        if end > 0x10000 {
            return true;
        }

        let port_lo = u32::from(port) & 0x7FFF;
        let bitmap = if (port_lo + len) > 0x8000 {
            // Straddles the bitmap-A/B boundary. Bochs reads byte 0xFFF of
            // bitmap A and byte 0x000 of bitmap B.
            let pa = self.vmcs.io_bitmap_addr[0] + 0xFFF;
            let pb = self.vmcs.io_bitmap_addr[1];
            let b0 = u16::from(self.read_physical_byte(pa));
            let b1 = u16::from(self.read_physical_byte(pb));
            (b1 << 8) | b0
        } else {
            // read_physical_byte cannot cross 4 KiB; do two single-byte reads.
            let which = usize::from((port >> 15) & 1);
            let pa = self.vmcs.io_bitmap_addr[which] + u64::from(port_lo / 8);
            let b0 = u16::from(self.read_physical_byte(pa));
            let b1 = u16::from(self.read_physical_byte(pa + 1));
            (b1 << 8) | b0
        };

        let len_mask = (1u16 << len) - 1;
        let mask = len_mask << (port & 7);
        (bitmap & mask) != 0
    }

    /// Decide whether a guest I/O access takes a VMEXIT — Bochs vmexit.cc
    /// VMexit_IO common predicate. Bitmap path beats the unconditional
    /// `IO_VMEXIT` bit when both might apply.
    fn io_should_vmexit(&mut self, port: u16, size: u32) -> bool {
        let ctls = self.proc_based_ctls1();
        if ctls & VMX_VM_EXEC_CTRL1_IO_BITMAPS != 0 {
            self.io_bitmap_says_vmexit(port, size)
        } else if ctls & VMX_VM_EXEC_CTRL1_IO_VMEXIT != 0 {
            true
        } else {
            false
        }
    }

    /// Build the I/O exit qualification — Bochs vmexit.cc VMexit_IO:
    ///   [2:0]   access size - 1     (packed value)
    ///   [3]     direction (0 = OUT, 1 = IN)
    ///   [4]     string instruction
    ///   [5]     REP prefix
    ///   [6]     operand encoding (0 = DX, 1 = immediate)
    ///   [31:16] port number          (packed value)
    fn io_qualification(
        port: u16,
        size: u32,
        direction_in: bool,
        string: bool,
        rep: bool,
        imm: bool,
    ) -> u64 {
        let mut flags = IoExitQual::empty();
        flags.set(IoExitQual::PORT_IN, direction_in);
        flags.set(IoExitQual::STRING, string);
        flags.set(IoExitQual::REP, rep);
        flags.set(IoExitQual::IMMEDIATE, imm);
        let mut qual = flags.bits();
        qual |= u64::from(size.saturating_sub(1) & 0x7);
        qual |= u64::from(port) << 16;
        qual
    }

    /// IN/OUT (non-string) intercept — Bochs vmexit.cc VMexit_IO with the
    /// `BX_IA_IN_*` / `BX_IA_OUT_*` cases that don't touch GUEST_LINEAR_ADDR
    /// or INSTRUCTION_INFO. Caller passes `imm=true` for the immediate-port
    /// variants (`IN AL, ib`, `OUT ib, AL`, …).
    pub(super) fn vmexit_check_io(
        &mut self,
        port: u16,
        size: u32,
        direction_in: bool,
        imm: bool,
    ) -> Result<bool> {
        if !self.io_should_vmexit(port, size) {
            return Ok(false);
        }
        let qual = Self::io_qualification(port, size, direction_in, false, false, imm);
        self.vmx_vmexit(VmxVmexitReason::IoInstruction, qual)?;
        Ok(true)
    }

    /// INS/OUTS string-form intercept — Bochs vmexit.cc VMexit_IO with the
    /// `BX_IA_REP_INSx` / `BX_IA_REP_OUTSx` cases. Writes the buffer linear
    /// address into `VMCS_GUEST_LINEAR_ADDR` and a packed
    /// `(seg << 15) | (as64 ? 1<<8 : 0) | (as32 ? 1<<7 : 0)` value into
    /// `VMCS_32BIT_VMEXIT_INSTRUCTION_INFO`. `port_in` semantics use ES:RDI;
    /// `port_out` (OUTS) uses the prefix segment with RSI.
    pub(super) fn vmexit_check_io_string(
        &mut self,
        port: u16,
        size: u32,
        direction_in: bool,
        rep: bool,
        linear_addr: u64,
        seg: u8,
        as64: bool,
        as32: bool,
    ) -> Result<bool> {
        if !self.io_should_vmexit(port, size) {
            return Ok(false);
        }
        let qual = Self::io_qualification(port, size, direction_in, true, rep, false);
        self.vmcs.guest_linear_addr = linear_addr;
        let mut info: u32 = u32::from(seg) << 15;
        if as64 {
            info |= 1 << 8;
        } else if as32 {
            info |= 1 << 7;
        }
        self.vmcs.exit_instruction_info = info;
        self.vmx_vmexit(VmxVmexitReason::IoInstruction, qual)?;
        Ok(true)
    }
}
