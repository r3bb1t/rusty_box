//! I/O APIC (82093AA) — Interrupt Redirection and Routing
//!
//! This module implements the Intel 82093AA I/O Advanced Programmable Interrupt
//! Controller. The I/O APIC receives interrupt signals from I/O devices and
//! routes them to Local APICs based on a programmable redirection table.
//!
//! ## MMIO Interface
//!
//! The I/O APIC is accessed through a memory-mapped register window at its
//! base address (default 0xFEC00000):
//!
//! - Offset 0x00: IOREGSEL — Index register (selects which internal register
//!   to access through the data register)
//! - Offset 0x10: IOWIN — Data register (read/write the register selected by
//!   IOREGSEL)
//!
//! ## Internal Registers (accessed via IOREGSEL/IOWIN)
//!
//! - 0x00: IOAPIC ID
//! - 0x01: IOAPIC Version
//! - 0x02: IOAPIC Arbitration ID
//! - 0x10-0x3F: I/O Redirection Table (24 entries, each 64 bits = 2 registers)
//!
//! ## Bochs Reference
//!
//! Ported from `cpp_orig/bochs/iodev/ioapic.cc` (370 lines) and
//! `cpp_orig/bochs/iodev/ioapic.h` (117 lines).


use crate::config::BxPhyAddress;
use crate::memory::BxMemC;

// ---------------------------------------------------------------------------
// Constants — matching Bochs ioapic.h
// ---------------------------------------------------------------------------

/// Base MMIO address for the I/O APIC (Intel 82093AA default).
/// Bochs: `#define BX_IOAPIC_BASE_ADDR (0xfec00000)` (ioapic.cc:121)
const IOAPIC_BASE_ADDR: u32 = 0xFEC0_0000;

/// Number of interrupt input pins on the 82093AA.
/// Bochs: `#define BX_IOAPIC_NUM_PINS (0x18)` (ioapic.h:34)
pub const IOAPIC_NUM_PINS: usize = 0x18; // 24

/// Version register value.
/// Low byte = APIC version (0x11 for 82093AA).
/// Bits [23:16] = maximum redirection entry index (NUM_PINS - 1).
/// Bochs: `const Bit32u BX_IOAPIC_VERSION_ID = (((BX_IOAPIC_NUM_PINS - 1) << 16) | 0x11)`
/// (ioapic.h:37)
const IOAPIC_VERSION_ID: u32 = ((IOAPIC_NUM_PINS as u32 - 1) << 16) | 0x11;

/// Default APIC ID assigned to the I/O APIC.
/// Bochs: `#define BX_IOAPIC_DEFAULT_ID (BX_SMP_PROCESSORS)` (ioapic.cc:122)
/// For a single-processor system, the I/O APIC gets ID 1.
const IOAPIC_DEFAULT_ID: u32 = 1;

/// APIC ID mask — determines the width of the APIC ID field.
/// In XAPIC mode (Bochs main.cc:1031): `apic_id_mask = simulate_xapic ? 0xFF : 0xF`.
/// We default to legacy (4-bit) for compatibility; XAPIC extends to 8-bit.
const APIC_ID_MASK: u32 = 0x0F;

/// MMIO region size (4KB page).
const IOAPIC_MMIO_SIZE: u32 = 0x1000;

/// Redirect entry default: masked, all other bits zero.
/// Bochs: `bx_io_redirect_entry_t(): hi(0), lo(0x10000) {}` (ioapic.h:43)
const REDIRECT_ENTRY_DEFAULT_LO: u32 = 0x0001_0000;

/// Mask applied when writing the low 32 bits of a redirect entry.
/// Preserves read-only bits (delivery status bit 12, remote IRR bit 14).
/// Bochs: `lo = val_lo_part & 0xffffafff` (ioapic.h:64)
const REDIRECT_LO_WRITE_MASK: u32 = 0xFFFF_AFFF;

// ---------------------------------------------------------------------------
// IOREGSEL register indices
// ---------------------------------------------------------------------------

/// IOREGSEL value for APIC ID register.
const IOREGSEL_ID: u32 = 0x00;

/// IOREGSEL value for version register.
const IOREGSEL_VERSION: u32 = 0x01;

/// IOREGSEL value for arbitration ID register.
const IOREGSEL_ARB_ID: u32 = 0x02;

/// First IOREGSEL value for the redirection table.
/// Entry N: low word = 0x10 + 2*N, high word = 0x11 + 2*N.
const IOREGSEL_REDTBL_BASE: u32 = 0x10;

// ---------------------------------------------------------------------------
// Delivery mode (bits [10:8] of redirect entry low word)
// ---------------------------------------------------------------------------

/// Interrupt delivery mode from the I/O redirection table.
/// Bochs: `entry->delivery_mode()` returns `(lo >> 8) & 7` (ioapic.h:52)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IoApicDeliveryMode {
    /// Deliver to the INTR signal of all destination processors.
    Fixed = 0,
    /// Deliver to the processor with the lowest interrupt priority.
    LowPriority = 1,
    /// System Management Interrupt — vector field ignored.
    Smi = 2,
    /// Reserved.
    Reserved3 = 3,
    /// Non-Maskable Interrupt — vector field ignored.
    Nmi = 4,
    /// INIT level de-assert — vector field ignored.
    Init = 5,
    /// Reserved.
    Reserved6 = 6,
    /// External interrupt (PIC-compatible) — vector read from PIC via INTA cycle.
    /// Bochs: `if (entry->delivery_mode() == 7) vector = DEV_pic_iac()` (ioapic.cc:312)
    ExtInt = 7,
}

impl IoApicDeliveryMode {
    /// Convert a 3-bit raw value to a delivery mode.
    pub fn from_raw(val: u8) -> Self {
        match val & 0x07 {
            0 => Self::Fixed,
            1 => Self::LowPriority,
            2 => Self::Smi,
            3 => Self::Reserved3,
            4 => Self::Nmi,
            5 => Self::Init,
            6 => Self::Reserved6,
            7 => Self::ExtInt,
            _ => unreachable!(),
        }
    }
}

// ---------------------------------------------------------------------------
// I/O Redirection Table Entry (64-bit, split hi/lo)
// ---------------------------------------------------------------------------

/// A single I/O redirection table entry.
///
/// Each entry is 64 bits wide, stored as two 32-bit registers (lo and hi).
///
/// ## Low 32 bits (lo) layout:
/// - Bits [7:0]   — Interrupt vector
/// - Bits [10:8]  — Delivery mode (see [`IoApicDeliveryMode`])
/// - Bit  11      — Destination mode (0 = physical, 1 = logical)
/// - Bit  12      — Delivery status (read-only: 0 = idle, 1 = send pending)
/// - Bit  13      — Pin polarity (0 = active high, 1 = active low)
/// - Bit  14      — Remote IRR (read-only for level-triggered)
/// - Bit  15      — Trigger mode (0 = edge, 1 = level)
/// - Bit  16      — Mask (1 = masked/disabled)
///
/// ## High 32 bits (hi) layout:
/// - Bits [31:24] — Destination APIC ID (or logical destination)
///
/// Bochs: `class bx_io_redirect_entry_t` (ioapic.h:39-72)
#[derive(Debug, Clone, Copy)]
pub struct IoRedirectEntry {
    lo: u32,
    hi: u32,
}

impl Default for IoRedirectEntry {
    /// Reset state: masked, all other fields zero.
    /// Bochs: `bx_io_redirect_entry_t(): hi(0), lo(0x10000) {}` (ioapic.h:43)
    fn default() -> Self {
        Self {
            lo: REDIRECT_ENTRY_DEFAULT_LO,
            hi: 0,
        }
    }
}

impl IoRedirectEntry {
    // -- Accessors matching Bochs ioapic.h:45-53 --

    /// Destination APIC ID (bits [31:24] of hi word).
    /// Bochs: `Bit8u destination() const { return (Bit8u)(hi >> 24); }` (ioapic.h:45)
    pub fn destination(&self) -> u8 {
        (self.hi >> 24) as u8
    }

    /// Whether the entry is masked (bit 16 of lo word).
    /// Bochs: `bool is_masked() const { return (bool)((lo >> 16) & 1); }` (ioapic.h:46)
    pub fn is_masked(&self) -> bool {
        (self.lo >> 16) & 1 != 0
    }

    /// Trigger mode (0 = edge, 1 = level). Bit 15 of lo word.
    /// Bochs: `Bit8u trigger_mode() const { return (Bit8u)((lo >> 15) & 1); }` (ioapic.h:47)
    pub fn trigger_mode(&self) -> u8 {
        ((self.lo >> 15) & 1) as u8
    }

    /// Remote IRR flag (bit 14, read-only). Set when LAPIC accepts level-triggered
    /// interrupt; cleared when EOI is received.
    /// Bochs: `bool remote_irr() const { return (bool)((lo >> 14) & 1); }` (ioapic.h:48)
    pub fn remote_irr(&self) -> bool {
        (self.lo >> 14) & 1 != 0
    }

    /// Pin polarity (0 = active high, 1 = active low). Bit 13.
    /// Bochs: `Bit8u pin_polarity() const { return (Bit8u)((lo >> 13) & 1); }` (ioapic.h:49)
    pub fn pin_polarity(&self) -> u8 {
        ((self.lo >> 13) & 1) as u8
    }

    /// Delivery status (bit 12, read-only). 1 = send pending.
    /// Bochs: `bool delivery_status() const { return (bool)((lo >> 12) & 1); }` (ioapic.h:50)
    pub fn delivery_status(&self) -> bool {
        (self.lo >> 12) & 1 != 0
    }

    /// Destination mode (bit 11). 0 = physical, 1 = logical.
    /// Bochs: `Bit8u destination_mode() const { return (Bit8u)((lo >> 11) & 1); }` (ioapic.h:51)
    pub fn destination_mode(&self) -> u8 {
        ((self.lo >> 11) & 1) as u8
    }

    /// Delivery mode (bits [10:8]).
    /// Bochs: `Bit8u delivery_mode() const { return (Bit8u)((lo >> 8) & 7); }` (ioapic.h:52)
    pub fn delivery_mode(&self) -> u8 {
        ((self.lo >> 8) & 7) as u8
    }

    /// Interrupt vector (bits [7:0]).
    /// Bochs: `Bit8u vector() const { return (Bit8u)(lo & 0xff); }` (ioapic.h:53)
    pub fn vector(&self) -> u8 {
        (self.lo & 0xFF) as u8
    }

    // -- Mutators matching Bochs ioapic.h:55-69 --

    /// Set delivery status bit (bit 12).
    /// Bochs: `void set_delivery_status() { lo |= (1<<12); }` (ioapic.h:55)
    pub fn set_delivery_status(&mut self) {
        self.lo |= 1 << 12;
    }

    /// Clear delivery status bit (bit 12).
    /// Bochs: `void clear_delivery_status() { lo &= ~(1<<12); }` (ioapic.h:56)
    pub fn clear_delivery_status(&mut self) {
        self.lo &= !(1 << 12);
    }

    /// Set remote IRR bit (bit 14).
    /// Bochs: `void set_remote_irr() { lo |= (1<<14); }` (ioapic.h:57)
    pub fn set_remote_irr(&mut self) {
        self.lo |= 1 << 14;
    }

    /// Clear remote IRR bit (bit 14).
    /// Bochs: `void clear_remote_irr() { lo &= ~(1<<14); }` (ioapic.h:58)
    pub fn clear_remote_irr(&mut self) {
        self.lo &= !(1 << 14);
    }

    /// Get low 32-bit register value.
    /// Bochs: `Bit32u get_lo_part() const { return lo; }` (ioapic.h:60)
    pub fn get_lo_part(&self) -> u32 {
        self.lo
    }

    /// Get high 32-bit register value.
    /// Bochs: `Bit32u get_hi_part() const { return hi; }` (ioapic.h:61)
    pub fn get_hi_part(&self) -> u32 {
        self.hi
    }

    /// Write the low 32-bit register, masking read-only bits.
    /// Bochs: `void set_lo_part(Bit32u val) { lo = val & 0xffffafff; }` (ioapic.h:62-65)
    pub fn set_lo_part(&mut self, value: u32) {
        self.lo = value & REDIRECT_LO_WRITE_MASK;
    }

    /// Write the high 32-bit register (destination field).
    /// Bochs: `void set_hi_part(Bit32u val) { hi = val; }` (ioapic.h:66-69)
    pub fn set_hi_part(&mut self, value: u32) {
        self.hi = value;
    }
}

// ---------------------------------------------------------------------------
// I/O APIC Device
// ---------------------------------------------------------------------------

/// 82093AA I/O Advanced Programmable Interrupt Controller.
///
/// The I/O APIC provides multi-processor interrupt management by routing
/// external interrupt signals to one or more Local APICs. It contains a
/// 24-entry I/O Redirection Table that programs the routing for each
/// interrupt input pin.
///
/// Bochs: `class bx_ioapic_c` (ioapic.h:74-112)
#[derive(Debug)]
pub struct BxIoApic {
    /// Whether the I/O APIC MMIO is enabled.
    /// Bochs: `bool enabled` (ioapic.h:98)
    enabled: bool,

    /// Base MMIO address. Default 0xFEC00000.
    /// Bochs: `bx_phy_address base_addr` (ioapic.h:99)
    base_addr: u32,

    /// I/O APIC identification register (4-bit ID in bits [27:24] when read).
    /// Bochs: `Bit32u id` (ioapic.h:100)
    id: u32,

    /// I/O Register Select — selects which register to access via the data window.
    /// Bochs: `Bit32u ioregsel` (ioapic.h:102)
    ioregsel: u32,

    /// Input pin level state (1 bit per pin). Tracks the actual level of each pin.
    /// Bochs: `Bit32u intin` (ioapic.h:103)
    intin: u32,

    /// Interrupt Request Register. Bits are set when an interrupt is triggered
    /// on a pin, and cleared when the interrupt is delivered (edge) or EOI'd (level).
    /// Bochs: `Bit32u irr` (ioapic.h:109)
    irr: u32,

    /// I/O Redirection Table — 24 entries, one per interrupt input pin.
    /// Bochs: `bx_io_redirect_entry_t ioredtbl[BX_IOAPIC_NUM_PINS]` (ioapic.h:111)
    ioredtbl: [IoRedirectEntry; IOAPIC_NUM_PINS],

    /// Stuck interrupt delivery counter for diagnostics.
    /// Bochs: `static unsigned int stuck` (ioapic.cc:301) — moved to instance field.
    stuck_count: u32,

    /// Pending interrupt deliveries queued when LAPIC is not available (MMIO path).
    /// Drained by the emulator after each tick/sync cycle.
    pub(crate) pending_deliveries: [(u8, u8, u8); 8],
    pub(crate) num_pending_deliveries: usize,
}

impl Default for BxIoApic {
    fn default() -> Self {
        Self::new()
    }
}

impl BxIoApic {
    /// Create a new I/O APIC with default settings.
    ///
    /// Bochs: `bx_ioapic_c::bx_ioapic_c()` (ioapic.cc:124-128)
    pub fn new() -> Self {
        Self {
            enabled: false,
            base_addr: IOAPIC_BASE_ADDR,
            id: IOAPIC_DEFAULT_ID,
            ioregsel: 0,
            intin: 0,
            irr: 0,
            ioredtbl: [IoRedirectEntry::default(); IOAPIC_NUM_PINS],
            stuck_count: 0,
            pending_deliveries: [(0, 0, 0); 8],
            num_pending_deliveries: 0,
        }
    }

    /// Initialize the I/O APIC and register its MMIO handlers.
    ///
    /// Bochs: `bx_ioapic_c::init()` (ioapic.cc:136-144)
    ///
    /// This registers memory handlers for the MMIO range at `base_addr`.
    /// The `mem` parameter is the memory subsystem used for handler registration.
    pub fn init(&mut self, mem: &mut BxMemC) -> crate::Result<()> {
        tracing::info!("initializing I/O APIC");
        // Bochs: set_enabled(1, 0x0000) (ioapic.cc:139)
        self.set_enabled_with_mem(true, 0x0000, mem)?;
        Ok(())
    }


    /// Reset all I/O APIC state.
    /// All redirection entries are masked, IRR and input state are cleared.
    ///
    /// Bochs: `bx_ioapic_c::reset(unsigned type)` (ioapic.cc:146-156)
    pub fn reset(&mut self) {
        // All interrupts masked
        // Bochs: for (int i=0; i<BX_IOAPIC_NUM_PINS; i++) { ... } (ioapic.cc:149-152)
        for entry in &mut self.ioredtbl {
            entry.set_lo_part(REDIRECT_ENTRY_DEFAULT_LO);
            entry.set_hi_part(0x0000_0000);
        }
        // Bochs: intin = 0; irr = 0; ioregsel = 0; (ioapic.cc:153-155)
        self.intin = 0;
        self.irr = 0;
        self.ioregsel = 0;
        self.stuck_count = 0;
    }

    /// Get redirect entry state for diagnostics.
    pub fn redirect_entry_diag(&self, pin: usize) -> (u8, bool, u8, u8) {
        let e = &self.ioredtbl[pin];
        (e.vector(), e.is_masked(), e.trigger_mode(), e.delivery_mode())
    }

    /// Get the intin and irr state for a pin.
    pub fn pin_state(&self, pin: u8) -> (bool, bool) {
        let bit = 1u32 << pin;
        (self.intin & bit != 0, self.irr & bit != 0)
    }

    /// Read an aligned 32-bit register.
    ///
    /// Address is masked to 8-bit offset within the MMIO page.
    /// Offset 0x00 reads IOREGSEL; offset 0x10 reads the register selected by IOREGSEL.
    ///
    /// Bochs: `Bit32u bx_ioapic_c::read_aligned(bx_phy_address address)` (ioapic.cc:158-194)
    pub fn read_aligned(&self, address: BxPhyAddress) -> u32 {
        let offset = (address as u32) & 0xFF;
        tracing::debug!(
            "IOAPIC: read aligned addr={:#010x} offset={:#04x}",
            address,
            offset
        );

        if offset == 0x00 {
            // Select register — return current IOREGSEL value
            // Bochs: return ioregsel; (ioapic.cc:164)
            return self.ioregsel;
        }

        if offset != 0x10 {
            // Bochs: BX_PANIC(("IOAPIC: read from unsupported address")); (ioapic.cc:167)
            tracing::error!("IOAPIC: read from unsupported MMIO offset {:#04x}", offset);
            return 0;
        }

        // Data register read — dispatch based on IOREGSEL
        // Bochs: switch (ioregsel) { ... } (ioapic.cc:173-191)
        match self.ioregsel {
            IOREGSEL_ID => {
                // APIC ID register: ID in bits [27:24]
                // Bochs: data = ((id & apic_id_mask) << 24); (ioapic.cc:175)
                (self.id & APIC_ID_MASK) << 24
            }
            IOREGSEL_VERSION => {
                // Version register
                // Bochs: data = BX_IOAPIC_VERSION_ID; (ioapic.cc:178)
                IOAPIC_VERSION_ID
            }
            IOREGSEL_ARB_ID => {
                // Arbitration ID — not meaningfully implemented in Bochs
                // Bochs: BX_INFO(("IOAPIC: arbitration ID unsupported, returned 0")); (ioapic.cc:181)
                tracing::info!("IOAPIC: arbitration ID unsupported, returned 0");
                0
            }
            _ => {
                // Redirection table entry access
                // Bochs: int index = (ioregsel - 0x10) >> 1; (ioapic.cc:184)
                let raw_index = self.ioregsel.wrapping_sub(IOREGSEL_REDTBL_BASE) >> 1;
                if (raw_index as usize) < IOAPIC_NUM_PINS {
                    let entry = &self.ioredtbl[raw_index as usize];
                    // Odd IOREGSEL reads hi part, even reads lo part
                    // Bochs: data = (ioregsel&1) ? entry->get_hi_part() : entry->get_lo_part();
                    // (ioapic.cc:187)
                    if self.ioregsel & 1 != 0 {
                        entry.get_hi_part()
                    } else {
                        entry.get_lo_part()
                    }
                } else {
                    tracing::error!(
                        "IOAPIC: IOREGSEL points to undefined register {:#04x}",
                        self.ioregsel
                    );
                    0
                }
            }
        }
    }

    /// Write an aligned 32-bit value to a register.
    ///
    /// Address is masked to 8-bit offset within the MMIO page.
    /// Offset 0x00 writes IOREGSEL; offset 0x10 writes the register selected by IOREGSEL.
    /// After writing a redirection table entry, `service_ioapic()` is called to
    /// check for newly unmasked interrupts.
    ///
    /// Bochs: `void bx_ioapic_c::write_aligned(bx_phy_address address, Bit32u value)`
    /// (ioapic.cc:196-236)
    pub fn write_aligned(
        &mut self,
        address: BxPhyAddress,
        value: u32,
        pic: Option<&mut super::pic::BxPicC>,
        #[cfg(feature = "bx_support_apic")]
        lapic: Option<&mut crate::cpu::apic::BxLocalApic>,
    ) {
        let offset = (address as u32) & 0xFF;
        tracing::debug!(
            "IOAPIC: write aligned addr={:#010x} offset={:#04x} data={:#010x}",
            address,
            offset,
            value
        );

        if offset == 0x00 {
            // Write to IOREGSEL
            // Bochs: ioregsel = value; return; (ioapic.cc:201)
            self.ioregsel = value;
            return;
        }

        if offset != 0x10 {
            // Bochs: BX_PANIC(("IOAPIC: write to unsupported address")); (ioapic.cc:205)
            tracing::error!("IOAPIC: write to unsupported MMIO offset {:#04x}", offset);
            return;
        }

        // Data register write — dispatch based on IOREGSEL
        // Bochs: switch (ioregsel) { ... } (ioapic.cc:208-235)
        match self.ioregsel {
            IOREGSEL_ID => {
                // Set APIC ID from bits [27:24]
                // Bochs: Bit8u newid = (value >> 24) & apic_id_mask; (ioapic.cc:211)
                let new_id = (value >> 24) & APIC_ID_MASK;
                tracing::info!("IOAPIC: setting id to {:#x}", new_id);
                self.id = new_id;
            }
            IOREGSEL_VERSION | IOREGSEL_ARB_ID => {
                // Version and arbitration ID are read-only
                // Bochs: BX_INFO(("IOAPIC: could not write, IOREGSEL=0x%02x", ioregsel));
                // (ioapic.cc:218)
                tracing::info!(
                    "IOAPIC: could not write, IOREGSEL={:#04x} (read-only)",
                    self.ioregsel
                );
            }
            _ => {
                // Redirection table entry access
                // Bochs: int index = (ioregsel - 0x10) >> 1; (ioapic.cc:221)
                let raw_index = self.ioregsel.wrapping_sub(IOREGSEL_REDTBL_BASE) >> 1;
                if (raw_index as usize) < IOAPIC_NUM_PINS {
                    let entry = &mut self.ioredtbl[raw_index as usize];
                    // Bochs: (ioapic.cc:224-226)
                    if self.ioregsel & 1 != 0 {
                        entry.set_hi_part(value);
                    } else {
                        entry.set_lo_part(value);
                    }
                    tracing::debug!(
                        "IOAPIC: entry[{}]: dest={:#04x} masked={} trig={} remote_irr={} \
                         polarity={} deliv_status={} dest_mode={} deliv_mode={} vector={:#04x}",
                        raw_index,
                        entry.destination(),
                        entry.is_masked() as u8,
                        entry.trigger_mode(),
                        entry.remote_irr() as u8,
                        entry.pin_polarity(),
                        entry.delivery_status() as u8,
                        entry.destination_mode(),
                        entry.delivery_mode(),
                        entry.vector(),
                    );
                    // Bochs: service_ioapic(); (ioapic.cc:231)
                    self.service_ioapic(
                        pic,
                        #[cfg(feature = "bx_support_apic")]
                        lapic,
                    );
                } else {
                    tracing::error!(
                        "IOAPIC: IOREGSEL points to undefined register {:#04x}",
                        self.ioregsel
                    );
                }
            }
        }
    }

    /// Enable or disable the I/O APIC MMIO region, optionally changing the base offset.
    ///
    /// When enabled, MMIO handlers are registered at `IOAPIC_BASE_ADDR | base_offset`.
    /// When disabled, MMIO handlers are unregistered.
    ///
    /// Bochs: `void bx_ioapic_c::set_enabled(bool _enabled, Bit16u base_offset)`
    /// (ioapic.cc:238-256)
    fn set_enabled_with_mem(
        &mut self,
        new_enabled: bool,
        base_offset: u16,
        mem: &mut BxMemC,
    ) -> crate::Result<()> {
        if new_enabled != self.enabled {
            if new_enabled {
                // Bochs: base_addr = BX_IOAPIC_BASE_ADDR | base_offset; (ioapic.cc:242)
                self.base_addr = IOAPIC_BASE_ADDR | (base_offset as u32);
                // Register MMIO handlers
                // Bochs: DEV_register_memory_handlers(..., base_addr, base_addr + 0xfff);
                // (ioapic.cc:243-244)
                let base = self.base_addr as BxPhyAddress;
                let device_id = crate::memory::MemoryDeviceId::IoApic(self as *mut BxIoApic);
                mem.register_memory_handlers(
                    device_id,
                    base,
                    base + (IOAPIC_MMIO_SIZE as BxPhyAddress) - 1,
                )?;
            }
            // Note: unregister_memory_handlers not yet implemented; on disable we just
            // mark the flag. Bochs: DEV_unregister_memory_handlers(...) (ioapic.cc:246)
            self.enabled = new_enabled;
        } else if self.enabled && (base_offset as u32 != (self.base_addr & 0xFFFF)) {
            // Base offset changed while enabled — re-register at new address
            // Bochs: (ioapic.cc:249-253)
            self.base_addr = IOAPIC_BASE_ADDR | (base_offset as u32);
            let base = self.base_addr as BxPhyAddress;
            let device_id = crate::memory::MemoryDeviceId::IoApic(self as *mut BxIoApic);
            mem.register_memory_handlers(
                device_id,
                base,
                base + (IOAPIC_MMIO_SIZE as BxPhyAddress) - 1,
            )?;
        }

        tracing::info!(
            "IOAPIC {}abled (base address = {:#010x})",
            if self.enabled { "en" } else { "dis" },
            self.base_addr,
        );
        Ok(())
    }

    /// Set the interrupt level on an input pin.
    ///
    /// IRQ 0 (system timer) is remapped to pin 2, matching the ISA-to-APIC mapping
    /// used by most chipsets and by Bochs.
    ///
    /// For **level-triggered** pins: asserting sets both `intin` and `irr`, deasserting
    /// clears both. Delivery is attempted on assert.
    ///
    /// For **edge-triggered** pins: a rising edge (0→1) sets `intin` and, if unmasked,
    /// also sets `irr` and attempts delivery. A falling edge only clears `intin`.
    ///
    /// Bochs: `void bx_ioapic_c::set_irq_level(Bit8u int_in, bool level)` (ioapic.cc:258-292)
    ///
    /// `pic` and `lapic` are threaded through to `service_ioapic()` for
    /// ExtINT vector lookup and LAPIC delivery respectively.
    pub fn set_irq_level(
        &mut self,
        mut int_in: u8,
        level: bool,
        pic: Option<&mut super::pic::BxPicC>,
        #[cfg(feature = "bx_support_apic")]
        lapic: Option<&mut crate::cpu::apic::BxLocalApic>,
    ) {
        // Bochs: if (int_in == 0) int_in = 2; // timer connected to pin #2 (ioapic.cc:260-262)
        if int_in == 0 {
            int_in = 2;
        }

        if (int_in as usize) >= IOAPIC_NUM_PINS {
            return;
        }

        let bit: u32 = 1 << int_in;
        let level_bit = if level { bit } else { 0 };

        // Only act on a change in pin level
        // Bochs: if (((Bit32u)level<<int_in) != (intin & bit)) { ... } (ioapic.cc:265)
        if level_bit != (self.intin & bit) {
            tracing::debug!(
                "IOAPIC: set_irq_level(): INTIN{}: level={}",
                int_in,
                level as u8
            );

            let entry = &self.ioredtbl[int_in as usize];
            if entry.trigger_mode() != 0 {
                // Level triggered
                // Bochs: (ioapic.cc:268-277)
                if level {
                    self.intin |= bit;
                    self.irr |= bit;
                    self.service_ioapic(
                        pic,
                        #[cfg(feature = "bx_support_apic")]
                        lapic,
                    );
                } else {
                    self.intin &= !bit;
                    self.irr &= !bit;
                }
            } else {
                // Edge triggered
                // Bochs: (ioapic.cc:278-289)
                if level {
                    self.intin |= bit;
                    if !entry.is_masked() {
                        self.irr |= bit;
                        self.service_ioapic(
                            pic,
                            #[cfg(feature = "bx_support_apic")]
                            lapic,
                        );
                    }
                } else {
                    self.intin &= !bit;
                }
            }
        }
    }

    /// Receive End-of-Interrupt for a specific vector.
    ///
    /// Called by the Local APIC when it receives an EOI for a level-triggered interrupt.
    /// In Bochs, this currently only logs.
    ///
    /// Bochs: `void bx_ioapic_c::receive_eoi(Bit8u vector)` (ioapic.cc:294-297)
    pub fn receive_eoi(&mut self, vector: u8) {
        tracing::debug!("IOAPIC: received EOI for vector {}", vector);
        // In a full implementation, we would clear the remote_irr bit for
        // any redirect table entry whose vector matches and that is level-triggered.
        // Bochs doesn't do this either — it just logs.
    }

    /// Scan the IRR for unmasked interrupts and attempt delivery via the APIC bus.
    ///
    /// For each unmasked pin with a pending interrupt (IRR bit set):
    /// - ExtINT mode (delivery_mode == 7): reads vector from PIC via INTA cycle.
    /// - All other modes: uses the vector from the redirection entry.
    ///
    /// If delivery succeeds:
    /// - Edge-triggered: IRR bit is cleared.
    /// - Level-triggered: IRR bit remains set (cleared by EOI).
    /// - Delivery status is cleared.
    ///
    /// If delivery fails, delivery status is set and a stuck counter increments.
    ///
    /// Bochs: `void bx_ioapic_c::service_ioapic()` (ioapic.cc:299-334)
    ///
    /// `pic` is needed for ExtINT delivery mode (calls `pic.iac()`).
    /// `lapic` is needed for direct LAPIC interrupt delivery.
    /// Either may be `None`; fallback paths handle the missing dependency.
    fn service_ioapic(
        &mut self,
        mut pic: Option<&mut super::pic::BxPicC>,
        #[cfg(feature = "bx_support_apic")]
        mut lapic: Option<&mut crate::cpu::apic::BxLocalApic>,
    ) {
        tracing::debug!("IOAPIC: servicing (irr={:#010x})", self.irr);

        for pin in 0..IOAPIC_NUM_PINS {
            let mask: u32 = 1 << pin;
            if self.irr & mask == 0 {
                continue;
            }

            let entry = &self.ioredtbl[pin];
            if entry.is_masked() {
                tracing::debug!("IOAPIC: service_ioapic(): INTIN{} is masked", pin);
                continue;
            }

            // Determine vector
            // Bochs: if (entry->delivery_mode() == 7) vector = DEV_pic_iac();
            // else vector = entry->vector(); (ioapic.cc:311-315)
            let vector = if entry.delivery_mode() == IoApicDeliveryMode::ExtInt as u8 {
                // ExtINT: Bochs calls DEV_pic_iac() for the vector (ioapic.cc:312).
                if let Some(pic) = pic.as_deref_mut() {
                    let v = pic.iac();
                    tracing::debug!(
                        "IOAPIC: ExtINT mode on pin {} — PIC IAC vector {:#04x}",
                        pin,
                        v
                    );
                    v
                } else {
                    // Fallback: no PIC reference, use entry vector
                    tracing::debug!(
                        "IOAPIC: ExtINT mode on pin {} — no PIC, using entry vector {:#04x}",
                        pin,
                        entry.vector()
                    );
                    entry.vector()
                }
            } else {
                entry.vector()
            };

            // Attempt delivery via APIC bus → Local APIC
            #[cfg(feature = "bx_support_apic")]
            let done = if let Some(lapic) = lapic.as_deref_mut() {
                // Single-CPU: deliver directly to the LAPIC.
                let trigger = entry.trigger_mode();
                lapic.deliver(vector, entry.delivery_mode(), trigger);
                true
            } else {
                // No LAPIC available (MMIO path) — enqueue for later delivery
                let trigger = entry.trigger_mode();
                self.enqueue_delivery(vector, entry.delivery_mode(), trigger);
                true
            };
            #[cfg(not(feature = "bx_support_apic"))]
            let done = {
                let trigger = entry.trigger_mode();
                self.enqueue_delivery(vector, entry.delivery_mode(), trigger);
                true
            };

            // Bochs: (ioapic.cc:317-327)
            let entry = &mut self.ioredtbl[pin];
            if done {
                // Edge-triggered: clear IRR; level-triggered: keep IRR set
                if entry.trigger_mode() == 0 {
                    self.irr &= !mask;
                }
                entry.clear_delivery_status();
                self.stuck_count = 0;
            } else {
                entry.set_delivery_status();
                self.stuck_count += 1;
                if self.stuck_count > 5 {
                    tracing::info!("IOAPIC: vector {:#04x} stuck?", vector);
                }
            }
        }
    }

    /// Get a reference to a redirection table entry (for diagnostics).
    pub fn redirect_entry(&self, index: usize) -> Option<&IoRedirectEntry> {
        self.ioredtbl.get(index)
    }

    /// Enqueue an interrupt delivery for later drain by the emulator.
    fn enqueue_delivery(&mut self, vector: u8, delivery_mode: u8, trigger_mode: u8) {
        if self.num_pending_deliveries < self.pending_deliveries.len() {
            self.pending_deliveries[self.num_pending_deliveries] = (vector, delivery_mode, trigger_mode);
            self.num_pending_deliveries += 1;
        }
    }

    /// Take all pending deliveries, resetting the queue.
    pub(crate) fn take_pending_deliveries(&mut self) -> ([(u8, u8, u8); 8], usize) {
        let result = (self.pending_deliveries, self.num_pending_deliveries);
        self.num_pending_deliveries = 0;
        result
    }

    /// Dump IOAPIC state for HLT diagnostics.
    pub fn dump_hlt_state(&self) {
        tracing::debug!("[HLT-STATE] IOAPIC: irr={:#010x} intin={:#010x}", self.irr, self.intin);
        for pin in 0..IOAPIC_NUM_PINS {
            let entry = &self.ioredtbl[pin];
            // Only show non-default entries (unmasked or configured)
            if !entry.is_masked() || entry.vector() != 0 {
                tracing::debug!("[HLT-STATE]   pin {:2}: lo={:#010x} hi={:#010x} vec={:#04x} masked={} trigger={} deliv={}",
                    pin, entry.get_lo_part(), entry.get_hi_part(), entry.vector(),
                    entry.is_masked(), entry.trigger_mode(), entry.delivery_mode());
            }
        }
    }

    /// Get the I/O APIC ID.
    pub fn apic_id(&self) -> u32 {
        self.id
    }

    /// Get the base MMIO address.
    pub fn base_address(&self) -> u32 {
        self.base_addr
    }

    /// Get whether the I/O APIC is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the current IRR value (for diagnostics).
    pub fn irr_value(&self) -> u32 {
        self.irr
    }

    /// Get the current INTIN value (for diagnostics).
    pub fn intin_value(&self) -> u32 {
        self.intin
    }
}

// ---------------------------------------------------------------------------
// APIC Bus Delivery Stub
// ---------------------------------------------------------------------------

/// Deliver an interrupt via the APIC bus to the target Local APIC(s).
///
/// Bochs: `apic_bus_deliver_interrupt(vector, dest, delivery_mode, logical_dest, level, trig_mode)`
/// (declared in ioapic.h:31, implemented in cpu/apic.cc:30-120)
///
/// This is the fallback path used when `bx_support_apic` is disabled or when
/// no LAPIC reference is provided. The primary code path (when APIC is enabled)
/// delivers directly to the LAPIC via the `lapic` parameter passed to
/// `service_ioapic()`. For a single-CPU emulator, that direct path handles all
/// destination modes — physical dest=0 or logical with flat model always matches
/// the single LAPIC.
///
/// Returns `true` if the interrupt was accepted by at least one LAPIC.
fn apic_bus_deliver_interrupt(
    vector: u8,
    dest: u8,
    delivery_mode: u8,
    dest_mode: u8,
    _pin_polarity: u8,
    trigger_mode: u8,
) -> bool {
    let mode = IoApicDeliveryMode::from_raw(delivery_mode);
    tracing::debug!(
        "APIC bus fallback: deliver vector={:#04x} dest={:#04x} mode={:?} dest_mode={} trigger={}",
        vector,
        dest,
        mode,
        dest_mode,
        trigger_mode,
    );

    // In single-CPU mode without LAPIC pointer, accept all interrupts so the
    // IOAPIC doesn't stall. The PIC path handles the actual CPU delivery.
    true
}

// ---------------------------------------------------------------------------
// MMIO Handler Functions
// ---------------------------------------------------------------------------

/// MMIO read handler for the I/O APIC.
///
/// Handles partial reads (1, 2, or 4 bytes) by reading the aligned 32-bit
/// register and shifting/masking as needed.
///
/// Bochs: `static bool ioapic_read(bx_phy_address a20addr, unsigned len, void *data, void *param)`
/// (ioapic.cc:53-74)
impl BxIoApic {
    pub(crate) fn mem_read(&self, addr: BxPhyAddress, len: u32, data: &mut [u8]) -> bool {
        // Check that access doesn't span a 32-bit boundary
        // Bochs: if((a20addr & ~0x3) != ((a20addr+len-1) & ~0x3)) (ioapic.cc:55)
        if (addr & !0x3) != ((addr + len as u64 - 1) & !0x3) {
            tracing::error!(
                "IOAPIC: read at address {:#x} spans 32-bit boundary (len={})",
                addr,
                len
            );
            return true;
        }

        let value = self.read_aligned(addr & !0x3);

        // Write result to caller's data buffer (Bochs ioapic.cc:60-71)
        match len {
            4 => {
                data[..4].copy_from_slice(&value.to_ne_bytes());
            }
            2 => {
                let shifted = value >> ((addr & 3) * 8) as u32;
                data[..2].copy_from_slice(&(shifted as u16).to_ne_bytes());
            }
            1 => {
                let shifted = value >> ((addr & 3) * 8) as u32;
                data[0] = (shifted & 0xFF) as u8;
            }
            _ => {
                tracing::error!("IOAPIC: unsupported read len={} at addr={:#x}", len, addr);
            }
        }
        true
    }

    /// MMIO write handler for the I/O APIC.
    ///
    /// Writes must be 16-byte aligned. Non-4-byte writes are zero-extended to 32 bits
    /// when writing to IOREGSEL (offset 0x00).
    ///
    /// Bochs: `static bool ioapic_write(bx_phy_address a20addr, unsigned len, void *data, void *param)`
    /// (ioapic.cc:76-99)
    pub(crate) fn mem_write(&mut self, addr: BxPhyAddress, len: u32, data: &[u8]) -> bool {
        // Bochs: if(a20addr & 0xf) { BX_PANIC(...); return 1; } (ioapic.cc:78-81)
        if addr & 0xF != 0 {
            tracing::error!("IOAPIC: write at unaligned address {:#x}", addr);
            return true;
        }

        // Bochs: (ioapic.cc:83-96)
        if len == 4 {
            let value = u32::from_ne_bytes(
                data[..4].try_into().expect("IOAPIC write: data too short for 4-byte access"),
            );
            self.write_aligned(
                addr,
                value,
                None, // no PIC available in MMIO callback
                #[cfg(feature = "bx_support_apic")]
                None, // no LAPIC available in MMIO callback
            );
        } else {
            // Non-4-byte writes: only accepted at IOREGSEL offset (0x00)
            let data_offset = (addr & 0xFF) as u32;
            if data_offset != 0 {
                tracing::error!(
                    "IOAPIC: write with len={} (should be 4) at address {:#x}",
                    len,
                    addr
                );
                return true;
            }

            let value = match len {
                2 => u16::from_ne_bytes(
                    data[..2].try_into().expect("IOAPIC write: data too short for 2-byte access"),
                ) as u32,
                1 => data[0] as u32,
                _ => {
                    tracing::error!("IOAPIC: unsupported write len={} at addr={:#x}", len, addr);
                    return true;
                }
            };
            self.write_aligned(
                addr,
                value,
                None, // no PIC available in MMIO callback
                #[cfg(feature = "bx_support_apic")]
                None, // no LAPIC available in MMIO callback
            );
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Debug dump
// ---------------------------------------------------------------------------

impl BxIoApic {
    /// Generate a diagnostic dump of all redirection table entries.
    ///
    /// Bochs: `void bx_ioapic_c::debug_dump(int argc, char **argv)` (ioapic.cc:354-367)
    pub fn debug_dump(&self) -> alloc::string::String {
        use alloc::format;
        use alloc::string::String;

        let mut out = String::from("82093AA I/O APIC\n\n");
        for i in 0..IOAPIC_NUM_PINS {
            let entry = &self.ioredtbl[i];
            out.push_str(&format!(
                "entry[{:2}]: dest={:#04x} masked={} trig_mode={} remote_irr={} \
                 polarity={} deliv_status={} dest_mode={} deliv_mode={} vector={:#04x}\n",
                i,
                entry.destination(),
                entry.is_masked() as u8,
                entry.trigger_mode(),
                entry.remote_irr() as u8,
                entry.pin_polarity(),
                entry.delivery_status() as u8,
                entry.destination_mode(),
                entry.delivery_mode(),
                entry.vector(),
            ));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redirect_entry_defaults() {
        let entry = IoRedirectEntry::default();
        assert!(entry.is_masked());
        assert_eq!(entry.vector(), 0);
        assert_eq!(entry.delivery_mode(), 0);
        assert_eq!(entry.destination(), 0);
        assert_eq!(entry.trigger_mode(), 0);
        assert!(!entry.remote_irr());
        assert!(!entry.delivery_status());
    }

    #[test]
    fn test_redirect_entry_accessors() {
        let mut entry = IoRedirectEntry::default();

        // Write a full configuration: vector=0x42, fixed mode, physical dest,
        // edge-triggered, active high, unmasked, destination ID=5
        entry.set_lo_part(0x0000_0042); // vector=0x42, unmasked (bit 16=0)
        entry.set_hi_part(0x0500_0000); // dest ID = 5

        assert_eq!(entry.vector(), 0x42);
        assert_eq!(entry.delivery_mode(), 0); // Fixed
        assert_eq!(entry.destination_mode(), 0); // Physical
        assert_eq!(entry.trigger_mode(), 0); // Edge
        assert_eq!(entry.pin_polarity(), 0); // Active high
        assert!(!entry.is_masked());
        assert_eq!(entry.destination(), 5);
    }

    #[test]
    fn test_redirect_entry_write_mask() {
        let mut entry = IoRedirectEntry::default();

        // Try to write delivery_status (bit 12) and remote_irr (bit 14) — should be masked
        entry.set_lo_part(0x0001_5042); // bits 12 and 14 set
        assert!(!entry.delivery_status()); // Bit 12 masked by 0xffffafff
        assert!(!entry.remote_irr()); // Bit 14 masked by 0xffffafff
        assert!(entry.is_masked()); // Bit 16 preserved
    }

    #[test]
    fn test_ioapic_new() {
        let ioapic = BxIoApic::new();
        assert!(!ioapic.enabled);
        assert_eq!(ioapic.base_addr, IOAPIC_BASE_ADDR);
        assert_eq!(ioapic.id, IOAPIC_DEFAULT_ID);
        assert_eq!(ioapic.ioregsel, 0);
        assert_eq!(ioapic.intin, 0);
        assert_eq!(ioapic.irr, 0);

        // All entries should be masked by default
        for entry in &ioapic.ioredtbl {
            assert!(entry.is_masked());
        }
    }

    #[test]
    fn test_ioapic_reset() {
        let mut ioapic = BxIoApic::new();
        ioapic.ioregsel = 0x42;
        ioapic.intin = 0xFF;
        ioapic.irr = 0xFF;
        ioapic.ioredtbl[0].set_lo_part(0x00000042);

        ioapic.reset();

        assert_eq!(ioapic.ioregsel, 0);
        assert_eq!(ioapic.intin, 0);
        assert_eq!(ioapic.irr, 0);
        assert!(ioapic.ioredtbl[0].is_masked());
    }

    #[test]
    fn test_ioapic_read_id() {
        let ioapic = BxIoApic::new();
        // Select APIC ID register
        let mut ioapic = ioapic;
        ioapic.ioregsel = IOREGSEL_ID;
        let value = ioapic.read_aligned(0xFEC00010);
        // ID = 1, masked to 4 bits, shifted to bits [27:24]
        assert_eq!(value, (IOAPIC_DEFAULT_ID & APIC_ID_MASK) << 24);
    }

    #[test]
    fn test_ioapic_read_version() {
        let mut ioapic = BxIoApic::new();
        ioapic.ioregsel = IOREGSEL_VERSION;
        let value = ioapic.read_aligned(0xFEC00010);
        assert_eq!(value, IOAPIC_VERSION_ID);
        assert_eq!(value, 0x00170011);
    }

    #[test]
    fn test_ioapic_write_read_redirect() {
        let mut ioapic = BxIoApic::new();

        // Write low word of entry 0 (IOREGSEL = 0x10)
        ioapic.ioregsel = 0x10;
        ioapic.write_aligned(0xFEC00010, 0x00000042, None, #[cfg(feature = "bx_support_apic")] None); // vector=0x42, unmasked

        // Read it back
        ioapic.ioregsel = 0x10;
        let lo = ioapic.read_aligned(0xFEC00010);
        assert_eq!(lo & 0xFF, 0x42);
        assert_eq!(lo & 0x10000, 0); // unmasked

        // Write high word of entry 0 (IOREGSEL = 0x11)
        ioapic.ioregsel = 0x11;
        ioapic.write_aligned(0xFEC00010, 0x03000000, None, #[cfg(feature = "bx_support_apic")] None); // dest = 3

        // Read it back
        ioapic.ioregsel = 0x11;
        let hi = ioapic.read_aligned(0xFEC00010);
        assert_eq!(hi, 0x03000000);
    }

    #[test]
    fn test_irq0_remapped_to_pin2() {
        let mut ioapic = BxIoApic::new();
        // Unmask pin 2 (edge-triggered)
        ioapic.ioredtbl[2].set_lo_part(0x00000020); // vector=0x20, unmasked

        // Assert IRQ 0 — should be remapped to pin 2
        ioapic.set_irq_level(0, true, None, #[cfg(feature = "bx_support_apic")] None);
        assert_eq!(ioapic.intin & (1 << 2), 1 << 2);
        // IRR is cleared because service_ioapic() delivered successfully (edge-triggered)
        assert_eq!(ioapic.irr & (1 << 2), 0);

        // Deassert
        ioapic.set_irq_level(0, false, None, #[cfg(feature = "bx_support_apic")] None);
        assert_eq!(ioapic.intin & (1 << 2), 0);
    }

    #[test]
    fn test_edge_triggered_irq() {
        let mut ioapic = BxIoApic::new();
        // Unmask pin 5, edge-triggered (bit 15 = 0), vector=0x25
        ioapic.ioredtbl[5].set_lo_part(0x00000025);

        // Rising edge triggers interrupt — delivery succeeds immediately (stub)
        // so IRR is cleared for edge-triggered entries after service_ioapic().
        ioapic.set_irq_level(5, true, None, #[cfg(feature = "bx_support_apic")] None);
        assert_ne!(ioapic.intin & (1 << 5), 0);
        // IRR was cleared by service_ioapic (delivery succeeded via stub)
        assert_eq!(ioapic.irr & (1 << 5), 0);

        // Falling edge clears input
        ioapic.set_irq_level(5, false, None, #[cfg(feature = "bx_support_apic")] None);
        assert_eq!(ioapic.intin & (1 << 5), 0);
    }

    #[test]
    fn test_level_triggered_irq() {
        let mut ioapic = BxIoApic::new();
        // Unmask pin 10, level-triggered (bit 15 = 1), vector=0x2A
        ioapic.ioredtbl[10].set_lo_part(0x0000802A); // bit 15 set

        // Assert level
        ioapic.set_irq_level(10, true, None, #[cfg(feature = "bx_support_apic")] None);
        assert_ne!(ioapic.intin & (1 << 10), 0);
        assert_ne!(ioapic.irr & (1 << 10), 0);

        // Deassert level — both intin and irr cleared
        ioapic.set_irq_level(10, false, None, #[cfg(feature = "bx_support_apic")] None);
        assert_eq!(ioapic.intin & (1 << 10), 0);
        assert_eq!(ioapic.irr & (1 << 10), 0);
    }

    #[test]
    fn test_masked_edge_no_irr() {
        let mut ioapic = BxIoApic::new();
        // Pin 3 is masked (default), edge-triggered
        assert!(ioapic.ioredtbl[3].is_masked());

        // Rising edge sets intin but NOT irr (because masked)
        ioapic.set_irq_level(3, true, None, #[cfg(feature = "bx_support_apic")] None);
        assert_ne!(ioapic.intin & (1 << 3), 0);
        assert_eq!(ioapic.irr & (1 << 3), 0); // Not set because masked
    }

    #[test]
    fn test_delivery_mode_enum() {
        assert_eq!(IoApicDeliveryMode::from_raw(0), IoApicDeliveryMode::Fixed);
        assert_eq!(
            IoApicDeliveryMode::from_raw(1),
            IoApicDeliveryMode::LowPriority
        );
        assert_eq!(IoApicDeliveryMode::from_raw(4), IoApicDeliveryMode::Nmi);
        assert_eq!(IoApicDeliveryMode::from_raw(7), IoApicDeliveryMode::ExtInt);
    }
}
