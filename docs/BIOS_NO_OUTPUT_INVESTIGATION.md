# BIOS No Output Investigation (2026-02-16)

## Problem
The emulator runs 50M instructions with BIOS-bochs-legacy but produces zero output:
no VGA text, no POST codes, no debug port output. Final RIP=0xC93 in real-mode low memory.

## Root Cause Found: `execute1` Bug in `MovGwEw` (opcodes_table.rs)

### The Bug
In `rusty_box/src/cpu/opcodes_table.rs`, the `MovGwEw` opcode entry has **both** `execute1` (memory form)
and `execute2` (register form) pointing to the **register** handler:

```rust
Opcode::MovGwEw => Some(BxOpcodeEntry {
    execute1: mov_gw_ew_r_wrapper,        // BUG: Should be memory form!
    execute2: Some(mov_gw_ew_r_wrapper),  // Correct: register form
    opflags: OpFlags::empty(),
}),
```

### Impact
When `MOV DX, [BP+4]` executes with a memory operand (mod != 11), the CPU incorrectly calls
`mov_gw_ew_r` (register form) instead of `mov_gw_ew_m` (memory form). This means:

- `MOV DX, [BP+4]` actually executes as `MOV DX, SI` (rm=6 → register SI)
- The BIOS `inb(port)` helper at 0x4F0 reads the port parameter from the stack via `[BP+4]`
- Instead of reading 0x0064 (keyboard status port), DX gets whatever value SI contains
- SI happens to contain 0xFF19 (and decrements), causing the BIOS to read garbage ports
- All I/O port reads return 0xFF (unhandled), so the BIOS spins endlessly

### Verification Chain
1. BIOS sets SP=0xFFFE, SS=0 at 0xE0A3-0xE0AD
2. BIOS calls keyboard init at `call 0xC57` (from 0xE134)
3. Keyboard init calls `inb(0x64)` via `push 0x64; call 0x4F0`
4. At 0x4F0 (inb helper): `push bp; mov bp, sp; push dx; mov dx, [bp+4]; in al, dx`
5. Stack dump at [BP+4] = 0xFFF6 correctly shows 0x0064
6. But DX = 0xFF19 (SI register value) because `mov dx, [bp+4]` uses register form

### Evidence
```
IN AL, DX: port=0xff19 -> 0xff at RIP=0x4f8 SP=0xfff0 BP=0xfff2 [instr #9003]
  [BP+0] @ 0xfff2 = 0xfffa   (old BP)
  [BP+2] @ 0xfff4 = 0x0c78   (return address from call 0x4F0 at 0xC75)
  [BP+4] @ 0xfff6 = 0x0064   (port parameter - CORRECT in memory!)
  [BP+6] @ 0xfff8 = 0xd82b
  [BP+8] @ 0xfffa = 0x0000
```

### Fix Applied (2026-02-16)

**Root fix in `opcodes_table.rs`:** Changed `execute1` (memory form) for all affected
opcode entries to use the correct memory-form handler, matching Bochs `ia_opcodes.def`.

**18 entries fixed** (all verified against `cpp_orig/bochs/cpu/decoder/ia_opcodes.def`):

| Opcode | execute1 (was) | execute1 (fixed to) |
|--------|---------------|---------------------|
| `MovOp32GdEd` | `mov_gd_ed_r_wrapper` | `mov_gd_ed_m_wrapper` |
| `MovOp32EdGd` | `mov_ed_gd_r_wrapper` | `mov_ed_gd_m_wrapper` |
| `MovEdId` | `mov_ed_id_r_wrapper` | `mov_ed_id_m_wrapper` |
| `MovGwEw` | `mov_gw_ew_r_wrapper` | `mov_gw_ew_m_wrapper` |
| `MovEwGw` | `mov_ew_gw_r_wrapper` | `mov_ew_gw_m_wrapper` |
| `MovEwIw` | `mov_rw_iw_wrapper` | `mov_ew_iw_m_wrapper` |
| `AddGdEd` | `add_gd_ed_r_wrapper` | `add_gd_ed_m_wrapper` |
| `AddEdGd` | `add_ed_gd_r_wrapper` | `add_ed_gd_m_wrapper` (+LOCKABLE) |
| `AddEwsIb` | `add_ew_ib_r_wrapper` | `add_ew_ib_m_wrapper` |
| `SubGdEd` | `sub_gd_ed_r_wrapper` | `sub_gd_ed_m_wrapper` |
| `SubEdGd` | `sub_ed_gd_r_wrapper` | `sub_ed_gd_m_wrapper` (+LOCKABLE) |
| `CmpGbEb` | `cmp_gb_eb_r_wrapper` | `cmp_gb_eb_m_wrapper` |
| `CmpEbGb` | `cmp_gb_eb_r_wrapper` | `cmp_eb_gb_m_wrapper` |
| `CmpGwEw` | `cmp_gw_ew_r_wrapper` | `cmp_gw_ew_m_wrapper` |
| `CmpGdEd` | `cmp_gd_ed_r_wrapper` | `cmp_gd_ed_m_wrapper` |
| `CmpEwIw` | `cmp_ew_iw_r_wrapper` | `cmp_ew_iw_m_wrapper` |
| `CmpEdId` | `cmp_ed_id_r_wrapper` | `cmp_ed_id_m_wrapper` |

**New function created:** `ADD_EwIbM` in `arith16.rs` (memory form for ADD r/m16, imm8).

**Cleanup:** Removed investigation debug logging from `io.rs`, `data_xfer_ext.rs`.

### How the BIOS Gets Stuck (before fix)
1. `inb(0x64)` reads port 0xFF19 instead of 0x64 → returns 0xFF
2. Status byte 0xFF has bit 1 set (input buffer full) → BIOS loops 0xFFFF times
3. `outb(0x80, 0x00)` also uses the outb helper which has the same register/memory bug
4. Port number for outb also comes from SI (decrementing) → writes to garbage ports
5. BIOS eventually times out, sends self-test command 0xAA to wrong port, repeats
6. Loop burns millions of instructions without making progress
7. Keyboard never initializes → BIOS never reaches VGA or output code

### Reference: Original Bochs C++ Code
In Bochs `ia_opcodes.def`, each opcode entry has two handler function pointers:
```
bx_define_opcode(BX_IA_MOV_GwEw, ..., &BX_CPU_C::MOV_GwEwM, &BX_CPU_C::MOV_GwEwR, ...)
```
- First pointer = `execute1` = memory form handler
- Second pointer = `execute2` = register form handler

The dispatch in `fetchdecode32.cc` (line 2046/2059) selects execute1 for memory
operands (modC0==false) and execute2 for register operands (modC0==true).
