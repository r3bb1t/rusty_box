//! Taking exceptions and returning from them (Arm ARM D1.10–D1.11,
//! `AArch64_TakeException` and `AArch64_ExceptionReturn`), and where each
//! exception is taken.
//!
//! An exception is taken to EL1, or with FEAT_EL2 to EL2; never to EL0, and
//! never to a level below the current one. Which of the two is the routing
//! of the Arm pseudocode function that raises it ([`Route`]): a trap or
//! call that names EL2, or the exception class's own routing (`EL2` for an
//! exception at EL2, and below that `HCR_EL2.TGE`, `MDCR_EL2.TDE` and
//! `HCR_EL2.TEA` as the class's function reads them).

use super::debug::Stepped;
use super::el2::{
    hpfar_fipa, El2Regs, El2Storage, HCR_AMO, HCR_FMO, HCR_IMO, HCR_TEA, HCR_TGE, HCR_VF, HCR_VI,
    HCR_VSE, MDCR_TDE, VSESR_MASK,
};
use super::features::Feature;
use super::mmu::{Fsc, Ipa};
use super::op::Extend;
use super::sysreg::{DBGPRCR_CORENPDRQ, MDSCR_KDE, MDSCR_SS, OSDLR_DLK, OSLSR_OSLK};
use super::{sctlr, Activity, Btype, Cpu, El, Levels, Pstate, SysRegs};
use crate::instrumentation::{ExceptionKind, ExceptionTaken};

/// `ESR_ELx.EC`, the exception classes this core can raise.
///
/// Closed (R5). "Lower" and "same" EL variants of the aborts are chosen when
/// the exception is taken, from the EL it is taken from and the EL it is
/// taken to, so a handler raises the abort without knowing which it will be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExceptionClass {
    /// EC 0x00: UNDEFINED instruction, an access to a register that does
    /// not exist at this EL, or a SIMD&FP access `CPACR_EL1.FPEN` traps to
    /// EL1 that `HCR_EL2.TGE` takes to EL2.
    Unknown,
    /// EC 0x01: `WFI`/`WFE` trapped from EL0 by `SCTLR_EL1.nTWI`/`nTWE`, or
    /// from EL0 and EL1 by `HCR_EL2.TWI`/`TWE`.
    WfxTrap,
    /// EC 0x07: SIMD&FP access trapped by `CPACR_EL1.FPEN` or
    /// `CPTR_EL2.TFP`.
    FpAccess,
    /// EC 0x09: a pointer authentication instruction trapped to EL2 by
    /// `HCR_EL2.API` (`TrapPACUse`).
    PacTrap,
    /// EC 0x0D: an indirect branch to an instruction in a guarded page
    /// that does not accept it (FEAT_BTI); `ISS[1:0]` is `PSTATE.BTYPE`.
    BranchTarget,
    /// EC 0x0E: execution with `PSTATE.IL` set.
    IllegalState,
    /// EC 0x15.
    Svc,
    /// EC 0x16: `HVC`, taken to EL2.
    Hvc,
    /// EC 0x17: `SMC` at EL1 trapped to EL2 by `HCR_EL2.TSC`; with no EL3
    /// there is no other `SMC` exception.
    Smc,
    /// EC 0x18: `MSR`/`MRS`/`SYS` trapped.
    SysRegTrap,
    /// EC 0x1C: a pointer authentication that failed, with FEAT_FPAC;
    /// `ISS[1:0]` says whether the key was a data key and which.
    PacFail,
    /// EC 0x1D: an SME instruction or register trapped by
    /// `CPACR_EL1.SMEN` or `CPTR_EL2.TSM`, or an instruction illegal in the
    /// current streaming mode; `ISS.SMTC` says which.
    SmeAccess,
    /// EC 0x20 / 0x21.
    InstructionAbort,
    /// EC 0x22.
    PcAlignment,
    /// EC 0x24 / 0x25.
    DataAbort,
    /// EC 0x26.
    SpAlignment,
    /// EC 0x30 / 0x31: a hardware breakpoint.
    Breakpoint,
    /// EC 0x32 / 0x33.
    SoftwareStep,
    /// EC 0x34 / 0x35: a hardware watchpoint.
    Watchpoint,
    /// EC 0x3C.
    Brk,
}

impl ExceptionClass {
    /// The `EC` value for an exception taken from `from` to `to` (Arm
    /// `AArch64_ExceptionClass`): an abort is "same EL" when the two are
    /// one level.
    pub const fn code(self, from: El, to: El) -> u64 {
        let same = from as u8 == to as u8;
        match self {
            Self::Unknown => 0x00,
            Self::WfxTrap => 0x01,
            Self::FpAccess => 0x07,
            Self::PacTrap => 0x09,
            Self::BranchTarget => 0x0D,
            Self::IllegalState => 0x0E,
            Self::Svc => 0x15,
            Self::Hvc => 0x16,
            Self::Smc => 0x17,
            Self::SysRegTrap => 0x18,
            Self::PacFail => 0x1C,
            Self::SmeAccess => 0x1D,
            Self::InstructionAbort => {
                if same {
                    0x21
                } else {
                    0x20
                }
            }
            Self::PcAlignment => 0x22,
            Self::DataAbort => {
                if same {
                    0x25
                } else {
                    0x24
                }
            }
            Self::SpAlignment => 0x26,
            Self::Breakpoint => {
                if same {
                    0x31
                } else {
                    0x30
                }
            }
            Self::SoftwareStep => {
                if same {
                    0x33
                } else {
                    0x32
                }
            }
            Self::Watchpoint => {
                if same {
                    0x35
                } else {
                    0x34
                }
            }
            Self::Brk => 0x3C,
        }
    }

    /// Whether the exception keeps `PSTATE.BTYPE` in `SPSR_ELx.BTYPE`
    /// (Arm `AArch64_TakeException`): the classes whose return can resume
    /// the branch target check, IRQ and FIQ too. Every other one saves 00,
    /// the choice `ConstrainUnpredictable(Unpredictable_ZEROBTYPE)` makes.
    const fn preserves_btype(self) -> bool {
        match self {
            Self::BranchTarget
            | Self::IllegalState
            | Self::InstructionAbort
            | Self::PcAlignment
            | Self::Breakpoint
            | Self::SoftwareStep
            | Self::Brk => true,
            Self::Unknown
            | Self::WfxTrap
            | Self::FpAccess
            | Self::PacTrap
            | Self::Svc
            | Self::Hvc
            | Self::Smc
            | Self::SysRegTrap
            | Self::PacFail
            | Self::SmeAccess
            | Self::DataAbort
            | Self::SpAlignment
            | Self::Watchpoint => false,
        }
    }
}

/// The exception classes that report a virtual address in `FAR_ELx` (Arm
/// `AArch64_ReportException`). Every other class leaves `FAR_ELx`
/// ARBITRARY; here, unchanged.
///
/// Closed (R5): only an exception of one of these carries an address
/// ([`Exception::at_address`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressedClass {
    InstructionAbort,
    PcAlignment,
    DataAbort,
    Watchpoint,
}

impl AddressedClass {
    const fn class(self) -> ExceptionClass {
        match self {
            Self::InstructionAbort => ExceptionClass::InstructionAbort,
            Self::PcAlignment => ExceptionClass::PcAlignment,
            Self::DataAbort => ExceptionClass::DataAbort,
            Self::Watchpoint => ExceptionClass::Watchpoint,
        }
    }
}

/// The exception classes that report no address, only a syndrome: every
/// class but an [`AddressedClass`].
///
/// Closed (R5): an exception of one of these is made without an address
/// ([`Exception::new`], [`Exception::routed`], [`Exception::to_el2`]), so
/// none of the classes that report one can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyndromeClass {
    Unknown,
    WfxTrap,
    FpAccess,
    PacTrap,
    BranchTarget,
    IllegalState,
    Svc,
    Hvc,
    Smc,
    SysRegTrap,
    PacFail,
    SmeAccess,
    SpAlignment,
    Breakpoint,
    SoftwareStep,
    Brk,
}

impl SyndromeClass {
    const fn class(self) -> ExceptionClass {
        match self {
            Self::Unknown => ExceptionClass::Unknown,
            Self::WfxTrap => ExceptionClass::WfxTrap,
            Self::FpAccess => ExceptionClass::FpAccess,
            Self::PacTrap => ExceptionClass::PacTrap,
            Self::BranchTarget => ExceptionClass::BranchTarget,
            Self::IllegalState => ExceptionClass::IllegalState,
            Self::Svc => ExceptionClass::Svc,
            Self::Hvc => ExceptionClass::Hvc,
            Self::Smc => ExceptionClass::Smc,
            Self::SysRegTrap => ExceptionClass::SysRegTrap,
            Self::PacFail => ExceptionClass::PacFail,
            Self::SmeAccess => ExceptionClass::SmeAccess,
            Self::SpAlignment => ExceptionClass::SpAlignment,
            Self::Breakpoint => ExceptionClass::Breakpoint,
            Self::SoftwareStep => ExceptionClass::SoftwareStep,
            Self::Brk => ExceptionClass::Brk,
        }
    }
}

/// Where a synchronous exception is taken from EL0 or EL1: from EL2 it is
/// always EL2.
///
/// Closed (R5): the two routings of the Arm functions that raise
/// exceptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    /// Where the exception class's own function takes it
    /// ([`Cpu::class_routes_to_el2`]): `AArch64_Undefined`, `AArch64_Abort`,
    /// `AArch64_CallSupervisor` and their kind, which take the exception to
    /// EL1 unless an EL2 control (`TGE`, `TDE`, `TEA`) sends it to EL2.
    Class,
    /// To EL2: a trap an EL2 control raises (`AArch64_SystemAccessTrap(EL2,
    /// ..)`, `AArch64_AdvSIMDFPAccessTrap(EL2)`, `TrapPACUse(EL2)`, the
    /// `HCR_EL2.TWx` and `TSC` traps), and `HVC`.
    El2,
}

/// Whether a Data Abort reports the syndrome of the load or store that made
/// it, once the instruction gives one (Arm `AArch64_InstructionSyndromeValid`
/// as far as the access decides it): `Open` for a stage 2 abort on the
/// access itself at its lowest address (`accdesc.lowestaddress`), unless it
/// is a synchronous External abort on stage 2's walk ("ISV on second stage
/// translation table walk" is FALSE here, the value FEAT_RAS gives it);
/// `Closed` for every other exception. The translation decides it where it
/// builds the abort; the instruction closes it, filled with its syndrome
/// or, where its form has none, empty
/// ([`Fault::reporting`](super::exec::Fault::reporting)).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyndromeSlot {
    Open,
    Closed,
}

/// The syndrome of a load or store of one general-purpose register without
/// writeback, which a stage 2 Data Abort on its access reports in
/// `ESR_EL2.ISS[24:14]` (Arm `LSInstructionSyndrome`, whose content the
/// Arm ARM's `ESR_ELx` gives, `ISV` = 1): `SAS`, the access size; `SSE`, a
/// load's sign extension; `SRT`, the transfer register; `SF`, a 64-bit
/// register; `AR`, acquire or release semantics. Such an instruction fills
/// the abort's [`SyndromeSlot`]: its class handler makes its access with
/// [`Cpu::read_gpr`] or [`Cpu::write_gpr`], and its pre-decoded op makes
/// it from the fields it holds when the access faults
/// (`op::one_register_syndrome`). Every other instruction — a load or store
/// exclusive, one with writeback, a pair, an atomic access, a SIMD&FP, SVE
/// or SME transfer, a cache maintenance or address translation instruction
/// — reports none (`ISV` = 0).
///
/// It holds the instruction's own fields, as its decode gives them, and
/// derives `SSE` and `SF` only when a fault reports them ([`Self::iss`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AccessSyndrome {
    /// The access's size, log2 of its bytes.
    size_log2: u8,
    /// How a load extends what it reads into `Rt`; a store's is
    /// [`Extend::Zero`], its register an X register for a doubleword as a
    /// zero-extending load's is.
    extend: Extend,
    rt: u8,
    acquire_release: bool,
}

impl AccessSyndrome {
    /// A load of `1 << size_log2` bytes into `Rt` = `rt`, extended as
    /// `extend` says.
    #[inline(always)]
    pub(crate) const fn load(size_log2: u32, extend: Extend, rt: u32) -> Self {
        Self {
            size_log2: size_log2 as u8,
            extend,
            rt: rt as u8,
            acquire_release: false,
        }
    }

    /// A store of the low `1 << size_log2` bytes of `Rt` = `rt`.
    #[inline(always)]
    pub(crate) const fn store(size_log2: u32, rt: u32) -> Self {
        Self::load(size_log2, Extend::Zero, rt)
    }

    /// The same access with acquire or release semantics (`AR`).
    #[inline(always)]
    pub(crate) const fn acquire_release(self) -> Self {
        Self {
            acquire_release: true,
            ..self
        }
    }

    /// `ISV`, `SAS`, `SSE`, `SRT`, `SF` and `AR`, at their `ISS` bits: a
    /// sign-extending load sets `SSE`, and `SF` is set for an X register,
    /// which every doubleword access and every sign extension to 64 bits
    /// transfers.
    pub(crate) const fn iss(self) -> u32 {
        let (sign_extend, sixty_four) = match self.extend {
            Extend::Zero => (false, self.size_log2 == 3),
            Extend::Signed64 => (true, true),
            Extend::Signed32 => (true, false),
        };
        (1 << 24)
            | ((self.size_log2 as u32 & 3) << 22)
            | ((sign_extend as u32) << 21)
            | ((self.rt as u32 & 31) << 16)
            | ((sixty_four as u32) << 15)
            | ((self.acquire_release as u32) << 14)
    }
}

/// Which addresses an [`Exception`] reports beside its syndrome, as it holds
/// them (Arm `AArch64_ReportException`): `Far`, the faulting virtual address
/// in `FAR_ELx`, for an exception of a class that reports one
/// ([`AddressedClass`]); `FarAndIpa`, the IPA in `HPFAR_EL2.FIPA` as well,
/// for a stage 2 abort where Arm's `IPAValid` holds; `Syndrome` for every
/// other. A register an exception does not report is ARBITRARY, and keeps
/// its value here.
///
/// Closed (R5): only an exception built with an address reports one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reports {
    Syndrome,
    Far,
    FarAndIpa,
}

/// A synchronous exception raised by an instruction.
///
/// It is 24 bytes, as every instruction's result carries one: the address
/// it reports is held where its class reports one, and read through
/// [`Exception::far`], and the IPA a stage 2 abort reports beside it
/// through [`Exception::hpfar`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Exception {
    pub(crate) class: ExceptionClass,
    pub(crate) iss: u32,
    /// The faulting virtual address, where `reports` reports one; 0
    /// otherwise.
    far: u64,
    /// A stage 2 abort's IPA, where `reports` reports one; 0 otherwise.
    ipa: Ipa,
    pub(crate) route: Route,
    reports: Reports,
    syndrome: SyndromeSlot,
}

// The size every instruction's result carries a fault in: a larger one
// costs the run loop that returns each instruction's result.
const _: () = assert!(core::mem::size_of::<Exception>() == 24);

impl Exception {
    /// An exception of `class`, which reports no address, taken where its
    /// class's function takes it.
    #[inline]
    pub(crate) const fn new(class: SyndromeClass, iss: u32) -> Self {
        Self::routed(class, iss, Route::Class)
    }

    /// An exception of `class` taken to EL2: a trap an EL2 control raises.
    #[inline]
    pub(crate) const fn to_el2(class: SyndromeClass, iss: u32) -> Self {
        Self::routed(class, iss, Route::El2)
    }

    /// An exception of `class`, which reports no address, taken as `route`
    /// says: for a trap whose target depends on the control that raised it.
    #[inline]
    pub(crate) const fn routed(class: SyndromeClass, iss: u32, route: Route) -> Self {
        Self {
            class: class.class(),
            iss,
            far: 0,
            ipa: Ipa::new(0),
            route,
            reports: Reports::Syndrome,
            syndrome: SyndromeSlot::Closed,
        }
    }

    /// An exception of `class`, which reports `far` in `FAR_ELx`, taken
    /// where its class's function takes it.
    #[inline]
    pub(crate) const fn at_address(class: AddressedClass, iss: u32, far: u64) -> Self {
        Self {
            class: class.class(),
            iss,
            far,
            ipa: Ipa::new(0),
            route: Route::Class,
            reports: Reports::Far,
            syndrome: SyndromeSlot::Closed,
        }
    }

    /// An abort of stage 2 of `class` at `far`, taken to EL2
    /// (`AArch64_DataAbort`, `AArch64_InstructionAbort` and
    /// `SyncExternalAbortTarget`: `IsSecondStage` routes it there), which
    /// reports `hpfar` in `HPFAR_EL2` (where `IPAValid` holds; `None` leaves
    /// the register as it is), and the syndrome of the instruction that
    /// made it where `syndrome` is open.
    #[inline]
    pub(crate) const fn second_stage_abort(
        class: AddressedClass,
        iss: u32,
        far: u64,
        hpfar: Option<Ipa>,
        syndrome: SyndromeSlot,
    ) -> Self {
        let (ipa, reports) = match hpfar {
            Some(ipa) => (ipa, Reports::FarAndIpa),
            None => (Ipa::new(0), Reports::Far),
        };
        Self {
            class: class.class(),
            iss,
            far,
            ipa,
            route: Route::El2,
            reports,
            syndrome,
        }
    }

    /// The exception, its syndrome slot closed where it is open: filled
    /// with `syndrome`, `ISV` and the instruction's fields joining the
    /// abort's syndrome, or left empty (`ISV` 0) where the instruction's
    /// form has none (`None`).
    #[inline]
    pub(crate) const fn reporting(self, syndrome: Option<AccessSyndrome>) -> Self {
        match (self.syndrome, syndrome) {
            (SyndromeSlot::Open, Some(syndrome)) => Self {
                iss: self.iss | syndrome.iss(),
                syndrome: SyndromeSlot::Closed,
                ..self
            },
            (SyndromeSlot::Open, None) => Self {
                syndrome: SyndromeSlot::Closed,
                ..self
            },
            (SyndromeSlot::Closed, _) => self,
        }
    }

    /// The faulting virtual address `FAR_ELx` reports, for an exception
    /// built with one ([`Exception::at_address`],
    /// [`Exception::second_stage_abort`]); `None` for every other.
    #[inline]
    pub(crate) const fn far(&self) -> Option<u64> {
        match self.reports {
            Reports::Far | Reports::FarAndIpa => Some(self.far),
            Reports::Syndrome => None,
        }
    }

    /// The IPA `HPFAR_EL2` reports, for a stage 2 abort whose IPA Arm's
    /// `IPAValid` reports; `None` for every other exception.
    #[inline]
    pub(crate) const fn hpfar(&self) -> Option<Ipa> {
        match self.reports {
            Reports::FarAndIpa => Some(self.ipa),
            Reports::Syndrome | Reports::Far => None,
        }
    }

    /// Whether the preferred return address is the next instruction rather
    /// than the one that raised the exception: `SVC` (`AArch64_CallSupervisor`)
    /// and `HVC` (`AArch64_CallHypervisor`), each `NextInstrAddr`. A trapped
    /// `SMC` returns to itself (`AArch64_CheckForSMCUndefOrTrap`).
    const fn returns_to_next(self) -> bool {
        matches!(
            (self.class, self.route),
            (ExceptionClass::Svc, Route::Class) | (ExceptionClass::Hvc, _)
        )
    }
}

/// Whether an Instruction or Data Abort's syndrome `iss` holds a
/// synchronous External abort's fault status code (`ESR_ELx.ISS.DFSC` and
/// `IFSC`, [`Fsc::is_external`]).
const fn is_external_abort(iss: u32) -> bool {
    Fsc::is_external(iss & 0x3F)
}

/// Which vector of the four in a vector-table quadrant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VectorKind {
    Synchronous,
    Irq,
    Fiq,
    SError,
}

impl VectorKind {
    const fn offset(self) -> u64 {
        match self {
            Self::Synchronous => 0x000,
            Self::Irq => 0x080,
            Self::Fiq => 0x100,
            Self::SError => 0x180,
        }
    }
}

/// What an exception entry records in the target level's `ESR_ELx` and
/// `FAR_ELx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryRecord {
    /// Nothing: an IRQ or FIQ leaves both as they were.
    Nothing,
    /// The syndrome, and the faulting address of a class that reports one.
    Syndrome { esr: u64, far: Option<u64> },
}

/// A level with exception registers, which an exception is taken to and
/// an `ERET` returns from: EL1, or EL2 on a processor with it. EL2 carries
/// the proof that the processor has EL2, so no processor without makes one
/// (R2), and only [`Cpu::target_level`] does: the level an exception's
/// syndrome and [`ExceptionTaken`] name is the level it enters, and the
/// registers [`Cpu::exception_regs`] gives are that level's.
enum Target<L: Levels> {
    El1,
    El2(<L::State as El2Storage>::Proof),
}

// By hand: a derive would bound `L` itself, where only the proof is copied.
impl<L: Levels> Clone for Target<L> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<L: Levels> Copy for Target<L> {}

impl<L: Levels> Target<L> {
    const fn el(self) -> El {
        match self {
            Self::El1 => El::El1,
            Self::El2(_) => El::El2,
        }
    }
}

/// `SPSR_ELx` and `ELR_ELx` of a level, as an `ERET` from it reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReturnRegs {
    spsr: u64,
    elr: u64,
}

/// How a physical interrupt is masked at the current level
/// ([`Cpu::interrupt_masking`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Masking {
    /// Never taken here: it targets a lower level.
    Never,
    /// Taken here whatever `PSTATE` masks: it is routed to a higher level.
    Unmasked,
    /// Taken here when its `PSTATE` mask is clear.
    ByPstate,
}

/// `ESR_ELx.EC` of an SError exception.
const EC_SERROR: u64 = 0x2F;

/// A level's exception registers: those an exception taken to it writes
/// (`ESR_ELx`, `FAR_ELx`, `SPSR_ELx`, `ELR_ELx`) and reads (`SCTLR_ELx`,
/// `VBAR_ELx`), and an `ERET` from it reads (`SPSR_ELx`, `ELR_ELx`).
struct ExceptionRegs<'a> {
    esr: &'a mut u64,
    far: &'a mut u64,
    spsr: &'a mut u64,
    elr: &'a mut u64,
    sctlr: u64,
    vbar: u64,
}

impl SysRegs {
    /// EL1's exception registers.
    fn el1_exception_regs(&mut self) -> ExceptionRegs<'_> {
        ExceptionRegs {
            sctlr: self.sctlr_el1,
            vbar: self.vbar_el1,
            esr: &mut self.esr_el1,
            far: &mut self.far_el1,
            spsr: &mut self.spsr_el1,
            elr: &mut self.elr_el1,
        }
    }
}

impl El2Regs {
    /// EL2's exception registers.
    fn exception_regs(&mut self) -> ExceptionRegs<'_> {
        ExceptionRegs {
            sctlr: self.sctlr,
            vbar: self.vbar,
            esr: &mut self.esr,
            far: &mut self.far,
            spsr: &mut self.spsr,
            elr: &mut self.elr,
        }
    }
}

impl<L: Levels> Cpu<L> {
    /// The exception registers of `level`, which an exception to it writes
    /// ([`Cpu::take_exception`]) and an `ERET` from it reads
    /// ([`Cpu::return_regs`]): EL2's on a processor with EL2, EL1's for
    /// EL1, each chosen by an exhaustive match on [`Target`] (R5).
    fn exception_regs(&mut self, level: Target<L>) -> ExceptionRegs<'_> {
        match level {
            Target::El1 => self.sys.el1_exception_regs(),
            Target::El2(proof) => self.el2_state.proven(proof).regs.exception_regs(),
        }
    }
}

/// The physical interrupts an asserted line raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interrupt {
    Irq,
    Fiq,
}

impl Interrupt {
    /// The `HCR_EL2` bit routing it to EL2 (`IMO`, `FMO`), which also
    /// makes its virtual kind exist for EL1 and EL0.
    const fn routing(self) -> u64 {
        match self {
            Self::Irq => HCR_IMO,
            Self::Fiq => HCR_FMO,
        }
    }
}

/// The virtual interrupts `HCR_EL2` raises for EL1 and EL0 (FEAT_EL2).
///
/// Closed (R5): with no GIC virtual CPU interface, `VirtualIRQPending()`
/// and `VirtualFIQPending()` are FALSE and `HCR_EL2` is their only source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VirtualInterrupt {
    Irq,
    Fiq,
    SError,
}

impl VirtualInterrupt {
    /// Every virtual interrupt, in the order this processor takes them when
    /// several are deliverable, after the physical ones. Arm leaves the
    /// prioritization of asynchronous exceptions IMPLEMENTATION DEFINED;
    /// this order is the choice made here (QEMU's `arm_cpu_exec_interrupt`
    /// makes the same one).
    const ALL: [Self; 3] = [Self::Irq, Self::Fiq, Self::SError];

    /// The `HCR_EL2` bit routing the physical kind to EL2, without which
    /// the virtual kind is not pending: `IMO`, `FMO`, `AMO`.
    const fn routing(self) -> u64 {
        match self {
            Self::Irq => Interrupt::Irq.routing(),
            Self::Fiq => Interrupt::Fiq.routing(),
            Self::SError => HCR_AMO,
        }
    }

    /// The `HCR_EL2` bit that makes it pending: `VI`, `VF`, `VSE`.
    const fn pending(self) -> u64 {
        match self {
            Self::Irq => HCR_VI,
            Self::Fiq => HCR_VF,
            Self::SError => HCR_VSE,
        }
    }
}

/// The `HCR_EL2` bits that make a virtual interrupt pending.
const HCR_VIRTUAL_PENDING: u64 = HCR_VI | HCR_VF | HCR_VSE;

/// An asynchronous exception that is deliverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Deliverable {
    Physical(Interrupt),
    Virtual(VirtualInterrupt),
}

impl<L: Levels> Cpu<L> {
    /// Whether the class's own exception function takes `e`, raised at EL0
    /// or EL1, to EL2 (the `route_to_el2` of each function):
    /// - `AArch64_Undefined`, `AArch64_CallSupervisor`,
    ///   `AArch64_BranchTargetException`, `AArch64_CheckIllegalState`,
    ///   `AArch64_PACFailException`, `SMEAccessTrap` to EL1, and
    ///   `AArch64_SystemAccessTraptoEL1orEL2`: from EL0 under `HCR_EL2.TGE`;
    /// - `AArch64_PCAlignmentFault`, `AArch64_SPAlignmentFault`, and the
    ///   trap to EL1 `AArch64_WFxTrap` takes: from EL0 or EL1 under `TGE`;
    /// - `AArch64_AdvSIMDFPAccessTrap`, which [`Cpu::fp_trap`] carries out
    ///   whole: under `TGE` its trap to EL1 is raised as the uncategorized
    ///   exception to EL2 the function takes instead, so one of this class
    ///   stays at EL1;
    /// - `AArch64_SoftwareBreakpoint`, `AArch64_BreakpointException`,
    ///   `AArch64_SoftwareStepException` and `AArch64_WatchpointException`:
    ///   under `TGE` or `MDCR_EL2.TDE`;
    /// - `AArch64_InstructionAbort`, `AArch64_DataAbort`: a synchronous
    ///   External abort where `SyncExternalAbortTarget` says (`HCR_EL2.TEA`,
    ///   or `TGE` from EL0), any other abort under `TGE`;
    /// - `AArch64_CallHypervisor`, a trapped `SMC` and `TrapPACUse` name EL2
    ///   themselves.
    fn class_routes_to_el2(&self, e: &Exception) -> bool {
        let from_el0 = self.pstate.el == El::El0;
        let tge = self.tge();
        match e.class {
            ExceptionClass::Unknown
            | ExceptionClass::Svc
            | ExceptionClass::BranchTarget
            | ExceptionClass::IllegalState
            | ExceptionClass::PacFail
            | ExceptionClass::SmeAccess
            | ExceptionClass::SysRegTrap => from_el0 && tge,
            ExceptionClass::PcAlignment | ExceptionClass::SpAlignment | ExceptionClass::WfxTrap => {
                tge
            }
            ExceptionClass::FpAccess => false,
            ExceptionClass::Brk
            | ExceptionClass::Breakpoint
            | ExceptionClass::SoftwareStep
            | ExceptionClass::Watchpoint => tge || self.mdcr_any(MDCR_TDE),
            ExceptionClass::InstructionAbort | ExceptionClass::DataAbort => {
                if is_external_abort(e.iss) {
                    self.hcr_any(HCR_TEA) || (from_el0 && tge)
                } else {
                    tge
                }
            }
            ExceptionClass::Hvc | ExceptionClass::Smc | ExceptionClass::PacTrap => true,
        }
    }

    /// The level an exception raised now is taken to: EL2 at EL2 or when
    /// its routing names EL2, EL1 otherwise. Arm routes to EL2 only where
    /// `EL2Enabled()`, so on a processor without EL2 this is EL1, a
    /// constant, whatever the routing says. The one place a level is chosen
    /// (R5): every [`Target::El2`] comes from here.
    #[inline(always)]
    fn target_level(&self, routed_to_el2: bool) -> Target<L> {
        match L::State::proof() {
            Some(proof) if self.pstate.el == El::El2 || routed_to_el2 => Target::El2(proof),
            Some(_) | None => Target::El1,
        }
    }

    /// The level an `ERET` returns from (Arm `SPSR_ELx[]`, `ELR_ELx[]`):
    /// the current one, EL1 or EL2, `ERET` being UNDEFINED at EL0.
    fn return_level(&self) -> Target<L> {
        self.target_level(false)
    }

    /// The level `e`, raised at the current level, is taken to: the
    /// current level at EL2, and from EL0 or EL1 its route's.
    fn exception_target(&self, e: &Exception) -> Target<L> {
        let routed_to_el2 = match e.route {
            Route::El2 => true,
            Route::Class => self.class_routes_to_el2(e),
        };
        self.target_level(routed_to_el2)
    }

    /// Take synchronous exception `e` raised by the instruction at `pc`. A
    /// stage 2 abort taken to EL2 reports its IPA in `HPFAR_EL2.FIPA`
    /// (`AArch64_ReportException`: IPA bits 47:12 in bits 39:4, with
    /// FEAT_LPA bits 51:12 in bits 43:4, `NS` 0 with no Secure EL2) where
    /// `IPAValid`, and leaves it as it was where it is ARBITRARY.
    pub(crate) fn take_sync_exception(&mut self, e: Exception, pc: u64) -> ExceptionTaken {
        let from = self.pstate.el;
        let target = self.exception_target(&e);
        let to = target.el();
        if let (Target::El2(proof), Some(ipa)) = (target, e.hpfar()) {
            let fipa = hpfar_fipa(self.imp.features);
            self.el2_state.proven(proof).regs.hpfar = ((ipa.get() >> 12) << 4) & fipa;
        }
        let class = e.class;
        // ESR_ELx.IL is 1: every A64 instruction is 32 bits.
        let esr = (class.code(from, to) << 26) | (1 << 25) | u64::from(e.iss & 0x01FF_FFFF);
        // Arm `AArch64_CallSupervisor` and `AArch64_CallHypervisor`: the
        // call completes the instruction it is, so Software Step advances
        // before SPSR_ELx.SS is saved.
        let return_addr = if e.returns_to_next() {
            self.ss_advance(None);
            pc.wrapping_add(4)
        } else {
            pc
        };
        let far = e.far();
        let saved = if class.preserves_btype() {
            self.pstate
        } else {
            Pstate {
                btype: Btype::None,
                ..self.pstate
            }
        };
        let record = EntryRecord::Syndrome { esr, far };
        let vector = self.take_exception(
            target,
            VectorKind::Synchronous,
            return_addr,
            saved.to_spsr(),
            record,
        );
        ExceptionTaken {
            kind: ExceptionKind::Synchronous(class),
            from,
            to,
            return_address: return_addr,
            vector,
            syndrome: esr,
            fault_address: far,
        }
    }

    /// Whether a physical `interrupt` is routed to EL2: `HCR_EL2.TGE` or the
    /// interrupt's routing bit (`IMO`, `FMO`) set.
    fn interrupt_routed_to_el2(&self, interrupt: Interrupt) -> bool {
        self.hcr_any(interrupt.routing() | HCR_TGE)
    }

    /// The level a physical `interrupt` taken now is taken to (Arm
    /// `AArch64_TakePhysicalIRQException` and its FIQ sibling): EL2 at EL2
    /// and when it is routed there, EL1 otherwise.
    fn interrupt_target(&self, interrupt: Interrupt) -> Target<L> {
        self.target_level(self.interrupt_routed_to_el2(interrupt))
    }

    /// How an asserted physical `interrupt` is masked at the current level
    /// (Arm ARM, "Asynchronous exception masking", the physical interrupt
    /// masking table for a PE with EL2 and no EL3): one that targets EL1 is
    /// never taken at EL2, the level above it; one routed to EL2 is taken at
    /// EL0 and EL1 whatever `PSTATE` masks; and otherwise its `PSTATE` mask
    /// (`I`, `F`) decides. The one reading of the table. A processor
    /// without EL2 is never at EL2 and routes nothing there, so for one
    /// every case is `ByPstate` and the run loop's test before each
    /// instruction reads no level.
    #[inline(always)]
    fn interrupt_masking(&self, interrupt: Interrupt) -> Masking {
        match (self.pstate.el, self.interrupt_routed_to_el2(interrupt)) {
            (El::El2, false) if L::EL2 => Masking::Never,
            (El::El0 | El::El1, true) => Masking::Unmasked,
            (El::El2, _) | (El::El0 | El::El1, false) => Masking::ByPstate,
        }
    }

    /// Whether an asserted physical `interrupt` is taken at the current
    /// level now ([`Cpu::interrupt_masking`]).
    #[inline(always)]
    fn interrupt_unmasked(&self, interrupt: Interrupt) -> bool {
        match self.interrupt_masking(interrupt) {
            Masking::Never => false,
            Masking::Unmasked => true,
            Masking::ByPstate => match interrupt {
                Interrupt::Irq => !self.pstate.i,
                Interrupt::Fiq => !self.pstate.f,
            },
        }
    }

    /// Whether virtual `interrupt` is pending at the current level (the
    /// virtual part of Arm `AArch64_InterruptPending`): at EL0 or EL1 with
    /// EL2 enabled, `HCR_EL2.TGE` clear, and its routing and pending bits
    /// set.
    fn virtual_pending(&self, interrupt: VirtualInterrupt) -> bool {
        let bits = interrupt.routing() | interrupt.pending();
        L::EL2 && self.el2().hcr & (bits | HCR_TGE) == bits && self.pstate.el != El::El2
    }

    /// Whether a virtual interrupt is raised: `HCR_EL2.VI`, `VF` or `VSE`
    /// set, whether or not it is pending at the current level.
    #[inline(always)]
    fn virtual_raised(&self) -> bool {
        L::EL2 && self.el2().hcr & HCR_VIRTUAL_PENDING != 0
    }

    /// Whether any virtual interrupt is pending at the current level,
    /// masked by `PSTATE` or not.
    pub(crate) fn virtual_interrupt_pending(&self) -> bool {
        self.virtual_raised()
            && VirtualInterrupt::ALL
                .into_iter()
                .any(|interrupt| self.virtual_pending(interrupt))
    }

    /// `ISR_EL1` as read at `el`: the IRQ (I), FIQ (F) and SError (A)
    /// pending. At EL1, an interrupt whose physical kind the effective
    /// `HCR_EL2.IMO`, `FMO` or `AMO` routes to EL2 (`TGE` routes all three)
    /// shows as its virtual kind ([`Cpu::virtual_pending`], which `TGE`
    /// leaves none of); elsewhere the physical kind shows, and no physical
    /// SError is ever pending.
    pub(crate) fn interrupt_status(&self, el: El) -> u64 {
        let shown = |interrupt: VirtualInterrupt, physical: bool| {
            if el == El::El1 && (self.tge() || self.hcr_any(interrupt.routing())) {
                self.virtual_pending(interrupt)
            } else {
                physical
            }
        };
        (u64::from(shown(VirtualInterrupt::SError, false)) << 8)
            | (u64::from(shown(VirtualInterrupt::Irq, self.irq_line)) << 7)
            | (u64::from(shown(VirtualInterrupt::Fiq, self.fiq_line)) << 6)
    }

    /// Arm `AArch64_vESBOperation`, the part of ESB that acts at EL0 and EL1
    /// with EL2 enabled: a pending virtual SError that `PSTATE.A` masks is
    /// deferred into `VDISR_EL2` (`A` set, with `VSESR_EL2`'s `IDS` and
    /// `ISS`) and is no longer pending (`ClearPendingVirtualSError`). An
    /// unmasked one was taken at the boundary before the ESB.
    pub(crate) fn virtual_esb(&mut self) {
        let deferred = self.virtual_pending(VirtualInterrupt::SError) && self.pstate.a;
        if let Some(el2) = self.el2_mut().filter(|_| deferred) {
            el2.vdisr = (1 << 31) | (el2.vsesr & VSESR_MASK);
            el2.hcr &= !HCR_VSE;
        }
    }

    /// Whether a pending virtual `interrupt` is taken now: its `PSTATE`
    /// mask (`I`, `F`, `A`) clear.
    fn virtual_unmasked(&self, interrupt: VirtualInterrupt) -> bool {
        match interrupt {
            VirtualInterrupt::Irq => !self.pstate.i,
            VirtualInterrupt::Fiq => !self.pstate.f,
            VirtualInterrupt::SError => !self.pstate.a,
        }
    }

    /// Whether an asynchronous exception is pending and unmasked, so that it
    /// would be taken at the next instruction boundary.
    #[inline]
    pub fn interrupt_deliverable(&self) -> bool {
        self.deliverable_interrupt().is_some()
    }

    /// The asynchronous exception taken at the next instruction boundary, if
    /// any: a physical FIQ, then IRQ, then a virtual one
    /// ([`VirtualInterrupt::ALL`]). The one test of deliverability: the run
    /// loop asks it before every instruction, inline, and with neither
    /// input asserted and no virtual interrupt raised it reads nothing else.
    #[inline(always)]
    fn deliverable_interrupt(&self) -> Option<Deliverable> {
        if self.fiq_line && self.interrupt_unmasked(Interrupt::Fiq) {
            Some(Deliverable::Physical(Interrupt::Fiq))
        } else if self.irq_line && self.interrupt_unmasked(Interrupt::Irq) {
            Some(Deliverable::Physical(Interrupt::Irq))
        } else if self.virtual_raised() {
            self.deliverable_virtual()
        } else {
            None
        }
    }

    /// The virtual interrupt taken at the next instruction boundary, if any.
    fn deliverable_virtual(&self) -> Option<Deliverable> {
        VirtualInterrupt::ALL
            .into_iter()
            .find(|&interrupt| self.virtual_pending(interrupt) && self.virtual_unmasked(interrupt))
            .map(Deliverable::Virtual)
    }

    /// Take the asynchronous exception [`Cpu::interrupt_deliverable`] says
    /// is deliverable, if any. The run loop asks before every instruction,
    /// so the test runs inline and the taking out of line.
    #[inline(always)]
    pub fn deliver_interrupt(&mut self) -> Option<ExceptionTaken> {
        let deliverable = self.deliverable_interrupt()?;
        Some(self.take_deliverable(deliverable))
    }

    /// Take `deliverable`, which [`Cpu::deliverable_interrupt`] chose.
    #[inline(never)]
    fn take_deliverable(&mut self, deliverable: Deliverable) -> ExceptionTaken {
        match deliverable {
            Deliverable::Physical(interrupt) => self.take_physical_interrupt(interrupt),
            Deliverable::Virtual(interrupt) => self.take_virtual_interrupt(interrupt),
        }
    }

    /// Take an IRQ at the current instruction boundary whatever `PSTATE.I`
    /// says, to the level its routing names. An IRQ whose routing names EL1
    /// is never taken at EL2, the level above it, so there nothing is taken.
    pub fn take_irq(&mut self) -> Option<ExceptionTaken> {
        self.take_unmasked(Interrupt::Irq)
    }

    /// Take an FIQ at the current instruction boundary whatever `PSTATE.F`
    /// says, to the level its routing names. An FIQ whose routing names EL1
    /// is never taken at EL2, the level above it, so there nothing is taken.
    pub fn take_fiq(&mut self) -> Option<ExceptionTaken> {
        self.take_unmasked(Interrupt::Fiq)
    }

    fn take_unmasked(&mut self, interrupt: Interrupt) -> Option<ExceptionTaken> {
        match self.interrupt_masking(interrupt) {
            Masking::Never => None,
            Masking::Unmasked | Masking::ByPstate => Some(self.take_physical_interrupt(interrupt)),
        }
    }

    /// Take `interrupt`, which can be taken at the current level, to the
    /// level [`Cpu::interrupt_target`] names.
    fn take_physical_interrupt(&mut self, interrupt: Interrupt) -> ExceptionTaken {
        let (vector_kind, kind) = match interrupt {
            Interrupt::Irq => (VectorKind::Irq, ExceptionKind::Irq),
            Interrupt::Fiq => (VectorKind::Fiq, ExceptionKind::Fiq),
        };
        let target = self.interrupt_target(interrupt);
        let from = self.pstate.el;
        let pc = self.pc;
        let vector = self.take_exception(
            target,
            vector_kind,
            pc,
            self.pstate.to_spsr(),
            EntryRecord::Nothing,
        );
        ExceptionTaken {
            kind,
            from,
            to: target.el(),
            return_address: pc,
            vector,
            syndrome: 0,
            fault_address: None,
        }
    }

    /// Take the pending, unmasked virtual `interrupt` to EL1 (Arm
    /// `AArch64_TakeVirtualIRQException` and its FIQ and SError siblings).
    /// A virtual SError reports EC 0x2F with, under FEAT_RAS, `VSESR_EL2`'s
    /// `IDS` and `ISS`; without it, the syndrome is IMPLEMENTATION DEFINED
    /// and none is chosen here (`ISS` 0). Taking it clears `HCR_EL2.VSE`
    /// (`ClearPendingVirtualSError`); a virtual IRQ or FIQ stays pending until
    /// EL2 clears `VI` or `VF`.
    fn take_virtual_interrupt(&mut self, interrupt: VirtualInterrupt) -> ExceptionTaken {
        let (vector_kind, kind, record) = match interrupt {
            VirtualInterrupt::Irq => (
                VectorKind::Irq,
                ExceptionKind::VirtualIrq,
                EntryRecord::Nothing,
            ),
            VirtualInterrupt::Fiq => (
                VectorKind::Fiq,
                ExceptionKind::VirtualFiq,
                EntryRecord::Nothing,
            ),
            VirtualInterrupt::SError => {
                let iss = if self.implements(Feature::Ras) {
                    self.el2().vsesr & VSESR_MASK
                } else {
                    0
                };
                if let Some(el2) = self.el2_mut() {
                    el2.hcr &= !HCR_VSE;
                }
                let esr = (EC_SERROR << 26) | (1 << 25) | iss;
                (
                    VectorKind::SError,
                    ExceptionKind::VirtualSError,
                    EntryRecord::Syndrome { esr, far: None },
                )
            }
        };
        let from = self.pstate.el;
        let pc = self.pc;
        let vector =
            self.take_exception(Target::El1, vector_kind, pc, self.pstate.to_spsr(), record);
        ExceptionTaken {
            kind,
            from,
            to: El::El1,
            return_address: pc,
            vector,
            syndrome: match record {
                EntryRecord::Nothing => 0,
                EntryRecord::Syndrome { esr, .. } => esr,
            },
            fault_address: None,
        }
    }

    /// `AArch64_TakeException` to `target` (never below the current level),
    /// saving `spsr`, the image of `PSTATE` the exception keeps, and
    /// `record` in the target's syndrome registers. Returns the vector now
    /// executing. `PSTATE.SM` and `PSTATE.ZA` are not part of that image and
    /// carry over unchanged.
    ///
    /// `target` is a level this processor implements ([`Target`]); on a
    /// processor without EL2 the EL2 registers [`Cpu::exception_regs`]
    /// would give are absent, so its EL2 entry is not compiled.
    fn take_exception(
        &mut self,
        target: Target<L>,
        kind: VectorKind,
        return_addr: u64,
        spsr: u64,
        record: EntryRecord,
    ) -> u64 {
        let to = target.el();
        self.maybe_zero_sve_uppers(to);
        let from = self.pstate.el;
        let quadrant = if to > from {
            0x400
        } else if self.pstate.sp_sel {
            0x200
        } else {
            0x000
        };
        let regs = self.exception_regs(target);
        *regs.spsr = spsr;
        *regs.elr = return_addr;
        match record {
            EntryRecord::Nothing => {}
            EntryRecord::Syndrome { esr, far } => {
                *regs.esr = esr;
                if let Some(far) = far {
                    *regs.far = far;
                }
            }
        }
        let (sctlr, vbar) = (regs.sctlr, regs.vbar);
        // With FEAT_PAN, PAN is set on taking an exception to EL1 when the
        // target's SPAN is 0 (to EL2 only in the EL2&0 regime, which needs
        // FEAT_VHE) and otherwise kept; UAO is cleared; SSBS becomes the
        // target's DSSBS; DIT is kept.
        let pan = self.pstate.pan
            || (to == El::El1 && self.implements(Feature::Pan) && sctlr & sctlr::SPAN == 0);
        let ssbs = self.implements(Feature::Ssbs) && sctlr & sctlr::DSSBS != 0;
        self.pstate = Pstate {
            n: self.pstate.n,
            z: self.pstate.z,
            c: self.pstate.c,
            v: self.pstate.v,
            d: true,
            a: true,
            i: true,
            f: true,
            el: to,
            sp_sel: true,
            ss: false,
            il: false,
            btype: Btype::None,
            pan,
            uao: false,
            dit: self.pstate.dit,
            ssbs,
        };
        self.pc = self.branch_addr((vbar & !0x7FF).wrapping_add(quadrant + kind.offset()));
        self.history.jump(self.pc, self.retired);
        self.monitor.armed = false;
        self.stepped = Stepped::Nothing;
        self.activity = Activity::Running;
        self.pc
    }

    /// Arm `DebugTargetFrom` on a processor with no EL3 and no Secure
    /// state: EL2 where `HCR_EL2.TGE` or `MDCR_EL2.TDE` routes debug
    /// exceptions there, EL1 otherwise.
    fn debug_target(&self) -> El {
        if self.tge() || self.mdcr_any(MDCR_TDE) {
            El::El2
        } else {
            El::El1
        }
    }

    /// Arm `AArch64_GenerateDebugExceptionsFrom(from_el, ss, mask)` on a
    /// processor with no EL3 and no Secure state, which is never in Debug
    /// state (so not `Halted()`): unless the OS Lock or the OS Double Lock
    /// is locked, debug exceptions are enabled from below their target
    /// ([`Cpu::debug_target`]) and, with `MDSCR_EL1.KDE` set and `mask`
    /// (PSTATE.D) clear, from the target itself.
    pub(super) fn debug_exceptions_enabled_from(&self, from_el: El, mask: bool) -> bool {
        if self.sys.oslsr_el1 & OSLSR_OSLK != 0 || self.double_lock_status() {
            return false;
        }
        let target = self.debug_target();
        if from_el == target {
            self.sys.mdscr_el1 & MDSCR_KDE != 0 && !mask
        } else {
            target > from_el
        }
    }

    /// Arm `DoubleLockStatus`: with FEAT_DoubleLock, `OSDLR_EL1.DLK` set
    /// and `DBGPRCR_EL1.CORENPDRQ` clear, the processor never being in
    /// Debug state.
    fn double_lock_status(&self) -> bool {
        self.implements(Feature::DoubleLock)
            && self.sys.osdlr_el1 & OSDLR_DLK != 0
            && self.sys.dbgprcr_el1 & DBGPRCR_CORENPDRQ == 0
    }

    /// Arm `IllegalExceptionReturn` for a return from the current level
    /// with `spsr`, and the level a legal one returns to: illegal to
    /// AArch32 (`M[4]`), to a level this processor lacks or above the
    /// current one, with `M[1]` set, to EL0 with `SP_EL0` not selected, and
    /// to EL1 while `HCR_EL2.TGE` is set.
    fn exception_return_target(&self, spsr: u64) -> Option<El> {
        let m = spsr & 0x1F;
        let target = El::from_index((m >> 2) & 3)?;
        let legal = m & 0x10 == 0
            && m & 0x2 == 0
            && target <= self.pstate.el
            && (target != El::El2 || self.have_el2())
            && !(target == El::El0 && m & 1 == 1)
            && !(target == El::El1 && self.tge());
        legal.then_some(target)
    }

    /// Arm `SPSR_ELx[]` and `ELR_ELx[]` of the level an `ERET` returns from
    /// ([`Cpu::return_level`]): the registers [`Cpu::exception_regs`] gives
    /// that level, read.
    fn return_regs(&self) -> ReturnRegs {
        match self.return_level() {
            Target::El1 => ReturnRegs {
                spsr: self.sys.spsr_el1,
                elr: self.sys.elr_el1,
            },
            Target::El2(proof) => {
                let regs = &self.el2_state.proven_ref(proof).regs;
                ReturnRegs {
                    spsr: regs.spsr,
                    elr: regs.elr,
                }
            }
        }
    }

    /// Arm `ELR_ELx[]`: the exception link register of the level an `ERET`
    /// returns from.
    #[inline]
    pub(crate) fn elr_current(&self) -> u64 {
        self.return_regs().elr
    }

    /// `AArch64_ExceptionReturn(target, SPSR_ELx)` at the current level:
    /// what `ERET` does with `ELR_ELx` and `ERETAA`/`ERETAB` with `ELR_ELx`
    /// authenticated. Returns the new PC. Every return, legal or not, clears
    /// the local exclusive monitor and sets the Event Register
    /// (`ClearExclusiveLocal`, `SendEventLocal`).
    pub(crate) fn exception_return(&mut self, target: u64) -> u64 {
        let spsr = self.return_regs().spsr;
        self.monitor.armed = false;
        let legal_target = self.exception_return_target(spsr);

        // Arm `DebugExceptionReturnSS`: PSTATE.SS takes SPSR.SS only for a
        // return, with `MDSCR_EL1.SS` set, from where debug exceptions are
        // disabled to where they are enabled, with SPSR.D as the
        // destination's mask; it is 0 otherwise. An illegal return's
        // destination is the current EL. Arm `SetPSTATEFromPSR`: writing
        // PSTATE.SS leaves Software Step not to advance past this
        // instruction.
        let dest_el = legal_target.unwrap_or(self.pstate.el);
        let ss = spsr & (1 << 21) != 0
            && self.sys.mdscr_el1 & MDSCR_SS != 0
            && !self.debug_exceptions_enabled_from(self.pstate.el, self.pstate.d)
            && self.debug_exceptions_enabled_from(dest_el, spsr & (1 << 9) != 0);
        self.should_advance_ss = false;
        self.stepped = Stepped::Nothing;

        // SPSR_ELx holds no bit of a feature this processor lacks: an MSR
        // writes only `spsr_mask`'s bits, and a snapshot that holds others
        // is refused.
        let p = &mut self.pstate;
        p.n = spsr & (1 << 31) != 0;
        p.z = spsr & (1 << 30) != 0;
        p.c = spsr & (1 << 29) != 0;
        p.v = spsr & (1 << 28) != 0;
        p.ss = ss;
        p.d = spsr & (1 << 9) != 0;
        p.a = spsr & (1 << 8) != 0;
        p.i = spsr & (1 << 7) != 0;
        p.f = spsr & (1 << 6) != 0;
        // Arm `SetPSTATEFromPSR` restores NZCV, SS, DAIF and PAN whether or
        // not the return is legal.
        p.pan = spsr & Pstate::PAN != 0;
        match legal_target {
            Some(el) => {
                p.el = el;
                p.sp_sel = spsr & 1 != 0;
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
            None => {
                p.il = true;
                p.btype = Btype::None;
            }
        }
        // Arm `AArch64_ExceptionReturn`: `SendEventLocal` after
        // `SetPSTATEFromPSR`, legal return or not.
        self.send_event();
        target
    }
}
