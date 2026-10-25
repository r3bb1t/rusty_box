//! Taking exceptions to EL1 and returning from them (Arm ARM D1.10–D1.11,
//! `AArch64.TakeException` and `AArch64.ExceptionReturn`).
//!
//! Every exception targets EL1: this core implements neither EL2 nor EL3, so
//! there is no routing to decide.

use super::{Activity, Cpu, El, Pstate};

/// `ESR_ELx.EC`, the exception classes this core can raise.
///
/// Closed (R5). "Lower" and "same" EL variants of the aborts are chosen when
/// the exception is taken, from the EL it is taken from, so a handler raises
/// the abort without knowing which it will be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionClass {
    /// EC 0x00: UNDEFINED instruction, or an access to a register that does
    /// not exist at this EL.
    Unknown,
    /// EC 0x01: `WFI`/`WFE` trapped from EL0 by `SCTLR_EL1.nTWI`/`nTWE`.
    WfxTrap,
    /// EC 0x07: SIMD&FP access trapped by `CPACR_EL1.FPEN`.
    FpAccess,
    /// EC 0x0E: execution with `PSTATE.IL` set.
    IllegalState,
    /// EC 0x15.
    Svc,
    /// EC 0x18: `MSR`/`MRS`/`SYS` trapped from EL0.
    SysRegTrap,
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
            Self::IllegalState => 0x0E,
            Self::Svc => 0x15,
            Self::SysRegTrap => 0x18,
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
    pub(crate) fn take_sync_exception(&mut self, e: Exception, pc: u64) {
        // ESR_EL1.IL is 1: every A64 instruction is 32 bits.
        let esr = (e.class.code(self.pstate.el) << 26) | (1 << 25) | u64::from(e.iss & 0x01FF_FFFF);
        let return_addr = if e.class.returns_to_next() {
            pc.wrapping_add(4)
        } else {
            pc
        };
        self.sys.esr_el1 = esr;
        if let Some(far) = e.far {
            self.sys.far_el1 = far;
        }
        self.enter_el1(VectorKind::Synchronous, return_addr);
    }

    /// Take an IRQ at the current instruction boundary.
    pub fn take_irq(&mut self) {
        let pc = self.pc;
        self.enter_el1(VectorKind::Irq, pc);
    }

    /// Take an FIQ at the current instruction boundary.
    pub fn take_fiq(&mut self) {
        let pc = self.pc;
        self.enter_el1(VectorKind::Fiq, pc);
    }

    /// `AArch64.TakeException` to EL1.
    fn enter_el1(&mut self, kind: VectorKind, return_addr: u64) {
        let quadrant = match (self.pstate.el, self.pstate.sp_sel) {
            (El::El0, _) => 0x400,
            (El::El1, false) => 0x000,
            (El::El1, true) => 0x200,
        };
        self.sys.spsr_el1 = self.pstate.to_spsr();
        self.sys.elr_el1 = return_addr;
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
        };
        self.pc = (self.sys.vbar_el1 & !0x7FF).wrapping_add(quadrant + kind.offset());
        self.monitor.armed = false;
        self.activity = Activity::Running;
    }

    /// `ERET`: `AArch64.ExceptionReturn(ELR_EL1, SPSR_EL1)`. Returns the new PC.
    pub(crate) fn exception_return(&mut self) -> u64 {
        let spsr = self.sys.spsr_el1;
        let target = self.sys.elr_el1;
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
        match (legal, target_el) {
            (true, Some(el)) => {
                p.el = el;
                p.sp_sel = m & 1 != 0;
                p.il = spsr & (1 << 20) != 0;
            }
            // Illegal exception return (Arm ARM D1.11.4): EL and SP are
            // unchanged, PSTATE.IL is set, and the next instruction raises
            // an Illegal Execution State exception.
            _ => {
                p.il = true;
            }
        }
        target
    }
}
