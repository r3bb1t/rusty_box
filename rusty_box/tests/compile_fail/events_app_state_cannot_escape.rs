use rusty_box::events::App;

struct Tick;
struct State(u32);

fn main() {
    let mut retained = None;
    let mut app = App::<State>::new()
        .on(|_: &Tick, state: &mut State| retained = Some(&mut state.0))
        .with_state(State(0));
    app.dispatch(&Tick).unwrap();
    assert!(retained.is_some());
}
