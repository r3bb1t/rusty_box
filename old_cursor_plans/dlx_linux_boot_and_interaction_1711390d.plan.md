---
name: DLX Linux Boot and Interaction
overview: ""
todos: []
---

# DLX Linux Boot and Interaction Implementation

## Overview
Enable DLX Linux to boot successfully and accept keyboard input for interactive use. The implementation will add keyboard input handling, integrate an event loop that processes GUI events and updates the display, and support both terminal and egui-based GUIs.

## Architecture

The main execution loop needs to:
1. Execute CPU instructions in batches
2. Periodically handle GUI events (keyboard input)
3. Update GUI display when VGA text memory changes
4. Process device interrupts

```
CPU Execution Loop:
  └─> Execute instructions (batch)
      └─> Periodically (every N instructions or time):
          ├─> GUI.handle_events() → Read keyboard input
          ├─> Convert input to scancodes → Keyboard device
          ├─> VGA.update_gui() → Refresh display if changed
          └─> Process pending interrupts
```

## Implementation Steps

### 1. Keyboard Scancode Mapping ([rusty_box/src/iodev/keyboard.rs](rusty_box/src/iodev/keyboard.rs))

Add scancode conversion from ASCII/Unicode to PS/2 scancode set 2:

- **Add scancode mapping function**: Create `ascii_to_scancode_set2(char) -> Option<u8>` that maps ASCII characters to make/break scancode pairs
- **Support for common keys**: Letters (a-z, A-Z), numbers (0-9), Enter, Backspace, Space, Escape, Tab, Arrow keys
- **Scancode set 2 format**: 
  - Make codes: 0x01-0x7E for regular keys
  - Extended keys (arrows): 0xE0 prefix + make code
  - Break codes: Make code | 0x80

**Reference**: `cpp_orig/bochs/iodev/keyboard.cc` lines 1000-1500 (scancode tables)

### 2. Terminal GUI Keyboard Input ([rusty_box/src/gui/term.rs](rusty_box/src/gui/term.rs))

Implement `handle_events` method:

- **Read from stdin**: Use non-blocking stdin reading (e.g., `std::io::stdin().read_line()` with timeout or `poll` on Unix)
- **Convert to scancodes**: Call keyboard scancode mapping function
- **Store input queue**: Maintain a queue of pending scancodes to send
- **Return scancodes**: Return an `Option<Vec<u8>>` of scancodes to send

**Implementation notes**:
- Use `std::sync::mpsc` or similar for non-blocking input on Windows
- On Unix, use `select`/`poll` for non-blocking stdin
- Handle Ctrl+C gracefully (may want to pass it through or handle as special case)

### 3. Emulator Event Loop Integration ([rusty_box/src/emulator.rs](rusty_box/src/emulator.rs))

Add a new method `run_with_event_loop` that:

- **Takes GUI as parameter**: Accepts a `Box<dyn BxGui>` to use for display/input
- **Main loop structure**:
  ```rust
  loop {
      // Execute CPU instructions in batches (e.g., 10000 at a time)
      let executed = cpu.cpu_loop_n(&mut memory, &[], 10_000)?;
      
      // Handle GUI events (keyboard input)
      if let Some(ref mut gui) = self.gui {
          gui.handle_events();
          // Get keyboard input from GUI and feed to keyboard device
          if let Some(scancodes) = gui.get_pending_scancodes() {
              for scancode in scancodes {
                  self.device_manager.keyboard.send_scancode(scancode);
              }
          }
      }
      
      // Update GUI display (check if VGA text memory changed)
      self.update_gui();
      
      // Process interrupts
      if self.has_interrupt() {
          let vector = self.iac();
          // Inject interrupt into CPU
      }
      
      // Check for exit condition
      if should_exit {
          break;
      }
  }
  ```

### 4. GUI Trait Extension ([rusty_box/src/gui/gui_trait.rs](rusty_box/src/gui/gui_trait.rs))

Add methods to `BxGui` trait:

- **`get_pending_scancodes(&mut self) -> Option<Vec<u8>>`**: Returns pending keyboard scancodes from the GUI
- **`has_input(&self) -> bool`**: Checks if input is available (for efficient polling)
- **Default implementations**: Provide no-op defaults for existing GUI implementations

### 5. VGA Text Memory Change Detection ([rusty_box/src/iodev/vga.rs](rusty_box/src/iodev/vga.rs))

Add dirty tracking for text memory:

- **Add `text_dirty` flag**: Boolean flag that gets set when text memory is written
- **Clear flag on read**: Reset `text_dirty` when GUI reads text memory
- **Set flag in write handlers**: When VGA text memory is written, set `text_dirty = true`
- **Update `update_gui` in emulator**: Only call `gui.text_update()` if `text_dirty` is set

### 6. Update DLX Linux Example ([rusty_box/examples/dlxlinux.rs](rusty_box/examples/dlxlinux.rs))

Modify the example to:

- **Create and set GUI**: Create a `TermGui` instance and set it on the emulator
- **Use event loop**: Replace `cpu_loop_n` call with `run_with_event_loop` method
- **Handle exit**: Detect when system wants to exit (e.g., reboot command) and break the loop
- **Configure terminal**: Set terminal to raw mode for proper input handling (on Unix: `libc::termios`, on Windows: console mode APIs)

### 7. egui GUI Implementation ([rusty_box/src/gui/egui.rs](rusty_box/src/gui/egui.rs)) - NEW FILE

Create egui-based GUI implementation:

- **Add egui dependency**: Add `egui = "0.24"` and `eframe = "0.24"` to `Cargo.toml`
- **Implement `BxGui` trait**: Create `EguiGui` struct that implements all trait methods
- **Text mode rendering**: Render VGA text mode (80x25) as a grid of characters in egui
- **Keyboard input**: Use egui's input handling to capture key presses and convert to scancodes
- **Window setup**: Create a native window with egui using `eframe::run_native()`

**Key implementation details**:
- Use `egui::TextEdit` or custom drawing for text mode display
- Map egui key events to keyboard scancodes
- Handle window close/exit gracefully

### 8. Update Cargo.toml ([rusty_box/Cargo.toml](rusty_box/Cargo.toml))

Add optional dependencies:

```toml
[features]
default = ["std"]
std = []
egui = ["dep:egui", "dep:eframe"]

[dependencies]
# ... existing dependencies ...
egui = { version = "0.24", optional = true }
eframe = { version = "0.24", optional = true }
```

## Files to Modify

1. **[rusty_box/src/iodev/keyboard.rs](rusty_box/src/iodev/keyboard.rs)**: Add scancode mapping functions
2. **[rusty_box/src/gui/term.rs](rusty_box/src/gui/term.rs