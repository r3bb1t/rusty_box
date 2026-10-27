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
  the exact first tick whose count reaches `CVAL`; the PL031 RTC counts
  emulated seconds from a fixed epoch. Nothing reads the host clock.
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
  Decisions are cached per word, so straight-line code skips the decoder.
- **Floating point is software.** `src/softfloat` is rusty_box's own port of
  Berkeley SoftFloat 3e (via Bochs softfloat3e), restricted to half, single
  and double precision and given an Arm specialization: Arm's default NaN and
  NaN-propagation order (including fused multiply-add), FPCR.DN, FPCR.FZ
  input and output flushing, tininess before rounding, and saturating
  float-to-integer conversions — and FEAT_AFP's alternate behaviours under
  FPCR.AH, chosen per operation: tininess and flush to zero after rounding,
  NaNs chosen by register, a negative default NaN, Input Denormal for a used
  denormal where Arm's FPProcessDenorms says, and FPCR.FIZ's silent operand
  flushing. On top of it `softfloat/arm_ops.rs` adds what
  Berkeley has no counterpart for — fixed-point conversions, FCVTXN's
  round-to-odd, the FPCR.AHP alternative half-precision format, FMULX,
  FRECPS and FRSQRTS — and `cpu/fp_estimate.rs` the bit-exact
  FRECPE/FRSQRTE/URECPE/URSQRTE/FRECPX algorithms. Exceptions accumulate into
  FPSR; like a Cortex-A57, the core does not trap them.

## What is modelled

| Part | Model |
|---|---|
| Processor | One of two models (`VirtConfig::cpu`, `CpuModel`). `cortex-a57`, the default: Cortex-A57-shaped ARMv8.0, the whole ARMv8.0 FP and Advanced SIMD instruction set (integer and floating point, vector and scalar, saturation setting FPSR.QC), the Cortex-A57's optional Cryptographic Extension (AES, 64-bit PMULL, SHA-1, SHA-256) and CRC32, as QEMU's `cortex-a57` has them, with QEMU's 62.5 MHz system counter. `armv9.2`: an Armv9.2 processor with SME and SME2 and no SVE (the features below), Streaming SVE vectors of 128 to 512 bits, `MIDR_EL1` 0x000F_0520 (implementer 0, a software model) and the 1 GHz counter Armv8.6 and later require. Both: EL0/EL1 AArch64 only, the Cortex-A57's caches as `CTR_EL0`, `CLIDR_EL1` and `CCSIDR_EL1` report them, 4 KiB-granule stage-1 MMU with TLB, exclusives, PSCI over `HVC` as QEMU provides it |
| Interrupts | GICv2 (no Security Extensions) + GICv2m frame |
| Timers | EL1 physical and virtual generic timers |
| Devices | PL011 UART (QEMU's 16-entry receive FIFO and loopback), PL031 RTC, PL061 GPIO with QEMU's `gpio-key` power button on line 3, CFI flash ×2 (QEMU `pflash_cfi01` semantics, whole 64 MiB banks decoded), `fw_cfg` (traditional interface, no files), 32 virtio-mmio transports (QEMU's legacy layout while empty; non-legacy, as QEMU's `force-legacy=false`, once the host plugs in QEMU's 2D `virtio-gpu-device` with its EDID, `virtio-keyboard-device`, `virtio-tablet-device` or `virtio-blk-device` over a raw image in memory or a host file, which take the highest free ones), an empty PCIe ECAM/windows |
| Board | QEMU `virt` memory map and IRQ wiring; QEMU 11.1.2's device tree in `assets/` (two QEMU bugs fixed, see "Where the references differ"), its memory node and `/chosen` set per machine, and its processor's `compatible` to the model's, where libfdt's `fdt_setprop` puts them, so a kernel boot's tree is QEMU's property for property; a host's own tree gets the memory node and `/chosen`, as QEMU's `-dtb` does |
| Boot | UEFI firmware from flash; or a Linux `Image`, initramfs and device tree placed as QEMU's `hw/arm/boot.c` places them, entered with the register state its boot stub leaves |

Provenance: instruction semantics follow the Arm ARM pseudocode; every device
cites the QEMU source file and symbols it was ported from.

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
implemented"), and halting and external debug, the PMU and EL2, which this
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
LDR and STR of it and `SMCR_EL1.EZT0`'s trap, MOVT between it and a
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
  clear of) and `docs/qemu-afp-vector-fabs-fneg.patch`, the fix for vector
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
  pages' decode pseudocode makes UNDEFINED, which QEMU refuses too, and that
  the SIMD&FP and Streaming SVE traps come after decode;
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
  and ERET, legal and illegal, do to each field; SPSR_EL1 holding them only
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
  record registers UNDEFINED, DISR_EL1's bits and its snapshot, EL0 kept
  out, a synchronous external abort's syndrome the same with the feature
  as without) and DBGAUTHSTATUS_EL1 with and without FEAT_Debugv8p4;
  `isa::tests` the RAS and DebugVer fields of the ID registers.
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
  feature set holds what Arm makes each of its features imply, and every
  constraint whose identifiers decide it holds when the model's features,
  its architecture version and those it includes, and the structural
  features every processor here has (AArch64 at EL0 and EL1, a PIPT
  instruction cache, the Cryptographic Extension) hold and nothing else
  does. AARCHMRS is Arm's, under the BSD 3-clause licence, and
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
  LDAPRB and LDAPRH have one too, `Rs`, which QEMU also refuses unless it
  is 11111 (`Rt2` of CAS and CASP is not one: Arm fixes it, so
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
  editor that sets the memory node and `/chosen` walks the tree with a
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
- **No RAS error records.** With FEAT_RAS, `ERRIDR_EL1.NUM` is 0, and
  `ERRSELR_EL1` and the `ERX*` registers (those of FEAT_RASv1p1 too) are
  UNDEFINED, as QEMU's `minimal_ras_reginfo` has them; its comment states
  that with no error records they may be. Arm's register data gives these
  registers to every processor with FEAT_RAS, reading the record
  `ERRSELR_EL1.SEL` selects; the Arm ARM's rule for a processor with no
  record to select is in its prose, which this model was not checked
  against. Nothing on this machine raises an SError, so ESB has nothing to
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
  `docs/qemu-afp-vector-fabs-fneg.patch`).
- **SVE FCMLA (indexed) leaves a NaN alone under FPCR.AH.** Its rotation
  negates a Zm element with FPNeg, which returns a NaN unchanged under
  FPCR.AH = 1. QEMU's SVE form (to master at least) never passes FPCR.AH
  to its helper, so it flips the NaN's sign there, which its Advanced SIMD
  form does not: 3 of the first 1,000 floating-point vectors differed on
  it. The vectors were generated with its fix applied
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 13,
  `docs/qemu-sve-fcmla-indexed-ah.patch`).
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
  `docs/qemu-sve-fmad-size00.patch`).
- **An SME load to a vertical tile slice zeroes every inactive element.**
  LD1B to LD1Q write Zeros to each inactive element of the slice. QEMU
  (to master at least) zeroes a vertical slice's inactive elements up to
  its last active one and leaves those after it: 8 of the 80 tile load
  vectors differed on it. The vectors were generated with its fix
  applied, and `tests/unit/za.rs` checks the elements after the last
  active one ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md)
  entry 15, `docs/qemu-sme-ld1-vertical-tail.patch`).
- **MOVT between ZT0 and a general register is UNDEFINED.** Arm defines
  it only in Debug state, which this core never enters. QEMU (to master
  at least) executes it in any state. No vector covers it, and
  `tests/unit/zt0.rs` checks the UNDEFINED
  ([docs/qemu-upstream-bugs.md](../docs/qemu-upstream-bugs.md) entry 16,
  `docs/qemu-sme2-movt-debug-state.patch`).
- **RPRFM is a prefetch.** Arm's data allocates PRFM (register) with
  `Rt = 11xxx` to RPRFM (FEAT_RPRFM, Armv8.8). On a core without the feature
  the word is a PRFM with a reserved `prfop`, which the Arm ARM executes as a
  hint, as QEMU does; the decoder conformance test lists it as its one
  departure from Arm's allocation.

## Not implemented

- EL2, EL3, AArch32, and on the Cortex-A57 every post-v8.0 feature (LSE
  atomics, PAuth, SME, …): absent from its ID registers, so UNDEFINED, as
  on a real ARMv8.0 core (the PAuth hints, NOPs). With no EL2 or EL3,
  nothing traps a key or a pointer authentication instruction, and the
  keys reset to zero, as QEMU's do, where Arm leaves them UNKNOWN.
- FEAT_SVE, and the SVE2.1 and later features (FEAT_SVE2p1, FEAT_SME2p1,
  FEAT_SME_FA64, …), on the Armv9.2 model: none is an Armv9.2
  requirement, and the SVE instructions it has are the ones FEAT_SME makes
  legal in Streaming SVE mode.
- SHA-512, SHA-3, SM3 and SM4 (Armv8.2 options) on any processor: every
  QEMU CPU model that has them also has features the core does not
  implement yet (SVE), so no model to take their ID values from matches
  one this core can be.
- Floating-point exception traps: FPCR's trap-enable bits read as zero, as on
  a Cortex-A57, so exceptions only accumulate in FPSR.
- Debug: breakpoint and watchpoint registers exist (two of each, as
  advertised) but never fire; no Debug state, no PMU. The Debug
  Communications Channel has no debugger on its other side, as on QEMU:
  `MDCCSR_EL0` reads 0, `DBGDTR_EL0` and `DBGDTRRX_EL0` read 0, writes to
  `DBGDTR_EL0` and `DBGDTRTX_EL0` vanish, and `MDSCR_EL1.TDCC` traps EL0's
  accesses (`tests/unit/sysreg.rs`).
- No instruction or data cache model: cache maintenance by address
  translates its address, with the faults Arm's `AArch64_DC` and
  `AArch64_IC` take, and has nothing else to act on; by set and way, or the
  whole instruction cache, it has nothing to act on at all.
- No network device, and nothing on the PCIe bus: storage is the initramfs
  or the opt-in virtio-blk disk, and the display and input devices are the
  opt-in virtio ones.
- UEFI firmware draws nothing on the virtio display: `ArmVirtQemu`'s boot
  manager (`PlatformBootManagerLibLight`) puts only PCI displays on its
  console, as it does on QEMU, so the screen waits for the operating
  system's driver.
