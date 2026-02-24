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

use alloc::{boxed::Box, string::String, vec::Vec};

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
            tracing::warn!("Emulator already initialized");
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
            tracing::warn!("Emulator already initialized");
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
            tracing::warn!("No GUI set, running headless");
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
            tracing::warn!(
                "VGA ROM check via peek_ram(0xC0000): {:02X?} (expect [55, AA, ...])",
                rom_bytes
            );
            // Also verify IPL table area is writable
            let ipl_bytes = self.memory.peek_ram(0x9FF00, 4);
            tracing::warn!(
                "IPL table check at 0x9FF00: {:02X?} (expect zeros before POST)",
                ipl_bytes
            );
            // Check total memory size
            tracing::warn!(
                "Memory len={:#x}", self.memory.get_memory_len()
            );
        }

        // Force initial GUI update to show initial state
        self.device_manager.vga.force_initial_update();
        self.update_gui(); // Force initial update

        let mut instructions_executed = 0u64;
        let mut last_gui_update = std::time::Instant::now();
        const GUI_UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100); // Update every 100ms
        let mut last_port92_value: u8 = self.system_control.value;

        const INSTRUCTION_BATCH_SIZE: u64 = 10000; // Larger batch size for better performance

        tracing::info!("Starting interactive execution loop");
        tracing::warn!(
            "[Emulator] Starting execution... (instructions will be processed in batches)"
        );

        // Progress tracking: detect stuck loops
        let mut last_rip: u64 = u64::MAX;
        let mut stuck_count: u32 = 0;
        let mut stuck_reported = false;
        while instructions_executed < max_instructions {
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
                        tracing::warn!(
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

                    // Detailed EIP trace to track POST progression
                    // Log every batch in the critical PM→POST transition range
                    if (440_000..480_000).contains(&instructions_executed) {
                        let mem = self.memory.ram_slice();
                        let ipl_count = if 0x9FF81 < mem.len() {
                            u16::from_le_bytes([mem[0x9FF80], mem[0x9FF81]])
                        } else { 0 };
                        let ipl0_type = if 0x9FF01 < mem.len() {
                            u16::from_le_bytes([mem[0x9FF00], mem[0x9FF01]])
                        } else { 0 };
                        tracing::warn!(
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
                            } else { 0 };
                            let bp4 = if bp_phys + 5 < mem_peek.len() {
                                u16::from_le_bytes([mem_peek[bp_phys + 4], mem_peek[bp_phys + 5]])
                            } else { 0 };
                            let bp6 = if bp_phys + 7 < mem_peek.len() {
                                u16::from_le_bytes([mem_peek[bp_phys + 6], mem_peek[bp_phys + 7]])
                            } else { 0 };
                            tracing::warn!(
                                "BIOS stuck at RIP={:#x} after {}k instructions, last I/O read: port={:#06x} value={:#x}, CS={:#06x} mode={}, BP={:#06x} AX={:#06x} [BP+2]={:#06x} [BP+4]={:#06x} [BP+6]={:#06x}",
                                current_rip,
                                instructions_executed / 1000,
                                self.devices.last_io_read_port,
                                self.devices.last_io_read_value,
                                self.cpu.get_cs_selector(),
                                self.cpu.get_cpu_mode(),
                                bp, ax, bp2, bp4, bp6,
                            );
                            // Dump IPL table and stack for debugging
                            {
                                let mem = self.memory.ram_slice();
                                let read_u16 = |addr: usize| -> u16 {
                                    if addr + 1 < mem.len() {
                                        u16::from_le_bytes([mem[addr], mem[addr + 1]])
                                    } else { 0 }
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
                                tracing::warn!(
                                    "IPL table @0x9FF00: count={:#x} seq={:#x} bootfirst={:#x} entry0_type={:#x} entry1_type={:#x}",
                                    ipl_count, ipl_seq, ipl_bootfirst, ipl0_type, ipl1_type,
                                );
                                tracing::warn!(
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
                                let full_words: Vec<u16> = (full_stack_start..full_stack_end).step_by(2)
                                    .map(|a| read_u16(a))
                                    .collect();
                                let full_hex: Vec<String> = full_words.iter().map(|w| format!("{:04x}", w)).collect();
                                tracing::warn!(
                                    "Full stack SS:SP={:#06x}:{:#06x} ({} words): {}",
                                    ss_sel, sp, full_words.len(), full_hex.join(" "),
                                );
                                tracing::warn!(
                                    "Stack dump SS:SP={:#06x}:{:#06x} (phys {:#x}): {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x} {:04x}",
                                    ss_sel, sp, stack_addr,
                                    stack_words[0], stack_words[1], stack_words[2], stack_words[3],
                                    stack_words[4], stack_words[5], stack_words[6], stack_words[7],
                                    stack_words[8], stack_words[9], stack_words[10], stack_words[11],
                                    stack_words[12], stack_words[13], stack_words[14], stack_words[15],
                                );
                                // Also dump BDA and IVT for diagnostics
                                let ebda_seg = read_u16(0x040E);
                                let int08_vec = read_u16(0x0020) as u32 | ((read_u16(0x0022) as u32) << 16);
                                let int13_vec = read_u16(0x004C) as u32 | ((read_u16(0x004E) as u32) << 16);
                                let int19_vec = read_u16(0x0064) as u32 | ((read_u16(0x0066) as u32) << 16);
                                let bda_ticks = read_u16(0x046C) as u32 | ((read_u16(0x046E) as u32) << 16);
                                let bda_kbd_head = read_u16(0x041A);
                                tracing::warn!(
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
                            let _ = bios_file.write_all(&e9);
                            let _ = bios_file.flush();
                        } else {
                            let mut out = std::io::stdout();
                            let _ = out.write_all(&e9);
                            let _ = out.flush();
                        }

                        #[cfg(not(feature = "std"))]
                        {
                            let mut out = std::io::stdout();
                            let _ = out.write_all(&e9);
                            let _ = out.flush();
                        }
                    }

                    // Advance virtual time (Bochs-like ticking).
                    // Required so PIT can generate IRQ0 and BIOS can progress past HLT waits.
                    if self.config.ips != 0 {
                        let usec_from_instr = (executed.saturating_mul(1_000_000)) / (self.config.ips as u64);
                        // Always advance at least 10 usec so PIT/RTC timers tick even when
                        // the CPU is halted or executed very few instructions (e.g., executed=1
                        // at IPS=15M gives usec=0, starving timers forever).
                        let usec = usec_from_instr.max(10);
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
                    if instructions_executed < 5 * INSTRUCTION_BATCH_SIZE || instructions_executed % 100_000 < INSTRUCTION_BATCH_SIZE {
                        let pit_c0_count = self.device_manager.pit.counters[0].count;
                        // Read BDA timer tick counter at 0x046C (4 bytes) directly from RAM
                        let bda_ticks = {
                            let (ptr, len) = self.memory.get_raw_memory_ptr();
                            if 0x046C + 4 <= len {
                                unsafe {
                                    let p = ptr.add(0x046C) as *const u32;
                                    *p
                                }
                            } else { 0 }
                        };
                        tracing::warn!("BATCH-DIAG: executed={}, total={}k, RIP={:#x}, PIT_count={}, activity={:?}, BDA_ticks={}",
                            executed, instructions_executed / 1000, self.cpu.rip(), pit_c0_count,
                            self.cpu.activity_state, bda_ticks);
                    }

                    // Periodic interrupt-chain diagnostic (every ~1M instructions)
                    if instructions_executed % 1_000_000 < INSTRUCTION_BATCH_SIZE {
                        let has_int = self.has_interrupt();
                        let if_flag = self.cpu.get_b_if();
                        let rip = self.cpu.rip();
                        let pit_c0 = &self.device_manager.pit.counters[0];
                        tracing::warn!(
                            "IRQ-DIAG: {}M instr, RIP={:#x}, IF={}, has_int={}, PIC_imr={:#04x}, PIC_irr={:#04x}, PIT_c0: mode={:?} init={} count={} enabled={} counting={} output={}",
                            instructions_executed / 1_000_000,
                            rip,
                            if_flag,
                            has_int,
                            self.device_manager.pic.master.imr,
                            self.device_manager.pic.master.irr,
                            self.device_manager.pit.counters[0].mode,
                            pit_c0.initial_count,
                            pit_c0.count,
                            pit_c0.enabled,
                            pit_c0.counting,
                            pit_c0.output,
                        );
                    }

                    // Deliver pending PIC interrupts to the CPU (Bochs-like).
                    {
                        let has_int = self.has_interrupt();
                        let if_flag = self.cpu.get_b_if();
                        if has_int {
                            tracing::warn!("INT-DELIVER: has_int={}, IF={}, activity={:?}, RIP={:#x}",
                                has_int, if_flag, self.cpu.activity_state, self.cpu.rip());
                        }
                    }
                    if self.has_interrupt() && self.cpu.get_b_if() != 0 {
                        let vector = self.iac();
                        tracing::warn!("INT-INJECT: vector={:#04x}, activity_before={:?}",
                            vector, self.cpu.activity_state);

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
                                tracing::warn!("INT-INJECT: OK! activity_after={:?}, RIP={:#x}",
                                    self.cpu.activity_state, self.cpu.rip());
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
                    tracing::warn!("[Emulator] ERROR: {:?}", e);
                    return Err(crate::Error::Cpu(e));
                }
            };

            // Update GUI after CPU execution (outside the match to avoid borrow conflicts)
            // Update more frequently if text is dirty OR periodically (like Bochs timer)
            if should_update_gui {
                self.update_gui();
            }

            // 5. Check if we should exit (e.g., shutdown requested)
            // TODO: Add shutdown flag check
        }

        tracing::warn!(
            "Interactive execution completed: {} instructions",
            instructions_executed
        );

        Ok(instructions_executed)
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
