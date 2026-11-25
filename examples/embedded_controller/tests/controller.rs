use rusty_box_aarch64::machine::link::{ConnectedPair, PairError, PairStop};
use rusty_box_aarch64::{ClockHz, Reg, RunBudget, StopReason};
use rusty_box_core::VmDuration;
use rusty_box_embedded_controller::{
    RAM_LEN, controller, digital_link, firmware, output, set_inputs,
};

const QUANTUM: VmDuration = VmDuration::from_ticks(32);

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
    assert_eq!(output(&machine), 0);

    let outcome = machine.step(RunBudget::Ticks(32)).unwrap();
    assert_eq!(outcome.stop, StopReason::Halted);
    assert_eq!(machine.reg_read(Reg::X1), 1, "guest read asserted alarm");
    assert_eq!(output(&machine), 95);
    let idle = machine.step(RunBudget::Ticks(8)).unwrap();
    assert_eq!(
        idle.progress.instructions(),
        Some(0),
        "guest acknowledged the IRQ"
    );

    let now = machine.clock().now();
    machine.reset();
    assert_eq!(machine.clock().now(), now);
    assert_eq!(output(&machine), 0);
    assert_eq!(
        machine.step(RunBudget::Ticks(32)).unwrap().stop,
        StopReason::Halted
    );
}

#[test]
fn each_guest_sees_local_output_before_equal_time_frames_and_runs_repeatably() {
    let mut observed = None;
    for _ in 0..2 {
        let rom = firmware();
        let mut a_ram = [0; RAM_LEN];
        let mut b_ram = [0; RAM_LEN];
        a_ram[0] = 11;
        b_ram[0] = 22;
        let rate = ClockHz::BOCHS_DEFAULT;
        let a = controller(&mut a_ram, &rom, rate).unwrap();
        let b = controller(&mut b_ram, &rom, rate).unwrap();
        let mut pair = ConnectedPair::new(a, b, digital_link())
            .unwrap()
            .with_return_link(digital_link());
        assert_eq!(pair.run(1, QUANTUM).unwrap().frames, 0);
        set_inputs(pair.first_mut(), 90, 5);
        set_inputs(pair.second_mut(), 100, 2);

        let boundary = pair.run(1, QUANTUM).unwrap();
        assert_eq!(boundary.stop, PairStop::Completed);
        assert_eq!(boundary.frames, 2);
        assert_eq!(output(pair.first()), 95);
        assert_eq!(output(pair.second()), 102);
        assert_eq!(pair.first().clock().now(), pair.second().clock().now());

        assert_eq!(pair.run(1, QUANTUM).unwrap().frames, 0);
        assert_eq!(output(pair.first()), 102);
        assert_eq!(output(pair.second()), 95);
        assert_eq!(pair.first().board().ram()[0], 11);
        assert_eq!(pair.second().board().ram()[0], 22);
        let state = (
            pair.first().instructions(),
            pair.second().instructions(),
            pair.first().clock().now(),
        );
        if let Some(previous) = observed {
            assert_eq!(state, previous);
        }
        observed = Some(state);
    }
}

#[test]
fn two_unconnected_guests_have_independent_peripherals() {
    let rom = firmware();
    let mut a_ram = [0; RAM_LEN];
    let mut b_ram = [0; RAM_LEN];
    let mut a = controller(&mut a_ram, &rom, ClockHz::BOCHS_DEFAULT).unwrap();
    let mut b = controller(&mut b_ram, &rom, ClockHz::BOCHS_DEFAULT).unwrap();
    assert_eq!(
        a.step(RunBudget::Ticks(32)).unwrap().stop,
        StopReason::Halted
    );
    assert_eq!(
        b.step(RunBudget::Ticks(32)).unwrap().stop,
        StopReason::Halted
    );
    set_inputs(&mut a, 90, 5);
    assert_eq!(
        a.step(RunBudget::Ticks(64)).unwrap().stop,
        StopReason::Halted
    );
    assert_eq!(
        b.step(RunBudget::Ticks(64))
            .unwrap()
            .progress
            .instructions(),
        Some(0)
    );
    assert_eq!(output(&a), 95);
    assert_eq!(output(&b), 0);
}

#[test]
fn an_incompatible_rate_is_rejected_before_pairing() {
    let rom = firmware();
    let mut a_ram = [0; RAM_LEN];
    let mut b_ram = [0; RAM_LEN];
    let a = controller(&mut a_ram, &rom, ClockHz::BOCHS_DEFAULT).unwrap();
    let b = controller(&mut b_ram, &rom, ClockHz::new(1_000).unwrap()).unwrap();
    assert!(matches!(
        ConnectedPair::new(a, b, digital_link()),
        Err(PairError::RateMismatch { .. })
    ));
}
