#![allow(unused_variables)]
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

use alloc::{boxed::Box, format, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::AtomicBool;
#[cfg(feature = "std")]
use core::sync::atomic::Ordering;

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
    /// Enable sync=slowdown clock synchronization.
    /// When true, the emulator sleeps to match wall-clock time during active
    /// (non-HLT) execution with a GUI attached. Matches Bochs `clock: sync=slowdown`.
    /// Default: true (GUI), false (headless). Override with RUSTY_BOX_NOSYNC=1.
    pub sync_slowdown: bool,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            guest_memory_size: 32 * 1024 * 1024,
            host_memory_size: 32 * 1024 * 1024,
            memory_block_size: 128 * 1024,
            ips: 4_000_000,
            pci_enabled: true,
            cpu_params: BxParams::default(),
            sync_slowdown: false,
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
    /// Extend the borrow of memory owned by this Emulator to match lifetime 'a.
    ///
    /// # Safety
    /// Sound because:
    /// 1. Memory is owned by Emulator which outlives every cpu_loop call
    /// 2. We hold &mut self, preventing concurrent access
    /// 3. CPU does not retain the reference beyond the call
    /// 4. No other code path accesses self.memory during CPU execution
    #[inline]
    unsafe fn borrow_memory_for_cpu(&mut self) -> &'a mut BxMemC<'a> {
        core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(&mut self.memory)
    }

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
        self.devices.init(&mut self.memory)?;

        // Initialize device manager (actual hardware + I/O handler registration)
        self.device_manager
            .init(&mut self.devices, &mut self.memory)?;
        // Initialize PCI bridge DRAM row boundaries from RAM size,
        // and wire PCI bridge to memory_type for immediate PAM updates.
        #[cfg(feature = "bx_support_pci")]
        {
            let ramsize_mb = (self.config.guest_memory_size / (1024 * 1024)) as u32;
            self.device_manager.pci_bridge.init_dram(ramsize_mb);
            tracing::debug!("PCI bridge DRAM initialized for {}MB", ramsize_mb);
        }
        tracing::debug!("Devices initialized");


        // Wire DMA→memory for physical DMA transfers
        let (ram_base, ram_len) = self.memory.get_ram_base_ptr();
        self.device_manager.dma.set_memory_ptrs(
            ram_base,
            ram_len,
        );


        // Register PCI IDE BM-DMA timers (Bochs pci_ide.cc:77-78)
        {
            // Channel 0 timer
            match self.pc_system.register_timer(
                crate::pc_system::TimerOwner::PciIdeCh0,
                0,
                false,
                false,
                "PIIX IDE ch0",
            ) {
                Ok(handle) => {
                    self.device_manager.pci_ide.bmdma[0].timer_index = Some(handle);
                    tracing::debug!("PCI IDE ch0 timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register PCI IDE ch0 timer: {}", e);
                }
            }
            // Channel 1 timer
            match self.pc_system.register_timer(
                crate::pc_system::TimerOwner::PciIdeCh1,
                0,
                false,
                false,
                "PIIX IDE ch1",
            ) {
                Ok(handle) => {
                    self.device_manager.pci_ide.bmdma[1].timer_index = Some(handle);
                    tracing::debug!("PCI IDE ch1 timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register PCI IDE ch1 timer: {}", e);
                }
            }
        }

        // PIC→IOAPIC forwarding is now handled at call sites: PIC's raise/lower_irq
        // return forwarding info, and the caller (DeviceManager::tick, etc.) forwards
        // to IOAPIC. No stored pointers needed.

        // IOAPIC→PIC (ExtINT) and IOAPIC→LAPIC (interrupt delivery) are now passed
        // as parameters to service_ioapic/set_irq_level/write_aligned.
        // The MMIO callback path uses fallback stubs (no PIC/LAPIC available).

        // Register LAPIC timer with pc_system (matches Bochs apic.cc:190-191)
        // Timer is registered inactive; activated when LAPIC timer ICR is written.
        #[cfg(feature = "bx_support_apic")]
        {
            let lapic_ptr = self.cpu.lapic_ptr_mut();
            let timer_handle = self.pc_system.register_timer(
                crate::pc_system::TimerOwner::Lapic,
                0,     // period=0 (inactive)
                false, // continuous=false (one-shot, re-armed by periodic())
                false, // active=false
                "lapic",
            );
            match timer_handle {
                Ok(handle) => {
                    // SAFETY: lapic_ptr set during CPU init; single-threaded access
                    let lapic = unsafe { &mut *lapic_ptr };
                    lapic.timer_handle = Some(handle);
                    tracing::debug!("LAPIC timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register LAPIC timer: {}", e);
                }
            }
        }

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
        self.devices.init(&mut self.memory)?;

        // Initialize device manager (actual hardware + I/O handler registration)
        self.device_manager
            .init(&mut self.devices, &mut self.memory)?;

        // Initialize PCI bridge DRAM row boundaries from RAM size.
        #[cfg(feature = "bx_support_pci")]
        {
            let ramsize_mb = (self.config.guest_memory_size / (1024 * 1024)) as u32;
            self.device_manager.pci_bridge.init_dram(ramsize_mb);
            tracing::debug!("PCI bridge DRAM initialized for {}MB", ramsize_mb);
        }
        tracing::info!("Device initialization complete");


        // Wire DMA→memory for physical DMA transfers
        let (ram_base, ram_len) = self.memory.get_ram_base_ptr();
        self.device_manager.dma.set_memory_ptrs(
            ram_base,
            ram_len,
        );


        // Register PCI IDE BM-DMA timers (Bochs pci_ide.cc:77-78)
        {
            // Channel 0 timer
            match self.pc_system.register_timer(
                crate::pc_system::TimerOwner::PciIdeCh0,
                0,
                false,
                false,
                "PIIX IDE ch0",
            ) {
                Ok(handle) => {
                    self.device_manager.pci_ide.bmdma[0].timer_index = Some(handle);
                    tracing::debug!("PCI IDE ch0 timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register PCI IDE ch0 timer: {}", e);
                }
            }
            // Channel 1 timer
            match self.pc_system.register_timer(
                crate::pc_system::TimerOwner::PciIdeCh1,
                0,
                false,
                false,
                "PIIX IDE ch1",
            ) {
                Ok(handle) => {
                    self.device_manager.pci_ide.bmdma[1].timer_index = Some(handle);
                    tracing::debug!("PCI IDE ch1 timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register PCI IDE ch1 timer: {}", e);
                }
            }
        }

        // PIC→IOAPIC, IOAPIC→PIC, IOAPIC→LAPIC: pointer wiring removed.
        // Forwarding is now done via parameters at call sites.

        // Register LAPIC timer with pc_system (matches Bochs apic.cc:190-191)
        // Timer is registered inactive; activated when LAPIC timer ICR is written.
        #[cfg(feature = "bx_support_apic")]
        {
            let lapic_ptr = self.cpu.lapic_ptr_mut();
            let timer_handle = self.pc_system.register_timer(
                crate::pc_system::TimerOwner::Lapic,
                0,     // period=0 (inactive)
                false, // continuous=false (one-shot, re-armed by periodic())
                false, // active=false
                "lapic",
            );
            match timer_handle {
                Ok(handle) => {
                    // SAFETY: lapic_ptr set during CPU init; single-threaded access
                    let lapic = unsafe { &mut *lapic_ptr };
                    lapic.timer_handle = Some(handle);
                    tracing::debug!("LAPIC timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register LAPIC timer: {}", e);
                }
            }
        }

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

    /// Get mutable reference to CPU for instrumentation setup.
    pub fn cpu_mut(&mut self) -> &mut crate::cpu::cpu::BxCpuC<'a, I> {
        &mut self.cpu
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
                    cursor_y,
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

        // Reset system control port and keyboard reset flag
        self.device_manager.port92 = SystemControlPort::new();
        self.device_manager.keyboard.reset_requested = false;

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



        // Initialize PIT icount sync so PIT counter reads advance with CPU time.
        // This is critical for kernel PIT-polling calibration loops (e.g., Alpine Linux).
        let ips = self.config.ips as u64;
        if ips > 0 {
            self.device_manager.pit.init_icount_sync(self.cpu.icount, ips);
        }

        // Initialize VGA icount-based timing for retrace computation.
        {
            let ips = self.config.ips as u64;
            self.device_manager.vga.set_icount_sync(ips);
        }


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
        self.pc_system.set_enable_a20(self.device_manager.port92.a20_gate);
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        // Bochs pc_system.cc MemoryMappingChanged() calls BX_CPU(0)->TLB_flush()
        // after A20 changes, since A20 masking affects physical address translation.
        self.cpu.tlb_flush();
    }

    /// Process a Port 92h write
    ///
    /// This updates the A20 state and checks for reset requests.
    /// Returns true if a reset was requested.
    pub fn write_port_92h(&mut self, value: u8) -> bool {
        let a20_changed = self.device_manager.port92.write(value);

        if a20_changed {
            self.sync_a20_state();
        }

        self.device_manager.port92.reset_request
    }

    /// Read Port 92h value
    pub fn read_port_92h(&self) -> u8 {
        self.device_manager.port92.read()
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

    /// Attach a CD-ROM ISO image to a channel/drive (requires std feature)
    #[cfg(feature = "std")]
    pub fn attach_cdrom(
        &mut self,
        channel: usize,
        drive: usize,
        path: &str,
    ) -> std::io::Result<()> {
        self.device_manager
            .harddrv
            .attach_cdrom_image(channel, drive, path)
    }

    /// Check if an interrupt is pending (PIC or LAPIC)
    pub fn has_interrupt(&self) -> bool {
        // Legacy PIC path
        if self.device_manager.has_interrupt() {
            return true;
        }
        // APIC path: check LAPIC for pending interrupts
        #[cfg(feature = "bx_support_apic")]
        if self.cpu.lapic_has_intr() {
            return true;
        }
        false
    }

    /// Acknowledge interrupt and get vector
    pub fn iac(&mut self) -> u8 {
        self.device_manager.iac()
    }

    /// Simulate time passing (for timer-based devices)
    pub fn tick_devices(&mut self, usec: u64) {
        self.device_manager.tick(usec, self.cpu.icount);
        // Process deferred ATAPI seek completion (Bochs seek_timer pattern).
        // In Bochs, start_seek() activates a timer that fires after a seek
        // delay and calls ready_to_send_atapi(). We process it here during
        // the next tick, providing the minimum 1-tick delay that separates
        // the PACKET CDB write from the data-ready interrupt.
        #[cfg(feature = "bx_support_pci")]
        {
            let dm = &mut self.device_manager;
            for ch in 0..2 {
                if dm.harddrv.seek_complete_pending[ch] {
                    dm.harddrv.seek_complete_pending[ch] = false;
                    let crate::iodev::devices::DeviceManager { ref mut harddrv, ref mut pic, ref mut pci_ide, .. } = *dm;
                    harddrv.ready_to_send_atapi(ch, pic, pci_ide);
                }
            }
        }
        #[cfg(not(feature = "bx_support_pci"))]
        {
            for ch in 0..2 {
                if self.device_manager.harddrv.seek_complete_pending[ch] {
                    self.device_manager.harddrv.seek_complete_pending[ch] = false;
                    let crate::iodev::devices::DeviceManager { ref mut harddrv, ref mut pic, .. } = self.device_manager;
                    let mut stub_pci_ide = crate::iodev::pci_ide::BxPciIde::new();
                    harddrv.ready_to_send_atapi(ch, pic, &mut stub_pci_ide);
                }
            }
        }
        // Process any deferred PCI port re-registrations and PAM changes
        #[cfg(feature = "bx_support_pci")]
        self.device_manager
            .process_pci_deferred::<I>(&mut self.devices, &mut self.memory);
    }

    /// Dispatch timer fires accumulated by `pc_system.tickn()`.
    ///
    /// `countdown_event` records fired timer owners instead of calling fn ptrs.
    /// This method drains them and performs the device-specific action.
    fn dispatch_timer_fires(&mut self) {
        let (owners, count) = self.pc_system.take_fired_timers();
        for &owner in &owners[..count] {
            match owner {
                crate::pc_system::TimerOwner::NullTimer => {}
                crate::pc_system::TimerOwner::PciIdeCh0 => {
                    self.device_manager.pci_ide.timer(0);
                }
                crate::pc_system::TimerOwner::PciIdeCh1 => {
                    self.device_manager.pci_ide.timer(1);
                }
                crate::pc_system::TimerOwner::Lapic => {
                    #[cfg(feature = "bx_support_apic")]
                    {
                        // SAFETY: lapic_ptr set during CPU init; single-threaded access
                        let lapic = unsafe { &mut *self.cpu.lapic_ptr_mut() };
                        lapic.timer_fired = true;
                    }
                }
            }
        }
    }

    /// Synchronize device event flags to CPU event fields.
    ///
    /// PIC, LAPIC, and pc_system set boolean flags when they need to
    /// signal the CPU. This method reads those flags, applies the
    /// corresponding bits to `cpu.pending_event` / `cpu.async_event`,
    /// and clears the flags.
    fn sync_event_flags(&mut self) {
        // PIC: BX_RAISE_INTR
        if self.device_manager.pic.irq_pending {
            self.cpu.pending_event |= BxCpuC::<I>::BX_EVENT_PENDING_INTR;
            self.cpu.async_event = 1;
            self.device_manager.pic.irq_pending = false;
        }
        // PIC: BX_CLEAR_INTR
        if self.device_manager.pic.irq_cleared {
            self.cpu.pending_event &= !BxCpuC::<I>::BX_EVENT_PENDING_INTR;
            self.device_manager.pic.irq_cleared = false;
        }
        // LAPIC: BX_EVENT_PENDING_LAPIC_INTR
        {
            let lapic = unsafe { &mut *self.cpu.lapic_ptr_mut() };
            if lapic.intr_pending {
                self.cpu.pending_event |= BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR;
                self.cpu.async_event = 1;
                lapic.intr_pending = false;
            }
        }
        // pc_system: raise_intr
        if self.pc_system.intr_raised {
            self.cpu.pending_event |= BxCpuC::<I>::BX_EVENT_PENDING_INTR;
            self.cpu.async_event = 1;
            self.pc_system.intr_raised = false;
        }
        // pc_system: clear_intr
        if self.pc_system.intr_cleared {
            self.cpu.pending_event &= !BxCpuC::<I>::BX_EVENT_PENDING_INTR;
            self.pc_system.intr_cleared = false;
        }
        // pc_system: async_event (HRQ/timer)
        if self.pc_system.async_event_pending {
            self.cpu.async_event = 1;
            self.pc_system.async_event_pending = false;
        }
    }

    /// Configure CMOS memory size from total RAM bytes.
    /// This is the preferred method — it matches Bochs devices.cc:320-345.
    pub fn configure_memory_in_cmos_from_config(&mut self) {
        self.device_manager
            .cmos
            .set_memory_size_from_bytes(self.config.guest_memory_size as u64);
    }

    /// Configure CMOS memory size (legacy interface)
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

    /// Configure floppy drives in CMOS
    ///
    /// drive_type: 0=none, 1=360K, 2=1.2M, 3=720K, 4=1.44M, 5=2.88M
    /// Matches Bochs bochsrc `floppya`/`floppyb` type configuration.
    pub fn configure_floppy_in_cmos(&mut self, drive_a_type: u8, drive_b_type: u8) {
        self.device_manager
            .cmos
            .set_floppy_config(drive_a_type, drive_b_type);
    }

    /// Configure boot sequence in CMOS
    ///
    /// Boot device codes: 0=none, 1=floppy, 2=hard disk, 3=cdrom
    pub fn configure_boot_sequence(&mut self, first: u8, second: u8, third: u8) {
        self.device_manager
            .cmos
            .set_boot_sequence(first, second, third);
    }

    /// Set up direct Linux kernel boot, bypassing BIOS entirely.
    ///
    /// Loads a bzImage kernel and optional initramfs into memory, sets up
    /// the Linux boot protocol "zero page" (boot_params), configures CPU
    /// for 32-bit protected mode, and points EIP at the kernel entry.
    ///
    /// This is equivalent to QEMU's `-kernel` / `-initrd` / `-append` options.
    ///
    /// # Arguments
    /// * `bzimage` - Raw bzImage kernel file contents
    /// * `initramfs` - Optional initramfs/initrd file contents
    /// * `cmdline` - Kernel command line string
    ///
    /// # Memory Layout
    /// * 0x1000: GDT (4 entries)
    /// * 0x10000: boot_params (4096 bytes)
    /// * 0x11000: command line (up to 2048 bytes)
    /// * 0x100000: protected-mode kernel
    /// * High memory: initramfs (if provided)
    pub fn setup_direct_linux_boot(
        &mut self,
        bzimage: &[u8],
        initramfs: Option<&[u8]>,
        cmdline: &str,
    ) -> Result<()> {
        // Validate bzImage header
        if bzimage.len() < 0x264 {
            return Err(crate::Error::Cpu(crate::cpu::CpuError::UnimplementedOpcode {
                opcode: "bzImage too small".into(),
            }));
        }
        if bzimage[0x1FE] != 0x55 || bzimage[0x1FF] != 0xAA {
            return Err(crate::Error::Cpu(crate::cpu::CpuError::UnimplementedOpcode {
                opcode: "Invalid bzImage boot signature".into(),
            }));
        }
        let header_magic = u32::from_le_bytes([
            bzimage[0x202], bzimage[0x203], bzimage[0x204], bzimage[0x205],
        ]);
        if header_magic != 0x53726448 {
            // "HdrS"
            return Err(crate::Error::Cpu(crate::cpu::CpuError::UnimplementedOpcode {
                opcode: "Invalid bzImage header magic".into(),
            }));
        }
        let boot_version = u16::from_le_bytes([bzimage[0x206], bzimage[0x207]]);
        if boot_version < 0x0204 {
            return Err(crate::Error::Cpu(crate::cpu::CpuError::UnimplementedOpcode {
                opcode: alloc::format!("Boot protocol {}.{} too old (need >= 2.04)",
                    boot_version >> 8, boot_version & 0xFF),
            }));
        }

        // Parse bzImage header
        let setup_sects = if bzimage[0x1F1] == 0 { 4 } else { bzimage[0x1F1] as usize };
        let setup_size = (setup_sects + 1) * 512;
        let pm_kernel = &bzimage[setup_size..];

        let code32_start = u32::from_le_bytes([
            bzimage[0x214], bzimage[0x215], bzimage[0x216], bzimage[0x217],
        ]);

        // Read pref_address (protocol >= 2.10) and init_size for boot_params placement
        let pref_address = if boot_version >= 0x020A {
            u64::from_le_bytes([
                bzimage[0x258], bzimage[0x259], bzimage[0x25A], bzimage[0x25B],
                bzimage[0x25C], bzimage[0x25D], bzimage[0x25E], bzimage[0x25F],
            ])
        } else {
            0 // Old kernels: use legacy boot_params address
        };
        let init_size = u32::from_le_bytes([
            bzimage[0x260], bzimage[0x261], bzimage[0x262], bzimage[0x263],
        ]) as u64;

        tracing::info!(
            "bzImage: protocol {}.{}, setup={}B, kernel={}B, entry={:#x}, pref={:#x}, init_size={:#x}",
            boot_version >> 8, boot_version & 0xFF,
            setup_size, pm_kernel.len(), code32_start, pref_address, init_size
        );

        // =====================================================================
        // Write GDT at 0x1000
        // =====================================================================
        const GDT_ADDR: u64 = 0x1000;
        let gdt: [u64; 4] = [
            0x0000000000000000, // Entry 0: null
            0x0000000000000000, // Entry 1: null (reserved)
            0x00CF9A000000FFFF, // Entry 2 (sel 0x10): 32-bit code, base=0, limit=4GB
            0x00CF92000000FFFF, // Entry 3 (sel 0x18): 32-bit data, base=0, limit=4GB
        ];
        let mut gdt_bytes = [0u8; 32];
        for (i, &entry) in gdt.iter().enumerate() {
            gdt_bytes[i * 8..(i + 1) * 8].copy_from_slice(&entry.to_le_bytes());
        }
        self.memory.load_RAM(&gdt_bytes, GDT_ADDR)?;

        // =====================================================================
        // Write boot_params (zero page)
        // =====================================================================
        // Place boot_params at 0x10000 (standard location, matches QEMU).
        // The decompressor relocates itself to ~pref_address+init_size area,
        // which would overwrite boot_params if placed there. Low addresses
        // (< 0x100000) are safe — the compressed kernel loads at 0x100000+
        // and the decompressor never touches conventional memory.
        // The kernel's early page fault handler (__early_make_pgtable) creates
        // identity mappings on demand for any unmapped physical address.
        let boot_params_addr: u64 = 0x10000;
        let cmdline_addr: u64 = 0x20000;
        tracing::info!(
            "boot_params at {:#x}, cmdline at {:#x} (pref={:#x}, init_size={:#x})",
            boot_params_addr, cmdline_addr, pref_address, init_size
        );
        let mut boot_params = [0u8; 4096];

        // Copy setup header from bzImage (offsets 0x1F1 to 0x268)
        let hdr_start = 0x1F1;
        let hdr_end = core::cmp::min(0x268, bzimage.len());
        boot_params[hdr_start..hdr_end].copy_from_slice(&bzimage[hdr_start..hdr_end]);

        // type_of_loader = 0xFF (unknown bootloader)
        boot_params[0x210] = 0xFF;

        // loadflags: set LOADED_HIGH (bit 0), keep CAN_USE_HEAP (bit 7)
        boot_params[0x211] |= 0x01; // LOADED_HIGH

        // cmd_line_ptr = physical address of command line
        boot_params[0x228..0x22C]
            .copy_from_slice(&(cmdline_addr as u32).to_le_bytes());

        // heap_end_ptr: relative to setup header start (unused for direct boot)
        boot_params[0x224..0x226].copy_from_slice(&0xFE00u16.to_le_bytes());

        // screen_info (struct screen_info at boot_params offset 0x000):
        //   0x00: orig_x           (cursor column)
        //   0x01: orig_y           (cursor row)
        //   0x02: ext_mem_k        (u16, extended memory in KB)
        //   0x04: orig_video_page  (u16, active display page)
        //   0x06: orig_video_mode  (video mode number)
        //   0x07: orig_video_cols  (text columns)
        //   0x0a: orig_video_ega_bx (u16, EGA/VGA info)
        //   0x0e: orig_video_lines (text rows)
        //   0x0f: orig_video_isVGA (0=no, 1=VGA, 0x22=EGA/VGA)
        //   0x10: orig_video_points (u16, font height in pixels)
        boot_params[0x00] = 0;    // orig_x
        boot_params[0x01] = 0;    // orig_y
        boot_params[0x06] = 0x03; // orig_video_mode = 3 (80x25 color text)
        boot_params[0x07] = 80;   // orig_video_cols
        boot_params[0x0E] = 25;   // orig_video_lines
        boot_params[0x0F] = 0x01; // orig_video_isVGA = 1
        boot_params[0x10..0x12].copy_from_slice(&16u16.to_le_bytes()); // orig_video_points = 16

        // vid_mode at 0x1FA (in setup header, but also used by kernel)
        boot_params[0x1FA..0x1FC].copy_from_slice(&0xFFFFu16.to_le_bytes()); // NORMAL_VGA

        // acpi_rsdp_addr at offset 0x070 (boot protocol 2.14+)
        // Tells kernel where to find RSDP without scanning BIOS area
        boot_params[0x070..0x078].copy_from_slice(&0x40000u64.to_le_bytes());

        // =====================================================================
        // Set up initramfs if provided
        // =====================================================================
        let kernel_end = code32_start as u64 + pm_kernel.len() as u64;

        // initrd_addr_max from boot protocol (offset 0x22C) - max address kernel can handle
        let initrd_addr_max = if boot_version >= 0x0203 {
            u32::from_le_bytes([
                bzimage[0x22C], bzimage[0x22D], bzimage[0x22E], bzimage[0x22F],
            ]) as u64
        } else {
            0x37FFFFFF // Default for old protocols
        };

        if let Some(initrd_data) = initramfs {
            let ram_top = self.config.guest_memory_size as u64;
            let max_addr = core::cmp::min(ram_top, initrd_addr_max + 1);

            // Place initramfs at top of allowed memory (QEMU strategy)
            // This prevents the kernel decompressor from overwriting the initramfs
            let initrd_load_addr = (max_addr - initrd_data.len() as u64) & !0xFFF;

            tracing::info!(
                "BOOT LAYOUT: kernel={} bytes at {:#x}..{:#x}, init_size={:#x}, initrd={} bytes at {:#x}..{:#x}, RAM top={:#x}, initrd_addr_max={:#x}",
                pm_kernel.len(), code32_start, kernel_end,
                init_size,
                initrd_data.len(), initrd_load_addr, initrd_load_addr + initrd_data.len() as u64,
                ram_top, initrd_addr_max
            );
            self.memory.load_RAM(initrd_data, initrd_load_addr)?;

            // ramdisk_image = physical address
            boot_params[0x218..0x21C]
                .copy_from_slice(&(initrd_load_addr as u32).to_le_bytes());
            // ramdisk_size
            boot_params[0x21C..0x220]
                .copy_from_slice(&(initrd_data.len() as u32).to_le_bytes());
        }

        // =====================================================================
        // E820 memory map
        // =====================================================================
        let ram_size = self.config.guest_memory_size as u64;
        let e820_base = 0x2D0; // offset in boot_params
        let mut e820_idx = 0;

        // Helper to write an e820 entry (20 bytes each)
        let mut write_e820 = |bp: &mut [u8], addr: u64, size: u64, etype: u32| {
            let off = e820_base + e820_idx * 20;
            bp[off..off + 8].copy_from_slice(&addr.to_le_bytes());
            bp[off + 8..off + 16].copy_from_slice(&size.to_le_bytes());
            bp[off + 16..off + 20].copy_from_slice(&etype.to_le_bytes());
            e820_idx += 1;
        };

        // Entry 1: 0 - 0x9FC00 (conventional memory, ~639KB)
        write_e820(&mut boot_params, 0, 0x9FC00, 1);
        // Entry 2: 0x9FC00 - 0xA0000 (reserved, EBDA)
        write_e820(&mut boot_params, 0x9FC00, 0x400, 2);
        // Entry 3: 0xF0000 - 0x100000 (reserved, BIOS)
        write_e820(&mut boot_params, 0xF0000, 0x10000, 2);
        // Entry 4: 0x100000 - top of RAM (usable extended memory)
        if ram_size > 0x100000 {
            write_e820(&mut boot_params, 0x100000, ram_size - 0x100000, 1);
        }

        // e820_entries count at offset 0x1E8
        boot_params[0x1E8] = e820_idx as u8;

        // Write boot_params to memory
        self.memory.load_RAM(&boot_params, boot_params_addr)?;

        // =====================================================================
        // Write command line
        // =====================================================================
        let cmdline_bytes = cmdline.as_bytes();
        let cmdline_len = core::cmp::min(cmdline_bytes.len(), 2047);
        let mut cmdline_buf = alloc::vec![0u8; cmdline_len + 1]; // null-terminated
        cmdline_buf[..cmdline_len].copy_from_slice(&cmdline_bytes[..cmdline_len]);
        self.memory.load_RAM(&cmdline_buf, cmdline_addr)?;
        tracing::info!("Command line: {}", cmdline);

        // =====================================================================
        // Create minimal ACPI tables (RSDP → XSDT → MADT)
        // Without these, the kernel can't find the APIC/IOAPIC and falls back
        // to a mode where no interrupt delivery works, stalling boot.
        // Layout: RSDP at 0xE0000, XSDT at 0xE0100, MADT at 0xE0200
        // =====================================================================
        {
            // Place in low memory (safe area: 0x40000-0x4FFFF unused by kernel/bootloader)
            const RSDP_ADDR: u64 = 0x40000;
            const XSDT_ADDR: u64 = 0x40100;
            const MADT_ADDR: u64 = 0x40200;

            // --- MADT (Multiple APIC Description Table) ---
            // Header: 44 bytes
            // + Local APIC entry: 8 bytes (type 0)
            // + I/O APIC entry: 12 bytes (type 1)
            // + Interrupt Source Override: 10 bytes (type 2) — IRQ0 → GSI2
            let madt_len: u32 = 44 + 8 + 12 + 10;
            let mut madt = alloc::vec![0u8; madt_len as usize];
            // Signature "APIC"
            madt[0..4].copy_from_slice(b"APIC");
            // Length
            madt[4..8].copy_from_slice(&madt_len.to_le_bytes());
            // Revision
            madt[8] = 3; // ACPI 2.0 revision
            // Checksum (byte 9) — filled later
            // OEM ID
            madt[10..16].copy_from_slice(b"RUSTYB");
            // OEM Table ID
            madt[16..24].copy_from_slice(b"BXMADT  ");
            // OEM Revision
            madt[24..28].copy_from_slice(&1u32.to_le_bytes());
            // Creator ID
            madt[28..32].copy_from_slice(b"RBOX");
            // Creator Revision
            madt[32..36].copy_from_slice(&1u32.to_le_bytes());
            // Local APIC Address (offset 36)
            madt[36..40].copy_from_slice(&0xFEE00000u32.to_le_bytes());
            // Flags (offset 40): bit 0 = PCAT_COMPAT (dual 8259 present)
            madt[40..44].copy_from_slice(&1u32.to_le_bytes());

            // Entry: Local APIC (type 0, len 8)
            let e = 44;
            madt[e] = 0; // type
            madt[e + 1] = 8; // length
            madt[e + 2] = 0; // ACPI Processor ID
            madt[e + 3] = 0; // APIC ID
            madt[e + 4..e + 8].copy_from_slice(&1u32.to_le_bytes()); // flags: enabled

            // Entry: I/O APIC (type 1, len 12)
            let e = 44 + 8;
            madt[e] = 1; // type
            madt[e + 1] = 12; // length
            madt[e + 2] = 1; // I/O APIC ID
            madt[e + 3] = 0; // reserved
            madt[e + 4..e + 8].copy_from_slice(&0xFEC00000u32.to_le_bytes()); // address
            madt[e + 8..e + 12].copy_from_slice(&0u32.to_le_bytes()); // GSI base

            // Entry: Interrupt Source Override (type 2, len 10) — IRQ0 → GSI 2
            let e = 44 + 8 + 12;
            madt[e] = 2; // type
            madt[e + 1] = 10; // length
            madt[e + 2] = 0; // bus (ISA)
            madt[e + 3] = 0; // source (IRQ0)
            madt[e + 4..e + 8].copy_from_slice(&2u32.to_le_bytes()); // GSI 2
            madt[e + 8..e + 10].copy_from_slice(&0u16.to_le_bytes()); // flags (conforming)

            // Checksum
            let sum: u8 = madt.iter().fold(0u8, |a, &b| a.wrapping_add(b));
            madt[9] = 0u8.wrapping_sub(sum);
            self.memory.load_RAM(&madt, MADT_ADDR)?;

            // --- XSDT (Extended System Description Table) ---
            // Header: 36 bytes + 1 pointer (8 bytes) = 44 bytes
            let xsdt_len: u32 = 36 + 8;
            let mut xsdt = alloc::vec![0u8; xsdt_len as usize];
            xsdt[0..4].copy_from_slice(b"XSDT");
            xsdt[4..8].copy_from_slice(&xsdt_len.to_le_bytes());
            xsdt[8] = 1; // revision
            xsdt[10..16].copy_from_slice(b"RUSTYB");
            xsdt[16..24].copy_from_slice(b"BXXSDT  ");
            xsdt[24..28].copy_from_slice(&1u32.to_le_bytes());
            xsdt[28..32].copy_from_slice(b"RBOX");
            xsdt[32..36].copy_from_slice(&1u32.to_le_bytes());
            // Pointer to MADT (64-bit)
            xsdt[36..44].copy_from_slice(&MADT_ADDR.to_le_bytes());
            let sum: u8 = xsdt.iter().fold(0u8, |a, &b| a.wrapping_add(b));
            xsdt[9] = 0u8.wrapping_sub(sum);
            self.memory.load_RAM(&xsdt, XSDT_ADDR)?;

            // --- RSDP (Root System Description Pointer) ---
            // RSDP v2.0 = 36 bytes
            let mut rsdp = [0u8; 36];
            rsdp[0..8].copy_from_slice(b"RSD PTR "); // signature
            // checksum (byte 8) — filled later
            rsdp[9..15].copy_from_slice(b"RUSTYB"); // OEM ID
            rsdp[15] = 2; // revision (2 = ACPI 2.0+)
            // RSDT address (offset 16) — point to XSDT address as 32-bit for v1 compat
            rsdp[16..20].copy_from_slice(&(XSDT_ADDR as u32).to_le_bytes());
            // Length (offset 20) — v2.0 extended length
            rsdp[20..24].copy_from_slice(&36u32.to_le_bytes());
            // XSDT address (offset 24) — 64-bit
            rsdp[24..32].copy_from_slice(&XSDT_ADDR.to_le_bytes());
            // Extended checksum (byte 32) — filled later
            // v1 checksum covers bytes 0-19
            let v1_sum: u8 = rsdp[0..20].iter().fold(0u8, |a, &b| a.wrapping_add(b));
            rsdp[8] = 0u8.wrapping_sub(v1_sum);
            // v2 extended checksum covers bytes 0-35
            let v2_sum: u8 = rsdp.iter().fold(0u8, |a, &b| a.wrapping_add(b));
            rsdp[32] = 0u8.wrapping_sub(v2_sum);
            self.memory.load_RAM(&rsdp, RSDP_ADDR)?;

            tracing::info!(
                "ACPI tables: RSDP at {:#x}, XSDT at {:#x}, MADT at {:#x} ({}B)",
                RSDP_ADDR, XSDT_ADDR, MADT_ADDR, madt_len
            );
        }

        // =====================================================================
        // Initialize PIC and PIT (normally done by BIOS POST)
        // Direct boot skips BIOS, so we must set up the interrupt controllers
        // manually. The kernel needs timer interrupts (IRQ0) for calibration
        // and early init functions that call udelay()/mdelay().
        // =====================================================================
        {
            // Initialize master PIC: ICW1-ICW4
            // ICW1: edge-triggered, cascade, ICW4 needed
            self.device_manager.pic.write(0x20, 0x11, 1);
            // ICW2: master vectors 0x20-0x27 (Linux kernel expects IRQ0=0x20)
            self.device_manager.pic.write(0x21, 0x20, 1);
            // ICW3: slave on IRQ2
            self.device_manager.pic.write(0x21, 0x04, 1);
            // ICW4: 8086 mode, normal EOI
            self.device_manager.pic.write(0x21, 0x01, 1);
            // OCW1: mask all master IRQs — kernel will unmask what it needs
            self.device_manager.pic.write(0x21, 0xFF, 1);

            // Initialize slave PIC: ICW1-ICW4
            self.device_manager.pic.write(0xA0, 0x11, 1);
            // ICW2: slave vectors 0x28-0x2F (Linux kernel expects IRQ8=0x28)
            self.device_manager.pic.write(0xA1, 0x28, 1);
            // ICW3: cascade identity = 2
            self.device_manager.pic.write(0xA1, 0x02, 1);
            // ICW4: 8086 mode
            self.device_manager.pic.write(0xA1, 0x01, 1);
            // OCW1: mask all slave IRQs
            self.device_manager.pic.write(0xA1, 0xFF, 1);

            // Do NOT program PIT — kernel will set up its own timer via time_init().
            // quick_pit_calibrate() programs PIT C2 via port 0x43/0x42 directly.
            tracing::info!("Direct boot: PIC initialized (master=0x20, slave=0x28), all IRQs masked");
        }

        // =====================================================================
        // Load protected-mode kernel at code32_start
        // =====================================================================
        tracing::info!(
            "Loading kernel ({} bytes) at {:#x}",
            pm_kernel.len(), code32_start
        );
        self.memory.load_RAM(pm_kernel, code32_start as u64)?;

        // =====================================================================
        // Configure CPU for protected mode
        // =====================================================================
        self.cpu.setup_for_direct_boot(GDT_ADDR);

        // Set entry point and registers
        self.cpu.set_rip(code32_start as u64);
        self.cpu.set_rsp(0x20000); // Temporary stack (kernel sets its own early)
        self.cpu.set_rsi(boot_params_addr); // ESI = pointer to boot_params

        tracing::info!(
            "Direct boot ready: EIP={:#x}, ESI={:#x}, ESP={:#x}",
            code32_start, boot_params_addr, 0x20000u32
        );

        Ok(())
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
        'a: 'static, // Required for borrow_memory_for_cpu safety
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
        let mut slowdown_start = std::time::Instant::now();
        let mut slowdown_ticks_base = self.pc_system.time_ticks();
        let mut last_gui_update = std::time::Instant::now();
        let mut last_ips_update = std::time::Instant::now();
        let mut last_ips_instructions = self.cpu.icount; // Bochs-compatible: track icount for IPS
        // MIPS terminal log: separate tracker fired every 5M instructions.
        // At 20 MIPS (active) fires every 250ms; at 40K IPS (idle) fires every ~125s.
        // This prevents flooding the terminal with "0.04 MIPS" lines during HLT idle.
        let mut last_mips_log_update = std::time::Instant::now();
        let mut last_mips_log_instructions = 0u64;
        // Bochs VGA timer fires every ~40ms (25 fps). Use same interval for display parity.
        const GUI_UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(40);
        const IPS_SHOW_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        const MIPS_LOG_INTERVAL: u64 = 50_000_000;
        let mut last_port92_value: u8 = self.device_manager.port92.value;

        const INSTRUCTION_BATCH_SIZE: u64 = 100_000;
        const PROGRESS_LOG_INTERVAL: u64 = 10_000_000;
        let mut next_progress_log: i64 = PROGRESS_LOG_INTERVAL as i64;

        tracing::info!("Starting interactive execution loop");
        tracing::debug!(
            "[Emulator] Starting execution... (instructions will be processed in batches)"
        );

        #[cfg(debug_assertions)]
        let mut last_rip: u64 = u64::MAX;
        #[cfg(debug_assertions)]
        let mut stuck_count: u32 = 0;
        #[cfg(debug_assertions)]
        let mut stuck_reported = false;
        // Counter for consecutive HLT+IF=0 zero-batches (transient recovery)
        let mut hlt_if0_count: u32 = 0;
        while instructions_executed < max_instructions && !self.stop_flag.load(Ordering::Relaxed) {
            // 1. Handle GUI events (keyboard input) - do this first to avoid borrow conflicts

            let mut scancodes_to_send = Vec::new();
            let mut serial_input = Vec::new();
            if let Some(ref mut gui) = self.gui {
                gui.handle_events();
                scancodes_to_send = gui.get_pending_scancodes();
                serial_input = gui.get_pending_serial_input();
            }

            // Send scancodes to keyboard device
            for scancode in scancodes_to_send {
                self.device_manager.keyboard.send_scancode(scancode);
            }

            // Send serial input to COM1 (ttyS0)
            for byte in serial_input {
                self.device_manager.serial.receive_byte(0, byte);
            }

            // 2. Execute CPU instructions in batches
            let batch_size = (max_instructions - instructions_executed).min(INSTRUCTION_BATCH_SIZE);
            // SAFETY: see borrow_memory_for_cpu
            let result = unsafe {
                let mem_extended = self.borrow_memory_for_cpu();
                let io_ptr = core::ptr::NonNull::from(&mut self.devices);
                let ps_ptr = core::ptr::NonNull::from(&mut self.pc_system);
                // Wire DeviceManager into BxDevicesC for enum-based I/O dispatch
                let dm_ptr = core::ptr::NonNull::from(&mut self.device_manager);
                io_ptr.as_ptr().as_mut().unwrap_unchecked().set_device_manager(dm_ptr);
                let pic_ptr = core::ptr::NonNull::from(&mut self.device_manager.pic);
                let dma_ptr = core::ptr::NonNull::from(&mut self.device_manager.dma);
                let r = self.cpu
                    .cpu_loop_n_with_io(mem_extended, &[], batch_size, io_ptr, ps_ptr, Some(pic_ptr), Some(dma_ptr));
                self.devices.clear_device_manager();
                r
            };

            let should_update_gui = match result {
                Ok(executed) => {
                    instructions_executed += executed;

                    // Reset HLT+IF=0 counter on any non-zero batch
                    if executed > 0 {
                        hlt_if0_count = 0;
                    }

                    // Milestone progress print every 500K instructions
                    #[cfg(debug_assertions)]
                    if instructions_executed % 500_000 < INSTRUCTION_BATCH_SIZE {
                        tracing::debug!(
                            "[{}k instr] RIP={:#010x} CS={:#06x} mode={} batch_returned={} activity={:?}",
                            instructions_executed / 1000,
                            self.cpu.rip(),
                            self.cpu.get_cs_selector(),
                            self.cpu.get_cpu_mode(),
                            executed,
                            self.cpu.activity_state,
                        );
                    }
                    // Detect zero-return batches (HLT or stuck)
                    if executed == 0 {
                        // HLT with IF=0: CPU is dead (panic or intentional halt)
                        // Use counter-based approach: only break after N consecutive
                        // zero-batch HLT+IF=0 cycles. This allows transient IF=0 states
                        // (e.g. kernel cli/hlt sequences before init scripts) to recover.
                        if matches!(self.cpu.activity_state,
                            crate::cpu::cpu::CpuActivityState::Hlt
                            | crate::cpu::cpu::CpuActivityState::Mwait
                            | crate::cpu::cpu::CpuActivityState::MwaitIf)
                            && !self.cpu.interrupts_enabled()
                        {
                            hlt_if0_count += 1;
                            // Warn once at 1000 but DON'T break — match egui behavior.
                            // The egui path never exits on HLT+IF=0 and eventually the
                            // kernel recovers (timer/NMI wakes CPU). Breaking here would
                            // prevent headless Alpine from reaching modloop phase.
                            if hlt_if0_count == 1000 {
                                tracing::debug!(
                                    "[ZERO-BATCH] HLT/MWAIT with IF=0 for 1000 consecutive batches at RIP={:#x} CS={:#06x} activity={:?} — continuing (egui-match)",
                                    self.cpu.rip(), self.cpu.get_cs_selector(), self.cpu.activity_state,
                                );
                            }
                        } else {
                            hlt_if0_count = 0;
                        }
                    }

                    // If CPU triple-faulted into shutdown, stop emulation loop
                    if self.cpu.is_in_shutdown() {
                        tracing::error!("[Emulator] CPU triple-fault shutdown — stopping");
                        break;
                    }

                    // Port 92h (System Control) may have changed A20 during execution.
                    // Sync PC system + memory masks if any writes occurred.
                    if self.device_manager.port92.value != last_port92_value {
                        last_port92_value = self.device_manager.port92.value;
                        self.sync_a20_state();
                    }

                    // Check for reset requests: Port 92h fast reset or keyboard 0xFE
                    if self.device_manager.port92.reset_request || self.device_manager.keyboard.reset_requested {
                        if self.device_manager.port92.reset_request {
                            tracing::info!("Reset requested via Port 92h (fast reset)");
                        }
                        if self.device_manager.keyboard.reset_requested {
                            tracing::info!("Reset requested via keyboard controller 0xFE");
                        }
                        self.device_manager.port92.reset_request = false;
                        self.device_manager.keyboard.reset_requested = false;
                        if let Err(e) = self.reset(ResetReason::Hardware) {
                            tracing::error!("Reset failed: {}", e);
                            break;
                        }
                        last_port92_value = self.device_manager.port92.value;
                        continue;
                    }

                    // -- Progress tracking --
                    let current_rip = self.cpu.rip();

                    // Log progress every 10M instructions (countdown-based)
                    next_progress_log -= executed as i64;
                    if next_progress_log <= 0 {
                        next_progress_log += PROGRESS_LOG_INTERVAL as i64;
                        tracing::info!(
                            "Progress: {}M instructions, RIP={:#x}",
                            instructions_executed / 1_000_000,
                            current_rip
                        );
                    }

                    // vsprintf diagnostic removed (bug found and fixed: ADD AL,Ib operated on AH)

                    // Detailed EIP trace to track POST progression
                    // Log every batch in the critical PM→POST transition range
                    #[cfg(debug_assertions)]
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

                    // Detect stuck loop: RIP unchanged for many batches (debug only)
                    #[cfg(debug_assertions)]
                    {
                        if current_rip == last_rip {
                            stuck_count += 1;
                            if stuck_count >= 10 && !stuck_reported {
                                stuck_reported = true;
                                let bp = self.cpu.bp() as usize;
                                let ss_base = self.cpu.get_ss_base() as usize;
                                let bp_phys = ss_base + bp;
                                let ax = self.cpu.eax() as u16;
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
                                    "STUCK at RIP={:#x} after {}k instructions, last I/O read: port={:#06x} value={:#x}, CS={:#06x} mode={}, BP={:#06x} AX={:#06x} [BP+2]={:#06x} [BP+4]={:#06x} [BP+6]={:#06x}",
                                    current_rip,
                                    instructions_executed / 1000,
                                    self.devices.last_io_read_port,
                                    self.devices.last_io_read_value,
                                    self.cpu.get_cs_selector(),
                                    self.cpu.get_cpu_mode(),
                                    bp, ax, bp2, bp4, bp6,
                                );
                            }
                        } else {
                            stuck_count = 0;
                            stuck_reported = false;
                            last_rip = current_rip;
                        }
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

                        // In no-std builds, port 0xE9 output is silently dropped
                        // (no stdout available)
                    }

                    // Advance virtual time (Bochs-like ticking).
                    // Required so PIT can generate IRQ0 and BIOS can progress past HLT waits.
                    if self.config.ips != 0 {
                        if matches!(
                            self.cpu.activity_state,
                            crate::cpu::cpu::CpuActivityState::Hlt
                            | crate::cpu::cpu::CpuActivityState::Mwait
                            | crate::cpu::cpu::CpuActivityState::MwaitIf
                        ) {
                            // CPU is halted/mwait: advance virtual clock in 10-usec steps until an
                            // interrupt is pending. Matches Bochs handleWaitForEvent + BX_TICKN.
                            //
                            // When a GUI is attached AND the CPU is in protected mode: sleep once
                            // after the batch to synchronise virtual time to wall-clock time.
                            // This prevents the Linux console blank timer from firing ~360x early.
                            //
                            // Protected-mode-only: BIOS runs in real mode (mode=0) and its F12
                            // boot-wait HLTs should execute at full speed so the BIOS boots
                            // quickly. The kernel (mode=2) is what needs real-time throttling.
                            //
                            // We sleep ONCE per batch (not per iteration): on Windows,
                            // thread::sleep rounds up to ~15.6ms so per-iteration sleeps of 10µs
                            // would become 15,600ms per batch instead of 1:1.
                            //
                            // Without a GUI (headless): spin at full speed; the caller injects
                            // periodic keystrokes to keep the screen alive.
                            // Bochs handleWaitForEvent (event.cc:40-116): while(1) + BX_TICKN(10).
                            // Advances pc_system time (NOT icount) until interrupt fires.
                            // TSC reads pc_system.time_ticks(), so TSC advances during HLT
                            // without inflating icount.
                            // Safety cap: Bochs uses while(1) on a separate CPU thread.
                            // We cap at 100M ticks to yield for max_instructions/GUI checks.
                            // No icount inflation — TSC reads pc_system.time_ticks() directly.
                            // MwaitIf: wake on interrupt even when IF=0 (ECX[0]=1).
                            let mwait_if = matches!(self.cpu.activity_state, crate::cpu::cpu::CpuActivityState::MwaitIf);
                            let mut hlt_budget = 0u64;
                            #[cfg(feature = "std")]
                            let hlt_wall_start = std::time::Instant::now();
                            while hlt_budget < 100_000_000 {
                                if self.has_interrupt() && (self.cpu.interrupts_enabled() || mwait_if) {
                                    break;
                                }
                                if self.stop_flag.load(core::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }
                                // 1. Process pending LAPIC requests FIRST so timers are active
                                #[cfg(feature = "bx_support_apic")]
                                {
                                    // SAFETY: lapic_ptr set during CPU init; single-threaded access
                                    let lapic = unsafe { &mut *self.cpu.lapic_ptr_mut() };
                                    if lapic.timer_fired {
                                        lapic.timer_fired = false;
                                        lapic.periodic(self.pc_system.time_ticks());
                                    }
                                    if lapic.timer_deactivate_request {
                                        lapic.timer_deactivate_request = false;
                                        if let Some(h) = lapic.timer_handle {
                                            if let Err(e) = self.pc_system.deactivate_timer(h) {
                                                tracing::error!("LAPIC deactivate: {}", e);
                                            }
                                        }
                                    }
                                    if let Some(period) = lapic.timer_activate_request.take() {
                                        if let Some(h) = lapic.timer_handle {
                                            if let Err(e) = self.pc_system.activate_timer(h, period, false) {
                                                tracing::error!("LAPIC activate: {}", e);
                                            }
                                        }
                                        lapic.set_ticks_initial(self.pc_system.time_ticks());
                                    }
                                    if let Some(eoi_vec) = lapic.pending_eoi_vector.take() {
                                        self.device_manager.ioapic.receive_eoi(eoi_vec);
                                    }
                                    if lapic.intr && (self.cpu.interrupts_enabled() || mwait_if) {
                                        self.cpu.signal_event(1 << 2);
                                        break;
                                    }
                                }
                                // 2. Now get accurate countdown and advance
                                let step = self.pc_system.get_num_ticks_left_next_event().clamp(1, 100_000);
                                self.pc_system.tickn(step);
                                self.dispatch_timer_fires();
                                hlt_budget += step as u64;
                                let dev_usec = (step as u64 * 1_000_000 / (self.config.ips as u64).max(1)).max(1);
                                self.tick_devices(dev_usec);
                                self.sync_event_flags();
                                // Wall-clock throttle: sleep if virtual time races ahead
                                #[cfg(feature = "std")]
                                {
                                    let virtual_usec = hlt_budget * 1_000_000 / (self.config.ips as u64).max(1);
                                    let wall_usec = hlt_wall_start.elapsed().as_micros() as u64;
                                    if virtual_usec > wall_usec + 1_000 {
                                        let sleep_usec = (virtual_usec - wall_usec).min(15_000);
                                        std::thread::sleep(std::time::Duration::from_micros(sleep_usec));
                                    }
                                }
                            }

                            // If LAPIC has a pending interrupt, signal CPU
                            #[cfg(feature = "bx_support_apic")]
                            if self.cpu.lapic_has_intr() {
                                self.cpu.signal_event(1 << 2); // BX_EVENT_PENDING_LAPIC_INTR
                            }

                            // Tight MWAIT loop: process multiple wake→execute→MWAIT
                            // cycles without returning to the outer loop. This matches
                            // Bochs's dedicated CPU thread which never yields to GUI
                            // between MWAIT wakes. Budget: 15ms wall-clock.
                            #[cfg(feature = "std")]
                            let mwait_wall_start = std::time::Instant::now();
                            #[cfg(feature = "std")]
                            let mwait_wall_budget = std::time::Duration::from_millis(15);
                            loop {
                                #[cfg(feature = "std")]
                                if mwait_wall_start.elapsed() >= mwait_wall_budget { break; }
                                if self.stop_flag.load(core::sync::atomic::Ordering::Relaxed) { break; }
                                // Deliver PIC interrupt if pending
                                if self.device_manager.has_interrupt()
                                    && self.cpu.get_b_if() != 0
                                    && !self.cpu.interrupts_inhibited(0x01)
                                {
                                    let vec = self.iac();
                                    // SAFETY: see borrow_memory_for_cpu
                                    unsafe {
                                        let mem_ext = self.borrow_memory_for_cpu();
                                        self.cpu.set_mem_bus_ptr(core::ptr::NonNull::from(&mut *mem_ext));
                                        let _ = self.cpu.inject_external_interrupt(vec);
                                        self.cpu.clear_mem_bus();
                                    };
                                }
                                // Run CPU batch — handle_async_event inside cpu_loop_n
                                // will process LAPIC events and wake from MWAIT.
                                // Don't check activity_state here — LAPIC uses signal_event
                                // which sets async_event but doesn't change activity_state
                                // until handle_async_event runs inside the CPU loop.
                                let batch2 = (max_instructions.saturating_sub(instructions_executed)).min(INSTRUCTION_BATCH_SIZE);
                                if batch2 == 0 { break; }
                                // SAFETY: see borrow_memory_for_cpu
                                let r2 = unsafe {
                                    let mem_ext = self.borrow_memory_for_cpu();
                                    let io2 = core::ptr::NonNull::from(&mut self.devices);
                                    let ps2 = core::ptr::NonNull::from(&mut self.pc_system);
                                    let dm2 = core::ptr::NonNull::from(&mut self.device_manager);
                                    io2.as_ptr().as_mut().unwrap_unchecked().set_device_manager(dm2);
                                    let pic2 = core::ptr::NonNull::from(&mut self.device_manager.pic);
                                    let dma2 = core::ptr::NonNull::from(&mut self.device_manager.dma);
                                    let r = self.cpu.cpu_loop_n_with_io(mem_ext, &[], batch2, io2, ps2, Some(pic2), Some(dma2));
                                    self.devices.clear_device_manager();
                                    r
                                };
                                if let Ok(ex2) = r2 {
                                    instructions_executed += ex2;
                                    let u2 = if self.config.ips != 0 { (ex2 * 1_000_000 / (self.config.ips as u64)).max(10) } else { 10 };
                                    self.tick_devices(u2);
                                    self.sync_event_flags();
                                    self.pc_system.tickn(ex2 as u32);
                                    self.dispatch_timer_fires();
                                    // LAPIC sync
                                    #[cfg(feature = "bx_support_apic")]
                                    {
                                        // SAFETY: lapic_ptr set during CPU init; single-threaded access
                                        let lapic = unsafe { &mut *self.cpu.lapic_ptr_mut() };
                                        let tn = self.pc_system.time_ticks();
                                        lapic.current_ticks = tn;
                                        lapic.ticks_at_sync = tn;
                                        lapic.icount_at_sync = self.cpu.icount;
                                        if lapic.timer_fired { lapic.timer_fired = false; lapic.periodic(tn); }
                                        if lapic.timer_deactivate_request {
                                            lapic.timer_deactivate_request = false;
                                            if let Some(h) = lapic.timer_handle { let _ = self.pc_system.deactivate_timer(h); }
                                        }
                                        if let Some(period) = lapic.timer_activate_request.take() {
                                            if let Some(h) = lapic.timer_handle { let _ = self.pc_system.reactivate_timer_relative(h, period); }
                                            lapic.set_ticks_initial(self.pc_system.time_ticks());
                                        }
                                        if let Some(eoi_vec) = lapic.pending_eoi_vector.take() {
                                            self.device_manager.ioapic.receive_eoi(eoi_vec);
                                        }
                                        if lapic.intr { self.cpu.signal_event(1 << 2); }
                                    }
                                } else { break; }
                                // If CPU re-entered MWAIT, advance time again
                                if !matches!(self.cpu.activity_state,
                                    crate::cpu::cpu::CpuActivityState::Hlt
                                    | crate::cpu::cpu::CpuActivityState::Mwait
                                    | crate::cpu::cpu::CpuActivityState::MwaitIf
                                ) {
                                    break; // CPU is active — return to outer loop
                                }
                                // HLT loop: advance time to next interrupt
                                let mwait_if2 = matches!(self.cpu.activity_state, crate::cpu::cpu::CpuActivityState::MwaitIf);
                                let mut hlt2 = 0u64;
                                while hlt2 < 100_000_000 {
                                    if self.has_interrupt() && (self.cpu.interrupts_enabled() || mwait_if2) { break; }
                                    #[cfg(feature = "bx_support_apic")]
                                    {
                                        // SAFETY: lapic_ptr set during CPU init; single-threaded access
                                        let lapic = unsafe { &mut *self.cpu.lapic_ptr_mut() };
                                        if lapic.timer_fired { lapic.timer_fired = false; lapic.periodic(self.pc_system.time_ticks()); }
                                        if lapic.timer_deactivate_request {
                                            lapic.timer_deactivate_request = false;
                                            if let Some(h) = lapic.timer_handle { let _ = self.pc_system.deactivate_timer(h); }
                                        }
                                        if let Some(period) = lapic.timer_activate_request.take() {
                                            if let Some(h) = lapic.timer_handle { let _ = self.pc_system.activate_timer(h, period, false); }
                                            lapic.set_ticks_initial(self.pc_system.time_ticks());
                                        }
                                        if let Some(eoi_vec) = lapic.pending_eoi_vector.take() { self.device_manager.ioapic.receive_eoi(eoi_vec); }
                                        if lapic.intr && (self.cpu.interrupts_enabled() || mwait_if2) { self.cpu.signal_event(1 << 2); break; }
                                    }
                                    let s = self.pc_system.get_num_ticks_left_next_event().clamp(1, 100_000);
                                    self.pc_system.tickn(s);
                                    self.dispatch_timer_fires();
                                    hlt2 += s as u64;
                                    let du = (s as u64 * 1_000_000 / (self.config.ips as u64).max(1)).max(1);
                                    self.tick_devices(du);
                                    self.sync_event_flags();
                                }
                                #[cfg(feature = "bx_support_apic")]
                                if self.cpu.lapic_has_intr() { self.cpu.signal_event(1 << 2); }
                            }
                        } else {
                            let usec_from_instr =
                                (executed.saturating_mul(1_000_000)) / (self.config.ips as u64);
                            // min 10 usec to prevent timer starvation at low instruction counts.
                            let usec = usec_from_instr.max(10);
                            self.tick_devices(usec);
                            self.sync_event_flags();
                        }
                    }

                    // Drive pc_system timers via Bochs-exact tickn() mechanism.
                    self.pc_system.tickn(executed as u32);
                    self.dispatch_timer_fires();

                    // Handle LAPIC timer fires. With small batches (500 ticks) and
                    // typical LAPIC period (~24K ticks), at most 1 fire per batch.
                    // The catch-up loop is retained as a safety net.
                    //
                    // IMPORTANT: The `lapic` borrow must be dropped before calling
                    // check_timers(), because the timer callback also mutably accesses
                    // the same BxLocalApic via raw pointer. Holding &mut across that
                    // call would be UB and the compiler may optimize away re-reads.
                    #[cfg(feature = "bx_support_apic")]
                    {
                        let lapic_ptr = self.cpu.lapic_ptr_mut();

                        // Sync LAPIC tick tracking for live timer reads
                        {
                            // SAFETY: lapic_ptr set during CPU init; single-threaded access
                            let lapic = unsafe { &mut *lapic_ptr };
                            let ticks_now = self.pc_system.time_ticks();
                            lapic.current_ticks = ticks_now;
                            lapic.ticks_at_sync = ticks_now;
                            lapic.icount_at_sync = self.cpu.icount;
                        }

                        // Catch-up loop: fire timer for each missed period in this batch.
                        // Each iteration: borrow lapic → process fire → drop lapic →
                        // check_timers (may set timer_fired via callback) → re-check.
                        let mut catchup_count = 0u32;
                        let max_catchup = 1000u32; // safety limit
                        loop {
                            // Borrow lapic, check timer_fired, process fire, drop borrow
                            let should_continue = {
                                // SAFETY: lapic_ptr set during CPU init; single-threaded access
                                let lapic = unsafe { &mut *lapic_ptr };
                                if !lapic.timer_fired || catchup_count >= max_catchup {
                                    false
                                } else {
                                    lapic.timer_fired = false;
                                    lapic.diag_timer_fires += 1;
                                    let ticks_now = self.pc_system.time_ticks();
                                    lapic.periodic(ticks_now);

                                    // Process pending timer deactivation
                                    if lapic.timer_deactivate_request {
                                        lapic.timer_deactivate_request = false;
                                        if let Some(handle) = lapic.timer_handle {
                                            let _ = self.pc_system.deactivate_timer(handle);
                                        }
                                    }

                                    // Process pending timer reactivation (periodic catch-up)
                                    if let Some(period) = lapic.timer_activate_request.take() {
                                        if let Some(handle) = lapic.timer_handle {
                                            let _ = self.pc_system.reactivate_timer_relative(handle, period);
                                        }
                                        lapic.set_ticks_initial(self.pc_system.time_ticks());
                                    }

                                    catchup_count += 1;
                                    true
                                }
                            }; // lapic borrow dropped here

                            if !should_continue {
                                break;
                            }

                            // Trigger any timers due at exactly the current tick.
                            // tickn(0) fires countdown_event() only if curr_countdown==0,
                            // which happens when reactivate_timer_relative set it to 0.
                            self.pc_system.tickn(0);
                            self.dispatch_timer_fires();
                        }

                        // Handle non-fire deactivate/activate requests (from
                        // set_initial_timer_count during instruction execution)
                        {
                            // SAFETY: lapic_ptr set during CPU init; single-threaded access
                            let lapic = unsafe { &mut *lapic_ptr };
                            if lapic.timer_deactivate_request {
                                lapic.timer_deactivate_request = false;
                                if let Some(handle) = lapic.timer_handle {
                                    let _ = self.pc_system.deactivate_timer(handle);
                                }
                            }
                            if let Some(period) = lapic.timer_activate_request.take() {
                                if let Some(handle) = lapic.timer_handle {
                                    // Fresh activation — use absolute time_to_fire
                                    let _ = self.pc_system.activate_timer(handle, period, false);
                                }
                                lapic.set_ticks_initial(self.pc_system.time_ticks());
                            }

                            // Forward EOI broadcast from LAPIC to I/O APIC
                            if let Some(eoi_vec) = lapic.pending_eoi_vector.take() {
                                self.device_manager.ioapic.receive_eoi(eoi_vec);
                            }

                            // Signal pending LAPIC interrupt to CPU event system
                            if lapic.intr {
                                self.cpu.signal_event(1 << 2); // BX_EVENT_PENDING_LAPIC_INTR
                            }
                        }
                    }

                    // Propagate A20 gate changes from keyboard controller to memory system
                    // Matching Bochs BX_SET_ENABLE_A20() which immediately updates pc_system and memory
                    if self.device_manager.keyboard.a20_change_pending {
                        self.device_manager.keyboard.a20_change_pending = false;
                        let a20 = self.device_manager.keyboard.a20_enabled;
                        self.pc_system.set_enable_a20(a20);
                        self.memory.set_a20_mask(self.pc_system.a20_mask());
                        // Bochs pc_system.cc MemoryMappingChanged() calls BX_CPU(0)->TLB_flush()
                        // after A20 changes, since A20 masking affects physical address translation.
                        self.cpu.tlb_flush();
                    }

                    // Log batch sizes and check if timer ticking works
                    #[cfg(debug_assertions)]
                    if instructions_executed < 5 * INSTRUCTION_BATCH_SIZE
                        || instructions_executed % 100_000 < INSTRUCTION_BATCH_SIZE
                    {
                        let pit_c0_count = self.device_manager.pit.counters[0].count;
                        // Read BDA timer tick counter at 0x046C (4 bytes) directly from RAM
                        let bda_ticks = {
                            let (ptr, len) = self.memory.get_raw_memory_ptr();
                            if 0x046C + 4 <= len {
                        // SAFETY: pointer and length validated by caller; memory region is valid
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
                    #[cfg(debug_assertions)]
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

                    // Deliver pending PIC interrupts to the CPU (Bochs-like).
                    // Only use PIC path — LAPIC interrupts are delivered via
                    // handleAsyncEvent() through the CPU event system.
                    if self.device_manager.has_interrupt()
                        && self.cpu.get_b_if() != 0
                        && !self.cpu.interrupts_inhibited(0x01)
                    // BX_INHIBIT_INTERRUPTS
                    {
                        let vector = self.iac();

                        // Temporarily wire the memory bus so the interrupt path can
                        // read IVT/IDT and push stack frames correctly.
                        // SAFETY: see borrow_memory_for_cpu
                        let inject_result = unsafe {
                            let mem_extended = self.borrow_memory_for_cpu();
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

            // Drain serial port output every batch for responsive serial console.
            // Previously gated by should_update_gui (100ms) — now immediate.
            {
                let serial_bytes: Vec<u8> = self.device_manager.drain_serial_tx(0).collect();
                if !serial_bytes.is_empty() {
                    if let Some(ref gui) = self.gui {
                        let text = String::from_utf8_lossy(&serial_bytes);
                        gui.append_serial_log(&text);
                    }
                    // Always write serial output to stdout for headless/terminal visibility
                    #[cfg(feature = "std")]
                    {
                        use std::io::Write;
                        let _ = std::io::stdout().write_all(serial_bytes.as_slice());
                        let _ = std::io::stdout().flush();
                    }
                }
            }

            // Update GUI after CPU execution (outside the match to avoid borrow conflicts)
            // Update more frequently if text is dirty OR periodically (like Bochs timer)
            if should_update_gui {
                self.update_gui();
            }

            // Update IPS: show_ips() every 1 real second (keeps egui status bar responsive).
            // Uses icount delta (Bochs-compatible: counts REP iterations as separate ticks).
            // Bochs main.cc:1472 — ips_count = bx_pc_system.time_ticks() delta
            let ips_elapsed = last_ips_update.elapsed();
            if ips_elapsed >= IPS_SHOW_INTERVAL {
                let current_icount = self.cpu.icount;
                let delta_ticks = current_icount - last_ips_instructions;
                let mips = (delta_ticks as f64 / ips_elapsed.as_secs_f64()) / 1_000_000.0;
                let ips = (mips * 1_000_000.0) as u32;
                last_ips_instructions = current_icount;
                last_ips_update = std::time::Instant::now();
                if let Some(ref mut gui) = self.gui {
                    gui.show_ips(ips);
                }
            }
            // Print MIPS terminal line every 50M instructions (~5s at 9 MIPS).
            if instructions_executed / MIPS_LOG_INTERVAL
                > last_mips_log_instructions / MIPS_LOG_INTERVAL
            {
                let log_elapsed = last_mips_log_update.elapsed();
                let log_delta = instructions_executed - last_mips_log_instructions;
                let mips = if log_elapsed.as_secs_f64() > 0.001 {
                    (log_delta as f64 / log_elapsed.as_secs_f64()) / 1_000_000.0
                } else {
                    0.0
                };
                last_mips_log_instructions = instructions_executed;
                last_mips_log_update = std::time::Instant::now();
                tracing::info!(
                    target: "mips",
                    "[{:>6}M instr] {:>6.2} MIPS  RIP={:#010x}  CS={:#06x}  mode={}",
                    instructions_executed / 1_000_000,
                    mips,
                    self.cpu.rip(),
                    self.cpu.get_cs_selector(),
                    self.get_cpu_mode_str(),
                );
            }

            // 5. sync=slowdown: interval-based throttle matching Bochs slowdown.cc.
            // Compares emulated vs wall-clock time over a sliding 1-second window.
            // Resets the window periodically to prevent unbounded deficit accumulation
            // (which would cause massive sleeps when transitioning from active to idle).
            if self.config.sync_slowdown && self.config.ips > 0 {
                let wall_elapsed = slowdown_start.elapsed().as_micros() as u64;
                // Reset window every 1 second to prevent deficit accumulation
                if wall_elapsed > 1_000_000 {
                    slowdown_start = std::time::Instant::now();
                    slowdown_ticks_base = self.pc_system.time_ticks();
                } else {
                    let delta_ticks = self.pc_system.time_ticks().saturating_sub(slowdown_ticks_base);
                    let emu_usec = delta_ticks.saturating_mul(1_000_000)
                        / (self.config.ips as u64);
                    // Sleep if emulated time is >50ms ahead within this window.
                    // 50ms threshold avoids Windows 15.6ms timer granularity issues.
                    if emu_usec > wall_elapsed + 50_000 {
                        let sleep_usec = (emu_usec - wall_elapsed).min(50_000);
                        std::thread::sleep(std::time::Duration::from_micros(sleep_usec));
                    }
                }
            }

            // 6. Check if we should exit (e.g., shutdown requested)
            // TODO: Add shutdown flag check
        }

        tracing::debug!(
            "Interactive execution completed: {} instructions",
            instructions_executed
        );

        // Print perf summary to stderr (only for large batches, not sub-batches)
        if instructions_executed >= 1_000_000 {
            let pi = self.cpu.perf_instructions;
            let tlb_h = self.cpu.perf_tlb_hit;
            let tlb_m = self.cpu.perf_tlb_miss;
            let pw = self.cpu.perf_page_walk;
            let ic_m = self.cpu.perf_icache_miss;
            let pf = self.cpu.perf_prefetch;
            let tlb_total = tlb_h + tlb_m;
            let tlb_pct = if tlb_total > 0 { tlb_h as f64 / tlb_total as f64 * 100.0 } else { 0.0 };
            // icount = instruction count (REP iterations count as separate ticks)
            let bochs_ticks = self.cpu.icount;
            tracing::info!("[PERF] dispatches={pi} bochs_ticks={bochs_ticks} tlb_hit={tlb_h} tlb_miss={tlb_m} tlb_hit%={tlb_pct:.2}% page_walks={pw}");
        }

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
        let ips = self.config.ips as u64;
        let mut total_executed = 0u64;
        // Wall-clock budget: 15ms keeps GUI responsive at 60 fps.
        // Bochs runs CPU on a dedicated thread with no frame budget; we emulate
        // that throughput by processing multiple MWAIT→wake→execute cycles here.
        #[cfg(feature = "std")]
        let wall_start = std::time::Instant::now();
        #[cfg(feature = "std")]
        let wall_budget = std::time::Duration::from_millis(15);

        'batch: loop {
            // --- Run CPU batch ---
            // SAFETY: see borrow_memory_for_cpu
            let result = unsafe {
                let mem_extended = self.borrow_memory_for_cpu();
                let io_ptr = core::ptr::NonNull::from(&mut self.devices);
                let ps_ptr = core::ptr::NonNull::from(&mut self.pc_system);
                let dm_ptr = core::ptr::NonNull::from(&mut self.device_manager);
                io_ptr.as_ptr().as_mut().unwrap_unchecked().set_device_manager(dm_ptr);
                let pic_ptr = core::ptr::NonNull::from(&mut self.device_manager.pic);
                let dma_ptr = core::ptr::NonNull::from(&mut self.device_manager.dma);
                let r = self.cpu
                    .cpu_loop_n_with_io(mem_extended, &[], max_instructions, io_ptr, ps_ptr, Some(pic_ptr), Some(dma_ptr));
                self.devices.clear_device_manager();
                r
            };

            let executed = match result {
                Ok(n) => n,
                Err(e) => return Err(crate::error::Error::Cpu(e)),
            };
            total_executed += executed;

            // --- Tick devices + pc_system ---
            let usec = (executed * 1_000_000).checked_div(ips).unwrap_or(0).max(10);
            self.tick_devices(usec);
            self.sync_event_flags();
            self.pc_system.tickn(executed as u32);
            self.dispatch_timer_fires();

            // --- LAPIC timer catchup ---
            #[cfg(feature = "bx_support_apic")]
            {
                let lapic_ptr = self.cpu.lapic_ptr_mut();
                {
                    // SAFETY: lapic_ptr set during CPU init; single-threaded access
                    let lapic = unsafe { &mut *lapic_ptr };
                    let ticks_now = self.pc_system.time_ticks();
                    lapic.current_ticks = ticks_now;
                    lapic.ticks_at_sync = ticks_now;
                    lapic.icount_at_sync = self.cpu.icount;
                }
                let mut catchup_count = 0u32;
                loop {
                    // SAFETY: lapic_ptr set during CPU init; single-threaded access
                    let lapic = unsafe { &mut *lapic_ptr };
                    if !lapic.timer_fired || catchup_count >= 1000 { break; }
                    lapic.timer_fired = false;
                    lapic.diag_timer_fires += 1;
                    lapic.periodic(self.pc_system.time_ticks());
                    if lapic.timer_deactivate_request {
                        lapic.timer_deactivate_request = false;
                        if let Some(h) = lapic.timer_handle {
                            let _ = self.pc_system.deactivate_timer(h);
                        }
                    }
                    if let Some(period) = lapic.timer_activate_request.take() {
                        if let Some(h) = lapic.timer_handle {
                            let _ = self.pc_system.reactivate_timer_relative(h, period);
                        }
                        lapic.set_ticks_initial(self.pc_system.time_ticks());
                    }
                    catchup_count += 1;
                    self.pc_system.tickn(0);
                    self.dispatch_timer_fires();
                }
                {
                    // SAFETY: lapic_ptr set during CPU init; single-threaded access
                    let lapic = unsafe { &mut *lapic_ptr };
                    if lapic.timer_deactivate_request {
                        lapic.timer_deactivate_request = false;
                        if let Some(h) = lapic.timer_handle {
                            let _ = self.pc_system.deactivate_timer(h);
                        }
                    }
                    if let Some(period) = lapic.timer_activate_request.take() {
                        if let Some(h) = lapic.timer_handle {
                            let _ = self.pc_system.activate_timer(h, period, false);
                        }
                        lapic.set_ticks_initial(self.pc_system.time_ticks());
                    }
                    if lapic.intr {
                        self.cpu.signal_event(1 << 2);
                    }
                }
            }

            // --- HLT/MWAIT: advance time until interrupt ---
            if matches!(
                self.cpu.activity_state,
                crate::cpu::cpu::CpuActivityState::Hlt
                | crate::cpu::cpu::CpuActivityState::Mwait
                | crate::cpu::cpu::CpuActivityState::MwaitIf
            ) {
                let mwait_if = matches!(self.cpu.activity_state, crate::cpu::cpu::CpuActivityState::MwaitIf);
                let mut hlt_budget = 0u64;
                #[cfg(feature = "std")]
                let hlt_wall_start = std::time::Instant::now();
                while hlt_budget < 100_000_000 {
                    if self.has_interrupt() && (self.cpu.interrupts_enabled() || mwait_if) { break; }
                    #[cfg(feature = "bx_support_apic")]
                    {
                        // SAFETY: lapic_ptr set during CPU init; single-threaded access
                        let lapic = unsafe { &mut *self.cpu.lapic_ptr_mut() };
                        if lapic.timer_fired {
                            lapic.timer_fired = false;
                            lapic.periodic(self.pc_system.time_ticks());
                        }
                        if lapic.timer_deactivate_request {
                            lapic.timer_deactivate_request = false;
                            if let Some(h) = lapic.timer_handle {
                                let _ = self.pc_system.deactivate_timer(h);
                            }
                        }
                        if let Some(period) = lapic.timer_activate_request.take() {
                            if let Some(h) = lapic.timer_handle {
                                let _ = self.pc_system.activate_timer(h, period, false);
                            }
                            lapic.set_ticks_initial(self.pc_system.time_ticks());
                        }
                        if let Some(eoi_vec) = lapic.pending_eoi_vector.take() {
                            self.device_manager.ioapic.receive_eoi(eoi_vec);
                        }
                        if lapic.intr && (self.cpu.interrupts_enabled() || mwait_if) {
                            self.cpu.signal_event(1 << 2);
                            break;
                        }
                    }
                    let step = self.pc_system.get_num_ticks_left_next_event().clamp(1, 100_000);
                    self.pc_system.tickn(step);
                    self.dispatch_timer_fires();
                    hlt_budget += step as u64;
                    let dev_usec = (step as u64 * 1_000_000 / ips.max(1)).max(1);
                    self.tick_devices(dev_usec);
                    self.sync_event_flags();
                    // Wall-clock throttle: sleep if virtual time races ahead
                    #[cfg(feature = "std")]
                    {
                        let virtual_usec = hlt_budget * 1_000_000 / ips.max(1);
                        let wall_usec = hlt_wall_start.elapsed().as_micros() as u64;
                        if virtual_usec > wall_usec + 1_000 {
                            let sleep_usec = (virtual_usec - wall_usec).min(15_000);
                            std::thread::sleep(std::time::Duration::from_micros(sleep_usec));
                        }
                    }
                }
                #[cfg(feature = "bx_support_apic")]
                if self.cpu.lapic_has_intr() {
                    self.cpu.signal_event(1 << 2);
                }
            }

            // --- Deliver PIC interrupt ---
            if self.device_manager.has_interrupt()
                && self.cpu.get_b_if() != 0
                && !self.cpu.interrupts_inhibited(0x01)
            {
                let vector = self.iac();
                // SAFETY: see borrow_memory_for_cpu
                unsafe {
                    let mem_extended = self.borrow_memory_for_cpu();
                    self.cpu
                        .set_mem_bus_ptr(core::ptr::NonNull::from(&mut *mem_extended));
                    let _ = self.cpu.inject_external_interrupt(vector);
                    self.cpu.clear_mem_bus();
                };
            }

            // --- Tight loop: if CPU was woken from MWAIT and wall budget remains,
            // run another cycle instead of returning to egui event loop.
            // This matches Bochs's dedicated CPU thread which never yields to GUI.
            if matches!(self.cpu.activity_state, crate::cpu::cpu::CpuActivityState::Active)
                && {
                    #[cfg(feature = "std")]
                    { wall_start.elapsed() < wall_budget }
                    #[cfg(not(feature = "std"))]
                    { true }
                } {
                    continue 'batch;
                }

            break 'batch;
        }

        // Sync A20 state
        self.sync_a20_state();

        // Handle keyboard scancodes and serial input from GUI
        let mut scancodes_to_send = Vec::new();
        let mut serial_input = Vec::new();
        if let Some(ref mut gui) = self.gui {
            gui.handle_events();
            scancodes_to_send = gui.get_pending_scancodes();
            serial_input = gui.get_pending_serial_input();
        }
        for scancode in scancodes_to_send {
            self.device_manager.keyboard.send_scancode(scancode);
        }
        for byte in serial_input {
            self.device_manager.serial.receive_byte(0, byte);
        }

        let shutdown = self.cpu.is_in_shutdown();
        Ok((total_executed, shutdown))
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
        // SAFETY: debug counter; single-threaded access
        unsafe {
            DBG_CTR += 1;
        }
        // SAFETY: debug counter; single-threaded access
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
                tracing::debug!(
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
                    update_result.iwidth.checked_div(update_result.fwidth).unwrap_or(update_result.iwidth),
                    update_result.iheight.checked_div(update_result.fheight).unwrap_or(update_result.iheight),
                    update_result.fwidth,
                    update_result.fheight,
                );
            }

            display.render_text_to_framebuffer(
                &update_result.text_buffer,
                cursor_x as u32,
                cursor_y,
                update_result.tm_info.cs_start,
                update_result.tm_info.cs_end,
                update_result.tm_info.line_graphics,
                update_result.tm_info.start_address as u32,
                update_result.tm_info.line_offset as u32,
                &update_result.tm_info.actl_palette,
            );
        } else if dbg % 300 == 1 {
            // VGA returned None — not in text mode or not initialized
            let gr6 = self.device_manager.vga.graphics_regs[6];
            let ga = (gr6 & 0x01) != 0;
            let mm = (gr6 >> 2) & 0x03;
            tracing::debug!(
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

    /// Send a string as PS/2 Set 2 scancodes (make + break for each character).
    ///
    /// Useful for headless testing — inject "root\n" to type at a login prompt.
    /// Each character is converted to its scancode sequence including shift
    /// modifier when needed.
    pub fn send_string(&mut self, text: &str) {
        for ch in text.chars() {
            let scancodes = crate::gui::keymap::char_to_scancode_sequence(ch);
            for &sc in &scancodes {
                self.device_manager.keyboard.send_scancode(sc);
            }
        }
    }

    /// Force VGA to generate an initial update (call before first `update_display`).
    pub fn force_vga_update(&mut self) {
        self.device_manager.vga.force_initial_update();
    }

    /// Initialize VGA to standard text mode 3 (80x25 color).
    /// Must be called for direct kernel boot where no BIOS runs.
    pub fn init_vga_text_mode3(&mut self) {
        self.device_manager.vga.init_text_mode3();
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

    /// Get ATA channel read counters for diagnostics.
    pub fn ata_diag_reads(&self) -> (u64, u64) {
        (0, 0)
    }

    /// Get ATA channel 1 (CD-ROM) controller state + interrupt routing diagnostics.
    pub fn ata_ch1_diag(&self) -> String {
        let ch1 = &self.device_manager.harddrv.channels[1];
        let d = ch1.selected_drive();
        let (vec15, masked15, trig15, _dmode15) = self.device_manager.ioapic.redirect_entry_diag(15);
        // Check LAPIC IRR/ISR for the IDE vector
        let (irr_set, isr_set) = if vec15 > 0 { self.cpu.lapic_vector_state(vec15) } else { (false, false) };
        format!("s={:?} cmd={:#04x} ip={} acmd={:#04x} nIEN={} IOAPIC15[v={:#04x} m={} t={}] LAPIC[irr={} isr={}]",
            d.controller.status, d.controller.current_command,
            d.controller.interrupt_pending,
            d.atapi.command,
            d.controller.control & 0x02,
            vec15, masked15 as u8, trig15,
            irr_set, isr_set)
    }

    /// Get total I/O port read/write counters for diagnostics.
    pub fn io_diag_counts(&self) -> (u64, u64) {
        (self.devices.diag_io_reads, self.devices.diag_io_writes)
    }

    /// Get CPU activity state and async_event for diagnostics.
    pub fn cpu_diag_state(&self) -> (u32, u32) {
        (self.cpu.activity_state as u32, self.cpu.async_event)
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

    /// Get CR3 (page directory base register) for page table walks.
    pub fn get_cr3(&self) -> u64 {
        self.cpu.cr3
    }

    /// Get EIP for diagnostics.
    pub fn get_eip(&self) -> u32 {
        self.cpu.eip()
    }

    /// Get segment register info: (selector, base, limit, valid_flags).
    pub fn get_seg_info(&self, seg_idx: usize) -> (u16, u64, u32, u32) {
        if seg_idx < 6 {
            let selector = self.cpu.sregs[seg_idx].selector.value;
            let valid = self.cpu.sregs[seg_idx].cache.valid;
            let base = self.cpu.sregs[seg_idx].cache.u.segment_base();
            let limit = self.cpu.sregs[seg_idx].cache.u.segment_limit_scaled();
            (selector, base, limit, valid)
        } else {
            (0, 0, 0, 0)
        }
    }

    /// Get EAX/EBX/ECX/EDX for diagnostics.
    pub fn get_gpr32(&self, reg: usize) -> u32 {
        match reg {
            0 => self.cpu.eax(),
            1 => self.cpu.ecx(),
            2 => self.cpu.edx(),
            3 => self.cpu.ebx(),
            4 => self.cpu.esp(),
            5 => self.cpu.ebp(),
            6 => self.cpu.esi(),
            7 => self.cpu.edi(),
            _ => 0,
        }
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

    /// Get DTLB entry info for a given linear address.
    /// Returns (lpf, ppf, access_bits, host_page_addr) for the TLB slot
    /// that would be used for a dword read at `laddr`.
    pub fn get_dtlb_info(&self, laddr: u64) -> (u64, u64, u32, u64) {
        let idx = self.cpu.dtlb.get_index_of(laddr, 3);
        let entry = &self.cpu.dtlb.entries[idx];
        (entry.lpf, entry.ppf, entry.access_bits, entry.host_page_addr)
    }

    /// Get user_pl flag (true = CPL==3).
    pub fn get_user_pl(&self) -> bool {
        self.cpu.user_pl
    }

    /// Get mem_host_base pointer value for diagnostics.
    pub fn get_mem_host_base(&self) -> u64 {
        self.cpu.mem_host_base as u64
    }

    /// Get mem_host_len for diagnostics.
    pub fn get_mem_host_len(&self) -> usize {
        self.cpu.mem_host_len
    }

    /// Read a physical dword directly from host memory (bypassing TLB/paging).
    /// Returns None if address is out of range.
    pub fn read_phys_dword(&self, paddr: u64) -> Option<u32> {
        let addr = paddr as usize;
        let host_base = self.cpu.mem_host_base;
        if !host_base.is_null() && addr + 4 <= self.cpu.mem_host_len {
            // SAFETY: host pointer validated during TLB fill; offset within page bounds
            Some(unsafe { (host_base.add(addr) as *const u32).read_unaligned() })
        } else {
            None
        }
    }
}

impl<I: BxCpuIdTrait> Emulator<'_, I> {
    /// Dump comprehensive diagnostic state (for Alpine debugging).
    #[cfg(all(feature = "std", debug_assertions))]
    pub fn dump_alpine_diag(&mut self) {
        tracing::debug!("\n=== DIAGNOSTIC DUMP ===");
        tracing::debug!("RIP={:#018x} RSP={:#018x} RBP={:#018x}",
            self.cpu.rip(), self.cpu.rsp(), self.cpu.rbp());
        tracing::debug!("RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
            self.cpu.rax(), self.cpu.rbx(), self.cpu.rcx(), self.cpu.rdx());
        tracing::debug!("RSI={:#018x} RDI={:#018x} R8={:#018x}  R9={:#018x}",
            self.cpu.rsi(), self.cpu.rdi(), self.cpu.r8(), self.cpu.r9());
        tracing::debug!("CS={:#06x} mode={} IF={}",
            self.cpu.get_cs_selector(), self.get_cpu_mode_str(),
            if self.cpu.get_b_if() != 0 { 1 } else { 0 });
        tracing::debug!("CR0={:#010x} CR3={:#018x}",
            self.cpu.cr0.bits(), self.cpu.cr3);
        tracing::debug!("pending_event={:#010x} event_mask={:#010x} async_event={}",
            self.cpu.pending_event, self.cpu.event_mask, self.cpu.async_event);
        #[cfg(debug_assertions)]
        {
            tracing::debug!("diag: intr_delivered={} if_blocked={} pic_empty={}",
                self.cpu.diag_hae_intr_delivered, self.cpu.diag_hae_intr_if_blocked,
                self.cpu.diag_hae_intr_pic_empty);
            // SYSCALL ring buffer
            tracing::debug!("--- Last {} SYSCALLs (total={}, sysret={}, blocked={}) ---",
                self.cpu.diag_syscall_ring_idx.min(32),
                self.cpu.diag_syscall_count,
                self.cpu.diag_sysret_count,
                self.cpu.diag_syscall_count.saturating_sub(self.cpu.diag_sysret_count));
            {
                let count = self.cpu.diag_syscall_ring_idx.min(32);
                let start = self.cpu.diag_syscall_ring_idx.saturating_sub(32);
                for i in start..self.cpu.diag_syscall_ring_idx {
                    let (nr, arg0, ic) = self.cpu.diag_syscall_ring[i % 32];
                    tracing::debug!("  syscall nr={} arg0={:#x} icount={}", nr, arg0, ic);
                }
            }
        }
        // PIC state
        tracing::debug!("--- PIC State ---");
        tracing::debug!("  master: IMR={:#04x} IRR={:#04x} ISR={:#04x} has_int={}",
            self.device_manager.pic.master.imr,
            self.device_manager.pic.master.irr,
            self.device_manager.pic.master.isr,
            self.device_manager.pic.has_interrupt());
        tracing::debug!("  slave:  IMR={:#04x} IRR={:#04x} ISR={:#04x}",
            self.device_manager.pic.slave.imr,
            self.device_manager.pic.slave.irr,
            self.device_manager.pic.slave.isr);
        // PIT state
        let pit_c0 = &self.device_manager.pit.counters[0];
        tracing::debug!("--- PIT State ---");
        tracing::debug!("  C0: mode={:?} count={} gate={} output={}",
            pit_c0.mode, pit_c0.count, pit_c0.gate, pit_c0.output);
        // Device tick diagnostics
        tracing::debug!("--- Device Tick Diag ---");
        tracing::debug!("  tick_count={} total_usec={} pit_fires={} irq0_latched={} iac_count={}",
            self.device_manager.diag_tick_count,
            self.device_manager.diag_total_usec,
            self.device_manager.diag_pit_fires,
            self.device_manager.diag_irq0_latched,
            self.device_manager.diag_iac_count);
        let lapic = self.cpu.lapic_ptr_mut();
        // SAFETY: lapic_ptr set during CPU init; single-threaded access
        let lapic_ref = unsafe { &*lapic };
        tracing::debug!("  lapic_timer_fires={} set_initial_count={} timer_masked={}",
            lapic_ref.diag_timer_fires, lapic_ref.diag_set_initial_count,
            lapic_ref.diag_timer_masked);
        // Show pc_system timer state for LAPIC timer
        if let Some(handle) = lapic_ref.timer_handle {
            let t = &self.pc_system.timers[handle];
            tracing::debug!("  pc_system_timer[{}]: flags={:?} time_to_fire={} period={} ticks_total={}",
                handle, t.flags, t.time_to_fire, t.period,
                self.pc_system.time_ticks());
        }
        lapic_ref.dump_state();
        // ATA channel diagnostics
        tracing::debug!("--- ATA Diag ---");
        tracing::debug!("  cmd_history (last 10):");
        let hist = &self.device_manager.harddrv.cmd_history;
        let start = if hist.len() > 10 { hist.len() - 10 } else { 0 };
        for (ch, cmd, lba) in &hist[start..] {
            tracing::debug!("    ch={} cmd={:#04x} lba={}", ch, cmd, lba);
        }
        // Dump key code addresses from memory
        {
            let ram = self.memory.ram_slice();
            let addrs: &[(u64, &str)] = &[
                (0x01e1d340, "delay_loop_entry"),
                (0x01e38ef0, "jmp_target_after_delay"),
                (0x01207430, "outer_loop_context"),
                (0x01207460, "stack_ret_addr_1"),
                (0x012074e0, "stack_ret_addr_2"),
            ];
            for (paddr, label) in addrs {
                let p = *paddr as usize;
                if p + 48 <= ram.len() {
                    let code = &ram[p..p+48];
                    tracing::debug!("--- {} (phys={:#010x}) ---", label, paddr);
                    for row in 0..3 {
                        let off = row * 16;
                        tracing::debug!("  +{:02x}: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}  {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                            off,
                            code[off], code[off+1], code[off+2], code[off+3],
                            code[off+4], code[off+5], code[off+6], code[off+7],
                            code[off+8], code[off+9], code[off+10], code[off+11],
                            code[off+12], code[off+13], code[off+14], code[off+15]);
                    }
                }
            }
        }
        // Dump stack (16 qwords)
        let rsp = self.cpu.rsp();
        if rsp > 0xffffffff80000000 {
            let cr3 = self.cpu.cr3 & !0xFFF;
            let ram = self.memory.ram_slice();
            let ram_len = ram.len();
            let read_u64 = |addr: u64| -> u64 {
                let pml4_idx = (addr >> 39) & 0x1FF;
                let pdpt_idx = (addr >> 30) & 0x1FF;
                let pd_idx = (addr >> 21) & 0x1FF;
                let pt_idx = (addr >> 12) & 0x1FF;
                let page_off = addr & 0xFFF;
                let safe_read = |phys: u64| -> u64 {
                    let off = phys as usize;
                    if off + 8 > ram_len { return 0; }
                    u64::from_le_bytes(ram[off..off + 8].try_into().unwrap())
                };
                let pml4e = safe_read(cr3 + pml4_idx * 8);
                if pml4e & 1 == 0 { return 0; }
                let pdpte = safe_read((pml4e & 0xFFFFF_FFFFF000) + pdpt_idx * 8);
                if pdpte & 1 == 0 { return 0; }
                if pdpte & 0x80 != 0 { return safe_read((pdpte & 0xFFFFF_C0000000) | (addr & 0x3FFFFFFF)); }
                let pde = safe_read((pdpte & 0xFFFFF_FFFFF000) + pd_idx * 8);
                if pde & 1 == 0 { return 0; }
                if pde & 0x80 != 0 { return safe_read((pde & 0xFFFFF_FFE00000) | (addr & 0x1FFFFF)); }
                let pte = safe_read((pde & 0xFFFFF_FFFFF000) + pt_idx * 8);
                if pte & 1 == 0 { return 0; }
                safe_read((pte & 0xFFFFF_FFFFF000) | page_off)
            };
            tracing::debug!("--- Stack at RSP={:#018x} ---", rsp);
            for i in 0..16 {
                let addr = rsp.wrapping_add(i * 8);
                let val = read_u64(addr);
                let marker = if val > 0xffffffff81000000 && val < 0xffffffff82000000 { " <-- kernel text?" } else { "" };
                tracing::debug!("  [{:+4}] {:#018x}{}", i * 8, val, marker);
            }
        }
        // Dump 64 bytes of code at current RIP via manual page walk
        let rip = self.cpu.rip();
        if rip > 0xffffffff80000000 {
            let cr3 = self.cpu.cr3 & !0xFFF;
            let ram = self.memory.ram_slice();
            let read_u64 = |paddr: u64| -> u64 {
                let p = paddr as usize;
                if p + 8 <= ram.len() {
                    u64::from_le_bytes(ram[p..p+8].try_into().unwrap())
                } else { 0 }
            };
            let pml4_idx = (rip >> 39) & 0x1FF;
            let pdpt_idx = (rip >> 30) & 0x1FF;
            let pd_idx = (rip >> 21) & 0x1FF;
            let pt_idx = (rip >> 12) & 0x1FF;
            let pml4e = read_u64(cr3 + pml4_idx * 8);
            if pml4e & 1 != 0 {
                let pdpte = read_u64((pml4e & 0x000FFFFF_FFFFF000) + pdpt_idx * 8);
                if pdpte & 1 != 0 {
                    let paddr = if pdpte & 0x80 != 0 {
                        (pdpte & 0x000FFFFF_C0000000) | (rip & 0x3FFFFFFF)
                    } else {
                        let pde = read_u64((pdpte & 0x000FFFFF_FFFFF000) + pd_idx * 8);
                        if pde & 1 != 0 {
                            if pde & 0x80 != 0 {
                                (pde & 0x000FFFFF_FFE00000) | (rip & 0x1FFFFF)
                            } else {
                                let pte = read_u64((pde & 0x000FFFFF_FFFFF000) + pt_idx * 8);
                                if pte & 1 != 0 {
                                    (pte & 0x000FFFFF_FFFFF000) | (rip & 0xFFF)
                                } else { 0 }
                            }
                        } else { 0 }
                    };
                    if paddr != 0 && (paddr as usize) + 64 <= ram.len() {
                        let code = &ram[paddr as usize..(paddr as usize) + 64];
                        tracing::debug!("--- Code at RIP={:#018x} (phys={:#010x}) ---", rip, paddr);
                        for row in 0..4 {
                            let off = row * 16;
                            tracing::debug!("  {:016x}: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}  {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                                rip + off as u64,
                                code[off], code[off+1], code[off+2], code[off+3],
                                code[off+4], code[off+5], code[off+6], code[off+7],
                                code[off+8], code[off+9], code[off+10], code[off+11],
                                code[off+12], code[off+13], code[off+14], code[off+15]);
                        }
                    }
                }
            }
        }
        tracing::debug!("=== END DIAGNOSTIC ===");
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
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let emu = Emulator::<Corei7SkylakeX>::new(config);
                assert!(emu.is_ok());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn test_emulator_initialization() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                assert!(!emu.is_initialized());

                let result = emu.initialize();
                assert!(result.is_ok());
                assert!(emu.is_initialized());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn test_multiple_instances_independent() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();

                let mut emu1 = Emulator::<Corei7SkylakeX>::new(config.clone()).unwrap();
                let emu2 = Emulator::<Corei7SkylakeX>::new(config).unwrap();

                emu1.initialize().unwrap();

                // emu2 should still be uninitialized
                assert!(emu1.is_initialized());
                assert!(!emu2.is_initialized());

                // Different tick counts
                emu1.pc_system.tickn(1000);
                assert_eq!(emu1.ticks(), 1000);
                assert_eq!(emu2.ticks(), 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
