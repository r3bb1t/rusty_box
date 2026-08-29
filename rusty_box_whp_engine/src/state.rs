//! Moving a processor's architectural state between this port and the platform.
//!
//! Both sides already describe a processor the same way, which is why this is a
//! transcription and not a translation. [`SegmentAttributes`] is the packed
//! `u16` a GDT descriptor uses, and `WHV_X64_SEGMENT_REGISTER.Attributes` is
//! the same word with the same bits in the same places — so the attribute
//! travels whole rather than being unpacked here and repacked there, which
//! would be two chances to disagree about, say, bit 13.
//!
//! What does NOT cross here is the x87 and vector file. The platform carries
//! them, but a guest reaches them only through instructions, and every
//! instruction the hypervisor cannot finish is finished by the shadow
//! processor — which already holds that state and never handed it away. Moving
//! it every exit would be several hundred bytes each way for a register file
//! neither side had touched.

use rusty_box_whp::{Reg, RegisterValue, SegmentRegister, TableRegister, WhpResult};
#[cfg(test)]
use rusty_box_whp::ALL_REGS;

use rusty_box::cpu::arch_state::{
    DescriptorTableState, MsrState, SegmentAttributes, SegmentState, VcpuArchState,
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
/// it was told rather than one that needs a hypervisor. Implemented for
/// `Partition`, whose methods these are.
pub(crate) trait VpRegisters {
    fn read_words(&self, regs: &[Reg], out: &mut [u64]) -> WhpResult<()>;
    fn write_words(&self, regs: &[Reg], words: &[u64]) -> WhpResult<()>;
    /// Registers of mixed shape in one call — what a whole-state exchange uses,
    /// and the only shape it needs. The per-shape calls the platform also
    /// offers are not here because nothing above this seam wants them: a
    /// processor is exchanged whole or not at all.
    fn read_registers(&self, regs: &[Reg], out: &mut [RegisterValue]) -> WhpResult<()>;
    fn write_registers(&self, regs: &[Reg], values: &[RegisterValue]) -> WhpResult<()>;
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

const fn from_platform_segment(seg: SegmentRegister) -> SegmentState {
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
    let mut words = [0u64; WORD_VALUES];
    words[..16].copy_from_slice(&state.gprs);
    words[16] = state.rip;
    words[17] = state.rflags;
    words[18] = state.cr0;
    words[19] = state.cr2;
    words[20] = state.cr3;
    words[21] = state.cr4;
    words[22] = state.cr8;
    words[23..27].copy_from_slice(&state.dr);
    words[27] = state.dr6;
    words[28] = state.dr7;
    // All but the time-stamp counter — the hardware owns that one, and writing
    // the shadow's copy back would drag the guest's clock backwards on every
    // slice. See [`IMPORTED_MSRS`].
    let msrs = msr_words(&state.msrs, state.xcr0);

    let mut segments = [SegmentRegister::default(); SEGMENT_VALUES];
    for (slot, seg) in segments.iter_mut().zip(&state.segments) {
        *slot = to_platform_segment(*seg);
    }
    segments[LDTR_SLOT] = to_platform_segment(state.ldtr);
    segments[TR_SLOT] = to_platform_segment(state.tr);

    let tables = [
        TableRegister { base: state.gdtr.base, limit: state.gdtr.limit },
        TableRegister { base: state.idtr.base, limit: state.idtr.limit },
    ];

    // The whole processor in one call, as `export` reads it — minus the
    // time-stamp counter, which is why the names are the whole-state list with
    // that one register left out rather than the list itself.
    let all = whole_state_regs();
    let mut names = [Reg::Rax; WHOLE_STATE - 1];
    let mut values = [RegisterValue::Word(0); WHOLE_STATE - 1];
    let mut at = 0;
    for (index, reg) in all.iter().enumerate() {
        // The one register the hardware owns. Writing the shadow's copy back
        // would drag the guest's clock backwards on every slice.
        if index == WORD_VALUES + TSC_SLOT {
            continue;
        }
        names[at] = *reg;
        values[at] = if index < WORD_VALUES {
            RegisterValue::Word(words[index])
        } else if index < SEGMENTS_AT {
            RegisterValue::Word(msrs[index - WORD_VALUES])
        } else if index < TABLES_AT {
            RegisterValue::Segment(segments[index - SEGMENTS_AT])
        } else {
            RegisterValue::Table(tables[index - TABLES_AT])
        };
        at += 1;
    }
    vp.write_registers(&names, &values)
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
    // replaces were three times the price for the same information.
    let names = whole_state_regs();
    let mut values = [RegisterValue::Word(0); WHOLE_STATE];
    vp.read_registers(&names, &mut values)?;
    let words: [u64; WORD_VALUES] =
        core::array::from_fn(|index| word_of(values[index]));
    let msrs: [u64; MSR_VALUES] =
        core::array::from_fn(|index| word_of(values[WORD_VALUES + index]));

    state.gprs.copy_from_slice(&words[..16]);
    state.rip = words[16];
    state.rflags = words[17];
    state.cr0 = words[18];
    state.cr2 = words[19];
    state.cr3 = words[20];
    state.cr4 = words[21];
    state.cr8 = words[22];
    state.dr.copy_from_slice(&words[23..27]);
    state.dr6 = words[27];
    state.dr7 = words[28];

    state.msrs = MsrState {
        efer: msrs[0],
        apic_base: msrs[1],
        star: msrs[2],
        lstar: msrs[3],
        cstar: msrs[4],
        sfmask: msrs[5],
        kernel_gs_base: msrs[6],
        sysenter_cs: msrs[7],
        sysenter_esp: msrs[8],
        sysenter_eip: msrs[9],
        pat: msrs[10],
        tsc: msrs[12],
    };
    // XCR0 is architecturally 64 bits and every bit this port models lives in
    // the low half, which is why the state holds it as a `u32`. Truncating is
    // the same narrowing `xcr0.get32()` performs on the processor.
    state.xcr0 = msrs[11] as u32;

    let segments: [SegmentRegister; SEGMENT_VALUES] = core::array::from_fn(|index| {
        match values[SEGMENTS_AT + index] {
            RegisterValue::Segment(seg) => seg,
            _ => SegmentRegister::default(),
        }
    });
    for (slot, seg) in state.segments.iter_mut().zip(&segments) {
        *slot = from_platform_segment(*seg);
    }
    state.ldtr = from_platform_segment(segments[LDTR_SLOT]);
    state.tr = from_platform_segment(segments[TR_SLOT]);

    let tables: [TableRegister; 2] =
        core::array::from_fn(|index| match values[TABLES_AT + index] {
            RegisterValue::Table(table) => table,
            _ => TableRegister::default(),
        });
    state.gdtr = DescriptorTableState {
        base: tables[GDTR_SLOT].base,
        limit: tables[GDTR_SLOT].limit,
    };
    state.idtr = DescriptorTableState {
        base: tables[IDTR_SLOT].base,
        limit: tables[IDTR_SLOT].limit,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box::cpu::arch_state::{SegmentAttributeParts, VECTOR_REGISTERS};
    use std::collections::BTreeMap;

    /// A processor that remembers what it was told, so the exchange can be
    /// exercised on a host with no hypervisor — which is every host the gates
    /// run on.
    #[derive(Default)]
    struct Recorder {
        words: core::cell::RefCell<BTreeMap<usize, u64>>,
        segments: core::cell::RefCell<BTreeMap<usize, SegmentRegister>>,
        tables: core::cell::RefCell<BTreeMap<usize, TableRegister>>,
    }

    /// A stable key per register, so a write and the read that follows it agree
    /// without the recorder knowing what any of them mean.
    fn key(reg: Reg) -> usize {
        ALL_REGS.iter().position(|candidate| *candidate == reg).expect("a known register")
    }

    /// The per-shape stores behind the recorder. Inherent rather than part of
    /// [`VpRegisters`], because that trait carries only what the exchange asks
    /// of a real processor and a mixed batch is all it asks.
    impl Recorder {
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
            }
            Ok(())
        }

        fn write_words(&self, regs: &[Reg], words: &[u64]) -> WhpResult<()> {
            let mut stored = self.words.borrow_mut();
            for (reg, word) in regs.iter().zip(words) {
                stored.insert(key(*reg), *word);
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
                        RegisterValue::Segment(seg[0])
                    }
                    RegisterValue::Table(_) => {
                        let mut table = [TableRegister::default()];
                        self.read_tables(&[*reg], &mut table)?;
                        RegisterValue::Table(table[0])
                    }
                };
            }
            Ok(())
        }

        fn write_registers(&self, regs: &[Reg], values: &[RegisterValue]) -> WhpResult<()> {
            for (reg, value) in regs.iter().zip(values) {
                match *value {
                    RegisterValue::Word(word) => self.write_words(&[*reg], &[word])?,
                    RegisterValue::Segment(seg) => self.write_segments(&[*reg], &[seg])?,
                    RegisterValue::Table(table) => self.write_tables(&[*reg], &[table])?,
                }
            }
            Ok(())
        }

    }

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
            !vp.words.borrow().contains_key(&key(Reg::Tsc)),
            "the seam wrote the shadow's time-stamp counter into the processor"
        );
        // The MSR beside it in the list did land, so this is not a batch that
        // silently failed to write anything.
        assert_eq!(vp.words.borrow().get(&key(Reg::Pat)).copied(), Some(state.msrs.pat));
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

    /// The register file the seam deliberately leaves alone stays untouched, so
    /// an export cannot quietly zero state the shadow processor still owns.
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

const fn msr_words(msrs: &MsrState, xcr0: u32) -> [u64; MSR_VALUES] {
    [
        msrs.efer,
        msrs.apic_base,
        msrs.star,
        msrs.lstar,
        msrs.cstar,
        msrs.sfmask,
        msrs.kernel_gs_base,
        msrs.sysenter_cs,
        msrs.sysenter_esp,
        msrs.sysenter_eip,
        msrs.pat,
        xcr0 as u64,
        msrs.tsc,
    ]
}
