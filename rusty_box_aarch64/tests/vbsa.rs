//! What a guest sees of the Virtual Base System Architecture rules the
//! board meets by a choice of its own (Arm DEN0150, README "V-BSA
//! conformance"), on both processors:
//!
//! - RV_L1PR_01 (BSA DEN0094C RB_PER_05): the UART is a PL011 r1p5, an
//!   instance of BSA's Generic UART, whose FIFOs hold at least 32 entries
//!   (DEN0094C B.1): `UARTPeriphID2` reads revision 3 (DDI 0183G Table
//!   3-21), and the transmit FIFO takes 32 characters before `UARTFR.TXFF`.
//! - RV_L1PP_00 (BSA RB_PPI_01): the DCC's COMMIRQ is a level-sensitive
//!   PPI, INTID 22: enabled by `MDCCINT_EL1.TX` while DTRTX is empty, it is
//!   pending at the distributor and acknowledged as 22, and filling DTRTX
//!   withdraws it.
//!
//! Each guest is a few instructions at the base of RAM, where a bare
//! machine starts, encoded by hand; the comments give their assembly.

use rusty_box_aarch64::board::{GIC_CPU_BASE, GIC_DIST_BASE, PPI_DCC, RAM_BASE, UART0_BASE};
use rusty_box_aarch64::{
    CpuModel, LentStorage, MachineBuilder, MemoryLoan, NoEl2, RamSize, Reg, RunBudget, StopReason,
    VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 1 << 20;
/// `b .`
const SPIN: u32 = 0x1400_0000;

/// Run `program` from the base of RAM on a bare machine with `cpu`, the
/// registers `regs` set first, until it reaches its last instruction, a
/// `b .`; then `check` the machine.
fn run<F>(cpu: CpuModel, program: &[u32], regs: &[(Reg, u64)], check: F)
where
    F: FnOnce(&mut VirtMachine<LentStorage<'_>, NoEl2>),
{
    let mut ram = vec![0u8; RAM_LEN];
    let mut machine = MachineBuilder::new(VirtConfig {
        cpu,
        ..VirtConfig::DEFAULT
    })
    .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
    .bare()
    .build_in(MemoryLoan {
        ram: &mut ram,
        vars: &mut [],
    })
    .expect("the test board builds");
    for (i, word) in program.iter().enumerate() {
        machine
            .mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    for &(reg, value) in regs {
        machine.reg_write(reg, value);
    }
    let spin = RAM_BASE + 4 * (program.len() as u64 - 1);
    assert_eq!(
        machine.step(RunBudget::Instructions(10_000)).stop,
        StopReason::BudgetExhausted,
        "{cpu}"
    );
    assert_eq!(
        machine.reg_read(Reg::Pc),
        spin,
        "{cpu}: the guest ran to its end"
    );
    check(&mut machine);
}

/// The UART is BSA's Generic UART: a PL011 r1p5 whose transmit FIFO, held
/// back with the UART disabled (`UARTCR` at reset), takes 32 characters
/// before it reports itself full.
#[test]
fn the_uart_is_a_pl011_r1p5_with_32_entry_fifos() {
    let program = [
        0xB900_2C22, // str w2, [x1, #0x2c]    UARTLCR_H: FEN, eight bits
        0xB94F_E829, // ldr w9, [x1, #0xfe8]   UARTPeriphID2
        0xB900_0023, // 1: str w3, [x1]        UARTDR
        0x9100_0484, // add x4, x4, #1
        0xB940_1825, // ldr w5, [x1, #0x18]    UARTFR
        0x362F_FFA5, // tbz w5, #5, 1b         until TXFF
        SPIN,
    ];
    for cpu in CpuModel::ALL {
        run(
            cpu,
            &program,
            &[
                (Reg::X1, UART0_BASE),
                (Reg::X2, 0x70),
                (Reg::X3, u64::from(b'v')),
                (Reg::X4, 0),
            ],
            |m| {
                assert_eq!(m.reg_read(Reg::X9), 0x34, "{cpu}: revision 3, designer 4");
                assert_eq!(
                    m.reg_read(Reg::X4),
                    32,
                    "{cpu}: 32 characters fill the FIFO"
                );
            },
        );
    }
}

/// COMMIRQ is PPI 22, level-sensitive: `MDCCINT_EL1.TX` with DTRTX empty
/// makes it pending at the distributor, the CPU interface signals it as
/// INTID 22, and a write that fills DTRTX takes it away again.
#[test]
fn the_dccs_interrupt_request_is_ppi_22() {
    let program = [
        0xB900_0022, // str w2, [x1]           GICD_CTLR: enabled
        0xB901_0023, // str w3, [x1, #0x100]   GICD_ISENABLER0: PPI 22
        0xB900_0082, // str w2, [x4]           GICC_CTLR: enabled
        0xB900_0485, // str w5, [x4, #4]       GICC_PMR: every priority
        0xB942_0029, // ldr w9, [x1, #0x200]   GICD_ISPENDR0
        0xD510_0206, // msr mdccint_el1, x6    TX: a request on DTRTX empty
        0xB942_002A, // ldr w10, [x1, #0x200]  GICD_ISPENDR0
        0xB940_0C8B, // ldr w11, [x4, #0xc]    GICC_IAR
        0xB900_108B, // str w11, [x4, #0x10]   GICC_EOIR
        0xD513_0508, // msr dbgdtrtx_el0, x8   DTRTX full
        0xB942_002C, // ldr w12, [x1, #0x200]  GICD_ISPENDR0
        SPIN,
    ];
    let ppi = 1u64 << PPI_DCC;
    assert_eq!(PPI_DCC, 22, "BSA's recommended INTID");
    for cpu in CpuModel::ALL {
        run(
            cpu,
            &program,
            &[
                (Reg::X1, GIC_DIST_BASE),
                (Reg::X2, 1),
                (Reg::X3, ppi),
                (Reg::X4, GIC_CPU_BASE),
                (Reg::X5, 0xFF),
                (Reg::X6, 1 << 29),
                (Reg::X8, 0x5A),
            ],
            |m| {
                assert_eq!(
                    m.reg_read(Reg::X9) & ppi,
                    0,
                    "{cpu}: no request before MDCCINT_EL1"
                );
                assert_eq!(
                    m.reg_read(Reg::X10) & ppi,
                    ppi,
                    "{cpu}: pending once enabled"
                );
                assert_eq!(m.reg_read(Reg::X11), 22, "{cpu}: acknowledged as INTID 22");
                assert_eq!(
                    m.reg_read(Reg::X12) & ppi,
                    0,
                    "{cpu}: withdrawn once DTRTX is full"
                );
            },
        );
    }
}
