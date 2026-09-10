//! Ask this host's Windows Hypervisor Platform the questions a design cannot
//! answer from documentation.
//!
//! Every answer below is measured, not assumed. Each experiment builds a
//! partition, runs a real-mode guest of a handful of hand-assembled bytes, and
//! reports exactly what came back — including when what came back is not what
//! the design expected, which is the outcome worth having.
//!
//! Run it with:
//!
//! ```text
//! cargo run --release -p rusty_box_whp --example whp_probe
//! ```
//!
//! On a host without the platform it prints one line saying so and exits 0;
//! there is nothing to measure and that is not a failure.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use rusty_box_whp::{
    capabilities, hypervisor_present, ApicRegister, ApicStatePage, ApicVector, Capabilities,
    DestinationMode, Exit, ExitReason, ExtendedVmExits, GpaPerms, HostPages, InternalActivity,
    InterruptKind, InterruptRequest, InterruptionType, LateProperty, LocalApicMode, Partition,
    PartitionConfig, PendingExtIntEvent, PendingInterruption, Reg, SegmentRegister,
    SyntheticFeatures, TriggerMode, Vcpu, WhpResult, PAGE_SIZE,
};

/// The guest-physical map every experiment shares.
mod layout {
    /// Low RAM: interrupt vectors at 0, code at [`CODE`], a scratch byte the
    /// guest writes at [`MARKER`], stack below 64 KiB.
    pub const RAM: u64 = 0x0000_0000;
    pub const RAM_PAGES: usize = 16;
    pub const CODE: u64 = 0x0000_1000;
    /// Where an interrupt handler writes its proof of having run.
    pub const MARKER: u16 = 0x0800;
    /// Where the code after a resumed `HLT` writes its own.
    pub const RESUMED: u16 = 0x0700;
    pub const HANDLER: u64 = 0x0000_1500;
    /// Where a fault handler lives, so an instruction the guest may not be
    /// allowed to execute lands somewhere that says so instead of running off
    /// into a zeroed interrupt-vector table.
    pub const FAULT_HANDLER: u64 = 0x0000_1600;
    pub const STACK_TOP: u64 = 0x0000_3000;
    /// One page mapped read-and-execute only: a shadowed ROM, or a chipset
    /// region whose writes the host must see.
    pub const READ_ONLY: u64 = 0x0001_0000;
    /// Never mapped, so every access is a second-level fault: device MMIO.
    pub const MMIO: u64 = 0x0002_0000;
    /// Where the mapping-churn experiment puts its pages.
    pub const CHURN: u64 = 0x0010_0000;
    /// The architectural xAPIC page. Never mapped by this probe: when the
    /// hypervisor emulates the APIC it owns these addresses, and when it does
    /// not a guest access there is an ordinary unmapped-range fault — which is
    /// itself the observation that separates the two.
    pub const XAPIC: u64 = 0xFEE0_0000;
}

/// One question and what the host said.
struct Finding {
    question: &'static str,
    answer: String,
    detail: String,
}

fn main() {
    match hypervisor_present() {
        Ok(true) => {}
        Ok(false) => {
            println!("No Windows Hypervisor Platform on this host — nothing to probe.");
            return;
        }
        Err(err) => {
            println!("Could not ask whether a hypervisor is present: {err}");
            return;
        }
    }

    let caps = match capabilities() {
        Ok(caps) => caps,
        Err(err) => {
            println!("Hypervisor present but capabilities refused: {err}");
            return;
        }
    };

    println!("== host ==");
    println!("  features word            {:#018x}", caps.features.raw);
    println!("  partial unmap            {}", caps.features.partial_unmap);
    println!("  local APIC emulation     {}", caps.features.local_apic_emulation);
    println!("  dirty page tracking      {}", caps.features.dirty_page_tracking);
    println!("  idle suspend             {}", caps.features.idle_suspend);
    println!("  physical address width   {} bits", caps.physical_address_width);
    // The decoded struct and the word it came from, together. A decode can only
    // report a bit it has a field for, so the remainder is what this host
    // offers and this port cannot ask for — the one number that makes the
    // decoded list checkable rather than merely believable.
    println!("  exits word               {:#018x}", caps.extended_exits_raw);
    println!(
        "  exits offered and unnamed by this port {:#018x}",
        caps.extended_exits_raw & !caps.supported_exits.as_word()
    );
    println!("  exits offered            {:?}", caps.supported_exits);
    println!("  processor features       {:#018x}", caps.processor_features);
    println!("  synthetic features       {:#018x}", caps.synthetic_features.bits());
    println!("  processor clock          {} Hz", caps.processor_clock_hz);
    println!("  interrupt clock          {} Hz", caps.interrupt_clock_hz);
    println!("  TSC deadline timer       {}", caps.tsc_deadline_timer);
    // Every duration this probe reports is read beside this number. A host
    // busy with someone else's work hands a sleeping thread its processor back
    // late, and a figure measured through that delay is a fact about the host's
    // load rather than about the platform.
    println!("  scheduling delay         {:?} (mean overshoot of ten 5 ms sleeps)", scheduling_delay());
    println!("  available parallelism    {}", available_parallelism());
    println!();

    let mut findings = Vec::new();
    let mut push = |result: WhpResult<Finding>, question: &'static str| match result {
        Ok(finding) => findings.push(finding),
        Err(err) => findings.push(Finding {
            question,
            answer: "EXPERIMENT FAILED".into(),
            detail: err.to_string(),
        }),
    };

    push(q1_read_only_window(caps.supported_exits.gpa_access_fault), Q1);
    push(q2_instruction_bytes_and_translation(), Q2);
    push(q3_halt_resume(), Q3);
    push(q4_exit_latency(), Q4);
    push(q5_mapping_cost(caps.features.partial_unmap), Q5);
    push(q6_separate_security_domain(), Q6);
    push(q7_cpuid_and_late_properties(caps.supported_exits.cpuid), Q7);
    push(q8_cancel_stickiness(), Q8);
    push(q9_mapping_churn(), Q9);
    push(q10_wake_under_an_emulated_apic(caps.features.local_apic_emulation), Q10);
    push(q11_halt_suspend_clear_under_x2apic(), Q11);
    push(q12_halt_exit_under_x2apic(), Q12);
    push(q13_cancel_stickiness_under_x2apic(), Q13);
    push(q14_lint0_trap_and_ext_int_gating(caps.supported_exits.apic_write_lint0_trap), Q14);
    push(q15_synthetic_bank_and_hv1(caps), Q15);
    push(q16_tsc_deadline_and_apic_clock(caps), Q16);
    push(q17_partition_time_and_the_tsc(), Q17);
    push(q18_in_service_versus_trigger_mode(), Q18);
    push(q19_the_apic_page_write_path(), Q19);
    push(q20_a_software_disabled_apic_drops_a_vector(), Q20);
    push(q21_the_interrupt_window_under_an_emulated_apic(), Q21);

    println!("== answers ==");
    for finding in &findings {
        println!("\n{}\n  -> {}", finding.question, finding.answer);
        for line in finding.detail.lines() {
            println!("     {line}");
        }
    }
}

const Q1: &str = "Q1  Does a guest write to a read-and-execute window exit, and is\n    \
                  ExtendedVmExits.GpaAccessFaultExit needed for it?";
const Q2: &str = "Q2  Does a memory-access exit carry the instruction bytes, and does\n    \
                  WHvTranslateGva work in real mode?";
const Q3: &str = "Q3  Under LocalApicEmulationMode::None, does a halted processor resume\n    \
                  on PendingInterruption alone, or must HaltSuspend be cleared?";
const Q4: &str = "Q4  What does one exit cost?";
const Q5: &str = "Q5  What does mapping cost, and can a sub-range be unmapped?";
const Q6: &str = "Q6  What does SeparateSecurityDomain buy?";
const Q7: &str = "Q7  Can CPUID leaves be answered by the host, including the\n    \
                  hypervisor-present bit, and are properties really pre-setup-only?";
const Q8: &str = "Q8  Is WHvCancelRunVirtualProcessor sticky when the processor is not\n    \
                  running?";
const Q9: &str = "Q9  How many separate mappings does a partition tolerate, and what\n    \
                  does each cost?";
const Q10: &str = "Q10 Under LocalApicEmulationMode::XApic — the mode a guest with a real\n    \
                   kernel needs — can the host still wake a halted processor, and by\n    \
                   which route?";
const Q11: &str = "P1  Under X2Apic, is HaltSuspend clearable, and does clearing it wake a\n    \
                   parked processor so it takes a host-placed pending ExtINT?";
const Q12: &str = "P3  Under X2Apic, does a HLT produce an exit at all?";
const Q13: &str = "P4  Under X2Apic, is a cancel issued while the processor is stopped still\n    \
                   sticky?";
const Q14: &str = "P5  Does a guest write to LINT0 trap, and is a host-placed pending ExtINT\n    \
                   gated by what LINT0 holds?";
const Q15: &str = "P6  Is the synthetic feature bank accepted, and does the guest then see\n    \
                   the Hv#1 interface?";
const Q16: &str = "P7  What are the platform's clocks, and do the guest's enlightened\n    \
                   frequency MSRs agree with the capability answers?";
const Q17: &str = "P8  Does suspending partition time freeze the guest's TSC?";
const Q18: &str = "P9  Which of the state page's two remaining 256-bit bitmaps is in-service\n    \
                   and which is trigger-mode?";
const Q19: &str = "P2  Does the APIC state page round-trip through the platform, and is it\n    \
                   the ONLY write path — or are the named APIC registers writable too?";
const Q20: &str = "P2b Does a software-disabled APIC DROP a requested vector, or latch it for\n    \
                   later delivery?";

const Q21: &str = "P10 Does an armed deliverability notification ever produce an\n    \
                   interrupt-window exit under an emulated local APIC — and does the\n    \
                   answer depend on the APIC mode, the priority, or when it is armed?";

/// One arming of the deliverability notification, and what came of it.
///
/// The guest spins with interrupts OFF for long enough that the notification is
/// certainly in force, then enables them and spins forever. A working
/// notification leaves the run with an interrupt-window exit at the `sti`; a
/// notification that does nothing leaves the rescue as the only way out.
///
/// The `sti` matters more than it looks: this asks the platform to report a
/// TRANSITION into interruptibility, which is the only thing the engine's
/// legacy path could use. A guest that was interruptible all along would not
/// distinguish "never fires" from "nothing to report".
fn window_after_sti(apic: LocalApicMode, armed: u64) -> WhpResult<String> {
    /// Long enough for a working notification to have fired many times over.
    const RESCUE_AFTER: Duration = Duration::from_millis(400);

    let mut configured = Ok(());
    let mut guest = Guest::new(|config| {
        if let Err(err) = config.local_apic(apic) {
            configured = Err(err.to_string());
        }
        Ok(())
    })?;
    if let Err(why) = configured {
        return Ok(format!("{apic:?}: the platform refused the mode ({why})"));
    }
    // cli                          FA
    // mov ecx, 0x0010_0000         66 B9 00 00 10 00
    // dec ecx                      66 49          <- the delay, with IF still 0
    // jnz -4                       75 FC
    // sti                          FB             <- the transition being measured
    // jmp $                        EB FE
    guest.load(&[
        0xFA, 0x66, 0xB9, 0x00, 0x00, 0x10, 0x00, 0x66, 0x49, 0x75, 0xFC, 0xFB, 0xEB, 0xFE,
    ])?;
    // Every step below is reported rather than propagated: a mode that refuses
    // the register is an ANSWER to this question, and a `?` here would throw
    // away the three arms that had not run yet.
    if let Err(err) = guest.vcpu.write_reg(Reg::DeliverabilityNotifications, armed) {
        return Ok(format!("{apic:?} armed {armed:#06x} -> the platform REFUSED the write: {err}"));
    }
    let read_back = match guest.vcpu.read_reg(Reg::DeliverabilityNotifications) {
        Ok(word) => word,
        Err(err) => {
            return Ok(format!("{apic:?} armed {armed:#06x} -> write took, read refused: {err}"))
        }
    };
    let run = run_with_rescue(&mut guest, RESCUE_AFTER)?;
    let window = matches!(run.exit.reason, ExitReason::InterruptWindow);
    Ok(format!(
        "{apic:?} armed {armed:#06x} read back {read_back:#06x} -> {} {}{}",
        reason_name(&run.exit.reason),
        if run.rescued { "(rescued)" } else { "(left on its own)" },
        if window { "  ** WINDOW **" } else { "" },
    ))
}

/// Whether the interrupt-window notification is usable at all under an
/// emulated APIC, and if not, which variable kills it.
///
/// Four arms, chosen so a failure names its own cause rather than leaving one:
/// `None` mode is the control that proves the guest and the arming are right,
/// and the three emulated-mode arms vary only the priority field and the mode.
/// QEMU arms this register in both APIC modes with no fallback path at all, so
/// either it works and this port is arming it wrongly, or QEMU's own legacy
/// path depends on something that does not hold here.
fn q21_the_interrupt_window_under_an_emulated_apic() -> WhpResult<Finding> {
    // Bit 1 is `InterruptNotification`; bits 5:2 are `InterruptPriority`.
    const NOTIFY_ANY: u64 = 0b10;
    const NOTIFY_PRIO_15: u64 = 0b10 | (15 << 2);

    let mut detail = String::new();
    let mut window_in_emulated_mode = false;
    let mut window_in_none_mode = false;
    for (apic, armed) in [
        (LocalApicMode::None, NOTIFY_ANY),
        (LocalApicMode::X2Apic, NOTIFY_ANY),
        (LocalApicMode::X2Apic, NOTIFY_PRIO_15),
        (LocalApicMode::XApic, NOTIFY_ANY),
    ] {
        let line = window_after_sti(apic, armed)?;
        if line.contains("** WINDOW **") {
            if apic == LocalApicMode::None {
                window_in_none_mode = true;
            } else {
                window_in_emulated_mode = true;
            }
        }
        detail.push_str(&line);
        detail.push('\n');
    }

    let answer = match (window_in_none_mode, window_in_emulated_mode) {
        (_, true) => "YES — the notification DOES fire under an emulated APIC; an engine that \
                      saw none was arming it wrongly"
            .to_string(),
        (true, false) => "NO — it fires under `None` and never under an emulated APIC, on the \
                          same guest with the same arming. The mode is the variable."
            .to_string(),
        (false, false) => "INCONCLUSIVE — no window exit in ANY mode, including the `None` \
                           control, so this experiment measured the guest or the arming rather \
                           than the platform"
            .to_string(),
    };
    Ok(Finding { question: Q21, answer, detail })
}

// ---------------------------------------------------------------- fixtures

/// A partition with the standard layout, ready to run real-mode code, and the
/// one processor it holds.
struct Guest {
    vcpu: Vcpu,
    partition: Partition,
}

impl Guest {
    /// Build one, letting the caller adjust the configuration first.
    fn new(configure: impl FnOnce(&mut PartitionConfig) -> WhpResult<()>) -> WhpResult<Self> {
        let mut config = PartitionConfig::new()?;
        config.processor_count(1)?;
        // The proof machine has no APIC, and the host owns delivery end to
        // end; every experiment here shares that premise.
        config.local_apic(LocalApicMode::None)?;
        configure(&mut config)?;
        let mut partition = config.setup()?;

        partition.map(layout::RAM, HostPages::new(layout::RAM_PAGES)?, GpaPerms::RWX)?;
        partition.map(layout::READ_ONLY, HostPages::new(1)?, GpaPerms::RX)?;
        partition.create_processor(0)?;
        let vcpu = partition.take_vcpu(0)?;
        Ok(Self { vcpu, partition })
    }

    /// Put the processor in real mode at [`layout::CODE`] with `code` loaded
    /// there, and every segment flat at zero.
    fn load(&mut self, code: &[u8]) -> WhpResult<()> {
        let at = layout::CODE as usize;
        let ram = self.partition.bytes_at_mut(layout::RAM).expect("RAM is mapped");
        ram[at..at + code.len()].copy_from_slice(code);

        self.vcpu.write_segments(
            &[Reg::Cs, Reg::Ds, Reg::Es, Reg::Ss],
            &[
                SegmentRegister::real_mode_code(0),
                SegmentRegister::real_mode_data(0),
                SegmentRegister::real_mode_data(0),
                SegmentRegister::real_mode_data(0),
            ],
        )?;
        self.vcpu.write_regs(
            &[Reg::Rip, Reg::Rsp, Reg::Rflags],
            // Bit 1 of RFLAGS reads as one on every x86; interrupts stay off
            // until a guest says otherwise.
            &[layout::CODE, layout::STACK_TOP, 0x0000_0002],
        )
    }

    fn run(&mut self) -> WhpResult<Exit> {
        self.vcpu.run()
    }

    fn peek(&self, offset: u16) -> u8 {
        self.partition.bytes_at(layout::RAM).expect("RAM is mapped")[offset as usize]
    }

    /// Put a byte back, so a marker left by an earlier run cannot be read as
    /// this one's.
    fn poke(&mut self, offset: u16, byte: u8) {
        self.partition.bytes_at_mut(layout::RAM).expect("RAM is mapped")[offset as usize] = byte;
    }
}

// -------------------------------------------------------------- the guests
//
// Hand-assembled 16-bit real-mode code. Each byte string is annotated with the
// instruction it encodes; nothing here is long enough to want an assembler,
// and spelling the bytes out keeps the encoding auditable.

/// Store a byte into the read-and-execute page, then halt.
const WRITE_READ_ONLY: &[u8] = &[
    0xB8, 0x00, 0x10, // mov ax, 0x1000        (segment for GPA 0x10000)
    0x8E, 0xD8, //       mov ds, ax
    0xC6, 0x06, 0x00, 0x00, 0x5A, // mov byte [0x0000], 0x5A
    0xF4, //             hlt
];

/// Enable interrupts, halt, and — once something wakes it — leave a mark.
const HALT_THEN_MARK: &[u8] = &[
    0xFB, //                             sti
    0xF4, //                             hlt
    0xC6, 0x06, 0x00, 0x07, 0x11, //     mov byte [RESUMED], 0x11
    0xF4, //                             hlt
];

/// The interrupt handler: leave a mark and return.
const HANDLER: &[u8] = &[
    0xC6, 0x06, 0x00, 0x08, 0xA5, // mov byte [MARKER], 0xA5
    0xCF, //                         iret
];

/// Write a port forever; every `out` is an exit.
const PORT_LOOP: &[u8] = &[
    0xBA, 0x80, 0x00, // mov dx, 0x80
    0xEE, //             out dx, al      <- 0x1003
    0xEB, 0xFD, //       jmp -3          (back to the out)
];

/// Store to unmapped guest-physical memory forever; every store is an exit.
const MMIO_LOOP: &[u8] = &[
    0xB8, 0x00, 0x20, // mov ax, 0x2000        (segment for GPA 0x20000)
    0x8E, 0xC0, //       mov es, ax
    0xB0, 0x5A, //       mov al, 0x5A
    0x26, 0xA2, 0x00, 0x00, // mov es:[0x0000], al   <- 0x1007
    0xEB, 0xFA, //       jmp -6          (back to the store)
];

/// Halt forever; every halt is an exit.
const HALT_LOOP: &[u8] = &[
    0xF4, //       hlt
    0xEB, 0xFD, // jmp -3
];

/// Spin forever without ever exiting, so only the host can stop it.
const SPIN: &[u8] = &[
    0xEB, 0xFE, // jmp -2
];

/// Ask for one CPUID leaf and halt. The `0x66` prefix makes the `mov`
/// 32-bit, which is legal in real mode.
fn cpuid_blob(leaf: u32) -> Vec<u8> {
    let mut code = vec![0x66, 0xB8];
    code.extend_from_slice(&leaf.to_le_bytes()); // mov eax, leaf
    code.extend_from_slice(&[0x0F, 0xA2]); //       cpuid
    code.push(0xF4); //                             hlt
    code
}

/// Halt with interrupts masked, so nothing but the platform can end the run.
const CLI_THEN_HALT: &[u8] = &[
    0xFA, // cli
    0xF4, // hlt
];

/// Spin with interrupts enabled, so a vector delivered mid-run is taken
/// without the guest ever halting. The route to an accepted interrupt that
/// does not depend on whether a halted processor can be woken.
const SPIN_WITH_INTERRUPTS: &[u8] = &[
    0xFB, //       sti
    0xEB, 0xFE, // jmp -2
];

/// An interrupt handler that leaves its mark and stops the guest INSIDE
/// itself: no end-of-interrupt, no `iret`. That is the one moment at which an
/// accepted vector is in service and no longer requested, which is what makes
/// the two bitmaps tell themselves apart.
const ACCEPT_AND_EXIT: &[u8] = &[
    0xC6, 0x06, 0x00, 0x08, 0xA5, // mov byte [MARKER], 0xA5
    0xE6, 0xE9, //                   out 0xE9, al
    0xEB, 0xFE, //                   jmp -2
];

/// A fault handler: leave the mark and exit through a port, so an instruction
/// the guest may not be permitted to execute is distinguishable from one that
/// ran.
const FAULT_MARK: &[u8] = &[
    0xC6, 0x06, 0x00, 0x08, 0xA5, // mov byte [MARKER], 0xA5
    0xE6, 0xE9, //                   out 0xE9, al
    0xEB, 0xFE, //                   jmp -2
];

/// Write `lvt0` to the xAPIC's LINT0 register through the memory-mapped page,
/// then enable interrupts and halt; the store past the halt says the guest got
/// its processor back.
///
/// The store is `mov [ds:0x0350], eax`, and it reaches the APIC page because
/// `DS` is given a base a real-mode selector cannot express — which the
/// platform accepts, since it writes the segment cache directly.
fn lint0_blob(lvt0: u32) -> Vec<u8> {
    let mut code = vec![0x66, 0xB8];
    code.extend_from_slice(&lvt0.to_le_bytes()); //           mov eax, lvt0   (0..6)
    code.extend_from_slice(&[0x66, 0xA3, 0x50, 0x03]); //     mov [0x0350], eax (6..10)
    code.extend_from_slice(&[0xFB, 0xF4]); //                 sti ; hlt        (10, 11)
    code.extend_from_slice(&[0xC6, 0x06, 0x00, 0x07, 0x11]); // mov byte [RESUMED], 0x11
    code.push(0xF4); //                                       hlt
    code
}

/// Ask for one CPUID leaf with `ECX` cleared and leave through a port write,
/// which exits whatever the partition's APIC mode is — unlike a halt.
fn hv_cpuid_blob(leaf: u32) -> Vec<u8> {
    let mut code = vec![0x66, 0xB9, 0x00, 0x00, 0x00, 0x00, 0x66, 0xB8];
    code.extend_from_slice(&leaf.to_le_bytes()); // mov ecx, 0 ; mov eax, leaf
    code.extend_from_slice(&[0x0F, 0xA2]); //       cpuid
    code.extend_from_slice(&[0xE6, 0xE9]); //       out 0xE9, al
    code.push(0xF4); //                             hlt
    code
}

/// Read one model-specific register and leave through a port write.
fn rdmsr_blob(msr: u32) -> Vec<u8> {
    let mut code = vec![0x66, 0xB9];
    code.extend_from_slice(&msr.to_le_bytes()); // mov ecx, msr
    code.extend_from_slice(&[0x0F, 0x32]); //      rdmsr
    code.extend_from_slice(&[0xE6, 0xE9]); //      out 0xE9, al
    code.push(0xF4); //                            hlt
    code
}

// ------------------------------------------------------------ experiments

fn q1_read_only_window(host_offers_fault_exit: bool) -> WhpResult<Finding> {
    /// Run the write and describe what came back.
    fn attempt(fault_exit: bool) -> WhpResult<String> {
        let mut guest = Guest::new(|config| {
            if fault_exit {
                config.extended_vm_exits(rusty_box_whp::ExtendedVmExits {
                    gpa_access_fault: true,
                    ..Default::default()
                })?;
            }
            Ok(())
        })?;
        guest.load(WRITE_READ_ONLY)?;
        let exit = guest.run()?;
        Ok(match exit.reason {
            ExitReason::MemoryAccess(access) => format!(
                "MemoryAccess gpa={:#x} access={:?} gpa_unmapped={} \
                 instruction_length={} rip={:#x}",
                access.gpa,
                access.access,
                access.gpa_unmapped,
                exit.vp.instruction_length,
                exit.vp.rip
            ),
            other => format!("{other:?} at rip={:#x}", exit.vp.rip),
        })
    }

    let without = attempt(false)?;
    let with = if host_offers_fault_exit {
        attempt(true)?
    } else {
        "host does not offer GpaAccessFaultExit".to_owned()
    };

    // The second run's first exit tells us what the bit costs. If it faults at
    // the code page — which is mapped read, write AND execute — then the bit
    // reports second-level faults the hypervisor would otherwise resolve by
    // itself, and it is not a switch a fast path can afford.
    let bit_faults_on_permitted_access = with.contains(&format!("gpa={:#x}", layout::CODE));

    let exits_without = without.starts_with("MemoryAccess");
    let answer = if exits_without && bit_faults_on_permitted_access {
        "YES, and the bit must stay OFF — a write to a mapped read-only \
         window exits on its own, while setting GpaAccessFaultExit also \
         faults accesses the mapping permits."
            .to_owned()
    } else if exits_without {
        "YES, and GpaAccessFaultExit is NOT needed — a write to a mapped \
         read-only window exits on its own."
            .to_owned()
    } else {
        "NO — a plain read-only mapping did not produce a memory-access exit; \
         the design must not rely on it."
            .to_owned()
    };
    let detail = format!(
        "without the bit: {without}\n\
         with the bit:    {with}\n\
         (the code page at {:#x} is mapped read+write+execute, so a fault \
         there is one the mapping permitted)",
        layout::CODE
    );
    Ok(Finding { question: Q1, answer, detail })
}

fn q2_instruction_bytes_and_translation() -> WhpResult<Finding> {
    /// What one exit tells a host that owns no instruction decoder.
    struct Told {
        kind: &'static str,
        /// Where the faulting instruction begins, known because this probe
        /// assembled the guest.
        faulting_at: u64,
        exit: Exit,
        /// Filled for memory exits only.
        instruction_bytes: Option<(u8, String)>,
    }

    fn observe(kind: &'static str, code: &[u8], faulting_at: u64) -> WhpResult<Told> {
        let mut guest = Guest::new(|_| Ok(()))?;
        guest.load(code)?;
        let exit = guest.run()?;
        let instruction_bytes = match exit.reason {
            ExitReason::MemoryAccess(access) => {
                let count = access.instruction_byte_count;
                let bytes: Vec<String> = access.instruction_bytes[..usize::from(count)]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                Some((count, bytes.join(" ")))
            }
            _ => None,
        };
        Ok(Told { kind, faulting_at, exit, instruction_bytes })
    }

    // The three trapped accesses a PC actually produces, plus a halt, each at
    // a known address so "did the platform already advance RIP?" is decidable.
    let observations = [
        // `mov byte [0], 0x5A` begins five bytes into the blob.
        observe("write to a read-only window", WRITE_READ_ONLY, layout::CODE + 5)?,
        // `mov es:[0], al` begins seven bytes into the blob.
        observe("write to an unmapped range", MMIO_LOOP, layout::CODE + 7)?,
        // `out dx, al` begins three bytes in.
        observe("port write", PORT_LOOP, layout::CODE + 3)?,
        observe("halt", HALT_LOOP, layout::CODE)?,
    ];

    let mut detail = String::new();
    let mut decoder_needed = Vec::new();
    for told in &observations {
        let advanced = told.exit.vp.rip != told.faulting_at;
        let can_resume = advanced || told.exit.vp.instruction_length != 0;
        if !can_resume {
            decoder_needed.push(told.kind);
        }
        let _ = writeln!(
            detail,
            "{:<28} rip={:#06x} (instruction at {:#06x}, platform advanced it: {advanced}), \
             instruction_length={}{}",
            told.kind,
            told.exit.vp.rip,
            told.faulting_at,
            told.exit.vp.instruction_length,
            match &told.instruction_bytes {
                Some((count, bytes)) if *count > 0 => format!(", bytes[{count}]=[{bytes}]"),
                Some(_) => ", no instruction bytes".to_owned(),
                None => String::new(),
            }
        );
    }

    // In real mode a linear address is the segment base plus the offset, so
    // the address the guest computed and the physical address coincide; this
    // is a clean test of whether the call works at all with paging off.
    let mut guest = Guest::new(|_| Ok(()))?;
    guest.load(WRITE_READ_ONLY)?;
    let translation = guest.vcpu.translate_gva(layout::READ_ONLY)?;
    let _ = writeln!(
        detail,
        "WHvTranslateGva({:#x}) with paging off -> result_code={} gpa={:#x}",
        layout::READ_ONLY,
        translation.result_code,
        translation.gpa
    );

    // Whether the host is handed the bytes matters separately from whether it
    // can resume: with them a decoder needs no guest-memory fetch, without
    // them it must read the instruction out of guest memory first.
    let starved: Vec<&str> = observations
        .iter()
        .filter(|told| matches!(told.instruction_bytes, Some((0, _))))
        .map(|told| told.kind)
        .collect();

    let answer = if decoder_needed.is_empty() {
        "YES for every exit kind — each either advances RIP itself or reports \
         an instruction length."
            .to_owned()
    } else {
        let mut answer = format!(
            "NO for {} — RIP still points at the faulting instruction and the \
             exit reports no length, so a host MUST decode it to make \
             progress.",
            decoder_needed.join(" and ")
        );
        if !starved.is_empty() {
            let _ = write!(
                answer,
                " And for {}, not even the instruction bytes come with the \
                 exit, so the decoder must fetch them from guest memory too.",
                starved.join(" and ")
            );
        }
        answer
    };
    Ok(Finding { question: Q2, answer, detail })
}

fn q3_halt_resume() -> WhpResult<Finding> {
    let mut guest = Guest::new(|_| Ok(()))?;
    guest.load(HALT_THEN_MARK)?;
    install_handler(&mut guest, 0x40)?;

    let halted = guest.run()?;
    if !matches!(halted.reason, ExitReason::Halt) {
        return Ok(Finding {
            question: Q3,
            answer: "INCONCLUSIVE — the guest did not reach its halt".into(),
            detail: format!("{:?} at rip={:#x}", halted.reason, halted.vp.rip),
        });
    }
    let activity_at_halt = guest.vcpu.internal_activity()?;

    // The question exactly: inject and nothing else.
    guest.vcpu.inject(PendingInterruption {
        kind: InterruptionType::Interrupt,
        vector: 0x40,
        error_code: None,
    })?;
    let after_injection = guest.run()?;
    let handler_ran = guest.peek(layout::MARKER) == 0xA5;
    let resumed = guest.peek(layout::RESUMED) == 0x11;

    let mut detail = String::new();
    let _ = writeln!(
        detail,
        "at the halt: rip={:#x} instruction_length={} activity={activity_at_halt:?}",
        halted.vp.rip, halted.vp.instruction_length
    );
    let _ = writeln!(
        detail,
        "after injecting vector 0x40: {:?} at rip={:#x}; handler ran={handler_ran} \
         resumed past the halt={resumed}",
        after_injection.reason, after_injection.vp.rip
    );

    let answer = if handler_ran {
        "PendingInterruption ALONE resumes the processor — HaltSuspend does \
         not have to be cleared."
            .to_owned()
    } else {
        // Fall back to the other half of the question rather than leaving it
        // unanswered: clear the halt suspend and try once more.
        let mut cleared = activity_at_halt;
        cleared.halt_suspend = false;
        guest.vcpu.set_internal_activity(cleared)?;
        let retried = guest.run()?;
        let handler_ran_now = guest.peek(layout::MARKER) == 0xA5;
        let _ = writeln!(
            detail,
            "after clearing HaltSuspend: {:?} at rip={:#x}; handler ran={handler_ran_now}",
            retried.reason, retried.vp.rip
        );
        if handler_ran_now {
            "HaltSuspend MUST be cleared — injection alone left the processor \
             parked."
                .to_owned()
        } else {
            "NEITHER worked — injection under LocalApicEmulationMode::None \
             does not wake this host's halted processor."
                .to_owned()
        }
    };
    Ok(Finding { question: Q3, answer, detail })
}

/// Which exit a timing loop is built to produce, so a loop that meets any
/// other has measured nothing and says so rather than reporting a number that
/// means something else.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    Memory,
    Port,
    Halt,
}

impl Expect {
    fn matches(self, reason: &ExitReason) -> bool {
        matches!(
            (self, reason),
            (Self::Memory, ExitReason::MemoryAccess(_))
                | (Self::Port, ExitReason::IoPortAccess(_))
                | (Self::Halt, ExitReason::Halt)
        )
    }
}

/// Time `rounds` exits of one kind.
///
/// Deliberately writes no registers between runs. The platform advances RIP
/// past a halt by itself and does not advance it for a trapped access, so each
/// re-entry produces exactly one more exit either way — which makes this the
/// cost of an exit and a re-entry and nothing else. It is also the only shape
/// that works: this host refuses `InternalActivityState` writes outright.
fn time_exits(
    code: &[u8],
    expect: Expect,
    rounds: u32,
    separate_security_domain: Option<bool>,
) -> WhpResult<Duration> {
    let mut guest = Guest::new(|config| {
        if let Some(separate) = separate_security_domain {
            config.separate_security_domain(separate)?;
        }
        Ok(())
    })?;
    guest.load(code)?;

    // One exit before the clock starts: it warms the mapping and establishes
    // that the loop faults where it was built to.
    let first = guest.run()?;
    if !expect.matches(&first.reason) {
        panic!("the loop's first exit was {:?} at rip={:#x}", first.reason, first.vp.rip);
    }
    if let ExitReason::MemoryAccess(access) = first.reason {
        if expect == Expect::Memory && access.gpa != layout::MMIO {
            panic!("the MMIO loop faulted at {:#x}, not {:#x}", access.gpa, layout::MMIO);
        }
    }

    let started = Instant::now();
    for _ in 0..rounds {
        let exit = guest.run()?;
        if !expect.matches(&exit.reason) {
            panic!("unexpected exit while timing: {:?} at rip={:#x}", exit.reason, exit.vp.rip);
        }
    }
    Ok(started.elapsed())
}

fn q4_exit_latency() -> WhpResult<Finding> {
    /// Enough exits to swamp the timer's resolution without making the probe
    /// slow; each loop is a fresh partition, so building one is outside the
    /// measurement.
    const ROUNDS: u32 = 20_000;

    let port = time_exits(PORT_LOOP, Expect::Port, ROUNDS, None)?;
    let mmio = time_exits(MMIO_LOOP, Expect::Memory, ROUNDS, None)?;
    let halt = time_exits(HALT_LOOP, Expect::Halt, ROUNDS, None)?;

    let answer = format!(
        "port I/O {:.2} us, MMIO {:.2} us, halt {:.2} us per exit",
        per_exit_us(port, ROUNDS),
        per_exit_us(mmio, ROUNDS),
        per_exit_us(halt, ROUNDS)
    );
    let detail = format!(
        "{ROUNDS} exits each, round-trip out of and back into the guest, with \
         no host register writes in between.\n\
         port I/O: {port:?}\n\
         MMIO:     {mmio:?}\n\
         halt:     {halt:?}"
    );
    Ok(Finding { question: Q4, answer, detail })
}

fn per_exit_us(total: Duration, rounds: u32) -> f64 {
    total.as_secs_f64() * 1e6 / f64::from(rounds)
}

fn q5_mapping_cost(host_offers_partial_unmap: bool) -> WhpResult<Finding> {
    /// A shadowed-BIOS flip is a handful of these at boot; time enough of them
    /// to see the per-call cost clearly.
    const FLIPS: u32 = 2_000;
    /// Two mebibytes, the size of a chipset's shadowable region.
    const BIG_PAGES: usize = 512;

    let mut guest = Guest::new(|_| Ok(()))?;

    let big = HostPages::new(BIG_PAGES)?;
    let started = Instant::now();
    guest.partition.map(0x0040_0000, big, GpaPerms::RWX)?;
    let map_big = started.elapsed();

    let started = Instant::now();
    for round in 0..FLIPS {
        let perms = if round % 2 == 0 { GpaPerms::RX } else { GpaPerms::RWX };
        guest.partition.remap(layout::READ_ONLY, perms)?;
    }
    let flips = started.elapsed();

    let partial = match guest.partition.unmap_subrange(0x0040_0000 + PAGE_SIZE as u64, PAGE_SIZE as u64)
    {
        Ok(()) => "accepted".to_owned(),
        Err(err) => format!("refused ({err})"),
    };

    let answer = format!(
        "a {} KiB map takes {:.1} us; a permission flip takes {:.2} us; a \
         partial unmap was {}",
        BIG_PAGES * PAGE_SIZE / 1024,
        map_big.as_secs_f64() * 1e6,
        per_exit_us(flips, FLIPS),
        partial.split_whitespace().next().unwrap_or("?")
    );
    let detail = format!(
        "WHvMapGpaRange of {BIG_PAGES} pages: {map_big:?}\n\
         {FLIPS} permission flips of one page: {flips:?}\n\
         capability says partial unmap is {host_offers_partial_unmap}; \
         attempting one: {partial}"
    );
    Ok(Finding { question: Q5, answer, detail })
}

fn q6_separate_security_domain() -> WhpResult<Finding> {
    const ROUNDS: u32 = 20_000;

    let timed =
        |separate: bool| time_exits(MMIO_LOOP, Expect::Memory, ROUNDS, Some(separate));

    // Interleaved, and each direction run twice, because a single ordering on
    // a busy desktop measures the desktop.
    let off_first = timed(false)?;
    let on_first = timed(true)?;
    let off_second = timed(false)?;
    let on_second = timed(true)?;
    let off = off_first.min(off_second);
    let on = on_first.min(on_second);

    let ratio = off.as_secs_f64() / on.as_secs_f64();
    let answer = format!(
        "MMIO exits cost {:.2} us with a separate domain and {:.2} us without \
         ({:+.1}%)",
        per_exit_us(on, ROUNDS),
        per_exit_us(off, ROUNDS),
        (ratio - 1.0) * 100.0
    );
    let detail = format!(
        "best of two runs each, interleaved.\n\
         separate=true:  {on_first:?} then {on_second:?}\n\
         separate=false: {off_first:?} then {off_second:?}"
    );
    Ok(Finding { question: Q6, answer, detail })
}

fn q7_cpuid_and_late_properties(host_offers_cpuid_exit: bool) -> WhpResult<Finding> {
    if !host_offers_cpuid_exit {
        return Ok(Finding {
            question: Q7,
            answer: "NO — this host does not offer CPUID exits at all".into(),
            detail: String::new(),
        });
    }

    /// The leaf whose ECX bit 31 is the hypervisor-present bit every guest
    /// checks first, and the base of the hypervisor vendor range.
    const LEAVES: [u32; 2] = [1, 0x4000_0000];
    /// ECX bit 31 of leaf 1.
    const HYPERVISOR_PRESENT: u64 = 1 << 31;

    let mut guest = Guest::new(|config| {
        config.extended_vm_exits(rusty_box_whp::ExtendedVmExits {
            cpuid: true,
            ..Default::default()
        })?;
        config.cpuid_exit_list(&LEAVES)?;
        Ok(())
    })?;

    let mut detail = String::new();
    // Control has to be shown in BOTH directions. This host's own answer for
    // leaf 1 already has the hypervisor-present bit clear, so clearing it
    // proves nothing on its own; setting it and seeing the guest observe the
    // change is what shows the host is the author of the answer.
    let mut set_landed = false;
    let mut cleared_landed = false;

    for leaf in LEAVES {
        for wanted_present in [true, false] {
            guest.load(&cpuid_blob(leaf))?;
            let exit = guest.run()?;
            let ExitReason::Cpuid(cpuid) = exit.reason else {
                let _ = writeln!(detail, "leaf {leaf:#x}: did NOT exit ({:?})", exit.reason);
                break;
            };
            if wanted_present {
                let _ = writeln!(
                    detail,
                    "leaf {leaf:#x}: EXITED at rip={:#x} (instruction_length={}); the host's \
                     own answer would have been eax={:#010x} ebx={:#010x} ecx={:#010x} \
                     edx={:#010x}",
                    exit.vp.rip,
                    exit.vp.instruction_length,
                    cpuid.default_rax,
                    cpuid.default_rbx,
                    cpuid.default_rcx,
                    cpuid.default_rdx
                );
            }

            let answered_rcx = if wanted_present {
                cpuid.default_rcx | HYPERVISOR_PRESENT
            } else {
                cpuid.default_rcx & !HYPERVISOR_PRESENT
            };
            guest.vcpu.write_regs(
                &[Reg::Rax, Reg::Rbx, Reg::Rcx, Reg::Rdx, Reg::Rip],
                &[
                    cpuid.default_rax,
                    cpuid.default_rbx,
                    answered_rcx,
                    cpuid.default_rdx,
                    exit.rip_after_instruction(),
                ],
            )?;
            let finished = guest.run()?;
            let mut observed = [0u64; 2];
            guest.vcpu.read_regs(&[Reg::Rax, Reg::Rcx], &mut observed)?;
            let observed_present = observed[1] & HYPERVISOR_PRESENT != 0;
            let _ = writeln!(
                detail,
                "           answered with the hypervisor-present bit {}: guest read \
                 eax={:#010x} ecx={:#010x} (bit reads {}), then {:?}",
                u8::from(wanted_present),
                observed[0],
                observed[1],
                u8::from(observed_present),
                finished.reason
            );
            if leaf == 1 && observed_present == wanted_present {
                if wanted_present {
                    set_landed = true;
                } else {
                    cleared_landed = true;
                }
            }
        }
    }

    // The other half of the question: are processor properties really
    // pre-setup-only? The platform is the authority, so ask it.
    let late = match guest.partition.set_property_late(LateProperty::ProcessorCount, 1) {
        Ok(()) => "ACCEPTED after setup".to_owned(),
        Err(err) => format!("refused after setup ({err})"),
    };
    let late_exits = match guest
        .partition
        .set_property_late(LateProperty::ExtendedVmExits, 0)
    {
        Ok(()) => "ACCEPTED after setup".to_owned(),
        Err(err) => format!("refused after setup ({err})"),
    };
    let _ = writeln!(detail, "ProcessorCount: {late}");
    let _ = writeln!(detail, "ExtendedVmExits: {late_exits}");

    let answer = if set_landed && cleared_landed {
        "YES — the host authors CPUID leaf 1 outright: it can set the \
         hypervisor-present bit and clear it again, which is the stealth \
         lever. Properties, however, are NOT uniformly pre-setup-only."
            .to_owned()
    } else if cleared_landed {
        "PARTIAL — the bit reads clear as asked, but setting it did not land, \
         so the host is not demonstrably the author of the answer."
            .to_owned()
    } else {
        "NO — leaf 1 exits, but the host-supplied ECX did not reach the guest."
            .to_owned()
    };
    Ok(Finding { question: Q7, answer, detail })
}

fn q8_cancel_stickiness() -> WhpResult<Finding> {
    /// How long to let a spinning guest run before deciding the earlier
    /// cancel did not stick.
    const RESCUE_AFTER: Duration = Duration::from_millis(250);

    let mut guest = Guest::new(|_| Ok(()))?;
    guest.load(SPIN)?;

    // Cancel while the processor is definitely NOT running.
    guest.vcpu.cancel_run()?;

    let canceller = guest.vcpu.canceller();
    let (exit, elapsed) = std::thread::scope(|scope| {
        // The rescue exists so that a cancel which does not stick still ends
        // the probe: without it a guest spinning on `jmp $` never returns.
        let rescue = scope.spawn(move || {
            std::thread::sleep(RESCUE_AFTER);
            canceller.cancel()
        });
        let started = Instant::now();
        let exit = guest.run();
        let elapsed = started.elapsed();
        // The rescue's own result matters: if it failed, a fast return was
        // not necessarily the earlier cancel's doing.
        let rescued = rescue.join().expect("the rescue thread did not panic");
        (exit.map(|exit| (exit, rescued)), elapsed)
    });
    let (exit, rescued) = exit?;

    let stuck = elapsed < RESCUE_AFTER / 2;
    let answer = if stuck {
        "YES, STICKY — a cancel issued while the processor was stopped ended \
         the next run immediately."
            .to_owned()
    } else {
        "NO — the earlier cancel was forgotten; the run only ended when a \
         second cancel arrived mid-run."
            .to_owned()
    };
    let detail = format!(
        "run returned {:?} after {elapsed:?} (the rescue fires at {RESCUE_AFTER:?})\n\
         rescue cancel result: {}",
        exit.reason,
        match rescued {
            Ok(()) => "accepted".to_owned(),
            Err(err) => format!("refused ({err})"),
        }
    );
    Ok(Finding { question: Q8, answer, detail })
}

/// The two routes a host has for getting a vector into a guest.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// `WHvRegisterPendingInterruption` — straight into the processor, past
    /// any APIC the hypervisor is emulating.
    Inject,
    /// `WHvRequestInterrupt` — handed to the emulated APIC to arbitrate.
    ApicRequest,
}

/// One run that was guaranteed to return.
struct Rescued {
    exit: Exit,
    /// True when the processor did not leave on its own and a cancel from
    /// another thread had to bring it out.
    rescued: bool,
}

/// Run the processor with a guarantee that this returns.
///
/// A halted processor whose hypervisor owns the APIC may never come back out
/// of `WHvRunVirtualProcessor` at all, so every run in this experiment needs a
/// way out — including the run that is merely *expected* to halt, because
/// whether it halts is part of what is being measured.
///
/// The rescue is told when the run ended rather than always sleeping out its
/// timeout, and that matters: this platform latches a cancel (see Q8), so a
/// rescue firing after a run had already finished would abort the *next* one
/// and make every later result read as "never woke".
fn run_with_rescue(guest: &mut Guest, after: Duration) -> WhpResult<Rescued> {
    let canceller = guest.vcpu.canceller();
    let (finished, wait_for_finish) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        let rescue = scope.spawn(move || match wait_for_finish.recv_timeout(after) {
            Ok(()) => Ok(false),
            Err(_) => canceller.cancel().map(|()| true),
        });
        let exit = guest.run();
        match finished.send(()) {
            // An error here means the rescue had already timed out and
            // cancelled; the exit reason shows that, so there is nothing to
            // report separately.
            Ok(()) | Err(_) => {}
        }
        let rescued = rescue.join().expect("the rescue thread did not panic")?;
        Ok(Rescued { exit: exit?, rescued })
    })
}

/// What one wake attempt did.
struct Woke {
    /// Whether the partition would take the APIC mode at all.
    configured: Result<(), String>,
    /// Whether the delivery call itself was accepted.
    delivered: Result<(), String>,
    /// `None` when the processor never woke and the rescue had to end the run.
    exit: Option<ExitReason>,
    handler_ran: bool,
}

/// Try to wake a halted real-mode guest with vector 0x40 and report what
/// happened, never blocking: a processor that does not wake would otherwise
/// leave `WHvRunVirtualProcessor` inside the hypervisor forever.
fn wake_attempt(apic: LocalApicMode, how: Wake) -> WhpResult<Woke> {
    /// Long enough that a delivery which works has certainly happened, short
    /// enough that a probe of ten experiments stays quick.
    const RESCUE_AFTER: Duration = Duration::from_millis(300);
    /// The vector the guest has an interrupt handler for.
    const VECTOR: u16 = 0x40;

    let mut configured = Ok(());
    let mut guest = Guest::new(|config| {
        if let Err(err) = config.local_apic(apic) {
            configured = Err(err.to_string());
        }
        Ok(())
    })?;
    if let Err(why) = configured {
        return Ok(Woke {
            configured: Err(why),
            delivered: Err("not attempted".into()),
            exit: None,
            handler_ran: false,
        });
    }

    guest.load(HALT_THEN_MARK)?;
    install_handler(&mut guest, VECTOR)?;

    let halted = run_with_rescue(&mut guest, RESCUE_AFTER)?;
    if !matches!(halted.exit.reason, ExitReason::Halt) {
        let why = if halted.rescued {
            "the run NEVER RETURNED on its own — with this APIC mode the \
             hypervisor parks a halted processor instead of exiting"
                .to_owned()
        } else {
            format!("the guest never halted: {:?}", halted.exit.reason)
        };
        return Ok(Woke {
            configured: Ok(()),
            delivered: Err(why),
            exit: Some(halted.exit.reason),
            handler_ran: false,
        });
    }

    let delivered = match how {
        Wake::Inject => guest.vcpu.inject(PendingInterruption {
            kind: InterruptionType::Interrupt,
            vector: VECTOR,
            error_code: None,
        }),
        Wake::ApicRequest => guest.partition.request_interrupt(InterruptRequest {
            kind: InterruptKind::Fixed,
            destination_mode: DestinationMode::Physical,
            trigger_mode: TriggerMode::Edge,
            // The boot processor's APIC ID.
            destination: 0,
            vector: u32::from(VECTOR),
        }),
    };
    if let Err(err) = delivered {
        return Ok(Woke {
            configured: Ok(()),
            delivered: Err(err.to_string()),
            exit: None,
            handler_ran: false,
        });
    }

    let after_delivery = run_with_rescue(&mut guest, RESCUE_AFTER)?;
    Ok(Woke {
        configured: Ok(()),
        delivered: Ok(()),
        exit: (!after_delivery.rescued).then_some(after_delivery.exit.reason),
        handler_ran: guest.peek(layout::MARKER) == 0xA5,
    })
}

/// Put the real-mode handler and its interrupt-vector entry in low memory.
fn install_handler(guest: &mut Guest, vector: u16) -> WhpResult<()> {
    install_vector(guest, vector, layout::HANDLER, HANDLER)
}

/// Put `code` at `at` and point real-mode interrupt vector `vector` at it.
///
/// The general form of [`install_handler`], for the experiments that need a
/// second handler — a fault handler beside an interrupt handler, or a handler
/// that stops the guest rather than returning from it.
fn install_vector(guest: &mut Guest, vector: u16, at: u64, code: &[u8]) -> WhpResult<()> {
    let ram = guest.partition.bytes_at_mut(layout::RAM).expect("RAM is mapped");
    let handler = at as usize;
    ram[handler..handler + code.len()].copy_from_slice(code);
    // A real-mode vector is four bytes: offset then segment.
    let entry = 4 * usize::from(vector);
    ram[entry..entry + 2].copy_from_slice(&(at as u16).to_le_bytes());
    ram[entry + 2..entry + 4].copy_from_slice(&0u16.to_le_bytes());
    Ok(())
}

fn q10_wake_under_an_emulated_apic(host_emulates_apic: bool) -> WhpResult<Finding> {
    if !host_emulates_apic {
        return Ok(Finding {
            question: Q10,
            answer: "N/A — this host does not offer local APIC emulation".into(),
            detail: String::new(),
        });
    }

    let mut detail = String::new();

    // First: does a halted processor come back out at all? Under `None` it
    // does (Q3), and that is what makes stop-then-inject possible there.
    let stops = wake_attempt(LocalApicMode::XApic, Wake::Inject)?;
    let parks = matches!(&stops.delivered, Err(why) if why.contains("NEVER RETURNED"));
    let _ = writeln!(
        detail,
        "XApic, stop-then-inject: {}",
        match (&stops.configured, &stops.delivered, &stops.exit) {
            (Err(why), _, _) => format!("the mode itself was refused ({why})"),
            (_, Err(why), _) => why.clone(),
            (_, _, None) => "never woke".to_owned(),
            (_, _, Some(reason)) => format!("woke to {reason:?}"),
        }
    );
    if parks {
        let _ = writeln!(
            detail,
            "  => `WHvRegisterPendingInterruption` cannot be applied to a run in\n  \
             \x20  progress: it is a register write, which needs a stopped processor,\n  \
             \x20  and this halted processor did not leave the run on its own. A\n  \
             \x20  partition-level call delivers without stopping it; a cancel stops\n  \
             \x20  it (P4), and whether the writes are then accepted is measured only\n  \
             \x20  under X2Apic (P1)."
        );
    }

    // So the only route left is a partition-level delivery from another thread
    // while the run is in progress. That is the whole question for a machine
    // whose 8259 lives in host userspace.
    //
    // Two deliveries, because one alone cannot be read. A `Fixed` vector is
    // what an 8259's output becomes, but this guest is in real mode and has
    // never enabled its own APIC, so the APIC is entitled to drop it — a
    // failure there would say nothing about the platform. An `Nmi` is not
    // gated by the APIC's software-enable, so it tests the mechanism itself.
    // Whether the handler ran is the measurement; whether the run returned is
    // not, because this guest halts a second time after its handler returns
    // and a second halt parks the processor exactly like the first. Reporting
    // only the run's fate would read as a failure when the delivery worked.
    let describe = |woke: &Woke| {
        let handler = if woke.handler_ran { "the handler RAN" } else { "the handler did NOT run" };
        match (&woke.delivered, &woke.exit) {
            (Err(why), _) => format!("delivery refused: {why}"),
            (_, None) => format!("{handler}; the run then parked again and was rescued"),
            (_, Some(reason)) => format!("{handler}; the run returned {reason:?}"),
        }
    };
    let during = wake_during_run(LocalApicMode::XApic, InterruptKind::Fixed, 0x40, 0x40)?;
    let _ = writeln!(detail, "XApic, Fixed vector 0x40 mid-run:  {}", describe(&during));
    // A real-mode NMI lands on interrupt vector 2, but the request itself
    // carries no vector — an NMI has none, and sending one is rejected as an
    // invalid argument.
    let nmi = wake_during_run(LocalApicMode::XApic, InterruptKind::Nmi, 2, 0)?;
    let _ = writeln!(detail, "XApic, NMI mid-run:                {}", describe(&nmi));
    if nmi.handler_ran && !during.handler_ran {
        let _ = writeln!(
            detail,
            "  => the mechanism WORKS; the Fixed vector was dropped by the guest's own\n  \
             \x20  APIC, which a real-mode guest never enabled. A kernel that enables it\n  \
             \x20  would receive the vector."
        );
    }

    // The control: the same partition-level call with no APIC to accept it.
    // Without this, a refusal above would not be evidence of anything.
    let control = wake_attempt(LocalApicMode::None, Wake::ApicRequest)?;
    let _ = writeln!(
        detail,
        "None + WHvRequestInterrupt (control): {}",
        match &control.delivered {
            Err(why) => format!("refused, as it should be — {why}"),
            Ok(()) => "ACCEPTED, which it should not be with no APIC present".to_owned(),
        }
    );

    let answer = if !parks {
        "INCONCLUSIVE — see the detail; the guest did not reach its halt."
            .to_owned()
    } else if during.handler_ran {
        "YES, but only from another thread. A halted processor does not leave \
         `WHvRunVirtualProcessor` under XApic on its own, so a register write \
         cannot reach it until something stops the run first; \
         `WHvRequestInterrupt` issued while the run is in progress wakes it \
         and the handler runs, with nothing stopped. A userspace 8259 can \
         deliver across threads rather than between runs."
            .to_owned()
    } else if nmi.handler_ran {
        "YES, from another thread, and the delivery MECHANISM is proven: a \
         mid-run NMI wakes the parked processor and its handler runs. The \
         Fixed vector was dropped by the guest's own APIC, which a real-mode \
         guest never enables — not by the platform. A halted processor does \
         not leave `WHvRunVirtualProcessor` under XApic on its own, so a \
         register write cannot reach it until a cancel stops the run first \
         (P4 measures the cancel, P1 the writes that follow it, both under \
         X2Apic); a userspace 8259 can deliver across threads without one."
            .to_owned()
    } else if nmi.delivered.is_err() {
        "PARTLY ANSWERED. A halted processor does not leave \
         `WHvRunVirtualProcessor` under XApic on its own, so a register write \
         cannot reach it until something stops the run first — that much is \
         settled. Whether a mid-run request can \
         wake it is NOT: the Fixed vector this guest's own disabled APIC was \
         entitled to drop, and the NMI meant to bypass that gate was refused \
         outright. Deciding it needs a guest that enables its LAPIC, which \
         real mode cannot reach."
            .to_owned()
    } else {
        "NO. A halted processor does not leave `WHvRunVirtualProcessor` under \
         XApic on its own, and neither a mid-run Fixed vector NOR an NMI — \
         which no APIC \
         mask can gate — woke it. Stage one of the Alpine plan is not viable \
         on this host; it needs LocalApicEmulationMode::None with our own \
         LAPIC behind the shadow CPU."
            .to_owned()
    };
    Ok(Finding { question: Q10, answer, detail })
}

/// Deliver through the emulated APIC from another thread WHILE the processor
/// runs, which is the only shape available once a halt stops producing an exit.
///
/// `kind` decides what is being tested. A `Fixed` vector is what an 8259's
/// output would become — but it is gated by the guest's own APIC, which a
/// real-mode guest has never enabled, so a `Fixed` delivery that does not
/// arrive proves nothing about the platform. `Nmi` is not gated by the APIC's
/// software-enable at all, so it separates "the platform cannot deliver
/// mid-run" from "this guest's APIC is switched off".
fn wake_during_run(
    apic: LocalApicMode,
    kind: InterruptKind,
    ivt_vector: u16,
    sent_vector: u32,
) -> WhpResult<Woke> {
    /// Long enough for the guest to have reached its `hlt`.
    const DELIVER_AFTER: Duration = Duration::from_millis(50);
    /// Long enough after that for a delivery which works to have landed.
    const RESCUE_AFTER: Duration = Duration::from_millis(400);

    let mut configured = Ok(());
    let mut guest = Guest::new(|config| {
        if let Err(err) = config.local_apic(apic) {
            configured = Err(err.to_string());
        }
        Ok(())
    })?;
    if let Err(why) = configured {
        return Ok(Woke {
            configured: Err(why),
            delivered: Err("not attempted".into()),
            exit: None,
            handler_ran: false,
        });
    }
    guest.load(HALT_THEN_MARK)?;
    install_handler(&mut guest, ivt_vector)?;

    let requester = guest.partition.interrupt_requester();
    let canceller = guest.vcpu.canceller();
    let (finished, wait_for_finish) = std::sync::mpsc::channel::<()>();

    let outcome = std::thread::scope(|scope| {
        let helper = scope.spawn(move || {
            std::thread::sleep(DELIVER_AFTER);
            let delivered = requester.request(InterruptRequest {
                kind,
                destination_mode: DestinationMode::Physical,
                trigger_mode: TriggerMode::Edge,
                destination: 0,
                vector: sent_vector,
            });
            // Whether the run then ends on its own is the measurement; the
            // rescue only guarantees this function returns.
            let rescued = match wait_for_finish.recv_timeout(RESCUE_AFTER) {
                Ok(()) => false,
                Err(_) => {
                    canceller.cancel()?;
                    true
                }
            };
            Ok::<_, rusty_box_whp::WhpError>((delivered, rescued))
        });
        let exit = guest.run();
        match finished.send(()) {
            // As in `run_with_rescue`: an error means the rescue already
            // fired, which the exit reason shows.
            Ok(()) | Err(_) => {}
        }
        let (delivered, rescued) = helper.join().expect("the helper thread did not panic")?;
        Ok::<_, rusty_box_whp::WhpError>((exit?, delivered, rescued))
    });
    let (exit, delivered, rescued) = outcome?;

    Ok(Woke {
        configured: Ok(()),
        delivered: delivered.map_err(|err| err.to_string()),
        exit: (!rescued).then_some(exit.reason),
        handler_ran: guest.peek(layout::MARKER) == 0xA5,
    })
}

fn q9_mapping_churn() -> WhpResult<Finding> {
    /// Far more separate ranges than a PC chipset needs, so the number that
    /// comes back is the platform's limit and not the experiment's. Worth
    /// pushing: Microsoft's own Hyperlight maps guest memory from a surrogate
    /// process, which is the shape a low mapping limit forces.
    const CAP: usize = 8_192;

    /// Time the first and last stretch separately: if mapping cost grows with
    /// the number of ranges already mapped, a design that maps many small
    /// ones pays a price an average would hide.
    const BATCH: usize = 512;

    let mut guest = Guest::new(|_| Ok(()))?;
    let started = Instant::now();
    let mut first_batch = Duration::ZERO;
    let mut mapped = 0usize;
    let mut refusal = None;
    for index in 0..CAP {
        let gpa = layout::CHURN + (index as u64) * PAGE_SIZE as u64;
        let pages = HostPages::new(1)?;
        match guest.partition.map(gpa, pages, GpaPerms::RWX) {
            Ok(()) => mapped += 1,
            Err(err) => {
                refusal = Some(format!("at range {index}: {err}"));
                break;
            }
        }
        if mapped == BATCH {
            first_batch = started.elapsed();
        }
    }
    let elapsed = started.elapsed();
    let last_batch_each = if mapped > BATCH {
        Some((elapsed - first_batch).as_secs_f64() * 1e6 / (mapped - BATCH) as f64)
    } else {
        None
    };

    // Unmapping matters as much as mapping: a chipset flip that could map but
    // not unmap would leak a range per boot.
    let mut unmapped = 0usize;
    let mut unmap_refusal = None;
    for index in 0..mapped {
        let gpa = layout::CHURN + (index as u64) * PAGE_SIZE as u64;
        match guest.partition.unmap(gpa) {
            Ok(_pages) => unmapped += 1,
            Err(err) => {
                unmap_refusal = Some(format!("at range {index}: {err}"));
                break;
            }
        }
    }

    let first_each = first_batch.as_secs_f64() * 1e6 / BATCH.min(mapped).max(1) as f64;
    let answer = match (&refusal, last_batch_each) {
        (None, Some(last_each)) => format!(
            "at least {mapped} separate ranges — no limit reached, but the \
             cost per map grows with the count: {first_each:.1} us for the \
             first {BATCH}, {last_each:.1} us thereafter"
        ),
        (None, None) => format!("at least {mapped} separate ranges, {first_each:.1} us each"),
        (Some(why), _) => format!("{mapped} separate ranges, then refused — {why}"),
    };
    let detail = format!(
        "mapped {mapped} single-page ranges in {elapsed:?} \
         (first {BATCH} in {first_batch:?})\n\
         unmapped {unmapped} of them{}\n\
         {}",
        match &unmap_refusal {
            None => String::new(),
            Some(why) => format!(", then refused — {why}"),
        },
        match &refusal {
            None => format!(
                "the cap of {CAP} was the experiment's, not the host's — no \
                 surrogate process is needed at a PC chipset's scale"
            ),
            Some(why) => format!("first refusal {why}"),
        }
    );
    Ok(Finding { question: Q9, answer, detail })
}

// ------------------------------------------------- the VMM-shape questions
//
// Everything below answers a question the VMM-shape design rests on. Two of
// them decide the shape of later work rather than informing it: P1, whether a
// processor parked in `HLT` can be woken for a legacy interrupt at all, and P3,
// whether a halt is even reported when the hypervisor owns the APIC.

/// How long a run that may never return on its own is given before a cancel
/// from another thread ends it. Long enough that a delivery which works has
/// certainly landed, short enough that a probe of twenty experiments stays
/// quick.
const RESCUE_AFTER: Duration = Duration::from_millis(200);

/// The vector every legacy-delivery experiment here uses. It is what an 8259
/// programmed the way a PC BIOS programs it puts IRQ0 on, so the interrupt
/// under test is the one the machine actually needs.
const IRQ0_VECTOR: u8 = 0x20;

/// One run of an experiment that may park, as the finding's table reports it.
struct HaltedRun {
    label: String,
    reason: ExitReason,
    /// Wall time the run took. Structural in every use here — the two outcomes
    /// differ by three orders of magnitude — so host contention cannot turn one
    /// reading into the other.
    elapsed: Duration,
    /// True when the processor did not leave on its own and the rescue cancel
    /// had to bring it out.
    rescued: bool,
    /// Whether the interrupt handler's marker byte is present.
    handler_ran: bool,
    rip: u64,
    instruction_length: u8,
}

/// [`run_with_rescue`], timed, and read together with the guest's markers.
fn timed_run(guest: &mut Guest, label: &str, after: Duration) -> WhpResult<HaltedRun> {
    let started = Instant::now();
    let finished = run_with_rescue(guest, after)?;
    let elapsed = started.elapsed();
    Ok(HaltedRun {
        label: label.to_owned(),
        reason: finished.exit.reason,
        elapsed,
        rescued: finished.rescued,
        handler_ran: guest.peek(layout::MARKER) == 0xA5,
        rip: finished.exit.vp.rip,
        instruction_length: finished.exit.vp.instruction_length,
    })
}

/// An exit reason short enough for a table cell, with the part of its payload
/// that identifies it kept.
fn reason_name(reason: &ExitReason) -> String {
    match reason {
        ExitReason::MemoryAccess(access) => format!("MemoryAccess({:#x})", access.gpa),
        ExitReason::IoPortAccess(port) => format!("IoPortAccess({:#x})", port.port),
        ExitReason::ApicWriteTrap { register, value } => {
            format!("ApicWriteTrap({register:?}={value:#x})")
        }
        ExitReason::Canceled { reason } => format!("Canceled({reason})"),
        other => format!("{other:?}"),
    }
}

/// The table every parked-run experiment reports: one row per run, so "never
/// woke" is distinguishable from "woke on the wrong run".
///
/// `rip` and `len` carry the weight for a run that ended at the rescue rather
/// than on its own. A rescue tells you only that nothing came back; where the
/// processor was standing when the cancel reached it says whether it had got as
/// far as the instruction under test. `handler` reads the marker byte at the
/// end of each run and the byte persists, so the column means "the marker is
/// present as of this run" — earlier rows reading false are what make a later
/// TRUE attributable to that run.
fn write_runs(detail: &mut String, runs: &[HaltedRun]) {
    let _ = writeln!(
        detail,
        "  {:<42} {:<26} {:>9} {:>8} {:>8} {:>8} {:>4}",
        "run", "exit reason", "elapsed", "rescued", "handler", "rip", "len"
    );
    for run in runs {
        let _ = writeln!(
            detail,
            "  {:<42} {:<26} {:>9} {:>8} {:>8} {:>8} {:>4}",
            run.label,
            reason_name(&run.reason),
            format!("{:.1?}", run.elapsed),
            run.rescued,
            run.handler_ran,
            format!("{:#x}", run.rip),
            run.instruction_length
        );
    }
}

/// How a host call that was made for its answer rather than its effect came
/// back.
fn outcome(result: &Result<(), String>) -> String {
    match result {
        Ok(()) => "ACCEPTED".to_owned(),
        Err(why) => format!("REFUSED ({why})"),
    }
}

/// The interrupt every experiment here delivers through the emulated APIC.
fn fixed_vector(vector: u32, trigger: TriggerMode) -> InterruptRequest {
    InterruptRequest {
        kind: InterruptKind::Fixed,
        destination_mode: DestinationMode::Physical,
        trigger_mode: trigger,
        // The boot processor's APIC ID.
        destination: 0,
        vector,
    }
}

/// One 256-bit bitmap as the raw words the platform wrote, low word first.
fn bitmap(words: [u32; ApicVector::WORDS]) -> String {
    words.iter().map(|word| format!("{word:08x}")).collect::<Vec<_>>().join(" ")
}

/// Every word of the state page the structure claims, so what is still
/// unpinned is recorded as data rather than described in prose.
fn page_words(page: &ApicStatePage) -> String {
    let mut out = String::new();
    for index in 0..ApicStatePage::WORDS {
        if index % 8 == 0 {
            let _ = write!(out, "\n  words {index:>2}..:");
        }
        let at = index * 4;
        let word =
            u32::from_le_bytes([page.0[at], page.0[at + 1], page.0[at + 2], page.0[at + 3]]);
        let _ = write!(out, " {word:08x}");
    }
    out
}

/// The four bytes of a CPUID register read as the ASCII a vendor string is.
fn ascii_of(word: u64) -> String {
    (word as u32)
        .to_le_bytes()
        .iter()
        .map(|byte| if byte.is_ascii_graphic() || *byte == b' ' { char::from(*byte) } else { '.' })
        .collect()
}

/// How contended this host is, as a number rather than an impression.
///
/// Ten five-millisecond sleeps; the mean overshoot beyond the requested time is
/// how late the scheduler handed this thread its processor back. A quiet host
/// overshoots by a fraction of a millisecond. Every duration this probe reports
/// is read beside it.
fn scheduling_delay() -> Duration {
    const SAMPLES: u32 = 10;
    const NAP: Duration = Duration::from_millis(5);
    let mut total = Duration::ZERO;
    for _ in 0..SAMPLES {
        let started = Instant::now();
        std::thread::sleep(NAP);
        total += started.elapsed().saturating_sub(NAP);
    }
    total / SAMPLES
}

fn available_parallelism() -> usize {
    std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get)
}

/// An activity state with nothing suspended — what a host writes to say the
/// processor should be executing again.
const RUNNING: InternalActivity =
    InternalActivity { startup_suspend: false, halt_suspend: false, idle_suspend: false };

/// What one park-then-wake attempt did, run by run.
struct ParkThenWake {
    runs: Vec<HaltedRun>,
    /// The activity state the platform reported once the first run had ended.
    activity_at_park: Result<InternalActivity, String>,
    pending_event_write: Result<(), String>,
    /// `None` when the experiment deliberately did not attempt it — the control
    /// that separates "clearing the suspend woke it" from "the pending event
    /// alone woke it".
    activity_write: Option<Result<(), String>>,
    /// Whether the guest reached the store that follows its first `HLT`.
    resumed_past_the_halt: bool,
}

/// Park a real-mode guest in `HLT`, then try to wake it the way a userspace
/// 8259 would have to: place an ExtINT in the pending-event slot, and —
/// optionally — clear the halt suspend that is keeping the processor parked.
///
/// The two runs are reported separately because "never woke" and "woke on the
/// wrong run" are different platforms to build on.
fn park_then_wake(apic: LocalApicMode, clear_halt_suspend: bool) -> WhpResult<ParkThenWake> {
    let mut guest = Guest::new(|config| {
        config.local_apic(apic)?;
        Ok(())
    })?;
    guest.load(HALT_THEN_MARK)?;
    install_handler(&mut guest, u16::from(IRQ0_VECTOR))?;

    let mut runs = vec![timed_run(&mut guest, "run 1: sti; hlt", RESCUE_AFTER)?];

    let activity_at_park = guest.vcpu.internal_activity().map_err(|err| err.to_string());
    let pending_event_write = guest
        .vcpu
        .write_words128(
            Reg::PendingEvent,
            PendingExtIntEvent { vector: IRQ0_VECTOR }.as_words(),
        )
        .map_err(|err| err.to_string());
    let activity_write = clear_halt_suspend
        .then(|| guest.vcpu.set_internal_activity(RUNNING).map_err(|err| err.to_string()));

    runs.push(timed_run(&mut guest, "run 2: after the host's writes", RESCUE_AFTER)?);

    Ok(ParkThenWake {
        runs,
        activity_at_park,
        pending_event_write,
        activity_write,
        resumed_past_the_halt: guest.peek(layout::RESUMED) == 0x11,
    })
}

fn q11_halt_suspend_clear_under_x2apic() -> WhpResult<Finding> {
    fn record(detail: &mut String, title: &str, attempt: &ParkThenWake) {
        let _ = writeln!(detail, "{title}");
        write_runs(detail, &attempt.runs);
        let _ = writeln!(
            detail,
            "  activity once run 1 ended: {}",
            match &attempt.activity_at_park {
                Ok(activity) => format!("{activity:?}"),
                Err(why) => format!("UNREADABLE ({why})"),
            }
        );
        let _ = writeln!(
            detail,
            "  WHvRegisterPendingEvent (ExtINT {IRQ0_VECTOR:#04x}): {}",
            outcome(&attempt.pending_event_write)
        );
        let _ = writeln!(
            detail,
            "  WHvRegisterInternalActivityState (halt_suspend=false): {}",
            match &attempt.activity_write {
                None => "not attempted (control)".to_owned(),
                Some(result) => outcome(result),
            }
        );
        let _ = writeln!(
            detail,
            "  guest reached the store past its first HLT: {}",
            attempt.resumed_past_the_halt
        );
    }

    let main = park_then_wake(LocalApicMode::X2Apic, true)?;
    let control_a = park_then_wake(LocalApicMode::X2Apic, false)?;
    let control_b = park_then_wake(LocalApicMode::None, true)?;

    let mut detail = String::new();
    record(&mut detail, "X2Apic, pending ExtINT then halt_suspend=false:", &main);
    record(&mut detail, "\ncontrol A — X2Apic, pending ExtINT and NO activity write:", &control_a);
    record(&mut detail, "\ncontrol B — None, the mode the earlier probe refused in:", &control_b);

    let woke = main.runs.last().is_some_and(|run| run.handler_ran);
    let woke_without_clear = control_a.runs.last().is_some_and(|run| run.handler_ran);
    let clear_accepted = matches!(main.activity_write, Some(Ok(())));
    let answer = match (woke, clear_accepted, woke_without_clear) {
        (true, true, false) =>
            "YES — under X2Apic the activity register IS writable, and clearing \
             HaltSuspend is what wakes the parked processor: with the ExtINT \
             placed but the suspend left standing, the handler never ran."
                .to_owned(),
        (true, true, true) =>
            "YES, IT WAKES — but the clear is not what does it. The pending \
             ExtINT alone wakes the parked processor; the activity write is \
             accepted and changes nothing."
                .to_owned(),
        (true, false, _) => format!(
            "IT WAKES, BUT NOT BY CLEARING THE SUSPEND — the activity write was \
             {}. The pending ExtINT alone is what reaches the guest.",
            match &main.activity_write {
                Some(Err(why)) => why.clone(),
                Some(Ok(())) | None => "not attempted".to_owned(),
            }
        ),
        (false, _, _) =>
            "NO — the parked processor did NOT take the host-placed ExtINT, with \
             or without the halt-suspend clear. The legacy interrupt path needs \
             a different shape on this host."
                .to_owned(),
    };
    Ok(Finding { question: Q11, answer, detail })
}

fn q12_halt_exit_under_x2apic() -> WhpResult<Finding> {
    fn attempt(apic: LocalApicMode, label: &str) -> WhpResult<HaltedRun> {
        let mut guest = Guest::new(|config| {
            config.local_apic(apic)?;
            Ok(())
        })?;
        guest.load(CLI_THEN_HALT)?;
        timed_run(&mut guest, label, RESCUE_AFTER)
    }

    /// `CLI_THEN_HALT` is `cli` at [`layout::CODE`] and `hlt` at the byte after
    /// it, so the halt itself is here and a processor still standing on
    /// [`layout::CODE`] never executed anything at all.
    const HALT_AT: u64 = layout::CODE + 1;

    // The control is the mode a halt is already known to exit in, so a
    // difference between the rows is about the APIC mode and nothing else.
    let x2apic = attempt(LocalApicMode::X2Apic, "X2Apic: cli; hlt")?;
    let none = attempt(LocalApicMode::None, "None (control): cli; hlt")?;

    let exits = matches!(x2apic.reason, ExitReason::Halt) && !x2apic.rescued;
    // A rescue on its own is an absence of evidence: it says nothing came back,
    // not that the halt was reached. RIP is what separates "parked in the HLT"
    // from "never got there", so the verdict is read off it rather than off the
    // rescue.
    let reached_the_halt = x2apic.rip >= HALT_AT;
    let x2apic_rip = x2apic.rip;
    let mut detail = String::new();
    write_runs(&mut detail, &[x2apic, none]);
    let _ = writeln!(
        &mut detail,
        "  the rescue cancel fires at {RESCUE_AFTER:?}; a row at that elapsed \
         with rescued=true is a run that never returned on its own"
    );
    let _ = writeln!(
        &mut detail,
        "  the guest is `cli` at {:#x} then `hlt` at {HALT_AT:#x}; the X2Apic \
         run was cancelled at rip={x2apic_rip:#x}, which is {}",
        layout::CODE,
        if reached_the_halt {
            "AT OR PAST the halt — the processor executed and parked in it, so \
             the absent exit is an absent exit and not an unreached instruction"
        } else {
            "BEFORE the halt — the processor never reached it, and this run \
             says nothing about whether a halt exits"
        }
    );
    let _ = writeln!(
        &mut detail,
        "  positive corroboration is P1's, not this experiment's: there a guest \
         parked in `sti; hlt` under the same mode resumed PAST its halt once the \
         host cleared halt_suspend, writing the RESUMED byte. A processor that \
         never reached a halt cannot resume past one."
    );

    let answer = match (exits, reached_the_halt) {
        (true, _) =>
            "YES — a HLT under X2Apic returns from WHvRunVirtualProcessor with \
             ExitReason::Halt, exactly as it does with no APIC at all. The run \
             loop needs a Halt arm."
                .to_owned(),
        (false, true) => format!(
            "NO — a HLT under X2Apic produces no exit; the hypervisor parks the \
             processor inside WHvRunVirtualProcessor and only a cancel from \
             another thread ends the run. The run loop's Halt arm is \
             unreachable in this mode. The processor genuinely reached the \
             halt: the cancelled run reports rip={x2apic_rip:#x}, at or past \
             the HLT at {HALT_AT:#x}."
        ),
        (false, false) => format!(
            "INCONCLUSIVE — no exit came back, but the cancelled run reports \
             rip={x2apic_rip:#x}, which is before the HLT at {HALT_AT:#x}. The \
             processor never reached the instruction under test, so this run \
             cannot say whether a halt exits."
        ),
    };
    Ok(Finding { question: Q12, answer, detail })
}

fn q13_cancel_stickiness_under_x2apic() -> WhpResult<Finding> {
    let mut guest = Guest::new(|config| {
        config.local_apic(LocalApicMode::X2Apic)?;
        Ok(())
    })?;
    guest.load(SPIN)?;

    // Cancel while the processor is definitely NOT running.
    guest.vcpu.cancel_run()?;
    let run = timed_run(&mut guest, "spin, after a cancel issued while stopped", RESCUE_AFTER)?;

    let sticky = !run.rescued && run.elapsed < RESCUE_AFTER / 2;
    let mut detail = String::new();
    write_runs(&mut detail, std::slice::from_ref(&run));
    let _ = writeln!(
        &mut detail,
        "  the guest is `jmp $`, which exits for nothing; without the earlier \
         cancel this run ends only when the rescue fires at {RESCUE_AFTER:?}"
    );

    let answer = if sticky {
        "YES, STICKY under X2Apic too — a cancel issued while the processor was \
         stopped ended the next run immediately, so the acquire-check every \
         reference VMM performs before a run guards a race the platform already \
         handles."
            .to_owned()
    } else {
        "NO — under X2Apic the earlier cancel was forgotten; the run ended only \
         when a second cancel arrived mid-run. A host must not rely on a \
         pre-run cancel being seen."
            .to_owned()
    };
    Ok(Finding { question: Q13, answer, detail })
}

/// One LINT0 experiment: what the guest's write to the APIC's LINT0 entry did,
/// and whether a host-placed ExtINT then reached the guest with that value
/// standing.
struct Lint0Attempt {
    lvt0_written: u32,
    runs: Vec<HaltedRun>,
    /// What the state page holds in LINT0 once the guest has written it — the
    /// evidence that the write reached the emulated APIC at all, and the only
    /// thing that makes the masked and unmasked attempts a real comparison.
    lvt0_in_page: Result<u32, String>,
    pending_event_write: Result<(), String>,
    activity_write: Result<(), String>,
}

/// Have the guest program LINT0, then place an ExtINT from the host and see
/// whether the handler runs with that LINT0 value standing.
///
/// The APIC is software-enabled from the host before the guest runs, and that
/// is load-bearing rather than tidy: a software-disabled APIC forces the mask
/// bit of every LVT entry, so a guest writing an UNMASKED entry into a disabled
/// APIC reads back a masked one — and the masked-versus-unmasked comparison
/// this experiment exists for would be two runs of the same case.
fn lint0_attempt(lvt0: u32, ask_for_the_trap: bool) -> WhpResult<Lint0Attempt> {
    /// Where the guest's `mov [ds:0x0350], eax` ends, so a host that has to
    /// finish the store itself knows where to put the guest back.
    const AFTER_THE_WRITE: u64 = layout::CODE + 10;

    let mut guest = Guest::new(|config| {
        config.local_apic(LocalApicMode::X2Apic)?;
        if ask_for_the_trap {
            config.extended_vm_exits(ExtendedVmExits {
                apic_write_lint0_trap: true,
                ..ExtendedVmExits::default()
            })?;
        }
        Ok(())
    })?;
    guest.load(&lint0_blob(lvt0))?;
    install_handler(&mut guest, u16::from(IRQ0_VECTOR))?;
    software_enable_the_apic(&guest)?;
    // `Guest::load` leaves every segment flat; the APIC page is out of a
    // real-mode selector's reach, so the base goes in directly.
    guest.vcpu.write_segments(
        &[Reg::Ds],
        &[SegmentRegister { base: layout::XAPIC, limit: 0xFFFF, selector: 0, attributes: 0x93 }],
    )?;

    let mut runs = vec![timed_run(&mut guest, "run 1: write LINT0, then sti; hlt", RESCUE_AFTER)?];
    // The two exits a store to the APIC page can produce leave RIP in
    // DIFFERENT places, and this probe measures both: an `ApicWriteTrap`
    // returns already PAST the store, while an unmapped-range `MemoryAccess`
    // fault returns standing ON it with `instruction_length` zero. Writing
    // `AFTER_THE_WRITE` is therefore a no-op for the first and the fix-up for
    // the second, which is why one assignment serves both; anything else means
    // the guest ran through to its halt on its own. Either way `DS` goes back
    // to flat before the handler runs, because the handler's marker store
    // addresses through it.
    let stopped_on_the_store = matches!(
        runs[0].reason,
        ExitReason::ApicWriteTrap { .. } | ExitReason::MemoryAccess(_)
    );
    guest.vcpu.write_segments(&[Reg::Ds], &[SegmentRegister::real_mode_data(0)])?;
    if stopped_on_the_store {
        guest.vcpu.write_reg(Reg::Rip, AFTER_THE_WRITE)?;
        runs.push(timed_run(&mut guest, "run 2: sti; hlt", RESCUE_AFTER)?);
    }

    let mut page = ApicStatePage::zeroed();
    let lvt0_in_page = guest
        .vcpu
        .read_apic_state(&mut page)
        .map(|()| page.register(ApicRegister::LvtLint0))
        .map_err(|err| err.to_string());

    let pending_event_write = guest
        .vcpu
        .write_words128(
            Reg::PendingEvent,
            PendingExtIntEvent { vector: IRQ0_VECTOR }.as_words(),
        )
        .map_err(|err| err.to_string());
    let activity_write =
        guest.vcpu.set_internal_activity(RUNNING).map_err(|err| err.to_string());
    runs.push(timed_run(&mut guest, "run 3: after the host places an ExtINT", RESCUE_AFTER)?);

    Ok(Lint0Attempt { lvt0_written: lvt0, runs, lvt0_in_page, pending_event_write, activity_write })
}

fn q14_lint0_trap_and_ext_int_gating(host_offers_the_trap: bool) -> WhpResult<Finding> {
    /// Masked, delivery mode ExtINT — what a kernel writes when it wants the
    /// 8259's INTR pin ignored.
    const MASKED_EXT_INT: u32 = 0x0001_0700;
    /// The same entry unmasked, which is the control: if the platform consults
    /// LINT0 at all, only this one may deliver.
    const UNMASKED_EXT_INT: u32 = 0x0000_0700;

    fn record(detail: &mut String, title: &str, attempt: &Lint0Attempt) {
        let _ = writeln!(detail, "{title} (guest writes {:#010x})", attempt.lvt0_written);
        write_runs(detail, &attempt.runs);
        if let Some(first) = attempt.runs.first() {
            let _ = writeln!(
                detail,
                "  the store begins at {:#x}; run 1 came back at rip={:#x} with \
                 instruction_length={}",
                layout::CODE + 6,
                first.rip,
                first.instruction_length
            );
        }
        let _ = writeln!(
            detail,
            "  LINT0 in the state page afterwards: {}",
            match &attempt.lvt0_in_page {
                Ok(value) => format!("{value:#010x}"),
                Err(why) => format!("UNREADABLE ({why})"),
            }
        );
        let _ = writeln!(
            detail,
            "  pending-event write {} / activity write {}",
            outcome(&attempt.pending_event_write),
            outcome(&attempt.activity_write)
        );
    }

    let masked = lint0_attempt(MASKED_EXT_INT, host_offers_the_trap)?;
    let unmasked = lint0_attempt(UNMASKED_EXT_INT, host_offers_the_trap)?;

    let mut detail = String::new();
    let _ = writeln!(
        &mut detail,
        "the host advertises ExtendedVmExits.apic_write_lint0_trap: \
         {host_offers_the_trap}; the APIC is software-enabled by the host before \
         each run, without which a disabled APIC forces every LVT mask bit and \
         the two attempts would be the same case twice"
    );
    record(&mut detail, "\nLINT0 masked:", &masked);
    record(&mut detail, "\nLINT0 unmasked (control):", &unmasked);

    let trapped = masked.runs.first().is_some_and(|run| {
        matches!(run.reason, ExitReason::ApicWriteTrap { register, value }
            if matches!(register, rusty_box_whp::ApicWriteType::Lint0) && value == u64::from(MASKED_EXT_INT))
    });
    let masked_delivered = masked.runs.last().is_some_and(|run| run.handler_ran);
    let unmasked_delivered = unmasked.runs.last().is_some_and(|run| run.handler_ran);

    let trap_half = if !host_offers_the_trap {
        "The LINT0 write trap is NOT offered by this host at all, so a guest \
         reprogramming the pin its 8259 drives cannot be observed by trapping \
         the write."
            .to_owned()
    } else if trapped {
        "The write TRAPS: ExitReason::ApicWriteTrap names LINT0 and carries the \
         value the guest wrote."
            .to_owned()
    } else {
        format!(
            "The trap was asked for and the write did NOT produce one — run 1 \
             came back as {}.",
            masked.runs.first().map_or("nothing".to_owned(), |run| reason_name(&run.reason))
        )
    };
    // The comparison is only a comparison if the two attempts actually left
    // different values in LINT0. They do not when the APIC refuses the write,
    // and a "not gated" read off two identical entries would be an artefact.
    let really_differed = matches!(
        (&masked.lvt0_in_page, &unmasked.lvt0_in_page),
        (Ok(masked_value), Ok(unmasked_value)) if masked_value != unmasked_value
    );
    let gating_half = match (really_differed, masked_delivered, unmasked_delivered) {
        (false, _, _) =>
            " The gating question is CONFOUNDED: the two attempts left the SAME \
             value standing in LINT0, so nothing here compares a masked entry \
             against an unmasked one.",
        (true, false, true) =>
            " A host-placed pending ExtINT IS gated by LINT0: it reached the \
             guest with the entry unmasked and not with it masked, so the \
             platform consults the entry and a fabric need not.",
        (true, true, true) =>
            " A host-placed pending ExtINT is NOT gated by LINT0: it reached the \
             guest with the entry masked just as with it unmasked, so a fabric \
             that must honour the mask has to consult LINT0 itself.",
        (true, true, false) =>
            " INVERTED, which no reading explains: the masked entry delivered \
             and the unmasked one did not. Treat the gating question as open.",
        (true, false, false) =>
            " The gating question is INCONCLUSIVE: neither entry delivered, so \
             the wake path itself did not carry the ExtINT and nothing can be \
             concluded about the mask.",
    };
    Ok(Finding { question: Q14, answer: trap_half + gating_half, detail })
}

fn q15_synthetic_bank_and_hv1(caps: Capabilities) -> WhpResult<Finding> {
    /// The hypervisor vendor leaf, the interface signature, and the feature
    /// leaf a guest reads before it will use either.
    const LEAVES: [u32; 3] = [0x4000_0000, 0x4000_0001, 0x4000_0003];
    /// Leaf 0x40000003 EAX: hypercall MSRs available.
    const HYPERCALL: u64 = 1 << 5;
    /// Leaf 0x40000003 EAX: the VP index MSR is readable.
    const VP_INDEX: u64 = 1 << 6;

    let wanted = SyntheticFeatures::OPENVMM_VTL0;
    let allowed = caps.synthetic_features;
    let mut detail = String::new();
    let _ = writeln!(
        &mut detail,
        "host allows {:#018x}; OPENVMM_VTL0 asks for {:#018x}; asked-for and not \
         allowed {:#018x}; allowed and unnamed by this port {:#018x}",
        allowed.bits(),
        wanted.bits(),
        wanted.difference(allowed).bits(),
        allowed.bits() & !SyntheticFeatures::all().bits()
    );

    // Ask for the whole set first. A refusal names no flag, so the fall-back is
    // the intersection — and which of the two the partition actually got is
    // recorded rather than assumed.
    let mut guest = match Guest::new(|config| {
        config.local_apic(LocalApicMode::X2Apic)?;
        config.synthetic_features(wanted)?;
        Ok(())
    }) {
        Ok(guest) => {
            let _ = writeln!(&mut detail, "the host accepted OPENVMM_VTL0 whole");
            guest
        }
        Err(err) => {
            let _ = writeln!(
                &mut detail,
                "OPENVMM_VTL0 whole was REFUSED ({err}); the partition took the \
                 intersection with what the host allows instead"
            );
            Guest::new(|config| {
                config.local_apic(LocalApicMode::X2Apic)?;
                config.synthetic_features(wanted.intersection(allowed))?;
                Ok(())
            })?
        }
    };

    let mut read = [[0u64; 4]; LEAVES.len()];
    for (slot, leaf) in read.iter_mut().zip(LEAVES) {
        guest.load(&hv_cpuid_blob(leaf))?;
        let exit = guest.run()?;
        guest.vcpu.read_regs(&[Reg::Rax, Reg::Rbx, Reg::Rcx, Reg::Rdx], slot)?;
        let _ = writeln!(
            &mut detail,
            "leaf {leaf:#010x}: left through {} — eax={:#010x} ebx={:#010x} \
             ecx={:#010x} edx={:#010x}",
            reason_name(&exit.reason),
            slot[0],
            slot[1],
            slot[2],
            slot[3]
        );
    }

    let vendor = format!("{}{}{}", ascii_of(read[0][1]), ascii_of(read[0][2]), ascii_of(read[0][3]));
    let interface = ascii_of(read[1][0]);
    let features = read[2][0];
    let _ = writeln!(
        &mut detail,
        "vendor string {vendor:?}, interface signature {interface:?}, feature \
         leaf EAX {features:#010x} (hypercall MSRs {}, VP index {})",
        features & HYPERCALL != 0,
        features & VP_INDEX != 0
    );

    let answer = if vendor == "Microsoft Hv" && interface == "Hv#1" {
        format!(
            "YES — the guest reads vendor {vendor:?} and interface {interface:?}, \
             and the feature leaf reports hypercall MSRs {} and VP index {}. A \
             kernel that gates on those two bits {} take the enlightenments.",
            features & HYPERCALL != 0,
            features & VP_INDEX != 0,
            if features & (HYPERCALL | VP_INDEX) == HYPERCALL | VP_INDEX {
                "WILL"
            } else {
                "will NOT"
            }
        )
    } else {
        format!(
            "NO — with the bank offered, the guest reads vendor {vendor:?} and \
             interface {interface:?}, not \"Microsoft Hv\" and \"Hv#1\"."
        )
    };
    Ok(Finding { question: Q15, answer, detail })
}

fn q16_tsc_deadline_and_apic_clock(caps: Capabilities) -> WhpResult<Finding> {
    /// `HV_X64_MSR_TSC_FREQUENCY`.
    const TSC_FREQUENCY: u32 = 0x4000_0022;
    /// `HV_X64_MSR_APIC_FREQUENCY` — what a guest's APIC timer counts against.
    const APIC_FREQUENCY: u32 = 0x4000_0023;

    let mut detail = String::new();
    let _ = writeln!(
        &mut detail,
        "capability answers: processor clock {} Hz, interrupt clock {} Hz, \
         TSC-deadline timer {}",
        caps.processor_clock_hz, caps.interrupt_clock_hz, caps.tsc_deadline_timer
    );

    let mut guest = Guest::new(|config| {
        config.local_apic(LocalApicMode::X2Apic)?;
        config.synthetic_features(
            SyntheticFeatures::OPENVMM_VTL0.intersection(caps.synthetic_features),
        )?;
        Ok(())
    })?;
    // An MSR the partition was not enlightened for faults, and a real-mode
    // guest with a zeroed vector table would run off into low memory rather
    // than say so.
    install_vector(&mut guest, 13, layout::FAULT_HANDLER, FAULT_MARK)?;

    let mut read = [0u64; 2];
    for (slot, msr) in read.iter_mut().zip([TSC_FREQUENCY, APIC_FREQUENCY]) {
        guest.load(&rdmsr_blob(msr))?;
        guest.poke(layout::MARKER, 0);
        let exit = guest.run()?;
        let faulted = guest.peek(layout::MARKER) == 0xA5;
        let mut halves = [0u64; 2];
        guest.vcpu.read_regs(&[Reg::Rax, Reg::Rdx], &mut halves)?;
        *slot = if faulted {
            0
        } else {
            u64::from(halves[1] as u32) << 32 | u64::from(halves[0] as u32)
        };
        let _ = writeln!(
            &mut detail,
            "guest RDMSR {msr:#010x}: {} — eax={:#010x} edx={:#010x} ({} Hz), left \
             through {}",
            if faulted { "#GP" } else { "answered" },
            halves[0] as u32,
            halves[1] as u32,
            slot,
            reason_name(&exit.reason)
        );
    }

    let tsc_agrees = read[0] == caps.processor_clock_hz;
    let apic_agrees = read[1] == caps.interrupt_clock_hz;
    let answer = format!(
        "TSC-deadline timer {}; processor clock {} Hz and the guest's \
         HV_X64_MSR_TSC_FREQUENCY {} Hz ({}); interrupt clock {} Hz and the \
         guest's HV_X64_MSR_APIC_FREQUENCY {} Hz ({}).",
        caps.tsc_deadline_timer,
        caps.processor_clock_hz,
        read[0],
        if tsc_agrees { "AGREE" } else { "DISAGREE" },
        caps.interrupt_clock_hz,
        read[1],
        if apic_agrees { "AGREE" } else { "DISAGREE" }
    );
    Ok(Finding { question: Q16, answer, detail })
}

fn q17_partition_time_and_the_tsc() -> WhpResult<Finding> {
    /// Long enough that a running clock cannot be mistaken for a held one: a
    /// guest TSC at any plausible frequency moves tens of millions of counts in
    /// this window.
    const NAP: Duration = Duration::from_millis(50);

    // `None`, so the guest's halt produces an exit and the processor is stopped
    // where the clock can be read.
    let mut guest = Guest::new(|_| Ok(()))?;
    guest.load(HALT_LOOP)?;
    let halted = guest.run()?;

    let free_start = guest.vcpu.read_reg(Reg::Tsc)?;
    let reference_start = guest.partition.reference_time_100ns()?;
    std::thread::sleep(NAP);
    let free_end = guest.vcpu.read_reg(Reg::Tsc)?;
    let reference_free = guest.partition.reference_time_100ns()?;

    guest.partition.suspend_time()?;
    let held_start = guest.vcpu.read_reg(Reg::Tsc)?;
    let reference_held_start = guest.partition.reference_time_100ns()?;
    std::thread::sleep(NAP);
    let held_end = guest.vcpu.read_reg(Reg::Tsc)?;
    let reference_held_end = guest.partition.reference_time_100ns()?;

    guest.partition.resume_time()?;
    let resumed = guest.run()?;
    let after_running = guest.vcpu.read_reg(Reg::Tsc)?;

    let runs_while_stopped = free_end > free_start;
    let frozen = held_end == held_start;
    let advances_again = after_running > held_end;

    let mut detail = String::new();
    let _ = writeln!(
        &mut detail,
        "the guest reached its halt: {}; the second run returned {}",
        reason_name(&halted.reason),
        reason_name(&resumed.reason)
    );
    let _ = writeln!(
        &mut detail,
        "  {:<44} {:>22} {:>18}",
        "observation", "guest TSC", "reference 100ns"
    );
    for (label, tsc, reference) in [
        ("after the halt", free_start, reference_start),
        ("+50 ms, partition time RUNNING", free_end, reference_free),
        ("partition time suspended", held_start, reference_held_start),
        ("+50 ms, partition time SUSPENDED", held_end, reference_held_end),
    ] {
        let _ = writeln!(&mut detail, "  {label:<44} {tsc:>22} {reference:>18}");
    }
    let _ = writeln!(
        &mut detail,
        "  {:<44} {after_running:>22} {:>18}",
        "resumed, and the guest ran again",
        guest.partition.reference_time_100ns()?
    );
    let _ = writeln!(
        &mut detail,
        "  the TSC moved {} counts across the running window and {} across the \
         suspended one; it advances while the processor is merely stopped: \
         {runs_while_stopped}",
        free_end.wrapping_sub(free_start),
        held_end.wrapping_sub(held_start)
    );

    let answer = match (runs_while_stopped, frozen, advances_again) {
        (_, true, true) =>
            "YES — suspending partition time freezes the guest's TSC exactly, \
             and resuming lets it advance again. Note the control: the TSC \
             advances while the processor is merely STOPPED, so a host that \
             wants a guest not to see its own downtime must suspend the clock, \
             not merely refrain from running."
                .to_owned(),
        (_, true, false) =>
            "PARTLY — the suspend froze the TSC, but it did not advance again \
             after the resume and another run, which no reading explains."
                .to_owned(),
        (_, false, _) =>
            "NO — the guest's TSC advanced while partition time was suspended. \
             The suspend does not reach the TSC on this host."
                .to_owned(),
    };
    Ok(Finding { question: Q17, answer, detail })
}

/// A vector the guest accepted, and the APIC page as it stood at that moment.
struct Accepted {
    exit_reason: ExitReason,
    delivered: Result<(), String>,
    rescued: bool,
    handler_ran: bool,
    page: ApicStatePage,
}

/// Let a spinning guest accept `vector` and stop INSIDE its handler, before any
/// end-of-interrupt.
///
/// The guest spins rather than halts on purpose: acceptance then depends on
/// nothing this probe is also trying to measure — not on whether a halted
/// processor can be woken, and not on the pending-event slot.
fn accept_without_eoi(vector: u8) -> WhpResult<Accepted> {
    /// Long enough for the guest to be spinning with interrupts enabled.
    const DELIVER_AFTER: Duration = Duration::from_millis(50);
    /// Long enough after that for an acceptance that works to have happened.
    const RESCUE_LATER: Duration = Duration::from_millis(400);

    let mut guest = Guest::new(|config| {
        config.local_apic(LocalApicMode::X2Apic)?;
        Ok(())
    })?;
    guest.load(SPIN_WITH_INTERRUPTS)?;
    install_vector(&mut guest, u16::from(vector), layout::HANDLER, ACCEPT_AND_EXIT)?;
    software_enable_the_apic(&guest)?;

    let requester = guest.partition.interrupt_requester();
    let canceller = guest.vcpu.canceller();
    let (finished, wait_for_finish) = std::sync::mpsc::channel::<()>();
    let outcome = std::thread::scope(|scope| {
        let helper = scope.spawn(move || {
            std::thread::sleep(DELIVER_AFTER);
            let delivered = requester.request(fixed_vector(u32::from(vector), TriggerMode::Edge));
            let rescued = match wait_for_finish.recv_timeout(RESCUE_LATER) {
                Ok(()) => false,
                Err(_) => {
                    canceller.cancel()?;
                    true
                }
            };
            Ok::<_, rusty_box_whp::WhpError>((delivered, rescued))
        });
        let exit = guest.run();
        match finished.send(()) {
            // As in `run_with_rescue`: an error means the rescue already fired,
            // which the exit reason shows.
            Ok(()) | Err(_) => {}
        }
        let (delivered, rescued) = helper.join().expect("the helper thread did not panic")?;
        Ok::<_, rusty_box_whp::WhpError>((exit?, delivered, rescued))
    });
    let (exit, delivered, rescued) = outcome?;

    let mut page = ApicStatePage::zeroed();
    guest.vcpu.read_apic_state(&mut page)?;
    Ok(Accepted {
        exit_reason: exit.reason,
        delivered: delivered.map_err(|err| err.to_string()),
        rescued,
        handler_ran: guest.peek(layout::MARKER) == 0xA5,
        page,
    })
}

/// Set the spurious-interrupt register's software-enable bit, keeping every
/// other bit the platform put there.
///
/// A reset APIC is software-disabled, and what a disabled APIC does with a
/// vector is P2b's question; every experiment that needs a vector to land goes
/// through here first so that answer cannot silently become its premise.
fn software_enable_the_apic(guest: &Guest) -> WhpResult<()> {
    /// Bit 8 of the spurious-interrupt register.
    const SOFTWARE_ENABLE: u32 = 1 << 8;
    let mut page = ApicStatePage::zeroed();
    guest.vcpu.read_apic_state(&mut page)?;
    let spurious = page.register(ApicRegister::Spurious);
    page.set_register(ApicRegister::Spurious, spurious | SOFTWARE_ENABLE);
    guest.vcpu.write_apic_state(&page)
}

fn q18_in_service_versus_trigger_mode() -> WhpResult<Finding> {
    /// Word 2, bit 1 of a bitmap — deliberately not under 32, so a reading
    /// whose words ran the other way would put the bit somewhere else.
    const VECTOR: u8 = 0x41;

    /// Request one vector into an APIC that has never run, and hand back the
    /// page.
    fn requested(trigger: TriggerMode) -> WhpResult<ApicStatePage> {
        let guest = Guest::new(|config| {
            config.local_apic(LocalApicMode::X2Apic)?;
            Ok(())
        })?;
        software_enable_the_apic(&guest)?;
        guest.partition.request_interrupt(fixed_vector(u32::from(VECTOR), trigger))?;
        let mut page = ApicStatePage::zeroed();
        guest.vcpu.read_apic_state(&mut page)?;
        Ok(page)
    }

    let mut detail = String::new();
    let mut expected = [0u32; ApicVector::WORDS];
    expected[usize::from(VECTOR) / 32] = 1 << (VECTOR % 32);
    let _ = writeln!(
        &mut detail,
        "vector {VECTOR:#04x} is bit {} of word {}, so the word this port \
         predicts is {}",
        VECTOR % 32,
        VECTOR / 32,
        bitmap(expected)
    );

    // Probe one: the trigger mode itself. Architecturally the trigger-mode bit
    // is set when an interrupt is accepted INTO THE REQUEST BITMAP, so a level
    // delivery marks it without the processor ever running — and an edge
    // delivery of the same vector does not. That asymmetry is the whole point:
    // it is an observation the platform makes differently for the two fields,
    // which agreeing-with-yourself cannot reproduce.
    let level = requested(TriggerMode::Level)?;
    let edge = requested(TriggerMode::Edge)?;
    for (label, page) in [("level-triggered", &level), ("edge-triggered", &edge)] {
        let _ = writeln!(
            &mut detail,
            "\n{label} request of {VECTOR:#04x}, processor never run:\n  \
             request      {}\n  in-service   {}\n  trigger-mode {}",
            bitmap(page.vector(ApicVector::Request)),
            bitmap(page.vector(ApicVector::InService)),
            bitmap(page.vector(ApicVector::TriggerMode))
        );
    }
    let level_marks_trigger_mode = level.vector(ApicVector::TriggerMode) == expected;
    let level_marks_in_service = level.vector(ApicVector::InService) == expected;
    let edge_marks_neither = edge.vector(ApicVector::TriggerMode) == [0; ApicVector::WORDS]
        && edge.vector(ApicVector::InService) == [0; ApicVector::WORDS];

    // Probe two: acceptance. Priority arbitration reads the in-service bitmap
    // and ignores trigger mode, so a vector the guest has accepted and not
    // acknowledged appears in one of the two and not the other — whatever the
    // first probe said.
    let accepted = accept_without_eoi(VECTOR)?;
    let _ = writeln!(
        &mut detail,
        "\naccepted and NOT acknowledged (guest stopped inside its handler):\n  \
         delivery {} / handler ran {} / exit {} / rescued {}\n  \
         request      {}\n  in-service   {}\n  trigger-mode {}",
        outcome(&accepted.delivered),
        accepted.handler_ran,
        reason_name(&accepted.exit_reason),
        accepted.rescued,
        bitmap(accepted.page.vector(ApicVector::Request)),
        bitmap(accepted.page.vector(ApicVector::InService)),
        bitmap(accepted.page.vector(ApicVector::TriggerMode))
    );
    let in_service_carries_it = accepted.page.vector(ApicVector::InService) == expected;
    let trigger_mode_carries_it = accepted.page.vector(ApicVector::TriggerMode) == expected;

    let _ = writeln!(
        &mut detail,
        "\nthe whole page as the platform left it after the acceptance, so the \
         words still unpinned are on the record as data:{}",
        page_words(&accepted.page)
    );

    let answer = match (
        accepted.handler_ran && in_service_carries_it && !trigger_mode_carries_it,
        accepted.handler_ran && trigger_mode_carries_it && !in_service_carries_it,
        level_marks_trigger_mode && !level_marks_in_service && edge_marks_neither,
    ) {
        (true, _, _) =>
            "CONFIRMED as this port has it — an accepted, unacknowledged vector \
             appears in the field the layout calls IN-SERVICE and not in the one \
             it calls trigger-mode. The two are not swapped."
                .to_owned(),
        (_, true, _) =>
            "SWAPPED — an accepted, unacknowledged vector appears in the field \
             this port calls TRIGGER-MODE. ApicVector::InService and \
             ApicVector::TriggerMode are the wrong way round and must be \
             exchanged before Stage 3 converts the page."
                .to_owned(),
        (_, _, true) =>
            "CONFIRMED by the trigger mode alone — a level-triggered request \
             marks the field the layout calls TRIGGER-MODE and leaves in-service \
             clear, and the same vector edge-triggered marks neither. The \
             acceptance probe did not add to it; see the detail."
                .to_owned(),
        _ =>
            "INCONCLUSIVE — neither the level-versus-edge asymmetry nor the \
             acceptance separated the two bitmaps on this host. The order stays \
             transcribed, not measured; see the raw words in the detail."
                .to_owned(),
    };
    Ok(Finding { question: Q18, answer, detail })
}

/// One named register attempted for its answer rather than its effect: what
/// stood before, what was asked for, how the platform answered, and what stands
/// after.
///
/// A write's status code alone does not settle whether the write took. A
/// platform may accept a name it then ignores, so the read-back is what
/// separates "written" from "acknowledged", and `before` is what says whether
/// the read-back proves anything — a write of the value already standing cannot.
struct NamedRegisterAttempt {
    reg: Reg,
    before: Result<u64, String>,
    wanted: u64,
    write: Result<(), String>,
    after: Result<u64, String>,
}

impl NamedRegisterAttempt {
    /// Read, write, read — with the value to ask for computed from what was
    /// standing, so a register whose legal values depend on its current one can
    /// be moved without asking for an illegal transition.
    fn take(guest: &Guest, reg: Reg, wanted_from: fn(Option<u64>) -> u64) -> Self {
        let before = guest.vcpu.read_reg(reg).map_err(|err| err.to_string());
        let wanted = wanted_from(before.as_ref().ok().copied());
        let write = guest.vcpu.write_reg(reg, wanted).map_err(|err| err.to_string());
        let after = guest.vcpu.read_reg(reg).map_err(|err| err.to_string());
        Self { reg, before, wanted, write, after }
    }

    /// Whether the value the platform reports afterwards is the one that was
    /// asked for. `false` for a refused write, and for a write the platform
    /// accepted and then did not apply.
    fn took(&self) -> bool {
        self.write.is_ok() && self.after.as_ref().is_ok_and(|value| *value == self.wanted)
    }

    /// Whether this attempt could have shown anything at all: a write of the
    /// value already standing agrees with itself no matter what the platform
    /// did with it.
    fn asked_for_a_change(&self) -> bool {
        self.before.as_ref().is_ok_and(|value| *value != self.wanted)
    }

    fn write_row(&self, detail: &mut String) {
        fn word(value: &Result<u64, String>) -> String {
            match value {
                Ok(value) => format!("{value:#018x}"),
                Err(why) => format!("REFUSED ({why})"),
            }
        }
        let _ = writeln!(
            detail,
            "  {:<10} before {:<40} write {:#018x} -> {:<40} after {}",
            format!("{:?}", self.reg),
            word(&self.before),
            self.wanted,
            outcome(&self.write),
            word(&self.after)
        );
    }
}

fn q19_the_apic_page_write_path() -> WhpResult<Finding> {
    /// A task priority that is neither the reset value nor a byte a stray write
    /// would plausibly produce.
    const PRIORITY: u64 = 0x20;
    /// The same priority as `CR8` spells it. `CR8` carries the task-priority
    /// *class* — the top four bits of the TPR — so this is the architectural
    /// image of [`PRIORITY`] in the register the port actually exchanges.
    const PRIORITY_CLASS: u64 = PRIORITY >> 4;
    /// `IA32_APIC_BASE`'s global enable (bit 11) and x2APIC enable (bit 10),
    /// asked for together and never cleared. Setting x2APIC enable on top of a
    /// standing global enable is the one transition the architecture allows
    /// from any starting state; every reverse is illegal, so asking for one
    /// would confuse a refusal of the *value* with a refusal of the *register*.
    const APIC_BASE_ENABLES: u64 = (1 << 11) | (1 << 10);
    /// The architectural reset base, used only to have something legal to ask
    /// for if the read itself is refused.
    const APIC_BASE_RESET: u64 = 0xFEE0_0000;

    let guest = Guest::new(|config| {
        config.local_apic(LocalApicMode::X2Apic)?;
        Ok(())
    })?;
    let mut page = ApicStatePage::zeroed();
    guest.vcpu.read_apic_state(&mut page)?;
    let version = page.register(ApicRegister::Version);
    let spurious = page.register(ApicRegister::Spurious);
    let dfr = page.register(ApicRegister::Dfr);

    guest.vcpu.write_apic_state(&page)?;
    let mut back = ApicStatePage::zeroed();
    guest.vcpu.read_apic_state(&mut back)?;
    let first_kib = page.0[..1024] == back.0[..1024];
    let whole_page = page.0[..] == back.0[..];

    // The other half: the SDK advertises names for a handful of APIC registers.
    // Whether this host honours them decides whether the page is the ONLY write
    // path or merely the widest one — and the question is not answerable from
    // one register, because the page carries neither an APIC base nor a task
    // priority while the engine's state exchange writes both. Each name gets a
    // read, a write of a value that differs from what is standing, and a
    // read-back, so an accepted-and-ignored write is distinguishable from an
    // applied one.
    let tpr_write =
        guest.vcpu.write_reg(Reg::ApicTpr, PRIORITY).map_err(|err| err.to_string());
    let tpr_read = guest.vcpu.read_reg(Reg::ApicTpr).map_err(|err| err.to_string());
    let apic_base = NamedRegisterAttempt::take(&guest, Reg::ApicBase, |before| {
        before.unwrap_or(APIC_BASE_RESET) | APIC_BASE_ENABLES
    });
    let cr8 = NamedRegisterAttempt::take(&guest, Reg::Cr8, |_| PRIORITY_CLASS);

    let mut detail = String::new();
    let _ = writeln!(
        &mut detail,
        "the page of a processor created and never run — no guest instruction \
         has executed, so this is the platform's reset state: version \
         {version:#010x}, spurious {spurious:#010x}, destination format \
         {dfr:#010x}"
    );
    let _ = writeln!(
        &mut detail,
        "read -> write -> read: first KiB identical {first_kib}, whole 4 KiB \
         identical {whole_page}"
    );
    let _ = writeln!(
        &mut detail,
        "WHvX64RegisterApicTpr write of {PRIORITY:#x}: {}",
        outcome(&tpr_write)
    );
    let _ = writeln!(
        &mut detail,
        "WHvX64RegisterApicTpr read: {}",
        match &tpr_read {
            Ok(value) => format!("{value:#x}"),
            Err(why) => format!("REFUSED ({why})"),
        }
    );
    let _ = writeln!(
        &mut detail,
        "the two registers the page carries no field for, each read -> written \
         -> read again on that same X2Apic processor — stopped, and still \
         never run:"
    );
    apic_base.write_row(&mut detail);
    cr8.write_row(&mut detail);
    let _ = writeln!(
        &mut detail,
        "  the write asked for a value other than the one already standing — \
         ApicBase: {}, Cr8: {}",
        apic_base.asked_for_a_change(),
        cr8.asked_for_a_change()
    );
    let _ = writeln!(&mut detail, "the page as read:{}", page_words(&page));

    /// What one named register settled, in the terms the answer may claim: a
    /// refusal, an accepted write the read-back confirms, an accepted write the
    /// read-back contradicts, or an attempt that asked for nothing.
    fn verdict(attempt: &NamedRegisterAttempt) -> String {
        let name = format!("{:?}", attempt.reg);
        match (&attempt.write, attempt.took(), attempt.asked_for_a_change()) {
            (Err(why), _, _) => format!("{name} write REFUSED ({why})"),
            (Ok(()), true, true) => {
                format!("{name} write ACCEPTED and the read-back carries it")
            }
            (Ok(()), true, false) => format!(
                "{name} write ACCEPTED, but it asked for the value already \
                 standing, so it shows only that the name is accepted"
            ),
            (Ok(()), false, _) => {
                format!("{name} write ACCEPTED and the read-back does NOT carry it")
            }
        }
    }

    let every_attempted_name_refused =
        tpr_write.is_err() && apic_base.write.is_err() && cr8.write.is_err();
    let answer = format!(
        "The page round-trips {}. The named registers, one at a time on an \
         X2Apic partition: WHvX64RegisterApicTpr write {}, read {}; {}; {}. {}",
        if first_kib { "bit-identically over its first KiB" } else { "with CHANGES in its first KiB" },
        outcome(&tpr_write),
        match &tpr_read {
            Ok(value) => format!("{value:#x}"),
            Err(why) => format!("REFUSED ({why})"),
        },
        verdict(&apic_base),
        verdict(&cr8),
        if every_attempted_name_refused {
            "Every named register attempted here is refused; the names this \
             probe did not attempt are unmeasured."
                .to_owned()
        } else {
            "So the page is NOT the only write path: state the page carries no \
             field for still crosses through the named registers that were \
             accepted."
                .to_owned()
        }
    );
    Ok(Finding { question: Q19, answer, detail })
}

fn q20_a_software_disabled_apic_drops_a_vector() -> WhpResult<Finding> {
    /// Word 2, bit 3 — as with Q18, high enough that a bitmap read the wrong
    /// way round would not land here.
    const VECTOR: u32 = 0x43;

    let guest = Guest::new(|config| {
        config.local_apic(LocalApicMode::X2Apic)?;
        Ok(())
    })?;
    let mut page = ApicStatePage::zeroed();
    guest.vcpu.read_apic_state(&mut page)?;
    let spurious_at_reset = page.register(ApicRegister::Spurious);

    let while_disabled = guest
        .partition
        .request_interrupt(fixed_vector(VECTOR, TriggerMode::Edge))
        .map_err(|err| err.to_string());
    guest.vcpu.read_apic_state(&mut page)?;
    let bitmap_while_disabled = page.vector(ApicVector::Request);

    software_enable_the_apic(&guest)?;
    guest.vcpu.read_apic_state(&mut page)?;
    let spurious_enabled = page.register(ApicRegister::Spurious);
    let bitmap_after_enabling = page.vector(ApicVector::Request);

    let while_enabled = guest
        .partition
        .request_interrupt(fixed_vector(VECTOR, TriggerMode::Edge))
        .map_err(|err| err.to_string());
    guest.vcpu.read_apic_state(&mut page)?;
    let bitmap_while_enabled = page.vector(ApicVector::Request);

    let mut expected = [0u32; ApicVector::WORDS];
    expected[VECTOR as usize / 32] = 1 << (VECTOR % 32);

    let mut detail = String::new();
    let _ = writeln!(
        &mut detail,
        "spurious register at reset {spurious_at_reset:#010x} (software enable \
         is bit 8), after the host sets the enable {spurious_enabled:#010x}"
    );
    let _ = writeln!(
        &mut detail,
        "  {:<48} {:<10} {}",
        "step", "call", "request bitmap"
    );
    for (label, call, bits) in [
        ("request of 0x43 with the APIC DISABLED", outcome(&while_disabled), bitmap_while_disabled),
        ("(no request) after the host enables the APIC", "-".to_owned(), bitmap_after_enabling),
        ("request of 0x43 with the APIC ENABLED", outcome(&while_enabled), bitmap_while_enabled),
    ] {
        let _ = writeln!(&mut detail, "  {label:<48} {call:<10} {}", bitmap(bits));
    }
    let _ = writeln!(&mut detail, "  the word this port predicts for 0x43 is {}", bitmap(expected));

    let accepted_while_disabled = while_disabled.is_ok();
    let dropped = bitmap_while_disabled == [0; ApicVector::WORDS];
    let latched_late = bitmap_after_enabling != [0; ApicVector::WORDS];
    let lands_when_enabled = bitmap_while_enabled == expected;

    let answer = match (accepted_while_disabled, dropped, latched_late, lands_when_enabled) {
        (true, true, false, true) =>
            "DROPPED, and silently: WHvRequestInterrupt returns SUCCESS with the \
             APIC software-disabled and the request bitmap stays EMPTY; the same \
             call after the host sets the spurious register's enable bit lands \
             the vector. A page conversion must carry the spurious register's \
             enable bit, or restoring a page silently disables the guest's APIC."
                .to_owned(),
        (true, true, true, _) =>
            "LATCHED — the request was accepted and invisible while the APIC was \
             disabled, but appeared in the bitmap as soon as the enable bit was \
             set. Nothing is lost by a disabled APIC on this host."
                .to_owned(),
        (true, false, _, _) =>
            "NOT GATED — the vector landed in the request bitmap with the APIC \
             software-disabled, so the enable bit does not stand between a \
             requested vector and the bitmap."
                .to_owned(),
        (false, _, _, _) => format!(
            "REFUSED — the platform would not take the request with the APIC \
             software-disabled: {}",
            outcome(&while_disabled)
        ),
        (true, true, false, false) =>
            "PARTLY — the disabled APIC dropped the vector, but enabling it did \
             not make the next request land where this port predicts either; see \
             the raw bitmaps."
                .to_owned(),
    };
    Ok(Finding { question: Q20, answer, detail })
}
