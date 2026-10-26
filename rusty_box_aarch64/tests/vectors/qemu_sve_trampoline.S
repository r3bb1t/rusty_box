// Runs the one instruction word at `insn_slot` in Streaming SVE mode from
// a whole register state, for gen_qemu_sve.py (see qemu_sve_harness.c).
//
// int run_one(const struct state *in, struct state *out)
//
// Saves what the C caller expects preserved, enters Streaming SVE mode
// (SMSTART SM, which zeroes Z0-Z31, P0-P15 and FFR and sets FPSR), loads
// X0-X30, SP, NZCV, FPCR, FPSR, Z0-Z31 and P0-P15 from `in`, executes the
// word, stores the same registers into `out` and returns 0. A word that
// raises a signal resumes at `fault_return` (the signal handler moves the
// PC there), which returns 1 with `out` untouched. Both leave Streaming SVE
// mode (SMSTOP SM) before restoring D8-D15, which it zeroes. TPIDR_EL0
// holds X0 for the two instructions after the word, the one register left
// free to find `out` with. SMSTART SM and SMSTOP SM are written as words
// (MSR SVCRSM, #1 and #0), which the assembler needs no SME support for.

#define OFF_SP   248
#define OFF_NZCV 256
#define OFF_FPCR 264
#define OFF_FPSR 272
#define OFF_Z    288
#define OFF_P    2336

    .arch armv8.2-a+sve

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

    .inst   0xd503437f          // SMSTART SM
    ldr     x10, [x0, #OFF_NZCV]
    msr     nzcv, x10
    ldr     x10, [x0, #OFF_FPCR]
    msr     fpcr, x10
    ldr     x10, [x0, #OFF_FPSR]
    msr     fpsr, x10
    ldr     x10, [x0, #OFF_SP]
    mov     sp, x10
    add     x10, x0, #OFF_Z
    ldr     z0, [x10, #0, mul vl]
    ldr     z1, [x10, #1, mul vl]
    ldr     z2, [x10, #2, mul vl]
    ldr     z3, [x10, #3, mul vl]
    ldr     z4, [x10, #4, mul vl]
    ldr     z5, [x10, #5, mul vl]
    ldr     z6, [x10, #6, mul vl]
    ldr     z7, [x10, #7, mul vl]
    ldr     z8, [x10, #8, mul vl]
    ldr     z9, [x10, #9, mul vl]
    ldr     z10, [x10, #10, mul vl]
    ldr     z11, [x10, #11, mul vl]
    ldr     z12, [x10, #12, mul vl]
    ldr     z13, [x10, #13, mul vl]
    ldr     z14, [x10, #14, mul vl]
    ldr     z15, [x10, #15, mul vl]
    ldr     z16, [x10, #16, mul vl]
    ldr     z17, [x10, #17, mul vl]
    ldr     z18, [x10, #18, mul vl]
    ldr     z19, [x10, #19, mul vl]
    ldr     z20, [x10, #20, mul vl]
    ldr     z21, [x10, #21, mul vl]
    ldr     z22, [x10, #22, mul vl]
    ldr     z23, [x10, #23, mul vl]
    ldr     z24, [x10, #24, mul vl]
    ldr     z25, [x10, #25, mul vl]
    ldr     z26, [x10, #26, mul vl]
    ldr     z27, [x10, #27, mul vl]
    ldr     z28, [x10, #28, mul vl]
    ldr     z29, [x10, #29, mul vl]
    ldr     z30, [x10, #30, mul vl]
    ldr     z31, [x10, #31, mul vl]
    add     x10, x0, #OFF_P
    ldr     p0, [x10, #0, mul vl]
    ldr     p1, [x10, #1, mul vl]
    ldr     p2, [x10, #2, mul vl]
    ldr     p3, [x10, #3, mul vl]
    ldr     p4, [x10, #4, mul vl]
    ldr     p5, [x10, #5, mul vl]
    ldr     p6, [x10, #6, mul vl]
    ldr     p7, [x10, #7, mul vl]
    ldr     p8, [x10, #8, mul vl]
    ldr     p9, [x10, #9, mul vl]
    ldr     p10, [x10, #10, mul vl]
    ldr     p11, [x10, #11, mul vl]
    ldr     p12, [x10, #12, mul vl]
    ldr     p13, [x10, #13, mul vl]
    ldr     p14, [x10, #14, mul vl]
    ldr     p15, [x10, #15, mul vl]
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
    add     x1, x0, #OFF_Z
    str     z0, [x1, #0, mul vl]
    str     z1, [x1, #1, mul vl]
    str     z2, [x1, #2, mul vl]
    str     z3, [x1, #3, mul vl]
    str     z4, [x1, #4, mul vl]
    str     z5, [x1, #5, mul vl]
    str     z6, [x1, #6, mul vl]
    str     z7, [x1, #7, mul vl]
    str     z8, [x1, #8, mul vl]
    str     z9, [x1, #9, mul vl]
    str     z10, [x1, #10, mul vl]
    str     z11, [x1, #11, mul vl]
    str     z12, [x1, #12, mul vl]
    str     z13, [x1, #13, mul vl]
    str     z14, [x1, #14, mul vl]
    str     z15, [x1, #15, mul vl]
    str     z16, [x1, #16, mul vl]
    str     z17, [x1, #17, mul vl]
    str     z18, [x1, #18, mul vl]
    str     z19, [x1, #19, mul vl]
    str     z20, [x1, #20, mul vl]
    str     z21, [x1, #21, mul vl]
    str     z22, [x1, #22, mul vl]
    str     z23, [x1, #23, mul vl]
    str     z24, [x1, #24, mul vl]
    str     z25, [x1, #25, mul vl]
    str     z26, [x1, #26, mul vl]
    str     z27, [x1, #27, mul vl]
    str     z28, [x1, #28, mul vl]
    str     z29, [x1, #29, mul vl]
    str     z30, [x1, #30, mul vl]
    str     z31, [x1, #31, mul vl]
    add     x1, x0, #OFF_P
    str     p0, [x1, #0, mul vl]
    str     p1, [x1, #1, mul vl]
    str     p2, [x1, #2, mul vl]
    str     p3, [x1, #3, mul vl]
    str     p4, [x1, #4, mul vl]
    str     p5, [x1, #5, mul vl]
    str     p6, [x1, #6, mul vl]
    str     p7, [x1, #7, mul vl]
    str     p8, [x1, #8, mul vl]
    str     p9, [x1, #9, mul vl]
    str     p10, [x1, #10, mul vl]
    str     p11, [x1, #11, mul vl]
    str     p12, [x1, #12, mul vl]
    str     p13, [x1, #13, mul vl]
    str     p14, [x1, #14, mul vl]
    str     p15, [x1, #15, mul vl]
    mov     w0, #0
    b       restore
fault_return:
    mov     w0, #1
restore:
    .inst   0xd503427f          // SMSTOP SM
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
