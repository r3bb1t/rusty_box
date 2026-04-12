use crate::config::BxAddress;

#[derive(Debug, Default)]
pub(crate) struct BxLazyflagsEntry {
    pub(super) result: BxAddress,
    pub(super) auxbits: BxAddress,
}
