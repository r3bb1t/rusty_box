#![cfg_attr(not(feature = "std"), no_std)]
// Bochs port: function/field/type names intentionally match C++ originals
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]
// Ported code not yet wired up — suppress until features are implemented
#![allow(dead_code, unused_variables, unused_assignments)]
// Union field accesses that may need unsafe in different configurations
#![allow(unused_unsafe)]
// Bochs port exposes types across module boundaries incrementally
#![allow(private_interfaces)]
// Feature flags for future extensions (SSE, AVX, EVEX, VMX, SVM, etc.)
#![allow(unexpected_cfgs)]
extern crate alloc;

//#[cfg(all(feature = "bx_little_endian", feature = "bx_big_endian"))]
//compile_error!(
//    r#"You can't have both "bx_little_endian" and "bx_big_endian" features enabled. Please, remove one of them"#
//);

pub mod error;
pub use error::{Error, Result};

mod config;
pub mod cpu;
mod crc;
pub mod emulator;
pub mod gui;
pub mod iodev;
pub mod memory;
mod misc;
pub mod params;
pub mod pc_system;
pub mod snapshot;

// Re-export commonly used types
pub use emulator::{Emulator, EmulatorConfig};
