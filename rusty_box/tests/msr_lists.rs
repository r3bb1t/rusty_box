//! RDMSRLIST and WRMSRLIST (Bochs cpu/msr.cc), run as guest instructions on a
//! model offered MSRLIST the way Bochs's `cpuid: msrlist=1` offers it.
//!
//! Each set bit of RCX names one slot of two tables: the MSR index at RSI and
//! the value at RDI. An entry's bit clears once its MSR is done, so a fault
//! part-way leaves RCX naming exactly the entries still to do.

#![cfg(feature = "std")]

use rusty_box::cpu::decoder::X86Feature;
use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};
use rusty_box::params::BxParams;

const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
const CODE: u64 = 0x0020_0000;
/// The MSR-index table, the values to write, and where the reads land.
const INDEXES: u64 = 0x0021_0000;
const VALUES_IN: u64 = 0x0021_1000;
const VALUES_OUT: u64 = 0x0021_2000;
const IDT: u64 = 0x0024_0000;
/// #GP handler: a HLT the run stops on.
const GP_HANDLER: u64 = 0x0025_0000;
const STACK_TOP: u64 = 0x0026_0000;

const SYSENTER_CS: u64 = 0x174;
const SYSENTER_ESP: u64 = 0x175;
/// Written to slot 1 of the output table, which no RCX bit names.
const UNTOUCHED: u64 = 0x5555_5555_5555_5555;

/// WRMSRLIST: F3 0F 01 C6. RDMSRLIST: F2 0F 01 C6.
const WRMSRLIST: [u8; 4] = [0xF3, 0x0F, 0x01, 0xC6];
const RDMSRLIST: [u8; 4] = [0xF2, 0x0F, 0x01, 0xC6];

fn emulator() -> Box<Emulator> {
    let config = EmulatorConfig {
        cpu_params: BxParams::default().including(X86Feature::IsaMsrlist),
        ..EmulatorConfig::default()
    };
    let mut emu = Emulator::new_with_mode(config, CpuSetupMode::FlatLong64).expect("emulator");
    emu.reg_write(X86Reg::IdtrBase, IDT);
    emu.reg_write(X86Reg::IdtrLimit, 256 * 16 - 1);
    emu.reg_write(X86Reg::Rsp, STACK_TOP);
    let mut gate = [0u8; 16];
    gate[0..2].copy_from_slice(&(GP_HANDLER as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((GP_HANDLER >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((GP_HANDLER >> 32) as u32).to_le_bytes());
    emu.mem_write(IDT + 13 * 16, &gate).expect("idt");
    emu.mem_write(GP_HANDLER, &[0xF4]).expect("handler");
    emu
}

fn write_table(emu: &mut Emulator, at: u64, entries: &[u64]) {
    let bytes: Vec<u8> = entries.iter().flat_map(|entry| entry.to_le_bytes()).collect();
    emu.mem_write(at, &bytes).expect("table");
}

fn read_qword(emu: &mut Emulator, at: u64) -> u64 {
    let mut bytes = [0u8; 8];
    emu.mem_read(at, &mut bytes).expect("read");
    u64::from_le_bytes(bytes)
}

/// How a run ended.
#[derive(Debug, PartialEq, Eq)]
enum Ended {
    Retired,
    GeneralProtection,
}

/// Run one instruction with RSI, RDI and RCX as given.
fn run(emu: &mut Emulator, instruction: &[u8], rdi: u64, rcx: u64) -> Ended {
    emu.reg_write(X86Reg::Rsi, INDEXES);
    emu.reg_write(X86Reg::Rdi, rdi);
    emu.reg_write(X86Reg::Rcx, rcx);
    let mut image = instruction.to_vec();
    let end = CODE + image.len() as u64;
    image.extend_from_slice(&[0xEB, 0xFE]);
    emu.mem_write(CODE, &image).expect("write code");
    emu.emu_start(CODE, Some(end), None, Some(16))
        .expect("execute");
    match emu.cpu().rip() {
        rip if rip == end => Ended::Retired,
        rip if rip == GP_HANDLER || rip == GP_HANDLER + 1 => Ended::GeneralProtection,
        rip => panic!("the run stopped at {rip:#x}, neither the end nor the #GP handler"),
    }
}

fn on_big_stack(check: fn()) {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(check)
        .expect("spawn")
        .join()
        .expect("join");
}

/// WRMSRLIST writes the MSRs RCX names, RDMSRLIST reads them back, each slot
/// pairing an index with a value, and RCX ends at zero.
#[test]
fn the_lists_write_and_read_the_msrs_rcx_names() {
    on_big_stack(|| {
        let mut emu = emulator();
        const ESP: u64 = 0xFFFF_8000_0000_1000;
        write_table(&mut emu, INDEXES, &[SYSENTER_CS, 0, SYSENTER_ESP]);
        write_table(&mut emu, VALUES_IN, &[0x10, 0, ESP]);
        write_table(&mut emu, VALUES_OUT, &[0, UNTOUCHED, 0]);

        assert_eq!(run(&mut emu, &WRMSRLIST, VALUES_IN, 0b101), Ended::Retired);
        assert_eq!(emu.reg_read(X86Reg::Rcx), 0, "every entry done");
        assert_eq!(emu.cpu().read_msr_for_api(SYSENTER_CS as u32).expect("msr"), 0x10);
        assert_eq!(emu.cpu().read_msr_for_api(SYSENTER_ESP as u32).expect("msr"), ESP);

        assert_eq!(run(&mut emu, &RDMSRLIST, VALUES_OUT, 0b101), Ended::Retired);
        assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
        assert_eq!(read_qword(&mut emu, VALUES_OUT), 0x10);
        assert_eq!(read_qword(&mut emu, VALUES_OUT + 8), UNTOUCHED, "slot 1 not named");
        assert_eq!(read_qword(&mut emu, VALUES_OUT + 16), ESP);
    });
}

/// An index entry with bits 63:32 set raises #GP(0) at that entry, after the
/// entries below it are done: RCX still names it and those above it.
#[test]
fn a_reserved_index_bit_faults_at_its_own_entry() {
    on_big_stack(|| {
        let mut emu = emulator();
        write_table(&mut emu, INDEXES, &[SYSENTER_CS, 0x1_0000_0000 | SYSENTER_ESP]);
        write_table(&mut emu, VALUES_IN, &[0x23, 0]);

        assert_eq!(run(&mut emu, &WRMSRLIST, VALUES_IN, 0b11), Ended::GeneralProtection);
        assert_eq!(emu.cpu().read_msr_for_api(SYSENTER_CS as u32).expect("msr"), 0x23);
        assert_eq!(emu.reg_read(X86Reg::Rcx), 0b10, "entry 1 is still owed");
    });
}

/// Both tables must be 8-byte aligned: #GP(0) before any entry is done.
#[test]
fn a_misaligned_table_faults_before_any_entry() {
    on_big_stack(|| {
        let mut emu = emulator();
        write_table(&mut emu, INDEXES, &[SYSENTER_CS]);
        assert_eq!(run(&mut emu, &RDMSRLIST, VALUES_OUT + 4, 0b1), Ended::GeneralProtection);
        assert_eq!(emu.reg_read(X86Reg::Rcx), 0b1, "nothing was done");
    });
}
