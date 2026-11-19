use rusty_box::events::{Emit, Subscriptions};

struct Completed;
struct Failed;

fn main() {
    let mut handlers = Subscriptions::<Completed, ()>::new().on(|_| {});
    let _ = handlers.emit(&Failed, &mut ());
}
