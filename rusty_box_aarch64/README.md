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
- **Decoding is split, and the decoder is this crate's own.**
  [`src/cpu/isa/encodings.txt`](src/cpu/isa/encodings.txt) lists every A64
  encoding the processor knows, each under the encoding class of the Arm
  ARM's A64 encoding index (DDI 0487, chapter C4) it belongs to, with that
  class's bit diagram. `cargo xtask a64-decode` checks every row against its
  diagram and compiles the list into a decision tree
  ([`table.rs`](src/cpu/isa/table.rs), checked in; `cargo xtask ci` fails
  while it is not what the list compiles to). A word decodes to the matching
  encoding with the most fixed bits; the encoding names its class, the class
  its feature (`FEAT_*`), and an encoding of a feature the ID registers do not
  advertise is UNDEFINED. One handler per class then reads every operand
  field from the word, following the Arm ARM. Dispatch is an exhaustive match
  over the closed class enum, so every encoding the decoder can name has a
  handler. Decisions are cached per word, so straight-line code skips the
  tree.
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
| Processor | Cortex-A57-shaped ARMv8.0: EL0/EL1 AArch64 only, the whole ARMv8.0 FP and Advanced SIMD instruction set (integer and floating point, vector and scalar, saturation setting FPSR.QC), 4 KiB-granule stage-1 MMU with TLB, exclusives, PSCI over `HVC` as QEMU provides it |
| Interrupts | GICv2 (no Security Extensions) + GICv2m frame |
| Timers | EL1 physical and virtual generic timers |
| Devices | PL011 UART (QEMU's 16-entry receive FIFO), PL031 RTC, PL061 GPIO with QEMU's `gpio-key` power button on line 3, CFI flash ×2 (QEMU `pflash_cfi01` semantics, whole 64 MiB banks decoded), `fw_cfg` (traditional interface, no files), 32 empty virtio-mmio transports, an empty PCIe ECAM/windows |
| Board | QEMU `virt` memory map and IRQ wiring; QEMU 8.2's device tree in `assets/`, its memory node and `/chosen` set per machine |
| Boot | UEFI firmware from flash; or a Linux `Image`, initramfs and device tree placed as QEMU's `hw/arm/boot.c` places them, entered with the register state its boot stub leaves |

Provenance: instruction semantics follow the Arm ARM pseudocode; every device
cites the QEMU source file and symbols it was ported from.

## Verification

- `src/cpu/difftest.rs` replays `tests/vectors/unicorn.txt`: 23,178
  single-instruction vectors over 72 encoding families (13,191 of them
  floating point), generated by `tests/vectors/gen_unicorn.py` with Unicorn
  (QEMU's TCG). Each vector stores only a seed and Unicorn's results — PC,
  NZCV, FPSR and every changed register and data byte — so the test needs no
  Unicorn. Floating-point inputs mix special values (zeros, infinities,
  quiet and signalling NaNs, denormals, range and tie boundaries) with
  ordinary ones under a random FPCR (rounding mode, FZ, DN, AHP). Unicorn
  runs as QEMU's cortex-a57 model, so every vector is an instruction a
  Cortex-A57 executes, and the replay fails on any of them this core treats
  as UNDEFINED — except the 32 it is meant to: the optional crypto/CRC32
  extensions, which this core does not advertise, and SMULH/UMULH with a
  non-ones Ra (CONSTRAINED UNPREDICTABLE). Every other vector agrees.
  Breaking QC saturation, round-to-odd, the AHP conversions or a decode fix
  on purpose makes dozens to hundreds of vectors fail.
- The decoder is equivalent, on all 2^32 words, to
  [`disarm64`](https://crates.io/crates/disarm64) 0.2 (a decoder generated
  from binutils' opcode table, used only as a dev-dependency): wherever
  disarm64 names an encoding of an advertised feature, this decoder names the
  same one (by opcode and mask), and everywhere else it says UNDEFINED. The
  deliberate exceptions are the HINT space, which executes whatever the hint
  (CHKFEAT, one word), and two disarm64 tagging faults: RPRFM filed as a
  base encoding (here it is PRFM (register) with a reserved prfop, 65,536
  words), and the fixed-point conversions by immediate filed as FEAT_FP16
  beyond `immh = 001x` (here the single- and double-precision forms, 589,824
  words). `cargo test --release -p rusty_box_aarch64 --lib -- --ignored
  the_classifier_matches_disarm64_on_every_word` runs it (about 14 s on four
  threads); a sampled form runs with every test run.
- When the encoding list was written it was also checked against QEMU 11.1.2's
  decoder (`target/arm/tcg/a64.decode`): row instances QEMU decodes, it
  decodes to the same instruction, and the 22,448 sampled row instances it
  leaves unallocated are all UNDEFINED here. That check found ADD/SUB
  (immediate) executing `shift = 1x`, a reserved value; and UDF, which the
  list gives its own class, was decoded as an exception-generating
  instruction, so `udf #1` and `udf #2` behaved as SVC and HVC.
- `tests/encoding_coverage.rs` (ignored: it reads disarm64's table from the
  cargo registry) runs 48 random instances of each of disarm64's 3,774
  encodings on a bare machine and prints, per feature set, how many execute
  and how many are UNDEFINED. Every ARMv8.0 base, FP and Advanced SIMD
  encoding executes except the ten that are UNDEFINED at EL1 by rule (UDF,
  DCPS1–3, DRPS, HLT, SMC, SYSL; MRS and MSR, whose random instances name
  registers that do not exist, are checked with ones that do), and every
  instance of an absent feature is UNDEFINED.
- The EDK2 boot was compared with QEMU 8.2 running the same firmware, device
  tree and memory size: identical console bytes.
- Alpine Linux 3.23.7 (kernel 6.18.58, Alpine's `virt` configuration) boots to
  a login prompt with no warnings in its log, logs in, and runs commands whose
  output matches the host (`seq 1 20000 | sha256sum`); `reboot` comes back to
  the login prompt and `poweroff` stops the machine.
- `tests/timer_interrupts.rs`: a hand-written guest drives GIC + timer + `WFI`
  + IRQ entry/`ERET` + PSCI; its outcome is identical for every budget
  slicing, and a snapshot taken mid-wait restores the same future.
- `tests/api.rs`: the host API with guest-visible assertions — a page-crossing
  store's data abort reporting the page that faulted, the terminal keyboard's
  bytes reaching a polling guest, PSCI power-off and reset, the power button's
  100 ms on GPIO line 3, an observer's view, direct kernel placement and its
  device tree, refused builds.

## Not implemented

- EL2, EL3, AArch32, and every post-v8.0 feature (LSE atomics, PAuth, SVE, …):
  absent from the ID registers, so UNDEFINED, as on a real ARMv8.0 core.
- Floating-point exception traps: FPCR's trap-enable bits read as zero, as on
  a Cortex-A57, so exceptions only accumulate in FPSR.
- Debug: breakpoint and watchpoint registers exist (two of each, as
  advertised) but never fire; no Debug state, no PMU.
- No instruction or data cache model: cache maintenance is accepted and has
  nothing to act on.
- No display, keyboard or network device: the console is the UART, and the
  PCIe bus and virtio-mmio slots are empty.
