use rusty_box::events::App;

struct Tick;
struct State(u32);

fn main() {
    let mut app = App::<State>::new()
        .on(|_: &Tick, state: &mut State| state.0 += 1)
        .with_state(State(0));
    let state = app.state();
    app.dispatch(&Tick).unwrap();
    assert_eq!(state.0, 0);
}
