//! Positive control for the `RUSTY_BOX_PF_DIAG` null-write tripwire
//! (cpu/pf_diag.rs).
//!
//! The tripwire exists to catch an intermittent Ubuntu guest segfault; a
//! diagnostic that silently fails to fire would produce a false "no signal"
//! verdict, so this test proves end-to-end that a genuine not-present write
//! fault at a null-page linear address produces a report file: guest code
//! above 2 MiB unmaps virtual 0..2 MiB (clears the 2 MiB PD entry the
//! FlatLong64 harness builds at phys 0x3000), INVLPGs, then stores through a
//! zero pointer.

#![cfg(feature = "std")]

use rusty_box::cpu::{core_i7_skylake::Corei7SkylakeX, CpuSetupMode};
use rusty_box::emulator::{Emulator, EmulatorConfig};

/// Same sizing rationale as fp_vex_scalar_ops.rs: the Emulator is ~4 MiB and
/// debug builds materialise copies during construction.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

/// Above the 2 MiB page this test unmaps, so instruction fetch keeps working.
const CODE: u64 = 0x0040_0000;

/// FlatLong64 page-table layout (emulator_api.rs `setup_flat_long64`):
/// PML4 @ 0x1000, PDPT @ 0x2000, PD0 @ 0x3000 — entry 0 maps virtual
/// 0..2 MiB.
const PD0_ENTRY0: u64 = 0x3000;

#[test]
fn pf_diag_tripwire_reports_a_null_page_write_fault() {
    std::thread::Builder::new()
        .stack_size(TEST_STACK_SIZE)
        .spawn(|| {
            let diag_path = std::env::temp_dir().join(format!(
                "rusty_box_pf_diag_control_{}.txt",
                std::process::id()
            ));
            match std::fs::remove_file(&diag_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => panic!("could not clear stale diag file: {e}"),
            }
            std::env::set_var("RUSTY_BOX_PF_DIAG", &diag_path);

            let cfg = EmulatorConfig::default();
            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(cfg, CpuSetupMode::FlatLong64)
                .expect("new emulator");

            // xor eax,eax ; invlpg [rax] ; mov [rax],rax ; jmp $
            emu.mem_write(
                CODE,
                &[0x31, 0xC0, 0x0F, 0x01, 0x38, 0x48, 0x89, 0x00, 0xEB, 0xFE],
            )
            .expect("write guest code");

            // Unmap virtual 0..2 MiB (guest code sits above it; the GDT and
            // page tables inside it are only ever read physically).
            emu.mem_write(PD0_ENTRY0, &0u64.to_le_bytes())
                .expect("clear PD0[0]");

            // The store faults; with no IDT installed delivery escalates, so
            // the run ends in an error — the tripwire fires before delivery
            // either way. Both outcomes are acceptable here.
            match emu.emu_start(CODE, Some(CODE + 8), None, Some(16)) {
                Ok(_) | Err(_) => {}
            }

            let report =
                std::fs::read_to_string(&diag_path).expect("the tripwire must write a report");
            assert!(
                report.contains("null-page write fault"),
                "report missing header:\n{report}"
            );
            assert!(
                report.contains("CR2=0x0"),
                "fault linear address must be 0:\n{report}"
            );
            assert!(
                report.contains("fresh decode at fault RIP"),
                "report must decode current memory at RIP:\n{report}"
            );
            assert!(
                report.contains("MISMATCHES"),
                "report must include the icache page audit:\n{report}"
            );

            std::fs::remove_file(&diag_path).expect("remove diag file");
        })
        .expect("spawn test thread")
        .join()
        .expect("join test thread");
}
