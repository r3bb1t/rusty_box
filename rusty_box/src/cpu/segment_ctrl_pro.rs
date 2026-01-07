use super::descriptor::BxSelector;

pub fn parse_selector(raw_selector: u16, selector: &mut BxSelector) {
    selector.value = raw_selector;
    selector.index = raw_selector >> 3;
    selector.ti = (raw_selector >> 2) & 0x01;
    // Note: bochs uses implicit cast
    selector.rpl = raw_selector as u8 & 0x03;
}
