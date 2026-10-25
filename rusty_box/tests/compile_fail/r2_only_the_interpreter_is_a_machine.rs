// R2 (docs/safety-doctrine.md): states are types. The verbs every
// interpreted machine shares (`Machine`) belong to the interpreter's machine
// alone; a machine on any other engine — a hypervisor's, which runs through a
// driver of its own — does not have them, so stepping one that way is a
// compile error rather than a `step` that refuses at run time.
use rusty_box::emulator::{Emulator, Machine, SliceEngine};

fn drive<M: Machine>(machine: &mut M) -> u64 {
    machine.ticks()
}

fn on_any_engine<E: SliceEngine<()>>(machine: &mut Emulator<(), E>) -> u64 {
    drive(machine)
}

fn main() {}
