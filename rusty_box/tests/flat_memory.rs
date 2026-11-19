use rusty_box::config::BxPhyAddress;
use rusty_box::memory::flat::{FlatMemory, FlatMemoryError};
#[test]
fn borrowed_dense_memory_checks_storage_and_writes_atomically() {
    let mut bytes = [0u8; 4100];
    let mut stamps = [0; 2];
    let mut ram = FlatMemory::new(&mut bytes[1..], &mut stamps).unwrap();
    assert_eq!(ram.len(), 4099);
    ram.write(BxPhyAddress::new(4096), &[1, 2, 3]).unwrap();
    assert!(ram.write(BxPhyAddress::new(4098), &[9, 9]).is_err());
    let mut out = [0; 3];
    ram.read(BxPhyAddress::new(4096), &mut out).unwrap();
    assert_eq!(out, [1, 2, 3]);
    assert!(ram.read(BxPhyAddress::new(u64::MAX), &mut out).is_err());
    assert!(matches!(
        FlatMemory::new(&mut bytes, &mut stamps[..1]),
        Err(FlatMemoryError::StampStorage {
            required: 2,
            provided: 1
        })
    ));
    assert_eq!(
        FlatMemory::required_stamps(usize::MAX),
        usize::MAX / 4096 + 1
    );
}
