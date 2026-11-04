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
// docs/patches/qemu/qemu-afp-vector-fabs-fneg.patch; sve_fcmla_idx_ah (entry 13), which
// prints DIFFERS on 11.1.2 and agrees with
// docs/patches/qemu/qemu-sve-fcmla-indexed-ah.patch; sve_fmad_size00 and
// sve_fsubr_size00 (entry 14), which execute on 11.1.2 where Arm raises
// SIGILL, the first until docs/patches/qemu/qemu-sve-fmad-size00.patch, the second
// until master's f2bfb1cf; sme_ld1_vertical_tail (entry 15), which prints
// DIFFERS on 11.1.2 and agrees with docs/patches/qemu/qemu-sme-ld1-vertical-tail.patch;
// sme2_movt (entry 16), which executes on 11.1.2 and master where Arm
// raises SIGILL.
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

// SVE FCMLA (indexed) under FPCR.AH = 1: rotation #90 negates the real
// half's Zm element (FPNeg), which leaves a NaN unchanged under AH, so
// 0 + 1.0 * FPNeg(NaN 0xffc1) is that NaN. The Advanced SIMD form agrees.
static void sve_fcmla_idx_ah(void) {
    uint32_t sve, simd;
    fpcr_set(1 << 1); /* FPCR.AH */
    __asm__ volatile(
        "mov w1, #0x3c00\n mov w2, #0xffc1\n mov w3, #0\n"
        ".inst 0x05603821\n"   /* dup z1.h, w1: 1.0 */
        ".inst 0x05603842\n"   /* dup z2.h, w2: a negative quiet NaN */
        ".inst 0x05603860\n"   /* dup z0.h, w3 */
        ".inst 0x64a21420\n"   /* fcmla z0.h, z1.h, z2.h[0], #90 */
        "umov %w0, v0.h[0]\n"
        "movi v3.2d, #0\n fcmla v3.8h, v1.8h, v2.h[0], #90\n umov %w1, v3.h[0]"
        : "=r"(sve), "=r"(simd) : : "x1", "x2", "x3", "v0", "v1", "v2", "v3");
    fpcr_set(0);
    printf("under FPCR.AH: fcmla z0.h, z1.h, z2.h[0], #90 of 1.0 by NaN 0xffc1: z0.h[0] = %#x: %s"
           " (Arm: 0xffc1, as Advanced SIMD fcmla v3.8h = %#x)\n",
           sve, sve == 0xffc1 ? "agrees" : "DIFFERS", simd);
}

// FMAD with size = 00, which Arm's decode makes UNDEFINED (`if size ==
// '00' then EndOfDecode(Decode_UNDEF)`); FEAT_SVE_B16B16 has no BFloat16
// FMAD.
static void sve_fmad_size00(void) {
    signal(SIGILL, on_sigill);
    __asm__ volatile(".inst 0x2518e3e0\n"   /* ptrue p0.b */
                     ".inst 0x65228020"     /* fmad z0.<size 00>, p0/m, z1, z2 */
                     ::: "p0", "v0");
    printf("0x65228020 (fmad, size = 00) executed (Arm: UNDEFINED)\n");
}

// FSUBR (vectors) with size = 00: UNDEFINED, as FMAD; FEAT_SVE_B16B16 has
// no BFloat16 FSUBR.
static void sve_fsubr_size00(void) {
    signal(SIGILL, on_sigill);
    __asm__ volatile(".inst 0x2518e3e0\n"   /* ptrue p0.b */
                     ".inst 0x65038020"     /* fsubr z0.<size 00>, p0/m, z0, z1 */
                     ::: "p0", "v0");
    printf("0x65038020 (fsubr, size = 00) executed (Arm: UNDEFINED)\n");
}

// SME LD1W to a vertical tile slice under a predicate whose last active
// element is the first: Arm writes Zeros to every inactive element of the
// slice, the ones after the last active element included. SMSTART and
// SMSTOP zero every Z, P and FFR register, so all of them are clobbered.
static void sme_ld1_vertical_tail(void) {
    static uint32_t words[4] = { 0x11111111, 0x22222222, 0x33333333, 0x44444444 };
    static uint32_t out[64];
    __asm__ volatile("mov x0, %0\n mov x1, %1\n mov x2, #1\n mov w12, #0\n"
                     ".inst 0xd503477f\n"   /* smstart */
                     ".inst 0x2598e3e1\n"   /* ptrue p1.s */
                     ".inst 0x25b8dfe1\n"   /* mov z1.s, #-1 */
                     ".inst 0x25a21fe0\n"   /* whilelo p0.s, xzr, x2: element 0 alone */
                     ".inst 0xc0808420\n"   /* mova za0v.s[w12, 0], p1/m, z1.s */
                     ".inst 0xe09f8000\n"   /* ld1w {za0v.s[w12, 0]}, p0/z, [x0] */
                     ".inst 0xc0828402\n"   /* mova z2.s, p1/m, za0v.s[w12, 0] */
                     ".inst 0xe5804022\n"   /* str z2, [x1] */
                     ".inst 0xd503467f"     /* smstop */
                     : : "r"(words), "r"(out)
                     : "x0", "x1", "x2", "x12", "memory",
                       "v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8", "v9", "v10", "v11", "v12", "v13",
                       "v14", "v15", "v16", "v17", "v18", "v19", "v20", "v21", "v22", "v23", "v24", "v25", "v26",
                       "v27", "v28", "v29", "v30", "v31", "p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8",
                       "p9", "p10", "p11", "p12", "p13", "p14", "p15", "ffr");
    printf("ld1w {za0v.s[w12, 0]}, p0/z with element 0 alone active: element 1 = %#x: %s (Arm: 0)\n", out[1],
           out[1] == 0 ? "agrees" : "DIFFERS");
}

// MOVT from ZT0 to a general register: defined only in Debug state
// (`movt_r_zt_`'s decode, `!Halted()`), so UNDEFINED in a program. ZA
// storage is enabled first, which SMSTART ZA does and ZT0's check needs.
static void sme2_movt(void) {
    signal(SIGILL, on_sigill);
    __asm__ volatile(".inst 0xd503457f\n"   /* smstart za */
                     ".inst 0xc04c03e0\n"   /* movt x0, zt0[0] */
                     ".inst 0xd503447f"     /* smstop za */
                     ::: "x0");
    printf("0xc04c03e0 (movt x0, zt0[0]) executed (Arm: UNDEFINED outside Debug state)\n");
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
    else if (!strcmp(c, "sve_fcmla_idx_ah")) sve_fcmla_idx_ah();
    else if (!strcmp(c, "sve_fmad_size00")) sve_fmad_size00();
    else if (!strcmp(c, "sve_fsubr_size00")) sve_fsubr_size00();
    else if (!strcmp(c, "sme_ld1_vertical_tail")) sme_ld1_vertical_tail();
    else if (!strcmp(c, "sme2_movt")) sme2_movt();
    else {
        fprintf(stderr, "cases: ldapur fcvtzs_half fcmla_elem_q0 fjcvtzs_fz fabs_scalar_half misc_fp16_abort"
                        " fneg_vector_ah sve_fcmla_idx_ah sve_fmad_size00 sve_fsubr_size00"
                        " sme_ld1_vertical_tail sme2_movt\n");
        return 2;
    }
    return 0;
}
