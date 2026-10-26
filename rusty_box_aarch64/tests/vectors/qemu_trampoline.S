// Runs the one instruction word at `insn_slot` from a whole register
// state, for gen_qemu.py (see qemu_harness.c).
//
// int run_one(const struct state *in, struct state *out)
//
// Saves what the C caller expects preserved, loads X0-X30, SP, NZCV,
// FPCR, FPSR and V0-V31 from `in`, executes the word, stores the same
// registers into `out` and returns 0. A word that raises a signal resumes
// at `fault_return` (the signal handler moves the PC there), which returns
// 1 with `out` untouched. TPIDR_EL0 holds X0 for the two instructions after
// the word, the one register left free to find `out` with.

#define OFF_SP   248
#define OFF_NZCV 256
#define OFF_FPCR 264
#define OFF_FPSR 272
#define OFF_V    288

    .bss
    .balign 16
host:
    // X19-X30, SP, `out`, TPIDR_EL0, FPCR, D8-D15.
    .space 192

    .section .trampoline, "awx"
    .balign 16
    .global run_one
    .global insn_slot
    .global fault_return
run_one:
    adrp    x9, host
    add     x9, x9, :lo12:host
    stp     x19, x20, [x9, #0]
    stp     x21, x22, [x9, #16]
    stp     x23, x24, [x9, #32]
    stp     x25, x26, [x9, #48]
    stp     x27, x28, [x9, #64]
    stp     x29, x30, [x9, #80]
    mov     x10, sp
    stp     x10, x1, [x9, #96]
    mrs     x10, tpidr_el0
    mrs     x11, fpcr
    stp     x10, x11, [x9, #112]
    stp     d8, d9, [x9, #128]
    stp     d10, d11, [x9, #144]
    stp     d12, d13, [x9, #160]
    stp     d14, d15, [x9, #176]

    ldr     x10, [x0, #OFF_NZCV]
    msr     nzcv, x10
    ldr     x10, [x0, #OFF_FPCR]
    msr     fpcr, x10
    ldr     x10, [x0, #OFF_FPSR]
    msr     fpsr, x10
    ldr     x10, [x0, #OFF_SP]
    mov     sp, x10
    add     x10, x0, #OFF_V
    ldp     q0, q1, [x10, #0]
    ldp     q2, q3, [x10, #32]
    ldp     q4, q5, [x10, #64]
    ldp     q6, q7, [x10, #96]
    ldp     q8, q9, [x10, #128]
    ldp     q10, q11, [x10, #160]
    ldp     q12, q13, [x10, #192]
    ldp     q14, q15, [x10, #224]
    ldp     q16, q17, [x10, #256]
    ldp     q18, q19, [x10, #288]
    ldp     q20, q21, [x10, #320]
    ldp     q22, q23, [x10, #352]
    ldp     q24, q25, [x10, #384]
    ldp     q26, q27, [x10, #416]
    ldp     q28, q29, [x10, #448]
    ldp     q30, q31, [x10, #480]
    ldp     x2, x3, [x0, #16]
    ldp     x4, x5, [x0, #32]
    ldp     x6, x7, [x0, #48]
    ldp     x8, x9, [x0, #64]
    ldp     x10, x11, [x0, #80]
    ldp     x12, x13, [x0, #96]
    ldp     x14, x15, [x0, #112]
    ldp     x16, x17, [x0, #128]
    ldp     x18, x19, [x0, #144]
    ldp     x20, x21, [x0, #160]
    ldp     x22, x23, [x0, #176]
    ldp     x24, x25, [x0, #192]
    ldp     x26, x27, [x0, #208]
    ldp     x28, x29, [x0, #224]
    ldr     x30, [x0, #240]
    ldp     x0, x1, [x0]
insn_slot:
    nop
    msr     tpidr_el0, x0
    adrp    x0, host
    add     x0, x0, :lo12:host
    ldr     x0, [x0, #104]
    str     x1, [x0, #8]
    stp     x2, x3, [x0, #16]
    stp     x4, x5, [x0, #32]
    stp     x6, x7, [x0, #48]
    stp     x8, x9, [x0, #64]
    stp     x10, x11, [x0, #80]
    stp     x12, x13, [x0, #96]
    stp     x14, x15, [x0, #112]
    stp     x16, x17, [x0, #128]
    stp     x18, x19, [x0, #144]
    stp     x20, x21, [x0, #160]
    stp     x22, x23, [x0, #176]
    stp     x24, x25, [x0, #192]
    stp     x26, x27, [x0, #208]
    stp     x28, x29, [x0, #224]
    str     x30, [x0, #240]
    mrs     x1, tpidr_el0
    str     x1, [x0, #0]
    mov     x1, sp
    str     x1, [x0, #OFF_SP]
    mrs     x1, nzcv
    str     x1, [x0, #OFF_NZCV]
    mrs     x1, fpcr
    str     x1, [x0, #OFF_FPCR]
    mrs     x1, fpsr
    str     x1, [x0, #OFF_FPSR]
    add     x1, x0, #OFF_V
    stp     q0, q1, [x1, #0]
    stp     q2, q3, [x1, #32]
    stp     q4, q5, [x1, #64]
    stp     q6, q7, [x1, #96]
    stp     q8, q9, [x1, #128]
    stp     q10, q11, [x1, #160]
    stp     q12, q13, [x1, #192]
    stp     q14, q15, [x1, #224]
    stp     q16, q17, [x1, #256]
    stp     q18, q19, [x1, #288]
    stp     q20, q21, [x1, #320]
    stp     q22, q23, [x1, #352]
    stp     q24, q25, [x1, #384]
    stp     q26, q27, [x1, #416]
    stp     q28, q29, [x1, #448]
    stp     q30, q31, [x1, #480]
    mov     w0, #0
    b       restore
fault_return:
    mov     w0, #1
restore:
    adrp    x9, host
    add     x9, x9, :lo12:host
    ldp     x10, x11, [x9, #112]
    msr     tpidr_el0, x10
    msr     fpcr, x11
    ldr     x10, [x9, #96]
    mov     sp, x10
    ldp     x19, x20, [x9, #0]
    ldp     x21, x22, [x9, #16]
    ldp     x23, x24, [x9, #32]
    ldp     x25, x26, [x9, #48]
    ldp     x27, x28, [x9, #64]
    ldp     x29, x30, [x9, #80]
    ldp     d8, d9, [x9, #128]
    ldp     d10, d11, [x9, #144]
    ldp     d12, d13, [x9, #160]
    ldp     d14, d15, [x9, #176]
    ret
