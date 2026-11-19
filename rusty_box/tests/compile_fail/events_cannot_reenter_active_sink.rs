use rusty_box::events::{Emit, Subscriptions};

fn main() {
    let mut handlers = Subscriptions::<u32, ()>::new().on(|_| {});
    let _ = handlers.emit(&1, &mut handlers);
}
