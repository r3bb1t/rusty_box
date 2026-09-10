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

| id | rule | enforcement |
|---|---|---|
| R0 | Public returns are named types; bounds are named generics | public-api diff (optional job) + review |
| R1 | No undefined behavior; `unsafe` only ratchets down | **mechanical** (xtask ratchet; P8 `forbid`) |
| R2 | States are types, not flags | **mechanical** (fixtures) + review |
| R3 | Machine parts have no loose currency | **mechanical** (visibility + fixture) |
| R4 | Units are types | review citation |
| R5 | One choke point per hazard | compiler exhaustiveness + review |
| R6 | `Send` derives, never promised | **mechanical** (ratchet + const asserts) |
| R7 | Bochs parity provenance | social — stated honestly |
| R8 | Erasure is opt-in and named | review + public-api diff |
| R9 | Tests assert guest-visible properties | social + fixture registry |

---

## R0 — Public returns are named types; bounds are named generics

No bare tuples in a public return (`Resolution { width, height }`, never `(u32, u32)` — a
tuple invites the width/height swap; `TextPos { row, col }`, never an ambiguous pair). No
anonymous `impl Trait` in public signatures: return types are named (`SerialDrain<'a>`,
`FiredTimers<'a>`), argument bounds are named generics (`fn disk<D: BlockDevice + Send +
'static>(…)`). No meaningless scalars where an enum speaks: `Repeat { Once, Periodic }`
replaced `continuous: bool`; `StopReason` replaced a stop `bool`.

*Why:* names survive at call sites; anonymous shapes leak auto-traits implicitly and cannot be
stored, documented, or diffed. *Scope:* the public surface. Internal choke points may use
tuples where destructuring is the point (named exemption: `ExecCtx::slice_parts`).

**Shape stability across features:** public types are the same shape in every configuration.
A cargo feature may gate a *type's existence* (`OwnedRam` arrives with `alloc` — additive) or
a *value's constructibility*, but never variant or field presence on a public type — no
`#[cfg]`'d enum variants, no feature-dependent match surfaces. Where a compile-time choice
between representations exists, it is a **type parameter** (`RamStore<B: RamBytes>`), not an
enum whose completeness depends on the build.

## R1 — No undefined behavior; `unsafe` only ratchets down

Every `unsafe` block carries a `// SAFETY:` comment naming **who owns the invariant** it
relies on. The per-crate count of `unsafe` tokens may never increase; when work removes some,
the baseline in `xtask/src/ci.rs` is tightened in the same commit. End state (campaign P8):
`#![forbid(unsafe_code)]` on every library crate but the host-FFI leaf, with no_alloc
placement delegated to the vetted `static_cell` crate so the one unavoidable `unsafe` lives
outside this tree. The leaf, `rusty_box_whp_sys`, is a permanent exception and is registered
below.

*Enforcement (mechanical):* the `doctrine ratchets` ci step counts comment-stripped `unsafe`
tokens per crate against the baseline and fails on any increase. A rise is not impossible,
only impossible quietly: ci passes again only when that crate's baseline is raised in the
same commit, under a comment saying what the new tokens are and why.

## R2 — States are types, not flags

If a value has more than one operational state that the *API author* controls, those states
are distinct types — never an `Option<NonNull<…>>` lifecycle field, never an `is_wired()` /
`is_initialized()` predicate. A transient capability is a borrow-carrying context (`ExecCtx`);
an assembly sequence is a builder (the powered-off machine *is* `MachineBuilder`; a built
machine is always at the reset vector). States the **guest** controls at runtime (ACPI
shutdown, halt) are typed *values* (`ShutdownState`), because a typestate the guest can
invalidate mid-instruction is a lie — the same line llvmkit D8 draws between author-controlled
transitions and witnessed runtime facts.

*Enforcement:* the campaign deletes the violating fields and predicates; trybuild fixtures pin
the shape (executing an instruction on a bare CPU without a machine does not compile).

## R3 — Machine parts have no loose currency

CPU, memory, devices, and pins never leave the machine that owns them. An `ExecCtx` is
assembled **only** by a machine facade destructuring its own `&mut self`; its fields are
private; no public API accepts or returns machine internals. This is the D7 (cross-module
mixing) answer for an ownership-shaped system: llvmkit needs brand types because its ids float
free as currency; rusty_box has no currency to brand, so a single assembly choke point makes
mixing unrepresentable — and the fleet case (`Vec<Emulator<_, P>>`) keeps full static typing,
which per-instance brands cannot (confirmed against ghost-cell/generativity adoption data:
no shipping crate exposes brands in a composition API).

*Enforcement (mechanical):* visibility — external assembly fails to compile (fixture-pinned);
the `ExecCtx::new` doc names its complete legitimate-caller list.

## R4 — Units are types

An offset is not an address; a guest-physical address is not a linear address; a port is not a
memory address; a window offset is not a VRAM offset. Precedents already in-tree: `RamPage`
(RAM offset), `AllocPage` (allocation offset), `FetchWindow { start, len }`,
`CpuMemoryPolicy`. Adopted from vm-memory/vm-device: position − position yields a raw
*distance* scalar, never another position; the port space gets its own `u16`-backed type. The
strongest form makes a conversion function the *only* path between unit domains — SVGA banking
becomes `MemoryWindow::translate(WindowOffset) -> Option<VramOffset>`, so an untranslated
access is a compile error, not a review catch. Instruction throughput is `Ips`, a newtype,
required at builder construction (`Ips::BOCHS_DEFAULT` = 50_000_000, Bochs config.cc
`cpu.ips`).

*Enforcement:* review citation; the types exist and new raw-scalar boundary crossings are
defects.

## R5 — One choke point per hazard

Every guarded hazard has exactly one code path through its guard: pin publication, `ExecCtx`
assembly, ISA gating (applied at icache fill), SMC stamping, banking translation. Adding a
second path to a choke-pointed hazard is a defect even if it is correct today. Closed dispatch
sets are exhaustive `match`es over enums *without* `#[non_exhaustive]`, so adding a variant
breaks every site the compiler can see — that is the feature. (Validated in production by
Firecracker's bus history: their closed-enum bus shipped for two years; the switch to
`Arc<Mutex<dyn>>` was code-import convenience and produced a use-after-detach race within a
year — PR #6062.)

*Enforcement:* compiler exhaustiveness where the pattern applies; marker comments + review for
the choke points themselves.

## R6 — `Send` derives, never promised

`unsafe impl Send`/`Sync` is banned. Thread safety is proven by construction — no raw-pointer
fields, no globals — and pinned by `const` assertions: `assert_send::<BxMemoryStubC>()` and
`assert_send::<Emulator<()>>()`. A `Send` assertion on a type that was accidentally `Send`
proves nothing, so each is paired with a non-vacuity check — for the machine, a generic
`assert_send::<Emulator<T>>()` over any `Send` tracer, so the property cannot rest on the
`()` default. The no-alloc `Emulator` is deliberately `!Send` (`ap_cpu_ptrs` is
caller-contract storage) and documents it.

*Enforcement (mechanical):* the `doctrine ratchets` ci step counts `unsafe impl … Send/Sync`
lines (baseline 0); `cargo-semver-checks`' `auto_trait_impl_removed` lint guards regressions
once adopted; the const asserts are compile-time.

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

## R8 — Erasure is opt-in and named

No `dyn` in any public signature and none on any dispatch path. Where an open set genuinely
exists, erasure lives in exactly one named place: an alloc-gated enum variant
(`AnyDisk::Custom(Box<dyn BlockDevice + Send>)`) or closure storage for hooks (`Box<dyn
FnMut…>`, alloc-gated). In-tree sets dispatch as enums (`DisplayDevice::{Vga, GeForce, …}`);
user extension composes statically through profile associated types (`type Display = MyGpu`).
Optional, profile-dependent capabilities use the `support_*() -> Option<XxxOps<'_>>` accessor
convention (gdbstub's IDET idiom — the one borrow-based capability shape the ecosystem already
knows). Traits that would be trivially erasable are kept deliberately non-dyn-safe (generic
methods) when no consumer needs erasure.

*Enforcement:* review + the public-api snapshot diff when adopted.

## R9 — Tests assert guest-visible properties, not transports

A test named after a guarantee must assert the guarantee, not the mechanism that currently
delivers it (the `…_at_issuing_epoch` lesson: it checked a request table instead of the
deadline). Type-level invariants get trybuild fixtures; runtime invariant tests get
mutation-checked once (break the invariant deliberately, watch the test fail) before they are
believed. Test provenance follows the parity rule: ported Bochs behavior cites its upstream
test or the Bochs source it exercises.

*Enforcement:* social; the compile_fail fixture registry (`rusty_box/tests/compile_fail/`)
is the mechanical half for type-level claims.

---

## The named erasure/exemption registry

Kept complete on purpose — an exemption not listed here is a violation. Counted, not
recalled: `dyn` in `rusty_box/src` is 43 occurrences, and every one of them is below.

**Sanctioned — a user's closure has no type to name, so erasure is the only form:**

- Hook storage `Box<dyn FnMut…>` — `cpu/instrumentation/{hooks,registry}.rs`, alloc-gated.
- The user MMIO registry `Box<dyn FnMut…>` — `memory/mmio.rs`, reached from
  `cpu/access.rs` behind an `is_empty()` guard and exposed as `Emulator::mmio_map`.
  Alloc-gated.
- `AnyDisk::Custom(Box<dyn BlockDevice + Send>)` — the reserved shape for a user disk
  backend (P7). Not built yet; listed so it is not re-litigated when it is.

**Tolerated, each with the unit that removes it — a `dyn` here is debt, not design:**

- `Box<dyn BxGui>` / `&mut dyn BxGui` — `emulator/mod.rs`, `emulator/builder.rs`,
  `gui/gui_trait.rs`. The display leaves the machine as a `DisplaySink` (REPLAN unit B/H2).
- `Box<dyn Fn()>` in `BxGui::headerbar_bitmap` and its three implementations — dies with
  the same seam.
- `&mut dyn CpuAccess` — `cpu/instrumentation/ctx.rs`. Load-bearing today: because
  `HookCtx` erases the whole context, dispatch has to move the tracer out of the registry
  rather than hold it beside `ExecCtx`. Making `HookCtx` generic is its own unit.
- `&mut dyn IrqSink` / `&mut dyn TimerService` in `rusty_box_devices`'s `DeviceCtx` — the
  fabric and the timer wheel live in `rusty_box`, so the devices crate has no concrete type
  to name until they move (unit L).

**Sealed by signature rather than by a private supertrait:**

- `SliceEngine` — public, because it bounds `Emulator`'s engine parameter, and
  implementable only inside this crate, because its arguments (`BxCpuC`'s
  siblings via `PcIo`, and `SliceRequest`, whose fields are crate-private) are.
  The seal is deliberate: an engine needs the machine's insides, and a backend
  crate stays a host-FFI leaf that this crate adapts. Per the per-trait sealing
  policy, that keeps the trait free to gain methods.

**Not erasure, but exempted from R0 by name:**

- `ExecCtx::slice_parts` — internal 6-tuple destructure (R0 scope note).
- `static_cell` — the one `unsafe` dependency for no_alloc placement (R1), outside this tree.
- no-alloc `Emulator` is `!Send` — documented caller-outlives contract (R6).
- `rusty_box_whp_sys` — the host-FFI leaf, permanently outside R1's `forbid` end state; the
  section below is its registration.

## The registered R1 exception: the host-FFI leaf

`rusty_box_whp_sys` holds the Windows Hypervisor Platform FFI and its vocabulary types, and
`rusty_box_whp` is the typed wrapper above it — the conventional `-sys` shape. The leaf
**cannot** carry `#![forbid(unsafe_code)]`: holding the host calls is its entire purpose. It
is a permanent, deliberate exception to R1's end state, not a crate that has yet to reach it.
What the ratchet buys here is **confinement**, not zero: the workspace lint table denies
`unsafe_code`, `rusty_box_whp_sys/src/windows.rs` is the one file that lifts the deny
wholesale, and everywhere else it is lifted a single item at a time under a named `#[expect]`.

**What the counts count.** The baselines are `rusty_box_whp_sys/src` 43 and `rusty_box_whp/src`
4 — 47 against the 38 of the single crate they replace. The number of unsafe *operations* is
unchanged: the same 37 host calls and union reads, all still in `windows.rs`. What rose is
markers. A public seam cannot lean on module privacy, so the three verbs carrying an
obligation no type can express state it in their signatures — `map_gpa`, which leaves the
hypervisor holding a host address past the borrow, and `delete_partition` / `delete_vp`, which
release a resource a `Copy` handle cannot stop anyone releasing twice. In `rusty_box_whp` the
three tokens that *discharge* those obligations sit where a signature cannot go: two `Drop`
bodies, because a destructor cannot be an `unsafe fn`, and `map_range`, the crate's single
door onto the retaining verb — R5's shape, reached by R5's argument.

R1 counts markers and operations with one number, so it reads this rise as a regression where
an implicit obligation in fact became explicit and compiler-enforced. The rule stands as
written and no second counting scheme is introduced; a reader who meets that wall has this
precedent to reason from. The way through is the one the ratchet already provides — a
re-baseline in the same commit, under a comment saying which tokens are markers and which are
operations.

## Running the mechanical enforcement

- `cargo xtask ci` — includes the `doctrine ratchets` step (unsafe count + unsafe-impl count
  vs the baselines in `xtask/src/ci.rs`; fails on any increase; tighten baselines in the same
  commit that lowers a count, and justify one in the same commit that raises it).
- `cargo test -p rusty_box --test compile_fail --features std` — the trybuild fixture
  registry. Deliberately *not* part of the release gate matrix (fixture goldens track the
  local toolchain; regenerate with `TRYBUILD=overwrite` after a toolchain bump).
