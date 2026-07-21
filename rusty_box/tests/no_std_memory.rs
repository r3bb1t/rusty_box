#![cfg(not(feature = "alloc"))]

use core::ptr::NonNull;

use rusty_box::{
    memory::{BxMemoryStubC, MemoryError},
    Error,
};

#[test]
fn undersized_raw_host_is_rejected() {
    let result = unsafe {
        BxMemoryStubC::create_from_raw(
            NonNull::<u8>::dangling().as_ptr(),
            0,
            2 * 1024 * 1024,
            1024 * 1024,
            128 * 1024,
        )
    };
    assert!(matches!(result, Err(Error::Memory(MemoryError::InsufficientRam))));
}

#[repr(align(4096))]
struct AlignedBacking([u8; 2 * 1024 * 1024]);

#[test]
fn unaligned_raw_host_is_rejected() {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            let mut backing = AlignedBacking([0; 2 * 1024 * 1024]);
            let result = unsafe {
                BxMemoryStubC::create_from_raw(
                    backing.0.as_mut_ptr().add(1),
                    backing.0.len() - 1,
                    1024 * 1024,
                    1024 * 1024,
                    128 * 1024,
                )
            };
            assert!(matches!(result, Err(Error::Memory(MemoryError::Internal(_)))));
        })
        .unwrap()
        .join()
        .unwrap();
}
