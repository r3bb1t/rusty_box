//! Register a function, give the app state, and deliver a typed event.

use rusty_box::events::{App, ExecutionRequest};

struct SelectProcess(u32);

#[derive(Default)]
struct State {
    selected: Option<u32>,
}

// The first argument tells App::on which event this function handles.
fn select_process(event: &SelectProcess, state: &mut State) {
    state.selected = Some(event.0);
}

fn main() {
    let mut app = App::<State>::new()
        .on(select_process)
        .with_state(State::default());

    assert_eq!(app.dispatch(&SelectProcess(12)), Ok(ExecutionRequest::None));
    assert_eq!(app.state().selected, Some(12));
    println!("Selected process {:?}", app.state().selected);
}
