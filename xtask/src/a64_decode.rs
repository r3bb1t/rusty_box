//! `cargo xtask a64-decode`: compile the AArch64 interpreter's encoding list
//! (`rusty_box_aarch64/src/cpu/isa/encodings.txt`) into its table and
//! decision tree (`rusty_box_aarch64/src/cpu/isa/table.rs`).
//!
//! The list is a sequence of sections, each followed by its rows:
//!
//! ```text
//! class AddSubImmediate  sf:1 op:1 S:1 10001 shift:2 imm12:12 Rn:5 Rd:5
//! x_0_0_10001_xx_xxxxxxxxxxxx_xxxxx_xxxxx  ADD
//! hint
//! 11010101000000110010xxxxxxx11111  HINT
//! absent Fp16
//! 00011110111xxxxx001010xxxxxxxxxx  FADD
//! ```
//!
//! A `class` section names a `Class` variant and the Arm ARM bit diagram of
//! that encoding class (fixed bits, and `name:width` fields, from bit 31
//! down). Each of its rows is a pattern of `0`, `1` and `x` split by `_` at
//! the diagram's field boundaries, and must agree with every bit the diagram
//! fixes. `hint` holds the one HINT-space row; `absent` names a `Feature`
//! this interpreter has no handler for. `#` starts a comment.
//!
//! The generator refuses a list in which two rows have the same fixed bits,
//! or in which two rows a word can match fix the same number of bits: the
//! decoder's rule is that the matching row with the most fixed bits wins, and
//! that rule must always name exactly one row.

use std::fs;
use std::path::PathBuf;

/// The encoding list, relative to the repository root.
pub const SPEC: &str = "rusty_box_aarch64/src/cpu/isa/encodings.txt";
/// The generated table, relative to the repository root.
pub const TABLE: &str = "rusty_box_aarch64/src/cpu/isa/table.rs";

/// `cargo xtask a64-decode [--check]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct A64DecodeCommand {
    /// Compare the generated table with the checked-in one instead of
    /// writing it.
    pub check: bool,
}

pub fn parse_args(args: &[String]) -> Result<A64DecodeCommand, String> {
    let mut command = A64DecodeCommand { check: false };
    for arg in args {
        match arg.as_str() {
            "--check" => command.check = true,
            other => return Err(format!("unknown a64-decode option {other:?}\n{}", usage())),
        }
    }
    Ok(command)
}

pub fn usage() -> &'static str {
    "Usage: cargo xtask a64-decode [--check]\n  \
     Compile rusty_box_aarch64/src/cpu/isa/encodings.txt into table.rs beside it;\n  \
     --check fails instead when table.rs differs from what the list compiles to."
}

pub fn execute(command: A64DecodeCommand) -> Result<(), String> {
    let root = repo_root()?;
    let spec_path = root.join(SPEC);
    let spec = fs::read_to_string(&spec_path).map_err(|e| format!("{}: {e}", spec_path.display()))?;
    let generated = generate(&spec)?;
    let table_path = root.join(TABLE);
    if command.check {
        let current = fs::read_to_string(&table_path).map_err(|e| format!("{}: {e}", table_path.display()))?;
        if current != generated {
            return Err(format!(
                "{TABLE} is not what {SPEC} compiles to; run `cargo xtask a64-decode` and commit the result"
            ));
        }
        println!("{TABLE} is current");
    } else {
        fs::write(&table_path, generated).map_err(|e| format!("{}: {e}", table_path.display()))?;
        println!("wrote {TABLE}");
    }
    Ok(())
}

fn repo_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "xtask manifest directory has no parent".to_string())
}

/// A class's bit diagram: the bits it fixes and its field widths, from bit 31.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Diagram {
    opcode: u32,
    mask: u32,
    widths: Vec<usize>,
}

/// What the rows of a section are.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Section {
    Class { name: String, diagram: Diagram },
    Hint,
    Absent { feature: String },
}

/// What one row decodes to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RowKind {
    Class(String),
    Hint,
    Absent(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    opcode: u32,
    mask: u32,
    mnemonic: String,
    kind: RowKind,
    /// The 1-based line of the list it came from.
    line: usize,
}

impl Row {
    fn fixed_bits(&self) -> u32 {
        self.mask.count_ones()
    }

    /// Whether some word is an instance of both rows.
    fn overlaps(&self, other: &Row) -> bool {
        (self.opcode ^ other.opcode) & self.mask & other.mask == 0
    }
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric())
}

fn parse_diagram(tokens: &[&str], line: usize) -> Result<Diagram, String> {
    let mut opcode = 0u32;
    let mut mask = 0u32;
    let mut widths = Vec::new();
    let mut bit = 32usize;
    for token in tokens {
        let width = match token.split_once(':') {
            Some((name, width)) => {
                if !is_identifier(name) {
                    return Err(format!("line {line}: field {token:?} has no name"));
                }
                width
                    .parse::<usize>()
                    .ok()
                    .filter(|w| (1..=32).contains(w))
                    .ok_or_else(|| format!("line {line}: field {token:?} has no width"))?
            }
            None => {
                if token.is_empty() || !token.chars().all(|c| c == '0' || c == '1') {
                    return Err(format!("line {line}: {token:?} is neither a field nor fixed bits"));
                }
                if token.len() > bit {
                    return Err(format!("line {line}: the diagram is longer than 32 bits"));
                }
                for (i, c) in token.chars().enumerate() {
                    let b = bit - 1 - i;
                    mask |= 1 << b;
                    if c == '1' {
                        opcode |= 1 << b;
                    }
                }
                token.len()
            }
        };
        if width > bit {
            return Err(format!("line {line}: the diagram is longer than 32 bits"));
        }
        bit -= width;
        widths.push(width);
    }
    if bit != 0 {
        return Err(format!("line {line}: the diagram covers {} bits, not 32", 32 - bit));
    }
    Ok(Diagram { opcode, mask, widths })
}

fn parse_pattern(text: &str, section: &Section, line: usize) -> Result<(u32, u32), String> {
    let groups: Vec<&str> = text.split('_').collect();
    if let Section::Class { diagram, .. } = section {
        let widths: Vec<usize> = groups.iter().map(|g| g.len()).collect();
        if widths != diagram.widths {
            return Err(format!(
                "line {line}: {text:?} is not split at the class diagram's fields {:?}",
                diagram.widths
            ));
        }
    }
    let bits: String = groups.concat();
    if bits.len() != 32 {
        return Err(format!("line {line}: {text:?} is {} bits, not 32", bits.len()));
    }
    let mut opcode = 0u32;
    let mut mask = 0u32;
    for (i, c) in bits.chars().enumerate() {
        let b = 31 - i;
        match c {
            '0' => mask |= 1 << b,
            '1' => {
                mask |= 1 << b;
                opcode |= 1 << b;
            }
            'x' => {}
            other => return Err(format!("line {line}: {other:?} in {text:?} is not 0, 1 or x")),
        }
    }
    if let Section::Class { name, diagram } = section {
        if mask & diagram.mask != diagram.mask || (opcode ^ diagram.opcode) & diagram.mask != 0 {
            return Err(format!(
                "line {line}: {text:?} leaves the bits class {name}'s diagram fixes ({:#010x}/{:#010x})",
                diagram.opcode, diagram.mask
            ));
        }
    }
    Ok((opcode, mask))
}

fn parse(spec: &str) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    let mut section: Option<Section> = None;
    let mut classes_seen: Vec<String> = Vec::new();
    let mut features_seen: Vec<String> = Vec::new();
    let mut hint_seen = false;
    for (index, raw) in spec.lines().enumerate() {
        let line = index + 1;
        let text = raw.split('#').next().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = text.split_whitespace().collect();
        match tokens[0] {
            "class" => {
                let name = tokens
                    .get(1)
                    .filter(|n| is_identifier(n))
                    .ok_or_else(|| format!("line {line}: `class` needs a class name"))?;
                if classes_seen.iter().any(|c| c == name) {
                    return Err(format!("line {line}: class {name} has a section already"));
                }
                classes_seen.push(name.to_string());
                let diagram = parse_diagram(&tokens[2..], line)?;
                section = Some(Section::Class {
                    name: name.to_string(),
                    diagram,
                });
            }
            "hint" => {
                if tokens.len() != 1 || hint_seen {
                    return Err(format!("line {line}: there is one `hint` section, and it takes no arguments"));
                }
                hint_seen = true;
                section = Some(Section::Hint);
            }
            "absent" => {
                let feature = match tokens.as_slice() {
                    [_, feature] if is_identifier(feature) => feature.to_string(),
                    _ => return Err(format!("line {line}: `absent` takes one feature name")),
                };
                if features_seen.contains(&feature) {
                    return Err(format!("line {line}: feature {feature} has a section already"));
                }
                features_seen.push(feature.clone());
                section = Some(Section::Absent { feature });
            }
            pattern => {
                let current = section
                    .as_ref()
                    .ok_or_else(|| format!("line {line}: a row before any section"))?;
                let mnemonic = match tokens.as_slice() {
                    [_, mnemonic] => mnemonic.to_string(),
                    _ => return Err(format!("line {line}: a row is a pattern and a mnemonic")),
                };
                if !mnemonic.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '/') {
                    return Err(format!("line {line}: {mnemonic:?} is not a mnemonic"));
                }
                let (opcode, mask) = parse_pattern(pattern, current, line)?;
                let kind = match current {
                    Section::Class { name, .. } => RowKind::Class(name.clone()),
                    Section::Hint => RowKind::Hint,
                    Section::Absent { feature } => RowKind::Absent(feature.clone()),
                };
                rows.push(Row {
                    opcode,
                    mask,
                    mnemonic,
                    kind,
                    line,
                });
            }
        }
    }
    let hints = rows.iter().filter(|r| r.kind == RowKind::Hint).count();
    if hints != 1 {
        return Err(format!("the list has {hints} HINT rows; it needs exactly one"));
    }
    Ok(rows)
}

/// Every word must name at most one row under "most fixed bits wins".
fn validate(rows: &[Row]) -> Result<(), String> {
    for (i, a) in rows.iter().enumerate() {
        for b in &rows[i + 1..] {
            if a.opcode == b.opcode && a.mask == b.mask {
                return Err(format!(
                    "lines {} and {} are the same encoding ({:#010x}/{:#010x})",
                    a.line, b.line, a.opcode, a.mask
                ));
            }
            if a.overlaps(b) && a.fixed_bits() == b.fixed_bits() {
                return Err(format!(
                    "lines {} and {} overlap and fix the same number of bits, so a word in both names neither",
                    a.line, b.line
                ));
            }
        }
    }
    Ok(())
}

/// A decision tree over the rows, by index into the list.
#[derive(PartialEq, Eq)]
enum Node {
    /// Test one bit of the word.
    Split { bit: u32, zero: Box<Node>, one: Box<Node> },
    /// The rows still possible, most fixed bits first.
    Leaf(Vec<usize>),
}

/// Split `set` (rows in priority order) on the bit that leaves the smallest
/// sum of squared side sizes — each side keeps the rows that fix the bit to
/// its value and every row that leaves it free — until one row or no
/// untested fixed bit is left.
///
/// A row whose fixed bits have all been tested matches every word that gets
/// here, so the rows after it never win and are dropped; and a split whose
/// two sides come out the same is no split, since a row can be on both
/// sides only if it leaves the bit free.
fn build(rows: &[Row], mut set: Vec<usize>, tested: u32) -> Node {
    if let Some(last) = set.iter().position(|&i| rows[i].mask & !tested == 0) {
        set.truncate(last + 1);
    }
    let untested = set.iter().fold(0u32, |acc, &i| acc | (rows[i].mask & !tested));
    if set.len() <= 1 || untested == 0 {
        return Node::Leaf(set);
    }
    let side = |bit: u32, value: bool| -> Vec<usize> {
        set.iter()
            .copied()
            .filter(|&i| rows[i].mask & bit == 0 || (rows[i].opcode & bit != 0) == value)
            .collect()
    };
    let mut best: Option<(usize, usize, u32)> = None;
    for b in (0..32).rev() {
        let bit = 1u32 << b;
        if untested & bit == 0 {
            continue;
        }
        let zeros = set
            .iter()
            .filter(|&&i| rows[i].mask & bit == 0 || rows[i].opcode & bit == 0)
            .count();
        let ones = set
            .iter()
            .filter(|&&i| rows[i].mask & bit == 0 || rows[i].opcode & bit != 0)
            .count();
        let score = zeros * zeros + ones * ones;
        let larger = zeros.max(ones);
        if best.is_none_or(|(s, l, _)| (score, larger) < (s, l)) {
            best = Some((score, larger, bit));
        }
    }
    let Some((_, _, bit)) = best else {
        return Node::Leaf(set);
    };
    let zero = build(rows, side(bit, false), tested | bit);
    let one = build(rows, side(bit, true), tested | bit);
    if zero == one {
        return zero;
    }
    Node::Split {
        bit,
        zero: Box::new(zero),
        one: Box::new(one),
    }
}

fn kind_expr(kind: &RowKind) -> String {
    match kind {
        RowKind::Class(name) => format!("Kind::Class(Class::{name})"),
        RowKind::Hint => "Kind::Hint".to_string(),
        RowKind::Absent(feature) => format!("Kind::Absent(Feature::{feature})"),
    }
}

/// `0x1234_5678`.
fn hex(value: u32) -> String {
    format!("0x{:04X}_{:04X}", value >> 16, value & 0xFFFF)
}

struct Emitter<'r> {
    rows: &'r [Row],
    out: String,
    nodes: usize,
    leaves: usize,
    depth: usize,
}

impl Emitter<'_> {
    fn indent(&mut self, level: usize) {
        for _ in 0..level {
            self.out.push_str("    ");
        }
    }

    /// The final value of a leaf: on its own line, or as the block of the
    /// `else` the previous test left open.
    fn last_arm(&mut self, level: usize, chained: bool, value: &str) {
        if chained {
            self.out.push_str("{\n");
            self.indent(level + 1);
            self.out.push_str(value);
            self.out.push('\n');
            self.indent(level);
            self.out.push_str("}\n");
        } else {
            self.indent(level);
            self.out.push_str(value);
            self.out.push('\n');
        }
    }

    /// Emit `node` as one expression at `level`: a split is an `if`/`else`
    /// on its bit; a leaf tests its rows in order with the bits the path
    /// has not tested, and is `None` when none matches.
    fn node(&mut self, node: &Node, level: usize, tested: u32) {
        self.depth = self.depth.max(level);
        match node {
            Node::Split { bit, zero, one } => {
                self.nodes += 1;
                self.indent(level);
                self.out.push_str(&format!("if word & {} == 0 {{\n", hex(*bit)));
                self.node(zero, level + 1, tested | bit);
                self.indent(level);
                self.out.push_str("} else {\n");
                self.node(one, level + 1, tested | bit);
                self.indent(level);
                self.out.push_str("}\n");
            }
            Node::Leaf(set) => {
                self.leaves += 1;
                // After the first test, each arm continues an `else`.
                let mut chained = false;
                for &i in set {
                    let row = &self.rows[i];
                    let residual = row.mask & !tested;
                    if residual == 0 {
                        self.last_arm(level, chained, &format!("Some(&ENCODINGS[{i}])"));
                        return;
                    }
                    if !chained {
                        self.indent(level);
                    }
                    self.out.push_str(&format!(
                        "if word & {} == {} {{\n",
                        hex(residual),
                        hex(row.opcode & residual)
                    ));
                    self.indent(level + 1);
                    self.out.push_str(&format!("Some(&ENCODINGS[{i}])\n"));
                    self.indent(level);
                    self.out.push_str("} else ");
                    chained = true;
                }
                self.last_arm(level, chained, "None");
            }
        }
    }
}

/// The checked-in `table.rs` for `spec`.
pub fn generate(spec: &str) -> Result<String, String> {
    let rows = parse(spec)?;
    validate(&rows)?;
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(rows[i].fixed_bits()), i));
    let tree = build(&rows, order, 0);

    let mut emitter = Emitter {
        rows: &rows,
        out: String::new(),
        nodes: 0,
        leaves: 0,
        depth: 0,
    };
    emitter.node(&tree, 1, 0);
    let body = std::mem::take(&mut emitter.out);

    let mut out = String::new();
    out.push_str(
        "//! The A64 encoding table and the decision tree that searches it.\n\
         //!\n\
         //! Generated by `cargo xtask a64-decode` (xtask/src/a64_decode.rs) from\n\
         //! `encodings.txt` beside this file. Do not edit: change the list and\n\
         //! regenerate. `cargo xtask ci` fails while this file is not what the list\n\
         //! compiles to.\n\n\
         use super::{Class, Encoding, Feature, Kind};\n\n",
    );
    out.push_str("/// Every row of `encodings.txt`, in its order.\n#[rustfmt::skip]\n");
    out.push_str(&format!("pub(crate) static ENCODINGS: [Encoding; {}] = [\n", rows.len()));
    for row in &rows {
        out.push_str(&format!(
            "    Encoding::new({}, {}, {:?}, {}), // line {}\n",
            hex(row.opcode),
            hex(row.mask),
            row.mnemonic,
            kind_expr(&row.kind),
            row.line
        ));
    }
    out.push_str("];\n\n");
    out.push_str(&format!(
        "/// The row of [`ENCODINGS`] that `word` is an instance of with the most\n\
         /// fixed bits, if any: {} tests and {} leaves, at most {} tests deep.\n",
        emitter.nodes,
        emitter.leaves,
        emitter.depth - 1
    ));
    out.push_str("#[rustfmt::skip]\npub(crate) fn lookup(word: u32) -> Option<&'static Encoding> {\n");
    out.push_str(&body);
    out.push_str("}\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_SNIPPET: &str = "\
class AddSubImmediate  sf:1 op:1 S:1 10001 shift:2 imm12:12 Rn:5 Rd:5
x_0_0_10001_xx_xxxxxxxxxxxx_xxxxx_xxxxx  ADD
x_1_0_10001_xx_xxxxxxxxxxxx_xxxxx_xxxxx  SUB
hint
11010101000000110010xxxxxxx11111  HINT
absent Mte
1001000110xxxxxx00xxxxxxxxxxxxxx  ADDG
";

    fn rows(spec: &str) -> Vec<Row> {
        parse(spec).expect("the snippet parses")
    }

    /// Interpret the tree the way the generated code does.
    fn search(rows: &[Row], node: &Node, word: u32) -> Option<usize> {
        match node {
            Node::Split { bit, zero, one } => search(rows, if word & bit == 0 { zero } else { one }, word),
            Node::Leaf(set) => set
                .iter()
                .copied()
                .find(|&i| word & rows[i].mask == rows[i].opcode),
        }
    }

    #[test]
    fn the_tree_finds_the_row_with_the_most_fixed_bits() {
        let rows = rows(SPEC_SNIPPET);
        let mut order: Vec<usize> = (0..rows.len()).collect();
        order.sort_by_key(|&i| (std::cmp::Reverse(rows[i].fixed_bits()), i));
        let tree = build(&rows, order, 0);
        assert_eq!(search(&rows, &tree, 0x9100_0400), Some(0), "add x0, x0, #1");
        assert_eq!(search(&rows, &tree, 0xD100_0400), Some(1), "sub x0, x0, #1");
        assert_eq!(search(&rows, &tree, 0x9180_0000), Some(3), "ADDG is more specific than ADD");
        assert_eq!(search(&rows, &tree, 0xD503_203F), Some(2), "YIELD");
        assert_eq!(search(&rows, &tree, 0x0000_0000), None);
    }

    #[test]
    fn a_row_outside_its_class_diagram_is_refused() {
        let spec = "class AddSubImmediate  sf:1 op:1 S:1 10001 shift:2 imm12:12 Rn:5 Rd:5\n\
                    x_0_0_10011_xx_xxxxxxxxxxxx_xxxxx_xxxxx  ADD\nhint\n11010101000000110010xxxxxxx11111  HINT\n";
        let err = parse(spec).expect_err("bits 28:24 differ from the diagram");
        assert!(err.contains("line 2"), "{err}");
    }

    #[test]
    fn a_row_not_split_at_the_diagram_fields_is_refused() {
        let spec = "class AddSubImmediate  sf:1 op:1 S:1 10001 shift:2 imm12:12 Rn:5 Rd:5\n\
                    x0010001xxxxxxxxxxxxxxxxxxxxxxxx  ADD\nhint\n11010101000000110010xxxxxxx11111  HINT\n";
        assert!(parse(spec).is_err());
    }

    #[test]
    fn rows_a_word_cannot_choose_between_are_refused() {
        let spec = "hint\n11010101000000110010xxxxxxx11111  HINT\nabsent Mte\n\
                    1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx  A\nx1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx  B\n";
        let err = validate(&rows(spec)).expect_err("0xC0000000 matches both with one fixed bit each");
        assert!(err.contains("lines 4 and 5"), "{err}");
        let duplicate = "hint\n11010101000000110010xxxxxxx11111  HINT\nabsent Mte\n\
                         1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx  A\n1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx  B\n";
        assert!(validate(&rows(duplicate)).is_err());
    }

    /// The checked-in table is what the checked-in list compiles to.
    #[test]
    fn the_checked_in_table_is_current() {
        let root = repo_root().expect("xtask sits in the repository");
        let spec = fs::read_to_string(root.join(SPEC)).expect("the encoding list is readable");
        let table = fs::read_to_string(root.join(TABLE)).expect("the generated table is readable");
        assert!(generate(&spec).expect("the list compiles") == table, "run `cargo xtask a64-decode`");
    }
}
