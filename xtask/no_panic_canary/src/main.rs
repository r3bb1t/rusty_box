//! See Cargo.toml: this must not link.

use no_panic::no_panic;

#[no_panic]
fn index_past_the_end(values: &[u8], at: usize) -> u8 {
    values[at]
}

fn main() {
    let values: Vec<u8> = std::env::args().map(|arg| arg.len().to_le_bytes()[0]).collect();
    println!("{}", index_past_the_end(&values, values.len()));
}
