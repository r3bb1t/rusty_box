# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rusty Box is a Rust port of the Bochs x86 emulator - a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support (VMX/SVM). The original C++ Bochs source is in `cpp_orig/bochs/` for reference during porting.

## Current BIOS Execution Status

### ✅ MAJOR BREAKTHROUGH (2026-02-16): BIOS-bochs-latest Runs Successfully!

**DISCOVERY**: The "corrupted BIOS symbol addresses" bug was NOT in the BIOS ROM files - it was caused by two emulator bugs:

1. **Segment default bug**: `[BP+disp]` addressing modes were defaulting to DS instead of SS. Fixed by adding proper segment override lookup tables (`SREG_MOD00_RM16`, `SREG_MOD01OR10_RM16`, `SREG_MOD0_BASE32`, `SREG_MOD1OR2_BASE32`) in `fetchdecode32.rs`.

2. **execute1/execute2 mismatch**: 18 opcodes in `opcodes_table.rs` had memory-form (`_M`) and register-form (`_R`) handlers swapped, causing memory operands to be read from registers and vice versa.

**Current Status:**
- ✅ BIOS-bochs-latest (128 KB) is now the primary BIOS
- ✅ Real mode BIOS executes ~362k instructions (keyboard init, memory probe, etc.)
- ✅ Transitions to protected mode via far jump (CS=0x10, flat memory model)
- ✅ rombios32 enters protected mode, _start executes with correct symbol addresses
- ✅ BSS clearing + .data copy complete correctly
- ✅ No unimplemented opcode errors
- ❌ **Stuck in infinite loop** at ~363k instructions (RIP=0xE08C0, protected mode)
- ❌ No BIOS output (ports 0xE9, 0x402, 0x403 silent; VGA memory untouched)

**What Fixed the "Corrupted Symbols":**
The previous investigation concluded BIOS ROM had wrong symbol addresses. In reality, the segment default bug caused stack reads via `[BP+offset]` to use DS (base=0) instead of SS, and the execute1/execute2 swap caused memory reads to return register values. Together, these made the BIOS load wrong values for `_end`, `__data_start`, etc. With both bugs fixed, the BIOS reads correct values from the stack and memory.

### Current Investigation: Infinite Loop at Protected Mode Entry (2026-02-17)

**Execution timeline (measured by instruction count):**
- 0-10: Real mode BIOS at F000:E0xx (initial setup)
- 10-100: Drops into low-address subroutines (F000:0Cxx area = keyboard/PCI init)
- ~360k: Real-mode init completes, BIOS enters protected mode
- At 362k: RIP=0xE08C0, CS=0x0010, mode=protected - rombios32 executing
- At ~363k: **Stuck in infinite loop** - likely polling an I/O port that never responds

**BIOS ROM shadow mapping bug found (partially fixed):**
The `get_host_mem_addr` PCI path for addresses 0xE0000-0xFFFFF was using the expansion ROM formula (`a20_addr & EXROM_MASK + BIOSROMSZ`) instead of `bios_map_last128k()`. Fixed to correctly distinguish 0xE0000-0xFFFFF (BIOS shadow) from 0xC0000-0xDFFFF (expansion ROM).

**Next step:** Add I/O port access logging around instruction 362k to identify which port the BIOS is polling in the infinite loop. Common culprits: PCI config space (0xCFC/0xCF8), CMOS (0x71 UIP flag), keyboard controller (0x64 status).

## Known Issues & Next Steps

### Next Steps
1. **Fix infinite loop at protected mode entry** - Add I/O port logging to identify the polling port, then implement proper device response
2. **Implement remaining instructions** - As discovered by running the emulator further
3. **Boot sector loading** - Once BIOS completes POST, load and execute boot sector

### Progress Metrics
- ✅ All major decoder bugs fixed (Group 1 opcodes, segment defaults, execute1/execute2)
- ✅ Protected mode transition works (far jump, GDT, segment loading)
- ✅ rombios32 initialization executes with correct symbols
- ✅ No unimplemented opcode errors (all needed opcodes implemented)
- ✅ Extensive instruction set coverage (arithmetic, logical, shift, rotate, control flow, data transfer, string ops)
- ✅ Debug instrumentation cleaned up (no more WARN/ERROR spam in hot paths)
- ❌ BIOS enters infinite loop immediately after entering protected mode
- ❌ No BIOS text output yet

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

### Infinite Loop After Protected Mode Entry (2026-02-17)

**Status:** Under investigation

BIOS enters protected mode at ~362k instructions (CS=0x10, RIP=0xE08C0) but immediately enters an infinite loop at ~363k instructions. The loop is likely polling an I/O port. Need to add I/O port access logging to identify the port and implement proper device response.

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

1. **Segment default fix (2026-02-16)**: `[BP+disp]` was using DS instead of SS. Fixed with lookup tables in `fetchdecode32.rs`.
2. **execute1/execute2 fix (2026-02-16)**: 18 opcodes had memory/register handler forms swapped in `opcodes_table.rs`.
3. **Group 1 decoder fix (2026-02-02)**: ModRM `reg` field stored instead of `r/m` for opcodes 0x80/0x81/0x83.
4. **BIOS load address fix (2026-02-07)**: Address calculated from BIOS size instead of hardcoded.
5. **Memory allocation fix (2026-02-06)**: `vec![0; size]` instead of loop-based `push()` for large allocations.
