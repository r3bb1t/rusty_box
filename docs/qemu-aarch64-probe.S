// Bare-metal EL1 probe for the QEMU AArch64 bugs in docs/qemu-upstream-bugs.md
// that need system mode. Each case prints one line on the PL011.
//
//   aarch64-linux-gnu-gcc -march=armv8.5-a+pauth -nostdlib -static \
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

    ldr x0, =0x84000008               // PSCI SYSTEM_OFF
    hvc #0
    b .

// The synchronous exception from the current EL with SP_ELx: print the
// case, ESR_EL1 and SPSR_EL1, and go on to the next case.
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
    mrs x0, spsr_el1
    bl puthex
    mov w0, #'\n'
    bl putc
    ldr x1, =resume
    ldr x0, [x1]
    br x0

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
    .skip 0x800 - 0x204

    .data
    .balign 16
data:   .skip 64
resume: .quad 0
which:  .byte 0
rxbuf:  .skip 32
    .balign 16
    .skip 4096
stack_top:
