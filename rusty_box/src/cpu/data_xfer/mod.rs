// Data transfer instruction handlers (MOV, etc.)
// Mirrors Bochs cpp/cpu/data_xfer*.cc structure

pub mod data_xfer8;
pub mod data_xfer16;
pub mod data_xfer32;
pub mod data_xfer64;

pub use data_xfer8::*;
pub use data_xfer16::*;
pub use data_xfer32::*;
pub use data_xfer64::*;