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
- **Decoding is split.** [`disarm64`](https://crates.io/crates/disarm64)
  classifies each word into its canonical encoding (never an alias) and its
  feature set; one handler per encoding *class* reads every operand field from
  the word, following the Arm ARM. Encodings of features the ID registers do
  not advertise are UNDEFINED. A defined encoding the interpreter did not
  implement would stop the machine (`StopReason::Unimplemented`) rather than
  misbehave; no ARMv8.0 encoding is left in that state.
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
