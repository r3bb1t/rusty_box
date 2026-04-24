// Intel VT-x (VMX) — Bochs cpu/vmx.cc.
//
// Session 3 scope: VMX operation-mode entry/exit (VMXON/VMXOFF), the
// flag-based success/failure helpers (VMsucceed / VMfailInvalid / VMfail),
// the IA32_FEATURE_CONTROL and IA32_VMX_* MSR surface, and the VMX
// instruction-error enum. VMCS-manipulation opcodes (VMCLEAR / VMPTRLD /
// VMREAD / VMWRITE / VMLAUNCH / VMRESUME) and the VM-entry/exit state
// machine are deferred to Sessions 4+.

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

#[derive(Debug, Default)]
pub struct BxVmcs {
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
            // Bochs: VMwrite32(VMCS_32BIT_INSTRUCTION_ERROR, error).
            // 0x4400 is the architecturally-assigned encoding.
            if let Some(off) = Self::vmcs_field_offset(0x4400) {
                self.mem_write_dword(self.vmcsptr + off, error as u32);
            }
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
    // VMREAD / VMWRITE — minimal VMCS field access.
    //
    // Bochs' vmx_map drives a per-field byte offset into the 4 KiB VMCS
    // region. That table isn't ported yet; for now we support the two
    // architecturally required fields Session 3's VMfail writes touch:
    // VMCS_LAUNCH_STATE (our stash at offset 4) and
    // VMCS_32BIT_INSTRUCTION_ERROR (encoding 0x4400, stashed at offset 8).
    // Any other encoding fails with VMXERR_UNSUPPORTED_VMCS_COMPONENT_ACCESS
    // so guests see a well-defined error instead of silent corruption.
    // =========================================================================

    /// Return the byte offset inside the VMCS where a given encoding lives.
    /// `None` means the encoding is not yet supported.
    fn vmcs_field_offset(encoding: u32) -> Option<u64> {
        match encoding {
            0x4400 => Some(8), // VMCS_32BIT_INSTRUCTION_ERROR
            _ => None,
        }
    }

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

        if let Some(off) = Self::vmcs_field_offset(encoding) {
            let v = self.mem_read_dword(self.vmcsptr + off);
            self.vmsucceed();
            Ok(v as u64)
        } else {
            self.vmfail(VmxErr::UnsupportedVmcsComponentAccess);
            Ok(0)
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

        if let Some(off) = Self::vmcs_field_offset(encoding) {
            self.mem_write_dword(self.vmcsptr + off, value as u32);
            self.vmsucceed();
            Ok(())
        } else {
            self.vmfail(VmxErr::UnsupportedVmcsComponentAccess);
            Ok(())
        }
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
}
