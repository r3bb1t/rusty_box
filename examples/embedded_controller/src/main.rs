use rusty_box_aarch64::{ClockHz, RunBudget};
use rusty_box_embedded_controller::{RAM_LEN, controller, firmware, set_inputs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rom = firmware();
    let mut ram = [0; RAM_LEN];
    let rate = ClockHz::new(1_000_000).ok_or("invalid tick rate")?;
    let mut machine = controller(&mut ram, &rom, rate)?;
    let idle = machine.step(RunBudget::Ticks(32))?;
    set_inputs(&mut machine, 90, 5);
    let awake = machine.step(RunBudget::Ticks(32))?;
    let output = machine.board().devices().second().device().output();
    println!(
        "Controller {:?} then {:?}. Guest wrote GPIO {output}.",
        idle.stop, awake.stop
    );
    Ok(())
}
