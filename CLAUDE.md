# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rusty Box is a Rust port of the Bochs x86 emulator - a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support (VMX/SVM). The original C++ Bochs source is in `cpp_orig/bochs/` for reference during porting.

## Current BIOS Execution Status

### ✅ MAJOR BREAKTHROUGH (2026-02-16): BIOS-bochs-latest Runs Successfully!

**DISCOVERY**: The "corrupted BIOS symbol addresses" bug was NOT in the BIOS ROM files - it was caused by two emulator bugs:

1. **Segment default bug**: `[BP+disp]` addressing modes were defaulting to DS instead of SS. Fixed by adding proper segment override lookup tables (`SREG_MOD00_RM16`, `SREG_MOD01OR10_RM16`, `SREG_MOD0_BASE32`, `SREG_MOD1OR2_BASE32`) in `fetchdecode32.rs`.

2. **execute1/execute2 mismatch**: 18 opcodes in `opcodes_table.rs` had memory-form (`_M`) and register-form (`_R`) handlers swapped, causing memory operands to be read from registers and vice versa.

**Current Status (2026-02-19):**
- ✅ BIOS-bochs-latest (128 KB) is now the primary BIOS
- ✅ Real mode BIOS executes ~362k instructions (keyboard init, memory probe, etc.)
- ✅ Transitions to protected mode via far jump (CS=0x10, flat memory model)
- ✅ rombios32 enters protected mode, _start executes with correct symbol addresses
- ✅ BSS clearing + .data copy complete correctly
- ✅ No unimplemented opcode errors
- ✅ Port 0x61 bit 4 toggle fix — `delay_ms()` in `smp_probe()` no longer infinite loops
- ✅ All hot-path logging fixed (no more debug!/info! on hot paths)
- ✅ dlxlinux.rs reads RUST_LOG env var (default WARN); use `RUSTY_BOX_HEADLESS=1` for headless
- ✅ Short jump sign-extension fix — byte displacements (0x70-0x7F, 0xEB, 0xE0-0xE3) now sign-extended
- ✅ All 17 missing Jbd dispatch variants added (JoJbd-JnleJbd, LoopJbd/LoopeJbd/LoopneJbd)
- ✅ CLC/STC/CMC flags implemented
- ✅ RDMSR/WRMSR stubs (return 0 / ignore)
- ✅ 1M instructions run cleanly at ~1.08 MIPS (final RIP=0xE1D81, CS=0x10, protected mode)
- ❌ No BIOS output yet — port 0x402 silent, VGA writes = 0 (BX_INFO not reached in 1M instructions)

**What Fixed the "Corrupted Symbols":**
The previous investigation concluded BIOS ROM had wrong symbol addresses. In reality, the segment default bug caused stack reads via `[BP+offset]` to use DS (base=0) instead of SS, and the execute1/execute2 swap caused memory reads to return register values. Together, these made the BIOS load wrong values for `_end`, `__data_start`, etc. With both bugs fixed, the BIOS reads correct values from the stack and memory.

### Investigation History: Protected Mode Init (2026-02-17 to 2026-02-19)

**Execution timeline (measured by instruction count):**
- 0-10: Real mode BIOS at F000:E0xx (initial setup)
- 10-100: Drops into low-address subroutines (F000:0Cxx area = keyboard/PCI init)
- ~360k: Real-mode init completes, BIOS enters protected mode
- At 362k: RIP=0xE08C0, CS=0x0010, mode=protected - rombios32 executing
- At ~363k+: Continues executing in protected mode

**Log flooding bug found and fixed (2026-02-17):**
The apparent "hang" at 363k instructions was caused by `tracing::debug!` in `misc_mem.rs` and `memory_stub.rs` logging every byte written beyond 32MB RAM. Changed to `tracing::trace!`.

**I/O port tracking added (2026-02-17):**
`BxDevicesC::inp()` now tracks the last I/O read port/value (`last_io_read_port`, `last_io_read_value`). The stuck-loop detector in `emulator.rs` reports this info. Signature changed from `&self` to `&mut self`.

**BIOS ROM shadow mapping bug found (partially fixed, 2026-02-17):**
The `get_host_mem_addr` PCI path for addresses 0xE0000-0xFFFFF was using the expansion ROM formula instead of `bios_map_last128k()`. Fixed.

**Root cause of "no BIOS output" found (2026-02-19): Port 0x61 delay_ms() infinite loop**

The Bochs BIOS `rombios32_init()` calls `smp_probe()` at line 2589 (after `BX_INFO` at 2576, `ram_probe` at 2583, `cpu_probe`, `setup_mtrr`). `smp_probe()` calls `delay_ms(10)`, which polls port 0x61 bit 4 (PIT channel 2 output) waiting for 66 edge transitions. Our emulator returned fixed `0x10` from port 0x61 — bit 4 never toggled → `delay_ms()` looped forever.

The two-part explanation for "no BIOS output":
1. **Performance**: Before logging fixes, debug flood made the emulator too slow to execute enough instructions to reach rombios32_init at all
2. **Correctness**: After logging fixes made it fast enough, the emulator reached rombios32_init and its BX_INFO calls, but then got stuck in `smp_probe()` → `delay_ms()` — the BIOS couldn't continue to print more output or do any useful work

**Fix**: `keyboard.rs` `SYSTEM_CONTROL_B` read handler now XORs bit 4 on each read:
```rust
self.system_control_b ^= 0x10;
```

**Hot-path logging fixed (2026-02-19):**
Multiple `debug!`/`info!` calls on hot paths were causing I/O-bound slowdowns:
- `cpu.rs`: `get_icache_entry` (every instruction) changed from `debug!` → `trace!`
- `cpu.rs`: Two `prefetch` messages changed from `info!` → `debug!`
- `stack.rs`: `PUSH16` message changed from `info!` → `debug!`
- `dlxlinux.rs`: Hardcoded `Level::DEBUG` replaced with `RUST_LOG` env var (default WARN)

**Note**: `tracing_subscriber::EnvFilter` requires the `env-filter` feature (not enabled).
Use `std::env::var("RUST_LOG").parse::<tracing::Level>()` instead.

**For headless testing on Windows**: Set `RUSTY_BOX_HEADLESS=1` to skip TermGUI repaint.
Performance: ~1.21 MIPS (100k instructions in 0.083s).

**New fixes (2026-02-19): Short jumps, CLC/STC/CMC, RDMSR/WRMSR, Jbd dispatch**

These bugs were causing an infinite loop at ~363k instructions and crashes in the first few hundred instructions of protected-mode execution:

1. **Short jump sign-extension** (`fetchdecode32.rs:586`): byte immediates for opcodes 0x70-0x7F, 0xEB, 0xE0-0xE3 were zero-extended. `jmp_jd` uses `instr.id() as i32` so 0xFE → 254 instead of -2. Fixed by sign-extending for branch opcodes only.
2. **Missing Jbd dispatch** (`cpu.rs`): Only `JmpJbd`, `JzJbd`, `JnzJbd`, `JecxzJbd` were handled. Added JoJbd, JnoJbd, JbJbd, JnbJbd, JbeJbd, JnbeJbd, JsJbd, JnsJbd, JpJbd, JnpJbd, JlJbd, JnlJbd, JleJbd, JnleJbd, LoopJbd, LoopeJbd, LoopneJbd.
3. **CLC/STC/CMC** (`cpu.rs`): Clear/Set/Complement CF flag — first crash after short-jump fix (opcode 0xF8 at protected mode entry). Added near Hlt/Cpuid.
4. **RDMSR/WRMSR stubs** (`cpu.rs`): Called by `setup_mtrr()` in rombios32_init. Return 0/ignore writes.
5. **mpool_start_idx fallback removed** (`cpu.rs`): Was emitting `warn!` on every first-trace icache lookup (index 0 is valid for the first cached trace). Removing the false-error code improved performance.

**Result**: BIOS now runs 1M instructions at ~1.08 MIPS without crashing. Final RIP=0xE1D81 (still in protected mode). Still no BIOS output — need to trace why `BX_INFO("Starting rombios32\n")` hasn't fired.

**Next investigation**: The BIOS spends 1M instructions in protected mode but never reaches `rombios32_init()` BX_INFO at the start. Possible causes:
- Long setup_mtrr/pci init before rombios32_init is called
- Some loop/spin consuming instructions before the BX_INFO point
- Run with RUST_LOG=debug to see RDMSR/WRMSR calls and trace what's happening

### BIOS Binary Analysis (2026-02-23)

**Confirmed BIOS layout** (128KB = file 0x0000-0x1FFFF = physical 0xFFFE0000-0xFFFFFFFF):
- File 0x0000 = phys 0xE0000: rombios32 _start (BSS clear, .data copy, JMP to rombios32_init)
- File 0x2980 = phys 0xE2980: `rombios32_init()` — first function called in 32-bit PM
- File 0x0B98 = phys 0xE0B98: `bios_printf()` — writes ALL formatted bytes to port 0x402
- File 0x075C = phys 0xE075C: `vsnprintf()` — called by bios_printf to format strings
- File 0x17F4 = phys 0xE17F4: `delay_ms()` — polls port 0x61 bit 4 (66 transitions/ms)
- File 0x1D3A = phys 0xE1D3A: `smp_probe()` — APIC check + AP trampoline copy + IPI + delay_ms
- File 0x1D74 = phys 0xE1D74: smp_probe copy loop (74 bytes, 0xE0028 → 0x9F000)
- Real-mode code: file 0x8000-0x1FFFF (16-bit code segment)

**True PM entry sequence** (not 0xE08C0 as previously thought):
```
Real-mode BIOS (~362K instr):
  → F000:XXXX: LGDT [rombios32_gdt_48]; MOV CR0, EAX; FAR JMP 0x10:0xF9E5F
  → phys 0xF9E5F (file 0x19E5F): PM setup (MOV DS/ES/SS=0x18, FS/GS=0; set stack)
  → PUSH 0x4B0; PUSH 0x4B2; MOV EAX, 0xE0000; CALL EAX (_start)
  → phys 0xE0000 (_start): XOR EAX; REP STOSB (BSS 88B); REP MOVSB (.data 12B)
  → JMP 0xE2980 (rombios32_init)
rombios32_init (0xE2980):
  1. bios_printf(4, "Starting rombios32\n")    ← first ASCII to port 0x402
  2. bios_printf(4, "Shutdown flag %x\n", ...)
  3. ram_probe() — CMOS reads for memory size
  4. cpu_probe() — CPUID
  5. setup_mtrr() — RDMSR/WRMSR (wrmsr stubs in emulator)
  6. smp_probe() — APIC check, 74-byte AP copy, INIT/SIPI IPI, delay_ms(10)
     → bios_printf("Found %d cpu(s)\n", num_cpus)
  7. find_bios_table_area()
  8. pci_bios_init()
```

**GDT (rombios32_gdt at line 10698 of rombios.c)**:
```c
// selector 0x10: 32-bit flat code  (base=0, limit=4GB, D=1, G=1)
dw 0xffff, 0, 0x9b00, 0x00cf
// selector 0x18: 32-bit flat data  (base=0, limit=4GB, D=1, G=1)
dw 0xffff, 0, 0x9300, 0x00cf
```
D=1 confirmed in bit 22 of dword2 (0x00CF... → bit 22 = 1). Decoder correctly reads CS.d_b.

**bios_printf port 0x402 behavior**: Port 0x402 is ONLY written from a single loop at file 0x0BE9. bios_printf(rombios32.c) ALWAYS writes ALL formatted chars to port 0x402, regardless of the `flags` argument. No flag gate before the output loop.

**smp_probe loop analysis** (ending at RIP=0xE1D81):
```asm
; EAX starts at 0x9F000, ECX = 0x9F04A (end), 74 iterations
0xE1D74: LEA EDX, [EAX+1]
0xE1D77: MOV BL, [EAX + 0x41028]    ; read from ROM (0xE0028..0xE0071)
0xE1D7D: MOV [EAX], BL              ; write to RAM (0x9F000..0x9F049)
0xE1D7F: MOV EAX, EDX
0xE1D81: CMP EDX, ECX
0xE1D83: JNZ → 0xE1D74
```
This copies the AP startup trampoline from ROM to RAM. After the copy, smp_probe sends INIT IPI + SIPI + delay_ms(10) + reports CPU count.

**Outstanding: 0xB2 and 0xFF at port 0x402** (in 500K instruction debug run):
- Only 2 writes seen (both non-ASCII) — suggests real-mode bios_printf before PM entry
- OR emulator reaches rombios32_init but vsnprintf produces wrong output
- Port 0x402 verified: only accessed from bios_printf at file 0x0BE9 (single OUT DX,AL loop)
- rombios.c's 16-bit bios_printf: only writes to 0x402 if `action & BIOS_PRINTF_INFO` set
- Possible source: `BX_INFO("BIOS BUILD DATE: %s\n", ...)` in real-mode at ~362K instructions
- Need to run with RIP logging on port 0x402 writes to pinpoint source

## Known Issues & Next Steps

### Next Steps
1. **Diagnose 0xB2 and 0xFF at port 0x402** — Add RIP logging to port_out for 0x402, then find what instruction writes these non-ASCII values. Are they from real-mode bios_printf or PM vsnprintf?
2. **Verify rombios32_init output reaches port 0x402** — Run 1M+ instructions with debug logging and check for ASCII chars at 0x402 (should see 'S'=0x53 from "Starting rombios32\n")
3. **Implement remaining instructions** — As discovered by running the emulator further
4. **Boot sector loading** — Once BIOS completes POST, load and execute boot sector

### Quick Debug Commands
```bash
# Build release binary
cargo build --release --example dlxlinux --features std

# Run headless (fast) with default WARN logging
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=2000000 ./target/release/examples/dlxlinux.exe

# Run with debug logging to see port 0x402 writes (and port 0x80 POST codes)
RUST_LOG=debug RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=500000 ./target/release/examples/dlxlinux.exe 2>&1 | grep -E "0x0402|0x0080|port_out.*402|BIOS output"

# Check BIOS output buffer drain in emulator summary
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=1000000 ./target/release/examples/dlxlinux.exe 2>&1
```

### Progress Metrics
- ✅ All major decoder bugs fixed (Group 1 opcodes, segment defaults, execute1/execute2)
- ✅ Protected mode transition works (far jump, GDT, segment loading)
- ✅ rombios32 initialization executes with correct symbols
- ✅ No unimplemented opcode errors (all needed opcodes implemented)
- ✅ Extensive instruction set coverage (arithmetic, logical, shift, rotate, control flow, data transfer, string ops)
- ✅ Log flooding fix: all hot-path messages at correct trace!/debug! level
- ✅ I/O port tracking: last read port/value reported in stuck-loop diagnostics
- ✅ Inner loop instruction limit check prevents single-batch hangs
- ✅ Port 0x61 bit 4 toggle fix: delay_ms() now terminates (keyboard.rs)
- ✅ REP STOSB/MOVSB 32-bit dispatch fixed (string.rs)
- ✅ dlxlinux.rs reads RUST_LOG env var; RUSTY_BOX_HEADLESS=1 for headless runs
- ✅ Short jump sign-extension fix: byte branch displacements now sign-extended in decoder
- ✅ All 17 missing Jbd dispatch variants added to cpu.rs
- ✅ CLC/STC/CMC flag instructions implemented
- ✅ RDMSR/WRMSR stubs implemented
- ✅ Runs 1M instructions cleanly at ~1.08 MIPS (no crashes)
- ❌ BIOS text output still pending — BX_INFO("Starting rombios32\n") not reached in 1M instructions

## Build Commands

```bash
# Build everything
cargo build --all-features

# Build with release optimizations (needed for acceptable performance)
cargo build --release --all-features

# Run tests
cargo test

# Run a single test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run examples (require --release for large stack)
cargo run --release --example init_and_run
cargo run --release --example dlxlinux --features std
cargo run --release --example dlxlinux --features "std,gui-egui"

# Fuzz the decoder
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_target_1
```

## Workspace Structure

- **rusty_box/**: Main emulator library
- **rusty_box_decoder/**: Separate crate for x86 instruction decoding (allows fuzzing and reuse)
- **cpp_orig/bochs/**: Original C++ Bochs source for reference

## Architecture

### No Global State
The emulator uses instance-based architecture. Each `Emulator<I>` is completely self-contained, allowing multiple independent emulator instances to run concurrently.

### Core Components

```
Emulator<'a, I: BxCpuIdTrait>
├── BxCpuC<I>         CPU (generic over CPUID model like Corei7SkylakeX)
├── BxMemC            Memory subsystem (block-based, supports >4GB)
├── BxDevicesC        I/O port handler manager
├── DeviceManager     Hardware devices (PIC, PIT, CMOS, DMA, VGA, Keyboard, HardDrive)
├── BxPcSystemC       Timers and A20 line control
└── GUI               Display (NoGui, TermGui, or EguiGui)
```

### Initialization Sequence

```rust
let config = EmulatorConfig::default();
let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;
emu.initialize()?;                    // Init PC system, memory, devices, CPU
emu.load_bios(&bios_data, 0xfffe0000)?;
emu.load_optional_rom(&vga_bios, 0xc0000)?;
emu.reset(ResetReason::Hardware)?;
emu.prepare_run();
emu.cpu.cpu_loop(&mut emu.memory, &[])?;
```

### CPU Module Organization

Instructions are organized by category (matching original Bochs cpp_orig/bochs/cpu/ structure):
- `cpu/arith/`: ADD, SUB, ADC, SBB, DEC, INC (arith8.rs, arith16.rs, arith32.rs)
- `cpu/logical*/`: AND, OR, XOR, NOT (8/16/32/64-bit variants)
- `cpu/mult*/`: MUL, IMUL (8/16/32/64-bit variants)
- `cpu/shift.rs`: SHL, SHR, SAR, ROR, ROL
- `cpu/ctrl_xfer*/`: JMP, CALL, RET, loops (ctrl_xfer16.rs, ctrl_xfer32.rs, ctrl_xfer64.rs)
- `cpu/data_xfer/`: MOV, LEA, XCHG (data_xfer8.rs, data_xfer16.rs, data_xfer32.rs, data_xfer64.rs)
- `cpu/stack.rs`: Common stack primitives (push_16/32, pop_16/32, stack memory access)
- `cpu/stack16.rs`: 16-bit stack ops (PUSH/POP r16, PUSHA16, POPA16, PUSHF, POPF)
- `cpu/stack32.rs`: 32-bit stack ops (PUSH/POP r32, PUSHAD, POPAD, PUSHFD, POPFD)
- `cpu/stack64.rs`: 64-bit stack ops (PUSH/POP r64, PUSHFQ, POPFQ)
- `cpu/string.rs`: MOVSB, STOSB, LODSB, REP string operations
- `cpu/io.rs`: IN, OUT, INS, OUTS
- `cpu/soft_int.rs`: INT, IRET, INTO, BOUND, HLT

### CPU State Access

```rust
// Read-only getters
cpu.rax()      // u64 register value
cpu.rip()      // instruction pointer
cpu.eflags()   // flags register

// Setters
cpu.set_rax(0x777)
cpu.set_rip(0)
```

### Decoder Usage

```rust
// 32-bit mode
let instr = fetch_decode32_chatgpt_generated_instr(&bytes, is_32_bit_mode)?;

// 64-bit mode
let instr = fetch_decode64(&bytes)?;

// Const (compile-time) decoding
const NOP: BxInstructionGenerated = const_fetch_decode64(&[0x90]).unwrap();

// Access decoded instruction data
instr.dst()           // destination register
instr.src1()          // source register 1
instr.ib()            // 8-bit immediate
instr.id()            // 32-bit immediate
instr.get_ia_opcode() // decoded opcode
instr.ilen()          // instruction length
```

### Decoder Validation

The decoder performs validation to ensure only valid x86 encodings are produced:

- **Segment register indices** must be 0-5 (ES, CS, SS, DS, FS, GS)
- Invalid segment register indices (6-7) cause `DecodeError::InvalidSegmentRegister`
- This prevents undefined behavior and catches decoder bugs early
- See `docs/DECODER_BUGS.md` for historical bug fixes and validation details

### Memory Layout

- **0x00000-0x9FFFF**: Conventional memory (640KB)
- **0xA0000-0xBFFFF**: VGA memory
- **0xC0000-0xDFFFF**: Expansion ROM (128KB)
- **0xE0000-0xFFFFF**: BIOS ROM area (128KB)
- **0xFFF80000-0xFFFFFFFF**: System ROM (512KB BIOS)

### I/O Device Registration

Devices register port handlers during init. Each port (0x0000-0xFFFF) can have read/write handlers:
- PIC: 0x20-0x21, 0xA0-0xA1
- PIT: 0x40-0x43
- Keyboard: 0x60, 0x64
- CMOS: 0x70-0x71
- VGA: 0x3B0-0x3DF
- IDE: 0x1F0-0x1F7, 0x3F6-0x3F7
- System Control (A20/reset): 0x92

## Feature Flags

Key Cargo features in `rusty_box/Cargo.toml`:
- `std`: Standard library support (terminal, file I/O)
- `gui-egui`: Graphical UI using egui
- `bx_full`: Enables all emulation features (default)
- `bx_little_endian` / `bx_big_endian`: Endianness (mutually exclusive)
- `bx_phy_address_long`: >4GB physical address support
- `bx_support_apic`: APIC support
- `bx_support_pci`: PCI bus support

## Key Files for Common Tasks

| Task | Files |
|------|-------|
| Add new instruction | `rusty_box_decoder/src/fetchdecode*.rs`, `cpu/<category>/` |
| Add new I/O device | `iodev/` (new file), `iodev/devices.rs` (registration) |
| Modify memory mapping | `memory/misc_mem.rs`, `memory/mod.rs` |
| Add CPUID model | `cpu/cpuid/` |
| Debug execution | Enable tracing: `Level::DEBUG` or `Level::TRACE` in examples |

## Error Handling

Uses `thiserror` with root `Error` enum in `src/error.rs` aggregating:
- `CpuError`: CPU execution errors, unimplemented instructions
- `MemoryError`: Memory access errors
- `DecodeError`: Instruction decoding errors

## Platform Notes

- Uses `OnceLock` (std) or `spin::once::Once` (no_std) for singletons
- Examples require large stack (500MB-1.5GB) - spawned on dedicated thread
- Register layout in `BxGenReg` union differs by endianness feature flag

## Known Issues

### No BIOS Output — Root Cause Found and Fixed (2026-02-19)

**Status:** Fix applied, pending verification

**Root cause**: `rombios32_init()` calls `smp_probe()` before any `BX_INFO()` output. `smp_probe()` calls `delay_ms(10)`, which polls port 0x61 bit 4 (PIT channel 2 output) waiting for 66 edge transitions. Our emulator returned a fixed `0x10` from port 0x61 — bit 4 never toggled — so `delay_ms()` was an infinite loop. The BIOS was alive and executing, just stuck before any output code.

**Fix**: `keyboard.rs` SYSTEM_CONTROL_B (port 0x61) read handler now XORs bit 4 on each read: `self.system_control_b ^= 0x10;`

**rombios32_init() call order** (from `cpp_orig/bochs/bios/rombios32.c:2574`):
1. `BX_INFO("Starting rombios32\n")` — line 2576, first output to port 0x402
2. `BX_INFO("Shutdown flag %x\n", ...)` — line 2577
3. `ram_probe()` — detects memory size, outputs more BX_INFO
4. `cpu_probe()` — detect CPU features
5. `setup_mtrr()` — RDMSR/WRMSR (needs MSR support)
6. `smp_probe()` — **HERE** was the delay_ms() infinite loop ← line 2589
7. `find_bios_table_area()`
8. `pci_bios_init()` — PCI enumeration

**Output chain**: `BX_INFO` → `bios_printf` (rombios32.c:354) → `putch()` (line 109) → `outb(INFO_PORT=0x402, c)` → captured in `iodev/mod.rs:port_e9_output` → drained in `emulator.rs` run loop and in `dlxlinux.rs` at end-of-run.

Since BX_INFO is called before smp_probe(), "Starting rombios32\n" was being written to port 0x402 before the delay_ms() hang — it may have been buffered but not printed due to how the stuck-loop detection interacted with the drain. After the port 0x61 fix, the BIOS should proceed through smp_probe() to pci_bios_init() and beyond.

### BIOS ROM Shadow Mapping (2026-02-17)

**Status:** Partially fixed

Found that `get_host_mem_addr` PCI path for 0xE0000-0xFFFFF used wrong ROM offset formula. Fixed to use `bios_map_last128k()` which maps shadow addresses to the last 128KB of the 4MB ROM array. Three locations in `misc_mem.rs` were corrected. The real-mode BIOS execution was NOT affected (it took an earlier correct code path), but protected-mode code that accesses the BIOS shadow area now gets correct data.

### Decoder Bug: Group 3a/3b Immediate Size (2026-02-02)

**Status:** Identified, not yet fixed (may no longer be hit with current BIOS path)

The decoder fails to account for immediate bytes in TEST instructions (opcodes 0xF6 and 0xF7 with ModRM.nnn=0 or 1). Impact: instruction length miscalculation causes RIP misalignment. This was the original cause of BIOS crashes at 0xe1d59 with the legacy BIOS, but may not be triggered by the current BIOS-bochs-latest execution path.

### Exception Handling (2026-02-02)

**Status:** Partially implemented

Exception handling infrastructure exists (Exception enum, IVT delivery in real mode). Protected mode IDT delivery needs work - currently fails with `BadVector` when IDT limit=0.

### Major Bug Fixes (Historical)

1. **Port 0x61 delay_ms fix (2026-02-19)**: `keyboard.rs` port 0x61 bit 4 now toggles on each read. Previously always returned `0x10`, causing `delay_ms()` in `smp_probe()` to loop infinitely — BIOS never produced output.
2. **Hot-path logging fix (2026-02-19)**: `cpu.rs` `get_icache_entry` (every instruction) changed from `debug!` to `trace!`. Two `prefetch` messages changed from `info!` to `debug!`. `stack.rs` PUSH16 from `info!` to `debug!`. `dlxlinux.rs` now reads `RUST_LOG` env var (default WARN) instead of hardcoding `Level::DEBUG`.
3. **REP STOSB/MOVSB 32-bit fix (2026-02-19)**: `RepStosbYbAl` and `RepMovsbYbXb` now dispatch to 32-bit variants (`rep_stosb32`, `rep_movsb32`) when `instr.as32_l() != 0`, matching how STOSD/MOVSD already worked.
4. **Log flooding fix (2026-02-17)**: Out-of-bounds memory write messages (`misc_mem.rs`, `memory_stub.rs`) downgraded from `debug!` to `trace!`.
5. **Segment default fix (2026-02-16)**: `[BP+disp]` was using DS instead of SS. Fixed with lookup tables in `fetchdecode32.rs`.
6. **execute1/execute2 fix (2026-02-16)**: 18 opcodes had memory/register handler forms swapped in `opcodes_table.rs`.
7. **Group 1 decoder fix (2026-02-02)**: ModRM `reg` field stored instead of `r/m` for opcodes 0x80/0x81/0x83.
8. **BIOS load address fix (2026-02-07)**: Address calculated from BIOS size instead of hardcoded.
9. **Memory allocation fix (2026-02-06)**: `vec![0; size]` instead of loop-based `push()` for large allocations.
