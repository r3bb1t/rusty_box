//! Cross references (spec `2026-10-17-cross-references-design.md`): who in
//! the guest's loaded modules calls, jumps to, reads, writes or points at an
//! address. `image` copies a module out of a paused guest; the rest works on
//! that copy, off the run thread.

pub(crate) mod decode;
pub(crate) mod image;
pub(crate) mod index;
pub(crate) mod service;
#[cfg(test)]
pub(crate) mod test_image;
