//! Taking exceptions to EL1 and returning from them (Arm ARM D1.10–D1.11,
//! `AArch64.TakeException` and `AArch64.ExceptionReturn`).
//!
//! Every exception targets EL1: this core implements neither EL2 nor EL3, so
//! there is no routing to decide.

use super::features::Feature;
use super::{sctlr, Activity, Btype, Cpu, El, Pstate};
use crate::instrumentation::{ExceptionKind, ExceptionTaken};

/// `ESR_ELx.EC`, the exception classes this core can raise.
///
/// Closed (R5). "Lower" and "same" EL variants of the aborts are chosen when
/// the exception is taken, from the EL it is taken from, so a handler raises
/// the abort without knowing which it will be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExceptionClass {
    /// EC 0x00: UNDEFINED instruction, or an access to a register that does
    /// not exist at this EL.
    Unknown,
    /// EC 0x01: `WFI`/`WFE` trapped from EL0 by `SCTLR_EL1.nTWI`/`nTWE`.
    WfxTrap,
    /// EC 0x07: SIMD&FP access trapped by `CPACR_EL1.FPEN`.
    FpAccess,
    /// EC 0x0D: an indirect branch to an instruction in a guarded page
    /// that does not accept it (FEAT_BTI); `ISS[1:0]` is `PSTATE.BTYPE`.
    BranchTarget,
    /// EC 0x0E: execution with `PSTATE.IL` set.
    IllegalState,
    /// EC 0x15.
    Svc,
    /// EC 0x18: `MSR`/`MRS`/`SYS` trapped from EL0.
    SysRegTrap,
    /// EC 0x1C: a pointer authentication that failed, with FEAT_FPAC;
    /// `ISS[1:0]` says whether the key was a data key and which.
    PacFail,
    /// EC 0x1D: an SME instruction or register trapped by
    /// `CPACR_EL1.SMEN`, or an instruction illegal in the current streaming
    /// mode; `ISS.SMTC` says which.
    SmeAccess,
    /// EC 0x20 / 0x21.
    InstructionAbort,
    /// EC 0x22.
    PcAlignment,
    /// EC 0x24 / 0x25.
    DataAbort,
    /// EC 0x26.
    SpAlignment,
    /// EC 0x3C.
    Brk,
}

impl ExceptionClass {
    /// The `EC` value for an exception taken from `from`.
    pub const fn code(self, from: El) -> u64 {
        let lower = matches!(from, El::El0);
        match self {
            Self::Unknown => 0x00,
            Self::WfxTrap => 0x01,
            Self::FpAccess => 0x07,
            Self::BranchTarget => 0x0D,
            Self::IllegalState => 0x0E,
            Self::Svc => 0x15,
            Self::SysRegTrap => 0x18,
            Self::PacFail => 0x1C,
            Self::SmeAccess => 0x1D,
            Self::InstructionAbort => {
                if lower {
                    0x20
                } else {
                    0x21
                }
            }
            Self::PcAlignment => 0x22,
            Self::DataAbort => {
                if lower {
                    0x24
                } else {
                    0x25
                }
            }
            Self::SpAlignment => 0x26,
            Self::Brk => 0x3C,
        }
    }

    /// Whether the exception keeps `PSTATE.BTYPE` in `SPSR_EL1.BTYPE`
    /// (Arm `AArch64_TakeException`): the classes whose return can resume
    /// the branch target check, IRQ and FIQ too. Every other one saves 00,
    /// the choice `ConstrainUnpredictable(Unpredictable_ZEROBTYPE)` makes.
    const fn preserves_btype(self) -> bool {
        match self {
            Self::BranchTarget
            | Self::IllegalState
            | Self::InstructionAbort
            | Self::PcAlignment
            | Self::Brk => true,
            Self::Unknown
            | Self::WfxTrap
            | Self::FpAccess
            | Self::Svc
            | Self::SysRegTrap
            | Self::PacFail
            | Self::SmeAccess
            | Self::DataAbort
            | Self::SpAlignment => false,
        }
    }

    /// Whether the preferred return address is the next instruction rather
    /// than the one that raised the exception (`SVC` only, on this core).
    const fn returns_to_next(self) -> bool {
        matches!(self, Self::Svc)
    }
}

/// A synchronous exception raised by an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Exception {
    pub(crate) class: ExceptionClass,
    pub(crate) iss: u32,
    /// The faulting virtual address, for the classes that report one.
    pub(crate) far: Option<u64>,
}

impl Exception {
    #[inline]
    pub(crate) const fn new(class: ExceptionClass, iss: u32) -> Self {
        Self {
            class,
            iss,
            far: None,
        }
    }

    #[inline]
    pub(crate) const fn with_far(mut self, far: u64) -> Self {
        self.far = Some(far);
        self
    }
}

/// Which vector of the four in a vector-table quadrant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VectorKind {
    Synchronous,
    Irq,
    Fiq,
}

impl VectorKind {
    const fn offset(self) -> u64 {
        match self {
            Self::Synchronous => 0x000,
            Self::Irq => 0x080,
            Self::Fiq => 0x100,
        }
    }
}

impl Cpu {
    /// Take synchronous exception `e` raised by the instruction at `pc`.
    pub(crate) fn take_sync_exception(&mut self, e: Exception, pc: u64) -> ExceptionTaken {
        let from = self.pstate.el;
        // ESR_EL1.IL is 1: every A64 instruction is 32 bits.
        let esr = (e.class.code(from) << 26) | (1 << 25) | u64::from(e.iss & 0x01FF_FFFF);
        let return_addr = if e.class.returns_to_next() {
            pc.wrapping_add(4)
        } else {
            pc
        };
        self.sys.esr_el1 = esr;
        if let Some(far) = e.far {
            self.sys.far_el1 = far;
        }
        let saved = if e.class.preserves_btype() {
            self.pstate
        } else {
            Pstate {
                btype: Btype::None,
                ..self.pstate
            }
        };
        let vector = self.enter_el1(VectorKind::Synchronous, return_addr, saved.to_spsr());
        ExceptionTaken {
            kind: ExceptionKind::Synchronous(e.class),
            from,
            return_address: return_addr,
            vector,
            syndrome: esr,
            fault_address: e.far,
        }
    }

    /// Take an IRQ at the current instruction boundary.
    pub fn take_irq(&mut self) -> ExceptionTaken {
        self.take_interrupt(VectorKind::Irq, ExceptionKind::Irq)
    }

    /// Take an FIQ at the current instruction boundary.
    pub fn take_fiq(&mut self) -> ExceptionTaken {
        self.take_interrupt(VectorKind::Fiq, ExceptionKind::Fiq)
    }

    fn take_interrupt(&mut self, vector_kind: VectorKind, kind: ExceptionKind) -> ExceptionTaken {
        let from = self.pstate.el;
        let pc = self.pc;
        let vector = self.enter_el1(vector_kind, pc, self.pstate.to_spsr());
        ExceptionTaken {
            kind,
            from,
            return_address: pc,
            vector,
            syndrome: 0,
            fault_address: None,
        }
    }

    /// `AArch64.TakeException` to EL1, saving `spsr`, the image of `PSTATE`
    /// the exception keeps. Returns the vector now executing. `PSTATE.SM`
    /// and `PSTATE.ZA` are not part of that image and carry over unchanged.
    fn enter_el1(&mut self, kind: VectorKind, return_addr: u64, spsr: u64) -> u64 {
        self.maybe_zero_sve_uppers();
        let quadrant = match (self.pstate.el, self.pstate.sp_sel) {
            (El::El0, _) => 0x400,
            (El::El1, false) => 0x000,
            (El::El1, true) => 0x200,
        };
        self.sys.spsr_el1 = spsr;
        self.sys.elr_el1 = return_addr;
        // With FEAT_PAN, PAN is set when SCTLR_EL1.SPAN is 0 and otherwise
        // kept; UAO is cleared; SSBS becomes SCTLR_EL1.DSSBS; DIT is kept.
        let pan = self.pstate.pan
            || (self.implements(Feature::Pan) && self.sys.sctlr_el1 & sctlr::SPAN == 0);
        let ssbs = self.implements(Feature::Ssbs) && self.sys.sctlr_el1 & sctlr::DSSBS != 0;
        self.pstate = Pstate {
            n: self.pstate.n,
            z: self.pstate.z,
            c: self.pstate.c,
            v: self.pstate.v,
            d: true,
            a: true,
            i: true,
            f: true,
            el: El::El1,
            sp_sel: true,
            ss: false,
            il: false,
            btype: Btype::None,
            pan,
            uao: false,
            dit: self.pstate.dit,
            ssbs,
        };
        self.pc =
            self.branch_addr((self.sys.vbar_el1 & !0x7FF).wrapping_add(quadrant + kind.offset()));
        self.history.jump(self.pc, self.retired);
        self.monitor.armed = false;
        self.activity = Activity::Running;
        self.pc
    }

    /// `AArch64.ExceptionReturn(target, SPSR_EL1)`: what `ERET` does with
    /// `ELR_EL1` and `ERETAA`/`ERETAB` with `ELR_EL1` authenticated. Returns
    /// the new PC. Every return, legal or not, clears the local exclusive
    /// monitor and sets the Event Register (`ClearExclusiveLocal`,
    /// `SendEventLocal`).
    pub(crate) fn exception_return(&mut self, target: u64) -> u64 {
        let spsr = self.sys.spsr_el1;
        self.monitor.armed = false;

        // M[4] = 1 names AArch32, M[3:2] an EL; this core has neither AArch32
        // nor EL2/EL3, and EL0 has only SP_EL0 (M[0] must be 0 there).
        let m = spsr & 0x1F;
        let target_el = match (m >> 2) & 3 {
            0 => Some(El::El0),
            1 => Some(El::El1),
            _ => None,
        };
        let legal = m & 0x10 == 0
            && m & 0x2 == 0
            && match target_el {
                Some(El::El0) => m & 1 == 0,
                Some(El::El1) => true,
                None => false,
            };

        // SPSR_EL1 holds no bit of a feature this processor lacks: an MSR
        // writes only `spsr_mask`'s bits, and a snapshot that holds others
        // is refused.
        let p = &mut self.pstate;
        p.n = spsr & (1 << 31) != 0;
        p.z = spsr & (1 << 30) != 0;
        p.c = spsr & (1 << 29) != 0;
        p.v = spsr & (1 << 28) != 0;
        p.ss = spsr & (1 << 21) != 0;
        p.d = spsr & (1 << 9) != 0;
        p.a = spsr & (1 << 8) != 0;
        p.i = spsr & (1 << 7) != 0;
        p.f = spsr & (1 << 6) != 0;
        // Arm `SetPSTATEFromPSR` restores NZCV, SS, DAIF and PAN whether or
        // not the return is legal.
        p.pan = spsr & Pstate::PAN != 0;
        match (legal, target_el) {
            (true, Some(el)) => {
                p.el = el;
                p.sp_sel = m & 1 != 0;
                p.il = spsr & (1 << 20) != 0;
                p.btype = Btype::from_bits(spsr >> 10);
                p.ssbs = spsr & Pstate::SSBS != 0;
                p.uao = spsr & Pstate::UAO != 0;
                p.dit = spsr & Pstate::DIT != 0;
            }
            // Illegal exception return (Arm ARM D1.11.4): EL and SP are
            // unchanged, PSTATE.IL is set, and the next instruction raises
            // an Illegal Execution State exception. PSTATE.BTYPE, SSBS, UAO
            // and DIT are UNKNOWN (Arm `SetPSTATEFromPSR`); this model's
            // choice is BTYPE 00 and the other three unchanged.
            _ => {
                p.il = true;
                p.btype = Btype::None;
            }
        }
        // Arm `AArch64.ExceptionReturn`: `SendEventLocal` after
        // `SetPSTATEFromPSR`, legal return or not.
        self.send_event();
        target
    }
}
