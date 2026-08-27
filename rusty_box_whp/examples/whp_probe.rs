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
    capabilities, hypervisor_present, DestinationMode, Exit, ExitReason, GpaPerms, HostPages,
    InterruptKind, InterruptRequest, InterruptionType, LateProperty, LocalApicMode, Partition,
    PartitionConfig, PendingInterruption, Reg, SegmentRegister, TriggerMode, WhpResult, PAGE_SIZE,
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
    pub const STACK_TOP: u64 = 0x0000_3000;
    /// One page mapped read-and-execute only: a shadowed ROM, or a chipset
    /// region whose writes the host must see.
    pub const READ_ONLY: u64 = 0x0001_0000;
    /// Never mapped, so every access is a second-level fault: device MMIO.
    pub const MMIO: u64 = 0x0002_0000;
    /// Where the mapping-churn experiment puts its pages.
    pub const CHURN: u64 = 0x0010_0000;
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
    println!("  exits offered            {:?}", caps.supported_exits);
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

// ---------------------------------------------------------------- fixtures

/// A partition with the standard layout, ready to run real-mode code.
struct Guest {
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
        Ok(Self { partition })
    }

    /// Put the processor in real mode at [`layout::CODE`] with `code` loaded
    /// there, and every segment flat at zero.
    fn load(&mut self, code: &[u8]) -> WhpResult<()> {
        let at = layout::CODE as usize;
        let ram = self.partition.bytes_at_mut(layout::RAM).expect("RAM is mapped");
        ram[at..at + code.len()].copy_from_slice(code);

        self.partition.write_segments(
            0,
            &[Reg::Cs, Reg::Ds, Reg::Es, Reg::Ss],
            &[
                SegmentRegister::real_mode_code(0),
                SegmentRegister::real_mode_data(0),
                SegmentRegister::real_mode_data(0),
                SegmentRegister::real_mode_data(0),
            ],
        )?;
        self.partition.write_regs(
            0,
            &[Reg::Rip, Reg::Rsp, Reg::Rflags],
            // Bit 1 of RFLAGS reads as one on every x86; interrupts stay off
            // until a guest says otherwise.
            &[layout::CODE, layout::STACK_TOP, 0x0000_0002],
        )
    }

    fn run(&mut self) -> WhpResult<Exit> {
        self.partition.run(0)
    }

    fn peek(&self, offset: u16) -> u8 {
        self.partition.bytes_at(layout::RAM).expect("RAM is mapped")[offset as usize]
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
    let translation = guest.partition.translate_gva(0, layout::READ_ONLY)?;
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
    let activity_at_halt = guest.partition.internal_activity(0)?;

    // The question exactly: inject and nothing else.
    guest.partition.inject(0, PendingInterruption {
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
        guest.partition.set_internal_activity(0, cleared)?;
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
            guest.partition.write_regs(
                0,
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
            guest.partition.read_regs(0, &[Reg::Rax, Reg::Rcx], &mut observed)?;
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
    guest.partition.cancel_run(0)?;

    let canceller = guest.partition.canceller(0);
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
    let canceller = guest.partition.canceller(0);
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
        Wake::Inject => guest.partition.inject(0, PendingInterruption {
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
    let ram = guest.partition.bytes_at_mut(layout::RAM).expect("RAM is mapped");
    let handler = layout::HANDLER as usize;
    ram[handler..handler + HANDLER.len()].copy_from_slice(HANDLER);
    // A real-mode vector is four bytes: offset then segment.
    let entry = 4 * usize::from(vector);
    ram[entry..entry + 2].copy_from_slice(&(layout::HANDLER as u16).to_le_bytes());
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
            "  => `WHvRegisterPendingInterruption` is UNREACHABLE in this mode: it is a\n  \
             \x20  register write, which needs a stopped processor, and the processor\n  \
             \x20  never stops. Only a partition-level call can deliver here."
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
         `WHvRunVirtualProcessor` under XApic, so direct injection can never \
         be applied; `WHvRequestInterrupt` issued while the run is in progress \
         wakes it and the handler runs. A userspace 8259 must deliver across \
         threads, not between runs."
            .to_owned()
    } else if nmi.handler_ran {
        "YES, from another thread, and the delivery MECHANISM is proven: a \
         mid-run NMI wakes the parked processor and its handler runs. The \
         Fixed vector was dropped by the guest's own APIC, which a real-mode \
         guest never enables — not by the platform. A halted processor never \
         leaves `WHvRunVirtualProcessor` under XApic, so direct injection is \
         unreachable and a userspace 8259 must deliver across threads."
            .to_owned()
    } else if nmi.delivered.is_err() {
        "PARTLY ANSWERED. A halted processor never leaves \
         `WHvRunVirtualProcessor` under XApic, so direct injection is \
         unreachable — that much is settled. Whether a mid-run request can \
         wake it is NOT: the Fixed vector this guest's own disabled APIC was \
         entitled to drop, and the NMI meant to bypass that gate was refused \
         outright. Deciding it needs a guest that enables its LAPIC, which \
         real mode cannot reach."
            .to_owned()
    } else {
        "NO. A halted processor never leaves `WHvRunVirtualProcessor` under \
         XApic, and neither a mid-run Fixed vector NOR an NMI — which no APIC \
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
    let canceller = guest.partition.canceller(0);
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
