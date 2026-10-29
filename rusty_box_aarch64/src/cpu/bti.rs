//! Branch target identification: FEAT_BTI.
//!
//! Provenance (R7): the Arm A64 ISA XML (release 2026-09), the pages of
//! BTI, BR, BLR and their authenticating forms, BRK, PACIASP and PACIBSP,
//! and the shared pseudocode `BranchTargetCheck`, `BTypeCompatible_BTI`,
//! `BTypeCompatible_PAC`, `EndOfDecode` (the check comes before an
//! UNDEFINED or NOP decode takes effect), `AArch64_S1CheckPermissions`
//! (`InGuardedPage`), `AArch64_TakeException` and `SetPSTATEFromPSR`
//! (BTYPE across an exception and its return).

use super::exception::{Exception, ExceptionClass};
use super::exec::bits;
use super::features::Feature;
use super::isa::{self, BranchTarget};
use super::{Btype, Cpu, El, Levels};

impl<L: Levels> Cpu<L> {
    /// Arm `BranchTargetCheck`, as `EndOfDecode` makes it for the
    /// instruction `insn` while PSTATE.BTYPE is not 00: in a guarded page
    /// (`in_guarded_page`, from the instruction's fetch) the instruction must
    /// accept the branch that led to it, or it raises the Branch Target
    /// exception with PSTATE.BTYPE unchanged. It then leaves PSTATE.BTYPE 00
    /// (`BTypeNext`), unless it is a branch that sets another.
    pub(crate) fn branch_target_check(
        &mut self,
        in_guarded_page: bool,
        insn: u32,
    ) -> Result<(), Exception> {
        let btype = self.pstate.btype;
        if in_guarded_page && !self.accepts(insn, btype) {
            return Err(Exception::new(
                ExceptionClass::BranchTarget,
                btype.bits() as u32,
            ));
        }
        self.pstate.btype = Btype::None;
        Ok(())
    }

    /// Arm `BTypeCompatible` of `insn` under `btype`, as its decode sets it
    /// ([`isa::Encoding::branch_target`]). A hint of a feature the
    /// processor lacks decodes as a NOP and accepts nothing.
    fn accepts(&self, insn: u32, btype: Btype) -> bool {
        let Some(encoding) = isa::decode(insn) else {
            return false;
        };
        if !self.implements(encoding.feature()) {
            return false;
        }
        match encoding.branch_target() {
            BranchTarget::Bti => match bits(insn, 7, 6) {
                0b00 => btype == Btype::None,
                0b01 => btype != Btype::Jump,
                0b10 => btype != Btype::Call,
                _ => true,
            },
            // `BTypeCompatible_PACIXSP`: `SCTLR_ELx()[35]` (`BT0`) at EL0,
            // bit 36 (`BT1` of `SCTLR_EL1`, `BT` of `SCTLR_EL2`) otherwise,
            // of the current regime's register.
            BranchTarget::PacSp => {
                let bt = match self.pstate.el {
                    El::El0 => 35,
                    El::El1 | El::El2 => 36,
                };
                btype != Btype::Jump || (self.regime_sctlr(self.regime()) >> bt) & 1 == 0
            }
            BranchTarget::Any => true,
            BranchTarget::Refused => false,
        }
    }

    /// `BTypeNext` of BR, BRAA, BRAAZ, BRAB and BRABZ through `Rn`: 11 from
    /// a guarded page unless `Rn` is X16 or X17, 01 otherwise.
    pub(crate) fn set_btype_jump(&mut self, in_guarded_page: bool, rn: u32) {
        if self.implements(Feature::Bti) {
            self.pstate.btype = if in_guarded_page && rn != 16 && rn != 17 {
                Btype::Jump
            } else {
                Btype::JumpOrCall
            };
        }
    }

    /// `BTypeNext` of BLR, BLRAA, BLRAAZ, BLRAB and BLRABZ: 10.
    pub(crate) fn set_btype_call(&mut self) {
        if self.implements(Feature::Bti) {
            self.pstate.btype = Btype::Call;
        }
    }
}
