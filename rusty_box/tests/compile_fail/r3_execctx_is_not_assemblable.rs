// R3 (docs/safety-doctrine.md): machine parts have no loose currency. The ONLY
// way an ExecCtx comes to exist is a machine facade destructuring its own
// `&mut self`; the module is crate-private, so external assembly is not merely
// discouraged — the path does not resolve.
fn main() {
    let _ = rusty_box::cpu::exec_ctx::ExecCtx::new;
}
