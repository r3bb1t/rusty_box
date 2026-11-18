//! A downstream caller can supply storage without forging a pointer. This
//! exercises the existing backing contract, not a generic CPU memory bus.
use rusty_box::config::BxPhyAddress;
use rusty_box::memory::{BxMemC, BxMemoryStubC};

#[repr(align(4096))]
struct Storage([u8; 6 * 1024 * 1024]);

#[test]
fn downstream_owned_storage_constructs_real_memory_safely() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let storage = Box::leak(Box::new(Storage([0; 6 * 1024 * 1024])));
            let stub = BxMemoryStubC::from_buffer(&mut storage.0, 1 << 20, 1 << 20, 4096).unwrap();
            assert_eq!(stub.get_memory_len(), 1 << 20);
            let mut memory = BxMemC::new_from_stub(stub, false);
            memory.set_a20_mask(rusty_box::config::A20Mask::OPEN);
            memory
                .load_RAM(&[0xB8, 42, 0, 0, 0, 0xF4], BxPhyAddress::new(0x2000))
                .unwrap();
            assert!(
                memory
                    .load_RAM(&[0; 2], BxPhyAddress::new((1 << 20) - 1))
                    .is_err()
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn downstream_unaligned_or_short_storage_is_refused() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let storage = Box::leak(Box::new(Storage([0; 6 * 1024 * 1024])));
            assert!(
                BxMemoryStubC::from_buffer(&mut storage.0[1..], 1 << 20, 1 << 20, 4096).is_err()
            );
            let storage = Box::leak(Box::new(Storage([0; 6 * 1024 * 1024])));
            assert!(
                BxMemoryStubC::from_buffer(&mut storage.0[..4096], 1 << 20, 1 << 20, 4096).is_err()
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
