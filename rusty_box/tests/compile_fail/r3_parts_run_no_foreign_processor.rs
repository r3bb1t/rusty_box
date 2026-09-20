// R3 (docs/safety-doctrine.md): a processor executes only against the parts of
// its own machine. Outside the crate the one pairing is `Processor`, which only
// a machine destructuring its own `&mut self` builds; the parts it lends run
// nothing on their own, so one machine's parts cannot be handed another
// machine's processor.
use rusty_box::emulator::{EmulatorConfig, MachineBuilder};

fn main() {
    let mut a = MachineBuilder::new(EmulatorConfig::default()).build().unwrap();
    let mut b = MachineBuilder::new(EmulatorConfig::default()).build().unwrap();
    let foreign = b.processor(0).into_cpu();
    let _ = a.processor(0).io().emulate_one(foreign);
}
