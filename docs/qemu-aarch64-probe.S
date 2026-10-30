// Bare-metal EL1 probe for the QEMU AArch64 bugs in docs/qemu-upstream-bugs.md
// that need system mode. Each case prints one line on the PL011.
//
//   aarch64-linux-gnu-gcc -march=armv8.7-a+pauth -nostdlib -static \
//       -Wl,-Ttext=0x40100000 -o qemu-aarch64-probe qemu-aarch64-probe.S
//   qemu-system-aarch64 -M virt -cpu max,pauth-qarma5=on -m 256M \
//       -display none -monitor none -nic none -serial stdio \
//       -kernel qemu-aarch64-probe
//
// (QEMU before 10.0 has no pauth-qarma5 property; QARMA5 is its default
// there: drop ",pauth-qarma5=on".)
//
// A  An illegal exception return (SPSR_EL1.M = EL2h, which a processor
//    without EL2 cannot enter) with SPSR_EL1.PAN = 1 while PSTATE.PAN = 0.
//    Arm's SetPSTATEFromPSR restores PAN on an illegal return too, so the
//    Illegal Execution State exception that follows saves PAN = 1 in
//    SPSR_EL1 (bit 22). Prints ESR_EL1 and SPSR_EL1.
//      Arm:          A 000000003a000000 00000000005003c5
//      QEMU 11.1.2:  A 000000003a000000 00000000001003c5   (PAN 0)
// B  LDADD of 8 bytes at an address that crosses a 16-byte boundary: an
//    alignment fault. Arm's AlignmentFault starts from NoFault, whose
//    `write` is `!accdesc.read && accdesc.write`, false for an atomic
//    (CreateAccDescAtomicOp sets both): ESR_EL1.WnR (bit 6) = 0.
//      Arm:          B 0000000096000021 ...
//      QEMU 11.1.2:  B 0000000096000061 ...                   (WnR 1)
// C  PACDA of 0x7f80000000001000 with TCR_EL1.T0SZ = 25, T1SZ = 20 and no
//    TBI. Bits 63 and 55 differ; with no TBI in either half, InsertPAC's
//    selbit is bit 63, so the code's bottom bit is 64 - T0SZ = 39: bits
//    54:39 hold the code. QEMU takes T1SZ from bit 55's half: 54:44.
//      Arm (QARMA5): C 3106208000001000
//      QEMU 11.1.2:  C 3106200000001000                       (43:39 zero)
// D  The PL011 in loopback (UARTCR.LBE) with its FIFOs on: 17 characters
//    written to UARTDR loop into the 16-entry receive FIFO. The 17th finds
//    it full and is lost: reading UARTDR until UARTFR.RXFE returns 'a' to
//    'p'. (QEMU echoes the 17 transmitted characters on the console before
//    the line.)
//      PL011:        D abcdefghijklmnop
//      QEMU 11.1.2:  D qbcdefghijklmnopq   (the 17th overwrote the 1st,
//                                           and 17 characters come back)
//      QEMU 8.2.2:   D                     (no loopback)
//
// E to I run with the MMU on, over a level-1 table of 1 GiB blocks: VA 0
// the UART's Device block, VA 1 GiB RAM (EL1's), VA 2 GiB nothing, VA 3 GiB
// RAM again with EL0 read/write, VA 4 GiB RAM again read-only. A case that
// faults prints ESR_EL1 and FAR_EL1, one that does not prints "none".
// Arm's AArch64_DC and AArch64_IC translate the address of every DC and IC
// by address and abort on a fault, with ISS.CM and WnR set
// (AArch64_FaultSyndrome) and FAR the register's value; DC IVAC needs
// write permission, a clean from EL1 none (AArch64_S1CheckPermissions),
// and PAN takes no part (CreateAccDescDC).
// E  DC CIVAC of 0x80000123 (unmapped).
//      Arm:          E 0000000096000145 0000000080000123
//      QEMU 11.1.2:  E none                                   (a NOP)
// F  DC CVAP of 0x80000123 (unmapped).
//      Arm:          F 0000000096000145 0000000080000123
//      QEMU 11.1.2:  F 0000000096000005 0000000080000100      (a load:
//                    no CM, no WnR, FAR rounded down to the line)
// G  DC CVAP of 0xc0000100 (EL0 read/write) at EL1 with PSTATE.PAN = 1.
//      Arm:          G none
//      QEMU 11.1.2:  G 000000009600000d 00000000c0000100      (PAN applied)
// H  IC IVAU of 0x80000123 (unmapped).
//      Arm:          H 0000000096000145 0000000080000123
//      QEMU 11.1.2:  H none                                   (a NOP)
// I  DC IVAC of 0x100000100 (read-only).
//      Arm:          I 000000009600014d 0000000100000100
//      QEMU 11.1.2:  I none                                   (a NOP)
//
// J to L run at EL0 (in the VA 1 GiB block, which EL0 may execute) with
// SCTLR_EL1.nTWE clear, so a WFE or WFET that would wait traps to EL1. Each
// ends in SVC, and prints ESR_EL1 and ELR_EL1 of the exception that ended
// it: the SVC (EC 0x15) when nothing trapped, the wait (EC 0x01) when it
// did. Arm's Hint_WFE and Hint_WFET complete without the trap check when
// the Event Register is set, and Hint_WFET when the timeout has passed;
// nothing else does. Of the three, the event stream is on for case K
// alone, so that none of its events reaches J's or L's wait.
// J  WFE first thing after the ERET to EL0: AArch64_ExceptionReturn calls
//    SendEventLocal, so the Event Register is set.
//      Arm:          J 0000000056000000 ...                   (the SVC)
//      QEMU 11.1.2:  J 0000000007e00001 ...                   (WFE trapped)
// K  SEVL and WFE (an event, consumed), a delay with the event stream on
//    (CNTKCTL_EL1.EVNTEN, EVNTI = 0, EVNTDIR = 0: every rise of the count's
//    bit 0), then WFE: TestEventCNTV set the Event Register during the
//    delay.
//      Arm:          K 0000000056000000 ...
//      QEMU 11.1.2:  K 0000000007e00001 ...                   (WFE trapped)
// L  SEVL and WFE, then WFET with a timeout far ahead while the virtual
//    timer's interrupt is pending (masked by PSTATE.I): Hint_WFET does not
//    look at interrupts before its trap check.
//      Arm:          L 0000000007e00027 ...                   (WFET trapped)
//      QEMU 11.1.2:  L 0000000056000000 ...                   (no trap)
//
// M  Back at EL1: MRS of DBGAUTHSTATUS_EL1, which Arm's data gives every
//    AArch64 processor (its accessor reads it at EL1). Prints its value,
//    which UpdateDbgAuthStatus sets for -cpu max (FEAT_Debugv8p4, no EL3)
//    with no external debugger to raise DBGEN: NSID 0b10, NSNID 0b11.
//      Arm:          M 000000000000000e
//      QEMU 11.1.2:  M 0000000002000000 ...                   (UNDEFINED)
//
// N and O run at EL1 with no interrupt pending and print what a wait that
// lasts until its wake-up event gives, which is what QEMU's wake-up timer
// means to give. Arm's WaitForEvent lets a PE resume whenever it chooses,
// so a shorter wait breaks no rule of Arm's.
// N  The event stream on bit 23 of the count (CNTKCTL_EL1.EVNTI = 15 with
//    EVNTIS): a WFE that waits for the stream, then eight rounds of SEVL,
//    WFE (an event, consumed) and WFE. Prints how many rounds took at
//    least half a stream period (2^23 counts): each WFE that waits, waits
//    for the stream's next event. QEMU's timer is set for a count already
//    past, so no WFE waits.
//      Waiting:      N 0000000000000008
//      QEMU 11.1.2:  N 0000000000000000
// O  WFIT with a timeout 2^24 counts ahead. Prints how many whole timeouts
//    the count moved by. QEMU's timer counts in nanoseconds, so with a
//    counter that does not run at 1 GHz (-M virt-9.0 and older run it at
//    62.5 MHz) the wait ends early, and so do N's.
//      Waiting:                    O 0000000000000001
//      QEMU 11.1.2:                O 0000000000000001
//      QEMU 11.1.2, -M virt-9.0:   O 0000000000000000
//
// P  MRS of ID_AA64MMFR0_EL1 on this processor without EL2 (-M virt).
//    Prints it. Arm's Features.json gives FEAT_ECV_POFF, ECV (bits 63:60)
//    2 or more, only to a processor with EL2 ((FEAT_AA64 && FEAT_ECV_POFF)
//    ==> FEAT_AA64EL2), so ECV is at most 1.
//      Arm:          P 1...............                       (ECV 1)
//      QEMU 11.1.2:  P 2100032310201126                       (ECV 2)

#define UART 0x09000000

    .text
    .global _start
_start:
    ldr x0, =stack_top
    mov sp, x0
    ldr x0, =vectors
    msr vbar_el1, x0
    isb

case_a:
    adr x0, case_b
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'A'
    ldr x1, =which
    strb w0, [x1]
    msr pan, #0
    ldr x0, =(0x3c9 | (1 << 22))     // EL2h, DAIF masked, PAN
    msr spsr_el1, x0
    adr x0, 1f
    msr elr_el1, x0
    eret
1:  b .                               // never reached: IL is set

case_b:
    adr x0, case_c
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'B'
    ldr x1, =which
    strb w0, [x1]
    ldr x2, =data
    add x2, x2, #12
    mov x3, #1
    ldadd x3, x4, [x2]
    b case_c

case_c:
    mov w0, #'C'
    ldr x1, =which
    strb w0, [x1]
    ldr x0, =0x0123456789abcdef
    msr apdakeylo_el1, x0
    ldr x0, =0xfedcba9876543210
    msr apdakeyhi_el1, x0
    ldr x0, =(25 | (20 << 16))       // T0SZ 25, T1SZ 20, TBI0 = TBI1 = 0
    msr tcr_el1, x0
    mrs x0, sctlr_el1
    orr x0, x0, #(1 << 27)            // EnDA
    msr sctlr_el1, x0
    isb
    ldr x5, =0x7f80000000001000
    mov x6, #0
    pacda x5, x6
    mov w0, #'C'
    bl putc
    mov w0, #' '
    bl putc
    mov x0, x5
    bl puthex
    mov w0, #'\n'
    bl putc

case_d:
    ldr x1, =UART
    mov w0, #0x70                     // UARTLCR_H: WLEN 8 bits, FEN
    str w0, [x1, #0x2c]
    mov w0, #0x381                    // UARTCR: UARTEN, TXE, RXE, LBE
    str w0, [x1, #0x30]
    mov w2, #'a'
1:  str w2, [x1]
    add w2, w2, #1
    cmp w2, #('a' + 17)
    b.ne 1b
    ldr x3, =rxbuf
    mov x4, #0
2:  ldr w0, [x1, #0x18]               // UARTFR
    tbnz w0, #4, 3f                   // RXFE
    ldr w0, [x1]
    strb w0, [x3, x4]
    add x4, x4, #1
    cmp x4, #32
    b.ne 2b
3:  mov w0, #0x301                    // loopback off
    str w0, [x1, #0x30]
    mov w0, #'D'
    bl putc
    mov w0, #' '
    bl putc
    mov x5, #0
4:  cmp x5, x4
    b.eq 5f
    ldr x3, =rxbuf
    ldrb w0, [x3, x5]
    bl putc
    add x5, x5, #1
    b 4b
5:  mov w0, #'\n'
    bl putc

mmu_on:
    ldr x0, =0xff00                   // MAIR: attr 0 Device-nGnRnE, attr 1 Normal WB
    msr mair_el1, x0
    // T0SZ 25, inner and outer write-back, inner shareable, EPD1, IPS 36
    // bits.
    ldr x0, =(25 | (0x5 << 8) | (3 << 12) | (1 << 23) | (1 << 32))
    msr tcr_el1, x0
    ldr x0, =l1
    msr ttbr0_el1, x0
    isb
    tlbi vmalle1
    dsb sy
    mrs x0, sctlr_el1
    orr x0, x0, #1                    // M
    msr sctlr_el1, x0
    isb

case_e:
    adr x0, case_f
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'E'
    ldr x1, =which
    strb w0, [x1]
    ldr x2, =0x80000123
    dc civac, x2
    bl none

case_f:
    adr x0, case_g
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'F'
    ldr x1, =which
    strb w0, [x1]
    ldr x2, =0x80000123
    dc cvap, x2
    bl none

case_g:
    adr x0, case_h
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'G'
    ldr x1, =which
    strb w0, [x1]
    msr pan, #1
    ldr x2, =0xc0000100
    dc cvap, x2
    msr pan, #0
    bl none

case_h:
    adr x0, case_i
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'H'
    ldr x1, =which
    strb w0, [x1]
    ldr x2, =0x80000123
    ic ivau, x2
    bl none

case_i:
    adr x0, el0_setup
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'I'
    ldr x1, =which
    strb w0, [x1]
    ldr x2, =0x100000100
    dc ivac, x2
    bl none

el0_setup:
    mrs x0, sctlr_el1
    bic x0, x0, #(1 << 18)            // nTWE: EL0's WFE and WFET trap
    msr sctlr_el1, x0
    // EL0 may read CNTVCT_EL0; the event stream is off.
    mov x0, #(1 << 1)
    msr cntkctl_el1, x0
    // The virtual timer's PPI (27) pending at the CPU interface: the GIC's
    // distributor and CPU interface on, the PPI enabled, and the timer
    // expired (CVAL 0) and enabled.
    ldr x1, =0x08000000
    mov w2, #1
    str w2, [x1]                      // GICD_CTLR
    mov w2, #(1 << 27)
    str w2, [x1, #0x100]              // GICD_ISENABLER0
    ldr x1, =0x08010000
    mov w2, #1
    str w2, [x1]                      // GICC_CTLR
    mov w2, #0xff
    str w2, [x1, #4]                  // GICC_PMR
    isb

case_j:
    adr x0, case_k
    adr x1, j_el0
    mov w2, #'J'
    b to_el0
j_el0:
    wfe
    svc #0

case_k:
    mov x0, #((1 << 2) | (1 << 1))    // the event stream on bit 0 of the count
    msr cntkctl_el1, x0
    isb
    adr x0, case_l
    adr x1, k_el0
    mov w2, #'K'
    b to_el0
k_el0:
    sevl
    wfe
    mov x3, #(1 << 16)
1:  subs x3, x3, #1
    b.ne 1b
    wfe
    svc #0

case_l:
    mov x0, #(1 << 1)                 // the event stream off
    msr cntkctl_el1, x0
    msr cntv_cval_el0, xzr
    mov x0, #1
    msr cntv_ctl_el0, x0              // enabled, condition met: PPI 27 pending
    isb
    adr x0, case_m
    adr x1, l_el0
    mov w2, #'L'
    b to_el0
l_el0:
    sevl
    wfe
    mov x1, #-1
    wfet x1
    svc #0

case_m:
    adr x0, case_n
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'M'
    ldr x1, =which
    strb w0, [x1]
    mrs x5, dbgauthstatus_el1
    mov w0, #'M'
    bl putc
    mov w0, #' '
    bl putc
    mov x0, x5
    bl puthex
    mov w0, #'\n'
    bl putc

case_n:
    adr x0, case_o
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'N'
    ldr x1, =which
    strb w0, [x1]
    msr cntv_ctl_el0, xzr             // the timer's interrupt no longer pending
    // The event stream on bit 23 of the count (EVNTI 15, EVNTIS), EL0 may
    // read CNTVCT_EL0.
    ldr x0, =((1 << 17) | (15 << 4) | (1 << 2) | (1 << 1))
    msr cntkctl_el1, x0
    isb
    sevl
    wfe
    wfe                               // to the stream's next event
    mov x6, #8                        // rounds
    mov x7, #0                        // rounds that took half a period
1:  isb
    mrs x5, cntvct_el0
    sevl
    wfe
    wfe
    isb
    mrs x0, cntvct_el0
    sub x0, x0, x5
    cmp x0, #(1 << 23)
    cinc x7, x7, hs
    subs x6, x6, #1
    b.ne 1b
    mov x0, #(1 << 1)                 // the event stream off
    msr cntkctl_el1, x0
    mov w0, #'N'
    bl putc
    mov w0, #' '
    bl putc
    mov x0, x7
    bl puthex
    mov w0, #'\n'
    bl putc

case_o:
    adr x0, done
    ldr x1, =resume
    str x0, [x1]
    mov w0, #'O'
    ldr x1, =which
    strb w0, [x1]
    isb
    mrs x5, cntvct_el0
    mov x1, #(1 << 24)
    add x1, x5, x1
    wfit x1
    isb
    mrs x0, cntvct_el0
    sub x5, x0, x5
    mov w0, #'O'
    bl putc
    mov w0, #' '
    bl putc
    lsr x0, x5, #24                   // whole timeouts the count moved by
    bl puthex
    mov w0, #'\n'
    bl putc

case_p:
    mrs x5, id_aa64mmfr0_el1
    mov w0, #'P'
    bl putc
    mov w0, #' '
    bl putc
    mov x0, x5
    bl puthex
    mov w0, #'\n'
    bl putc
    b done

// Resume at x0 after the case, then ERET to EL0 at x1 with every interrupt
// masked; w2 names the case.
to_el0:
    ldr x3, =resume
    str x0, [x3]
    ldr x3, =which
    strb w2, [x3]
    msr elr_el1, x1
    mov x0, #0x3c0                    // EL0t, D, A, I and F masked
    msr spsr_el1, x0
    eret

done:
    ldr x0, =0x84000008               // PSCI SYSTEM_OFF
    hvc #0
    b .

// A synchronous exception, from EL1 with SP_ELx or from EL0: print the
// case, ESR_EL1, and SPSR_EL1 for cases A to D, FAR_EL1 for E to I or
// ELR_EL1 for the rest, and go on to the next case at EL1 with PSTATE.PAN
// clear.
handler:
    ldr x1, =which
    ldrb w0, [x1]
    bl putc
    mov w0, #' '
    bl putc
    mrs x0, esr_el1
    bl puthex
    mov w0, #' '
    bl putc
    ldr x1, =which
    ldrb w2, [x1]
    mrs x0, spsr_el1
    cmp w2, #'E'
    b.lo 1f
    mrs x0, far_el1
    cmp w2, #'J'
    b.lo 1f
    mrs x0, elr_el1
1:  bl puthex
    mov w0, #'\n'
    bl putc
    msr pan, #0
    ldr x1, =resume
    ldr x0, [x1]
    br x0

// A case that did not fault: print it and "none".
none:
    mov x14, x30
    ldr x1, =which
    ldrb w0, [x1]
    bl putc
    adr x15, none_text
1:  ldrb w0, [x15], #1
    cbz w0, 2f
    bl putc
    b 1b
2:  mov x30, x14
    ret
none_text:
    .asciz " none\n"
    .balign 4

putc:
    ldr x1, =UART
    strb w0, [x1]
    ret

puthex:                                // x0, as 16 hex digits
    mov x9, x30
    mov x10, x0
    mov x11, #60
2:  lsr x0, x10, x11
    and x0, x0, #0xf
    cmp x0, #10
    add x12, x0, #'0'
    add x13, x0, #('a' - 10)
    csel x0, x12, x13, lo
    bl putc
    subs x11, x11, #4
    b.ge 2b
    mov x30, x9
    ret

    .balign 2048
vectors:
    .skip 0x200
    b handler                          // current EL, SP_ELx, synchronous
    .skip 0x400 - 0x204
    b handler                          // lower EL, AArch64, synchronous
    .skip 0x800 - 0x404

    .data
    .balign 16
data:   .skip 64
resume: .quad 0
which:  .byte 0
rxbuf:  .skip 32
    .balign 16
    .skip 4096
stack_top:

// The level-1 table of cases E to I: 1 GiB blocks, each with the access
// flag; VA 1 GiB, 3 GiB and 4 GiB are RAM (attr 1, inner shareable).
    .balign 4096
l1:
    .quad 0x00000000 | (1 << 54) | (1 << 53) | (1 << 10) | 1                   // Device, EL1's
    .quad 0x40000000 | (3 << 8) | (1 << 10) | (1 << 2) | 1                     // EL1's
    .quad 0                                                                    // nothing
    .quad 0x40000000 | (1 << 54) | (1 << 53) | (3 << 8) | (1 << 10) | (1 << 6) | (1 << 2) | 1  // EL0 read/write
    .quad 0x40000000 | (1 << 54) | (1 << 53) | (3 << 8) | (1 << 10) | (2 << 6) | (1 << 2) | 1  // read-only
    .skip 4096 - 5 * 8
