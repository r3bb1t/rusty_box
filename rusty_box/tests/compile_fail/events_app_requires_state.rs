use rusty_box::events::App;

struct Tick;
struct State(u32);

fn main() {
    let mut app = App::new().route(|_: &Tick, state: &mut State| state.0 += 1);
    app.dispatch(&Tick).unwrap();
}
