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

(`apt install qemu-efi-aarch64` provides the firmware.) Both run a
Cortex-A57 unless `--cpu armv9.2` asks for the Armv9.2 processor with SME2.
A device tree given with `--dtb` must name the SPIs the board wires; one
QEMU wrote needs QEMU's, `--interrupts
uart=1,rtc=2,gpio=7,virtio=16,pcie-intx=3,msi-first=48,msi-count=64`.

`--interactive` hands the console to the terminal once the script is done:
stdin goes to the UART byte for byte (the runner puts a terminal in raw
mode for the run and restores it however the run ends), and Ctrl-] ends
the run. `cargo xtask nested-kvm` puts it all together for a nested KVM
guest, with nothing installed beyond cargo: it downloads Alpine's aarch64
ISO and QEMU's packages (checking the ISO's SHA-512, each repository
index's signature against the keys in the ISO's initramfs, and each
package's checksum against its index), builds a disk of the ISO followed
by a tar of the packages and the runner, and boots the Cortex-A57 with
virtualization to the nested guest's own `login:`, then leaves the terminal
to you (Ctrl-A x quits the nested QEMU). Its steps are kept in
`target/nested-kvm` and skipped when done; `--help` lists its options.

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

The machine names itself [`BoardIdentity::RUSTY_BOX`](src/identity.rs)
unless the builder is given another: `MachineBuilder::identity` takes a
`BoardIdentity::new(IdentityNames { model: "Acme Board", ..IdentityNames::RUSTY_BOX })`,
whose names are checked and copied into the machine, with no allocator.

Where Arm's documents allow a guest more than one answer, the answer is
the host's to choose, as a field of `VirtConfig` of a type that holds only
the answers the document allows, and the board takes the one in the table
unless told otherwise. A snapshot records each, and is refused by a
machine built with another (the guest may have read and acted on it). No
default is QEMU's; a host may still choose a value QEMU also has (256
SPIs, eight priority bits, a 62.5 MHz counter), and the choice is its own.

| Setting | Type | Answers (default first) | Specification |
|---|---|---|---|
| `spectre` | `SpectreReport` | `NotRequired`, `Mitigated`, `Unreported` | DEN 0028F §7.5 to §7.7 |
| `counter` | `CounterRate` | 1 GHz; any rate from BSA's 10 MHz to what `CNTFRQ_EL0`'s 32 bits hold, on a processor whose architecture leaves it open (the Cortex-A57; Armv8.6 and later fix 1 GHz, and the build refuses another, `BuildError::FixedCounterRate`) | Arm ARM "The system counter"; BSA DEN0094C RB_TIME_02 |
| `gic` | `GicShape { spis: SpiCount, priority_bits: PriorityBits }` | 480 SPIs, five priority bits (the GIC-400's); SPIs in 32s from 32 (the fewest any interrupt map fits, the devices' 11 lines on the board's own layout and an MSI; the board's own map needs 64), priority bits `Four` to `Eight`, the binary points and active priority registers following | IHI 0048B §4.3.2, Table 3-1, Table 4-47; DDI 0471B §2.3.3 |
| `interrupts` | `InterruptMap { uart, rtc, gpio, virtio, pcie_intx: Spi, msi: MsiSpis }` | `InterruptMap::own(layout)`: the devices' lines from SPI 0 in the order of their registers' addresses (UART 0, RTC 1, GPIO 2, the layout's virtio transports from 3, the PCIe host's INTA to INTD after them) and the GICv2m frame every SPI from the first after them whose interrupt ID is a multiple of 32 (`MsiExtent::ToTheLast`); on the board's own layout (`InterruptMap::DEFAULT`) the 4 transports 3 to 6, INTA to INTD 7 to 10 and the frame from SPI 32, INTID 64, so 32 to 479 on 480; on QEMU's the 32 transports 3 to 34, INTA to INTD 35 to 38 and the frame from SPI 64, INTID 96. Any map whose SPIs the GIC has, the frame at least one, and no two sources share, the layout's transports taking a run of SPIs (`BuildError::InterruptMap`), the frame's count 1 to 1023 | IHI 0048B §2.2.1; BSA DEN0094C I.5, I.9.1; PCI Local Bus 3.0 §6.8.1.6 |
| `transport_order` | `TransportOrder` | `FirstFree`, each virtio device (the display, the keyboard, the tablet, the disk, in that order) in the first transport the ones before it left free, so the first at the lowest address and first in the device tree; `LastFree`, from the last down | OASIS virtio 1.2 §4.2 (left to the platform) |
| `layout` | `DeviceLayout` | `Board`, the board's own (the GIC-400's block at 0x0800_0000, the GICv2m frame, the PrimeCells and 4 virtio transports, one for each device the board can plug in, in a 64 KiB page each from 0x0801_0000, each transport decoding its page, the PCIe host's 32-bit window at 0x2000_0000 and its ECAM and 64-bit window from 256 GiB); `QemuVirt`, QEMU `virt`'s, for a tree QEMU wrote, its 32 transports 512 bytes apart; RAM at 0x4000_0000 and the flash at 0 in both (see "QEMU fingerprints left"); the power button on the GPIO's line 0 on `Board`, line 3 on `QemuVirt`. A host's tree naming another address for a device fails the build (`BuildError::HostTreeAddress`), as does one putting a `gpio-keys` key on another line of the board's GPIO (`BuildError::HostTreePowerButton`) | DEN0001C 4.1.2, 4.3.2, P29; DDI 0471B Table 3-1; OASIS virtio 1.2 §4.2.2 |

The front end takes them as its `[virt]` table's keys (`spectre =
"mitigated"`, `transport_order = "last-free"` and `layout = "qemu-virt"`, by each answer's
`name`; `counter_hz`, `gic_spis` and
`gic_priority_bits` as numbers) and its `[virt.interrupts]` table's
(`uart`, `rtc`, `gpio`, `virtio`, `pcie_intx`, `msi_first`, `msi_count`,
each a number, a key left out the board's own for the layout), refusing a
name or number none has with the ones there are, and a map the GIC cannot
hold; the examples' `--interrupts` keys work the same way.

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
  the exact first tick whose count reaches `CVAL`; the PL031 RTC reads a
  fixed value at power-on and, once the guest starts it, counts emulated
  seconds. Nothing reads the host clock.
- **Waiting is free and slicing is invisible.** `WFI`, `WFE` and the timed
  `WFIT` and `WFET` jump time to the next event that can end them (an
  interrupt, the timer's event stream, their timeout), and the processor
  stays in the waiting state across `step` calls, so
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
  operand fields extracted once, with what depends on the word alone
  worked out ahead (a logical immediate's mask, a branch offset); every
  other word stays on its class handler.
  [`src/cpu/trace.rs`](src/cpu/trace.rs) keeps straight-line runs
  of ops, at most 32 and never across a page, keyed by the physical address
  of their first: the PC is translated, with the fetch permission check,
  once per trace, and a trace ends after a branch that always leaves (the
  pointer-authenticating ones included), an exception-generating
  instruction, `ERET`, `WFI`/`WFE`/`WFIT`/`WFET`, `ISB`, every `MSR`
  (`SMSTART` and `SMSTOP` among them) and `SYS`. A trace starts only with
  PSTATE.BTYPE clear: an instruction a `BR` or `BLR` leads to runs by
  itself, after its branch target check (FEAT_BTI;
  `tests/bti_traces.rs`). A trace runs only as far as the
  instruction-at-a-time loop would without looking outside the processor —
  the budget, the next timed event, the exits — and stops early after an
  instruction that reached a device, wrote a timer register, filled the
  TLB or removed any of it, or wrote over cached code. Code stays coherent with
  memory without the guest's help: each page's 128-byte lines that hold
  cached instructions are marked (Bochs's page write stamps), and every
  store the processor makes, every device DMA write and every host write
  forgets the traces on the lines it touches, so `IC` maintenance has
  nothing left to invalidate (`IC IVAU` still translates its address, as
  Arm's `AArch64_IC` does); and only RAM and a flash bank in read-array mode are cached
  at all (the bus says which, and counts the flash writes that could
  change it). Every instruction a trace runs is fetched, checked, executed
  and reported to an observer exactly as one at a time would be, so a run
  ends in the same state either way (the speed section lists the boots'
  state hashes). The one exception is the counter poll, below: when the
  observer does not hear of every instruction, whole iterations of a poll
  may be skipped, unfetched and unreported, leaving exactly the state
  running them would leave.
- **Traces are linked.** As Bochs links the trace a direct jump leads to
  (`cpu/cpu.cc` `linkTrace`, `cpu/cpu.h` `BX_LINK_TRACE`), a trace that
  ends in a taken direct branch (`B`, `BL`, `B.cond`, `CBZ`/`CBNZ`,
  `TBZ`/`TBNZ`) to its own page goes on into the cached trace there without
  going back to the run loop, which saves the loop's checks, the fetch's
  translation and the memory's source for one trace in every one linked
  (`Cpu::linked_trace`). The translation and the source are the trace's:
  nothing in a trace changes them, or it ends. The link is taken only where
  the loop would have run that trace next with nothing done in between: the
  burst has instructions left (the next trace is cut to them), no
  interrupt is deliverable, and the run watches no exit address. Where Bochs
  keeps the next trace's pointer in the branch, valid while a timestamp
  every invalidation moves is unchanged, the link here looks the trace up in
  the direct-mapped cache by address, epoch and generation, which is what a
  flush, a write over the code or a flash write changes; a trace marked as a
  poll's is never found that way, so the poll's decision stays on the run
  loop's miss path. `tests/trace_linking.rs` runs linked loops, one rewritten
  by a store and one taking timer interrupts, against a machine stepping one
  instruction at a time, which never links, comparing whole snapshots and
  everything an observer hears after every step.
- **Counter polls fast-forward exactly.** When nobody watches every
  instruction, a loop that reads the physical or the virtual count
  (`CNTPCT_EL0`, with `CNTPOFF_EL2` subtracted at EL0 and EL1 under
  `CNTHCTL_EL2.ECV`, or `CNTVCT_EL0` with `CNTVOFF_EL2` subtracted), compares
  it once with a value it does not change and branches on an ordering of
  the two jumps over every whole iteration that goes as the current one
  does: along a growing count each such branch changes its outcome at most
  once, so a binary search finds the last of them. The read must pass every
  check the instruction makes, EL2's traps included, and a stretch across
  an offset count's wrap is not skipped. The skip stops short of the budget and of
  the next timer or device event, passes no exit address, and leaves
  iterations to run that refill the PC history. A trace that may begin such
  a loop is marked when it is built, so no other trace pays for the check
  ([`src/cpu/poll.rs`](src/cpu/poll.rs); `tests/counter_poll.rs` compares
  whole snapshots with a machine that runs every instruction).
- **Floating point is defined in software.** `src/softfloat` is rusty_box's
  own port of Berkeley SoftFloat 3e (via Bochs softfloat3e), restricted to
  half, single and double precision and given an Arm specialization: Arm's
  default NaN and NaN-propagation order (including fused multiply-add),
  FPCR.DN, FPCR.FZ input and output flushing, tininess before rounding,
  and saturating float-to-integer conversions — and FEAT_AFP's alternate
  behaviours under FPCR.AH, chosen per operation: tininess and flush to
  zero after rounding, NaNs chosen by register, a negative default NaN,
  Input Denormal for a used denormal where Arm's FPProcessDenorms says, and
  FPCR.FIZ's silent operand flushing. On top of it
  `softfloat/arm_ops.rs` adds what Berkeley has no counterpart for —
  fixed-point conversions, FCVTXN's round-to-odd, the FPCR.AHP alternative
  half-precision format, FMULX, FRECPS and FRSQRTS — and `cpu/fp_estimate.rs`
  the bit-exact FRECPE/FRSQRTE/URECPE/URSQRTE/FRECPX algorithms. Exceptions
  accumulate into FPSR; like a Cortex-A57, the core does not trap them.
  SoftFloat is the definition:
  [`src/softfloat/host.rs`](src/softfloat/host.rs) lets the host compute a
  single- or double-precision add, subtract, multiply, divide or, with `std`
  on a host whose `fma` is known to round once, fused multiply-add only where
  IEEE 754 makes the host's answer SoftFloat's (QEMU's "hardfloat" rule,
  narrowed). Rounding must be to nearest even with inexact already raised,
  every operand normal, and the result normal, finite and above the smallest
  normal; FEAT_AFP's FPCR.AH, FIZ and NEP then change nothing the host
  computes (the module's documentation gives the pseudocode for each).
  `tests/unit/host_float.rs` checks the host against SoftFloat at
  those boundaries under every FPCR; on x86-64 Linux it passes both with and
  without the processor's FMA, the second under `qemu-x86_64 -cpu
  Nehalem`, where `fma` and `fmaf` run compiler-builtins' software
  fallbacks.

## Speed

`aarch64_perfbench` ([`examples/perfbench.rs`](examples/perfbench.rs))
measures the interpreter on five embedded loops and two boots; `cargo xtask
perf-baseline` archives it for A/B runs. Host: Intel Xeon @ 2.8 GHz
(AVX-512), a 4-core VM, 2026-10-27. Each figure is the median of 10
interleaved rounds, in millions of instructions a second; "hooked" runs
under an observer with a code hook and a memory hook. "Before" is the
interpreter without a trace cache (5028db8); "Unmerged" is the trace cache
before experimental/2-multiarch-gui's Armv9.2 work was merged in (157c4c7);
the ratio is "Now" against "Before".

| Workload | Before | Unmerged | Now | |
|---|---:|---:|---:|---:|
| `alu` | 53.1 | 165.3 | 163.4 | 3.08× |
| `alu`, hooked | 53.8 | 162.8 | 161.5 | 3.00× |
| `memcpy` (`ldp`/`stp`) | 27.1 | 89.8 | 74.7 | 2.76× |
| `memcpy`, hooked | 24.5 | 81.4 | 77.4 | 3.16× |
| `fnv` (byte loads, `mul`) | 58.3 | 138.6 | 133.4 | 2.29× |
| `fnv`, hooked | 58.1 | 130.7 | 135.6 | 2.33× |
| `fp` (`fmadd`/`fmul`/`fadd`) | 37.6 | 81.4 | 67.0 | 1.78× |
| `fp`, hooked | 34.3 | 75.4 | 65.7 | 1.92× |
| EDK2 (AAVMF) to `Shell>` | 51.0 | 184.0 | 176.9 | 3.47× |
| Linux 6.18 + initramfs to `login:` | 39.0 | 78.9 | 75.0 | 1.93× |

`fp` is short of the 2× set as the loops' target (the boots' was 1.5×).
EDK2's figure counts the instructions of the counter polls it
fast-forwards as executed, since the guest retires them.

Counted exactly with callgrind, as host instructions inside `run_until`,
the merged tree costs more than the unmerged one: +6.4% on `alu`, +6.5% on
`memcpy`, +2.0% on `fnv`, +4.0% on `mmio` and +18.5% on `fp`. `fp`'s
share is FEAT_AFP's per-operation work: every FP data-processing
instruction builds its environment from FPCR with AH, FIZ, NEP and
FEAT_RPRES, and merges the upper elements under NEP, which `fp_dp2`, `fp_dp3` and
`fp_binary` show as 0.9, 0.7 and 0.3 billion more host instructions on the
loop. The rest is in `run_until`'s own inlined loop (its register
allocation and the addresses it recomputes per guest instruction), not in
any function it calls; removing BTI's trace diversion, BTYPE, PAN, the
wake deadline or the ZA array's size one at a time does not recover it.

On this host (an AVX-512 Xeon) a loop's ratio moves by as much as 16%
with branch placement alone: one change measured `alu` at 0.87× and
`memcpy` at 0.84× of its parent in the default build, and at 1.03× and
1.09× with both binaries built with
`-C llvm-args=-x86-branches-within-32B-boundaries`, from the same source.
Read a single row's ratio to about that precision.

The run loop's register allocation follows the layout of the machine it
runs in. rustc orders a plain (`repr(Rust)`) struct's fields by
alignment, counting a field's size towards it. Laid out that way, 8 more
bytes in the PL011, which no loop reaches, make the board's size a
multiple of 16 and move the board ahead of the CPU, to the machine's own
address. The board's pointer, which the loop passes as the bus, is then
the machine's, and LLVM allocates the loop's registers differently: the
base of the general registers is no longer held in a register but
recomputed before every access, and two more values spill to the stack on
every guest instruction. Measured with callgrind as host instructions
inside `run_until`, that costs +4.0% on `alu`, +3.7% on `fnv`, +0.8% on
`memcpy` and +0.7% on `fp`.

`VirtMachine` and `VirtBoard` are therefore `#[repr(C)]`, laid out for the
loop. The board is the machine's last field, so no device's size moves
another field. Only fixed-size records come before the CPU, and neither
the CPU nor the board is at offset 0, where a field shares the machine's
address. In the board the devices come first and the fields the loop
reads (the RAM, `deadline_moved`, `touched`, `flash_epoch` and the
devices' RAM writes) after them: a device's size moves those fields, but
every access to them keeps a 32-bit displacement, so the loop's
instructions keep their lengths and its alignment padding. Placed ahead of
the devices instead, the same fields are reached with shorter
displacements, which moves the padding and costs 0.1–0.2% host
instructions on the plain loops. With the PL011 padded by 0, 8, 16, 24 or
40 bytes the loop's code is the same instruction for instruction, and so
are the host instruction counts. A change to the loop or to the layout is
held by `cargo xtask ci`, which fails on a rise of more than 0.5% in those
counts (["Keeping the interpreter's
speed"](../docs/aarch64-automation.md#keeping-the-interpreters-speed)).

The fifth loop, `mmio`, runs `alu`'s chain from the GIC distributor's
group bits, whose fetches cannot be cached, so every instruction takes
the per-instruction path, at about 560 host instructions each. That path
is kept out of line and marked cold for the trace loop's sake: without the
mark, `mmio` takes 3.5% fewer host instructions, and `alu` and `fnv` 4.5%
more. It has no row in the table: 5028db8 has no such loop to compare it
with, and what it is there for, the cost of the per-instruction path, is
held as a host instruction count by `cargo xtask ci` instead of timed.

The boots' state hashes at 10M, 100M and 400M instructions on this tree,
which a change meant to leave the guest's behaviour alone (as speed work
is) must reproduce, are:

| | 10M | 100M | 400M |
|---|---|---|---|
| EDK2 | `01b13a2711ecf8f8` | `5eb35b3f7f833ed0` | `0b84647223d9bbc8` |
| Linux | `e2d1415e61651fa0` | `5b03e228ac3717cc` | `6c7c458dadcddc2f` |

## What is modelled

| Part | Model |
|---|---|
| Processor | One of two models (`VirtConfig::cpu`, `CpuModel`). `cortex-a57`, the default: Cortex-A57-shaped ARMv8.0, of revision r1p3 (`MIDR_EL1` 0x411F_D073, the revision its TRM, DDI 0488H, describes; see "Where the references differ"), the whole ARMv8.0 FP and Advanced SIMD instruction set (integer and floating point, vector and scalar, saturation setting FPSR.QC), the Cortex-A57's optional Cryptographic Extension (AES, 64-bit PMULL, SHA-1, SHA-256) and CRC32, as QEMU's `cortex-a57` has them, and the board's 1 GHz system counter (QEMU's runs at 62.5 MHz; see "Where the references differ"). `armv9.2`: an Armv9.2 processor with SME and SME2 and no SVE (the features below), Streaming SVE vectors of 128 to 512 bits, `MIDR_EL1` 0x000F_0520 (implementer 0, a software model) and the 1 GHz counter Armv8.6 and later require. Both: EL0 and EL1, and on the Cortex-A57 EL2 with `MachineBuilder::with_el2` (see below), AArch64 only, the Cortex-A57's `CTR_EL0` and `CLIDR_EL1` (64-byte lines, split L1 and a unified L2), on the Cortex-A57 its TRM's IMPLEMENTATION DEFINED registers (`CPUACTLR_EL1`, `CPUECTLR_EL1`, `L2CTLR_EL1`, `L2ECTLR_EL1`, `L2ACTLR_EL1`, the memory error syndrome registers, `CBAR_EL1` reading where the layout has the GIC, `RAMINDEX` and its data registers, and `ACTLR_EL2`'s access controls; see "Where the references differ") and none on the Armv9.2 model, each model's own caches in `CCSIDR_EL1` (the Cortex-A57's its TRM's with a 512 KiB L2, the Armv9.2 model's the board's own; see "Where the references differ"), an MMU with the 4 KiB and 64 KiB granules, the Cortex-A57's (DDI 0488H, 4.3.21), and 44-bit physical and 48-bit virtual addresses (the Armv9.2 model the 16 KiB granule too, FEAT_TTST's small tables, and 52-bit physical addresses, FEAT_LPA: an untranslated address of up to 52 bits, and walks that output up to 52 bits with the 64 KiB granule, taking 52-bit virtual addresses with it, FEAT_LVA, and with the 4 KiB and 16 KiB granules under `TCR_ELx.DS`, FEAT_LPA2, walking from level −1 with the 4 KiB one, and up to 48 bits otherwise; at stage 1, as it has no EL2 yet), stage 1 and with EL2 stage 2, with a TLB holding both stages' translations, exclusives, PSCI 1.2 (DEN 0022E) and SMCCC 1.5's Arm Architecture Calls (DEN 0028F: `SMCCC_VERSION`, `SMCCC_ARCH_FEATURES`, and the speculation workarounds as `VirtConfig::spectre` reports them) over `HVC`, or over `SMC` with EL2, the conduits QEMU's `virt` provides (see "Where the references differ"); a PMU (`src/cpu/pmu.rs`) with the cycle counter and six event counters, the Cortex-A57's its TRM's PMUv3 (DDI 0488H chapter 11: `PMCR_EL0.IMP` 0x41, `IDCODE` 0x01, 32-bit event counters) and the Armv9.2 model's FEAT_PMUv3p7 with FEAT_PMUv3p5's 64-bit event counters and FEAT_PMUv3p4's `PMMIR_EL1`, with `PMUSERENR_EL0`'s EL0 access and `MDCR_EL2`'s traps, `HPMN`, `HPME`, `HPMD`, `HCCD`, `HLP` and `HPMFZO`, the `P`, `U` and `NSH` filters, `CHAIN`, freezing on overflow (FEAT_PMUv3p7), and PMUIRQ on PPI 23; it counts the events it advertises in `PMCEID0_EL0` and `PMCEID1_EL0` — `SW_INCR`, `INST_RETIRED`, `EXC_TAKEN`, `EXC_RETURN`, `CID_WRITE_RETIRED`, `CPU_CYCLES`, `TTBR_WRITE_RETIRED` and `CHAIN`, and on the Armv9.2 model the stall events, which count nothing — with the processor's clock the instruction clock: one cycle per instruction, stopped while the processor waits (see "Where the references differ") |
| Interrupts | GICv2 without the Security Extensions, for one processor, as IHI 0048B specifies it, in QEMU's structure, with the GIC-400's implementation choices (PPIs 25 to 31, level-sensitive, and BSA's PPI 22 for the DCC's COMMIRQ and PPI 23 for the PMU's PMUIRQ; by default 480 SPIs and five priority bits, minimum binary points 2 and 3, one active priority register a group, the SPIs and priority bits a host's `VirtConfig::gic`; see "Where the references differ"): both interrupt groups, with Group 0 on FIQ under `FIQEn`, the Group 1 aliases (`GICC_AIAR`, `GICC_AEOIR`, `GICC_AHPPIR`, `GICC_ABPR`), preemption by group priority, the running priority and active priority registers, and 4 KiB distributor and 8 KiB CPU interface regions; + GICv2m frame; with EL2, the Virtualization Extensions (IHI 0048B chapter 5) as the GIC-400 has them: the virtual interface control registers (`GICH_*` at 0x0800_4000, the common base and processor 0's at 0x0800_5000, 4 KiB each, on the board's own layout) with four List registers, the virtual CPU interface (`GICV_*`, 8 KiB at 0x0800_6000) driving the processor's virtual IRQ and FIQ, hardware interrupts deactivated in the distributor through their List register, and the maintenance interrupt on INTID 25, all named in the device tree's GIC node as the GICv2 binding names them |
| Timers | EL1 physical and virtual generic timers; with EL2, the EL2 physical timer (`CNTHP_*`, PPI 10), `CNTVOFF_EL2` and `CNTHCTL_EL2`'s traps and event stream; with FEAT_VHE too, the EL2 virtual timer (`CNTHV_*`, PPI 12, BSA's INTID 28, the fifth interrupt of the device tree's `timer` node) and `CNTHCTL_EL2`'s `E2H` layout; with FEAT_ECV_POFF too, `CNTPOFF_EL2` (both built, but no model has FEAT_VHE or FEAT_ECV_POFF with EL2 yet) |
| Devices | PL011 UART r1p5 (`UARTPeriphID2` revision 3, DDI 0183G; BSA's Generic UART: a 32-entry receive FIFO with overrun and break status, receive interrupts at the `UARTIFLS` level, modem-status interrupts on line changes, loopback, a receiver gated by `UARTEN` and `RXE`, a transmitter gated by `UARTEN` and `TXE` whose 32-entry transmit FIFO holds what is written meanwhile, transmit interrupts as the transmit FIFO drains through the `UARTIFLS` level, and the receive timeout counted in bit periods of the programmed baud rate on the machine's clock, while characters themselves cross the line in no time; see "Where the references differ"), PL031 RTC (r1p3, as DDI 0224C describes it: stopped until `RTCCR` starts it, zeroed by any later `RTCCR` write, stopped again by a system reset), PL061 GPIO as DDI 0190B describes it (address-masked `GPIODATA`, edge and level detection on the lines' levels, outputs included) with a `gpio-key` power button on its first line, 0 (line 3, QEMU's, on QEMU's layout), CFI flash ×2 (QEMU `pflash_cfi01` semantics, whole 64 MiB banks decoded; the identity's manufacturer and device codes, by default none, and a CFI query table of the board's own; see "Where the references differ"), no `fw_cfg` (see "Where the references differ"), virtio-mmio transports, 4 on the board's own layout and 32 on QEMU's (QEMU's legacy layout while empty; non-legacy, as QEMU's `force-legacy=false`, once the host plugs in QEMU's 2D `virtio-gpu-device` with its EDID, `virtio-keyboard-device`, `virtio-tablet-device` or `virtio-blk-device` over a raw image in memory or a host file, which take the first free ones, or the last free with `VirtConfig::transport_order`; every queue 256 entries), an empty PCIe ECAM/windows, the 32-bit memory window non-prefetchable and the 64-bit one prefetchable |
| Board | the board's own device layout after Arm's memory map principles, or QEMU `virt`'s (`VirtConfig::layout`, see "Where the references differ"), RAM and the flash where QEMU's `virt` has them in both; the board's own interrupt wiring (`VirtConfig::interrupts`, see "Where the references differ"); the board's own device tree, written from the board (`src/boot/tree.rs`): generic node names (Devicetree Specification 2.2.2), each node as its binding describes it, the GIC with each frame the size it decodes, `dma-coherent` on each device that makes DMA, the processor's caches down to the L2 as `CCSIDR_EL1` reports them, `/psci` for PSCI 1.0 and 0.2 on the conduit the firmware answers on, `/chosen` with EBBR's `stdout-path` and no boot entropy (see "Where the references differ"), and the identity's `/model` and root `compatible`; a `pmu` node of the PMU binding, the processor's own (`arm,cortex-a57-pmu`) or the generic PMUv3's (`arm,armv8-pmuv3`) with PMUIRQ on PPI 23; no `fw-cfg` or platform bus node, which the board does not have; with EL2, the virtual interface's two frames and its maintenance interrupt in the GIC node; a host's own tree gets its names the identity's, every memory node and its `/psci` node replaced and `/chosen` added where it has none, as QEMU's `-dtb` does, and is refused where it puts a device anywhere but where the layout has it (`BuildError::HostTreeAddress`) or names a device's interrupt other than the one the board's interrupt map wires it to (`BuildError::HostTreeInterrupt`) |
| Boot | UEFI firmware from flash; or a Linux `Image`, initramfs and device tree placed and entered as Linux's arm64 boot protocol (`Documentation/arch/arm64/booting.rst`) asks — the kernel at the base of RAM, the device tree in the 2 MiB block after its `image_size`, the initramfs in the next, `x0` the tree and every other general-purpose register zero — and the UART's console set up at 115200 baud, 8N1, as firmware leaves it (see "Where the references differ") |

Provenance: Arm's documents decide wherever they speak — the Arm ARM (DDI
0487) and its machine-readable data (AARCHMRS) for the processor, IHI 0048B
for the GIC, DDI 0183F and its r1p5 issue DDI 0183G for the PL011, DDI
0224C for the PL031, DDI 0190B for the PL061, DEN 0022E for PSCI and DEN
0028F for the SMC Calling Convention (and their earlier issues D and B,
whose sections the older citations name), and BSA (DEN0094C) and V-BSA
(DEN0150) for what the platform as a whole provides. QEMU's `virt` board decides what those documents
do not: the memory map, the interrupt wiring, the device tree, the choices
Arm leaves to an implementation, and the devices Arm does not document
(virtio, the CFI flash, `gpio-key`). A device with its own published
specification is held to it as to Arm's: the virtio devices to OASIS virtio
1.2, the display's EDID to VESA E-EDID 1.4. No limit comes from one guest's
driver. What the board tells a guest it is —
the device tree's model, the virtio vendor and device names, the display's
EDID — is its own, not QEMU's. Each device cites its Arm
document or specification where there is one, then the QEMU source file and
symbols it was ported from, and every departure from QEMU is listed under
"Where the references differ". The QEMU reference is release 11.1.2 and its newest
`virt` machine, `virt-11.1`.

Beyond the Cortex-A57, the Armv9.2 model implements the features of an
Armv9.2 processor: FEAT_LSE (the atomic memory operations, SWP,
CAS, CASP), FEAT_LSE2's alignment rules, FEAT_LRCPC (LDAPR) and FEAT_LRCPC2
(LDAPUR, STLUR), FEAT_RDM (SQRDMLAH, SQRDMLSH), FEAT_DotProd (SDOT, UDOT)
and FEAT_I8MM (USDOT, SUDOT, SMMLA, USMMLA, UMMLA), each in its Advanced
SIMD forms; FEAT_FP16 (half-precision arithmetic, scalar and Advanced SIMD,
with FPCR.FZ16), FEAT_FHM (FMLAL, FMLSL, FMLAL2, FMLSL2), FEAT_FCMA (FCMLA,
FCADD), FEAT_JSCVT (FJCVTZS), FEAT_FRINTTS (FRINT32Z, FRINT32X, FRINT64Z,
FRINT64X) and FEAT_BF16 (BFCVT, BFCVTN, BFDOT, BFMLALB, BFMLALT, BFMMLA);
FEAT_PAuth with FEAT_PAuth2, FEAT_FPAC, FEAT_FPACCOMBINE and the QARMA5
algorithm (FEAT_PACQARMA5), as QEMU's `max` has them: the keys, every PAC,
AUT, XPAC and PACGA instruction and hint, the branches, returns, exception
returns and loads that authenticate, and TCR_EL1.TBID0/TBID1; FEAT_BTI
(BTI, PSTATE.BTYPE, guarded pages, the Branch Target exception and
SCTLR_EL1.BT0/BT1); FEAT_FlagM and FEAT_FlagM2 (CFINV, RMIF, SETF8,
SETF16, XAFLAG, AXFLAG) and FEAT_SB; FEAT_PAN, FEAT_PAN2 and FEAT_PAN3
(PSTATE.PAN and its refusal of EL1 data accesses to memory EL0 can reach,
SCTLR_EL1.SPAN and EPAN, AT S1E1RP and AT S1E1WP), FEAT_UAO (LDTR and STTR
with EL1's permissions), FEAT_DIT, and FEAT_SSBS with FEAT_SSBS2, each
PSTATE field with its MSR forms, register, SPSR bit and exception-entry
rule; FEAT_TGran16K (the 16 KiB granule, beside the 4 KiB and 64 KiB
ones both models have) and FEAT_TTST (input addresses down to 16 bits, 17
with the 64 KiB granule), each at stage 1, and built for stage 2 too —
FEAT_S2TGran16K, FEAT_GTG's stage 2 granule fields, and FEAT_TTST's
small stage 2 tables and stage 2 walk starting at level 3 with the 4 KiB
granule — which no guest reaches yet: this model cannot have EL2
(`CpuModel::el2_available`); FEAT_LPA and FEAT_LVA (a 52-bit physical
address size, `ID_AA64MMFR0_EL1.PARange` 0b0110, and with the 64 KiB
granule 52-bit input addresses, `TxSZ` down to 12, output addresses
whose bits 51:48 a descriptor holds in bits 15:12 and, under a 52-bit
`PS`, a table base register in bits 5:2, and a block at level 1), at
stage 1 and built for stage 2 (a 52-bit IPA, and `HPFAR_EL2.FIPA` to
bit 43); FEAT_LPA2 (the same with the 4 KiB and 16 KiB granules under
`TCR_EL1.DS`, `TCR_EL2.DS` or `VTCR_EL2.DS`: `TxSZ` down to 12, a walk
from level −1 with the 4 KiB granule, and at stage 2 `VTCR_EL2.SL2`'s
start there, its faults' own status codes, output addresses whose bits
51:50 a descriptor holds in bits 9:8 and 49:48 in 49:48, a table base
register's bits 5:2 whatever the output size, blocks at level 0 with the
4 KiB granule and level 1 with the 16 KiB one, and the shareability of
a leaf from the `TCR` or `VTCR_EL2`; `ID_AA64MMFR0_EL1.TGran4` 1,
`TGran16` 2, and with EL2 `TGran4_2` and `TGran16_2` 3), at stage 1 and
built for stage 2; FEAT_HAF and FEAT_HAFDBS (`ID_AA64MMFR1_EL1.HAFDBS`
2: with `TCR_EL1.HA` the walk sets a Block or Page descriptor's clear
Access flag rather than faulting on it, and with `HD` too a write to a
writable-clean descriptor, `DBM` and `AP[2]` set, clears `AP[2]` rather
than faulting, each an atomic swap of the descriptor in memory once the
access's checks pass, the translation made again where the descriptor
changed under it, and the TLB keeping no descriptor whose flag is clear
and a writable-clean one without its write permission, so that a write
always reaches the update; an access its checks refuse updates nothing,
nor does an address translation instruction's leaf, while cache
maintenance, which faults on a clear flag here, sets it), at stage 1 and
built for the EL2 regime (`TCR_EL2.HA`, `HD`) and stage 2 (`VTCR_EL2.HA`,
`HD`, `S2AP[1]` set on a write, and stage 1's update of its own
descriptor needing stage 2's write permission, which stage 2's own
update of a writable-clean table page gives); FEAT_HPDS
(TCR_EL1.HPD0 and HPD1), FEAT_TTCNP (TTBRn_EL1.CnP),
FEAT_E0PD (TCR_EL1.E0PD0 and E0PD1) with the FEAT_CSV3 it requires,
FEAT_TLBIOS, FEAT_TLBIRANGE and FEAT_TTL (the Outer Shareable and range
TLBI forms and the TTL hint), FEAT_XS (DSB nXS and the TLBI nXS forms),
FEAT_DPB and FEAT_DPB2 (DC CVAP, DC CVADP), and FEAT_LOR (LDLAR, STLLR and
the LORegion registers, with no LORegions); FEAT_IDST (reads of the feature
ID space that would be UNDEFINED trap with EC 0x18), FEAT_SPECRES (CFP, DVP
and CPP RCTX, with SCTLR_EL1.EnRCTX), FEAT_CSV2, FEAT_ECV (CNTPCTSS_EL0,
CNTVCTSS_EL0 and CNTKCTL_EL1.EVNTIS) and FEAT_WFxT (WFET and WFIT, which
wait until their timeout); FEAT_RAS with FEAT_RASv1p1 and FEAT_RASSAv1p1
(ESB, ERRIDR_EL1 with no error records, DISR_EL1, and the error state a
synchronous external abort reports), and FEAT_Debugv8p1, FEAT_Debugv8p2
and FEAT_Debugv8p4 (`DebugVer`, DBGAUTHSTATUS_EL1's always-enabled
non-invasive debug, a breakpoint type that matches `CONTEXTIDR_EL1` and the
watchpoint number in a Watchpoint exception's syndrome; what else they
change is halting and external debug and the PMU's counting in Secure
state, which this model, with neither and no Secure state, does not
have); FEAT_AFP (FPCR.FIZ, FPCR.AH's alternate
floating-point behaviours in every scalar, Advanced SIMD and BFloat16
instruction, and FPCR.NEP's merging of scalar results into the source
register each instruction names) and FEAT_RPRES (12-bit single-precision
FRECPE and FRSQRTE under FPCR.AH); FEAT_PMUv3 with FEAT_PMUv3p1 (16-bit
event numbers, `MDCR_EL2.HPMD`), FEAT_PMUv3p4 (`PMMIR_EL1`), FEAT_PMUv3p5
(64-bit event counters overflowing at bit 31 or 63 by `PMCR_EL0.LP` and
`MDCR_EL2.HLP`, `MDCR_EL2.HCCD`) and FEAT_PMUv3p7 (`PMCR_EL0.FZO` and
`MDCR_EL2.HPMFZO`, freezing the counters on overflow, and `PMCR_EL0.IMP`
reading as zero), the PMU revisions Arm requires of an Armv8.7 and later
processor with a PMU, and `ID_AA64DFR0_EL1.MTPMU` 0b1111, which Armv8.6
and later require without FEAT_MTPMU;
and FEAT_SME and FEAT_SME2 (FEAT_SME's state, registers, SMSTART
and SMSTOP, RDSVL, ADDSVL and ADDSPL, and the SVE and SVE2 integer,
element count, immediate, permute, compare, predicate and WHILE
instructions, the integer dot products (SVE's SDOT and UDOT, FEAT_I8MM's
USDOT and SUDOT) and SVE2's complex, long and saturating multiply-adds,
multiplies by element and predicated shifts, halving, pairwise and
saturating operations, SCLAMP and UCLAMP (FEAT_SME's), its widening,
narrowing and accumulating integer instructions, the contiguous loads,
stores and prefetches, and the SVE and SVE2 floating-point and BFloat16
instructions it makes legal in Streaming SVE mode, 823 encodings, which
outside that mode take the SME trap, this processor having no FEAT_SVE;
and the instructions on the ZA array: the outer products into 32-bit
tiles of single-precision values, of half-precision and BFloat16 pairs
and of bytes, MOVA to and from a tile slice, ZERO, ADDHA and ADDVA, the
tile slice loads and stores LD1B to LD1Q and ST1B to ST1Q, and LDR and
STR of an array vector, 39 encodings; and of FEAT_SME2, ZT0 with ZERO,
LDR and STR of it and the traps of `SMCR_EL1.EZT0` (to EL1) and, with
EL2, `SMCR_EL2.EZT0` (to EL2), MOVT between it and a
general register UNDEFINED as outside Debug state, the
predicate-as-counter with WHILE to a counter or to a pair of predicates,
PTRUE, PEXT and CNTP of one, the contiguous loads and stores of two or
four vectors, consecutive or strided, under one, and the SVE operations
on groups of two or four vectors (SEL under a counter; the integer and
floating-point maxima and minima, rounding shifts, additions, doubling
multiplies and clamps; ZIP and UZP of elements and of quadwords; the
rounding and saturating narrows; BFCVT and FCVT; the conversions between
words and single precision, FRINT and the unpacks), 257 encodings, and
the instructions on groups of ZA array vectors (the four-way and two-way
long integer products, the long products of half-precision and BFloat16
values, the dot products of bytes, of halfwords and of half-precision
and BFloat16 pairs, FMLA and FMLS of single-precision values, ADD and
SUB of vectors to them, and ADD, SUB, FADD and FSUB of vectors into
them, of one, two or four vectors by one or by as many), 121 encodings,
and their forms by an indexed element with the vertical dot products
FVDOT, BFVDOT, SVDOT, UVDOT, USVDOT and SUVDOT, 70 encodings, and BMOPA
and BMOPS, SMOPA to UMOPS of halfwords into 32-bit tiles, MOVA between
two or four registers and as many tile slices or ZA array vectors, and
LUTI2 and LUTI4 of ZT0 to one, two or four registers, 32 encodings, and
the SVE2.1 instructions it makes legal (SDOT and UDOT of halfword pairs,
SQRSHRN, SQRSHRUN, UQRSHRN, SQCVTN, SQCVTUN and UQCVTN of two vectors,
FCLAMP, FDOT of half-precision pairs, and BFMLSLB and BFMLSLT), 17
encodings).
Its feature set is one Arm permits: every feature Arm's `Features.json`
requires of an Armv9.2 processor with FP and Advanced SIMD and AArch64 at
EL0 and EL1 alone, with SME, SME2 and the options listed above, and
`tests/unit/spec.rs` holds it to every constraint of `Features.json` that
its identifiers decide (1,306 of 1,694), as it holds the Cortex-A57's
(1,337). Linux 6.18 finds every one of these features in its ID registers,
its SME vector lengths from 16 to 64 bytes, and the 1 GHz counter, and
boots Alpine to a shell on it.

Built with `MachineBuilder::with_el2` (QEMU's
`-M virt,virtualization=on`), the Cortex-A57 has FEAT_EL2, as QEMU's does,
and the core runs a hypervisor's EL2 as Arm's pseudocode defines it: the
processor resets at EL2h (`AArch64_TakeReset`, there being no EL3), with
SP_EL2 and the EL2 system registers, each keeping only the bits Arm's data
gives it; exceptions are taken to EL2 as `HCR_EL2.TGE`, IMO, FMO, AMO and
TEA route them, and every trap `HCR_EL2`, `CPTR_EL2`, `MDCR_EL2`,
`HSTR_EL2` and `CNTHCTL_EL2` define for the features a processor has
reaches EL2 with Arm's syndrome (with FEAT_SME, `CPTR_EL2.TSM` and `TFP`
trap the SME instructions after `CPACR_EL1`'s enables, `SMCR_EL2.EZT0`
traps ZT0 at EL0, EL1 and EL2, and `SMCR_EL2.LEN` caps the Streaming SVE
vector length below EL2 and sets it at EL2); HVC calls the hypervisor
(`AArch64_CallHypervisor`, UNDEFINED under `HCR_EL2.HCD`) and SMC is
trapped by `HCR_EL2.TSC`; ERET from EL2 returns to EL2, EL1 or EL0, with
the illegal returns `AArch64_ExceptionReturn` defines; EL2's own stage-1
translation regime has its own TLB entries, which the `TLBI …E2` forms and
`AT S1E2R` and `AT S1E2W` reach; and the EL2 physical timer interrupts on
PPI 10, the virtual counter reads `CNTVOFF_EL2` back from the physical one,
and `CNTHCTL_EL2` traps EL1's and EL0's timer accesses and drives the event
stream EL2 sees. A processor that has FEAT_ECV_POFF as well (none of the
models yet: the Armv9.2 model, which has FEAT_ECV, has no EL2) has
`CNTPOFF_EL2`: under `CNTHCTL_EL2.ECV`, EL0 and EL1 read the physical count
less it, and the EL1 physical timer compares that count at every level
(Arm `PhysicalOffsetIsValid`, `AArch64_CheckTimerConditions`).
`HCR_EL2.VI`, `VF` and `VSE` raise a virtual IRQ, FIQ
and SError for EL1 and EL0 where `IMO`, `FMO` and `AMO` route the physical
kind to EL2: `ISR_EL1` shows them, each is taken to EL1 when its `PSTATE`
mask is clear (after a deliverable physical interrupt, SError before FIQ
before IRQ: Arm leaves the order IMPLEMENTATION DEFINED; see "Where the
references differ"), a pending one
ends `WFI` and `WFE`, a virtual SError reports `VSESR_EL2` (FEAT_RAS) or,
without FEAT_RAS, no syndrome (see "Where the references differ"), and
ESB defers a masked one into `VDISR_EL2`. The machine's PSCI firmware then
answers `SMC`, as QEMU's does (see "Where the references differ"), and the
device tree says so. Without it, the default, the processor has no EL2: it
resets at EL1, `HVC` is the PSCI call, and the EL2 registers and
instructions are UNDEFINED. The level set is a type parameter of the
machine and its processor (`NoEl2` or `WithEl2`), so a processor without
EL2 holds no EL2 state and every EL2 rule, testing a constant, compiles
away from it.

Under `HCR_EL2.VM` or `DC`, stage 2 translates the EL1&0 regime's
intermediate physical addresses as Arm's `AArch64_S2Translate` and
`AArch64_S2Walk` define it: `VTCR_EL2`'s `T0SZ`, `SL0`, `TG0` and `PS`,
with each granule `ID_AA64MMFR0_EL1` offers at stage 2 (on the Cortex-A57
the 4 KiB and 64 KiB ones, `TGran4_2`, `TGran16_2` and `TGran64_2` reading
0, "as stage 1"), `SL0` naming the start level as each granule decodes it,
up to 16 concatenated start tables, the configurations Arm refuses taking a
level 0 Translation fault; `S2AP`, `XN` and the access flag (which with
FEAT_HAF's `VTCR_EL2.HA`, on a processor that has it, the walk sets, and
with FEAT_HAFDBS's `HD` a write to a leaf whose `DBM` is set makes
writable, as stage 1's are); and `MemAttr`
combined with stage 1's attributes by `S2CombineS1MemAttrs`, so that
Device memory at either stage refuses a misaligned access. Stage 1's walk
reads its descriptors through stage 2 too, `HCR_EL2.PTW` refusing it
Device memory there. A stage 2 abort is taken to EL2 with `HPFAR_EL2`
holding the IPA where `IPAValid` says so, `S1PTW` set on stage 1's walk,
and for a load or store of one general-purpose register without writeback
the instruction syndrome (`ISV`, `SAS`, `SSE`, `SRT`, `SF`, `AR`) a
hypervisor's emulation of MMIO reads. `AT S12E*` translates by both
stages and reports the combined attributes, `AT S1E*` the IPA, and a
debugger's `virt_to_phys` walks both stages. The TLB holds both stages'
results, each stage's permissions and memory type apart. A translation
larger than 4 KiB — a block, a 16 KiB or 64 KiB page, at either stage —
is held whole, of the size both stages map alike: one walk serves every
page of it, an access that misses its own page's entry finding the
translation by its first page's, while the lookup an access makes on a
hit stays a 4 KiB one. A TLB maintenance instruction removes what Arm's
`TLBIMatch` says it names and nothing else (`src/cpu/tlbi.rs`): by
regime, VMID and ASID, a global translation (`nG` clear) outliving a
`TLBI ASIDE1`; by address, a translation matching any address in the
leaf whose information it holds, the whole of a block a `TLBI VAE1`
anywhere in it names (Arm ARM D8.17 RLGSCG); by the level and granule a
FEAT_TTL hint names; by a FEAT_TLBIRANGE range, its base address in
FEAT_LPA2's form under `DS`; and by stage, `TLBI IPAS2E1` removing the
translations of stage 2 alone — the kept stage 2 translations of IPAs,
and those made with stage 1 disabled — and `TLBI VMALLE1` those holding
stage 1's, so that a hypervisor's change of stage 2 takes both, as Linux
KVM issues them. Its EL1&0 entries are tagged with the
context they were made in — the values of the EL1 translation registers,
`VTTBR_EL2`, `VTCR_EL2` and the `HCR_EL2` bits that decide which walks
are made (`VM`, `DC`, `PTW`, `TGE`) — up to fifteen contexts at once, so
a hypervisor's switch to its own registers and back to a guest's finds
the translations each had, where a write that changes one of the
registers makes the next access find its context anew, never using an
entry made under other values. The stage 2 translations of the IPAs a
walk reads its descriptors at, and of its output, are kept beside the
entries, tagged with stage 2's context alone (`VTTBR_EL2`, with its VMID,
and `VTCR_EL2`; fifteen at once), as Arm tags them with the VMID: a
guest's switch between its processes keeps them, and one translation
walks stage 2 once for each IPA page it reads. The GIC's Virtualization
Extensions (`src/devices/gicv2_virt.rs`) give a hypervisor the List
registers and the guest its virtual CPU interface; the processor takes
their virtual IRQ and FIQ as `HCR_EL2.VI` and `VF` would raise them (Arm
`AArch64_InterruptPending`), routed by `IMO` and `FMO`.
FEAT_VHE (`src/cpu/el2.rs`, `src/cpu/regime.rs`) gives a processor with
EL2 `HCR_EL2.E2H`, writable (FEAT_E2H0), and with it the EL2&0
translation regime (Arm `ELIsInHost`): `TCR_EL2` in `TCR_EL1`'s layout
with two address ranges, `TTBR1_EL2`, and the ASIDs of `TTBR0_EL2` and
`TTBR1_EL2` tagging non-global translations; `SCTLR_EL2`, `CPTR_EL2` and
`CNTHCTL_EL2` in their `E2H` 1 layouts, each register's value relaid
as `E2H` changes; at EL2 the EL1 register names reaching the EL2
registers and the `*_EL12` and `*_EL02` encodings reaching the EL1 and
EL0 registers; and with `HCR_EL2.TGE` too, EL0 in the host — running in
the EL2&0 regime under `SCTLR_EL2`, its exceptions and interrupts taken
to EL2, `CPTR_EL2` gating its FP, SIMD and SME as `CPACR_EL1` gates
EL1's, `CNTHCTL_EL2` its counter and timer accesses, and its `LDTR` and
`STTR` checked as EL0's at EL2, where `PAN` applies (Arm
`AArch64_IsUnprivAccessPriv`, D8.13.1), an exception taken to EL2 in the
host setting `PAN` where `SCTLR_EL2.SPAN` is 0. One taken to EL2 with
`E2H` set and `TGE` clear keeps `PAN`: `AArch64_TakeException` leaves
setting it there IMPLEMENTATION DEFINED, required only with FEAT_NV3,
which the core does not have. `TLBI *E2*` under `E2H` and
`TLBI *E1*` in the host name the EL2&0 regime's translations by ASID and
address as `TLBIMatch` has them (`src/cpu/tlbi.rs`); `AT S1E2*`
translates in EL2&0 under `E2H`, `AT S1E0*` in the host. The EL2 and
EL2&0 regimes' entries carry no context tag: a write that changes what
their walks read — `E2H` among it, as Arm ARM D8.13.1 lets a TLB cache
`E2H` (ISTSYY), and a `TTBR0_EL2` or `TTBR1_EL2` write switching ASIDs —
empties the TLB (see "Not implemented").
The other EL2 features `Features.json` requires of an Armv9.2
processor with EL2 are built beside it. FEAT_FGT (`src/cpu/fgt.rs`):
`HFGRTR_EL2`, `HFGWTR_EL2`, `HFGITR_EL2`, `HDFGRTR_EL2` and
`HDFGWTR_EL2`, each with the fields of the registers and instructions
the processor has (the others RES0), trap EL1's accesses, and EL0's
outside the host, to EL2 where each accessor checks them — among its
traps to EL2, after EL0's own traps to EL1, ahead of every other
outcome — with the access's EC 0x18 syndrome; `HFGITR_EL2.SVC_EL0` and
`SVC_EL1` take an `SVC` to EL2 returning to it, and `ERET` takes EL1's
`ERET`, `ERETAA` and `ERETAB` to EL2 with EC 0x1A; `MDCR_EL2.TDCC`
traps the Debug Communications Channel; and the PMU's accesses to a
counter `MDCR_EL2.HPMN` reserves trap to EL2, to one the PMU lacks are
UNDEFINED, as the accessors make them with FEAT_FGT. FEAT_HCX:
`HCRX_EL2` with `FnXS` and `FGTnXS` (FEAT_XS), under which the
fine-grained traps of the TLB maintenance instructions spare their nXS
forms, and `FNB`, which Arm's data gives every processor with FEAT_HCX;
`SMPME` is RES0 with `SMIDR_EL1.SMPS` 0, and the controls of features the
processor lacks are RES0. FEAT_EVT and FEAT_EVT2 (`ID_AA64MMFR2_EL1.EVT`
2): `HCR_EL2.TID4` traps `CCSIDR_EL1`, `CLIDR_EL1` and `CSSELR_EL1`,
`TICAB` `IC IALLUIS`, `TOCU` the maintenance to the PoU (`IC IALLU`, `IC
IVAU`, `DC CVAU`), `TTLBIS` and `TTLBOS` the Inner and Outer Shareable
TLB maintenance. FEAT_XNX (`src/cpu/stage2.rs`): a stage 2 descriptor's
`XN[1:0]` gives EL1 and EL0 their own execute permission. FEAT_S2FWB:
under `HCR_EL2.FWB` a stage 2 `MemAttr` makes the access's attributes
from stage 1's by `AArch64_S2ApplyFWBMemAttrs` — Device where its bit 2
is clear, stage 1's own, forced Write-Back, or Non-cacheable — which the
TLB holds by `FWB`'s rule, each stage 2 context and TLB context carrying
`FWB`, and `AT S12E*` reports in `PAR_EL1`; the processor's `CLIDR_EL1`
names no cache to the PoU. The Armv9.2 model is refused EL2
(`BuildError::El2Unavailable`): its feature set (`Features::ARMV9P2`)
lists none of FEAT_VHE, FEAT_XNX, FEAT_S2FWB, FEAT_EVT, FEAT_EVT2,
FEAT_FGT and FEAT_HCX, which `Features.json` requires of an Armv9.2
processor with EL2, and an Armv9.2 processor without them is not one Arm
permits; the Cortex-A57, an ARMv8.0 processor, has none of them
(`ID_AA64MMFR1_EL1.VH` 0, the `*_EL12` encodings UNDEFINED, the
fine-grained trap registers and `HCRX_EL2` UNDEFINED, its `ID_AA64MMFR0_EL1`
to `ID_AA64MMFR2_EL1` and `CLIDR_EL1` as before). So these features are
tested on the test processor with every feature
(`tests/unit/vhe.rs`, `tests/unit/el2_features.rs`, and
`tests/unit/sysreg_spec.rs` over every state of the trap registers) and
reach a guest with EL2 on the Armv9.2 model, where Linux's KVM runs in
VHE mode, once the model is given EL2 and them (plan item F3).

## Verification

The unit tests that need the library's private items are compiled into the
library under `#[cfg(test)]`: most live next to `src`, in `tests/unit/`,
through `#[path]` module declarations, and some sit in a `tests` module at
the end of the file they test (the device models, the boot code, and
parts of the processor and of SoftFloat). The files directly in `tests/` are
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
  the processor with every feature: 35,400 vectors of FEAT_LSE (LDADD,
  LDCLR, LDEOR, LDSET, LDSMAX, LDSMIN, LDUMAX, LDUMIN and SWP of every size
  and ordering, CAS, CASP), FEAT_LRCPC and LRCPC2 (LDAPR, LDAPUR*,
  STLUR*), FEAT_RDM (vector, scalar and by element), FEAT_DotProd and
  FEAT_I8MM (vector and by element, and the matrix multiplies), FEAT_FP16
  (the scalar data processing, compares, selects, FMOV and conversions, and
  the Advanced SIMD three-same, miscellaneous, by-element, across-lanes,
  pairwise, FMOV and fixed-point forms), FEAT_FHM (vector and by
  element), FEAT_FCMA (FCMLA and FCADD of every precision, vector and by
  element), FEAT_JSCVT, FEAT_FRINTTS (scalar and vector), FEAT_BF16
  (every instruction), FEAT_FlagM and FEAT_FlagM2 (CFINV, XAFLAG, AXFLAG,
  RMIF, SETF8, SETF16), FEAT_SB, FEAT_LOR (LDLAR and STLLR of every size)
  and FEAT_XS (DSB nXS), and 21,800 of FEAT_AFP and FEAT_RPRES: every
  floating-point family — scalar and Advanced SIMD, single, double and half
  precision, BFloat16, FHM, FCMA, JSCVT, FRINTTS — again under FPCR.FIZ,
  AH and NEP at random. The floating-point ones are from special and
  ordinary inputs under a random FPCR that sets FZ16 too, in the same
  format from the same initial states, generated by
  `tests/vectors/gen_qemu.py` with QEMU 11.1.2's user-mode `max` processor
  (whose FEAT_FPRCVT, FEAT_FAMINMAX and FP8 encodings the families steer
  clear of) and `docs/patches/qemu/qemu-afp-vector-fabs-fneg.patch`, the fix for vector
  FABS and FNEG under FPCR.AH, which changes nothing else: a static program
  (`qemu_harness.c`, `qemu_trampoline.S`)
  runs each word from the whole register state and data region and
  reports both back. QEMU 11.1.2 reproduces the 13,200 vectors QEMU 8.2.2
  generated before the LOR and XS families were added, and gets right the
  instructions of five families
  that 8.2.2 gets wrong ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)), which the vectors
  now hold; with the patch it reproduces the 13,600 that came before the
  FEAT_AFP families.
  Run through that harness, a quarter of `unicorn.txt` agrees with Unicorn
  on all of its 5,895 vectors but one, an LDP with `Rt == Rt2` (CONSTRAINED
  UNPREDICTABLE). Breaking LDCLR, CASP's pair order, USDOT's operand
  signedness, SQRDMLAH's rounding, FZ16's flush of a tiny result,
  FMLAL2's choice of the upper half, FCMLA's negation at 90° or BFDOT's
  rounding to odd on purpose fails 14 to 590 of them.
  `tests/unit/simd_extensions.rs` checks the sizes the RDM and dot product
  pages' decode pseudocode makes UNDEFINED, which QEMU refuses too, that
  the SIMD&FP and Streaming SVE traps come after decode, and that the
  64-bit PMULL is UNDEFINED on a processor with FEAT_AES and not
  FEAT_PMULL;
  `tests/unit/fp16.rs` checks FZ16 against FPUnpack, FPRound and FPMulAddH
  (no input denormal exception for half precision, underflow alone when a
  tiny result is flushed, FMLAL's addend under FZ), that a guest can set
  it, and the forms QEMU 8.2.2 gets wrong ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md));
  `tests/unit/fp_extensions.rs` checks FEAT_FCMA, FEAT_JSCVT, FEAT_FRINTTS
  and FEAT_BF16 against FPToFixedJS, FPRoundIntN, FPConvertBF (its ties,
  which random inputs almost never hold), BFDotAdd and BFMulAddH, the sizes
  their decode pseudocode refuses, FCMA's half-precision forms needing
  FEAT_FP16, and what QEMU 8.2.2 gets wrong;
  `tests/unit/afp.rs` checks FEAT_AFP and FEAT_RPRES where random inputs
  rarely land, each case against its pseudocode (FPProcessNaNs and
  FPProcessNaNs3's choice by register, FPDefaultNaN's sign, FPUnpackBase's
  FIZ and FZ, FPRoundBase's tininess and flush after rounding, the cases
  FPProcessDenorms leaves out, FPMax and FPMaxNum, FPAbs and FPNeg on NaNs,
  the steps and estimates raising nothing, the BFloat16 operations, FZ16
  after rounding, and IsMerging's source register for each kind of
  instruction, and none in Streaming SVE mode), and that a guest sets
  FIZ, AH and NEP only with FEAT_AFP.
  `tests/unit/atomic.rs` checks what the vectors cannot,
  over a stage-1 table of Normal, read-only and Device memory: an atomic
  reports a permission fault on its write as a write and its alignment and
  translation faults as reads, a CAS whose comparison fails writes nothing
  but still needs write permission, CASP with an odd register is
  UNDEFINED, the LSE2 and `nAA` alignment rules with and without the
  feature, signed LDAPUR offsets, the SP alignment check, and Device
  memory's alignment fault ahead of a permission fault on the same page
  (`AArch64_S1Translate`), for a load, a store, an atomic, a page-crossing
  access and DC ZVA.
- `tests/unit/pointer_auth.rs` replays `tests/vectors/qemu_pauth.txt`, on
  the processor with every feature: 2,400 vectors of pointer
  authentication in eight families (PAC*, AUT*, XPAC*, the hints, PACGA,
  the authenticating branches and returns, ERETAA/ERETAB, LDRAA/LDRAB),
  generated by `tests/vectors/gen_qemu_pauth.py` with QEMU 11.1.2's system
  emulator (8.2.2 makes the same file), because user mode cannot choose
  the keys: a bare-metal EL1 program (`qemu_pauth_runner.S`,
  `qemu_pauth.ld`) on `-M virt -cpu max,pauth-qarma5=on`
  sets random keys, key enables and TCR_EL1 size, TBI and TBID fields,
  usually signs what the word authenticates and sometimes breaks it with a
  flipped bit, then reports what the word did: the registers, a landing
  on the branch target, or the PAC Fail syndrome. All of them agree. The
  same file checks what QEMU's `max` cannot show against Arm's pseudocode:
  the processors without FEAT_PAuth2 (the code replacing the pointer's
  bits, the corrupted code of a non-canonical pointer, the error code of a
  failed authentication), with FEAT_FPAC but not FEAT_FPACCOMBINE, and
  without FEAT_PAuth (the hints NOPs, the rest UNDEFINED), the keys as
  EL1 registers and in a snapshot, LDRAA's SP alignment check after the
  authentication, and a tag TBID keeps faulting the fetch with the MMU on
  and off, a TLB entry a load filled for the same address included.
  Making the signing take its half of the address space from bit 55 alone
  fails both the replay and the test that pins InsertPAC's choice.
- `tests/unit/bti.rs` checks FEAT_BTI against Arm's pseudocode, over a
  stage-1 table with guarded and unguarded blocks: the BTYPE each branch
  leaves (BR in and out of a guarded page, through X16 and X17 and other
  registers, BLR, RET, the authenticating forms), which BTI, PACIASP and
  PACIBSP (under SCTLR_EL1.BT0 and BT1) and BRK accept it, the check
  before an UNDEFINED decode, no check outside a guarded page or with the
  MMU off, BTYPE kept in SPSR_EL1 by an instruction abort, a BRK and an
  IRQ and zeroed by an SVC, restored by ERET, cleared by a host write of
  the PC, carried by a snapshot with the TLB's guarded pages (which only a
  processor with the feature restores), and none of it on a processor
  without the feature. Leaving PSTATE.BTYPE as the faulting instruction
  found it fails the BRK case.
- `tests/unit/pstate_features.rs` checks FEAT_PAN, PAN2, PAN3, UAO, DIT,
  SSBS and SSBS2 against Arm's pseudocode, over a stage-1 table of EL1's
  memory, EL0 read/write, EL0 execute-only and EL0 read-only blocks: PAN
  refusing an EL1 load, store, atomic (WnR clear: its read is refused
  first), exclusive and DC ZVA to memory EL0 can read or write, and with
  SCTLR_EL1.EPAN to memory EL0 can only execute; LDTR and STTR with EL0's
  permissions, or EL1's under UAO, which PAN then applies to; MSR
  (immediate) and the PAN, UAO, DIT and SSBS registers at EL1 and EL0; AT
  S1E1RP and S1E1WP against AT S1E1R and S1E1W; what taking an exception
  and ERET, legal and illegal, do to each field, an illegal return's
  Illegal Execution state exception coming after the next fetch's PC
  alignment fault and Instruction Abort, and ERET restoring PSTATE.SS only
  into an enabled debug target (`DebugExceptionReturnSS`); SPSR_EL1 holding them only
  with their features, a snapshot whose SPSR_EL1 or PSTATE holds one
  refused by a processor without it; and every encoding UNDEFINED without
  its feature. Ignoring PAN, EPAN or UAO, leaving PAN alone on exception
  entry or on an illegal return, or checking an atomic's write before its
  read each fails a test.
- `tests/unit/memory_system.rs` checks FEAT_HPDS, TTCNP, E0PD, TLBIOS,
  TLBIRANGE, XS, DPB, DPB2 and LOR, and the cache maintenance instructions
  by address every processor has, against Arm's pseudocode, over a stage-1
  table both halves of the address space share, with a table descriptor
  that takes EL0's access and both ELs' execution away: HPD0 and HPD1
  turning that off for their half alone; E0PD0 and E0PD1 faulting an EL0
  load, fetch, LDTR at EL1, DC and AT S1E0R at level 0, ahead of a
  permission or Device alignment fault and through a TLB entry an EL1
  access filled, with PAN still judging EL0's permissions there and a
  snapshot's TLB entry E0PD would refuse EL0 refused; DC and IC by address faulting with CM and WnR
  at the register's address, on an Access flag of 0 as well, DC IVAC needing
  write permission, a clean at
  EL0 read permission, PAN taking no part, and UCI's trap first; `AT` and
  `PAR_EL1` under `SCTLR_EL1.C` and for the reserved shareability value;
  snapshot TLB entries no walk could have produced refused; each later
  TLBI form UNDEFINED without its features and at EL0 and invalidating a
  stale translation with them; DSB nXS; LDLAR and STLLR as ordered
  accesses PAN applies to; the LORegion registers; and each feature's
  bits RES0 or its encodings UNDEFINED without it.
- `tests/unit/tlb_maintenance.rs` checks TLB maintenance against Arm's
  `TLBIMatch` and `TLBIRange`, by guest loads that read which page each
  reaches after its descriptor changed with no maintenance — the old one
  while the TLB holds the translation, the new one once a `TLBI` removed
  it: `VAE1` removing its page and leaving the pages beside it; a block
  and a 64 KiB page held whole, a page of them no access touched reaching
  the old output, and removed whole by a `TLBI` of any address in them;
  `ASIDE1` sparing the global pages and other ASIDs', `VAE1` of another
  ASID removing a global page alone; FEAT_TTL hints of the right and wrong
  level and granule, and a reserved one; FEAT_TLBIRANGE ranges, `TG`
  0b00 and another granule's invalidating nothing; `IPAS2E1` removing
  stage 2's translations alone, a stage 2 block whole, and with
  `VMALLE1` the translations of both stages, as KVM issues them;
  `VMALLS12E1` sparing another VMID's and `ALLE1` not; stage 2's
  translations of the tables stage 1 walks through held whole; each
  regime's maintenance keeping to its regime; FEAT_HAFDBS's updates
  reached through a block held whole; and a snapshot keeping what the
  TLB held, an entry whose output no walk of its level gives refused.
  Making every TLBI empty the TLB, holding blocks as 4 KiB pieces at
  either stage, ignoring `nG`, the TTL hint or the VMID each fails a
  test.
- `tests/unit/granules.rs` checks the translation granules and FEAT_TTST
  against Arm's pseudocode (`AArch64_S1StartLevel`,
  `AArch64_TTEntryAddressBounds`, `AArch64_BlockDescSupported`,
  `AArch64_S1DecodeTG0` and `TG1`, `AArch64_MaxTxSZ`,
  `AArch64_S2StartLevel`, `AArch64_S2InvalidSL`,
  `AArch64_S2InconsistentSL`, `AArch64_PACEffectiveTxSZ`): a load and
  `AT S1E1R` through stage 1 tables of each granule for input addresses
  from 48 bits down to FEAT_TTST's 16 (17 with the 64 KiB granule), each
  start level and page offset the granule's, the next page faulting at
  level 3 and the address above the range at level 0; a block a leaf only
  at the levels its granule has blocks at; the upper range walking with
  `TG1`'s granule; a granule the processor lacks (the Cortex-A57's 16 KiB
  one in both regimes), and each reserved encoding, walking as the 4 KiB
  one; the Cortex-A57's 64 KiB granule walking in both ranges of the
  EL1&0 regime and in EL2's; a `TxSZ` beyond its granule's range
  walking as the limit it passes, the first and last page of the limit's
  range translating and the address above it faulting at level 0, with
  FEAT_TTST and without, but below the minimum with FEAT_LVA faulting at
  level 0; `XPACI` ending the code
  where its granule's range does; and through stage 2 tables, each
  granule's `SL0` decode (the Cortex-A57's 64 KiB one among them),
  concatenated start tables, every invalid or
  inconsistent `SL0`, and a `T0SZ` below the minimum with FEAT_LPA, refusing the instruction fetch
  at level 0 with its IPA in `HPFAR_EL2`, and any other out-of-range `T0SZ`
  walking as its limit; each 4 KiB piece of a 64 KiB
  page reading its own piece under stage 2 of each granule; and a
  snapshot's TLB entry, or kept stage 2 translation, refused at a level no
  walk of the context it was made in ends at, or for an address that walk
  does not translate, at either stage and in either regime
  (`tests/unit/memory_system.rs` checks `EPD` and `T1SZ` there). It checks
  FEAT_LPA and FEAT_LVA the same way (`AArch64_PAMax`,
  `AArch64_PhysicalAddressSize`, `AArch64_S1TTBaseAddress`,
  `AArch64_NextTableBase`, `AArch64_S1LeafBase`, `AArch64_S2MinTxSZ`): a
  64 KiB descriptor's bits 15:12 as address bits 51:48, reported by `AT`
  at both stages under a 52-bit size and an Address size fault under a
  48-bit one, and not address bits without a 52-bit `PAMax`; a table base
  register's bits 5:2 under a 52-bit `IPS` or `PS` alone, at both stages;
  52-bit input addresses in both EL1&0 ranges and in EL2's, and the
  smallest `T0SZ` at stage 2 per granule; the code bits of `XPACI` up to
  bit 52; an untranslated address up to `PAMax`; and `HPFAR_EL2` holding
  a 52-bit IPA. It checks FEAT_LPA2's `DS` the same way
  (`AArch64_S1StartLevel`, `AArch64_S2StartLevel`, `AArch64_S2InvalidSL`,
  `AArch64_S1MinTxSZ`, `AArch64_S2MinTxSZ`, `AArch64_BlockDescSupported`,
  `AArch64_S1NextWalkStateLeaf`, `EncodeLDFSC`): walks of 52-bit input
  addresses from level −1 with the 4 KiB granule and from level 0 with the
  16 KiB one, in both EL1&0 ranges, in EL2's and under stage 2, and the
  least `TxSZ` with `DS` and without, on a processor with FEAT_LPA2 and
  without; a level −1 fault's own status codes (Translation, Address
  size, External abort on the walk, and stage 2's Translation); a
  descriptor's bits 9:8 and 49:48 as address bits 51:48 at both stages,
  reported by `AT` under a 52-bit size and an Address size fault under a
  48-bit one; a table base register's bits 5:2 under any output size;
  blocks at the 4 KiB granule's level 0 and the 16 KiB granule's level 1
  at both stages; stage 2's start at level −1 for `SL2:SL0` 0b100 and at
  level 0 for the 16 KiB granule's `SL0` 0b11, and the encodings that name
  no level; and a leaf's shareability from `TCR_EL1.SH0` or `SH1`,
  `TCR_EL2.SH0` and `VTCR_EL2.SH0`, as `PAR_EL1.SH` reports it.
  `tests/unit/el2.rs` checks each model's `PARange`, `VARange`, granule
  fields (the Cortex-A57's whole `ID_AA64MMFR0_EL1`, 0x24) and
  `CpuModel::physical_address_bits`.
- `tests/unit/load_store.rs` checks, over a bus that logs every access,
  what Arm's `Mem` and the load and store pages decide and the vectors
  cannot show: PRFM, PRFUM and PRFM (register) checking no SP alignment
  and accessing nothing; a register offset without `option<1>` UNDEFINED
  ahead of the SIMD&FP trap; a pair of Q registers at 8 mod 16 on Device
  memory as four aligned doubleword accesses, which `SCTLR_EL1.A` faults;
  LDXP and STXP of words as one doubleword access; and with FEAT_LSE2 a
  pair of words in one 16-byte granule as one access on Normal Write-Back
  Shareable memory and two on any other, or with the MMU off.
- `tests/unit/debug.rs` checks self-hosted debug against Arm's pseudocode,
  set up by the guest's own `MSR` and `ERET`: Software Step's states, its
  syndrome, what an SVC, another exception or an interrupt saves in
  `SPSR_EL1.SS`, and stepping code a machine runs from traces, across a
  snapshot; each model's count of breakpoints and watchpoints, as
  `ID_AA64DFR0_EL1` reports it, every one of them firing, the registers
  past them UNDEFINED, and the highest numbered the context-aware ones;
  breakpoints by address and by `CONTEXTIDR_EL1`, linked and
  linking, their enables and reserved values, inside a trace; and watchpoints
  by byte, word and masked range, at the level of the access (LDTR), on
  pairs, across pages and through the TLB, after a permission fault, on
  atomics, Store-Exclusives and `DC ZVA` and `DC IVAC`, on SVE, SME and
  streaming SIMD&FP accesses rounded to 16 bytes, and with FEAT_Debugv8p2's
  syndrome.
- `tests/unit/wait.rs` checks the waits every processor has and FEAT_WFxT's
  against Arm's pseudocode (`Hint_WFI`, `Hint_WFE`, `Hint_WFIT`,
  `Hint_WFET`, `TestEventCNTV`): SEV, SEVL and every exception return,
  legal or not, setting the Event Register and WFE consuming it; WFE and
  WFET completing on it ahead of the EL0 trap, WFI and WFIT on a pending
  interrupt, masked or not, and WFIT and WFET on a passed timeout, each
  trap's syndrome; the timed waits ending at the tick `CNTVCT_EL0` reaches
  the timeout and not one before; the event stream's event, rising or
  falling, ending a WFE at its tick and setting the register while the
  processor runs, `EVNTIS` with FEAT_ECV only, and a `CNTKCTL_EL1` write
  judging the count before it by the controls that held then; WFE not
  waiting while the exclusive monitor is armed; and snapshots keeping the
  wait, its timeout and the register. `tests/unit/sysreg.rs` checks
  FEAT_IDST, FEAT_SPECRES and FEAT_ECV's counter views, and `tests/api.rs`
  a machine whose WFE the event stream ends. Turning off any of 20
  of these rules on purpose fails a test.
- `tests/unit/sysreg.rs` also checks FEAT_RAS (ESB, ERRIDR_EL1, the error
  record registers reading as zero and ignoring writes, `ERXFR_EL1` and
  `ERXPFGF_EL1` read-only, DISR_EL1's bits and its snapshot, EL0 kept
  out, a synchronous external abort's syndrome the same with the feature
  as without) and DBGAUTHSTATUS_EL1 with and without FEAT_Debugv8p4;
  RVBAR_EL1, RMR_EL1 and MDRAR_EL1 at EL1 and EL0; DBGCLAIMSET_EL1
  reading as ones; and FEAT_DoubleLock's ID field and `OSDLR_EL1.DLK` on
  the Cortex-A57, which has it, and the Armv9.2 model, which may not;
  what each model reads in `MIDR_EL1`, `REVIDR_EL1`, `AIDR_EL1` and
  `SMIDR_EL1` (the Cortex-A57 r1p3; the software model's `SMIDR_EL1.SH`);
  `isa::tests` the RAS, DebugVer and DoubleLock fields of the ID
  registers.
- `tests/crypto.rs` runs the cryptographic and CRC32 instructions as a guest
  uses them, to the standards' own answers: AES-128 encryption and
  decryption of the FIPS-197 example, SHA-1 and SHA-256 of "abc" (FIPS
  180-4), the CRC-32 and CRC-32C check values of "123456789", 64-bit
  polynomial products; every instruction it builds is pinned to the word
  GNU as assembles; and `CPACR_EL1.FPEN` traps the cryptographic
  instructions, which are Advanced SIMD, but not CRC32, which is not.
- `tests/unit/difftest.rs` also replays `tests/vectors/qemu_sve.txt`, in
  Streaming SVE mode at a 512-bit vector length with ZA enabled and ZT0
  accessible: 40 vectors for each of the 238 encoding classes of
  SVE integer arithmetic, element counts, immediates, permutes, compares,
  predicate operations, WHILE, contiguous loads, stores and prefetches,
  floating point, and SVE2's integer instructions that FEAT_SME makes
  legal there, of the 13 classes of its ZA instructions, and of FEAT_SME2's
  136 (the 5 predicate-as-counter classes, the 16 of the multi-vector
  loads and stores, ZERO and LDR and STR of ZT0, the 41 of the
  multi-vector SVE operations, the 43 of the instructions on ZA array
  vectors, the 11 of their indexed forms, the 13 of BMOPA, the halfword
  outer products, MOVA of groups and LUTI2 and LUTI4, and the 5 of the
  SVE2.1 instructions it makes legal), each the class's fixed bits with
  the rest random, and of FDOT, BFMLSLB and BFMLSLT by vector and by
  element, whose classes' samples go to BFDOT and BFMLALB, each the
  encoding's fixed bits: 13,480 in all. The
  initial state is `qemu_max.txt`'s with Z0-Z31 and P0-P15 in place of the
  SIMD&FP registers (random bits, small elements, element extremes;
  random, all-true and per-size canonical predicates), ZA and ZT0 zero but
  for the classes on them, whose ZA vectors are zero, single-precision
  lanes, element extremes or random bits and ZT0 random, a load's or
  store's base register in its data region; for the floating-point
  classes, half the Z registers hold special and ordinary values of the
  word's own precision (single precision for FEAT_SME2's conversions of
  words) and the rest of half, single or double precision, under a
  random FPCR (RMode, FZ, DN, AHP, FZ16, FIZ, AH, NEP), and 1,227 of
  their 1,720 vectors raise an exception (every kind does), and the
  floating-point ZA instructions' sources are of their own precision,
  their 920 vectors leaving FPSR alone as their pseudocode does; for
  SVE2's integer classes, three in four Z registers hold elements of the
  word's own size, each a shift amount in range or an extreme, or
  extremes of half that size, a long operation's sources; for FEAT_SME2's
  narrows, three in four hold extremes of words or of doublewords, or
  words each a shift amount in range. The vectors
  were generated by `tests/vectors/gen_qemu_sve.py` with the same QEMU's
  `max` processor without FEAT_SME_FA64 (`qemu_sve_harness.c`,
  `qemu_sve_trampoline.S`), with the fixes for its two SVE
  floating-point bugs, its SME tile load bug and its MOVT applied (below,
  "Where the references differ"), and all agree. `tests/unit/za.rs` checks the
  ZA instructions against the pseudocode: the order of their enable
  checks and which execute outside Streaming SVE mode, the tile slices'
  vectors and columns, the slice index modulo the slices, a ZA vector
  zeroed above the vector length, MOVA's predication and the quadword's
  one predicate bit, ZERO's tiles, ADDHA's and ADDVA's sources, the
  integer products' signedness, byte predicates, wrap and subtraction,
  FMOPA's one rounding, default NaN and FPSR left alone, the widening
  products' two roundings and their inactive elements not negated, the
  tile loads' inactive elements zeroed and their faults part-way, LDR and
  STR of ZA and their alignment, and the 64-bit, FEAT_SME2p1 and
  FEAT_SME_LUTv2 encodings UNDEFINED; and of FEAT_SME2's on tiles,
  BMOPA's count of agreeing bits, the halfword products' pairs and their
  predicates, MOVA of a group's slices from an index aligned to the group,
  as rows and as columns, four doubleword slices UNDEFINED below 256 bits
  (in the decode for a processor whose longest vector length is), and the
  instructions without FEAT_SME2 UNDEFINED; `tests/unit/softfloat.rs` checks
  FPDot of half-precision pairs, NaNs, infinities, signed zeros and
  rounding. `tests/unit/zt0.rs` checks ZT0's: the order of its enable
  checks (SME's, ZA storage, `SMCR_EL1.EZT0`) in or out of Streaming SVE
  mode, ZERO, LDR and STR of its 64 bytes from a register or SP, their
  alignment and a fault part-way, MOVT UNDEFINED, ZT0 zeroed by enabling
  ZA, EZT0 RES0 and the instructions UNDEFINED without FEAT_SME2, and ZT0
  in a snapshot; and LUTI2's and LUTI4's indexes from the segment the
  immediate selects modulo the segments, for one, two and four
  registers, read before a register of the group is written, their enable
  checks with Streaming SVE mode first, and their UNDEFINED sizes.
  `tests/unit/pred_counter.rs` checks the
  predicate-as-counter: its encoding, a full count held as an inverted
  zero, the counter's element size read at another, a zero low nibble;
  WHILE counting up and down, signed and unsigned, with its flags, and of
  a pair; PEXT's parts and its pair wrapping from P15 to P0; CNTP's
  width; and the multi-vector loads and stores' registers, consecutive and
  strided, their layout in memory, the counter's elements across the
  registers, the inactive elements, the immediate offset in whole
  transfers, and a fault part-way. `tests/unit/multi.rs` checks the
  multi-vector SVE operations: SEL's elements by the counter across the
  registers, every source read before a destination is written,
  SQDMULH's saturation and exact 64-bit product, the rounding shifts'
  clamped shift and wrapped result, the maxima and minima signed and
  unsigned by one vector and by a group, the floating-point ones on NaNs
  and zeros, the clamps' order, ZIP and UZP of two and four vectors and
  of quadwords and the vector lengths they need in the decode and after
  the enable check, the narrowing shifts' rounding, saturation,
  concatenation and interleaving and the reserved `tsize`, the
  saturating narrows, BFCVT, and FCVT in IEEE half precision under
  FPCR.AHP, the word conversions' saturation and rounding, FRINT's own
  rounding with no Inexact, the unpacks and their reserved size; and of
  the SVE2.1 instructions FEAT_SME2 makes legal, the narrows of two
  vectors interleaving, rounding and saturating, and FCLAMP of one vector
  at each size, BFCLAMP's UNDEFINED.
  `tests/unit/za_array.rs` checks the instructions on ZA array vectors:
  the vector `(Wv + offset) MOD vstride` in each register's stride, the
  stride by the vector length and the register count, the long forms'
  vectors from a multiple of two or four and their elements, a register
  group wrapping from Z31 to Z0 and a second group stepping with the
  first; the integer products' signs, mixed signs, subtraction and wrap;
  the dot products of bytes and halfwords; FMLA's one rounding, default
  NaN (negative under FPCR.AH) and FPSR left alone; FDOT's rounding of
  the pair before the sum; FMLSL's and BFMLSL's negation; ADD and SUB
  replacing the vectors and the accumulating forms adding to them; MOVA
  between a group and the vectors, its offset unscaled; by an
  indexed element, the element or group of each result's own 128-bit
  segment, every class's index fields and a group's mixed-sign bit, and
  the vertical dot products' elements across the registers; the enable
  checks, and the 64-bit, FP8 and half-precision forms and the
  instructions without FEAT_SME2 UNDEFINED.
  `tests/unit/sve.rs` checks what random operands rarely
  reach, against the pseudocode: the SME trap outside Streaming SVE mode
  and the order of the enable checks, the vector length (and the bits
  above it zeroed), `DecodePredCount`, WHILE through the wrap of its
  register width, CTERM's flags, SDIV's zero and overflow quotients, EXT
  and DUP (indexed) beyond the vector, SQINCB's 32-bit saturation and
  PSEL's index; of the loads and stores, the inactive elements, a fault
  part-way, the register numbers wrapping, the alignment of LDR and STR,
  the SP check with no element active, and the accesses a prefetch and an
  all-inactive load do not make; and of floating point, FADDV's order of
  reduction and the identities of the inactive elements, the arithmetic
  immediates, FSCALE's clamped scale in each precision, FLOGB's special
  values and denormals, FCVTZS's sign extension and saturation,
  the multiply-add negations, FCMLA's rotations and indexed segments,
  BFCVT's zeroed upper half, the exceptions of the active elements alone,
  FPCR.AHP ignored, and `size = 00` UNDEFINED before the SME trap; and of
  SVE2's integer instructions, SQRDMLAH, SQDMULH and SQRDMULH of
  doublewords at their extremes, the shifts by the whole element (not its
  low byte) with their rounding and saturation, the reversed and pairwise
  operands, SUQADD's and USQADD's mixed signedness and the halving
  operations' range, SQDMLALB's saturation of the product before the
  accumulate, the dot products' wrap and USDOT's and SUDOT's signedness,
  CDOT's and CMLA's rotations, SQRDCMLAH's rounding, the source element of
  the long forms by element, SCLAMP's order, the elements the long and
  wide arithmetic, SSHLLT, EORBT and SABALB take, PMULL's polynomial
  products and SQDMULLB's saturation, the carry chain of ADCLB and SBCLB,
  CADD's rotations and SQCADD's saturation, URSRA, SRI and SLI at the
  element size, the narrowing instructions' halves, rounding and
  saturation, and the reserved sizes and `tsize` values UNDEFINED before
  the SME trap; no SVE saturation sets FPSR.QC. Of the SVE2.1
  instructions FEAT_SME2 makes legal: the two-way SDOT's and UDOT's pairs
  and wrap, by vector and by each segment's word, FDOT's two roundings and
  the Inexact it raises, BFMLSLB's and BFMLSLT's negation, and a NaN's
  sign that BFNeg keeps under FPCR.AH, and their traps and FEAT_SME2.
- The decoder is checked against Arm's machine-readable specification
  (`tests/unit/spec.rs`). `syn` reads the decoder's source and follows
  `decode` through every `match` to every encoding it returns. Against
  AARCHMRS release 2026-09: the functions are Arm's groups and classes,
  called from Arm's parents; every field is read under Arm's name and at
  Arm's bits; each class names exactly the encodings of that class whose
  condition holds for the features the decoder knows (the base set and
  every feature "What is modelled" lists); each encoding's arms fix
  exactly Arm's fixed bits, should-be bits included; each carries Arm's
  feature. And on all 2^32 words, the decoder
  names the encoding Arm's data allocates the word to, and nothing where it
  allocates none: 0 disagreements, about 15 s on four threads. The one
  departure is PRFM (register) with `Rt = 11xxx`, which Arm's data gives to
  RPRFM (FEAT_RPRFM) and which on a core without the feature is a PRFM with
  a reserved prfop. Against `Features.json`, each processor model's
  feature set holds what Arm makes each of its features imply, and what a
  conjunction of identifiers implies where all of them hold of a
  processor that can be built (`(FEAT_AA64 && FEAT_ECV_POFF) ==>
  FEAT_AA64EL2`, with and without EL2, on the implementation with every
  feature too), and every
  constraint whose identifiers decide it holds when the model's features,
  its architecture version and those it includes, and the structural
  features every processor here has (AArch64 at EL0 and EL1, a PIPT
  instruction cache, the Cryptographic Extension) hold and nothing else
  does. AARCHMRS is Arm's, under the BSD 3-clause licence, and
  is not in this repository: set `RUSTY_BOX_AARCHMRS` to the unpacked
  package when building the tests (`build.rs` then enables them; without it
  they are ignored, and `cargo xtask ci` refuses to run unless given
  `--skip-aarchmrs`), and `cargo test --release -p rusty_box_aarch64 --lib --
  --ignored the_decoder_allocates_as_arm_does_on_every_word` runs the
  exhaustive one. The source reading itself runs with every test run.
- System register and System instruction accesses are checked against the
  same package's `Registers.json` (`tests/unit/sysreg_spec.rs`): an
  interpreter of each accessor's pseudocode decides what each `MRS`, `MSR`
  and System instruction encoding an accessor names (17,967 of them, an
  index or a generic encoding's fields expanded) does, on the Cortex-A57
  and the Armv9.2 model, each with and without EL2 (the Armv9.2 model with
  EL2 a processor no machine builds, see "What is modelled"), and the
  test-only processors with every feature with and without it, at EL0,
  EL1 and EL2, in 103 to 1,090 states (per processor) of the registers that
  control access (everything clear; SIMD&FP,
  SVE and SME enabled; that with each bit `HCR_EL2`, `CPTR_EL2`,
  `MDCR_EL2`, `HSTR_EL2`, `CNTHCTL_EL2`, the fine-grained trap registers
  (`HFGRTR_EL2`, `HFGWTR_EL2`, `HFGITR_EL2`, `HDFGRTR_EL2`, `HDFGWTR_EL2`),
  `HCRX_EL2`, `SCTLR_EL2`, `CPACR_EL1`,
  `SCTLR_EL1`, `CNTKCTL_EL1` or `MDSCR_EL1` keeps set alone, with
  FEAT_VHE again under `HCR_EL2.E2H` and in the host; the OS lock;
  `SP_EL0` selected; random states): the access, an UNDEFINED exception
  or a trap to a level with a class. The processor must take the same
  exception with the syndrome `AArch64_SystemAccessTrapSyndrome` gives, or
  none, and an encoding no accessor names (87,054 of them) must be
  UNDEFINED but where the Arm ARM's prose on the ID register space says
  otherwise: 20.9 million steps, 0 disagreements, about a minute. Its table of
  IMPLEMENTATION DEFINED choices is this model's, each with its reason.
  Where an accessor hands an EL1 or EL2 access to the implementation
  (`AArch64_ImpDefSysRegRead` and the like) and the processor gives the
  encoding a register or an instruction of its own — the Cortex-A57's
  IMPLEMENTATION DEFINED registers and `RAMINDEX` — the TRM decides, and
  `tests/unit/impdef.rs` checks it; `HCR_EL2.TIDCP`'s trap of the same
  accesses, and every encoding the processor gives nothing, stay Arm's.
  The same file checks every EL2 register's
  fieldset: written at EL2 with all ones and all zeros, each field bit
  reads back as written, each RES0 bit as 0, each RES1 bit as 1 (175
  registers over the three processors with EL2, in each layout of
  `HCR_EL2.E2H`), the rules the data does not carry
  (CNTHP_CTL_EL2.ISTATUS and CNTHV_CTL_EL2.ISTATUS read-only, TCR_EL2.DS
  RES0 without FEAT_LPA2, SMPRIMAP_EL2 and HCRX_EL2.SMPME RES0 when
  `SMIDR_EL1.SMPS` is 0) named with their source.
- `tests/unit/el2_features.rs` runs guests on the processor with every
  feature and EL2 through FEAT_FGT, FEAT_HCX, FEAT_EVT, FEAT_EVT2,
  FEAT_XNX and FEAT_S2FWB: an `MRS` and an `MSR` each trapped by its own
  fine-grained field with EC 0x18 and its syndrome, EL0's outside the
  host and not in it, a negative field trapping where clear, the debug
  and PMU fields and `MDCR_EL2.TDCC`, a counter `HPMN` reserves trapping
  and one the PMU lacks UNDEFINED, `TLBI`, `DC ZVA`, `SVC` (returning to
  itself) and `ERET`, `ERETAA`, `ERETAB` (EC 0x1A) trapped, `FGTnXS`
  sparing a TLBI's nXS form; each EVT trap and what it leaves alone;
  `HCRX_EL2`'s kept fields; stage 2's `XN[1:0]` refusing EL1 and EL0
  their fetches apart; `HCR_EL2.FWB` as `AT S12E1R` reports it in
  `PAR_EL1` and as a misaligned load meets stage 2's Device memory, the
  TLB round-tripping a snapshot; the trap registers in a snapshot; and
  the ID registers and `CLIDR_EL1` with EL2, without, and on the
  Cortex-A57, unchanged. `src/cpu/mmu.rs` checks the TLB's memory of
  both stages against the attributes they make whole for every stage 1
  attribute and stage 2 `MemAttr`, under `FWB` and without.
- `tests/unit/impdef.rs` checks the Cortex-A57's IMPLEMENTATION DEFINED
  registers against the values worked out from its TRM's tables: each
  register's reset value at EL1 and EL2 and through a host's read; what
  a write of all ones and of all zeros keeps, `L2CTLR_EL1`'s read-only
  fields left, the syndrome registers zeroed and `CBAR_EL1`'s `MSR`
  UNDEFINED; EL0's accesses UNDEFINED; `HCR_EL2.TIDCP`'s trap of EL1's
  (EC 0x18, the ISS naming the encoding), `RAMINDEX`'s and an
  unimplemented encoding's included; `ACTLR_EL2` deciding EL1's writes of
  the five registers it controls; `ACTLR_EL2`'s kept bits and the RES0
  auxiliary registers; what `RAMINDEX` fills for every `RAMID`;
  `CPUACTLR_EL1`'s WFI and WFE NOPs; a snapshot of the registers, and one
  refused; and the Armv9.2 model, which has none of them.
  `tests/impdef.rs` runs a guest on the board that reads `CBAR_EL1` and
  `L2CTLR_EL1` on both layouts, with and without EL2, and finds them
  UNDEFINED on the Armv9.2 model.
- `tests/unit/el2.rs` checks FEAT_EL2 against Arm's pseudocode where the
  accessors do not reach: reset at EL2, `HVC` and `HCR_EL2.HCD`, `SMC`
  under `HCR_EL2.TSC` and as the PSCI call, `ERET` from EL2 to each level
  and its illegal returns, the PSTATE.SS it restores by where
  `MDCR_EL2.TDE` and `HCR_EL2.TGE` take debug exceptions, `RVBAR_EL2` and
  `RMR_EL2` in place of EL1's, `HCR_EL2.TGE` taking EL0's exceptions to EL2,
  `HCR_EL2.TEA` taking EL1's external aborts there (and `TGE` only EL0's,
  `SyncExternalAbortTarget`), physical IRQs and FIQs routed by `IMO` and
  `FMO` (and masked or not by PSTATE accordingly), the virtual IRQ, FIQ
  and SError `HCR_EL2.VI`, `VF` and `VSE` raise (taken at EL1, never at
  EL2 or under `TGE`, their priority, the SError's `VSESR_EL2` syndrome,
  `WFI` not waiting), `ISR_EL1` showing them, ESB deferring a masked
  virtual SError into `VDISR_EL2`, `VPIDR_EL2` and `VMPIDR_EL2` standing in
  for EL1's identity, SME under EL2 (`CheckSMEEnabled`'s order of
  `CPACR_EL1.SMEN`, `FPEN`, `CPTR_EL2.TSM` and `TFP` at EL0 under `TGE`,
  EL1 and EL2; `CheckSMEZT0Enabled`'s inactive-ZA trap, then
  `SMCR_EL1.EZT0` below EL2, then `SMCR_EL2.EZT0`; and RDSVL and CNTB
  reading the Streaming SVE vector length `SMCR_EL2.LEN` caps below EL2
  and sets at EL2, whatever `EZT0` holds), and the EL2 translation regime: its own tables and
  TLB entries apart from EL1&0's, `TLBI ALLE2`, one privilege level's
  permissions (AP[1] and bit 53 ignored, XN), one address range with its
  own TBI, `AT S1E2R` and `S1E2W`, and `AT S12E*` from EL2 through the
  EL1&0 regime; and FEAT_ECV_POFF on the processor with every feature:
  `CNTPOFF_EL2` and `CNTHCTL_EL2.ECV`, the offset count EL0 and EL1 read
  and EL2 does not (EL0's under `HCR_EL2.TGE` included), the EL1 physical
  timer's `TVAL` read and written at each level, `ISTATUS`, line and
  deadline on the offset count (its wrap included), the timers a machine
  holds after each EL2 write that moves them (`CNTPOFF_EL2`,
  `CNTHCTL_EL2`, `CNTVOFF_EL2`, `CNTHP_CVAL_EL2`) or does not, and that a
  write of any other EL2 register moves nothing they read, no offset
  without the feature even where `ECV` is held set, a counter poll
  fast-forwarded on the offset count (and none skipped across its wrap),
  the register by a host's name for it, and a snapshot that keeps the
  offset where the processor has it and is refused where it does not.
  `tests/unit/el2_vectors.rs` replays `tests/vectors/qemu_el2.txt`:
  2,400 cases, generated by `tests/vectors/gen_qemu_el2.py` with a
  bare-metal EL2 program (`qemu_el2_runner.S`, `qemu_el2.ld`) on QEMU
  11.1.2's `-M virt,virtualization=on -cpu cortex-a57`, each writing
  random EL2 and EL1 controls and running a System register access, a
  hypervisor, secure monitor or supervisor call, a trapped WFI or WFE,
  cache, TLB or address translation maintenance, a SIMD&FP access or an
  exception return at EL0, EL1 or EL2, and reporting the exception it
  took with its level, ESR, ELR, SPSR and FAR, or X0 after it. All of them
  agree, but for the firmware's `PSCI_VERSION`, which the replay expects
  as the board's PSCI 1.2 where QEMU recorded 1.1 (see "Where the
  references differ"), and for `IL1DATA0_EL1`, the Cortex-A57's
  IMPLEMENTATION DEFINED register QEMU's lacks, whose `MRS` and `MSR` at
  EL1 and EL2 the replay expects to complete where QEMU's were UNDEFINED
  ("The Cortex-A57's IMPLEMENTATION DEFINED registers are its TRM's");
  the generator leaves out what QEMU's Cortex-A57 has and this core
  does not (AArch32), the rest of the IMPLEMENTATION DEFINED registers,
  which QEMU's read as constants and whose writes it ignores, the PMU, whose
  cycles QEMU counts on its virtual clock where this core counts
  instructions ("The PMU's clock is the instruction clock"), and
  DBGPRCR_EL1, which QEMU lacks. `tests/virtualization.rs` runs the board
  with virtualization: firmware entered at EL2, the device tree's PSCI
  method, a host's tree given the board's `/psci` node and an empty
  `/chosen` where it has none (and `tests/unit/machine.rs` one whose `/psci`
  node is a PSCI 0.1 node and which has another memory node, both
  replaced), a hypervisor taking the EL2 timer's PPI 10 and powering off with
  `SMC`, the virtual timer's PPI 11 taken within two turns of the guest's
  two-instruction loop after the `CNTVOFF_EL2` write that moves the
  virtual count past `CVAL`, and a
  snapshot holding the EL2 state that a machine built without
  virtualization refuses before changing anything.
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
  same firmware (Debian's AAVMF) with 256 MiB and no network card
  (`-nic none`), QEMU with its own device tree and `fw_cfg`, this board with
  neither: identical console bytes, 1,333 of them to the shell's prompt.
  The shell's `memmap` differs: QEMU's holds 16 pages of ACPI tables, which
  EDK2 installs from QEMU's `fw_cfg` files, and more boot-services and
  runtime memory (BS_Data 4,565 pages against 4,305, BS_Code 950 against
  933, RT_Data 816 against 784), which this comparison does not attribute
  further.
- Alpine Linux 3.23.7 (kernel 6.18.58, Alpine's `virt` configuration) boots to
  a login prompt with no warnings in its log, logs in, and runs commands whose
  output matches the host (`seq 1 20000 | sha256sum`); `reboot` comes back to
  the login prompt and `poweroff` stops the machine.
- With virtualization (`--virtualization`) on the Cortex-A57, the same
  Alpine starts at EL2, and KVM initializes in nVHE mode with a 44-bit IPA
  limit. From a disk carrying QEMU 10.1.5 and the packages it needs, it
  runs `qemu-system-aarch64 -M virt,gic-version=2,kernel-irqchip=off
  -accel kvm -cpu host -m 192` with Alpine's own kernel and initramfs. The
  nested guest boots under KVM, through stage 2, to its initramfs shell,
  and there `uname -m` answers `aarch64` (2026-11-02). That run has QEMU
  emulate the guest's GIC in user space. Counted through the instrumentation hooks
  (`docs/aarch64-automation.md`), nine of the guest's exits in ten are
  stage 2 aborts on two device pages, the GIC CPU interface and the UART,
  each a round trip through KVM to QEMU; an exit costs about 24,000
  instructions, a timer tick about 2.2 exits, and no stage 2 fault repeats
  on memory. Time is counted in instructions, so that cost is fixed
  against the clock rate: a nested hypervisor needs a realistic one. At
  the default `ips` of 50M a second, the guest's 1 kHz tick takes some 97%
  of each emulated millisecond, and the guest reaches `Run /init` at 599 s
  of its own time, about 16 minutes of host time. With `--ips 1000000000`
  the tick takes under 5%: the guest reaches `Run /init` at 1.1 s and its
  shell at 6.55 s, 52 s of host time after its kernel starts, and the
  whole run, the host's boot and QEMU's install included, takes under 4
  minutes.
- With the GIC's Virtualization Extensions, the same Alpine's KVM finds
  the board's virtual interface (`kvm [1]: vgic interrupt IRQ9`, the
  maintenance interrupt the device tree names), and QEMU 10.1.5 runs with
  its default in-kernel irqchip (`-M virt,gic-version=2 -accel kvm`): KVM
  drives the List registers and the guest acknowledges through `GICV`.
  The nested guest boots to its shell, `uname -m` answering `aarch64` at
  6.49 s of its uptime; its timer, which KVM maps to the host's through a
  hardware List register, keeps interrupting across a 10 s `sleep` (264
  ticks, its uptime 6.49 s to 16.57 s). At `--ips 1000000000` the whole
  run takes 24.2 s of emulated time and 134 s of host time, the same
  each time to the state hash (2026-11-04).
- `tests/timer_interrupts.rs`: hand-written guests drive the interrupt path
  end to end. One runs GIC + timer + `WFI` + IRQ entry/`ERET` + PSCI; its
  outcome is identical for every budget slicing, and a snapshot taken
  mid-wait restores the same future. One sends itself an SGI and enters
  the FIQ vector for Group 0 under `GICC_CTLR.FIQEn`, and the IRQ vector
  for Group 1, or for Group 0 without it, as `ISR_EL1` shows beforehand.
  One sets the GIC and the PL011 up as Linux's console does — the UART's
  SPI 0 enabled, the receive interrupt at half full, `RTIM` unmasked — and
  waits in `WFI`; five characters typed at it, fewer than the level, wake
  it through the receive timeout: it takes the IRQ, acknowledges INTID
  32, finds `RTMIS` and reads the five characters, at the tick the 32 bit
  periods end, whatever the slicing. It needs every link on the way: the
  UART's deadline in the machine's next event, the deadline's expiry
  setting `RTRIS`, and the UART's line into the GIC.
- `src/devices/gicv2.rs` and `src/devices/pl011.rs` test the device models
  against IHI 0048B and DDI 0183F register by register. The GIC: both
  groups and the Group 1 aliases, with `AckCtl` set and clear; a group the
  distributor or the CPU interface leaves disabled hiding nothing;
  preemption by group priority under each group's binary point; SGIs on a
  uniprocessor; and snapshots whose SGI state disagrees, refused with
  nothing changed. Its Virtualization Extensions
  (`src/devices/gicv2_virt.rs`), through the `GICH` and `GICV` frames as a
  hypervisor and its guest use them: the reset values and `GICH_VTR`; List
  registers signalled by priority under `GICV_PMR`, each group's enable and
  binary point, and `GICH_HCR.En`; both acknowledge views and `AckCtl`;
  Group 0 on the virtual FIQ under `FIQEn`; a hardware interrupt's EOI
  deactivating it in the distributor; `EOImode` splitting the priority
  drop from `GICV_DIR`; `EOICount`; every maintenance condition, and
  INTID 25 following them at the distributor; `GICH_VMCR`; and snapshots
  refused by a GIC whose extensions differ. `tests/virtualization.rs` runs
  the whole path as guest code: a List register's interrupt taken at EL1,
  acknowledged and completed through `GICV`, its `EOI` maintenance
  interrupt taken at EL2; and without EL2, both frames unassigned. The
  PL011: the receiver and transmitter gated by
  `UARTEN`, `RXE` and `TXE`; both FIFO interrupts as transitions through
  the `UARTIFLS` levels, with the FIFO enabled and disabled, and a
  `TXIFLSEL` write alone changing nothing; a burst longer than the FIFO,
  written as Linux writes one, going out whole; the receive timeout's 32
  bit periods at the latched divisor, whether `RTIM` or the receiver is
  on, cleared by reading the holding register with the FIFO disabled; the
  `FEN` flush on clearing it, and the holding registers kept on setting
  it; an empty FIFO read as 0 with nothing changed; overruns, breaks (one
  lost to an overrun included, and the loopback receiver's input held low
  by one), loopback holding the host's bytes back, modem status
  interrupts, the inputs deasserted out of loopback, and reserved bits;
  and snapshots, with the states they must refuse. `src/devices/pl031.rs`
  tests the RTC: started, zeroed, loaded and reset, and its match only at
  equality, a match value already passed reached when the counter wraps.
- `tests/unit/softfloat.rs`: the softfloat surface against the host's IEEE
  arithmetic in round-to-nearest, and Arm's rules — default NaN, NaN
  propagation order, FPCR.DN and FZ, tininess before rounding, saturating
  conversions — including a fused multiply-add whose addend cancels the
  product's high word exactly.
- The virtio devices are driven register by register and ring by ring, as a
  driver drives them, by a test driver laid out like Linux's
  (`tests/unit/virtio_driver.rs`): the transport's registers, feature
  negotiation, resets, a queue's reset, `QueueReady` and `DRIVER_OK`,
  interrupt and configuration-change behaviour, chains ending in an
  indirect table, and every rule whose breach breaks the device
  (`tests/unit/virtio_transport.rs`); the keyboard's and tablet's
  configuration entries, the board's own: the names, vendor, products and
  versions its identity's, and exactly the keys, buttons and axes the host
  sends, in a configuration of virtio 1.2's layout, and the events each
  host action becomes
  (`tests/unit/virtio_input.rs`); the GPU's 16-byte configuration, every
  2D GPU command and its errors in QEMU's order, the host-memory budget
  of the guest's RAM, the cursor queue's answers, frames and the cursor as
  the host receives them (`tests/unit/virtio_gpu.rs`); the block device's
  configuration, its want of a geometry and of a request limit, every
  request type and its errors, the used
  lengths, the cache mode, read-only and file-backed disks and snapshots
  (`tests/unit/virtio_block.rs`). The
  EDID, read as a guest's parser reads it, at sizes from none to more than
  DisplayID holds: a valid E-EDID 1.4 structure in every block, the
  display's own size as its preferred mode with VESA CVT 1.2's
  reduced-blanking timing, the DMT modes that fit, sRGB, 96 pixels an
  inch, range limits holding every listed mode, a DisplayID 1.3 block
  only for a mode the base block cannot hold, and the board's names
  (`tests/unit/virtio_edid.rs`). `tests/identity.rs`
  reads, as a guest, the board's names: the device tree's, every
  transport's `VendorID`, the input devices' names and vendor, the flash
  devices' manufacturer and device codes. Alpine with the
  three devices binds `virtio_gpu` (`+edid`), shows its console in the
  framebuffer at the requested size, and receives a key typed on the virtio
  keyboard in its evdev; a snapshot taken after the drivers bound, restored
  in place or into a fresh machine, gives the same machine and the same
  picture as the uninterrupted run.
- `tests/api.rs`: the host API with guest-visible assertions — a page-crossing
  store's data abort reporting the page that faulted, the terminal keyboard's
  bytes reaching a polling guest once it has switched the UART's receiver on
  (and waiting until then), a guest's character leaving the UART only once
  it has enabled it, PSCI power-off and reset, the power button's 100 ms on
  its layout's GPIO line, an observer's view, direct kernel placement and
  the registers it is entered with, its device tree
  and the console it finds set up, the receive timeout a polling guest sees
  32 bit periods after a character, or after it sets `RTIM` with one
  waiting, at the same tick however the run is sliced, and at the same
  tick after a snapshot taken once it has latched a divisor without a baud
  clock, refused builds, a snapshot of an earlier version refused with
  nothing changed; and that a run cut into instruction budgets of any
  size, or stopped by an exit at any instruction of straight-line code,
  ends exactly where one instruction at a time would.
- The pre-decoded ops are checked against the class handlers they stand in
  for (`tests/unit/op.rs`): 400,000 words drawn from the templates of the
  classes `op.rs` specialises, and wholly random ones, each executed both
  ways from the same random state — registers pointing into RAM or not, the
  exception level, the alignment checks, the MMU off or on over a mapping of
  random permissions — must end in the same result, processor snapshot (TLB
  included) and memory, with the op held by the instruction-at-a-time path
  and with it read where a trace's op lies, in the trace pool. Every
  specialised form runs hundreds of times.
- Smaller unit tests pin single mechanisms: a RAM store writes exactly its
  width, little-endian, or nothing when it does not fit
  (`tests/unit/bus.rs`); every page of RAM has a write stamp of its own,
  and only a trace whose `B.cond` tests an ordering is marked as one that
  may begin a counter poll (`tests/unit/trace.rs`); a processor waiting for
  an interrupt sleeps to the first tick after now at which a timer or device
  changes an output by itself, and halts when there is none, so a device
  that reports a deadline it has reached and never clears changes neither:
  under an instruction budget, `emu_start` with no count and a tick budget,
  the run ends as it does without that device (`tests/unit/machine.rs`);
  and the AES S-boxes and `MixColumns`, carry-less multiplication and the
  CRC32 table match the definitions they are computed from
  (`tests/unit/crypto.rs`).
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
  and the pair is not, and fault where an element is misaligned; a pair of
  Q registers to Device memory loads at a doubleword boundary and faults
  at a word one.
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
  poll is reached by both; a direct poll and a timer-interrupted one at
  clock rates where the counter advances no whole number of counts a tick
  (10/3 and 1/3) end the same and are still fast-forwarded; an EL0 poll
  traps where `CNTKCTL_EL1` denies the counter and is fast-forwarded where
  it allows it; and polls outside the rule (an equality, a 32-bit compare,
  a store) run every instruction.
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
  `MDSCR_EL1` shows both flags, its save/restore fields, the flags among
  them, take a write only while the OS Lock is locked, and its RES0 bits
  read as zero; and EL0's accesses trap under `MDSCR_EL1.TDCC` before they
  take effect.
- `tests/translation_and_firmware.rs`: a T1SZ above 39 walking as 39 and
  one below 16 as 16 on the Cortex-A57, the latter a level 0 fault on the
  Armv9.2 model (FEAT_LVA), a table base or
  block beyond the output size `TCR_EL1.IPS` selects, `AT` meeting an
  external abort on its walk, PSCI's target, entry-point and
  affinity-level checks and the W registers of its 32-bit calls, the
  firmware's PSCI 1.2 and SMCCC 1.5 (`SMCCC_VERSION` through
  `PSCI_FEATURES`, `SMCCC_ARCH_FEATURES`, the SVE hint bit taken as 0) and
  each `SpectreReport`'s answers and workaround calls, and a
  store-exclusive to an unmonitored address failing without a fault.
- `tests/hardware_updates.rs`: FEAT_HAF and FEAT_HAFDBS at stage 1 as a
  guest reads its own descriptor back — `ID_AA64MMFR1_EL1.HAFDBS` 2 on the
  Armv9.2 model and 0 on the Cortex-A57; with `TCR_EL1.HA` a load and
  cache maintenance set a clear Access flag, which without it faults;
  with `HD` too a store makes a writable-clean page dirty, and after a
  load that leaves it clean and in the TLB; without `HD`, or with `HD`
  and not `HA`, `DBM` gives no write; an access its permissions refuse
  and an `AT` update nothing; and the Cortex-A57 ignoring both bits.
  `tests/unit/el2.rs` checks stage 2's and the EL2 regime's on a
  processor with every feature.
- `tests/device_regions.rs`: the GIC distributor and CPU interface answer
  in their 4 KiB and 8 KiB regions, and an access to the rest of their 64
  KiB windows is a synchronous external abort.
- `tests/vbsa.rs`: the V-BSA rules the board meets by a choice of its own,
  as a guest on either processor sees them — the PL011's `UARTPeriphID2`
  reading revision 3 and its transmit FIFO taking 32 characters before
  `TXFF`, and COMMIRQ pending on PPI 22 once `MDCCINT_EL1.TX` asks for it,
  acknowledged as INTID 22, and withdrawn once DTRTX is full.
- `tests/pmu.rs`: the PMU as a guest on either processor sees it —
  `PMCR_EL0`, `PMCEIDn_EL0` and `PMUVer` naming it, the cycle counter,
  `INST_RETIRED`, `CPU_CYCLES`, `EXC_TAKEN` and `EXC_RETURN` counting the
  instructions run around a `UDF` and its handler, the filters counting
  only the levels they leave, an overflow pending on PPI 23 and
  acknowledged as INTID 23, the cycle counter stopping through a `WFI`
  while the generic timer runs on, and EL0's access as `PMUSERENR_EL0`
  allows. `tests/unit/pmu.rs` checks `PMUSERENR_EL0`'s and `MDCR_EL2`'s
  traps, `HPMN`'s ranges and each processor's choice for the counters
  out of reach, `SW_INCR`, `CHAIN` with 32-bit and 64-bit counters,
  freezing on overflow, `HPMD` and `HCCD`, the register-write events and
  a snapshot; `tests/unit/sysreg_spec.rs` holds every PMU accessor to
  Arm's on each processor.

## Where the references differ

Arm's documents decide wherever they speak: the Arm ARM and AARCHMRS what
an instruction does, IHI 0048B the GIC, DDI 0183F and DDI 0183G the
PL011, DEN 0022E and DEN 0028F the firmware calls, BSA and V-BSA the
platform; OASIS virtio 1.2 the virtio devices and VESA
E-EDID 1.4 the display's EDID. QEMU 11.1.2's `virt` board and `cortex-a57`
decide what they do not: the board's layout, wiring and device tree, the
devices no specification describes, and the choices the specifications
leave to an implementation. Every place this model departs from QEMU is listed here
with its evidence, as are the choices it makes where the references leave
one open.

- **Should-be fields are UNDEFINED when set otherwise.** Arm's encodings of
  34 ARMv8.0 instructions have should-be bits (`(0)` and `(1)` in the Arm
  ARM's encoding diagrams; `should_be_mask` in AARCHMRS): `Rs` and `Rt2` of
  the exclusive and ordered loads and stores (LDXR, STXR, LDXP, LDAXP, LDAR,
  STLR and the rest of `ldstexclr`, `ldstexclp` and `ldstord`), `Ra` of SMULH
  and UMULH, and `Rm` of FCMP and FCMPE with zero. FEAT_LRCPC's LDAPR,
  LDAPRB and LDAPRH have one too, `Rs`, which QEMU also refuses unless it
  is 11111, and so do FEAT_FlagM's CFINV and FEAT_FlagM2's XAFLAG and
  AXFLAG, `CRm`, which QEMU fixes at 0000 (`Rt2` of CAS and CASP is not
  one: Arm fixes it, so its other values are unallocated). A word that sets one of
  them otherwise is CONSTRAINED UNPREDICTABLE, and UNDEFINED is among the
  behaviours the Arm ARM permits; it is this model's, so the decoder is
  exactly Arm's encoding table, which fixes those bits. QEMU 11.1.2
  (`target/arm/tcg/a64.decode`) fixes them for LDAR, STLR, SMULH and UMULH
  only and executes the exclusives (`STXR`, `LDXR`, `STXP`, `LDXP`) and the
  compares with zero (`FCMP`, whose `rm` it ignores when `z` is set) as if the
  field held its should-be value. The Unicorn replay, whose QEMU fixes none
  of them, skips its vectors that set such a field.
- **LDSMAX and LDSMIN compare signed at every size.** Arm's `MemAtomicInt`
  computes `Max(SInt(op1), SInt(op2))` (and `Min`) on the access's width.
  Unicorn 2.1.4's `max` processor compares the byte, halfword and word forms
  unsigned: 25 of 200 generated LDADD-group vectors disagreed with this
  model, all of them those forms. QEMU agrees with Arm, so these vectors
  come from it (`gen_qemu.py`).
- **A pointer whose bits 55 and 63 differ is signed for InsertPAC's
  half.** Arm's `InsertPAC` takes the half of the address space, and with
  it the extension and the bottom bit of the code, from bit 55 where a
  top-byte-ignore applies to the access in either half, and from bit 63
  otherwise. QEMU's `pauth_addpac` (to 11.1.2 at least) takes the bottom
  bit from bit 55's half always, and the extension from bit 55 only where
  that half's TBI applies: `pacda` of `0x7f80000000001000` with T0SZ = 25,
  T1SZ = 20 and no TBI codes bits 54:44 there, 54:39 here.
  `gen_qemu_pauth.py` signs no such pointer (`qemu_selects_differently`),
  and `tests/unit/pointer_auth.rs` checks Arm's choice. With the fix in
  [docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) applied, QEMU signs every pointer as this
  model does: all 2,400 vectors generated without that filter agree.
- **LDRAA and LDRAB check SP's alignment.** Arm's pages call
  `CheckSPAlignment` with SP as the base, after the authentication. QEMU
  makes no SP alignment check (`gen_check_sp_alignment` is empty, a choice
  its comment states): an `ldraa x0, [sp]` with SP 8 bytes off and
  SCTLR_EL1.SA set loads. `gen_qemu_pauth.py` keeps a base SP 16-byte
  aligned, and `tests/unit/pointer_auth.rs` checks the fault and its order.
- **The pointer authentication keys reset to the board's own values.**
  Each key register, `APIAKeyLo_EL1` to `APGAKeyHi_EL1`, resets to an
  architecturally UNKNOWN value (DDI 0487M.d, D24.2.27 and its siblings);
  QEMU's are zero. The board is
  deterministic, so it fixes a value too, its own: the ten registers in
  the order of their encodings take the first 640 bits of the fractional
  part of √2, 64 bits each (`PacKeys::RESET`), from 0x6A09_E667_F3BC_C908
  in `APIAKeyLo_EL1` to 0xA60B_38C0_2378_66F7 in `APGAKeyHi_EL1`. None is
  zero and no two are equal, so a guest that signs or authenticates
  before it programs a key, or reads one to see whether it was
  programmed, finds a value it did not write rather than one that looks
  like an empty register; and none is a digit of π, as QARMA5's own
  constants are (`ComputePACQARMA`). Linux programs a key before it sets
  the `SCTLR_EL1` enable that uses it, and EDK2 sets none, so neither
  reads them. Only the Armv9.2 model has the keys, the Cortex-A57 having
  no FEAT_PAuth. `tests/unit/pointer_auth.rs`
  (`the_keys_reset_to_the_boards_fixed_values`) checks what an `MRS` of
  each reads, built and after a system reset, against √2 worked out in the
  test.
- **Registers Arm leaves UNKNOWN at reset take the Cortex-A57 TRM's value
  or the board's own.** Arm's `AArch64_TakeReset` (DDI 0487M.d, J1.2.2.30)
  leaves the general-purpose and SIMD&FP registers, the stack pointers,
  `ELR_ELx`, `SPSR_ELx`, NZCV and every register field it does not reset
  ARBITRARY; QEMU zeroes them all. Where the Cortex-A57 TRM (DDI 0488H,
  4.2, Tables 9-2 and 14-2) gives a value, both models take it:
  `SCTLR_EL2` 0x30C5_0838 (its RES1 bits and `SA`), `CPTR_EL2` its RES1
  bits, `HCR_EL2`, `ESR_EL2`,
  `HPFAR_EL2`, `CPACR_EL1`, `FPCR` and `FPSR` zero, and bits 63:32 of
  `FAR_ELx`, `VBAR_ELx`, `TTBRn_ELx`, `VTTBR_EL2` and `PAR_EL1` zero.
  The rest is the board's own, by what each register holds
  (`src/cpu/reset.rs`): a control — `TCR_ELx`, `VTCR_EL2`, `MAIR_ELx`,
  `MDSCR_EL1`, the breakpoint and watchpoint controls, `CNTVOFF_EL2`,
  `SMCR_ELx`, the timers' `IMASK` — zero, its off state, so nothing acts
  that software did not ask for and software that read-modify-writes a
  control (Linux its `CNTKCTL_EL1`, EDK2 its `CPACR_EL1` and `CPTR_EL2`)
  finds the fields it leaves alone off; `CSSELR_EL1` the L1 data cache,
  so a `CCSIDR_EL1` read before any selection describes a cache that
  exists; `DISR_EL1.A` and the Event Register clear; and a value — the
  general-purpose and SIMD&FP registers, the stack pointers, `ELR_ELx`,
  `SPSR_ELx`, `ESR_EL1`, `FAR_ELx`, `PAR_EL1`, `VBAR_ELx`, the table
  base registers, `CONTEXTIDR_ELx`, the thread pointers, the timers'
  compare values, `DBGBVRn_EL1` and `DBGWVRn_EL1`, the DCC's data
  registers — the board's fill, 0xA5A5_A5A5_A5A5_0000 with the
  register's name in bits 15:0 (`n` for `Xn`, 0x100 + `n` in each half
  of `Vn`, the `MRS` encoding for the rest), kept to the bits the
  register holds; NZCV 0b1010. A guest that reads a register it never
  wrote finds a value it did not put there, which names its source, and
  one that dereferences or branches through such a register faults there:
  bits 55:52 are 0b1010, outside both halves of the address space for
  any VA size and top-byte-ignore, and above every physical address. The
  bits of Z above V, P, FFR, ZA and ZT0 stay zero: only Streaming SVE
  mode or ZA storage reaches them, and entering either zeroes them. A
  directly booted kernel's `x1` to `x3` are zero as `booting.rst` §4
  asks. `tests/reset_state.rs` reads each register with `MRS` on both
  models and the Cortex-A57 with EL2, and a branch through an unwritten
  register taking its Address size fault; the vector replays, whose
  references start from QEMU's zeroes, zero the fill first
  (`Cpu::zero_the_fill`).
- **Overlapping load and store registers are UNDEFINED.** Arm's decode
  pseudocode makes a load or store CONSTRAINED UNPREDICTABLE where its
  registers overlap: writeback into a transfer register
  (`Unpredictable_WBOVERLAPLD`, `WBOVERLAPST`: LDR, STR and their byte,
  halfword and signed forms, LDP, STP, LDPSW, LDRAA and LDRAB), a load
  pair or load-exclusive pair into one register (`LDPOVERLAP`: LDP,
  LDNP, LDPSW, the SIMD&FP LDP and LDNP, LDXP, LDAXP), and a
  store-exclusive's status register among its transfer registers or its
  base (`DATAOVERLAP`, `BASEOVERLAP`). UNDEFINED is among the permitted
  behaviours of each, and the Cortex-A57 TRM states no choice of the
  A57's (its Appendix B covers AArch32), so both models take UNDEFINED,
  at decode, before any SIMD&FP trap, SP alignment check or
  authentication: the one choice that stops the guest at the instruction
  rather than leaving a register silently holding what the program did
  not mean. QEMU 11.1.2 (`target/arm/tcg/translate-a64.c`) executes each:
  a load writes the loaded value and then the written-back base, a store
  stores the original value, `ldp w1, w1` writes Rt2 then Rt and
  `ldp x1, x1` Rt then Rt2, and a store-exclusive stores the value and
  writes its status. `tests/unit/load_store.rs` checks every case on both
  models; the Unicorn, QEMU `max` and pointer authentication replays
  skip their vectors of these words (`registers_overlap`,
  `writes_back_into_its_base`).
- **An atomic reports only a refused write as a write.** Arm's
  `CreateAccDescAtomicOp` makes an atomic access a read and a write;
  `NoFault`, whose record an alignment or translation fault starts from,
  sets `write` to `!read && write`, and `AArch64_S1CheckPermissions`
  checks read permission before write permission. So WnR is set only for
  a permission fault on the write: an LDADD to an unmapped page, a
  misaligned one, or one that PSTATE.PAN refuses reports a read. QEMU (to
  11.1.2 at least) looks an atomic up as a store (`atomic_mmu_lookup` in
  accel/tcg/cputlb.c, `MMU_DATA_STORE` for its alignment check and TLB
  fill), so it reports every one of those as a write. User-mode vectors
  cannot fault; `tests/unit/atomic.rs` and `tests/unit/pstate_features.rs`
  check Arm's rule.
- **An illegal exception return restores PSTATE.PAN.** Arm's
  `SetPSTATEFromPSR` sets PAN from SPSR whether or not the return is legal,
  and makes SSBS, UAO, DIT and BTYPE UNKNOWN on an illegal one; this model
  keeps SSBS, UAO and DIT as they were and clears BTYPE. QEMU's
  `HELPER(exception_return)` (to 11.1.2 at least) restores only NZCV, DAIF
  and ALLINT on an illegal return (helper-a64.c, `illegal_return`), keeping
  PAN. `tests/unit/pstate_features.rs` checks Arm's rule.
- **DC ZVA faults on Device memory.** Arm's
  `AArch64_S1HasAlignmentFaultDueToMemType` returns TRUE for
  `AccessType_DCZero` on Device memory, ahead of the permission check;
  QEMU's `HELPER(dc_zva)` zeroes it, a choice its comment states.
  `tests/unit/atomic.rs` checks the fault, on a page that refuses the write
  too.
- **The Cortex-A57's L1 data cache is 2-way.** The Cortex-A57 TRM (ARM DDI
  0488C, 6.1) gives a 32 KiB 2-way set-associative data cache with 64-byte
  lines, which `CCSIDR_EL1` reports here (0x701F_E00A, 256 sets). QEMU 9.2
  and later report 4 ways and 128 sets, in `CCSIDR_EL1` and in the `virt`
  device tree's `d-cache-sets`, and name a `next-level-cache` of phandle 0
  when they make no L2 node: [docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)
  has both patches. The board's own tree describes the caches from
  `CCSIDR_EL1`, the L2 among them, and `tests/unit/board_tree.rs` checks
  them against it.
- **Each model's L2, and the Armv9.2 model's caches, are the board's
  own.** The Cortex-A57's L2 is a configuration option of the design,
  512 KiB, 1 MiB or 2 MiB, 16-way with 64-byte lines whichever it is
  (DDI 0488H, 1.5 Table 1-1, 7.1, 7.2); QEMU's `cortex-a57` takes 2 MiB.
  The board takes 512 KiB, `CCSIDR_EL1` 0x703F_E07A for `CSSELR_EL1` 2
  (Table 4-37): it models no cache, so a size buys a guest nothing, and
  cleaning a whole cache by set/way, which walks every set and way of
  each level to the PoC (the Arm ARM's example, DDI 0487M.d,
  D7.5.9.16.1), costs an instruction a line, 8,192 at L2 where 2 MiB
  takes 32,768.
  The Armv9.2 model is no Arm part, and no Arm document gives a generic
  Armv9.2 processor caches: the Arm ARM leaves each cache's geometry
  IMPLEMENTATION DEFINED and says `CCSIDR_EL1` describes only what
  maintenance by set/way needs (DDI 0487M.d, D24.2.31). QEMU's `max`
  (`aarch64_max_tcg_initfn`) reports 64 KiB 4-way L1 caches, a 1 MiB
  8-way L2 and a 2 MiB 8-way L3, `CLIDR_EL1.LoUU` and `LoUIS` 0, and
  `CTR_EL0.IDC` and `DIC` set. The
  model reports 32 KiB 4-way L1 data and instruction caches and a
  256 KiB 8-way unified L2 (`CCSIDR_EL1` 0x000F_E01A, 0x000F_E01A,
  0x003F_E03A), in the format without FEAT_CCIDX, which it does not
  implement, with bits [31:28], UNKNOWN in that format, 0; every line 64
  bytes, the size the Arm ARM gives generic software as
  `hardware_constructive_interference_size` (D24.2.31), so the
  Cortex-A57's `CTR_EL0` (64-byte lines and writeback granule, IDC and
  DIC 0) and `CLIDR_EL1` (LoUIS 1, LoC 2, LoUU 1) hold for it as they
  stand; a processor with FEAT_S2FWB, which `Features.json` requires to
  name no cache to the Point of Unification (`FEAT_S2FWB ==>
  (UInt(CLIDR_EL1.LoUU) == 0)`, and `LoUIS` alike), reads `LoUU` and
  `LoUIS` 0 in the same hierarchy (`id::clidr_el1`), `CTR_EL0.IDC` 0
  saying no more than that then. The device tree's cache nodes give each model's sizes, and
  `tests/unit/board_tree.rs`, `tests/unit/sysreg.rs`
  (`ccsidr_reports_the_cache_csselr_selects`) and `tests/api.rs` check
  what a guest reads.
- **A `CSSELR_EL1` selection of no cache selects a Level 1 cache.** A
  selection `CLIDR_EL1` does not name — Level 3 or beyond, the
  instruction cache at the unified L2, a reserved Level — leaves both
  reads open: `CSSELR_EL1` returns UNKNOWN Level and InD (DDI 0487M.d,
  D24.2.40, K1.2.12), and a `CCSIDR_EL1` read is CONSTRAINED
  UNPREDICTABLE, a NOP, UNDEFINED or an UNKNOWN value (D24.2.31). The
  Cortex-A57 TRM makes only Level UNKNOWN (DDI 0488H, 4.3.25) and has
  `CCSIDR_EL1` return an UNKNOWN value (4.3.22). QEMU keeps the selection
  as written and reads `CCSIDR_EL1` as 0, a cache of one set and one way
  with 16-byte lines, which no `CLIDR_EL1` here describes. On both models
  the board maps the selection onto the Level 1 cache of the kind its
  InD names (`Caches::selection`), the first of the ways the Arm ARM lets
  an unallocated field value behave (K1.2.19: it "maps onto any of the
  allocated values"), keeping InD as the A57's TRM requires: `CSSELR_EL1`
  reads that selection back and `CCSIDR_EL1` describes that cache, so
  every read names a cache the guest can find in `CLIDR_EL1` and the two
  registers agree. `tests/unit/sysreg.rs`
  (`ccsidr_reports_the_cache_csselr_selects`) reads every selection back
  on both models.
- **The Armv9.2 model is a software model in its device tree too.** Its
  `MIDR_EL1` names implementer 0, reserved for software use, as QEMU's
  `max` does (`aarch64_max_tcg_initfn`), with part 0x052 where QEMU's is
  `'Q'`; and its processor node's `compatible` is `arm,armv8`, which Linux's
  binding (`Documentation/devicetree/bindings/arm/cpus.yaml`) gives software
  models. QEMU's `max` keeps the `arm,cortex-a57` of the Cortex-A57 it is
  built from in its device tree ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)
  entry 17). The model is not QEMU's `max`: that one is every feature QEMU
  has, SVE and Armv9.6's among them, and changes with each release.
- **A device tree nested deeper than a guest reads is refused.** The
  editor that replaces the memory and `/psci` nodes and sets `/chosen`
  walks the tree with a
  stack frame per level, so it refuses a tree with a node more than 62
  levels below the root (`DeviceTreeError::TooDeep`), where QEMU would hand
  it to the guest. Linux's `unflatten_dt_nodes` skips every such node
  (`FDT_MAX_DEPTH`), so no guest reads one; QEMU's own tree goes five
  levels down (`cpus/cpu-map/socket0/cluster0/core0`).
- **A PL011 in loopback overruns as the PL011 does.** With `UARTCR.LBE`
  set, a character written while the receive FIFO is full is lost, the
  FIFO keeps what it holds, and the overrun is flagged (`UARTRSR.OE`,
  `UARTDR.OE`, the overrun interrupt), as the PL011 TRM (ARM DDI 0183)
  describes. QEMU 11.1.2's `pl011_loopback_tx` puts it in anyway: it
  overwrites the oldest character and the FIFO's count runs past its depth,
  so 17 characters written come back as `qbcdefghijklmnopq`
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 6
  and its patch). `src/devices/pl011.rs` checks the overrun.
- **Without FEAT_LSE2, a misaligned load-acquire or store-release faults.**
  The current `AArch64_UnalignedAccessFaults` faults an ordered access only
  when it leaves its 16-byte granule with `SCTLR_ELx.nAA` = 0, testing no
  feature. `nAA` is a FEAT_LSE2 field, and Arm ARM B2.8.2.1.3 applies the
  rule to a processor with FEAT_LSE2 alone, so this model applies it with
  FEAT_LSE2 and the Armv8.0 one without it, as the Cortex-A57 and QEMU
  (`check_ordered_align`, alignment enforced unless the processor has
  LSE2) do: any misaligned LDAR, STLR, LDAXR or STLXR faults.
- **With FEAT_LSE2, a misaligned atomic or ordered access off Write-Back
  Shareable memory is performed.** `Unpredictable_LSE2_ALIGNMENT_FAULT` lets
  such an access fault or not on Normal memory that is not Write-Back
  Shareable; this model takes Arm's listed default, no fault, and no atomic
  access takes the fault `Unpredictable_Atomic_MMU_IMPDEF_FAULT` allows.
  Device memory refuses it, as every misaligned access.
- **Reserved memory attributes, Contiguous and fetches from Device memory.**
  A `MAIR_EL1` attribute Arm reserves, which `Unpredictable_RESMAIR` maps to
  an UNKNOWN allocated value, is read by its fields, as QEMU reads it: a top
  nibble of zero is Device memory, and anything else Normal. Shareability
  0b01 is Outer Shareable (`DecodeShareability`, Constraint_OSH). The
  Contiguous bit changes nothing, as `AArch64_ContiguousBit` allows ("Treat
  Contiguous bit as 0"), so an entry here always maps one leaf. A fetch
  from Device memory that is not execute-never runs
  (`Unpredictable_INSTRDEVICE`, Constraint_NONE); Arm ARM B2.10.2 allows a
  Permission fault instead. Every misaligned access to Device memory
  faults on alignment: `AArch64_S1HasAlignmentFaultDueToMemType` lets a
  location accept one, and no location here does. A byte of a misaligned
  access that reaches Device memory on the next page faults the same way
  (`Unpredictable_DEVPAGE2`, Constraint_FAULT).
- **`AT` reports the memory the access would use, not the descriptor's
  attributes.** Arm ARM D7.5.5 and rule RLRYRY (D8.2.13.1, the resulting
  attributes returned in `PAR_EL1`) make `PAR_EL1` report what
  applies with `SCTLR_EL1.C` 0, so Normal memory reports Non-cacheable
  (0x44, or 0x40 for Write-Back memory under FEAT_XS), Outer Shareable,
  and a reserved shareability value reports as what it decodes to. QEMU
  11.1.2 reads `MAIR_EL1` with no regard to `SCTLR_EL1.C`, and reports
  shareability 0b01 as it stands (`do_ats_write`, `par_el1_shareability`);
  the page of `PAR_EL1` allows the descriptor's own value, so only the
  first is a departure from Arm's rule. `tests/unit/memory_system.rs` checks
  Arm's.
- **Cache maintenance by address takes the Access flag fault.** Whether it
  can is IMPLEMENTATION DEFINED (Arm ARM D8.15.3, RTRTWX). These
  instructions are not NOPs here and share the walk of every other access,
  so `DC` and `IC` by address on a page with AF 0 fault, with CM and WnR
  set; `tests/unit/memory_system.rs` checks it. Where FEAT_HAF's `HA`
  manages the flag they set it instead, as an access does
  (`AArch64_SetAccessFlag` asks the same IMPLEMENTATION DEFINED question);
  `tests/hardware_updates.rs` checks it.
- **A descriptor's hardware update is made only for an access its checks
  permit.** Whether an access that takes an Alignment or Permission fault
  sets the Access flag or the dirty state is CONSTRAINED UNPREDICTABLE
  (Arm ARM D8.5.1.2 RQSPMS, D8.5.2.1 RDYCFD; `Unpredictable_AFUPDATE`,
  `Unpredictable_DBUPDATE`): this core updates nothing, and so caches no
  leaf whose flag is still clear. An address translation instruction
  updates neither the flag nor the dirty state of the leaf it reports
  (RXQSMX; "AT updates AF" FALSE) and reports a leaf whose managed flag
  is clear as translating. A descriptor in memory that is not Write-Back
  Shareable is updated like any other
  (`Unpredictable_Unsupported_Atomic_HW_Update` FALSE), the bus swapping
  every descriptor atomically. QEMU's `get_phys_addr_lpae` makes the same
  choices (no update under `in_at`, and the update only after the
  permission check). `tests/hardware_updates.rs` checks the first two.
- **Self-hosted debug takes Arm's listed default in every open case.** An
  SME access, a contiguous SVE access and a SIMD&FP access in Streaming SVE
  mode (`IsRelaxedWatchpointAccess`) are watched out to the 16-byte
  boundaries around them, with WPF set (`Unpredictable_16BYTEROUNDEDDOWNACCESS`,
  `...UPACCESS`); Arm ARM D2.9.6.3 (RLVLYV) would allow the exact bytes too. A
  reserved
  `DBGBCR<n>.BT`, HMC, SSC or PMC, or `DBGWCR<n>` equivalent, behaves as
  disabled (`Unpredictable_RESBPTYPE`, `CheckValidStateMatch`), as does a
  link to anything but a context-aware breakpoint with linking enabled
  (`BPNOTLINKING`, `BPLINKEDADDRMATCH`); `DBGWCR<n>.MASK` 1 or 2 is disabled
  (`RESWPMASK`), and a mask with BAS other than all ones, a non-contiguous
  BAS, or set bits inside a mask match nothing (`WPMASKANDBAS`,
  `WPBASCONTIGUOUS`, `WPMASKEDBITS`). Bits of `DBGBVR<n>` and `DBGWVR<n>`
  above `DebugAddrTop` — bit 52 with FEAT_LVA, the Armv9.2 processor's,
  bit 48 without — are not compared (`DBGxVR_RESS`). With EL2, `HMC` and
  `SSC` 0b11 reach EL2 as `AArch64_StateMatch` has them on a processor
  with no EL3 and no Secure state, the VMID breakpoint types compare
  `VTTBR_EL2.VMID`, and the `CONTEXTIDR_EL2` types (`match_cid2`), which
  need FEAT_VHE or FEAT_Debugv8p2 with EL2 as `AArch64_ReservedBreakpointType`
  has it, compare `CONTEXTIDR_EL2`; the Cortex-A57 with EL2 reserves them.
  The IMPLEMENTATION DEFINED
  choices: a Software Step exception after a stepped instruction records
  ISV, EX for a Load-Exclusive, and one after an exception return records
  none; a Watchpoint exception reports the first watched byte in FAR_EL1,
  the last matching watchpoint with FEAT_Debugv8p2 (`WPTV` 1), and `WPF` 0
  for an access that is not rounded;
  a Store-Exclusive is checked at its translation, so one that fails on
  the monitor's address before translating raises no watchpoint, and one
  that fails after it can; a store of one register or a pair across pages
  is checked whole before anything is written, in the order of its bytes
  (the first page's translation, then its watchpoints, then the second
  page's), and so is a pair in one page, where Arm's `Mem` setter writes
  an aligned pair as two single-copy atomic accesses and so may write the
  first before the second's watchpoint fires: Arm ARM D1.4.1.4 makes the
  memory an instruction of more than one such access affects UNKNOWN after
  a precise exception, so the first element's old value is one the
  architecture allows. `ST1` and the SVE and
  SME stores are checked an element at a time. `tests/unit/debug.rs` checks
  them, with `ST1` standing for the stores made an element at a time.
- **Cache maintenance by address translates, and faults as cache
  maintenance.** Arm's `AArch64_DC` and `AArch64_IC` translate the address
  of DC IVAC, CVAC, CVAU, CVAP, CVADP, CIVAC and IC IVAU and abort on a
  fault, with `ISS.CM` and WnR set and FAR the register's value
  (`AArch64_FaultSyndrome`); `AArch64_S1CheckPermissions` asks DC IVAC for
  write permission, a clean at EL0 for read permission and nothing more, and
  PSTATE.PAN takes no part (`CreateAccDescDC`). Whether IC IVAU at EL0
  needs read permission is IMPLEMENTATION DEFINED ("Permission fault on EL0
  IC_IVAU execution"); here it needs none. QEMU (to 11.1.2 at least)
  executes all but DC CVAP and DC CVADP as NOPs, a choice its comment
  states ("Cache ops: all NOPs since we don't emulate caches"), and probes
  DC CVAP and DC CVADP as a load (`dccvap_writefn`): no CM, no WnR, FAR
  rounded down to the line, and an EL1 permission fault under PAN.
  [docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) has the
  probe's cases and a patch for DC CVAP and DC CVADP;
  `tests/unit/memory_system.rs` checks Arm's rules.
- **No LORegions.** With FEAT_LOR, `LORID_EL1` reports no LORegions and no
  LORegion descriptors, and `LORSA_EL1`, `LOREA_EL1`, `LORN_EL1` and
  `LORC_EL1` are RES0. How many of each a processor has is its own choice
  (Arm ARM D24.2.120), and this is the board's: a LORegion narrows the
  accesses LDLAR and STLLR order to those in its region (B2.4.3), and the
  board's one processor completes every access before the next, in program
  order for every observer, the devices' DMA included, so no region could
  change what a guest observes. LDLAR and STLLR are then the acquire and
  release accesses their pseudocode makes them (`CreateAccDescLOR`), as
  the register's note says of a processor with none. QEMU's `lor_reginfo`
  reports none too.
- **The Armv9.2 model says its Streaming SVE mode is its own.** Its
  `SMIDR_EL1` reads 0x4000: `SH` 0b10, "not shared with other PEs", said in
  its own field, the processor executing its streaming instructions
  itself, with `Affinity`, `Affinity2` and `NSMC` 0 as an unshared
  implementation has them; `Implementer` 0 and `Revision` 0, its
  `MIDR_EL1`'s; and `SMPS` 0, no streaming execution priority, so
  `SMPRI_EL1` and `SMPRIMAP_EL2` are RES0. Each field is IMPLEMENTATION
  DEFINED (Arm ARM D24.2.186). QEMU's `sme_reginfo` reads 0, `SH` 0b00
  leaving `Affinity` 0 to say the same.
- **An exception return and the event stream set the Event Register.**
  Arm's `AArch64_ExceptionReturn` calls `SendEventLocal` on every return,
  legal or not, and `TestEventCNTV` sets the register at each selected
  transition of the count whatever the processor is doing; this model
  does both (the stream's events reach the register when it is next read,
  from the count it was last brought up to). QEMU 11.1.2 sets it on ERET
  not at all, and from the stream only while a WFE waits, so a WFE right
  after an ERET, or after a stream event while running, waits instead of
  completing, and at EL0 with SCTLR_EL1.nTWE clear traps.
  [docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) has probe
  cases J and K and a patch for each.
- **WFET traps before it looks at interrupts.** Arm's `Hint_WFET` completes
  ahead of the trap check only for a set Event Register or a passed
  timeout; `Hint_WFI` and `Hint_WFIT` complete on a pending interrupt
  first. QEMU 11.1.2's `HELPER(wfet)` completes on a pending interrupt
  ahead of the trap check too (probe case L and its patch).
- **A WFE wait ends on any pending interrupt, and its end clears the Event
  Register.** WFE's wake-up events include interrupts; a masked one ends
  the wait here too, which a PE may do (it may leave its low-power state
  for any reason), as QEMU's `arm_cpu_has_work` does. Whether a PE's
  restart after the wait clears the Event Register is IMPLEMENTATION
  DEFINED (`WaitForEvent`); here it does, so the event that ended a wait
  is consumed with it. QEMU's `arm_wfxt_timer_cb` leaves the register set
  after the event stream or a WFET timeout ends a wait, so the next WFE
  completes at once. WFE and WFET do not wait while the local exclusive
  monitor is armed, as QEMU's `HELPER(wfe)` chooses: the global monitor's
  clearing by another observer's store, a wake-up event, is not watched,
  and a device's DMA is such a store.
- **No RAS error records.** With FEAT_RAS, `ERRIDR_EL1.NUM` is 0. Arm's
  register data gives `ERRSELR_EL1` and the `ERX*` registers (those of
  FEAT_RASv1p1 too) to every processor with FEAT_RAS, each `ERX*` register
  reading the record `ERRSELR_EL1.SEL` selects, and their accessors trap
  EL1 to EL2 under `HCR_EL2.TERR` (the fault injection registers under
  `HCR_EL2.FIEN` clear); here they have those accessors, and with no
  record to select they read as zero and ignore writes, as Linux's KVM
  presents them to a guest with no error records. Arm permits it (DDI
  0601, release 2026-09): with zero error records "it is IMPLEMENTATION
  DEFINED whether ERRSELR_EL1 is UNDEFINED or RES0", and each `ERX*`
  register's page lists, when `ERRIDR_EL1.NUM` is 0, an UNKNOWN record,
  RAZ/WI, NOPs or UNDEFINED as the outcomes an access may have. QEMU's
  `minimal_ras_reginfo` makes them UNDEFINED, its comment stating that with
  no error records they may be, which loses the `HCR_EL2.TERR` trap a
  hypervisor sets. Nothing on this machine raises an SError, so ESB has nothing to
  defer into `DISR_EL1` and only an `MSR` writes that register; it keeps
  A, IDS and bits 23:0 (which IDS makes a syndrome or RES0) and reads the
  bits RES0 whatever IDS holds as zero, where QEMU's `disr_write` keeps all
  64. A
  synchronous external abort's error state (`ISS.SET`, which
  `PEErrorState` leaves IMPLEMENTATION DEFINED) is Recoverable (UER, 0b00),
  as QEMU reports it.
- **DBGAUTHSTATUS_EL1 reports Non-secure debug implemented.** Arm's
  `UpdateDbgAuthStatus` gives a Non-secure-only processor NSID and NSNID of
  0b10 (disabled) or 0b11 (enabled), by the external debugger's `DBGEN` and
  `NIDEN`, and FEAT_Debugv8p4 makes NSNID 0b11. With no external debugger,
  both signals are low here: 0x0A, or 0x0E with FEAT_Debugv8p4. QEMU (to
  master at least) has no DBGAUTHSTATUS_EL1, so reading it is UNDEFINED
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md), probe case
  M and its patch).
- **Vector FABS and FNEG leave a NaN alone under FPCR.AH.** Arm's FABS and
  FNEG (vector) apply FPAbs and FPNeg to each element, which return a NaN
  unchanged under FPCR.AH = 1, as the scalar forms do here and in QEMU.
  QEMU's vector forms (to master at least) flip or clear a NaN's sign
  anyway; the vectors were generated with its fix applied
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 12,
  `docs/patches/qemu/qemu-afp-vector-fabs-fneg.patch`).
- **SVE FCMLA (indexed) leaves a NaN alone under FPCR.AH.** Its rotation
  negates a Zm element with FPNeg, which returns a NaN unchanged under
  FPCR.AH = 1. QEMU's SVE form (to master at least) never passes FPCR.AH
  to its helper, so it flips the NaN's sign there, which its Advanced SIMD
  form does not: 3 of the first 1,000 floating-point vectors differed on
  it. The vectors were generated with its fix applied
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 13,
  `docs/patches/qemu/qemu-sve-fcmla-indexed-ah.patch`).
- **FMAD, FMSB, FNMAD, FNMSB and FSUBR with `size = 00` are UNDEFINED.**
  Their encodings carry no `size != '00'` condition, so the word is
  theirs, and their decode makes it UNDEFINED; FEAT_SVE_B16B16 adds no
  BFloat16 form of them. QEMU with FEAT_SVE_B16B16 (`-cpu max`, to master
  at least for FMAD and its three siblings, to 11.1.2 for FSUBR) executes
  the word as a BFloat16 operation. The SVE vectors give each of those
  classes a precision other than `size = 00` (`fp_size` in
  `gen_qemu_sve.py`, which also steers clear of the other features'
  encodings there), and `tests/unit/sve.rs` checks the UNDEFINED
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 14,
  `docs/patches/qemu/qemu-sve-fmad-size00.patch`).
- **An SME load to a vertical tile slice zeroes every inactive element.**
  LD1B to LD1Q write Zeros to each inactive element of the slice. QEMU
  (to master at least) zeroes a vertical slice's inactive elements up to
  its last active one and leaves those after it: 8 of the 80 tile load
  vectors differed on it. The vectors were generated with its fix
  applied, and `tests/unit/za.rs` checks the elements after the last
  active one ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)
  entry 15, `docs/patches/qemu/qemu-sme-ld1-vertical-tail.patch`).
- **MOVT between ZT0 and a general register is UNDEFINED.** Arm defines
  it only in Debug state, which this core never enters. QEMU (to master
  at least) executes it in any state. No vector covers it, and
  `tests/unit/zt0.rs` checks the UNDEFINED
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 16,
  `docs/patches/qemu/qemu-sme2-movt-debug-state.patch`).
- **PSCI is answered at the call, by firmware the processor cannot see.**
  The board has no EL3, and Arm makes `HVC` UNDEFINED on a processor
  without EL2 and `SMC` UNDEFINED on one without EL3 (outside the
  `HCR_EL2.TSC` trap, `AArch64_CheckForSMCUndefOrTrap`). QEMU's `virt`
  answers PSCI at the instruction instead, as firmware at a higher
  exception level would (`arm_handle_psci_call`), over `HVC` without EL2
  and over `SMC` with it (`virt.c`, `psci_conduit`), and writes that
  conduit in the device tree's `/psci` `method`. This model does the same:
  without EL2, `HVC` at EL1 is the PSCI call whatever its immediate, and
  `SMC` is UNDEFINED; with EL2, `HVC` is Arm's, and `SMC` at EL2, or at
  EL1 when `HCR_EL2.TSC` does not trap it, is the PSCI call. The call
  returns to the next instruction with its result in X0, as an SMC or HVC
  handled at a higher exception level does.
- **DBGPRCR_EL1 exists.** Arm's register data gives it to every AArch64
  processor, `MDCR_EL2.TDOSA` and `TDE` trapping EL1's accesses to EL2;
  here it keeps CORENPDRQ. QEMU (11.1.2) has no DBGPRCR_EL1, so an access
  is UNDEFINED at EL1 even where EL2 asked for the trap
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 17);
  the EL2 vectors leave it out.
- **RPRFM is a prefetch.** Arm's data allocates PRFM (register) with
  `Rt = 11xxx` to RPRFM (FEAT_RPRFM, Armv8.8). On a core without the feature
  the word is a PRFM with a reserved `prfop`, which the Arm ARM executes as a
  hint, as QEMU does; the decoder conformance test lists it as its one
  departure from Arm's allocation.
- **A pair of Q registers at 8 mod 16 is four aligned accesses.** Arm's
  `Mem` makes the 32-byte access of an LDP, STP, LDNP or STNP of Q
  registers at an address 8-byte aligned four 8-byte single-copy atomic
  accesses, each passed `aligned = TRUE`, so Device memory, and with the
  MMU off every data access, takes a pair at 8 mod 16 where one Q register
  there faults; `SCTLR_EL1.A` still judges the pair by its 16-byte
  elements. QEMU (master as of 2026-10-28) makes two 16-byte accesses
  that Device memory requires to be 16-byte aligned
  (`finalize_memop_asimd`, with the MMU off through `align_mem`, on a
  Device page in `get_phys_addr_lpae`), and so faults
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)). This model
  follows Arm.
- **A misaligned SVE element faults under `SCTLR_EL1.A`.** Each element of
  an SVE contiguous load or store, here in Streaming SVE mode, is an
  access of its own size (`Mem{msize}`), which `SCTLR_EL1.A` faults when
  misaligned (`AlignmentEnforced`). QEMU (11.1.2 and master as of
  2026-10-28) makes those element accesses unaligned (`MO_UNALN`) and
  checks only Device memory's alignment
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)). This
  model follows Arm.
- **A 16-byte single-copy atomic access reaches the bus as two 8-byte
  halves.** Arm makes some 16-byte accesses one single-copy atomic access:
  STXP and STLXP of X registers (`Mem{128}`), and with FEAT_LSE2 LDXP and
  LDAXP of X registers and an LDP or STP of X registers in one 16-byte
  granule; CASP of X registers is one atomic access too. The bus has no
  16-byte width (`AccessWidth`), so each is two 8-byte accesses, the low
  half first, with nothing between them: values, faults and the exclusive
  monitor are Arm's, and only a device would see two transactions where
  Arm makes one. The 8-byte cases are one access, as Arm makes them: LDXP
  and STXP of W registers, and with FEAT_LSE2 an LDP or STP of W or S
  registers in one 16-byte granule of Normal Write-Back Shareable memory
  (`AArch64_MemSingleRead`, `IsWBShareable`). On any other memory, Device
  memory and the MMU off among them, that pair is two accesses. QEMU 11.1.2
  issues every LDP and STP of words as one 8-byte operation, on any memory
  and with or without FEAT_LSE2 (`trans_LDP`, `trans_STP`,
  [docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 34);
  `tests/unit/load_store.rs` checks Arm's rule.
- **The bits of a scalable register above the length written are zeroed.**
  Where an instruction writes a Z or P register at a shorter length than
  the register's own (a SIMD&FP write of a V register, `V{128}(n)`; an
  SVE or SME write at the current vector length), and at the entry from
  EL0 that `MaybeZeroSVEUppers` covers, Arm leaves the bits above the
  length to the implementation (`Unpredictable_SVEZEROUPPER`), and the
  bits of ZA outside the current SVL square likewise
  (`Unpredictable_SMEZEROUPPER`). This model zeroes them, both taken as
  TRUE, as it zeroes ZA when PSTATE.ZA is cleared, which
  `ResetSMEState('0')` leaves ARBITRARY.
- **A predicated access with no element active still checks SP's
  alignment.** An SVE or SME load or store whose base is SP takes the SP
  alignment check even when its predicate has no element active
  (`Unpredictable_CHECKSPNONEACTIVE` taken as TRUE): the check depends on
  the base register alone, as for LDR and STR.
- **The reset management registers report this board's reset.** The
  highest EL has them (Arm's data: `IsHighestEL`): `RVBAR_EL1` and
  `RMR_EL1` without EL2, `RVBAR_EL2` and `RMR_EL2` with it. `RVBAR_ELx`,
  the IMPLEMENTATION DEFINED address execution starts from after reset,
  reads the address the machine's last reset started it from: the flash
  for firmware, the kernel's entry for a direct boot, where QEMU reports
  its `rvbar` property (0) whatever it boots. `RMR_ELx.AA64` is RAO/WI,
  there being no AArch32, and `RR`, an ordinary read/write field in Arm's
  register data, a request to the system for a Warm reset, keeps what is
  written: nothing on this board answers the request, as nothing on
  QEMU's `virt` does. QEMU makes both registers RAZ/WI but AA64, its
  comment noting that the bit is strictly a request, so RR reads 0 there
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md), "Deliberate
  QEMU omissions"); the EL2 vectors leave them out. `MDRAR_EL1` reports no
  ROM table (`Valid` 0b00), as QEMU's does.
- **The Cortex-A57's ID registers are its TRM's, cut to what the model
  has.** Every field whose feature the model implements reads the value
  of DDI 0488H (Table 4-1, 4.3.18 to 4.3.21, with the Cryptography engine
  and `GICCDISABLE` HIGH, the board's GIC being memory-mapped):
  `ID_AA64ISAR0_EL1` 0x11120; `ID_AA64MMFR0_EL1`'s `PARange` (44 bits),
  `ASIDBits` (16) and granules — the 4 KiB and 64 KiB ones, not the 16
  KiB one, and with EL2 the same at stage 2, the stage 2 fields reading
  0, "as stage 1" (no FEAT_GTG), where the TRM's bits 63:32 are RES0;
  `ID_AA64PFR0_EL1`'s FP, AdvSIMD and GIC; `ID_AA64DFR0_EL1`'s `DebugVer`
  (v8.0), `TraceVer` (0), `BRPs` (5), `WRPs` (3) and `CTX_CMPs` (1) — six
  breakpoints, the highest two, 4 and 5, context-aware, and four
  watchpoints, whose registers past them (`DBGBVR6_EL1`, `DBGWVR4_EL1` and
  on) are UNDEFINED (`src/cpu/debug.rs`) — and `PMUVer` (1, PMUv3), the
  whole register reading the TRM's 0x10305106; and `ID_AA64PFR1_EL1`,
  `ID_AA64DFR1_EL1`, `ID_AA64ISAR1_EL1` and `ID_AA64MMFR1_EL1` 0, the
  TRM's RES0 (10.7,
  Table 10-10). Where the model lacks a part of the core, the field says
  so, and differs from the TRM's: `ID_AA64PFR0_EL1` reads 0x11 (0x111
  with EL2) where the TRM's 0x2222 has AArch32 at EL0 to EL2 and an EL3,
  neither of which the model has; `ID_AA64MMFR0_EL1` reads 0x24 where the TRM's
  0x1124
  has `BigEnd` 1 (mixed endianness: `SCTLR_ELx.EE` and `SCTLR_EL1.E0E`
  configurable) and `SNSMem` 1 (Secure and Non-secure memory told apart),
  the model's accesses being little-endian whatever `EE` and `E0E` hold
  and its processor having no Secure state (Arm forbids `SNSMem` 0 only
  with EL3); and the AArch32 ID registers (`ID_PFR0_EL1` to
  `ID_ISAR5_EL1`, `MVFR0_EL1` to `MVFR2_EL1`), UNKNOWN on a processor
  without AArch32 (Arm ARM D24.2.113 and the others), read 0, where the
  TRM's describe its AArch32 state. QEMU's `cortex-a57` reads the TRM's
  `ID_AA64MMFR0_EL1` and r1p0's `MIDR_EL1` (below); a guest reads the
  same granules from both. The board also has the TRM's `MIDR_EL1`,
  `REVIDR_EL1`, `AIDR_EL1`, `CTR_EL0`, `CLIDR_EL1` and `DCZID_EL0`.
- **The Cortex-A57 is r1p3, the revision its TRM describes.** Its
  `MIDR_EL1` reads 0x411F_D073 (DDI 0488H, Table 4-1), where QEMU's
  `cortex-a57` reads r1p0's 0x411F_D070 (`aarch64_a57_initfn`). Of what
  the TRM's product revisions (1.8) change from r1p0 to r1p3, the model
  has `MIDR_EL1` and the IMPLEMENTATION DEFINED `CPUACTLR_EL1`, whose
  fields are r1p3's (the others are the trace unit's and the external
  debug interface's), and `REVIDR_EL1` and `AIDR_EL1` read the TRM's
  zeros. A guest applies the
  errata of the revision it reads: Linux 6.18 (`cpu_errata.c`) applies
  832075 and 834220 to r0p0 to r1p2 only, so on r1p3 it reads device
  memory with LDR, not LDAR (`ARM64_WORKAROUND_DEVICE_LOAD_ACQUIRE`), and
  its KVM takes the faulting IPA of a stage 2 Translation fault from
  `HPFAR_EL2` rather than translating `FAR_EL2` again with `AT S1E1R`
  (`ARM64_WORKAROUND_834220`, `__hpfar_valid` in
  `arch/arm64/kvm/hyp/include/hyp/fault.h`); 1319367 (all revisions) and
  1742098 (r0p1 on) apply on both, as do the Spectre mitigations and the
  KPTI exemption, which name every revision. A KVM host gives its guests
  its own `MIDR_EL1` through `VPIDR_EL2`, so a nested guest is r1p3 too.
- **The Cortex-A57's IMPLEMENTATION DEFINED registers are its TRM's.**
  `src/cpu/impdef.rs` gives the Cortex-A57 every AArch64 register of its
  TRM's IMPLEMENTATION DEFINED summary that a processor without EL3 has
  (DDI 0488H, 4.2.15, Table 4-15; the descriptions in 4.3), at EL1 and
  EL2, UNDEFINED at EL0:
  `L2CTLR_EL1` (`S3_1_C11_C0_2`, reset 0), `L2ECTLR_EL1` (`S3_1_C11_C0_3`,
  0), `L2ACTLR_EL1` (`S3_1_C15_C0_0`, 0x10, the ACE interface's),
  `CPUACTLR_EL1` (`S3_1_C15_C2_0`, 0), `CPUECTLR_EL1` (`S3_1_C15_C2_1`,
  0x1B_0000_0000), `CPUMERRSR_EL1` and `L2MERRSR_EL1` (`S3_1_C15_C2_2`
  and `_3`, 0), `CBAR_EL1` (`S3_1_C15_C3_0`, read-only), `IL1DATA0_EL1`
  to `IL1DATA3_EL1` (`S3_0_C15_C0_n`) and `DL1DATA0_EL1` to
  `DL1DATA4_EL1` (`S3_0_C15_C1_n`), 32-bit, UNKNOWN at reset and so the
  board's fill, and `RAMINDEX` (`SYS #0, C15, C4, #0`). Each write keeps
  its register's fields, the RES0 bits zero and the read-only fields as
  they are; `CBAR_EL1`'s `MSR` is UNDEFINED. The architectural registers
  whose contents Arm leaves to the implementation are the TRM's too:
  `ACTLR_EL2` keeps its five access controls (bits 6:4 and 1:0),
  and `ACTLR_EL1`, `AFSR0_EL1`, `AFSR1_EL1`, `AMAIR_EL1`, `HACR_EL2`,
  `AFSR0_EL2`, `AFSR1_EL2` and `AMAIR_EL2` are RES0. `HCR_EL2.TIDCP` traps
  every EL1 access of the space to EL2 (EC 0x18), as Arm's accessors do.
  Writes of `L2CTLR_EL1`, `L2ECTLR_EL1`, `L2ACTLR_EL1`, `CPUACTLR_EL1` and
  `CPUECTLR_EL1` need their `ACTLR_EL3` bit and, from EL1, their
  `ACTLR_EL2` bit (the TRM's footnotes to each); with no EL3 each
  `ACTLR_EL3` bit is 1, and without EL2 each `ACTLR_EL2` bit, the
  Effective value of a control that would take an exception to an absent
  level (Arm ARM D1.1.3, RHRDVX). The TRM does not say what a write it
  does not permit does: here it traps to EL2 with the syndrome
  `HCR_EL2.TIDCP` gives the same access, and changes nothing. The values
  that describe the board are the board's: `CBAR_EL1` reads the
  PERIPHBASE the layout ties to its GIC (`GicWindows::periphbase`: the
  GIC-400's base, 0x0800_0000, on the board's own layout, and QEMU's
  `VIRT_CPUPERIPHS`, 0x0800_0000 too, on QEMU's), and `L2CTLR_EL1`'s
  read-only fields one processor, ECC in the L2 only and no register
  slices (Table 1-1; the L2's 512 KiB is `CCSIDR_EL1`'s, L2CTLR having no
  size field). The board's documented behaviour: no memory error occurs,
  so the syndrome registers read zero and `L2ECTLR_EL1`'s error flags stay
  clear; `CPUACTLR_EL1` bit 8 makes `WFI` a NOP and bit 7 `WFE`, which
  then neither waits nor traps and leaves the Event Register alone; every
  other field — latencies, prefetchers, clock gating, dynamic retention,
  ECC enables, the barrier, coherency and streaming optimisations,
  `SMPEN` — is kept and read back and changes nothing, the model having no
  caches, timing, bus or other processor for it to act on (bit 5, `PLD`
  as a NOP, is what `PRFM` already is here); and `RAMINDEX` finds every
  cache and TLB RAM entry invalid, the model holding none, so in the
  Non-secure EL1 and EL2 that are this processor's only states it fills
  the data registers of the RAM's side with zeros (4.3.64), and with a
  reserved `RAMID`, which the TRM says corrupts them, both sides'.
  QEMU's `cortex-a57` (`cortex_a72_a57_a53_cp_reginfo` in
  `target/arm/cortex-regs.c`, and `ACTLR_EL2` in `helper.c`) reads all of
  its IMPLEMENTATION DEFINED registers as constants and ignores writes —
  `L2ACTLR_EL1` 0 where the TRM resets 0x10, `CPUECTLR_EL1` 0 where it
  resets 0x1B_0000_0000, `ACTLR_EL2` 0 whatever is written — lets EL1
  write them whatever `ACTLR_EL2` holds, and lacks `IL1DATAn_EL1`,
  `DL1DATAn_EL1` and `RAMINDEX`, which are UNDEFINED there
  (docs/qemu-upstream-bugs.md, entry 57). The Armv9.2 model, which no
  document gives such registers, has none: the space's every access is
  UNDEFINED or `HCR_EL2.TIDCP`'s trap, and its architectural
  IMPLEMENTATION DEFINED registers keep all 64 bits. A snapshot carries
  the Cortex-A57's registers.
- **A virtual SError reports no syndrome without FEAT_RAS.** On the
  Cortex-A57 with EL2, `HCR_EL2.VSE`'s SError reaches EL1 with `ESR_EL1`
  EC 0x2F, IL set and `ISS` 0: `IDS` clear, so bits 23:0 are RES0 (the
  ESR's ISS encoding for an SError exception). Arm's
  `AArch64_TakeVirtualSErrorException` leaves the syndrome IMPLEMENTATION
  DEFINED ("Virtual SError syndrome"), reported only with its `IDS` set.
  The Cortex-A57's own IMPLEMENTATION DEFINED SError syndrome (DDI 0488H,
  Tables 4-61 and 4-66) describes an error in the memory system — whether
  it can be attributed and contained, and its source, a decode, ECC or
  slave error — and a virtual SError, which a hypervisor raises carrying
  no error, is none of them, so the processor reports no syndrome rather
  than a source that did not err. QEMU's reads the same. Under FEAT_RAS
  the syndrome is `VSESR_EL2`'s.
- **The Armv9.2 model offers its stage 2 granules at stage 2.** With EL2
  it would have FEAT_GTG, which Arm requires of an Armv8.5 processor with
  EL2: `ID_AA64MMFR0_EL1.TGran4_2` and `TGran16_2` 3 and `TGran64_2` 2.
  The Cortex-A57, an ARMv8.0 core without it, reads 0 in all three, "as
  stage 1", its TRM's RES0, as QEMU's does.
- **Both stages make the same IMPLEMENTATION DEFINED choices.** The
  pseudocode asks both stages the same questions: a `TxSZ` below its
  minimum (at stage 1 16, or 12 with FEAT_LVA and the 64 KiB granule or
  FEAT_LPA2's `DS`; at stage 2 64 − PAMax, 20 on the Cortex-A57, and with
  the Armv9.2 model's 52-bit `PAMax` 12 with the 64 KiB granule or `DS`
  and 16 otherwise) or
  above `AArch64_MaxTxSZ` (39; with FEAT_TTST 48, or 47 with the 64 KiB
  granule) is taken as that limit for every purpose but reading it back
  ("Fault on TxSZ value below minimum" and "... above maximum" FALSE, the
  first behaviour D8.1.9 permits, R_YXNYW and R_SXWGM), except below the
  minimum with FEAT_LVA at stage 1 and FEAT_LPA at stage 2, where Arm
  requires a level 0 Translation fault. The Cortex-A57's TRM does not say
  which its core does; the limit is the value the architecture gives an
  out-of-range `TxSZ` wherever it decides one itself —
  `AArch64_PACEffectiveTxSZ` takes it for a pointer authentication code's
  bottom bit, by the `Unpredictable_RESTnSZ` its `ConstrainUnpredictable`
  resolves to `Constraint_FORCE`, and D8.10.1 (R_FCMCH) makes a processor
  that translates with the limit take it there too — so one rule holds for
  both. QEMU faults instead (`tsz_oob`): a guest that writes such a `TxSZ`
  translates the limit's input address here where QEMU's takes a level 0
  Translation fault (`tests/translation_and_firmware.rs`,
  `tests/unit/granules.rs`). A `TG0` or
  `TG1` naming a granule the processor lacks, or its reserved encoding, is
  the 4 KiB one ("TG0 encoded granule size", "TG1 encoded granule size",
  one granule whatever the encoding), the one granule both processor
  models implement at both stages, so the encoded granule is always one
  the processor walks (QEMU's `sanitize_gran_size` takes the same): so on
  the Cortex-A57 a `TG0` or `TG1` naming the
  16 KiB granule walks 4 KiB tables, at stage 1 and, through
  `VTCR_EL2.TG0`, at stage 2. Its TRM does not decide it: it gives `TG0`
  as bit 14 alone, bit 15 RES0, and `TG1` as bit 30 alone, bit 31 RES1
  (DDI 0488H, 4.3.41 to 4.3.43), so every 16 KiB and reserved encoding
  sets a reserved bit; with the 64
  KiB granule where `PAMax` is under 52 bits, neither a descriptor's bits
  15:12 nor a table base register's bits 5:2 are address bits 51:48
  ("descriptor[15:12] for 64KB granule are OA[51:48]" and "BADDR
  expresses 52 bits for 64KB granule" FALSE, as QEMU uses them only for a
  52-bit output size, which needs FEAT_LPA); the
  Contiguous bit is treated as 0, and a clear access flag faults for cache
  maintenance too. A synchronous External abort on stage 2's walk reports
  no instruction syndrome ("ISV on second stage translation table walk"
  FALSE, the value FEAT_RAS forces on the Armv9.2 model), where QEMU's
  `merge_syn_data_abort` reports the instruction's. Stage 1 reports its
  effective shareability under stage 2 as without ("Apply effective
  shareability at stage 1" TRUE), which nothing a guest reads tells apart:
  the two values differ only for Device and Non-cacheable memory, which
  `S2CombineS1MemAttrs` and `PAREncodeShareability` make Outer Shareable
  whatever stage 1 said.
- **`HPFAR_EL2` keeps its value for a stage 2 fault whose IPA Arm does not
  report.** `IPAValid` reports the IPA for a Translation, Access flag or
  Address size fault, and on stage 1's walk for a Permission fault too;
  for every other stage 2 fault `AArch64_ReportException` leaves
  `HPFAR_EL2` ARBITRARY, and here it keeps its value. QEMU writes the IPA
  for every stage 2 fault, which Arm permits too; Linux's KVM finds the
  IPA of a Permission fault with `AT S1E1R`, as Arm intends.
- **Several asynchronous exceptions deliverable at once are taken in the
  board's order.** Arm leaves the prioritization of physical and virtual
  asynchronous exceptions IMPLEMENTATION DEFINED (DDI 0487M.d, D1.4.6.5),
  and the Cortex-A57 TRM gives none. QEMU's `arm_cpu_exec_interrupt`
  takes a physical FIQ, a physical IRQ, then a virtual IRQ, FIQ and
  SError. The board's order is a physical FIQ, a physical IRQ, then a
  virtual SError, FIQ and IRQ. FIQ goes before IRQ, physical and virtual,
  as the interrupt controller ranks them: a GICv2 CPU interface signals
  Group 0 as FIQ and Group 1 as IRQ under `FIQEn` (IHI 0048B, 3.9.2 for a
  GIC without the Security Extensions, as this board's is, 3.9.3 for the
  virtual interface), and with `AckCtl` 0, the model Arm recommends,
  every Group 0 interrupt must have a higher priority than every Group 1
  one (3.4, 5.2.3). The virtual SError, an error report, goes first of
  the virtual ones, so a guest learns of the error before it services an
  interrupt on state the error may have touched; EL2 raises it once
  (`HCR_EL2.VSE`, cleared when it is taken, D1.4.6.1) where a virtual IRQ
  or FIQ stays pending. A physical interrupt goes before a virtual one:
  where its own virtual kind exists it is routed to EL2, which once it has
  serviced it may change what is virtually pending (its `HCR_EL2` bits,
  the GIC's List registers) before EL1 runs a handler. No physical SError
  is ever pending here. `tests/unit/el2.rs`
  (`virtual_interrupts_are_taken_at_el1`) checks the order.
- **The TLB keeps the entries of an EL1&0 context it leaves.** Arm's TLB
  tags an entry with its VMID and ASID and keeps it until a TLB
  maintenance instruction removes it, so a switch to other values and back
  may find it again; software that changes a translation table issues the
  `TLBI` Arm requires. This TLB keeps the entries of up to fifteen sets of
  the EL1&0 translation registers' values with EL2, and the stage 2
  translations of up to fifteen pairs of `VTTBR_EL2` and `VTCR_EL2`, and
  never uses one under other values. QEMU 11.1.2 flushes its TLB on a `VTTBR_EL2` write that
  changes the VMID and on a `TTBRn_EL1` write that changes the ASID
  (`target/arm/helper.c` `vttbr_write`, `vmsa_ttbr_write`), so a guest that
  rewrites a table without the `TLBI` and then returns to it sees the new
  mapping there, and the old one here, as Arm permits.
- **A `TLBI RIPAS2E1` range's base address is in FEAT_LPA2's form under
  `VTCR_EL2.DS`.** Arm's two descriptions disagree: the `TLBI RIPAS2E1`
  page of `Registers.json` gives `BaseADDR` FEAT_LPA2's form where
  `VTCR_EL2.DS` is 1, while `AArch64_TLBI_RIPAS2` hands `TLBIRange` the
  EL1&0 regime, whose `HasLargeAddress` reads `TCR_EL1.DS`. The range is
  of IPAs, which stage 2's walk, under `VTCR_EL2`, translates, so the
  register page is followed (`src/cpu/tlbi.rs`). No processor here meets
  the case yet: the Armv9.2 model, the one with FEAT_TLBIRANGE and
  FEAT_LPA2, has no EL2.
- **`LDLAR` and `STLLR` report `AR` 1 with a stage 2 Data Abort.** The
  Arm ARM's `ESR_ELx.AR` is 1 for an instruction with acquire or release
  semantics, and the LORegion forms have them (Arm `CreateAccDescLOR`:
  `acqsc`, `relsc`), within their LORegion. QEMU 11.1.2 passes the
  encoding's `o0` bit as `AR` (`target/arm/tcg/translate-a64.c`
  `trans_LDAR`, `trans_STLR`: `a->lasr`), which is 0 for them
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 26).
- **A stage 2 fault past a split access's first byte reports no
  instruction syndrome.** `Mem[]` splits a misaligned access into bytes
  and clears `accdesc.lowestaddress` after the first, and
  `AArch64_InstructionSyndromeValid` reports `ISV` only for a fault at the
  lowest address; so a load or store crossing into a page stage 2 refuses
  is taken to EL2 with `ISV` 0, and the hypervisor decodes the instruction
  itself. QEMU's `merge_syn_data_abort` reports the instruction's syndrome
  for every stage 2 abort on the access, wherever in it the fault lies
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 27).
- **A reserved `MAIR_ELx` attribute is Device-nGnRnE.**
  `S1DecodeMemAttrs` maps a reserved encoding (`Unpredictable_RESMAIR`) to
  an UNKNOWN allocated one, which the pseudocode's own
  `ConstrainUnpredictableBits` makes zeros: Device-nGnRnE, so a misaligned
  access to it faults and `PAR_EL1.ATTR` reports 0x00. A reserved stage 2
  `MemAttr` is Normal Non-cacheable, 0b0101 (`Unpredictable_S2RESMEMATTR`'s
  `Constraint_NC`). QEMU takes a `MAIR_ELx` attribute as written, Normal
  unless its high four bits are zero, and reports it so.
- **`PAR_EL1.ATTR` reports the attributes the translation gives.**
  `AArch64_AT` encodes the attributes of the translation, so with
  `SCTLR_ELx.C` clear stage 1's Normal memory is reported Non-cacheable
  (`AArch64_S1TreatAsNormalNC`), 0x44, as is stage 2's under `HCR_EL2.CD`
  (`AArch64_S2TreatAsNormalNC`), as D7.5.5 requires: "The effect of
  SCTLR_ELx.C, HCR_EL2.DC and HCR_EL2.CD is reflected in the result of the
  address translation instructions in the PAR". The comment on the
  IMPLEMENTATION DEFINED `ReportedPARAttrs` lets the field be the resulting
  attribute or the descriptor's; for these three controls D7.5.5 decides.
  QEMU reports stage 1's `MAIR_ELx` attribute as written whatever
  `SCTLR_ELx.C` says ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)
  entry 35). Without FEAT_XS the
  encodings of memory with XS = 0 — 0x40, 0xA0, and Device with bit 0 set
  — are reserved, and `PAR_EL1.ATTR` takes `MAIR_ELx`'s encodings; the
  pseudocode's `EncodePARAttrs` reaches them without testing the feature,
  Normal Write-Back memory at stage 1 having XS = 0 whatever the processor,
  so here a processor without FEAT_XS reports such memory as if XS were
  1 (0x44 rather than 0x40), and one with it as the pseudocode does.
- **A reserved `SH` of 0b01 is Outer Shareable.** `DecodeShareability`
  takes the reserved value through
  `ConstrainUnpredictable(Unpredictable_Shareability)`, which permits Outer,
  Inner or Non-shareable; this model takes `Constraint_OSH`, at either
  stage, so `PAR_EL1.SH` reports 0b10 for memory a descriptor maps with
  `SH` 0b01. QEMU passes the descriptor's `SH` through to `PAR_EL1.SH` as
  written, 0b01, which `PAREncodeShareability` never gives
  (`target/arm/tcg/cpregs-at.c` `par_el1_shareability`;
  [docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 28).
- **No FEAT_ECV_POFF without EL2.** Arm's `Features.json` gives FEAT_ECV_POFF
  only to a processor with EL2 (`(FEAT_AA64 && FEAT_ECV_POFF) ==>
  FEAT_AA64EL2`), so a processor built without EL2 drops it, and with
  FEAT_ECV its `ID_AA64MMFR0_EL1.ECV` reads 1. QEMU 11.1.2's `max` sets
  `ECV` to 2 (`target/arm/tcg/cpu64.c` `aarch64_max_tcg_initfn`) and keeps
  it when `virtualization=off` takes EL2 away: `arm_cpu_realizefn` clears
  the hypervisor fields of the ID registers then, but not `ECV`
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 29).
- **A 64 KiB descriptor's bits 15:12 are address bits whatever the output
  size.** With a 52-bit `PAMax` (FEAT_LPA), Arm's `AArch64_NextTableBase`,
  `AArch64_S1LeafBase` and `AArch64_S2LeafBase` take them as bits 51:48 of
  the address the descriptor gives, so under an `IPS` or `PS` below 52
  bits a nonzero value is an Address size fault. QEMU 11.1.2's
  `get_phys_addr_lpae` takes them only for a 52-bit output size and
  otherwise drops them, translating where Arm faults
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 30,
  probe case Q).
- **A table base register's bits 5:2 are `BADDR[51:48]` under `PS` 0b110
  alone.** Arm's `AArch64_S1TTBaseAddress` and `AArch64_S2TTBaseAddress`
  take them so with FEAT_LPA and the 64 KiB granule when the field is
  0b110; under 0b111 (56 bits, 52 with this `PAMax`) they are the base's
  own bits 5:2, below the alignment of any start table of 64 bytes or
  more. QEMU 11.1.2 takes them as bits 51:48 whenever the output size is
  52 bits, 0b111 too
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 31,
  probe case R).
- **Under FEAT_LPA2's `DS` a descriptor's bits 9:8 are address bits
  whatever the output size.** Arm's `AArch64_NextTableBase`,
  `AArch64_S1LeafBase` and `AArch64_S2LeafBase` take them as bits 51:50
  of the address whenever `DS` is set, so under an `IPS` or `PS` below 52
  bits a nonzero value is an Address size fault. QEMU 11.1.2 takes them
  only for a 52-bit output size and otherwise drops them, translating
  where Arm faults ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)
  entry 32, probe case S).
- **Under FEAT_LPA2's `DS` a table base register's bits 5:2 are
  `BADDR[51:48]` whatever the output size.** Arm's
  `AArch64_S1TTBaseAddress` and `AArch64_S2TTBaseAddress` take them so
  whenever `DS` is set, so under an output size below 52 bits a nonzero
  value is a level 0 Address size fault. QEMU 11.1.2 takes them only for a
  52-bit output size and otherwise as the base's own bits 5:2, translating
  where Arm faults ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)
  entry 33, probe case T).
- **`DC ZVA` on Device memory is an alignment fault.** Arm ARM
  `AArch64.S1HasAlignmentFault` faults every `DC ZVA` to Device memory, and
  with the MMU off every data access is to Device memory; under stage 2,
  `AArch64_S2HasAlignmentFaultDueToMemType` faults it too where stage 2
  maps Device memory, a stage 2 Alignment fault taken to EL2. QEMU 11.1.2
  zeroes the block instead at either stage (`target/arm/tcg/helper-a64.c`
  `HELPER(dc_zva)`, whose comment notes that it does not implement the
  architecturally mandated fault); this model takes the fault.
- **The Event Register is set by exception returns and the event stream
  too.** Arm ARM `AArch64.ExceptionReturn` sets it (`SendEventLocal`), and
  so does each event of the virtual event stream (`TestEventCNTV`, under
  `CNTKCTL_EL1.EVNTEN`). QEMU 11.1.2 sets it for `SEV` and `SEVL` only. The
  register decides whether a WFE at EL0 traps under `SCTLR_EL1.nTWE`; this
  model follows the Arm ARM. A WFE that does not consume an event completes
  at once rather than waiting, as the Arm ARM permits, so only the
  instruction count of a WFE loop differs from QEMU, whose WFE sleeps.
- **A store across a page boundary.** Arm's `Mem` setter writes element by
  element and leaves what a faulting store has written unspecified. One
  register, a pair or a pair of Q registers across pages here translates
  both pages before writing anything, so a fault leaves memory as it was, as
  QEMU does (accel/tcg/cputlb.c `mmu_lookup`). `ST1` and the SVE and SME
  stores make each element an access of its own, as Arm's do, so a fault on
  a later element leaves the earlier ones written. A load across pages reads
  a byte at a time, and reports the first byte that faulted.
- **An 8-byte access to the PL031 or PL061 is two word accesses.** Their
  TRMs describe 32-bit APB slaves and say nothing of a 64-bit master, which
  leaves it to the bus bridge. QEMU's memory core splits the access for a
  device that declares no access size (`access_with_adjusted_size`), so
  `ldr x0, [rtc]` reads RTCDR and then RTCMR, the low word first. This model
  does the same; the tests in `src/devices/pl031.rs` and
  `src/devices/pl061.rs` check it.
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
- **The firmware is PSCI 1.2 and SMCCC 1.5.** It reports PSCI 1.2 (DEN
  0022E), where QEMU 11.1.2 reports 1.1, with the same functions: every
  one DEN 0022E Table 17 makes mandatory, and `MIGRATE_INFO_TYPE`, which
  answers 2, the answer §5.9 gives "a system that does not use a Trusted
  OS", as this board has none: the call by which the firmware says so,
  where `NOT_SUPPORTED` would leave the OS to infer it (§5.1.7). The rest
  of Table 17's optional functions are features the board does not have
  (suspend, a second reset, residency statistics, memory protection). It
  implements SMCCC 1.5 (DEN 0028F), where QEMU's firmware has no SMCCC
  call and so reads as SMCCC 1.0: `SMCCC_VERSION`, which `PSCI_FEATURES`
  reports (DEN 0022E §5.15.2), `SMCCC_ARCH_FEATURES`, and the three
  speculation workarounds as the host's `SpectreReport` answers them
  (§7.5 to §7.7). The interpreter executes nothing speculatively, so by
  default (`NotRequired`) each workaround is reported as not required,
  `WORKAROUND_1` and `WORKAROUND_3` being calls with no effect;
  `Mitigated` reports each as required and provided, and `Unreported`
  gives no information, as SMCCC 1.0 firmware does. `SMCCC_ARCH_SOC_ID`,
  `SMCCC_ARCH_FEATURE_AVAILABILITY` and the general service queries are
  optional (§6.2, §7.4, §7.8) and not implemented.
- **PSCI as DEN 0022D specifies it.** In four cases QEMU (target/arm/tcg/psci.c
  `arm_handle_psci_call`, target/arm/arm-powerctl.c `arm_set_cpu_on`)
  answers otherwise; this model answers as the specification says, or,
  where it leaves the choice open, as stated here:
  - The function identifier is W0 in both conventions, and an SMC32
    call's arguments are W1 to W3 (DEN 0028B §2.6 and §2.7, and §3.1
    Table 3-1, whose `Wn` is a register's low 32 bits with the upper ones
    ignored — §3.1 says "The least significant 32-bits are used, and the
    most significant 32-bits are zero. Implementations must ignore the
    least significant bits", the last word a slip for "most"; DEN 0022D
    §5.2.1), as are the parameters DEN 0022D declares
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
- **The device tree's seeds and L1 data cache.** QEMU draws a fresh
  `/chosen` `rng-seed` and `kaslr-seed` from the host on every boot and
  every reset (`qemu_fdt_randomize_seeds`). This board writes neither:
  Linux credits an `rng-seed` as entropy (`random.trust_bootloader`), and
  a deterministic machine would give the same bytes to every boot of the
  same configuration and again after each reset, so a guest's keys would
  be computable from its configuration. The guest gathers its own
  entropy, and a directly booted kernel, finding no `kaslr-seed`, leaves
  its base where it loaded. A host's own tree keeps whatever `/chosen`
  seeds the host wrote into it, which then repeat as the host wrote them.
  QEMU 11's
  `cortex-a57` (`aarch64_aa32_a57_init` in `target/arm/cpu-max.c`) makes
  the L1 data cache 4-way, where the Cortex-A57 MPCore TRM, and QEMU 8.2,
  make it 2-way; this model's `CCSIDR_EL1` is the TRM's, and the tree's
  `d-cache-sets` is `0x100` to agree with it.
- **The GIC makes the GIC-400's choices where IHI 0048B leaves them to
  the implementation.** By default (the host's `VirtConfig::gic` may
  choose fewer SPIs, from 64, and four to eight
  priority bits) it has 480 SPIs, the GIC-400's largest
  configuration (DDI 0471B §2.3.3), so `GICD_TYPER.ITLinesNumber` reads
  15. Its PPIs are the GIC-400's 25 to 31 (§3.3), PPI 22, on which the
  board drives the DCC's COMMIRQ, the PPI BSA recommends for it (DEN0094C
  RB_PPI_01), which V-BSA makes a PPI of in every virtual environment
  (DEN0150 RV_L1PP_00), and PPI 23, on which it drives the PMU's PMUIRQ,
  the PPI BSA recommends for that (RB_PPI_01) and the Arm ARM too (D13.3.1),
  neither of which the GIC-400 has; PPIs 16 to 21 and 24 do not exist and
  so are RAZ/WI in every register, and every
  PPI is level-sensitive and cannot be made edge-triggered (Table 3-2,
  `GICD_ICFGR1` RO). It has five priority bits,
  32 levels (§2.6, Secure state: this GIC has no Non-secure view), so
  every priority field's bits [2:0] are RAZ/WI (IHI 0048B Table 3-1); its
  virtual interface's five (`GICH_VTR.PRIbits`) are the architecture's,
  whatever the physical GIC's are. The minimum binary points
  are then 2 for `GICC_BPR` and 3 for `GICC_ABPR`, raised to on a lower
  write and set on reset, and the 32 preemption levels are bits of
  `GICC_APR0` and `GICC_NSAPR0` alone (Table 4-47; DDI 0471B Table 3-6).
  QEMU's `virt` gives its GICv2 256 SPIs, every PPI, PPIs whose trigger
  mode can be written, eight priority bits, minimum binary points of 0
  and 1, and four of each active priority register. The GIC-400's legacy
  `Int_config[0]` bits (DDI 0471B §3.4.2) are not taken: IHI 0048B
  reserves them, and they read as zero.
- **The devices' SPIs and the GICv2m frame's are the board's own.** IHI
  0048B leaves which source drives which SPI to the system, and BSA gives
  the GICv2m frame "an IMPLEMENTATION DEFINED number of contiguous SPIs"
  that no other source shares (DEN0094C I.5). By default
  (`VirtConfig::interrupts`, `InterruptMap::DEFAULT`) the devices take SPIs
  from 0 in the order of their registers' addresses — the UART 0, the RTC
  1, the GPIO 2, the 4 virtio transports 3 to 6, the PCIe host's INTA to
  INTD 7 to 10 — and the frame every SPI from the first after them whose
  interrupt ID is a multiple of 32, SPI 32, so its `MSI_TYPER` reads base
  INTID 64 and 448 SPIs on the GIC-400's 480. A function sending multiple
  messages sets the low bits of the message data to the vector it sends,
  up to 32 of them (PCI Local Bus Specification 3.0 §6.8.1.6), and the
  data is the SPI's interrupt ID (BSA I.9.2), so a frame based at a
  multiple of 32 gives any block a guest aligns within it SPIs of its own;
  SPIs 11 to 31 are left free. The same rule on QEMU's layout, of 32
  transports (`InterruptMap::own(DeviceLayout::QemuVirt)`), puts them at 3
  to 34, INTA to INTD at 35 to 38 and the frame from SPI 64, INTID 96; a
  map is checked against its layout's transports, so the board's own
  layout's map is refused on QEMU's, its transports running into the PCIe
  host's lines. QEMU's `virt` wires them as `a15irqmap`
  does, after the Versatile Express it grew out of: the UART SPI 1, the RTC
  2, the PCIe host 3 to 6, the GPIO 7, the transports 16 to 47, and a frame
  of 64 SPIs from INTID 80 (SPI 48), leaving SPI 0 and 8 to 15 unused. A
  host's device tree given in place of the board's names the devices'
  interrupts itself; QEMU's `arm_load_dtb` takes one whatever it names, and
  its guest waits on lines nothing drives. This board refuses one that
  names, for an enabled UART, RTC, GPIO or virtio transport node, by its
  `interrupts` (through its `interrupt-parent`) or its
  `interrupts-extended`, an SPI the map does not wire it to
  (`BuildError::HostTreeInterrupt`). It reads the tree as the board's and
  QEMU's are written, the devices children of the root and their
  interrupts going to a controller of three-cell specifiers; a device node
  placed deeper, under a bus, it does not look for. A tree written for QEMU's
  `virt` takes QEMU's wiring as the board's map (`[virt.interrupts]` in the
  front end, `--interrupts` for the examples). The PCIe host's INTA to INTD
  and the GICv2m frame's SPIs no device of the board drives, its bus
  having none, so a tree describes them as it likes.
- **The first device plugged in takes the first virtio transport.** The
  virtio specification leaves where a device sits to the platform (OASIS
  virtio 1.2 §4.2). By default (`VirtConfig::transport_order`,
  `TransportOrder::FirstFree`) the display, the keyboard, the tablet and
  the disk, in that order, take the transports from the first up (from
  0x0805_0000 on the board's own layout), so the first is the first the
  device tree lists and the guest probes. QEMU's `virt` fills them from
  the last down (`create_virtio_devices`, its bus list prepended), putting
  a lone disk at the last transport (0x0A00_3E00 on its layout);
  `TransportOrder::LastFree` does the same.
- **The devices sit at the board's own addresses.** Arm's *Principles of
  ARM Memory Maps* (DEN0001C, `cpp_orig/arm-docs/memmap/`) put a system's
  peripherals in the 32-bit map's region for ROM, RAM and SoC I/O (4.1.2),
  each in one or more 64 KiB pages of its own (P29), and mapped I/O such as
  PCIe at 256 GiB to 512 GiB of a 40-bit map (4.3.2); the GIC-400 lays its
  frames out in one block (DDI 0471B Table 3-1: reserved to 0x0FFF, the
  distributor at 0x1000, the CPU interface at 0x2000, `GICH` at 0x4000 and
  its alias at 0x5000, `GICV` at 0x6000). By default (`VirtConfig::layout`,
  `DeviceLayout::Board`) the board does both: the GIC-400's block at
  0x0800_0000, the first 64 KiB page after the flash; the GICv2m frame,
  the UART, the RTC, the GPIO and each of the 4 virtio transports in the
  next pages (0x0801_0000, 0x0802_0000, 0x0803_0000, 0x0804_0000, and
  0x0805_0000 to 0x0808_0000), the PCIe host's I/O window after them
  (0x0809_0000);
  its ECAM at 256 GiB and its 64-bit window at 384 GiB, 128 GiB of it,
  each aligned to its size; and its 32-bit window, which the 32-bit map
  puts at 1 GiB to 2 GiB where RAM is here, in the top half of the I/O
  region (0x2000_0000, 512 MiB). QEMU's `virt` gives the GIC's distributor,
  CPU interface and virtual interface 64 KiB each from 0x0800_0000, the
  PrimeCells from 0x0900_0000 and the virtio transports 512 bytes apart
  from 0x0A00_0000, with its PCIe windows at 0x3EFF_0000, 0x1000_0000 and
  512 GiB (`base_memmap`, `extended_memmap`); `DeviceLayout::QemuVirt` is
  that layout, for a guest whose device tree or firmware variable store
  was made for QEMU's addresses. RAM at 0x4000_0000 and the flash at 0
  stay where QEMU has them in both, as stock EDK2 is built to find them
  ("QEMU fingerprints left"). A host's tree that puts the GIC, a
  PrimeCell, a virtio transport or the PCIe host anywhere but where the
  layout has it is refused (`BuildError::HostTreeAddress`), as the board
  would decode nothing where the guest looks.
- **The power button is on the GPIO's first line.** DDI 0190B leaves what
  each PL061 line is wired to to the system. On the board's own layout
  the power button drives line 0, the first, which the device tree's
  `gpio-keys` key names (`gpios = <&gpio 0 0>`), where Linux's
  `gpio-keys` driver takes it from. QEMU's `virt` wires its `gpio-key` to line
  3 (`create_gpio_keys`); `DeviceLayout::QemuVirt` keeps that line with
  QEMU's addresses, so a tree QEMU wrote names the line the button drives
  (`DeviceLayout::power_button_line`). A host's tree whose `gpio-keys`
  puts an enabled key, by its `gpios`, on another line of the board's
  GPIO is refused (`BuildError::HostTreePowerButton`): nothing drives the
  board's other lines, so its guest would wait on one that never changes.
  `tests/api.rs` reads the line rise and fall in `GPIODATA` and its
  interrupt pending at the GPIO's SPI.
- **A directly booted kernel is placed and entered as Linux's boot
  protocol asks, and the protocol's open choices are the board's.**
  Linux's arm64 `Documentation/arch/arm64/booting.rst` fixes the entry
  state — `x0` the device tree's physical address, `x1` to `x3` zero
  (§4) — and names no other register; every other register here holds
  its reset value, for `x4` to `x30` and the stack pointer the board's
  fill ("Registers Arm leaves UNKNOWN at reset" below). QEMU enters through a boot
  stub of its own at the base of RAM (`bootloader_aarch64` in
  `hw/arm/boot.c`), which leaves the kernel's entry in `x4`. The protocol
  places the kernel `text_offset` above a 2 MiB-aligned base, asks for
  `image_size` bytes free from its start, keeps the device tree 8-byte
  aligned, at most 2 MiB and out of any 2 MiB block mapped with
  attributes of its own (§2; within 512 MiB of the base before v4.2),
  and the initrd in a 1 GiB-aligned window of at most 32 GiB that covers
  the kernel (§4). Here the base is the base of RAM, which an `Image`
  with flags bit 3 clear asks to be "as close as possible to"; the device
  tree takes the 2 MiB block at the first 2 MiB boundary after the
  kernel's `image_size`, and the initrd starts at the next, leaving RAM
  above it one free run; an initrd that would end beyond the 32 GiB
  window is refused (`PlacementError::InitrdOutOfReach`). QEMU moves a
  kernel whose `text_offset` is below 4 KiB up 2 MiB to clear its stub
  (`BOOTLOADER_MAX_SIZE`), so a kernel of `text_offset` 0, as recent
  kernels' is, is entered 2 MiB higher there; puts the initrd at half of RAM up to
  128 MiB, or above the kernel when that is higher; and the device tree at
  the 2 MiB boundary after the initrd (`arm_setup_direct_kernel_boot`).
  For an `Image` too old to give its size (before v3.17) both load it at
  0x80000, as §4 says to assume; the protocol asks a boot loader to "keep
  as much memory as possible free for use by the kernel immediately after
  the end of the kernel image", so the tree here takes the last 2 MiB
  block below the end of RAM or of the 512 MiB such a kernel searches,
  whichever is lower, and the initrd ends where that block starts.
  `tests/api.rs` and `src/boot.rs` check where each piece lands and the
  registers the kernel finds.
- **The board has a virtio transport for each device it can plug in, each
  decoding its page.** OASIS virtio 1.2 §4.2.2 fixes a transport's
  registers to 0x100 and puts the device's configuration after them; how
  many transports a platform has and how far each one's window runs it
  leaves to the platform. On the board's own layout (`DeviceLayout::Board`)
  there are 4, one each for the display, the keyboard, the tablet and the
  disk, so none is there that no device could fill, and each decodes the
  whole 64 KiB page the layout gives it (DEN0001C P29), which its device
  tree node's `reg` names: the registers, the device's configuration, and
  past the configuration what a read past it gets in any transport (all
  ones behind a device, zero on an empty transport). QEMU's `virt` has 32
  (`NUM_VIRTIO_TRANSPORTS`), which its tree lists and a guest probing them
  finds, 512 bytes apart and each decoding 512 bytes (`base_memmap`'s
  `VIRT_MMIO`, `virtio_mmio_realizefn`), which each node's `reg` names.
  `DeviceLayout::QemuVirt` keeps both: the count because a tree or
  firmware variable store QEMU wrote names all 32, the window because its
  transports are 512 bytes apart. The transports take a run of SPIs as
  long as the layout has transports, so the board's own interrupt map is
  the layout's (`InterruptMap::own`), and a map is checked against the
  layout it is used with.
- **The system counter runs at 1 GHz on both processors by default.** Before
  Armv8.6 the Arm ARM leaves the counter's frequency to the system, BSA
  asking at least 10 MHz (DEN0094C RB_TIME_02); from Armv8.6 it is 1 GHz.
  QEMU runs the Cortex-A57's at 62.5 MHz (`GTIMER_BACKCOMPAT_HZ`) on
  every machine type, the processor type being one it keeps at the old
  rate (`ARM_FEATURE_BACKCOMPAT_CNTFRQ`), as it does every processor on
  `virt-9.0` and older; this board gives the Cortex-A57 the same 1 GHz
  as the Armv9.2 model, which `CNTFRQ_EL0` reads at reset and firmware
  and Linux take from it. A host may give the Cortex-A57 another rate
  BSA allows (`VirtConfig::counter`); the Armv9.2 model takes 1 GHz alone.
- **The device tree describes what the board has, as it has it.** Each
  virtio-mmio transport's interrupt is level-high, the line being held
  while its `InterruptStatus` is not zero; QEMU's `virtio_mmio_update_irq`
  drives the same level but its tree calls it rising-edge
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 53).
  The GIC node's regions are the 4 KiB distributor and 8 KiB CPU
  interface the GIC decodes (and with EL2 the 8 KiB virtual interface
  frames, as the GIC-400 has them, DDI 0471B), where QEMU's give the
  64 KiB windows `virt` reserves. A hypervisor built for 64 KiB pages
  cannot map an 8 KiB `GICV_*` frame into its guest page for page, so
  Linux's KVM then traps the guest's CPU interface accesses ("GICV region
  size/alignment is unsafe, using trapping"), as it does on hardware
  whose GIC is not aliased across 64 KiB; with 4 KiB pages it maps the
  frame. `/psci`
  names PSCI 1.0 and 0.2, whose callers find every function by its
  standard identifier (DEN 0022); QEMU's adds the 0.1 binding and its
  function identifiers, so a guest that knows the 0.1 binding alone
  (Linux before 3.16) finds PSCI on QEMU and not here, in the board's tree
  or in a host's, whose `/psci` node is replaced by the board's. `dma-coherent` is on each device that makes DMA,
  the property being a device's (Devicetree Specification 2.3), not on
  the root as well. The `pmu` node names the Cortex-A57's own PMU,
  `arm,cortex-a57-pmu`, which the PMU binding lists, where QEMU names every
  processor's `arm,armv8-pmuv3`; the Armv9.2 model, a software model with
  no entry of its own, takes that generic one. The PCIe host names the
  GICv2m frame as its `msi-parent`, the frame taking no MSI specifier;
  QEMU's gives it an `msi-map` entry whose specifier cell the frame does
  not have.
- **virtio DMA reaches RAM only.** QEMU's `dma_memory_map` maps a ring or
  buffer wherever the address space has a region (`address_space_map`):
  RAM and, for reading, a flash bank in read-array mode directly, anything
  else through its bounce buffer. This board's virtio devices reach guest
  RAM only, so a ring or buffer anywhere else breaks the transfer, as one in
  an unassigned hole does in QEMU. Linux and EDK2 place their rings and
  buffers in RAM.
- **Every virtio queue has 256 entries.** virtio 1.2 leaves a queue's size
  to the device, a power of two of at most 32768 for a split queue (2.7),
  which the driver reads in `QueueNumMax` as the most it may use (Table
  4.1, 4.2.3.2).
  The board gives every queue of its devices 256 (`queue::QUEUE_SIZE`): a
  descriptor table, 16 bytes an entry, of one 4 KiB page, the smallest
  translation granule every processor here has and UEFI's page, with the available
  and used rings (518 and 2054 bytes) fitting a second. The devices take
  and complete every chain when the driver notifies them, so a queue's
  depth costs a device nothing; it bounds what a driver makes available
  between notifications and, through `seg_max`, the queue less a
  request's header and status (2.7.5.3.1: no chain longer than the
  queue), a virtio-blk request's data buffers, 254. Linux and EDK2
  (`VirtioRingInit`) allocate the whole `QueueNumMax`, three 4 KiB pages a
  queue with the used ring on a page of its own; Linux's virtio-blk takes
  the queue size as its queue depth, and its virtio-input fills 64 event
  buffers whatever the queue (`struct virtio_input`'s `evts`), a guest's
  own limit. QEMU gives its 2D virtio-gpu a control queue of 64 and a
  cursor queue of 16 (`virtio_gpu_base_device_realize`), its virtio-input
  event and status queues 64 each (`virtio_input_device_realize`), and its
  virtio-blk the 256 of its `queue-size` default, which the page gives
  here too.
- **The board names itself.** QEMU's `virt` tells a guest it is QEMU: its
  device tree's `/model` and root `compatible` are `linux,dummy-virt`, it
  has a platform bus `qemu,platform`, every virtio-mmio transport's
  `VendorID` reads "QEMU", the virtio keyboard and tablet are "QEMU Virtio
  Keyboard" and "QEMU Virtio Tablet" of USB vendor 0x0627, products 1 and
  3 of QEMU's numbering, and the
  display's EDID is vendor "RHT", model 0x1234, "QEMU Monitor", made in
  week 42 of 2014, and the GICv2m frame's `MSI_IIDR` reads QEMU's product
  ID, 'Q' (`PRODUCT_ID_QEMU`). Software keys on them: systemd's `detect_vm_device_tree`
  reports QEMU for a node named `fw-cfg` and a generic virtual machine for
  `linux,dummy-virt`. This board gives its own instead, a host's
  `BoardIdentity` or by default `BoardIdentity::RUSTY_BOX` (`src/identity.rs`):
  model "Rusty Box virt", compatible `rusty-box,virt` (dt-schema's pattern
  for a `compatible` string), `VendorID` "RBOX", "Rusty Box Virtio
  Keyboard" and "Rusty Box Virtio Tablet" of vendor 0x5242 on
  `BUS_VIRTUAL`, which has no vendor registry, products 1 and 2 at version
  1, and a display of PNP ID
  "RBX", which the UEFI Forum's registry assigns to no one (its export of
  2026-10-31), product 1, named "Rusty Box", made in 2026 with no week
  given; a GICv2m `MSI_IIDR` of 0, which names no product and no
  implementer, with the architecture version 0, GICv2m v0 (Arm Base
  System Architecture 1.0C, DEN0094C, I.9.3). That section asks for the
  implementer's JEP106 code, which this project does not hold, so the
  field reads 0, as QEMU's does; a host with a code gives it, and its
  product and revision, through `IidrFields`. The GIC's identification
  registers are the identity's too (`GicIdentity`): the distributor's
  `GICD_IIDR` (IHI 0048B §4.3.3), and the CPU interface's `GICC_IIDR` and
  with EL2 `GICV_IIDR`, which reads the same (§4.4.15, §5.5.13; the
  GIC-400 gives the two one value, DDI 0471B Tables 3-6 and 3-10). QEMU's
  (`gic_dist_readb`, `gic_cpu_read`) read Arm's JEP106 code, 0x43B, saying
  Arm made the GIC, where this board's read no product and no
  implementer by default, `GICD_IIDR` 0 and `GICC_IIDR` 0x0002_0000 with
  its architecture version 2, and a host's code, product, variant and
  revision through `GicDistIidr` and `GicCpuIidr`. The GIC's peripheral ID
  registers stay Arm's: "despite their names", they "refer only to the
  architecture of the implementation" (§4.3.18), Arm's GICv2. Linux's GIC
  driver keys on Arm's code in `GICC_IIDR` to recognise a GIC-400 behind
  a CPU interface a device tree gives as 4 KiB (`gic_check_gicv2`); this
  board's tree gives 8 KiB, the region the CPU interface decodes. The
  flash devices' manufacturer and device codes are the identity's
  (`FlashIdentity`), by default no manufacturer and no part, where QEMU's
  give Intel's ("The flash names no manufacturer", below). And no device tree
  has QEMU's platform bus, on which the board puts no device. A tree the
  host gives is named the same way: QEMU's
  `-dtb` keeps a tree's own names, but a tree dumped from QEMU would then
  tell the guest it is QEMU's `virt` (Linux's binding for
  `linux,dummy-virt` is titled "QEMU virt machine"), so a host names the
  board through its `BoardIdentity`. So a guest that tells a virtual
  machine from the device tree, as systemd's `detect_vm_device_tree` does
  (`fw-cfg`, `linux,dummy-virt` or a `/hypervisor` node), finds none here,
  where on QEMU it finds QEMU by `fw-cfg`; what systemd's SMBIOS check
  finds is the firmware's to say, and a kernel booted directly has no
  SMBIOS tables. Linux and
  EDK2 key on none of these: Alpine boots to its shell and EDK2 to its
  UEFI shell with them. `tests/identity.rs` checks what a guest reads.
- **Names as their specifications hold them.** QEMU's keyboard and tablet
  count a NUL in their name's size (`sizeof` of the string in
  `virtio-input-hid.c`), where virtio 1.2 (5.8.4) says a string has no NUL
  terminator (its bug entry 36); this board sizes a name to its length, up
  to the string's 128 bytes. QEMU's EDID generator (`edid_desc_text`) keeps 12 characters
  of a display's name and always ends them with a line feed; VESA E-EDID
  1.4 (3.10.3.4) allows 13, a line feed ending only a shorter name, and
  this board writes up to 13 so. EDID 1.4 reserves the year bytes 00h to
  0Fh (3.4.4, Table 3.8), so a display's year is 2006 to 2245, and a
  display gives either the year it was made, with its week where stated,
  or a model year (the week byte FFh), as `EdidDate` says. A guest's
  driver may keep less of a name than its field holds (Linux's virtio
  input driver keeps 64 bytes); that is the guest's to decide, and the
  board does not narrow its names to suit one guest.
- **The display's EDID is laid out by VESA's documents.** QEMU's
  generator (`qemu_edid_generate` in `hw/display/edid-generate.c`, with
  the inputs `virtio_gpu_base_generate_edid` gives it) fills what VESA
  E-EDID 1.4 leaves to a display's maker with choices of its own, and in
  places breaks E-EDID; this board's (`src/devices/virtio/edid.rs`) takes
  each such value from a published standard instead:
  - Rate: QEMU's preferred mode is at 75 Hz; every mode here is at
    60 Hz, the base video mode's rate (E-EDID 3.8) and the one VESA CVT
    1.2 defines reduced blanking for.
  - Preferred timing: QEMU blanks a share of each direction of its own
    (`generate_timings`: 35% of the width, 3.5% of the height) with both
    syncs negative; here it is CVT 1.2 reduced blanking (160 pixels, a
    positive horizontal sync, at least 460 µs of vertical blank, a
    negative vertical sync of 4 to 10 lines by the aspect ratio, the
    clock in 0.25 MHz steps), at the display's exact width, not rounded
    down to CVT's 8-pixel cell, so the preferred mode is the display's
    size.
  - Mode list: QEMU lists the same modes whatever the display's size —
    640 by 480, 800 by 600 and 1024 by 768, eight standard timings from
    1280 by 960 to 2048 by 1152, Established Timings III from 1280 by 768
    to 1920 by 1440, and in a
    CTA-861 block television VICs at 50 Hz up to 5120 by 2160 and 3840 by
    2160 at 60 Hz. Here the list is VESA DMT's 60 Hz modes that fit
    within the display, each by its E-EDID code (Established Timings I &
    II, then III, then a standard timing), and the base video mode
    always; the display has no television timings, so there is no CTA-861
    block.
  - Density: QEMU's is 100 dpi, its sizes truncated; here it is 96 pixels
    an inch, CSS's reference pixel, each size rounded to the nearest, as
    E-EDID 3.6.2 says. A screen past 255 cm gives its aspect ratio instead
    (Table 3.12), and a display of no pixels no size.
  - Input: QEMU says DisplayPort; here no interface is named (Table 3.11,
    0000b), as a virtual display is attached through none, at the 8 bits a
    primary of every virtio-gpu format.
  - Range limits: QEMU gives 50 to 125 Hz, 30 to 160 kHz and 2550 MHz
    whatever it lists, which leaves out its own preferred mode on a
    display 2062 lines tall or more (`docs/qemu-upstream-bugs.md` entry
    56); here they are the listed modes' own bounds, the preferred mode's
    included, and there is no range descriptor where one cannot hold them
    (it is optional in EDID 1.4).
  - The first descriptor: for a display 4096 pixels or more either way, or
    whose preferred clock is 655.36 MHz or more, QEMU puts no detailed
    timing in the base block, where E-EDID 1.4 requires the preferred
    timing first (3.10.1; entry 55). Here the base block always starts
    with one: the display's own mode where a detailed timing holds it,
    else the largest listed mode, with feature bit 1 clear to say it is
    not the native one (3.6.4 note 4).
  - DisplayID: both give a mode the base block cannot hold as a DisplayID
    1.3 Type I timing. QEMU stores its pixel clock as it is, where every
    Type I field, the clock among them, is stored less one, so a guest
    reads a clock 10 kHz fast (entry 54); here it is stored less one, the
    horizontal sync positive as CVT's, and the aspect ratio coded where
    DisplayID has a code for it.
  - Extension blocks: QEMU always adds the CTA-861 block; here the only
    extension is the DisplayID block, and only when it is needed.
  - A display of no pixels (its output off): QEMU describes a display of
    1280 by 800; here the EDID lists the base video mode first, gives no
    screen size and no native mode.
  Linux's `drm_edid` reads every field either generator writes; with this
  one it finds the display's size preferred, as with QEMU's.
- **No `fw_cfg`.** QEMU's `virt` has `fw_cfg` at 0x09020000, whose
  signature reads "QEMU" and whose device tree node is `qemu,fw-cfg-mmio`;
  it gives a guest ACPI tables, SMBIOS and the boot order as files. This
  board has no such files to give, so the device's one effect on a guest
  would be QEMU's signature, and the board has no `fw_cfg`: its window is
  unassigned, and no device tree, the board's own or a host's, has an
  `fw-cfg` node. EDK2 (`ArmVirtQemu`) boots to its UEFI shell without
  one, describing the machine from the device tree, and Linux does not need
  one.
- **The PCIe bus is empty.** QEMU's `virt` puts its host bridge's own
  function at 00:00.0; this board has no function there, so configuration
  reads find nothing and the I/O and memory windows answer like an empty
  bus, all ones, with writes ignored (`src/board.rs`).
- **The PCIe host's 64-bit window is prefetchable memory.** The PCI bus
  binding marks a `ranges` entry prefetchable by phys.hi's `p` bit, bit 30,
  and leaves which windows are to the platform. Arm's Base System
  Architecture (DEN0094C, E.2) puts non-prefetchable memory below 4 GiB, as
  a PCI Type 1 header holds only 32-bit addresses for it (RPCI_MM_04,
  method 1: one to one with the bus); what a bridge forwards above 4 GiB
  is its prefetchable window. So the board's tree, on both layouts, gives
  the 32-bit window as non-prefetchable 32-bit memory (0x0200_0000) and the
  64-bit window as prefetchable 64-bit memory (0x4300_0000); the windows
  read all ones and ignore writes, so no read there has a side effect.
  QEMU's `create_pcie` gives the 64-bit window non-prefetchable
  (0x0300_0000), which Linux warns of ("Memory resource size exceeds max
  for 32 bits", `pci_parse_request_of_pci_ranges` in `drivers/pci/of.c`,
  for a non-prefetchable window of 4 GiB or more). Stock EDK2's
  `FdtPciHostBridgeLib` (`ArmVirtQemu`) reads only `ranges` entries
  whose `p` bit is clear (`DTB_PCI_HOST_RANGE_TYPEMASK` holds it), so it
  passes over the 64-bit window and gives its PCI bus the 32-bit one
  alone; the board's bus has no function for it to place, and Linux takes
  both windows from the tree and assigns them itself.
- **The flash names no manufacturer, and its query table describes the
  board's device.** QEMU's `virt` flash is `pflash_cfi01` with Intel's
  JEP106 manufacturer code, 0x89, device code 0x18, and a CFI table of
  QEMU's values. A JEP106 code names a company, and JEP106 has none a
  maker may take without being assigned it; but every code carries odd
  parity in bit 7, so 0x00 is no company's. The board's flash devices give the
  identity's codes (`BoardIdentity`'s `flash`, `FlashIdentity`),
  by default manufacturer 0x00 and device 0x0000, as the GIC's and the
  GICv2m frame's identification registers name no implementer; a host with
  a JEP106 code of bank 1 gives its own. No stock firmware reads them:
  EDK2's NOR flash driver reads only Read Identifier offset 2, a block's
  lock status (`NorFlashBlockIsLocked`), and Linux's `cfi_probe` finds the
  device by the query table's "QRY", reading the codes only to apply
  quirks of named makers' parts, and a device of no maker is none of
  them. The CFI
  query table (JESD68.01; `src/devices/cfi_flash.rs`, `CfiFlash::new`)
  keeps what the specification and the command set fix — "QRY", command
  set 0001 (JEP137), no alternate command set, no chip erase (the command
  set has none), no Vpp pin — and derives the rest from the board: the
  device size, a 16-bit interface (0x0001, the devices' width, where QEMU
  says 8- or 16-bit, 0x0002), one erase block region of the banks' 256 KiB
  sectors, the extended query table straight after it, and a write buffer
  of 4 KiB a bank, the 4 KB translation granule's page. The device has no
  supply and finishes every operation within the access that starts it,
  so its Vcc range is every voltage the fields can state, 0.0 V to 9.9 V
  (QEMU's 4.5 V to 5.5 V), and each typical time is the shortest its field
  can state, 1 µs to program a word, 2 µs a buffer and 1 ms to erase a
  block, each also the longest (QEMU's 128 µs, 128 µs and 1 s, each up
  to 16 times that). The extended query table, version 1.0, reports none
  of the optional features, no suspend, no block status bits and a Vcc
  optimum of 0.0 V, and ends with its Vpp optimum; QEMU's sets one
  protection register field after it, for a device that has no protection
  register. Linux takes a typical word-program or block-erase time field of
  0 as unstated and waits by its own defaults (50 ms and 2 s), which a
  device that is ready on the first status read never reaches.
- **A flash bank holds an image of any length.** QEMU insists on a drive
  of exactly the bank's 64 MiB. Each bank here decodes its whole window,
  as QEMU maps it (`virt_flash_map`), and reads the part an image does not
  cover as erased flash, so a firmware image needs no padding and a
  directly booted kernel no flash images at all (`src/board.rs`).
- **The PL031 reads a fixed value at power-on.** QEMU starts the RTC from
  the host's wall clock; this one reads a configured value
  (`VirtConfig::rtc_epoch_seconds`) and counts emulated seconds, so the
  guest sees the same date on every run (`src/devices/pl031.rs`).
- **The PL031 counts only once started, and `RTCCR` zeroes it after.**
  DDI 0224C (r1p3; Table B-2: every revision) has the RTC count only once
  a 1 is written to `RTCCR` bit 0, which then reads 1, and any later
  `RTCCR` write resets the RTC value to zero (§2.2.4, Table 3-5); a system
  reset clears `RTCCR`, `RTCLR`, `RTCIMSC` and `RTCRIS` and keeps the value
  and `RTCMR` (§2.3.1). QEMU's RTC always counts, reads `RTCCR` as 1,
  ignores writes to it and has no reset. EDK2's `InitializePL031` writes
  `RTCCR` only while it reads 0, as DDI 0224C asks; Linux's `pl031_probe`
  writes it on every boot, so on this board, as on a PL031 that DDI 0224C
  describes, Linux finds the RTC zeroed — 1970-01-01 — once firmware has
  started it, until the time is set again
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md), "Deliberate
  QEMU omissions"). The alarm is armed for the start of the second at
  which the RTC reaches `RTCMR`; QEMU's fires a whole number of seconds
  after the write, up to a second late (entry 20). Only equality matches
  (§2.1, §2.2.6: a comparator asserts the interrupt "When both values are
  equal"): a match value the RTC reads when `RTCMR` or `RTCLR` is
  written, or when the RTC is zeroed or started, raises the raw status at
  once, and one it has already passed is reached once the 32-bit counter
  wraps, as in QEMU. The RTC reaches a match value again 2^32 seconds
  after it last did, so once `RTCICR` clears the raw status the alarm is
  armed for that; QEMU's fires once per write. `RTCPeriphID2` reads
  0x04, revision 0, which DDI 0224C
  gives r1p3; QEMU's reads 0x14. Every access reaches the register
  `PADDR[11:2]` decodes (Table A-1), where QEMU reads a byte inside a
  register as an unmapped offset.
- **The PL061 follows its lines.** DDI 0190B has interrupt detection
  watch the lines' levels, an output's included, since the pads loop it
  back (§2.2.2, Table 2-2); a level-sensitive line's raw status is its
  level, which `GPIOIC` does not clear (§2.3.4, §3.3.9); a `GPIODATA`
  write changes the register whatever the direction (§3.3.1); a line
  under hardware control ignores `GPIODIR` (§2.3.5); and an input reads
  the level outside, whatever it last drove as an output. QEMU's
  `pl061_update` watches inputs only, latches a level-sensitive line until
  `GPIOIC`, drops `GPIODATA` writes to inputs, ignores `GPIOAFSEL` and
  keeps an input's last driven value until the outside changes
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 21).
  Every access reaches the register `PADDR[11:2]` decodes
  (`src/devices/pl061.rs`).
- **virtio-blk completes a request before its notification returns.**
  QEMU hands reads, writes and flushes to its block layer, which completes
  them later, in an order and at a time the host decides; here each
  completes at the notification, or at the `Status` write of `DRIVER_OK`
  for one made available before it, with the same results, always at the
  same instruction. A host error is reported to the guest as
  `VIRTIO_BLK_S_IOERR`, QEMU's default except for a write that finds the
  host's disk full, where QEMU stops the machine until the user resumes
  it (`werror=enospc`) (`src/devices/virtio/block.rs`).
- **A virtqueue chain is read from guest memory at each use.** QEMU maps a
  chain's buffers once when it takes it; this device walks the chain again
  each time it reads or writes it. The two agree for every driver that
  keeps virtio 1.2's rule of not changing descriptors it has made
  available until the device has used them (`src/devices/virtio/queue.rs`).
- **virtio-input takes a host batch whole or waits.** When the driver has
  not queued buffers enough for a whole batch of events, QEMU's device
  drops what does not fit; this one takes none of the batch and the host
  offers it again later. The guest sees each batch whole or not at all
  either way (`src/devices/virtio/input.rs`).
- **The virtio-mmio transport keeps virtio 1.2's register rules.** QEMU
  11.1.2's `virtio-mmio.c` and `virtio.c` depart from section 4.2 and 2 in
  ways a driver that follows the specification can see; this transport
  does as the specification says (`src/devices/virtio/mod.rs`):
  - `QueueReset` (0x0C0) resets the selected queue, whatever was
    negotiated (4.2.2.1: "The device MUST reset the queue when 1 is
    written to QueueReset", which Table 4.1 describes for a driver that
    accepted `VIRTIO_F_RING_RESET`); QEMU offers the feature and has no
    such register, so a reset it never does reads as done
    (`docs/qemu-upstream-bugs.md` entry 42).
  - A queue whose `QueueReady` is 0 is not touched ("The device MUST NOT
    access virtual queue contents when QueueReady is zero"), and no queue
    is before `DRIVER_OK` (2.1.2); QEMU keeps using the rings it was given,
    and runs a notified queue whatever the status (entry 44). What a
    driver made available before `DRIVER_OK` is taken when it sets it, and
    what a queue holds when it is made ready again after `DRIVER_OK`, as
    after a queue reset, is taken then (2.6.1.2: as at initialisation).
  - A configuration change sets only the configuration bit of
    `InterruptStatus` ("Bits which do not represent events which took place
    MUST be zero"); QEMU's `virtio_notify_config` sets the used-buffer bit
    too (entry 43).
  - `DeviceFeaturesSel` and `DriverFeaturesSel` select any word of the
    feature bits, the words past the second all zero, and `QueueSel` any
    queue, one the device does not have reading `QueueNumMax` 0 (Table
    4.1); QEMU takes every feature selector but 0 for the second word, so
    a driver writing word 2 overwrites its acceptance of `VERSION_1`, and
    ignores a queue selector past 1023, leaving the last one selected
    (entry 45). A `QueueNotify` value past 16 bits, which is no
    virtqueue index (4.2.3.3: the driver writes "the 16-bit virtqueue
    index"), notifies nothing; QEMU's `uint16_t vq_idx` keeps its low 16
    bits, taking 0x1_0001 for queue 1. The specification leaves the choice
    to the device.
  - A driver's reset, `Status` 0, forgets the features it wrote, which only
    the next negotiation would otherwise read; QEMU keeps them until a
    system reset, so a driver that then writes only `DriverFeatures` word 0
    negotiates with the last driver's word 1.
  - `SHMBase` reads all ones, as `SHMLen` does, for the shared memory
    region the device does not have (Table 4.1); QEMU's reads 0 (entry
    45).
  - A `Status` write adds bits and clears none, which the driver may not
    do (2.1.1), and `DRIVER_OK` counts only once `FEATURES_OK` holds
    (3.1.1); QEMU stores what is written. A driver accepting a feature that
    was not offered, in any word, bits past 127 included (2.2 reserves
    "128 and above"; a bit set in a word past the fourth is remembered
    until a reset even if the driver writes that word 0 again, the
    transport keeping four words, where 2.2.2 says only that the device
    "SHOULD accept any valid subset"), finds `FEATURES_OK` clear (2.2.2), so its device does
    not start; QEMU drops the feature and sets it (entry 46). QEMU's legacy
    notify-on-empty and any-layout, which it offers but keeps out of
    `DeviceFeatures`, are not offered.
  - The interrupt line follows `InterruptStatus` on a broken device as on a
    working one, and `DEVICE_NEEDS_RESET`, the device's alone to set
    (2.1), is set however far negotiation has gone (2.1.2: the device
    "SHOULD set DEVICE_NEEDS_RESET when it enters an error state") and
    stays set until a reset, a device broken before `DRIVER_OK` telling
    the driver with a configuration change when it sets it (2.1.2).
    QEMU's `virtio_error` sets the bit only once `VIRTIO_F_VERSION_1` is
    negotiated, for a legacy device's sake, so a device broken before
    `FEATURES_OK` never says so; its `virtio_notify_vector` leaves a broken
    device's line where it was, and its `Status` takes the bit as the
    driver writes it, clearing or setting it. A broken device gives no
    buffer back, so it sets no used-buffer bit; QEMU's `virtio_irq` sets
    the bit before `virtio_notify_vector` looks at whether the device is
    broken, and its virtio-input status queue notifies whether or not it
    took a report, so a notification there sets the bit with no buffer used,
    on a broken device or a working one (entry 52).
- **The split virtqueue takes every chain section 2.7 allows.** A chain of
  normal descriptors may end in one naming an indirect table
  (2.7.5.3.2: "The device MUST handle the case of zero or more normal
  chained descriptors followed by a single descriptor with
  flags&VIRTQ_DESC_F_INDIRECT"), and a buffer may have no bytes; QEMU's
  `virtqueue_split_pop` takes an indirect table only at a chain's head,
  reading a later one's table as data, and breaks the device for a buffer
  of no bytes (`docs/qemu-upstream-bugs.md` entry 47). An indirect table
  naming another, which 2.7.5.3.1 forbids the driver, breaks the device
  (`src/devices/virtio/queue.rs`).
- **A used length counts the bytes written.** virtio 1.2 2.7.8.2: "The
  device MUST write at least len bytes ... beginning at the first
  device-writable buffer". The block device writes every device-writable
  byte, zeroing those a failed read, an unsupported request or a
  `VIRTIO_BLK_T_GET_ID` into a buffer longer than 20 bytes leaves, so the
  used length, every device-writable byte, is true and takes in the status
  byte, which a driver may not read past it (2.7.8.3). Buffers that
  overlap are zeroed once over the memory they cover, so a chain naming
  the same bytes many times costs no more than the RAM it covers. virtio-input's
  status queue gives a report back with 0, nothing of a device-readable
  report being written. QEMU's `virtio_blk_req_complete` counts every
  device-writable byte though it wrote only the status, and
  `virtio_input_handle_sts` the bytes it read
  (`docs/qemu-upstream-bugs.md` entry 48; `src/devices/virtio/block.rs`,
  `input.rs`).
- **virtio-blk is virtio 1.2's.** Its configuration runs to the end of
  `unused1`, which reads 0 (5.2.5.2: "The device MUST initialize padding
  bytes unused0 and unused1 to 0"), where QEMU's ends a byte after
  `write_zeroes_may_unmap`. The cache mode is set when the driver sets
  `FEATURES_OK`, the first time it may read `writeback` (5.2.5.1): write
  through without `VIRTIO_BLK_F_FLUSH` (case 1 of 5.2.6.2, and 5.2.5.2's
  "MUST initialize writeback to 0" with `VIRTIO_BLK_F_CONFIG_WCE`),
  write-back with it but without `VIRTIO_BLK_F_CONFIG_WCE`; QEMU sets it
  at `DRIVER_OK`, and only without `VIRTIO_BLK_F_CONFIG_WCE`, so a driver
  that accepts that feature without `VIRTIO_BLK_F_FLUSH` reads 1. A
  `writeback` write takes effect only with `VIRTIO_BLK_F_CONFIG_WCE`
  negotiated. A discard or write-zeroes range with a flag it may not have
  is `VIRTIO_BLK_S_UNSUPP` whether or not its sectors are on the disk
  (5.2.6.2); QEMU answers `VIRTIO_BLK_S_IOERR` for sectors past the disk
  first. These three are `docs/qemu-upstream-bugs.md` entry 49.
  `VIRTIO_BLK_T_GET_ID` writes the identifier NUL-padded to 20
  bytes (5.2.6); QEMU writes one NUL. The request types are 1.2's
  exactly, any other `VIRTIO_BLK_S_UNSUPP`: QEMU takes legacy SCSI
  requests apart, masks the barrier and output bits off a type (so the
  legacy `VIRTIO_BLK_T_FLUSH_OUT` flushes and a read with the barrier bit
  reads), and answers zone commands, which 1.2 does not have, as a device
  that is not zoned would, a short zone report breaking the device
  (`src/devices/virtio/block.rs`).
- **virtio-blk offers no geometry and no request limit.** It does not
  offer `VIRTIO_BLK_F_GEOMETRY`, optional in 5.2.3 and meaningless to a
  virtio disk, so the configuration's `geometry` reads 0 (5.2.4: valid only
  with the feature); QEMU offers the cylinders, heads and sectors
  `hd_geometry_guess` makes up from the image's partition table or size.
  A request moves as many sectors as lie in the disk, and
  `max_discard_sectors` and `max_write_zeroes_sectors` are 0xFFFF_FFFF, the
  most their le32 fields hold; QEMU refuses a request of more than its
  block layer's `BDRV_REQUEST_MAX_SECTORS` (0x3F_FFFF sectors) and reports
  that as both limits (`src/devices/virtio/block.rs`).
- **virtio-input's configuration is `struct virtio_input_config` whole.**
  It is 136 bytes, the union's bytes past the selected entry 0, and
  `select` and `subsel` read as the driver wrote them, which it may do in
  either order (5.8.5.1). QEMU's ends after the largest entry, past which
  the transport reads all ones, and zeroes `select` and `subsel` while
  they name no entry, so a driver that writes `select`, reads, then
  writes `subsel` has `select` 0. A status-queue notification that takes
  no report interrupts nothing; QEMU's interrupts anyway
  (`src/devices/virtio/input.rs`).
- **virtio-input offers what the host can send, and nothing else.** virtio
  1.2 (5.8.4) leaves a device's event codes to it ("implementation-defined
  input event codes"); QEMU's are its own, and this board derives each set
  from its own vocabulary (`src/devices/virtio/input.rs`):
  - The keyboard's `EV_KEY` codes are the Linux code of every key the host
    can press (`rusty_box_core`'s `Key::ALL`), 117 codes up to `KEY_SEARCH`
    (217), a 28-byte bitmap; QEMU offers every code of its qcode table
    (`qemu_input_map_qcode_to_linux`), many of which no front end of this
    board can press. Every code from `KEY_ESC` to `KEY_S` is there, which udev's
    `input_id` requires of a keyboard (`ID_INPUT_KEYBOARD`). Its LEDs are
    one for each lock key among those (`LED_NUML`, `LED_CAPSL`,
    `LED_SCROLLL`, as QEMU's), and `EV_REP` is offered with no codes, as
    the device never sends a repeat and Linux repeats a held key itself.
  - The tablet's buttons are `BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`,
    `BTN_SIDE` and `BTN_EXTRA`, the host's `PointerButton::ALL`. QEMU's add
    `BTN_TOUCH` (its `keymap_button`), which this board's host has no
    button to send, and `BTN_GEAR_UP` and `BTN_GEAR_DOWN`, which QEMU sends
    as a release after each wheel detent, of a key never pressed; Linux
    drops those releases.
    Linux still calls the tablet a mouse with absolute axes (udev's
    `ID_INPUT_MOUSE`), which it decides by `BTN_LEFT` before it looks for
    `BTN_TOUCH`.
  - A turn of the wheel is one `REL_WHEEL` event of its detents and its
    sync (`Documentation/input/event-codes.rst`: the value "is the number
    of detents moved"), so a turn of any length fits two buffers; QEMU
    sends each detent as its own event, sync, gear release and sync.
  - The axes `ABS_X` and `ABS_Y` run from 0 to 0xFFFF, the whole 16-bit
    coordinate a position carries (`PointerPos::MAX`), where QEMU's run to
    0x7FFF (`INPUT_EVENT_ABS_MAX`). Fuzz, flat and resolution are 0, as
    QEMU's: a position is exact ("precise", in Linux's
    `input-programming.rst`), and is a fraction of a screen whose size in
    millimetres the device does not know. A Windows guest's virtio-win
    `vioinput` driver carries an axis in a 16-bit report field, which the
    whole 16-bit range fills and does not overflow.
  - Neither device sets an input property, as QEMU's do not: where none is
    set, Linux deduces a device's kind from its events (`event-codes.rst`,
    "Device properties"), and its guidelines set `INPUT_PROP_DIRECT` for
    touchscreens and pen tablets and `INPUT_PROP_POINTER` for touchpads and
    pen tablets, none of which a pointer with mouse buttons and absolute
    axes is.
- **virtio-gpu answers on its cursor queue, and keeps offsets whole.**
  Both cursor commands answer `VIRTIO_GPU_RESP_OK_NODATA` (5.7.6.10) to a
  chain with room for it, a command shorter than `struct
  virtio_gpu_update_cursor` `VIRTIO_GPU_RESP_ERR_INVALID_PARAMETER`, any
  other command `VIRTIO_GPU_RESP_ERR_UNSPEC`, and a scanout or resource the
  device does not have, or a cursor image not 64 by 64, the error the
  control queue gives it, each changing nothing; a move takes only its
  position ("The other fields are not used and will
  be ignored by the device"); QEMU writes no answer, takes any command but
  a move as an update, and shows or hides the cursor by a move's
  `resource_id` (`docs/qemu-upstream-bugs.md` entry 51). Linux sends its cursor commands
  with no room for an answer. `VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D` takes
  its 64-bit `offset` whole (5.7.6.8), where QEMU computes each row's
  offset in 32 bits (entry 50). `VIRTIO_GPU_CMD_GET_EDID`'s `size` is the EDID's own length,
  its base block and the extension blocks byte 7Eh counts (VESA E-EDID
  1.4), where QEMU gives the whole 1024-byte field
  (`src/devices/virtio/gpu.rs`).
- **virtio-gpu's configuration is virtio 1.2's, and its limits and error
  codes the board's own.** The configuration is `struct
  virtio_gpu_config` of §5.7.4, 16 bytes, past which the transport reads
  all ones; QEMU 11.1.2's is 20, a fifth field, `blob_alignment`, coming
  from its copy of a later Linux header, for a blob feature virtio 1.2
  does not have and QEMU's 2D device does not offer. Virtio leaves the device's limits open, so they
  are the board's: the host memory a guest makes the device hold for it,
  its resources' pixels and 16 bytes for each `struct
  virtio_gpu_mem_entry` of their backing, is at most as many bytes as the
  guest has RAM, past which a resource or an attachment is
  `VIRTIO_GPU_RESP_ERR_OUT_OF_MEMORY`, and nothing else limits a resource
  or a backing list. QEMU allows 256 MiB of pixels (`max_hostmem`), no
  resource of 4 GiB or more, and 16384 backing entries
  (`virtio_gpu_create_mapping_iov`), and a scanout rectangle no smaller
  than 16 by 16, where here one pixel is enough (§5.7.6.8 asks only that
  the resource cover it). Virtio leaves which error answers which fault
  open too; here it is the one whose name says what is wrong, so where
  QEMU picks another: a resource of no pixels is
  `VIRTIO_GPU_RESP_ERR_INVALID_PARAMETER` (QEMU's `ERR_OUT_OF_MEMORY`, its
  pixman image having no bytes to map), as are backing entries the
  request does not carry and an entry outside guest RAM or of no bytes
  (QEMU's `ERR_UNSPEC`); `VIRTIO_GPU_CMD_GET_EDID` for a scanout the
  device does not have is `VIRTIO_GPU_RESP_ERR_INVALID_SCANOUT_ID`, as
  `SET_SCANOUT` answers it (QEMU's `ERR_INVALID_PARAMETER`); and the blob
  commands, of a feature the device does not offer, are
  `VIRTIO_GPU_RESP_ERR_UNSPEC`, as every other such command is (QEMU's
  `ERR_INVALID_PARAMETER`). A driver reads `blob_alignment` only with
  its feature, and one that backs each resource with its own guest memory,
  as Linux's driver and EDK2's `VirtioGpuDxe` do, holds less than the
  budget (`src/devices/virtio/gpu.rs`).
- **The GICv2's Group 1 aliases exist.** IHI 0048B's CPU interface map
  (Table 4-2) has `GICC_AIAR`, `GICC_AEOIR` and `GICC_AHPPIR` in every
  GICv2, and with `GICC_CTLR.AckCtl` clear, the setting Arm recommends
  (§4.4.1), they are how Group 1 interrupts are acknowledged, completed and
  reported. QEMU 11.1.2 (`gic_cpu_read`, `gic_cpu_write`) reads them as zero
  and ignores writes. This model implements them as QEMU's own Non-secure
  view of `GICC_IAR`, `GICC_EOIR` and `GICC_HPPIR`: a Group 0 interrupt
  reads as 1023 and an end of interrupt naming one is ignored. The virtual
  CPU interface has them too, `GICV_AIAR`, `GICV_AEOIR` and `GICV_AHPPIR`
  (Table 5-10), and its `GICV_HPPIR` and `GICV_AHPPIR` report a software
  interrupt's `CPUID` (§5.5.7), where QEMU's vCPU has no aliases and
  reports the `VirtualID` alone (`docs/qemu-upstream-bugs.md` entry 39).
- **A GIC reads 1022 only for an interrupt it could signal.** IHI 0048B
  Tables 4-35 (`GICC_IAR`) and 5-12 (`GICV_IAR`) give 1022 for a
  highest-priority pending Group 1 interrupt while `AckCtl` is clear only
  when it is "of sufficient priority to be signaled" (note a), and 1023
  otherwise; §4.4.4's `ReadGICC_IAR` gives 1023 too while `GICC_CTLR`
  does not enable Group 1. QEMU 11.1.2's `gic_acknowledge_irq` returns
  1022 before it compares the interrupt's group priority with the running
  priority, and whatever the CPU interface enables, so a Group 1 interrupt
  behind a higher active one, or one while only Group 0 is enabled, reads
  1022; both CPU interfaces here read 1023 then
  (`docs/qemu-upstream-bugs.md` entry 38).
- **A deactivation naming a List register only pending is not counted.**
  IHI 0048B Table 5-2 counts in `GICH_HCR.EOICount` the EOIs "that do not
  have a corresponding entry in the List registers", and §5.5.14 the
  `GICV_DIR` writes whose interrupt "does not exist in the List
  registers". A List register in the pending state is such an entry, so
  an EOI or `GICV_DIR` naming one counts nothing here and leaves it
  pending (deactivating an interrupt that is not active is UNPREDICTABLE,
  §5.5.14). QEMU 11.1.2's `gic_complete_irq` and `gic_deactivate_irq`
  count it, `gic_virq_is_valid` holding only for an active List register.
  The write is outside the registers' usage constraints (an EOI must
  match the last acknowledge), so this is a choice where Arm's text
  leaves room, not a QEMU bug.
- **`GICV_IAR` reports a software List register's `CPUID` whatever its
  interrupt.** IHI 0048B §5.5.4: "If the `GICH_LRn.HW` bit is set to 0
  ... bits [12:10] of the `GICH_LRn`, that indicate the CPU ID, are
  returned in the `GICV_IAR.CPUID` field". Table 5-9 has a hypervisor
  write them as 0 for an interrupt that is not an SGI; QEMU 11.1.2's
  `gic_acknowledge_irq` returns them only for an SGI, so a hypervisor
  that sets them for another interrupt reads them in `GICV_IAR` here and
  not there.
- **`GICV_ABPR` keeps the virtual minimum.** IHI 0048B §5.5.3 sets the
  virtual CPU interface's minimum binary point from `GICH_VTR.PREbits`,
  2 for the GIC-400's five preemption bits, and `GICV_ABPR`'s one above
  it (§3.5.3), 3, so a write of less reads back 3. QEMU 11.1.2's
  `gic_cpu_write` raises a vCPU's `GICV_ABPR` only to the physical
  interface's minimum, 1, where its own `GICH_VMCR` path uses
  `GIC_VIRT_MIN_ABPR` (`docs/qemu-upstream-bugs.md` entry 41).
- **The GIC's virtual interface control registers have both bases, and
  four List registers.** IHI 0048B §5.2.7 requires a common base, which
  redirects an access to the accessing processor's registers, and a
  processor-specific base for each processor. This board decodes 8 KiB of
  `GICH`: the common base and processor 0's base 4 KiB above it (0x0800_4000
  and 0x0800_5000 on the board's own layout), as the GIC-400 places them (DDI 0471B §3.7, the CPU ID in
  address bits [11:9]); QEMU's `virt` maps only the common base. With four
  List registers (`GICH_VTR.ListRegs` 3, the GIC-400's), `GICH_LR4` and up
  are RAZ/WI, where QEMU's `gic_hyp_read`/`gic_hyp_write` keep a fifth
  (`docs/qemu-upstream-bugs.md` entry 37). The NP maintenance interrupt
  counts a List register as pending only in the pending state, not active
  and pending (Table 5-5: "no List register is in pending state"), as
  QEMU's `gic_extract_lr_info` does.
- **A hardware List register deactivates either group's interrupt.**
  IHI 0048B §5.5.5 likens the deactivation a hardware List register sends
  the distributor to a Non-secure `GICC_DIR` write, which "is ignored" for
  a Group 0 interrupt. §5.1's note says the chapter describes only a GIC
  that implements the Security Extensions too, "for which Group 0
  physical interrupts are Secure interrupts". This GIC, like QEMU's
  `virt` GIC with the Virtualization Extensions, has no Security
  Extensions: Group 0 is not Secure, no access is Non-secure, and its CPU
  interface controls both groups (§4.4.1). So the deactivation reaches the
  distributor for either group, as QEMU's does (`gic_cpu_ns_access` is
  false without the extensions). A `PhysicalID` naming an SGI or a special
  ID is UNPREDICTABLE (Table 5-9) and deactivates nothing.
- **The distributor's group enables mask before prioritisation.** IHI
  0048B leaves it to the implementation whether a group the distributor
  does not forward (§4.3.1 `GICD_CTLR.EnableGrpN`) takes part in choosing
  the highest-priority pending interrupt (`IGNORE_GROUP_ENABLE`, §3.7.1;
  §4.1.5: "Although this is IMPLEMENTATION DEFINED"). Here it does not, so
  such a group's interrupts hide nothing; QEMU 11.1.2
  (`gic_update_internal`) chooses first and checks the group after, so a
  higher-priority interrupt of a group the distributor does not forward
  blocks the other group's. The CPU interface's group enables (§4.4.1
  `GICC_CTLR.EnableGrpN`) take no part in the choice in either, as the
  pseudocode has it (§3.7.2 `UpdateExceptionState`: "masked by
  distributor enables but not cpu i/f enables"; §4.4.4 `ReadGICC_IAR`):
  an interrupt of a group the CPU interface does not signal is neither
  signalled nor acknowledged, and hides the other group's. The one
  exception is §4.1.5's: with `AckCtl` set, Group 0 disabled and Group 1
  enabled, "Group 0 interrupts are ignored and GICC_IAR behaves as
  GICC_AIAR", so there a Group 0 interrupt hides nothing; QEMU lets it
  hide the Group 1 one there too. `GICC_HPPIR` and `GICC_AHPPIR` still report it,
  the other IMPLEMENTATION DEFINED choice (`GICC_MASK_HPPIR`, §4.4.1) taken
  as the GIC-400 takes it ("even when the appropriate `GICC_CTLR.EnableGrp`
  bit is set to 0", DDI 0471B Table 3-6, note c), where QEMU's read 1023
  while the CPU interface enables neither group. The
  virtual CPU interface chooses among its List registers the same way,
  whatever `GICV_CTLR` enables (§5.4.1; DDI 0471B Table 3-10, note b for
  `GICV_HPPIR`), as QEMU's `gic_get_best_virq` does, but that QEMU's
  `GICV_HPPIR` and `GICV_AHPPIR` read 1023 while `GICV_CTLR` enables
  neither group, as its `GICC_HPPIR` does, where here they report the
  highest pending List register.
- **GIC preemption compares group priorities.** IHI 0048B §3.3.3 signals a
  pending interrupt only if its group priority is higher than the running
  priority's, both under the pending interrupt's own group's binary point
  (`GICC_BPR`, or `GICC_ABPR` for Group 1 while `CBPR` is clear), and
  §3.7.2's `UpdateExceptionState` tests `PriorityIsHigher(priority,
  GICC_RPR AND mask)`. QEMU 11.1.2 (`gic_update_internal`,
  `gic_acknowledge_irq`) compares the whole priority with the running
  priority, which differs only when the two groups' binary points differ:
  there it lets an interrupt preempt whose group priority equals the
  running priority's under its own point. Both CPU interfaces here,
  physical and virtual (`GICV_BPR`, `GICV_ABPR`, §5.5.3), compare as the
  pseudocode does (`docs/qemu-upstream-bugs.md` entry 40).
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
  the data registers keep what is written to them; an unlocked write
  leaves the save/restore fields as they are; and `MDSCR_EL1` keeps only
  the fields an ARMv8.0 processor has, its RES0 bits reading as zero.
  Two UNKNOWN cases have fixed answers: a read with RXfull clear returns
  zero, and a write while TXfull is set leaves TXfull set and both data
  registers as they were.
- **The DCC's COMMIRQ is PPI 22.** Arm `CheckForDCCInterrupts` drives
  COMMIRQ from the channel's flags and `MDCCINT_EL1`, and Arm ARM H4.6
  recommends a level-sensitive PPI for it; BSA assigns it INTID 22
  (DEN0094C RB_PPI_01), and V-BSA requires every virtual environment to
  map it to a PPI (DEN0150 RV_L1PP_00). The board drives PPI 22 with it,
  level-sensitive, sampled after every access that can change it (a write
  of `MDCCINT_EL1`, of DTRTX, of `MDSCR_EL1`'s flags under the OS Lock, a
  read that empties DTRRX), so `MDCCINT_EL1.TX` with DTRTX empty raises
  it and the first write to DTRTX takes it away. The GIC-400 has no PPI
  22, so the GIC departs from it there (the GIC entry above). QEMU 11.1.2
  connects COMMIRQ nowhere (`hw/arm/virt.c` wires the timers, the PMU and
  the GIC maintenance interrupt alone), so its `MDCCINT_EL1` raises
  nothing. The legacy COMMRX and COMMTX signals, which Arm calls optional
  and not recommended for new designs, are not modelled.
- **The PMU's PMUIRQ is PPI 23.** Arm recommends a PPI for it, ID 23 from
  GICv3 (D13.3.1), BSA assigns it INTID 23 (DEN0094C RB_PPI_01), and
  V-BSA requires a PMU of a virtual environment's processing element
  (DEN0150 RV_L1PE_02). The board drives PPI 23 with Arm
  `CheckForPMUOverflow`'s request, level-sensitive, sampled after every
  access that can change it and at the instruction a counter can next
  overflow at, which the machine runs to; the device tree's `pmu` node
  names it. The GIC-400 has no PPI 23, so the GIC departs from it there
  (the GIC entry above). QEMU 11.1.2 wires its PMU to the same PPI
  (`VIRTUAL_PMU_IRQ`).
- **The PMU's clock is the instruction clock.** Arm leaves how the cycle
  counter relates to real time IMPLEMENTATION DEFINED (D14.4.2), and
  whether it and `CPU_CYCLES` count while the processor waits in `WFI` or
  `WFE` IMPLEMENTATION SPECIFIC (D14.4). The board's processor runs one
  instruction a cycle of its clock, whose rate is the machine's
  instruction rate (`VirtConfig::ips`, 50M a second by default), and the
  clock stops while it waits (`WFI`, `WFE`, their timed forms, PSCI
  `CPU_SUSPEND`). So `PMCCNTR_EL0` and `CPU_CYCLES` advance by one for
  each instruction run, an instruction that took a synchronous exception
  in its place included; `INST_RETIRED` by one for each instruction
  executed, which leaves those out but for `SVC` and `HVC`, whose
  exceptions complete them (the at-retirement events, D14.1); and the
  generic timer's count, which goes on through a wait, by `CNTFRQ_EL0` /
  `ips` for each tick. Every cycle sends an instruction for execution, so
  `STALL_FRONTEND`, `STALL_BACKEND` and `STALL`, which count the cycles in
  which none is, count nothing. QEMU 11.1.2 counts its cycles on its
  virtual clock at a fixed 1 GHz (`target/arm/cpregs-pmu.c`,
  `ARM_CPU_FREQ`, `cycles_get_count`), whatever its processor executes,
  and counts `INST_RETIRED` only under precise icount.
- **The PMU counts only the events the model knows.** Each event
  `PMCEID0_EL0` and `PMCEID1_EL0` advertise is counted as its description
  (D14.3) says: `SW_INCR`, `INST_RETIRED`, `EXC_TAKEN` and `EXC_RETURN`
  (each at the level the exception is taken from or the return executes
  at, D13.7.3.1), `CID_WRITE_RETIRED` (`MSR`s to `CONTEXTIDR_EL1`),
  `CPU_CYCLES`, `TTBR_WRITE_RETIRED` (`MSR`s to `TTBR0_EL1` and
  `TTBR1_EL1`) and `CHAIN`, and on the Armv9.2 model the stall events,
  which never occur. The model has no cache, no TLB timing, no branch
  predictor and no bus to count, so the Cortex-A57's `PMCEID0_EL0` reads
  0x5002_0F01, its TRM's 0x7FFF_0F3F (DDI 0488H Table 11-5) less the
  cache, TLB, branch prediction, memory access, bus, memory error and
  speculation events; and the events D14.6 requires of a processor with a
  level 1 data cache (`L1D_CACHE`, `L1D_CACHE_REFILL`, with FEAT_PMUv3p4
  `L1D_CACHE_RD`, `L1D_CACHE_LMISS_RD` and `L1I_CACHE_LMISS`) or with
  FEAT_SME (`SVE_INST_RETIRED` or `SVE_INST_SPEC`, `SME_INST_RETIRED` or
  `SME_INST_SPEC`, `SME_RETIRED` or `SME_SPEC`), which the Armv9.2 model
  is, are not implemented: an event number not advertised counts
  nothing. The Armv9.2 model's `PMCEID1_EL0`, 0x1000_0018, is QEMU's too,
  both being the three stall events FEAT_PMUv3p1 and FEAT_PMUv3p4
  require. QEMU 11.1.2 implements `SW_INCR`, `INST_RETIRED` (with precise
  icount), `CPU_CYCLES` and the stall events (`pm_events`), not
  `EXC_TAKEN`, `EXC_RETURN`, `CID_WRITE_RETIRED`, `TTBR_WRITE_RETIRED` or
  `CHAIN`.
- **The Cortex-A57's PMU is its TRM's, without AArch32.** `PMCR_EL0`
  reads 0x4101_3040 out of reset, the TRM's 0x4101_3000 (Table 4-10) with
  `LC` set: with no AArch32 Arm makes `LC` RES1 (the cycle counter
  overflows at bit 63) and `D` RES0, which the TRM's processor, having
  AArch32, implements; `DP`, which Arm gives a processor only with EL3,
  with FEAT_PMUv3p1 and EL2, or with FEAT_PMUv3p7, is RES0. Where Arm
  leaves an access CONSTRAINED UNPREDICTABLE (K1.2.6.2, K1.2.6.3), the
  Cortex-A57 makes the TRM's choices (B.2.20 to B.2.25): `MDCR_EL2.HPMN`
  above six counts as six and zero as none, reading back as written; an
  event counter EL0 or EL1 cannot reach is RES0 through `PMEVCNTR<n>_EL0`,
  `PMEVTYPER<n>_EL0`, `PMXEVCNTR_EL0` and `PMXEVTYPER_EL0`, and one it does
  not have UNDEFINED through the first two. The Armv9.2 model takes Arm's
  own refinement of those accesses (`ConstrainUnpredictableProcedure`:
  UNDEFINED) and the Cortex-A57's answer for `HPMN`. Its six event
  counters are the Cortex-A57's count, the board's choice, so the two
  models' PMUs are one size.
- **The PL011 is an r1p5, BSA's Generic UART.** BSA's UART must be its
  Generic UART or a 16550 (DEN0094C RB_PER_05), and V-BSA requires that of
  a virtual environment (DEN0150 RV_L1PR_01); the Generic UART is a subset
  of the PL011 r1p5 and needs transmit and receive FIFOs of one depth and
  at least 32 entries (B.1). DDI 0183G §1.2 gives r1p5's two differences
  from r1p4: FIFOs of 32 entries (§2.2.4, §2.2.5), so the `UARTIFLS`
  trigger levels are 4, 8, 16, 24 and 28 entries, and `UARTPeriphID2`'s
  revision field 3 (Table 3-21), which EDK2's `PL011UartLib` and Linux's
  `amba-pl011` read to learn the FIFOs' depth. QEMU 11.1.2's PL011 is the
  r1p1 its `pl011_id_arm` names (revision 1), with 16-entry FIFOs
  (`PL011_FIFO_DEPTH`), the depth DDI 0183F gives every revision to r1p4,
  so it is no Generic UART.
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
- **Clearing the PL011's `FEN` flushes both FIFOs; setting it flushes
  neither.** DDI 0183F §3.3.8 (Note to Table 3-12) flushes the transmit
  FIFO by clearing `UARTLCR_H.FEN` and names no other flush; it says
  nothing of the receive FIFO, or of setting `FEN`. `FEN` enables the two
  FIFOs together (Table 3-10) and §2.4.3 turns both into holding registers
  alike, so clearing it flushes the receive FIFO too. Each holding
  register is "the bottom entry of the FIFOs" (§2.4.3, "Disabling the
  FIFOs"), so setting `FEN` keeps the character either holds as its
  FIFO's one entry, `UARTFR` reporting it at the new depth, with the
  interrupts and a running receive timeout as they were. Disabling the
  UART flushes nothing, as §3.3.8's sequence disables it and then
  flushes through `FEN` as a step of its own, and nor does a `UARTLCR_H`
  write that leaves `FEN` as it was. QEMU 11.1.2 (`pl011_write`,
  `pl011_reset_rx_fifo`, `pl011_reset_tx_fifo`) empties both FIFOs on
  every change of `FEN`, set or cleared. DDI 0183F gives no rule for the
  receive interrupts across a flush either: §3.4.2 clears `RXRIS` by
  reads and by `UARTICR`, and §3.4.4 asserts `RTRIS` only while the FIFO
  is not empty. This model's reading is that an empty FIFO is below every
  trigger level, so it clears `RXRIS` and `RTRIS` with the FIFO. QEMU
  11.1.2 leaves its interrupt state as it was, so `RXRIS` stays set with
  nothing to read.
- **A PL011 read of an empty receive FIFO returns 0.** DDI 0183F does
  not say what `UARTDR` returns with nothing received. This model returns
  0, no data and no error bits, and changes nothing: `UARTRSR` keeps the
  status of the character last read, which §3.3.2 says it reports. QEMU
  11.1.2 (`pl011_read_rxdata`) returns the FIFO slot at the read
  position, a character from up to 16 before or one a flush or reset left
  behind, and loads `UARTRSR` from its error bits.
- **The PL011's loopback is the TRM's.** `UARTCR.LBE` feeds the transmit
  line to the receive line and "the modem outputs ... through to the modem
  inputs" (Table 3-12); this model pairs each output with the input whose
  signal it carries in DCE mode (Table 2-2: `RTS` to `CTS`, `DTR` to
  `DSR`, `Out1` to `DCD`, `Out2` to `RI`), as QEMU 11.1.2's
  `pl011_loopback_mdmctrl` does. Out of loopback the inputs are the
  board's lines, which no modem drives, so they read deasserted, as at
  reset, and leaving loopback changes back each one loopback asserted,
  raising its interrupt (§3.4.1); QEMU leaves them where loopback put
  them. Loopback disconnects the receiver from the host line, whose bytes
  wait until it ends, as they wait while the receiver is off; QEMU
  (`pl011_receive`) takes them and drops them. The transmit line still
  carries what is sent: DDI 0183F says when it holds `UARTTXD` in the
  marking state (`SIREN`) and says nothing of the kind for `LBE`. A break
  holds the transmit line low (Table 3-10), so in loopback the receiver's
  input goes low when `BRK` is set or loopback is entered with `BRK` set,
  each time loading one break character, and the receiver takes no other
  until the input returns to the marking state (Table 3-2, `BE`); QEMU
  (`pl011_loopback_break`) loads one only when `BRK` is set, and goes on
  looping characters back under the break. Characters sent under a break
  still reach the host, whose line carries no break, where DDI 0183F
  leaves them open ("The FIFO integrity is not guaranteed ... after the
  BRK bit has been initiated", §3.3.7). In loopback, as everywhere, the
  characters cross in no time (the last PL011 entry).
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
  like any other. The receive timeout is the one interval the UART times;
  characters themselves cross the line in no time (the last PL011 entry).
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
  3-4); enabling transmission sends what waits, and clearing `FEN` flushes
  it (§3.3.8, Note); setting `FEN` keeps the character the holding
  register holds (the `FEN` entry above). QEMU 11.1.2 (`pl011_write_txdata`) logs such a
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
  model follows the TRM, with characters leaving in no time (below):
  while transmission is enabled each character leaves as it is written,
  so with the FIFO enabled the level never rises above one entry and
  writes never set `TXRIS`; with the FIFO disabled every write fills and
  empties the holding register and sets it; and characters held back
  while transmission is off set it when enabling transmission sends more
  than the trigger level of them. Where the TRM is silent: the reserved
  `TXIFLSEL` encodings 0b101–0b111 select the half-full level, as for
  `RXIFLSEL`; a change of `TXIFLSEL` alone moves no fill level, so changes
  nothing; and the flush a change of `FEN` makes is a fall of the level
  like any other, judged against the level of the FIFO it empties.
- **PL011 characters cross the line in no time.** DDI 0183F sends a
  character as a frame of a start bit, five to eight data bits, a parity
  bit if enabled and one or two stop bits (§2.2.6, §2.4.5), each bit one
  period of the baud rate the divisor sets (§2.4.3), and receives one the
  same way (§2.2.7). This model times no frame, whatever the divisor,
  `UARTIBRD` 0 included: the transmitter sends a character the moment it
  is written with transmission enabled, or the moment transmission is
  enabled, and the host's bytes arrive the moment the receiver has room,
  as with a QEMU character backend; each crosses as the byte written or
  sent, whatever word length, parity and stop bits `UARTLCR_H` selects.
  The receive timeout (above) is the one interval the UART times.

  A driver can see the difference in one way. While transmission is
  enabled, a write never takes the transmit FIFO above one character, so
  with the FIFO enabled no write clears `TXRIS` and no drain sets it.
  With transmission enabled, `TXRIS` is then clear only after a reset or
  after `UARTICR` clears it, and once set it stays set until one of those
  clears it. (Writes made while transmission is off can take the FIFO
  above the level and clear it too, but enabling transmission drains the
  FIFO through the level and sets it again.) A driver stalls
  only if, with the FIFO enabled and `TXRIS` clear in that way, it writes
  a burst larger than the trigger level without stopping at `TXFF` and
  then waits for `TXRIS`: on hardware the burst fills the FIFO above the
  level and its drain through the level sets `TXRIS`; here neither
  happens, and the driver waits for good. A driver that waits with
  `TXRIS` still set finds it set at once, where hardware would set it
  again once the burst had drained; and one that waits for the interrupt
  only with characters left behind a full FIFO never waits, since the
  FIFO is never full while transmission is enabled. Linux 6.18's
  `pl011_tx_chars` is of that kind: it writes until `TXFF` or until it
  has nothing left, and unmasks `TXIM` only for characters a full FIFO
  left behind. Timed transmission — characters leaving at the
  programmed baud rate on the machine's clock, `UARTFR.BUSY` set until
  the last stop bit — would remove this difference. It would also make a
  driver that waits for `BUSY` after every character, as Linux's
  `pl011_putc` does, spin for each frame's time, and a guest that
  programs no divisor unable to transmit, as on hardware.

## V-BSA conformance

Arm's Virtual Base System Architecture 1.0 (DEN0150) sets what a virtual
environment gives an operating system, mostly by naming rules of BSA 1.0C
(DEN0094C). The board on its own layout (`DeviceLayout::Board`), with
either processor, audited 2026-11-08 against its Level 1 rules:

| Rule | What it asks | Verdict |
|---|---|---|
| RV_L1PE_01 | BSA B_PE_01–08, 10, 13, 14 | Conforms: one PE (Armv8.0 Cortex-A57 or Armv9.2), FP and Advanced SIMD, the 4 KiB granule, AES, SHA-1, SHA-256 and CRC32 on both, little-endian, AArch64 EL0 and EL1, no SVE; B_PE_10 is GICv3's |
| RV_L1PE_02 | FEAT_PMUv3 and `PMCCNTR_EL0` | **Fixed**: both processors have a PMU with the cycle counter and six event counters, the Cortex-A57's PMUv3 (`ID_AA64DFR0_EL1.PMUVer` 1, DDI 0488H chapter 11) and the Armv9.2 model's FEAT_PMUv3p7 (`PMUVer` 7), which `Features.json` requires of an Armv8.7 and later processor with a PMU; their registers with `PMUSERENR_EL0`'s and `MDCR_EL2`'s traps, PMUIRQ on PPI 23 with the device tree's `pmu` node ("The PMU's PMUIRQ is PPI 23"), counting on the instruction clock ("The PMU's clock is the instruction clock") the events the model knows ("The PMU counts only the events the model knows") |
| RV_L1MM_01 | BSA B_MEM_01, 02, 04, 05, 07 | Conforms: every access completes; an unassigned address takes a synchronous External abort; every window and RAM lie within the 44-bit (52-bit) physical address space; no SMMU |
| RV_L1MM_02 | DMA reaches all Non-secure space | Conforms: the virtio devices take 64-bit addresses (virtio 1.2 4.2.2) and are coherent |
| RV_L1GI_01 | BSA B_GIC_01–03, 05 | Conforms: GICv2 with GICv2m for MSIs, one PE, 16 SGIs; B_GIC_03 is GICv3's |
| RV_L1PP_00 | BSA B_PPI_01's PPIs | **Fixed**: COMMIRQ is PPI 22, where QEMU connects it nowhere ("The DCC's COMMIRQ is PPI 22"); the timers are on BSA's 30 and 27, PMUIRQ on its 23; there is no CTI or SPE |
| RV_L1SM_01–03 | SMMU | Conforms: no SMMU, so nothing blocks a boot device |
| RV_L1TM_01–04 | Counters, at least 10 MHz, one rate, 56–64 bits, never backwards | Conforms: physical and virtual counters at 1 GHz by default (`CounterRate` refuses below 10 MHz), the virtual one the physical less `CNTVOFF_EL2`, 64 bits wide, a function of the instruction count |
| RV_L1WK_01–04 | Wake-up | Conforms: the PE's only low-power state is standby (`WFI`, and PSCI `CPU_SUSPEND` for every power state, as DEN 0022E §5.4.5 permits), which any interrupt the GIC signals ends whatever `PSTATE.A`, `I` and `F`; the interrupt stays pending in the GIC (the GIC's own priority masking, IHI 0048B §3.7, applies to its wake-up signals as well) |
| RV_L1WK_05–09 | Power state semantics | Conforms: the PE is in Run, Idle or Off; the GIC, the timers and the counter are on while it is not Off, and off with it (`CPU_OFF` and `SYSTEM_OFF` stop the machine); no tables in memory, no messaged interrupts while the GIC is off |
| RV_L1PR_01 | BSA B_PER_01–06, 09–12 | **Fixed** B_PER_05: the PL011 is an r1p5, whose 32-entry FIFOs make it BSA's Generic UART, where QEMU's r1p1 with 16-entry FIFOs is neither that nor a 16550 ("The PL011 is an r1p5"). The rest conform: its interrupt is an SPI, DMA is coherent, and there is no USB, SATA or TPM, and no PCIe device |
| RV_L1PR_02 | BSA section E but PCI_MM_02 | Conforms: ECAM over one window, no programming needed, unoccupied functions reading all ones at any width; the 32-bit window below 4 GiB at the same bus addresses; INTA–INTD on four SPIs, level-sensitive; MSIs through GICv2m; I/O space mapped one to one. The Root Port rules have no Root Port to apply to |

Level 1's recommendations (six breakpoints, two context-aware, and four
watchpoints) are met by the Cortex-A57, not by the Armv9.2 model, which
has Arm's minimum of two, one and two. Of the future levels' rules,
RV_L2PE_01 (at least two event counters beside the cycle counter) and
RV_L2PE_02 (each PE's PMU overflow on an interrupt of its own, with no
logic between) are met by both processors' six counters and PMUIRQ, PPI
23; RV_L2WD_01 is not: it asks for a watchdog, which the board does not
have.

## QEMU fingerprints left

The board never names QEMU to a guest ("The board names itself" above),
and it is to show no value a guest can read that came from QEMU's own
choices where Arm's, virtio's or VESA's documents leave the choice
open. The device tree is the board's own (`src/boot/tree.rs`). A survey
of 2026-11-05 counted 135 such values; what is left of them:

**Kept for stock firmware.** Stock EDK2 `ArmVirtQemu` is built around
these as fixed PCDs or fixed code, so changing one stops it booting; each
waits for a firmware build of the board's own:

- RAM at 0x4000_0000 and, for a firmware boot, the device tree at its
  base (`PcdSystemMemoryBase`, `PcdDeviceTreeInitialBaseAddress`,
  `PcdCPUCoresStackBase` in `ArmVirtQemu.dsc`).
- The flash: banks at 0 and 0x0400_0000 of 64 MiB, the firmware volume at
  0 with its jump at the reset vector (`ArmVirtQemu.fdf`), 256 KiB sectors
  (`FdtNorFlashQemuLib.c`, `VarStore.fdf.inc`), each bank two 16-bit
  devices side by side (`VirtNorFlashDeviceLib.h`), and its device-ID
  query reading every block unlocked, which EDK2 reads at offset 2. The
  CFI query table's geometry (each device's size, its 16-bit interface,
  the erase block region) follows from these.
- The PL011's 24 MHz `UARTCLK` (`PL011UartClkInHz`, which EDK2 works the
  baud divisor out from).
- `arm,cortex-a15-gic` as the GIC node's `compatible`, the string
  EDK2's GIC discovery looks for (`ArmVirtGicPlatformDxe` on its master
  branch, `ArmVirtGicArchLib` in releases before it). The GICv2 binding names no
  generic GICv2, only products, and every product it names has the
  Security Extensions this GIC lacks, so no string fits better.
- Not a fingerprint but a risk: EDK2 master's `QemuVirtMemInfoLib` asks
  for a `qemu,fw-cfg-mmio` node and stops without one, so a current build
  of `ArmVirtQemu` does not boot this board, which has no `fw_cfg`; the
  builds tested here (Ubuntu's, UEFI 2.70) do.

**Still to replace**, each with a value from a specification or one of
the board's own: none. Every guest-readable value the 2026-11-05 survey
found QEMU had chosen is either replaced, kept for stock firmware (above)
or kept as a host's choice (below). The PMU's values are none of QEMU's
choosing: those a guest finds on QEMU too — PMUIRQ on INTID 23,
`arm,armv8-pmuv3` for the Armv9.2 model and its `PMCEID1_EL0`
0x1000_0018 — are BSA's (RB_PPI_01), the PMU binding's, and the stall
events D14.6 requires.

**Kept as a host's choice**, each off by default, for a VM made on QEMU's
`virt` or on this board before it had its own answer:

- `TransportOrder::LastFree`, QEMU's order of filling the transports, for
  a VM whose firmware variable store names its devices where that order
  put them.
- `DeviceLayout::QemuVirt`, QEMU's device addresses (with the Cortex-A57's
  `CBAR_EL1` reading QEMU's `VIRT_CPUPERIPHS`), its 32 virtio
  transports of 512 bytes (`NUM_VIRTIO_TRANSPORTS`, `VIRT_MMIO`) and its
  power button's PL061 line, 3, for a guest booted with a device tree QEMU
  wrote or a firmware variable store whose boot entries name QEMU's device
  paths.

## Not implemented

- EL3, AArch32, EL2 unless the machine asks for it, and on the Cortex-A57
  every post-v8.0 feature (LSE atomics, PAuth, SME, …): absent from its ID
  registers, so UNDEFINED, as on a real ARMv8.0 core (the PAuth hints,
  NOPs). Of the Cortex-A57's IMPLEMENTATION DEFINED registers, those of
  EL3 (`ACTLR_EL3`, `AFSR0_EL3`, `AFSR1_EL3`, `AMAIR_EL3`) and AArch32
  go with them; its AArch64 ones are the TRM's ("The Cortex-A57's
  IMPLEMENTATION DEFINED registers are its TRM's"). With no EL3, nothing but `HCR_EL2.API` and `APK` traps a key or a
  pointer authentication instruction, and the keys reset to the board's
  own values ("The pointer authentication keys reset to the board's own
  values"), where Arm leaves them UNKNOWN.
- FEAT_SVE, and the SVE2.1 and later features (FEAT_SVE2p1, FEAT_SME2p1,
  FEAT_SME_FA64, …), on the Armv9.2 model: none is an Armv9.2
  requirement, and the SVE instructions it has are the ones FEAT_SME makes
  legal in Streaming SVE mode.
- Of the translation regime: FEAT_HAFT (the walk setting the Access flag
  of Table descriptors) and FEAT_HDBSS (a log of the pages stage 2 makes
  dirty), beside FEAT_HAF and FEAT_HAFDBS. Two invalidations go further
  than Arm requires, as it permits: the `TLBI` nXS forms (FEAT_XS) remove
  the translations of memory with the XS attribute too, which `TLBIMatch`
  lets them leave; and without EL2 a write that changes what the EL1&0
  regime's translations read of its registers — a `TTBR0_EL1` or
  `TTBR1_EL1` write switching ASIDs among them — empties the TLB, where
  the ASID tag would let it keep the translations of the ASID it leaves
  (with EL2 the entries of each set of those registers' values are kept,
  above); and likewise with FEAT_VHE a write that changes what the EL2&0
  regime's translations read — a `TTBR0_EL2` or `TTBR1_EL2` write
  switching ASIDs among them — empties the TLB.
- EL2 on the Armv9.2 model. `Features.json` (AARCHMRS 2026-09), its
  constraints evaluated over the features the Armv9.2 model has with
  `FEAT_EL2` and `FEAT_AA64EL2` added, requires of it these EL2 features
  beyond FEAT_EL2's own, each by the constraint named:
  - FEAT_VHE (`(v8Ap1 && FEAT_AA64EL2) ==> FEAT_VHE`): built, with
    FEAT_E2H0 (`HCR_EL2.E2H` writable), tested on the test processor with
    every feature.
  - FEAT_XNX (`(v8Ap2 && FEAT_EL2) ==> FEAT_XNX`): built — stage 2's
    `XN[1:0]`, EL1's and EL0's execute permission apart.
  - FEAT_S2FWB (`(v8Ap4 && FEAT_AA64EL2) ==> FEAT_S2FWB`): built —
    `HCR_EL2.FWB`, stage 2's `MemAttr` forcing write-back, and
    `CLIDR_EL1` with no cache to the PoU.
  - FEAT_EVT and FEAT_EVT2 (`(v8Ap5 && FEAT_EL2) ==> FEAT_EVT`, and
    `FEAT_EVT2` alike): built — the enhanced virtualization traps of
    `HCR_EL2` (`TOCU`, `TICAB`, `TID4`, `TTLBIS`, `TTLBOS`), with
    `ID_AA64MMFR2_EL1.EVT` 2.
  - FEAT_FGT (`(v8Ap6 && (FEAT_AA64EL2 || FEAT_AA64EL3)) ==> FEAT_FGT`,
    and `(FEAT_SME && FEAT_EL2) ==> (FEAT_FGT && FEAT_HCX)`): built — the
    fine-grained trap registers `HFGRTR_EL2`, `HFGWTR_EL2`, `HFGITR_EL2`,
    `HDFGRTR_EL2` and `HDFGWTR_EL2`, and `MDCR_EL2.TDCC`.
  - FEAT_HCX (`(v8Ap7 && FEAT_AA64EL2) ==> FEAT_HCX`, and the SME
    constraint above): built — `HCRX_EL2`, with `FnXS`, `FGTnXS` and
    `FNB`, and `SMPME` RES0 as `SMIDR_EL1.SMPS` is 0.
  - FEAT_GTG (`(v8Ap5 && FEAT_AA64EL2) ==> FEAT_GTG`), FEAT_S2TGran16K and
    FEAT_S2TGran64K (`((v8Ap5 && FEAT_AA64EL2) && FEAT_TGran16K) ==>
    FEAT_S2TGran16K`, and 64K alike): built — stage 2's granules as
    stage 1's, with FEAT_TTST's stage 2 rules, FEAT_HAF's and
    FEAT_HAFDBS's stage 2 updates and FEAT_ECV_POFF's `CNTPOFF_EL2`
    beside them.

  No other constraint requires an EL2 feature of it: FEAT_ECV and
  FEAT_IDST, which version constraints require whatever the levels, the
  model has already; FEAT_VMID16, FEAT_TWED, FEAT_NV and FEAT_NV2 are
  optional at Armv9.2 (FEAT_NV is plan item H7); and FEAT_HPMN0 is
  required from Armv8.8 (`(v8Ap8 && (FEAT_PMUv3 && FEAT_EL2)) ==>
  FEAT_HPMN0`). Every one of these is built and tested on the test
  processor with every feature; they reach a guest once the model's
  feature set lists them and the model is given EL2 (plan item F3).
- SHA-512, SHA-3, SM3 and SM4 (Armv8.2 options) on any processor: every
  QEMU CPU model that has them also has features the core does not
  implement yet (SVE), so no model to take their ID values from matches
  one this core can be.
- Floating-point exception traps: FPCR's trap-enable bits read as zero, as on
  a Cortex-A57, so exceptions only accumulate in FPSR.
- Debug: Software Step, and the breakpoints and watchpoints
  `ID_AA64DFR0_EL1` advertises — the Cortex-A57's six breakpoints, two of
  them context-aware, and four watchpoints (DDI 0488H, 4.3.19), the Armv9.2
  model's two, one and two — raise their exceptions (`src/cpu/debug.rs`);
  there is no Debug state, no halting debug and no Vector Catch (see
  "Where the references differ"). The
  Debug Communications Channel's COMMIRQ, which `MDCCINT_EL1` enables,
  drives PPI 22 (see "Where the references differ"); the legacy COMMRX and
  COMMTX signals are not modelled.
- Of the PMU: the events of what the model does not have — caches, TLB
  timing, branch prediction, a bus, speculation — and the per-instruction
  class events (`LD_RETIRED`, `BR_RETIRED` and their kind, `SVE_INST_RETIRED`,
  `SME_INST_RETIRED`), which leaves out events D14.6 requires of the
  Armv9.2 model ("The PMU counts only the events the model knows"); the
  external debug and memory-mapped interface to the PMU registers (DDI
  0488H 11.6, optional in Arm's, D13.1), with no external debugger to use
  it; and FEAT_PMUv3_TH, FEAT_PMUv3_EDGE, FEAT_PMUv3_ICNTR, FEAT_PMUv3_SS,
  FEAT_PMUv3p8, FEAT_PMUv3p9, FEAT_EBEP, FEAT_MTPMU and FEAT_HPMN0, none of
  which Armv9.2 requires.
- No data cache model, and no instruction cache a guest can see: the
  decoded traces are kept coherent with every write. Cache maintenance by
  address translates its address, with the faults Arm's `AArch64_DC` and
  `AArch64_IC` take, and has nothing else to act on; by set and way, or the
  whole instruction cache, it has nothing to act on at all. The
  Cortex-A57's `RAMINDEX` finds no cache or TLB RAM entry valid, and the
  controls of caches, prefetchers and timing in its IMPLEMENTATION DEFINED
  registers are kept and act on nothing.
- PL011 features (DDI 0183F §1.1.1) beyond its FIFOs, interrupts,
  loopback and receive timeout: DMA (§2.7; `UARTDMACR` keeps what is
  written and requests nothing); the IrDA SIR ENDEC (§2.3; `UARTCR.SIREN`
  and `SIRLP` and `UARTILPR` are kept and change nothing); hardware flow
  control (§2.6; `UARTCR.RTSEn` and `CTSEn` likewise); errors on the host
  line — it carries bytes, not frames, so no framing or parity error,
  break or false start bit ever arrives from it, and a break reaches the
  receiver only in loopback; modem status inputs outside loopback, which
  no host line drives, so they read deasserted; and the integration test registers `UARTITCR`,
  `UARTITIP`, `UARTITOP` and `UARTTDR` (chapter 4), which read as zero and
  ignore writes.
- No network device, and nothing on the PCIe bus: storage is the initramfs
  or the opt-in virtio-blk disk, and the display and input devices are the
  opt-in virtio ones.
- UEFI firmware draws nothing on the virtio display: `ArmVirtQemu`'s boot
  manager (`PlatformBootManagerLibLight`) puts only PCI displays on its
  console, as it does on QEMU, so the screen waits for the operating
  system's driver.
