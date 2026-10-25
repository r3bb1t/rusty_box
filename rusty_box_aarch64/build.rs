//! Sets `cfg(aarchmrs)` when `RUSTY_BOX_AARCHMRS` names Arm's machine-readable
//! specification, so that the tests reading it (tests/unit/spec.rs) run
//! instead of being ignored. The library itself does not depend on it.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(aarchmrs)");
    println!("cargo::rerun-if-env-changed=RUSTY_BOX_AARCHMRS");
    if std::env::var_os("RUSTY_BOX_AARCHMRS").is_some() {
        println!("cargo::rustc-cfg=aarchmrs");
    }
}
