//! Which A64 encodings this core executes, by feature set, measured against
//! an independent encoding table: disarm64's, generated from binutils'
//! `opcodes/aarch64-tbl.h`.
//!
//! Every row of disarm64's `INSNS` table (mnemonic, opcode, mask, class,
//! feature set) is read from its `decoder_full.rs` in the cargo registry.
//! For each row, 48 random words that disarm64 decodes back to that row's
//! opcode/mask run on a bare machine for one instruction, with SIMD&FP
//! enabled (`CPACR_EL1.FPEN = 0b11`) and UNDEFINED stopping the machine. A
//! row *executes* when some instance retires (an exception it raises, such as
//! a data abort or `SVC`, counts) and is *UNDEFINED* when every instance is.
//! No outcome says an encoding has no handler: dispatch matches every class
//! the decoder returns, so the machine cannot stop for that.
//!
//! The property asserted is the guest-visible one (R9): every encoding of a
//! feature set this processor model advertises executes, except the ones
//! [`UNDEFINED_AT_EL1`] lists with the Arm ARM rule that makes them so; every
//! instance of an encoding of any other feature set is UNDEFINED, except the
//! HINT space, which executes whatever the hint, and the single- and
//! double-precision words of the fixed-point conversions disarm64 files
//! under FEAT_FP16.
//!
//! It reads disarm64's source from the cargo registry
//! (`$CARGO_HOME/registry/src/*/disarm64-0.2.*`, which building the tests
//! fetches), and runs with every test run: it is the check, needing no Arm
//! data, that no encoding of an advertised feature has gone missing from the
//! decoder. `cargo test --release -p rusty_box_aarch64 --features std --test
//! encoding_coverage -- --nocapture` prints the table.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use disarm64::InsnOpcode;
use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::cpu::DebugStops;
use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, StopReason, SysReg, VirtConfig};

/// One row of disarm64's `INSNS`.
struct Row {
    mnemonic: String,
    opcode: u32,
    mask: u32,
    class: String,
    feature_set: String,
}

impl Row {
    /// The SCVTF, UCVTF, FCVTZS and FCVTZU (vector and scalar, by
    /// immediate) rows disarm64 files under FEAT_FP16 for every `immh<1>`
    /// set, though only `immh = 001x` is half precision.
    fn is_fixed_point_conversion_filed_as_fp16(&self) -> bool {
        self.feature_set == "SIMD_F16" && (self.class == "ASIMDSHF" || self.class == "ASISDSHF")
    }
}

/// What one encoding did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    Executes,
    Undefined,
    /// disarm64 decodes no word to this row: a more specific row covers all of it.
    NoInstances,
}

/// The feature sets the Cortex-A57 model advertises: ARMv8.0 base, FP and
/// Advanced SIMD, and the optional AES (with the 64-bit PMULL), SHA-1 and
/// SHA-256 (binutils' SHA2 set) and CRC32.
const ADVERTISED: &[&str] = &["V8", "FP", "SIMD", "AES", "SHA2", "CRC"];

/// Encodings of advertised feature sets that are UNDEFINED in the state the
/// harness runs them in — EL1, no EL2 or EL3, not in Debug state — with the
/// Arm ARM rule that makes them so.
const UNDEFINED_AT_EL1: &[(&str, &str)] = &[
    ("udf", "UDF: permanently undefined (C6.2 UDF)"),
    ("dcps1", "DCPS1: UNDEFINED in Non-debug state (C6.2 DCPS1)"),
    ("dcps2", "DCPS2: UNDEFINED in Non-debug state (C6.2 DCPS2)"),
    ("dcps3", "DCPS3: UNDEFINED in Non-debug state (C6.2 DCPS3)"),
    ("drps", "DRPS: UNDEFINED in Non-debug state (C6.2 DRPS)"),
    (
        "hlt",
        "HLT: UNDEFINED while halting is not allowed (C6.2 HLT, EDSCR.HDE = 0)",
    ),
    ("smc", "SMC: UNDEFINED without EL3 (C6.2 SMC)"),
    (
        "sysl",
        "SYSL: every encoding is IMPLEMENTATION DEFINED, and this core defines none (C5.1)",
    ),
];

/// Encodings whose random instances name system registers this core does
/// not implement — almost every `op0:op1:CRn:CRm:op2` is unallocated — so
/// their verdict says nothing; one instance naming a register that exists
/// stands in for them.
const PINNED: &[(&str, u32, &str)] = &[
    ("mrs", 0xD538_0000, "mrs x0, midr_el1"),
    ("msr", 0xD518_D080, "msr tpidr_el1, x0"),
    // The ARMv8.0 SYS operations reachable from EL1 (AT, DC, IC, TLBI) are
    // 27 of the 2^14 op1:CRn:CRm:op2 values.
    ("sys", 0xD508_871F, "tlbi vmalle1"),
];

fn registry_file() -> Option<PathBuf> {
    // Cargo's own resolution: CARGO_HOME (which rustup sets for every
    // process it starts), else `.cargo` in the home directory.
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cargo")))
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| Path::new(&h).join(".cargo")))?;
    let src = cargo_home.join("registry").join("src");
    for index in std::fs::read_dir(src).ok()?.flatten() {
        for krate in std::fs::read_dir(index.path()).ok()?.flatten() {
            let name = krate.file_name();
            if name.to_string_lossy().starts_with("disarm64-0.2.") {
                let file = krate.path().join("src").join("decoder_full.rs");
                if file.is_file() {
                    return Some(file);
                }
            }
        }
    }
    None
}

/// The text after `key` up to the next `,` in `block`.
fn field<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    let start = block.find(key)? + key.len();
    let rest = &block[start..];
    Some(rest[..rest.find(',')?].trim())
}

fn parse_rows(source: &str) -> Vec<Row> {
    let table = source
        .find("static INSNS:")
        .map(|start| &source[start..])
        .expect("decoder_full.rs has an INSNS table");
    table
        .split("Insn {")
        .skip(1)
        .map(|block| {
            let hex = |key: &str| {
                let text = field(block, key).expect("a hexadecimal field");
                u32::from_str_radix(text.trim_start_matches("0x"), 16).expect("a 32-bit value")
            };
            Row {
                mnemonic: field(block, "mnemonic:")
                    .expect("a mnemonic")
                    .trim_matches('"')
                    .to_string(),
                opcode: hex("opcode:"),
                mask: hex("mask:"),
                class: field(block, "class: InsnClass::")
                    .expect("a class")
                    .to_string(),
                feature_set: field(block, "feature_set: InsnFeatureSet::")
                    .expect("a feature set")
                    .to_string(),
            }
        })
        .collect()
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

const INSTANCES: usize = 48;

/// Up to [`INSTANCES`] random words disarm64 decodes to `row`'s opcode/mask.
fn instances(row: &Row, seed: u64) -> Vec<u32> {
    let mut rng = SplitMix64(seed);
    let mut words = Vec::new();
    for _ in 0..INSTANCES * 256 {
        let word = row.opcode | (rng.next() as u32 & !row.mask);
        let same = disarm64::decoder::decode(word)
            .map(|op| op.definition())
            .is_some_and(|d| d.opcode == row.opcode && d.mask == row.mask);
        if same && !words.contains(&word) {
            words.push(word);
            if words.len() == INSTANCES {
                break;
            }
        }
    }
    words
}

/// Run `word` for one instruction on a fresh bare machine: whether it
/// retired (executing, or taking an exception other than UNDEFINED).
fn executes(ram: &mut [u8], word: u32) -> bool {
    let config = VirtConfig {
        debug_stops: DebugStops {
            on_undefined: true,
            on_bus_fault: false,
        },
        ..VirtConfig::DEFAULT
    };
    let mut machine = MachineBuilder::new(config)
        .ram(RamSize::MIN)
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    machine
        .mem_write(RAM_BASE, &word.to_le_bytes())
        .expect("code is in RAM");
    machine
        .sysreg_write(SysReg::CPACR_EL1, 3 << 20)
        .expect("CPACR_EL1 is writable");
    match machine.emu_start(RAM_BASE, None, Some(1)).stop {
        StopReason::Undefined(_) => false,
        StopReason::BudgetExhausted
        | StopReason::Halted
        | StopReason::StopRequested
        | StopReason::ExitReached
        | StopReason::PoweredOff
        | StopReason::ResetRequested
        | StopReason::ProcessorOff
        | StopReason::BusFault { .. } => true,
    }
}

/// Whether an instance of `row` must execute on this model (`Some`), or
/// may do either (`None`).
fn must_execute(row: &Row, word: u32) -> Option<bool> {
    if word & 0xFFFF_F01F == 0xD503_201F {
        return Some(true);
    }
    if row.is_fixed_point_conversion_filed_as_fp16() {
        // immh = 001x is half precision, absent here; the rest is the
        // ARMv8.0 single- or double-precision form, whose handler rejects
        // its own reserved values (immh = 1xxx with Q = 0).
        let immh = (word >> 19) & 0xF;
        return if immh & 0b1100 == 0 {
            Some(false)
        } else {
            None
        };
    }
    if !ADVERTISED.contains(&row.feature_set.as_str()) {
        return Some(false);
    }
    if UNDEFINED_AT_EL1.iter().any(|(m, _)| *m == row.mnemonic) {
        return Some(false);
    }
    // Any one instance of an advertised encoding may be a reserved value
    // its handler rejects; the row's verdict is what must execute.
    None
}

#[derive(Default)]
struct Tally {
    rows: usize,
    executes: usize,
    undefined: usize,
    no_instances: usize,
}

#[test]
fn every_encoding_of_an_advertised_feature_executes_and_every_other_is_undefined() {
    let path = registry_file().expect("disarm64 0.2 is in the cargo registry");
    let source = std::fs::read_to_string(&path).expect("decoder_full.rs is readable");
    let rows = parse_rows(&source);
    println!("{} rows from {}", rows.len(), path.display());

    let mut ram = vec![0u8; RamSize::MIN.get() as usize];
    let mut by_feature: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut undefined_advertised = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let words = instances(row, 0xC0DE_0000 + index as u64);
        let mut executed = false;
        for &word in &words {
            let ran = executes(&mut ram, word);
            executed |= ran;
            if let Some(want) = must_execute(row, word) {
                if ran != want {
                    failures.push(format!(
                        "{} {:08x}/{:08x} {}: instance {word:08x} {}",
                        row.mnemonic,
                        row.opcode,
                        row.mask,
                        row.feature_set,
                        if ran { "executed" } else { "is UNDEFINED" }
                    ));
                }
            }
        }
        let verdict = match (words.is_empty(), executed) {
            (true, _) => Verdict::NoInstances,
            (false, true) => Verdict::Executes,
            (false, false) => Verdict::Undefined,
        };
        let tally = by_feature.entry(row.feature_set.as_str()).or_default();
        tally.rows += 1;
        match verdict {
            Verdict::Executes => tally.executes += 1,
            Verdict::Undefined => tally.undefined += 1,
            Verdict::NoInstances => tally.no_instances += 1,
        }
        let advertised = ADVERTISED.contains(&row.feature_set.as_str());
        if advertised && verdict == Verdict::Undefined {
            let pinned = PINNED.iter().find(|(m, _, _)| *m == row.mnemonic);
            undefined_advertised.push(format!(
                "{} {:08x}/{:08x} {}",
                row.mnemonic, row.opcode, row.mask, row.class
            ));
            let explained = UNDEFINED_AT_EL1.iter().any(|(m, _)| *m == row.mnemonic);
            match pinned {
                Some((_, word, text)) => {
                    if !executes(&mut ram, *word) {
                        failures.push(format!("{text} ({word:08x}) is UNDEFINED"));
                    }
                }
                None if explained => {}
                None => failures.push(format!(
                    "{} {:08x}/{:08x}: no instance executes",
                    row.mnemonic, row.opcode, row.mask
                )),
            }
        }
    }

    println!("feature set              rows  executes  UNDEFINED  no-instances");
    for (feature, t) in &by_feature {
        println!(
            "{feature:<22} {:>6} {:>9} {:>10} {:>13}",
            t.rows, t.executes, t.undefined, t.no_instances
        );
    }
    println!(
        "UNDEFINED encodings of advertised feature sets ({}):",
        undefined_advertised.len()
    );
    for line in &undefined_advertised {
        println!("  {line}");
    }
    for line in &failures {
        println!("FAIL {line}");
    }
    assert!(
        failures.is_empty(),
        "{} instances disagree with the model",
        failures.len()
    );
}
