pub mod error;
pub use error::{Error, Result};

mod config;
pub(crate) mod cpu;
mod crc;
mod memory;

mod pc_system;

fn main() {
    println!("Hello, world!");
}
