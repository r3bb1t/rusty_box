//! Reuse the x86 example's register model on an application-owned AArch64 bus.

#[path = "../../rusty_box/examples/hardware_topology/register_bank.rs"]
mod register_bank;

use core::convert::Infallible;

use register_bank::{RegisterBank, RegisterChanged};
use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::cpu::Step;
use rusty_box_aarch64::devices::gicv2::GicV2;
use rusty_box_aarch64::layout::Window;
use rusty_box_aarch64::{
    AccessWidth, BusFault, ClockHz, Cpu, CpuConfig, CpuModel, DeviceLayout, NoEl2,
    NoInstrumentation, PhysAddr, PhysBus, Spi,
};
use rusty_box_core::VmClock;
use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::{
    Emit, ExecutionRequest, InterruptOutput, MmioWrite, Subscriptions, publish_events,
};

const FIRST: u64 = 0x1000_0000;
const SECOND: u64 = FIRST + 4096;
const GUEST_SPI: Spi = match Spi::new(3) {
    Some(spi) => spi,
    None => panic!("invalid register device SPI"),
};

struct RegisterWindow<S> {
    model: RegisterBank,
    subscribers: S,
    spi: Spi,
}

struct GicLine<'a> {
    gic: &'a mut GicV2,
    spi: Spi,
}

impl InterruptOutput for GicLine<'_> {
    fn set_level(&mut self, asserted: bool) {
        self.gic.set_line(self.spi.intid(), asserted);
    }
}

impl<S: for<'a> Emit<RegisterChanged, GicLine<'a>, Error = Infallible>> RegisterWindow<S> {
    fn write(&mut self, access: MmioWrite, gic: &mut GicV2) -> ExecutionRequest {
        let events = self.model.write(access);
        let mut line = GicLine { gic, spi: self.spi };
        match publish_events(events, &mut self.subscribers, &mut line) {
            Ok(request) => request,
            Err(never) => match never {},
        }
    }
}

// The application chooses placements and owns the wiring. Use the native
// GIC's register windows and outputs rather than a PC IRQ adapter.
struct AppBus<A, B> {
    instructions: [u32; 12],
    first: RegisterWindow<A>,
    second: RegisterWindow<B>,
    gic: GicV2,
    stop: bool,
}

fn offset(window: Window, addr: PhysAddr, width: AccessWidth) -> Option<WindowOffset> {
    let offset = window.offset_of(addr)?.get();
    (width.bytes() <= window.size - offset).then_some(WindowOffset(offset))
}

impl<A, B> PhysBus for AppBus<A, B>
where
    A: for<'a> Emit<RegisterChanged, GicLine<'a>, Error = Infallible>,
    B: for<'a> Emit<RegisterChanged, GicLine<'a>, Error = Infallible>,
{
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _: VmClock) -> Result<u64, BusFault> {
        let dist = DeviceLayout::DEFAULT.windows().gic.dist_window();
        let data = if let Some(off) = offset(Window::new(PhysAddr::new(FIRST), 4096), addr, width) {
            self.first.model.read(off, width.bytes() as usize)
        } else if let Some(off) = offset(Window::new(PhysAddr::new(SECOND), 4096), addr, width) {
            self.second.model.read(off, width.bytes() as usize)
        } else if offset(dist, addr, width).is_some() {
            self.gic
                .dist_read(dist.offset_of(addr).ok_or(BusFault::Unmapped)?, width)
        } else {
            return Err(BusFault::Unmapped);
        };
        Ok(data & width.value_mask())
    }

    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        _: VmClock,
    ) -> Result<(), BusFault> {
        let access = |offset| MmioWrite {
            offset,
            len: width.bytes() as usize,
            value: value & width.value_mask(),
        };
        let request = if let Some(off) =
            offset(Window::new(PhysAddr::new(FIRST), 4096), addr, width)
        {
            self.first.write(access(off), &mut self.gic)
        } else if let Some(off) = offset(Window::new(PhysAddr::new(SECOND), 4096), addr, width) {
            self.second.write(access(off), &mut self.gic)
        } else {
            return Err(BusFault::Unmapped);
        };
        // The guest write committed. Stop the host runner at the next
        // instruction boundary instead of manufacturing a guest abort.
        self.stop |= request == ExecutionRequest::Stop;
        Ok(())
    }

    fn fetch(&mut self, addr: PhysAddr, _: VmClock) -> Result<u32, BusFault> {
        addr.get()
            .checked_sub(RAM_BASE)
            .filter(|offset| offset % 4 == 0)
            .and_then(|offset| usize::try_from(offset / 4).ok())
            .and_then(|index| self.instructions.get(index).copied())
            .ok_or(BusFault::Unmapped)
    }
}

fn main() {
    use core::cell::Cell;

    let order = Cell::new(0);
    let second_value = Cell::new(0);
    let first = RegisterWindow {
        spi: GUEST_SPI,
        model: RegisterBank::new(7),
        subscribers: Subscriptions::new()
            .on(|event: &RegisterChanged| {
                assert_eq!(event.value, 42);
                assert_eq!(order.replace(1), 0);
            })
            .on_with(|_: &RegisterChanged, irq: &mut GicLine<'_>| {
                assert_eq!(order.replace(2), 1);
                irq.set_level(true);
                Ok(ExecutionRequest::Stop)
            }),
    };
    let second = RegisterWindow {
        spi: Spi::new(4).unwrap(),
        model: RegisterBank::new(19),
        subscribers: Subscriptions::new()
            .on(|event: &RegisterChanged| second_value.set(event.value)),
    };

    let dist = DeviceLayout::DEFAULT.windows().gic.dist.get() as u32;
    let mut bus = AppBus {
        instructions: [
            0xD2A2_0001,                          // movz x1,#0x1000,lsl #16
            0x9140_0424,                          // add x4,x1,#1,lsl #12
            0x5280_0542,                          // mov w2,#42
            0xB900_0022,                          // str w2,[x1] (first subscriber stops here)
            0xB940_0023,                          // ldr w3,[x1]
            0xB940_0088,                          // ldr w8,[x4]
            0x5280_0122,                          // mov w2,#9
            0xB900_0082,                          // str w2,[x4]
            0xB940_0087,                          // ldr w7,[x4]
            0x5280_0005 | ((dist & 0xFFFF) << 5), // movz w5,#lo16(dist)
            0x72A0_0005 | ((dist >> 16) << 5),    // movk w5,#hi16(dist),lsl #16
            0xB942_04A6,                          // ldr w6,[x5,#0x204] (GICD_ISPENDR1)
        ],
        first,
        second,
        gic: GicV2::new(),
        stop: false,
    };
    let mut cpu = Cpu::<NoEl2>::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: ClockHz::new(1_000_000).unwrap(),
            reset_pc: RAM_BASE,
        },
    );
    for index in 0..bus.instructions.len() {
        cpu.set_interrupt_lines(bus.gic.irq_out(), bus.gic.fiq_out());
        assert_eq!(cpu.step(&mut bus, &mut NoInstrumentation), Step::Retired);
        if core::mem::take(&mut bus.stop) {
            assert_eq!(index, 3);
            assert_eq!(cpu.pc(), RAM_BASE + 16);
            assert_eq!(order.get(), 2);
            assert_eq!(second_value.get(), 0);
            // Continue from the retired write. Its handlers run once.
        }
    }
    assert_eq!(cpu.xreg(3), 42);
    assert_eq!(cpu.xreg(8), 19);
    assert_eq!(cpu.xreg(7), 9);
    assert_ne!(cpu.xreg(6) & (1 << GUEST_SPI.number()), 0);
    assert_eq!(order.get(), 2);
    assert_eq!(second_value.get(), 9);
    println!("AArch64 read both register banks and observed the native GIC pending bit.");
}

#[test]
fn reused_device_runs_against_aarch64() {
    main();
}
