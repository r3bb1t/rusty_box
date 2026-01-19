# Keyboard Input and Interactive GUI Implementation

## Overview

This document describes the implementation of keyboard input handling and interactive GUI support for the Rusty Box x86 emulator, enabling DLX Linux to boot and accept user input.

## Completed Features

### 1. Keyboard Scancode Mapping (`rusty_box/src/gui/keymap.rs`)

- **PS/2 Scancode Set 2**: Implements complete mapping from ASCII characters to PS/2 scancode set 2
- **Key Support**: 
  - All letters (a-z, A-Z)
  - All numbers (0-9) and shifted symbols
  - Special keys: Enter, Backspace, Escape, Tab, Space
  - Punctuation and symbols
- **Shift Handling**: Automatically generates shift make/break codes for uppercase letters and shifted symbols
- **Break Codes**: Generates proper make/break code sequences (0xF0 prefix)

### 2. Terminal GUI (`rusty_box/src/gui/term.rs`)

- **Raw Terminal Mode**: Sets up terminal for non-blocking keyboard input
  - Unix: Uses `libc::termios` for raw mode
  - Windows: Uses Windows Console API
- **Non-blocking Input**: Reads keyboard input without blocking CPU execution
- **Scancode Queue**: Queues scancodes for processing by emulator
- **VGA Text Rendering**: Displays VGA text mode (80x25) with ANSI color codes
- **Terminal Cleanup**: Properly restores terminal state on exit

### 3. Interactive Execution Loop (`rusty_box/src/emulator.rs`)

- **Event-driven Architecture**: Integrates CPU execution with GUI events
- **Key Features**:
  - Processes keyboard input from GUI
  - Sends scancodes to keyboard device
  - Raises keyboard interrupts via PIC
  - Executes CPU instructions in batches (1000 at a time)
  - Updates GUI display periodically (every 50ms)
  - Handles device interrupts

### 4. VGA Display Optimization (`rusty_box/src/iodev/vga.rs`)

- **Dirty Tracking**: Added `text_dirty` flag to track when VGA text memory changes
- **Efficient Updates**: GUI only updates when text memory has actually changed
- **Performance**: Reduces unnecessary screen redraws

### 5. egui GUI (`rusty_box/src/gui/egui_gui.rs`)

- **Modern GUI**: Optional egui-based GUI for cross-platform visual display
- **Feature Flag**: Controlled by `gui-egui` Cargo feature
- **Text Mode Rendering**: Renders VGA text mode (80x25) in egui window
- **Keyboard Input**: Captures keyboard input via egui's event system
- **Note**: Full integration requires running in separate thread or adapting event loop

### 6. Tests (`rusty_box/src/gui/keymap_tests.rs`)

- **Comprehensive Tests**: Tests for scancode mapping functionality
- **Coverage**: 
  - Letter mappings (a-z, A-Z)
  - Special keys (Enter, Backspace, etc.)
  - Shift handling
  - Scancode sequence generation

## Usage

### Terminal GUI (Default)

```rust
use rusty_box::{gui::TermGui, emulator::Emulator};

let mut emu = Emulator::new(config)?;
emu.initialize()?;

// Create and set terminal GUI
let term_gui = TermGui::new();
emu.set_gui(term_gui);
emu.init_gui(0, &[])?;

// Load BIOS and reset
emu.load_bios(&bios_data)?;
emu.reset(ResetReason::Hardware)?;

// Run interactively
emu.run_interactive(1_000_000_000)?;
```

### egui GUI (Optional)

Enable the `gui-egui` feature:

```toml
[dependencies]
rusty_box = { path = "../rusty_box", features = ["gui-egui"] }
```

Then use `EguiGui` instead of `TermGui`. Note: egui requires its own event loop, so integration may need additional threading or event loop adaptation.

## Architecture

```
┌─────────────┐
│   GUI       │ (TermGui or EguiGui)
│             │
│ - Reads     │
│   keyboard  │
│ - Queues    │
│   scancodes │
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Emulator   │
│             │
│ - Processes │
│   scancodes │
│ - Sends to  │
│   keyboard  │
│   device    │
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Keyboard   │
│   Device    │
│             │
│ - Receives  │
│   scancodes │
│ - Raises    │
│   IRQ1      │
└──────┬──────┘
       │
       ▼
┌─────────────┐
│     PIC     │
│             │
│ - Raises    │
│   interrupt │
│   to CPU    │
└─────────────┘
```

## Key Implementation Details

### Scancode Format

PS/2 Scancode Set 2 uses:
- **Make codes**: 0x01-0x83 (key press)
- **Break codes**: 0xF0 followed by make code (key release)
- **Extended keys**: 0xE0 prefix + make code

### Keyboard Interrupt Flow

1. User types on host keyboard
2. GUI captures character
3. Character converted to scancode sequence
4. Scancodes queued in GUI
5. Emulator processes queue and sends to keyboard device
6. Keyboard device processes scancode and sets IRQ1 flag
7. PIC raises interrupt
8. CPU handles interrupt and processes keyboard input

### VGA Update Flow

1. CPU writes to VGA text memory (0xB8000-0xBFFFF)
2. VGA memory handler sets `text_dirty = true`
3. Emulator's interactive loop checks dirty flag periodically
4. If dirty, GUI is updated with new text
5. Dirty flag is cleared after update

## Testing

Run the keyboard tests:

```bash
cargo test --lib gui::keymap::tests
```

Run the DLX Linux example:

```bash
cargo run --example dlxlinux
```

Type characters in the terminal and they should appear in the emulated system.

## Future Improvements

1. **Mouse Support**: Add mouse input handling
2. **Enhanced egui Integration**: Better threading/event loop integration for egui
3. **Scancode Verification**: Verify all scancode mappings against hardware
4. **Graphics Mode**: Support for VGA graphics modes in GUI
5. **Audio**: Add audio device support for sound output

## References

- PS/2 Keyboard Scancode Set 2 Standard
- Bochs iodev/keyboard.cc (reference implementation)
- egui documentation: https://docs.rs/egui/
