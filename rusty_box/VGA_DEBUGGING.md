# VGA Display Debugging Guide

## Current Status

The emulator is executing instructions (nearly 1 billion) but no VGA text is appearing on screen.

## Added Debugging Features

1. **VGA Memory Write Logging**: 
   - Logs first 20 calls to `vga_mem_write_handler`
   - Logs first 10 writes to text memory (0xB8000-0xBFFFF)
   - Shows address, length, and value changes

2. **GUI Update Logging**:
   - Logs when GUI update is triggered
   - Logs when text is dirty

## How to Debug

When running the example, check the logs for:

1. **VGA Handler Calls**:
   ```
   VGA mem_write_handler called #X: addr=0x..., len=...
   ```
   - If you see these: Handler is being called, but maybe not for text memory
   - If you DON'T see these: Handler isn't being called at all (registration issue)

2. **Text Memory Writes**:
   ```
   VGA TEXT MEM WRITE #X: addr=0xB8000, len=...
     offset=0x...: 0x00 -> 0x41
   ```
   - If you see these: Text is being written, but display isn't updating
   - If you DON'T see these: BIOS isn't writing to VGA text memory

3. **GUI Updates**:
   ```
   update_gui: Text is dirty, updating display
   TermGUI: Rendering text with X non-zero bytes
   ```
   - If you see these: GUI is trying to render, but maybe rendering is broken

## Possible Issues

### Issue 1: Handler Not Being Called
**Symptoms**: No "VGA mem_write_handler called" messages
**Causes**:
- Memory handler not registered correctly
- Memory writes going through different path (TLB cache)
- BIOS not writing to VGA memory at all

### Issue 2: Handler Called But Not For Text Memory
**Symptoms**: See handler calls but for addresses != 0xB8000
**Causes**:
- BIOS writing to different VGA memory range
- VGA not in text mode (memory_mapping != 3)

### Issue 3: Text Written But Not Displayed
**Symptoms**: See "VGA TEXT MEM WRITE" but no screen output
**Causes**:
- GUI rendering broken
- Text buffer not being read correctly
- Terminal not displaying properly

## Next Steps

1. Run with logging enabled: `RUST_LOG=debug cargo run --example dlxlinux`
2. Check which of the above scenarios you're in
3. Based on logs, fix the specific issue

## Reference: Bochs Behavior

In Bochs:
- VGA has a timer that calls `refresh_display()` periodically
- `refresh_display()` calls `update()` which checks `vga_mem_updated` flag
- `vga_mem_updated` is set in `mem_write()` when VGA memory is written
- Text mode uses memory_mapping = 3, which maps to 0xB8000-0xBFFFF
