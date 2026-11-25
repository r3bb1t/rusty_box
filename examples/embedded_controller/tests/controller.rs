use rusty_box_aarch64::{ClockHz, Reg, RunBudget, StopReason};
use rusty_box_embedded_controller::{RAM_LEN, controller, firmware, set_inputs};

#[test]
fn a_timed_sensor_irq_wakes_the_guest_and_drives_gpio() {
    let rom = firmware();
    let mut ram = [0; RAM_LEN];
    let mut machine = controller(&mut ram, &rom, ClockHz::new(1_000_000).unwrap()).unwrap();
    assert_eq!(
        machine.step(RunBudget::Ticks(32)).unwrap().stop,
        StopReason::Halted
    );
    set_inputs(&mut machine, 90, 5);
    let early = machine.step(RunBudget::Ticks(3)).unwrap();
    assert_eq!(early.progress.instructions(), Some(0));
    assert_eq!(machine.board().devices().second().device().output(), 0);

    let outcome = machine.step(RunBudget::Ticks(32)).unwrap();
    assert_eq!(outcome.stop, StopReason::Halted);
    assert_eq!(machine.reg_read(Reg::X1), 1, "guest read asserted alarm");
    assert_eq!(machine.board().devices().second().device().output(), 95);
    let idle = machine.step(RunBudget::Ticks(8)).unwrap();
    assert_eq!(
        idle.progress.instructions(),
        Some(0),
        "guest acknowledged the IRQ"
    );

    let now = machine.clock().now();
    machine.reset();
    assert_eq!(machine.clock().now(), now);
    assert_eq!(machine.board().devices().second().device().output(), 0);
    assert_eq!(
        machine.step(RunBudget::Ticks(32)).unwrap().stop,
        StopReason::Halted
    );
}
