use rusty_box::emulator::hardware::PcInterrupt;
use rusty_box::events::{ExecutionRequest, Subscriptions};

fn main() {
    let mut retained: Option<&mut PcInterrupt<'_>> = None;
    let _handlers = Subscriptions::<(), ()>::new().on_with(|_: &(), irq: &mut PcInterrupt<'_>| {
        retained = Some(irq);
        Ok::<_, ()>(ExecutionRequest::None)
    });
}
