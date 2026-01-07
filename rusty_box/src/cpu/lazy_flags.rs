use crate::config::BxAddress;

#[derive(Debug, Default)]
pub(crate) struct BxLazyflagsEntry {
    result: BxAddress,
    auxbits: BxAddress,
}
