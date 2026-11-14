//! The owner's source for the files a module's tables name: found on this
//! computer or in the PDB, checked against the build's checksum, decoded to
//! lines. Everything here reads files through the symbol engine's
//! [`FileStore`](rusty_box_symbols::store::FileStore) port, so tests run on
//! an in-memory one.

pub(crate) mod find;
pub(crate) mod folders;
pub(crate) mod text;
