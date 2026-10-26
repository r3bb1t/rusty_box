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
  ends in the same state either way: the equivalence hashes in the speed
  table below are equal before and after. The one exception is the counter
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
| `alu` | 73.8 | 163.2 | 2.21× |
| `alu`, hooked | 70.5 | 149.9 | 2.13× |
| `memcpy` (`ldp`/`stp`) | 41.6 | 67.3 | 1.62× |
| `memcpy`, hooked | 39.7 | 68.5 | 1.73× |
| `fnv` (byte loads, `mul`) | 79.9 | 123.9 | 1.55× |
| `fnv`, hooked | 75.2 | 123.4 | 1.64× |
| `fp` (`fmadd`/`fmul`/`fadd`) | 55.2 | 81.3 | 1.47× |
| `fp`, hooked | 55.3 | 78.3 | 1.42× |
| EDK2 (AAVMF) to `Shell>` | 69.7 | 100.4 | 1.44× |
| Linux 6.18 + initramfs to `login:` | 50.9 | 76.8 | 1.51× |
| Alpine 3.23 with its root on virtio-blk, logged in (`linux_virt`; one run each, M ticks/s) | 50.1 | 77.6 | 1.55× |

On this host one loop's figure moves by up to 13% with code layout alone:
a change to code no loop executes measured `alu`, hooked, at 0.90× and
`memcpy` at 0.87× of its parent. Read a single row's ratio to about that
precision; `fnv` measured 1.87× on an earlier build of the same design.

Every run ends in the same state before and after; for the boots, the state
hashes at 10M, 100M and 400M instructions are:

| | 10M | 100M | 400M |
|---|---|---|---|
| EDK2 | `9cf8cd4686ce4463` | `e3e98f8e2c20afc7` | `cd35ddd4435e80fd` |
| Linux | `55803c9740c289d7` | `f573c06f44a5c887` | `13aa27f25afc0dea` |

## What is modelled

| Part | Model |
|---|---|
| Processor | Cortex-A57-shaped ARMv8.0: EL0/EL1 AArch64 only, the whole ARMv8.0 FP and Advanced SIMD instruction set (integer and floating point, vector and scalar, saturation setting FPSR.QC), the Cortex-A57's optional Cryptographic Extension (AES, 64-bit PMULL, SHA-1, SHA-256) and CRC32, as QEMU's `cortex-a57` has them, 4 KiB-granule stage-1 MMU with TLB, exclusives, PSCI over `HVC` as QEMU provides it |
| Interrupts | QEMU's GICv2 (no Security Extensions): its running priority and active priority registers, SGI source processors, and 4 KiB distributor and 8 KiB CPU interface regions; + GICv2m frame |
| Timers | EL1 physical and virtual generic timers |
| Devices | PL011 UART (QEMU's 16-entry receive FIFO and loopback), PL031 RTC, PL061 GPIO with QEMU's `gpio-key` power button on line 3, CFI flash ×2 (QEMU `pflash_cfi01` semantics, whole 64 MiB banks decoded), `fw_cfg` (traditional interface, no files), 32 virtio-mmio transports (QEMU's legacy layout while empty; non-legacy, as QEMU's `force-legacy=false`, once the host plugs in QEMU's 2D `virtio-gpu-device` with its EDID, `virtio-keyboard-device`, `virtio-tablet-device` or `virtio-blk-device` over a raw image in memory or a host file, which take the highest free ones), an empty PCIe ECAM/windows |
| Board | QEMU `virt` memory map and IRQ wiring; QEMU 11.1.2's device tree in `assets/` (two values pinned; see below), its memory node and `/chosen` set per machine |
| Boot | UEFI firmware from flash; or a Linux `Image`, initramfs and device tree placed as QEMU's `hw/arm/boot.c` places them, entered with the register state its boot stub leaves |

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
  bytes reaching a polling guest, PSCI power-off and reset, the power button's
  100 ms on GPIO line 3, an observer's view, direct kernel placement and its
  device tree, refused builds; and that a run cut into instruction budgets of
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
  completes.
- `tests/wfx_and_debug.rs`: WFI and WFE trap from EL0 only when they would
  wait — WFI not when an interrupt is pending, masked or not; WFE not when
  the Event Register, set by `SEV`, `SEVL`, an exception return or the
  virtual event stream, is set — and the Debug Communications Channel
  registers read as zero, ignore writes and trap from EL0 under
  `MDSCR_EL1.TDCC`.
- `tests/translation_and_firmware.rs`: an out-of-range T1SZ, a table base or
  block beyond the output size `TCR_EL1.IPS` selects, `AT` meeting an
  external abort on its walk, PSCI's affinity and entry-point checks, and a
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
- **Where QEMU has no defined behaviour.** Acknowledging an SGI that is
  pending from no processor, which only a `GICD_SPENDSGIR` write of zero
  leaves, fails an assertion in QEMU (`gic_clear_pending_sgi`); this model
  acknowledges it as from processor 0. The PL011's receive count, which
  loopback can push past the FIFO depth, is a signed `int` in QEMU whose
  overflow is undefined; this model's saturates.

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
