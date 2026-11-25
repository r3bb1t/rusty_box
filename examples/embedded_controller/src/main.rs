use rusty_box_aarch64::ClockHz;
use rusty_box_aarch64::machine::link::{ConnectedPair, PairStop};
use rusty_box_core::VmDuration;
use rusty_box_embedded_controller::{
    GpioSlot, RAM_LEN, TemperatureSlot, controller, digital_link, firmware,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rom = firmware();
    let mut ram_a = [0; RAM_LEN];
    let mut ram_b = [0; RAM_LEN];
    let rate = ClockHz::new(1_000_000).ok_or("invalid tick rate")?;
    let controller_a = controller(&mut ram_a, &rom, rate)?;
    let controller_b = controller(&mut ram_b, &rom, rate)?;
    let mut pair = ConnectedPair::new(controller_a, controller_b, digital_link())?;
    let quantum = VmDuration::from_ticks(32);
    if pair.run(1, quantum)?.stop != PairStop::Completed {
        return Err("a controller stopped during boot".into());
    }
    let clock = pair.first().clock();
    pair.first_mut().with_devices_mut(|mut devices| {
        devices.get::<TemperatureSlot>().set(90, clock);
        devices.get::<GpioSlot>().set_input(5);
    });
    let delivery = pair.run(2, quantum)?;
    if delivery.stop != PairStop::Completed || delivery.frames != 1 {
        return Err("expected one delivered frame".into());
    }
    let gpio_a = pair
        .first_mut()
        .with_devices_mut(|mut devices| devices.get::<GpioSlot>().output());
    let gpio_b = pair
        .second_mut()
        .with_devices_mut(|mut devices| devices.get::<GpioSlot>().output());
    println!("Controller A GPIO {gpio_a}, controller B GPIO {gpio_b}");
    Ok(())
}
