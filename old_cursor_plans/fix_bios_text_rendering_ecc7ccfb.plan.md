---
name: Fix BIOS Text Rendering
overview: Fix the emulator to properly display BIOS text output on the terminal UI by ensuring VGA memory writes are captured, the GUI is updated, and the terminal GUI is actually used.
todos:
  - id: fix-example-gui
    content: Change dlxlinux.rs example to use TermGui instead of NoGui
    status: in_progress
  - id: implement-vga-update
    content: Implement VGA update() function (no_std) matching vgacore.cc:1598-1693 - process text mode, copy memory to text_buffer, return update data for GUI
    status: pending
  - id: add-text-buffer-snapshot
    content: Add text_buffer and text_snapshot fields to BxVgaC struct, and text_buffer_update flag
    status: pending
  - id: implement-timer-updates
    content: Implement periodic VGA updates in run_interactive() loop (can use std) - call vga.update() and gui.flush() every ~100ms
    status: pending
  - id: fix-mem-write-flags
    content: Fix memory write handler to set vga_mem_updated flag matching vgacore.cc:1818-2180
    status: pending
  - id: fix-term-rendering
    content: Fix terminal GUI text_update() to match term.cc:551-608 - byte-by-byte comparison, proper color/character rendering
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

## Problem Analysis (Based on Original Bochs Code)

After analyzing `cpp_orig/bochs/iodev/display/vgacore.cc` and `cpp_orig/bochs/gui/term.cc`, the issues are:

1. **Example uses NoGui**: `dlxlinux.rs:191` uses `NoGui` instead of `TermGui`
2. **Missing Timer-Based Update System**: Original Bochs uses `vga_timer_handler()` (vgacore.cc:2413) that periodically calls `update()` (line 2427), which processes text mode and calls `bx_gui->text_update_common()` (line 1685)
3. **Missing Text Buffer/Snapshot Management**: Original has `s.text_buffer` and `s.text_snapshot` - snapshot is old state, buffer is new state. The `update()` function copies VGA memory to buffer (line 1679-1683), compares with snapshot (line 1685), then copies buffer to snapshot (line 1689-1691)
4. **Missing Update Flag Handling**: Original sets `s.vga_mem_updated` flag on memory writes (line 1852, 2180), and `update()` checks this flag (line 1687)
5. **Missing Update() Function**: The Rust VGA doesn't have the equivalent of `bx_vgacore_c::update()` that processes text mode and extracts text buffer from VGA memory

## Implementation Plan (Matching Original Bochs)

### 1. Fix Example to Use TermGui

- **File**: `rusty_box/examples/dlxlinux.rs`
- Change line 191 from `NoGui` to `TermGui`
- Ensure terminal GUI is properly initialized

### 2. Implement VGA Update() Function (Matching vgacore.cc:1598-1693)

**Constraint**: Must be no_std compatible (only `core` + `alloc`).

- **File**: `rusty_box/src/iodev/vga.rs`
- Add `text_buffer` and `text_snapshot` fields to `BxVgaC` struct (using `alloc::vec::Vec<u8>`)
- Add `text_buffer_update` boolean flag and `vga_mem_updated` u8 flag
- Add `update()` method that:
- Checks if in text mode (`graphics_ctrl.memory_mapping == 3`)
- Calculates text mode parameters (rows, cols, cursor position, line_offset)
- Copies from VGA memory (`text_memory`) to `text_buffer` if `text_buffer_update` is true
- Returns update data (text buffer, cursor position, text mode info) for GUI to process
- Copies new buffer to snapshot if `vga_mem_updated > 0`
- Resets `vga_mem_updated` flag
- **Alternative**: Have `update()` return a struct with text buffer data, let example handle GUI call

### 3. Implement Timer-Based Update System (Matching vgacore.cc:2413-2430)

**Constraint**: Emulator core (VGA, memory, etc.) must use only `core` + `alloc` (no_std). Examples can use `std`.

- **File**: `rusty_box/src/emulator.rs` (in `run_interactive()` - can use std)
- Add periodic VGA update mechanism in the execution loop:
- Call `vga.update(gui)` periodically (every ~100ms or based on instruction count)
- After `update()`, call `gui.flush()` to refresh display
- **File**: `rusty_box/src/iodev/vga.rs` (must be no_std compatible)
- Add `update()` method signature that takes optional GUI reference:
- Method must use only `core` + `alloc` (no std features)
- Can accept `Option<&mut dyn BxGui>` parameter (trait object, no_std compatible)
- Or return text buffer data for example to handle GUI update

### 4. Fix Memory Write Handler (Matching vgacore.cc:1818-2180)

- **File**: `rusty_box/src/iodev/vga.rs`
- In `mem_write()`, set `vga_mem_updated` flag when text mode memory is written
- For text mode (memory_mapping == 3), set appropriate bits in `vga_mem_updated`
- Set `text_buffer_update = true` when text mode parameters change

### 5. Fix Terminal GUI Rendering (Matching term.cc:551-608)

- **File**: `rusty_box/src/gui/term.rs`
- Fix `text_update()` to match original implementation:
- Compare old_text and new_text byte-by-byte (char and attribute)
- Only update characters that changed (line 573-574 in original)
- Use proper color mapping (get_color_pair function)
- Handle cursor visibility and positioning correctly
- Use proper character rendering (get_term_char equivalent)

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

## Architecture Constraints

- **Emulator Core** (`rusty_box/src/`): Must use only `core` + `alloc` (no_std compatible)
- VGA, memory, CPU, devices must not depend on `std`
- Can use `alloc::vec::Vec`, `alloc::collections`, etc.
- **Examples** (`rusty_box/examples/`): Can use `std` features
- Can use `std::time::Instant` for timing
- Can use `std::io` for file I/O
- **GUI** (`rusty_box/src/gui/`): Can use `std` features
- Terminal GUI already uses `std::io`, `std::os::unix`, etc.

## Notes (From Original Bochs Code)

- **VGA Text Mode**: Uses 2 bytes per character: [char, attribute]
- **Text Memory**: 0xB8000-0xBFFFF (32KB, supports multiple pages)
- **Standard Text Mode**: 80x25 = 2000 characters = 4000 bytes per page
- **Update Flow** (vgacore.cc):

1. BIOS writes to VGA memory → `mem_write_handler()` → `mem_write()` → sets `vga_mem_updated` flag
2. Timer fires → `vga_timer_handler()` → `update()` → processes text mode
3. `update()` copies VGA memory to `text_buffer` if needed
4. `update()` calls `bx_gui->text_update_common(old_snapshot, new_buffer, cursor_addr, tm_info)`
5. After update, copies `text_buffer` to `text_snapshot`
6. Calls `bx_gui->flush()` to refresh display

- **Text Buffer Update** (vgacore.cc:1677-1691):
- `text_buffer_update` flag triggers copy from VGA memory to text_buffer
- `vga_mem_updated` flag indicates memory changed and snapshot needs update
- Text buffer is extracted from VGA memory using `memory_mapping` mode
- **Terminal GUI** (term.cc:551-608):
- Compares old_text[0] and old_text[1] with new_text[0] and new_text[1]
- Only updates characters that changed
- Uses curses library for terminal rendering
- Handles cursor visibility based on cursor start/end registers