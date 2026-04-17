use crate::memory::BxMemoryStubC;

//fn enable_logging() {
//    tracing_subscriber::fmt()
//        .without_time()
//        .with_target(false)
//        //.with_env_filter(EnvFilter::from_default_env())
//        .init();
//}
#[test]
fn stub_creates_successfully() {
    //enable_logging();
    let guest_mb = 32;
    let host_mb = 32;
    let mem_stub =
        BxMemoryStubC::create_and_init(guest_mb * 1024 * 1024, host_mb * 1024 * 1024, 128 * 1024)
            .unwrap();

    let actual_vector = mem_stub.actual_vector();
    actual_vector[0] = b'a';
    actual_vector[1] = b'b';
    actual_vector[2] = b'c';
    actual_vector[3] = b'd';
    let vector = mem_stub.vector();
    vector[3] = b's';
    let rom = mem_stub.rom();

    tracing::debug!(
        "Pointers: \n actual vector: {:#?} ,\n vector: {:#?} \n rom: {:p}",
        &actual_vector[0..10],
        &vector[0..10],
        rom
    );
}

#[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
#[test]
fn init_memory_with_address_assertions() {
    //enable_logging();
    const THIRTYTWO_MEGABYTES_IN_BITS: u64 = 32 * 1024 * 32;
    let guest = THIRTYTWO_MEGABYTES_IN_BITS;
    let host = THIRTYTWO_MEGABYTES_IN_BITS;
    let block_size = 128 * 1024;

    let mem_stub = BxMemoryStubC::create_and_init(
        guest.try_into().unwrap(),
        host.try_into().unwrap(),
        block_size,
    )
    .unwrap();

    let actual_vector = mem_stub.actual_vector();
    let vector = mem_stub.vector();
    let rom = mem_stub.rom();

    let actual_vector_ptr = actual_vector.as_ptr();
    let vector_ptr = vector.as_ptr();
    let rom_ptr = rom.as_ptr();

    // allocated memory at 0x72c626200010. after alignment, vector=0x72c626201000, block_size = 128K
    tracing::debug!(
        "Pointers: \n actual vector: {:p} ,\n vector: {:p} \n rom: {:p}",
        actual_vector_ptr,
        vector_ptr,
        rom_ptr
    );
    // assert_eq!(vector_ptr - actual_vector_ptr, 0xff0);
    assert_eq!(unsafe { vector_ptr.offset_from(actual_vector_ptr) }, 0xff0);
}
