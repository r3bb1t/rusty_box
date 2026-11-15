//! A board with a GICv3, as a guest finds it: the processor's GIC CPU
//! interface behind its System registers, interrupts configured through the
//! Distributor and the Redistributor, taken, ended, masked and preempting,
//! the timer's PPI and SGIs to itself; and what a board with a GICv2 keeps
//! instead. Each guest is assembled with GNU as for aarch64 and linked at
//! the base of RAM, where a bare machine starts; its words below are
//! `objdump`'s, its source above each.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    BuildError, CpuModel, DeviceLayout, GicCpuInterfaceBits, GicShape, GicVersion, LentStorage,
    MachineBuilder, MemoryLoan, NoEl2, PriorityBits, RamSize, Reg, RunBudget, StopReason,
    VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 1 << 20;
const GIVE_UP_AFTER: u64 = 50_000_000;

/// ```text
///         .equ GICD, 0x08000000
///         .equ GICR, 0x080A0000
/// _start: ldr x0, =0x40001000          ; msr vbar_el1, x0
///         ldr x0, =0x40010000          ; mov sp, x0
///         mrs x21, icc_sre_el1         ; mrs x23, id_aa64pfr0_el1
///         ldr x1, =GICR                ; str wzr, [x1, #0x14]       // GICR_WAKER: awake
/// 1:      ldr w2, [x1, #0x14]          ; tbnz w2, #2, 1b            // until ChildrenAsleep is 0
///         add x3, x1, #0x10, lsl #12                                // SGI_base
///         mov w2, #(1 << 27)           ; orr w2, w2, #(1 << 5)
///         str w2, [x3, #0x80]                                       // GICR_IGROUPR0: PPI 27, SGI 5 Group 1
///         orr w2, w2, #(1 << 3)        ; str w2, [x3, #0x100]       // GICR_ISENABLER0: and SGI 3
///         ldr x4, =GICD                ; mov w2, #3 ; str w2, [x4]  // GICD_CTLR: EnableGrp0, EnableGrp1
///         mov x2, #0xff                ; msr icc_pmr_el1, x2
///         mov x2, #1                   ; msr icc_igrpen0_el1, x2 ; msr icc_igrpen1_el1, x2
///         mov x5, #16000               ; msr cntv_tval_el0, x5
///         mov x5, #1                   ; msr cntv_ctl_el0, x5
///         mov x20, #0 ; mov x22, #0 ; mov x24, #0 ; msr daifclr, #3
/// 2:      wfi                          ; cmp x20, #5 ; b.lt 2b
///         msr cntv_ctl_el0, xzr
///         ldr x6, =0x05000001          ; msr icc_sgi1r_el1, x6 ; isb    // SGI 5 to TargetList bit 0
/// 3:      cbz x22, 3b
///         ldr x6, =0x03000001          ; msr icc_sgi0r_el1, x6 ; isb    // SGI 3
/// 4:      cbz x24, 4b
///         ldr x0, =0x84000008          ; hvc #0                         // PSCI SYSTEM_OFF
/// 5:      b 5b
///         .org 0x1280                                               // IRQ, current EL with SP_ELx
///         mrs x7, icc_iar1_el1         ; cmp x7, #27 ; b.ne 6f
///         add x20, x20, #1             ; mov x5, #16000 ; msr cntv_tval_el0, x5 ; b 7f
/// 6:      mov x22, x7
/// 7:      msr icc_eoir1_el1, x7        ; eret
///         .org 0x1300                                               // FIQ, current EL with SP_ELx
///         mrs x8, icc_iar0_el1         ; mov x24, x8 ; msr icc_eoir0_el1, x8 ; eret
/// ```
const INTERRUPTS: &[(u64, u32)] = &[
    (0x0, 0x5800_0600),
    (0x4, 0xD518_C000),
    (0x8, 0x5800_0600),
    (0xC, 0x9100_001F),
    (0x10, 0xD538_CCB5),
    (0x14, 0xD538_0417),
    (0x18, 0x5800_05C1),
    (0x1C, 0xB900_143F),
    (0x20, 0xB940_1422),
    (0x24, 0x3717_FFE2),
    (0x28, 0x9140_4023),
    (0x2C, 0x52A1_0002),
    (0x30, 0x321B_0042),
    (0x34, 0xB900_8062),
    (0x38, 0x321D_0042),
    (0x3C, 0xB901_0062),
    (0x40, 0x5800_04C4),
    (0x44, 0x5280_0062),
    (0x48, 0xB900_0082),
    (0x4C, 0xD280_1FE2),
    (0x50, 0xD518_4602),
    (0x54, 0xD280_0022),
    (0x58, 0xD518_CCC2),
    (0x5C, 0xD518_CCE2),
    (0x60, 0xD287_D005),
    (0x64, 0xD51B_E305),
    (0x68, 0xD280_0025),
    (0x6C, 0xD51B_E325),
    (0x70, 0xD280_0014),
    (0x74, 0xD280_0016),
    (0x78, 0xD280_0018),
    (0x7C, 0xD503_43FF),
    (0x80, 0xD503_207F),
    (0x84, 0xF100_169F),
    (0x88, 0x54FF_FFCB),
    (0x8C, 0xD51B_E33F),
    (0x90, 0x5800_0286),
    (0x94, 0xD518_CBA6),
    (0x98, 0xD503_3FDF),
    (0x9C, 0xB400_0016),
    (0xA0, 0x5800_0246),
    (0xA4, 0xD518_CBE6),
    (0xA8, 0xD503_3FDF),
    (0xAC, 0xB400_0018),
    (0xB0, 0x5800_0200),
    (0xB4, 0xD400_0002),
    (0xB8, 0x1400_0000),
    (0xC0, 0x4000_1000),
    (0xC8, 0x4001_0000),
    (0xD0, 0x080A_0000),
    (0xD8, 0x0800_0000),
    (0xE0, 0x0500_0001),
    (0xE8, 0x0300_0001),
    (0xF0, 0x8400_0008),
    (0x1280, 0xD538_CC07),
    (0x1284, 0xF100_6CFF),
    (0x1288, 0x5400_00A1),
    (0x128C, 0x9100_0694),
    (0x1290, 0xD287_D005),
    (0x1294, 0xD51B_E305),
    (0x1298, 0x1400_0002),
    (0x129C, 0xAA07_03F6),
    (0x12A0, 0xD518_CC27),
    (0x12A4, 0xD69F_03E0),
    (0x1300, 0xD538_C808),
    (0x1304, 0xAA08_03F8),
    (0x1308, 0xD518_C828),
    (0x130C, 0xD69F_03E0),
];

/// With IRQs masked, SPIs 40, 41 and 42 (Group 1, priorities 0x80, 0x40
/// and 0x90) made pending through `GICD_ISPENDR1`, and what the CPU
/// interface makes of them read into registers; then, with SPI 40 active
/// and SPI 42 pending, IRQs unmasked and SPI 41 made pending: the handler
/// takes 41 alone.
///
/// ```text
///         ldr x0, =0x40001000          ; msr vbar_el1, x0
///         ldr x0, =0x40010000          ; mov sp, x0
///         ldr x1, =GICR                ; str wzr, [x1, #0x14]
/// 1:      ldr w2, [x1, #0x14]          ; tbnz w2, #2, 1b
///         ldr x4, =GICD
///         mov w2, #(7 << 8)            ; str w2, [x4, #0x84] ; str w2, [x4, #0x104] // IGROUPR1, ISENABLER1
///         mov w2, #0x80 ; strb w2, [x4, #(0x400 + 40)]
///         mov w2, #0x40 ; strb w2, [x4, #(0x400 + 41)]
///         mov w2, #0x90 ; strb w2, [x4, #(0x400 + 42)]
///         mov w2, #2 ; str w2, [x4]                                 // GICD_CTLR: EnableGrp1
///         mov x2, #0xff ; msr icc_pmr_el1, x2 ; mov x2, #1 ; msr icc_igrpen1_el1, x2
///         mov w5, #(1 << 8) ; str w5, [x4, #0x204]                  // SPI 40 pending
///         mrs x10, icc_hppir1_el1 ; mrs x11, icc_iar1_el1 ; mrs x12, icc_rpr_el1
///         mov w5, #(1 << 9) ; str w5, [x4, #0x204]                  // SPI 41 pending
///         mrs x13, icc_iar1_el1 ; mrs x14, icc_rpr_el1
///         mov w5, #(1 << 10) ; str w5, [x4, #0x204]                 // SPI 42 pending
///         mrs x15, icc_iar1_el1
///         msr icc_eoir1_el1, x13 ; mrs x16, icc_rpr_el1
///         msr icc_eoir1_el1, x11 ; mrs x17, icc_rpr_el1
///         mov x2, #0x90 ; msr icc_pmr_el1, x2 ; mrs x18, icc_iar1_el1
///         mov x2, #0xa0 ; msr icc_pmr_el1, x2 ; mrs x19, icc_iar1_el1
///         mov x2, #2 ; msr icc_ctlr_el1, x2                         // EOImode
///         msr icc_eoir1_el1, x19 ; ldr w20, [x4, #0x304] ; mrs x21, icc_rpr_el1
///         msr icc_dir_el1, x19 ; ldr w22, [x4, #0x304]
///         msr icc_ctlr_el1, xzr ; mov x2, #0xff ; msr icc_pmr_el1, x2
///         mov w5, #(1 << 8) ; str w5, [x4, #0x204] ; mrs x24, icc_iar1_el1
///         mov w5, #(1 << 10) ; str w5, [x4, #0x204]
///         mov x25, #0 ; msr daifclr, #2 ; nop ; nop
///         mov w5, #(1 << 9) ; str w5, [x4, #0x204]
/// 2:      cbz x25, 2b
///         msr daifset, #2 ; msr icc_eoir1_el1, x24 ; mrs x26, icc_hppir1_el1
///         ldr x0, =0x84000008 ; hvc #0
/// 3:      b 3b
///         .org 0x1280
///         mrs x7, icc_iar1_el1 ; mov x25, x7 ; msr icc_eoir1_el1, x7 ; eret
/// ```
const PRIORITIES: &[(u64, u32)] = &[
    (0x0, 0x5800_0940),
    (0x4, 0xD518_C000),
    (0x8, 0x5800_0940),
    (0xC, 0x9100_001F),
    (0x10, 0x5800_0941),
    (0x14, 0xB900_143F),
    (0x18, 0xB940_1422),
    (0x1C, 0x3717_FFE2),
    (0x20, 0x5800_0904),
    (0x24, 0x5280_E002),
    (0x28, 0xB900_8482),
    (0x2C, 0xB901_0482),
    (0x30, 0x5280_1002),
    (0x34, 0x3910_A082),
    (0x38, 0x5280_0802),
    (0x3C, 0x3910_A482),
    (0x40, 0x5280_1202),
    (0x44, 0x3910_A882),
    (0x48, 0x5280_0042),
    (0x4C, 0xB900_0082),
    (0x50, 0xD280_1FE2),
    (0x54, 0xD518_4602),
    (0x58, 0xD280_0022),
    (0x5C, 0xD518_CCE2),
    (0x60, 0x5280_2005),
    (0x64, 0xB902_0485),
    (0x68, 0xD538_CC4A),
    (0x6C, 0xD538_CC0B),
    (0x70, 0xD538_CB6C),
    (0x74, 0x5280_4005),
    (0x78, 0xB902_0485),
    (0x7C, 0xD538_CC0D),
    (0x80, 0xD538_CB6E),
    (0x84, 0x5280_8005),
    (0x88, 0xB902_0485),
    (0x8C, 0xD538_CC0F),
    (0x90, 0xD518_CC2D),
    (0x94, 0xD538_CB70),
    (0x98, 0xD518_CC2B),
    (0x9C, 0xD538_CB71),
    (0xA0, 0xD280_1202),
    (0xA4, 0xD518_4602),
    (0xA8, 0xD538_CC12),
    (0xAC, 0xD280_1402),
    (0xB0, 0xD518_4602),
    (0xB4, 0xD538_CC13),
    (0xB8, 0xD280_0042),
    (0xBC, 0xD518_CC82),
    (0xC0, 0xD518_CC33),
    (0xC4, 0xB943_0494),
    (0xC8, 0xD538_CB75),
    (0xCC, 0xD518_CB33),
    (0xD0, 0xB943_0496),
    (0xD4, 0xD518_CC9F),
    (0xD8, 0xD280_1FE2),
    (0xDC, 0xD518_4602),
    (0xE0, 0x5280_2005),
    (0xE4, 0xB902_0485),
    (0xE8, 0xD538_CC18),
    (0xEC, 0x5280_8005),
    (0xF0, 0xB902_0485),
    (0xF4, 0xD280_0019),
    (0xF8, 0xD503_42FF),
    (0xFC, 0xD503_201F),
    (0x100, 0xD503_201F),
    (0x104, 0x5280_4005),
    (0x108, 0xB902_0485),
    (0x10C, 0xB400_0019),
    (0x110, 0xD503_42DF),
    (0x114, 0xD518_CC38),
    (0x118, 0xD538_CC5A),
    (0x11C, 0x5800_0160),
    (0x120, 0xD400_0002),
    (0x124, 0x1400_0000),
    (0x128, 0x4000_1000),
    (0x130, 0x4001_0000),
    (0x138, 0x080A_0000),
    (0x140, 0x0800_0000),
    (0x148, 0x8400_0008),
    (0x1280, 0xD538_CC07),
    (0x1284, 0xAA07_03F9),
    (0x1288, 0xD518_CC27),
    (0x128C, 0xD69F_03E0),
];

/// ```text
/// _start: ldr x0, =0x40001000 ; msr vbar_el1, x0 ; ldr x0, =0x40010000 ; mov sp, x0
///         mov x9, #0 ; mov x11, #0 ; mov x21, #0xdead
///         mrs x23, id_aa64pfr0_el1
///         mrs x21, icc_sre_el1
///         adr x0, el0 ; msr elr_el1, x0 ; msr spsr_el1, xzr ; eret
/// el0:    mrs x12, icc_pmr_el1
///         svc #0
/// 1:      b 1b
///         .org 0x1200                  // Synchronous, current EL: x9 the syndrome, skip
///         mrs x9, esr_el1 ; mrs x10, elr_el1 ; add x10, x10, #4 ; msr elr_el1, x10 ; eret
///         .org 0x1400                  // Synchronous, lower EL: SVC powers off, else x11
///         mrs x13, esr_el1 ; lsr x14, x13, #26 ; cmp x14, #0x15 ; b.eq 2f
///         mov x11, x13 ; mrs x10, elr_el1 ; add x10, x10, #4 ; msr elr_el1, x10 ; eret
/// 2:      ldr x0, =0x84000008 ; hvc #0
/// 3:      b 3b
/// ```
const INTERFACE: &[(u64, u32)] = &[
    (0x0, 0x5800_0200),
    (0x4, 0xD518_C000),
    (0x8, 0x5800_0200),
    (0xC, 0x9100_001F),
    (0x10, 0xD280_0009),
    (0x14, 0xD280_000B),
    (0x18, 0xD29B_D5B5),
    (0x1C, 0xD538_0417),
    (0x20, 0xD538_CCB5),
    (0x24, 0x1000_0080),
    (0x28, 0xD518_4020),
    (0x2C, 0xD518_401F),
    (0x30, 0xD69F_03E0),
    (0x34, 0xD538_460C),
    (0x38, 0xD400_0001),
    (0x3C, 0x1400_0000),
    (0x40, 0x4000_1000),
    (0x48, 0x4001_0000),
    (0x1200, 0xD538_5209),
    (0x1204, 0xD538_402A),
    (0x1208, 0x9100_114A),
    (0x120C, 0xD518_402A),
    (0x1210, 0xD69F_03E0),
    (0x1400, 0xD538_520D),
    (0x1404, 0xD35A_FDAE),
    (0x1408, 0xF100_55DF),
    (0x140C, 0x5400_00C0),
    (0x1410, 0xAA0D_03EB),
    (0x1414, 0xD538_402A),
    (0x1418, 0x9100_114A),
    (0x141C, 0xD518_402A),
    (0x1420, 0xD69F_03E0),
    (0x1424, 0x5800_0060),
    (0x1428, 0xD400_0002),
    (0x142C, 0x1400_0000),
    (0x1430, 0x8400_0008),
];

/// The UNDEFINED syndrome: EC 0, IL 1.
const UNDEFINED: u64 = 1 << 25;

fn config(cpu: CpuModel, gic_version: GicVersion, layout: DeviceLayout) -> VirtConfig {
    VirtConfig {
        cpu,
        gic_version,
        layout,
        interrupts: rusty_box_aarch64::InterruptMap::own(layout),
        ..VirtConfig::DEFAULT
    }
}

/// A bare machine of `config` running `program` from the base of RAM to
/// its power-off; `check` reads what it left.
fn run<F>(config: VirtConfig, program: &[(u64, u32)], check: F)
where
    F: FnOnce(&mut VirtMachine<LentStorage<'_>, NoEl2>),
{
    let mut ram = vec![0u8; RAM_LEN];
    let mut machine = MachineBuilder::new(config)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("the board builds");
    for &(at, word) in program {
        machine
            .mem_write(RAM_BASE + at, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    let out = machine.step(RunBudget::Ticks(GIVE_UP_AFTER));
    assert_eq!(
        out.stop,
        StopReason::PoweredOff,
        "the guest powers off; it stopped at {:#x}",
        machine.reg_read(Reg::Pc)
    );
    check(&mut machine);
}

/// On a GICv3 board, on either processor and either layout, the guest
/// finds the System register interface (`ID_AA64PFR0_EL1.GIC` 1,
/// `ICC_SRE_EL1` 0b111), takes the virtual timer's PPI 27 five times as a
/// Group 1 IRQ, and an SGI it sends itself through `ICC_SGI1R_EL1` as IRQ
/// and one through `ICC_SGI0R_EL1`, of Group 0, as FIQ.
#[test]
fn the_timer_and_sgis_interrupt_through_the_cpu_interface() {
    for cpu in CpuModel::ALL {
        for layout in DeviceLayout::ALL {
            run(config(cpu, GicVersion::V3, layout), INTERRUPTS, |m| {
                assert_eq!((m.reg_read(Reg::X23) >> 24) & 0xF, 1, "{cpu} {layout}: GIC");
                assert_eq!(m.reg_read(Reg::X21), 0b111, "{cpu} {layout}: ICC_SRE_EL1");
                assert_eq!(m.reg_read(Reg::X20), 5, "{cpu} {layout}: timer interrupts");
                assert_eq!(m.reg_read(Reg::X22), 5, "{cpu} {layout}: SGI 5, an IRQ");
                assert_eq!(m.reg_read(Reg::X24), 3, "{cpu} {layout}: SGI 3, an FIQ");
            });
        }
    }
}

/// The CPU interface acknowledges the highest priority pending interrupt,
/// makes its priority the running priority, lets one of higher priority
/// preempt it and holds back one of lower, masks what `ICC_PMR_EL1` masks,
/// splits the priority drop from the deactivation under `EOImode`; and an
/// IRQ is taken for the interrupt that preempts, not for the one that
/// cannot.
#[test]
fn priorities_mask_preempt_and_end() {
    run(
        config(CpuModel::CortexA57, GicVersion::V3, DeviceLayout::Board),
        PRIORITIES,
        |m| {
            let read = |m: &mut VirtMachine<LentStorage<'_>, NoEl2>, reg| m.reg_read(reg);
            let got: Vec<u64> = [
                Reg::X10,
                Reg::X11,
                Reg::X12,
                Reg::X13,
                Reg::X14,
                Reg::X15,
                Reg::X16,
                Reg::X17,
                Reg::X18,
                Reg::X19,
                Reg::X20,
                Reg::X21,
                Reg::X22,
                Reg::X24,
                Reg::X25,
                Reg::X26,
            ]
            .into_iter()
            .map(|reg| read(m, reg))
            .collect();
            assert_eq!(
                got,
                [
                    40,      // HPPIR1: SPI 40
                    40,      // IAR1 acknowledges it
                    0x80,    // running at its priority
                    41,      // SPI 41, at 0x40, preempts
                    0x40,    // running at its priority
                    1023,    // SPI 42, at 0x90, does not
                    0x80,    // 41 ended: running at 40's again
                    0xFF,    // 40 ended: idle
                    1023,    // PMR 0x90 masks 42
                    42,      // PMR 0xA0 lets it through
                    1 << 10, // EOImode: ended, still active
                    0xFF,    // its priority dropped
                    0,       // ICC_DIR_EL1 deactivated it
                    40,      // acknowledged again
                    41,      // the IRQ taken: 41, not 42
                    42,      // and 42 pending all along
                ]
            );
        },
    );
}

/// The processor's GIC CPU interface follows the board's GIC: on a GICv3
/// board `ID_AA64PFR0_EL1.GIC` is 1, `ICC_SRE_EL1` reads 0b111 at EL1 and
/// `ICC_PMR_EL1` is UNDEFINED at EL0; on a GICv2 board `GIC` is 0 and every
/// `ICC_*` register UNDEFINED.
#[test]
fn the_cpu_interface_follows_the_boards_gic() {
    for cpu in CpuModel::ALL {
        run(
            config(cpu, GicVersion::V3, DeviceLayout::Board),
            INTERFACE,
            |m| {
                assert_eq!((m.reg_read(Reg::X23) >> 24) & 0xF, 1, "{cpu}");
                assert_eq!(m.reg_read(Reg::X21), 0b111, "{cpu}");
                assert_eq!(m.reg_read(Reg::X9), 0, "{cpu}: nothing trapped at EL1");
                assert_eq!(m.reg_read(Reg::X11), UNDEFINED, "{cpu}: EL0's access");
            },
        );
        run(
            config(cpu, GicVersion::V2, DeviceLayout::Board),
            INTERFACE,
            |m| {
                assert_eq!((m.reg_read(Reg::X23) >> 24) & 0xF, 0, "{cpu}");
                assert_eq!(m.reg_read(Reg::X21), 0xDEAD, "{cpu}: not written");
                assert_eq!(m.reg_read(Reg::X9), UNDEFINED, "{cpu}: EL1's access");
                assert_eq!(m.reg_read(Reg::X11), UNDEFINED, "{cpu}: EL0's access");
            },
        );
    }
}

/// A GICv3 board's processor may have EL2 (its CPU interface then has its
/// virtualization support, `tests/gicv3_virtualization.rs`), and the
/// Cortex-A57's CPU interface has five priority bits, so a GICv3 of other
/// bits does not match it; the Armv9.2 model takes the board's.
#[test]
fn a_gicv3_board_refuses_what_its_processor_cannot_have() {
    let v3 = config(CpuModel::CortexA57, GicVersion::V3, DeviceLayout::Board);
    let mut ram = vec![0u8; RAM_LEN];
    let built = MachineBuilder::new(v3)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .with_el2()
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        });
    assert!(built.is_ok(), "a GICv3 board with EL2 builds");
    let six = GicShape {
        priority_bits: PriorityBits::Six,
        ..GicShape::DEFAULT
    };
    let built = MachineBuilder::new(VirtConfig { gic: six, ..v3 })
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        });
    assert_eq!(
        built.err(),
        Some(BuildError::GicCpuInterface(GicCpuInterfaceBits {
            cpu: CpuModel::CortexA57,
            interface: PriorityBits::Five,
            configured: PriorityBits::Six,
        }))
    );
    let armv9 = VirtConfig {
        cpu: CpuModel::Armv9p2,
        gic: six,
        ..v3
    };
    run(armv9, INTERFACE, |m| {
        assert_eq!(m.reg_read(Reg::X21), 0b111);
    });
}

/// A GICv3 board's snapshot restores mid-run to the same end, and is a
/// different machine from a GICv2 board's.
#[test]
fn a_gicv3_boards_snapshot_restores() {
    let v3 = config(CpuModel::CortexA57, GicVersion::V3, DeviceLayout::Board);
    let mut ram = vec![0u8; RAM_LEN];
    let mut machine = MachineBuilder::new(v3)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("the board builds");
    for &(at, word) in INTERRUPTS {
        machine
            .mem_write(RAM_BASE + at, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    // Into the timer interrupts, 16 µs apart (800 ticks at the default
    // rate): the handler has run.
    let midway = machine.step(RunBudget::Ticks(2_000));
    assert_eq!(midway.stop, StopReason::BudgetExhausted);
    let taken = machine.reg_read(Reg::X20);
    assert!((1..5).contains(&taken), "{taken} timer interrupts by then");
    let mut image = Vec::new();
    machine.save_snapshot(&mut image).expect("save");
    let finish = |m: &mut VirtMachine<LentStorage<'_>, NoEl2>| {
        let out = m.step(RunBudget::Ticks(GIVE_UP_AFTER));
        (
            out.stop,
            m.ticks(),
            m.reg_read(Reg::X22),
            m.reg_read(Reg::X24),
        )
    };
    let ran_on = finish(&mut machine);
    assert_eq!(ran_on.0, StopReason::PoweredOff);
    machine
        .restore_snapshot(&mut image.as_slice())
        .expect("the image restores");
    assert_eq!(machine.reg_read(Reg::X20), taken);
    assert_eq!(finish(&mut machine), ran_on);

    let mut v2_ram = vec![0u8; RAM_LEN];
    let mut v2 = MachineBuilder::new(VirtConfig {
        gic_version: GicVersion::V2,
        ..v3
    })
    .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
    .bare()
    .build_in(MemoryLoan {
        ram: &mut v2_ram,
        vars: &mut [],
    })
    .expect("the board builds");
    assert!(v2.restore_snapshot(&mut image.as_slice()).is_err());
}

/// The version's names read back, and only they.
#[test]
fn gic_versions_are_named_and_read_back() {
    for version in GicVersion::ALL {
        assert_eq!(version.name().parse(), Ok(version));
    }
    assert_eq!(GicVersion::DEFAULT, GicVersion::V2);
    assert!("v4".parse::<GicVersion>().is_err());
}
