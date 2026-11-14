// A bare-metal EL2 program for gen_qemu_el2.py: runs each case of `cases`
// (the cases file the script writes) on
// `qemu-system-aarch64 -M virt,virtualization=on -cpu cortex-a57`, which
// starts it at EL2, and prints what each case did on the PL011 UART.
//
// A case writes EL2's controls (HCR_EL2, CPTR_EL2, MDCR_EL2, CNTHCTL_EL2,
// HSTR_EL2) and EL1's (CPACR_EL1, SCTLR_EL1, CNTKCTL_EL1, MDSCR_EL1,
// OSLAR_EL1, SPSR_EL1, ELR_EL1), puts its word in `slot` with a sentinel
// after it, and runs the word at EL0 or EL1 (an ERET from EL2 with the
// case's SPSR_EL2, the word's X0 the case's) or at EL2 (a branch). The
// sentinel is `svc #0xffff` at EL0 and `hvc #0xffff` at EL1 and EL2: a
// word that completes reaches it, and its exception, at `slot + 8`, says
// so. Every exception ends at EL2: EL1's handler saves X0 and calls
// `hvc #0xfffe`, and EL2 reads the exception EL1 took from EL1's
// registers. The cases never set HCR_EL2.HCD, nor HCR_EL2.TGE for EL1.
//
// Output, one line each, every value 16 hex digits:
//   S <MIDR_EL1> <ID_AA64PFR0_EL1> <ID_AA64DFR0_EL1> <HCR_EL2> <SCTLR_EL2>
//     <CPTR_EL2> <MDCR_EL2> <CNTHCTL_EL2> <SCTLR_EL1>        (at reset)
//   R <outcome> <X0> <ESR> <FAR> <ELR - slot> <SPSR>         (per case)
//   END
// The outcome is 0 for a word that completed (the rest zero but X0, its
// value after the word), 1 for an exception taken to EL1 and 2 for one
// taken to EL2 (that level's ESR, FAR, ELR and SPSR).

#define UART        0x09000000

#define C_HCR       0
#define C_CPTR      8
#define C_MDCR      16
#define C_CNTHCTL   24
#define C_HSTR      32
#define C_CPACR     40
#define C_SCTLR1    48
#define C_CNTKCTL   56
#define C_MDSCR     64
#define C_TARGET    72
#define C_X0        80
#define C_SPSR1     88
#define C_ELR1      96
#define C_WORD      104
#define C_OSLAR     112
#define C_ENTRY     120
#define CASE_SIZE   128

#define SVC_SENTINEL    0xd41fffe1
#define HVC_SENTINEL    0xd41fffe2
#define SVC_FFFF_ESR    0x5600ffff
#define HVC_FFFF_ESR    0x5a00ffff
#define HVC_FFFE_ESR    0x5a00fffe

    .text
    .global _start
_start:
    ldr x0, =stack_top
    mov sp, x0
    ldr x0, =el2_vectors
    msr vbar_el2, x0
    ldr x0, =el1_vectors
    msr vbar_el1, x0
    ldr x0, =el1_stack_top
    msr sp_el1, x0
    ldr x0, =el0_stack_top
    msr sp_el0, x0
    isb
    mov w0, #'S'
    bl putc
    mrs x0, midr_el1
    bl put_hex
    mrs x0, id_aa64pfr0_el1
    bl put_hex
    mrs x0, id_aa64dfr0_el1
    bl put_hex
    mrs x0, hcr_el2
    bl put_hex
    mrs x0, sctlr_el2
    bl put_hex
    mrs x0, cptr_el2
    bl put_hex
    mrs x0, mdcr_el2
    bl put_hex
    mrs x0, cnthctl_el2
    bl put_hex
    mrs x0, sctlr_el1
    bl put_hex
    bl newline
    ldr x0, =cases
    ldr x1, =current_case
    str x0, [x1]

next_case:
    ldr x1, =current_case
    ldr x19, [x1]
    ldr x20, =cases_end
    cmp x19, x20
    b.hs finished

    ldr x1, [x19, #C_HCR]
    msr hcr_el2, x1
    ldr x1, [x19, #C_CPTR]
    msr cptr_el2, x1
    ldr x1, [x19, #C_MDCR]
    msr mdcr_el2, x1
    ldr x1, [x19, #C_CNTHCTL]
    msr cnthctl_el2, x1
    ldr x1, [x19, #C_HSTR]
    msr hstr_el2, x1
    ldr x1, [x19, #C_CPACR]
    msr cpacr_el1, x1
    ldr x1, [x19, #C_SCTLR1]
    msr sctlr_el1, x1
    ldr x1, [x19, #C_CNTKCTL]
    msr cntkctl_el1, x1
    ldr x1, [x19, #C_MDSCR]
    msr mdscr_el1, x1
    ldr x1, [x19, #C_OSLAR]
    msr oslar_el1, x1
    ldr x1, [x19, #C_SPSR1]
    msr spsr_el1, x1
    ldr x1, [x19, #C_ELR1]
    msr elr_el1, x1

    // The slot: the word, then the sentinel for its level.
    ldr x1, =slot
    ldr w2, [x19, #C_WORD]
    str w2, [x1]
    ldr x3, [x19, #C_TARGET]
    ldr w2, =SVC_SENTINEL
    ldr w4, =HVC_SENTINEL
    cmp x3, #0
    csel w2, w2, w4, eq
    str w2, [x1, #4]
    dc cvau, x1
    dsb ish
    ic iallu
    dsb ish
    isb

    ldr x0, [x19, #C_X0]
    cmp x3, #2
    b.eq 1f
    ldr x2, [x19, #C_ENTRY]
    msr spsr_el2, x2
    msr elr_el2, x1
    eret
1:  br x1

// A synchronous exception taken to EL2, from EL2 (the word, or the
// sentinel after it) or from below.
el2_sync:
    mov x21, x0
    ldr x22, =slot
    mrs x23, esr_el2
    mrs x24, elr_el2
    sub x24, x24, x22
    ldr x2, =HVC_FFFE_ESR
    cmp x23, x2
    b.eq from_el1
    ldr x2, =HVC_FFFF_ESR
    cmp x23, x2
    b.eq 2f
    ldr x2, =SVC_FFFF_ESR
    cmp x23, x2
    b.ne 3f
2:  cmp x24, #8
    b.ne 3f
    mov x20, #0
    mov x23, #0
    mov x24, #0
    mov x25, #0
    mov x26, #0
    b report
3:  mov x20, #2
    mrs x25, far_el2
    mrs x26, spsr_el2
    b report

// EL1 took the exception and called `hvc #0xfffe`: read it from EL1's
// registers, X0 from where EL1's handler saved it.
from_el1:
    ldr x2, =el1_x0
    ldr x21, [x2]
    mrs x23, esr_el1
    mrs x24, elr_el1
    sub x24, x24, x22
    mrs x25, far_el1
    mrs x26, spsr_el1
    ldr x2, =SVC_FFFF_ESR
    cmp x23, x2
    b.ne 4f
    cmp x24, #8
    b.ne 4f
    mov x20, #0
    mov x23, #0
    mov x24, #0
    mov x25, #0
    mov x26, #0
    b report
4:  mov x20, #1

report:
    ldr x0, =stack_top
    mov sp, x0
    mov w0, #'R'
    bl putc
    mov x0, x20
    bl put_hex
    mov x0, x21
    bl put_hex
    mov x0, x23
    bl put_hex
    mov x0, x25
    bl put_hex
    mov x0, x24
    bl put_hex
    mov x0, x26
    bl put_hex
    bl newline
    ldr x1, =current_case
    ldr x19, [x1]
    add x19, x19, #CASE_SIZE
    str x19, [x1]
    b next_case

// Anything else at EL2 ends the run: 'U', the vector offset, and ESR_EL2.
el2_unexpected:
    ldr x1, =stack_top
    mov sp, x1
    mov x19, x0
    mov w0, #'U'
    bl putc
    mov x0, x19
    bl put_hex
    mrs x0, esr_el2
    bl put_hex
    bl newline
    b finished

finished:
    mov w0, #'E'
    bl putc
    mov w0, #'N'
    bl putc
    mov w0, #'D'
    bl putc
    bl newline
    // PSCI SYSTEM_OFF, through the firmware's SMC conduit.
    ldr x0, =0x84000008
    smc #0
5:  wfi
    b 5b

// ' ' and x0 in 16 hex digits.
put_hex:
    stp x29, x30, [sp, #-32]!
    stp x19, x20, [sp, #16]
    mov x19, x0
    mov w0, #' '
    bl putc
    mov x20, #60
1:  lsr x0, x19, x20
    and x0, x0, #0xf
    cmp x0, #10
    add x1, x0, #'0'
    add x2, x0, #('a' - 10)
    csel x0, x1, x2, lo
    bl putc
    subs x20, x20, #4
    b.pl 1b
    ldp x19, x20, [sp, #16]
    ldp x29, x30, [sp], #32
    ret

newline:
    mov w0, #'\n'
putc:
    ldr x9, =UART
    strb w0, [x9]
    ret

    .ltorg

    .section .el2_vectors, "ax"
    .balign 2048
    .global el2_vectors
el2_vectors:
    .irp offset, 0x000, 0x080, 0x100, 0x180
    .balign 128
    mov x0, #\offset
    b el2_unexpected
    .endr
    .balign 128
    b el2_sync
    .irp offset, 0x280, 0x300, 0x380
    .balign 128
    mov x0, #\offset
    b el2_unexpected
    .endr
    .balign 128
    b el2_sync
    .irp offset, 0x480, 0x500, 0x580, 0x600, 0x680, 0x700, 0x780
    .balign 128
    mov x0, #\offset
    b el2_unexpected
    .endr

    .section .el1_vectors, "ax"
    .balign 2048
    .global el1_vectors
el1_vectors:
    .rept 16
    .balign 128
    b el1_sync
    .endr
el1_sync:
    ldr x1, =el1_x0
    str x0, [x1]
    hvc #0xfffe
    .ltorg

    .section .slot, "awx"
    .balign 64
    .global slot
slot:
    nop
    nop

    .section .xdata, "aw"
    .balign 64
    .global xdata
xdata:
    .space 4096

    .bss
    .balign 16
current_case:
    .space 8
el1_x0:
    .space 8
    .balign 16
    .space 4096
el0_stack_top:
    .space 4096
el1_stack_top:
    .space 16384
stack_top:
