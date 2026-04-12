# Bochs Bugs Found During Rusty Box AVX-512 Audit

Issues discovered in Bochs source code (`cpp_orig/bochs/`) during the
line-by-line comparison of Rusty Box AVX-512 handlers against Bochs.

## 1. VPCONFLICTD/Q Off-by-One ()

**Severity**: Correctness bug — produces wrong results.

**Location**: `cpp_orig/bochs/cpu/avx/simd_int.h` lines 1137 and 1151

**Bug**: The inner loop uses `i < index-1` instead of `i < index`:
```cpp
// 
for (int i=0; i < index-1; i++) {
    if (op->vmm32u(index) == op->vmm32u(i)) result |= (1 << i);
}
```

**Expected** (per Intel SDM): Element `i` should compare against ALL
earlier elements `j` where `j < i`. The loop bound `index-1` skips
the immediately preceding element.

**Example**: For vector `[5, 5, 5, 5]`:
- Bochs result:  `[0b0000, 0b0000, 0b0001, 0b0011]` (misses adjacent)
- Correct result: `[0b0000, 0b0001, 0b0011, 0b0111]`

Same bug exists in `simd_pconflictq` at line 1151.

**Our code**: Correct — uses `for j in 0..i` (avx512_misc.rs:410).

---

## 2. KSHIFTLW/KSHIFTRW Threshold ()

**Severity**: Minor — affects only count=15 edge case.

**Location**: `cpp_orig/bochs/cpu/avx/avx512_mask16.cc` lines 155 and 170

**Bug**: Uses `count < 15` threshold instead of `count < 16`:
```cpp
// 
if (count < 15)
    opmask = BX_READ_16BIT_OPMASK(i->src()) << count;
```

For count=15, Bochs returns 0. But `(u16) << 15` is valid and should
shift bit 0 to position 15. All other widths use the correct threshold:
- B: `count < 8` (correct for 8-bit)
- W: `count < 15` (**should be `< 16`**)
- D: `count < 32` (correct for 32-bit)
- Q: `count < 64` (correct for 64-bit)

**Our code**: Uses `count >= 16` which is correct per Intel SDM.

---

## 3. VPSRLQ Shift-by-64 UB ()

**Severity**: Undefined behavior — compiler-dependent result.

**Location**: `cpp_orig/bochs/cpu/avx/simd_int.h` line 1340

**Bug**: Uses `shift_64 > 64` threshold (not `> 63`):
```cpp
// 
if(shift_64 > 64) { op->xmm64u(n) = 0; continue; }
op->xmm64u(n) >>= shift_64;
```

When `shift_64 == 64`, the condition is false and `val >>= 64` executes,
which is C++ undefined behavior for a 64-bit type. On most x86 compilers
this happens to produce 0, but it's not guaranteed.

The equivalent dword function `xmm_psrld` correctly uses `> 31` (= `>= 32`).

**Our code**: Uses `count >= 64` which correctly zeros. Same pattern
for VPSRAVQ at  (`> 64` instead of `> 63`).

---

## 4. VPSRLQ/VPSRAVQ Shift-by-Register Count==64 (same root cause)

Same issue manifests in the variable shift path:
- `xmm_psrlvq` (): `shift > 63` — correct
- `xmm_psravq` (): `shift > 64` — UB at shift==64

**Our code**: Both use `>= 64` which is correct.

---

## Notes

These are genuine bugs in Bochs 3.0 source code. The off-by-one in
VPCONFLICTD produces verifiably wrong results. The shift UB and
KSHIFTLW threshold are edge cases unlikely to affect normal software.

Our Rusty Box implementations follow the Intel SDM specification,
which in these cases differs from Bochs behavior.
