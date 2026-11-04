//! Debugger code breakpoints: addresses a processor stops before executing,
//! with nothing written into guest memory and no debug register touched, so
//! the guest cannot tell one is set.
//!
//! Bochs checks its debugger's code breakpoints at every instruction
//! boundary while `cpu_loop_debugger` runs one instruction per trace
//! (cpu.cc `dbg_instruction_epilog`, `dbg_check_code_bpoints`,
//! `dbg_code_bp_after_fetch`). This port keeps whole traces instead
//! (divergence D20 in `docs/bochs-parity-divergences.md`): while a set is
//! armed, a trace is cut before every instruction that sits at a
//! breakpoint's page offset and no trace is merged or linked, so every
//! breakpoint address begins a trace; the loop checks the boundary at each
//! trace start and where it leaves the processor. A trace is keyed by its
//! physical page and may run under any linear mapping of it, which is why
//! the cut is by page offset and the check is exact.

use super::exec_ctx::ExecCtx;
use super::instrumentation::Instrumentation;
use super::paging::DbgWalk;
use super::BxCpuC;

/// How many linear code breakpoints a machine holds: Bochs config.h
/// `BX_DBG_MAX_LIN_BPOINTS`.
pub const MAX_LINEAR_BREAKPOINTS: usize = 16;
/// How many physical code breakpoints a machine holds: Bochs config.h
/// `BX_DBG_MAX_PHY_BPOINTS`.
pub const MAX_PHYSICAL_BREAKPOINTS: usize = 16;

/// Where a code breakpoint stops a processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodeBreakpoint {
    /// Before the instruction at this linear address (CS base plus RIP), in
    /// whatever address space reaches it: Bochs's `lbreak`.
    Linear(u64),
    /// Before the instruction at this physical address: Bochs's `pbreak`.
    Physical(u64),
}

/// A processor that stopped before the instruction at a code breakpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CodeBreakpointHit {
    /// The processor's index (0 is the boot processor).
    pub cpu: usize,
    pub breakpoint: CodeBreakpoint,
    /// The linear address of the instruction it stopped before.
    pub linear: u64,
}

/// Why a set of code breakpoints was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BreakpointRefusal {
    #[error("more than {MAX_LINEAR_BREAKPOINTS} linear code breakpoints")]
    TooManyLinear,
    #[error("more than {MAX_PHYSICAL_BREAKPOINTS} physical code breakpoints")]
    TooManyPhysical,
    /// The engine runs the guest on the host's processor, where nothing of
    /// this port sees an instruction boundary.
    #[error("this machine's engine does not run the guest through the interpreter, so it cannot stop before an instruction")]
    EngineCannotStop,
}

/// Whether an engine stops before an instruction at a code breakpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakpointSupport {
    /// It runs the guest through this port's interpreter loop, which checks.
    Honoured,
    /// It runs the guest elsewhere; a set of breakpoints is refused.
    Refused,
}

/// Which kind of breakpoint a recorded hit was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum HitKind {
    Linear = 0,
    Physical = 1,
}

/// Whether a hit waits for the machine to collect it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum HitLatch {
    Clear = 0,
    Pending = 1,
}

/// Whether a boundary has been reported to the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Reported {
    Nothing = 0,
    /// The boundary at `reported_icount` and `reported_linear`.
    Boundary = 1,
}

/// A hit the machine collects from a processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PendingHit {
    pub(crate) breakpoint: CodeBreakpoint,
    pub(crate) linear: u64,
}

/// The breakpoint, if any, at the boundary a processor stands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BreakpointHere {
    breakpoint: CodeBreakpoint,
    linear: u64,
}

/// One processor's code breakpoints and what it has reported of them.
///
/// Every field is valid as zero bytes — a processor is built over a zeroed
/// allocation (`cpu/builder.rs`, `init_cpu_at`) — and zero means no
/// breakpoints, no hit, nothing reported. Each enum below has its zero
/// variant for that reason.
#[derive(Clone, Debug)]
pub(crate) struct CodeBreakpointState {
    linear: [u64; MAX_LINEAR_BREAKPOINTS],
    linear_count: u8,
    physical: [u64; MAX_PHYSICAL_BREAKPOINTS],
    physical_count: u8,
    latch: HitLatch,
    hit_kind: HitKind,
    hit_address: u64,
    hit_linear: u64,
    /// The boundary last reported: a run that resumes on it executes its
    /// instruction rather than stopping again (Bochs cpu.cc
    /// `dbg_loop_icount`), and any other boundary is checked.
    reported: Reported,
    reported_icount: u64,
    reported_linear: u64,
}

impl Default for CodeBreakpointState {
    fn default() -> Self {
        Self {
            linear: [0; MAX_LINEAR_BREAKPOINTS],
            linear_count: 0,
            physical: [0; MAX_PHYSICAL_BREAKPOINTS],
            physical_count: 0,
            latch: HitLatch::Clear,
            hit_kind: HitKind::Linear,
            hit_address: 0,
            hit_linear: 0,
            reported: Reported::Nothing,
            reported_icount: 0,
            reported_linear: 0,
        }
    }
}

impl CodeBreakpointState {
    /// Replaces the set with `breakpoints`; refused whole, leaving the set as
    /// it was, when a kind has more than its slots.
    pub(crate) fn set(&mut self, breakpoints: &[CodeBreakpoint]) -> Result<(), BreakpointRefusal> {
        let linear = breakpoints.iter().filter(|bp| matches!(bp, CodeBreakpoint::Linear(_))).count();
        let physical = breakpoints.len().saturating_sub(linear);
        if linear > MAX_LINEAR_BREAKPOINTS {
            return Err(BreakpointRefusal::TooManyLinear);
        }
        if physical > MAX_PHYSICAL_BREAKPOINTS {
            return Err(BreakpointRefusal::TooManyPhysical);
        }
        let mut next_linear = self.linear.iter_mut();
        let mut next_physical = self.physical.iter_mut();
        for breakpoint in breakpoints {
            match *breakpoint {
                CodeBreakpoint::Linear(address) => {
                    if let Some(slot) = next_linear.next() {
                        *slot = address;
                    }
                }
                CodeBreakpoint::Physical(address) => {
                    if let Some(slot) = next_physical.next() {
                        *slot = address;
                    }
                }
            }
        }
        // Both counts were checked against their slots above.
        self.linear_count = linear as u8;
        self.physical_count = physical as u8;
        Ok(())
    }

    /// Whether any breakpoint is set: the one test the CPU loop pays per
    /// trace start while none is.
    #[inline]
    pub(crate) fn armed(&self) -> bool {
        self.linear_count != 0 || self.physical_count != 0
    }

    fn linear_set(&self) -> &[u64] {
        self.linear.get(..usize::from(self.linear_count)).unwrap_or(&[])
    }

    fn physical_set(&self) -> &[u64] {
        self.physical.get(..usize::from(self.physical_count)).unwrap_or(&[])
    }

    /// Whether a trace being built must end before an instruction at
    /// `page_offset`: some breakpoint sits at that offset in its page.
    pub(crate) fn cuts_trace_at(&self, page_offset: u32) -> bool {
        let offset = u64::from(page_offset & 0xFFF);
        self.linear_set()
            .iter()
            .chain(self.physical_set())
            .any(|&address| address & 0xFFF == offset)
    }

    /// Whether a hit waits for the machine to collect it.
    #[inline]
    pub(crate) fn hit_pending(&self) -> bool {
        self.latch == HitLatch::Pending
    }

    /// The hit waiting for the machine, collected.
    pub(crate) fn take_hit(&mut self) -> Option<PendingHit> {
        match self.latch {
            HitLatch::Clear => None,
            HitLatch::Pending => {
                self.latch = HitLatch::Clear;
                let breakpoint = match self.hit_kind {
                    HitKind::Linear => CodeBreakpoint::Linear(self.hit_address),
                    HitKind::Physical => CodeBreakpoint::Physical(self.hit_address),
                };
                Some(PendingHit {
                    breakpoint,
                    linear: self.hit_linear,
                })
            }
        }
    }

    /// Forgets the reported boundary: a reset starts the instruction count
    /// again, so the count no longer names the same boundary.
    pub(crate) fn forget_reported(&mut self) {
        self.reported = Reported::Nothing;
    }

    fn reported_at(&self, icount: u64, linear: u64) -> bool {
        match self.reported {
            Reported::Nothing => false,
            Reported::Boundary => self.reported_icount == icount && self.reported_linear == linear,
        }
    }

    /// Records `here` as hit at the boundary `icount`, for the machine to
    /// collect, and as reported, so the run that resumes there executes it.
    fn record(&mut self, here: BreakpointHere, icount: u64) {
        match here.breakpoint {
            CodeBreakpoint::Linear(address) => {
                self.hit_kind = HitKind::Linear;
                self.hit_address = address;
            }
            CodeBreakpoint::Physical(address) => {
                self.hit_kind = HitKind::Physical;
                self.hit_address = address;
            }
        }
        self.latch = HitLatch::Pending;
        self.hit_linear = here.linear;
        self.reported = Reported::Boundary;
        self.reported_icount = icount;
        self.reported_linear = here.linear;
    }
}

impl<T: Instrumentation> BxCpuC<T> {
    /// Replaces this processor's code breakpoints and drops every cached
    /// trace, so each trace is built again with the cuts the new set needs.
    pub(crate) fn set_code_breakpoints(&mut self, breakpoints: &[CodeBreakpoint]) -> Result<(), BreakpointRefusal> {
        self.code_breakpoints.set(breakpoints)?;
        self.i_cache.flush_all();
        Ok(())
    }
}

impl<T: Instrumentation> ExecCtx<'_, T> {
    /// The breakpoint at the instruction boundary this processor stands on,
    /// unless that boundary was already reported (Bochs cpu.cc
    /// `dbg_check_code_bpoints`): a linear breakpoint matches CS base plus
    /// RIP; a physical one the instruction's physical address, from the fetch
    /// window when RIP lies in it and through the page tables when not.
    fn code_breakpoint_here(&mut self) -> Option<BreakpointHere> {
        let rip = self.rip();
        let linear = if self.long64_mode() {
            self.get_laddr64(super::decoder::BxSegregs::Cs as usize, rip)
        } else {
            u64::from(self.get_laddr32(super::decoder::BxSegregs::Cs as usize, rip as u32))
        };
        if self.code_breakpoints.reported_at(self.icount, linear) {
            return None;
        }
        if let Some(&address) = self.code_breakpoints.linear_set().iter().find(|&&address| address == linear) {
            return Some(BreakpointHere {
                breakpoint: CodeBreakpoint::Linear(address),
                linear,
            });
        }
        if self.code_breakpoints.physical_set().is_empty() {
            return None;
        }
        let eip_biased = rip.wrapping_add(self.eip_page_bias);
        let physical = if eip_biased < u64::from(self.eip_page_window_size) {
            self.p_addr_fetch_page.wrapping_add(eip_biased)
        } else {
            self.dbg_xlate_linear2phy(linear, DbgWalk::Guest)?
        };
        self.code_breakpoints
            .physical_set()
            .iter()
            .find(|&&address| address == physical)
            .map(|&address| BreakpointHere {
                breakpoint: CodeBreakpoint::Physical(address),
                linear,
            })
    }

    /// Whether this processor stops at the boundary it stands on: a
    /// breakpoint is there and the boundary was not reported. A stop is
    /// recorded for the machine to collect.
    pub(super) fn stops_at_code_breakpoint(&mut self) -> bool {
        match self.code_breakpoint_here() {
            Some(here) => {
                let icount = self.icount;
                self.code_breakpoints.record(here, icount);
                true
            }
            None => false,
        }
    }
}
