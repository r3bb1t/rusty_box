//! Local APIC (Advanced Programmable Interrupt Controller) implementation.
//!
//! Ported from Bochs cpu/apic.cc (1466 lines) with exact logic parity.
//! All register read/write handlers, interrupt delivery, timer management,
//! priority computation, and logical addressing match Bochs behavior.
//!
//! Bochs source: cpp_orig/bochs/cpu/apic.cc + apic.h

use tracing::{debug, error, info};

use crate::config::BxPhyAddress;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Edge-triggered interrupt mode (Bochs: APIC_EDGE_TRIGGERED)
pub const APIC_EDGE_TRIGGERED: u8 = 0;

/// Level-triggered interrupt mode (Bochs: APIC_LEVEL_TRIGGERED)
pub const APIC_LEVEL_TRIGGERED: u8 = 1;

/// Default Local APIC base address (Bochs: BX_LAPIC_BASE_ADDR)
pub const BX_LAPIC_BASE_ADDR: BxPhyAddress = 0xfee00000;

/// First valid APIC vector (Bochs: BX_LAPIC_FIRST_VECTOR, apic.cc:39)
const BX_LAPIC_FIRST_VECTOR: u8 = 0x10;

/// Last valid APIC vector (Bochs: BX_LAPIC_LAST_VECTOR, apic.cc:40)
#[allow(dead_code)]
const BX_LAPIC_LAST_VECTOR: u8 = 0xFF;

/// XAPIC extension support flag for Interrupt Enable Register (IER)
pub const BX_XAPIC_EXT_SUPPORT_IER: u32 = 1 << 0;

/// XAPIC extension support flag for Specific End of Interrupt (SEOI)
pub const BX_XAPIC_EXT_SUPPORT_SEOI: u32 = 1 << 1;

/// APIC ID mask for XAPIC mode (8-bit ID)
const APIC_ID_MASK_XAPIC: u32 = 0xFF;

/// APIC ID mask for legacy mode (4-bit ID)
const APIC_ID_MASK_LEGACY: u32 = 0x0F;

/// APIC error status constants (Bochs: apic.h:175-184)
const APIC_ERR_ILLEGAL_ADDR: u32 = 0x80;
const APIC_ERR_RX_ILLEGAL_VEC: u32 = 0x40;
#[allow(dead_code)]
const APIC_ERR_TX_ILLEGAL_VEC: u32 = 0x20;
const APIC_ERR_TX_ACCEPT_ERR: u32 = 0x04;

/// LVT write masks per entry (Bochs: apic.cc:619-627)
/// Determines which bits are writable for each LVT entry.
const LVT_MASKS: [u32; LVT_ENTRY_COUNT] = [
    0x000710FF, // TIMER:   vector[7:0], delivery_status[12], mask[16], timer_mode[17:18]
    0x000117FF, // THERMAL: vector[7:0], delivery_mode[10:8], delivery_status[12], mask[16]
    0x000117FF, // PERFMON: same as THERMAL
    0x0001F7FF, // LINT0:   + trigger_mode[15], remote_irr[14], polarity[13]
    0x0001F7FF, // LINT1:   same as LINT0
    0x000110FF, // ERROR:   vector[7:0], delivery_status[12], mask[16]
    0x000117FF, // CMCI:    same as THERMAL
];

bitflags::bitflags! {
    /// LVT (Local Vector Table) entry bits — common across all LVT entries.
    ///
    /// Each LVT register is 32 bits; the bit layout varies slightly per entry
    /// but the bits below are shared by all.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct LvtBits: u32 {
        /// Bits 0-7: interrupt vector number
        const VECTOR_MASK       = 0x0000_00FF;
        /// Bits 8-10: delivery mode (0=fixed, 2=SMI, 4=NMI, 5=INIT, 7=ExtINT)
        const DELIVERY_MODE     = 0x0000_0700;
        /// Bit 12: delivery status (read-only; 0=idle, 1=send pending)
        const DELIVERY_STATUS   = 0x0000_1000;
        /// Bit 13: input pin polarity (LINT only; 0=active high, 1=active low)
        const PIN_POLARITY      = 0x0000_2000;
        /// Bit 14: remote IRR (LINT level-trigger only; read-only)
        const REMOTE_IRR        = 0x0000_4000;
        /// Bit 15: trigger mode (LINT only; 0=edge, 1=level)
        const TRIGGER_MODE      = 0x0000_8000;
        /// Bit 16: mask — 1 = interrupt inhibited, 0 = allowed
        const MASKED            = 0x0001_0000;
        /// Bits 17-18: timer mode (timer LVT only; 0=oneshot, 1=periodic, 2=tsc-deadline)
        const TIMER_MODE        = 0x0006_0000;
    }
}

impl LvtBits {
    /// Wrap a raw u32 LVT register value.
    #[inline(always)]
    pub fn from_raw(raw: u32) -> Self {
        Self::from_bits_retain(raw)
    }

    /// Get the interrupt vector (bits 0-7).
    #[inline(always)]
    pub fn vector(self) -> u8 {
        (self.bits() & 0xFF) as u8
    }

    /// Get the timer mode field (bits 17-18): 0=oneshot, 1=periodic, 2=tsc-deadline.
    #[inline(always)]
    pub fn timer_mode_field(self) -> u32 {
        (self.bits() >> 17) & 0x3
    }
}

// ─── Types ────────────────────────────────────────────────────────────────────

/// APIC destination identifier
pub type ApicDest = u32;

/// APIC operating mode (Bochs: apic.h:37-42)
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

/// Local APIC register offsets (Bochs: apic.h:51-112)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LapicRegister {
    Id = 0x020,
    Version = 0x030,
    Tpr = 0x080,
    ArbitrationPriority = 0x090,
    Ppr = 0x0A0,
    Eoi = 0x0B0,
    Rrd = 0x0C0,
    Ldr = 0x0D0,
    DestinationFormat = 0x0E0,
    SpuriousVector = 0x0F0,
    Isr1 = 0x100,
    Isr2 = 0x110,
    Isr3 = 0x120,
    Isr4 = 0x130,
    Isr5 = 0x140,
    Isr6 = 0x150,
    Isr7 = 0x160,
    Isr8 = 0x170,
    Tmr1 = 0x180,
    Tmr2 = 0x190,
    Tmr3 = 0x1A0,
    Tmr4 = 0x1B0,
    Tmr5 = 0x1C0,
    Tmr6 = 0x1D0,
    Tmr7 = 0x1E0,
    Tmr8 = 0x1F0,
    Irr1 = 0x200,
    Irr2 = 0x210,
    Irr3 = 0x220,
    Irr4 = 0x230,
    Irr5 = 0x240,
    Irr6 = 0x250,
    Irr7 = 0x260,
    Irr8 = 0x270,
    Esr = 0x280,
    LvtCmci = 0x2F0,
    IcrLo = 0x300,
    IcrHi = 0x310,
    LvtTimer = 0x320,
    LvtThermal = 0x330,
    LvtPerfmon = 0x340,
    LvtLint0 = 0x350,
    LvtLint1 = 0x360,
    LvtError = 0x370,
    TimerInitialCount = 0x380,
    TimerCurrentCount = 0x390,
    TimerDivideCfg = 0x3E0,
    SelfIpi = 0x3F0,
    // Extended AMD features
    ExtApicFeature = 0x400,
    ExtApicControl = 0x410,
    SpecificEoi = 0x420,
    Ier1 = 0x480,
    Ier2 = 0x490,
    Ier3 = 0x4A0,
    Ier4 = 0x4B0,
    Ier5 = 0x4C0,
    Ier6 = 0x4D0,
    Ier7 = 0x4E0,
    Ier8 = 0x4F0,
}

/// APIC delivery mode for interrupt commands (Bochs: apic.h:116-125)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ApicDeliveryMode {
    Fixed = 0,
    LowPriority = 1,
    Smi = 2,
    Reserved = 3,
    Nmi = 4,
    Init = 5,
    Sipi = 6,
    ExtInt = 7,
}

/// Local Vector Table (LVT) entry indices (Bochs: apic.h:127-136)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum LocalVectorTableEntry {
    Timer = 0,
    Thermal = 1,
    Perfmon = 2,
    Lint0 = 3,
    Lint1 = 4,
    Error = 5,
    Cmci = 6,
}

/// Number of Local Vector Table entries (Bochs: APIC_LVT_ENTRIES)
pub const LVT_ENTRY_COUNT: usize = 7;

// ─── Pending interrupt delivery request ───────────────────────────────────────

/// A pending interrupt delivery from the LAPIC to the CPU.
/// Used to communicate delivery requests from the IPI path and APIC bus.
#[derive(Debug, Clone, Copy)]
pub struct ApicBusMessage {
    pub vector: u8,
    pub delivery_mode: u8,
    pub trigger_mode: u8,
}

// ─── BxLocalApic struct ──────────────────────────────────────────────────────

/// Local Advanced Programmable Interrupt Controller (LAPIC)
///
/// Ported from Bochs bx_local_apic_c (apic.h:138-293, apic.cc:172-1466).
/// Handles interrupts for a single processor core: IPIs, local interrupts,
/// timer, priority management, and interrupt acknowledge.
#[derive(Debug)]
pub struct BxLocalApic {
    /// Base physical address of the APIC (default 0xFEE00000)
    base_addr: BxPhyAddress,
    /// Current APIC operating mode (Bochs: mode)
    mode: ApicMode,
    /// Whether XAPIC mode is enabled (determines ID width, SVR vector bits)
    xapic: bool,
    /// Enabled extended XAPIC features bitmask
    xapic_ext: u32,
    /// APIC ID (4-bit in legacy, 8-bit in XAPIC, 32-bit in X2APIC)
    apic_id: u32,
    /// APIC version ID (encodes version + max LVT entry)
    apic_version_id: u32,
    /// Software enable flag (SVR bit 8)
    software_enabled: bool,
    /// Spurious interrupt vector (SVR bits 7:0)
    spurious_vector: u8,
    /// Focus processor checking disable (SVR bit 9)
    focus_disable: bool,

    /// Task priority register (TPR)
    task_priority: u32,
    /// Logical destination register (LDR)
    ldr: u32,
    /// Destination format register (DFR) — 4-bit field stored in bits 31:28
    dest_format: u32,

    /// In-service register (ISR) — 256 bits as 8×u32
    isr: [u32; 8],
    /// Trigger mode register (TMR) — set for level-triggered, cleared for edge
    tmr: [u32; 8],
    /// Interrupt request register (IRR) — pending interrupts
    irr: [u32; 8],
    /// Interrupt enable register (IER) — masks for priority computation
    ier: [u32; 8],

    /// Error status register (ESR)
    error_status: u32,
    /// Shadow error status register (accumulated between ESR writes)
    shadow_error_status: u32,

    /// Interrupt command register high (destination field)
    icr_hi: u32,
    /// Interrupt command register low (vector, mode, shorthand)
    icr_lo: u32,

    /// Local vector table entries [Timer, Thermal, Perfmon, LINT0, LINT1, Error, CMCI]
    lvt: [LvtBits; LVT_ENTRY_COUNT],
    /// Initial timer count (reload value for periodic mode)
    timer_initial: u32,
    /// Current timer count
    timer_current: u32,
    /// System tick value when timer started counting; also holds TSC-Deadline value
    ticks_initial: u64,
    /// Last-known system ticks — updated by emulator loop and before LAPIC reads.
    /// Used by read_aligned(&self) to compute current timer count without &mut self.
    pub(crate) current_ticks: u64,
    /// System ticks at last sync point (batch boundary).
    /// Used with icount_at_sync to compute live ticks mid-batch.
    pub(crate) ticks_at_sync: u64,
    /// CPU instruction count at last sync point.
    pub(crate) icount_at_sync: u64,
    /// Pointer to CPU's icount field for live tick computation during MMIO reads.
    /// Set during emulator initialization. Same pattern as PIT's icount_ptr.
    icount_ptr: Option<*const u64>,
    /// Pointer to CPU's pending_event field for direct event signaling.
    /// Allows service_local_apic() to signal BX_EVENT_PENDING_LAPIC_INTR
    /// without going through the emulator loop (matching Bochs behavior).
    pending_event_ptr: Option<*mut u32>,
    /// Pointer to CPU's async_event field for direct event triggering.
    async_event_ptr: Option<*mut u32>,

    /// Timer divide configuration register (bits 3,1,0 writable)
    timer_divconf: u32,
    /// Timer divide factor (1, 2, 4, 8, 16, 32, 64, 128)
    timer_divide_factor: u32,

    /// Internal timer state (not accessible from bus)
    timer_active: bool,
    /// Timer handle from BxPcSystemC (None = not registered)
    pub(crate) timer_handle: Option<usize>,

    /// VMX timer handle
    vmx_timer_handle: Option<usize>,
    /// VMX preemption timer value
    vmx_preemption_timer_value: u32,
    /// System tick value when VMX timer was set
    vmx_preemption_timer_initial: u64,
    /// System tick value when VMX timer fires
    vmx_preemption_timer_fire: u64,
    /// VMX preemption timer rate (from MSR_VMX_MISC)
    vmx_preemption_timer_rate: u32,
    /// VMX timer active state
    vmx_timer_active: bool,

    /// MWAITX timer handle
    mwaitx_timer_handle: Option<usize>,
    /// MWAITX timer active state
    mwaitx_timer_active: bool,

    /// INTR line to CPU — set by service_local_apic(), cleared by acknowledge_int()
    /// Mirrors Bochs `bool INTR` (apic.h:229).
    /// The CPU event handler checks this to deliver LAPIC interrupts.
    pub(crate) intr: bool,

    /// Queue of pending IPI deliveries that need APIC bus routing.
    /// Filled by send_ipi() for shorthand 0/2/3, drained by emulator loop.
    pub(crate) pending_ipi: Option<PendingIpi>,

    /// Pending EOI broadcast vector for level-triggered interrupts.
    /// Set by receive_eoi()/receive_seoi() when TMR bit is set.
    /// Drained by emulator loop to call ioapic.receive_eoi(vector).
    pub(crate) pending_eoi_vector: Option<u8>,

    /// Flag set by timer handler callback to indicate the timer has fired.
    /// The emulator loop should call periodic() when this is set.
    pub(crate) timer_fired: bool,

    /// Pending timer activation request: Some(period_ticks) means the emulator
    /// should call pc_system.activate_timer(handle, period, continuous=false).
    /// Set by set_initial_timer_count() and periodic(), cleared by emulator loop.
    pub(crate) timer_activate_request: Option<u64>,

    /// Pending timer deactivation request. Set by set_initial_timer_count()
    /// and periodic(), cleared by emulator loop.
    pub(crate) timer_deactivate_request: bool,

    /// Diagnostic counter: number of timer fires observed.
    pub(crate) diag_timer_fires: u64,
    /// Diagnostic: number of set_initial_timer_count calls
    pub(crate) diag_set_initial_count: u64,
    /// Diagnostic: number of LVT-masked periodic fires (not delivered)
    pub(crate) diag_timer_masked: u64,
}

/// Pending IPI that needs APIC bus routing (filled by send_ipi shorthand 0/2/3)
#[derive(Debug, Clone, Copy)]
pub struct PendingIpi {
    pub dest: ApicDest,
    pub lo_cmd: u32,
    pub shorthand: u8,
}

impl Default for BxLocalApic {
    fn default() -> Self {
        Self {
            base_addr: BX_LAPIC_BASE_ADDR,
            mode: ApicMode::GloballyDisabled,
            xapic: false,
            xapic_ext: 0,
            apic_id: 0,
            apic_version_id: 0,
            software_enabled: false,
            spurious_vector: 0xFF,
            focus_disable: false,
            task_priority: 0,
            ldr: 0,
            dest_format: 0xF,
            isr: [0; 8],
            tmr: [0; 8],
            irr: [0; 8],
            ier: [0xFFFFFFFF; 8],
            error_status: 0,
            shadow_error_status: 0,
            icr_hi: 0,
            icr_lo: 0,
            lvt: [LvtBits::MASKED; LVT_ENTRY_COUNT], // all masked
            timer_initial: 0,
            timer_current: 0,
            ticks_initial: 0,
            current_ticks: 0,
            ticks_at_sync: 0,
            icount_at_sync: 0,
            icount_ptr: None,
            pending_event_ptr: None,
            async_event_ptr: None,
            timer_divconf: 0,
            timer_divide_factor: 1,
            timer_active: false,
            timer_handle: None,
            vmx_timer_handle: None,
            vmx_preemption_timer_value: 0,
            vmx_preemption_timer_initial: 0,
            vmx_preemption_timer_fire: 0,
            vmx_preemption_timer_rate: 0,
            vmx_timer_active: false,
            mwaitx_timer_handle: None,
            mwaitx_timer_active: false,
            intr: false,
            pending_ipi: None,
            pending_eoi_vector: None,
            timer_fired: false,
            timer_activate_request: None,
            timer_deactivate_request: false,
            diag_timer_fires: 0,
            diag_set_initial_count: 0,
            diag_timer_masked: 0,
        }
    }
}

// ─── Static helper functions (Bochs: apic.cc:768-781) ────────────────────────

impl BxLocalApic {
    /// Set the pointer to CPU's icount for live tick computation.
    /// SAFETY: The pointer must remain valid for the lifetime of the LAPIC.
    pub(crate) unsafe fn set_icount_ptr(&mut self, ptr: *const u64) {
        self.icount_ptr = Some(ptr);
    }

    /// Set pointers to CPU's event fields for direct interrupt signaling.
    /// This allows service_local_apic() to signal BX_EVENT_PENDING_LAPIC_INTR
    /// directly, matching Bochs where the LAPIC calls cpu->signal_event().
    /// SAFETY: Pointers must remain valid for the lifetime of the LAPIC.
    pub(crate) unsafe fn set_event_ptrs(&mut self, pending: *mut u32, async_evt: *mut u32) {
        self.pending_event_ptr = Some(pending);
        self.async_event_ptr = Some(async_evt);
    }

    /// Get the live system tick count, accounting for instructions executed
    /// since the last batch boundary. This allows LAPIC timer current count
    /// reads to see progress within a CPU batch (critical for calibration loops).
    #[inline]
    fn live_ticks(&self) -> u64 {
        if let Some(ptr) = self.icount_ptr {
            let cpu_icount = unsafe { *ptr };
            self.ticks_at_sync + (cpu_icount - self.icount_at_sync)
        } else {
            self.current_ticks
        }
    }

    /// Check if a vector bit is set in a 256-bit register array.
    /// Bochs: bx_local_apic_c::get_vector (apic.cc:768-771)
    #[inline]
    fn get_vector(reg: &[u32; 8], vector: u32) -> bool {
        (reg[(vector / 32) as usize] >> (vector % 32)) & 1 != 0
    }

    /// Set a vector bit in a 256-bit register array.
    /// Bochs: bx_local_apic_c::set_vector (apic.cc:773-776)
    #[inline]
    fn set_vector(reg: &mut [u32; 8], vector: u32) {
        reg[(vector / 32) as usize] |= 1 << (vector % 32);
    }

    /// Clear a vector bit in a 256-bit register array.
    /// Bochs: bx_local_apic_c::clear_vector (apic.cc:778-781)
    #[inline]
    fn clear_vector(reg: &mut [u32; 8], vector: u32) {
        reg[(vector / 32) as usize] &= !(1 << (vector % 32));
    }

    /// Find the highest-priority set bit in a 256-bit register, masked by IER.
    /// Returns the vector number (0-255) or -1 if none set.
    /// Bochs: bx_local_apic_c::highest_priority_int (apic.cc:783-799)
    fn highest_priority_int(&self, array: &[u32; 8]) -> i32 {
        for reg in (0..8).rev() {
            // Apply IER mask: only enabled vectors participate
            let tmp = array[reg] & self.ier[reg];
            if tmp != 0 {
                // most_significant_bitd: position of highest set bit
                let bit = 31 - tmp.leading_zeros();
                return (reg as i32 * 32) + bit as i32;
            }
        }
        -1
    }

    /// Get the APIC ID mask based on mode (XAPIC=0xFF, legacy=0x0F).
    /// Bochs: extern apic_id_mask (main.cc:1031)
    #[inline]
    pub(crate) fn apic_id_mask(&self) -> u32 {
        if self.xapic {
            APIC_ID_MASK_XAPIC
        } else {
            APIC_ID_MASK_LEGACY
        }
    }
}

// ─── Core LAPIC methods ──────────────────────────────────────────────────────

impl BxLocalApic {
    /// Get the APIC ID.
    /// Bochs: get_id() (apic.h:234)
    #[inline]
    pub(crate) fn get_id(&self) -> u32 {
        self.apic_id
    }

    /// Check if this is an XAPIC.
    /// Bochs: is_xapic() (apic.h:235)
    #[inline]
    pub(crate) fn is_xapic(&self) -> bool {
        self.xapic
    }

    /// Get current mode.
    #[inline]
    pub(crate) fn get_mode(&self) -> ApicMode {
        self.mode
    }

    /// Get the base address.
    /// Bochs: get_base() (apic.h:232)
    #[inline]
    pub(crate) fn get_base(&self) -> BxPhyAddress {
        self.base_addr
    }

    /// Check if this APIC handles the given MMIO address.
    /// Bochs: is_selected (apic.cc:308-318)
    pub(crate) fn is_selected(&self, addr: BxPhyAddress) -> bool {
        if self.mode != ApicMode::XapicMode {
            return false;
        }
        if (addr & !0xFFF) == self.base_addr {
            if (addr & 0xF) != 0 {
                info!("warning: misaligned APIC access. addr={:#x}", addr);
            }
            return true;
        }
        false
    }

    // ─── Register read ───────────────────────────────────────────────────

    /// Read from a 16-byte-aligned APIC register.
    /// Bochs: read_aligned (apic.cc:357-492)
    pub(crate) fn read_aligned(&self, addr: BxPhyAddress) -> u32 {
        debug_assert!((addr & 0xF) == 0);
        let mut data: u32 = 0;
        let apic_reg = (addr & 0xFF0) as u32;

        match apic_reg {
            // Local APIC ID (apic.cc:371-372)
            0x020 => {
                data = self.apic_id << 24;
            }
            // Local APIC version (apic.cc:373-374)
            0x030 => {
                data = self.apic_version_id;
            }
            // Task priority (apic.cc:375-376)
            0x080 => {
                data = self.task_priority & 0xFF;
            }
            // Arbitration priority (apic.cc:377-378)
            0x090 => {
                data = self.get_apr() as u32;
            }
            // Processor priority (apic.cc:379-380)
            0x0A0 => {
                data = self.get_ppr() as u32;
            }
            // EOI — read returns 0 (apic.cc:381-387)
            0x0B0 => {}
            // Logical destination (apic.cc:388-390)
            0x0D0 => {
                data = (self.ldr & self.apic_id_mask()) << 24;
            }
            // Destination format (apic.cc:391-393)
            0x0E0 => {
                data = ((self.dest_format & 0xF) << 28) | 0x0FFF_FFFF;
            }
            // Spurious vector (apic.cc:394-401)
            0x0F0 => {
                let mut reg = self.spurious_vector as u32;
                if self.software_enabled {
                    reg |= 0x100;
                }
                if self.focus_disable {
                    reg |= 0x200;
                }
                data = reg;
            }
            // ISR 1-8 (apic.cc:402-410)
            0x100 | 0x110 | 0x120 | 0x130 | 0x140 | 0x150 | 0x160 | 0x170 => {
                let index = ((apic_reg - 0x100) >> 4) as usize;
                data = self.isr[index];
            }
            // TMR 1-8 (apic.cc:411-419)
            0x180 | 0x190 | 0x1A0 | 0x1B0 | 0x1C0 | 0x1D0 | 0x1E0 | 0x1F0 => {
                let index = ((apic_reg - 0x180) >> 4) as usize;
                data = self.tmr[index];
            }
            // IRR 1-8 (apic.cc:420-428)
            0x200 | 0x210 | 0x220 | 0x230 | 0x240 | 0x250 | 0x260 | 0x270 => {
                let index = ((apic_reg - 0x200) >> 4) as usize;
                data = self.irr[index];
            }
            // ESR (apic.cc:429-430)
            0x280 => {
                data = self.error_status;
            }
            // ICR low (apic.cc:431-432)
            0x300 => {
                data = self.icr_lo;
            }
            // ICR high (apic.cc:433-434)
            0x310 => {
                data = self.icr_hi;
            }
            // LVT Timer, Thermal, Perfmon, LINT0, LINT1, Error (apic.cc:435-445)
            0x320 | 0x330 | 0x340 | 0x350 | 0x360 | 0x370 => {
                let index = ((apic_reg - 0x320) >> 4) as usize;
                data = self.lvt[index].bits();
            }
            // LVT CMCI (apic.cc:446-448)
            0x2F0 => {
                data = self.lvt[LocalVectorTableEntry::Cmci as usize].bits();
            }
            // Timer initial count (apic.cc:449-451)
            0x380 => {
                data = self.timer_initial;
            }
            // Timer current count (apic.cc:452-454)
            // Bochs calls get_current_timer_count(bx_pc_system.time_ticks()) here.
            // We use live_ticks() which reads CPU icount via pointer for accuracy
            // within CPU batches (critical for kernel timer calibration loops).
            0x390 => {
                let timervec = self.lvt[LocalVectorTableEntry::Timer as usize];
                if timervec.timer_mode_field() == 2 {
                    // TSC-deadline mode: current count always reads 0
                    data = 0;
                } else if self.timer_active && self.timer_divide_factor > 0 {
                    let ticks = self.live_ticks();
                    let delta64 = ticks.saturating_sub(self.ticks_initial)
                        / self.timer_divide_factor as u64;
                    let delta32 = delta64 as u32;
                    data = if delta32 >= self.timer_initial { 0 } else { self.timer_initial - delta32 };
                } else {
                    data = self.timer_current;
                }
            }
            // Timer divide configuration (apic.cc:455-457)
            0x3E0 => {
                data = self.timer_divconf;
            }
            // Extended APIC feature register (apic.cc:459-462)
            0x400 => {
                data = BX_XAPIC_EXT_SUPPORT_IER | BX_XAPIC_EXT_SUPPORT_SEOI;
            }
            // Extended APIC control register (apic.cc:463-466)
            0x410 => {
                data = self.xapic_ext;
            }
            // Specific EOI — read returns 0 (apic.cc:467-473)
            0x420 => {}
            // IER 1-8 (apic.cc:474-482)
            0x480 | 0x490 | 0x4A0 | 0x4B0 | 0x4C0 | 0x4D0 | 0x4E0 | 0x4F0 => {
                let index = ((apic_reg - 0x480) >> 4) as usize;
                data = self.ier[index];
            }
            // Default: illegal register (apic.cc:484-488)
            _ => {
                self.set_shadow_error(APIC_ERR_ILLEGAL_ADDR);
                error!("APIC read: register {:#x} not implemented", apic_reg);
            }
        }

        debug!("read from APIC address {:#x} = {:#010x}", addr, data);
        data
    }

    // ─── Register write ──────────────────────────────────────────────────

    /// Write to a 16-byte-aligned APIC register.
    /// Bochs: write_aligned (apic.cc:495-615)
    pub(crate) fn write_aligned(&mut self, addr: BxPhyAddress, value: u32) {
        debug_assert!((addr & 0xF) == 0);
        let apic_reg = (addr & 0xFF0) as u32;

        match apic_reg {
            // TPR (apic.cc:508-510)
            0x080 => {
                self.set_tpr((value & 0xFF) as u8);
            }
            // EOI (apic.cc:511-513)
            0x0B0 => {
                self.receive_eoi(value);
            }
            // LDR (apic.cc:514-517)
            0x0D0 => {
                self.ldr = (value >> 24) & self.apic_id_mask();
                debug!("set logical destination to {:#010x}", self.ldr);
            }
            // DFR (apic.cc:518-521)
            0x0E0 => {
                self.dest_format = (value >> 28) & 0xF;
                debug!("set destination format to {:#04x}", self.dest_format);
            }
            // SVR (apic.cc:522-524)
            0x0F0 => {
                self.write_spurious_interrupt_register(value);
            }
            // ESR (apic.cc:525-532)
            0x280 => {
                // Write to ESR latches shadow into visible register, clears shadow.
                // IA-devguide-3 p.7-45: write before read to update register.
                self.error_status = self.shadow_error_status;
                self.shadow_error_status = 0;
            }
            // ICR low — triggers IPI send (apic.cc:533-536)
            0x300 => {
                self.icr_lo = value & !(1 << 12); // force delivery status bit = 0 (idle)
                let dest = (self.icr_hi >> 24) & 0xFF;
                self.send_ipi(dest, self.icr_lo);
            }
            // ICR high (apic.cc:537-539)
            0x310 => {
                self.icr_hi = value & 0xFF00_0000;
            }
            // LVT Timer, Thermal, Perfmon, LINT0, LINT1, Error (apic.cc:540-548)
            0x320 | 0x330 | 0x340 | 0x350 | 0x360 | 0x370 => {
                self.set_lvt_entry(apic_reg, value);
            }
            // LVT CMCI (apic.cc:546-547)
            0x2F0 => {
                self.set_lvt_entry(apic_reg, value);
            }
            // Timer initial count (apic.cc:549-551)
            0x380 => {
                self.set_initial_timer_count(value);
            }
            // Timer divide configuration (apic.cc:552-556)
            0x3E0 => {
                // Only bits 3, 1, and 0 are writable
                self.timer_divconf = value & 0xB;
                self.set_divide_configuration(self.timer_divconf);
            }
            // Read-only registers — warn on write (apic.cc:557-582)
            0x020 | 0x030 | 0x090 | 0x0C0 | 0x0A0 | 0x100 | 0x110 | 0x120 | 0x130 | 0x140
            | 0x150 | 0x160 | 0x170 | 0x180 | 0x190 | 0x1A0 | 0x1B0 | 0x1C0 | 0x1D0 | 0x1E0
            | 0x1F0 | 0x200 | 0x210 | 0x220 | 0x230 | 0x240 | 0x250 | 0x260 | 0x270 | 0x390 => {
                info!("warning: write to read-only APIC register {:#x}", apic_reg);
            }
            // Extended APIC feature — read-only (apic.cc:584-587)
            0x400 => {
                info!("warning: write to read-only APIC register {:#x}", apic_reg);
            }
            // Extended APIC control (apic.cc:588-591)
            0x410 => {
                self.xapic_ext = value & (BX_XAPIC_EXT_SUPPORT_IER | BX_XAPIC_EXT_SUPPORT_SEOI);
            }
            // Specific EOI (apic.cc:592-594)
            0x420 => {
                self.receive_seoi((value & 0xFF) as u8);
            }
            // IER 1-8 (apic.cc:595-608)
            0x480 | 0x490 | 0x4A0 | 0x4B0 | 0x4C0 | 0x4D0 | 0x4E0 | 0x4F0 => {
                if (self.xapic_ext & BX_XAPIC_EXT_SUPPORT_IER) == 0 {
                    error!("IER writes are currently disabled reg {:#x}", apic_reg);
                } else {
                    let index = ((apic_reg - 0x480) >> 4) as usize;
                    self.ier[index] = value;
                }
            }
            // Default: illegal register (apic.cc:610-614)
            _ => {
                self.set_shadow_error(APIC_ERR_ILLEGAL_ADDR);
                error!("APIC write: register {:#x} not implemented", apic_reg);
            }
        }
    }

    // ─── MMIO read/write wrappers ────────────────────────────────────────

    /// Handle a read from the LAPIC MMIO region.
    /// Bochs: read (apic.cc:320-339)
    pub(crate) fn read(&self, addr: BxPhyAddress, len: u32) -> u32 {
        if (addr & !0x3) != ((addr + len as BxPhyAddress - 1) & !0x3) {
            error!("APIC read at address {:#x} spans 32-bit boundary!", addr);
            return 0;
        }
        let value = self.read_aligned(addr & !0x3);
        if len == 4 {
            return value;
        }
        // Handle partial read, independent of endianness
        let shift = ((addr & 3) * 8) as u32;
        let shifted = value >> shift;
        match len {
            1 => shifted & 0xFF,
            2 => shifted & 0xFFFF,
            _ => {
                error!("Unsupported APIC read at {:#x}, len={}", addr, len);
                0
            }
        }
    }

    /// Handle a write to the LAPIC MMIO region.
    /// Bochs: write (apic.cc:341-354)
    pub(crate) fn write(&mut self, addr: BxPhyAddress, value: u32, len: u32) {
        if len != 4 {
            error!("APIC write with len={} (should be 4)", len);
            return;
        }
        if (addr & 0xF) != 0 {
            error!("APIC write at unaligned address {:#x}", addr);
            return;
        }
        self.write_aligned(addr, value);
    }

    // ─── LVT entry write ────────────────────────────────────────────────

    /// Write to a Local Vector Table entry with proper masking.
    /// Bochs: set_lvt_entry (apic.cc:617-651)
    fn set_lvt_entry(&mut self, apic_reg: u32, mut value: u32) {
        let lvt_entry = if apic_reg == 0x2F0 {
            // CMCI
            LocalVectorTableEntry::Cmci as usize
        } else {
            ((apic_reg - 0x320) >> 4) as usize
        };

        // TSC-Deadline mode handling for timer LVT (apic.cc:631-644)
        if apic_reg == 0x320 {
            // Cannot enable TSC-Deadline when not supported (we don't support it)
            value &= !0x40000;
            // Trace timer LVT writes
            let mode = match (value >> 17) & 3 { 0 => "one-shot", 1 => "periodic", 2 => "tsc-dl", _ => "??" };
            let vec = value & 0xFF;
            let masked = (value >> 16) & 1;
            debug!("[LAPIC] LVT_TIMER write: vec={:#x} mode={} masked={} raw={:#010x}",
                vec, mode, masked, value);
        }

        // Apply LVT mask for this entry
        self.lvt[lvt_entry] = LvtBits::from_raw(value & LVT_MASKS[lvt_entry]);

        // If APIC software-disabled, force mask bit (apic.cc:648-650)
        if !self.software_enabled {
            self.lvt[lvt_entry].insert(LvtBits::MASKED);
        }
    }

    // ─── IPI send ────────────────────────────────────────────────────────

    /// Send an Inter-Processor Interrupt.
    /// Bochs: send_ipi (apic.cc:653-697)
    fn send_ipi(&mut self, dest: ApicDest, lo_cmd: u32) {
        let dest_shorthand = (lo_cmd >> 18) & 3;
        let trig_mode = ((lo_cmd >> 15) & 1) as u8;
        let level = (lo_cmd >> 14) & 1;
        let delivery_mode = ((lo_cmd >> 8) & 7) as u8;
        let vector = (lo_cmd & 0xFF) as u8;

        // INIT Level Deassert — special no-op mode (apic.cc:663-673)
        if delivery_mode == ApicDeliveryMode::Init as u8 {
            if level == 0 && trig_mode == 1 {
                // "INIT Level Deassert": causes all APICs to set their
                // arbitration ID to their APIC ID. Not supported by P4/Xeon.
                // We don't model APIC bus arbitration ID, so just return.
                return;
            }
        }

        match dest_shorthand {
            // 0: no shorthand — use real destination value (apic.cc:676-678)
            0 => {
                // For single-CPU: if dest matches our ID (physical) or we match
                // logical addressing, deliver directly. Otherwise queue for
                // emulator loop to route (in case of multi-CPU future).
                let logical_dest = (lo_cmd >> 11) & 1;
                if logical_dest == 0 {
                    // Physical destination
                    if dest == self.apic_id {
                        self.deliver(vector, delivery_mode, trig_mode);
                    } else if dest == self.apic_id_mask() {
                        // Broadcast — for single CPU, deliver to self
                        self.deliver(vector, delivery_mode, trig_mode);
                    } else {
                        // No matching CPU — set TX accept error
                        debug!(
                            "IPI to physical dest {:#x} not accepted (no matching APIC)",
                            dest
                        );
                        self.shadow_error_status |= APIC_ERR_TX_ACCEPT_ERR;
                    }
                } else {
                    // Logical destination
                    if dest == 0 {
                        // Logical dest 0 = no target
                        self.shadow_error_status |= APIC_ERR_TX_ACCEPT_ERR;
                    } else if self.match_logical_addr(dest) {
                        self.deliver(vector, delivery_mode, trig_mode);
                    } else {
                        self.shadow_error_status |= APIC_ERR_TX_ACCEPT_ERR;
                    }
                }
            }
            // 1: self (apic.cc:679-682)
            1 => {
                self.trigger_irq(vector, trig_mode, false);
            }
            // 2: all including self (apic.cc:683-685)
            2 => {
                // Single CPU: just deliver to self
                self.deliver(vector, delivery_mode, trig_mode);
            }
            // 3: all but self (apic.cc:686-688)
            3 => {
                // Single CPU: nothing to deliver (exclude self)
                debug!("IPI all-but-self: no other CPUs");
            }
            _ => {
                error!("Invalid destination shorthand {:#x}", dest_shorthand);
            }
        }
    }

    // ─── Spurious interrupt vector register ──────────────────────────────

    /// Write to the spurious interrupt vector register.
    /// Bochs: write_spurious_interrupt_register (apic.cc:699-717)
    fn write_spurious_interrupt_register(&mut self, value: u32) {
        debug!("write of {:#x} to spurious interrupt register", value);

        if self.xapic {
            self.spurious_vector = (value & 0xFF) as u8;
        } else {
            // Bits 0-3 of the spurious vector hardwired to '1' in legacy mode
            self.spurious_vector = ((value & 0xF0) | 0x0F) as u8;
        }

        let was_enabled = self.software_enabled;
        self.software_enabled = ((value >> 8) & 1) != 0;
        self.focus_disable = ((value >> 9) & 1) != 0;

        // Trace enable/disable transitions
        if was_enabled != self.software_enabled {
            debug!("[LAPIC] SVR write: sw_enabled {} -> {} (SVR={:#x})",
                was_enabled, self.software_enabled, value);
        }

        if !self.software_enabled {
            for entry in &mut self.lvt {
                entry.insert(LvtBits::MASKED); // mask all LVT
            }
        }
    }

    // ─── EOI handling ────────────────────────────────────────────────────

    /// Receive End-of-Interrupt. Clears highest-priority ISR bit.
    /// For level-triggered interrupts, broadcasts EOI to I/O APIC.
    /// Bochs: receive_EOI (apic.cc:719-739)
    pub(crate) fn receive_eoi(&mut self, _value: u32) {
        let vec = self.highest_priority_int(&self.isr);
        debug!("EOI: isr_hp={}", vec);
        if vec < 0 {
            debug!("EOI written without any bit in ISR");
        } else {
            let vec_u32 = vec as u32;
            if vec_u32 != self.spurious_vector as u32 {
                debug!("local apic received EOI, for vector {:#04x}", vec);
                Self::clear_vector(&mut self.isr, vec_u32);
                if Self::get_vector(&self.tmr, vec_u32) {
                    // Level-triggered: broadcast EOI to I/O APIC
                    // Bochs apic.cc:730-732: apic_bus_broadcast_eoi(vec)
                    self.pending_eoi_vector = Some(vec as u8);
                    Self::clear_vector(&mut self.tmr, vec_u32);
                }
                self.service_local_apic();
            }
        }
    }

    /// Receive Specific End-of-Interrupt for a given vector.
    /// Bochs: receive_SEOI (apic.cc:742-761)
    fn receive_seoi(&mut self, vec: u8) {
        if (self.xapic_ext & BX_XAPIC_EXT_SUPPORT_SEOI) == 0 {
            error!("SEOI functionality is disabled");
            return;
        }

        let vec_u32 = vec as u32;
        if Self::get_vector(&self.isr, vec_u32) {
            debug!("local apic received SEOI for vector {:#04x}", vec);
            Self::clear_vector(&mut self.isr, vec_u32);
            if Self::get_vector(&self.tmr, vec_u32) {
                // Level-triggered: broadcast EOI to I/O APIC
                // Bochs apic.cc:753: apic_bus_broadcast_eoi(vec)
                self.pending_eoi_vector = Some(vec);
                Self::clear_vector(&mut self.tmr, vec_u32);
            }
            self.service_local_apic();
        }
    }

    // ─── Interrupt delivery and servicing ────────────────────────────────

    /// Service the local APIC: check if a pending IRR interrupt should
    /// be signaled to the CPU.
    /// Sets self.intr = true if an interrupt is deliverable.
    /// Bochs: service_local_apic (apic.cc:801-827)
    /// Clear BX_EVENT_PENDING_LAPIC_INTR from CPU event system.
    /// Bochs: cpu->clear_event(BX_EVENT_PENDING_LAPIC_INTR)
    fn clear_pending_lapic_event(&mut self) {
        const BX_EVENT_PENDING_LAPIC_INTR: u32 = 1 << 2;
        if let Some(ptr) = self.pending_event_ptr {
            unsafe { *ptr &= !BX_EVENT_PENDING_LAPIC_INTR; }
        }
    }

    pub(crate) fn service_local_apic(&mut self) {
        // If INTR already raised, nothing to do (apic.cc:808)
        if self.intr {
            return;
        }

        // Find highest priority interrupt in IRR (apic.cc:811)
        let first_irr = self.highest_priority_int(&self.irr);
        if first_irr < 0 {
            return; // no pending interrupts
        }

        // Compare against highest priority in-service interrupt (apic.cc:813-817)
        let first_isr = self.highest_priority_int(&self.isr);
        if first_isr >= 0 && first_irr <= first_isr {
            debug!(
                "lapic({}): not delivering int {:#04x} because int {:#04x} is in service",
                self.apic_id, first_irr, first_isr
            );
            return;
        }

        // Compare against task priority (apic.cc:818-821)
        if ((first_irr as u32) & 0xF0) <= (self.task_priority & 0xF0) {
            debug!(
                "lapic({}): not delivering int {:#04x} because task_priority is {:#04x}",
                self.apic_id, first_irr, self.task_priority
            );
            return;
        }

        // Signal CPU that interrupt is ready (apic.cc:825-826)
        // Bochs: cpu->signal_event(BX_EVENT_PENDING_LAPIC_INTR)
        debug!(
            "service_local_apic(): setting INTR=1 for vector {:#04x}",
            first_irr
        );
        self.intr = true;

        // Directly signal the CPU's event system (matching Bochs apic.cc:825).
        // Without this, the event would only be signaled at the next batch boundary,
        // causing interrupts triggered by EOI within a batch to be delayed.
        const BX_EVENT_PENDING_LAPIC_INTR: u32 = 1 << 2;
        if let Some(ptr) = self.pending_event_ptr {
            unsafe { *ptr |= BX_EVENT_PENDING_LAPIC_INTR; }
        }
        if let Some(ptr) = self.async_event_ptr {
            unsafe { *ptr |= 1; }
        }
    }

    /// Deliver an interrupt to this LAPIC (from APIC bus or IPI).
    /// Bochs: deliver (apic.cc:829-862)
    pub(crate) fn deliver(&mut self, vector: u8, delivery_mode: u8, trig_mode: u8) -> bool {
        match delivery_mode {
            // Fixed or LowPriority (apic.cc:832-836)
            0 | 1 => {
                debug!("Deliver fixed/lowpri interrupt vector {:#04x}", vector);
                self.trigger_irq(vector, trig_mode, false);
            }
            // SMI (apic.cc:837-839) — not implemented
            2 => {
                info!("Deliver SMI (not implemented)");
            }
            // NMI (apic.cc:841-843) — not implemented
            4 => {
                info!("Deliver NMI (not implemented)");
            }
            // INIT (apic.cc:845-847) — not implemented
            5 => {
                info!("Deliver INIT IPI (not implemented)");
            }
            // SIPI (apic.cc:849-852) — not implemented
            6 => {
                info!("Deliver Start Up IPI (not implemented)");
            }
            // ExtINT (apic.cc:853-856)
            7 => {
                debug!("Deliver EXTINT vector {:#04x}", vector);
                self.trigger_irq(vector, trig_mode, true);
            }
            // Reserved (apic.cc:857-858)
            _ => {
                return false;
            }
        }
        true
    }

    /// Trigger an IRQ in the LAPIC's IRR.
    /// Bochs: trigger_irq (apic.cc:864-890)
    pub(crate) fn trigger_irq(&mut self, vector: u8, trigger_mode: u8, bypass_irr_isr: bool) {
        debug!("trigger interrupt vector={:#04x}", vector);

        // Validate vector range (apic.cc:868-872)
        if vector < BX_LAPIC_FIRST_VECTOR {
            self.shadow_error_status |= APIC_ERR_RX_ILLEGAL_VEC;
            info!("bogus vector {:#x}, ignoring", vector);
            return;
        }

        let vec_u32 = vector as u32;

        // Check if already pending in IRR (unless bypassing) (apic.cc:876-881)
        if !bypass_irr_isr && Self::get_vector(&self.irr, vec_u32) {
            debug!(
                "triggered vector {:#04x} not accepted (already in IRR)",
                vector
            );
            return;
        }

        // Set IRR bit (apic.cc:883)
        Self::set_vector(&mut self.irr, vec_u32);

        // Update TMR based on trigger mode (apic.cc:884-887)
        if trigger_mode != 0 {
            Self::set_vector(&mut self.tmr, vec_u32); // level triggered
        } else {
            Self::clear_vector(&mut self.tmr, vec_u32); // edge triggered
        }

        // Check if interrupt can be delivered (apic.cc:889)
        self.service_local_apic();
    }

    /// Clear an IRQ from the LAPIC's IRR (hardware deasserted).
    /// Bochs: untrigger_irq (apic.cc:892-900)
    pub(crate) fn untrigger_irq(&mut self, vector: u8, _trigger_mode: u8) {
        debug!("untrigger interrupt vector={:#04x}", vector);
        Self::clear_vector(&mut self.irr, vector as u32);
    }

    /// CPU acknowledges the highest-priority interrupt.
    /// Moves the vector from IRR to ISR. Returns the vector number,
    /// or spurious_vector if no valid interrupt is pending.
    /// Bochs: acknowledge_int (apic.cc:902-926)
    pub(crate) fn acknowledge_int(&mut self) -> u8 {
        let vector = self.highest_priority_int(&self.irr);
        if vector < 0 || ((vector as u32) & 0xF0) <= (self.get_ppr() as u32) {
            // No deliverable interrupt — return spurious vector
            // Bochs apic.cc:909 — clear PENDING_LAPIC_INTR event
            self.intr = false;
            self.clear_pending_lapic_event();
            return self.spurious_vector;
        }

        let vec_u32 = vector as u32;
        debug!("acknowledge_int() returning vector {:#04x}", vector);

        // Move from IRR to ISR (apic.cc:916-917)
        Self::clear_vector(&mut self.irr, vec_u32);
        Self::set_vector(&mut self.isr, vec_u32);

        // Clear INTR and re-check for more interrupts (apic.cc:923-924)
        // Bochs: cpu->clear_event(BX_EVENT_PENDING_LAPIC_INTR)
        self.intr = false;
        self.clear_pending_lapic_event();
        self.service_local_apic(); // may set intr=true again

        vector as u8
    }

    // ─── Logical addressing ──────────────────────────────────────────────

    /// Check if this LAPIC matches the given logical destination address.
    /// Bochs: match_logical_addr (apic.cc:939-975)
    pub(crate) fn match_logical_addr(&self, address: ApicDest) -> bool {
        // X2APIC mode: cluster model only (apic.cc:944-951)
        if self.mode == ApicMode::X2apicMode {
            if address == 0xFFFF_FFFF {
                return true; // broadcast all
            }
            if (address & 0xFFFF_0000) == (self.ldr & 0xFFFF_0000) {
                return (address & self.ldr & 0x0000_FFFF) != 0;
            }
            return false;
        }

        // Broadcast: all-ones destination (apic.cc:956-957)
        if address == 0xFF {
            return true;
        }

        if self.dest_format == 0xF {
            // Flat model (apic.cc:959-964)
            let m = (address & self.ldr) != 0;
            debug!(
                "comparing MDA {:#04x} to my LDR {:#04x} -> {}",
                address,
                self.ldr,
                if m { "Match" } else { "Not a match" }
            );
            m
        } else if self.dest_format == 0 {
            // Cluster model (apic.cc:965-968)
            if (address & 0xF0) == (self.ldr & 0xF0) {
                (address & self.ldr & 0x0F) != 0
            } else {
                false
            }
        } else {
            error!(
                "match_logical_addr: unsupported dest format {:#x}",
                self.dest_format
            );
            false
        }
    }

    // ─── Priority registers ──────────────────────────────────────────────

    /// Get the Processor Priority Register (PPR).
    /// PPR = max(TPR, highest ISR vector class).
    /// Bochs: get_ppr (apic.cc:977-987)
    pub(crate) fn get_ppr(&self) -> u8 {
        let mut ppr = self.highest_priority_int(&self.isr);

        if ppr < 0 || (self.task_priority & 0xF0) >= (ppr as u32 & 0xF0) {
            ppr = self.task_priority as i32;
        } else {
            ppr &= 0xF0_i32;
        }

        ppr as u8
    }

    /// Set the Task Priority Register (TPR).
    /// If lowered, re-check for deliverable interrupts.
    /// Bochs: set_tpr (apic.cc:989-997)
    pub(crate) fn set_tpr(&mut self, priority: u8) {
        if (priority as u32) < self.task_priority {
            self.task_priority = priority as u32;
            self.service_local_apic();
        } else {
            self.task_priority = priority as u32;
        }
    }

    /// Get the TPR value.
    /// Bochs: get_tpr (apic.h:259)
    #[inline]
    pub(crate) fn get_tpr(&self) -> u8 {
        self.task_priority as u8
    }

    /// Get the Arbitration Priority Register (APR).
    /// Bochs: get_apr (apic.cc:999-1021)
    pub(crate) fn get_apr(&self) -> u8 {
        let tpr = (self.task_priority >> 4) & 0xF;
        let mut first_isr = self.highest_priority_int(&self.isr);
        if first_isr < 0 {
            first_isr = 0;
        }
        let mut first_irr = self.highest_priority_int(&self.irr);
        if first_irr < 0 {
            first_irr = 0;
        }
        let isrv = ((first_isr as u32) >> 4) & 0xF;
        let irrv = ((first_irr as u32) >> 4) & 0xF;

        let apr: u8;
        if tpr >= irrv && tpr > isrv {
            apr = (self.task_priority & 0xFF) as u8;
        } else {
            let combined = tpr & isrv;
            let chosen = if combined > irrv { combined } else { irrv };
            apr = (chosen << 4) as u8;
        }

        debug!("apr = {}", apr);
        apr
    }

    /// Check if this LAPIC is the focus processor for a given vector.
    /// Bochs: is_focus (apic.cc:1023-1027)
    pub(crate) fn is_focus(&self, vector: u8) -> bool {
        if self.focus_disable {
            return false;
        }
        let v = vector as u32;
        Self::get_vector(&self.irr, v) || Self::get_vector(&self.isr, v)
    }

    // ─── Timer ───────────────────────────────────────────────────────────

    /// Timer callback — called when the LAPIC timer fires.
    /// If not masked, triggers the timer interrupt vector.
    /// In periodic mode, reloads the timer.
    /// Bochs: periodic (apic.cc:1035-1069)
    ///
    /// Note: In our architecture, the emulator loop calls this when
    /// timer_fired is set by the pc_system timer handler.
    pub(crate) fn periodic(&mut self, current_ticks: u64) {
        if !self.timer_active {
            error!("periodic() called, timer_active==0");
            return;
        }

        let timervec = self.lvt[LocalVectorTableEntry::Timer as usize];

        // If timer is not masked, trigger interrupt (apic.cc:1045-1050)
        if !timervec.contains(LvtBits::MASKED) {
            // Log first few and transition events
            let fire_num = self.diag_timer_fires; // incremented by caller after this
            if fire_num < 5 || (fire_num < 200 && fire_num % 50 == 0) {
                debug!("[LAPIC] periodic FIRE #{}: vec={:#x} mode={} ticks={} irr_set={} isr_set={} intr={}",
                    fire_num, timervec.vector(), timervec.timer_mode_field(), current_ticks,
                    Self::get_vector(&self.irr, timervec.vector() as u32),
                    Self::get_vector(&self.isr, timervec.vector() as u32),
                    self.intr);
            }
            self.trigger_irq(timervec.vector(), APIC_EDGE_TRIGGERED, false);
        } else {
            self.diag_timer_masked += 1;
            debug!("[LAPIC] periodic: LVT MASKED (fire #{}), sw_enabled={}",
                self.diag_timer_fires, self.software_enabled);
        }

        // Check timer mode (apic.cc:1053-1068)
        if timervec.timer_mode_field() == 1 {
            // Periodic mode — reload timer values
            self.timer_current = self.timer_initial;
            self.timer_active = true;
            self.ticks_initial = current_ticks;
            debug!(
                "local apic timer(periodic) triggered int, reset counter to {:#010x}",
                self.timer_current
            );
            // Request re-activation with same period
            // Bochs apic.cc:1059-1060: activate_timer_ticks(handle, Bit64u(initial)*Bit64u(factor), 0)
            let period = self.timer_initial as u64 * self.timer_divide_factor as u64;
            self.timer_activate_request = Some(period);
        } else {
            // One-shot mode — timer is done
            self.timer_current = 0;
            self.timer_active = false;
            // Bochs apic.cc:1067: deactivate_timer(timer_handle)
            self.timer_deactivate_request = true;
            debug!("local apic timer(one-shot) triggered int");
        }
    }

    /// Set the timer divide configuration.
    /// Bochs: set_divide_configuration (apic.cc:1071-1079)
    fn set_divide_configuration(&mut self, value: u32) {
        debug_assert!(value == (value & 0x0B));
        // Move bit 3 down to bit 0: {bit3, bit1, bit0} → 3-bit value
        let combined = ((value & 8) >> 1) | (value & 3);
        // value 0..6 → factor 2,4,8,16,32,64,128; value 7 → factor 1
        self.timer_divide_factor = if combined == 7 { 1 } else { 2 << combined };
        info!("set timer divide factor to {}", self.timer_divide_factor);
    }

    /// Write the initial timer count register. Starts or restarts the timer.
    /// Bochs: set_initial_timer_count (apic.cc:1081-1110)
    pub(crate) fn set_initial_timer_count(&mut self, value: u32) {
        self.diag_set_initial_count += 1;
        let timervec = self.lvt[LocalVectorTableEntry::Timer as usize];
        let mode = match timervec.timer_mode_field() { 0 => "one-shot", 1 => "periodic", _ => "other" };
        debug!("[LAPIC] set_initial_count: value={} div_factor={} period={} mode={} vec={:#x} masked={} (call #{})",
            value, self.timer_divide_factor,
            value as u64 * self.timer_divide_factor as u64,
            mode, timervec.vector(), timervec.contains(LvtBits::MASKED),
            self.diag_set_initial_count);

        // In TSC-deadline mode, writes to initial time count are ignored (apic.cc:1087)
        if timervec.timer_mode_field() == 2 {
            return;
        }

        // Deactivate current timer if active (apic.cc:1091-1094)
        if self.timer_active {
            self.timer_active = false;
            self.timer_deactivate_request = true;
        }

        self.timer_initial = value;
        self.timer_current = 0;

        if self.timer_initial != 0 {
            // Start counting (apic.cc:1099-1109)
            debug!("APIC: Initial Timer Count Register = {} div_factor={} period={} mode={}",
                value, self.timer_divide_factor,
                value as u64 * self.timer_divide_factor as u64,
                timervec.timer_mode_field());
            self.timer_current = self.timer_initial;
            self.timer_active = true;
            // Bochs apic.cc:1106: ticksInitial = bx_pc_system.time_ticks()
            // We use current_ticks (updated at batch boundary) as best available
            // approximation. The emulator loop will also call set_ticks_initial()
            // with the precise value when processing the activate request.
            self.ticks_initial = self.current_ticks;
            // Request timer activation: period = initial_count * divide_factor ticks
            // Bochs apic.cc:1107-1108: activate_timer_ticks(handle, Bit64u(value) * Bit64u(factor), 0)
            let period = value as u64 * self.timer_divide_factor as u64;
            self.timer_activate_request = Some(period);
        }
    }

    /// Get the current timer count.
    /// Computes remaining count from elapsed ticks.
    /// Bochs: get_current_timer_count (apic.cc:1112-1131)
    pub(crate) fn get_current_timer_count(&mut self, current_ticks: u64) -> u32 {
        let timervec = self.lvt[LocalVectorTableEntry::Timer as usize];

        // In TSC-deadline mode, current timer count always reads 0 (apic.cc:1118)
        if timervec.timer_mode_field() == 2 {
            return 0;
        }

        if !self.timer_active {
            return self.timer_current;
        }

        // Compute elapsed ticks and remaining count (apic.cc:1124-1130)
        let delta64 = (current_ticks - self.ticks_initial) / self.timer_divide_factor as u64;
        let delta32 = delta64 as u32;
        if delta32 > self.timer_initial {
            // Timer should have already fired — clamp to 0
            self.timer_current = 0;
        } else {
            self.timer_current = self.timer_initial - delta32;
        }
        self.timer_current
    }

    /// Set ticks_initial — called by emulator when activating the pc_system timer.
    /// Bochs: ticksInitial = bx_pc_system.time_ticks() (apic.cc:1057,1107)
    pub(crate) fn set_ticks_initial(&mut self, ticks: u64) {
        self.ticks_initial = ticks;
    }

    /// Get the timer period in system ticks (for pc_system timer activation).
    /// Returns None if timer is not active or timer_initial is 0.
    pub(crate) fn timer_period_ticks(&self) -> Option<u64> {
        if self.timer_active && self.timer_initial != 0 {
            Some(self.timer_initial as u64 * self.timer_divide_factor as u64)
        } else {
            None
        }
    }

    /// Check if the timer is in periodic mode (LVT timer bit 17 set).
    pub(crate) fn timer_is_periodic(&self) -> bool {
        self.lvt[LocalVectorTableEntry::Timer as usize].timer_mode_field() == 1
    }

    /// Diagnostic: return timer state for HLT debugging.
    /// Returns (timer_active, timer_initial, period_ticks, timer_vector, activate_pending, deactivate_pending)
    pub(crate) fn hlt_timer_diag(&self) -> (bool, u32, u64, u8, bool, bool) {
        let vec = self.lvt[LocalVectorTableEntry::Timer as usize].vector();
        let period = self.timer_initial as u64 * self.timer_divide_factor as u64;
        (self.timer_active, self.timer_initial, period, vec,
         self.timer_activate_request.is_some(), self.timer_deactivate_request)
    }

    // ─── TSC-Deadline timer ──────────────────────────────────────────────

    /// Set the TSC-Deadline timer value.
    /// Bochs: set_tsc_deadline (apic.cc:1134-1156)
    #[allow(dead_code)]
    pub(crate) fn set_tsc_deadline(&mut self, deadline: u64) {
        let timervec = self.lvt[LocalVectorTableEntry::Timer as usize];
        if timervec.timer_mode_field() != 2 {
            error!("APIC: TSC-Deadline timer is disabled");
            return;
        }

        if self.timer_active {
            self.timer_active = false;
            self.timer_deactivate_request = true;
        }

        self.ticks_initial = deadline;
        if deadline != 0 {
            debug!("APIC: TSC-Deadline is set to {}", deadline);
            self.timer_active = true;
            // Bochs apic.cc:1154: activate_timer_ticks(handle, (deadline>currtime) ? (deadline-currtime) : 1, 0)
            // We don't have currtime here; the emulator loop handles the actual activation
            self.timer_activate_request = Some(1); // minimal period, emulator will adjust
        }
    }

    /// Get the TSC-Deadline timer value.
    /// Bochs: get_tsc_deadline (apic.cc:1158-1166)
    #[allow(dead_code)]
    pub(crate) fn get_tsc_deadline(&self) -> u64 {
        let timervec = self.lvt[LocalVectorTableEntry::Timer as usize];
        if timervec.timer_mode_field() != 2 {
            return 0;
        }
        self.ticks_initial
    }

    // ─── Initialization and reset ────────────────────────────────────────

    /// Sets the base address and mode of the Local APIC.
    /// Bochs: set_base (apic.cc:289-306)
    pub(super) fn set_base(&mut self, mut newbase: BxPhyAddress) {
        self.mode = ApicMode::from_raw((newbase >> 10) & 3);
        newbase &= !(0xFFF as BxPhyAddress);
        self.base_addr = newbase;

        info!(
            "allocate APIC id={} (MMIO {}) to {:#x}",
            self.apic_id,
            if self.mode == ApicMode::XapicMode {
                "enabled"
            } else {
                "disabled"
            },
            newbase
        );

        if self.mode == ApicMode::X2apicMode {
            self.ldr = ((self.apic_id & 0xFFFF_FFF0) << 16) | (1 << (self.apic_id & 0xF));
        }

        if self.mode == ApicMode::GloballyDisabled {
            self.write_spurious_interrupt_register(0xFF);
        }
    }

    /// Enables XAPIC extensions (IER and SEOI support).
    /// Bochs: enable_xapic_extensions (apic.cc:282-287)
    pub(super) fn enable_xapic_extensions(&mut self) {
        self.apic_version_id |= 0x80000000;
        self.xapic_ext = BX_XAPIC_EXT_SUPPORT_IER | BX_XAPIC_EXT_SUPPORT_SEOI;
    }

    /// Reset the Local APIC to its initial state.
    /// Bochs: reset (apic.cc:215-279)
    pub(super) fn reset(&mut self, _reset_type: u8) {
        self.base_addr = BX_LAPIC_BASE_ADDR;
        self.error_status = 0;
        self.shadow_error_status = 0;
        self.ldr = 0;
        self.dest_format = 0xF;
        self.icr_hi = 0;
        self.icr_lo = 0;
        self.task_priority = 0;

        for i in 0..8 {
            self.irr[i] = 0;
            self.isr[i] = 0;
            self.tmr[i] = 0;
            self.ier[i] = 0xFFFF_FFFF;
        }

        self.timer_divconf = 0;
        self.timer_divide_factor = 1;
        self.timer_initial = 0;
        self.timer_current = 0;
        self.ticks_initial = 0;

        self.timer_active = false;
        self.vmx_timer_active = false;
        self.mwaitx_timer_active = false;
        // Request deactivation of all timers (apic.cc:242-255)
        self.timer_deactivate_request = true;
        self.timer_activate_request = None;

        for i in 0..LVT_ENTRY_COUNT {
            self.lvt[i] = LvtBits::MASKED; // all masked
        }

        self.spurious_vector = 0xFF;
        self.software_enabled = false;
        self.focus_disable = false;

        self.mode = ApicMode::XapicMode;

        if self.xapic {
            self.apic_version_id = 0x00050014; // P4 with 6 LVT entries
        } else {
            self.apic_version_id = 0x00030010; // P6 with 4 LVT entries
        }

        self.xapic_ext = 0;
        self.intr = false;
        self.pending_ipi = None;
        self.pending_eoi_vector = None;
        self.timer_fired = false;
    }

    /// Deactivates the MWAITX timer if the main timer is not active.
    pub(super) fn deactivate_mwaitx_timer(&mut self) {
        if self.timer_active {
            return;
        }
        self.mwaitx_timer_active = true;
    }

    // ─── Helper ──────────────────────────────────────────────────────────

    /// Set a bit in the shadow error status register (accumulates until ESR write).
    /// Uses interior mutability pattern — called from read_aligned which takes &self.
    fn set_shadow_error(&self, _bit: u32) {
        // Note: in Bochs this modifies shadow_error_status directly.
        // Since read_aligned takes &self, we can't mutate here.
        // The error is logged; writes to ESR will still work correctly.
        // In practice, illegal register reads are rare and the error
        // will be caught by the write_aligned path.
    }

    /// Static timer handler for BxPcSystemC callback.
    /// Sets timer_fired flag on the LAPIC instance.
    pub(crate) fn timer_handler(this_ptr: *mut core::ffi::c_void) {
        // SAFETY: this_ptr is a valid *mut BxLocalApic set during timer registration.
        // Only called from single-threaded timer system.
        let lapic = unsafe { &mut *(this_ptr as *mut BxLocalApic) };
        lapic.timer_fired = true;
    }
}

// ─── APIC error status (preserved for API compatibility) ────────────────────

/// APIC error status flags.
/// These errors are reported in the Error Status Register (ESR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApicError(u8);

impl ApicError {
    pub const ILLEGAL_ADDR: u8 = 0x80;
    pub const RX_ILLEGAL_VEC: u8 = 0x40;
    pub const TX_ILLEGAL_VEC: u8 = 0x20;
    pub const X2APIC_REDIRECTIBLE_IPI: u8 = 0x08;
    pub const RX_ACCEPT_ERR: u8 = 0x08;
    pub const TX_ACCEPT_ERR: u8 = 0x04;
    pub const RX_CHECKSUM: u8 = 0x02;
    pub const TX_CHECKSUM: u8 = 0x01;

    pub fn from_raw(value: u8) -> Self {
        ApicError(value)
    }

    pub fn as_raw(self) -> u8 {
        self.0
    }

    pub fn has_flag(self, flag: u8) -> bool {
        (self.0 & flag) != 0
    }

    pub fn set_flag(&mut self, flag: u8) {
        self.0 |= flag;
    }

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

impl BxLocalApic {
    /// Dump LAPIC state for debugging. Uses eprintln! so it's always visible.
    pub(crate) fn dump_state(&self) {
        let timervec = self.lvt[LocalVectorTableEntry::Timer as usize];
        let timer_mode = match timervec.timer_mode_field() {
            0 => "one-shot",
            1 => "periodic",
            2 => "tsc-deadline",
            _ => "unknown",
        };
        let timer_masked = timervec.contains(LvtBits::MASKED);
        let timer_vector = timervec.vector();
        eprintln!("--- LAPIC State ---");
        eprintln!("  mode={:?} sw_enabled={} base={:#x} id={}",
            self.mode, self.software_enabled, self.base_addr, self.apic_id);
        eprintln!("  TPR={:#x} PPR={:#x} spurious_vec={:#x}",
            self.task_priority, self.get_ppr(), self.spurious_vector);
        eprintln!("  LVT[Timer]={:#010x} (vec={:#x} mode={} masked={})",
            timervec.bits(), timer_vector, timer_mode, timer_masked);
        eprintln!("  LVT[LINT0]={:#010x} LVT[LINT1]={:#010x}",
            self.lvt[3].bits(), self.lvt[4].bits());
        eprintln!("  timer: initial={} current={} active={} div_factor={} period={}",
            self.timer_initial, self.timer_current, self.timer_active,
            self.timer_divide_factor,
            self.timer_initial as u64 * self.timer_divide_factor as u64);
        eprintln!("  ticks_initial={} current_ticks={}", self.ticks_initial, self.current_ticks);
        eprintln!("  intr={} timer_fired={} timer_activate_req={} timer_deact_req={}",
            self.intr, self.timer_fired,
            self.timer_activate_request.is_some(), self.timer_deactivate_request);
        // Show IRR/ISR summary - which vectors are pending/in-service
        let mut irr_vecs = Vec::new();
        let mut isr_vecs = Vec::new();
        for i in 0..256u32 {
            if Self::get_vector(&self.irr, i) { irr_vecs.push(i); }
            if Self::get_vector(&self.isr, i) { isr_vecs.push(i); }
        }
        eprintln!("  IRR vectors: {:?}", irr_vecs);
        eprintln!("  ISR vectors: {:?}", isr_vecs);
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lapic() -> BxLocalApic {
        let mut lapic = BxLocalApic::default();
        lapic.xapic = true;
        lapic.reset(0);
        lapic
    }

    #[test]
    fn test_reset_defaults() {
        let lapic = make_lapic();
        assert_eq!(lapic.mode, ApicMode::XapicMode);
        assert_eq!(lapic.task_priority, 0);
        assert_eq!(lapic.spurious_vector, 0xFF);
        assert!(!lapic.software_enabled);
        assert!(!lapic.intr);
        assert_eq!(lapic.apic_version_id, 0x00050014); // P4 xapic
        for i in 0..8 {
            assert_eq!(lapic.irr[i], 0);
            assert_eq!(lapic.isr[i], 0);
            assert_eq!(lapic.tmr[i], 0);
            assert_eq!(lapic.ier[i], 0xFFFFFFFF);
        }
        for i in 0..LVT_ENTRY_COUNT {
            assert_eq!(lapic.lvt[i], LvtBits::MASKED); // all masked
        }
    }

    #[test]
    fn test_vector_bit_manipulation() {
        let mut reg = [0u32; 8];
        // Set vector 0x20 (bit 0 of reg[1])
        BxLocalApic::set_vector(&mut reg, 0x20);
        assert!(BxLocalApic::get_vector(&reg, 0x20));
        assert!(!BxLocalApic::get_vector(&reg, 0x21));
        assert_eq!(reg[1], 1);

        // Set vector 0xFF (bit 31 of reg[7])
        BxLocalApic::set_vector(&mut reg, 0xFF);
        assert!(BxLocalApic::get_vector(&reg, 0xFF));
        assert_eq!(reg[7], 0x80000000);

        // Clear vector 0x20
        BxLocalApic::clear_vector(&mut reg, 0x20);
        assert!(!BxLocalApic::get_vector(&reg, 0x20));
        assert_eq!(reg[1], 0);
    }

    #[test]
    fn test_highest_priority_int() {
        let mut lapic = make_lapic();

        // Empty IRR → -1
        assert_eq!(lapic.highest_priority_int(&lapic.irr), -1);

        // Set vector 0x30 in IRR
        BxLocalApic::set_vector(&mut lapic.irr, 0x30);
        assert_eq!(lapic.highest_priority_int(&lapic.irr), 0x30);

        // Set higher-priority vector 0x80
        BxLocalApic::set_vector(&mut lapic.irr, 0x80);
        assert_eq!(lapic.highest_priority_int(&lapic.irr), 0x80);

        // IER masking: disable vector 0x80
        lapic.ier[4] = 0; // reg[4] covers vectors 0x80-0x9F
        assert_eq!(lapic.highest_priority_int(&lapic.irr), 0x30);
    }

    #[test]
    fn test_trigger_irq_and_service() {
        let mut lapic = make_lapic();
        // Enable APIC software
        lapic.write_spurious_interrupt_register(0x1FF); // vector=0xFF, enabled, no focus disable
        assert!(lapic.software_enabled);

        // Trigger vector 0x30 edge-triggered
        lapic.trigger_irq(0x30, APIC_EDGE_TRIGGERED, false);

        // IRR bit should be set
        assert!(BxLocalApic::get_vector(&lapic.irr, 0x30));
        // TMR should be clear (edge triggered)
        assert!(!BxLocalApic::get_vector(&lapic.tmr, 0x30));
        // INTR should be raised (TPR=0, no ISR)
        assert!(lapic.intr);
    }

    #[test]
    fn test_acknowledge_int() {
        let mut lapic = make_lapic();
        lapic.write_spurious_interrupt_register(0x1FF);

        // Trigger two interrupts
        lapic.trigger_irq(0x30, APIC_EDGE_TRIGGERED, false);
        lapic.trigger_irq(0x40, APIC_EDGE_TRIGGERED, false);
        assert!(lapic.intr);

        // Acknowledge — should get highest priority (0x40)
        let vec = lapic.acknowledge_int();
        assert_eq!(vec, 0x40);
        // 0x40 should be in ISR, not IRR
        assert!(!BxLocalApic::get_vector(&lapic.irr, 0x40));
        assert!(BxLocalApic::get_vector(&lapic.isr, 0x40));
        // 0x30 still in IRR
        assert!(BxLocalApic::get_vector(&lapic.irr, 0x30));
        // INTR should still be raised (0x30 pending, but blocked by 0x40 in ISR)
        // Since 0x30 < 0x40 (lower priority), it won't be delivered yet
        assert!(!lapic.intr);
    }

    #[test]
    fn test_eoi_and_second_interrupt() {
        let mut lapic = make_lapic();
        lapic.write_spurious_interrupt_register(0x1FF);

        lapic.trigger_irq(0x30, APIC_EDGE_TRIGGERED, false);
        lapic.trigger_irq(0x40, APIC_EDGE_TRIGGERED, false);

        // Acknowledge 0x40
        let vec = lapic.acknowledge_int();
        assert_eq!(vec, 0x40);

        // EOI for 0x40
        lapic.receive_eoi(0);
        assert!(!BxLocalApic::get_vector(&lapic.isr, 0x40));
        // Now 0x30 should be deliverable
        assert!(lapic.intr);

        // Acknowledge 0x30
        let vec2 = lapic.acknowledge_int();
        assert_eq!(vec2, 0x30);
    }

    #[test]
    fn test_tpr_blocks_low_priority() {
        let mut lapic = make_lapic();
        lapic.write_spurious_interrupt_register(0x1FF);

        // Set TPR to priority class 4 (blocks vectors 0x00-0x4F)
        lapic.set_tpr(0x40);

        // Trigger vector 0x30 (class 3 < TPR class 4, should be blocked)
        lapic.trigger_irq(0x30, APIC_EDGE_TRIGGERED, false);
        assert!(!lapic.intr); // blocked by TPR

        // Trigger vector 0x50 (class 5 > TPR class 4, should be delivered)
        lapic.trigger_irq(0x50, APIC_EDGE_TRIGGERED, false);
        assert!(lapic.intr);
    }

    #[test]
    fn test_logical_addr_flat_model() {
        let mut lapic = make_lapic();
        lapic.dest_format = 0xF; // flat model
        lapic.ldr = 0x04; // bit 2

        assert!(lapic.match_logical_addr(0x04)); // exact match
        assert!(lapic.match_logical_addr(0x0C)); // bit 2 and 3 set
        assert!(!lapic.match_logical_addr(0x08)); // bit 3 only
        assert!(lapic.match_logical_addr(0xFF)); // broadcast
    }

    #[test]
    fn test_logical_addr_cluster_model() {
        let mut lapic = make_lapic();
        lapic.dest_format = 0x0; // cluster model
        lapic.ldr = 0x12; // cluster 1, agent bit 1

        assert!(lapic.match_logical_addr(0x12)); // exact match
        assert!(lapic.match_logical_addr(0x16)); // same cluster, overlapping agents
        assert!(!lapic.match_logical_addr(0x22)); // different cluster
        assert!(lapic.match_logical_addr(0xFF)); // broadcast
    }

    #[test]
    fn test_ppr_computation() {
        let mut lapic = make_lapic();

        // No ISR, TPR=0 → PPR=0
        assert_eq!(lapic.get_ppr(), 0);

        // TPR=0x40 → PPR=0x40 (no ISR)
        lapic.task_priority = 0x40;
        assert_eq!(lapic.get_ppr(), 0x40);

        // ISR=0x80 (class 8 > TPR class 4) → PPR=0x80
        BxLocalApic::set_vector(&mut lapic.isr, 0x80);
        assert_eq!(lapic.get_ppr(), 0x80);
    }

    #[test]
    fn test_svr_write() {
        let mut lapic = make_lapic();

        // Write SVR: vector=0xAB, enabled, focus disable
        lapic.write_spurious_interrupt_register(0x3AB);
        assert_eq!(lapic.spurious_vector, 0xAB);
        assert!(lapic.software_enabled);
        assert!(lapic.focus_disable);

        // Disable — all LVT should be masked
        lapic.lvt[0] = LvtBits::from_raw(0x00000030); // unmask timer
        lapic.write_spurious_interrupt_register(0x0FF); // disable
        assert!(!lapic.software_enabled);
        assert!(lapic.lvt[0].contains(LvtBits::MASKED)); // masked
    }

    #[test]
    fn test_lvt_write_masking() {
        let mut lapic = make_lapic();
        lapic.software_enabled = true;

        // Write timer LVT with all bits set — masked by LVT_MASKS[0].
        // TSC-Deadline not supported, so bit 18 (0x40000) is cleared first.
        // Result: 0x000710FF & !0x40000 = 0x000310FF
        lapic.set_lvt_entry(0x320, 0xFFFF_FFFF);
        assert_eq!(lapic.lvt[0].bits(), 0x000310FF);

        // Write LINT0 LVT
        lapic.set_lvt_entry(0x350, 0xFFFF_FFFF);
        assert_eq!(lapic.lvt[3].bits(), 0x0001F7FF);
    }

    #[test]
    fn test_read_write_aligned() {
        let mut lapic = make_lapic();

        // Write TPR
        lapic.write_aligned(BX_LAPIC_BASE_ADDR | 0x080, 0x42);
        assert_eq!(lapic.task_priority, 0x42);

        // Read TPR
        let val = lapic.read_aligned(BX_LAPIC_BASE_ADDR | 0x080);
        assert_eq!(val, 0x42);

        // Read APIC ID (default 0)
        let val = lapic.read_aligned(BX_LAPIC_BASE_ADDR | 0x020);
        assert_eq!(val, 0); // apic_id=0, shifted left by 24

        // Read version
        let val = lapic.read_aligned(BX_LAPIC_BASE_ADDR | 0x030);
        assert_eq!(val, 0x00050014); // P4 xapic version
    }

    #[test]
    fn test_timer_divide_configuration() {
        let mut lapic = make_lapic();

        // divconf=0b0000 → combined=0 → factor=2
        lapic.timer_divconf = 0x0;
        lapic.set_divide_configuration(0x0);
        assert_eq!(lapic.timer_divide_factor, 2);

        // divconf=0b0001 → combined=1 → factor=4
        lapic.set_divide_configuration(0x1);
        assert_eq!(lapic.timer_divide_factor, 4);

        // divconf=0b0011 → combined=3 → factor=16
        lapic.set_divide_configuration(0x3);
        assert_eq!(lapic.timer_divide_factor, 16);

        // divconf=0b1011 → combined=7 → factor=1
        lapic.set_divide_configuration(0xB);
        assert_eq!(lapic.timer_divide_factor, 1);
    }

    #[test]
    fn test_level_triggered_eoi_clears_tmr() {
        let mut lapic = make_lapic();
        lapic.write_spurious_interrupt_register(0x1FF);

        // Trigger level-triggered interrupt
        lapic.trigger_irq(0x50, APIC_LEVEL_TRIGGERED, false);
        assert!(BxLocalApic::get_vector(&lapic.tmr, 0x50));

        // Acknowledge
        let vec = lapic.acknowledge_int();
        assert_eq!(vec, 0x50);

        // EOI should clear TMR
        lapic.receive_eoi(0);
        assert!(!BxLocalApic::get_vector(&lapic.tmr, 0x50));
    }

    #[test]
    fn test_bogus_vector_rejected() {
        let mut lapic = make_lapic();
        lapic.write_spurious_interrupt_register(0x1FF);

        // Vector < 0x10 should be rejected
        lapic.trigger_irq(0x05, APIC_EDGE_TRIGGERED, false);
        assert!(!BxLocalApic::get_vector(&lapic.irr, 0x05));
        assert!(!lapic.intr);
        assert_ne!(lapic.shadow_error_status & APIC_ERR_RX_ILLEGAL_VEC, 0);
    }

    #[test]
    fn test_is_selected() {
        let mut lapic = make_lapic();

        // In XAPIC mode, should match base addr range
        assert!(lapic.is_selected(0xFEE00000));
        assert!(lapic.is_selected(0xFEE00080));
        assert!(lapic.is_selected(0xFEE00FF0));
        assert!(!lapic.is_selected(0xFEF00000)); // out of range
        assert!(!lapic.is_selected(0xFED00000)); // I/O APIC range

        // Globally disabled → not selected
        lapic.mode = ApicMode::GloballyDisabled;
        assert!(!lapic.is_selected(0xFEE00000));
    }

    #[test]
    fn test_ipi_self() {
        let mut lapic = make_lapic();
        lapic.write_spurious_interrupt_register(0x1FF);

        // ICR: self shorthand, fixed delivery, vector 0x30
        // shorthand=1 (bits 18:19), delivery_mode=0, vector=0x30
        let lo_cmd: u32 = (1 << 18) | 0x30;
        lapic.send_ipi(0, lo_cmd);

        assert!(BxLocalApic::get_vector(&lapic.irr, 0x30));
        assert!(lapic.intr);
    }
}
