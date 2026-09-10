# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Rules

- Do NOT commit unless the user explicitly tells you to.
- Do NOT push unless the user explicitly tells you to.
- Prefer LSP tools (definition, references, hover, diagnostics) over grep for code navigation.
- Use `lsp references` before modifying any function, type, or exported symbol to find all consumers. When no LSP server is exposed to the session — check before assuming one is — the compiler is the stronger oracle for a removal or a signature change: make the change, build, and read the error list, which enumerates every consumer exactly. Grep is the fallback for the read-only survey that precedes it, and it misses line-wrapped and deref spellings (`&mut *self.x`), so never treat a grep count as complete.
- **Never skip an inaccuracy against the original Bochs.** Every divergence from `cpp_orig/bochs/` is a bug, *unless* the divergence is purely an improvement to idiomatic Rust (enum instead of magic-number int, bitflags instead of raw u32, RAII instead of manual lifecycle, etc.) that does not change observable behavior. The deliberate exceptions are registered in `docs/bochs-parity-divergences.md`, each with the evidence that justifies it — read it before re-litigating one, and add an entry (never just a code comment) when a new one is agreed. If a Bochs feature is too large to fix in the current change, **call it out explicitly** in the end-of-work overview so the user can decide whether to defer it — never silently leave it as a stub or `// not implemented yet` comment.
- **Thread safety trumps Bochs literalness.** If a Bochs construct is non-thread-safe (sequenced read+write that says "should be atomic RMW", shared state accessed without a lock, etc.), fix it with Rust atomics / Mutex / lock-free primitives — even when Bochs itself doesn't. Single-threaded-per-CPU Bochs assumptions silently transfer correctness obligations to the caller; don't inherit them. Scope: cross-thread / shared state only; CPU-local state doesn't need atomics.
- Bochs-source comments must cite the file + symbol (e.g. `// Bochs cet.cc INCSSPD`), never the specific line number — the upstream snapshot rebases.
- **Comments describe the code as it stands, never how it got there.** State the invariant a construct maintains and cite the Bochs symbol it mirrors. Do NOT narrate history: no "this used to…", "previously…", "now lives here", "the old code…", no description of the bug a line fixes, and no before/after comparison. A reader needs the rule that holds today, not a changelog — that belongs in the commit message. The same goes for naming: nothing is `*_fixed`, `*_new`, or `*_v2`.
- **No partial work.** Every change must be a fully working implementation of the feature AND every related piece of code it touches. No `TODO`, `FIXME`, "deferred", "stub", "skeleton", or "follow-up" implementations — if a function is added it does the real work, including any helpers it requires; if a helper does not yet exist (because the existing code couldn't be called by the new caller), refactor the existing code so the helper exists, then call it. If the work is too large to finish in one change, STOP and ask the user — do not ship a half-implementation.
- **Never `let _ = call_returning_result()`.** Discarding a `Result` hides failures. Either propagate with `?`, or handle every variant explicitly with `match`. The same rule applies to discarding a meaningful return value of any kind: if the function returns useful information (a count, an index, a Bochs-style "0 = ok / N = failed at item N" code), the caller MUST react to it, not throw it away.
- **The Safety Doctrine R0–R9 governs every API and safety seam** — full prose in `docs/safety-doctrine.md`. Cite rule ids in commit messages and reviews whenever a change turns on one. Short list: R0 named returns/generics (no tuples, no `impl Trait`, no meaningless bools in public APIs) · R1 unsafe only ratchets down, every block names its invariant owner · R2 states are types, not flags · R3 machine parts have no loose currency (`ExecCtx` assembled only by a machine's own `&mut self`) · R4 units are types (offset ≠ address ≠ port) · R5 one choke point per hazard, exhaustive matches on closed sets · R6 `Send` derives, never promised · R7 parity provenance (this file's Bochs rules, plus declared provenance for devices Bochs lacks) · R8 erasure opt-in and named (no `dyn` in signatures; the exemption registry is in the doctrine doc) · R9 tests assert guest-visible properties.
- **Verification cadence:** `cargo check --release -p rusty_box --features std --lib` after each edit batch, **plus `--no-default-features`** whenever the change touches cfg-gated code or `emulator/`/`memory/` (the no_std build is a different compile, not a subset — it has caught breaks the std build cannot see). Full `cargo xtask ci` before every commit, and commit only the tree the gates saw.
- **Edit with the Edit/Write tools, not shell heredocs or Python.** Edit's exact-match requirement is the safety check; a script has none. Scripted edits have silently damaged this tree twice: a block delete by line range swallowed the `#[cfg]` belonging to the *next* item, and a regex over `self.cpu.` missed the `&mut *self.cpu` and line-wrapped spellings it was meant to catch. A throwaway script is justified only for a genuinely repetitive mechanical rewrite across many files — say so, show it, and re-read a sample of what it produced.
- **"It compiles clean" says nothing about whether code is REACHED.** Four things here are invisible to the compiler, and each has hidden a real defect: (1) a source file with no `mod` declaration is never compiled at all — two such files existed, both holding `unimplemented!()`; when sweeping for them note that `#[path = "…"] mod x;` counts as a declaration; (2) a file-level `#![allow(dead_code)]` — `cpu/vmx.rs` has one — hides every unused item in that file; (3) a feature can be fully implemented yet unreachable because its capability bit is masked off, so no guest can request it; (4) a trybuild golden outside `cargo xtask ci` rots silently. Establish that a guest (or a caller) can actually reach code before calling it either a live bug or dead code.
- **Measure a blast radius before quoting one.** Claiming "this touches N files" from a guess about how a type propagates is how a sound change gets talked out of. Count it — `grep -c` on the impl headers, or a trial edit — and say what you counted. A defaulted type parameter in particular does NOT propagate through `&mut T` positions; it silently binds the default instead.
- **"Zero callers" is a claim, so measure it like one.** Never conclude something is dead from a grep you piped through `head`, or from one alternation whose noisiest term buries the rest — three "unused" accessors deleted that way turned out to have eight callers in tests, and the compiler found them, not the grep. Search one symbol at a time, with no truncation, across `src` AND `tests`. The compiler is the oracle: delete, build, read the errors. And when the answer is "genuinely unused", it is still not automatically deletable — ported-ahead Bochs parity (`geforce.rs`, `tlb.h`'s `memtype`/`isReadOK`) is kept with a TARGETED `#[allow(dead_code)]` naming its upstream symbol, never a file-level one.
- **`replace_all` replaces all of them.** Check every site first, or edit them individually. One blind `replace_all` deleted the used bindings along with the two unused ones and had to be reverted wholesale — the same class of damage as a scripted edit, from a tool that is otherwise exact.
- **A capability constant and the MSR/CPUID leaf advertising it change together.** Widening an allowed-1 mask without updating the MSR that reports it leaves the feature unreachable; the reverse advertises something VMENTRY will reject. The same holds for any spec-defined encoding table: before writing a test over one you did not author, check the table against Bochs or the SDM — a test written on top of a wrong table cements the bug instead of catching it (the VMX capability MSR block was shifted by one encoding, and the first test written over it asserted the shift was correct).
- **A background wait must have a sentinel that can actually occur.** Waiting on a string the producer never writes hangs forever — one such loop span for 2h41m against a workflow journal that emits `"type":"result"`, never `"type":"finished"`. Prefer waiting on the task-completion notification; if polling, first confirm the sentinel appears in real output.

## Project Overview

Rusty Box is a Rust port of the Bochs x86 emulator -- a complete CPU/system emulator targeting 32/64-bit x86 architecture. The original C++ Bochs source is in `cpp_orig/bochs/` for reference.

**Status:** DLX Linux boots to interactive bash shell. Alpine Linux fully boots. UEFI example completes BIOS POST and reaches boot sector.

## Build Commands

```bash
cargo xtask ci                                # THE gate suite (every step in xtask/src/ci.rs + doctrine ratchets); run before every commit
cargo build --release --all-features          # Full build
cargo test --release -p rusty_box --lib --features std   # lib tests (fast loop)
cargo run --release --example dlxlinux --features std            # DLX headless
cargo run --release --example rusty_box_egui --features "std,gui-egui"  # GUI
cd examples/rusty_box_web && trunk serve      # WASM dev server
cargo check --no-default-features -p rusty_box  # no_std + no_alloc build
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi  # UEFI app
cargo test --release -p rusty_box --test compile_fail --features std  # doctrine fixture registry (a ci step; TRYBUILD=overwrite regenerates goldens)
```

## Observing the egui GUI (agents: use this, not desktop screenshots)

To see what the egui GUI is rendering (the VMware-style shell **and** the guest's
VGA console) and to drive it, use egui 0.35's built-in **inspection protocol** via
the `egui_mcp` server — do NOT rely on OS-level desktop screenshots (they capture
the wrong window, need the window foregrounded, and can't synthesize input into egui
reliably). The inspection path attaches straight to the running app, reads its
accessibility tree, and synthesizes real input events.

One-time setup:
```bash
# rusty_box_gui already builds eframe with the `inspection` feature.
cargo install --git https://github.com/rerun-io/kittest_inspector egui_mcp  # installs egui-mcp
claude mcp add egui egui-mcp   # register the stdio MCP server (needs a session reload to expose mcp__egui__* tools)
```

Launch the GUI with inspection enabled (the app then listens on `127.0.0.1:5719`):
```bash
EGUI_INSPECTION=1 cargo run --release -p rusty_box_gui -- --no-config --display egui \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom <image.iso> --boot cdrom --memory-mib 256 --host-memory-mib 256 --ips 300000000
```

Then work in an **observe → act → verify** loop with the `egui` MCP tools: `attach`
first, then `screenshot` (renders the exact egui frame, VGA texture included) and
`query_tree` (find widget `id`/`role`/text) to orient; `click`/`type_text`/`press_key`
to drive (e.g. click the "Power On VM" button to boot); `wait_for` to poll async UI.
Prefer text/`id` locators over raw `pos`. The GUI starts **Stopped** on the launcher —
it must be powered on (click "Power On VM") before the guest console appears under the
Console tab; the status bar shows `Running · <n> IPS` once booting.

If the `mcp__egui__*` tools aren't loaded in the current session, drive `egui-mcp`
directly over stdio (newline-delimited JSON-RPC: `initialize` → `notifications/initialized`
→ `tools/call attach` → `tools/call screenshot {"save_path": "..."}`) and read the saved PNG.

## Architecture

```
Emulator<T: Instrumentation = ()>        (no lifetime params; CPU model is runtime data: CpuModel enum)
+-- BxCpuC<T>         CPU core state (registers, TLBs as OFFSETS into the allocation, icache)
+-- BxMemC            Memory subsystem (block-based, supports >4GB; owns its RAM as Box<[GuestPage]>)
+-- BxDevicesC        I/O port handler manager (65536 ports, fixed arrays)
+-- DeviceManager     Hardware devices (PIC, PIT, CMOS, DMA, VGA, Keyboard, IDE, Serial)
+-- BxPcSystemC       Timers and A20 line control
+-- GUI               Display (NoGui, TermGui, or EguiGui) [alloc only]
```

**Execution runs on `ExecCtx` (`cpu/exec_ctx.rs`), not on a bare CPU.** The dispatcher and CPU
loop are `impl ExecCtx` methods; an `ExecCtx` holds disjoint `&mut` borrows of cpu/memory/
devices/pc_system plus the shared pin slice, built per scheduler slice by
`Emulator::exec_ctx(index)` (both alloc and no-alloc). It `Deref`s to `BxCpuC`, so CPU-state
code reads `self.…` unchanged. Unit tests that execute instructions use `TestMachine::ctx()` /
`exec_with` in exec_ctx.rs. Deref hazards to know: a `BxCpuC` method named after a universal
trait method (`into`, `from`, `clone`, …) or after an ExecCtx field (`memory`, `devices`,
`pc_system`, `pins`) is silently shadowed at ExecCtx call sites; nested
`self.field[i].set(self.field[i].get())` patterns that were one borrow on the CPU are two
through Deref — bind the inner value first.

### Key Design Principles

- **No global state** -- each `Emulator` is fully self-contained
- **Bochs parity** -- all logic must match Bochs C++ source exactly; deviations are bugs
- **no_std + no_alloc core** -- CPU, memory, decoder, I/O devices, emulator all compile without alloc. Fixed-size arrays and RingBuffer replace Vec/VecDeque. Alloc-dependent features (GUI, diagnostic String returns, StopHandle, hook closures) are behind `#[cfg(feature = "alloc")]`.
- **Send by derivation** -- no `unsafe impl Send` in the tree. `BxMemoryStubC` and the alloc-build `Emulator` are pinned by `const` asserts in their own modules; the no-alloc `Emulator` is deliberately `!Send` (caller-supplied AP CPU pointers). The doctrine-ratchets ci step enforces both directions.

### no_alloc Construction (UEFI path)

```rust
// Placement construction -- no Box, no allocator
BxCpuBuilder::<I>::init_cpu_at(cpu_ptr, tracer)     // CPU at raw pointer
BxMemoryStubC::create_from_raw(ptr, len, ...)       // Memory from raw buffer
Emulator::init_at(emu_ptr, cpu, mem_stub, config)   // Emulator at raw pointer
```

## Workspace Structure

- **rusty_box/** -- Main emulator library
- **rusty_box_decoder/** -- Separate crate for x86 instruction decoding
- **examples/rusty_box_web/** -- WASM web frontend
- **examples/rusty_box_uefi/** -- UEFI bootable emulator application (no allocator)
- **cpp_orig/bochs/** -- Original C++ Bochs source for reference

## Key Files for Common Tasks

| Task | Files |
|------|-------|
| Add new instruction | `rusty_box_decoder/src/fetchdecode*.rs`, `rusty_box/src/cpu/<category>/` |
| Add new I/O device | `rusty_box/src/iodev/` (new file), `iodev/devices.rs` (registration) |
| Modify memory mapping | `rusty_box/src/memory/misc_mem.rs`, `memory/mod.rs` |
| Add/modify FPU instruction | `rusty_box/src/cpu/fpu/` (handlers), `cpu/softfloat3e/` (math) |
| Ring buffer (replaces VecDeque) | `rusty_box/src/ring_buffer.rs` |

## Feature Flags

- `std` -- Standard library support (terminal, file I/O, tempfile). Implies `alloc`.
- `alloc` -- Heap allocation. Enables `Emulator::new()`, GUI, diagnostic methods, StopHandle.
- `gui-egui` -- Graphical UI using egui.
- `instrumentation` -- Closure-based CPU hooks. Implies `alloc`.
- `profiling` -- Profiling support. Implies `std`.
