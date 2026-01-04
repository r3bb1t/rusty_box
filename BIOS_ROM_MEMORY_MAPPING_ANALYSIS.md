# BIOS ROM Memory Mapping in Bochs CPU Emulator

## Summary

This document provides a detailed analysis of how BIOS ROM memory mapping works in the Bochs CPU emulator, including where handlers are registered, memory address ranges, and the memory handler chain setup.

---

## 1. Memory Address Ranges for BIOS ROM

### Memory Map Inside the 1st Megabyte

**File**: [cpp_orig/bochs/memory/memory.cc](cpp_orig/bochs/memory/memory.cc#L26-L37)

```cpp
// Memory map inside the 1st megabyte:
//
// 0x00000 - 0x7ffff    DOS area (512K)
// 0x80000 - 0x9ffff    Optional fixed memory hole (128K)
// 0xa0000 - 0xbffff    Standard PCI/ISA Video Mem / SMMRAM (128K)
// 0xc0000 - 0xdffff    Expansion Card BIOS and Buffer Area (128K)
// 0xe0000 - 0xeffff    Lower BIOS Area (64K)
// 0xf0000 - 0xfffff    Upper BIOS Area (64K)
```

### Memory Area Enumerations

**File**: [cpp_orig/bochs/memory/memory-bochs.h](cpp_orig/bochs/memory/memory-bochs.h#L114-L128)

The memory system divides the ROM address space (0xc0000-0xfffff) into 13 areas:

```cpp
enum memory_area_t {
  BX_MEM_AREA_C0000 = 0,   // Expansion ROM start (0xc0000)
  BX_MEM_AREA_C4000,
  BX_MEM_AREA_C8000,
  BX_MEM_AREA_CC000,
  BX_MEM_AREA_D0000,
  BX_MEM_AREA_D4000,
  BX_MEM_AREA_D8000,
  BX_MEM_AREA_DC000,
  BX_MEM_AREA_E0000,       // Lower BIOS (0xe0000)
  BX_MEM_AREA_E4000,
  BX_MEM_AREA_E8000,
  BX_MEM_AREA_EC000,
  BX_MEM_AREA_F0000        // Upper BIOS (0xf0000)
};
```

Each area corresponds to a 16KB block:
- **0xc0000-0xdffff**: Expansion Card BIOS and Buffer Area (128K) - 8 areas
- **0xe0000-0xfffff**: System BIOS Area (128K) - 5 areas

### ROM Constants

**File**: [cpp_orig/bochs/memory/memory-bochs.h](cpp_orig/bochs/memory/memory-bochs.h#L39-L45)

```cpp
const Bit32u BIOSROMSZ = (1 << 22);    //   4M BIOS ROM @0xffc00000, must be a power of 2
const Bit32u EXROMSIZE = (0x20000);    // ROMs 0xc0000-0xdffff (area 0xe0000-0xfffff=bios mapped)

const Bit32u BIOS_MASK  = BIOSROMSZ-1;
const Bit32u EXROM_MASK = EXROMSIZE-1;

#define BIOS_MAP_LAST128K(addr) (((addr) | 0xfff00000) & BIOS_MASK)
```

This means:
- **BIOSROMSZ**: 4MB total ROM space (power of 2 requirement)
- **EXROMSIZE**: 128KB for expansion ROMs (0xc0000-0xdffff)
- **BIOS_MAP_LAST128K**: Maps addresses in 0xe0000-0xfffff range to the last 128KB of ROM buffer

---

## 2. Memory Handler Structure

### Handler Definition

**File**: [cpp_orig/bochs/memory/memory-bochs.h](cpp_orig/bochs/memory/memory-bochs.h#L100-L110)

```cpp
struct memory_handler_struct {
  struct memory_handler_struct *next;        // Linked list for handler chaining
  void *param;                                // Parameter passed to handlers
  bx_phy_address begin;                       // Start address of handler range
  bx_phy_address end;                         // End address of handler range
  Bit16u bitmap;                              // Bitmap for 16 sub-pages in 1MB region
  bool overlap;                               // Flag for overlapping read-only handlers
  memory_handler_t read_handler;              // Read handler callback
  memory_handler_t write_handler;             // Write handler callback
  memory_direct_access_handler_t da_handler;  // Direct access handler
};
```

### Handler Callbacks

**File**: [cpp_orig/bochs/memory/memory-bochs.h](cpp_orig/bochs/memory/memory-bochs.h#L94-L98)

```cpp
typedef bool (*memory_handler_t)(bx_phy_address addr, unsigned len, void *data, void *param);
// return a pointer to 4K region containing <addr> or NULL if direct access is not allowed
// same format as getHostMemAddr method
typedef Bit8u* (*memory_direct_access_handler_t)(bx_phy_address addr, unsigned rw, void *param);
```

---

## 3. Memory Handler Registration

### Handler Array Initialization

**File**: [cpp_orig/bochs/memory/misc_mem.cc](cpp_orig/bochs/memory/misc_mem.cc#L51-L78)

In `BX_MEM_C::init_memory()`, the handler array is initialized:

```cpp
void BX_MEM_C::init_memory(Bit64u guest, Bit64u host, Bit32u block_size)
{
  unsigned idx, i;

  BX_MEMORY_STUB_C::init_memory(guest, host, block_size);

  // ... other initialization ...

  // Initialize memory handler array with BX_MEM_HANDLERS entries
  // Each entry is a pointer to a linked list of handlers
  BX_MEM_THIS memory_handlers = new struct memory_handler_struct *[BX_MEM_HANDLERS];
  for (idx = 0; idx < BX_MEM_HANDLERS; idx++)
    BX_MEM_THIS memory_handlers[idx] = NULL;

  BX_MEM_THIS pci_enabled = SIM->get_param_bool(BXPN_PCI_ENABLED)->get();
  BX_MEM_THIS bios_write_enabled = false;
  BX_MEM_THIS bios_rom_addr = 0xffff0000;  // Default BIOS ROM start address
  BX_MEM_THIS flash_type = 0;
  BX_MEM_THIS flash_status = 0x80;
  // ... more initialization ...

  for (i = 0; i <= BX_MEM_AREA_F0000; i++) {
    BX_MEM_THIS memory_type[i][0] = false;  // ROM vs ShadowRAM read control
    BX_MEM_THIS memory_type[i][1] = false;  // ROM vs ShadowRAM write control
  }
}
```

**Key Points:**
- Handler array has `BX_MEM_HANDLERS` entries (typically 4096 for 1MB regions indexed by page_idx)
- Each array element is a pointer to a linked list of handlers
- Initial BIOS ROM address is set to `0xffff0000`
- `memory_type[area][0]`: Read control (0=ROM, 1=ShadowRAM)
- `memory_type[area][1]`: Write control (0=ROM, 1=ShadowRAM)

### Handler Registration Function

**File**: [cpp_orig/bochs/memory/misc_mem.cc](cpp_orig/bochs/memory/misc_mem.cc#L792-L838)

```cpp
bool
BX_MEM_C::registerMemoryHandlers(void *param, memory_handler_t read_handler,
                memory_handler_t write_handler, memory_direct_access_handler_t da_handler,
                bx_phy_address begin_addr, bx_phy_address end_addr)
{
  if (end_addr < begin_addr)
    return false;
  if (!read_handler) // allow NULL write and fetch handler
    return false;
  bool ro_handler = (!write_handler && !da_handler);
  BX_INFO(("Register memory access handlers: 0x" FMT_PHY_ADDRX " - 0x" FMT_PHY_ADDRX, begin_addr, end_addr));
  
  // Process each 1MB page that the handler address range spans
  for (Bit32u page_idx = (Bit32u)(begin_addr >> 20); page_idx <= (Bit32u)(end_addr >> 20); page_idx++) {
    Bit16u bitmap = 0xffff;  // Bitmap for 16 sub-pages (64KB each) within 1MB page
    bool overlap = false;
    
    // Calculate bitmap for partial pages
    if (begin_addr > (page_idx << 20)) {
      bitmap &= (0xffff << ((begin_addr >> 16) & 0xf));
    }
    if (end_addr < ((page_idx + 1) << 20)) {
      bitmap &= (0xffff >> (0x0f - ((end_addr >> 16) & 0xf)));
    }
    
    // Check for overlapping handlers
    if (BX_MEM_THIS memory_handlers[page_idx] != NULL) {
      if (((bitmap & BX_MEM_THIS memory_handlers[page_idx]->bitmap) == bitmap) && ro_handler) {
        BX_INFO(("Registering overlapping r/o memory handler"));
        overlap = true;
      } else if ((bitmap & BX_MEM_THIS memory_handlers[page_idx]->bitmap) != 0) {
        BX_ERROR(("Register failed: overlapping memory handlers!"));
        return false;
      } else {
        bitmap |= BX_MEM_THIS memory_handlers[page_idx]->bitmap;
      }
    }
    
    // Create new handler structure and link it to the front of the chain
    struct memory_handler_struct *memory_handler = new struct memory_handler_struct;
    memory_handler->next = BX_MEM_THIS memory_handlers[page_idx];
    BX_MEM_THIS memory_handlers[page_idx] = memory_handler;
    memory_handler->read_handler = read_handler;
    memory_handler->write_handler = write_handler;
    memory_handler->da_handler = da_handler;
    memory_handler->param = param;
    memory_handler->begin = begin_addr;
    memory_handler->end = end_addr;
    memory_handler->bitmap = bitmap;
    memory_handler->overlap = overlap;
    
    // Trigger PCI memory mapping change callback for 0xc0000-0xe0000 range
    if ((begin_addr >= 0xc0000) && (end_addr < 0xe0000)) {
      bx_pc_system.MemoryMappingChanged();
    }
  }
  return true;
}
```

---

## 4. ROM Loading and Initialization

### Load ROM Function

**File**: [cpp_orig/bochs/main.cc](cpp_orig/bochs/main.cc#L1314-L1326)

In main initialization:

```cpp
  // First load the system BIOS (VGABIOS loading moved to the vga code)
  BX_MEM(0)->load_ROM(SIM->get_param_string(BXPN_ROM_PATH)->getptr(),
                      SIM->get_param_num(BXPN_ROM_ADDRESS)->get(), 0);

  // Then load the optional ROM images
  for (i=0; i<BX_N_OPTROM_IMAGES; i++) {
    sprintf(pname, "%s.%d", BXPN_OPTROM_BASE, i+1);
    base = (bx_list_c*) SIM->get_param(pname);
    if (!SIM->get_param_string("file", base)->isempty())
      BX_MEM(0)->load_ROM(SIM->get_param_string("file", base)->getptr(),
                          SIM->get_param_num("address", base)->get(), 2);
  }
```

### ROM Loading Implementation

**File**: [cpp_orig/bochs/memory/misc_mem.cc](cpp_orig/bochs/memory/misc_mem.cc#L290-L370)

The `load_ROM()` function handles three ROM types:

```cpp
//
// Values for type:
//   0 : System Bios
//   1 : VGA Bios
//   2 : Optional ROM Bios
//
void BX_MEM_C::load_ROM(const char *path, bx_phy_address romaddress, Bit8u type)
{
  // ... error checking ...
  
  size = (unsigned long)stat_buf.st_size;

  if (type > 0) {
    max_size = 0x20000;      // 128K max for VGA/optional ROMs
  } else {
    max_size = BIOSROMSZ;    // 4M for system BIOS
  }
  
  if (type == 0) {
    // System BIOS processing
    if (romaddress > 0) {
      // Verify it ends at 0xfffff
      if ((romaddress + size) != 0x100000 && (romaddress + size)) {
        BX_PANIC(("ROM: System BIOS must end at 0xfffff"));
        return;
      }
    } else {
      // If no address specified, calculate to end at 0xfffff
      romaddress = ~(size - 1);
    }
    offset = romaddress & BIOS_MASK;
    
    // Check for expansion ROM
    if ((romaddress & 0xf0000) < 0xf0000) {
      BX_MEM_THIS rom_present[64] = true;
    }
    BX_MEM_THIS bios_rom_addr = (Bit32u)romaddress;
    
    // Detect and store flash type for BIOS
    if (size == 0x40000) {
      BX_MEM_THIS flash_type = 2; // 28F002BC-T (256K flash)
    } else if (size == 0x20000) {
      BX_MEM_THIS flash_type = 1; // 28F001BX-T (128K flash)
    }
  } else {
    // VGA/Optional ROM processing (0xc0000-0xdffff range)
    if ((size % 512) != 0) {
      BX_PANIC(("ROM: ROM image size must be multiple of 512"));
      return;
    }
    if ((romaddress % 2048) != 0) {
      BX_PANIC(("ROM: ROM image must start at a 2k boundary"));
      return;
    }
    
    // Validate address range: 0xc0000-0xdffff or 0xe0000+
    if ((romaddress < 0xc0000) ||
        (((romaddress + size - 1) > 0xdffff) && (romaddress < 0xe0000))) {
      BX_PANIC(("ROM: ROM address space out of range"));
      return;
    }
    
    // Calculate ROM array offset and area indices
    if (romaddress < 0xe0000) {
      offset = (romaddress & EXROM_MASK) + BIOSROMSZ;
      start_idx = (((Bit32u)romaddress - 0xc0000) >> 11);  // 2KB per unit
      end_idx = start_idx + (size >> 11) + (((size % 2048) > 0) ? 1 : 0);
    } else {
      offset = romaddress & BIOS_MASK;
      start_idx = 64;
      end_idx = 64;
    }
    
    // Check for ROM address conflicts
    for (i = start_idx; i < end_idx; i++) {
      if (BX_MEM_THIS rom_present[i]) {
        BX_PANIC(("ROM: address space 0x%x already in use", (i * 2048) + 0xc0000));
        return;
      } else {
        BX_MEM_THIS rom_present[i] = true;
      }
    }
  }
  
  // Read ROM file into memory
  while (size > 0) {
    ret = read(fd, (bx_ptr_t) &BX_MEM_THIS rom[offset], size);
    if (ret <= 0) {
      BX_PANIC(("ROM: read failed on BIOS image: '%s'",path));
    }
    size -= ret;
    offset += ret;
  }
  close(fd);
  
  // ... checksum verification ...
  
  BX_INFO(("rom at 0x%05x/%u ('%s')",
                        (unsigned) romaddress,
                        (unsigned) stat_buf.st_size,
                         path));
}
```

**Key Points:**
- System BIOS (type=0) must end at 0xfffff
- VGA/Optional ROMs (type=1,2) must be in 0xc0000-0xdffff or 0xe0000+ ranges
- Optional ROMs must be 2KB-aligned and 512-byte multiples
- ROM files are loaded into a single large ROM buffer with offset calculations
- `rom_present` array tracks which 2KB blocks are in use

---

## 5. Memory Handler Chain - ROM Read Access

### Read Physical Page with Handler Chain

**File**: [cpp_orig/bochs/memory/memory.cc](cpp_orig/bochs/memory/memory.cc#L181-290)

The handler chain is traversed in `readPhysicalPage()`:

```cpp
void BX_MEM_C::readPhysicalPage(BX_CPU_C *cpu, bx_phy_address addr, unsigned len, void *data)
{
  Bit8u *data_ptr;
  bx_phy_address a20addr = A20ADDR(addr);
  struct memory_handler_struct *memory_handler = NULL;

  // ... initial checks ...

  bool is_bios = (a20addr >= (bx_phy_address)BX_MEM_THIS bios_rom_addr);

  // ... device and SMRAM handling ...

  // STEP 1: Check registered memory handlers for this 1MB page
  memory_handler = BX_MEM_THIS memory_handlers[a20addr >> 20];
  while (memory_handler) {
    if (memory_handler->begin <= a20addr &&
          memory_handler->end >= a20addr &&
          memory_handler->read_handler(a20addr, len, data, memory_handler->param))
    {
      // Handler processed the read
      // Check if we should still read from ROM based on PCI memory type
#if BX_SUPPORT_PCI
      if (BX_MEM_THIS pci_enabled && ((a20addr & 0xfffc0000) == 0x000c0000)) {
        unsigned area = (unsigned)(a20addr >> 14) & 0x0f;
        if (area > BX_MEM_AREA_F0000) area = BX_MEM_AREA_F0000;
        if (BX_MEM_THIS memory_type[area][0] == 0) { // Read from ROM
          return;
        }
      } else
#endif
      {
        return;
      }
    }
    memory_handler = memory_handler->next;
  }

mem_read:

  // STEP 2: Default ROM reading for 0xc0000-0xfffff range
  if ((a20addr < BX_MEM_THIS len) && !is_bios) {
    if (a20addr < 0x000a0000 || a20addr >= 0x00100000) {
      BX_MEMORY_STUB_C::readPhysicalPage(cpu, addr, len, data);
      return;
    }

#ifdef BX_LITTLE_ENDIAN
    data_ptr = (Bit8u *) data;
#else
    data_ptr = (Bit8u *) data + (len - 1);
#endif

    // addr must be in range 000A0000 .. 000FFFFF
    for (unsigned i=0; i<len; i++) {

      // SMMRAM (0xa0000-0xbffff)
      if (a20addr < 0x000c0000) {
        if (cpu) *data_ptr = *(BX_MEM_THIS get_vector(a20addr));
        goto inc_one;
      }

#if BX_SUPPORT_PCI
      if (BX_MEM_THIS pci_enabled && ((a20addr & 0xfffc0000) == 0x000c0000)) {
        unsigned area = (unsigned)(a20addr >> 14) & 0x0f;
        if (area > BX_MEM_AREA_F0000) area = BX_MEM_AREA_F0000;
        
        if (BX_MEM_THIS memory_type[area][0] == 0) {
          // Read from ROM
          if ((a20addr & 0xfffe0000) == 0x000e0000) {
            // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
            if (BX_MEM_THIS flash_type > 0) {
              *data_ptr = BX_MEM_THIS flash_read(BIOS_MAP_LAST128K(a20addr));
            } else {
              *data_ptr = BX_MEM_THIS rom[BIOS_MAP_LAST128K(a20addr)];
            }
          } else {
            // Expansion ROM at 0xc0000-0xdffff
            *data_ptr = BX_MEM_THIS rom[(a20addr & EXROM_MASK) + BIOSROMSZ];
          }
        } else {
          // Read from ShadowRAM
          *data_ptr = *(BX_MEM_THIS get_vector(a20addr));
        }
      }
      else
#endif  // #if BX_SUPPORT_PCI
      {
        // No PCI support: direct ROM read
        if ((a20addr & 0xfffc0000) != 0x000c0000) {
          *data_ptr = *(BX_MEM_THIS get_vector(a20addr));
        }
        else if ((a20addr & 0xfffe0000) == 0x000e0000) {
          // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
          *data_ptr = BX_MEM_THIS rom[BIOS_MAP_LAST128K(a20addr)];
        }
        else {
          // Expansion ROM
          *data_ptr = BX_MEM_THIS rom[(a20addr & EXROM_MASK) + BIOSROMSZ];
        }
      }

inc_one:
      a20addr++;
      // ... endian handling ...
    }
  } else if (BX_MEM_THIS bios_write_enabled && is_bios) {
    // Read from BIOS ROM area (above 0xfff00000)
    // ... BIOS ROM read handling ...
  }
}
```

### Direct Host Memory Address (Fast Path)

**File**: [cpp_orig/bochs/memory/misc_mem.cc](cpp_orig/bochs/memory/misc_mem.cc#L698-750)

For fast direct memory access:

```cpp
Bit8u * BX_MEM_C::getHostMemAddr(BX_CPU_C *cpu, bx_phy_address a20addr, unsigned rw)
{
  // ... validation ...

#if BX_SUPPORT_MONITOR_MWAIT
  if (write && BX_MEM_THIS is_monitor(a20addr & ~((bx_phy_address)(0xfff)), 0xfff)) {
    // Vetoed! Write monitored page !
    return(NULL);
  }
#endif

  // Check registered memory handlers that provide direct access
  struct memory_handler_struct *memory_handler = BX_MEM_THIS memory_handlers[a20addr >> 20];
  while (memory_handler) {
    if (memory_handler->begin <= a20addr &&
        memory_handler->end >= a20addr) {
      if (memory_handler->da_handler)
        return memory_handler->da_handler(a20addr, rw, memory_handler->param);
      else
        return(NULL); // Vetoed! memory handler for i/o apic, vram, mmio and PCI PnP
    }
    memory_handler = memory_handler->next;
  }

  // Default handling for ROM areas
  if (! write) {
    if ((a20addr >= 0x000a0000 && a20addr < 0x000c0000))
      return(NULL); // Vetoed!  Mem mapped IO (VGA)
#if BX_SUPPORT_PCI
    else if (BX_MEM_THIS pci_enabled && (a20addr >= 0x000c0000 && a20addr < 0x00100000)) {
      unsigned area = (unsigned)(a20addr >> 14) & 0x0f;
      if (area > BX_MEM_AREA_F0000) area = BX_MEM_AREA_F0000;
      if (BX_MEM_THIS memory_type[area][0] == false) {
        // Read from ROM - cannot provide direct access to ROM
        if ((a20addr & 0xfffe0000) == 0x000e0000) {
          // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
          return (Bit8u *) &BX_MEM_THIS rom[BIOS_MAP_LAST128K(a20addr)];
        } else {
          return (Bit8u *) &BX_MEM_THIS rom[(a20addr & EXROM_MASK) + BIOSROMSZ];
        }
      } else {
        // Read from ShadowRAM
        return BX_MEM_THIS get_vector(a20addr);
      }
    }
#endif
    else if ((a20addr < BX_MEM_THIS len) && !is_bios)
    {
      if (a20addr < 0x000c0000 || a20addr >= 0x00100000) {
        return BX_MEM_THIS get_vector(a20addr);
      }
      // must be in C0000 - FFFFF range
      else if ((a20addr & 0xfffe0000) == 0x000e0000) {
        // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
        return (Bit8u *) &BX_MEM_THIS rom[BIOS_MAP_LAST128K(a20addr)];
      }
      else {
        return((Bit8u *) &BX_MEM_THIS rom[(a20addr & EXROM_MASK) + BIOSROMSZ]);
      }
    }
  }
  // ... more handling ...
}
```

---

## 6. ROM Address Mapping Summary

### How ROM Buffer Addressing Works

The ROM buffer contains two sections:

1. **System BIOS** (0xffc00000-0xfff00000 in ROM address space)
   - Mapped to 0xe0000-0xfffff in CPU address space (last 128KB)
   - Accessed via macro: `BIOS_MAP_LAST128K(addr) = ((addr) | 0xfff00000) & BIOS_MASK`

2. **Expansion ROMs** (0x000000-0x020000 in ROM address space)  
   - Mapped to 0xc0000-0xdffff in CPU address space (128KB)
   - Accessed via: `(addr & EXROM_MASK) + BIOSROMSZ`

### Example Address Mapping

For a read from 0xf0000 (upper BIOS area):
1. Address matches `(a20addr & 0xfffe0000) == 0x000e0000` 
2. Use `BIOS_MAP_LAST128K(0xf0000) = (0xf0000 | 0xfff00000) & 0x3fffff = 0x3f0000`
3. Read from `rom[0x3f0000]`

For a read from 0xc0000 (expansion ROM area):
1. Address doesn't match BIOS mapping condition
2. Use `(0xc0000 & EXROM_MASK) + BIOSROMSZ = 0 + 0x400000 = 0x400000`
3. Read from `rom[0x400000]`

---

## 7. BIOS Device Integration

**File**: [cpp_orig/bochs/iodev/biosdev.cc](cpp_orig/bochs/iodev/biosdev.cc#L78-L88)

The BIOS device registers I/O port handlers for debugging messages:

```cpp
void bx_biosdev_c::init(void)
{
  DEV_register_iowrite_handler(this, write_handler, 0x0400, "Bios Panic Port 1", 3);
  DEV_register_iowrite_handler(this, write_handler, 0x0401, "Bios Panic Port 2", 3);
  DEV_register_iowrite_handler(this, write_handler, 0x0402, "Bios Info Port", 1);
  DEV_register_iowrite_handler(this, write_handler, 0x0403, "Bios Debug Port", 1);

  DEV_register_iowrite_handler(this, write_handler, 0x0500, "VGABios Info Port", 1);
  DEV_register_iowrite_handler(this, write_handler, 0x0501, "VGABios Panic Port 1", 3);
  DEV_register_iowrite_handler(this, write_handler, 0x0502, "VGABios Panic Port 2", 3);
  DEV_register_iowrite_handler(this, write_handler, 0x0503, "VGABios Debug Port", 1);
}
```

These are I/O port handlers (separate from memory handlers) used to capture BIOS debug messages.

---

## Key Architecture Points

1. **Two-Level Indexing**: Memory handlers use 1MB page indexing (addr >> 20) with linked lists for overlapping handlers
2. **Bitmap Granularity**: Each 1MB page can have handlers for 16 sub-pages (64KB each)
3. **Handler Chain**: Handlers are checked in order, allowing overlapping read-only handlers
4. **ROM Buffer**: Single 4MB ROM buffer stores both system BIOS and expansion ROMs with calculated offsets
5. **ShadowRAM Support**: PCI memory_type arrays allow enabling/disabling ROM mapping for write-back RAM
6. **Flash Support**: BIOS flash memory emulation via flash_type and flash_read/write callbacks
7. **Address Mapping**: BIOS_MAP_LAST128K macro handles address translation for 0xe0000-0xfffff range
