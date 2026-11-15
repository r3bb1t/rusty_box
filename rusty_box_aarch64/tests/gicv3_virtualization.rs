//! A GICv3 board whose processor has EL2, as a hypervisor and its guest
//! find it (R9): the hypervisor at EL2 reads `ICH_VTR_EL2` and programs the
//! List registers, `ICH_HCR_EL2` and `ICC_SRE_EL2`; its guest at EL1, under
//! `HCR_EL2.IMO` and `FMO`, takes the virtual interrupt through
//! `ICC_IAR1_EL1` (the virtual CPU interface's `ICV_IAR1_EL1`) and ends it,
//! the List register's EOI maintenance interrupt reaching the hypervisor as
//! PPI 25; virtual priorities mask and preempt; each EL2 control traps what
//! it traps; a guest hypervisor's `ICH_*_EL2` accesses under FEAT_NV2 load
//! and store its `VNCR_EL2` page; and a snapshot taken with a virtual
//! interrupt active restores to the same end.
//!
//! Each guest is assembled with GNU as for aarch64 (`-march=armv9.2-a`) and
//! linked at the base of RAM, where a bare machine with EL2 starts at EL2;
//! its words below are the assembler's, its source above each.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    CpuModel, DeviceLayout, GicShape, GicVersion, LentStorage, MachineBuilder, MemoryLoan,
    PriorityBits, RamSize, Reg, RunBudget, StopReason, VirtConfig, VirtMachine, WithEl2,
};

const RAM_LEN: usize = 4 << 20;
const GIVE_UP_AFTER: u64 = 50_000_000;
/// Where the guests below keep their log of syndromes, and the page their
/// `VNCR_EL2` names.
const LOG: u64 = RAM_BASE + 0x3_0000;
const VNCR_PAGE: u64 = RAM_BASE + 0x4_0000;

/// `ICH_VTR_EL2` of each processor on its board of [`config`]: the
/// Cortex-A57's (DDI 0488H Table 8-13) and the Armv9.2 model's, seven
/// virtual priority bits on a CPU interface of eight.
const fn vtr(cpu: CpuModel) -> u64 {
    match cpu {
        CpuModel::CortexA57 => 0x9000_0003,
        CpuModel::Armv9p2 => 0xD818_000F,
    }
}

/// The empty List registers of `cpu`'s interface, every one.
const fn all_list_registers(cpu: CpuModel) -> u64 {
    match cpu {
        CpuModel::CortexA57 => 0xF,
        CpuModel::Armv9p2 => 0xFFFF,
    }
}

/// A GICv3 board of `cpu`: the Cortex-A57's with its CPU interface's five
/// priority bits, the Armv9.2 model's with eight, so that its virtual
/// interface has seven (`VirtualInterfaceShape::armv9p2`).
fn config(cpu: CpuModel) -> VirtConfig {
    let priority_bits = match cpu {
        CpuModel::CortexA57 => PriorityBits::Five,
        CpuModel::Armv9p2 => PriorityBits::Eight,
    };
    VirtConfig {
        cpu,
        gic_version: GicVersion::V3,
        layout: DeviceLayout::Board,
        interrupts: rusty_box_aarch64::InterruptMap::own(DeviceLayout::Board),
        gic: GicShape {
            priority_bits,
            ..GicShape::DEFAULT
        },
        ..VirtConfig::DEFAULT
    }
}

/// A bare machine with EL2 of `cpu` on a GICv3 board, `program` loaded at
/// the base of RAM.
fn machine<'a>(
    cpu: CpuModel,
    ram: &'a mut [u8],
    program: &[(u64, u32)],
) -> VirtMachine<LentStorage<'a>, WithEl2> {
    let mut machine = MachineBuilder::new(config(cpu))
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .with_el2()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a GICv3 board with EL2 builds");
    for &(at, word) in program {
        machine
            .mem_write(RAM_BASE + at, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    machine
}

/// Run `machine` to the guest's power-off.
fn run_to_power_off(machine: &mut VirtMachine<LentStorage<'_>, WithEl2>) {
    let out = machine.step(RunBudget::Ticks(GIVE_UP_AFTER));
    assert_eq!(
        out.stop,
        StopReason::PoweredOff,
        "the guest powers off; it stopped at {:#x}",
        machine.reg_read(Reg::Pc)
    );
}

fn read_u64(machine: &VirtMachine<LentStorage<'_>, WithEl2>, addr: u64) -> u64 {
    let mut bytes = [0u8; 8];
    machine.mem_read(addr, &mut bytes).expect("in RAM");
    u64::from_le_bytes(bytes)
}

/// ```text
///         .equ GICD, 0x08000000
///         .equ GICR, 0x080A0000
/// _start: ldr x0, =0x40002000          ; msr vbar_el2, x0
///         ldr x0, =0x40020000          ; mov sp, x0
///         ldr x1, =GICR                ; str wzr, [x1, #0x14]
/// 1:      ldr w2, [x1, #0x14]          ; tbnz w2, #2, 1b
///         add x3, x1, #0x10, lsl #12
///         mov w2, #(1 << 25)           ; str w2, [x3, #0x80] ; str w2, [x3, #0x100]
///         ldr x4, =GICD                ; mov w2, #2 ; str w2, [x4]
///         mov x2, #0xff                ; msr icc_pmr_el1, x2
///         mov x2, #1                   ; msr icc_igrpen1_el1, x2
///         mov x2, #0xf                 ; msr icc_sre_el2, x2 ; mrs x19, icc_sre_el2
///         mrs x20, ich_vtr_el2
///         ldr x2, =0x80000018          ; msr hcr_el2, x2       // RW, IMO, FMO
///         ldr x2, =0x50A002000000002A  ; msr ich_lr0_el2, x2   // pending, Group 1, EOI, 0xA0, 42
///         mov x2, #1                   ; msr ich_hcr_el2, x2   // En
///         adr x0, el1                  ; msr elr_el2, x0
///         mov x0, #0x3c5               ; msr spsr_el2, x0
///         mov x21, #0 ; mov x24, #0 ; mov x25, #0
///         eret
/// el1:    ldr x0, =0x40001000          ; msr vbar_el1, x0
///         ldr x0, =0x40010000          ; mov sp, x0
///         mov x2, #0xff                ; msr icc_pmr_el1, x2
///         mov x2, #1                   ; msr icc_igrpen1_el1, x2
///         msr daifclr, #2
/// 2:      cbz x25, 2b
///         mrs x26, icc_rpr_el1
///         hvc #0
/// 3:      b 3b
///         .ltorg
///         .org 0x1280                                          // EL1 IRQ
///         mrs x7, icc_iar1_el1         ; mov x24, x7 ; mrs x27, icc_rpr_el1
/// eoi:    msr icc_eoir1_el1, x7        ; mov x25, #1 ; eret
///         .org 0x2400                                          // EL2, from EL1: HVC
///         ldr x0, =0x84000008          ; smc #0
/// 4:      b 4b
///         .ltorg
///         .org 0x2480                                          // EL2 IRQ, from EL1
///         mrs x8, icc_iar1_el1         ; mov x21, x8
///         mrs x22, ich_misr_el2        ; mrs x23, ich_eisr_el2
///         msr ich_lr0_el2, xzr
///         msr icc_eoir1_el1, x8        ; eret
/// ```
const VIRTUAL_IRQ: &[(u64, u32)] = &[
    (0x0, 0x5800_0640),
    (0x4, 0xD51C_C000),
    (0x8, 0x5800_0640),
    (0xC, 0x9100_001F),
    (0x10, 0x5800_0641),
    (0x14, 0xB900_143F),
    (0x18, 0xB940_1422),
    (0x1C, 0x3717_FFE2),
    (0x20, 0x9140_4023),
    (0x24, 0x52A0_4002),
    (0x28, 0xB900_8062),
    (0x2C, 0xB901_0062),
    (0x30, 0x5800_0584),
    (0x34, 0x5280_0042),
    (0x38, 0xB900_0082),
    (0x3C, 0xD280_1FE2),
    (0x40, 0xD518_4602),
    (0x44, 0xD280_0022),
    (0x48, 0xD518_CCE2),
    (0x4C, 0xD280_01E2),
    (0x50, 0xD51C_C9A2),
    (0x54, 0xD53C_C9B3),
    (0x58, 0xD53C_CB34),
    (0x5C, 0x5800_0462),
    (0x60, 0xD51C_1102),
    (0x64, 0x5800_0462),
    (0x68, 0xD51C_CC02),
    (0x6C, 0xD280_0022),
    (0x70, 0xD51C_CB02),
    (0x74, 0x1000_0100),
    (0x78, 0xD51C_4020),
    (0x7C, 0xD280_78A0),
    (0x80, 0xD51C_4000),
    (0x84, 0xD280_0015),
    (0x88, 0xD280_0018),
    (0x8C, 0xD280_0019),
    (0x90, 0xD69F_03E0),
    (0x94, 0x5800_0320),
    (0x98, 0xD518_C000),
    (0x9C, 0x5800_0320),
    (0xA0, 0x9100_001F),
    (0xA4, 0xD280_1FE2),
    (0xA8, 0xD518_4602),
    (0xAC, 0xD280_0022),
    (0xB0, 0xD518_CCE2),
    (0xB4, 0xD503_42FF),
    (0xB8, 0xB400_0019),
    (0xBC, 0xD538_CB7A),
    (0xC0, 0xD400_0002),
    (0xC4, 0x1400_0000),
    (0xC8, 0x4000_2000),
    (0xD0, 0x4002_0000),
    (0xD8, 0x080A_0000),
    (0xE0, 0x0800_0000),
    (0xE8, 0x8000_0018),
    (0xF0, 0x0000_002A),
    (0xF4, 0x50A0_0200),
    (0xF8, 0x4000_1000),
    (0x100, 0x4001_0000),
    (0x1280, 0xD538_CC07),
    (0x1284, 0xAA07_03F8),
    (0x1288, 0xD538_CB7B),
    (0x128C, 0xD518_CC27),
    (0x1290, 0xD280_0039),
    (0x1294, 0xD69F_03E0),
    (0x2400, 0x5800_0080),
    (0x2404, 0xD400_0003),
    (0x2408, 0x1400_0000),
    (0x2410, 0x8400_0008),
    (0x2480, 0xD538_CC08),
    (0x2484, 0xAA08_03F5),
    (0x2488, 0xD53C_CB56),
    (0x248C, 0xD53C_CB77),
    (0x2490, 0xD51C_CC1F),
    (0x2494, 0xD518_CC28),
    (0x2498, 0xD69F_03E0),
];

/// `eoi` in [`VIRTUAL_IRQ`]: the guest's EOI, its virtual interrupt active.
const VIRTUAL_IRQ_EOI: u64 = RAM_BASE + 0x128C;

/// The registers [`VIRTUAL_IRQ`] leaves its findings in.
fn virtual_irq_findings(machine: &VirtMachine<LentStorage<'_>, WithEl2>) -> [u64; 8] {
    [
        Reg::X19,
        Reg::X20,
        Reg::X21,
        Reg::X22,
        Reg::X23,
        Reg::X24,
        Reg::X26,
        Reg::X27,
    ]
    .map(|reg| machine.reg_read(reg))
}

/// A hypervisor injects virtual interrupt 42 through `ICH_LR0_EL2` with
/// `EOI` set; its guest takes it as a virtual IRQ, reads it from
/// `ICV_IAR1_EL1` and the running priority 0xA0 from `ICV_RPR_EL1`, and
/// ends it, which leaves the running priority idle and asserts the EOI
/// maintenance interrupt: the hypervisor takes PPI 25 and finds
/// `ICH_MISR_EL2.EOI` and `ICH_EISR_EL2` bit 0 set. `ICC_SRE_EL2` reads
/// `SRE`, `DFB`, `DIB` and the `Enable` written, `ICH_VTR_EL2` the
/// processor's.
#[test]
fn a_list_register_interrupt_reaches_el1_and_its_eoi_reaches_el2() {
    for cpu in CpuModel::ALL {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(cpu, &mut ram, VIRTUAL_IRQ);
        run_to_power_off(&mut m);
        assert_eq!(
            virtual_irq_findings(&m),
            [0xF, vtr(cpu), 25, 1, 1, 42, 0xFF, 0xA0],
            "{cpu:?}: SRE_EL2, VTR, the maintenance INTID, MISR, EISR, the virtual INTID, \
             the running priority after and during"
        );
    }
}

/// A snapshot taken at the guest's EOI, its virtual interrupt active in
/// `ICH_LR0_EL2` and `ICH_AP1R0_EL2`, restores to the same end: the
/// maintenance interrupt follows, as it does running on.
#[test]
fn a_snapshot_with_a_virtual_interrupt_active_restores_to_the_same_end() {
    for cpu in CpuModel::ALL {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(cpu, &mut ram, VIRTUAL_IRQ);
        m.set_exits(&[VIRTUAL_IRQ_EOI]).expect("one exit");
        let out = m.step(RunBudget::Ticks(GIVE_UP_AFTER));
        assert_eq!(out.stop, StopReason::ExitReached, "{cpu:?}");
        assert_eq!(m.reg_read(Reg::X24), 42, "{cpu:?}: acknowledged");
        m.set_exits(&[]).expect("none");
        let mut image = Vec::new();
        m.save_snapshot(&mut image).expect("save");
        run_to_power_off(&mut m);
        let ran_on = (virtual_irq_findings(&m), m.ticks());
        m.restore_snapshot(&mut image.as_slice())
            .expect("the image restores");
        assert_eq!(m.reg_read(Reg::X21), 0, "{cpu:?}: no maintenance yet");
        run_to_power_off(&mut m);
        assert_eq!((virtual_irq_findings(&m), m.ticks()), ran_on, "{cpu:?}");

        // A machine without EL2 holds no virtual interface to restore.
        let mut plain_ram = vec![0u8; RAM_LEN];
        let mut plain = MachineBuilder::new(config(cpu))
            .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
            .bare()
            .build_in(MemoryLoan {
                ram: &mut plain_ram,
                vars: &mut [],
            })
            .expect("the board builds");
        assert!(plain.restore_snapshot(&mut image.as_slice()).is_err());
    }
}

/// ```text
/// _start: ldr x0, =0x40002000          ; msr vbar_el2, x0
///         mov x2, #0xf                 ; msr icc_sre_el2, x2
///         ldr x2, =0x80000018          ; msr hcr_el2, x2
///         ldr x2, =0x5080000000000028  ; msr ich_lr0_el2, x2   // G1 0x80 40
///         ldr x2, =0x5090000000000029  ; msr ich_lr1_el2, x2   // G1 0x90 41
///         ldr x2, =0x404000000000002B  ; msr ich_lr2_el2, x2   // G0 0x40 43
///         mov x2, #1                   ; msr ich_hcr_el2, x2
///         adr x0, el1                  ; msr elr_el2, x0
///         mov x0, #0x3c5               ; msr spsr_el2, x0
///         eret
/// el1:    mov x2, #0xff                ; msr icc_pmr_el1, x2
///         mov x2, #1                   ; msr icc_igrpen1_el1, x2
///         mrs x10, icc_hppir1_el1
///         mrs x11, icc_iar1_el1        ; mrs x12, icc_rpr_el1
///         mrs x13, icc_iar1_el1        ; mrs x14, icc_hppir1_el1
///         mov x2, #1                   ; msr icc_igrpen0_el1, x2
///         mrs x15, icc_hppir1_el1      ; mrs x16, icc_iar0_el1 ; mrs x17, icc_rpr_el1
///         msr icc_eoir0_el1, x16       ; mrs x18, icc_rpr_el1
///         msr icc_eoir1_el1, x11       ; mrs x19, icc_rpr_el1
///         mov x2, #0x80                ; msr icc_pmr_el1, x2 ; mrs x20, icc_iar1_el1
///         mov x2, #0xff                ; msr icc_pmr_el1, x2 ; mrs x21, icc_iar1_el1
///         mrs x22, icc_ctlr_el1
///         hvc #0
/// 1:      b 1b
///         .ltorg
///         .org 0x2400
///         mrs x23, ich_ap1r0_el2       ; mrs x24, ich_elrsr_el2
///         ldr x0, =0x84000008          ; smc #0
/// 2:      b 2b
/// ```
const PRIORITIES: &[(u64, u32)] = &[
    (0x0, 0x5800_05C0),
    (0x4, 0xD51C_C000),
    (0x8, 0xD280_01E2),
    (0xC, 0xD51C_C9A2),
    (0x10, 0x5800_0582),
    (0x14, 0xD51C_1102),
    (0x18, 0x5800_0582),
    (0x1C, 0xD51C_CC02),
    (0x20, 0x5800_0582),
    (0x24, 0xD51C_CC22),
    (0x28, 0x5800_0582),
    (0x2C, 0xD51C_CC42),
    (0x30, 0xD280_0022),
    (0x34, 0xD51C_CB02),
    (0x38, 0x1000_00A0),
    (0x3C, 0xD51C_4020),
    (0x40, 0xD280_78A0),
    (0x44, 0xD51C_4000),
    (0x48, 0xD69F_03E0),
    (0x4C, 0xD280_1FE2),
    (0x50, 0xD518_4602),
    (0x54, 0xD280_0022),
    (0x58, 0xD518_CCE2),
    (0x5C, 0xD538_CC4A),
    (0x60, 0xD538_CC0B),
    (0x64, 0xD538_CB6C),
    (0x68, 0xD538_CC0D),
    (0x6C, 0xD538_CC4E),
    (0x70, 0xD280_0022),
    (0x74, 0xD518_CCC2),
    (0x78, 0xD538_CC4F),
    (0x7C, 0xD538_C810),
    (0x80, 0xD538_CB71),
    (0x84, 0xD518_C830),
    (0x88, 0xD538_CB72),
    (0x8C, 0xD518_CC2B),
    (0x90, 0xD538_CB73),
    (0x94, 0xD280_1002),
    (0x98, 0xD518_4602),
    (0x9C, 0xD538_CC14),
    (0xA0, 0xD280_1FE2),
    (0xA4, 0xD518_4602),
    (0xA8, 0xD538_CC15),
    (0xAC, 0xD538_CC96),
    (0xB0, 0xD400_0002),
    (0xB4, 0x1400_0000),
    (0xB8, 0x4000_2000),
    (0xC0, 0x8000_0018),
    (0xC8, 0x0000_0028),
    (0xCC, 0x5080_0000),
    (0xD0, 0x0000_0029),
    (0xD4, 0x5090_0000),
    (0xD8, 0x0000_002B),
    (0xDC, 0x4040_0000),
    (0x2400, 0xD53C_C917),
    (0x2404, 0xD53C_CBB8),
    (0x2408, 0x5800_0080),
    (0x240C, 0xD400_0003),
    (0x2410, 0x1400_0000),
    (0x2418, 0x8400_0008),
];

/// Three virtual interrupts in the List registers — Group 1 40 at 0x80 and
/// 41 at 0x90, Group 0 43 at 0x40 — as the guest's virtual CPU interface
/// presents them: with Group 0 disabled, 40 is the highest pending and
/// acknowledged, the running priority 0x80, and 41 cannot preempt it; with
/// Group 0 enabled, 43 is the highest, is acknowledged over the active 40
/// and runs at 0x40; their EOIs drop the running priority to 0x80 and to
/// idle; a priority mask of 0x80 hides 41, at 0x90, which 0xFF shows.
/// `ICV_CTLR_EL1.PRIbits` is the virtual interface's; the hypervisor
/// finds 41's preemption level active in `ICH_AP1R0_EL2` (the Cortex-A57's
/// five bits: bit 18; the Armv9.2 model's seven put it in `ICH_AP1R2_EL2`)
/// and the two ended List registers empty.
#[test]
fn virtual_priorities_mask_preempt_and_end() {
    for cpu in CpuModel::ALL {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(cpu, &mut ram, PRIORITIES);
        run_to_power_off(&mut m);
        let read = |r| m.reg_read(r);
        let got = [
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
        ]
        .map(read);
        assert_eq!(
            got,
            [40, 40, 0x80, 1023, 41, 1023, 43, 0x40, 0x80, 0xFF, 1023, 41],
            "{cpu:?}"
        );
        let (pri_bits, ap1r0) = match cpu {
            CpuModel::CortexA57 => (4, 1 << 18),
            CpuModel::Armv9p2 => (6, 0),
        };
        assert_eq!(read(Reg::X22), pri_bits << 8, "{cpu:?}: ICV_CTLR_EL1");
        assert_eq!(read(Reg::X23), ap1r0, "{cpu:?}: ICH_AP1R0_EL2");
        assert_eq!(
            read(Reg::X24),
            all_list_registers(cpu) & !(1 << 1),
            "{cpu:?}: ICH_ELRSR_EL2"
        );
    }
}

/// On a GICv3 of the board's default five priority bits, the Armv9.2
/// model's virtual interface has five too: its guest's `ICV_CTLR_EL1`
/// reads `PRIbits` 4, so the one active priority register a group it
/// finds there is one the CPU interface has, and 41's preemption level is
/// bit 18 of `ICH_AP1R0_EL2`.
#[test]
fn the_armv9p2_models_virtual_priority_bits_follow_the_cpu_interfaces() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = MachineBuilder::new(VirtConfig {
        gic: GicShape::DEFAULT,
        ..config(CpuModel::Armv9p2)
    })
    .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
    .bare()
    .with_el2()
    .build_in(MemoryLoan {
        ram: &mut ram,
        vars: &mut [],
    })
    .expect("the board builds");
    for &(at, word) in PRIORITIES {
        m.mem_write(RAM_BASE + at, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    run_to_power_off(&mut m);
    assert_eq!(m.reg_read(Reg::X22), 4 << 8, "ICV_CTLR_EL1.PRIbits");
    assert_eq!(m.reg_read(Reg::X23), 1 << 18, "ICH_AP1R0_EL2");
    assert_eq!(m.reg_read(Reg::X21), 41);
}

/// ```text
/// _start: ldr x0, =0x40002000          ; msr vbar_el2, x0
///         ldr x28, =0x40030000                                 // the log
///         ldr x2, =0x80000018          ; msr hcr_el2, x2
///         mov x2, #1                   ; msr ich_hcr_el2, x2
///         adr x0, el1                  ; msr elr_el2, x0
///         mov x0, #0x3c5               ; msr spsr_el2, x0
///         eret
/// el1:    ldr x0, =0x40001000          ; msr vbar_el1, x0
///         mrs x1, icc_sre_el1                                  // Enable 0: traps
///         mov x0, #0xf                 ; hvc #2
///         mrs x11, icc_sre_el1
///         mov x0, #(1 << 12)           ; hvc #1                // TALL1
///         mrs x1, icc_iar1_el1                                 // traps
///         mrs x12, icc_iar0_el1
///         mov x0, #(1 << 11)           ; hvc #1                // TALL0
///         mrs x1, icc_hppir0_el1                               // traps
///         mrs x13, icc_hppir1_el1
///         mov x0, #(1 << 10)           ; hvc #1                // TC
///         mrs x1, icc_pmr_el1                                  // traps
///         msr icc_ctlr_el1, xzr                                // traps
///         mrs x14, icc_bpr1_el1
///         mov x0, #0                   ; hvc #1
///         msr icc_sgi1r_el1, xzr                               // traps under IMO
///         mrs x1, ich_vtr_el2                                  // UNDEFINED
///         msr ich_hcr_el2, xzr                                 // UNDEFINED
///         mrs x1, icc_sre_el2                                  // UNDEFINED
///         mov x0, #(1 << 14)           ; hvc #1                // TDIR
///         msr icc_dir_el1, xzr                                 // traps with TDS
///         mov x0, #0                   ; hvc #1
///         mov x0, #(1 << 39)           ; hvc #3                // HFGRTR_EL2.ICC_IGRPENn_EL1
///         mrs x1, icc_igrpen1_el1                              // traps with FEAT_FGT
///         msr icc_igrpen0_el1, xzr
///         hvc #0
/// 1:      b 1b
///         .ltorg
///         .org 0x1200                                          // EL1, current EL: UNDEFINED
///         mrs x9, esr_el1              ; orr x9, x9, #(1 << 63) ; str x9, [x28], #8
///         mrs x10, elr_el1             ; add x10, x10, #4 ; msr elr_el1, x10
///         eret
///         .org 0x2400                                          // EL2, from EL1
///         mrs x9, esr_el2              ; lsr x10, x9, #26
///         cmp x10, #0x16               ; b.eq 5f
///         str x9, [x28], #8
///         mrs x10, elr_el2             ; add x10, x10, #4 ; msr elr_el2, x10
///         eret
/// 5:      and x10, x9, #0xffff         ; cbz x10, 8f
///         cmp x10, #1                  ; b.ne 6f
///         msr ich_hcr_el2, x0          ; eret
/// 6:      cmp x10, #2                  ; b.ne 7f
///         msr icc_sre_el2, x0          ; eret
/// 7:      mrs x10, id_aa64mmfr0_el1    ; ubfx x10, x10, #56, #4 ; cbz x10, 9f
///         msr s3_4_c1_c1_4, x0                                 // HFGRTR_EL2
/// 9:      eret
/// 8:      ldr x0, =0x84000008          ; smc #0
/// 10:     b 10b
/// ```
const TRAPS: &[(u64, u32)] = &[
    (0x0, 0x5800_0600),
    (0x4, 0xD51C_C000),
    (0x8, 0x5800_061C),
    (0xC, 0x5800_0622),
    (0x10, 0xD51C_1102),
    (0x14, 0xD280_0022),
    (0x18, 0xD51C_CB02),
    (0x1C, 0x1000_00A0),
    (0x20, 0xD51C_4020),
    (0x24, 0xD280_78A0),
    (0x28, 0xD51C_4000),
    (0x2C, 0xD69F_03E0),
    (0x30, 0x5800_0540),
    (0x34, 0xD518_C000),
    (0x38, 0xD538_CCA1),
    (0x3C, 0xD280_01E0),
    (0x40, 0xD400_0042),
    (0x44, 0xD538_CCAB),
    (0x48, 0xD282_0000),
    (0x4C, 0xD400_0022),
    (0x50, 0xD538_CC01),
    (0x54, 0xD538_C80C),
    (0x58, 0xD281_0000),
    (0x5C, 0xD400_0022),
    (0x60, 0xD538_C841),
    (0x64, 0xD538_CC4D),
    (0x68, 0xD280_8000),
    (0x6C, 0xD400_0022),
    (0x70, 0xD538_4601),
    (0x74, 0xD518_CC9F),
    (0x78, 0xD538_CC6E),
    (0x7C, 0xD280_0000),
    (0x80, 0xD400_0022),
    (0x84, 0xD518_CBBF),
    (0x88, 0xD53C_CB21),
    (0x8C, 0xD51C_CB1F),
    (0x90, 0xD53C_C9A1),
    (0x94, 0xD288_0000),
    (0x98, 0xD400_0022),
    (0x9C, 0xD518_CB3F),
    (0xA0, 0xD280_0000),
    (0xA4, 0xD400_0022),
    (0xA8, 0xD2C0_1000),
    (0xAC, 0xD400_0062),
    (0xB0, 0xD538_CCE1),
    (0xB4, 0xD518_CCDF),
    (0xB8, 0xD400_0002),
    (0xBC, 0x1400_0000),
    (0xC0, 0x4000_2000),
    (0xC8, 0x4003_0000),
    (0xD0, 0x8000_0018),
    (0xD8, 0x4000_1000),
    (0x1200, 0xD538_5209),
    (0x1204, 0xB241_0129),
    (0x1208, 0xF800_8789),
    (0x120C, 0xD538_402A),
    (0x1210, 0x9100_114A),
    (0x1214, 0xD518_402A),
    (0x1218, 0xD69F_03E0),
    (0x2400, 0xD53C_5209),
    (0x2404, 0xD35A_FD2A),
    (0x2408, 0xF100_595F),
    (0x240C, 0x5400_00C0),
    (0x2410, 0xF800_8789),
    (0x2414, 0xD53C_402A),
    (0x2418, 0x9100_114A),
    (0x241C, 0xD51C_402A),
    (0x2420, 0xD69F_03E0),
    (0x2424, 0x9240_3D2A),
    (0x2428, 0xB400_01CA),
    (0x242C, 0xF100_055F),
    (0x2430, 0x5400_0061),
    (0x2434, 0xD51C_CB00),
    (0x2438, 0xD69F_03E0),
    (0x243C, 0xF100_095F),
    (0x2440, 0x5400_0061),
    (0x2444, 0xD51C_C9A0),
    (0x2448, 0xD69F_03E0),
    (0x244C, 0xD538_070A),
    (0x2450, 0xD378_ED4A),
    (0x2454, 0xB400_004A),
    (0x2458, 0xD51C_1180),
    (0x245C, 0xD69F_03E0),
    (0x2460, 0x5800_0080),
    (0x2464, 0xD400_0003),
    (0x2468, 0x1400_0000),
    (0x2470, 0x8400_0008),
];

/// The syndrome of an `MRS` (`read`) or `MSR` of `(op0, op1, CRn, CRm,
/// op2)` naming `rt` trapped to EL2: EC 0x18, IL 1, and the ISS of
/// `AArch64_SystemAccessTrapSyndrome`.
const fn sysreg_trap(enc: [u64; 5], rt: u64, read: bool) -> u64 {
    let [op0, op1, crn, crm, op2] = enc;
    0x18 << 26
        | 1 << 25
        | op0 << 20
        | op2 << 17
        | op1 << 14
        | crn << 10
        | rt << 5
        | crm << 1
        | read as u64
}

/// An UNDEFINED instruction's syndrome as [`TRAPS`]' EL1 handler logs it:
/// EC 0, IL 1, marked by bit 63.
const UNDEFINED_AT_EL1: u64 = 1 << 63 | 1 << 25;

/// At EL1 under `HCR_EL2.IMO` and `FMO`, each control traps to EL2 the
/// accesses it names and no other: `ICC_SRE_EL2.Enable` clear,
/// `ICC_SRE_EL1`; `ICH_HCR_EL2.TALL1`, Group 1's registers; `TALL0`,
/// Group 0's; `TC`, the common ones; the SGI registers under `IMO` alone;
/// `TDIR` (the Armv9.2 model's, with `ICH_VTR_EL2.TDS`), `ICC_DIR_EL1`;
/// and `HFGRTR_EL2.ICC_IGRPENn_EL1` (the Armv9.2 model's, with FEAT_FGT)
/// reads of `ICC_IGRPEN1_EL1`. The EL2 registers are UNDEFINED at EL1
/// without `HCR_EL2.NV`. The accesses no control traps reach the virtual
/// CPU interface: `ICC_SRE_EL1` 0b111, no virtual interrupt to acknowledge
/// or pending (1023), `ICV_BPR1_EL1` its least.
#[test]
fn each_el2_control_traps_what_it_traps() {
    const SRE_EL1: [u64; 5] = [3, 0, 12, 12, 5];
    const IAR1: [u64; 5] = [3, 0, 12, 12, 0];
    const HPPIR0: [u64; 5] = [3, 0, 12, 8, 2];
    const PMR: [u64; 5] = [3, 0, 4, 6, 0];
    const CTLR: [u64; 5] = [3, 0, 12, 12, 4];
    const SGI1R: [u64; 5] = [3, 0, 12, 11, 5];
    const DIR: [u64; 5] = [3, 0, 12, 11, 1];
    const IGRPEN1: [u64; 5] = [3, 0, 12, 12, 7];
    for cpu in CpuModel::ALL {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(cpu, &mut ram, TRAPS);
        run_to_power_off(&mut m);
        let mut want = vec![
            sysreg_trap(SRE_EL1, 1, true),
            sysreg_trap(IAR1, 1, true),
            sysreg_trap(HPPIR0, 1, true),
            sysreg_trap(PMR, 1, true),
            sysreg_trap(CTLR, 31, false),
            sysreg_trap(SGI1R, 31, false),
            UNDEFINED_AT_EL1,
            UNDEFINED_AT_EL1,
            UNDEFINED_AT_EL1,
        ];
        let least_bpr1 = match cpu {
            CpuModel::CortexA57 => 3,
            CpuModel::Armv9p2 => {
                want.push(sysreg_trap(DIR, 31, false));
                want.push(sysreg_trap(IGRPEN1, 1, true));
                1
            }
        };
        let logged = (m.reg_read(Reg::X28) - LOG) / 8;
        let got: Vec<u64> = (0..logged).map(|n| read_u64(&m, LOG + 8 * n)).collect();
        assert_eq!(got, want, "{cpu:?}: the syndromes, in order");
        assert_eq!(
            [Reg::X11, Reg::X12, Reg::X13, Reg::X14].map(|r| m.reg_read(r)),
            [0b111, 1023, 1023, least_bpr1],
            "{cpu:?}: ICC_SRE_EL1, ICV_IAR0_EL1, ICV_HPPIR1_EL1, ICV_BPR1_EL1"
        );
    }
}

/// ```text
/// _start: ldr x0, =0x40002000          ; msr vbar_el2, x0
///         ldr x28, =0x40030000                                 // the log
///         ldr x0, =0x40040000          ; msr s3_4_c2_c2_0, x0  // VNCR_EL2
///         mov x2, #0x1234              ; str x2, [x0, #0x4c0]  // the guest's ICH_HCR_EL2
///         mov x2, #1                   ; msr ich_hcr_el2, x2
///         ldr x2, =0x0000240080000000  ; msr hcr_el2, x2       // RW, NV, NV2
///         adr x0, el1                  ; msr elr_el2, x0
///         mov x0, #0x3c5               ; msr spsr_el2, x0
///         eret
/// el1:    ldr x1, =0x50A000000000002A
///         msr ich_lr0_el2, x1                                  // VNCR + 0x400
///         mrs x2, ich_hcr_el2                                  // VNCR + 0x4C0
///         msr ich_vmcr_el2, x1                                 // VNCR + 0x4C8
///         msr ich_ap1r0_el2, x1                                // VNCR + 0x4A0
///         mrs x3, ich_vtr_el2                                  // traps
///         mrs x4, icc_sre_el2                                  // traps
///         ldr x0, =0x0000040080000000  ; hvc #4                // NV alone
///         msr ich_lr1_el2, x1                                  // traps
///         mrs x5, ich_hcr_el2                                  // traps
///         hvc #0
/// 1:      b 1b
///         .ltorg
///         .org 0x2400                                          // EL2, from EL1
///         mrs x9, esr_el2              ; lsr x10, x9, #26
///         cmp x10, #0x16               ; b.eq 5f
///         str x9, [x28], #8
///         mrs x10, elr_el2             ; add x10, x10, #4 ; msr elr_el2, x10
///         eret
/// 5:      and x10, x9, #0xffff         ; cbz x10, 8f
///         msr hcr_el2, x0              ; eret                  // HVC #4
/// 8:      mrs x6, ich_lr0_el2          ; mrs x7, ich_lr1_el2 ; mrs x8, ich_hcr_el2
///         mrs x11, ich_vmcr_el2        ; mrs x12, ich_ap1r0_el2
///         ldr x0, =0x84000008          ; smc #0
/// 10:     b 10b
/// ```
const NESTED: &[(u64, u32)] = &[
    (0x0, 0x5800_03C0),
    (0x4, 0xD51C_C000),
    (0x8, 0x5800_03DC),
    (0xC, 0x5800_03E0),
    (0x10, 0xD51C_2200),
    (0x14, 0xD282_4682),
    (0x18, 0xF902_6002),
    (0x1C, 0xD280_0022),
    (0x20, 0xD51C_CB02),
    (0x24, 0x5800_0362),
    (0x28, 0xD51C_1102),
    (0x2C, 0x1000_00A0),
    (0x30, 0xD51C_4020),
    (0x34, 0xD280_78A0),
    (0x38, 0xD51C_4000),
    (0x3C, 0xD69F_03E0),
    (0x40, 0x5800_02C1),
    (0x44, 0xD51C_CC01),
    (0x48, 0xD53C_CB02),
    (0x4C, 0xD51C_CBE1),
    (0x50, 0xD51C_C901),
    (0x54, 0xD53C_CB23),
    (0x58, 0xD53C_C9A4),
    (0x5C, 0x5800_0220),
    (0x60, 0xD400_0082),
    (0x64, 0xD51C_CC21),
    (0x68, 0xD53C_CB05),
    (0x6C, 0xD400_0002),
    (0x70, 0x1400_0000),
    (0x78, 0x4000_2000),
    (0x80, 0x4003_0000),
    (0x88, 0x4004_0000),
    (0x90, 0x8000_0000),
    (0x94, 0x0000_2400),
    (0x98, 0x0000_002A),
    (0x9C, 0x50A0_0000),
    (0xA0, 0x8000_0000),
    (0xA4, 0x0000_0400),
    (0x2400, 0xD53C_5209),
    (0x2404, 0xD35A_FD2A),
    (0x2408, 0xF100_595F),
    (0x240C, 0x5400_00C0),
    (0x2410, 0xF800_8789),
    (0x2414, 0xD53C_402A),
    (0x2418, 0x9100_114A),
    (0x241C, 0xD51C_402A),
    (0x2420, 0xD69F_03E0),
    (0x2424, 0x9240_3D2A),
    (0x2428, 0xB400_006A),
    (0x242C, 0xD51C_1100),
    (0x2430, 0xD69F_03E0),
    (0x2434, 0xD53C_CC06),
    (0x2438, 0xD53C_CC27),
    (0x243C, 0xD53C_CB08),
    (0x2440, 0xD53C_CBEB),
    (0x2444, 0xD53C_C90C),
    (0x2448, 0x5800_0080),
    (0x244C, 0xD400_0003),
    (0x2450, 0x1400_0000),
    (0x2458, 0x8400_0008),
];

/// On the Armv9.2 model (FEAT_NV2), a guest hypervisor at EL1 under
/// `HCR_EL2.{NV2, NV}` reaches its `ICH_LR0_EL2`, `ICH_HCR_EL2`,
/// `ICH_VMCR_EL2` and `ICH_AP1R0_EL2` as stores to and loads from its
/// `VNCR_EL2` page at 0x400, 0x4C0, 0x4C8 and 0x4A0, the real registers
/// untouched, while `ICH_VTR_EL2` and `ICC_SRE_EL2` trap to EL2; under
/// `NV` alone `ICH_LR1_EL2` and `ICH_HCR_EL2` trap too.
#[test]
fn a_guest_hypervisors_ich_registers_are_its_vncr_page() {
    const VTR: [u64; 5] = [3, 4, 12, 11, 1];
    const SRE_EL2: [u64; 5] = [3, 4, 12, 9, 5];
    const LR1: [u64; 5] = [3, 4, 12, 12, 1];
    const HCR: [u64; 5] = [3, 4, 12, 11, 0];
    let lr = 0x50A0_0000_0000_002A;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, NESTED);
    run_to_power_off(&mut m);
    assert_eq!(read_u64(&m, VNCR_PAGE + 0x400), lr, "ICH_LR0_EL2");
    assert_eq!(m.reg_read(Reg::X2), 0x1234, "ICH_HCR_EL2 from the page");
    assert_eq!(read_u64(&m, VNCR_PAGE + 0x4C8), lr, "ICH_VMCR_EL2");
    assert_eq!(read_u64(&m, VNCR_PAGE + 0x4A0), lr, "ICH_AP1R0_EL2");
    assert_eq!(read_u64(&m, VNCR_PAGE + 0x408), 0, "ICH_LR1_EL2 trapped");
    let logged = (m.reg_read(Reg::X28) - LOG) / 8;
    let got: Vec<u64> = (0..logged).map(|n| read_u64(&m, LOG + 8 * n)).collect();
    assert_eq!(
        got,
        [
            sysreg_trap(VTR, 3, true),
            sysreg_trap(SRE_EL2, 4, true),
            sysreg_trap(LR1, 1, false),
            sysreg_trap(HCR, 5, true),
        ]
    );
    // The hypervisor's own registers: as it left them, the virtual
    // interface's binary points their least out of reset.
    assert_eq!(
        [Reg::X6, Reg::X7, Reg::X8, Reg::X11, Reg::X12].map(|r| m.reg_read(r)),
        [0, 0, 1, 0x0004_0008, 0]
    );
}
