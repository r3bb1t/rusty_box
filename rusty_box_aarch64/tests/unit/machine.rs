//! The run loop's sleep: a processor waiting for an interrupt always moves
//! time on, so a run ends even when a device reports a deadline it has
//! already reached and never clears.

use super::wake_tick;

#[test]
fn a_waiting_processor_sleeps_to_the_next_event_or_the_slice_end() {
    assert_eq!(wake_tick(150, 100, 1000), 150, "the next event");
    assert_eq!(wake_tick(5000, 100, 1000), 1000, "the slice ends first");
    assert_eq!(wake_tick(u64::MAX, 100, 1000), 1000);
}

#[test]
fn an_event_already_due_wakes_it_at_the_next_tick() {
    // A deadline reached and not cleared: time moves one tick at a time,
    // so the slice reaches its end and no later event is passed.
    assert_eq!(wake_tick(100, 100, 1000), 101);
    assert_eq!(wake_tick(40, 100, 1000), 101);
    let mut now = 100;
    let mut sleeps = 0;
    while now < 1000 && sleeps < 2000 {
        now = wake_tick(40, now, 1000);
        sleeps += 1;
    }
    assert_eq!(
        (now, sleeps),
        (1000, 900),
        "the slice ends, a tick at a time"
    );
    // At the slice's end, the slice's end.
    assert_eq!(wake_tick(40, 1000, 1000), 1000);
    assert_eq!(wake_tick(40, u64::MAX, u64::MAX), u64::MAX);
}
