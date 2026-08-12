
/// Emulator construction needs a bigger stack than the default 2 MiB test
/// thread: `Emulator` is ~4 MiB and the debug build materialises a few
/// copies while boxing it. 64 MiB is ample; the previous 256 MiB made
/// enough concurrent reservations to intermittently exhaust the process
/// and fail unrelated tests with STATUS_STACK_OVERFLOW.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
    use super::*;
    use crate::cpu::core_i7_skylake::Corei7SkylakeX;
    use crate::cpu::decoder::Instruction;
    use crate::cpu::{
        instrumentation::{CpuSetupMode, X86Reg},
        rusty_box::MemoryAccessType,
    };
    use crate::cpu::apic::LocalApicCpuEvent;
    use crate::iodev::{DeviceTimerOwner, TimerRequest};
    use crate::memory::CpuMemoryPolicy;
    use crate::pc_system::TimerOwner;
    const TEST_SMP_PACKAGES: u32 = 2;
    const TEST_SMP_CORES: u32 = 1;
    const TEST_SMP_THREADS: u32 = 1;
    const BSP_INDEX: usize = 0;
    const AP_INDEX: usize = 1;
    const AP_TRAMPOLINE_VECTOR: u8 = 0x08;
    const AP_TRAMPOLINE_ADDR: u64 = (AP_TRAMPOLINE_VECTOR as u64) << 12;
    const AP_TRAMPOLINE_OPCODE: u8 = 0x90;
    const AP_TRAMPOLINE_LEN: usize = 32;
    const AP_BATCH_INSTRUCTIONS: u64 = 16;
    const UNSET_APIC_ID: u8 = 0xFF;
    const ACPI_CHECKSUM_VALID_SUM: u8 = 0;
    // ACPI/MADT wire-format offsets used by the fw_cfg table-parsing helpers
    // below. The MADT *builder* lives in `crate::boot` (shared with no-alloc).
    const DIRECT_MADT_SIGNATURE: &[u8; 4] = b"APIC";
    const DIRECT_MADT_HEADER_SIZE: usize = 44;
    const DIRECT_MADT_ENTRY_TYPE_LAPIC: u8 = 0;
    const DIRECT_MADT_ENTRY_TYPE_IOAPIC: u8 = 1;
    const ACPI_TABLE_LENGTH_OFFSET: usize = 4;
    const MADT_ENTRY_TYPE_OFFSET: usize = 0;
    const MADT_ENTRY_LENGTH_OFFSET: usize = 1;
    const MADT_LAPIC_APIC_ID_OFFSET: usize = 3;
    const MADT_IOAPIC_ID_OFFSET: usize = 2;
    const TEST_LAPIC_TIMER_VECTOR: u32 = 0x40;
    const LVT_TIMER_PERIODIC_MODE: u32 = 1 << 17;
    const TEST_LAPIC_TIMER_PERIOD_TICKS: u64 = 10;
    const TEST_LAPIC_TIMER_ELAPSED_TICKS: u32 = 50;
    const FW_CFG_IO_BASE: u16 = 0x510;
    const FW_CFG_DATA_PORT: u16 = 0x511;
    const FW_CFG_NB_CPUS_KEY: u16 = 0x05;
    const FW_CFG_MAX_CPUS_KEY: u16 = 0x0F;
    const FW_CFG_SELECTOR_WRITE_BYTES: u8 = 2;
    const FW_CFG_DATA_READ_BYTES: u8 = 1;
    const NONFLAT_TOPOLOGY_CPUS: u32 = 8;
    const MAX_SUPPORTED_TEST_CPUS: u32 = 254;
    const CPUID_LEAF_FEATURE_INFO: u32 = 0x0000_0001;
    const CPUID_LEAF_EXTENDED_TOPOLOGY: u32 = 0x0000_000B;

    /// Bochs cpu_loop's setjmp handler commits `prev_rip = RIP` whenever an
    /// exception longjmps out — including one raised while DELIVERING an
    /// external interrupt (event.cc HandleExtInterrupt only commits on the
    /// success path). Our injection converts that unwind to `Ok`, so it must
    /// perform the same commit itself: a stale prev_rip would make the next
    /// fault in the nested handler push the WRONG return address.
    #[test]
    fn faulted_interrupt_delivery_commits_prev_rip() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let cfg = EmulatorConfig::default();
                let mut emu = Emulator::new_with_mode(
                    cfg,
                    CpuSetupMode::FlatLong64,
                )
                .expect("new emulator");

                const HANDLER: u64 = 0x0040_0000;
                // Valid 16-byte long-mode interrupt gate for #GP (vector 13)
                // at the reset IDT base (0). Vector 0x20's gate stays all
                // zero (P=0), so delivering it raises #NP/#GP, which nests
                // into this gate.
                let mut gate = [0u8; 16];
                gate[0..2].copy_from_slice(&(HANDLER as u16).to_le_bytes());
                gate[2..4].copy_from_slice(&0x0008u16.to_le_bytes());
                gate[5] = 0x8E; // P=1 DPL=0 type=interrupt gate
                gate[6..8].copy_from_slice(&(((HANDLER >> 16) & 0xFFFF) as u16).to_le_bytes());
                gate[8..12].copy_from_slice(&((HANDLER >> 32) as u32).to_le_bytes());
                // #NP (11) and #GP (13) share the same handler for this test.
                emu.mem_write(11 * 16, &gate).expect("write #NP gate");
                emu.mem_write(13 * 16, &gate).expect("write #GP gate");
                emu.mem_write(HANDLER, &[0xEB, 0xFE]).expect("write handler");
                // The FlatLong64 harness GDT (install_flat_gdt) holds a
                // 32-bit code descriptor at selector 0x08 (the API loads
                // descriptor CACHES directly); gate delivery reloads CS from
                // the GDT and requires L=1 in long mode, so give it a real
                // 64-bit code descriptor.
                emu.mem_write(0x808, &0x00AF_9A00_0000_FFFFu64.to_le_bytes())
                    .expect("write 64-bit code descriptor");
                emu.reg_write(X86Reg::Rsp, 0x0058_0000);

                // SAFETY: memory-bus wiring invariants held by the emulator.
                unsafe { emu.inject_interrupt(0x20) }.expect("inject");

                assert_eq!(
                    emu.cpu().rip(),
                    HANDLER,
                    "the nested exception must have been delivered; \
                     exception ring: {:?}",
                    &emu.cpu().exc_diag_ring[..8]
                );
                assert_eq!(
                    emu.cpu().prev_rip_for_test(),
                    HANDLER,
                    "prev_rip must be committed to the nested handler's RIP \
                     (Bochs cpu.cc setjmp: prev_rip = RIP)"
                );
            })
            .expect("spawn test thread")
            .join()
            .expect("join test thread");
    }
    const CPUID_LEAF1_LOGICAL_COUNT_SHIFT: u32 = 16;
    const CPUID_LEAF1_APIC_ID_SHIFT: u32 = 24;
    const CPUID_APIC_ID_BYTE_MASK: u32 = 0xFF;
    const CPUID_TOPOLOGY_SUBLEAF_SMT: u32 = 0;
    const CPUID_TOPOLOGY_SUBLEAF_CORE: u32 = 1;
    const CPUID_TOPOLOGY_SUBLEAF_PACKAGE: u32 = 2;
    const CPUID_TOPOLOGY_LEVEL_TYPE_SHIFT: u32 = 8;
    const CPUID_TOPOLOGY_LEVEL_TYPE_SMT: u32 = 1;
    const CPUID_TOPOLOGY_LEVEL_TYPE_CORE: u32 = 2;

    /// Allocation offset of the resident block backing guest address 0, and the
    /// base it is measured from.
    ///
    /// A synthetic DTLB entry needs BOTH: the sidecar publishes allocation
    /// offsets, so a test that sets only `mem_host_base` publishes a wild
    /// offset, and the pin silently stops covering the block it names — which
    /// is exactly what these eviction tests exist to prove it does cover.
    fn resident_block(emu: &mut Emulator<'_>) -> (usize, *mut u8) {
        let pins_ptr = emu.tlb_pins().as_ptr();
        let pins_len = emu.tlb_pins().len();
        // Stable CPU pin storage outlives the exclusive memory borrow.
        let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
        let start = emu
            .memory
            .host_mem_range_pinned(0, MemoryAccessType::RW, pins, CpuMemoryPolicy::default())
            .unwrap()
            .expect("resident block must have a pinned direct span")
            .start;
        (start, emu.memory.allocation_span().0)
    }

    fn topology_level_ecx(subleaf: u32, level_type: u32) -> u32 {
        subleaf | (level_type << CPUID_TOPOLOGY_LEVEL_TYPE_SHIFT)
    }

    const ICR_LOW: u64 = 0x300;
    const ICR_HIGH: u64 = 0x310;
    const ICR_TARGET_AP: u32 = 1;
    const ICR_LEVEL_ASSERT: u32 = 1 << 14;
    const ICR_TRIGGER_LEVEL: u32 = 1 << 15;

    fn send_bsp_icr_init(emu: &mut Emulator<'_, ()>) {
        let bsp = emu.cpu_mut_at(BSP_INDEX);
        bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24, 0);
        bsp.lapic.write_aligned(ICR_LOW, ((crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8)
            | ICR_LEVEL_ASSERT
            | ICR_TRIGGER_LEVEL, 0);
        bsp.lapic.write_aligned(ICR_LOW, (crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8 | ICR_TRIGGER_LEVEL, 0);
        emu.refresh_cpu_masks(BSP_INDEX);
    }

    fn send_bsp_icr_sipi(emu: &mut Emulator<'_, ()>, vector: u8) {
        let bsp = emu.cpu_mut_at(BSP_INDEX);
        bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24, 0);
        bsp.lapic.write_aligned(ICR_LOW, vector as u32
            | ((crate::cpu::apic::ApicDeliveryMode::Sipi as u32) << 8)
            | ICR_LEVEL_ASSERT, 0);
        emu.refresh_cpu_masks(BSP_INDEX);
    }

    fn read_fw_cfg_u16(fw_cfg: &mut crate::iodev::fw_cfg::BxFwCfg, key: u16) -> u16 {
        fw_cfg.write_port(
            FW_CFG_IO_BASE,
            key as u32,
            FW_CFG_SELECTOR_WRITE_BYTES,
            None,
            &[],
        );
        let lo = fw_cfg.read_port_mut(FW_CFG_DATA_PORT, FW_CFG_DATA_READ_BYTES) as u16;
        let hi = fw_cfg.read_port_mut(FW_CFG_DATA_PORT, FW_CFG_DATA_READ_BYTES) as u16;
        lo | (hi << 8)
    }

    fn acpi_madt_from_tables(tables: &[u8]) -> &[u8] {
        let offset = tables
            .windows(DIRECT_MADT_SIGNATURE.len())
            .position(|window| window == DIRECT_MADT_SIGNATURE)
            .expect("MADT signature missing from ACPI tables");
        let len = u32::from_le_bytes(
            tables[offset + ACPI_TABLE_LENGTH_OFFSET
                ..offset + ACPI_TABLE_LENGTH_OFFSET + core::mem::size_of::<u32>()]
                .try_into()
                .unwrap(),
        ) as usize;
        &tables[offset..offset + len]
    }

    fn parse_madt_ids<const N: usize>(madt: &[u8]) -> ([u8; N], usize, Option<u8>) {
        let mut offset = DIRECT_MADT_HEADER_SIZE;
        let mut lapic_ids = [UNSET_APIC_ID; N];
        let mut lapic_count = 0usize;
        let mut ioapic_id = None;

        while offset < madt.len() {
            let entry_type = madt[offset + MADT_ENTRY_TYPE_OFFSET];
            let entry_len = madt[offset + MADT_ENTRY_LENGTH_OFFSET] as usize;
            match entry_type {
                DIRECT_MADT_ENTRY_TYPE_LAPIC => {
                    if lapic_count < N {
                        lapic_ids[lapic_count] = madt[offset + MADT_LAPIC_APIC_ID_OFFSET];
                    }
                    lapic_count += 1;
                }
                DIRECT_MADT_ENTRY_TYPE_IOAPIC => {
                    ioapic_id = Some(madt[offset + MADT_IOAPIC_ID_OFFSET]);
                }
                _ => {}
            }
            offset += entry_len;
        }

        (lapic_ids, lapic_count, ioapic_id)
    }

    #[test]
    fn test_emulator_creation() {
        // BxICache contains ~19MB fixed arrays; debug-mode struct literal needs large stack
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let emu = Emulator::new(config).unwrap();
                #[cfg(feature = "std")]
                assert!(emu.bios_output_file.is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn instrumented_constructor_applies_configured_cpuid_frequency() {
        struct NoopTracer;
        impl crate::cpu::instrumentation::Instrumentation for NoopTracer {}

        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE_ADDR: u64 = 0x1000;
                let mut config = EmulatorConfig::default();
                config.cpuid_freq = crate::cpu::cpuid::CpuidFreq::Ips;
                config.ips = 120_000_000;
                let mut emu =
                    Emulator::<NoopTracer>::new_with_mode_and_instrumentation(
                        config,
                        CpuSetupMode::FlatProtected32,
                        NoopTracer,
                    )
                    .unwrap();
                emu.virt_write(CODE_ADDR, &[0x0F, 0xA2, 0xEB, 0xFE])
                    .unwrap();
                emu.reg_write(X86Reg::Rax, 0x15);
                emu.reg_write(X86Reg::Rcx, 0);

                emu.emu_start(CODE_ADDR, Some(CODE_ADDR + 2), None, Some(8))
                    .unwrap();

                assert_eq!(emu.reg_read(X86Reg::Rax), 1);
                assert_eq!(emu.reg_read(X86Reg::Rbx), 1);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 120_000_000);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn test_emulator_initialization() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::new(config).unwrap();
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
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let config = EmulatorConfig::default();

                let mut emu1 = Emulator::new(config.clone()).unwrap();
                let emu2 = Emulator::new(config).unwrap();

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


    #[test]
    fn strict_smp_deadline_keeps_bochs_idle_cpu_quantum_credit() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[0x90; 32], 0x1000).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let quantum = emu.smp_quantum_ticks();
                assert!(quantum > 1);
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, 1, true, false, "one_tick")
                    .unwrap();

                let elapsed = unsafe { emu.run_cpu_batch(quantum / 2) }.unwrap();

                assert_eq!(elapsed, (1 + quantum) / 2);
                assert_eq!(emu.pc_system.time_ticks(), elapsed);
                assert_eq!(emu.smp_tick_remainder, (1 + quantum) % 2);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn equal_smp_deadline_truncates_a_short_final_round() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[0x90; 32], 0x1000).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let quantum = emu.smp_quantum_ticks();
                let short_round = quantum / 2;
                assert!(short_round > 1);
                let _ = unsafe { emu.run_cpu_batch(quantum) }.unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                emu.pc_system.initialize(1_000_000);
                emu.pc_system
                    .register_timer(
                        TimerOwner::NullTimer,
                        short_round,
                        false,
                        true,
                        "equal_short_round",
                    )
                    .unwrap();
                let icount_before = emu.cpu_ref(BSP_INDEX).icount;

                let elapsed = unsafe { emu.run_cpu_batch(short_round) }.unwrap();

                assert_eq!(
                    emu.cpu_ref(BSP_INDEX).icount - icount_before,
                    short_round,
                    "an equal deadline must make the shortened round strict"
                );
                assert_eq!(elapsed, (short_round + quantum) / 2);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn strict_budget_stops_a_linked_branch_chain_exactly() {
        // A hot dec/jnz loop links its back edge (Bochs cpu.cc linkTrace);
        // the strict instruction budget must stop the linked chain at exactly
        // the requested count — the link guard's `iteration < max` is the
        // batch-capped UP form of linkTrace's ticks-left guard.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                const CODE: u64 = 0x1000;
                emu.reg_write(X86Reg::Rcx, 1_000_000);
                // dec ecx; jnz -3
                emu.virt_write(CODE, &[0x49, 0x75, 0xFD]).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);

                let executed =
                    unsafe { emu.run_cpu_batch_with_strict_limit(501, true) }.unwrap();

                assert_eq!(executed, 501, "strict budget must be exact");
                // 501 instructions = 251 decs + 250 taken jnz.
                assert_eq!(emu.reg_read(X86Reg::Rcx), 1_000_000 - 251);
                // The loop is hot enough that the back edge must have linked;
                // the budget stop above therefore covers the linked path.
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn run_cpu_batch_stops_at_the_next_exact_timer_deadline() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = [0x90u8; 8192];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, 1, true, true, "one_tick")
                    .unwrap();

                let executed = unsafe { emu.run_cpu_batch(4096) }.unwrap();
                assert!(
                    (1..128).contains(&executed),
                    "one-tick deadline did not stop the active batch promptly: {executed}"
                );
                assert_eq!(emu.pc_system.time_ticks(), executed);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smi_apm_handshake_runs_the_guest_smm_handler() {
        // The Bochs BIOS smm_init contract (rombios32.c): outb(0xb3, 1),
        // outb(0xb2, 0) raises an SMI (APMC_EN set via ACPI config 0x58 bit
        // 25); the CPU enters SMM at SMBASE+0x8000 = 0x38000 and the GUEST
        // handler clears 0xb3 and RSMs. POST then polls 0xb3 until 0.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();

                // BIOS smm_init: enable SMI generation on APMC writes.
                emu.device_manager.acpi.pci_write(0x58, 1 << 25, 4);

                // Guest code at 0x1000: apms := 1, then the SMI command.
                //   mov al, 1 ; out 0xb3, al ; mov al, 0 ; out 0xb2, al ; nops
                let mut code = [0x90u8; 64];
                code[..8].copy_from_slice(&[0xB0, 0x01, 0xE6, 0xB3, 0xB0, 0x00, 0xE6, 0xB2]);
                emu.virt_write(0x1000, &code).unwrap();
                // SMM handler at 0x38000 (SMBASE 0x30000 + entry 0x8000):
                //   mov al, 0 ; out 0xb3, al ; rsm
                emu.virt_write(0x38000, &[0xB0, 0x00, 0xE6, 0xB3, 0x0F, 0xAA])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                // Run: OUT 0xB2 ends the slice (machine boundary), the
                // boundary delivers the SMI to CPU 0, the next batch enters
                // SMM, runs the handler, and RSM resumes the interrupted code.
                // NOTE: sampling `smm_mode()` at batch boundaries cannot observe
                // the SMM visit — entry, handler and RSM all complete inside a
                // single batch, so the flag is already false at every sample.
                // The deterministic proof that SMM ran is `apms == 0` below:
                // only the handler at 0x38000 issues `out 0xb3, 0`, and that
                // address is reachable only via SMI entry at SMBASE+0x8000.
                for _ in 0..8 {
                    let _ = unsafe { emu.run_cpu_batch(64) }.unwrap();
                    emu.service_scheduler_boundary(0).unwrap();
                    if emu.device_manager.pci2isa.apms == 0
                        && !emu.cpu_mut_at(0).smm_mode()
                        && emu.reg_read(X86Reg::Rip) > 0x1008
                    {
                        break;
                    }
                }

                assert_eq!(
                    emu.device_manager.pci2isa.apms, 0,
                    "the guest SMM handler must clear apms (out 0xb3, 0) — this is \
                     the proof that the SMI was delivered and SMM was entered, since \
                     the handler at 0x38000 is only reachable via SMBASE+0x8000"
                );
                assert!(
                    !emu.cpu_mut_at(0).smm_mode(),
                    "RSM must have exited System Management Mode"
                );
                // Execution resumed past the OUT 0xB2 that raised the SMI (the
                // interrupted instruction stream continues; where it stops among
                // the trailing nops is irrelevant).
                assert!(
                    emu.reg_read(X86Reg::Rip) > 0x1008,
                    "execution must resume after the OUT that raised the SMI"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pit_generates_periodic_irq0_across_multiple_periods() {
        // Linux check_timer() needs the 8254 PIT to deliver *repeated* IRQ0
        // ticks. Program counter 0 in mode 2 (rate generator) and confirm the
        // owner keeps firing, producing many IRQ0 rising edges — not one.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                // 1 tick = 1 microsecond. The rate has to go in the config too,
                // not just into pc_system: `service_scheduler_boundary` converts
                // its budget through `config.ips`, so a config still holding the
                // default would disagree with the clock programmed below.
                let cfg = EmulatorConfig {
                    ips: 1_000_000,
                    ..EmulatorConfig::default()
                };
                let mut emu =
                    Emulator::new_with_mode(cfg, CpuSetupMode::FlatProtected32)
                        .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();

                // Counter 0, LSB/MSB, mode 2 (rate generator), binary: 0x34.
                // Divisor 100 → ~84 us period.
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_CONTROL, 0x34, 1, 0);
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_COUNTER0, 100, 1, 0);
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_COUNTER0, 0, 1, 0);

                let now = emu.pc_system.time_ticks();
                let delay = emu.device_manager.pit.next_event_usec();
                assert!(delay.is_some(), "programmed PIT must have a periodic deadline");
                emu.devices
                    .request_timer_after_usec(DeviceTimerOwner::Pit, now, delay);
                emu.drain_device_timer_requests();

                let before = emu.device_manager.pit.diag_fires;
                // Advance 2 ms; a ~84 us period should fire ~23 times.
                emu.service_scheduler_boundary(2_000).unwrap();
                let fires = emu.device_manager.pit.diag_fires - before;
                assert!(
                    fires >= 5,
                    "PIT mode 2 must generate repeated IRQ0 edges, got {fires}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn atapi_seek_timer_completes_the_read_at_the_exact_deadline() {
        // Bochs harddrv.cc start_seek/seek_timer: an ATAPI READ arms a
        // distance-proportional seek timer; DRQ and the channel IRQ appear
        // only when pc_system reaches that deadline — never before.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                use crate::iodev::harddrv::AtaStatus;

                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.pc_system.initialize(1_000_000); // 1 tick = 1 microsecond
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();

                // 4-sector disc: media init parks curr_lba at 3; READ(10) of
                // LBA 0 seeks |0 - 3 + 1| / 4 of the 80 ms stroke = 40000 us.
                emu.device_manager
                    .ide.drives
                    .attach_cdrom_data(0, 0, vec![0u8; 2048 * 4]);
                {
                    let crate::iodev::devices::DeviceManager { ide, pic, .. } =
                        &mut emu.device_manager;
                    ide.write(0x1f7, 0xA0, 1, pic); // PACKET
                    let packet = [0x28u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
                    for word in packet.chunks_exact(2) {
                        let value = u16::from_le_bytes([word[0], word[1]]) as u32;
                        ide.write(0x1f0, value, 2, pic);
                    }
                }
                // The I/O layer drains the arm right after the OUT dispatch
                // (iodev/mod.rs) — mirror that contract here.
                let now = emu.pc_system.time_ticks();
                let arm = emu.device_manager.ide.drives.take_pending_seek_arm(0, 0);
                assert_eq!(arm, Some(40_000));
                emu.devices.request_timer_after_usec(
                    DeviceTimerOwner::HdSeek(0),
                    now,
                    arm.map(u64::from),
                );
                emu.drain_device_timer_requests();

                // One tick before the deadline: still seeking, no DRQ, no IRQ.
                emu.service_scheduler_boundary(39_999).unwrap();
                let drive = &emu.device_manager.ide.drives.channels[0].drives[0];
                assert!(!drive.controller.status.contains(AtaStatus::DRQ));
                assert!(!drive.controller.interrupt_pending);
                assert!(!emu.device_manager.pic.irq_line_level(14));

                // Crossing the deadline completes the command.
                emu.service_scheduler_boundary(2).unwrap();
                let drive = &emu.device_manager.ide.drives.channels[0].drives[0];
                assert!(drive.controller.status.contains(AtaStatus::DRQ));
                assert!(drive.controller.interrupt_pending);
                assert!(emu.device_manager.pic.irq_line_level(14));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pit_irq0_delivers_through_ioapic_pin2_to_cpu0() {
        // Linux check_timer()'s primary path: the 8259 masks IRQ0 and the
        // timer is routed via IOAPIC pin 2 (GSI2, the IRQ0->GSI2 override) to
        // CPU 0's LAPIC. A PIT tick must deliver the redirection vector.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                // 1 tick = 1 microsecond, in the config as well as in pc_system —
                // `service_scheduler_boundary` converts its budget through
                // `config.ips`.
                let cfg = EmulatorConfig {
                    ips: 1_000_000,
                    ..EmulatorConfig::default()
                };
                let mut emu =
                    Emulator::new_with_mode(cfg, CpuSetupMode::FlatProtected32)
                        .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();

                // PIT counter 0, mode 2, divisor 100 (~84 us period).
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_CONTROL, 0x34, 1, 0);
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_COUNTER0, 100, 1, 0);
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_COUNTER0, 0, 1, 0);
                let now = emu.pc_system.time_ticks();
                let delay = emu.device_manager.pit.next_event_usec();
                emu.devices
                    .request_timer_after_usec(DeviceTimerOwner::Pit, now, delay);
                emu.drain_device_timer_requests();

                // Software-enable CPU 0's LAPIC (spurious vector 0xFF, bit 8).
                emu.cpu_mut().lapic.write_aligned(0xF0, 0x1FF, now);

                // Program IOAPIC pin 2: vector 0x30, fixed, physical dest CPU 0,
                // edge, unmasked. Redirection low index = 0x10 + 2*2 = 0x14.
                emu.device_manager
                    .ioapic
                    .write_aligned(0xFEC0_0000, 0x14, None, None);
                emu.device_manager
                    .ioapic
                    .write_aligned(0xFEC0_0010, 0x30, None, None);
                emu.device_manager
                    .ioapic
                    .write_aligned(0xFEC0_0000, 0x15, None, None);
                emu.device_manager
                    .ioapic
                    .write_aligned(0xFEC0_0010, 0x00, None, None);

                // Linux masks IRQ0 in the 8259 when routing via the IOAPIC.
                emu.device_manager.pic.master.imr |= 0x01;

                // Fire the PIT across several periods.
                emu.service_scheduler_boundary(2_000).unwrap();

                assert_ne!(
                    emu.cpu.pending_event
                        & BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR,
                    0,
                    "IOAPIC pin-2 timer interrupt never raised LAPIC INTR on CPU 0"
                );
                assert_eq!(
                    emu.cpu.lapic.acknowledge_int(),
                    0x30,
                    "CPU 0 LAPIC did not receive the IOAPIC pin-2 timer vector"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn keyboard_one_usec_deadline_ends_up_batch_and_raises_irq1() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(0x1000, &[0x90; 64]).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();
                emu.device_manager.keyboard.send_scancode(0x1E);
                emu.devices.request_timer_after_usec(
                    DeviceTimerOwner::Keyboard,
                    0,
                    Some(1),
                );
                emu.drain_device_timer_requests();

                // The tick-1 fire ends the batch and transfers the byte to the
                // output buffer, but Bochs keyboard.cc periodic() only LATCHES
                // the IRQ on a transfer — it is not raised until the next fire.
                let executed = unsafe { emu.run_cpu_batch(4_096) }.unwrap();
                assert_eq!(executed, 1);
                assert!(emu.device_manager.keyboard.kbd_controller.outb);
                assert_eq!(emu.device_manager.pic.master.irq_in[1], 0);

                // The following serial-delay fire's top-of-function collection
                // raises IRQ1 (one period after the transfer, exactly as Bochs).
                let now = emu.pc_system.time_ticks();
                emu.devices.request_timer_after_usec(
                    DeviceTimerOwner::Keyboard,
                    now,
                    Some(1),
                );
                emu.drain_device_timer_requests();
                unsafe { emu.run_cpu_batch(4_096) }.unwrap();
                assert_ne!(emu.device_manager.pic.master.irq_in[1], 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn deferred_continuous_timer_keeps_its_programmed_period() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const PROGRAMMING_TICKS: u64 = 100;
                const PERIOD_TICKS: u64 = 10;
                let mut emu =
                    Emulator::new(EmulatorConfig::default()).unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::CmosPeriodic,
                        1,
                        false,
                        false,
                        "deferred continuous",
                    )
                    .unwrap();
                emu.device_manager.cmos.periodic_timer_handle = Some(handle);

                emu.devices.request_timer_after_usec_with_mode(
                    DeviceTimerOwner::CmosPeriodic,
                    PROGRAMMING_TICKS,
                    Some(PERIOD_TICKS),
                    true,
                );
                emu.drain_device_timer_requests();

                assert_eq!(
                    emu.pc_system.next_timer_deadline_ticks(),
                    Some(PROGRAMMING_TICKS + PERIOD_TICKS)
                );
                emu.service_scheduler_boundary(PROGRAMMING_TICKS + PERIOD_TICKS)
                    .unwrap();
                assert_eq!(
                    emu.pc_system.next_timer_deadline_ticks(),
                    Some(PROGRAMMING_TICKS + 2 * PERIOD_TICKS),
                    "repeat interval must exclude ticks elapsed before programming"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn deadline_scheduler_preserves_windows_timer_order() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu =
                    Emulator::new(EmulatorConfig::default()).unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();
                emu.device_manager.keyboard.send_scancode(0x1E);
                emu.devices.request_timer(
                    DeviceTimerOwner::Keyboard,
                    TimerRequest::Activate {
                        deadline_ticks: 1,
                        period_ticks: 1,
                        continuous: false,
                    },
                );
                emu.devices.request_timer(
                    DeviceTimerOwner::CmosOneSecond,
                    TimerRequest::Activate {
                        deadline_ticks: 2,
                        period_ticks: 2,
                        continuous: false,
                    },
                );
                emu.drain_device_timer_requests();
                let uip_handle = emu.device_manager.cmos.uip_timer_handle.unwrap();

                emu.service_scheduler_boundary(1).unwrap();
                assert!(emu.device_manager.keyboard.kbd_controller.outb);
                assert!(!emu.pc_system.is_timer_active(uip_handle));

                emu.service_scheduler_boundary(1).unwrap();
                assert!(emu.pc_system.is_timer_active(uip_handle));
                assert_eq!(emu.pc_system.next_timer_deadline_ticks(), Some(246));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn no_fixed_device_polling_state_remains() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(CODE, &[0x90; 512]).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();
                let keyboard_handle = emu.device_manager.keyboard.timer_handle().unwrap();
                let one_second_handle =
                    emu.device_manager.cmos.one_second_timer_handle.unwrap();
                let uip_handle = emu.device_manager.cmos.uip_timer_handle.unwrap();

                // Keyboard, CMOS one-second, and CMOS UIP owners share the
                // first exact deadline. Registration order must deliver the
                // one-second owner before UIP, then rearm UIP for its distinct
                // later deadline. Running this through the CPU loop makes
                // duplicate fixed polling observable as a reordered or
                // replaced UIP arm.
                emu.device_manager.keyboard.send_scancode(0x1E);
                for owner in [
                    DeviceTimerOwner::Keyboard,
                    DeviceTimerOwner::CmosOneSecond,
                    DeviceTimerOwner::CmosUip,
                ] {
                    emu.devices.request_timer(
                        owner,
                        TimerRequest::Activate {
                            deadline_ticks: 1,
                            period_ticks: 1,
                            continuous: false,
                        },
                    );
                }
                emu.drain_device_timer_requests();

                let executed = unsafe { emu.run_cpu_batch(512) }.unwrap();
                assert_eq!(executed, 1, "the tied exact deadline must end the batch");
                assert_eq!(emu.pc_system.time_ticks(), 1);
                assert!(emu.device_manager.keyboard.kbd_controller.outb);
                assert!(!emu.pc_system.is_timer_active(keyboard_handle));
                assert!(!emu.pc_system.is_timer_active(one_second_handle));
                assert!(emu.pc_system.is_timer_active(uip_handle));
                assert_eq!(emu.pc_system.timer_countdown(uip_handle), 244);
                let _ = emu.device_manager.cmos.write(
                    0x70,
                    crate::iodev::cmos::REG_STAT_A as u32,
                    1,
                );
                assert_eq!(
                    emu.device_manager.cmos.read(0x71, 1) & 0x80,
                    0,
                    "tied CMOS owners must fire in one-second-then-UIP order"
                );

                let executed = unsafe { emu.run_cpu_batch(512) }.unwrap();
                assert_eq!(executed, 244, "the mixed owner must fire at its exact deadline");
                assert_eq!(emu.pc_system.time_ticks(), 245);
                assert!(!emu.pc_system.is_timer_active(uip_handle));
                assert_eq!(
                    emu.device_manager.keyboard.kbd_controller.outb as u8,
                    1,
                    "the tied keyboard owner must fire exactly once"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn phase1_tests_subpage_guest_blocks_fetch_sequential_instructions() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const BLOCK_SIZE: usize = 1024;
                const START: u64 = (BLOCK_SIZE - 2) as u64;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 1024 * 1024;
                config.host_memory_size = 1024 * 1024;
                config.memory_block_size = BLOCK_SIZE;
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // Four NOPs cross the 1 KiB guest-block edge.  The following
                // self-loop ends the trace without executing unrelated zero RAM.
                emu.virt_write(START, &[0x90, 0x90, 0x90, 0x90, 0xeb, 0xfe])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, START);

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed = unsafe { emu.run_cpu_batch(4) }.unwrap();
                assert!(executed >= 5);
                assert!(emu.cpu_ref(BSP_INDEX).icount - before >= 5);
                assert_eq!(emu.reg_read(X86Reg::Rip), START + 4);
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn strict_cpu_batch_limit_does_not_execute_trace_tail() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const START: u64 = 0x1000;
                let mut emu =
                    Emulator::new_with_mode(
                        EmulatorConfig::default(),
                        CpuSetupMode::FlatProtected32,
                    )
                    .unwrap();
                emu.virt_write(START, &[0x90, 0x90, 0x90, 0x90, 0xeb, 0xfe])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, START);

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed =
                    unsafe { emu.run_cpu_batch_with_strict_limit(4, true) }.unwrap();

                assert_eq!(executed, 4);
                assert_eq!(emu.cpu_ref(BSP_INDEX).icount - before, 4);
                assert_eq!(emu.reg_read(X86Reg::Rip), START + 4);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn memory_reinitialization_invalidates_every_cpu_host_pin() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = 2 * MIB;
                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::new(config).unwrap();
                let (block, alloc_base) = resident_block(&mut emu);

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.mem_host_base = alloc_base.wrapping_add(block);
                    cpu.mem_alloc_base = alloc_base;
                    let entry = &mut cpu.dtlb.entries[0];
                    entry.lpf = 0;
                    entry.host_page = crate::cpu::tlb::RamPage::from_ram_offset(0);
                }
                emu.refresh_tlb_pins();
                for pin in emu.tlb_pins() {
                    assert!(pin.is_alloc_range_pinned(block, block + MIB));
                }

                emu.init_memory_and_pc_system().unwrap();

                for pin in emu.tlb_pins() {
                    assert!(!pin.is_alloc_range_pinned(block, block + MIB));
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn run_interactive_stops_at_exact_instruction_limit_across_batches() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const START: u64 = 0x1000;
                const LIMIT: u64 = 100_000 + 2;
                let mut emu =
                    Emulator::new_with_mode(
                        EmulatorConfig::default(),
                        CpuSetupMode::FlatProtected32,
                    )
                    .unwrap();
                emu.virt_write(START, &[0x90, 0x90, 0x90, 0x90, 0xeb, 0xfa])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, START);

                let executed = emu.run_interactive(LIMIT).unwrap();

                assert_eq!(executed, LIMIT);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn phase1_tests_emulator_loader_respects_sibling_tlb_pins() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = MIB;

                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.memory.set_a20_mask(u64::MAX);

                emu.load_ram(&[0x5a], 0).unwrap();
                let (block, alloc_base) = resident_block(&mut emu);
                let cpu = emu.cpu_mut_at(AP_INDEX);
                cpu.mem_host_base = alloc_base.wrapping_add(block);
                cpu.mem_alloc_base = alloc_base;
                let entry = &mut cpu.dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page = crate::cpu::tlb::RamPage::from_ram_offset(0);
                assert!(!emu.tlb_pins()[AP_INDEX]
                    .is_alloc_range_pinned(block, block + MIB));
                emu.refresh_tlb_pins();
                assert!(emu.tlb_pins()[AP_INDEX]
                    .is_alloc_range_pinned(block, block + MIB));

                assert!(matches!(
                    emu.load_ram(&[0xa5], MIB as u64),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
                let mut retained = [0];
                emu.mem_read(0, &mut retained).unwrap();
                assert_eq!(retained, [0x5a]);
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn run_cpu_batch_passes_all_cpu_tlb_pins_to_eviction() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 3 * MIB;
                config.host_memory_size = 2 * MIB;
                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.memory.set_a20_mask(u64::MAX);
                emu.cpu_mut_at(AP_INDEX).activity_state = CpuActivityState::WaitForSipi;
                emu.service_scheduler_boundary(0).unwrap();

                // Block 0 is pinned only by the non-running AP.  BSP code
                // occupies the second resident slot; its fetch pin protects
                // that slot.  The store targets swapped block 2, so a
                // current-BSP-only pin set would evict the AP's block 0.
                emu.load_ram(&[0x5a], 0).unwrap();
                emu.load_ram(&[0xC6, 0x07, 0xA5, 0xEB, 0xFE], MIB as u64 + 0x1000)
                    .unwrap();
                let (block, alloc_base) = resident_block(&mut emu);
                let cpu = emu.cpu_mut_at(AP_INDEX);
                cpu.mem_host_base = alloc_base.wrapping_add(block);
                cpu.mem_alloc_base = alloc_base;
                let entry = &mut cpu.dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page = crate::cpu::tlb::RamPage::from_ram_offset(0);
                assert!(!emu.tlb_pins()[AP_INDEX]
                    .is_alloc_range_pinned(block, block + MIB));
                emu.refresh_tlb_pins();
                assert!(emu.tlb_pins()[AP_INDEX]
                    .is_alloc_range_pinned(block, block + MIB));


                emu.reg_write(X86Reg::Rip, MIB as u64 + 0x1000);
                emu.reg_write(X86Reg::Rdi, (2 * MIB) as u64);
                let executed = unsafe { emu.run_cpu_batch(1) }.unwrap();
                assert!(executed >= 1);
                assert!(
                    emu.tlb_pins()[AP_INDEX]
                        .is_alloc_range_pinned(block, block + MIB),
                    "a non-running sibling's direct mapping must survive a BSP slice"
                );

                // Both resident slots are pinned: AP owns block 0 and the
                // running BSP owns its code block.  The target remains
                // unavailable; omitting the AP descriptor makes this read
                // succeed after block 0 is incorrectly evicted.
                let mut target = [0];
                assert!(matches!(
                    emu.mem_read((2 * MIB) as u64, &mut target),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
                let mut retained = [0];
                emu.mem_read(0, &mut retained).unwrap();
                assert_eq!(retained, [0x5a]);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn phase1_tests_unsafe_cpu_mutation_keeps_cached_pin_valid() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = MIB;
                config.memory_block_size = MIB;
                let mut emu = Emulator::new(config).unwrap();
                emu.memory.set_a20_mask(u64::MAX);

                emu.load_ram(&[0x5a], 0).unwrap();
                let (block, alloc_base) = resident_block(&mut emu);
                let cpu_address = emu.cpu() as *const BxCpuC;
                let cpu = unsafe { emu.cpu_mut_unchecked() };
                cpu.mem_host_base = alloc_base.wrapping_add(block);
                cpu.mem_alloc_base = alloc_base;
                let entry = &mut cpu.dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page = crate::cpu::tlb::RamPage::from_ram_offset(0);

                assert_eq!(cpu_address, emu.cpu() as *const BxCpuC);
                assert!(!emu.tlb_pins()[BSP_INDEX]
                    .is_alloc_range_pinned(block, block + MIB));
                emu.refresh_tlb_pins();
                assert!(emu.tlb_pins()[BSP_INDEX]
                    .is_alloc_range_pinned(block, block + MIB));
                assert!(matches!(
                    emu.load_ram(&[0xa5], MIB as u64),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
                let mut retained = [0];
                emu.mem_read(0, &mut retained).unwrap();
                assert_eq!(retained, [0x5a]);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn rep_insw32_fast_path_retires_exact_iteration_count() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x2000;
                const COUNT: u64 = 64;
                const UNMAPPED_PORT: u64 = 0x1234;

                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // REP INSW with 32-bit address size, then a parking jump.
                emu.virt_write(CODE_ADDR, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);
                emu.reg_write(X86Reg::Rcx, COUNT);
                let timer = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Keyboard,
                        COUNT + 1,
                        false,
                        true,
                        "REP deadline",
                    )
                    .unwrap();
                let ticks_before = emu.pc_system.time_ticks();

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed = unsafe { emu.run_cpu_batch(1) }.unwrap();
                let retired = emu.cpu_ref(BSP_INDEX).icount - before;

                // The trace executes REP INSW plus the following parking jump.
                // The REP contributes exactly COUNT retirements, and the batch
                // reports Bochs icount units (retired instructions, including
                // repeat() iterations) — not handler dispatches.
                assert_eq!(executed, retired);
                assert_eq!(retired, COUNT + 1);
                assert_eq!(emu.pc_system.time_ticks() - ticks_before, retired);
                assert_eq!(
                    emu.pc_system.timer_countdown(timer),
                    0,
                    "the timer due on the final REP retirement must fire in this batch"
                );
                assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
                assert_eq!(emu.reg_read(X86Reg::Rdi), DEST_ADDR + COUNT * 2);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn rep_insw32_stops_at_event_budget_across_page_boundary() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x2ff0;
                const EVENT_BUDGET: u64 = 16;
                const COUNT: u64 = 24;
                const IDE_DATA_PORT: u64 = 0x1f0;

                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.attach_disk_data(0, 0, vec![0xa5; 512], 1, 1, 1);
                let drive = &mut emu.device_manager.ide.drives.channels[0].drives[0];
                drive
                    .controller
                    .status
                    .insert(crate::iodev::harddrv::AtaStatus::DRQ);
                drive.controller.buffer[..512].fill(0xa5);
                drive.controller.buffer_size = 512;
                drive.controller.buffer_index = 0;
                emu.pc_system.initialize(1_000_000);
                emu.cpu.mmio = crate::memory::mmio::MmioRegistry::new();

                // REP INSW crosses from 0x2fff to 0x3000 before the timer
                // deadline. The initialized IDE data buffer makes both chunks
                // use the real bulk-port path.
                emu.virt_write(CODE_ADDR, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, IDE_DATA_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);
                emu.reg_write(X86Reg::Rcx, COUNT);
                emu.pc_system
                    .register_timer(
                        TimerOwner::NullTimer,
                        EVENT_BUDGET,
                        false,
                        true,
                        "rep_insw_deadline",
                    )
                    .unwrap();
                assert_eq!(
                    emu.pc_system.get_num_cpu_ticks_left_next_event(),
                    EVENT_BUDGET as u32
                );

                unsafe { emu.run_cpu_batch(1) }.unwrap();

                assert_eq!(emu.reg_read(X86Reg::Rcx), COUNT - EVENT_BUDGET);
                assert_eq!(
                    emu.reg_read(X86Reg::Rdi),
                    DEST_ADDR + EVENT_BUDGET * 2
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smp_batch_with_parked_application_processors_reaches_timer_quantum() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(8, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = vec![0x90u8; 131_072];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed = unsafe { emu.run_cpu_batch(4096) }.unwrap();
                let retired = emu.cpu_ref(BSP_INDEX).icount - before;

                assert!(
                    executed >= 1024,
                    "SMP batch with parked APs collapsed to trace-sized elapsed ticks: {executed}"
                );
                assert!(
                    retired >= 1024,
                    "SMP batch with parked APs collapsed to trace-sized BSP retirement: {retired}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn halted_application_processors_do_not_force_trace_sized_batches() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(8, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = vec![0x90u8; 131_072];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                for cpu_index in 1..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.activity_state = CpuActivityState::Hlt;
                    cpu.async_event = 1;
                }

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed = unsafe { emu.run_cpu_batch(4096) }.unwrap();
                let retired = emu.cpu_ref(BSP_INDEX).icount - before;

                assert!(
                    executed >= 1024,
                    "halted APs collapsed elapsed ticks to {executed}"
                );
                assert!(
                    retired >= 1024,
                    "halted APs forced trace-sized BSP retirement: {retired}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn parked_application_processor_ipi_breaks_bsp_batch_for_delivery() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(2, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = [
                    0xC7, 0x05, 0x10, 0x03, 0xE0, 0xFE, 0x00, 0x00, 0x00,
                    0x01, // ICR high: APIC ID 1
                    0xC7, 0x05, 0x00, 0x03, 0xE0, 0xFE, 0x09, 0x06, 0x00,
                    0x00, // ICR low: SIPI vector 0x09
                    0x90, 0x90, 0x90, 0x90,
                ];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let executed = unsafe { emu.run_cpu_batch(4096) }.unwrap();

                assert!(
                    executed < 4096,
                    "parked-AP fast path must return after IPI write, got full batch {executed}"
                );
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active,
                    "SIPI delivery to parked AP was delayed past the BSP batch"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bochs icache.cc handleSMC: a store that hits a page with cached traces
    /// sets BX_ASYNC_EVENT_STOP_TRACE on the WRITING cpu too, so the remainder
    /// of the currently-executing trace is abandoned and re-decoded from the
    /// (now patched) memory. Without it, the stale tail of the running trace
    /// executes the pre-patch instruction.
    #[test]
    fn smc_store_within_current_trace_stops_trace() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // 0x1000: mov byte [0x100E], 0x41   ; patch "inc eax" -> "inc ecx"
                // 0x1007: nop x7                    ; same trace, past first_bytes
                // 0x100E: inc eax (0x40)            ; stale target
                // 0x100F: hlt
                let mut code = vec![0xC6, 0x05, 0x0E, 0x10, 0x00, 0x00, 0x41];
                code.extend_from_slice(&[0x90; 7]);
                code.push(0x40);
                code.push(0xF4);
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                unsafe { emu.run_cpu_batch(4096) }.unwrap();

                assert_eq!(
                    emu.reg_read(X86Reg::Rcx),
                    1,
                    "patched inc ecx must execute (trace re-decoded after SMC store)"
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Rax),
                    0,
                    "stale inc eax executed from the invalidated trace"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The final fw_cfg DMA OUT in this cached trace overwrites the next
    /// decoded `inc ecx` with HLT.  The issuing CPU must abandon its stale
    /// trace tail before it executes that original instruction.
    #[test]
    fn fw_cfg_dma_out_stops_cached_trace_before_following_instruction() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                const DESCRIPTOR: u64 = 0x3000;
                const KEY: u16 = 0x1234;

                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // `new_with_mode` skips device initialization, so register the
                // real fw_cfg port handler before running guest OUTs.
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.device_manager.fw_cfg.add_bytes(KEY, &[0xF4]);

                // mov edx,0x514; xor eax,eax; out dx,eax;
                // add edx,4; mov eax,bswap(0x3000); out dx,eax;
                // inc ecx; hlt
                let mut code = vec![
                    0xBA, 0x14, 0x05, 0x00, 0x00, 0x31, 0xC0, 0xEF, 0x83, 0xC2, 0x04, 0xB8,
                    0x00, 0x00, 0x30, 0x00, 0xEF,
                ];
                let patched_inc = CODE + code.len() as u64;
                code.extend_from_slice(&[0x41, 0xF4]);

                let control = ((KEY as u32) << 16) | 0x08 | 0x02;
                let mut descriptor = [0u8; 16];
                descriptor[..4].copy_from_slice(&control.to_be_bytes());
                descriptor[4..8].copy_from_slice(&(1u32).to_be_bytes());
                descriptor[8..].copy_from_slice(&patched_inc.to_be_bytes());
                emu.load_ram(&descriptor, DESCRIPTOR).unwrap();
                emu.virt_write(CODE, &code).unwrap();
                emu.reg_write(X86Reg::Rcx, 0);
                emu.reg_write(X86Reg::Rip, CODE);

                unsafe { emu.run_cpu_batch(4096) }.unwrap();

                assert_eq!(emu.mem_read_vec(patched_inc, 1).unwrap(), [0xF4]);
                assert_eq!(emu.cpu_ref(0).activity_state, CpuActivityState::Hlt);
                assert_eq!(
                    emu.reg_read(X86Reg::Rcx),
                    0,
                    "the stale inc ecx after OUT executed before fw_cfg SMC was applied"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bochs icache.cc handleSMC loops over BX_SMP_PROCESSORS: a write by one
    /// CPU to a page with cached traces must flush EVERY cpu's icache, not just
    /// the writer's. The AP spins on a nop-sled loop whose jmp sits past the
    /// 8-byte first_bytes guard; the BSP patches the jmp to hlt;hlt. Stale
    /// sibling caches spin forever.
    #[test]
    fn smp_cross_cpu_code_patch_invalidates_sibling_icache() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(2, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // AP (real mode, SIPI vector 0x09 -> 0x0900:0000 = phys 0x9000):
                //   0x9000: nop x8
                //   0x9008: jmp 0x9000 (EB F6)
                let mut ap_code = vec![0x90u8; 8];
                ap_code.extend_from_slice(&[0xEB, 0xF6]);
                emu.virt_write(0x9000, &ap_code).unwrap();
                // BSP: spin long enough that the AP has cached its loop trace,
                // then patch the AP's jmp to hlt;hlt, then halt.
                let mut bsp_code = vec![0x90u8; 600];
                // mov word [0x9008], 0xF4F4
                bsp_code.extend_from_slice(&[0x66, 0xC7, 0x05, 0x08, 0x90, 0x00, 0x00, 0xF4, 0xF4]);
                bsp_code.push(0xF4); // hlt
                emu.virt_write(0x1000, &bsp_code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(0x09);
                emu.rebuild_cpu_masks_from_scan();

                for _ in 0..100 {
                    unsafe { emu.run_cpu_batch(4096) }.unwrap();
                    if matches!(emu.cpu_ref(AP_INDEX).activity_state, CpuActivityState::Hlt) {
                        break;
                    }
                }
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Hlt,
                    "AP kept executing a stale cached trace after the BSP patched its code"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bochs memory.cc dmaWritePhysicalPage -> pageWriteStampTable.decWriteStamp:
    /// a real legacy-DMA physical write must invalidate cached traces exactly
    /// like CPU stores.
    #[test]
    fn dma_write_to_cached_code_page_invalidates_icache() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                fn dma_read(_data: &[u8], _maxlen: u16) -> u16 {
                    0
                }
                fn dma_write(data: &mut [u8], maxlen: u16) -> u16 {
                    let patch = [0xF4, 0xF4];
                    let len = patch.len().min(maxlen as usize);
                    data[..len].copy_from_slice(&patch[..len]);
                    len as u16
                }

                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // 0x1000: nop x8 / 0x1008: jmp 0x1000 — jmp is past first_bytes.
                let mut code = vec![0x90u8; 8];
                code.extend_from_slice(&[0xEB, 0xF6]);
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                // Let the CPU cache and spin the loop trace.
                unsafe { emu.run_cpu_batch(4096) }.unwrap();
                // A real DMA controller write patches the jmp to hlt;hlt.
                let pins_ptr = emu.tlb_pins().as_ptr();
                let pins_len = emu.tlb_pins().len();
                let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
                let dma = &mut emu.device_manager.dma;
                assert!(dma.register_dma8_channel(2, dma_read, dma_write, "SMC test"));
                dma.s[0].mask[2] = false;
                dma.s[1].mask[0] = false;
                dma.s[0].status_reg |= 1 << 6;
                dma.s[1].status_reg |= 1 << 4;
                dma.s[0].chan[2].current_address = 0x1008;
                dma.s[0].chan[2].current_count = 1;
                dma.s[0].chan[2].mode.transfer_type = 1;
                dma.raise_hlda(Some(&mut emu.memory), pins);

                for _ in 0..50 {
                    unsafe { emu.run_cpu_batch(4096) }.unwrap();
                    if matches!(emu.cpu_ref(0).activity_state, CpuActivityState::Hlt) {
                        break;
                    }
                }
                assert_eq!(
                    emu.cpu_ref(0).activity_state,
                    CpuActivityState::Hlt,
                    "CPU kept executing a stale cached trace after DMA overwrote the code page"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn active_cpu_batch_has_no_fixed_millisecond_polling_cap() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = vec![0x90u8; 131_072];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let executed = unsafe { emu.run_cpu_batch(100_000) }.unwrap();
                assert!(
                    executed >= 100_000,
                    "active batch retained a fixed polling cap: {executed}"
                );

                let mut high_ips_config = EmulatorConfig::default();
                high_ips_config.ips = 300_000_000;
                let mut high_ips_emu = Emulator::new_with_mode(
                    high_ips_config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                high_ips_emu.virt_write(0x1000, &code).unwrap();
                high_ips_emu.reg_write(X86Reg::Rip, 0x1000);

                let executed = unsafe { high_ips_emu.run_cpu_batch(100_000) }.unwrap();
                assert!(
                    executed >= 100_000,
                    "configured IPS reintroduced an active polling cap: {executed}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn keyboard_reset_ack_reaches_bios_poll_before_timeout_at_high_configured_ips() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.ips = 300_000_000;
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = [
                    0xB0, 0xFF, 0xE6, 0x60, 0xB9, 0xFF, 0xFF, 0x00, 0x00, 0xE4, 0x64, 0xA8, 0x01,
                    0x75, 0x0B, 0xE2, 0xF8, 0xC6, 0x05, 0x00, 0x20, 0x00, 0x00, 0xEE, 0xEB, 0x09,
                    0xE4, 0x60, 0xC6, 0x05, 0x00, 0x20, 0x00, 0x00, 0xAA, 0xF4,
                ];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                for _ in 0..16 {
                    let executed = unsafe { emu.run_cpu_batch(100_000) }.unwrap();
                    if !emu.batch_advanced_pc_system {
                        emu.advance_pc_system_after_cpu_ticks(executed);
                    }

                    if emu.virt_read_u8(0x2000).unwrap() != 0 {
                        break;
                    }
                }

                assert_eq!(
                    emu.virt_read_u8(0x2000).unwrap(),
                    0xaa,
                    "BIOS-style keyboard reset poll timed out before OBF/ACK reached port 0x64"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn status_ips_uses_retired_instructions_not_virtual_wait_ticks() {
        let elapsed = std::time::Duration::from_secs(1);

        assert_eq!(
            status_ips_from_retired_instructions(1_000, 1_081, elapsed),
            81
        );
    }
    #[test]
    fn hlt_wait_step_uses_exact_next_timer_deadline() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let emu = Emulator::new(config).unwrap();

                assert_eq!(emu.hlt_wait_step_ticks(), u32::MAX);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn hlt_wait_step_respects_near_pc_system_timer() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::new(config).unwrap();
                emu.pc_system.initialize(emu.config.ips);
                emu.pc_system
                    .register_timer(TimerOwner::Lapic(0), 37, false, true, "near_timer")
                    .unwrap();

                assert_eq!(emu.hlt_wait_step_ticks(), 37);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn slowdown_policy_matches_bochs_quantum() {
        let behind = SlowdownTimerState::decide(500, 600, 0);
        assert_eq!(behind.next_delay_usec, 1_500);
        assert!(!behind.sleep_one_quantum);
        assert_eq!(behind.next_last_time_usec, 1_000);

        let normal = SlowdownTimerState::decide(600, 500, 0);
        assert_eq!(normal.next_delay_usec, 1_000);
        assert!(!normal.sleep_one_quantum);

        let one_second_ahead = SlowdownTimerState::decide(2_000_000, 0, 1_001_000);
        assert_eq!(one_second_ahead.next_delay_usec, 1_000);
        assert!(one_second_ahead.sleep_one_quantum);
    }

    #[cfg(feature = "std")]
    #[test]
    fn slowdown_owner_bounds_hlt_wait() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.sync_slowdown = true;
                let mut emu = Emulator::new(config).unwrap();
                emu.pc_system.initialize(emu.config.ips);
                emu.devices.set_timer_ips(u64::from(emu.config.ips));
                emu.register_timer_owners().unwrap();

                let slowdown_ticks = emu
                    .pc_system
                    .usec_to_ticks(SLOWDOWN_QUANTUM_USEC)
                    .unwrap() as u32;
                assert_eq!(emu.hlt_wait_step_ticks(), slowdown_ticks);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn slowdown_state_reanchors_on_restore() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                use std::io::Cursor;
                let build = || {
                    let config = EmulatorConfig {
                        guest_memory_size: 4 * 1024 * 1024,
                        host_memory_size: 4 * 1024 * 1024,
                        sync_slowdown: true,
                        ..EmulatorConfig::default()
                    };
                    let mut emu = Emulator::new(config).unwrap();
                    emu.initialize().unwrap();
                    emu.reset(ResetReason::Hardware).unwrap();
                    emu
                };

                let mut source = build();
                // Dirty the host pacing history the way a long pre-snapshot
                // run would.
                source.slowdown_timer.last_time_usec = 777_000;
                source.service_scheduler_boundary(0).unwrap();
                let mut saved = Vec::new();
                source.save_snapshot(&mut saved).unwrap();

                let mut restored = build();
                restored.slowdown_timer.last_time_usec = 55; // stale target state
                restored
                    .restore_snapshot(&mut Cursor::new(&saved))
                    .unwrap();

                let handle = restored.slowdown_timer.timer_handle.unwrap();
                restored
                    .pc_system
                    .validate_timer_handle_owner(handle, TimerOwner::Slowdown)
                    .unwrap();
                // Host anchors restart: no pre-restore lead/lag survives and
                // the emulated baseline is the restored virtual clock.
                assert_eq!(restored.slowdown_timer.last_time_usec, 0);
                assert_eq!(
                    restored.slowdown_timer.start_emulated_time_usec,
                    restored.pc_system.time_usec()
                );
                // Pacing continues: the one-shot is armed or already queued.
                assert!(
                    restored.pc_system.is_timer_active(handle)
                        || restored.pc_system.has_fired_owner(TimerOwner::Slowdown)
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn slowdown_config_mismatch_rejects_restore() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                use std::io::Cursor;
                let build = |sync_slowdown: bool| {
                    let config = EmulatorConfig {
                        guest_memory_size: 4 * 1024 * 1024,
                        host_memory_size: 4 * 1024 * 1024,
                        sync_slowdown,
                        ..EmulatorConfig::default()
                    };
                    let mut emu = Emulator::new(config).unwrap();
                    emu.initialize().unwrap();
                    emu.reset(ResetReason::Hardware).unwrap();
                    emu
                };

                for (save_slowdown, restore_slowdown) in [(true, false), (false, true)] {
                    let mut source = build(save_slowdown);
                    source.service_scheduler_boundary(0).unwrap();
                    let mut saved = Vec::new();
                    source.save_snapshot(&mut saved).unwrap();

                    let mut target = build(restore_slowdown);
                    let error = target
                        .restore_snapshot(&mut Cursor::new(&saved))
                        .unwrap_err();
                    assert_eq!(
                        error.kind(),
                        std::io::ErrorKind::InvalidData,
                        "slowdown cross-config restore (save={save_slowdown}, \
                         restore={restore_slowdown}) must be rejected: {error}"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn hardware_reset_rearms_exact_device_owners() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu =
                    Emulator::new(EmulatorConfig::default()).unwrap();
                emu.pc_system.initialize(emu.config.ips);
                emu.devices.set_timer_ips(u64::from(emu.config.ips));
                emu.register_timer_owners().unwrap();
                let keyboard_handle = emu.device_manager.keyboard.timer_handle().unwrap();
                let one_second_handle =
                    emu.device_manager.cmos.one_second_timer_handle.unwrap();
                emu.pc_system
                    .activate_timer_usec(keyboard_handle, 7, false)
                    .unwrap();
                assert!(emu.pc_system.is_timer_active(keyboard_handle));

                emu.reset(ResetReason::Hardware).unwrap();

                assert_eq!(
                    emu.device_manager.keyboard.timer_handle(),
                    Some(keyboard_handle)
                );
                // Bochs keyboard.cc registers the 8042 serial-delay timer
                // continuous and always active — reset restarts it at the
                // serial_delay period instead of deactivating it.
                assert!(emu.pc_system.is_timer_active(keyboard_handle));
                assert!(emu.pc_system.is_timer_active(one_second_handle));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn software_reset_requests_preserve_device_state() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.device_manager.pic.master.imr = 0x00;

                // Port 92 bit 0 requests software reset while bit 1 requests
                // A20 disable. Reset must discard the queued disable.
                assert!(emu.write_port_92h(0x01));
                assert!(emu.pc_system.get_enable_a20());
                assert!(!emu.device_manager.port92.a20_change_pending);
                assert!(!emu.device_manager.keyboard.a20_change_pending);

                // The keyboard output-port form has the same reset-dominates
                // rule, while the PIC remains untouched by software reset.
                emu.device_manager
                    .keyboard
                    .write(crate::iodev::keyboard::KBD_COMMAND_PORT, 0xD1, 1);
                emu.device_manager
                    .keyboard
                    .write(crate::iodev::keyboard::KBD_DATA_PORT, 0x00, 1);
                emu.service_scheduler_boundary(0).unwrap();

                assert_eq!(
                    emu.device_manager.pic.master.imr, 0x00,
                    "Bochs software reset must not reset devices"
                );
                assert!(emu.pc_system.get_enable_a20());
                assert!(emu.device_manager.port92.a20_gate);
                assert!(emu.device_manager.keyboard.a20_enabled);
                assert!(!emu.device_manager.port92.a20_change_pending);
                assert!(!emu.device_manager.keyboard.a20_change_pending);
                assert!(emu.device_manager.port92.reset_request.is_none());
                assert!(emu.device_manager.keyboard.reset_requested.is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pci_cf9_hardware_reset_resets_device_state() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.device_manager.pic.master.imr = 0x00;
                emu.device_manager.pci2isa.reset_request = Some(ResetReason::Hardware);

                assert!(emu.check_and_handle_resets().unwrap());

                assert_eq!(
                    emu.device_manager.pic.master.imr, 0xFF,
                    "Bochs hardware reset resets devices"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pam_boundary_flushes_all_cpu_mappings() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = 2 * MIB;
                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                let (block, alloc_base) = resident_block(&mut emu);

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.mem_host_base = alloc_base.wrapping_add(block);
                    cpu.mem_alloc_base = alloc_base;
                    let entry = &mut cpu.dtlb.entries[0];
                    entry.lpf = 0;
                    entry.host_page = crate::cpu::tlb::RamPage::from_ram_offset(0);
                }
                emu.refresh_tlb_pins();
                assert!(emu
                    .tlb_pins()
                    .iter()
                    .all(|pin| pin.is_alloc_range_pinned(block, block + MIB)));

                emu.device_manager.pci_conf_addr = 0x8000_0058;
                emu.device_manager.pci_write(0x0CFD, 0x30, 1);
                assert!(emu.device_manager.pam_needs_update);
                emu.service_scheduler_boundary(0).unwrap();

                assert!(emu
                    .tlb_pins()
                    .iter()
                    .all(|pin| !pin.is_alloc_range_pinned(block, block + MIB)));
                assert!(emu.memory.memory_type(12, 1));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn a20_port92_and_keyboard_transitions_flush_all_cpu_mappings() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = 2 * MIB;
                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                let (block, alloc_base) = resident_block(&mut emu);

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.mem_host_base = alloc_base.wrapping_add(block);
                    cpu.mem_alloc_base = alloc_base;
                    let entry = &mut cpu.dtlb.entries[0];
                    entry.lpf = 0;
                    entry.host_page = crate::cpu::tlb::RamPage::from_ram_offset(0);
                }
                emu.refresh_tlb_pins();
                emu.write_port_92h(0x00);
                assert!(!emu.pc_system.get_enable_a20());
                assert!(emu
                    .tlb_pins()
                    .iter()
                    .all(|pin| !pin.is_alloc_range_pinned(block, block + MIB)));

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.mem_host_base = alloc_base.wrapping_add(block);
                    cpu.mem_alloc_base = alloc_base;
                    let entry = &mut cpu.dtlb.entries[0];
                    entry.lpf = 0;
                    entry.host_page = crate::cpu::tlb::RamPage::from_ram_offset(0);
                }
                emu.refresh_tlb_pins();
                emu.device_manager.keyboard.write(
                    crate::iodev::keyboard::KBD_COMMAND_PORT,
                    0xDF,
                    1,
                );
                assert!(emu.device_manager.keyboard.a20_change_pending);
                emu.service_scheduler_boundary(0).unwrap();
                assert!(emu.pc_system.get_enable_a20());
                assert!(emu
                    .tlb_pins()
                    .iter()
                    .all(|pin| !pin.is_alloc_range_pinned(block, block + MIB)));

                // Regression for independent controller mirrors. Before the
                // boundary synchronization, each second write matched its own
                // stale mirror and was dropped instead of reaching the global
                // A20 gate.
                emu.device_manager.keyboard.write(
                    crate::iodev::keyboard::KBD_COMMAND_PORT,
                    0xDD,
                    1,
                );
                emu.service_scheduler_boundary(0).unwrap();
                assert!(!emu.pc_system.get_enable_a20());
                emu.device_manager.port92.write(0x02);
                assert!(emu.device_manager.port92.a20_change_pending);
                emu.service_scheduler_boundary(0).unwrap();
                assert!(emu.pc_system.get_enable_a20());

                emu.device_manager.port92.write(0x00);
                assert!(emu.device_manager.port92.a20_change_pending);
                emu.service_scheduler_boundary(0).unwrap();
                assert!(!emu.pc_system.get_enable_a20());
                emu.device_manager.keyboard.write(
                    crate::iodev::keyboard::KBD_COMMAND_PORT,
                    0xD1,
                    1,
                );
                emu.device_manager.keyboard.write(
                    crate::iodev::keyboard::KBD_DATA_PORT,
                    0x03,
                    1,
                );
                emu.service_scheduler_boundary(0).unwrap();
                assert!(emu.pc_system.get_enable_a20());
                assert!(emu.device_manager.port92.a20_gate);
                assert!(emu.device_manager.keyboard.a20_enabled);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn all_reset_ports_stop_before_the_next_instruction() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                let reset_guest = |code: &[u8]| {
                    let mut emu = Emulator::new_with_mode(
                        EmulatorConfig::default(),
                        CpuSetupMode::FlatProtected32,
                    )
                    .unwrap();
                    // `new_with_mode` deliberately omits device initialization.
                    emu.devices.init(&mut emu.memory).unwrap();
                    emu.device_manager
                        .init(&mut emu.devices, &mut emu.memory)
                        .unwrap();
                    emu.virt_write(CODE, code).unwrap();
                    emu.reg_write(X86Reg::Rip, CODE);
                    let executed = unsafe { emu.run_cpu_batch(64) }.unwrap();
                    assert!(executed > 0);
                    assert!(
                        emu.devices.take_port_e9_output().is_empty(),
                        "the visible marker OUT after the reset request executed"
                    );
                    emu
                };

                // mov edx,0x92; mov al,0x01; out dx,al; out 0xe9,al; hlt
                let emu = reset_guest(&[
                    0xBA, 0x92, 0x00, 0x00, 0x00, 0xB0, 0x01, 0xEE, 0xE6, 0xE9, 0xF4,
                ]);
                assert!(emu.pc_system.get_enable_a20());
                assert!(emu.device_manager.port92.a20_gate);
                assert!(emu.device_manager.keyboard.a20_enabled);
                assert!(emu.device_manager.port92.reset_request.is_none());

                // mov edx,0x64; mov al,0xfe; out dx,al; out 0xe9,al; hlt
                let emu = reset_guest(&[
                    0xBA, 0x64, 0x00, 0x00, 0x00, 0xB0, 0xFE, 0xEE, 0xE6, 0xE9, 0xF4,
                ]);
                assert!(emu.device_manager.keyboard.reset_requested.is_none());

                // mov edx,0xcf9; mov al,0x02; out dx,al; mov al,0x06;
                // out dx,al; out 0xe9,al; hlt
                let emu = reset_guest(&[
                    0xBA, 0xF9, 0x0C, 0x00, 0x00, 0xB0, 0x02, 0xEE, 0xB0, 0x06, 0xEE,
                    0xE6, 0xE9, 0xF4,
                ]);
                assert!(emu.device_manager.pci2isa.reset_request.is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn reset_boundary_discards_elapsed_ticks_and_pending_timers() {
        // Bochs pc_system.cc bx_pc_system_c::Reset runs synchronously inside
        // the triggering OUT: no pre-reset elapsed tick, deferred timer
        // request, or queued callback may reach the post-reset machine.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                for hardware in [false, true] {
                    let mut emu = Emulator::new_with_mode(
                        EmulatorConfig::default(),
                        CpuSetupMode::FlatProtected32,
                    )
                    .unwrap();
                    emu.devices.init(&mut emu.memory).unwrap();
                    emu.device_manager
                        .init(&mut emu.devices, &mut emu.memory)
                        .unwrap();

                    // A device files a pre-reset one-shot request, exactly as
                    // a PIT port write would.
                    let now = emu.pc_system.time_ticks();
                    emu.devices
                        .request_timer_after_usec(DeviceTimerOwner::Pit, now, Some(1));

                    if hardware {
                        emu.device_manager.pci2isa.reset_request =
                            Some(ResetReason::Hardware);
                    } else {
                        emu.device_manager.port92.write(0x01);
                    }

                    let t0 = emu.pc_system.time_ticks();
                    let reset_applied = emu.service_scheduler_boundary(10_000).unwrap();
                    assert!(reset_applied, "reset must be reported by the boundary");

                    // Virtual time did not advance: the elapsed ticks were
                    // discarded, so no timer can fire before the first
                    // instruction at the reset vector.
                    assert_eq!(emu.pc_system.time_ticks(), t0);
                    assert!(!emu.pc_system.has_fired_timers());

                    // The pre-reset PIT request is gone (the post-reset rearm
                    // drained its own requests inside reset()).
                    let table = emu.devices.take_timer_requests();
                    assert_eq!(
                        table.get(DeviceTimerOwner::Pit),
                        TimerRequest::Unchanged,
                        "pre-reset timer request survived the reset boundary"
                    );

                    // CPU is at the reset vector.
                    assert_eq!(emu.reg_read(X86Reg::Rip), 0xFFF0);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn legacy_dma_drq_to_hlda_terminal_count_end_to_end() {
        // Bochs dma.cc: set_DRQ -> control_HRQ -> bx_pc_system.set_HRQ(1) ->
        // CPU handleAsyncEvent -> raise_HLDA -> transfer -> terminal count ->
        // set_HRQ(0). The full chain must work through the deferred-request
        // transport: a DRQ must wake the CPU and the TC must drop HRQ.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                fn dma_write(data: &mut [u8], maxlen: u16) -> u16 {
                    let payload = [0xd1, 0xa5, 0x5e, 0x33];
                    let len = payload.len().min(maxlen as usize);
                    data[..len].copy_from_slice(&payload[..len]);
                    len as u16
                }
                fn dma_read(_data: &[u8], _maxlen: u16) -> u16 {
                    0
                }

                const CODE: u64 = 0x1000;
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();

                {
                    let dma = &mut emu.device_manager.dma;
                    assert!(dma.register_dma8_channel(
                        2,
                        dma_read,
                        dma_write,
                        "hrq end-to-end"
                    ));
                    dma.s[0].mask[2] = false;
                    dma.s[1].mask[0] = false;
                    dma.s[0].chan[2].mode.mode_type = 1; // single (Bochs dma.cc)
                    dma.s[0].chan[2].mode.transfer_type = 1; // I/O -> memory
                    dma.s[0].chan[2].page_reg = 0x20;
                    dma.s[0].chan[2].base_count = 3;
                    dma.s[0].chan[2].current_count = 3;
                    dma.set_drq(2, true);
                }

                // The boundary transports the request to pc_system and nudges
                // the CPU (Bochs pc_system.cc set_HRQ).
                assert!(!emu.service_scheduler_boundary(0).unwrap());
                assert!(emu.pc_system.get_hrq());
                assert_ne!(emu.cpu.async_event, 0);

                // nop; hlt — handle_async_event services HLDA before the
                // first instruction completes the batch.
                emu.virt_write(CODE, &[0x90, 0xF4]).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                let executed = unsafe { emu.run_cpu_batch(8) }.unwrap();
                assert!(executed > 0);

                // Payload landed at page_reg 0x20 -> physical 0x20_0000.
                let mut received = [0u8; 4];
                assert_eq!(
                    emu.memory
                        .read_ram(&[], 0x20_0000, &mut received)
                        .unwrap(),
                    4
                );
                assert_eq!(received, [0xd1, 0xa5, 0x5e, 0x33]);

                let dma = &emu.device_manager.dma;
                // Terminal count reached: status bit set, non-autoinit
                // channel re-masked (Bochs dma.cc raise_HLDA).
                assert_ne!(dma.s[0].status_reg & (1 << 2), 0, "TC status not set");
                assert!(dma.s[0].mask[2], "non-autoinit channel must re-mask at TC");
                // The synchronous TC deassert reached pc_system.
                assert!(!emu.pc_system.get_hrq(), "HRQ must drop at terminal count");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn mid_batch_reset_commits_no_pre_reset_ticks() {
        // A guest-triggered reset mid-batch must not commit the instructions
        // executed before the reset as post-reset virtual time.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();

                // mov edx,0x92; mov al,0x01; out dx,al; out 0xe9,al; hlt
                emu.virt_write(
                    CODE,
                    &[0xBA, 0x92, 0x00, 0x00, 0x00, 0xB0, 0x01, 0xEE, 0xE6, 0xE9, 0xF4],
                )
                .unwrap();
                emu.reg_write(X86Reg::Rip, CODE);

                let t0 = emu.pc_system.time_ticks();
                let executed = unsafe { emu.run_cpu_batch(64) }.unwrap();
                assert!(executed > 0);
                assert!(
                    emu.devices.take_port_e9_output().is_empty(),
                    "the marker OUT after the reset request executed"
                );
                // The three pre-reset instructions were discarded, not
                // committed: the post-reset clock still reads the pre-batch
                // epoch.
                assert_eq!(
                    emu.pc_system.time_ticks(),
                    t0,
                    "pre-reset elapsed ticks were committed to the fresh machine"
                );
                assert_eq!(emu.reg_read(X86Reg::Rip), 0xFFF0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn initialize_enables_configured_pci_vga() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                for (pci_enabled, pci_vga) in
                    [(false, false), (false, true), (true, false), (true, true)]
                {
                    let mut config = EmulatorConfig::default();
                    config.pci_enabled = pci_enabled;
                    config.pci_vga = pci_vga;
                    let mut emu = Emulator::new(config).unwrap();
                    emu.initialize().unwrap();

                    let expected = pci_enabled && pci_vga;
                    assert_eq!(emu.device_manager.vga.pci_enabled(), expected);
                    assert_eq!(
                        emu.device_manager.vga.pci_read(0x04, 1),
                        if expected { 0x03 } else { 0xFFFF_FFFF }
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn emulator_new_builds_and_resets_all_configured_cpus() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new(config).unwrap();

                assert_eq!(emu.cpu_count(), TEST_SMP_PACKAGES as usize);
                assert_eq!(emu.cpu_ref(BSP_INDEX).lapic.get_id(), BSP_INDEX as u32);
                assert_eq!(emu.cpu_ref(AP_INDEX).lapic.get_id(), AP_INDEX as u32);

                emu.reset(ResetReason::Hardware).unwrap();

                assert_eq!(
                    emu.cpu_ref(BSP_INDEX).activity_state,
                    CpuActivityState::Active
                );
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn nonflat_topology_reports_bochs_compatible_guest_tables() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default().with_topology(2, 2, 2).unwrap();
                let mut emu = Emulator::new(config).unwrap();
                let instr = Instruction::default();

                assert_eq!(emu.cpu_count(), NONFLAT_TOPOLOGY_CPUS as usize);
                for cpu_index in 0..NONFLAT_TOPOLOGY_CPUS as usize {
                    assert_eq!(emu.cpu_ref(cpu_index).lapic.get_id(), cpu_index as u32);

                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.set_eax(CPUID_LEAF_FEATURE_INFO);
                    cpu.set_ecx(0);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(
                        (cpu.ebx() >> CPUID_LEAF1_LOGICAL_COUNT_SHIFT) & CPUID_APIC_ID_BYTE_MASK,
                        4
                    );
                    assert_eq!(
                        (cpu.ebx() >> CPUID_LEAF1_APIC_ID_SHIFT) & CPUID_APIC_ID_BYTE_MASK,
                        cpu_index as u32
                    );

                    cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
                    cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_SMT);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(cpu.eax(), 1);
                    assert_eq!(cpu.ebx(), 2);
                    assert_eq!(
                        cpu.ecx(),
                        topology_level_ecx(
                            CPUID_TOPOLOGY_SUBLEAF_SMT,
                            CPUID_TOPOLOGY_LEVEL_TYPE_SMT
                        )
                    );
                    assert_eq!(cpu.edx(), cpu_index as u32);

                    cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
                    cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_CORE);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(cpu.eax(), BxCpuC::<()>::bochs_topology_shift(4));
                    assert_eq!(cpu.ebx(), 4);
                    assert_eq!(
                        cpu.ecx(),
                        topology_level_ecx(
                            CPUID_TOPOLOGY_SUBLEAF_CORE,
                            CPUID_TOPOLOGY_LEVEL_TYPE_CORE
                        )
                    );
                    assert_eq!(cpu.edx(), cpu_index as u32);

                    cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
                    cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_PACKAGE);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!((cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()), (0, 0, 0, 0));
                }

                emu.initialize().unwrap();

                assert_eq!(emu.device_manager.ioapic.apic_id(), NONFLAT_TOPOLOGY_CPUS);
                assert_eq!(
                    read_fw_cfg_u16(&mut emu.device_manager.fw_cfg, FW_CFG_NB_CPUS_KEY),
                    NONFLAT_TOPOLOGY_CPUS as u16
                );
                assert_eq!(
                    read_fw_cfg_u16(&mut emu.device_manager.fw_cfg, FW_CFG_MAX_CPUS_KEY),
                    NONFLAT_TOPOLOGY_CPUS as u16
                );

                let acpi = AcpiTableGenerator::generate(
                    emu.config.guest_memory_size as u64,
                    NONFLAT_TOPOLOGY_CPUS,
                );
                let madt = acpi_madt_from_tables(acpi.tables_blob());
                let (lapic_ids, lapic_count, ioapic_id) =
                    parse_madt_ids::<{ NONFLAT_TOPOLOGY_CPUS as usize }>(madt);

                assert_eq!(lapic_count, NONFLAT_TOPOLOGY_CPUS as usize);
                for (expected_id, actual_id) in lapic_ids.iter().copied().enumerate() {
                    assert_eq!(actual_id, expected_id as u8);
                }
                assert_eq!(ioapic_id, Some(NONFLAT_TOPOLOGY_CPUS as u8));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn cpuid_freq_config_reaches_every_cpu_through_cpuid_instruction() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                // `ips` mode: leaf 0x15 must report a crystal of `ips` Hz with
                // a 1/1 ratio and leaf 0x16 the rate in MHz — on the BSP and
                // on every AP (Bochs cpuid.cc get_freq_leaf_15/16).
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default().with_topology(1, 2, 1).unwrap();
                config.ips = 120_000_000;
                config.cpuid_freq = CpuidFreq::Ips;
                let mut emu = Emulator::new(config).unwrap();
                let instr = Instruction::default();

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.set_eax(0x15);
                    cpu.set_ecx(0);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(
                        (cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()),
                        (1, 1, 120_000_000, 0)
                    );

                    cpu.set_eax(0x16);
                    cpu.set_ecx(0);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(
                        (cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()),
                        (120, 120, 100, 0)
                    );

                    // Max standard leaf stays Bochs-exact 0x16 in every mode
                    // (Bochs corei7_skylake-x.cc max_std_leaf).
                    cpu.set_eax(0);
                    cpu.set_ecx(0);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(cpu.eax(), 0x16);
                }

                // Default config (CpuidFreq::None): the frequency leaves read
                // as not enumerated so guests PIT-calibrate the true rate.
                let mut emu = Emulator::new(EmulatorConfig::default()).unwrap();
                let cpu = emu.cpu_mut_at(0);
                cpu.set_eax(0x15);
                cpu.set_ecx(0);
                cpu.cpuid(&instr).unwrap();
                assert_eq!((cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()), (0, 0, 0, 0));
                cpu.set_eax(0x16);
                cpu.set_ecx(0);
                cpu.cpuid(&instr).unwrap();
                assert_eq!((cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()), (0, 0, 0, 0));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn topology_254_generates_non_wrapping_firmware_ids() {
        let params = BxParams::default().with_topology(127, 2, 1).unwrap();
        let cpu_count = params.cpu_count();
        assert_eq!(cpu_count, MAX_SUPPORTED_TEST_CPUS);

        let mut fw_cfg = crate::iodev::fw_cfg::BxFwCfg::new();
        fw_cfg.init(
            EmulatorConfig::default().guest_memory_size as u64,
            cpu_count,
        );
        assert_eq!(
            read_fw_cfg_u16(&mut fw_cfg, FW_CFG_NB_CPUS_KEY),
            MAX_SUPPORTED_TEST_CPUS as u16
        );
        assert_eq!(
            read_fw_cfg_u16(&mut fw_cfg, FW_CFG_MAX_CPUS_KEY),
            MAX_SUPPORTED_TEST_CPUS as u16
        );

        let acpi = AcpiTableGenerator::generate(
            EmulatorConfig::default().guest_memory_size as u64,
            cpu_count,
        );
        let madt = acpi_madt_from_tables(acpi.tables_blob());
        let (lapic_ids, lapic_count, ioapic_id) =
            parse_madt_ids::<{ MAX_SUPPORTED_TEST_CPUS as usize }>(madt);

        assert_eq!(lapic_count, MAX_SUPPORTED_TEST_CPUS as usize);
        assert_eq!(
            lapic_ids[(MAX_SUPPORTED_TEST_CPUS - 1) as usize],
            (MAX_SUPPORTED_TEST_CPUS - 1) as u8
        );
        assert_eq!(ioapic_id, Some(MAX_SUPPORTED_TEST_CPUS as u8));
        assert_eq!(
            madt.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
            ACPI_CHECKSUM_VALID_SUM
        );
    }

    #[test]
    fn cpu_masks_preserve_indices_32_and_253_across_scan_oracle() {
        let mut runnable = CpuMask::default();
        let mut lapic_work = CpuMask::default();
        runnable.assign(32, true);
        runnable.assign(253, true);
        lapic_work.assign(32, true);
        lapic_work.assign(253, true);

        let mut scanned_runnable = CpuMask::default();
        let mut scanned_lapic_work = CpuMask::default();
        for index in 0..MAX_SUPPORTED_TEST_CPUS as usize {
            scanned_runnable.assign(index, index == 32 || index == 253);
            scanned_lapic_work.assign(index, index == 32 || index == 253);
        }

        assert_eq!(runnable, scanned_runnable);
        assert_eq!(lapic_work, scanned_lapic_work);
        assert_eq!(runnable.count(MAX_SUPPORTED_TEST_CPUS as usize), 2);
        assert_eq!(runnable.next_set(0, MAX_SUPPORTED_TEST_CPUS as usize), Some(32));
        assert_eq!(runnable.next_set(33, MAX_SUPPORTED_TEST_CPUS as usize), Some(253));
        assert_eq!(runnable.next_set(254, MAX_SUPPORTED_TEST_CPUS as usize), None);

        // The shipped HLT fast-forward predicate at maximum topology. A
        // 254-CPU Emulator is not constructible in tests (each BxCpuC is tens
        // of megabytes), so the production associated fn is exercised at the
        // mask level across every word boundary the full machine would hit;
        // the 2-CPU transition matrix covers the live plumbing.
        type TestEmu<'a> = Emulator<'a>;
        const MAX: usize = 254;
        assert!(!TestEmu::ap_fast_forward_allowed(runnable, MAX), "bit 32+253");
        assert!(TestEmu::ap_fast_forward_allowed(CpuMask::default(), MAX));
        for ap_bit in [1usize, 32, 63, 64, 191, 192, 253] {
            let mut mask = CpuMask::default();
            mask.assign(ap_bit, true);
            assert!(
                !TestEmu::ap_fast_forward_allowed(mask, MAX),
                "runnable AP bit {ap_bit} must block fast-forward"
            );
        }
        // A runnable BSP alone never blocks AP fast-forward.
        let mut bsp_only = CpuMask::default();
        bsp_only.assign(0, true);
        assert!(TestEmu::ap_fast_forward_allowed(bsp_only, MAX));
        // An AP bit at/beyond the CPU count is outside the topology.
        let mut beyond = CpuMask::default();
        beyond.assign(200, true);
        assert!(TestEmu::ap_fast_forward_allowed(beyond, 100));
    }

    #[test]
    fn cpu_masks_match_scan_oracle_for_scheduler_transition_matrix() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const FIXED_VECTOR: u32 = 0x42;
                const LEVEL_VECTOR: u8 = 0xE0;
                const ICR_SELF: u32 = 1 << 18;
                const ICR_ALL_INCLUDING_SELF: u32 = 2 << 18;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();

                // Reset leaves the BSP runnable and the AP waiting for SIPI.
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi
                );
                emu.assert_cpu_masks_match_scan();
                // WAIT_FOR_SIPI AP: BSP HLT may fast-forward.
                assert!(emu.can_fast_forward_bsp_hlt());

                // HLT and a local wake event update only the affected CPU bit.
                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.activity_state = CpuActivityState::Hlt;
                    bsp.pending_event = 0;
                    bsp.async_event = 0;
                }
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                assert!(!emu.runnable_mask.contains(BSP_INDEX));

                emu.apply_lapic_cpu_event(BSP_INDEX, Some(LocalApicCpuEvent::Nmi));
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(BSP_INDEX));

                // A reset clears the local wake and reinstates WAIT_FOR_SIPI.
                emu.reset(ResetReason::Hardware).unwrap();
                emu.assert_cpu_masks_match_scan();
                emu.apply_lapic_cpu_event(
                    AP_INDEX,
                    Some(LocalApicCpuEvent::Sipi(AP_TRAMPOLINE_VECTOR)),
                );
                emu.refresh_cpu_masks(AP_INDEX);
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(AP_INDEX));
                // SIPI'd (Active) AP: fast-forward is forbidden.
                assert!(!emu.can_fast_forward_bsp_hlt());

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.activity_state = CpuActivityState::Hlt;
                    cpu.set_rflags_for_api(0x202);
                    cpu.pending_event = 0;
                    cpu.async_event = 0;
                    cpu.lapic.intr = false;
                    cpu.lapic.intr_pending = false;
                    cpu.lapic.write_aligned(0xF0, 0x1FF, 0);
                    emu.refresh_cpu_masks(cpu_index);
                }
                emu.assert_cpu_masks_match_scan();
                // Both halted with no pending events: fast-forward allowed.
                assert!(emu.can_fast_forward_bsp_hlt());

                // A physical-destination IPI exercises one remote LAPIC.
                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0x310, (AP_INDEX as u32) << 24, 0);
                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0x300, FIXED_VECTOR, 0);
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.drain_lapic_bus();
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(AP_INDEX));

                // Self shorthand keeps routing and membership local to the BSP.
                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.activity_state = CpuActivityState::Hlt;
                    bsp.pending_event = 0;
                    bsp.async_event = 0;
                    bsp.lapic.intr = false;
                    bsp.lapic.intr_pending = false;
                    bsp.lapic
                        .write_aligned(0x300, ICR_SELF | (FIXED_VECTOR + 1), 0);
                }
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.drain_lapic_bus();
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(BSP_INDEX));

                // All-including-self shorthand updates both target bits.
                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.activity_state = CpuActivityState::Hlt;
                    cpu.pending_event = 0;
                    cpu.async_event = 0;
                    cpu.lapic.intr = false;
                    cpu.lapic.intr_pending = false;
                    emu.refresh_cpu_masks(cpu_index);
                }
                emu.cpu_mut_at(BSP_INDEX).lapic.write_aligned(
                    0x300,
                    ICR_ALL_INCLUDING_SELF | (FIXED_VECTOR + 2),
                    0,
                );
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.drain_lapic_bus();
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(BSP_INDEX));
                assert!(emu.runnable_mask.contains(AP_INDEX));
                // Halted AP holding a delivered IPI wake: fast-forward
                // is forbidden.
                assert!(!emu.can_fast_forward_bsp_hlt());

                // EOI is deferred local LAPIC work until the central boundary.
                {
                    let lapic = &mut emu.cpu_mut_at(BSP_INDEX).lapic;
                    lapic.deliver(LEVEL_VECTOR, 0, crate::cpu::apic::APIC_LEVEL_TRIGGERED);
                    assert_eq!(lapic.acknowledge_int(), LEVEL_VECTOR);
                    lapic.receive_eoi(0);
                    assert_eq!(lapic.pending_eoi_vector, Some(LEVEL_VECTOR));
                }
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.service_lapic_local_events();
                emu.assert_cpu_masks_match_scan();
                assert_eq!(emu.cpu_ref(BSP_INDEX).lapic.pending_eoi_vector, None);

                // Timer programming and fire are both represented as LAPIC work.
                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(1, 0);
                }
                emu.refresh_cpu_masks(AP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.service_lapic_timer_requests();
                emu.assert_cpu_masks_match_scan();
                emu.cpu_mut_at(AP_INDEX).lapic.timer_fired = true;
                emu.refresh_cpu_masks(AP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.service_lapic_local_events();
                emu.assert_cpu_masks_match_scan();

                // Mixed two-CPU state: halted BSP, active AP, AP-only LAPIC work.
                emu.reset(ResetReason::Hardware).unwrap();
                emu.apply_lapic_cpu_event(
                    AP_INDEX,
                    Some(LocalApicCpuEvent::Sipi(AP_TRAMPOLINE_VECTOR)),
                );
                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.activity_state = CpuActivityState::Hlt;
                    bsp.pending_event = 0;
                    bsp.async_event = 0;
                }
                emu.cpu_mut_at(AP_INDEX).lapic.timer_fired = true;
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.refresh_cpu_masks(AP_INDEX);
                emu.assert_cpu_masks_match_scan();
                assert!(!emu.runnable_mask.contains(BSP_INDEX));
                assert!(emu.runnable_mask.contains(AP_INDEX));
                assert!(!emu.lapic_work_mask.contains(BSP_INDEX));
                assert!(emu.lapic_work_mask.contains(AP_INDEX));

                emu.reset(ResetReason::Hardware).unwrap();
                emu.assert_cpu_masks_match_scan();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn run_cpu_batch_executes_application_processor_after_sipi() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu_mut().activity_state = CpuActivityState::WaitForSipi;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                emu.rebuild_cpu_masks_from_scan();
                let before = emu.cpu_ref(AP_INDEX).icount;

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();

                assert!(executed > 0);
                assert!(
                    emu.cpu_ref(AP_INDEX).icount > before,
                    "active AP did not receive a CPU batch"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn bsp_icr_init_sipi_wakes_and_runs_application_processor() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const ICR_LOW: u64 = 0x300;
                const ICR_HIGH: u64 = 0x310;
                const TARGET_APIC_ID: u32 = 1;
                const ICR_LEVEL_ASSERT: u32 = 1 << 14;
                const ICR_TRIGGER_LEVEL: u32 = 1 << 15;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                let before = emu.cpu_ref(AP_INDEX).icount;

                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.lapic.write_aligned(ICR_HIGH, TARGET_APIC_ID << 24, 0);
                    bsp.lapic.write_aligned(ICR_LOW, ((crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8)
                        | ICR_LEVEL_ASSERT
                        | ICR_TRIGGER_LEVEL, 0);
                    bsp.lapic.write_aligned(ICR_LOW, (crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8 | ICR_TRIGGER_LEVEL, 0);
                    bsp.lapic.write_aligned(ICR_HIGH, TARGET_APIC_ID << 24, 0);
                    bsp.lapic.write_aligned(ICR_LOW, AP_TRAMPOLINE_VECTOR as u32
                        | ((crate::cpu::apic::ApicDeliveryMode::Sipi as u32) << 8)
                        | ICR_LEVEL_ASSERT, 0);
                }
                emu.refresh_cpu_masks(BSP_INDEX);

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();

                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active
                );
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).get_cs_selector(),
                    (AP_TRAMPOLINE_VECTOR as u16) << 8
                );
                assert!(emu.cpu_ref(AP_INDEX).rip() > 0);
                assert!(emu.cpu_ref(AP_INDEX).icount > before);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn bsp_icr_init_sipi_restarts_active_application_processor() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const SECOND_TRAMPOLINE_VECTOR: u8 = AP_TRAMPOLINE_VECTOR + 1;
                const SECOND_TRAMPOLINE_ADDR: u64 = (SECOND_TRAMPOLINE_VECTOR as u64) << 12;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], SECOND_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active
                );

                // Bochs deliver_INIT only signals the event; the AP resets at
                // its next instruction boundary. A SIPI sent in the same drain
                // would be dropped ("was not halted at the time"), exactly as
                // in Bochs — so the INIT must be processed before the SIPI is
                // sent, mirroring the MP-spec INIT/SIPI delay.
                send_bsp_icr_init(&mut emu);
                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();
                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi,
                    "INIT must software-reset the AP at its next boundary"
                );

                send_bsp_icr_sipi(&mut emu, SECOND_TRAMPOLINE_VECTOR);
                let before = emu.cpu_ref(AP_INDEX).icount;

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();

                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active
                );
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).get_cs_selector(),
                    (SECOND_TRAMPOLINE_VECTOR as u16) << 8
                );
                assert!(emu.cpu_ref(AP_INDEX).rip() > 0);
                assert!(emu.cpu_ref(AP_INDEX).icount > before);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn init_does_not_recall_ipis_already_sent_by_active_ap() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const IN_FLIGHT_VECTOR: u32 = 0x44;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.write_aligned(0x310, (BSP_INDEX as u32) << 24, 0);
                    ap.lapic.write_aligned(0x300, IN_FLIGHT_VECTOR, 0);
                }
                emu.refresh_cpu_masks(AP_INDEX);
                assert!(!emu.cpu_ref(BSP_INDEX).lapic.intr);

                send_bsp_icr_init(&mut emu);
                emu.drain_lapic_bus();

                // Bochs apic.cc send_ipi delivers the AP's ICR write to the
                // bus before the INIT is even processed — an INIT does not
                // recall an IPI that is already in flight.
                assert!(
                    emu.cpu_ref(BSP_INDEX).lapic.intr,
                    "the AP's in-flight IPI must reach the BSP despite the INIT"
                );
                // The INIT itself is only signaled; the AP resets at its next
                // instruction boundary (Bochs event.cc handleAsyncEvent).
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active
                );
                assert!(emu
                    .cpu_ref(AP_INDEX)
                    .is_unmasked_event_pending(BxCpuC::<()>::BX_EVENT_INIT));

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();
                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn init_ipi_to_active_ap_stays_pending_until_instruction_boundary() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);

                send_bsp_icr_init(&mut emu);
                emu.drain_lapic_bus();

                // Bochs deliver_INIT: signal only — no reset from the bus.
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active,
                    "INIT must not reset the AP before its next boundary"
                );
                assert!(emu
                    .cpu_ref(AP_INDEX)
                    .is_unmasked_event_pending(BxCpuC::<()>::BX_EVENT_INIT));

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();
                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smi_then_init_ipis_are_both_signaled_not_collapsed() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);

                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24, 0);
                    bsp.lapic.write_aligned(ICR_LOW, (crate::cpu::apic::ApicDeliveryMode::Smi as u32) << 8, 0);
                }
                send_bsp_icr_init(&mut emu);
                emu.drain_lapic_bus();

                // Bochs signals both events; SMI is processed before INIT at
                // the AP's next boundary. An eager INIT reset would clear
                // pending_event and destroy the SMI.
                let ap = emu.cpu_ref(AP_INDEX);
                assert!(
                    ap.is_unmasked_event_pending(BxCpuC::<()>::BX_EVENT_SMI),
                    "SMI queued before INIT must survive the drain"
                );
                assert!(
                    ap.is_unmasked_event_pending(BxCpuC::<()>::BX_EVENT_INIT),
                    "INIT must be pending alongside the SMI"
                );
                assert_eq!(ap.activity_state, CpuActivityState::Active);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn run_cpu_batch_uses_smp_time_when_peer_is_started_but_halted() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu_mut().activity_state = CpuActivityState::Hlt;
                emu.cpu_mut().pending_event = 0;
                emu.cpu_mut().async_event = 0;
                emu.cpu_mut().lapic.intr = false;
                emu.cpu_mut().lapic.intr_pending = false;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                emu.rebuild_cpu_masks_from_scan();

                assert!(
                    !emu.cpu_runnable_for_batch(BSP_INDEX),
                    "test requires a started halted peer with no runnable event"
                );
                assert!(emu.cpu_runnable_for_batch(AP_INDEX));

                let before = emu.cpu_ref(AP_INDEX).icount;
                let quantum = emu.smp_quantum_ticks();
                let elapsed = unsafe { emu.run_cpu_batch(quantum) }.unwrap();
                let ap_delta = emu.cpu_ref(AP_INDEX).icount - before;

                assert!(elapsed >= quantum);
                assert!(
                    elapsed < ap_delta,
                    "elapsed ticks {elapsed} were not averaged with the halted peer quantum; AP delta was {ap_delta}"
                );
                assert_eq!(
                    emu.smp_tick_remainder,
                    (quantum + ap_delta) % 2
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smp_batch_keeps_guest_time_at_the_frozen_round_epoch() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[0x0F, 0x31, 0xF4], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu_mut().activity_state = CpuActivityState::Hlt;
                emu.cpu_mut().pending_event = 0;
                emu.cpu_mut().async_event = 0;
                emu.cpu_mut().lapic.intr = false;
                emu.cpu_mut().lapic.intr_pending = false;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                emu.rebuild_cpu_masks_from_scan();

                let quantum = emu.smp_quantum_ticks();
                let _elapsed = unsafe { emu.run_cpu_batch(quantum) }.unwrap();
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).rax(),
                    0,
                    "AP RDTSC observed peer elapsed time before the round boundary"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smp_batch_services_lapic_timer_at_round_boundary() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                let handle = emu
                    .pc_system
                    .register_timer(TimerOwner::Lapic(AP_INDEX), 1, false, false, "ap_lapic")
                    .unwrap();

                emu.cpu_mut().activity_state = CpuActivityState::Hlt;
                emu.cpu_mut().pending_event = 0;
                emu.cpu_mut().async_event = 0;
                emu.cpu_mut().lapic.intr = false;
                emu.cpu_mut().lapic.intr_pending = false;

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(1, 0);
                    ap.deliver_sipi(AP_TRAMPOLINE_VECTOR);
                }
                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();

                let quantum = emu.smp_quantum_ticks();
                let elapsed = unsafe { emu.run_cpu_batch(quantum) }.unwrap();

                assert!(elapsed > 0);
                assert!(
                    emu.pc_system.time_ticks() > 0,
                    "SMP batch did not advance pc_system at the Bochs round boundary"
                );
                assert!(
                    emu.cpu_ref(AP_INDEX).lapic.intr,
                    "AP LAPIC timer did not interrupt during the SMP batch"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bochs main.cc SMP loop: `ticksTotal` grows by BX_TICKN once per round,
    /// so apic.cc get_current_timer_count — frozen within a trace — still
    /// advances between rounds. A CPU whose LAPIC has no queued scheduler
    /// work must therefore observe TMCCT moving across SMP rounds; a frozen
    /// TMCCT hangs guest APIC-timer calibration during AP bring-up.
    #[test]
    fn smp_lapic_tmcct_advances_across_rounds_without_scheduler_work() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const HUGE_TMICT: u32 = 0x0FFF_FFFF;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                let handle = emu
                    .pc_system
                    .register_timer(TimerOwner::Lapic(AP_INDEX), 1, false, false, "ap_lapic")
                    .unwrap();

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(HUGE_TMICT, 0);
                    ap.deliver_sipi(AP_TRAMPOLINE_VECTOR);
                }
                emu.rebuild_cpu_masks_from_scan();
                // Apply the deferred timer activation; afterwards the AP LAPIC
                // has no scheduler work left, exactly like a guest spinning in
                // an APIC-timer calibration loop.
                emu.service_scheduler_boundary(0).unwrap();

                let read_tmcct = |emu: &mut Emulator<'_>| {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    let icount = ap.icount;
                    ap.lapic.read_aligned(0x390, icount)
                };

                let quantum = emu.smp_quantum_ticks();
                let first = read_tmcct(&mut emu);
                for _ in 0..8 {
                    unsafe { emu.run_cpu_batch(quantum) }.unwrap();
                }
                let later = read_tmcct(&mut emu);

                assert!(
                    later < first,
                    "TMCCT frozen across SMP rounds ({later:#x} vs {first:#x}): \
                     round epoch was not stamped into the LAPIC time base"
                );
                assert!(later > 0, "huge TMICT must not have expired in this window");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn bsp_hlt_fast_forward_stays_available_until_application_processors_start() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(8, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();

                assert!(
                    emu.can_fast_forward_bsp_hlt(),
                    "APs waiting for SIPI must not disable the BSP HLT real-time pacing path"
                );

                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                // Direct state poke: production SIPI delivery refreshes the
                // masks itself (refresh_cpu_masks contract); mirror it here.
                emu.refresh_cpu_masks(AP_INDEX);

                assert!(
                    !emu.can_fast_forward_bsp_hlt(),
                    "once an AP is active, the SMP scheduler must own HLT progress"
                );
                emu.assert_cpu_masks_match_scan();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn all_but_self_fixed_ipi_wakes_halted_application_processors() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const FIXED_IPI_VECTOR: u32 = 0xF1;
                const ICR_ALL_BUT_SELF: u32 = 3 << 18;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default().with_topology(2, 2, 2).unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.lapic.write_aligned(0xF0, 0x1FF, 0);
                    if cpu_index != BSP_INDEX {
                        cpu.activity_state = CpuActivityState::Hlt;
                        cpu.set_rflags_for_api(0x202);
                        cpu.pending_event = 0;
                        cpu.async_event = 0;
                        cpu.lapic.intr = false;
                        cpu.lapic.intr_pending = false;
                    }
                }

                emu.cpu_mut_at(BSP_INDEX).lapic.write_aligned(0x300, ICR_ALL_BUT_SELF | FIXED_IPI_VECTOR, 0);
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.drain_lapic_bus();

                for cpu_index in 1..emu.cpu_count() {
                    assert!(
                        emu.cpu_ref(cpu_index).lapic.intr,
                        "AP {cpu_index} did not receive fixed IPI in LAPIC IRR/INTR"
                    );
                    assert!(
                        emu.cpu_ref(cpu_index).pending_event
                            & BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR
                            != 0,
                        "AP {cpu_index} did not get a CPU LAPIC event bit"
                    );
                    assert!(
                        emu.cpu_runnable_for_batch(cpu_index),
                        "halted AP {cpu_index} was not made runnable by fixed IPI"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn nmi_ipi_wakes_shutdown_application_processor() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const ICR_DELIVERY_NMI: u32 = 4 << 8;
                const NMI_HANDLER_SEG: u16 = 0x0500;
                const NMI_HANDLER_ADDR: u64 = (NMI_HANDLER_SEG as u64) << 4;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();

                // Real-mode IVT entry 2 (NMI) -> NMI_HANDLER_SEG:0000, handler = HLT.
                let ivt_entry: [u8; 4] = [
                    0x00,
                    0x00,
                    (NMI_HANDLER_SEG & 0xFF) as u8,
                    (NMI_HANDLER_SEG >> 8) as u8,
                ];
                emu.load_ram(&ivt_entry, 8).unwrap();
                emu.load_ram(&[0xF4], NMI_HANDLER_ADDR).unwrap();

                for cpu_index in 0..emu.cpu_count() {
                    emu.cpu_mut_at(cpu_index).lapic.write_aligned(0xF0, 0x1FF, 0);
                }

                // SIPI-start the AP (unmasks NMI per Bochs deliver_SIPI), then
                // put it into the triple-fault SHUTDOWN state.
                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.deliver_sipi(AP_TRAMPOLINE_VECTOR);
                    ap.activity_state = CpuActivityState::Shutdown;
                    ap.pending_event = 0;
                    ap.async_event = 0;
                }
                emu.cpu_mut().activity_state = CpuActivityState::Hlt;
                emu.cpu_mut().pending_event = 0;
                emu.cpu_mut().async_event = 0;

                assert!(
                    !emu.cpu_runnable_for_batch(AP_INDEX),
                    "shutdown AP with no pending event must stay unscheduled"
                );
                assert!(
                    emu.can_fast_forward_bsp_hlt(),
                    "idle shutdown AP must not disable the BSP HLT pacing path"
                );

                // BSP sends a physical-destination NMI IPI to the AP.
                emu.cpu_mut_at(BSP_INDEX).lapic.write_aligned(0x310, (AP_INDEX as u32) << 24, 0);
                emu.cpu_mut_at(BSP_INDEX).lapic.write_aligned(0x300, ICR_DELIVERY_NMI, 0);
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.drain_lapic_bus();

                assert!(
                    emu.cpu_ref(AP_INDEX)
                        .is_unmasked_event_pending(BxCpuC::<()>::BX_EVENT_NMI),
                    "NMI IPI was not signaled on the shutdown AP"
                );
                assert!(
                    emu.cpu_runnable_for_batch(AP_INDEX),
                    "pending NMI must make a shutdown AP schedulable (Bochs \
                     event.cc handleWaitForEvent wakes SHUTDOWN like HLT)"
                );
                assert!(
                    !emu.can_fast_forward_bsp_hlt(),
                    "pending NMI on a shutdown AP must disable BSP HLT fast-forward"
                );

                let baseline_icount = emu.cpu_ref(AP_INDEX).icount;
                unsafe { emu.run_cpu_batch(256) }.unwrap();

                let ap = emu.cpu_ref(AP_INDEX);
                assert!(
                    !matches!(ap.activity_state, CpuActivityState::Shutdown),
                    "AP did not leave SHUTDOWN after NMI"
                );
                assert!(
                    ap.icount > baseline_icount,
                    "AP did not execute the NMI handler"
                );
                assert_eq!(
                    ap.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
                        .selector
                        .value,
                    NMI_HANDLER_SEG,
                    "AP did not vector through IVT entry 2 to the NMI handler"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn lapic_timer_request_is_activated_before_round_ticks_advance() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Lapic(BSP_INDEX),
                        TEST_LAPIC_TIMER_PERIOD_TICKS,
                        false,
                        false,
                        "bsp_lapic",
                    )
                    .unwrap();
                let programmed_at = emu.pc_system.time_ticks();
                {
                    let cpu = emu.cpu_mut_at(BSP_INDEX);
                    cpu.lapic.timer_handle = Some(handle);
                    cpu.lapic.write_aligned(0xF0, 0x1FF, 0);
                    cpu.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    cpu.lapic.set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32, 0);
                    assert!(cpu.lapic.timer_activate_request.is_some());
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_timer_requests();

                assert!(emu
                    .cpu_ref(BSP_INDEX)
                    .lapic
                    .timer_activate_request
                    .is_none());
                assert_eq!(
                    emu.pc_system.timers[handle].time_to_fire,
                    programmed_at + TEST_LAPIC_TIMER_PERIOD_TICKS
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn self_ipi_control_event_ends_up_batch() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                const SELF_NMI_IPI: u32 =
                    (crate::cpu::apic::ApicDeliveryMode::Nmi as u32) << 8;
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0xF0, 0x1FF, 0);
                // mov dword ptr [FEE00300], SELF_NMI_IPI; inc ebx; hlt
                emu.virt_write(
                    CODE,
                    &[
                        0xC7,
                        0x05,
                        0x00,
                        0x03,
                        0xE0,
                        0xFE,
                        SELF_NMI_IPI as u8,
                        (SELF_NMI_IPI >> 8) as u8,
                        (SELF_NMI_IPI >> 16) as u8,
                        (SELF_NMI_IPI >> 24) as u8,
                        0xFF,
                        0xC3,
                        0xF4,
                    ],
                )
                .unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rbx, 0);

                unsafe { emu.run_cpu_batch(64) }.unwrap();

                assert_eq!(
                    emu.reg_read(X86Reg::Rbx),
                    0,
                    "sentinel after the self-targeted NMI IPI executed before the boundary"
                );
                assert_ne!(
                    emu.cpu_ref(BSP_INDEX).pending_event
                        & BxCpuC::<()>::BX_EVENT_NMI,
                    0,
                    "the queued self-targeted NMI was not committed at the boundary"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn up_lapic_timer_uses_programming_instruction_epoch() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                const INITIAL_COUNT: u32 = 4;
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(TimerOwner::Lapic(BSP_INDEX), 1, false, false, "lapic")
                    .unwrap();
                {
                    let cpu = emu.cpu_mut_at(BSP_INDEX);
                    cpu.lapic.timer_handle = Some(handle);
                    cpu.lapic.write_aligned(0xF0, 0x1FF, 0);
                    cpu.lapic
                        .write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                }
                // mov dword ptr [FEE00380], INITIAL_COUNT; inc ebx; hlt
                emu.virt_write(
                    CODE,
                    &[
                        0xC7,
                        0x05,
                        0x80,
                        0x03,
                        0xE0,
                        0xFE,
                        INITIAL_COUNT as u8,
                        (INITIAL_COUNT >> 8) as u8,
                        (INITIAL_COUNT >> 16) as u8,
                        (INITIAL_COUNT >> 24) as u8,
                        0xFF,
                        0xC3,
                        0xF4,
                    ],
                )
                .unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rbx, 0);

                unsafe { emu.run_cpu_batch(64) }.unwrap();

                let timer_period = emu
                    .cpu_ref(BSP_INDEX)
                    .lapic
                    .timer_period_ticks()
                    .expect("guest initial count must arm the LAPIC timer");
                assert_eq!(
                    emu.reg_read(X86Reg::Rbx),
                    0,
                    "sentinel after timer programming executed before the boundary"
                );
                assert_eq!(
                    emu.pc_system.timers[handle].time_to_fire,
                    timer_period,
                    "LAPIC deadline must be based on the programming instruction epoch"
                );
                assert_eq!(
                    emu.pc_system.time_ticks(),
                    1,
                    "only the programming instruction may retire before the boundary"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn halted_application_processor_lapic_timer_disables_bsp_hlt_fast_forward() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.cpu_mut().activity_state = CpuActivityState::Hlt;

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.activity_state = CpuActivityState::Hlt;
                    ap.set_rflags_for_api(0x202);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, 0x30, 0);
                    ap.lapic.set_initial_timer_count(1, 0);
                    ap.lapic.timer_fired = true;
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.sync_event_flags();

                assert!(
                    !emu.can_fast_forward_bsp_hlt(),
                    "a halted AP with a pending LAPIC timer interrupt must re-enter SMP scheduling"
                );
                assert!(
                    emu.cpu_runnable_for_batch(AP_INDEX),
                    "AP LAPIC timer interrupt was not surfaced as a runnable CPU event"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn service_lapic_local_events_catches_up_overdue_periodic_ap_timers() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Lapic(AP_INDEX),
                        TEST_LAPIC_TIMER_PERIOD_TICKS,
                        false,
                        false,
                        "ap_lapic",
                    )
                    .unwrap();

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, LVT_TIMER_PERIODIC_MODE | TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32, 0);
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();
                emu.service_scheduler_boundary(TEST_LAPIC_TIMER_ELAPSED_TICKS as u64)
                    .unwrap();

                assert_eq!(
                    emu.cpu_ref(AP_INDEX).lapic.diag_timer_fires,
                    TEST_LAPIC_TIMER_ELAPSED_TICKS as u64 / TEST_LAPIC_TIMER_PERIOD_TICKS
                );
                assert_eq!(
                    emu.pc_system.timers[handle].time_to_fire,
                    TEST_LAPIC_TIMER_ELAPSED_TICKS as u64 + TEST_LAPIC_TIMER_PERIOD_TICKS
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn service_lapic_local_events_catches_up_overdue_periodic_ap_timers_beyond_previous_cap() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const ELAPSED_TICKS: u32 = 10_050;
                const EXPECTED_FIRES: u64 = 1005;
                const EXPECTED_NEXT_FIRE: u64 = 10_060;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Lapic(AP_INDEX),
                        TEST_LAPIC_TIMER_PERIOD_TICKS,
                        false,
                        false,
                        "ap_lapic",
                    )
                    .unwrap();

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, LVT_TIMER_PERIODIC_MODE | TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32, 0);
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();
                emu.service_scheduler_boundary(ELAPSED_TICKS as u64)
                    .unwrap();

                assert_eq!(emu.cpu_ref(AP_INDEX).lapic.diag_timer_fires, EXPECTED_FIRES);
                assert_eq!(
                    emu.pc_system.timers[handle].time_to_fire,
                    EXPECTED_NEXT_FIRE
                );
                assert!(emu.cpu_ref(AP_INDEX).lapic.intr);
                assert_ne!(
                    emu.cpu_ref(AP_INDEX).pending_event
                        & BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR,
                    0
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn reset_deactivates_lapic_pc_timer_before_next_tick() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Lapic(BSP_INDEX),
                        TEST_LAPIC_TIMER_PERIOD_TICKS,
                        false,
                        false,
                        "bsp_lapic",
                    )
                    .unwrap();

                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.lapic.timer_handle = Some(handle);
                    bsp.lapic.write_aligned(0xF0, 0x1FF, 0);
                    bsp.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    bsp.lapic.set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32, 0);
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();
                assert!(emu.pc_system.is_timer_active(handle));

                emu.reset(ResetReason::Hardware).unwrap();
                emu.pc_system
                    .tickn((TEST_LAPIC_TIMER_PERIOD_TICKS as u32) + 1);
                emu.dispatch_timer_fires();

                assert!(!emu.cpu_ref(BSP_INDEX).lapic.timer_fired);
                assert_eq!(emu.cpu_ref(BSP_INDEX).lapic.diag_timer_fires, 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn service_lapic_local_events_drains_level_triggered_eoi() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const VECTOR: u8 = 0x40;
                const VECTOR_BIT: u32 = 1;

                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();

                {
                    let lapic = &mut emu.cpu_mut_at(BSP_INDEX).lapic;
                    lapic.write_aligned(0xF0, 0x1FF, 0);
                    lapic.deliver(VECTOR, 0, crate::cpu::apic::APIC_LEVEL_TRIGGERED);
                    assert_eq!(lapic.read_aligned(0x220, 0) & VECTOR_BIT, VECTOR_BIT);
                    assert_eq!(lapic.read_aligned(0x1A0, 0) & VECTOR_BIT, VECTOR_BIT);

                    let acknowledged = lapic.acknowledge_int();
                    assert_eq!(acknowledged, VECTOR);
                    assert_eq!(lapic.read_aligned(0x220, 0) & VECTOR_BIT, 0);
                    assert_eq!(lapic.read_aligned(0x120, 0) & VECTOR_BIT, VECTOR_BIT);
                    assert_eq!(lapic.read_aligned(0x1A0, 0) & VECTOR_BIT, VECTOR_BIT);

                    lapic.receive_eoi(0);
                    assert_eq!(lapic.read_aligned(0x120, 0) & VECTOR_BIT, 0);
                    assert_eq!(lapic.read_aligned(0x1A0, 0) & VECTOR_BIT, 0);
                    assert_eq!(lapic.pending_eoi_vector, Some(VECTOR));
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();

                assert_eq!(emu.cpu_ref(BSP_INDEX).lapic.pending_eoi_vector, None);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn wait_for_sipi_application_processors_credit_quantum_ticks() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(8, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(0x1000, &[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let cpu_count = emu.cpu_count() as u64;
                let before = emu.cpu_ref(BSP_INDEX).icount;
                // batch_size=1 finishes after the first round: the quantum
                // credits alone guarantee elapsed >= 1.
                let elapsed = unsafe { emu.run_cpu_batch(1) }.unwrap();

                let retired = emu.cpu_ref(BSP_INDEX).icount - before;
                // Bochs main.cc bx_begin_simulation: every CPU that executes
                // nothing — including APs parked in WAIT_FOR_SIPI — is
                // credited one SMP quantum ("if (n == 0) n = quantum"), and
                // time advances by executed / BX_SMP_PROCESSORS.
                let expected = (retired + emu.smp_quantum_ticks() * (cpu_count - 1)) / cpu_count;
                assert_eq!(
                    elapsed, expected,
                    "SMP round must advance (retired + quantum credits) / cpu_count \
                     ticks for {retired} retired BSP instructions"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pic_deferred_clear_cannot_erase_reasserted_int_pin() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.cpu
                    .clear_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
                emu.device_manager.pic.irq_pending = true;
                emu.device_manager.pic.irq_cleared = true;
                emu.device_manager.pic.master.int_pin = true;

                emu.sync_event_flags();

                assert_ne!(
                    emu.cpu().pending_event & BxCpuC::<()>::BX_EVENT_PENDING_INTR,
                    0
                );
                assert!(!emu.device_manager.pic.irq_pending);
                assert!(!emu.device_manager.pic.irq_cleared);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn scheduler_boundary_republishes_asserted_pic_without_edge_history() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.device_manager.pic.master.int_pin = true;
                emu.device_manager.pic.irq_pending = false;
                emu.device_manager.pic.irq_cleared = false;
                emu.cpu
                    .clear_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);

                emu.sync_event_flags();

                assert_ne!(
                    emu.cpu().pending_event & BxCpuC::<()>::BX_EVENT_PENDING_INTR,
                    0
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn rep_insw_respects_configured_page_write_permissions() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x2000;
                const UNMAPPED_PORT: u64 = 0x1234;

                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(CODE_ADDR, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.mem_write(DEST_ADDR, &[0x34, 0x12]).unwrap();
                emu.mem_protect(
                    DEST_ADDR,
                    0x1000,
                    crate::cpu::instrumentation::MemPerms::READ,
                );
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);
                emu.reg_write(X86Reg::Rcx, 1);

                unsafe { emu.run_cpu_batch(1) }.unwrap();
                assert_eq!(emu.mem_read_vec(DEST_ADDR, 2).unwrap(), [0x34, 0x12]);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn split_page_rmw_faults_before_first_mmio_read() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                use std::sync::{
                    atomic::{AtomicUsize, Ordering},
                    Arc,
                };

                const CODE_ADDR: u64 = 0x10_0000;
                const DEST_ADDR: u64 = 0x1F_FFFF;
                const SECOND_LARGE_PAGE_PDE: u64 = 0x3008;
                const UNMAPPED_PORT: u64 = 0x1234;

                let reads = Arc::new(AtomicUsize::new(0));
                let read_count = Arc::clone(&reads);
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatLong64,
                )
                .unwrap();
                emu.mem_write(CODE_ADDR, &[0x66, 0x6D, 0xEB, 0xFE]).unwrap();
                // The first byte remains mapped at the end of the first 2 MiB
                // page; the second byte faults in the now-nonpresent page.
                emu.mem_write(SECOND_LARGE_PAGE_PDE, &0u64.to_le_bytes())
                    .unwrap();
                emu.mmio_map(
                    DEST_ADDR,
                    1,
                    Box::new(move |_addr, _size| {
                        read_count.fetch_add(1, Ordering::SeqCst);
                        0
                    }),
                    Box::new(|_addr, _size, _value| {}),
                );
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);

                unsafe { emu.run_cpu_batch(1) }.unwrap();
                assert_eq!(emu.reg_read(X86Reg::Cr2), 0x20_0000);
                assert_eq!(
                    reads.load(Ordering::SeqCst),
                    0,
                    "first MMIO byte was consumed before second-page translation"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn insw_permission_fault_precedes_destructive_mmio_read() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                use std::sync::{
                    atomic::{AtomicUsize, Ordering},
                    Arc,
                };

                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x20_0000;
                const UNMAPPED_PORT: u64 = 0x1234;

                let reads = Arc::new(AtomicUsize::new(0));
                let writes = Arc::new(AtomicUsize::new(0));
                let read_count = Arc::clone(&reads);
                let write_count = Arc::clone(&writes);
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(CODE_ADDR, &[0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.mmio_map(
                    DEST_ADDR,
                    2,
                    Box::new(move |_addr, _size| {
                        read_count.fetch_add(1, Ordering::SeqCst);
                        0
                    }),
                    Box::new(move |_addr, _size, _value| {
                        write_count.fetch_add(1, Ordering::SeqCst);
                    }),
                );
                emu.mem_protect(
                    DEST_ADDR,
                    0x1000,
                    crate::cpu::instrumentation::MemPerms::READ,
                );
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);

                unsafe { emu.run_cpu_batch(1) }.unwrap();
                assert_eq!(reads.load(Ordering::SeqCst), 0);
                assert_eq!(writes.load(Ordering::SeqCst), 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn rep_insw_mmio_fallback_reads_once_per_input_word() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                use std::sync::{
                    atomic::{AtomicUsize, Ordering},
                    Arc,
                };

                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x20_0000;
                const UNMAPPED_PORT: u64 = 0x1234;

                let reads = Arc::new(AtomicUsize::new(0));
                let writes = Arc::new(AtomicUsize::new(0));
                let read_count = Arc::clone(&reads);
                let write_count = Arc::clone(&writes);
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(CODE_ADDR, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.mmio_map(
                    DEST_ADDR,
                    2,
                    Box::new(move |_addr, _size| {
                        read_count.fetch_add(1, Ordering::SeqCst);
                        0
                    }),
                    Box::new(move |_addr, _size, _value| {
                        write_count.fetch_add(1, Ordering::SeqCst);
                    }),
                );
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);
                emu.reg_write(X86Reg::Rcx, 1);

                unsafe { emu.run_cpu_batch(1) }.unwrap();

                assert_eq!(reads.load(Ordering::SeqCst), 1);
                assert_eq!(writes.load(Ordering::SeqCst), 1);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    const PHASE6_FW_CFG_KEY: u16 = 0x1234;

    fn phase6_large_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }
    fn phase6_lock<T>(lock: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }


    fn phase6_flat32() -> Box<Emulator<'static>> {
        Emulator::new_with_mode(
            EmulatorConfig::default(),
            CpuSetupMode::FlatProtected32,
        )
        .unwrap()
    }

    fn phase6_prepare_fw_cfg<T: crate::cpu::instrumentation::Instrumentation>(
        emu: &mut Emulator<'_, T>,
        stream: &[u8],
    ) {
        emu.devices.init(&mut emu.memory).unwrap();
        emu.device_manager
            .init(&mut emu.devices, &mut emu.memory)
            .unwrap();
        emu.device_manager.fw_cfg.add_bytes(PHASE6_FW_CFG_KEY, stream);
        emu.device_manager.fw_cfg.write_port(
            FW_CFG_IO_BASE,
            PHASE6_FW_CFG_KEY as u32,
            FW_CFG_SELECTOR_WRITE_BYTES,
            None,
            &[],
        );
    }

    fn phase6_next_fw_cfg_byte<T: crate::cpu::instrumentation::Instrumentation>(
        emu: &mut Emulator<'_, T>,
    ) -> u8 {
        emu.device_manager
            .fw_cfg
            .read_port_mut(FW_CFG_DATA_PORT, FW_CFG_DATA_READ_BYTES) as u8
    }

    fn phase6_run<T: crate::cpu::instrumentation::Instrumentation>(
        emu: &mut Emulator<'_, T>,
    ) {
        unsafe { emu.run_cpu_batch(1) }.unwrap();
    }

    #[cfg(feature = "instrumentation")]
    #[derive(Clone)]
    struct Phase6RepeatTrace(std::sync::Arc<std::sync::Mutex<Vec<u64>>>);

    #[cfg(feature = "instrumentation")]
    impl crate::cpu::instrumentation::Instrumentation for Phase6RepeatTrace {
        fn active_hooks(&self) -> crate::cpu::instrumentation::HookMask {
            crate::cpu::instrumentation::HookMask::EXEC
        }

        fn repeat_iteration(&mut self, rip: u64, _instr: &Instruction) {
            phase6_lock(&self.0).push(rip);
        }
    }

    #[cfg(feature = "instrumentation")]
    fn phase6_repeat_trace() -> (
        Phase6RepeatTrace,
        std::sync::Arc<std::sync::Mutex<Vec<u64>>>,
    ) {
        let repeats = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        (Phase6RepeatTrace(std::sync::Arc::clone(&repeats)), repeats)
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn ins_byte_and_dword_prefault_before_destructive_port_read() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const DEST: u64 = 0x2000;

            for (code, width, stream) in [
                (
                    &[0xF3, 0x67, 0x6C, 0xEB, 0xFE][..],
                    1usize,
                    &[0xA1, 0xB2][..],
                ),
                (
                    &[0xF3, 0x67, 0x6D, 0xEB, 0xFE][..],
                    4usize,
                    &[0x11, 0x22, 0x33, 0x44, 0x55][..],
                ),
            ] {
                let mut emu = phase6_flat32();
                phase6_prepare_fw_cfg(&mut emu, stream);
                let port = if width == 1 {
                    FW_CFG_DATA_PORT
                } else {
                    crate::iodev::keyboard::KBD_DATA_PORT
                };
                if width == 4 {
                    emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = stream[0];
                    emu.device_manager.keyboard.kbd_controller.outb = true;
                }
                let before = vec![0xCC; width];
                emu.virt_write(CODE, code).unwrap();
                emu.mem_write(DEST, &before).unwrap();
                emu.mem_protect(
                    DEST,
                    0x1000,
                    crate::cpu::instrumentation::MemPerms::READ,
                );
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rdx, u64::from(port));
                emu.reg_write(X86Reg::Rdi, DEST);
                emu.reg_write(X86Reg::Rcx, 1);

                phase6_run(&mut emu);

                assert_eq!(emu.mem_read_vec(DEST, width).unwrap(), before);
                assert_eq!(emu.reg_read(X86Reg::Rdi), DEST);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
                if width == 1 {
                    assert_eq!(
                        phase6_next_fw_cfg_byte(&mut emu),
                        stream[0],
                        "the faulting byte input consumed destructive fw_cfg data"
                    );
                } else {
                    assert!(
                        emu.device_manager.keyboard.kbd_controller.outb,
                        "the faulting dword input consumed the keyboard output byte"
                    );
                }

                let mut emu = phase6_flat32();
                phase6_prepare_fw_cfg(&mut emu, stream);
                if width == 4 {
                    emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = stream[0];
                    emu.device_manager.keyboard.kbd_controller.outb = true;
                }
                emu.virt_write(CODE, code).unwrap();
                emu.mem_write(DEST, &before).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rdx, u64::from(port));
                emu.reg_write(X86Reg::Rdi, DEST);
                emu.reg_write(X86Reg::Rcx, 1);
                phase6_run(&mut emu);

                let expected = if width == 1 {
                    vec![stream[0]]
                } else {
                    vec![stream[0], 0, 0, 0]
                };
                assert_eq!(emu.mem_read_vec(DEST, width).unwrap(), expected);
                assert_eq!(emu.reg_read(X86Reg::Rdi), DEST + width as u64);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
                if width == 1 {
                    assert_eq!(
                        phase6_next_fw_cfg_byte(&mut emu),
                        stream[1],
                        "the successful byte input consumed the wrong fw_cfg span"
                    );
                } else {
                    assert!(
                        !emu.device_manager.keyboard.kbd_controller.outb,
                        "the successful dword input did not consume the keyboard output byte"
                    );
                }
            }
        });
    }

    // Debug-only: asserts on `#[cfg(debug_assertions)]` diagnostic counters
    // (or a `debug_assert!`), which do not exist in a release build.
    #[cfg(debug_assertions)]
    #[cfg(feature = "instrumentation")]
    #[test]
    fn rep_string_io_checks_permission_once_even_when_count_zero() {
        phase6_large_stack(|| {

            const CODE: u64 = 0x1000;
            const HIGH_ZERO_ECX: u64 = 0xDEAD_BEEF_0000_0000;
            const PORT: u16 = 0x80;
            const IO_BITMAP_BASE: u16 = 0x100;
            const GP_VECTOR: u64 = 13;
            const GDT_BASE: u64 = 0x0800;
            const USER_CODE_SELECTOR: u16 = 0x001B;
            const GP_HANDLER: u64 = 0x2000;
            const IDT_BASE: u64 = 0x3000;
            const STACK_TOP: u64 = 0x5000;
            const FORMS: [&[u8]; 6] = [
                &[0xF3, 0x6C, 0xEB, 0xFE],
                &[0xF3, 0x66, 0x6D, 0xEB, 0xFE],
                &[0xF3, 0x6D, 0xEB, 0xFE],
                &[0xF3, 0x6E, 0xEB, 0xFE],
                &[0xF3, 0x66, 0x6F, 0xEB, 0xFE],
                &[0xF3, 0x6F, 0xEB, 0xFE],
            ];

            for code in FORMS {
                let mut emu = phase6_flat32();
                emu.virt_write(CODE, code).unwrap();
                emu.mem_write(GP_HANDLER, &[0xEB, 0xFE]).unwrap();
                emu.mem_write(
                    GDT_BASE + 0x18,
                    &0x00CF_FA00_0000_FFFFu64.to_le_bytes(),
                )
                .unwrap();
                let mut gate = [0u8; 8];
                gate[0..2].copy_from_slice(&(GP_HANDLER as u16).to_le_bytes());
                gate[2..4].copy_from_slice(&USER_CODE_SELECTOR.to_le_bytes());
                gate[5] = 0x8E;
                gate[6..8].copy_from_slice(&((GP_HANDLER >> 16) as u16).to_le_bytes());
                emu.mem_write(IDT_BASE + GP_VECTOR * 8, &gate).unwrap();
                emu.reg_write(X86Reg::IdtrBase, IDT_BASE);
                emu.reg_write(X86Reg::IdtrLimit, GP_VECTOR * 8 + 7);
                emu.reg_write(X86Reg::Rsp, STACK_TOP);
                // The reset task register is a valid 386 TSS at base zero.
                // Install a real denying I/O bitmap entry.  The instruction
                // must raise exactly one #GP before its zero-count exit.
                emu.mem_write(102, &IO_BITMAP_BASE.to_le_bytes()).unwrap();
                emu.mem_write(
                    u64::from(IO_BITMAP_BASE) + u64::from(PORT / 8),
                    &[0xFF, 0xFF],
                )
                .unwrap();
                emu.reg_write(X86Reg::Cs, u64::from(USER_CODE_SELECTOR));
                emu.reg_write(X86Reg::Eflags, 0x2);
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rdx, u64::from(PORT));
                emu.reg_write(X86Reg::Rcx, HIGH_ZERO_ECX);

                phase6_run(&mut emu);

                assert_eq!(
                    emu.cpu().get_exception_diag()[crate::cpu::cpu::Exception::Gp as usize],
                    1,
                    "string I/O permission must be checked once before the zero-count exit"
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Rcx),
                    HIGH_ZERO_ECX,
                    "a zero 32-bit REP count must not clear high RCX"
                );
            }
        });
    }

    #[test]
    fn rep_bulk_respects_32bit_source_and_destination_segment_limits() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const SRC: u64 = 0x3000;
            const DST: u64 = 0x5000;
            const COUNT: u64 = 3;
            const SOURCE: [u8; 12] = [
                0x10, 0x11, 0x12, 0x13, 0x20, 0x21, 0x22, 0x23, 0x30, 0x31, 0x32, 0x33,
            ];

            let run_movsd = |ds_limit, es_limit| {
                let mut emu = phase6_flat32();
                emu.virt_write(CODE, &[0xF3, 0xA5, 0xEB, 0xFE]).unwrap();
                emu.mem_write(SRC, &SOURCE).unwrap();
                emu.mem_fill(DST, SOURCE.len(), 0xCC).unwrap();
                emu.cpu_mut()
                    .set_seg_for_api(X86Reg::Ds, 0x10, 0, ds_limit, false, false);
                emu.cpu_mut()
                    .set_seg_for_api(X86Reg::Es, 0x10, 0, es_limit, false, false);
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rsi, SRC);
                emu.reg_write(X86Reg::Rdi, DST);
                emu.reg_write(X86Reg::Rcx, COUNT);
                phase6_run(&mut emu);
                (
                    emu.mem_read_vec(DST, SOURCE.len()).unwrap(),
                    emu.reg_read(X86Reg::Rcx),
                    emu.reg_read(X86Reg::Rsi),
                    emu.reg_read(X86Reg::Rdi),
                )
            };

            let source_limited = run_movsd((SRC + 7) as u32, u32::MAX);
            assert_eq!(&source_limited.0[..8], &SOURCE[..8]);
            assert_eq!(&source_limited.0[8..], &[0xCC; 4]);
            assert_eq!(source_limited.1, 1);
            assert_eq!(source_limited.2, SRC + 8);
            assert_eq!(source_limited.3, DST + 8);

            let destination_limited = run_movsd(u32::MAX, (DST + 7) as u32);
            assert_eq!(&destination_limited.0[..8], &SOURCE[..8]);
            assert_eq!(&destination_limited.0[8..], &[0xCC; 4]);
            assert_eq!(destination_limited.1, 1);
            assert_eq!(destination_limited.2, SRC + 8);
            assert_eq!(destination_limited.3, DST + 8);

            let mut emu = phase6_flat32();
            emu.virt_write(CODE, &[0xF3, 0xAB, 0xEB, 0xFE]).unwrap();
            emu.mem_fill(DST, 12, 0xCC).unwrap();
            emu.cpu_mut()
                .set_seg_for_api(X86Reg::Es, 0x10, 0, (DST + 7) as u32, false, false);
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0x4433_2211);
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(
                emu.mem_read_vec(DST, 12).unwrap(),
                [0x11, 0x22, 0x33, 0x44, 0x11, 0x22, 0x33, 0x44, 0xCC, 0xCC, 0xCC, 0xCC]
            );
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(emu.reg_read(X86Reg::Rdi), DST + 8);
        });
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn rep_bulk_falls_back_for_hooks_and_page_permissions() {
        phase6_large_stack(|| {
            use crate::cpu::instrumentation::{IoHookType, MemHookType};
            use std::sync::{Arc, Mutex};

            const CODE: u64 = 0x1000;
            const SRC: u64 = 0x2000;
            const DST: u64 = 0x3000;
            const COUNT: u64 = 3;
            const PCI_CONFIG_DATA: u16 = 0x0CFC;

            let events = Arc::new(Mutex::new(Vec::new()));
            let (trace, repeats) = phase6_repeat_trace();
            let exec_events = Arc::clone(&events);
            let mut emu =
                Emulator::<Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            let _ = emu.hook_add_code(CODE..=CODE, move |_rip, _instr| {
                phase6_lock(&exec_events).push("exec");
            });
            emu.virt_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(SRC, &[1, 2, 3]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, SRC);
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(emu.mem_read_vec(DST, 3).unwrap(), [1, 2, 3]);
            assert_eq!(phase6_lock(&events).as_slice(), ["exec"]);
            assert_eq!(phase6_lock(&repeats).len(), COUNT as usize);

            let writes = Arc::new(Mutex::new(Vec::new()));
            let observed_writes = Arc::clone(&writes);
            let mut emu = phase6_flat32();
            let _ = emu.hook_add_mem(MemHookType::Write, DST..=DST + COUNT - 1, move |ev| {
                phase6_lock(&observed_writes).push((ev.addr, ev.size));
            });
            emu.virt_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(SRC, &[4, 5, 6]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, SRC);
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(
                phase6_lock(&writes).as_slice(),
                &[(DST, 1), (DST + 1, 1), (DST + 2, 1)]
            );

            let inputs = Arc::new(Mutex::new(Vec::new()));
            let observed_inputs = Arc::clone(&inputs);
            let mut emu = phase6_flat32();
            phase6_prepare_fw_cfg(&mut emu, &[]);
            emu.device_manager.pci_conf_addr = 0x8000_0000;
            let expected_word = emu.device_manager.pci_read(PCI_CONFIG_DATA, 2) as u16;
            let word_bytes = expected_word.to_le_bytes();
            let _ = emu.hook_add_io(IoHookType::In, PCI_CONFIG_DATA..=PCI_CONFIG_DATA, move |ev| {
                phase6_lock(&observed_inputs).push((ev.port, ev.size, ev.value));
            });
            emu.virt_write(CODE, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                .unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rdx, u64::from(PCI_CONFIG_DATA));
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(
                emu.mem_read_vec(DST, 6).unwrap(),
                [
                    word_bytes[0],
                    word_bytes[1],
                    word_bytes[0],
                    word_bytes[1],
                    word_bytes[0],
                    word_bytes[1],
                ]
            );
            assert_eq!(
                phase6_lock(&inputs).as_slice(),
                &[(PCI_CONFIG_DATA, 2, u32::from(expected_word)); 3]
            );

            let mut emu = phase6_flat32();
            emu.virt_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(SRC, &[0x7A, 0x7B, 0x7C]).unwrap();
            emu.mem_fill(DST, COUNT as usize, 0xCC).unwrap();
            emu.mem_protect(
                DST,
                0x1000,
                crate::cpu::instrumentation::MemPerms::READ,
            );
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, SRC);
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(emu.mem_read_vec(DST, COUNT as usize).unwrap(), [0xCC; 3]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), COUNT);
        });
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn repeat_iteration_is_not_reported_for_faulting_element() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const FIRST_PAGE_END: u64 = 0x2FFF;
            const SECOND_PAGE: u64 = 0x3000;

            let (trace, repeats) = phase6_repeat_trace();
            let mut emu =
                Emulator::<Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            emu.virt_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(0x2000, &[0x41, 0x42]).unwrap();
            emu.mem_protect(
                SECOND_PAGE,
                0x1000,
                crate::cpu::instrumentation::MemPerms::READ,
            );
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, 0x2000);
            emu.reg_write(X86Reg::Rdi, FIRST_PAGE_END);
            emu.reg_write(X86Reg::Rcx, 2);
            phase6_run(&mut emu);
            assert_eq!(emu.mem_read_vec(FIRST_PAGE_END, 1).unwrap(), [0x41]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(phase6_lock(&repeats).len(), 1);

            let (trace, repeats) = phase6_repeat_trace();
            let mut emu =
                Emulator::<Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            emu.virt_write(CODE, &[0xF3, 0xAE, 0xEB, 0xFE]).unwrap();
            emu.mem_write(FIRST_PAGE_END, &[0x55]).unwrap();
            emu.mem_protect(
                SECOND_PAGE,
                0x1000,
                crate::cpu::instrumentation::MemPerms::WRITE,
            );
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0x55);
            emu.reg_write(X86Reg::Rdi, FIRST_PAGE_END);
            emu.reg_write(X86Reg::Rcx, 2);
            phase6_run(&mut emu);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(emu.reg_read(X86Reg::Rdi), SECOND_PAGE);
            assert_eq!(phase6_lock(&repeats).len(), 1);

            let (trace, repeats) = phase6_repeat_trace();
            let mut emu =
                Emulator::<Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            phase6_prepare_fw_cfg(&mut emu, &[0xA5, 0xB6]);
            emu.virt_write(CODE, &[0xF3, 0x6C, 0xEB, 0xFE]).unwrap();
            emu.mem_protect(
                SECOND_PAGE,
                0x1000,
                crate::cpu::instrumentation::MemPerms::READ,
            );
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rdx, u64::from(FW_CFG_DATA_PORT));
            emu.reg_write(X86Reg::Rdi, FIRST_PAGE_END);
            emu.reg_write(X86Reg::Rcx, 2);
            phase6_run(&mut emu);
            assert_eq!(emu.mem_read_vec(FIRST_PAGE_END, 1).unwrap(), [0xA5]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(phase6_lock(&repeats).len(), 1);
            assert_eq!(phase6_next_fw_cfg_byte(&mut emu), 0xB6);
        });
    }
    #[test]
    fn word_mmio_access_preserves_callback_width() {
        phase6_large_stack(|| {
            use std::sync::{Arc, Mutex};

            const CODE: u64 = 0x1000;
            const MMIO: u64 = 0x4000;

            let writes = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::clone(&writes);
            let mut emu = phase6_flat32();
            emu.mmio_map(
                MMIO,
                0x2000,
                Box::new(|_addr, _size| 0),
                Box::new(move |addr, size, value| {
                    phase6_lock(&observed).push((addr, size, value));
                }),
            );
            emu.virt_write(CODE, &[0x66, 0xAB, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0x1234);
            emu.reg_write(X86Reg::Rdi, MMIO);
            phase6_run(&mut emu);
            assert_eq!(
                phase6_lock(&writes).as_slice(),
                &[(MMIO, 2, 0x1234)],
                "a same-page STOSW must be one width-2 memory-handler transaction"
            );

            let writes = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::clone(&writes);
            let mut emu = phase6_flat32();
            emu.mmio_map(
                MMIO,
                0x2000,
                Box::new(|_addr, _size| 0),
                Box::new(move |addr, size, value| {
                    phase6_lock(&observed).push((addr, size, value));
                }),
            );
            emu.virt_write(CODE, &[0x66, 0xAB, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0x1234);
            emu.reg_write(X86Reg::Rdi, MMIO + 0xFFF);
            phase6_run(&mut emu);
            assert_eq!(
                phase6_lock(&writes).as_slice(),
                &[(MMIO + 0xFFF, 1, 0x34), (MMIO + 0x1000, 1, 0x12)],
                "only a real 4 KiB crossing may split a word handler access"
            );

            let mut emu = phase6_flat32();
            emu.virt_write(MMIO, &[0x90, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, MMIO);
            phase6_run(&mut emu);
            let smc_before = emu.memory.smc_seq_next();
            emu.mmio_map(
                MMIO,
                0x1000,
                Box::new(|_addr, _size| 0),
                Box::new(|_addr, _size, _value| {}),
            );
            emu.virt_write(CODE, &[0x66, 0xAB, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0xBEEF);
            emu.reg_write(X86Reg::Rdi, MMIO);
            phase6_run(&mut emu);
            assert_eq!(
                emu.memory.smc_seq_next(),
                smc_before + 1,
                "one successful same-page word span must enqueue one SMC invalidation"
            );
        });
    }

    #[test]
    fn rep_insw_smc_preflight_consumes_only_scalar_committed_word() {
        phase6_large_stack(|| {
            const CODE_AND_DEST: u64 = 0x1000;

            let mut emu = phase6_flat32();
            phase6_prepare_fw_cfg(&mut emu, &[]);
            emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = 0xA5;
            emu.device_manager.keyboard.kbd_controller.outb = true;
            emu.virt_write(CODE_AND_DEST, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                .unwrap();
            emu.reg_write(X86Reg::Rip, CODE_AND_DEST);
            emu.reg_write(
                X86Reg::Rdx,
                u64::from(crate::iodev::keyboard::KBD_DATA_PORT),
            );
            emu.reg_write(X86Reg::Rdi, CODE_AND_DEST);
            emu.reg_write(X86Reg::Rcx, 1);
            let smc_before = emu.memory.smc_seq_next();
            let io_reads_before = emu.devices.diag_io_reads;

            phase6_run(&mut emu);

            assert_eq!(emu.mem_read_vec(CODE_AND_DEST, 2).unwrap(), [0xA5, 0]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
            assert_eq!(emu.reg_read(X86Reg::Rdi), CODE_AND_DEST + 2);
            assert_eq!(
                emu.devices.diag_io_reads,
                io_reads_before + 1,
                "an SMC preflight must not read the device before scalar commit"
            );
            assert!(!emu.device_manager.keyboard.kbd_controller.outb);
            assert_eq!(emu.memory.smc_seq_next(), smc_before + 1);
        });
    }

    #[test]
    fn rep_insd_obeys_one_element_event_budget() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const DEST: u64 = 0x3000;

            let mut emu = phase6_flat32();
            phase6_prepare_fw_cfg(&mut emu, &[]);
            emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = 0xA5;
            emu.device_manager.keyboard.kbd_controller.outb = true;
            emu.pc_system
                .register_timer(TimerOwner::NullTimer, 1, true, true, "phase6 insd deadline")
                .unwrap();
            emu.virt_write(CODE, &[0xF3, 0x6D, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(
                X86Reg::Rdx,
                u64::from(crate::iodev::keyboard::KBD_DATA_PORT),
            );
            emu.reg_write(X86Reg::Rdi, DEST);
            emu.reg_write(X86Reg::Rcx, 1);

            phase6_run(&mut emu);

            assert_eq!(emu.mem_read_vec(DEST, 4).unwrap(), [0xA5, 0, 0, 0]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
            assert_eq!(emu.reg_read(X86Reg::Rdi), DEST + 4);
            assert!(!emu.device_manager.keyboard.kbd_controller.outb);
            assert_eq!(emu.reg_read(X86Reg::Rflags) & (1 << 16), 0);
        });
    }

    /// Bochs unit contract (string.cc fast path vs cpu.cc repeat()):
    /// the fast path charges elements to TICKS (`BX_TICKN(count-1)` →
    /// `tick_surplus`) and retires one icount per chunk, while the scalar
    /// repeat() loop retires one icount per element. Tick totals match
    /// exactly; icount deliberately differs.
    #[cfg(feature = "instrumentation")]
    #[test]
    fn fast_rep_charges_ticks_not_icount_and_matches_scalar_ticks() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const SRC: u64 = 0x2FF8;
            const DST: u64 = 0x4FF8;
            const COUNT: u64 = 8;

            let mut fast = phase6_flat32();
            fast.pc_system
                .register_timer(TimerOwner::NullTimer, COUNT, true, true, "phase6 fast")
                .unwrap();
            fast.virt_write(CODE, &[0xF3, 0x66, 0xA5, 0xEB, 0xFE])
                .unwrap();
            fast.mem_fill(SRC, (COUNT * 2) as usize, 0x5A).unwrap();
            fast.reg_write(X86Reg::Rip, CODE);
            fast.reg_write(X86Reg::Rsi, SRC);
            fast.reg_write(X86Reg::Rdi, DST);
            fast.reg_write(X86Reg::Rcx, COUNT);
            let fast_before = fast.cpu_ref(BSP_INDEX).icount;
            let fast_surplus_before = fast.cpu_ref(BSP_INDEX).tick_surplus;
            phase6_run(&mut fast);
            let fast_retired = fast.cpu_ref(BSP_INDEX).icount - fast_before;
            let fast_surplus = fast.cpu_ref(BSP_INDEX).tick_surplus - fast_surplus_before;
            let fast_ticks = fast.ticks();

            let (trace, repeats) = phase6_repeat_trace();
            let mut scalar =
                Emulator::<Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            scalar
                .pc_system
                .register_timer(TimerOwner::NullTimer, COUNT, true, true, "phase6 scalar")
                .unwrap();
            scalar
                .virt_write(CODE, &[0xF3, 0x66, 0xA5, 0xEB, 0xFE])
                .unwrap();
            scalar.mem_fill(SRC, (COUNT * 2) as usize, 0x5A).unwrap();
            scalar.reg_write(X86Reg::Rip, CODE);
            scalar.reg_write(X86Reg::Rsi, SRC);
            scalar.reg_write(X86Reg::Rdi, DST);
            scalar.reg_write(X86Reg::Rcx, COUNT);
            let scalar_before = scalar.cpu_ref(BSP_INDEX).icount;
            phase6_run(&mut scalar);
            let scalar_retired = scalar.cpu_ref(BSP_INDEX).icount - scalar_before;

            assert_eq!(fast.mem_read_vec(DST, (COUNT * 2) as usize).unwrap(), scalar.mem_read_vec(DST, (COUNT * 2) as usize).unwrap());
            assert_eq!(fast.reg_read(X86Reg::Rcx), 0);
            assert_eq!(scalar.reg_read(X86Reg::Rcx), 0);
            // Both batches execute the identical instruction tail (the REP
            // plus one parking jump), so moving element charges into
            // tick_surplus must conserve the tick total exactly while icount
            // retires per chunk (fast) vs per element (scalar repeat()).
            assert!(
                fast_surplus > 0,
                "fast path must charge elements to tick_surplus, not icount"
            );
            assert_eq!(fast_retired + fast_surplus, scalar_retired);
            assert!(fast_retired < scalar_retired);
            assert_eq!(fast_ticks, scalar.ticks());
            assert_eq!(phase6_lock(&repeats).len(), COUNT as usize);
        });
    }

    #[test]
    fn fast_rep_element_budget_uses_elements_not_bytes() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const SRC: u64 = 0x2000;
            const DST: u64 = 0x4000;
            const DEADLINE: u64 = 3;
            const COUNT: u64 = 7;

            for (code, width, stores) in [
                (&[0xF3, 0x66, 0xA5, 0xEB, 0xFE][..], 2u64, false),
                (&[0xF3, 0xA5, 0xEB, 0xFE][..], 4u64, false),
                (&[0xF3, 0x66, 0xAB, 0xEB, 0xFE][..], 2u64, true),
                (&[0xF3, 0xAB, 0xEB, 0xFE][..], 4u64, true),
            ] {
                let mut emu = phase6_flat32();
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, DEADLINE, true, true, "phase6 element budget")
                    .unwrap();
                emu.virt_write(CODE, code).unwrap();
                emu.mem_fill(SRC, (COUNT * width) as usize, 0x6D).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rax, 0x1122_3344);
                emu.reg_write(X86Reg::Rsi, SRC);
                emu.reg_write(X86Reg::Rdi, DST);
                emu.reg_write(X86Reg::Rcx, COUNT);
                phase6_run(&mut emu);
                assert_eq!(emu.reg_read(X86Reg::Rcx), COUNT - DEADLINE);
                assert_eq!(emu.reg_read(X86Reg::Rdi), DST + DEADLINE * width);
                if !stores {
                    assert_eq!(emu.reg_read(X86Reg::Rsi), SRC + DEADLINE * width);
                }
            }

            const LONG_CODE: u64 = 0x10_000;
            const LONG_SRC: u64 = 0x12_000;
            const LONG_DST: u64 = 0x14_000;
            for (code, stores) in [
                (&[0xF3, 0x48, 0xA5, 0xEB, 0xFE][..], false),
                (&[0xF3, 0x48, 0xAB, 0xEB, 0xFE][..], true),
            ] {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatLong64,
                )
                .unwrap();
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, DEADLINE, true, true, "phase6 qword budget")
                    .unwrap();
                emu.mem_write(LONG_CODE, code).unwrap();
                emu.mem_fill(LONG_SRC, (COUNT * 8) as usize, 0x7E).unwrap();
                emu.reg_write(X86Reg::Rip, LONG_CODE);
                emu.reg_write(X86Reg::Rax, 0x1122_3344_5566_7788);
                emu.reg_write(X86Reg::Rsi, LONG_SRC);
                emu.reg_write(X86Reg::Rdi, LONG_DST);
                emu.reg_write(X86Reg::Rcx, COUNT);
                phase6_run(&mut emu);
                assert_eq!(emu.reg_read(X86Reg::Rcx), COUNT - DEADLINE);
                assert_eq!(emu.reg_read(X86Reg::Rdi), LONG_DST + DEADLINE * 8);
                if !stores {
                    assert_eq!(emu.reg_read(X86Reg::Rsi), LONG_SRC + DEADLINE * 8);
                }
            }
        });
    }

    // Debug-only: asserts on `#[cfg(debug_assertions)]` diagnostic counters
    // (or a `debug_assert!`), which do not exist in a release build.
    #[cfg(debug_assertions)]
    #[test]
    fn cold_tlb_rep_movsb_propagates_one_page_fault_without_committing() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x10_0000;
            const SRC: u64 = 0x10_1000;
            const DEST: u64 = 0x20_0000;
            const SECOND_LARGE_PAGE_PDE: u64 = 0x3008;

            let mut emu = Emulator::new_with_mode(
                EmulatorConfig::default(),
                CpuSetupMode::FlatLong64,
            )
            .unwrap();
            emu.mem_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(SRC, &[0x5A]).unwrap();
            emu.mem_write(SECOND_LARGE_PAGE_PDE, &0u64.to_le_bytes())
                .unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, SRC);
            emu.reg_write(X86Reg::Rdi, DEST);
            emu.reg_write(X86Reg::Rcx, 1);

            let page_faults_before = emu.cpu().get_exception_diag()[14];
            phase6_run(&mut emu);

            assert_eq!(emu.reg_read(X86Reg::Cr2), DEST);
            assert_eq!(emu.cpu().get_exception_diag()[14], page_faults_before + 1);
            assert_eq!(emu.reg_read(X86Reg::Rsi), SRC);
            assert_eq!(emu.reg_read(X86Reg::Rdi), DEST);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
        });
    }

    // Debug-only: asserts on `#[cfg(debug_assertions)]` diagnostic counters
    // (or a `debug_assert!`), which do not exist in a release build.
    #[cfg(debug_assertions)]
    #[test]
    fn cold_tlb_rep_insw_propagates_one_fault_without_consuming_port_input() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x10_0000;
            const DEST: u64 = 0x20_0000;
            const SECOND_LARGE_PAGE_PDE: u64 = 0x3008;

            let mut emu = Emulator::new_with_mode(
                EmulatorConfig::default(),
                CpuSetupMode::FlatLong64,
            )
            .unwrap();
            phase6_prepare_fw_cfg(&mut emu, &[0xA1, 0xB2]);
            emu.mem_write(CODE, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                .unwrap();
            emu.mem_write(SECOND_LARGE_PAGE_PDE, &0u64.to_le_bytes())
                .unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rdx, FW_CFG_DATA_PORT as u64);
            emu.reg_write(X86Reg::Rdi, DEST);
            emu.reg_write(X86Reg::Rcx, 1);

            let page_faults_before = emu.cpu().get_exception_diag()[14];
            phase6_run(&mut emu);

            assert_eq!(emu.reg_read(X86Reg::Cr2), DEST);
            assert_eq!(emu.cpu().get_exception_diag()[14], page_faults_before + 1);
            assert_eq!(emu.reg_read(X86Reg::Rdi), DEST);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(phase6_next_fw_cfg_byte(&mut emu), 0xA1);
        });
    }

    #[test]
    fn empty_memory_write_is_a_no_op() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let before = emu.mem_read_vec(0x1000, 4).unwrap();

                emu.mem_write(0x1000, &[]).unwrap();

                assert_eq!(emu.mem_read_vec(0x1000, 4).unwrap(), before);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[cfg(feature = "std")]
    #[test]
    fn snapshot_rebuilds_runnable_and_lapic_work_masks_before_resume() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let config = EmulatorConfig {
                    guest_memory_size: 4 * 1024 * 1024,
                    host_memory_size: 4 * 1024 * 1024,
                    cpu_params: BxParams::default().with_topology(2, 1, 1).unwrap(),
                    ..EmulatorConfig::default()
                };
                let mut source = Emulator::new(config.clone()).unwrap();
                source.initialize().unwrap();
                source.reset(ResetReason::Hardware).unwrap();
                source.cpu_mut_at(BSP_INDEX).activity_state = CpuActivityState::Active;
                source.cpu_mut_at(AP_INDEX).activity_state = CpuActivityState::WaitForSipi;
                source.cpu_mut_at(BSP_INDEX).lapic.timer_fired = true;
                source.runnable_mask = CpuMask::default();
                source.lapic_work_mask = CpuMask::default();

                let mut snapshot = Vec::new();
                source.save_snapshot(&mut snapshot).unwrap();

                let mut restored = Emulator::new(config).unwrap();
                restored.initialize().unwrap();
                restored.reset(ResetReason::Hardware).unwrap();
                restored.runnable_mask.assign(AP_INDEX, true);
                restored.lapic_work_mask.assign(AP_INDEX, true);
                restored
                    .restore_snapshot(&mut std::io::Cursor::new(snapshot))
                    .unwrap();

                assert!(restored.runnable_mask.contains(BSP_INDEX));
                assert!(!restored.runnable_mask.contains(AP_INDEX));
                assert!(restored.lapic_work_mask.contains(BSP_INDEX));
                assert!(!restored.lapic_work_mask.contains(AP_INDEX));
            })
            .unwrap()
            .join()
            .unwrap();
    }
