//! Emulator Container
//!
//! This module provides the `Emulator` struct that owns and coordinates all
//! emulator components: CPU, Memory, Devices, and PC System.
//!
//! Each `Emulator` instance is fully independent with no global state,
//! allowing hundreds of emulator instances to run concurrently on different threads.

use crate::{
    cpu::{builder::BxCpuBuilder, BxCpuC, BxCpuIdTrait, ResetReason},
    iodev::{devices::SystemControlPort, BxDevicesC},
    memory::{BxMemC, BxMemoryStubC},
    params::BxParams,
    pc_system::BxPcSystemC,
    Result,
};

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
            guest_memory_size: 32 * 1024 * 1024,  // 32 MB
            host_memory_size: 32 * 1024 * 1024,   // 32 MB
            memory_block_size: 128 * 1024,         // 128 KB blocks
            ips: 4_000_000,                        // 4 MIPS
            pci_enabled: false,
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
    /// Device controller
    pub devices: BxDevicesC,
    /// PC system (timers, A20, etc.)
    pub pc_system: BxPcSystemC,
    /// System Control Port state (Port 0x92)
    pub system_control: SystemControlPort,
    /// Configuration
    config: EmulatorConfig,
    /// Whether the emulator has been initialized
    initialized: bool,
}

impl<'a, I: BxCpuIdTrait> Emulator<'a, I> {
    /// Create a new emulator instance with the given configuration
    ///
    /// This creates all components but does not initialize them.
    /// Call `initialize()` after creation to complete setup.
    pub fn new(config: EmulatorConfig) -> Result<Self> {
        // Create PC system
        let pc_system = BxPcSystemC::new();

        // Create memory
        let mem_stub = BxMemoryStubC::create_and_init(
            config.guest_memory_size,
            config.host_memory_size,
            config.memory_block_size,
        )?;
        let mut memory = BxMemC::new(mem_stub, config.pci_enabled);

        // Initialize memory with the same parameters
        memory.init_memory(
            config.guest_memory_size,
            config.host_memory_size,
            config.memory_block_size,
        )?;

        // Sync A20 mask from PC system
        memory.set_a20_mask(pc_system.a20_mask());

        // Create devices
        let devices = BxDevicesC::new();

        // Create CPU
        let builder: BxCpuBuilder<I> = BxCpuBuilder::new();
        let cpu = builder.build()?;

        Ok(Self {
            cpu,
            memory,
            devices,
            pc_system,
            system_control: SystemControlPort::new(),
            config,
            initialized: false,
        })
    }

    /// Initialize the emulator
    ///
    /// This runs the full initialization sequence from Bochs main.cc:1300-1363:
    /// 1. PC system initialization (timers, IPS)
    /// 2. Memory initialization (already done in new())
    /// 3. CPU initialization
    /// 4. Device initialization
    /// 5. State registration
    ///
    /// After this, call `load_bios()` to load a BIOS image, then `reset()` and `run()`.
    pub fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            tracing::warn!("Emulator already initialized");
            return Ok(());
        }

        tracing::info!("Initializing emulator");

        // Step 1: Initialize PC system with IPS
        self.pc_system.initialize(self.config.ips);
        tracing::debug!("PC system initialized with {} IPS", self.config.ips);

        // Step 2: Memory is already initialized in new()
        // Sync A20 mask
        self.memory.set_a20_mask(self.pc_system.a20_mask());

        // Step 3: Initialize CPU
        self.cpu.initialize(self.config.cpu_params.clone())?;
        tracing::debug!("CPU initialized");

        // Step 4: Initialize devices
        self.devices.init(&mut self.memory)?;
        tracing::debug!("Devices initialized");

        // Step 5: Register state for save/restore
        self.pc_system.register_state();
        self.devices.register_state()?;
        tracing::debug!("State registered");

        self.initialized = true;
        tracing::info!("Emulator initialization complete");

        Ok(())
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
        tracing::info!("Loaded optional ROM ({} bytes) at {:#x}", rom_data.len(), address);
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
        if matches!(reset_type, ResetReason::Hardware) {
            self.devices.reset(reset_type)?;
        }

        // Reset system control port state
        self.system_control = SystemControlPort::new();

        Ok(())
    }

    /// Start timers and prepare for execution
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

