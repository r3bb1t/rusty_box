# Alpine Linux Direct Boot — Triple Fault Investigation

## Status: ROOT CAUSE IDENTIFIED (2026-03-09)

Alpine Linux (6.18.7-virt, x86_64) direct kernel boot triple-faults at instruction ~620,815,422.
The crash is a **page fault on an unmapped identity-map address** (0x10000 = boot_params),
escalating through an empty IDT to triple fault.

## Crash Sequence

```
i=620815296  MOV CR3, 0x30dc000       — switch to kernel page tables
i=620815298  JMP 0xffffffff8128ca78   — enter kernel text mapping
i=620815299-334  CR4/GDT/segments/WRMSR setup
i=620815335  CALL idt_setup_early_handler thunk (0xffffffff8128cd20)
i=620815338  XOR EDI,EDI              — thunk zeros first argument!
i=620815339  JMP 0xffffffff812493d0   — tail-call to actual function
i=620815343  MOV RSI, RDI             — RSI = 0 (from zeroed RDI)
i=620815359  TEST RSI, RSI            — tests early_idts pointer
i=620815360  JZ → skip IDT filling    — RSI=0 → skip!
i=620815361  LIDT (base=0xffffffff82a04000, limit=0x1ff)  — empty IDT loaded
i=620815395  MOV RDI, R15=0x10000     — boot_params pointer
i=620815397  CALL x86_64_start_kernel
i=620815422  REP STOSD at RDI=0x10000 — #PF: page not present
             → IDT vec 14 is all zeros (P=0) → #GP
             → IDT vec 8 is all zeros (P=0) → #GP → TRIPLE FAULT
```

## Root Cause Analysis

### 1. Page Tables Missing Identity Map for 0x0-0xFFFFFF

Kernel page tables at CR3=0x30dc000 only identity-map PD[8]-PD[26]:

```
PML4[0] → PDPT[0] → PD:
  PD[0]-PD[7]   = 0 (NOT MAPPED — 0x0 to 0xFFFFFF)
  PD[8]         = 0x010000e3 → phys 0x1000000 (16MB)
  PD[9]-PD[26]  = mapped (18MB-54MB, kernel image range)
```

This is **correct kernel behavior**. Linux 6.x `__startup_64()` creates dynamic
identity mapping for the **kernel image range only** (physaddr to _end), not the
full first 1GB. Boot_params at 0x10000 falls outside this range.

**Additional anomaly**: PDPT[0] and PDPT[1] both point to the same PD (0x030df063).
This should be investigated — PDPT[1] should be zero or point to a separate table.

### 2. IDT Empty Due to Unpatched FineIBT Thunk

The kernel binary uses FineIBT (Fine-grained Indirect Branch Tracking). Each function
call goes through a thunk. The thunk for `idt_setup_early_handler` at 0xffffffff8128cd20:

```
Bytes: f3 0f 1e fa 31 ff e9 a5 c6 fb ff
  ENDBR64
  XOR EDI, EDI        ← DEFAULT/UNPATCHED: passes NULL as first arg
  JMP idt_setup_from_table
```

The actual function at 0xffffffff812493d0 expects a pointer to the `early_idts` table
in RDI (saved to RSI). When RSI=NULL, it skips the IDT filling loop and only loads LIDT.

In a normal boot, `__apply_fineibt` patches these thunks BEFORE `idt_setup_early_handler`
is called. The patched thunk would load the correct `early_idts` pointer into RDI.

**In our emulator, `__apply_fineibt` either doesn't run or is a no-op**, leaving the
default thunk behavior (XOR EDI,EDI). Between CR3 switch and the IDT setup call,
only ~37 instructions execute — no call to `__apply_fineibt` is visible in the trace.

### 3. Why the Kernel Expects This to Work

On real hardware and in Bochs:
- `__apply_fineibt` patches thunks → IDT entries filled → #PF handler available
- OR the identity mapping covers boot_params → no #PF occurs

In our emulator:
- Thunks unpatched → IDT empty → no #PF handler
- Identity mapping excludes 0x10000 → #PF on boot_params access → triple fault

## Verified Raw Bytes

```
Thunk (phys 0x128cd20): f3 0f 1e fa 31 ff e9 a5 c6 fb ff ...
Function (phys 0x12493d0): f3 0f 1e fa 55 48 89 fe b9 06 00 00 00 31 c0 4c 8d 05 ...
```

Both decoded correctly by our decoder.

## Next Steps (Priority Order)

1. **Investigate `__apply_fineibt`**: Find where it should run in the boot sequence.
   Check if it's called from `startup_64` (head_64.S) before `x86_64_start_kernel`.
   If our emulator reaches it, trace what it does. If it never executes, find out why.

2. **Investigate PDPT[0]=PDPT[1] anomaly**: Both point to the same PD table (0x030df000).
   This might indicate a bug in `__startup_64()` page table construction on our emulator.

3. **Alternative: Widen identity mapping**: If FineIBT patching is too complex, investigate
   why `__startup_64` creates a narrow identity mapping. The compile-time `level2_ident_pgt`
   has all 512 entries (0-1GB). The dynamic mapping created by `__startup_64` only covers
   the kernel range. Perhaps the compile-time mapping should be preserved alongside the
   dynamic one.

4. **Alternative: Place boot_params in mapped range**: Move boot_params from 0x10000 to
   an address within PD[8] (0x1000000+). Requires ensuring no collision with kernel text.

## Files Investigated

| File | Purpose |
|------|---------|
| `rusty_box/src/cpu/cpu.rs` | Instruction trace in cpu_loop |
| `rusty_box/src/cpu/exception.rs` | Exception vector trace |
| `rusty_box/src/cpu/protected_interrupts.rs` | IDT entry content dump |
| `rusty_box/src/cpu/protect_ctrl.rs` | LIDT operation logging |
| `rusty_box/src/cpu/crregs.rs` | CR3 write logging |
| `rusty_box/examples/alpine_direct.rs` | Direct boot loader config |

## Key Addresses

| Address | What |
|---------|------|
| 0x10000 | boot_params (physical, identity-mapped) |
| 0x30dc000 | CR3 — kernel page table root (PML4) |
| 0x30df000 | Page directory for identity mapping |
| 0x1000000 | Kernel text start (physical) |
| 0xffffffff81000000 | Kernel text start (virtual) |
| 0xffffffff82a04000 | idt_table (BSS, all zeros) |
| 0xffffffff8128cd20 | FineIBT thunk for idt_setup_early_handler |
| 0xffffffff812493d0 | idt_setup_from_table (actual function) |
| 0xffffffff82ff61d0 | x86_64_start_kernel |
