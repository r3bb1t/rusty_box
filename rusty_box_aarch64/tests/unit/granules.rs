//! The translation granules (FEAT_TGran16K, FEAT_TGran64K, FEAT_S2TGran16K,
//! FEAT_S2TGran64K), small translation tables (FEAT_TTST), the 64 KiB
//! granule's 52-bit addresses (FEAT_LPA, FEAT_LVA), and the 4 KiB and 16
//! KiB granules' under `DS` (FEAT_LPA2) as a guest sees them (R9): loads,
//! `AT` and pointer authentication at EL1 through stage 1 tables of each
//! granule and input address size, loads at EL1 through stage 2 tables of
//! each granule and start level, the fault status codes of level -1, and
//! the IPA size `HPFAR_EL2` holds, on processors with each feature and
//! without. (The ID registers' granule and address size fields are
//! `el2.rs`'s `the_id_registers_offer_each_models_granules_at_both_stages`.)
//!
//! The rules are the Arm ARM's pseudocode: `AArch64_S1TTWParamsEL10`
//! (`AArch64_S1DecodeTG0`, `AArch64_S1DecodeTG1`, `AArch64_HaveS1TG`),
//! `AArch64_S1Translate` (`AArch64_S1TxSZFaults`, `AArch64_MaxTxSZ`),
//! `AArch64_S1StartLevel`, `AArch64_TTEntryAddressBounds`,
//! `AArch64_DecodeDescriptorType` (`AArch64_BlockDescSupported`),
//! `AArch64_S1LeafBase`, `AArch64_NSS2TTWParams` (`AArch64_S2DecodeTG0`,
//! `AArch64_HaveS2TG`), `AArch64_S2Translate` (`AArch64_S2TxSZFaults`,
//! `AArch64_S2InvalidSL`, `AArch64_S2InconsistentSL`),
//! `AArch64_S2StartLevel`, `AArch64_PACEffectiveTxSZ`,
//! `AArch64_PhysicalAddressSize`, `AArch64_S1TTBaseAddress`,
//! `AArch64_NextTableBase`, `AArch64_S1DisabledOutput` and
//! `AArch64_EncodePAR`; the stage 2 fault's report is `AArch64_DataAbort`
//! with `HPFAR_EL2`. For FEAT_LPA2, `AArch64_S1TTWParamsEL10`,
//! `AArch64_S1TTWParamsEL2` and `AArch64_NSS2TTWParams` (where `DS` and
//! `SL2` apply), `AArch64_S1MinTxSZ`, `AArch64_S2MinTxSZ`,
//! `AArch64_S1NextWalkStateLeaf` and `AArch64_S2NextWalkStateLeaf` (the
//! shareability), and `EncodeLDFSC` (level -1's codes).
//!
//! The words are GNU as's (2.42, `-march=armv8.3-a`, objdump -d).

use core::ops::{Deref, DerefMut};
use std::vec::Vec;

use super::el2::{El2Reg, HCR_TEA, HCR_VM};
use super::el2_tests::{self, restored, SavedTlb};
use super::features::{Feature, Implementation};
use super::mmu::saved_layout::{ENTRY_LEVEL, ENTRY_S2_LEVEL, KEPT_BYTES, KEPT_LEVEL, NOT_WALKED};
use super::{id, sctlr, El, WithEl2};

const LDR: u32 = 0xF940_0022; // ldr x2, [x1]
const AT_S1E1R: u32 = 0xD508_7801; // at s1e1r, x1
const AT_S12E1R: u32 = 0xD50C_7881; // at s12e1r, x1
const AT_S1E2R: u32 = 0xD50C_7801; // at s1e2r, x1
const XPACI: u32 = 0xDAC1_43E0; // xpaci x0
const MSR_HPFAR: u32 = 0xD51C_6080; // msr hpfar_el2, x0
const MRS_HPFAR: u32 = 0xD53C_6080; // mrs x0, hpfar_el2

/// The processor models: the Cortex-A57, with the 4 KiB granule alone and
/// no FEAT_TTST, and the Armv9.2 one, with every granule and FEAT_TTST.
const A57: Implementation = Implementation::CORTEX_A57;
const V9: Implementation = Implementation::ARMV9P2;

/// Where the instruction under test executes (its virtual address and,
/// mapped to itself, its physical one), within the 16-bit input address
/// FEAT_TTST allows and apart from the vectors at 0x8000 and 0xA000; the
/// translation tables' memory; and the page every walk's leaf maps, below
/// the end of RAM at 16 MiB.
const CODE: u64 = 0x4000;
const TABLES: u64 = 0x10_0000;
const DATA: u64 = 0xC0_0000;

/// The input address each walk translates: its bits below the input
/// address size, which index every level and the page, up to 52 bits.
const PATTERN: u64 = 0x000B_9A5C_3E71_D468;

/// Descriptor bits: a block (bit 1 clear) or a table or page (bit 1 set),
/// and the access flag.
const BLOCK: u64 = 0b01;
const TABLE: u64 = 0b11;
const AF: u64 = 1 << 10;
/// A stage 2 leaf's `S2AP` read and write and `MemAttr` Normal Write-Back.
const S2_RW_NORMAL: u64 = (0b11 << 6) | (0b1111 << 2);

/// `TCR_EL1` and `VTCR_EL2` fields: `TG0` (15:14), `TG1` (31:30), `EPD1`,
/// `T1SZ`, `IPS` and `PS` of 44 bits, `SL0` (7:6).
const TG0: u32 = 14;
const TG1: u32 = 30;
const EPD1: u64 = 1 << 23;
const T1SZ: u32 = 16;
const IPS_44: u64 = 0b100 << 32;
const PS_44: u64 = 0b100 << 16;
const SL0: u32 = 6;
/// `IPS` and `PS` of 48 and 52 bits, and `IPS` of 56, which
/// [`Machine::stage1`] and [`Machine::stage2`]'s 44 bits leave as they are
/// when ORed in.
const IPS_48: u64 = 0b101 << 32;
const IPS_52: u64 = 0b110 << 32;
const IPS_56: u64 = 0b111 << 32;
const PS_48: u64 = 0b101 << 16;
const PS_52: u64 = 0b110 << 16;
/// FEAT_LPA2's `TCR_EL1.DS`, `TCR_EL2.DS`, and `VTCR_EL2`'s `DS` and `SL2`.
const DS_EL1: u64 = 1 << 59;
const DS_EL2: u64 = 1 << 32;
const VTCR_DS: u64 = 1 << 32;
const VTCR_SL2: u64 = 1 << 33;
/// `TCR_EL1.SH0` and `SH1`, `TCR_EL2.SH0` and `VTCR_EL2.SH0`, and
/// `PAR_EL1.SH`: 0b10 Outer Shareable, 0b11 Inner Shareable.
const SH0: u32 = 12;
const SH1: u32 = 28;
const PAR_SH: u32 = 7;
const OUTER: u64 = 0b10;
const INNER: u64 = 0b11;

/// The `TG0` encodings, and `TG1`'s.
const TG0_4K: u64 = 0b00;
const TG0_64K: u64 = 0b01;
const TG0_16K: u64 = 0b10;
const TG1_16K: u64 = 0b01;
const TG1_4K: u64 = 0b10;
const TG1_64K: u64 = 0b11;
/// Reserved: `TG0` = 0b11, `TG1` = 0b00.
const TG0_RESERVED: u64 = 0b11;
const TG1_RESERVED: u64 = 0b00;

/// Exception classes and fault status codes.
const EC_INSTRUCTION_ABORT_LOWER_EL: u64 = 0x20;
const EC_INSTRUCTION_ABORT_SAME_EL: u64 = 0x21;
const EC_DATA_ABORT_LOWER_EL: u64 = 0x24;
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
/// The fault status codes at a level (`EncodeLDFSC`), of which level -1's
/// (FEAT_LPA2's) are codes of their own.
const fn translation(level: i32) -> u64 {
    if level == -1 {
        0b10_1011
    } else {
        0b00_0100 | level as u64
    }
}
const fn address_size(level: i32) -> u64 {
    if level == -1 {
        0b10_1001
    } else {
        level as u64
    }
}
const fn external_on_walk(level: i32) -> u64 {
    if level == -1 {
        0b01_0011
    } else {
        0b01_0100 | level as u64
    }
}

/// Translation tables of one granule as a test lays them out: the page
/// size's bits, the input address size, the level the walk starts at, the
/// start level's table, and the bits each table descriptor
/// [`Machine::map`] makes holds besides the table's address in RAM — below
/// the page, or address bits from 48 up.
#[derive(Debug, Clone, Copy)]
struct Tables {
    page_bits: u32,
    ia_bits: u32,
    start: i32,
    root: u64,
    extra: u64,
}

impl Tables {
    /// The page size.
    const fn page(self) -> u64 {
        1 << self.page_bits
    }

    /// The input address bits a level of eight-byte descriptors resolves.
    const fn stride(self) -> u32 {
        self.page_bits - 3
    }

    /// The lowest input address bit `level`'s tables index.
    const fn lsb(self, level: i32) -> u32 {
        self.page_bits + self.stride() * (3 - level) as u32
    }

    /// The input address bits `level`'s tables index: a stride, but at the
    /// start level the rest of the input address, more than a stride for
    /// stage 2's concatenated tables.
    const fn width(self, level: i32) -> u32 {
        if level == self.start {
            self.ia_bits - self.lsb(level)
        } else {
            self.stride()
        }
    }

    /// The index of `ia`'s entry in its table at `level`.
    const fn index(self, level: i32, ia: u64) -> u64 {
        (ia >> self.lsb(level)) & ((1 << self.width(level)) - 1)
    }
}

/// A stage 1 page or block descriptor of Normal memory (`AttrIndx` 0) at
/// `out`; or a stage 2 one, every access allowed. Their bits 9:8, `SH`
/// (Non-shareable) or under FEAT_LPA2's `DS` address bits 51:50, are 0, so
/// that one descriptor gives `out` with `DS` and without.
const fn s1_leaf(out: u64, kind: u64) -> u64 {
    out | AF | kind
}
const fn s2_leaf(out: u64, kind: u64) -> u64 {
    out | AF | S2_RW_NORMAL | kind
}

/// An exception the step took: the level, the class and the fault status
/// code of its syndrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    to: El,
    ec: u64,
    fsc: u64,
}

/// The EL2 tests' processor with EL2 ([`el2_tests::Machine`]) and the
/// memory its translation tables are made in.
struct Machine {
    inner: el2_tests::Machine<WithEl2>,
    /// The first byte of [`TABLES`] no table holds.
    free: u64,
}

impl Deref for Machine {
    type Target = el2_tests::Machine<WithEl2>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Machine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Machine {
    /// A processor implementing `imp` at EL1, out of reset with its vector
    /// bases set, both stages of translation off, and `MAIR_EL1` attribute
    /// 0 Normal Write-Back memory.
    fn new(imp: Implementation) -> Self {
        let mut inner = el2_tests::Machine::<WithEl2>::new(imp);
        inner.cpu.sys.mair_el1 = 0xFF;
        inner.at(El::El1);
        Self {
            inner,
            free: TABLES,
        }
    }

    /// Step `insn` at [`CODE`]: the exception it took, if any. The
    /// processor is left at the level it executed at.
    fn run(&mut self, insn: u32) -> Option<Taken> {
        let pstate = self.cpu.pstate;
        let taken = self.inner.run_at(CODE, insn)?;
        self.cpu.pstate = pstate;
        Some(Taken {
            to: taken.to,
            ec: taken.esr >> 26,
            fsc: taken.esr & 0x3F,
        })
    }

    /// `bytes` of zeroed memory for tables, aligned to their size.
    fn alloc(&mut self, bytes: u64) -> u64 {
        let at = self.free.next_multiple_of(bytes);
        self.free = at + bytes;
        assert!(self.free <= DATA, "the tables fit below DATA");
        at
    }

    /// Empty tables of `2^page_bits`-byte pages for `ia_bits`-bit input
    /// addresses whose walk starts at `start`: the start level's table, or
    /// concatenated tables, aligned to their size.
    fn tables(&mut self, page_bits: u32, ia_bits: u32, start: i32) -> Tables {
        let mut tables = Tables {
            page_bits,
            ia_bits,
            start,
            root: 0,
            extra: 0,
        };
        let root_bytes = 8u64 << tables.width(start);
        tables.root = self.alloc(root_bytes.max(tables.page()));
        tables
    }

    /// Map input address `ia` with the descriptor `leaf` at `level` of
    /// `tables`, making the tables on the way.
    fn map(&mut self, tables: Tables, ia: u64, level: i32, leaf: u64) {
        let mut table = tables.root;
        for l in tables.start..level {
            let at = table + 8 * tables.index(l, ia);
            let mut desc = self.ram.get64(at);
            if desc == 0 {
                desc = self.alloc(tables.page()) | tables.extra | TABLE;
                self.ram.put64(at, desc);
            }
            table = desc & !tables.extra & !(tables.page() - 1);
        }
        self.ram.put64(table + 8 * tables.index(level, ia), leaf);
    }

    /// Turn stage 1 of the EL1&0 regime on: `TCR_EL1` `tcr` (`IPS` 44 bits
    /// added), `TTBR0_EL1` `ttbr0` and `TTBR1_EL1` `ttbr1`, the regime's
    /// context left to be found again as an `MSR`'s change of it leaves it.
    fn stage1(&mut self, tcr: u64, ttbr0: u64, ttbr1: u64) {
        let sys = &mut self.cpu.sys;
        sys.tcr_el1 = tcr | IPS_44;
        sys.ttbr0_el1 = ttbr0;
        sys.ttbr1_el1 = ttbr1;
        sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;
        self.cpu.el10_context_changed();
    }

    /// Turn stage 2 on (`HCR_EL2.VM`) with `VTCR_EL2` `vtcr` (`PS` 44 bits
    /// added, its RES1 bit with the write) and `VTTBR_EL2` `vttbr`.
    fn stage2(&mut self, vtcr: u64, vttbr: u64) {
        let features = self.cpu.imp.features;
        let el2 = &mut self.cpu.el2_state.regs;
        el2.write(El2Reg::Vtcr, features, 0, vtcr | PS_44);
        el2.vttbr = vttbr;
        el2.hcr |= HCR_VM;
        self.cpu.el10_context_changed();
    }

    /// Load from `va` at EL1: the doubleword, or the exception taken.
    fn load(&mut self, va: u64) -> Result<u64, Taken> {
        self.cpu.x[1] = va;
        match self.run(LDR) {
            None => Ok(self.cpu.x[2]),
            Some(taken) => Err(taken),
        }
    }

    /// `AT S1E1R` of `va` at EL1: `PAR_EL1`.
    fn at_s1e1r(&mut self, va: u64) -> u64 {
        self.cpu.x[1] = va;
        assert_eq!(self.run(AT_S1E1R), None, "AT S1E1R {va:#x}");
        self.cpu.sys.par_el1
    }

    /// Whether a restore of `image` into the TLB of a processor holding this
    /// one's registers refuses it as holding an entry no walk makes.
    fn refuses(&self, image: &[u8]) -> bool {
        match restored::<WithEl2>(&self.tlb_context(), image) {
            Ok(_) => false,
            Err(refusal) => {
                assert_eq!(refusal, NOT_WALKED);
                true
            }
        }
    }

    /// [`Machine::refuses`] of `saved` with the byte at `at` set to `level`.
    fn refuses_level(&self, saved: &SavedTlb, at: usize, level: u8) -> bool {
        let mut image = saved.0.clone();
        image[at] = level;
        self.refuses(&image)
    }
}

/// `PAR_EL1`'s `F`, its `PA` (47:12), with FEAT_LPA's bits 51:48 too, a
/// fault's `FST` (6:1), `S`, a fault of stage 2, and `PTW`, one on stage
/// 1's walk.
const PAR_F: u64 = 1;
const PAR_PA: u64 = 0x0000_FFFF_FFFF_F000;
const PAR_PA_52: u64 = 0x000F_FFFF_FFFF_F000;
const PAR_FST: u64 = 0x3F << 1;
const PAR_S: u64 = 1 << 9;
const PAR_PTW: u64 = 1 << 8;

/// The doubleword each test puts where a translation of `ia` leads.
const fn marker(ia: u64) -> u64 {
    ia ^ 0x5A5A_0000_0000_5A5A
}

/// A stage 1 Data Abort at EL1 with fault status `fsc`.
fn s1_fault(fsc: u64) -> Result<u64, Taken> {
    Err(Taken {
        to: El::El1,
        ec: EC_DATA_ABORT_SAME_EL,
        fsc,
    })
}

/// A stage 1 Data Abort at EL1 with Translation fault status at `level`.
fn s1_translation_fault(level: i32) -> Result<u64, Taken> {
    s1_fault(translation(level))
}

/// A processor implementing `imp` at EL1 with stage 1 on over tables of
/// the granule of `2^page_bits`-byte pages that `TG0` `tg0` names, for
/// `T0SZ` `t0sz`, whose walk starts at `start`, [`CODE`]'s page mapped to
/// itself; and those tables.
fn lower_range(
    imp: Implementation,
    tg0: u64,
    page_bits: u32,
    t0sz: u32,
    start: i32,
) -> (Machine, Tables) {
    let mut m = Machine::new(imp);
    let tables = m.tables(page_bits, 64 - t0sz, start);
    let code_page = CODE & !(tables.page() - 1);
    m.map(tables, code_page, 3, s1_leaf(code_page, TABLE));
    m.stage1(u64::from(t0sz) | (tg0 << TG0) | EPD1, tables.root, 0);
    (m, tables)
}

/// [`lower_range`] for the EL2 regime: a processor implementing `imp` at
/// EL2 with its stage 1 on over tables of the granule `TCR_EL2.TG0` `tg0`
/// names for `T0SZ` `t0sz`, whose walk starts at `start`, [`CODE`]'s page
/// mapped to itself; and those tables.
fn el2_range(
    imp: Implementation,
    tg0: u64,
    page_bits: u32,
    t0sz: u32,
    start: i32,
) -> (Machine, Tables) {
    let mut m = Machine::new(imp);
    let tables = m.tables(page_bits, 64 - t0sz, start);
    let code_page = CODE & !(tables.page() - 1);
    m.map(tables, code_page, 3, s1_leaf(code_page, TABLE));
    let features = m.cpu.imp.features;
    let tcr = u64::from(t0sz) | (tg0 << TG0) | PS_44;
    let regs = &mut m.cpu.el2_state.regs;
    regs.write(El2Reg::Tcr, features, 0, tcr);
    regs.ttbr0 = tables.root;
    regs.mair = 0xFF;
    regs.sctlr |= 1;
    m.at(El::El2);
    (m, tables)
}

/// [`lower_range`]'s upper-range sibling: a processor implementing `imp`
/// with `TTBR1_EL1`'s tables of the granule of `2^page_bits`-byte pages
/// that `TG1` `tg1` names, for `T1SZ` 25, whose walk starts at `start`, and
/// `TTBR0_EL1`'s 4 KiB ones mapping [`CODE`]'s page to itself; and the
/// upper range's tables.
fn upper_range(imp: Implementation, tg1: u64, page_bits: u32, start: i32) -> (Machine, Tables) {
    let mut m = Machine::new(imp);
    let lower = m.tables(12, 39, 1);
    let code_page = CODE & !0xFFF;
    m.map(lower, code_page, 3, s1_leaf(code_page, TABLE));
    let upper = m.tables(page_bits, 39, start);
    m.stage1(
        25 | (TG0_4K << TG0) | (25 << T1SZ) | (tg1 << TG1),
        lower.root,
        upper.root,
    );
    (m, upper)
}

/// An address of the upper range of 39 bits.
const UPPER_VA: u64 = (!0 << 39) | (PATTERN & ((1 << 39) - 1));

/// `va`, in the range `tables` translate, mapped by a page to the page at
/// [`DATA`], read at EL1, translated by `AT S1E1R` to where the page puts
/// it, its neighbouring page unmapped at level 3 and the address just
/// beyond the range on its side (above the lower, below the upper) out of
/// it at level 0.
fn walks_to_its_page(m: &mut Machine, tables: Tables, va: u64, case: &str) {
    let page = tables.page();
    let offset = va & (page - 1);
    m.map(tables, va, 3, s1_leaf(DATA, TABLE));
    m.ram.put64(DATA + offset, marker(va));
    assert_eq!(m.load(va), Ok(marker(va)), "{case}: load");
    assert_eq!(
        m.at_s1e1r(va) & (PAR_PA | PAR_F),
        (DATA + offset) & PAR_PA,
        "{case}: PAR_EL1"
    );
    let neighbour = va ^ page;
    if neighbour / page != CODE / page {
        assert_eq!(
            m.load(neighbour),
            s1_translation_fault(3),
            "{case}: the next page"
        );
        assert_eq!(m.cpu.sys.far_el1, neighbour, "{case}: FAR_EL1");
    }
    let beyond = if (va >> 55) & 1 == 1 {
        va & !(1 << tables.ia_bits)
    } else {
        va | (1 << tables.ia_bits)
    };
    assert_eq!(
        m.load(beyond),
        s1_translation_fault(0),
        "{case}: out of range"
    );
}

/// Each granule's walk starts at the level whose table resolves the input
/// address's top bit, each level after it resolving a stride (9, 11 and 13
/// bits) and the page the rest (12, 14 and 16): a page at level 3 of each
/// granule and input address size, down to FEAT_TTST's 16 bits (17 with
/// the 64 KiB granule), is where a load reads and where `AT` puts the
/// address, the page offset the address's own bits below the page size.
#[test]
fn each_granule_walks_from_the_level_its_input_address_size_gives() {
    let cases = [
        // (TG0, page bits, T0SZ, start level)
        (TG0_4K, 12, 16, 0),
        (TG0_4K, 12, 25, 1),
        (TG0_4K, 12, 34, 2),
        (TG0_4K, 12, 48, 3),
        (TG0_16K, 14, 16, 0),
        (TG0_16K, 14, 17, 1),
        (TG0_16K, 14, 25, 1),
        (TG0_16K, 14, 28, 2),
        (TG0_16K, 14, 48, 3),
        (TG0_64K, 16, 16, 1),
        (TG0_64K, 16, 22, 2),
        (TG0_64K, 16, 25, 2),
        (TG0_64K, 16, 35, 3),
        (TG0_64K, 16, 47, 3),
    ];
    for (tg0, page_bits, t0sz, start) in cases {
        let case = std::format!("TG0 {tg0:#b}, T0SZ {t0sz}");
        let (mut m, tables) = lower_range(V9, tg0, page_bits, t0sz, start);
        let va = PATTERN & ((1 << tables.ia_bits) - 1);
        assert_ne!(va / tables.page(), CODE / tables.page(), "{case}");
        walks_to_its_page(&mut m, tables, va, &case);
    }
}

/// A descriptor with bit 1 clear is a block only at a level its granule
/// has blocks at on its processor (without FEAT_LPA2's `DS`): levels 1 and
/// 2 with the 4 KiB granule, level 2 with the 16 KiB one, and level 2 with
/// the 64 KiB one, and level 1 too with a 52-bit physical address
/// (FEAT_LPA). At level 0, at the 16 KiB granule's level 1, at the 64 KiB
/// granule's without a 52-bit physical address, and at level 3 it is
/// invalid, a Translation fault at its level; at stage 1, and for the 64
/// KiB granule's level 1 at stage 2.
#[test]
fn a_block_is_a_leaf_only_at_the_levels_its_granule_has_blocks_at() {
    const BLOCK_VA: u64 = 0x8000_0000_0000;
    const OFFSET: u64 = 0x3_0468;
    let no_lpa = Implementation::every_feature_but(&[Feature::Lpa]);
    let cases = [
        // (processor, TG0, page bits, start level, block level, a block there)
        ("v9.2", V9, TG0_4K, 12, 0, 0, false),
        ("v9.2", V9, TG0_4K, 12, 0, 1, true),
        ("v9.2", V9, TG0_4K, 12, 0, 2, true),
        ("v9.2", V9, TG0_4K, 12, 0, 3, false),
        ("v9.2", V9, TG0_16K, 14, 0, 0, false),
        ("v9.2", V9, TG0_16K, 14, 0, 1, false),
        ("v9.2", V9, TG0_16K, 14, 0, 2, true),
        ("v9.2", V9, TG0_16K, 14, 0, 3, false),
        ("v9.2", V9, TG0_64K, 16, 1, 1, true),
        ("v9.2", V9, TG0_64K, 16, 1, 2, true),
        ("v9.2", V9, TG0_64K, 16, 1, 3, false),
        ("no LPA", no_lpa, TG0_64K, 16, 1, 1, false),
        ("no LPA", no_lpa, TG0_64K, 16, 1, 2, true),
        ("no LPA", no_lpa, TG0_4K, 12, 0, 1, true),
    ];
    for (name, imp, tg0, page_bits, start, level, block) in cases {
        let case = std::format!("{name}: TG0 {tg0:#b}, level {level}");
        let (mut m, tables) = lower_range(imp, tg0, page_bits, 16, start);
        m.map(tables, BLOCK_VA, level, s1_leaf(0, BLOCK));
        m.ram.put64(OFFSET, marker(OFFSET));
        let want = if block {
            Ok(marker(OFFSET))
        } else {
            s1_translation_fault(level)
        };
        assert_eq!(m.load(BLOCK_VA + OFFSET), want, "{case}");
    }

    // Stage 2's: a level 1 block of the 64 KiB granule, the low 4 TiB of
    // IPAs mapped to themselves, under a walk starting at level 1 (`SL0`
    // 0b10, `T0SZ` the smallest the processor takes). A load at EL1 reads
    // through it where `PAMax` is 52 bits; where it is 44 the instruction
    // fetch takes a Translation fault at level 1 to EL2.
    for (name, imp, t0sz, block) in [("v9.2", V9, 12, true), ("no LPA", no_lpa, 20, false)] {
        let mut m = Machine::new(imp);
        let tables = m.tables(16, 64 - t0sz, 1);
        m.map(tables, 0, 1, s2_leaf(0, BLOCK));
        let vtcr = u64::from(t0sz) | (0b10 << SL0) | (TG0_64K << TG0);
        m.stage2(vtcr, tables.root);
        m.ram.put64(DATA, marker(DATA));
        let want = if block {
            Ok(marker(DATA))
        } else {
            Err(Taken {
                to: El::El2,
                ec: EC_INSTRUCTION_ABORT_LOWER_EL,
                fsc: translation(1),
            })
        };
        assert_eq!(m.load(DATA), want, "{name}: stage 2");
    }
}

/// The upper range walks with the granule `TCR_EL1.TG1` names, whose
/// encodings differ from `TG0`'s, from `TTBR1_EL1`; the reserved 0b00
/// names the 4 KiB granule (the IMPLEMENTATION DEFINED "TG1 encoded
/// granule size").
#[test]
fn the_upper_range_walks_with_the_granule_tg1_names() {
    let cases = [
        // (TG1, page bits, start level for 39 bits)
        (TG1_4K, 12, 1),
        (TG1_16K, 14, 1),
        (TG1_64K, 16, 2),
        (TG1_RESERVED, 12, 1),
    ];
    for (tg1, page_bits, start) in cases {
        let (mut m, tables) = upper_range(V9, tg1, page_bits, start);
        walks_to_its_page(&mut m, tables, UPPER_VA, &std::format!("TG1 {tg1:#b}"));
    }
}

/// A `TG0` or `TG1` naming a granule the processor lacks, or `TG0`'s
/// reserved 0b11, names the 4 KiB granule (`AArch64_S1DecodeTG0` and
/// `TG1` with the IMPLEMENTATION DEFINED "TG0 encoded granule size" and
/// "TG1 encoded granule size"): the walk reads 4 KiB tables. The Cortex-A57
/// has the 4 KiB granule alone.
#[test]
fn a_granule_the_processor_lacks_is_the_4k_one() {
    let no_16k = Implementation::every_feature_but(&[Feature::TGran16K, Feature::S2TGran16K]);
    let no_64k = Implementation::every_feature_but(&[Feature::TGran64K, Feature::S2TGran64K]);
    let lower_cases = [
        // (processor, TG0, page bits, start level for 39 bits)
        ("no 16K", no_16k, TG0_16K, 12, 1),
        ("no 64K", no_64k, TG0_64K, 12, 1),
        ("A57", A57, TG0_16K, 12, 1),
        ("A57", A57, TG0_64K, 12, 1),
        ("A57", A57, TG0_RESERVED, 12, 1),
        ("no 64K", no_64k, TG0_16K, 14, 1),
        ("no 16K", no_16k, TG0_64K, 16, 2),
    ];
    for (name, imp, tg0, page_bits, start) in lower_cases {
        let case = std::format!("{name}: TG0 {tg0:#b}");
        let (mut m, tables) = lower_range(imp, tg0, page_bits, 25, start);
        let va = PATTERN & ((1 << 39) - 1);
        walks_to_its_page(&mut m, tables, va, &case);
    }
    let upper_cases = [
        // (processor, TG1, page bits, start level for 39 bits)
        ("no 16K", no_16k, TG1_16K, 12, 1),
        ("no 64K", no_64k, TG1_64K, 12, 1),
        ("A57", A57, TG1_16K, 12, 1),
        ("A57", A57, TG1_64K, 12, 1),
    ];
    for (name, imp, tg1, page_bits, start) in upper_cases {
        let (mut m, tables) = upper_range(imp, tg1, page_bits, start);
        let case = std::format!("{name}: TG1 {tg1:#b}");
        walks_to_its_page(&mut m, tables, UPPER_VA, &case);
    }
}

/// A descriptor's bits between bit 12 and its granule's page are not its
/// address (Arm `AArch64_NextTableBase`, `AArch64_S1LeafBase`,
/// `AArch64_S2LeafBase`): with every one of them set in each table and page
/// descriptor of the 16 KiB granule, on the Armv9.2 model and on a
/// processor without FEAT_LPA, and of the 64 KiB granule on the latter
/// (where `PAMax` is 52 bits they are address bits 51:48,
/// `a_64k_descriptor_holds_bits_51_to_48_of_its_address_in_bits_15_to_12`),
/// a load reads what it reads without them, at stage 1 and at stage 2.
#[test]
fn the_bits_below_a_page_in_a_descriptor_are_not_its_address() {
    let no_lpa = Implementation::every_feature_but(&[Feature::Lpa]);
    let cases = [
        ("v9.2", V9, TG0_16K, 14, 1),
        ("no LPA", no_lpa, TG0_16K, 14, 1),
        ("no LPA", no_lpa, TG0_64K, 16, 2),
    ];
    for (name, imp, tg0, page_bits, start) in cases {
        let below_page = ((1 << page_bits) - 1) & !0xFFF;
        let va = PATTERN & ((1 << 39) - 1);
        let offset = va & ((1 << page_bits) - 1);
        let (mut m, mut tables) = lower_range(imp, tg0, page_bits, 25, start);
        tables.extra = below_page;
        m.map(tables, va, 3, s1_leaf(DATA | below_page, TABLE));
        m.ram.put64(DATA + offset, marker(va));
        assert_eq!(m.load(va), Ok(marker(va)), "{name}: stage 1, TG0 {tg0:#b}");

        let mut m = Machine::new(imp);
        // T0SZ 24, a 40-bit IPA, from level 2 (`SL0` 0b01).
        let mut tables = m.tables(page_bits, 40, 2);
        tables.extra = below_page;
        let code_page = CODE & !(tables.page() - 1);
        m.map(tables, code_page, 3, s2_leaf(code_page | below_page, TABLE));
        let ipa = va & ((1 << 40) - 1);
        m.map(tables, ipa, 3, s2_leaf(DATA | below_page, TABLE));
        m.stage2(24 | (0b01 << SL0) | (tg0 << TG0), tables.root);
        m.ram.put64(DATA + offset, marker(ipa));
        let read = m.load(ipa);
        assert_eq!(read, Ok(marker(ipa)), "{name}: stage 2, TG0 {tg0:#b}");
    }
}

/// A granule a processor has at stage 1 alone (FEAT_TGran16K and
/// FEAT_TGran64K without FEAT_S2TGran16K and FEAT_S2TGran64K) walks at
/// stage 1 and names the 4 KiB granule at stage 2 (`AArch64_HaveS1TG`,
/// `AArch64_HaveS2TG`), as `ID_AA64MMFR0_EL1` says: `TGran16` 2 (supported,
/// with FEAT_LPA2's 52-bit addresses) and `TGran64` 0 (supported),
/// `TGran16_2` and `TGran64_2` 1 (not).
#[test]
fn a_granule_at_stage_1_alone_is_the_4k_one_at_stage_2() {
    const MRS_MMFR0: u32 = 0xD538_0700; // mrs x0, id_aa64mmfr0_el1
    let imp = Implementation::every_feature_but(&[Feature::S2TGran16K, Feature::S2TGran64K]);
    let mut m = Machine::new(imp);
    assert_eq!(m.run(MRS_MMFR0), None);
    let mmfr0 = m.cpu.x[0];
    let fields = [20, 24, 32, 36].map(|lsb| (mmfr0 >> lsb) & 0xF);
    // TGran16, TGran64, TGran16_2, TGran64_2.
    assert_eq!(fields, [2, 0, 1, 1]);
    for (tg0, page_bits, start) in [(TG0_16K, 14, 1), (TG0_64K, 16, 2)] {
        let case = std::format!("stage 1, TG0 {tg0:#b}");
        let (mut m, tables) = lower_range(imp, tg0, page_bits, 25, start);
        walks_to_its_page(&mut m, tables, PATTERN & ((1 << 39) - 1), &case);
    }
    for tg0 in [TG0_16K, TG0_64K] {
        let mut m = Machine::new(imp);
        identity_stage2(&mut m, 24 | (0b01 << SL0) | (tg0 << TG0), 12, 40, 1);
        m.ram.put64(DATA + 0x468, marker(DATA));
        let read = m.load(DATA + 0x468);
        assert_eq!(read, Ok(marker(DATA)), "stage 2, TG0 {tg0:#b}");
    }
}

/// `TxSZ` above `AArch64_MaxTxSZ` — 39 without FEAT_TTST, with it 48, or 47
/// with the 64 KiB granule — or below `AArch64_S1MinTxSZ` — 16, but 12 with
/// FEAT_LVA and the 64 KiB granule — takes a level 0 Translation fault for
/// every address (the IMPLEMENTATION DEFINED "Fault on TxSZ value above
/// maximum" and "... below minimum", which FEAT_LVA makes no choice),
/// which `AT S1E1R` at EL2 reports in `PAR_EL1`; within them the walk
/// translates.
#[test]
fn txsz_beyond_its_granules_range_faults_at_level_0() {
    let no_ttst = Implementation::every_feature_but(&[Feature::Ttst]);
    let no_lva = Implementation::every_feature_but(&[Feature::Lva, Feature::Lpa2]);
    let cases = [
        // (processor, TG0, T0SZ, translates)
        ("TTST", V9, TG0_4K, 48, true),
        ("TTST", V9, TG0_4K, 49, false),
        ("TTST", V9, TG0_16K, 48, true),
        ("TTST", V9, TG0_16K, 49, false),
        ("TTST", V9, TG0_64K, 47, true),
        ("TTST", V9, TG0_64K, 48, false),
        ("TTST", V9, TG0_4K, 16, true),
        ("TTST", V9, TG0_4K, 15, false),
        ("LVA", V9, TG0_64K, 12, true),
        ("LVA", V9, TG0_64K, 11, false),
        ("LVA", V9, TG0_4K, 12, false),
        ("LVA", V9, TG0_16K, 15, false),
        ("no LVA", no_lva, TG0_64K, 16, true),
        ("no LVA", no_lva, TG0_64K, 15, false),
        ("no TTST", no_ttst, TG0_4K, 39, true),
        ("no TTST", no_ttst, TG0_4K, 40, false),
        ("no TTST", no_ttst, TG0_16K, 40, false),
        ("no TTST", no_ttst, TG0_64K, 39, true),
        ("no TTST", no_ttst, TG0_64K, 40, false),
        ("A57", A57, TG0_4K, 39, true),
        ("A57", A57, TG0_4K, 40, false),
        ("A57", A57, TG0_64K, 40, false),
    ];
    for (name, imp, tg0, t0sz, translates) in cases {
        let case = std::format!("{name}: TG0 {tg0:#b}, T0SZ {t0sz}");
        assert_eq!(at_txsz(imp, tg0, t0sz, 0), translates, "{case}");
    }
}

/// With FEAT_LPA2's `DS` (`TCR_EL1.DS`) the 4 KiB and 16 KiB granules'
/// `TxSZ` goes down to 12, a 52-bit input address (`AArch64_S1MinTxSZ`),
/// one below that a level 0 Translation fault; `DS` clear, or on a
/// processor without FEAT_LPA2, where it is no field, their least is 16.
#[test]
fn ds_takes_txsz_down_to_12_with_the_4k_and_16k_granules() {
    let no_lpa2 = Implementation::every_feature_but(&[Feature::Lpa2]);
    let cases = [
        // (processor, TG0, DS, T0SZ, translates)
        ("LPA2", V9, TG0_4K, DS_EL1, 12, true),
        ("LPA2", V9, TG0_4K, DS_EL1, 11, false),
        ("LPA2", V9, TG0_16K, DS_EL1, 12, true),
        ("LPA2", V9, TG0_16K, DS_EL1, 11, false),
        ("LPA2", V9, TG0_4K, 0, 15, false),
        ("LPA2", V9, TG0_16K, 0, 15, false),
        ("no LPA2", no_lpa2, TG0_4K, DS_EL1, 16, true),
        ("no LPA2", no_lpa2, TG0_4K, DS_EL1, 15, false),
        ("no LPA2", no_lpa2, TG0_16K, DS_EL1, 15, false),
    ];
    for (name, imp, tg0, ds, t0sz, translates) in cases {
        let case = std::format!("{name}: TG0 {tg0:#b}, DS {ds:#x}, T0SZ {t0sz}");
        assert_eq!(at_txsz(imp, tg0, t0sz, ds), translates, "{case}");
    }
}

/// Whether `AT S1E1R` at EL2 of an address in the smallest range of the
/// lower range of the EL1&0 regime, under `TCR_EL1` with `TG0` `tg0`,
/// `T0SZ` `t0sz` and the bits `tcr`, on a processor implementing `imp`,
/// translates it to where its page leads; otherwise `PAR_EL1` must report a
/// level 0 Translation fault.
fn at_txsz(imp: Implementation, tg0: u64, t0sz: u32, tcr: u64) -> bool {
    let mut m = Machine::new(imp);
    // Every address of the smallest range translates to itself: a page at
    // a level 3 table, under tables made for whichever level the walk
    // starts at.
    let page_bits = match tg0 {
        TG0_16K => 14,
        TG0_64K => 16,
        _ => 12,
    };
    // Wider than a page, as every `TxSZ` the walk takes gives, and no wider
    // than the granule's level -1, 0 or 1 table resolves.
    let ia_bits = (64 - t0sz).clamp(page_bits + 1, 52);
    let stride = page_bits - 3;
    let start = 3 - ((ia_bits - 1 - page_bits) / stride) as i32;
    let tables = m.tables(page_bits, ia_bits, start);
    m.map(tables, 0, 3, s1_leaf(DATA, TABLE));
    m.stage1(u64::from(t0sz) | (tg0 << TG0) | EPD1 | tcr, tables.root, 0);
    m.at(El::El2);
    let par = m.at_s1e1r(0x468) & (PAR_PA | PAR_FST | PAR_F);
    if par == DATA {
        return true;
    }
    assert_eq!(par, PAR_F | (translation(0) << 1), "a level 0 fault");
    false
}

/// The bottom bit of a pointer authentication code is `64 - TxSZ`, `TxSZ`
/// held within `AArch64_S1MinTxSZ` (16, or 12 with FEAT_LVA and the 64 KiB
/// granule or FEAT_LPA2's `DS`) and `AArch64_MaxTxSZ` of the range's
/// granule (`AArch64_PACEffectiveTxSZ`): `XPACI` clears the bits from there
/// up.
#[test]
fn a_pointer_authentication_code_ends_where_its_granules_txsz_range_does() {
    const POINTER: u64 = 0x000F_7FFF_FFFF_0123;
    let no_ttst = Implementation::every_feature_but(&[Feature::Ttst]);
    let no_64k = Implementation::every_feature_but(&[Feature::TGran64K, Feature::S2TGran64K]);
    let no_lva = Implementation::every_feature_but(&[Feature::Lva, Feature::Lpa2]);
    let cases = [
        // (processor, TG0, T0SZ, the code's bottom bit)
        ("TTST", V9, TG0_4K, 48, 16),
        ("TTST", V9, TG0_4K, 63, 16),
        ("TTST", V9, TG0_16K, 48, 16),
        ("TTST", V9, TG0_64K, 48, 17),
        ("TTST", V9, TG0_64K, 63, 17),
        ("TTST", V9, TG0_4K, 8, 48),
        ("TTST", V9, TG0_4K, 25, 39),
        ("LVA", V9, TG0_64K, 12, 52),
        ("LVA", V9, TG0_64K, 8, 52),
        ("LVA", V9, TG0_16K, 12, 48),
        ("no LVA", no_lva, TG0_64K, 12, 48),
        ("no TTST", no_ttst, TG0_4K, 48, 25),
        ("no TTST", no_ttst, TG0_64K, 48, 25),
        ("no 64K", no_64k, TG0_64K, 48, 16),
    ];
    // FEAT_LPA2's `DS` takes the 4 KiB and 16 KiB granules' least `TxSZ` to
    // 12 too; a processor without FEAT_LPA2 has no `DS`.
    let no_lpa2 = Implementation::every_feature_but(&[Feature::Lpa2]);
    let ds_cases = [
        ("LPA2", V9, TG0_4K, 12, 52),
        ("LPA2", V9, TG0_16K, 8, 52),
        ("LPA2", V9, TG0_4K, 25, 39),
        ("no LPA2", no_lpa2, TG0_4K, 12, 48),
    ];
    let cases = cases.map(|case| (case, 0)).into_iter();
    for ((name, imp, tg0, t0sz, bottom), ds) in cases.chain(ds_cases.map(|case| (case, DS_EL1))) {
        let mut m = Machine::new(imp);
        m.cpu.sys.tcr_el1 = t0sz | (tg0 << TG0) | EPD1 | ds;
        m.cpu.x[0] = POINTER;
        assert_eq!(m.run(XPACI), None);
        assert_eq!(
            m.cpu.x[0],
            POINTER & ((1 << bottom) - 1),
            "{name}: TG0 {tg0:#b}, T0SZ {t0sz}, DS {ds:#x}"
        );
    }
}

/// A stage 2 walk starts at the level `VTCR_EL2.SL0` gives for its
/// granule, which may index up to 16 concatenated tables; an encoding
/// `AArch64_S2InvalidSL` refuses (with the 4 KiB granule 0b11 without
/// FEAT_TTST, with the 16 KiB granule 0b11 without FEAT_LPA2's `DS`, with
/// the 64 KiB granule 0b11), a start level that resolves no bit of the
/// input address or more than a stride and four, and a `T0SZ` outside
/// `AArch64_S2MinTxSZ` and `AArch64_MaxTxSZ` of the granule take a level 0
/// Translation fault at stage 2 for every access — here the instruction
/// fetch, taken to EL2 with `HPFAR_EL2` holding its IPA. The smallest
/// `T0SZ` is `64 - PAMax`, 20 for the Cortex-A57's 44 bits; where `PAMax`
/// is 52 bits (FEAT_LPA) it is 12 with the 64 KiB granule and 16 with the
/// others. Where the walk translates, a load reads the page it leads to,
/// up to a 52-bit IPA, and the next page faults at level 3 with
/// `HPFAR_EL2` holding its IPA.
#[test]
fn stage_2_starts_where_sl0_and_its_granule_say() {
    let no_lpa = Implementation::every_feature_but(&[Feature::Lpa]);
    let cases = [
        // (processor, TG0, page bits, T0SZ, SL0, start level or refused)
        ("v9.2", V9, TG0_4K, 12, 24, 0b01, Some(1)),
        ("v9.2", V9, TG0_4K, 12, 24, 0b10, Some(0)),
        ("v9.2", V9, TG0_4K, 12, 24, 0b00, None),
        ("v9.2", V9, TG0_4K, 12, 39, 0b11, Some(3)),
        ("v9.2", V9, TG0_4K, 12, 40, 0b11, Some(3)),
        ("v9.2", V9, TG0_4K, 12, 48, 0b11, Some(3)),
        ("v9.2", V9, TG0_4K, 12, 49, 0b11, None),
        ("v9.2", V9, TG0_4K, 12, 38, 0b11, None),
        ("v9.2", V9, TG0_4K, 12, 30, 0b00, Some(2)),
        ("v9.2", V9, TG0_4K, 12, 29, 0b00, None),
        ("v9.2", V9, TG0_4K, 12, 21, 0b01, Some(1)),
        ("v9.2", V9, TG0_4K, 12, 20, 0b01, None),
        ("v9.2", V9, TG0_4K, 12, 19, 0b10, Some(0)),
        ("v9.2", V9, TG0_4K, 12, 16, 0b10, Some(0)),
        ("v9.2", V9, TG0_4K, 12, 15, 0b10, None),
        ("v9.2", V9, TG0_4K, 12, 12, 0b10, None),
        ("v9.2", V9, TG0_16K, 14, 24, 0b10, Some(1)),
        ("v9.2", V9, TG0_16K, 14, 24, 0b01, Some(2)),
        ("v9.2", V9, TG0_16K, 14, 23, 0b01, None),
        ("v9.2", V9, TG0_16K, 14, 24, 0b00, None),
        ("v9.2", V9, TG0_16K, 14, 24, 0b11, None),
        ("v9.2", V9, TG0_16K, 14, 16, 0b11, None),
        ("v9.2", V9, TG0_16K, 14, 16, 0b10, Some(1)),
        ("v9.2", V9, TG0_16K, 14, 15, 0b10, None),
        ("v9.2", V9, TG0_16K, 14, 48, 0b00, Some(3)),
        ("v9.2", V9, TG0_16K, 14, 35, 0b00, Some(3)),
        ("v9.2", V9, TG0_16K, 14, 34, 0b00, None),
        ("v9.2", V9, TG0_64K, 16, 24, 0b01, Some(2)),
        ("v9.2", V9, TG0_64K, 16, 24, 0b00, None),
        ("v9.2", V9, TG0_64K, 16, 24, 0b10, None),
        ("v9.2", V9, TG0_64K, 16, 24, 0b11, None),
        ("v9.2", V9, TG0_64K, 16, 20, 0b10, Some(1)),
        ("v9.2", V9, TG0_64K, 16, 47, 0b00, Some(3)),
        ("v9.2", V9, TG0_64K, 16, 48, 0b00, None),
        ("v9.2", V9, TG0_64K, 16, 31, 0b00, Some(3)),
        ("v9.2", V9, TG0_64K, 16, 30, 0b00, None),
        ("v9.2", V9, TG0_64K, 16, 12, 0b10, Some(1)),
        ("v9.2", V9, TG0_64K, 16, 11, 0b10, None),
        ("no LPA", no_lpa, TG0_64K, 16, 20, 0b10, Some(1)),
        ("no LPA", no_lpa, TG0_64K, 16, 19, 0b10, None),
        ("A57", A57, TG0_4K, 12, 20, 0b10, Some(0)),
        ("A57", A57, TG0_4K, 12, 19, 0b10, None),
        ("A57", A57, TG0_4K, 12, 39, 0b11, None),
        ("A57", A57, TG0_4K, 12, 39, 0b00, Some(2)),
        ("A57", A57, TG0_4K, 12, 39, 0b01, None),
        ("A57", A57, TG0_4K, 12, 40, 0b01, None),
        ("A57", A57, TG0_16K, 12, 24, 0b01, Some(1)),
        ("A57", A57, TG0_RESERVED, 12, 24, 0b01, Some(1)),
        ("A57", A57, TG0_64K, 12, 24, 0b01, Some(1)),
    ];
    for (name, imp, tg0, page_bits, t0sz, sl0, start) in cases {
        let case = std::format!("{name}: TG0 {tg0:#b}, T0SZ {t0sz}, SL0 {sl0:#b}");
        let vtcr = t0sz | (sl0 << SL0) | (tg0 << TG0);
        stage_2_starts(&case, imp, page_bits, vtcr, start);
    }
}

/// That a stage 2 walk under `VTCR_EL2` `vtcr` (`PS` 44 bits added), with
/// the granule of `2^page_bits`-byte pages its `TG0` names, on a processor
/// implementing `imp`, starts at level `start`, or for `None` that it is
/// refused, every access a level 0 Translation fault at stage 2 — here the
/// instruction fetch, taken to EL2 with `HPFAR_EL2` holding its IPA. Where
/// the walk translates, a load reads the page it leads to, and the next
/// page faults at level 3 with `HPFAR_EL2` holding its IPA.
fn stage_2_starts(case: &str, imp: Implementation, page_bits: u32, vtcr: u64, start: Option<i32>) {
    let mut m = Machine::new(imp);
    let ia_bits = 64 - (vtcr & 0x3F) as u32;
    let Some(start) = start else {
        // No walk is made. Where `SL0` names a level the input address
        // fits, tables from it translate the fetch, so the refusal is what
        // faults; elsewhere they are empty memory, which a walk from a
        // level other than 0 faults at that level.
        let root = match sl0_level(page_bits, (vtcr >> SL0) & 3) {
            Some(level) if fits_level(page_bits, ia_bits, level) => {
                let tables = m.tables(page_bits, ia_bits, level);
                let code_page = CODE & !(tables.page() - 1);
                m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
                tables.root
            }
            Some(_) | None => TABLES,
        };
        m.stage2(vtcr, root);
        m.cpu.el2_state.regs.hpfar = 0;
        assert_eq!(
            m.run(LDR),
            Some(Taken {
                to: El::El2,
                ec: EC_INSTRUCTION_ABORT_LOWER_EL,
                fsc: translation(0),
            }),
            "{case}"
        );
        assert_eq!(m.cpu.el2().hpfar, (CODE >> 12) << 4, "{case}: HPFAR_EL2");
        return;
    };
    let tables = m.tables(page_bits, ia_bits, start);
    let code_page = CODE & !(tables.page() - 1);
    m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
    m.stage2(vtcr, tables.root);
    let ipa = PATTERN & ((1 << tables.ia_bits) - 1);
    assert_ne!(ipa / tables.page(), CODE / tables.page(), "{case}");
    let offset = ipa & (tables.page() - 1);
    m.map(tables, ipa, 3, s2_leaf(DATA, TABLE));
    m.ram.put64(DATA + offset, marker(ipa));
    assert_eq!(m.load(ipa), Ok(marker(ipa)), "{case}: load");
    let neighbour = ipa ^ tables.page();
    if neighbour / tables.page() != CODE / tables.page() {
        assert_eq!(
            m.load(neighbour),
            Err(Taken {
                to: El::El2,
                ec: EC_DATA_ABORT_LOWER_EL,
                fsc: translation(3),
            }),
            "{case}: the next page"
        );
        assert_eq!(
            m.cpu.el2().hpfar,
            (neighbour >> 12) << 4,
            "{case}: HPFAR_EL2"
        );
    }
}

/// The level `SL0` `sl0` names for the granule of `2^page_bits`-byte pages,
/// whether or not a processor accepts it (`AArch64_S2StartLevel` without
/// FEAT_LPA2's `SL2`): the level a walk would start at. With the 64 KiB
/// granule 0b11 names none.
const fn sl0_level(page_bits: u32, sl0: u64) -> Option<i32> {
    match (page_bits, sl0) {
        (12, 0b00) => Some(2),
        (12, 0b01) => Some(1),
        (12, 0b10) => Some(0),
        (12, _) => Some(3),
        (_, 0b00) => Some(3),
        (_, 0b01) => Some(2),
        (_, 0b10) => Some(1),
        (14, _) => Some(0),
        _ => None,
    }
}

/// Whether a stage 2 walk of an `ia_bits`-bit input address with the
/// granule of `2^page_bits`-byte pages can start at `level`
/// (`AArch64_S2InconsistentSL`, negated): its table resolves at least one
/// bit and at most a stride and four more, 16 concatenated tables.
const fn fits_level(page_bits: u32, ia_bits: u32, level: i32) -> bool {
    let stride = page_bits - 3;
    let lsb = page_bits + stride * (3 - level) as u32;
    ia_bits > lsb && ia_bits <= lsb + stride + 4
}

/// The levels, from 3 down, a walk with the granule of `2^page_bits`-byte
/// pages starting at `start` can end at on the Armv9.2 model: at or below
/// the start, at a page at level 3 or a block at level 2 with every
/// granule, at a block at level 1 with the 4 KiB granule, and with the 64
/// KiB one where `PAMax` is 52 bits, as it is there (FEAT_LPA), and never
/// at level 0 (`AArch64_DecodeDescriptorType` without FEAT_LPA2's `DS`).
const fn leaf_levels(page_bits: u32, start: i32) -> [bool; 4] {
    let level_1 = page_bits == 12 || (page_bits == 16 && id::pa_bits(V9.features) >= 52);
    [true, start <= 2, start <= 1 && level_1, false]
}

/// A snapshot's TLB entry is refused at a level no stage 1 walk the
/// context it was made in describes ends at — above its start level, or
/// where its granule has no leaf — in the EL1&0 regime (`TCR_EL1`) and the
/// EL2 regime (`TCR_EL2`) alike.
#[test]
fn a_snapshot_entry_holds_a_level_its_stage_1_walk_ends_at() {
    let cases = [
        // (TG0, page bits, T0SZ, start level)
        (TG0_4K, 12, 25, 1),
        (TG0_4K, 12, 34, 2),
        (TG0_4K, 12, 48, 3),
        (TG0_16K, 14, 25, 1),
        (TG0_64K, 16, 16, 1),
        (TG0_64K, 16, 25, 2),
        (TG0_64K, 16, 47, 3),
    ];
    for (tg0, page_bits, t0sz, start) in cases {
        let va = PATTERN & ((1 << (64 - t0sz)) - 1);
        let regimes = [
            ("EL1&0", lower_range(V9, tg0, page_bits, t0sz, start)),
            ("EL2", el2_range(V9, tg0, page_bits, t0sz, start)),
        ];
        for (regime, (mut m, tables)) in regimes {
            let case = std::format!("{regime}, TG0 {tg0:#b}, T0SZ {t0sz}");
            m.map(tables, va, 3, s1_leaf(DATA, TABLE));
            let offset = va & ((1 << page_bits) - 1);
            m.ram.put64(DATA + offset, marker(va));
            assert_eq!(m.load(va), Ok(marker(va)), "{case}");
            let saved = m.saved_tlb();
            let at = saved.entry_at(va) + ENTRY_LEVEL;
            assert_eq!(
                [3, 2, 1, 0].map(|level| !m.refuses_level(&saved, at, level)),
                leaf_levels(page_bits, start),
                "{case}: levels 3 to 0"
            );
        }
    }
}

/// Stage 2 mapping every IPA of `m`'s RAM to itself under `VTCR_EL2`
/// `vtcr` (`PS` 44 bits added) with tables of `2^page_bits`-byte pages for
/// `ia_bits`-bit IPAs whose walk starts at `start`, by blocks at level 2 or
/// by pages where the walk starts at level 3.
fn identity_stage2(m: &mut Machine, vtcr: u64, page_bits: u32, ia_bits: u32, start: i32) {
    let tables = m.tables(page_bits, ia_bits, start);
    let level = start.max(2);
    let kind = if level == 3 { TABLE } else { BLOCK };
    let leaf_bytes = 1usize << (page_bits + (page_bits - 3) * (3 - level) as u32);
    for ipa in (0..m.ram.0.len() as u64).step_by(leaf_bytes) {
        m.map(tables, ipa, level, s2_leaf(ipa, kind));
    }
    m.stage2(vtcr, tables.root);
}

/// A snapshot's stage 2 leaf is refused at a level no stage 2 walk the
/// context it was made in describes ends at — above the start level `SL0`
/// gives, or where `VTCR_EL2.TG0`'s granule has no leaf — in an entry and
/// in a kept translation of an IPA alike: with stage 1 mapping by 4 KiB
/// pages, an EL1 load's entry and every IPA its walk read.
#[test]
fn a_snapshot_entry_holds_a_level_its_stage_2_walk_ends_at() {
    let cases = [
        // (TG0, page bits, T0SZ, SL0, start level)
        (TG0_4K, 12, 24, 0b01, 1),
        (TG0_4K, 12, 32, 0b00, 2),
        (TG0_4K, 12, 40, 0b11, 3),
        (TG0_16K, 14, 24, 0b01, 2),
        (TG0_64K, 16, 24, 0b01, 2),
    ];
    for (tg0, page_bits, t0sz, sl0, start) in cases {
        let case = std::format!("TG0 {tg0:#b}, T0SZ {t0sz}, SL0 {sl0:#b}");
        let mut m = Machine::new(V9);
        let vtcr = t0sz | (sl0 << SL0) | (tg0 << TG0);
        identity_stage2(&mut m, vtcr, page_bits, 64 - t0sz as u32, start);
        let stage_1 = m.tables(12, 39, 1);
        let va = PATTERN & ((1 << 39) - 1);
        m.map(stage_1, CODE, 3, s1_leaf(CODE, TABLE));
        m.map(stage_1, va, 3, s1_leaf(DATA, TABLE));
        m.stage1(25 | (TG0_4K << TG0) | EPD1, stage_1.root, 0);
        m.ram.put64(DATA + (va & 0xFFF), marker(va));
        assert_eq!(m.load(va), Ok(marker(va)), "{case}");
        let saved = m.saved_tlb();
        assert!(saved.kept() > 0, "{case}: the walk's IPAs are kept");
        let entry = saved.entry_at(va) + ENTRY_S2_LEVEL;
        let kept = (0..saved.kept()).map(|i| saved.kept_at() + 4 + i * KEPT_BYTES + KEPT_LEVEL);
        for at in core::iter::once(entry).chain(kept) {
            assert_eq!(
                [3, 2, 1, 0].map(|level| !m.refuses_level(&saved, at, level)),
                leaf_levels(page_bits, start),
                "{case}: the stage 2 level at byte {at}, levels 3 to 0"
            );
        }
    }
}

/// `saved` with the doubleword at `at` — an entry's page number or a kept
/// translation's IPA page number, with its tag — XORed with `bits`.
fn with_page_flipped(saved: &SavedTlb, at: usize, bits: u64) -> Vec<u8> {
    let mut image = saved.0.clone();
    let mut page = [0; 8];
    page.copy_from_slice(&image[at..at + 8]);
    let page = u64::from_le_bytes(page) ^ bits;
    image[at..at + 8].copy_from_slice(&page.to_le_bytes());
    image
}

/// A snapshot's TLB entry, or kept stage 2 translation, for an address its
/// context's walk does not translate is refused, each restoring at its own
/// address: an entry stage 1's walk made of a virtual address moved out of
/// the 39-bit range `TCR_EL1.T0SZ` or `TCR_EL2.T0SZ` gives
/// (`AArch64_VAIsOutOfRange`); and under stage 2 for 40-bit IPAs, an entry
/// made with stage 1 disabled and a kept translation, each moved to an IPA
/// beyond them (`AArch64_IPAIsOutOfRange`) but within the physical address
/// size; and a kept translation whose key has a bit above every IPA page
/// number and its tag, bit 63, set.
#[test]
fn a_snapshot_entry_for_an_address_its_walk_does_not_translate_is_refused() {
    // Bits 39 and 41 of an address, in a page number.
    const BIT_39: u64 = 1 << (39 - 12);
    const BIT_41: u64 = 1 << (41 - 12);
    let va = PATTERN & ((1 << 39) - 1);
    let regimes = [
        ("EL1&0", lower_range(V9, TG0_4K, 12, 25, 1)),
        ("EL2", el2_range(V9, TG0_4K, 12, 25, 1)),
    ];
    for (regime, (mut m, tables)) in regimes {
        m.map(tables, va, 3, s1_leaf(DATA, TABLE));
        assert_eq!(m.load(va), Ok(0), "{regime}");
        let saved = m.saved_tlb();
        assert!(!m.refuses(&saved.0), "{regime}: as saved");
        let moved = with_page_flipped(&saved, saved.entry_at(va), BIT_39);
        assert!(m.refuses(&moved), "{regime}: beyond the range");
    }
    let mut m = Machine::new(V9);
    identity_stage2(&mut m, 24 | (0b01 << SL0) | (TG0_4K << TG0), 12, 40, 1);
    let ipa = DATA + 0x468;
    assert_eq!(m.load(ipa), Ok(0), "stage 1 disabled");
    let saved = m.saved_tlb();
    assert!(saved.kept() > 0, "the IPAs are kept");
    assert!(!m.refuses(&saved.0), "as saved");
    let entry = saved.entry_at(ipa);
    assert!(
        m.refuses(&with_page_flipped(&saved, entry, BIT_41)),
        "an entry beyond stage 2's IPAs"
    );
    for kept in 0..saved.kept() {
        let at = saved.kept_at() + 4 + kept * KEPT_BYTES;
        assert!(
            m.refuses(&with_page_flipped(&saved, at, BIT_41)),
            "kept translation {kept} beyond stage 2's IPAs"
        );
        assert!(
            m.refuses(&with_page_flipped(&saved, at, 1 << 63)),
            "kept translation {kept} with bit 63 of its key set"
        );
    }
}

/// Each 4 KiB piece of stage 1's 64 KiB page reads the piece of the page
/// its address's bits 15:12 name, without stage 2 and under stage 2 of
/// each granule mapping the page to itself (`AArch64_S1LeafBase` and
/// `AArch64_S2LeafBase` keep the address's bits below the leaf's size).
#[test]
fn every_4k_piece_of_a_64k_page_reads_its_own_piece() {
    let cases = [
        // stage 2: TG0, page bits, start level for 40-bit IPAs
        None,
        Some((TG0_4K, 12, 1)),
        Some((TG0_16K, 14, 2)),
        Some((TG0_64K, 16, 2)),
    ];
    for stage2 in cases {
        let case = std::format!("stage 2 {stage2:?}");
        let (mut m, tables) = lower_range(V9, TG0_64K, 16, 25, 2);
        if let Some((tg0, page_bits, start)) = stage2 {
            let vtcr = 24 | (0b01 << SL0) | (tg0 << TG0);
            identity_stage2(&mut m, vtcr, page_bits, 40, start);
        }
        let page = PATTERN & ((1 << 39) - 1) & !0xFFFF;
        m.map(tables, page, 3, s1_leaf(DATA, TABLE));
        for piece in 0..16 {
            m.ram.put64(DATA + piece * 0x1000, marker(DATA + piece));
        }
        for piece in 0..16 {
            let read = m.load(page + piece * 0x1000);
            assert_eq!(read, Ok(marker(DATA + piece)), "{case}: piece {piece}");
        }
    }
}

/// With the 64 KiB granule where `PAMax` is 52 bits (FEAT_LPA), a table or
/// leaf descriptor's bits 15:12 are bits 51:48 of the address it gives
/// (`AArch64_NextTableBase`, `AArch64_S1LeafBase`, `AArch64_S2LeafBase`),
/// which must fit the output size `IPS` or `PS` gives: at 52 bits `AT`
/// reports a leaf's address in `PAR_EL1.PA` (51:12), and the next table is
/// read at a table descriptor's, a read nothing answers a synchronous
/// External abort on the walk at the next level; at 48 bits either is an
/// Address size fault at the level of the descriptor that holds it. Where
/// `PAMax` is less the bits are not the address.
#[test]
fn a_64k_descriptor_holds_bits_51_to_48_of_its_address_in_bits_15_to_12() {
    const HIGH: u64 = 0xA;
    let no_lpa = Implementation::every_feature_but(&[Feature::Lpa]);
    let va = PATTERN & ((1 << 39) - 1);
    let offset = va & 0xFFFF;
    let high_page = (HIGH << 48) | DATA | (offset & !0xFFF);
    let low_page = DATA | (offset & !0xFFF);
    let address_size_at = |level| PAR_F | (address_size(level) << 1);
    let table_read = s1_fault(external_on_walk(3));
    let table_too_wide = s1_fault(address_size(2));
    // (processor, IPS, PAR_EL1 for the leaf, the load through the table)
    let cases = [
        ("v9.2", V9, IPS_52, high_page, table_read),
        ("v9.2", V9, IPS_56, high_page, table_read),
        ("v9.2", V9, IPS_48, address_size_at(3), table_too_wide),
        ("no LPA", no_lpa, IPS_52, low_page, Ok(marker(va))),
    ];
    for (name, imp, ips, leaf_par, through_table) in cases {
        let (mut m, tables) = lower_range(imp, TG0_64K, 16, 25, 2);
        m.map(tables, va, 3, s1_leaf(DATA | (HIGH << 12), TABLE));
        m.cpu.sys.tcr_el1 |= ips;
        m.cpu.el10_context_changed();
        assert_eq!(par_result(m.at_s1e1r(va)), leaf_par, "{name}: leaf");

        let (mut m, mut tables) = lower_range(imp, TG0_64K, 16, 25, 2);
        tables.extra = HIGH << 12;
        m.map(tables, va, 3, s1_leaf(DATA, TABLE));
        m.ram.put64(DATA + offset, marker(va));
        m.cpu.sys.tcr_el1 |= ips;
        m.cpu.el10_context_changed();
        assert_eq!(m.load(va), through_table, "{name}: table");
    }

    // Stage 2's leaf, `AT S12E1R` at EL2 reporting it or its fault.
    let cases = [
        ("v9.2", V9, PS_52, high_page),
        ("v9.2", V9, PS_48, PAR_S | address_size_at(3)),
        ("no LPA", no_lpa, PS_52, low_page),
    ];
    for (name, imp, ps, want) in cases {
        let mut m = Machine::new(imp);
        // T0SZ 24, a 40-bit IPA, from level 2 (`SL0` 0b01).
        let tables = m.tables(16, 40, 2);
        let code_page = CODE & !0xFFFF;
        m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
        m.map(tables, va, 3, s2_leaf(DATA | (HIGH << 12), TABLE));
        m.stage2(24 | (0b01 << SL0) | (TG0_64K << TG0) | ps, tables.root);
        m.at(El::El2);
        m.cpu.x[1] = va;
        assert_eq!(m.run(AT_S12E1R), None, "{name}: AT S12E1R");
        assert_eq!(par_result(m.cpu.sys.par_el1), want, "{name}: stage 2 leaf");
    }

    // Stage 2's table descriptor: a load at EL1 through it, its fault taken
    // to EL2.
    let stage_2_fault = |fsc| {
        Err(Taken {
            to: El::El2,
            ec: EC_DATA_ABORT_LOWER_EL,
            fsc,
        })
    };
    let cases = [
        ("v9.2", V9, PS_52, stage_2_fault(external_on_walk(3))),
        ("v9.2", V9, PS_48, stage_2_fault(address_size(2))),
        ("no LPA", no_lpa, PS_52, Ok(marker(va))),
    ];
    for (name, imp, ps, want) in cases {
        let mut m = Machine::new(imp);
        let mut tables = m.tables(16, 40, 2);
        let code_page = CODE & !0xFFFF;
        m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
        tables.extra = HIGH << 12;
        m.map(tables, va, 3, s2_leaf(DATA, TABLE));
        m.ram.put64(DATA + offset, marker(va));
        m.stage2(24 | (0b01 << SL0) | (TG0_64K << TG0) | ps, tables.root);
        assert_eq!(m.load(va), want, "{name}: stage 2 table");
    }
}

/// What a test compares of `PAR_EL1` `par`: a translation's `PA` (51:12),
/// or a fault's `F`, `FST`, `S` and `PTW`.
const fn par_result(par: u64) -> u64 {
    if par & PAR_F == 0 {
        par & PAR_PA_52
    } else {
        par & (PAR_F | PAR_FST | PAR_S | PAR_PTW)
    }
}

/// Under a 52-bit `IPS` or `PS` (0b110) with the 64 KiB granule and
/// FEAT_LPA, a table base register holds bits 51:48 of the start table's
/// address in bits 5:2, the table at least 64-byte aligned
/// (`AArch64_S1TTBaseAddress`, `AArch64_S2TTBaseAddress`, the
/// IMPLEMENTATION DEFINED "BADDR expresses 52 bits for 64KB granule" FALSE
/// otherwise): the walk reads its start table there, a read nothing
/// answers a synchronous External abort on the walk at the start level,
/// `TTBR1_EL1`'s at stage 1 and `VTTBR_EL2`'s at stage 2. Under any other
/// size — 48 bits, or 56 (0b111), which `PAMax` makes 52 bits too — with
/// the 4 KiB or 16 KiB granule, or without FEAT_LPA, they are the base's
/// own bits 5:2, which every start table here, 64 bytes or more, is
/// aligned below: the walk reads where the rest of `BADDR` says.
#[test]
fn a_64k_table_base_holds_bits_51_to_48_in_bits_5_to_2_under_a_52_bit_ips() {
    let no_lpa = Implementation::every_feature_but(&[Feature::Lpa]);
    let offset = UPPER_VA & 0xFFFF;
    let translated = Ok(marker(UPPER_VA));
    let cases = [
        // (processor, IPS, TTBR1_EL1 bits 5:2, the load)
        ("v9.2", V9, IPS_52, 0b1010, s1_fault(external_on_walk(2))),
        ("v9.2", V9, IPS_52, 0b1000, s1_fault(external_on_walk(2))),
        ("v9.2", V9, IPS_52, 0b0000, translated),
        ("v9.2", V9, IPS_48, 0b1010, translated),
        ("v9.2", V9, IPS_56, 0b1010, translated),
        ("no LPA", no_lpa, IPS_52, 0b1010, translated),
    ];
    for (name, imp, ips, high, want) in cases {
        let (mut m, tables) = upper_range(imp, TG1_64K, 16, 2);
        m.map(tables, UPPER_VA, 3, s1_leaf(DATA, TABLE));
        m.ram.put64(DATA + offset, marker(UPPER_VA));
        m.cpu.sys.ttbr1_el1 = tables.root | (high << 2);
        m.cpu.sys.tcr_el1 |= ips;
        m.cpu.el10_context_changed();
        assert_eq!(m.load(UPPER_VA), want, "{name}: IPS {ips:#x}, {high:#06b}");
    }
    // The 4 KiB and 16 KiB granules' base is `BADDR` alone under a 52-bit
    // `IPS` too, its bits 5:2 below the alignment of these start tables.
    for (tg1, page_bits, start) in [(TG1_4K, 12, 1), (TG1_16K, 14, 1)] {
        let (mut m, tables) = upper_range(V9, tg1, page_bits, start);
        m.map(tables, UPPER_VA, 3, s1_leaf(DATA, TABLE));
        let offset = UPPER_VA & ((1 << page_bits) - 1);
        m.ram.put64(DATA + offset, marker(UPPER_VA));
        m.cpu.sys.ttbr1_el1 = tables.root | (0b1010 << 2);
        m.cpu.sys.tcr_el1 |= IPS_52;
        m.cpu.el10_context_changed();
        assert_eq!(m.load(UPPER_VA), translated, "TG1 {tg1:#b}");
    }

    // Stage 2's: `VTTBR_EL2` under `VTCR_EL2.PS`, read first for the
    // instruction fetch at EL1, whose fault is taken to EL2.
    let va = PATTERN & ((1 << 39) - 1);
    let cases = [
        // (processor, PS, VTTBR_EL2 bits 5:2, the fetch's fault)
        ("v9.2", V9, PS_52, 0b1010, Some(external_on_walk(2))),
        ("v9.2", V9, PS_52, 0b1000, Some(external_on_walk(2))),
        ("v9.2", V9, PS_52, 0b0000, None),
        ("v9.2", V9, PS_48, 0b1010, None),
        ("no LPA", no_lpa, PS_52, 0b1010, None),
    ];
    for (name, imp, ps, high, fault) in cases {
        let mut m = Machine::new(imp);
        let tables = m.tables(16, 40, 2);
        let code_page = CODE & !0xFFFF;
        m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
        m.map(tables, va, 3, s2_leaf(DATA, TABLE));
        m.ram.put64(DATA + (va & 0xFFFF), marker(va));
        let vtcr = 24 | (0b01 << SL0) | (TG0_64K << TG0) | ps;
        m.stage2(vtcr, tables.root | (high << 2));
        let want = match fault {
            Some(fsc) => Err(Taken {
                to: El::El2,
                ec: EC_INSTRUCTION_ABORT_LOWER_EL,
                fsc,
            }),
            None => Ok(marker(va)),
        };
        assert_eq!(m.load(va), want, "{name}: PS {ps:#x}, {high:#06b}");
    }
}

/// With FEAT_LVA and the 64 KiB granule an input address is up to 52 bits
/// (`T0SZ` or `T1SZ` 12, `AArch64_S1MinTxSZ`), its walk starting at level 1
/// with a table of 1024 entries: a page there is where a load reads and
/// `AT` puts it, in each range of the EL1&0 regime and in the EL2 regime's,
/// and an address one bit wider is out of range.
#[test]
fn a_64k_virtual_address_reaches_52_bits_with_feat_lva() {
    let va = PATTERN & ((1 << 52) - 1);
    assert_ne!(va >> 48, 0, "the address has bits above 48");
    let (mut m, tables) = lower_range(V9, TG0_64K, 16, 12, 1);
    walks_to_its_page(&mut m, tables, va, "TTBR0_EL1");

    let mut m = Machine::new(V9);
    let lower = m.tables(12, 39, 1);
    let code_page = CODE & !0xFFF;
    m.map(lower, code_page, 3, s1_leaf(code_page, TABLE));
    let upper = m.tables(16, 52, 1);
    let tcr = 25 | (TG0_4K << TG0) | (12 << T1SZ) | (TG1_64K << TG1);
    m.stage1(tcr, lower.root, upper.root);
    walks_to_its_page(&mut m, upper, (!0 << 52) | va, "TTBR1_EL1");

    let (mut m, tables) = el2_range(V9, TG0_64K, 16, 12, 1);
    m.map(tables, va, 3, s1_leaf(DATA, TABLE));
    m.ram.put64(DATA + (va & 0xFFFF), marker(va));
    assert_eq!(m.load(va), Ok(marker(va)), "EL2");
    assert_eq!(
        m.load(va | (1 << 52)),
        Err(Taken {
            to: El::El2,
            ec: EC_DATA_ABORT_SAME_EL,
            fsc: translation(0),
        }),
        "EL2: out of range"
    );
}

/// With stage 1 disabled an address is its own output address, which must
/// fit `PAMax` (`AArch64_S1DisabledOutput`): `AT S1E1R` reports the widest
/// address below it, 52 bits with FEAT_LPA and the Cortex-A57's 44, and an
/// Address size fault at level 0 for one bit more.
#[test]
fn an_untranslated_address_fits_pamax() {
    for (name, imp, pa_bits) in [("v9.2", V9, 52), ("A57", A57, 44)] {
        let mut m = Machine::new(imp);
        let widest = (1 << (pa_bits - 1)) | DATA;
        let par = m.at_s1e1r(widest) & (PAR_PA_52 | PAR_F);
        assert_eq!(par, widest, "{name}: {widest:#x}");
        let beyond = (1 << pa_bits) | DATA;
        let par = m.at_s1e1r(beyond) & (PAR_FST | PAR_F);
        assert_eq!(par, PAR_F | (address_size(0) << 1), "{name}: {beyond:#x}");
    }
}

/// `HPFAR_EL2.FIPA` holds IPA bits 51:12 in bits 43:4 with FEAT_LPA, and
/// bits 47:12 in bits 39:4 without, the bits above it RES0: an `MSR` keeps
/// those alone. (A stage 2 abort's report of a 52-bit IPA there is
/// `stage_2_starts_where_sl0_and_its_granule_say`'s.)
#[test]
fn hpfar_el2_holds_a_52_bit_ipa_with_feat_lpa() {
    let cases = [
        ("v9.2", V9, 0x0000_0FFF_FFFF_FFF0),
        ("A57", A57, 0x0000_00FF_FFFF_FFF0),
    ];
    for (name, imp, fipa) in cases {
        let mut m = Machine::new(imp);
        m.at(El::El2);
        m.cpu.x[0] = !0;
        assert_eq!(m.run(MSR_HPFAR), None, "{name}: MSR");
        m.cpu.x[0] = 0;
        assert_eq!(m.run(MRS_HPFAR), None, "{name}: MRS");
        assert_eq!(m.cpu.x[0], fipa, "{name}");
    }
}

// ----- FEAT_LPA2: 52-bit addresses with the 4 KiB and 16 KiB granules -----

/// [`lower_range`] under FEAT_LPA2's `DS` (`TCR_EL1.DS`) and a 52-bit
/// `IPS`.
fn ds_lower_range(
    imp: Implementation,
    tg0: u64,
    page_bits: u32,
    t0sz: u32,
    start: i32,
) -> (Machine, Tables) {
    let (mut m, tables) = lower_range(imp, tg0, page_bits, t0sz, start);
    m.cpu.sys.tcr_el1 |= DS_EL1 | IPS_52;
    m.cpu.el10_context_changed();
    (m, tables)
}

/// A stage 2 Data Abort taken to EL2 from EL1 with fault status `fsc`.
fn s2_fault(fsc: u64) -> Result<u64, Taken> {
    Err(Taken {
        to: El::El2,
        ec: EC_DATA_ABORT_LOWER_EL,
        fsc,
    })
}

/// Under FEAT_LPA2's `DS` the 4 KiB granule's walk of an input address
/// wider than 48 bits starts at level -1, whose table resolves its bits
/// from 48 up (`AArch64_S1StartLevel`), and the 16 KiB granule's at level 0
/// with up to five bits there: a page at level 3 is where a load reads and
/// `AT` puts the address, in each range of the EL1&0 regime, in the EL2
/// regime's (`TCR_EL2.DS`) and under stage 2, which translates the five
/// descriptors the walk reads and its output; an address one bit wider is
/// out of range. With `TCR_EL2.DS` clear the EL2 regime's `T0SZ` 12 is
/// below the least (`AArch64_S1MinTxSZ` 16), a level 0 Translation fault for
/// every address.
#[test]
fn ds_walks_52_bit_input_addresses_from_level_minus_1_with_the_4k_granule() {
    let cases = [
        // (TG0, page bits, T0SZ, start level)
        (TG0_4K, 12, 12, -1),
        (TG0_4K, 12, 15, -1),
        (TG0_4K, 12, 16, 0),
        (TG0_16K, 14, 12, 0),
        (TG0_16K, 14, 16, 0),
    ];
    for (tg0, page_bits, t0sz, start) in cases {
        let case = std::format!("TG0 {tg0:#b}, T0SZ {t0sz}");
        let (mut m, tables) = ds_lower_range(V9, tg0, page_bits, t0sz, start);
        let va = PATTERN & ((1 << tables.ia_bits) - 1);
        walks_to_its_page(&mut m, tables, va, &case);
    }

    let mut m = Machine::new(V9);
    let lower = m.tables(12, 39, 1);
    let code_page = CODE & !0xFFF;
    m.map(lower, code_page, 3, s1_leaf(code_page, TABLE));
    let upper = m.tables(12, 52, -1);
    let tcr = 25 | (TG0_4K << TG0) | (12 << T1SZ) | (TG1_4K << TG1) | DS_EL1 | IPS_52;
    m.stage1(tcr, lower.root, upper.root);
    walks_to_its_page(&mut m, upper, (!0 << 52) | PATTERN, "TTBR1_EL1");

    for ds in [DS_EL2, 0] {
        let (mut m, tables) = el2_range(V9, TG0_4K, 12, 12, -1);
        let features = m.cpu.imp.features;
        let tcr = m.cpu.el2().tcr | ds | PS_52;
        m.cpu.el2_state.regs.write(El2Reg::Tcr, features, 0, tcr);
        m.map(tables, PATTERN, 3, s1_leaf(DATA, TABLE));
        m.ram.put64(DATA + (PATTERN & 0xFFF), marker(PATTERN));
        if ds == 0 {
            // `T0SZ` 12 is below the least, which faults every address, the
            // instruction fetch's first.
            let fetch = Err(Taken {
                to: El::El2,
                ec: EC_INSTRUCTION_ABORT_SAME_EL,
                fsc: translation(0),
            });
            assert_eq!(m.load(PATTERN), fetch, "EL2 without DS");
            continue;
        }
        assert_eq!(m.load(PATTERN), Ok(marker(PATTERN)), "EL2");
        assert_eq!(
            m.load(PATTERN | (1 << 52)),
            Err(Taken {
                to: El::El2,
                ec: EC_DATA_ABORT_SAME_EL,
                fsc: translation(0),
            }),
            "EL2: out of range"
        );
    }

    let (mut m, tables) = ds_lower_range(V9, TG0_4K, 12, 12, -1);
    identity_stage2(&mut m, 24 | (0b01 << SL0) | (TG0_4K << TG0), 12, 40, 1);
    walks_to_its_page(&mut m, tables, PATTERN, "under stage 2");
}

/// A fault at level -1, which only a walk under FEAT_LPA2's `DS` with the 4
/// KiB granule reaches, has a status code of its own (`EncodeLDFSC`): a
/// Translation fault 0b101011 for an invalid descriptor there, a block
/// among them (`AArch64_BlockDescSupported`), an Address size fault
/// 0b101001 for a table address there wider than the output size, and a
/// synchronous External abort on the walk 0b010011 for a read of the level
/// -1 table that nothing answers: in `ESR_EL1` for a load, and the first
/// two in `PAR_EL1.FST` for `AT` (which takes an External abort as an
/// exception); `HCR_EL2.TEA` takes the External abort to EL2, as at any
/// other level. The table base register's own Address size fault is level
/// 0's (`AArch64_S1Walk`). At stage 2, starting at level -1 (`SL2:SL0`
/// 0b100), an invalid descriptor there is a stage 2 Translation fault
/// 0b101011 taken to EL2, `HPFAR_EL2` holding the IPA.
#[test]
fn a_fault_at_level_minus_1_has_its_own_status_code() {
    let (mut m, _) = ds_lower_range(V9, TG0_4K, 12, 12, -1);
    assert_eq!(m.load(PATTERN), s1_fault(translation(-1)), "invalid");
    assert_eq!(
        par_result(m.at_s1e1r(PATTERN)),
        PAR_F | (translation(-1) << 1),
        "invalid: AT"
    );

    let (mut m, tables) = ds_lower_range(V9, TG0_4K, 12, 12, -1);
    m.map(tables, PATTERN, -1, s1_leaf(0, BLOCK));
    assert_eq!(m.load(PATTERN), s1_fault(translation(-1)), "a block");

    // The level -1 table descriptor holds address bit 48 (descriptor bit
    // 48): beyond a 48-bit `IPS`, and beyond RAM under a 52-bit one, where
    // the read of the level 0 table it leads to faults.
    for (ips, fsc) in [(IPS_48, address_size(-1)), (IPS_52, external_on_walk(0))] {
        let (mut m, mut tables) = ds_lower_range(V9, TG0_4K, 12, 12, -1);
        m.cpu.sys.tcr_el1 = (m.cpu.sys.tcr_el1 & !IPS_56) | ips;
        m.cpu.el10_context_changed();
        tables.extra = 1 << 48;
        m.map(tables, PATTERN, 3, s1_leaf(DATA, TABLE));
        assert_eq!(m.load(PATTERN), s1_fault(fsc), "a table, IPS {ips:#x}");
        if ips == IPS_48 {
            let par = par_result(m.at_s1e1r(PATTERN));
            assert_eq!(par, PAR_F | (fsc << 1), "a table, IPS {ips:#x}: AT");
        }
    }

    // `TTBR1_EL1` names a level -1 table beyond RAM.
    let mut m = Machine::new(V9);
    let lower = m.tables(12, 39, 1);
    let code_page = CODE & !0xFFF;
    m.map(lower, code_page, 3, s1_leaf(code_page, TABLE));
    let tcr = 25 | (TG0_4K << TG0) | (12 << T1SZ) | (TG1_4K << TG1) | DS_EL1 | IPS_52;
    m.stage1(tcr, lower.root, 1 << 40);
    let upper_va = (!0 << 52) | PATTERN;
    assert_eq!(
        m.load(upper_va),
        s1_fault(external_on_walk(-1)),
        "the table read"
    );
    // `HCR_EL2.TEA` takes it to EL2, an External abort at level -1 as at
    // any other (`SyncExternalAbortTarget`, `IsExternalAbort`).
    m.cpu.el2_state.regs.hcr |= HCR_TEA;
    let to_el2 = Err(Taken {
        to: El::El2,
        ec: EC_DATA_ABORT_LOWER_EL,
        fsc: external_on_walk(-1),
    });
    assert_eq!(m.load(upper_va), to_el2, "the table read, HCR_EL2.TEA");
    m.cpu.el2_state.regs.hcr &= !HCR_TEA;
    // ... and one beyond a 48-bit `IPS`: level 0's fault.
    m.cpu.sys.tcr_el1 = (tcr & !IPS_56) | IPS_48;
    m.cpu.sys.ttbr1_el1 = 1 << 2;
    m.cpu.el10_context_changed();
    assert_eq!(
        m.load(upper_va),
        s1_fault(address_size(0)),
        "the table base"
    );

    // Stage 2, the fetch's IPA mapped from level -1 and `PATTERN`'s level -1
    // entry empty.
    let mut m = Machine::new(V9);
    let tables = m.tables(12, 52, -1);
    m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
    let vtcr = 12 | (TG0_4K << TG0) | VTCR_DS | VTCR_SL2;
    m.stage2(vtcr, tables.root);
    assert_eq!(m.load(PATTERN), s2_fault(translation(-1)), "stage 2");
    assert_eq!(m.cpu.el2().hpfar, (PATTERN >> 12) << 4, "HPFAR_EL2");
}

/// Under FEAT_LPA2's `DS` a table or leaf descriptor of the 4 KiB or 16 KiB
/// granule holds bits 51:50 of the address it gives in bits 9:8 and bits
/// 49:48 in bits 49:48 (`AArch64_NextTableBase`, `AArch64_S1LeafBase`,
/// `AArch64_S2LeafBase`), which must fit the output size `IPS` or `PS`
/// gives: at 52 bits `AT` reports a leaf's address in `PAR_EL1.PA`, and the
/// next table is read at a table descriptor's, a read nothing answers a
/// synchronous External abort on the walk at the next level; at 48 bits
/// either is an Address size fault at the level of the descriptor that
/// holds it. With `DS` clear, bits 9:8 are a leaf's shareability and bits
/// 49:48 RES0, neither part of the address.
#[test]
fn a_ds_descriptor_holds_bits_51_to_48_of_its_address_in_bits_9_8_and_49_48() {
    // Address bits 51:48 0b1001: bit 51 in descriptor bit 9, and bit 48.
    const HIGH: u64 = (0b10 << 8) | (1 << 48);
    const HIGH_PA: u64 = 0b1001 << 48;
    let va = PATTERN & ((1 << 39) - 1);
    // (DS, IPS, the leaf's address bits from 48 up or its Address size
    // fault, the fault of the load through the table)
    let cases = [
        (DS_EL1, IPS_52, Some(HIGH_PA), Some(external_on_walk(2))),
        (DS_EL1, IPS_48, None, Some(address_size(1))),
        (0, IPS_52, Some(0), None),
    ];
    for (tg0, page_bits) in [(TG0_4K, 12), (TG0_16K, 14)] {
        let offset = va & ((1 << page_bits) - 1);
        for (ds, ips, leaf, through_table) in cases {
            let case = std::format!("TG0 {tg0:#b}, DS {ds:#x}, IPS {ips:#x}");
            let (mut m, tables) = lower_range(V9, tg0, page_bits, 25, 1);
            m.map(tables, va, 3, s1_leaf(DATA | HIGH, TABLE));
            m.cpu.sys.tcr_el1 |= ds | ips;
            m.cpu.el10_context_changed();
            let leaf_par = match leaf {
                Some(high) => high | DATA | (offset & !0xFFF),
                None => PAR_F | (address_size(3) << 1),
            };
            assert_eq!(par_result(m.at_s1e1r(va)), leaf_par, "{case}: leaf");

            let (mut m, mut tables) = lower_range(V9, tg0, page_bits, 25, 1);
            tables.extra = HIGH;
            m.map(tables, va, 3, s1_leaf(DATA, TABLE));
            m.ram.put64(DATA + offset, marker(va));
            m.cpu.sys.tcr_el1 |= ds | ips;
            m.cpu.el10_context_changed();
            let want = match through_table {
                Some(fsc) => s1_fault(fsc),
                None => Ok(marker(va)),
            };
            assert_eq!(m.load(va), want, "{case}: table");
        }
    }
    // Bits 9:8 alone, address bit 50, are beyond a 48-bit `IPS` too.
    let (mut m, tables) = lower_range(V9, TG0_4K, 12, 25, 1);
    m.map(tables, va, 3, s1_leaf(DATA | (0b01 << 8), TABLE));
    m.cpu.sys.tcr_el1 |= DS_EL1 | IPS_48;
    m.cpu.el10_context_changed();
    let fault = PAR_F | (address_size(3) << 1);
    assert_eq!(par_result(m.at_s1e1r(va)), fault, "bits 9:8 alone");
    let offset = va & 0xFFF;
    let page = DATA;
    let address_size_at = |level| PAR_F | (address_size(level) << 1);

    // Stage 2's leaf, `AT S12E1R` at EL2 reporting it or its fault.
    let cases = [
        (VTCR_DS, PS_52, HIGH_PA | page),
        (VTCR_DS, PS_48, PAR_S | address_size_at(3)),
        (0, PS_52, page),
    ];
    for (ds, ps, want) in cases {
        let mut m = Machine::new(V9);
        // T0SZ 24, a 40-bit IPA, from level 1 (`SL0` 0b01).
        let tables = m.tables(12, 40, 1);
        let code_page = CODE & !0xFFF;
        m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
        m.map(tables, va, 3, s2_leaf(DATA | HIGH, TABLE));
        m.stage2(24 | (0b01 << SL0) | (TG0_4K << TG0) | ds | ps, tables.root);
        m.at(El::El2);
        m.cpu.x[1] = va;
        assert_eq!(m.run(AT_S12E1R), None, "DS {ds:#x}, PS {ps:#x}: AT S12E1R");
        let par = par_result(m.cpu.sys.par_el1);
        assert_eq!(par, want, "DS {ds:#x}, PS {ps:#x}: stage 2 leaf");
    }

    // Stage 2's table descriptor: a load at EL1 through it, its fault taken
    // to EL2.
    let cases = [
        (VTCR_DS, PS_52, s2_fault(external_on_walk(2))),
        (VTCR_DS, PS_48, s2_fault(address_size(1))),
        (0, PS_52, Ok(marker(va))),
    ];
    for (ds, ps, want) in cases {
        let mut m = Machine::new(V9);
        let mut tables = m.tables(12, 40, 1);
        let code_page = CODE & !0xFFF;
        m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
        tables.extra = HIGH;
        m.map(tables, va, 3, s2_leaf(DATA, TABLE));
        m.ram.put64(DATA + offset, marker(va));
        m.stage2(24 | (0b01 << SL0) | (TG0_4K << TG0) | ds | ps, tables.root);
        assert_eq!(m.load(va), want, "DS {ds:#x}, PS {ps:#x}: stage 2 table");
    }
}

/// Under FEAT_LPA2's `DS` a table base register holds bits 51:48 of the
/// start table's address in bits 5:2 whatever the output size, the table
/// at least 64-byte aligned (`AArch64_S1TTBaseAddress`,
/// `AArch64_S2TTBaseAddress`): the walk reads its start table there, a
/// read nothing answers a synchronous External abort on the walk at the
/// start level, and a base wider than the output size is an Address size
/// fault at level 0 — `TTBR1_EL1`'s at stage 1, and `VTTBR_EL2`'s at stage
/// 2 for the instruction fetch at EL1. With `DS` clear, or with the 64 KiB
/// granule, which `DS` leaves alone, under an output size below 52 bits
/// they are the base's own bits 5:2, below the alignment of these start
/// tables.
#[test]
fn a_ds_table_base_holds_bits_51_to_48_in_bits_5_to_2_whatever_the_ips() {
    let translated = Ok(marker(UPPER_VA));
    // (TG1, page bits, the start level for 39 bits)
    const K4: (u64, u32, i32) = (TG1_4K, 12, 1);
    const K64: (u64, u32, i32) = (TG1_64K, 16, 2);
    let cases = [
        // (granule, DS, IPS, TTBR1_EL1 bits 5:2, the load)
        (K4, DS_EL1, IPS_52, 0b1010, s1_fault(external_on_walk(1))),
        (K4, DS_EL1, IPS_52, 0b0001, s1_fault(external_on_walk(1))),
        (K4, DS_EL1, IPS_48, 0b0001, s1_fault(address_size(0))),
        (K4, DS_EL1, IPS_44, 0b0001, s1_fault(address_size(0))),
        (K4, DS_EL1, IPS_52, 0b0000, translated),
        (K4, 0, IPS_52, 0b1010, translated),
        (K64, DS_EL1, IPS_48, 0b0001, translated),
    ];
    for ((tg1, page_bits, start), ds, ips, high, want) in cases {
        let (mut m, tables) = upper_range(V9, tg1, page_bits, start);
        m.map(tables, UPPER_VA, 3, s1_leaf(DATA, TABLE));
        let offset = UPPER_VA & (tables.page() - 1);
        m.ram.put64(DATA + offset, marker(UPPER_VA));
        m.cpu.sys.ttbr1_el1 = tables.root | (high << 2);
        m.cpu.sys.tcr_el1 |= ds | ips;
        m.cpu.el10_context_changed();
        let case = std::format!("TG1 {tg1:#b}, DS {ds:#x}, IPS {ips:#x}, {high:#06b}");
        assert_eq!(m.load(UPPER_VA), want, "{case}");
    }

    let va = PATTERN & ((1 << 39) - 1);
    // (VTCR_EL2.TG0, page bits, the start level SL0 0b01 names)
    const S2_K4: (u64, u32, i32) = (TG0_4K, 12, 1);
    const S2_K64: (u64, u32, i32) = (TG0_64K, 16, 2);
    let cases = [
        // (granule, DS, PS, VTTBR_EL2 bits 5:2, the fetch's fault)
        (S2_K4, VTCR_DS, PS_52, 0b1010, Some(external_on_walk(1))),
        (S2_K4, VTCR_DS, PS_44, 0b0001, Some(address_size(0))),
        (S2_K4, VTCR_DS, PS_52, 0b0000, None),
        (S2_K4, 0, PS_52, 0b1010, None),
        (S2_K64, VTCR_DS, PS_48, 0b0001, None),
    ];
    for ((tg0, page_bits, start), ds, ps, high, fault) in cases {
        let mut m = Machine::new(V9);
        let tables = m.tables(page_bits, 40, start);
        let code_page = CODE & !(tables.page() - 1);
        m.map(tables, code_page, 3, s2_leaf(code_page, TABLE));
        m.map(tables, va, 3, s2_leaf(DATA, TABLE));
        m.ram.put64(DATA + (va & (tables.page() - 1)), marker(va));
        let vtcr = 24 | (0b01 << SL0) | (tg0 << TG0) | ds | ps;
        m.stage2(vtcr, tables.root | (high << 2));
        let want = match fault {
            Some(fsc) => Err(Taken {
                to: El::El2,
                ec: EC_INSTRUCTION_ABORT_LOWER_EL,
                fsc,
            }),
            None => Ok(marker(va)),
        };
        let case = std::format!("TG0 {tg0:#b}, DS {ds:#x}, PS {ps:#x}, {high:#06b}");
        assert_eq!(m.load(va), want, "{case}");
    }
}

/// Under FEAT_LPA2's `DS` the 4 KiB granule has blocks at level 0 (512
/// GiB) and the 16 KiB one at level 1 (64 GiB) (`AArch64_BlockDescSupported`),
/// at stage 1 and at stage 2; with `DS` clear, a block descriptor there is
/// invalid, a Translation fault at its level.
#[test]
fn ds_makes_blocks_at_4k_level_0_and_16k_level_1() {
    const BLOCK_VA: u64 = 0x8000_0000_0000;
    const OFFSET: u64 = 0x3_0468;
    let cases = [
        // (TG0, page bits, start level, block level, DS, a block there)
        (TG0_4K, 12, 0, 0, DS_EL1, true),
        (TG0_4K, 12, 0, 0, 0, false),
        (TG0_16K, 14, 0, 1, DS_EL1, true),
        (TG0_16K, 14, 0, 1, 0, false),
        (TG0_16K, 14, 0, 0, DS_EL1, false),
    ];
    for (tg0, page_bits, start, level, ds, block) in cases {
        let case = std::format!("TG0 {tg0:#b}, level {level}, DS {ds:#x}");
        let (mut m, tables) = lower_range(V9, tg0, page_bits, 16, start);
        m.cpu.sys.tcr_el1 |= ds;
        m.cpu.el10_context_changed();
        m.map(tables, BLOCK_VA, level, s1_leaf(0, BLOCK));
        m.ram.put64(OFFSET, marker(OFFSET));
        let want = if block {
            Ok(marker(OFFSET))
        } else {
            s1_translation_fault(level)
        };
        assert_eq!(m.load(BLOCK_VA + OFFSET), want, "{case}");
    }

    // Stage 2's: a block mapping the low IPAs to themselves, under a walk of
    // 48-bit IPAs from its level (`SL0` 0b10). A load at EL1 reads through
    // it with `DS`; without, the fetch takes a Translation fault at its
    // level to EL2.
    for (tg0, page_bits, level) in [(TG0_4K, 12, 0), (TG0_16K, 14, 1)] {
        for ds in [VTCR_DS, 0] {
            let mut m = Machine::new(V9);
            let tables = m.tables(page_bits, 48, level);
            m.map(tables, 0, level, s2_leaf(0, BLOCK));
            m.stage2(16 | (0b10 << SL0) | (tg0 << TG0) | ds, tables.root);
            m.ram.put64(DATA, marker(DATA));
            let want = if ds == 0 {
                Err(Taken {
                    to: El::El2,
                    ec: EC_INSTRUCTION_ABORT_LOWER_EL,
                    fsc: translation(level),
                })
            } else {
                Ok(marker(DATA))
            };
            let case = std::format!("stage 2, TG0 {tg0:#b}, DS {ds:#x}");
            assert_eq!(m.load(DATA), want, "{case}");
        }
    }
}

/// Under FEAT_LPA2's `DS` a stage 2 walk with the 4 KiB granule starts at
/// level -1 for `SL2:SL0` 0b100 where `PAMax` is 52 bits, its table
/// resolving the IPA's bits from 48 up, and 0b101, 0b110 and 0b111 name no
/// level (`AArch64_S2StartLevel`, `AArch64_S2InvalidSL`); `SL2` counts with
/// `DS` alone, and on a processor without FEAT_LPA2 neither is a field. A
/// start at level 0 with the 4 KiB granule takes 16 concatenated tables for
/// a 52-bit IPA (`AArch64_S2InconsistentSL`), and the 16 KiB granule starts
/// at level 0 for `SL0` 0b11 (`DS` and a 52-bit `PAMax`); the least `T0SZ`
/// with `DS` is 12 (`AArch64_S2MinTxSZ`).
#[test]
fn stage_2_starts_at_level_minus_1_for_sl2_under_ds() {
    let no_lpa2 = Implementation::every_feature_but(&[Feature::Lpa2]);
    let ds_sl2 = VTCR_DS | VTCR_SL2;
    let cases = [
        // (processor, TG0, page bits, T0SZ, DS and SL2, SL0, start level)
        ("v9.2", V9, TG0_4K, 12, 12, ds_sl2, 0b00, Some(-1)),
        ("v9.2", V9, TG0_4K, 12, 15, ds_sl2, 0b00, Some(-1)),
        ("v9.2", V9, TG0_4K, 12, 16, ds_sl2, 0b00, None),
        ("v9.2", V9, TG0_4K, 12, 12, ds_sl2, 0b01, None),
        ("v9.2", V9, TG0_4K, 12, 12, ds_sl2, 0b10, None),
        ("v9.2", V9, TG0_4K, 12, 12, ds_sl2, 0b11, None),
        ("v9.2", V9, TG0_4K, 12, 12, VTCR_DS, 0b10, Some(0)),
        ("v9.2", V9, TG0_4K, 12, 11, VTCR_DS, 0b10, None),
        ("v9.2", V9, TG0_4K, 12, 12, 0, 0b10, None),
        ("v9.2", V9, TG0_4K, 12, 30, VTCR_SL2, 0b00, Some(2)),
        ("v9.2", V9, TG0_16K, 14, 12, VTCR_DS, 0b11, Some(0)),
        ("v9.2", V9, TG0_16K, 14, 16, VTCR_DS, 0b11, Some(0)),
        ("v9.2", V9, TG0_16K, 14, 12, ds_sl2, 0b11, Some(0)),
        ("v9.2", V9, TG0_16K, 14, 11, VTCR_DS, 0b11, None),
        ("no LPA2", no_lpa2, TG0_4K, 12, 30, ds_sl2, 0b00, Some(2)),
        ("no LPA2", no_lpa2, TG0_16K, 14, 16, VTCR_DS, 0b11, None),
    ];
    for (name, imp, tg0, page_bits, t0sz, extra, sl0, start) in cases {
        let case = std::format!("{name}: TG0 {tg0:#b}, T0SZ {t0sz}, {extra:#x}, SL0 {sl0:#b}");
        let vtcr = t0sz | (sl0 << SL0) | (tg0 << TG0) | extra;
        stage_2_starts(&case, imp, page_bits, vtcr, start);
    }
}

/// Under FEAT_LPA2's `DS` a leaf's shareability is its walk's, `TCR_EL1.SH0`
/// or `SH1` for its range, `TCR_EL2.SH0` in the EL2 regime and
/// `VTCR_EL2.SH0` at stage 2, its bits 9:8 being address bits
/// (`AArch64_S1NextWalkStateLeaf`, `AArch64_S2NextWalkStateLeaf`); with `DS`
/// clear, with the 64 KiB granule, which `DS` leaves alone, or on a
/// processor without FEAT_LPA2, it is the leaf's own `SH`. `AT` reports it in
/// `PAR_EL1.SH`, for Normal Write-Back memory with the data cache on
/// (`SCTLR_ELx.C`, without which every access is Non-cacheable and so Outer
/// Shareable) as it is, and under stage 2 the wider of the two stages'
/// (`S2CombineS1Shareability`).
#[test]
fn a_ds_leaf_takes_its_shareability_from_the_tcr() {
    let no_lpa2 = Implementation::every_feature_but(&[Feature::Lpa2]);
    let va = PATTERN & ((1 << 39) - 1);
    let sh = |par: u64| (par >> PAR_SH) & 3;
    let cases = [
        // (processor, TG0, page bits, start level, DS, the leaf's SH, PAR_EL1.SH)
        ("v9.2", V9, TG0_4K, 12, 1, DS_EL1, 0, OUTER),
        ("v9.2", V9, TG0_16K, 14, 1, DS_EL1, 0, OUTER),
        ("v9.2", V9, TG0_4K, 12, 1, 0, INNER, INNER),
        ("v9.2", V9, TG0_64K, 16, 2, DS_EL1, INNER, INNER),
        ("no LPA2", no_lpa2, TG0_4K, 12, 1, DS_EL1, INNER, INNER),
    ];
    for (name, imp, tg0, page_bits, start, ds, leaf_sh, want) in cases {
        let (mut m, tables) = lower_range(imp, tg0, page_bits, 25, start);
        m.map(tables, va, 3, s1_leaf(DATA, TABLE) | (leaf_sh << 8));
        m.cpu.sys.tcr_el1 |= ds | (OUTER << SH0);
        m.cpu.sys.sctlr_el1 |= sctlr::C;
        m.cpu.el10_context_changed();
        let case = std::format!("{name}: TG0 {tg0:#b}, DS {ds:#x}");
        assert_eq!(sh(m.at_s1e1r(va)), want, "{case}");
    }

    let (mut m, tables) = upper_range(V9, TG1_4K, 12, 1);
    m.map(tables, UPPER_VA, 3, s1_leaf(DATA, TABLE));
    m.cpu.sys.tcr_el1 |= DS_EL1 | (OUTER << SH0) | (INNER << SH1);
    m.cpu.sys.sctlr_el1 |= sctlr::C;
    m.cpu.el10_context_changed();
    assert_eq!(sh(m.at_s1e1r(UPPER_VA)), INNER, "TTBR1_EL1: SH1");

    let (mut m, tables) = el2_range(V9, TG0_4K, 12, 25, 1);
    m.map(tables, va, 3, s1_leaf(DATA, TABLE));
    let features = m.cpu.imp.features;
    let tcr = m.cpu.el2().tcr | DS_EL2 | (OUTER << SH0);
    m.cpu.el2_state.regs.write(El2Reg::Tcr, features, 0, tcr);
    m.cpu.el2_state.regs.sctlr |= sctlr::C;
    m.cpu.x[1] = va;
    assert_eq!(m.run(AT_S1E2R), None, "AT S1E2R");
    assert_eq!(sh(m.cpu.sys.par_el1), OUTER, "EL2");

    // Stage 1's leaf Non-shareable, stage 2's from `VTCR_EL2.SH0` under `DS`
    // and its own bits 9:8, 0, without.
    for (ds, want) in [(VTCR_DS, INNER), (0, 0)] {
        let (mut m, tables) = lower_range(V9, TG0_4K, 12, 25, 1);
        m.map(tables, va, 3, s1_leaf(DATA, TABLE));
        m.cpu.sys.sctlr_el1 |= sctlr::C;
        let vtcr = 24 | (0b01 << SL0) | (TG0_4K << TG0) | ds | (INNER << SH0);
        identity_stage2(&mut m, vtcr, 12, 40, 1);
        m.at(El::El2);
        m.cpu.x[1] = va;
        assert_eq!(m.run(AT_S12E1R), None, "AT S12E1R");
        assert_eq!(sh(m.cpu.sys.par_el1), want, "stage 2, DS {ds:#x}");
    }
}
