// Arithmetic instruction handlers (ADD, SUB, etc.)
// Mirrors Bochs cpp/cpu/arith*.cc structure

pub mod arith16;
pub mod arith32;
pub mod arith8;

pub use arith16::*;
pub use arith32::*;
pub use arith8::*;