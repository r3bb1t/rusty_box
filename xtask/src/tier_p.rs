//! The tier-P sweep (spec §15): every function `rusty_box_symbols` marks
//! `#[no_panic]` (as `#[cfg_attr(all(test, not(debug_assertions)), no_panic)]`)
//! is reached from the crate's own tests. The proof is made when the
//! crate's release test binary links, and a function no test reaches may
//! never be linked, so nothing proves it. Only a library file's
//! `#[cfg(test)]` modules count: an integration test (`tests/*.rs`) compiles
//! the crate without `cfg(test)`, so the guard is not there and its link
//! proves nothing. A function is reached when test code refers to it, or a
//! reached function does, since a function a linked one calls is linked
//! with it.
//!
//! A function is known by its module (its file's path under `src`, then
//! any inline `mod`), the type its `impl` names when it is a method, and its
//! name, so a tested function lends nothing to an untested namesake in
//! another module or on another type. A reference — a path in code, or in a
//! macro whose arguments parse as expressions (`assert_eq!(decode(…), …)`) —
//! resolves through the module's own functions, its `use` items (`crate`,
//! `self`, `super`, aliases, re-exports, `*`), `Self`, and `Type::name` for a
//! type's method. What does not resolve counts as every function of its
//! name, erring towards passing: `receiver.name(..)`, whose receiver's type
//! is not read, reaches every method of that name; a path through a generic
//! or a name nothing in scope defines reaches every function of that name;
//! and a macro whose arguments are not expressions holds every identifier it
//! spells.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{TokenStream, TokenTree};
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};

/// Which tests a file can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRole {
    /// A library file: its `#[cfg(test)]` modules are the crate's own tests,
    /// which link the proof.
    Library,
    /// An integration test (`tests/*.rs`): it links the crate without
    /// `cfg(test)`, so the names it holds prove nothing.
    Test,
}

/// A Rust source file to sweep.
pub struct Source {
    pub path: String,
    pub text: String,
    pub role: SourceRole,
}

/// A `#[no_panic]` function that no test reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Untested {
    pub path: String,
    pub function: String,
}

/// Every marked function in `sources` that the crate's test code does not
/// reach. A method is reported as `Type::name`.
pub fn untested_tier_p(sources: &[Source]) -> Result<Vec<Untested>, String> {
    let mut krate = Crate::default();
    for source in sources {
        let file = syn::parse_file(&source.text).map_err(|error| format!("{}: {error}", source.path))?;
        match source.role {
            // Parsed so a broken file still fails the sweep; what it holds
            // proves nothing.
            SourceRole::Test => continue,
            SourceRole::Library => {}
        }
        let module = module_of(&source.path);
        krate.modules.insert(module.clone());
        let mut sweep = Sweep {
            path: &source.path,
            module,
            owner: None,
            in_test: false,
            current: None,
            krate: &mut krate,
        };
        sweep.visit_file(&file);
    }
    let reached = krate.reached();
    Ok(krate
        .marked
        .iter()
        .filter(|marked| !reached.contains(&marked.id))
        .map(|marked| Untested { path: marked.path.clone(), function: marked.id.shown() })
        .collect())
}

/// The module a file under `src` holds: `src/lib.rs` is `crate`,
/// `src/pdb/mod.rs` is `crate::pdb`, `src/pdb/msf.rs` is `crate::pdb::msf`.
fn module_of(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|part| !part.is_empty()).collect();
    let under = match parts.iter().rposition(|part| *part == "src") {
        Some(at) => parts.get(at + 1..).unwrap_or(&[]),
        None => &parts[..],
    };
    let mut module = vec![CRATE.to_owned()];
    for (index, part) in under.iter().enumerate() {
        let file = index + 1 == under.len();
        let name = if file { part.strip_suffix(".rs").unwrap_or(part) } else { part };
        if file && matches!(name, "lib" | "main" | "mod") {
            continue;
        }
        module.push(name.to_owned());
    }
    module
}

/// The first segment of every path inside the crate.
const CRATE: &str = "crate";
/// How many `use` aliases one resolution follows, so a cycle of re-exports
/// ends.
const MAX_ALIAS_HOPS: u32 = 8;

/// A function: its module's path from `crate`, the type its `impl` names
/// when it is a method, and its name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FnId {
    module: Vec<String>,
    owner: Option<String>,
    name: String,
}

impl FnId {
    /// How a report names it: `Type::name` for a method.
    fn shown(&self) -> String {
        match &self.owner {
            Some(owner) => format!("{owner}::{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// A marked function and the file holding it.
struct Marked {
    id: FnId,
    path: String,
}

/// A module's `use` items, as written: each alias's path, and each path a
/// `*` opens. They are resolved against the module when read.
#[derive(Default)]
struct Scope {
    aliases: BTreeMap<String, Vec<String>>,
    globs: Vec<Vec<String>>,
}

/// A reference some code holds, with what resolving it needs.
enum Reference {
    /// A path: `decode`, `super::decode`, `Bytes::u8`, `Self::bit`.
    Path {
        module: Vec<String>,
        owner: Option<String>,
        segments: Vec<String>,
    },
    /// `receiver.name(..)`.
    Method { name: String },
    /// An identifier in a macro whose arguments are not expressions.
    Loose { name: String },
}

/// Which functions of a name an unresolved reference reaches.
#[derive(Clone, Copy)]
enum Candidates {
    Free,
    Methods,
    All,
}

/// Where a path's leading segments lead.
enum Expanded {
    /// A path inside the crate, from `crate`.
    Crate(Vec<String>),
    /// Another crate's: nothing here.
    External,
    /// Nothing in scope names its first segment as a module.
    Unknown,
}

/// What the library sources define and what their code refers to.
#[derive(Default)]
struct Crate {
    fns: BTreeSet<FnId>,
    modules: BTreeSet<Vec<String>>,
    scopes: BTreeMap<Vec<String>, Scope>,
    marked: Vec<Marked>,
    /// The references test code holds.
    seeds: Vec<Reference>,
    /// Per library function, the references its body holds.
    calls: BTreeMap<FnId, Vec<Reference>>,
}

impl Crate {
    /// What the seeds resolve to, and what every reached function's
    /// references resolve to, transitively.
    fn reached(&self) -> BTreeSet<FnId> {
        let mut reached = BTreeSet::new();
        let mut pending: Vec<FnId> = self.seeds.iter().flat_map(|reference| self.resolve(reference)).collect();
        while let Some(id) = pending.pop() {
            if reached.contains(&id) {
                continue;
            }
            if let Some(references) = self.calls.get(&id) {
                for reference in references {
                    pending.extend(self.resolve(reference).into_iter().filter(|callee| !reached.contains(callee)));
                }
            }
            reached.insert(id);
        }
        reached
    }

    /// Every function named `name` of the kind given.
    fn every(&self, name: &str, kind: Candidates) -> Vec<FnId> {
        self.fns
            .iter()
            .filter(|id| {
                id.name == name
                    && match kind {
                        Candidates::Free => id.owner.is_none(),
                        Candidates::Methods => id.owner.is_some(),
                        Candidates::All => true,
                    }
            })
            .cloned()
            .collect()
    }

    /// The methods named `name` of every type named `owner`.
    fn methods_of(&self, owner: &str, name: &str) -> Vec<FnId> {
        self.fns
            .iter()
            .filter(|id| id.name == name && id.owner.as_deref() == Some(owner))
            .cloned()
            .collect()
    }

    fn resolve(&self, reference: &Reference) -> Vec<FnId> {
        match reference {
            Reference::Method { name } => self.every(name, Candidates::Methods),
            Reference::Loose { name } => self.every(name, Candidates::All),
            Reference::Path { module, owner, segments } => {
                self.resolve_path(module, owner.as_deref(), segments, MAX_ALIAS_HOPS)
            }
        }
    }

    /// What `segments`, written in `module` (inside `owner`'s `impl` when
    /// there is one), names.
    fn resolve_path(&self, module: &[String], owner: Option<&str>, segments: &[String], hops: u32) -> Vec<FnId> {
        let Some((name, prefix)) = segments.split_last() else {
            return Vec::new();
        };
        match prefix {
            [] => self.resolve_name(module, name, hops),
            [only] if only == "Self" => match owner {
                Some(owner) => self.methods_of(owner, name),
                None => self.every(name, Candidates::Methods),
            },
            _ => match self.expand(module, prefix, hops) {
                Expanded::External => Vec::new(),
                Expanded::Crate(path) => self.resolve_in_crate(&path, name, hops),
                Expanded::Unknown => {
                    // `Type::name`, the type in scope by name: its methods.
                    let methods = match prefix {
                        [type_name] => self.methods_of(type_name, name),
                        _ => Vec::new(),
                    };
                    if methods.is_empty() {
                        self.every(name, Candidates::All)
                    } else {
                        methods
                    }
                }
            },
        }
    }

    /// A bare `name` in `module`: its own function, an imported one, or one
    /// a `*` import brings in.
    fn resolve_name(&self, module: &[String], name: &str, hops: u32) -> Vec<FnId> {
        let own = FnId { module: module.to_vec(), owner: None, name: name.to_owned() };
        if self.fns.contains(&own) {
            return vec![own];
        }
        if let Some(scope) = self.scopes.get(module) {
            if let Some(target) = scope.aliases.get(name) {
                return match hops.checked_sub(1) {
                    Some(left) => self.resolve_path(module, None, target, left),
                    None => self.every(name, Candidates::Free),
                };
            }
            for glob in &scope.globs {
                if let Expanded::Crate(opened) = self.expand(module, glob, hops) {
                    // A `*` brings in the module's own functions and what it
                    // imports.
                    let id = FnId { module: opened.clone(), owner: None, name: name.to_owned() };
                    if self.fns.contains(&id) {
                        return vec![id];
                    }
                    let imported = self.scopes.get(&opened).is_some_and(|scope| scope.aliases.contains_key(name));
                    if let (true, Some(left)) = (imported, hops.checked_sub(1)) {
                        return self.resolve_name(&opened, name, left);
                    }
                }
            }
        }
        self.every(name, Candidates::Free)
    }

    /// `name` under `path`, a path inside the crate: a module's function (or
    /// its re-export), else a method of the type `path` ends in.
    fn resolve_in_crate(&self, path: &[String], name: &str, hops: u32) -> Vec<FnId> {
        if self.modules.contains(path) {
            let id = FnId { module: path.to_vec(), owner: None, name: name.to_owned() };
            if self.fns.contains(&id) {
                return vec![id];
            }
            let re_exported = self.scopes.get(path).is_some_and(|scope| scope.aliases.contains_key(name));
            return match (re_exported, hops.checked_sub(1)) {
                (true, Some(left)) => self.resolve_name(path, name, left),
                (true, None) | (false, _) => self.every(name, Candidates::Free),
            };
        }
        let methods = path.last().map(|type_name| self.methods_of(type_name, name)).unwrap_or_default();
        if methods.is_empty() {
            self.every(name, Candidates::All)
        } else {
            methods
        }
    }

    /// Where `path`, written in `module`, leads: `crate`, `self` and `super`
    /// as Rust reads them, a child module, an alias, or another crate.
    fn expand(&self, module: &[String], path: &[String], hops: u32) -> Expanded {
        let Some((first, rest)) = path.split_first() else {
            return Expanded::Crate(module.to_vec());
        };
        let joined = |base: Vec<String>| {
            let mut full = base;
            full.extend(rest.iter().cloned());
            Expanded::Crate(full)
        };
        match first.as_str() {
            CRATE => joined(vec![CRATE.to_owned()]),
            "self" => joined(module.to_vec()),
            "super" => {
                let mut parent = module.to_vec();
                parent.pop();
                self.expand(&parent, rest, hops)
            }
            "std" | "core" | "alloc" => Expanded::External,
            _ => {
                let mut child = module.to_vec();
                child.push(first.clone());
                if self.modules.contains(&child) {
                    return joined(child);
                }
                let alias = self.scopes.get(module).and_then(|scope| scope.aliases.get(first));
                match (alias, hops.checked_sub(1)) {
                    (Some(target), Some(left)) => match self.expand(module, target, left) {
                        Expanded::Crate(base) => joined(base),
                        Expanded::External => Expanded::External,
                        Expanded::Unknown => Expanded::Unknown,
                    },
                    (Some(_), None) | (None, _) => Expanded::Unknown,
                }
            }
        }
    }
}

fn is_tier_p(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::Path(path) => path.is_ident("no_panic"),
        syn::Meta::List(list) => list.path.is_ident("cfg_attr") && list.tokens.to_string().contains("no_panic"),
        syn::Meta::NameValue(_) => false,
    })
}

fn is_cfg_test(attr: &syn::Attribute) -> bool {
    match &attr.meta {
        syn::Meta::List(list) => list.path.is_ident("cfg") && list.tokens.to_string() == "test",
        syn::Meta::Path(_) | syn::Meta::NameValue(_) => false,
    }
}

/// Every identifier in `tokens`, groups included.
fn collect_idents(tokens: TokenStream, into: &mut BTreeSet<String>) {
    for tree in tokens {
        match tree {
            TokenTree::Ident(ident) => {
                into.insert(ident.to_string());
            }
            TokenTree::Group(group) => collect_idents(group.stream(), into),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
}

/// The type an `impl` block names, when it names one by path.
fn type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

/// A `use` tree's aliases and `*` paths, each path as written from the
/// tree's root.
fn flatten_use(tree: &syn::UseTree, prefix: &mut Vec<String>, scope: &mut Scope) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            flatten_use(&path.tree, prefix, scope);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut target = prefix.clone();
            // `use a::b::{self}` names `b`.
            if name.ident != "self" {
                target.push(name.ident.to_string());
            }
            if let Some(alias) = target.last().cloned() {
                scope.aliases.insert(alias, target);
            }
        }
        syn::UseTree::Rename(rename) => {
            let mut target = prefix.clone();
            if rename.ident != "self" {
                target.push(rename.ident.to_string());
            }
            scope.aliases.insert(rename.rename.to_string(), target);
        }
        syn::UseTree::Glob(_) => scope.globs.push(prefix.clone()),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                flatten_use(item, prefix, scope);
            }
        }
    }
}

struct Sweep<'s> {
    path: &'s str,
    /// The module being read, from `crate`.
    module: Vec<String>,
    /// The type of the `impl` block being read.
    owner: Option<String>,
    /// Inside a `#[cfg(test)]` module.
    in_test: bool,
    /// The function whose body is being read.
    current: Option<FnId>,
    krate: &'s mut Crate,
}

impl Sweep<'_> {
    /// Records the function `name` defined here; marked, it is swept.
    fn define(&mut self, attrs: &[syn::Attribute], name: &syn::Ident) -> FnId {
        let id = FnId { module: self.module.clone(), owner: self.owner.clone(), name: name.to_string() };
        self.krate.fns.insert(id.clone());
        if is_tier_p(attrs) {
            self.krate.marked.push(Marked { id: id.clone(), path: self.path.to_owned() });
        }
        id
    }

    /// Records a reference the code being read holds: test code's seeds the
    /// reach, a library function's adds to what that function calls.
    fn hold(&mut self, reference: Reference) {
        if self.in_test {
            self.krate.seeds.push(reference);
        } else if let Some(current) = &self.current {
            self.krate.calls.entry(current.clone()).or_default().push(reference);
        }
    }

    /// Reads a function's body as `id`'s, then restores the enclosing one.
    fn within<F: FnOnce(&mut Self)>(&mut self, id: FnId, read: F) {
        let outer = self.current.replace(id);
        read(self);
        self.current = outer;
    }
}

impl<'ast> Visit<'ast> for Sweep<'_> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        // A function item is a free function wherever it is written.
        let owner = self.owner.take();
        let id = self.define(&node.attrs, &node.sig.ident);
        self.within(id, |sweep| visit::visit_item_fn(sweep, node));
        self.owner = owner;
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let outer = std::mem::replace(&mut self.owner, type_name(&node.self_ty));
        visit::visit_item_impl(self, node);
        self.owner = outer;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let id = self.define(&node.attrs, &node.sig.ident);
        self.within(id, |sweep| visit::visit_impl_item_fn(sweep, node));
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        let outer_test = self.in_test;
        let outer_owner = self.owner.take();
        self.in_test = outer_test || node.attrs.iter().any(is_cfg_test);
        self.module.push(node.ident.to_string());
        self.krate.modules.insert(self.module.clone());
        visit::visit_item_mod(self, node);
        self.module.pop();
        self.owner = outer_owner;
        self.in_test = outer_test;
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        let scope = self.krate.scopes.entry(self.module.clone()).or_default();
        flatten_use(&node.tree, &mut Vec::new(), scope);
        visit::visit_item_use(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        let segments = node.path.segments.iter().map(|segment| segment.ident.to_string()).collect();
        self.hold(Reference::Path { module: self.module.clone(), owner: self.owner.clone(), segments });
        visit::visit_expr_path(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        self.hold(Reference::Method { name: node.method.to_string() });
        visit::visit_expr_method_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // Arguments that parse as expressions are read as code; others are
        // their identifiers.
        let parsed = node.parse_body_with(Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated);
        match parsed {
            Ok(arguments) => {
                for argument in &arguments {
                    visit::visit_expr(self, argument);
                }
            }
            Err(_) => {
                let mut idents = BTreeSet::new();
                collect_idents(node.tokens.clone(), &mut idents);
                for name in idents {
                    self.hold(Reference::Loose { name });
                }
            }
        }
        visit::visit_macro(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(text: &str) -> Source {
        Source { path: "src/read.rs".to_owned(), text: text.to_owned(), role: SourceRole::Library }
    }

    fn names(untested: Vec<Untested>) -> Vec<String> {
        untested.into_iter().map(|found| found.function).collect()
    }

    #[test]
    fn a_marked_function_named_in_its_module_tests_passes() {
        let source = library(
            "#[cfg_attr(all(test, not(debug_assertions)), no_panic)]\npub fn decode(b: &[u8]) -> u8 { 0 }\n\
             #[cfg(test)]\nmod tests { #[test] fn t() { assert_eq!(super::decode(&[]), 0); } }",
        );
        assert_eq!(names(untested_tier_p(&[source]).unwrap()), Vec::<String>::new());
    }

    #[test]
    fn a_marked_function_no_test_names_is_reported_with_its_file() {
        let source = library("#[no_panic]\npub fn decode(b: &[u8]) -> u8 { 0 }\npub fn caller() -> u8 { decode(&[]) }");
        let untested = untested_tier_p(&[source]).unwrap();
        assert_eq!(untested, [Untested { path: "src/read.rs".to_owned(), function: "decode".to_owned() }]);
    }

    /// An integration test compiles the crate without `cfg(test)`, so the
    /// guard is not there and its link proves nothing.
    #[test]
    fn a_marked_method_named_only_in_an_integration_test_is_reported() {
        let source = library(
            "pub struct Bytes; impl Bytes {\n#[cfg_attr(all(test, not(debug_assertions)), no_panic)]\npub fn u32_le(&self) -> u32 { 0 } }",
        );
        let test = Source {
            path: "tests/corruption.rs".to_owned(),
            text: "#[test] fn t() { let b = Bytes; assert_eq!(b.u32_le(), 0); }".to_owned(),
            role: SourceRole::Test,
        };
        assert_eq!(names(untested_tier_p(&[source, test]).unwrap()), ["Bytes::u32_le"]);
    }

    #[test]
    fn a_marked_method_named_in_its_crate_tests_passes() {
        let source = library(
            "pub struct Bytes; impl Bytes {\n#[cfg_attr(all(test, not(debug_assertions)), no_panic)]\npub fn u32_le(&self) -> u32 { 0 } }\n\
             #[cfg(test)]\nmod tests { #[test] fn t() { let b = super::Bytes; assert_eq!(b.u32_le(), 0); } }",
        );
        assert_eq!(names(untested_tier_p(&[source]).unwrap()), Vec::<String>::new());
    }

    /// A marked helper a tested function calls is linked into the test
    /// binary with it, so the link proves it.
    #[test]
    fn a_marked_helper_reached_through_a_tested_function_passes() {
        let source = library(
            "#[no_panic]\nfn bit(b: &[u8]) -> u8 { 0 }\nfn bits(b: &[u8]) -> u8 { bit(b) }\n\
             pub fn decode(b: &[u8]) -> u8 { bits(b) }\n\
             #[cfg(test)]\nmod tests { #[test] fn t() { assert_eq!(super::decode(&[]), 0); } }",
        );
        assert_eq!(names(untested_tier_p(&[source]).unwrap()), Vec::<String>::new());
    }

    fn source(path: &str, text: &str) -> Source {
        Source { path: path.to_owned(), text: text.to_owned(), role: SourceRole::Library }
    }

    const MARK: &str = "#[cfg_attr(all(test, not(debug_assertions)), no_panic)]";

    /// `cab::decode_header` is tested; `pdb::dbi::decode_header` is not, and
    /// sharing the name does not lend it the proof.
    #[test]
    fn a_tested_function_does_not_cover_an_untested_namesake_elsewhere() {
        let cab = source(
            "C:\\repo\\rusty_box_symbols\\src\\cab.rs",
            &format!(
                "{MARK}\npub fn decode_header(b: &[u8]) -> u8 {{ 0 }}\n\
                 #[cfg(test)]\nmod tests {{ use super::*; #[test] fn t() {{ assert_eq!(decode_header(&[]), 0); }} }}"
            ),
        );
        let dbi = source("C:\\repo\\rusty_box_symbols\\src\\pdb\\dbi.rs", &format!("{MARK}\npub fn decode_header(b: &[u8]) -> u8 {{ 0 }}"));
        let untested = untested_tier_p(&[cab, dbi]).unwrap();
        assert_eq!(
            untested,
            [Untested { path: "C:\\repo\\rusty_box_symbols\\src\\pdb\\dbi.rs".to_owned(), function: "decode_header".to_owned() }]
        );
    }

    /// A call through a `use` import reaches the function it imports, and
    /// only that one.
    #[test]
    fn a_call_through_a_use_import_reaches_its_target() {
        let read = source("src/read.rs", &format!("{MARK}\npub fn decode(b: &[u8]) -> u8 {{ 0 }}"));
        let other = source("src/other.rs", &format!("{MARK}\npub fn decode(b: &[u8]) -> u8 {{ 0 }}"));
        let cab = source(
            "src/cab/mod.rs",
            "use crate::read::decode;\npub fn open(b: &[u8]) -> u8 { decode(b) }\n\
             #[cfg(test)]\nmod tests { #[test] fn t() { assert_eq!(super::open(&[]), 0); } }",
        );
        let untested = untested_tier_p(&[read, other, cab]).unwrap();
        assert_eq!(untested, [Untested { path: "src/other.rs".to_owned(), function: "decode".to_owned() }]);
    }

    /// `Bytes::u8(..)` through an imported type reaches `Bytes`'s method,
    /// not `Cursor`'s.
    #[test]
    fn a_type_qualified_call_through_an_imported_type_reaches_that_types_method() {
        let read = source(
            "src/read.rs",
            &format!(
                "pub struct Bytes; impl Bytes {{ {MARK}\npub fn u8(&self) -> u8 {{ 0 }} }}\n\
                 pub struct Cursor; impl Cursor {{ {MARK}\npub fn u8(&self) -> u8 {{ 0 }} }}"
            ),
        );
        let msf = source(
            "src/pdb/msf.rs",
            "use crate::read::Bytes;\n#[cfg(test)]\nmod tests { use super::*; #[test] fn t() { assert_eq!(Bytes::u8(&Bytes), 0); } }",
        );
        let untested = untested_tier_p(&[read, msf]).unwrap();
        assert_eq!(untested, [Untested { path: "src/read.rs".to_owned(), function: "Cursor::u8".to_owned() }]);
    }

    /// `receiver.name()` does not say the receiver's type, so it reaches
    /// every method of that name: the one case still decided by name.
    #[test]
    fn a_method_call_reaches_every_method_of_its_name() {
        let read = source(
            "src/read.rs",
            &format!(
                "pub struct Bytes; impl Bytes {{ {MARK}\npub fn u8(&self) -> u8 {{ 0 }} }}\n\
                 pub struct Cursor; impl Cursor {{ {MARK}\npub fn u8(&self) -> u8 {{ 0 }} }}\n\
                 #[cfg(test)]\nmod tests {{ #[test] fn t() {{ assert_eq!(super::Bytes.u8(), 0); }} }}"
            ),
        );
        assert_eq!(names(untested_tier_p(&[read]).unwrap()), Vec::<String>::new());
    }

    #[test]
    fn an_unmarked_function_is_not_swept() {
        let source = library("pub fn assemble(b: &[u8]) -> Vec<u8> { b.to_vec() }");
        assert_eq!(names(untested_tier_p(&[source]).unwrap()), Vec::<String>::new());
    }
}
