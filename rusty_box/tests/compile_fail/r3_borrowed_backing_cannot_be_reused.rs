// R1/R3: handing storage to memory is an exclusive loan for its lifetime.
// A downstream host cannot mutate cached code behind the memory's SMC guard.
fn main() {
    let bytes = Box::leak(Box::new([0u8; 4096]));
    let _memory = rusty_box::memory::BxMemoryStubC::from_buffer(bytes, 4096, 4096, 4096);
    bytes[0] = 0xF4;
}
