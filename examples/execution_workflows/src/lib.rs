#![forbid(unsafe_code)]

pub mod linux;

#[cfg(feature = "libafl")]
pub mod fixture;
