//! The Scalable Matrix Extension's architectural state and the rules that
//! guard it (FEAT_SME): Streaming SVE mode and the ZA array (`PSTATE.SM` and
//! `PSTATE.ZA`, which `SVCR` reads and writes), the scalable vector,
//! predicate and ZA storage they make reachable, the Streaming SVE vector
//! length, and the enable checks and traps every SME instruction and
//! register goes through.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09 (Arm ARM M.d), its
//! shared pseudocode (`aarch64/functions/sve`, `aarch64/functions/sme`,
//! `aarch64/exceptions/traps`) and the instruction pages named per item.
//! Each function here names the pseudocode function it is. Where the
//! pseudocode leaves a choice to the implementation, the choice made is
//! stated where it is made: the upper bits of a register written at a
//! shorter length are zeroed (`Unpredictable_SVEZEROUPPER` and
//! `Unpredictable_SMEZEROUPPER` taken as TRUE), as is ZA when `PSTATE.ZA`
//! is cleared (`ResetSMEState('0')` leaves it ARBITRARY).

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

#[cfg(test)]
#[path = "../../tests/unit/sme.rs"]
mod tests;

use super::exception::{Exception, ExceptionClass};
use super::exec::{bits, ExecResult, Fault, Flow};
use super::features::{Feature, VecLen};
use super::state::FPSR_MASK;
use super::sve::{Pv, Zv};
use super::{Cpu, El};

/// `MAX_VL` in bytes: the storage behind every Z register and every
/// horizontal vector of ZA, whatever this processor's lengths are.
const MAX_VL_BYTES: usize = VecLen::MAX.bytes();

/// `MAX_PL` in bytes: the storage behind every predicate register and FFR.
const MAX_PL_BYTES: usize = VecLen::MAX.predicate_bytes();

/// The bytes of a Z register above its low 128 bits, which are `V<n>`
/// (`Cpu::v`).
const Z_UPPER_BYTES: usize = MAX_VL_BYTES - 16;

/// `FPSR` as `ResetSVEState` leaves it: QC and every cumulative exception
/// flag set.
const FPSR_AFTER_SVE_RESET: u64 = 0x0800_009F;

/// `PSTATE.SM` and `PSTATE.ZA`: the `SVCR` register's two fields. They are
/// not part of `SPSR_ELx`, so taking or returning from an exception leaves
/// them as they are; only [`Cpu::set_pstate_sm`] and [`Cpu::set_pstate_za`]
/// change them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Svcr {
    /// Streaming SVE mode.
    pub(crate) sm: bool,
    /// ZA storage is enabled.
    pub(crate) za: bool,
}

impl Svcr {
    /// The state out of reset: neither.
    pub(crate) const RESET: Self = Self { sm: false, za: false };

    /// The register's value: SM in bit 0, ZA in bit 1.
    pub(crate) const fn bits(self) -> u64 {
        ((self.za as u64) << 1) | self.sm as u64
    }
}

/// The scalable registers beyond the SIMD&FP ones: the upper bits of
/// Z0-Z31, P0-P15, FFR and the ZA array, each held at `MAX_VL` and read at
/// whatever length is current, as the Arm ARM's `_Z`, `_P`, `_FFR` and
/// `_ZA` are.
pub(crate) struct ScalableRegs {
    /// Bytes 16 and up of Z0-Z31, little-endian like the vectors.
    z_upper: [[u8; Z_UPPER_BYTES]; 32],
    /// Bit n is clear when `z_upper[n]` is all zero: a write of `V<n>`, which
    /// zeroes everything above bit 127, clears the bit instead of the
    /// bytes.
    z_upper_live: u32,
    p: [[u8; MAX_PL_BYTES]; 16],
    ffr: [u8; MAX_PL_BYTES],
    /// `_ZA`: horizontal vector n of the array is `za[n]`.
    za: [[u8; MAX_VL_BYTES]; MAX_VL_BYTES],
}

impl ScalableRegs {
    /// Every bit zero.
    pub(crate) const ZERO: Self = Self {
        z_upper: [[0; Z_UPPER_BYTES]; 32],
        z_upper_live: 0,
        p: [[0; MAX_PL_BYTES]; 16],
        ffr: [0; MAX_PL_BYTES],
        za: [[0; MAX_VL_BYTES]; MAX_VL_BYTES],
    };
}

/// Arm `SMEExceptionType`: why an SME trap (EC 0x1D) is taken, reported in
/// `ESR_ELx.ISS.SMTC` (`SMEAccessTrap`).
///
/// Closed (R5): the types this processor raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmeTrap {
    /// SMTC 0b000: SME, or a register of it, is disabled by
    /// `CPACR_EL1.SMEN`.
    AccessTrap,
    /// SMTC 0b001: an instruction that is illegal in Streaming SVE mode.
    Streaming,
    /// SMTC 0b010: an instruction that is illegal outside Streaming SVE
    /// mode, which every SVE instruction is on a processor without
    /// FEAT_SVE.
    NotStreaming,
    /// SMTC 0b011: an instruction that accesses ZA while `PSTATE.ZA` is
    /// 0.
    InactiveZa,
}

impl SmeTrap {
    const fn smtc(self) -> u32 {
        match self {
            Self::AccessTrap => 0b000,
            Self::Streaming => 0b001,
            Self::NotStreaming => 0b010,
            Self::InactiveZa => 0b011,
        }
    }
}

/// Which of `PSTATE.SM` and `PSTATE.ZA` an `MSR SVCR*, #imm` writes
/// (`PSTATEField_SVCRSM`, `_SVCRZA`, `_SVCRSMZA`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SvcrField {
    Sm,
    Za,
    SmZa,
}

impl Cpu {
    /// Arm `V{128}(n) = value`: the SIMD&FP register is the low 128 bits of
    /// `Z<n>`, and a write of it zeroes the bits above
    /// (`Unpredictable_SVEZEROUPPER` taken as TRUE). Every write of a V
    /// register goes through here.
    #[inline]
    pub(crate) fn write_v(&mut self, n: usize, value: u128) {
        self.v[n] = value;
        self.scalable.z_upper_live &= !(1 << n);
    }

    /// Arm `Z{VL}(n)`: the low `vl` bits of `Z<n>`, whose low 128 are
    /// `V<n>`.
    pub(crate) fn z(&self, n: u32, vl: VecLen) -> Zv {
        let n = n as usize;
        let upper = vl.bytes() - 16;
        if self.scalable.z_upper_live & (1 << n) != 0 {
            Zv::from_parts(self.v[n], &self.scalable.z_upper[n][..upper])
        } else {
            Zv::from_parts(self.v[n], &[])
        }
    }

    /// Arm `Z{VL}(n) = value`: the bits above `vl` zeroed
    /// (`Unpredictable_SVEZEROUPPER` taken as TRUE).
    pub(crate) fn set_z(&mut self, n: u32, vl: VecLen, value: &Zv) {
        let n = n as usize;
        let upper = vl.bytes() - 16;
        self.v[n] = value.low128();
        let s = &mut self.scalable;
        s.z_upper[n][..upper].copy_from_slice(&value.upper()[..upper]);
        s.z_upper[n][upper..].fill(0);
        s.z_upper_live |= 1 << n;
    }

    /// Arm `P{PL}(n)`: the low `PL` bits of `P<n>` for vectors of length
    /// `vl`.
    pub(crate) fn p(&self, n: u32, vl: VecLen) -> Pv {
        Pv::from_bytes(&self.scalable.p[n as usize][..vl.predicate_bytes()])
    }

    /// Arm `P{PL}(n) = value`: the bits above `PL` zeroed
    /// (`Unpredictable_SVEZEROUPPER` taken as TRUE).
    pub(crate) fn set_p(&mut self, n: u32, vl: VecLen, value: &Pv) {
        let len = vl.predicate_bytes();
        let p = &mut self.scalable.p[n as usize];
        p[..len].copy_from_slice(&value.as_bytes()[..len]);
        p[len..].fill(0);
    }

    /// Whether this processor implements `feature`.
    #[inline]
    pub(crate) fn implements(&self, feature: Feature) -> bool {
        self.imp.has(feature)
    }

    /// Arm `IsSMEEnabled(el)` for EL0 and EL1, the only levels:
    /// `CPACR_EL1.SMEN` allows SME there.
    pub(crate) fn sme_enabled_at(&self, el: El) -> bool {
        match (self.sys.cpacr_el1 >> 24) & 3 {
            0b11 => true,
            0b01 => el == El::El1,
            _ => false,
        }
    }

    /// Arm `IsSVEEnabled(el)` on a processor without FEAT_SVE: SVE
    /// registers are reachable only in Streaming SVE mode, where SME's
    /// enable governs them.
    fn sve_enabled_at(&self, el: El) -> bool {
        self.svcr.sm && self.sme_enabled_at(el)
    }

    /// The SME trap of type `kind`: `SMEAccessTrap`, taken to EL1 from either
    /// level, the instruction's own address the preferred return.
    pub(crate) fn sme_trap(&self, kind: SmeTrap) -> Fault {
        Exception::new(ExceptionClass::SmeAccess, kind.smtc()).into()
    }

    /// Arm `CheckSMEAccess`: the check on the SME System registers and on
    /// `MSR SVCR*, #imm`. `SMEN` = `01` disables only EL0.
    pub(crate) fn check_sme_access(&self) -> Result<(), Fault> {
        if self.sme_enabled_at(self.pstate.el) {
            Ok(())
        } else {
            Err(self.sme_trap(SmeTrap::AccessTrap))
        }
    }

    /// Arm `CheckSMEEnabled`: SME's enable first, then the SIMD&FP enable,
    /// each with its own trap.
    pub(crate) fn check_sme_enabled(&self) -> Result<(), Fault> {
        self.check_sme_access()?;
        self.fp_check()
    }

    /// Arm `CheckSMEAndZAEnabled`, and `CurrentSVL` after it: SME's enable
    /// checks, then the SME trap when ZA storage is disabled. ZERO, LDR
    /// and STR of ZA take it, in or out of Streaming SVE mode.
    pub(crate) fn check_sme_and_za_enabled(&self) -> Result<VecLen, Fault> {
        self.check_sme_enabled()?;
        if !self.svcr.za {
            return Err(self.sme_trap(SmeTrap::InactiveZa));
        }
        Ok(self.current_svl())
    }

    /// Arm `CheckStreamingSVEEnabled`, and `CurrentVL` after it, which in
    /// Streaming SVE mode is the Streaming SVE vector length: SME's enable
    /// checks, then the SME trap outside Streaming SVE mode.
    pub(crate) fn check_streaming_sve_enabled(&self) -> Result<VecLen, Fault> {
        self.check_sme_enabled()?;
        if !self.svcr.sm {
            return Err(self.sme_trap(SmeTrap::NotStreaming));
        }
        Ok(self.current_svl())
    }

    /// Arm `CheckStreamingSVEAndZAEnabled`, and `CurrentVL` after it:
    /// [`Cpu::check_streaming_sve_enabled`], then the SME trap when ZA
    /// storage is disabled. The instructions on ZA other than ZERO, LDR
    /// and STR take it: they execute only in Streaming SVE mode.
    pub(crate) fn check_streaming_sve_and_za_enabled(&self) -> Result<VecLen, Fault> {
        let vl = self.check_streaming_sve_enabled()?;
        if !self.svcr.za {
            return Err(self.sme_trap(SmeTrap::InactiveZa));
        }
        Ok(vl)
    }

    /// Arm `ZAvector{SVL}(index)`, as its bytes: horizontal vector `index`
    /// of the ZA array, at the Streaming SVE vector length `svl`.
    pub(crate) fn za_row(&self, index: usize, svl: VecLen) -> &[u8] {
        &self.scalable.za[index][..svl.bytes()]
    }

    /// Arm `ZAvector{SVL}(index)`.
    pub(crate) fn za_vector(&self, index: usize, svl: VecLen) -> Zv {
        Zv::from_bytes(self.za_row(index, svl))
    }

    /// Arm `ZAvector{SVL}(index) = value`: the bits above `svl` zeroed
    /// (`Unpredictable_SMEZEROUPPER` taken as TRUE).
    pub(crate) fn set_za_vector(&mut self, index: usize, svl: VecLen, value: &Zv) {
        let len = svl.bytes();
        self.set_za_bytes(index, svl, 0, &value.as_bytes()[..len]);
    }

    /// Arm `ZAvector{SVL}(index) = value` of the vector with its bytes from
    /// `at` replaced by `bytes`: the bits above `svl` zeroed, as
    /// [`Cpu::set_za_vector`] leaves them.
    pub(crate) fn set_za_bytes(&mut self, index: usize, svl: VecLen, at: usize, bytes: &[u8]) {
        let row = &mut self.scalable.za[index];
        row[at..at + bytes.len()].copy_from_slice(bytes);
        row[svl.bytes()..].fill(0);
    }

    /// Arm `CurrentSVL`: `SMCR_EL1.LEN` reduced to a supported length, at
    /// EL0 and EL1 alike (there is no EL2 or EL3 to constrain it).
    pub(crate) fn current_svl(&self) -> VecLen {
        VecLen::from_len_field(self.sys.smcr_el1, self.imp.max_svl)
    }

    /// Arm `SetPSTATE_SM`: entering or leaving Streaming SVE mode resets the
    /// SVE state.
    pub(crate) fn set_pstate_sm(&mut self, value: bool) {
        if self.svcr.sm != value {
            self.reset_sve_state();
            self.svcr.sm = value;
        }
    }

    /// Arm `SetPSTATE_ZA`: enabling or disabling ZA storage resets it.
    pub(crate) fn set_pstate_za(&mut self, value: bool) {
        if self.svcr.za != value {
            self.reset_sme_state();
            self.svcr.za = value;
        }
    }

    /// Arm `ResetSVEState`: every Z, P and FFR bit zero and `FPSR` set to
    /// 0x0800_009F. (`FPMR`, which it also zeroes, is FEAT_FPMR's.)
    fn reset_sve_state(&mut self) {
        self.v = [0; 32];
        let s = &mut self.scalable;
        s.z_upper_live = 0;
        s.p = [[0; MAX_PL_BYTES]; 16];
        s.ffr = [0; MAX_PL_BYTES];
        self.fpsr = FPSR_AFTER_SVE_RESET & FPSR_MASK;
    }

    /// Arm `ResetSMEState`: ZA zeroed, on enabling as the pseudocode
    /// requires and on disabling, where it leaves ZA ARBITRARY. Only the
    /// `MaxImplementedSVL` square of the storage is ever written.
    fn reset_sme_state(&mut self) {
        let n = self.imp.max_svl.bytes();
        for row in self.scalable.za[..n].iter_mut() {
            row[..n].fill(0);
        }
    }

    /// Arm `MaybeZeroSVEUppers(EL1)`, which `AArch64_TakeException` does
    /// before an exception from EL0 to EL1 changes `PSTATE`: when SVE is
    /// enabled at EL1 and SIMD&FP at EL0, the bits of every Z, P and FFR
    /// register above the length EL0 uses, and with ZA enabled the bits of
    /// ZA outside the current SVL square, are zeroed
    /// (`Unpredictable_SVEZEROUPPER` and `_SMEZEROUPPER` taken as TRUE).
    pub(crate) fn maybe_zero_sve_uppers(&mut self) {
        // PSTATE.EL is still EL0 here, so `fp_enabled` is IsFPEnabled(EL0).
        if self.pstate.el != El::El0 || !self.sve_enabled_at(El::El1) || !self.fp_enabled() {
            return;
        }
        let vl = if self.sve_enabled_at(El::El0) { self.current_svl() } else { VecLen::MIN };
        let svl = self.current_svl().bytes();
        let all = self.imp.max_svl.bytes();
        let s = &mut self.scalable;
        match vl.bytes().checked_sub(16) {
            Some(0) | None => s.z_upper_live = 0,
            Some(kept) => {
                for z in s.z_upper.iter_mut() {
                    z[kept..].fill(0);
                }
            }
        }
        for p in s.p.iter_mut() {
            p[vl.predicate_bytes()..].fill(0);
        }
        s.ffr[vl.predicate_bytes()..].fill(0);
        if self.svcr.za {
            for (n, row) in s.za[..all].iter_mut().enumerate() {
                let from = if n < svl { svl } else { 0 };
                row[from..].fill(0);
            }
        }
    }

    /// MSR SVCRSM, SVCRZA and SVCRSMZA, `#imm` (SMSTART and SMSTOP are its
    /// aliases): `MSR (immediate)` with `op1 = 011, op2 = 011` and the field
    /// in `CRm<3:1>`. The decode makes them UNDEFINED without FEAT_SME, and
    /// every other `CRm<3:1>`; the operation is `CheckSMEAccess`, then
    /// `SetPSTATE_SM` and `SetPSTATE_ZA` with `CRm<0>`.
    pub(crate) fn exec_msr_svcr(&mut self, pc: u64, insn: u32) -> ExecResult {
        let crm = bits(insn, 11, 8);
        let field = match crm >> 1 {
            0b001 => SvcrField::Sm,
            0b010 => SvcrField::Za,
            0b011 => SvcrField::SmZa,
            _ => return self.undefined(pc, insn),
        };
        if !self.implements(Feature::Sme) {
            return self.undefined(pc, insn);
        }
        self.check_sme_access()?;
        let value = crm & 1 != 0;
        match field {
            SvcrField::Sm => self.set_pstate_sm(value),
            SvcrField::Za => self.set_pstate_za(value),
            SvcrField::SmZa => {
                self.set_pstate_sm(value);
                self.set_pstate_za(value);
            }
        }
        Ok(Flow::Next)
    }

    /// RDSVL: `00000100 1 0 1 11111 01011 1 imm6 Rd` — `Rd` (XZR at 31)
    /// becomes `imm6` times the Streaming SVE vector length in bytes.
    pub(crate) fn exec_rdsvl(&mut self, insn: u32) -> ExecResult {
        self.check_sme_enabled()?;
        self.read_vector_length(insn, self.current_svl());
        Ok(Flow::Next)
    }

    /// ADDSVL and ADDSPL: `00000100 0 op 1 Rn 01011 1 imm6 Rd` — `Rd|SP`
    /// becomes `Rn|SP` plus `imm6` times the Streaming SVE vector length in
    /// bytes (ADDSVL, `op` = 0) or the predicate length in bytes (ADDSPL,
    /// `op` = 1, `SVL DIV 64`).
    pub(crate) fn exec_addsvl(&mut self, insn: u32) -> ExecResult {
        self.check_sme_enabled()?;
        self.add_vector_length(insn, self.current_svl());
        Ok(Flow::Next)
    }

    /// The SME state a snapshot carries, for a processor implementing
    /// FEAT_SME: `SVCR`, then Z (above V), P, FFR and ZA at
    /// `MaxImplementedSVL`.
    pub(crate) fn save_sme<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        if !self.implements(Feature::Sme) {
            return Ok(());
        }
        let vl = self.imp.max_svl;
        let s = &self.scalable;
        w.write_u8(self.svcr.bits() as u8)?;
        for (n, z) in s.z_upper.iter().enumerate() {
            if s.z_upper_live & (1 << n) != 0 {
                w.write_bytes(&z[..vl.bytes() - 16])?;
            } else {
                w.write_bytes(&[0; Z_UPPER_BYTES][..vl.bytes() - 16])?;
            }
        }
        for p in s.p.iter() {
            w.write_bytes(&p[..vl.predicate_bytes()])?;
        }
        w.write_bytes(&s.ffr[..vl.predicate_bytes()])?;
        for row in s.za[..vl.bytes()].iter() {
            w.write_bytes(&row[..vl.bytes()])?;
        }
        Ok(())
    }

    /// The inverse of [`Cpu::save_sme`], read in place: a snapshot that
    /// fails part-way leaves the machine switched off, which is what makes
    /// a partly read register file harmless.
    pub(crate) fn restore_sme<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        if !self.implements(Feature::Sme) {
            return Ok(());
        }
        let svcr = r.read_u8()?;
        if svcr > 0b11 {
            return Err(SnapError::Invalid("SVCR in the snapshot sets a reserved bit"));
        }
        self.svcr = Svcr {
            sm: svcr & 1 != 0,
            za: svcr & 2 != 0,
        };
        let vl = self.imp.max_svl;
        let s = &mut self.scalable;
        s.z_upper_live = 0;
        for (n, z) in s.z_upper.iter_mut().enumerate() {
            r.read_bytes(&mut z[..vl.bytes() - 16])?;
            if z.iter().any(|&b| b != 0) {
                s.z_upper_live |= 1 << n;
            }
        }
        for p in s.p.iter_mut() {
            r.read_bytes(&mut p[..vl.predicate_bytes()])?;
        }
        r.read_bytes(&mut s.ffr[..vl.predicate_bytes()])?;
        for row in s.za[..vl.bytes()].iter_mut() {
            r.read_bytes(&mut row[..vl.bytes()])?;
        }
        Ok(())
    }
}
