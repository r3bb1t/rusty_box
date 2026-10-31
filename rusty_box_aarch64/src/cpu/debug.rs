//! Self-hosted debug (Arm ARM D2): Software Step, Breakpoint and Watchpoint
//! exceptions, taken where `AArch64_SoftwareStepException`,
//! `AArch64_BreakpointException` and `AArch64_WatchpointException` take
//! them ([`Cpu::debug_exceptions_enabled_from`] and the exception's class
//! routing). The breakpoints and watchpoints match as Arm's
//! `AArch64_StateMatch` and `AArch64_BreakpointValueMatch` match them on a
//! processor with no EL3, no Secure state and no AArch32: with EL2, `HMC`
//! and `SSC` 0b11 reach EL2, the VMID breakpoint types compare
//! `VTTBR_EL2.VMID`, and the `CONTEXTIDR_EL2` types, which need FEAT_VHE or
//! FEAT_Debugv8p2 as well, compare `CONTEXTIDR_EL2`.
//!
//! None of this costs an instruction that no debugger watches: a trace runs
//! its instructions unchecked while [`Cpu::step_each`] is clear, and a data
//! access that hits the TLB is unchecked while [`Cpu::watch_armed`] is clear.
//! Both are derived from the registers, by [`Cpu::debug_controls_changed`].

use super::dp::ones;
use super::el2::{has_contextidr_el2, VTTBR_VMID8};
use super::exception::{AddressedClass, Exception, SyndromeClass};
use super::exec::Fault;
use super::features::{Feature, Features};
use super::mmu::DataDirection;
use super::sysreg::{MDSCR_MDE, MDSCR_SS};
use super::{Cpu, El, Levels};

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
/// Watchpoint exception. FnP and FnV stay 0: `FAR_ELx` holds the byte that
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
    /// Arm `AArch64_WatchpointMatch`'s `ls_match` for `DBGWCR<n>.LSC`: bit 0
    /// watches what reads, bit 1 what writes, which a read-modify-write
    /// does both of and `DC IVAC` counts as.
    const fn watched_by(self, lsc: u64) -> bool {
        match self {
            Self::Load => lsc & 0b01 != 0,
            Self::Store | Self::CacheInvalidate => lsc & 0b10 != 0,
            Self::ReadModifyWrite => lsc & 0b11 != 0,
        }
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
/// no EL3, no FEAT_BWE, no FEAT_ABLE and no AArch32 (Arm
/// `AArch64_ReservedBreakpointType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakpointType {
    Address,
    LinkedAddress,
    Context(ContextMatch),
    /// A context, for the breakpoints and watchpoints that link to it. It
    /// generates no event of its own.
    LinkingContext(ContextMatch),
    /// A reserved value, which behaves as disabled, Arm's
    /// `Unpredictable_RESBPTYPE` default.
    Reserved,
}

/// What a context breakpoint compares (Arm `AArch64_BreakpointValueMatch`'s
/// `match_cid` and `match_cid1`, `match_vmid`, `match_cid2`):
/// `CONTEXTIDR_EL1` with `DBGBVR<n>` bits 31:0, which matches only at EL0
/// and EL1; the current VMID with bits 39:32 (8-bit VMIDs: no
/// FEAT_VMID16), which matches only at EL0 and EL1 with EL2;
/// `CONTEXTIDR_EL2` with bits 63:32, which matches at every level with
/// EL2; or the VMID or `CONTEXTIDR_EL2` together with `CONTEXTIDR_EL1`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextMatch {
    ContextId,
    Vmid,
    VmidAndContextId,
    El2ContextId,
    El2ContextIdAndContextId,
}

/// Arm `AArch64_StateMatch`, before any linking: whether a breakpoint or
/// watchpoint with `control` applies to an access at `el` on a processor
/// of the levels `L`, which has no EL3, no Secure state and no AArch32.
/// A combination of HMC, SSC and PxC that `CheckValidStateMatch` makes
/// reserved behaves as disabled, Arm's `Unpredictable_RESBPWPCTRL`
/// default: without EL2 every nonzero HMC or SSC, and PxC 0b00; with EL2
/// SSC 0b01 and 0b10, SSC 0b11 but with HMC set and PxC 0b00, and HMC set
/// with SSC 0b00 and PxC 0b00 or 0b10, and HMC clear with PxC 0b00. What
/// is left matches EL0 by PxC bit 1, EL1 by PxC bit 0, and EL2 with HMC
/// set (SSC 0b11 matching EL2 alone).
const fn state_matches<L: Levels>(control: u64, el: El) -> bool {
    let pxc = (control >> 1) & 0b11;
    let hmc = control & HMC != 0;
    let ssc = (control & SSC) >> 14;
    let valid = if L::EL2 {
        match (hmc, ssc, pxc) {
            (true, 0b11, 0b00) => true,
            (_, 0b01 | 0b10 | 0b11, _) => false,
            (true, 0b00, 0b00 | 0b10) => false,
            (false, 0b00, 0b00) => false,
            _ => true,
        }
    } else {
        !hmc && ssc == 0 && pxc != 0
    };
    valid
        && match el {
            El::El0 => pxc & 0b10 != 0,
            El::El1 => pxc & 0b01 != 0,
            El::El2 => hmc,
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

/// Arm `DebugAddrTop` on a processor implementing `features`: the top bit a
/// breakpoint or watchpoint compares, 52 with FEAT_LVA and 48 without (55,
/// FEAT_LVA3's, is no processor's here). Bits above it are compared with
/// nothing, Arm's `Unpredictable_DBGxVR_RESS` default.
const fn debug_addr_top(features: Features) -> u32 {
    if features.has(Feature::Lva) {
        52
    } else {
        48
    }
}

/// Whether the watchpoint with `control` and `DBGWVR<n>` `value` watches the
/// byte at `address`, its bits up to `top` ([`debug_addr_top`]) compared:
/// Arm `AArch64_WatchpointByteMatch`, with its listed default for every
/// CONSTRAINED UNPREDICTABLE case, `FALSE` for `Unpredictable_WPMASKANDBAS`,
/// `WPBASCONTIGUOUS`, `WPMASKEDBITS` and `DBGxVR_RESS`, and disabled for
/// `RESWPMASK`.
const fn watches_byte(control: u64, value: u64, address: u64, top: u32) -> bool {
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
    let compared = ones(top + 1) & !ones(lowest);
    let masked_out = ((1 << lowest) - 1) & !((1 << bottom) - 1);
    (address ^ value) & compared == 0 && value & masked_out == 0
}

impl<L: Levels> Cpu<L> {
    /// Recompute what the instruction loop and the data accesses check,
    /// from the registers, after a write of one of them. The running trace
    /// ends, for the next instruction to see the change.
    pub(crate) fn debug_controls_changed(&mut self) {
        self.derive_debug_controls();
        self.trace_break = true;
    }

    /// [`Cpu::step_each`] and [`Cpu::watch_armed`], from the registers:
    /// after a write of one of them, and on a restore.
    pub(crate) fn derive_debug_controls(&mut self) {
        let mde = self.sys.mdscr_el1 & MDSCR_MDE != 0;
        let breakpoint_enabled = self.sys.dbgbcr.iter().any(|control| control & ENABLE != 0);
        let watchpoint_enabled = self.sys.dbgwcr.iter().any(|control| control & ENABLE != 0);
        self.step_each = self.sys.mdscr_el1 & MDSCR_SS != 0 || (mde && breakpoint_enabled);
        self.watch_armed = mde && watchpoint_enabled;
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
            return Err(Exception::new(SyndromeClass::SoftwareStep, iss).into());
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
    /// translated and not yet read. A Breakpoint exception leaves `FAR_ELx`
    /// as it is.
    #[inline]
    pub(crate) fn check_breakpoints(&self, pc: u64) -> Result<(), Fault> {
        if self.sys.mdscr_el1 & MDSCR_MDE != 0
            && self.debug_exceptions_enabled()
            && (0..BREAKPOINTS).any(|n| self.breakpoint_fires(n, pc))
        {
            return Err(Exception::new(SyndromeClass::Breakpoint, DEBUG_EXCEPTION).into());
        }
        Ok(())
    }

    /// Arm `AArch64_BreakpointMatch` of breakpoint `n`.
    fn breakpoint_fires(&self, n: usize, pc: u64) -> bool {
        let control = self.sys.dbgbcr[n];
        if control & ENABLE == 0 || !state_matches::<L>(control, self.pstate.el) {
            return false;
        }
        match self.breakpoint_type(n) {
            BreakpointType::Address => self.address_matches(n, pc),
            BreakpointType::LinkedAddress => {
                self.address_matches(n, pc) && self.linked_context_matches(control)
            }
            BreakpointType::Context(context) => self.context_matches(n, context),
            BreakpointType::LinkingContext(_) | BreakpointType::Reserved => false,
        }
    }

    /// `DBGBCR<n>.BT` of breakpoint `n`: the address types for every
    /// breakpoint, the context types for the context-aware one alone —
    /// 0b0110 and 0b0111 with FEAT_Debugv8p2 (or FEAT_VHE, which no
    /// processor here has), the VMID types 0b1000 to 0b1011 with EL2, and
    /// the `CONTEXTIDR_EL2` types 0b1100 to 0b1111 with EL2 and either
    /// feature, where `CONTEXTIDR_EL2` exists. The address mismatch types
    /// need FEAT_BWE or AArch32, so they are reserved.
    fn breakpoint_type(&self, n: usize) -> BreakpointType {
        let context_aware = n == CONTEXT_AWARE_BREAKPOINT;
        let v8p2 = self.implements(Feature::Debugv8p2);
        let el2_context = L::EL2 && has_contextidr_el2(self.imp.features);
        match (self.sys.dbgbcr[n] >> 20) & 0xF {
            0b0000 => BreakpointType::Address,
            0b0001 => BreakpointType::LinkedAddress,
            0b0010 if context_aware => BreakpointType::Context(ContextMatch::ContextId),
            0b0011 if context_aware => BreakpointType::LinkingContext(ContextMatch::ContextId),
            0b0110 if context_aware && v8p2 => BreakpointType::Context(ContextMatch::ContextId),
            0b0111 if context_aware && v8p2 => {
                BreakpointType::LinkingContext(ContextMatch::ContextId)
            }
            0b1000 if context_aware && L::EL2 => BreakpointType::Context(ContextMatch::Vmid),
            0b1001 if context_aware && L::EL2 => BreakpointType::LinkingContext(ContextMatch::Vmid),
            0b1010 if context_aware && L::EL2 => {
                BreakpointType::Context(ContextMatch::VmidAndContextId)
            }
            0b1011 if context_aware && L::EL2 => {
                BreakpointType::LinkingContext(ContextMatch::VmidAndContextId)
            }
            0b1100 if context_aware && el2_context => {
                BreakpointType::Context(ContextMatch::El2ContextId)
            }
            0b1101 if context_aware && el2_context => {
                BreakpointType::LinkingContext(ContextMatch::El2ContextId)
            }
            0b1110 if context_aware && el2_context => {
                BreakpointType::Context(ContextMatch::El2ContextIdAndContextId)
            }
            0b1111 if context_aware && el2_context => {
                BreakpointType::LinkingContext(ContextMatch::El2ContextIdAndContextId)
            }
            _ => BreakpointType::Reserved,
        }
    }

    /// Arm `AArch64_BreakpointValueMatch` of an address: bits
    /// [`debug_addr_top`] to 2.
    fn address_matches(&self, n: usize, pc: u64) -> bool {
        let compared = ones(debug_addr_top(self.imp.features) + 1) & !3;
        (pc ^ self.sys.dbgbvr[n]) & compared == 0
    }

    /// Arm `AArch64_BreakpointValueMatch` of a context breakpoint `n`
    /// comparing `context` (no FEAT_VHE, so never in the host):
    /// `CONTEXTIDR_EL1` with `DBGBVR<n>` bits 31:0 and `VTTBR_EL2.VMID`
    /// with bits 39:32 at EL0 or EL1 alone, `CONTEXTIDR_EL2` with bits
    /// 63:32 at any level, the last two with EL2 (`EL2Enabled`, which with
    /// no EL3 is having it).
    fn context_matches(&self, n: usize, context: ContextMatch) -> bool {
        let value = self.sys.dbgbvr[n];
        let below_el2 = matches!(self.pstate.el, El::El0 | El::El1);
        let context_id = || below_el2 && self.sys.contextidr_el1 as u32 == value as u32;
        let vmid = || {
            below_el2 && L::EL2 && (self.el2().vttbr & VTTBR_VMID8) >> 48 == (value >> 32) & 0xFF
        };
        let el2_context_id = || L::EL2 && self.el2().contextidr as u32 == (value >> 32) as u32;
        match context {
            ContextMatch::ContextId => context_id(),
            ContextMatch::Vmid => vmid(),
            ContextMatch::VmidAndContextId => vmid() && context_id(),
            ContextMatch::El2ContextId => el2_context_id(),
            ContextMatch::El2ContextIdAndContextId => el2_context_id() && context_id(),
        }
    }

    /// The linking half of Arm `AArch64_StateMatch`, for the breakpoint or
    /// watchpoint with `control`: it needs an enabled context-aware
    /// breakpoint, with linking enabled, in its context. Any other is no
    /// match, Arm's `Unpredictable_BPNOTLINKING` default.
    fn linked_context_matches(&self, control: u64) -> bool {
        let n = linked_breakpoint(control);
        if n != CONTEXT_AWARE_BREAKPOINT || self.sys.dbgbcr[n] & ENABLE == 0 {
            return false;
        }
        match self.breakpoint_type(n) {
            BreakpointType::LinkingContext(context) => self.context_matches(n, context),
            BreakpointType::Address
            | BreakpointType::LinkedAddress
            | BreakpointType::Context(_)
            | BreakpointType::Reserved => false,
        }
    }

    /// Arm `AArch64_CheckDebug` of a data access of `bytes` at `va`, `kind`,
    /// with `el`'s permissions, once its translation and permission checks
    /// have passed: the last watchpoint to match, as Arm's loop leaves it.
    /// `FAR_ELx` is the first byte it matched, and with FEAT_Debugv8p2 the
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
                raised = Some(Exception::at_address(
                    AddressedClass::Watchpoint,
                    iss,
                    address,
                ));
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
            || !state_matches::<L>(control, el)
            || !kind.watched_by(lsc)
            || (control & WATCH_LINKED != 0 && !self.linked_context_matches(control))
        {
            return None;
        }
        let value = self.sys.dbgwvr[n];
        let top = debug_addr_top(self.imp.features);
        (0..bytes)
            .map(|byte| va.wrapping_add(byte))
            .find(|&address| watches_byte(control, value, address, top))
    }
}
