use rusty_box::config::BxPhyAddress;
use rusty_box::memory::flat::FlatMemory;
fn main() {
    let mut bytes = [0u8; 4096];
    let mut stamps = [0; 1];
    let mut memory = FlatMemory::new(&mut bytes, &mut stamps).unwrap();
    bytes[0] = 0x90;
    memory.write(BxPhyAddress::new(0), &[0xF4]).unwrap();
}
