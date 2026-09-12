# Contributing to Rusty Box

The binding project rules are in [CLAUDE.md](CLAUDE.md), the [Safety Doctrine](docs/safety-doctrine.md) (R0-R9) and the [Bochs parity divergence registry](docs/bochs-parity-divergences.md). This page summarises them and adds the mechanics.

## Setup

- A stable Rust toolchain. Work in release builds only (`--release`).
- A Bochs checkout at `cpp_orig/bochs/` (gitignored): `git clone https://github.com/bochs-emu/Bochs cpp_orig/bochs`. It is the reference source every change is checked against, and the source of the BIOS ROMs (`cpp_orig/bochs/bochs/bios/`).
- The DLX Linux disk image at `dlxlinux/hd10meg.img` (gitignored), from [Bochs disk images](https://bochs.sourceforge.io/diskimages.html). The boot gate, the UEFI build and the web demo need it.
- Rust targets used by the gate: `rustup target add x86_64-unknown-none x86_64-unknown-uefi wasm32-unknown-unknown`.
- On Linux, the GUI's display-server packages: `pkg-config libgl1-mesa-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev libxkbcommon-dev libwayland-dev`.

The repository's `.cargo/config.toml` sets two environment variables for every cargo command. `RUST_MIN_STACK=16777216` exists because a test that builds a ~1.4 MiB `TestMachine` per loop iteration overflows libtest's default 2 MiB thread stack. `MAX_INSTRUCTIONS=20000000000` caps the boot examples that read it (`dlxlinux`, `dlxlinux_egui`, `rusty_box_egui` and the Alpine examples).

## The gate: `cargo xtask ci`

Run it before every commit, and commit only the tree it saw. It runs, in order:

1. **Doctrine ratchets.** They scan seven crates: `rusty_box`, the decoder, `rusty_box_core`, `rusty_box_devices` and the three WHP crates. In each, the `unsafe` token count may only go down (a commit that removes unsafe tightens its baseline in `xtask/src/ci.rs` in the same commit), the number of files with a blanket `dead_code` allow may only go down, and there is no `.unwrap()` / `.expect(…)` outside test code. `rusty_box` has no `unsafe impl Send/Sync`.
2. **The test and check matrix.** It tests `rusty_box_core` and `rusty_box_devices` (including their no_std + no_alloc builds), the WHP crates (their hardware tests skip without a hypervisor), the decoder and the `rusty_box` library. It checks `rusty_box` as no_std with and without alloc, for `x86_64-unknown-none`, and for `wasm32-unknown-unknown` with alloc, and builds the UEFI application. It runs the public-API examples, the doc examples and the doctrine compile-fail fixtures, checks the GUI for wasm and with its tests, and does an all-features check. Last comes a debug-assertions check, the only step that compiles `#[cfg(debug_assertions)]` code. The gate does not run the tests of `rusty_box_bximage` or `rusty_box_gui`; run them with `cargo test --release -p <crate>`.
3. **The DLX boot gate.** It runs `dlxlinux` headlessly and requires the login prompt within 450 million instructions. This step takes several minutes.

`--skip-boot` drops the boot gate; `--full` adds the full `rusty_box` integration test suite and the GUI release build.

## Fast loop

```bash
# After each edit batch
cargo check --release -p rusty_box --features std --lib
# ...plus this whenever the change touches cfg-gated code, emulator/ or memory/
cargo check --release -p rusty_box --no-default-features

# Unit tests
cargo test --release -p rusty_box --lib --features std
cargo test --release -p <crate>

# Doctrine compile-fail fixtures (regenerate goldens with TRYBUILD=overwrite,
# only after confirming the rule they pin still holds)
cargo test --release -p rusty_box --test compile_fail --features std

# UEFI application
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi

# Fuzz the decoder (fuzz_fetchdecode64, fuzz_fetchdecode32_32, fuzz_fetchdecode32_16)
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_fetchdecode64
```

The WHP crates (`rusty_box_whp_sys`, `rusty_box_whp`, `rusty_box_whp_engine`) build and run their tests on any host, and the tests that start a machine on hardware skip when there is no hypervisor. Only a Windows host with the Hypervisor Platform enabled exercises them, so test changes to them there.

DLX runs a 32-bit kernel. A change that touches long-mode paths also needs an Alpine x86_64 boot:

```bash
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=3500000000 cargo run --release --example alpine_direct --features std
```

## Rules

1. **Bochs parity.** Every behavioural divergence from Bochs is a bug. Purely idiomatic Rust structure (an enum instead of a magic integer, bitflags, RAII) is allowed when it changes nothing observable. Deliberate exceptions are registered, with evidence, in [docs/bochs-parity-divergences.md](docs/bochs-parity-divergences.md); add an entry there, never just a code comment. If a Bochs feature is too large for your change, say so explicitly instead of leaving a stub.
2. **Thread safety over Bochs literalness.** Shared, cross-thread state uses atomics or locks even where Bochs assumes a single thread.
3. **The Safety Doctrine.** R0-R9 in [docs/safety-doctrine.md](docs/safety-doctrine.md) govern every API and safety seam. Cite the rule ids in commit messages and reviews when a change turns on one. Among them: no `dyn` in signatures outside the doctrine's named exemption registry (R8), generics and trait bounds instead; `unsafe` only ratchets down, and every block names who owns its invariant (R1); tests assert guest-visible properties (R9).
4. **No partial work.** No `TODO`, `FIXME`, stub or skeleton implementations. If a change is too large to finish, stop and discuss it.
5. **Never discard a `Result`** with `let _ =`. Propagate it with `?` or match every variant. The same goes for any meaningful return value.
6. **Comments state the invariant the code maintains** and cite the Bochs file and symbol they mirror (e.g. `// Bochs cet.cc INCSSPD`), never a line number. They do not narrate history; that belongs in the commit message.
7. **No global state.** The emulator is fully instance-based, with no static mutables.
8. **no_std + no_alloc first.** Core emulation compiles without std or alloc. Heap-dependent features (display front ends, diagnostic `String` returns, `StopHandle`) sit behind `#[cfg(feature = "alloc")]`.

## Workspace

| Crate | Role |
|-------|------|
| `rusty_box` | The emulator library: CPU, memory, machine, machine-wired I/O devices, examples |
| `rusty_box_core` | no_std, `forbid(unsafe_code)` foundations shared by devices and engines |
| `rusty_box_devices` | no_std, `forbid(unsafe_code)` device models with no CPU or bus access (VGA, PCI) and the device API traits |
| `rusty_box_decoder` | x86 instruction decoder |
| `rusty_box_whp_sys` | Windows Hypervisor Platform FFI leaf; every host call is confined to `windows.rs` |
| `rusty_box_whp` | Safe wrapper over the WHP leaf |
| `rusty_box_whp_engine` | Runs a `rusty_box` machine's guest on WHP |
| `rusty_box_gui` | The VMware-style egui VM shell (desktop, Android and wasm), with CLI/TOML config and a choice of interpreter or WHP engine |
| `rusty_box_bximage` | bximage-compatible disk image creation |
| `xtask` | The `cargo xtask ci` gate, `perf-baseline`, and Android packaging |
| `rusty_box_web` | Standalone WASM web demo (`examples/rusty_box_web/`) |
| `rusty_box_uefi` | UEFI application with no allocator (`examples/rusty_box_uefi/`) |
| `rusty_box_no_alloc_smoke` | no_alloc link smoke test for `x86_64-unknown-none` (`examples/no_alloc_smoke/`) |

### Execution model

The dispatcher and the instruction handlers run on an `ExecCtx` (`rusty_box/src/cpu/exec_ctx.rs`), not on a bare CPU. An `ExecCtx` holds disjoint `&mut` borrows of one CPU, memory, devices and the PC system for one scheduler slice, and `Deref`s to `BxCpuC`, so CPU-state code reads `self.…` directly. Two hazards follow. A `BxCpuC` method named after a universal trait method (`into`, `from`, `clone`, …) or after an `ExecCtx` field (`memory`, `devices`, `pc_system`, `pins`) is silently shadowed at `ExecCtx` call sites. And a nested `self.field[i].set(self.field[i].get())` is two borrows through `Deref`, so bind the inner value first.

### Code organization

CPU instructions are organized by category, mirroring the Bochs `cpu/` directory:

| Category | Files (`rusty_box/src/cpu/`) |
|----------|-------|
| Arithmetic | `arith8.rs`, `arith16.rs`, `arith32.rs`, `arith64.rs` |
| Logical | `logical8.rs`, `logical16.rs`, `logical32.rs`, `logical64.rs` |
| Data transfer | `data_xfer8.rs`, `data_xfer16.rs`, `data_xfer32.rs`, `data_xfer64.rs` |
| Control flow | `ctrl_xfer16.rs`, `ctrl_xfer32.rs`, `ctrl_xfer64.rs` |
| Stack | `stack.rs`, `stack16.rs`, `stack32.rs`, `stack64.rs` |
| FPU | `fpu/*.rs` (handlers) + `softfloat3e/*.rs` (math library) |

### no_alloc design

The core emulator compiles without `alloc`. Device state uses fixed-size types:

| Heap type | Replacement |
|-----------|-------------|
| `Vec<T>` | `[T; N]` (compile-time sized arrays) |
| `VecDeque<T>` | `RingBuffer<T, N>` (`rusty_box_core::ring_buffer`) |
| `String` | `&'static str` or `[u8; N]` + length |
| `Box<dyn BxGui>` | gated behind `#[cfg(feature = "alloc")]` |
| `Arc<AtomicBool>` | `AtomicBool` (no-alloc) / `Arc<AtomicBool>` (alloc) |

Construction without alloc places each large structure in caller-provided memory. `examples/rusty_box_uefi/src/main.rs` is the reference:

- `BxCpuBuilder::new().init_cpu_at(ptr, tracer)` -- a CPU at caller-provided memory (`unsafe`)
- `BxMemoryStubC::create_from_raw(ptr, len, ...)` -- memory from a raw buffer (`unsafe`)
- `MachineBuilder::new(config).bios(..).vga_bios(..)...build_at(storage, cpus, mem_stub)` -- the machine in caller-provided `MaybeUninit<Emulator<T>>` storage (no-alloc builds only)

## Adding new instructions

1. Add the `Opcode` variant in `rusty_box_decoder/src/opcode.rs`. Legacy and VEX entries go by hand into the maps under `rusty_box_decoder/src/decoder/` (`opmap.rs`, `opmap_0f38.rs`, `opmap_0f3a.rs`, and `x87.rs` for x87 and 3DNow!). EVEX entries are never hand-edited: regenerate `opmap_evex.rs` and `evex_operands.rs` with `python scripts/gen_opmap_evex.py` and `python scripts/gen_evex_operands.py`.
2. Regenerate the per-opcode ISA gate with `python scripts/gen_opcode_isa.py`. It rewrites `rusty_box_decoder/src/opcode_isa.rs`, which must never be edited by hand. The other `scripts/gen_*.py` generators transcribe decoder tables from the Bochs source the same way.
3. Implement the handler in the matching `rusty_box/src/cpu/<category>.rs` file, mirroring the Bochs handler it ports.
4. Add its arm to the `Opcode` match in `rusty_box/src/cpu/dispatcher.rs`.
5. Add a unit test that executes the instruction through `TestMachine::ctx()` / `exec_with` (`rusty_box/src/cpu/exec_ctx.rs`) and asserts guest-visible state (R9).
6. Run `cargo xtask ci`.

## Adding new I/O devices

1. A model that needs no CPU, scheduler or bus belongs in `rusty_box_devices` (no_std, `forbid(unsafe_code)`). `PioDevice`, the trait a device implements, and `TimerService`, which it arms its timers through (`DeviceCtx::timers`), are defined in `rusty_box_devices/src/api.rs`. Devices wired directly into the machine live in `rusty_box/src/iodev/`.
2. Give the device fixed-size fields (no `Vec`/`String`; use arrays and `&'static str`).
3. Add it to `DeviceManager` in `rusty_box/src/iodev/devices.rs`.
4. Add a `DevSlot` constant in `rusty_box/src/iodev/mod.rs` and register its ports with `register_io_handler(DevSlot::YOUR_DEVICE, port, "name", mask)`.
5. Route the slot: implement `PioDevice` and add an arm to `DeviceManager::bind_pio`.
6. Arm timers through the `TimerService` in the device context, not through `BxPcSystemC` directly.
7. Check the no-default-features build (`cargo check --release -p rusty_box --no-default-features`), then run `cargo xtask ci`.
