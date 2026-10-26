// A bare-metal EL1 program for gen_qemu_pauth.py: runs each case of
// `cases` (qemu_pauth_cases.S, which the script writes) on
// `qemu-system-aarch64 -M virt -cpu max` and prints what it did on the
// PL011 UART.
//
// A case sets the pointer authentication keys, the key enables of
// SCTLR_EL1 (the rest is the reset value), TCR_EL1, ELR_EL1, NZCV, SP and
// X0-X30, then runs `slot`: three setup words, a snapshot of every
// register (`I`), the word under test at `test_insn`, and a second
// snapshot (`F`). The word under test either falls through (outcome 0),
// branches to `land` (outcome 1, ERETAA and ERETAB too) or raises an
// exception (outcome 2, with ESR_EL1, FAR_EL1 and ELR_EL1). The slot, the
// landing pad and the data region a load reads are at fixed addresses
// (qemu_pauth.ld), which the replay maps the same way.
//
// Output, one line each, every value 16 hex digits:
//   S <SCTLR_EL1 at reset> <ID_AA64ISAR1_EL1>
//   I <X0..X30> <SP> <ELR_EL1> <NZCV> <DAIF>
//   F <X0..X30> <SP> <ELR_EL1> <NZCV> <DAIF> <outcome> <ESR> <FAR> <ELR>
//   END

#define CASE_SIZE   384
#define CASE_SCTLR  80
#define CASE_TCR    88
#define CASE_ELR    96
#define CASE_NZCV   104
#define CASE_SP     112
#define CASE_X      120
#define CASE_WORDS  368

#define BUF_SP      248
#define BUF_ELR     256
#define BUF_NZCV    264
#define BUF_DAIF    272
#define BUF_OUTCOME 280
#define BUF_ESR     288
#define BUF_FAR     296
#define BUF_EXC_ELR 304
#define BUF_SIZE    312

// SCTLR_EL1.EnIA, EnIB, EnDA, EnDB.
#define EN_MASK     ((1 << 31) | (1 << 30) | (1 << 27) | (1 << 13))

#define UART        0x09000000

    .text
    .global _start
_start:
    ldr x0, =stack_top
    mov sp, x0
    ldr x0, =vectors
    msr vbar_el1, x0
    isb
    mrs x0, sctlr_el1
    ldr x1, =EN_MASK
    bic x0, x0, x1
    ldr x1, =sctlr_base
    str x0, [x1]
    mov w0, #'S'
    bl putc
    ldr x1, =sctlr_base
    ldr x0, [x1]
    bl put_hex
    mrs x0, id_aa64isar1_el1
    bl put_hex
    bl newline
    ldr x19, =cases

next_case:
    ldr x20, =cases_end
    cmp x19, x20
    b.hs finished
    ldr x0, =current_case
    str x19, [x0]

    // The keys: APIAKeyLo_EL1 .. APGAKeyHi_EL1.
    ldp x1, x2, [x19, #0]
    msr s3_0_c2_c1_0, x1
    msr s3_0_c2_c1_1, x2
    ldp x1, x2, [x19, #16]
    msr s3_0_c2_c1_2, x1
    msr s3_0_c2_c1_3, x2
    ldp x1, x2, [x19, #32]
    msr s3_0_c2_c2_0, x1
    msr s3_0_c2_c2_1, x2
    ldp x1, x2, [x19, #48]
    msr s3_0_c2_c2_2, x1
    msr s3_0_c2_c2_3, x2
    ldp x1, x2, [x19, #64]
    msr s3_0_c2_c3_0, x1
    msr s3_0_c2_c3_1, x2

    ldr x1, =sctlr_base
    ldr x1, [x1]
    ldr x2, [x19, #CASE_SCTLR]
    ldr x3, =EN_MASK
    and x2, x2, x3
    orr x1, x1, x2
    msr sctlr_el1, x1
    ldr x1, [x19, #CASE_TCR]
    msr tcr_el1, x1
    ldr x1, [x19, #CASE_ELR]
    msr elr_el1, x1
    mov x1, #0x3c5
    msr spsr_el1, x1

    // The slot's words, then the instruction cache.
    ldr x1, =slot
    ldr w2, [x19, #CASE_WORDS]
    str w2, [x1, #0]
    ldr w2, [x19, #CASE_WORDS + 4]
    str w2, [x1, #4]
    ldr w2, [x19, #CASE_WORDS + 8]
    str w2, [x1, #8]
    ldr w2, [x19, #CASE_WORDS + 12]
    str w2, [x1, #16]
    dc cvau, x1
    dsb ish
    ic iallu
    dsb ish
    isb

    ldr x1, [x19, #CASE_SP]
    mov sp, x1
    ldr x1, [x19, #CASE_NZCV]
    msr nzcv, x1
    mov x0, x19
    ldp x1, x2, [x0, #CASE_X + 8]
    ldp x3, x4, [x0, #CASE_X + 24]
    ldp x5, x6, [x0, #CASE_X + 40]
    ldp x7, x8, [x0, #CASE_X + 56]
    ldp x9, x10, [x0, #CASE_X + 72]
    ldp x11, x12, [x0, #CASE_X + 88]
    ldp x13, x14, [x0, #CASE_X + 104]
    ldp x15, x16, [x0, #CASE_X + 120]
    ldp x17, x18, [x0, #CASE_X + 136]
    ldp x19, x20, [x0, #CASE_X + 152]
    ldp x21, x22, [x0, #CASE_X + 168]
    ldp x23, x24, [x0, #CASE_X + 184]
    ldp x25, x26, [x0, #CASE_X + 200]
    ldp x27, x28, [x0, #CASE_X + 216]
    ldp x29, x30, [x0, #CASE_X + 232]
    ldr x0, [x0, #CASE_X]
    b slot

// Every register into `buf`, X0 and X1 included, leaving X0 = `buf` and
// NZCV untouched.
.macro SNAPSHOT buf
    msr tpidr_el1, x0
    ldr x0, =\buf
    stp x1, x2, [x0, #8]
    stp x3, x4, [x0, #24]
    stp x5, x6, [x0, #40]
    stp x7, x8, [x0, #56]
    stp x9, x10, [x0, #72]
    stp x11, x12, [x0, #88]
    stp x13, x14, [x0, #104]
    stp x15, x16, [x0, #120]
    stp x17, x18, [x0, #136]
    stp x19, x20, [x0, #152]
    stp x21, x22, [x0, #168]
    stp x23, x24, [x0, #184]
    stp x25, x26, [x0, #200]
    stp x27, x28, [x0, #216]
    stp x29, x30, [x0, #232]
    mrs x1, tpidr_el1
    str x1, [x0, #0]
    mov x1, sp
    str x1, [x0, #BUF_SP]
    mrs x1, elr_el1
    str x1, [x0, #BUF_ELR]
    mrs x1, nzcv
    str x1, [x0, #BUF_NZCV]
    mrs x1, daif
    str x1, [x0, #BUF_DAIF]
.endm

snap_init:
    SNAPSHOT init_buf
    ldr x1, [x0, #8]
    mrs x0, tpidr_el1
    b test_insn

snap_final:
    SNAPSHOT final_buf
    mov x1, #0
    str x1, [x0, #BUF_OUTCOME]
    b case_done

snap_land:
    SNAPSHOT final_buf
    mov x1, #1
    str x1, [x0, #BUF_OUTCOME]
    b case_done

exception_entry:
    SNAPSHOT final_buf
    mov x1, #2
    str x1, [x0, #BUF_OUTCOME]
    mrs x1, esr_el1
    str x1, [x0, #BUF_ESR]
    mrs x1, far_el1
    str x1, [x0, #BUF_FAR]
    mrs x1, elr_el1
    str x1, [x0, #BUF_EXC_ELR]
    b case_done

case_done:
    ldr x0, =stack_top
    mov sp, x0
    mov w0, #'I'
    bl putc
    ldr x0, =init_buf
    mov x1, #35
    bl put_buf
    mov w0, #'F'
    bl putc
    ldr x0, =final_buf
    mov x1, #39
    bl put_buf
    ldr x0, =current_case
    ldr x19, [x0]
    add x19, x19, #CASE_SIZE
    b next_case

finished:
    mov w0, #'E'
    bl putc
    mov w0, #'N'
    bl putc
    mov w0, #'D'
    bl putc
    bl newline
    // PSCI SYSTEM_OFF.
    ldr x0, =0x84000008
    hvc #0
1:  wfi
    b 1b

// ' ' and the `x1` doublewords at `x0` in hex, then a newline.
put_buf:
    stp x29, x30, [sp, #-32]!
    stp x19, x20, [sp, #16]
    mov x19, x0
    mov x20, x1
1:  ldr x0, [x19], #8
    bl put_hex
    subs x20, x20, #1
    b.ne 1b
    bl newline
    ldp x19, x20, [sp, #16]
    ldp x29, x30, [sp], #32
    ret

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

    .section .slot, "awx"
    .balign 64
    .global slot
slot:
    nop
    nop
    nop
    b snap_init
test_insn:
    nop
    b snap_final

    .section .land, "ax"
    .global land
land:
    b snap_land

    .section .vectors, "ax"
    .balign 2048
vectors:
    .rept 16
    .balign 128
    b exception_entry
    .endr

    .bss
    .balign 16
sctlr_base:
    .space 8
current_case:
    .space 8
init_buf:
    .space BUF_SIZE
final_buf:
    .space BUF_SIZE
    .balign 16
    .space 16384
stack_top:
