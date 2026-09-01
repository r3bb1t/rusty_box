//! The architectural state of one x86 processor, in the shape a hypervisor
//! hands it over.
//!
//! This is the bridge between an execution engine that runs the guest natively
//! and the software CPU that finishes what the engine cannot. A trapped exit
//! imports the engine's registers into a shadow [`BxCpuC`], executes there, and
//! exports the result back — so this type is the vocabulary both sides speak,
//! and it carries exactly the state a processor has, not the bookkeeping this
//! emulator keeps beside it.
//!
//! **Deliberately absent**, because they are host bookkeeping rather than
//! architectural state and a hypervisor has no equivalent to hand over:
//! retired-instruction count, tick surplus, interrupt-inhibit bookkeeping, the
//! instruction cache, and the TLBs. A shadow CPU rebuilds those; importing a
//! stale copy of them would be worse than not having them.
//!
//! Also absent, and this one is a limitation rather than a design choice: the
//! nested-virtualization state (VMCS and VMCB caches). A guest that itself runs
//! a hypervisor cannot be serviced through this seam, which is why the engine
//! seam refuses that configuration rather than carrying half of it.

use super::cpu::{BxCpuC, CpuActivityState};
use super::decoder::BxSegregs;
use super::descriptor::SEG_VALID_CACHE;
use super::instrumentation::Instrumentation;

/// The trace-scheduling bits of a processor's `async_event`, held while
/// something else runs.
///
/// Opaque, and returned rather than described, because the only thing a caller
/// may do with it is give it back: these bits are work the machine still owes
/// itself, and a caller that could read or forge them could quietly drop a
/// pending scheduler boundary — which is how a chipset's PAM flip would go
/// missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceBookkeeping(u32);

/// A processor's pending external interrupt, held while a trapped instruction
/// finishes.
///
/// Opaque for the same reason as [`TraceBookkeeping`]: the only thing a caller
/// may do with it is give it back. A caller that could forge one would deliver
/// a vector the controller never raised; a caller that dropped one would leave
/// a guest waiting forever on a line that is still asserted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeliverableInterrupt(u32);

impl DeliverableInterrupt {
    /// The external-interrupt bits, and only those. NOT the whole deliverable
    /// set: an `INIT`, an `SMI` or a shutdown is not something to postpone for
    /// the convenience of finishing an instruction, and a fault raised BY that
    /// instruction must still be taken by it.
    ///
    /// All three of them, because all three are what
    /// [`BxCpuC::handle_async_event`] will deliver as an external interrupt.
    /// Holding back the 8259's bit alone left the other two deliverable in the
    /// middle of a trapped instruction — the very thing this type exists to
    /// prevent — and which one a machine uses is a property of the guest: a
    /// guest routing its timer through the 8259 was safe, one routing it
    /// through an I/O APIC to the local APIC was not.
    const MASK: u32 = BxCpuC::<()>::BX_EVENT_PENDING_INTR
        | BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR
        | BxCpuC::<()>::BX_EVENT_PENDING_VMX_VIRTUAL_INTR;
}

impl TraceBookkeeping {
    /// The bits that are the trace scheduler's rather than the guest's.
    ///
    /// `BX_ASYNC_EVENT_STOP_TRACE` ends the current trace — set by a taken
    /// branch, by self-modifying code, and by a fast string burst reaching a
    /// device deadline. `BX_ASYNC_EVENT_SCHEDULER_BOUNDARY` says a device
    /// latched work for the machine. Neither is an event the guest may take.
    const MASK: u32 =
        BxCpuC::<()>::BX_ASYNC_EVENT_STOP_TRACE | super::cpu::BX_ASYNC_EVENT_SCHEDULER_BOUNDARY;
}

/// The number of vector registers carried, matching the processor's own file.
///
/// The whole file travels, not just the XMM halves. A trapped instruction may
/// read any of it, and a partial copy would leave the shadow processor holding
/// stale upper lanes that no one could see going wrong.
pub const VECTOR_REGISTERS: usize = super::decoder::BX_XMM_REGISTERS;

/// The packed descriptor attribute word.
///
/// One `u16` in the layout a GDT descriptor's upper bytes use, which is also
/// the layout `WHV_X64_SEGMENT_REGISTER.Attributes` uses and the one KVM's
/// `kvm_segment` unpacks. Keeping it packed is what lets a backend hand its
/// register straight across: unpacking into eight fields here and repacking
/// there would be two chances to disagree about bit 13.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct SegmentAttributes(u16);

impl SegmentAttributes {
    const TYPE: u16 = 0x000F;
    const NON_SYSTEM: u16 = 1 << 4;
    const DPL: u16 = 0x0060;
    const DPL_SHIFT: u32 = 5;
    const PRESENT: u16 = 1 << 7;
    const AVAILABLE: u16 = 1 << 12;
    const LONG: u16 = 1 << 13;
    const DEFAULT_BIG: u16 = 1 << 14;
    const GRANULAR: u16 = 1 << 15;

    /// Build from the packed word, as a backend's register carries it.
    #[must_use]
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// The packed word.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// The four-bit segment type: for a code or data segment, the
    /// executable / expand-down / writable / accessed encoding; for a system
    /// segment, which kind it is.
    #[must_use]
    pub const fn kind(self) -> u8 {
        (self.0 & Self::TYPE) as u8
    }

    /// The descriptor's S bit: set for a code or data segment, clear for a
    /// system one (an LDT or a task-state segment).
    #[must_use]
    pub const fn is_code_or_data(self) -> bool {
        self.0 & Self::NON_SYSTEM != 0
    }

    /// Descriptor privilege level, 0 through 3.
    #[must_use]
    pub const fn dpl(self) -> u8 {
        ((self.0 & Self::DPL) >> Self::DPL_SHIFT) as u8
    }

    #[must_use]
    pub const fn is_present(self) -> bool {
        self.0 & Self::PRESENT != 0
    }

    #[must_use]
    pub const fn is_available(self) -> bool {
        self.0 & Self::AVAILABLE != 0
    }

    /// The L bit: a 64-bit code segment. Mutually exclusive with
    /// [`Self::is_default_big`], which the architecture requires and
    /// [`VcpuArchState`]'s import checks.
    #[must_use]
    pub const fn is_long(self) -> bool {
        self.0 & Self::LONG != 0
    }

    /// The D/B bit: 32-bit default operand size for code, and for SS the
    /// difference between `ESP` and `SP`.
    #[must_use]
    pub const fn is_default_big(self) -> bool {
        self.0 & Self::DEFAULT_BIG != 0
    }

    /// The G bit: the limit field counts 4 KiB pages rather than bytes.
    #[must_use]
    pub const fn is_granular(self) -> bool {
        self.0 & Self::GRANULAR != 0
    }

    /// Assemble the word from its parts.
    #[must_use]
    pub const fn new(parts: SegmentAttributeParts) -> Self {
        let mut bits = (parts.kind as u16) & Self::TYPE;
        bits |= ((parts.dpl as u16) << Self::DPL_SHIFT) & Self::DPL;
        if parts.code_or_data {
            bits |= Self::NON_SYSTEM;
        }
        if parts.present {
            bits |= Self::PRESENT;
        }
        if parts.available {
            bits |= Self::AVAILABLE;
        }
        if parts.long {
            bits |= Self::LONG;
        }
        if parts.default_big {
            bits |= Self::DEFAULT_BIG;
        }
        if parts.granular {
            bits |= Self::GRANULAR;
        }
        Self(bits)
    }
}

/// The parts of a [`SegmentAttributes`] word, for assembling one.
///
/// A struct rather than eight positional arguments: six of them are booleans,
/// so nothing but position would tell `long` from `granular` (R0).
#[derive(Clone, Copy, Default)]
pub struct SegmentAttributeParts {
    pub kind: u8,
    pub dpl: u8,
    pub code_or_data: bool,
    pub present: bool,
    pub available: bool,
    pub long: bool,
    pub default_big: bool,
    pub granular: bool,
}

impl core::fmt::Debug for SegmentAttributes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegmentAttributes")
            .field("bits", &format_args!("{:#06x}", self.0))
            .field("kind", &self.kind())
            .field("dpl", &self.dpl())
            .field("present", &self.is_present())
            .field("long", &self.is_long())
            .field("default_big", &self.is_default_big())
            .field("granular", &self.is_granular())
            .finish()
    }
}

/// One segment register, selector and cached descriptor together.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct SegmentState {
    pub selector: u16,
    pub base: u64,
    /// The descriptor's twenty-bit limit FIELD, unscaled.
    ///
    /// Granularity lives in `attributes`, and the two are always read
    /// together. Storing the scaled byte limit here as well would be two
    /// answers to one question, which is the shape that lets a value and the
    /// bit describing it drift apart.
    pub limit: u32,
    pub attributes: SegmentAttributes,
}

impl SegmentState {
    /// The last offset the segment addresses, with granularity applied.
    ///
    /// Bochs descriptor.h keeps this as `limit_scaled`; it is what the
    /// processor actually compares an access against.
    #[must_use]
    pub const fn scaled_limit(self) -> u32 {
        if self.attributes.is_granular() {
            (self.limit << 12) | 0xFFF
        } else {
            self.limit
        }
    }
}

/// A descriptor table register: GDTR or IDTR.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct DescriptorTableState {
    pub base: u64,
    pub limit: u16,
}

/// The model-specific registers a hypervisor carries across an exit.
///
/// Named individually rather than as an index/value list, because these are
/// the ones every backend has a register slot for; the open-ended MSR file
/// stays on the processor, where a trapped `RDMSR` reads it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct MsrState {
    pub efer: u64,
    pub apic_base: u64,
    pub star: u64,
    pub lstar: u64,
    pub cstar: u64,
    pub sfmask: u64,
    pub kernel_gs_base: u64,
    pub sysenter_cs: u64,
    pub sysenter_esp: u64,
    pub sysenter_eip: u64,
    pub pat: u64,
    pub tsc: u64,
}

/// The x87 register file and its control words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FpuState {
    pub control_word: u16,
    pub status_word: u16,
    pub tag_word: u16,
    pub opcode: u16,
    pub instruction_pointer: u64,
    pub data_pointer: u64,
    pub instruction_selector: u16,
    pub data_selector: u16,
    /// The eight physical stack slots, each an 80-bit extended double as
    /// `(significand, sign-and-exponent)`.
    pub stack: [(u64, u16); 8],
}

impl Default for FpuState {
    fn default() -> Self {
        Self {
            control_word: 0,
            status_word: 0,
            tag_word: 0,
            opcode: 0,
            instruction_pointer: 0,
            data_pointer: 0,
            instruction_selector: 0,
            data_selector: 0,
            stack: [(0, 0); 8],
        }
    }
}

/// Everything one processor holds that the architecture defines.
///
/// `#[non_exhaustive]` because a backend fills it field by field and the set
/// will grow — nested-virtualization state is the known gap — and a caller
/// that constructed it literally would break on every addition. Build one with
/// [`VcpuArchState::default`] and assign.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub struct VcpuArchState {
    /// RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, then R8 through R15.
    pub gprs: [u64; 16],
    pub rip: u64,
    pub rflags: u64,

    /// ES, CS, SS, DS, FS, GS — in the processor's own index order, which is
    /// what [`BxSegregs`] numbers.
    pub segments: [SegmentState; 6],
    pub ldtr: SegmentState,
    pub tr: SegmentState,
    pub gdtr: DescriptorTableState,
    pub idtr: DescriptorTableState,

    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    /// The task-priority register, as `MOV CR8` sees it: the top four bits of
    /// the local APIC's TPR.
    pub cr8: u64,

    pub dr: [u64; 4],
    pub dr6: u64,
    pub dr7: u64,

    pub msrs: MsrState,

    pub fpu: FpuState,
    /// The vector register file, whole. See [`VECTOR_REGISTERS`].
    pub vector: [[u8; 64]; VECTOR_REGISTERS],
    pub opmask: [u64; 8],
    pub mxcsr: u32,
    pub xcr0: u32,
}

impl Default for VcpuArchState {
    fn default() -> Self {
        Self {
            gprs: [0; 16],
            rip: 0,
            rflags: 0,
            segments: [SegmentState::default(); 6],
            ldtr: SegmentState::default(),
            tr: SegmentState::default(),
            gdtr: DescriptorTableState::default(),
            idtr: DescriptorTableState::default(),
            cr0: 0,
            cr2: 0,
            cr3: 0,
            cr4: 0,
            cr8: 0,
            dr: [0; 4],
            dr6: 0,
            dr7: 0,
            msrs: MsrState::default(),
            fpu: FpuState::default(),
            vector: [[0; 64]; VECTOR_REGISTERS],
            opmask: [0; 8],
            mxcsr: 0,
            xcr0: 0,
        }
    }
}

/// Why a [`VcpuArchState`] could not be loaded into a processor.
///
/// `#[non_exhaustive]` because it crosses a crate boundary and will gain
/// variants as more of the state is validated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ArchStateError {
    /// A segment claims to be both 64-bit and 32-bit. The architecture gives
    /// L and D/B meaning only one at a time, and a processor loaded with both
    /// would fetch at a width nothing agrees on.
    SegmentIsLongAndBig { index: usize },
    /// A segment's DPL does not fit the two bits it has. Only reachable from a
    /// hand-built state, but the field is a `u8`.
    SegmentDplOutOfRange { index: usize, dpl: u8 },
    /// A granular segment's limit field does not fit twenty bits, so the
    /// scaled limit it describes is not expressible.
    SegmentLimitOutOfRange { index: usize, limit: u32 },
}

impl core::fmt::Display for ArchStateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SegmentIsLongAndBig { index } => {
                write!(f, "segment {index} sets both the L and D/B bits")
            }
            Self::SegmentDplOutOfRange { index, dpl } => {
                write!(f, "segment {index} has DPL {dpl}, which exceeds 3")
            }
            Self::SegmentLimitOutOfRange { index, limit } => {
                write!(
                    f,
                    "segment {index} is granular with limit field {limit:#x}, \
                     which exceeds twenty bits"
                )
            }
        }
    }
}

/// The order [`VcpuArchState::segments`] uses, which is the processor's.
const SEGMENT_ORDER: [BxSegregs; 6] = [
    BxSegregs::Es,
    BxSegregs::Cs,
    BxSegregs::Ss,
    BxSegregs::Ds,
    BxSegregs::Fs,
    BxSegregs::Gs,
];

impl<T: Instrumentation> BxCpuC<T> {
    /// Record that this processor halted while an engine was running it.
    ///
    /// The engine's counterpart to the `HLT` handler, and the reason it is
    /// needed: a processor run on the host's own hardware executes `HLT`
    /// itself, so the checks Bochs makes before halting — CPL, an SVM or VMX
    /// intercept — are the hardware's, and what reaches an engine is a
    /// processor that has already halted. Halting is not architectural state
    /// either, so it does not travel in a [`VcpuArchState`]; the platform
    /// reports it as the reason a run ended, and this is where that reason
    /// becomes something the machine can read.
    ///
    /// It goes through the same `enter_sleep_state` the interpreter's `HLT`
    /// ends in, so a processor halted by hardware and one halted by the
    /// interpreter are indistinguishable to the scheduler that stops running
    /// it and to the wake-up path that decides an interrupt may start it
    /// again.
    pub fn record_halt(&mut self) {
        self.enter_sleep_state(CpuActivityState::Hlt);
    }

    /// Drop every decoded trace this processor holds.
    ///
    /// For an engine whose guest also executes outside this interpreter. The
    /// interpreter's self-modifying-code tracking sees a write only when it
    /// makes one, so a store the hardware makes to the shared guest memory
    /// bumps no page stamp — and a trace decoded before the hardware ran may
    /// describe bytes that are no longer there. A kernel that patches its own
    /// text is enough: the transient `INT3` a live patch site holds must not
    /// outlive the patch inside a cached trace. Called at the hand-back,
    /// before this processor executes again. (Bochs icache.h
    /// `flushICacheEntries`, the same conservative flush the SMC
    /// pending-queue overflow takes.)
    pub fn discard_decoded_traces(&mut self) {
        self.i_cache.flush_all();
        self.invalidate_prefetch_q();
    }

    /// Lift the interpreter's trace bookkeeping out of the way, and hand it
    /// back to be restored.
    ///
    /// Bochs keeps two different things in one word. Some bits of
    /// `async_event` say an EVENT is deliverable; the rest are the trace
    /// scheduler talking to itself — stop chaining traces because a jump was
    /// taken, because code was written, because a device latched some work for
    /// the machine's next boundary. A repeated string instruction consults the
    /// whole word between items and stops for any of it, which is right for an
    /// interpreter that will service the bookkeeping in a moment.
    ///
    /// It is wrong for an engine servicing a trap. There, the bookkeeping
    /// cannot be acted on until the whole slice ends, so stopping for it
    /// achieves nothing and costs everything: a `REP INSW` reading one disk
    /// sector stops after each of its 256 words, and each restart is another
    /// exit and another exchange of the processor's entire architectural
    /// state. What may still stop the instruction is what the architecture
    /// says may — a deliverable interrupt — and that is a different bit.
    ///
    /// Returns what was taken so the caller can put it back. The bits are not
    /// discarded: the machine still has that work to do, and still does it,
    /// one slice boundary later.
    #[must_use]
    pub fn park_trace_bookkeeping(&mut self) -> TraceBookkeeping {
        let parked = self.async_event & TraceBookkeeping::MASK;
        self.async_event &= !TraceBookkeeping::MASK;
        TraceBookkeeping(parked)
    }

    /// Put back what [`Self::park_trace_bookkeeping`] took, alongside anything
    /// that arrived meanwhile.
    pub fn resume_trace_bookkeeping(&mut self, parked: TraceBookkeeping) {
        self.async_event |= parked.0;
    }

    /// Hold back an external interrupt while a trapped instruction finishes.
    ///
    /// The interpreter's loop asks `handle_async_event` at the head of EVERY
    /// iteration, and a strict one-instruction budget does not change that —
    /// so a processor asked to finish one trapped instruction will instead
    /// deliver a pending interrupt, push a frame, and jump to a handler.
    ///
    /// For an engine that runs the guest on hardware, that is delivery in a
    /// place the engine does not know it happened: its contract is that a
    /// vector reaches the guest at the head of a slice and nowhere else, so
    /// that the frame, the handler and the `IRET` all belong to the same
    /// story. A delivery nested inside the servicing of a disk's `REP INSW`
    /// leaves the guest somewhere its own driver never agreed to be, and the
    /// stack does not come back — which is a `general protection` on the
    /// `IRET` that ends the handler, some hundred bytes below where its frame
    /// was pushed.
    ///
    /// Held, not dropped: the line is still asserted, the machine still owes
    /// the guest the vector, and [`Self::resume_deliverable_interrupt`] hands
    /// it back for the next slice head to deliver properly.
    /// Masked rather than cleared, deliberately. The interpreter's own loop
    /// drains the bus at instruction boundaries and re-signals the pending bit
    /// the moment a device raises its line, so a bit merely taken away comes
    /// straight back and is delivered anyway. Masking is what actually holds.
    #[must_use]
    pub fn park_deliverable_interrupt(&mut self) -> DeliverableInterrupt {
        let was = self.event_mask & DeliverableInterrupt::MASK;
        self.event_mask |= DeliverableInterrupt::MASK;
        DeliverableInterrupt(was)
    }

    /// Restore the mask to what it was, so a processor whose guest had
    /// interrupts disabled anyway stays that way.
    pub fn resume_deliverable_interrupt(&mut self, parked: DeliverableInterrupt) {
        self.event_mask &= !DeliverableInterrupt::MASK;
        self.event_mask |= parked.0;
    }

    /// Whether this processor has an event waiting that it may take now.
    ///
    /// The same question the scheduler asks to decide whether a processor is
    /// runnable — an unmasked pending event, or a local-APIC interrupt with
    /// interrupts enabled — and deliberately NOT the whole of Bochs's
    /// `async_event` word, which also carries bookkeeping that is not a
    /// delivery: a trace that must not be chained, a scheduler boundary to
    /// service. A processor fresh from reset has the first of those set, so an
    /// engine keying off the raw word would interpret an instruction at the
    /// head of every stretch of guest execution.
    ///
    /// An engine running the guest on the host's own processor has to ask,
    /// because the hardware cannot answer: this machine's 8259 pair and local
    /// APIC are its own, and a partition created with no APIC of its own knows
    /// nothing of either. When this says yes, the engine runs one instruction
    /// on the shadow first, so delivery is the interpreter's — which is what
    /// makes an interrupt taken under a hypervisor identical, frame for frame,
    /// to the same interrupt taken without one.
    #[must_use]
    pub fn has_an_event_to_deliver(&self) -> bool {
        self.is_unmasked_event_pending(u32::MAX)
            || (self.lapic.intr && self.interrupts_enabled())
    }

    /// Whether this processor is inside system-management mode.
    ///
    /// An engine running the guest on the host's own processor has to ask,
    /// because SMM is not a mode it can hand over: no hypervisor offers one,
    /// the state a processor saves on entry lives in SMRAM in a layout the
    /// hardware would not produce, and `RSM` outside SMM is an invalid opcode.
    /// A handler is therefore run to completion on the shadow, where SMM is
    /// this port's own and behaves exactly as Bochs does — which is what keeps
    /// a chipset's SMI a real one rather than something the engine had to
    /// pretend away.
    #[must_use]
    pub fn is_in_smm(&self) -> bool {
        self.smm_mode()
    }

    /// Read this processor's architectural state out.
    ///
    /// Pure: nothing about the processor changes, so an engine may export as
    /// often as it likes to compare states.
    pub fn export_arch_state(&self, out: &mut VcpuArchState) {
        for (slot, reg) in out.gprs.iter_mut().zip(self.gen_reg.iter()) {
            *slot = reg.rrx();
        }
        out.rip = self.rip();
        out.rflags = u64::from(self.eflags_materialized());

        for (slot, seg) in SEGMENT_ORDER.iter().enumerate() {
            out.segments[slot] = segment_out(&self.sregs[*seg as usize]);
        }
        out.ldtr = segment_out(&self.ldtr);
        out.tr = segment_out(&self.tr);
        out.gdtr = DescriptorTableState {
            base: self.gdtr.base,
            limit: self.gdtr.limit,
        };
        out.idtr = DescriptorTableState {
            base: self.idtr.base,
            limit: self.idtr.limit,
        };

        out.cr0 = u64::from(self.cr0.bits());
        out.cr2 = self.cr2;
        out.cr3 = self.cr3;
        out.cr4 = self.cr4.bits();
        out.cr8 = u64::from(self.lapic.get_tpr() >> 4);

        // Only DR0 through DR3 are address registers. The processor's array
        // carries a fifth slot; DR4 and DR5 alias DR6 and DR7 on the
        // architecture and are not separate state to hand over.
        for (slot, reg) in out.dr.iter_mut().zip(self.dr.iter()) {
            *slot = *reg;
        }
        out.dr6 = u64::from(self.dr6.bits());
        out.dr7 = u64::from(self.dr7.bits());

        out.msrs = MsrState {
            efer: u64::from(self.efer.bits()),
            apic_base: self.msr.apicbase,
            star: self.msr.star,
            lstar: self.msr.lstar,
            cstar: self.msr.cstar,
            sfmask: u64::from(self.msr.fmask),
            kernel_gs_base: self.msr.kernelgsbase,
            sysenter_cs: u64::from(self.msr.sysenter_cs_msr),
            sysenter_esp: self.msr.sysenter_esp_msr,
            sysenter_eip: self.msr.sysenter_eip_msr,
            pat: self.msr.pat.U64(),
            tsc: self.cpu_local_ticks(),
        };

        out.fpu = FpuState {
            control_word: self.the_i387.cwd,
            status_word: self.the_i387.swd,
            tag_word: self.the_i387.twd,
            opcode: self.the_i387.foo,
            instruction_pointer: self.the_i387.fip,
            data_pointer: self.the_i387.fdp,
            instruction_selector: self.the_i387.fcs,
            data_selector: self.the_i387.fds,
            stack: core::array::from_fn(|i| {
                let reg = self.the_i387.st_space[i];
                (reg.signif, reg.sign_exp)
            }),
        };
        for (slot, reg) in out.vector.iter_mut().zip(self.vmm.iter()) {
            *slot = *reg.raw();
        }
        for (slot, mask) in out.opmask.iter_mut().zip(self.opmask.iter()) {
            *slot = mask.rrx();
        }
        out.mxcsr = self.mxcsr.mxcsr;
        out.xcr0 = self.xcr0.value;
    }

    /// Load an architectural state into this processor.
    ///
    /// Every write goes through the path that maintains what the processor
    /// derives from it — the segment loads re-derive the fetch-mode mask and
    /// drop the prefetch and stack windows, the control registers re-derive
    /// the CPU mode and flush the TLBs, and the flags write re-evaluates
    /// interrupt masking. Assigning the fields and returning is how a
    /// processor ends up describing one state while behaving as another.
    ///
    /// Validation happens before anything is written, so a rejected state
    /// leaves the processor as it was rather than half-loaded.
    pub fn import_arch_state(
        &mut self,
        state: &VcpuArchState,
    ) -> Result<(), ArchStateError> {
        for (index, seg) in state
            .segments
            .iter()
            .chain([&state.ldtr, &state.tr])
            .enumerate()
        {
            let attributes = seg.attributes;
            if attributes.is_long() && attributes.is_default_big() {
                return Err(ArchStateError::SegmentIsLongAndBig { index });
            }
            if attributes.dpl() > 3 {
                return Err(ArchStateError::SegmentDplOutOfRange {
                    index,
                    dpl: attributes.dpl(),
                });
            }
            if attributes.is_granular() && seg.limit > 0x000F_FFFF {
                return Err(ArchStateError::SegmentLimitOutOfRange {
                    index,
                    limit: seg.limit,
                });
            }
        }

        for (slot, reg) in state.gprs.iter().zip(self.gen_reg.iter_mut()) {
            reg.set_rrx(*slot);
        }

        // Control registers before segments and RIP: the CPU mode a segment
        // load derives its fetch-mode mask from is decided by CR0 and EFER.
        self.cr0 = super::crregs::BxCr0::from_bits_retain(state.cr0 as u32);
        self.cr2 = state.cr2;
        self.cr3 = state.cr3;
        self.cr4 = super::crregs::BxCr4::from_bits_retain(state.cr4);
        self.efer = super::crregs::BxEfer::from_bits_retain(state.msrs.efer as u32);
        self.linaddr_width = if self.cr4.la57() { 57 } else { 48 };
        self.handle_cpu_mode_change();

        for (slot, seg) in SEGMENT_ORDER.iter().enumerate() {
            self.load_segment(*seg, &state.segments[slot]);
        }
        segment_in(&mut self.ldtr, &state.ldtr);
        segment_in(&mut self.tr, &state.tr);
        self.gdtr.base = state.gdtr.base;
        self.gdtr.limit = state.gdtr.limit;
        self.idtr.base = state.idtr.base;
        self.idtr.limit = state.idtr.limit;

        self.set_rip(state.rip);
        // An imported processor stands at an instruction boundary, so the
        // instruction it is about to execute begins where `RIP` points. That is
        // what `prev_rip` means, and it is not architectural state — no
        // hypervisor carries it, so it survives an import holding whatever the
        // last guest this processor emulated left behind.
        //
        // It is load-bearing rather than cosmetic. A repeated string
        // instruction broken by an asynchronous event rewinds with
        // `set_rip(prev_rip)` so the remaining items re-execute (Bochs cpu.cc
        // `repeat`), and `exception` restarts a faulting instruction the same
        // way. Left stale, both resume the guest at an address belonging to
        // some earlier trap — a `REP INSW` servicing a disk sector jumps into
        // an unrelated interrupt stub mid-transfer, and the guest is lost with
        // no fault to show for it.
        self.prev_rip = state.rip;
        // Through the API path, which calls `handle_interrupt_mask_change`:
        // IF gates the deliverable events, and a flags write that skips that
        // leaves the processor unable to take an interrupt it says it can.
        self.set_rflags_for_api(state.rflags);

        for (reg, slot) in self.dr.iter_mut().zip(state.dr.iter()) {
            *reg = *slot;
        }
        self.dr6 = super::crregs::BxDr6::from_bits_retain(state.dr6 as u32);
        self.dr7 = super::crregs::BxDr7::from_bits_retain(state.dr7 as u32);

        self.msr.apicbase = state.msrs.apic_base;
        self.msr.star = state.msrs.star;
        self.msr.lstar = state.msrs.lstar;
        self.msr.cstar = state.msrs.cstar;
        self.msr.fmask = state.msrs.sfmask as u32;
        self.msr.kernelgsbase = state.msrs.kernel_gs_base;
        self.msr.sysenter_cs_msr = state.msrs.sysenter_cs as u32;
        self.msr.sysenter_esp_msr = state.msrs.sysenter_esp;
        self.msr.sysenter_eip_msr = state.msrs.sysenter_eip;
        self.msr.pat.set_U64(state.msrs.pat);
        // A TPR change can make a pending LAPIC interrupt deliverable, which
        // is why this goes through the APIC rather than at its register.
        self.lapic.set_tpr(((state.cr8 & 0xF) as u8) << 4);

        self.the_i387.cwd = state.fpu.control_word;
        self.the_i387.swd = state.fpu.status_word;
        self.the_i387.twd = state.fpu.tag_word;
        self.the_i387.foo = state.fpu.opcode;
        self.the_i387.fip = state.fpu.instruction_pointer;
        self.the_i387.fdp = state.fpu.data_pointer;
        self.the_i387.fcs = state.fpu.instruction_selector;
        self.the_i387.fds = state.fpu.data_selector;
        for (slot, (signif, sign_exp)) in
            self.the_i387.st_space.iter_mut().zip(state.fpu.stack)
        {
            slot.signif = signif;
            slot.sign_exp = sign_exp;
        }
        for (reg, slot) in self.vmm.iter_mut().zip(state.vector.iter()) {
            *reg.raw_mut() = *slot;
        }
        for (mask, slot) in self.opmask.iter_mut().zip(state.opmask.iter()) {
            mask.set_rrx(*slot);
        }
        self.mxcsr.mxcsr = state.mxcsr;
        self.xcr0.value = state.xcr0;

        // Anything cached from the state just replaced describes a processor
        // that no longer exists.
        self.handle_alignment_check();
        self.update_fetch_mode_mask();
        self.invalidate_prefetch_q();
        self.invalidate_stack_cache();
        self.tlb_flush();
        Ok(())
    }
}

/// Read one segment register out of the processor.
fn segment_out(seg: &super::descriptor::BxSegmentReg) -> SegmentState {
    let scaled = seg.cache.u.segment_limit_scaled();
    let granular = seg.cache.u.segment_g();
    SegmentState {
        selector: seg.selector.value,
        base: seg.cache.u.segment_base(),
        // The inverse of the scaling the cache applied: a granular limit was
        // stored with its low twelve bits set, so shifting them back off
        // recovers the field a descriptor held.
        limit: if granular { scaled >> 12 } else { scaled },
        attributes: SegmentAttributes::new(SegmentAttributeParts {
            kind: seg.cache.r#type,
            dpl: seg.cache.dpl,
            code_or_data: seg.cache.segment,
            present: seg.cache.p,
            available: seg.cache.u.segment_avl(),
            long: seg.cache.u.segment_l(),
            default_big: seg.cache.u.segment_d_b(),
            granular,
        }),
    }
}

/// Write one system segment's cache: LDTR or TR.
///
/// These are the two the processor derives nothing from — no fetch-mode mask,
/// no prefetch window, no stack window — so they do not go through
/// `BxCpuC::load_segment`, which exists to carry exactly those effects.
fn segment_in(seg: &mut super::descriptor::BxSegmentReg, state: &SegmentState) {
    super::segment_ctrl_pro::parse_selector(state.selector, &mut seg.selector);
    // As in `BxCpuC::load_segment`: a descriptor that is not present describes
    // nothing, and a cache that claims otherwise is believed downstream.
    seg.cache.valid = if state.attributes.is_present() {
        SEG_VALID_CACHE
    } else {
        0
    };
    seg.cache.p = state.attributes.is_present();
    seg.cache.dpl = state.attributes.dpl();
    seg.cache.segment = state.attributes.is_code_or_data();
    seg.cache.r#type = state.attributes.kind();
    seg.cache.u.set_segment_base(state.base);
    seg.cache.u.set_segment_limit_scaled(state.scaled_limit());
    seg.cache.u.set_segment_g(state.attributes.is_granular());
    seg.cache.u.set_segment_d_b(state.attributes.is_default_big());
    seg.cache.u.set_segment_l(state.attributes.is_long());
    seg.cache.u.set_segment_avl(state.attributes.is_available());
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::ResetReason;

    /// A packed attribute word survives being taken apart and put back
    /// together — including the two bits that are one apart and mean opposite
    /// things.
    #[test]
    fn an_attribute_word_round_trips_through_its_parts() {
        // A 64-bit code segment: present, DPL 0, type 0xB, L set, D/B clear.
        let long_code = SegmentAttributes::from_bits(0xA09B);
        assert_eq!(long_code.kind(), 0xB);
        assert!(long_code.is_code_or_data());
        assert!(long_code.is_present());
        assert!(long_code.is_long());
        assert!(
            !long_code.is_default_big(),
            "a 64-bit code segment must not also claim 32-bit default size"
        );
        assert!(long_code.is_granular());

        let rebuilt = SegmentAttributes::new(SegmentAttributeParts {
            kind: long_code.kind(),
            dpl: long_code.dpl(),
            code_or_data: long_code.is_code_or_data(),
            present: long_code.is_present(),
            available: long_code.is_available(),
            long: long_code.is_long(),
            default_big: long_code.is_default_big(),
            granular: long_code.is_granular(),
        });
        assert_eq!(rebuilt, long_code);

        // A ring-3 32-bit data segment, to pin DPL and D/B independently.
        let user_data = SegmentAttributes::new(SegmentAttributeParts {
            kind: 0x3,
            dpl: 3,
            code_or_data: true,
            present: true,
            default_big: true,
            granular: true,
            ..SegmentAttributeParts::default()
        });
        assert_eq!(user_data.dpl(), 3);
        assert!(user_data.is_default_big());
        assert!(!user_data.is_long());
        assert_eq!(user_data.bits(), 0xC0F3);
    }

    /// Granularity scales the limit, and the scaling is invertible: what the
    /// processor compares against comes back as the field a descriptor held.
    #[test]
    fn a_granular_limit_scales_and_unscales() {
        let byte_granular = SegmentState {
            limit: 0xFFFF,
            attributes: SegmentAttributes::default(),
            ..SegmentState::default()
        };
        assert_eq!(byte_granular.scaled_limit(), 0xFFFF);

        let page_granular = SegmentState {
            limit: 0xFFFFF,
            attributes: SegmentAttributes::new(SegmentAttributeParts {
                granular: true,
                ..SegmentAttributeParts::default()
            }),
            ..SegmentState::default()
        };
        assert_eq!(
            page_granular.scaled_limit(),
            0xFFFF_FFFF,
            "a full 20-bit page-granular limit addresses the whole 4 GiB"
        );
    }

    /// A processor with an event waiting says so; one with nothing waiting
    /// says no, and says no straight out of reset.
    ///
    /// The question an engine running the guest on the host's own processor
    /// asks before handing the hardware anything: delivery belongs to this
    /// port, so a stretch that begins with an event waiting begins on the
    /// interpreter instead. Both directions matter, and the negative one is
    /// the trap — a processor fresh from reset has Bochs's `async_event` word
    /// non-zero (its prefetch queue was invalidated, which asks for the trace
    /// not to be chained), so an engine reading that word would interpret an
    /// instruction at the head of every stretch and quietly run most of the
    /// guest in software.
    #[test]
    fn a_processor_reports_an_event_to_deliver_only_when_it_has_one() {
        let mut cpu = BxCpuBuilder::new().build().expect("a processor");
        cpu.reset(ResetReason::Hardware);
        assert!(
            !cpu.has_an_event_to_deliver(),
            "a processor just reset has nothing to deliver"
        );

        // An NMI, because it is not gated on IF and a processor at reset has
        // interrupts disabled — this asks about the predicate, not about the
        // flag.
        cpu.signal_event(BxCpuC::<()>::BX_EVENT_NMI);
        assert!(
            cpu.has_an_event_to_deliver(),
            "an unmasked pending event is exactly what an engine must let the \
             interpreter deliver"
        );
    }

    /// Recording a halt leaves the processor a scheduler will stop running,
    /// and does it the way the `HLT` instruction does.
    ///
    /// An engine's only way to report that the hardware halted the guest —
    /// halting is not architectural state, so it cannot travel in a
    /// [`VcpuArchState`].
    #[test]
    fn a_recorded_halt_leaves_the_processor_asleep() {
        let mut cpu = BxCpuBuilder::new().build().expect("a processor");
        cpu.reset(ResetReason::Hardware);
        assert_eq!(cpu.activity_state, crate::cpu::cpu::CpuActivityState::Active);

        cpu.record_halt();
        assert_eq!(
            cpu.activity_state,
            crate::cpu::cpu::CpuActivityState::Hlt,
            "a machine reads the activity state to decide whether to keep \
             scheduling this processor"
        );
        assert!(
            !cpu.has_an_event_to_deliver(),
            "a halt is not itself an event to deliver: what wakes a halted \
             processor is something arriving afterwards"
        );
    }

    /// Exporting a processor and importing the result leaves a processor that
    /// exports the same thing — so nothing is dropped on the way through, and
    /// nothing is invented.
    #[test]
    fn a_state_round_trips_through_a_processor() {
        let mut cpu = BxCpuBuilder::new().build().unwrap();
        cpu.reset(ResetReason::Hardware);

        let mut original = VcpuArchState::default();
        cpu.export_arch_state(&mut original);

        // Reset leaves CS at the ROM aperture; a state that says so is
        // exactly what a hypervisor would hand over for a processor at its
        // power-on vector, so this is the interesting one to carry.
        assert_eq!(original.segments[BxSegregs::Cs as usize].selector, 0xF000);

        let mut fresh = BxCpuBuilder::new().build().unwrap();
        fresh.reset(ResetReason::Hardware);
        fresh.import_arch_state(&original).unwrap();

        let mut returned = VcpuArchState::default();
        fresh.export_arch_state(&mut returned);
        assert_eq!(returned, original);
    }

    /// Importing a state that describes a 16-bit stack gives the processor a
    /// 16-bit stack — asserted by pushing, not by reading the bit back.
    ///
    /// This is the property the whole import exists for, and the reason it
    /// goes through `load_segment` rather than assigning the cache. The day
    /// this file was written, a segment written without its derived state
    /// produced a real-mode machine whose pushes addressed `ESP`, ran off the
    /// 64 KiB limit, and took #SS on every interrupt. Reading the D/B bit back
    /// would have gone green through all of it.
    #[test]
    fn importing_a_16_bit_stack_puts_a_push_where_a_16_bit_stack_would() {
        let mut machine = crate::cpu::exec_ctx::TestMachine::new();
        let mut ctx = machine.ctx();
        ctx.reset(ResetReason::Hardware);

        let mut state = VcpuArchState::default();
        ctx.export_arch_state(&mut state);
        state.segments[BxSegregs::Ss as usize] = SegmentState {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attributes: SegmentAttributes::new(SegmentAttributeParts {
                kind: 0x3,
                code_or_data: true,
                present: true,
                ..SegmentAttributeParts::default()
            }),
        };
        // A stack pointer of zero is the interesting one: the first push wraps
        // to the top of the segment, which a 32-bit stack would put four
        // billion bytes away instead.
        state.gprs[4] = 0;
        ctx.import_arch_state(&state).unwrap();

        ctx.push_16(0x1234).expect("a 16-bit stack has room at its top");
        assert_eq!(
            ctx.rsp(),
            0xFFFE,
            "the push must land at the top of the 64 KiB segment"
        );
    }

    /// The same import with the D/B bit set addresses `ESP` instead, which on
    /// a 64 KiB segment means the push does not fit at all.
    #[test]
    fn importing_a_32_bit_stack_addresses_esp_and_faults_past_the_limit() {
        let mut machine = crate::cpu::exec_ctx::TestMachine::new();
        let mut ctx = machine.ctx();
        ctx.reset(ResetReason::Hardware);

        let mut state = VcpuArchState::default();
        ctx.export_arch_state(&mut state);
        state.segments[BxSegregs::Ss as usize] = SegmentState {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attributes: SegmentAttributes::new(SegmentAttributeParts {
                kind: 0x3,
                code_or_data: true,
                present: true,
                default_big: true,
                ..SegmentAttributeParts::default()
            }),
        };
        state.gprs[4] = 0;
        ctx.import_arch_state(&state).unwrap();

        assert!(
            ctx.push_16(0x1234).is_err(),
            "ESP wrapping to 0xFFFFFFFE is past a 0xFFFF limit, so this must \
             fault rather than write — which is what makes the previous test's \
             success mean something"
        );
    }

    /// Importing CS re-derives the fetch-mode mask, so the processor decodes
    /// at the width the imported segment declares.
    #[test]
    fn importing_cs_re_derives_the_decode_width() {
        let mut cpu = BxCpuBuilder::new().build().unwrap();
        cpu.reset(ResetReason::Hardware);

        let mut state = VcpuArchState::default();
        cpu.export_arch_state(&mut state);
        let sixteen_bit = cpu.fetch_mode_mask;

        state.segments[BxSegregs::Cs as usize].attributes =
            SegmentAttributes::new(SegmentAttributeParts {
                kind: 0xB,
                code_or_data: true,
                present: true,
                default_big: true,
                granular: true,
                ..SegmentAttributeParts::default()
            });
        state.segments[BxSegregs::Cs as usize].limit = 0xFFFFF;
        cpu.import_arch_state(&state).unwrap();

        assert_ne!(
            cpu.fetch_mode_mask, sixteen_bit,
            "a CS that changed width must change how the next instruction is \
             decoded; an import that only stored the bit would leave this equal"
        );
    }

    /// A state the architecture has no processor for is refused, and refused
    /// before anything is written — so a rejected import leaves the processor
    /// exactly as it was rather than half-loaded.
    #[test]
    fn an_impossible_segment_is_refused_without_touching_the_processor() {
        let mut cpu = BxCpuBuilder::new().build().unwrap();
        cpu.reset(ResetReason::Hardware);

        let mut before = VcpuArchState::default();
        cpu.export_arch_state(&mut before);

        let mut state = before.clone();
        state.gprs[0] = 0xDEAD_BEEF;
        state.segments[BxSegregs::Cs as usize].attributes =
            SegmentAttributes::new(SegmentAttributeParts {
                kind: 0xB,
                code_or_data: true,
                present: true,
                long: true,
                default_big: true,
                ..SegmentAttributeParts::default()
            });

        let refusal = cpu.import_arch_state(&state).unwrap_err();
        assert_eq!(
            refusal,
            ArchStateError::SegmentIsLongAndBig {
                index: BxSegregs::Cs as usize
            }
        );

        let mut after = VcpuArchState::default();
        cpu.export_arch_state(&mut after);
        assert_eq!(
            after, before,
            "a refused import must write nothing, including the register it \
             would have got to before reaching the bad segment"
        );
    }
}
