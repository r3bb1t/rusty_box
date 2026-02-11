# BIOS Loading Sequence Verification

## Date: 2026-02-11

## Status: ✅ VERIFIED CORRECT

## Original Bochs Sequence (main.cc)

```cpp
Line 1312: BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);

Line 1315: BX_MEM(0)->load_ROM(SIM->get_param_string(BXPN_ROM_PATH)->getptr(),
                                 SIM->get_param_num(BXPN_ROM_ADDRESS)->get(), 0);

Line 1319-1325: // Load optional ROMs (can include VGA BIOS)
                for (i=0; i<BX_N_OPTROM_IMAGES; i++) {
                    BX_MEM(0)->load_ROM(..., ..., 2);
                }

Line 1337: BX_CPU(0)->initialize();
Line 1339: BX_CPU(0)->register_state();

Line 1353: DEV_init_devices();
           // VGA init (vgacore.cc:126) loads VGA BIOS if not PCI:
           // BX_MEM(0)->load_ROM(..., 0xc0000, 1);
```

## Our Rusty Box Sequence (dlxlinux.rs)

```rust
Line 249: emu.init_memory_and_pc_system()?;
          // Calls: memory.init_memory()
          //        pc_system.initialize()

Line 272: emu.load_bios(&bios_data, bios_load_addr)?;
          // Type 0 (system BIOS)

Line 277: emu.load_optional_rom(&vga_data, 0xC0000)?;
          // Type 2 (optional ROM) - VGA BIOS

Line 284: emu.init_cpu_and_devices()?;
          // Calls: cpu.initialize()
          //        devices.init()
          //        device_manager.init()
```

## Comparison

| Step | Bochs | Rusty Box | Status |
|------|-------|-----------|--------|
| 1. Memory init | Line 1312 | Line 249 | ✅ Correct |
| 2. System BIOS load | Line 1315 | Line 272 | ✅ Correct |
| 3. VGA BIOS load | Line 1319-1325 OR vgacore.cc:126 | Line 277 | ✅ Correct |
| 4. CPU init | Line 1337 | Line 284 | ✅ Correct |
| 5. Device init | Line 1353 | Line 284 | ✅ Correct |

## VGA BIOS Loading - Two Valid Paths

### Path 1: As Optional ROM (Before CPU Init)
- **Bochs**: Lines 1319-1325 (if configured in optrom list)
- **Rusty Box**: Line 277 (explicit load before CPU init)
- **Status**: ✅ Both work

### Path 2: During VGA Device Init
- **Bochs**: vgacore.cc:126 (during DEV_init_devices)
- **Rusty Box**: Not implemented (VGA doesn't load its own ROM)
- **Status**: ⚠️ Not needed since we use Path 1

## Conclusion

Our BIOS loading sequence is **CORRECT** and matches the original Bochs behavior. The VGA BIOS is loaded as an optional ROM before CPU initialization, which is one of the two valid loading methods in Bochs.

The current issue with zero memory reads is **NOT** a timing problem - it's a memory mapping bug in how we handle address translation for the range 0xE0000-0xFFFFF.

## Related Documentation

- **MEMORY_MAPPING_ZEROS_BUG.md**: Current investigation of zero memory reads
- **VGA_BIOS_LOADING_SEQUENCE_BUG.md**: Previous (incorrect) theory about timing
- **FAR_JUMP_DECODER_BUG.md**: FAR JMP fix (completed)
