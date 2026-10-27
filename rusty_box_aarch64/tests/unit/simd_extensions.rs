//! FEAT_RDM, FEAT_DotProd and FEAT_I8MM as a guest sees them, on a
//! processor that implements them ([`Implementation::EVERY_FEATURE`]) and
//! on the Cortex-A57, which does not.
//!
//! The values the instructions compute are checked against QEMU
//! (`tests/vectors/qemu_max.txt`); this file holds what those vectors cannot
//! show (R9): the words the pages' decode pseudocode makes UNDEFINED (QEMU
//! refuses them too, so no vector has them), and the exception an allocated
//! word takes when SIMD&FP is disabled or the processor is in Streaming SVE
//! mode (`AArch64_CheckFPAdvSIMDEnabled`), after decode.

use std::vec::Vec;

use super::super::features::Implementation;
use super::super::step::{exec, processor, CPACR_ALL, FP_TRAP, ILLEGAL_WHEN_STREAMING, UNDEFINED};

/// `sqrdmlah v0.<T>, v1.<T>, v2.<T>` (`sqrdmlsh` with `sub`) at `size`.
const fn rdm_vector(size: u32, sub: bool) -> u32 {
    0x6E00_8420 | (size << 22) | (2 << 16) | ((sub as u32) << 11)
}

/// `sqrdmlah <V>0, <V>1, <V>2` (`sqrdmlsh` with `sub`) at `size`.
const fn rdm_scalar(size: u32, sub: bool) -> u32 {
    0x7E00_8420 | (size << 22) | (2 << 16) | ((sub as u32) << 11)
}

/// `sqrdmlah v0.<T>, v1.<T>, v2.<Ts>[0]` (`sqrdmlsh` with `sub`) at `size`.
const fn rdm_elem(size: u32, sub: bool) -> u32 {
    0x6F00_D020 | (size << 22) | (2 << 16) | ((sub as u32) << 13)
}

/// `sqrdmlah <V>0, <V>1, v2.<Ts>[0]` (`sqrdmlsh` with `sub`) at `size`.
const fn rdm_scalar_elem(size: u32, sub: bool) -> u32 {
    0x7F00_D020 | (size << 22) | (2 << 16) | ((sub as u32) << 13)
}

/// `sdot v0.4s, v1.16b, v2.16b` (`udot` with `unsigned`) at `size`.
const fn dot_vector(size: u32, unsigned: bool) -> u32 {
    0x4E00_9420 | ((unsigned as u32) << 29) | (size << 22) | (2 << 16)
}

/// `sdot v0.4s, v1.16b, v2.4b[0]` (`udot` with `unsigned`) at `size`.
const fn dot_elem(size: u32, unsigned: bool) -> u32 {
    0x4F00_E020 | ((unsigned as u32) << 29) | (size << 22) | (2 << 16)
}

/// `usdot v0.4s, v1.16b, v2.16b`.
const USDOT: u32 = 0x4E82_9C20;
/// `sudot v0.4s, v1.16b, v2.4b[0]`; `usdot` (by element) sets `US`.
const SUDOT_ELEM: u32 = 0x4F02_F020;
const USDOT_ELEM: u32 = 0x4F82_F020;
/// `smmla`, `usmmla`, `ummla v0.4s, v1.16b, v2.16b`.
const SMMLA: u32 = 0x4E82_A420;
const USMMLA: u32 = 0x4E82_AC20;
const UMMLA: u32 = 0x6E82_A420;

/// One allocated word of every form, each of them executable.
const ALLOCATED: [u32; 21] = [
    rdm_vector(1, false),
    rdm_vector(2, true),
    rdm_scalar(1, false),
    rdm_scalar(2, true),
    rdm_elem(1, false),
    rdm_elem(2, true),
    rdm_scalar_elem(1, true),
    rdm_scalar_elem(2, false),
    dot_vector(2, false),
    dot_vector(2, true),
    USDOT,
    dot_elem(2, false),
    dot_elem(2, true),
    SUDOT_ELEM,
    USDOT_ELEM,
    SMMLA,
    USMMLA,
    UMMLA,
    rdm_vector(2, false),
    rdm_elem(2, false),
    rdm_scalar_elem(1, false),
];

#[test]
fn the_encoders_match_the_assembler() {
    // aarch64-linux-gnu-as -march=armv8.6-a+i8mm, objdump -d.
    assert_eq!(
        rdm_vector(1, false),
        0x6E42_8420,
        "sqrdmlah v0.8h, v1.8h, v2.8h"
    );
    assert_eq!(rdm_scalar(1, false), 0x7E42_8420, "sqrdmlah h0, h1, h2");
    assert_eq!(
        rdm_elem(1, false),
        0x6F42_D020,
        "sqrdmlah v0.8h, v1.8h, v2.h[0]"
    );
    assert_eq!(
        rdm_scalar_elem(2, false),
        0x7F82_D020,
        "sqrdmlah s0, s1, v2.s[0]"
    );
    assert_eq!(
        dot_vector(2, false),
        0x4E82_9420,
        "sdot v0.4s, v1.16b, v2.16b"
    );
    assert_eq!(
        dot_elem(2, false),
        0x4F82_E020,
        "sdot v0.4s, v1.16b, v2.4b[0]"
    );
    assert_eq!(USDOT, 0x4E82_9C20, "usdot v0.4s, v1.16b, v2.16b");
    assert_eq!(SUDOT_ELEM, 0x4F02_F020, "sudot v0.4s, v1.16b, v2.4b[0]");
    assert_eq!(USDOT_ELEM, 0x4F82_F020, "usdot v0.4s, v1.16b, v2.4b[0]");
    assert_eq!(SMMLA, 0x4E82_A420, "smmla v0.4s, v1.16b, v2.16b");
    assert_eq!(USMMLA, 0x4E82_AC20, "usmmla v0.4s, v1.16b, v2.16b");
    assert_eq!(UMMLA, 0x6E82_A420, "ummla v0.4s, v1.16b, v2.16b");
}

#[test]
fn without_the_features_their_encodings_are_undefined() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    for insn in ALLOCATED {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

#[test]
fn with_the_features_every_form_executes() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in ALLOCATED {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    }
}

/// The decode pseudocode: SQRDMLAH and SQRDMLSH `if size == '11' || size
/// == '00' then EndOfDecode(Decode_UNDEF)` (by element, `otherwise`), SDOT
/// and UDOT `if size != '10'`.
#[test]
fn sizes_the_decode_pseudocode_refuses_are_undefined() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    let mut refused = Vec::new();
    for size in [0b00, 0b11] {
        for sub in [false, true] {
            refused.extend([
                rdm_vector(size, sub),
                rdm_scalar(size, sub),
                rdm_elem(size, sub),
                rdm_scalar_elem(size, sub),
            ]);
        }
    }
    for size in [0b00, 0b01, 0b11] {
        for unsigned in [false, true] {
            refused.extend([dot_vector(size, unsigned), dot_elem(size, unsigned)]);
        }
    }
    for insn in refused {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// `AArch64_CheckFPAdvSIMDEnabled` runs after decode: with SIMD&FP disabled
/// an allocated word takes the access trap and a refused size is still
/// UNDEFINED; in Streaming SVE mode each of these Advanced SIMD
/// instructions takes the SME trap (SMTC 1).
#[test]
fn the_simd_checks_follow_decode() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.cpacr_el1 = 3 << 24;
    for insn in ALLOCATED {
        assert_eq!(exec(&mut cpu, insn), Some(FP_TRAP), "{insn:#010x}");
    }
    assert_eq!(exec(&mut cpu, rdm_vector(0, false)), Some(UNDEFINED));
    assert_eq!(exec(&mut cpu, dot_vector(1, false)), Some(UNDEFINED));

    cpu.sys.cpacr_el1 = CPACR_ALL;
    // smstart sm
    assert_eq!(exec(&mut cpu, 0xD503_437F), None);
    for insn in ALLOCATED {
        assert_eq!(
            exec(&mut cpu, insn),
            Some(ILLEGAL_WHEN_STREAMING),
            "{insn:#010x}"
        );
    }
}
