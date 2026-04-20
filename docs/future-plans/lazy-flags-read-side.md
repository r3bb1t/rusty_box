# Lazy Flags Read-Side Wiring Plan

## Status

The **write side** is done (`dbbb088`): every `set_flags_oszapc_*` and `update_flags_*` function writes to `oszapc` alongside `eflags`. The **read side** is NOT done: flag reads still go through `self.eflags.contains()`.

## Why It's Not Done Yet

Switching flag readers from `eflags` to `oszapc` requires that ALL flag-writing instructions update `oszapc`. Currently ~100+ sites write arithmetic flags directly to `eflags` without going through `oszapc`:

- Shifts/rotates: set CF, OF directly (`shift8.rs`, `shift16.rs`, `shift32.rs`)
- BT/BTS/BTR/BTC: set CF directly (`bit16.rs`, `bit32.rs`, `bit64.rs`)
- MUL/IMUL: override CF+OF after logic flags (`mult8/16/32/64.rs`)
- BMI: set CF, ZF, SF, OF individually (`bmi32.rs`, `bmi64.rs`)
- BCD (DAA/DAS/AAA/AAS): set CF, AF (`bcd.rs`)
- RDRAND: clear OSZAPC then set CF (`rdrand.rs`)
- SSE compares (COMISS/UCOMISS): set ZF, PF, CF (`sse_pfp.rs`)
- SSE string (PCMPISTRI/PCMPISTRM): set CF, SF, ZF, OF (`sse_string.rs`)
- SSE PTEST: set ZF, CF (`sse.rs`)
- AVX-512 mask ops (KORTEST/KTEST): set ZF, CF (`avx512_mask.rs`)
- FPU compares (FCOMI/FUCOMI): set CF, ZF, PF (`fpu/ferr.rs`)
- VERR/VERW/LAR/LSL/ARPL: set ZF (`protect_ctrl.rs`)
- CET CLRSSBSY: clear OSZAPC then set CF (`cet.rs`)
- Flag control: CLC/STC/CMC set CF, SALC reads CF (`flag_ctrl.rs`)

## The set_oszap Problem

INC/DEC use `set_oszap` which preserves CF by reading the old CF from `oszapc.auxbits`. This breaks when CF was set directly on `eflags` by a non-wired instruction (e.g., a shift before an INC). The `set_oszap` calls were removed in the current commit. They can only be re-added once ALL CF writers go through `oszapc`.

## Approach

### Phase 1: Wire ALL remaining flag writers to oszapc

Every `self.eflags.insert/remove/set(EFlags::<arith>)` must be replaced with `self.oszapc.set_<flag>(val)` (or `self.set_<flag>(val)` via CPU wrapper methods).

Key patterns:
- `self.eflags.insert(EFlags::CF)` → `self.set_cf(true)`
- `self.eflags.remove(EFlags::CF)` → `self.set_cf(false)`
- `self.eflags.set(EFlags::CF, val)` → `self.set_cf(val)`
- `self.eflags.toggle(EFlags::CF)` → `self.set_cf(!self.get_cf())`
- `self.eflags.remove(EFlags::OSZAPC)` followed by individual sets → unconditional `self.set_cf(cf); self.set_pf(pf);` etc. (the clear is implicit since each setter is unconditional)
- `self.eflags.insert(EFlags::CF.union(EFlags::OF))` → `self.set_cf(true); self.set_of(true)`

**Critical**: when a block had `eflags.remove(OSZAPC)` + conditional `if flag { insert }`, the remove provided the false-by-default. With oszapc setters, each flag must be set unconditionally: `self.set_cf(cf)` not `if cf { self.set_cf(true) }`.

System flags (IF, DF, TF, VM, NT, RF, VIF, VIP, ID, AC, IOPL) stay on `eflags` — they are never lazy.

### Phase 2: Re-add set_oszap calls for INC/DEC

Once all CF writers go through oszapc, re-add:
- `logical16.rs` set_flags_oszap_inc_16/dec_16: `self.oszapc.set_oszap_add_16(op1, 1, result)`
- `arith64.rs` update_flags_oszap_add64/sub64: `self.oszapc.set_oszap_add_64(op1, op2, res)`
- Add `set_oszap_add_32`/`set_oszap_sub_32` to `lazy_flags.rs` for 32-bit INC/DEC

### Phase 3: Switch flag readers to oszapc

Change the 6 getters in `ctrl_xfer32.rs`:
```rust
pub fn get_cf(&self) -> bool { self.oszapc.getb_cf() != 0 }
// ... same for zf, sf, of, pf, af
```

Also fix direct `eflags.contains()` reads of arithmetic flags:
- `bmi32.rs`, `bmi64.rs`: `eflags.contains(EFlags::CF/OF)` → `get_cf()/get_of()`
- `flag_ctrl.rs` SALC: `eflags.contains(EFlags::CF)` → `get_cf()`

### Phase 4: Strip eager eflags writes from helper functions

Once readers use oszapc, the eager eflags writes in `set_flags_oszapc_*` and `update_flags_*` functions are dead code. Strip them, keeping only the oszapc call:
```rust
pub fn set_flags_oszapc_logic_8(&mut self, result: u8) {
    self.oszapc.set_oszapc_logic_8(result);
}
```

### Phase 5: Materialization and sync points

**Materialization** (`force_flags`): Before any code that reads the full `eflags` value, materialize oszapc into eflags:
- PUSHF/PUSHFD/PUSHFQ, LAHF
- Interrupt delivery (push eflags on stack)
- SYSCALL (save rflags)
- Task switch, SVM VMRUN/VMEXIT, FRED
- Snapshot save, API bridge (`rflags_for_api`)

**Sync** (`set_eflags_oszapc`): After any code that writes raw eflags from a u32:
- `set_eflags_internal` (called by POPF, IRET)
- SAHF
- `set_rflags_for_api`
- CPU init/reset, snapshot restore

For `&self` contexts (API reads, snapshots), use `eflags_materialized()` which computes without mutation.

## CPU Methods Needed (add to cpu.rs)

```rust
// Flag reads
#[inline] pub(super) fn getb_cf(&self) -> u32 { self.oszapc.getb_cf() }
// ... same for pf, af, zf, sf, of

// Flag writes  
#[inline] pub(super) fn set_cf(&mut self, val: bool) { self.oszapc.set_cf(val) }
// ... same for pf, af, zf, sf, of

// Materialize oszapc → eflags
pub(super) fn force_flags(&mut self) { ... }

// Read eflags with materialization
pub(crate) fn read_eflags(&mut self) -> u32 { self.force_flags(); self.eflags.bits() }

// Non-mutating materialization for &self contexts
pub(crate) fn eflags_materialized(&self) -> u32 { ... }

// Sync eflags → oszapc (after raw eflags write)
pub(super) fn set_eflags_oszapc(&mut self, flags32: u32) { ... }
```

## Verification

1. Add `assert_oszapc_matches_eflags()` debug assertions after every oszapc write during development
2. `cargo check` all feature combos + UEFI target
3. `cargo test -p rusty_box --lib` — 187 tests pass
4. Alpine Linux boots to login prompt via egui example
5. DLX Linux boots to shell

## Files (complete list)

Phase 1 touches ~30 files. See `git grep 'self.eflags.(insert|remove|set).*EFlags::(CF|PF|AF|ZF|SF|OF)'` for the full list.
