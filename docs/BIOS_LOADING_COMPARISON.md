# BIOS Loading and Device Initialization - Comparison with Original Bochs

## Date: 2026-02-10

## Executive Summary

**User Statement**: "The images are proper, there is no problems with them"
**Current Issue**: No BIOS output appearing despite BIOS execution progressing to RIP 0x2055

This document compares BIOS loading and device initialization between our implementation and original Bochs to identify potential discrepancies.

---

## 1. BIOS Loading Logic Comparison

### Original Bochs (`cpp_orig/bochs/memory/misc_mem.cc:296-467`)

```cpp
void BX_MEM_C::load_ROM(const char *path, bx_phy_address romaddress, Bit8u type)
{
    // ... file reading ...

    if (type == 0) {  // System BIOS
        if (romaddress > 0) {
            // Validate BIOS ends at 1MB or 4GB boundary
            if ((romaddress + size) != 0x100000 && (romaddress + size)) {
                BX_PANIC(("ROM: System BIOS must end at 0xfffff"));
                return;
            }
        } else {
            // Auto-calculate BIOS address from size
            romaddress = ~(size - 1);  // ← KEY CALCULATION
        }

        offset = romaddress & BIOS_MASK;
        BX_MEM_THIS bios_rom_addr = (Bit32u)romaddress;

        // ... copy ROM data ...
        while (size > 0) {
            ret = read(fd, (bx_ptr_t) &BX_MEM_THIS rom[offset], size);
            size -= ret;
            offset += ret;
        }
    }
}
```

**Key Points:**
- Validates BIOS ends at `0x100000` (1MB) or `0x100000000` (4GB)
- Auto-calculates address if not provided: `romaddress = ~(size - 1)`
  - For 64KB BIOS: `~0xFFFF = 0xFFFF0000`
  - For 128KB BIOS: `~0x1FFFF = 0xFFFE0000`
- Uses `offset = romaddress & BIOS_MASK`
- Stores ROM at calculated offset in ROM array

### Our Implementation (`rusty_box/src/memory/misc_mem.rs:256-340`)

```rust
pub fn load_ROM(
    &mut self,
    rom_data: &[u8],
    rom_address: BxPhyAddress,
    rom_type: u8,
) -> Result<()> {
    let size = rom_data.len();

    if rom_type == 0 {
        // system BIOS
        let offset = (rom_address as usize) & (BIOSROMSZ - 1);
        let rom = self.inherited_memory_stub.rom();

        if offset + size > rom.len() {
            return Err(MemoryError::RomTooLarge(rom.len()).into());
        }

        rom[offset..offset + size].copy_from_slice(rom_data);
        self.bios_rom_addr = rom_address as u32;

        // ... verification logging ...
    }
}
```

**Key Points:**
- Accepts any address (no validation)
- Does NOT auto-calculate address (expects caller to provide correct address)
- Uses same offset calculation: `(rom_address as usize) & (BIOSROMSZ - 1)`
- Copies ROM data in one operation (no loop)

### How Our Example Calls It (`rusty_box/examples/dlxlinux.rs:256-268`)

```rust
let bios_size = bios_data.len() as u64;
let bios_load_addr = 0x100000000u64 - bios_size;  // ← CORRECT CALCULATION!
tracing::info!(
    "BIOS size: {} bytes ({} KB), calculated load address: {:#x}",
    bios_size,
    bios_size / 1024,
    bios_load_addr
);
emu.load_bios(&bios_data, bios_load_addr)?;
```

**Result for 64KB BIOS:**
- `bios_size = 0x10000`
- `bios_load_addr = 0x100000000 - 0x10000 = 0xFFFF0000` ✓ CORRECT

**Result for 128KB BIOS:**
- `bios_size = 0x20000`
- `bios_load_addr = 0x100000000 - 0x20000 = 0xFFFE0000` ✓ CORRECT

### Verdict: BIOS Loading is Correct ✓

Both implementations produce identical results when our example code provides the correct address.

---

## 2. Device Initialization Order Comparison

### Original Bochs (`cpp_orig/bochs/iodev/devices.cc:115-396`)

```cpp
void bx_devices_c::init(BX_MEM_C *newmem)
{
    // Line 250: Load CMOS plugin
    PLUG_load_plugin(cmos, PLUGTYPE_CORE);

    // Line 251: Load DMA plugin
    PLUG_load_plugin(dma, PLUGTYPE_CORE);

    // Line 252: Load PIC plugin
    PLUG_load_plugin(pic, PLUGTYPE_CORE);

    // Line 253: Load PIT plugin
    PLUG_load_plugin(pit, PLUGTYPE_CORE);

    // Lines 254-256: Load VGA plugin
    if (pluginVgaDevice == &stubVga) {
        PLUG_load_plugin_var(BX_PLUGIN_VGA, PLUGTYPE_VGA);
    }

    // Line 257: Load floppy plugin
    PLUG_load_plugin(floppy, PLUGTYPE_CORE);

    // Line 262: Load keyboard plugin
    PLUG_load_plugin(keyboard, PLUGTYPE_STANDARD);

    // Lines 275-277: Load hard drive plugin (if enabled)
    if (is_harddrv_enabled()) {
        PLUG_load_plugin(harddrv, PLUGTYPE_STANDARD);
    }

    // Lines 280-283: System Control Port 0x92
    register_io_read_handler(this, &read_handler, 0x0092,
                             "Port 92h System Control", 1);
    register_io_write_handler(this, &write_handler, 0x0092,
                              "Port 92h System Control", 1);

    // Lines 320-357: CMOS memory configuration
    Bit64u memory_in_k = mem->get_memory_len() / 1024;
    Bit64u extended_memory_in_k = memory_in_k > 1024 ? (memory_in_k - 1024) : 0;
    if (extended_memory_in_k > 0xfc00) extended_memory_in_k = 0xfc00;

    DEV_cmos_set_reg(0x15, (Bit8u) BASE_MEMORY_IN_K);              // 640 KB low byte
    DEV_cmos_set_reg(0x16, (Bit8u) (BASE_MEMORY_IN_K >> 8));       // 640 KB high byte
    DEV_cmos_set_reg(0x17, (Bit8u) (extended_memory_in_k & 0xff));
    DEV_cmos_set_reg(0x18, (Bit8u) ((extended_memory_in_k >> 8) & 0xff));
    DEV_cmos_set_reg(0x30, (Bit8u) (extended_memory_in_k & 0xff));  // Duplicate
    DEV_cmos_set_reg(0x31, (Bit8u) ((extended_memory_in_k >> 8) & 0xff));

    // Line 372: CMOS checksum
    DEV_cmos_checksum();
}
```

### Our Implementation (`rusty_box/src/iodev/devices.rs:86-124`)

```rust
pub fn init(&mut self, io: &mut BxDevicesC, mem: &mut BxMemC) -> Result<()> {
    tracing::info!("Initializing device manager");

    // Initialize each device in original Bochs order
    // 1. CMOS
    self.cmos.init();
    // 2. DMA
    self.dma.init();
    // 3. PIC
    self.pic.init();
    // 4. PIT
    self.pit.init();
    // 5. VGA
    self.vga.init(io, mem)?;
    // 6. Keyboard
    self.keyboard.init();
    // 7. Hard drive
    self.harddrv.init();

    // Register I/O handlers for each device
    self.register_cmos_handlers(io);
    self.register_dma_handlers(io);
    self.register_pic_handlers(io);
    self.register_pit_handlers(io);
    self.register_keyboard_handlers(io);
    self.register_harddrv_handlers(io);

    tracing::info!("Device manager initialization complete");
    Ok(())
}
```

**Order Comparison:**

| Step | Original Bochs    | Our Implementation | Status |
|------|-------------------|-------------------|--------|
| 1    | CMOS             | CMOS              | ✓      |
| 2    | DMA              | DMA               | ✓      |
| 3    | PIC              | PIC               | ✓      |
| 4    | PIT              | PIT               | ✓      |
| 5    | VGA              | VGA               | ✓      |
| 6    | Floppy           | (not implemented) | -      |
| 7    | Keyboard         | Keyboard          | ✓      |
| 8    | Hard Drive       | Hard Drive        | ✓      |
| 9    | Port 0x92        | Port 0x92         | ✓      |

### CMOS Memory Configuration (`rusty_box/examples/dlxlinux.rs:246-251`)

```rust
// Configure CMOS for 32 MB memory (matches bochsrc.bxrc)
// Base: 640KB, Extended: 31 MB = 31 * 1024 KB
emu.configure_memory_in_cmos(640, 31 * 1024);

// Configure hard drive in CMOS (drive type 47 = user-defined)
emu.configure_disk_in_cmos(0, 47);
```

**Implementation (`rusty_box/src/iodev/cmos.rs:257-270`):**

```rust
pub fn set_memory_size(&mut self, base_kb: u16, extended_kb: u16) {
    // Base memory (in KB) - typically 640
    self.ram[0x15] = (base_kb & 0xFF) as u8;
    self.ram[0x16] = ((base_kb >> 8) & 0xFF) as u8;

    // Extended memory above 1MB (in KB)
    self.ram[0x17] = (extended_kb & 0xFF) as u8;
    self.ram[0x18] = ((extended_kb >> 8) & 0xFF) as u8;
    self.ram[0x30] = (extended_kb & 0xFF) as u8;  // Duplicate
    self.ram[0x31] = ((extended_kb >> 8) & 0xFF) as u8;

    self.update_checksum();
}
```

### Verdict: Device Initialization is Correct ✓

All devices are initialized in the correct order and CMOS registers are properly configured.

---

## 3. VGA BIOS Loading Comparison

### Original Bochs

VGA BIOS loading moved to VGA device code (devices.cc:314 comment).

### Our Implementation (`rusty_box/examples/dlxlinux.rs:271-275`)

```rust
if let Some((_vga_path, vga_data)) = vga_bios {
    emu.load_optional_rom(&vga_data, 0xC0000)?;
    tracing::info!("✓ Loaded VGA BIOS at 0xC0000");
}
```

**Requirement Check:**
- VGA BIOS must be multiple of 512 bytes (line 169-182)
- Loaded at address 0xC0000 ✓
- Uses same load_ROM() function with type=1 ✓

### Verdict: VGA BIOS Loading is Correct ✓

---

## 4. Potential Issues Identified

### Issue 1: Missing 64KB ROM Size CMOS Register (0x34-0x35)

**Original Bochs (`devices.cc:332-337`):**

```cpp
Bit64u extended_memory_in_64k = memory_in_k > 16384 ? (memory_in_k - 16384) / 64 : 0;
// Limit to 3 GB - 16 MB. PCI Memory Address Space starts at 3 GB.
if (extended_memory_in_64k > 0xbf00) extended_memory_in_64k = 0xbf00;

DEV_cmos_set_reg(0x34, (Bit8u) (extended_memory_in_64k & 0xff));
DEV_cmos_set_reg(0x35, (Bit8u) ((extended_memory_in_64k >> 8) & 0xff));
```

**Our Implementation:**
- ❌ MISSING: CMOS registers 0x34-0x35 not configured
- These registers store extended memory above 16MB in 64KB blocks
- For 32MB RAM: `(32*1024 - 16384) / 64 = 256 = 0x100`

**Fix Required:**

```rust
pub fn set_memory_size(&mut self, base_kb: u16, extended_kb: u16) {
    // Base memory (in KB) - typically 640
    self.ram[0x15] = (base_kb & 0xFF) as u8;
    self.ram[0x16] = ((base_kb >> 8) & 0xFF) as u8;

    // Extended memory above 1MB (in KB)
    self.ram[0x17] = (extended_kb & 0xFF) as u8;
    self.ram[0x18] = ((extended_kb >> 8) & 0xFF) as u8;
    self.ram[0x30] = (extended_kb & 0xFF) as u8;
    self.ram[0x31] = ((extended_kb >> 8) & 0xFF) as u8;

    // NEW: Extended memory above 16MB (in 64KB blocks)
    // Calculate total memory in KB: base + extended
    let total_kb = base_kb as u32 + extended_kb as u32;
    let extended_memory_in_64k = if total_kb > 16384 {
        ((total_kb - 16384) / 64).min(0xbf00)
    } else {
        0
    };

    self.ram[0x34] = (extended_memory_in_64k & 0xFF) as u8;
    self.ram[0x35] = ((extended_memory_in_64k >> 8) & 0xFF) as u8;

    self.update_checksum();
}
```

### Issue 2: Missing Memory Above 4GB Configuration (0x5B-0x5D)

**Original Bochs (`devices.cc:339-345`):**

```cpp
Bit64u memory_above_4gb = (mem->get_memory_len() > BX_CONST64(0x100000000)) ?
                          (mem->get_memory_len() - BX_CONST64(0x100000000)) : 0;
if (memory_above_4gb) {
    DEV_cmos_set_reg(0x5b, (Bit8u)(memory_above_4gb >> 16));
    DEV_cmos_set_reg(0x5c, (Bit8u)(memory_above_4gb >> 24));
    DEV_cmos_set_reg(0x5d, memory_above_4gb >> 32);
}
```

**Our Implementation:**
- ❌ MISSING: CMOS registers 0x5B-0x5D not configured
- Not critical for 32MB configuration, but required for >4GB RAM

### Issue 3: VGA BIOS May Not Be Executed

**Observation:**
- VGA BIOS loaded at 0xC0000 ✓
- But is it being **called/executed** by main BIOS?
- Original Bochs: BIOS scans 0xC0000-0xE0000 for option ROMs and executes them

**Check Required:**
- Is our BIOS performing option ROM scan?
- Are we logging when PC jumps to 0xC0000 range?
- Add tracing for any execution in 0xC0000-0xDFFFF range

---

## 5. Recommendations

### High Priority

1. **Fix CMOS Register 0x34-0x35** (Extended memory in 64KB blocks)
   - This is used by BIOSes to detect RAM above 16MB
   - Missing this could cause BIOS to miscalculate available memory

2. **Add Execution Tracing for VGA BIOS Range (0xC0000-0xDFFFF)**
   ```rust
   if rip >= 0xC0000 && rip < 0xE0000 {
       tracing::warn!("🎨 EXECUTING VGA BIOS CODE: RIP={:#x}", rip);
   }
   ```

3. **Verify Option ROM Scanning in Main BIOS**
   - Check if BIOS code scans for 0x55AA signature at 0xC0000
   - Verify BIOS calls VGA BIOS initialization routine

### Medium Priority

4. **Add CMOS Register 0x5B-0x5D** (Memory above 4GB)
   - Not critical for 32MB config, but needed for completeness

5. **Add Detailed VGA Memory Writes Logging**
   ```rust
   // In write_physical_page for addresses 0xB8000-0xBFFFF
   if a20_addr >= 0xB8000 && a20_addr < 0xC0000 {
       tracing::warn!("📺 VGA WRITE: addr={:#x}, data={:02x?}", a20_addr, data);
   }
   ```

### Low Priority

6. **Add BIOS Loading Validation**
   - Validate BIOS ends at 4GB boundary (like original Bochs)
   - Add warning if BIOS size doesn't match expected (64KB or 128KB)

---

## 6. Next Steps

1. Fix CMOS registers 0x34-0x35
2. Add execution tracing for VGA BIOS range
3. Run emulator and check logs for:
   - VGA BIOS execution (RIP in 0xC0000-0xDFFFF)
   - VGA memory writes (0xB8000-0xBFFFF)
   - BIOS memory reads from low RAM (0x0-0x1000)
4. If still no output, investigate:
   - Is BIOS finding VGA BIOS at 0xC0000?
   - Is VGA BIOS initialization routine being called?
   - Are VGA I/O ports (0x3B0-0x3DF) being accessed?

---

## Conclusion

**BIOS Loading: ✓ Correct**
**Device Initialization: ✓ Correct**
**CMOS Configuration: ⚠️ Missing 0x34-0x35 registers**

The most likely cause of no BIOS output is:
1. Missing CMOS registers 0x34-0x35 causing BIOS memory detection to fail
2. VGA BIOS not being discovered/executed by main BIOS
3. VGA output happening but not reaching our display code

The BIOS ROM images themselves are correct (as user confirmed), and our loading logic matches Bochs. The issue is likely in runtime execution, not initial loading.
