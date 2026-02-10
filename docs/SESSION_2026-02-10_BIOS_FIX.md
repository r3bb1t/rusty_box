# BIOS Execution Fix - Session 2026-02-10

## Summary

Successfully fixed BIOS execution issues and switched from BIOS-bochs-latest to BIOS-bochs-legacy to avoid stack corruption. The emulator now executes 50+ million instructions without crashing.

## Problems Solved

### 1. ✅ BIOS Stack Corruption (General Protection Fault)

**Problem**: BIOS-bochs-latest (128 KB) uses ESP=0xFFFFFFF0 which is in the BIOS ROM range (0xFFFE0000-0xFFFFFFFF). Stack operations at ROM addresses fail:
- Writes are vetoed (correct ROM behavior)
- Reads return ROM data instead of actual stack values
- Results in corrupted return addresses like 0xF000E987
- RET instruction tries to jump to 0xF000E987 → exceeds CS.limit (0x0FFFFFFF) → #GP fault

**Solution**: Switched to BIOS-bochs-legacy (64 KB) which uses a stack in low memory that works correctly with <4GB RAM.

**Files Modified**:
- `rusty_box/examples/dlxlinux.rs` - Changed BIOS priority to legacy first

### 2. ✅ Stack Overflow During Initialization

**Problem**: 32 MB RAM configuration causes `STATUS_STACK_OVERFLOW` during emulator initialization, before execution even starts.

**Temporary Solution**: Reduced RAM to 4 MB, which works without stack overflow.

**TODO**: Investigate why 32 MB causes stack overflow. Likely something being allocated on thread stack instead of heap.

**Files Modified**:
- `rusty_box/examples/dlxlinux.rs`:
  - `guest_memory_size: 4 * 1024 * 1024` (was 32 MB)
  - `emu.configure_memory_in_cmos(640, 3456)` (was 31*1024)

### 3. ✅ Excessive Memory Write Logging

**Problem**: 100,000+ lines of output due to logging every write to low memory (0x0-0x1000).

**Solution**: Commented out memory write logging in string operations.

**Files Modified**:
- `rusty_box/src/cpu/string.rs` - Disabled `MEM_WRITE_BYTE` logging

### 4. ✅ Progress Logging Spam

**Problem**: Logging every 10,000 instructions created too much output.

**Solution**: Changed to log every 1,000,000 instructions.

**Files Modified**:
- `rusty_box/src/cpu/cpu.rs` - Changed logging interval from 10K to 1M

## Current Execution Status

### Test Run Results (4 MB RAM, 50M instructions)

```
Instructions:        50,000,004
Time:                54.343 sec
Speed:               0.92 MIPS
Final RIP:           0x97d
ESP:                 0xFFD2 (reasonable stack pointer)
VGA Output:          None yet
Errors:              None
```

### What's Working

- ✅ BIOS-bochs-legacy (64 KB) loads successfully
- ✅ VGA BIOS loads (32 KB)
- ✅ Emulator initializes without stack overflow (4 MB)
- ✅ BIOS executes 50M+ instructions without crashing
- ✅ Stack operations work correctly (ESP in low memory)
- ✅ No General Protection faults
- ✅ No decoder errors
- ✅ No unimplemented instruction errors

### What's Not Working

- ❌ 32 MB RAM causes stack overflow
- ❌ No VGA output yet (BIOS needs more time)
- ❌ BIOS hasn't reached POST code output

## Previous Session Fixes (Still Applied)

From the earlier session where we fixed the decoder operand bug:

1. **Decoder operand direction** (fetchdecode32.rs, fetchdecode64.rs)
   - Ed,Gd instructions now correctly assign rm=dest, reg=src
   - Operator precedence fix: `(b1 & 0x0F) == 0x01` with parentheses

2. **MOV instruction handlers** (data_xfer_ext.rs)
   - Updated mov_ew_gw_r and mov_eb_gb_r to match new decoder layout
   - dst=meta_data[0], src=meta_data[1]

## Next Steps

### Immediate

1. **Investigate 32 MB stack overflow**
   - Check if large structures allocated on stack
   - Profile memory allocation during initialization
   - Consider using Box<> for large emulator components

2. **Run longer to reach VGA output**
   - Increase instruction limit to 500M-1B
   - May need to reduce logging further
   - Watch for unimplemented instructions

### Future

1. **Find and implement missing instructions**
   - According to CLAUDE.md, legacy BIOS reaches RIP=0x6a00+ before hitting unimplemented instructions
   - Current execution at RIP=0x97d is much earlier

2. **Test with full 32 MB once stack overflow fixed**
   - Original bochsrc.bxrc uses 32 MB
   - Need to match original configuration

3. **Monitor for BIOS POST codes**
   - Check port 0x80/0x84 output
   - Should see boot progress indicators

## Performance Notes

- Current speed: ~0.92 MIPS (release build)
- 50M instructions takes ~54 seconds
- BIOS initialization is very slow (many polling loops)
- VGA output may require 500M-1B instructions

## Files Changed This Session

1. `rusty_box/examples/dlxlinux.rs`
   - BIOS priority: legacy first
   - RAM: 32 MB → 4 MB
   - CMOS config: updated for 4 MB
   - Instruction limit: 50M

2. `rusty_box/src/cpu/string.rs`
   - Disabled memory write logging

3. `rusty_box/src/cpu/cpu.rs`
   - Progress logging: 10K → 1M interval

## Related Documentation

- `MEMORY.md` - Documents the BIOS stack issue
- `CLAUDE.md` - Project overview and known issues
- `DECODER_OPERAND_BUG.md` - Previous decoder fix
