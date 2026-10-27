# rusty_box_aarch64

A deterministic AArch64 (ARMv8.0-A) interpreter and a board compatible with
QEMU's `virt` machine, built the way rusty_box is: `no_std`, no `unsafe`,
time counted in retired instructions, and allocation-free unless asked to
own its memory.

It boots unmodified UEFI firmware — Ubuntu's stock EDK2 `ArmVirtQemu` build
(`/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd`) — to the UEFI Shell, with
console output up to `Shell> ` byte-for-byte what QEMU prints for the same
firmware; and it boots a Linux kernel directly, as QEMU's `-kernel` does —
Alpine Linux 3.23 (kernel 6.18) to a root shell that runs pipelines, checksums
and `apk`, reboots through PSCI, and powers off.

```text
cargo run --release -p rusty_box_aarch64 --example uefi_virt --features std -- \
    --code /usr/share/AAVMF/AAVMF_CODE.no-secboot.fd \
    --vars /usr/share/AAVMF/AAVMF_VARS.fd \
    --until 'Shell>' --type 'memmap\r' --until 'Shell>'

cargo run --release -p rusty_box_aarch64 --example linux_virt --features std -- \
    --kernel Image --initrd initramfs.cpio.gz \
    --until 'login: ' --type 'root\r' --until '# ' \
    --type 'uname -a\r' --until '# '
```

(`apt install qemu-efi-aarch64` provides the firmware.)

## Driving it from Rust

```rust,no_run
use rusty_box_aarch64::boot::{LinuxImage, SERIAL_CONSOLE_COMMAND_LINE};
use rusty_box_aarch64::{KeyboardInput, MachineBuilder, RunBudget, VirtConfig};

let kernel = std::fs::read("Image")?;
let initrd = std::fs::read("initramfs.cpio.gz")?;
let mut machine = MachineBuilder::new(VirtConfig::default())
    .linux(LinuxImage::parse(&kernel)?)
    .initrd(&initrd)
    .command_line(SERIAL_CONSOLE_COMMAND_LINE)
    .build()?;
machine.step(RunBudget::Ticks(600_000_000));
let console: Vec<u8> = machine.serial(0).map(|mut s| s.take_output().collect()).unwrap_or_default();
let typed = machine.keyboard().type_text("root\n");
assert!(typed.is_complete());
# Ok::<(), Box<dyn std::error::Error>>(())
```

[docs/aarch64-automation.md](../docs/aarch64-automation.md) is the guide:
building (firmware, direct kernel or bare; owned or lent memory), the serial
console and the terminal keyboard, budgets and stop reasons, the power button
and reset, memory and registers by name, Unicorn-style `emu_start`, the
observer type parameter, and snapshots. The budget, progress and keyboard
vocabulary are `rusty_box_core`'s, shared with the x86 machine.

## Design

- **Time is a tick count.** One tick per retired instruction, at a stated rate
  (`ips`, default 50 M/s), as Bochs keeps `ticks_total`. The generic timer's
  counter is `floor(ticks × counter_hz ÷ ips)` in `u128`; timer deadlines are
  the exact first tick whose count reaches `CVAL`; the PL031 RTC counts
  emulated seconds from a fixed epoch. Nothing reads the host clock.
- **Waiting is free and slicing is invisible.** `WFI` jumps time to the next
  event, and the processor stays in the waiting state across `step` calls, so
  a run cut into any budgets ends in exactly the same state
  (`tests/timer_interrupts.rs` asserts this, mutation-checked). Host input —
  serial bytes, keys, the power button — lands only between steps.
- **The builder's type is the boot.** `MachineBuilder<Firmware>`,
  `<DirectKernel>` and `<Bare>` build; `<NoBoot>` cannot. A machine owns its
  memory (`build`, with `alloc`) or runs in buffers a caller lends it
  (`build_in`, no allocator needed); a sealed `GuestStorage` trait is the
  difference, with static dispatch throughout.
- **Decoding is split, and the decoder is written against Arm's
  specification.** [`src/cpu/isa/`](src/cpu/isa/mod.rs) is a hand-written A64
  decoder that follows the encoding tree of Arm's A-profile Architecture
  Machine Readable Specification (AARCHMRS, `Instructions.json`: the Arm
  ARM's A64 encoding index, DDI 0487 chapter C4, as data). It has one
  function per Arm group and per encoding class, each matching that node's
  fields under Arm's names, and returns a closed enum of the encodings this
  processor implements, each named for its Arm encoding id
  (`ADD_32_addsub_imm` is `AddsubImm::Add32`). Anything else — unallocated,
  or of a feature the ID registers do not advertise — is UNDEFINED. Each
  encoding maps to a handler class; the handler reads every operand field
  from the word, following the Arm ARM, and raises UNDEFINED for the
  reserved values its decode pseudocode names. Dispatch is an exhaustive
  match over the closed class enum, so every encoding has a handler.
- **Instructions are decoded once, into traces.** The design is Bochs's
  trace cache (`cpu/icache.h`, `cpu/icache.cc`, `cpu/cpu.cc`), as
  rusty_box's x86 interpreter ports it. [`src/cpu/op.rs`](src/cpu/op.rs)
  turns a word into an `Op`: for the integer, branch, general-register
  load/store and scalar FP arithmetic forms that dominate guest code, the
  operand fields extracted
  once, with what depends on the word alone worked out ahead (a logical
  immediate's mask, a branch offset); every other word stays on its class
  handler. [`src/cpu/trace.rs`](src/cpu/trace.rs) keeps straight-line runs
  of ops, at most 32 and never across a page, keyed by the physical address
  of their first: the PC is translated, with the fetch permission check,
  once per trace, and a trace ends after a branch that always leaves, an
  exception-generating instruction, `ERET`, `WFI`/`WFE`, `ISB`, every `MSR`
  and `SYS`. A trace runs only as far as the instruction-at-a-time loop
  would without looking outside the processor — the budget, the next timed
  event, the exits — and stops early after an instruction that reached a
  device, wrote a timer register, filled or flushed the TLB, or wrote over
  cached code. Code stays coherent with memory without the guest's help:
  each page's 128-byte lines that hold cached instructions are marked
  (Bochs's page write stamps), and every store the processor makes, every
  device DMA write and every host write forgets the traces on the lines it
  touches, so `IC` maintenance has nothing left to do (as in QEMU's system
  emulation); and only RAM and a flash bank in read-array mode are cached
  at all (the bus says which, and counts the flash writes that could
  change it). Every instruction a trace runs is fetched, checked, executed
  and reported to an observer exactly as one at a time would be, so a run
  ends in the same state either way (the speed section lists the boots'
  state hashes). The one exception is the counter
  poll, below: when the observer does not hear of every instruction, whole
  iterations of a poll may be skipped, unfetched and unreported, leaving
  exactly the state running them would leave.
- **Counter polls fast-forward exactly.** When nobody watches every
  instruction, a loop that reads the system counter, compares it once with a
  value it does not change and branches on an ordering of the two jumps over
  every whole iteration that goes as the current one does: along a growing
  counter each such branch changes its outcome at most once, so a binary
  search finds the last of them. The skip stops short of the budget and of
  the next timer or device event, passes no exit address, and leaves
  iterations to run that refill the PC history. A trace that may begin such
  a loop is marked when it is built, so no other trace pays for the check
  ([`src/cpu/poll.rs`](src/cpu/poll.rs); `tests/counter_poll.rs` compares
  whole snapshots with a machine that runs every instruction).
- **Floating point is defined in software.** `src/softfloat` is rusty_box's
  own port of Berkeley SoftFloat 3e (via Bochs softfloat3e), restricted to
  half, single and double precision and given an Arm specialization: Arm's
  default NaN and NaN-propagation order (including fused multiply-add),
  FPCR.DN, FPCR.FZ
  input and output flushing, tininess before rounding, and saturating
  float-to-integer conversions. On top of it `softfloat/arm_ops.rs` adds what
  Berkeley has no counterpart for — fixed-point conversions, FCVTXN's
  round-to-odd, the FPCR.AHP alternative half-precision format, FMULX,
  FRECPS and FRSQRTS — and `cpu/fp_estimate.rs` the bit-exact
  FRECPE/FRSQRTE/URECPE/URSQRTE/FRECPX algorithms. Exceptions accumulate into
  FPSR; like a Cortex-A57, the core does not trap them. SoftFloat is the
  definition: [`src/softfloat/host.rs`](src/softfloat/host.rs) lets the host
  compute a single- or double-precision add, subtract, multiply, divide or,
  with `std` on a host whose `fma` is known to round once, fused
  multiply-add only where IEEE 754 makes the host's answer
  SoftFloat's (QEMU's "hardfloat" rule, narrowed). Rounding must be to
  nearest even with inexact already raised, every operand normal, and the
  result normal, finite and above the smallest normal.
  `tests/unit/host_float.rs` checks the host
  against SoftFloat at those boundaries under every FPCR, mutation-checked.

## Speed

`aarch64_perfbench` ([`examples/perfbench.rs`](examples/perfbench.rs))
measures the interpreter on four embedded loops and two boots; `cargo xtask
perf-baseline` archives it for A/B runs. Host: Intel Xeon @ 2.1 GHz, a
4-core VM, 2026-10-26. Each figure is the median of 10 interleaved rounds,
in millions of instructions a second; "hooked" runs under an observer with
a code hook and a memory hook. "Before" is the interpreter without a trace
cache (5028db8).

| Workload | Before | Now | |
|---|---:|---:|---:|
| `alu` | 77.2 | 204.3 | 2.65× |
| `alu`, hooked | 73.3 | 196.8 | 2.68× |
| `memcpy` (`ldp`/`stp`) | 43.2 | 101.3 | 2.34× |
| `memcpy`, hooked | 41.8 | 94.8 | 2.27× |
| `fnv` (byte loads, `mul`) | 78.3 | 172.0 | 2.20× |
| `fnv`, hooked | 75.8 | 165.0 | 2.18× |
| `fp` (`fmadd`/`fmul`/`fadd`) | 55.3 | 108.1 | 1.96× |
| `fp`, hooked | 56.8 | 110.1 | 1.94× |
| EDK2 (AAVMF) to `Shell>` | 73.6 | 237.1 | 3.22× |
| Linux 6.18 + initramfs to `login:` | 52.6 | 106.8 | 2.03× |
| Alpine 3.23 with its root on virtio-blk, logged in (`linux_virt`; one run each, M ticks/s) | 51.9 | 102.7 | 1.98× |

`fp`, plain and hooked, is the one workload short of the 2× set as the
loops' target (the boots' was 1.5×). EDK2's figure counts the instructions
of the counter polls it fast-forwards as executed, since the guest retires
them: of the 630.8 million instructions EDK2 retires in this run, 39.5%
are iterations of its delays' polls that are skipped.

On this host (an AVX-512 Xeon) a loop's ratio moves by as much as 16%
with branch placement alone: one change measured `alu` at 0.87× and
`memcpy` at 0.84× of its parent in the default build, and at 1.03× and
1.09× with both binaries built with
`-C llvm-args=-x86-branches-within-32B-boundaries`, from the same source.
Read a single row's ratio to about that precision.

The boots' state hashes at 10M, 100M and 400M instructions on this tree,
which a change meant to leave the guest's behaviour alone (as speed work
is) must reproduce, are:

| | 10M | 100M | 400M |
|---|---|---|---|
| EDK2 | `3eddc8be457d3db9` | `0eb363d26f0e7495` | `a7ab2001870414d5` |
| Linux | `200880268c21867e` | `f88d120c5c976b9f` | `785f7412a7700778` |

## What is modelled

| Part | Model |
|---|---|
| Processor | Cortex-A57-shaped ARMv8.0: EL0/EL1 AArch64 only, the whole ARMv8.0 FP and Advanced SIMD instruction set (integer and floating point, vector and scalar, saturation setting FPSR.QC), the Cortex-A57's optional Cryptographic Extension (AES, 64-bit PMULL, SHA-1, SHA-256) and CRC32, as QEMU's `cortex-a57` has them, 4 KiB-granule stage-1 MMU with TLB, exclusives, PSCI 1.1 (DEN 0022D) over `HVC`, the conduit QEMU's `virt` provides (see "Where the references differ") |
| Interrupts | GICv2 without the Security Extensions, for one processor, as IHI 0048B specifies it, in QEMU's structure: both interrupt groups, with Group 0 on FIQ under `FIQEn`, the Group 1 aliases (`GICC_AIAR`, `GICC_AEOIR`, `GICC_AHPPIR`, `GICC_ABPR`), preemption by group priority, the running priority and active priority registers, and 4 KiB distributor and 8 KiB CPU interface regions; + GICv2m frame |
| Timers | EL1 physical and virtual generic timers |
| Devices | PL011 UART (`UARTPeriphID2` revision 1, QEMU's ID, as DDI 0183F describes the revisions up to r1p4: a 16-entry receive FIFO with overrun and break status, receive interrupts at the `UARTIFLS` level, modem-status interrupts on line changes, loopback, a receiver gated by `UARTEN` and `RXE`, a transmitter gated by `UARTEN` and `TXE` whose 16-entry transmit FIFO holds what is written meanwhile, transmit interrupts as the transmit FIFO drains through the `UARTIFLS` level, and the receive timeout counted in bit periods of the programmed baud rate on the machine's clock, while characters themselves cross the line in no time; see "Where the references differ"), PL031 RTC, PL061 GPIO with QEMU's `gpio-key` power button on line 3, CFI flash ×2 (QEMU `pflash_cfi01` semantics, whole 64 MiB banks decoded), `fw_cfg` (traditional interface, no files), 32 virtio-mmio transports (QEMU's legacy layout while empty; non-legacy, as QEMU's `force-legacy=false`, once the host plugs in QEMU's 2D `virtio-gpu-device` with its EDID, `virtio-keyboard-device`, `virtio-tablet-device` or `virtio-blk-device` over a raw image in memory or a host file, which take the highest free ones), an empty PCIe ECAM/windows |
| Board | QEMU `virt` memory map and IRQ wiring; QEMU 11.1.2's device tree in `assets/` (two values pinned; see below), its memory node and `/chosen` set per machine |
| Boot | UEFI firmware from flash; or a Linux `Image`, initramfs and device tree placed as QEMU's `hw/arm/boot.c` places them, entered with the register state its boot stub leaves, and the UART's console set up at 115200 baud, 8N1, as firmware leaves it (see "Where the references differ") |

Provenance: instruction semantics follow the Arm ARM pseudocode; every device
cites the QEMU source file and symbols it was ported from. The QEMU
reference is release 11.1.2 and its newest `virt` machine, `virt-11.1`.

## Verification

The unit tests that need the library's private items live next to `src`,
in `tests/unit/`, and are compiled into the library under `#[cfg(test)]`
through `#[path]` module declarations; the files directly in `tests/` are
integration tests of the public API.

- `tests/unit/difftest.rs` replays `tests/vectors/unicorn.txt`: 24,778
  single-instruction vectors over 78 encoding families (13,791 of them
  floating point, 1,000 of them AES, SHA, PMULL and CRC32), generated by
  `tests/vectors/gen_unicorn.py` with Unicorn
  (QEMU's TCG). Each vector stores only a seed and Unicorn's results — PC,
  NZCV, FPSR and every changed register and data byte — so the test needs no
  Unicorn. Floating-point inputs mix special values (zeros, infinities,
  quiet and signalling NaNs, denormals, range and tie boundaries) with
  ordinary ones under a random FPCR (rounding mode, FZ, DN, AHP). Unicorn
  runs as QEMU's cortex-a57 model, so every vector is an instruction a
  Cortex-A57 executes, and the replay fails on any of them this core treats
  as UNDEFINED — except the 336 it is meant to: SMULH/UMULH with a
  non-ones Ra and FCMP/FCMPE with zero with a nonzero Rm (should-be fields,
  CONSTRAINED UNPREDICTABLE; see "Where the references differ"). Every
  other vector agrees. Breaking QC saturation, round-to-odd, the AHP
  conversions, a decode fix, the CRC32C polynomial, one byte of ShiftRows or
  SHA256SU1's second half on purpose makes dozens to hundreds of vectors
  fail.
- `tests/crypto.rs` runs the cryptographic and CRC32 instructions as a guest
  uses them, to the standards' own answers: AES-128 encryption and
  decryption of the FIPS-197 example, SHA-1 and SHA-256 of "abc" (FIPS
  180-4), the CRC-32 and CRC-32C check values of "123456789", 64-bit
  polynomial products; every instruction it builds is pinned to the word
  GNU as assembles; and `CPACR_EL1.FPEN` traps the cryptographic
  instructions, which are Advanced SIMD, but not CRC32, which is not.
- The decoder is checked against Arm's machine-readable specification
  (`tests/unit/spec.rs`). `syn` reads the decoder's source and follows
  `decode` through every `match` to every encoding it returns. Against
  AARCHMRS release 2026-09: the functions are Arm's groups and classes,
  called from Arm's parents; every field is read under Arm's name and at
  Arm's bits; each class names exactly the encodings of that class whose
  condition holds for the implemented features (1,061: base, FP, Advanced
  SIMD, AES, SHA-1, SHA-256, CRC32); each encoding's arms fix exactly Arm's
  fixed bits, should-be bits
  included; each carries Arm's feature. And on all 2^32 words, the decoder
  names the encoding Arm's data allocates the word to, and nothing where it
  allocates none: 0 disagreements, about 15 s on four threads. The one
  departure is PRFM (register) with `Rt = 11xxx`, which Arm's data gives to
  RPRFM (FEAT_RPRFM) and which on a core without the feature is a PRFM with
  a reserved prfop. AARCHMRS is Arm's, under the BSD 3-clause licence, and
  is not in this repository: set `RUSTY_BOX_AARCHMRS` to the unpacked
  package when building the tests (`build.rs` then enables them; without it
  they are ignored), and `cargo test --release -p rusty_box_aarch64 --lib --
  --ignored the_decoder_allocates_as_arm_does_on_every_word` runs the
  exhaustive one. The source reading itself runs with every test run.
- A second, independent check needs no Arm data: on all 2^32 words, every
  word the decoder hands to a handler that
  [`disarm64`](https://crates.io/crates/disarm64) 0.2 (generated from
  binutils' opcode table, a dev-dependency only) allocates to nothing this
  core implements is UNDEFINED when it executes — 327,827 words, where Arm's
  encodings leave a reserved value to the decode pseudocode that binutils'
  rows leave out. The exceptions are disarm64's tagging faults: the HINT
  space, which executes whatever the hint (CHKFEAT, one word), and the
  fixed-point conversions by immediate filed as FEAT_FP16 beyond `immh =
  001x` (589,824 words). `cargo test --release -p rusty_box_aarch64 --lib --
  --ignored words_disarm64_does_not_allocate_are_undefined_on_every_word`
  runs it (about 8 s on four threads); a sampled form runs with every test
  run.
- Regression tests (`tests/unit/difftest.rs`) hold the decode faults the
  decoder's checks have found: ADD/SUB (immediate) with `shift = 1x`, a
  reserved value, executed; UDF was decoded as an exception-generating
  instruction, so `udf #1` and `udf #2` behaved as SVC and HVC; the barriers
  executed with any `op1`, where only `op1 = 011` is allocated; TLBI flushed
  the TLB for the `CRm`/`op2` values of FEAT_TLBIOS and FEAT_TLBIRANGE.
- `tests/encoding_coverage.rs` runs 48 random instances of each of
  disarm64's 3,774 encodings on a bare machine and prints, per feature set,
  how many execute and how many are UNDEFINED. Every ARMv8.0 base, FP and
  Advanced SIMD encoding executes (V8 183 of 194 rows, FP 53 of 53, SIMD 454
  of 454) except the eight that are UNDEFINED at EL1 by rule (UDF, DCPS1–3,
  DRPS, HLT, SMC, SYSL) and MRS, MSR and SYS, whose random instances name
  registers and operations that do not exist and which are checked with
  ones that do; every instance of an absent feature is UNDEFINED.
  It reads disarm64's table from the cargo registry and runs with every test
  run, so an encoding gone missing from the decoder fails it without Arm's
  data.
- The EDK2 boot to the UEFI shell was compared with QEMU 11.1.2 running the
  same firmware (Debian's AAVMF), device tree and memory size, without a
  network card (`-nic none`): identical console bytes. The shell's `memmap`
  differs where this board leaves QEMU's parts out: QEMU's ACPI tables,
  which EDK2 installs from `fw_cfg` files, and allocations still present
  with `acpi=off` and fw_cfg DMA off, which this comparison does not
  attribute further.
- Alpine Linux 3.23.7 (kernel 6.18.58, Alpine's `virt` configuration) boots to
  a login prompt with no warnings in its log, logs in, and runs commands whose
  output matches the host (`seq 1 20000 | sha256sum`); `reboot` comes back to
  the login prompt and `poweroff` stops the machine.
- `tests/timer_interrupts.rs`: a hand-written guest drives GIC + timer + `WFI`
  + IRQ entry/`ERET` + PSCI; its outcome is identical for every budget
  slicing, and a snapshot taken mid-wait restores the same future.
- The virtio devices are driven register by register and ring by ring, as a
  driver drives them, by a test driver laid out like Linux's
  (`tests/unit/virtio_driver.rs`): the transport's registers, feature
  negotiation, resets, interrupt and configuration-change behaviour and
  every rule whose breach QEMU reports with `virtio_error`
  (`tests/unit/virtio_transport.rs`); QEMU's keyboard and tablet
  configuration byte for byte and the events each host action becomes
  (`tests/unit/virtio_input.rs`); every 2D GPU command and its errors in
  QEMU's order, frames and the cursor as the host receives them
  (`tests/unit/virtio_gpu.rs`). The EDID matches QEMU's own generator byte
  for byte at nine sizes (`tests/vectors/qemu_edid.txt`, from
  `tests/vectors/gen_qemu_edid.sh`, which builds `edid-generate.c` from a
  QEMU checkout). Alpine with the three devices binds `virtio_gpu` (`+edid`),
  shows its console in the framebuffer at the requested size, and receives a
  key typed on the virtio keyboard in its evdev; a snapshot taken after the
  drivers bound, restored in place or into a fresh machine, gives the same
  machine and the same picture as the uninterrupted run.
- `tests/api.rs`: the host API with guest-visible assertions — a page-crossing
  store's data abort reporting the page that faulted, the terminal keyboard's
  bytes reaching a polling guest once it has switched the UART's receiver on
  (and waiting until then), a guest's character leaving the UART only once
  it has enabled it, PSCI power-off and reset, the power button's 100 ms on
  GPIO line 3, an observer's view, direct kernel placement, its device tree
  and the console it finds set up, the receive timeout a polling guest sees
  32 bit periods after a character, or after it sets `RTIM` with one
  waiting, at the same tick however the run is sliced, and at the same
  tick after a snapshot taken once it has latched a divisor without a baud
  clock, refused builds, a snapshot of an earlier
  version refused with nothing changed; and that a run cut into instruction
  budgets of
  any size, or stopped by an exit at any instruction of straight-line code,
  ends exactly where one instruction at a time would.
- The pre-decoded ops are checked against the class handlers they stand in
  for (`tests/unit/op.rs`): 400,000 words drawn from the templates of the
  classes `op.rs` specialises, and wholly random ones, each executed both
  ways from the same random state — registers pointing into RAM or not, the
  exception level, the alignment checks, the MMU off or on over a mapping of
  random permissions — must end in the same result, processor snapshot (TLB
  included) and memory. Every specialised form runs hundreds of times.
- `tests/self_modifying.rs`: code changed after it ran runs as changed —
  rewritten just ahead of the store that rewrites it, in the same trace;
  rewritten through a second virtual mapping of its page; read over by a
  virtio-blk request the guest itself sets up; made visible with `DC CVAU`,
  `IC IVAU` and `IC IALLU`, with the MMU off and on, through either mapping
  of the page; and in flash, where the CFI Read Status command makes the
  next fetch of code that already ran read the status register, an
  UNDEFINED instruction.
- `tests/alignment.rs`: alignment faults as the guest sees them — a Q
  register access to Device memory (and with the MMU off) checked against 16
  bytes, an alignment fault on a Device page taken ahead of its permission
  fault, an access running from Normal memory into a Device page faulting
  where that page starts, and a Q register store across into an unmapped
  page writing nothing. Each fault is also taken on a page already in the
  TLB, after an aligned load filled its entry: a misaligned access to a
  Device page (an alignment fault, ahead of a store's permission fault), a
  misaligned access to Normal memory with `SCTLR_EL1.A` set or by a
  load-acquire, a store to a read-only page, and an `LDTR`/`STTR` at EL1 to
  a page EL0 cannot reach; a misaligned load from Normal memory in the TLB
  completes. A load or store pair checks each element against its own
  size, not the pair's (ARMv8.0 has no FEAT_LSE2): word, signed-word,
  single- and double-precision pairs to Device memory, and to Normal
  memory with `SCTLR_EL1.A` set, complete where each element is aligned
  and the pair is not, and fault where an element is misaligned.
- `tests/pair_access.rs`: a load or store pair acts as its two elements'
  accesses made one after the other — a store pair whose second element
  alone rewrites the next instruction runs it rewritten, a Q register
  store whose high half alone overlaps the monitored doubleword clears the
  exclusive monitor, a load pair within a page reads both elements, and a
  pair whose second element alone runs off the end of RAM, or within a
  page reaches an address nothing answers, takes that element's external
  abort after the first element's access: a load writes neither register,
  a store has made its first element's write.
- `tests/counter_poll.rs`: a counter poll fast-forwarded ends as running
  every instruction ends it — two machines, one skipping and one hearing
  of every instruction, compared snapshot for snapshot after every step at
  budget slices from 1 to 2^24 instructions: direct polls, polls through a
  call and return, through `BR`/`BLR`, with a `NOP`, on `CNTPCT_EL0`,
  comparing with a register (in either order) or an immediate on signed
  and unsigned conditions, and through the MMU; a timer interrupt during a
  poll arrives at the same instruction; an exit address inside or after a
  poll is reached by both; an EL0 poll traps where `CNTKCTL_EL1` denies
  the counter and is fast-forwarded where it allows it; and polls outside
  the rule (an equality, a 32-bit compare, a store) run every instruction.
- `tests/wfx_and_debug.rs`: WFI and WFE trap from EL0 only when they would
  wait — WFI not when an interrupt is pending, masked or not; WFE not when
  the Event Register, set by `SEV`, `SEVL`, an exception return or the
  virtual event stream, is set — and the Debug Communications Channel
  behaves as one with no debugger attached: a write fills DTRTX and sets
  TXfull for good, so a probe shaped like Linux's `hvc_dcc_check` gives up;
  a read of the receive register returns DTRRX while RXfull is set and
  clears RXfull, and `DBGDTR_EL0` reads and writes both data registers;
  `OSDTRRX_EL1` and `OSDTRTX_EL1` read and write them without touching the
  flags, with the OS Lock locked or not, and a snapshot carries them;
  `MDSCR_EL1` shows both flags, and its save/restore fields, the flags
  among them, take a write only while the OS Lock is locked; and EL0's
  accesses trap under `MDSCR_EL1.TDCC` before they take effect.
- `tests/translation_and_firmware.rs`: an out-of-range T1SZ, a table base or
  block beyond the output size `TCR_EL1.IPS` selects, `AT` meeting an
  external abort on its walk, PSCI's target, entry-point and
  affinity-level checks and the W registers of its 32-bit calls, and a
  store-exclusive to an unmonitored address failing without a fault.
- `tests/device_regions.rs`: the GIC distributor and CPU interface answer
  in their 4 KiB and 8 KiB regions, and an access to the rest of their 64
  KiB windows is a synchronous external abort.

## Where the references differ

The Arm ARM and Arm's machine-readable data (AARCHMRS) decide what an
instruction does; QEMU's `virt` board and `cortex-a57` decide the rest.
Where those references disagree, or leave the choice open, this model's
choice is listed here with its evidence.

- **Should-be fields are UNDEFINED when set otherwise.** Arm's encodings of
  34 ARMv8.0 instructions have should-be bits (`(0)` and `(1)` in the Arm
  ARM's encoding diagrams; `should_be_mask` in AARCHMRS): `Rs` and `Rt2` of
  the exclusive and ordered loads and stores (LDXR, STXR, LDXP, LDAXP, LDAR,
  STLR and the rest of `ldstexclr`, `ldstexclp` and `ldstord`), `Ra` of SMULH
  and UMULH, and `Rm` of FCMP and FCMPE with zero. A word that sets one of
  them otherwise is CONSTRAINED UNPREDICTABLE, and UNDEFINED is among the
  behaviours the Arm ARM permits; it is this model's, so the decoder is
  exactly Arm's encoding table, which fixes those bits. QEMU 11.1.2
  (`target/arm/tcg/a64.decode`) fixes them for LDAR, STLR, SMULH and UMULH
  only and executes the exclusives (`STXR`, `LDXR`, `STXP`, `LDXP`) and the
  compares with zero (`FCMP`, whose `rm` it ignores when `z` is set) as if the
  field held its should-be value. The Unicorn replay, whose QEMU fixes none
  of them, skips its vectors that set such a field.
- **RPRFM is a prefetch.** Arm's data allocates PRFM (register) with
  `Rt = 11xxx` to RPRFM (FEAT_RPRFM, Armv8.8). On a core without the feature
  the word is a PRFM with a reserved `prfop`, which the Arm ARM executes as a
  hint, as QEMU does; the decoder conformance test lists it as its one
  departure from Arm's allocation.
- **`DC ZVA` on Device memory is an alignment fault.** Arm ARM
  `AArch64.S1HasAlignmentFault` faults every `DC ZVA` to Device memory, and
  with the MMU off every data access is to Device memory. QEMU 11.1.2 zeroes
  the block instead (`target/arm/tcg/helper-a64.c` `HELPER(dc_zva)`, whose
  comment notes that it does not implement the architecturally mandated
  fault); this model takes the fault.
- **The Event Register is set by exception returns and the event stream
  too.** Arm ARM `AArch64.ExceptionReturn` sets it (`SendEventLocal`), and
  so does each event of the virtual event stream (`TestEventCNTV`, under
  `CNTKCTL_EL1.EVNTEN`). QEMU 11.1.2 sets it for `SEV` and `SEVL` only. The
  register decides whether a WFE at EL0 traps under `SCTLR_EL1.nTWE`; this
  model follows the Arm ARM. A WFE that does not consume an event completes
  at once rather than waiting, as the Arm ARM permits, so only the
  instruction count of a WFE loop differs from QEMU, whose WFE sleeps.
- **A pair or a 16-byte store across a page boundary.** Arm ARM `STP`
  makes two accesses, so a fault on the second element's page comes after
  the first element is written; QEMU 11.1.2 stores a pair of X registers as
  one 16-byte access whose pages are both translated first. This model makes
  the pseudocode's two accesses. A single 16-byte `STR Q` across a page, like
  a scalar store across one, translates both pages before writing anything,
  as QEMU does.
- **`HVC` is the firmware's conduit although EL2 is absent.** The Arm ARM
  makes `HVC` UNDEFINED when EL2 is not implemented (the `HVC` pseudocode:
  `if !HaveEL(EL2) || PSTATE.EL == EL0 ... UNDEFINED`), and
  `ID_AA64PFR0_EL1.EL2` reads 0 here. QEMU 11.1.2's `virt` acts as the
  firmware a hypervisor would be: its device tree's `/psci` node says
  `method = "hvc"`, and every `HVC` from EL1 is a PSCI call
  (target/arm/tcg/psci.c `arm_is_psci_call`, `arm_handle_psci_call`),
  whatever its immediate. Linux and EDK2 call PSCI so, as the device tree
  tells them, so this model does the same: at EL1 `HVC` is a PSCI call,
  and at EL0 it is UNDEFINED. The SMC Calling Convention (DEN 0028B §2.9)
  leaves nonzero `HVC` immediates to the hypervisor, and they are
  answered like `#0`.
- **PSCI as DEN 0022D specifies it.** The firmware reports PSCI 1.1, as
  QEMU 11.1.2 does. In four cases QEMU (target/arm/tcg/psci.c
  `arm_handle_psci_call`, target/arm/arm-powerctl.c `arm_set_cpu_on`)
  answers otherwise; this model answers as the specification says, or,
  where it leaves the choice open, as stated here:
  - The function identifier is W0 in both conventions, and an SMC32
    call's arguments are W1 to W3 (DEN 0028B §2.6 and §2.7, and §3.1
    Table 3-1, whose `Wn` is a register's low 32 bits with the upper ones
    ignored; DEN 0022D §5.2.1), as are the parameters DEN 0022D declares
    `uint32` in an SMC64 call (`lowest_affinity_level`); the upper halves
    of those X registers take no part, as in TF-A (`psci_smc_handler`).
    QEMU compares the whole of X0 and uses the whole of X1 to X3.
  - `CPU_ON` with an entry point that is not 4-byte aligned, which no
    AArch64 processor can start at, returns INVALID_ADDRESS. DEN 0022D
    §5.6.2 gives that code to an entry point the implementation knows to
    be invalid "because it is in a range that is known not to be
    available to the caller" (INVALID_PARAMETERS before PSCI 1.0).
    Misalignment is not a range condition, so the specification names no
    code for it; INVALID_ADDRESS, its code for an invalid entry point, is
    this model's choice. QEMU returns INVALID_PARAMETERS.
  - `CPU_ON` checks its target first, so an invalid target is
    INVALID_PARAMETERS whatever the entry point; then the entry point,
    then whether the processor is on (ALREADY_ON). DEN 0022D gives no
    order; this one is TF-A's (`psci_cpu_on_by_core_pos`). QEMU checks the
    entry point's alignment first.
  - `AFFINITY_INFO` above level 0 names an affinity instance, ignoring the
    fields below the level (DEN 0022D §5.1.5): one that holds this
    processor is ON, and one that does not, or a level above 3, is not
    present in the platform and gets INVALID_PARAMETERS (§5.7.2). §5.7.1
    lets PSCI 1.0 and later answer levels above 0 so or refuse them. QEMU
    answers ON for every level above 0, whatever the affinity.

  `CPU_SUSPEND` makes two choices DEN 0022D leaves open, as QEMU makes
  them. Its `power_state` is in the original format, as `PSCI_FEATURES`
  reports (§5.15.2, Table 11), and one with any of bits 31:17 set, a
  reserved bit or a PowerLevel above the core's (§5.4.2.1, Table 7), is a
  power state this platform does not provide: INVALID_PARAMETERS
  (§5.4.5). Every other request, powerdown (StateType 1) included, is
  entered as standby and returns SUCCESS at the next instruction once an
  interrupt is pending; §5.4.5 permits downgrading a powerdown request
  so.
- **The device tree's seeds and L1 data cache.** The tree in `assets/` is
  QEMU 11.1.2's `dumpdtb` output with two values replaced in place. QEMU
  draws a fresh `/chosen` `rng-seed` and `kaslr-seed` on every run; this
  board's are fixed, so runs are deterministic. QEMU 11's `cortex-a57`
  (`aarch64_aa32_a57_init` in `target/arm/cpu-max.c`) makes the L1 data
  cache 4-way, where the Cortex-A57 MPCore TRM, and QEMU 8.2, make it 2-way;
  this model's `CCSIDR_EL1` is the TRM's, and the tree's `d-cache-sets` is
  `0x100` to agree with it.
- **virtio DMA reaches RAM only.** QEMU's `dma_memory_map` maps a ring or
  buffer wherever the address space has a region (`address_space_map`):
  RAM and, for reading, a flash bank in read-array mode directly, anything
  else through its bounce buffer. This
  board's virtio devices reach guest RAM only, so a ring or buffer anywhere
  else breaks the device, as one in an unassigned hole does in QEMU. Linux
  and EDK2 place their rings and buffers in RAM.
- **`fw_cfg` without DMA or files.** QEMU's `virt` offers the DMA interface
  and a file directory (ACPI tables, SMBIOS, boot order); this board reports
  the traditional interface only (`FW_CFG_ID` = 1) and an empty directory, a
  configuration the interface defines, so firmware describes the machine
  from the device tree (`src/devices/fw_cfg.rs`).
- **The GICv2's Group 1 aliases exist.** IHI 0048B's CPU interface map
  (Table 4-2) has `GICC_AIAR`, `GICC_AEOIR` and `GICC_AHPPIR` in every
  GICv2, and with `GICC_CTLR.AckCtl` clear, the setting Arm recommends
  (§4.4.1), they are how Group 1 interrupts are acknowledged, completed and
  reported. QEMU 11.1.2 (`gic_cpu_read`, `gic_cpu_write`) reads them as zero
  and ignores writes. This model implements them as QEMU's own Non-secure
  view of `GICC_IAR`, `GICC_EOIR` and `GICC_HPPIR`: a Group 0 interrupt
  reads as 1023 and an end of interrupt naming one is ignored.
- **A disabled GIC group takes no part.** IHI 0048B §4.3.1 (`GICD_CTLR`)
  forwards a group's interrupts to the CPU interface only while
  `EnableGrpN` is set, so an interrupt the distributor does not forward is
  no candidate at the CPU interface. §4.4.1 (`GICC_CTLR`) makes the
  interface's `EnableGrpN` the enable for signalling the group's
  interrupts, and §4.4.4 has `GICC_IAR` return 1023 while signalling is
  disabled. QEMU 11.1.2 (`gic_update_internal`, `gic_acknowledge_irq`)
  chooses the highest-priority interrupt across both groups and only then
  checks its group, so a higher-priority interrupt of a disabled group
  blocks the enabled group's, and `GICC_IAR`/`GICC_HPPIR` return it. This
  model chooses among the groups both enables allow, so a disabled group's
  interrupt is neither signalled nor read and hides nothing. For the
  distributor's enable and for `GICC_IAR` that is IHI 0048B's rule. For the
  CPU interface's enable, whether a group it does not signal still hides
  the other group's is this model's reading where IHI 0048B is silent:
  neither §4.4.1 nor §4.4.4 says, and an architecture in which the
  distributor picks the highest-priority interrupt and the CPU interface
  then decides whether to signal it could be read QEMU's way. The reading
  taken is the rule the GICv3 architecture (IHI 0069) states, where
  `ICC_HPPIR0_EL1`/`ICC_HPPIR1_EL1` and `ICC_IAR0_EL1`/`ICC_IAR1_EL1`
  consider only interrupts of a group enabled by
  `ICC_IGRPEN0_EL1`/`ICC_IGRPEN1_EL1`.
- **GIC preemption compares group priorities.** IHI 0048B §3.3
  ("Preemption", "Priority grouping") signals a pending interrupt only if its
  group priority, under its own group's binary point (`GICC_BPR`, or
  `GICC_ABPR` for Group 1 while `CBPR` is clear), is higher than the
  running priority. QEMU 11.1.2 (`gic_update_internal`,
  `gic_acknowledge_irq`) compares the whole priority, which differs only
  when the two groups' binary points differ; this model compares group
  priorities.
- **SGIs on a uniprocessor GIC.** The GIC here has one CPU interface
  (`GICD_TYPER.CPUNumber` = 0). IHI 0048B gives `GICD_CPENDSGIRn` and
  `GICD_SPENDSGIRn` a bit per supported SGI and source processor, where a
  0 has no effect, and makes `GICC_IAR.CPUID` RAZ except for an SGI in a
  multiprocessor implementation (§4.4.4). QEMU 11.1.2 (`gic_dist_writeb`)
  pends the SGI on any write to its `GICD_SPENDSGIRn` byte, a byte of zero
  included (and then fails an assertion acknowledging it,
  `gic_clear_pending_sgi`), records source processors 1–7, which do not
  exist, and reports them in `GICC_IAR.CPUID`. This model keeps processor
  0's bit only: other bits read as zero and are ignored, and `CPUID` reads
  as zero.
- **The Debug Communications Channel's transmit register stays full.** The
  Arm ARM (DDI 0487, chapter H4, and the `DBGDTRTX_EL0`, `DBGDTR_EL0`,
  `DBGDTRRX_EL0`, `MDCCSR_EL0`, `MDSCR_EL1` and `EDSCR` descriptions) sets
  TXfull on every write to `DBGDTRTX_EL0` or `DBGDTR_EL0`, and only an
  external debugger's read of DTRTX clears it; a read of `DBGDTRRX_EL0` or
  `DBGDTR_EL0` returns DTRRX (and DTRTX in `DBGDTR_EL0`'s bits [63:32])
  and clears RXfull; `OSDTRRX_EL1` and `OSDTRTX_EL1` read and write DTRRX
  and DTRTX and leave the flags alone (their use while the OS Lock is
  unlocked is deprecated, not prevented); `MDSCR_EL1`'s save/restore
  fields — TXfull and RXfull, which restore the flags, and RXO, TXU,
  INTdis, TDA, HDE and ERR — are read-only while the OS Lock is unlocked
  and written while it is locked (`__set_MDSCR_EL1` in Arm's ASL). QEMU
  11.1.2 (`debug_cp_reginfo` in `target/arm/debug_helper.c`) reads
  `MDCCSR_EL0` and the four data register encodings as zero and ignores
  their writes, so a channel that nothing drains looks drained at once and
  a saved DTRRX or DTRTX restores as zero, and keeps every `MDSCR_EL1`
  field as written whatever the OS Lock. This model has no debugger either,
  and follows the Arm ARM: after the first write TXfull stays set, so
  Linux's `hvc_dcc_check` (with `CONFIG_HVC_DCC`) concludes that no channel
  is attached instead of registering a console whose output goes nowhere;
  the data registers keep what is written to them; and an unlocked write
  leaves the save/restore fields as they are. Two UNKNOWN cases have fixed
  answers: a read with RXfull clear returns zero, and a write while TXfull
  is set leaves TXfull set and both data registers as they were.
- **The PL011 receives only while `UARTEN` and `RXE` are set.** The
  PrimeCell UART (PL011) TRM, ARM DDI 0183F §3.3.8 (Note to Table 3-12),
  enables reception with both bits; the reset value of `UARTCR` (0x300) has
  `UARTEN` clear. QEMU 11.1.2 (`pl011_can_receive`, `pl011_loopback_tx`)
  keeps the receiver on whatever the bits say, for guests tested only on
  QEMU that never enable the UART. This model's receiver takes nothing
  while it is off: the host's bytes wait for it, as they wait for room in
  the FIFO, and a character looped back to it is lost. Linux enables the
  port when it opens it, and EDK2 when it initialises it.
- **A PL011 overrun keeps the FIFO and is reported.** DDI 0183F §2.4.3
  ("Overrun bit") and Tables 3-2 and 3-3: a character that arrives while the
  receive FIFO is full is lost, the FIFO's contents stay valid,
  `UARTRSR.OE` is set at once and cleared only by `UARTECR`, `OERIS` is
  raised (§3.4.5), and the next character written to the FIFO carries
  `UARTDR.OE`. QEMU 11.1.2 (`pl011_loopback_tx`, `pl011_fifo_rx_put`) puts
  a looped-back character in a full FIFO anyway: it overwrites the oldest
  unread entry and counts past the depth, so `RXFF` and `RXFE` disagree
  with what is queued, and no overrun is ever flagged; `pl011_read_rxdata`
  also replaces all of `UARTRSR` on each read. This model follows the TRM.
  A UARTRSR read returns bits [3:0] only; bits [7:4], which DDI 0183F calls
  unpredictable, read as zero.
- **A PL011 break raises `BERIS`.** DDI 0183F Table 3-2 loads one 0
  character with `BE` set, and §3.4.5 raises the break error interrupt.
  QEMU 11.1.2 (`pl011_loopback_break`) queues the character and raises only
  the receive interrupt; this model raises `BERIS` too.
- **The PL011 receive interrupt waits for the `UARTIFLS` level.**
  DDI 0183F §3.4.2 and §3.3.9: with the FIFO enabled, `RXRIS` is set when
  the fill level rises to the level `RXIFLSEL` selects (2, 4, 8, 12 or 14
  of 16 entries; 8 at reset) and cleared once reads take it below; with the
  FIFO disabled, by each character. QEMU 11.1.2 (`pl011_set_read_trigger`)
  keeps that rule under `#if 0`, with a comment that Linux worked better
  with an interrupt per character, and raises it at one entry. This model
  follows the TRM; the reserved encodings 0b101–0b111 select the half-full
  level. Linux waits for the receive timeout (below) for anything
  shorter than the level, as on hardware.
- **Emptying the PL011 FIFO through `FEN` clears its receive interrupts.**
  A change of `UARTLCR_H.FEN` empties the receive FIFO (QEMU 11.1.2
  `pl011_write`, `pl011_reset_rx_fifo`). An empty FIFO is below every
  trigger level, where DDI 0183F §3.4.2 has `RXRIS` cleared, and §3.4.4
  asserts `RTRIS` only while the FIFO is not empty. QEMU 11.1.2 leaves its
  interrupt state as it was, so `RXRIS` stays set with nothing to read.
  This model clears `RXRIS` and `RTRIS` with the FIFO.
- **The PL011 raises the receive timeout interrupt, 32 bit periods after
  the last character.** DDI 0183F §3.4.4: `RTRIS` is set when the FIFO is
  not empty and nothing has arrived for 32 bit periods, only while `RTIM`
  is set (note a to Table 3-15: the mask "acts as an enable"), and cleared
  when reads empty the FIFO or by `UARTICR`. A bit is 16 periods of
  `Baud16`, whose period is the divisor `UARTIBRD` + `UARTFBRD`/64 in
  `UARTCLK` cycles (§2.2.3, §2.4.3), so 32 bits are 512 × that divisor:
  6664 cycles of the 24 MHz `UARTCLK` at 115200 baud. The divisor counts
  from the last `UARTLCR_H` write, the strobe that updates `UARTIBRD` and
  `UARTFBRD` (§3.3.7, Note); the frame format `UARTLCR_H` sets does not
  change the bit period. QEMU 11.1.2 never sets `RTRIS`. This model counts
  on the machine's clock: `RTRIS` is set at the first tick by which 32 bit
  periods have passed since the last character arrived, or since `RTIM`
  was set with characters waiting, and the count, with the divisor it ran
  at, is part of a snapshot. Where the TRM leaves the details open: every
  character the receiver takes starts the count again, one lost to an
  overrun included; a count fires once, and the next character or `RTIM`
  set again starts another; a count keeps running while the receiver is
  disabled (`UARTEN` or `RXE` clear), since the FIFO still holds what it
  received and nothing more arrives; without a baud clock (`UARTIBRD` 0,
  invalid, or 0xFFFF with `UARTFBRD` above 0, which aborts reception,
  §3.3.6) no count starts; and a count running when a new divisor is
  latched keeps the deadline it started with, whatever that divisor, one
  without a baud clock included. That last state is saved and restored
  like any other. Characters themselves cross the line in no time,
  whatever the divisor, `UARTIBRD` 0 included: the transmitter sends the
  moment it is enabled, and the host's bytes arrive the moment the
  receiver has room, as with a QEMU character backend.
- **PL011 modem status interrupts mark changes.** DDI 0183F §3.4.1 raises
  `RIRMIS`, `CTSRMIS`, `DCDRMIS` and `DSRRMIS` when their modem status line
  changes, and only `UARTICR` clears them. QEMU 11.1.2
  (`pl011_loopback_mdmctrl`) sets each while its line is asserted and
  clears it while deasserted, on every `UARTCR` write in loopback. This
  model raises each on a change, in either direction, and leaves clearing
  to `UARTICR`.
- **PL011 reserved bits read as zero.** DDI 0183F gives `UARTLCR_H`,
  `UARTIFLS`, `UARTIMSC` and `UARTDMACR` 8, 6, 11 and 3 bits (Table 3-1)
  and calls the bits above them reserved, "read as zero" (Tables 3-10,
  3-13, 3-14 and 3-18), as it does `UARTCR` bits [6:3] (Table 3-12).
  `UARTILPR` is 8 bits wide (Table 3-1), and Table 3-5 defines only its
  bits [7:0], `ILPDVSR`, saying nothing of the rest. QEMU 11.1.2 stores
  what the guest wrote. This model keeps the defined bits only, so the
  rest of `UARTILPR` reads as zero too.
- **The PL011 transmits only while `UARTEN` and `TXE` are set, and a
  kernel booted directly finds its console set up.** DDI 0183F §3.3.8
  (Note to Table 3-12) needs both bits to transmit. A character written
  while either is clear waits in the transmit FIFO (§2.2.4: 16 bytes; the
  one-byte holding register while `UARTLCR_H.FEN` is clear), `UARTFR.BUSY`
  is set while the FIFO holds anything, "regardless of whether the UART is
  enabled or not", with `TXFE` clear and `TXFF` set once it is full (Table
  3-4); enabling transmission sends what waits, and a change of `FEN`
  flushes it (§3.3.8). QEMU 11.1.2 (`pl011_write_txdata`) logs such a
  write and transmits the character anyway, in loopback to the receiver
  too. This model follows the TRM. The TRM does not say what a write to a
  full transmit FIFO does; here the character is lost and the FIFO keeps
  what it holds.

  Reset leaves `UARTEN` clear (Table 3-1), and QEMU's direct kernel boot
  (`hw/arm/boot.c`) leaves the UART at reset, relying on its transmitting
  regardless; Linux's `earlycon=pl011` writes to the port as it finds it,
  and its `pl011_putc` waits for `BUSY` to clear after every character. So
  this board's direct boot leaves the UART as boot firmware leaves a
  console it has set up: `UARTIBRD` 13 and `UARTFBRD` 1, which are
  115200 baud from the 24 MHz `UARTCLK` the device tree declares, worked
  out as TF-A's `console_pl011_core_init` and EDK2's
  `PL011UartInitializePort` work it out (`UARTCLK × 4 / baud`, in 64ths);
  `UARTLCR_H` 0x70, eight data bits, no parity, one stop bit and the FIFOs
  enabled (TF-A's `PL011_LINE_CONTROL`, and what EDK2 writes for 8N1 with
  its FIFO); and `UARTCR` 0x301, the UART, its transmitter and its receiver
  enabled, as both write it. 115200 is the rate TF-A's QEMU platforms set
  (`PLAT_QEMU_CONSOLE_BAUDRATE`); EDK2's `ArmVirtQemu` sets 38400. Linux
  finds `UARTEN` set and takes the console's rate from those registers
  (`pl011_console_get_options`). Firmware booted from flash sets the UART
  up itself, as EDK2 does, and finds it at reset; so does code on a bare
  machine.
- **The PL011 transmit interrupt marks the FIFO draining through the
  `UARTIFLS` level.** DDI 0183F §3.3.9: the FIFO interrupts "are generated
  based on a transition through a level rather than being based on the
  level"; §3.4.3: with the FIFO enabled, `TXRIS` is set when the transmit
  FIFO reaches the `TXIFLSEL` level ("becomes <= 1/8 full" … "<= 7/8
  full": 2, 4, 8, 12 or 14 of 16 entries, 8 at reset) and cleared by
  writes that take it above, or by `UARTICR`; with the FIFO disabled, set
  when the holding register empties and cleared by a write; and enabling
  the UART and the interrupt before anything is written sets nothing.
  QEMU 11.1.2 (`pl011_write_txdata`) sets `TXRIS` after every write. This
  model follows the TRM, with characters leaving in no time: while
  transmission is enabled each character leaves as it is written, so with
  the FIFO enabled the level never rises above one entry and writes never
  set `TXRIS`; with the FIFO disabled every write fills and empties the
  holding register and sets it; and characters held back while
  transmission is off set it when enabling transmission sends more than
  the trigger level of them. Where the TRM is silent: the reserved
  `TXIFLSEL` encodings 0b101–0b111 select the half-full level, as for
  `RXIFLSEL`; a change of `TXIFLSEL` alone moves no fill level, so changes
  nothing; and the flush a change of `FEN` makes is a fall of the level
  like any other, judged against the level of the FIFO it empties. Linux's
  `pl011_tx_chars` writes until `TXFF` or until it has nothing left, and
  unmasks `TXIM` only for characters left behind a full FIFO; here the
  FIFO is never full while transmission is enabled, so its transmit path
  never waits for the interrupt.

## Not implemented

- EL2, EL3, AArch32, and every post-v8.0 feature (LSE atomics, PAuth, SVE, …):
  absent from the ID registers, so UNDEFINED, as on a real ARMv8.0 core.
  That includes the cryptographic instructions the Cortex-A57 lacks, SHA-512,
  SHA-3, SM3 and SM4 (Armv8.2 options): every QEMU CPU model that has them
  also has features this core does not (LSE, RDM, the dot product, half
  precision), so no model to take their ID values from matches this one.
- Floating-point exception traps: FPCR's trap-enable bits read as zero, as on
  a Cortex-A57, so exceptions only accumulate in FPSR.
- Debug: breakpoint and watchpoint registers exist (two of each, as
  advertised) but never fire; no Debug state, no PMU.
- No data cache model, and no instruction cache a guest can see: the
  decoded traces are kept coherent with every write, so neither `IC` nor
  data cache maintenance has anything to act on.
- No network device, and nothing on the PCIe bus: storage is the initramfs
  or the opt-in virtio-blk disk, and the display and input devices are the
  opt-in virtio ones.
- UEFI firmware draws nothing on the virtio display: `ArmVirtQemu`'s boot
  manager (`PlatformBootManagerLibLight`) puts only PCI displays on its
  console, as it does on QEMU, so the screen waits for the operating
  system's driver.
