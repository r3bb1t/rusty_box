# The rusty_box Safety Doctrine (R0–R9)

Ten rules govern every public API and every internal safety-relevant seam in this workspace.
They are not graded by module and they are not stylistic preferences: **violating one is a
defect.** Cite rules by id (`R0`–`R9`) in commit messages and reviews whenever a change turns
on one.

The shape of this document follows the llvmkit Type-Safety Doctrine (D1–D11), adapted for an
emulator whose non-negotiables are Bochs behavioral parity, `no_std + no_alloc` support in the
core, and machines that are `Send` by derivation. Each rule states its **enforcement**
honestly: *mechanical* means the compiler or a CI step rejects violations; *social* means
review discipline, and saying so is part of the rule.

Each rule ends with code samples, one or more blocks under each label:

- **In the tree** is code copied from this repository, under the path it names. An excerpt
  may start at the item it shows, leaving out the doc comment and attributes above it; any
  other lines left out are marked `// …`.
- **Violates** is a minimal illustration of the shape the rule forbids. It is not code from
  the tree.

Where a rule names a design the code does not have, the text marks it as a **target** and says
it is not built.

| id | rule | enforcement |
|---|---|---|
| R0 | Public returns are named types; bounds are named generics | review (no public-API diff is adopted) |
| R1 | No undefined behavior; `unsafe` only ratchets down | **mechanical** (xtask ratchets; `forbid` end state) |
| R2 | States are types, not flags | **mechanical** (fixtures) + review |
| R3 | Machine parts have no loose currency | **mechanical** (visibility + fixtures for assembly and pairing; a choke point for moved processor state) |
| R4 | Units are types | review citation |
| R5 | One choke point per hazard | compiler exhaustiveness + review |
| R6 | `Send` derives, never promised | **mechanical** (ratchet + const asserts) |
| R7 | Bochs parity provenance | social — stated honestly |
| R8 | Erasure is opt-in and named | review (no public-API diff is adopted) |
| R9 | Tests assert guest-visible properties | social + fixture registry |

---

## R0 — Public returns are named types; bounds are named generics

No bare tuples in a public return (`Resolution { width, height }`, never `(u32, u32)` — a
tuple invites the width/height swap; `TextPos { row, col }`, never an ambiguous pair). No
anonymous `impl Trait` in public signatures: return types are named (`SerialTxDrain<'a>`,
`RowChars<'d, D>`), argument bounds are named generics
(`MachineBuilder::tracer<U: Instrumentation>(self, tracer: U)`). No meaningless scalars where an
enum speaks: `StopReason` says why a batch returned, where a `bool` could only say that it did.

The timer wheel does not meet the last clause. `BxPcSystemC::register_timer`
(`rusty_box/src/pc_system.rs`) is public and takes `continuous: bool, active: bool`, and its
callers pass bare `false, false`. The target for `continuous` is `Repeat { Once, Periodic }`;
it is not built.

*Why:* names survive at call sites; anonymous shapes leak auto-traits implicitly and cannot be
stored, documented, or diffed. *Scope:* the public surface. Internal choke points may use
tuples where destructuring is the point (named exemption: `ExecCtx::slice_parts`).

**Shape stability across features:** public types are the same shape in every configuration.
A cargo feature may gate a *type's existence* (`OwnedCpus` arrives with `alloc` — additive) or
a *value's constructibility*, but never variant or field presence on a public type — no
`#[cfg]`'d enum variants, no feature-dependent match surfaces. A compile-time choice between
representations is carried by a type, never by an enum whose completeness depends on the
build: `Emulator` declares one `cpus` field of type `cpu_store::MachineCpus<T>`, an alias that
resolves to `OwnedCpus<T>` with `alloc` and to `BorrowedCpus<'static, T>` without it. The
general form is a type parameter; for guest RAM that is the target `RamStore<B: RamBytes>`,
which is not built.

**In the tree:**

```rust
// rusty_box_devices/src/display/mod.rs
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

// rusty_box/src/iodev/serial.rs
/// Draining iterator over one UART's transmitted bytes.
///
/// A named type rather than `impl Iterator` (doctrine R0): the machine's
/// serial role handle forwards this return, and an anonymous type cannot be
/// forwarded, stored, or documented. Wrapping also keeps
/// [`TX_OUTPUT_CAPACITY`] out of the public signature, so the buffer can be
/// resized without a breaking change.
pub struct SerialTxDrain<'a>(crate::ring_buffer::Drain<'a, u8, TX_OUTPUT_CAPACITY>);
```

**Violates:**

```rust
// Illustration, not code from the tree.
impl<'m, D: DisplaySource> Display<'m, D> {
    pub fn resolution(&self) -> (u32, u32) { /* … */ } // which half is the width?
}

impl Serial<'_> {
    // Cannot be named, so a caller cannot store it or forward it from its own API.
    pub fn take_output(&mut self) -> impl Iterator<Item = u8> + '_ { /* … */ }
}
```

## R1 — No undefined behavior; `unsafe` only ratchets down

Every `unsafe` block names **who owns the invariant** it relies on: in a `// SAFETY:` comment
on the block, or, where an item exists to discharge one obligation and its body is that
operation, in an `// UNSAFETY:` comment on the item (`OwnedPartition::drop`,
`Partition::drop` and `map_range` in `rusty_box_whp/src/partition.rs`). The workspace lint
table (root `Cargo.toml`) denies `unsafe_code` in every crate that inherits it. Such a crate
lifts the deny with a named `#[expect(unsafe_code, reason = "…")]`, placed on a block
(`rusty_box_whp/src/deadline.rs`), an item (`partition.rs`), a file
(`rusty_box_whp_sys/src/windows.rs`, `rusty_box_gui/src/android.rs`) or the whole crate
(`rusty_box_whp_engine/src/lib.rs`). The count of `unsafe` tokens in each scanned tree may
never increase; when work removes some, the baseline in `xtask/src/ci.rs` is tightened in the
same commit.

The end state is a target: `#![forbid(unsafe_code)]` on every library crate except the
host-FFI leaf, with no_alloc placement delegated to the vetted `static_cell` crate, so that the
one unavoidable `unsafe` lives outside this tree. Today three crates carry the `forbid`:
`rusty_box_core`, `rusty_box_decoder` and `rusty_box_devices`. `static_cell` is not a
dependency. The leaf, `rusty_box_whp_sys`, is a permanent exception and is registered below.
Four other scanned trees hold `unsafe` tokens:

- `rusty_box/src`, the machine. The crate does not inherit the workspace lint table.
- `rusty_box_whp/src`, the safe wrapper over the leaf, where the leaf's obligations are
  discharged; the leaf's registration below says what each token is.
- `rusty_box_whp_engine/src`: one block, in `map_window` (`engine.rs`), which installs the
  machine's memory into a partition. The crate lifts the workspace deny for all of itself, with
  a crate-root `#![expect(unsafe_code, reason = …)]` in `lib.rs`, and its baseline is 1.
- `rusty_box_gui/src`: two blocks, in `android.rs`, registered below.

Whether `rusty_box_whp` and `rusty_box_whp_engine` stay outside the `forbid` end state, as the
leaf does, is the owner's call and is not decided here. This document declares neither
permanent; until that call is made, their tokens are held by their ratchet baselines, not by a
registry entry.

*Enforcement (mechanical):* the `doctrine ratchets` step of `cargo xtask ci` scans eight trees,
the `src` of `rusty_box`, `rusty_box_decoder`, `rusty_box_core`, `rusty_box_devices`,
`rusty_box_whp_sys`, `rusty_box_whp`, `rusty_box_whp_engine` and `rusty_box_gui`. It fails when,
in any of them:

- the `unsafe` tokens exceed that tree's entry in `UNSAFE_TOKEN_BASELINES`. The scan counts
  every whole-word `unsafe` that comes before any `//` on a line, string literals included,
  and skips lines that begin with `//`. A rise is not impossible, only impossible quietly: ci
  passes again only when that tree's baseline is raised in the same commit, under a comment
  saying what the new tokens are and why;
- a `.unwrap()` or `.expect(…)` appears outside test code. `PRODUCTION_PANIC_BASELINE` is zero
  for every scanned tree: a library that panics on a condition it could have returned is one
  its caller cannot contain;
- more files carry a crate-inner `allow` or `expect` that switches `dead_code` off wholesale
  than `BLANKET_DEAD_CODE_BASELINES` allows. A tree with no entry there is held at zero. A
  targeted `#[allow(dead_code)]` on one item is not counted.

**In the tree:**

```rust
// rusty_box_whp/src/deadline.rs
impl Drop for DeadlineTimer {
    fn drop(&mut self) {
        if let Some(raw) = self.raw.take() {
            // SAFETY: `raw` is this value's own pair, taken here so it cannot
            // be closed twice, and no waiter can be inside a call on it — a
            // `&mut self` drop means nobody else holds a reference.
            #[expect(
                unsafe_code,
                reason = "UNSAFETY: discharging the seam's close-once obligation, which this \
                          type owns because it owns the handles"
            )]
            unsafe {
                sys::close_deadline(raw);
            }
        }
    }
}
```

The baseline entry that holds a tree's count, with the comment saying what its tokens are:

```rust
// xtask/src/ci.rs
    // The adapter between the machine and the leaf. ONE: installing the
    // machine's memory into a partition hands the hypervisor host addresses
    // …
    ("rusty_box_whp_engine/src", 1),
```

**Violates:**

```rust
// Illustration, not code from the tree.
impl Drop for DeadlineTimer {
    #[allow(unsafe_code)] // not a named `expect`: no reason, and silent once stale
    fn drop(&mut self) {
        // No SAFETY comment, and nothing owns close-once: `RawDeadline` is
        // `Copy`, so reading `self.raw` without `take()` leaves the pair behind.
        if let Some(raw) = self.raw {
            unsafe { sys::close_deadline(raw) }
        }
    }
}
// …and in xtask/src/ci.rs, ("rusty_box_whp/src", 5) edited to 6 with no comment.
```

## R2 — States are types, not flags

If a value has more than one operational state that the *API author* controls, those states
are distinct types — never an `Option<NonNull<…>>` lifecycle field, never an `is_wired()` /
`is_initialized()` predicate. A transient capability is a borrow-carrying context (`ExecCtx`);
an assembly sequence is a builder (the powered-off machine *is* `MachineBuilder`; a built
machine is always at the reset vector, as `MachineBuilder::build` documents in
`rusty_box/src/emulator/builder.rs`). States the **guest** controls at runtime (ACPI
shutdown, a triple fault) are typed *values* (`PowerState`), because a typestate the guest can
invalidate mid-instruction is a lie — the same line llvmkit D8 draws between author-controlled
transitions and witnessed runtime facts.

*Enforcement:* a violating field or predicate is a defect to delete, not a pattern to extend;
trybuild fixtures pin the shape (executing an instruction on a bare CPU without a machine
does not compile: `rusty_box/tests/compile_fail/r2_bare_cpu_cannot_execute.rs`).

**In the tree:**

```rust
// rusty_box/src/emulator/run.rs
pub enum PowerState {
    /// Executing, or halted waiting for an interrupt — either way, alive.
    Running,
    /// The guest asked to be powered off: ACPI `PM1_CNT` with `SLP_EN` and
    /// `SLP_TYP` = S5, or the port-0x8900 shutdown protocol. The CPU is
    /// perfectly healthy; it is the machine that is finished.
    PoweredOff,
    // …
    CpuShutdown,
}
```

**Violates:**

```rust
// Illustration, not code from the tree.
pub struct Machine {
    cpu: Option<NonNull<BxCpuC>>, // wired to a CPU, or not
    initialized: bool,
}

impl Machine {
    pub fn is_initialized(&self) -> bool { self.initialized }

    pub fn step(&mut self) -> Result<StopReason> {
        if !self.initialized { return Err(Error::NotInitialized); } // repeated at every entry point
        // …
    }
}
```

## R3 — Machine parts have no loose currency

CPU, memory, devices and the PC system belong to the machine that owns them, and only that
machine lends them out. The rule is that a processor executes only against the parts of its
own machine, and that the only thing pairing the two is that machine destructuring its own
`&mut self`. This is the D7 (cross-module mixing) answer for an ownership-shaped system:
llvmkit needs brand types because its ids float free as currency; rusty_box has no currency to
brand, so a single pairing choke point would make mixing unrepresentable — and the fleet case
(`Vec<Emulator<T, E>>`) keeps full static typing, which per-instance brands cannot (confirmed
against ghost-cell/generativity adoption data: no shipping crate exposes brands in a
composition API).

Assembly holds first. Outside the crate, neither an `ExecCtx` nor a `PcIo` can be built from
loose parts: `ExecCtx` is crate-private, and `PcIo`, whose fields are public because an engine
needs three of them at once, carries a private field and a `pub(crate)` constructor. Inside the
crate, `Emulator::exec_ctx`, `Emulator::run_slice` and `Emulator::processor` build them by
destructuring the machine.

The pairing holds through `Processor` (`rusty_box/src/emulator/mod.rs`), the one value outside
the crate that holds a processor together with its machine's parts. Its fields are private, and
every verb that runs the processor against the parts — `emulate_one`, `emulate_batch`,
`finish_the_instruction`, `deliver_the_trap_owed`, `pop_deliverable_vector`, `sync_io_events`,
`deliver_smi` — is a method on it that takes both from itself. The `PcIo` forms that accept a
CPU are `pub(crate)` (`rusty_box/src/emulator/io.rs`). What `Processor` lends apart —
`into_cpu`, `into_io`, `parts` — runs nothing.

A type cannot stop a processor's state from moving. `&mut BxCpuC` is public, because an engine
imports registers through it, so `mem::swap` can carry one machine's processor, cached page
offsets included, into another. The offsets are therefore checked where a processor meets
memory: `ExecCtx::new` hands the allocation to `BxCpuC::adopt_backing`
(`rusty_box/src/cpu/cpu.rs`), which drops every host-memory cache filled against a different
allocation — the same set a snapshot restore drops (R5).

*Enforcement:* mechanical. Visibility covers assembly and pairing, pinned by
`rusty_box/tests/compile_fail/r3_execctx_is_not_assemblable.rs` and
`r3_parts_run_no_foreign_processor.rs`. The choke point covers moved state, pinned by the unit
test `a_processor_assembled_with_other_memory_keeps_no_fetch_window_into_the_first`
(`rusty_box/src/cpu/exec_ctx.rs`).

**In the tree:**

```rust
// rusty_box/src/emulator/mod.rs
pub(crate) fn exec_ctx(&mut self, index: usize) -> crate::cpu::exec_ctx::ExecCtx<'_, T> {
    let Self {
        cpus,
        memory,
        devices,
        device_manager,
        pc_system,
        ..
    } = self;
    // `get_mut` borrows only the store, so memory, devices and the PC
    // system stay independently live — the disjointness `ExecCtx` rests on.
    crate::cpu::exec_ctx::ExecCtx::new(
        cpus.get_mut(index),
        PcIo::new(memory, devices, device_manager, pc_system),
    )
}
```

**Violates:**

```rust
// Illustration, not code from the tree.
pub fn exec_ctx_from_parts<'a>(
    cpu: &'a mut BxCpuC,
    memory: &'a mut BxMemC, // any memory, from any machine
    devices: &'a mut BxDevicesC,
    device_manager: &'a mut DeviceManager,
    pc_system: &'a mut BxPcSystemC,
) -> ExecCtx<'a, ()> {
    // …
}
```

## R4 — Units are types

An offset is not an address; a guest-physical address is not a linear address; a port is not a
memory address; a window offset is not a VRAM offset; a RAM offset is not an allocation
offset. The unit types in the tree:

- `RamPage` (a page named by its RAM offset) and `AllocPage` (a page named by its allocation
  offset), in `rusty_box/src/cpu/tlb.rs`, with `FetchWindow { start, len }` beside them;
- `CpuMemoryPolicy` (`rusty_box/src/memory/mod.rs`);
- `WindowOffset`, how far into a device's window an access landed
  (`rusty_box_devices/src/api.rs`);
- `VmInstant`, `VmDuration`, `HostInstant` and `ClockHz` (`rusty_box_core/src/time.rs`);
- `Ips`, the emulated instructions per second in `EmulatorConfig::ips`, defaulting to
  `Ips::BOCHS_DEFAULT` = 50_000_000 (Bochs config.cc `cpu: ips`).

Adopted from vm-memory/vm-device: position − position yields a raw *distance* scalar, never
another position. The strongest form makes a conversion function the *only* path between unit
domains, so an unconverted access is a compile error, not a review catch.

Target design, not built: the port space gets its own `u16`-backed type (a port is a bare
`u16` today, as in `PioDevice::pio_read(&mut self, port: u16, …)`), and SVGA banking becomes
`MemoryWindow::translate(WindowOffset) -> Option<VramOffset>`. Neither a port type nor
`MemoryWindow` nor `VramOffset` exists in the tree.

**RAM offsets and allocation offsets** are the pair easiest to mix up, because both are
`usize` byte offsets and they are equal whenever residency is full.

- A **RAM offset** counts from the start of guest RAM. `bx_guest_ram_span`
  (`rusty_box/src/memory/memory_rusty_box.rs`) takes a guest-physical address (`u64`) and
  returns a span of RAM offsets (`Option<Range<usize>>`), skipping the PCI hole.
- An **allocation offset** counts from the start of the one allocation that holds guest RAM,
  the ROM image and the bogus page. When residency is partial a guest block may sit in a
  relocated slot, so a RAM offset reaches the allocation only through the residency map.
  `BxMemC::host_mem_range` (`rusty_box/src/memory/misc_mem.rs`) takes a guest-physical address
  (`BxPhyAddress`) and returns an allocation range (`Result<Option<Range<usize>>>`, `None` when
  the access has no direct mapping). For guest RAM it calls `BxMemC::resident_ram_range`,
  which passes the RAM offset from `bx_guest_ram_span` to `BxMemoryStubC::resident_slot_range`
  (`rusty_box/src/memory/memory_stub.rs`): a RAM offset in, as `usize`, and an allocation range
  out, as `Range<usize>`.

No type in those signatures stops a caller computing an allocation offset by adding
`vector_offset` to a RAM offset instead; only review does. The TLBs are where the tree types
the distinction: `BxCpuC::dtlb` is a `Tlb<RamPage, _>` and `BxCpuC::itlb` a `Tlb<AllocPage, _>`
(`rusty_box/src/cpu/cpu.rs`), and each page type is built only by its own constructor. Other
cached offsets are bare `usize`: `FetchWindow::start` (`rusty_box/src/cpu/tlb.rs`) and
`BxCpuC::vmcb_host_offset` (`rusty_box/src/cpu/cpu.rs`).

*Enforcement:* review citation; the types exist and new raw-scalar boundary crossings are
defects.

**In the tree:**

```rust
// rusty_box/src/cpu/tlb.rs
pub(crate) struct RamPage(core::num::NonZeroUsize);

// …
/// Deliberately a different type from [`RamPage`], measured from a different
/// base. Instruction fetch also runs out of the ROM image and the bogus page,
/// which live in the same allocation as guest RAM but *past* it, and out of
/// relocated RAM slots when residency is partial — none of which identity
/// guest RAM can name. The two bases happen to be equal whenever residency is
/// full, so nothing but the type system would catch a mix-up: it would pass
/// every test and corrupt every ROM fetch the moment host memory is smaller
/// than guest memory.
// …
pub(crate) struct AllocPage(core::num::NonZeroUsize);
```

The data TLB's page is built from a RAM offset, and the instruction TLB's from the start of the
allocation range `host_mem_range` returned:

```rust
// rusty_box/src/cpu/paging.rs
bx_guest_ram_span(a20_ppf, 0x1000, host_len)
    .and_then(|span| super::tlb::RamPage::from_ram_offset(span.start))

// rusty_box/src/cpu/cpu.rs
let host_page = super::tlb::AllocPage::from_alloc_offset(offset);
```

**Violates:**

```rust
// Illustration, not code from the tree. A RAM offset becomes an allocation
// offset by addition: both are `usize`, so nothing records the change of unit,
// and the sum is right only while residency is full.
fn read_ram_dword_le(stub: &mut BxMemoryStubC, gpa: BxPhyAddress) -> Option<u32> {
    let ram_offset = bx_guest_ram_span(gpa, 4, stub.guest_len())?.start;
    let alloc_offset = stub.vector_offset.checked_add(ram_offset)?; // skips the residency map
    let bytes = stub.actual_vector_mut().get(alloc_offset..alloc_offset + 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}
```

## R5 — One choke point per hazard

Every guarded hazard has exactly one code path through its guard: `ExecCtx` and `PcIo`
assembly, ISA gating (applied at icache fill), SMC stamping, and — as a target, with R4's
`MemoryWindow` — banking translation. Adding a second path to a choke-pointed hazard is a
defect even if it is correct today. Closed dispatch sets are exhaustive `match`es over enums
*without* `#[non_exhaustive]`, so adding a variant breaks every site the compiler can see —
that is the feature. (Validated in production by Firecracker's bus history: their closed-enum
bus shipped for two years; the switch to `Arc<Mutex<dyn>>` was code-import convenience and
produced a use-after-detach race within a year — PR #6062.)

*Enforcement:* compiler exhaustiveness where the pattern applies; marker comments + review for
the choke points themselves.

**In the tree:**

```rust
// rusty_box/src/cpu/icache.rs
// Bochs init_FetchDecodeTables (fetchdecode32.cc) makes an opcode
// whose CPUID feature this model lacks execute as BxError. Applied
// here, once per trace fill, so the dispatch loop is untouched. The
// decoded length is preserved: Bochs's decode also succeeds, only
// the handler changes.
// …
if decode_result.is_ok() {
    let decoded = self.i_cache.mpool[current_mpindex].get_ia_opcode();
    let resolved = self.state_resolve_opcode(self.isa_resolve_opcode(decoded));
    if resolved != decoded {
        self.i_cache.mpool[current_mpindex].set_ia_opcode(resolved);
    }
}
```

For the closed-set half, `StopReason` and `PowerState` (`rusty_box/src/emulator/run.rs`) each
state in their docs that they are closed on purpose (R5).

**Violates:**

```rust
// Illustration, not code from the tree: a second ISA gate, in one handler.
pub(super) fn vpermd(&mut self, instr: &Instruction) -> super::Result<()> {
    // Correct for AVX2 today; skips the special cases `isa_resolve_opcode`
    // carries, and every other handler has to remember to repeat it.
    if !self.bx_cpuid_support_isa_extension(X86Feature::IsaAvx2) {
        return self.exception(Exception::Ud, 0);
    }
    // …
}
```

## R6 — `Send` derives, never promised

`unsafe impl Send`/`Sync` is banned. Thread safety is proven by construction — no raw-pointer
fields, no globals — and pinned by `const` assertions: `assert_send::<BxMemoryStubC>()`
(`rusty_box/src/memory/mod.rs`), `assert_send::<Emulator<()>>()`
(`rusty_box/src/emulator/mod.rs`), and one for each CPU store
(`rusty_box/src/emulator/cpu_store.rs`). A `Send` assertion over a defaulted type parameter
proves nothing about the other instantiations, so the machine's is paired with a non-vacuity
check: a generic `assert_send::<Emulator<T>>()` over any `Send` tracer, so the property cannot
rest on the `()` default. The assertion holds in every build, the no-alloc one included: that
machine borrows its CPUs exclusively (`BorrowedCpus<'static, T>`), and an exclusive borrow of
a `Send` type is itself `Send`.

*Enforcement (mechanical):* the `doctrine ratchets` ci step counts `unsafe impl … Send/Sync`
lines in `rusty_box/src` against a baseline of 0; in the other scanned trees such a line is
caught by their `unsafe` token baselines. `cargo-semver-checks`' `auto_trait_impl_removed` lint
would guard regressions and is not adopted. The const asserts are compile-time.

**In the tree:**

```rust
// rusty_box/src/emulator/mod.rs
const _: () = {
    const fn assert_send<M: Send>() {}
    assert_send::<Emulator<()>>();
};

/// Non-vacuity for the assertion above: the machine stays `Send` for any
/// `Send` tracer, not just the `()` default.
#[allow(dead_code)]
fn assert_machine_send_for_every_send_tracer<T: Instrumentation + Send>() {
    const fn assert_send<M: Send>() {}
    assert_send::<Emulator<T>>();
}
```

**Violates:**

```rust
// Illustration, not code from the tree.
pub struct Emulator {
    cpus: [*mut BxCpuC; 253], // a raw pointer, so the type does not derive `Send`…
}

// SAFETY: the caller keeps every CPU alive and never aliases one.
unsafe impl Send for Emulator {} // …and this promises what the compiler cannot check
```

## R7 — Bochs parity provenance

Every behavioral divergence from `cpp_orig/bochs/` cites the upstream file + symbol (never a
line number — the snapshot rebases). Idiomatic-Rust restructuring that preserves observable
behavior is the *sanctioned* deviation class and still cites what it restructures. A device
with no Bochs upstream (e.g. a future AMD GPU — Bochs has none) must declare its provenance
explicitly: ported from a named source (86Box/PCem, license noted) or declared novel.
Automation readers are adapters over parity-faithful state, never parallel paths (`TextView`
reads exactly what `get_text_snapshot` reads).

Divergences that are deliberate rather than idiomatic-restructuring live in
`docs/bochs-parity-divergences.md`, one entry each, carrying the measurement or platform
constraint that justifies them. Declaring one there is what makes it a decision instead of a
defect; a code comment alone does not.

*Enforcement:* social — this is review discipline, and no checker exists; that is stated here
so nobody mistakes convention for guarantee. (llvmkit's D11 has the same honest gap.)

**In the tree:** the comment names the Bochs file and symbol, says what this port does
differently, and names the registry entry (D5) that justifies it.

```rust
// rusty_box/src/cpu/flag_ctrl_pro.rs
// TF set => the next boundary arms the single-step trap. Bochs
// flag_ctrl_pro.cc setEFlags assigns `async_event = 1`; this port
// raises the bit and keeps the rest of the word, because here the
// word also carries the scheduler boundary a device may have just
// requested — divergence D5.
if new_flags.contains(EFlags::TF) {
    self.raise_async_event();
}
```

**Violates:**

```rust
// Illustration, not code from the tree: a behaviour change with no upstream
// file or symbol, and no registry entry.
fn read(&mut self, port: u16) -> u8 {
    // No media: every register reads 0xFF.
    if self.media.is_none() {
        return 0xFF;
    }
    // …
}
```

## R8 — Erasure is opt-in and named

No `dyn` in any public signature and none on any dispatch path. Where an open set genuinely
exists, erasure lives in exactly one named place, and that place is alloc-gated: closure
storage for a user's callbacks (`Box<dyn FnMut…>`, the MMIO registry below), or an enum variant
for a user backend, which is a target (`AnyDisk::Custom(Box<dyn BlockDevice + Send>)`, not
built). A machine names its parts' types at compile time: its observer is a type parameter
(`T: Instrumentation`), and so is its execution engine (`Emulator<T, E = SoftwareEngine>`, whose
impls bound `E: SliceEngine<T>`). Traits that would be trivially erasable are kept deliberately
non-dyn-compatible (generic methods, associated consts) when no consumer needs erasure:
`SliceEngine` and `VgaExtension` are.

Target design, not built: in-tree device sets dispatch as enums (`DisplayDevice::{Vga, GeForce,
…}`); user extension composes statically through profile associated types
(`type Display = MyGpu`); optional, profile-dependent capabilities use the
`support_*() -> Option<XxxOps<'_>>` accessor convention (gdbstub's IDET idiom — the one
borrow-based capability shape the ecosystem already knows). The tree has no `DisplayDevice`
enum, no profile trait with a `type Display`, and no `…Ops<'_>` capability type. VGA cards are
selected by type parameter today (`VgaCard<E>`).

*Enforcement:* review; a public-API snapshot diff would add a mechanical check and is not
adopted.

**In the tree:**

```rust
// rusty_box/src/emulator/builder.rs
    /// Build the machine on a NAMED execution engine.
    ///
    /// `MachineBuilder::new(config).bios(rom).build_on::<WhpEngine>()` — the
    /// engine is a type argument because it is a decision about the machine
    /// rather than a piece of its configuration, and because a machine that
    /// runs its guest on hardware and one that interprets it are different
    /// types with different capabilities.
    // …
    pub fn build_on<E>(self) -> Result<alloc::boxed::Box<Emulator<T, E>>>
    where
        E: SliceEngine<T> + Default,
    {
        // …
    }
```

**Violates:**

```rust
// Illustration, not code from the tree. It does not compile: `SliceEngine`
// has associated consts, which is what keeps it from being made into an
// object, and is why the erased shape cannot creep back in.
pub struct Emulator<T: Instrumentation = ()> {
    engine: Box<dyn SliceEngine<T>>, // chosen at run time; every slice through a vtable
    // …
}
```

## R9 — Tests assert guest-visible properties, not transports

A test named after a guarantee must assert the guarantee, not the mechanism that currently
delivers it (the `…_at_issuing_epoch` lesson: it checked a request table instead of the
deadline). Type-level invariants get trybuild fixtures; runtime invariant tests get
mutation-checked once (break the invariant deliberately, watch the test fail) before they are
believed. Test provenance follows the parity rule: ported Bochs behavior cites its upstream
test or the Bochs source it exercises.

*Enforcement:* social; the compile_fail fixture registry (`rusty_box/tests/compile_fail/`),
which `cargo xtask ci` runs, is the mechanical half for type-level claims.

**In the tree:** the guest runs `mov edx, 0x21; mov al, 0xAB; out dx, al; in al, dx; hlt`
against the real PIC, and the assertion is the value the guest read back. The doc comment
cites the Bochs handlers it exercises.

```rust
// rusty_box/src/emulator/tests.rs
/// Port I/O is not feature-gated: a guest `OUT` reaches the device and the
/// following `IN` reads back what the device now holds. Bochs pic.cc
/// write_handler/read_handler for 0x21 (the master IMR).
#[test]
fn a_guest_out_and_in_round_trip_through_a_real_device() {
    // …
    assert_eq!(
        emu.reg_read(X86Reg::Rax) as u8,
        IMR,
        "the guest must read back the mask its own OUT installed"
    );
}
```

**Violates:**

```rust
// Illustration, not code from the tree: the name promises a deadline, and the
// assertion only checks that a request was queued.
#[test]
fn bmdma_start_arms_its_timer_at_issuing_epoch() {
    // … the guest programs DTPR and sets START at tick 41 …
    assert!(dm.ide.bus_master.take_pending_timer_arm(0).is_some());
}
```

---

## The named erasure/exemption registry

Kept complete on purpose — an exemption not listed here is a violation. Counted, not
recalled: a whole-word search for `dyn` over the `src` tree of every workspace crate finds:

- `rusty_box/src`: 22 occurrences. Nineteen are code, and every one of them is below. The
  other three are comments that name the pattern: in `emulator/engine.rs`, `emulator/run.rs`
  and `cpu/instrumentation/bochs.rs`.
- `rusty_box_gui/src`: 5, all code, all below.
- `rusty_box_devices/src`: 4. Two are code (`DeviceCtx`, below); two are comments, in
  `api.rs` and `display/card.rs`.
- `rusty_box_decoder/src` and `rusty_box_whp_engine/src`: 1 each, code, below.
- `rusty_box_core`, `rusty_box_whp_sys`, `rusty_box_whp`, `rusty_box_bximage`, `xtask`, and
  the three crates under `examples/`: none.

Integration tests, benches and example programs (the `tests/`, `benches/` and `examples/`
folders inside a crate) are outside this count.

**Sanctioned — a user's closure has no type to name, so erasure is the only form:**

- The user MMIO registry `Box<dyn FnMut…>` — `memory/mmio.rs`, reached from
  `cpu/access.rs` behind an `is_empty()` guard and exposed as `Emulator::mmio_map`
  (`emulator_api.rs`). Alloc-gated.
- `AnyDisk::Custom(Box<dyn BlockDevice + Send>)` — the reserved shape for a user disk
  backend. A target: neither `AnyDisk` nor `BlockDevice` exists in the tree. Listed so it is
  not re-litigated when it is built.

Instrumentation needs no entry: an observer is a type parameter
(`InstrumentationRegistry<T: Instrumentation = ()>`, `cpu/instrumentation/registry.rs`),
not stored closures.

**Tolerated, each with what removes it — a `dyn` here is debt, not design:**

- `Box<dyn BxGui>` / `&mut dyn BxGui` — `emulator/mod.rs`, `emulator/builder.rs`,
  `gui/gui_trait.rs`. The display adapters already draw into the `DisplaySink` trait
  (`rusty_box_devices/src/display/sink.rs`), and `GuiSink` in `gui/gui_trait.rs` is what
  still presents a `BxGui` as one. These go when the machine hands its frames only to a
  `DisplaySink`.
- `Box<dyn Fn()>` in `BxGui::headerbar_bitmap` and its three implementations — dies with
  the same seam.
- `&mut dyn CpuAccess` — `cpu/instrumentation/ctx.rs`. Load-bearing today: because
  `HookCtx` erases the whole context, dispatch has to move the tracer out of the registry
  rather than hold it beside `ExecCtx`. Making `HookCtx` generic is its own unit.
- `&mut dyn IrqSink` / `&mut dyn TimerService` in `rusty_box_devices`'s `DeviceCtx` — the
  fabric (`IrqFabric`, `rusty_box/src/iodev/irq.rs`) and the timer wheel live in
  `rusty_box`, so the devices crate has no concrete type to name until they move into a
  crate it can depend on.
- `&mut dyn FnMut(&mut [u8], core::ops::Range<usize>)` — the `write` parameter of
  `extended`, a closure local to `XsaveArea::patch` (`rusty_box_whp_engine/src/xsave.rs`),
  in production code. A closure cannot take a generic parameter, so each caller's writer is
  erased. A generic helper `fn` bounded by `F: FnMut(&mut [u8], core::ops::Range<usize>)`
  removes it.

**Unsealed on purpose:**

- `SliceEngine` (`rusty_box/src/emulator/engine.rs`) — public, because it bounds
  `Emulator`'s engine parameter, and implemented outside this crate:
  `rusty_box_whp_engine`'s `WhpEngine` is an implementation. A hypervisor
  backend must never be a dependency of the machine crate, so the adapter that
  knows both sides is a third crate (see the trait's own doc). That is why
  `PcIo`'s fields and `Processor::emulate_one` (`emulator/mod.rs`) are public:
  an engine services, on the machine's parts, what its hardware could not
  finish, while only the machine assembles a `PcIo` and pairs it with a
  processor (R3). The trait is not dyn-compatible,
  and not meant to be (R8): a machine names its engine at compile time.
  Because implementations live outside this crate, the trait gains only
  defaulted methods — a new required method would break every one of them.

**Foreign trait signatures — the shape is the foreign crate's, not chosen here:**

- `&mut dyn eframe::Storage` in `eframe::App::save`, implemented by `NativeShellApp`
  (`rusty_box_gui/src/app.rs`) and `AndroidShellApp` (`rusty_box_gui/src/android.rs`). The
  trait declares the parameter erased; the override is what makes eframe's suspend call on
  Android — the last call before the process can be killed — write the VM library.
- `Closure<dyn FnMut(web_sys::Event)>` in `rusty_box_gui/src/app.rs`, the browser shell's
  file-input change handler (`WebFilePicker`): `wasm_bindgen::closure::Closure` names its
  callback by a `dyn` trait object.
- `Option<&(dyn core::error::Error + 'static)>`, the return type of `core::error::Error::source`,
  implemented by `DecodeError` (`rusty_box_decoder/src/error.rs`).

**Not erasure, but exempted from R0 by name:**

- `ExecCtx::slice_parts` — internal 4-tuple destructure (R0 scope note).
- `static_cell` — reserved as the one `unsafe` dependency for no_alloc placement (R1),
  outside this tree; not yet a dependency.
- `rusty_box_whp_sys` — the host-FFI leaf, permanently outside R1's `forbid` end state; the
  section below is its registration.
- `rusty_box_gui/src/android.rs` — the Android front end's JNI calls on the Java VM and the
  NativeActivity that android-activity hands the process. It is host FFI in a crate that is
  not a named leaf: the crate inherits the workspace's `deny(unsafe_code)`, this one file lifts
  it with an `#![expect]` naming the invariant owner, every Java call goes through its single
  helper `with_activity`, and `xtask/src/ci.rs` holds the crate's count under its own baseline.
  Moving the calls into a leaf crate of their own is the open alternative.

## The registered R1 exception: the host-FFI leaf

`rusty_box_whp_sys` holds the Windows Hypervisor Platform FFI and its vocabulary types, and
`rusty_box_whp` is the typed wrapper above it — the conventional `-sys` shape. The leaf
**cannot** carry `#![forbid(unsafe_code)]`: holding the host calls is its entire purpose. It
is a permanent, deliberate exception to R1's end state, not a crate that has yet to reach it.
What the ratchet buys here is **confinement**, not zero: the workspace lint table denies
`unsafe_code`; inside the leaf, `rusty_box_whp_sys/src/windows.rs` is the one file that lifts
the deny wholesale, and everywhere else in the leaf it is lifted a single item at a time under
a named `#[expect]`.

**What the counts count.** The leaf's baseline counts two kinds of token with one number.
Every unsafe *operation* — a host call, or a read of the platform's register-value union —
lives in `windows.rs`: in a block, or, for `delete_partition`, `map_gpa` and `delete_vp`,
directly in the `unsafe fn` body, which needs no block. The other tokens are *markers*.

A public seam cannot lean on module privacy, so each verb that carries an obligation no type
can express states it in its signature, as `unsafe fn`:

- `map_gpa` leaves the hypervisor holding a host address past the borrow;
- `delete_partition`, `delete_vp` and `close_deadline` release a resource that a `Copy`
  handle cannot stop anyone releasing twice.

Each of those signatures is a token three times: in `windows.rs`; in `unsupported.rs`, which
performs no unsafe operation and carries a targeted `#[expect]` on each; and as a field type of
`ImpSignatures` in `lib.rs`, which `_IMP_IS_COMPLETE` uses to pin the signature on both
targets.

The tokens in `rusty_box_whp` *discharge* those obligations, each where a signature cannot go:

- three `Drop` bodies, of `OwnedPartition`, `Partition` and `DeadlineTimer`, because a
  destructor cannot be an `unsafe fn`;
- `map_range`, the crate's single door onto the retaining verb — R5's shape, reached by R5's
  argument.

Its one marker is `Partition::map_borrowed`, an `unsafe fn` because it maps memory the
partition does not own and so forwards the obligation to its caller. The current values, and
what each token is, are recorded beside the numbers in `xtask/src/ci.rs`.

R1 counts markers and operations with one number, so making an implicit obligation explicit as
an `unsafe fn` raises the count although no unsafe operation was added. The rule stands as
written and there is no second counting scheme; a reader who meets that wall has this
precedent to reason from. The way through is the one the ratchet already provides — a
re-baseline in the same commit, under a comment saying which tokens are markers and which are
operations.

## Running the mechanical enforcement

- `cargo xtask ci` — includes the `doctrine ratchets` step: per scanned tree, the `unsafe`
  token count, production `.unwrap()`/`.expect(…)`, and blanket `dead_code` allows, plus the
  `unsafe impl … Send/Sync` count in `rusty_box/src`, each against its baseline in
  `xtask/src/ci.rs`. It fails on any increase. Tighten a baseline in the same commit that
  lowers a count, and justify one in the same commit that raises it.
- `cargo test --release -p rusty_box --test compile_fail --features std` — the trybuild
  fixture registry. `cargo xtask ci` runs it too, as its `doctrine compile-fail fixtures`
  step. The goldens are rustc's rendered output and track the local toolchain; after a
  toolchain bump, regenerate them with `TRYBUILD=overwrite` once each fixture's rule is
  confirmed to still hold.
