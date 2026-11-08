//! TLB maintenance as a guest sees it (R9): a translation the TLB holds
//! stays in use after its descriptor changes, as the architecture lets a
//! TLB keep it (Arm ARM D8.17 RFFWJK), until a TLBI that names it — by
//! address, ASID, VMID, regime, level or range — removes it, and a TLBI
//! leaves every translation it does not name. Each test warms the TLB with
//! loads, rewrites descriptors with no maintenance, and reads which
//! physical page each load reaches before and after a TLBI: the old one
//! where the TLB still holds the translation, the new one where the load
//! walked again.
//!
//! A block, and a 64 KiB page, is held whole: after its descriptor changes,
//! a page of it the processor never touched still reaches the old output,
//! where a TLB holding 4 KiB pieces would have walked again; and a TLBI by
//! any address in it removes all of it (Arm ARM D8.17 RLGSCG). Stage 2's
//! translations of IPAs, which stage 1's walks read their tables through,
//! are held whole the same way.
//!
//! The rules are the Arm ARM's pseudocode: `TLBIMatch`, `TLBIRange`,
//! `ResTLBITTL`, `ResTLBIRTTL`, `AArch64_TLBI_VA`, `AArch64_TLBI_VAA`,
//! `AArch64_TLBI_ASID`, `AArch64_TLBI_VMALL`, `AArch64_TLBI_VMALLS12`,
//! `AArch64_TLBI_IPAS2`, `AArch64_TLBI_ALL`, `AArch64_TLBContextEL10` and
//! `AArch64_S1NextWalkStateLeaf` (`nG`). The words are `SYS #op1, C8, Cm,
//! #op2, Xt`, as GNU as assembles each `TLBI` alias, and `MSR`/`LDR` as it
//! assembles them.

use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::el2::{El2Reg, HCR_VM};
use super::el2_tests::SavedTlb;
use super::features::{Feature, Features, Implementation};
use super::mmu::saved_layout::{ENTRY_LEVEL, NOT_WALKED};
use super::mmu::{El2RegimeContext, TlbContext};
use super::{id, Cpu, CpuConfig, El, Levels, NoEl2, Pstate, Step, WithEl2};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const LDR: u32 = 0xF940_0022; // ldr x2, [x1]
const STR: u32 = 0xF900_0022; // str x2, [x1]
const MSR_VTTBR_EL2: u32 = 0xD51C_2100; // msr vttbr_el2, x0

/// `TLBI <op>, x1` (`x1` named even where the operation takes no operand,
/// which leaves it unread): `SYS #op1, C8, C<crm>, #op2, x1`.
const fn tlbi(op1: u32, crm: u32, op2: u32) -> u32 {
    0xD508_0000 | (op1 << 16) | (8 << 12) | (crm << 8) | (op2 << 5) | 1
}

const VMALLE1: u32 = tlbi(0, 7, 0);
const VAE1: u32 = tlbi(0, 7, 1);
const ASIDE1: u32 = tlbi(0, 7, 2);
const VAAE1: u32 = tlbi(0, 7, 3);
const VALE1IS: u32 = tlbi(0, 3, 5);
const RVAE1: u32 = tlbi(0, 6, 1);
const IPAS2E1: u32 = tlbi(4, 4, 1);
const IPAS2E1IS: u32 = tlbi(4, 0, 1);
const VMALLS12E1: u32 = tlbi(4, 7, 6);
const ALLE1: u32 = tlbi(4, 7, 4);
const ALLE2: u32 = tlbi(4, 7, 0);
const VAE2: u32 = tlbi(4, 7, 1);

const RAM_BYTES: usize = 0x100_0000;
/// Where the instruction under test executes, identity mapped at every
/// stage, and the vector bases.
const CODE: u64 = 0x4_0000;
const VBAR_EL1: u64 = 0x8000;
const VBAR_EL2: u64 = 0xA000;

/// EL1&0's stage 1 tables (4 KiB granule, `T0SZ` 25: a walk from level 1):
/// `L1`'s first entry is the table `L2`, whose entries map VA 0 to itself
/// by a block (the code, the tables, the vectors), VA [`PAGES`] by the
/// pages of `L3`, VA [`BLOCK`] by a block, and VA [`MORE_PAGES`] by the
/// pages of `L3B`.
const L1: u64 = 0x1000;
const L2: u64 = 0x2000;
const L3: u64 = 0x3000;
const L3B: u64 = 0x4000;
/// The EL2 regime's tables, of the same shape as `L1` and `L2`.
const EL2_L1: u64 = 0x1_0000;
const EL2_L2: u64 = 0x1_1000;
/// Stage 2's tables (`VTCR_EL2` T0SZ 25, SL0 1: a walk from level 1):
/// `S2_L1`'s first entry is the table `S2_L2`, whose entries map each 2 MiB
/// of IPAs to itself by a block.
const S2_L1: u64 = 0x1_4000;
const S2_L2: u64 = 0x1_5000;

/// VAs of 4 KiB pages, the block, and more pages.
const PAGES: u64 = 0x20_0000;
const BLOCK: u64 = 0x40_0000;
const MORE_PAGES: u64 = 0x60_0000;
/// Where the pages of `PAGES` lead, and where their rewritten descriptors
/// lead.
const FRAMES: u64 = 0x60_0000;
const NEW_FRAMES: u64 = 0x70_0000;
/// Where the block leads, and where its rewritten descriptor leads.
const OLD_BLOCK: u64 = 0x80_0000;
const NEW_BLOCK: u64 = 0xA0_0000;

/// Descriptor bits: valid, table or page, block; the Access flag, Inner
/// Shareable, `nG`.
const TABLE: u64 = 0b11;
const PAGE: u64 = 0b11;
const BLOCK_DESC: u64 = 0b01;
const AF: u64 = 1 << 10;
const INNER_SHAREABLE: u64 = 3 << 8;
const NG: u64 = 1 << 11;
/// A stage 1 leaf's attributes: `AttrIndx` 0 (`MAIR` 0xFF, Normal
/// Write-Back), Inner Shareable, accessed.
const S1_LEAF: u64 = AF | INNER_SHAREABLE;
/// A stage 2 leaf's: `S2AP` read and write, `MemAttr` Normal Write-Back,
/// Inner Shareable, accessed.
const S2_LEAF: u64 = AF | INNER_SHAREABLE | (0b11 << 6) | (0b1111 << 2);

/// `TCR_EL1`: `T0SZ` 25, `EPD1`, `IPS` 40 bits, `TG0` 4 KiB.
const TCR_EL1: u64 = 25 | (1 << 23) | (0b010 << 32);
/// `TCR_EL1.AS`: 16-bit ASIDs.
const TCR_AS: u64 = 1 << 36;
/// `TCR_EL1.TG0` 64 KiB.
const TG0_64K: u64 = 0b01 << 14;
/// `VTCR_EL2`: `T0SZ` 25, `SL0` 1, `PS` 40 bits.
const VTCR: u64 = 25 | (1 << 6) | (0b010 << 16);
/// `TCR_EL2`: `T0SZ` 25, `PS` 40 bits.
const TCR_EL2: u64 = 25 | (0b010 << 16);

/// What a load finds at physical address `pa`: each test puts it there.
const fn marker(pa: u64) -> u64 {
    pa ^ 0x5A5A_0000_0000_5A5A
}

struct Ram(Vec<u8>);

impl PhysBus for Ram {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get_mut(a..a + n).ok_or(BusFault::Unmapped)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

impl Ram {
    fn put64(&mut self, at: u64, value: u64) {
        let a = at as usize;
        self.0[a..a + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn get64(&self, at: u64) -> u64 {
        let a = at as usize;
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.0[a..a + 8]);
        u64::from_le_bytes(bytes)
    }

    /// Put the marker of each of `pas` at it.
    fn mark(&mut self, pas: &[u64]) {
        for &pa in pas {
            self.put64(pa, marker(pa));
        }
    }
}

struct Machine<L: Levels> {
    cpu: Cpu<L>,
    ram: Ram,
}

impl<L: Levels> Machine<L> {
    /// A processor implementing `imp` at EL1, its EL1&0 regime's stage 1
    /// on over the tables above: [`PAGES`] page `i` leads to [`FRAMES`]
    /// page `i` (`nG` set from page 4 on), [`BLOCK`] to [`OLD_BLOCK`], and
    /// [`MORE_PAGES`] page `i` to [`FRAMES`] page `16 + i`.
    fn new(imp: Implementation) -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let mut cpu = Cpu::<L>::implementing(
            CpuConfig {
                ips,
                reset_pc: CODE,
            },
            Implementation {
                counter_hz: ips,
                ..imp
            },
        );
        cpu.sys.vbar_el1 = VBAR_EL1;
        if let Some(el2) = cpu.el2_mut() {
            el2.vbar = VBAR_EL2;
        }
        let mut m = Self {
            cpu,
            ram: Ram(std::vec![0; RAM_BYTES]),
        };
        m.ram.put64(L1, L2 | TABLE);
        m.ram.put64(L2, S1_LEAF | BLOCK_DESC);
        m.ram.put64(L2 + 8, L3 | TABLE);
        m.ram.put64(L2 + 16, OLD_BLOCK | S1_LEAF | BLOCK_DESC);
        m.ram.put64(L2 + 24, L3B | TABLE);
        for i in 0..16 {
            m.map_page(i, FRAMES + i * 0x1000);
            m.ram
                .put64(L3B + 8 * i, (FRAMES + (16 + i) * 0x1000) | S1_LEAF | PAGE);
        }
        m.cpu.sys.mair_el1 = 0xFF;
        m.cpu.sys.tcr_el1 = TCR_EL1;
        m.cpu.sys.ttbr0_el1 = L1;
        m.cpu.sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;
        m.at(El::El1);
        m
    }

    /// Point [`PAGES`] page `i` at `frame`, `nG` set from page 4 on, with
    /// no TLB maintenance.
    fn map_page(&mut self, i: u64, frame: u64) {
        let ng = if i >= 4 { NG } else { 0 };
        self.ram.put64(L3 + 8 * i, frame | S1_LEAF | ng | PAGE);
    }

    /// Point [`BLOCK`] at `output`, with no TLB maintenance.
    fn map_block(&mut self, output: u64) {
        self.ram.put64(L2 + 16, output | S1_LEAF | BLOCK_DESC);
    }

    /// Execute at `el` from [`CODE`], `SP_ELx` selected, every interrupt
    /// masked.
    fn at(&mut self, el: El) {
        self.cpu.pstate = Pstate {
            el,
            sp_sel: true,
            ..Pstate::RESET
        };
    }

    /// Step `insn` at [`CODE`] at the current EL, which takes no
    /// exception.
    fn run(&mut self, insn: u32) {
        let at = CODE as usize;
        self.ram.0[at..at + 4].copy_from_slice(&insn.to_le_bytes());
        self.cpu.pc = CODE;
        assert_eq!(
            self.cpu.step(&mut self.ram, &mut NoInstrumentation),
            Step::Retired,
            "{insn:#010x}"
        );
        assert_eq!(
            self.cpu.pc,
            CODE + 4,
            "{insn:#010x} took an exception: ESR_EL1 {:#x}, FAR_EL1 {:#x}",
            self.cpu.sys.esr_el1,
            self.cpu.sys.far_el1
        );
    }

    /// What a load from `va` at the current EL reads.
    fn load(&mut self, va: u64) -> u64 {
        self.cpu.x[1] = va;
        self.run(LDR);
        self.cpu.x[2]
    }

    /// Whether a load from `va` still reaches `old` (the TLB held its
    /// translation) or reaches `new` (it walked again); a panic for
    /// anything else.
    fn reaches(&mut self, va: u64, old: u64, new: u64) -> Reached {
        match self.load(va) {
            value if value == marker(old) => Reached::Old,
            value if value == marker(new) => Reached::New,
            value => panic!("a load from {va:#x} read {value:#x}"),
        }
    }

    /// `TLBI` `op` with operand `xt` at the current EL.
    fn tlbi(&mut self, op: u32, xt: u64) {
        self.cpu.x[1] = xt;
        self.run(op);
    }
}

/// Which output a load reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reached {
    /// The one before the descriptor changed: the TLB held it.
    Old,
    /// The one the changed descriptor gives: the load walked again.
    New,
}

use Reached::{New, Old};

/// The VA of [`PAGES`] page `i`, and the frames it leads to before and
/// after [`Machine::map_page`] moves it to [`NEW_FRAMES`].
const fn page(i: u64) -> u64 {
    PAGES + i * 0x1000
}
const fn frame(i: u64) -> u64 {
    FRAMES + i * 0x1000
}
const fn new_frame(i: u64) -> u64 {
    NEW_FRAMES + i * 0x1000
}

/// A machine with [`PAGES`] pages `0..pages` and the block's pages at the
/// offsets `offsets` loaded, then every one of their descriptors moved
/// (pages to [`NEW_FRAMES`], the block to [`NEW_BLOCK`]) with no
/// maintenance: the TLB holds the old translations.
fn warmed<L: Levels>(imp: Implementation, pages: u64, offsets: &[u64]) -> Machine<L> {
    let mut m = Machine::<L>::new(imp);
    for i in 0..16 {
        m.ram.mark(&[frame(i), new_frame(i)]);
    }
    for offset in (0..0x20_0000).step_by(0x1000) {
        m.ram.mark(&[OLD_BLOCK + offset, NEW_BLOCK + offset]);
    }
    for i in 0..pages {
        assert_eq!(m.load(page(i)), marker(frame(i)));
    }
    for &offset in offsets {
        assert_eq!(m.load(BLOCK + offset), marker(OLD_BLOCK + offset));
    }
    for i in 0..16 {
        m.map_page(i, new_frame(i));
    }
    m.map_block(NEW_BLOCK);
    m
}

/// Where page `i` of [`PAGES`] and the block at `offset` lead now.
fn page_reached<L: Levels>(m: &mut Machine<L>, i: u64) -> Reached {
    m.reaches(page(i), frame(i), new_frame(i))
}
fn block_reached<L: Levels>(m: &mut Machine<L>, offset: u64) -> Reached {
    m.reaches(BLOCK + offset, OLD_BLOCK + offset, NEW_BLOCK + offset)
}

/// A TLBI by VA removes the translation of its page and no other: the
/// pages beside it, and the block, keep the translations the TLB holds
/// (`TLBIOp_VA`'s address match); `VAAE1` does the same for every ASID,
/// and `VMALLE1` removes them all.
#[test]
fn a_tlbi_by_va_removes_its_page_and_keeps_every_other() {
    let mut m = warmed::<NoEl2>(Implementation::CORTEX_A57, 3, &[0x1000]);
    for i in 0..3 {
        assert_eq!(page_reached(&mut m, i), Old, "the TLB holds page {i}");
    }
    assert_eq!(block_reached(&mut m, 0x1000), Old);
    m.tlbi(VAE1, page(1) >> 12);
    assert_eq!(
        [0, 1, 2].map(|i| page_reached(&mut m, i)),
        [Old, New, Old],
        "VAE1 of page 1"
    );
    assert_eq!(block_reached(&mut m, 0x1000), Old, "VAE1 of page 1");
    m.tlbi(VAAE1, page(2) >> 12);
    assert_eq!(
        [0, 1, 2].map(|i| page_reached(&mut m, i)),
        [Old, New, New],
        "VAAE1 of page 2"
    );
    m.tlbi(VMALLE1, 0);
    assert_eq!(page_reached(&mut m, 0), New, "VMALLE1");
    assert_eq!(block_reached(&mut m, 0x1000), New, "VMALLE1");
}

/// A block is held whole: after its descriptor changes, a page of it no
/// access touched before still reaches the old output, from the one walk
/// made for another page; and a TLBI by VA anywhere in the block — at its
/// last page, in its middle — removes the whole of it, each page any access
/// reached included (Arm ARM D8.17 RLGSCG), while the pages beside it stay.
#[test]
fn a_block_is_held_whole_and_removed_by_any_address_in_it() {
    let mut m = warmed::<NoEl2>(Implementation::CORTEX_A57, 1, &[0x1000]);
    for offset in [0x1000, 0x0, 0x7000, 0x1F_F000] {
        assert_eq!(
            block_reached(&mut m, offset),
            Old,
            "the walk for offset 0x1000 serves offset {offset:#x}"
        );
    }
    m.tlbi(VALE1IS, (BLOCK + 0x10_0000) >> 12);
    for offset in [0x1000, 0x0, 0x7000, 0x1F_F000] {
        assert_eq!(
            block_reached(&mut m, offset),
            New,
            "a TLBI in the block's middle removes offset {offset:#x}"
        );
    }
    assert_eq!(page_reached(&mut m, 0), Old, "the page beside it stays");
    // Held whole again, from a walk for its last page, and removed by a
    // TLBI of its first.
    m.map_block(OLD_BLOCK);
    assert_eq!(block_reached(&mut m, 0x1F_F000), New);
    assert_eq!(block_reached(&mut m, 0x3000), New);
    m.tlbi(VAE1, BLOCK >> 12);
    assert_eq!(block_reached(&mut m, 0x1F_F000), Old);
    assert_eq!(block_reached(&mut m, 0x3000), Old);
}

/// A TLBI by ASID removes the non-global translations of its ASID alone,
/// and a TLBI by VA with an ASID the global translations of the address
/// whatever the ASID, the non-global ones of that ASID alone
/// (`TLBIOp_ASID`, `TLBIOp_VA`): pages 0 to 3 are global, 4 and 5 not, all
/// made under ASID 5 (`TTBR0_EL1.ASID`, 16 bits under `TCR_EL1.AS`).
#[test]
fn an_asid_invalidation_spares_the_global_translations() {
    let mut m = Machine::<NoEl2>::new(Implementation::CORTEX_A57);
    m.cpu.sys.tcr_el1 = TCR_EL1 | TCR_AS;
    m.cpu.sys.ttbr0_el1 = L1 | (0x105 << 48);
    for i in 0..6 {
        m.ram.mark(&[frame(i), new_frame(i)]);
        assert_eq!(m.load(page(i)), marker(frame(i)));
        m.map_page(i, new_frame(i));
    }
    m.tlbi(ASIDE1, 0x106 << 48);
    m.tlbi(ASIDE1, 0x5 << 48);
    assert_eq!(
        [0, 4].map(|i| page_reached(&mut m, i)),
        [Old, Old],
        "another ASID's TLBIs, the 16-bit one of the same low byte among them"
    );
    m.tlbi(ASIDE1, 0x105 << 48);
    assert_eq!(
        [0, 1, 4, 5].map(|i| page_reached(&mut m, i)),
        [Old, Old, New, New],
        "ASIDE1 of ASID 0x105 removes its non-global pages alone"
    );
    m.map_page(5, frame(5));
    m.tlbi(VAE1, (0x106 << 48) | (page(5) >> 12));
    m.tlbi(VAE1, (0x106 << 48) | (page(1) >> 12));
    assert_eq!(
        [1, 5].map(|i| page_reached(&mut m, i)),
        [New, New],
        "VAE1 of another ASID removes the global page 1, and leaves page 5 \
         as its last walk found it"
    );
    m.tlbi(VAE1, (0x105 << 48) | (page(5) >> 12));
    assert_eq!(page_reached(&mut m, 5), Old, "VAE1 of its own ASID");
}

/// FEAT_TTL: a TLBI by VA whose `TTL` names the granule and level of the
/// leaf that maps the address removes it, and one naming another leaves
/// it (Arm ARM D8.17.5.3 RSQXYZ, `TLBIMatch`); a reserved `TTL` is no hint
/// (`ResTLBITTL`), nor is any `TTL` on a processor without FEAT_TTL, whose
/// field is RES0.
#[test]
fn a_ttl_hint_names_the_level_of_the_leaf() {
    const TTL_4K_L2: u64 = 0b0110 << 44;
    const TTL_4K_L3: u64 = 0b0111 << 44;
    const TTL_16K_L2: u64 = 0b1010 << 44;
    const TTL_64K_L0: u64 = 0b1100 << 44;
    let mut m = warmed::<NoEl2>(Implementation::ARMV9P2, 1, &[0x1000]);
    for (ttl, reached) in [(TTL_4K_L3, Old), (TTL_16K_L2, Old), (TTL_4K_L2, New)] {
        m.tlbi(VAE1, ttl | (BLOCK >> 12));
        assert_eq!(
            block_reached(&mut m, 0x1000),
            reached,
            "the block, TTL {ttl:#x}"
        );
    }
    m.tlbi(VAE1, TTL_4K_L2 | (page(0) >> 12));
    assert_eq!(page_reached(&mut m, 0), Old, "page 0, TTL level 2");
    m.tlbi(VAE1, TTL_64K_L0 | (page(0) >> 12));
    assert_eq!(page_reached(&mut m, 0), New, "page 0, a reserved TTL");
    let mut a57 = warmed::<NoEl2>(Implementation::CORTEX_A57, 1, &[0x1000]);
    a57.tlbi(VAE1, TTL_4K_L3 | (BLOCK >> 12));
    assert_eq!(block_reached(&mut a57, 0x1000), New, "no FEAT_TTL");
}

/// FEAT_TLBIRANGE: `RVAE1` removes the translations whose leaves lie in
/// its range, in the granule its `TG` names, and nothing where `TG` is
/// 0b00 (`TLBIRange`); a block is removed whole by a range that reaches
/// any of it.
#[test]
fn a_range_removes_the_translations_it_covers() {
    /// `TG` (47:46) 4 KiB and 16 KiB; `NUM` (43:39) and `SCALE` (45:44) 0
    /// make the range `2 << granule bits` long.
    const TG_4K: u64 = 0b01 << 46;
    const TG_16K: u64 = 0b10 << 46;
    let mut m = warmed::<NoEl2>(Implementation::ARMV9P2, 4, &[0x1000]);
    m.tlbi(RVAE1, page(1) >> 12);
    m.tlbi(RVAE1, TG_16K | (page(0) >> 14));
    assert_eq!(
        [0, 1, 2, 3].map(|i| page_reached(&mut m, i)),
        [Old; 4],
        "TG 0b00, and the 16 KiB granule's range"
    );
    m.tlbi(RVAE1, TG_4K | (page(1) >> 12));
    assert_eq!(
        [0, 1, 2, 3].map(|i| page_reached(&mut m, i)),
        [Old, New, New, Old],
        "pages 1 and 2"
    );
    m.tlbi(RVAE1, TG_4K | ((BLOCK + 0x1F_F000) >> 12));
    assert_eq!(block_reached(&mut m, 0x1000), New, "the block's last page");
}

/// A 64 KiB page is held whole, as a block is: after its descriptor
/// changes, a 4 KiB piece of it no access touched still reaches the old
/// output, and a TLBI of any piece removes all of it.
#[test]
fn a_64k_page_is_held_whole() {
    /// The 64 KiB granule's level 2 table (`T0SZ` 25: a walk from level 2)
    /// and the level 3 table of its first entry: VA 0 to 64 KiB and the
    /// code's 64 KiB page map to themselves, VA [`PAGE_64K`] to `OLD`.
    const TL2: u64 = 0x2_0000;
    const TL3: u64 = 0x3_0000;
    const PAGE_64K: u64 = 0x10_0000;
    const OLD: u64 = 0x60_0000;
    const NEW: u64 = 0x70_0000;
    let mut m = Machine::<NoEl2>::new(Implementation::CORTEX_A57);
    m.ram.put64(TL2, TL3 | TABLE);
    m.ram.put64(TL3, S1_LEAF | PAGE);
    m.ram.put64(TL3 + 8 * (CODE >> 16), CODE | S1_LEAF | PAGE);
    m.ram
        .put64(TL3 + 8 * (PAGE_64K >> 16), OLD | S1_LEAF | PAGE);
    m.cpu.sys.tcr_el1 = TCR_EL1 | TG0_64K;
    m.cpu.sys.ttbr0_el1 = TL2;
    for offset in (0..0x1_0000).step_by(0x1000) {
        m.ram.mark(&[OLD + offset, NEW + offset]);
    }
    assert_eq!(m.load(PAGE_64K + 0x1000), marker(OLD + 0x1000));
    m.ram
        .put64(TL3 + 8 * (PAGE_64K >> 16), NEW | S1_LEAF | PAGE);
    for offset in [0x1000, 0x0, 0xF000] {
        assert_eq!(
            m.reaches(PAGE_64K + offset, OLD + offset, NEW + offset),
            Old,
            "offset {offset:#x}"
        );
    }
    m.tlbi(VAE1, (PAGE_64K + 0x8000) >> 12);
    for offset in [0x1000, 0x0, 0xF000] {
        assert_eq!(
            m.reaches(PAGE_64K + offset, OLD + offset, NEW + offset),
            New,
            "offset {offset:#x}"
        );
    }
}

/// FEAT_HAFDBS under a block held whole: a writable-clean block is held
/// without its write permission, so that a store to a page of it no
/// access touched walks and makes the block's descriptor writable-dirty
/// (Arm `AArch64_S1Translate`'s update), and a block whose Access flag is
/// clear is not held, so that a load anywhere in it walks and sets the
/// flag.
#[test]
fn a_block_held_whole_still_takes_its_hardware_updates() {
    /// `TCR_EL1.HA` and `HD`; a leaf's `DBM` and `AP[2]`.
    const HA: u64 = 1 << 39;
    const HD: u64 = 1 << 40;
    const DBM: u64 = 1 << 51;
    const AP2: u64 = 1 << 7;
    let mut m = Machine::<NoEl2>::new(Implementation::ARMV9P2);
    m.cpu.sys.tcr_el1 = TCR_EL1 | HA | HD;
    let clean = OLD_BLOCK | S1_LEAF | BLOCK_DESC | DBM | AP2;
    m.ram.put64(L2 + 16, clean);
    m.ram.mark(&[OLD_BLOCK + 0x1000]);
    assert_eq!(m.load(BLOCK + 0x1000), marker(OLD_BLOCK + 0x1000));
    assert_eq!(m.ram.get64(L2 + 16), clean, "a load leaves it clean");
    m.cpu.x[2] = 0x1234;
    m.cpu.x[1] = BLOCK + 0x5000;
    m.run(STR);
    assert_eq!(
        m.ram.get64(L2 + 16),
        clean & !AP2,
        "a store to another page of the block makes it dirty"
    );
    assert_eq!(m.ram.get64(OLD_BLOCK + 0x5000), 0x1234);
    m.ram
        .put64(L2 + 16, (NEW_BLOCK | S1_LEAF | BLOCK_DESC) & !AF);
    m.tlbi(VAE1, BLOCK >> 12);
    m.ram.mark(&[NEW_BLOCK + 0x2000]);
    assert_eq!(m.load(BLOCK + 0x2000), marker(NEW_BLOCK + 0x2000));
    m.ram
        .put64(L2 + 16, NEW_BLOCK | S1_LEAF | BLOCK_DESC | (1 << 52));
    m.ram.mark(&[NEW_BLOCK + 0x3000]);
    assert_eq!(
        m.load(BLOCK + 0x3000),
        marker(NEW_BLOCK + 0x3000),
        "the walk set the Access flag of a descriptor that had it clear"
    );
}

/// A snapshot keeps what the TLB holds as it held it: a block held whole
/// serves a page no access touched, and a global translation outlives an
/// ASID's invalidation, after a restore as before. An entry whose output
/// is not where a leaf of its level puts its address is one no walk makes,
/// and refused.
#[test]
fn a_restored_tlb_holds_what_it_held() {
    let mut m = warmed::<NoEl2>(Implementation::CORTEX_A57, 6, &[0x1000]);
    let context = TlbContext {
        features: m.cpu.imp.features,
        el10: m.cpu.el10_context(),
        el2: El2RegimeContext::NONE,
    };
    let mut image = Vec::new();
    assert_eq!(m.cpu.tlb.save(&mut image), Ok(()));
    m.cpu.flush_tlb();
    assert_eq!(m.cpu.tlb.restore(&mut &image[..], &context), Ok(()));
    assert_eq!(block_reached(&mut m, 0x9000), Old, "the block, whole");
    m.tlbi(ASIDE1, 0);
    assert_eq!(
        [0, 5].map(|i| page_reached(&mut m, i)),
        [Old, New],
        "the global page 0 outlives ASIDE1, the non-global page 5 does not"
    );
    // Page 0's entry made a level 1 block's: VA 0x20_0000 and its output
    // 0x60_0000 differ in bits a 1 GiB block maps alike.
    let saved = SavedTlb(image);
    let mut misplaced = saved.0.clone();
    misplaced[saved.entry_at(page(0)) + ENTRY_LEVEL] = 1;
    assert_eq!(
        m.cpu.tlb.restore(&mut &misplaced[..], &context),
        Err(NOT_WALKED)
    );
}

/// [`Machine::new`] with EL2, stage 2 on (`HCR_EL2.VM`) for VMID `vmid`,
/// every 2 MiB of IPAs mapped to itself by a block, and the EL2 regime's
/// own stage 1 on, mapping its first 2 MiB to themselves and [`BLOCK`] to
/// [`OLD_BLOCK`]; at EL1.
fn virtualized(vmid: u64) -> Machine<WithEl2> {
    let mut m = Machine::<WithEl2>::new(Implementation::CORTEX_A57);
    m.ram.put64(S2_L1, S2_L2 | TABLE);
    for i in 0..(RAM_BYTES as u64 >> 21) {
        m.ram.put64(S2_L2 + 8 * i, (i << 21) | S2_LEAF | BLOCK_DESC);
    }
    m.ram.put64(EL2_L1, EL2_L2 | TABLE);
    m.ram.put64(EL2_L2, S1_LEAF | BLOCK_DESC);
    m.ram.put64(EL2_L2 + 16, OLD_BLOCK | S1_LEAF | BLOCK_DESC);
    let imp = m.cpu.imp.get();
    let el2 = &mut m.cpu.el2_state.regs;
    el2.write(El2Reg::Vtcr, imp, 0, VTCR);
    el2.vttbr = S2_L1 | (vmid << 48);
    el2.write(El2Reg::Tcr, imp, 0, TCR_EL2);
    el2.ttbr0 = EL2_L1;
    el2.mair = 0xFF;
    el2.sctlr |= 1;
    el2.hcr |= HCR_VM;
    m.cpu.flush_tlb();
    m
}

/// Point stage 2's block of IPA `ipa` at `output`, with no maintenance.
fn map_ipa(m: &mut Machine<WithEl2>, ipa: u64, output: u64) {
    m.ram
        .put64(S2_L2 + 8 * (ipa >> 21), output | S2_LEAF | BLOCK_DESC);
}

/// A TLBI by IPA removes stage 2's translations of its IPA's block —
/// those of every page of it, the ones no access touched included — and
/// no other, VMALLS12E1 every translation of its VMID: with EL1's stage 1
/// disabled, each translation is stage 2's alone, for the IPA the address
/// is (`TLBIOp_IPAS2`, `TLBIOp_VMALLS12`).
#[test]
fn ipas2_removes_the_translations_of_stage_2_alone() {
    /// IPAs in two blocks, and where their rewritten descriptors lead.
    const X: u64 = 0x80_1000;
    const X_UNTOUCHED: u64 = 0x80_3000;
    const Y: u64 = 0xC0_0000;
    const X_NEW: u64 = 0xA0_0000;
    const Y_NEW: u64 = 0xE0_0000;
    let mut m = virtualized(1);
    m.cpu.sys.sctlr_el1 &= !1;
    m.cpu.el10_context_changed();
    m.ram
        .mark(&[X, X_UNTOUCHED, Y, X_NEW + 0x1000, X_NEW + 0x3000, Y_NEW]);
    assert_eq!(m.load(X), marker(X));
    assert_eq!(m.load(Y), marker(Y));
    map_ipa(&mut m, X, X_NEW);
    map_ipa(&mut m, Y, Y_NEW);
    assert_eq!(m.reaches(X, X, X_NEW + 0x1000), Old);
    assert_eq!(
        m.reaches(X_UNTOUCHED, X_UNTOUCHED, X_NEW + 0x3000),
        Old,
        "a page of X's block no access touched: stage 2's block is held whole"
    );
    m.at(El::El2);
    m.tlbi(IPAS2E1IS, 0x9F_F000 >> 12);
    m.at(El::El1);
    assert_eq!(
        m.reaches(X, X, X_NEW + 0x1000),
        New,
        "IPAS2E1IS in X's block"
    );
    assert_eq!(m.reaches(X_UNTOUCHED, X_UNTOUCHED, X_NEW + 0x3000), New);
    assert_eq!(m.reaches(Y, Y, Y_NEW), Old, "Y's block stays");
    m.at(El::El2);
    m.tlbi(VMALLS12E1, 0);
    m.at(El::El1);
    assert_eq!(m.reaches(Y, Y, Y_NEW), New, "VMALLS12E1");
}

/// A translation of both stages is outside a TLBI by IPA, and its stage 2
/// translation of the output IPA, kept apart, outside VMALLE1, so that a
/// hypervisor's change of stage 2 takes both, in Linux KVM's order
/// (`TLBIOp_IPAS2` leaves entries of both stages, `TLBIOp_VMALL` those of
/// stage 2 alone).
#[test]
fn a_stage_2_change_needs_ipas2_and_vmalle1() {
    let mut m = virtualized(1);
    m.ram.mark(&[
        OLD_BLOCK + 0x1000,
        OLD_BLOCK + 0x5000,
        NEW_BLOCK + 0x1000,
        NEW_BLOCK + 0x5000,
    ]);
    assert_eq!(m.load(BLOCK + 0x1000), marker(OLD_BLOCK + 0x1000));
    map_ipa(&mut m, OLD_BLOCK, NEW_BLOCK);
    assert_eq!(
        block_reached(&mut m, 0x5000),
        Old,
        "the block of both stages, whole"
    );
    m.at(El::El2);
    m.tlbi(IPAS2E1, OLD_BLOCK >> 12);
    m.at(El::El1);
    assert_eq!(block_reached(&mut m, 0x1000), Old, "IPAS2E1 alone");
    m.map_block(OLD_BLOCK);
    let mut fresh = virtualized(1);
    fresh.ram.mark(&[OLD_BLOCK + 0x1000, NEW_BLOCK + 0x1000]);
    assert_eq!(fresh.load(BLOCK + 0x1000), marker(OLD_BLOCK + 0x1000));
    map_ipa(&mut fresh, OLD_BLOCK, NEW_BLOCK);
    fresh.tlbi(VMALLE1, 0);
    assert_eq!(
        block_reached(&mut fresh, 0x1000),
        Old,
        "VMALLE1 alone: the walk finds stage 2's kept translation"
    );
    m.tlbi(VMALLE1, 0);
    assert_eq!(block_reached(&mut m, 0x1000), New, "IPAS2E1 and VMALLE1");
}

/// VMALLS12E1 removes the translations of the current VMID alone: a
/// guest's under VMID 1 outlive the one made for VMID 2's
/// (`TLBIOp_VMALLS12`'s `vmid`), and ALLE1 removes every VMID's.
#[test]
fn vmalls12e1_spares_the_other_vmids() {
    let mut m = virtualized(1);
    m.ram.mark(&[OLD_BLOCK + 0x1000, NEW_BLOCK + 0x1000]);
    assert_eq!(block_reached(&mut m, 0x1000), Old, "VMID 1 warmed");
    m.at(El::El2);
    m.cpu.x[0] = S2_L1 | (2 << 48);
    m.run(MSR_VTTBR_EL2);
    m.at(El::El1);
    assert_eq!(block_reached(&mut m, 0x1000), Old, "VMID 2 warmed");
    m.map_block(NEW_BLOCK);
    m.at(El::El2);
    m.tlbi(VMALLS12E1, 0);
    m.at(El::El1);
    assert_eq!(block_reached(&mut m, 0x1000), New, "VMID 2's removed");
    m.at(El::El2);
    m.cpu.x[0] = S2_L1 | (1 << 48);
    m.run(MSR_VTTBR_EL2);
    m.at(El::El1);
    assert_eq!(block_reached(&mut m, 0x1000), Old, "VMID 1's kept");
    m.at(El::El2);
    m.tlbi(ALLE1, 0);
    m.at(El::El1);
    assert_eq!(block_reached(&mut m, 0x1000), New, "ALLE1");
}

/// The stage 2 translations a stage 1 walk reads its tables through are
/// kept apart, a stage 2 block whole: after stage 2's block of the tables
/// moves to a copy that maps [`MORE_PAGES`] elsewhere, a walk through a
/// table page no walk read before still reads the old copy; IPAS2E1 of
/// any IPA in the block, with VMALLE1, makes the next walk read the new.
#[test]
fn a_stage_1_walk_reads_its_tables_through_stage_2_held_whole() {
    /// The IPA block stage 1's tables are moved to, and the copy's own
    /// output for `MORE_PAGES` page 0.
    const TABLES_IPA: u64 = 0x20_0000;
    const COPY: u64 = 0xC0_0000;
    const OTHER_FRAME: u64 = 0xE0_0000;
    let mut m = virtualized(1);
    // Stage 1's tables at IPAs in the block `TABLES_IPA`, identity mapped
    // by stage 2; their copy at `COPY`, the same but for `L3B`'s first
    // page.
    for (from, to) in [
        (L1, TABLES_IPA + L1),
        (L2, TABLES_IPA + L2),
        (L3, TABLES_IPA + L3),
        (L3B, TABLES_IPA + L3B),
    ] {
        for i in 0..512 {
            let mut desc = m.ram.get64(from + 8 * i);
            if desc & TABLE == TABLE && from != L3 && from != L3B {
                desc += TABLES_IPA;
            }
            m.ram.put64(to + 8 * i, desc);
            m.ram.put64(COPY + (to - TABLES_IPA) + 8 * i, desc);
        }
    }
    m.ram.put64(COPY + L3B, OTHER_FRAME | S1_LEAF | PAGE);
    m.cpu.sys.ttbr0_el1 = TABLES_IPA + L1;
    m.cpu.el10_context_changed();
    m.ram.mark(&[frame(0), frame(16), OTHER_FRAME]);
    assert_eq!(
        m.load(page(0)),
        marker(frame(0)),
        "a walk through L1, L2 and L3"
    );
    map_ipa(&mut m, TABLES_IPA, COPY);
    assert_eq!(
        m.reaches(MORE_PAGES, frame(16), OTHER_FRAME),
        Old,
        "a walk through L3B, whose IPA page no walk read: stage 2's block of \
         the tables is held whole"
    );
    m.at(El::El2);
    m.tlbi(IPAS2E1, (TABLES_IPA + 0x1F_F000) >> 12);
    m.at(El::El1);
    m.tlbi(VMALLE1, 0);
    assert_eq!(m.reaches(MORE_PAGES, frame(16), OTHER_FRAME), New);
}

/// The EL1&0 regime's maintenance leaves the EL2 regime's translations,
/// and the EL2 regime's the EL1&0 regime's, each regime's TLBIs matching
/// their own (`TLBIMatch`'s `regime`).
#[test]
fn each_regime_s_maintenance_keeps_to_its_regime() {
    let mut m = virtualized(1);
    m.el2_hcr_without_stage2();
    m.ram.mark(&[OLD_BLOCK + 0x1000, NEW_BLOCK + 0x1000]);
    for el in [El::El2, El::El1] {
        m.at(el);
        assert_eq!(block_reached(&mut m, 0x1000), Old, "{el:?} warmed");
    }
    m.map_block(NEW_BLOCK);
    m.ram.put64(EL2_L2 + 16, NEW_BLOCK | S1_LEAF | BLOCK_DESC);
    m.at(El::El2);
    m.tlbi(VMALLE1, 0);
    assert_eq!(block_reached(&mut m, 0x1000), Old, "EL2's, after VMALLE1");
    m.at(El::El1);
    assert_eq!(block_reached(&mut m, 0x1000), New, "EL1's, after VMALLE1");
    m.map_block(OLD_BLOCK);
    m.at(El::El2);
    m.tlbi(VAE2, (BLOCK + 0x10_0000) >> 12);
    assert_eq!(block_reached(&mut m, 0x1000), New, "EL2's, after VAE2");
    m.at(El::El1);
    assert_eq!(
        block_reached(&mut m, 0x1000),
        New,
        "EL1's, after VAE2: still the TLB's"
    );
    m.ram.put64(EL2_L2 + 16, OLD_BLOCK | S1_LEAF | BLOCK_DESC);
    m.at(El::El2);
    m.tlbi(ALLE2, 0);
    assert_eq!(block_reached(&mut m, 0x1000), Old, "EL2's, after ALLE2");
    m.at(El::El1);
    assert_eq!(
        block_reached(&mut m, 0x1000),
        New,
        "EL1's, after ALLE2: still the TLB's"
    );
}

impl Machine<WithEl2> {
    /// Stage 2 off (`HCR_EL2.VM` clear), the EL1&0 regime's context left
    /// to be found again as an `MSR` of `HCR_EL2` leaves it.
    fn el2_hcr_without_stage2(&mut self) {
        self.cpu.el2_state.regs.hcr &= !HCR_VM;
        self.cpu.el10_context_changed();
    }
}

/// The processors whose features each test above needs have them.
#[test]
fn the_processors_have_the_features_the_tests_read() {
    let v9 = Implementation::ARMV9P2.features;
    for feature in [
        Feature::Ttl,
        Feature::TlbiRange,
        Feature::Hafdbs,
        Feature::Lpa2,
    ] {
        assert!(v9.has(feature), "{feature:?}");
    }
    let a57: Features = Implementation::CORTEX_A57.features;
    assert!(a57.has(Feature::TGran64K) && !a57.has(Feature::Ttl));
}
