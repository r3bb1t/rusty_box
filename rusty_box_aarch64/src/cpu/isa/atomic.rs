//! The atomic and ordered classes of Arm group `ldst` (Arm ARM C4.1,
//! "Loads and Stores"): `memop` (the atomic memory operations and SWP of
//! FEAT_LSE, and LDAPR of FEAT_LRCPC), `comswap` and `comswappr` (CAS and
//! CASP, FEAT_LSE), and `ldapstl_unscaled` (LDAPUR and STLUR, FEAT_LRCPC2).
//! The `memop` encodings of FEAT_LSFE, FEAT_LS64 and FEAT_THE, which this
//! processor does not implement, are absent.

use super::Feature;
use crate::cpu::exec::bits;

/// Arm group `memop`: Atomic memory operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Memop {
    Ldaddb32,
    Ldclrb32,
    Ldeorb32,
    Ldsetb32,
    Ldsmaxb32,
    Ldsminb32,
    Ldumaxb32,
    Lduminb32,
    Swpb32,
    Ldaddlb32,
    Ldclrlb32,
    Ldeorlb32,
    Ldsetlb32,
    Ldsmaxlb32,
    Ldsminlb32,
    Ldumaxlb32,
    Lduminlb32,
    Swplb32,
    Ldaddab32,
    Ldclrab32,
    Ldeorab32,
    Ldsetab32,
    Ldsmaxab32,
    Ldsminab32,
    Ldumaxab32,
    Lduminab32,
    Swpab32,
    Ldaprb32l,
    Ldaddalb32,
    Ldclralb32,
    Ldeoralb32,
    Ldsetalb32,
    Ldsmaxalb32,
    Ldsminalb32,
    Ldumaxalb32,
    Lduminalb32,
    Swpalb32,
    Ldaddh32,
    Ldclrh32,
    Ldeorh32,
    Ldseth32,
    Ldsmaxh32,
    Ldsminh32,
    Ldumaxh32,
    Lduminh32,
    Swph32,
    Ldaddlh32,
    Ldclrlh32,
    Ldeorlh32,
    Ldsetlh32,
    Ldsmaxlh32,
    Ldsminlh32,
    Ldumaxlh32,
    Lduminlh32,
    Swplh32,
    Ldaddah32,
    Ldclrah32,
    Ldeorah32,
    Ldsetah32,
    Ldsmaxah32,
    Ldsminah32,
    Ldumaxah32,
    Lduminah32,
    Swpah32,
    Ldaprh32l,
    Ldaddalh32,
    Ldclralh32,
    Ldeoralh32,
    Ldsetalh32,
    Ldsmaxalh32,
    Ldsminalh32,
    Ldumaxalh32,
    Lduminalh32,
    Swpalh32,
    Ldadd32,
    Ldclr32,
    Ldeor32,
    Ldset32,
    Ldsmax32,
    Ldsmin32,
    Ldumax32,
    Ldumin32,
    Swp32,
    Ldaddl32,
    Ldclrl32,
    Ldeorl32,
    Ldsetl32,
    Ldsmaxl32,
    Ldsminl32,
    Ldumaxl32,
    Lduminl32,
    Swpl32,
    Ldadda32,
    Ldclra32,
    Ldeora32,
    Ldseta32,
    Ldsmaxa32,
    Ldsmina32,
    Ldumaxa32,
    Ldumina32,
    Swpa32,
    Ldapr32l,
    Ldaddal32,
    Ldclral32,
    Ldeoral32,
    Ldsetal32,
    Ldsmaxal32,
    Ldsminal32,
    Ldumaxal32,
    Lduminal32,
    Swpal32,
    Ldadd64,
    Ldclr64,
    Ldeor64,
    Ldset64,
    Ldsmax64,
    Ldsmin64,
    Ldumax64,
    Ldumin64,
    Swp64,
    Ldaddl64,
    Ldclrl64,
    Ldeorl64,
    Ldsetl64,
    Ldsmaxl64,
    Ldsminl64,
    Ldumaxl64,
    Lduminl64,
    Swpl64,
    Ldadda64,
    Ldclra64,
    Ldeora64,
    Ldseta64,
    Ldsmaxa64,
    Ldsmina64,
    Ldumaxa64,
    Ldumina64,
    Swpa64,
    Ldapr64l,
    Ldaddal64,
    Ldclral64,
    Ldeoral64,
    Ldsetal64,
    Ldsmaxal64,
    Ldsminal64,
    Ldumaxal64,
    Lduminal64,
    Swpal64,
}

/// Arm group `comswap`: Compare and swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Comswap {
    CasbC32,
    CaslbC32,
    CasabC32,
    CasalbC32,
    CashC32,
    CaslhC32,
    CasahC32,
    CasalhC32,
    CasC32,
    CaslC32,
    CasaC32,
    CasalC32,
    CasC64,
    CaslC64,
    CasaC64,
    CasalC64,
}

/// Arm group `comswappr`: Compare and swap pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Comswappr {
    CaspCp32,
    CasplCp32,
    CaspaCp32,
    CaspalCp32,
    CaspCp64,
    CasplCp64,
    CaspaCp64,
    CaspalCp64,
}

/// Arm group `ldapstl_unscaled`: Load/store ordered (unscaled immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdapstlUnscaled {
    Stlurb32,
    Ldapurb32,
    Ldapursb64,
    Ldapursb32,
    Stlurh32,
    Ldapurh32,
    Ldapursh64,
    Ldapursh32,
    Stlur32,
    Ldapur32,
    Ldapursw64,
    Stlur64,
    Ldapur64,
}

pub(super) fn memop(word: u32) -> Option<Memop> {
    let size = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let a = bits(word, 23, 23);
    let r = bits(word, 22, 22);
    let rs = bits(word, 20, 16);
    let o3 = bits(word, 15, 15);
    let opc = bits(word, 14, 12);
    match (size, vr, a, r, o3, opc) {
        (0b00, 0b0, 0b0, 0b0, 0b0, 0b000) => Some(Memop::Ldaddb32),
        (0b00, 0b0, 0b0, 0b0, 0b0, 0b001) => Some(Memop::Ldclrb32),
        (0b00, 0b0, 0b0, 0b0, 0b0, 0b010) => Some(Memop::Ldeorb32),
        (0b00, 0b0, 0b0, 0b0, 0b0, 0b011) => Some(Memop::Ldsetb32),
        (0b00, 0b0, 0b0, 0b0, 0b0, 0b100) => Some(Memop::Ldsmaxb32),
        (0b00, 0b0, 0b0, 0b0, 0b0, 0b101) => Some(Memop::Ldsminb32),
        (0b00, 0b0, 0b0, 0b0, 0b0, 0b110) => Some(Memop::Ldumaxb32),
        (0b00, 0b0, 0b0, 0b0, 0b0, 0b111) => Some(Memop::Lduminb32),
        (0b00, 0b0, 0b0, 0b0, 0b1, 0b000) => Some(Memop::Swpb32),
        (0b00, 0b0, 0b0, 0b1, 0b0, 0b000) => Some(Memop::Ldaddlb32),
        (0b00, 0b0, 0b0, 0b1, 0b0, 0b001) => Some(Memop::Ldclrlb32),
        (0b00, 0b0, 0b0, 0b1, 0b0, 0b010) => Some(Memop::Ldeorlb32),
        (0b00, 0b0, 0b0, 0b1, 0b0, 0b011) => Some(Memop::Ldsetlb32),
        (0b00, 0b0, 0b0, 0b1, 0b0, 0b100) => Some(Memop::Ldsmaxlb32),
        (0b00, 0b0, 0b0, 0b1, 0b0, 0b101) => Some(Memop::Ldsminlb32),
        (0b00, 0b0, 0b0, 0b1, 0b0, 0b110) => Some(Memop::Ldumaxlb32),
        (0b00, 0b0, 0b0, 0b1, 0b0, 0b111) => Some(Memop::Lduminlb32),
        (0b00, 0b0, 0b0, 0b1, 0b1, 0b000) => Some(Memop::Swplb32),
        (0b00, 0b0, 0b1, 0b0, 0b0, 0b000) => Some(Memop::Ldaddab32),
        (0b00, 0b0, 0b1, 0b0, 0b0, 0b001) => Some(Memop::Ldclrab32),
        (0b00, 0b0, 0b1, 0b0, 0b0, 0b010) => Some(Memop::Ldeorab32),
        (0b00, 0b0, 0b1, 0b0, 0b0, 0b011) => Some(Memop::Ldsetab32),
        (0b00, 0b0, 0b1, 0b0, 0b0, 0b100) => Some(Memop::Ldsmaxab32),
        (0b00, 0b0, 0b1, 0b0, 0b0, 0b101) => Some(Memop::Ldsminab32),
        (0b00, 0b0, 0b1, 0b0, 0b0, 0b110) => Some(Memop::Ldumaxab32),
        (0b00, 0b0, 0b1, 0b0, 0b0, 0b111) => Some(Memop::Lduminab32),
        (0b00, 0b0, 0b1, 0b0, 0b1, 0b000) => Some(Memop::Swpab32),
        (0b00, 0b0, 0b1, 0b0, 0b1, 0b100) => match rs {
            0b11111 => Some(Memop::Ldaprb32l),
            _ => None,
        },
        (0b00, 0b0, 0b1, 0b1, 0b0, 0b000) => Some(Memop::Ldaddalb32),
        (0b00, 0b0, 0b1, 0b1, 0b0, 0b001) => Some(Memop::Ldclralb32),
        (0b00, 0b0, 0b1, 0b1, 0b0, 0b010) => Some(Memop::Ldeoralb32),
        (0b00, 0b0, 0b1, 0b1, 0b0, 0b011) => Some(Memop::Ldsetalb32),
        (0b00, 0b0, 0b1, 0b1, 0b0, 0b100) => Some(Memop::Ldsmaxalb32),
        (0b00, 0b0, 0b1, 0b1, 0b0, 0b101) => Some(Memop::Ldsminalb32),
        (0b00, 0b0, 0b1, 0b1, 0b0, 0b110) => Some(Memop::Ldumaxalb32),
        (0b00, 0b0, 0b1, 0b1, 0b0, 0b111) => Some(Memop::Lduminalb32),
        (0b00, 0b0, 0b1, 0b1, 0b1, 0b000) => Some(Memop::Swpalb32),
        (0b01, 0b0, 0b0, 0b0, 0b0, 0b000) => Some(Memop::Ldaddh32),
        (0b01, 0b0, 0b0, 0b0, 0b0, 0b001) => Some(Memop::Ldclrh32),
        (0b01, 0b0, 0b0, 0b0, 0b0, 0b010) => Some(Memop::Ldeorh32),
        (0b01, 0b0, 0b0, 0b0, 0b0, 0b011) => Some(Memop::Ldseth32),
        (0b01, 0b0, 0b0, 0b0, 0b0, 0b100) => Some(Memop::Ldsmaxh32),
        (0b01, 0b0, 0b0, 0b0, 0b0, 0b101) => Some(Memop::Ldsminh32),
        (0b01, 0b0, 0b0, 0b0, 0b0, 0b110) => Some(Memop::Ldumaxh32),
        (0b01, 0b0, 0b0, 0b0, 0b0, 0b111) => Some(Memop::Lduminh32),
        (0b01, 0b0, 0b0, 0b0, 0b1, 0b000) => Some(Memop::Swph32),
        (0b01, 0b0, 0b0, 0b1, 0b0, 0b000) => Some(Memop::Ldaddlh32),
        (0b01, 0b0, 0b0, 0b1, 0b0, 0b001) => Some(Memop::Ldclrlh32),
        (0b01, 0b0, 0b0, 0b1, 0b0, 0b010) => Some(Memop::Ldeorlh32),
        (0b01, 0b0, 0b0, 0b1, 0b0, 0b011) => Some(Memop::Ldsetlh32),
        (0b01, 0b0, 0b0, 0b1, 0b0, 0b100) => Some(Memop::Ldsmaxlh32),
        (0b01, 0b0, 0b0, 0b1, 0b0, 0b101) => Some(Memop::Ldsminlh32),
        (0b01, 0b0, 0b0, 0b1, 0b0, 0b110) => Some(Memop::Ldumaxlh32),
        (0b01, 0b0, 0b0, 0b1, 0b0, 0b111) => Some(Memop::Lduminlh32),
        (0b01, 0b0, 0b0, 0b1, 0b1, 0b000) => Some(Memop::Swplh32),
        (0b01, 0b0, 0b1, 0b0, 0b0, 0b000) => Some(Memop::Ldaddah32),
        (0b01, 0b0, 0b1, 0b0, 0b0, 0b001) => Some(Memop::Ldclrah32),
        (0b01, 0b0, 0b1, 0b0, 0b0, 0b010) => Some(Memop::Ldeorah32),
        (0b01, 0b0, 0b1, 0b0, 0b0, 0b011) => Some(Memop::Ldsetah32),
        (0b01, 0b0, 0b1, 0b0, 0b0, 0b100) => Some(Memop::Ldsmaxah32),
        (0b01, 0b0, 0b1, 0b0, 0b0, 0b101) => Some(Memop::Ldsminah32),
        (0b01, 0b0, 0b1, 0b0, 0b0, 0b110) => Some(Memop::Ldumaxah32),
        (0b01, 0b0, 0b1, 0b0, 0b0, 0b111) => Some(Memop::Lduminah32),
        (0b01, 0b0, 0b1, 0b0, 0b1, 0b000) => Some(Memop::Swpah32),
        (0b01, 0b0, 0b1, 0b0, 0b1, 0b100) => match rs {
            0b11111 => Some(Memop::Ldaprh32l),
            _ => None,
        },
        (0b01, 0b0, 0b1, 0b1, 0b0, 0b000) => Some(Memop::Ldaddalh32),
        (0b01, 0b0, 0b1, 0b1, 0b0, 0b001) => Some(Memop::Ldclralh32),
        (0b01, 0b0, 0b1, 0b1, 0b0, 0b010) => Some(Memop::Ldeoralh32),
        (0b01, 0b0, 0b1, 0b1, 0b0, 0b011) => Some(Memop::Ldsetalh32),
        (0b01, 0b0, 0b1, 0b1, 0b0, 0b100) => Some(Memop::Ldsmaxalh32),
        (0b01, 0b0, 0b1, 0b1, 0b0, 0b101) => Some(Memop::Ldsminalh32),
        (0b01, 0b0, 0b1, 0b1, 0b0, 0b110) => Some(Memop::Ldumaxalh32),
        (0b01, 0b0, 0b1, 0b1, 0b0, 0b111) => Some(Memop::Lduminalh32),
        (0b01, 0b0, 0b1, 0b1, 0b1, 0b000) => Some(Memop::Swpalh32),
        (0b10, 0b0, 0b0, 0b0, 0b0, 0b000) => Some(Memop::Ldadd32),
        (0b10, 0b0, 0b0, 0b0, 0b0, 0b001) => Some(Memop::Ldclr32),
        (0b10, 0b0, 0b0, 0b0, 0b0, 0b010) => Some(Memop::Ldeor32),
        (0b10, 0b0, 0b0, 0b0, 0b0, 0b011) => Some(Memop::Ldset32),
        (0b10, 0b0, 0b0, 0b0, 0b0, 0b100) => Some(Memop::Ldsmax32),
        (0b10, 0b0, 0b0, 0b0, 0b0, 0b101) => Some(Memop::Ldsmin32),
        (0b10, 0b0, 0b0, 0b0, 0b0, 0b110) => Some(Memop::Ldumax32),
        (0b10, 0b0, 0b0, 0b0, 0b0, 0b111) => Some(Memop::Ldumin32),
        (0b10, 0b0, 0b0, 0b0, 0b1, 0b000) => Some(Memop::Swp32),
        (0b10, 0b0, 0b0, 0b1, 0b0, 0b000) => Some(Memop::Ldaddl32),
        (0b10, 0b0, 0b0, 0b1, 0b0, 0b001) => Some(Memop::Ldclrl32),
        (0b10, 0b0, 0b0, 0b1, 0b0, 0b010) => Some(Memop::Ldeorl32),
        (0b10, 0b0, 0b0, 0b1, 0b0, 0b011) => Some(Memop::Ldsetl32),
        (0b10, 0b0, 0b0, 0b1, 0b0, 0b100) => Some(Memop::Ldsmaxl32),
        (0b10, 0b0, 0b0, 0b1, 0b0, 0b101) => Some(Memop::Ldsminl32),
        (0b10, 0b0, 0b0, 0b1, 0b0, 0b110) => Some(Memop::Ldumaxl32),
        (0b10, 0b0, 0b0, 0b1, 0b0, 0b111) => Some(Memop::Lduminl32),
        (0b10, 0b0, 0b0, 0b1, 0b1, 0b000) => Some(Memop::Swpl32),
        (0b10, 0b0, 0b1, 0b0, 0b0, 0b000) => Some(Memop::Ldadda32),
        (0b10, 0b0, 0b1, 0b0, 0b0, 0b001) => Some(Memop::Ldclra32),
        (0b10, 0b0, 0b1, 0b0, 0b0, 0b010) => Some(Memop::Ldeora32),
        (0b10, 0b0, 0b1, 0b0, 0b0, 0b011) => Some(Memop::Ldseta32),
        (0b10, 0b0, 0b1, 0b0, 0b0, 0b100) => Some(Memop::Ldsmaxa32),
        (0b10, 0b0, 0b1, 0b0, 0b0, 0b101) => Some(Memop::Ldsmina32),
        (0b10, 0b0, 0b1, 0b0, 0b0, 0b110) => Some(Memop::Ldumaxa32),
        (0b10, 0b0, 0b1, 0b0, 0b0, 0b111) => Some(Memop::Ldumina32),
        (0b10, 0b0, 0b1, 0b0, 0b1, 0b000) => Some(Memop::Swpa32),
        (0b10, 0b0, 0b1, 0b0, 0b1, 0b100) => match rs {
            0b11111 => Some(Memop::Ldapr32l),
            _ => None,
        },
        (0b10, 0b0, 0b1, 0b1, 0b0, 0b000) => Some(Memop::Ldaddal32),
        (0b10, 0b0, 0b1, 0b1, 0b0, 0b001) => Some(Memop::Ldclral32),
        (0b10, 0b0, 0b1, 0b1, 0b0, 0b010) => Some(Memop::Ldeoral32),
        (0b10, 0b0, 0b1, 0b1, 0b0, 0b011) => Some(Memop::Ldsetal32),
        (0b10, 0b0, 0b1, 0b1, 0b0, 0b100) => Some(Memop::Ldsmaxal32),
        (0b10, 0b0, 0b1, 0b1, 0b0, 0b101) => Some(Memop::Ldsminal32),
        (0b10, 0b0, 0b1, 0b1, 0b0, 0b110) => Some(Memop::Ldumaxal32),
        (0b10, 0b0, 0b1, 0b1, 0b0, 0b111) => Some(Memop::Lduminal32),
        (0b10, 0b0, 0b1, 0b1, 0b1, 0b000) => Some(Memop::Swpal32),
        (0b11, 0b0, 0b0, 0b0, 0b0, 0b000) => Some(Memop::Ldadd64),
        (0b11, 0b0, 0b0, 0b0, 0b0, 0b001) => Some(Memop::Ldclr64),
        (0b11, 0b0, 0b0, 0b0, 0b0, 0b010) => Some(Memop::Ldeor64),
        (0b11, 0b0, 0b0, 0b0, 0b0, 0b011) => Some(Memop::Ldset64),
        (0b11, 0b0, 0b0, 0b0, 0b0, 0b100) => Some(Memop::Ldsmax64),
        (0b11, 0b0, 0b0, 0b0, 0b0, 0b101) => Some(Memop::Ldsmin64),
        (0b11, 0b0, 0b0, 0b0, 0b0, 0b110) => Some(Memop::Ldumax64),
        (0b11, 0b0, 0b0, 0b0, 0b0, 0b111) => Some(Memop::Ldumin64),
        (0b11, 0b0, 0b0, 0b0, 0b1, 0b000) => Some(Memop::Swp64),
        (0b11, 0b0, 0b0, 0b1, 0b0, 0b000) => Some(Memop::Ldaddl64),
        (0b11, 0b0, 0b0, 0b1, 0b0, 0b001) => Some(Memop::Ldclrl64),
        (0b11, 0b0, 0b0, 0b1, 0b0, 0b010) => Some(Memop::Ldeorl64),
        (0b11, 0b0, 0b0, 0b1, 0b0, 0b011) => Some(Memop::Ldsetl64),
        (0b11, 0b0, 0b0, 0b1, 0b0, 0b100) => Some(Memop::Ldsmaxl64),
        (0b11, 0b0, 0b0, 0b1, 0b0, 0b101) => Some(Memop::Ldsminl64),
        (0b11, 0b0, 0b0, 0b1, 0b0, 0b110) => Some(Memop::Ldumaxl64),
        (0b11, 0b0, 0b0, 0b1, 0b0, 0b111) => Some(Memop::Lduminl64),
        (0b11, 0b0, 0b0, 0b1, 0b1, 0b000) => Some(Memop::Swpl64),
        (0b11, 0b0, 0b1, 0b0, 0b0, 0b000) => Some(Memop::Ldadda64),
        (0b11, 0b0, 0b1, 0b0, 0b0, 0b001) => Some(Memop::Ldclra64),
        (0b11, 0b0, 0b1, 0b0, 0b0, 0b010) => Some(Memop::Ldeora64),
        (0b11, 0b0, 0b1, 0b0, 0b0, 0b011) => Some(Memop::Ldseta64),
        (0b11, 0b0, 0b1, 0b0, 0b0, 0b100) => Some(Memop::Ldsmaxa64),
        (0b11, 0b0, 0b1, 0b0, 0b0, 0b101) => Some(Memop::Ldsmina64),
        (0b11, 0b0, 0b1, 0b0, 0b0, 0b110) => Some(Memop::Ldumaxa64),
        (0b11, 0b0, 0b1, 0b0, 0b0, 0b111) => Some(Memop::Ldumina64),
        (0b11, 0b0, 0b1, 0b0, 0b1, 0b000) => Some(Memop::Swpa64),
        (0b11, 0b0, 0b1, 0b0, 0b1, 0b100) => match rs {
            0b11111 => Some(Memop::Ldapr64l),
            _ => None,
        },
        (0b11, 0b0, 0b1, 0b1, 0b0, 0b000) => Some(Memop::Ldaddal64),
        (0b11, 0b0, 0b1, 0b1, 0b0, 0b001) => Some(Memop::Ldclral64),
        (0b11, 0b0, 0b1, 0b1, 0b0, 0b010) => Some(Memop::Ldeoral64),
        (0b11, 0b0, 0b1, 0b1, 0b0, 0b011) => Some(Memop::Ldsetal64),
        (0b11, 0b0, 0b1, 0b1, 0b0, 0b100) => Some(Memop::Ldsmaxal64),
        (0b11, 0b0, 0b1, 0b1, 0b0, 0b101) => Some(Memop::Ldsminal64),
        (0b11, 0b0, 0b1, 0b1, 0b0, 0b110) => Some(Memop::Ldumaxal64),
        (0b11, 0b0, 0b1, 0b1, 0b0, 0b111) => Some(Memop::Lduminal64),
        (0b11, 0b0, 0b1, 0b1, 0b1, 0b000) => Some(Memop::Swpal64),
        _ => None,
    }
}

pub(super) fn comswap(word: u32) -> Option<Comswap> {
    let size = bits(word, 31, 30);
    let a = bits(word, 22, 22);
    let r = bits(word, 15, 15);
    let rt2 = bits(word, 14, 10);
    match (size, a, r, rt2) {
        (0b00, 0b0, 0b0, 0b11111) => Some(Comswap::CasbC32),
        (0b00, 0b0, 0b1, 0b11111) => Some(Comswap::CaslbC32),
        (0b00, 0b1, 0b0, 0b11111) => Some(Comswap::CasabC32),
        (0b00, 0b1, 0b1, 0b11111) => Some(Comswap::CasalbC32),
        (0b01, 0b0, 0b0, 0b11111) => Some(Comswap::CashC32),
        (0b01, 0b0, 0b1, 0b11111) => Some(Comswap::CaslhC32),
        (0b01, 0b1, 0b0, 0b11111) => Some(Comswap::CasahC32),
        (0b01, 0b1, 0b1, 0b11111) => Some(Comswap::CasalhC32),
        (0b10, 0b0, 0b0, 0b11111) => Some(Comswap::CasC32),
        (0b10, 0b0, 0b1, 0b11111) => Some(Comswap::CaslC32),
        (0b10, 0b1, 0b0, 0b11111) => Some(Comswap::CasaC32),
        (0b10, 0b1, 0b1, 0b11111) => Some(Comswap::CasalC32),
        (0b11, 0b0, 0b0, 0b11111) => Some(Comswap::CasC64),
        (0b11, 0b0, 0b1, 0b11111) => Some(Comswap::CaslC64),
        (0b11, 0b1, 0b0, 0b11111) => Some(Comswap::CasaC64),
        (0b11, 0b1, 0b1, 0b11111) => Some(Comswap::CasalC64),
        _ => None,
    }
}

pub(super) fn comswappr(word: u32) -> Option<Comswappr> {
    let sz = bits(word, 30, 30);
    let a = bits(word, 22, 22);
    let r = bits(word, 15, 15);
    let rt2 = bits(word, 14, 10);
    match (sz, a, r, rt2) {
        (0b0, 0b0, 0b0, 0b11111) => Some(Comswappr::CaspCp32),
        (0b0, 0b0, 0b1, 0b11111) => Some(Comswappr::CasplCp32),
        (0b0, 0b1, 0b0, 0b11111) => Some(Comswappr::CaspaCp32),
        (0b0, 0b1, 0b1, 0b11111) => Some(Comswappr::CaspalCp32),
        (0b1, 0b0, 0b0, 0b11111) => Some(Comswappr::CaspCp64),
        (0b1, 0b0, 0b1, 0b11111) => Some(Comswappr::CasplCp64),
        (0b1, 0b1, 0b0, 0b11111) => Some(Comswappr::CaspaCp64),
        (0b1, 0b1, 0b1, 0b11111) => Some(Comswappr::CaspalCp64),
        _ => None,
    }
}

pub(super) fn ldapstl_unscaled(word: u32) -> Option<LdapstlUnscaled> {
    let size = bits(word, 31, 30);
    let opc = bits(word, 23, 22);
    match (size, opc) {
        (0b00, 0b00) => Some(LdapstlUnscaled::Stlurb32),
        (0b00, 0b01) => Some(LdapstlUnscaled::Ldapurb32),
        (0b00, 0b10) => Some(LdapstlUnscaled::Ldapursb64),
        (0b00, 0b11) => Some(LdapstlUnscaled::Ldapursb32),
        (0b01, 0b00) => Some(LdapstlUnscaled::Stlurh32),
        (0b01, 0b01) => Some(LdapstlUnscaled::Ldapurh32),
        (0b01, 0b10) => Some(LdapstlUnscaled::Ldapursh64),
        (0b01, 0b11) => Some(LdapstlUnscaled::Ldapursh32),
        (0b10, 0b00) => Some(LdapstlUnscaled::Stlur32),
        (0b10, 0b01) => Some(LdapstlUnscaled::Ldapur32),
        (0b10, 0b10) => Some(LdapstlUnscaled::Ldapursw64),
        (0b11, 0b00) => Some(LdapstlUnscaled::Stlur64),
        (0b11, 0b01) => Some(LdapstlUnscaled::Ldapur64),
        _ => None,
    }
}

impl Memop {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Ldaddb32
            | Self::Ldclrb32
            | Self::Ldeorb32
            | Self::Ldsetb32
            | Self::Ldsmaxb32
            | Self::Ldsminb32
            | Self::Ldumaxb32
            | Self::Lduminb32
            | Self::Swpb32
            | Self::Ldaddlb32
            | Self::Ldclrlb32
            | Self::Ldeorlb32
            | Self::Ldsetlb32
            | Self::Ldsmaxlb32
            | Self::Ldsminlb32
            | Self::Ldumaxlb32
            | Self::Lduminlb32
            | Self::Swplb32
            | Self::Ldaddab32
            | Self::Ldclrab32
            | Self::Ldeorab32
            | Self::Ldsetab32
            | Self::Ldsmaxab32
            | Self::Ldsminab32
            | Self::Ldumaxab32
            | Self::Lduminab32
            | Self::Swpab32
            | Self::Ldaddalb32
            | Self::Ldclralb32
            | Self::Ldeoralb32
            | Self::Ldsetalb32
            | Self::Ldsmaxalb32
            | Self::Ldsminalb32
            | Self::Ldumaxalb32
            | Self::Lduminalb32
            | Self::Swpalb32
            | Self::Ldaddh32
            | Self::Ldclrh32
            | Self::Ldeorh32
            | Self::Ldseth32
            | Self::Ldsmaxh32
            | Self::Ldsminh32
            | Self::Ldumaxh32
            | Self::Lduminh32
            | Self::Swph32
            | Self::Ldaddlh32
            | Self::Ldclrlh32
            | Self::Ldeorlh32
            | Self::Ldsetlh32
            | Self::Ldsmaxlh32
            | Self::Ldsminlh32
            | Self::Ldumaxlh32
            | Self::Lduminlh32
            | Self::Swplh32
            | Self::Ldaddah32
            | Self::Ldclrah32
            | Self::Ldeorah32
            | Self::Ldsetah32
            | Self::Ldsmaxah32
            | Self::Ldsminah32
            | Self::Ldumaxah32
            | Self::Lduminah32
            | Self::Swpah32
            | Self::Ldaddalh32
            | Self::Ldclralh32
            | Self::Ldeoralh32
            | Self::Ldsetalh32
            | Self::Ldsmaxalh32
            | Self::Ldsminalh32
            | Self::Ldumaxalh32
            | Self::Lduminalh32
            | Self::Swpalh32
            | Self::Ldadd32
            | Self::Ldclr32
            | Self::Ldeor32
            | Self::Ldset32
            | Self::Ldsmax32
            | Self::Ldsmin32
            | Self::Ldumax32
            | Self::Ldumin32
            | Self::Swp32
            | Self::Ldaddl32
            | Self::Ldclrl32
            | Self::Ldeorl32
            | Self::Ldsetl32
            | Self::Ldsmaxl32
            | Self::Ldsminl32
            | Self::Ldumaxl32
            | Self::Lduminl32
            | Self::Swpl32
            | Self::Ldadda32
            | Self::Ldclra32
            | Self::Ldeora32
            | Self::Ldseta32
            | Self::Ldsmaxa32
            | Self::Ldsmina32
            | Self::Ldumaxa32
            | Self::Ldumina32
            | Self::Swpa32
            | Self::Ldaddal32
            | Self::Ldclral32
            | Self::Ldeoral32
            | Self::Ldsetal32
            | Self::Ldsmaxal32
            | Self::Ldsminal32
            | Self::Ldumaxal32
            | Self::Lduminal32
            | Self::Swpal32
            | Self::Ldadd64
            | Self::Ldclr64
            | Self::Ldeor64
            | Self::Ldset64
            | Self::Ldsmax64
            | Self::Ldsmin64
            | Self::Ldumax64
            | Self::Ldumin64
            | Self::Swp64
            | Self::Ldaddl64
            | Self::Ldclrl64
            | Self::Ldeorl64
            | Self::Ldsetl64
            | Self::Ldsmaxl64
            | Self::Ldsminl64
            | Self::Ldumaxl64
            | Self::Lduminl64
            | Self::Swpl64
            | Self::Ldadda64
            | Self::Ldclra64
            | Self::Ldeora64
            | Self::Ldseta64
            | Self::Ldsmaxa64
            | Self::Ldsmina64
            | Self::Ldumaxa64
            | Self::Ldumina64
            | Self::Swpa64
            | Self::Ldaddal64
            | Self::Ldclral64
            | Self::Ldeoral64
            | Self::Ldsetal64
            | Self::Ldsmaxal64
            | Self::Ldsminal64
            | Self::Ldumaxal64
            | Self::Lduminal64
            | Self::Swpal64 => Feature::Lse,
            Self::Ldaprb32l | Self::Ldaprh32l | Self::Ldapr32l | Self::Ldapr64l => Feature::Lrcpc,
        }
    }
}

impl Comswap {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::CasbC32
            | Self::CaslbC32
            | Self::CasabC32
            | Self::CasalbC32
            | Self::CashC32
            | Self::CaslhC32
            | Self::CasahC32
            | Self::CasalhC32
            | Self::CasC32
            | Self::CaslC32
            | Self::CasaC32
            | Self::CasalC32
            | Self::CasC64
            | Self::CaslC64
            | Self::CasaC64
            | Self::CasalC64 => Feature::Lse,
        }
    }
}

impl Comswappr {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::CaspCp32
            | Self::CasplCp32
            | Self::CaspaCp32
            | Self::CaspalCp32
            | Self::CaspCp64
            | Self::CasplCp64
            | Self::CaspaCp64
            | Self::CaspalCp64 => Feature::Lse,
        }
    }
}

impl LdapstlUnscaled {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Stlurb32
            | Self::Ldapurb32
            | Self::Ldapursb64
            | Self::Ldapursb32
            | Self::Stlurh32
            | Self::Ldapurh32
            | Self::Ldapursh64
            | Self::Ldapursh32
            | Self::Stlur32
            | Self::Ldapur32
            | Self::Ldapursw64
            | Self::Stlur64
            | Self::Ldapur64 => Feature::Lrcpc2,
        }
    }
}
