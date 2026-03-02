//! Emulator Container
//!
//! This module provides the `Emulator` struct that owns and coordinates all
//! emulator components: CPU, Memory, Devices, and PC System.
//!
//! Each `Emulator` instance is fully independent with no global state,
//! allowing hundreds of emulator instances to run concurrently on different threads.

use crate::{
    cpu::{builder::BxCpuBuilder, BxCpuC, BxCpuIdTrait, ResetReason},
    gui::BxGui,
    iodev::{
        devices::{DeviceManager, SystemControlPort},
        BxDevicesC,
    },
    memory::{BxMemC, BxMemoryStubC},
    params::BxParams,
    pc_system::BxPcSystemC,
    Result,
};

use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

/// Emulator configuration
#[derive(Debug, Clone)]
pub struct EmulatorConfig {
    /// Guest memory size in bytes
    pub guest_memory_size: usize,
    /// Host memory size in bytes (can be less than guest for swapping)
    pub host_memory_size: usize,
    /// Memory block size for allocation
    pub memory_block_size: usize,
    /// Instructions per second for timing
    pub ips: u32,
    /// Enable PCI support
    pub pci_enabled: bool,
    /// CPU parameters
    pub cpu_params: BxParams,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            guest_memory_size: 32 * 1024 * 1024, // 32 MB
            host_memory_size: 32 * 1024 * 1024,  // 32 MB
            memory_block_size: 128 * 1024,       // 128 KB blocks
            ips: 4_000_000,                      // 4 MIPS
            pci_enabled: true,                   // Enable PCI for shadow RAM support
            cpu_params: BxParams::default(),
        }
    }
}

/// Emulator instance containing all hardware components
///
/// This struct owns the CPU, Memory, Devices, and PC System, providing
/// a fully self-contained emulator instance with no global state.
///
/// # Thread Safety
///
/// Each `Emulator` instance is `Send` and can be moved to a different thread.
/// Multiple instances can run concurrently without any shared state.
///
/// # Example
///
/// ```ignore
/// use rusty_box::emulator::{Emulator, EmulatorConfig};
/// use rusty_box::cpu::core_i7_skylake::Corei7SkylakeX;
///
/// let config = EmulatorConfig::default();
/// let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;
/// emu.initialize()?;
/// emu.load_bios(&bios_data, 0xfffe0000)?;
/// emu.reset(ResetReason::Hardware)?;
/// // Access components directly for cpu_loop:
/// // emu.cpu.cpu_loop(&mut emu.memory, &[]);
/// ```
pub struct Emulator<'a, I: BxCpuIdTrait> {
    /// CPU instance
    pub cpu: BxCpuC<'a, I>,
    /// Memory subsystem
    pub memory: BxMemC<'a>,
    /// Device controller (I/O port handlers)
    pub devices: BxDevicesC,
    /// Device manager (actual hardware devices)
    pub device_manager: DeviceManager,
    /// PC system (timers, A20, etc.)
    pub pc_system: BxPcSystemC,
    /// System Control Port state (Port 0x92)
    pub system_control: SystemControlPort,
    /// Configuration
    config: EmulatorConfig,
    /// Whether the emulator has been initialized
    initialized: bool,
    /// GUI instance (optional, can be None for headless operation)
    gui: Option<Box<dyn BxGui>>,
    /// BIOS output file for port 0x402/0x403/0xE9 messages (std feature only)
    #[cfg(feature = "std")]
    bios_output_file: Option<std::fs::File>,
    /// Shared stop flag: when set to true by the GUI thread, run_interactive exits the loop
    pub stop_flag: Arc<AtomicBool>,
}

impl<'a, I: BxCpuIdTrait> Emulator<'a, I> {
    /// Create a new emulator instance with the given configuration
    ///
    /// This creates all components but does not initialize them.
    /// Call `initialize()` after creation to complete setup.
    ///
    /// Returns Box<Emulator> to avoid stack overflow (Emulator is ~1.4 MB).
    /// This matches original Bochs behavior which uses `new BX_CPU_C(i)`.
    pub fn new(config: EmulatorConfig) -> Result<Box<Self>> {
        // Create PC system
        let pc_system = BxPcSystemC::new();

        // Create memory (but don't initialize yet - that's done in initialize() to match original)
        // In original Bochs, BX_MEM(0) is created first, then init_memory() is called in bx_init_hardware()
        let mem_stub = BxMemoryStubC::create_and_init(
            config.guest_memory_size,
            config.host_memory_size,
            config.memory_block_size,
        )?;
        let memory = BxMemC::new(mem_stub, config.pci_enabled);

        // Create devices (I/O port handlers)
        let devices = BxDevicesC::new();

        // Create device manager (actual hardware)
        let device_manager = DeviceManager::new();

        // Create CPU
        let builder: BxCpuBuilder<I> = BxCpuBuilder::new();
        let cpu = builder.build()?;

        // Box to allocate on heap (matches Bochs's `new BX_CPU_C(i)`)
        Ok(Box::new(Self {
            cpu,
            memory,
            devices,
            device_manager,
            pc_system,
            system_control: SystemControlPort::new(),
            config,
            initialized: false,
            gui: None,
            #[cfg(feature = "std")]
            bios_output_file: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// Initialize the emulator
    ///
    /// This runs the full initialization sequence from Bochs main.cc:1192-1401 (bx_init_hardware):
    /// 1. PC system initialization (timers, IPS) - line 1201
    /// 2. Memory initialization - line 1312
    /// 3. BIOS load - line 1315-1316 (done via load_bios() after this call)
    /// 4. Optional ROM load - line 1319-1325 (done via load_optional_rom())
    /// 5. Optional RAM load - line 1328-1334 (done via load_ram())
    /// 6. CPU initialization - line 1337
    /// 7. CPU sanity checks - line 1338
    /// 8. CPU register state - line 1339
    /// 9. Device initialization - line 1353
    /// 10. PC system register state - line 1356
    /// 11. Device register state - line 1357
    /// 12. Reset - line 1363 (done via reset() after this call)
    /// 13. GUI signal handlers - line 1383 (done via init_gui() or after reset)
    /// 14. Start timers - line 1384 (done in reset())
    ///
    /// After this, call `load_bios()` to load a BIOS image, then `reset()` and `run()`.
    ///
    /// **IMPORTANT**: For correct BIOS initialization sequence matching original Bochs,
    /// use `init_memory()` + `load_bios()` + `init_cpu_and_devices()` instead of this method.
    /// See main.cc:1312-1353 for the correct sequence.
    pub fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            tracing::debug!("Emulator already initialized");
            return Ok(());
        }

        tracing::info!("Initializing emulator");

        // Step 1: Initialize PC system with IPS (line 1201)
        self.pc_system.initialize(self.config.ips);
        tracing::debug!("PC system initialized with {} IPS", self.config.ips);

        // Step 2: Memory initialization (line 1312)
        // In original: BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);
        self.memory.init_memory(
            self.config.guest_memory_size,
            self.config.host_memory_size,
            self.config.memory_block_size,
        )?;

        // Sync A20 mask from PC system (after memory init, matching original)
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        tracing::debug!("Memory initialized and A20 mask synced");

        // Step 3-5: BIOS/ROM/RAM loading should happen HERE (after memory init, before CPU init)
        // But since this method doesn't have BIOS data, it's loaded separately after this call.
        // For correct initialization, use init_memory() + load_bios() + init_cpu_and_devices()

        // Step 6: Initialize CPU (line 1337)
        self.cpu.initialize(self.config.cpu_params.clone())?;
        tracing::debug!("CPU initialized");

        // Step 7: CPU sanity checks (line 1338) - separate call to match original
        self.cpu.sanity_checks()?;
        tracing::debug!("CPU sanity checks passed");

        // Step 8: Register CPU state (line 1339)
        self.cpu.register_state();
        tracing::debug!("CPU state registered");

        // Note: BX_INSTR_INITIALIZE(0) at line 1340 is instrumentation initialization
        // This is optional and not yet implemented in Rust version

        // Step 9: Initialize devices (line 1353)
        // Pass pointer to system_control for Port 92h handling
        let port92_ptr = &mut self.system_control as *mut SystemControlPort;
        self.devices.init(&mut self.memory, Some(port92_ptr))?;

        // Initialize device manager (actual hardware + I/O handler registration)
        self.device_manager
            .init(&mut self.devices, &mut self.memory)?;
        tracing::debug!("Devices initialized");

        // Wire PIC→CPU interrupt signaling (Bochs BX_RAISE_INTR / BX_CLEAR_INTR).
        // The PIC needs raw pointers to cpu.async_event and cpu.pending_event so it
        // can break the CPU inner loop when master int_pin asserts/deasserts.
        // Also give the CPU a raw pointer to the PIC for DEV_pic_iac() in
        // handle_async_event()'s external interrupt delivery.
        unsafe {
            let async_ptr = &mut self.cpu.async_event as *mut u32;
            let pending_ptr = &mut self.cpu.pending_event as *mut u32;
            self.device_manager
                .pic
                .set_cpu_signal_ptrs(async_ptr, pending_ptr);
        }
        self.cpu.pic_ptr = &mut self.device_manager.pic as *mut crate::iodev::pic::BxPicC;

        // Wire HardDrive→PIC for immediate IRQ raise/lower (matches Bochs DEV_pic_raise_irq)
        self.device_manager.harddrv.pic_ptr =
            &mut self.device_manager.pic as *mut crate::iodev::pic::BxPicC;

        // Note: SIM->opt_plugin_ctrl("*", 0) at line 1355 unloads unused optional plugins
        // This is optional plugin management, not yet implemented in Rust version

        // Step 10: PC system register state (line 1356)
        self.pc_system.register_state();

        // Step 11: Device register state (line 1357)
        self.devices.register_state()?;
        tracing::debug!("State registered");

        // Note: bx_set_log_actions_by_device(1) at line 1359 sets up logging per device
        // This is only called if not restoring state, and is optional logging setup

        self.initialized = true;
        tracing::info!("Emulator initialization complete");

        // Note: Steps 12-14 (Reset, GUI signal handlers, Start timers) are done via:
        // - reset() method (called after BIOS loading)
        // - init_gui() method (calls init_signal_handlers)
        // - reset() also calls start_timers()

        Ok(())
    }

    /// Initialize memory and PC system (Step 1-2 of initialization)
    ///
    /// This is the first part of the initialization sequence from Bochs main.cc:
    /// 1. PC system initialization (timers, IPS) - line 1201
    /// 2. Memory initialization - line 1312
    ///
    /// After this, call `load_bios()` and `load_optional_rom()`, then `init_cpu_and_devices()`.
    /// This matches the original Bochs sequence: Memory init → Load BIOS → CPU init → Device init.
    pub fn init_memory_and_pc_system(&mut self) -> Result<()> {
        if self.initialized {
            tracing::debug!("Emulator already initialized");
            return Ok(());
        }

        tracing::info!("Initializing hardware...");

        // Step 1: Initialize PC system with IPS (line 1201)
        self.pc_system.initialize(self.config.ips);
        tracing::debug!("PC system initialized with {} IPS", self.config.ips);

        // Step 2: Memory initialization (line 1312)
        // In original: BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);
        self.memory.init_memory(
            self.config.guest_memory_size,
            self.config.host_memory_size,
            self.config.memory_block_size,
        )?;

        // Sync A20 mask from PC system (after memory init, matching original)
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        tracing::debug!("Memory initialized and A20 mask synced");

        Ok(())
    }

    /// Initialize CPU and devices (Step 6-11 of initialization)
    ///
    /// This is the second part of the initialization sequence from Bochs main.cc:
    /// 6. CPU initialization - line 1337
    /// 7. CPU sanity checks - line 1338
    /// 8. CPU register state - line 1339
    /// 9. Device initialization - line 1353
    /// 10. PC system register state - line 1356
    /// 11. Device register state - line 1357
    ///
    /// Call this AFTER `init_memory_and_pc_system()` and `load_bios()`.
    pub fn init_cpu_and_devices(&mut self) -> Result<()> {
        // Step 6: Initialize CPU (line 1337)
        self.cpu.initialize(self.config.cpu_params.clone())?;
        tracing::debug!("CPU initialized");

        // Step 7: CPU sanity checks (line 1338) - separate call to match original
        self.cpu.sanity_checks()?;
        tracing::debug!("CPU sanity checks passed");

        // Step 8: Register CPU state (line 1339)
        self.cpu.register_state();
        tracing::debug!("CPU state registered");

        // Note: BX_INSTR_INITIALIZE(0) at line 1340 is instrumentation initialization
        // This is optional and not yet implemented in Rust version

        // Step 9: Initialize devices (line 1353)
        // Pass pointer to system_control for Port 92h handling
        let port92_ptr = &mut self.system_control as *mut SystemControlPort;
        self.devices.init(&mut self.memory, Some(port92_ptr))?;

        // Initialize device manager (actual hardware + I/O handler registration)
        self.device_manager
            .init(&mut self.devices, &mut self.memory)?;
        tracing::info!("Device initialization complete");

        // Wire PIC→CPU interrupt signaling (same as in initialize())
        unsafe {
            let async_ptr = &mut self.cpu.async_event as *mut u32;
            let pending_ptr = &mut self.cpu.pending_event as *mut u32;
            self.device_manager
                .pic
                .set_cpu_signal_ptrs(async_ptr, pending_ptr);
        }
        self.cpu.pic_ptr = &mut self.device_manager.pic as *mut crate::iodev::pic::BxPicC;

        // Wire HardDrive→PIC for immediate IRQ raise/lower (matches Bochs DEV_pic_raise_irq)
        self.device_manager.harddrv.pic_ptr =
            &mut self.device_manager.pic as *mut crate::iodev::pic::BxPicC;

        // Note: SIM->opt_plugin_ctrl("*", 0) at line 1355 unloads unused optional plugins
        // This is optional plugin management, not yet implemented in Rust version

        // Step 10: PC system register state (line 1356)
        self.pc_system.register_state();

        // Step 11: Device register state (line 1357)
        self.devices.register_state()?;
        tracing::debug!("State registered");

        // Note: bx_set_log_actions_by_device(1) at line 1359 sets up logging per device
        // This is only called if not restoring state, and is optional logging setup

        self.initialized = true;
        tracing::info!("Emulator initialization complete");

        // Note: Steps 12-14 (Reset, GUI signal handlers, Start timers) are done via:
        // - reset() method (called after BIOS loading)
        // - init_gui() method (calls init_signal_handlers)
        // - reset() also calls start_timers()

        Ok(())
    }

    /// Set the GUI instance
    ///
    /// Based on load_and_init_display_lib() in main.cc:964-1006
    pub fn set_gui<G: BxGui + 'static>(&mut self, gui: G) {
        self.gui = Some(Box::new(gui));
        tracing::info!("GUI set");
    }

    /// Initialize the GUI
    ///
    /// Based on bx_init_hardware() GUI initialization in main.cc:1017-1020
    /// This calls specific_init() to set up the GUI, but signal handlers are
    /// initialized separately via init_gui_signal_handlers() after reset.
    pub fn init_gui(&mut self, argc: i32, argv: &[String]) -> Result<()> {
        if let Some(ref mut gui) = self.gui {
            gui.specific_init(argc, argv, 32); // BX_HEADER_BAR_Y = 32
            gui.update_drive_status_buttons();

            // Connect keyboard callback if GUI supports it
            self.connect_keyboard_callback();

            tracing::info!("GUI initialized (signal handlers will be set up after reset)");
        } else {
            tracing::debug!("No GUI set, running headless");
        }
        Ok(())
    }

    /// Connect keyboard callback from GUI to keyboard device
    /// (No-op now - we use queue-based approach instead)
    fn connect_keyboard_callback(&mut self) {
        // Keyboard input is now handled via get_pending_scancodes() in the event loop
    }

    /// Get mutable reference to GUI (if set)
    pub fn gui_mut(&mut self) -> Option<&mut (dyn BxGui + 'static)> {
        self.gui.as_deref_mut()
    }

    /// Get reference to GUI (if set)
    pub fn gui(&self) -> Option<&(dyn BxGui + 'static)> {
        self.gui.as_deref()
    }

    /// Update GUI with VGA text mode changes
    ///
    /// Call this periodically to refresh the display (matching vgacore.cc:2413-2430)
    /// Uses VGA update() function to process text mode and get update data
    pub fn update_gui(&mut self) {
        if let Some(ref mut gui) = self.gui {
            // Call VGA update() to process text mode (matching vgacore.cc:2427)
            if let Some(update_result) = self.device_manager.vga.update() {
                // Calculate cursor position from cursor address
                let cursor_x = if update_result.cursor_address < 0x7fff {
                    // Cursor address is byte offset, convert to column
                    let offset_from_start = update_result
                        .cursor_address
                        .saturating_sub(update_result.tm_info.start_address);
                    (offset_from_start % update_result.tm_info.line_offset) / 2
                } else {
                    0xffff
                };

                let cursor_y = if update_result.cursor_address < 0x7fff {
                    // Cursor address is byte offset, convert to row
                    let offset_from_start = update_result
                        .cursor_address
                        .saturating_sub(update_result.tm_info.start_address);
                    (offset_from_start / update_result.tm_info.line_offset) as u32
                } else {
                    0xffff
                };

                // Notify GUI of dimension changes (matching vgacore.cc:1661)
                if update_result.dimension_changed {
                    gui.dimension_update(
                        update_result.iwidth,
                        update_result.iheight,
                        update_result.fheight,
                        update_result.fwidth,
                        8, // bpp for text mode
                    );
                }

                // Call GUI text_update with old snapshot and new buffer (matching vgacore.cc:1685)
                gui.text_update(
                    &update_result.text_snapshot,
                    &update_result.text_buffer,
                    cursor_x as u32,
                    cursor_y as u32,
                    &update_result.tm_info,
                );
            }

            // Flush display (matching vgacore.cc:2429)
            gui.flush();
        }
    }

    /// Load a BIOS ROM image
    ///
    /// # Arguments
    /// * `bios_data` - Raw BIOS ROM data
    /// * `address` - Load address (typically 0xfffe0000 for 128KB BIOS)
    pub fn load_bios(&mut self, bios_data: &[u8], address: u64) -> Result<()> {
        self.memory.load_ROM(bios_data, address, 0)?;
        tracing::info!("Loaded BIOS ({} bytes) at {:#x}", bios_data.len(), address);
        Ok(())
    }

    /// Load an optional ROM image (VGA BIOS, expansion ROMs, etc.)
    ///
    /// # Arguments
    /// * `rom_data` - Raw ROM data
    /// * `address` - Load address (must be in 0xC0000-0xFFFFF range)
    pub fn load_optional_rom(&mut self, rom_data: &[u8], address: u64) -> Result<()> {
        self.memory.load_ROM(rom_data, address, 2)?;
        tracing::info!(
            "Loaded optional ROM ({} bytes) at {:#x}",
            rom_data.len(),
            address
        );
        Ok(())
    }

    /// Load an optional RAM image
    ///
    /// Based on `BX_MEM(0)->load_RAM()` in Bochs main.cc
    ///
    /// # Arguments
    /// * `ram_data` - Raw RAM image data
    /// * `address` - Load address in physical memory
    pub fn load_ram(&mut self, ram_data: &[u8], address: u64) -> Result<()> {
        self.memory.load_RAM(ram_data, address)?;
        tracing::info!(
            "Loaded RAM image ({} bytes) at {:#x}",
            ram_data.len(),
            address
        );
        Ok(())
    }

    /// Perform a system reset
    ///
    /// This corresponds to `bx_pc_system.Reset()` in Bochs.
    ///
    /// # Arguments
    /// * `reset_type` - Type of reset (Hardware or Software)
    pub fn reset(&mut self, reset_type: ResetReason) -> Result<()> {
        tracing::info!("Emulator reset ({:?})", reset_type);

        // Reset PC system (enables A20)
        self.pc_system.reset(reset_type)?;

        // Sync A20 mask to memory
        self.memory.set_a20_mask(self.pc_system.a20_mask());

        // Reset CPU
        self.cpu.reset(reset_type);

        // Reset devices (only on hardware reset)
        // Matches original: DEV_reset_devices(type) at pc_system.cc:201
        // which calls bx_devices_c::reset() at devices.cc:398-411
        if matches!(reset_type, ResetReason::Hardware) {
            // Original bx_devices_c::reset() does (in order):
            // 1. Clear PCI confAddr if PCI enabled (line 402) - done in devices.reset()
            // 2. mem->disable_smram() (line 405) - disable SMRAM
            // 3. bx_reset_plugins(type) (line 406) - reset all device plugins
            // 4. release_keys() (line 407) - release keyboard keys
            // 5. paste.stop = 1 (line 409) - stop paste buffer

            // Step 1: Clear PCI confAddr (done in devices.reset())
            self.devices.reset(reset_type)?;

            // Step 2: Disable SMRAM (matches original line 405: mem->disable_smram())
            self.memory.disable_smram();

            // Step 3: Reset all device plugins (matches original line 406: bx_reset_plugins())
            // This resets all devices: PIC, PIT, CMOS, DMA, Keyboard, HardDrive, VGA
            self.device_manager.reset(reset_type)?;

            // Note: release_keys() at line 407 and paste.stop at line 409 not yet implemented
        }

        // Reset system control port state
        self.system_control = SystemControlPort::new();

        // Note: start_timers() is called separately after GUI signal handlers
        // to match original Bochs order: reset -> init_signal_handlers -> start_timers

        Ok(())
    }

    /// Initialize GUI signal handlers
    ///
    /// This should be called after reset() and before start_timers() to match
    /// original Bochs sequence (line 1383).
    pub fn init_gui_signal_handlers(&mut self) {
        if let Some(ref mut gui) = self.gui {
            gui.init_signal_handlers();
            tracing::debug!("GUI signal handlers initialized");
        }
    }

    /// Start timers and prepare for execution
    /// Note: Timers are now started in reset(), so this is mostly for compatibility
    pub fn start(&mut self) {
        self.pc_system.start_timers();
        tracing::debug!("Timers started");
    }

    /// Check if the emulator is ready to run
    ///
    /// Call this before accessing `cpu.cpu_loop()`.
    pub fn ready_to_run(&self) -> Result<()> {
        if !self.initialized {
            return Err(crate::Error::Cpu(crate::cpu::CpuError::CpuNotInitialized));
        }
        Ok(())
    }

    /// Prepare for execution (start timers and log)
    ///
    /// Call this before entering the CPU loop.
    pub fn prepare_run(&mut self) {
        tracing::info!("Starting CPU execution at RIP={:#x}", self.cpu.rip());
        self.start();
    }

    /// Get current instruction pointer
    pub fn rip(&self) -> u64 {
        self.cpu.rip()
    }

    /// Return the current VGA text-mode screen as a string.
    ///
    /// This is useful for headless debugging (no terminal repaint).
    pub fn vga_text_dump(&self) -> String {
        self.device_manager.vga.get_text_screen()
    }

    pub fn vga_probe_dump(&self) -> String {
        self.device_manager.vga.probe_summary()
    }

    /// Scan all VGA text memory for any non-space printable characters.
    /// Useful when the screen has been cleared and we need to find if a new
    /// prompt was written somewhere in text_memory that the CRTC start address
    /// may not be pointing to yet.
    pub fn vga_scan_text_memory(&self) -> String {
        self.device_manager.vga.scan_all_text_memory()
    }

    /// Return all rows from VGA text memory (for full-dump diagnostics).
    pub fn vga_all_text_rows(&self) -> alloc::vec::Vec<alloc::string::String> {
        self.device_manager.vga.get_all_text_rows()
    }

    /// Peek at raw RAM at a physical address range (for diagnostics).
    /// Returns up to `len` bytes from the physical RAM array.
    pub fn peek_ram_at(&self, addr: usize, len: usize) -> alloc::vec::Vec<u8> {
        let ram = self.memory.ram_slice();
        if addr + len <= ram.len() {
            ram[addr..addr + len].to_vec()
        } else if addr < ram.len() {
            ram[addr..].to_vec()
        } else {
            alloc::vec::Vec::new()
        }
    }

    /// Check if the emulator has been initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get the current system tick count
    pub fn ticks(&self) -> u64 {
        self.pc_system.time_ticks()
    }

    /// Sync A20 state from system control port to PC system and memory
    ///
    /// Call this after Port 92h writes to update A20 state throughout the system.
    pub fn sync_a20_state(&mut self) {
        self.pc_system.set_enable_a20(self.system_control.a20_gate);
        self.memory.set_a20_mask(self.pc_system.a20_mask());
    }

    /// Process a Port 92h write
    ///
    /// This updates the A20 state and checks for reset requests.
    /// Returns true if a reset was requested.
    pub fn write_port_92h(&mut self, value: u8) -> bool {
        let a20_changed = self.system_control.write(value);

        if a20_changed {
            self.sync_a20_state();
        }

        self.system_control.reset_request
    }

    /// Read Port 92h value
    pub fn read_port_92h(&self) -> u8 {
        self.system_control.read()
    }

    /// Set BIOS output file for port 0x402/0x403/0xE9 messages (requires std feature)
    ///
    /// When set, BIOS debug output will be written to this file instead of stdout.
    #[cfg(feature = "std")]
    pub fn set_bios_output_file(&mut self, file: std::fs::File) {
        self.bios_output_file = Some(file);
    }

    /// Attach a hard disk image (requires std feature)
    ///
    /// # Arguments
    /// * `channel` - ATA channel (0=primary, 1=secondary)
    /// * `drive` - Drive number (0=master, 1=slave)
    /// * `path` - Path to the disk image file
    /// * `cylinders` - Number of cylinders
    /// * `heads` - Number of heads
    /// * `spt` - Sectors per track
    #[cfg(feature = "std")]
    pub fn attach_disk(
        &mut self,
        channel: usize,
        drive: usize,
        path: &str,
        cylinders: u16,
        heads: u8,
        spt: u8,
    ) -> std::io::Result<()> {
        self.device_manager
            .harddrv
            .attach_disk(channel, drive, path, cylinders, heads, spt)
    }

    /// Check if an interrupt is pending
    pub fn has_interrupt(&self) -> bool {
        self.device_manager.has_interrupt()
    }

    /// Acknowledge interrupt and get vector
    pub fn iac(&mut self) -> u8 {
        self.device_manager.iac()
    }

    /// Simulate time passing (for timer-based devices)
    pub fn tick_devices(&mut self, usec: u64) {
        self.device_manager.tick(usec);
    }

    /// Configure CMOS memory size
    pub fn configure_memory_in_cmos(&mut self, base_kb: u16, extended_kb: u16) {
        self.device_manager
            .cmos
            .set_memory_size(base_kb, extended_kb);
    }

    /// Configure CMOS hard drive (type byte only — legacy)
    pub fn configure_disk_in_cmos(&mut self, drive_num: u8, drive_type: u8) {
        self.device_manager
            .cmos
            .set_hard_drive(drive_num, drive_type);
    }

    /// Configure full CMOS hard drive geometry (matching Bochs harddrv.cc:448-474)
    pub fn configure_disk_geometry_in_cmos(
        &mut self,
        drive: u8,
        cylinders: u16,
        heads: u8,
        spt: u8,
    ) {
        self.device_manager
            .cmos
            .configure_disk_geometry(drive, cylinders, heads, spt);
    }

    /// Configure boot sequence in CMOS
    ///
    /// Boot device codes: 0=none, 1=floppy, 2=hard disk, 3=cdrom
    pub fn configure_boot_sequence(&mut self, first: u8, second: u8, third: u8) {
        self.device_manager
            .cmos
            .set_boot_sequence(first, second, third);
    }

    /// Run emulator interactively with GUI event handling
    ///
    /// This method integrates CPU execution with GUI event processing:
    /// - Handles keyboard input from GUI
    /// - Updates GUI display periodically
    /// - Processes device interrupts
    /// - Executes CPU instructions in batches
    ///
    /// Returns the number of instructions executed, or an error.
    #[cfg(feature = "std")]
    pub fn run_interactive(&mut self, max_instructions: u64) -> Result<u64>
    where
        'a: 'static, // Required for unsafe transmute
    {
        self.prepare_run();

        // Verify VGA BIOS ROM is accessible
        {
            // Check through ROM area (not RAM)
            let rom_bytes = self.memory.peek_ram(0xC0000, 4);
            tracing::debug!(
                "VGA ROM check via peek_ram(0xC0000): {:02X?} (expect [55, AA, ...])",
                rom_bytes
            );
            // Also verify IPL table area is writable
            let ipl_bytes = self.memory.peek_ram(0x9FF00, 4);
            tracing::debug!(
                "IPL table check at 0x9FF00: {:02X?} (expect zeros before POST)",
                ipl_bytes
            );
            // Check total memory size
            tracing::debug!("Memory len={:#x}", self.memory.get_memory_len());
        }

        // Force initial GUI update to show initial state
        self.device_manager.vga.force_initial_update();
        self.update_gui(); // Force initial update

        let mut instructions_executed = 0u64;
        let mut last_gui_update = std::time::Instant::now();
        let mut last_ips_update = std::time::Instant::now();
        let mut last_ips_instructions = 0u64;
        const GUI_UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100); // Update every 100ms
        const IPS_UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        let mut last_port92_value: u8 = self.system_control.value;

        const INSTRUCTION_BATCH_SIZE: u64 = 10000; // Larger batch size for better performance

        tracing::info!("Starting interactive execution loop");
        tracing::debug!(
            "[Emulator] Starting execution... (instructions will be processed in batches)"
        );

        // Progress tracking: detect stuck loops
        let mut last_rip: u64 = u64::MAX;
        let mut stuck_count: u32 = 0;
        let mut stuck_reported = false;
        while instructions_executed < max_instructions
            && !self.stop_flag.load(Ordering::Relaxed)
        {
            // 1. Handle GUI events (keyboard input) - do this first to avoid borrow conflicts

            let mut scancodes_to_send = Vec::new();
            if let Some(ref mut gui) = self.gui {
                gui.handle_events();
                scancodes_to_send = gui.get_pending_scancodes();
            }

            // Send scancodes to keyboard device
            for scancode in scancodes_to_send {
                self.device_manager.keyboard.send_scancode(scancode);
            }

            // 2. Execute CPU instructions in batches
            let batch_size = (max_instructions - instructions_executed).min(INSTRUCTION_BATCH_SIZE);
            let batch_start_time = std::time::Instant::now();
            // Use unsafe to work around lifetime issues - the memory borrow is safe because
            // we control the lifetime and the CPU doesn't outlive the memory
            let result = unsafe {
                let mem_extended: &'a mut BxMemC<'a> =
                    core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(&mut self.memory);
                let io_ptr = core::ptr::NonNull::from(&mut self.devices);
                self.cpu
                    .cpu_loop_n_with_io(mem_extended, &[], batch_size, io_ptr)
            };

            let should_update_gui = match result {
                Ok(executed) => {
                    instructions_executed += executed;

                    // Batch timing: log when a batch takes >50ms (indicating perf cliff)
                    let batch_elapsed = batch_start_time.elapsed();
                    if batch_elapsed.as_millis() > 50 {
                        tracing::debug!(
                            "SLOW-BATCH: {}ms for {} instr at {}k total, CS:RIP={:#06x}:{:#x}, icache_miss={}, prefetch={}",
                            batch_elapsed.as_millis(),
                            executed,
                            instructions_executed / 1000,
                            self.cpu.get_cs_selector(),
                            self.cpu.rip(),
                            self.cpu.perf_icache_miss,
                            self.cpu.perf_prefetch,
                        );
                        self.cpu.perf_icache_miss = 0;
                        self.cpu.perf_prefetch = 0;
                    }

                    // If CPU triple-faulted into shutdown, stop emulation loop
                    if self.cpu.is_in_shutdown() {
                        tracing::error!("[Emulator] CPU triple-fault shutdown — stopping");
                        break;
                    }

                    // Port 92h (System Control) may have changed A20 during execution.
                    // Sync PC system + memory masks if any writes occurred.
                    if self.system_control.value != last_port92_value {
                        last_port92_value = self.system_control.value;
                        self.sync_a20_state();
                    }

                    // -- Progress tracking --
                    let current_rip = self.cpu.rip();

                    // Log progress every 10M instructions
                    if instructions_executed % 10_000_000 < INSTRUCTION_BATCH_SIZE {
                        tracing::info!(
                            "Progress: {}M instructions, RIP={:#x}",
                            instructions_executed / 1_000_000,
                            current_rip
                        );
                    }

                    // vsprintf diagnostic removed (bug found and fixed: ADD AL,Ib operated on AH)

                    // Detailed EIP trace to track POST progression
                    // Log every batch in the critical PM→POST transition range
                    if (440_000..480_000).contains(&instructions_executed) {
                        let mem = self.memory.ram_slice();
                        let ipl_count = if 0x9FF81 < mem.len() {
                            u16::from_le_bytes([mem[0x9FF80], mem[0x9FF81]])
                        } else {
                            0
                        };
                        let ipl0_type = if 0x9FF01 < mem.len() {
                            u16::from_le_bytes([mem[0x9FF00], mem[0x9FF01]])
                        } else {
                            0
                        };
                        tracing::debug!(
                            "EIP trace: {} instr, CS:IP={:#06x}:{:#06x}, mode={}, IPL_count={}, IPL0_type={}",
                            instructions_executed,
                            self.cpu.get_cs_selector(),
                            current_rip,
                            self.cpu.get_cpu_mode(),
                            ipl_count, ipl0_type,
                        );
                    }

                    // Detect stuck loop: RIP unchanged for many batches
                    if current_rip == last_rip {
                        stuck_count += 1;
                        if stuck_count >= 10 && !stuck_reported {
                            stuck_reported = true;
                            let bp = self.cpu.bp() as usize;
                            let ss_base = self.cpu.get_ss_base() as usize;
                            let bp_phys = ss_base + bp;
                            let ax = self.cpu.eax() as u16;
                            // Read [BP+2] (return addr) and [BP+4] (action) from memory
                            let mem_peek = self.memory.ram_slice();
                            let bp2 = if bp_phys + 3 < mem_peek.len() {
                                u16::from_le_bytes([mem_peek[bp_phys + 2], mem_peek[bp_phys + 3]])
                            } else {
                                0
                            };
                            let bp4 = if bp_phys + 5 < mem_peek.len() {
                                u16::from_le_bytes([mem_peek[bp_phys + 4], mem_peek[bp_phys + 5]])
                            } else {
                                0
                            };
                            let bp6 = if bp_phys + 7 < mem_peek.len() {
                                u16::from_le_bytes([mem_peek[bp_phys + 6], mem_peek[bp_phys + 7]])
                            } else {
                                0
                            };
                            tracing::debug!(
                                "BIOS stuck at RIP={:#x} after {}k instructions, last I/O read: port={:#06x} value={:#x}, CS={:#06x} mode={}, BP={:#06x} AX={:#06x} [BP+2]={:#06x} [BP+4]={:#06x} [BP+6]={:#06x}",
                                current_rip,
                                instructions_executed / 1000,
                                self.devices.last_io_read_port,
                                self.devices.last_io_read_value,
                                self.cpu.get_cs_selector(),
                                self.cpu.get_cpu_mode(),
                                bp, ax, bp2, bp4, bp6,
                            );
                            // Dump code bytes at stuck RIP for disassembly
                            {
                                let mem = self.memory.ram_slice();
                                let rip_usize = current_rip as usize;
                                if rip_usize + 32 < mem.len() {
                                    let bytes: Vec<String> = mem[rip_usize..rip_usize + 32]
                                        .iter()
                                        .map(|b| format!("{:02x}", b))
                                        .collect();
                                    tracing::debug!(
                                        "Code at RIP={:#x}: {}",
                                        current_rip,
                                        bytes.join(" ")
                                    );
                                }
                                // Dump 256 bytes BEFORE stuck point to see the comparison code
                                let pre_start = rip_usize.saturating_sub(256);
                                if pre_start < mem.len() && rip_usize < mem.len() {
                                    // Dump in 32-byte lines
                                    for offset in (pre_start..rip_usize).step_by(32) {
                                        let end = (offset + 32).min(rip_usize);
                                        let bytes: Vec<String> = mem[offset..end]
                                            .iter()
                                            .map(|b| format!("{:02x}", b))
                                            .collect();
                                        tracing::debug!(
                                            "Code@{:#06x}: {}",
                                            offset,
                                            bytes.join(" ")
                                        );
                                    }
                                }
                                // Also dump all general registers + CR0
                                tracing::debug!(
                                    "Regs: EAX={:#010x} EBX={:#010x} ECX={:#010x} EDX={:#010x} ESI={:#010x} EDI={:#010x} ESP={:#010x} EBP={:#010x} CR0={:#010x}",
                                    self.cpu.eax(), self.cpu.ebx(), self.cpu.ecx(), self.cpu.edx(),
                                    self.cpu.esi(), self.cpu.edi(), self.cpu.esp(), self.cpu.ebp(),
                                    self.cpu.get_cr0_val(),
                                );
                                // For PM stuck points: dump 32-bit stack frame (saved EBP, return addr, args)
                                let ebp = self.cpu.ebp() as usize;
                                if ebp + 16 < mem.len() {
                                    let saved_ebp = u32::from_le_bytes([
                                        mem[ebp],
                                        mem[ebp + 1],
                                        mem[ebp + 2],
                                        mem[ebp + 3],
                                    ]);
                                    let ret_addr = u32::from_le_bytes([
                                        mem[ebp + 4],
                                        mem[ebp + 5],
                                        mem[ebp + 6],
                                        mem[ebp + 7],
                                    ]);
                                    let arg1 = u32::from_le_bytes([
                                        mem[ebp + 8],
                                        mem[ebp + 9],
                                        mem[ebp + 10],
                                        mem[ebp + 11],
                                    ]);
                                    let arg2 = u32::from_le_bytes([
                                        mem[ebp + 12],
                                        mem[ebp + 13],
                                        mem[ebp + 14],
                                        mem[ebp + 15],
                                    ]);
                                    tracing::debug!(
                                        "Stack frame: saved_EBP={:#010x} ret_addr={:#010x} arg1={:#010x} arg2={:#010x}",
                                        saved_ebp, ret_addr, arg1, arg2,
                                    );
                                    // Follow up: dump code around the return address (128 bytes before)
                                    let ra = ret_addr as usize;
                                    if ra > 128 && ra + 32 < mem.len() {
                                        for off in (ra.saturating_sub(128)..ra).step_by(32) {
                                            let end = (off + 32).min(ra);
                                            let bytes: Vec<String> = mem[off..end]
                                                .iter()
                                                .map(|b| format!("{:02x}", b))
                                                .collect();
                                            tracing::debug!(
                                                "Caller@{:#06x}: {}",
                                                off,
                                                bytes.join(" ")
                                            );
                                        }
                                        let after: Vec<String> = mem[ra..ra + 32]
                                            .iter()
                                            .map(|b| format!("{:02x}", b))
                                            .collect();
                                        tracing::debug!(
                                            "Caller code at ret_addr: {}",
                                            after.join(" ")
                                        );
                                    }
                                    // Dump the error message string (EBX = msg ptr in error())
                                    let ebx = self.cpu.ebx() as usize;
                                    if ebx + 64 < mem.len() {
                                        let msg_bytes = &mem[ebx..ebx + 64];
                                        let msg_end =
                                            msg_bytes.iter().position(|&b| b == 0).unwrap_or(64);
                                        let msg_str =
                                            String::from_utf8_lossy(&msg_bytes[..msg_end]);
                                        tracing::debug!(
                                            "Error msg at EBX={:#x}: {:?}",
                                            ebx,
                                            msg_str
                                        );
                                    }
                                    // Also dump string at arg1
                                    let a1 = arg1 as usize;
                                    if a1 + 64 < mem.len() && a1 != ebx {
                                        let msg_bytes = &mem[a1..a1 + 64];
                                        let msg_end =
                                            msg_bytes.iter().position(|&b| b == 0).unwrap_or(64);
                                        let msg_str =
                                            String::from_utf8_lossy(&msg_bytes[..msg_end]);
                                        tracing::debug!(
                                            "Error msg at arg1={:#x}: {:?}",
                                            a1,
                                            msg_str
                                        );
                                    }
                                    // Walk one more frame up
                                    let parent_ebp = saved_ebp as usize;
                                    if parent_ebp + 16 < mem.len() {
                                        let p_saved = u32::from_le_bytes([
                                            mem[parent_ebp],
                                            mem[parent_ebp + 1],
                                            mem[parent_ebp + 2],
                                            mem[parent_ebp + 3],
                                        ]);
                                        let p_ret = u32::from_le_bytes([
                                            mem[parent_ebp + 4],
                                            mem[parent_ebp + 5],
                                            mem[parent_ebp + 6],
                                            mem[parent_ebp + 7],
                                        ]);
                                        tracing::debug!(
                                            "Parent frame: saved_EBP={:#010x} ret_addr={:#010x}",
                                            p_saved,
                                            p_ret
                                        );
                                    }
                                }
                                // Search memory for gzip magic (1f 8b 08) to find where compressed kernel data is
                                {
                                    let search_end = mem.len().min(0x200000); // search first 2MB
                                    let mut found_count = 0;
                                    for i in 0..search_end.saturating_sub(3) {
                                        if mem[i] == 0x1f
                                            && mem[i + 1] == 0x8b
                                            && mem[i + 2] == 0x08
                                        {
                                            let context: Vec<String> = mem[i..i
                                                .min(search_end)
                                                .wrapping_add(32)
                                                .min(search_end)]
                                                .iter()
                                                .map(|b| format!("{:02x}", b))
                                                .collect();
                                            tracing::debug!(
                                                "GZIP magic found at {:#x}: {}",
                                                i,
                                                context.join(" ")
                                            );
                                            found_count += 1;
                                            if found_count >= 10 {
                                                break;
                                            }
                                        }
                                    }
                                    if found_count == 0 {
                                        tracing::debug!("NO gzip magic (1f 8b 08) found in first 2MB of memory!");
                                    }
                                    // Dump expected locations for compressed kernel data:
                                    // After head.S relocates: system code at 0x1000, compressed data at offset ~0x2000-0x4000
                                    for addr in [
                                        0x1000usize,
                                        0x2000,
                                        0x3000,
                                        0x4000,
                                        0x5000,
                                        0x10000,
                                        0x11000,
                                        0x52E00,
                                        0x53E00,
                                        0x62E00,
                                        0x63E00,
                                    ] {
                                        if addr + 16 < mem.len() {
                                            let bytes: Vec<String> = mem[addr..addr + 16]
                                                .iter()
                                                .map(|b| format!("{:02x}", b))
                                                .collect();
                                            tracing::debug!(
                                                "Mem@{:#07x}: {}",
                                                addr,
                                                bytes.join(" ")
                                            );
                                        }
                                    }
                                    // Dump the decompressor's input_data pointer
                                    let esi = self.cpu.esi() as usize;
                                    if esi + 32 < mem.len() {
                                        let bytes: Vec<String> = mem[esi..esi + 32]
                                            .iter()
                                            .map(|b| format!("{:02x}", b))
                                            .collect();
                                        tracing::debug!(
                                            "Data at ESI={:#x}: {}",
                                            esi,
                                            bytes.join(" ")
                                        );
                                    }
                                    // Search for the address 0x41d8 (little-endian) in code region 0x1000-0x4200
                                    // This should appear in instructions that reference input_data
                                    let search_val = [0xd8u8, 0x41, 0x00, 0x00];
                                    for i in 0x1000..0x4200usize {
                                        if i + 4 <= mem.len() && mem[i..i + 4] == search_val {
                                            let ctx_start = i.saturating_sub(4);
                                            let ctx_end = (i + 8).min(mem.len());
                                            let ctx: Vec<String> = mem[ctx_start..ctx_end]
                                                .iter()
                                                .map(|b| format!("{:02x}", b))
                                                .collect();
                                            tracing::debug!(
                                                "Found 0x41d8 ref at {:#06x}: {}",
                                                i,
                                                ctx.join(" ")
                                            );
                                        }
                                    }
                                    // Dump memory right around the gzip data to check alignment
                                    for addr in [0x41d0usize, 0x41d8, 0x41e0, 0x41e8, 0x41f0] {
                                        if addr + 16 < mem.len() {
                                            let bytes: Vec<String> = mem[addr..addr + 16]
                                                .iter()
                                                .map(|b| format!("{:02x}", b))
                                                .collect();
                                            tracing::debug!(
                                                "Mem@{:#07x}: {}",
                                                addr,
                                                bytes.join(" ")
                                            );
                                        }
                                    }
                                    // Dump decompressor key variables (found from code analysis):
                                    // inbuf pointer at 0x510B0, input_len at 0x510AC
                                    // Also dump surrounding BSS to find inptr/insize
                                    for addr in (0x51080..0x51100).step_by(16) {
                                        if addr + 16 < mem.len() {
                                            let v0 = u32::from_le_bytes([
                                                mem[addr],
                                                mem[addr + 1],
                                                mem[addr + 2],
                                                mem[addr + 3],
                                            ]);
                                            let v1 = u32::from_le_bytes([
                                                mem[addr + 4],
                                                mem[addr + 5],
                                                mem[addr + 6],
                                                mem[addr + 7],
                                            ]);
                                            let v2 = u32::from_le_bytes([
                                                mem[addr + 8],
                                                mem[addr + 9],
                                                mem[addr + 10],
                                                mem[addr + 11],
                                            ]);
                                            let v3 = u32::from_le_bytes([
                                                mem[addr + 12],
                                                mem[addr + 13],
                                                mem[addr + 14],
                                                mem[addr + 15],
                                            ]);
                                            tracing::debug!(
                                                "BSS@{:#07x}: {:08x} {:08x} {:08x} {:08x}",
                                                addr,
                                                v0,
                                                v1,
                                                v2,
                                                v3
                                            );
                                        }
                                    }
                                    // Also dump inbuf pointer specifically
                                    if 0x510B4 < mem.len() {
                                        let inbuf_ptr = u32::from_le_bytes([
                                            mem[0x510B0],
                                            mem[0x510B1],
                                            mem[0x510B2],
                                            mem[0x510B3],
                                        ]);
                                        let insize = u32::from_le_bytes([
                                            mem[0x510AC],
                                            mem[0x510AD],
                                            mem[0x510AE],
                                            mem[0x510AF],
                                        ]);
                                        tracing::debug!("Decompressor: inbuf={:#010x} (should be 0x41d8), val_at_0x510AC={:#010x}", inbuf_ptr, insize);
                                        // DIRECT CHECK: Read via peek_ram (which applies vector_offset)
                                        let peek = self.memory.peek_ram(0x510B0, 4);
                                        let peek_val = if peek.len() >= 4 {
                                            u32::from_le_bytes([peek[0], peek[1], peek[2], peek[3]])
                                        } else {
                                            0xDEAD
                                        };
                                        tracing::debug!(
                                            "  DIRECT CHECK: peek_ram(0x510B0)={:#010x} ram_slice[0x510B0]={:#010x}",
                                            peek_val, inbuf_ptr,
                                        );
                                        // Also try reading via CPU's public read_physical_byte path
                                        // (mem_read_dword is private, so use peek_ram from a different offset)
                                        let peek2 = self.memory.peek_ram(0x510A0, 32);
                                        if peek2.len() >= 32 {
                                            let bytes: Vec<String> = peek2
                                                .iter()
                                                .map(|b| format!("{:02x}", b))
                                                .collect();
                                            tracing::debug!(
                                                "  peek_ram(0x510A0..0x510C0): {}",
                                                bytes.join(" ")
                                            );
                                        }
                                        // If inbuf is valid, dump what inbuf points to
                                        let ibp = inbuf_ptr as usize;
                                        if ibp + 16 < mem.len() {
                                            let bytes: Vec<String> = mem[ibp..ibp + 16]
                                                .iter()
                                                .map(|b| format!("{:02x}", b))
                                                .collect();
                                            tracing::debug!("  *inbuf = {}", bytes.join(" "));
                                        }
                                    }
                                    // Also dump wider BSS around 0x50000-0x51200 to find all decompressor globals
                                    for addr in (0x50F80..0x51200).step_by(32) {
                                        if addr + 32 < mem.len() {
                                            let bytes: Vec<String> = mem[addr..addr + 32]
                                                .iter()
                                                .map(|b| format!("{:02x}", b))
                                                .collect();
                                            tracing::debug!(
                                                "WiderBSS@{:#07x}: {}",
                                                addr,
                                                bytes.join(" ")
                                            );
                                        }
                                    }
                                }
                                // Dump Linux boot parameters (memory detection)
                                if mem.len() > 0x90100 {
                                    let ext_mem_k =
                                        u16::from_le_bytes([mem[0x90002], mem[0x90003]]);
                                    let alt_mem_k = u32::from_le_bytes([
                                        mem[0x901e0],
                                        mem[0x901e1],
                                        mem[0x901e2],
                                        mem[0x901e3],
                                    ]);
                                    // Also check BDA memory size at 0x413
                                    let bda_mem = u16::from_le_bytes([mem[0x413], mem[0x414]]);
                                    // Check CMOS values directly (what the BIOS should have read)
                                    let _cmos_ext_lo = mem.get(0x90030).copied().unwrap_or(0);
                                    let _cmos_ext_hi = mem.get(0x90031).copied().unwrap_or(0);
                                    tracing::debug!(
                                        "Boot params: ext_mem_k(0x90002)={} KB, alt_mem_k(0x901e0)={} KB, BDA_mem(0x413)={} KB",
                                        ext_mem_k, alt_mem_k, bda_mem,
                                    );
                                    // Dump first 16 bytes of boot params header at 0x90000
                                    let hdr: Vec<String> = mem[0x90000..0x90010]
                                        .iter()
                                        .map(|b| format!("{:02x}", b))
                                        .collect();
                                    tracing::debug!("Boot params @0x90000: {}", hdr.join(" "));
                                    // Dump setup header at 0x901F1+ (boot protocol version)
                                    let setup_hdr: Vec<String> = mem[0x901F0..0x90200]
                                        .iter()
                                        .map(|b| format!("{:02x}", b))
                                        .collect();
                                    tracing::debug!(
                                        "Setup header @0x901F0: {}",
                                        setup_hdr.join(" ")
                                    );
                                }
                            }
                            // Dump IPL table and stack for debugging
                            {
                                let mem = self.memory.ram_slice();
                                let read_u16 = |addr: usize| -> u16 {
                                    if addr + 1 < mem.len() {
                                        u16::from_le_bytes([mem[addr], mem[addr + 1]])
                                    } else {
                                        0
                                    }
                                };
                                let ipl_count = read_u16(0x9FF80);
                                let ipl_seq = read_u16(0x9FF82);
                                let ipl_bootfirst = read_u16(0x9FF84);
                                let ipl0_type = read_u16(0x9FF00);
                                let ipl1_type = read_u16(0x9FF10);
                                // Also check the WRONG address (get_vector bug: addr % 128KB)
                                let wrong_ipl_count = read_u16(0x1FF80);
                                let wrong_ipl0_type = read_u16(0x1FF00);
                                let wrong_ipl1_type = read_u16(0x1FF10);
                                tracing::debug!(
                                    "IPL table @0x9FF00: count={:#x} seq={:#x} bootfirst={:#x} entry0_type={:#x} entry1_type={:#x}",
                                    ipl_count, ipl_seq, ipl_bootfirst, ipl0_type, ipl1_type,
                                );
                                tracing::debug!(
                                    "IPL table @0x1FF00 (get_vector mapped): count={:#x} entry0_type={:#x} entry1_type={:#x}",
                                    wrong_ipl_count, wrong_ipl0_type, wrong_ipl1_type,
                                );
                                // Dump stack to find caller of bios_printf/BX_PANIC
                                let ss_sel = self.cpu.get_ss_selector();
                                let ss_base = self.cpu.get_ss_base() as usize;
                                let sp = self.cpu.sp() as usize;
                                let stack_addr = ss_base + sp;
                                let mut stack_words = [0u16; 16];
                                for i in 0..16 {
                                    stack_words[i] = read_u16(stack_addr + i * 2);
                                }
                                // Also dump the full stack from SP to 0xFFFE
                                let full_stack_start = stack_addr;
                                let full_stack_end = (ss_base + 0xFFFE).min(mem.len());
                                let full_words: Vec<u16> = (full_stack_start..full_stack_end)
                                    .step_by(2)
                                    .map(|a| read_u16(a))
                                    .collect();
                                let full_hex: Vec<String> =
                                    full_words.iter().map(|w| format!("{:04x}", w)).collect();
                                tracing::debug!(
                                    "Full stack SS:SP={:#06x}:{:#06x} ({} words): {}",
                                    ss_sel,
                                    sp,
                                    full_words.len(),
                                    full_hex.join(" "),
                                );
                                tracing::debug!(
                                    "Stack dump SS:SP={:#06x}:{:#06x} (phys {:#x}): {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x}",
                                    ss_sel, sp, stack_addr,
                                    stack_words[0], stack_words[1], stack_words[2], stack_words[3],
                                    stack_words[4], stack_words[5], stack_words[6], stack_words[7],
                                    stack_words[8], stack_words[9], stack_words[10], stack_words[11],
                                    stack_words[12], stack_words[13], stack_words[14], stack_words[15],
                                );
                                // Also dump BDA and IVT for diagnostics
                                let ebda_seg = read_u16(0x040E);
                                let int08_vec =
                                    read_u16(0x0020) as u32 | ((read_u16(0x0022) as u32) << 16);
                                let int13_vec =
                                    read_u16(0x004C) as u32 | ((read_u16(0x004E) as u32) << 16);
                                let int19_vec =
                                    read_u16(0x0064) as u32 | ((read_u16(0x0066) as u32) << 16);
                                let bda_ticks =
                                    read_u16(0x046C) as u32 | ((read_u16(0x046E) as u32) << 16);
                                let bda_kbd_head = read_u16(0x041A);
                                tracing::debug!(
                                    "IVT: INT08={:#010x} INT13={:#010x} INT19={:#010x} | BDA: EBDA={:#06x} ticks={} kbd_head={:#06x}",
                                    int08_vec, int13_vec, int19_vec, ebda_seg, bda_ticks, bda_kbd_head,
                                );
                            }
                        }
                    } else {
                        stuck_count = 0;
                        stuck_reported = false;
                        last_rip = current_rip;
                    }

                    // Drain Bochs-style port 0xE9 output (if any) and print it.
                    // This is useful for very early debug output before VGA is initialized.
                    let e9 = self.devices.take_port_e9_output();
                    if !e9.is_empty() {
                        use std::io::Write;
                        // Write to BIOS output file if configured, otherwise to stdout
                        #[cfg(feature = "std")]
                        if let Some(ref mut bios_file) = self.bios_output_file {
                            bios_file.write_all(&e9).ok();
                            bios_file.flush().ok();
                        } else {
                            let mut out = std::io::stdout();
                            out.write_all(&e9).ok();
                            out.flush().ok();
                        }

                        #[cfg(not(feature = "std"))]
                        {
                            let mut out = std::io::stdout();
                            out.write_all(&e9).ok();
                            out.flush().ok();
                        }
                    }

                    // Advance virtual time (Bochs-like ticking).
                    // Required so PIT can generate IRQ0 and BIOS can progress past HLT waits.
                    if self.config.ips != 0 {
                        let usec_from_instr =
                            (executed.saturating_mul(1_000_000)) / (self.config.ips as u64);
                        // When CPU is halted, advance time aggressively (5ms per batch) so PIT
                        // fires IRQ0 quickly (~11 batches per 54.9ms PIT cycle). When active,
                        // use min 10 usec to prevent timer starvation at low instruction counts.
                        let min_usec = if matches!(
                            self.cpu.activity_state,
                            crate::cpu::cpu::CpuActivityState::Hlt
                        ) {
                            5000
                        } else {
                            10
                        };
                        let usec = usec_from_instr.max(min_usec);
                        self.tick_devices(usec);
                    }

                    // Propagate A20 gate changes from keyboard controller to memory system
                    // Matching Bochs BX_SET_ENABLE_A20() which immediately updates pc_system and memory
                    if self.device_manager.keyboard.a20_change_pending {
                        self.device_manager.keyboard.a20_change_pending = false;
                        let a20 = self.device_manager.keyboard.a20_enabled;
                        self.pc_system.set_enable_a20(a20);
                        self.memory.set_a20_mask(self.pc_system.a20_mask());
                    }

                    // Log batch sizes and check if timer ticking works
                    if instructions_executed < 5 * INSTRUCTION_BATCH_SIZE
                        || instructions_executed % 100_000 < INSTRUCTION_BATCH_SIZE
                    {
                        let pit_c0_count = self.device_manager.pit.counters[0].count;
                        // Read BDA timer tick counter at 0x046C (4 bytes) directly from RAM
                        let bda_ticks = {
                            let (ptr, len) = self.memory.get_raw_memory_ptr();
                            if 0x046C + 4 <= len {
                                unsafe {
                                    let p = ptr.add(0x046C) as *const u32;
                                    *p
                                }
                            } else {
                                0
                            }
                        };
                        tracing::debug!("BATCH-DIAG: executed={}, total={}k, RIP={:#x}, PIT_count={}, activity={:?}, BDA_ticks={}",
                            executed, instructions_executed / 1000, self.cpu.rip(), pit_c0_count,
                            self.cpu.activity_state, bda_ticks);
                    }

                    // Periodic interrupt-chain diagnostic (every ~1M instructions)
                    if instructions_executed % 1_000_000 < INSTRUCTION_BATCH_SIZE {
                        let has_int = self.has_interrupt();
                        let if_flag = self.cpu.get_b_if();
                        let rip = self.cpu.rip();
                        let pit_c0 = &self.device_manager.pit.counters[0];
                        tracing::debug!(
                            "IRQ-DIAG: {}M instr, RIP={:#x}, IF={}, has_int={}, PIC_imr={:#04x}, PIC_irr={:#04x}, PIT_c0: mode={:?} inlatch={} count={} count_written={} gate={} output={}",
                            instructions_executed / 1_000_000,
                            rip,
                            if_flag,
                            has_int,
                            self.device_manager.pic.master.imr,
                            self.device_manager.pic.master.irr,
                            self.device_manager.pit.counters[0].mode,
                            pit_c0.inlatch,
                            pit_c0.count,
                            pit_c0.count_written,
                            pit_c0.gate,
                            pit_c0.output,
                        );
                    }

                    // Decompressor progress check (every 1M instructions) — DISABLED (decompressor works)
                    if false && instructions_executed % 1_000_000 < INSTRUCTION_BATCH_SIZE {
                        let rip = self.cpu.rip();
                        // Check if we're in the decompressor (RIP in 0x1000-0x6000 range)
                        if rip >= 0x1000 && rip < 0x6000 {
                            let peek_inptr = self.memory.peek_ram(0x4004, 4);
                            let inptr_val = u32::from_le_bytes([
                                peek_inptr[0],
                                peek_inptr[1],
                                peek_inptr[2],
                                peek_inptr[3],
                            ]);
                            // Dump code at the inflate loop addresses
                            let mem = self.memory.ram_slice();
                            let _code_23e0: Vec<String> = mem[0x23E0..0x2420]
                                .iter()
                                .map(|b| format!("{:02x}", b))
                                .collect();
                            // Decompressor global vars
                            let peek = |addr: usize| -> u32 {
                                let p = self.memory.peek_ram(addr, 4);
                                u32::from_le_bytes([p[0], p[1], p[2], p[3]])
                            };
                            let outcnt = peek(0x4008);
                            let bytes_out = peek(0x400C);
                            let _wp = peek(0x4010); // window position / output_ptr
                                                    // Check output at 0x100000 and 0x108000
                            let peek_out = self.memory.peek_ram(0x100000, 8);
                            let out_hex: Vec<String> =
                                peek_out.iter().map(|b| format!("{:02x}", b)).collect();
                            let peek_out2 = self.memory.peek_ram(0x108000, 8);
                            let _out2_hex: Vec<String> =
                                peek_out2.iter().map(|b| format!("{:02x}", b)).collect();
                            // Check the window buffer area (0x51100-0x59100 based on BSS layout)
                            let peek_win = self.memory.peek_ram(0x51100, 8);
                            let _win_hex: Vec<String> =
                                peek_win.iter().map(|b| format!("{:02x}", b)).collect();
                            // Also check window buffer contents (wider sample)
                            let peek_win16 = self.memory.peek_ram(0x510b8, 32);
                            let win16_hex: Vec<String> =
                                peek_win16.iter().map(|b| format!("{:02x}", b)).collect();
                            // Check Huffman table area — tl/td pointers stored somewhere
                            // Dump the key inflate globals (bb, bk might be on stack)
                            let esp_val = self.cpu.esp() as usize;
                            let stack_peek = if esp_val > 0 && esp_val < 0x100000 {
                                let s = self.memory.peek_ram(esp_val, 32);
                                let h: Vec<String> =
                                    s.iter().map(|b| format!("{:02x}", b)).collect();
                                h.join(" ")
                            } else {
                                "N/A".to_string()
                            };
                            tracing::debug!(
                                "DECOMP-PROGRESS: {}M instr, inptr={}/{} outcnt={} bytes_out={:#x} RIP={:#x} out@100000:{} win@510b8:{} stack@ESP:{}",
                                instructions_executed / 1_000_000,
                                inptr_val, 0x4CED4u32,
                                outcnt, bytes_out,
                                rip, out_hex.join(" "), win16_hex.join(" "), stack_peek,
                            );
                            if instructions_executed / 1_000_000 >= 1
                                && instructions_executed / 1_000_000 <= 3
                            {
                                // inflate_codes EBP is 0x5CF1C (from trace), not the current EBP (which may be memcpy's)
                                let ic_ebp = 0x5CF1Cu32 as usize;
                                if ic_ebp + 0x30 < mem.len() {
                                    let rd = |off: usize| -> u32 {
                                        u32::from_le_bytes([
                                            mem[ic_ebp + off],
                                            mem[ic_ebp + off + 1],
                                            mem[ic_ebp + off + 2],
                                            mem[ic_ebp + off + 3],
                                        ])
                                    };
                                    let saved_ebp = rd(0);
                                    let ret_addr = rd(4);
                                    let arg1 = rd(8); // tl
                                    let arg2 = rd(0xC); // td
                                    let arg3 = rd(0x10); // bl
                                    let arg4 = rd(0x14); // bd
                                    tracing::debug!("INFLATE-CODES @EBP=0x5CF1C: saved_EBP={:#x} ret={:#x} tl={:#x} td={:#x} bl={} bd={}",
                                        saved_ebp, ret_addr, arg1, arg2, arg3, arg4);
                                    // Also dump the inflate_codes local variables
                                    let locals = self.memory.peek_ram(ic_ebp - 0x30, 0x60);
                                    let locals_hex: Vec<String> =
                                        locals.iter().map(|b| format!("{:02x}", b)).collect();
                                    tracing::debug!(
                                        "inflate_codes frame [EBP-0x30..EBP+0x30]: {}",
                                        locals_hex.join(" ")
                                    );
                                    // Check heap pointer
                                    let free_mem_ptr = u32::from_le_bytes([
                                        mem[0x4014],
                                        mem[0x4015],
                                        mem[0x4016],
                                        mem[0x4017],
                                    ]);
                                    tracing::debug!("free_mem_ptr@0x4014={:#x}", free_mem_ptr);
                                    // Dump compressed data (gzip+deflate) at 0x41D8
                                    let cdata = self.memory.peek_ram(0x41D8, 64);
                                    let cdata_hex: Vec<String> =
                                        cdata.iter().map(|b| format!("{:02x}", b)).collect();
                                    tracing::debug!(
                                        "Compressed data @0x41D8: {}",
                                        cdata_hex.join(" ")
                                    );
                                    // Check who called inflate_codes (return address 0x253D)
                                    // Dump code around 0x2530 to see the CALL instruction
                                    let call_area = self.memory.peek_ram(0x2520, 64);
                                    let call_hex: Vec<String> =
                                        call_area.iter().map(|b| format!("{:02x}", b)).collect();
                                    tracing::debug!(
                                        "Code around inflate_codes CALL @0x2520: {}",
                                        call_hex.join(" ")
                                    );
                                    // Dump the full inflate_dynamic loop body (0x21A0-0x2430)
                                    let loop_code1 = self.memory.peek_ram(0x21A0, 96);
                                    let lc1_hex: Vec<String> =
                                        loop_code1.iter().map(|b| format!("{:02x}", b)).collect();
                                    tracing::debug!("Code @0x21A0-0x21FF: {}", lc1_hex.join(" "));
                                    let loop_code2 = self.memory.peek_ram(0x2200, 32);
                                    let lc2_hex: Vec<String> =
                                        loop_code2.iter().map(|b| format!("{:02x}", b)).collect();
                                    tracing::debug!("Code @0x2200-0x221F: {}", lc2_hex.join(" "));
                                    let loop_code3 = self.memory.peek_ram(0x2410, 32);
                                    let lc3_hex: Vec<String> =
                                        loop_code3.iter().map(|b| format!("{:02x}", b)).collect();
                                    tracing::debug!("Code @0x2410-0x242F: {}", lc3_hex.join(" "));
                                    // Dump code before the second loop to find the first loop
                                    let code_2100 = self.memory.peek_ram(0x2100, 96);
                                    let c2100_hex: Vec<String> =
                                        code_2100.iter().map(|b| format!("{:02x}", b)).collect();
                                    tracing::debug!("Code @0x2100-0x215F: {}", c2100_hex.join(" "));
                                    let code_2160 = self.memory.peek_ram(0x2160, 64);
                                    let c2160_hex: Vec<String> =
                                        code_2160.iter().map(|b| format!("{:02x}", b)).collect();
                                    tracing::debug!("Code @0x2160-0x219F: {}", c2160_hex.join(" "));
                                    // Dump huft_build function (starts at 0x108C)
                                    for chunk_start in (0x108Cu32..0x1700u32).step_by(64) {
                                        let cs = chunk_start as usize;
                                        let code = self.memory.peek_ram(cs, 64);
                                        let hex: Vec<String> =
                                            code.iter().map(|b| format!("{:02x}", b)).collect();
                                        tracing::debug!(
                                            "Code @{:#06x}: {}",
                                            chunk_start,
                                            hex.join(" ")
                                        );
                                    }
                                    // Also check the caller's (inflate_fixed/dynamic) stack frame
                                    // Dump code-length Huffman table at tl=0x5d4e0
                                    // Each entry is 8 bytes: [exop:1][bits:1][pad:2][base:4]
                                    // Bochs inflate huft: { e:u8, b:u8, v:{n:u16 or t:ptr} }
                                    let tl_addr = 0x5d4e0usize;
                                    if tl_addr + 128 * 8 < mem.len() {
                                        // Dump ALL non-zero entries in first 128
                                        let mut nonzero_count = 0;
                                        for idx in 0..128 {
                                            let off = tl_addr + idx * 8;
                                            let e = mem[off];
                                            let b = mem[off + 1];
                                            let vn =
                                                u16::from_le_bytes([mem[off + 4], mem[off + 5]]);
                                            if e != 0 || b != 0 || vn != 0 {
                                                nonzero_count += 1;
                                                if nonzero_count <= 40 {
                                                    tracing::debug!(
                                                        "HUFT[{}] @{:#x}: e={} b={} v.n={}",
                                                        idx,
                                                        off,
                                                        e,
                                                        b,
                                                        vn
                                                    );
                                                }
                                            }
                                        }
                                        tracing::debug!(
                                            "HUFT table: {} non-zero entries out of 128",
                                            nonzero_count
                                        );
                                        // Also dump entry 39
                                        let idx = 39;
                                        let off = tl_addr + idx * 8;
                                        tracing::debug!("HUFT[39] raw: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                                            mem[off], mem[off+1], mem[off+2], mem[off+3], mem[off+4], mem[off+5], mem[off+6], mem[off+7]);
                                    }
                                    // Dump ll[] array (inflate_dynamic local at [EBP-0x4F0])
                                    // inflate_dynamic EBP = 0x5D45C
                                    let id_ebp = 0x5D45Cusize;
                                    let ll_base = id_ebp - 0x4F0; // 0x5CF6C
                                                                  // ll[] has 19 entries for code-length codes (border[0..18])
                                                                  // Actually ll[] is 286+30=316 entries of unsigned int
                                                                  // First dump the 19 code-length code lengths
                                    let mut ll_vals = Vec::new();
                                    for idx in 0..19 {
                                        let off = ll_base + idx * 4;
                                        if off + 4 <= mem.len() {
                                            let v = u32::from_le_bytes([
                                                mem[off],
                                                mem[off + 1],
                                                mem[off + 2],
                                                mem[off + 3],
                                            ]);
                                            ll_vals.push(format!("{}:{}", idx, v));
                                        }
                                    }
                                    tracing::debug!(
                                        "ll[0..19] (code-length code lengths): {}",
                                        ll_vals.join(" ")
                                    );
                                    // Also check bl and nb values
                                    let bl_val = u32::from_le_bytes([
                                        mem[id_ebp - 0x4F8],
                                        mem[id_ebp - 0x4F7],
                                        mem[id_ebp - 0x4F6],
                                        mem[id_ebp - 0x4F5],
                                    ]);
                                    tracing::debug!(
                                        "inflate_dynamic: bl=[EBP-0x4F8]={}, tl=[EBP-0x4F4]={:#x}",
                                        bl_val,
                                        u32::from_le_bytes([
                                            mem[id_ebp - 0x4F4],
                                            mem[id_ebp - 0x4F3],
                                            mem[id_ebp - 0x4F2],
                                            mem[id_ebp - 0x4F1]
                                        ])
                                    );
                                    // Dump border[] array at 0x4024 (19 entries of 4 bytes)
                                    let border_addr = 0x4024usize;
                                    if border_addr + 19 * 4 <= mem.len() {
                                        let mut bvals = Vec::new();
                                        for idx in 0..19 {
                                            let off = border_addr + idx * 4;
                                            let v = u32::from_le_bytes([
                                                mem[off],
                                                mem[off + 1],
                                                mem[off + 2],
                                                mem[off + 3],
                                            ]);
                                            bvals.push(format!("{}", v));
                                        }
                                        tracing::debug!("border[] @0x4024: {}", bvals.join(" "));
                                    }
                                    // Dump mask_bits[] array (used for lookup mask)
                                    // mask_bits is at 0x4064 based on code patterns (17 entries of 2 bytes: ush)
                                    // Actually, let's find it. mask_bits[bl] was loaded as [EBP-0x508].
                                    // The code at 0x2160 references 0x4164:
                                    //   0f b7 04 45 64 41 00 00 = MOVZX EAX, word [EAX*2+0x4164]
                                    // So mask_bits is at 0x4164
                                    let mask_addr = 0x4164usize;
                                    if mask_addr + 17 * 2 <= mem.len() {
                                        let mut mvals = Vec::new();
                                        for idx in 0..17 {
                                            let off = mask_addr + idx * 2;
                                            let v = u16::from_le_bytes([mem[off], mem[off + 1]]);
                                            mvals.push(format!("{}", v));
                                        }
                                        tracing::debug!("mask_bits[] @0x4164: {}", mvals.join(" "));
                                    }
                                    // saved_EBP from inflate_codes points to caller
                                    if saved_ebp > 0 && (saved_ebp as usize) + 0x30 < mem.len() {
                                        let caller_ebp = saved_ebp as usize;
                                        let caller_ret = u32::from_le_bytes([
                                            mem[caller_ebp + 4],
                                            mem[caller_ebp + 5],
                                            mem[caller_ebp + 6],
                                            mem[caller_ebp + 7],
                                        ]);
                                        tracing::debug!(
                                            "Caller frame @EBP={:#x}: ret={:#x}",
                                            saved_ebp,
                                            caller_ret
                                        );
                                        // Dump caller's local variables
                                        let caller_locals =
                                            self.memory.peek_ram(caller_ebp - 0x10, 0x30);
                                        let cl_hex: Vec<String> = caller_locals
                                            .iter()
                                            .map(|b| format!("{:02x}", b))
                                            .collect();
                                        tracing::debug!(
                                            "Caller locals [EBP-0x10..EBP+0x20]: {}",
                                            cl_hex.join(" ")
                                        );
                                    }
                                }
                                // Also dump registers
                                tracing::debug!("REGS: EAX={:#x} ECX={:#x} EDX={:#x} EBX={:#x} ESP={:#x} EBP={:#x} ESI={:#x} EDI={:#x}",
                                    self.cpu.eax(), self.cpu.ecx(), self.cpu.edx(), self.cpu.ebx(),
                                    self.cpu.esp(), self.cpu.ebp(), self.cpu.esi(), self.cpu.edi());
                            }
                        }
                    }

                    // Deliver pending PIC interrupts to the CPU (Bochs-like).
                    {
                        let has_int = self.has_interrupt();
                        let if_flag = self.cpu.get_b_if();
                        if has_int {
                            tracing::trace!(
                                "INT-DELIVER: has_int={}, IF={}, activity={:?}, RIP={:#x}",
                                has_int,
                                if_flag,
                                self.cpu.activity_state,
                                self.cpu.rip()
                            );
                        }
                    }
                    if self.has_interrupt()
                        && self.cpu.get_b_if() != 0
                        && !self.cpu.interrupts_inhibited(0x01)
                    // BX_INHIBIT_INTERRUPTS
                    {
                        let vector = self.iac();
                        tracing::trace!(
                            "INT-INJECT: vector={:#04x}, activity_before={:?}",
                            vector,
                            self.cpu.activity_state
                        );

                        // Temporarily wire the memory bus so the interrupt path can
                        // read IVT/IDT and push stack frames correctly.
                        let inject_result = unsafe {
                            let mem_extended: &'a mut BxMemC<'a> =
                                core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(
                                    &mut self.memory,
                                );
                            self.cpu
                                .set_mem_bus_ptr(core::ptr::NonNull::from(&mut *mem_extended));
                            let r = self.cpu.inject_external_interrupt(vector);
                            self.cpu.clear_mem_bus();
                            r
                        };

                        match &inject_result {
                            Ok(()) => {
                                tracing::debug!(
                                    "INT-INJECT: OK! activity_after={:?}, RIP={:#x}",
                                    self.cpu.activity_state,
                                    self.cpu.rip()
                                );
                            }
                            Err(e) => {
                                tracing::error!("INT-INJECT: FAILED: {:?}", e);
                                return Err(crate::Error::Cpu(inject_result.unwrap_err()));
                            }
                        }
                    }

                    // Progress logging removed per user request

                    // 4. Check if GUI should be updated
                    // Update when text is dirty, or periodically to catch any missed updates
                    let text_dirty = self.device_manager.vga.is_text_dirty();
                    let time_since_update = last_gui_update.elapsed();
                    // Update if text changed OR periodically (like Bochs timer-based updates)
                    let should_update = text_dirty || time_since_update >= GUI_UPDATE_INTERVAL;

                    // Update timestamp if we're going to update
                    if should_update {
                        last_gui_update = std::time::Instant::now();
                    }

                    should_update
                }
                Err(e) => {
                    tracing::error!("CPU execution error: {:?}", e);
                    tracing::debug!("[Emulator] ERROR: {:?}", e);
                    return Err(crate::Error::Cpu(e));
                }
            };

            // Update GUI after CPU execution (outside the match to avoid borrow conflicts)
            // Update more frequently if text is dirty OR periodically (like Bochs timer)
            if should_update_gui {
                self.update_gui();
            }

            // Update IPS counter every second
            let ips_elapsed = last_ips_update.elapsed();
            if ips_elapsed >= IPS_UPDATE_INTERVAL {
                let delta_instr = instructions_executed - last_ips_instructions;
                let mips = (delta_instr as f64 / ips_elapsed.as_secs_f64()) / 1_000_000.0;
                let ips = (mips * 1_000_000.0) as u32;
                last_ips_instructions = instructions_executed;
                last_ips_update = std::time::Instant::now();
                if let Some(ref mut gui) = self.gui {
                    gui.show_ips(ips);
                }
                tracing::error!(
                    target: "mips",
                    "[{:>6}M instr] {:>6.2} MIPS  RIP={:#010x}  CS={:#06x}  mode={}",
                    instructions_executed / 1_000_000,
                    mips,
                    self.cpu.rip(),
                    self.cpu.get_cs_selector(),
                    self.cpu.get_cpu_mode(),
                );
            }

            // 5. Check if we should exit (e.g., shutdown requested)
            // TODO: Add shutdown flag check
        }

        tracing::debug!(
            "Interactive execution completed: {} instructions",
            instructions_executed
        );

        Ok(instructions_executed)
    }

    /// Execute a batch of instructions cooperatively (no blocking loop).
    ///
    /// Designed for single-threaded environments like WASM where the caller
    /// must yield control back to the event loop regularly. Runs up to
    /// `max_instructions`, ticks devices, syncs A20, then returns.
    ///
    /// Returns `(instructions_executed, is_shutdown)`.
    pub fn step_batch(&mut self, max_instructions: u64) -> Result<(u64, bool)>
    where
        'a: 'static,
    {
        let result = unsafe {
            let mem_extended: &'a mut BxMemC<'a> =
                core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(&mut self.memory);
            let io_ptr = core::ptr::NonNull::from(&mut self.devices);
            self.cpu
                .cpu_loop_n_with_io(mem_extended, &[], max_instructions, io_ptr)
        };

        match result {
            Ok(mut executed) => {
                let ips = self.config.ips as u64;
                let usec = if ips > 0 {
                    (executed * 1_000_000 / ips).max(10)
                } else {
                    10
                };
                self.tick_devices(usec);

                // When CPU is halted, advance one full PIT cycle so timer
                // interrupts can fire (Bochs handleWaitForEvent BX_TICKN loop).
                if matches!(
                    self.cpu.activity_state,
                    crate::cpu::cpu::CpuActivityState::Hlt
                ) {
                    self.tick_devices(60_000);
                }

                // Deliver pending PIC interrupts — matches run_interactive().
                // This must happen EVERY batch, not just during HLT, because
                // the BIOS and OS rely on timer interrupts during normal
                // execution (not only when halted).
                if self.has_interrupt()
                    && self.cpu.get_b_if() != 0
                    && !self.cpu.interrupts_inhibited(0x01)
                {
                    let vector = self.iac();
                    let _inject = unsafe {
                        let mem_extended: &'a mut BxMemC<'a> =
                            core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(
                                &mut self.memory,
                            );
                        self.cpu
                            .set_mem_bus_ptr(core::ptr::NonNull::from(&mut *mem_extended));
                        let r = self.cpu.inject_external_interrupt(vector);
                        self.cpu.clear_mem_bus();
                        r
                    };
                }

                // If CPU was halted and we just delivered an interrupt,
                // re-enter CPU loop so the handler runs in this batch.
                if matches!(
                    self.cpu.activity_state,
                    crate::cpu::cpu::CpuActivityState::Active
                ) && executed == 0
                {
                    let result2 = unsafe {
                        let mem_extended: &'a mut BxMemC<'a> =
                            core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(
                                &mut self.memory,
                            );
                        let io_ptr = core::ptr::NonNull::from(&mut self.devices);
                        self.cpu
                            .cpu_loop_n_with_io(mem_extended, &[], max_instructions, io_ptr)
                    };
                    if let Ok(executed2) = result2 {
                        executed += executed2;
                        let usec2 = if ips > 0 {
                            (executed2 * 1_000_000 / ips).max(10)
                        } else {
                            10
                        };
                        self.tick_devices(usec2);
                    }
                }

                // Sync A20 state
                self.sync_a20_state();

                // Handle keyboard scancodes from GUI
                let mut scancodes_to_send = Vec::new();
                if let Some(ref mut gui) = self.gui {
                    gui.handle_events();
                    scancodes_to_send = gui.get_pending_scancodes();
                }
                for scancode in scancodes_to_send {
                    self.device_manager.keyboard.send_scancode(scancode);
                }

                let shutdown = self.cpu.is_in_shutdown();
                Ok((executed, shutdown))
            }
            Err(e) => Err(crate::error::Error::Cpu(e)),
        }
    }

    /// Attach a hard disk from in-memory data (for no_std / WASM environments).
    ///
    /// Wraps `HardDrive::attach_disk_data()` which stores the disk image
    /// in a `Vec<u8>` instead of using file I/O.
    #[cfg(not(feature = "std"))]
    pub fn attach_disk_data(
        &mut self,
        channel: usize,
        drive: usize,
        data: alloc::vec::Vec<u8>,
        cylinders: u16,
        heads: u8,
        spt: u8,
    ) {
        self.device_manager
            .harddrv
            .attach_disk_data(channel, drive, data, cylinders, heads, spt);
    }

    /// Render VGA text output into a `SharedDisplay` framebuffer.
    ///
    /// This is the single-threaded equivalent of `update_gui()` — instead of
    /// going through the `BxGui` trait (which requires `Arc<Mutex<>>` for
    /// thread-safe sharing), it writes directly to the provided display.
    /// Ideal for WASM where the emulator and display are owned by the same
    /// event loop.
    pub fn update_display(&mut self, display: &mut crate::gui::shared_display::SharedDisplay) {
        // Debug: log VGA state periodically
        static mut DBG_CTR: u32 = 0;
        unsafe {
            DBG_CTR += 1;
        }
        let dbg = unsafe { DBG_CTR };

        if let Some(update_result) = self.device_manager.vga.update() {
            if dbg % 300 == 1 {
                // Check if text_buffer has any non-zero bytes
                let non_zero = update_result
                    .text_buffer
                    .iter()
                    .filter(|&&b| b != 0)
                    .count();
                let first_16: Vec<u8> =
                    update_result.text_buffer.iter().take(32).copied().collect();
                tracing::warn!(
                    "VGA update: dim_changed={}, needs_update={}, buf_non_zero={}, first_32={:02x?}, start_addr={}",
                    update_result.dimension_changed,
                    update_result.needs_update,
                    non_zero,
                    first_16,
                    update_result.tm_info.start_address,
                );
            }
            let cursor_x = if update_result.cursor_address < 0x7fff {
                let offset_from_start = update_result
                    .cursor_address
                    .saturating_sub(update_result.tm_info.start_address);
                (offset_from_start % update_result.tm_info.line_offset) / 2
            } else {
                0xffff
            };

            let cursor_y = if update_result.cursor_address < 0x7fff {
                let offset_from_start = update_result
                    .cursor_address
                    .saturating_sub(update_result.tm_info.start_address);
                (offset_from_start / update_result.tm_info.line_offset) as u32
            } else {
                0xffff
            };

            if update_result.dimension_changed {
                display.resize(
                    if update_result.fwidth > 0 {
                        update_result.iwidth / update_result.fwidth
                    } else {
                        update_result.iwidth
                    },
                    if update_result.fheight > 0 {
                        update_result.iheight / update_result.fheight
                    } else {
                        update_result.iheight
                    },
                    update_result.fwidth,
                    update_result.fheight,
                );
            }

            display.render_text_to_framebuffer(
                &update_result.text_buffer,
                cursor_x as u32,
                cursor_y as u32,
                update_result.tm_info.cs_start,
                update_result.tm_info.cs_end,
                update_result.tm_info.line_graphics,
                update_result.tm_info.start_address as u32,
                update_result.tm_info.line_offset as u32,
            );
        } else if dbg % 300 == 1 {
            // VGA returned None — not in text mode or not initialized
            let gr6 = self.device_manager.vga.graphics_regs[6];
            let ga = (gr6 & 0x01) != 0;
            let mm = (gr6 >> 2) & 0x03;
            tracing::warn!(
                "VGA update returned None: graphics_alpha={}, memory_mapping={}, gr6=0x{:02x}",
                ga,
                mm,
                gr6,
            );
        }
    }

    /// Send a PS/2 scancode to the keyboard device.
    ///
    /// For environments that handle keyboard input outside of `BxGui`
    /// (e.g. the WASM app processes egui events directly).
    pub fn send_scancode(&mut self, scancode: u8) {
        self.device_manager.keyboard.send_scancode(scancode);
    }

    /// Force VGA to generate an initial update (call before first `update_display`).
    pub fn force_vga_update(&mut self) {
        self.device_manager.vga.force_initial_update();
    }

    /// Get VGA memory handler probe summary for diagnostics.
    pub fn vga_probe_summary(&self) -> alloc::string::String {
        self.device_manager.vga.probe_summary()
    }

    /// Get the number of registered memory handlers (for diagnostics).
    pub fn memory_handler_count(&self) -> usize {
        self.memory.memory_handler_info()
    }

    /// Get current CS:RIP for diagnostics.
    pub fn get_cs_rip(&self) -> (u16, u64) {
        (self.cpu.get_cs_selector(), self.cpu.rip())
    }

    /// Get CPU mode string for diagnostics.
    pub fn get_cpu_mode_str(&self) -> &'static str {
        match self.cpu.get_cpu_mode() {
            0 => "real",
            1 => "v8086",
            2 => "protected",
            3 => "long-compat",
            4 => "long-64",
            _ => "unknown",
        }
    }

    /// Get CR0 for diagnostics (bit 0 = PE).
    pub fn get_cr0(&self) -> u32 {
        self.cpu.cr0.bits()
    }

    /// Get IF flag for diagnostics.
    pub fn get_if_flag(&self) -> bool {
        self.cpu.get_b_if() != 0
    }

    /// Read a few bytes from the BIOS ROM array at the given ROM offset.
    pub fn peek_rom(&self, offset: usize, len: usize) -> alloc::vec::Vec<u8> {
        self.memory.peek_rom(offset, len)
    }

    /// Get VGA Graphics Register 6 (memory mapping control).
    pub fn peek_vga_gr6(&self) -> u8 {
        self.device_manager.vga.graphics_regs[6]
    }

    /// Get the activity state string.
    pub fn get_activity_str(&self) -> &'static str {
        match self.cpu.activity_state {
            crate::cpu::cpu::CpuActivityState::Active => "active",
            crate::cpu::cpu::CpuActivityState::Hlt => "hlt",
            crate::cpu::cpu::CpuActivityState::Shutdown => "shutdown",
            _ => "other",
        }
    }
}

// Ensure Emulator is Send (can be moved between threads)
// Each instance is fully independent with no shared state
unsafe impl<I: BxCpuIdTrait + Send> Send for Emulator<'_, I> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::core_i7_skylake::Corei7SkylakeX;

    #[test]
    fn test_emulator_creation() {
        let config = EmulatorConfig::default();
        let emu = Emulator::<Corei7SkylakeX>::new(config);
        assert!(emu.is_ok());
    }

    #[test]
    fn test_emulator_initialization() {
        let config = EmulatorConfig::default();
        let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
        assert!(!emu.is_initialized());

        let result = emu.initialize();
        assert!(result.is_ok());
        assert!(emu.is_initialized());
    }

    #[test]
    fn test_multiple_instances_independent() {
        let config = EmulatorConfig::default();

        let mut emu1 = Emulator::<Corei7SkylakeX>::new(config.clone()).unwrap();
        let emu2 = Emulator::<Corei7SkylakeX>::new(config).unwrap();

        emu1.initialize().unwrap();

        // emu2 should still be uninitialized
        assert!(emu1.is_initialized());
        assert!(!emu2.is_initialized());

        // Different tick counts
        emu1.pc_system.tick(1000);
        assert_eq!(emu1.ticks(), 1000);
        assert_eq!(emu2.ticks(), 0);
    }
}
