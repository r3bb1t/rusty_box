//! Test inputs for `rusty_box_symbols`: every file its tests read is built
//! here, by the third-party writers (and, where none exists, by hand from the
//! format's published layout), so no binary fixture is committed except the
//! one `makecab` LZX cabinet (Task 7).
#![forbid(unsafe_code)]

pub mod cab;
pub mod compress;
pub mod dwarf;
pub mod elf;
pub mod mutate;
pub mod pdb;
pub mod pe;
