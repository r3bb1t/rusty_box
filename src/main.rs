#[cfg(target_pointer_width = "32")]
compile_error!("This project requires a target with at least 64-bit usize.");

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
