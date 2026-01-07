// Data transfer instruction handlers (MOV, etc.)
// Mirrors Bochs cpp/cpu/data_xfer*.cc structure

pub mod data_xfer32;

pub use data_xfer32::*;
