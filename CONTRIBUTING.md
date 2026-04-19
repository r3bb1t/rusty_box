# Contributing to Rusty Box

## Build & Test

```bash
# Full build
cargo build --release --all-features

# Run tests (187 tests)
cargo test

# no_std + no_alloc build (full emulator, no heap)
cargo check -p rusty_box --no-default-features

# no_std + alloc (adds Emulator::new(), GUI, diagnostic methods)
cargo check -p rusty_box --no-default-features --features alloc

# UEFI target (no allocator)
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi

# Fuzz the decoder
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_target_1
```

## Architecture

The emulator is organized as a Cargo workspace with four crates:

- **rusty_box** -- main emulator library (CPU, memory, I/O devices)
- **rusty_box_decoder** -- standalone x86 instruction decoder
- **rusty_box_web** -- WASM web frontend (in `examples/rusty_box_web/`)
- **rusty_box_uefi** -- UEFI bootable emulator (in `examples/rusty_box_uefi/`)

### Code Organization

CPU instructions are organized by category, mirroring the Bochs `cpu/` directory:

| Category | Files |
|----------|-------|
| Arithmetic | `arith8.rs`, `arith16.rs`, `arith32.rs`, `arith64.rs` |
| Logical | `logical8.rs`, `logical16.rs`, `logical32.rs`, `logical64.rs` |
| Data transfer | `data_xfer8.rs`, `data_xfer16.rs`, `data_xfer32.rs`, `data_xfer64.rs` |
| Control flow | `ctrl_xfer16.rs`, `ctrl_xfer32.rs`, `ctrl_xfer64.rs` |
| Stack | `stack.rs`, `stack16.rs`, `stack32.rs`, `stack64.rs` |
| FPU | `fpu/*.rs` (handlers) + `softfloat3e/*.rs` (math library) |

### no_alloc Design

The core emulator compiles without `alloc`. All I/O device state uses fixed-size types:

| Heap type | Replacement |
|-----------|-------------|
| `Vec<T>` | `[T; N]` (compile-time sized arrays) |
| `VecDeque<T>` | `RingBuffer<T, N>` (fixed-capacity ring buffer) |
| `String` | `&'static str` or `[u8; N]` + length |
| `Box<dyn BxGui>` | gated behind `#[cfg(feature = "alloc")]` |
| `Arc<AtomicBool>` | `AtomicBool` (no-alloc) / `Arc<AtomicBool>` (alloc) |

Construction without alloc uses placement initialization:
- `BxCpuBuilder::init_cpu_at(ptr, tracer)` -- CPU at caller-provided memory
- `Emulator::init_at(ptr, cpu, mem_stub, config)` -- Emulator at caller-provided memory
- `BxMemoryStubC::create_from_raw(ptr, len, ...)` -- Memory from raw buffer

### Key Design Rules

1. **Match Bochs exactly** -- all logic must correspond to the Bochs C++ source. The original source is in `cpp_orig/bochs/` for reference. Structural deviations are bugs.
2. **No global state** -- the emulator is fully instance-based. No static mutables.
3. **no_std + no_alloc first** -- core emulation compiles without std or alloc. Heap-dependent features (GUI, diagnostic String returns, StopHandle) are behind `#[cfg(feature = "alloc")]`.

## Testing Requirements

After any code change, test **both** boot targets:

```bash
# DLX Linux (fast -- ~10 seconds)
RUSTY_BOX_HEADLESS=1 cargo run --release --example dlxlinux --features std

# Alpine Linux (slower -- ~60 seconds)
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=3500000000 cargo run --release --example alpine_direct --features std 2>/dev/null
```

DLX alone is insufficient -- many bugs only manifest in Alpine's 64-bit kernel.

Also verify no-alloc and UEFI builds:

```bash
cargo check -p rusty_box --no-default-features
cargo check -p rusty_box_uefi --target x86_64-unknown-uefi
```

## Adding New Instructions

1. Add the decoder entry in `rusty_box_decoder/src/fetchdecode*.rs`
2. Implement the handler in the appropriate `rusty_box/src/cpu/<category>.rs` file
3. Wire the dispatcher in `rusty_box/src/cpu/dispatcher.rs`
4. Test with both DLX and Alpine boot

## Adding New I/O Devices

1. Create a new file in `rusty_box/src/iodev/`
2. Add the device struct with fixed-size fields (no Vec/String -- use arrays and `&'static str`)
3. Add it to `DeviceManager` in `rusty_box/src/iodev/devices.rs`
4. Register port handlers via `register_io_handler(DeviceId::YourDevice, port, "name", mask)`
5. Wire timer callbacks through `BxPcSystemC` if needed
6. Verify both `--all-features` and `--no-default-features` compile
