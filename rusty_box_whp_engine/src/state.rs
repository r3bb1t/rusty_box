//! Moving a processor's architectural state between this port and the platform.
//!
//! Both sides already describe a processor the same way, which is why this is a
//! transcription and not a translation. [`SegmentAttributes`] is the packed
//! `u16` a GDT descriptor uses, and `WHV_X64_SEGMENT_REGISTER.Attributes` is
//! the same word with the same bits in the same places — so the attribute
//! travels whole rather than being unpacked here and repacked there, which
//! would be two chances to disagree about, say, bit 13.
//!
//! What does NOT cross here is the x87 and vector file: the platform names no
//! register for most of it, so it crosses as the architecture's own XSAVE
//! area instead — [`crate::xsave`], refreshed and written back beside this
//! exchange. This module's export leaves those fields of the state exactly as
//! they were, which is what lets that area fill them afterwards.

use rusty_box_whp::{Reg, RegisterValue, SegmentRegister, TableRegister, Vcpu, WhpResult};

use rusty_box::cpu::arch_state::{
    ArchGroups, DescriptorTableState, SegmentAttributes, SegmentState, VcpuArchState,
};

/// The word-shaped registers, in the order [`WORD_VALUES`] reads and writes.
///
/// One list, used for both directions, so an import and an export cannot
/// disagree about which slot holds what.
const WORD_REGS: &[Reg] = &[
    Reg::Rax,
    Reg::Rcx,
    Reg::Rdx,
    Reg::Rbx,
    Reg::Rsp,
    Reg::Rbp,
    Reg::Rsi,
    Reg::Rdi,
    Reg::R8,
    Reg::R9,
    Reg::R10,
    Reg::R11,
    Reg::R12,
    Reg::R13,
    Reg::R14,
    Reg::R15,
    Reg::Rip,
    Reg::Rflags,
    Reg::Cr0,
    Reg::Cr2,
    Reg::Cr3,
    Reg::Cr4,
    Reg::Cr8,
    Reg::Dr0,
    Reg::Dr1,
    Reg::Dr2,
    Reg::Dr3,
    Reg::Dr6,
    Reg::Dr7,
];

/// How many words the list above carries.
const WORD_VALUES: usize = WORD_REGS.len();

/// The model-specific registers, kept in a second batch because the platform
/// takes at most thirty-two names per call.
const MSR_REGS: &[Reg] = &[
    Reg::Efer,
    Reg::ApicBase,
    Reg::Star,
    Reg::Lstar,
    Reg::Cstar,
    Reg::Sfmask,
    Reg::KernelGsBase,
    Reg::SysenterCs,
    Reg::SysenterEsp,
    Reg::SysenterEip,
    Reg::Pat,
    Reg::Xcr0,
    // LAST, and deliberately: everything before this is written back to the
    // processor, and the time-stamp counter is not. See [`IMPORTED_MSRS`].
    Reg::Tsc,
];

const MSR_VALUES: usize = MSR_REGS.len();

/// How many of [`MSR_REGS`] are written back into the processor.
///
/// All but the last, which is the time-stamp counter — read so the state is
/// complete, never written.
///
/// The hardware owns the TSC on this engine. `TRAPPED_MSRS` deliberately
/// leaves both TSC entries with the platform, because this port derives its
/// TSC from retired instructions and a shadow processor retires almost none
/// while the guest runs on hardware. Writing that shadow value back therefore
/// STOMPS the guest's time-stamp counter to near zero on every single slice,
/// while the hardware has been advancing it at gigahertz — a counter that
/// jumps backwards thousands of times a second, which is the one thing every
/// timing loop in a guest assumes cannot happen.
///
/// Measured before the fix: the state exchange round-tripped every field
/// exactly EXCEPT `tsc`, which came back `0` from the shadow against the
/// hardware's real value, on 278,069 of 278,069 slices.
///
/// It is [`TSC_SLOT`] that carries the rule now — the whole-state write skips
/// that one register by index rather than by truncating a list.
const IMPORTED_MSRS: usize = MSR_VALUES - 1;

const _: () = assert!(
    IMPORTED_MSRS == TSC_SLOT,
    "the time-stamp counter is the register the import leaves out, and it is last"
);

/// The six segment registers plus the two system ones, in `VcpuArchState`'s own
/// order so the first six index its `segments` array directly.
const SEGMENT_REGS: &[Reg] = &[
    Reg::Es,
    Reg::Cs,
    Reg::Ss,
    Reg::Ds,
    Reg::Fs,
    Reg::Gs,
    Reg::Ldtr,
    Reg::Tr,
];

/// Where `ldtr` and `tr` sit in [`SEGMENT_REGS`], after the six the guest names.
const LDTR_SLOT: usize = 6;
const TR_SLOT: usize = 7;

const TABLE_REGS: &[Reg] = &[Reg::Gdtr, Reg::Idtr];
const GDTR_SLOT: usize = 0;
const IDTR_SLOT: usize = 1;

/// What a state exchange talks to.
///
/// A trait so the exchange can be tested against a processor that records what
/// it was told rather than one that needs a hypervisor. The processor a
/// running engine exchanges with is [`Vcpu`], whose verbs these are; the
/// recorder in this module's tests is the other implementation.
///
/// Everything a slice moves in or out of a processor goes through here, the
/// vector file included — one seam for the hazard rather than two (R5), so a
/// caller holding a processor needs no second way to reach it.
pub(crate) trait VpRegisters {
    fn read_words(&self, regs: &[Reg], out: &mut [u64]) -> WhpResult<()>;
    fn write_words(&self, regs: &[Reg], words: &[u64]) -> WhpResult<()>;
    /// Registers of mixed shape in one call — what a whole-state exchange uses,
    /// and the only shape it needs. The per-shape calls the platform also
    /// offers are not here because nothing above this seam wants them: a
    /// processor is exchanged whole or not at all.
    fn read_registers(&self, regs: &[Reg], out: &mut [RegisterValue]) -> WhpResult<()>;
    fn write_registers(&self, regs: &[Reg], values: &[RegisterValue]) -> WhpResult<()>;
    /// The x87 and vector file as the architecture's own XSAVE area, which is
    /// the only shape the platform offers it in. Answers how many bytes were
    /// written.
    fn read_xsave(&self, out: &mut [u8]) -> WhpResult<usize>;
    fn write_xsave(&self, area: &[u8]) -> WhpResult<()>;
}

/// The processor a running engine exchanges with.
///
/// Each body names the inherent verb of the same job on [`Vcpu`] — spelled as
/// a path rather than a method call, so that which of the two is meant is
/// visible at every line.
impl VpRegisters for Vcpu {
    fn read_words(&self, regs: &[Reg], out: &mut [u64]) -> WhpResult<()> {
        Vcpu::read_regs(self, regs, out)
    }

    fn write_words(&self, regs: &[Reg], words: &[u64]) -> WhpResult<()> {
        Vcpu::write_regs(self, regs, words)
    }

    fn read_registers(&self, regs: &[Reg], out: &mut [RegisterValue]) -> WhpResult<()> {
        Vcpu::read_registers(self, regs, out)
    }

    fn write_registers(&self, regs: &[Reg], values: &[RegisterValue]) -> WhpResult<()> {
        Vcpu::write_registers(self, regs, values)
    }

    fn read_xsave(&self, out: &mut [u8]) -> WhpResult<usize> {
        Vcpu::read_xsave(self, out)
    }

    fn write_xsave(&self, area: &[u8]) -> WhpResult<()> {
        Vcpu::write_xsave(self, area)
    }
}

/// Every register a whole-state exchange moves, in the order the exchange reads
/// and writes them.
///
/// One list, so one platform call carries the lot. The layout is words, then
/// model specific registers, then segments, then descriptor tables, and
/// [`WORD_VALUES`], [`MSR_VALUES`] and the segment count are the offsets into
/// it — a register added to any group shifts the ones after it and both sides
/// follow, because both index the same constants.
fn whole_state_regs() -> [Reg; WHOLE_STATE] {
    let mut regs = [Reg::Rax; WHOLE_STATE];
    let (words, rest) = regs.split_at_mut(WORD_VALUES);
    words.copy_from_slice(WORD_REGS);
    let (msrs, rest) = rest.split_at_mut(MSR_VALUES);
    msrs.copy_from_slice(MSR_REGS);
    let (segments, tables) = rest.split_at_mut(SEGMENT_VALUES);
    segments.copy_from_slice(SEGMENT_REGS);
    tables.copy_from_slice(TABLE_REGS);
    regs
}

/// How many segment registers the exchange carries.
const SEGMENT_VALUES: usize = 8;

/// Where the time-stamp counter sits within [`MSR_REGS`] — last, which is what
/// [`IMPORTED_MSRS`] rests on.
const TSC_SLOT: usize = MSR_VALUES - 1;

/// How many registers a whole-state exchange names.
const WHOLE_STATE: usize = WORD_VALUES + MSR_VALUES + SEGMENT_VALUES + 2;

/// Where the segments begin in [`whole_state_regs`].
const SEGMENTS_AT: usize = WORD_VALUES + MSR_VALUES;

/// Where the descriptor tables begin in [`whole_state_regs`].
const TABLES_AT: usize = SEGMENTS_AT + SEGMENT_VALUES;

/// Where each word-shaped register sits in [`WORD_REGS`], and so in
/// [`whole_state_regs`].
///
/// Named rather than written as literals at each use, and each one asserted
/// against the list below: a register inserted into [`WORD_REGS`] shifts every
/// position after it, and the assertions are what turn that into a build
/// failure rather than a processor whose `CR3` arrives in `CR2`.
const RIP_AT: usize = 16;
const RFLAGS_AT: usize = 17;
const CR0_AT: usize = 18;
const CR2_AT: usize = 19;
const CR3_AT: usize = 20;
const CR4_AT: usize = 21;
const CR8_AT: usize = 22;
const DR0_AT: usize = 23;
const DR6_AT: usize = 27;
const DR7_AT: usize = 28;

/// Where each model-specific register sits in [`MSR_REGS`], asserted against
/// that list for the reason the word positions are.
const EFER_SLOT: usize = 0;
const APIC_BASE_SLOT: usize = 1;
const STAR_SLOT: usize = 2;
const LSTAR_SLOT: usize = 3;
const CSTAR_SLOT: usize = 4;
const SFMASK_SLOT: usize = 5;
const KERNEL_GS_BASE_SLOT: usize = 6;
const SYSENTER_CS_SLOT: usize = 7;
const SYSENTER_ESP_SLOT: usize = 8;
const SYSENTER_EIP_SLOT: usize = 9;
const PAT_SLOT: usize = 10;
const XCR0_SLOT: usize = 11;

const _: () = {
    assert!(MSR_REGS[EFER_SLOT] as u32 == Reg::Efer as u32);
    assert!(MSR_REGS[APIC_BASE_SLOT] as u32 == Reg::ApicBase as u32);
    assert!(MSR_REGS[STAR_SLOT] as u32 == Reg::Star as u32);
    assert!(MSR_REGS[LSTAR_SLOT] as u32 == Reg::Lstar as u32);
    assert!(MSR_REGS[CSTAR_SLOT] as u32 == Reg::Cstar as u32);
    assert!(MSR_REGS[SFMASK_SLOT] as u32 == Reg::Sfmask as u32);
    assert!(MSR_REGS[KERNEL_GS_BASE_SLOT] as u32 == Reg::KernelGsBase as u32);
    assert!(MSR_REGS[SYSENTER_CS_SLOT] as u32 == Reg::SysenterCs as u32);
    assert!(MSR_REGS[SYSENTER_ESP_SLOT] as u32 == Reg::SysenterEsp as u32);
    assert!(MSR_REGS[SYSENTER_EIP_SLOT] as u32 == Reg::SysenterEip as u32);
    assert!(MSR_REGS[PAT_SLOT] as u32 == Reg::Pat as u32);
    assert!(MSR_REGS[XCR0_SLOT] as u32 == Reg::Xcr0 as u32);
    assert!(MSR_REGS[TSC_SLOT] as u32 == Reg::Tsc as u32);
};

const _: () = {
    assert!(WORD_REGS[RIP_AT] as u32 == Reg::Rip as u32);
    assert!(WORD_REGS[RFLAGS_AT] as u32 == Reg::Rflags as u32);
    assert!(WORD_REGS[CR0_AT] as u32 == Reg::Cr0 as u32);
    assert!(WORD_REGS[CR2_AT] as u32 == Reg::Cr2 as u32);
    assert!(WORD_REGS[CR3_AT] as u32 == Reg::Cr3 as u32);
    assert!(WORD_REGS[CR4_AT] as u32 == Reg::Cr4 as u32);
    assert!(WORD_REGS[CR8_AT] as u32 == Reg::Cr8 as u32);
    assert!(WORD_REGS[DR0_AT] as u32 == Reg::Dr0 as u32);
    assert!(WORD_REGS[DR6_AT] as u32 == Reg::Dr6 as u32);
    assert!(WORD_REGS[DR7_AT] as u32 == Reg::Dr7 as u32);
    assert!(DR7_AT + 1 == WORD_VALUES, "the debug registers end the word list");
};

/// The groups this module carries.
///
/// Not the vector file, which crosses as an XSAVE area ([`crate::xsave`]), and
/// not the interrupt state, which is no part of a [`VcpuArchState`] — a
/// backend exchanges that one register itself.
pub(crate) const NAMED_GROUPS: ArchGroups = ArchGroups::GPRS
    .union(ArchGroups::RIP_RFLAGS)
    .union(ArchGroups::CONTROL_REGS)
    .union(ArchGroups::DEBUG_REGS)
    .union(ArchGroups::SEGMENTS)
    .union(ArchGroups::TABLES)
    .union(ArchGroups::MSRS);

/// Which direction a listing of registers is for (R2).
///
/// The two lists differ in exactly one register: the time-stamp counter is
/// read so a state is complete, and never written — see [`IMPORTED_MSRS`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Transfer {
    Read,
    Write,
}

/// Which group owns position `index` of [`whole_state_regs`].
///
/// The one place the register list is divided into groups (R5): both
/// directions ask here, so a group cannot mean one set of registers on the way
/// in and another on the way out.
const fn group_at(index: usize) -> ArchGroups {
    if index < RIP_AT {
        ArchGroups::GPRS
    } else if index < CR0_AT {
        ArchGroups::RIP_RFLAGS
    } else if index < DR0_AT {
        ArchGroups::CONTROL_REGS
    } else if index < WORD_VALUES {
        ArchGroups::DEBUG_REGS
    } else if index < SEGMENTS_AT {
        ArchGroups::MSRS
    } else if index < TABLES_AT {
        ArchGroups::SEGMENTS
    } else {
        ArchGroups::TABLES
    }
}

/// The positions of [`whole_state_regs`] that `groups` names, in that list's
/// own order. Answers how many were written into `out`.
fn positions_of(groups: ArchGroups, transfer: Transfer, out: &mut [usize; WHOLE_STATE]) -> usize {
    let mut at = 0;
    for index in 0..WHOLE_STATE {
        if !groups.contains(group_at(index)) {
            continue;
        }
        // The one register the hardware owns. Writing the shadow's copy back
        // would drag the guest's clock backwards on every slice.
        if transfer == Transfer::Write && index == WORD_VALUES + TSC_SLOT {
            continue;
        }
        out[at] = index;
        at += 1;
    }
    at
}

/// Read position `index` of [`whole_state_regs`] out of a state.
fn value_at(state: &VcpuArchState, index: usize) -> RegisterValue {
    if index < WORD_VALUES {
        RegisterValue::Word(match index {
            RIP_AT => state.rip,
            RFLAGS_AT => state.rflags,
            CR0_AT => state.cr0,
            CR2_AT => state.cr2,
            CR3_AT => state.cr3,
            CR4_AT => state.cr4,
            CR8_AT => state.cr8,
            DR6_AT => state.dr6,
            DR7_AT => state.dr7,
            _ if index < RIP_AT => state.gprs[index],
            _ => state.dr[index - DR0_AT],
        })
    } else if index < SEGMENTS_AT {
        RegisterValue::Word(match index - WORD_VALUES {
            EFER_SLOT => state.msrs.efer,
            APIC_BASE_SLOT => state.msrs.apic_base,
            STAR_SLOT => state.msrs.star,
            LSTAR_SLOT => state.msrs.lstar,
            CSTAR_SLOT => state.msrs.cstar,
            SFMASK_SLOT => state.msrs.sfmask,
            KERNEL_GS_BASE_SLOT => state.msrs.kernel_gs_base,
            SYSENTER_CS_SLOT => state.msrs.sysenter_cs,
            SYSENTER_ESP_SLOT => state.msrs.sysenter_esp,
            SYSENTER_EIP_SLOT => state.msrs.sysenter_eip,
            PAT_SLOT => state.msrs.pat,
            XCR0_SLOT => u64::from(state.xcr0),
            _ => state.msrs.tsc,
        })
    } else if index < TABLES_AT {
        RegisterValue::Segment(to_platform_segment(match index - SEGMENTS_AT {
            LDTR_SLOT => state.ldtr,
            TR_SLOT => state.tr,
            slot => state.segments[slot],
        }))
    } else {
        // The table list holds two, so the last arm is unreachable — and
        // naming both is what keeps it that way if a third is ever added.
        let table = match index - TABLES_AT {
            GDTR_SLOT => state.gdtr,
            IDTR_SLOT => state.idtr,
            _ => DescriptorTableState::default(),
        };
        RegisterValue::Table(TableRegister { base: table.base, limit: table.limit })
    }
}

/// Write position `index` of [`whole_state_regs`] into a state.
///
/// The exact inverse of [`value_at`], position for position — the two are
/// adjacent so that a register moved in one is moved in the other, and
/// `a_processor_survives_being_installed_and_read_back` fails on the value if
/// it is not.
fn place_at(state: &mut VcpuArchState, index: usize, value: RegisterValue) {
    if index < WORD_VALUES {
        let word = word_of(value);
        match index {
            RIP_AT => state.rip = word,
            RFLAGS_AT => state.rflags = word,
            CR0_AT => state.cr0 = word,
            CR2_AT => state.cr2 = word,
            CR3_AT => state.cr3 = word,
            CR4_AT => state.cr4 = word,
            CR8_AT => state.cr8 = word,
            DR6_AT => state.dr6 = word,
            DR7_AT => state.dr7 = word,
            _ if index < RIP_AT => state.gprs[index] = word,
            _ => state.dr[index - DR0_AT] = word,
        }
    } else if index < SEGMENTS_AT {
        let word = word_of(value);
        match index - WORD_VALUES {
            EFER_SLOT => state.msrs.efer = word,
            APIC_BASE_SLOT => state.msrs.apic_base = word,
            STAR_SLOT => state.msrs.star = word,
            LSTAR_SLOT => state.msrs.lstar = word,
            CSTAR_SLOT => state.msrs.cstar = word,
            SFMASK_SLOT => state.msrs.sfmask = word,
            KERNEL_GS_BASE_SLOT => state.msrs.kernel_gs_base = word,
            SYSENTER_CS_SLOT => state.msrs.sysenter_cs = word,
            SYSENTER_ESP_SLOT => state.msrs.sysenter_esp = word,
            SYSENTER_EIP_SLOT => state.msrs.sysenter_eip = word,
            PAT_SLOT => state.msrs.pat = word,
            // XCR0 is architecturally 64 bits and every bit this port models
            // lives in the low half, which is why the state holds it as a
            // `u32`. Truncating is the same narrowing `xcr0.get32()` performs
            // on the processor.
            XCR0_SLOT => state.xcr0 = word as u32,
            _ => state.msrs.tsc = word,
        }
    } else if index < TABLES_AT {
        let seg = match value {
            RegisterValue::Segment(seg) => seg,
            _ => SegmentRegister::default(),
        };
        let seg = from_platform_segment(seg);
        match index - SEGMENTS_AT {
            LDTR_SLOT => state.ldtr = seg,
            TR_SLOT => state.tr = seg,
            slot => state.segments[slot] = seg,
        }
    } else {
        let table = match value {
            RegisterValue::Table(table) => table,
            _ => TableRegister::default(),
        };
        let table = DescriptorTableState { base: table.base, limit: table.limit };
        // Two entries, as [`value_at`] says.
        match index - TABLES_AT {
            GDTR_SLOT => state.gdtr = table,
            IDTR_SLOT => state.idtr = table,
            _ => {}
        }
    }
}

/// Read the registers `groups` names into `state`, leaving every other field
/// as it was. Answers how many platform calls it made — one, or none when
/// `groups` names no register.
///
/// # Errors
/// Whatever the platform said about a register it would not give up.
pub(crate) fn read_groups(
    vp: &impl VpRegisters,
    groups: ArchGroups,
    state: &mut VcpuArchState,
) -> WhpResult<usize> {
    let mut positions = [0usize; WHOLE_STATE];
    let named = positions_of(groups, Transfer::Read, &mut positions);
    if named == 0 {
        return Ok(0);
    }
    let all = whole_state_regs();
    let mut names = [Reg::Rax; WHOLE_STATE];
    for (name, index) in names.iter_mut().zip(&positions[..named]) {
        *name = all[*index];
    }
    let mut values = [RegisterValue::Word(0); WHOLE_STATE];
    vp.read_registers(&names[..named], &mut values[..named])?;
    for (index, value) in positions[..named].iter().zip(&values[..named]) {
        place_at(state, *index, *value);
    }
    Ok(1)
}

/// Write the registers `groups` names out of `state`, touching no other
/// register of the processor. Answers how many platform calls it made.
///
/// # Errors
/// Whatever the platform said about a register it would not take.
pub(crate) fn write_groups(
    vp: &impl VpRegisters,
    groups: ArchGroups,
    state: &VcpuArchState,
) -> WhpResult<usize> {
    let mut positions = [0usize; WHOLE_STATE];
    let named = positions_of(groups, Transfer::Write, &mut positions);
    if named == 0 {
        return Ok(0);
    }
    let all = whole_state_regs();
    let mut names = [Reg::Rax; WHOLE_STATE];
    let mut values = [RegisterValue::Word(0); WHOLE_STATE];
    for ((name, value), index) in
        names.iter_mut().zip(values.iter_mut()).zip(&positions[..named])
    {
        *name = all[*index];
        *value = value_at(state, *index);
    }
    vp.write_registers(&names[..named], &values[..named])?;
    Ok(1)
}

/// Take a word from a value the platform returned, or zero if it is not one.
///
/// A mismatch means [`whole_state_regs`] and the offsets above have drifted
/// apart, which is a defect in this file rather than anything the platform did
/// — so it reads as zero and the guest-visible tests catch it, instead of a
/// panic in a library.
const fn word_of(value: RegisterValue) -> u64 {
    match value {
        RegisterValue::Word(word) => word,
        _ => 0,
    }
}

/// The two sides count a segment's limit differently, and this is where that
/// is reconciled.
///
/// The platform's limit is the EFFECTIVE one — the descriptor's field with
/// granularity already applied, which is the convention the VMCS uses and what
/// this host demonstrably hands back: a guest that loads a flat 4 GiB segment
/// produces `0xFFFF_FFFF` here. This port keeps the descriptor's own twenty-bit
/// field and applies granularity when it needs the effective value, which is
/// what [`SegmentState::scaled_limit`] is for.
///
/// The conversion is exact in both directions for any legal descriptor: a
/// granular segment's effective limit always has its low twelve bits set, so
/// shifting them away loses nothing that was ever chosen.
const fn to_platform_segment(state: SegmentState) -> SegmentRegister {
    SegmentRegister {
        base: state.base,
        limit: state.scaled_limit(),
        selector: state.selector,
        attributes: state.attributes.bits(),
    }
}

pub(crate) const fn from_platform_segment(seg: SegmentRegister) -> SegmentState {
    let attributes = SegmentAttributes::from_bits(seg.attributes);
    SegmentState {
        selector: seg.selector,
        base: seg.base,
        limit: if attributes.is_granular() {
            seg.limit >> 12
        } else {
            seg.limit
        },
        attributes,
    }
}

/// Write `state` into the processor.
///
/// # Errors
/// Whatever the platform said about a register it would not take.
pub(crate) fn import(vp: &impl VpRegisters, state: &VcpuArchState) -> WhpResult<()> {
    // The whole processor in one call — minus the time-stamp counter, which
    // [`Transfer::Write`] leaves out because the hardware owns it. The count
    // `write_groups` answers with is for a caller whose group set is decided
    // at run time; `NAMED_GROUPS` is never empty, so here it is always one.
    write_groups(vp, NAMED_GROUPS, state)?;
    Ok(())
}

/// Read the processor into `state`, leaving the parts this seam does not carry
/// as they were.
///
/// # Errors
/// Whatever the platform said about a register it would not give up.
pub(crate) fn export(vp: &impl VpRegisters, state: &mut VcpuArchState) -> WhpResult<()> {
    // The whole processor in one call. The cost of a transfer is the call and
    // not the registers in it, and a booting machine makes one of these per
    // slice — a hundred thousand over a DLX boot — so the three calls this
    // replaces were three times the price for the same information. The count
    // is one for the reason [`import`] gives.
    read_groups(vp, NAMED_GROUPS, state)?;
    // A processor cannot be executing through the CS this read-back carries
    // unless the pieces agree: a long (L=1) CS is legal exactly when
    // `EFER.LMA` says long mode is active — its base and limit are then out
    // of play — and any other CS under CR0.PE must be present with a nonzero
    // effective limit. A state that breaks this will not fail here; it fails
    // later as an unattributable guest fault, because the import cannot
    // refuse it and the shadow then applies the wrong mode's rules — a limit
    // check against a long segment, an eight-byte gate walk through a
    // sixteen-byte IDT. Reported on the state the read-back just produced, the
    // one moment the disagreement is still visible. CS only: data segments
    // legitimately read back zeroed after a null selector load.
    {
        /// `EFER.LMA`, bit 10 — long mode ACTIVE, the bit the mode derivation
        /// turns on.
        const LMA: u64 = 1 << 10;
        /// Where CS, SS and DS sit in a state's own segment array.
        const CS: usize = 1;
        const SS: usize = 2;
        const DS: usize = 3;
        let cs = state.segments[CS];
        let attributes = cs.attributes;
        let executable = attributes.is_present()
            && if attributes.is_long() {
                state.msrs.efer & LMA != 0
            } else {
                cs.limit != 0 || attributes.is_granular()
            };
        if state.cr0 & 1 != 0 && !executable {
            tracing::error!(
                "read-back CS cannot be executing: cs sel={:#06x} base={:#x} limit={:#x} \
                 attr={:#06x}; ss sel={:#06x} limit={:#x} attr={:#06x} ds attr={:#06x} \
                 rip={:#x} cr0={:#x} rflags={:#x} efer={:#x} cr4={:#x}",
                cs.selector,
                cs.base,
                cs.scaled_limit(),
                attributes.bits(),
                state.segments[SS].selector,
                state.segments[SS].scaled_limit(),
                state.segments[SS].attributes.bits(),
                state.segments[DS].attributes.bits(),
                state.rip,
                state.cr0,
                state.rflags,
                state.msrs.efer,
                state.cr4
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::test_vp::Recorder;
    use super::*;
    use rusty_box::cpu::arch_state::{MsrState, SegmentAttributeParts, VECTOR_REGISTERS};

    /// A distinctive state, so a field that lands in the wrong slot shows up as
    /// a wrong VALUE rather than as a zero that a default would also produce.
    fn distinctive() -> VcpuArchState {
        let mut state = VcpuArchState::default();
        for (index, gpr) in state.gprs.iter_mut().enumerate() {
            *gpr = 0x1000 + index as u64;
        }
        state.rip = 0xDEAD_BEEF;
        state.rflags = 0x0000_0202;
        state.cr0 = 0x8000_0011;
        state.cr2 = 0x2222;
        state.cr3 = 0x3333;
        state.cr4 = 0x4444;
        state.cr8 = 0x000F;
        for (index, dr) in state.dr.iter_mut().enumerate() {
            *dr = 0xD000 + index as u64;
        }
        state.dr6 = 0xD6;
        state.dr7 = 0xD7;
        state.msrs = MsrState {
            efer: 0xE0,
            apic_base: 0xE1,
            star: 0xE2,
            lstar: 0xE3,
            cstar: 0xE4,
            sfmask: 0xE5,
            kernel_gs_base: 0xE6,
            sysenter_cs: 0xE7,
            sysenter_esp: 0xE8,
            sysenter_eip: 0xE9,
            pat: 0xEA,
            tsc: 0xEB,
        };
        state.xcr0 = 0x0000_0007;
        for (index, seg) in state.segments.iter_mut().enumerate() {
            *seg = SegmentState {
                selector: 0x10 + index as u16,
                base: 0x10_0000 + index as u64,
                limit: 0xF_0000 + index as u32,
                attributes: SegmentAttributes::new(SegmentAttributeParts {
                    kind: 3,
                    dpl: 0,
                    code_or_data: true,
                    present: true,
                    available: false,
                    long: false,
                    default_big: true,
                    granular: true,
                }),
            };
        }
        state.ldtr = SegmentState { selector: 0x50, base: 0x5000, ..state.segments[0] };
        state.tr = SegmentState { selector: 0x60, base: 0x6000, ..state.segments[0] };
        state.gdtr = DescriptorTableState { base: 0x7000, limit: 0x7F };
        state.idtr = DescriptorTableState { base: 0x8000, limit: 0x8F };
        state
    }

    /// Everything this seam carries survives the round trip.
    ///
    /// The whole point of the exchange: a processor described here, installed
    /// on the platform and read back, describes the same processor. A field
    /// written to the wrong slot fails on the value, not on a zero.
    #[test]
    fn a_processor_survives_being_installed_and_read_back() {
        let vp = Recorder::default();
        let original = distinctive();
        import(&vp, &original).expect("a recorder refuses nothing");

        let mut read_back = VcpuArchState::default();
        export(&vp, &mut read_back).expect("a recorder refuses nothing");

        assert_eq!(read_back.gprs, original.gprs);
        assert_eq!(read_back.rip, original.rip);
        assert_eq!(read_back.rflags, original.rflags);
        assert_eq!(read_back.cr0, original.cr0);
        assert_eq!(read_back.cr2, original.cr2);
        assert_eq!(read_back.cr3, original.cr3);
        assert_eq!(read_back.cr4, original.cr4);
        assert_eq!(read_back.cr8, original.cr8);
        assert_eq!(read_back.dr, original.dr);
        assert_eq!(read_back.dr6, original.dr6);
        assert_eq!(read_back.dr7, original.dr7);
        // Every model-specific register EXCEPT the time-stamp counter, which
        // this seam deliberately does not write — see [`IMPORTED_MSRS`]. Named
        // field by field rather than compared whole, so that a future field
        // added to `MsrState` and forgotten in `msr_words` fails here.
        assert_eq!(
            MsrState { tsc: 0, ..read_back.msrs },
            MsrState { tsc: 0, ..original.msrs }
        );
        assert_eq!(read_back.xcr0, original.xcr0);
        assert_eq!(read_back.segments, original.segments);
        assert_eq!(read_back.ldtr, original.ldtr);
        assert_eq!(read_back.tr, original.tr);
        assert_eq!(read_back.gdtr, original.gdtr);
        assert_eq!(read_back.idtr, original.idtr);
    }

    /// The guest's time-stamp counter is never written by this seam.
    ///
    /// The hardware owns it (`TRAPPED_MSRS` leaves both TSC entries with the
    /// platform), and the shadow's copy is derived from retired instructions —
    /// of which a shadow retires almost none while the guest runs on hardware.
    /// Writing that copy back therefore drags the guest's clock to near zero on
    /// every single slice, while the processor has been advancing it at
    /// gigahertz. A counter that jumps backwards thousands of times a second is
    /// the one thing every timing loop in a guest assumes cannot happen.
    ///
    /// Asserted on the RECORDER rather than on a read-back, because the point
    /// is that no write reached the register at all.
    #[test]
    fn the_time_stamp_counter_is_never_written_to_the_processor() {
        let vp = Recorder::default();
        let mut state = distinctive();
        state.msrs.tsc = 0xDEAD_BEEF;
        import(&vp, &state).expect("a recorder refuses nothing");

        assert!(
            !vp.was_written(Reg::Tsc),
            "the seam wrote the shadow's time-stamp counter into the processor"
        );
        // The MSR beside it in the list did land, so this is not a batch that
        // silently failed to write anything.
        assert!(vp.was_written(Reg::Pat));
        assert_eq!(vp.value_of(Reg::Pat), state.msrs.pat);
    }

    /// A segment's attribute word crosses whole.
    ///
    /// The reason both sides store the packed `u16` rather than parts: bit 13
    /// is `L` and bit 14 is `D/B`, they mean opposite things about the same
    /// segment, and unpacking on one side and repacking on the other is two
    /// chances to swap them.
    #[test]
    fn a_segment_keeps_its_attribute_word_across_the_seam() {
        let long_code = SegmentState {
            selector: 0x08,
            base: 0,
            limit: 0xFFFFF,
            attributes: SegmentAttributes::new(SegmentAttributeParts {
                kind: 0xB,
                dpl: 0,
                code_or_data: true,
                present: true,
                available: false,
                long: true,
                default_big: false,
                granular: true,
            }),
        };
        let crossed = from_platform_segment(to_platform_segment(long_code));
        assert_eq!(crossed, long_code);
        assert!(crossed.attributes.is_long());
        assert!(!crossed.attributes.is_default_big());
    }

    /// The register file this exchange leaves to [`crate::xsave`] stays
    /// untouched, so an export cannot quietly zero the fields the area fills
    /// in afterwards.
    #[test]
    fn the_vector_file_is_not_carried_and_is_not_disturbed() {
        let vp = Recorder::default();
        import(&vp, &distinctive()).expect("a recorder refuses nothing");

        let mut read_back = VcpuArchState::default();
        read_back.vector[1][0] = 0xA5;
        read_back.opmask[2] = 0x1234;
        read_back.mxcsr = 0x1F80;
        export(&vp, &mut read_back).expect("a recorder refuses nothing");

        assert_eq!(read_back.vector[1][0], 0xA5, "the vector file is the shadow's");
        assert_eq!(read_back.opmask[2], 0x1234);
        assert_eq!(read_back.mxcsr, 0x1F80);
        assert_eq!(read_back.vector.len(), VECTOR_REGISTERS);
    }
}

/// A processor that remembers what it was told, so an exchange can be
/// exercised on a host with no hypervisor — which is every host the gates run
/// on.
///
/// Crate-visible rather than private to this module's tests: the seam it
/// stands in for is exchanged from [`crate::exchange`] as well, and one
/// recorder that both drive is one account of what a processor was asked.
#[cfg(test)]
pub(crate) mod test_vp {
    use super::{Reg, RegisterValue, SegmentRegister, TableRegister, VpRegisters, WhpResult};
    use rusty_box_whp::ALL_REGS;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct Recorder {
        words: RefCell<BTreeMap<usize, u64>>,
        segments: RefCell<BTreeMap<usize, SegmentRegister>>,
        tables: RefCell<BTreeMap<usize, TableRegister>>,
        /// The extended-state area, exactly as it was last written. Empty
        /// until then, which is what a processor whose area has never been
        /// written would report: nothing to read.
        xsave: RefCell<std::vec::Vec<u8>>,
        /// How many times each register was asked for and told, by the key
        /// [`key`] gives it. Counted per register rather than per call,
        /// because what an exit costs is what it moves and the batching is a
        /// separate question — the platform-call count the exchange itself
        /// answers with.
        reads: RefCell<BTreeMap<usize, usize>>,
        writes: RefCell<BTreeMap<usize, usize>>,
        xsave_writes: Cell<usize>,
    }

    /// A stable key per register, so a write and the read that follows it agree
    /// without the recorder knowing what any of them mean.
    pub(crate) fn key(reg: Reg) -> usize {
        ALL_REGS
            .iter()
            .position(|candidate| *candidate == reg)
            .expect("a known register")
    }

    /// The per-shape stores behind the recorder, and the account of what was
    /// asked of them. Inherent rather than part of [`VpRegisters`], because
    /// that trait carries only what an exchange asks of a real processor.
    impl Recorder {
        /// Put a value where the processor holds it, without counting a write:
        /// a test seeding a partition's live value is not the exchange writing
        /// one, and [`Self::writes_of`] must be able to tell them apart.
        pub(crate) fn seed(&self, reg: Reg, word: u64) {
            self.words.borrow_mut().insert(key(reg), word);
        }

        /// A segment the processor holds, seeded the way [`Self::seed`] seeds
        /// a register — which is what a test needs to stand a partition in a
        /// mode the shadow is not in.
        pub(crate) fn seed_segment(&self, reg: Reg, segment: SegmentRegister) {
            self.segments.borrow_mut().insert(key(reg), segment);
        }

        /// The area the processor holds, seeded the way [`Self::seed`] seeds a
        /// register — a real processor always has one.
        pub(crate) fn seed_xsave(&self, area: &[u8]) {
            let mut stored = self.xsave.borrow_mut();
            stored.clear();
            stored.extend_from_slice(area);
        }

        pub(crate) fn value_of(&self, reg: Reg) -> u64 {
            self.words.borrow().get(&key(reg)).copied().unwrap_or(0)
        }

        pub(crate) fn reads_of(&self, reg: Reg) -> usize {
            self.reads.borrow().get(&key(reg)).copied().unwrap_or(0)
        }

        pub(crate) fn writes_of(&self, reg: Reg) -> usize {
            self.writes.borrow().get(&key(reg)).copied().unwrap_or(0)
        }

        /// Every register read out of this processor, over every call.
        pub(crate) fn total_reads(&self) -> usize {
            self.reads.borrow().values().sum()
        }

        /// How many times the whole extended-state area crossed back.
        pub(crate) fn xsave_writes(&self) -> usize {
            self.xsave_writes.get()
        }

        /// Whether the processor was ever told this register's value.
        pub(crate) fn was_written(&self, reg: Reg) -> bool {
            self.words.borrow().contains_key(&key(reg))
        }

        fn count(tally: &RefCell<BTreeMap<usize, usize>>, reg: Reg) {
            *tally.borrow_mut().entry(key(reg)).or_insert(0) += 1;
        }

        fn read_segments(&self, regs: &[Reg], out: &mut [SegmentRegister]) -> WhpResult<()> {
            let segments = self.segments.borrow();
            for (slot, reg) in out.iter_mut().zip(regs) {
                *slot = segments.get(&key(*reg)).copied().unwrap_or_default();
            }
            Ok(())
        }

        fn write_segments(&self, regs: &[Reg], segments: &[SegmentRegister]) -> WhpResult<()> {
            let mut stored = self.segments.borrow_mut();
            for (reg, seg) in regs.iter().zip(segments) {
                stored.insert(key(*reg), *seg);
            }
            Ok(())
        }

        fn read_tables(&self, regs: &[Reg], out: &mut [TableRegister]) -> WhpResult<()> {
            let tables = self.tables.borrow();
            for (slot, reg) in out.iter_mut().zip(regs) {
                *slot = tables.get(&key(*reg)).copied().unwrap_or_default();
            }
            Ok(())
        }

        fn write_tables(&self, regs: &[Reg], tables: &[TableRegister]) -> WhpResult<()> {
            let mut stored = self.tables.borrow_mut();
            for (reg, table) in regs.iter().zip(tables) {
                stored.insert(key(*reg), *table);
            }
            Ok(())
        }
    }

    impl VpRegisters for Recorder {
        fn read_words(&self, regs: &[Reg], out: &mut [u64]) -> WhpResult<()> {
            let words = self.words.borrow();
            for (slot, reg) in out.iter_mut().zip(regs) {
                *slot = words.get(&key(*reg)).copied().unwrap_or(0);
                Self::count(&self.reads, *reg);
            }
            Ok(())
        }

        fn write_words(&self, regs: &[Reg], words: &[u64]) -> WhpResult<()> {
            let mut stored = self.words.borrow_mut();
            for (reg, word) in regs.iter().zip(words) {
                stored.insert(key(*reg), *word);
                Self::count(&self.writes, *reg);
            }
            Ok(())
        }

        /// Routed by the same shape rule the platform uses, so the recorder
        /// files a mixed batch exactly where the per-shape calls would.
        fn read_registers(&self, regs: &[Reg], out: &mut [RegisterValue]) -> WhpResult<()> {
            for (slot, reg) in out.iter_mut().zip(regs) {
                *slot = match rusty_box_whp::shape_of(*reg) {
                    RegisterValue::Word(_) => {
                        let mut word = [0u64];
                        self.read_words(&[*reg], &mut word)?;
                        RegisterValue::Word(word[0])
                    }
                    RegisterValue::Segment(_) => {
                        let mut seg = [SegmentRegister::default()];
                        self.read_segments(&[*reg], &mut seg)?;
                        Self::count(&self.reads, *reg);
                        RegisterValue::Segment(seg[0])
                    }
                    RegisterValue::Table(_) => {
                        let mut table = [TableRegister::default()];
                        self.read_tables(&[*reg], &mut table)?;
                        Self::count(&self.reads, *reg);
                        RegisterValue::Table(table[0])
                    }
                    // The whole-state exchange carries no 128-bit register, and
                    // the seam's `the_exchange_list_holds_no_register_a_word_
                    // transfer_would_truncate` is what keeps it that way. A
                    // recorder that invented a value here would let a caller
                    // reach one anyway and pass.
                    RegisterValue::Words128(_) => {
                        return Err(rusty_box_whp::WhpError::contract(
                            "the whole-state exchange holds no 128-bit register",
                        ))
                    }
                };
            }
            Ok(())
        }

        fn write_registers(&self, regs: &[Reg], values: &[RegisterValue]) -> WhpResult<()> {
            for (reg, value) in regs.iter().zip(values) {
                match *value {
                    RegisterValue::Word(word) => self.write_words(&[*reg], &[word])?,
                    RegisterValue::Segment(seg) => {
                        self.write_segments(&[*reg], &[seg])?;
                        Self::count(&self.writes, *reg);
                    }
                    RegisterValue::Table(table) => {
                        self.write_tables(&[*reg], &[table])?;
                        Self::count(&self.writes, *reg);
                    }
                    // Refused for the reason `read_registers` gives.
                    RegisterValue::Words128(_) => {
                        return Err(rusty_box_whp::WhpError::contract(
                            "the whole-state exchange holds no 128-bit register",
                        ))
                    }
                }
            }
            Ok(())
        }

        /// Hands back exactly what was written, truncated to what the caller
        /// offered — the platform's own rule, so an area read from a recorder
        /// and one read from a processor are read the same way.
        ///
        /// A recorder that has never been written holds no area, and refuses
        /// rather than answering with a zero-length one: a real processor
        /// always has an area, and the platform refuses an answer too short to
        /// carry its own header, which is what lets
        /// [`crate::xsave::XsaveArea`] index that header without doubting it.
        fn read_xsave(&self, out: &mut [u8]) -> WhpResult<usize> {
            let stored = self.xsave.borrow();
            if stored.is_empty() {
                return Err(rusty_box_whp::WhpError::contract(
                    "the recorder was never given an extended-state area",
                ));
            }
            let written = stored.len().min(out.len());
            out[..written].copy_from_slice(&stored[..written]);
            Ok(written)
        }

        fn write_xsave(&self, area: &[u8]) -> WhpResult<()> {
            self.seed_xsave(area);
            self.xsave_writes.set(self.xsave_writes.get() + 1);
            Ok(())
        }
    }
}
