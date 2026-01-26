---
name: Fix BIOS Text Rendering
overview: Fix the emulator to properly display BIOS text output on the terminal UI by ensuring VGA memory writes are captured, the GUI is updated, and the terminal GUI is actually used.
todos:
  - id: fix-example-gui
    content: Change dlxlinux.rs example to use TermGui instead of NoGui
    status: in_progress
  - id: verify-vga-handler
    content: Verify VGA memory write handler is properly capturing writes to 0xB8000
    status: pending
  - id: improve-gui-updates
    content: Improve GUI update frequency in run_interactive() to update on every dirty change
    status: pending
  - id: fix-term-rendering
    content: Fix terminal GUI rendering to properly display VGA text buffer
    status: pending
  - id: add-debug-logs
    content: Add trace logging for VGA memory writes and GUI updates
    status: pending
  - id: test-bios-output
    content: Test BIOS text output appears correctly in terminal
    status: pending
isProject: false
---

# Fix BIOS Text Rendering

## Problem Analysis

The emulator boots and executes instructions, but BIOS text is not visible because:
1. The example uses `NoGui` instead of `TermGui` (dlxlinux.rs:191)
2. VGA memory writes may not be properly triggering GUI updates
3. The GUI update frequency might be insufficient
4. VGA text memory handler might need better integration

## Implementation Plan

### 1. Fix Example to Use TermGui
- **File**: `rusty_box/examples/dlxlinux.rs`
- Change line 191 from `NoGui` to `TermGui`
- Ensure terminal GUI is properly initialized

### 2. Verify VGA Memory Handler Integration
- **File**: `rusty_box/src/iodev/vga.rs`
- Ensure `vga_mem_write_handler` properly marks text as dirty
- Verify memory handler is registered for 0xB8000-0xBFFFF range
- Check that writes to VGA text memory (0xB8000) are captured

### 3. Improve GUI Update Frequency
- **File**: `rusty_box/src/emulator.rs`
- Review `run_interactive()` update logic (line 554-627)
- Ensure GUI updates happen frequently enough (currently 100ms interval)
- Consider updating on every dirty flag change, not just periodically

### 4. Fix Terminal GUI Rendering
- **File**: `rusty_box/src/gui/term.rs`
- Verify `text_update()` properly handles VGA text buffer format
- Ensure cursor positioning works correctly
- Check that ANSI color codes are properly applied
- Fix any issues with character rendering

### 5. Add Debugging/Logging
- Add trace logs for VGA memory writes to verify handler is called
- Log when GUI updates occur
- Verify text buffer contents are being read correctly

### 6. Test BIOS Text Output
- Run the emulator and verify BIOS messages appear
- Check that cursor updates work
- Verify text scrolling if BIOS writes beyond screen

## Key Files to Modify

1. `rusty_box/examples/dlxlinux.rs` - Switch to TermGui
2. `rusty_box/src/iodev/vga.rs` - Verify memory handler
3. `rusty_box/src/emulator.rs` - Improve update frequency
4. `rusty_box/src/gui/term.rs` - Fix rendering issues

## Testing Strategy

1. Run `cargo run --release --example dlxlinux`
2. Verify BIOS text appears in terminal
3. Check that cursor updates correctly
4. Verify text colors are rendered properly
5. Test with different BIOS messages

## Notes

- VGA text mode uses 2 bytes per character: [char, attribute]
- Text memory is at 0xB8000-0xBFFFF (32KB, supports multiple pages)
- Standard text mode is 80x25 = 2000 characters = 4000 bytes per page
- BIOS typically writes directly to VGA memory using MOV instructions
- The VGA memory handler must intercept these writes and mark text as dirty