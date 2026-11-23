//! Register a function, give the app state, and deliver a typed event.

use rusty_box::events::App;

struct SelectProcess(u32);

#[derive(Default)]
struct State {
    selected: Option<u32>,
}

// The argument pattern takes the ID straight out of the borrowed event.
fn select_process(&SelectProcess(pid): &SelectProcess, state: &mut State) {
    state.selected = Some(pid);
}

fn main() {
    let mut app = App::<State>::new()
        .on(select_process)
        .with_state(State::default());

    let delivery = app.dispatch(&SelectProcess(12));
    println!("Delivery: {delivery:?}");
    println!("Selected process {:?}", app.state().selected);
}
