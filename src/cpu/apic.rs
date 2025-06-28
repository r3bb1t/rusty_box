use crate::config::BxPhyAddress;


pub const APIC_EDGE_TRIGGERED: u8 = 0;
pub const APIC_LEVEL_TRIGGERED: u8 = 1;

pub const BX_LAPIC_BASE_ADDR: BxPhyAddress = 0xfee00000;

// TODO: add BX_NUM_LOCAL_APICS

// TODO: make it enum

#[derive(Debug)]
pub enum ApicState {
    BxApicGloballyDisabled = 0,
    BxApicStateInvalid = 1,
    BxApicXapicMode = 2,
    BxApicX2apicMode = 3,
}

pub const BX_XAPIC_EXT_SUPPORT_IER: u32 = 1 << 0;
pub const BX_XAPIC_EXT_SUPPORT_SEOI: u32 = 1 << 1;

pub type ApicDest = u32;

#[derive(Debug)]
pub enum LapicRegister {
    BxLapicId = 0x020,
    BxLapicVersion = 0x030,
    BxLapicTpr = 0x080,
    BxLapicArbitrationPriority = 0x090,
    BxLapicPpr = 0x0A0,
    BxLapicEoi = 0x0B0,
    BxLapicRrd = 0x0C0,
    BxLapicLdr = 0x0D0,
    BxLapicDestinationFormat = 0x0E0,
    BxLapicSpuriousVector = 0x0F0,
    BxLapicIsr1 = 0x100,
    BxLapicIsr2 = 0x110,
    BxLapicIsr3 = 0x120,
    BxLapicIsr4 = 0x130,
    BxLapicIsr5 = 0x140,
    BxLapicIsr6 = 0x150,
    BxLapicIsr7 = 0x160,
    BxLapicIsr8 = 0x170,
    BxLapicTmr1 = 0x180,
    BxLapicTmr2 = 0x190,
    BxLapicTmr3 = 0x1A0,
    BxLapicTmr4 = 0x1B0,
    BxLapicTmr5 = 0x1C0,
    BxLapicTmr6 = 0x1D0,
    BxLapicTmr7 = 0x1E0,
    BxLapicTmr8 = 0x1F0,
    BxLapicIrr1 = 0x200,
    BxLapicIrr2 = 0x210,
    BxLapicIrr3 = 0x220,
    BxLapicIrr4 = 0x230,
    BxLapicIrr5 = 0x240,
    BxLapicIrr6 = 0x250,
    BxLapicIrr7 = 0x260,
    BxLapicIrr8 = 0x270,
    BxLapicEsr = 0x280,
    BxLapicLvtCmci = 0x2F0,
    BxLapicIcrLo = 0x300,
    BxLapicIcrHi = 0x310,
    BxLapicLvtTimer = 0x320,
    BxLapicLvtThermal = 0x330,
    BxLapicLvtPerfmon = 0x340,
    BxLapicLvtLint0 = 0x350,
    BxLapicLvtLint1 = 0x360,
    BxLapicLvtError = 0x370,
    BxLapicTimerInitialCount = 0x380,
    BxLapicTimerCurrentCount = 0x390,
    BxLapicTimerDivideCfg = 0x3E0,
    BxLapicSelfIpi = 0x3F0,

    // extended AMD
    BxLapicExtApicFeature = 0x400,
    BxLapicExtApicControl = 0x410,
    BxLapicSpecificEoi = 0x420,
    BxLapicIer1 = 0x480,
    BxLapicIer2 = 0x490,
    BxLapicIer3 = 0x4A0,
    BxLapicIer4 = 0x4B0,
    BxLapicIer5 = 0x4C0,
    BxLapicIer6 = 0x4D0,
    BxLapicIer7 = 0x4E0,
    BxLapicIer8 = 0x4F0,
}

#[derive(Debug)]
pub enum APicDeliveryMode {
    ApicDmFixed = 0,
    ApicDmLowpri = 1,
    ApicDmSmi = 2,
    ApicDmReserved = 3,
    ApicDmNmi = 4,
    ApicDmInit = 5,
    ApicDmSipi = 6,
    ApicDmExtint = 7,
}

// (LVT)
#[derive(Debug)]
pub enum LocalVectorTableRegister {
    ApicLvtTimer = 0,
    ApicLvtThermal = 1,
    ApicLvtPerfmon = 2,
    ApicLvtLint0 = 3,
    ApicLvtLint1 = 4,
    ApicLvtError = 5,
    ApicLvtCmci = 6,
    ApicLvtEntries,
}

#[derive(Debug, Default)]
pub struct BxLocalApic {
    base_addr: BxPhyAddress,
    mode: u32,
    xapic: bool,

    xapic_ext: u32, // enabled extended XAPIC features
    ///  4 bit in legacy mode, 8 bit in XAPIC mode
    /// 32 bit in X2APIC mode
    apic_id: u32,
    apic_version_id: u32,
    software_enabled: bool,
    spurious_vector: u8,
    focus_disable: bool,

    /// Task priority (TPR)
    task_priority: u32,
    /// Logical destination (LDR)
    ldr: u32,
    /// Destination format (DFR)
    dest_format: u32,

    /// ISR=in-service register. When an IRR bit is cleared, the corresponding
    /// bit in ISR is set.
    isr: [u32; 8],
    /// TMR=trigger mode register.  Cleared for edge-triggered interrupts
    /// and set for level-triggered interrupts. If set, local APIC must send
    /// EOI message to all other APICs.
    tmr: [u32; 8],
    /// IRR=interrupt request register. When an interrupt is triggered by
    /// the I/O APIC or another processor, it sets a bit in irr. The bit is
    /// cleared when the interrupt is acknowledged by the processor.
    irr: [u32; 8],

    /// IER=interrupt enable register. Only vectors that are enabled in IER
    /// participare in APIC's computation of highest priority pending interrupt.
    ier: [u32; 8],

    // Error status Register (ESR)
    error_status: u32,
    shadow_error_status: u32,

    /// Interrupt command register (ICR)
    icr_hi: u32,
    icr_lo: u32,

    lvt: [u32; LocalVectorTableRegister::ApicLvtEntries as _],
    /// Initial timer count (in order to reload periodic timer)
    timer_initial: u32,
    /// Current timer count
    timer_current: u32,
    /// Timer value when it started to count, also holds TSC-Deadline value
    ticks_initial: u64,

    /// Timer divide configuration register
    timer_divconf: u32,
    timer_divide_factor: u32,

    /// Internal timer state, not accessible from bus
    timer_active: bool,
    timer_handle: i32,

    vmx_timer_handle: i32,
    vmx_preemption_timer_value: u32,
    /// The value of system tick when set the timer (absolute value)
    vmx_preemption_timer_initial: u64,
    /// The value of system tick when fire the exception (absolute value)
    vmx_preemption_timer_fire: u64,
    /// rate stated in MSR_VMX_MISC
    vmx_preemption_timer_rate: u32,
    vmx_timer_active: bool,

    mwaitx_timer_handle: i32,
    mwaitx_timer_active: bool,
    // ???
    //cpu: &'c BxCpuC<'c, I>,
}

#[derive(Debug)]
enum ApicError {
    ApicErrIllegalAddr,
    ApicErrRxIllegalVec,
    ApicErrTxIllegalVec,
    X2apicErrRedirectibleIpi,
    ApicErrRxAcceptErr,
    ApicErrTxAcceptErr,
    ApicErrRxChecksum,
    ApicErrTxChecksum,
}

// Hack since it returns 0x08 in two variants
impl From<ApicError> for u8 {
    fn from(value: ApicError) -> Self {
        match value {
            ApicError::ApicErrIllegalAddr => 0x80,
            ApicError::ApicErrRxIllegalVec => 0x40,
            ApicError::ApicErrTxIllegalVec => 0x20,
            ApicError::X2apicErrRedirectibleIpi => 0x08,
            ApicError::ApicErrRxAcceptErr => 0x08,
            ApicError::ApicErrTxAcceptErr => 0x04,
            ApicError::ApicErrRxChecksum => 0x02,
            ApicError::ApicErrTxChecksum => 0x01,
        }
    }
}
