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
  Decisions are cached per word, so straight-line code skips the decoder.
- **Floating point is software.** `src/softfloat` is rusty_box's own port of
  Berkeley SoftFloat 3e (via Bochs softfloat3e), restricted to half, single
  and double precision and given an Arm specialization: Arm's default NaN and
  NaN-propagation order (including fused multiply-add), FPCR.DN, FPCR.FZ
  input and output flushing, tininess before rounding, and saturating
  float-to-integer conversions. On top of it `softfloat/arm_ops.rs` adds what
  Berkeley has no counterpart for — fixed-point conversions, FCVTXN's
  round-to-odd, the FPCR.AHP alternative half-precision format, FMULX,
  FRECPS and FRSQRTS — and `cpu/fp_estimate.rs` the bit-exact
  FRECPE/FRSQRTE/URECPE/URSQRTE/FRECPX algorithms. Exceptions accumulate into
  FPSR; like a Cortex-A57, the core does not trap them.

## What is modelled

| Part | Model |
|---|---|
| Processor | Cortex-A57-shaped ARMv8.0: EL0/EL1 AArch64 only, the whole ARMv8.0 FP and Advanced SIMD instruction set (integer and floating point, vector and scalar, saturation setting FPSR.QC), the Cortex-A57's optional Cryptographic Extension (AES, 64-bit PMULL, SHA-1, SHA-256) and CRC32, as QEMU's `cortex-a57` has them, 4 KiB-granule stage-1 MMU with TLB, exclusives, PSCI over `HVC` as QEMU provides it |
| Interrupts | GICv2 (no Security Extensions) + GICv2m frame |
| Timers | EL1 physical and virtual generic timers |
| Devices | PL011 UART (QEMU's 16-entry receive FIFO), PL031 RTC, PL061 GPIO with QEMU's `gpio-key` power button on line 3, CFI flash ×2 (QEMU `pflash_cfi01` semantics, whole 64 MiB banks decoded), `fw_cfg` (traditional interface, no files), 32 virtio-mmio transports (QEMU's legacy layout while empty; non-legacy, as QEMU's `force-legacy=false`, once the host plugs in QEMU's 2D `virtio-gpu-device` with its EDID, `virtio-keyboard-device`, `virtio-tablet-device` or `virtio-blk-device` over a raw image in memory or a host file, which take the highest free ones), an empty PCIe ECAM/windows |
| Board | QEMU `virt` memory map and IRQ wiring; QEMU 8.2's device tree in `assets/`, its memory node and `/chosen` set per machine |
| Boot | UEFI firmware from flash; or a Linux `Image`, initramfs and device tree placed as QEMU's `hw/arm/boot.c` places them, entered with the register state its boot stub leaves |

Provenance: instruction semantics follow the Arm ARM pseudocode; every device
cites the QEMU source file and symbols it was ported from.

Beyond the Cortex-A57, the core implements the features of an Armv9.2
processor as they are built: FEAT_LSE (the atomic memory operations, SWP,
CAS, CASP), FEAT_LSE2's alignment rules, FEAT_LRCPC (LDAPR) and FEAT_LRCPC2
(LDAPUR, STLUR); and, being built, FEAT_SME (its state, registers, SMSTART
and SMSTOP, RDSVL, ADDSVL and ADDSPL). No processor a machine runs has them
yet: the tests run them on a test-only processor with every feature, whose
ID registers advertise each one.

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
- `tests/unit/difftest.rs` also replays `tests/vectors/qemu_max.txt`, on
  the processor with every feature: 1,000 vectors of FEAT_LSE (LDADD,
  LDCLR, LDEOR, LDSET, LDSMAX, LDSMIN, LDUMAX, LDUMIN and SWP of every size
  and ordering, CAS, CASP) and FEAT_LRCPC and LRCPC2 (LDAPR, LDAPUR*,
  STLUR*), in the same format from the same initial states, generated by
  `tests/vectors/gen_qemu.py` with QEMU 8.2.2's user-mode `max` processor:
  a static program (`qemu_harness.c`, `qemu_trampoline.S`) runs each word
  from the whole register state and data region and reports both back.
  Run through that harness, a quarter of `unicorn.txt` agrees with Unicorn
  on all of its 5,895 vectors but one, an LDP with `Rt == Rt2` (CONSTRAINED
  UNPREDICTABLE). Breaking LDCLR or CASP's pair order on purpose fails 22
  and 187 of them. `tests/unit/atomic.rs` checks what the vectors cannot,
  over a stage-1 table of Normal, read-only and Device memory: an atomic
  reports its faults as writes, a CAS whose comparison fails writes nothing
  but still needs write permission, CASP with an odd register is
  UNDEFINED, the LSE2 and `nAA` alignment rules with and without the
  feature, signed LDAPUR offsets, the SP alignment check.
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
- The EDK2 boot was compared with QEMU 8.2 running the same firmware, device
  tree and memory size: identical console bytes.
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
  device tree, refused builds.

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
  and UMULH, and `Rm` of FCMP and FCMPE with zero. FEAT_LRCPC's LDAPR,
  LDAPRB and LDAPRH have one too, `Rs`, which QEMU 8.2.2 also refuses
  unless it is 11111 (`Rt2` of CAS and CASP is not one: Arm fixes it, so
  its other values are unallocated). A word that sets one of
  them otherwise is CONSTRAINED UNPREDICTABLE, and UNDEFINED is among the
  behaviours the Arm ARM permits; it is this model's, so the decoder is
  exactly Arm's encoding table, which fixes those bits. QEMU 11
  (`target/arm/tcg/a64.decode`) fixes them for LDAR, STLR, SMULH and UMULH
  only and executes the exclusives (`STXR`, `LDXR`, `STXP`, `LDXP`) and the
  compares with zero (`FCMP`, whose `rm` it ignores when `z` is set) as if the
  field held its should-be value. The Unicorn replay, whose QEMU fixes none
  of them, skips its vectors that set such a field.
- **LDSMAX and LDSMIN compare signed at every size.** Arm's `MemAtomicInt`
  computes `Max(SInt(op1), SInt(op2))` (and `Min`) on the access's width.
  Unicorn 2.1.4's `max` processor compares the byte, halfword and word forms
  unsigned: 25 of 200 generated LDADD-group vectors disagreed with this
  model, all of them those forms. QEMU 8.2.2 agrees with Arm, so these
  vectors come from it (`gen_qemu.py`).
- **LDAPUR and STLUR offsets are signed.** Arm's pages compute
  `SignExtend(imm9, 64)`. QEMU 8.2.2 adds `imm9` unsigned: LDAPURSB with
  offset -166 reads at +346. `gen_qemu.py` keeps the offset non-negative,
  and `tests/unit/atomic.rs` checks negative ones against Arm's definition.
- **Without FEAT_LSE2, a misaligned load-acquire or store-release faults.**
  The current `AArch64_UnalignedAccessFaults` faults an ordered access only
  when it leaves its 16-byte granule with `SCTLR_ELx.nAA` = 0, testing no
  feature. `nAA` is a FEAT_LSE2 field, so this model applies that rule with
  FEAT_LSE2 and the Armv8.0 one without it, as the Cortex-A57 and QEMU
  (`check_ordered_align`, alignment enforced unless the processor has
  LSE2) do: any misaligned LDAR, STLR, LDAXR or STLXR faults.
- **RPRFM is a prefetch.** Arm's data allocates PRFM (register) with
  `Rt = 11xxx` to RPRFM (FEAT_RPRFM, Armv8.8). On a core without the feature
  the word is a PRFM with a reserved `prfop`, which the Arm ARM executes as a
  hint, as QEMU does; the decoder conformance test lists it as its one
  departure from Arm's allocation.

## Not implemented

- EL2, EL3, AArch32, and on the Cortex-A57 every post-v8.0 feature (LSE
  atomics, PAuth, SVE, …): absent from its ID registers, so UNDEFINED, as
  on a real ARMv8.0 core. That includes the cryptographic instructions the
  Cortex-A57 lacks, SHA-512, SHA-3, SM3 and SM4 (Armv8.2 options): every
  QEMU CPU model that has them also has features this core does not (RDM,
  the dot product, half precision), so no model to take their ID values
  from matches this one. The later features the core implements (see "What
  is modelled") are in no processor a machine runs yet.
- Floating-point exception traps: FPCR's trap-enable bits read as zero, as on
  a Cortex-A57, so exceptions only accumulate in FPSR.
- Debug: breakpoint and watchpoint registers exist (two of each, as
  advertised) but never fire; no Debug state, no PMU.
- No instruction or data cache model: cache maintenance is accepted and has
  nothing to act on.
- No network or block device, and nothing on the PCIe bus: storage is the
  initramfs, and the display and input devices are the opt-in virtio ones.
- UEFI firmware draws nothing on the virtio display: `ArmVirtQemu`'s boot
  manager (`PlatformBootManagerLibLight`) puts only PCI displays on its
  console, as it does on QEMU, so the screen waits for the operating
  system's driver.
