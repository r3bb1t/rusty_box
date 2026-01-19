# Initialization Order Comparison: Bochs vs Our Implementation

## Bochs Initialization Order (from main.cc)

1. `bx_init_hardware()` (line 1192-1401):
   - `bx_pc_system.initialize(IPS)` - Initialize PC system
   - `BX_MEM(0)->init_memory()` - Initialize memory
   - `BX_MEM(0)->load_ROM()` - Load BIOS
   - `BX_CPU(0)->initialize()` - Initialize CPU
   - `DEV_init_devices()` - Initialize devices (VGA registers handlers here)
   - `bx_pc_system.Reset(BX_RESET_HARDWARE)` - Hardware reset (A20 enabled, CPU reset, devices reset)
   - `bx_gui->init_signal_handlers()` - GUI signal handlers
   - `bx_pc_system.start_timers()` - **Start timers** (line 1384)

2. Main loop (line 1082-1089):
   ```cpp
   while (1) {
     BX_CPU(0)->cpu_loop();  // Runs forever until kill_bochs_request
     if (bx_pc_system.kill_bochs_request)
       break;
   }
   ```

## Our Initialization Order

1. `emu.initialize()`:
   - `pc_system.initialize(IPS)` - Initialize PC system
   - `memory.init_memory()` - Initialize memory (done in new())
   - `cpu.initialize()` - Initialize CPU
   - `devices.init()` - Initialize I/O handlers
   - `device_manager.init()` - Initialize devices (VGA registers handlers here)

2. `emu.load_bios()` - Load BIOS

3. `emu.load_optional_rom()` - Load VGA BIOS

4. `emu.set_gui()` + `emu.init_gui()` - GUI setup

5. `emu.reset(ResetReason::Hardware)` - Hardware reset (A20 enabled, CPU reset, devices reset)

6. `emu.run_interactive()`:
   - `prepare_run()` → `start()` → `pc_system.start_timers()` - **Start timers** (called here)
   - Loop: `cpu_loop_n()` in batches with max instruction limit

## Key Differences

1. **Timer Start Timing**: 
   - Bochs: `start_timers()` called in `bx_init_hardware()` AFTER reset, BEFORE main loop
   - Ours: `start_timers()` called in `prepare_run()` which is INSIDE `run_interactive()`

2. **CPU Loop**:
   - Bochs: `cpu_loop()` runs forever in infinite loop
   - Ours: `cpu_loop_n()` runs in batches with max instruction limit

3. **GUI Setup**:
   - Bochs: GUI initialized before `bx_init_hardware()` (via `bx_init_main()` → `bx_gui->init()`)
   - Ours: GUI setup happens after `initialize()` but before `reset()`

## Potential Issues

1. **Timer start timing**: ✅ FIXED - Moved `start_timers()` to `reset()` to match Bochs order
2. **VGA timer**: VGA device has its own timer that needs to be started - this might not be running
3. **CPU loop**: Using `cpu_loop_n()` in batches vs continuous `cpu_loop()` - might miss timing/events
4. **Call chain**: Need to verify all initialization calls match Bochs exactly
