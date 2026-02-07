# BIOS Execution Investigation Notes

**Date**: 2026-02-06
**Critical Finding**: No actual BIOS output observed

## Key Insight from User

**IMPORTANT**: The messages in `bios_out.txt` are from the **emulator's internal debug code** (`self.debug_puts()`), NOT from the BIOS writing to debug ports!

```
[IVT->0000:0000]                     ← Emulator's RIP=0 detection code
[RIP=0 cs:ip=0000:0000] 00 00 00 00 ← Emulator's debug output
```

## What This Reveals

### Expected BIOS Behavior:
- BIOS should write to ports 0xE9, 0x402, 0x403 during initialization
- POST codes should appear on port 0x80
- VGA BIOS should write to port 0x500
- Debug messages like "Starting..." should appear

### Actual Behavior:
- ❌ NO output to any BIOS debug ports
- ❌ NO POST codes
- ❌ NO progress indicators
- ✅ Emulator runs for 50+ seconds (but doing what?)

## Possible Scenarios

### 1. Tight Loop (Most Likely)
```
BIOS might be stuck in:
- Early initialization waiting for hardware
- Polling an I/O port that never changes
- Waiting for timer interrupt that never fires
- Spinning on keyboard controller ready bit
```

### 2. Executing Zeros/NOPs
```
- Jumped to uninitialized memory early
- Reading zeros which decode as:
  - 0x00 = ADD [BX+SI], AL (harmless in many cases)
  - Executes continuously without crashing
```

### 3. Missing Hardware Response
```
Common BIOS dependencies:
- Timer (PIT 8254) - ports 0x40-0x43
- Keyboard Controller (8042) - ports 0x60, 0x64
- CMOS/RTC - ports 0x70, 0x71
- PCI Configuration - ports 0xCF8, 0xCFC
- DMA Controller - ports 0x00-0x0F, 0x80-0x8F
```

## Investigation Plan

### Phase 1: Add Instruction-Level Tracing ✅
- [x] Log RIP, linear address, instruction bytes
- [x] Sample every 100k instructions
- [ ] Capture actual trace output (in progress)

### Phase 2: Analyze Execution Pattern
- [ ] Identify if RIP is changing or stuck
- [ ] Check if executing from BIOS ROM or RAM
- [ ] Determine if it's a tight loop or progressing

### Phase 3: I/O Port Monitoring
- [ ] Log all I/O port reads/writes
- [ ] Check what ports BIOS is accessing
- [ ] Identify missing hardware responses

### Phase 4: Hardware Device Status
- [ ] PIT (Timer): Is it configured? Interrupting?
- [ ] PIC (Interrupt Controller): Are interrupts enabled?
- [ ] Keyboard: Is controller responding?
- [ ] CMOS: Are reads returning valid data?

## Code Changes for Investigation

### Added Tracing (cpu.rs)
```rust
// Sample instruction execution every 100k instructions
if iteration % 100_000 == 0 && iteration <= 1_000_000 {
    let current_rip = self.rip();
    let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
    let linear_addr = cs_base + current_rip;
    let bytes: Vec<u8> = (0..8).map(|i| self.mem_read_byte(linear_addr + i)).collect();
    println!("Trace [{}k]: RIP={:#010x}, Linear={:#010x}, Bytes=[{:02x} ...]",
        iteration / 1000, current_rip, linear_addr, bytes[0]);
}
```

### I/O Port Logging (Needed)
```rust
// In iodev/mod.rs, add verbose logging:
pub fn inp(&mut self, addr: u16, len: u8) -> u32 {
    if addr != 0x64 && addr != 0x61 {  // Skip noise
        println!("I/O READ:  port={:#06x}, len={}", addr, len);
    }
    // ... existing code
}

pub fn outp(&mut self, addr: u16, value: u32, len: u8) {
    if addr != 0x80 {  // Skip POST codes for now
        println!("I/O WRITE: port={:#06x}, value={:#x}, len={}", addr, value, len);
    }
    // ... existing code
}
```

## Comparison: Working vs Non-Working

### What We Know Works:
- ✅ Memory subsystem (verified vs Bochs)
- ✅ Instruction decoder (Group 1 fix applied)
- ✅ Basic arithmetic/logic instructions
- ✅ CPUID
- ✅ Protected mode entry mechanics

### What Might Not Work:
- ❓ Hardware interrupt delivery (timer, keyboard)
- ❓ I/O port responses (PIT, PIC, keyboard controller, CMOS)
- ❓ Exception handling in protected mode
- ❓ A20 gate handling
- ❓ BIOS ROM shadowing / memory mapping

## Test with Original Bochs

To verify our findings, we should test the same BIOS+config with original Bochs:

```bash
cd cpp_orig/bochs
./bochs -f ../../dlxlinux/bochsrc.bxrc -q
```

Expected outcomes:
1. **If original Bochs also hangs**: BIOS incompatibility confirmed
2. **If original Bochs boots**: Missing emulator feature identified

## Next Immediate Steps

1. **Capture Trace Output**: Get the actual RIP/instruction trace
2. **Analyze Pattern**: Determine if stuck loop or progression
3. **Add I/O Logging**: See what hardware BIOS is trying to access
4. **Test with Original Bochs**: Verify expected behavior
5. **Compare Execution**: Identify divergence point

## Expected BIOS Boot Sequence

For reference, what a working BIOS should do:

1. **Reset Vector (F000:FFF0)**
   - Jump to BIOS entry point
   - Should see output: "Starting BIOS..."

2. **CPU Detection**
   - CPUID instructions
   - Should see output: "CPU: ..."

3. **Memory Detection**
   - Test memory size
   - Configure CMOS
   - Should see output: "Memory: 32 MB"

4. **Hardware Init**
   - PIC, PIT, DMA setup
   - Keyboard controller init
   - Should see output: "Initializing..."

5. **POST**
   - Power-On Self-Test
   - POST codes to port 0x80
   - Should see various POST codes

6. **Boot Device**
   - Read INT 0x13 (disk)
   - Load boot sector
   - Should see output: "Booting..."

7. **Jump to 0x7C00**
   - Transfer control to OS loader

We're currently failing before step 1 completes!

## Conclusion

The lack of ANY BIOS output is a critical indicator that something is fundamentally wrong. Either:
1. BIOS is stuck in early initialization waiting for hardware
2. BIOS jumped to wrong address and is executing garbage
3. Missing hardware implementation preventing progress

The instruction-level trace will reveal which scenario we're in.

---

Last Updated: 2026-02-06
Status: Investigation In Progress
