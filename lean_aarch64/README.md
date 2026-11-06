# lean_aarch64

A deterministic AArch64 interpreter written in pure Lean 4, in rusty_box's
style, checked instruction-for-instruction against `rusty_box_aarch64`.
rusty_box is the oracle: it is deterministic (time is a retired-instruction
count, runs are slice-invariant), so a second implementation can be held to
exact equality — the same retired count, registers and memory — and every
divergence bisected to one instruction.

## Status (2026-11-03)

**Alpine Linux 3.23 boots, logs in and runs a shell in pure Lean,
lockstep-clean**: the kernel (6.18.58-0-virt, booted directly) and an
initramfs of Alpine's minirootfs (`tools/mkinitramfs.py`: busybox init, a
getty on ttyAMA0, root's password emptied) run to `(none) login: `, take
`root` typed on the serial port (board.rs `send_serial` and the pump after
UART accesses, ported), and reach `(none):~# ` at 548M retired instructions,
with the same 548 checkpoints as `oracle/ref/linux_shell_1M.txt` and the same
12,225 console bytes. On the way the guest takes about 10,000 IRQs, its
system calls and its page faults, and runs the kernel's SHA-256 self-tests on
the Cryptographic Extension. Speed, same host: rusty_box 6.2 s, Lean 22.1 s
(25 M instructions/s). In that shell `uname -a`, `cat /etc/alpine-release`,
`ls /` and `echo $((6*7))` run identically to rusty_box; `seq` stops the
machine at its first floating-point instruction (`FMOV d15, #imm`), FP
arithmetic not being ported yet.

**AAVMF (EDK2) boots to `Shell>` in pure Lean, lockstep-clean against
rusty_box**: `leanbox check 1000000 700000000` prints the same 636 checkpoints
as `oracle/ref/ref_1M.txt` (retired count, tick count, PC and the state hash
over x0..x30, SP_EL0, SP_EL1, NZCV and DAIF, every million instructions) and
the same console, byte for byte. Speed, same host (4-core VM):

| | to `Shell>` | |
|---|---|---|
| rusty_box (`oracle edk2 check`) | 3.1 s | |
| Lean (`leanbox edk2 check`) | 11.5 s | 55 M instructions/s, counting the polls it skips |

The machine (`Leanbox/`): the A64 base integer ISA as rusty_box decodes it for a
Cortex-A57 (strict should-be bits, everything else UNDEFINED), CRC32, the system
registers and system instructions of an EL1-only processor, the stage 1 MMU
(4 KiB granule, levels 0–3, TTBR0/1, permissions, TBI, MMU off), the TLB,
exceptions (synchronous, IRQ, FIQ, ERET, illegal return), the exclusive monitor,
PSCI over HVC, the generic timer and event stream, WFI/WFE, and the board:
GICv2, PL011, PL031, PL061, both CFI flash banks, the empty virtio-mmio
transports and the PCI windows. The run loop is machine.rs `run_slice`'s:
bursts to the next event, `sync_events`, `after_device_access`, interrupts
between traces, WFI sleeping to the next event; and poll.rs's exact
fast-forward of counter polls (`Engine/Poll.lean`): at a counter read that
starts a trace, one iteration is walked (only NOPs, one 64-bit compare of the
counter register, ordering B.conds, B/BL/BR/BLR/RET, all fetched from TLB
hits on stable memory, x30 periodic) and the iterations that take its path
and fit in the burst are skipped, leaving the counter register, NZCV and the
tick count as running them would.

**Single instructions, against rusty_box**: `tools/difftest_gen.py` writes random
words of one encoding group with random start states (registers half the time
pointing into a scratch window), `oracle bare` and `leanbox bare` run each alone
(EL1h, MMU off, so loads and stores reach Device memory) and print the state
after it: registers, NZCV, PC, all SIMD&FP registers, ESR/FAR/ELR/SPSR, FPSR
and a hash of the window. 80,000 words of the data-processing, load/store and
branch groups (about 60% of them raising an exception) give identical lines.
Of 5,000 SIMD&FP data-processing words, the 64 this machine executes match and
the other 4,936 stop it as unmodelled. The tester found one bug the boots had
not: a 128-bit access's Device-memory alignment is judged by its 16 bytes, and
a pair of Q registers at an 8-byte-aligned address is four doubleword accesses
(mmu.rs `read_mem_u128`, `read_mem_q_pair`, `write_mem_q_pair`).

```bash
python3 -I tools/difftest_gen.py ldst 20000 7 > t.txt    # dpimm dpreg ldst branch simd all
oracle/target/release/oracle bare t.txt > rusty.txt
./.lake/build/bin/leanbox bare t.txt > lean.txt && diff rusty.txt lean.txt
```

Not modelled yet, each stopping the machine with a message rather than being
taken for something else: `AT` (address translation), armed breakpoints,
watchpoints and software step, and the SIMD&FP data processing outside the
set these boots execute (`Leanbox/Cpu/Simd.lean`: modified immediate, the
copies, integer three-same and compare-with-zero, `REV16/32/64`, `ADDP`,
`SSHLL`/`USHLL`, `PMULL`/`PMULL2`, `SHA256H`/`H2`/`SU0`/`SU1`,
`SCVTF`/`UCVTF`, `FMOV` general): no FP arithmetic yet (rusty_box's
softfloat is not ported), no AES or SHA-1.

### The perfbench loops

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

### What decided the speed of the machine

On the boots, measured as host instructions per guest instruction with
callgrind (Alpine to its first 40M instructions):

| step | host/guest | EDK2 to `Shell>` | Alpine to `login:` |
|---|---|---|---|
| every op but a few ALU ones through `execOp` (an `St` allocated per op) | 535 | 40.6 s | 33.4 s |
| loads and stores on data micro-TLBs, refilled from the TLB without allocating | | 34.6 s | 30.9 s |
| every pure ALU op in `run` (`Isa/Alu.lean`, shared with `execOp`) | | 24.6 s | 23.6 s |
| the common trace lookup inline in `run` (no `Next` allocated) | 394 | 15.2 s | 20.9 s |
| counter polls fast-forwarded exactly (`Engine/Poll.lean`, poll.rs's rule) | | 11.5 s | 20.9 s |
| a word in bounds read under one check, its bytes proven in bounds (`ByteArray.WordIn`) | 300 | | |
| CLZ, CLS, RBIT, REV16, REV32, REV computed loop-free, proven equal to their pseudocode (`@[csimp]`, `bv_decide`: proofs that trust the compiler, `docs/type-doctrine.md` L0) | 283 | | |
| the TLB hit and the translation inline in their callers, so their pairs and `Except`s are never built (the walk stays out of line) | 246 | | |
| `DC ZVA` to RAM one `copySlice` of a zero block; the eight micro-TLB words one `MicroTlb` structure in the loop | 242 | | |
| the op read and matched in the branch that proves its index, so it is a borrow of the trace (no reference count per op) | 232 | | |
| a second code page in the micro-TLB, so a branch back to the page it came from stays on the inline trace lookup | 191 | | |

The last step, measured on a slower 4-core VM, best of three, the timed loop
only: Alpine to its root shell 23.3 → 30.2 M instructions/s, EDK2 to `Shell>`
46.7 → 62.0 M (rusty_box on the same VM: 97.3 M and 209.7 M). One 8-byte
access per word through a C helper, tried to measure the ceiling and not kept
(it would not be pure Lean), reached 268 host instructions per guest
instruction, 33.5 M and 68.1 M: the proof recovers about 70% of that, the
rest being the stores, whose bytes the C compiler cannot join because the
runtime checks the array's exclusivity before each.

The accesses are written as Lean writes array indexing, `m[i]`, `m[i+7]`,
`m.uset i v`, with no proof terms in the code: `get_elem_tactic`, Lean's
tactic for those side conditions, is extended (`macro_rules` on
`get_elem_tactic_extensible`, `Bytes.lean`) to find them from the check in
context. The bit-counting and byte-reversing ops are defined as the Arm
ARM's pseudocode loops (structural recursion over the bits, which a proof
can unfold, where a `while` loop is opaque) and run as their loop-free forms:
a `@[csimp]` theorem per op, proved by `bv_decide` for all 2^64 inputs of
both widths, makes the compiler substitute the fast form. Over Alpine's first
40M instructions, where the kernel counts bits often, that is 6% of the host
instructions; over a whole boot it is within the timing noise.

A function returning a pair or an `Except` allocates both when it is called
out of line; inlined, the compiler's simplifier takes each apart where it is
built (a `match` on a constructor it can see, and LCNF's `JpCases` for one
reaching a join point), and nothing is allocated. `tlbEntry`, `translate`
and `fetchTranslate` keep those types and are inline, with the walk on a miss
(`tlbFill`) out of line: about 11M allocations in the 40M-instruction window
went to none, 283 → 246 host instructions per guest instruction, and,
interleaved runs on the same VM, Alpine 29.5–31.4 → 34.9–36.1 M
instructions/s and EDK2 60.4–63.5 → 66.7–67.9 M (rusty_box 91–95 M and
219–227 M).

An element read from an array is a borrow, with no reference count taken,
as long as the array outlives it (LCNF's `ExplicitRC` derived borrows). `run`
read its op with a default for an index past the end (`agu`), and the two
values met at a join point whose parameter is owned, so every op was
incremented and decremented; matched where `i < tr.usize` is in context, the
op is `tr[i]` and stays a borrow. The kernel's calls and returns cross pages
constantly, and with one code page cached each crossing missed the inline
lookup: with two, the out-of-line `lookup` (and the `Next` it allocates) ran
for 1.6M of the window's traces before and a fifth of that after,
232 → 191 host instructions per guest instruction.

What remains in `run` is mostly the C compiler's: `run` is one function with
a value for each loop parameter live across every op, and calls in its slow
arms, so most of those values live in its stack frame and each op reloads
them: about 28% of the instructions `run` executes are loads and stores to
its frame. Folding the eight micro-TLB words into one structure took that
from 32%. Inlining `lookup` into `run` (to drop its `Next`) made `run`'s code
much worse, 9.84G → 15.3G host instructions over the window, and was not
kept.

A register file of `FloatArray` words (one unboxed element per register,
`Float.ofBits`/`toBits` as the casts) was tried and dropped: Lean canonicalizes
a NaN's payload through `Float.ofBits`, so a register holding a NaN-shaped bit
pattern changes, and the boot diverged at once. The register file stays a
`ByteArray`: an 8-byte read is one bounds check and one load, an 8-byte store
one bounds check and eight byte stores, each after an exclusivity check.
Lean has no wider write; `docs/bytearray-word-access.md` is the proposal for
one in Lean core, and the pure-Lean work that does not wait on it.

### What decided the speed of the perfbench loops

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

The library reads in the order of `Leanbox.lean`'s imports, each part resting
only on the parts before it; that module's documentation is the table of
contents. `docs/type-doctrine.md` is the type doctrine (L0–L18) every module is written to; `docs/semantics.md` is how semantic correctness is stated and proved, `docs/hooks.md` the hooks whose conditions are types, and `docs/proofs.md` lists what is proved, checked and open.

| path | what |
|---|---|
| `Leanbox.lean` | the whole library, in reading order, with its table of contents |
| `Leanbox/Bytes.lean` | words read from and written to a `ByteArray`, every index proven in bounds |
| `Leanbox/Isa/` | the instruction set before any machine: the Arm ARM's shared pseudocode (`Pseudocode`), `Op` (`Op`), the decoder (`Decode`), the data-processing values shared by `execOp` and `run`, with the fast bit operations proven equal to their definitions (`Alu`) |
| `Leanbox/Dev/` | GICv2, PL011, PL031, PL061, CFI flash, the empty virtio-mmio transport |
| `Leanbox/Board/` | the memory map (`Map`), the board and its bus outside RAM (`Basic`) |
| `Leanbox/Cpu/` | the processor: its state (`State`), the generic timer and event register (`Timer`), stage 1 translation (`Mmu`), exceptions and retiring (`Exception`), data accesses (`Memory`), `MRS`/`MSR` (`Sysreg`), PSCI and the system instructions (`System`), SIMD&FP (`Simd`), and `execOp`, every op completely, the reference the fast paths agree with (`Step`) |
| `Leanbox/Engine/` | the fast engine: traces and their lookup (`Trace`), the exact fast-forward of counter polls (`Poll`), the chained interpreter `run` (`Run`), the run loop (`Machine`) |
| `Leanbox/Boot.lean` | the start states: the firmware boot and the direct kernel boot |
| `Leanbox/Lockstep.lean` | the oracle's checkpoint, per-instruction and single-instruction lines, and the console script |
| `Main.lean` | `leanbox edk2|linux check K MAX [script]` / `detail FROM COUNT` / `bare TESTS` |
| `LeanboxTest/` | each device's Rust unit tests as `#guard`s (`lake test`) |
| `bench/Chain.lean` | the chained interpreter's prototype on the perfbench loops |
| `bench/Burst.lean` | `Chain.lean` with the per-trace burst limit `lim` |
| `bench/history/` | the structure-threading design it replaced (`Arm3.lean.tmpl`, generated per register-file variant by `gen.py`) |
| `loops_oracle/` | runs the perfbench loops on rusty_box and prints the final registers and hashes the Lean runs are compared with |
| `oracle/` | the lockstep oracle: `edk2`/`linux` `dump` (device tree, start state), `check K MAX [UNTIL]` (a state hash every K retired instructions), `detail FROM COUNT` (full registers after each instruction) |
| `oracle/ref/` | `linux_dtb.bin` and `linux_first_1M.txt`: the device tree rusty_box gives the kernel with the netboot initramfs, and its reference run to the first console line; `linux_shell_dtb.bin` and `linux_shell_1M.txt`: the same with `tools/mkinitramfs.py`'s initramfs, through the login to `(none):~# `; rusty_box's reference run of AAVMF to `Shell>`: `ref_1M.txt` (a checkpoint every 1M instructions; `Shell>` at 636M), `console.txt`, `dtb.bin` (the board's device tree as the firmware finds it at RAM base) |
| `tools/mkinitramfs.py` | Alpine's minirootfs as an initramfs that ends at `login:`, root without a password |
| `tools/difftest_gen.py` | random single-instruction tests for `oracle bare` / `leanbox bare` |
| `edkprobe/` | the instruction mix of the EDK2 boot through the instrumentation API |

## Building and running

```bash
# Lean 4.35 (lean, leanc and lake on PATH)
cd lean_aarch64 && lake build          # leanc -O3 -DNDEBUG, from lakefile.toml
lake test                              # every device's #guard checks
./.lake/build/bin/leanbox edk2 check 1000000 700000000 > lean.txt   # to Shell>, writes console.txt
diff oracle/ref/ref_1M.txt lean.txt
./.lake/build/bin/leanbox edk2 detail 5000000 20   # registers after each of 20 instructions

# Alpine 3.23: the netboot vmlinuz-virt is an EFI zboot image; its gzip
# payload (offset and size at bytes 8 and 12) is the arm64 Image.
LINUX_IMAGE=Image LINUX_INITRD=initramfs-virt LINUX_DTB=oracle/ref/linux_dtb.bin \
  ./.lake/build/bin/leanbox linux check 1000000 400000000 "Booting Linux"
(cd oracle && LINUX_IMAGE=... LINUX_INITRD=... cargo run --release -- linux check 1000000 400000000 "Booting Linux")

# to a root shell: the minirootfs as an initramfs, and a console script
# (until:TEXT waits for TEXT after the previous item, type:TEXT sends it to the
# UART, \r is a carriage return; acted on after every checkpoint, the same way
# by the oracle and by leanbox)
python3 -I tools/mkinitramfs.py alpine-minirootfs-3.23.7-aarch64.tar.gz initramfs.cpio.gz
LINUX_IMAGE=Image LINUX_INITRD=initramfs.cpio.gz LINUX_DTB=oracle/ref/linux_shell_dtb.bin \
  ./.lake/build/bin/leanbox linux check 1000000 3000000000 "until:login: " 'type:root\r' "until:# "
diff oracle/ref/linux_shell_1M.txt <the output>
# every SIMD&FP, system and exception-generating word a boot executes, with counts
(cd oracle && LINUX_INITRD=... cargo run --release -- linux probe "login: ")

cd bench
lean -c Chain.c Chain.lean && leanc -O3 -DNDEBUG -o chain Chain.c
./chain alu 1        # also: memcpy, fnv; a scale of 0 runs a short version


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
