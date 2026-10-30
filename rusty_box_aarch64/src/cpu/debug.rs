//! Self-hosted debug (Arm ARM D2): Software Step, Breakpoint and Watchpoint
//! exceptions, which all target EL1 on a processor with no EL2.
//!
//! None of this costs an instruction that no debugger watches: a trace runs
//! its instructions unchecked while [`Cpu::step_each`] is clear, and a data
//! access that hits the TLB is unchecked while [`Cpu::watch_armed`] is clear.
//! Both are derived from the registers, by [`Cpu::debug_controls_changed`].

use super::exception::{Exception, ExceptionClass};
use super::exec::Fault;
use super::features::Feature;
use super::mmu::DataDirection;
use super::sysreg::{MDSCR_MDE, MDSCR_SS};
use super::{Cpu, El, SysRegs};

/// `ID_AA64DFR0_EL1.BRPs` + 1.
const BREAKPOINTS: usize = 2;

/// The context-aware breakpoint: `CTX_CMPs` + 1 of them, the highest
/// numbered (Arm `IsContextAwareBreakpoint`).
const CONTEXT_AWARE_BREAKPOINT: usize = BREAKPOINTS - 1;

/// The fields `DBGBCR<n>` and `DBGWCR<n>` share.
const ENABLE: u64 = 1;
const HMC: u64 = 1 << 13;
const SSC: u64 = 0b11 << 14;
const LBN_SHIFT: u32 = 16;

/// IFSC and DFSC: a debug exception.
const DEBUG_EXCEPTION: u32 = 0b10_0010;

/// `ISS.ISV` and `ISS.EX` of a Software Step exception.
const STEP_ISV: u32 = 1 << 24;
const STEP_EX: u32 = 1 << 6;

/// `ISS.WnR`, `CM`, `WPF`, `WPTV` and the bottom bit of `WPT` of a
/// Watchpoint exception. FnP and FnV stay 0: `FAR_EL1` holds the byte that
/// matched.
const WATCH_WNR: u32 = 1 << 6;
const WATCH_CM: u32 = 1 << 8;
const WATCH_WPF: u32 = 1 << 16;
const WATCH_WPTV: u32 = 1 << 17;
const WATCH_WPT_SHIFT: u32 = 18;

/// `DBGWCR<n>.WT`: the watchpoint is linked.
const WATCH_LINKED: u64 = 1 << 20;

/// What an access does, for the watchpoints that may match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchedAccess {
    Load,
    Store,
    ReadModifyWrite,
    /// `DC IVAC`, which Arm's `AArch64_WatchpointMatch` counts with the
    /// stores, and `IsWatchpointableAccess` alone of the cache maintenance
    /// instructions admits.
    CacheInvalidate,
}

impl From<DataDirection> for WatchedAccess {
    #[inline]
    fn from(direction: DataDirection) -> Self {
        match direction {
            DataDirection::Read => Self::Load,
            DataDirection::Write => Self::Store,
            DataDirection::ReadWrite => Self::ReadModifyWrite,
        }
    }
}

impl WatchedAccess {
    /// Arm `AArch64_WatchpointMatch`'s `ls_match` for `DBGWCR<n>.LSC`.
    const fn watched_by(self, lsc: u64) -> bool {
        let reads = matches!(self, Self::Load | Self::ReadModifyWrite);
        let writes = !matches!(self, Self::Load);
        (lsc & 0b01 != 0 && reads) || (lsc & 0b10 != 0 && writes)
    }

    /// `ISS.WnR` and `CM` of a match on a watchpoint with `lsc`: Arm
    /// `AArch64_CheckWatchpoint` reports a read-modify-write as a load when
    /// the watchpoint would have matched a load, and `AArch64_FaultSyndrome`
    /// sets both for a cache maintenance instruction.
    const fn syndrome(self, lsc: u64) -> u32 {
        match self {
            Self::Load => 0,
            Self::Store => WATCH_WNR,
            Self::ReadModifyWrite => {
                if lsc & 0b01 != 0 {
                    0
                } else {
                    WATCH_WNR
                }
            }
            Self::CacheInvalidate => WATCH_WNR | WATCH_CM,
        }
    }
}

/// How far around the bytes of an access a watchpoint looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchedRange {
    /// The bytes accessed.
    Exact,
    /// Those bytes, out to the 16-byte boundaries around them: a relaxed
    /// access (Arm `IsRelaxedWatchpointAccess`), under Arm's listed outcome
    /// of `Unpredictable_16BYTEROUNDEDDOWNACCESS` and `...UPACCESS`. `WPF`
    /// then says a match may be of a byte it did not access.
    RoundedTo16Bytes,
}

impl WatchedRange {
    /// The first byte looked at of an access from `va`.
    const fn first(self, va: u64) -> u64 {
        match self {
            Self::Exact => va,
            Self::RoundedTo16Bytes => va & !15,
        }
    }

    /// The byte past the last one looked at of an access that ends before
    /// `end`.
    const fn end(self, end: u64) -> u64 {
        match self {
            Self::Exact => end,
            Self::RoundedTo16Bytes => end.wrapping_add(15) & !15,
        }
    }

    /// `ISS.WPF`.
    const fn syndrome(self) -> u32 {
        match self {
            Self::Exact => 0,
            Self::RoundedTo16Bytes => WATCH_WPF,
        }
    }
}

/// `ID_AA64DFR0_EL1.WRPs` + 1.
const WATCHPOINTS: usize = 2;

/// The instruction that took Software Step from active-not-pending to
/// active-pending, whose exception's syndrome reports it (D2.11.9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stepped {
    /// None did: an exception return or the unmasking of debug exceptions
    /// led here, so there is no syndrome to record.
    Nothing,
    Instruction,
    /// One whose mnemonic starts with LDX or LDAX.
    LoadExclusive,
}

impl Stepped {
    /// `ISS.ISV` and `ISS.EX` (Table D2-23).
    const fn syndrome(self) -> u32 {
        match self {
            Self::Nothing => 0,
            Self::Instruction => STEP_ISV,
            Self::LoadExclusive => STEP_ISV | STEP_EX,
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Nothing => 0,
            Self::Instruction => 1,
            Self::LoadExclusive => 2,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Nothing),
            1 => Some(Self::Instruction),
            2 => Some(Self::LoadExclusive),
            _ => None,
        }
    }
}

/// Whether `word` is LDXR, LDAXR, LDXRB, LDAXRB, LDXRH, LDAXRH, LDXP or
/// LDAXP: the load/store exclusive group, with `o2` clear and `L` set.
/// Its 32-bit and 64-bit forms hold a register pair with bit 21 set; below
/// them that encoding is CASP.
const fn is_load_exclusive(word: u32) -> bool {
    word & 0x3FC0_0000 == 0x0840_0000 && (word >> 31 != 0 || word & (1 << 21) == 0)
}

/// What `DBGBCR<n>.BT` asks breakpoint `n` to compare, on a processor with
/// no EL2, no FEAT_BWE and no AArch32 (Arm `AArch64_ReservedBreakpointType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakpointType {
    Address,
    LinkedAddress,
    Context,
    /// A context, for the breakpoints and watchpoints that link to it. It
    /// generates no event of its own.
    LinkingContext,
    /// A reserved value, which behaves as disabled, Arm's
    /// `Unpredictable_RESBPTYPE` default.
    Reserved,
}

/// Arm `AArch64_StateMatch`, before any linking: whether a breakpoint or
/// watchpoint with `control` applies to an access at `el`. With neither EL2
/// nor EL3, a nonzero HMC or SSC is reserved, and behaves as disabled, Arm's
/// `CheckValidStateMatch`; PxC 0b00 matches no level.
const fn state_matches(control: u64, el: El) -> bool {
    let pxc = control >> 1;
    control & (HMC | SSC) == 0
        && match el {
            El::El0 => pxc & 0b10 != 0,
            El::El1 => pxc & 0b01 != 0,
        }
}

/// The breakpoint a breakpoint or watchpoint with `control` links to.
const fn linked_breakpoint(control: u64) -> usize {
    ((control >> LBN_SHIFT) & 0xF) as usize
}

/// Whether the eight bits of `DBGWCR<n>.BAS` select contiguous bytes, as
/// Arm `AArch64_WatchpointByteMatch` tests it, in eight-bit arithmetic.
const fn bas_is_contiguous(bas: u8) -> bool {
    let lowest = bas & bas.wrapping_neg();
    let above = bas.wrapping_add(lowest);
    above & above.wrapping_sub(1) == 0
}

/// Whether the watchpoint with `control` and `DBGWVR<n>` `value` watches the
/// byte at `address`: Arm `AArch64_WatchpointByteMatch`, with Arm's
/// `DebugAddrTop` at 48 and its listed default for every CONSTRAINED
/// UNPREDICTABLE case, `FALSE` for `Unpredictable_WPMASKANDBAS`,
/// `WPBASCONTIGUOUS`, `WPMASKEDBITS` and `DBGxVR_RESS`, and disabled for
/// `RESWPMASK`.
const fn watches_byte(control: u64, value: u64, address: u64) -> bool {
    let bas = (control >> 5) as u8;
    let mask = ((control >> 24) & 0x1F) as u32;
    let bottom = if value & 4 != 0 { 2 } else { 3 };
    if (mask > 0 && bas != 0xFF) || !bas_is_contiguous(bas) || matches!(mask, 1 | 2) {
        return false;
    }
    let select = address & ((1 << bottom) - 1);
    if bas & (1 << select) == 0 {
        return false;
    }
    let lowest = if mask > bottom { mask } else { bottom };
    let compared = ((1 << 49) - 1) & !((1 << lowest) - 1);
    let masked_out = ((1 << lowest) - 1) & !((1 << bottom) - 1);
    (address ^ value) & compared == 0 && value & masked_out == 0
}

/// [`Cpu::watch_armed`] of the registers `sys`.
pub(super) fn watch_armed(sys: &SysRegs) -> bool {
    sys.mdscr_el1 & MDSCR_MDE != 0 && sys.dbgwcr.iter().any(|control| control & ENABLE != 0)
}

impl Cpu {
    /// Recompute what the instruction loop and the TLB check, from the
    /// registers: after a write of one of them, and on a restore. The
    /// running trace ends, for the next instruction to see the change; a
    /// change of [`Cpu::watch_armed`] flushes the TLB, whose entries are
    /// filled by its rule.
    pub(crate) fn debug_controls_changed(&mut self) {
        let was_armed = self.watch_armed;
        self.derive_debug_controls();
        self.trace_break = true;
        if self.watch_armed != was_armed {
            self.flush_tlb();
        }
    }

    /// [`Cpu::step_each`] and [`Cpu::watch_armed`], from the registers.
    pub(crate) fn derive_debug_controls(&mut self) {
        let mde = self.sys.mdscr_el1 & MDSCR_MDE != 0;
        let breakpoint_enabled = self.sys.dbgbcr.iter().any(|control| control & ENABLE != 0);
        self.step_each = self.sys.mdscr_el1 & MDSCR_SS != 0 || (mde && breakpoint_enabled);
        self.watch_armed = watch_armed(&self.sys);
    }

    /// Arm `AArch64_GenerateDebugExceptions`.
    #[inline]
    fn debug_exceptions_enabled(&self) -> bool {
        self.debug_exceptions_enabled_from(self.pstate.el, self.pstate.d)
    }

    /// Arm `CheckSoftwareStep`, before an instruction: the Software Step
    /// exception of the active-pending state, ahead of every other
    /// synchronous exception.
    #[inline]
    pub(crate) fn check_software_step(&mut self) -> Result<(), Fault> {
        self.should_advance_ss = true;
        if self.sys.mdscr_el1 & MDSCR_SS != 0 && !self.pstate.ss && self.debug_exceptions_enabled()
        {
            let iss = self.stepped.syndrome() | DEBUG_EXCEPTION;
            return Err(Exception::new(ExceptionClass::SoftwareStep, iss).into());
        }
        Ok(())
    }

    /// Arm `SSAdvance`, after the instruction `word` completed, or as an SVC
    /// takes its exception, with no word to tell: from active-not-pending to
    /// active-pending, unless the instruction wrote PSTATE.SS. An
    /// instruction that leaves debug exceptions disabled leaves Software
    /// Step inactive instead, and what enters active-pending from there has
    /// no syndrome (Arm ARM D2.11.9.1).
    #[inline]
    pub(crate) fn ss_advance(&mut self, word: Option<u32>) {
        if self.should_advance_ss && self.sys.mdscr_el1 & MDSCR_SS != 0 && self.pstate.ss {
            self.pstate.ss = false;
            self.stepped = match word {
                _ if !self.debug_exceptions_enabled() => Stepped::Nothing,
                Some(word) if is_load_exclusive(word) => Stepped::LoadExclusive,
                _ => Stepped::Instruction,
            };
        }
        self.should_advance_ss = false;
    }

    /// Arm `AArch64_CheckDebug` of the fetch of the instruction at `pc`,
    /// translated and not yet read. A Breakpoint exception leaves `FAR_EL1`
    /// as it is.
    #[inline]
    pub(crate) fn check_breakpoints(&self, pc: u64) -> Result<(), Fault> {
        if self.sys.mdscr_el1 & MDSCR_MDE != 0
            && self.debug_exceptions_enabled()
            && (0..BREAKPOINTS).any(|n| self.breakpoint_fires(n, pc))
        {
            return Err(Exception::new(ExceptionClass::Breakpoint, DEBUG_EXCEPTION).into());
        }
        Ok(())
    }

    /// Arm `AArch64_BreakpointMatch` of breakpoint `n`.
    fn breakpoint_fires(&self, n: usize, pc: u64) -> bool {
        let control = self.sys.dbgbcr[n];
        if control & ENABLE == 0 || !state_matches(control, self.pstate.el) {
            return false;
        }
        match self.breakpoint_type(n) {
            BreakpointType::Address => self.address_matches(n, pc),
            BreakpointType::LinkedAddress => {
                self.address_matches(n, pc) && self.linked_context_matches(control)
            }
            BreakpointType::Context => self.context_matches(n),
            BreakpointType::LinkingContext | BreakpointType::Reserved => false,
        }
    }

    /// `DBGBCR<n>.BT` of breakpoint `n`.
    fn breakpoint_type(&self, n: usize) -> BreakpointType {
        let context_aware = n == CONTEXT_AWARE_BREAKPOINT;
        let v8p2 = self.implements(Feature::Debugv8p2);
        match (self.sys.dbgbcr[n] >> 20) & 0xF {
            0b0000 => BreakpointType::Address,
            0b0001 => BreakpointType::LinkedAddress,
            0b0010 if context_aware => BreakpointType::Context,
            0b0011 if context_aware => BreakpointType::LinkingContext,
            0b0110 if context_aware && v8p2 => BreakpointType::Context,
            0b0111 if context_aware && v8p2 => BreakpointType::LinkingContext,
            _ => BreakpointType::Reserved,
        }
    }

    /// Arm `AArch64_BreakpointValueMatch` of an address, with Arm's
    /// `DebugAddrTop` at 48: bits 48:2. Bits 63:49 of the register are
    /// compared with nothing, Arm's `Unpredictable_DBGxVR_RESS` default.
    const fn address_matches(&self, n: usize, pc: u64) -> bool {
        const COMPARED: u64 = ((1 << 49) - 1) & !3;
        (pc ^ self.sys.dbgbvr[n]) & COMPARED == 0
    }

    /// Arm `AArch64_BreakpointValueMatch` of a context: `CONTEXTIDR_EL1`.
    const fn context_matches(&self, n: usize) -> bool {
        self.sys.contextidr_el1 as u32 == self.sys.dbgbvr[n] as u32
    }

    /// The linking half of Arm `AArch64_StateMatch`, for the breakpoint or
    /// watchpoint with `control`: it needs an enabled context-aware
    /// breakpoint, with linking enabled, in its context. Any other is no
    /// match, Arm's `Unpredictable_BPNOTLINKING` default.
    fn linked_context_matches(&self, control: u64) -> bool {
        let n = linked_breakpoint(control);
        n == CONTEXT_AWARE_BREAKPOINT
            && self.sys.dbgbcr[n] & ENABLE != 0
            && self.breakpoint_type(n) == BreakpointType::LinkingContext
            && self.context_matches(n)
    }

    /// Arm `AArch64_CheckDebug` of a data access of `bytes` at `va`, `kind`,
    /// with `el`'s permissions, once its translation and permission checks
    /// have passed: the last watchpoint to match, as Arm's loop leaves it.
    /// `FAR_EL1` is the first byte it matched, and with FEAT_Debugv8p2 the
    /// syndrome names it, the IMPLEMENTATION DEFINED choice of `WPTV` 1.
    #[inline]
    pub(crate) fn check_watchpoints(
        &self,
        va: u64,
        bytes: u64,
        kind: WatchedAccess,
        el: El,
        range: WatchedRange,
    ) -> Result<(), Fault> {
        if !self.watch_armed || !self.debug_exceptions_enabled() {
            return Ok(());
        }
        let first = range.first(va);
        let length = range.end(va.wrapping_add(bytes)).wrapping_sub(first);
        let mut raised = None;
        for n in 0..WATCHPOINTS {
            if let Some(address) = self.watchpoint_match(n, first, length, kind, el) {
                let lsc = (self.sys.dbgwcr[n] >> 3) & 3;
                let number = if self.implements(Feature::Debugv8p2) {
                    WATCH_WPTV | ((n as u32) << WATCH_WPT_SHIFT)
                } else {
                    0
                };
                let iss = kind.syndrome(lsc) | range.syndrome() | number | DEBUG_EXCEPTION;
                raised = Some(Exception::new(ExceptionClass::Watchpoint, iss).with_far(address));
            }
        }
        match raised {
            Some(exception) => Err(exception.into()),
            None => Ok(()),
        }
    }

    /// Arm `AArch64_WatchpointMatch` of watchpoint `n`: the first byte of
    /// the access it watches, if any.
    fn watchpoint_match(
        &self,
        n: usize,
        va: u64,
        bytes: u64,
        kind: WatchedAccess,
        el: El,
    ) -> Option<u64> {
        let control = self.sys.dbgwcr[n];
        let lsc = (control >> 3) & 3;
        if control & ENABLE == 0
            || !state_matches(control, el)
            || !kind.watched_by(lsc)
            || (control & WATCH_LINKED != 0 && !self.linked_context_matches(control))
        {
            return None;
        }
        let value = self.sys.dbgwvr[n];
        (0..bytes)
            .map(|byte| va.wrapping_add(byte))
            .find(|&address| watches_byte(control, value, address))
    }
}
