# Bochs CPU Emulator - Device Initialization Sequence Analysis

## Overview
This document details the device initialization sequence in the Bochs x86 CPU emulator, extracted from the C++ source code (cpp_orig/bochs/).

---

## 1. Main Initialization Entry Points

### Starting Point: `bx_init_main()` - [main.cc:614](main.cc#L614)
```
Function: void bx_init_bx_dbg(void)
Purpose: Initialize debugger state
Action: Called at [main.cc:614]
```

### Hardware Initialization: `bx_init_hardware()` - [main.cc:1300-1400]
This is the **main hardware initialization function** that:
1. Initializes the PC system (timer, interrupts, DMA)
2. Loads memory and BIOS
3. Initializes CPUs
4. **Calls `DEV_init_devices()`** at [main.cc:1353]
5. Resets the system

---

## 2. Device Initialization Flow

### Step 1: PC System Initialization - [main.cc:1301]
```cpp
bx_pc_system.initialize(SIM->get_param_num(BXPN_IPS)->get());
```
**What it does**: Initializes the PC system timer infrastructure.
- Sets up instruction-per-second timing
- Initializes timer array
- Prepares interrupt handling

**File**: [pc_system.cc:61]
**Type**: bx_pc_system_c::initialize()

---

### Step 2: Memory and CPU Initialization - [main.cc:1331-1351]
```cpp
// Memory initialization
BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);
BX_MEM(0)->load_ROM(...);  // Load BIOS
BX_MEM(0)->load_RAM(...);  // Load optional RAM images

// CPU initialization
BX_CPU(0)->initialize();    // Initialize CPU
BX_CPU(0)->sanity_checks(); // Validate CPU state
BX_CPU(0)->register_state(); // Register save/restore state
```

---

### Step 3: Device Initialization - [main.cc:1353]
```cpp
DEV_init_devices();
```
This macro expands to: `bx_devices.init(BX_MEM(0));`

**File**: [iodev/devices.cc:116] - `bx_devices_c::init()`

This is the **core device initialization function** that loads all hardware devices.

---

## 3. Device Plugin Loading Sequence

The `bx_devices_c::init()` function loads devices in this order:

### A. I/O Handler Registration - [devices.cc:119-145]
```cpp
register_default_io_read_handler(NULL, &default_read_handler, def_name, 7);
register_default_io_write_handler(NULL, &default_write_handler, def_name, 7);
read_port_to_handler = new struct io_handler_struct *[PORTS];
write_port_to_handler = new struct io_handler_struct *[PORTS];
```
**Purpose**: Initialize default I/O port handler tables (sets all 65536 I/O ports to default handlers)

### B. Removable Devices Initialization - [devices.cc:147-163]
```cpp
// Keyboard and mouse device stubs
for (i=0; i < 2; i++) {
    bx_keyboard[i].dev = NULL;
    bx_keyboard[i].gen_scancode = NULL;
    bx_keyboard[i].led_mask = 0;
}
for (i=0; i < 2; i++) {
    bx_mouse[i].dev = NULL;
    bx_mouse[i].enq_event = NULL;
    bx_mouse[i].enabled_changed = NULL;
}
```
**Devices Initialized**: Keyboard, Mouse input devices

### C. Timer Devices - [devices.cc:177-179]
```cpp
bx_virt_timer.init();      // Virtual timer
bx_slowdown_timer.init();  // Slowdown timer
```
**Devices Initialized**: Virtual timer, Slowdown timer

### D. Core Devices (PCI/ISA Architecture) - [devices.cc:183-250]

#### If PCI Enabled - [devices.cc:188-232]
```cpp
PLUG_load_plugin(pci, PLUGTYPE_CORE);        // PCI controller
PLUG_load_plugin(pci2isa, PLUGTYPE_CORE);    // PCI-to-ISA bridge
PLUG_load_plugin(acpi, PLUGTYPE_STANDARD);   // ACPI (if not disabled)
PLUG_load_plugin(hpet, PLUGTYPE_STANDARD);   // High Precision Event Timer (if not disabled)
```
**I/O Ports Used**:
- **0x0CF8**: PCI Configuration Address Register
- **0x0CFC-0x0CFF**: PCI Configuration Data Registers
- **0x0092**: Port 92h System Control (A20 line, reset)

#### Core ISA Devices - [devices.cc:245-262]
```cpp
PLUG_load_plugin(cmos, PLUGTYPE_CORE);       // CMOS RTC & configuration
PLUG_load_plugin(dma, PLUGTYPE_CORE);        // DMA controller
PLUG_load_plugin(pic, PLUGTYPE_CORE);        // Programmable Interrupt Controller
PLUG_load_plugin(pit, PLUGTYPE_CORE);        // Programmable Interval Timer
PLUG_load_plugin(keyboard, PLUGTYPE_STANDARD);  // Keyboard controller
PLUG_load_plugin(floppy, PLUGTYPE_CORE);     // Floppy disk controller
```

**I/O Port Ranges Registered** (via `register_io_read_handler()` / `register_io_write_handler()`):
- **Port 0x0092**: System Control Port (A20 enable, soft reset)

### E. APIC and I/O APIC - [devices.cc:264-265]
```cpp
#if BX_SUPPORT_APIC
PLUG_load_plugin(ioapic, PLUGTYPE_STANDARD);  // I/O APIC
#endif
```
**Purpose**: Advanced Programmable Interrupt Controller for multi-processor systems

### F. Storage Devices - [devices.cc:267-276]
```cpp
PLUG_load_plugin(pci_ide, PLUGTYPE_STANDARD);   // IDE controller (if PCI enabled)
PLUG_load_plugin(harddrv, PLUGTYPE_STANDARD);   // Hard drive emulation (if enabled)
```

### G. Optional Devices
```cpp
// Bus mouse (if mouse type is INPORT or BUS)
if ((mouse_type == BX_MOUSE_TYPE_INPORT) || (mouse_type == BX_MOUSE_TYPE_BUS)) {
    SIM->opt_plugin_ctrl("busmouse", 1);
}
```

---

## 4. CMOS Memory Configuration - [devices.cc:315-340]

After device loading, CMOS is configured with system information:

```cpp
// Base memory (first 640K)
DEV_cmos_set_reg(0x15, (Bit8u) BASE_MEMORY_IN_K);
DEV_cmos_set_reg(0x16, (Bit8u) (BASE_MEMORY_IN_K >> 8));

// Extended memory (above 1MB)
DEV_cmos_set_reg(0x17, (Bit8u) (extended_memory_in_k & 0xff));
DEV_cmos_set_reg(0x18, (Bit8u) ((extended_memory_in_k >> 8) & 0xff));

// Extended memory 64K blocks (above 16MB)
DEV_cmos_set_reg(0x34, (Bit8u) (extended_memory_in_64k & 0xff));
DEV_cmos_set_reg(0x35, (Bit8u) ((extended_memory_in_64k >> 8) & 0xff));

// Memory above 4GB
if (memory_above_4gb) {
    DEV_cmos_set_reg(0x5b, (Bit8u)(memory_above_4gb >> 16));
    DEV_cmos_set_reg(0x5c, (Bit8u)(memory_above_4gb >> 24));
    DEV_cmos_set_reg(0x5d, memory_above_4gb >> 32);
}
```

---

## 5. Device Registration and Plugin Cleanup - [devices.cc:343-365]

```cpp
DEV_register_state();  // Register device state for save/restore

// Unload optional plugins which are unused and marked for removal
SIM->opt_plugin_ctrl("*", 0);

// Verify PCI device configuration
// (checks that all configured PCI devices were actually loaded)
```

---

## 6. System Reset and Initialization - [main.cc:1360] & [pc_system.cc:187]

### PC System Reset
```cpp
bx_pc_system.Reset(BX_RESET_HARDWARE);
```

**File**: [pc_system.cc:187] - `bx_pc_system_c::Reset()`

**What it does**:
```cpp
// Enable A20 line
set_enable_a20(1);

// Reset all CPUs
for (int i=0; i<BX_SMP_PROCESSORS; i++) {
    BX_CPU(i)->reset(type);  // BX_RESET_HARDWARE or BX_RESET_SOFTWARE
}

// Reset all devices (hardware reset only)
if (type==BX_RESET_HARDWARE) {
    DEV_reset_devices(type);  // Expands to: bx_devices.reset(type);
}
```

### Device Reset - [iodev/devices.cc:628-640]
```cpp
void bx_devices_c::reset(unsigned type)
{
#if BX_SUPPORT_PCI
  if (pci.enabled) {
    pci.confAddr = 0;  // Clear PCI configuration address
  }
#endif
  mem->disable_smram();  // Disable System Management RAM
  bx_reset_plugins(type);  // Reset all loaded device plugins
  release_keys();  // Release any held keyboard keys
  if (paste.buf != NULL) {
    paste.stop = 1;  // Stop paste operation if in progress
  }
}
```

---

## 7. Timer and Signal Handler Setup - [main.cc:1365-1390]

```cpp
// Initialize GUI signal handlers
bx_gui->init_signal_handlers();

// Start all registered timers
bx_pc_system.start_timers();

// Set up SIGINT handler (Ctrl+C)
signal(SIGINT, bx_signal_handler);

#if BX_SHOW_IPS
// Set up timer for displaying instructions per second
signal(SIGALRM, bx_signal_handler);
alarm(1);
#endif
```

---

## 8. CPU Main Loop - [main.cc:1370-1450]

After hardware initialization, the CPU execution begins:

```cpp
// Single processor mode
while (1) {
    BX_CPU(0)->cpu_loop();
    if (bx_pc_system.kill_bochs_request)
        break;
}

// Or multi-processor mode (SMP)
// - Quantum-based execution of multiple CPUs
// - Each CPU executes a trace, then switches to next CPU
```

---

## 9. I/O Port Address Mapping Summary

### System Control
| Port | Name | Register | Purpose |
|------|------|----------|---------|
| 0x0092 | Port 92h | Read/Write | A20 line enable, Software reset |

### PCI Configuration (if enabled)
| Port Range | Name | Purpose |
|------------|------|---------|
| 0x0CF8 | PCI Address | Select PCI device/register |
| 0x0CFC-0x0CFF | PCI Data | Read/write PCI device config |

### Chipset Devices
| Device | Type | Port Range | Purpose |
|--------|------|-----------|---------|
| **DMA** | ISA | 0x0000-0x000F, 0x0080-0x008F, 0x00C0-0x00DF | Direct Memory Access controller |
| **PIC** | ISA | 0x0020-0x003F, 0x00A0-0x00BF | Interrupt controller |
| **PIT** | ISA | 0x0040-0x005F | Timer (system clock 18.2 Hz, speaker, etc.) |
| **CMOS** | ISA | 0x0070-0x0071 | Real-time clock & NVRAM |
| **Keyboard** | ISA | 0x0060, 0x0064 | Keyboard controller |
| **Floppy** | ISA | 0x03F0-0x03F7 | Floppy disk controller |
| **IDE** | ISA/PCI | 0x01F0-0x01F7, 0x03F6-0x03F7 | IDE disk controller (primary) |

---

## 10. Key Initialization Functions Called

| Function | File | Line | Purpose |
|----------|------|------|---------|
| `bx_init_main()` | main.cc | 1266 | Initial startup, parse command-line |
| `bx_init_hardware()` | main.cc | 1300 | Main hardware initialization |
| `DEV_init_devices()` | devices.cc | 116 (via macro in plugin.h:141) | Initialize and load device plugins |
| `bx_devices_c::init()` | devices.cc | 116 | Core device initialization |
| `bx_pc_system.initialize()` | pc_system.cc | 61 | Initialize PC system timer |
| `bx_pc_system.Reset()` | pc_system.cc | 187 | Hardware reset, enable A20, reset CPU/devices |
| `bx_devices_c::reset()` | devices.cc | 628 | Reset all device plugins |
| `bx_devices_c::register_state()` | devices.cc | 645 | Register device state for save/restore |
| `bx_pc_system.start_timers()` | pc_system.cc | 472 | Activate all timers |

---

## 11. Memory Mapping for Hardware Devices

### BIOS ROM
- **Address**: 0xE0000 - 0xFFFFF (128 KB)
- **Type**: Read-only BIOS firmware
- **Loaded by**: `BX_MEM(0)->load_ROM()` at [main.cc:1309]

### A20 Line Control
- **Port**: 0x0092 (bit 1)
- **Purpose**: Enable/disable address line 20 for extended memory access
- **Controlled by**: System Control Port handler in devices.cc

### PCI Configuration Space
- **Method**: Configuration address/data port I/O
- **Address Port**: 0x0CF8
- **Data Ports**: 0x0CFC-0x0CFF
- **Format**: Address bits specify bus/device/function/register

### ISA I/O Space
- **Range**: 0x0000 - 0xFFFF (64 KB)
- **Standard PC Devices**: Mapped to standard ISA I/O ports

---

## 12. Execution Flow Summary

```
main()
  ↓
bx_init_main()
  - Parse command-line arguments
  - Initialize plugins system
  - Load configuration file
  ↓
SIM->configuration_interface()
  - Load GUI/config interface
  ↓
bx_begin_simulation()
  ↓
bx_init_hardware()
  - bx_pc_system.initialize()       ← Timer init
  - BX_MEM(0)->init_memory()        ← Memory allocation
  - BX_MEM(0)->load_ROM()           ← BIOS loading
  - BX_CPU(i)->initialize()         ← CPU init
  - DEV_init_devices()              ← DEVICE LOADING
    * PCI controller (if enabled)
    * PCI-to-ISA bridge
    * CMOS RTC
    * DMA controller
    * PIC (interrupt controller)
    * PIT (timer)
    * Keyboard controller
    * Floppy controller
    * IDE controller
    * Hard drive
    * I/O APIC (for SMP)
  - bx_pc_system.Reset(BX_RESET_HARDWARE)  ← Hardware reset
    * Enable A20 line
    * Reset all CPUs
    * Reset all devices
  - bx_gui->init_signal_handlers()  ← Signal setup
  - bx_pc_system.start_timers()     ← Start timers
  ↓
CPU Main Loop
  - BX_CPU(0)->cpu_loop()           ← Execute instructions
  - Interrupt handling
  - Device I/O operations
```

---

## 13. Plugin System Functions

The Bochs plugin system uses macros for device management:

```cpp
#define PLUG_load_plugin(sym, type)
    - Load a device plugin by symbol name
    - Example: PLUG_load_plugin(pic, PLUGTYPE_CORE)

#define PLUG_load_plugin_var(name, type)
    - Load a plugin by string name
    - Example: PLUG_load_plugin_var(BX_PLUGIN_VGA, PLUGTYPE_VGA)

#define PLUG_device_present(name)
    - Check if a device is already loaded
```

Plugin types:
- **PLUGTYPE_CORE**: Essential devices (PIC, PIT, DMA, CMOS, etc.)
- **PLUGTYPE_STANDARD**: Standard devices (keyboard, IDE, floppy, etc.)
- **PLUGTYPE_VGA**: Video device
- **PLUGTYPE_GUI**: Display/GUI
- **PLUGTYPE_CI**: Configuration interface

---

## 14. State Registration and Save/Restore

Devices register their state for save/restore functionality:

```cpp
bx_devices.register_state();   // Register device state
bx_pc_system.register_state(); // Register PC system state
BX_CPU(i)->register_state();   // Register CPU state
```

This allows the emulator to:
- Save complete system state to disk
- Restore and continue execution
- Debug state inspection

---

## Conclusion

The Bochs device initialization follows this pattern:

1. **System Initialization**: Timer and PC system setup
2. **Memory Setup**: BIOS and RAM allocation  
3. **CPU Initialization**: CPU core setup and state registration
4. **Device Plugin Loading**: Load all hardware devices in order
5. **Memory Configuration**: Set up CMOS with system memory info
6. **Hardware Reset**: Full reset sequence (A20, CPU, devices)
7. **Timer Activation**: Start all event timers
8. **Execution**: Begin CPU instruction loop

All device initialization happens through a plugin system that allows dynamic loading of device implementations, making Bochs highly modular and configurable.
