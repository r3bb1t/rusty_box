#[derive(Debug)]
pub struct BxCr0 {
    pub val32: u32, // 32bit value of register
}

#[derive(Debug)]
pub struct BxCr4 {
    pub val32: u32, // 32bit value of register
}

#[derive(Debug)]
pub struct BxDr6 {
    pub val32: u32, // 32bit value of register
}

#[derive(Debug)]
pub struct BxDr7 {
    pub val32: u32, // 32bit value of register
}

#[derive(Debug)]
pub struct BxEfer {
    pub value: u32,
}

#[derive(Debug)]
pub struct Xcr0 {
    pub value: u32,
}

#[derive(Debug)]
enum Xcr0Enum {
    BxXcr0FpuBit = 0,
    BxXcr0SseBit = 1,
    BxXcr0YmmBit = 2,
    BxXcr0BndregsBit = 3, // not implemented, deprecated
    BxXcr0BndcfgBit = 4,  // not implemented, deprecated
    BxXcr0OpmaskBit = 5,
    BxXcr0ZmmHi256Bit = 6,
    BxXcr0HiZmmBit = 7,
    BxXcr0PtBit = 8, // not implemented yet
    BxXcr0PkruBit = 9,
    BxXcr0PasidBit = 10, // not implemented yet
    BxXcr0CetUBit = 11,
    BxXcr0CetSBit = 12,
    BxXcr0HdcBit = 13, // not implemented yet
    BxXcr0UintrBit = 14,
    BxXcr0LbrBit = 15, // not implemented yet
    BxXcr0HwpBit = 16, // not implemented yet
    BxXcr0XtilecfgBit = 17,
    BxXcr0XtiledataBit = 18,
    BxXcr0ApxBit = 19,
    BxXcr0Last, // make sure it is < 32
}

#[derive(Debug)]
pub struct MSR {
    /// MSR index
    index: u32,
    /// MSR type: 1 - lin address, 2 - phy address
    r#type: u32,
    /// current MSR value
    val64: u64,
    /// reset value
    reset_value: u64,
    /// r/o bits - fault on write
    reserved: u64,
    /// hardwired bits - ignored on write
    ignored: u64,
}

impl MSR {
    const BX_LIN_ADDRESS_MSR: u32 = 1;
    const BX_PHY_ADDRESS_MSR: u32 = 2;
}
