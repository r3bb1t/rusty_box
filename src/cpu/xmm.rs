pub type BxPackedAvxRegister = BxPackedZmmRegister;

pub type BxZmmReg = BxPackedZmmRegister;
#[derive(Debug)]
pub enum BxPackedZmmRegister {
    ZmmSbyte([i8; 64]),
    ZmmS16([i16; 32]),
    ZmmS32([i32; 16]),
    ZmmS64([i64; 8]),
    ZmmUbyte([u8; 64]),
    ZmmU16([u16; 32]),
    ZmmU32([u32; 16]),
    ZmmU64([u64; 8]),
    ZmmV128([BxPackedXmmRegister; 4]),
    ZmmV256([BxPackedYmmRegister; 2]),
}

pub type BxXmmReg = BxPackedXmmRegister;
#[derive(Debug)]
pub enum BxPackedXmmRegister {
    XmmSbyte([i8; 16]),
    XmmS16([i16; 8]),
    XmmS32([i32; 4]),
    XmmS64([i64; 2]),
    XmmUbyte([u8; 16]),
    XmmU16([i16; 8]),
    XmmU32([i32; 4]),
    XmmU64([u64; 2]),
}

pub type BxYmmReg = BxPackedYmmRegister;
#[derive(Debug)]
pub enum BxPackedYmmRegister {
    YmmSbyte([i8; 32]),
    YmmS16([i16; 16]),
    YmmS32([i32; 8]),
    YmmS64([i64; 4]),
    YmmUbyte([u8; 32]),
    YmmU16([u16; 16]),
    YmmU32([u32; 8]),
    YmmU64([u64; 4]),
    YmmV128([BxPackedXmmRegister; 2]),
}

#[derive(Debug)]
pub struct BxMxcsr {
    mxcsr: u32,
}
