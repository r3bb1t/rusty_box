//! Sets `cfg(aarchmrs)` when `RUSTY_BOX_AARCHMRS` names Arm's machine-readable
//! specification, so that the tests reading it run instead of being ignored:
//! the decoder's against `Instructions.json` and `Features.json`
//! (tests/unit/spec.rs), and the System registers' against `Registers.json`
//! (tests/unit/sysreg_spec.rs). `cargo xtask ci` requires it unless given
//! `--skip-aarchmrs`. The library itself does not depend on it.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(aarchmrs)");
    println!("cargo::rerun-if-env-changed=RUSTY_BOX_AARCHMRS");
    if std::env::var_os("RUSTY_BOX_AARCHMRS").is_some() {
        println!("cargo::rustc-cfg=aarchmrs");
    }
}
