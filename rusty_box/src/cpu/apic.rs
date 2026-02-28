//! Local APIC (Advanced Programmable Interrupt Controller) implementation.
//!
//! This module provides the Local APIC functionality, including register definitions,
//! interrupt handling, and timer management.

use tracing::{debug, info};

use crate::config::BxPhyAddress;

/// Edge-triggered interrupt mode
pub const APIC_EDGE_TRIGGERED: u8 = 0;

/// Level-triggered interrupt mode
pub const APIC_LEVEL_TRIGGERED: u8 = 1;

/// Default Local APIC base address
pub const BX_LAPIC_BASE_ADDR: BxPhyAddress = 0xfee00000;

/// XAPIC extension support flag for Interrupt Enable Register (IER)
pub const BX_XAPIC_EXT_SUPPORT_IER: u32 = 1 << 0;

/// XAPIC extension support flag for Specific End of Interrupt (SEOI)
pub const BX_XAPIC_EXT_SUPPORT_SEOI: u32 = 1 << 1;

/// APIC destination identifier
pub type ApicDest = u32;

/// APIC operating mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApicMode {
    /// APIC is globally disabled
    #[default]
    GloballyDisabled = 0,
    /// APIC state is invalid
    StateInvalid = 1,
    /// XAPIC mode (x86 APIC)
    XapicMode = 2,
    /// X2APIC mode (extended x86-64 APIC)
    X2apicMode = 3,
}

impl ApicMode {
    /// Converts a raw u64 value to ApicMode
    pub fn from_raw(value: u64) -> Self {
        match value & 3 {
            0 => ApicMode::GloballyDisabled,
            1 => ApicMode::StateInvalid,
            2 => ApicMode::XapicMode,
            3 => ApicMode::X2apicMode,
            _ => unreachable!(),
        }
    }

    /// Converts ApicMode to raw u64 value
    pub fn as_raw(self) -> u64 {
        self as u64
    }
}

/// Local APIC register offsets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LapicRegister {
    /// Local APIC ID register
    Id = 0x020,
    /// Local APIC version register
    Version = 0x030,
    /// Task priority register (TPR)
    Tpr = 0x080,
    /// Arbitration priority register
    ArbitrationPriority = 0x090,
    /// Processor priority register (PPR)
    Ppr = 0x0A0,
    /// End of interrupt register (EOI)
    Eoi = 0x0B0,
    /// Remote read register (RRD)
    Rrd = 0x0C0,
    /// Logical destination register (LDR)
    Ldr = 0x0D0,
    /// Destination format register (DFR)
    DestinationFormat = 0x0E0,
    /// Spurious interrupt vector register
    SpuriousVector = 0x0F0,
    /// In-service register 1-8
    Isr1 = 0x100,
    Isr2 = 0x110,
    Isr3 = 0x120,
    Isr4 = 0x130,
    Isr5 = 0x140,
    Isr6 = 0x150,
    Isr7 = 0x160,
    Isr8 = 0x170,
    /// Trigger mode register 1-8
    Tmr1 = 0x180,
    Tmr2 = 0x190,
    Tmr3 = 0x1A0,
    Tmr4 = 0x1B0,
    Tmr5 = 0x1C0,
    Tmr6 = 0x1D0,
    Tmr7 = 0x1E0,
    Tmr8 = 0x1F0,
    /// Interrupt request register 1-8
    Irr1 = 0x200,
    Irr2 = 0x210,
    Irr3 = 0x220,
    Irr4 = 0x230,
    Irr5 = 0x240,
    Irr6 = 0x250,
    Irr7 = 0x260,
    Irr8 = 0x270,
    /// Error status register (ESR)
    Esr = 0x280,
    /// Local vector table - CMCI
    LvtCmci = 0x2F0,
    /// Interrupt command register low
    IcrLo = 0x300,
    /// Interrupt command register high
    IcrHi = 0x310,
    /// Local vector table - Timer
    LvtTimer = 0x320,
    /// Local vector table - Thermal
    LvtThermal = 0x330,
    /// Local vector table - Performance monitor
    LvtPerfmon = 0x340,
    /// Local vector table - LINT0
    LvtLint0 = 0x350,
    /// Local vector table - LINT1
    LvtLint1 = 0x360,
    /// Local vector table - Error
    LvtError = 0x370,
    /// Timer initial count register
    TimerInitialCount = 0x380,
    /// Timer current count register
    TimerCurrentCount = 0x390,
    /// Timer divide configuration register
    TimerDivideCfg = 0x3E0,
    /// Self IPI register
    SelfIpi = 0x3F0,

    // Extended AMD features
    /// Extended APIC feature register
    ExtApicFeature = 0x400,
    /// Extended APIC control register
    ExtApicControl = 0x410,
    /// Specific end of interrupt register
    SpecificEoi = 0x420,
    /// Interrupt enable register 1-8
    Ier1 = 0x480,
    Ier2 = 0x490,
    Ier3 = 0x4A0,
    Ier4 = 0x4B0,
    Ier5 = 0x4C0,
    Ier6 = 0x4D0,
    Ier7 = 0x4E0,
    Ier8 = 0x4F0,
}

/// APIC delivery mode for interrupt commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ApicDeliveryMode {
    /// Fixed delivery mode
    Fixed = 0,
    /// Low priority delivery mode
    LowPriority = 1,
    /// System management interrupt (SMI)
    Smi = 2,
    /// Reserved
    Reserved = 3,
    /// Non-maskable interrupt (NMI)
    Nmi = 4,
    /// INIT delivery mode
    Init = 5,
    /// Startup IPI (SIPI)
    Sipi = 6,
    /// External interrupt
    ExtInt = 7,
}

/// Local Vector Table (LVT) entry indices
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum LocalVectorTableEntry {
    /// Timer LVT entry
    Timer = 0,
    /// Thermal sensor LVT entry
    Thermal = 1,
    /// Performance monitor LVT entry
    Perfmon = 2,
    /// Local interrupt 0 (LINT0) LVT entry
    Lint0 = 3,
    /// Local interrupt 1 (LINT1) LVT entry
    Lint1 = 4,
    /// Error LVT entry
    Error = 5,
    /// Corrected machine check interrupt (CMCI) LVT entry
    Cmci = 6,
}

/// Number of Local Vector Table entries
pub const LVT_ENTRY_COUNT: usize = 7;

/// Local Advanced Programmable Interrupt Controller (LAPIC)
///
/// The Local APIC handles interrupts for a single processor core, including
/// inter-processor interrupts (IPIs), local interrupts, and timer functionality.
#[derive(Debug, Default)]
pub struct BxLocalApic {
    /// Base physical address of the APIC
    base_addr: BxPhyAddress,
    /// Current APIC operating mode
    mode: ApicMode,
    /// Whether XAPIC mode is enabled
    xapic: bool,
    /// Enabled extended XAPIC features bitmask
    xapic_ext: u32,
    /// APIC ID (4-bit in legacy mode, 8-bit in XAPIC mode, 32-bit in X2APIC mode)
    apic_id: u32,
    /// APIC version ID
    apic_version_id: u32,
    /// Software enable flag
    software_enabled: bool,
    /// Spurious interrupt vector
    spurious_vector: u8,
    /// Focus processor checking disable flag
    focus_disable: bool,

    /// Task priority register (TPR)
    task_priority: u32,
    /// Logical destination register (LDR)
    ldr: u32,
    /// Destination format register (DFR)
    dest_format: u32,

    /// In-service register (ISR). When an IRR bit is cleared, the corresponding
    /// bit in ISR is set, indicating the interrupt is being serviced.
    isr: [u32; 8],
    /// Trigger mode register (TMR). Cleared for edge-triggered interrupts
    /// and set for level-triggered interrupts. If set, local APIC must send
    /// EOI message to all other APICs.
    tmr: [u32; 8],
    /// Interrupt request register (IRR). When an interrupt is triggered by
    /// the I/O APIC or another processor, it sets a bit in IRR. The bit is
    /// cleared when the interrupt is acknowledged by the processor.
    irr: [u32; 8],
    /// Interrupt enable register (IER). Only vectors that are enabled in IER
    /// participate in APIC's computation of highest priority pending interrupt.
    ier: [u32; 8],

    /// Error status register (ESR)
    error_status: u32,
    /// Shadow error status register
    shadow_error_status: u32,

    /// Interrupt command register high (ICR)
    icr_hi: u32,
    /// Interrupt command register low (ICR)
    icr_lo: u32,

    /// Local vector table entries
    lvt: [u32; LVT_ENTRY_COUNT],
    /// Initial timer count (for reloading periodic timer)
    timer_initial: u32,
    /// Current timer count
    timer_current: u32,
    /// Timer value when it started counting, also holds TSC-Deadline value
    ticks_initial: u64,

    /// Timer divide configuration register
    timer_divconf: u32,
    /// Timer divide factor
    timer_divide_factor: u32,

    /// Internal timer state (not accessible from bus)
    timer_active: bool,
    /// Timer handle identifier
    timer_handle: i32,

    /// VMX timer handle identifier
    vmx_timer_handle: i32,
    /// VMX preemption timer value
    vmx_preemption_timer_value: u32,
    /// System tick value when timer was set (absolute value)
    vmx_preemption_timer_initial: u64,
    /// System tick value when timer fires (absolute value)
    vmx_preemption_timer_fire: u64,
    /// VMX preemption timer rate (from MSR_VMX_MISC)
    vmx_preemption_timer_rate: u32,
    /// VMX timer active state
    vmx_timer_active: bool,

    /// MWAITX timer handle identifier
    mwaitx_timer_handle: i32,
    /// MWAITX timer active state
    mwaitx_timer_active: bool,
}

impl BxLocalApic {
    /// Deactivates the MWAITX timer if the main timer is not active
    pub(super) fn deactivate_mwaitx_timer(&mut self) {
        if self.timer_active {
            return;
        }
        // TODO: Implement timer activation via system interface
        // bx_pc_system.activate_timer_ticks(mwaitx_timer_handle, value, 0);
        self.mwaitx_timer_active = true;
    }

    /// Sets the base address and mode of the Local APIC
    ///
    /// The base address determines the APIC's operating mode:
    /// - Bits [11:10] specify the mode (0=disabled, 2=XAPIC, 3=X2APIC)
    /// - The base address is aligned to a 4KB boundary
    pub(super) fn set_base(&mut self, mut newbase: BxPhyAddress) {
        let mode_raw = (newbase >> 10) & 3;
        self.mode = ApicMode::from_raw(mode_raw);

        // Clear lower 8 bits to align base address
        newbase &= !(0xff as BxPhyAddress);
        self.base_addr = newbase;

        info!(
            "allocate APIC id={} (MMIO {}) to {newbase:x}",
            if self.mode == ApicMode::XapicMode {
                "enabled"
            } else {
                "disabled"
            },
            self.apic_id
        );

        if self.mode == ApicMode::X2apicMode {
            // In X2APIC mode, LDR is calculated from APIC ID
            self.ldr = ((self.apic_id & 0xfffffff0) << 16) | (1 << (self.apic_id & 0xf));
        }

        if self.mode == ApicMode::GloballyDisabled {
            // If local APIC becomes globally disabled, reset some fields to defaults
            self.write_spurious_interrupt_register(0xff);
        }
    }

    /// Enables XAPIC extensions (IER and SEOI support)
    pub(super) fn enable_xapic_extensions(&mut self) {
        self.apic_version_id = 0x80000000;
        self.xapic_ext = BX_XAPIC_EXT_SUPPORT_IER | BX_XAPIC_EXT_SUPPORT_SEOI;
    }

    /// Resets the Local APIC to its initial state (called on CPU reset)
    pub(super) fn reset(&mut self, _reset_type: u8) {
        // Initialize APIC registers to their reset values
        // base_addr remains unchanged (set by set_base)

        self.error_status = 0;
        self.shadow_error_status = 0;
        self.ldr = 0;
        self.dest_format = 0xf;
        self.icr_hi = 0;
        self.icr_lo = 0;
        self.task_priority = 0;

        // Clear interrupt request, in-service, and trigger mode registers
        for i in 0..8 {
            self.irr[i] = 0;
            self.isr[i] = 0;
            self.tmr[i] = 0;
            // All interrupts are enabled by default
            self.ier[i] = 0xFFFFFFFF;
        }

        // Reset timer configuration
        self.timer_divconf = 0;
        self.timer_divide_factor = 1;
        self.timer_initial = 0;
        self.timer_current = 0;
        self.ticks_initial = 0;

        // Deactivate timers
        self.timer_active = false;
        self.vmx_timer_active = false;
        self.mwaitx_timer_active = false;

        // Mask all LVT entries (0x10000 = masked bit)
        for i in 0..LVT_ENTRY_COUNT {
            self.lvt[i] = 0x10000;
        }

        // Reset spurious vector register fields
        self.spurious_vector = 0xff;
        self.software_enabled = false;
        self.focus_disable = false;

        // Set APIC mode to XAPIC by default
        self.mode = ApicMode::XapicMode;

        // Set version ID based on xapic flag
        if self.xapic {
            self.apic_version_id = 0x00050014; // P4 with 6 LVT entries
        } else {
            self.apic_version_id = 0x00030010; // P6 with 4 LVT entries
        }

        // Clear XAPIC extensions
        self.xapic_ext = 0;
    }

    /// Writes to the spurious interrupt vector register
    ///
    /// Bits [7:0]: Spurious vector (bits 0-3 hardwired to 1 in legacy mode)
    /// Bit 8: Software enable
    /// Bit 9: Focus processor checking disable
    fn write_spurious_interrupt_register(&mut self, value: u32) {
        debug!("write of {value:#x} to spurious interrupt register");

        if self.xapic {
            self.spurious_vector = (value & 0xff) as u8;
        } else {
            // Bits 0-3 of the spurious vector are hardwired to '1' in legacy mode
            self.spurious_vector = ((value & 0xf0) | 0x0f) as u8;
        }

        self.software_enabled = ((value >> 8) & 1) != 0;
        self.focus_disable = ((value >> 9) & 1) != 0;

        if !self.software_enabled {
            // When disabled, mask all LVT entries
            const LVT_MASK: u32 = 0x10000;
            for entry in &mut self.lvt {
                *entry |= LVT_MASK;
            }
        }
    }
}

/// APIC error status flags
///
/// These errors are reported in the Error Status Register (ESR).
/// Multiple errors can be active simultaneously, so this is represented
/// as a bitmask rather than individual enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApicError(u8);

impl ApicError {
    /// Illegal address error
    pub const ILLEGAL_ADDR: u8 = 0x80;
    /// Receive illegal vector error
    pub const RX_ILLEGAL_VEC: u8 = 0x40;
    /// Transmit illegal vector error
    pub const TX_ILLEGAL_VEC: u8 = 0x20;
    /// X2APIC redirectible IPI error
    pub const X2APIC_REDIRECTIBLE_IPI: u8 = 0x08;
    /// Receive accept error
    pub const RX_ACCEPT_ERR: u8 = 0x08;
    /// Transmit accept error
    pub const TX_ACCEPT_ERR: u8 = 0x04;
    /// Receive checksum error
    pub const RX_CHECKSUM: u8 = 0x02;
    /// Transmit checksum error
    pub const TX_CHECKSUM: u8 = 0x01;

    /// Creates a new ApicError from a raw byte value
    pub fn from_raw(value: u8) -> Self {
        ApicError(value)
    }

    /// Returns the raw byte value
    pub fn as_raw(self) -> u8 {
        self.0
    }

    /// Checks if a specific error flag is set
    pub fn has_flag(self, flag: u8) -> bool {
        (self.0 & flag) != 0
    }

    /// Sets an error flag
    pub fn set_flag(&mut self, flag: u8) {
        self.0 |= flag;
    }

    /// Clears an error flag
    pub fn clear_flag(&mut self, flag: u8) {
        self.0 &= !flag;
    }
}

impl From<ApicError> for u8 {
    fn from(value: ApicError) -> Self {
        value.as_raw()
    }
}

impl From<u8> for ApicError {
    fn from(value: u8) -> Self {
        ApicError::from_raw(value)
    }
}
