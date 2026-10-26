//! FEAT_LSE, FEAT_LSE2, FEAT_LRCPC and FEAT_LRCPC2 as a guest sees them,
//! on a processor that implements them ([`Implementation::EVERY_FEATURE`])
//! and on the Cortex-A57, which does not.
//!
//! The values the instructions compute are checked against QEMU as well
//! (`tests/vectors/qemu_max.txt`); this file holds what those vectors
//! cannot show (R9): the exception a word takes and its syndrome
//! (`ESR_EL1`, `FAR_EL1`), the writes an access makes, and the alignment
//! rules over Normal and Device memory with the MMU on. The rules are the
//! Arm ARM's shared pseudocode (`MemAtomic`, `MemAtomicInt`,
//! `AArch64_UnalignedAccessFaults`, `CreateAccDescAtomicOp`,
//! `AArch64.FaultSyndrome`) and the pages of the instructions.

use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::super::features::Implementation;
use super::super::id;
use super::super::{Cpu, CpuConfig, Step};
use super::{atomic_result, AtomicOp};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const CODE: u64 = 0x1_0000;
const VBAR: u64 = 0x2_0000;
/// The level-1 and level-2 translation tables.
const L1: u64 = 0x1000;
const L2: u64 = 0x2000;
/// Data in the first 2 MiB block: Normal memory, read/write at EL1.
const RW: u64 = 0x4_0000;
/// Data in the second: Normal memory, read-only.
const RO: u64 = 0x24_0000;
/// Data in the third: Device-nGnRnE, read/write.
const DEVICE: u64 = 0x44_0000;
/// The fourth level-2 entry is invalid. Data in the fifth: Device-nGnRnE,
/// read-only at EL1 and out of EL0's reach.
const DEVICE_RO: u64 = 0x84_0000;
const RAM_BYTES: usize = 0xA0_0000;

/// `SCTLR_EL1.A`, alignment checking for every access.
const SCTLR_A: u64 = 1 << 1;
/// `SCTLR_EL1.nAA`, the LSE2 relaxation for ordered accesses.
const SCTLR_NAA: u64 = 1 << 6;

/// Memory the processor reaches through translation, recording every
/// write it is sent.
struct Ram {
    bytes: Vec<u8>,
    writes: Vec<u64>,
}

impl PhysBus for Ram {
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.bytes.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, _clock: VmClock) -> Result<(), BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.bytes.get_mut(a..a + n).ok_or(BusFault::Unmapped)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        self.writes.push(addr.get());
        Ok(())
    }
}

/// A taken exception, as `ESR_EL1` and `FAR_EL1` report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    ec: u64,
    iss: u64,
    far: u64,
}

const UNDEFINED: Taken = Taken { ec: 0x00, iss: 0, far: 0 };
const SP_ALIGNMENT: Taken = Taken { ec: 0x26, iss: 0, far: 0 };

/// A data abort taken from EL1: the fault status code and WnR.
fn data_abort(far: u64, dfsc: u64, write: bool) -> Taken {
    Taken {
        ec: 0x25,
        iss: dfsc | (u64::from(write) << 6),
        far,
    }
}

/// `DFSC` of an alignment fault, and of a translation and a permission
/// fault at level 2.
const ALIGNMENT: u64 = 0b10_0001;
const TRANSLATION_L2: u64 = 0b00_0110;
const PERMISSION_L2: u64 = 0b00_1110;

/// A processor at EL1 with the MMU on over [`Ram`]: a level-1 table whose
/// first entry points to a level-2 table of 2 MiB blocks, identity mapped:
/// Normal read/write, Normal read-only, Device, an invalid entry, and
/// read-only Device.
struct Machine {
    cpu: Cpu,
    ram: Ram,
}

impl Machine {
    fn new(imp: Implementation) -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let mut cpu = Cpu::implementing(
            CpuConfig {
                ips,
                counter_hz: ips,
                reset_pc: CODE,
            },
            imp,
        );
        let mut ram = Ram {
            bytes: std::vec![0; RAM_BYTES],
            writes: Vec::new(),
        };
        const AF: u64 = 1 << 10;
        const INNER_SHAREABLE: u64 = 3 << 8;
        const BLOCK: u64 = 0b01;
        let descriptors = [
            (L1, L2 | 0b11),
            (L2, AF | INNER_SHAREABLE | BLOCK),
            (L2 + 8, 0x20_0000 | AF | INNER_SHAREABLE | (0b10 << 6) | BLOCK),
            (L2 + 16, 0x40_0000 | AF | (1 << 2) | BLOCK),
            (L2 + 32, 0x80_0000 | AF | (0b10 << 6) | (1 << 2) | BLOCK),
        ];
        for (at, desc) in descriptors {
            let at = at as usize;
            ram.bytes[at..at + 8].copy_from_slice(&desc.to_le_bytes());
        }
        // Attribute 0 Normal write-back, attribute 1 Device-nGnRnE.
        cpu.sys.mair_el1 = 0x00FF;
        // T0SZ = 25 (a 39-bit space, walks starting at level 1), TTBR1
        // walks disabled (EPD1).
        cpu.sys.tcr_el1 = 25 | (1 << 23);
        cpu.sys.ttbr0_el1 = L1;
        cpu.sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;
        cpu.sys.vbar_el1 = VBAR;
        Self { cpu, ram }
    }

    /// Step `insn` at `CODE`, returning the exception it took, if any. The
    /// processor stays at EL1.
    fn exec(&mut self, insn: u32) -> Option<Taken> {
        let pstate = self.cpu.pstate;
        let code = CODE as usize;
        self.ram.bytes[code..code + 4].copy_from_slice(&insn.to_le_bytes());
        self.cpu.pc = CODE;
        self.cpu.sys.esr_el1 = 0x3F << 26;
        self.cpu.sys.far_el1 = 0;
        self.ram.writes.clear();
        assert_eq!(self.cpu.step(&mut self.ram, &mut NoInstrumentation), Step::Retired, "{insn:#010x}");
        let esr = self.cpu.sys.esr_el1;
        if esr >> 26 == 0x3F {
            assert_eq!(self.cpu.pc, CODE + 4, "{insn:#010x}");
            return None;
        }
        assert_eq!(self.cpu.pc & !0x7FF, VBAR, "{insn:#010x} took an exception to the vector table");
        self.cpu.pstate = pstate;
        Some(Taken {
            ec: esr >> 26,
            iss: esr & 0x01FF_FFFF,
            far: self.cpu.sys.far_el1,
        })
    }

    fn poke(&mut self, at: u64, value: u64, bytes: usize) {
        let at = at as usize;
        self.ram.bytes[at..at + bytes].copy_from_slice(&value.to_le_bytes()[..bytes]);
    }

    fn peek(&self, at: u64, bytes: usize) -> u64 {
        let at = at as usize;
        self.ram.bytes[at..at + bytes].iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b))
    }
}

/// The atomic memory operations: `size 111 0 00 A R 1 Rs o3 opc 00 Rn Rt`.
const fn memop(size: u32, o3: u32, opc: u32, rs: u32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x3820_0000 | (rs << 16) | (o3 << 15) | (opc << 12) | (rn << 5) | rt
}

/// `ldadd`: opc 000.
const fn ldadd(size: u32, rs: u32, rn: u32, rt: u32) -> u32 {
    memop(size, 0, 0b000, rs, rn, rt)
}

/// `cas`: `size 001000 1 0 1 Rs 0 11111 Rn Rt`.
const fn cas(size: u32, rs: u32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x08A0_7C00 | (rs << 16) | (rn << 5) | rt
}

/// `casp`: `0 sz 001000 0 0 1 Rs 0 11111 Rn Rt`.
const fn casp(sz: u32, rs: u32, rn: u32, rt: u32) -> u32 {
    (sz << 30) | 0x0820_7C00 | (rs << 16) | (rn << 5) | rt
}

/// `ldapr`: `size 111 0 00 1 0 1 11111 1 100 00 Rn Rt`.
const fn ldapr(size: u32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x38BF_C000 | (rn << 5) | rt
}

/// LDAPUR*, STLUR*: `size 011001 opc 0 imm9 00 Rn Rt`.
const fn ldapstl(size: u32, opc: u32, imm9: i32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x1900_0000 | (opc << 22) | (((imm9 as u32) & 0x1FF) << 12) | (rn << 5) | rt
}

/// `ldxr`: `size 001000 0 1 0 11111 0 11111 Rn Rt`.
const fn ldxr(size: u32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x085F_7C00 | (rn << 5) | rt
}

/// `ldar`: `size 001000 1 1 0 11111 1 11111 Rn Rt`.
const fn ldar(size: u32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x08DF_FC00 | (rn << 5) | rt
}

/// `ldur`: `size 111 0 00 01 0 imm9 00 Rn Rt`.
const fn ldur(size: u32, imm9: i32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x3840_0000 | (((imm9 as u32) & 0x1FF) << 12) | (rn << 5) | rt
}

/// STR (immediate, unsigned offset) of offset 0: `size 111 0 01 00 imm12 Rn Rt`.
const fn str_imm(size: u32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x3900_0000 | (rn << 5) | rt
}

/// LDTR of offset 0: `size 111 0 00 01 0 imm9 10 Rn Rt`.
const fn ldtr(size: u32, rn: u32, rt: u32) -> u32 {
    (size << 30) | 0x3840_0800 | (rn << 5) | rt
}

/// `DC ZVA, Xt`: `SYS #3, C7, C4, #1, Xt`.
const fn dc_zva(rt: u32) -> u32 {
    0xD50B_7420 | rt
}

/// `str q1, [x3]` and `ldr q1, [x3]`.
const STR_Q1_X3: u32 = 0x3D80_0061;
const LDR_Q1_X3: u32 = 0x3DC0_0061;

#[test]
fn the_encoders_match_the_assembler() {
    // aarch64-linux-gnu-as -march=armv8.4-a+rcpc2, objdump -d.
    assert_eq!(ldadd(2, 1, 3, 2), 0xB821_0062, "ldadd w1, w2, [x3]");
    assert_eq!(memop(3, 1, 0b000, 1, 3, 2), 0xF821_8062, "swp x1, x2, [x3]");
    assert_eq!(cas(3, 1, 3, 2), 0xC8A1_7C62, "cas x1, x2, [x3]");
    assert_eq!(casp(1, 0, 4, 2), 0x4820_7C82, "casp x0, x1, x2, x3, [x4]");
    assert_eq!(ldapr(3, 3, 1), 0xF8BF_C061, "ldapr x1, [x3]");
    assert_eq!(ldapstl(3, 0b01, 8, 3, 1), 0xD940_8061, "ldapur x1, [x3, #8]");
    assert_eq!(ldapstl(0, 0b10, -2, 3, 1), 0x199F_E061, "ldapursb x1, [x3, #-2]");
    assert_eq!(ldxr(2, 3, 1), 0x885F_7C61, "ldxr w1, [x3]");
    assert_eq!(ldar(3, 3, 1), 0xC8DF_FC61, "ldar x1, [x3]");
    assert_eq!(ldur(3, 12, 3, 1), 0xF840_C061, "ldur x1, [x3, #12]");
    assert_eq!(str_imm(2, 3, 2), 0xB900_0062, "str w2, [x3]");
    assert_eq!(str_imm(3, 3, 1), 0xF900_0061, "str x1, [x3]");
    assert_eq!(ldtr(2, 3, 2), 0xB840_0862, "ldtr w2, [x3]");
    assert_eq!(ldtr(3, 3, 2), 0xF840_0862, "ldtr x2, [x3]");
    assert_eq!(dc_zva(3), 0xD50B_7423, "dc zva, x3");
}

/// Arm `AArch64_S1Translate` checks the memory type before the
/// permissions (`AArch64_S1HasAlignmentFaultDueToMemType`, then
/// `AArch64_S1CheckPermissions`): an access Device memory refuses for its
/// alignment takes an Alignment fault where the page refuses its
/// permission too — on one page, across two (reported at the access's
/// address), and for a read-modify-write whose read is allowed and write is
/// not. `DC ZVA` is refused by Device memory whatever its alignment. An
/// aligned access to the same page takes the Permission fault.
#[test]
fn device_memory_refuses_misalignment_before_permission() {
    // A permission fault at level 2 with the WnR the access has.
    let permission = |far, write| data_abort(far, PERMISSION_L2, write);
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.x[1] = 1;
    for (address, expected) in [
        (DEVICE_RO + 2, data_abort(DEVICE_RO + 2, ALIGNMENT, true)),
        (DEVICE_RO + 0xFFC, data_abort(DEVICE_RO + 0xFFC, ALIGNMENT, true)),
        (DEVICE_RO + 8, permission(DEVICE_RO + 8, true)),
    ] {
        m.cpu.x[3] = address;
        assert_eq!(m.exec(str_imm(3, 3, 1)), Some(expected), "str x1 at {address:#x}");
    }
    // LDTR reads with EL0's permissions, which the page refuses.
    for (address, expected) in [
        (DEVICE_RO + 3, data_abort(DEVICE_RO + 3, ALIGNMENT, false)),
        (DEVICE_RO + 0xFFE, data_abort(DEVICE_RO + 0xFFE, ALIGNMENT, false)),
        (DEVICE_RO + 4, permission(DEVICE_RO + 4, false)),
    ] {
        m.cpu.x[3] = address;
        assert_eq!(m.exec(ldtr(2, 3, 2)), Some(expected), "ldtr w2 at {address:#x}");
    }
    // A read-modify-write's alignment fault reports WnR clear, its refused
    // write WnR set.
    m.cpu.x[3] = DEVICE_RO + 2;
    assert_eq!(m.exec(ldadd(2, 1, 3, 2)), Some(data_abort(DEVICE_RO + 2, ALIGNMENT, false)));
    m.cpu.x[3] = DEVICE_RO + 4;
    assert_eq!(m.exec(ldadd(2, 1, 3, 2)), Some(permission(DEVICE_RO + 4, true)));
    for address in [DEVICE_RO + 0x40, DEVICE_RO + 0x48] {
        m.cpu.x[3] = address;
        assert_eq!(m.exec(dc_zva(3)), Some(data_abort(address, ALIGNMENT, true)), "dc zva at {address:#x}");
    }
    assert!(m.ram.writes.is_empty());
    // Normal read-only memory refuses only the permission.
    m.cpu.x[3] = RO + 2;
    assert_eq!(m.exec(str_imm(3, 3, 1)), Some(permission(RO + 2, true)));
}

/// Arm `Mem[]` makes a 16-byte SIMD&FP access two 8-byte single-copy
/// atomic halves, each translated with the whole access's alignment: 8
/// bytes past a 16-byte boundary, Device memory refuses it though each
/// half is aligned, writable or not. Normal memory takes it.
#[test]
fn device_memory_judges_a_q_register_access_whole() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.sys.cpacr_el1 = 3 << 20;
    for (insn, write) in [(STR_Q1_X3, true), (LDR_Q1_X3, false)] {
        for base in [DEVICE, DEVICE_RO] {
            m.cpu.x[3] = base + 8;
            assert_eq!(m.exec(insn), Some(data_abort(base + 8, ALIGNMENT, write)), "{insn:#010x} at {base:#x} + 8");
        }
        m.cpu.x[3] = DEVICE + 16;
        assert_eq!(m.exec(insn), None, "{insn:#010x}, aligned");
        m.cpu.x[3] = RW + 8;
        assert_eq!(m.exec(insn), None, "{insn:#010x} on Normal memory");
    }
}

#[test]
fn atomic_operations_follow_mem_atomic_int() {
    // Signed comparisons at the access's width: 0x80 is -128 as a byte.
    assert_eq!(atomic_result(AtomicOp::Smax, 0x80, 0x01, 8), Some(0x01));
    assert_eq!(atomic_result(AtomicOp::Smin, 0x80, 0x01, 8), Some(0x80));
    assert_eq!(atomic_result(AtomicOp::Umax, 0x80, 0x01, 8), Some(0x80));
    // The operand is compared at the width too: its upper bits are not
    // part of it.
    assert_eq!(atomic_result(AtomicOp::Umax, 0xDD4C_C57B, 0xFFFF_FFFF_843D_4D4C, 32), Some(0xDD4C_C57B));
    assert_eq!(atomic_result(AtomicOp::Smin, 0x7F, 0x0100, 8), Some(0x00));
    // ADD wraps at the width; LDCLR clears the operand's bits.
    assert_eq!(atomic_result(AtomicOp::Add, 0xFFFF, 1, 16), Some(0));
    assert_eq!(atomic_result(AtomicOp::Bic, 0xFF, 0x0F, 8), Some(0xF0));
    // A failed comparison writes nothing.
    assert_eq!(atomic_result(AtomicOp::Cas(5), 4, 9, 32), None);
    assert_eq!(atomic_result(AtomicOp::Cas(4), 4, 9, 32), Some(9));
    assert_eq!(atomic_result(AtomicOp::Cas(u128::MAX), u128::MAX, 1, 128), Some(1));
}

#[test]
fn without_lse_and_rcpc_their_encodings_are_undefined() {
    let mut m = Machine::new(Implementation::CORTEX_A57);
    m.cpu.x[3] = RW;
    m.cpu.x[4] = RW;
    for insn in [
        ldadd(3, 1, 3, 2),
        memop(3, 1, 0b000, 1, 3, 2),
        cas(3, 1, 3, 2),
        casp(1, 0, 4, 2),
        ldapr(3, 3, 1),
        ldapstl(3, 0b01, 8, 3, 1),
        ldapstl(3, 0b00, 8, 3, 1),
    ] {
        assert_eq!(m.exec(insn), Some(UNDEFINED), "{insn:#010x}");
        assert!(m.ram.writes.is_empty(), "{insn:#010x}");
    }
}

/// `CreateAccDescAtomicOp` makes an atomic access a read and a write, and
/// `AArch64_FaultSyndrome` reports `fault.write` in WnR. A permission
/// fault's is the access the page refuses, read before write
/// (`AArch64_S1CheckPermissions`): an atomic on a read-only page reports a
/// write, whatever it would compute, even a compare-and-swap whose
/// comparison would fail. Any other fault's is `NoFault`'s, `!read &&
/// write`: a translation fault reports a read. A load-acquire is a read.
#[test]
fn an_atomic_reports_a_refused_write_as_a_write() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.poke(RO, 0x1234, 8);
    m.cpu.x[1] = 1;
    m.cpu.x[2] = 0xAAAA;
    m.cpu.x[3] = RO;
    for insn in [ldadd(3, 1, 3, 2), memop(2, 1, 0b000, 1, 3, 2), cas(3, 1, 3, 2), casp(1, 0, 3, 2)] {
        assert_eq!(m.exec(insn), Some(data_abort(RO, PERMISSION_L2, true)), "{insn:#010x}");
        assert!(m.ram.writes.is_empty(), "{insn:#010x}");
        assert_eq!((m.cpu.x[1], m.cpu.x[2]), (1, 0xAAAA), "{insn:#010x} changed a register");
    }
    assert_eq!(m.exec(ldapr(3, 3, 2)), None);
    assert_eq!(m.cpu.x[2], 0x1234);

    // The level-2 entry past the three blocks is invalid.
    const UNMAPPED: u64 = 0x60_0000;
    m.cpu.x[3] = UNMAPPED;
    for insn in [ldadd(3, 1, 3, 2), cas(3, 1, 3, 2), casp(1, 0, 3, 2)] {
        assert_eq!(m.exec(insn), Some(data_abort(UNMAPPED, TRANSLATION_L2, false)), "{insn:#010x}");
    }
}

/// `MemAtomic`: a compare-and-swap whose comparison fails makes no write
/// (`Unpredictable_WRITEFAILEDCAS` taken as FALSE) and returns the value
/// read; one that succeeds writes, and returns the value read too.
#[test]
fn a_failed_compare_and_swap_writes_nothing() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.poke(RW, 5, 4);
    m.cpu.x[3] = RW;
    m.cpu.x[1] = 0xFFFF_FFFF_0000_0004;
    m.cpu.x[2] = 9;
    assert_eq!(m.exec(cas(2, 1, 3, 2)), None);
    assert!(m.ram.writes.is_empty(), "a failed CAS wrote {:x?}", m.ram.writes);
    assert_eq!((m.peek(RW, 4), m.cpu.x[1]), (5, 5));

    assert_eq!(m.exec(cas(2, 1, 3, 2)), None);
    assert_eq!(m.ram.writes, std::vec![RW]);
    assert_eq!((m.peek(RW, 4), m.cpu.x[1]), (9, 5));

    // CASP: the first register of each pair is the half at the lower
    // address.
    m.poke(RW + 16, 0x1111, 8);
    m.poke(RW + 24, 0x2222, 8);
    m.cpu.x[4] = RW + 16;
    m.cpu.x[0] = 0x1111;
    m.cpu.x[1] = 0x2223;
    m.cpu.x[2] = 0xA;
    m.cpu.x[3] = 0xB;
    assert_eq!(m.exec(casp(1, 0, 4, 2)), None);
    assert!(m.ram.writes.is_empty());
    assert_eq!((m.cpu.x[0], m.cpu.x[1]), (0x1111, 0x2222));
    assert_eq!(m.exec(casp(1, 0, 4, 2)), None);
    assert_eq!((m.peek(RW + 16, 8), m.peek(RW + 24, 8)), (0xA, 0xB));
}

/// CASP: an odd `Rs` or `Rt` is UNDEFINED (`if Rs<0> == '1' then
/// EndOfDecode(Decode_UNDEF)`, the same for `Rt`).
#[test]
fn casp_with_an_odd_register_is_undefined() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.x[4] = RW;
    for insn in [casp(1, 1, 4, 2), casp(1, 0, 4, 3), casp(0, 31, 4, 2)] {
        assert_eq!(m.exec(insn), Some(UNDEFINED), "{insn:#010x}");
        assert!(m.ram.writes.is_empty(), "{insn:#010x}");
    }
}

/// `AArch64_UnalignedAccessFaults` for an atomic or exclusive access: with
/// FEAT_LSE2 a misaligned one faults only when it leaves its 16-byte
/// granule; without, always. Device memory faults on any misaligned
/// access, and `SCTLR_EL1.A` makes every one fault. An atomic's alignment
/// fault reports WnR clear (`AlignmentFault` is `NoFault`'s record, whose
/// `write` is `!accdesc.read && accdesc.write`).
#[test]
fn with_lse2_an_atomic_may_be_misaligned_within_16_bytes() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.poke(RW + 0xC, 0x10, 4);
    m.cpu.x[1] = 2;
    m.cpu.x[3] = RW + 0xC;
    assert_eq!(m.exec(ldadd(2, 1, 3, 2)), None);
    assert_eq!((m.peek(RW + 0xC, 4), m.cpu.x[2]), (0x12, 0x10));

    for (size, address) in [(2, RW + 0xE), (3, RW + 0x9)] {
        m.cpu.x[3] = address;
        assert_eq!(m.exec(ldadd(size, 1, 3, 2)), Some(data_abort(address, ALIGNMENT, false)));
        assert!(m.ram.writes.is_empty());
    }
    m.cpu.x[4] = RW + 8;
    assert_eq!(m.exec(casp(1, 0, 4, 2)), Some(data_abort(RW + 8, ALIGNMENT, false)));

    m.cpu.x[3] = DEVICE + 1;
    assert_eq!(m.exec(ldadd(1, 1, 3, 2)), Some(data_abort(DEVICE + 1, ALIGNMENT, false)));

    m.cpu.x[3] = RW + 0x2;
    assert_eq!(m.exec(ldxr(2, 3, 2)), None);
    m.cpu.sys.sctlr_el1 |= SCTLR_A;
    assert_eq!(m.exec(ldadd(2, 1, 3, 2)), Some(data_abort(RW + 0x2, ALIGNMENT, false)));
    assert_eq!(m.exec(ldxr(2, 3, 2)), Some(data_abort(RW + 0x2, ALIGNMENT, false)));

    let mut a57 = Machine::new(Implementation::CORTEX_A57);
    a57.cpu.x[3] = RW + 0x2;
    assert_eq!(a57.exec(ldxr(2, 3, 2)), Some(data_abort(RW + 0x2, ALIGNMENT, false)));
}

/// `AArch64_UnalignedAccessFaults` for a load-acquire or store-release:
/// with FEAT_LSE2 a misaligned one faults only when it leaves its 16-byte
/// granule while `SCTLR_EL1.nAA` is 0; without, always. An ordinary load
/// is not ordered, and does not fault.
#[test]
fn an_ordered_access_may_leave_its_granule_only_with_naa() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.poke(RW, 0x0807_0605_0403_0201, 8);
    m.poke(RW + 8, 0x100F_0E0D_0C0B_0A09, 8);
    m.cpu.x[3] = RW + 4;
    assert_eq!(m.exec(ldar(3, 3, 1)), None);
    assert_eq!(m.cpu.x[1], 0x0C0B_0A09_0807_0605);

    m.cpu.x[3] = RW + 0xC;
    for insn in [ldar(3, 3, 1), ldapr(3, 3, 1), ldapstl(3, 0b01, 0, 3, 1)] {
        assert_eq!(m.exec(insn), Some(data_abort(RW + 0xC, ALIGNMENT, false)), "{insn:#010x}");
    }
    assert_eq!(m.exec(ldur(3, 0, 3, 1)), None);
    m.cpu.sys.sctlr_el1 |= SCTLR_NAA;
    for insn in [ldar(3, 3, 1), ldapr(3, 3, 1), ldapstl(3, 0b01, 0, 3, 1)] {
        m.cpu.x[1] = 0;
        assert_eq!(m.exec(insn), None, "{insn:#010x}");
        assert_eq!(m.cpu.x[1], m.peek(RW + 0xC, 8), "{insn:#010x}");
    }

    let mut a57 = Machine::new(Implementation::CORTEX_A57);
    a57.cpu.sys.sctlr_el1 |= SCTLR_NAA;
    a57.cpu.x[3] = RW + 4;
    assert_eq!(a57.exec(ldar(3, 3, 1)), Some(data_abort(RW + 4, ALIGNMENT, false)));
}

/// LDAPUR* and STLUR* take a signed offset (`SignExtend(imm9, 64)`);
/// LDAPURSB, LDAPURSH and LDAPURSW sign-extend to 64 bits, or to 32 with a
/// W destination, which zeroes the upper half.
#[test]
fn ldapur_and_stlur_take_a_signed_offset() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.poke(RW + 0x100 - 2, 0x87, 1);
    m.poke(RW + 0x100 - 8, 0x8001, 2);
    m.poke(RW + 0x100 - 16, 0x8000_0002, 4);
    m.cpu.x[3] = RW + 0x100;
    let cases = [
        (ldapstl(0, 0b10, -2, 3, 1), 0xFFFF_FFFF_FFFF_FF87),
        (ldapstl(0, 0b11, -2, 3, 1), 0xFFFF_FF87),
        (ldapstl(0, 0b01, -2, 3, 1), 0x87),
        (ldapstl(1, 0b10, -8, 3, 1), 0xFFFF_FFFF_FFFF_8001),
        (ldapstl(1, 0b11, -8, 3, 1), 0xFFFF_8001),
        (ldapstl(2, 0b10, -16, 3, 1), 0xFFFF_FFFF_8000_0002),
    ];
    for (insn, want) in cases {
        m.cpu.x[1] = 0x5555_5555_5555_5555;
        assert_eq!(m.exec(insn), None, "{insn:#010x}");
        assert_eq!(m.cpu.x[1], want, "{insn:#010x}");
    }
    m.cpu.x[1] = 0xABCD;
    assert_eq!(m.exec(ldapstl(1, 0b00, -6, 3, 1)), None);
    assert_eq!(m.ram.writes, std::vec![RW + 0x100 - 6]);
    assert_eq!(m.peek(RW + 0x100 - 6, 2), 0xABCD);
}

/// Each of these instructions with SP as its base checks SP's alignment
/// first (`if n == 31 then CheckSPAlignment()`).
#[test]
fn an_sp_base_is_checked_for_alignment() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.sp_el1 = RW + 8;
    for insn in [ldadd(3, 1, 31, 2), cas(3, 1, 31, 2), casp(1, 0, 31, 2), ldapr(3, 31, 1), ldapstl(3, 0b01, 8, 31, 1)] {
        assert_eq!(m.exec(insn), Some(SP_ALIGNMENT), "{insn:#010x}");
    }
}
