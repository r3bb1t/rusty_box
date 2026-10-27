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
//!   access, an UNDEFINED exception, or a trap to an exception level with a
//!   class. The processor stepping the word must take the same exception,
//!   with the syndrome `AArch64_SystemAccessTrapSyndrome` gives it, or
//!   none. An encoding no accessor names is UNDEFINED.
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

use super::features::{Feature, Implementation};
use super::{
    id, sme, Activity, Cpu, CpuConfig, El, ExclusiveMonitor, Levels, NoEl2, Pstate, Step, SysRegs, WithEl2,
};
use super::pauth::PacKeys;
use super::wait::EventRegister;
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
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
const IMPLEMENTATION_DEFINED: [(&str, bool, &str); 22] = [
    ("Implementation can reset into AArch32 state", false, "there is no AArch32"),
    ("IMPLEMENTED_TRCIMSPEC<n>", false, "there is no trace unit"),
    ("AArch64-AMEVCNTR1_EL0[n] is fixed", false, "there are no activity monitors"),
    (
        "IMPLEMENTED_ACTLR_ELx accessor behavior",
        false,
        "the ACTLR_ELx encodings reach the register they name; the choice \
         matters only with FEAT_VHE or FEAT_NV",
    ),
    ("ID_AA64DFR2_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64FPFR0_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64ISAR2_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64ISAR3_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64MMFR2_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64MMFR3_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64MMFR4_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64PFR2_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64SMFR0_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_AA64ZFR0_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_DFR1_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_ISAR6_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_MMFR4_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_MMFR5_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    ("ID_PFR2_EL1 trapped by HCR_EL2.TID3", true, "TID3 traps every ID register of the ID space"),
    (
        "When DC is treated as NOP, data cache maintenance operations are trapped",
        true,
        "no DC is a NOP here: CLIDR_EL1 reports the Cortex-A57's caches",
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
];

/// IMPLEMENTATION DEFINED constants the accessors read.
const CONSTANTS: [(&str, i128); 3] = [
    // Two breakpoints and two watchpoints, as ID_AA64DFR0_EL1 reports.
    ("NUM_BREAKPOINTS", 2),
    ("NUM_WATCHPOINTS", 2),
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
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// Arm's `Registers.json`, read once.
fn registers_json() -> &'static Value {
    static ROOT: OnceLock<Value> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = std::env::var_os(SPEC_DIR_VAR)
            .unwrap_or_else(|| panic!("set {SPEC_DIR_VAR} to the unpacked AARCHMRS directory (holding Registers.json)"));
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
            (0..w).rev().map(|i| Part::Var { name, bit: start + i }).collect()
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
                        Some((h, l)) => (h.parse::<u32>().unwrap_or(0), l.parse::<u32>().unwrap_or(0)),
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
    quoted.trim_matches('\'').chars().map(Part::Literal).collect()
}

/// The instruction words of one accessor encoding, each with the values of
/// its variables.
fn expand(instr: &'static str, encoding: &'static Value) -> Vec<(u32, Vec<(&'static str, i128)>)> {
    let fields = &encoding["encodings"];
    let get = |name: &str| fields.get(name);
    let order: [(&str, u32); 5] = [("op0", 2), ("op1", 3), ("CRn", 4), ("CRm", 4), ("op2", 3)];
    let all: Vec<Vec<Part>> = order.iter().map(|&(name, width)| parts(get(name), width)).collect();
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
    let x_count = free.iter().filter(|p| matches!(p, Part::Literal('x'))).count();
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
        // `AArch64_CanTrapDC` and `AArch64_CanTrapIC` read these, and
        // `GetSPSR_EL1_M4` and `GetSPSR_EL2_M4`, which choose the SPSRs'
        // fieldsets, these.
        inputs.extend(["CLIDR_EL1", "CTR_EL0", "SPSR_EL1", "SPSR_EL2"]);
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
        for s in &sites {
            let write = match s.instr {
                "A64.MRS" => false,
                "A64.MSRregister" => true,
                _ => continue,
            };
            if s.asm == s.register {
                own.entry((s.register, write)).or_insert(s.word & !0x1F);
            }
        }
        Spec {
            release,
            sites,
            registers,
            inputs,
            own,
        }
    })
}

// ----- the interpreter -----

/// What an access does, as Arm's accessor decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The register is read or written, or the instruction performed.
    Access,
    /// `Undefined()`.
    Undefined,
    /// `AArch64_SystemAccessTrap(el, ec)`.
    Trap { el: u8, ec: u64 },
    /// An IMPLEMENTATION DEFINED register or instruction
    /// (`AArch64_ImpDefSysRegRead` and the like): this processor implements
    /// none, so the access is UNDEFINED.
    ImplementationDefined,
}

/// An evaluated pseudocode value.
#[derive(Debug, Clone, PartialEq)]
enum V {
    Bool(bool),
    Int(i128),
    Bits { value: u64, width: u32 },
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
    V::Int(body.parse::<i128>().unwrap_or_else(|e| panic!("{quoted}: {e}")))
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
        (V::Int(x), V::Bits { value, .. }) | (V::Bits { value, .. }, V::Int(x)) => *x == i128::from(*value),
        (V::Open, _) | (_, V::Open) => panic!("comparing a value the pseudocode leaves open"),
        _ => a == b,
    }
}

/// The processor state an accessor's pseudocode reads.
struct Eval<'a> {
    spec: &'static Spec,
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

    fn feature(&self, name: &str) -> bool {
        match name {
            "FEAT_AA64" | "FEAT_AA64EL0" | "FEAT_AA64EL1" | "FEAT_EL0" | "FEAT_EL1" | "v8Ap0" => true,
            "FEAT_AA64EL3" | "FEAT_EL3" => false,
            // As the processor advertises it: `ID_AA64DFR0_EL1.DoubleLock`
            // 0b0000 is the OS Double Lock implemented.
            "FEAT_DoubleLock" => (id::id_aa64dfr0(self.imp.features) >> 36) & 0xF == 0,
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
        *self
            .values
            .get(name)
            .unwrap_or_else(|| panic!("the accessors read {name}, which the test does not read the value of"))
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

    fn imp_def(&self, name: &str) -> bool {
        IMPLEMENTATION_DEFINED
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, choice, _)| *choice)
            .unwrap_or_else(|| panic!("ImpDefBool(\"{name}\") has no answer in IMPLEMENTATION_DEFINED"))
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
                "Fields.Field" | "Fields.ConstantField" | "Fields.Array" | "Fields.Dynamic" => text(v, "name") == field,
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
            .find(|v| list(v, "fields").iter().any(|f| text(&f["field"], "name") == field))
            .map(|v| list(v, "rangeset").iter().map(|r| r["width"].as_u64().unwrap_or(0) as u32).sum::<u32>());
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
            "EffectiveHCR_EL2_NVx" => {
                assert!(!self.feature("FEAT_NV"), "FEAT_NV is not modelled by the test");
                V::bits(0, 3)
            }
            "EffectiveHCRX_EL2_NVTGE" | "IsHCRXEL2Enabled" => {
                assert!(!self.feature("FEAT_HCX"), "FEAT_HCX is not modelled by the test");
                if name == "IsHCRXEL2Enabled" {
                    V::Bool(false)
                } else {
                    V::bits(0, 1)
                }
            }
            "EL3SDDUndef" | "EL3SDDUndefPriority" | "Halted" | "HaltingAllowed" => V::Bool(false),
            "ELUsingAArch32" => V::Bool(false),
            "IsHighestEL" => {
                let highest = if self.have_el2() { 2 } else { 1 };
                V::Bool(self.el_arg(args) == highest)
            }
            "EffectiveTGEforEL0Traps" => V::bits(u64::from(self.hcr_bit("TGE")), 1),
            "SecurityStateAtEL" => V::Name("SS_NonSecure".to_string()),
            "IsCurrentSecurityState" => V::Bool(self.eval(&args[0]) == V::Name("SS_NonSecure".to_string())),
            "ValidSecurityStateAtEL" => V::Bool(self.have_el(self.el_arg(args))),
            // Only Non-secure state, with no FEAT_Secure, FEAT_SEL2 or
            // FEAT_RME.
            "HaveELUsingSecurityState" => {
                let el = self.el_arg(args);
                V::Bool(self.have_el(el) && el != 3 && self.eval(&args[1]) == V::Name("SS_NonSecure".to_string()))
            }
            "EffectiveSCR_EL3_NS" => V::bits(1, 1),
            // No EL3 and no FEAT_RME.
            "EffectiveSCR_EL3_TERR" | "EffectiveSCR_EL3_TWERR" | "EffectiveSCR_EL3_NSE" => V::bits(0, 1),
            "EffectiveHCR_EL2_E2H" => V::bits(u64::from(self.e2h()), 1),
            "GetSPSR_EL1_M4" => V::bits((self.register("SPSR_EL1") >> 4) & 1, 1),
            "GetSPSR_EL2_M4" => V::bits((self.register("SPSR_EL2") >> 4) & 1, 1),
            "IsSCTLR2EL1Enabled" | "IsSCTLR2EL2Enabled" => {
                assert!(!self.feature("FEAT_SCTLR2"), "FEAT_SCTLR2 is not modelled by the test");
                V::Bool(false)
            }
            "UInt" => V::Int(self.eval(&args[0]).int()),
            "IsZero" => V::Bool(self.eval(&args[0]).int() == 0),
            "X" => V::bits(0, 64),
            "ImpDefBool" => V::Bool(self.imp_def(text(&args[0], "value"))),
            "EffectiveHCR_EL2_TERR" => {
                let records = CONSTANTS.iter().find(|(n, _)| *n == "NUM_ERROR_RECORDS").map_or(0, |(_, v)| *v);
                let zero = records == 0 && self.imp_def("HCR_EL2.TERR,SCR_EL3.TERR/TWERR=0 when ERRIDR_EL1.NUM==0");
                V::bits(u64::from(self.have_el2() && !zero && self.hcr_bit("TERR")), 1)
            }
            "AArch64_CanTrapDC" => V::Bool(
                !self.treat_dc_as_nop(args)
                    || self.imp_def("When DC is treated as NOP, data cache maintenance operations are trapped"),
            ),
            "AArch64_TreatDCAsNOP" => V::Bool(self.treat_dc_as_nop(args)),
            "AArch64_CanTrapIC" => V::Bool(
                !self.treat_ic_as_nop()
                    || self.imp_def("When IC is treated as NOP, data cache maintenance operations are trapped"),
            ),
            "AArch64_TreatICAsNOP" => V::Bool(self.treat_ic_as_nop()),
            _ if args.is_empty() && self.spec.registers.contains_key(name) => V::bits(self.register(name), 64),
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
            "CacheOpScope_PoU" => louu == 0 && louis == 0,
            "CacheOpScope_PoC" | "CacheOpScope_PoP" | "CacheOpScope_PoDP" => {
                assert!(loc != 0, "a processor without caches is not modelled by the test");
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
                "AST.Assignment" | "AST.Return" => Outcome::Access,
                other => panic!("statement of type {other:?}: {node}"),
            },
        }
    }

    fn statement(&self, name: &str, args: &[Value]) -> Outcome {
        match name {
            "Undefined" => Outcome::Undefined,
            "AArch64_SystemAccessTrap" => Outcome::Trap {
                el: self.el_arg(args),
                ec: self.eval(&args[1]).int() as u64,
            },
            "AArch64_SystemAccessTraptoEL1orEL2" => self.trap_to_el1_or_el2(self.eval(&args[0]).int() as u64),
            "UnimplementedIDRegister" => {
                if !self.feature("FEAT_IDST") {
                    Outcome::Undefined
                } else if self.el == 0 {
                    self.trap_to_el1_or_el2(0x18)
                } else {
                    Outcome::Trap { el: self.el, ec: 0x18 }
                }
            }
            "AArch64_ImpDefSysRegRead"
            | "AArch64_ImpDefSysRegWrite"
            | "AArch64_ImpDefSysRegRead128"
            | "AArch64_ImpDefSysRegWrite128"
            | "AArch64_ImpDefSysInstr"
            | "AArch64_ImpDefSysInstrWithResult"
            | "AArch64_ImpDefSysInstr128" => Outcome::ImplementationDefined,
            "AArch64_FGDTSystemAccessTrap" | "Halt" | "EXLOCKException" | "ConstrainUnpredictableProcedure" => {
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

struct Ram(Vec<u8>);

impl PhysBus for Ram {
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, _clock: VmClock) -> Result<(), BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get_mut(a..a + n).ok_or(BusFault::Unmapped)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

/// What the processor did with a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Observed {
    Completed,
    Exception { el: u8, ec: u64, iss: u64 },
}

/// Everything an access can change, to put back after each step.
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
    monitor: ExclusiveMonitor,
    event: EventRegister,
    activity: Activity,
}

fn save<L: Levels>(cpu: &Cpu<L>) -> Saved<L> {
    Saved {
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
        monitor: cpu.monitor,
        event: cpu.event,
        activity: cpu.activity,
    }
}

fn restore<L: Levels>(cpu: &mut Cpu<L>, s: &Saved<L>) {
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
    assert_eq!(cpu.step(ram, &mut NoInstrumentation), Step::Retired, "{word:#010x}");
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
    let undefined = Observed::Exception { el: routed, ec: 0, iss: 0 };
    match outcome {
        Outcome::Access if writes => Observed::Exception {
            el: routed,
            ec: if routed == el { 0x25 } else { 0x24 },
            iss: 0b10_0001 | (1 << 6),
        },
        Outcome::Access => Observed::Completed,
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
fn processor<L: Levels>(imp: Implementation) -> (Cpu<L>, Ram) {
    let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
    let mut cpu = Cpu::<L>::implementing(
        CpuConfig {
            ips,
            counter_hz: ips,
            reset_pc: CODE,
        },
        imp,
    );
    assert_eq!(cpu.imp, imp, "the implementation names the level set");
    cpu.sys.vbar_el1 = VBAR_EL1;
    if let Some(el2) = cpu.el2_mut() {
        el2.vbar = VBAR_EL2;
    }
    (cpu, Ram(vec![0; RAM_BYTES]))
}

/// The `MRS` (`write` false) or `MSR` word of register `name`'s own
/// encoding, naming X0.
fn sysreg_word(name: &str, write: bool) -> u32 {
    *spec()
        .own
        .get(&(name, write))
        .unwrap_or_else(|| panic!("no {} of {name}", if write { "MSR" } else { "MRS" }))
}

/// Read `name` at the processor's highest exception level, if it has the
/// register; the processor is left as it was.
fn read_register<L: Levels>(cpu: &mut Cpu<L>, ram: &mut Ram, name: &str) -> Option<u64> {
    let saved = save(cpu);
    cpu.pstate.el = if L::EL2 { El::El2 } else { El::El1 };
    cpu.pstate.sp_sel = true;
    let value = match step(cpu, ram, sysreg_word(name, false)) {
        Observed::Completed => Some(cpu.x[0]),
        Observed::Exception { .. } => None,
    };
    restore(cpu, &saved);
    value
}

/// Write `value` to `name` at the processor's highest exception level.
fn write_register<L: Levels>(cpu: &mut Cpu<L>, ram: &mut Ram, name: &str, value: u64) {
    let saved_el = cpu.pstate.el;
    let saved_sp = cpu.pstate.sp_sel;
    cpu.pstate.el = if L::EL2 { El::El2 } else { El::El1 };
    cpu.pstate.sp_sel = true;
    cpu.x[0] = value;
    let observed = step(cpu, ram, sysreg_word(name, true));
    assert_eq!(observed, Observed::Completed, "writing {name}");
    cpu.pstate.el = saved_el;
    cpu.pstate.sp_sel = saved_sp;
}

/// The registers whose fields decide accesses, which the configurations
/// set, and the bits they leave clear: the stage 1 MMU enables (an EL1 or
/// EL2 fetch would translate), `HCR_EL2.VM` and `DC` (stage 2), and
/// `MDSCR_EL1.SS` (software step).
const CONTROLS: [(&str, u64); 10] = [
    ("HCR_EL2", (1 << 0) | (1 << 12)),
    ("CPTR_EL2", 0),
    ("MDCR_EL2", 0),
    ("HSTR_EL2", 0),
    ("CNTHCTL_EL2", 0),
    ("SCTLR_EL2", 1),
    ("CPACR_EL1", 0),
    ("SCTLR_EL1", 1),
    ("CNTKCTL_EL1", 0),
    ("MDSCR_EL1", 1),
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
fn configs<L: Levels>(imp: Implementation) -> Vec<Config> {
    let (mut cpu, mut ram) = processor::<L>(imp);
    let mut variable = Vec::new();
    for (name, never) in CONTROLS {
        let Some(_) = read_register(&mut cpu, &mut ram, name) else {
            continue;
        };
        write_register(&mut cpu, &mut ram, name, !never);
        let ones = read_register(&mut cpu, &mut ram, name).unwrap_or(0);
        write_register(&mut cpu, &mut ram, name, 0);
        let zeros = read_register(&mut cpu, &mut ram, name).unwrap_or(0);
        variable.push((name, (ones ^ zeros) & !never));
    }
    let open = |label: String| Config {
        label,
        writes: variable
            .iter()
            .map(|&(name, _)| (name, if name == "CPACR_EL1" { CPACR_OPEN } else { 0 }))
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
    let features = imp.features.bits();
    let mut seed = 0x5EED_0000_0000_0000 ^ features as u64 ^ (features >> 64) as u64;
    for i in 0..12 {
        let writes = variable.iter().map(|&(name, mask)| (name, splitmix(&mut seed) & mask)).collect();
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

/// The processors, each implementation with the features of its level set,
/// which [`Eval`] reads.
const IMPLEMENTATIONS: [(&str, Implementation); 4] = [
    ("Cortex-A57", Implementation::CORTEX_A57.with_levels::<NoEl2>()),
    ("Cortex-A57 with EL2", Implementation::CORTEX_A57.with_levels::<WithEl2>()),
    ("every feature", Implementation::EVERY_FEATURE.with_levels::<NoEl2>()),
    ("every feature with EL2", Implementation::EVERY_FEATURE.with_levels::<WithEl2>()),
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
    imp: Implementation,
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
    let saved = save(&cpu);
    // The outcome each word's applicable accessors give it.
    let mut want: BTreeMap<u32, (Outcome, &Site)> = BTreeMap::new();
    for site in &spec.sites {
        if generic(site) && !config.sweep {
            continue;
        }
        let eval = Eval {
            spec,
            imp,
            el,
            sp: cpu.pstate.sp_sel,
            values: &values,
            vars: &site.vars,
        };
        if !eval.truth(site.condition) {
            continue;
        }
        let outcome = eval.run(site.access);
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
        let got = step(&mut cpu, &mut ram, word);
        restore(&mut cpu, &saved);
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
            imp,
            el,
            sp: cpu.pstate.sp_sel,
            values: &values,
            vars: &[],
        };
        for &word in unnamed {
            let got = step(&mut cpu, &mut ram, word);
            restore(&mut cpu, &saved);
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
fn tally<L: Levels>(name: &'static str, imp: Implementation, unnamed: &[u32]) -> Tally {
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
#[cfg_attr(not(aarchmrs), ignore = "reads Arm's AARCHMRS from the directory RUSTY_BOX_AARCHMRS names")]
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
        let handles: Vec<_> = IMPLEMENTATIONS
            .iter()
            .map(|&(name, imp)| {
                let unnamed = &unnamed;
                scope.spawn(move || {
                    if imp.has(Feature::El2) {
                        tally::<WithEl2>(name, imp, unnamed)
                    } else {
                        tally::<NoEl2>(name, imp, unnamed)
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
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
            m.register,
            m.instr,
            m.asm,
            m.imp,
            m.want,
            m.got
        );
        seen.entry(key).or_insert_with(|| format!("EL{} {} {:#010x}", m.el, m.config, m.word));
    }
    let report: Vec<String> = seen.iter().map(|(k, v)| format!("{k} (first: {v})")).collect();
    assert!(mismatches.is_empty(), "{} disagreements:\n{}", mismatches.len(), report.join("\n"));
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

/// The fields of the EL2 registers whose whole rule the data does not
/// give, each with the rule and where it comes from. Both processors with
/// EL2 have `SMIDR_EL1.SMPS` = 0 and no FEAT_LPA2, so each rule holds for
/// both.
const FIELD_RULES: [(&str, &str, BitRule, &str); 3] = [
    (
        "CNTHP_CTL_EL2",
        "ISTATUS",
        BitRule::Open,
        "read-only: whether the timer condition is met (AArch64_CheckTimerConditions)",
    ),
    (
        "TCR_EL2",
        "DS",
        BitRule::Zero,
        "the register description makes DS RES0 without FEAT_LPA2; the data's \
         fallback field for that case carries no values",
    ),
    (
        "SMPRIMAP_EL2",
        "*",
        BitRule::Zero,
        "the register description: when SMIDR_EL1.SMPS is 0, this register is RES0",
    ),
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
fn apply_fields(eval: &Eval<'_>, reg: &str, values: &'static [Value], base: usize, rules: &mut [BitRule; 64]) {
    let mut set = |ranges: &[Value], rule: BitRule| {
        for r in ranges {
            let start = base + r["start"].as_u64().unwrap_or(0) as usize;
            let width = r["width"].as_u64().unwrap_or(0) as usize;
            rules[start..start + width].fill(rule);
        }
    };
    let field_rule = |field: &Value| -> BitRule {
        let name = text(field, "name");
        if let Some((_, _, rule, _)) = FIELD_RULES.iter().find(|(r, f, _, _)| *r == reg && (*f == name || *f == "*")) {
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
                let rule = match list(v, "fields").iter().find(|f| eval.truth(&f["condition"])) {
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
                    list(v, "instances").iter().filter(|i| eval.truth(&i["condition"])).collect()
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
/// unchanged, by the fieldset Arm's data gives the processor's features.
#[test]
#[cfg_attr(not(aarchmrs), ignore = "reads Arm's AARCHMRS from the directory RUSTY_BOX_AARCHMRS names")]
fn each_el2_register_keeps_the_bits_its_fieldset_gives_it() {
    use super::sysreg::SysRegKey;

    let spec = spec();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (name, imp) in IMPLEMENTATIONS.into_iter().filter(|(_, imp)| imp.has(Feature::El2)) {
        let (mut reader, mut ram) = processor::<WithEl2>(imp);
        let values = input_values(&mut reader, &mut ram);
        let eval = Eval {
            spec,
            imp,
            el: 2,
            sp: true,
            values: &values,
            vars: &[],
        };
        for (&(reg, write), &word) in &spec.own {
            // The EL2 registers: `op0` 3, `op1` 4.
            if write || (word >> 16) & 0x1F != 0b11_100 || !spec.own.contains_key(&(reg, true)) {
                continue;
            }
            let (mut cpu, _) = processor::<WithEl2>(imp);
            cpu.pstate.el = El::El2;
            let key = SysRegKey::from_insn(word);
            if cpu.sysreg_read(key).is_err() {
                continue;
            }
            let rules = bit_rules(&eval, reg);
            let mut read_after = |value: u64| {
                match cpu.sysreg_write(key, value) {
                    Ok(effect) => cpu.apply_write_effect(effect),
                    Err(e) => panic!("{name}: writing {reg}: {e:?}"),
                }
                cpu.sysreg_read(key).unwrap_or_else(|e| panic!("{name}: reading {reg}: {e:?}"))
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
                        "{name}: {reg} bit {bit} is {rule:?} but reads {one} after all ones, {zero} after all zeros"
                    ));
                }
            }
        }
    }
    std::eprintln!("AARCHMRS {}: {checked} EL2 registers, {} disagreements", spec.release, failures.len());
    assert!(checked > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
