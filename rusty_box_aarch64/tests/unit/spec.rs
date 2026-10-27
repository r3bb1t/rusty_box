//! The decoder checked against Arm's machine-readable specification.
//!
//! The source of truth is the A64 instruction set of the Arm A-profile
//! Architecture Machine Readable Specification (AARCHMRS),
//! `Instructions.json`, as Arm distributes it under the BSD 3-clause
//! licence: the encoding tree of the Arm ARM's A64 index, every group and
//! encoding with its fields, its fixed bits and its condition. It is not part
//! of this repository. The tests that need it read it from the unpacked
//! AARCHMRS directory `RUSTY_BOX_AARCHMRS` names, and are ignored when the
//! variable was not set for the build (`build.rs` sets `cfg(aarchmrs)`).
//! They were written against release 2026-09 and report the release they
//! read.
//!
//! The decoder's source is read with `syn`, in the shape the `isa` module
//! documentation gives: from [`decode`] through every `match` to every
//! `Some(Class::Variant)`, recording the bits each arm on the way fixes. That
//! reading needs no Arm data and is checked on every run
//! ([`the_decoder_source_has_the_shape_the_spec_test_reads`]). Against Arm's
//! data:
//!
//! - [`the_decoder_is_arms_encoding_tree_for_the_implemented_features`]:
//!   the functions are Arm's groups and classes, called from Arm's parents;
//!   every field is read under the name and at the bits Arm gives it; each
//!   class enum names exactly the encodings of that class whose condition
//!   holds for the features this processor implements ([`Feature`]); each
//!   encoding's arms fix exactly Arm's fixed bits; each encoding carries
//!   Arm's feature.
//! - [`the_decoder_allocates_as_arm_does_on_sampled_words`] and
//!   [`the_decoder_allocates_as_arm_does_on_every_word`]: on every word,
//!   [`decode`] names the encoding Arm's data allocates the word to among
//!   the implemented features — the most specific match, which only the
//!   HINT space needs, where `HINT_HM_hints` spans the named hints — and
//!   nothing where Arm allocates it to none; with the one departure
//!   [`DEPARTURES`] lists.

use std::boxed::Box;
use std::collections::{BTreeMap, BTreeSet};
use std::format;
use std::path::PathBuf;
use std::string::{String, ToString};
use std::sync::{Mutex, OnceLock};
use std::vec;
use std::vec::Vec;

use serde_json::Value;
use syn::{Expr, ImplItem, Item, Pat, ReturnType, Stmt, Type};

use super::{decode, Encoding, Feature};

/// The environment variable naming the unpacked AARCHMRS directory.
const SPEC_DIR_VAR: &str = "RUSTY_BOX_AARCHMRS";

/// Arm's top-level A64 groups the decoder has a function for: all but
/// `sve` and `sme`.
const TOP_GROUPS: [&str; 6] = ["reserved", "dpimm", "control", "ldst", "dpreg", "simd_dp"];

/// The encodings whose Arm condition the decoder does not follow: the
/// encoding, the condition as Arm writes it, and why.
const DEPARTURES: [(&str, &str, &str); 1] = [(
    "PRFM_P_ldst_regoff",
    "!((Rt IN {'11xxx'}))",
    "Arm's data takes Rt = 11xxx from PRFM (register) for RPRFM (FEAT_RPRFM); \
     on a core without the feature the word is a PRFM with a reserved prfop, \
     a prefetch hint (Arm ARM C6.2, PRFM (register))",
)];

// ----- the source -----

/// The words `word & mask == opcode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cube {
    pub(crate) opcode: u32,
    pub(crate) mask: u32,
}

impl Cube {
    fn free_bits(self) -> u32 {
        32 - self.mask.count_ones()
    }

    fn within(self, outer: Cube) -> bool {
        self.mask & outer.mask == outer.mask && self.opcode & outer.mask == outer.opcode
    }

    fn meets(self, other: Cube) -> bool {
        (self.opcode ^ other.opcode) & self.mask & other.mask == 0
    }

    pub(crate) fn contains(self, word: u32) -> bool {
        word & self.mask == self.opcode
    }
}

/// One `Some(Class::Variant)` the decoder reaches, with the bits the arms
/// on the way there fix.
#[derive(Debug, Clone)]
pub(crate) struct Occurrence {
    pub(crate) class: String,
    pub(crate) variant: String,
    /// The class function it is returned from.
    pub(crate) class_fn: String,
    /// The top-level group function on the way.
    pub(crate) top_fn: String,
    pub(crate) cube: Cube,
}

/// A field read under a name: `let name = bits(word, hi, lo)` in `function`,
/// which decodes Arm node `node`.
#[derive(Debug, Clone)]
struct Binding {
    function: String,
    node: String,
    name: String,
    hi: u32,
    lo: u32,
}

/// What reading the decoder's source finds.
pub(crate) struct Source {
    pub(crate) occurrences: Vec<Occurrence>,
    /// Each class enum an `Encoding` variant carries, with its variants.
    pub(crate) classes: BTreeMap<String, Vec<String>>,
    bindings: Vec<Binding>,
    /// The `Feature` variant each encoding is tagged with.
    features: BTreeMap<(String, String), String>,
    feature_variants: Vec<String>,
    /// What could not be read, or was read in a shape the reading does not
    /// follow. A test that uses the reading asserts this is empty.
    pub(crate) errors: Vec<String>,
}

/// `addsub_imm` as `AddsubImm`.
fn camel(snake: &str) -> String {
    snake
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            let first = chars.next().map(|c| c.to_ascii_uppercase());
            first.into_iter().chain(chars.map(|c| c.to_ascii_lowercase())).collect::<String>()
        })
        .collect()
}

/// The variant name of Arm encoding `encoding` of class `class`: the
/// encoding id without the class name and without `only` (Arm's mark for
/// the one encoding of an instruction in its class), in UpperCamelCase.
fn variant_name(encoding: &str, class: &str) -> Option<String> {
    let parts: Vec<&str> = encoding.split('_').collect();
    let class_parts: Vec<&str> = class.split('_').collect();
    let at = (0..=parts.len().checked_sub(class_parts.len())?).find(|&i| parts[i..i + class_parts.len()] == class_parts[..])?;
    let rest: Vec<&str> = parts[..at]
        .iter()
        .chain(parts[at + class_parts.len()..].iter())
        .copied()
        .filter(|&part| part != "only")
        .collect();
    Some(camel(&rest.join("_")))
}

fn last_ident(path: &syn::Path) -> Option<String> {
    path.segments.last().map(|s| s.ident.to_string())
}

fn int(expr: &Expr) -> Option<u32> {
    match expr {
        Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(i), .. }) => i.base10_parse::<u32>().ok(),
        _ => None,
    }
}

/// `Option<T>` as `T`.
fn option_of(output: &ReturnType) -> Option<String> {
    let ReturnType::Type(_, ty) = output else { return None };
    let Type::Path(p) = ty.as_ref() else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else { return None };
    match args.args.first()? {
        syn::GenericArgument::Type(Type::Path(inner)) => last_ident(&inner.path),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct Pin {
    hi: u32,
    lo: u32,
    value: u32,
}

#[derive(Debug, Clone)]
struct Context {
    function: String,
    node: String,
    top: Option<String>,
    class: Option<String>,
    fields: BTreeMap<String, (u32, u32)>,
}

struct Reader<'a> {
    fns: BTreeMap<String, &'a syn::ItemFn>,
    occurrences: Vec<Occurrence>,
    bindings: Vec<Binding>,
    bound: BTreeSet<String>,
    /// Every match arm met, by address, with its function and whether some
    /// path reached it.
    arms: BTreeMap<usize, (String, bool)>,
    errors: Vec<String>,
}

impl Reader<'_> {
    fn error(&mut self, function: &str, what: String) {
        self.errors.push(format!("{function}: {what}"));
    }

    /// Follow `name(word)`, reached with `pins` fixed.
    fn enter(&mut self, name: &str, node: String, top: Option<String>, class: Option<String>, pins: &[Pin]) {
        let Some(&item) = self.fns.get(name) else {
            self.error(name, "no such decoder function".to_string());
            return;
        };
        let takes_word = item.sig.inputs.len() == 1
            && matches!(item.sig.inputs.first(), Some(syn::FnArg::Typed(t))
                if matches!(t.pat.as_ref(), Pat::Ident(i) if i.ident == "word" || i.ident == "_word"));
        if !takes_word {
            self.error(name, "a decoder function takes exactly `word: u32`".to_string());
        }
        let mut cx = Context {
            function: name.to_string(),
            node,
            top,
            class,
            fields: BTreeMap::new(),
        };
        let stmts = &item.block.stmts;
        for stmt in stmts.iter().take(stmts.len().saturating_sub(1)) {
            let Stmt::Local(local) = stmt else {
                self.error(name, "a statement other than `let field = bits(word, hi, lo);`".to_string());
                continue;
            };
            let (Pat::Ident(ident), Some(init)) = (&local.pat, &local.init) else {
                self.error(name, "a `let` that is not `let field = bits(...)`".to_string());
                continue;
            };
            match self.bits_call(&init.expr) {
                Some((hi, lo)) => {
                    let field = ident.ident.to_string();
                    cx.fields.insert(field.clone(), (hi, lo));
                    if self.bound.insert(format!("{name}/{field}")) {
                        self.bindings.push(Binding {
                            function: name.to_string(),
                            node: cx.node.clone(),
                            name: field,
                            hi,
                            lo,
                        });
                    }
                }
                None => self.error(name, format!("`let {}` is not `bits(word, hi, lo)`", ident.ident)),
            }
        }
        match stmts.last() {
            Some(Stmt::Expr(expr, None)) => self.expr(expr, &cx, pins),
            _ => self.error(name, "the body does not end in an expression".to_string()),
        }
    }

    /// `bits(word, hi, lo)`.
    fn bits_call(&self, expr: &Expr) -> Option<(u32, u32)> {
        let Expr::Call(call) = expr else { return None };
        let Expr::Path(func) = call.func.as_ref() else { return None };
        if last_ident(&func.path)? != "bits" || call.args.len() != 3 {
            return None;
        }
        let mut args = call.args.iter();
        let word = args.next()?;
        if !matches!(word, Expr::Path(p) if p.path.is_ident("word")) {
            return None;
        }
        let hi = int(args.next()?)?;
        let lo = int(args.next()?)?;
        (hi < 32 && lo <= hi).then_some((hi, lo))
    }

    fn field(&mut self, expr: &Expr, cx: &Context) -> Option<(u32, u32)> {
        if let Expr::Path(p) = expr {
            if let Some(ident) = p.path.get_ident() {
                let found = cx.fields.get(&ident.to_string()).copied();
                if found.is_none() {
                    self.error(&cx.function, format!("`{ident}` is not a field bound in this function"));
                }
                return found;
            }
        }
        let found = self.bits_call(expr);
        if found.is_none() {
            self.error(&cx.function, "a scrutinee that is neither a bound field nor `bits(word, hi, lo)`".to_string());
        }
        found
    }

    fn scrutinee(&mut self, expr: &Expr, cx: &Context) -> Option<Vec<(u32, u32)>> {
        match expr {
            Expr::Tuple(t) => t.elems.iter().map(|e| self.field(e, cx)).collect(),
            Expr::Paren(p) => self.scrutinee(&p.expr, cx),
            _ => self.field(expr, cx).map(|f| vec![f]),
        }
    }

    /// The values one element of a pattern admits: `None` for `_`.
    fn element(&mut self, pat: &Pat, cx: &Context) -> Option<Vec<Option<u32>>> {
        match pat {
            Pat::Wild(_) => Some(vec![None]),
            Pat::Lit(lit) => match &lit.lit {
                syn::Lit::Int(i) => i.base10_parse::<u32>().ok().map(|v| vec![Some(v)]),
                _ => None,
            },
            Pat::Range(range) => {
                let lo = int(range.start.as_deref()?)?;
                let hi = int(range.end.as_deref()?)?;
                let hi = match range.limits {
                    syn::RangeLimits::Closed(_) => hi,
                    syn::RangeLimits::HalfOpen(_) => hi.checked_sub(1)?,
                };
                Some((lo..=hi).map(Some).collect())
            }
            Pat::Or(or) => {
                let mut all = Vec::new();
                for case in &or.cases {
                    all.extend(self.element(case, cx)?);
                }
                Some(all)
            }
            Pat::Paren(p) => self.element(&p.pat, cx),
            _ => {
                self.error(&cx.function, "a pattern other than a literal, a range, `|` or `_`".to_string());
                None
            }
        }
    }

    /// Every alternative a pattern over `arity` fields admits.
    fn pattern(&mut self, pat: &Pat, arity: usize, cx: &Context) -> Option<Vec<Vec<Option<u32>>>> {
        match pat {
            Pat::Wild(_) => Some(vec![vec![None; arity]]),
            Pat::Or(or) if arity > 1 => {
                let mut all = Vec::new();
                for case in &or.cases {
                    all.extend(self.pattern(case, arity, cx)?);
                }
                Some(all)
            }
            Pat::Tuple(t) if arity > 1 && t.elems.len() == arity => {
                let mut all: Vec<Vec<Option<u32>>> = vec![Vec::new()];
                for elem in &t.elems {
                    let values = self.element(elem, cx)?;
                    all = all
                        .into_iter()
                        .flat_map(|prefix| {
                            values.iter().map(move |v| {
                                let mut next = prefix.clone();
                                next.push(*v);
                                next
                            })
                        })
                        .collect();
                }
                Some(all)
            }
            _ if arity == 1 => Some(self.element(pat, cx)?.into_iter().map(|v| vec![v]).collect()),
            _ => {
                self.error(&cx.function, "a pattern that does not match the scrutinee's shape".to_string());
                None
            }
        }
    }

    /// The words `pins` admit, or `None` when two pins contradict: a path
    /// through an outer or-pattern that this arm cannot follow.
    fn cube(pins: &[Pin]) -> Option<Cube> {
        let mut cube = Cube { opcode: 0, mask: 0 };
        for pin in pins {
            let width = pin.hi - pin.lo + 1;
            let mask = (((1u64 << width) - 1) as u32) << pin.lo;
            let value = pin.value << pin.lo;
            if (cube.opcode ^ value) & cube.mask & mask != 0 {
                return None;
            }
            cube.mask |= mask;
            cube.opcode |= value;
        }
        Some(cube)
    }

    fn expr(&mut self, expr: &Expr, cx: &Context, pins: &[Pin]) {
        match expr {
            Expr::Match(m) => {
                let Some(fields) = self.scrutinee(&m.expr, cx) else { return };
                for arm in &m.arms {
                    let id = arm as *const syn::Arm as usize;
                    self.arms.entry(id).or_insert((cx.function.clone(), false));
                    if arm.guard.is_some() {
                        self.error(&cx.function, "a match guard".to_string());
                        continue;
                    }
                    let Some(alternatives) = self.pattern(&arm.pat, fields.len(), cx) else { continue };
                    for alternative in alternatives {
                        let mut next = pins.to_vec();
                        for (&(hi, lo), value) in fields.iter().zip(alternative) {
                            if let Some(value) = value {
                                if u64::from(value) >> (hi - lo + 1) != 0 {
                                    self.error(&cx.function, format!("{value:#b} is wider than bits {hi}:{lo}"));
                                }
                                next.push(Pin { hi, lo, value });
                            }
                        }
                        if Self::cube(&next).is_some() {
                            self.arms.insert(id, (cx.function.clone(), true));
                            self.expr(&arm.body, cx, &next);
                        }
                    }
                }
            }
            Expr::Call(call) => {
                let Expr::Path(func) = call.func.as_ref() else {
                    self.error(&cx.function, "a call of something other than a path".to_string());
                    return;
                };
                let callee = last_ident(&func.path).unwrap_or_default();
                if callee == "Some" {
                    let returned = match call.args.first() {
                        Some(Expr::Path(p)) if p.path.segments.len() == 2 && call.args.len() == 1 => {
                            Some((p.path.segments[0].ident.to_string(), p.path.segments[1].ident.to_string()))
                        }
                        _ => None,
                    };
                    let Some((class, variant)) = returned else {
                        self.error(&cx.function, "`Some` of something other than `Class::Variant`".to_string());
                        return;
                    };
                    if cx.class.as_deref() != Some(class.as_str()) {
                        self.error(&cx.function, format!("returns {class}::{variant} outside class function of {class}"));
                        return;
                    }
                    let Some(cube) = Self::cube(pins) else { return };
                    self.occurrences.push(Occurrence {
                        class,
                        variant,
                        class_fn: cx.function.clone(),
                        top_fn: cx.top.clone().unwrap_or_default(),
                        cube,
                    });
                } else {
                    if !matches!(call.args.first(), Some(Expr::Path(p)) if p.path.is_ident("word")) || call.args.len() != 1 {
                        self.error(&cx.function, format!("`{callee}` is not called with `word`"));
                        return;
                    }
                    let returns = self.fns.get(&callee).and_then(|f| option_of(&f.sig.output));
                    if returns.as_deref() != Some("Encoding") {
                        self.error(&cx.function, format!("`{callee}(word)` must return Option<Encoding>, or be mapped into it"));
                        return;
                    }
                    let is_top = TOP_GROUPS.contains(&callee.as_str());
                    let node = if is_top { callee.clone() } else { cx.node.clone() };
                    let top = if is_top { Some(callee.clone()) } else { cx.top.clone() };
                    self.enter(&callee, node, top, None, pins);
                }
            }
            Expr::MethodCall(mc) if mc.method == "map" && mc.args.len() == 1 => {
                let target = match mc.receiver.as_ref() {
                    Expr::Call(call) => match call.func.as_ref() {
                        Expr::Path(p) => last_ident(&p.path),
                        _ => None,
                    },
                    _ => None,
                };
                let wrapper = match mc.args.first() {
                    Some(Expr::Path(p)) if p.path.segments.len() == 2 && p.path.segments[0].ident == "Encoding" => {
                        Some(p.path.segments[1].ident.to_string())
                    }
                    _ => None,
                };
                let (Some(target), Some(wrapper)) = (target, wrapper) else {
                    self.error(&cx.function, "a `.map` other than `class(word).map(Encoding::Class)`".to_string());
                    return;
                };
                let returns = self.fns.get(&target).and_then(|f| option_of(&f.sig.output));
                if returns.as_deref() != Some(wrapper.as_str()) || camel(&target) != wrapper {
                    self.error(
                        &cx.function,
                        format!("`{target}(word).map(Encoding::{wrapper})`: the class function of {wrapper} is `{}` returning Option<{wrapper}>", wrapper.to_lowercase()),
                    );
                    return;
                }
                self.enter(&target, target.clone(), cx.top.clone(), Some(wrapper), pins);
            }
            Expr::Path(p) if p.path.is_ident("None") => {}
            Expr::Block(b) if b.block.stmts.len() == 1 => match &b.block.stmts[0] {
                Stmt::Expr(inner, None) => self.expr(inner, cx, pins),
                _ => self.error(&cx.function, "a block that is not one expression".to_string()),
            },
            Expr::Paren(p) => self.expr(&p.expr, cx, pins),
            _ => self.error(&cx.function, "an expression other than a match, `Some(Class::Variant)`, `None` or a decoder call".to_string()),
        }
    }
}

/// The variant names in a `feature` match arm's pattern.
fn arm_variants(pat: &Pat) -> Vec<String> {
    match pat {
        Pat::Or(or) => or.cases.iter().flat_map(arm_variants).collect(),
        Pat::TupleStruct(t) => last_ident(&t.path).into_iter().collect(),
        Pat::Path(p) => last_ident(&p.path).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// What a `feature` match arm answers: `Feature::X`, or the class's own
/// `feature()`.
enum Tag {
    Fixed(String),
    PerVariant,
}

fn arm_tag(body: &Expr) -> Option<Tag> {
    match body {
        Expr::Path(p) if p.path.segments.len() == 2 && p.path.segments[0].ident == "Feature" => {
            Some(Tag::Fixed(p.path.segments[1].ident.to_string()))
        }
        Expr::MethodCall(mc) if mc.method == "feature" => Some(Tag::PerVariant),
        _ => None,
    }
}

/// The `feature` method of `impl`, as (variants, tag) per arm.
fn feature_arms(imp: &syn::ItemImpl) -> Option<Vec<(Vec<String>, Tag)>> {
    let body = imp.items.iter().find_map(|item| match item {
        ImplItem::Fn(f) if f.sig.ident == "feature" => f.block.stmts.last(),
        _ => None,
    })?;
    let Stmt::Expr(Expr::Match(m), None) = body else { return None };
    m.arms.iter().map(|arm| Some((arm_variants(&arm.pat), arm_tag(&arm.body)?))).collect()
}

/// The decoder's source as [`read_source`] reads it, read once per test
/// binary.
pub(crate) fn source() -> &'static Source {
    static SOURCE: OnceLock<Source> = OnceLock::new();
    SOURCE.get_or_init(read_source)
}

/// Read the decoder's source: every `.rs` file of `src/cpu/isa`.
fn read_source() -> Source {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("cpu").join("isa");
    let mut errors = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => paths.push(entry.path()),
                    Err(e) => errors.push(format!("{}: {e}", dir.display())),
                }
            }
        }
        Err(e) => errors.push(format!("{}: {e}", dir.display())),
    }
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|text| syn::parse_file(&text).map_err(|e| e.to_string())) {
            Ok(file) => files.push(file),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    if files.is_empty() {
        errors.push(format!("{}: no decoder source", dir.display()));
    }
    let items: Vec<&Item> = files.iter().flat_map(|f| f.items.iter()).collect();

    let mut fns = BTreeMap::new();
    let mut enums: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut impls: Vec<&syn::ItemImpl> = Vec::new();
    for item in &items {
        match item {
            Item::Fn(f) => {
                fns.insert(f.sig.ident.to_string(), f);
            }
            Item::Enum(e) => {
                enums.insert(e.ident.to_string(), e.variants.iter().map(|v| v.ident.to_string()).collect());
            }
            Item::Impl(i) => impls.push(i),
            _ => {}
        }
    }

    let mut reader = Reader {
        fns,
        occurrences: Vec::new(),
        bindings: Vec::new(),
        bound: BTreeSet::new(),
        arms: BTreeMap::new(),
        errors,
    };
    reader.enter("decode", "A64".to_string(), None, None, &[]);
    let unreached: Vec<String> = reader.arms.values().filter(|(_, reached)| !reached).map(|(f, _)| f.clone()).collect();
    for function in unreached {
        reader.errors.push(format!("{function}: an arm no word reaches: it contradicts the arms enclosing it"));
    }

    let mut classes = BTreeMap::new();
    for class in enums.get("Encoding").cloned().unwrap_or_default() {
        match enums.get(&class) {
            Some(variants) => {
                classes.insert(class, variants.clone());
            }
            None => reader.errors.push(format!("Encoding::{class} carries no enum named {class}")),
        }
    }

    let mut features = BTreeMap::new();
    let self_name = |imp: &syn::ItemImpl| match imp.self_ty.as_ref() {
        Type::Path(p) => last_ident(&p.path),
        _ => None,
    };
    let per_class: BTreeMap<String, Vec<(Vec<String>, Tag)>> = impls
        .iter()
        .filter_map(|imp| Some((self_name(imp)?, feature_arms(imp)?)))
        .collect();
    match per_class.get("Encoding") {
        Some(arms) => {
            for (wrapped, tag) in arms {
                for class in wrapped {
                    let variants = classes.get(class).cloned().unwrap_or_default();
                    match tag {
                        Tag::Fixed(f) => {
                            for v in variants {
                                features.insert((class.clone(), v), f.clone());
                            }
                        }
                        Tag::PerVariant => {
                            let Some(inner) = per_class.get(class) else {
                                reader.errors.push(format!("{class} has no `feature` method"));
                                continue;
                            };
                            for (vs, tag) in inner {
                                for v in vs {
                                    match tag {
                                        Tag::Fixed(f) => {
                                            features.insert((class.clone(), v.clone()), f.clone());
                                        }
                                        Tag::PerVariant => reader.errors.push(format!("{class}::{v}: a feature that is not `Feature::X`")),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None => reader.errors.push("Encoding has no `feature` method".to_string()),
    }

    Source {
        occurrences: reader.occurrences,
        classes,
        bindings: reader.bindings,
        features,
        feature_variants: enums.get("Feature").cloned().unwrap_or_default(),
        errors: reader.errors,
    }
}

#[test]
fn the_decoder_source_has_the_shape_the_spec_test_reads() {
    let src = source();
    assert!(src.errors.is_empty(), "{:#?}", src.errors);
    assert!(src.classes.len() > 50, "{} classes", src.classes.len());
    for (class, variants) in &src.classes {
        for variant in variants {
            assert!(
                src.occurrences.iter().any(|o| &o.class == class && &o.variant == variant),
                "{class}::{variant} is never returned"
            );
            assert!(src.features.contains_key(&(class.clone(), variant.clone())), "{class}::{variant} has no feature");
        }
    }
    let all: Vec<String> = Feature::ALL.iter().map(|f| format!("{f:?}")).collect();
    assert_eq!(all, src.feature_variants, "Feature::ALL lists every feature, in order");
    // What reading the source finds, the compiled decoder does.
    let mut state = 0x5EED_u64;
    for o in &src.occurrences {
        for _ in 0..8 {
            let word = o.cube.opcode | (splitmix(&mut state) & !o.cube.mask);
            let name = decode(word).map(|e| format!("{e:?}"));
            let want = format!("{}({})", o.class, o.variant);
            // A narrower arm or an exclusion may take the word: the
            // allocation tests settle those; here the reading must match
            // the decoder wherever the decoder names this class.
            if let Some(name) = name.filter(|n| n.starts_with(&format!("{}(", o.class))) {
                if name != want {
                    let others: Vec<&Occurrence> = src
                        .occurrences
                        .iter()
                        .filter(|p| p.class == o.class && p.cube.contains(word) && p.cube.mask.count_ones() > o.cube.mask.count_ones())
                        .collect();
                    assert!(!others.is_empty(), "{word:#010x}: the source reads {want}, the decoder says {name}");
                }
            }
        }
    }
}

fn splitmix(state: &mut u64) -> u32 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) as u32
}

// ----- Arm's data -----

/// A condition with the features folded in.
#[derive(Debug, Clone)]
enum Cond {
    Const(bool),
    Not(Box<Cond>),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    /// The word matches a field pattern: `word & mask == value`.
    Matches { mask: u32, value: u32 },
}

impl Cond {
    fn eval(&self, word: u32) -> bool {
        match self {
            Cond::Const(b) => *b,
            Cond::Not(c) => !c.eval(word),
            Cond::And(a, b) => a.eval(word) && b.eval(word),
            Cond::Or(a, b) => a.eval(word) || b.eval(word),
            Cond::Matches { mask, value } => word & mask == *value,
        }
    }

    fn referenced(&self) -> u32 {
        match self {
            Cond::Const(_) => 0,
            Cond::Not(c) => c.referenced(),
            Cond::And(a, b) | Cond::Or(a, b) => a.referenced() | b.referenced(),
            Cond::Matches { mask, .. } => *mask,
        }
    }
}

/// One A64 encoding of Arm's data.
#[derive(Debug, Clone)]
struct Leaf {
    name: String,
    /// The groups above it, top first.
    path: Vec<String>,
    cube: Cube,
    condition: Cond,
    /// The features its condition names.
    features: BTreeSet<String>,
    /// A word of it its condition holds for, if any does: the encoding is
    /// one of an implemented feature.
    instance: Option<u32>,
}

struct Arm {
    release: String,
    leaves: Vec<Leaf>,
    /// The named fields of each group, by lower-case name.
    fields: BTreeMap<String, BTreeMap<String, (u32, u32)>>,
    /// The groups directly under the A64 root.
    tops: Vec<String>,
}

fn is_implemented(feature: &str) -> bool {
    Feature::ALL.iter().any(|f| f.arm_name() == Some(feature))
}

fn render(ast: &Value) -> String {
    let s = |key: &str| ast.get(key).map(render).unwrap_or_default();
    match ast.get("_type").and_then(Value::as_str).unwrap_or_default() {
        "AST.Bool" => ast.get("value").and_then(Value::as_bool).unwrap_or_default().to_string(),
        "AST.BinaryOp" => format!("({} {} {})", s("left"), ast.get("op").and_then(Value::as_str).unwrap_or_default(), s("right")),
        "AST.UnaryOp" => format!("{}({})", ast.get("op").and_then(Value::as_str).unwrap_or_default(), s("expr")),
        "AST.Function" => {
            let args: Vec<String> = ast.get("arguments").and_then(Value::as_array).map(|a| a.iter().map(render).collect()).unwrap_or_default();
            format!("{}({})", ast.get("name").and_then(Value::as_str).unwrap_or_default(), args.join(", "))
        }
        "AST.Identifier" | "Values.Value" => ast.get("value").and_then(Value::as_str).unwrap_or_default().to_string(),
        "AST.Set" => {
            let values: Vec<String> = ast.get("values").and_then(Value::as_array).map(|a| a.iter().map(render).collect()).unwrap_or_default();
            format!("{{{}}}", values.join(", "))
        }
        other => format!("<{other}>"),
    }
}

/// `field` matching `'01x'`.
fn field_pattern(field: (u32, u32), pattern: &str) -> Cond {
    let (start, width) = field;
    let pattern = pattern.trim_matches('\'');
    assert_eq!(pattern.len() as u32, width, "pattern {pattern} for a {width}-bit field");
    let (mut mask, mut value) = (0u32, 0u32);
    for (i, ch) in pattern.chars().enumerate() {
        let bit = start + width - 1 - i as u32;
        match ch {
            '0' => mask |= 1 << bit,
            '1' => {
                mask |= 1 << bit;
                value |= 1 << bit;
            }
            'x' => {}
            other => panic!("pattern character {other:?}"),
        }
    }
    Cond::Matches { mask, value }
}

/// The named fields of an encoding in Arm's data, as (start, width).
type Fields = BTreeMap<String, (u32, u32)>;

fn compile(ast: &Value, fields: &Fields, features: &mut BTreeSet<String>) -> Cond {
    let get = |key: &str| ast.get(key).unwrap_or(&Value::Null);
    let text = |v: &Value| v.get("value").and_then(Value::as_str).unwrap_or_default().to_string();
    let field = |v: &Value| {
        let name = text(v);
        *fields.get(&name).unwrap_or_else(|| panic!("condition names field {name}, which no encoding on its path has"))
    };
    match get("_type").as_str().unwrap_or_default() {
        "AST.Bool" => Cond::Const(get("value").as_bool().unwrap_or_default()),
        "AST.UnaryOp" if get("op") == "!" => Cond::Not(Box::new(compile(get("expr"), fields, features))),
        "AST.BinaryOp" => {
            let (left, right) = (get("left"), get("right"));
            match get("op").as_str().unwrap_or_default() {
                "&&" => Cond::And(Box::new(compile(left, fields, features)), Box::new(compile(right, fields, features))),
                "||" => Cond::Or(Box::new(compile(left, fields, features)), Box::new(compile(right, fields, features))),
                "==" => field_pattern(field(left), &text(right)),
                "!=" => Cond::Not(Box::new(field_pattern(field(left), &text(right)))),
                "IN" => {
                    let values = right.get("values").and_then(Value::as_array).cloned().unwrap_or_default();
                    values
                        .iter()
                        .map(|v| field_pattern(field(left), &text(v)))
                        .reduce(|a, b| Cond::Or(Box::new(a), Box::new(b)))
                        .unwrap_or(Cond::Const(false))
                }
                op => panic!("operator {op} in a condition"),
            }
        }
        "AST.Function" if get("name") == "IsFeatureImplemented" => {
            let feature = get("arguments").get(0).map(text).unwrap_or_default();
            let holds = is_implemented(&feature);
            features.insert(feature);
            Cond::Const(holds)
        }
        other => panic!("{other} in a condition: {}", render(ast)),
    }
}

fn encodeset(node: &Value, fields: &mut Fields) -> Cube {
    let mut cube = Cube { opcode: 0, mask: 0 };
    let values = node.get("encoding").and_then(|e| e.get("values")).and_then(Value::as_array).cloned().unwrap_or_default();
    for v in values {
        let start = v.get("range").and_then(|r| r.get("start")).and_then(Value::as_u64).unwrap_or_default() as u32;
        let width = v.get("range").and_then(|r| r.get("width")).and_then(Value::as_u64).unwrap_or_default() as u32;
        if let Some(name) = v.get("name").and_then(Value::as_str) {
            fields.insert(name.to_string(), (start, width));
        }
        if let Cond::Matches { mask, value } = field_pattern((start, width), v.get("value").and_then(|x| x.get("value")).and_then(Value::as_str).unwrap_or_default()) {
            cube.mask |= mask;
            cube.opcode |= value;
        }
    }
    cube
}

impl Arm {
    fn load() -> Arm {
        let dir = std::env::var_os(SPEC_DIR_VAR)
            .unwrap_or_else(|| panic!("set {SPEC_DIR_VAR} to the unpacked AARCHMRS directory (holding Instructions.json)"));
        let path = PathBuf::from(dir).join("Instructions.json");
        let text = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let root: Value = serde_json::from_slice(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let release = root.pointer("/_meta/version/ref").and_then(Value::as_str).unwrap_or("?").to_string();
        let a64 = root
            .get("instructions")
            .and_then(Value::as_array)
            .and_then(|sets| sets.iter().find(|s| s.get("name").and_then(Value::as_str) == Some("A64")))
            .unwrap_or_else(|| panic!("no A64 instruction set in {}", path.display()));
        let mut arm = Arm {
            release,
            leaves: Vec::new(),
            fields: BTreeMap::new(),
            tops: Vec::new(),
        };
        let mut root_fields = BTreeMap::new();
        encodeset(a64, &mut root_fields);
        arm.fields.insert("A64".to_string(), root_fields.iter().map(|(k, &(s, w))| (k.to_lowercase(), (s + w - 1, s))).collect());
        for child in a64.get("children").and_then(Value::as_array).cloned().unwrap_or_default() {
            if let Some(name) = child.get("name").and_then(Value::as_str) {
                arm.tops.push(name.to_string());
            }
            arm.walk(&child, &[], &root_fields, &[]);
        }
        arm
    }

    fn walk(&mut self, node: &Value, path: &[String], fields: &Fields, conditions: &[(Value, Fields)]) {
        let kind = node.get("_type").and_then(Value::as_str).unwrap_or_default();
        if kind == "Instruction.InstructionAlias" {
            return;
        }
        let name = node.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let mut own = BTreeMap::new();
        let cube = encodeset(node, &mut own);
        let mut fields = fields.clone();
        fields.extend(own.iter().map(|(k, v)| (k.clone(), *v)));
        let mut conditions = conditions.to_vec();
        if let Some(c) = node.get("condition") {
            conditions.push((c.clone(), fields.clone()));
        }
        match kind {
            "Instruction.InstructionGroup" => {
                self.fields.insert(name.clone(), own.iter().map(|(k, &(s, w))| (k.to_lowercase(), (s + w - 1, s))).collect());
                let mut below = path.to_vec();
                below.push(name);
                for child in node.get("children").and_then(Value::as_array).cloned().unwrap_or_default() {
                    self.walk(&child, &below, &fields, &conditions);
                }
            }
            "Instruction.Instruction" => {
                let mut features = BTreeSet::new();
                let mut condition = Cond::Const(true);
                for (ast, at) in &conditions {
                    let departed = DEPARTURES.iter().find(|d| d.0 == name && d.1 == render(ast));
                    let c = if departed.is_some() { Cond::Const(true) } else { compile(ast, at, &mut features) };
                    condition = Cond::And(Box::new(condition), Box::new(c));
                }
                let free = condition.referenced() & !cube.mask;
                assert!(free.count_ones() <= 20, "{name}: its condition reads {} free bits", free.count_ones());
                let instance = subsets(free).map(|bits| cube.opcode | bits).find(|&w| condition.eval(w));
                self.leaves.push(Leaf {
                    name,
                    path: path.to_vec(),
                    cube,
                    condition,
                    features,
                    instance,
                });
            }
            other => panic!("{other} node {name} in the A64 tree"),
        }
    }

    /// The leaves of the implemented features.
    fn implemented(&self) -> impl Iterator<Item = &Leaf> + '_ {
        self.leaves.iter().filter(|l| l.instance.is_some())
    }
}

/// Every subset of the bits of `mask`.
fn subsets(mask: u32) -> impl Iterator<Item = u32> {
    let mut next = Some(0u32);
    core::iter::from_fn(move || {
        let current = next?;
        next = if current == mask { None } else { Some((current.wrapping_sub(mask)) & mask) };
        Some(current)
    })
}

/// The class enum name and variant name a leaf has in the decoder.
fn rust_names(leaf: &Leaf) -> Option<(String, String)> {
    let class = leaf.path.last()?;
    Some((camel(class), variant_name(&leaf.name, class)?))
}

#[test]
#[cfg_attr(not(aarchmrs), ignore = "reads Arm's AARCHMRS from the directory RUSTY_BOX_AARCHMRS names")]
fn the_decoder_is_arms_encoding_tree_for_the_implemented_features() {
    let arm = Arm::load();
    let src = source();
    assert!(src.errors.is_empty(), "{:#?}", src.errors);
    let mut problems: Vec<String> = Vec::new();
    std::eprintln!("AARCHMRS {}: {} A64 encodings, {} of the implemented features", arm.release, arm.leaves.len(), arm.implemented().count());

    // The top-level groups.
    let mut tops: Vec<&str> = arm.tops.iter().map(String::as_str).filter(|t| !matches!(*t, "sve" | "sme")).collect();
    tops.sort_unstable();
    let mut ours = TOP_GROUPS.to_vec();
    ours.sort_unstable();
    if tops != ours {
        problems.push(format!("Arm's top-level groups are {tops:?}, the decoder's {ours:?}"));
    }
    let implemented_tops: BTreeSet<&str> = arm.implemented().filter_map(|l| l.path.first().map(String::as_str)).collect();
    if implemented_tops.iter().any(|t| matches!(*t, "sve" | "sme")) {
        problems.push("an SVE or SME encoding belongs to an implemented feature".to_string());
    }

    // Names: the classes and their encodings, both ways.
    let mut wanted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut leaf_of: BTreeMap<(String, String), &Leaf> = BTreeMap::new();
    for leaf in arm.implemented() {
        let Some((class, variant)) = rust_names(leaf) else {
            problems.push(format!("{}: no Rust name (class {:?})", leaf.name, leaf.path));
            continue;
        };
        if leaf_of.insert((class.clone(), variant.clone()), leaf).is_some() {
            problems.push(format!("{}: {class}::{variant} names two encodings", leaf.name));
        }
        wanted.entry(class).or_default().insert(variant);
    }
    for (class, variants) in &wanted {
        let have: BTreeSet<String> = src.classes.get(class).map(|v| v.iter().cloned().collect()).unwrap_or_default();
        for missing in variants.difference(&have) {
            problems.push(format!("{class}::{missing} ({}) is missing", leaf_of[&(class.clone(), missing.clone())].name));
        }
        for extra in have.difference(variants) {
            problems.push(format!("{class}::{extra} is no encoding of an implemented feature in class {class}"));
        }
    }
    for class in src.classes.keys() {
        if !wanted.contains_key(class) {
            problems.push(format!("{class} is no class with an encoding of an implemented feature"));
        }
    }

    // Structure, fixed bits and features.
    for ((class, variant), leaf) in &leaf_of {
        let occurrences: Vec<&Occurrence> = src.occurrences.iter().filter(|o| &o.class == class && &o.variant == variant).collect();
        let mut covered: u64 = 0;
        for (i, o) in occurrences.iter().enumerate() {
            if leaf.path.first() != Some(&o.top_fn) || leaf.path.last() != Some(&o.class_fn) {
                problems.push(format!("{class}::{variant} is decoded in {}/{}, Arm puts {} in {:?}", o.top_fn, o.class_fn, leaf.name, leaf.path));
            }
            if !o.cube.within(leaf.cube) {
                problems.push(format!(
                    "{class}::{variant}: an arm path fixes {:08x}/{:08x}, outside {} {:08x}/{:08x}",
                    o.cube.opcode, o.cube.mask, leaf.name, leaf.cube.opcode, leaf.cube.mask
                ));
            }
            if occurrences[..i].iter().any(|p| p.cube.meets(o.cube)) {
                problems.push(format!("{class}::{variant}: two arm paths overlap"));
            }
            covered += 1u64 << o.cube.free_bits();
        }
        if covered != 1u64 << leaf.cube.free_bits() {
            problems.push(format!(
                "{class}::{variant}: its arms fix other bits than {} {:08x}/{:08x} ({covered} of {} words)",
                leaf.name,
                leaf.cube.opcode,
                leaf.cube.mask,
                1u64 << leaf.cube.free_bits()
            ));
        }
        let tag = src.features.get(&(class.clone(), variant.clone())).cloned().unwrap_or_default();
        let tagged: BTreeSet<String> = Feature::ALL
            .iter()
            .filter(|f| format!("{f:?}") == tag)
            .filter_map(|f| f.arm_name().map(str::to_string))
            .collect();
        if tagged != leaf.features {
            problems.push(format!("{class}::{variant} is tagged {tag}; Arm's condition for {} names {:?}", leaf.name, leaf.features));
        }
    }

    // Field names and positions.
    for b in &src.bindings {
        match arm.fields.get(&b.node).and_then(|f| f.get(&b.name)) {
            Some(&(hi, lo)) if (hi, lo) == (b.hi, b.lo) => {}
            Some(&(hi, lo)) => problems.push(format!("{}: `{}` is bits {}:{}, Arm's {} field {} is {hi}:{lo}", b.function, b.name, b.hi, b.lo, b.node, b.name)),
            None => problems.push(format!("{}: `{}` is no field of Arm's {}", b.function, b.name, b.node)),
        }
    }

    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// Arm's allocation of words among the implemented encodings, indexed by
/// bits 31:21 and 15:10.
struct Allocation<'a> {
    leaves: Vec<&'a Leaf>,
    /// What [`decode`] must return for each leaf.
    expected: Vec<Encoding>,
    buckets: Vec<Vec<u16>>,
}

const INDEX_BITS: u32 = 0xFFE0_FC00;

fn index(word: u32) -> usize {
    (((word >> 21) << 6) | ((word >> 10) & 0x3F)) as usize
}

impl<'a> Allocation<'a> {
    fn new(arm: &'a Arm) -> (Allocation<'a>, Vec<String>) {
        let mut problems = Vec::new();
        let leaves: Vec<&Leaf> = arm.implemented().collect();
        let mut buckets = vec![Vec::new(); 1 << 17];
        for (i, leaf) in leaves.iter().enumerate() {
            let free = INDEX_BITS & !leaf.cube.mask;
            for bits in subsets(free) {
                buckets[index((leaf.cube.opcode & INDEX_BITS) | bits)].push(i as u16);
            }
        }
        let mut allocation = Allocation {
            leaves,
            expected: Vec::new(),
            buckets,
        };
        // What the decoder returns for each leaf, read off a word Arm
        // allocates to it (not a more specific one).
        let mut state = 0x5EED_u64;
        let mut expected = Vec::new();
        for (i, leaf) in allocation.leaves.iter().enumerate() {
            let want = rust_names(leaf).map(|(c, v)| format!("{c}({v})")).unwrap_or_default();
            let mut candidates = leaf
                .instance
                .into_iter()
                .chain((0..4096).map(|_| leaf.cube.opcode | (splitmix(&mut state) & !leaf.cube.mask)));
            let Some(word) = candidates.find(|&w| allocation.leaf(w) == Ok(Some(i))) else {
                problems.push(format!("{}: no word is allocated to it", leaf.name));
                expected.push(Encoding::PermUndef(super::PermUndef::Udf));
                continue;
            };
            match decode(word) {
                Some(e) if format!("{e:?}") == want => expected.push(e),
                other => {
                    problems.push(format!("{} {word:#010x}: decoded as {other:?}, not {want}", leaf.name));
                    expected.push(Encoding::PermUndef(super::PermUndef::Udf));
                }
            }
        }
        allocation.expected = expected;
        (allocation, problems)
    }

    /// The leaf Arm allocates `word` to: the most specific that matches.
    fn leaf(&self, word: u32) -> Result<Option<usize>, String> {
        let mut best: Option<usize> = None;
        let mut tied = false;
        for &i in &self.buckets[index(word)] {
            let leaf = self.leaves[i as usize];
            if leaf.cube.contains(word) && leaf.condition.eval(word) {
                match best {
                    Some(b) if self.leaves[b].cube.mask.count_ones() > leaf.cube.mask.count_ones() => {}
                    Some(b) if self.leaves[b].cube.mask.count_ones() == leaf.cube.mask.count_ones() => tied = true,
                    _ => {
                        best = Some(i as usize);
                        tied = false;
                    }
                }
            }
        }
        if tied {
            return Err(format!("{word:#010x}: two encodings of equal specificity match"));
        }
        Ok(best)
    }

    fn agrees(&self, word: u32) -> Result<(), String> {
        let leaf = self.leaf(word)?;
        let ours = decode(word);
        match (leaf, ours) {
            (None, None) => Ok(()),
            (Some(i), Some(e)) if self.expected[i] == e => Ok(()),
            (leaf, ours) => Err(format!("{word:#010x}: Arm {:?}, decoder {ours:?}", leaf.map(|i| &self.leaves[i].name))),
        }
    }
}

#[test]
#[cfg_attr(not(aarchmrs), ignore = "reads Arm's AARCHMRS from the directory RUSTY_BOX_AARCHMRS names")]
fn the_decoder_allocates_as_arm_does_on_sampled_words() {
    let arm = Arm::load();
    let (allocation, mut problems) = Allocation::new(&arm);
    let mut state = 0x5EED_u64;
    let mut words: Vec<u32> = Vec::new();
    for leaf in arm.leaves.iter() {
        for _ in 0..64 {
            words.push(leaf.cube.opcode | (splitmix(&mut state) & !leaf.cube.mask));
        }
    }
    words.extend((0..1_000_000).map(|_| splitmix(&mut state)));
    for word in words {
        if let Err(e) = allocation.agrees(word) {
            if problems.len() < 64 {
                problems.push(e);
            }
        }
    }
    assert!(problems.is_empty(), "AARCHMRS {}:\n{}", arm.release, problems.join("\n"));
}

#[test]
#[ignore = "exhaustive: every 32-bit word, on every host thread; reads Arm's AARCHMRS from the directory RUSTY_BOX_AARCHMRS names"]
fn the_decoder_allocates_as_arm_does_on_every_word() {
    let arm = Arm::load();
    let (allocation, problems) = Allocation::new(&arm);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let first = Mutex::new(Vec::new());
    let disagreements = Mutex::new(0u64);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()) as u64;
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        for t in 0..threads {
            let (allocation, first, disagreements) = (&allocation, &first, &disagreements);
            scope.spawn(move || {
                let mut count = 0u64;
                let mut word = t;
                while word < 1 << 32 {
                    if let Err(e) = allocation.agrees(word as u32) {
                        count += 1;
                        if let Ok(mut first) = first.lock() {
                            if first.len() < 32 {
                                first.push(e);
                            }
                        }
                    }
                    word += threads;
                }
                if let Ok(mut total) = disagreements.lock() {
                    *total += count;
                }
            });
        }
    });
    let total = disagreements.into_inner().unwrap_or(u64::MAX);
    std::eprintln!(
        "AARCHMRS {}: 2^32 words, {total} disagreements, {threads} threads, {:.1} s",
        arm.release,
        started.elapsed().as_secs_f64()
    );
    assert_eq!(total, 0, "{:#?}", first.into_inner().unwrap_or_default());
}
