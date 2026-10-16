//! The shape rules no compiler lint enforces: no tuple types or tuple
//! values (a tuple used only as a `match` scrutinee aside), no `dyn` trait
//! objects, no `impl Trait` types (owner rules, CLAUDE.md "The Safety
//! Doctrine" R0 and R8). Parsing with `syn` sees through line breaks and
//! nesting that hide a construct from a grep.

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

/// One construct the rules refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeFinding {
    pub line: usize,
    pub column: usize,
    pub what: &'static str,
}

/// Every refused construct in `source`, a whole Rust file.
pub fn check_source(source: &str) -> Result<Vec<ShapeFinding>, String> {
    let file = syn::parse_file(source).map_err(|error| error.to_string())?;
    let mut finder = Finder {
        findings: Vec::new(),
    };
    finder.visit_file(&file);
    Ok(finder.findings)
}

struct Finder {
    findings: Vec<ShapeFinding>,
}

impl Finder {
    fn found(&mut self, span: Span, what: &'static str) {
        let start = span.start();
        self.findings.push(ShapeFinding {
            line: start.line,
            column: start.column,
            what,
        });
    }
}

impl<'ast> Visit<'ast> for Finder {
    fn visit_type_tuple(&mut self, node: &'ast syn::TypeTuple) {
        if !node.elems.is_empty() {
            self.found(node.span(), "a tuple type");
        }
        visit::visit_type_tuple(self, node);
    }

    fn visit_expr_tuple(&mut self, node: &'ast syn::ExprTuple) {
        if !node.elems.is_empty() {
            self.found(node.span(), "a tuple expression");
        }
        visit::visit_expr_tuple(self, node);
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        // A tuple that exists only to be matched on is no data shape: its
        // elements are still checked, the tuple itself is not.
        match &*node.expr {
            syn::Expr::Tuple(tuple) => {
                for element in &tuple.elems {
                    self.visit_expr(element);
                }
            }
            other => self.visit_expr(other),
        }
        for arm in &node.arms {
            self.visit_arm(arm);
        }
    }

    fn visit_type_trait_object(&mut self, node: &'ast syn::TypeTraitObject) {
        self.found(node.span(), "a `dyn` trait object");
        visit::visit_type_trait_object(self, node);
    }

    fn visit_type_impl_trait(&mut self, node: &'ast syn::TypeImplTrait) {
        self.found(node.span(), "an `impl Trait` type");
        visit::visit_type_impl_trait(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whats(source: &str) -> Vec<&'static str> {
        check_source(source).unwrap().into_iter().map(|finding| finding.what).collect()
    }

    #[test]
    fn tuples_in_types_and_values_are_found() {
        assert_eq!(whats("fn f() -> (u8, u8) { todo!() }"), ["a tuple type"]);
        assert_eq!(whats("struct S { pairs: Vec<(u8, u8)> }"), ["a tuple type"]);
        assert_eq!(whats("fn f() { let pair = (1, 2); }"), ["a tuple expression"]);
        assert_eq!(
            whats("fn f(v: &[u8]) { let _ = v.iter().map(|b| (b, b)); }"),
            ["a tuple expression"]
        );
    }

    #[test]
    fn the_unit_type_and_a_match_scrutinee_are_not_tuples() {
        assert_eq!(whats("fn f() -> () {}"), Vec::<&str>::new());
        assert_eq!(
            whats("fn f(a: Option<u8>, b: Option<u8>) -> u8 { match (a, b) { (Some(x), Some(y)) => x ^ y, _ => 0 } }"),
            Vec::<&str>::new()
        );
        assert_eq!(
            whats("fn f(v: &[u8]) { for (i, b) in v.iter().enumerate() { let _ = i; let _ = b; } }"),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn dyn_and_impl_trait_are_found() {
        assert_eq!(whats("fn f(x: &dyn Fn()) {}"), ["a `dyn` trait object"]);
        assert_eq!(whats("fn f() -> impl Iterator<Item = u8> { 0..1 }"), ["an `impl Trait` type"]);
    }

    #[test]
    fn a_finding_names_its_line() {
        let findings = check_source("fn a() {}\nfn b() -> (u8, u8) { todo!() }\n").unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 2);
    }
}
