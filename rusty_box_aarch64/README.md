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
  instruction that reached a device, wrote a timer register, filled or
  flushed the TLB, or wrote over cached code. Code stays coherent with
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
priority bytes, whose fetches cannot be cached, so every instruction takes
the per-instruction path, at about 570 host instructions each. That path
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
| Processor | One of two models (`VirtConfig::cpu`, `CpuModel`). `cortex-a57`, the default: Cortex-A57-shaped ARMv8.0, the whole ARMv8.0 FP and Advanced SIMD instruction set (integer and floating point, vector and scalar, saturation setting FPSR.QC), the Cortex-A57's optional Cryptographic Extension (AES, 64-bit PMULL, SHA-1, SHA-256) and CRC32, as QEMU's `cortex-a57` has them, with QEMU's 62.5 MHz system counter. `armv9.2`: an Armv9.2 processor with SME and SME2 and no SVE (the features below), Streaming SVE vectors of 128 to 512 bits, `MIDR_EL1` 0x000F_0520 (implementer 0, a software model) and the 1 GHz counter Armv8.6 and later require. Both: EL0 and EL1, and on the Cortex-A57 EL2 with `MachineBuilder::with_el2` (see below), AArch64 only, the Cortex-A57's caches as `CTR_EL0`, `CLIDR_EL1` and `CCSIDR_EL1` report them, 4 KiB-granule MMU, stage 1 and with EL2 stage 2, with a TLB holding both stages' translations, exclusives, PSCI 1.1 (DEN 0022D) over `HVC`, or over `SMC` with EL2, the conduits QEMU's `virt` provides (see "Where the references differ") |
| Interrupts | GICv2 without the Security Extensions, for one processor, as IHI 0048B specifies it, in QEMU's structure: both interrupt groups, with Group 0 on FIQ under `FIQEn`, the Group 1 aliases (`GICC_AIAR`, `GICC_AEOIR`, `GICC_AHPPIR`, `GICC_ABPR`), preemption by group priority, the running priority and active priority registers, and 4 KiB distributor and 8 KiB CPU interface regions; + GICv2m frame |
| Timers | EL1 physical and virtual generic timers; with EL2, the EL2 physical timer (`CNTHP_*`, PPI 10), `CNTVOFF_EL2` and `CNTHCTL_EL2`'s traps and event stream; with FEAT_ECV_POFF too, `CNTPOFF_EL2` (built, but no model has FEAT_ECV_POFF and EL2 yet) |
| Devices | PL011 UART (`UARTPeriphID2` revision 1, QEMU's ID, as DDI 0183F describes the revisions up to r1p4: a 16-entry receive FIFO with overrun and break status, receive interrupts at the `UARTIFLS` level, modem-status interrupts on line changes, loopback, a receiver gated by `UARTEN` and `RXE`, a transmitter gated by `UARTEN` and `TXE` whose 16-entry transmit FIFO holds what is written meanwhile, transmit interrupts as the transmit FIFO drains through the `UARTIFLS` level, and the receive timeout counted in bit periods of the programmed baud rate on the machine's clock, while characters themselves cross the line in no time; see "Where the references differ"), PL031 RTC (r1p3, as DDI 0224C describes it: stopped until `RTCCR` starts it, zeroed by any later `RTCCR` write, stopped again by a system reset), PL061 GPIO as DDI 0190B describes it (address-masked `GPIODATA`, edge and level detection on the lines' levels, outputs included) with QEMU's `gpio-key` power button on line 3, CFI flash ×2 (QEMU `pflash_cfi01` semantics, whole 64 MiB banks decoded), `fw_cfg` (traditional and DMA interfaces, no files), 32 virtio-mmio transports (QEMU's legacy layout while empty; non-legacy, as QEMU's `force-legacy=false`, once the host plugs in QEMU's 2D `virtio-gpu-device` with its EDID, `virtio-keyboard-device`, `virtio-tablet-device` or `virtio-blk-device` over a raw image in memory or a host file, which take the highest free ones), an empty PCIe ECAM/windows |
| Board | QEMU `virt` memory map and IRQ wiring; QEMU 11.1.2's device tree in `assets/` (two QEMU bugs fixed, see "Where the references differ"; its `rng-seed` and `kaslr-seed` fixed, see below), its memory node replaced by one for the machine's RAM, `/chosen` set per machine, its `/psci` node replaced by QEMU's for the conduit the firmware answers on (`fdt_add_psci_node`), and its processor's `compatible` set to the model's, where libfdt puts them, so a kernel boot's tree is QEMU's property for property; a host's own tree gets the same, every memory node and its `/psci` node replaced and `/chosen` added where it has none, as QEMU's `-dtb` does |
| Boot | UEFI firmware from flash; or a Linux `Image`, initramfs and device tree placed as QEMU's `hw/arm/boot.c` places them, entered with the register state its boot stub leaves, and the UART's console set up at 115200 baud, 8N1, as firmware leaves it (see "Where the references differ") |

Provenance: Arm's documents decide wherever they speak — the Arm ARM (DDI
0487) and its machine-readable data (AARCHMRS) for the processor, IHI 0048B
for the GIC, DDI 0183F for the PL011, DDI 0224C for the PL031, DDI 0190B for
the PL061, DEN 0022D for PSCI and DEN 0028B for the SMC Calling Convention.
QEMU's `virt` board decides what those documents
do not: the memory map, the interrupt wiring, the device tree, the choices
Arm leaves to an implementation, and the devices Arm does not document
(virtio, `fw_cfg`, the CFI flash, `gpio-key`). Each device cites its Arm
document where there is one, then the QEMU source file and symbols it was
ported from, and every departure from QEMU is listed under "Where the
references differ". The QEMU reference is release 11.1.2 and its newest
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
rule; FEAT_HPDS (TCR_EL1.HPD0 and HPD1), FEAT_TTCNP (TTBRn_EL1.CnP),
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
and FEAT_Debugv8p4 (`DebugVer`, and DBGAUTHSTATUS_EL1's always-enabled
non-invasive debug; what else they change is how breakpoints and
watchpoints match, which never fire on any processor here (see "Not
implemented"), and halting and external debug and the PMU, which this
model does not have); FEAT_AFP (FPCR.FIZ, FPCR.AH's alternate
floating-point behaviours in every scalar, Advanced SIMD and BFloat16
instruction, and FPCR.NEP's merging of scalar results into the source
register each instruction names) and FEAT_RPRES (12-bit single-precision
FRECPE and FRSQRTE under FPCR.AH);
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
mask is clear (after a deliverable physical interrupt, IRQ before FIQ
before SError: Arm leaves the order IMPLEMENTATION DEFINED), a pending one
ends `WFI` and `WFE`, a virtual SError reports `VSESR_EL2` (FEAT_RAS), and
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
`AArch64_S2Walk` define it: `VTCR_EL2`'s `T0SZ`, `SL0` and `PS` with the
4 KiB granule, which `ID_AA64MMFR0_EL1.TGran4_2` offers (2), up to 16
concatenated start tables, the configurations Arm refuses taking a level 0
Translation fault; `S2AP`, `XN` and the access flag; and `MemAttr`
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
results, each stage's permissions and memory type apart, and is emptied by
every TLB maintenance instruction. Its EL1&0 entries are tagged with the
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
walks stage 2 once for each IPA page it reads. The GIC's virtualization
extensions are not built yet (see "Not implemented").
The Armv9.2 model is refused EL2 (`BuildError::El2Unavailable`):
`Features.json` requires of an Armv9.2 processor with EL2 FEAT_VHE,
FEAT_FGT, FEAT_HCX, FEAT_EVT, FEAT_EVT2, FEAT_S2FWB and FEAT_XNX, none of
which is built yet, and an Armv9.2 processor without them is not one Arm
permits.

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
  at the register's address, DC IVAC needing write permission, a clean at
  EL0 read permission, PAN taking no part, and UCI's trap first; each later
  TLBI form UNDEFINED without its features and at EL0 and invalidating a
  stale translation with them; DSB nXS; LDLAR and STLLR as ordered
  accesses PAN applies to; the LORegion registers; and each feature's
  bits RES0 or its encodings UNDEFINED without it.
- `tests/unit/load_store.rs` checks, over a bus that logs every access,
  what Arm's `Mem` and the load and store pages decide and the vectors
  cannot show: PRFM, PRFUM and PRFM (register) checking no SP alignment
  and accessing nothing; a register offset without `option<1>` UNDEFINED
  ahead of the SIMD&FP trap; a pair of Q registers at 8 mod 16 on Device
  memory as four aligned doubleword accesses, which `SCTLR_EL1.A` faults;
  LDXP and STXP of words as one doubleword access; and with FEAT_LSE2 a
  pair of words in one 16-byte granule as one access.
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
  EL1 and EL2, in 94 to 165 states (per processor) of the registers that
  control access (everything clear; SIMD&FP,
  SVE and SME enabled; that with each bit `HCR_EL2`, `CPTR_EL2`,
  `MDCR_EL2`, `HSTR_EL2`, `CNTHCTL_EL2`, `SCTLR_EL2`, `CPACR_EL1`,
  `SCTLR_EL1`, `CNTKCTL_EL1` or `MDSCR_EL1` keeps set alone; the OS lock;
  `SP_EL0` selected; random states): the access, an UNDEFINED exception
  or a trap to a level with a class. The processor must take the same
  exception with the syndrome `AArch64_SystemAccessTrapSyndrome` gives, or
  none, and an encoding no accessor names (87,010 of them) must be
  UNDEFINED but where the Arm ARM's prose on the ID register space says
  otherwise: 12.2 million steps, 0 disagreements, about 4 s. Its table of
  IMPLEMENTATION DEFINED choices is this model's, each with its reason.
  The same file checks every EL2 register's
  fieldset: written at EL2 with all ones and all zeros, each field bit
  reads back as written, each RES0 bit as 0, each RES1 bit as 1 (115
  registers over the three processors with EL2), the three rules the data
  does not carry (CNTHP_CTL_EL2.ISTATUS read-only, TCR_EL2.DS RES0 without
  FEAT_LPA2, SMPRIMAP_EL2 RES0 when `SMIDR_EL1.SMPS` is 0) named with their
  source.
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
  agree; the generator leaves out what QEMU's Cortex-A57 has and this core
  does not (AArch32, the PMU, its IMPLEMENTATION DEFINED registers) and
  DBGPRCR_EL1, which QEMU lacks. `tests/virtualization.rs` runs the board
  with virtualization: firmware entered at EL2, the device tree's PSCI
  method, a host's tree given QEMU's `/psci` node and an empty `/chosen`
  where it has none (and `tests/unit/machine.rs` one whose `/psci` node is
  another PSCI 0.1 node, and another memory node, coming out as QEMU's own
  tree does), a hypervisor taking the EL2 timer's PPI 10 and powering off with
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
- `tests/timer_interrupts.rs`: hand-written guests drive the interrupt path
  end to end. One runs GIC + timer + `WFI` + IRQ entry/`ERET` + PSCI; its
  outcome is identical for every budget slicing, and a snapshot taken
  mid-wait restores the same future. One sends itself an SGI and enters
  the FIQ vector for Group 0 under `GICC_CTLR.FIQEn`, and the IRQ vector
  for Group 1, or for Group 0 without it, as `ISR_EL1` shows beforehand.
  One sets the GIC and the PL011 up as Linux's console does — SPI 33
  enabled, the receive interrupt at half full, `RTIM` unmasked — and
  waits in `WFI`; five characters typed at it, fewer than the level, wake
  it through the receive timeout: it takes the IRQ, acknowledges INTID
  33, finds `RTMIS` and reads the five characters, at the tick the 32 bit
  periods end, whatever the slicing. It needs every link on the way: the
  UART's deadline in the machine's next event, the deadline's expiry
  setting `RTRIS`, and the UART's line into the GIC.
- `src/devices/gicv2.rs` and `src/devices/pl011.rs` test the device models
  against IHI 0048B and DDI 0183F register by register. The GIC: both
  groups and the Group 1 aliases, with `AckCtl` set and clear; a group the
  distributor or the CPU interface leaves disabled hiding nothing;
  preemption by group priority under each group's binary point; SGIs on a
  uniprocessor; and snapshots whose SGI state disagrees, refused with
  nothing changed. The PL011: the receiver and transmitter gated by
  `UARTEN`, `RXE` and `TXE`; both FIFO interrupts as transitions through
  the `UARTIFLS` levels, with the FIFO enabled and disabled, and a
  `TXIFLSEL` write alone changing nothing; a burst longer than the FIFO,
  written as Linux writes one, going out whole; the receive timeout's 32
  bit periods at the latched divisor, whether `RTIM` or the receiver is
  on, cleared by reading the holding register with the FIFO disabled; the
  `FEN` flush; overruns, breaks (one lost to an overrun included), modem
  status interrupts and reserved bits; and snapshots, with the states
  they must refuse.
- `tests/unit/softfloat.rs`: the softfloat surface against the host's IEEE
  arithmetic in round-to-nearest, and Arm's rules — default NaN, NaN
  propagation order, FPCR.DN and FZ, tininess before rounding, saturating
  conversions — including a fused multiply-add whose addend cancels the
  product's high word exactly.
- The virtio devices are driven register by register and ring by ring, as a
  driver drives them, by a test driver laid out like Linux's
  (`tests/unit/virtio_driver.rs`): the transport's registers, feature
  negotiation, resets, interrupt and configuration-change behaviour and
  every rule whose breach QEMU reports with `virtio_error`
  (`tests/unit/virtio_transport.rs`); QEMU's keyboard and tablet
  configuration byte for byte and the events each host action becomes
  (`tests/unit/virtio_input.rs`); every 2D GPU command and its errors in
  QEMU's order, frames and the cursor as the host receives them
  (`tests/unit/virtio_gpu.rs`); the block device's configuration and
  geometry, every request type and its errors, the cache mode, read-only
  and file-backed disks and snapshots (`tests/unit/virtio_block.rs`). The
  EDID matches QEMU's own generator byte for byte at nine sizes
  (`tests/vectors/qemu_edid.txt`, from `tests/vectors/gen_qemu_edid.sh`,
  which builds `edid-generate.c` from a QEMU checkout). Alpine with the
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
  GPIO line 3, an observer's view, direct kernel placement, its device tree
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
  `MDSCR_EL1` shows both flags, its save/restore fields, the flags among
  them, take a write only while the OS Lock is locked, and its RES0 bits
  read as zero; and EL0's accesses trap under `MDSCR_EL1.TDCC` before they
  take effect.
- `tests/translation_and_firmware.rs`: an out-of-range T1SZ, a table base or
  block beyond the output size `TCR_EL1.IPS` selects, `AT` meeting an
  external abort on its walk, PSCI's target, entry-point and
  affinity-level checks and the W registers of its 32-bit calls, and a
  store-exclusive to an unmonitored address failing without a fault.
- `tests/device_regions.rs`: the GIC distributor and CPU interface answer
  in their 4 KiB and 8 KiB regions, and an access to the rest of their 64
  KiB windows is a synchronous external abort.

## Where the references differ

Arm's documents decide wherever they speak: the Arm ARM and AARCHMRS what
an instruction does, IHI 0048B the GIC, DDI 0183F the PL011, DEN 0022D and
DEN 0028B the firmware calls. QEMU 11.1.2's `virt` board and `cortex-a57`
decide what they do not: the board's layout, wiring and device tree, the
devices Arm does not document, and the choices Arm leaves to an
implementation. Every place this model departs from QEMU is listed here
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
  device tree's `d-cache-sets`. The device tree in `assets/` is the one
  QEMU 11.1.2 generates with that fixed and with no `next-level-cache` (QEMU
  writes phandle 0 when it makes no L2 node): [docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)
  has both patches. `src/boot.rs` checks the device tree against
  `CCSIDR_EL1`.
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
  feature. `nAA` is a FEAT_LSE2 field, so this model applies that rule with
  FEAT_LSE2 and the Armv8.0 one without it, as the Cortex-A57 and QEMU
  (`check_ordered_align`, alignment enforced unless the processor has
  LSE2) do: any misaligned LDAR, STLR, LDAXR or STLXR faults.
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
  `LORC_EL1` hold nothing, as QEMU's `lor_reginfo` has them; Arm's data
  gives the registers' fields, and how many LORegions a processor has is
  its own choice. LDLAR and STLLR are then the acquire and release accesses
  their pseudocode makes them (`CreateAccDescLOR`).
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
  registers in one 16-byte granule.
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
- **The stage 2 granule is offered at stage 2.** With EL2,
  `ID_AA64MMFR0_EL1.TGran4_2` reads 2 and `TGran16_2` and `TGran64_2` 1:
  FEAT_GTG, saying at stage 2 what stage 1's fields say, which Arm's data
  permits before ARMv8.5. QEMU's Cortex-A57 reads 0 in all three, each
  meaning "as stage 1"; a hypervisor reads the same granules either way.
- **Stage 2's IMPLEMENTATION DEFINED choices are stage 1's.** The
  pseudocode asks both stages the same questions: a `VTCR_EL2.T0SZ` below
  64 − PAMax (20) or above 39 takes a level 0 Translation fault ("Fault on
  TxSZ value below minimum", "... above maximum"), `TG0` naming a granule
  this processor lacks is the 4 KiB one ("TG0 encoded granule size"), the
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
  (`AArch64_S2TreatAsNormalNC`). QEMU reports stage 1's `MAIR_ELx`
  attribute as written whatever `SCTLR_ELx.C` says, which Arm permits: the
  IMPLEMENTATION DEFINED `ReportedPARAttrs` may report the descriptor's
  attribute rather than the resulting one, and this model reports the
  resulting one, as the function as Arm writes it does. Without FEAT_XS the
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
- **The device tree's seeds and L1 data cache.** The tree in `assets/` is
  QEMU 11.1.2's `dumpdtb` output with two values replaced in place. QEMU
  draws a fresh `/chosen` `rng-seed` and `kaslr-seed` on every run; this
  board's are fixed, so runs are deterministic. QEMU 11's `cortex-a57`
  (`aarch64_aa32_a57_init` in `target/arm/cpu-max.c`) makes the L1 data
  cache 4-way, where the Cortex-A57 MPCore TRM, and QEMU 8.2, make it 2-way;
  this model's `CCSIDR_EL1` is the TRM's, and the tree's `d-cache-sets` is
  `0x100` to agree with it.
- **virtio and `fw_cfg` DMA reach RAM only.** QEMU's `dma_memory_map` maps a
  ring or buffer wherever the address space has a region
  (`address_space_map`): RAM and, for reading, a flash bank in read-array
  mode directly, anything else through its bounce buffer; `fw_cfg`'s
  `dma_memory_read` and `dma_memory_write` reach the same regions. This
  board's virtio devices and `fw_cfg` reach guest RAM only, so a ring,
  buffer or `fw_cfg` descriptor anywhere else breaks the transfer, as one in
  an unassigned hole does in QEMU: an `fw_cfg` descriptor outside RAM is
  never read and gets no ERROR status, so firmware polling it waits for
  ever. Linux and EDK2 place their rings, buffers and descriptors in RAM.
- **`fw_cfg` without files.** QEMU's `virt` offers a file directory (ACPI
  tables, SMBIOS, boot order); this board offers the traditional and DMA
  interfaces as QEMU does (`FW_CFG_ID` = 3) with an empty directory, a
  configuration the interface defines, so firmware describes the machine
  from the device tree (`src/devices/fw_cfg.rs`). EDK2's `QemuFwCfgMmioPei`
  enables DMA from the device tree's 0x18-byte region alone, without
  reading `FW_CFG_ID`, so the DMA interface is required, not optional.
- **The PCIe bus is empty.** QEMU's `virt` puts its host bridge's own
  function at 00:00.0; this board has no function there, so configuration
  reads find nothing and the I/O and memory windows answer like an empty
  bus, all ones, with writes ignored (`src/board.rs`).
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
  after the write, up to a second late (entry 20). `RTCPeriphID2` reads
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
  completes at the notification, with the same results, always at the
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
  taken treats the CPU interface's enable as §4.3.1 treats the
  distributor's. It is also the rule the GICv3 architecture (IHI 0069) is
  cited for, where `ICC_HPPIR0_EL1`/`ICC_HPPIR1_EL1` and
  `ICC_IAR0_EL1`/`ICC_IAR1_EL1` consider only interrupts of a group
  enabled by `ICC_IGRPEN0_EL1`/`ICC_IGRPEN1_EL1`; that citation is from
  memory, not from IHI 0069's text, and QEMU 11.1.2's GICv3 CPU interface
  (`icc_no_enabled_hppi`) does not follow it.
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
  the data registers keep what is written to them; an unlocked write
  leaves the save/restore fields as they are; and `MDSCR_EL1` keeps only
  the fields an ARMv8.0 processor has, its RES0 bits reading as zero.
  Two UNKNOWN cases have fixed answers: a read with RXfull clear returns
  zero, and a write while TXfull is set leaves TXfull set and both data
  registers as they were.
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
- **Emptying the PL011 FIFOs through `FEN` clears the receive interrupts.**
  DDI 0183F §3.3.8 (Note to Table 3-12) flushes the transmit FIFO by
  clearing `UARTLCR_H.FEN`; it says nothing of the receive FIFO, or of
  setting `FEN`. QEMU 11.1.2 (`pl011_write`, `pl011_reset_rx_fifo`,
  `pl011_reset_tx_fifo`) empties both FIFOs on every change of `FEN`, and
  so does this model. DDI 0183F gives no rule for the receive interrupts
  across such a flush either: §3.4.2 clears `RXRIS` by reads and by
  `UARTICR`, and §3.4.4 asserts `RTRIS` only while the FIFO is not empty.
  This model's reading is that an empty FIFO is below every trigger
  level, so it clears `RXRIS` and `RTRIS` with the FIFO. QEMU 11.1.2
  leaves its interrupt state as it was, so `RXRIS` stays set with nothing
  to read.
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
  it (§3.3.8, Note). Setting `FEN` flushes it too, as QEMU's `pl011_write`
  does; DDI 0183F is silent there, and that is this model's reading (the
  `FEN` entry above). QEMU 11.1.2 (`pl011_write_txdata`) logs such a
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

## Not implemented

- EL3, AArch32, EL2 unless the machine asks for it, and on the Cortex-A57
  every post-v8.0 feature (LSE atomics, PAuth, SME, …): absent from its ID
  registers, so UNDEFINED, as on a real ARMv8.0 core (the PAuth hints,
  NOPs). With no EL3, nothing but `HCR_EL2.API` and `APK` traps a key or a
  pointer authentication instruction, and the keys reset to zero, as
  QEMU's do, where Arm leaves them UNKNOWN.
- FEAT_SVE, and the SVE2.1 and later features (FEAT_SVE2p1, FEAT_SME2p1,
  FEAT_SME_FA64, …), on the Armv9.2 model: none is an Armv9.2
  requirement, and the SVE instructions it has are the ones FEAT_SME makes
  legal in Streaming SVE mode.
- Of EL2: the GIC's virtualization extensions (no virtual CPU interface
  raises a virtual interrupt; only `HCR_EL2` does, and the device tree's
  GIC node has no GICH and GICV frames or maintenance interrupt, which
  QEMU's `virtualization=on` board gives it, so Linux's KVM finds no vGIC
  and cannot give a guest its interrupts yet); FEAT_VHE and the later EL2
  features.
- SHA-512, SHA-3, SM3 and SM4 (Armv8.2 options) on any processor: every
  QEMU CPU model that has them also has features the core does not
  implement yet (SVE), so no model to take their ID values from matches
  one this core can be.
- Floating-point exception traps: FPCR's trap-enable bits read as zero, as on
  a Cortex-A57, so exceptions only accumulate in FPSR.
- Debug: breakpoint and watchpoint registers exist (two of each, as
  advertised) but never fire; no Debug state, no PMU. The Debug
  Communications Channel's interrupt requests (COMMRX and COMMTX, which
  `MDCCINT_EL1` enables) reach no interrupt controller: where they connect
  is the system's choice, and QEMU's `virt` connects them nowhere, so
  `MDCCINT_EL1` keeps what is written and raises nothing.
- No data cache model, and no instruction cache a guest can see: the
  decoded traces are kept coherent with every write. Cache maintenance by
  address translates its address, with the faults Arm's `AArch64_DC` and
  `AArch64_IC` take, and has nothing else to act on; by set and way, or the
  whole instruction cache, it has nothing to act on at all.
- PL011 features (DDI 0183F §1.1.1) beyond its FIFOs, interrupts,
  loopback and receive timeout: DMA (§2.7; `UARTDMACR` keeps what is
  written and requests nothing); the IrDA SIR ENDEC (§2.3; `UARTCR.SIREN`
  and `SIRLP` and `UARTILPR` are kept and change nothing); hardware flow
  control (§2.6; `UARTCR.RTSEn` and `CTSEn` likewise); errors on the host
  line — it carries bytes, not frames, so no framing or parity error,
  break or false start bit ever arrives from it, and a break reaches the
  receiver only in loopback; modem status inputs outside loopback, which
  no host line drives; and the integration test registers `UARTITCR`,
  `UARTITIP`, `UARTITOP` and `UARTTDR` (chapter 4), which read as zero and
  ignore writes.
- No network device, and nothing on the PCIe bus: storage is the initramfs
  or the opt-in virtio-blk disk, and the display and input devices are the
  opt-in virtio ones.
- UEFI firmware draws nothing on the virtio display: `ArmVirtQemu`'s boot
  manager (`PlatformBootManagerLibLight`) puts only PCI displays on its
  console, as it does on QEMU, so the screen waits for the operating
  system's driver.
