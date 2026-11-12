//! System register and System instruction accesses checked against Arm's
//! machine-readable specification (R7, R9).
//!
//! The source of truth is `Registers.json` of the Arm A-profile Architecture
//! Machine Readable Specification (AARCHMRS), as Arm distributes it under
//! the BSD 3-clause licence: for each AArch64 System register and System
//! instruction, the accessors of its encodings, each a decision list in
//! Arm's pseudocode ending in the access, `Undefined()` or a trap, and its
//! fieldsets. As for `spec.rs`, it is not part of this repository: the
//! tests read it from the unpacked AARCHMRS directory `RUSTY_BOX_AARCHMRS`
//! names, and are ignored when the variable was not set for the build. They
//! were written against release 2026-09 and report the release they read.
//!
//! - [`every_system_access_takes_the_exception_arms_accessor_takes`] has an
//!   interpreter of the accessors' pseudocode decide what each `MRS`,
//!   `MSR` and System instruction encoding does on each processor this
//!   crate builds and the test-only ones, at EL0, EL1 and (with EL2) EL2,
//!   in each of a set of states of the registers that control access: the
//!   access, an UNDEFINED exception, a trap to an exception level with a
//!   class, or with FEAT_NV2 a load or store at an offset of the page
//!   `VNCR_EL2` names (`NVMem`). The processor stepping the word must take
//!   the same exception, with the syndrome `AArch64_SystemAccessTrapSyndrome`
//!   gives it, or none, loading from or storing to that offset and no other
//!   where the accessor does. An encoding no accessor names is UNDEFINED.
//!   With FEAT_NV the states include a guest hypervisor's at EL1, under each
//!   `HCR_EL2.{NV2, NV1, NV}` it can run with.
//! - [`each_el2_register_keeps_the_bits_its_fieldset_gives_it`] writes every
//!   EL2 register at EL2 with all ones and with all zeros: each bit of a
//!   field reads back as written, each RES0 bit reads 0 and each RES1 bit
//!   reads 1, by the fieldset Arm's data gives the processor's features.

use std::collections::{BTreeMap, BTreeSet};
use std::format;
use std::path::PathBuf;
use std::string::{String, ToString};
use std::sync::OnceLock;
use std::vec;
use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};
use serde_json::Value;

use super::features::{Feature, Implementation, LeveledImplementation};
use super::pauth::PacKeys;
use super::pmu::Pmu;
use super::wait::EventRegister;
use super::{
    id, sme, Activity, Cpu, CpuConfig, El, ExclusiveMonitor, GicCpuInterface, Levels, NoEl2,
    Pstate, Step, SysRegs, SystemRegisterInterface, WithEl2,
};
use crate::bus::{
    AccessWidth, BusFault, GicEl1Traps, GicRegister, NoGicSystemRegisters, PhysAddr, PhysBus,
};
use crate::devices::gicv2::{GicNames, GicShape, PriorityBits};
use crate::devices::gicv3::GicV3;
use crate::devices::gicv3_virt::VirtualInterfaceShape;
use crate::instrumentation::NoInstrumentation;

/// The environment variable naming the unpacked AARCHMRS directory.
const SPEC_DIR_VAR: &str = "RUSTY_BOX_AARCHMRS";

/// Where the word under test executes, and the vector bases: a completed
/// access leaves the processor at the next word, an exception at a vector.
const CODE: u64 = 0x1000;
const VBAR_EL1: u64 = 0x8000;
const VBAR_EL2: u64 = 0xC000;
const RAM_BYTES: usize = 0x1_0000;

/// The register an access names when it names none: `XZR`, so a write
/// writes zero and a read is discarded. The 128-bit forms name the pair
/// X0, X1.
const RT: u32 = 31;
const RT_PAIR: u32 = 0;

/// The choices Arm leaves to an implementation (`ImpDefBool`) that the
/// accessors this processor reaches ask about, with this processor's
/// answer and why.
const IMPLEMENTATION_DEFINED: [(&str, bool, &str); 27] = [
    (
        "Treat ICC_SRE_EL2.SRE as RAO/WI",
        true,
        "the GIC CPU interface has its System registers only",
    ),
    (
        "Treat ICC_SRE_EL2.Enable as RAO/WI",
        false,
        "ICC_SRE_EL2.Enable is a field, which traps ICC_SRE_EL1 while clear",
    ),
    (
        "Implementation can reset into AArch32 state",
        false,
        "there is no AArch32",
    ),
    ("IMPLEMENTED_TRCIMSPEC<n>", false, "there is no trace unit"),
    (
        "AArch64-AMEVCNTR1_EL0[n] is fixed",
        false,
        "there are no activity monitors",
    ),
    (
        "IMPLEMENTED_ACTLR_ELx accessor behavior",
        false,
        "the ACTLR_ELx encodings reach the register they name, with no \
         ACTLR_EL12; under FEAT_NV2 a guest hypervisor's ACTLR_EL1 is \
         transformed whatever HCR_EL2.NV1 says",
    ),
    (
        "Programming HCR_EL2.<NV,NV2> to '10' behaves as '11'",
        false,
        "NV2 acts only with NV",
    ),
    (
        "ID_AA64DFR2_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64FPFR0_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64ISAR2_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64ISAR3_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64MMFR2_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64MMFR3_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64MMFR4_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64PFR2_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64SMFR0_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_AA64ZFR0_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_DFR1_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_ISAR6_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_MMFR4_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_MMFR5_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "ID_PFR2_EL1 trapped by HCR_EL2.TID3",
        true,
        "TID3 traps every ID register of the ID space",
    ),
    (
        "When DC is treated as NOP, data cache maintenance operations are trapped",
        true,
        "no DC is a NOP here: CLIDR_EL1 names caches to the PoC, and to the PoU but with \
         FEAT_S2FWB, where DC to the PoU is not a NOP either",
    ),
    (
        "DC to PoU is treated as a NOP",
        false,
        "DC CVAU translates its address and takes its faults where CLIDR_EL1 names no cache \
         to the PoU (FEAT_S2FWB) as where it names one",
    ),
    (
        "When IC is treated as NOP, data cache maintenance operations are trapped",
        true,
        "no IC is a NOP here: CTR_EL0.DIC is 0",
    ),
    (
        "HCR_EL2.TERR,SCR_EL3.TERR/TWERR=0 when ERRIDR_EL1.NUM==0",
        false,
        "HCR_EL2.TERR traps the error record registers though there are none",
    ),
    (
        "Trapped by MDCR_EL2.TDOSA",
        true,
        "without FEAT_DoubleLock, TDOSA traps OSDLR_EL1 as it traps the other \
         OS-related registers and as it does with the feature, as QEMU's \
         OSDLR_EL1 (`access_tdosa`) is trapped",
    ),
];

/// IMPLEMENTATION DEFINED constants the accessors read, the same on every
/// processor. `NUM_BREAKPOINTS` and `NUM_WATCHPOINTS` are each processor's
/// own, as its `ID_AA64DFR0_EL1` reports them, and the GIC CPU
/// interface's counts its `ICC_CTLR_EL1` and `ICH_VTR_EL2` report
/// ([`Eval::identifier`]).
const CONSTANTS: [(&str, i128); 1] = [
    // ERRIDR_EL1.NUM.
    ("NUM_ERROR_RECORDS", 0),
];

// ----- the specification -----

fn ty(v: &Value) -> &str {
    v.get("_type").and_then(Value::as_str).unwrap_or("")
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Arm's `Registers.json`, read once.
fn registers_json() -> &'static Value {
    static ROOT: OnceLock<Value> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = std::env::var_os(SPEC_DIR_VAR).unwrap_or_else(|| {
            panic!("set {SPEC_DIR_VAR} to the unpacked AARCHMRS directory (holding Registers.json)")
        });
        let path = PathBuf::from(dir).join("Registers.json");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    })
}

/// One instruction word an accessor gives a meaning to.
#[derive(Debug)]
struct Site {
    /// The register or instruction entry the accessor is listed under.
    register: &'static str,
    /// Arm's instruction: `A64.MRS`, `A64.MSRregister`, `A64.TLBI`, ...
    instr: &'static str,
    /// The operand as the assembler writes it.
    asm: &'static str,
    word: u32,
    /// The encoding's variables (an index, or the fields of a generic
    /// encoding) and their values in `word`.
    vars: Vec<(&'static str, i128)>,
    /// Whether the accessor applies at all.
    condition: &'static Value,
    access: &'static Value,
}

struct Spec {
    release: String,
    sites: Vec<Site>,
    /// The AArch64 registers by name, for their fieldsets.
    registers: BTreeMap<&'static str, &'static Value>,
    /// The registers the accessors' conditions read.
    inputs: BTreeSet<&'static str>,
    /// Each register's `MRS` and `MSR` (register) word of its own encoding,
    /// naming X0, by name and whether it writes.
    own: BTreeMap<(&'static str, bool), u32>,
    /// The `MRS` and `MSR` word of each EL1 or EL0 register's `*_EL12` or
    /// `*_EL02` alias (FEAT_VHE), naming X0, by the register's name and
    /// whether it writes: what reaches the register at EL2 under
    /// `HCR_EL2.E2H`, where its own name reaches EL2's.
    alias: BTreeMap<(&'static str, bool), u32>,
}

/// Add the registers condition `e` reads to `out`.
fn read_by(e: &'static Value, out: &mut BTreeSet<&'static str>) {
    match e {
        Value::Array(items) => items.iter().for_each(|i| read_by(i, out)),
        Value::Object(map) => {
            match ty(e) {
                "Types.Field" | "Types.RegisterType" => {
                    out.insert(text(&e["value"], "name"));
                }
                "AST.Function" if list(e, "arguments").is_empty() => {
                    out.insert(text(e, "name"));
                }
                _ => {}
            }
            map.values().for_each(|v| read_by(v, out));
        }
        _ => {}
    }
}

/// Add the registers every condition in accessor body `node` reads to
/// `out`.
fn conditions_read(node: &'static Value, out: &mut BTreeSet<&'static str>) {
    match node {
        Value::Array(items) => items.iter().for_each(|i| conditions_read(i, out)),
        Value::Object(map) => {
            if let Some(c) = map.get("condition") {
                read_by(c, out);
            }
            if let Some(a) = map.get("access") {
                conditions_read(a, out);
            }
        }
        _ => {}
    }
}

/// One field of an encoding: its bits, most significant first, each a
/// literal (`0`, `1`, or `x` for either) or a bit of a variable.
#[derive(Debug, Clone)]
enum Part {
    Literal(char),
    Var { name: &'static str, bit: u32 },
}

/// The parts of encoding field `value`, `width` bits wide.
fn parts(value: Option<&'static Value>, width: u32) -> Vec<Part> {
    let Some(value) = value else {
        return vec![Part::Literal('x'); width as usize];
    };
    match ty(value) {
        "Values.Value" => literal_parts(text(value, "value")),
        "Values.EquationValue" => {
            let name = text(value, "value");
            let slice = &list(value, "slice")[0];
            let start = slice["start"].as_u64().unwrap_or(0) as u32;
            let w = slice["width"].as_u64().unwrap_or(0) as u32;
            assert_eq!(w, width, "{value}");
            (0..w)
                .rev()
                .map(|i| Part::Var {
                    name,
                    bit: start + i,
                })
                .collect()
        }
        "Values.Group" => {
            // `'011':p[3]`, `m[4]:'00'`, `'10':m[4:3]`.
            let mut out = Vec::new();
            let whole = text(value, "value");
            let mut pieces = Vec::new();
            let (mut depth, mut from) = (0, 0);
            for (i, c) in whole.char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    ':' if depth == 0 => {
                        pieces.push(&whole[from..i]);
                        from = i + 1;
                    }
                    _ => {}
                }
            }
            pieces.push(&whole[from..]);
            for piece in pieces {
                if piece.starts_with('\'') {
                    out.extend(literal_parts(piece));
                } else {
                    let (name, bits) = piece.split_once('[').unwrap_or_else(|| panic!("{value}"));
                    let bits = bits.trim_end_matches(']');
                    let (hi, lo) = match bits.split_once(':') {
                        Some((h, l)) => {
                            (h.parse::<u32>().unwrap_or(0), l.parse::<u32>().unwrap_or(0))
                        }
                        None => {
                            let b = bits.parse::<u32>().unwrap_or(0);
                            (b, b)
                        }
                    };
                    out.extend((lo..=hi).rev().map(|bit| Part::Var { name, bit }));
                }
            }
            assert_eq!(out.len(), width as usize, "{value}");
            out
        }
        other => panic!("encoding value of type {other}: {value}"),
    }
}

fn literal_parts(quoted: &str) -> Vec<Part> {
    quoted
        .trim_matches('\'')
        .chars()
        .map(Part::Literal)
        .collect()
}

/// The instruction words of one accessor encoding, each with the values of
/// its variables.
fn expand(instr: &'static str, encoding: &'static Value) -> Vec<(u32, Vec<(&'static str, i128)>)> {
    let fields = &encoding["encodings"];
    let get = |name: &str| fields.get(name);
    let order: [(&str, u32); 5] = [("op0", 2), ("op1", 3), ("CRn", 4), ("CRm", 4), ("op2", 3)];
    let all: Vec<Vec<Part>> = order
        .iter()
        .map(|&(name, width)| parts(get(name), width))
        .collect();
    // Free bits: every `x` literal and every distinct variable bit.
    let mut free: Vec<Part> = Vec::new();
    for part in all.iter().flatten() {
        let known = match part {
            Part::Literal('x') => false,
            Part::Literal(_) => true,
            Part::Var { name, bit } => free
                .iter()
                .any(|f| matches!(f, Part::Var { name: n, bit: b } if n == name && b == bit)),
        };
        if !known {
            free.push(part.clone());
        }
    }
    let x_count = free
        .iter()
        .filter(|p| matches!(p, Part::Literal('x')))
        .count();
    let mut out = Vec::new();
    for combo in 0u64..(1u64 << free.len()) {
        // Assign each free bit; `x` literals are taken in order.
        let mut vars: Vec<(&'static str, i128)> = Vec::new();
        let mut xs = Vec::with_capacity(x_count);
        for (i, f) in free.iter().enumerate() {
            let bit = (combo >> i) & 1;
            match f {
                Part::Literal(_) => xs.push(bit),
                Part::Var { name, bit: b } => match vars.iter_mut().find(|(n, _)| n == name) {
                    Some((_, v)) => *v |= i128::from(bit as u8) << b,
                    None => vars.push((name, i128::from(bit as u8) << b)),
                },
            }
        }
        let mut x_iter = xs.into_iter();
        let mut values = [0u32; 5];
        for (k, field) in all.iter().enumerate() {
            let mut v = 0u32;
            for part in field {
                let bit = match part {
                    Part::Literal('0') => 0,
                    Part::Literal('1') => 1,
                    Part::Literal(_) => x_iter.next().unwrap_or(0) as u32,
                    Part::Var { name, bit } => {
                        let value = vars.iter().find(|(n, _)| n == name).map_or(0, |(_, v)| *v);
                        ((value >> bit) & 1) as u32
                    }
                };
                v = (v << 1) | bit;
            }
            values[k] = v;
        }
        let [op0, op1, crn, crm, op2] = values;
        out.push((word(instr, op0, op1, crn, crm, op2), vars));
    }
    out
}

/// The word of Arm's `instr` with these operands.
fn word(instr: &str, op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> u32 {
    let (base, l, rt) = match instr {
        "A64.MRS" => (0xD500_0000, 1, RT),
        "A64.SYSL" | "A64.GCSPOPM" | "A64.GCSSS2" | "A64.GICR" => (0xD500_0000, 1, RT),
        "A64.MRRS" => (0xD540_0000, 1, RT_PAIR),
        "A64.MSRRregister" | "A64.SYSP" | "A64.TLBIP" => (0xD540_0000, 0, RT_PAIR),
        _ => (0xD500_0000, 0, RT),
    };
    base | (l << 21) | (op0 << 19) | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt
}

fn spec() -> &'static Spec {
    static SPEC: OnceLock<Spec> = OnceLock::new();
    SPEC.get_or_init(|| {
        let root = registers_json();
        let entries = root.as_array().map_or(&[][..], Vec::as_slice);
        let release = entries
            .first()
            .and_then(|e| e.pointer("/_meta/version/ref"))
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let mut sites = Vec::new();
        let mut registers = BTreeMap::new();
        let mut inputs = BTreeSet::new();
        for entry in entries {
            if text(entry, "state") != "AArch64" {
                continue;
            }
            let name = text(entry, "name");
            registers.insert(name, entry);
            // The indexes an indexed register has (`PMEVTYPER<n>_EL0`'s 0
            // to 30): an encoding of an index past them is another
            // register's (`PMCCFILTR_EL0`'s), or none.
            let indexes: Vec<(i128, i128)> = list(entry, "indexes")
                .iter()
                .map(|r| {
                    let start = i128::from(r["start"].as_u64().unwrap_or(0));
                    (start, start + i128::from(r["width"].as_u64().unwrap_or(0)))
                })
                .collect();
            let indexed = |vars: &[(&str, i128)]| match (vars, indexes.is_empty()) {
                ([(_, index)], false) => indexes.iter().any(|&(lo, hi)| (lo..hi).contains(index)),
                _ => true,
            };
            for accessor in list(entry, "accessors") {
                // The `MSR` (immediate) forms have no accessor body: what
                // they do is the instruction's own pseudocode, which the
                // decoder and the PSTATE tests check.
                if accessor["access"].is_null() {
                    continue;
                }
                read_by(&accessor["condition"], &mut inputs);
                conditions_read(&accessor["access"], &mut inputs);
                let instr = text(accessor, "name");
                for encoding in list(accessor, "encoding") {
                    for (word, vars) in expand(instr, encoding) {
                        if !indexed(&vars) {
                            continue;
                        }
                        sites.push(Site {
                            register: name,
                            instr,
                            asm: text(encoding, "asmvalue"),
                            word,
                            vars,
                            condition: &accessor["condition"],
                            access: &accessor["access"],
                        });
                    }
                }
            }
        }
        // `AArch64_CanTrapDC` and `AArch64_CanTrapIC` read these,
        // `GetSPSR_EL1_M4` and `GetSPSR_EL2_M4`, which choose the SPSRs'
        // fieldsets, these, and `GetNumEventCountersSelfHosted` the count of
        // event counters `PMCR_EL0.N` reports.
        inputs.extend(["CLIDR_EL1", "CTR_EL0", "SPSR_EL1", "SPSR_EL2", "PMCR_EL0"]);
        // The conditions choosing the fields of the registers read.
        loop {
            let mut more = BTreeSet::new();
            for name in &inputs {
                if let Some(entry) = registers.get(name) {
                    for fieldset in list(entry, "fieldsets") {
                        read_by(&fieldset["condition"], &mut more);
                        for v in list(fieldset, "values") {
                            for f in list(v, "fields") {
                                read_by(&f["condition"], &mut more);
                            }
                        }
                    }
                }
            }
            more.retain(|name| !inputs.contains(name));
            if more.is_empty() {
                break;
            }
            inputs.extend(more);
        }
        inputs.retain(|name| registers.contains_key(name));
        let mut own = BTreeMap::new();
        let mut alias = BTreeMap::new();
        for s in &sites {
            let write = match s.instr {
                "A64.MRS" => false,
                "A64.MSRregister" => true,
                _ => continue,
            };
            if s.asm == s.register {
                own.entry((s.register, write)).or_insert(s.word & !0x1F);
            } else if s.asm.strip_suffix('2') == Some(s.register)
                && (s.register.ends_with("_EL1") || s.register.ends_with("_EL0"))
            {
                alias.entry((s.register, write)).or_insert(s.word & !0x1F);
            }
        }
        Spec {
            release,
            sites,
            registers,
            inputs,
            own,
            alias,
        }
    })
}

// ----- the interpreter -----

/// What an access does, as Arm's accessor decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The register is read or written, or the instruction performed.
    Access,
    /// FEAT_NV2's `NVMem(offset)`: a load (`write` false) or store of 64
    /// bits at `offset` in the page `VNCR_EL2` names, in place of the
    /// register's access.
    Memory { offset: u64, write: bool },
    /// `Undefined()`.
    Undefined,
    /// `AArch64_SystemAccessTrap(el, ec)`.
    Trap { el: u8, ec: u64 },
    /// An IMPLEMENTATION DEFINED register or instruction
    /// (`AArch64_ImpDefSysRegRead` and the like): UNDEFINED where the
    /// processor gives the encoding nothing; the one it gives a register or
    /// an instruction, its TRM's, is not checked here ([`check`]).
    ImplementationDefined,
}

/// An evaluated pseudocode value.
#[derive(Debug, Clone, PartialEq)]
enum V {
    Bool(bool),
    Int(i128),
    Bits {
        value: u64,
        width: u32,
    },
    /// A bitstring constant with don't-care bits, as Arm writes it.
    Pattern(String),
    El(u8),
    /// An enumeration constant.
    Name(String),
    Set(Vec<V>),
    /// A value the pseudocode leaves open (`ARBITRARY`), or one only the
    /// access itself uses.
    Open,
}

impl V {
    fn bits(value: u64, width: u32) -> Self {
        Self::Bits { value, width }
    }

    fn int(&self) -> i128 {
        match self {
            Self::Int(i) => *i,
            Self::Bits { value, .. } => i128::from(*value),
            other => panic!("{other:?} is not a number"),
        }
    }
}

/// A Values.Value constant: `'101'`, `'1x'`.
fn constant(quoted: &str) -> V {
    let body = quoted.trim_matches('\'');
    if body.contains('x') {
        return V::Pattern(body.to_string());
    }
    if quoted.starts_with('\'') {
        let value = u64::from_str_radix(body, 2).unwrap_or_else(|e| panic!("{quoted}: {e}"));
        return V::bits(value, body.len() as u32);
    }
    V::Int(
        body.parse::<i128>()
            .unwrap_or_else(|e| panic!("{quoted}: {e}")),
    )
}

fn equal(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Bits { value, width }, V::Pattern(p)) | (V::Pattern(p), V::Bits { value, width }) => {
            assert_eq!(p.len(), *width as usize, "{p} against a {width}-bit value");
            p.chars().enumerate().all(|(i, c)| {
                let bit = (value >> (*width as usize - 1 - i)) & 1;
                c == 'x' || (c == '1') == (bit == 1)
            })
        }
        (V::Bits { value: x, .. }, V::Bits { value: y, .. }) => x == y,
        (V::Int(x), V::Bits { value, .. }) | (V::Bits { value, .. }, V::Int(x)) => {
            *x == i128::from(*value)
        }
        (V::Open, _) | (_, V::Open) => panic!("comparing a value the pseudocode leaves open"),
        _ => a == b,
    }
}

/// The processor state an accessor's pseudocode reads.
struct Eval<'a> {
    spec: &'static Spec,
    /// The processor's implementation, leveled for its level set.
    imp: Implementation,
    el: u8,
    /// `PSTATE.SP`.
    sp: bool,
    /// The values of the registers whose fields the accessors read, as an
    /// `MRS` at the highest exception level reads them.
    values: &'a BTreeMap<&'static str, u64>,
    vars: &'a [(&'static str, i128)],
}

impl Eval<'_> {
    fn have_el2(&self) -> bool {
        self.imp.has(Feature::El2)
    }

    /// `ID_AA64DFR0_EL1`, as the processor advertises it.
    fn dfr0(&self) -> u64 {
        id::id_aa64dfr0(self.imp.features, self.imp.debug, self.imp.pmu)
    }

    /// Arm `NUM_PMU_COUNTERS`, `GetNumEventCountersSelfHosted` with no
    /// FEAT_PMUv3_EXTPMN: the processor's event counters, as `PMCR_EL0.N`
    /// reports them at its highest level.
    fn pmu_counters(&self) -> i128 {
        if self.feature("FEAT_PMUv3") {
            ((self.register("PMCR_EL0") >> 11) & 0x1F).into()
        } else {
            0
        }
    }

    /// Arm `GetNumEventCountersAccessible`: at EL0 and EL1 with EL2 enabled
    /// the Effective `MDCR_EL2.HPMN`, every counter otherwise. `HPMN` above
    /// the counters or zero, which Arm leaves CONSTRAINED UNPREDICTABLE
    /// without FEAT_HPMN0 (K1.2.6.3), is the counters or none, the
    /// Cortex-A57's answers (DDI 0488H B.2.20), which every processor here
    /// gives.
    fn pmu_accessible(&self) -> i128 {
        let counters = self.pmu_counters();
        if self.el < 2 && self.have_el2() {
            i128::from(self.register("MDCR_EL2") & 0x1F).min(counters)
        } else {
            counters
        }
    }

    /// `ConstrainUnpredictableProcedure(Unpredictable_PMUEVENTCOUNTER)`,
    /// reached by an access to an event counter beyond the accessor's
    /// reach, as the processor's PMU refines it: Arm's own refinement,
    /// UNDEFINED; or the Cortex-A57's (DDI 0488H B.2.22 to B.2.25), RES0
    /// but for a direct access (`m`) to a counter the processor does not
    /// implement, which is UNALLOCATED.
    fn pmu_event_counter_unpredictable(&self) -> Outcome {
        use super::pmu::CounterBeyondReach;
        let direct = self.vars.iter().find(|(n, _)| *n == "m").map(|(_, v)| *v);
        match (self.imp.pmu.beyond_reach, direct) {
            (CounterBeyondReach::Undefined, _) => Outcome::Undefined,
            (CounterBeyondReach::Res0, Some(m)) if m >= self.pmu_counters() => Outcome::Undefined,
            (CounterBeyondReach::Res0, Some(_) | None) => Outcome::Access,
        }
    }

    fn feature(&self, name: &str) -> bool {
        match name {
            "FEAT_AA64" | "FEAT_AA64EL0" | "FEAT_AA64EL1" | "FEAT_EL0" | "FEAT_EL1" | "v8Ap0" => {
                true
            }
            "FEAT_AA64EL3" | "FEAT_EL3" => false,
            // As the processor advertises it: `ID_AA64DFR0_EL1.DoubleLock`
            // 0b0000 is the OS Double Lock implemented.
            "FEAT_DoubleLock" => (self.dfr0() >> 36) & 0xF == 0,
            // `ID_AA64MMFR0_EL1.ASIDBits` 0b0010: 16-bit ASIDs.
            "FEAT_ASID16" => (id::id_aa64mmfr0(self.imp.features) >> 4) & 0xF == 2,
            // `ID_AA64MMFR4_EL1` reads as zero, its `E2H0` 0b0000: with
            // FEAT_VHE, `HCR_EL2.E2H` is programmable (`Features.json`:
            // `(FEAT_VHE && FEAT_AA64EL1) ==> (FEAT_E2H0 <=>
            // SInt(ID_AA64MMFR4_EL1.E2H0) >= 0)`).
            "FEAT_E2H0" => self.feature("FEAT_VHE"),
            // `ID_AA64PFR0_EL1.GIC` 1, the processor's interface to a
            // GICv3's CPU interface, which its board gives it.
            "FEAT_GICv3" => self.imp.gic != GicCpuInterface::Absent,
            // `ICH_HCR_EL2.TDIR`, which `ICH_VTR_EL2.TDS` reports, of the
            // processor's virtual interface.
            "FEAT_GICv3_TDIR" => match self.imp.gic {
                GicCpuInterface::SystemRegisters(interface) => {
                    interface.virtual_interface.vtr() & (1 << 19) != 0
                }
                GicCpuInterface::Absent => false,
            },
            _ => Feature::ALL
                .into_iter()
                .find(|f| f.arm_name() == Some(name))
                .is_some_and(|f| self.imp.has(f)),
        }
    }

    fn have_el(&self, el: u8) -> bool {
        match el {
            0 | 1 => true,
            2 => self.have_el2(),
            _ => false,
        }
    }

    fn register(&self, name: &str) -> u64 {
        *self.values.get(name).unwrap_or_else(|| {
            panic!("the accessors read {name}, which the test does not read the value of")
        })
    }

    fn hcr_bit(&self, field: &str) -> bool {
        self.have_el2() && self.field("HCR_EL2", field).int() == 1
    }

    /// `ELIsInHost(el)`.
    fn in_host(&self, el: u8) -> bool {
        if !self.feature("FEAT_VHE") {
            return false;
        }
        match el {
            2 => self.have_el2() && self.e2h(),
            0 => self.have_el2() && self.e2h() && self.hcr_bit("TGE"),
            _ => false,
        }
    }

    /// `EffectiveHCR_EL2_E2H()`.
    fn e2h(&self) -> bool {
        self.feature("FEAT_VHE") && (!self.feature("FEAT_E2H0") || self.hcr_bit("E2H"))
    }

    /// `EffectiveHCR_EL2_NVx()`, `NV2:NV1:NV`, on a processor whose `NV1` is
    /// writable (FEAT_E2H0, which every one here with FEAT_NV has), with
    /// `{NV, NV1}` `{0, 1}` as `{0, 0}`: the processor's choice of
    /// `ConstrainUnpredictable(Unpredictable_NVNV1)`, `Constraint_NVNV1_00`.
    fn nvx(&self) -> u64 {
        if !self.have_el2() || !self.feature("FEAT_NV") || !self.hcr_bit("NV") {
            return 0b000;
        }
        assert!(self.feature("FEAT_E2H0"), "NV1 RAZ is not modelled");
        let nv1 = u64::from(self.hcr_bit("NV1"));
        if !self.feature("FEAT_NV2") {
            return (nv1 << 1) | 1;
        }
        let nv2 = self.hcr_bit("NV2")
            || self.imp_def("Programming HCR_EL2.<NV,NV2> to '10' behaves as '11'");
        (u64::from(nv2) << 2) | (nv1 << 1) | 1
    }

    fn imp_def(&self, name: &str) -> bool {
        IMPLEMENTATION_DEFINED
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, choice, _)| *choice)
            .unwrap_or_else(|| {
                panic!("ImpDefBool(\"{name}\") has no answer in IMPLEMENTATION_DEFINED")
            })
    }

    /// Field `field` of register `reg`, by the fieldset Arm gives this
    /// processor.
    fn field(&self, reg: &str, field: &str) -> V {
        let entry = self
            .spec
            .registers
            .get(reg)
            .unwrap_or_else(|| panic!("no register {reg} in the specification"));
        // `E2H0.EL1PCTEN` names a field of the fieldset named `E2H0`.
        let (fieldset, field) = match field.split_once('.') {
            Some((set, field)) => (
                list(entry, "fieldsets")
                    .iter()
                    .find(|fs| text(fs, "name") == set)
                    .unwrap_or_else(|| panic!("{reg}: no fieldset {set}")),
                field,
            ),
            None => (
                list(entry, "fieldsets")
                    .iter()
                    .find(|fs| self.truth(&fs["condition"]))
                    .unwrap_or_else(|| panic!("{reg}: no fieldset applies")),
                field,
            ),
        };
        let value = self.register(reg);
        for v in list(fieldset, "values") {
            let ranges: &[Value] = list(v, "rangeset");
            let named = match ty(v) {
                "Fields.Field" | "Fields.ConstantField" | "Fields.Array" | "Fields.Dynamic" => {
                    text(v, "name") == field
                }
                "Fields.ConditionalField" => list(v, "fields")
                    .iter()
                    .find(|f| self.truth(&f["condition"]))
                    .is_some_and(|f| text(&f["field"], "name") == field),
                _ => false,
            };
            if named {
                let (mut out, mut width) = (0u64, 0u32);
                for r in ranges {
                    let start = r["start"].as_u64().unwrap_or(0) as u32;
                    let w = r["width"].as_u64().unwrap_or(0) as u32;
                    out = (out << w) | ((value >> start) & ((1u64 << w) - 1));
                    width += w;
                }
                return V::bits(out, width);
            }
        }
        // A field this processor's fieldset leaves reserved reads as zero.
        let reserved_width = list(fieldset, "values")
            .iter()
            .filter(|v| ty(v) == "Fields.ConditionalField")
            .find(|v| {
                list(v, "fields")
                    .iter()
                    .any(|f| text(&f["field"], "name") == field)
            })
            .map(|v| {
                list(v, "rangeset")
                    .iter()
                    .map(|r| r["width"].as_u64().unwrap_or(0) as u32)
                    .sum::<u32>()
            });
        match reserved_width {
            Some(width) => V::bits(0, width),
            None => panic!("{reg}.{field}: no such field"),
        }
    }

    fn truth(&self, cond: &Value) -> bool {
        if cond.is_null() {
            return true;
        }
        match self.eval(cond) {
            V::Bool(b) => b,
            other => panic!("condition {cond} is {other:?}"),
        }
    }

    fn eval(&self, e: &Value) -> V {
        match ty(e) {
            "AST.Bool" => V::Bool(e["value"].as_bool().unwrap_or(false)),
            "AST.Integer" => V::Int(i128::from(e["value"].as_i64().unwrap_or(0))),
            "Values.Value" => constant(text(e, "value")),
            "Types.String" => V::Name(text(e, "value").to_string()),
            "AST.Identifier" => self.identifier(text(e, "value")),
            "AST.UnaryOp" => match (text(e, "op"), self.eval(&e["expr"])) {
                ("!", V::Bool(b)) => V::Bool(!b),
                ("-", v) => V::Int(-v.int()),
                (op, v) => panic!("{op} {v:?}"),
            },
            "AST.BinaryOp" => self.binary(e),
            "AST.Set" => V::Set(list(e, "values").iter().map(|v| self.eval(v)).collect()),
            "AST.Concat" => {
                let (mut value, mut width) = (0u64, 0u32);
                for part in list(e, "values") {
                    match self.eval(part) {
                        V::Bits { value: v, width: w } => {
                            value = (value << w) | v;
                            width += w;
                        }
                        other => panic!("concatenating {other:?}"),
                    }
                }
                V::bits(value, width)
            }
            "AST.SquareOp" => {
                let base = match self.eval(&e["var"]) {
                    V::Bits { value, .. } => value,
                    other => panic!("indexing {other:?}"),
                };
                let args = list(e, "arguments");
                assert_eq!(args.len(), 1, "{e}");
                let arg = &args[0];
                if ty(arg) == "AST.Slice" {
                    let hi = self.eval(&arg["left"]).int() as u32;
                    let lo = self.eval(&arg["right"]).int() as u32;
                    let w = hi - lo + 1;
                    V::bits((base >> lo) & (u64::MAX >> (64 - w)), w)
                } else {
                    let bit = self.eval(arg).int() as u32;
                    V::bits((base >> bit) & 1, 1)
                }
            }
            "AST.TypeAnnotation" => self.eval(&e["var"]),
            "Types.Field" => {
                let f = &e["value"];
                assert!(f["instance"].is_null() && f["slices"].is_null(), "{e}");
                self.field(text(f, "name"), text(f, "field"))
            }
            "Types.RegisterType" => V::bits(self.register(text(&e["value"], "name")), 64),
            "AST.DotAtom" => {
                let parts = list(e, "values");
                assert_eq!(parts.len(), 2, "{e}");
                let field = text(&parts[1], "value");
                match (ty(&parts[0]), text(&parts[0], "value")) {
                    ("AST.Identifier", "PSTATE") => self.pstate(field),
                    ("AST.Identifier", "FGDTState") => {
                        panic!("FGDTState.{field}: fine-grained traps are not implemented")
                    }
                    ("AST.Function", _) => self.field(text(&parts[0], "name"), field),
                    _ => panic!("{e}"),
                }
            }
            "AST.Function" => self.call(text(e, "name"), list(e, "arguments")),
            other => panic!("expression of type {other}: {e}"),
        }
    }

    fn identifier(&self, name: &str) -> V {
        match name {
            "EL0" => V::El(0),
            "EL1" => V::El(1),
            "EL2" => V::El(2),
            "EL3" => V::El(3),
            "TRUE" => V::Bool(true),
            "FALSE" => V::Bool(false),
            "t" | "t2" => V::Int(i128::from(RT)),
            "ARBITRARY" => V::Open,
            // `ID_AA64DFR0_EL1.BRPs` and `WRPs`, each one less than the
            // count.
            "NUM_BREAKPOINTS" => V::Int(i128::from((self.dfr0() >> 12) & 0xF) + 1),
            "NUM_WATCHPOINTS" => V::Int(i128::from((self.dfr0() >> 20) & 0xF) + 1),
            // `ICC_CTLR_EL1.PRIbits` plus one, of a processor with the GIC
            // CPU interface's System registers; the accessors read it only
            // past their FEAT_GICv3 check.
            "NUM_GIC_PRIORITY_BITS" => match self.imp.gic {
                GicCpuInterface::SystemRegisters(interface) => {
                    V::Int(i128::from(interface.priority_bits.count()))
                }
                GicCpuInterface::Absent => V::Open,
            },
            // `ICH_VTR_EL2.PREbits` plus one and `ListRegs` plus one, of the
            // virtual interface; read past the same check.
            "NUM_GIC_PREEMPTION_BITS" => match self.imp.gic {
                GicCpuInterface::SystemRegisters(interface) => {
                    V::Int(i128::from((interface.virtual_interface.vtr() >> 26) & 7) + 1)
                }
                GicCpuInterface::Absent => V::Open,
            },
            "NUM_GIC_LIST_REGS" => match self.imp.gic {
                GicCpuInterface::SystemRegisters(interface) => {
                    V::Int(i128::from(interface.virtual_interface.vtr() & 0x1F) + 1)
                }
                GicCpuInterface::Absent => V::Open,
            },
            _ => {
                if let Some((_, v)) = self.vars.iter().find(|(n, _)| *n == name) {
                    return V::Int(*v);
                }
                if let Some((_, v)) = CONSTANTS.iter().find(|(n, _)| *n == name) {
                    return V::Int(*v);
                }
                V::Name(name.to_string())
            }
        }
    }

    fn pstate(&self, field: &str) -> V {
        match field {
            "EL" => V::El(self.el),
            "SP" => V::bits(u64::from(self.sp), 1),
            other => panic!("PSTATE.{other} is not one the test sets"),
        }
    }

    fn binary(&self, e: &Value) -> V {
        let op = text(e, "op");
        match op {
            "&&" => V::Bool(self.truth(&e["left"]) && self.truth(&e["right"])),
            "||" => V::Bool(self.truth(&e["left"]) || self.truth(&e["right"])),
            _ => {
                let l = self.eval(&e["left"]);
                let r = self.eval(&e["right"]);
                match op {
                    "==" => V::Bool(equal(&l, &r)),
                    "!=" => V::Bool(!equal(&l, &r)),
                    "IN" => match r {
                        V::Set(items) => V::Bool(items.iter().any(|i| equal(&l, i))),
                        other => panic!("IN {other:?}"),
                    },
                    "<" => V::Bool(l.int() < r.int()),
                    "<=" => V::Bool(l.int() <= r.int()),
                    ">" => V::Bool(l.int() > r.int()),
                    ">=" => V::Bool(l.int() >= r.int()),
                    "+" => V::Int(l.int() + r.int()),
                    "-" => V::Int(l.int() - r.int()),
                    "*" => V::Int(l.int() * r.int()),
                    other => panic!("operator {other}: {e}"),
                }
            }
        }
    }

    fn el_arg(&self, args: &[Value]) -> u8 {
        match self.eval(&args[0]) {
            V::El(el) => el,
            other => panic!("{other:?} is not an exception level"),
        }
    }

    fn call(&self, name: &str, args: &[Value]) -> V {
        match name {
            "IsFeatureImplemented" => V::Bool(self.feature(text(&args[0], "value"))),
            "HaveEL" => V::Bool(self.have_el(self.el_arg(args))),
            "EL2Enabled" => V::Bool(self.have_el2()),
            "ELIsInHost" => V::Bool(self.in_host(self.el_arg(args))),
            "EffectivelyAtEL0InHost" => V::Bool(self.el == 0 && self.in_host(0)),
            "EffectivelyAtEL0NotInHost" => V::Bool(self.el == 0 && !self.in_host(0)),
            "EffectiveHCR_EL2_NVx" => V::bits(self.nvx(), 3),
            // With no EL3 there is no `SCR_EL3.HXEn`.
            "IsHCRXEL2Enabled" => V::Bool(self.feature("FEAT_HCX") && self.have_el2()),
            // `HCRX_EL2.NVTGE` is FEAT_NV3's.
            "EffectiveHCRX_EL2_NVTGE" => {
                assert!(
                    !self.feature("FEAT_NV3"),
                    "FEAT_NV3 is not modelled by the test"
                );
                V::bits(0, 1)
            }
            "EL3SDDUndef" | "EL3SDDUndefPriority" | "Halted" | "HaltingAllowed" => V::Bool(false),
            "ELUsingAArch32" => V::Bool(false),
            "IsHighestEL" => {
                let highest = if self.have_el2() { 2 } else { 1 };
                V::Bool(self.el_arg(args) == highest)
            }
            "EffectiveTGEforEL0Traps" => V::bits(u64::from(self.hcr_bit("TGE")), 1),
            "SecurityStateAtEL" => V::Name("SS_NonSecure".to_string()),
            "IsCurrentSecurityState" => {
                V::Bool(self.eval(&args[0]) == V::Name("SS_NonSecure".to_string()))
            }
            "ValidSecurityStateAtEL" => V::Bool(self.have_el(self.el_arg(args))),
            // Only Non-secure state, with no FEAT_Secure, FEAT_SEL2 or
            // FEAT_RME.
            "HaveELUsingSecurityState" => {
                let el = self.el_arg(args);
                V::Bool(
                    self.have_el(el)
                        && el != 3
                        && self.eval(&args[1]) == V::Name("SS_NonSecure".to_string()),
                )
            }
            "EffectiveSCR_EL3_NS" => V::bits(1, 1),
            // No EL3 and no FEAT_RME.
            "EffectiveSCR_EL3_TERR" | "EffectiveSCR_EL3_TWERR" | "EffectiveSCR_EL3_NSE" => {
                V::bits(0, 1)
            }
            "EffectiveHCR_EL2_E2H" => V::bits(u64::from(self.e2h()), 1),
            // The shared pseudocode's, with no FEAT_GCIE: `Enable` but
            // where both RAO/WI choices hold or `SRE` reads 0.
            "EffectiveICC_SRE_EL2_Enable" => {
                let rao = self.imp_def("Treat ICC_SRE_EL2.SRE as RAO/WI")
                    && self.imp_def("Treat ICC_SRE_EL2.Enable as RAO/WI");
                let sre = self.register("ICC_SRE_EL2");
                V::bits(u64::from(rao || sre & 1 == 0 || sre & (1 << 3) != 0), 1)
            }
            "GetSPSR_EL1_M4" => V::bits((self.register("SPSR_EL1") >> 4) & 1, 1),
            "GetSPSR_EL2_M4" => V::bits((self.register("SPSR_EL2") >> 4) & 1, 1),
            "IsSCTLR2EL1Enabled" | "IsSCTLR2EL2Enabled" => {
                assert!(
                    !self.feature("FEAT_SCTLR2"),
                    "FEAT_SCTLR2 is not modelled by the test"
                );
                V::Bool(false)
            }
            "UInt" => V::Int(self.eval(&args[0]).int()),
            "IsZero" => V::Bool(self.eval(&args[0]).int() == 0),
            "GetNumEventCountersSelfHosted" => V::Int(self.pmu_counters()),
            "GetNumEventCountersAccessible" => V::Int(self.pmu_accessible()),
            "X" => V::bits(0, 64),
            "ImpDefBool" => V::Bool(self.imp_def(text(&args[0], "value"))),
            "EffectiveHCR_EL2_TERR" => {
                let records = CONSTANTS
                    .iter()
                    .find(|(n, _)| *n == "NUM_ERROR_RECORDS")
                    .map_or(0, |(_, v)| *v);
                let zero = records == 0
                    && self.imp_def("HCR_EL2.TERR,SCR_EL3.TERR/TWERR=0 when ERRIDR_EL1.NUM==0");
                V::bits(
                    u64::from(self.have_el2() && !zero && self.hcr_bit("TERR")),
                    1,
                )
            }
            "AArch64_CanTrapDC" => V::Bool(
                !self.treat_dc_as_nop(args)
                    || self.imp_def(
                        "When DC is treated as NOP, data cache maintenance operations are trapped",
                    ),
            ),
            "AArch64_TreatDCAsNOP" => V::Bool(self.treat_dc_as_nop(args)),
            "AArch64_CanTrapIC" => V::Bool(
                !self.treat_ic_as_nop()
                    || self.imp_def(
                        "When IC is treated as NOP, data cache maintenance operations are trapped",
                    ),
            ),
            "AArch64_TreatICAsNOP" => V::Bool(self.treat_ic_as_nop()),
            _ if args.is_empty() && self.spec.registers.contains_key(name) => {
                V::bits(self.register(name), 64)
            }
            _ => panic!("the test does not evaluate {name}()"),
        }
    }

    /// `AArch64_TreatDCAsNOP(cachetype, cacheop, opscope)`.
    fn treat_dc_as_nop(&self, args: &[Value]) -> bool {
        let scope = text(&args[2], "value");
        let clidr = self.register("CLIDR_EL1");
        let louis = (clidr >> 21) & 7;
        let loc = (clidr >> 24) & 7;
        let louu = (clidr >> 27) & 7;
        let cachetype = text(&args[0], "value");
        assert!(
            !(cachetype == "CacheType_Tag" && !self.feature("FEAT_MTE2")),
            "Allocation tag maintenance is not modelled by the test"
        );
        match scope {
            "CacheOpScope_PoU" => {
                louu == 0 && louis == 0 && self.imp_def("DC to PoU is treated as a NOP")
            }
            "CacheOpScope_PoC" | "CacheOpScope_PoP" | "CacheOpScope_PoDP" => {
                assert!(
                    loc != 0,
                    "a processor without caches is not modelled by the test"
                );
                false
            }
            _ => false,
        }
    }

    /// `AArch64_TreatICAsNOP()`.
    fn treat_ic_as_nop(&self) -> bool {
        let dic = (self.register("CTR_EL0") >> 29) & 1;
        assert_eq!(dic, 0, "CTR_EL0.DIC set is not modelled by the test");
        false
    }

    /// Run an accessor's decision list to the outcome it reaches.
    fn run(&self, node: &Value) -> Outcome {
        match node {
            Value::Array(items) => {
                for item in items {
                    if self.truth(&item["condition"]) {
                        return self.run(&item["access"]);
                    }
                }
                panic!("no branch of {node} applies")
            }
            _ => match ty(node) {
                "Accessors.Permission.SystemAccess" => {
                    assert!(self.truth(&node["condition"]), "{node}");
                    self.run(&node["access"])
                }
                "AST.Function" => self.statement(text(node, "name"), list(node, "arguments")),
                "AST.Assignment" => self.assignment(node),
                "AST.Return" => Outcome::Access,
                other => panic!("statement of type {other:?}: {node}"),
            },
        }
    }

    /// An assignment's outcome: a load (`X(t) = NVMem(offset)`) or store
    /// (`NVMem(offset) = X(t)`) of the `VNCR_EL2` page, or the access of a
    /// register, whichever register it names.
    fn assignment(&self, node: &Value) -> Outcome {
        let nvmem = |side: &Value| {
            (ty(side) == "AST.Function" && text(side, "name") == "NVMem").then(|| {
                let args = list(side, "arguments");
                assert_eq!(args.len(), 1, "{side}");
                self.eval(&args[0]).int() as u64
            })
        };
        match (nvmem(&node["var"]), nvmem(&node["val"])) {
            (Some(offset), None) => Outcome::Memory {
                offset,
                write: true,
            },
            (None, Some(offset)) => Outcome::Memory {
                offset,
                write: false,
            },
            (None, None) => Outcome::Access,
            (Some(_), Some(_)) => panic!("NVMem on both sides: {node}"),
        }
    }

    fn statement(&self, name: &str, args: &[Value]) -> Outcome {
        match name {
            "Undefined" => Outcome::Undefined,
            "AArch64_SystemAccessTrap" => Outcome::Trap {
                el: self.el_arg(args),
                ec: self.eval(&args[1]).int() as u64,
            },
            "AArch64_SystemAccessTraptoEL1orEL2" => {
                self.trap_to_el1_or_el2(self.eval(&args[0]).int() as u64)
            }
            "UnimplementedIDRegister" => {
                if !self.feature("FEAT_IDST") {
                    Outcome::Undefined
                } else if self.el == 0 {
                    self.trap_to_el1_or_el2(0x18)
                } else {
                    Outcome::Trap {
                        el: self.el,
                        ec: 0x18,
                    }
                }
            }
            "AArch64_ImpDefSysRegRead"
            | "AArch64_ImpDefSysRegWrite"
            | "AArch64_ImpDefSysRegRead128"
            | "AArch64_ImpDefSysRegWrite128"
            | "AArch64_ImpDefSysInstr"
            | "AArch64_ImpDefSysInstrWithResult"
            | "AArch64_ImpDefSysInstr128" => Outcome::ImplementationDefined,
            "ConstrainUnpredictableProcedure"
                if text(&args[0], "value") == "Unpredictable_PMUEVENTCOUNTER" =>
            {
                self.pmu_event_counter_unpredictable()
            }
            "AArch64_FGDTSystemAccessTrap"
            | "Halt"
            | "EXLOCKException"
            | "ConstrainUnpredictableProcedure" => {
                panic!("{name}: a path no processor here reaches")
            }
            _ => Outcome::Access,
        }
    }

    fn trap_to_el1_or_el2(&self, ec: u64) -> Outcome {
        let el = if self.hcr_bit("TGE") { 2 } else { 1 };
        Outcome::Trap { el, ec }
    }
}

// ----- the processor -----

/// The processor's RAM, and on a processor with FEAT_GICv3 the board's
/// GICv3, whose CPU interface its System registers reach: its EL2 trap
/// controls (`ICH_HCR_EL2`, `ICC_SRE_EL2.Enable`) are among the states the
/// accessors read.
struct Ram(Vec<u8>, Option<GicV3>);

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

    fn gic_register_read(&mut self, reg: GicRegister) -> Result<u64, NoGicSystemRegisters> {
        match &mut self.1 {
            Some(gic) => Ok(gic.register_read(reg)),
            None => Err(NoGicSystemRegisters),
        }
    }

    fn gic_register_write(
        &mut self,
        reg: GicRegister,
        value: u64,
    ) -> Result<(), NoGicSystemRegisters> {
        match &mut self.1 {
            Some(gic) => {
                gic.register_write(reg, value);
                Ok(())
            }
            None => Err(NoGicSystemRegisters),
        }
    }

    fn gic_el1_traps(&self) -> Result<GicEl1Traps, NoGicSystemRegisters> {
        match &self.1 {
            Some(gic) => Ok(gic.el1_traps()),
            None => Err(NoGicSystemRegisters),
        }
    }
}

/// What the processor did with a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Observed {
    Completed,
    Exception {
        el: u8,
        ec: u64,
        iss: u64,
    },
    /// It completed by a load (`write` false) or store at `offset` in the
    /// `VNCR_EL2` page, which [`VNCR_PAGE`] is.
    Memory {
        offset: u64,
        write: bool,
    },
}

/// Where `VNCR_EL2` points on every processor with FEAT_NV2 under test: a
/// page of the RAM below [`CODE`], each of whose doublewords holds
/// [`vncr_pattern`] of its offset, so a load says where it read and a
/// store where it wrote.
const VNCR_PAGE: u64 = 0;

/// The doubleword at `offset` in the `VNCR_EL2` page: a value no register
/// holds, naming its offset.
const fn vncr_pattern(offset: u64) -> u64 {
    0x5EED_0000_0000_0000 | offset
}

/// The `VNCR_EL2` page's contents, as [`processor`] fills it.
fn vncr_page() -> Vec<u8> {
    (0..0x1000u64)
        .step_by(8)
        .flat_map(|offset| vncr_pattern(offset).to_le_bytes())
        .collect()
}

/// The `VNCR_EL2` page of `ram`.
fn vncr_page_of(ram: &mut Ram) -> &mut [u8] {
    &mut ram.0[VNCR_PAGE as usize..VNCR_PAGE as usize + 0x1000]
}

/// The offset of the first doubleword of the `VNCR_EL2` page of `ram` that
/// no longer holds its pattern, if one changed, and the page put back.
fn vncr_store(ram: &mut Ram) -> Option<u64> {
    let fill = vncr_page();
    let page = vncr_page_of(ram);
    let changed = page
        .chunks(8)
        .zip(fill.chunks(8))
        .position(|(now, was)| now != was)
        .map(|slot| 8 * slot as u64);
    page.copy_from_slice(&fill);
    changed
}

/// Step `word`, whose accessor makes it a load or store of the `VNCR_EL2`
/// page if `memory` is that outcome, and say what it did: the word with
/// X1 as its register in place of XZR, so a load lands where it can be
/// read and a store stores X1, which holds a value no doubleword of the
/// page does; any other word as it is, with a store to the page reported
/// as one ([`Observed::Memory`]).
fn observe<L: Levels>(cpu: &mut Cpu<L>, ram: &mut Ram, word: u32, memory: Outcome) -> Observed {
    const STORED: u64 = 0xC0DE_0000_0000_0000;
    let (word, rt) = match memory {
        Outcome::Memory { .. } => ((word & !0x1F) | 1, Some(1)),
        _ => (word, None),
    };
    if let Some(rt) = rt {
        cpu.x[rt] = STORED;
    }
    let got = step(cpu, ram, word);
    let stored = vncr_store(ram);
    match (got, stored, rt) {
        (Observed::Completed, Some(offset), _) => Observed::Memory {
            offset,
            write: true,
        },
        (Observed::Completed, None, Some(rt)) => {
            let loaded = cpu.x[rt];
            match (0..0x1000).step_by(8).find(|&o| vncr_pattern(o) == loaded) {
                Some(offset) => Observed::Memory {
                    offset,
                    write: false,
                },
                None => Observed::Completed,
            }
        }
        (observed, _, _) => observed,
    }
}

/// Everything an access can change, to put back after each step: the
/// processor's state and the GIC's.
struct Saved<L: Levels> {
    x: [u64; 31],
    sp: [u64; 2],
    el2: L::State,
    pc: u64,
    pstate: Pstate,
    fpcr: u64,
    fpsr: u64,
    svcr: sme::Svcr,
    pac_keys: PacKeys,
    sys: SysRegs,
    pmu: Pmu,
    monitor: ExclusiveMonitor,
    event: EventRegister,
    activity: Activity,
    gic: Option<GicV3>,
}

fn save<L: Levels>(cpu: &Cpu<L>, ram: &Ram) -> Saved<L> {
    Saved {
        gic: ram.1.clone(),
        x: cpu.x,
        sp: [cpu.sp_el0, cpu.sp_el1],
        el2: cpu.el2_state.clone(),
        pc: cpu.pc,
        pstate: cpu.pstate,
        fpcr: cpu.fpcr,
        fpsr: cpu.fpsr,
        svcr: cpu.svcr,
        pac_keys: cpu.pac_keys,
        sys: cpu.sys.clone(),
        pmu: cpu.pmu.clone(),
        monitor: cpu.monitor,
        event: cpu.event,
        activity: cpu.activity,
    }
}

fn restore<L: Levels>(cpu: &mut Cpu<L>, ram: &mut Ram, s: &Saved<L>) {
    ram.1.clone_from(&s.gic);
    cpu.x = s.x;
    [cpu.sp_el0, cpu.sp_el1] = s.sp;
    cpu.el2_state = s.el2.clone();
    cpu.pc = s.pc;
    cpu.pstate = s.pstate;
    cpu.fpcr = s.fpcr;
    cpu.fpsr = s.fpsr;
    cpu.svcr = s.svcr;
    cpu.pac_keys = s.pac_keys;
    cpu.sys = s.sys.clone();
    cpu.pmu = s.pmu.clone();
    cpu.monitor = s.monitor;
    cpu.event = s.event;
    cpu.activity = s.activity;
}

fn el_number(el: El) -> u8 {
    match el {
        El::El0 => 0,
        El::El1 => 1,
        El::El2 => 2,
    }
}

/// Step `word` at [`CODE`].
fn step<L: Levels>(cpu: &mut Cpu<L>, ram: &mut Ram, word: u32) -> Observed {
    ram.0[CODE as usize..CODE as usize + 4].copy_from_slice(&word.to_le_bytes());
    cpu.pc = CODE;
    let before = cpu.pstate.el;
    assert_eq!(
        cpu.step(ram, &mut NoInstrumentation),
        Step::Retired,
        "{word:#010x}"
    );
    if cpu.pc == CODE + 4 && cpu.pstate.el == before {
        return Observed::Completed;
    }
    let (vbar, esr) = match cpu.pstate.el {
        El::El1 => (VBAR_EL1, cpu.sys.esr_el1),
        El::El2 => (VBAR_EL2, cpu.el2().esr),
        El::El0 => panic!("{word:#010x} left the processor at EL0, PC {:#x}", cpu.pc),
    };
    assert_eq!(cpu.pc & !0x7FF, vbar, "{word:#010x} went to {:#x}", cpu.pc);
    Observed::Exception {
        el: el_number(cpu.pstate.el),
        ec: esr >> 26,
        iss: esr & 0x01FF_FFFF,
    }
}

/// The syndrome `AArch64_SystemAccessTrapSyndrome` gives a trap of `word`
/// with class `ec`.
fn trap_iss(ec: u64, word: u32) -> u64 {
    let w = u64::from(word);
    match ec {
        0x18 => {
            ((w >> 19) & 3) << 20
                | ((w >> 5) & 7) << 17
                | ((w >> 16) & 7) << 14
                | ((w >> 12) & 0xF) << 10
                | (w & 0x1F) << 5
                | ((w >> 8) & 0xF) << 1
                | ((w >> 21) & 1)
        }
        0x14 => {
            ((w >> 19) & 3) << 20
                | ((w >> 5) & 7) << 17
                | ((w >> 16) & 7) << 14
                | ((w >> 12) & 0xF) << 10
                | ((w >> 1) & 0xF) << 6
                | ((w >> 8) & 0xF) << 1
                | ((w >> 21) & 1)
        }
        // `ConditionSyndrome()` in AArch64 state: CV 1, COND 0b1110.
        0x07 => 0x01E0_0000,
        _ => 0,
    }
}

/// Whether `site` writes memory when its access is performed: `DC ZVA`, and
/// FEAT_MTE's `DC GVA` and `DC GZVA`. With the stage 1 MMU off, as the
/// configurations leave it, every data access is to Device memory, which
/// `AArch64_S1HasAlignmentFaultDueToMemType` refuses them with an alignment
/// fault.
fn writes_memory(site: &Site) -> bool {
    site.instr == "A64.DC" && matches!(site.asm, "ZVA" | "GVA" | "GZVA")
}

/// What the processor must do for `outcome` at `el` with `HCR_EL2.TGE`
/// `tge`; an access that writes memory ([`writes_memory`]) takes the data
/// abort of an alignment fault on a write (DFSC 0b100001, WnR).
fn expected(outcome: Outcome, el: u8, tge: bool, word: u32, writes: bool) -> Observed {
    // Where an exception from `el` that does not name its level goes.
    let routed = if el == 2 || (el == 0 && tge) { 2 } else { 1 };
    let undefined = Observed::Exception {
        el: routed,
        ec: 0,
        iss: 0,
    };
    match outcome {
        Outcome::Access if writes => Observed::Exception {
            el: routed,
            ec: if routed == el { 0x25 } else { 0x24 },
            iss: 0b10_0001 | (1 << 6),
        },
        Outcome::Access => Observed::Completed,
        Outcome::Memory { offset, write } => Observed::Memory { offset, write },
        Outcome::Undefined | Outcome::ImplementationDefined => undefined,
        Outcome::Trap { el, ec } => Observed::Exception {
            el,
            ec,
            iss: trap_iss(ec, word),
        },
    }
}

/// A processor implementing `imp` and the levels `L` at its reset exception
/// level, with the vector bases set and the RAM it runs from.
fn processor<L: Levels>(imp: LeveledImplementation<L>) -> (Cpu<L>, Ram) {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut cpu = Cpu::<L>::leveled(
        CpuConfig {
            ips,
            reset_pc: CODE,
        },
        imp,
    );
    cpu.sys.vbar_el1 = VBAR_EL1;
    let nv2 = imp.has(Feature::Nv2);
    if let Some(el2) = cpu.el2_mut() {
        el2.vbar = VBAR_EL2;
        if nv2 {
            el2.vncr = VNCR_PAGE;
        }
    }
    // The board's GICv3, with the virtual interface of the processor's
    // shape where it has EL2, its Redistributor awake as the board's
    // firmware leaves it.
    let gic = match imp.gic {
        GicCpuInterface::SystemRegisters(interface) => {
            let el2 = L::EL2.then_some(interface.virtual_interface);
            let shape = GicShape {
                spis: GicShape::DEFAULT.spis,
                priority_bits: interface.priority_bits,
            };
            let mut gic = GicV3::with_el2(shape, GicNames::NONE, el2);
            gic.power_on_processor();
            Some(gic)
        }
        GicCpuInterface::Absent => None,
    };
    let mut ram = Ram(vec![0; RAM_BYTES], gic);
    vncr_page_of(&mut ram).copy_from_slice(&vncr_page());
    (cpu, ram)
}

/// `HCR_EL2.E2H`.
const HCR_E2H: u64 = 1 << 34;

/// The `MRS` word of register `name`'s own encoding, naming X0.
fn sysreg_word_own(name: &str) -> u32 {
    *spec()
        .own
        .get(&(name, false))
        .unwrap_or_else(|| panic!("no MRS of {name}"))
}

/// The `MRS` (`write` false) or `MSR` word that reaches register `name` at
/// the highest exception level of `cpu`: its own encoding, naming X0, but
/// at EL2 under `HCR_EL2.E2H` its `*_EL12` or `*_EL02` alias where it has
/// one, its own name reaching EL2's register there (FEAT_VHE).
fn sysreg_word<L: Levels>(cpu: &Cpu<L>, name: &str, write: bool) -> u32 {
    let spec = spec();
    let e2h = L::EL2 && cpu.el2().hcr & HCR_E2H != 0;
    let aliased = if e2h {
        spec.alias.get(&(name, write))
    } else {
        None
    };
    *aliased
        .or_else(|| spec.own.get(&(name, write)))
        .unwrap_or_else(|| panic!("no {} of {name}", if write { "MSR" } else { "MRS" }))
}

/// Read `name` at the processor's highest exception level, if it has the
/// register; the processor is left as it was.
fn read_register<L: Levels>(cpu: &mut Cpu<L>, ram: &mut Ram, name: &str) -> Option<u64> {
    let saved = save(cpu, ram);
    cpu.pstate.el = if L::EL2 { El::El2 } else { El::El1 };
    cpu.pstate.sp_sel = true;
    let word = sysreg_word(cpu, name, false);
    // A step reports the access of a register, or an exception; at the
    // highest level nothing is transformed into a load or store.
    let value = match step(cpu, ram, word) {
        Observed::Completed => Some(cpu.x[0]),
        Observed::Exception { .. } | Observed::Memory { .. } => None,
    };
    restore(cpu, ram, &saved);
    value
}

/// Write `value` to `name` at the processor's highest exception level.
fn write_register<L: Levels>(cpu: &mut Cpu<L>, ram: &mut Ram, name: &str, value: u64) {
    let saved_el = cpu.pstate.el;
    let saved_sp = cpu.pstate.sp_sel;
    cpu.pstate.el = if L::EL2 { El::El2 } else { El::El1 };
    cpu.pstate.sp_sel = true;
    cpu.x[0] = value;
    let word = sysreg_word(cpu, name, true);
    let observed = step(cpu, ram, word);
    assert_eq!(observed, Observed::Completed, "writing {name}");
    cpu.pstate.el = saved_el;
    cpu.pstate.sp_sel = saved_sp;
}

/// The registers whose fields decide accesses, which the configurations
/// set, and the bits they leave clear: the stage 1 MMU enables (an EL1 or
/// EL2 fetch would translate), `HCR_EL2.VM` and `DC` (stage 2), and
/// `MDSCR_EL1.SS` (software step). `PMUSERENR_EL0` gives EL0 the PMU, and
/// `PMSELR_EL0` selects the counter `PMXEVCNTR_EL0` and `PMXEVTYPER_EL0`
/// reach. The fine-grained trap registers (FEAT_FGT) and `HCRX_EL2`
/// (FEAT_HCX) each hold traps of single accesses. With FEAT_GICv3 and EL2,
/// `ICH_HCR_EL2`'s `TC`, `TALL0`, `TALL1` and `TDIR` and `ICC_SRE_EL2`'s
/// `Enable` trap EL1's accesses to the GIC CPU interface; the virtual
/// interface's enable, maintenance controls and `EOIcount` trap nothing,
/// and `ICC_SRE_EL2`'s other bits are RAO/WI.
const CONTROLS: [(&str, u64); 20] = [
    ("HCR_EL2", (1 << 0) | (1 << 12)),
    ("CPTR_EL2", 0),
    ("MDCR_EL2", 0),
    ("HSTR_EL2", 0),
    ("CNTHCTL_EL2", 0),
    ("HFGRTR_EL2", 0),
    ("HFGWTR_EL2", 0),
    ("HFGITR_EL2", 0),
    ("HDFGRTR_EL2", 0),
    ("HDFGWTR_EL2", 0),
    ("HCRX_EL2", 0),
    ("SCTLR_EL2", 1),
    ("CPACR_EL1", 0),
    ("SCTLR_EL1", 1),
    ("CNTKCTL_EL1", 0),
    ("MDSCR_EL1", 1),
    ("PMUSERENR_EL0", 0),
    ("PMSELR_EL0", 0),
    ("ICH_HCR_EL2", (0x1F << 27) | 0xFF),
    ("ICC_SRE_EL2", 0b111),
];

/// The value of every register the accessors' conditions read
/// ([`Spec::inputs`]) that the processor has, as an `MRS` at its highest
/// exception level reads it.
fn input_values<L: Levels>(cpu: &mut Cpu<L>, ram: &mut Ram) -> BTreeMap<&'static str, u64> {
    let mut values = BTreeMap::new();
    for &reg in &spec().inputs {
        if !has_mrs(reg) {
            continue;
        }
        if let Some(v) = read_register(cpu, ram, reg) {
            values.insert(reg, v);
        }
    }
    values
}

/// Whether register `name` has an `MRS` of its own encoding.
fn has_mrs(name: &str) -> bool {
    spec().own.contains_key(&(name, false))
}

/// `CPACR_EL1` with SIMD&FP, SVE and SME enabled at EL0 and EL1.
const CPACR_OPEN: u64 = (3 << 20) | (3 << 16) | (3 << 24);

/// `HCR_EL2.TGE` and `TIDCP`.
const HCR_TGE: u64 = 1 << 27;
const HCR_TIDCP: u64 = 1 << 20;
/// `HCR_EL2.NV`, `NV1` and `NV2` (FEAT_NV, FEAT_NV2).
const HCR_NV: u64 = 1 << 42;
const HCR_NV1: u64 = 1 << 43;
const HCR_NV2: u64 = 1 << 45;

/// One state of the controls.
#[derive(Debug, Clone)]
struct Config {
    label: String,
    writes: Vec<(&'static str, u64)>,
    os_lock: bool,
    sp: bool,
    /// Whether to step the generic IMPLEMENTATION DEFINED encodings and the
    /// encodings no accessor names, which only a few states are worth.
    sweep: bool,
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The states to test `imp` in: everything clear; SIMD&FP, SVE and SME
/// enabled ("open"); open with each bit a control keeps set alone; open with
/// the OS lock set; and random states.
fn configs<L: Levels>(imp: LeveledImplementation<L>) -> Vec<Config> {
    let (mut cpu, mut ram) = processor::<L>(imp);
    let mut variable = Vec::new();
    // Each control's bits that a write sets and clears, in each layout
    // `HCR_EL2.E2H` gives it where the processor has FEAT_VHE: a state
    // writes `HCR_EL2` first (`CONTROLS`), and the others in its layout.
    let layouts: &[u64] = if imp.has(Feature::Vhe) {
        &[0, HCR_E2H]
    } else {
        &[0]
    };
    for (name, never) in CONTROLS {
        let Some(_) = read_register(&mut cpu, &mut ram, name) else {
            continue;
        };
        let mut bits = 0;
        for &hcr in layouts {
            if L::EL2 && name != "HCR_EL2" {
                write_register(&mut cpu, &mut ram, "HCR_EL2", hcr);
            }
            write_register(&mut cpu, &mut ram, name, !never);
            let ones = read_register(&mut cpu, &mut ram, name).unwrap_or(0);
            write_register(&mut cpu, &mut ram, name, 0);
            let zeros = read_register(&mut cpu, &mut ram, name).unwrap_or(0);
            bits |= (ones ^ zeros) & !never;
        }
        if L::EL2 {
            write_register(&mut cpu, &mut ram, "HCR_EL2", 0);
        }
        variable.push((name, bits));
    }
    // Open: SIMD&FP, SVE and SME enabled at EL0 and EL1 by `CPACR_EL1`, and
    // with `HCR_EL2.E2H` by `CPTR_EL2`'s enables at the same bits, which
    // its other layout keeps none of.
    let open_value = |name: &str| match name {
        "CPACR_EL1" | "CPTR_EL2" => CPACR_OPEN,
        _ => 0,
    };
    let open = |label: String| Config {
        label,
        writes: variable
            .iter()
            .map(|&(name, _)| (name, open_value(name)))
            .collect(),
        os_lock: false,
        sp: true,
        sweep: false,
    };
    let mut out = vec![
        Config {
            label: "all clear".to_string(),
            writes: variable.iter().map(|&(name, _)| (name, 0)).collect(),
            os_lock: false,
            sp: true,
            sweep: true,
        },
        Config {
            sweep: true,
            ..open("open".to_string())
        },
        Config {
            os_lock: true,
            ..open("open, OS lock set".to_string())
        },
        Config {
            sp: false,
            ..open("open, SP_EL0 selected".to_string())
        },
    ];
    for &(name, mask) in &variable {
        for bit in 0..64 {
            if mask & (1 << bit) == 0 {
                continue;
            }
            let mut c = open(format!("open, {name} bit {bit}"));
            for w in c.writes.iter_mut().filter(|w| w.0 == name) {
                w.1 ^= 1 << bit;
            }
            c.sweep = name == "HCR_EL2" && 1u64 << bit == HCR_TIDCP;
            out.push(c);
        }
    }
    // With FEAT_VHE, the same open states under `HCR_EL2.E2H`, and in the
    // host (`E2H` and `TGE`), with each bit of another control set alone:
    // the layouts `E2H` gives `CPTR_EL2`, `CNTHCTL_EL2` and `SCTLR_EL2`,
    // and the controls of EL0 in the host.
    if imp.has(Feature::Vhe) {
        for (label, hcr) in [("E2H", HCR_E2H), ("host", HCR_E2H | HCR_TGE)] {
            let under = |label: String| {
                let mut c = open(label);
                for w in c.writes.iter_mut().filter(|w| w.0 == "HCR_EL2") {
                    w.1 = hcr;
                }
                c
            };
            out.push(Config {
                sweep: true,
                ..under(format!("{label}, open"))
            });
            for &(name, mask) in variable.iter().filter(|&&(name, _)| name != "HCR_EL2") {
                for bit in (0..64).filter(|bit| mask & (1 << bit) != 0) {
                    let mut c = under(format!("{label}, open, {name} bit {bit}"));
                    for w in c.writes.iter_mut().filter(|w| w.0 == name) {
                        w.1 ^= 1 << bit;
                    }
                    out.push(c);
                }
            }
        }
    }
    // With FEAT_NV, the open states of a guest hypervisor at EL1: under each
    // `HCR_EL2.{NV2, NV1, NV}` it runs with, with `E2H` clear and, with
    // FEAT_VHE, set, alone and with each other bit a control keeps set
    // alone (`HCR_EL2`'s own among them, but `E2H`, `TGE` and the `NV`
    // bits themselves), whose traps the accessors order around the guest
    // hypervisor's own outcomes. The fine-grained trap registers' many
    // bits, each trapping one register as `HCR_EL2`'s do groups of them,
    // are set alone under the two states that transform the most: {1, 1,
    // 1} with `E2H` clear and {1, 0, 1} with it set (without FEAT_NV2,
    // {0, 1, 1} and {0, 0, 1}).
    if imp.has(Feature::Nv) {
        let nested: &[u64] = if imp.has(Feature::Nv2) {
            &[
                HCR_NV,
                HCR_NV | HCR_NV1,
                HCR_NV | HCR_NV2,
                HCR_NV | HCR_NV1 | HCR_NV2,
            ]
        } else {
            &[HCR_NV, HCR_NV | HCR_NV1]
        };
        let most = |hcr: u64| {
            let deepest = nested[nested.len() - 1];
            hcr == deepest || hcr == (deepest & !HCR_NV1) | HCR_E2H
        };
        let fine_grained = |name: &str| name.starts_with("HFG") || name.starts_with("HDFG");
        let fixed = HCR_E2H | HCR_TGE | HCR_NV | HCR_NV1 | HCR_NV2;
        for &nv in nested {
            for &e2h in layouts {
                let hcr = nv | e2h;
                let under = |label: String| {
                    let mut c = open(label);
                    for w in c.writes.iter_mut().filter(|w| w.0 == "HCR_EL2") {
                        w.1 = hcr;
                    }
                    c
                };
                out.push(Config {
                    sweep: true,
                    ..under(format!("HCR_EL2 {hcr:#x}, open"))
                });
                for &(name, mask) in &variable {
                    let mask = match name {
                        "HCR_EL2" => mask & !fixed,
                        _ if fine_grained(name) && !most(hcr) => 0,
                        _ => mask,
                    };
                    for bit in (0..64).filter(|bit| mask & (1 << bit) != 0) {
                        let mut c = under(format!("HCR_EL2 {hcr:#x}, open, {name} bit {bit}"));
                        for w in c.writes.iter_mut().filter(|w| w.0 == name) {
                            w.1 ^= 1 << bit;
                        }
                        out.push(c);
                    }
                }
            }
        }
    }
    let features = imp.features.bits();
    let mut seed = 0x5EED_0000_0000_0000 ^ features as u64 ^ (features >> 64) as u64;
    for i in 0..12 {
        let writes = variable
            .iter()
            .map(|&(name, mask)| (name, splitmix(&mut seed) & mask))
            .collect();
        let r = splitmix(&mut seed);
        out.push(Config {
            label: format!("random {i}"),
            writes,
            os_lock: r & 1 == 1,
            sp: r & 2 == 2,
            sweep: i < 2,
        });
    }
    out
}

/// One disagreement between the processor and Arm's accessor.
struct Mismatch {
    imp: &'static str,
    el: u8,
    config: String,
    register: &'static str,
    instr: &'static str,
    asm: &'static str,
    word: u32,
    want: Observed,
    got: Observed,
}

/// The processors without EL2, each implementation with the features of
/// that level set, which [`Eval`] reads. The Armv9.2 model lacks
/// FEAT_DoubleLock, which every other has. Each model is here a second
/// time with the System register interface to a GICv3's CPU interface
/// (FEAT_GICv3), as a board with a GICv3 builds it: the Cortex-A57's of
/// its five priority bits, the Armv9.2 model's of eight, every active
/// priority register its own.
const WITHOUT_EL2: [(&str, LeveledImplementation<NoEl2>); 5] = [
    (
        "Cortex-A57",
        Implementation::CORTEX_A57.with_levels::<NoEl2>(),
    ),
    ("Armv9.2", Implementation::ARMV9P2.with_levels::<NoEl2>()),
    (
        "every feature",
        Implementation::EVERY_FEATURE.with_levels::<NoEl2>(),
    ),
    (
        "Cortex-A57 with a GICv3",
        Implementation::CORTEX_A57
            .with_gic(A57_GIC)
            .with_levels::<NoEl2>(),
    ),
    (
        "Armv9.2 with a GICv3",
        Implementation::ARMV9P2
            .with_gic(ARMV9P2_GIC)
            .with_levels::<NoEl2>(),
    ),
];

/// The Cortex-A57's GIC CPU interface: five priority bits, and its virtual
/// interface (DDI 0488H Table 8-13).
const A57_GIC: GicCpuInterface = GicCpuInterface::SystemRegisters(SystemRegisterInterface {
    priority_bits: PriorityBits::Five,
    virtual_interface: VirtualInterfaceShape::CORTEX_A57,
});

/// The Armv9.2 model's on a board whose GIC has eight priority bits, every
/// active priority register its own, and the model's virtual interface of
/// seven priority bits.
const ARMV9P2_GIC: GicCpuInterface = GicCpuInterface::SystemRegisters(SystemRegisterInterface {
    priority_bits: PriorityBits::Eight,
    virtual_interface: VirtualInterfaceShape::armv9p2(PriorityBits::Eight),
});

/// The processors with EL2, as [`WITHOUT_EL2`]: each model with its GIC
/// CPU interface's virtualization support too, the Cortex-A57's
/// `ICH_HCR_EL2` without `TDIR` and four List registers, the Armv9.2
/// model's with `TDIR`, sixteen List registers and every active priority
/// register.
const WITH_EL2: [(&str, LeveledImplementation<WithEl2>); 5] = [
    (
        "Cortex-A57 with EL2",
        Implementation::CORTEX_A57.with_levels::<WithEl2>(),
    ),
    (
        "Armv9.2 with EL2",
        Implementation::ARMV9P2.with_levels::<WithEl2>(),
    ),
    (
        "every feature with EL2",
        Implementation::EVERY_FEATURE.with_levels::<WithEl2>(),
    ),
    (
        "Cortex-A57 with EL2 and a GICv3",
        Implementation::CORTEX_A57
            .with_gic(A57_GIC)
            .with_levels::<WithEl2>(),
    ),
    (
        "Armv9.2 with EL2 and a GICv3",
        Implementation::ARMV9P2
            .with_gic(ARMV9P2_GIC)
            .with_levels::<WithEl2>(),
    ),
];

/// What an encoding no accessor names does, by the Arm ARM's prose, which
/// `Registers.json` does not carry: a read of an unallocated encoding of the
/// ID register space (`op0=3, op1=0, CRn=0, CRm=1..7`) reads as zero at
/// EL1 and above (D18.2, "reserved ID registers are RAZ"), with
/// `HCR_EL2.TID3` trapping EL1's (for these reserved encodings, the
/// IMPLEMENTATION DEFINED choice this model makes); with FEAT_IDST, a read
/// of the feature ID space (`op0=3, op1=0, 1 or 3, CRn=0, CRm=0..7`) that
/// would otherwise be UNDEFINED is `UnimplementedIDRegister()`; every other
/// encoding is UNDEFINED.
fn unnamed_outcome(eval: &Eval<'_>, word: u32) -> Outcome {
    let read = (word >> 21) & 1 == 1;
    let op0 = (word >> 19) & 3;
    let op1 = (word >> 16) & 7;
    let crn = (word >> 12) & 15;
    let crm = (word >> 8) & 15;
    let id_space = read && op0 == 3 && matches!(op1, 0 | 1 | 3) && crn == 0 && crm < 8;
    let reserved_id = id_space && op1 == 0 && crm >= 1;
    if reserved_id && eval.el == 1 && eval.hcr_bit("TID3") {
        Outcome::Trap { el: 2, ec: 0x18 }
    } else if reserved_id && eval.el != 0 {
        Outcome::Access
    } else if id_space {
        eval.statement("UnimplementedIDRegister", &[])
    } else {
        Outcome::Undefined
    }
}

/// Whether a site is one of the generic encodings of the IMPLEMENTATION
/// DEFINED space, which [`Config::sweep`] states alone step.
fn generic(site: &Site) -> bool {
    site.register.starts_with("S1_<") || site.register.starts_with("S3_<")
}

/// Check every site on `imp` at `el` in `config`, adding what disagrees to
/// `mismatches`; the number of words stepped.
fn check<L: Levels>(
    name: &'static str,
    imp: LeveledImplementation<L>,
    el: u8,
    config: &Config,
    unnamed: &[u32],
    mismatches: &mut Vec<Mismatch>,
) -> usize {
    let spec = spec();
    let (mut cpu, mut ram) = processor::<L>(imp);
    for &(reg, value) in &config.writes {
        write_register(&mut cpu, &mut ram, reg, value);
    }
    if config.os_lock {
        write_register(&mut cpu, &mut ram, "OSLAR_EL1", 1);
    }
    let values = input_values(&mut cpu, &mut ram);
    let tge = values.get("HCR_EL2").is_some_and(|h| h & HCR_TGE != 0);
    if el == 1 && tge {
        // With HCR_EL2.{E2H, TGE} = {0, 1} nothing executes at EL1.
        return 0;
    }
    cpu.pstate.el = match el {
        0 => El::El0,
        1 => El::El1,
        _ => El::El2,
    };
    cpu.pstate.sp_sel = el != 0 && config.sp;
    let saved = save(&cpu, &ram);
    // The outcome each word's applicable accessors give it.
    let mut want: BTreeMap<u32, (Outcome, &Site)> = BTreeMap::new();
    for site in &spec.sites {
        if generic(site) && !config.sweep {
            continue;
        }
        let eval = Eval {
            spec,
            imp: imp.get(),
            el,
            sp: cpu.pstate.sp_sel,
            values: &values,
            vars: &site.vars,
        };
        if !eval.truth(site.condition) {
            continue;
        }
        let outcome = eval.run(site.access);
        // Where Arm's accessor hands EL1's or EL2's access to the
        // implementation and the processor gives the encoding a register or
        // an instruction of its own (the Cortex-A57's), its TRM decides what
        // the access does: `impdef.rs`, checked by `tests/unit/impdef.rs`.
        let own = imp.impdef.implements(super::sysreg::SysRegKey::from_insn(site.word));
        if outcome == Outcome::ImplementationDefined && el != 0 && own {
            continue;
        }
        match want.get(&site.word) {
            Some((earlier, other)) if *earlier != outcome => panic!(
                "{name}, EL{el}, {}: {} ({}) and {} ({}) give {:#010x} different outcomes: {earlier:?}, {outcome:?}",
                config.label, other.register, other.asm, site.register, site.asm, site.word
            ),
            Some(_) => {}
            None => {
                want.insert(site.word, (outcome, site));
            }
        }
    }
    let mut stepped = 0;
    for (&word, &(outcome, site)) in &want {
        let got = observe(&mut cpu, &mut ram, word, outcome);
        restore(&mut cpu, &mut ram, &saved);
        stepped += 1;
        let w = expected(outcome, el, tge, word, writes_memory(site));
        if got != w {
            mismatches.push(Mismatch {
                imp: name,
                el,
                config: config.label.clone(),
                register: site.register,
                instr: site.instr,
                asm: site.asm,
                word,
                want: w,
                got,
            });
        }
    }
    if config.sweep {
        let eval = Eval {
            spec,
            imp: imp.get(),
            el,
            sp: cpu.pstate.sp_sel,
            values: &values,
            vars: &[],
        };
        for &word in unnamed {
            let got = observe(&mut cpu, &mut ram, word, Outcome::Undefined);
            restore(&mut cpu, &mut ram, &saved);
            stepped += 1;
            let want = expected(unnamed_outcome(&eval, word), el, tge, word, false);
            if got != want {
                mismatches.push(Mismatch {
                    imp: name,
                    el,
                    config: config.label.clone(),
                    register: "(no accessor)",
                    instr: "",
                    asm: "",
                    word,
                    want,
                    got,
                });
            }
        }
    }
    stepped
}

/// What one processor's thread did.
struct Tally {
    stepped: usize,
    states: usize,
    mismatches: Vec<Mismatch>,
}

/// [`check`] `imp`, of the level set `L`, in each of its [`configs`] at each
/// exception level it has.
fn tally<L: Levels>(name: &'static str, imp: LeveledImplementation<L>, unnamed: &[u32]) -> Tally {
    let mut tally = Tally {
        stepped: 0,
        states: 0,
        mismatches: Vec::new(),
    };
    let els: &[u8] = if L::EL2 { &[0, 1, 2] } else { &[0, 1] };
    for config in configs::<L>(imp) {
        tally.states += 1;
        for &el in els {
            tally.stepped += check::<L>(name, imp, el, &config, unnamed, &mut tally.mismatches);
        }
    }
    tally
}

/// Each processor, at each exception level it has and in each state of the
/// controls, takes the exception Arm's accessor pseudocode takes for every
/// System register and System instruction encoding, with Arm's syndrome, or
/// performs the access where the accessor does; and an encoding of the
/// System register and System instruction spaces no accessor names is
/// UNDEFINED.
#[test]
#[cfg_attr(
    not(aarchmrs),
    ignore = "reads Arm's AARCHMRS from the directory RUSTY_BOX_AARCHMRS names"
)]
fn every_system_access_takes_the_exception_arms_accessor_takes() {
    let spec = spec();
    let named: BTreeSet<u32> = spec.sites.iter().map(|s| s.word & !0x1F).collect();
    // The MRS, MSR (register), SYS and SYSL words no accessor names.
    let mut unnamed = Vec::new();
    for l in 0..2u32 {
        for op0 in 1..4u32 {
            for rest in 0..(1u32 << 14) {
                let w = 0xD500_0000 | (l << 21) | (op0 << 19) | (rest << 5) | RT;
                if !named.contains(&(w & !0x1F)) {
                    unnamed.push(w);
                }
            }
        }
    }
    let started = std::time::Instant::now();
    let results: Vec<Tally> = std::thread::scope(|scope| {
        let unnamed = &unnamed;
        let without =
            WITHOUT_EL2.map(|(name, imp)| scope.spawn(move || tally::<NoEl2>(name, imp, unnamed)));
        let with =
            WITH_EL2.map(|(name, imp)| scope.spawn(move || tally::<WithEl2>(name, imp, unnamed)));
        let handles: Vec<_> = without.into_iter().chain(with).collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    let stepped: usize = results.iter().map(|r| r.stepped).sum();
    let states: Vec<usize> = results.iter().map(|r| r.states).collect();
    let mismatches: Vec<&Mismatch> = results.iter().flat_map(|r| r.mismatches.iter()).collect();
    std::eprintln!(
        "AARCHMRS {}: {} accessor sites, {} unnamed words, {states:?} states, {stepped} steps, {} disagreements, \
         {:.1} s",
        spec.release,
        spec.sites.len(),
        unnamed.len(),
        mismatches.len(),
        started.elapsed().as_secs_f64()
    );
    // One line per register, instruction and kind of disagreement.
    let mut seen = BTreeMap::new();
    for m in &mismatches {
        let key = format!(
            "{} {} {} | {}: want {:?} got {:?}",
            m.register, m.instr, m.asm, m.imp, m.want, m.got
        );
        seen.entry(key)
            .or_insert_with(|| format!("EL{} {} {:#010x}", m.el, m.config, m.word));
    }
    let report: Vec<String> = seen
        .iter()
        .map(|(k, v)| format!("{k} (first: {v})"))
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} disagreements:\n{}",
        mismatches.len(),
        report.join("\n")
    );
}

// ----- the fields -----

/// What a bit of a register reads after a write, by its fieldset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BitRule {
    /// A bit of a field: it reads back as written.
    Kept,
    /// RES0, RAZ: it reads 0.
    Zero,
    /// RES1, RAO: it reads 1.
    One,
    /// A constant field: it reads the same whatever is written.
    Fixed,
    /// The architecture leaves it open (UNKNOWN, IMPLEMENTATION DEFINED), or
    /// [`FIELD_RULES`] says why it does not read back as written.
    Open,
}

/// A field of an EL2 register whose whole rule the data does not give.
struct FieldRule {
    register: &'static str,
    /// The field's name, or `*` for every field of the register.
    field: &'static str,
    rule: BitRule,
    /// The feature whose processors the rule leaves to the data, which
    /// gives their field whole; `None` where it covers every processor.
    unless: Option<Feature>,
}

/// The fields of the EL2 registers whose whole rule the data does not
/// give, each with where its rule comes from. Every processor with EL2 has
/// `SMIDR_EL1.SMPS` = 0.
const FIELD_RULES: [FieldRule; 5] = [
    // Read-only: whether the timer condition is met
    // (AArch64_CheckTimerConditions).
    FieldRule {
        register: "CNTHP_CTL_EL2",
        field: "ISTATUS",
        rule: BitRule::Open,
        unless: None,
    },
    FieldRule {
        register: "CNTHV_CTL_EL2",
        field: "ISTATUS",
        rule: BitRule::Open,
        unless: None,
    },
    // The register description makes DS RES0 without FEAT_LPA2; the data's
    // fallback field for that case carries no values.
    FieldRule {
        register: "TCR_EL2",
        field: "DS",
        rule: BitRule::Zero,
        unless: Some(Feature::Lpa2),
    },
    // The register description: when SMIDR_EL1.SMPS is 0, this register is
    // RES0.
    FieldRule {
        register: "SMPRIMAP_EL2",
        field: "*",
        rule: BitRule::Zero,
        unless: None,
    },
    // The field's description (Arm ARM D24.2, `HCRX_EL2`): when
    // SMIDR_EL1.SMPS is 0, this field is RES0.
    FieldRule {
        register: "HCRX_EL2",
        field: "SMPME",
        rule: BitRule::Zero,
        unless: None,
    },
];

/// The rule of each bit of `reg`'s fieldset, for the processor `eval`
/// describes.
fn bit_rules(eval: &Eval<'_>, reg: &str) -> [BitRule; 64] {
    let entry = spec().registers[reg];
    let fieldset = list(entry, "fieldsets")
        .iter()
        .find(|fs| eval.truth(&fs["condition"]))
        .unwrap_or_else(|| panic!("{reg}: no fieldset applies"));
    let mut rules = [BitRule::Open; 64];
    apply_fields(eval, reg, list(fieldset, "values"), 0, &mut rules);
    rules
}

/// Set the rules of the bits `values` (a fieldset's, or a dynamic field's
/// instance's) describe, their ranges counted from bit `base`.
fn apply_fields(
    eval: &Eval<'_>,
    reg: &str,
    values: &'static [Value],
    base: usize,
    rules: &mut [BitRule; 64],
) {
    let mut set = |ranges: &[Value], rule: BitRule| {
        for r in ranges {
            let start = base + r["start"].as_u64().unwrap_or(0) as usize;
            let width = r["width"].as_u64().unwrap_or(0) as usize;
            rules[start..start + width].fill(rule);
        }
    };
    let field_rule = |field: &Value| -> BitRule {
        let name = text(field, "name");
        let covers = |rule: &&FieldRule| {
            rule.register == reg
                && (rule.field == name || rule.field == "*")
                && !rule.unless.is_some_and(|feature| eval.imp.has(feature))
        };
        if let Some(FieldRule { rule, .. }) = FIELD_RULES.iter().find(covers) {
            return *rule;
        }
        match ty(field) {
            "Fields.Field" | "Fields.Array" => BitRule::Kept,
            "Fields.ConstantField" => BitRule::Fixed,
            "Fields.Reserved" => reserved_rule(text(field, "value")),
            _ => BitRule::Open,
        }
    };
    let mut dynamic = Vec::new();
    for v in values {
        let ranges = list(v, "rangeset");
        match ty(v) {
            "Fields.ConditionalField" => {
                let rule = match list(v, "fields")
                    .iter()
                    .find(|f| eval.truth(&f["condition"]))
                {
                    Some(f) => field_rule(&f["field"]),
                    None => reserved_rule(text(v, "reservedtype")),
                };
                set(ranges, rule);
            }
            // A field whose layout the features decide: the one instance
            // whose condition holds, its ranges counted from the field's own
            // first bit. One with several layouts that hold at once, which
            // the register's own value chooses between (ESR_ELx.ISS, by
            // EC), or whose conditions read a register, holds whatever an
            // all-ones or all-zeros write puts in it.
            "Fields.Dynamic" => {
                let holding: Vec<&Value> = if reads_registers(v) {
                    Vec::new()
                } else {
                    list(v, "instances")
                        .iter()
                        .filter(|i| eval.truth(&i["condition"]))
                        .collect()
                };
                match holding[..] {
                    [instance] => {
                        assert_eq!(ranges.len(), 1, "{reg}.{}", text(v, "name"));
                        let start = base + ranges[0]["start"].as_u64().unwrap_or(0) as usize;
                        dynamic.push((list(instance, "values"), start));
                    }
                    _ => set(ranges, BitRule::Open),
                }
            }
            _ => set(ranges, field_rule(v)),
        }
    }
    for (values, start) in dynamic {
        apply_fields(eval, reg, values, start, rules);
    }
}

/// Whether any instance condition of dynamic field `v` reads a register.
fn reads_registers(v: &'static Value) -> bool {
    let mut read = BTreeSet::new();
    for instance in list(v, "instances") {
        read_by(&instance["condition"], &mut read);
    }
    !read.is_empty()
}

fn reserved_rule(kind: &str) -> BitRule {
    match kind {
        "RES0" | "RAZ" | "RAZ/WI" => BitRule::Zero,
        "RES1" | "RAO" | "RAO/WI" => BitRule::One,
        _ => BitRule::Open,
    }
}

/// Every EL2 register of each processor with EL2, written at EL2 with all
/// ones and then all zeros, reads back each bit of its fields as written,
/// each RES0 bit as 0 and each RES1 bit as 1, and its constant fields
/// unchanged, by the fieldset Arm's data gives the processor's features:
/// with `HCR_EL2.E2H` clear, and with FEAT_VHE set too, which gives the
/// registers it redefines their other layout. A bit of one of those that is
/// a field in the other layout reads back as written in both (Arm ARM
/// Glossary, a bit reserved only in some contexts).
#[test]
#[cfg_attr(
    not(aarchmrs),
    ignore = "reads Arm's AARCHMRS from the directory RUSTY_BOX_AARCHMRS names"
)]
fn each_el2_register_keeps_the_bits_its_fieldset_gives_it() {
    use super::sysreg::SysRegKey;

    let spec = spec();
    let mut failures = Vec::new();
    let mut checked = 0;
    let hcr_key = SysRegKey::from_insn(sysreg_word_own("HCR_EL2"));
    let with_hcr = WITH_EL2.into_iter().flat_map(|(name, imp)| {
        let layouts: &[u64] = if imp.has(Feature::Vhe) {
            &[0, HCR_E2H]
        } else {
            &[0]
        };
        layouts.iter().map(move |&hcr| (name, imp, hcr))
    });
    for (name, imp, hcr) in with_hcr {
        let set_hcr_to = |cpu: &mut Cpu<WithEl2>, hcr: u64| {
            cpu.pstate.el = El::El2;
            match cpu.sysreg_write(hcr_key, hcr) {
                Ok(effect) => cpu.apply_write_effect(effect),
                Err(e) => panic!("{name}: writing HCR_EL2: {e:?}"),
            }
        };
        let set_hcr = |cpu: &mut Cpu<WithEl2>| set_hcr_to(cpu, hcr);
        let (mut reader, mut ram) = processor::<WithEl2>(imp);
        set_hcr(&mut reader);
        let values = input_values(&mut reader, &mut ram);
        let eval = Eval {
            spec,
            imp: imp.get(),
            el: 2,
            sp: true,
            values: &values,
            vars: &[],
        };
        // The other `E2H` layout, whose fields the registers it redefines
        // keep too.
        let other_values = imp.has(Feature::Vhe).then(|| {
            let (mut other, mut ram) = processor::<WithEl2>(imp);
            set_hcr_to(&mut other, hcr ^ HCR_E2H);
            input_values(&mut other, &mut ram)
        });
        let other_eval = other_values.as_ref().map(|values| Eval {
            spec,
            imp: imp.get(),
            el: 2,
            sp: true,
            values,
            vars: &[],
        });
        for (&(reg, write), &word) in &spec.own {
            // The EL2 registers: `op0` 3, `op1` 4.
            if write || (word >> 16) & 0x1F != 0b11_100 || !spec.own.contains_key(&(reg, true)) {
                continue;
            }
            // `HCR_EL2` itself, whose all-ones write would change the layout
            // the evaluation reads, is checked in the first pass alone.
            if hcr != 0 && reg == "HCR_EL2" {
                continue;
            }
            let (mut cpu, _) = processor::<WithEl2>(imp);
            set_hcr(&mut cpu);
            let key = SysRegKey::from_insn(word);
            if cpu.sysreg_read(key).is_err() {
                continue;
            }
            let mut rules = bit_rules(&eval, reg);
            if let Some(other) = &other_eval {
                if ["SCTLR_EL2", "CPTR_EL2", "TCR_EL2", "CNTHCTL_EL2"].contains(&reg) {
                    for (rule, there) in rules.iter_mut().zip(bit_rules(other, reg)) {
                        if matches!(there, BitRule::Kept) {
                            *rule = BitRule::Kept;
                        }
                    }
                }
            }
            let mut read_after = |value: u64| {
                match cpu.sysreg_write(key, value) {
                    Ok(effect) => cpu.apply_write_effect(effect),
                    Err(e) => panic!("{name}: writing {reg}: {e:?}"),
                }
                cpu.sysreg_read(key)
                    .unwrap_or_else(|e| panic!("{name}: reading {reg}: {e:?}"))
            };
            let ones = read_after(u64::MAX);
            let zeros = read_after(0);
            checked += 1;
            for (bit, rule) in rules.iter().enumerate() {
                let (one, zero) = ((ones >> bit) & 1, (zeros >> bit) & 1);
                let good = match rule {
                    BitRule::Kept => one == 1 && zero == 0,
                    BitRule::Zero => one == 0 && zero == 0,
                    BitRule::One => one == 1 && zero == 1,
                    BitRule::Fixed => one == zero,
                    BitRule::Open => true,
                };
                if !good {
                    failures.push(format!(
                        "{name}, HCR_EL2 {hcr:#x}: {reg} bit {bit} is {rule:?} but reads {one} after all ones, {zero} after all zeros"
                    ));
                }
            }
        }
    }
    std::eprintln!(
        "AARCHMRS {}: {checked} EL2 registers, {} disagreements",
        spec.release,
        failures.len()
    );
    assert!(checked > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
