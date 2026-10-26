//! Unit tests of `src/devices/virtio/mod.rs` and `queue.rs`: the
//! non-legacy virtio-mmio transport and the split ring, driven through the
//! registers and guest memory as a driver drives them, against what QEMU's
//! `virtio-mmio.c` and `virtio.c` do. The device behind the transport is the
//! keyboard, whose status queue takes any report a driver offers.

use super::input::{Input, InputKind};
use super::test_driver::*;
use super::Device;
use crate::bus::AccessWidth;

fn keyboard() -> Driver {
    Driver::new(Device::Input(Input::new(InputKind::Keyboard)))
}

/// What Linux accepts of what the device offers.
const LINUX_FEATURES: u64 = F_VERSION_1 | F_INDIRECT_DESC | F_EVENT_IDX;

/// Offer one LED report on the keyboard's status queue and notify it.
fn report(d: &mut Driver) {
    let at = d.alloc_with(&[0x11, 0, 0, 0, 1, 0, 0, 0]);
    d.offer(1, &[Buf::readable(at, 8)]);
    d.notify(1);
}

#[test]
fn the_registers_identify_a_non_legacy_qemu_transport() {
    let mut d = keyboard();
    assert_eq!(d.read(MAGIC_VALUE), 0x7472_6976);
    assert_eq!(d.read(VERSION), 2);
    assert_eq!(d.read(DEVICE_ID), 18);
    assert_eq!(d.read(VENDOR_ID), 0x554D_4551);
    // No shared memory regions: their length is all ones, their base 0.
    assert_eq!(d.read(SHM_LEN_LOW), u32::MAX);
    assert_eq!(d.read(SHM_LEN_HIGH), u32::MAX);
    assert_eq!(d.read(SHM_BASE_LOW), 0);
    for q in 0..2 {
        d.write(QUEUE_SEL, q);
        assert_eq!(d.read(QUEUE_NUM_MAX), 64, "queue {q}");
    }
    d.write(QUEUE_SEL, 2);
    assert_eq!(d.read(QUEUE_NUM_MAX), 0, "a queue the device does not have");
}

#[test]
fn write_only_registers_read_as_zero() {
    let mut d = keyboard();
    d.write(QUEUE_SEL, 1);
    d.write(QUEUE_NUM, 32);
    d.write(QUEUE_DESC_LOW, 0x4001_0000);
    d.write(DRIVER_FEATURES, 0xFFFF_FFFF);
    for off in [QUEUE_SEL, QUEUE_NUM, QUEUE_DESC_LOW, DRIVER_FEATURES, DEVICE_FEATURES_SEL, QUEUE_NOTIFY, INTERRUPT_ACK] {
        assert_eq!(d.read(off), 0, "{off:#x}");
    }
}

#[test]
fn registers_take_four_byte_accesses_and_eight_byte_ones_split() {
    let mut d = keyboard();
    assert_eq!(d.read_width(MAGIC_VALUE, AccessWidth::Byte), 0);
    assert_eq!(d.read_width(MAGIC_VALUE, AccessWidth::Half), 0);
    assert_eq!(d.read_width(MAGIC_VALUE, AccessWidth::Dword), 0x7472_6976 | (2 << 32));
    d.write_width(STATUS, AccessWidth::Byte, u64::from(ACKNOWLEDGE));
    assert_eq!(d.read(STATUS), 0, "a byte write to a register is ignored");
    d.write_width(STATUS, AccessWidth::Dword, u64::from(ACKNOWLEDGE) | (0xFFFF_FFFF << 32));
    assert_eq!(d.read(STATUS), ACKNOWLEDGE);
}

#[test]
fn the_device_offers_qemus_common_features() {
    let mut d = keyboard();
    // Indirect descriptors, event index, VERSION_1 and queue reset; QEMU's
    // notify-on-empty and any-layout are legacy, so not shown.
    assert_eq!(d.device_features(), F_INDIRECT_DESC | F_EVENT_IDX | F_VERSION_1 | (1 << 40));
}

#[test]
fn features_ok_stays_set_when_the_driver_accepts_too_much() {
    let mut d = keyboard();
    d.negotiate(u64::MAX);
    assert_eq!(d.read(STATUS), ACKNOWLEDGE | DRIVER | FEATURES_OK);
}

#[test]
fn the_device_publishes_avail_event_only_under_event_idx() {
    let mut d = keyboard();
    d.start(F_VERSION_1);
    report(&mut d);
    assert_eq!(d.used(1), [(0, 8)]);
    assert_eq!(d.avail_event(1), 0);

    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    report(&mut d);
    report(&mut d);
    assert_eq!(d.used(1), [(0, 8), (1, 8)]);
    assert_eq!(d.avail_event(1), 2);
}

#[test]
fn writing_status_zero_resets_the_device() {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    report(&mut d);
    assert_ne!(d.read(INTERRUPT_STATUS), 0);
    assert!(d.irq());
    d.write(STATUS, 0);
    assert_eq!(d.read(STATUS), 0);
    assert_eq!(d.read(INTERRUPT_STATUS), 0);
    assert!(!d.irq());
    for q in 0..2 {
        d.write(QUEUE_SEL, q);
        assert_eq!(d.read(QUEUE_READY), 0, "queue {q}");
    }
}

#[test]
fn queue_ready_is_kept_for_every_queue_index() {
    let mut d = keyboard();
    d.write(QUEUE_SEL, 700);
    d.write(QUEUE_READY, 1);
    assert_eq!(d.read(QUEUE_READY), 1);
    d.write(QUEUE_SEL, 1024);
    assert_eq!(d.read(QUEUE_READY), 1, "a selector past VIRTIO_QUEUE_MAX is ignored");
    d.write(QUEUE_READY, 0);
    assert_eq!(d.read(QUEUE_READY), 0);
}

#[test]
fn rings_take_effect_when_the_driver_sets_queue_ready() {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    d.write(QUEUE_SEL, 1);
    // New addresses, in no memory region, are only latched...
    d.write(QUEUE_DESC_LOW, UNASSIGNED as u32);
    d.write(QUEUE_DESC_HIGH, (UNASSIGNED >> 32) as u32);
    report(&mut d);
    assert_eq!(d.used(1).len(), 1, "the queue still runs on the rings it was enabled with");
    // ...until QueueReady hands them to the device, which cannot map them.
    d.write(QUEUE_READY, 1);
    assert_eq!(d.read(STATUS) & NEEDS_RESET, NEEDS_RESET);
}

#[test]
fn a_driver_reset_keeps_the_latched_rings_and_a_system_reset_forgets_them() {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    d.write(STATUS, 0);
    d.write(STATUS, ACKNOWLEDGE | DRIVER);
    d.accept(LINUX_FEATURES);
    d.write(STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK);
    d.write(QUEUE_SEL, 1);
    d.write(QUEUE_READY, 1);
    d.write(STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK);
    report(&mut d);
    assert_eq!(d.used(1).len(), 1, "the rings latched before the driver's reset");

    d.transport.reset();
    d.write(STATUS, ACKNOWLEDGE | DRIVER);
    d.accept(LINUX_FEATURES);
    d.write(STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK);
    d.write(QUEUE_SEL, 1);
    d.write(QUEUE_READY, 1);
    d.write(STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK);
    report(&mut d);
    assert_eq!(d.used(1).len(), 0, "no rings: the queue has no descriptor table");
    assert_eq!(d.read(STATUS) & NEEDS_RESET, 0);
}

#[test]
fn a_queue_larger_than_the_devices_breaks_it() {
    let mut d = keyboard();
    d.negotiate(F_VERSION_1);
    d.write(QUEUE_SEL, 0);
    d.write(QUEUE_NUM, 65);
    assert_eq!(d.read(STATUS) & NEEDS_RESET, NEEDS_RESET);
    assert!(!d.irq(), "no configuration interrupt before DRIVER_OK");

    let mut d = keyboard();
    d.negotiate(0);
    d.write(QUEUE_SEL, 0);
    d.write(QUEUE_NUM, 65);
    assert_eq!(d.read(STATUS) & NEEDS_RESET, 0, "only a VERSION_1 driver is told");
}

/// Start the keyboard, break it with `bad` (which offers something on the
/// status queue and notifies it), and check the device says it needs a reset
/// and used nothing; then reset it and check it works again.
fn breaks_the_device<F: FnOnce(&mut Driver)>(bad: F) {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    bad(&mut d);
    assert_eq!(d.read(STATUS) & NEEDS_RESET, NEEDS_RESET);
    assert_eq!(d.read(INTERRUPT_STATUS), 3, "a configuration change");
    assert!(d.irq());
    assert!(d.used(1).is_empty());

    d.start(LINUX_FEATURES);
    assert_eq!(d.read(STATUS) & NEEDS_RESET, 0);
    report(&mut d);
    assert_eq!(d.used(1).len(), 1);
}

#[test]
fn a_buffer_of_no_bytes_breaks_the_device() {
    breaks_the_device(|d| {
        let at = d.alloc(8);
        d.offer(1, &[Buf::readable(at, 0)]);
        d.notify(1);
    });
}

/// QEMU's `virtqueue_split_pop` asks only whether the queue is empty, so an
/// available index further ahead than the queue holds is taken as it
/// stands: every head it reaches is popped, here the one chain at ring
/// entry 0 a hundred times.
#[test]
fn an_available_index_past_the_queue_size_is_popped_to_its_end() {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    let at = d.alloc_with(&[0x11, 0, 0, 0, 1, 0, 0, 0]);
    d.offer(1, &[Buf::readable(at, 8)]);
    let ring = d.avail_ring(1);
    d.put_u16(ring + 2, 100);
    d.notify(1);
    assert_eq!(d.read(STATUS) & NEEDS_RESET, 0);
    assert_eq!(d.used(1), std::vec![(0, 8); 100]);
}

#[test]
fn a_buffer_in_no_memory_region_breaks_the_device() {
    breaks_the_device(|d| {
        d.offer(1, &[Buf::readable(UNASSIGNED, 8)]);
        d.notify(1);
    });
}

#[test]
fn a_readable_buffer_after_a_writable_one_breaks_the_device() {
    breaks_the_device(|d| {
        let (a, b) = (d.alloc(8), d.alloc(8));
        d.offer(1, &[Buf::writable(a, 8), Buf::readable(b, 8)]);
        d.notify(1);
    });
}

#[test]
fn a_looped_chain_breaks_the_device() {
    breaks_the_device(|d| {
        let at = d.alloc(8);
        d.put_desc(1, 0, at, 8, DESC_NEXT, 1);
        d.put_desc(1, 1, at, 8, DESC_NEXT, 0);
        d.publish(1, 0);
        d.notify(1);
    });
}

#[test]
fn a_next_past_the_table_breaks_the_device() {
    breaks_the_device(|d| {
        let at = d.alloc(8);
        d.put_desc(1, 0, at, 8, DESC_NEXT, 64);
        d.publish(1, 0);
        d.notify(1);
    });
}

#[test]
fn a_head_past_the_table_breaks_the_device() {
    breaks_the_device(|d| {
        d.publish(1, 64);
        d.notify(1);
    });
}

#[test]
fn an_available_index_moved_past_the_queue_breaks_the_device() {
    breaks_the_device(|d| {
        let avail = d.avail_ring(1);
        d.put_u16(avail + 2, 65);
        d.notify(1);
    });
}

#[test]
fn an_indirect_table_of_no_entries_breaks_the_device() {
    breaks_the_device(|d| {
        let table = d.alloc(16);
        d.put_desc(1, 0, table, 0, DESC_INDIRECT, 0);
        d.publish(1, 0);
        d.notify(1);
    });
}

#[test]
fn an_indirect_table_of_part_of_an_entry_breaks_the_device() {
    breaks_the_device(|d| {
        let table = d.alloc(32);
        d.put_desc(1, 0, table, 24, DESC_INDIRECT, 0);
        d.publish(1, 0);
        d.notify(1);
    });
}

#[test]
fn an_indirect_table_in_no_memory_region_breaks_the_device() {
    breaks_the_device(|d| {
        d.put_desc(1, 0, UNASSIGNED, 32, DESC_INDIRECT, 0);
        d.publish(1, 0);
        d.notify(1);
    });
}

#[test]
fn an_indirect_chain_is_read_across_its_buffers() {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    let (a, b) = (d.alloc_with(&[0x11, 0, 0, 0]), d.alloc_with(&[1, 0, 0, 0]));
    d.offer_indirect(1, &[Buf::readable(a, 4), Buf::readable(b, 4)]);
    d.notify(1);
    assert_eq!(d.used(1), [(0, 8)]);
}

#[test]
fn a_broken_device_keeps_its_interrupt_line_until_reset() {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    d.offer(1, &[Buf::readable(UNASSIGNED, 8)]);
    d.notify(1);
    assert!(d.irq());
    assert_eq!(d.take_interrupt(), 3);
    assert_eq!(d.read(INTERRUPT_STATUS), 0);
    assert!(d.irq(), "QEMU's virtio_notify_vector leaves a broken device's line alone");
    report(&mut d);
    assert!(d.used(1).is_empty(), "a broken device ignores its queues");
    d.write(STATUS, 0);
    assert!(!d.irq());
}

#[test]
fn no_interrupt_suppresses_the_interrupt_without_event_idx() {
    let mut d = keyboard();
    d.start(F_VERSION_1);
    d.set_avail_flags(1, 1);
    report(&mut d);
    assert_eq!(d.used(1).len(), 1);
    assert_eq!(d.read(INTERRUPT_STATUS), 0);
    assert!(!d.irq());
}

#[test]
fn used_event_decides_the_interrupt_under_event_idx() {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    report(&mut d);
    assert_eq!(d.take_interrupt(), 1, "the first use always interrupts");
    d.set_used_event(1, 5);
    report(&mut d);
    assert_eq!(d.read(INTERRUPT_STATUS), 0, "used index 2 has not passed 5");
    d.set_used_event(1, 2);
    report(&mut d);
    assert_eq!(d.take_interrupt(), 1, "used index 3 passed 2");
    assert_eq!(d.used(1).len(), 3);
}

#[test]
fn configuration_past_its_end_reads_all_ones() {
    let mut d = keyboard();
    // The keyboard's configuration is 37 bytes: the header and its 29-byte
    // key bitmap.
    assert_eq!(d.read_width(CONFIG + 37, AccessWidth::Byte), 0xFF);
    assert_eq!(d.read_width(CONFIG + 36, AccessWidth::Half), 0xFFFF);
    assert_eq!(d.read_width(CONFIG + 34, AccessWidth::Word), 0xFFFF_FFFF);
    assert_eq!(d.read_width(CONFIG + 36, AccessWidth::Byte), 0, "the last byte is inside");
}

#[test]
fn a_configuration_write_is_a_change_only_once_the_driver_is_ready() {
    let mut d = keyboard();
    d.negotiate(LINUX_FEATURES);
    d.write_width(CONFIG, AccessWidth::Byte, 1);
    assert_eq!(d.read(CONFIG_GENERATION), 0);
    assert_eq!(d.read(INTERRUPT_STATUS), 0);

    d.start(LINUX_FEATURES);
    d.write_width(CONFIG, AccessWidth::Byte, 1);
    assert_eq!(d.read(CONFIG_GENERATION), 1);
    assert_eq!(d.read(INTERRUPT_STATUS), 3);
    assert!(d.irq());
    d.take_interrupt();
    assert!(!d.irq());
}

#[test]
fn a_snapshot_resumes_where_the_queues_were() {
    let mut d = keyboard();
    d.start(LINUX_FEATURES);
    report(&mut d);
    d.set_used_event(1, 7);
    let saved = d.save();
    let mut resumed = d.restored(Device::Input(Input::new(InputKind::Keyboard)), &saved);
    assert_eq!(resumed.save(), saved);

    // The same next steps on both give the same guest memory and interrupt.
    for m in [&mut d, &mut resumed] {
        m.take_interrupt();
        report(m);
        report(m);
    }
    assert!(d.ram == resumed.ram);
    assert_eq!(resumed.read(INTERRUPT_STATUS), d.read(INTERRUPT_STATUS));
    assert_eq!(resumed.used(1), d.used(1));
}
