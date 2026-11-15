//! A board with a GICv3's LPIs and ITS, as a guest finds them: the LPI
//! Configuration and Pending tables and the ITS's Device and Collection
//! tables set up in RAM, `MAPD`, `MAPC` and `MAPTI` through the command
//! queue, an event written to `GITS_TRANSLATER` and its LPI taken through
//! `ICC_IAR1_EL1` and ended; `INT`, `CLEAR`, `INV` after a change to the LPI
//! Configuration table, `INVALL`, `MOVI`, `DISCARD`, a command error that
//! stalls the queue until `GITS_CWRITER.Retry`, LPIs disabled at the
//! Redistributor; and a snapshot taken with an LPI pending. Each guest is
//! assembled with GNU as for aarch64 and linked at the base of RAM, where a
//! bare machine starts; its words below are `objcopy`'s, its source above
//! them.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    CpuModel, DeviceLayout, GicVersion, LentStorage, MachineBuilder, MemoryLoan, NoEl2, RamSize,
    Reg, RunBudget, StopReason, VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 1 << 20;
const GIVE_UP_AFTER: u64 = 50_000_000;
/// Where the guests record what they read, a doubleword each, and the flag
/// the snapshot's guest waits on.
const RESULTS: u64 = 0x4007_0000;
const FLAG: u64 = 0x4007_1000;
/// The literal both guests load the ITS's address from (`its_base`), its
/// low word; the high word is zero for every layout.
const ITS_BASE: u64 = 0xFF8;

// Both guests are built on `common.inc`:
//
// ```text
//         .equ GICD, 0x08000000
//         .equ GICR, 0x080A0000
//         .equ PROP, 0x40020000
//         .equ PEND, 0x40030000
//         .equ QUEUE, 0x40040000
//         .equ RESULTS, 0x40070000
//         .equ FLAG, 0x40071000
//         .macro cmd a, b, c, d           // queue a command, write GITS_CWRITER
//         ldr x0, =\a ; ldr x1, =\b ; ldr x2, =\c ; ldr x3, =\d ; bl send
//         .endm
//         .macro rec                      // record x2 at the next result
//         str x2, [x24], #8
//         .endm
//         .macro setup
//         ldr x0, =0x40001000 ; msr vbar_el1, x0 ; ldr x0, =0x40010000 ; mov sp, x0
//         ldr x27, its_base               // the ITS
//         add x23, x27, #0x10, lsl #12    // its translation frame
//         ldr x26, =GICR ; ldr x25, =QUEUE ; ldr x24, =RESULTS
//         mov x19, #0 ; mov x21, #0       // the write pointer, the IRQs taken
//         str wzr, [x26, #0x14]           // GICR_WAKER: awake
// 1:      ldr w2, [x26, #0x14] ; tbnz w2, #2, 1b
//         ldr x4, =GICD ; mov w2, #2 ; str w2, [x4]    // GICD_CTLR.EnableGrp1
//         mov x2, #0xff ; msr icc_pmr_el1, x2 ; mov x2, #1 ; msr icc_igrpen1_el1, x2
//         ldr x5, =PROP
//         mov w2, #0xA3 ; strb w2, [x5, #32]           // LPI 8224 at 0xA0, enabled
//         mov w2, #0x83 ; strb w2, [x5, #33]           // LPI 8225 at 0x80, enabled
//         add x2, x5, #13 ; str x2, [x26, #0x70]       // GICR_PROPBASER, IDbits 13
//         ldr x2, =PEND ; str x2, [x26, #0x78]         // GICR_PENDBASER
//         mov w2, #1 ; str w2, [x26]                   // GICR_CTLR.EnableLPIs
//         ldr x2, =0x8000000040050000 ; str x2, [x27, #0x100]  // GITS_BASER0: Devices
//         ldr x2, =0x8000000040051000 ; str x2, [x27, #0x108]  // GITS_BASER1: Collections
//         ldr x2, =0x8000000040040000 ; str x2, [x27, #0x80]   // GITS_CBASER
//         mov w2, #1 ; str w2, [x27]                   // GITS_CTLR.Enabled
//         cmd 0x08, 4, 0x8000000040060000, 0           // MAPD 0, ITT, Size 4
//         cmd 0x09, 0, 0x8000000000000000, 0           // MAPC 0, processor 0
//         cmd 0x0A, (8224 << 32) | 5, 0, 0             // MAPTI 0, 5, 8224, 0
//         cmd 0x0A, (8225 << 32) | 6, 0, 0             // MAPTI 0, 6, 8225, 0
//         cmd 0x05, 0, 0, 0                            // SYNC processor 0
//         .endm
//         .macro send_routine
// send:   add x9, x25, x19 ; stp x0, x1, [x9] ; stp x2, x3, [x9, #16]
//         add x19, x19, #32 ; dsb sy ; str x19, [x27, #0x88] ; ret
//         .endm
//         .macro power_off
//         ldr x0, =0x84000008 ; hvc #0
// 9:      b 9b
//         .endm
// ```

/// ```text
/// _start: setup
///         ldr w2, [x26] ; rec                          // R0 GICR_CTLR
///         ldr x2, [x26, #8] ; rec                      // R1 GICR_TYPER
///         ldr x4, =GICD ; ldr w2, [x4, #4] ; rec       // R2 GICD_TYPER
///         ldr x2, [x27, #8] ; rec                      // R3 GITS_TYPER
///         ldr x2, [x27, #0x90] ; rec                   // R4 GITS_CREADR
///         msr daifclr, #2
///         mov w2, #5 ; str w2, [x23, #0x40]            // GITS_TRANSLATER: event 5
/// 2:      cmp x21, #1 ; b.lt 2b                        // R5, R6 the handler's
///         cmd 0x03, 6, 0, 0                            // INT 0, 6
/// 3:      cmp x21, #2 ; b.lt 3b                        // R7, R8 the handler's
///         msr daifset, #2
///         cmd 0x03, 5, 0, 0                            // INT 0, 5
///         mrs x2, icc_hppir1_el1 ; rec                 // R9
///         cmd 0x04, 5, 0, 0                            // CLEAR 0, 5
///         mrs x2, icc_hppir1_el1 ; rec                 // R10
///         ldr x5, =PROP ; mov w2, #0xA2 ; strb w2, [x5, #32]   // 8224 disabled in memory
///         cmd 0x03, 5, 0, 0                            // INT 0, 5
///         mrs x2, icc_hppir1_el1 ; rec                 // R11
///         cmd 0x04, 5, 0, 0                            // CLEAR 0, 5
///         cmd 0x0C, 5, 0, 0                            // INV 0, 5
///         cmd 0x03, 5, 0, 0                            // INT 0, 5
///         mrs x2, icc_hppir1_el1 ; rec                 // R12
///         mov w2, #0x63 ; strb w2, [x5, #32]           // 8224 at 0x60, enabled
///         cmd 0x0D, 0, 0, 0                            // INVALL 0
///         mrs x2, icc_hppir1_el1 ; rec                 // R13
///         mrs x7, icc_iar1_el1 ; mov x2, x7 ; rec      // R14
///         mrs x2, icc_rpr_el1 ; rec                    // R15
///         msr icc_eoir1_el1, x7
///         cmd 0x09, 0, 0x8000000000000001, 0           // MAPC 1, processor 0
///         cmd 0x01, 5, 1, 0                            // MOVI 0, 5, 1
///         cmd 0x09, 0, 0, 0                            // MAPC 0, unmapped
///         cmd 0x03, 5, 0, 0                            // INT 0, 5
///         mrs x2, icc_hppir1_el1 ; rec                 // R16
///         mrs x7, icc_iar1_el1 ; mov x2, x7 ; rec      // R17
///         msr icc_eoir1_el1, x7
///         cmd 0x0F, 5, 0, 0                            // DISCARD 0, 5
///         mov w2, #5 ; str w2, [x23, #0x40]            // GITS_TRANSLATER: event 5
///         mrs x2, icc_hppir1_el1 ; rec                 // R18
///         cmd 0x03, 5, 0, 0                            // INT 0, 5: unmapped
///         ldr x2, [x27, #0x90] ; rec                   // R19 GITS_CREADR
///         cmd 0x05, 0, 0, 0                            // SYNC, behind it
///         ldr x2, [x27, #0x90] ; rec                   // R20 GITS_CREADR
///         sub x9, x19, #64 ; mov x2, #0x05 ; str x2, [x25, x9]  // the INT made a SYNC
///         orr x2, x19, #1 ; str x2, [x27, #0x88]       // GITS_CWRITER.Retry
///         ldr x2, [x27, #0x90] ; rec                   // R21 GITS_CREADR
///         cmd 0x09, 0, 0x8000000000000000, 0           // MAPC 0, processor 0
///         str wzr, [x26]                               // EnableLPIs 0
///         ldr w2, [x26] ; rec                          // R22 GICR_CTLR
///         mov w2, #6 ; str w2, [x23, #0x40]            // GITS_TRANSLATER: event 6
///         mov w2, #1 ; str w2, [x26]                   // EnableLPIs 1
///         mrs x2, icc_hppir1_el1 ; rec                 // R23
///         mov w2, #6 ; str w2, [x23, #0x40]            // GITS_TRANSLATER: event 6
///         mrs x2, icc_hppir1_el1 ; rec                 // R24
///         power_off
///         send_routine
///         .ltorg
///         .org 0xFF8
/// its_base: .quad 0x080C0000
///         .org 0x1280                                  // IRQ, current EL with SP_ELx
///         mrs x7, icc_iar1_el1 ; str x7, [x24], #8
///         mrs x8, icc_rpr_el1 ; str x8, [x24], #8
///         msr icc_eoir1_el1, x7 ; add x21, x21, #1 ; eret
/// ```
const LPIS: &[(u64, u32)] = &[
    (0x0, 0x5800_1CC0),
    (0x4, 0xD518_C000),
    (0x8, 0x5800_1CC0),
    (0xC, 0x9100_001F),
    (0x10, 0x5800_7F5B),
    (0x14, 0x9140_4377),
    (0x18, 0x5800_1C9A),
    (0x1C, 0x5800_1CB9),
    (0x20, 0x5800_1CD8),
    (0x24, 0xD280_0013),
    (0x28, 0xD280_0015),
    (0x2C, 0xB900_175F),
    (0x30, 0xB940_1742),
    (0x34, 0x3717_FFE2),
    (0x38, 0x5800_1C44),
    (0x3C, 0x5280_0042),
    (0x40, 0xB900_0082),
    (0x44, 0xD280_1FE2),
    (0x48, 0xD518_4602),
    (0x4C, 0xD280_0022),
    (0x50, 0xD518_CCE2),
    (0x54, 0x5800_1BA5),
    (0x58, 0x5280_1462),
    (0x5C, 0x3900_80A2),
    (0x60, 0x5280_1062),
    (0x64, 0x3900_84A2),
    (0x68, 0x9100_34A2),
    (0x6C, 0xF900_3B42),
    (0x70, 0x5800_1B02),
    (0x74, 0xF900_3F42),
    (0x78, 0x5280_0022),
    (0x7C, 0xB900_0342),
    (0x80, 0x5800_1AC2),
    (0x84, 0xF900_8362),
    (0x88, 0x5800_1AC2),
    (0x8C, 0xF900_8762),
    (0x90, 0x5800_1AC2),
    (0x94, 0xF900_4362),
    (0x98, 0x5280_0022),
    (0x9C, 0xB900_0362),
    (0xA0, 0x5800_1A80),
    (0xA4, 0x5800_1AA1),
    (0xA8, 0x5800_1AC2),
    (0xAC, 0x5800_1AE3),
    (0xB0, 0x9400_00B2),
    (0xB4, 0x5800_1AE0),
    (0xB8, 0x5800_1A81),
    (0xBC, 0x5800_1AE2),
    (0xC0, 0x5800_1A43),
    (0xC4, 0x9400_00AD),
    (0xC8, 0x5800_1AC0),
    (0xCC, 0x5800_1AE1),
    (0xD0, 0x5800_19C2),
    (0xD4, 0x5800_19A3),
    (0xD8, 0x9400_00A8),
    (0xDC, 0x5800_1A20),
    (0xE0, 0x5800_1A81),
    (0xE4, 0x5800_1922),
    (0xE8, 0x5800_1903),
    (0xEC, 0x9400_00A3),
    (0xF0, 0x5800_1A40),
    (0xF4, 0x5800_18A1),
    (0xF8, 0x5800_1882),
    (0xFC, 0x5800_1863),
    (0x100, 0x9400_009E),
    (0x104, 0xB940_0342),
    (0x108, 0xF800_8702),
    (0x10C, 0xF940_0742),
    (0x110, 0xF800_8702),
    (0x114, 0x5800_1564),
    (0x118, 0xB940_0482),
    (0x11C, 0xF800_8702),
    (0x120, 0xF940_0762),
    (0x124, 0xF800_8702),
    (0x128, 0xF940_4B62),
    (0x12C, 0xF800_8702),
    (0x130, 0xD503_42FF),
    (0x134, 0x5280_00A2),
    (0x138, 0xB900_42E2),
    (0x13C, 0xF100_06BF),
    (0x140, 0x54FF_FFEB),
    (0x144, 0x5800_17E0),
    (0x148, 0x5800_1801),
    (0x14C, 0x5800_15E2),
    (0x150, 0x5800_15C3),
    (0x154, 0x9400_0089),
    (0x158, 0xF100_0ABF),
    (0x15C, 0x54FF_FFEB),
    (0x160, 0xD503_42DF),
    (0x164, 0x5800_16E0),
    (0x168, 0x5800_1681),
    (0x16C, 0x5800_14E2),
    (0x170, 0x5800_14C3),
    (0x174, 0x9400_0081),
    (0x178, 0xD538_CC42),
    (0x17C, 0xF800_8702),
    (0x180, 0x5800_13C0),
    (0x184, 0x5800_15A1),
    (0x188, 0x5800_1402),
    (0x18C, 0x5800_13E3),
    (0x190, 0x9400_007A),
    (0x194, 0xD538_CC42),
    (0x198, 0xF800_8702),
    (0x19C, 0x5800_1165),
    (0x1A0, 0x5280_1442),
    (0x1A4, 0x3900_80A2),
    (0x1A8, 0x5800_14C0),
    (0x1AC, 0x5800_1461),
    (0x1B0, 0x5800_12C2),
    (0x1B4, 0x5800_12A3),
    (0x1B8, 0x9400_0070),
    (0x1BC, 0xD538_CC42),
    (0x1C0, 0xF800_8702),
    (0x1C4, 0x5800_11A0),
    (0x1C8, 0x5800_1381),
    (0x1CC, 0x5800_11E2),
    (0x1D0, 0x5800_11C3),
    (0x1D4, 0x9400_0069),
    (0x1D8, 0x5800_13C0),
    (0x1DC, 0x5800_12E1),
    (0x1E0, 0x5800_1142),
    (0x1E4, 0x5800_1123),
    (0x1E8, 0x9400_0064),
    (0x1EC, 0x5800_12A0),
    (0x1F0, 0x5800_1241),
    (0x1F4, 0x5800_10A2),
    (0x1F8, 0x5800_1083),
    (0x1FC, 0x9400_005F),
    (0x200, 0xD538_CC42),
    (0x204, 0xF800_8702),
    (0x208, 0x5280_0C62),
    (0x20C, 0x3900_80A2),
    (0x210, 0x5800_1240),
    (0x214, 0x5800_0FA1),
    (0x218, 0x5800_0F82),
    (0x21C, 0x5800_0F63),
    (0x220, 0x9400_0056),
    (0x224, 0xD538_CC42),
    (0x228, 0xF800_8702),
    (0x22C, 0xD538_CC07),
    (0x230, 0xAA07_03E2),
    (0x234, 0xF800_8702),
    (0x238, 0xD538_CB62),
    (0x23C, 0xF800_8702),
    (0x240, 0xD518_CC27),
    (0x244, 0x5800_0E60),
    (0x248, 0x5800_0E01),
    (0x24C, 0x5800_10A2),
    (0x250, 0x5800_0DC3),
    (0x254, 0x9400_0049),
    (0x258, 0x5800_1080),
    (0x25C, 0x5800_0EE1),
    (0x260, 0x5800_1042),
    (0x264, 0x5800_0D23),
    (0x268, 0x9400_0044),
    (0x26C, 0x5800_0D20),
    (0x270, 0x5800_0CC1),
    (0x274, 0x5800_0CA2),
    (0x278, 0x5800_0C83),
    (0x27C, 0x9400_003F),
    (0x280, 0x5800_0E00),
    (0x284, 0x5800_0DA1),
    (0x288, 0x5800_0C02),
    (0x28C, 0x5800_0BE3),
    (0x290, 0x9400_003A),
    (0x294, 0xD538_CC42),
    (0x298, 0xF800_8702),
    (0x29C, 0xD538_CC07),
    (0x2A0, 0xAA07_03E2),
    (0x2A4, 0xF800_8702),
    (0x2A8, 0xD518_CC27),
    (0x2AC, 0x5800_0E20),
    (0x2B0, 0x5800_0C41),
    (0x2B4, 0x5800_0AA2),
    (0x2B8, 0x5800_0A83),
    (0x2BC, 0x9400_002F),
    (0x2C0, 0x5280_00A2),
    (0x2C4, 0xB900_42E2),
    (0x2C8, 0xD538_CC42),
    (0x2CC, 0xF800_8702),
    (0x2D0, 0x5800_0B80),
    (0x2D4, 0x5800_0B21),
    (0x2D8, 0x5800_0982),
    (0x2DC, 0x5800_0963),
    (0x2E0, 0x9400_0026),
    (0x2E4, 0xF940_4B62),
    (0x2E8, 0xF800_8702),
    (0x2EC, 0x5800_0A60),
    (0x2F0, 0x5800_08C1),
    (0x2F4, 0x5800_08A2),
    (0x2F8, 0x5800_0883),
    (0x2FC, 0x9400_001F),
    (0x300, 0xF940_4B62),
    (0x304, 0xF800_8702),
    (0x308, 0xD101_0269),
    (0x30C, 0xD280_00A2),
    (0x310, 0xF829_6B22),
    (0x314, 0xB240_0262),
    (0x318, 0xF900_4762),
    (0x31C, 0xF940_4B62),
    (0x320, 0xF800_8702),
    (0x324, 0x5800_0760),
    (0x328, 0x5800_0701),
    (0x32C, 0x5800_0762),
    (0x330, 0x5800_06C3),
    (0x334, 0x9400_0011),
    (0x338, 0xB900_035F),
    (0x33C, 0xB940_0342),
    (0x340, 0xF800_8702),
    (0x344, 0x5280_00C2),
    (0x348, 0xB900_42E2),
    (0x34C, 0x5280_0022),
    (0x350, 0xB900_0342),
    (0x354, 0xD538_CC42),
    (0x358, 0xF800_8702),
    (0x35C, 0x5280_00C2),
    (0x360, 0xB900_42E2),
    (0x364, 0xD538_CC42),
    (0x368, 0xF800_8702),
    (0x36C, 0x5800_0860),
    (0x370, 0xD400_0002),
    (0x374, 0x1400_0000),
    (0x378, 0x8B13_0329),
    (0x37C, 0xA900_0520),
    (0x380, 0xA901_0D22),
    (0x384, 0x9100_8273),
    (0x388, 0xD503_3F9F),
    (0x38C, 0xF900_4773),
    (0x390, 0xD65F_03C0),
    (0x398, 0x4000_1000),
    (0x3A0, 0x4001_0000),
    (0x3A8, 0x080A_0000),
    (0x3B0, 0x4004_0000),
    (0x3B8, 0x4007_0000),
    (0x3C0, 0x0800_0000),
    (0x3C8, 0x4002_0000),
    (0x3D0, 0x4003_0000),
    (0x3D8, 0x4005_0000),
    (0x3DC, 0x8000_0000),
    (0x3E0, 0x4005_1000),
    (0x3E4, 0x8000_0000),
    (0x3E8, 0x4004_0000),
    (0x3EC, 0x8000_0000),
    (0x3F0, 0x0000_0008),
    (0x3F8, 0x0000_0004),
    (0x400, 0x4006_0000),
    (0x404, 0x8000_0000),
    (0x410, 0x0000_0009),
    (0x41C, 0x8000_0000),
    (0x420, 0x0000_000A),
    (0x428, 0x0000_0005),
    (0x42C, 0x0000_2020),
    (0x430, 0x0000_0006),
    (0x434, 0x0000_2021),
    (0x438, 0x0000_0005),
    (0x440, 0x0000_0003),
    (0x448, 0x0000_0006),
    (0x450, 0x0000_000C),
    (0x458, 0x0000_000D),
    (0x460, 0x0000_0001),
    (0x464, 0x8000_0000),
    (0x468, 0x0000_0001),
    (0x470, 0x0000_000F),
    (0x478, 0x8400_0008),
    (0xFF8, 0x080C_0000),
    (0x1280, 0xD538_CC07),
    (0x1284, 0xF800_8707),
    (0x1288, 0xD538_CB68),
    (0x128C, 0xF800_8708),
    (0x1290, 0xD518_CC27),
    (0x1294, 0x9100_06B5),
    (0x1298, 0xD69F_03E0),
];

/// ```text
/// _start: setup
///         cmd 0x03, 5, 0, 0                            // INT 0, 5, IRQs masked
///         ldr x28, =FLAG
/// 1:      ldr x2, [x28] ; cbz x2, 1b                   // until the host sets the flag
///         msr daifclr, #2
/// 2:      cbz x21, 2b
///         power_off
///         send_routine
///         .ltorg
///         .org 0xFF8
/// its_base: .quad 0x080C0000
///         .org 0x1280                                  // IRQ, current EL with SP_ELx
///         mrs x20, icc_iar1_el1 ; msr icc_eoir1_el1, x20 ; mov x21, #1 ; eret
/// ```
const PENDING: &[(u64, u32)] = &[
    (0x0, 0x5800_0AC0),
    (0x4, 0xD518_C000),
    (0x8, 0x5800_0AC0),
    (0xC, 0x9100_001F),
    (0x10, 0x5800_7F5B),
    (0x14, 0x9140_4377),
    (0x18, 0x5800_0A9A),
    (0x1C, 0x5800_0AB9),
    (0x20, 0x5800_0AD8),
    (0x24, 0xD280_0013),
    (0x28, 0xD280_0015),
    (0x2C, 0xB900_175F),
    (0x30, 0xB940_1742),
    (0x34, 0x3717_FFE2),
    (0x38, 0x5800_0A44),
    (0x3C, 0x5280_0042),
    (0x40, 0xB900_0082),
    (0x44, 0xD280_1FE2),
    (0x48, 0xD518_4602),
    (0x4C, 0xD280_0022),
    (0x50, 0xD518_CCE2),
    (0x54, 0x5800_09A5),
    (0x58, 0x5280_1462),
    (0x5C, 0x3900_80A2),
    (0x60, 0x5280_1062),
    (0x64, 0x3900_84A2),
    (0x68, 0x9100_34A2),
    (0x6C, 0xF900_3B42),
    (0x70, 0x5800_0902),
    (0x74, 0xF900_3F42),
    (0x78, 0x5280_0022),
    (0x7C, 0xB900_0342),
    (0x80, 0x5800_08C2),
    (0x84, 0xF900_8362),
    (0x88, 0x5800_08C2),
    (0x8C, 0xF900_8762),
    (0x90, 0x5800_08C2),
    (0x94, 0xF900_4362),
    (0x98, 0x5280_0022),
    (0x9C, 0xB900_0362),
    (0xA0, 0x5800_0880),
    (0xA4, 0x5800_08A1),
    (0xA8, 0x5800_08C2),
    (0xAC, 0x5800_08E3),
    (0xB0, 0x9400_0022),
    (0xB4, 0x5800_08E0),
    (0xB8, 0x5800_0881),
    (0xBC, 0x5800_08E2),
    (0xC0, 0x5800_0843),
    (0xC4, 0x9400_001D),
    (0xC8, 0x5800_08C0),
    (0xCC, 0x5800_08E1),
    (0xD0, 0x5800_07C2),
    (0xD4, 0x5800_07A3),
    (0xD8, 0x9400_0018),
    (0xDC, 0x5800_0820),
    (0xE0, 0x5800_0881),
    (0xE4, 0x5800_0722),
    (0xE8, 0x5800_0703),
    (0xEC, 0x9400_0013),
    (0xF0, 0x5800_0840),
    (0xF4, 0x5800_06A1),
    (0xF8, 0x5800_0682),
    (0xFC, 0x5800_0663),
    (0x100, 0x9400_000E),
    (0x104, 0x5800_07E0),
    (0x108, 0x5800_0781),
    (0x10C, 0x5800_05E2),
    (0x110, 0x5800_05C3),
    (0x114, 0x9400_0009),
    (0x118, 0x5800_079C),
    (0x11C, 0xF940_0382),
    (0x120, 0xB4FF_FFE2),
    (0x124, 0xD503_42FF),
    (0x128, 0xB400_0015),
    (0x12C, 0x5800_0720),
    (0x130, 0xD400_0002),
    (0x134, 0x1400_0000),
    (0x138, 0x8B13_0329),
    (0x13C, 0xA900_0520),
    (0x140, 0xA901_0D22),
    (0x144, 0x9100_8273),
    (0x148, 0xD503_3F9F),
    (0x14C, 0xF900_4773),
    (0x150, 0xD65F_03C0),
    (0x158, 0x4000_1000),
    (0x160, 0x4001_0000),
    (0x168, 0x080A_0000),
    (0x170, 0x4004_0000),
    (0x178, 0x4007_0000),
    (0x180, 0x0800_0000),
    (0x188, 0x4002_0000),
    (0x190, 0x4003_0000),
    (0x198, 0x4005_0000),
    (0x19C, 0x8000_0000),
    (0x1A0, 0x4005_1000),
    (0x1A4, 0x8000_0000),
    (0x1A8, 0x4004_0000),
    (0x1AC, 0x8000_0000),
    (0x1B0, 0x0000_0008),
    (0x1B8, 0x0000_0004),
    (0x1C0, 0x4006_0000),
    (0x1C4, 0x8000_0000),
    (0x1D0, 0x0000_0009),
    (0x1DC, 0x8000_0000),
    (0x1E0, 0x0000_000A),
    (0x1E8, 0x0000_0005),
    (0x1EC, 0x0000_2020),
    (0x1F0, 0x0000_0006),
    (0x1F4, 0x0000_2021),
    (0x1F8, 0x0000_0005),
    (0x200, 0x0000_0003),
    (0x208, 0x4007_1000),
    (0x210, 0x8400_0008),
    (0xFF8, 0x080C_0000),
    (0x1280, 0xD538_CC14),
    (0x1284, 0xD518_CC34),
    (0x1288, 0xD280_0035),
    (0x128C, 0xD69F_03E0),
];

/// What the main guest records, R0 to R24.
const EXPECTED: [u64; 25] = [
    0b11,        // R0 GICR_CTLR: EnableLPIs, CES
    0x11,        // R1 GICR_TYPER: Last, PLPIS, processor 0
    0x006A_000F, // R2 GICD_TYPER: 14 INTID bits, LPIS, 480 SPIs
    0x1_EF71,    // R3 GITS_TYPER: Physical, 8-byte ITEs, 16 EventID and DeviceID bits
    0xA0,        // R4 GITS_CREADR: the five commands run
    8224,        // R5 the translated event's LPI, taken as an IRQ
    0xA0,        // R6 at its priority
    8225,        // R7 INT's LPI, taken as an IRQ
    0x80,        // R8 at its priority
    8224,        // R9 INT: pending
    1023,        // R10 CLEAR: not
    8224,        // R11 disabled in memory, enabled in the cache
    1023,        // R12 disabled once INV brought it in
    8224,        // R13 enabled again by INVALL
    8224,        // R14 acknowledged
    0x60,        // R15 at the priority INVALL brought in
    8224,        // R16 in collection 1 after MOVI, collection 0 unmapped
    8224,        // R17 acknowledged
    1023,        // R18 DISCARD: the event no longer translates
    0x241,       // R19 INT of the discarded event: stalled at command 18
    0x241,       // R20 the SYNC behind it waits
    0x280,       // R21 retried: run to the write pointer
    0b10,        // R22 GICR_CTLR: EnableLPIs cleared
    1023,        // R23 the translation made while LPIs were disabled was lost
    8225,        // R24 one made once they are enabled again is not (collection 0
                 // mapped again first)
];

fn config(cpu: CpuModel, layout: DeviceLayout) -> VirtConfig {
    VirtConfig {
        cpu,
        gic_version: GicVersion::V3,
        layout,
        interrupts: rusty_box_aarch64::InterruptMap::own(layout),
        ..VirtConfig::DEFAULT
    }
}

/// A bare machine of `config` in `ram` with `program` at the base of RAM,
/// its `its_base` the layout's ITS.
fn machine<'r>(
    ram: &'r mut [u8],
    config: VirtConfig,
    program: &[(u64, u32)],
) -> VirtMachine<LentStorage<'r>, NoEl2> {
    let its = config.layout.windows().gic_v3.its.get();
    let mut machine = MachineBuilder::new(config)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("the board builds");
    for &(at, word) in program {
        let word = if at == ITS_BASE {
            u32::try_from(its).expect("an ITS below 4 GiB")
        } else {
            word
        };
        machine
            .mem_write(RAM_BASE + at, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    machine
}

/// On a GICv3 board, on either processor and either layout, the guest sets
/// up its LPIs and the ITS, translates an event and takes its LPI, and
/// finds each command's effect as the specification gives it (`EXPECTED`).
#[test]
fn lpis_come_through_the_its() {
    for cpu in CpuModel::ALL {
        for layout in DeviceLayout::ALL {
            let mut ram = vec![0u8; RAM_LEN];
            let mut m = machine(&mut ram, config(cpu, layout), LPIS);
            let out = m.step(RunBudget::Ticks(GIVE_UP_AFTER));
            assert_eq!(
                out.stop,
                StopReason::PoweredOff,
                "{cpu} {layout}: the guest powers off; it stopped at {:#x}",
                m.reg_read(Reg::Pc)
            );
            let mut bytes = [0u8; 8 * 25];
            m.mem_read(RESULTS, &mut bytes).expect("in RAM");
            let got: Vec<u64> = bytes
                .chunks_exact(8)
                .map(|d| u64::from_le_bytes(d.try_into().expect("eight bytes")))
                .collect();
            for (n, (got, expected)) in got.iter().zip(EXPECTED).enumerate() {
                assert_eq!(*got, expected, "{cpu} {layout}: R{n}");
            }
        }
    }
}

/// A snapshot taken while an LPI is pending, with IRQs masked, restores it
/// pending: the guest, let on, takes it as it would have.
#[test]
fn a_snapshot_keeps_a_pending_lpi() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(
        &mut ram,
        config(CpuModel::CortexA57, DeviceLayout::Board),
        PENDING,
    );
    let waiting = m.step(RunBudget::Ticks(200_000));
    assert_eq!(
        waiting.stop,
        StopReason::BudgetExhausted,
        "waiting on the flag"
    );
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("save");
    let finish = |m: &mut VirtMachine<LentStorage<'_>, NoEl2>| {
        m.mem_write(FLAG, &1u64.to_le_bytes()).expect("in RAM");
        let out = m.step(RunBudget::Ticks(GIVE_UP_AFTER));
        (out.stop, m.reg_read(Reg::X20))
    };
    assert_eq!(finish(&mut m), (StopReason::PoweredOff, 8224));
    m.restore_snapshot(&mut image.as_slice())
        .expect("the image restores");
    assert_eq!(m.reg_read(Reg::X21), 0, "before the LPI was taken");
    assert_eq!(finish(&mut m), (StopReason::PoweredOff, 8224));
}
