use crate::memory::{BxMemoryStubC, CpuTlbPin};

#[test]
fn stub_keeps_guest_ram_separate_from_rom_storage() {
    let mut mem_stub =
        BxMemoryStubC::create_and_init(32 * 1024 * 1024, 32 * 1024 * 1024, 128 * 1024)
            .unwrap();

    {
        let backing = mem_stub.actual_vector_mut();
        backing[..4].copy_from_slice(b"abcd");
    }
    {
        let guest = mem_stub
            .get_vector_offset(0, &[] as &[CpuTlbPin])
            .unwrap();
        guest[3] = b's';
        assert_eq!(&guest[..4], b"abcs");
    }
    assert_eq!(mem_stub.rom()[0], 0xff);
}

#[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
#[test]
fn guest_block_zero_starts_at_the_internal_guest_base() {
    const THIRTYTWO_MEGABYTES: usize = 32 * 1024 * 1024;
    let mut mem_stub =
        BxMemoryStubC::create_and_init(THIRTYTWO_MEGABYTES, THIRTYTWO_MEGABYTES, 128 * 1024)
            .unwrap();

    let backing_ptr = mem_stub.actual_vector_slice().as_ptr();
    let guest_ptr = mem_stub
        .get_vector_offset(0, &[] as &[CpuTlbPin])
        .unwrap()
        .as_ptr();
    let rom_ptr = mem_stub.rom().as_ptr();

    assert_eq!(guest_ptr, backing_ptr);
    assert!(rom_ptr > guest_ptr);
}
