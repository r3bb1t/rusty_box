// User-mode probe for the QEMU AArch64 bugs docs/qemu-upstream-bugs.md lists
// as fixed upstream since QEMU 8.2.2, and for the open ones a Linux program
// reaches. Each case prints one line: what the instruction did, and what the
// Arm ARM's pseudocode says it must do.
//
//   aarch64-linux-gnu-gcc -O1 -static -march=armv8.4-a+fp16+fp16fml \
//       -o qemu-aarch64-probe-user qemu-aarch64-probe-user.c
//   qemu-aarch64 -cpu max ./qemu-aarch64-probe-user <case>
//
// Cases: ldapur fcvtzs_half fcmla_elem_q0 fjcvtzs_fz fabs_scalar_half
// misc_fp16_abort, fixed: on QEMU 8.2.2 the first four print DIFFERS and the
// last two abort QEMU (scalar FABS's word, 0x5ef8f820, aborts; FNEG's,
// 0x7ef8f820, executes); on 11.1.2 every one agrees with Arm. Open:
// fneg_vector_ah (entry 12), which prints DIFFERS on 11.1.2 and agrees with
// docs/qemu-afp-vector-fabs-fneg.patch.
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void on_sigill(int sig) {
    (void)sig;
    printf("SIGILL (UNDEFINED)\n");
    exit(0);
}

static uint64_t fpsr_get(void) { uint64_t v; __asm__ volatile("mrs %0, fpsr" : "=r"(v)); return v; }
static void fpsr_set(uint64_t v) { __asm__ volatile("msr fpsr, %0" ::"r"(v)); }
static void fpcr_set(uint64_t v) { __asm__ volatile("msr fpcr, %0" ::"r"(v)); }

// LDAPURSB with a negative offset: Arm SignExtend(imm9).
static void ldapur(void) {
    static uint8_t buf[1024];
    for (int i = 0; i < 1024; i++) buf[i] = i < 512 ? 0x11 : 0x22;
    uint8_t *base = buf + 400;
    uint64_t v;
    // ldapursb x1, [x3, #-166]
    __asm__ volatile("mov x3, %1\n .inst 0x1995a061\n mov %0, x1" : "=r"(v) : "r"(base) : "x1", "x3", "memory");
    printf("ldapursb x1, [x3, #-166] read %#llx: %s (Arm: 0x11, from base - 166)\n", (unsigned long long)v,
           v == 0x11 ? "agrees" : "DIFFERS");
}

// FCVTZS (scalar, fixed-point) to half precision: Arm writes V[d] = the
// 16-bit result, zeros above it.
static void fcvtzs_half(void) {
    uint32_t w;
    __asm__ volatile("mov w1, #0xc800\n fmov h15, w1\n fcvtzs h9, h15, #12\n fmov w0, s9\n mov %w0, w0"
                     : "=r"(w) : : "x0", "x1", "v9", "v15");
    printf("fcvtzs h9, h15, #12 of -8.0: s9 = %#x: %s (Arm: 0x8000)\n", w, w == 0x8000 ? "agrees" : "DIFFERS");
}

// FCMLA (by element) of half precision with Q = 0: Arm computes the four
// elements of the 64-bit vector only; signalling NaNs above them raise
// nothing.
static void fcmla_elem_q0(void) {
    fpcr_set(0);
    fpsr_set(0);
    __asm__ volatile(
        "movi v4.2d, #0\n movi v13.2d, #0\n movi v6.2d, #0\n"
        "mov w1, #0x3c00\n dup v13.4h, w1\n dup v6.4h, w1\n"   /* 1.0 in the low 64 bits */
        "mov w1, #0x7c01\n mov x2, #0\n dup v20.4h, w1\n"      /* half sNaN */
        "mov v13.d[1], v20.d[0]\n mov v6.d[1], v20.d[0]\n mov v4.d[1], v20.d[0]\n"
        "fcmla v4.4h, v13.4h, v6.h[1], #0\n"
        ::: "x1", "x2", "v4", "v6", "v13", "v20");
    uint64_t s = fpsr_get();
    printf("fcmla v4.4h, v13.4h, v6.h[1], #0 with sNaN above 64 bits: FPSR = %#llx: %s (Arm: 0)\n",
           (unsigned long long)s, s == 0 ? "agrees" : "DIFFERS");
}

// FJCVTZS of a positive denormal under FPCR.FZ: Arm's FPToFixedJS clears Z
// (the operand's fraction is nonzero, so the conversion is not exact).
static void fjcvtzs_fz(void) {
    uint64_t nzcv;
    fpcr_set(1 << 24);
    __asm__ volatile("mov x1, #1\n fmov d1, x1\n fjcvtzs w0, d1\n mrs %0, nzcv" : "=r"(nzcv) : : "x0", "x1", "v1");
    fpcr_set(0);
    printf("fjcvtzs w0, d1 of the smallest denormal, FPCR.FZ = 1: NZCV = %#llx: %s (Arm: Z clear)\n",
           (unsigned long long)nzcv, (nzcv & (1 << 30)) == 0 ? "agrees" : "DIFFERS");
}

// The scalar half-precision class asisdmiscfp16 allocates nothing at a = 1,
// opcode 01111 (the vector class's FABS/FNEG): UNDEFINED.
static void fabs_scalar_half(void) {
    signal(SIGILL, on_sigill);
    __asm__ volatile(".inst 0x5ef8f820" ::: "v0");
    printf("0x5ef8f820 (scalar \"fabs h0, h1\") executed (Arm: UNDEFINED)\n");
}

// An unallocated two-register miscellaneous half-precision word: Arm:
// UNDEFINED; QEMU 8.2.2 aborts in disas_simd_two_reg_misc_fp16.
static void misc_fp16_abort(void) {
    signal(SIGILL, on_sigill);
    __asm__ volatile(".inst 0x4ef9fabd" ::: "v29");
    printf("0x4ef9fabd executed (Arm: UNDEFINED)\n");
}

// FNEG and FABS (vector) under FPCR.AH = 1 (FEAT_AFP): Arm's FPNeg() and
// FPAbs() leave a NaN unchanged, as the scalar forms do.
static void fneg_vector_ah(void) {
    uint32_t neg, neg_scalar, abs;
    fpcr_set(1 << 1); /* FPCR.AH */
    __asm__ volatile(
        "movz w1, #0x0001\n movk w1, #0x7fc0, lsl #16\n dup v1.4s, w1\n"
        "fneg v2.4s, v1.4s\n fneg s3, s1\n umov %w0, v2.s[0]\n umov %w1, v3.s[0]\n"
        "movz w1, #0x0001\n movk w1, #0xffc0, lsl #16\n dup v4.4s, w1\n"
        "fabs v5.4s, v4.4s\n umov %w2, v5.s[0]"
        : "=r"(neg), "=r"(neg_scalar), "=r"(abs) : : "x1", "v1", "v2", "v3", "v4", "v5");
    fpcr_set(0);
    printf("under FPCR.AH: fneg v2.4s of 0x7fc00001 = %#x, fabs v5.4s of 0xffc00001 = %#x: %s"
           " (Arm: unchanged, as scalar fneg s3 = %#x)\n",
           neg, abs, neg == 0x7fc00001 && abs == 0xffc00001 ? "agrees" : "DIFFERS", neg_scalar);
}

int main(int argc, char **argv) {
    const char *c = argc > 1 ? argv[1] : "";
    if (!strcmp(c, "ldapur")) ldapur();
    else if (!strcmp(c, "fcvtzs_half")) fcvtzs_half();
    else if (!strcmp(c, "fcmla_elem_q0")) fcmla_elem_q0();
    else if (!strcmp(c, "fjcvtzs_fz")) fjcvtzs_fz();
    else if (!strcmp(c, "fabs_scalar_half")) fabs_scalar_half();
    else if (!strcmp(c, "misc_fp16_abort")) misc_fp16_abort();
    else if (!strcmp(c, "fneg_vector_ah")) fneg_vector_ah();
    else {
        fprintf(stderr, "cases: ldapur fcvtzs_half fcmla_elem_q0 fjcvtzs_fz fabs_scalar_half misc_fp16_abort"
                        " fneg_vector_ah\n");
        return 2;
    }
    return 0;
}
