# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rusty Box is a Rust port of the Bochs x86 emulator - a complete CPU/system emulator targeting 32/64-bit x86 architecture with virtualization support (VMX/SVM). The original C++ Bochs source is in `cpp_orig/bochs/` for reference during porting.

## Current BIOS Execution Status

As of 2026-02-02, the emulator successfully executes BIOS code in protected mode:

- ✅ Protected mode entry working (CS.base=0x0, GDT loaded correctly)
- ✅ CRITICAL FIX: Decoder bug fixed - opcodes 0x80, 0x81, 0x83 (Group 1) now correctly recognized
- ✅ Stack corruption resolved - function calls/returns work correctly
- ✅ BIOS executes continuously - running for extended periods without errors
- ✅ HLT instruction properly halts CPU and returns control to emulator
- ✅ CPUID implemented - BIOS can query CPU features
- ✅ Multiple critical instructions implemented - MOVZX, SHLD/SHRD, MOV moffs, INC/DEC 8-bit
- ✅ **BIOS executes continuously and produces output!**
- ⚠️  Current limitation: Hits illegal opcode at RIP 0xe1d59 (may need exception handling)

### Major Fixes (2026-02-02)

**Decoder Bug Fix:**
- **Problem:** ModRM byte parsing incorrectly stored `reg` field instead of `r/m` field for Group 1 instructions (0x80, 0x81, 0x83)
- **Impact:** `SUB ESP, 0x400` was modifying EBP instead of ESP, causing complete stack corruption
- **Fix:** Added 0x80, 0x81, 0x83 to `is_group_opcode` list in both `fetchdecode32.rs` and `fetchdecode64.rs`
- **Result:** Function calls/returns now work correctly, BIOS executes much further

### Recently Implemented Instructions (2026-02-02)

**Data Transfer:**
- `MOVZX r32, r/m8` (MovzxGdEb) - Move byte to dword with zero extension
- `MOVZX r32, r/m16` (MovzxGdEw) - Move word to dword with zero extension
- `MOV EAX, moffs32` (MovEaxod) - Load EAX from memory at direct address
- `MOV moffs32, EAX` (MovOdEax) - Store EAX to memory at direct address

**Shift Instructions (Double Precision):**
- `SHLD r32, r32, imm8` (ShldEdGdIb) - Shift left double precision with immediate
- `SHLD r32, r32, CL` (ShldEdGd) - Shift left double precision with CL
- `SHRD r32, r32, imm8` (ShrdEdGdIb) - Shift right double precision with immediate
- `SHRD r32, r32, CL` (ShrdEdGd) - Shift right double precision with CL
- `SHR r32, imm8` (ShrEdIb) - Logical shift right with immediate

**Arithmetic (8-bit):**
- `INC r/m8` (IncEb) - Increment 8-bit register/memory by 1
- `DEC r/m8` (DecEb) - Decrement 8-bit register/memory by 1

**CPU Identification:**
- `CPUID` - Returns CPU vendor, family, and feature flags (basic implementation)

### Previously Implemented Instructions (2026-02-01)

**Stack Operations (32-bit):**
- `PUSH imm32` (PushId) - Push 32-bit immediate value onto stack
- `PUSH imm8` (PushSIb32) - Push sign-extended 8-bit immediate

**Arithmetic (32-bit):**
- `ADD r32, imm8` (AddEdsIb) - Add sign-extended 8-bit immediate to r32
- `ADD r32, imm32` (AddEdId) - Add 32-bit immediate to r32
- `SUB r32, imm8` (SubEdsIb) - Subtract sign-extended 8-bit immediate from r32
- `SUB r32, imm32` (SubEdId) - Subtract 32-bit immediate from r32
- `CMP r32, imm8` (CmpEdsIb) - Compare r32 with sign-extended 8-bit immediate

**Logical (32-bit):**
- `AND r32, imm8` (AndEdsIb) - Bitwise AND r32 with sign-extended 8-bit immediate

**Control Flow:**
- `JNZ rel8` (JnzJbd) - Jump if not zero (byte displacement)
- `JZ rel8` (JzJbd) - Jump if zero (byte displacement)

**Data Transfer:**
- `MOVSX r32, r/m8` (MovsxGdEb) - Move byte to dword with sign extension

## Known Issues & Next Steps

### Current Limitation
The BIOS executes successfully for an extended period but eventually hits an illegal opcode (FE /7) at RIP 0xe1d59. This appears to be either:
1. A control flow issue causing execution of data as code
2. Missing exception handling - real x86 would trigger #UD (Undefined Instruction) fault
3. A rare BIOS code path that expects exception handling

**BIOS Output Observed:**
```
[cpu_loop start] ea 5b e0 00 f0 30 35 2f
```
This indicates the BIOS is actively running and producing output!

### Next Steps
1. **Implement Exception Handling** - Add #UD (Undefined Instruction) fault support
2. **Add Execution Tracing** - Better logging to identify what causes jump to 0xe1d59
3. **Implement Remaining Common Instructions** - As they're discovered
4. **Boot Sector Loading** - Once BIOS completes POST, load and execute boot sector

### Progress Metrics
- ✅ Decoder bug fixed (Group 1 opcodes 0x80, 0x81, 0x83)
- ✅ Stack operations working correctly
- ✅ 10+ new instructions implemented in this session
- ✅ BIOS produces output to debug ports
- ✅ Executes continuously without crashes (until hitting illegal opcode)

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

### Decoder Bug: Group 3a/3b Immediate Size (2026-02-02)

**Status:** Identified, not yet fixed

The decoder fails to account for immediate bytes in TEST instructions (opcodes 0xF6 and 0xF7 with ModRM.nnn=0 or 1):

**Problem:** In `fetchdecode32.rs`, `get_immediate_size_32()` returns 0 for opcode 0xF6/0xF7, but TEST variants need a 1-byte immediate.

**Impact:** Instruction length miscalculation causes RIP misalignment. BIOS execution fails with "illegal opcode" at 0xe1d59 after hitting misaligned TEST instruction at 0xe1d44.

**Example:**
```
0xe1d44: f6 05 31 07 00 00 02  = TEST BYTE PTR [0x731], 0x02
Correct length: 7 bytes (opcode + ModRM + disp32 + imm8)
Decoder calculates: 6 bytes (missing immediate!)
```

**Fix:** Add 0xF6/0xF7 to immediate size handling, with conditional check for ModRM.nnn field (only /0 and /1 have immediates).

**See:** `DECODER_BUG_F6_IMMEDIATE.md` for detailed analysis

### Exception Handling (2026-02-02)

**Status:** Partially implemented

Exception handling infrastructure added:
- `Exception` enum with all x86 exception vectors (cpu.rs:237)
- `exception()` method to generate exceptions (exception.rs:243)
- `#UD` (Undefined Instruction) exception detection in icache (icache.rs:630)
- Real mode exception delivery via IVT works
- Protected mode requires IDT to be initialized

**Current limitation:** When BIOS runs in protected mode with uninitialized IDT (limit=0), exception delivery fails with `BadVector` error. Need to either:
1. Implement IDT fallback to real mode IVT
2. Allow exceptions to proceed even with IDT.limit=0
3. Fix the underlying decoder bug so exceptions aren't triggered

### BIOS Output

**Current behavior:** BIOS executes 80,000+ instructions in protected mode before hitting decoder bug. No readable text output appears on VGA or debug ports (0xE9, 0x402, 0x403) - only hex bytes.

**Reason:** BIOS crashes due to decoder bug before reaching output-generating code.