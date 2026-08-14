//! The doctrine fixture registry (docs/safety-doctrine.md, R9's mechanical half).
//!
//! Each fixture pins a TYPE-LEVEL guarantee: the claim is not that the code is
//! wrong, but that it CANNOT COMPILE — and the .stderr golden records the exact
//! diagnostic a user sees. One fixture per claim; a fixture that proves
//! something weaker than it appears is worse than none (llvmkit's rule).
//!
//! Run: `cargo test -p rusty_box --test compile_fail --features std`
//! Regenerate goldens after a toolchain bump: `TRYBUILD=overwrite cargo test …`
#![cfg(feature = "std")]

#[test]
fn doctrine_fixtures() {
    let t = trybuild::TestCases::new();
    // R3: an ExecCtx cannot be assembled outside the crate — machine parts
    // have no loose currency; the module itself is not reachable.
    t.compile_fail("tests/compile_fail/r3_execctx_is_not_assemblable.rs");
    // R2: a bare CPU cannot execute an instruction — execution requires the
    // machine context, so "CPU wired to nothing" is unrepresentable.
    t.compile_fail("tests/compile_fail/r2_bare_cpu_cannot_execute.rs");
}
