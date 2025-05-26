#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

#[cfg(all(feature = "bx_little_endian", feature = "bx_big_endian"))]
compile_error!(
    r#"You can't have both "bx_little_endian" and "bx_big_endian" features enabled. Please, remove one of them"#
);

pub mod error;
pub use error::{Error, Result};

mod config;
pub mod cpu;
mod crc;
mod memory;
mod misc;
mod params;
mod pc_system;
