use crate::config::BxAddress;

#[derive(Debug)]
pub(crate) struct BxLazyflagsEntry {
    result: BxAddress,
    auxbits: BxAddress,
}
