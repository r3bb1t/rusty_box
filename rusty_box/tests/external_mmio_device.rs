#![cfg(feature = "std")]

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::memory::mmio::{MmioAttachRefusal, MmioDevice};
use rusty_box::{Emulator, EmulatorConfig};

/// A register device owns its state. A board places its one page.
struct Register(u32);

impl MmioDevice for Register {
    fn read(&mut self, offset: u64, len: usize) -> u64 {
        let bytes = self.0.to_le_bytes();
        let mut value = [0xFF; 8];
        for (index, byte) in value.iter_mut().take(len).enumerate() {
            if let Some(&source) = bytes.get(offset as usize + index) {
                *byte = source;
            }
        }
        u64::from_le_bytes(value)
    }

    fn write(&mut self, offset: u64, len: usize, value: u64) {
        let mut bytes = self.0.to_le_bytes();
        for (index, &byte) in value.to_le_bytes().iter().take(len).enumerate() {
            if let Some(target) = bytes.get_mut(offset as usize + index) {
                *target = byte;
            }
        }
        self.0 = u32::from_le_bytes(bytes);
    }
}

#[test]
fn device_register_is_testable_without_a_machine() {
    let mut device = Register(0xAABB_CCDD);
    device.write(1, 2, 0x3344);
    assert_eq!(device.read(1, 2) as u16, 0x3344);
    assert_eq!(device.read(0, 4) as u32, 0xAA33_44DD);
}

#[test]
fn full_pc_routes_guest_mmio_and_recovers_the_device_on_detach() {
    let mut machine =
        Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64).unwrap();
    let register = BxPhyAddress::new(0x90000);
    let code = BxPhyAddress::new(0x10000);
    // mov eax,[0x90000]; inc eax; mov [0x90000],eax
    machine
        .mem_write(
            code,
            &[
                0xA1, 0, 0, 9, 0, 0, 0, 0, 0, 0xFF, 0xC0, 0xA3, 0, 0, 9, 0, 0, 0, 0, 0,
            ],
        )
        .unwrap();
    machine.mem_write(register, &7u32.to_le_bytes()).unwrap();
    machine.reg_write(X86Reg::Rip, code.get());
    machine.step_one().unwrap(); // establish warm RAM mapping
    assert_eq!(machine.reg_read(X86Reg::Eax), 7);

    let device = Box::leak(Box::new(Register(41)));
    machine.attach_mmio_device(register, device).unwrap();
    let spare = Box::leak(Box::new(Register(99)));
    let refusal = machine.attach_mmio_device(register, spare).unwrap_err();
    assert_eq!(refusal.reason, MmioAttachRefusal::Overlap);
    assert_eq!(refusal.device.read(0, 4) as u32, 99);

    machine.reg_write(X86Reg::Rip, code.get());
    for _ in 0..3 {
        machine.step_one().unwrap();
    }
    assert_eq!(machine.reg_read(X86Reg::Eax), 42);
    let mut data = [0; 4];
    machine.mem_read(register, &mut data).unwrap();
    assert_eq!(u32::from_le_bytes(data), 7);

    let device = machine.detach_mmio_device(register).unwrap();
    assert_eq!(device.read(0, 4) as u32, 42);
    machine.reg_write(X86Reg::Rip, code.get());
    machine.step_one().unwrap();
    assert_eq!(machine.reg_read(X86Reg::Eax), 7);
}
