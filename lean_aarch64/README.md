# lean_aarch64

A deterministic AArch64 interpreter written in pure Lean 4, in rusty_box's
style, checked instruction-for-instruction against `rusty_box_aarch64`.
rusty_box is the oracle: it is deterministic (time is a retired-instruction
count, runs are slice-invariant), so a second implementation can be held to
exact equality — the same retired count, registers and memory — and every
divergence bisected to one instruction.

## Status (2026-11-03)

The chained interpreter (`bench/Chain.lean`, `bench/Burst.lean`) runs rusty_box's
`aarch64_perfbench` loops bit-exactly: on `alu`, `memcpy` and `fnv` it ends in the
same registers, the same retired-instruction count and the same destination-block
and FNV hashes as rusty_box. Speed on a 2-core Xeon VM, three back-to-back rounds,
M instructions/s:

| loop | rusty_box | Lean, chained | share |
|---|---|---|---|
| `alu` | 133–146 | 66–69 | ~50% |
| `fnv` | 127–137 | 55–59 | ~43% |
| `memcpy` | 79–86 | 37–38 | ~46% |

Against rusty_box's best runs on the same host (171 / 154 / 101) the share is
38–39% on all three. Lean is built with `leanc -O3 -DNDEBUG`.

Not yet: the system level (system registers, the full stage 1 MMU, exceptions,
the generic timer), the devices other than the PL011, and so no firmware boot.
The plan and the reference data for booting EDK2 to `Shell>` are below.

## The design, after Bochs's handlers chaining

- **One self-tail-recursive function, `run`.** Every handler ends by calling `run`
  on the next op — Bochs's `BX_NEXT_INSTR` (cpu/cpu.h), a tail call into the next
  handler — and Lean compiles a self tail call to a jump, so a trace of any length
  and any number of traces run in one activation with no stack growth (Bochs bounds
  `linkTrace`'s depth because C++ does not promise the tail call).
- **Every trace ends in an `endTrace` op**, Bochs's `BxEndTrace`
  (cpu/icache.cc `genDummyTraceCacheEntry`), so no handler checks for the end of its
  trace; a trace continues over a not-taken `B.cond`, as Bochs's do.
- **Split handlers chosen at decode** for the hot encodings, and the register-31
  meaning (XZR or SP) resolved at decode into separate register-file slots, so no
  operand access branches on it.
- **The hot state is the loop's unboxed parameters**: the trace and op index, the
  trace's PC, NZCV, the instruction count, a burst limit (`lim`, rusty_box's
  `run_slice` burst: one compare per trace), a two-entry data micro-TLB and a
  one-entry code micro-TLB in front of a 64-entry TLB and the stage 1 walk. Registers
  and RAM are `ByteArray`s the loop owns, written in place; the rarely changing
  state is one `Sys` object, rebuilt only on a miss. Nothing is allocated per
  instruction.
- **Byte access with `USize` indices and a bounds lemma**
  (`ByteArray.lt_size_of_lt_usize`): with `Nat` indices every access carries a
  bignum fallback.

### What decided the speed

| step | `alu` | `fnv` | `memcpy` |
|---|---|---|---|
| a `Cpu` structure rebuilt per instruction (`bench/history`) | 15.6–18 | 6.8–8.2 | 2.8 |
| the same, C built with `-DNDEBUG` | 27.8 | 15.4 | 4.3 |
| the chained interpreter (`Chain.lean`) | 66–69 | 55–59 | 37–38 |

Pitfalls the profiles found in the structure-threading design, each invisible to
the compiler: every `{ c with … }` rebuilt the whole object and touched the
reference count of every heap field in it; RAM nested one level down
(`c.sys.mem`) was copied whole on every store; a `for` loop in a write helper
compiled to a closure holding a second reference to RAM; and the compiler floated
reads of the old state past its update, which kept it alive and copied the
register array. The chained design has none of these. Trace linking measured
0–5% at best on these loops (each loops on its own trace), so the remaining cost
is inside the handlers: about 223 host instructions per guest instruction on
`alu` and 387 on `memcpy`, led by per-byte bounds checks and byte assembly in
register and RAM access.

## Layout

| path | what |
|---|---|
| `bench/Chain.lean` | the chained interpreter and the perfbench loop setup |
| `bench/Burst.lean` | `Chain.lean` with the per-trace burst limit `lim` |
| `bench/history/` | the structure-threading design it replaced (`Arm3.lean.tmpl`, generated per register-file variant by `gen.py`) |
| `Leanbox/Dev/Pl011.lean` | the PL011 UART, ported exactly from `rusty_box_aarch64/src/devices/pl011.rs`, with 14 checks reproducing 11 of its unit tests |
| `loops_oracle/` | runs the perfbench loops on rusty_box and prints the final registers and hashes the Lean runs are compared with |
| `oracle/` | the EDK2 lockstep oracle: `dump` (device tree, start state), `check K MAX` (a state hash every K retired instructions), `detail FROM COUNT` (full registers after each instruction) |
| `oracle/ref/` | rusty_box's reference run of AAVMF to `Shell>`: `ref_1M.txt` (a checkpoint every 1M instructions; `Shell>` at 636M), `console.txt`, `dtb.bin` (the board's device tree as the firmware finds it at RAM base) |
| `edkprobe/` | the instruction mix of the EDK2 boot through the instrumentation API |

## Building and running

```bash
# Lean 4.35 (lean and leanc on PATH)
cd lean_aarch64/bench
lean -c Chain.c Chain.lean && leanc -O3 -DNDEBUG -o chain Chain.c
./chain alu 1        # also: memcpy, fnv; a scale of 0 runs a short version

lean ../Leanbox/Dev/Pl011.lean   # compiles and runs its #guard checks

# the oracles (standalone crates, outside the workspace)
cd ../loops_oracle && cargo run --release -- alu 1   # also: memcpy, fnv
cd ../oracle && cargo run --release -- edk2 check 1000000 2000000000 > ref.txt
```

`oracle` and `edkprobe` read `/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd` and
`AAVMF_VARS.fd` (`apt install qemu-efi-aarch64`).

## What EDK2 needs (measured with `edkprobe`)

AAVMF to `Shell>` retires 635,056,311 instructions (3.3 s on rusty_box): 24,656
distinct instruction words, 86 mnemonics, all base A64 integer except 0.015%
(`ld4`, `movi`, `dup`, `umov`, `addp`, `cmeq`, `scvtf`). It runs at EL1 only; the
only exceptions are 1,209 IRQs, EL1 to EL1. It reads and writes 25 system
registers, and touches the CFI flash banks (code and variables), the GICv2
distributor and CPU interface, the PL011, the PL031 and 96 accesses of virtio-mmio
probing. `subs`/`b.cond`/`mrs` are 39% of what it retires: polls of the counter,
which rusty_box fast-forwards and counts as retired.

The start state (`oracle dump`): PC 0 (flash), EL1h, DAIF all masked, the device
tree (7,500 bytes) at RAM base. The PL011 has no transmit timing in rusty_box: a
byte written while the UART and its transmitter are enabled leaves at once, so the
flag register never shows a transmitter busy over time; only the receive timeout is
timed.
