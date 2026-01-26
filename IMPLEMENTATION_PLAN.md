# Implementation Plan: Fix BIOS Text Display

## Problem Summary

BIOS text is not displaying in the terminal GUI because VGA port handling is incomplete. The VGABIOS cannot configure color text mode properly.

## Root Cause Analysis

### Issue 1: Missing VGA Port Registrations

**Bochs registers ALL ports 0x3C0-0x3CF** (see `vgacore.cc:223`):
```cpp
for (addr=0x03C0; addr<=0x03CF; addr++) {
    DEV_register_ioread_handler(this, f_read, addr, name, io_mask[i++]);
    DEV_register_iowrite_handler(this, f_write, addr, name, 3);
}
```

**Rust only registers**: 0x3C0, 0x3C1, 0x3C4, 0x3C5, 0x3CC, 0x3CE, 0x3CF

**Missing critical ports**:
| Port | Name | Purpose |
|------|------|---------|
| **0x3C2** | Misc Output Write | **CRITICAL**: BIOS writes here to set color mode |
| 0x3C3 | VGA Enable | Enables/disables VGA |
| 0x3C6 | PEL Mask | Palette mask |
| 0x3C7 | DAC State / PEL Read Addr | DAC state read; palette read addr write |
| 0x3C8 | PEL Write Addr | Palette write address |
| 0x3C9 | PEL Data | Palette RGB data |

### Issue 2: Port 0x3C2 Write Not Handled

**VGA has asymmetric Misc Output ports**:
- **0x3C2** = Write port (BIOS writes here to configure)
- **0x3CC** = Read port (BIOS reads here to check status)

The current Rust code only registers 0x3CC (read), not 0x3C2 (write).

The VGABIOS writes to port 0x3C2 to configure the Misc Output Register:
- Bit 0: `color_emulation` - selects CRTC at 0x3D4 (color) vs 0x3B4 (mono)
- Bit 1: `enable_ram` - enables VGA memory access

Without handling 0x3C2 writes, the BIOS cannot configure color mode.

**Bochs handling** (`vgacore.cc:902-910`):
```cpp
case 0x03c2: // Miscellaneous Output Register
  BX_VGA_THIS s.misc_output.color_emulation  = (value >> 0) & 0x01;
  BX_VGA_THIS s.misc_output.enable_ram       = (value >> 1) & 0x01;
  BX_VGA_THIS s.misc_output.clock_select     = (value >> 2) & 0x03;
  // ...
```

### Issue 3: Missing PEL/DAC State Variables

The `BxVgaC` struct is missing these Bochs fields:
- `vga_enabled: bool` (from port 0x3C3)
- `pel_mask: u8`
- `dac_state: u8` (0x00=write mode, 0x03=read mode)
- `pel_write_addr: u8`
- `pel_read_addr: u8`
- `pel_write_cycle: u8` (0,1,2 for R,G,B)
- `pel_read_cycle: u8`
- `pel_data: [[u8; 3]; 256]` (256 palette entries × RGB)

### Issue 4: Memory Mapping Default

Current default in Rust:
```rust
vga.graphics_regs[6] = 0x08;  // memory_mapping = 2 (mono: 0xB0000-0xB7FFF)
```

But BIOS writes to 0xB8000 (color text). The BIOS should set `memory_mapping = 3` via port 0x3CF, but this requires:
1. Port 0x3C2 handling (to set color_emulation=1)
2. Graphics reg 6 write to set memory_mapping=3

---

## Implementation Steps

### Phase 1: Add Missing State Variables to BxVgaC

**File**: `rusty_box/src/iodev/vga.rs`

Add to `BxVgaC` struct:
```rust
/// VGA enable (port 0x3C3)
vga_enabled: bool,

/// PEL/DAC registers
pel_mask: u8,                    // Port 0x3C6
dac_state: u8,                   // 0x00 = write, 0x03 = read
pel_write_addr: u8,              // Port 0x3C8
pel_read_addr: u8,               // Port 0x3C7 (write)
pel_write_cycle: u8,             // 0, 1, 2 for R, G, B
pel_read_cycle: u8,
pel_data: [[u8; 3]; 256],        // 256 colors × [R, G, B]

/// Misc output register parsed fields (for easier access)
misc_color_emulation: bool,      // Bit 0: color vs mono
misc_enable_ram: bool,           // Bit 1: RAM enabled
misc_clock_select: u8,           // Bits 2-3
misc_select_high_bank: bool,     // Bit 5
misc_horiz_sync_pol: bool,       // Bit 6
misc_vert_sync_pol: bool,        // Bit 7
```

Initialize in `new()`:
```rust
vga_enabled: true,
pel_mask: 0xFF,
dac_state: 0x01,
pel_write_addr: 0,
pel_read_addr: 0,
pel_write_cycle: 0,
pel_read_cycle: 0,
pel_data: [[0; 3]; 256],
misc_color_emulation: true,   // Default to color mode
misc_enable_ram: true,
misc_clock_select: 0,
misc_select_high_bank: false,
misc_horiz_sync_pol: true,
misc_vert_sync_pol: true,
```

### Phase 2: Register All Missing Ports

**File**: `rusty_box/src/iodev/vga.rs`, in `init()` function

Add port registrations (after existing ones):
```rust
// Misc Output Write (0x3C2) - CRITICAL for BIOS
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3C2, "VGA Misc Output Write", 0x1);

// VGA Enable (0x3C3)
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3C3, "VGA Enable", 0x1);

// PEL Mask (0x3C6)
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3C6, "VGA PEL Mask", 0x1);

// DAC State Read / PEL Address Read Mode (0x3C7)
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3C7, "VGA DAC State", 0x1);

// PEL Address Write Mode (0x3C8)
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3C8, "VGA PEL Address Write", 0x1);

// PEL Data (0x3C9)
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3C9, "VGA PEL Data", 0x1);

// EGA compatibility (0x3CA, 0x3CB, 0x3CD) - stubs
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3CA, "VGA EGA Compat", 0x1);
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3CB, "VGA EGA Compat", 0x1);
io.register_io_handler(vga_ptr, vga_read_handler, vga_write_handler,
    0x3CD, "VGA EGA Compat", 0x1);
```

### Phase 3: Implement Port Read/Write Handlers

**File**: `rusty_box/src/iodev/vga.rs`

#### Update `read_port()`:
```rust
// Add these cases:
0x3C2 => 0xFF, // Write-only port, return 0xFF on read
0x3C3 => self.vga_enabled as u32,
0x3C6 => self.pel_mask as u32,
0x3C7 => self.dac_state as u32,
0x3C8 => self.pel_write_addr as u32,
0x3C9 => {
    // Read palette data (only if dac_state == 0x03)
    if self.dac_state == 0x03 {
        let color = self.pel_data[self.pel_read_addr as usize];
        let val = color[self.pel_read_cycle as usize];
        self.pel_read_cycle += 1;
        if self.pel_read_cycle >= 3 {
            self.pel_read_cycle = 0;
            self.pel_read_addr = self.pel_read_addr.wrapping_add(1);
        }
        val as u32
    } else {
        0x3F
    }
}
0x3CA | 0x3CB | 0x3CD => 0x00, // EGA compat, stub
```

#### Update `write_port()`:
```rust
// Add these cases:
0x3C2 => {
    // Misc Output Register Write - CRITICAL
    self.misc_color_emulation = (value & 0x01) != 0;
    self.misc_enable_ram = (value & 0x02) != 0;
    self.misc_clock_select = (value >> 2) & 0x03;
    self.misc_select_high_bank = (value & 0x20) != 0;
    self.misc_horiz_sync_pol = (value & 0x40) != 0;
    self.misc_vert_sync_pol = (value & 0x80) != 0;
    // Also update the combined misc_output field for read at 0x3CC
    self.misc_output = value;
    tracing::debug!("VGA Misc Output Write: {:#04x} (color_emulation={})",
        value, self.misc_color_emulation);
}
0x3C3 => {
    self.vga_enabled = (value & 0x01) != 0;
}
0x3C6 => {
    self.pel_mask = value;
}
0x3C7 => {
    // PEL Address Read Mode
    self.pel_read_addr = value;
    self.pel_read_cycle = 0;
    self.dac_state = 0x03; // Read mode
}
0x3C8 => {
    // PEL Address Write Mode
    self.pel_write_addr = value;
    self.pel_write_cycle = 0;
    self.dac_state = 0x00; // Write mode
}
0x3C9 => {
    // PEL Data Write
    self.pel_data[self.pel_write_addr as usize][self.pel_write_cycle as usize] = value;
    self.pel_write_cycle += 1;
    if self.pel_write_cycle >= 3 {
        self.pel_write_cycle = 0;
        self.pel_write_addr = self.pel_write_addr.wrapping_add(1);
    }
}
0x3CA | 0x3CB | 0x3CD => {
    // EGA compat, ignore
}
```

### Phase 4: Enhance Graphics Reg 6 Handler

Ensure that when graphics_regs[6] is written, memory mapping is properly tracked:

```rust
VGA_GRAPHICS_DATA => {
    if self.graphics_index < 9 {
        let old_value = self.graphics_regs[self.graphics_index as usize];
        self.graphics_regs[self.graphics_index as usize] = value;

        // Special handling for register 6 (Miscellaneous)
        if self.graphics_index == 6 {
            let old_mapping = (old_value >> 2) & 0x03;
            let new_mapping = (value >> 2) & 0x03;
            if old_mapping != new_mapping {
                tracing::debug!("VGA memory_mapping changed: {} -> {}", old_mapping, new_mapping);
                self.text_buffer_update = true;
            }
        }
    }
}
```

### Phase 5: Add Debug Tracing

Add tracing in key locations to verify BIOS is configuring VGA properly:

1. In `write_port()` for 0x3C2 - log when misc output is set
2. In `write_port()` for graphics reg 6 - log memory_mapping changes
3. In `vga_mem_write_handler()` - log first few writes with addresses

---

## Files to Modify

| File | Changes |
|------|---------|
| `rusty_box/src/iodev/vga.rs` | All changes (struct, init, read/write handlers) |

## Testing

1. Run `cargo run --release --example dlxlinux --features std`
2. Verify VGA probe summary shows mapped writes to 0xB8000+
3. Verify terminal displays BIOS text in cyan on black background

## Success Criteria

- [ ] Port 0x3C2 writes are logged (misc output config)
- [ ] memory_mapping changes from 2 to 3 after BIOS init
- [ ] VGA probe shows `probe_mapped_writes > 0`
- [ ] VGA probe shows `probe_first_mapped.addr >= 0xB8000`
- [ ] Terminal displays BIOS messages (VGABios, BIOS version, disk info)

---

## Data Flow Analysis

### VGA Text Display Pipeline

```
1. BIOS/Guest Code
   └── Writes to 0xB8000-0xBFFFF (color text memory)
       └── Via CPU memory access (STOSB, MOVSB, MOV, etc.)

2. Memory Subsystem (memory/mod.rs)
   └── Detects write to 0xA0000-0xBFFFF range
       └── Routes to vga_mem_write_handler()

3. VGA Memory Handler (iodev/vga.rs:802-865)
   └── Checks memory_mapping (graphics_regs[6] bits 2-3)
       ├── mapping=2: Accept 0xB0000-0xB7FFF (mono)
       ├── mapping=3: Accept 0xB8000-0xBFFFF (color) ← EXPECTED
       └── Sets text_dirty=true, vga_mem_updated=1

4. VGA Update (iodev/vga.rs:610-738)
   └── Called from emulator.update_gui()
       └── Returns VgaUpdateResult with:
           - text_buffer (new VGA memory contents)
           - text_snapshot (old contents for diffing)
           - tm_info (start_address, line_offset, cursor info)

5. GUI text_update (gui/term.rs:141-163)
   └── Stores new_text and tm_info
       └── Calls render_text_mode()

6. Terminal Rendering (gui/term.rs:342-484)
   └── Uses tm_info.start_address and line_offset
       └── Renders each character with ANSI colors
```

### VGABIOS Initialization Sequence

The VGABIOS performs these port I/O operations during init:

```
1. Write 0x3C2 (Misc Output) = 0x67
   - color_emulation = 1 (use 0x3D4/0x3D5 CRTC)
   - enable_ram = 1
   - clock_select = 1

2. Write 0x3C4 (Seq Index) = 0x00
   Write 0x3C5 (Seq Data) = reset sequence

3. Write 0x3CE (Graphics Index) = 0x06
   Write 0x3CF (Graphics Data) = 0x0E
   - graphics_alpha = 0 (text mode)
   - chain_odd_even = 1
   - memory_mapping = 3 (color text: 0xB8000-0xBFFFF) ← CRITICAL

4. Set up CRTC registers (0x3D4/0x3D5)

5. Set palette via 0x3C8/0x3C9

6. Write text to 0xB8000+
```

### Why Text Doesn't Display Currently

```
Current State:
  graphics_regs[6] = 0x08 (default)
  └── memory_mapping = (0x08 >> 2) & 0x03 = 2 (mono window)

What Happens:
  1. BIOS tries to write to port 0x3C2 - NOT REGISTERED, ignored
  2. BIOS tries to write to port 0x3CF (graphics reg 6) - WORKS
     BUT: BIOS may skip this if 0x3C2 write failed
  3. BIOS writes to 0xB8000 (color text area)
  4. vga_mem_write_handler checks: memory_mapping=2
     └── 0xB8000 NOT in range 0xB0000-0xB7FFF → IGNORED
  5. probe_unmapped_writes++ (write rejected)
  6. text_memory stays empty
  7. GUI shows blank screen

After Fix:
  1. BIOS writes to port 0x3C2 - HANDLED, sets misc_output
  2. BIOS writes to port 0x3CF (graphics reg 6) = 0x0E
     └── memory_mapping changes to 3
  3. BIOS writes to 0xB8000 (color text area)
  4. vga_mem_write_handler checks: memory_mapping=3
     └── 0xB8000 IN range 0xB8000-0xBFFFF → ACCEPTED
  5. probe_mapped_writes++ (write accepted)
  6. text_memory contains BIOS text
  7. GUI displays BIOS messages
```

---

## Debugging Strategy

### Step 1: Add Port I/O Tracing

Add `tracing::debug!` for every unhandled port:
```rust
_ => {
    tracing::debug!("VGA UNHANDLED port write: {:#06x} = {:#04x}", port, value);
    // ... existing code
}
```

Run with `RUST_LOG=debug` to see which ports BIOS accesses.

### Step 2: Verify Port 0x3C2 is Called

After implementing 0x3C2 handler, add:
```rust
0x3C2 => {
    tracing::info!("VGA Misc Output Write: {:#04x} (color_emulation={}, enable_ram={})",
        value, (value & 0x01) != 0, (value & 0x02) != 0);
    // ... handler code
}
```

### Step 3: Verify Graphics Reg 6 Changes

Add logging when memory_mapping changes:
```rust
if self.graphics_index == 6 {
    let old_mapping = (old_value >> 2) & 0x03;
    let new_mapping = (value >> 2) & 0x03;
    tracing::info!("VGA graphics_reg[6] write: {:#04x} -> {:#04x} (memory_mapping: {} -> {})",
        old_value, value, old_mapping, new_mapping);
}
```

### Step 4: Check VGA Probe Summary

The existing `probe_summary()` method shows:
- `probe_mapped_writes` - writes accepted by current mapping
- `probe_unmapped_writes` - writes rejected (wrong address range)
- `probe_first_mapped` - first successful write (addr, value, mapping)
- `probe_first_unmapped` - first rejected write (addr, value, mapping)

After running, check:
```
mapped_writes=0 unmapped_writes=12345  ← BAD (all writes rejected)
first_mapped: <none>
first_unmapped: addr=0xb8000 val=0x56 memory_mapping=2  ← Confirms the issue
```

vs.
```
mapped_writes=12345 unmapped_writes=0  ← GOOD
first_mapped: addr=0xb8000 val=0x56 memory_mapping=3  ← Working correctly
first_unmapped: <none>
```

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Missing other ports | Medium | Low | Log unhandled ports, iterate |
| Incorrect misc_output parsing | Low | High | Match Bochs exactly |
| PEL/DAC timing issues | Low | Medium | Use simple state machine |
| Memory mapping edge cases | Low | Medium | Add bounds checking |

---

## Alternative Approaches Considered

### Option A: Change Default memory_mapping to 3
**Rejected**: This is a workaround, not a fix. The BIOS expects to configure VGA via ports.

### Option B: Accept writes to both B0000 and B8000
**Rejected**: Violates VGA specification. Would cause incorrect behavior for mono mode.

### Option C: Register ports as no-ops
**Rejected**: Must implement properly for full BIOS compatibility.

**Selected Approach**: Implement full port handling (Phase 1-5 above)

---

## Current Status (Updated)

### Implementation Complete

✅ All VGA port handling has been implemented:
- Added missing state variables (vga_enabled, pel_mask, dac_state, pel_data, etc.)
- Registered missing ports: 0x3C2, 0x3C3, 0x3C6-0x3C9, 0x3CA, 0x3CB, 0x3CD
- Implemented read handlers for all new ports
- Implemented write handlers with debug tracing
- Added memory_mapping change logging

### New Issue Discovered: Stack Corruption Causing RIP=0

**Symptom**: After ~5 million instructions:
- VGA probe shows `mapped_writes=0, unmapped_writes=0`
- No BIOS POST codes captured (port 0x80/0x84)
- Final RIP = 0x0000000000000000 (abnormal)
- EAX=0000ffee, ESP=00000004
- Repeated POP16 from SS:SP=0000:0000 and 0000:0002

**Root Cause Analysis**:

1. **Stack pointer starts at 0**:
   - CPU reset sets all general registers to 0, including SP
   - SS is set to 0x0000 with base=0x00000000
   - Initial SS:SP = 0000:0000

2. **BIOS first instruction is PUSH CS**:
   - BIOS at F000:E05B starts with `0E` (PUSH CS)
   - push_16: SP = 0 - 2 = 0xFFFE (wrapping), writes to 0000:FFFE

3. **Stack wraps around**:
   - As BIOS executes, stack grows downward from 0xFFFE
   - Eventually wraps back to low addresses near 0

4. **RET pops 0 from IVT area**:
   - When SP becomes 0, POP/RET reads from address 0
   - Memory at 0-3 is IVT (Interrupt Vector Table), uninitialized = 0
   - RET pops 0 → IP becomes 0

5. **Execution loops at F000:0000**:
   - Code at F000:0000 is a function prologue (55 89 e5 50 51 06 57 8b)
   - This code also uses stack, continues the loop

**Key Debug Findings**:
```
set_eip(0) called! old_eip=0x65f, SS:SP=0000:0004
POP16: popped 0 from SS:SP=0000:0000 (laddr=0x0)
POP16: popped 0 from SS:SP=0000:0002 (laddr=0x2)
[repeats endlessly]
```

**BIOS Memory Mapping Verified**:
- BIOS loaded at ROM offset 0x3E0000 (128KB at 0xFFFE0000)
- bios_map_last128k(0xFE05B) = 0x3E05B ✓
- BIOS file offset 0x5B contains valid code: `0E 66 B9...` (PUSH CS, MOV ECX,...)

### Possible Causes

1. **BIOS expects different stack setup**:
   - Original Bochs may initialize SP to non-zero value
   - BIOS may rely on pre-initialized stack segment

2. **Stack operations corrupting state**:
   - Possible issue with stack segment limit checking
   - Wrapping behavior may differ from original

3. **Missing early BIOS setup**:
   - BIOS should set up SS:SP before using stack
   - Our code may be missing pre-BIOS initialization

### Investigation Progress

**Stack tracing added** - First push operations show:
```
PUSH16[33]: SP 0000->fffe, val=e0c9  // Return address from CALL
PUSH16[34]: SP fffe->fffc, val=0000
PUSH16[37]: SP 0000->fffe, val=0000  // SP reset to 0!
```

SP repeatedly cycles 0→fffe→fffc→0, suggesting the BIOS is in a loop.

**BIOS POST Entry Analysis**:
- POST entry at 0xE05B (confirmed in rombios.c line 11096)
- First instructions: xor ax,ax; out DMA; check CMOS shutdown status
- CMOS register 0x0F (shutdown) should be 0 for normal boot
- If shutdown=0, jumps to `normal_post` which sets up SS:SP=0000:FFFE

**CMOS I/O ports (0x70-0x71) ARE registered** in devices.rs.
But no CMOS shutdown status read logs appearing - I/O may not be reaching handlers.

### Next Steps

1. **Debug I/O port handling**:
   - Verify IN/OUT instructions are calling port handlers
   - Add logging to port 0x70 and 0x71 writes/reads
   - Trace CMOS address selection and data reads

2. **Trace early BIOS execution**:
   - Log first 50 OUT instructions to see DMA init
   - Verify CMOS port I/O (OUT 0x70, IN 0x71) happens

3. **Check instruction implementations**:
   - Verify OUT/IN byte instructions working
   - Check that DMA ports (0x0D, 0xDA) are handled

### Code Changes Made (VGA)

**File: `rusty_box/src/iodev/vga.rs`**

```rust
// New state variables added to BxVgaC struct:
vga_enabled: bool,
pel_mask: u8,
dac_state: u8,
pel_write_addr: u8,
pel_read_addr: u8,
pel_write_cycle: u8,
pel_read_cycle: u8,
pel_data: [[u8; 3]; 256],
misc_color_emulation: bool,
misc_enable_ram: bool,
misc_clock_select: u8,
misc_select_high_bank: bool,
misc_horiz_sync_pol: bool,
misc_vert_sync_pol: bool,

// New ports registered in init():
0x3C2 - VGA Misc Output Write
0x3C3 - VGA Enable
0x3C6 - PEL Mask
0x3C7 - DAC State
0x3C8 - PEL Address Write
0x3C9 - PEL Data

// Write handler for 0x3C2:
0x3C2 => {
    self.misc_color_emulation = (value & 0x01) != 0;
    self.misc_enable_ram = (value & 0x02) != 0;
    self.misc_clock_select = (value >> 2) & 0x03;
    self.misc_select_high_bank = (value & 0x20) != 0;
    self.misc_horiz_sync_pol = (value & 0x40) != 0;
    self.misc_vert_sync_pol = (value & 0x80) != 0;
    self.misc_output = value;
    tracing::info!("VGA Misc Output Write: {:#04x}...", value);
}

// Enhanced graphics reg 6 handler with memory_mapping logging
```

### Other Fixes Made (Build Errors)

- Fixed typo `y16` → `u16` in cpu.rs
- Added cfg guards for conditional fields in builder.rs
- Fixed `Amx` → `AMX` type name
- Added `Copy, Clone` derives to `MemType` enum
- Made several private fields `pub(super)` for builder access

---

## Critical Decoder Bug Found: Register Assignment for Non-ModRM Opcodes

**Date**: January 2026

### Problem Discovery

While debugging BIOS execution, discovered that MOV AL, imm8 (opcode B0) was writing to
the wrong register. Tracing showed:

```
MOV r8, imm8 [#4]: reg6 = 0xc0, AL before=0x00
OUT 0xd6, AL (0x00) [instr #5] EIP=0xe065
```

The destination was reg6 (DH) instead of reg0 (AL), causing CMOS initialization to fail.

### Root Cause

In `fetchdecode32.rs` and `fetchdecode64.rs`, the register fields were assigned as:

```rust
instr.meta_data[BX_INSTR_METADATA_DST] = nnn as u8;  // bits 3-5 of opcode
instr.meta_data[BX_INSTR_METADATA_SRC1] = rm as u8;  // bits 0-2 of opcode
```

For opcodes B0-B7 (MOV r8, imm8):
- `nnn = (0xB0 >> 3) & 0x7 = 6` ← This was being used as DST (wrong!)
- `rm = 0xB0 & 0x7 = 0` ← This should be DST (correct: register 0 = AL)

### Bochs Reference

Bochs uses `assign_srcs()` with source type information from the opcode table:
- `MOV_EbIb` has `OP_Eb = BX_FORM_SRC(BX_GPR8, BX_SRC_RM)` as destination
- `BX_SRC_RM` means use the `rm` value (bits 0-2)
- `BX_SRC_NNN` means use the `nnn` value (bits 3-5)

### The Complication

A simple swap (DST=rm, SRC1=nnn for non-ModRM) breaks other instructions:
- **PUSH ES** (opcode 06): segment is in bits 3-5 (nnn=0=ES), rm=6
- **PUSH CS** (opcode 0E): segment is in bits 3-5 (nnn=1=CS), rm=6

Different non-ModRM opcodes encode registers differently:
| Opcode Range | Register Location | Examples |
|--------------|-------------------|----------|
| B0-B7 | bits 0-2 (rm) | MOV r8, imm8 |
| B8-BF | bits 0-2 (rm) | MOV r16/32/64, imm |
| 50-57 | bits 0-2 (rm) | PUSH r16/32 |
| 58-5F | bits 0-2 (rm) | POP r16/32 |
| 06,0E,16,1E | bits 3-5 (nnn) | PUSH ES/CS/SS/DS |
| 07,17,1F | bits 3-5 (nnn) | POP ES/SS/DS |
| 40-47 | bits 0-2 (rm) | INC r16/32 |
| 48-4F | bits 0-2 (rm) | DEC r16/32 |
| 90-97 | bits 0-2 (rm) | XCHG r16/32, AX |

### Proper Fix Required

The correct fix is to implement source type lookup from the opcode table, similar to
Bochs' `assign_srcs()` function. For each operand position:
1. Look up the source type (BX_SRC_NNN, BX_SRC_RM, BX_SRC_EAX, etc.)
2. Assign the appropriate register value

**Temporary Workaround**: For segment push/pop (0x06, 0x07, 0x0E, 0x16, 0x17, 0x1E, 0x1F),
keep DST=nnn. For other non-ModRM opcodes, use DST=rm.

### Files Modified

- `rusty_box_decoder/src/fetchdecode32.rs` - Register assignment logic
- `rusty_box_decoder/src/fetchdecode64.rs` - Same fix for 64-bit decoder

### Testing Results

After fix (before segment push issue):
```
MOV r8, imm8 [#4]: reg0 = 0xc0, AL before=0x00   ← Correct!
OUT 0xd6, AL (0xc0) [instr #5] EIP=0xe065        ← AL now has 0xC0!
OUT 0x70, AL (0x0f) [instr #9] EIP=0xe06d        ← CMOS address 0x0F!
CMOS: Read shutdown status [0x0f] = 0x00         ← CMOS working!
```

BIOS now reads CMOS shutdown status correctly and can proceed to normal POST.

### Next Steps

1. Implement proper source type lookup in decoder (long-term fix)
2. Add special handling for segment push/pop opcodes (short-term fix)
3. Test BIOS boot to verify stack setup works
