//! Loads and stores as a guest sees them (R9), where Arm's pseudocode
//! decides what QEMU's vectors cannot show: the prefetch hints, which
//! access nothing and check no stack pointer alignment (`prfm_imm.xml`,
//! `prfum.xml`, `prfm_reg.xml`); a register offset without `option<1>`,
//! UNDEFINED at decode and so ahead of the SIMD&FP enable check
//! (`ldr_reg_fpsimd.xml`); a pair of Q registers on Device memory, which
//! Arm's `Mem` makes four aligned doubleword accesses at an address 8-byte
//! aligned; and the single-copy atomic accesses of LDXP and STXP of words
//! (`Mem{64}`) and, with FEAT_LSE2, of a pair of words in one 16-byte
//! granule of Normal Write-Back Shareable memory (`Mem`, `AllInSCAGranule`,
//! `AArch64_MemSingleRead`).
//!
//! The processor runs at EL1 with the MMU off, so every data access is to
//! Device-nGnRnE memory. The words are hand-assembled from the Arm ARM's
//! encoding diagrams; each one's assembly is beside it.

use std::vec::Vec;

use rusty_box_core::VmClock;

use super::features::Implementation;
use super::step::{processor, Taken, CODE, DATA, FP_TRAP, UNDEFINED};
use super::{Cpu, NoEl2, Step};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const PRFM_SP: u32 = 0xF980_03E0; // prfm pldl1keep, [sp]
const PRFUM_SP: u32 = 0xF880_13E0; // prfum pldl1keep, [sp, #1]
const PRFM_SP_X1: u32 = 0xF8A1_6BE0; // prfm pldl1keep, [sp, x1]
const LDR_SP: u32 = 0xF940_03E0; // ldr x0, [sp]
const LDR_B_UXTB: u32 = 0x3C62_0820; // ldr b0, [x1, w2, uxtb]
const STR_B_UXTB: u32 = 0x3C22_0820; // str b0, [x1, w2, uxtb]
const LDR_B_X2: u32 = 0x3C62_6820; // ldr b0, [x1, x2]
const LDP_Q: u32 = 0xAD40_0400; // ldp q0, q1, [x0]
const STP_Q: u32 = 0xAD00_0400; // stp q0, q1, [x0]
const LDR_Q: u32 = 0x3DC0_0000; // ldr q0, [x0]
const LDXP_W: u32 = 0x887F_0C22; // ldxp w2, w3, [x1]
const STXP_W: u32 = 0x8824_0C22; // stxp w4, w2, w3, [x1]
const LDP_W: u32 = 0x2940_0C22; // ldp w2, w3, [x1]
const STP_W: u32 = 0x2900_0C22; // stp w2, w3, [x1]

/// `SCTLR_EL1.A`: every misaligned data access faults.
const SCTLR_A: u64 = 1 << 1;

/// The SP alignment fault, EC 0x26.
const SP_ALIGNMENT: Taken = Taken { ec: 0x26, iss: 0 };
/// A Data Abort taken from EL1 (EC 0x25) with DFSC 0x21, an alignment
/// fault, on a read: no ISV, as there is no EL2.
const ALIGNMENT_READ: Taken = Taken { ec: 0x25, iss: 0x21 };

/// One data access as the bus saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Access {
    address: u64,
    width: AccessWidth,
    write: bool,
}

/// RAM from address 0 to past [`DATA`], logging every data access, which
/// is every access but the instruction fetch at [`CODE`].
struct LoggedRam {
    bytes: Vec<u8>,
    log: Vec<Access>,
}

impl LoggedRam {
    /// The RAM with each data byte its offset from `DATA`.
    fn new() -> Self {
        let mut bytes = std::vec![0; (DATA + 0x100) as usize];
        for (i, b) in bytes[DATA as usize..].iter_mut().enumerate() {
            *b = i as u8;
        }
        Self {
            bytes,
            log: Vec::new(),
        }
    }

    fn quadword(&self, at: u64) -> u128 {
        let at = at as usize;
        let mut q = [0u8; 16];
        q.copy_from_slice(&self.bytes[at..at + 16]);
        u128::from_le_bytes(q)
    }

    fn note(&mut self, address: u64, width: AccessWidth, write: bool) {
        if !(CODE..CODE + 4).contains(&address) {
            self.log.push(Access {
                address,
                width,
                write,
            });
        }
    }
}

impl PhysBus for LoggedRam {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        self.note(addr.get(), width, false);
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.bytes.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        self.note(addr.get(), width, true);
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.bytes.get_mut(a..a + n).ok_or(BusFault::Unmapped)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

/// Step `insn` at `CODE` with an empty log: the exception it took, as
/// `ESR_EL1` reports it, if any. After an exception the processor is put
/// back at EL1.
fn step(cpu: &mut Cpu<NoEl2>, ram: &mut LoggedRam, insn: u32) -> Option<Taken> {
    let at = CODE as usize;
    ram.bytes[at..at + 4].copy_from_slice(&insn.to_le_bytes());
    ram.log.clear();
    let pstate = cpu.pstate;
    cpu.pc = CODE;
    cpu.sys.esr_el1 = 0x3F << 26;
    assert_eq!(cpu.step(ram, &mut NoInstrumentation), Step::Retired);
    let esr = cpu.sys.esr_el1;
    if esr >> 26 == 0x3F {
        return None;
    }
    cpu.pstate = pstate;
    Some(Taken {
        ec: esr >> 26,
        iss: esr & 0x01FF_FFFF,
    })
}

const fn read(address: u64, width: AccessWidth) -> Access {
    Access {
        address,
        width,
        write: false,
    }
}

const fn write(address: u64, width: AccessWidth) -> Access {
    Access {
        address,
        width,
        write: true,
    }
}

/// PRFM (immediate), PRFUM and PRFM (register) read SP without
/// `CheckSPAlignment` and access nothing: with SP misaligned and
/// `SCTLR_EL1.SA` set, they retire, where a load from SP takes the SP
/// alignment fault.
#[test]
fn prefetches_check_no_stack_pointer_alignment() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    let mut ram = LoggedRam::new();
    assert_ne!(cpu.sys.sctlr_el1 & (1 << 3), 0, "SCTLR_EL1.SA at reset");
    cpu.sp_el1 = DATA + 8;
    for insn in [PRFM_SP, PRFUM_SP, PRFM_SP_X1] {
        assert_eq!(step(&mut cpu, &mut ram, insn), None, "{insn:#010x}");
        assert_eq!(ram.log, [], "{insn:#010x} accesses nothing");
    }
    assert_eq!(step(&mut cpu, &mut ram, LDR_SP), Some(SP_ALIGNMENT));
}

/// LDR and STR (register, SIMD&FP) with `option<1>` clear are UNDEFINED
/// at decode, so even with SIMD&FP disabled they are UNDEFINED, where an
/// allocated register offset takes the SIMD&FP access trap.
#[test]
fn a_register_offset_without_option_1_is_undefined_before_the_fp_check() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    let mut ram = LoggedRam::new();
    cpu.sys.cpacr_el1 = 0;
    assert_eq!(step(&mut cpu, &mut ram, LDR_B_UXTB), Some(UNDEFINED));
    assert_eq!(step(&mut cpu, &mut ram, STR_B_UXTB), Some(UNDEFINED));
    assert_eq!(step(&mut cpu, &mut ram, LDR_B_X2), Some(FP_TRAP));
}

/// A pair of Q registers at an address 8-byte but not 16-byte aligned is
/// four doubleword accesses, each aligned, which Device memory takes: the
/// pair loads and stores where one Q register there, a misaligned 16-byte
/// access, takes an alignment fault. `SCTLR_EL1.A` judges the pair by its
/// 16-byte elements and faults it.
#[test]
fn a_q_register_pair_at_8_mod_16_is_four_aligned_doubleword_accesses() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    let mut ram = LoggedRam::new();
    let at = DATA + 8;
    cpu.x[0] = at;
    assert_eq!(step(&mut cpu, &mut ram, LDP_Q), None);
    assert_eq!(cpu.v[0], ram.quadword(at));
    assert_eq!(cpu.v[1], ram.quadword(at + 16));
    let doublewords = |access: fn(u64, AccessWidth) -> Access| {
        [0, 8, 16, 24].map(|offset| access(at + offset, AccessWidth::Dword))
    };
    assert_eq!(ram.log, doublewords(read));

    cpu.v[0] = 0x0F0E_0D0C_0B0A_0908_0706_0504_0302_0100 ^ !0;
    cpu.v[1] = 0x1F1E_1D1C_1B1A_1918_1716_1514_1312_1110 ^ !0;
    assert_eq!(step(&mut cpu, &mut ram, STP_Q), None);
    assert_eq!(ram.log, doublewords(write));
    assert_eq!(ram.quadword(at), cpu.v[0]);
    assert_eq!(ram.quadword(at + 16), cpu.v[1]);

    assert_eq!(step(&mut cpu, &mut ram, LDR_Q), Some(ALIGNMENT_READ));
    cpu.sys.sctlr_el1 |= SCTLR_A;
    assert_eq!(step(&mut cpu, &mut ram, LDP_Q), Some(ALIGNMENT_READ));
    assert_eq!(ram.log, [], "the pair faults before it accesses anything");
}

/// LDXP and STXP of two words are one single-copy atomic doubleword
/// access, the first register's word the low one.
#[test]
fn ldxp_and_stxp_of_words_are_one_doubleword_access() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    let mut ram = LoggedRam::new();
    cpu.x[1] = DATA;
    assert_eq!(step(&mut cpu, &mut ram, LDXP_W), None);
    assert_eq!(ram.log, [read(DATA, AccessWidth::Dword)]);
    assert_eq!((cpu.x[2], cpu.x[3]), (0x0302_0100, 0x0706_0504));

    cpu.x[2] = 0xAAAA_AAAA_1111_1111;
    cpu.x[3] = 0xBBBB_BBBB_2222_2222;
    assert_eq!(step(&mut cpu, &mut ram, STXP_W), None);
    assert_eq!(cpu.x[4], 0, "the store-exclusive succeeded");
    assert_eq!(ram.log, [write(DATA, AccessWidth::Dword)]);
    let d = DATA as usize;
    assert_eq!(
        ram.bytes[d..d + 8],
        0x2222_2222_1111_1111u64.to_le_bytes()
    );
}

/// `SCTLR_EL1.M` and `C`.
const SCTLR_M: u64 = 1 << 0;
const SCTLR_C: u64 = 1 << 2;
/// `MAIR_EL1` attribute bytes: Normal Write-Back, Normal Non-cacheable,
/// Normal Write-Back outer and Non-cacheable inner, and Device-nGnRnE.
const NORMAL_WRITE_BACK: u64 = 0xFF;
const NORMAL_NON_CACHEABLE: u64 = 0x44;
const OUTER_WRITE_BACK_ONLY: u64 = 0xF4;
const DEVICE: u64 = 0x00;
/// A block descriptor's SH field: Inner, Outer and Non-shareable.
const INNER_SHAREABLE: u64 = 0b11;
const OUTER_SHAREABLE: u64 = 0b10;
const NON_SHAREABLE: u64 = 0b00;

/// Map the first 2 MiB of RAM, identity, as one block with MAIR attribute
/// `attr` and shareability `sh`, and turn the MMU on with `SCTLR_EL1.C` as
/// `cached` says, through tables in the RAM below `CODE`.
fn map_block(cpu: &mut Cpu<NoEl2>, ram: &mut LoggedRam, attr: u64, sh: u64, cached: bool) {
    const L1: usize = 0x1000;
    const L2: usize = 0x2000;
    const AF: u64 = 1 << 10;
    let table = L2 as u64 | 0b11;
    let block = AF | (sh << 8) | 0b01;
    ram.bytes[L1..L1 + 8].copy_from_slice(&table.to_le_bytes());
    ram.bytes[L2..L2 + 8].copy_from_slice(&block.to_le_bytes());
    cpu.sys.mair_el1 = attr;
    cpu.sys.tcr_el1 = 25;
    cpu.sys.ttbr0_el1 = L1 as u64;
    let c = if cached { SCTLR_C } else { 0 };
    cpu.sys.sctlr_el1 = (cpu.sys.sctlr_el1 & !SCTLR_C) | SCTLR_M | c;
    cpu.tlb.flush();
}

/// The data accesses LDP and STP of two words at `at` make, and the words
/// they load and store, each pair checked against `loads` and `stores`. The
/// first run of each fills the TLB, whose table walk the log shows; the
/// second is the one checked.
fn word_pair_accesses(
    cpu: &mut Cpu<NoEl2>,
    ram: &mut LoggedRam,
    at: u64,
    one_access: bool,
    case: &str,
) {
    let (loads, stores) = if one_access {
        (
            std::vec![read(at, AccessWidth::Dword)],
            std::vec![write(at, AccessWidth::Dword)],
        )
    } else {
        (
            std::vec![read(at, AccessWidth::Word), read(at + 4, AccessWidth::Word)],
            std::vec![write(at, AccessWidth::Word), write(at + 4, AccessWidth::Word)],
        )
    };
    let offset = at - DATA;
    cpu.x[1] = at;
    assert_eq!(step(cpu, ram, LDP_W), None);
    assert_eq!(step(cpu, ram, LDP_W), None);
    assert_eq!(ram.log, loads, "{case}");
    let word = |first: u64| (0..4).fold(0, |w, i| w | ((first + i) << (8 * i)));
    assert_eq!((cpu.x[2], cpu.x[3]), (word(offset), word(offset + 4)));

    cpu.x[2] = 0x5555_5555;
    cpu.x[3] = 0x6666_6666;
    assert_eq!(step(cpu, ram, STP_W), None);
    assert_eq!(ram.log, stores, "{case}");
    let a = at as usize;
    assert_eq!(
        ram.bytes[a..a + 8],
        0x6666_6666_5555_5555u64.to_le_bytes()
    );
}

/// With FEAT_LSE2, LDP and STP of two words that lie in one 16-byte
/// granule of Normal Write-Back Shareable memory are one single-copy atomic
/// doubleword access (Arm `AArch64_MemSingleRead`: `IsWBShareable`). Across
/// a granule boundary, on a processor without FEAT_LSE2, and on any other
/// memory they are two word accesses: the same words reach the registers,
/// but a device gets the two accesses it answers.
#[test]
fn with_lse2_a_word_pair_is_one_access_only_on_write_back_shareable_memory() {
    const IN_GRANULE: u64 = DATA + 4;
    const ACROSS_GRANULES: u64 = DATA + 12;
    for (imp, attr, sh, cached, at, one_access, case) in [
        (
            Implementation::ARMV9P2,
            NORMAL_WRITE_BACK,
            INNER_SHAREABLE,
            true,
            IN_GRANULE,
            true,
            "Write-Back Inner Shareable",
        ),
        (
            Implementation::ARMV9P2,
            NORMAL_WRITE_BACK,
            OUTER_SHAREABLE,
            true,
            IN_GRANULE,
            true,
            "Write-Back Outer Shareable",
        ),
        (
            Implementation::ARMV9P2,
            NORMAL_WRITE_BACK,
            0b01,
            true,
            IN_GRANULE,
            true,
            "the reserved SH value, Outer Shareable by Arm's default",
        ),
        (
            Implementation::ARMV9P2,
            NORMAL_WRITE_BACK,
            INNER_SHAREABLE,
            true,
            ACROSS_GRANULES,
            false,
            "across a 16-byte granule",
        ),
        (
            Implementation::ARMV9P2,
            NORMAL_WRITE_BACK,
            NON_SHAREABLE,
            true,
            IN_GRANULE,
            false,
            "Non-shareable",
        ),
        (
            Implementation::ARMV9P2,
            NORMAL_WRITE_BACK,
            INNER_SHAREABLE,
            false,
            IN_GRANULE,
            false,
            "SCTLR_EL1.C clear makes it Non-cacheable",
        ),
        (
            Implementation::ARMV9P2,
            NORMAL_NON_CACHEABLE,
            INNER_SHAREABLE,
            true,
            IN_GRANULE,
            false,
            "Normal Non-cacheable",
        ),
        (
            Implementation::ARMV9P2,
            OUTER_WRITE_BACK_ONLY,
            INNER_SHAREABLE,
            true,
            IN_GRANULE,
            false,
            "Write-Back outer alone",
        ),
        (
            Implementation::ARMV9P2,
            DEVICE,
            NON_SHAREABLE,
            true,
            IN_GRANULE,
            false,
            "Device memory",
        ),
        (
            Implementation::CORTEX_A57,
            NORMAL_WRITE_BACK,
            INNER_SHAREABLE,
            true,
            IN_GRANULE,
            false,
            "without FEAT_LSE2",
        ),
    ] {
        let mut cpu = processor(imp);
        let mut ram = LoggedRam::new();
        map_block(&mut cpu, &mut ram, attr, sh, cached);
        word_pair_accesses(&mut cpu, &mut ram, at, one_access, case);
    }
}

/// With the MMU off every data access is to Device-nGnRnE memory, so with
/// FEAT_LSE2 too a pair of words is two word accesses, which a device with
/// no 8-byte access answers.
#[test]
fn with_the_mmu_off_a_word_pair_is_two_accesses() {
    let mut cpu = processor(Implementation::ARMV9P2);
    let mut ram = LoggedRam::new();
    word_pair_accesses(&mut cpu, &mut ram, DATA + 4, false, "MMU off");
}
